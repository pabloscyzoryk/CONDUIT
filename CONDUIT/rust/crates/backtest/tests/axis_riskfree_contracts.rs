//! Real Engine + SimBroker failure-mode contracts, not profitability evidence.
//! A passing `legacy_*` test documents a defect; it does not certify safety.
use conduit_backtest::SimBroker;
use conduit_core::broker::{Broker, BResult, BrokerError, OrderReq, PendingReq};
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T: Ts = 1_800_000_000_000;
fn msg(id: i64, text: &str) -> IncomingMessage {
    IncomingMessage { ts: T + id * 1000, source: SourceKey::new(-100_3108, None),
        source_name: "RF-CONTRACT-SYNTHETIC".into(), msg_id: id,
        reply_to: (id != 1).then_some(1), edit_of: None, text: text.into() }
}
fn quote(t: Ts, bid: f64) -> Quote { Quote { ts: t, bid, ask: bid + 0.20 } }
fn cfg() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3;
    c.lot_mode_percent = false;
    c.lot_fixed = 0.04;
    c.risk_per_basket_pct = 0.0;
    c.max_portfolio_risk_pct = 0.0;
    c.tp_source = TpSource::SignalOnly;
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
    c.be_offset = 0.0;
    c
}
fn tick(e: &mut Engine, b: &mut SimBroker, t: Ts, bid: f64) {
    let q = quote(t, bid); b.on_quote(q); e.on_tick(b, &q);
}
fn filled(c: Settings) -> (Engine, SimBroker) {
    let mut b = SimBroker::z_ustawien(1000.0, &c);
    b.on_quote(quote(T, 4008.0));
    let mut e = Engine::new(c, 1000.0);
    e.on_message(&mut b, &msg(1, "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3995"));
    assert_eq!(b.pendings().len(), 3);
    tick(&mut e, &mut b, T + 2000, 3999.0);
    assert_eq!(b.positions().len(), 3);
    (e, b)
}
fn filled_side(c: Settings, side: Side) -> (Engine, SimBroker) {
    if side == Side::Buy { return filled(c); }
    let mut b = SimBroker::z_ustawien(1000.0, &c);
    b.on_quote(quote(T, 3992.0));
    let mut e = Engine::new(c, 1000.0);
    e.on_message(&mut b, &msg(1, "SELL LIMITS GOLD @ 3995/4000 AREA\nTP 3990\nTP 3980\nTP 3970\nSL 4005"));
    assert_eq!(b.pendings().len(), 3);
    tick(&mut e, &mut b, T + 2000, 4001.0);
    assert_eq!(b.positions().len(), 3);
    (e, b)
}
fn stop_floor(e: &Engine, b: &SimBroker) -> f64 {
    e.baskets[0].realized + b.positions().iter().map(|p| {
        (p.sl.expect("fixture has an actual stop") - p.open_price)
            * p.side.sign() * p.volume * XAU_CONTRACT
    }).sum::<f64>()
}

/// Same observable past, two different FUTURE continuations. The negative
/// threshold is an explicit synthetic research input, not a selected preset.
/// A better result on the UP tail is not permission to conceal a realized loss.
fn causal_rearm_branch(allow_loss: bool, up_tail: bool, journal: bool) -> (Vec<u8>, f64, u32) {
    let mut c = cfg();
    c.risk_free_mode = RiskFreeMode::CloseAllKeepNearest;
    c.risk_free_runners = 1;
    c.basket_realized_broker_only = true;
    c.rearm_grid_on_return = true;
    c.rearm_block_after_secured = false;
    c.rearm_min_gap_min = 0.0;
    c.rearm_max_times = 1;
    c.rearm_min_basket_profit = if allow_loss { -100.0 } else { 0.0 };
    c.journal_enabled = journal;
    let (mut e, mut b) = filled(c);
    b.on_quote(quote(T + 3000, 4001.0));
    e.on_message(&mut b, &msg(3, "RISK FREE 4000"));
    tick(&mut e, &mut b, T + 4000, 4001.0);
    assert!(e.baskets[0].realized < 0.0, "real RF losses must stay booked");
    let rearms = e.baskets[0].rearms;
    assert_eq!(rearms, u32::from(allow_loss));
    let prefix = serde_json::to_vec(&serde_json::json!({
        "baskets": e.baskets, "positions": b.positions(), "pendings": b.pendings(),
        "history": b.history, "account": b.account(),
    })).unwrap();
    // Only now may the future diverge. No future return value selects the rule.
    let final_bid = if up_tail { 4025.0 } else { 3990.0 };
    tick(&mut e, &mut b, T + 5000, final_bid);
    let tickets: Vec<_> = b.positions().iter().map(|p| p.ticket).collect();
    for t in tickets { b.close_position(t, CloseReason::Manual).unwrap(); }
    tick(&mut e, &mut b, T + 6000, final_bid);
    assert!(b.positions().is_empty());
    let actual: f64 = b.history.iter().map(|t| t.profit).sum();
    assert!((e.baskets[0].realized - actual).abs() < 1e-7,
        "basket ledger must equal all actual fixture closes");
    assert!((b.account().balance - 1000.0 - actual).abs() < 1e-7);
    (prefix, actual, rearms)
}

