"""Prepare the owner's complete fresh-account coronation matrix, without running it.

Every calendar window has its own process/account. The candidate names remain
research names; only the owner can select and name a production GOD-X8 preset.
"""
from __future__ import annotations
import argparse
import copy
import datetime as dt
import json
from pathlib import Path

from giga_sweep8 import fingerprint
from research_runner import file_hash


def calendar_windows(start: dt.date, end: dt.date, tick_days: list[str]):
    if start >= end:
        raise ValueError("Window end is exclusive and must follow its start")
    active = sorted({dt.date.fromisoformat(day) for day in tick_days
                     if start <= dt.date.fromisoformat(day) < end})
    if not active:
        raise ValueError("No market days in the requested full window")
    windows = [{"id": "full", "kind": "full", "from": start.isoformat(), "to": end.isoformat()}]
    for day in active:
        windows.append({"id": "day_" + day.isoformat(), "kind": "day",
                        "from": day.isoformat(), "to": (day + dt.timedelta(days=1)).isoformat()})
    weeks = sorted({day - dt.timedelta(days=day.weekday()) for day in active})
    months = sorted({day.replace(day=1) for day in active})
    for week in weeks:
        windows.append({"id": "week_" + week.isoformat(), "kind": "week",
                        "from": max(start, week).isoformat(),
                        "to": min(end, week + dt.timedelta(days=7)).isoformat()})
    for month in months:
        after = dt.date(month.year + (month.month == 12), month.month % 12 + 1, 1)
        windows.append({"id": "month_" + month.strftime("%Y-%m"), "kind": "month",
                        "from": max(start, month).isoformat(), "to": min(end, after).isoformat()})
    return windows


