import json
from pathlib import Path
import tempfile
import unittest

from giga_sweep8 import fingerprint
from prepare_replay_cases8 import prepare, validate_case


class ReplayPlanTests(unittest.TestCase):
    def test_case_cannot_disguise_strategy_change_as_broker_overlay(self):
        case = {'id': 'comparison', 'signal_contract': 'observed_receipts',
                'from': '2026-08-05', 'to': '2026-09-05', 'deposit': 600, 'lot_cap': 10,
                'broker_overlay': {'entry_units': 1}}
        with self.assertRaisesRegex(ValueError, 'strategy axes'):
            validate_case(case)
        case['broker_overlay'] = {'commission_per_lot': 0}
        validate_case(case)
        case['deposit'] = float('nan')
        with self.assertRaisesRegex(ValueError, 'finite'):
            validate_case(case)

    def test_exact_plan_keeps_cap5_original_and_separates_ingress(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            space = root/'space'
            (space/'configs').mkdir(parents=True)
            settings = {'lot_max': 5, 'entry_units': 8, 'swap_long_points': -75.82}
            preset = space/'configs/GOD-X7-cap5.json'
            original = json.dumps({'settings': settings})
            preset.write_text(original)
            (space/'manifest.json').write_text('{}')
            selection = root/'selection.json'
            selection.write_text(json.dumps({'selected_for_exact_replay': [
                {'name': 'GOD-X7-cap5', 'fingerprint': fingerprint(settings)}]}))
            for filename in ('engine.exe', 'source.json', 'ticks.bin', 'signals.json', 'sessions.json'):
                (root/filename).write_text('{}')
            cases = []
            for contract in ('historical_reference', 'observed_receipts'):
                cases.append({'id': contract, 'signal_contract': contract,
                              'from': '2026-08-05', 'to': '2026-09-05', 'deposit': 600, 'lot_cap': 10,
                              'ticks': str(root/'ticks.bin'), 'signals': str(root/'signals.json'),
                              'trade_sessions': str(root/'sessions.json'),
                              'broker_overlay': {'swap_long_points': 0}})
            cases_path = root/'cases.json'
            cases_path.write_text(json.dumps(cases))
            output = root/'exact'
            prepare(selection, space, root/'engine.exe', root/'source.json', cases_path,
                    output, root/'progress', '2026-09-05T20:00:00+02:00', 'full_comparison')
            plan = json.loads((output/'plan.json').read_text('utf-8'))
            for job in plan['jobs']:
                self.assertEqual('--live-telegram-ingress' in job['argv'], job['id'] == 'observed_receipts')
                self.assertIn('--sim-trade-sessions', job['argv'])
                self.assertEqual(job['argv'][job['argv'].index('--quick-tick-stride')+1], '1')
                changed = json.loads((output/'configs'/job['id']/'GOD-X7-cap5.json').read_text())['settings']
                self.assertEqual(changed, {'lot_max': 10, 'entry_units': 8, 'swap_long_points': 0})
            self.assertEqual(preset.read_text(), original)
            self.assertFalse(plan['protocol']['production_choice_made'])


if __name__ == '__main__':
    unittest.main()
