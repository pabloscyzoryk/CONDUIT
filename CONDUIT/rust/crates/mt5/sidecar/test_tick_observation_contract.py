"""Actual sidecar polling contracts, offline: never imports real MetaTrader5.

Passing legacy tests document limits of the observed stream, not full-tick
parity. A server-side SL/TP can still execute between client observations.
"""
import unittest
from types import SimpleNamespace as NS

from test_follow_terminal import FakeMT5, mod


class SnapshotObservationTests(unittest.TestCase):
    def setUp(self):
        self.fake = FakeMT5()
        mod.mt5 = self.fake
        self.latest = NS(time_msc=1000, bid=4438.80, ask=4439.00)
        self.fake.symbol_info_tick = lambda symbol: self.latest
        self.sidecar = mod.Sidecar(mod.parse_args(["--port", "1"]))
        self.observed = []
        self.sidecar.send = self.observed.append
        # Construction and poll do not initialize/login or place any order.

    def tearDown(self):
        self.assertEqual(self.fake.initializations, [])
        self.assertEqual(self.fake.orders_sent, [])

    def test_legacy_poll_only_sees_latest_quote_not_touch_between_polls(self):
        actual_market = [self.latest]
        self.sidecar.poll_tick()
        # Price touches a TP, then returns before the next client poll.
        self.latest = NS(time_msc=1005, bid=4439.20, ask=4439.40)
        actual_market.append(self.latest)
        self.latest = NS(time_msc=1010, bid=4438.80, ask=4439.00)
        actual_market.append(self.latest)
        self.sidecar.poll_tick()
        self.assertTrue(any(t.bid >= 4439.0 for t in actual_market))
        self.assertFalse(any(t["bid"] >= 4439.0 for t in self.observed))
        self.assertEqual([t["ts"] for t in self.observed], [1000, 1010])

    def test_identical_snapshot_is_deduplicated_but_same_millisecond_new_price_is_not(self):
        self.sidecar.poll_tick()
        self.sidecar.poll_tick()
        self.assertEqual(len(self.observed), 1)
        self.latest = NS(time_msc=1000, bid=4438.81, ask=4439.01)
        self.sidecar.poll_tick()
        self.assertEqual(len(self.observed), 2)
        self.assertEqual([t["ts"] for t in self.observed], [1000, 1000])

    def test_legacy_older_timestamp_is_forwarded_not_rejected(self):
        self.sidecar.poll_tick()
        self.latest = NS(time_msc=990, bid=4438.50, ask=4438.70)
        self.sidecar.poll_tick()
        self.assertEqual([t["ts"] for t in self.observed], [1000, 990])


if __name__ == "__main__":
    unittest.main()
