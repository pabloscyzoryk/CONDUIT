"""Prepare a deterministic quick-backtest candidate pool; never runs it."""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BASE = ROOT / "config/presets/GOD-X7.json"
DEFAULT_PRIOR = ROOT / "work/previous_sweep_manifest.json"
DEFAULT_OUT = ROOT / "work/quick_sweep"
FAMILY_COUNTS = {"neighborhood": 4000, "adaptive": 4000, "breadth": 2000}
SEED = 8_104_2026
LOT_MAX = 5.0

ADAPTIVE_DEFAULTS: dict[str, Any] = {
    "trail_adaptive_enabled": False,
    "trail_adaptive_runners_only": True,
    "trail_adaptive_window_s": 90.0,
    "trail_adaptive_min_samples": 8,
    "trail_adaptive_trend_er": 0.55,
    "trail_adaptive_reversal_er": 0.45,
    "trail_adaptive_trend_gap_mult": 1.60,
    "trail_adaptive_chop_gap_mult": 0.85,
    "trail_adaptive_reversal_gap_mult": 0.45,
    "trail_adaptive_fast_vol_s": 20.0,
    "trail_adaptive_slow_vol_s": 120.0,
    "trail_adaptive_vol_ratio": 1.80,
    "trail_adaptive_vol_favorable_mult": 1.25,
    "trail_adaptive_vol_adverse_mult": 0.65,
    "trail_adaptive_min_peak": 0.0,
    "trail_adaptive_min_gap": 0.0,
    "trail_adaptive_max_gap": 0.0,
}
ADAPTIVE_AXES = tuple(ADAPTIVE_DEFAULTS)

# Local search around the selected base preset. Values are symmetric around the
# base where practical and every gated family is activated below.
NEIGHBORHOOD: dict[str, list[Any]] = {
    "fast_addon_max": [0, 1, 2, 3],
    "fast_addon_move_usd": [3.0, 5.0, 8.0, 12.0, 16.0],
    "fast_addon_window_s": [30.0, 60.0, 90.0, 120.0, 240.0],
    "fast_addon_cooldown_s": [0.0, 30.0, 60.0, 120.0],
    "fast_addon_lot_mult": [0.35, 0.5, 0.75, 1.0, 1.25],
    "fast_addon_min_stage": [0, 1, 2],
    "market_hybrid_now_units": [0, 1, 2, 3],
    "market_hybrid_pending_units": [0, 1, 2, 4, 6],
    "market_hybrid_lot_mult": [0.35, 0.5, 0.75, 1.0],
    "market_hybrid_max_chase_usd": [0.3, 0.5, 0.8, 1.2],
    "market_hybrid_tp_stage": [0, 1, 2, 3],
    "pending_drop_on_target": [False, True],
    "pending_ttl_h": [6.0, 12.0, 18.0, 24.0, 36.0],
    "pending_lifetime": ["UntilTp1", "UntilTp2", "UntilTp3", "Never"],
    "out_at_entry_mode": ["CloseAll", "CloseLosersOnly", "CloseFlatOnly", "MoveSlToBe"],
    "smart_sl_mode": ["Ladder", "BreakevenOnly", "LadderWithBe", "Off"],
    "rearm_max_times": [0, 1, 2, 3],
    "rearm_min_gap_min": [5.0, 10.0, 15.0, 22.5, 30.0, 45.0],
    "bank_all_at_stage": [0, 1, 2, 3],
    "riskfree_trigger_usd": [5.0, 10.0, 20.0, 30.0, 50.0],
    "riskfree_trigger_r": [0.0, 0.25, 0.5, 0.75, 1.0],
    "riskfree_keep_units": [1, 2, 3],
    "riskfree_be_offset": [0.0, 0.1, 0.25, 0.5],
    "riskfree_runner_stop": ["BeOwn", "Be", "TrailGap"],
    "riskfree_runner_gap": [3.0, 5.0, 8.0, 12.0, 16.0],
    "trail_runners_n": [1, 2, 3, 4],
    "trail_runner_mode": ["Gap", "Atr", "Chandelier", "Tiered"],
    "trail_runner_start": [5.0, 8.0, 12.0, 16.0, 20.0, 30.0],
    "trail_runner_gap": [3.0, 5.0, 8.0, 12.0, 16.0, 20.0, 30.0],
    "trail_runner_lock_pct": [25.0, 40.0, 50.0, 65.0, 75.0, 85.0],
    "trail_sr_min_gain": [0.0, 3.0, 5.0, 8.0, 12.0, 20.0],
    "trail_sr_tf_min": [1, 2, 3, 5, 15],
    "trail_sr_fractal_n": [1, 2, 3, 4, 5],
    "trail_sr_offset": [0.1, 0.25, 0.5, 0.75, 1.0],
    "entry_weights_from_rr": [False, True],
    "entry_weights_rr_power": [0.35, 0.5, 0.75, 1.0, 1.5],
    "entry_weights_rr_cap": [2.0, 3.0, 4.0, 6.0, 8.0],
    "entry_depth_curve": [0.5, 0.75, 1.0, 1.25, 1.5, 2.0],
    "entry_deep_offset": [0.0, 0.5, 1.0, 1.5, 2.0],
    "max_open_baskets": [0, 1, 2, 3, 4, 6],
    "max_portfolio_risk_pct": [0.0, 20.0, 40.0, 60.0, 80.0],
    "risk_per_basket_pct": [5.0, 7.5, 10.0, 15.0, 20.0],
}

