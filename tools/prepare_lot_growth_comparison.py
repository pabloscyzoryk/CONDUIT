"""Prepare the full owner comparison after the August selection is frozen.

  python tools/prepare_lot_growth_comparison.py full --final-selection FINAL --output FULL
  python tools/prepare_lot_growth_comparison.py check --study FULL
  python tools/research_runner_sizing.py FULL/PLAN.json

Only the last, separately authorized command launches research. This generator
recomputes TRAIN and August qualification from their bound completed artifacts.
It compares every TRAIN shortlist member and one GOD-X7 control at caps .01, 5
and 10, with independent $600 accounts from June 20 to September 2 exclusive.
An empty August finalist set is valid: this diagnostic comparison cannot select
or promote a candidate. Historical publication-final data are not a holdout.
Day/week/month/LOTTO coronations remain the separate, unexecuted blueprint.
"""
from __future__ import annotations

import argparse
import copy
from pathlib import Path

import prepare_lot_growth_sweep as prep
import research_runner_sizing as sizing
import validate_lot_growth_finalists as august

SCHEMA = 'conduit.sizing-full-owner-comparison.v1'
WINDOW = ('2026-06-20', '2026-09-02')
CAPS = (('cap001', .01), ('cap5', 5), ('cap10', 10))
CONTROL = 'GOD-X7-comparison'
FINAL_FILES = ('FINAL_SELECTION.json', 'FINALISTS.json',
               'FOLLOWUP_BLUEPRINT.json', 'SELECTION_RECEIPT.json')
LIMITATION = 'Historical publication-final source, not a complete live edit chronology; no untouched holdout.'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def equal(actual, expected, message):
    require(prep.exact_equal(actual, expected), message)


def frozen_selection(directory):
    """Reconstruct the final freeze, never trust a pasted list or a PASS label."""
    final = Path(directory).resolve()
    stored = prep.read(final/'FINAL_SELECTION.json')
    august.checked_pin(stored['validation_plan'])
    validation = Path(stored['validation_plan']['path']).parent
    root, plan, inputs, contract, train = august.validation_contract(validation)
    accounts, artifacts = {}, []
    for job in plan['jobs']:
        rows, pins = august.completed_job(root/'PLAN.json', plan, job, contract['registry'], inputs)
        accounts[job['id']] = rows
        artifacts += pins
    expected = august.select_rows(train['rows'], accounts['august_300'], accounts['august_600'],
                                  contract['selected_train_ids'])
    configurations = [train['registry'][row['id']] for row in expected['selected']]
    expected.update(schema=august.FINAL_SCHEMA, selector=august.SELECTOR,
                    validation_plan=prep.pin(root/'PLAN.json'),
                    preparation=prep.pin(root/'PREPARATION_RECEIPT.json'),
                    helper=prep.pin(august.__file__),
                    evidence=august.unique_pins(train['evidence']+artifacts),
                    selected_configurations=configurations, limitation=LIMITATION)
    equal(stored, expected, 'August selection differs from recomputed completed evidence')
    equal(prep.read(final/'FINALISTS.json'), configurations, 'Frozen finalist configurations changed')
    expected_blueprint = august.blueprint([r['id'] for r in configurations], contract['selected_train_ids'])
    equal(prep.read(final/'FOLLOWUP_BLUEPRINT.json'), expected_blueprint, 'Follow-up blueprint changed')
    expected_receipt = dict(schema=august.FINAL_SCHEMA, status=expected['status'],
        selection=prep.pin(final/'FINAL_SELECTION.json'), finalists=prep.pin(final/'FINALISTS.json'),
        blueprint=prep.pin(final/'FOLLOWUP_BLUEPRINT.json'), helper=prep.pin(august.__file__),
        evidence=expected['evidence'], no_process_launched=True)
    equal(prep.read(final/'SELECTION_RECEIPT.json'), expected_receipt, 'Final selection receipt changed or incomplete')
    final_pins = [prep.pin(final/name) for name in FINAL_FILES]
    evidence = august.unique_pins(list(train['inputs'].values()) + list(inputs.values())
        + expected['evidence'] + [expected['validation_plan'], expected['preparation'], expected['helper']]
        + final_pins)
    return dict(directory=final, train=train, selection=expected, finalists=configurations,
                final_pins=final_pins, evidence=evidence)


