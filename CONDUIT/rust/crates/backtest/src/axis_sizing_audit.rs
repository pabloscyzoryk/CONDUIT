//! Behavioral audit and opt-in volume-contract regression: public Engine + SimBroker.
//! Tests named `observed_*` deliberately record a demonstrated limitation;
//! passing them is NOT certification that the recorded behavior is desirable.
use crate::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_700_000_000_000;
const BUY: &str = "BUY GOLD @ 4005/4000\nTP 4030\nTP 4060\nTP 4090\nSL 3990";
const SELL: &str = "SELL GOLD @ 4000/4005\nTP 3970\nTP 3940\nTP 3910\nSL 4015";

fn cfg() -> Settings {
    Settings {
        auto_limit: true,
        entry_units: 3,
        lot_fixed: 0.03,
        lot_max: 0.0,
        pending_lifetime: PendingLifetime::Never,
        pending_drop_on_target: false,
        tp_source: TpSource::PriceOnly,
        tp_price_only_strict: true,
        swap_enabled: false,
        max_dd_pct: 0.0,
        max_dd_usd: 0.0,
        equity_floor_pct: 0.0,
        ..Settings::default()
    }
}
fn quote(ts: Ts, bid: f64) -> Quote { Quote { ts, bid, ask: bid + 0.20 } }
fn message(ts: Ts, text: &str) -> IncomingMessage {
    IncomingMessage {
        ts, source: SourceKey::new(-990001, None), source_name: "axis-audit".into(),
        msg_id: 1, reply_to: None, edit_of: None, text: text.into(),
    }
}
fn rig(c: Settings, balance: f64, text: &str, bid: f64) -> (Engine, SimBroker) {
    let mut b = SimBroker::z_ustawien(balance, &c);
    let mut e = Engine::new(c, balance);
    tick(&mut e, &mut b, T0, bid);
    e.on_message(&mut b, &message(T0, text));
    assert!(!e.baskets.is_empty(), "fixture must parse an actual entry");
    (e, b)
}
fn tick(e: &mut Engine, b: &mut SimBroker, ts: Ts, bid: f64) {
    let q = quote(ts, bid); b.on_quote(q); e.on_tick(b, &q);
}
fn volume(b: &SimBroker) -> f64 { b.pendings().iter().map(|x| x.volume).sum() }
fn risk(b: &SimBroker) -> f64 {
    b.pendings().iter().map(|x| x.sl.map_or(0.0, |s| (x.price-s).abs()*100.0*x.volume)).sum()
}
fn prices(b: &SimBroker) -> Vec<f64> {
    let mut x: Vec<_> = b.pendings().iter().map(|p| p.price).collect();
    x.sort_by(f64::total_cmp); x
}
fn near(a: f64, b: f64) { assert!((a-b).abs()<1e-8, "{a} != {b}"); }

#[test]
fn observed_single_unit_precedes_explicit_layout() {
    let mut c = cfg(); c.entry_units = 1; c.entry_uklad = "1,2,3".into();
    let (_, b) = rig(c.clone(), 1000.0, BUY, 4012.0);
    assert_eq!(prices(&b), vec![4005.0]);
    c.entry_units = 3;
    let (_, b) = rig(c, 1000.0, BUY, 4012.0);
    assert_eq!(b.pendings().len(), 6, "layout becomes active above one base unit");
}

#[test]
fn observed_ppm_precedes_layout_and_depth_curve() {
    let mut c = cfg(); c.entry_uklad = "0,3,1".into(); c.entry_depth_curve = 4.0;
    c.ppm_enabled = true; c.ppm = 1.0; c.units_per_level = false;
    let (_, a) = rig(c.clone(), 1000.0, BUY, 4012.0);
    assert_eq!(prices(&a), vec![4000.,4001.,4002.,4003.,4004.,4005.]);
    c.entry_uklad.clear(); c.entry_depth_curve = 0.25;
    let (_, b) = rig(c, 1000.0, BUY, 4012.0);
    assert_eq!(prices(&a), prices(&b));
}

