"""Exact runner with completion-bound basket and execution ledgers for sizing."""
from pathlib import Path
import re
import sys
import research_runner_exact8 as exact
from prepare_lot_growth_sweep import (read, pin, verify_pin, verify_build, fingerprint,
                                      exact_equal, option, recipe, materialize, verify_reference)

INPUT_SCHEMA = 'conduit.sizing-completed-inputs.v1'

def validate_plan_inputs(plan):
    """Shared by execution and result readers; windows remain plan-specific."""
    records = plan['inputs']
    inputs = {}
    for record in records:
        current = verify_pin(record)
        if current['path'] in inputs:
            raise ValueError('Duplicate input path')
        inputs[current['path']] = current
    for name in ('research_runner.py', 'research_runner_exact8.py',
                 'research_runner_sizing.py', 'prepare_lot_growth_sweep.py',
                 'rank_lot_growth_sweep.py'):
        current = pin(Path(__file__).with_name(name))
        if not exact_equal(inputs.get(current['path']), current):
            raise ValueError('Plan must bind current research helpers')
    q = plan['qualification']
    if q.get('schema') != 'conduit.sizing-plan-inputs.v1':
        raise ValueError('Missing sizing input qualification')
    for key in ('build_receipt','reference_binding','baseline','defaults',
                'materialized_baseline','preregistration'):
        record = q[key]
        if not exact_equal(inputs.get(str(Path(record['path']).resolve())), record):
            raise ValueError('Qualification is outside the bound inputs: ' + key)
    if plan['source_revision'] != q['build_receipt']['sha256']:
        raise ValueError('Source revision does not identify this build receipt')
    expected_base = materialize(read(q['baseline']['path']), read(q['defaults']['path']))
    if not exact_equal(read(q['materialized_baseline']['path']), expected_base):
        raise ValueError('Materialized baseline differs from qualified settings')
    reference = read(q['reference_binding']['path'])
    verify_reference(reference, expected_base, read(q['defaults']['path']))
    reference_config = reference['cases'][1]['config']
    if not exact_equal(inputs.get(str(Path(reference_config['path']).resolve())),reference_config):
        raise ValueError('Historical GOD-X7 settings are not bound')
    output = Path(plan['output']).resolve()
    jobs = plan['jobs']
    if not jobs or len({j['id'] for j in jobs}) != len(jobs):
        raise ValueError('Missing or duplicate jobs')
    checked_exes = set()
    for job in jobs:
        if not re.fullmatch(r'[A-Za-z0-9_-]+',job['id']):
            raise ValueError('Unsafe job identifier')
        if (type(job['threads']) is not int or type(plan['threads']) is not int
                or not 1 <= job['threads'] <= plan['threads'] <= 24):
            raise ValueError('Invalid shared CPU budget')
        argv = job['argv']
        executable = str(Path(argv[0]).resolve())
        if executable not in inputs:
            raise ValueError('Job executable is not bound')
        if executable not in checked_exes:
            verify_build(executable, q['build_receipt']['path'])
            checked_exes.add(executable)
        result = Path(job['result_dir']).resolve()
        if result != output/job['id']/'results' or Path(option(argv,'--out')).resolve() != result:
            raise ValueError('Result directory does not belong to its job')
        configs = Path(option(argv,'--sweep')).resolve()
        if not configs.is_relative_to(output):
            raise ValueError('Configuration directory is outside this study')
        if any(configs == output/other['id'] for other in jobs):
            raise ValueError('Sweep configs must not share a directory with execution receipts')
        files = sorted(configs.glob('*.json'))
        if len(files) != job['expected_candidates'] or not files:
            raise ValueError('Unexpected configuration file set')
        for path in files:
            if str(path.resolve()) not in inputs:
                raise ValueError('Unbound configuration in sweep directory')
        for flag in ('--ticks','--signals'):
            if str(Path(option(argv,flag)).resolve()) not in inputs:
                raise ValueError('Market input is not bound')
        expected = recipe(reference, executable, configs, result, option(argv,'--from'),
                          option(argv,'--to'), option(argv,'--balance'))
        if not exact_equal(argv, expected):
            raise ValueError('Job changed the qualified replay recipe')
    return fingerprint(sorted(inputs.values(), key=lambda r:r['path']))

