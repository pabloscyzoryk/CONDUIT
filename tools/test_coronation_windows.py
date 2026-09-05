import datetime as dt
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from prepare_coronation8 import calendar_windows, main, validate_candidate_names
from giga_sweep8 import fingerprint
from research_runner import file_hash


class CoronationWindowsTests(unittest.TestCase):
    def test_optional_reference_and_case_only_names_cannot_overwrite_a_finalist(self):
        for names in [
            ['GOD-X7-reference', 'candidateA', 'GOD-X7-reference'],
            ['god-x7-reference', 'candidateA', 'GOD-X7-reference'],
            ['CandidateA', 'candidatea'],
        ]:
            with self.assertRaisesRegex(ValueError, 'unique'):
                validate_candidate_names(names)
        validate_candidate_names(['candidateA', 'candidateB', 'GOD-X7-reference'])

    def test_calendar_weeks_are_not_five_day_rolling_windows(self):
        start, end = dt.date(2026, 8, 27), dt.date(2026, 9, 3)
        days = ["2026-08-27", "2026-08-28", "2026-08-31", "2026-09-01", "2026-09-02"]
        windows = calendar_windows(start, end, days)
        weeks = [(w["from"], w["to"]) for w in windows if w["kind"] == "week"]
        self.assertEqual(weeks, [("2026-08-27", "2026-08-31"), ("2026-08-31", "2026-09-03")])
        months = [(w["from"], w["to"]) for w in windows if w["kind"] == "month"]
        self.assertEqual(months, [("2026-08-27", "2026-09-01"), ("2026-09-01", "2026-09-03")])
        self.assertEqual(sum(w["kind"] == "day" for w in windows), 5)

    def test_exclusive_end_and_missing_tick_days_are_respected(self):
        windows = calendar_windows(dt.date(2026, 12, 30), dt.date(2027, 1, 4),
                                   ["2026-12-30", "2026-12-31", "2027-01-04"])
        self.assertEqual([w["id"] for w in windows if w["kind"] == "day"],
                         ["day_2026-12-30", "day_2026-12-31"])
        self.assertEqual([(w["from"], w["to"]) for w in windows if w["kind"] == "month"],
                         [("2026-12-30", "2027-01-01")])
        with self.assertRaises(ValueError):
            calendar_windows(dt.date(2026, 9, 5), dt.date(2026, 9, 5), [])

    def test_every_coronation_window_retains_measured_broker_sessions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixtures = {}
            for name in ('engine.exe', 'source.json', 'messages.json', 'ticks.bin', 'sessions.json'):
                path = root / name
                path.write_bytes(b'{}')
                fixtures[name] = path
            tick_manifest = root / 'ticks-manifest.json'
            tick_manifest.write_text(json.dumps({'per_day_ticks': {'2026-09-01': 1},
                                                'output_sha256': file_hash(fixtures['ticks.bin'])}), encoding='utf-8')
            finalists = []
            for i in range(5):
                settings = {'lot_max': 5, 'entry_units': i + 1}
                preset = root / f'candidate{i}.json'
                preset.write_text(json.dumps({'settings': settings}), encoding='utf-8')
                finalists.append({'id': f'candidate{i}', 'preset_path': str(preset),
                                  'fingerprint': fingerprint(settings)})
            selection = root / 'finalists.json'
            selection.write_text(json.dumps(finalists), encoding='utf-8')
            argv = ['prepare_coronation8', '--finalists', str(selection),
                    '--exe', str(fixtures['engine.exe']), '--source-manifest', str(fixtures['source.json']),
                    '--ticks', str(fixtures['ticks.bin']), '--tick-manifest', str(tick_manifest),
                    '--signals', str(fixtures['messages.json']), '--trade-sessions', str(fixtures['sessions.json']),
                    '--from', '2026-09-01', '--to', '2026-09-02',
                    '--progress-dir', str(root/'progress'), '--stop-at', '2026-09-05T21:30:00+02:00',
                    '--source-state', 'fresh_engine']
            reference = root/'reference.json'
            reference.write_text(json.dumps({'settings': {'lot_max': 5, 'entry_units': 99}}))
            collision = [dict(row) for row in finalists]
            collision[0]['id'] = 'GOD-X7-reference'
            selection.write_text(json.dumps(collision), encoding='utf-8')
            collision_output = root/'collision-output'
            with patch('sys.argv', argv + ['--output', str(collision_output), '--signal-contract', 'historical_reference', '--include-reference', str(reference)]), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(ValueError, 'unique'):
                    main()
            self.assertFalse(collision_output.exists(), 'collision must fail before any candidate files are written')
            selection.write_text(json.dumps(finalists), encoding='utf-8')
            for contract in ('historical_reference', 'observed_receipts', 'mixed_missing_original_stress'):
                with self.subTest(contract=contract):
                    output = root / contract
                    command_line = argv + ['--output', str(output), '--signal-contract', contract]
                    with patch('sys.argv', command_line), contextlib.redirect_stdout(io.StringIO()):
                        main()
                    plan = json.loads((output/'plan.json').read_text('utf-8'))
                    self.assertEqual(len(plan['jobs']), 24)  # full/day/week/month x 2 deposits x 3 caps
                    self.assertEqual(plan['protocol']['signal_contract'], contract)
                    for job in plan['jobs']:
                        command = job['argv']
                        self.assertEqual(command[command.index('--sim-trade-sessions')+1],
                                         str(fixtures['sessions.json'].resolve()))
                        self.assertEqual(command[command.index('--quick-tick-stride')+1], '1')
                        self.assertEqual('--live-telegram-ingress' in command,
                                         contract != 'historical_reference')
                        cap = {'lot001': .01, 'lot10': 10, 'arithmetic': 0}[job['cap_mode']]
                        candidates = list(Path(command[command.index('--sweep')+1]).glob('*.json'))
                        self.assertEqual(len(candidates), job['expected_candidates'])
                        self.assertEqual({json.loads(p.read_text())['settings']['lot_max'] for p in candidates}, {cap})
                        self.assertIn(job['deposit'], (300, 600))
                    for finalist in finalists:
                        self.assertEqual(json.loads(Path(finalist['preset_path']).read_text())['settings']['lot_max'], 5)
                    self.assertIn({'path': str(fixtures['sessions.json'].resolve()),
                                   'sha256': file_hash(fixtures['sessions.json'])}, plan['inputs'])


if __name__ == "__main__":
    unittest.main()
