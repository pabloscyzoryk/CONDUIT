"""Prepare exactly 300 sizing-only GOD-X7 experiments; never deploy a preset.

Inputs are explicit local paths. Private prices/messages and results are not
copied into the public source tree. Run with research_runner_sizing.py.
"""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
import itertools
import random
import math
from collections import Counter
from pathlib import Path, PureWindowsPath
import subprocess

ANCHORS = (600, 1000, 1500, 2500)
SHAPES = {
    'Power': ('lot_growth_power', (.4, .55, .7, .85, 1.)),
    'ThresholdLinear': ('lot_growth_rate_pct', (.15, .25, .35, .5, .75)),
    'GeometricSteps': ('lot_growth_capital_multiple', (1.5, 1.75, 2., 2.5, 3.)),
}
ALLOCATIONS = ('Uniform', 'EqualSLRisk', 'Depth', 'EqualSLRiskDepth', 'ExposureAwareRisk')
NEW_FIELDS = {
    'lot_growth_mode', 'lot_growth_reference_lot', 'lot_growth_reference_balance',
    'lot_growth_power', 'lot_growth_rate_pct', 'lot_growth_capital_multiple',
    'lot_growth_lot_multiple', 'lot_growth_basket_risk_pct', 'lot_growth_allocation',
}
STRESS_FIELDS = tuple('lot_growth_'+name+'_strength' for name in (
    'equity_stress', 'portfolio_load', 'direction_load', 'basket_count', 'spread_stress',
    'tp1_deficit', 'stop_width', 'age_decay', 'rearm_decay', 'day_dd'))
NEW_FIELDS.update(STRESS_FIELDS)
INTERLEAVE_SEED = 915012

def stress_assignments():
    """150 B arms: 20 single-axis, 65 two-axis, 65 three-axis combinations."""
    rng=random.Random(INTERLEAVE_SEED)
    exposure=[0]*10; level_counts=[Counter() for _ in range(10)]
    assignments=[]; used=Counter()
    groups=[(i,) for i in range(10)]*2
    for n in range(130):
        size=2+n%2
        combinations=list(itertools.combinations(range(10),size))
        rng.shuffle(combinations)
        group=min(combinations,key=lambda c:(
            max(exposure[i]+(i in c) for i in range(10)),
            sum((exposure[i]+(i in c))**2 for i in range(10)),used[c]))
        groups.append(group);used[group]+=1
        for i in group:exposure[i]+=1
    # A balanced strength schedule is determined before any replay result exists.
    for group in groups:
        strengths={f:0. for f in STRESS_FIELDS}
        for axis in group:
            levels=[.25,.5,1.];rng.shuffle(levels)
            level=min(levels,key=lambda x:level_counts[axis][x])
            level_counts[axis][level]+=1
            strengths[STRESS_FIELDS[axis]]=level
        assignments.append(strengths)
    rng.shuffle(assignments)
    return assignments

def read(path): return json.loads(Path(path).read_text('utf-8-sig'))
def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'),
                                    allow_nan=False).encode()).hexdigest()
def pin(path):
    path = Path(path).resolve()
    with path.open('rb') as stream: digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'path': str(path), 'bytes': path.stat().st_size, 'sha256': digest}

def exact_equal(left, right):
    return fingerprint(left) == fingerprint(right)

def settings_equal(left, right):
    """JSON integers and floats represent the same numeric setting; booleans do not."""
    if type(left) in (int,float) and type(right) in (int,float):
        return math.isfinite(left) and math.isfinite(right) and left == right
    if type(left) is not type(right):
        return False
    if isinstance(left,dict):
        return left.keys() == right.keys() and all(settings_equal(left[k],right[k]) for k in left)
    if isinstance(left,list):
        return len(left) == len(right) and all(settings_equal(a,b) for a,b in zip(left,right))
    return left == right

def verify_pin(record):
    actual = pin(record['path'])
    if not exact_equal(actual, record):
        raise ValueError('Pinned artifact changed: ' + Path(record['path']).name)
    return actual

