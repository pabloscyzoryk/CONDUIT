"""Offline synthetic fixtures only; no executable or market feed is launched."""
import copy
import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import prepare_lot_growth_sweep as prep
import rank_lot_growth_sweep as rank
import validate_lot_growth_finalists as subject


def timestamp(date):
    return int(dt.datetime.fromisoformat(date).replace(tzinfo=dt.timezone.utc).timestamp()*1000)+43_200_000


def describe_fixture(name, balance, dates, sha, dd=20, profit=100):
    daily=[{'date':date,'start_equity':balance+index*profit/2,
            'end_equity':balance+(index+1)*profit/2,'real_dd_pct':1} for index,date in enumerate(dates)]
    metrics={'start_balance':balance,'end_equity':balance+profit,'total_profit':profit,'max_dd_pct':dd,
             'min_equity':balance-20,'blown':False,'stop_outs':0,'positive_market_days_pct':100,
             'market_days':len(dates),'trades':2,'baskets':1,'profit_factor':2,'min_margin_level':1000,
             'max_open_volume':.02,'max_open_risk_pct':1}
    baskets=[{'id':1,'had_positions':True,'first_open_ts':timestamp(dates[0]),
              'warstwy':[{'filled':True,'fill_ts':timestamp(dates[1])}]}]
    ledger={'transakcje':[{'otwarcie_ms':timestamp(date),'koszyk':1} for date in dates]}
    detail={'dni':daily}
    row=rank.describe(name,metrics,detail,baskets,sha,trades=ledger)
    return metrics,detail,baskets,ledger,row


def completed(plan_path, name, registry, dates):
    plan=prep.read(plan_path); job=next(j for j in plan['jobs'] if j['id']==name)
    result=Path(job['result_dir']); result.mkdir(parents=True)
    summary={}; details={}; rows={}; balance=float(subject.flag(job['argv'],'--balance'))
    for identity,record in registry.items():
        m,d,b,t,r=describe_fixture(identity,balance,dates,record['settings_sha256'])
        d.update(preset=identity,tryb='compound',od=subject.flag(job['argv'],'--from'),
                 do=subject.flag(job['argv'],'--to'),saldo_start=balance,metryki=m)
        t.update(preset=identity,tryb='compound',od=d['od'],do=d['do'],transakcji=m['trades'])
        summary[identity]=m;rows[identity]=r
        for suffix,value in [('_compound_dane.json',d),('_compound_koszyki.json',b),('_compound_transakcje.json',t)]:
            path=result/(identity+suffix);prep.write_new(path,value);p=prep.pin(path)
            details[path.name]={'bytes':p['bytes'],'sha256':p['sha256']}
    path=result/'wyniki_compound.json';prep.write_new(path,summary)
    receipt={'id':name,'status':'complete','returncode':0,'partial':False,'validation_errors':[],
             'argv':job['argv'],'threads':job['threads'],'plan_sha256':prep.pin(plan_path)['sha256'],
             'source_revision':plan['source_revision'],'exe_sha256':prep.pin(job['argv'][0])['sha256'],
             'result_files':[str(path)],'result_sha256':{path.name:prep.pin(path)['sha256']},
             'detail_artifacts_schema':'conduit.completed-detail-artifacts.v1','detail_artifacts':details}
    digest=subject.sizing.validate_plan_inputs(plan)
    receipt.update(input_binding_schema=subject.sizing.INPUT_SCHEMA,inputs_before_sha256=digest,inputs_after_sha256=digest)
    prep.write_new(Path(plan['output'])/name/'receipt.json',receipt)
    return rows