#[test]
fn observed_layout_bypasses_per_level_count_budgets() {
    let mut c = cfg(); c.entry_uklad = "1,2,3".into();
    c.entry_risk_budget = 0.01; c.entry_tp1_budget = 0.01;
    let (_, b) = rig(c, 1000.0, BUY, 4012.0);
    assert_eq!(b.pendings().len(), 6, "layout writes base_units directly");
}

#[test]
fn absolute_anchor_and_units_per_level_zone_are_independent() {
    let mut c = cfg(); c.ppm_enabled = true; c.ppm = 1.0;
    c.grid_anchor_absolute = true; c.entry_units = 4;
    for (zone_mult, count) in [(true,24),(false,6)] {
        c.units_per_level_zone = zone_mult;
        let (_, b) = rig(c.clone(), 1000.0, BUY, 4012.0);
        assert_eq!(b.pendings().len(), count);
    }
    c.units_per_level = false;
    let (_, b) = rig(c, 1000.0, &BUY.replace("BUY GOLD", "BUY LIMIT GOLD"), 4012.0);
    assert_eq!(b.pendings().len(), 6);
}

#[test]
fn zero_ppm_is_not_zero_trades_or_one_level_when_units_exceed_one() {
    let mut c = cfg(); c.ppm_enabled = true; c.ppm = 0.0;
    let (_, b) = rig(c, 1000.0, BUY, 4012.0);
    assert_eq!(prices(&b), vec![4000.,4002.5,4005.]);
}

#[test]
fn geometry_fraction_has_own_gate_and_zone_mode_remains_parent() {
    let mut c = cfg(); c.adaptive_params = false;
    c.entry_deep_frac_to_sl = 0.5; c.entry_deep_offset = 99.0;
    c.entry_tol_offset = 0.0; c.zone_offset_mode = ZoneOffsetMode::Directional;
    let (e, _) = rig(c.clone(), 1000.0, BUY, 4012.0);
    near(e.baskets[0].zone_lo,3995.0);
    c.zone_offset_mode = ZoneOffsetMode::None;
    let (e, _) = rig(c, 1000.0, BUY, 4012.0);
    near(e.baskets[0].zone_lo,4000.0);
}

#[test]
fn initial_text_offset_mirrors_buy_and_sell_but_preserves_first_entry() {
    let mut c = cfg(); c.entry_warstwy_z_tekstu = true;
    let buy = format!("{BUY}\nADDING 3 PIPS TO EACH LIMIT ORDER");
    let sell = format!("{SELL}\nSUBTRACTING 3 PIPS FROM EACH LIMIT ORDER");
    let (_, b) = rig(c.clone(), 1000.0, &buy, 4012.0);
    assert_eq!(prices(&b),vec![4000.3,4002.8,4005.0]);
    let (_, b) = rig(c, 1000.0, &sell, 3988.0);
    assert_eq!(prices(&b),vec![4000.0,4002.2,4004.7]);
}

#[test]
fn observed_text_offset_edit_is_not_applied_to_armed_grid() {
    let mut c = cfg(); c.entry_warstwy_z_tekstu = true; c.dedup_edited_signals = true;
    let text = format!("{BUY}\nADDING 3 PIPS TO EACH LIMIT ORDER");
    let (mut e, mut b) = rig(c, 1000.0, &text, 4012.0);
    let before = prices(&b);
    let mut edit = message(T0+1000, &text.replace("ADDING 3", "ADDING 4"));
    edit.edit_of = Some(1); e.on_message(&mut b, &edit);
    assert_eq!(e.baskets.len(),1);
    near(e.baskets[0].warstwy_offset.unwrap(),0.3);
    assert_eq!(prices(&b),before,"observed missing edit field, not correct 4-pips geometry");
}