# These values may never be imported from a historical breadth patch.
FORBIDDEN = {
    "lot_max", "server_tz_offset_ms", "msg_clock_offset_ms", "exec_latency_ms",
    "slippage_pts", "ai_enabled", "ai_replaces_management", "ai_model",
    "ea_enabled", "mt5_autostart", "mt5_watchdog", "mt5_terminal_path",
    "journal_enabled", "journal_retention_days", "close_receipt_reconcile",
    "defer_entry_until_receipts", "closed_profit_net_costs",
    "restore_strategy_continuation", "mt5_follow_terminal_account",
    "tp_stage_from_broker_fill", "only_limit_signals", "signal_filter",
    "session_filter", "side_filter", "skip_tags", "require_tags",
    "regime_filter", "trend_filter_enabled", "daily_signal_budget",
    "signal_min_rr", "signal_min_zone_width", "signal_max_zone_width",
}


def packed(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(",", ":"), allow_nan=False).encode("utf-8")


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def load(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def write_atomic(path: Path, value: Any) -> None:
    data = json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n"
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(data, encoding="utf-8")
    tmp.replace(path)


def activate(settings: dict[str, Any], varied: set[str]) -> None:
    if any(k.startswith("market_hybrid_") and k != "market_hybrid_now_units" for k in varied):
        settings["auto_limit"] = True
        settings["market_hybrid_now_units"] = max(1, int(settings.get("market_hybrid_now_units", 0)))
    if any(k.startswith("riskfree_") for k in varied):
        settings["riskfree_enabled"] = True
        if float(settings.get("riskfree_trigger_usd", 0)) <= 0 and float(settings.get("riskfree_trigger_r", 0)) <= 0:
            settings["riskfree_trigger_usd"] = 20.0
    if any(k.startswith("trail_runner_") or k == "trail_runners_n" for k in varied):
        settings["trail_split"] = True
        if settings.get("trail_runner_mode") == "Off":
            settings["trail_runner_mode"] = "Gap"
    if any(k.startswith("trail_sr_") for k in varied):
        settings["trail_sr_enabled"] = True
    if any(k.startswith("rearm_") for k in varied):
        settings["rearm_grid_on_return"] = True
        settings["rearm_max_times"] = max(1, int(settings.get("rearm_max_times", 0)))
    if any(k.startswith("entry_weights_") for k in varied):
        settings["entry_weights_from_rr"] = True
        settings["risk_per_basket_pct"] = max(5.0, float(settings.get("risk_per_basket_pct", 0)))


def adaptive_patch(rng: random.Random) -> dict[str, Any]:
    fast = rng.choice([10.0, 15.0, 20.0, 30.0, 45.0])
    slow = rng.choice([v for v in [60.0, 90.0, 120.0, 180.0, 300.0] if v > fast])
    min_gap = rng.choice([0.0, 1.0, 2.0, 3.0, 5.0])
    max_gap = rng.choice([0.0, 12.0, 20.0, 30.0, 45.0])
    if max_gap > 0 and max_gap < min_gap:
        max_gap = min_gap
    runners_only = rng.random() < 0.78
    patch: dict[str, Any] = {
        "trail_adaptive_enabled": True,
        "trail_adaptive_runners_only": runners_only,
        "trail_adaptive_window_s": rng.choice([30.0, 60.0, 90.0, 150.0, 240.0]),
        "trail_adaptive_min_samples": rng.choice([4, 6, 8, 12, 20]),
        "trail_adaptive_trend_er": rng.choice([0.40, 0.55, 0.70, 0.85]),
        "trail_adaptive_reversal_er": rng.choice([0.25, 0.35, 0.45, 0.60]),
        "trail_adaptive_trend_gap_mult": rng.choice([1.15, 1.35, 1.60, 2.0, 2.5]),
        "trail_adaptive_chop_gap_mult": rng.choice([0.50, 0.65, 0.75, 0.85, 1.0]),
        "trail_adaptive_reversal_gap_mult": rng.choice([0.25, 0.35, 0.45, 0.65, 0.85]),
        "trail_adaptive_fast_vol_s": fast,
        "trail_adaptive_slow_vol_s": slow,
        "trail_adaptive_vol_ratio": rng.choice([1.25, 1.5, 1.8, 2.2, 2.8]),
        "trail_adaptive_vol_favorable_mult": rng.choice([1.05, 1.15, 1.25, 1.5]),
        "trail_adaptive_vol_adverse_mult": rng.choice([0.35, 0.50, 0.65, 0.85]),
        "trail_adaptive_min_peak": rng.choice([0.0, 3.0, 5.0, 8.0, 12.0]),
        "trail_adaptive_min_gap": min_gap,
        "trail_adaptive_max_gap": max_gap,
        "trail_split": True,
        "trail_runners_n": rng.choice([1, 1, 2, 3]),
        "trail_runners_by_depth": rng.choice([True, True, False]),
        "trail_runner_mode": rng.choice(["Gap", "Atr", "Chandelier"]),
        "trail_runner_start": rng.choice([5.0, 8.0, 12.0, 16.0, 20.0, 30.0]),
        "trail_runner_gap": rng.choice([3.0, 5.0, 8.0, 12.0, 16.0, 20.0, 30.0]),
    }
    if not runners_only:
        patch["trail_mode"] = rng.choice(["Gap", "Atr", "Chandelier"])
        patch["trail_start"] = rng.choice([8.0, 12.0, 16.0, 20.0])
        patch["trail_gap"] = rng.choice([5.0, 8.0, 12.0, 16.0, 20.0])
        patch["trail_atr_mult"] = rng.choice([0.75, 1.0, 1.5, 2.0])
    return patch


def validate(settings: dict[str, Any], family: str) -> None:
    if float(settings.get("lot_max", 0)) != LOT_MAX:
        raise ValueError("lot_max cap is not exactly 5")
    if family == "adaptive":
        missing = [k for k in ADAPTIVE_AXES if k not in settings]
        if missing or not settings["trail_adaptive_enabled"]:
            raise ValueError(f"dead/incomplete adaptive family: {missing}")
        if float(settings["trail_adaptive_fast_vol_s"]) >= float(settings["trail_adaptive_slow_vol_s"]):
            raise ValueError("adaptive fast window must be shorter than slow window")
        lo, hi = float(settings["trail_adaptive_min_gap"]), float(settings["trail_adaptive_max_gap"])
        if hi > 0 and hi < lo:
            raise ValueError("adaptive gap clamps reversed")


def make_row(base_doc: dict[str, Any], base_settings: dict[str, Any], family: str,
             patch: dict[str, Any], ordinal: int, seen: set[str]) -> dict[str, Any] | None:
    settings = copy.deepcopy(base_settings)
    patch = {k: v for k, v in patch.items() if k not in FORBIDDEN}
    settings.update(patch)
    activate(settings, set(patch))
    settings["lot_max"] = LOT_MAX
    validate(settings, family)
    settings_hash = sha(packed(settings))
    if settings_hash in seen:
        return None
    seen.add(settings_hash)
    name = f"QS1-{family[:3].upper()}-{ordinal:04d}"
    doc = copy.deepcopy(base_doc)
    doc["name"] = name
    doc["nazwa"] = name
    doc["description"] = (
        "quick-backtest research candidate; hard lot_max=5; "
        "approximate stages are screening only and require exact N=1 validation."
    )
    doc["settings"] = settings
    payload = (json.dumps(doc, ensure_ascii=False, indent=1, allow_nan=False) + "\n").encode("utf-8")
    return {"name": name, "family": family, "primary_patch": patch,
            "settings_sha256": settings_hash, "payload_sha256": sha(payload),
            "payload": payload}


def generate(base_doc: dict[str, Any], prior: dict[str, Any]) -> dict[str, list[dict[str, Any]]]:
    base_settings = copy.deepcopy(base_doc["settings"])
    for key, value in ADAPTIVE_DEFAULTS.items():
        base_settings.setdefault(key, value)
    base_settings["lot_max"] = LOT_MAX
    rng = random.Random(SEED)
    seen: set[str] = set()
    rows = {family: [] for family in FAMILY_COUNTS}

    def add(family: str, patch: dict[str, Any]) -> None:
        row = make_row(base_doc, base_settings, family, patch, len(rows[family]), seen)
        if row is not None:
            rows[family].append(row)

    add("neighborhood", {})  # exact base control, except the requested cap 5
    keys = sorted(k for k in NEIGHBORHOOD if k in base_settings)
    attempts = 0
    while len(rows["neighborhood"]) < FAMILY_COUNTS["neighborhood"]:
        chosen = rng.sample(keys, rng.choice([2, 3, 4, 5, 6]))
        add("neighborhood", {k: rng.choice(NEIGHBORHOOD[k]) for k in chosen})
        attempts += 1
        if attempts > 300_000:
            raise RuntimeError("neighborhood uniqueness exhausted")

    attempts = 0
    while len(rows["adaptive"]) < FAMILY_COUNTS["adaptive"]:
        patch = adaptive_patch(rng)
        if rng.random() < 0.55:
            chosen = rng.sample(keys, rng.choice([1, 2, 3]))
            patch.update({k: rng.choice(NEIGHBORHOOD[k]) for k in chosen})
        add("adaptive", patch)
        attempts += 1
        if attempts > 300_000:
            raise RuntimeError("adaptive uniqueness exhausted")

    prior_rows = list(prior.get("queue", []))
    if not prior_rows:
        raise RuntimeError("prior GIGA7 manifest has no queue")
    cursor = 0
    attempts = 0
    while len(rows["breadth"]) < FAMILY_COUNTS["breadth"]:
        source = prior_rows[cursor % len(prior_rows)]
        patch = dict(source.get("primary_patch") or {})
        patch.update(source.get("activation_patch") or {})
        # Later cycles add a small local interaction, retaining the prior axis.
        if cursor >= len(prior_rows):
            k = keys[(cursor * 17) % len(keys)]
            patch[k] = rng.choice(NEIGHBORHOOD[k])
        add("breadth", patch)
        cursor += 1
        attempts += 1
        if attempts > 100_000:
            raise RuntimeError("breadth uniqueness exhausted")
    return rows


def link_or_copy(source: Path, target: Path) -> None:
    if target.exists():
        return
    try:
        os.link(source, target)
    except OSError:
        shutil.copy2(source, target)


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--base", type=Path, default=DEFAULT_BASE)
    p.add_argument("--prior-manifest", type=Path, default=DEFAULT_PRIOR)
    p.add_argument("--out", type=Path, default=DEFAULT_OUT)
    p.add_argument("--resume", action="store_true")
    p.add_argument("--pilot-count", type=int, default=96)
    return p.parse_args()


def main() -> None:
    args = parse_args()
    base_path, prior_path, out = args.base.resolve(), args.prior_manifest.resolve(), args.out.resolve()
    for path in (base_path, prior_path):
        if not path.is_file():
            raise SystemExit(f"missing input: {path}")
    if args.pilot_count < 72 or args.pilot_count % 3:
        raise SystemExit("--pilot-count must be >=72 and divisible by 3")
    base_doc, prior = load(base_path), load(prior_path)
    if not isinstance(base_doc.get("settings"), dict):
        raise SystemExit("base preset has no settings object")
    state = {"schema": "conduit.quick-sweep1-generation.v1", "seed": SEED,
             "base": str(base_path), "base_sha256": file_sha(base_path),
             "prior_manifest": str(prior_path), "prior_sha256": file_sha(prior_path),
             "count": sum(FAMILY_COUNTS.values()), "families": FAMILY_COUNTS,
             "lot_max": LOT_MAX, "pilot_count": args.pilot_count}
    state_path = out / "generation_state.json"
    if out.exists():
        if not args.resume:
            raise SystemExit(f"refuse to overwrite {out}; use --resume")
        if not state_path.is_file() or load(state_path) != state:
            raise SystemExit("resume refused: generation state mismatch")
    else:
        out.mkdir(parents=True)
        write_atomic(state_path, state)

    rows = generate(base_doc, prior)
    configs = out / "configs"
    configs.mkdir(exist_ok=True)
    queue: list[dict[str, Any]] = []
    # 10-row deterministic interleave: 4 neighborhood, 4 adaptive, 2 breadth.
    for block in range(1000):
        order = [("neighborhood", block * 4 + i) for i in range(4)]
        order += [("adaptive", block * 4 + i) for i in range(4)]
        order += [("breadth", block * 2 + i) for i in range(2)]
        for family, index in order:
            row = dict(rows[family][index])
            payload = row.pop("payload")
            queue_index = len(queue)
            filename = f"QS1-{queue_index:05d}-{family}.json"
            path = configs / filename
            if path.exists():
                if file_sha(path) != row["payload_sha256"]:
                    raise RuntimeError(f"resume hash mismatch: {path}")
            else:
                path.write_bytes(payload)
            queue.append({"queue_index": queue_index, "file": filename, **row})

    if len(queue) != 10_000 or len({r["settings_sha256"] for r in queue}) != 10_000:
        raise RuntimeError("10k uniqueness contract failed")
    if any(float(load(configs / r["file"])["settings"]["lot_max"]) != LOT_MAX for r in queue):
        raise RuntimeError("lot cap disk audit failed")

    per_family = args.pilot_count // 3
    pilot_rows: list[dict[str, Any]] = []
    for family in FAMILY_COUNTS:
        family_rows = [r for r in queue if r["family"] == family]
        # Include the control and cover the whole deterministic family range.
        indices = sorted({round(i * (len(family_rows) - 1) / (per_family - 1))
                          for i in range(per_family)})
        pilot_rows.extend(family_rows[i] for i in indices)
    pilot_rows.sort(key=lambda r: r["queue_index"])
    pilot_dir = out / f"pilot{len(pilot_rows)}_configs"
    pilot_dir.mkdir(exist_ok=True)
    for row in pilot_rows:
        link_or_copy(configs / row["file"], pilot_dir / row["file"])

    manifest = {**state, "status": "PREPARED_NOT_STARTED", "automatic_coronation": False,
                "approximate_results_coronation_eligible": False,
                "exact_n1_required_for_top24": True,
                "families_interleave": "4 neighborhood + 4 adaptive + 2 breadth per 10",
                "adaptive_axes": list(ADAPTIVE_AXES),
                "pilot": {"count": len(pilot_rows), "config_dir": str(pilot_dir),
                          "queue_indices": [r["queue_index"] for r in pilot_rows]},
                "queue": queue,
                "warnings": [
                    "Prepared only; this script never starts btp.",
                    "Every approximate finalist must be rerun with N=1.",
                    "10,000/30 min is not promised: the launcher enforces its measured budget gate.",
                ]}
    write_atomic(out / "manifest.json", manifest)
    print(json.dumps({k: v for k, v in manifest.items() if k != "queue"},
                     ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
