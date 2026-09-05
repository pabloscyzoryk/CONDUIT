import copy
import unittest
from export_research_site8 import checked_series, extrema_preview


class SiteExportTests(unittest.TestCase):
    def fixture(self):
        return ({'dni': [{'date': '2026-06-01', 'start_equity': 600, 'end_equity': 610, 'profit': 10},
                         {'date': '2026-06-02', 'start_equity': 610, 'end_equity': 580, 'profit': -30}],
                 'krzywa': [[1, 600], [2, 620], [3, 580]], 'saldo': [[1, 600], [3, 590]],
                 'sygnaly_sciezka': 'PRIVATE_DO_NOT_EXPORT', 'sygnaly': ['PRIVATE_MESSAGE']},
                {'market_days': 2, 'start_balance': 600, 'end_equity': 580, 'end_balance': 590, 'total_profit': -20})

    def test_floating_loss_and_private_fields(self):
        document, metrics = self.fixture()
        points, daily = checked_series(document, metrics)
        self.assertEqual(daily[-1]['profit'], -30)
        self.assertEqual(points[-1], {'timestamp': 3, 'equity': 580, 'balance': 590})
        self.assertNotIn('PRIVATE', repr(points)+repr(daily))

    def test_rejects_edited_daily_history_and_wrong_curve_end(self):
        document, metrics = self.fixture()
        bad = copy.deepcopy(document)
        bad['dni'][1]['profit'] = 30
        with self.assertRaises(ValueError):
            checked_series(bad, metrics)
        bad = copy.deepcopy(document)
        bad['krzywa'][-1][1] = 590
        with self.assertRaises(ValueError):
            checked_series(bad, metrics)

    def test_preview_retains_one_tick_spike_and_crash_in_order(self):
        points = [{'timestamp': i, 'equity': 600+i/100, 'balance': 600} for i in range(10000)]
        points[1337]['equity'] = 10000
        points[1338]['equity'] = 10
        points[999]['balance'] = 50
        selected = extrema_preview(points, 100)
        self.assertLessEqual(len(selected), 100)
        self.assertEqual(selected[0], points[0])
        self.assertEqual(selected[-1], points[-1])
        self.assertIn(points[1337], selected)
        self.assertIn(points[1338], selected)
        self.assertIn(points[999], selected)
        self.assertEqual([p['timestamp'] for p in selected], sorted(p['timestamp'] for p in selected))

    def test_rejects_incomplete_day_count(self):
        document, metrics = self.fixture()
        document['dni'].pop()
        with self.assertRaises(ValueError):
            checked_series(document, metrics)

    def test_rejects_reversed_market_dates_and_wrong_balance(self):
        document, metrics = self.fixture()
        document['dni'][0]['date'], document['dni'][1]['date'] = document['dni'][1]['date'], document['dni'][0]['date']
        with self.assertRaisesRegex(ValueError, 'dates'):
            checked_series(document, metrics)
        document, metrics = self.fixture()
        document['saldo'][-1][1] = 9999
        with self.assertRaisesRegex(ValueError, 'Balance'):
            checked_series(document, metrics)


if __name__ == '__main__':
    unittest.main()