#[test]
fn tp_only_entry_edit_updates_basket_and_armed_pending_targets() {
    let mut c = cfg();
    c.tp_schedule = TpSchedule::AllAtTp1;
    c.assign_tp_per_position = true;
    let (mut e, mut b) = rig(c, 1000.0, BUY, 4012.0);
    assert_eq!(b.pendings().len(), 3);
    assert!(b.pendings().iter().all(|p| p.tp == Some(4030.0)));
    let original_tickets: Vec<_> = b.pendings().iter().map(|p| p.ticket).collect();
    let mut edit = message(T0 + 1000, &BUY.replace("TP 4030", "TP 4031"));
    edit.edit_of = Some(1);
    e.on_message(&mut b, &edit);
    assert_eq!(e.baskets[0].tps[0], 4031.0);
    assert_eq!(b.pendings().len(), original_tickets.len());
    assert!(b.pendings().iter().all(|p| p.tp == Some(4031.0)),
        "every armed broker order must use the edited target");
    assert_ne!(original_tickets, b.pendings().iter().map(|p| p.ticket).collect::<Vec<_>>(),
        "legacy edit path must rebuild its broker plan once");
}

#[test]
fn generated_runner_targets_do_not_make_cosmetic_edit_replace_grid() {
    let mut c = cfg();
    c.runner_cele_n = 2;
    c.runner_cele_krok = 5.0;
    let (mut e, mut b) = rig(c, 1000.0, BUY, 4012.0);
    let old_prices = prices(&b);
    let old_tickets: Vec<_> = b.pendings().iter().map(|p| p.ticket).collect();
    let old_targets = e.baskets[0].tps.clone();
    let mut edit = message(T0 + 1000, &format!("{BUY}\nUPDATED NOTE"));
    edit.edit_of = Some(1);
    e.on_message(&mut b, &edit);
    assert_eq!(old_targets, e.baskets[0].tps);
    assert_eq!(old_prices, prices(&b));
    assert_eq!(b.pendings().len(), old_tickets.len());
    assert_eq!(old_tickets, b.pendings().iter().map(|p| p.ticket).collect::<Vec<_>>(),
        "unchanged source targets must preserve the exact broker order identities");
}

#[test]
fn cosmetic_entry_edit_preserves_tighter_stop_on_working_position() {
    let mut c = cfg();
    c.entry_units = 1;
    c.auto_limit = false;
    c.be_never_loosen = true;
    let (mut e, mut b) = rig(c, 1000.0, BUY, 4004.0);
    assert_eq!(b.positions().len(), 1);
    let ticket = b.positions()[0].ticket;
    b.on_quote(quote(T0 + 1000, 4008.0));
    let tp = b.positions()[0].tp;
    b.modify_position(ticket, Some(4006.0), tp).unwrap();
    assert_eq!(b.positions()[0].sl, Some(4006.0));
    let mut edit = message(T0 + 2000, &format!("{BUY}\nUPDATED NOTE"));
    edit.edit_of = Some(1);
    e.on_message(&mut b, &edit);
    assert_eq!(b.positions()[0].sl, Some(4006.0),
        "cosmetic source changes must never overwrite the tighter broker stop");
    // be_never_loosen only guards the BE paths; it does not make arbitrary
    // set_basket_sl calls safe. A no-op edit needs its own semantic contract.
}

#[test]
fn observed_nearest_lot_rounding_can_exceed_non_step_cap() {
    let mut c = cfg(); c.lot_fixed = 0.10; c.lot_max = 0.015;
    let (_, b) = rig(c, 1000.0, BUY, 4012.0);
    assert!(b.pendings().iter().all(|p| (p.volume-0.02).abs()<1e-9));
}

#[test]
fn observed_inverted_lot_bounds_swap_instead_of_enforcing_maximum() {
    let mut c = cfg(); c.lot_fixed = 0.10; c.lot_min = 0.10; c.lot_max = 0.05;
    let (_, b) = rig(c, 1000.0, BUY, 4012.0);
    assert!(b.pendings().iter().all(|p| (p.volume-0.10).abs()<1e-9));
}

#[test]
fn observed_weight_minimum_changes_total_despite_normalized_mean() {
    let mut c = cfg(); c.lot_fixed = 0.01;
    let (_, a) = rig(c.clone(), 1000.0, BUY, 4012.0);
    c.entry_weights = "1,1,4".into();
    let (_, b) = rig(c, 1000.0, BUY, 4012.0);
    near(volume(&a),0.03); near(volume(&b),0.04);
}

fn relot_cfg() -> Settings {
    let mut c = cfg(); c.entry_units=1; c.lot_mode_percent=true; c.lot_percent=1.0;
    c.pending_relot_on_balance=true; c.pending_relot_wg_planu=true;
    c.pending_relot_topup=true; c.pending_resize_s=1.0; c
}

