"""Read-only production-sidecar mock. No MT5, network, sleep or orders.

The fake implements the documented history_deals_get(ticket=ORDER) contract.
Run with Python -B. All identifiers, prices and account values are synthetic.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "sidecar/mt5_sidecar.py"
SOURCE_SHA = hashlib.sha256(SOURCE.read_bytes()).hexdigest()


class FakeMt5(types.ModuleType):
    def __init__(self):
        super().__init__("MetaTrader5")
        self.rows = []
        self.position_rows = []
        self.lookups = []

    def history_deals_get(self, *args, **kwargs):
        self.lookups.append(dict(kwargs))
        if "ticket" in kwargs:
            return tuple(d for d in self.rows if d.order == kwargs["ticket"])
        if "position" in kwargs:
            return tuple(d for d in self.rows if d.position_id == kwargs["position"])
        return tuple(self.rows)

    def positions_get(self, **kwargs):
        return tuple(p for p in self.position_rows
                     if ("ticket" not in kwargs or p.ticket == kwargs["ticket"])
                     and ("symbol" not in kwargs or p.symbol == kwargs["symbol"]))

    def symbol_info_tick(self, _symbol):
        return types.SimpleNamespace(bid=4002.0, ask=4002.2)

    def initialize(self, *args, **kwargs):
        raise AssertionError("FORBIDDEN: initialize")

    def login(self, *args, **kwargs):
        raise AssertionError("FORBIDDEN: login")

    def order_send(self, *args, **kwargs):
        raise AssertionError("FORBIDDEN: order_send")


FAKE = FakeMt5()
# importlib does not add the loaded script directory to sys.path. Mirror the
# packaged sibling-module layout without relying on a global PYTHONPATH.
with patch.dict(sys.modules, {"MetaTrader5": FAKE}), patch.object(sys, "path", [str(SOURCE.parent), *sys.path]):
    SPEC = importlib.util.spec_from_file_location("rf_pending_sidecar_id_probe", SOURCE)
    SIDECAR = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(SIDECAR)


def deal(ticket=880, order=700, identifier=433, entry=0, profit=0.0):
    return types.SimpleNamespace(ticket=ticket, order=order, position_id=identifier,
        entry=entry, type=0 if entry == 0 else 1, volume=.05, price=4000.0 if entry == 0 else 4002.0,
        time_msc=1700000000000 + entry, profit=profit, commission=-.2, swap=-.3,
        reason=3, magic=770077, comment="owned", symbol="XAUUSD")


def position(ticket=922, identifier=433):
    return types.SimpleNamespace(ticket=ticket, identifier=identifier, type=0,
        symbol="XAUUSD", volume=.05)


def subject(strict=True, ack=None):
    s = SIDECAR.Sidecar.__new__(SIDECAR.Sidecar)
    s.symbol = "XAUUSD"
    s.magic = 770077
    s.deviation = 30
    s.close_receipt_reconcile = strict
    s.closed_profit_net_costs = False
    s.filling_market = 1
    s.filling_pending = 2
    s.seen_deals = set()
    s.seen_order = []
    s.frames = []
    s.send = s.frames.append
    s._zapisz_poslizg = lambda *args: None
    response = ack or types.SimpleNamespace(retcode=10009, order=700, deal=880,
        volume=.05, price=4000.0, comment="mock ack")
    s._send_order = lambda *args: response
    return s


class ProductionHistoryIdentity(unittest.TestCase):
    def setUp(self):
        FAKE.rows = [deal()]
        FAKE.position_rows = [position()]
        FAKE.lookups = []
        self.sleep_patch = patch.object(SIDECAR.time, "sleep", lambda _: None)
        self.sleep_patch.start()

    def tearDown(self):
        self.sleep_patch.stop()

    def test_documented_order_lookup_is_not_deal_lookup_control(self):
        self.assertEqual(FAKE.history_deals_get(ticket=700)[0].ticket, 880)
        self.assertEqual(FAKE.history_deals_get(ticket=880), ())

    def test_actual_position_helper_finds_stable_identifier_when_order_differs(self):
        self.assertEqual(subject()._position_of_deal(880), 433)

    def test_actual_open_ack_keeps_verified_stable_identity_when_order_differs(self):
        ack = subject().cmd_open_market({"side": "buy", "volume": .05})
        self.assertEqual((ack["position_identifier"], ack["position"]), (433, 922))

    def test_actual_helper_must_not_take_another_orders_first_deal(self):
        FAKE.rows.append(deal(ticket=990, order=880, identifier=555))
        self.assertEqual(subject()._position_of_deal(880), 433)

    def test_actual_open_must_not_assign_another_live_position_on_id_collision(self):
        FAKE.rows.append(deal(ticket=990, order=880, identifier=555))
        FAKE.position_rows.append(position(ticket=999, identifier=555))
        ack = subject().cmd_open_market({"side": "buy", "volume": .05})
        self.assertEqual((ack["position_identifier"], ack["position"]), (433, 922))

    def test_actual_close_ack_profit_uses_its_exact_deal_not_order_with_same_number(self):
        FAKE.rows.append(deal(ticket=881, order=701, entry=1, profit=7.0))
        ack = types.SimpleNamespace(retcode=10009, order=701, deal=881,
            volume=.05, price=4002.0, comment="mock close")
        result = subject(ack=ack)._close(922, .05)
        self.assertAlmostEqual(result["profit"], 6.5)

    def test_actual_stream_preserves_position_identifier_independently_of_bad_ack_lookup(self):
        FAKE.rows.append(deal(ticket=881, order=701, entry=1, profit=7.0))
        s = subject()
        s.poll_deals()
        self.assertEqual(len(s.frames), 1)
        closed = s.frames[0]
        self.assertEqual((closed["deal"], closed["position"]), (881, 433))
        self.assertEqual((closed["profit"], closed["commission"], closed["swap"]), (7.0, -.2, -.3))

    def test_exact_ack_selection_proposal_uses_order_then_filters_deal(self):
        # Proposed algorithm only in this fixture; production remains untouched.
        FAKE.rows.extend([deal(ticket=879), deal(ticket=990, order=880, identifier=555)])
        rows = FAKE.history_deals_get(ticket=700)
        matches = [d for d in rows if d.order == 700 and d.ticket == 880]
        self.assertEqual(len(matches), 1)
        self.assertEqual(matches[0].position_id, 433)


if __name__ == "__main__":
    print(json.dumps({"source": str(SOURCE.relative_to(ROOT)), "sha256": SOURCE_SHA,
        "mode": "documented-api-mock-only", "real_mt5_imported": False,
        "mock_contract_test": True}), flush=True)
    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(ProductionHistoryIdentity))
    unchanged = SOURCE_SHA == hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    print(json.dumps({"source_unchanged": unchanged, "run": result.testsRun,
        "failures": len(result.failures), "errors": len(result.errors),
        "successful": result.wasSuccessful()}), flush=True)
    raise SystemExit(0 if result.wasSuccessful() and unchanged else 1)