def verify_build(exe, receipt_path):
    """A label or agreement between unrelated files is not a build proof."""
    receipt = read(receipt_path)
    if (receipt.get('schema') != 'conduit.sizing-instrument.v1'
            or receipt.get('status') != 'PASS'
            or receipt.get('source_revalidated_after') is not True):
        raise ValueError('Expected a completed sizing instrument build')
    expected = receipt.get('binaries', {}).get('btp.exe')
    if not expected or not exact_equal(pin(exe), expected):
        raise ValueError('Executable does not belong to the qualified build')
    verify_pin(receipt['source_manifest'])
    verify_pin(receipt['log'])
    source = Path(receipt['source']).resolve()
    rows = read(receipt['source_manifest']['path'])
    if not isinstance(rows, list) or not rows:
        raise ValueError('Empty frozen source manifest')
    seen = set()
    for row in rows:
        relative = PureWindowsPath(row['path'])
        if relative.drive or relative.root or '..' in relative.parts or not relative.parts:
            raise ValueError('Unsafe frozen source path')
        name = relative.as_posix()
        if name in seen:
            raise ValueError('Duplicate frozen source path')
        seen.add(name)
        path = source.joinpath(*relative.parts).resolve()
        if not path.is_relative_to(source) or pin(path)['sha256'] != row['sha256']:
            raise ValueError('Frozen source changed')
    return receipt

def option(argv, flag):
    if argv.count(flag) != 1 or argv.index(flag)+1 >= len(argv):
        raise ValueError('Expected one explicit ' + flag)
    return argv[argv.index(flag)+1]