#[test]
fn observed_relot_empty_budget_plan_leaves_old_pending_risk() {
    let mut c=relot_cfg(); c.risk_per_basket_pct=20.0;
    let (mut e,mut b)=rig(c,1000.0,BUY,4012.0);
    near(risk(&b),150.0);
    b.balance=10.0; tick(&mut e,&mut b,T0+2000,4012.0);
    near(risk(&b),150.0);
    assert!(e.stats.relot_plan_pusty>0);
    assert!(risk(&b)>b.equity()*0.20,"current empty plan does not retire old orders");
}

#[test]
fn observed_relot_topup_below_minimum_oscillates() {
    let mut c=relot_cfg(); c.lot_min=0.03;
    let (mut e,mut b)=rig(c,300.0,BUY,4012.0);
    near(volume(&b),0.03);
    b.balance=400.0;
    tick(&mut e,&mut b,T0+2000,4012.0); near(volume(&b),0.06);
    tick(&mut e,&mut b,T0+4000,4012.0); near(volume(&b),0.03);
    tick(&mut e,&mut b,T0+6000,4012.0); near(volume(&b),0.06);
}

#[test]
fn relot_up_and_down_switches_are_real_gates() {
    let mut c=relot_cfg(); c.pending_relot_up=false; c.pending_relot_down=false;
    let (mut e,mut b)=rig(c,1000.0,BUY,4012.0);
    b.balance=2000.0; tick(&mut e,&mut b,T0+2000,4012.0); near(volume(&b),0.10);
    e.cfg.pending_relot_up=true;
    tick(&mut e,&mut b,T0+4000,4012.0); near(volume(&b),0.20);
    b.balance=1000.0; tick(&mut e,&mut b,T0+6000,4012.0); near(volume(&b),0.20);
    e.cfg.pending_relot_down=true;
    tick(&mut e,&mut b,T0+8000,4012.0); near(volume(&b),0.10);
}

#[test]
fn relot_and_resize_share_interval_setting_but_not_enable_gate() {
    let mut c=relot_cfg(); c.pending_resize_on_vol=false; c.vol_window_min=0.0;
    c.pending_resize_s=10.0;
    let (mut e,mut b)=rig(c,1000.0,BUY,4012.0);
    tick(&mut e,&mut b,T0+1000,4012.0);
    b.balance=2000.0; tick(&mut e,&mut b,T0+2000,4012.0); near(volume(&b),0.10);
    tick(&mut e,&mut b,T0+11000,4012.0); near(volume(&b),0.20);
}

#[test]
fn market_single_then_market_entry_units_limits_count_not_combined_volume() {
    let mut c=cfg(); c.auto_limit=false; c.market_entry_mode=MarketEntryMode::Single;
    c.market_entry_units=1;
    let (_,b)=rig(c,1000.0,BUY,4004.0);
    assert_eq!(b.positions().len(),1);
    near(b.positions()[0].volume,0.09);
}

#[test]
fn observed_rearm_restores_frozen_initial_lot_without_relot() {
    let mut c=relot_cfg(); c.pending_relot_on_balance=false;
    c.rearm_grid_on_return=true; c.rearm_bez_pozycji=true; c.rearm_min_gap_min=0.0;
    let (mut e,mut b)=rig(c,1000.0,BUY,4012.0);
    for t in b.pendings().iter().map(|p|p.ticket).collect::<Vec<_>>() { b.cancel_pending(t).unwrap(); }
    b.balance=2000.0; tick(&mut e,&mut b,T0+2000,4003.0);
    assert_eq!(b.positions().len(),1);
    near(b.positions()[0].volume,0.10);
    near(e.lot_size(e.podstawa_lota()),0.20);
}

