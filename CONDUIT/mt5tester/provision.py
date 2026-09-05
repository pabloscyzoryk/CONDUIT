"""Provision a private offline MT5 tester without changing the installed terminal.

Cached account files remain exclusively in the explicitly named VPSREADY sandbox.
Only terminal binaries, cached symbol/tick data and standard includes are copied.
"""
from __future__ import annotations

import argparse
import configparser
import json
import shutil
from pathlib import Path

from portable import read_text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--cached-data", type=Path, required=True)
    parser.add_argument("--sandbox", type=Path, required=True)
    parser.add_argument("--server", required=True)
    parser.add_argument("--symbol", required=True)
    parser.add_argument("--months", nargs="+", required=True, help="Cached tick months as YYYYMM.")
    args = parser.parse_args()
    source = args.cached_data.resolve(strict=True)
    binaries = args.binaries.resolve(strict=True)
    sandbox = args.sandbox.resolve()
    if not sandbox.name.startswith("VPSREADY_"):
        raise ValueError("Account cache must be placed in a private VPSREADY_ sandbox.")
    if sandbox in {source, binaries} or source in sandbox.parents or binaries in sandbox.parents \
            or sandbox in source.parents or sandbox in binaries.parents:
        raise ValueError("Sandbox must be separate from the source terminal.")
    if sandbox.exists() and any(sandbox.iterdir()):
        raise ValueError("Provisioning requires a fresh empty sandbox; it will not overwrite an existing terminal.")
    for value in [args.server, args.symbol, *args.months]:
        if any(c in value for c in ["/", "\\", "\r", "\n"]) or value in {".", ".."}:
            raise ValueError("Unsafe cache component.")
    cache = source / "bases" / args.server
    symbols = cache / "symbols"
    symbol_files = sorted(symbols.glob("symbols-*.dat"))
    if len(symbol_files) != 1:
        raise ValueError("Expected one unambiguous cached account symbol file for this server.")
    sandbox.mkdir(parents=True, exist_ok=True)
    for name in ["terminal64.exe", "MetaEditor64.exe", "metatester64.exe"]:
        shutil.copy2(binaries / name, sandbox / name)
    (sandbox / "portable.txt").write_text("", encoding="utf-8")
    (sandbox / "Config").mkdir(exist_ok=True)
    for name in ["accounts.dat", "servers.dat", "common.ini"]:
        shutil.copy2(source / "config" / name, sandbox / "Config" / name)
    config = configparser.ConfigParser(interpolation=None, strict=False)
    config.optionxform = str
    config.read_string(read_text(sandbox / "Config" / "common.ini"))
    for section in ["Common", "Experts"]:
        if not config.has_section(section):
            config.add_section(section)
    config["Common"].update({"Login": symbol_files[0].stem.split("-")[-1], "Server": args.server,
                            "ProxyEnable": "1", "ProxyType": "0", "ProxyAddress": "127.0.0.1:1",
                            "ProxyAuth": "", "NewsEnable": "0", "CertInstall": "0"})
    config["Experts"].update({"Enabled": "0", "AllowLiveTrading": "0", "AllowDllImport": "0"})
    with (sandbox / "Config" / "common.ini").open("w", encoding="utf-16") as stream:
        config.write(stream, space_around_delimiters=False)
    counts = {"symbols": 0, "tick_months": 0}
    (sandbox / "Bases" / args.server / "symbols").mkdir(parents=True, exist_ok=True)
    for path in symbols.glob("*.dat"):
        shutil.copy2(path, sandbox / "Bases" / args.server / "symbols" / path.name)
        counts["symbols"] += 1
    ticks = sandbox / "Bases" / args.server / "ticks" / args.symbol
    ticks.mkdir(parents=True, exist_ok=True)
    for month in args.months:
        if len(month) != 6 or not month.isdigit():
            raise ValueError("Tick month must be YYYYMM.")
        shutil.copy2(cache / "ticks" / args.symbol / (month + ".tkc"), ticks / (month + ".tkc"))
        counts["tick_months"] += 1
    tick_index = cache / "ticks" / args.symbol / "ticks.dat"
    if tick_index.exists():
        shutil.copy2(tick_index, ticks / tick_index.name)
    # Locked live .hcc history is deliberately unnecessary: model 4 rebuilds bars
    # from real tick cache. Fresh profiles ensure no live chart expert is restored.
    for name in ["Experts", "Files", "Scripts", "Profiles/Tester"]:
        (sandbox / "MQL5" / name).mkdir(parents=True, exist_ok=True)
    shutil.copytree(source / "MQL5" / "Include", sandbox / "MQL5" / "Include", dirs_exist_ok=True)
    (sandbox / "conduit_tester_sandbox.json").write_text(json.dumps({
        "purpose": "offline_strategy_tester", "live_trading": False,
        "server": args.server, "symbol": args.symbol, "tick_months": args.months,
    }, indent=2), encoding="utf-8")
    print(json.dumps({"ready": True, "copied": counts, "live_terminal_modified": False}))


if __name__ == "__main__":
    main()
