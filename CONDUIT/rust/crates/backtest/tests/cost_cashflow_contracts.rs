//! Executed SimBroker evidence for the pre-canonical cost contract.
//! `legacy_*` passing means the discrepancy is reproduced, NOT corrected.
use conduit_backtest::SimBroker;
use conduit_core::broker::{Broker, OrderReq, PendingReq};
use conduit_core::settings::Settings;
use conduit_core::types::*;

const DAY: i64 = 86_400_000;
const T: i64 = 20_695 * DAY + DAY / 2;
fn q(ts: i64, bid: f64) -> Quote { Quote { ts, bid, ask: bid + 0.2 } }
fn near(a: f64, b: f64) { assert!((a-b).abs() < 1e-7, "{a} != {b}"); }
fn cfg() -> Settings {
    let mut c = Settings::default();
    c.commission_per_lot = 7.0;
    c.swap_enabled = false;
    c.stops_level = 0.0;
    c
}
fn open(b: &mut SimBroker, volume: f64) -> Ticket {
    b.open_market(OrderReq { side: Side::Buy, volume, sl: None, tp: None,
        basket: Some(1), level: 0, is_toucher: false, comment: "COST-CONTRACT".into() }).unwrap()
}

#[test]
fn legacy_market_entry_commission_changes_cash_but_is_missing_from_closed_ledger() {
    let mut b = SimBroker::z_ustawien(1000.0, &cfg());
    b.on_quote(q(T, 100.0));
    let t = open(&mut b, 0.07);
    near(b.account().balance, 999.51);
    b.on_quote(q(T + 1000, 101.0));
    b.close_position(t, CloseReason::Manual).unwrap();
    let c = &b.history[0];
    near(c.profit, 5.6);
    near(c.commission, 0.0);
    near(b.account().balance - 1000.0, 5.11);
    near(c.profit - (b.account().balance - 1000.0), 0.49);
    println!("LEGACY DISCREPANCY: cash=+5.11, closed.profit=+5.60, missing entry commission=-0.49");
}

#[test]
fn legacy_pending_commission_is_charged_only_on_fill_but_never_allocated_to_close() {
    let mut b = SimBroker::z_ustawien(1000.0, &cfg());
    b.on_quote(q(T, 101.0));
    b.place_pending(PendingReq { kind: PendingKind::BuyLimit, volume: 0.07,
        price: 100.0, sl: None, tp: None, basket: Some(1), level: 0,
        is_toucher: false, is_topup: false,no_market_fallback:false, comment: "COST-CONTRACT".into() }).unwrap();
    near(b.account().balance, 1000.0);
    b.on_quote(q(T + 1000, 99.7));
    assert_eq!(b.positions().len(), 1);
    near(b.account().balance, 999.51);
    let t = b.positions()[0].ticket;
    b.on_quote(q(T + 2000, 101.0));
    b.close_position(t, CloseReason::Manual).unwrap();
    let sum: f64 = b.history.iter().map(|c| c.profit).sum();
    near(sum - (b.account().balance - 1000.0), 0.49);
    assert!(b.history.iter().all(|c| c.commission == 0.0));
}

#[test]
fn legacy_partial_defers_all_swap_to_last_exit_and_omits_entry_commission() {
    let mut c = cfg();
    c.swap_enabled = true;
    c.swap_long_points = -10.0;
    c.swap_point_value = 1.0;
    c.swap_rollover_mult = 1.0;
    c.swap_pomijaj_weekend = false;
    let mut b = SimBroker::z_ustawien(1000.0, &c);
    b.on_quote(q(T, 100.0));
    let t = open(&mut b, 0.08);
    near(b.account().balance, 999.44);
    b.on_quote(q((T / DAY + 1) * DAY + 1000, 101.0));
    near(b.account().balance, 998.64);
    b.close_partial(t, 0.04, CloseReason::Manual).unwrap();
    near(b.history[0].profit, 3.2);
    near(b.history[0].swap, 0.0);
    near(b.positions()[0].volume, 0.04);
    b.close_position(t, CloseReason::Manual).unwrap();
    near(b.history[1].profit, 2.4);
    near(b.history[1].swap, -0.8);
    let sum: f64 = b.history.iter().map(|c| c.profit).sum();
    near(sum, 5.6);
    near(b.account().balance - 1000.0, 5.04);
    println!("LEGACY DISCREPANCY: first close gross=3.20 vs proportional closed-net=2.52; complete cash=5.04 vs closed ledger=5.60");
}
