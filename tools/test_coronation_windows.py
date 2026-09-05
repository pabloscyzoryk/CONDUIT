import datetime as dt
import unittest

from prepare_coronation8 import calendar_windows


class CoronationWindowsTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
