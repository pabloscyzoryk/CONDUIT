use super::*;
use crate::lot_growth::{LotGrowthMode,LotGrowthAllocation};
use crate::profit_budget::tests::{broker,TestBroker};

fn cfg()->Settings {Settings {lot_growth_mode:LotGrowthMode::Power,lot_growth_reference_lot:0.08,
    lot_growth_reference_balance:700.0,lot_growth_power:1.0,lot_growth_allocation:LotGrowthAllocation::Depth,
    entry_units:2,entry_units_limit:2,units_per_level:false,units_per_level_zone:false,
    grid_anchor_absolute:false,lot_max:0.0,risk_per_basket_pct:0.0,max_portfolio_risk_pct:0.0,
    max_open_positions:0,max_open_baskets:0,pending_resize_s:0.0,
    ..Settings::default()}}
fn rig(c:Settings)->(Engine,TestBroker) {
    let mut b=broker();b.authoritative_cancel=true;let mut e=Engine::new(c,700.0);
    let ts=b.q.ts;
    e.on_message(&mut b,&IncomingMessage{ts,source:SourceKey::new(-900071,None),
        source_name:"synthetic-growth".into(),msg_id:1,reply_to:None,edit_of:None,
        text:"BUY LIMIT GOLD @ 3990/3980\nSL 3970\nTP 4050".into()});
    (e,b)
}
#[test]
fn lot_growth_pending_price_bound_flag_preserves_legacy_json_and_recorded_replay() {
    use crate::recorded_broker::{Recorder,ReplayBroker,Trace};
    for enabled in [false,true] {
        let mut c=cfg();
        if !enabled {c.lot_growth_mode=LotGrowthMode::Off;}
        let mut e=Engine::new(c.clone(),700.0);
        let mut b=broker();
        let request=PendingReq{kind:PendingKind::BuyLimit,price:3990.0,volume:0.02,
            sl:Some(3980.0),tp:None,basket:None,level:0,is_toucher:false,is_topup:false,
            no_market_fallback:false,comment:"synthetic price-bound request".into()};
        let legacy=serde_json::to_value(&request).unwrap();
        assert!(legacy.get("no_market_fallback").is_none());
        let restored:PendingReq=serde_json::from_value(legacy.clone()).unwrap();
        assert!(!restored.no_market_fallback);
        assert_eq!(serde_json::to_value(restored).unwrap(),legacy);
        let (trace,ticket)={
            let mut recorder=Recorder::new(&mut b);
            let ticket=e.place_pending_order(&mut recorder,request.clone()).unwrap();
            (recorder.finish(),ticket)
        };
        let sent=trace.calls.iter().find(|call|call.method=="place_pending").unwrap();
        let actual:PendingReq=crate::recorded_broker::exact::decode(&trace.values[sent.args]).unwrap();
        assert_eq!(actual.no_market_fallback,enabled);
        let wire=serde_json::to_value(&actual).unwrap();
        assert_eq!(wire.get("no_market_fallback").cloned(),enabled.then_some(serde_json::Value::Bool(true)));
        let stored:Trace=serde_json::from_slice(&serde_json::to_vec(&trace).unwrap()).unwrap();
        let mut replay=ReplayBroker::new(stored).unwrap();
        let mut e2=Engine::new(c,700.0);
        assert_eq!(e2.place_pending_order(&mut replay,request),Ok(ticket));
        replay.finish().unwrap();
    }
}
#[test]
fn lot_growth_real_entry_weights_only_volumes_and_off_ignores_all_growth_parameters() {
    let mut c=cfg();c.lot_growth_mode=LotGrowthMode::Off;c.lot_mode_percent=false;c.lot_fixed=0.08;
    let (e,b)=rig(c.clone());
    c.lot_growth_power=f64::NAN;c.lot_growth_reference_balance=-1.0;c.lot_growth_basket_risk_pct=f64::NAN;
    c.lot_growth_allocation=LotGrowthAllocation::ExposureAwareRisk;
    let (e2,b2)=rig(c);
    assert_eq!(serde_json::to_value(&b.pendings).unwrap(),serde_json::to_value(&b2.pendings).unwrap());
    assert_eq!(e.odrzuty,e2.odrzuty);
    let (weighted,wb)=rig(cfg());assert_eq!(wb.pendings.len(),b.pendings.len());
    assert!(!wb.pendings.is_empty());
    for (a,w) in b.pendings.iter().zip(&wb.pendings) {
        assert_eq!((a.kind,a.price,a.sl,a.tp,a.level),(w.kind,w.price,w.sl,w.tp,w.level));
        let bk=&weighted.baskets[0];
        let depth=(bk.entry_hi-w.price)/(bk.entry_hi-bk.entry_lo);
        let want=crate::volume_contract::normalize_open_volume(0.01+(a.volume-0.01)*(0.5+depth),
            crate::volume_contract::VolumeSpec{minimum:0.01,step:0.01,maximum:100.0},weighted.volume_limits()).unwrap();
        assert_eq!(w.volume,want);
    }
}
#[test]
fn lot_growth_real_relot_topup_and_replacement_allocate_target_once() {
    for reconcile in [false,true] {for topup in [false,true] {for planned in [false,true] {
        let mut c=cfg();c.pending_relot_on_balance=true;c.pending_relot_topup=topup;
        c.pending_relot_reconcile_target=reconcile;c.pending_relot_wg_planu=planned;
        let (mut e,mut b)=rig(c);assert_eq!(b.pendings.len(),2,"fixture needs two real limit levels");
        let bk=&e.baskets[0];
        let before:HashMap<i32,(f64,f64)>=b.pendings.iter().map(|p|{
            let depth=(bk.entry_hi-p.price)/(bk.entry_hi-bk.entry_lo);
            let target=(0.01+(0.16-0.01)*(0.5+depth))*100.0;
            (p.level,(p.volume,(target+1e-12).floor()/100.0))
        }).collect();
        b.equity=1400.0;e.stats.balance=1400.0;e.stats.equity=1400.0;
        let ts=b.q.ts+10_000;e.relot_pendings(&mut b,ts);
        for (level,(old,target)) in before {
            let total:f64=b.pendings.iter().filter(|p|p.level==level).map(|p|p.volume).sum();
            assert!((total-target).abs()<1e-9,"reconcile={reconcile} topup={topup} planned={planned} level={level}: {old} -> {total}, target {target}");
        }
        let count=b.sends;e.relot_pendings(&mut b,ts+10_000);
        assert_eq!(b.sends,count,"stable weighted target must not repeatedly top up: reconcile={reconcile} topup={topup} planned={planned}");
    }}}
}
#[test]
fn lot_growth_strict_floor_has_no_legacy_roundup_or_scale_max() {
    let mut c=cfg();c.lot_growth_mode=LotGrowthMode::GeometricSteps;
    c.lot_growth_reference_lot=0.01;c.lot_growth_reference_balance=350.0;c.lot_scale_step=1.0;
    c.lot_growth_allocation=LotGrowthAllocation::Uniform;
    let (e,b)=rig(c);assert_eq!(e.lot_size(700.0),0.015);
    assert!(b.pendings.iter().all(|p|p.volume==0.01));
    let mut c=cfg();c.lot_growth_reference_lot=0.01;
    let (_,b)=rig(c);assert_eq!(b.pendings.len(),2,"surplus allocation preserves the minimum leg");
    assert!(b.pendings.iter().all(|p|p.volume==0.01));
}
#[test]
fn lot_growth_every_send_rechecks_actual_basket_capacity_and_t100_is_rejected() {
    let mut c=cfg();c.lot_growth_allocation=LotGrowthAllocation::Uniform;c.lot_growth_basket_risk_pct=5.0;
    let mut b=broker();let mut e=Engine::new(c,700.0);
    let pending=PendingReq{kind:PendingKind::BuyLimit,price:3990.0,volume:1.0,sl:Some(3980.0),
        tp:None,basket:Some(1),level:0,is_toucher:false,is_topup:false,no_market_fallback:false,comment:"synthetic".into()};
    e.place_pending_order(&mut b,pending.clone()).unwrap();assert_eq!(b.pendings[0].volume,0.03);
    let mut topup=pending.clone();topup.is_topup=true;
    assert!(e.place_pending_order_allocated(&mut b,topup,true).is_err());assert_eq!(b.sends,1);
    b.cancel_pending(1).unwrap();e.place_pending_order(&mut b,pending.clone()).unwrap();
    e.cfg.t100.enabled=true;b.pendings.clear();
    assert!(e.place_pending_order(&mut b,pending).is_err());
    assert_eq!(crate::lot_growth::ready(&e.cfg,&b),Err(crate::lot_growth::GrowthError::UnsupportedEngine));
}