#[test]
fn observed_fast_addon_multiplier_bypasses_dynamic_lot_cap() {
    let mut c=cfg(); c.auto_limit=false; c.lot_fixed=0.10;
    c.lot_max_z_salda=10_000.0; c.fast_addon_move_usd=1.0;
    c.fast_addon_max=1; c.fast_addon_lot_mult=2.0; c.fast_addon_cooldown_s=0.0;
    let (mut e,mut b)=rig(c,1000.0,BUY,4004.0);
    // The engine records the momentum buffer every 5 seconds, not every tick.
    tick(&mut e,&mut b,T0+6000,4004.5);
    tick(&mut e,&mut b,T0+12000,4005.5);
    let addon=b.positions().iter().find(|p|p.level == -4).expect("active momentum add-on");
    near(addon.volume,0.20);
    assert!(addon.volume>e.podstawa_lota()/e.cfg.lot_max_z_salda);
}

#[test]
fn observed_pyramid_multiplier_bypasses_fixed_lot_cap() {
    let mut c=cfg(); c.auto_limit=false; c.entry_units=1; c.lot_max=0.03;
    c.tp_schedule=TpSchedule::AllRunners; c.assign_tp_per_position=false;
    c.pyramid_after_stage=1; c.pyramid_lot_mult=3.0;
    let (mut e,mut b)=rig(c,1000.0,BUY,4004.0);
    tick(&mut e,&mut b,T0+1000,4030.2);
    let addon=b.pendings().iter().find(|p|p.level == -3).expect("active TP pyramid");
    near(addon.volume,0.09);
    assert!(addon.volume>e.cfg.lot_max);
}

#[test]
fn allowance_is_additive_not_a_redistribution_of_base_units() {
    let mut c=cfg(); c.entry_allowance_usd=1.0; c.entry_allowance_units=2;
    let (_,b)=rig(c,1000.0,BUY,4012.0);
    assert_eq!(b.pendings().len(),5);
    near(volume(&b),0.15);
    assert_eq!(b.pendings().iter().filter(|p|p.level == 2000).count(),2);
}

#[test]
fn relative_grid_does_not_reach_shallow_edge_when_step_does_not_divide_width() {
    let mut c=cfg(); c.ppm_enabled=true; c.ppm=0.4; c.grid_anchor_absolute=false;
    let (_,b)=rig(c,1000.0,&BUY.replace("4005/4000","4004/4000"),4012.0);
    assert_eq!(prices(&b),vec![4000.0,4002.5]);
}

#[test]
fn risk_cap_still_applies_after_explicit_layout_and_allowance() {
    let mut c=cfg(); c.entry_uklad="1,2,3".into();
    c.entry_allowance_usd=1.0; c.entry_allowance_units=2; c.risk_per_basket_pct=5.0;
    let (_,b)=rig(c,1000.0,BUY,4012.0);
    assert!(!b.pendings().is_empty());
    assert!(risk(&b)<=50.0+1e-9);
}

#[test]
fn v2_caps_nonstep_limit_and_rejects_inverted_limits() {
    let mut c=cfg(); c.order_volume_contract_v2=true; c.lot_fixed=0.10;c.lot_max=0.015;
    let (_,b)=rig(c.clone(),1000.0,BUY,4012.0);
    assert_eq!(b.pendings().len(),3);
    assert!(b.pendings().iter().all(|p|(p.volume-0.01).abs()<1e-10));
    c.lot_min=0.10;c.lot_max=0.05;
    let (e,b)=rig(c,1000.0,BUY,4012.0);
    assert!(b.pendings().is_empty()&&b.positions().is_empty());
    assert!(e.odrzuty.keys().any(|k|k.starts_with("VolumeContract::")));
}

#[test]
fn v2_pyramid_final_volume_obeys_static_cap_after_multiplier() {
    let mut c=cfg();c.order_volume_contract_v2=true;c.auto_limit=false;c.entry_units=1;
    c.lot_max=0.03;c.tp_schedule=TpSchedule::AllRunners;c.assign_tp_per_position=false;
    c.pyramid_after_stage=1;c.pyramid_lot_mult=3.0;
    let (mut e,mut b)=rig(c,1000.0,BUY,4004.0);
    tick(&mut e,&mut b,T0+1000,4030.2);
    let p=b.pendings().iter().find(|p|p.level == -3).expect("pyramid should still work");
    near(p.volume,0.03);
}

