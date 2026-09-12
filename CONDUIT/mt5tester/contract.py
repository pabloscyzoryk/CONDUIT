"""Build explicit tester inputs and expose every unsupported core setting.

The defaults file must come from --dump-settings on the exact btp binary being
compared. A compatibility report describes the mapping; it is not proof of
execution parity. Reports and parameters belong in a private experiment folder.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
from pathlib import Path

from mapping import MAP, ENUMY, MOST_ONLY, BROKER_SPEC_FIELDS

LOT_GROWTH_DEFAULTS = {
    "lot_growth_allocation": "Uniform",
    "lot_growth_equity_stress_strength": 0.0,
    "lot_growth_portfolio_load_strength": 0.0,
    "lot_growth_direction_load_strength": 0.0,
    "lot_growth_basket_count_strength": 0.0,
    "lot_growth_spread_stress_strength": 0.0,
    "lot_growth_tp1_deficit_strength": 0.0,
    "lot_growth_stop_width_strength": 0.0,
    "lot_growth_age_decay_strength": 0.0,
    "lot_growth_rearm_decay_strength": 0.0,
    "lot_growth_day_dd_strength": 0.0,

    "lot_growth_mode": "Off",
    "lot_growth_reference_lot": 0.01,
    "lot_growth_reference_balance": 1000.0,
    "lot_growth_power": 0.7,
    "lot_growth_rate_pct": 0.35,
    "lot_growth_capital_multiple": 2.0,
    "lot_growth_lot_multiple": 1.5,
    "lot_growth_basket_risk_pct": 0.0,
}

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
    "close_all_scope": "Global",
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


def wire_action_error(action: str, capacity: int) -> str | None:
    """Validate generated numeric wire fields before permissive MQL conversion."""
    kind, separator, payload = action.partition(":")
    fields = payload.split(",")
    if not separator or not fields[0]:
        return "missing_action_key"

    def number(index, optional=False):
        if index >= len(fields) or not fields[index]:
            return False
        if optional and fields[index] == "nan":
            return True
        try:
            return math.isfinite(float(fields[index]))
        except ValueError:
            return False

    def integer(index, minimum=0):
        return index < len(fields) and re.fullmatch(r"-?\d+", fields[index]) is not None and int(fields[index]) >= minimum

    def targets(start):
        values = fields[start:]
        if values == [""]:
            values = []
        return all(number(index) for index in range(start, start + len(values)))

    valid = False
    if kind in {"ENTRY", "ENTRY2"}:
        v2 = kind == "ENTRY2"
        minimum = 13 if v2 else 7
        booleans = [2, 3, 7, 9, 10, 11] if v2 else [2, 6]
        valid = (len(fields) >= minimum and fields[1] in {"BUY", "SELL"}
                 and all(fields[i] in {"0", "1"} for i in booleans)
                 and all(number(i) for i in ([4, 5] if v2 else [3, 4]))
                 and all(number(i, True) for i in ([6, 8] if v2 else [5]))
                 and (not v2 or integer(12)) and targets(minimum))
        if valid and v2:
            target_count = len(fields) - minimum
            if fields[minimum:] == [""]:
                target_count = 0
            if int(fields[12]) != target_count:
                return "malformed_target_count"
    elif kind in {"TPHIT", "TPHIT2"}:
        valid = len(fields) == (4 if kind == "TPHIT2" else 2) and integer(1, -1)
        if kind == "TPHIT2":
            valid = valid and number(2, True) and fields[3] in {"0", "1"}
    elif kind == "SPP":
        valid = len(fields) >= 3 and number(1, True) and number(2, True) and targets(3)
    elif kind == "RF":
        valid = len(fields) == 2 and number(1, True)
    elif kind == "SETSL":
        valid = len(fields) == 2 and number(1)
    elif kind == "TPCORR":
        valid = len(fields) == 3 and integer(1) and number(2)
        if valid and int(fields[1]) > capacity:
            return "native_target_capacity"
    elif kind in {"INFO", "SLHIT", "OAE", "CANCEL", "CLOSEALL", "PARTIALS", "BE"}:
        valid = len(fields) == 1
    return None if valid else "malformed_or_unsupported_action"


def validate_bridge_capacity(bridge: Path, settings: dict, source: str) -> dict:
    """Refuse target truncation before launching MT5; no events are filtered.

    Entry target filtering is price dependent, so raw targets plus enabled runner
    expansion form a conservative bound. SPP does not expand runner targets.
    """
    match = re.search(r"^\s*#define\s+MAXTP\s+(\d+)", source, re.M)
    if not match:
        raise ValueError("Native target capacity is not declared in the frozen source.")
    capacity = int(match.group(1))
    runner = max(0, int(settings.get("runner_cele_n", 0))) if settings.get("runner_cele_krok", 0) > 0 else 0
    maximum_raw = maximum_effective = entries = spp = 0
    errors = []
    for line_number, line in enumerate(bridge.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        actions = [action for action in line.split("|")[7:] if action]
        if len(actions) > 12:
            errors.append({"line": line_number, "reason": "native_action_capacity"})
        for action in actions:
            kind, separator, payload = action.partition(":")
            error = wire_action_error(action, capacity)
            if error:
                errors.append({"line": line_number, "kind": kind, "reason": error})
                continue
            if not separator or kind not in {"ENTRY", "ENTRY2", "SPP"}:
                continue
            fields = payload.split(",")
            if kind == "ENTRY2":
                count = int(fields[12]) if len(fields) >= 13 else -1
                raw_targets = fields[13:]
                if count == 0 and raw_targets == [""]:
                    raw_targets = []
                if count < 0 or count != len(raw_targets) or any(not value for value in raw_targets):
                    errors.append({"line": line_number, "kind": kind, "reason": "malformed_target_count"})
                    continue
            elif kind == "ENTRY":
                count = sum(bool(value) for value in fields[7:])
            else:
                if settings.get("spp_keep_tp", False):
                    continue
                count = sum(bool(value) for value in fields[3:])
            expansion = runner if kind in {"ENTRY", "ENTRY2"} and count else 0
            maximum_raw = max(maximum_raw, count)
            maximum_effective = max(maximum_effective, count + expansion)
            entries += kind in {"ENTRY", "ENTRY2"}
            spp += kind == "SPP"
            if count + expansion > capacity:
                errors.append({"line": line_number, "kind": kind, "raw_targets": count,
                               "effective_upper_bound": count + expansion, "reason": "native_target_capacity"})
    return {"complete": not errors, "capacity": capacity, "entry_actions": entries, "spp_actions": spp,
            "maximum_raw_targets": maximum_raw, "maximum_effective_target_bound": maximum_effective,
            "scope": "Preflight conservative bound; price filtering cannot excuse silent truncation.", "errors": errors}


def build(defaults: dict, explicit: dict, source: str, bridge: str,
          available_inputs: set[str] | None = None) -> tuple[dict, dict]:
    settings = {**defaults, **explicit}
    old_lot_growth = not any(name.startswith("lot_growth_") for name in settings)
    if old_lot_growth:
        # An old binary cannot turn on a newly introduced execution axis.
        # A partially supplied group is deliberately not completed this way.
        settings.update(LOT_GROWTH_DEFAULTS)
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
        if name.startswith("lot_growth_") and (
                isinstance(value, bool) or not isinstance(value, (int, float))
                or not math.isfinite(value)):
            errors.append({"field": name, "reason": "finite_numeric_type_required"})
            continue
        parameters[target] = convert(value)
        states[name] = ("older_binary_feature_absent_disabled_in_ea"
                        if old_lot_growth and name.startswith("lot_growth_") else "mapped")
    for name, (target, values) in ENUMY.items():
        value = settings.get(name)
        if target not in available or not isinstance(value, str) or value not in values:
            errors.append({"field": name, "reason": "missing_or_unsupported_enum"})
            continue
        parameters[target] = values[value]
        states[name] = ("older_binary_feature_absent_disabled_in_ea"
                        if old_lot_growth and name.startswith("lot_growth_") else "mapped")

    if settings.get("lot_growth_mode") != "Off":
        domains = {"lot_growth_reference_lot": lambda v: v >= .01,
                   "lot_growth_reference_balance": lambda v: v > 0,
                   "lot_growth_basket_risk_pct": lambda v: 0 <= v <= 100}
        if settings.get("lot_growth_mode") == "Power":
            domains["lot_growth_power"] = lambda v: 0 < v <= 1
        elif settings.get("lot_growth_mode") == "ThresholdLinear":
            domains["lot_growth_rate_pct"] = lambda v: v >= 0
        elif settings.get("lot_growth_mode") == "GeometricSteps":
            domains.update({"lot_growth_capital_multiple": lambda v: v > 1,
                            "lot_growth_lot_multiple": lambda v: v >= 1})
        domains.update({name: lambda v: 0 <= v <= 2 for name in LOT_GROWTH_DEFAULTS if name.endswith("_strength")})
        for name, check in domains.items():
            value = settings.get(name)
            if isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and not check(value):
                errors.append({"field": name, "reason": "invalid_active_lot_growth_domain"})

    for name, value in settings.items():
        if name in states:
            continue
        if name == "t100":
            # XT has no autonomous T-100 policy. The nested defaults must not
            # break legacy comparisons, or be mistaken for a native mapping.
            states[name] = ("unsupported_feature_inactive"
                            if isinstance(value, dict) and value.get("enabled") is False
                            else "unmapped_requires_review")
        elif name in RUNTIME:
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
