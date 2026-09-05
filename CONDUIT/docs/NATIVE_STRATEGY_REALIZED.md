# Confirmed strategy profit and broker cash

The native tester supports `closed_profit_net_costs=false`. Its basket strategy
profit uses confirmed entry price, exit price, direction and closed volume:

`(exit - entry) * side_sign * 100 * closed_volume + allocated_swap`

This matches the simulator's legacy `PricePlusSwap` basis. Commission and fees
remain in the actual broker accounting and exported deal ledger. Neither the
broker balance nor a recorded deal is rewritten. Canonical net mode remains
outside the native preset mapping and is rejected when active; the shared
calculation helper preserves an explicitly supplied net result without deriving
it again.

Each position's cumulative strategy result is reconciled against its last booked
amount. Partial closes therefore contribute once while the residual position
remains open. Later reconciliation, basket compaction and committed edit rollback
preserve the cumulative receipt book. Missing ownership or confirmed geometry
blocks new risk; incomplete registered receipts remain blocked until all such
receipts for that basket are reconciled. Unknown ownership requires review.
Existing position management remains available. The tester requires a USD
account, USD profit currency and contract size 100; other contracts fail at init.

The strategy consumers share `Basket.realized`: rearm profitability, risk-free
eligibility and basket profit calculations. Floating strategy profit remains the
unrounded price-derived value. Account/equity reporting continues to use actual
MT5 cash and position cost fields.

Native scenario 21 demonstrates why the distinction matters at a zero-profit
boundary. Confirmed prices produce approximately +9.09e-13 after adding a retained
position; mixing rounded cash receipts with that floating value produces
approximately -1.46e-13. The strategy now uses one consistent basis. No epsilon,
time tolerance or modified comparator hides the difference. The same scenario
checks allocated swap, partial volume, invalid geometry and preserving a supplied
net result; scenario 6 verifies real partial/final broker reconciliation.

CONDUIT_XT remains a Strategy Tester instrument. A new terminal test reconstructs
state from its explicit replay; the EA does not claim a live-account restart
recovery feature from an arbitrary terminal snapshot.
