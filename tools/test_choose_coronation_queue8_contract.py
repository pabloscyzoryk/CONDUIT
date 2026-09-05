"""Offline scenario/argv regression coverage; no workspace data or executable needed."""
import copy
import unittest

from choose_coronation_queue8 import validate_plans, REFERENCE


def plans():
    scenarios = [
        ('historical_owner_recipe_full10', '2026-06-20', '2026-09-02', 10, 'historical_reference'),
        ('historical_latest_full_5', '2026-06-20', '2026-09-05', 5, 'historical_reference'),
        ('historical_latest_full_10', '2026-06-20', '2026-09-05', 10, 'historical_reference'),
        ('historical_latest_full_0p01', '2026-06-20', '2026-09-05', .01, 'historical_reference'),
        ('observed_full_5', '2026-08-05', '2026-09-05', 5, 'observed_receipts'),
        ('observed_full_10', '2026-08-05', '2026-09-05', 10, 'observed_receipts'),
        ('observed_full_0p01', '2026-08-05', '2026-09-05', .01, 'observed_receipts'),
        ('mixed_stress_full_5', '2026-06-01', '2026-09-05', 5, 'mixed_missing_original_stress'),
        ('mixed_stress_full_10', '2026-06-01', '2026-09-05', 10, 'mixed_missing_original_stress'),
        ('mixed_stress_full_0p01', '2026-06-01', '2026-09-05', .01, 'mixed_missing_original_stress'),
    ]
    later = [
        ('historical_latest_later5', '2026-08-17', '2026-09-05', 5, 'historical_reference'),
        ('observed_later5', '2026-08-17', '2026-09-05', 5, 'observed_receipts'),
    ]

    def document(stage, cases):
        jobs = []
        for name, start, end, cap, corpus in cases:
            argv = ['fixture-engine.exe', '--from', start, '--to', end,
                    '--quick-tick-stride', '1', '--signal-time-offset-min', '0', '--balance', '600']
            if corpus != 'historical_reference':
                argv.append('--live-telegram-ingress')
            jobs.append({'id': name, 'window': {'from': start, 'to': end}, 'lot_cap': cap,
                         'deposit': 600, 'signal_contract': corpus, 'argv': argv})
        return {'metadata': {'etap_badania': stage}, 'jobs': jobs,
                'selection': [{'name': REFERENCE, 'fingerprint': 'a'*64}]}
    return document('full_comparison', scenarios), document('chronological_validation', later)


class ScenarioArgumentTests(unittest.TestCase):
    def test_scope_metadata_cannot_hide_different_or_ambiguous_actual_arguments(self):
        full, later = plans()
        self.assertEqual(validate_plans(full, later), {REFERENCE: 'a'*64})
        for flag, value in [('--from', '2020-01-01'), ('--to', '2020-01-02'),
                            ('--balance', '300'), ('--quick-tick-stride', '50'),
                            ('--signal-time-offset-min', '180')]:
            with self.subTest(flag=flag):
                changed = copy.deepcopy(full)
                argv = changed['jobs'][0]['argv']
                argv[argv.index(flag)+1] = value
                with self.assertRaises(ValueError):
                    validate_plans(changed, later)
        for mutation in ('duplicate', 'missing', 'missing_value'):
            with self.subTest(mutation=mutation):
                changed = copy.deepcopy(full)
                argv = changed['jobs'][0]['argv']
                i = argv.index('--from')
                if mutation == 'duplicate':
                    argv.extend(['--from', '2026-06-20'])
                elif mutation == 'missing':
                    del argv[i:i+2]
                else:
                    del argv[i:i+2]
                    argv.append('--from')
                with self.assertRaises(ValueError):
                    validate_plans(changed, later)

    def test_source_label_cannot_hide_changed_actual_ingress_policy(self):
        full, later = plans()
        for index in (0, 4, 7):
            with self.subTest(case=full['jobs'][index]['id']):
                changed = copy.deepcopy(full)
                argv = changed['jobs'][index]['argv']
                if '--live-telegram-ingress' in argv:
                    argv.remove('--live-telegram-ingress')
                else:
                    argv.append('--live-telegram-ingress')
                with self.assertRaisesRegex(ValueError, 'ingress'):
                    validate_plans(changed, later)
        changed = copy.deepcopy(later)
        changed['jobs'][1]['argv'].remove('--live-telegram-ingress')
        with self.assertRaisesRegex(ValueError, 'ingress'):
            validate_plans(full, changed)


if __name__ == '__main__':
    unittest.main()
