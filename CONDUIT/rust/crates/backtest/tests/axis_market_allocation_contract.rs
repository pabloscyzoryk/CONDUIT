//! Public Engine/Sim geometry, not a historical replay or a profit estimate.
//! In particular, one MARKET-class unit + auto_limit does NOT mean one
//! immediate order at the current market price: allocation keeps deep layers.
use conduit_backtest::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T: Ts = 1_700_000_000_000;
fn config(cap: u32) -> Settings {
    Settings {
        auto_limit: true, entry_units: 8, market_entry_units: cap,
        market_entry_mode: MarketEntryMode::GridAtOnce,
        zone_offset_mode: ZoneOffsetMode::Directional,
        entry_deep_offset: 1.0, entry_tol_offset: 2.0,
        entry_depth_curve: 1.0, entry_warstwy_z_tekstu: false,
        zakaz_ponizej_krawedzi: false,
        drop_unplaceable_levels: true, stops_level: 0.20,
        lot_fixed: 0.05, lot_mode_percent: false, lot_max: 0.0,
        risk_per_basket_pct: 0.0, max_portfolio_risk_pct: 0.0,
        pending_lifetime: PendingLifetime::Never, pending_drop_on_target: false,
        tp_source: TpSource::PriceOnly, tp_schedule: TpSchedule::AllRunners,
        swap_enabled: false, max_dd_pct: 0.0, max_dd_usd: 0.0,
        equity_floor_pct: 0.0, ..Settings::default()
    }
}
fn sell_text(limits: bool) -> String {
    format!("SELL {}GOLD @ 4455/4460 AREA\nTP 4453\nTP 4450\nTP 4446\nSL 4461",
        if limits { "LIMITS " } else { "" })
}
fn run(c: Settings, text: &str, bid: f64) -> (Engine, SimBroker) {
    let mut b = SimBroker::z_ustawien(941.37, &c);
    let quote = Quote { ts: T, bid, ask: bid + 0.22 };
    b.on_quote(quote);
    let mut e = Engine::new(c, 941.37);
    e.on_tick(&mut b, &quote);
    e.on_message(&mut b, &IncomingMessage {
        ts: T, source: SourceKey::new(-100_310831, None),
        source_name: "allocation-contract-synthetic".into(), msg_id: 1001,
        reply_to: None, edit_of: None, text: text.into(),
    });
    assert_eq!(e.baskets.len(), 1);
    (e, b)
}
fn near(a: f64, b: f64) { assert!((a-b).abs()<1e-8, "{a} != {b}"); }

#[test]
fn market_class_cap_one_with_auto_limit_waits_for_deepest_valid_layer() {
    let (e, b) = run(config(1), &sell_text(false), 4454.21);
    assert!(b.positions().is_empty());
    assert_eq!(b.pendings().len(), 1);
    near(e.baskets[0].zone_lo, 4453.0); near(e.baskets[0].zone_hi, 4461.0);
    // 4461 itself coincides with SL and is invalid; next grid layer survives.
    near(b.pendings()[0].price, 4461.0 - 8.0/7.0);
    near(b.pendings()[0].volume, 0.05);
    assert!(b.pendings()[0].price > 4457.0,
        "a rebound only to 4457 cannot fill this pending order");
}

#[test]
fn cap_zero_restores_shallow_layers_but_does_not_prove_they_are_profitable() {
    let (_, capped) = run(config(1), &sell_text(false), 4454.21);
    let (_, full) = run(config(0), &sell_text(false), 4454.21);
    assert!(full.pendings().len() + full.positions().len() > 1);
    assert!(full.pendings().iter().any(|p| p.price <= 4457.0)
        || !full.positions().is_empty());
    assert!(full.pendings().iter().any(|p|
        (p.price-capped.pendings()[0].price).abs()<1e-8));
}

#[test]
fn explicit_limits_class_bypasses_market_cap_even_with_identical_prices() {
    let (_, full) = run(config(0), &sell_text(true), 4454.21);
    let (_, limited) = run(config(1), &sell_text(true), 4454.21);
    assert_eq!(serde_json::to_value(full.pendings()).unwrap(),
        serde_json::to_value(limited.pendings()).unwrap());
    assert_eq!(serde_json::to_value(full.positions()).unwrap(),
        serde_json::to_value(limited.positions()).unwrap());
    assert!(limited.pendings().len()+limited.positions().len()>1);
}

#[test]
fn deep_selection_is_mirrored_for_buy_not_a_sell_only_price_bug() {
    let (_, b) = run(config(1),
        "BUY GOLD @ 4000/4005 AREA\nTP 4007\nTP 4010\nTP 4014\nSL 3999", 4005.57);
    assert!(b.positions().is_empty()); assert_eq!(b.pendings().len(), 1);
    near(b.pendings()[0].price, 3999.0 + 8.0/7.0);
    assert!(b.pendings()[0].price < 4003.0);
}
