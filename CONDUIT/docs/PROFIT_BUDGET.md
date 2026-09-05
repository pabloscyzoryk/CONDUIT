# Daily profit budget (opt-in)

`profit_budget_arm_pct=0` disables the daily profit reserve. A separately configured `max_portfolio_risk_pct > 0` still constrains every new order. When both controls are disabled (portfolio <= 0 and arm = 0), sizing passes through unchanged. Defaults are arm=0, keep=50, deploy=100. Enabled values must be finite: arm>0, keep and deploy in [0,100]. Invalid settings block new exposure; they are not silently clamped.

The state is the observed trading-day identity, starting equity and greatest observed equity. The day uses the existing session offset. No future daily maximum or future trade outcome is available to this calculation. It is an equity-peak mechanism, not a realized-profit-only mechanism.

The portfolio capacity is max(current equity, 0) * portfolio / 100 when portfolio > 0. It is active from the first order, without a day-anchor dependency. A nonfinite portfolio value is rejected; finite values <= 0 disable it. Once positive peak profit reaches start * arm / 100, the additional reserve is armed:

- floor = start + max(peak - start, 0) * keep / 100;
- reserve capacity = max(current equity - floor, 0) * deploy / 100;
- total capacity = min(active portfolio capacity, active reserve capacity), with each inactive capacity omitted;
- downside of positions = sum(max((current exit quote - SL) * direction, 0) * 100 * volume), using bid for BUY and ask for SELL;
- downside of pending orders = sum(max((entry - SL) * direction, 0) * 100 * volume);
- remaining = max(capacity - all downside, 0).

The contract multiplier 100 is the existing XAUUSD contract. All visible and routed hidden positions/orders consume the same budget. Positions use their actual broker SL, or recorded virtual SL only when broker SL is absent. Missing/nonfinite/nonpositive protection, invalid quotes/exposure, or an unknown day anchor blocks new exposure after activation. A stale/missing day anchor is never replaced with a guessed initial balance. Before the arm threshold, the independently enabled portfolio cap still applies. The day anchor is required only when the daily reserve itself is enabled. If the reserve is not armed and the portfolio cap is disabled, sizing remains the historical passthrough.

Every actual market or pending send re-evaluates current broker equity/exposure, including grid, hybrid/fallback market, rearm, relot, reentry, and EA-BETA. New market risk uses executable entry ask/BUY or bid/SELL to the actual requested broker SL after the adapter's price-digit normalization; pending risk uses its requested entry and SL. The requested volume is reduced by remaining/per-lot-risk and floored to the broker step with broker and strategy bounds. It is never rounded up to the minimum; if no legal minimum fits, the send is refused. Sequential orders consume the exposure returned by earlier broker acknowledgements. Relot may reclaim its own nonfrozen pending exposure for planning only; the actual send never reclaims unconfirmed cancellation.

The feature does not close positions, cancel existing pending orders, or rewrite source validity. It restricts new exposure; existing SL changes and later price moves can exceed the earlier budget. Combine it with the separate day-trail ProfitPeak guard if closure is desired. It is not a guaranteed equity floor: gaps, costs, delayed broker state and external changes remain material. Deployment must not describe this as a guaranteed positive day.

Panel OpenOrder also respects enabled active-strategy budgets: an oversized explicitly entered volume is rejected with the allowable maximum, never silently resized. Direct trades in the external MT5 terminal are outside this application.

Starting equity, day and peak are restored from the existing account-scoped risk snapshot even when optional strategy continuation is off. The three new profit-anchor scalars also use exact floating-point bit snapshots to avoid a decimal JSON parser changing a lot-boundary decision by one ULP. The live risk signature observes these three scalars while the feature is enabled, so a changed peak cannot be silently omitted from a normal persistence cycle. Account/server/mode/magic/symbol scopes remain separate. Manual risk override does not bypass this sizing mechanism.

Rejected engine sends retain the stable `ProfitBudget::<reason>` diagnostic prefix for compatibility; the visible message is RISK BUDGET. They use RiskBudgetExhausted in the journal; reports must show filled-basket/signal usage as well as PnL. Existing min-lot constraints can reduce coverage sharply at a 0.01 cap.

Validation contract: disabled/unarmed parity; exact arm boundary; BUY/SELL marks; protected SL, missing SL and hidden exposure; portfolio minimum; zero deploy/100% keep; broker min/step/max flooring; sequential sends, hybrid/rearm/reentry/EA paths; relot reclaim only after actual cancellation; same-day restore and account switch; next-day reset; generated replay activity agrees with the complete ledger.

FastAddon capacity counts attempts at the Engine-to-Broker interface. Local price,
volume or risk validation sends nothing and cannot consume the slot. A submitted
request with a rejected or uncertain acknowledgement retains the existing consumed
slot/cooldown protection. A local invalid-TP attempt retains the preset's configured cooldown without consuming capacity, matching the archived temporal rule. The internal submission sequence wraps and is compared
for inequality; it is operational state, not an additional strategy setting.