def write_new(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x', encoding='utf-8') as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False, allow_nan=False)
        stream.write('\n')

def materialize(base, defaults):
    if set(base['settings']) - set(defaults):
        raise ValueError('The new binary does not understand every baseline setting')
    if not NEW_FIELDS <= set(defaults):
        raise ValueError('Build lacks the complete sizing API')
    result = copy.deepcopy(base)
    result['settings'] = {**defaults, **base['settings']}
    s = result['settings']
    if (s['lot_base'] != 'Balance' or s.get('ea_enabled') is not False
            or s.get('t100', {}).get('enabled', False) is not False
            or s.get('lot_growth_mode') != 'Off'):
        raise ValueError('Expected unchanged AUTO GOD-X7, Balance basis, sizing feature OFF')
    return result

def verify_reference(reference, base, defaults):
    if (reference.get('schema') != 'conduit.g7.lot-growth.owner-reference.v1'
            or reference.get('status') != 'PASS_REVALIDATED_REFERENCE_ONLY'):
        raise ValueError('Expected the qualified historical GOD-X7 reference')
    config = reference['cases'][1]['config']
    verify_pin(config)
    expected = materialize(read(config['path']), defaults)
    if not settings_equal(base['settings'], expected['settings']):
        raise ValueError('Baseline changes an existing GOD-X7 strategy setting')

def candidates(base):
    """No source-selection, entry, SL, TP, rearm or existing risk-axis changes."""
    bases = []
    flat_index=0
    for mode_index, (mode, (shape_field, values)) in enumerate(SHAPES.items()):
        for shape_index, shape in enumerate(values):
            rotation=(mode_index*5+shape_index)%4
            rotated=ANCHORS[rotation:]+ANCHORS[:rotation]
            for anchor in rotated:
                for allocation in ALLOCATIONS:
                    take=flat_index%2==0;flat_index+=1
                    if not take:continue
                    doc = copy.deepcopy(base)
                    s = doc['settings']
                    s.update(lot_max=5., lot_growth_mode=mode,
                             lot_growth_reference_lot=.01,
                             lot_growth_reference_balance=anchor,
                             lot_growth_power=.7, lot_growth_rate_pct=.35,
                             lot_growth_capital_multiple=2., lot_growth_lot_multiple=1.5,
                             lot_growth_basket_risk_pct=0., lot_growth_allocation=allocation)
                    s[shape_field] = shape
                    s.update({f:0. for f in STRESS_FIELDS})
                    bases.append((doc,mode,shape,anchor,allocation))
    if len(bases)!=150:raise ValueError('Expected 150 paired base configurations')
    records=[]
    for i,((base_doc,mode,shape,anchor,allocation),strengths) in enumerate(zip(bases,stress_assignments())):
        for arm in ('A','B'):
            doc=copy.deepcopy(base_doc);identity=f'G8-sizing-{i+1:03d}{arm}'
            doc['name']=identity;s=doc['settings']
            if arm=='B':s.update(strengths)
            changes={k:v for k,v in s.items() if not exact_equal(v,base['settings'].get(k))}
            if set(changes)-NEW_FIELDS-{'lot_max'}:raise ValueError('A non-sizing setting changed')
            records.append({'id':identity,'pair':i+1,'arm':arm,'preset':doc,
                            'settings_sha256':fingerprint(s),'changes':changes,
                            'curve':mode,'shape':shape,'anchor':anchor,'allocation':allocation,
                            'active_stress_axes':[f for f in STRESS_FIELDS if s[f]>0]})
    if len(records) != 300 or len({x['settings_sha256'] for x in records}) != 300:
        raise ValueError('The experiment requires exactly 300 distinct configurations')
    return records

def recipe(reference, exe, cfgdir, out, start, end, balance=600):
    argv = list(reference['cases'][1]['argv_exact'])
    argv[0] = str(Path(exe).resolve())
    for flag, value in {'--sweep': cfgdir, '--out': out, '--from': start,
                        '--to': end, '--balance': balance}.items():
        option(argv, flag)
        argv[argv.index(flag)+1] = str(value)
    # Exact source rows; chart storage resolution has no effect on DD metrics.
    if '--quick-tick-stride' not in argv: argv.extend(['--quick-tick-stride', '1'])
    if option(argv, '--quick-tick-stride') != '1':
        raise ValueError('Sizing qualification requires every tick')
    if '--dump-trades' not in argv or any(flag in argv for flag in
            ('--quick-sweep', '--daily-reset', '--auto-ea', '--chain')):
        raise ValueError('Expected exact compound AUTO GOD-X7 with detailed ledgers')
    return argv

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ('exe', 'baseline', 'reference-binding', 'build-receipt', 'methods', 'output'):
        parser.add_argument('--'+flag, required=True, type=Path)
    args = parser.parse_args()
    out = args.output.resolve()
    if out.exists(): raise FileExistsError(out)
    verify_build(args.exe.resolve(), args.build_receipt)
    defaults = json.loads(subprocess.check_output([str(args.exe.resolve()), '--dump-settings'],
                                                creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0),
                                                timeout=60))
    verify_build(args.exe.resolve(), args.build_receipt)
    base = materialize(read(args.baseline), defaults)
    reference = read(args.reference_binding)
    verify_reference(reference, base, defaults)
    records = candidates(base)
    out.mkdir(parents=True)
    write_new(out/'DEFAULTS.json', defaults)
    write_new(out/'BASELINE_MATERIALIZED.json', base)
    inputs = [pin(p) for p in (args.exe, args.baseline, args.reference_binding,
                              args.build_receipt, args.methods, Path(__file__),
                              Path(__file__).with_name('research_runner.py'),
                              Path(__file__).with_name('research_runner_exact8.py'),
                              Path(__file__).with_name('research_runner_sizing.py'),
                              Path(__file__).with_name('rank_lot_growth_sweep.py'))]
    inputs.extend([pin(out/'DEFAULTS.json'), pin(out/'BASELINE_MATERIALIZED.json')])
    reference_config = reference['cases'][1]['config']
    if str(Path(reference_config['path']).resolve()) not in {r['path'] for r in inputs}:
        inputs.append(verify_pin(reference_config))
    for flag in ('--ticks', '--signals'):
        argv = reference['cases'][1]['argv_exact']
        inputs.append(pin(argv[argv.index(flag)+1]))
    for row in records:
        path = out/'configs'/f"{row['id']}.json"
        write_new(path, row.pop('preset'))
        row['file'] = pin(path)
        inputs.append(row['file'])
    control_ids = []
    for label, cap in [('fixed001', .01), ('cap5', 5.)]:
        doc = copy.deepcopy(base); doc['name'] = 'GOD-X7-'+label
        doc['settings']['lot_max'] = cap
        path = out/'control_configs'/(doc['name']+'.json')
        write_new(path, doc); inputs.append(pin(path)); control_ids.append(doc['name'])
    jobs = []
    for name, folder, count, threads in [('controls', 'control_configs', 2, 2),
                                          ('sizing300', 'configs', 300, 24)]:
        result = out/name/'results'
        argv = recipe(reference, args.exe, out/folder, result, '2026-06-20', '2026-08-01')
        jobs.append({'id': name, 'argv': argv, 'threads': threads,
                     'result_dir': str(result), 'expected_candidates': count})
    prereg = {
        'schema': 'conduit.sizing300.preregistered.v3', 'candidate_count': 300,
        'design': {'pairs':150,'A':'capital curve and per-entry allocation; ten extra axes OFF',
                   'B':'same capital curve/allocation; 1, 2 or 3 extra sizing axes active',
                   'seed':INTERLEAVE_SEED,'extra_axis_fields':STRESS_FIELDS,
                   'active_strength_levels':[.25,.5,1.],
                   'B_arms_by_active_axis_count':dict(Counter(len(r['active_stress_axes']) for r in records if r['arm']=='B')),
                   'capital_anchor_counts_A':dict(Counter(r['anchor'] for r in records if r['arm']=='A')),
                   'axis_exposure_counts_B':dict(Counter(f for r in records if r['arm']=='B' for f in r['active_stress_axes'])),
                   'combination_rule':'minimum of per-axis multipliers; each bounded .25..1; never their product',
                   'application':'SURPLUS only: m+(requested-m)*factor when requested>legal minimum m; below-minimum requests never raised',
                   'not_a_full_factorial':'Interleaved paired experiment; does not identify every higher-order interaction'},
        'configuration_changes': 'Only new sizing fields and common cap5; every old GOD-X7 axis fixed.',
        'capital': 600, 'max_lot': 5, 'workers': 24, 'exact_ticks': True,
        'training': {'from': '2026-06-20', 'to_exclusive': '2026-08-01'},
        'validation': {'from': '2026-08-01', 'to_exclusive': '2026-09-01', 'fresh_deposits': [300,600]},
        'full_reference': {'from': '2026-06-20', 'to_exclusive': '2026-09-02', 'capital': 600},
        'transfer': {'from': '2026-09-01', 'to_exclusive': '2026-09-13',
                     'status': 'previously_seen; sealed until finalists selected; never an untouched holdout'},
        'gates': {'complete': True, 'stopout': False, 'min_equity_gt': 0,
                  'profit_gt': 0, 'max_dd_pct_lte': 35, 'positive_market_days_pct_gte': 50,
                  'filled_baskets_fraction_of_fixed001_gte': .5,
                  'active_entry_days_fraction_of_fixed001_gte': .8},
        'selection': {'maximum': 10, 'first_five': 'ceil(DD/2.5) asc, positive fraction desc, median weekly return desc, log growth desc, SHA asc',
                      'next_five': 'highest log growth among eligible not already selected; tie first_five rank',
                      'zero_eligible': 'No qualifying candidate; diagnostic near-misses cannot be crowned.',
                      'max_finalists': 5, 'no_tuning_after_results': True,
                      'final_validation_selection':'same gates on August fresh 300 and 600, then worst DD band across TRAIN/Aug300/Aug600; min August positive fraction, median weekly return, loggrowth descending; SHA ascending',
                      'full_reference_and_september_not_used_to_select_finalists':True},
        'economic_duplicates': 'Report identical execution fingerprints; do not replace rounded duplicates with new searches.',
        'coronation': {'fresh_windows': ['full','calendar_day','calendar_week','calendar_month'],
                       'deposits': [300,600], 'caps': [.01,5,10,'no_application_cap_broker_constraints_remain'],
                       'best_day_subtraction_only_at_cap001': True},
        'baseline_controls_not_candidates': control_ids, 'candidates': records,
        'methods': pin(args.methods), 'production_preset_changed': False,
        'source_contract': 'Historical publication-final benchmark. Missing original message versions are not reconstructed.'}
    write_new(out/'PREREGISTRATION.json', prereg)
    inputs.append(pin(out/'PREREGISTRATION.json'))
    plan = {'id': 'giga_sweep8_sizing300', 'name': 'giga_sweep8 — 300 konfiguracji lotowania GOD-X7',
            'output': str(out), 'progress_dir': str(out/'progress'), 'threads': 24,
            'source_revision': pin(args.build_receipt)['sha256'], 'inputs': inputs, 'jobs': jobs,
            'qualification': {'schema':'conduit.sizing-plan-inputs.v1',
                              'build_receipt':pin(args.build_receipt),
                              'reference_binding':pin(args.reference_binding),
                              'baseline':pin(args.baseline), 'defaults':pin(out/'DEFAULTS.json'),
                              'materialized_baseline':pin(out/'BASELINE_MATERIALIZED.json'),
                              'preregistration':pin(out/'PREREGISTRATION.json')},
            'metadata': {'etap_badania': 'TRAIN 20.06–31.07; tylko wolumen poszczególnych pozycji',
                         'kanal': 'Synergy', 'kandydaci': 300, 'pary_AB':150, 'nowe_osie':10, 'kontrole': 2, 'kapital': 600,
                         'max_lot': 5, 'tryb_obliczen': 'exact',
                         'status_walidacji': 'w próbie; historyczne wersje końcowe wiadomości'}}
    write_new(out/'PLAN.json', plan)
    print(json.dumps({'prepared': 300, 'controls': 2, 'plan': str(out/'PLAN.json'),
                      'preregistration_sha256': pin(out/'PREREGISTRATION.json')['sha256']}))

if __name__ == '__main__': main()
