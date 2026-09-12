"""Opt-in, read-only, bounded closed BID M1 delivery for T-100.

SDK contract: https://www.mql5.com/en/docs/python_metatrader5/mt5copyratesfrompos_py
Index zero is current. Raw rates.time and quote.time_msc stay on the same
terminal clock; this module never guesses a UTC offset. Availability requires
bar end <= the quote captured BEFORE the history call. observed_utc_ms is the
local observation AFTER that call, not a rewritten market timestamp.

Only available terminal history is observable (the SDK's chart history limit
still applies). Missing minutes are not synthesized. Consumers reset their
warmup across gaps. rates.spread/tick_volume do not prove maximum spread or
complete quote observation: the unused policy metadata is explicitly zero.
"""
import math
import time


MINUTE_MS = 60_000
MAX_CLOSED_BARS = 512
RETRY_SECONDS = 5.0
MAX_TS = (1 << 63) - 1 - MINUTE_MS


class ClosedM1Error(Exception):
    """Fixed, credential-free diagnostic codes only."""


class ClosedM1:
    def __init__(self):
        self.enabled = False
        self.reset()

    def reset(self, error=None):
        self.cursor = None
        self.scope = None
        self.last_minute = None
        self.last_quote = None
        self.next_retry = 0.0
        self.failed = True
        self.bid_qualified = False
        self.pending_error = error
        self.latest_returned = None

    def configure(self, enabled):
        if type(enabled) is not bool:
            raise ClosedM1Error("invalid_enabled")
        changed = self.enabled != enabled
        if changed:
            self.enabled = enabled
            self.reset()
        return changed

    def packet(self, symbol, account, available, *, complete=False, error=None, bars=None,
               truncated=False):
        return {"ev": "m1_bars", "schema": 1, "symbol": symbol, "account": account,
                "observed_utc_ms": int(time.time() * 1000),
                "available_at_ms": available, "complete": complete, "error": error,
                "bars": bars or [], "catchup_truncated": truncated}

    def _failure(self, symbol, account, available, code, now):
        self.failed = True
        self.next_retry = max(now, time.monotonic()) + RETRY_SECONDS
        return self.packet(symbol, account, available, error=code)

    def poll(self, api, symbol, tick, account_key):
        """Return at most one packet; caller commits only after successful send.

        No native method, including account/symbol/history, is called while OFF.
        At most one bounded history call per new quote minute or retry interval.
        Native SDK blocking time itself has no documented hard upper bound.
        """
        if not self.enabled:
            return None
        now = time.monotonic()
        try:
            raw_stamp = getattr(tick, "time_msc", 0)
            available = int(raw_stamp)
            if isinstance(raw_stamp, bool) or available != raw_stamp or not 0 < available <= MAX_TS:
                raise ValueError()
        except (TypeError, ValueError, OverflowError):
            if now < self.next_retry:
                return None
            account = self.scope[1] if self.scope else None
            return self._failure(symbol, account, 0, "quote_unavailable", now)

        minute = available // MINUTE_MS
        if self.last_quote is not None and available < self.last_quote:
            old_account = self.scope[1] if self.scope else None
            self.reset()
            self.last_quote = available
            self.last_minute = minute
            return self._failure(symbol, old_account, available, "quote_clock_reversed", now)
        self.last_quote = available
        if minute == self.last_minute and (not self.failed or now < self.next_retry):
            return None
        self.last_minute = minute
        self.failed = True
        self.next_retry = now + RETRY_SECONDS
        account = None
        try:
            account = account_key(api.account_info())
            scope = (symbol, account)
            if self.scope is not None and scope != self.scope:
                self.reset()
                self.scope = scope
                self.last_minute = minute
                self.last_quote = available
                return self._failure(symbol, account, available, "account_or_symbol_changed", now)
            self.scope = scope
            if self.pending_error is not None:
                error, self.pending_error = self.pending_error, None
                return self._failure(symbol, account, available, error, now)
            if not self.bid_qualified:
                info = api.symbol_info(symbol)
                mode = getattr(info, "chart_mode", None)
                bid_mode = getattr(api, "SYMBOL_CHART_MODE_BID", 0)
                if mode is None or isinstance(mode, bool) or mode != bid_mode:
                    raise ClosedM1Error("unsupported_non_bid_chart")
                self.bid_qualified = True
            # Read index zero as well, then exclude it by its actual bar-end
            # timestamp. A slow call cannot smuggle the new current bar's OHLC.
            rates = api.copy_rates_from_pos(symbol, api.TIMEFRAME_M1, 0, MAX_CLOSED_BARS + 1)
            after = account_key(api.account_info())
            if after != account:
                self.reset()
                self.scope = (symbol, after)
                self.last_minute = minute
                self.last_quote = available
                return self._failure(symbol, after, available, "account_changed_during_query", now)
            if rates is None:
                raise ClosedM1Error("history_unavailable")
            bars = self._decode(rates, available)
            self.latest_returned = bars[-1]["ts"] if bars else None
            truncated = False
            if self.cursor is not None:
                # Full returned window without the previous cursor cannot
                # certify intervening history. Mark the bounded recovery.
                truncated = (len(bars) >= MAX_CLOSED_BARS and bars[0]["ts"] > self.cursor + MINUTE_MS)
                bars = [bar for bar in bars if bar["ts"] > self.cursor]
            if len(bars) > MAX_CLOSED_BARS:
                truncated = True
                bars = bars[-MAX_CLOSED_BARS:]
            self.next_retry = time.monotonic() + RETRY_SECONDS
            return self.packet(symbol, account, available, complete=True, bars=bars,
                               truncated=truncated)
        except ClosedM1Error as error:
            return self._failure(symbol, account, available, str(error), now)
        except Exception:
            # SDK exceptions may contain account/path/argument reprs.
            return self._failure(symbol, account, available, "history_query_failed", now)

    @staticmethod
    def _decode(rates, available):
        if len(rates) > MAX_CLOSED_BARS + 1:
            raise ClosedM1Error("history_response_overflow")
        bars = []
        previous = None
        current_minute = available // MINUTE_MS * MINUTE_MS
        for row in rates:
            try:
                raw_time = row["time"]
                seconds = int(raw_time)
                ts = seconds * 1000
                values = [float(row[key]) for key in ("open", "high", "low", "close")]
                if isinstance(raw_time, bool) or seconds != raw_time or not 0 <= ts <= MAX_TS:
                    raise ValueError()
                if ts % MINUTE_MS or ts > current_minute or (previous is not None and ts <= previous):
                    raise ValueError()
                if not all(math.isfinite(value) and value > 0 for value in values):
                    raise ValueError()
                op, high, low, close = values
                if high < max(op, close) or low > min(op, close) or high < low:
                    raise ValueError()
            except (KeyError, TypeError, ValueError, OverflowError, IndexError):
                raise ClosedM1Error("invalid_history_rows") from None
            previous = ts
            if ts + MINUTE_MS <= available:
                bars.append({"ts": ts, "open": op, "high": high, "low": low, "close": close,
                             "max_spread": 0.0, "observations": 0})
        return bars

    def delivered(self, packet):
        if not packet["complete"]:
            return
        if packet["bars"]:
            self.cursor = packet["bars"][-1]["ts"]
        # A cached/empty history success may be legitimate at reopening, but
        # also may lag the first quote of a minute. Retry without inventing a
        # missing bar or emitting the same candle twice.
        expected = packet["available_at_ms"] // MINUTE_MS * MINUTE_MS - MINUTE_MS
        self.failed = self.latest_returned is None or self.latest_returned < expected
        self.next_retry = time.monotonic() + RETRY_SECONDS if self.failed else 0.0
