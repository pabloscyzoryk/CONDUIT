"""Receipt identity fixtures only: imports the fake package, never real MetaTrader5."""
import unittest
from types import SimpleNamespace as NS
from unittest.mock import patch
from test_follow_terminal import mod, FakeMT5


class ReceiptIdentityTests(unittest.TestCase):
    def setUp(self):
        self.fake = FakeMT5()
        mod.mt5 = self.fake
        self.position = NS(ticket=2000, identifier=1000, volume=.08, type=0,
            symbol="XAUUSD", price_open=3000., time_msc=1000, sl=2990., tp=3010.,
            profit=0., magic=777, comment="CD1.0")
        self.fake.positions_get = lambda **kw: [self.position]
        self.history = []
        # Documented Python API: `ticket` filters DEAL_ORDER, not DEAL ticket.
        self.fake.history_deals_get = lambda **kw: [d for d in self.history
            if ("ticket" not in kw or d.order == kw["ticket"])
            and ("position" not in kw or d.position_id == kw["position"])]
        def send(req):
            self.history = [NS(ticket=4000, order=3000,
                position_id=self.position.identifier,
                type=req["type"], entry=1 if "position" in req else 0,
                symbol="XAUUSD", magic=777, volume=.02, price=3000.5,
                profit=1., commission=0., swap=0., fee=0.)]
            return NS(retcode=10009, order=3000, deal=4000,
                comment="fixture", price=3000.5, volume=.02)
        self.fake.order_send = send

    def sidecar(self, on=True):
        args = mod.parse_args(["--port", "1", "--login", "999", "--server", "OLD-REAL", "--magic", "777"]
            + (["--close-receipt-reconcile"] if on else []))
        if not on:
            # Preserve this historical fixture's OFF assertion unchanged.
            # The new exact-ACK suite separately proves OFF byte/query behavior
            # against the realistic ORDER-filter mock (including wrong lookup).
            self.fake.history_deals_get = lambda **kw: [NS(position_id=1000,
                profit=1., commission=0., swap=0.)]
        s = mod.Sidecar(args)
        s.init_terminal()
        s.request_account = dict(s.bound_account) if s.bound_account else None
        return s

    def test_position_snapshot_carries_distinct_ticket_and_identifier(self):
        row = self.sidecar().cmd_positions({"all": True})[0]
        self.assertEqual((row["ticket"], row["identifier"]), (2000, 1000))

    def test_close_ack_carries_identifier_and_actual_executed_volume(self):
        result = self.sidecar().cmd_close_partial({"ticket": 2000, "volume": .04})
        self.assertEqual((result["position"], result["position_identifier"], result["deal"]), (2000, 1000, 4000))
        self.assertEqual(result["volume"], .02)

    def test_close_ack_uses_exact_position_deal_when_broker_omits_order_ticket(self):
        original_send = self.fake.order_send
        def send_without_order(req):
            result = original_send(req)
            result.order = 0
            self.history[0].order = 0
            return result
        self.fake.order_send = send_without_order
        result = self.sidecar().cmd_close_partial({"ticket": 2000, "volume": .04})
        self.assertEqual((result["position"], result["position_identifier"], result["deal"]), (2000, 1000, 4000))
        self.assertTrue(result["ack_identity_complete"])

    def test_open_resolves_current_ticket_separately_from_deal_position_id(self):
        result = self.sidecar().cmd_open_market({"side": "buy", "volume": .02})
        self.assertEqual((result["position"], result["position_identifier"]), (2000, 1000))

    def test_off_preserves_legacy_open_ticket_result(self):
        result = self.sidecar(False).cmd_open_market({"side": "buy", "volume": .02})
        self.assertEqual(result["position"], 1000)

    def test_missing_physical_open_ticket_is_not_guessed_from_identifier(self):
        self.fake.positions_get = lambda **kw: []
        result = self.sidecar().cmd_open_market({"side": "buy", "volume": .02})
        self.assertEqual(result["position"], 0)
        self.assertEqual(result["position_identifier"], 1000)

    def test_none_snapshot_is_error_on_not_silently_empty(self):
        s = self.sidecar()
        self.fake.positions_get = lambda **kw: None
        self.fake.orders_get = lambda **kw: None
        with self.assertRaises(mod.BrokerError): s.cmd_positions({})
        with self.assertRaises(mod.BrokerError): s.cmd_orders({})
        legacy = self.sidecar(False)
        self.assertEqual(legacy.cmd_positions({}), [])
        self.assertEqual(legacy.cmd_orders({}), [])

    def test_receipts_pin_fixed_account_without_altering_legacy_login_policy(self):
        s = self.sidecar()
        self.assertEqual(self.fake.initializations[-1]["login"], 999)
        self.fake.account = NS(login=42, server="OTHER", company="Vantage", trade_mode=0)
        with self.assertRaises(mod.BrokerError): s.cmd_close_partial({"ticket": 2000, "volume": .02})

    def test_zero_identifier_is_explicit_not_guessed_from_ticket(self):
        self.position.identifier = 0
        s = self.sidecar()
        self.assertEqual(s.cmd_positions({})[0]["identifier"], 0)
        self.assertEqual(s.cmd_close_partial({"ticket": 2000, "volume": .02})["position_identifier"], 0)


if __name__ == "__main__":
    unittest.main()
