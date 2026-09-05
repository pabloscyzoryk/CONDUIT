"""Build reviewed CONDUIT packages without reading private values into reports.

No command starts Conduit, Telegram, MetaTrader, or a trading connection.
Public packages start from source examples and an explicit file allowlist.
Private packages preserve authentication from an explicitly named VPSREADY
template. They never inherit historical logs, receipts, or executable helpers.
"""
from __future__ import annotations

import argparse
import ast
import base64
import hashlib
import io
import ipaddress
import json
import math
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import uuid
import zipfile
from datetime import datetime, timezone

SCHEMA = "conduit.package.v3"
PRIVATE_FILES = {"settings.json", "channels.json", "lancuchy.json", "secrets.json", "telegram.session"}
DISALLOWED_PARTS = {".git", "node_modules", "target", "logs", "backup_memory", "exports", "__pycache__"}
DISALLOWED_NAMES = {"accounts.dat", "servers.dat", "common.ini", "terminal.ini", "koszyki.json", "kronika.json", "smtp.json"}
SECRET_KEYS = {"apihash", "apiid", "sessionstring", "userid", "username", "handle", "password", "mt5password", "mt5login", "mt5server", "mt5terminalpath", "phone", "email", "mailuser", "mailto", "mailfrom", "mailhost", "channelid", "chatid", "topicid"}
SAFE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,79}$")
CHAIN_CAP_KEYS = {"maxPozycji", "maxKoszykow", "maxLotow", "maxLotowKierunkowo", "maxRyzykoPct", "maxDdPct", "maxDdUsd", "podlogaEquityUsd", "celDniaUsd", "celDniaPct", "celDniaZamyka", "limitStratyDniaUsd", "limitStratyDniaPct", "blokujPrzeciwneKierunki", "pauzaPoStratachN", "pauzaPoStratachMin"}
# These economic/runtime fields override each leg's strategy in live mode.
# A reviewed selection must record them instead of inheriting an old VPS math
# contract. Operational preferences and authentication stay with the template.
ACCOUNT_OVERLAY_KEYS = set("close_receipt_reconcile closed_profit_net_costs restore_strategy_continuation order_volume_contract_v2 commission_per_lot swap_enabled swap_long_points swap_short_points swap_point_value swap_rollover_mult swap_rollover_weekday swap_pomijaj_weekend swap_rollover_z_serwera swap_rollover3days_mt5 runner_ksiegowanie_v2 msg_kurs_sprzed_luki slippage_pts slippage_pending_pts stops_level exec_latency_ms msg_clock_offset_ms server_tz_offset_ms stop_out_level_pct margin_call_level_pct expo_cap_pct sim_margin_check_on_fill sim_validate_pending_stops sim_margin_at_market expo_cap_ml_pct expo_cap_close expo_cap_s lot_base odlicz_kredyt credit_balance_separate kredyt_reczny konto_dzwignia".split())


OVERLAY_BOOL_KEYS = set("close_receipt_reconcile closed_profit_net_costs credit_balance_separate expo_cap_close msg_kurs_sprzed_luki odlicz_kredyt order_volume_contract_v2 restore_strategy_continuation runner_ksiegowanie_v2 sim_margin_at_market sim_margin_check_on_fill sim_validate_pending_stops swap_enabled swap_pomijaj_weekend swap_rollover_z_serwera".split())
OVERLAY_INT_KEYS = {"exec_latency_ms", "server_tz_offset_ms", "msg_clock_offset_ms", "swap_rollover3days_mt5", "swap_rollover_weekday"}
CHAIN_BOOL_KEYS = {"celDniaZamyka", "blokujPrzeciwneKierunki"}
CHAIN_INT_KEYS = {"maxPozycji", "maxKoszykow", "pauzaPoStratachN"}
AI_ACCOUNT_KEYS = {"ai_enabled", "ai_model", "ai_decision_interval_s", "ai_replaces_management"}
TECHNICAL_ACCOUNT_KEYS = set("mt5_autostart mt5_watchdog mt5_health_interval_s mt5_restart_after mt5_retry_attempts mt5_retry_delay_s mt5_terminal_path journal_enabled journal_min_level journal_text_mirror journal_retention_days journal_excursions journal_snapshots journal_buffer_cap".split())


def finite_number(value):
    try:
        return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)
    except OverflowError:
        return False


def validate_overlay(overlay):
    if not isinstance(overlay, dict) or set(overlay) != ACCOUNT_OVERLAY_KEYS:
        fail("explicit_complete_account_overlay_required")
    for key, value in overlay.items():
        if key in OVERLAY_BOOL_KEYS:
            valid = isinstance(value, bool)
        elif key == "lot_base":
            valid = isinstance(value, str) and value in {"Balance", "Equity", "MinOfBoth"}
        elif key == "msg_clock_offset_ms" and value is None:
            valid = True
        elif key in OVERLAY_INT_KEYS:
            valid = isinstance(value, int) and not isinstance(value, bool) and -(2**63) <= value < 2**63
            if key in {"swap_rollover3days_mt5", "swap_rollover_weekday"}:
                valid = valid and 0 <= value <= 0xffffffff
        else:
            valid = finite_number(value)
        if not valid:
            fail("invalid_account_overlay_value")


