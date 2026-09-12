import csv
import json
from pathlib import Path
import struct
import tempfile
import unittest

import compare
import contract
from mapping import MAP, ENUMY
import tick_compare


class MappingContractTests(unittest.TestCase):
    def test_wire_numeric_schema_preserves_none_and_rejects_accidental_zero(self):
        for action in ['SPP:key,nan,nan,', 'SPP:key,nan,100,110,120', 'RF:key,nan', 'SETSL:key,0',
                       'TPHIT2:key,-1,nan,1', 'TPHIT2:key,0,100,0', 'TPCORR:key,32,130',
                       'ENTRY2:key,BUY,1,0,100,101,nan,1,nan,0,0,0,0,',
                       'ENTRY:key,BUY,1,100,101,nan,1,']:
            self.assertIsNone(contract.wire_action_error(action, 32), action)
        for action in ['SPP:key,,nan,', 'RF:key,', 'SETSL:key,nan', 'SETSL:key,', 'SETSL:key,inf',
                       'TPHIT2:key,-2,nan,0', 'TPCORR:key,33,130', 'BE:key,100',
                       'ENTRY2:key,BUY,1,0,100,101,nan,1,nan,0,0,0,1,',
                       'SPP:key,nan,nan,110,,120']:
            self.assertIsNotNone(contract.wire_action_error(action, 32), action)

    def test_explicit_g8i_profit_basis_never_adds_swap_twice(self):
        self.assertEqual(compare.rust_net({'profit': 7, 'commission': -1, 'swap': -3,
                                          'profit_basis': 'PricePlusSwap', 'net_profit': 6}), 6)
        self.assertEqual(compare.rust_net({'profit': 10, 'commission': -1, 'swap': -3,
                                          'profit_basis': 'PriceOnlyGross', 'net_profit': 6}), 6)
        with self.assertRaises(ValueError):
            compare.rust_net({'profit': 7, 'commission': -1, 'swap': -3,
                              'profit_basis': 'PricePlusSwap', 'net_profit': 3})
        with self.assertRaises(ValueError):
            compare.rust_net({'profit': 7, 'commission': float('nan'), 'swap': -3,
                              'profit_basis': 'PricePlusSwap'})

    def test_native_target_preflight_bound_without_truncation(self):
        def entry(count):
            return 'M|1000|1|0|0||synthetic|ENTRY2:key,BUY,1,0,100,101,90,0,nan,0,0,0,' + str(count) + ',' + ','.join(str(110+i) for i in range(count))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'bridge.csv'
            path.write_text(entry(30))
            result = contract.validate_bridge_capacity(path, {'runner_cele_n': 2, 'runner_cele_krok': 1}, '#define MAXTP 32')
            self.assertTrue(result['complete'])
            self.assertEqual(result['maximum_effective_target_bound'], 32)
            result = contract.validate_bridge_capacity(path, {'runner_cele_n': 3, 'runner_cele_krok': 1}, '#define MAXTP 32')
            self.assertFalse(result['complete'])
            self.assertEqual(result['errors'][0]['reason'], 'native_target_capacity')
            path.write_text(entry(33))
            self.assertFalse(contract.validate_bridge_capacity(path, {}, '#define MAXTP 32')['complete'])

    def test_native_target_preflight_rejects_malformed_count_and_respects_inactive_spp(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'bridge.csv'
            path.write_text('M|1000|1|0|0||synthetic|ENTRY2:key,BUY,1,0,100,101,90,0,nan,0,0,0,2,110')
            self.assertEqual(contract.validate_bridge_capacity(path, {}, '#define MAXTP 32')['errors'][0]['reason'], 'malformed_target_count')
            path.write_text('M|1000|1|0|0||synthetic|SPP:key,nan,nan,' + ','.join(str(100+i) for i in range(33)))
            self.assertFalse(contract.validate_bridge_capacity(path, {}, '#define MAXTP 32')['complete'])
            self.assertTrue(contract.validate_bridge_capacity(path, {'spp_keep_tp': True}, '#define MAXTP 32')['complete'])

    def baseline(self):
        values = {name: False if convert is bool else "" if convert is str else 0
                  for name, _, convert in MAP}
        values.update({name: next(iter(options)) for name, (_, options) in ENUMY.items()})
        source = (Path(__file__).resolve().parents[1] / "mql5/CONDUIT_XT.mq5").read_text(encoding="utf-8")
        return values, source

    def test_unknown_active_default_is_not_silently_ignored(self):
        defaults, source = self.baseline()
        defaults["future_strategy_feature"] = True
        _, report = contract.build(defaults, {}, source, "synthetic.csv")
        self.assertFalse(report["mapping_complete"])
        self.assertIn("future_strategy_feature", [x["field"] for x in report["errors"]])

    def test_supported_defaults_have_real_ea_inputs(self):
        defaults, source = self.baseline()
        _, report = contract.build(defaults, {}, source, "synthetic.csv")
        self.assertEqual(report["errors"], [])
        self.assertFalse(report["execution_parity_proven"])

    def test_active_hybrid_leg_is_rejected(self):
        defaults, source = self.baseline()
        _, report = contract.build(defaults, {"market_hybrid_now_units": 1}, source, "synthetic.csv")
        self.assertIn("market_hybrid_now_units", [x["field"] for x in report["errors"]])

    def test_t100_inactive_defaults_preserve_xt_but_active_policy_is_not_native(self):
        defaults, source = self.baseline()
        defaults["t100"] = {"enabled": False, "risk_pct": 1.0, "experts": 15}
        _, inactive = contract.build(defaults, {}, source, "synthetic.csv")
        self.assertEqual(inactive["errors"], [])
        self.assertEqual(inactive["fields"]["t100"], "unsupported_feature_inactive")
        for value in [{"enabled": True}, {"enabled": 0}, {"enabled": "false"}, {}, None]:
            _, report = contract.build(defaults, {"t100": value}, source, "synthetic.csv")
            self.assertFalse(report["mapping_complete"], value)
            self.assertIn("t100", [row["field"] for row in report["errors"]])
            self.assertFalse(report["execution_parity_proven"])

    def test_older_binary_explicitly_disables_new_profit_budget(self):
        defaults, source = self.baseline()
        for name in list(defaults):
            if name.startswith("profit_budget_"):
                del defaults[name]
        inputs, report = contract.build(defaults, {}, source, "synthetic.csv")
        self.assertEqual(report["errors"], [])
        self.assertEqual(inputs["In_ProfitBudgetArmPct"], 0.0)
        _, partial = contract.build(defaults, {"profit_budget_arm_pct": 1.0}, source, "synthetic.csv")
        self.assertIn("profit_budget_keep_pct", [x["field"] for x in partial["errors"]])

    def test_boolean_string_cannot_silently_enable_a_feature(self):
        defaults, source = self.baseline()
        _, report = contract.build(defaults, {"trail_adaptive_enabled": "false"}, source, "synthetic.csv")
        self.assertIn({"field": "trail_adaptive_enabled", "reason": "boolean_type_required"}, report["errors"])

    def test_unported_volume_sizing_rejects_active_modes_but_accepts_disabled_parameters(self):
        defaults, source = self.baseline()
        defaults.update(vol_size_mode="Off", vol_size_target=4.0, vol_size_percentile=60.0)
        _, inactive = contract.build(defaults, {}, source, "synthetic.csv")
        self.assertEqual(inactive["errors"], [])
        for mode in ["Percentile", "Target"]:
            _, active = contract.build(defaults, {"vol_size_mode": mode}, source, "synthetic.csv")
            self.assertIn("vol_size_mode", [error["field"] for error in active["errors"]])


class TickComparisonTests(unittest.TestCase):
    def run_case(self, rows, native_rows):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            header = bytearray(64)
            header[:4] = b"CDTK"
            struct.pack_into("<Q", header, 8, len(rows))
            (root / "ticks.bin").write_bytes(header + b"".join(tick_compare.RECORD.pack(*row) for row in rows))
            with (root / "native.csv").open("w", encoding="utf-8", newline="") as stream:
                writer = csv.writer(stream, delimiter=";")
                writer.writerow(["server_time_ms", "bid", "ask"])
                writer.writerows(native_rows)
            return tick_compare.compare(root / "native.csv", root / "ticks.bin", 1000, 2000)

    def test_equal_count_does_not_hide_changed_quote(self):
        rows = [(1100, 2090.01, 2090.03), (1200, 2090.02, 2090.04)]
        self.assertTrue(self.run_case(rows, rows)["bitwise_equal"])
        changed = rows[:1] + [(1200, 2090.03, 2090.04)]
        self.assertFalse(self.run_case(rows, changed)["bitwise_equal"])

    def test_missing_tail_is_not_equal(self):
        rows = [(1100, 2090.01, 2090.03), (1200, 2090.02, 2090.04)]
        self.assertFalse(self.run_case(rows, rows[:1])["bitwise_equal"])

    def test_empty_window_is_not_native_evidence(self):
        self.assertFalse(self.run_case([], [])["bitwise_equal"])


class LedgerTests(unittest.TestCase):
    def test_final_float_mismatch_is_not_hidden_by_equal_closed_profit(self):
        with tempfile.TemporaryDirectory() as temp:
            ledger = Path(temp) / "ledger.csv"
            ledger.write_text("FINAL_ACCOUNT\t2000\t2090\t2090.2\t301\t299\t4\n"
                              "OPEN_POSITION\t1\t0\t0.01\t2092\t1000\t2080\t2100\t-2\t0\n", encoding="utf-16")
            result = compare.compare_final_account(ledger, {"start_balance": 300, "end_equity": 300}, 1)
            self.assertTrue(result["cash_matches_closed_ledger"])
            self.assertTrue(result["native_snapshot_consistent"])
            self.assertFalse(result["account_fields_match"])

    def test_missing_final_snapshot_is_not_assumed_flat(self):
        with tempfile.TemporaryDirectory() as temp:
            ledger = Path(temp) / "ledger.csv"
            ledger.write_text("", encoding="utf-16")
            with self.assertRaises(ValueError):
                compare.compare_final_account(ledger, {"start_balance": 300, "end_equity": 300}, 0)

    def test_missing_cost_read_proof_is_not_confirmed_zero(self):
        with tempfile.TemporaryDirectory() as temp:
            ledger = Path(temp) / "ledger.csv"
            ledger.write_text("COST_DIAG_SCHEMA\t3\t1\n", encoding="utf-16")
            with self.assertRaises(ValueError):
                compare.native_trades(ledger)

    def test_equal_profit_does_not_hide_different_execution(self):
        native = [{"open_ts": 1000, "close_ts": 2000, "side": "Buy", "volume": 0.01,
                   "open_price": 2090.0, "close_price": 2091.0, "net_profit": 1.0, "reason": "Sl"}]
        rust = [{**native[0], "profit": 1.0, "close_ts": 2001}]
        result = compare.compare(native, rust)
        self.assertFalse(result["execution_fields_match"])
        self.assertEqual(result["differences_by_field"]["close_ts"], 1)

    def test_legacy_simulator_profit_contains_swap_exactly_once(self):
        trade = {"profit": 1.49, "swap": -0.81, "commission": 0}
        self.assertEqual(compare.rust_net(trade), 1.49)
        with self.assertRaises(ValueError):
            compare.rust_net({**trade, "commission": -0.5})

    def test_session_snapshot_requires_closed_day_proof_and_exact_intervals(self):
        days = [{"day_sun0": day, "trade": [], "quote": []} for day in range(7)]
        diagnostic = "\n".join(f"BROKER_SESSION_SUMMARY day_sun0={day} trade_count=0 quote_count=0 clock=broker" for day in range(7))
        profile = {"clock": "broker", "days": days}
        self.assertTrue(compare.compare_trade_sessions(profile, diagnostic)["matches"])
        with self.assertRaises(ValueError):
            compare.compare_trade_sessions(profile, "\n".join(diagnostic.splitlines()[:-1]))
        days[1]["trade"] = [[3660, 86280]]
        self.assertFalse(compare.compare_trade_sessions(profile, diagnostic)["matches"])

    def test_canonical_net_contains_every_cost_and_requires_reconciliation(self):
        receipt = {"completeness": {"status": "complete"}, "volume": 0.01,
                   "gross_profit": 10, "entry_commission_alloc": -0.1, "exit_commission": -0.2,
                   "entry_fee_alloc": -0.3, "exit_fee": -0.4, "swap": -0.5}
        trade = {"profit_basis": "CanonicalClosedNetV1", "profit": 8.5, "commission": -0.3,
                 "swap": -0.5, "volume": 0.01, "cost_receipt": receipt}
        self.assertAlmostEqual(compare.rust_net(trade), 8.5)
        with self.assertRaises(ValueError):
            compare.rust_net({**trade, "profit": 8})
        with self.assertRaises(ValueError):
            compare.rust_net({**trade, "cost_receipt": {**receipt, "swap": None}})


if __name__ == "__main__":
    unittest.main()