#[test]
fn different_future_wins_and_losses_have_identical_rearm_decision_prefix() {
    for allow in [false, true] {
        let (up_prefix, up_result, _) = causal_rearm_branch(allow, true, true);
        let (down_prefix, down_result, _) = causal_rearm_branch(allow, false, true);
        assert_eq!(up_prefix, down_prefix, "future branch cannot alter earlier decisions");
        assert!(up_result > down_result);
    }
}

#[test]
fn explicit_underwater_rearm_can_amplify_both_profit_and_loss_without_false_ledger() {
    let (_, up_without, _) = causal_rearm_branch(false, true, true);
    let (_, up_with, _) = causal_rearm_branch(true, true, true);
    let (_, down_without, _) = causal_rearm_branch(false, false, true);
    let (_, down_with, _) = causal_rearm_branch(true, false, true);
    assert!(up_with > up_without, "deliberate rearm can retain upside");
    assert!(down_with < down_without, "it also adds real downside, not risk-free profit");
    println!("SYNTHETIC ONLY: up no-rearm={up_without:.2}, rearm={up_with:.2}; down no-rearm={down_without:.2}, rearm={down_with:.2}");
}

#[test]
fn decision_journal_does_not_change_rearm_orders_or_money() {
    for allow in [false, true] {
        for up in [false, true] {
            assert_eq!(causal_rearm_branch(allow, up, false), causal_rearm_branch(allow, up, true));
        }
    }
}

/// The production Engine sees real simulated positions/receipts. Only the
/// broker's modify response is fault-injected; no second management engine.
struct RejectModify(SimBroker);
impl Broker for RejectModify {
    fn quote(&self) -> Quote { self.0.quote() }
    fn account(&self) -> Account { self.0.account() }
    fn stops_level(&self) -> f64 { self.0.stops_level() }
    fn volume_min(&self) -> f64 { self.0.volume_min() }
    fn volume_step(&self) -> f64 { self.0.volume_step() }
    fn volume_max(&self) -> f64 { self.0.volume_max() }
    fn close_receipt_reconciliation_active(&self) -> bool { self.0.close_receipt_reconciliation_active() }
    fn close_receipts_pending(&self) -> bool { self.0.close_receipts_pending() }
    fn positions(&self) -> &[Position] { self.0.positions() }
    fn pendings(&self) -> &[PendingOrder] { self.0.pendings() }
    fn positions_mut(&mut self) -> &mut Vec<Position> { self.0.positions_mut() }
    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> { self.0.pendings_mut() }
    fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> { self.0.open_market(r) }
    fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> { self.0.place_pending(r) }
    fn modify_position(&mut self, _: Ticket, _: Option<Px>, _: Option<Px>) -> BResult<()> { Err(BrokerError::Rejected) }
    fn modify_pending(&mut self, t: Ticket, p: Px, sl: Option<Px>, tp: Option<Px>) -> BResult<()> { self.0.modify_pending(t, p, sl, tp) }
    fn close_position(&mut self, t: Ticket, r: CloseReason) -> BResult<f64> { self.0.close_position(t, r) }
    fn close_partial(&mut self, t: Ticket, v: f64, r: CloseReason) -> BResult<f64> { self.0.close_partial(t, v, r) }
    fn cancel_pending(&mut self, t: Ticket) -> BResult<()> { self.0.cancel_pending(t) }
    fn drain_closed(&mut self) -> Vec<ClosedTrade> { self.0.drain_closed() }
}

