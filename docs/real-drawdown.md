# Real Drawdown (RDD)

RDD measures the largest observed loss below the day's starting equity:

```
RDD amount = max(0, starting equity − minimum observed equity)
RDD percent = 100 × RDD amount / starting equity
```

The percentage is defined only for positive starting equity. With a zero or
negative starting value, a known monetary loss can still be displayed while the
percentage remains unavailable. Negative current equity can produce RDD above
100% when the starting value was positive.

For `200 → 250 → 180 → 230`, RDD is **20 (10%)**. A later recovery does not erase
that day's loss. Peak-to-trough drawdown uses a different reference point and
remains a separate statistic.

The Drawdown card includes a small RDD percentage. Hovering over it or focusing
it with the keyboard explains the calculation and shows the account-currency
amount and percentage. Both Polish and English are supported.

## Live observations

The tracker consumes confirmed account-equity reads independently of the panel's
rendering frequency. It does not invent account valuations from prices, interpolate
between observations, or request additional trading operations.

Daily observations belong to the broker's clock and the connected account. The
sidecar qualifies the existing account response only when its already-observed
advancing quote clock is recent, agrees with the account and stays within one
broker day during the read. Unknown or stale clock evidence does not become a
known day through a configured timezone fallback.

The starting value is the first qualified equity observation of a day whose
transition the running tracker observed. It is not a claim to know equity at an
unobserved midnight. Starting the program in the middle of a day cannot reconstruct
that day's missing starting value or minimum; the card shows unavailable data.
Older backups without RDD also remain unavailable rather than acquiring a false
zero. A same-day backup containing a qualified starting value and minimum retains
them on recovery. An account change clears the previous account's observations.
Resuming after a risk guard does not reset RDD.

RDD is an observation-based statistic. It cannot reveal an equity trough while
the terminal was unreachable or between account samples. It is not a separate
trading rule and does not change a preset's risk limits or position management.

## Backtests and old results

New daily results carry their observed minimum equity, RDD amount and RDD
percentage. Backtests collect observations during simulation rather than
reconstructing a minimum from the reduced number of chart points. When separate
credit is enabled, the reporting equity basis applies consistently to starting
equity, minimum equity and the percentage denominator.

Legacy results without these fields have unknown RDD. Their published chart
cannot supply an exact missing minimum. Likewise, legacy global drawdown derived
from a reduced chart may omit intermediate peaks and troughs. New full-observation
drawdown may therefore differ even when every trade, profit and published curve
is unchanged. Compare the metric's observation basis as well as its value.

Daily consistency research should report RDD alongside peak drawdown, profit,
positive and negative equity days, signal use and filled baskets. Reducing RDD by
avoiding most entries is a material change in activity, not sufficient evidence
that a strategy improved.
