"""Prepare fresh August accounts and select at most five sizing finalists.

This tool never launches a backtest, terminal or monitor. It prepares immutable
plans for research_runner_sizing.py, then reads completed receipts. August and
September are previously inspected historical data, not untouched holdouts.

  python tools/validate_lot_growth_finalists.py prepare --study TRAIN --output AUGUST
  python tools/research_runner_sizing.py AUGUST/PLAN.json   # separate authorization
  python tools/validate_lot_growth_finalists.py select --validation AUGUST --output FINAL

Full owner/LOTTO/day/week/month follow-ups are a blueprint only. Their results
cannot enter this selector, since the full owner window includes September 1.
"""
from __future__ import annotations

import argparse
import copy
import datetime as dt
import math
import re
from pathlib import Path

import prepare_lot_growth_sweep as prep
import rank_lot_growth_sweep as rank
import research_runner_sizing as sizing
from research_runner_exact8 import verify_detail_binding

SCHEMA = 'conduit.sizing-finalists.august-plan.v1'
FINAL_SCHEMA = 'conduit.sizing-finalists.august-selection.v1'
CONTROL_IDS = ('GOD-X7-fixed001', 'GOD-X7-cap5')
WINDOW = ('2026-08-01', '2026-09-01')
SAFE = re.compile(r'[A-Za-z0-9_-]+\Z')
SELECTOR = {
    'maximum': 5, 'gates': 'same V3 TRAIN gates on each fresh August deposit',
    'rank': ['max ceil(DD/2.5) over TRAIN/Aug300/Aug600 ascending',
             'min August positive_market_days_pct descending',
             'min August median_weekly_return descending',
             'min August log_growth descending', 'settings_sha256 ascending'],
    'september_or_full_owner_used_for_selection': False,
    'relax_gates': False, 'untouched_holdout': False,
}
HELPERS = ('prepare_lot_growth_sweep.py', 'rank_lot_growth_sweep.py',
           'research_runner.py', 'research_runner_exact8.py', 'research_runner_sizing.py')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def number(value):
    return type(value) in (int, float) and math.isfinite(value)


def checked_pin(record):
    require(isinstance(record, dict) and set(('path', 'bytes', 'sha256')) <= set(record), 'Incomplete file pin')
    require(type(record['bytes']) is int and record['bytes'] >= 0, 'Invalid pinned length')
    actual = prep.pin(record['path'])
    require(actual == {k: record[k] for k in actual}, 'Pinned file changed: '+str(record['path']))
    return actual


def unique_pins(records):
    result = {}
    for record in records:
        key = str(Path(record['path']).resolve())
        if key in result:
            require(result[key] == record, 'Conflicting pins for one input')
        result[key] = record
    return list(result.values())


def flag(argv, key):
    require(argv.count(key) == 1, 'Expected one argument '+key)
    index = argv.index(key)+1
    require(index < len(argv), 'Missing argument '+key)
    return argv[index]


def safe_id(value):
    require(isinstance(value, str) and SAFE.fullmatch(value), 'Unsafe candidate/job identity')
    return value


def plan_inputs(plan):
    # Reuse the production research gate: EXE/build/frozen-source identity,
    # baseline materialization and the complete reference argv are mandatory.
    sizing.validate_plan_inputs(plan)
    records = unique_pins(plan['inputs'])
    require(len(records) == len(plan['inputs']), 'Repeated input pin')
    for record in records:
        checked_pin(record)
    result = {r['path']: r for r in records}
    for name in HELPERS:
        path = Path(__file__).with_name(name).resolve()
        require(result.get(str(path)) == prep.pin(path), 'Plan does not pin current helper '+name)
    return result


def referenced(inputs, path):
    path = str(Path(path).resolve())
    require(path in inputs, 'Unbound input '+path)
    return inputs[path]


