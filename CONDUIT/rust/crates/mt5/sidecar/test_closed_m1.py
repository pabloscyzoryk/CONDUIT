"""Real helper and sidecar callbacks against FakeMT5 only; no terminal/auth."""
import copy
import math
import unittest
from types import SimpleNamespace as NS
from unittest.mock import patch

from closed_m1 import ClosedM1, MAX_CLOSED_BARS
from test_follow_terminal import FakeMT5, mod


BASE = 1_800_000_000_000  # minute-aligned synthetic broker clock


def bar(minute, **changes):
    row = dict(time=(BASE + minute * 60_000) // 1000,
               open=4000., high=4005., low=3995., close=4001.,
               tick_volume=999, spread=47, real_volume=0)
    row.update(changes)
    return row


class ClosedM1Tests(unittest.TestCase):
    def setUp(self):
        self.api = FakeMT5()
        self.api.TIMEFRAME_M1 = 1
        self.api.SYMBOL_CHART_MODE_BID = 0
        self.api.symbols["XAUUSD"].chart_mode = 0
        self.rows = [bar(0), bar(1)]
        self.tick = NS(time_msc=BASE + 60_000, bid=4000., ask=4000.2)
        self.calls = []
        self.after_query = None
        self.api.copy_rates_from_pos = self.query
        self.api.symbol_info_tick = lambda symbol: self.tick
        mod.mt5 = self.api
        self.s = mod.Sidecar(mod.parse_args(["--port", "1"]))
        self.events = []
        self.s.send = self.send
        self.mono = 100.
        self.utc = (BASE - 10_800_000) / 1000
        self.clock = patch("closed_m1.time.monotonic", side_effect=lambda: self.mono)
        self.wall = patch("closed_m1.time.time", side_effect=lambda: self.utc)
        self.clock.start()
        self.wall.start()
        self.addCleanup(self.clock.stop)
        self.addCleanup(self.wall.stop)

    def tearDown(self):
        self.assertEqual(self.api.initializations, [])
        self.assertEqual(self.api.orders_sent, [])

    def send(self, obj):
        self.events.append(copy.deepcopy(obj))
        return True

    def query(self, *args):
        self.calls.append(args)
        result = copy.deepcopy(self.rows)
        if self.after_query:
            self.after_query()
        return result

    def enable(self):
        self.assertEqual(self.s.cmd_t100_bars({"enabled": True}), {"enabled": True, "schema": 1})
        self.assertEqual(self.events[-1]["error"], "initializing")
        self.assertFalse(self.events[-1]["complete"])
        self.assertEqual(self.calls, [])

    def latest(self):
        return [event for event in self.events if event.get("ev") == "m1_bars"][-1]

    def poll(self):
        self.s.poll_tick()
        return self.latest()

    def test_off_has_no_account_symbol_or_history_reads_and_legacy_tick_unchanged(self):
        self.api.account_info = lambda: self.fail("OFF account read")
        self.api.symbol_info = lambda symbol: self.fail("OFF symbol read")
        self.api.copy_rates_from_pos = lambda *args: self.fail("OFF history read")
        self.s.poll_tick()
        self.s.poll_tick()
        self.assertEqual(self.events, [{"ev": "tick", "ts": self.tick.time_msc,
                                       "bid": 4000., "ask": 4000.2}])

    def test_optin_complete_bid_and_raw_clock_no_current_future_ohlc(self):
        self.rows[-1]["high"] = 9000.
        self.enable()
        event = self.poll()
        self.assertEqual(self.calls, [("XAUUSD", 1, 0, 513)])
        self.assertTrue(event["complete"])
        self.assertEqual([row["ts"] for row in event["bars"]], [BASE])
        self.assertEqual(event["bars"][0]["high"], 4005.)
        self.assertEqual(event["bars"][0]["max_spread"], 0)
        self.assertEqual(event["bars"][0]["observations"], 0)
        self.assertEqual(event["available_at_ms"], BASE + 60_000)
        self.assertEqual(event["observed_utc_ms"], BASE - 10_800_000)
        self.assertEqual([row["ev"] for row in self.events[-2:]], ["m1_bars", "tick"])
        self.assertNotIn("password", event)

    def test_query_completion_does_not_expand_precaptured_availability(self):
        self.enable()
        self.after_query = lambda: (setattr(self.tick, "time_msc", BASE + 120_000),
                                    setattr(self, "utc", self.utc + 2))
        event = self.poll()
        self.assertEqual(event["available_at_ms"], BASE + 60_000)
        self.assertEqual(event["observed_utc_ms"], BASE - 10_800_000 + 2000)
        self.assertEqual([row["ts"] for row in event["bars"]], [BASE])

    def test_same_minute_no_duplicate_and_next_minute_suffix_only(self):
        self.enable()
        self.poll()
        self.s.cmd_t100_bars({"enabled": True})
        self.poll()
        self.assertEqual(len(self.calls), 1)
        self.rows = [bar(0), bar(1), bar(2)]
        self.tick.time_msc += 60_000
        event = self.poll()
        self.assertEqual([row["ts"] for row in event["bars"]], [BASE + 60_000])

    def test_none_retries_after_five_seconds_even_same_quote_then_catches_up(self):
        self.enable()
        self.rows = None
        self.assertEqual(self.poll()["error"], "history_unavailable")
        self.mono += 4.9
        self.poll()
        self.assertEqual(len(self.calls), 1)
        self.rows = [bar(0), bar(1)]
        self.mono += .1
        self.assertTrue(self.poll()["complete"])
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(self.s._closed_m1.cursor, BASE)

    def test_empty_or_lagged_history_retry_without_synthetic_minutes(self):
        self.enable()
        self.rows = []
        event = self.poll()
        self.assertTrue(event["complete"])
        self.assertEqual(event["bars"], [])
        self.rows = [bar(0), bar(1)]
        self.mono += 5
        self.assertEqual(len(self.poll()["bars"]), 1)

    def test_real_market_gap_preserved_not_filled(self):
        self.enable()
        self.poll()
        self.tick.time_msc = BASE + 61 * 60_000
        self.rows = [bar(0), bar(60), bar(61)]
        event = self.poll()
        self.assertTrue(event["complete"])
        self.assertEqual([row["ts"] for row in event["bars"]], [BASE + 60 * 60_000])
        self.assertFalse(event["catchup_truncated"])

    def test_account_change_during_query_rejects_all_rows_and_resets(self):
        self.enable()
        self.after_query = lambda: setattr(self.api.account, "server", "other-fixture")
        event = self.poll()
        self.assertEqual(event["error"], "account_changed_during_query")
        self.assertEqual(event["bars"], [])
        self.assertIsNone(self.s._closed_m1.cursor)
        self.after_query = None
        self.mono += 5
        self.assertTrue(self.poll()["complete"])

    def test_scope_change_between_queries_blocks_once_and_requalifies_chart(self):
        self.enable()
        self.poll()
        self.api.account.server = "other-fixture"
        self.api.symbols["XAUUSD"].chart_mode = 1
        self.tick.time_msc += 60_000
        self.assertEqual(self.poll()["error"], "account_or_symbol_changed")
        self.mono += 5
        self.assertEqual(self.poll()["error"], "unsupported_non_bid_chart")
        self.assertEqual(len(self.calls), 1)

    def test_missing_or_last_chart_mode_is_not_bid(self):
        for mode in (None, 1, True):
            with self.subTest(mode=mode):
                self.s._closed_m1 = ClosedM1()
                self.api.symbols["XAUUSD"].chart_mode = mode
                self.enable()
                self.assertEqual(self.poll()["error"], "unsupported_non_bid_chart")
                self.assertEqual(self.calls, [])

    def test_bid_mode_documented_zero_fallback(self):
        del self.api.SYMBOL_CHART_MODE_BID
        self.enable()
        self.assertTrue(self.poll()["complete"])

    def test_backward_quote_and_restart_discard_cursor(self):
        self.enable()
        self.poll()
        self.tick.time_msc -= 1
        self.assertEqual(self.poll()["error"], "quote_clock_reversed")
        self.assertIsNone(self.s._closed_m1.cursor)
        self.s._reset_quote_clock()
        self.assertEqual(self.poll()["error"], "session_reset")
        self.assertIsNone(self.s._closed_m1.cursor)

    def test_disable_reenable_resets_without_history_on_disable(self):
        self.enable()
        self.poll()
        self.s.cmd_t100_bars({"enabled": False})
        self.poll()
        self.assertEqual(len(self.calls), 1)
        self.assertIsNone(self.s._closed_m1.cursor)
        self.s.cmd_t100_bars({"enabled": True})
        self.assertEqual(len(self.poll()["bars"]), 1)

    def test_invalid_enabled_and_missing_account_do_not_enable(self):
        for value in (None, 1, "true", []):
            with self.subTest(value=value), self.assertRaises(mod.BrokerError):
                self.s.cmd_t100_bars({"enabled": value})
            self.assertFalse(self.s._closed_m1.enabled)
        self.api.account = None
        with self.assertRaises(mod.BrokerError):
            self.s.cmd_t100_bars({"enabled": True})
        self.assertFalse(self.s._closed_m1.enabled)

    def test_bad_ohlc_order_nan_time_duplicate_and_future_reject_whole_batch(self):
        cases = [[bar(0), bar(0)], [bar(1), bar(0)], [bar(0, high=math.nan)],
                 [bar(0, low=4002.)], [bar(0, time=1.1)], [bar(0, time=-60)],
                 [bar(0), bar(2)], [dict(time=BASE // 1000)], [bar(0, close=0.)]]
        for rows in cases:
            with self.subTest(rows_type=len(rows)):
                self.s._closed_m1 = ClosedM1()
                self.calls.clear()
                self.rows = rows
                self.enable()
                event = self.poll()
                self.assertFalse(event["complete"])
                self.assertEqual(event["bars"], [])
                self.assertIsNone(self.s._closed_m1.cursor)

    def test_closed_window_is_bounded_and_overflow_explicit(self):
        self.enable()
        self.poll()
        self.rows = [bar(i) for i in range(10, 523)]
        self.tick.time_msc = BASE + 522 * 60_000
        event = self.poll()
        self.assertEqual(len(event["bars"]), MAX_CLOSED_BARS)
        self.assertTrue(event["catchup_truncated"])
        self.rows = [bar(i) for i in range(514)]
        self.tick.time_msc += 60_000
        event = self.poll()
        self.assertEqual(event["error"], "history_response_overflow")
        self.assertEqual(event["bars"], [])

    def test_socket_failure_does_not_advance_delivery_cursor(self):
        self.enable()
        self.s.send = lambda obj: False
        self.s.poll_tick()
        self.assertIsNone(self.s._closed_m1.cursor)
        self.s.send = self.send
        self.mono += 5
        self.assertEqual([row["ts"] for row in self.poll()["bars"]], [BASE])

    def test_sdk_exception_is_sanitized_and_no_rows_leak(self):
        self.enable()
        def failed(*args):
            raise RuntimeError("SYNTHETIC_PRIVATE_ARGUMENT_DO_NOT_ECHO")
        self.api.copy_rates_from_pos = failed
        event = self.poll()
        self.assertEqual(event["error"], "history_query_failed")
        self.assertNotIn("SYNTHETIC_PRIVATE_ARGUMENT", str(event))

    def test_missing_quote_has_no_history_query_and_slow_failure_retry_after_callback(self):
        self.enable()
        event = self.s._closed_m1.poll(self.api, "XAUUSD", None, mod.account_key)
        self.assertEqual(event["error"], "quote_unavailable")
        self.assertEqual(self.calls, [])
        self.rows = None
        self.after_query = lambda: setattr(self, "mono", self.mono + 8)
        self.poll()
        before = len(self.calls)
        self.mono += 4
        self.poll()
        self.assertEqual(len(self.calls), before)
        self.after_query = None
        self.mono += 1
        self.poll()
        self.assertEqual(len(self.calls), before + 1)


if __name__ == "__main__":
    unittest.main()
