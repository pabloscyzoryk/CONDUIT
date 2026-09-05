"""Select a diverse exact-replay queue from verified screening receipts.

This is a research queue, never a production-default or coronation decision.
Receipt hashes prevent partial, stale or edited summaries entering the ranking.
"""
from __future__ import annotations
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path

from research_metrics import compact, read_summary
from research_runner import file_hash


def collect(plan_path: Path, manifest_path: Path, prefix: str = "screen_"):
    plan = json.loads(plan_path.read_text("utf-8-sig"))
    manifest = json.loads(manifest_path.read_text("utf-8-sig"))
    candidates = {row["id"]: row for row in manifest["candidates"]}
    if manifest.get("search_lot_cap") != 5 or plan.get("metadata", {}).get("max_lot") != 5:
        raise ValueError("Screening must use the common user lot cap of 5")
    plan_hash = file_hash(plan_path)
    rows, receipts, missing = {}, [], []
    for job in plan["jobs"]:
        if not job["id"].startswith(prefix):
            continue
        receipt_path = Path(plan["output"]) / job["id"] / "receipt.json"
        if not receipt_path.exists():
            missing.append(job["id"])
            continue
        receipt = json.loads(receipt_path.read_text("utf-8-sig"))
        if receipt.get("status") != "complete":
            missing.append(job["id"])
            continue
        if receipt.get("plan_sha256") != plan_hash or receipt.get("partial") is not False:
            raise ValueError("Receipt does not prove completion under the current plan")
        if receipt.get("source_revision") != plan.get("source_revision"):
            raise ValueError("Receipt uses a different source snapshot")
        if receipt.get("returncode") != 0 or receipt.get("validation_errors"):
            raise ValueError("Failed receipt cannot enter selection")
        files = receipt.get("result_files", [])
        if len(files) != 1:
            raise ValueError("Expected exactly one replay summary per screening job")
        source = Path(files[0])
        if file_hash(source) != receipt.get("result_sha256", {}).get(source.name):
            raise ValueError("Result hash changed after completion")
        metrics, metadata = read_summary(source)
        if len(metrics) != job["expected_candidates"]:
            raise ValueError("Screening batch is incomplete")
        for name, result in metrics.items():
            if name not in candidates or name in rows:
                raise ValueError("Unexpected or duplicate candidate in screening results")
            rows[name] = {**compact(name, result), "family": candidates[name]["family"],
                          "fingerprint": candidates[name]["fingerprint"],
                          "approximate": metadata["approximate"],
                          "job": job["id"]}
        receipts.append({"id": job["id"], "sha256": file_hash(receipt_path),
                         "exe_sha256": receipt["exe_sha256"]})
    if len({r["exe_sha256"] for r in receipts}) > 1:
        raise ValueError("Different research instruments cannot share one ranking")
    return rows, {"plan_sha256": plan_hash, "manifest_sha256": file_hash(manifest_path),
                  "planned_candidates": manifest["candidate_count"],
                  "completed_candidates": len(rows), "missing_jobs": missing,
                  "interim": bool(missing), "receipts": receipts}


def select(rows: dict, count: int, min_activity_ratio: float = .7):
    if count < 2 or not 0 <= min_activity_ratio <= 1:
        raise ValueError("Invalid selection size or activity ratio")
    reference = rows["GOD-X7-cap5"]
    limits = {"filled_baskets": reference["filled_baskets"] * min_activity_ratio,
              "signal_utilization_pct": reference["signal_utilization_pct"] * min_activity_ratio}
    pool, rejected = [], []
    for row in rows.values():
        reasons = []
        if row["total_profit"] <= 0:
            reasons.append("nonpositive_profit")
        if row["blown"] or row["stop_outs"] or row["min_equity"] <= 0:
            reasons.append("insolvent_or_stop_out")
        if any(row[key] < minimum for key, minimum in limits.items()):
            reasons.append("below_activity_floor")
        (rejected if reasons else pool).append({**row, "screening_rejections": reasons})
    chosen, why = {}, defaultdict(list)

    def add(row, reason):
        if row["name"] not in chosen and len(chosen) >= count:
            return
        chosen[row["name"]] = row
        if reason not in why[row["name"]]:
            why[row["name"]].append(reason)

    add(reference, "mandatory_reference_with_source_validity_contract")
    by_family = defaultdict(list)
    for row in pool:
        by_family[row["family"]].append(row)
    for family, members in sorted(by_family.items()):
        add(max(members, key=lambda r: (r["total_profit"], r["win_days_pct"])), "family_profit_leader")
        add(max(members, key=lambda r: (r["win_days_pct"], r["total_profit"])), "family_positive_day_leader")
    views = [
        ("profit", sorted(pool, key=lambda r: (r["total_profit"], r["filled_baskets"]), reverse=True)),
        ("positive_trading_days", sorted(pool, key=lambda r: (r["win_days_pct"], r["total_profit"]), reverse=True)),
        ("activity", sorted(pool, key=lambda r: (r["filled_baskets"], r["total_profit"]), reverse=True)),
        ("profit_per_drawdown", sorted(pool, key=lambda r: (r["total_profit"] / max(r["max_daily_dd"], 1.), r["win_days_pct"]), reverse=True)),
    ]
    for index in range(len(pool)):
        for label, queue in views:
            add(queue[index], label)
        if len(chosen) >= count:
            break
    selection = [{**row, "selection_reasons": why[name]} for name, row in chosen.items()]
    return {"coronation_eligible": False, "selected_for_exact_replay": selection,
            "activity_floor": limits, "activity_ratio_of_reference": min_activity_ratio,
            "profitable_active_pool": len(pool), "rejected_count": len(rejected),
            "rejection_counts": dict(Counter(reason for r in rejected for reason in r["screening_rejections"])),
            "highest_positive_day_diagnostics": sorted(rejected, key=lambda r: (r["win_days_pct"], r["total_profit"]), reverse=True)[:10],
            "notes": ["Reference includes the mandatory source-validity contract; it is not an unchanged historical GOD-X7 result.",
                      "win_days_pct counts days with closed trades; positive_market_days_pct includes every day with ticks.",
                      "Daily reset end_equity is start deposit plus the sum of independent daily profits, not a continuous account balance.",
                      "Activity floor is a screening choice, not evidence against overfitting. Exact validation and owner selection remain required."]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--count", type=int, default=128)
    parser.add_argument("--min-activity-ratio", type=float, default=.7)
    args = parser.parse_args()
    rows, provenance = collect(args.plan, args.manifest)
    result = {"provenance": provenance, **select(rows, args.count, args.min_activity_ratio)}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"completed_candidates": len(rows), "interim": provenance["interim"],
                      "exact_queue": len(result["selected_for_exact_replay"]),
                      "profitable_active_pool": result["profitable_active_pool"]}))


if __name__ == "__main__":
    main()