def fixture(root):
    root=Path(root);root.mkdir()
    exe=root/'fake-never-executed.exe';exe.write_bytes(b'OFFLINE SYNTHETIC NO EXECUTION')
    ticks=root/'ticks.bin';ticks.write_bytes(b'synthetic ticks, not market evidence')
    signals=root/'signals.json';signals.write_text('{}',encoding='utf-8')
    defaults={k:0.0 for k in prep.NEW_FIELDS}
    defaults.update(lot_growth_mode='Off',lot_growth_allocation='Uniform',lot_growth_reference_lot=.01,
        lot_growth_reference_balance=1000.,lot_growth_power=.7,lot_growth_rate_pct=.35,
        lot_growth_capital_multiple=2.,lot_growth_lot_multiple=1.5,lot_max=10.,lot_base='Balance',
        ea_enabled=False,t100={'enabled':False},risk_per_basket_pct=20.,entry_units=4)
    base={'name':'GOD-X7','settings':defaults}
    source=root/'source';source.mkdir();(source/'synthetic.rs').write_text('synthetic source',encoding='utf-8')
    prep.write_new(root/'source_manifest.json',[{'path':'synthetic.rs','sha256':prep.pin(source/'synthetic.rs')['sha256']}])
    (root/'build.log').write_text('synthetic qualified build, never executed',encoding='utf-8')
    prep.write_new(root/'BUILD.json',{'schema':'conduit.sizing-instrument.v1','status':'PASS','source_revalidated_after':True,
        'source':str(source),'source_manifest':prep.pin(root/'source_manifest.json'),'log':prep.pin(root/'build.log'),
        'binaries':{'btp.exe':prep.pin(exe)}})
    prep.write_new(root/'DEFAULTS.json',defaults);prep.write_new(root/'BASELINE.json',base)
    prep.write_new(root/'BASELINE_MATERIALIZED.json',base)
    records=prep.candidates(base);registry={}
    inputs=[prep.pin(p) for p in (exe,ticks,signals)]
    inputs += [prep.pin(Path(subject.__file__).with_name(p)) for p in subject.HELPERS]
    for record in records:
        record=copy.deepcopy(record);doc=record.pop('preset');path=root/'configs'/(record['id']+'.json')
        prep.write_new(path,doc);record['file']=prep.pin(path);inputs.append(record['file']);registry[record['id']]=record
    controls={}
    for name,cap in [(subject.CONTROL_IDS[0],.01),(subject.CONTROL_IDS[1],5)]:
        doc=copy.deepcopy(base);doc['name']=name;doc['settings']['lot_max']=cap
        path=root/'control_configs'/(name+'.json');prep.write_new(path,doc);p=prep.pin(path);inputs.append(p)
        controls[name]={'id':name,'file':p,'settings_sha256':prep.fingerprint(doc['settings'])}
    prereg={'schema':'conduit.sizing300.preregistered.v3','candidate_count':300,'capital':600,'max_lot':5,
        'training':{'from':'2026-06-20','to_exclusive':'2026-08-01'},
        'validation':{'from':'2026-08-01','to_exclusive':'2026-09-01','fresh_deposits':[300,600]},
        'gates':{'complete':True,'stopout':False,'min_equity_gt':0,'profit_gt':0,'max_dd_pct_lte':35,
                 'positive_market_days_pct_gte':50,'filled_baskets_fraction_of_fixed001_gte':.5,
                 'active_entry_days_fraction_of_fixed001_gte':.8},'candidates':list(registry.values())}
    prep.write_new(root/'PREREGISTRATION.json',prereg);inputs.append(prep.pin(root/'PREREGISTRATION.json'))
    plan={'id':'synthetic_train','name':'Synthetic only','output':str(root),'progress_dir':str(root/'progress'),
          'threads':24,'source_revision':prep.pin(root/'BUILD.json')['sha256'],'inputs':inputs,'jobs':[]}
    for identity,folder,count,threads in [('sizing300','configs',300,24),('controls','control_configs',2,2)]:
        result=root/identity/'results'
        argv=[str(exe),'--ticks',str(ticks),'--signals',str(signals),'--from','2026-06-20','--to','2026-08-01',
              '--balance','600','--sweep',str(root/folder),'--out',str(result),'--quick-tick-stride','1',
              '--signal-time-offset-min','0','--rozgrzewka-h','72','--sim-price-digits','2','--dump-trades']
        plan['jobs'].append({'id':identity,'argv':argv,'threads':threads,'expected_candidates':count,'result_dir':str(result)})
    prep.write_new(root/'REFERENCE.json',{'schema':'conduit.g7.lot-growth.owner-reference.v1',
        'status':'PASS_REVALIDATED_REFERENCE_ONLY',
        'cases':[{}, {'argv_exact':plan['jobs'][0]['argv'],'config':prep.pin(root/'BASELINE.json')}]})
    plan['qualification']={'schema':'conduit.sizing-plan-inputs.v1','build_receipt':prep.pin(root/'BUILD.json'),
        'reference_binding':prep.pin(root/'REFERENCE.json'),'baseline':prep.pin(root/'BASELINE.json'),
        'defaults':prep.pin(root/'DEFAULTS.json'),'materialized_baseline':prep.pin(root/'BASELINE_MATERIALIZED.json'),
        'preregistration':prep.pin(root/'PREREGISTRATION.json')}
    plan['inputs'] += [r for k,r in plan['qualification'].items() if k not in ('schema','preregistration')]
    plan_path=root/'PLAN.json';prep.write_new(plan_path,plan)
    rows=completed(plan_path,'sizing300',registry,['2026-06-22','2026-06-23'])
    refs=completed(plan_path,'controls',controls,['2026-06-22','2026-06-23'])
    chosen=rank.select(list(rows.values()),refs[subject.CONTROL_IDS[0]])
    chosen.update(schema='conduit.sizing300.train-selection.v1',baseline={**refs[subject.CONTROL_IDS[0]],'settings_sha256':'reference'},
                  preregistration=prep.pin(root/'PREREGISTRATION.json'),plan=prep.pin(root/'PLAN.json'),
                  completion_receipts=[prep.pin(root/job/'receipt.json') for job in ('controls','sizing300')])
    prep.write_new(root/'TRAIN_SELECTION.json',chosen)
    prep.write_new(root/'SHORTLIST.json',[registry[r['id']] for r in chosen['selected']])
    return root


class FinalistTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup);self.root=Path(self.tmp.name)

    def test_cli_help_works_with_windows_cp1250(self):
        env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',PYTHONIOENCODING='cp1250')
        result=subprocess.run([sys.executable,subject.__file__,'--help'],env=env,
                              stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=20,check=False)
        self.assertEqual(result.returncode,0,result.stderr.decode('cp1250'))
        self.assertIn(b'Freeze at most 5 finalists',result.stdout)

    def test_prepare_then_select_uses_actual_bound_artifact_paths(self):
        train=fixture(self.root/'train');out=self.root/'august'
        result=subject.prepare(train,out)
        self.assertEqual(result['status'],'PREPARED_NOT_RUN');self.assertEqual(result['candidate_runs'],20)
        plan=prep.read(out/'PLAN.json');registry=plan['finalists_contract']['registry']
        self.assertEqual(len(registry),12)
        self.assertEqual([j['threads'] for j in plan['jobs']],[12,12]);self.assertEqual(plan['threads'],12)
        for job in plan['jobs']:
            completed(out/'PLAN.json',job['id'],registry,['2026-08-03','2026-08-04'])
        selection=subject.select(out,self.root/'final')
        self.assertEqual(selection['status'],'FINALISTS_FROZEN');self.assertEqual(len(selection['selected']),5)
        self.assertFalse(selection['selection_uses_september_or_full_owner'])
        blueprint=prep.read(self.root/'final/FOLLOWUP_BLUEPRINT.json')
        self.assertEqual(blueprint['status'],'BLUEPRINT_NOT_RUNNABLE')
        self.assertEqual(len(blueprint['full_owner']['comparison_candidates']),10)
        self.assertEqual(blueprint['lotto']['deposits'],[300,600])

    def test_argv_only_changes_window_deposit_sweep_and_output(self):
        argv=['/fake.exe','--from','2026-06-20','--to','2026-08-01','--balance','600','--sweep','/old',
              '--out','/oldout','--signals','/unchanged','--signal-time-offset-min','0','--rozgrzewka-h','72']
        jobs=subject.make_jobs(argv,self.root/'new',12,8)
        allowed={argv.index(k)+1 for k in ('--from','--to','--balance','--sweep','--out')}
        for job in jobs:
            self.assertEqual(len(job['argv']),len(argv))
            self.assertTrue(all(a==b or i in allowed for i,(a,b) in enumerate(zip(argv,job['argv']))))
            self.assertEqual(subject.flag(job['argv'],'--to'),'2026-09-01')
        self.assertEqual(argv[2],'2026-06-20')

    def test_shortlist_substitution_is_rejected_before_writes(self):
        train=fixture(self.root/'train');path=train/'SHORTLIST.json';rows=prep.read(path)
        rows.reverse();path.write_text(json.dumps(rows),encoding='utf-8')
        with self.assertRaisesRegex(ValueError,'SHORTLIST'):subject.prepare(train,self.root/'no_output')
        self.assertFalse((self.root/'no_output').exists())

    def test_wrong_receipt_plan_is_rejected(self):
        train=fixture(self.root/'train');path=train/'sizing300/receipt.json';value=prep.read(path)
        value['plan_sha256']='0'*64;path.write_text(json.dumps(value),encoding='utf-8')
        with self.assertRaisesRegex(ValueError,'plan_sha256'):subject.prepare(train,self.root/'no_output')

    def test_tampered_bound_execution_ledger_is_rejected(self):
        train=fixture(self.root/'train');path=next((train/'sizing300/results').glob('*_transakcje.json'))
        path.write_text('{}',encoding='utf-8')
        with self.assertRaisesRegex(ValueError,'changed after completion'):subject.prepare(train,self.root/'no_output')

    def test_unchanged_counts_do_not_hide_extra_sweep_file(self):
        train=fixture(self.root/'train');(train/'configs/extra.json').write_text('{}',encoding='utf-8')
        with self.assertRaisesRegex(ValueError,'configuration file set|extra files'):subject.prepare(train,self.root/'no_output')

    def test_repaired_artifact_hash_does_not_hide_wrong_run_metadata(self):
        train=fixture(self.root/'train')
        path=next((train/'sizing300/results').glob('*_compound_dane.json'))
        value=prep.read(path);value['saldo_start']=300
        path.write_text(json.dumps(value),encoding='utf-8')
        receipt_path=train/'sizing300/receipt.json';receipt=prep.read(receipt_path);p=prep.pin(path)
        receipt['detail_artifacts'][path.name]={'bytes':p['bytes'],'sha256':p['sha256']}
        receipt_path.write_text(json.dumps(receipt),encoding='utf-8')
        with self.assertRaisesRegex(ValueError,'Detailed result'):
            subject.prepare(train,self.root/'no_output')
        self.assertFalse((self.root/'no_output').exists())

    def test_missing_before_after_input_proof_is_rejected(self):
        train=fixture(self.root/'train');path=train/'sizing300/receipt.json';receipt=prep.read(path)
        receipt.pop('inputs_after_sha256');path.write_text(json.dumps(receipt),encoding='utf-8')
        with self.assertRaisesRegex(ValueError,'before/after'):
            subject.prepare(train,self.root/'no_output')

    def test_zero_activity_baseline_cannot_pass_a_candidate(self):
        r=describe_fixture('control',600,['2026-08-03','2026-08-04'],'sha')[-1]
        r['filled_baskets']=0
        with self.assertRaisesRegex(ValueError,'denominator'):subject.check_row(r,subject.WINDOW,600,True)

    def test_validation_output_and_plan_are_immutable(self):
        train=fixture(self.root/'train');out=self.root/'august';subject.prepare(train,out)
        with self.assertRaisesRegex(ValueError,'already exists'):subject.prepare(train,out)
        path=out/'PLAN.json';plan=prep.read(path);plan['jobs'][0]['argv'][plan['jobs'][0]['argv'].index('--to')+1]='2026-09-02'
        path.write_text(json.dumps(plan),encoding='utf-8')
        with self.assertRaisesRegex(ValueError,'argv'):subject.validation_contract(out)

    def test_final_rank_uses_worst_dd_then_minimum_august_metrics_and_sha(self):
        names=['best','bad','gain'];train={};a={};b={}
        for name in names+list(subject.CONTROL_IDS):
            train[name]=describe_fixture(name,600,['2026-06-22','2026-06-23'],name)[-1]
            a[name]=describe_fixture(name,300,['2026-08-03','2026-08-04'],name)[-1]
            b[name]=describe_fixture(name,600,['2026-08-03','2026-08-04'],name)[-1]
        b['bad']['max_dd_pct']=36
        a['gain']['log_growth']=100;b['gain']['log_growth']=100
        train['gain']['max_dd_pct']=30
        result=subject.select_rows(train,a,b,names)
        self.assertEqual([r['id'] for r in result['selected']],['best','gain'])
        self.assertFalse(result['all_shortlisted'][1]['eligible'])

    def test_no_candidate_is_promoted_when_validation_fails(self):
        r=describe_fixture('fail',300,['2026-08-03','2026-08-04'],'same')[-1]
        t={**r,'start_balance':600};r['total_profit']=-1
        a={'fail':r,subject.CONTROL_IDS[0]:{**r,'total_profit':10}}
        result=subject.select_rows({'fail':t},a,copy.deepcopy(a),['fail'])
        self.assertEqual(result['status'],'NO_QUALIFYING_FINALIST');self.assertEqual(result['selected'],[])
        self.assertEqual(subject.blueprint([],['fail'])['status'],'NO_FINALISTS')

    def test_bad_numeric_flags_and_duplicate_arguments_fail_closed(self):
        with self.assertRaises(ValueError):subject.make_jobs([],self.root,2,True)
        with self.assertRaises(ValueError):subject.flag(['--balance','300','--balance','600'],'--balance')
        r=describe_fixture('bad',600,['2026-08-03','2026-08-04'],'sha')[-1];r['stop_outs']=False
        with self.assertRaisesRegex(ValueError,'count'):subject.check_row(r,subject.WINDOW,600)

    def test_validation_recomputes_settings_sha_and_rejects_bool_numeric_alias(self):
        source=self.root/'source.json';target=self.root/'copy.json'
        original={'name':'candidate','settings':{'enabled':False,'units':1}}
        prep.write_new(source,original)
        expected={'file':prep.pin(source),'settings_sha256':prep.fingerprint(original['settings'])}
        for changes in ({'enabled':0},{'units':1.0}):
            altered=copy.deepcopy(original);altered['settings'].update(changes)
            self.assertEqual(altered,original) # Python equality alone would accept both.
            target.write_text(json.dumps(altered),encoding='utf-8')
            record={'file':prep.pin(target),'source':expected['file'],'settings_sha256':expected['settings_sha256']}
            with self.assertRaisesRegex(ValueError,'Settings SHA'):
                subject.verify_validation_config(record,expected)


if __name__=='__main__':unittest.main()
