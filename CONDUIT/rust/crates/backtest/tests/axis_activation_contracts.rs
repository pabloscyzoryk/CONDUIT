//! Behavioral axis contracts. Synthetic input, never profitability evidence.
//! Tests public Engine ingress plus actual SimBroker state, not a second engine.
use conduit_backtest::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_800_000_000_000;
const ENTRY: &str = "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nTP OPEN\nSL 3995";

fn message(id: i64, text: &str) -> IncomingMessage {
    IncomingMessage { ts: T0 + id * 1_000, source: SourceKey::new(-100_777_3108, None),
        source_name: "AXIS-CONTRACT-SYNTHETIC".into(), msg_id: id,
        reply_to: if id == 1 { None } else { Some(1) }, edit_of: None, text: text.into() }
}

fn quote(ts: Ts, bid: f64) -> Quote { Quote { ts, bid, ask: bid + 0.20 } }

fn tick(e: &mut Engine, b: &mut SimBroker, ts: Ts, bid: f64) {
    let q = quote(ts, bid);
    b.on_quote(q);
    e.on_tick(b, &q);
}

fn settings() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3;
    c.lot_mode_percent = false;
    c.lot_fixed = 0.01;
    c.risk_per_basket_pct = 0.0;
    c.max_portfolio_risk_pct = 0.0;
    c.tp_schedule = TpSchedule::AllRunners;
    c.assign_tp_per_position = false;
    c.last_runner = LastRunner::NoTp;
    c.bank_all_at_stage = 0;
    c.pending_lifetime = PendingLifetime::Never;
    c.pending_drop_on_target = false;
    c.pending_ttl_h = 0.0;
    c.trail_mode = TrailMode::Off;
    c.trail_split = false;
    c.risk_free_trail = false;
    c.riskfree_enabled = false;
    c.trail_sr_enabled = false;
    c.be_at_tp1 = false;
    c.be_od_etapu = 0;
    c.be_lock_pts = 0.0;
    c
}

fn filled(c: Settings) -> (Engine, SimBroker) {
    let mut b = SimBroker::z_ustawien(1000.0, &c);
    b.on_quote(quote(T0, 4008.0));
    let mut e = Engine::new(c, 1000.0);
    e.on_message(&mut b, &message(1, ENTRY));
    assert_eq!(b.pendings().len(), 3);
    tick(&mut e, &mut b, T0 + 2000, 3999.0);
    tick(&mut e, &mut b, T0 + 3000, 4008.0);
    assert_eq!(b.positions().len(), 3);
    (e, b)
}

#[test]
fn legacy_price_only_can_be_bypassed_by_unindexed_pips_guard() {
    for (guard, expected_stage) in [(false, 0), (true, 1)] {
        let mut c = settings();
        c.tp_source = TpSource::PriceOnly;
        c.tp_unindexed_pips_require_price = guard;
        c.tp_price_tolerance = 0.25;
        let (mut e, mut b) = filled(c);
        // Below the real target. No Engine tick: testing receipt of a message,
        // not falsely attributing a price-triggered stage to Telegram.
        b.on_quote(quote(T0 + 4000, 4009.90));
        assert_eq!(e.baskets[0].tp_stage, 0);
        e.on_message(&mut b, &message(4, "+50 PIPS HIT"));
        assert_eq!(e.baskets[0].tp_stage, expected_stage, "guard={guard}");
    }
}

#[test]
fn official_pct_is_dormant_under_all_runners_but_active_under_official_pct() {
    for (schedule, percentage, close_last, expected_open) in [
        (TpSchedule::AllRunners, 0.0, false, 3),
        (TpSchedule::AllRunners, 100.0, true, 3),
        (TpSchedule::OfficialPct, 0.0, true, 3),
        // 100% is still constrained by the separate last-position guard.
        (TpSchedule::OfficialPct, 100.0, false, 1),
        (TpSchedule::OfficialPct, 100.0, true, 0),
    ] {
        let mut c = settings();
        c.tp_schedule = schedule;
        c.official_pct = [percentage; 4];
        c.bank_close_last = close_last;
        let (mut e, mut b) = filled(c);
        e.on_message(&mut b, &message(4, "TP1 HIT"));
        assert_eq!(b.positions().len(), expected_open, "schedule={schedule:?}, pct={percentage}, close_last={close_last}");
    }
}

#[test]
fn all_runners_does_not_override_explicit_bank_all_at_stage() {
    for (bank, expected_open) in [(0, 3), (3, 0)] {
        let mut c = settings();
        c.bank_all_at_stage = bank;
        let (mut e, mut b) = filled(c);
        e.on_message(&mut b, &message(4, "TP3 HIT"));
        assert_eq!(b.positions().len(), expected_open, "bank_all_at_stage={bank}");
    }
}