def validate_candidate_names(names):
    # Output targets are Windows filenames. Case-only differences collide too.
    allowed = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-"
    if any(not isinstance(name, str) or not name or any(char not in allowed for char in name) for name in names):
        raise ValueError("Safe research candidate identifiers required")
    if len({name.casefold() for name in names}) != len(names):
        raise ValueError("Candidate identifiers must remain unique including the optional reference")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--finalists", required=True, type=Path,
                        help="JSON list of five to ten objects with id, preset_path, fingerprint")
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--source-manifest", required=True, type=Path)
    parser.add_argument("--ticks", required=True, type=Path)
    parser.add_argument("--tick-manifest", required=True, type=Path)
    parser.add_argument("--signals", required=True, type=Path)
    parser.add_argument("--signal-contract", required=True,
                        choices=["historical_reference", "observed_receipts", "mixed_missing_original_stress"],
                        help="Keep the historical publication benchmark distinct from actual receipt replay")
    parser.add_argument("--trade-sessions", required=True, type=Path,
                        help="Measured broker trade-session JSON used by the qualifying search")
    parser.add_argument("--from", dest="start", required=True, type=dt.date.fromisoformat)
    parser.add_argument("--to", dest="end", required=True, type=dt.date.fromisoformat)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--progress-dir", required=True, type=Path)
    parser.add_argument("--stop-at", required=True, help="ISO deadline with UTC offset")
    parser.add_argument("--source-state", required=True, choices=["fresh_engine"],
                        help="Explicit reset semantics. Carrying earlier source signals is not implemented by this plan.")
    parser.add_argument("--include-reference", type=Path,
                        help="Optional extra GOD-X7 reference, outside the five to ten finalists")
    args = parser.parse_args()
    if args.output.exists():
        raise FileExistsError(args.output)
    if dt.datetime.fromisoformat(args.stop_at).tzinfo is None:
        raise ValueError("Deadline must include its timezone")
    finalists = json.loads(args.finalists.read_text("utf-8-sig"))
    if not 5 <= len(finalists) <= 10 or len({row["id"] for row in finalists}) != len(finalists):
        raise ValueError("Exactly five to ten unique finalists are required")
    loaded = []
    for row in finalists:
        doc = json.loads(Path(row["preset_path"]).read_text("utf-8-sig"))
        if fingerprint(doc["settings"]) != row["fingerprint"]:
            raise ValueError("Finalist changed since selection")
        if doc["settings"].get("lot_max") != 5:
            raise ValueError("Finalists must come from the common cap-5 search")
        if "GOD-X8" in row["id"].upper():
            raise ValueError("Production GOD-X8 name is reserved for the owner's decision")
        loaded.append((row["id"], doc))
    if args.include_reference:
        loaded.append(("GOD-X7-reference", json.loads(args.include_reference.read_text("utf-8-sig"))))
    validate_candidate_names([name for name, _ in loaded])
    # Legacy research arithmetic has no broker-volume ceiling. V2 deliberately
    # enforces one; silently calling that path unlimited would be misleading.
    if any(doc["settings"].get("order_volume_contract_v2", False) for _, doc in loaded):
        raise ValueError("V2 requires an explicit arithmetic broker-volume profile before an unlimited coronation run")
    tick_manifest = json.loads(args.tick_manifest.read_text("utf-8-sig"))
    windows = calendar_windows(args.start, args.end, list(tick_manifest["per_day_ticks"]))
    output = args.output.resolve()
    output.mkdir(parents=True)
    presets = {}
    for label, cap in [("lot001", .01), ("lot10", 10.), ("arithmetic", 0.)]:
        folder = output / "presets" / label
        folder.mkdir(parents=True)
        presets[label] = folder
        for name, original in loaded:
            doc = copy.deepcopy(original)
            doc.update({"name": name, "nazwa": name})
            doc["settings"]["lot_max"] = cap
            (folder / (name + ".json")).write_text(json.dumps(doc, indent=2), encoding="utf-8")
    jobs = []
    worker_threads = max(n for n in (1, 2, 3, 4, 6, 8) if n <= len(loaded))
    for window in windows:
        for deposit in (300, 600):
            for label, folder in presets.items():
                identity = f"{window['id']}_{deposit}_{label}"
                result = output / identity / "results"
                argv = [str(args.exe.resolve()), "--ticks", str(args.ticks.resolve()),
                        "--signals", str(args.signals.resolve()), "--sweep", str(folder),
                        "--from", window["from"], "--to", window["to"],
                        "--balance", str(deposit), "--signal-time-offset-min", "0",
                        "--sim-limit-price-improvement", "--sim-price-digits", "2",
                        "--sim-new-pending-sl-next-tick", "--sim-native-swap-cash-digits", "2",
                        "--sim-trade-sessions", str(args.trade_sessions.resolve()),
                        "--rozgrzewka-h", "72",
                        "--quick-tick-stride", "1", "--no-charts", "--dump-trades", "--out", str(result)]
                if args.signal_contract != "historical_reference":
                    argv.append("--live-telegram-ingress")
                jobs.append({"id": identity, "argv": argv, "result_dir": str(result),
                             "threads": worker_threads, "expected_candidates": len(loaded),
                             "window": window, "deposit": deposit, "cap_mode": label,
                             "signal_contract": args.signal_contract})
    inputs = [{"path": str(path.resolve()), "sha256": file_hash(path)}
              for path in (args.finalists, args.source_manifest, args.signals, args.tick_manifest,
                           args.exe, args.trade_sessions)]
    inputs.append({"path": str(args.ticks.resolve()), "sha256": tick_manifest["output_sha256"]})
    for folder in presets.values():
        inputs.extend({"path": str(p), "sha256": file_hash(p)} for p in sorted(folder.glob("*.json")))
    plan = {"id": "giga_sweep8_coronation_" + args.signal_contract, "name": "GOD-X8 — koronacja kandydatów",
            "threads": 24, "output": str(output), "progress_dir": str(args.progress_dir.resolve()),
            "source_revision": file_hash(args.source_manifest), "stop_at": args.stop_at,
            "inputs": inputs, "jobs": jobs,
            "metadata": {"etap_badania": "Koronacja — dokładne niezależne rachunki",
                         "tryb_obliczen": "exact", "kanal": "Synergy", "kandydaci": len(finalists),
                         "status_walidacji": "coronation", "source_state": args.source_state},
            "protocol": {"window_boundaries": "inclusive from, exclusive to; broker wall-clock dates",
                         "signal_contract": args.signal_contract,
                         "separate_contracts": "Never combine profits across overlapping histories or describe them as independent samples.",
                         "historical_reference": "Publication-final benchmark assumptions; unrecorded original versions remain unknown.",
                         "observed_receipts": "Only actually recorded message receipts; no reconstruction of earlier positions or missing originals.",
                         "mixed_missing_original_stress": "Missing-original stress evidence; not a complete historical live replay.",
                         "week": "calendar Monday through Sunday, clipped to the full window",
                         "source_state": args.source_state,
                         "broker_trade_sessions": {"path": str(args.trade_sessions.resolve()),
                                                   "sha256": file_hash(args.trade_sessions)},
                         "market_warmup_hours": 72, "warmup_trading": False,
                         "best_day_exclusion": "only max lot 0.01; never subtract days from compounding at higher caps",
                         "arithmetic": "lot_max=0; strategy sizing, margin and risk rules remain active; no claim of broker-executable unlimited orders",
                         "coronation_eligible_without_owner_decision": False},
            "windows": windows}
    (output / "plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"windows": len(windows), "jobs": len(jobs),
                      "candidate_runs": len(jobs) * len(loaded), "launched": False}))


if __name__ == "__main__":
    main()