def originals(freeze):
    train = freeze['train']
    shortlist = train['shortlist']
    require(len(shortlist) <= 10, 'TRAIN shortlist exceeds ten candidates')
    records = {row['id']: row for row in shortlist}
    require(len(records) == len(shortlist) and CONTROL not in records, 'Duplicate comparison identity')
    baseline_pin = train['plan']['qualification']['materialized_baseline']
    docs = {name: prep.read(row['file']['path']) for name, row in records.items()}
    docs[CONTROL] = prep.read(baseline_pin['path'])
    require(docs[CONTROL]['settings']['lot_growth_mode'] == 'Off', 'Baseline growth mode is not Off')
    sources = {name: row['file'] for name, row in records.items()}
    sources[CONTROL] = baseline_pin
    return docs, sources


def capped_document(original, name, cap):
    doc = copy.deepcopy(original)
    doc['name'] = name
    doc['settings']['lot_max'] = cap
    return doc


def jobs_for(freeze, output, cpu_budget):
    require(type(cpu_budget) is int and 1 <= cpu_budget <= 24, 'CPU budget must be 1..24')
    train_plan = freeze['train']['plan']
    source_job = next(job for job in train_plan['jobs'] if job['id'] == 'sizing300')
    reference = prep.read(train_plan['qualification']['reference_binding']['path'])
    jobs = []
    for label, cap in CAPS:
        identity = 'owner_'+label
        result = output/identity/'results'
        argv = prep.recipe(reference, source_job['argv'][0], output/'configs'/label,
                           result, *WINDOW, 600)
        jobs.append(dict(id=identity, argv=argv, threads=cpu_budget, result_dir=str(result),
                         expected_candidates=len(freeze['train']['shortlist'])+1))
    return jobs


def comparison_contract(freeze, registry):
    return dict(schema=SCHEMA, mode='full_owner_comparison',
        final_selection_directory=str(freeze['directory']), final_freeze=freeze['final_pins'],
        august_status=freeze['selection']['status'],
        train_plan=prep.pin(freeze['train']['root']/'PLAN.json'),
        train_selection=prep.pin(freeze['train']['root']/'TRAIN_SELECTION.json'),
        shortlist=prep.pin(freeze['train']['root']/'SHORTLIST.json'),
        train_shortlist_ids=[r['id'] for r in freeze['train']['shortlist']],
        august_finalist_ids=[r['id'] for r in freeze['finalists']],
        comparison_control=CONTROL, registry=registry, window=list(WINDOW), deposit=600,
        caps=[cap for _, cap in CAPS], strategy_changes=['lot_max only; display name for the single control'],
        used_for_selection=False, selection_changes_allowed=False, untouched_holdout=False,
        independent_accounts=True,
        execution='Three jobs; each reserves the entire CPU budget, hence sequential. A single config uses one actual worker.',
        memory='No measured RAM qualification. Approve memory and launch separately with the shared progress monitor.',
        limitation=LIMITATION)


def expected_receipt(plan_path, plan):
    contract = plan['comparison_contract']
    return dict(schema=SCHEMA, status='PREPARED_NOT_RUN', plan=prep.pin(plan_path),
        helper=prep.pin(__file__), final_freeze=contract['final_freeze'],
        august_status=contract['august_status'], candidate_runs=3*len(contract['train_shortlist_ids']),
        control_runs=3, total_runs=sum(j['expected_candidates'] for j in plan['jobs']),
        inputs=plan['inputs'], used_for_selection=False, no_process_launched=True)


