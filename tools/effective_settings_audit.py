"""Read-only selected-preset/live merge comparison; never reads authentication.

The Rust probe calls the production mapper, account merge and routing builder.
This wrapper resolves the selected file by its internal name, like Workspace,
and checks the reviewed package recipe without starting the application.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess


class AuditError(Exception):
    pass


def read(path: Path):
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except (ValueError, OSError):
        raise AuditError("configuration_unreadable") from None


def resolve_preset(workspace: Path, name: str):
    matches = []
    for path in sorted((workspace / "presets").glob("*.json")):
        value = read(path)
        if str(value.get("name", "")).casefold() == name.casefold():
            matches.append(value)
    if not matches:
        raise AuditError("selected_preset_file_missing")
    if any(value.get("settings") != matches[0].get("settings") for value in matches[1:]):
        raise AuditError("duplicate_preset_identity_with_different_settings")
    return matches[0]


def audit(workspace: Path, probe: Path, *, preset_path: Path | None = None,
          selection_path: Path | None = None, expected_path: Path | None = None,
          expected_caps_path: Path | None = None, broker_stops: float | None = None):
    # This allowlist deliberately excludes secrets, Telegram sessions and logs.
    doc = read(workspace / "settings.json")
    chains = read(workspace / "lancuchy.json")
    channels = read(workspace / "channels.json")
    active = next((x for x in chains.get("lista", []) if x.get("nazwa") == chains.get("aktywny")), None)
    if active is None:
        raise AuditError("active_chain_missing")
    selected_name = next((v for k, v in active.get("presety", {}).items() if k.strip().casefold() == "synergy" and v), None)
    if not selected_name:
        raise AuditError("synergy_leg_missing")
    proposed = selection_path is not None
    selection = read(selection_path) if proposed else None
    expected_ingress = None
    tested_overlay_matches = None
    preset = read(preset_path) if preset_path else resolve_preset(workspace, selected_name)
    if not proposed and str(preset.get("name", "")).casefold() != selected_name.casefold():
        raise AuditError("preset_is_not_active_synergy_leg")
    raw = preset.get("settings", preset)
    expected = copy.deepcopy(read(expected_path) if expected_path else raw)
    expected = expected.get("settings", expected)
    caps = active.get("pulapy", {})
    expected_caps = read(expected_caps_path) if expected_caps_path else {}
    if proposed:
        from package_release import ACCOUNT_OVERLAY_KEYS, CHAIN_CAP_KEYS, check_public_config, validate_ingress, validate_overlay, validate_caps, tested_overlay, tested_ai_settings
        if not preset_path:
            raise AuditError("proposed_selection_requires_exact_preset_file")
        # An audit can review an UNAPPROVED proposal. It never upgrades approval
        # or calls package creation; actual packaging retains its approval gate.
        if selection.get("preset_sha256") != hashlib.sha256(preset_path.read_bytes()).hexdigest():
            raise AuditError("proposed_preset_hash_mismatch")
        if selection.get("preset_id") != preset.get("name") or preset.get("format") != "Synergy":
            raise AuditError("proposed_preset_identity_mismatch")
        if set(selection.get("chain_caps", {})) != CHAIN_CAP_KEYS or not ACCOUNT_OVERLAY_KEYS.issubset(selection.get("account_overlay", {})):
            raise AuditError("proposed_complete_overlay_and_caps_required")
        validate_overlay(selection["account_overlay"])
        validate_caps(selection["chain_caps"])
        tested_overlay_matches = tested_overlay(raw) == selection["account_overlay"]
        check_public_config(selection)
        check_public_config(preset)
        validate_ingress(selection.get('ingress'))
        expected_ingress = selection['ingress']
        doc.setdefault("settings", {}).update(selection["account_overlay"])
        doc['settings'].update(tested_ai_settings(raw))
        doc['settings']['signal_max_age_min'] = expected_ingress['live_ingress_max_age_min']
        caps = selection["chain_caps"]
        if not expected_path:
            expected.update(selection["account_overlay"])
        if not expected_caps_path:
            expected_caps = caps
    elif (workspace / 'PACKAGE_MANIFEST.json').is_file():
        from package_release import validate_ingress, tested_overlay, validate_overlay
        packaged_manifest = read(workspace / 'PACKAGE_MANIFEST.json')
        expected_ingress = packaged_manifest.get('ingress_explicit')
        actual_overlay = {key: doc.get('settings', {}).get(key) for key in tested_overlay(raw)}
        validate_overlay(actual_overlay)
        tested_overlay_matches = tested_overlay(raw) == actual_overlay
        validate_ingress(expected_ingress)
    observed = set()
    for binding in channels.get("bindings", {}).values():
        if not binding.get("monitored"):
            continue
        formats = [binding.get("format", ""), *binding.get("topics", {}).values()]
        observed.update(v.strip().casefold() for v in formats if isinstance(v, str) and v.strip())
    other_legs = [] if proposed else [k for k, v in active.get("presety", {}).items() if v and k.strip().casefold() != "synergy" and k.strip().casefold() in observed]
    request = {"settings_doc":doc, "preset":preset, "expected":expected,
               "chain_caps":caps, "expected_chain_caps":expected_caps}
    from package_release import finite_number
    age = doc.get('settings', {}).get('signal_max_age_min', 5.0)
    if not finite_number(age) or age < 0:
        raise AuditError('invalid_live_ingress_max_age')
    if broker_stops is not None:
        if not finite_number(broker_stops) or broker_stops < 0:
            raise AuditError('invalid_broker_stops_level')
        request["broker_stops_level"] = broker_stops
    try:
        run = subprocess.run([str(probe.resolve())], input=json.dumps(request), capture_output=True,
                             encoding="utf-8", check=False, timeout=120)
    except (OSError, subprocess.TimeoutExpired):
        raise AuditError("offline_probe_unavailable") from None
    if run.returncode:
        # Compiler/runtime error detail could include input paths; retain no stderr.
        raise AuditError("offline_probe_rejected_configuration")
    try:
        report = json.loads(run.stdout)
    except ValueError:
        raise AuditError("offline_probe_invalid_output") from None
    report["routing"] = {"synergy_observed": "synergy" in observed,
                         "other_observed_trading_legs":len(other_legs),
                         "selected_preset_resolved":True}
    report["proposed_package_recipe"] = proposed
    report["selection_owner_approved"] = selection.get("approved") is True if proposed else None
    report["authentication_read"] = False
    report["services_started"] = False
    ingress_matches = (age == expected_ingress['live_ingress_max_age_min']) if expected_ingress else None
    report['ingress'] = {'live_ingress_max_age_min': age,
                         'expected': expected_ingress, 'matches': ingress_matches}
    from package_release import TECHNICAL_ACCOUNT_KEYS
    # Preserve every raw difference. Only this explicit operational allowlist
    # can vary between a tested preset and the installed runtime.
    report['technical_runtime_differences'] = [d for d in report['core_differences'] if d['field'] in TECHNICAL_ACCOUNT_KEYS]
    recipe_differences = [d for d in report['core_differences'] if d['field'] not in TECHNICAL_ACCOUNT_KEYS]
    strategy_overrides = [d for d in report.get('preset_to_live_overrides', []) if d['field'] not in TECHNICAL_ACCOUNT_KEYS]
    report['recipe_matches'] = bool(not recipe_differences and not report['chain_differences']
        and not report['live_start_blockers'] and report.get('single_and_multiple_routing_equal') is True
        and 'synergy' in observed and not other_legs and ingress_matches is not False)
    # The Rust probe also checks all non-overlay strategy fields against the
    # unmodified expected preset, not merely the Python document shape.
    report['strategy_matches_tested_preset'] = bool(tested_overlay_matches is not False and not strategy_overrides)
    report['matches'] = report['recipe_matches'] and report['strategy_matches_tested_preset']
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--preset", type=Path)
    parser.add_argument("--selection", type=Path)
    parser.add_argument("--expected", type=Path)
    parser.add_argument("--expected-caps", type=Path)
    parser.add_argument("--broker-stops", type=float)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = audit(args.workspace, args.probe, preset_path=args.preset,
                       selection_path=args.selection, expected_path=args.expected,
                       expected_caps_path=args.expected_caps, broker_stops=args.broker_stops)
    except Exception as error:
        code = str(error) if isinstance(error, AuditError) else "audit_failed"
        print(json.dumps({"error":code}))
        return 2
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"matches":report["matches"],"core_differences":len(report["core_differences"]),
                      "chain_differences":len(report["chain_differences"]),
                      "live_start_blockers":report["live_start_blockers"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