#[test]
fn lot_growth_uncertain_receipts_cannot_reclaim_invisible_exposure() {
    let mut e=Engine::new(cfg(),700.0);let mut b=broker();
    let r=PendingReq{kind:PendingKind::BuyLimit,price:3990.0,volume:0.08,sl:Some(3980.0),
        tp:Some(4010.0),basket:Some(1),level:0,is_toucher:false,is_topup:false,no_market_fallback:false,comment:"synthetic".into()};
    for barrier in [ReceiptBarrier::Temporary,ReceiptBarrier::RequiresReview] {
        b.barrier=barrier;assert!(e.place_pending_order(&mut b,r.clone()).is_err());assert_eq!(b.sends,0);
    }
    b.barrier=ReceiptBarrier::Clear;e.place_pending_order(&mut b,r).unwrap();assert_eq!(b.sends,1);
}

#[test]
fn lot_growth_initial_plan_preserves_affordable_minimum_subset_and_rechecks_total_risk() {
    let mut c=cfg();c.lot_growth_mode=LotGrowthMode::Off;c.lot_mode_percent=false;c.lot_fixed=0.01;
    let (mut legacy,_)=rig(c.clone());
    let plan=legacy.baskets[0].levels.clone();assert_eq!(plan.len(),2);
    assert!(plan.iter().all(|g|g.volume==0.01));
    legacy.stats.equity=100.0;legacy.cfg.risk_per_basket_pct=20.0;
    c.lot_growth_mode=LotGrowthMode::Power;c.lot_growth_reference_lot=0.01;c.risk_per_basket_pct=20.0;
    let growth=Engine::new(c,100.0);
    let (mut old,mut new)=(plan.clone(),plan);
    legacy.cap_basket_risk(&mut old,Some(3970.0),None);
    growth.cap_basket_risk(&mut new,Some(3970.0),None);
    assert_eq!(old.len(),1);assert_eq!(new.len(),1);
    assert_eq!((new[0].price,new[0].volume),(3980.0,0.01));
    assert_eq!(serde_json::to_value(&old).unwrap(),serde_json::to_value(&new).unwrap());
    let risk:f64=new.iter().map(|g|(g.price-3970.0).abs()*XAU_CONTRACT*g.volume*g.base_units.max(1) as f64).sum();
    assert!(risk<=20.0);
    growth.cap_basket_risk(&mut new,Some(3970.0),Some(5.0));
    assert!(new.is_empty(),"a minimum leg cannot bypass an unaffordable plan budget");
}

