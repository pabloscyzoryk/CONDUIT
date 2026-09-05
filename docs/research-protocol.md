# Reproducible strategy research

Research results must identify the executable, source revision, complete settings,
input hashes, time convention, account assumptions and replay mode. A successful
process exit is insufficient: every requested configuration must appear in a
complete result file. Partial and approximate results cannot select a production
default.

## Data chronology

Telegram exports normally contain the last known text, rather than every previous
version of an edited message. Preserve the original publication-final replay as an
explicit historical benchmark when that is the reference being reproduced. Its
assumption is not proof that every final parameter was available at publication.
Keep actual observed NEW/EDIT receipts in a separate replay. A mixture with unknown
original versions is a missing-original stress test, not a complete live history.
Never silently replace one of these contracts with another to explain a profit
difference. Empty edits matter because they can withdraw a deferred entry. Do not
invent a pre-edit signal or describe an incomplete reconstruction as exact history.

A complete EDIT can be the first version observed by the bot. Handle it from that
observation time, without backdating an order. A later NEW must not duplicate the
basket or restore an older plan. Source validity and preset-controlled order
management are separate: source cancellation must be bound to the correct signal.

Quote import preserves physical row order, including equal timestamps. Forward
fill may use only a preceding valid quote. The broker clock and Telegram clock
must be aligned explicitly; publication and receipt timestamps use the same
conversion. Use the export's Unix timestamp rather than its localized display
string. Establish an offset from paired quote/receipt evidence, never by optimizing
profit. A summer clock observation does not establish a future winter offset.
Keep execution latency separate from timezone conversion, and retain a receipt for
price precision and execution assumptions.

## Selection

Search uses one common lot cap and broker profile. Broker costs, message IDs,
individual dates and future results are not strategy search axes. Include the
unchanged reference strategy and diversify candidate families before evaluating
their outcomes.

Calibrate approximate replay against exact replay on the same candidates and
window. Measure ranking changes, profit error, basket counts and positive-day
counts. Exact replay is required for shortlisted candidates regardless of the
screening result. Record every period used in selection; do not relabel it as an
untouched holdout later. A previously used period remains previously used when a
new strategy generation is started.

Count positive days against all observed market days as well as the legacy
closed-trade-day denominator. Report flat and negative days separately. Report
received entry signals, attempted baskets, filled baskets and signal utilization
alongside profit. Very low activity must not win solely through compounded profit.
Use actual accepted entry-source counts; the ratio of closed baskets to messages is
only a legacy proxy. Removing the best one, three or five days is allowed only for
the fixed 0.01-lot comparison, never for a higher-cap compounded account.

The exact confirmation and full-comparison queues retain the same reference. A
finalist queue may contain explicitly flagged diagnostic alternatives when fewer
than five configurations satisfy every requirement. It must not silently relabel
them as qualified or claim that the positive-day target was reached. Any later
validation used to choose this queue is selection data, not an untouched holdout.

## Independent account windows

Run the full period and each day, calendar week and calendar month with a fresh
engine, fresh account and specified starting deposit. Keep a causal market-data
warmup separate from trading history. Test each requested volume cap and deposit;
an unlimited arithmetic scenario is distinct from a broker's maximum volume.
The sum of profits from daily resets is not the terminal balance of one continuous
account. Label drawdown denominators explicitly.
If the tick history begins before the first available source message, retain those
leading flat days in the full-window account and show a separate source-start view.
That view may not hide interior missing captures, losing days or flat days.

## Native execution comparison

Compare identical quote sequences before attributing differences to execution.
Begin at the smallest broker volume, compare the first differing event, then
increase volume. Separate broker-reported costs from historical assumptions.
Order submission, acceptance and final execution are different events. Failed
protective requests must remain visible and be retried where appropriate.

A surprising behavior is eligible as a strategy feature only if it is causal,
specified, reproducible through the live broker contract and covered by a
regression test. A profitable simulator-only artifact is not a feature.

## Release gate

Keep live credentials, Telegram sessions, private messages, tick exports and
account logs outside public Git history and release archives. Audit the staged
source and reachable history before pushing. Candidate evaluation does not
authorize selecting a new default: the owner chooses after seeing the requested
validation matrix.
