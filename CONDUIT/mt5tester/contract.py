"""Build explicit tester inputs and expose every unsupported core setting.

The defaults file must come from --dump-settings on the exact btp binary being
compared. A compatibility report describes the mapping; it is not proof of
execution parity. Reports and parameters belong in a private experiment folder.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

from mapping import MAP, ENUMY, MOST_ONLY, BROKER_SPEC_FIELDS

RUNTIME = {
    "mt5_autostart", "mt5_watchdog", "mt5_terminal_path", "mt5_retry_attempts",
    "mt5_retry_delay_s", "mt5_restart_after", "mt5_health_interval_s",
    "journal_enabled", "journal_min_level", "journal_snapshots",
    "journal_excursions", "journal_text_mirror", "journal_retention_days",
    "journal_buffer_cap", "stat_be_prog_usd", "restore_strategy_continuation",
    "server_tz_offset_ms", "msg_clock_offset_ms",
}

# These features have no EA implementation. Only their inactive value is valid.
INACTIVE = {
    "entry_deep_frac_to_sl": 0, "toucher_units": 0, "signal_filter": False,
    "pending_resize_on_vol": False, "units_by_hour": "", "ai_enabled": False,
    "risk_free_be_min_profit": 0, "partials_wykonuj": False,
    "entry_edit_geometry_v2": False, "market_hybrid_now_units": 0,
    "market_hybrid_pending_units": 0, "market_unfilled_cancel_stage": 0,
    "oae_skip_after_riskfree": False, "no_reenter_from_stage": 0,
    "sl_wlasny_na_pozycje": 0, "runner_partial_pct": 0,
    "trail_sr_min_prominence_atr": 0, "trail_sr_offset_atr_mult": 0,
    "trail_sr_offset_spread_mult": 0, "limit_kasuje_tylko_nadmiar": False,
    "skip_tags": "", "require_tags": "", "regime_pilnuj_limitow": False,
    "regime_strefa_martwa": 0, "regime_okno2_h": 0,
    "regime_zmiennosc_min": 0, "regime_zmiennosc_max": 0,
    "sanity_zone_max": 0, "sanity_tp_max": 0, "sanity_tp_rosnace": False,
    "sanity_tp_strona": False, "slhit_pause_lot_mult": 0,
    "cel_z_przeciwnego": "Off", "zakaz_ponizej_krawedzi": False,
    "runner_ksiegowanie_v2": False, "msg_kurs_sprzed_luki": False,
    "runner_max_hold_bez_reguly": False, "trail_adaptive_enabled": False,
    "vol_size_mode": "Off", "ea_enabled": False,
    "order_volume_contract_v2": False, "pending_relot_reconcile_target": False,
    "tp_price_only_strict": False,
}

# Fixed assumptions still require behavioral regression comparisons. Values
# outside this contract are rejected instead of silently changing strategy.
FIXED = {
    "entry_idempotencja": True, "close_all_scope": "Global",
    "sesja_bramka": "Sygnal", "regime_cena": "Rynkowa",
    "regime_miara": "Srednia", "regime_gdy_rozerwany": "Milcz",
    "day_trail_basis": "EquityPeak", "sim_validate_pending_stops": True,
    "sim_margin_check_on_fill": True, "basket_realized_broker_only": True,
    "close_receipt_reconcile": True, "defer_entry_until_receipts": True,
    "sim_margin_at_market": False, "closed_profit_net_costs": False,
    "credit_balance_separate": False,
}

GROUPS = [
    ("trail_adaptive_", "trail_adaptive_enabled", False),
    ("vol_size_", "vol_size_mode", "Off"),
    ("ea_", "ea_enabled", False),
    ("ai_", "ai_enabled", False),
    ("toucher_", "toucher_units", 0),
    ("market_hybrid_", "market_hybrid_now_units", 0),
]


def fingerprint(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build(defaults: dict, explicit: dict, source: str, bridge: str,
          available_inputs: set[str] | None = None) -> tuple[dict, dict]:
    settings = {**defaults, **explicit}
    available = available_inputs if available_inputs is not None else set(re.findall(r"^\s*input\s+\w+\s+(In_\w+)", source, re.M))
    parameters = {"In_Plik": bridge, "In_Magic": 770077,
                  "In_MostRequireSchema2": True, "In_ResetTpOnTargetEdit": True}
    states = {}
    errors = []
    old_profit_budget = not any(name in settings for name in (
        "profit_budget_arm_pct", "profit_budget_keep_pct", "profit_budget_deploy_pct"))
    for name, target, convert in MAP:
        if name not in settings:
            if old_profit_budget and name.startswith("profit_budget_"):
                parameters[target] = {"profit_budget_arm_pct": 0.0,
                                      "profit_budget_keep_pct": 50.0,
                                      "profit_budget_deploy_pct": 100.0}[name]
                states[name] = "older_binary_feature_absent_disabled_in_ea"
                continue
            if name == "explicit_pending_until_cancel":
                parameters[target] = False
                states[name] = "older_binary_feature_absent_disabled_in_ea"
                continue
            errors.append({"field": name, "reason": "missing_from_binary_defaults"})
            continue
        if target not in available:
            errors.append({"field": name, "reason": "missing_ea_input"})
            continue
        value = settings[name]
        if convert is bool and not isinstance(value, bool):
            errors.append({"field": name, "reason": "boolean_type_required"})
            continue
        parameters[target] = convert(value)
        states[name] = "mapped"
    for name, (target, values) in ENUMY.items():
        value = settings.get(name)
        if target not in available or value not in values:
            errors.append({"field": name, "reason": "missing_or_unsupported_enum"})
            continue
        parameters[target] = values[value]
        states[name] = "mapped"

    for name, value in settings.items():
        if name in states:
            continue
        if name in RUNTIME:
            states[name] = "runtime_or_clock_configured_separately"
        elif name in MOST_ONLY:
            states[name] = "shared_parser_bridge"
        elif name in BROKER_SPEC_FIELDS or name == "stop_out_level_pct":
            states[name] = "native_broker_requires_matching_simulator_profile"
        elif name in INACTIVE and value == INACTIVE[name]:
            states[name] = "unsupported_feature_inactive"
        elif name in FIXED and value == FIXED[name]:
            states[name] = "fixed_ea_assumption_requires_regression"
        elif any(name.startswith(prefix) and settings.get(switch) == off
                 for prefix, switch, off in GROUPS):
            # Hybrid configuration is inactive only when BOTH entry legs are off.
            if name.startswith("market_hybrid_") and settings.get("market_hybrid_pending_units", 0) != 0:
                states[name] = "unmapped_requires_review"
            else:
                states[name] = "disabled_feature_parameter"
        elif name == "partials_pct" and not settings.get("partials_wykonuj"):
            states[name] = "disabled_feature_parameter"
        elif name == "cel_z_przeciwnego_zapas" and settings.get("cel_z_przeciwnego") == "Off":
            states[name] = "disabled_feature_parameter"
        elif name == "regime_percentyl" and settings.get("regime_miara") != "Percentyl":
            states[name] = "disabled_feature_parameter"
        elif name == "trail_sr_atr_period" and all(settings.get(k, 0) == 0 for k in (
                "trail_sr_min_prominence_atr", "trail_sr_offset_atr_mult")):
            states[name] = "disabled_feature_parameter"
        elif name == "sr_warmup_exact_ticks":
            states[name] = "warmup_policy_requires_matching_window"
        elif name == "deferred_entry_max_age_s":
            states[name] = "native_synchronous_execution_no_receipt_delay"
        else:
            states[name] = "unmapped_requires_review"
        if states[name] == "unmapped_requires_review":
            errors.append({"field": name, "reason": "unsupported_active_or_unclassified_setting"})
    for target in parameters:
        if target not in available:
            errors.append({"field": target, "reason": "missing_ea_harness_input"})
    if settings.get("trail_adaptive_enabled"):
        horizon = min(86400, max(settings.get("vol_window_min", 0),
                                settings.get("rev_exit_window_min", 0), 60) * 120)
        requested = max(settings.get(name, 0) for name in (
            "trail_adaptive_window_s", "trail_adaptive_fast_vol_s", "trail_adaptive_slow_vol_s"))
        if requested > horizon:
            errors.append({"field": "trail_adaptive_window_s", "reason": "adaptive_window_exceeds_shared_history_retention"})
    report = {"schema": "conduit.mt5.contract.v1", "mapping_complete": not errors,
              "execution_parity_proven": False, "setting_count": len(settings),
              "explicit_parameter_count": len(parameters), "fields": states, "errors": errors,
              "limitations": [
                  "Native broker costs, leverage and execution rules must match the simulator.",
                  "The EA is an independent management implementation; mapping alone proves no parity.",
                  "Synchronous no-fault runs do not prove receipt, rejection or restart parity.",
              ]}
    return parameters, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--defaults", type=Path, required=True)
    parser.add_argument("--preset", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--bridge", required=True, help="Relative path inside MT5 Common/Files.")
    parser.add_argument("--diagnostic", action="store_true")
    args = parser.parse_args()
    if Path(args.bridge).is_absolute() or ".." in Path(args.bridge).parts:
        raise ValueError("Bridge must be a relative Common/Files path without traversal.")
    defaults = json.loads(args.defaults.read_text(encoding="utf-8-sig"))
    preset = json.loads(args.preset.read_text(encoding="utf-8-sig"))
    source = Path(__file__).resolve().parents[1] / "mql5" / "CONDUIT_XT.mq5"
    parameters, report = build(defaults, preset.get("settings", preset), source.read_text(encoding="utf-8"), args.bridge)
    parameters["In_Diag"] = args.diagnostic
    report["fingerprints"] = {"defaults": fingerprint(args.defaults), "preset": fingerprint(args.preset), "expert": fingerprint(source)}
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "contract.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    # A diagnostic experiment can still use these inputs to locate the first
    # divergence. It must retain the incomplete contract in its evidence.
    (args.out / "parameters.json").write_text(json.dumps(parameters, indent=2), encoding="utf-8")
    print(json.dumps({"mapping_complete": report["mapping_complete"], "errors": report["errors"],
                      "setting_count": report["setting_count"], "execution_parity_proven": False}))
    raise SystemExit(0 if report["mapping_complete"] else 2)


if __name__ == "__main__":
    main()
