//! Metamorphic S/R tests: parent-OFF, real tick/warmup basis, causal state.
//! No profitability claim. A `legacy_*` test can document a parity defect.
use conduit_backtest::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage, SrWarmupBar};
use conduit_core::settings::*;
use conduit_core::types::*;
use serde_json::{json, Value};

const T: Ts = 1_800_000_000_000;
fn config() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3; c.lot_mode_percent = false; c.lot_fixed = 0.04;
    c.risk_per_basket_pct = 0.0; c.max_portfolio_risk_pct = 0.0;
    c.tp_source = TpSource::SignalOnly; c.tp_schedule = TpSchedule::AllRunners;
    c.pending_lifetime = PendingLifetime::Never; c.pending_ttl_h = 0.0;
    c.trail_mode = TrailMode::Off; c.trail_split = false;
    c.risk_free_trail = false; c.riskfree_enabled = false;
    c.trail_sr_enabled = false; c
}
fn q(ts: Ts, bid: f64, spread: f64) -> Quote { Quote { ts, bid, ask: bid + spread } }
fn state_and_ledger(c: Settings) -> Vec<u8> {
    let mut b = SimBroker::z_ustawien(1000.0, &c);
    b.on_quote(q(T, 4015.0, 0.2));
    let mut e = Engine::new(c, 1000.0);
    e.on_message(&mut b, &IncomingMessage {
        ts: T + 1000, source: SourceKey::new(-100_3108, None),
        source_name: "SR-CONTRACT-SYNTHETIC".into(), msg_id: 1,
        reply_to: None, edit_of: None,
        text: "BUY LIMITS GOLD @ 3998/3992 AREA\nTP 4020\nTP 4030\nTP 4050\nSL 3980".into(),
    });
    let quote = q(T + 2000, 3991.0, 0.2);
    b.on_quote(quote); e.on_tick(&mut b, &quote);
    assert_eq!(b.positions().len(), 3);
    for i in 1..=36 {
        let quote = q(T + i * 60_000, 4004.0 + ((i * 7) % 11) as f64 - 5.0, 0.2 + (i % 3) as f64 * 0.1);
        b.on_quote(quote); e.on_tick(&mut b, &quote);
    }
    serde_json::to_vec(&json!({"baskets": e.baskets, "positions": b.positions(),
        "history": b.history, "balance": b.account().balance, "sr": e.stan_sr()})).unwrap()
}

#[test]
fn every_sr_child_is_dormant_with_its_parent_off_individually_and_together() {
    let changes = json!({
        "trail_sr_scope": "All", "trail_sr_activation": "Entry",
        "trail_sr_min_gain": 0.0, "trail_sr_min_dist_price": 0.0,
        "trail_sr_tf_min": 2, "trail_sr_fractal_n": 1, "trail_sr_offset": 0.0,
        "trail_sr_min_dist_tp": 0.0, "trail_sr_struct_window_h": 1,
        "trail_sr_min_prominence_atr": 0.1, "trail_sr_offset_atr_mult": 2.0,
        "trail_sr_offset_spread_mult": 5.0, "trail_sr_atr_period": 2
    });
    let baseline = state_and_ledger(config());
    for (name, value) in changes.as_object().unwrap() {
        let mut raw = serde_json::to_value(config()).unwrap(); raw[name] = value.clone();
        assert_eq!(baseline, state_and_ledger(serde_json::from_value(raw).unwrap()), "child={name}");
    }
    let mut raw = serde_json::to_value(config()).unwrap();
    for (name, value) in changes.as_object().unwrap() { raw[name] = value.clone(); }
    assert_eq!(baseline, state_and_ledger(serde_json::from_value(raw).unwrap()), "all children together");
}

fn dynamic_config() -> Settings {
    let mut c = config(); c.trail_sr_enabled = true;
    c.trail_sr_scope = TrailSrScope::All; c.trail_sr_activation = TrailSrActivation::Entry;
    c.trail_sr_tf_min = 2; c.trail_sr_fractal_n = 1;
    c.trail_sr_atr_period = 3; c.trail_sr_min_dist_price = 0.0;
    c.trail_sr_offset_atr_mult = 0.25; c.trail_sr_offset_spread_mult = 2.0; c
}