def check_row(row, window, balance, baseline=False):
    numeric = ('start_balance', 'end_equity', 'total_profit', 'max_dd_pct', 'min_equity',
               'positive_market_days_pct')
    require(all(number(row.get(k)) for k in numeric), 'Missing/non-finite financial metric: '+row['id'])
    require(row['start_balance'] == balance, 'Unexpected starting account balance')
    require(row['max_dd_pct'] >= 0 and 0 <= row['positive_market_days_pct'] <= 100, 'Invalid percentage')
    require(row.get('median_weekly_return') is None or number(row['median_weekly_return']), 'Invalid weekly statistic')
    require(type(row.get('blown')) is bool, 'Missing insolvency observation')
    for key in ('stop_outs', 'filled_baskets', 'active_entry_days', 'market_days', 'trades', 'baskets'):
        require(type(row.get(key)) is int and row[key] >= 0, 'Missing/non-integer count '+key)
    dates = row['market_dates']
    require(dates and dates == sorted(set(dates)), 'Market dates absent, duplicated or unordered')
    for date in dates:
        dt.date.fromisoformat(date)
        require(window[0] <= date < window[1], 'A result day lies outside the declared window')
    require(row['market_days'] == len(dates), 'Market-day count and daily path disagree')
    require(row['active_entry_days'] <= len(dates), 'Entry days exceed observed market days')
    if row['end_equity'] > 0:
        require(number(row.get('log_growth')), 'Positive equity without log growth')
    if baseline:
        require(row['filled_baskets'] > 0 and row['active_entry_days'] > 0,
                'A zero/missing activity denominator cannot qualify candidates')


def completed_job(plan_path, plan, job, expected, inputs):
    """Exact completion identity plus all three consumed artifact families."""
    root = Path(plan['output']).resolve()
    require(Path(plan_path).resolve() == root/'PLAN.json', 'Plan must belong to its declared study directory')
    identity = safe_id(job['id'])
    result_dir = (root/identity/'results').resolve()
    require(Path(job['result_dir']).resolve() == result_dir, 'Unexpected result directory')
    argv = job['argv']
    require(isinstance(argv, list) and all(isinstance(v, str) for v in argv), 'Invalid argv')
    require(Path(flag(argv, '--out')).resolve() == result_dir, 'Output argv mismatch')
    require(job['expected_candidates'] == len(expected), 'Expected candidate count mismatch')
    exe = referenced(inputs, argv[0])
    for key in ('--ticks', '--signals'):
        referenced(inputs, flag(argv, key))
    require('--auto-ea' not in argv, 'GOD-X7 validation cannot switch to AUTO-EA')
    require(flag(argv, '--quick-tick-stride') == '1', 'Approximate tick stride is forbidden')
    folder = Path(flag(argv, '--sweep')).resolve()
    require({p.name for p in folder.iterdir()} == {name+'.json' for name in expected}, 'Sweep directory contains missing or extra files')
    for name, config in expected.items():
        safe_id(name)
        require(referenced(inputs, folder/(name+'.json')) == config['file'], 'Sweep file is not the expected pinned config')
    receipt_path = root/identity/'receipt.json'
    receipt = prep.read(receipt_path)
    require(receipt.get('status') == 'complete' and receipt.get('returncode') == 0
            and receipt.get('partial') is False and receipt.get('validation_errors') == [], 'Job did not complete exactly')
    normalized_argv = list(argv); normalized_argv[0] = str(Path(argv[0]).resolve())
    for key, value in {'id':identity, 'argv':normalized_argv, 'threads':job['threads'],
                       'plan_sha256':prep.pin(plan_path)['sha256'], 'source_revision':plan['source_revision'],
                       'exe_sha256':exe['sha256']}.items():
        require(receipt.get(key) == value, 'Completion receipt differs from plan: '+key)
    summary_path = result_dir/'wyniki_compound.json'
    require(receipt.get('result_files') == [str(summary_path)], 'Unexpected summary artifact list')
    require(receipt.get('result_sha256') == {summary_path.name:prep.pin(summary_path)['sha256']}, 'Summary differs from completion')
    summary = prep.read(summary_path)
    require(set(summary) == set(expected), 'Summary candidate set differs from the plan')
    require(not (result_dir/'PRZERWANE.txt').exists(), 'Interrupted result marker exists')
    qualified_summary, qualified_details = rank.verified_results(root, identity)
    require(prep.exact_equal(summary, qualified_summary), 'Qualified summary differs')
    rows, evidence = {}, [prep.pin(receipt_path), prep.pin(summary_path)]
    window = (flag(argv,'--from'), flag(argv,'--to'))
    balance = float(flag(argv,'--balance'))
    for name, metrics in summary.items():
        detail_path = result_dir/(name+'_compound_dane.json')
        basket_path = result_dir/(name+'_compound_koszyki.json')
        artifacts = [detail_path, basket_path]
        require(type(metrics.get('trades')) is int and metrics['trades'] >= 0, 'Missing trade count')
        if metrics['trades']:
            artifacts.append(result_dir/(name+'_compound_transakcje.json'))
        for path in artifacts:
            verify_detail_binding(path, receipt)
            evidence.append(prep.pin(path))
        detail = qualified_details[name]
        baskets = detail['koszyki']
        require(isinstance(detail.get('dni'), list) and isinstance(baskets, list), 'Missing daily/basket path')
        for day in detail['dni']:
            require(number(day.get('start_equity')) and number(day.get('end_equity')), 'Invalid daily equity observations')
        ledger = detail['transakcje']
        row = rank.describe(name, metrics, detail, baskets, expected[name]['settings_sha256'], trades=ledger)
        check_row(row, window, balance, baseline=name == CONTROL_IDS[0])
        rows[name] = row
    return rows, evidence