def validate_caps(caps):
    if not isinstance(caps, dict) or set(caps) != CHAIN_CAP_KEYS:
        fail("explicit_complete_chain_caps_required")
    for key, value in caps.items():
        valid = isinstance(value, bool) if key in CHAIN_BOOL_KEYS else finite_number(value) and value >= 0
        if key in CHAIN_INT_KEYS:
            valid = isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= 0xffffffff
        if not valid:
            fail("invalid_chain_cap_value")


# Verified against production Rust defaults. Missing values outside these four
# are not guessed. Stage checks the current source defaults before relying on them.
OVERLAY_MISSING_DEFAULTS = {"closed_profit_net_costs": False, "credit_balance_separate": False,
                          "order_volume_contract_v2": False, "restore_strategy_continuation": False}


def overlay_defaults_source(source):
    path = checked_file(source, "rust/crates/core/src/settings.rs")
    text = path.read_text(encoding="utf-8-sig")
    body = text.split("impl Default for Settings", 1)[-1].split("impl Settings", 1)[0]
    for key, expected in OVERLAY_MISSING_DEFAULTS.items():
        found = re.findall(r"(?m)^\s*" + re.escape(key) + r":\s*(true|false)\s*,", body)
        if found != ["true" if expected else "false"]:
            fail("production_overlay_default_contract_changed")
    return sha(path.read_bytes())


def tested_overlay(settings):
    values = {}
    for key in ACCOUNT_OVERLAY_KEYS:
        if key in settings:
            values[key] = settings[key]
        elif key in OVERLAY_MISSING_DEFAULTS:
            values[key] = OVERLAY_MISSING_DEFAULTS[key]
        else:
            fail("selected_preset_incomplete_tested_overlay")
    validate_overlay(values)
    return values


def require_tested_overlay(settings, overlay):
    expected = tested_overlay(settings)
    validate_overlay(overlay)
    if any(expected[key] != overlay[key] for key in ACCOUNT_OVERLAY_KEYS):
        fail("account_overlay_changes_tested_preset")


def tested_ai_settings(settings):
    if not AI_ACCOUNT_KEYS.issubset(settings):
        fail("selected_preset_incomplete_ai_contract")
    values = {key: settings[key] for key in AI_ACCOUNT_KEYS}
    if (type(values["ai_enabled"]) is not bool or type(values["ai_replaces_management"]) is not bool
            or not isinstance(values["ai_model"], str)
            or not finite_number(values["ai_decision_interval_s"]) or values["ai_decision_interval_s"] < 0):
        fail("invalid_ai_account_value")
    return values


class PackageError(Exception):
    """Only stable, non-sensitive codes may cross the command-line boundary."""


def fail(code: str):
    raise PackageError(code)


def read_json(path: Path):
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, ValueError):
        fail("json_unreadable")


