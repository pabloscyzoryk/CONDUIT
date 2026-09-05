import copy
import unittest

from narrow_exact_sweep8 import choose, from_verified_report


def candidate(name, profit=100, family='a', green=50, activity=100):
    return dict(name=name, family=family, fingerprint=name, total_profit=profit,
        end_equity=600+profit, filled_baskets=activity, accepted_entry_sources_pct=80,
        min_equity=500, positive_market_days_pct=green, blown=False, stop_outs=0)


class ExactNarrowingTests(unittest.TestCase):
    def test_short_queue_retains_global_leader_after_many_families(self):
        rows = {f'A{i}': candidate(f'A{i}', 101+i, f'family{i}') for i in range(30)}
        rows['GOD-X7-cap5'] = candidate('GOD-X7-cap5')
        rows['Zleader'] = candidate('Zleader', 10000, 'zzz')
        chosen = choose(rows, copy.deepcopy(rows), 6)['selected_for_exact_replay']
        self.assertEqual(len(chosen), 6)
        self.assertIn('Zleader', {r['name'] for r in chosen})
        self.assertIn('GOD-X7-cap5', {r['name'] for r in chosen})

    def test_profit_does_not_hide_low_activity_or_missing_equity_days(self):
        rows = {n: candidate(n) for n in ['GOD-X7-cap5', 'valid']}
        rows['jackpot'] = candidate('jackpot', 1e9, activity=5)
        rows['unknown_days'] = candidate('unknown_days', 1e9, green=None)
        chosen = choose(rows, copy.deepcopy(rows), 6)['selected_for_exact_replay']
        self.assertEqual({r['name'] for r in chosen}, {'GOD-X7-cap5', 'valid'})

    def test_observed_loss_disqualifies_historical_winner(self):
        historical = {n: candidate(n) for n in ['GOD-X7-cap5', 'fragile', 'robust']}
        historical['fragile']['total_profit'] = 1e9
        observed = copy.deepcopy(historical)
        observed['fragile']['total_profit'] = -1
        chosen = choose(historical, observed, 6)['selected_for_exact_replay']
        self.assertNotIn('fragile', {r['name'] for r in chosen})

    def test_partial_report_never_narrows(self):
        with self.assertRaisesRegex(ValueError, 'complete'):
            from_verified_report({'all_cases_complete': False}, {})

    def test_full_window_cannot_masquerade_as_training(self):
        with self.assertRaisesRegex(ValueError, 'training'):
            from_verified_report({'all_cases_complete': True}, {'jobs': [{'id': 'historical_full'}]})


if __name__ == '__main__':
    unittest.main()