#[test]
fn v2_fast_addon_obeys_dynamic_cap_after_multiplier() {
    let mut c=cfg();c.order_volume_contract_v2=true;c.auto_limit=false;c.lot_fixed=0.10;
    c.lot_max_z_salda=10_000.0;c.fast_addon_move_usd=1.0;c.fast_addon_max=1;
    c.fast_addon_lot_mult=2.0;c.fast_addon_cooldown_s=0.0;
    let (mut e,mut b)=rig(c,1000.0,BUY,4004.0);
    tick(&mut e,&mut b,T0+6000,4004.5);tick(&mut e,&mut b,T0+12000,4005.5);
    near(b.positions().iter().find(|p|p.level == -4).unwrap().volume,0.10);
}

#[test]
fn v2_does_not_promote_small_topup_or_oscillate() {
    let mut c=relot_cfg();c.order_volume_contract_v2=true;c.lot_min=0.03;
    let (mut e,mut b)=rig(c,300.0,BUY,4012.0);b.balance=400.0;
    for ms in [2000,4000,6000] {tick(&mut e,&mut b,T0+ms,4012.0);near(volume(&b),0.03);}
    assert!(e.odrzuty.contains_key("VolumeContract::BelowMinimum"));
}

fn custom_spec(c:Settings,s:conduit_core::volume_contract::VolumeSpec)->(Engine,SimBroker) {
    let mut b=SimBroker::z_ustawien(1000.0,&c);
    b.volume_min=s.minimum;b.volume_step=s.step;b.volume_max=s.maximum;
    let mut e=Engine::new(c,1000.0);tick(&mut e,&mut b,T0,4012.0);
    e.on_message(&mut b,&message(T0,BUY));(e,b)
}

#[test]
fn v2_pending_respects_broker_point001_step_without_early_cent_rounding() {
    let mut c=cfg();c.order_volume_contract_v2=true;c.lot_fixed=0.0079;c.lot_min=0.001;
    let (_,b)=custom_spec(c,conduit_core::volume_contract::VolumeSpec{minimum:0.001,step:0.001,maximum:100.0});
    assert_eq!(b.pendings().len(),3);
    assert!(b.pendings().iter().all(|p|(p.volume-0.007).abs()<1e-12));
}

#[test]
fn v2_market_respects_point001_step_without_early_cent_rounding() {
    let mut c=cfg();c.order_volume_contract_v2=true;c.auto_limit=false;c.lot_fixed=0.0079;c.lot_min=0.001;
    let (_,b)=custom_spec(c,conduit_core::volume_contract::VolumeSpec{minimum:0.001,step:0.001,maximum:100.0});
    assert_eq!(b.positions().len(),3);
    assert!(b.positions().iter().all(|p|(p.volume-0.007).abs()<1e-12));
}

#[test]
fn v2_unknown_broker_spec_is_fail_closed_but_off_is_legacy() {
    let s=conduit_core::volume_contract::VolumeSpec{minimum:0.01,step:0.01,maximum:f64::NAN};
    let mut c=cfg();let (_,off)=custom_spec(c.clone(),s);assert_eq!(off.pendings().len(),3);
    c.order_volume_contract_v2=true;let(e,on)=custom_spec(c,s);
    assert!(on.pendings().is_empty());assert!(e.odrzuty.contains_key("VolumeContract::InvalidBrokerMaximum"));
}

#[test]
fn v2_broker_cap_is_not_disabled_by_user_zero_and_minimum_does_not_raise_request() {
    let mut c=cfg();c.order_volume_contract_v2=true;c.lot_fixed=10.0;c.lot_max=0.0;
    let (_,b)=custom_spec(c.clone(),conduit_core::volume_contract::VolumeSpec{minimum:0.01,step:0.01,maximum:0.07});
    assert!(b.pendings().iter().all(|p|(p.volume-0.07).abs()<1e-12));
    c.lot_fixed=0.03;
    let(e,b)=custom_spec(c,conduit_core::volume_contract::VolumeSpec{minimum:0.10,step:0.10,maximum:100.0});
    assert!(b.pendings().is_empty());assert!(e.odrzuty.contains_key("VolumeContract::BelowMinimum"));
}

