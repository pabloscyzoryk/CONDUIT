"""Compile and run tester-only experts in an explicitly provisioned offline sandbox.

The sandbox must contain a private cached broker context and a marker file.
No credentials or terminal location are distributed with this helper.
"""
from __future__ import annotations

import argparse
import configparser
from contextlib import contextmanager
from datetime import date
import hashlib
import json
import os
import re
import shutil
import subprocess
import time
from pathlib import Path

EXPERTS = {"CONDUIT_XT", "CONDUIT_TICK_DUMP", "CONDUIT_SESSION_PROBE"}


def read_text(path: Path) -> str:
    raw = path.read_bytes()
    if raw.startswith((b"\xff\xfe", b"\xfe\xff")):
        return raw.decode("utf-16")
    return raw.decode("utf-8-sig", errors="replace")


def hidden_options() -> dict:
    info = subprocess.STARTUPINFO()
    info.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = 0
    return {"startupinfo": info}


def fingerprint(path: Path) -> dict:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return {"sha256": digest.hexdigest(), "bytes": path.stat().st_size}


def validate_sandbox(sandbox: Path) -> dict:
    sandbox = sandbox.resolve(strict=True)
    marker = sandbox / "conduit_tester_sandbox.json"
    data = json.loads(marker.read_text(encoding="utf-8"))
    if data.get("purpose") != "offline_strategy_tester" or data.get("live_trading") is not False:
        raise ValueError("Sandbox marker must explicitly identify an offline tester with live trading disabled.")
    if not (sandbox / "portable.txt").is_file():
        raise ValueError("Missing portable sandbox marker.")
    context = configparser.ConfigParser(interpolation=None, strict=False)
    context.read_string(read_text(sandbox / "Config" / "common.ini"))
    common = context["Common"]
    if common.get("ProxyEnable") != "1" or common.get("ProxyAddress") != "127.0.0.1:1":
        raise ValueError("Offline sandbox proxy is not configured.")
    if context.get("Experts", "Enabled", fallback="1") != "0":
        raise ValueError("Experts must remain disabled outside the Strategy Tester.")
    return {"login": common.get("Login", "0"), "server": common.get("Server", "")}


@contextmanager
def sandbox_lock(sandbox: Path):
    path = sandbox / "conduit_tester.lock"
    # A second launcher must not activate or reconfigure an already running
    # portable terminal. The lock is scoped to this sandbox only.
    with path.open("x", encoding="ascii") as lock:
        lock.write(str(os.getpid()))
    try:
        yield
    finally:
        path.unlink(missing_ok=True)


def compile_expert(sandbox: Path, expert: str, output: Path, source_bytes: bytes | None = None) -> dict:
    if expert not in EXPERTS:
        raise ValueError("Unsupported expert.")
    source = Path(__file__).resolve().parents[1] / "mql5" / (expert + ".mq5")
    frozen_source = source.read_bytes() if source_bytes is None else source_bytes
    if "MQLInfoInteger(MQL_TESTER)" not in frozen_source.decode("utf-8-sig"):
        raise ValueError("Expert lacks its tester-only guard.")
    destination = sandbox / "MQL5" / "Experts" / source.name
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(frozen_source)
    (output / source.name).write_bytes(frozen_source)
    log = output / ("compile_" + expert + ".log")
    if log.exists():
        log.unlink()
    proc = subprocess.run([str(sandbox / "MetaEditor64.exe"), "/compile:" + str(destination),
                           "/inc:" + str(sandbox / "MQL5"), "/log:" + str(log)],
                          capture_output=True, timeout=120, **hidden_options())
    match = re.search(r"Result:\s*(\d+)\s*errors?,\s*(\d+)\s*warnings?", read_text(log))
    if not match or int(match[1]):
        raise RuntimeError("Expert compilation failed; inspect the private compilation log.")
    binary = destination.with_suffix(".ex5")
    if not binary.exists() or binary.stat().st_mtime_ns < destination.stat().st_mtime_ns:
        raise RuntimeError("Compiled expert is missing or predates its source.")
    shutil.copy2(binary, output / binary.name)
    return {"expert": expert, "errors": int(match[1]), "warnings": int(match[2]),
            "metaeditor_exit_code": proc.returncode, "source": fingerprint(output / source.name), "binary": fingerprint(output / binary.name)}