#[test]
fn legacy_channel_rf_closes_losers_and_own_be_does_not_cover_their_loss() {
    let mut c = cfg();
    c.risk_free_mode = RiskFreeMode::CloseAllKeepNearest;
    c.risk_free_runners = 1;
    c.basket_realized_broker_only = true;
    let (mut e, mut b) = filled(c);
    b.on_quote(quote(T + 3000, 4001.0));
    let before: Vec<_> = b.positions().iter().map(|p| p.open_price).collect();
    assert!(before.iter().any(|v| *v > 4001.0), "fixture must contain losing entries: {before:?}");
    e.on_message(&mut b, &msg(3, "RISK FREE 4000"));
    assert_eq!(b.positions().len(), 1);
    assert!(b.history.iter().map(|x| x.profit).sum::<f64>() < -1.0);
    assert_eq!(b.positions()[0].sl, Some(b.positions()[0].open_price));
    tick(&mut e, &mut b, T + 4000, 4001.0); // consume actual Sim receipts
    assert!(e.baskets[0].secured, "legacy marks the underwater basket secured");
    assert!(stop_floor(&e, &b) < -1.0, "own BE is NOT basket BE");
}

#[test]
fn legacy_channel_rf_marks_secured_even_when_no_be_can_be_placed() {
    let mut c = cfg();
    c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
    let (mut e, mut b) = filled(c);
    // All three entries are above the current BUY liquidation price.
    b.on_quote(quote(T + 3000, 3999.0));
    e.on_message(&mut b, &msg(3, "RISK FREE"));
    assert!(b.positions().iter().all(|p| p.sl == Some(3995.0)));
    assert!(e.baskets[0].secured);
    assert!(stop_floor(&e, &b) < -1.0);
}

#[test]
fn legacy_automatic_rf_stop_off_marks_secured_without_any_stop() {
    let mut c = cfg();
    c.riskfree_enabled = true;
    c.riskfree_trigger_usd = 1.0;
    c.riskfree_keep_units = 1;
    c.riskfree_runner_stop = RiskFreeRunnerStop::Off;
    c.basket_realized_broker_only = true;
    let (mut e, mut b) = filled(c);
    tick(&mut e, &mut b, T + 3000, 4010.0);
    assert_eq!(b.positions().len(), 1);
    assert_eq!(b.positions()[0].sl, None);
    assert!(e.baskets[0].secured);
}

#[test]
fn legacy_automatic_rf_marks_secured_after_broker_rejects_be_and_floor_is_negative() {
    let mut c = cfg();
    c.riskfree_enabled = true;
    c.riskfree_trigger_usd = 1.0;
    c.riskfree_keep_units = 1;
    c.riskfree_runner_stop = RiskFreeRunnerStop::BeOwn;
    c.basket_realized_broker_only = true;
    let (mut e, b) = filled(c);
    let mut b = RejectModify(b);
    let q = quote(T + 3000, 4005.2);
    b.0.on_quote(q);
    e.on_tick(&mut b, &q);
    assert_eq!(b.positions().len(), 1);
    assert_eq!(b.positions()[0].sl, Some(3995.0), "rejected BE cannot change the broker stop");
    // Consume the closure receipts without another quote that could hit stops.
    e.on_tick(&mut b, &q);
    assert!(e.baskets[0].secured, "legacy incorrectly treats attempted BE as confirmed");
    assert!(stop_floor(&e, &b.0) < -1.0, "actual remaining SL can still make the basket lose");
}

#[test]
fn legacy_tp_be_can_loosen_a_better_stop_unless_sr_family_is_enabled() {
    for sr in [false, true] {
        let (mut e, mut b) = filled(cfg());
        b.on_quote(quote(T + 3000, 4008.0));
        let tickets: Vec<_> = b.positions().iter().map(|p| p.ticket).collect();
        for ticket in tickets { b.modify_position(ticket, Some(4006.0), None).unwrap(); }
        e.cfg.be_at_tp1 = true;
        e.cfg.trail_sr_enabled = sr;
        e.on_message(&mut b, &msg(3, "TP1 HIT"));
        for p in b.positions() {
            assert_eq!(p.sl, Some(if sr { 4006.0 } else { p.open_price }), "SR={sr}");
        }
    }
}

#[test]
fn be_never_loosen_preserves_better_sl_for_buy_sell_tp_and_channel_rf() {
    for side in [Side::Buy, Side::Sell] {
        for channel_rf in [false, true] {
            for strict in [false, true] {
                let mut c = cfg();
                c.be_never_loosen = strict;
                c.be_at_tp1 = true;
                c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
                let (mut e, mut b) = filled_side(c, side);
                let (bid, better_sl) = if side == Side::Buy { (4008.0, 4006.0) } else { (3992.0, 3994.0) };
                b.on_quote(quote(T + 3000, bid));
                let tickets: Vec<_> = b.positions().iter().map(|p| p.ticket).collect();
                for ticket in tickets { b.modify_position(ticket, Some(better_sl), None).unwrap(); }
                e.on_message(&mut b, &msg(3, if channel_rf { "RISK FREE" } else { "TP1 HIT" }));
                for p in b.positions() {
                    assert_eq!(p.sl, Some(if strict { better_sl } else { p.open_price }),
                        "side={side:?}, RF={channel_rf}, strict={strict}");
                }
            }
        }
    }
}