#[test]
fn v2_all_production_engine_open_calls_use_the_final_boundary() {
    let src=include_str!("../../core/src/engine.rs").split("#[cfg(test)]").next().unwrap();
    assert_eq!(src.matches("b.open_market(").count(),1,"only the guarded wrapper may call Broker");
    assert_eq!(src.matches("b.place_pending(").count(),1,"only the guarded wrapper may call Broker");
    assert!(!src.contains("b.open_market(OrderReq {"));
    assert!(!src.contains("b.place_pending(PendingReq {"));
}

#[test]
fn off_broker_cap_metadata_cannot_change_legacy_order_tape() {
    let c=cfg();let s=conduit_core::volume_contract::VolumeSpec{minimum:0.01,step:0.01,maximum:0.01};
    let(_,a)=custom_spec(c.clone(),s);
    let(_,b)=custom_spec(c,conduit_core::volume_contract::VolumeSpec{maximum:f64::NAN,..s});
    assert_eq!(serde_json::to_vec(a.pendings()).unwrap(),serde_json::to_vec(b.pendings()).unwrap());
}

#[test]
fn v2_sim_boundary_rejects_illegal_market_and_pending_without_mutation() {
    use conduit_core::broker::{BrokerError,OrderReq,PendingReq};
    let mut c=cfg();c.order_volume_contract_v2=true;
    let mut b=SimBroker::z_ustawien(1000.0,&c);b.on_quote(quote(T0,4012.0));
    for vol in [0.0,-0.01,f64::NAN,f64::INFINITY,0.015,100.01] {
        let market=OrderReq{side:Side::Buy,volume:vol,sl:None,tp:None,basket:None,level:0,is_toucher:false,comment:String::new()};
        let pending=PendingReq{kind:PendingKind::BuyLimit,volume:vol,price:4005.0,sl:None,tp:None,basket:None,level:0,is_toucher:false,is_topup:false,comment:String::new()};
        assert_eq!(b.open_market(market),Err(BrokerError::InvalidVolume));
        assert_eq!(b.place_pending(pending),Err(BrokerError::InvalidVolume));
        assert!(b.positions().is_empty()&&b.pendings().is_empty());near(b.balance,1000.0);
    }
}

#[test]
fn v2_relot_supports_point001_topup_without_cent_rounding() {
    let mut c=relot_cfg();c.order_volume_contract_v2=true;c.lot_min=0.001;
    let mut b=SimBroker::z_ustawien(70.0,&c);b.volume_min=0.001;b.volume_step=0.001;
    let mut e=Engine::new(c,70.0);tick(&mut e,&mut b,T0,4012.0);
    e.on_message(&mut b,&message(T0,BUY));near(volume(&b),0.007);
    b.balance=80.0;tick(&mut e,&mut b,T0+2000,4012.0);
    near(volume(&b),0.008);
    assert!(b.pendings().iter().any(|p|p.is_topup&&(p.volume-0.001).abs()<1e-12));
}

#[test]
fn v2_illegal_relot_replacement_does_not_cancel_existing_base() {
    let mut c=relot_cfg();c.order_volume_contract_v2=true;c.pending_relot_topup=false;
    let (mut e,mut b)=rig(c,1000.0,BUY,4012.0);
    let old=serde_json::to_vec(b.pendings()).unwrap();
    b.volume_max=f64::NAN;b.balance=2000.0;tick(&mut e,&mut b,T0+2000,4012.0);
    assert_eq!(old,serde_json::to_vec(b.pendings()).unwrap());
    assert!(e.odrzuty.contains_key("VolumeContract::InvalidBrokerMaximum"));
}

#[test]
fn v2_preflight_is_explicit_and_off_diagnostics_remain_legacy() {
    let mut c=cfg();c.lot_min=0.10;c.lot_max=0.05;
    assert!(!c.pulapki_konfiguracji().iter().any(|s|s.contains("order_volume_contract_v2")));
    c.order_volume_contract_v2=true;
    let warnings=c.pulapki_konfiguracji();
    assert!(warnings.iter().any(|s|s.contains("fail-closed")));
    assert!(warnings.iter().any(|s|s.contains("nie limit brokera")));
}