def registry_config(record, inputs):
    name = safe_id(record['id'])
    checked_pin(record['file'])
    require(referenced(inputs, record['file']['path']) == record['file'], 'Config absent from plan inputs')
    doc = prep.read(record['file']['path'])
    require(doc['name'] == name, 'Config name differs from registry')
    require(prep.fingerprint(doc['settings']) == record['settings_sha256'], 'Settings fingerprint mismatch')
    require(doc['settings']['lot_max'] == 5 and doc['settings']['lot_growth_basket_risk_pct'] == 0,
            'Candidate cap or additional basket budget changed')
    return doc


def train_contract(study):
    root = Path(study).resolve(); plan_path = root/'PLAN.json'
    plan = prep.read(plan_path); inputs = plan_inputs(plan)
    require(Path(plan['output']).resolve() == root, 'TRAIN output location mismatch')
    prereg_path = root/'PREREGISTRATION.json'
    referenced(inputs, prereg_path)
    prereg = rank.verify_training_design(root)
    require(prereg['schema'] == 'conduit.sizing300.preregistered.v3' and prereg['candidate_count'] == 300,
            'Expected the frozen V3 paired TRAIN study')
    require(prereg['training'] == {'from':'2026-06-20','to_exclusive':'2026-08-01'}, 'Unexpected TRAIN window')
    require(prereg['validation'] == {'from':WINDOW[0],'to_exclusive':WINDOW[1],'fresh_deposits':[300,600]}, 'Unexpected validation contract')
    require(prereg['gates'] == {'complete':True,'stopout':False,'min_equity_gt':0,'profit_gt':0,
        'max_dd_pct_lte':35,'positive_market_days_pct_gte':50,'filled_baskets_fraction_of_fixed001_gte':.5,
        'active_entry_days_fraction_of_fixed001_gte':.8}, 'Gate thresholds changed')
    registry = {r['id']:r for r in prereg['candidates']}
    require(len(registry) == len(prereg['candidates']) == 300, 'Expected all 300 unique candidates')
    for r in registry.values(): registry_config(r, inputs)
    jobs = {j['id']:j for j in plan['jobs']}
    require(set(jobs) == {'sizing300','controls'} and len(plan['jobs']) == 2, 'Unexpected TRAIN jobs')
    controls = {}
    control_configs = Path(flag(jobs['controls']['argv'], '--sweep')).resolve()
    baseline_settings = prep.read(plan['qualification']['materialized_baseline']['path'])['settings']
    for name in CONTROL_IDS:
        file = referenced(inputs, control_configs/(name+'.json'))
        doc = prep.read(file['path'])
        require(doc['name'] == name and doc['settings']['lot_growth_mode'] == 'Off'
                and not doc['settings'].get('ea_enabled'), 'Control is not legacy GOD-X7')
        require(doc['settings']['lot_max'] == (.01 if name == CONTROL_IDS[0] else 5), 'Control cap changed')
        expected_settings = {**baseline_settings, 'lot_max': doc['settings']['lot_max']}
        require(prep.settings_equal(doc['settings'], expected_settings), 'Control changed a GOD-X7 strategy setting')
        controls[name] = {'id':name,'file':file,'settings_sha256':prep.fingerprint(doc['settings'])}
    first, second = [prep.read(controls[n]['file']['path'])['settings'] for n in CONTROL_IDS]
    require({k:v for k,v in first.items() if k!='lot_max'} == {k:v for k,v in second.items() if k!='lot_max'}, 'Controls differ outside cap')
    for job in jobs.values():
        require((flag(job['argv'],'--from'),flag(job['argv'],'--to')) == ('2026-06-20','2026-08-01')
                and float(flag(job['argv'],'--balance')) == 600, 'Unexpected TRAIN recipe')
    rows, evidence = completed_job(plan_path,plan,jobs['sizing300'],registry,inputs)
    control_rows, control_evidence = completed_job(plan_path,plan,jobs['controls'],controls,inputs)
    baseline = control_rows[CONTROL_IDS[0]]
    evaluated = rank.select(list(rows.values()),baseline)
    selection_path, shortlist_path = root/'TRAIN_SELECTION.json', root/'SHORTLIST.json'
    actual = prep.read(selection_path)
    require(actual.get('schema') == 'conduit.sizing300.train-selection.v1'
            and actual.get('preregistration') == prep.pin(prereg_path), 'TRAIN selection provenance mismatch')
    require(actual.get('plan') == prep.pin(plan_path)
            and actual.get('completion_receipts') == [prep.pin(root/job/'receipt.json') for job in ('controls','sizing300')],
            'TRAIN selection is not bound to its completed plan and receipts')
    for key, value in evaluated.items():
        require(prep.exact_equal(actual.get(key), value), 'TRAIN selection was not recomputed from qualified evidence: '+key)
    # The existing TRAIN selector uses a display-only literal SHA for its baseline.
    require(prep.exact_equal(actual.get('baseline'), {**baseline,'settings_sha256':'reference'}), 'TRAIN baseline changed')
    shortlist = [registry[r['id']] for r in evaluated['selected']]
    require(prep.exact_equal(prep.read(shortlist_path), shortlist) and len(shortlist) <= 10, 'SHORTLIST differs from TRAIN selection')
    evidence += control_evidence + [prep.pin(selection_path),prep.pin(shortlist_path),prep.pin(prereg_path),prep.pin(plan_path)]
    return dict(root=root,plan=plan,prereg=prereg,inputs=inputs,registry=registry,shortlist=shortlist,
                rows=rows,controls=controls,evidence=unique_pins(evidence))


