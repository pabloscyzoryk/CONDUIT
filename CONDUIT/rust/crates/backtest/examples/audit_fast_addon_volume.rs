//! Isolated actual Engine::on_tick -> fast_addon_sweep -> SimBroker probe.
//! No Telegram, network, MT5 or production files. No preset is modified.
//! Run after release build lock is free:
//! cargo run --offline --locked -p conduit-backtest --example audit_fast_addon_volume
use conduit_backtest::SimBroker;
use conduit_core::{Basket, Broker, Engine, OrderReq, Quote, Settings, Side};
use serde_json::json;

fn probe(max_addons: u32) -> (usize, Vec<f64>) {
    let ts = 1_788_170_400_000_i64; // synthetic timestamp
    let mut cfg = Settings::default();
    cfg.lot_mode_percent = true;
    cfg.lot_percent = 0.5;
    cfg.lot_min = 0.01;
    cfg.lot_max = 0.0;
    cfg.lot_max_z_salda = 0.0;
    cfg.max_open_positions = 0;
    cfg.max_open_baskets = 0;
    cfg.risk_per_basket_pct = 0.0;
    cfg.max_portfolio_risk_pct = 0.0;
    cfg.fast_addon_move_usd = 8.0;
    cfg.fast_addon_window_s = 60.0;
    cfg.fast_addon_max = max_addons;
    cfg.fast_addon_lot_mult = 1.5;
    cfg.fast_addon_min_stage = 0;
    cfg.ml_min_fast_addon = 0.0;
    cfg.pending_relot_on_balance = false;
    cfg.rearm_grid_on_return = false;
    cfg.reenter_after_tp = false;
    cfg.bank_all_at_stage = 0;
    cfg.trail_mode = conduit_core::settings::TrailMode::Off;
    cfg.trail_sr_enabled = false;
    cfg.ea_enabled = false;

    let mut broker = SimBroker::new(600.0, 0.0, 0.0);
    broker.mark(Quote {
        ts: ts - 30_000,
        bid: 100.0,
        ask: 100.2,
    });
    let ticket = broker
        .open_market(OrderReq {
            side: Side::Buy,
            volume: 0.03,
            sl: Some(90.0),
            tp: Some(200.0),
            basket: Some(1),
            level: 0,
            is_toucher: false,
            comment: "synthetic-owned-start".into(),
        })
        .unwrap();
    let basket: Basket = serde_json::from_value(json!({
        "id":1,"source":{"chat_id":-42,"topic_id":null},"source_name":"SYNTHETIC",
        "msg_id":1,"side":"Buy","is_limit":true,"entry_lo":95.0,"entry_hi":101.0,
        "zone_lo":95.0,"zone_hi":101.0,"sl":90.0,"tps":[150.0,175.0,200.0],
        "tp_stage":0,"created_ts":ts-30_000,"state":"Working","tickets":[ticket],
        "pendings":[],"had_positions":true,"realized":0.0,"events":[]
    }))
    .unwrap();
    let mut engine = Engine::new(cfg, 600.0);
    engine.adopt_baskets(vec![basket]);
    engine.set_market_history(
        vec![],
        vec![
            (ts - 20_000, 100.0),
            (ts - 10_000, 104.0),
            (ts - 5_000, 106.0),
        ],
    );
    let quote = Quote {
        ts,
        bid: 108.0,
        ask: 108.2,
    };
    broker.mark(quote);
    engine.on_tick(&mut broker, &quote);
    let volumes: Vec<f64> = broker
        .positions()
        .iter()
        .filter(|p| p.level == -4)
        .map(|p| p.volume)
        .collect();
    println!(
        "{}",
        json!({"max_addons":max_addons,"base_lot":engine.lot_size(engine.podstawa_lota()),
        "fast_addons_observed":volumes.len(),"raw_sim_volumes":volumes,
        "open_count":broker.positions().len(),"notes":engine.baskets[0].events,
        "scope":"synthetic activation only; NOT historical corpus profit attribution"})
    );
    (volumes.len(), volumes)
}

fn main() {
    let disabled = probe(0);
    assert_eq!(disabled.0, 0, "max0 is OFF, not unlimited");
    let enabled = probe(1);
    assert_eq!(
        enabled.0, 1,
        "synthetic path must actually exercise fast-addon caller"
    );
    assert!(
        (enabled.1[0] - 0.045).abs() < 1e-10,
        "record current raw SimBroker behavior; a future normalization fix changes this"
    );
}