def run_test(sandbox: Path, output: Path, expert: str, symbol: str, start: str, end: str,
             deposit: float, leverage: int, parameters: dict, timeout: int) -> dict:
    context = validate_sandbox(sandbox)
    if deposit <= 0 or leverage <= 0:
        raise ValueError("Deposit and leverage must be positive.")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", symbol):
        raise ValueError("Invalid symbol name.")
    if date.fromisoformat(start) >= date.fromisoformat(end):
        raise ValueError("Tester window must be nonempty with an exclusive end date.")
    for key, value in parameters.items():
        if not re.fullmatch(r"In_[A-Za-z0-9_]+", key) or any(c in str(value) for c in ["\r", "\n"]):
            raise ValueError("Invalid tester parameter name or multiline value.")
    # A unique private run directory and set filename prevent stale reports and collisions.
    run_id = expert + "_" + str(time.time_ns())
    set_path = sandbox / "MQL5" / "Profiles" / "Tester" / (run_id + ".set")
    set_path.parent.mkdir(parents=True, exist_ok=True)
    lines = [f"{k}={str(v).lower() if isinstance(v, bool) else v}" for k, v in parameters.items()]
    set_path.write_text("\n".join(lines) + "\n", encoding="utf-16")
    report_name = "report_" + run_id
    configuration = f"""[Common]
Login={context['login']}
Server={context['server']}
ProxyEnable=1
ProxyType=0
ProxyAddress=127.0.0.1:1
NewsEnable=0
CertInstall=0

[Experts]
Enabled=0
AllowLiveTrading=0
AllowDllImport=0

[Tester]
Expert={expert}
ExpertParameters={set_path.name}
Symbol={symbol}
Period=M1
Model=4
ExecutionMode=0
Optimization=0
FromDate={start.replace('-', '.')}
ToDate={end.replace('-', '.')}
ForwardMode=0
Deposit={deposit:.2f}
Currency=USD
Leverage=1:{leverage}
Report={report_name}
ReplaceReport=0
ShutdownTerminal=1
Visual=0
UseLocal=1
UseRemote=0
UseCloud=0
"""
    # Configuration contains private cached account identity, so keep it inside the private sandbox.
    ini = sandbox / (run_id + ".ini")
    ini.write_text(configuration, encoding="utf-8")
    offsets = {str(p): p.stat().st_size for p in (sandbox / "Tester").glob("**/logs/*.log")}
    started = time.time()
    result = subprocess.run([str(sandbox / "terminal64.exe"), "/portable", "/config:" + str(ini)],
                            capture_output=True, timeout=timeout, **hidden_options())
    report = sandbox / (report_name + ".htm")
    fresh_report = report.exists() and report.stat().st_mtime >= started
    if fresh_report:
        shutil.copy2(report, output / "native_report.htm")
    logs = sorted((sandbox / "Tester").glob("**/logs/*.log"), key=lambda p: p.stat().st_mtime)
    specs, diagnostics = [], []
    for log in logs:
        if log.stat().st_mtime >= started:
            raw = log.read_bytes()
            offset = offsets.get(str(log), 0)
            if offset > len(raw):
                offset = 0
            encoding = "utf-16-le" if raw.startswith(b"\xff\xfe") else "utf-8"
            content = raw[offset:].decode(encoding, errors="replace")
            for line in content.splitlines():
                if "BROKER_SPEC " in line:
                    specs.append(line[line.index("BROKER_SPEC "):])
                for marker in ["BROKER_SPEC ", "BROKER_PENDING_LIMIT ", "BROKER_SESSION ", "BROKER_SESSION_SUMMARY ", "SESSION_PROBE ", "OPEN_VOLUME_AUDIT ", "BLAD:", "CEXIT_TEST_RESULT|", "CEXIT_TEST_EVENT|", "EQ_STAT ", "most:"]:
                    if marker in line:
                        diagnostics.append(line[line.index(marker):])
                        break
    (output / "tester_diagnostics.txt").write_text("\n".join(diagnostics), encoding="utf-8")
    manifest = {"expert": expert, "symbol": symbol, "from": start, "to_exclusive": end,
                "deposit": deposit, "leverage": leverage, "elapsed_seconds": time.time() - started,
                "terminal_exit_code": result.returncode, "report": fingerprint(report) if fresh_report else None,
                "broker_specs": list(dict.fromkeys(specs)), "model": "every_tick_based_on_real_ticks"}
    (output / "native_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    if not fresh_report:
        raise RuntimeError("Tester produced no fresh report. Inspect private tester diagnostics and sandbox logs.")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sandbox", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--expert", choices=sorted(EXPERTS), default="CONDUIT_XT")
    parser.add_argument("--compile-only", action="store_true")
    parser.add_argument("--symbol", default="XAUUSD")
    parser.add_argument("--from", dest="start", required=True)
    parser.add_argument("--to", dest="end", required=True)
    parser.add_argument("--deposit", type=float, default=300)
    parser.add_argument("--leverage", type=int, default=500)
    parser.add_argument("--parameters", type=Path, help="JSON object of explicit expert inputs.")
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    sandbox = args.sandbox.resolve(strict=True)
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    validate_sandbox(sandbox)
    with sandbox_lock(sandbox):
        compiled = compile_expert(sandbox, args.expert, output)
        (output / "compile_manifest.json").write_text(json.dumps(compiled, indent=2), encoding="utf-8")
        print(json.dumps(compiled), flush=True)
        if not args.compile_only:
            parameters = json.loads(args.parameters.read_text(encoding="utf-8")) if args.parameters else {}
            print(json.dumps(run_test(sandbox, output, args.expert, args.symbol, args.start, args.end,
                                     args.deposit, args.leverage, parameters, args.timeout)), flush=True)


if __name__ == "__main__":
    main()
