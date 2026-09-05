"""Compare native millisecond deal ledger with btp transakcje.json.

This comparison contains simulated execution facts, no account identity or
message text. Preserve the original private ledgers as the detailed evidence.
"""
from __future__ import annotations

import argparse
import csv
import json
from pathlib import Path

from portable import read_text


def native_trades(path: Path) -> list[dict]:
    rows = list(csv.reader(read_text(path).splitlines(), delimiter="\t"))
    proof = [r for r in rows if r and r[0] == "COST_DIAG_READ_PROOF"]
    if not proof or proof[-1][1] != "1" or proof[-1][4] != "0":
        raise ValueError("Native ledger lacks a complete cost-read proof.")
    positions = {}
    trades = []
    for row in rows:
        if not row or row[0] != "DEAL":
            continue
        if len(row) < 14:
            raise ValueError("Native ledger uses an incomplete deal schema.")
        ts, position, direction, side = int(row[1]), row[3], int(row[4]), int(row[5])
        volume, price = float(row[6]), float(row[7])
        if direction == 0:
            if position in positions:
                raise ValueError("Multiple native entries into one position need an explicit netting/scale model.")
            positions[position] = {"open_ts": ts, "side": "Buy" if side == 0 else "Sell",
                                   "open_price": price, "entry_cost": float(row[10]) + float(row[13]),
                                   "entry_volume": volume}
        elif direction == 1:
            if position not in positions:
                raise ValueError("Native close has no corresponding entry in the ledger.")
            entry = positions[position]
            trades.append({k: entry[k] for k in ["open_ts", "side", "open_price"]} | {
                "close_ts": ts, "volume": volume, "close_price": price,
                "net_profit": float(row[8]) + float(row[9]) + float(row[10]) + float(row[13])
                              + entry["entry_cost"] * volume / entry["entry_volume"],
                "reason": {3: "Expert", 4: "Sl", 5: "Tp", 6: "StopOut"}.get(int(row[11]), "Other"),
            })
        else:
            raise ValueError("Unsupported native close-by or netting ledger direction.")
    return sorted(trades, key=lambda t: (t["close_ts"], t["open_ts"], t["side"], t["volume"]))


def compare(native: list[dict], rust: list[dict]) -> dict:
    rust = sorted(rust, key=lambda t: (t["close_ts"], t["open_ts"], t["side"], t["volume"]))
    mismatches = []
    keys = ["open_ts", "close_ts", "side", "volume", "open_price", "close_price"]
    differences = {key: 0 for key in keys + ["net_profit", "reason"]}
    for number, (mt, bt) in enumerate(zip(native, rust), 1):
        details = {}
        for key in keys:
            equal = mt[key] == bt[key] if key in {"open_ts", "close_ts", "side"} else abs(mt[key] - bt[key]) < 1e-7
            if not equal:
                details[key] = {"native": mt[key], "rust": bt[key]}
                differences[key] += 1
        net = bt["profit"] + bt.get("commission", 0) + bt.get("swap", 0)
        if abs(mt["net_profit"] - net) > 0.0050001:
            details["net_profit"] = {"native": mt["net_profit"], "rust": net}
            differences["net_profit"] += 1
        # Expert represents multiple application-directed close reasons.
        if mt["reason"] in {"Sl", "Tp", "StopOut"} and mt["reason"] != bt["reason"]:
            details["reason"] = {"native": mt["reason"], "rust": bt["reason"]}
            differences["reason"] += 1
        if details:
            mismatches.append({"ordinal": number, "differences": details})
    complete = len(native) == len(rust)
    return {"schema": "conduit.mt5.execution-comparison.v1", "native_trades": len(native),
            "rust_trades": len(rust), "counts_match": complete,
            "native_net": sum(x["net_profit"] for x in native),
            "rust_net": sum(x["profit"] + x.get("commission", 0) + x.get("swap", 0) for x in rust),
            "matching_trades": len(native) - len(mismatches) if complete else None,
            "differences_by_field": differences, "mismatches": mismatches,
            "execution_fields_match": complete and not mismatches,
            "limits": "Chronological closed-trade comparison; matching outcomes alone do not prove identical pending or basket state."}


def compare_final_account(path: Path, metrics: dict, closed_net: float) -> dict:
    """Compare equity before native tester liquidation with Rust marked equity.

    Rust's end_balance metric includes final marking in this report format, so
    it must not be mistaken for broker cash while positions remain open.
    """
    rows = list(csv.reader(read_text(path).splitlines(), delimiter="\t"))
    accounts = [r for r in rows if r and r[0] == "FINAL_ACCOUNT"]
    if len(accounts) != 1 or len(accounts[0]) != 7:
        raise ValueError("Exactly one complete pre-liquidation account snapshot is required.")
    account = accounts[0]
    balance, equity, margin = map(float, account[4:7])
    positions = [r for r in rows if r and r[0] == "OPEN_POSITION"]
    orders = [r for r in rows if r and r[0] == "OPEN_ORDER"]
    if any(len(r) != 10 for r in positions) or any(len(r) != 8 for r in orders):
        raise ValueError("Incomplete final open-position/order schema.")
    floating = sum(float(r[8]) + float(r[9]) for r in positions)
    rust_equity = float(metrics["end_equity"])
    tolerance = 0.0050001
    equity_match = abs(equity - rust_equity) <= tolerance
    cash_match = abs(balance - (float(metrics["start_balance"]) + closed_net)) <= tolerance
    snapshot_consistent = abs(equity - balance - floating) <= tolerance
    return {"native_balance_before_liquidation": balance, "native_equity": equity,
            "rust_marked_equity": rust_equity, "equity_difference": equity - rust_equity,
            "native_margin": margin, "native_floating_net": floating,
            "native_open_positions": len(positions), "native_pending_orders": len(orders),
            "equity_matches": equity_match, "cash_matches_closed_ledger": cash_match,
            "native_snapshot_consistent": snapshot_consistent,
            "account_fields_match": equity_match and cash_match and snapshot_consistent,
            "limits": "Final equity and native cash/float reconcile; individual Rust open positions and pending state are not exported by this comparison."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = compare(native_trades(args.native), json.loads(args.rust.read_text(encoding="utf-8")))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps({k: v for k, v in result.items() if k not in {"mismatches", "limits"}}))


if __name__ == "__main__":
    main()
