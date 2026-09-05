"""Prepare exact comparisons of a qualified queue under separate data contracts.

Every case uses the same candidate queue and a continuous account. Only the
explicit lot cap and common broker overlay may differ from the cap-5 search.
This is a validation plan, never a preset selection or coronation decision.
"""
from __future__ import annotations
import argparse
import copy
import datetime as dt
import json
import math
from pathlib import Path
import re

from giga_sweep8 import BROKER_FIELDS, fingerprint
from research_runner import file_hash

CONTRACTS = {'historical_reference', 'observed_receipts', 'mixed_missing_original_stress'}


def validate_case(case):
    if not re.fullmatch(r'[A-Za-z0-9_-]+', case['id']):
        raise ValueError('Unsafe case identifier')
    if case['signal_contract'] not in CONTRACTS:
        raise ValueError('Explicit supported signal contract required')
    if dt.date.fromisoformat(case['from']) >= dt.date.fromisoformat(case['to']):
        raise ValueError('Case end is exclusive and must follow its start')
    if case.get('lot_cap') not in (0, .01, 5, 10):
        raise ValueError('Unsupported validation lot cap')
    if not isinstance(case.get('deposit'), (int, float)) or not math.isfinite(case['deposit']) or case['deposit'] <= 0:
        raise ValueError('Positive finite deposit required')
    if set(case.get('broker_overlay', {})) - BROKER_FIELDS:
        raise ValueError('A comparison overlay cannot change strategy axes')


def prepare(selection, space, exe, source_manifest, cases_path, output, progress, stop_at, stage):
    if output.exists():
        raise FileExistsError(output)
    if dt.datetime.fromisoformat(stop_at).tzinfo is None:
        raise ValueError('Deadline requires an explicit timezone')
    chosen = json.loads(selection.read_text('utf-8-sig'))['selected_for_exact_replay']
    if not chosen or len({row['name'] for row in chosen}) != len(chosen):
        raise ValueError('Nonempty unique exact queue required')
    if 'GOD-X7-cap5' not in {row['name'] for row in chosen}:
        raise ValueError('The same GOD-X7 reference is required in every exact comparison')
    originals = {}
    for row in chosen:
        if not re.fullmatch(r'[A-Za-z0-9_-]+', row['name']):
            raise ValueError('Unsafe candidate identifier')
        path = space / 'configs' / (row['name'] + '.json')
        document = json.loads(path.read_text('utf-8-sig'))
        if document['settings']['lot_max'] != 5 or fingerprint(document['settings']) != row['fingerprint']:
            raise ValueError('Selected cap-5 candidate changed')
        originals[row['name']] = document
    cases = json.loads(cases_path.read_text('utf-8-sig'))
    if not cases or len({case['id'] for case in cases}) != len(cases):
        raise ValueError('Nonempty unique cases required')
    for case in cases:
        validate_case(case)
        if case['lot_cap'] == 0 and any(doc['settings'].get('order_volume_contract_v2') for doc in originals.values()):
            raise ValueError('V2 unlimited arithmetic requires an explicit broker-volume profile')
    artifacts = {selection, space/'manifest.json', exe, source_manifest, cases_path, Path(__file__),
                 Path(__file__).with_name('research_runner.py')}
    for case in cases:
        artifacts.update(Path(case[key]) for key in ('ticks', 'signals'))
        if case.get('trade_sessions'):
            artifacts.add(Path(case['trade_sessions']))
    for path in artifacts:
        if not path.is_file():
            raise FileNotFoundError(path)
    output.mkdir(parents=True)
    jobs = []
    for case in cases:
        identity = case['id']
        presets = output/'configs'/identity
        presets.mkdir(parents=True)
        for name, original in originals.items():
            document = copy.deepcopy(original)
            document['settings'].update(case.get('broker_overlay', {}))
            document['settings']['lot_max'] = case['lot_cap']
            path = presets/(name+'.json')
            path.write_text(json.dumps(document, indent=2), encoding='utf-8')
            artifacts.add(path)
        result = output/identity/'results'
        command = [str(exe.resolve()), '--ticks', str(Path(case['ticks']).resolve()),
                   '--signals', str(Path(case['signals']).resolve()), '--sweep', str(presets.resolve()),
                   '--from', case['from'], '--to', case['to'], '--balance', str(case['deposit']),
                   '--signal-time-offset-min', '0', '--sim-limit-price-improvement', '--sim-price-digits', '2',
                   '--sim-new-pending-sl-next-tick', '--sim-native-swap-cash-digits', '2',
                   '--rozgrzewka-h', '72', '--quick-tick-stride', '1', '--no-charts', '--dump-trades',
                   '--out', str(result.resolve())]
        if case['signal_contract'] != 'historical_reference':
            command.append('--live-telegram-ingress')
        if case.get('trade_sessions'):
            command += ['--sim-trade-sessions', str(Path(case['trade_sessions']).resolve())]
        jobs.append({'id': identity, 'argv': command, 'threads': min(24, len(originals)),
                     'expected_candidates': len(originals), 'result_dir': str(result.resolve()),
                     'deposit': case['deposit'], 'lot_cap': case['lot_cap'],
                     'window': {'from': case['from'], 'to': case['to']},
                     'signal_contract': case['signal_contract'], 'broker_overlay': case.get('broker_overlay', {})})
    plan = {'id': 'giga_sweep8_exact_' + stage, 'name': 'giga_sweep8 — dokładne porównanie osobnych historii',
            'threads': 24, 'output': str(output.resolve()), 'progress_dir': str(progress.resolve()),
            'stop_at': stop_at, 'source_revision': file_hash(source_manifest), 'jobs': jobs,
            'inputs': [{'path': str(path.resolve()), 'sha256': file_hash(path)} for path in sorted(artifacts)],
            'metadata': {'etap_badania': stage, 'tryb_obliczen': 'exact', 'kanal': 'Synergy',
                         'kandydaci': len(originals), 'status_walidacji': 'full_window_comparison'},
            'protocol': {'continuous_account': True, 'parameter_search': False,
                         'starting_selection': 'All candidates were searched at lot cap5; only cap and stated common broker overlays change.',
                         'overlap': 'Separate contracts; never combine their profits or call overlapping periods independent.',
                         'calibration': 'Quick screening is not an exact result; this plan replays every retained tick.',
                         'unknown_originals': 'Historical publication-final assumptions and mixed missing-original stress remain explicit.',
                         'best_day_exclusion': 'Only permitted at cap0.01, never at higher compounding caps.',
                         'production_choice_made': False}, 'selection': chosen}
    (output/'plan.json').write_text(json.dumps(plan, ensure_ascii=False, indent=2), encoding='utf-8')
    return {'cases': len(cases), 'candidates': len(originals), 'exact_runs': len(cases)*len(originals), 'launched': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('selection', 'space', 'exe', 'source-manifest', 'cases', 'output', 'progress-dir'):
        parser.add_argument('--'+name, required=True, type=Path)
    parser.add_argument('--stop-at', required=True)
    parser.add_argument('--stage', required=True, choices=['training_confirmation', 'full_comparison', 'chronological_validation'])
    args = parser.parse_args()
    print(json.dumps(prepare(args.selection, args.space, args.exe, args.source_manifest, args.cases,
                             args.output, args.progress_dir, args.stop_at, args.stage)))


if __name__ == '__main__':
    main()