def new_output(path):
    out = Path(path).resolve()
    require(not out.exists(), 'Output already exists; use a new immutable directory')
    return out


def make_jobs(train_argv, out, count, cpu_budget):
    require(type(cpu_budget) is int and 1 <= cpu_budget <= 24, 'CPU budget must be 1..24')
    jobs=[]
    for deposit in (300,600):
        identity=f'august_{deposit}'; result=out/identity/'results'
        argv=list(train_argv)
        for key,value in {'--from':WINDOW[0],'--to':WINDOW[1],'--balance':str(deposit),
                          '--sweep':str(out/'configs'),'--out':str(result)}.items():
            flag(argv,key); argv[argv.index(key)+1]=value
        jobs.append({'id':identity,'argv':argv,'threads':cpu_budget,'result_dir':str(result),
                     'expected_candidates':count})
    return jobs


def prepare(study, output, cpu_budget=12):
    out=new_output(output); train=train_contract(study)
    require(train['shortlist'], 'No qualifying TRAIN candidates; do not prepare a coronation')
    jobs_by_id={j['id']:j for j in train['plan']['jobs']}
    configurations={r['id']:r for r in train['shortlist']}
    configurations.update(train['controls'])
    docs={name:prep.read(r['file']['path']) for name,r in configurations.items()}
    require(len(docs)==len(train['shortlist'])+2, 'Candidate and control names collide')
    jobs=make_jobs(jobs_by_id['sizing300']['argv'],out,len(docs),cpu_budget)
    stage_registry={}
    # All validation and source checks above precede the first output write.
    out.mkdir(parents=True)
    for name,doc in docs.items():
        path=out/'configs'/(name+'.json'); prep.write_new(path,doc)
        stage_registry[name]={'id':name,'file':prep.pin(path),'settings_sha256':prep.fingerprint(doc['settings']),
                              'source':configurations[name]['file'],'control':name in CONTROL_IDS}
    inputs=unique_pins(list(train['inputs'].values())+train['evidence']
                       +[prep.pin(__file__)]+[r['file'] for r in stage_registry.values()])
    plan={'id':'g7_sizing_august_finalists','name':'GOD-X7 sizing — świeży sierpień 300/600 USD',
          'output':str(out),'progress_dir':str(out/'progress'),'threads':cpu_budget,
          'source_revision':train['plan']['source_revision'],'inputs':inputs,'jobs':jobs,
          'qualification':copy.deepcopy(train['plan']['qualification']),
          'finalists_contract':{'schema':SCHEMA,'train_plan':prep.pin(train['root']/'PLAN.json'),
              'train_selection':prep.pin(train['root']/'TRAIN_SELECTION.json'),
              'shortlist':prep.pin(train['root']/'SHORTLIST.json'),'registry':stage_registry,
              'selected_train_ids':[r['id'] for r in train['shortlist']],'selector':SELECTOR,
              'window':list(WINDOW),'fresh_deposits':[300,600],'cap':5,
              'execution':'two separate jobs; each reserves the full CPU budget, hence sequential',
              'memory':'No claim of measured RAM fit. Operator must approve memory and launch separately.',
              'publication_final_source':True,'untouched_holdout':False,
              'full_owner_and_september_sealed_until_finalists':True},
          'metadata':{'etap_badania':'Sierpień — świeże konta300/600; wybór≤5 bez września',
                      'kanal':'Synergy','tryb_obliczen':'exact','status_walidacji':'previously-seen historical validation'}}
    sizing.validate_plan_inputs(plan)
    prep.write_new(out/'PLAN.json',plan)
    receipt={'schema':SCHEMA,'status':'PREPARED_NOT_RUN','plan':prep.pin(out/'PLAN.json'),
             'helper':prep.pin(__file__),'selected_train_count':len(train['shortlist']),
             'candidate_runs':len(train['shortlist'])*2,'control_runs':4,'selector':SELECTOR,
             'inputs':inputs,'no_process_launched':True}
    prep.write_new(out/'PREPARATION_RECEIPT.json',receipt)
    return receipt