def write_json(path: Path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def checked_file(root: Path, relative: str) -> Path:
    rel = Path(relative)
    if rel.is_absolute() or ".." in rel.parts or not rel.parts:
        fail("unsafe_member_path")
    root = root.resolve(strict=True)
    current = root
    for part in rel.parts:
        current = current / part
        try:
            info = current.lstat()
        except OSError:
            fail("required_payload_missing")
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
            fail("reparse_point_not_allowed")
    if not current.resolve(strict=True).is_relative_to(root) or not current.is_file():
        fail("unsafe_member_path")
    return current


def python_closure(source_root: Path) -> list[Path]:
    """Find local imports, including imports inside feature-gated functions."""
    root = source_root / "rust/crates/mt5/sidecar"
    pending = ["mt5_sidecar.py"]
    found = {}
    while pending:
        name = pending.pop()
        if name in found:
            continue
        path = checked_file(root, name)
        try:
            tree = ast.parse(path.read_text(encoding="utf-8-sig"))
        except (OSError, SyntaxError, UnicodeError):
            fail("sidecar_parse_failed")
        found[name] = path
        for node in ast.walk(tree):
            modules = [a.name for a in node.names] if isinstance(node, ast.Import) else [node.module] if isinstance(node, ast.ImportFrom) and node.module else []
            for module in modules:
                relative = module.replace(".", "/") + ".py"
                if (root / relative).is_file():
                    pending.append(relative)
    return [found[name] for name in sorted(found)]


def sensitive_values(templates: list[Path]) -> set[bytes]:
    """Private values and their derived hashes exist only in this process."""
    values = set()

    def add(value):
        if isinstance(value, bool) or value is None:
            return
        text = str(value)
        if len(text) < 6 or text in {"false", "true"}:
            return
        for encoded in (text.encode("utf-8"), text.encode("utf-16-le")):
            values.add(encoded)
        # Old private verifiers embedded a hash of the account login.
        values.add(hashlib.sha256(text.encode("utf-8")).hexdigest().encode("ascii"))

    def visit(value, all_values=False):
        if isinstance(value, dict):
            for key, item in value.items():
                normalized = re.sub(r"[^a-z0-9]", "", key.lower())
                if all_values and re.fullmatch(r"-?\d{6,}", key):
                    add(key)
                if normalized in SECRET_KEYS and not isinstance(item, (dict, list)):
                    add(item)
                visit(item, all_values)
        elif isinstance(value, list):
            for item in value:
                visit(item, all_values)
        elif all_values:
            add(value)

    for root in templates:
        for name in ("secrets.json", "settings.json", "channels.json"):
            path = checked_file(root, name)
            visit(read_json(path), name == "secrets.json")
        session = root / "telegram.session"
        if session.is_file():
            values.add(checked_file(root, "telegram.session").read_bytes())
    return {v for v in values if v}


def public_bytes(data: bytes, private_values: set[bytes]):
    if any(value in data for value in private_values):
        fail("private_value_in_public_payload")
    patterns = [rb"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----", rb"\bgh[pousr]_[A-Za-z0-9]{30,}", rb"\bgithub_pat_[A-Za-z0-9_]{40,}", rb"\b\d{7,13}:[A-Za-z0-9_-]{30,}\b"]
    if any(re.search(pattern, data) for pattern in patterns):
        fail("credential_pattern_in_public_payload")


def check_public_config(value):
    if isinstance(value, dict):
        for key, item in value.items():
            normalized = re.sub(r"[^a-z0-9]", "", key.lower())
            if normalized in SECRET_KEYS and item not in (None, "", 0, False, [], {}):
                fail("nonempty_identity_in_public_configuration")
            check_public_config(item)
    elif isinstance(value, list):
        for item in value:
            check_public_config(item)


def source_inspection(template: Path) -> dict:
    """Return presence and semantic equality, never identities or hashes."""
    doc = read_json(checked_file(template, "settings.json"))
    settings = doc.get("settings", {})
    secret = read_json(checked_file(template, "secrets.json"))
    tg = secret.get("telegram", {})
    chains = read_json(checked_file(template, "lancuchy.json"))
    active = next((x for x in chains.get("lista", []) if x.get("nazwa") == chains.get("aktywny")), None)
    if not active:
        fail("active_template_chain_missing")
    session_file = template / "telegram.session"
    session_match = None
    if tg.get("sessionString"):
        try:
            # String-only sessions are supported, but must be structurally valid
            # even when no on-disk session exists for comparison.
            encoded = tg["sessionString"]
            if not isinstance(encoded, str):
                raise ValueError("invalid_session_type")
            def unique_object(pairs):
                result = {}
                for key, value in pairs:
                    if key in result:
                        raise ValueError('duplicate_session_field')
                    result[key] = value
                return result
            b = json.loads(base64.b64decode(encoded.strip(), validate=True).decode('utf-8'), object_pairs_hook=unique_object)
            def auth(record):
                def integer(value, bits):
                    return type(value) is int and -(2**(bits-1)) <= value < 2**(bits-1)
                def socket(value, version):
                    if not isinstance(value, str):
                        raise ValueError('invalid_session_socket')
                    host, port = value.rsplit(':', 1)
                    if version == 6:
                        if not (host.startswith('[') and host.endswith(']')):
                            raise ValueError('invalid_session_socket')
                        host = host[1:-1]
                    if ipaddress.ip_address(host).version != version or not port.isdecimal() or not 0 <= int(port) <= 65535:
                        raise ValueError('invalid_session_socket')
                if not isinstance(record, dict) or type(record.get("version")) is not int or record["version"] != 1:
                    raise ValueError("invalid_session_version")
                home = record.get("home_dc")
                options = record.get("dc_options")
                if not integer(home, 32) or not isinstance(options, list):
                    raise ValueError("invalid_session_shape")
                keys = {}
                for item in options:
                    if not isinstance(item, dict) or not integer(item.get("id"), 32) or item["id"] in keys:
                        raise ValueError("invalid_session_dc")
                    socket(item.get('ipv4'), 4)
                    socket(item.get('ipv6'), 6)
                    key = item.get('auth_key')
                    # grammers-session 0.10 serde uses Option<Hex<[u8;256]>>.
                    if key is not None and (not isinstance(key, str) or re.fullmatch(r'[0-9a-fA-F]{512}', key) is None):
                        raise ValueError('invalid_session_auth_key')
                    keys[item["id"]] = key.lower() if key is not None else None
                key = keys.get(home)
                if not key:
                    raise ValueError("empty_session_home_auth")
                peers, updates = record.get('peers'), record.get('updates')
                if not isinstance(peers, list) or not isinstance(updates, dict):
                    raise ValueError('invalid_session_state')
                for peer in peers:
                    if not isinstance(peer, dict) or len(peer) != 1:
                        raise ValueError('invalid_session_peer')
                    kind, value = next(iter(peer.items()))
                    if kind not in {'User', 'Chat', 'Channel'} or not isinstance(value, dict) or not integer(value.get('id'), 64):
                        raise ValueError('invalid_session_peer')
                    if kind != 'Chat' and value.get('auth') is not None and not integer(value['auth'], 64):
                        raise ValueError('invalid_session_peer_auth')
                    if kind == 'User' and any(value.get(k) is not None and type(value[k]) is not bool for k in ('bot', 'is_self')):
                        raise ValueError('invalid_session_peer_flags')
                    if kind == 'Channel' and value.get('kind') not in {None, 'Broadcast', 'Megagroup', 'Gigagroup'}:
                        raise ValueError('invalid_session_channel_kind')
                if any(not integer(updates.get(k), 32) for k in ('pts', 'qts', 'date', 'seq')) or not isinstance(updates.get('channels'), list):
                    raise ValueError('invalid_session_updates')
                for channel in updates['channels']:
                    if not isinstance(channel, dict) or not integer(channel.get('id'), 64) or not integer(channel.get('pts'), 32):
                        raise ValueError('invalid_session_channel_state')
                return home, keys
            ab = auth(b)
            if session_file.is_file():
                a = json.loads(checked_file(template, 'telegram.session').read_text(encoding='utf-8-sig'), object_pairs_hook=unique_object)
                session_match = auth(a) == ab
            else:
                session_match = True
        except (ValueError, KeyError, TypeError):
            session_match = False
    return {
        "mode": doc.get("mode"),
        "follow_terminal": bool(settings.get("mt5_follow_terminal_account")),
        "fixed_login_present": bool(settings.get("mt5_login")),
        "server_present": bool(settings.get("mt5_server")),
        "password_present": bool(secret.get("mt5", {}).get("password") or settings.get("mt5_password")),
        "terminal_path_present": bool(settings.get("mt5_terminal_path")),
        "telegram_api_present": bool(tg.get("apiId") and tg.get("apiHash")),
        "telegram_string_present": bool(tg.get("sessionString")),
        "telegram_file_present": session_file.is_file(),
        "telegram_authorization_matches": session_match,
        "active_formats": sorted(k for k, v in active.get("presety", {}).items() if v),
        "chain_caps": active.get("pulapy", {}),
        "live_authentication_verified": False,
    }


def plan(source: Path, template: Path) -> dict:
    inspection = source_inspection(template)
    helpers = python_closure(source)
    return {"schema": SCHEMA, "status": "awaiting_selected_preset_and_final_build", "template": inspection,
            "runtime_payload": ["conduit.exe", "postep.exe", "START_CONDUIT.vbs", "runtime/"] + [p.name for p in helpers] + ["verify_package.py"],
            "private_preserved": sorted(PRIVATE_FILES),
            "excluded": ["logs", "exports", "backup_memory", "old runtime state", "old verifiers", "terminal caches", "private credential hashes"],
            "public_configuration": "source examples, manual mode, empty channel bindings, no authentication files",
            "host_requirements": ["Windows x64", "MetaTrader 5 terminal", "WebView2 for the native window; browser launcher also included"],
            "python_runtime": "Pinned, hash-verified embedded CPython, MetaTrader5 and NumPy included; no global Python required",
            "follow_contract": "Exactly one running signed-in terminal, or an explicit terminal path; no automatic account switching.",
            "source_export": "Explicit immutable Git commit; no untracked workspace content",
            "no_services_started": True}


def selected_preset(preset: Path, selection: Path):
    chosen = read_json(selection)
    raw = preset.read_bytes()
    doc = read_json(preset)
    if chosen.get("approved") is not True or chosen.get("preset_sha256") != sha(raw):
        fail("selected_preset_not_approved_or_changed")
    name = chosen.get("preset_id", "")
    if not isinstance(name, str) or not SAFE_ID.fullmatch(name) or doc.get("name", doc.get("nazwa")) != name:
        fail("selected_preset_identity_mismatch")
    if doc.get("format") != "Synergy" or not isinstance(doc.get("settings"), dict):
        fail("selected_preset_must_be_synergy")
    if not isinstance(chosen.get("chain_caps"), dict) or set(chosen["chain_caps"]) != CHAIN_CAP_KEYS:
        fail("explicit_complete_chain_caps_required")
    if not isinstance(chosen.get("account_overlay"), dict) or not ACCOUNT_OVERLAY_KEYS.issubset(chosen["account_overlay"]):
        fail("explicit_complete_account_overlay_required")
    if any(re.sub(r"[^a-z0-9]", "", key.lower()) in SECRET_KEYS for key in chosen["account_overlay"]):
        fail("account_overlay_cannot_change_identity")
    validate_overlay(chosen["account_overlay"])
    validate_caps(chosen["chain_caps"])
    require_tested_overlay(doc["settings"], chosen["account_overlay"])
    validate_ingress(chosen.get("ingress"))
    check_public_config(chosen)
    check_public_config(doc)
    return chosen, doc


def validate_ingress(ingress):
    if (not isinstance(ingress, dict)
            or set(ingress) != {"live_telegram_ingress", "live_ingress_max_age_min"}
            or ingress.get("live_telegram_ingress") is not True):
        fail("explicit_live_ingress_contract_required")
    age = ingress["live_ingress_max_age_min"]
    if not finite_number(age) or age < 0:
        fail("invalid_live_ingress_max_age")


def production_preset_metadata(document: dict, selected_id: str) -> dict:
    # Candidate documents may inherit historical performance copy. Approval
    # selects settings and identity, never silently endorses that old prose.
    return {"name": selected_id, "nazwa": selected_id, "format": "Synergy",
            "tagline": "Synergy", "description": "", "opis": "",
            "settings": document["settings"]}


def settings_sha256(settings: dict) -> str:
    return sha(json.dumps(settings, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode("utf-8"))


def safe_destination(destination: Path, kind: str) -> Path:
    destination = destination.absolute()
    # Resolve all existing parents; directory junctions cannot redirect a public
    # target into a private runtime or place secrets outside VPSREADY.
    parent = destination.parent.resolve(strict=True)
    result = parent / destination.name
    if result.exists() or result.is_symlink():
        fail("destination_already_exists")
    private = any(part.upper().startswith("VPSREADY") for part in result.parts)
    if kind == "private" and not private:
        fail("private_destination_must_be_vpsready")
    if kind != "private" and private:
        fail("public_destination_cannot_be_private")
    return result


def package_readme(kind: str) -> bytes:
    pl = ("CONDUIT — SZYBKI START\n\n"
          "1. Rozpakuj cały folder. Nie przenoś pojedynczego pliku EXE.\n"
          "2. Uruchom START_CONDUIT.vbs. Jeśli okno się nie otwiera, użyj START_BROWSER.vbs.\n"
          "3. Python i moduły MT5 są dołączone w runtime/; globalny Python nie jest wymagany.\n"
          "4. Terminal MetaTrader 5 musi być zainstalowany i zalogowany na wybrane konto.\n"
          "5. Instrumenty: PUPrime — XAUUSD.s; Vantage — XAUUSD. Sprawdź instrument w panelu.\n")
    en = ("CONDUIT — QUICK START\n\n"
          "1. Extract the entire folder. Do not move the EXE on its own.\n"
          "2. Run START_CONDUIT.vbs. If the window does not open, use START_BROWSER.vbs.\n"
          "3. Python and the MT5 modules are included in runtime/; no global Python is required.\n"
          "4. MetaTrader 5 must be installed and signed in to the intended account.\n"
          "5. Symbols: PUPrime — XAUUSD.s; Vantage — XAUUSD. Check the symbol in the panel.\n")
    if kind == 'private':
        pl += ("\nVPS: zachowano konfigurację prywatną. Przy FOLLOW_TERMINAL pozostaw uruchomiony\n"
               "dokładnie jeden zalogowany terminal albo jawnie wybierz jego ścieżkę w panelu.\n"
               "Przed startem sprawdź wybrane konto. Uruchomienie może wznowić tryb AUTO.\n"
               "Ten folder jest prywatny: nie udostępniaj go innym osobom.\n")
        en += ("\nVPS: the private configuration is preserved. With FOLLOW_TERMINAL, keep exactly\n"
               "one signed-in terminal running, or explicitly select its path in the panel.\n"
               "Check the selected account before starting. Startup may resume AUTO mode.\n"
               "This folder is private: do not share it with other people.\n")
        return (pl + '\n' + en).encode('utf-8')
    en += "\nPUBLIC: starts in MANUAL mode. Configure Telegram and the intended terminal\nbefore enabling AUTO. This package contains no login data or Telegram session.\n"
    pl += "\nPUBLIC: startuje w trybie MANUAL. Skonfiguruj Telegram i wybrany terminal\nprzed włączeniem AUTO. Paczka nie zawiera danych logowania ani sesji Telegrama.\n"
    return (en + '\n' + pl).encode('utf-8')


def stage(source: Path, template: Path, executable: Path, preset: Path, selection: Path,
          destination: Path, kind: str, runtime: Path, monitor: Path) -> dict:
    if kind not in {"private", "public"}:
        fail("invalid_package_kind")
    destination = safe_destination(destination, kind)
    defaults_source_sha = overlay_defaults_source(source)
    chosen, preset_doc = selected_preset(preset, selection)
    name = chosen["preset_id"]
    private_values = sensitive_values([template])
    inspection = source_inspection(template)
    if kind == "private" and (not inspection["telegram_api_present"] or not inspection["telegram_string_present"]
                              or inspection["telegram_authorization_matches"] is False):
        fail("private_telegram_material_incomplete_or_mismatched")
    helpers = python_closure(source)
    from portable_runtime import verify as verify_runtime
    verify_runtime(runtime)
    executable = checked_file(executable.parent, executable.name)
    exe_bytes = executable.read_bytes()
    if not exe_bytes.startswith(b"MZ"):
        fail("runtime_not_windows_executable")
    public_bytes(exe_bytes, private_values)
    monitor_bytes = checked_file(monitor.parent, monitor.name).read_bytes()
    if not monitor_bytes.startswith(b"MZ"):
        fail("monitor_not_windows_executable")
    public_bytes(monitor_bytes, private_values)

    settings = read_json(checked_file(template, "settings.json")) if kind == "private" else read_json(checked_file(source, "config/examples/settings.example.json"))
    chains = read_json(checked_file(template, "lancuchy.json")) if kind == "private" else read_json(checked_file(source, "config/examples/lancuchy.example.json"))
    active = next((x for x in chains.get("lista", []) if x.get("nazwa") == chains.get("aktywny")), None)
    if active is None:
        fail("active_chain_missing")
    settings["presetId"] = name
    settings.setdefault("settings", {}).update(chosen["account_overlay"])
    # These account-merge fields can replace strategy management. They belong
    # to the tested preset, never an unrelated private/public template.
    settings["settings"].update(tested_ai_settings(preset_doc["settings"]))
    settings["settings"]["signal_max_age_min"] = chosen["ingress"]["live_ingress_max_age_min"]
    # The app resolves this packaged path relative to its executable, not CWD.
    settings["settings"]["mt5_python"] = "runtime/python.exe"
    active["pulapy"] = chosen["chain_caps"]
    active["presety"] = {key: name if key == "Synergy" else "" for key in set(active.get("presety", {})) | {"Synergy"}}
    if kind == "public":
        settings["mode"] = "MANUAL"
        settings["language"] = "en"
        check_public_config(settings)
        check_public_config(chains)

    staging = destination.with_name(destination.name + ".staging-" + uuid.uuid4().hex)
    staging.mkdir()
    # A failed staging directory remains marked incomplete, never masquerading
    # as a final package. No existing destination is deleted or overwritten.
    (staging / "INCOMPLETE").write_text("Package staging has not passed verification.\n", encoding="utf-8")
    public_members = {}

    def put(relative, data, public=True):
        if public:
            public_bytes(data, private_values)
            public_members[relative] = {"sha256": sha(data), "size": len(data)}
        path = staging / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    put("conduit.exe", exe_bytes)
    put("postep.exe", monitor_bytes)
    for item in sorted(runtime.rglob("*")):
        if item.is_file():
            relative = item.relative_to(runtime).as_posix()
            put("runtime/" + relative, checked_file(runtime, relative).read_bytes())
    # VBS uses its own directory and a hidden process. Launchers are packaged,
    # never executed by this builder or verifier.
    launcher = ('Option Explicit\r\nDim shell, fso, folder, command\r\n'
                'Set shell = CreateObject("WScript.Shell")\r\n'
                'Set fso = CreateObject("Scripting.FileSystemObject")\r\n'
                'folder = fso.GetParentFolderName(WScript.ScriptFullName)\r\n'
                'shell.CurrentDirectory = folder\r\n'
                'command = Chr(34) & folder & "\\conduit.exe" & Chr(34)\r\n')
    put("START_CONDUIT.vbs", (launcher + 'shell.Run command, 0, False\r\n').encode("ascii"))
    put("START_BROWSER.vbs", (launcher + 'shell.Run command & " --headless --open", 0, False\r\n').encode("ascii"))
    put('README.txt', package_readme(kind))
    # python_closure returns checked, absolute files. Compare like domains even
    # when the CLI source root is relative; file checks remain in the closure.
    sidecar_root = (source / "rust/crates/mt5/sidecar").resolve(strict=True)
    for helper in helpers:
        put(helper.relative_to(sidecar_root).as_posix(), helper.read_bytes())
    put("verify_package.py", Path(__file__).read_bytes())
    for item in sorted((source / "config/presets").glob("*.json")):
        item = checked_file(source / "config/presets", item.name)
        check_public_config(read_json(item))
        put("presets/" + item.name, item.read_bytes())
    packaged_preset = production_preset_metadata(preset_doc, name)
    packaged_preset_bytes = (json.dumps(packaged_preset, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    put("presets/" + name + ".json", packaged_preset_bytes)
    # BIEZACY is the identical reviewed strategy with normalized presentation.
    put("presets/BIEZACY.json", packaged_preset_bytes)
    def put_json(relative, value, public=True):
        put(relative, (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8"), public)
    put_json("settings.json", settings, kind == "public")
    put_json("lancuchy.json", chains, kind == "public")
    if kind == "private":
        for private_name in ("secrets.json", "telegram.session", "channels.json"):
            if private_name == "telegram.session" and not (template / private_name).is_file():
                continue
            put(private_name, checked_file(template, private_name).read_bytes(), False)
    else:
        put_json("channels.json", {"bindings": {}})
    stamp = {"nazwa": destination.name, "zbudowano": datetime.now(timezone.utc).isoformat(),
             "aktywny_lancuch": chains["aktywny"], "presety_nog": {"Synergy": name}}
    put_json("PACZKA.json", stamp)
    manifest = {"schema": SCHEMA, "kind": kind, "preset_id": name,
                "selected_preset_sha256": chosen["preset_sha256"],
                "packaged_preset_sha256": sha(packaged_preset_bytes),
                "selected_settings_sha256": settings_sha256(preset_doc["settings"]),
                "preset_metadata_policy": "selected_identity_only_v1", "public_files": public_members,
                "private_files": sorted(f for f in PRIVATE_FILES if (staging / f).is_file()) if kind == "private" else [],
                "chain_caps": active.get("pulapy", {}), "account_overlay_explicit": chosen.get("account_overlay", {}),
                "ingress_explicit": chosen["ingress"],
                "tested_overlay_policy": "all_36_exact_v1",
                "overlay_missing_defaults": OVERLAY_MISSING_DEFAULTS,
                "overlay_defaults_source_sha256": defaults_source_sha,
                "runtime_authentication_verified": False, "execution_started": False}
    check_public_config(manifest)
    public_bytes(json.dumps(manifest, ensure_ascii=False).encode("utf-8"), private_values)
    write_json(staging / "PACKAGE_MANIFEST.json", manifest)
    result = verify(staging, template, allow_incomplete=True)
    (staging / "INCOMPLETE").unlink()
    staging.rename(destination)
    return {**result, "status": "staged_and_verified_offline", "no_services_started": True}


def verify(package: Path, template: Path | None = None, allow_incomplete=False) -> dict:
    manifest = read_json(checked_file(package, "PACKAGE_MANIFEST.json"))
    if manifest.get("schema") != SCHEMA or manifest.get("kind") not in {"public", "private"}:
        fail("package_manifest_invalid")
    kind = manifest["kind"]
    if (package / "INCOMPLETE").exists() and not allow_incomplete:
        fail("package_incomplete")
    if kind == "private" and template is None:
        fail("private_verification_requires_explicit_template")
    values = sensitive_values([template]) if template else set()
    check_public_config(manifest)
    public_bytes(checked_file(package, "PACKAGE_MANIFEST.json").read_bytes(), values)
    expected = set(manifest["public_files"]) | set(manifest.get("private_files", [])) | {"PACKAGE_MANIFEST.json"}
    if allow_incomplete:
        expected.add("INCOMPLETE")
    for path in package.rglob("*"):
        if not path.is_file():
            if path.is_symlink() or getattr(path.lstat(), "st_file_attributes", 0) & 0x400:
                fail("reparse_point_not_allowed")
            continue
        relative = path.relative_to(package).as_posix()
        checked_file(package, relative)
        if relative not in expected:
            fail("unexpected_package_file")
        if kind == "public" and (path.name in DISALLOWED_NAMES or path.name.startswith("secrets.") or ".session" in path.name or DISALLOWED_PARTS.intersection(path.relative_to(package).parts)):
            fail("private_artifact_in_public_package")
    for name, evidence in manifest["public_files"].items():
        data = checked_file(package, name).read_bytes()
        if sha(data) != evidence.get("sha256") or len(data) != evidence.get("size"):
            fail("runtime_or_public_payload_changed")
        public_bytes(data, values)
    settings = read_json(checked_file(package, "settings.json"))
    chains = read_json(checked_file(package, "lancuchy.json"))
    stamp = read_json(checked_file(package, "PACZKA.json"))
    active = next((x for x in chains.get("lista", []) if x.get("nazwa") == chains.get("aktywny")), {})
    active_presets = {k: v for k, v in active.get("presety", {}).items() if v}
    expected_presets = {"Synergy": manifest["preset_id"]}
    if settings.get("presetId") != manifest["preset_id"] or active_presets != expected_presets or stamp.get("presety_nog") != expected_presets or stamp.get("aktywny_lancuch") != chains.get("aktywny"):
        fail("preset_chain_stamp_mismatch")
    if sha(checked_file(package, "presets/" + manifest["preset_id"] + ".json").read_bytes()) != manifest.get("packaged_preset_sha256"):
        fail("selected_preset_changed")
    if sha(checked_file(package, "presets/BIEZACY.json").read_bytes()) != manifest.get("packaged_preset_sha256"):
        fail("current_preset_snapshot_changed")
    packaged_preset = read_json(checked_file(package, "presets/" + manifest["preset_id"] + ".json"))
    if not isinstance(packaged_preset.get("settings"), dict) or settings_sha256(packaged_preset["settings"]) != manifest.get("selected_settings_sha256"):
        fail("selected_preset_settings_changed")
    if manifest.get("preset_metadata_policy") != "selected_identity_only_v1" or packaged_preset != production_preset_metadata(packaged_preset, manifest["preset_id"]):
        fail("selected_preset_metadata_not_normalized")
    if active.get("pulapy") != manifest.get("chain_caps") or set(active.get("pulapy", {})) != CHAIN_CAP_KEYS:
        fail("chain_caps_differ_from_reviewed_selection")
    overlay = manifest.get("account_overlay_explicit", {})
    validate_overlay(overlay)
    if (manifest.get("tested_overlay_policy") != "all_36_exact_v1"
            or manifest.get("overlay_missing_defaults") != OVERLAY_MISSING_DEFAULTS
            or any(type(v) is not bool for v in manifest.get("overlay_missing_defaults", {}).values())
            or not re.fullmatch(r"[0-9a-f]{64}", str(manifest.get("overlay_defaults_source_sha256", "")))):
        fail("tested_overlay_contract_missing")
    require_tested_overlay(packaged_preset["settings"], overlay)
    validate_caps(active.get("pulapy"))
    validate_caps(manifest.get("chain_caps"))
    actual_overlay = {key: settings.get("settings", {}).get(key) for key in ACCOUNT_OVERLAY_KEYS}
    validate_overlay(actual_overlay)
    if any(actual_overlay[k] != v for k, v in overlay.items()):
        fail("account_overlay_differs_from_reviewed_selection")
    if tested_ai_settings(settings.get("settings", {})) != tested_ai_settings(packaged_preset["settings"]):
        fail("ai_account_changes_tested_preset")
    ingress = manifest.get("ingress_explicit")
    validate_ingress(ingress)
    actual_age = settings.get("settings", {}).get("signal_max_age_min")
    if not finite_number(actual_age) or actual_age < 0 or actual_age != ingress["live_ingress_max_age_min"]:
        fail("live_ingress_differs_from_reviewed_selection")
    if settings.get("settings", {}).get("mt5_python") != "runtime/python.exe":
        fail("portable_interpreter_not_selected")
    if kind == "public":
        check_public_config(settings)
        check_public_config(chains)
        if settings.get("mode") != "MANUAL" or read_json(checked_file(package, "channels.json")) != {"bindings": {}}:
            fail("public_package_not_unconfigured_manual")
    else:
        inspection = source_inspection(template)
        if (not inspection['telegram_api_present'] or not inspection['telegram_string_present']
                or inspection['telegram_authorization_matches'] is not True):
            fail('private_telegram_material_incomplete_or_mismatched')
        for name in ("secrets.json", "telegram.session", "channels.json"):
            a, b = package / name, template / name
            if a.exists() != b.exists() or a.exists() and checked_file(package, name).read_bytes() != checked_file(template, name).read_bytes():
                fail("private_material_differs_from_template")
        original = read_json(checked_file(template, "settings.json")).get("settings", {})
        actual = settings.get("settings", {})
        for key in ("mt5_login", "mt5_server", "mt5_password", "mt5_terminal_path", "mt5_follow_terminal_account", "mt5_allow_real_account"):
            if original.get(key) != actual.get(key):
                fail("private_account_selection_changed")
    return {"schema": SCHEMA, "ok": True, "kind": kind, "public_files_verified": len(manifest["public_files"]),
            "private_material_compared_in_memory": kind == "private", "credential_values_or_hashes_reported": False,
            "live_authentication_verified": False, "preset_and_chain_match": True,
            "recipe_matches": True, "strategy_matches_tested_preset": True}


def export_source(repo: Path, revision: str, destination: Path, template: Path) -> dict:
    destination = safe_destination(destination, "public")
    try:
        commit = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "--verify", revision + "^{commit}"], stderr=subprocess.PIPE).decode().strip()
        archive = subprocess.check_output(["git", "-C", str(repo), "archive", "--format=zip", commit], stderr=subprocess.PIPE)
    except subprocess.CalledProcessError:
        fail("source_revision_unavailable")
    values = sensitive_values([template])
    members = []
    with zipfile.ZipFile(io.BytesIO(archive)) as zipped:
        for info in zipped.infolist():
            if info.is_dir():
                continue
            name = Path(info.filename)
            mode = info.external_attr >> 16
            if name.is_absolute() or ".." in name.parts or stat.S_ISLNK(mode):
                fail("unsafe_source_archive_member")
            if name.parts[0] not in {"CONDUIT", "tools", "docs", "report", "README.md", ".gitignore", ".gitattributes", ".github", "LICENSE", "LICENSE.md"} or DISALLOWED_PARTS.intersection(name.parts) or any(p.upper().startswith("VPSREADY") for p in name.parts):
                fail("source_member_outside_allowlist")
            # Exact source paths only. The Rust module is code, not a secret
            # document; both exceptions still pass the public payload scan.
            secret_name_source_exceptions = {"CONDUIT/config/examples/secrets.example.json", "CONDUIT/rust/crates/server/src/secrets.rs"}
            if name.name in DISALLOWED_NAMES or (name.name.startswith("secrets.") and name.as_posix() not in secret_name_source_exceptions) or ".session" in name.name or name.suffix.lower() in {".exe", ".dll", ".pdb", ".zip", ".bin", ".pyc"}:
                fail("private_or_build_artifact_in_source")
            data = zipped.read(info)
            if name.as_posix() == "CONDUIT/config/examples/secrets.example.json":
                check_public_config(json.loads(data))
            public_bytes(data, values)
            members.append((name, data))
    destination.mkdir()
    for name, data in members:
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    write_json(destination / "SOURCE_EXPORT.json", {"schema": SCHEMA, "commit": commit, "files": len(members), "source": "immutable Git commit", "private_values_or_hashes_reported": False})
    return {"ok": True, "kind": "public_source", "files": len(members), "commit": commit}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("plan")
    p.add_argument("--source-root", type=Path, required=True)
    p.add_argument("--private-template", type=Path, required=True)
    p = sub.add_parser("stage")
    for name in ("source-root", "private-template", "executable", "preset", "selection", "destination", "runtime", "monitor"):
        p.add_argument("--" + name, type=Path, required=True)
    p.add_argument("--kind", choices=("public", "private"), required=True)
    p = sub.add_parser("verify")
    p.add_argument("--package", type=Path, required=True)
    p.add_argument("--private-template", type=Path)
    p = sub.add_parser("export-source")
    p.add_argument("--repo", type=Path, required=True)
    p.add_argument("--revision", required=True)
    p.add_argument("--destination", type=Path, required=True)
    p.add_argument("--private-template", type=Path, required=True)
    for p in sub.choices.values():
        p.add_argument("--report", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "plan":
            result = plan(args.source_root, args.private_template)
        elif args.command == "stage":
            result = stage(args.source_root, args.private_template, args.executable, args.preset, args.selection, args.destination, args.kind, args.runtime, args.monitor)
        elif args.command == "verify":
            result = verify(args.package, args.private_template)
        else:
            result = export_source(args.repo, args.revision, args.destination, args.private_template)
    except PackageError as error:
        result = {"ok": False, "code": str(error), "private_values_or_hashes_reported": False}
    except Exception:
        # No traceback from a private parser/path/OS error may reveal contents.
        result = {"ok": False, "code": "packaging_io_or_format_error", "private_values_or_hashes_reported": False}
    if args.report:
        write_json(args.report, result)
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result.get("ok", args.command == "plan") else 1


if __name__ == "__main__":
    raise SystemExit(main())
