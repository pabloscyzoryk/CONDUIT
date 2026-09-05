"""Run one private, reproducible Rust/native MT5 comparison on a shared corpus."""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import subprocess
import uuid

import compare
import contract
import portable


def invoke(command: list[str], output: Path, name: str, timeout: int = 10800):
    process = subprocess.run(command, capture_output=True, timeout=timeout)
    (output / (name + "_stdout.txt")).write_bytes(process.stdout)
    (output / (name + "_stderr.txt")).write_bytes(process.stderr)
    if process.returncode:
        raise RuntimeError(name + " failed; inspect its private stderr log.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["btp", "most", "defaults", "ticks", "messages", "preset", "sandbox", "common-files", "out"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--from", dest="start", required=True)
    parser.add_argument("--to", dest="end", required=True)
    parser.add_argument("--symbol", required=True)
    parser.add_argument("--channel", default="Synergy")
    parser.add_argument("--deposit", type=float, default=300)
    parser.add_argument("--leverage", type=int, default=500)
    parser.add_argument("--price-digits", type=int, required=True)
    parser.add_argument("--limit-price-improvement", action="store_true")
    parser.add_argument("--live-telegram-ingress", action="store_true")
    parser.add_argument("--native-swap-cash-digits", type=int, choices=range(9))
    parser.add_argument("--new-pending-sl-next-tick", action="store_true")
    parser.add_argument("--native-detail-from-ms", type=int, default=0)
    parser.add_argument("--native-detail-to-ms", type=int, default=0)
    parser.add_argument("--timeout", type=int, default=10800,
                        help="Per-process ceiling in seconds; full native windows can exceed 30 minutes.")
    args = parser.parse_args()
    if args.timeout <= 0:
        raise ValueError("Timeout must be positive.")
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if (output / "experiment.json").exists():
        raise ValueError("Use a fresh output directory to preserve earlier evidence.")
    sandbox = args.sandbox.resolve(strict=True)
    portable.validate_sandbox(sandbox)
    defaults = json.loads(args.defaults.read_text(encoding="utf-8-sig"))
    preset = json.loads(args.preset.read_text(encoding="utf-8-sig"))
    preset["settings"] = {**defaults, **preset["settings"]}
    effective = output / "effective_preset.json"
    effective.write_text(json.dumps(preset, indent=2), encoding="utf-8")
    settings = preset["settings"]
    offset = settings.get("msg_clock_offset_ms")
    if offset is None:
        offset = settings["server_tz_offset_ms"]
    run_id = uuid.uuid4().hex
    common = args.common_files.resolve(strict=True) / "CONDUIT_TEST" / run_id
    common.mkdir(parents=True)
    bridge_name = "CONDUIT_TEST/" + run_id + "/bridge.csv"
    diag_name = "CONDUIT_TEST/" + run_id + "/diagnostics.csv"
    source = Path(__file__).resolve().parents[1] / "mql5/CONDUIT_XT.mq5"
    source_bytes = source.read_bytes()
    parameters, report = contract.build(defaults, settings, source_bytes.decode("utf-8-sig"), bridge_name)
    report["fingerprints"] = {"defaults": contract.fingerprint(args.defaults),
                              "preset": contract.fingerprint(effective), "expert": hashlib.sha256(source_bytes).hexdigest()}
    (output / "contract.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    if not report["mapping_complete"]:
        raise ValueError("Incomplete strategy mapping; inspect contract.json before running a different strategy.")
    parameters.update(In_Diag=True, In_DiagFile=diag_name,
                      In_DiagDetailFromMs=args.native_detail_from_ms,
                      In_DiagDetailToMs=args.native_detail_to_ms)
    (output / "parameters.json").write_text(json.dumps(parameters, indent=2), encoding="utf-8")
    bridge = output / "bridge.csv"
    command = [str(args.most.resolve()), "--signals", str(args.messages.resolve()), "--preset", str(effective),
               "--from", args.start, "--to", args.end, "--offset-ms", str(offset),
               "--kanal", args.channel, "--schema", "v2", "--out", str(bridge)]
    if args.live_telegram_ingress:
        command.append("--live-telegram-ingress")
    invoke(command, output, "bridge", args.timeout)
    (common / "bridge.csv").write_bytes(bridge.read_bytes())
    native = output / "native"
    native.mkdir()
    rust = output / "rust"
    rust_command = [str(args.btp.resolve()), "--ticks", str(args.ticks.resolve()),
                    "--signals", str(args.messages.resolve()), "--preset", str(effective),
                    "--from", args.start, "--to", args.end, "--balance", str(args.deposit),
                    "--out", str(rust), "--sim-price-digits", str(args.price_digits),
                    "--dump-trades", "--no-charts", "--quiet"]
    if args.limit_price_improvement:
        rust_command.append("--sim-limit-price-improvement")
    if args.live_telegram_ingress:
        rust_command.append("--live-telegram-ingress")
    if args.native_swap_cash_digits is not None:
        rust_command.extend(["--sim-native-swap-cash-digits", str(args.native_swap_cash_digits)])
    if args.new_pending_sl_next_tick:
        rust_command.append("--sim-new-pending-sl-next-tick")
    with portable.sandbox_lock(sandbox):
        compiled = portable.compile_expert(sandbox, "CONDUIT_XT", native, source_bytes)
        (native / "compile_manifest.json").write_text(json.dumps(compiled, indent=2), encoding="utf-8")
        with ThreadPoolExecutor(max_workers=1) as pool:
            future = pool.submit(invoke, rust_command, output, "rust", args.timeout)
            portable.run_test(sandbox, native, "CONDUIT_XT", args.symbol, args.start, args.end,
                              args.deposit, args.leverage, parameters, args.timeout)
            future.result()
    ledger = native / "diagnostics.csv"
    ledger.write_bytes((common / "diagnostics.csv").read_bytes())
    result = compare.compare(compare.native_trades(ledger), json.loads((rust / "transakcje.json").read_text(encoding="utf-8")))
    metrics = json.loads((rust / "wyniki_compound.json").read_text(encoding="utf-8"))
    if len(metrics) != 1:
        raise ValueError("A paired experiment must produce exactly one Rust metrics record.")
    result["final_account"] = compare.compare_final_account(ledger, next(iter(metrics.values())), result["native_net"])
    result["ledger_and_equity_match"] = result["execution_fields_match"] and result["final_account"]["account_fields_match"]
    (output / "comparison.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    experiment = {"schema": "conduit.mt5.experiment.v1", "from": args.start, "to_exclusive": args.end,
                  "symbol": args.symbol, "deposit": args.deposit, "leverage": args.leverage,
                  "live_telegram_ingress": args.live_telegram_ingress,
                  "native_swap_cash_digits": args.native_swap_cash_digits,
                  "new_pending_sl_next_tick": args.new_pending_sl_next_tick,
                  "limit_price_improvement": args.limit_price_improvement, "price_digits": args.price_digits,
                  "clock_offset_ms": offset, "settings": contract.fingerprint(effective),
                  "btp": portable.fingerprint(args.btp), "most": portable.fingerprint(args.most),
                  "defaults": portable.fingerprint(args.defaults), "native_expert": compiled,
                  "ticks": portable.fingerprint(args.ticks), "messages": portable.fingerprint(args.messages),
                  "comparison": {k: v for k, v in result.items() if k not in {"mismatches", "limits"}}}
    (output / "experiment.json").write_text(json.dumps(experiment, indent=2), encoding="utf-8")
    print(json.dumps(experiment["comparison"]), flush=True)


if __name__ == "__main__":
    main()
