# Local order sequence at the live boundary

The MT5 adapter records a local submission ordinal within each basket. Its
counter increases monotonically during one adapter instance. New market and pending requests use the compact comment
`<TAG><basket>.<level>[t]~<base36 ordinal>![-note]`. The existing basket, level and
toucher fields stay first. The complete machine section must fit the existing
29-character budget. The optional note may be shortened. A long custom tag or
extreme identifiers may leave no room for a complete ordinal; the adapter then
uses the legacy comment and emits an explicit diagnostic.

The closing `!` is mandatory when reading an ordinal. A broker truncating
`~12!` to `~1` must never change ordinal 38 into ordinal 1. Invalid, zero,
overflowed or duplicate ordinals are not proof of ordering. They never become
an arbitrary technical ticket tie-break.

For a newly discovered batch of owned positions in one basket, the primary
key is actual positive MT5 fill time. A complete unique local ordinal orders
positions only when their fill times are equal. Thus a genuinely earlier fill
stays before a later fill even if its order was submitted later. The same rule
uses placement time for pending snapshots. Other baskets retain their slots in
the incoming snapshot, and previously known positions keep their remembered
order. No previous trading decision is replayed or rolled back when a missing
fill is discovered late.

Startup/reconcile reconstructs this ordering and the next local ordinal from
complete comments on currently open positions and pending orders. The next value
is greater than every currently known active ordinal in that basket. Closed
history is not retained by this counter; this is an active-order rank, not a
globally unique historical event ID. Failed sends
may leave gaps. Retries of one wire request retain its original comment.
Counter overflow falls back explicitly; it does not wrap to a reused ordinal.

During the same running adapter session, a later clipped comment may retain
an already proven ordinal from the exact cached object identity, provided its
basket, level and toucher identity still agree. Ticket equality can identify
that cached object; ticket magnitude never determines its rank. A cold start
with clipped or legacy comments has no such proof. For an equal-time cohort
without a complete unique ordinal, input order is retained and the adapter
marks `local_order_sequence_incomplete`. Protective management remains
available; this is not a fabricated broker receipt fault or a new strategy gate.

The simulator and strategy code are unchanged. The simulator already appends
fills in actual processing time and local pending/request order. This adapter
rule removes a dependence on the first `positions_get()` enumeration for new
sequenced orders. The MT5 documentation describes returned position records,
but does not promise the local bot's request order; no such promise is assumed.
See [MetaQuotes positions_get documentation](https://www.mql5.com/en/docs/python_metatrader5/mt5positionsget_py).

Synthetic tests exercise the real Transport + Bridge + Engine over loopback:
permuted equal-time fills preserve the same logical RiskFree runner even when
its ticket is numerically larger; differing fill times precede ordinal rank;
fresh reconciliation restores the next market/pending ordinal; warm clipping
preserves proven metadata; and cold legacy/clipped cohorts disclose their limit.
No MT5 process, real account or Telegram service is started by these tests.