#[test]
fn be_never_loosen_does_not_change_ledger_when_be_really_improves_the_stop() {
    for side in [Side::Buy, Side::Sell] {
        for channel_rf in [false, true] {
            let mut results = Vec::new();
            for strict in [false, true] {
                let mut c = cfg(); c.be_never_loosen = strict; c.be_at_tp1 = true;
                c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
                let (mut e, mut b) = filled_side(c, side);
                b.on_quote(quote(T + 3000, if side == Side::Buy {4008.0} else {3992.0}));
                e.on_message(&mut b, &msg(3, if channel_rf {"RISK FREE"} else {"TP1 HIT"}));
                results.push(serde_json::to_vec(&serde_json::json!({
                    "baskets": e.baskets, "positions": b.positions(), "history": b.history
                })).unwrap());
            }
            assert_eq!(results[0], results[1], "side={side:?}, RF={channel_rf}");
        }
    }
}

#[test]
fn retarget_correction_honors_last_target_and_does_not_restore_no_tp_runners() {
    for strict in [false, true] {
        for last_only in [false, true] {
            let mut c = cfg(); c.tp_schedule = TpSchedule::OfficialPct;
            c.official_pct = [0.0; 4];
            c.cele_na_ostatnim = last_only;
            c.retarget_respects_final_target = strict;
            let (mut e, mut b) = filled(c);
            b.on_quote(quote(T + 3000, 4008.0));
            let no_tp_ticket = b.positions()[0].ticket;
            let sl = b.positions()[0].sl;
            b.modify_position(no_tp_ticket, sl, None).unwrap();
            e.on_message(&mut b, &msg(3, "TP1 HIT"));
            for p in b.positions() {
                let expected = if p.ticket == no_tp_ticket {None}
                    else {Some(if strict && last_only {4030.0} else {4020.0})};
                assert_eq!(p.tp, expected, "strict={strict}, last_only={last_only}");
            }
        }
    }
}

#[test]
fn retarget_final_includes_explicit_tp_open_offset_without_compounding_it() {
    for strict in [false, true] {
        let mut c = cfg(); c.tp_schedule = TpSchedule::OfficialPct;
        c.official_pct = [0.0; 4]; c.cele_na_ostatnim = true;
        c.retarget_respects_final_target = strict;
        c.tp_open_extra = true; c.tp_open_offset = 12.0;
        c.tp_freeze_after_ladder = false;
        let (mut e, mut b) = filled(c);
        // Fixture variation: the actual basket advertises TP OPEN.
        e.baskets[0].tp_open = true;
        b.on_quote(quote(T + 3000, 4008.0));
        for (id, stage, legacy_target) in [(3, 1, 4020.0), (4, 2, 4030.0), (5, 3, 4042.0), (6, 4, 4054.0)] {
            e.on_message(&mut b, &msg(id, &format!("TP{stage} HIT")));
            for p in b.positions() {
                assert_eq!(p.tp, Some(if strict {4042.0} else {legacy_target}), "strict={strict}, stage={stage}");
            }
        }
    }
}

#[test]
fn partial_volume_rounding_can_turn_ten_percent_into_fifty_percent() {
    for (volume, expected_remainder) in [(0.02, 0.01), (0.07, 0.06), (0.08, 0.07)] {
        let mut c = cfg();
        c.entry_units = 1;
        c.lot_fixed = volume;
        c.tp_schedule = TpSchedule::OfficialPct;
        c.official_pct = [10.0, 10.0, 0.0, 0.0];
        c.partial_close = true;
        c.partial_min_lot = 0.02;
        let mut b = SimBroker::z_ustawien(1000.0, &c);
        b.on_quote(quote(T, 4008.0));
        let mut e = Engine::new(c, 1000.0);
        e.on_message(&mut b, &msg(1, "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3995"));
        tick(&mut e, &mut b, T + 2000, 3999.0);
        assert_eq!(b.positions().len(), 1);
        // Do not let the broker's own target close the position first.
        // This case isolates the Telegram-driven partial-volume schedule.
        b.on_quote(quote(T + 3000, 4009.0));
        e.on_message(&mut b, &msg(3, "TP1 HIT"));
        assert_eq!(b.positions().len(), 1);
        assert!((b.positions()[0].volume - expected_remainder).abs() < 1e-9);
    }
}