#[test]
fn true_mid_ohlc_warmup_matches_ticks_but_bid_ohlc_is_not_the_same_input() {
    let c = dynamic_config();
    let mut continuous = Engine::new(c.clone(), 1000.0);
    let mut b = SimBroker::z_ustawien(1000.0, &c);
    let mut mid_bars = Vec::new(); let mut bid_bars = Vec::new();
    for i in 0..12 {
        let base = 4000.0 + ((i * 3) % 5) as f64;
        let ts = T + i * 60_000;
        let quotes = [q(ts, base + 1.0, 0.8), q(ts + 30_000, base - 1.0, 0.4), q(ts + 59_000, base, 0.2)];
        for quote in quotes { b.on_quote(quote); continuous.on_tick(&mut b, &quote); }
        mid_bars.push(SrWarmupBar { ts,
            high: quotes.iter().map(Quote::mid).fold(f64::NEG_INFINITY, f64::max),
            low: quotes.iter().map(Quote::mid).fold(f64::INFINITY, f64::min),
            close: quotes[2].mid(), spread: quotes[2].ask - quotes[2].bid });
        bid_bars.push(SrWarmupBar { ts, high: base + 1.0, low: base - 1.0,
            close: base, spread: quotes[2].ask - quotes[2].bid });
    }
    let mut exact = Engine::new(c.clone(), 1000.0);
    let mut wrong_basis = Engine::new(c, 1000.0);
    assert!(exact.rozgrzej_sr_z_m1(&mid_bars));
    assert!(wrong_basis.rozgrzej_sr_z_m1(&bid_bars));
    assert_eq!(continuous.stan_sr(), exact.stan_sr(), "true MID extrema/close are sufficient for this state");
    assert_ne!(continuous.stan_sr(), wrong_basis.stan_sr(), "a ready flag does not certify BID OHLC == MID ticks");
    // Adding half of just the bar's last spread is not an exact repair when
    // the spread at the intrabar high and low was different.
    for bar in &mut bid_bars { bar.high += bar.spread / 2.0; bar.low += bar.spread / 2.0; bar.close += bar.spread / 2.0; }
    let mut approximate = Engine::new(dynamic_config(), 1000.0);
    assert!(approximate.rozgrzej_sr_z_m1(&bid_bars));
    assert_ne!(continuous.stan_sr(), approximate.stan_sr());
}

#[test]
fn legacy_out_of_order_tick_can_rewind_the_sr_bucket() {
    let c = dynamic_config();
    let mut e = Engine::new(c.clone(), 1000.0);
    let mut b = SimBroker::z_ustawien(1000.0, &c);
    for i in [0, 2, 4] {
        let quote = q(T + i * 60_000, 4000.0 + i as f64, 0.2);
        b.on_quote(quote); e.on_tick(&mut b, &quote);
    }
    let before: Value = serde_json::to_value(e.stan_sr()).unwrap();
    let stale = q(T + 60_000, 3990.0, 0.2);
    b.on_quote(stale); e.on_tick(&mut b, &stale);
    let after: Value = serde_json::to_value(e.stan_sr()).unwrap();
    assert_ne!(before, after, "the core does not currently reject out-of-order quote delivery");
    assert!(after["kubelek"].as_i64().unwrap() < before["kubelek"].as_i64().unwrap());
}

#[test]
fn legacy_minute_open_timestamp_changes_swing_confirmation_time() {
    let c = dynamic_config();
    let mut continuous = Engine::new(c.clone(), 1000.0);
    let mut broker = SimBroker::z_ustawien(1000.0, &c);
    let mut rounded_bars = Vec::new();
    let mut first_tick_bars = Vec::new();
    // Real quote streams do not have a tick at every exact minute open.
    // The price/spread history is identical in both reconstructions; only
    // the timestamp passed to sr_zamknij_swiece differs.
    for i in 0..24 {
        let minute = T + i * 60_000;
        let base = 4000.0 + ((i * 3) % 7) as f64;
        let quotes = [
            q(minute + 1_234 + i * 97, base + 1.0, 0.8),
            q(minute + 30_000, base - 1.0, 0.4),
            q(minute + 59_000, base, 0.2),
        ];
        for quote in quotes {
            broker.on_quote(quote);
            continuous.on_tick(&mut broker, &quote);
        }
        let bar = SrWarmupBar {
            ts: minute,
            high: quotes.iter().map(Quote::mid).fold(f64::NEG_INFINITY, f64::max),
            low: quotes.iter().map(Quote::mid).fold(f64::INFINITY, f64::min),
            close: quotes[2].mid(),
            spread: quotes[2].ask - quotes[2].bid,
        };
        rounded_bars.push(bar);
        first_tick_bars.push(SrWarmupBar { ts: quotes[0].ts, ..bar });
    }
    let mut rounded = Engine::new(c.clone(), 1000.0);
    let mut exact = Engine::new(c, 1000.0);
    assert!(rounded.rozgrzej_sr_z_m1(&rounded_bars));
    assert!(exact.rozgrzej_sr_z_m1(&first_tick_bars));
    assert_eq!(continuous.stan_sr(), exact.stan_sr(),
        "MID extrema and the actual first-tick confirmation time preserve this state");
    assert_ne!(continuous.stan_sr(), rounded.stan_sr(),
        "rounding timestamps changes the state even with exact MID prices");
    // A passing legacy reproduction is NOT a claim that the current
    // runner/live warmup producer already preserves first-tick timestamps.
}