#[test]
fn runner_trailing_requires_a_real_activation_branch_not_just_non_off_mode() {
    for (split, rf_trail, secured, expected_changed) in [
        (false, false, false, 0),
        (false, false, true, 0),
        (true, false, false, 1),
        (false, true, false, 0),
        // Current channel-RF branch applies to every position in a secured basket.
        (false, true, true, 3),
    ] {
        let mut c = settings();
        c.tp_source = TpSource::SignalOnly;
        c.trail_runner_mode = TrailMode::Gap;
        c.trail_runner_start = 1.0;
        c.trail_runner_gap = 1.0;
        c.trail_runners_by_depth = false;
        let (mut e, mut b) = filled(c);
        e.cfg.trail_split = split;
        e.cfg.risk_free_trail = rf_trail;
        e.baskets[0].secured = secured;
        for (i, p) in b.positions_mut().iter_mut().enumerate() { p.is_runner = i == 0; }
        let old: Vec<_> = b.positions().iter().map(|p| (p.ticket, p.sl)).collect();
        tick(&mut e, &mut b, T0 + 4000, 4012.0);
        let changed = b.positions().iter().filter(|p| old.iter().find(|(t, _)| *t == p.ticket).unwrap().1 != p.sl).count();
        assert_eq!(changed, expected_changed, "split={split}, RFtrail={rf_trail}, secured={secured}");
    }
}

#[test]
fn at_telemetry_switch_does_not_disable_formal_hit_or_explicit_sl_management() {
    for (text, expected_stage, expected_sl) in [
        ("AT TP1 +50 PIPS", 0, Some(3995.0)),
        ("TP1 HIT +50 PIPS", 1, Some(3995.0)),
        ("AT TP1 +50 PIPS\nMOVE SL TO 3997", 0, Some(3997.0)),
    ] {
        let mut c = settings();
        c.profit_update_telemetry_only = true;
        let (mut e, mut b) = filled(c);
        e.on_message(&mut b, &message(4, text));
        assert_eq!(e.baskets[0].tp_stage, expected_stage, "{text}");
        assert_eq!(e.baskets[0].sl, expected_sl, "{text}");
    }
}

#[test]
fn defaults_do_not_enable_runner_trailing_without_split_or_channel_rf_gate() {
    let c = Settings::default();
    assert_eq!(c.trail_mode, TrailMode::Off);
    assert_eq!(c.trail_runner_mode, TrailMode::Tiered);
    assert!(!c.trail_split);
    assert!(!c.risk_free_trail);
    assert!(!c.riskfree_enabled);
}

#[test]
fn strict_price_only_blocks_every_tp_message_but_keeps_price_and_explicit_management() {
    for guard in [false, true] {
        for text in ["+50 PIPS HIT", "TP1 HIT", "AT TP1 +50 PIPS"] {
            let mut c = settings();
            c.tp_source = TpSource::PriceOnly;
            c.tp_price_only_strict = true;
            c.tp_unindexed_pips_require_price = guard;
            c.tp_price_tolerance = 0.25;
            let (mut e, mut b) = filled(c);
            b.on_quote(quote(T0 + 4000, 4009.90));
            e.on_message(&mut b, &message(4, text));
            assert_eq!(e.baskets[0].tp_stage, 0, "guard={guard}: {text}");
            e.on_message(&mut b, &message(5, "MOVE SL TO 3997"));
            assert_eq!(e.baskets[0].sl, Some(3997.0));
            tick(&mut e, &mut b, T0 + 6000, 4010.0);
            assert_eq!(e.baskets[0].tp_stage, 1, "real price touch remains active");
        }
    }
}

#[test]
fn strict_price_only_is_a_noop_in_every_other_tp_source() {
    for source in [TpSource::SignalOnly, TpSource::Either, TpSource::SignalConfirmedByPrice, TpSource::PriceFirstSignalWindow] {
        for guard in [false, true] {
            for text in ["+50 PIPS HIT", "TP1 HIT", "AT TP1 +50 PIPS"] {
                let mut results = Vec::new();
                for strict in [false, true] {
                    let mut c = settings();
                    c.tp_source = source;
                    c.tp_price_only_strict = strict;
                    c.tp_unindexed_pips_require_price = guard;
                    c.tp_price_tolerance = 0.25;
                    let (mut e, mut b) = filled(c);
                    b.on_quote(quote(T0 + 4000, 4009.90));
                    e.on_message(&mut b, &message(4, text));
                    results.push(serde_json::json!({"baskets": e.baskets, "positions": b.positions(), "history": b.history, "rejects": e.odrzuty}));
                }
                assert_eq!(results[0], results[1], "source={source:?}, guard={guard}, text={text}");
            }
        }
    }
}

#[test]
fn unsupported_runner_partial_and_dormant_strict_mode_are_reported_not_silently_used() {
    let mut c = settings();
    c.runner_partial_pct = 15.0;
    assert!(c.martwe_ustawienia().iter().any(|x| x.pole == "runner_partial_pct"));
    c.runner_partial_pct = 0.0;
    assert!(!c.martwe_ustawienia().iter().any(|x| x.pole == "runner_partial_pct"));
    c.tp_price_only_strict = true;
    c.tp_source = TpSource::Either;
    assert!(c.martwe_ustawienia().iter().any(|x| x.pole == "tp_price_only_strict"));
    c.tp_source = TpSource::PriceOnly;
    assert!(!c.martwe_ustawienia().iter().any(|x| x.pole == "tp_price_only_strict"));
}
