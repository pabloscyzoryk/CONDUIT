import json
from pathlib import Path
import tempfile
import unittest

from rank_giga_sweep8 import collect, select
from research_runner import file_hash


def candidate(name, profit, filled, green=60, family="synthetic"):
    return {"name": name, "family": family, "total_profit": profit,
            "filled_baskets": filled, "signal_utilization_pct": filled / 4,
            "win_days_pct": green, "blown": False, "stop_outs": 0,
            "min_equity": 400, "max_daily_dd": 100}


class ActivitySelectionTests(unittest.TestCase):
    def test_large_profit_from_tiny_activity_cannot_displace_active_candidate(self):
        rows = {r["name"]: r for r in [candidate("GOD-X7-cap5", -100, 200),
                candidate("tiny", 1e9, 20, 100), candidate("active", 500, 230)]}
        result = select(rows, 5)
        self.assertEqual({r["name"] for r in result["selected_for_exact_replay"]}, {"GOD-X7-cap5", "active"})
        self.assertEqual(result["rejection_counts"]["below_activity_floor"], 1)
        self.assertFalse(result["coronation_eligible"])

    def test_high_green_percentage_cannot_hide_overall_loss_or_stop_out(self):
        rows = {r["name"]: r for r in [candidate("GOD-X7-cap5", 100, 200),
                candidate("green_but_losing", -5, 300, 98), candidate("margin", 1000, 250)]}
        rows["margin"]["stop_outs"] = 1
        result = select(rows, 5)
        self.assertEqual(len(result["selected_for_exact_replay"]), 1)
        self.assertEqual(result["rejection_counts"], {"nonpositive_profit": 1, "insolvent_or_stop_out": 1})

    def test_receipt_hash_rejects_edited_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            job = root / "screen_000"
            job.mkdir()
            result = job / "wyniki_daily.json"
            result.write_text("{}", encoding="utf-8")
            plan = root / "plan.json"
            plan.write_text(json.dumps({"output": str(root), "source_revision": "synthetic-v1",
                "metadata": {"max_lot": 5}, "jobs": [{"id": "screen_000", "expected_candidates": 1}]}), encoding="utf-8")
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"search_lot_cap": 5, "candidate_count": 1,
                "candidates": [{"id": "A", "family": "test", "fingerprint": "synthetic"}]}), encoding="utf-8")
            receipt = {"status": "complete", "returncode": 0, "partial": False,
                "source_revision": "synthetic-v1", "plan_sha256": file_hash(plan),
                "result_files": [str(result)], "result_sha256": {result.name: file_hash(result)}}
            (job / "receipt.json").write_text(json.dumps(receipt), encoding="utf-8")
            result.write_text('{"A": {"total_profit": 1000}}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "hash changed"):
                collect(plan, manifest)


if __name__ == "__main__":
    unittest.main()