def verify_validation_config(record, expected):
    checked_pin(record['file'])
    require(prep.exact_equal(record['source'], expected['file']), 'Config source changed')
    actual = prep.read(record['file']['path'])
    require(prep.fingerprint(actual['settings']) == record['settings_sha256'] == expected['settings_sha256'],
            'Validation Settings SHA changed')
    require(prep.exact_equal(actual, prep.read(record['source']['path'])), 'Validation config altered')


def validation_contract(validation):
    root=Path(validation).resolve(); path=root/'PLAN.json'; plan=prep.read(path)
    inputs=plan_inputs(plan)
    require(inputs.get(str(Path(__file__).resolve()))==prep.pin(__file__), 'Finalist helper changed since preparation')
    contract=plan['finalists_contract']
    require(contract['schema']==SCHEMA and prep.exact_equal(contract['selector'],SELECTOR), 'Validation selector contract changed')
    for key in ('train_plan','train_selection','shortlist'): checked_pin(contract[key])
    train=train_contract(Path(contract['train_plan']['path']).parent)
    require(contract['selected_train_ids']==[r['id'] for r in train['shortlist']], 'Validation list differs from frozen TRAIN shortlist')
    registry=contract['registry']; expected={r['id']:r for r in train['shortlist']};expected.update(train['controls'])
    require(set(registry)==set(expected), 'Validation candidate/control set changed')
    for name,record in registry.items():
        verify_validation_config(record,expected[name])
    train_job=next(j for j in train['plan']['jobs'] if j['id']=='sizing300')
    require(prep.exact_equal(plan['jobs'],make_jobs(train_job['argv'],root,len(registry),plan['threads'])), 'Validation argv/allocation changed')
    require(plan['source_revision']==train['plan']['source_revision'], 'Validation source revision changed')
    require(contract['window']==list(WINDOW) and contract['fresh_deposits']==[300,600]
            and contract['cap']==5 and contract['full_owner_and_september_sealed_until_finalists'] is True,
            'Validation scope changed')
    preparation=prep.read(root/'PREPARATION_RECEIPT.json')
    require(preparation['status']=='PREPARED_NOT_RUN' and preparation['plan']==prep.pin(path)
            and preparation['helper']==prep.pin(__file__) and preparation['selector']==SELECTOR,
            'Validation plan is not the prepared immutable plan')
    return root,plan,inputs,contract,train


