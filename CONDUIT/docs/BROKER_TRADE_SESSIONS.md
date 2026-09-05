# Broker execution sessions

`btp --sim-trade-sessions profile.json` selects an explicit execution model,
independent of strategy settings and sweep axes. The default is absent: the
historical simulator path and its manifests remain unchanged. The same option
on `most_eaF22audit` validates the profile and writes its canonical hash in the
bridge header; it never filters or delays source messages.

The JSON schema is `conduit.trade-sessions.v1`, with `clock: "broker"`, seven
explicit `days` (`day_sun0` 0–6), each containing `trade` and `quote` arrays of
`[from_seconds, to_seconds]`, and `take_profit_price_improvement: true|false`.
Intervals include their start and exclude their end. End values can be 86400
or extend into the next weekday; they must not be reduced modulo 24 hours.
Missing weekdays, ambiguous overlaps, unknown fields and invalid intervals
fail validation. The simulator does not convert these seconds to UTC.

This follows the official [SymbolInfoSessionTrade contract](https://www.mql5.com/en/docs/marketinformation/symbolinfosessiontrade).
The [MetaQuotes book](https://www.mql5.com/en/book/automation/symbols/symbols_sessions)
also describes sessions whose end reaches or exceeds 24 hours. Profile times
come from the tested symbol, not from a strategy's preferred trading hours.

During a closed trade session the quote still updates mark-to-market, swap,
statistics and the Engine. Physical entry, close, cancellation and modification
requests fail with `BrokerError::MarketClosed` (native return code 10018), after
existing applicable price/stop validation. Orders and positions remain intact.
Pending fills and server SL/TP await the first tradable observation. This is
supported by a native real-tick fixture that placed four crossed exposures
before an overnight break: no fill or SL/TP occurred on 291 quote-only ticks;
all four executed on the first tradable observation. A valid SL modification
returned 10018; an invalid one returned 10016 before that check.

The same fixture observed an improved native TP exit after the gap. The explicit
`take_profit_price_improvement` switch reproduces that price rule; false keeps
the old fixed-target exit price. This observation is a qualified broker/tester
profile, not a universal assertion about every broker's execution policy.

`CONDUIT_SESSION_PROBE.mq5` is a tester-only, synthetic-order probe. It refuses
non-tester execution and non-hedging accounts. Its inputs describe the chosen
real-tick boundary; fixtures and reports belong in private experiment folders.
`CONDUIT_XT` emits actual symbol sessions at initialization, and the experiment
records source/binary/profile hashes. No broker login or credentials belong in
the profile.

Known remaining scope: this session gate does not change the simulator's
historical ordering of older stops versus pending fills on the same tradable
tick. Native margin stress must verify that ordering before a larger-cap
comparison can claim complete execution parity. Broker holiday overrides,
changing historical specifications and abrupt live disconnects need separate
evidence; a weekly snapshot does not reconstruct those events.
