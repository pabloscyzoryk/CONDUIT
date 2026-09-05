# Time in Telegram, the broker and the interface

CONDUIT uses two different time domains. A Telegram Unix timestamp represents an
actual instant in UTC. An MT5 tick export contains the broker's displayed clock;
its numeric representation is not automatically a UTC instant. The browser's
local timezone is a third presentation choice, not an execution offset.

## Live processing

The age of a newly received Telegram message is checked using its UTC publication
time and the UTC receipt time. The configured `signal_max_age_min` controls that
ingress check: zero disables it. This is separate from the selected preset's
management of an already received signal or its pending orders.

After ingress, the trading engine processes the message on the latest broker
quote's timeline. It does not add the historical replay offset again. A complete
EDIT may be the first version this installation observes. It is processed once
at observation time; an older NEW arriving later must not duplicate or revert it.

Market-data responses identify the time basis. A real UTC quote time is supplied
only when supported by fresh broker/receipt evidence. A cached timestamp alone
does not prove quote freshness. Restart, account and symbol changes invalidate
that evidence. The display must not turn an unknown quote age into zero seconds.

## Historical replay

1. Preserve the broker wall-clock timestamps and physical row order of ticks.
2. Read Telegram `date_unixtime` and the corresponding edit Unix timestamp when
   available. A localized export display string must not replace a Unix instant.
3. Establish the broker offset from paired, independently recorded broker quotes
   and UTC receipts. Check price identity and quote age, as well as timestamps.
4. Apply the declared message offset exactly once. Keep the command-line offset,
   preset offset and inherited server offset visible in the run's input receipt.
5. Apply execution latency separately. A 250 ms execution delay is not part of a
   timezone correction. Market-data warmup does not authorize earlier trades.

For the June–early September 2026 corpus audited in this project, the evidence
supports UTC+3 broker time. Telegram's human-readable export strings used UTC+2.
Adding another two hours to the Unix field would therefore be wrong. The audit
used matching broker quotes and original CSV rows, not a search for the offset
with the highest profit.

Do not extrapolate this summer observation across a broker clock change. A new
historical window needs its own clock evidence. Split a replay or provide the
correct timestamp mapping when the offset changes; one constant offset cannot
describe two different broker clock regimes. The current engine's management
durations follow broker time, so a discontinuity in that clock can also affect a
duration crossing the transition. There is no automatic future DST guarantee.

## Reporting

Broker wall-clock dates are formatted as that clock, without an additional
browser timezone conversion. UTC instants retain their actual instant semantics.
Historical chart labels use the timestamp's date rather than today's DST state.
A closed trade already timestamped in broker time must not receive another
server offset when assigned to a daily report.

An event's publication, receipt, processing and execution times are distinct.
Preserve them in diagnostics instead of silently replacing one with another.
Comparisons must declare their time domain, source contract and inclusive start /
exclusive end boundaries before their results can be interpreted.