def select_rows(train_rows, august300, august600, names):
    evaluated=[]
    for name in names:
        t,a,b=train_rows[name],august300[name],august600[name]
        require(t['settings_sha256']==a['settings_sha256']==b['settings_sha256'], 'Candidate settings changed between accounts')
        failures={label:rank.reasons(row,baseline) for label,row,baseline in
                  [('august300',a,august300[CONTROL_IDS[0]]),('august600',b,august600[CONTROL_IDS[0]])]}
        eligible=not any(failures.values())
        key=None
        if eligible:
            key=[max(math.ceil(r['max_dd_pct']/2.5) for r in (t,a,b)),
                 -min(a['positive_market_days_pct'],b['positive_market_days_pct']),
                 -min(a['median_weekly_return'],b['median_weekly_return']),
                 -min(a['log_growth'],b['log_growth']),t['settings_sha256']]
        evaluated.append({'id':name,'settings_sha256':t['settings_sha256'],'eligible':eligible,
                          'rejections':failures,'rank_key':key,'train':t,'august300':a,'august600':b})
    chosen=sorted((r for r in evaluated if r['eligible']),key=lambda r:r['rank_key'])[:5]
    return {'selected':chosen,'all_shortlisted':evaluated,'eligible_count':sum(r['eligible'] for r in evaluated),
            'status':'FINALISTS_FROZEN' if chosen else 'NO_QUALIFYING_FINALIST',
            'selection_uses_september_or_full_owner':False,'gates_relaxed':False,'untouched_holdout':False}