#[test]
fn lot_growth_scaled_plan_roundoff_does_not_remove_an_affordable_level() {
    // Synthetic seven-level geometry reproduces a one-ULP sum overshoot after
    // proportional scaling. No message, identity or original market input is used.
    let (mut e,_)=rig(cfg());e.cfg.risk_per_basket_pct=20.0;e.stats.equity=270.24;
    let template=e.baskets[0].levels[0].clone();
    let plan:Vec<_>=(0..7).map(|i|{let mut g=template.clone();g.level=i;
        g.price=4000.0+8.0*(i+1) as f64/7.0;g.volume=0.03;g.base_units=1;g.is_toucher=false;g}).collect();
    let risk=|levels:&Vec<GridLevel>|levels.iter().map(|g|(g.price-4000.0).abs()*XAU_CONTRACT*g.volume).sum::<f64>();
    let cap=e.stats.equity*20.0/100.0;
    let mut scaled=plan.clone();let factor=cap/risk(&scaled);
    for g in &mut scaled {g.volume*=factor;}
    assert_eq!(risk(&scaled).to_bits(),cap.to_bits()+1,"fixture must exercise actual arithmetic noise");
    assert!(!crate::lot_growth::plan_risk_exceeds(risk(&scaled),cap));
    assert!(crate::lot_growth::plan_risk_exceeds(cap+0.01,cap));
    assert!(crate::lot_growth::plan_risk_exceeds(f64::NAN,cap));
    assert!(crate::lot_growth::plan_risk_exceeds(cap,f64::INFINITY));
    let mut growth=plan.clone();e.cap_basket_risk(&mut growth,Some(4000.0),None);
    assert_eq!(growth.len(),7,"a representation-only excess cannot remove a strategy level");
    assert_eq!(risk(&growth).to_bits(),cap.to_bits()+1);
    e.cfg.lot_growth_mode=LotGrowthMode::Off;
    let mut old=plan;e.cap_basket_risk(&mut old,Some(4000.0),None);
    assert_eq!(old.len(),6,"legacy rounding and comparisons remain unchanged");
    e.cfg.lot_growth_mode=LotGrowthMode::Power;e.stats.equity=159.95;
    for g in &mut scaled {g.volume=0.01;}
    e.cap_basket_risk(&mut scaled,Some(4000.0),None);
    assert_eq!(scaled.len(),6,"a real one-cent excess still prunes the minimum plan");
    assert!(risk(&scaled)<=e.stats.equity*20.0/100.0);
}

