import unittest

from day_concentration import analyze_days


def days(profits):
    return [{'date': f'2026-08-{i+1:02}', 'profit': p, 'start_equity': 600., 'end_equity': 600.+p}
            for i, p in enumerate(profits)]


class ConcentrationTests(unittest.TestCase):
    def test_large_bonus_day_does_not_disqualify_ordinary_profits(self):
        robust = analyze_days(days([10, 10, 1000, 10, 10]), .01, 'continuous')
        fragile = analyze_days(days([-10, -10, 1000, -10, -10]), .01, 'continuous')
        self.assertEqual(robust['fixed_lot_exclusion']['profit_without_best_1'], 40)
        self.assertEqual(fragile['fixed_lot_exclusion']['profit_without_best_1'], -40)
        self.assertEqual(robust['median_day'], 10)
        self.assertEqual(fragile['median_day'], -10)

    def test_exclusion_cannot_be_reported_for_compounding_caps(self):
        for cap in (0, .1, 5, 10):
            result = analyze_days(days([10, 100]), cap, 'continuous')
            self.assertFalse(result['fixed_lot_exclusion_enabled'])
            self.assertIsNone(result['fixed_lot_exclusion'])

    def test_cash_concentration_and_percentage_growth_are_distinct(self):
        observed = days([600, 1200, 2400, 4800])
        for day in observed:
            day['start_equity'] = day['profit']
            day['end_equity'] = day['profit'] * 2
        result = analyze_days(observed, 10, 'continuous')
        self.assertGreater(result['concentration']['top_1_share_of_positive_pnl_pct'], 50)
        self.assertEqual(result['median_daily_return_pct'], 100)
        self.assertIsNone(result['fixed_lot_exclusion'])

    def test_bad_or_duplicate_day_cannot_silently_enter_report(self):
        malformed = days([10])
        malformed[0]['profit'] = 20
        with self.assertRaises(ValueError):
            analyze_days(malformed, .01, 'continuous')
        with self.assertRaises(ValueError):
            analyze_days(days([10]) * 2, .01, 'continuous')


if __name__ == '__main__':
    unittest.main()
