"""Observed candle metadata through the real sidecar, entirely offline."""
import unittest
from types import SimpleNamespace as NS
from unittest.mock import patch

from test_follow_terminal import FakeMT5, mod


class CandleClockTests(unittest.TestCase):
    def setUp(self):
        self.fake = FakeMT5()
        self.expected_fake_initializations = 0
        mod.mt5 = self.fake
        self.fake.symbols["XAUUSD"].name = "XAUUSD"
        self.fake.symbols["XAUUSD.s"] = self.fake.si()
        self.fake.symbols["XAUUSD.s"].name = "XAUUSD.s"
        self.mono = 100.0
        self.utc = 1_800_000_000.0
        self.tick = NS(time_msc=int((self.utc + 10800) * 1000), bid=100., ask=100.2)
        self.fake.symbol_info_tick = lambda symbol: self.tick
        self.bars = [dict(time=self.tick.time_msc // 1000 // 60 * 60,
                          open=100., high=101., low=99., close=100., tick_volume=3, spread=20)]
        self.fake.copy_rates_from_pos = lambda *args: self.bars
        self.fake.copy_rates_from = lambda *args: self.bars
        self.clock = patch.object(mod.time, "monotonic", side_effect=lambda: self.mono)
        self.wall = patch.object(mod.time, "time", side_effect=lambda: self.utc)
        self.clock.start()
        self.wall.start()
        self.addCleanup(self.clock.stop)
        self.addCleanup(self.wall.stop)
        self.sidecar = mod.Sidecar(mod.parse_args(["--port", "1", "--symbol", "XAUUSD"]))

    def tearDown(self):
        self.assertEqual(len(self.fake.initializations), self.expected_fake_initializations)
        self.assertEqual(self.fake.orders_sent, [], "no trading permitted")

    def candles(self, symbol="XAUUSD"):
        return self.sidecar.cmd_candles({"symbol": symbol, "tf": "M1", "count": 1})

    def advance(self, seconds=1):
        self.mono += seconds
        self.utc += seconds
        self.tick = NS(time_msc=self.tick.time_msc + int(seconds * 1000), bid=100.1, ask=100.3)

    def established(self):
        self.candles()
        self.advance()
        return self.candles()

    def test_first_cached_quote_preserves_raw_data_but_has_no_clock_proof(self):
        self.utc += 86400  # Friday quote seen on Saturday after a cold start.
        c = self.candles()
        self.assertEqual(c["server_time_ms"], self.tick.time_msc)
        self.assertIsNone(c["quote_observed_utc_ms"])
        self.assertIsNone(c["quote_observation_age_ms"])
        self.assertEqual(c["bars"][0][0], self.bars[0]["time"] * 1000)

    def test_real_advance_pairs_quote_with_utc_then_same_tick_ages(self):
        c = self.established()
        paired = c["quote_observed_utc_ms"]
        self.assertEqual(c["server_time_ms"] - paired, 10800000)
        self.assertEqual(c["quote_observation_age_ms"], 0)
        self.utc += 40
        self.mono += 40
        c = self.candles()
        self.assertEqual(c["quote_observed_utc_ms"], paired)
        self.assertEqual(c["quote_observation_age_ms"], 40000)
        self.assertEqual(c["server_time_ms"] - paired, 10800000)

    def test_price_only_change_is_not_new_time_and_missing_or_backward_tick_resets(self):
        first = self.established()
        self.mono += 2
        self.utc += 2
        self.tick.bid += .01
        self.assertEqual(self.candles()["quote_observation_age_ms"], 2000)
        self.assertEqual(self.candles()["quote_observed_utc_ms"], first["quote_observed_utc_ms"])
        self.tick.time_msc -= 1000
        self.assertIsNone(self.candles()["quote_observed_utc_ms"])
        self.advance(2)
        self.assertIsNotNone(self.candles()["quote_observed_utc_ms"])
        self.tick = None
        self.assertIsNone(self.candles()["quote_observed_utc_ms"])

    def test_restart_and_reconnect_reset_require_two_observations(self):
        self.established()
        self.sidecar._reset_quote_clock()  # Same production reset used before initialize.
        self.assertIsNone(self.candles()["quote_observed_utc_ms"])
        self.advance()
        self.assertIsNotNone(self.candles()["quote_observed_utc_ms"])
        self.sidecar = mod.Sidecar(mod.parse_args(["--port", "1"]))
        self.assertIsNone(self.candles()["quote_observed_utc_ms"])

    def test_actual_reinitialize_boundary_resets_clock_with_fake_terminal_only(self):
        self.established()
        # mt5 is the injected FakeMT5 object; no actual terminal API is imported.
        self.expected_fake_initializations = 1
        self.sidecar.init_terminal()
        self.assertIsNone(self.candles()["quote_observed_utc_ms"])
        self.advance()
        self.assertIsNotNone(self.candles()["quote_observed_utc_ms"])

    def test_account_and_symbol_switch_do_not_reuse_previous_clock(self):
        self.established()
        self.fake.account.server = "Synthetic-Other"
        self.advance()
        self.assertIsNone(self.candles()["quote_observed_utc_ms"])
        self.advance()
        self.assertIsNotNone(self.candles()["quote_observed_utc_ms"])
        self.sidecar.cmd_subscribe_ticks({"symbol": "XAUUSD.s"})
        self.assertIsNone(self.candles("XAUUSD.s")["quote_observed_utc_ms"])
        self.advance()
        self.assertIsNotNone(self.candles("XAUUSD.s")["quote_observed_utc_ms"])
        self.assertIsNone(self.candles("XAUUSD")["quote_observed_utc_ms"])

    def test_poll_observation_is_shared_without_changing_tick_stream(self):
        self.candles()  # Establish account context without advancing the quote.
        sent = []
        self.sidecar.send = sent.append
        self.sidecar.poll_tick()
        self.advance()
        self.sidecar.poll_tick()
        c = self.candles()
        self.assertEqual(c["quote_observed_utc_ms"], int(self.utc * 1000))
        self.assertEqual(len(sent), 2)
        self.assertEqual(sent[-1], {"ev": "tick", "ts": self.tick.time_msc,
                                    "bid": self.tick.bid, "ask": self.tick.ask})


if __name__ == "__main__":
    unittest.main()
