"""Decision regressions for the research queue, before any preset coronation."""
import unittest

from rank_giga_sweep8 import select


def row(name, profit=100, filled=100, used=80, green=70, closed_green=80, family="test"):
    return {"name": name, "family": family, "total_profit": profit,
            "filled_baskets": filled, "signal_utilization_pct": used,
            "positive_market_days_pct": green, "win_days_pct": closed_green,
            "blown": False, "stop_outs": 0, "min_equity": 450,
            "max_daily_dd": 20}


class ResearchRankingTests(unittest.TestCase):
    def test_activity_floor_rejects_high_profit_from_few_baskets(self):
        rows = {r["name"]: r for r in [
            row("GOD-X7-cap5", family="reference"),
            row("sparse_jackpot", profit=1_000_000, filled=20, used=15),
            row("broad", profit=200, filled=95, used=78),
        ]}
        result = select(rows, 10)
        selected = {r["name"] for r in result["selected_for_exact_replay"]}
        self.assertEqual(selected, {"GOD-X7-cap5", "broad"})
        self.assertEqual(result["rejection_counts"]["below_activity_floor"], 1)
        self.assertFalse(result["coronation_eligible"])

    def test_floating_loss_days_cannot_disappear_from_green_day_ranking(self):
        rows = {r["name"]: r for r in [
            row("GOD-X7-cap5", family="reference"),
            row("close_winners_hold_losers", profit=1000, green=30, closed_green=100),
            row("broad_daily_gains", profit=500, green=85, closed_green=85),
        ]}
        selected = {r["name"]: r for r in select(rows, 10)["selected_for_exact_replay"]}
        reason = "family_positive_equity_day_leader"
        self.assertIn(reason, selected["broad_daily_gains"]["selection_reasons"])
        self.assertNotIn(reason, selected["close_winners_hold_losers"]["selection_reasons"])

    def test_stopout_and_nonpositive_profit_are_diagnostics_only(self):
        stopped = row("stopped", profit=1000)
        stopped["stop_outs"] = 1
        rows = {r["name"]: r for r in [
            row("GOD-X7-cap5", family="reference"), stopped, row("loss", profit=-1),
        ]}
        result = select(rows, 10)
        self.assertEqual([r["name"] for r in result["selected_for_exact_replay"]], ["GOD-X7-cap5"])
        self.assertEqual(result["rejection_counts"]["insolvent_or_stop_out"], 1)
        self.assertEqual(result["rejection_counts"]["nonpositive_profit"], 1)


if __name__ == "__main__":
    unittest.main()
