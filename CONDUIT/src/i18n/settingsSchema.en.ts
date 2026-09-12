/* ============================================================
   ENGLISH OVERLAY FOR THE SETTINGS SCHEMA (PR2 — the bulk).

   Merged over `settingsSchema.ts` BY FIELD KEY (`przetlumaczGrupy`).
   Only display strings live here: group titles/descriptions, labels,
   hints, warnings and select-option labels. Setting KEYS and option
   VALUES are never touched — the Rust canary
   (`zaden_klucz_presetu_nie_ginie_po_tlumaczeniu`) guards those.

   Translation rules (JEZYKI_SPEC.md):
   * professional trading English — the evidence-report register;
   * hints are translated BY MEANING, not word for word;
   * technical identifiers such as `XAUUSD.s` and `mt5_symbol` stay unchanged.
   ============================================================ */

import type { Settings } from "@/types";
import type { GrupaEn } from "./schema";

const gapTrapEn = (s: Settings) =>
  s.trail_mode === "gap" && s.runner_trail && s.runner_trail_gap >= s.runner_trail_start
    ? "Gap ≥ activation threshold: the SL will land at BE and you will hand back the ENTIRE profit (real case: threshold 10, gap 12 → +$11 went to zero)."
    : null;

export const SCHEMA_EN: Record<string, GrupaEn> = {
  t100: { title: "T-100 · experimental", desc: "Autonomous policy for the selected preset. Its enable switch applies independently of application mode.", fields: {
    t100: { label: "T-100 — autonomous entries, exits and Synergy context", hint: "Experimental configuration: entry rules, risk, SL/TP, volatility, Synergy context and UTC session." },
  } },
  /* ================= ENTRIES ================= */
  entry: {
    title: "Entries and zone",
    desc: "How the bot reads the signal's entry zone and when it enters at all.",
    fields: {
      explicit_pending_until_cancel: {
        label: "LIMIT/STOP SIGNALS VALID UNTIL CANCELED",
        hint: "An explicit LIMIT/STOP signal does not expire by age, TTL, TP or RISK FREE. A CANCEL / NO LONGER VALID reply withdraws the signal. Account protection may still remove exposure; that does not cancel the source or promise automatic order restoration. Does not apply to a market signal for which the bot generates a limit grid.",
      },
      auto_limit: {
        label: "AUTO LIMIT",
        hint:
          "A signal WITHOUT the word LIMIT is laid out as a grid of pending orders in the zone instead of entering at market. " +
          "Applies to EVERY market signal, not just one whose price ran outside the zone. " +
          "Disabled: the whole grid (levels × units) may open in a single tick at one price, " +
          "which can bypass the intended staggered exposure. Leave it on unless this behavior is deliberate.",
      },
      only_limit_signals: {
        label: "LIMIT SIGNALS ONLY",
        hint: "Ignore market BUY/SELL signals — trade limits exclusively (the best family in the sweeps).",
      },
      valid_till_tp2: {
        label: "VALID TILL TP2",
        hint: "For ordinary signals, moves entry expiry from TP1 to TP2. Explicit LIMIT/STOP signals valid until cancellation do not expire at TP.",
      },
      custom_entry: {
        label: "CUSTOM ENTRY LEVEL",
        hint: "Custom price offsets for the zone edges. Enabling this clears the competing direction-aware offset mode.",
      },
      entry_high_offset: { label: "upper edge ±" },
      entry_low_offset: { label: "lower edge ±" },
      entry_offset_dir: {
        label: "DIRECTION-AWARE OFFSETS (SELL fix)",
        hint: "BUY: a better price is lower. SELL: higher. Enabling this clears the competing CUSTOM ENTRY price-offset mode.",
      },
      entry_deep_offset: { label: "depth (towards better entries)" },
      entry_deep_frac_to_sl: {
        label: "depth as a FRACTION of the distance to the SL",
        hint:
          "0 = use the fixed amount above. A fraction scales depth from the edge-to-SL distance, so it adapts to narrow and wide zones. A value at or above 1.0 can place a rung on the stop; validate any non-zero value in simulation first.",
      },
      entry_tol_offset: {
        label: "tolerance (past the edge)",
        hint: "Allows a configurable tolerance beyond the signalled zone edge.",
      },
      sl_dist_limit: {
        label: "DISTANCE-TO-SL LIMIT",
        hint: "Do not open an entry farther from the SL than X. NOTE: in the backtest this option did NOT help — with X ≤ 6 it halves the profit.",
      },
      sl_dist_max: { label: "max distance to SL" },
      sl_max_dist: {
        label: "MAX SL WIDTH",
        hint: "Clamp the basket SL to this distance from the zone midpoint. 0 = take the signal's SL as is.",
      },
      skip_if_sl_breached: {
        label: "SKIP A SIGNAL WITH A BREACHED SL",
        hint: "Rejects a setup when price has already crossed its stop before the signal arrives, preventing an unreachable entry.",
      },
      max_chase_beyond_zone: {
        label: "DO NOT CHASE THE SETUP BEYOND",
        hint: "Maximum distance of price from the better zone edge at which we still enter at market. 0 = no limit.",
      },
      side_filter: {
        label: "SIGNAL DIRECTION",
        hint: "Optionally allows only selected directions. Measure the impact on your own data.",
        options: { both: "both directions", buy: "BUY only", sell: "SELL only" },
      },
      sl_min_dist: {
        label: "MIN SL WIDTH",
        hint: "Widen the basket SL to at least this distance from the zone midpoint. A tight SL may exit before a reversal; measure the effect on your own data. 0 = OFF.",
      },
      ignore_old_after_min: {
        label: "IGNORE STALE SIGNALS AFTER",
        hint: "Expires waiting signals without an open position after X minutes. 0 disables the threshold. Explicit LIMIT/STOP signals valid until cancellation are exempt.",
      },
    },
  },

  /* ================= GRID ================= */
  grid: {
    title: "Units and position grid",
    desc: "How many positions the bot lays out in the entry zone, and how far apart.",
    fields: {
      entry_units: {
        label: "ENTRY UNITS per level",
        hint: "The MEGA champion structure: N minimum-lot positions on every entry level.",
      },
      entry_units_limit: {
        label: "units for LIMIT baskets",
        hint: "Separate sizing for limit baskets. 0 = same as above.",
      },
      ppm_enabled: {
        label: "PLACING POSITION MULTIPLIER",
        hint: "Densifies positions: price step = 1/PPM (PPM 2 → every 0.50, PPM 10 → every 0.10).",
      },
      ppm: { label: "PPM" },
      ppm_immediate: { label: "PPM for market entries" },
      ppm_for_limits: { label: "PPM for pendings" },
      entry_weights: {
        label: "VOLUME WEIGHTS BY DEPTH",
        hint:
          "Volume distribution across the zone, from the shallowest entry to the deepest — e.g. “1,2,4”. The median zone is $5 wide: at the shallow edge R:R is 0.50 (SL $6, TP1 $3), at the deep edge 8.0 (SL $1, TP1 $8). Equal volumes hand the decision to the worst entry. Weights only REWEIGHT the basket — its total size stays unchanged. Empty = a flat ladder.",
      },
      risk_per_basket_pct: {
        label: "BASKET RISK LIMIT",
        hint: "A hard cap on the sum of |entry − SL| × 100 × volume over all of the basket's orders, as % of capital. A basket has one shared SL, so it either exits on targets or dies whole. Once breached, the engine first scales volumes down, then drops the shallowest levels, finally the whole basket. 0 = no limit.",
        warn: (s: Settings) =>
          s.risk_per_basket_pct > 10
            ? "Over 10% of capital on a single basket. Three such baskets at once is already close to half the account."
            : null,
      },
      market_entry_step: {
        label: "MARKET-ENTRY STEP",
        hint: "How far price must move in our favor before the bot adds another market position or re-entry. Without this step the bot would add on every tick inside the zone.",
      },
      pending_ttl_from_basket: {
        label: "TTL counted from basket creation",
        hint: "The rule reads “a stale setup only fills in a crash”, and re-arranging the grid does not rejuvenate the setup. Disabled = age counted from when the individual order was placed.",
      },
      entry_risk_budget: {
        label: "RISK BUDGET PER LEVEL",
        hint: "Units = clamp(budget / distance to SL, 1, units). A wide SL gets fewer units. 0 = OFF.",
      },
      entry_tp1_budget: {
        label: "SIZING FROM SIGNAL GEOMETRY (TP1)",
        hint: "Units = clamp(budget / |TP1 − level price|, 1, units). Closer to TP1 = more units. 0 = OFF.",
      },
      entry_touch_units: {
        label: "TOUCHER — units at the top of the zone",
        hint: "Extra units at the zone edge (BUY: the top) with their own early TP — they catch signals that barely grazed the zone and ran away.",
      },
      entry_touch_tp: { label: "toucher TP", hint: "1 = TP1, 2 = TP2…" },
      entry_touch_levels: {
        label: "DEPTH BANDS (off:units:tp)",
        hint: "List of toucher levels, offset in PIPS downward from the zone top. Champion: “0:4:2,7:4:2”. Overrides the field above.",
      },
      pending_resize_on_vol: {
        label: "PENDING-RESIZE by volatility",
        hint: "Recompute the unit count of unfilled pendings every X seconds — parity with the engine (vol@fill instead of vol@placement).",
      },
      pending_resize_sec: { label: "every N seconds" },
      pending_relot_on_balance: {
        label: "PENDING-RELOT by balance",
        hint: "Update the VOLUME of resting limits when the base lot changes. Upwards it recovers profit (a grid placed at $200 does not fill with a 0.01 lot once the account is $500), DOWNWARDS it protects capital (an order from a $5000 account must not open a huge position after the account fell to $200). Recomputed on the same cadence as PENDING-RESIZE.",
      },
      pending_relot_topup: {
        label: "…by top-up",
        hint: "ENABLED: add a separate order for just the difference — the original rung never leaves the market and keeps its place in the queue. DISABLED: cancel and re-place (the rung vanishes for a moment and can be rejected).",
      },
      pending_relot_up: {
        label: "…UPWARDS (add)",
        hint: "The PROFIT side: when a rung is smaller than it should be, add volume. Disabling gives the reduce-only variant (HYPER-X1A), tuned for lower risk rather than higher growth.",
      },
      pending_relot_down: {
        label: "…DOWNWARDS (reduce)",
        hint: "The RISK side: when a rung is bigger than the account can afford, shed the excess. Top-ups are cancelled first; only then is the base touched.",
      },
      pending_relot_up_od_salda: {
        label: "…add only from balance of",
        hint: "The UPWARD direction arms only once the balance reaches this amount; below it, reduction alone operates. 0 = no threshold.",
      },
      pending_relot_wg_planu: {
        label: "…target per PLAN, not the bare lot",
        hint: "ENABLED: a rung's target is the volume from the grid plan recomputed for the current balance — with RR weights and the basket risk limit. DISABLED (as before): the target is unit count × bare base lot, which flattens the ladder and bypasses the risk throttle.",
        warn: (s) => s.pending_relot_reconcile_target ? "Overridden: the new relot contract always uses the full validated plan. The legacy choice is retained for returning to OFF." : null,
      },
      pending_relot_reconcile_target: {
        label: "RELOT — RECONCILE THE TOTAL RUNG TARGET",
        hint: "The new contract uses the full validated plan and aggregate rung volume instead of treating each top-up as an independent target. Overrides pending_relot_wg_planu but preserves the master relot switch, up/down gates, thresholds and margin checks. OFF keeps the previous mechanism. Owned by the selected preset, not the account.",
      },
      pending_ttl_h: {
        label: "PENDING TTL",
        hint: "Removes unfilled orders after X hours; 0 disables TTL. Explicit LIMIT/STOP signals valid until cancellation are exempt. Risk protection remains active.",
      },
      pending_never_cancel: {
        label: "PENDINGS NEVER EXPIRE",
        hint: "Limits are never cancelled after a TP (the group often fills limits over hours).",
      },
      entry_allowance_usd: {
        label: "ALLOWANCE LAYER BEFORE THE ZONE",
        hint: "Allowed distance before the zone: above the upper edge for BUY and below the lower edge for SELL. This axis adds exposure; 0 disables it.",
      },
      entry_allowance_units: {
        label: "units on the allowance layer",
        hint: "Additional orders on top of the in-zone grid. Zero units or zero amount disables this layer.",
      },
      entry_depth_curve: {
        label: "entry depth curve",
        hint: "1.0 = no bending. NOTE: the measured effect of deeper entries is a BETTER PRICE, not an informational edge of the channel — so this effect must NOT be stacked with anything justified by signal quality. That is why 1.0 stays the default.",
      },
    },
  },

  /* ================= TP ================= */
  targets: {
    title: "Target management (TP)",
    desc: "Who gets which TP, when the bot counts a target as hit, and what it banks then.",
    fields: {
      all_runners: {
        label: "ALL TPS ARE RUNNERS",
        hint: "Every position gets the most favorable TP. Enabling this clears SCALE-OUT and official mode. Trailing uses its own settings.",
      },
      scale_out: {
        label: "SCALE-OUT",
        hint: "Closes a percentage of positions at successive TPs. Enabling this clears ALL RUNNERS and official mode.",
      },
      scale_out_pct: { label: "% of positions per TP" },
      scale_out_round: {
        label: "rounding of position count",
        options: { up: "up (30% of 13 → 4)", down: "down (30% of 13 → 3)" },
      },
      scale_out_from: {
        label: "take the % from",
        options: {
          worst: "the worst — they close first",
          best: "the best — they close first",
        },
      },
      scale_out_last_runner: {
        label: "the last position (LAST RUNNER)",
        options: {
          runner: "runner — the farthest target of the ladder",
          next_tp: "next TP — gets the nearest unhit target",
          no_tp: "no target — run by the trailing stop alone",
        },
      },
      tp_open_offset: {
        label: "TP OPEN — step of successive targets",
        hint: "How many points apart TP OPEN and further generated targets are placed.",
      },
      tp_source: {
        label: "HOW WE KNOW A TARGET WAS HIT",
        hint: "Target confirmation may come from price, a message, or both. Price-aware modes limit the impact of early or delayed messages.",
        options: {
          Either: "whichever comes first (price or channel)",
          PriceOnly: "price from MT5 only",
          SignalOnly: "channel message only",
          SignalConfirmedByPrice: "message confirmed by price",
          PriceFirstSignalWindow: "price decides, message within a time window",
        },
      },
      tp_price_tolerance: {
        label: "price-confirmation tolerance",
        hint: "The signaller's broker has a different BID than ours — a target “almost” hit at one is hit at the other.",
      },
      tp_price_front_run_usd: {
        label: "AUTONOMOUS MT5-PRICE TP FRONT-RUN",
        hint: "0 = off: require the full target touch as before. Above zero, count the next TP this many USD early using only the current MT5 Bid/Ask, without a Telegram message. It applies only to a basket with an open position and never shortens a pending grid's life.",
      },
      tp_signal_max_lead_s: {
        label: "seconds BEFORE the price touch",
        hint: "How far the message may lead the market. 0 = require price confirmation.",
      },
      tp_signal_max_lag_s: {
        label: "seconds AFTER the fact",
        hint: "Past this, a late message is no longer current. 0 = no limit.",
      },
      tp_stage_from_broker_fill: {
        label: "COUNT A TARGET FROM THE BROKER FILL",
        hint: "The broker closed the position at its take-profit — the hardest proof of a hit, independent of both our price feed and of whether the signaller got around to writing.",
      },
      tp_detect_price: {
          label: "detect TP from PRICE (MT5)",
          hint: "Shortcut for TP source above. A click updates both switches and the mode together. Both OFF is unavailable: PriceOnly remains. The separate broker TP-fill option is not controlled here.",
        },
      tp_detect_signal: { label: "detect TP from MESSAGES", hint: "Shortcut for TP source. A click selects a simple mode: price, message, or whichever arrives first. It does not disable other Telegram management commands." },
      tp_freeze_after_ladder: {
        label: "FREEZE TP AFTER THE LADDER",
        hint: "Once the real TP ladder is exhausted, runners stay on the last real target (no extrapolation, which tends to run away from price).",
      },
      tp_hit_fill_stages: {
        label: "BACKFILL SKIPPED STAGES",
        hint: "“TP3 HIT” at stage 0 also executes the TP1 and TP2 plans instead of losing them.",
      },
      spp_max_age_h: {
        label: "SPP-REARM GUARD",
        hint: "Re-arming a basket with a new target ladder applies only to baskets younger than X h. 0 = no guard.",
      },
    },
  },

  /* ================= TRAILING ================= */
  trailing: {
    title: "Trailing SL",
    desc: "How the stop-loss follows profit — the most important profit axis in the backtests.",
    fields: {
      runner_trail: {
        label: "SAFETY TRAILING STOP",
        hint: "After the profit threshold, moves the SL using the selected mode. Split bank + runner trailing has a separate switch. A protective order does not guarantee its fill price or net profit.",
        warn: (s: Settings) =>
          !s.runner_trail && s.trail_split
            ? "Basic trailing is OFF, but “split trailing (bank + runner)” below is ON and keeps working — runners still get a trailing stop. This is a separate settings family."
            : null,
      },
      runner_trail_start: { label: "activate at profit ≥" },
      trail_mode: {
        label: "TRAILING MODE",
        options: {
          gap: "gap — SL a gap behind price (classic)",
          lock_pct: "lock_pct — lock a % of the peak (recommended)",
          tiered: "tiered — a ladder of thresholds",
          atr: "ATR — volatility-based gap",
          chandelier: "Chandelier — extreme and ATR",
        },
      },
      runner_trail_gap: { label: "SL gap behind price", warn: gapTrapEn },
      trail_lock_pct: {
        label: "lock % of the peak",
        hint: "95 = the stop attempts to retain 95% of the position's best observed profit. Validate the value on independent windows.",
      },
      trail_tiers: {
        label: "“profit:lock” ladder",
        hint: "E.g. 10:5 = at a +10 peak, lock in +5.",
      },
      trail_split: {
        label: "SPLIT TRAILING (bank + runner)",
        hint: "The group's philosophy: “CLOSE 3 LAYERS AND LEAVE BEST ENTRY RUNNING”. The N best positions get a loose trail, the rest a tight one. A family INDEPENDENT of “SAFETY TRAILING STOP” above — turning that off does NOT turn this off.",
        warn: (s: Settings) =>
          s.trail_split && !s.runner_trail
            ? "Works DESPITE basic trailing being off. If you want trailing fully off, disable THIS field as well."
            : null,
      },
      trail_runners_by_depth: {
        label: "RUNNERS BY ENTRY DEPTH",
        hint: "Without this, “how many positions as runners” is DEAD: n=1 and n=3 gave results identical to the sixth decimal place, because “runner” simply meant “a position without a TP”. Only this picks runners by how deep they entered.",
      },
      trail_runners_n: {
        label: "how many positions as runners",
        hint: "Takes effect ONLY with “runners by entry depth” enabled — otherwise the value changes nothing.",
      },
      trail_runner_mode: {
        label: "runner mode",
        options: { gap: "gap", lock_pct: "lock_pct", tiered: "tiered", atr: "ATR", chandelier: "Chandelier" },
      },
      trail_runner_start: { label: "runner threshold" },
      trail_runner_gap: { label: "runner gap" },
      trail_runner_lock_pct: { label: "runner % of peak" },
      trail_runner_tiers: {
        label: "runner ladder",
        hint: "Loose at first (gives the move room), tight once the profit is large — the opposite of a regular position.",
      },
      trail_adaptive_enabled: {
        label: "MARKET-QUALITY ADAPTIVE TRAILING",
        hint: "Uses past ticks only. A clean favorable move widens the runner gap, noise normalizes it, and a fast reversal tightens it. Applies to Gap/ATR/Chandelier and never loosens an SL already accepted.",
      },
      trail_adaptive_runners_only: {
        label: "runners only",
        hint: "When enabled, ordinary positions retain their original trailing and only the profit tail is adapted.",
      },
      trail_adaptive_window_s: { label: "move-quality window" },
      trail_adaptive_min_samples: { label: "minimum samples" },
      trail_adaptive_trend_er: {
        label: "clean-trend threshold (ER)",
        hint: "0..1; 1 means a move with no pullback at all.",
      },
      trail_adaptive_reversal_er: { label: "clean-reversal threshold (ER)" },
      trail_adaptive_trend_gap_mult: { label: "gap multiplier: favorable trend" },
      trail_adaptive_chop_gap_mult: { label: "gap multiplier: noise" },
      trail_adaptive_reversal_gap_mult: { label: "gap multiplier: reversal" },
      trail_adaptive_fast_vol_s: { label: "fast speed window" },
      trail_adaptive_slow_vol_s: { label: "slow speed window" },
      trail_adaptive_vol_ratio: { label: "speed-expansion threshold" },
      trail_adaptive_vol_favorable_mult: { label: "favorable expansion multiplier" },
      trail_adaptive_vol_adverse_mult: { label: "adverse expansion multiplier" },
      trail_adaptive_min_peak: { label: "adapt only after peak" },
      trail_adaptive_min_gap: { label: "minimum effective gap", hint: "0 = no lower clamp." },
      trail_adaptive_max_gap: { label: "maximum effective gap", hint: "0 = no upper clamp." },
      trail_sr_enabled: {
        label: "S/R STRUCTURE TRAILING (1M)",
        hint: "The bot finds swings on 1M candles (fractal n=3 on the mid price) and trails the SL below the last confirmed swing low for BUY or above the swing high for SELL, with a configurable offset. Decisions occur only on candle close; the SL ratchets toward profit only, and a broker refusal skips that candle rather than clamping the level.",
      },
      trail_sr_scope: {
        label: "position scope",
        hint: "Runner = Hold positions (the same definition as in split trailing). Tp3Up and All broaden the scope and should be evaluated separately.",
        options: {
          Runner: "Runner — the Hold runner only",
          Tp3Up: "Tp3Up — the TP3+ layer",
          All: "All — every position",
        },
      },
      sr_warmup_exact_ticks: {
        label: "EXPERIMENT: EXACT DYNAMIC S/R WARMUP",
        hint: "OFF preserves legacy warmup. V2 requires causal Bid/Ask ticks, actual first-tick timestamps, MID extrema and last spread. It serves existing dynamic S/R axes; it does not enable the parent or ATR axes. Full broker ticks differ from bot-observed samples. Research implementation; LIVE history and native parity are not yet qualified.",
        warn: (s: Settings) => s.sr_warmup_exact_ticks ? "NOT LIVE-QUALIFIED: the V2 history producer and observation-stream parity are not complete." : null,
      },
      trail_sr_activation: {
        label: "activate from",
        hint: "Stages counted by TOUCHES of the signal's targets (the basket stage), not the layer's own targets. The curve is monotonic toward late activation with a Tp2–Tp3 plateau.",
        options: {
          Entry: "Entry — from entry",
          Gain: "Gain — from a $ profit threshold",
          Tp1: "Tp1 — after the first target",
          Tp2: "Tp2 — after the second target (recommended)",
          Tp3: "Tp3 — after the third target",
        },
      },
      trail_sr_min_gain: {
        label: "profit threshold (Gain)",
        hint: "Read ONLY with the Gain activation. 0 = from entry.",
      },
      trail_sr_min_dist_price: {
        label: "breathing room from price ≥",
        hint: "A candidate level must be at least this far from the current mid price. 0 disables the filter.",
      },
      smart_sl: {
        label: "SMART SL TRAILING",
        hint: "After the n-th TP, the n best positions get an SL per the ladder (Initial → TP1 → TP2…).",
      },
      breakeven_protection: {
        label: "BREAKEVEN PROTECTION LAYER",
        hint: "Adds a BE stage to the ladder: Initial → BE → TP1 → TP2… Without SMART SL it independently enables BreakevenOnly; scale-out is not required.",
      },
      trail_after_tp2: {
          label: "SMART SL LADDER DELAY",
          hint: "Shortcut: ON sets a one-stage delay (from TP2 for the best entry), OFF sets zero. An existing preset value above one is preserved until you click. This does not block independent BE, RF, or other trailing methods.",
      },
      harvest: {
        label: "PROFIT HARVEST",
        hint: "Does not wait for the SL to be hit — catches “+20 and a sudden reversal”.",
      },
      harvest_start: { label: "watch from profit of" },
      harvest_retrace_pct: { label: "retrace of % of the peak → close" },
      be_offset: {
        label: "BREATHING ROOM ABOVE ENTRY (BE)",
        hint: "How far above the entry price to place breakeven. Exactly at entry the position exits at zero MINUS the spread — i.e. at a loss.",
      },
      be_covers_late_fills: {
        label: "BE COVERS LATE FILLS AND RE-ENTRIES",
        hint: "When enabled, positions filled after a BE command inherit protection from their own entry price. This keeps pending orders without leaving them on the original deep stop.",
      },
      be_never_loosen: {
        label: "BE NEVER LOOSENS A BETTER SL",
        hint: "Enabled: automatic BE after TP/RISK FREE cannot worsen an already more protective SL. Does not require S/R trailing, does not arm BE itself, does not guarantee net profit and does not block an intentional manual modification. Disabled preserves the historical path.",
      },
      sltp_retry_s: {
        label: "RETRY REJECTED SL/TP EVERY",
        hint: "The broker rejects a stop too close to price or on a requote. One attempt and silence means the tightened stop simply VANISHES — the difference between “closed at +18” and “closed at SL”. 0 = no retries.",
      },
      trail_min_dist: {
        label: "MIN SL DISTANCE",
        hint: "Instead of sending an SL the broker will reject (closer than STOPS LEVEL), push it out to this distance from price. 0 = OFF.",
      },
      ladder_from_tp: {
        label: "SL LADDER — start from TP",
        hint: "SL = reached TP[stage − lag] minus breathing room. 0 = disabled.",
      },
      ladder_lag: { label: "ladder lag" },
      ladder_offset: {
        label: "ladder breathing room",
        hint: "SL below/above the level, so positions are not shaken out right before a hit.",
      },
    },
  },

  /* ================= VIRTUAL SL ================= */
  vsl: {
    title: "Virtual SL",
    desc: "An SL level kept at the bot — the broker never sees it, so it cannot be hunted.",
    fields: {
      virtual_sl: {
        label: "VIRTUAL SL",
        hint: "The bot closes at market when price crosses the level. The real SL from the signal stays in place as a safety net in case the VPS dies.",
      },
      virtual_sl_only_when_rejected: { label: "only when the broker rejects the SL" },
      vsl_eval_s: {
        label: "check cadence",
        hint: "0 = every loop. 15–30 s = “wick tolerance”: a spike between checks is invisible.",
      },
      virtual_sl_all: {
        label: "ALL SLs VIRTUAL",
        hint: "Every SL is kept at the bot, and the broker gets an SL pushed out by the rescue net. Requires VIRTUAL SL = ON.",
      },
      vsl_net_off: { label: "broker net offset" },
    },
  },

  /* ================= ATFX PHILOSOPHY ================= */
  atfx: {
    title: "ATFX philosophy",
    desc: "Mirrors the signal group's behavior — more days in the green.",
    fields: {
      be_lock: {
        label: "BE-LOCK",
        hint: "After the profit threshold, moves the SL to the entry with the configured lock. Costs and the actual fill price affect net profit.",
      },
      be_lock_points: { label: "BE-LOCK threshold" },
      be_at_tp1: {
        label: "BE ONLY AT TP1",
        hint: "The trader's system: “as soon as the trade hits tp1 set break even on ALL positions”.",
      },
      reenter_after_tp: {
        label: "RE-ENTRY after TP",
        hint: "Enter again when price returns to the zone (“TP1 HIT AGAIN AFTER PULLING BACK”).",
      },
      reenter_max: {
        label: "max re-entries per basket",
        hint: '0 = NO LIMIT (not "zero re-entries"). Bounds how long a single setup may keep renewing itself.',
        warn: (s: Settings) =>
          s.reenter_max === 0
            ? '0 means INFINITELY MANY re-entries into one basket, not "zero". With "VALID TILL TP2" ' +
              "the bot may add every time price returns to the zone. To cap this behavior, enter a finite " +
              "number and validate it on a demo account."
            : null,
      },
      reenter_min_tp_stage: {
        label: "required TP stage for re-entry",
        hint: "0 = enter on the first zone touch, 1 = only after TP1.",
      },
      oae_timeout_min: {
        label: "OUT-AT-ENTRY after",
        hint: "Close the position ~at BE if it hangs X minutes without profit. 0 = disabled.",
      },
      oae_profit_min: { label: "“no profit” means below" },
      ignore_out_at_entry: { label: "IGNORE “OUT AT ENTRY” from the channel" },
      ignore_risk_free: {
        label: "IGNORE “RISK FREE” from the channel",
        hint: "A permissiveness-hypothesis test: do not close the grid on a RISK FREE message.",
      },
    },
  },

  /* ================= OFFICIAL SYSTEM ================= */
  official: {
    title: "Official ATFX system",
    desc: "The percentage schedule stated outright by the group's trader: 15 / 30 / 30 / 20.",
    fields: {
      official_mode: {
        label: "OFFICIAL MODE",
        hint: "Uses a separate percentage or position-count schedule. Enabling this clears ALL RUNNERS and SCALE-OUT.",
      },
      official_pct_tp1: { label: "close at TP1" },
      official_pct_tp2: { label: "at TP2" },
      official_pct_tp3: { label: "at TP3" },
      official_pct_spp: { label: "each SPP target" },
      official_use_counts: {
        label: "COUNTS instead of percentages",
        hint: "The way the trader counts: “1,2,1” = 1 position at TP1, 2 at TP2, 1 at TP3, the rest = runners.",
      },
      official_counts: { label: "count schedule" },
      official_spp: {
        label: "NEW SPP",
        hint: "After TP3 close a % on EVERY target + trailing SL; positions stay open.",
      },
      official_round: {
        label: "whole-position rounding",
        hint: "“nearest” loses TP1 on a small account: 15% of 3 positions = 0.45 → 0 closed. “up” always banks ≥ 1.",
        options: {
          nearest: "nearest — closest (default)",
          up: "up — always ≥ 1 position",
          down: "down — conservative",
        },
      },
      official_assign_tps: {
        label: "TP PER POSITION",
        hint: "Every position gets its own TP per the schedule — MT5 closes them itself at the exact level, even when the bot is offline.",
      },
      official_close_last: { label: "close the last position" },
      spp_keep_tp: { label: "SPP keeps the TP" },
      partial_close: {
        label: "VOLUME PARTIALS",
        hint: "Close a % of EACH position's volume (0.05 → 0.04) instead of a % of the count of whole positions. The group's main mode.",
      },
      partial_min_lot: {
        label: "partials threshold: lot ≥",
        hint: "0.01 is indivisible. PROVEN: from 0.02, partials add +13 pp of profitable days.",
      },
      partial_pct_od_pierwotnego: {
        label: "percentages OF THE ORIGINAL POSITION",
        hint:
          "Without this, 15/40/15 is taken off WHAT IS LEFT: 15 %, then 40 % of 85 % (=34 %), " +
          "then 15 % of 51 % (=7.7 %) — 56.7 % in total instead of 70 %, and a sequence that never " +
          "closes. Turn on for the TYLER family.",
      },
      cele_na_ostatnim: {
        label: "EVERY POSITION AIMS AT THE FURTHEST TARGET",
        hint:
          "Separates TARGETS from BANKING. Without it the percentage schedule spreads positions " +
          "across ladder rungs, so a position assigned to TP1 closes at TP1 with the broker and " +
          "never lives to TP2 — and then there is nothing left to cut the next tranche from.",
      },
      retarget_respects_final_target: {
        label: "RETARGET RESPECTS THE FINAL TARGET",
        hint: "Enabled together with cele_na_ostatnim: retarget keeps live positions at the final target instead of a nearer TP. Only applies when the position already has a broker TP; does not restore intentionally removed TP=None. Does not change tranche cash-outs. Disabled preserves the historical path.",
      },
      sl_polowa_od_konca: {
        label: "SL halfway — from which target COUNTING BACK",
        hint:
          "0 = off. 1 = after every target except the last. Counted from the end, because Synergy " +
          "publishes a different number of targets per signal. Ratchet: the stop only moves toward profit.",
      },
      sl_polowa_ulamek: {
        label: "fraction of the entry → price distance",
        hint: "0.5 = halfway. 0 reads as 0.5. Less leaves the runner more air.",
      },
    },
  },

  /* ================= RISK FREE ================= */
  riskfree: {
    title: "RISK FREE",
    desc: "What the bot does with a basket after a RISK FREE message from the channel.",
    fields: {
      risk_free_runner_target: {
        label: "RUNNER TARGET AFTER RISK FREE",
        options: {
          last: "the farthest target of the ladder",
          keep: "keep the current one",
          next: "the next unhit target",
          none: "no target — trailing only",
        },
      },
      risk_free_trail: { label: "runner gets a trailing stop after RISK FREE" },
      risk_free_be_min_profit: {
        label: "RISK FREE only moves the stop once the profit reaches",
        hint:
          "Separates the stop location from the minimum current profit required before moving it. 0 preserves legacy behavior.",
      },
      out_at_entry_mode: {
        label: "OUT AT ENTRY — what to close",
        hint: "The message means different things from different signallers: sometimes “I'm out entirely”, sometimes “dump whatever is going nowhere”.",
        options: {
          close_all: "the whole basket",
          losers: "losers only",
          flat: "only the ~flat ones",
          be: "close nothing, move SL to entry",
        },
      },
      oae_band_pts: { label: "“flat” band" },
      sl_hit_mode: {
        label: "SL HIT from the channel — what to do",
        options: {
          cancel_pendings: "cancel limits, leave positions on their own SL",
          close_all: "close everything at market",
          verify: "verify against price (reject a false one)",
          ignore: "ignore",
        },
      },
      honor_cancel: { label: "react to CANCEL from the channel" },
      honor_close_all: {
        label: "react to CLOSE ALL from the channel",
        hint: "This message closes EVERYTHING, not just the referenced basket — unless you narrow it with the field next to it.",
      },
      close_all_scope: {
        label: "CLOSE ALL scope",
        hint: "Global mode ignores the reply address and can close unrelated baskets. Use it only when the source syntax unambiguously means the entire portfolio.",
        options: {
          Global: "Global — close everything (as before)",
          Basket: "Basket — the referenced basket only",
        },
      },
      partials_wykonuj: {
        label: "EXECUTE “TAKE PARTIALS” FROM THE CHANNEL",
        hint: "An unambiguous partial-close command may realize profit immediately. Conditional wording or a suggestion remains informational.",
      },
      partials_pct: {
        label: "tranche on “take partials”",
        hint: "% of VOLUME banked on the command. A separate number, NOT a rung of the banking ladder (TP1/TP2/TP3): the command carries no stage, so using the ladder would either invent a stage or eat a rung the real TP report will need in a moment. Ordering, the partial threshold and last-position protection are shared with banking on target. 0 = bank nothing.",
      },
      honor_market_open: {
        label: "react to “BUY NOW” / “SELL NOW”",
        hint: "Such a message carries neither an SL nor targets — the position is born without an exit plan. Off by default.",
      },
      dedup_edited_signals: {
        label: "DO NOT REPEAT ACTIONS ON EDIT",
        hint: "The signaller appends to an already-sent message (“TP1 HIT” → “TP1 HIT · SECURING PARTIAL PROFITS”). Without a memory of executed actions, the edit banks TP1 a second time.",
      },
      risk_free_runners: {
        label: "how many best positions to keep",
        hint: "The rest are closed; these stay open with SL at breakeven.",
      },
      risk_free_mode: {
        label: "basket mode after RISK FREE",
        options: {
          scale_out: "keep SCALE-OUT (runners in a cascade)",
          all_runners: "ALL TPS ARE RUNNERS (this basket only)",
        },
      },
      risk_free_smart_sl: {
        label: "SMART SL for runners after RISK FREE",
        hint: "A stepped SL computed relative to each other — the best runner gets the strongest SL.",
      },
      sl_hit_verify_tol: {
        label: "“SL HIT” price-verification tolerance",
        hint: "Only with SL HIT mode = verify price. If the mid price is more than X $ from SL on the profit side, ignore the message. Zero means strict tolerance without a buffer, NOT disabled verification. Select the mode separately.",
      },
    },
  },

  /* ================= EDITS AND DEDUP (Package A) ================= */
  edycje: {
    title: "Message edits and dedup",
    desc: "Advanced parser axes: what the bot does with channel message EDITS and with re-deliveries after a reconnect (Package A).",
    fields: {
      dedup_pelny_status: {
        label: "FULL ACTION EXECUTION STATUS",
        hint: "An IGNORED action (for example, one rejected by price or lacking a target basket) is not recorded as executed, so a later edited retry remains eligible. Turning it off restores the legacy dedup behavior.",
      },
      edycja_wykonuje_reszte_akcji: {
        label: "an edit with an entry executes the remaining actions",
        hint: "Today an edit that hits a basket re-arms it and ENDS message handling — a “TP1 HIT”/“SPP” appended in the same edit is lost. Enabling this sends the remaining actions down the normal path.",
      },
      dedup_klucz_z_wartoscia: {
        label: "dedup key carries the value",
        hint: "An edit changing a LEVEL (“MOVE SL TO 4120” → “4110”) no longer vanishes as a duplicate: the action key includes the value (setsl@4110, corr2@4162, rf@4536, spp@targets|sl|be).",
      },
      edycja_sieroty_nie_otwiera: {
        label: "an orphan edit does not open a basket",
        hint: "ON blocks a new entry from an unknown edit. OFF accepts its first complete signal with a protective SL at receipt time, without backdating; a bare BUY/SELL NOW still cannot open a basket. Source memory prevents duplicates and re-entry after CANCEL, including after restart. Management of an existing basket remains active.",
      },
      entry_idempotencja: {
        label: "new-message idempotency",
        hint: "Re-delivery after a reconnect: a new message with a msg_id that already opened a basket is treated as an edit of that basket instead of opening a second one.",
      },
      dedup_management_po_restarcie: {
        label: "persistent management dedup after restart",
        hint: "Stores executed TP/SPP/BE/SL/CANCEL actions with a live basket and restores that memory from koszyki.json. The same message or a later edit cannot execute old actions twice after a restart. Off by default for the legacy contract.",
      },
      profit_update_telemetry_only: {
        label: "AT TP is information only (TP HIT remains active)",
        hint: "AT TP1/2/3 means price proximity only and cannot bank or advance the stage. Other explicit actions in the same message (RF, SPP, CANCEL, BE/SL, new targets) still execute. TPn HIT remains information; with tp_source=SignalConfirmedByPrice the current MT5 Bid/Ask is the execution authority. Off by default for 1:1 legacy behaviour.",
      },
    },
  },

  tpPriceOnlyContract: {
    title: "Explicit TP source authority",
    desc: "An explicit PriceOnly contract; separate management instructions remain independent.",
    fields: {
      tp_price_only_strict: {
        label: "PriceOnly: no TP message advances the stage",
        hint: "In PriceOnly, also blocks the historical +N PIPS HIT bypass when tp_unindexed_pips_require_price is enabled. Explicit RF, SPP, SL and other management remain active; price ticks and broker fills are separate. Other TP modes are unchanged. Off preserves legacy Rust ordering; on matches the MT5 tester's PriceOnly gate.",
      },
    },
  },

  /* ================= TYLER AUDIT (Package B) ================= */
  tyler: {
    title: "Market-signal execution",
    desc: "Market-execution controls: RISK FREE intent guard, entry size, pending cleanup after RF and optional banking at a TP stage.",
    fields: {
      rf_wymaga_wykonania: {
        label: "RISK FREE requires execution, not an announcement",
        hint: "Conditional intent without a numeric level remains informational; an unambiguous command with a valid level may execute.",
      },
      market_entry_units: {
        label: "MARKET entry units (0 = full grid)",
        hint: "Caps the entire plan for a signal without “LIMITS”, including with auto_limit=ON. Keeps N units from the deepest valid layer; this does not mean N immediate market entries. With N=1 the bot may place only a deep pending order and miss a shallow reversal to TP. Also caps the entry_units plan; explicit LIMITS signals bypass this cap. 0 disables only this trimming, not other risk limits.",
      },
      market_hybrid_now_units: {
        label: "hybrid: units NOW (0 = OFF)",
        hint: "For a signal without LIMITS, opens N units from the first/shallow part of the plan immediately at market and leaves the rest as lower limits.",
      },
      market_hybrid_pending_units: {
        label: "hybrid: max pending units (0 = all)",
        hint: "How many remaining limit units to keep, starting with the deepest/best entries. Immediate units do not consume this budget.",
      },
      market_hybrid_lot_mult: {
        label: "hybrid: NOW-leg lot multiplier",
        hint: "Scales only the hybrid's immediate units. 1 means unchanged; non-positive values safely behave as 1.",
      },
      market_hybrid_max_chase_usd: {
        label: "hybrid: max chase from zone, $ (0 = unlimited)",
        hint: "If the quote is farther beyond the shallow edge, the market leg does not chase; the full grid still waits as limits, so the signal is not filtered.",
      },
      market_hybrid_tp_stage: {
        label: "hybrid: NOW-leg TP (0 = planner, 255 = OPEN)",
        hint: "1/2/3… assigns that signal TP to the immediate leg; values beyond the list clamp to the last TP. 255 leaves it without a fixed TP.",
      },
      market_unfilled_cancel_stage: {
        label: "market without fill: cancel pendings at TP (0 = shared rule)",
        hint: "For a non-LIMITS signal without any owned position, this TP stage ends the late grid. 1 prevents a fill only after TP1; 2/3 allow longer; 255 disables this rule. Explicit LIMITS are unchanged.",
      },
      pending_cancel_on_riskfree: {
        label: "RISK FREE cancels resting limits",
        hint: "Cancels unfilled basket orders when RISK FREE executes, preventing later fills from rebuilding exposure.",
      },
      bank_all_at_stage: {
        label: "bank the WHOLE basket from TP stage (0 = OFF)",
        hint: "At stage ≥ N the whole basket is closed (positions + pendings) instead of partials and runners. Tyler banks wholesale in the TP3 zone. Ambiguous in measurement (+930 $ / −478 $) — a sweep axis, not for production.",
      },
      stat_be_prog_usd: {
        label: "break-even threshold in reports, $",
        hint: "MEASUREMENT only — changes no decision. A trade with |result| ≤ the threshold counts as a draw instead of a loss; without it “OUT AT ENTRY” and “SL to BE” exits were dragging the win rate down. 0 = exact zero only (old tables unchanged).",
      },
    },
  },

  /* ================= PACKAGE F: LIVE-BOT BUGS ================= */
  pakiet_f: {
    title: "Message addressee and the SL-HIT brake",
    desc: "Axes covering unknown reply targets and the optional brake after source-reported stops.",
    fields: {
      reply_veto: {
        label: "a reply to an UNKNOWN message does not touch a basket",
        hint: "A reply to an unknown signal is ignored instead of falling back to the newest basket. A message without reply_to keeps the legacy fallback.",
      },
    },
  },

  /* ================= EXITS ================= */
  exits: {
    title: "Stagnation and emergency exits",
    desc: "Banking positions that stopped working, and exits on a reversal.",
    fields: {
      stale_take_min: {
        label: "STAGNATION — no new peak for",
        hint: "The oracle: median peak → close 51 min. A position in profit that made no new peak for X minutes is closed at market. 0 = OFF.",
      },
      stale_take_profit: { label: "at profit ≥" },
      stale_take_min2: {
        label: "second stagnation leg — after",
        hint: "Adds a second OR branch so either age/profit condition may trigger. Tune both branches on independent data.",
      },
      stale_take_profit2: { label: "at profit ≥" },
      rev_exit_range: {
        label: "REV-EXIT — reversal range",
        hint: "Exit on detected momentum reversal. 0 = OFF.",
      },
      rev_exit_slope: { label: "slope" },
      rev_exit_profit: { label: "min profit to exit" },
      rev_exit_window_min: {
        label: "measurement window",
        hint: "The range and the given-back edge are measured within this window.",
      },
    },
  },

  /* ================= SMART EXIT ================= */
  smartexit: {
    title: "Smart exit",
    desc: "What a human does watching the chart: take a large profit, flee on a sudden drop — but do NOT flee when your own limit waits just under the price.",
    fields: {
      smart_exit: {
        label: "ENABLE SMART EXIT",
        hint: "Off by default. Works alongside the ratchet and the targets — it does not replace them.",
      },
      smart_exit_take: {
        label: "take profit immediately from",
        hint: "0 = never close for this reason.",
      },
      smart_exit_giveback: {
        label: "close after giving back part of the peak",
        hint: "0.30 = 30% of the position's best result given back. 0 = OFF.",
      },
      smart_exit_min_peak: {
        label: "…but only from a peak of",
        hint: "Without this threshold the rule would cut positions that barely got into profit.",
      },
      smart_exit_drop_speed: {
        label: "close on a drop faster than",
        hint: "0 = ignore speed.",
      },
      smart_exit_speed_window_s: { label: "speed measurement window" },
      smart_exit_hold_if_pending: {
        label: "do NOT close when a limit waits closer than",
        hint: "The heart of the rule. A position falling towards its OWN grid is a different situation than one falling into a void: the limit will lower the basket average, and the comeback takes the whole thing green. 0 = ignore the grid.",
      },
      smart_exit_min_pendings: { label: "this many waiting limits suffice" },
      smart_exit_pending_scope: {
        label: "whose limits count",
        options: {
          SameBasket: "only this basket's (they average the price down)",
          AnyBasket: "any of ours (support for the price)",
        },
      },
      smart_exit_pending_min_dist: {
        label: "skip limits closer than",
        hint: "A limit right under the price is about to fill anyway, so it carries no “price has somewhere to return to” information.",
      },
    },
  },

  /* ================= VOLATILITY REGIME ================= */
  volregime: {
    title: "Volatility regime",
    desc: "Cut size in a storm — a smaller drawdown with a higher profit.",
    fields: {
      vol_window_min: {
        label: "range measurement window",
        hint: "The median 60-min range on XAUUSD is ~$19, so a threshold of 15 with a 30-min window is realistic. 0 = OFF.",
      },
      vol_range_usd: { label: "H−L range threshold" },
      vol_units_mult: { label: "units multiplier in a storm" },
    },
  },

  /* ================= CAPITAL PROTECTION ================= */
  guards: {
    title: "Capital protection and entry gates",
    desc: "Guards that stop the bot before a bad day turns into a catastrophe. The drawdown threshold is inactive at zero and must be armed explicitly.",
    fields: {
      max_dd_pct: {
        label: "MAX DRAWDOWN — CAPITAL GUARD",
        hint: "At the threshold the bot closes everything and blocks new signals. 0 = GUARD OFF, the account has no lower bound at all. All presets ship with 0 — this is the only place it can be armed. Removing the guard changed the KRATA preset's result from 520 to $2381 in long compounding, but it also changed the risk from bounded to unbounded.",
        warn: (s: Settings) =>
          s.max_dd_pct === 0 && s.max_dd_usd === 0
            ? "GUARD OFF — the bot will not stop trading at any drawdown. Enter a value in this field or in the dollar one to arm it."
            : null,
      },
      max_dd_usd: {
        label: "MAX DRAWDOWN in dollars",
        hint: "The same in dollars. Works independently of the percentage threshold — whichever trips first. 0 = this threshold inactive.",
      },
      alert_dd_pct: {
        label: "E-MAIL WARNING at drawdown",
        hint: "An e-mail only, WITHOUT halting trading — works also (and especially) with the guard off. Without it, at 0/0 thresholds the “drawdown” category would not send a single mail no matter what happened to the account overnight. Further warnings every +5 pp of deepening. 0 = no warnings.",
      },
      signal_max_age_min: {
        label: "MAX AGE OF AN OPENING SIGNAL",
        hint: "Rejects stale entry signals. Management messages, edits of existing baskets and explicit LIMIT/STOP signals valid until cancellation are exempt. 0 disables the age gate.",
      },
      dd_guard_scope: {
        label: "SCOPE OF THE POST-DRAWDOWN LOCK",
        hint: "The “all-time peak” variant with a 40% limit stopped the backtest on April 8 and the bot did not trade until the end of July — three days of data instead of four months. For a 24/7 bot the right answer to a bad day is a break until midnight.",
        options: {
          daily: "daily — from the day's peak, expires at midnight",
          lifetime: "lifetime — from the all-time peak, until resumed manually",
          lifetime_daily_reset: "from the all-time peak, lock until midnight",
        },
      },
      max_portfolio_risk_pct: {
        label: "OPEN-RISK CEILING — WHOLE ACCOUNT",
        hint: "A percentage of current equity covering position risk from the current price to SL and pending risk from entry to SL. Applies to every new order, even before the profit reserve arms. Reduces automatic volume; rejects an oversized manual order. 0 disables this limit.",
      },
      dd_soft_pct: {
        label: "THROTTLE — FIRST DRAWDOWN THRESHOLD",
        hint: "Above this drawdown the bot trades a SMALLER grid instead of not trading. The drawdown base is the same as the guard's above (the “lock scope” field). 0 = disabled.",
      },
      dd_soft_mult: {
        label: "…budget multiplier at the first threshold",
        hint: "0.5 = trade at half. The throttle does not touch positions already alive.",
      },
      dd_hard_pct: {
        label: "THROTTLE — SECOND DRAWDOWN THRESHOLD",
        hint: "Deeper drawdown, stronger throttling. 0 = disabled.",
      },
      dd_hard_mult: {
        label: "…budget multiplier at the second threshold",
        hint: "0.25 = trade at a quarter. The account keeps earning, just slower — so it has something to come back with.",
      },
      max_open_baskets: {
        label: "MAX OPEN BASKETS",
        hint: 'A limit on simultaneous setups, independent of the position limit. 0 = NO LIMIT (not "zero baskets").',
        warn: (s: Settings) =>
          s.max_open_baskets === 0
            ? '0 means ARBITRARILY MANY simultaneous setups, not "zero". Every basket is its own grid ' +
              "and its own margin — with a streak of one-way signals the account ends up with all its " +
              "volume on one side of the market. Want a ceiling — type a number."
            : null,
      },
      max_directional_lots: {
        label: "MAX VOLUME ONE WAY",
        hint: "Signals come in streaks and tend to be one-directional — without this limit the account stands with all its volume on one side of the market. 0 = no limit.",
      },
      equity_floor_pct: {
        label: "EQUITY FLOOR",
        hint: "Below this % of starting capital the bot opens nothing new. Open positions finish normally. 0 = disabled.",
      },
      regime_filter: {
        label: "MARKET REGIME FILTER",
        hint: "Trade only with the slope of an N-hour moving average — or exclusively against it.",
        options: { off: "disabled", trend: "with the trend", counter: "against the trend (fade)" },
      },
      regime_ma_hours: { label: "average window" },
      max_open_positions: {
        label: "EXPOSURE GUARD",
        hint:
          'Max number of simultaneously open positions. 0 = NO LIMIT (not "zero positions") — the guard ' +
          "then DOES NOT EXIST. Protects a small account from being zeroed in a sudden drawdown.",
        warn: (s: Settings) =>
          s.max_open_positions === 0
            ? '0 means ARBITRARILY MANY open positions at once, not "zero" — the guard is off. ' +
              "Filled pending orders can increase exposure unless they are counted by the companion setting. " +
              'Want the guard — type a number and enable "count pendings towards the limit".'
            : null,
      },
      exposure_count_pendings: { label: "count pendings towards the limit" },
      lot_scale_step: {
        label: "AUTO LOT-SCALING: +0.01 lots per",
        hint: "Compounding. E.g. 1000 → after $1000 of balance, lot 0.02. 0 = fixed lot.",
      },
      day_target_usd: {
        label: "DAILY TARGET",
        hint: "After +X $ on the day the bot opens nothing new. 0 = disabled.",
      },
      day_target_close: {
        label: "after the target CLOSE everything",
        hint: "Close positions and cancel pending orders after the daily target is reached.",
      },
      day_target_scale_lot: { label: "scale the target with the lot" },
      day_trail_basis: {
        label: "Daily stop basis",
        options: { equity_peak: "Peak equity · legacy", profit_peak: "Peak daily profit" },
        hint: "Equity: the drop is a percentage of the day's peak equity. Profit: it is a percentage of positive daily peak profit; the stop cannot arm before a positive profit exists. Selecting a basis does not enable the stop — set its threshold below.",
      },
      day_trail_stop_pct: {
        label: "Daily stop: allowed drop",
        hint: "Distance from the peak using the selected basis. 0 disables the percentage daily stop. For a $100 profit peak and 30% threshold, exit after giving back $30.",
      },
      day_trail_arm_pct: {
        label: "Daily stop: profit to arm",
        hint: "Minimum gain from the day-start capital before the percentage stop can activate. 0 removes this extra arm threshold; the profit basis still requires a positive gain.",
      },
      day_trail_stop_usd: {
        label: "DAY-TRAIL: close after a drop from the day peak",
        hint: "The automated version of the manual close. 0 = disabled.",
      },
      usd_scale_with_lot: {
        label: "MASTER: scale $ thresholds with the lot",
        hint: "All dollar thresholds multiplied by lot/0.01. Point thresholds scale naturally.",
      },
      eod_flat_hour: {
        label: "EOD-FLAT at hour",
        hint: "Close positions and cancel pending orders at this hour. The broker-clock setting selects the clock. 0 = disabled.",
      },
      flat_weekend: { label: "FLAT BEFORE THE WEEKEND", hint: "Protection against the weekend gap." },
      flat_weekend_hour: { label: "hour on Friday" },
      day_flat_broker_clock: { label: "broker clock instead of local" },
      session_filter: {
        label: "SESSION FILTER",
        hint: "Trade only within the given hours. Gold's hours differ in character.",
      },
      session_hours: { label: "session hours", hint: "Format: “7-20” or “7-11,13-20”." },
      streak_pause_n: {
        label: "PAUSE AFTER A LOSING STREAK — after N baskets",
        hint: "Signals come in streaks. 0 = disabled.",
      },
      streak_pause_min: { label: "pause length" },
      slhit_pause_n: {
        label: "SL-HIT BRAKE — after N of the CHANNEL's stops in a day",
        hint: "A different source than the losing-streak pause: it counts the channel's own “SL HIT” messages, so it may react before a loss materialises on the account. A very small N may suppress many later signals. 0 = disabled.",
      },
      slhit_pause_min: { label: "pause length (0 = until end of day)" },
      slhit_pause_lot_mult: {
        label: "SOFT brake — a lot multiplier instead of a block",
        hint: "0 = hard brake (entries blocked, as before). Above zero the signal DOES enter, but with a smaller lot — the soft-regime pattern. A hard block treats a weaker edge exactly like no edge and may discard too much trading on the strength of one message. With a basket risk cap (risk_per_basket_pct > 0), a base-lot multiplier can have no effect.",
      },
      signal_filter: {
        label: "SIGNAL QUALITY FILTER (tags)",
        hint: "Optionally classifies signals by explicit text tags such as “HIGH RISK TRADE” or “MAY NOT BE AROUND”.",
      },
      skip_tags: { label: "skip signals with tags" },
      require_tags: { label: "require tags" },
    },
  },

  /* ================= AUDIT ================= */
  parity: {
    title: "Engine audit and parity",
    desc: "Fixes for bugs uncovered in the bot ↔ Rust engine divergence forensics. All default to OFF = the old behavior.",
    fields: {
      commission_per_lot: {
        label: "COMMISSION PER LOT",
        hint: "Enter the commission charged by the target account. If commission is already embedded in the spread, keep this at zero; otherwise the backtest will overstate results.",
      },
      exec_latency_ms: {
        label: "EXECUTION LATENCY",
        hint: "Modeled time from message to order. Zero would mean the bot enters at the price from the moment the signaller was still pressing send.",
      },
      slippage_pts: {
        label: "SLIPPAGE of MARKET orders",
        hint: "Zero here means “WE DON'T KNOW”, not a guarantee of no slippage. Enter a conservative value based on execution at the target broker.",
      },
      server_tz_offset_h: {
        label: "SERVER TIME ZONE",
        hint: "Tick timestamps are recorded in the broker server's time. This field converts the message clock (UTC) to the tick clock — without it the engine receives the signal alongside a price stream from three hours earlier and “earns” on trades that live trading would never have seen.",
      },
      msg_clock_offset_h: {
        label: "message clock offset",
        hint: "Empty = use the server zone (correct when the Telegram export is in UTC). An explicit value helps when the message source has its own zone.",
      },
      lot_min: { label: "MINIMUM LOT", hint: "Strategy minimum, separate from the broker minimum and step. Contract V2 requires a positive value and does not round an infeasible volume up." },
      lot_max: { label: "MAXIMUM LOT", hint: "Single-order ceiling. 0 removes only this strategy limit; broker, risk and margin limits still apply. lot_max_z_salda provides an additional capital-based ceiling." },
      sim_stops_level: {
        label: "STOPS LEVEL (broker emulation)",
        hint: "A broker rejects SL/TP inside the symbol's minimum stop distance. Enter the target broker's value so simulations do not assume impossible levels.",
      },
    },
  },

  /* ================= AI ================= */
  ai: {
    title: "AI model",
    desc: "The trained BETAZERO neural network takes over full position management — direction still comes from the signal.",
    fields: {
      ai_model: { label: "Model" },
      ai_replaces_management: {
        label: "AI INSTEAD OF MANAGEMENT RULES",
        hint: "ENABLED = the model runs the position alone, and all management rules (TP ladder, trailing, risk free, guards) stop working. That is intended, but the name “enable AI” did not say so — it looked like “add AI” while it meant “remove the safeguards”. DISABLED = the model runs IN PARALLEL with the rules, and that is the right mode until the model beats the presets in dollars.",
        warn: (s: Settings) =>
          s.ai_replaces_management
            ? "Management rules are OFF — the position is run by the model alone."
            : null,
      },
      ai_decision_interval_s: {
        label: "how often the model decides (seconds)",
        hint: "The network was trained at a specific cadence. Changing this means the model sees the market differently than during training.",
      },
    },
  },

  /* ================= BONUS CREDIT ================= */
  kredyt: {
    title: "Bonus credit",
    desc:
      "MT5 reports balance and credit separately (e.g. $300 balance + $300 credit = $600 equity with no positions). These fields say " +
      "what the LOT is computed from; the credit itself remains a margin cushion. The three " +
      "numbers (balance / credit / lot base) are shown on the “Bot lot size” card.",
    fields: {
      credit_balance_separate: {
        label: "CREDIT SEPARATE FROM BALANCE (MT5 model)",
        hint: "ON: Balance excludes credit already, so do not subtract it twice. With credit deduction enabled, Equity excludes credit and MinOfBoth uses min(Balance, Equity minus credit). OFF restores the historical credit-in-balance model.",
      },
      odlicz_kredyt: {
        label: "DEDUCT BONUS CREDIT from the lot base",
        hint:
          "Enabled excludes the bonus from sizing. With separate credit, Balance is already your own cash; deduction applies to Equity/MinOfBoth. Disabled keeps the selected basis without credit deduction. Credit remains a cushion for equity and free margin.",
      },
      kredyt_reczny: {
        label: "credit amount (0 = AUTO from the terminal)",
        hint:
          "ZERO MEANS AUTO, not “there is no credit” — the bot then takes ACCOUNT_CREDIT " +
          "from MT5. Enter an amount only when the terminal does not report it, and REMEMBER " +
          "to zero it once the broker withdraws the bonus: otherwise the bot still reduces the Equity basis " +
          "(also Balance in the old model) and can trade an undersized lot. The panel " +
          "shows a mismatch with the terminal as a warning.",
      },
    },
  },

  /* ================= METATRADER 5 ================= */
  mt5: {
    title: "MetaTrader 5",
    desc: "Follow the account selected manually in MT5, or use the historical fixed-login mode. Following requires an already-running terminal; the bot does not launch or log it in.",
    fields: {
      mt5_follow_terminal_account: {
        label: "FOLLOW THE ACCOUNT SELECTED IN MT5",
        hint: "Attach without passing login, server or password. A manual account change rebuilds the bridge with a new identity. Automatically selects XAUUSD/XAUUSD.s; ambiguous terminal or symbol blocks trading. Autostart and watchdog are disabled in this mode.",
      },
      mt5_allow_real_account: {
        label: "ALLOW REAL ACCOUNTS in follow mode",
        hint: "Enable deliberately only if you want to trade real money on the account selected in MT5. Disabled prevents follow mode from sending orders to REAL after a manual switch. Does not govern historical fixed-login mode.",
        warn: (s: Settings) => s.mt5_allow_real_account ? "Selecting a REAL account can send real orders. Verify broker, login, preset and exposure." : null,
      },
      mt5_symbol: {
        label: "instrument symbol",
        hint: "Exactly as YOUR broker names it in Market Watch. It can be \"XAUUSD\", \"XAUUSD.m\", \"GOLD\". A wrong symbol = the bridge never comes up and the bot places not a single order.",
      },
      mt5_login: {
        label: "account number (login)",
        hint:
          "The bot will REFUSE to trade if the terminal is logged into a different account — " +
          "a red banner + mail instead of a quiet log line. " +
          "Empty (0) = no verification: the bot accepts ANY account, and the MT5 indicator " +
          "shows a yellow “UNVERIFIED”.",
        warn: (s: Settings) =>
          !s.mt5_login
            ? "Without an account number the bot trades on WHATEVER account the terminal is logged into. On real money that is the “I think I'm trading X but I'm trading Y” scenario."
            : null,
      },
      mt5_server: {
        label: "broker server",
        hint:
          "E.g. \"Broker-Demo\" or \"Broker-Live\". Together with the login " +
          "it lets the sidecar log the terminal into the right account at startup.",
      },
      mt5_password: {
        label: "account password (for headless login)",
        hint:
          "A DROP-BOX FIELD: on save the value goes to secrets.json (a separate file, owner-only " +
          "permissions) and disappears from here — it never lands in settings.json. " +
          "Empty = keep the old one / the terminal logs in with remembered credentials. " +
          "Needed only when the bot is to switch the terminal to the account above BY ITSELF.",
      },
      mt5_magic: {
        label: "magic number",
        hint: "The marker by which the bot recognizes ITS OWN orders. It does not touch positions with a different number — so one account can also hold manual trading or a second bot.",
        warn: (s: Settings) =>
          s.mt5_magic !== 770077
            ? `Magic ${s.mt5_magic} is NOT CONDUIT's default number (770077). Positions opened with a different number will be treated as FOREIGN and no longer managed — they will be left with just their stop-loss. Change it only deliberately, e.g. when running two instances on one account.`
            : null,
      },
      mt5_python: {
        label: "Python interpreter",
        hint: "Empty = \"python\" from PATH. Fill in a full path (e.g. C:\\Python313\\python.exe) when the MetaTrader5 package is installed in a different interpreter than the default.",
      },
      mt5_deviation_points: {
        label: "allowed slippage",
        hint: "How many price points the broker may deviate on a market order before rejecting it. Too little = refusals in a fast market.",
      },
      mt5_autostart: {
        label: "LAUNCH MT5 AT STARTUP",
        hint: "If the terminal is not running, the bot starts it itself right after the .exe launches.",
      },
      close_receipt_reconcile: {
        label: "RECONCILE DELAYED CLOSE RECEIPTS",
        hint: "Experimental account-wide ownership contract. Retains position ownership after an RPC close and reconciles delayed deals, including partial closes. OFF preserves the legacy path. Changing it requires reconnecting the bridge. This does not certify complete costs or durable recovery after restart.",
      },
      order_volume_contract_v2: {
        label: "STRICT BROKER VOLUME CONTRACT",
        hint: "Experimental account-wide contract. Enabled validates final volume against broker min/step/max and the strategy's calculated limit, flooring to the volume step. Rejects an infeasible volume instead of increasing it above the limit. Does not introduce a fixed lot cap or disable dynamic sizing. Disabled preserves the historical path.",
      },
      restore_strategy_continuation: {
        label: "RESTORE STRATEGY INTENTS — EXPERIMENTAL",
        hint: "OFF by default; an account/runtime option, not a profit-search parameter. Stage A restores pending SL/TP changes, deferred exits and the day-stop after reconciling account and broker positions. Incompatible or missing continuation on restore requires Review before new entries; protective exits remain available. This does not restore all strategy state, provide atomic persistence or durable receipt accounting. Enabling cannot retroactively repair old state files. OFF preserves the legacy restart path and its known limitations.",
        warn: (s: Settings) => s.restore_strategy_continuation
          ? "EXPERIMENT UNDER VALIDATION: three continuation contracts only. Full live restart is not qualified; ordinary Resume cannot replace reconciliation of incomplete state."
          : null,
      },
      closed_profit_net_costs: {
        label: "CLOSED NET PROFIT INCLUDING COSTS — EXPERIMENTAL",
        hint: "OFF by default; an account-wide accounting convention, not a strategy optimization. ON assigns complete signed entry/exit costs and swap to each closed slice exactly once, without charging the account balance again. Requires basket_realized_broker_only=true in every active preset and, for the MT5 adapter, close_receipt_reconcile=true. A visible switch does not certify adapter readiness: live execution and durable cost recovery require a separate qualification gate. Do not enable on a VPS before that verification. OFF preserves the historical, source-defined profit convention.",
        warn: (s: Settings) => s.closed_profit_net_costs
          ? "Experimental cost contract: the current stage is for Sim research. Live and restart fidelity are not yet verified; active-preset prerequisites require a separate gate."
          : null,
      },
      mt5_watchdog: {
        label: "WATCH AND RECOVER",
        hint: "Disabling keeps the autostart, but the bot stops reacting when the terminal is closed mid-run.",
      },
      mt5_terminal_path: {
        label: "path to terminal64.exe",
        hint: "Follow mode: empty requires one running terminal; an explicit path selects an already-running terminal, never launches it. Fixed-login mode: empty auto-detects the installation.",
      },
      mt5_retry_attempts: {
        label: "attempts per series",
        hint: "After a series is exhausted the bot waits 1 min, 3 min, 15 min, 30 min, 1 h, 2 h, 4 h, 8 h — and then every 8 h. It never gives up.",
      },
      mt5_retry_delay_s: { label: "delay between attempts" },
      mt5_restart_after: {
        label: "restart the app after N attempts",
        hint: "1 = the first attempt is “dry”, a restart only before the second. 0 = never restart. bot.py restarted the terminal before EVERY attempt — a momentary hiccup cost a full close-kill-launch cycle back then.",
        warn: (s: Settings) =>
          s.mt5_restart_after === 0 && s.mt5_watchdog
            ? "The bot will not restart the terminal — a hung MT5 stays hung."
            : null,
      },
      mt5_health_interval_s: { label: "health-check interval" },
      przerwa_dobowa_od_h: {
        label: "daily quote break FROM",
        hint:
          "An hour in the broker SERVER's time, fractional (1.083 = 01:05). In this window the lack " +
          "of ticks is the market sleeping, not a failure: the watchdog does not rebuild a healthy " +
          "bridge and does not send “Lost/Connected”. FROM equal to TO disables the break.",
      },
      przerwa_dobowa_do_h: {
        label: "daily quote break TO",
        hint:
          "Default 1.083 (= 01:05) — five minutes of headroom over gold's break, because " +
          "quotes come back unevenly after it. The window may cross midnight (e.g. 23.5 → 0.5).",
      },
      puls_h: {
        label: "PULSE: life report every N hours",
        hint:
          "An “alive: account, balance, bridge, signals” mail every N hours, even when nothing is " +
          "happening — because silence is indistinguishable from a stopped bot. Muted while the " +
          "market is closed (weekend, " +
          "daily break); the overdue pulse goes out after the open. 0 = disabled.",
      },
    },
  },

  /* ================= EVENT JOURNAL ================= */
  journal: {
    title: "Event journal",
    desc:
      "A JSON Lines stream next to the text log: one line = one event, with a full date and zone, " +
      "identifiers (basket / order / message), a state snapshot at every decision and a reason from a closed list. " +
      "It answers the questions the previous bot's log failed at: how much was made on a given day and which decision cost money.",
    fields: {
      journal_enabled: {
        label: "WRITE THE JOURNAL (.jsonl)",
        hint:
          "One file per broker trading day, date in the name: logs/journal/demo-YYYY-MM-DD.jsonl. " +
          "Analysis: loganaliza logs/journal. Disabling is irreversible — past events cannot be reconstructed.",
      },
      journal_min_level: {
        label: "lowest recorded level",
        hint: "debug also records messages that changed nothing. info is a sensible baseline; warn keeps only problems.",
        options: {
          debug: "debug — everything",
          info: "info — the bot's work",
          ok: "ok — successful actions and up",
          warn: "warn — warnings and errors only",
          error: "error — errors only",
        },
      },
      journal_snapshots: {
        label: "STATE SNAPSHOT AT DECISION",
        hint:
          "Bid, ask, spread, equity, balance, margin, position count, volume sum and the current drawdown at the moment of decision. " +
          "Without it there is no judging whether skipping a signal was right.",
      },
      journal_excursions: {
        label: "MEASURE PRICE EXCURSIONS (MFE / MAE)",
        hint:
          "The largest profit and largest loss the position showed during its life. The only source of the " +
          "“money left on the table” report — without it there is no telling which management rule costs the most.",
        warn: (s: Settings) =>
          s.journal_enabled && !s.journal_excursions
            ? "Without excursion measurement the “on the table” report will be empty — no way to answer whether a better close existed."
            : null,
      },
      journal_text_mirror: {
        label: "mirror .log file for the eye",
        hint: "The same event as a one-liner in Polish, next to the .jsonl file.",
      },
      journal_retention_days: {
        label: "keep days",
        hint: "0 = never delete. Counted by the date IN THE FILE NAME, not the modification time — copying the directory does not rejuvenate the days.",
      },
      archive_retention_days: {
        label: "MESSAGE ARCHIVE: keep days",
        hint: "Our own channel history (logs/wiadomosci/*.jsonl): every message and EVERY EDIT of it as a separate row. A Telegram export is no substitute — it collapses edits into the final version with an original marker and loses deleted messages. 0 = never delete.",
      },
      journal_buffer_cap: {
        label: "event buffer ceiling",
        hint: "How many events the core may hold between flushes to disk. Beyond it the oldest are dropped — explicitly, with a counter.",
      },
    },
  },

  /* ================= PANEL ================= */
  panel: {
    title: "Panel and execution",
    desc: "Interface behavior, display currency and the bot loop frequency.",
    fields: {
      one_click: { label: "ONE CLICK TRADING", hint: "The panel without confirmations and alerts." },
      display_currency: {
        label: "PANEL CURRENCY",
        hint: "Monetary values converted independently of MT5; prices/SL/TP stay in the instrument's price.",
        /* Lista walut jest IMPORTOWANA do schematu (`CURRENCIES`
           z `data/defaultSettings.ts`), więc statyczny skan kanarka jej
           nie widzi — te trzy etykiety trzeba było dopisać ręcznie.
           Reszta wpisów („USD — $") jest językowo neutralna. */
        options: {
          OFF: "no currency (plain numbers)",
          MT5: "from MT5 (account native)",
          PLN: "PLN — zł",
        },
      },
      poll_ms: { label: "Bot refresh", hint: "0 = maximum frequency (10 ms floor)." },
      price_tol: {
        label: "PRICE TOLERANCE",
        hint: "Matching TP/hit levels from signals to prices (different brokers = different spreads).",
      },
      show_positions_on_chart: { label: "Show positions on the chart" },
      show_potential_tpsl: {
        label: "SHOW POTENTIAL TP/SL",
        hint: "Potential profit/loss at TP and at SL — per position, in the sums and on the chart.",
      },
      exclude_pending_potential: { label: "EXCLUDE PENDING ORDERS from the sums" },
      comment_mode: {
        label: "POSITION COMMENT (MT5)",
        options: { source: "server / source name", custom: "custom text" },
      },
      comment_include_topic: { label: "append the topic name (forum)" },
      comment_custom: {
        label: "custom comment",
        hint: "MAX 18 CHARACTERS. The bridge appends a basket/grid suffix and a source label, while MT5 comments have a strict total-length limit. Text beyond the budget may be silently truncated or rejected by the API. Empty = the default marker.",
        warn: (s: Settings) => {
          const n = (s.comment_custom ?? "").trim().length;
          const budzet = 29 - 5 - 6;
          return n > budzet
            ? `${n - budzet} characters too long: the comment will be SILENTLY TRUNCATED on the account (the budget is ${budzet} characters; the rest is taken by the basket number and the source label).`
            : null;
        },
      },
    },
  },

  /* ================= CONDITIONAL EXITS ================= */
  exitrules: {
    title: "Conditional exits",
    desc: "Additional close conditions, independent of TP and SL. All disabled by default (0 = inactive).",
    fields: {
      exit_r_multiple: {
        label: "close after reaching R",
        hint: "A multiple of the risk (distance to SL). 2 = close when profit = 2× risk. 0 = disabled.",
      },
      exit_min_profit: {
        label: "minimum profit to exit",
        hint: "Exit conditions do not fire below this amount — protects against closing for free.",
      },
      exit_min_hold_min: {
        label: "minimum holding time",
        hint: "A position younger than this will not be closed by an exit condition.",
      },
      exit_on_opposite_signal: {
        label: "close on an opposite signal",
        hint: "A new signal in the other direction closes open positions of the previous direction.",
      },
      exit_spread_mult: {
        label: "exit a profitable position on a wide spread",
        hint: "Spread ≥ this multiple of its median triggers an exit from a profitable position (or exit-via-limit queue), NOT an exit block. Subject to exit_min_hold_min, exit_min_profit and hold_after_tp_hit_min. Zero disables this rule.",
      },
      exit_round_dist: {
        label: "round-number exit — reach",
        hint: "Close when price comes this close to a round level. 0 = disabled.",
      },
      exit_round_step: {
        label: "round-level step",
        hint: "How many dollars apart the “round” levels lie (default every 10).",
      },
    },
  },

  /* ================= GRID AND PENDINGS — DETAILS ================= */
  gridextra: {
    title: "Grid and pendings — details",
    desc: "Settings that decide WHERE exactly the limit grid lands and when pendings disappear. The difference between the RUNNER preset and the KRATA champion sits right here.",
    fields: {
      grid_anchor_absolute: {
        label: "GRID ON THE ABSOLUTE LATTICE",
        hint: "Limits placed on multiples of the step (1/PPM) rather than relative to the signal price — exactly what the old bot.py does. This is the defining trait of the KRATA preset. NOTE: the anchoring itself only works when the step (PPM) is enabled; multiplying orders on a level has had its own field since 18.08.",
      },
      units_per_level: {
        label: "lattice multiplies orders ON A LEVEL",
        hint: "On (default) = `entry_units` orders are placed on every lattice level. Off = exactly one order per level, independently of anchoring. This setting is separate from the lattice-step switch.",
      },
      pending_drop_arm: {
        label: "arm pending deletion",
        hint: "Pendings are deleted only after the arming condition is met, not immediately.",
      },
      ml_licz_wiszace: {
        label: "count margin WITH PENDING ORDERS",
        hint: "Margin level includes estimated margin reserved by pending orders, not only open positions, so gates account for the grid before it fills.",
      },
      ml_min_wejscie: {
        label: "min margin - NEW BASKET (%)",
        hint: "Below this margin level the bot opens no new basket. 0 = no gate. Forty profitable positions with SL at break-even are not a problem; two that wreck the margin are.",
      },
      ml_min_warstwa: {
        label: "min margin - NEXT GRID LAYER (%)",
        hint: "Below this level the bot adds no further grid layers to an existing basket. 0 = no gate.",
      },
      ml_min_reentry: {
        label: "min margin - RE-ENTRY (%)",
        hint: "Below this level the bot does not re-enter after a target is hit. 0 = no gate.",
      },
      ml_min_rearm: {
        label: "min margin - GRID RE-ARM (%)",
        hint: "Below this level the bot does not re-place unfilled grid levels. 0 = no gate.",
      },
      ml_min_piramida: {
        label: "min margin - PYRAMID (%)",
        hint: "Below this level the bot does not add to a winning position. 0 = no gate.",
      },
      ml_min_fast_addon: {
        label: "min margin - FAST ADD-ON (%)",
        hint: "Below this level the bot performs no fast add-on after a rapid fill. 0 = no gate.",
      },
      ml_min_relot_up: {
        label: "min margin - LOT INCREASE (%)",
        hint: "Below this level the bot does not raise the volume of existing orders. 0 = no gate.",
      },
      ml_min_drabina: {
        label: "min margin - MARKET LADDER RUNG (%)",
        hint: "Below this level the bot does not place the next rung of the market entry ladder. 0 = no gate.",
      },
      konto_dzwignia: {
        label: "account leverage (0 = from broker)",
        hint: "Leverage used to compute margin. 0 = take it from the broker account. Set manually only if the broker does not report it - a wrong value shifts EVERY margin gate.",
      },
      wiek_od_wypelnienia: {
        label: "basket age FROM FIRST FILL",
        hint: "The basket age clock starts at the first fill, not when the grid was created. A limit grid may wait a day and still be valid - timing it from publication kills setups that never got a chance.",
      },
      pending_drop_grace_min: {
        label: "grace window before dropping the grid (min)",
        hint: "After a target-reached-without-entry event the grid waits this many minutes before removal. 0 = remove immediately. Combine with the distance guard to limit stale exposure.",
      },
      pending_drop_grace_max_dist: {
        label: "grace window distance guard ($)",
        hint: "The grace window applies only while price is within this many dollars of the zone; beyond it the grid dies at once. 0 = no guard. This is the only feature that survived a permutation test (AUC 0.657): price returns in 78 % of cases at 2-3 $ from the zone and in 5 % beyond 20 $.",
      },
      pending_drop_keep_n: {
        label: "keep the N shallowest orders",
        hint: "When dropping the grid, keep this many orders closest to price. 0 = drop them all, as before. A third way between dropping everything (4 060 of 4 260 orders) and dropping nothing (-87 % and a wiped account).",
      },
      pending_drop_on_target: {
        label: "delete pendings once the target is reached",
        hint: "Removes unfilled limits after a basket target. Explicit LIMIT/STOP signals valid until cancellation are exempt; bot-generated grids for ordinary signals remain subject to this rule.",
      },
      pending_drop_require_zone_touch: {
        label: "require a zone touch",
        hint: "Without it the condition is only that price reached the target. For a pull-up signal this may already be true when the grid is placed and can remove pending orders before an entry.",
      },
      tp_open_extra: {
        label: "TP also for added positions",
        hint: "Positions added after the basket's start also receive a TP.",
      },
      hold_after_tp_hit_min: {
        label: "pause selected exit rules after a TP",
        hint: "Temporarily pauses R-multiple, round-number, spread and smart-exit rules after a TP. Does NOT block new entries, SL, harvest or every other exit. Zero disables this pause.",
      },
      basket_target_usd: {
        label: "basket target",
        hint: "Close the whole basket after reaching this result. 0 = disabled.",
      },
      toucher_tp_one_based: {
        label: "toucher TP counted from 1",
        hint: "Changes TP indexing for toucher positions (1 = the first TP instead of the zeroth).",
      },
      smart_sl_floor_be_after_rf: {
        label: "SL floor at BE after RISK FREE",
        hint: "After RISK FREE is declared, the smart SL never goes below the breakeven threshold.",
      },
    },
  },

  /* ================= RISK FREE AS A RULE ================= */
  riskfreerule: {
    title: "RISK FREE as a rule",
    desc: "The basket frees ITSELF after a profit threshold — without waiting for a channel message. This is an automat, not a message reaction.",
    fields: {
      riskfree_enabled: {
        label: "AUTOMATIC RISK FREE",
        hint: "Without this, the whole family below does nothing. Off by default: enabling changes the strategy, so it must be a deliberate decision.",
      },
      riskfree_trigger_usd: {
        label: "threshold in dollars",
        hint: "The basket profit at which the release happens. 0 = this condition inactive. Either of the two thresholds firing is enough.",
      },
      riskfree_trigger_r: {
        label: "threshold in R",
        hint: "The same threshold as a multiple of the basket risk (distance to SL). 0 = inactive. More robust to lot changes than the dollar threshold.",
      },
      riskfree_keep_units: {
        label: "how many positions stay as runners",
        hint: "The rest of the basket is closed. Runners are the main profit source in this system — zero means giving up the entire tail.",
      },
      riskfree_runner_stop: {
        label: "RUNNER STOP",
        hint: "A stop at each layer's own entry preserves the geometry of staggered entries. A basket-average stop can sit above a deeper runner's entry and close it prematurely.",
        options: {
          Be: "at the basket's weighted average",
          BeOwn: "at the layer's own entry",
          TrailGap: "start at BE, then a ratchet",
          Off: "no stop",
        },
      },
      riskfree_runner_gap: {
        label: "ratchet slack",
        hint: "Only for the “start at BE, then a ratchet” mode. Smaller values protect profit more tightly; larger values give the runner more room. Validate on independent windows.",
      },
      riskfree_be_offset: {
        label: "runner stop margin",
        hint: "Stop offset from the reference point. Positive = stop farther from price (looser), negative = closer.",
      },
      riskfree_runner_target: {
        label: "RUNNER TARGET",
        options: {
          KeepTp: "keeps the current one",
          LastTp: "the farthest target of the ladder",
          NextTp: "the next unhit one",
          NoTpTrailOnly: "no TP — trailing only",
        },
      },
      runner_max_hold_bez_reguly: {
        label: "the holding limit works ALSO without the RISK FREE rule",
        hint:
          "When enabled, the hold limit also covers baskets secured by a channel message while the autonomous RISK FREE rule is off.",
      },
      riskfree_runner_max_hold_min: {
        label: "wind the runner down after",
        hint: "Minutes counted from basket release, not from position open. 0 disables the time limit.",
      },
    },
  },

  /* ================= SIGNAL-DERIVED PARAMETERS ================= */
  adaptive: {
    title: "Signal-derived parameters",
    desc: "Instead of one constant for every signal — quantities derived from the zone width or the current volatility.",
    fields: {
      adaptive_params: {
        label: "ADAPTIVE PARAMETERS",
        hint: "The master switch of the whole family. Without it, the fields below do nothing even when set.",
      },
      sl_min_dist_zone_mult: {
        label: "SL = multiplier × zone width",
        hint: "0 = this source inactive. With both sources active, the larger value wins.",
      },
      sl_min_dist_atr_mult: {
        label: "SL = multiplier × window range",
        hint: "The H−L range from the ATR-substitute window. 0 = inactive.",
      },
      sl_min_dist_floor: { label: "floor of the computed SL" },
      sl_min_dist_cap: { label: "cap of the computed SL", hint: "0 = no cap." },
      entry_deep_zone_mult: { label: "deep entry = multiplier × zone" },
      entry_units_zone_ref: {
        label: "zone reference width",
        hint: "The rung count scales as signal width ÷ this value. 0 = no scaling.",
      },
      adaptive_atr_window_min: { label: "ATR-substitute window" },
      units_by_hour: {
        label: "rung multiplier by hour",
        hint: "Format “7-11:0.5,15-17:2” — the same end-exclusive range notation as the session filter. Validate hour multipliers across independent periods.",
      },
    },
  },

  /* ================= SIGNAL QUALITY AND BUDGET ================= */
  signalbudget: {
    title: "Trade budget and signal quality",
    desc: "How many signals may be taken per day, and which are worth it at all.",
    fields: {
      daily_signal_budget: {
        label: "BASKET LIMIT PER DAY",
        hint: "0 = no limit. Counted from the server's trading-day boundary, not local midnight.",
      },
      signal_min_rr: {
        label: "minimum signal R:R",
        hint: "Computed at the WORSE zone edge, so it is a pessimistic value. 0 = no filter.",
      },
      signal_min_zone_width: { label: "narrowest allowed zone" },
      signal_max_zone_width: { label: "widest allowed zone", hint: "0 = no upper bound." },
      entry_weights_from_rr: {
        label: "VOLUME WEIGHTS FROM RUNG R:R",
        hint: "Instead of the fixed “entry weights” ladder — volume proportional to each rung's own R:R.",
      },
      entry_weights_rr_power: {
        label: "bend exponent",
        hint: "0.5 = square root (gentle), 1 = directly proportional, 2 = square (aggressive).",
      },
      entry_weights_rr_cap: {
        label: "weight spread cap",
        hint: "How many times the largest weight may exceed the smallest.",
      },
    },
  },

  /* ================= BASKET MERGING AND RE-ARMING ================= */
  basketmerge: {
    title: "Basket merging and re-arming",
    desc: "What to do when the channel refines a signal in a separate message, or when price returns to the zone.",
    fields: {
      merge_same_side: {
        label: "MERGE SAME-SIDE SIGNALS",
        hint: "A new signal matching in direction and overlapping in zone UPDATES the live basket instead of creating a second one. Without this, the channel refining a zone doubles the exposure.",
      },
      merge_window_min: {
        label: "merge window",
        hint: "How old the basket may be for a new signal to count as its continuation.",
      },
      merge_min_overlap: {
        label: "required zone overlap",
        hint: "0–1, measured against the NARROWER zone. 0.5 means “half of the narrower zone lies inside the wider one”.",
      },
      basket_realized_broker_only: {
        label: "basket result from broker confirmations only",
        hint: "ON books closes only from the confirmed broker ledger, including partial closes. Prevents adding the same close result manually and again during synchronization. Affects rearm_min_basket_profit and other basket-result rules; it does not add profit to account balance. OFF preserves legacy accounting for comparison.",
      },
      rearm_grid_on_return: {
        label: "RE-ARM THE GRID ON A PRICE RETURN",
        hint: "Re-place the limits when price comes back into the zone after leaving it.",
      },
      confirmed_exit_retry: {
        label: "confirm basket exit and retry rejected closes",
        hint: "ON persists exit intent and finishes a basket only after the broker confirms no remaining positions or pending orders for that basket. Retries failed closes/cancellations at most once a second, including after restart with saved state. Blocks fresh entries and rearm in that basket until complete. Applies to managed baskets and does not extend authority over unrelated positions. OFF restores legacy exit behavior.",
      },
      defer_entry_until_receipts: {
        label: "DEFER A FULL ENTRY UNTIL CLOSES SETTLE",
        hint: "Experimental, OFF by default. Requires the global close_receipt_reconcile contract and a verified broker session. Only a full ENTRY is deferred during a temporary settlement barrier, not NOW. After the barrier clears, the signal must still pass validity checks. The queue lives only in session RAM and is not restored on restart. A permanent fault or unknown execution outcome is not automatically retried. Preset-owned.",
      },
      entry_edit_geometry_v2: {
        label: "EXPERIMENT: SOURCE AND PLAN EDITS V2",
        hint: "OFF preserves legacy editing. V2 separates the source signal, derived plan and confirmed execution: cosmetic edits must not change SL, TP, volume or progress. Real changes require revision and broker validation. Missing source snapshots or cancel/fill evidence require explicit review, not a guessed grid replacement. Implementation and validation are ongoing; this is not LIVE or restart certification.",
        warn: (s: Settings) => s.entry_edit_geometry_v2 ? "EXPERIMENT UNDER VALIDATION: complete cancel/fill, restart and native parity remain separate gates." : null,
      },
      deferred_entry_max_age_s: {
        label: "MAXIMUM TIME SINCE FIRST ENTRY RECEIPT",
        hint: "Seconds since the bot first received the signal (UTC), not Telegram publication or broker clock time. Edits do not extend the deadline. Must be finite and greater than zero. Zero does not mean unlimited or disabled; an invalid age cannot authorize a delayed entry. Other signal-validity rules still apply.",
      },
      rearm_keep_empty_alive: {
        label: "keep an empty basket alive until the return",
        hint: "After the first wave of positions and orders is gone, do not expire the basket after 60 seconds; retain its plan so a valid price return can re-arm the grid. Off preserves historical behavior.",
      },
      rearm_block_after_secured: {
        label: "do not re-arm after securing",
        hint: "Blocks another wave after RISK FREE / SPP, protecting a secured result from reopening exposure.",
      },
      spp_blocks_rearm_when_flat: {
        label: "SPP on a flat basket blocks re-arm",
        hint: "When an explicit SECURING PARTIAL PROFITS / DO NOT ENTER AGAIN arrives after positions have already closed, remember the veto for future re-arming without pretending a flat basket is a secured position.",
      },
      rearm_min_basket_profit: {
        label: "minimum basket result",
        hint: "We only add to a basket that is already earning — adding to a losing one is averaging down.",
      },
      rearm_max_times: { label: "times per basket", hint: "0 = no limit." },
      rearm_min_gap_min: { label: "shortest gap" },
    },
  },

  /* ================= EXIT VIA LIMIT ================= */
  exitlimit: {
    title: "Exit via limit",
    desc: "Discretionary exits (harvest, stagnation, smart exit) wait for the other side of the spread instead of paying it up front.",
    fields: {
      exit_via_limit: {
        label: "EXIT VIA LIMIT",
        hint: "Applies to DISCRETIONARY exits only. Stop losses and targets always go at market — waiting for a better price on an SL is a recipe for a bottomless loss.",
      },
      exit_limit_offset: { label: "beyond the other side of the spread" },
      exit_limit_wait_s: {
        label: "after how many seconds go to market",
        hint: "The emergency exit when the limit did not fill. Without it the exit may never happen.",
      },
      exit_limit_min_profit: {
        label: "below this profit exit immediately",
        hint: "A position barely in the green has nothing to fund the waiting with.",
      },
    },
  },

  /* ================= HIGHER-ORDER TREND FILTER ================= */
  trendfilter: {
    title: "Higher-order trend filter",
    desc: "Optional long-horizon trend filter. It can reduce directional exposure, but its effect must be validated out of sample on your own data.",
    fields: {
      trend_filter_enabled: {
        label: "TREND FILTER",
        hint: "Rejects or shrinks signals going against the trend measured over a long window. This is NOT the same as the “regime filter”, which looks at a moving average's slope.",
      },
      trend_filter_window_h: { label: "reference window", hint: "24 = a day, 168 = a week." },
      trend_filter_drop_pct: {
        label: "price-change threshold",
        hint: "From what change in the window we deem the trend opposite to the signal. 0.5 = “gold fell half a percent”. 0 = the filter inactive despite the switch.",
      },
      trend_filter_mode: {
        label: "what to do with a counter-trend signal",
        hint: "“Smaller size” is an EQUAL variant, not a fallback: a hard block against the dominant signal direction can remove most trading, so the test no longer isolates the filter.",
        options: { Shrink: "enter at a smaller size", Block: "do not enter at all" },
      },
      trend_filter_shrink: { label: "size multiplier", hint: "0.5 = half the units." },
      basket_max_age_min: {
        label: "HARD BASKET LIFETIME",
        hint: "After this many minutes the basket is wound down at market together with unfilled limits — including one that never reached the RISK FREE threshold. Rationale: the signal's edge lives about an hour; a basket held longer is a directional position, not signal execution. Sensible range 15–180. 0 = disabled.",
      },
    },
  },

  /* ================= BROKER COST MODEL ================= */
  kosztybrokera: {
    title: "Broker cost model",
    desc: "This is a broker-cost model, not strategy. Disabling swap can overstate a backtest; enter values reported by the broker for the target account and symbol.",
    fields: {
      swap_enabled: {
        label: "CHARGE SWAP",
        hint: "Charges broker-configured swap points for each rollover while a position remains open. Disable only for an explicit cost-isolation experiment.",
      },
      swap_long_points: {
        label: "swap of a LONG position",
        hint: "Negative values represent a nightly cost. Copy the current long-swap value from the broker specification.",
      },
      swap_short_points: {
        label: "swap of a SHORT position",
        hint: "Positive values represent nightly income; negative values represent a cost. Long and short swap may differ materially.",
      },
      swap_point_value: {
        label: "swap point value",
        hint: "Cash value of one swap point per lot. Obtain it from the broker's symbol specification or a controlled demo measurement.",
      },
      swap_rollover_weekday: {
        label: "day ENTERED on the triple charge (0 = Mon, 3 = Thu)",
        hint: "The field names the day entered at the triple rollover, not the day whose financing is covered. Confirm the broker convention before setting it manually.",
      },
      swap_rollover_mult: { label: "multiplier on that day" },
      swap_pomijaj_weekend: {
        label: "do not charge the Saturday and Sunday nights",
        hint: "Triple rollover normally settles weekend financing in advance. Enable this to avoid charging Saturday and Sunday again in the simulation.",
      },
      swap_rollover_z_serwera: {
        label: "derive the rollover day from the server value",
        hint: "Use the raw SYMBOL_SWAP_ROLLOVER3DAYS value reported by the terminal; the engine converts it to its internal weekday convention.",
      },
      swap_rollover3days_mt5: {
        label: "SYMBOL_SWAP_ROLLOVER3DAYS from the terminal (0 = Sun)",
        hint: "Raw MT5 value, where the week starts on Sunday. Read only when deriving rollover from the server is enabled.",
      },
      runner_ksiegowanie_v2: {
        label: "day boundary: fixed bookkeeping and ordering",
        hint: "Three backtest-loop fixes at once. (1) Closures at the day boundary — overnight SL/TP from the gap and the whole EodFlat — counted towards NO day, so daily mode, the main preset-selection criterion, had an understated trade count. (2) EodFlat landed in the NEW engine and inflated its losing streak, so the day started with a pause for yesterday's flatten. (3) At the boundary the broker executed BEFORE the overnight messages (the opposite of a normal tick), and with a daily reset it counted the same tick three times in the margin-level counters. Off = numbers consistent with the whole archive.",
      },
      msg_kurs_sprzed_luki: {
        label: "a message does not see the price across the gap",
        hint: "A message that arrived BETWEEN ticks was given the NEXT tick's quote — so during the daily and weekend break it decided while already knowing the price after the gap. That is knowledge from the future and it inflates the backtest; the live bot does the opposite. Enabling it hands such messages the quote from before the break.",
      },
      slippage_pending_pts: {
        label: "slippage of PENDING orders",
        hint: "Expected slippage for pending-order fills, in points. Keep zero only when supported by the target broker's execution data; otherwise enter a conservative value.",
      },
      stop_out_level_pct: {
        label: "STOP OUT LEVEL",
        hint: "Below this margin level the broker closes positions ITSELF — the single most losing one first, recomputing the level after each. With a grid of many small positions that is a completely different trajectory than closing everything at once, and it concerns the only absolute threshold: “the account must not be zeroed”.",
      },
      margin_call_level_pct: {
        label: "margin call level",
        hint: "⚠ NOT MODELED YET. The field reaches the engine, but the engine does not read it today — the bot will NOT stop opening positions at this level. Stop out (the field above) works for real. The control stays because the value is part of the account's description, but changing this number changes nothing in the bot's behavior today.",
      },
    },
  },

  /* ================= EA LAYER (AUTO-EA) ================= */
  ea_layer: {
    title: "EA layer (AUTO-EA)",
    desc:
      "Signal-driven EA: entries, direction and key levels (TP, SL) come from trader signals, " +
      "while the EA layer runs the positions with professional-EA precision and reacts to the " +
      "channel's management messages (BE, partials, close) when needed. No signals = no trades — " +
      "the EA never opens anything on its own. Below is the layer's SKELETON (EA-CORE): its own " +
      "clock independent of the quote stream, a Defense/Neutral/Offense machine with two-sided " +
      "hysteresis, and a ratchet. The trading axes themselves (families A–G) will land HERE as " +
      "further preset fields. Today the skeleton CHANGES NOT A SINGLE NUMBER — and that is its " +
      "acceptance gate, not a shortcoming.",
    fields: {
      ea_enabled: {
        label: "EA LAYER — MASTER SWITCH",
        hint:
          "Off = the engine takes today's path, not one extra operation, and the account is not read a single time more. " +
          "On with all zeros below = the layer runs (pulses, reads account state, stamps baskets, drives the state machine) " +
          "but CHANGES NOT A SINGLE NUMBER — so that the cost of the skeleton can be measured separately from the cost of the policies.",
      },
      ea_tick_s: {
        label: "management clock",
        hint:
          "0 = no own clock: the layer wakes on every quote, exactly like today's rules. " +
          "Above zero enables a cadence — a pulse fires once this many seconds have passed since the previous one, REGARDLESS of whether a tick or the clock woke it. " +
          "Why: the previous bot had an exit rule frozen for 12 h because it woke only from the stream — and a backtest does NOT SEE that class of bug. " +
          "Quote silence is a market state, not a failure.",
      },
      ea_state_src: {
        label: "state signal source",
        hint: "What “how bad is it right now” is computed from. R = the sum of the live baskets' original risk, so the measure does not depend on account size.",
        options: {
          FloatR: "FloatR — floating result / baskets' R (recommended)",
          FloatPctEquity: "FloatPctEquity — floating result as % of equity",
        },
      },
      ea_defense_enter: {
        label: "enter DEFENSE at a loss ≥",
        hint:
          "0 = DEFENSE NEVER (not “defense at zero loss”). Defense is a state of the WHOLE PORTFOLIO: a loss never raises the risk of the whole. " +
          "Units follow the “state signal source” field: R or % of equity.",
      },
      ea_defense_exit: {
        label: "leave defense at a loss ≤",
        hint: "Must be LESS severe than the entry threshold — that is the entire hysteresis. A signal oscillating between the thresholds does not flip the state.",
      },
      ea_offense_enter: {
        label: "enter OFFENSE at a profit ≥",
        hint:
          "0 = OFFENSE NEVER. Offense is a state of a SINGLE BASKET, not of the portfolio: the winner runs alone and does not license its colleagues. " +
          "This is a direct translation of the Synergy canon — a win is not a pass for the next setup.",
      },
      ea_offense_exit: {
        label: "leave offense at a profit ≤",
        hint: "The other side of the hysteresis, this time on the profit side.",
      },
      ea_state_dwell_s: {
        label: "minimum state duration",
        hint:
          "Anti-flapping. 0 = no requirement. THE ASYMMETRY IS BUILT IN AND NOT CONFIGURABLE: tightening (entering a more cautious state) " +
          "acts IMMEDIATELY, loosening requires holding the condition for this long. Protection does not wait for a clock.",
      },
      ea_state_ratchet: {
        label: "state ratchet",
        hint:
          "What happens to a basket opened in a more cautious state once the portfolio calms down. " +
          "Ratchet = the basket lives out its state; only NEW baskets get the slack. Cleared when the basket closes.",
        options: {
          NieLuzujWKoszyku: "Do not loosen inside an open basket (recommended)",
          Swobodny: "Free — the state applies to everything [only to measure the ratchet's cost]",
        },
      },
      ea_state_journal: {
        label: "state-change journal",
        hint: "Every state change with its reason, the signal value and the pulse SOURCE (tick or clock). Changes no decision — an in-memory buffer with a hard cap.",
      },
      ea_dozor_sl: {
        label: "SL WATCHDOG — place missing stops",
        hint:
          "Off (default) = the layer only COUNTS positions without a stop-loss at the broker; not one modification goes to the account. " +
          "On = a position without an SL receives ITS BASKET'S SL. The watchdog NEVER invents a risk level: a position whose basket has no stop either " +
          "ends up as an incident, not a fabrication. Positions frozen by manual edit and tickets with no basket are untouchable. " +
          "Closes the “late fill / restart left a position without a stop” class of bug.",
      },
    },
  },

  engine_entries: {
    title: "Entries — full geometry",
    desc: "Advanced preset axes. Edits apply to the selected preset; reveal dependent fields or use search.",
    fields: {
      drop_unplaceable_levels: {
        label: "Drop unplaceable levels"
      },
      enforce_position_limit_on_fill: {
        label: "Enforce position limit on fill"
      },
      entry_jeden_na_glebokiej: {
        label: "One entry at the deep edge"
      },
      entry_krzywa_kotwica: {
        label: "Entry curve anchor",
        options: {
          Ocalaly: "Surviving",
          Original: "Original"
        }
      },
      entry_uklad: {
        label: "Position counts by entry level",
        hint: "Position count at successive levels, e.g. 1,2,1. Each value is 0–9; empty or all-zero keeps the default layout."
      },
      entry_uklad_kotwica: {
        label: "Entry layout anchor",
        options: {
          Ocalaly: "Surviving",
          Original: "Original"
        }
      },
      entry_warstwy_offset: {
        label: "Entry layer offset"
      },
      entry_warstwy_z_tekstu: {
        label: "Read entry layers from message"
      },
      fast_fill_layers: {
        label: "Fast fill layer count"
      },
      fast_fill_reject_s: {
        label: "Fast fill rejection time threshold"
      },
      fast_fill_soft_age_min: {
        label: "Fast fill soft age threshold"
      },
      honor_stop_orders: {
        label: "Honor explicit STOP orders",
        hint: "An explicit STOP keeps its STOP order type. Does not enable market signals or disable account protection."
      },
      limit_kasuje_tylko_nadmiar: {
        label: "Limit cancels only excess orders"
      },
      market_entry_mode: {
        label: "Market entry execution mode",
        hint: "Controls market entries. Does not set the validity of an explicit LIMIT/STOP signal.",
        options: {
          GridAtOnce: "Whole grid at once",
          Single: "Single entry",
          Laddered: "Laddered"
        }
      },
      pending_cross_policy: {
        label: "When price crossed a pending entry",
        options: {
          Market: "Market",
          Stop: "Stop",
          Shift: "Shift",
          Skip: "Skip"
        }
      },
      units_per_level_zone: {
        label: "Count units per zone level"
      },
      zakaz_ponizej_krawedzi: {
        label: "Block entries beyond the edge"
      }
    }
  },

  engine_addons: {
    title: "Addons, pyramids and re-entry",
    desc: "Advanced preset axes. Edits apply to the selected preset; reveal dependent fields or use search.",
    fields: {
      fast_addon_cooldown_s: {
        label: "Momentum addon cooldown"
      },
      fast_addon_lot_mult: {
        label: "Momentum addon lot multiplier"
      },
      fast_addon_max: {
        label: "Maximum momentum addons"
      },
      fast_addon_min_stage: {
        label: "Momentum addons from TP stage"
      },
      fast_addon_move_usd: {
        label: "Required move for momentum addon"
      },
      fast_addon_window_s: {
        label: "Momentum addon lookback"
      },
      no_reenter_from_stage: {
        label: "Block re-entry from TP stage"
      },
      pyramid_after_stage: {
        label: "Pyramid from TP stage"
      },
      pyramid_lot_mult: {
        label: "Pyramid lot multiplier"
      },
      pyramid_min_equity_mult: {
        label: "Pyramid equity threshold"
      },
      pyramid_regime_lookback: {
        label: "Pyramid regime lookback"
      },
      pyramid_regime_max_fast_pct: {
        label: "Pyramid maximum fast-market share"
      },
      rearm_bez_pozycji: {
        label: "Rearm a basket without positions"
      },
      rearm_bez_pozycji_max_h: {
        label: "Maximum age for empty-basket rearm"
      },
      reenter_min_return_s: {
        label: "Re-entry minimum return time"
      },
      reenter_respect_cap: {
        label: "Re-entry respects the cap"
      },
      reenter_stop_after_riskfree: {
        label: "Block re-entry after RISK FREE"
      }
    }
  },

  engine_small: {
    title: "Small-account thresholds",
    desc: "Advanced preset axes. Edits apply to the selected preset; reveal dependent fields or use search.",
    fields: {
      basket_max_age_min_small: {
        label: "Basket max age: small account"
      },
      basket_max_age_min_small_mult: {
        label: "Capital threshold: basket age",
        hint: "Threshold = starting capital × multiplier. 0 disables the small-account variant."
      },
      entry_units_small: {
        label: "Entry units: small account"
      },
      entry_units_small_mult: {
        label: "Capital threshold: entry units",
        hint: "Threshold = starting capital × multiplier. 0 disables the small-account variant."
      },
      fast_fill_soft_age_min_small: {
        label: "Fast fill age for small account"
      },
      fast_fill_soft_age_min_small_mult: {
        label: "Capital threshold: fast fill age",
        hint: "Threshold = starting capital × multiplier. 0 disables the small-account variant."
      },
      market_entry_step_small: {
        label: "Market spacing: small account"
      },
      market_entry_step_small_mult: {
        label: "Capital threshold: market spacing",
        hint: "Threshold = starting capital × multiplier. 0 disables the small-account variant."
      },
      max_open_baskets_small: {
        label: "Maximum baskets: small account"
      },
      max_open_baskets_small_mult: {
        label: "Capital threshold: maximum baskets",
        hint: "Threshold = starting capital × multiplier. 0 disables the small-account variant."
      },
      max_open_positions_small: {
        label: "Maximum positions: small account"
      },
      max_open_positions_small_mult: {
        label: "Capital threshold: maximum positions",
        hint: "Threshold = starting capital × multiplier. 0 disables the small-account variant."
      },
      reenter_max_small: {
        label: "Maximum re-entries: small account"
      },
      reenter_max_small_mult: {
        label: "Capital threshold: maximum re-entries",
        hint: "Threshold = starting capital × multiplier. 0 disables the small-account variant."
      },
      risk_per_basket_pct_small: {
        label: "Basket risk: small account"
      },
      risk_per_basket_pct_small_mult: {
        label: "Capital threshold: basket risk",
        hint: "Threshold = starting capital × multiplier. 0 disables the small-account variant."
      },
      sl_min_dist_small: {
        label: "Minimum SL distance: small account"
      },
      sl_min_dist_small_mult: {
        label: "Capital threshold: minimum SL",
        hint: "Threshold = starting capital × multiplier. 0 disables the small-account variant."
      }
    }
  },

  engine_regime: {
    title: "Regime and volatility sizing",
    desc: "Advanced preset axes. Edits apply to the selected preset; reveal dependent fields or use search.",
    fields: {
      regime_cena: {
        label: "Regime reference price",
        options: {
          Rynkowa: "Market",
          Wejscia: "Wejscia",
          Obie: "Obie"
        }
      },
      regime_gdy_rozerwany: {
        label: "Regime behavior on disagreement",
        options: {
          Milcz: "No action",
          KrotkieOkno: "KrotkieOkno",
          Miekko: "Miekko"
        }
      },
      regime_miara: {
        label: "Regime measure",
        options: {
          Srednia: "Mean",
          Mediana: "Mediana",
          Kanal: "Kanal",
          Wykladnicza: "Wykladnicza",
          Percentyl: "Percentyl"
        }
      },
      regime_okno2_h: {
        label: "Second regime window"
      },
      regime_percentyl: {
        label: "Regime percentile"
      },
      regime_pilnuj_limitow: {
        label: "Apply regime to existing limits"
      },
      regime_soft: {
        label: "Soft regime adaptation"
      },
      regime_soft_lot_mult: {
        label: "Soft regime lot multiplier"
      },
      regime_soft_max_positions: {
        label: "Soft regime maximum positions"
      },
      regime_soft_risk_mult: {
        label: "Soft regime risk multiplier"
      },
      regime_soft_units_mult: {
        label: "Soft regime unit multiplier"
      },
      regime_strefa_martwa: {
        label: "Regime dead zone"
      },
      regime_zmiennosc_max: {
        label: "Regime maximum volatility"
      },
      regime_zmiennosc_min: {
        label: "Regime minimum volatility"
      },
      vol_size_max_mult: {
        label: "Volatility sizing: maximum multiplier"
      },
      vol_size_min_mult: {
        label: "Volatility sizing: minimum multiplier"
      },
      vol_size_mode: {
        label: "Volatility sizing mode",
        options: {
          Off: "Off",
          Target: "Volatility target",
          Percentile: "Percentile"
        }
      },
      vol_size_odsezonuj: {
        label: "Remove volatility seasonality"
      },
      vol_size_percentile_okno: {
        label: "Volatility percentile window"
      },
      vol_size_target: {
        label: "Target volatility range"
      }
    }
  },

  engine_targets: {
    title: "Targets, BE and runner management",
    desc: "Advanced preset axes. Edits apply to the selected preset; reveal dependent fields or use search.",
    fields: {
      be_min_pozycji: {
        label: "BE: minimum positions"
      },
      be_od_etapu: {
        label: "BE: starting TP stage"
      },
      cel_z_przeciwnego: {
        label: "Target from opposite signal",
        options: {
          Off: "Off",
          DalszaKrawedz: "DalszaKrawedz",
          Srodek: "Srodek"
        }
      },
      cel_z_przeciwnego_zapas: {
        label: "Opposite target buffer"
      },
      cele_pomin_za_cena: {
        label: "Skip targets behind current price"
      },
      no_tp_after_stage: {
        label: "Remove TP from stage"
      },
      oae_pod_woda: {
        label: "OUT AT ENTRY while losing",
        options: {
          NicNieRob: "No action",
          Zamknij: "Close",
          DociagnijStop: "Tighten stop"
        }
      },
      oae_skip_after_riskfree: {
        label: "Skip OUT AT ENTRY after RISK FREE"
      },
      runner_cele_krok: {
        label: "Runner target spacing"
      },
      runner_cele_n: {
        label: "Runner target count"
      },
      runner_max_hold_rule_only: {
        label: "Runner age limit from rule only"
      },
      runner_partial_pct: {
        label: "Runner partial close"
      },
      sl_po_tp1_na_krawedz: {
        label: "Move SL to edge after TP1"
      },
      sl_wlasny_na_pozycje: {
        label: "Individual stop per position"
      },
      spp_arms_runner_clock: {
        label: "SPP arms the runner clock"
      },
      spp_sl_mode: {
        label: "SPP stop-loss mode",
        options: {
          Off: "Off",
          Stop: "Stop",
          OnlyIfBetter: "Only a better SL",
          RunnersOnly: "Runners only",
          RunnersOnlyIfBetter: "Runners, only a better SL",
          BankersOnly: "Bankers only"
        }
      },
      spp_sl_pad: {
        label: "SPP stop-loss buffer"
      },
      tp_drabinka_kotwica: {
        label: "Target ladder anchor",
        options: {
          Ocalaly: "Surviving",
          Original: "Original"
        }
      }
    }
  },

  engine_sr: {
    title: "S/R trailing — structure parameters",
    desc: "Advanced preset axes. Edits apply to the selected preset; reveal dependent fields or use search.",
    fields: {
      trail_atr_mult: {
        label: "Trailing ATR multiplier"
      },
      trail_sr_atr_period: {
        label: "S/R ATR period"
      },
      trail_sr_fractal_n: {
        label: "S/R fractal width",
        hint: "Candle count required to confirm a local extreme. A larger value confirms structure later."
      },
      trail_sr_min_dist_tp: {
        label: "S/R minimum distance from TP"
      },
      trail_sr_min_prominence_atr: {
        label: "S/R minimum prominence in ATR"
      },
      trail_sr_offset: {
        label: "S/R fixed buffer"
      },
      trail_sr_offset_atr_mult: {
        label: "S/R buffer in ATR multiples"
      },
      trail_sr_offset_spread_mult: {
        label: "S/R buffer in spread multiples"
      },
      trail_sr_struct_window_h: {
        label: "S/R structure window"
      },
      trail_sr_tf_min: {
        label: "S/R candle timeframe",
        hint: "S/R structure candle timeframe in minutes. Used while S/R trailing is enabled."
      }
    }
  },

  engine_messages: {
    title: "Messages — interpretation and consistency",
    desc: "Advanced preset axes. Edits apply to the selected preset; reveal dependent fields or use search.",
    fields: {
      hint_veto: {
        label: "Recipient hint may veto an action"
      },
      live_tick_order_strict: {
        label: "Strict live tick ordering"
      },
      parser_geometryczny: {
        label: "Signal geometry parser"
      },
      parser_luz_interpunkcyjny: {
        label: "Tolerate punctuation variants"
      },
      parser_min_pewnosc: {
        label: "Minimum parser confidence"
      },
      recap_guard: {
        label: "Recognize trade recaps"
      },
      reply_graph_transitive: {
        label: "Follow transitive Telegram replies"
      },
      rf_level_sanity_max_usd: {
        label: "Maximum RISK FREE level distance"
      },
      sanity_tp_max: {
        label: "Maximum TP distance"
      },
      sanity_tp_rosnace: {
        label: "Require ordered TP ladder"
      },
      sanity_tp_strona: {
        label: "Validate TP direction"
      },
      sanity_zone_max: {
        label: "Maximum entry zone width"
      },
      sl_edit_reaches_pendings: {
        label: "Apply SL edits to pending orders"
      },
      sync_only_live_levels: {
        label: "Synchronize only live levels"
      },
      tp_correction_to_broker: {
        label: "Send TP corrections to broker"
      },
      tp_hit_match_level: {
        label: "Match TP HIT to price level"
      },
      tp_unindexed_pips_require_price: {
        label: "Unindexed TP requires price confirmation"
      }
    }
  },

  engine_ea: {
    title: "EA layer — capital and addons",
    desc: "Advanced preset axes. Edits apply to the selected preset; reveal dependent fields or use search.",
    fields: {
      ea_lot_z_wolnego_marginesu: {
        label: "EA: sizing from free margin"
      },
      ea_redukcja_przy_zageszczeniu: {
        label: "EA: density reduction"
      },
      ea_stan_dnia: {
        label: "EA: daily state mode",
        options: {
          Off: "Off",
          TylkoInkaso: "Profit taking only"
        }
      },
      ea_stan_dnia_jednostki_mult: {
        label: "EA: units multiplier after daily threshold"
      },
      ea_stan_dnia_prog_sl: {
        label: "EA: SL threshold for daily state"
      },
      ea_stop_dokladek_powrot: {
        label: "EA: resume after addon pause"
      },
      ea_stop_dokladek_przy_stracie: {
        label: "EA: pause addons on loss"
      },
      ea_zageszczenie_podloga: {
        label: "EA: density floor"
      }
    }
  },

  engine_risk: {
    title: "Protection — gates and exposure",
    desc: "Advanced preset axes. Edits apply to the selected preset; reveal dependent fields or use search.",
    fields: {
      profit_budget_arm_pct: {
        label: "New-entry budget: daily profit threshold",
        hint: "Arms from peak daily profit relative to starting capital. 0 disables only the profit reserve; the separate portfolio risk ceiling still applies. Limits new entries; does not close existing positions."
      },
      profit_budget_keep_pct: {
        label: "Retain a share of peak profit",
        hint: "Budget basis = starting capital + this share of peak profit. This does not guarantee an equity floor during gaps."
      },
      profit_budget_deploy_pct: {
        label: "Use available headroom",
        hint: "Share of equity above the basis, minus the downside of open positions and pending orders to SL. New lots are rounded to the broker step; entry is rejected if the minimum does not fit."
      },
      day_gate_do_salda: {
        label: "Daily gate: upper equity boundary",
        hint: "Day-start equity must be below this value. 0 removes the upper boundary."
      },
      day_gate_od_salda: {
        label: "Daily gate: lower equity boundary",
        hint: "Day-start equity must be at least this value. 0 removes the lower boundary."
      },
      exposure_bonus_baskets: {
        label: "Extra baskets after profit threshold"
      },
      exposure_bonus_positions: {
        label: "Extra positions after profit threshold"
      },
      exposure_bonus_profit_pct: {
        label: "Profit threshold for extra exposure"
      },
      lot_max_z_salda: {
        label: "Capital per lot of order limit",
        hint: "Single-order limit = capital basis / this value. 0 disables this limit. lot_max, risk budget and broker constraints still apply."
      },
      sesja_bramka: {
        label: "Session gate scope",
        options: {
          Sygnal: "Signal",
          Wypelnienie: "Wypelnienie",
          Oba: "Oba"
        }
      },
      zone_exit_adverse_close: {
        label: "Close on adverse zone exit"
      },
      zone_exit_adverse_s: {
        label: "Adverse zone exit delay"
      }
    }
  },

  engine_broker: {
    title: "Account — execution and margin",
    desc: "Advanced account-wide settings. They do not replace symbol specifications received from the broker.",
    fields: {
      lot_base: {
        label: "Capital basis for position sizing",
        hint: "Shared by the account. Balance, equity or their minimum; credit deduction still follows the bonus settings.",
        options: { Balance: "Balance", Equity: "Equity", MinOfBoth: "Minimum of balance / equity" },
      },
      expo_cap_close: {
        label: "Close on exposure breach"
      },
      expo_cap_ml_pct: {
        label: "Minimum margin level for exposure"
      },
      expo_cap_pct: {
        label: "Account exposure percentage cap"
      },
      expo_cap_s: {
        label: "Exposure check interval"
      },
      sim_margin_at_market: {
        label: "Simulation: margin at market price"
      },
      sim_margin_check_on_fill: {
        label: "Simulation: margin check on fill"
      },
      sim_validate_pending_stops: {
        label: "Simulation: validate pending stops"
      }
    }
  },
};