def verified_job(root, identity):
    root = Path(root).resolve()
    plan_path = root/'PLAN.json'
    plan = read(plan_path)
    if Path(plan['output']).resolve() != root:
        raise ValueError('Study path differs from its plan')
    inputs_digest = validate_plan_inputs(plan)
    matches = [j for j in plan['jobs'] if j['id'] == identity]
    if len(matches) != 1:
        raise ValueError('Unknown result job')
    job = matches[0]
    receipt = read(root/identity/'receipt.json')
    if (receipt.get('status') != 'complete' or receipt.get('returncode') != 0
            or receipt.get('partial') is not False or receipt.get('validation_errors') != []
            or receipt.get('id') != identity
            or receipt.get('plan_sha256') != pin(plan_path)['sha256']
            or not exact_equal(receipt.get('argv'),job['argv'])
            or receipt.get('exe_sha256') != pin(job['argv'][0])['sha256']
            or receipt.get('source_revision') != plan['source_revision']
            or receipt.get('threads') != job['threads']):
        raise ValueError('Completion receipt does not identify this execution plan')
    if (receipt.get('input_binding_schema') != INPUT_SCHEMA
            or receipt.get('inputs_before_sha256') != inputs_digest
            or receipt.get('inputs_after_sha256') != inputs_digest):
        raise ValueError('Missing or stale before/after input proof')
    expected_result = str(Path(job['result_dir'])/'wyniki_compound.json')
    if receipt.get('result_files') != [expected_result]:
        raise ValueError('Receipt points at another result file')
    return plan, job, receipt

def main():
    if len(sys.argv)!=2: raise ValueError('Expected one immutable plan')
    plan=read(sys.argv[1])
    inputs_before = validate_plan_inputs(plan)
    root = Path(plan['output']).resolve()
    if Path(sys.argv[1]).resolve() != root/'PLAN.json':
        raise ValueError('Expected the study PLAN.json')
    for job in plan['jobs']:
        receipt_path = root/job['id']/'receipt.json'
        if receipt_path.exists() and read(receipt_path).get('status') == 'complete':
            verified_job(root, job['id'])
        elif Path(job['result_dir']).exists() and any(Path(job['result_dir']).iterdir()):
            raise ValueError('Refusing to reuse incomplete or unreceipted result files')
    original=exact.detail_manifest
    def with_ledgers(files):
        result=original(files)
        for summary in files:
            for name,metrics in read(summary).items():
                suffixes=['_compound_koszyki.json']
                if metrics['trades']>0 or (summary.parent/(name+'_compound_transakcje.json')).exists():
                    suffixes.append('_compound_transakcje.json')
                for suffix in suffixes:
                    path=summary.parent/(name+suffix)
                    record=pin(path)
                    result[path.name]={'sha256':record['sha256'],'bytes':record['bytes']}
        return result
    original_validate, original_write = exact.runner.validate_results, exact.runner.write_json
    def validate(out, expected_count=None):
        files, errors = original_validate(out, expected_count)
        try:
            if validate_plan_inputs(plan) != inputs_before:
                raise ValueError('Input set changed during execution')
        except (OSError, ValueError, KeyError, TypeError) as exc:
            errors.append('Completion input verification failed: ' + str(exc))
        return files, errors
    def write(path, payload):
        if path.name == 'receipt.json' and payload.get('status') == 'complete':
            payload['input_binding_schema'] = INPUT_SCHEMA
            payload['inputs_before_sha256'] = inputs_before
            payload['inputs_after_sha256'] = inputs_before
        original_write(path, payload)
    exact.detail_manifest=with_ledgers
    exact.runner.validate_results, exact.runner.write_json = validate, write
    try:return exact.main()
    finally:
        exact.detail_manifest=original
        exact.runner.validate_results, exact.runner.write_json = original_validate, original_write

if __name__=='__main__':raise SystemExit(main())
