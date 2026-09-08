"""RDD account clock metadata through the real sidecar, using only FakeMT5."""
import unittest
from types import SimpleNamespace as NS
from unittest.mock import patch
from test_follow_terminal import FakeMT5, mod


class AccountRddClockTests(unittest.TestCase):
    def setUp(self):
        self.fake = FakeMT5()
        mod.mt5 = self.fake
        self.fake.account = NS(login=42, server="fixture-demo", company="fixture", trade_mode=0,
                               balance=200., equity=180., margin=0., margin_free=180.,
                               credit=0., leverage=100, currency="USD", name="fixture")
        self.mono = 100.0
        self.utc = 1_800_000_000.0
        self.stamp = 100 * 86400000 + 12 * 3600000
        self.clock = patch.object(mod.time, "monotonic", side_effect=lambda: self.mono)
        self.wall = patch.object(mod.time, "time", side_effect=lambda: self.utc)
        self.clock.start(); self.wall.start()
        self.addCleanup(self.clock.stop); self.addCleanup(self.wall.stop)
        self.sidecar = mod.Sidecar(mod.parse_args(["--port", "1", "--symbol", "XAUUSD"]))
        self.sidecar._quote_clock_account = mod.account_key(self.fake.account)

    def tearDown(self):
        self.assertEqual(self.fake.initializations, [])
        self.assertEqual(self.fake.orders_sent, [])

    def advance(self, delta=1):
        self.mono += delta; self.utc += delta; self.stamp += int(delta * 1000)
        self.sidecar._observe_quote_clock("XAUUSD", NS(time_msc=self.stamp))

    def established(self):
        self.sidecar._observe_quote_clock("XAUUSD", NS(time_msc=self.stamp))
        self.advance()

    def test_cached_initial_and_stale_weekend_do_not_invent_broker_day(self):
        self.sidecar._observe_quote_clock("XAUUSD", NS(time_msc=self.stamp))
        self.assertIsNone(self.sidecar.cmd_account({})["observation_broker_day"])
        self.advance()
        self.assertEqual(self.sidecar.cmd_account({})["observation_broker_day"], 100)
        self.mono += 31; self.utc += 31
        self.assertIsNone(self.sidecar.cmd_account({})["observation_broker_day"])

    def test_each_real_account_read_keeps_values_with_fresh_day_without_extra_mt5_calls(self):
        self.established()
        reads = []
        self.fake.account_info = lambda: reads.append(1) or self.fake.account
        for equity in [200., 250., 180., 230.]:
            self.fake.account.equity = equity
            row = self.sidecar.cmd_account({})
            self.assertEqual(row["equity"], equity)
            self.assertEqual(row["observation_broker_day"], 100)
        self.assertEqual(len(reads), 4)

    def test_midnight_read_clock_jump_and_other_account_are_unqualified(self):
        self.stamp = 101 * 86400000 - 2000
        self.established()  # one second before midnight
        self.assertEqual(self.sidecar.cmd_account({})["observation_broker_day"], 100)
        self.mono += 2; self.utc += 2
        self.assertIsNone(self.sidecar.cmd_account({})["observation_broker_day"])
        self.advance()
        self.assertEqual(self.sidecar.cmd_account({})["observation_broker_day"], 101)
        self.utc += 3600
        self.assertIsNone(self.sidecar.cmd_account({})["observation_broker_day"])
        self.utc -= 3600
        self.fake.account.server = "other-fixture"
        self.assertIsNone(self.sidecar.cmd_account({})["observation_broker_day"])


if __name__ == "__main__":
    unittest.main()