def prepare(final_selection, output, cpu_budget=12):
    out = august.new_output(output)
    freeze = frozen_selection(final_selection)
    docs, sources = originals(freeze)
    jobs = jobs_for(freeze, out, cpu_budget)
    registry = {}
    # The complete prerequisite proof precedes any output creation.
    out.mkdir(parents=True)
    for (label, cap), job in zip(CAPS, jobs):
        records = {}
        for name, original in docs.items():
            august.safe_id(name)
            doc = capped_document(original, name, cap)
            path = out/'configs'/label/(name+'.json')
            prep.write_new(path, doc)
            records[name] = dict(id=name, file=prep.pin(path), source=sources[name],
                settings_sha256=prep.fingerprint(doc['settings']), control=name == CONTROL, cap=cap)
        registry[job['id']] = records
    inputs = august.unique_pins(freeze['evidence'] + [prep.pin(__file__)]
        + [record['file'] for records in registry.values() for record in records.values()])
    plan = dict(id='g7_sizing_full_owner_comparison', name='GOD-X7 full owner comparison, frozen TRAIN shortlist',
        output=str(out), progress_dir=str(out/'progress'), threads=cpu_budget,
        source_revision=freeze['train']['plan']['source_revision'], inputs=inputs, jobs=jobs,
        qualification=copy.deepcopy(freeze['train']['plan']['qualification']),
        comparison_contract=comparison_contract(freeze, registry),
        metadata=dict(etap_badania='Full owner diagnostic comparison after August freeze',
            kanal='Synergy', tryb_obliczen='exact', status_walidacji='Previously seen; cannot select candidates'))
    sizing.validate_plan_inputs(plan)
    prep.write_new(out/'PLAN.json', plan)
    receipt = expected_receipt(out/'PLAN.json', plan)
    prep.write_new(out/'PREPARATION_RECEIPT.json', receipt)
    return receipt


def check(study):
    """Recheck the prepared economic contract in addition to generic runner pins."""
    root = Path(study).resolve()
    plan = prep.read(root/'PLAN.json')
    equal(plan['output'], str(root), 'Comparison output path changed')
    sizing.validate_plan_inputs(plan)
    contract = plan['comparison_contract']
    freeze = frozen_selection(contract['final_selection_directory'])
    docs, sources = originals(freeze)
    expected_jobs = jobs_for(freeze, root, plan['threads'])
    equal(plan['jobs'], expected_jobs, 'Full owner window, cap jobs or recipe changed')
    registry = {}
    for (label, cap), job in zip(CAPS, expected_jobs):
        records = {}
        folder = root/'configs'/label
        require({p.name for p in folder.iterdir()} == {name+'.json' for name in docs}, 'Unexpected comparison config file set')
        for name, original in docs.items():
            path = folder/(name+'.json')
            expected = capped_document(original, name, cap)
            equal(prep.read(path), expected, 'Comparison changed a strategy setting or metadata: '+name)
            records[name] = dict(id=name, file=prep.pin(path), source=sources[name],
                settings_sha256=prep.fingerprint(expected['settings']), control=name == CONTROL, cap=cap)
        registry[job['id']] = records
    equal(contract, comparison_contract(freeze, registry), 'Comparison contract or registry changed')
    equal(plan['qualification'], freeze['train']['plan']['qualification'], 'Comparison input qualification changed')
    equal(plan['source_revision'], freeze['train']['plan']['source_revision'], 'Comparison build changed')
    expected_inputs = august.unique_pins(freeze['evidence'] + [prep.pin(__file__)]
        + [r['file'] for records in registry.values() for r in records.values()])
    equal(plan['inputs'], expected_inputs, 'Comparison inputs differ from the verified preparation')
    equal(prep.read(root/'PREPARATION_RECEIPT.json'), expected_receipt(root/'PLAN.json', plan),
          'Comparison preparation receipt changed')
    return dict(schema=SCHEMA, status='PASS', plan=prep.pin(root/'PLAN.json'),
                total_runs=sum(job['expected_candidates'] for job in plan['jobs']),
                august_status=freeze['selection']['status'], no_process_launched=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    full = commands.add_parser('full', help='Prepare all TRAIN shortlist candidates and one control at three caps')
    full.add_argument('--final-selection', required=True, type=Path)
    full.add_argument('--output', required=True, type=Path)
    full.add_argument('--cpu-budget', default=12, type=int)
    verify = commands.add_parser('check', help='Recompute the freeze and verify an existing comparison plan')
    verify.add_argument('--study', required=True, type=Path)
    args = parser.parse_args()
    result = prepare(args.final_selection, args.output, args.cpu_budget) if args.command == 'full' else check(args.study)
    print(result['status'])


if __name__ == '__main__':
    main()
