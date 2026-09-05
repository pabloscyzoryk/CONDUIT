# Strategy result and broker cash

GOD-X7 uses `basket_realized_broker_only=true` and the legacy
`closed_profit_net_costs=false` strategy. In this mode its management result is
confirmed price movement plus allocated swap:

`(exit_price - entry_price) * side_sign * 100 * closed_volume + allocated_swap`

This management result is used for basket protection, rearming, closed-result
observations and the strategy's realised-day limits. It is computed only after a
confirmed close. The confirmed closed tranche supplies the volume; a requested
partial-close volume is not a substitute.

Broker balance, equity, deal profit, commission, swap, history and validated net
exports remain broker accounting. Commission is excluded from this legacy
strategy formula, just as in its historical simulator; it is not removed from
actual net results. The UI identifies the strategy summary separately from
account results. Journal market snapshots mark the strategy-result field's basis.

The existing simulator's `PricePlusSwap` result is used bit for bit. Live
`PriceOnlyGross` receipts supply verified geometry for the formula above. This
avoids using broker cent rounding as a different input to a strategy threshold;
no numerical tolerance or epsilon is added to trading decisions.

The live calculation requires the verified XAUUSD or XAUUSD.s instrument, a USD
account and a 100-unit contract. Unknown geometry, allocation, identity or this
account contract blocks new exposure. Protective close, SL/TP and cancellation
remain available. Source-defined historical records are not declared to have a
verified live basis.

The separate canonical-net mode retains its existing validated receipt contract.
This change does not enable that mode or change its execution support.

Internal account-scoped recovery records carry the strategy-basis version and
exact result bits. Active carry and same-day nonzero strategy state from an older,
unqualified snapshot require review; the program does not silently relabel or
reset them. Empty state and irrelevant closed archives do not block a fresh start.
The last 50 closed-result observations are retained with their original ordering.
This record does not claim to be an atomic checkpoint of all broker activity.