#[test]
fn lot_growth_minimum_plateau_preserves_entries_across_300_curve_profiles_with_all_contexts() {
    let allocations=[LotGrowthAllocation::Uniform,LotGrowthAllocation::EqualSLRisk,LotGrowthAllocation::Depth,
        LotGrowthAllocation::EqualSLRiskDepth,LotGrowthAllocation::ExposureAwareRisk];
    let mut count=0;
    for mode in [LotGrowthMode::Power,LotGrowthMode::ThresholdLinear,LotGrowthMode::GeometricSteps] {
        for shape in 0..5 {for anchor in [600.0,1000.0,1500.0,2500.0] {for allocation in allocations {
            let mut c=cfg();c.lot_growth_mode=mode;c.lot_growth_allocation=allocation;
            c.lot_growth_reference_lot=0.01;c.lot_growth_reference_balance=anchor;
            c.lot_growth_power=[0.4,0.55,0.7,0.85,1.0][shape];
            c.lot_growth_rate_pct=[0.15,0.25,0.35,0.5,0.75][shape];
            c.lot_growth_capital_multiple=[1.5,1.75,2.0,2.5,3.0][shape];
            c.risk_per_basket_pct=20.0;
            c.lot_growth_equity_stress_strength=2.0;c.lot_growth_portfolio_load_strength=2.0;
            c.lot_growth_direction_load_strength=2.0;c.lot_growth_basket_count_strength=2.0;
            c.lot_growth_spread_stress_strength=2.0;c.lot_growth_tp1_deficit_strength=2.0;
            c.lot_growth_stop_width_strength=2.0;c.lot_growth_age_decay_strength=2.0;
            c.lot_growth_rearm_decay_strength=2.0;c.lot_growth_day_dd_strength=2.0;
            let mut b=broker();b.equity=300.0;let mut e=Engine::new(c,300.0);
            let q=b.q;e.on_tick(&mut b,&q);
            e.on_message(&mut b,&IncomingMessage{ts:q.ts,source:SourceKey::new(-900073,None),
                source_name:"synthetic-growth".into(),msg_id:1,reply_to:None,edit_of:None,
                text:"BUY LIMIT GOLD @ 3990/3980\nSL 3970\nTP 4050".into()});
            assert_eq!(b.pendings.len(),2,"{mode:?}/{shape}/{anchor}/{allocation:?}: {:?}",e.odrzuty);
            assert!(b.pendings.iter().all(|p|p.volume==0.01));count+=1;
        }}}
    }
    assert_eq!(count,300);
}