def blueprint(selected, train_shortlist):
    return {'schema':'conduit.sizing-finalists.followup-blueprint.v1',
            'status':'BLUEPRINT_NOT_RUNNABLE' if selected else 'NO_FINALISTS',
            'finalists':selected,'selection_changes_allowed':False,
            'full_owner':{'window':['2026-06-20','2026-09-02'],'deposits':[600],'caps':[5,10,.01],
                          'comparison_candidates':train_shortlist,'only_after_august_selection_frozen':True,
                          'used_to_select':False},
            'lotto':{'start':'every observed market-day start','end_exclusive':'2026-09-02',
                     'deposits':[300,600],'caps':[.01,10,'no_application_cap'],'broker_volume_max':100,
                     'independent_accounts':True,'not_single_day_windows':True},
            'fresh_resets':{'calendar_windows':['day','week','month'],'deposits':[300,600],
                            'caps':[.01,10,'no_application_cap'],'broker_volume_max':100},
            'september':{'window':['2026-09-01','2026-09-13'],'previously_seen_transfer_validation':True,
                         'used_to_select':False,'untouched_holdout':False},
            'prerequisites':['Bind observed market-day calendar and every explicit window before execution.',
                'LOTTO is each day-start to common end, distinct from independent daily/week/month accounts.',
                'Prove no-app-cap encoding, broker maximum100 and all remaining risk/margin limits in actual BTP recipe.',
                'Only lot_max, starting balance/window and output paths may vary; no strategy retuning.',
                'Keep no-session/missing-data distinct; preserve all attempted windows and failures.',
                'Reserve measured memory/CPU budget and bind progress monitor before a separately authorized launch.']}


def select(validation, output):
    out=new_output(output)
    root,plan,inputs,contract,train=validation_contract(validation)
    accounts={};evidence=[]
    for job in plan['jobs']:
        rows,pins=completed_job(root/'PLAN.json',plan,job,contract['registry'],inputs)
        accounts[job['id']]=rows;evidence+=pins
    result=select_rows(train['rows'],accounts['august_300'],accounts['august_600'],contract['selected_train_ids'])
    configs=[train['registry'][r['id']] for r in result['selected']]
    result.update(schema=FINAL_SCHEMA,selector=SELECTOR,validation_plan=prep.pin(root/'PLAN.json'),
                  preparation=prep.pin(root/'PREPARATION_RECEIPT.json'),helper=prep.pin(__file__),
                  evidence=unique_pins(train['evidence']+evidence),selected_configurations=configs,
                  limitation='Historical publication-final source, not a complete live edit chronology; no untouched holdout.')
    out.mkdir(parents=True)
    prep.write_new(out/'FINAL_SELECTION.json',result)
    prep.write_new(out/'FINALISTS.json',configs)
    prep.write_new(out/'FOLLOWUP_BLUEPRINT.json',blueprint([r['id'] for r in configs],contract['selected_train_ids']))
    prep.write_new(out/'SELECTION_RECEIPT.json',{'schema':FINAL_SCHEMA,'status':result['status'],
        'selection':prep.pin(out/'FINAL_SELECTION.json'),'finalists':prep.pin(out/'FINALISTS.json'),
        'blueprint':prep.pin(out/'FOLLOWUP_BLUEPRINT.json'),'helper':prep.pin(__file__),
        'evidence':result['evidence'],'no_process_launched':True})
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    commands=parser.add_subparsers(dest='command',required=True)
    prepare_args=commands.add_parser('prepare',help='Prepare two fresh August jobs from the completed TRAIN shortlist')
    prepare_args.add_argument('--study',required=True,type=Path)
    prepare_args.add_argument('--output',required=True,type=Path)
    prepare_args.add_argument('--cpu-budget',type=int,default=12)
    select_args=commands.add_parser('select',help='Freeze at most 5 finalists from completed August300/600 accounts')
    select_args.add_argument('--validation',required=True,type=Path)
    select_args.add_argument('--output',required=True,type=Path)
    args=parser.parse_args()
    result=prepare(args.study,args.output,args.cpu_budget) if args.command=='prepare' else select(args.validation,args.output)
    print(result['status'])


if __name__=='__main__': main()
