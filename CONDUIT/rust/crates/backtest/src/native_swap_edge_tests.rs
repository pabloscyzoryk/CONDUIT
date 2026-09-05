//! Synthetic swap differential and explicit execution-profile gates.
//! The geometry below is generated regression data; no market export, strategy
//! engine, external analysis directory, or broker account is required.
use crate::SimBroker;
use conduit_core::{broker::*, settings::Settings, types::*};

const OPEN: Quote = Quote {ts:1_893_618_000_017,bid:4000.00,ask:4000.20};
const PRE: Quote = Quote {ts:1_893_621_600_003,bid:3995.95,ask:3996.18};
const TRIPLE: Quote = Quote {ts:1_893_636_000_016,bid:4011.50,ask:4011.73};
const NEXT: Quote = Quote {ts:1_893_722_400_024,bid:4012.88,ask:4013.11};
const NATIVE_SWAP:[f64;8]=[0.0,0.0,-16.91,6.86,-4.83,1.96,-9.67,3.92];
const NATIVE_GROSS:[f64;8]=[-8.50,7.64,79.10,-82.11,22.60,-23.46,38.04,-39.33];

struct Probe {
    broker:SimBroker,
    before_triple_close_balance:f64,
    before_triple_close_equity:f64,
    close_cash_deltas:Vec<f64>,
}
fn near(a:f64,b:f64,why:&str){assert!((a-b).abs()<1e-8,"{why}: actual {a:.12}, native/expected {b:.12}");}
fn settings(net:bool,commission:f64)->Settings {
    Settings {closed_profit_net_costs:net,basket_realized_broker_only:true,
        commission_per_lot:commission,swap_enabled:true,swap_long_points:-80.54,swap_short_points:32.67,
        swap_point_value:1.0,swap_rollover_z_serwera:true,swap_rollover3days_mt5:3,
        swap_rollover_mult:3.0,swap_pomijaj_weekend:false,
        server_tz_offset_ms:0,slippage_pts:0.0,..Settings::default()}
}
fn run_with(native:Option<u32>,net:bool,commission:f64)->Probe {
    let cfg=settings(net,commission);
    let mut b=SimBroker::z_ustawien(5000.0,&cfg);
    b.set_native_swap_cash_digits(native).unwrap();
    b.price_digits=Some(2);
    b.mark(OPEN);
    let mut ids=vec![];
    for (i,side) in [Side::Buy,Side::Sell,Side::Buy,Side::Sell].into_iter().enumerate() {
        ids.push(b.open_market(OrderReq {side,volume:0.07,sl:None,tp:None,basket:Some(i as u32+1),
            level:0,is_toucher:false,comment:format!("native-edge-{i}")}).unwrap());
    }
    let mut deltas=vec![];
    b.mark(PRE);
    for &id in &ids[2..] {
        let before=b.balance;b.close_partial(id,0.02,CloseReason::Partial).unwrap();deltas.push(b.balance-before);
    }
    b.mark(TRIPLE);
    let before_triple_close_balance=b.balance;
    let before_triple_close_equity=b.account().equity;
    for (i,&id) in ids.iter().enumerate() {
        let before=b.balance;
        if i<2 {b.close_position(id,CloseReason::Manual).unwrap();}
        else {b.close_partial(id,0.02,CloseReason::Partial).unwrap();}
        deltas.push(b.balance-before);
    }
    b.mark(NEXT);
    for &id in &ids[2..] {
        let before=b.balance;b.close_position(id,CloseReason::Manual).unwrap();deltas.push(b.balance-before);
    }
    Probe {broker:b,before_triple_close_balance,before_triple_close_equity,close_cash_deltas:deltas}
}

#[test]
fn synthetic_geometry_and_internal_net_cash_are_consistent() {
    let p=run_with(Some(2),true,0.0);let b=&p.broker;
    assert_eq!(b.history.len(),8);assert!(b.positions().is_empty());
    let mut net=0.0;
    for (i,tr) in b.history.iter().enumerate() {
        near(tr.cost_receipt.as_ref().unwrap().gross_profit.unwrap(),NATIVE_GROSS[i],"gross executable price geometry");
        net+=tr.canonical_net().unwrap();
        assert_eq!(tr.open_ts,OPEN.ts);
        assert_eq!(tr.close_ts,if i<2{PRE.ts}else if i<6{TRIPLE.ts}else{NEXT.ts});
        near(tr.open_price,if tr.side==Side::Buy{OPEN.ask}else{OPEN.bid},"entry");
    }
    near(net,b.balance-5000.0,"intrinsic canonical net vs simulator cash");
    let value=serde_json::json!({"status":"ACTUAL_SIM_NATIVE_SWAP_PROFILE_ON",
        "before_triple_close_balance":p.before_triple_close_balance,
        "native_before_triple_close_balance":4999.14,
        "before_triple_close_equity":p.before_triple_close_equity,
        "native_before_triple_close_equity":4976.75,
        "close_cash_deltas":p.close_cash_deltas,
        "native_close_cash_deltas":(0..8).map(|i|NATIVE_GROSS[i]+NATIVE_SWAP[i]).collect::<Vec<_>>(),
        "native_swaps":NATIVE_SWAP,"history":b.history,"swap_total":b.swap_total,
        "final_balance":b.balance,"native_final_balance":4975.31});
    assert_eq!(value["status"], "ACTUAL_SIM_NATIVE_SWAP_PROFILE_ON");
    assert_eq!(value["history"].as_array().map(Vec::len), Some(8));
}

#[test]
fn signed_swap_and_partial_allocation_match_expected_cents() {
    for net in [false,true] {
    let p=run_with(Some(2),net,0.0);
    for (i,tr) in p.broker.history.iter().enumerate() {
        near(tr.swap,NATIVE_SWAP[i],&format!("deal index {i} swap"));
    }
    near(p.broker.balance,4975.31,"final cash");
    }
}

#[test]
fn unrealized_swap_does_not_change_balance_before_close() {
    for net in [false,true] {
    let p=run_with(Some(2),net,0.0);
    near(p.before_triple_close_balance,4999.14,"native retains accrued swap outside Balance before closure");
    near(p.before_triple_close_equity,4976.75,"native equity includes accrued swap");
    }
}

#[test]
fn each_close_cash_delta_equals_gross_plus_realized_swap() {
    for net in [false,true] {
    let p=run_with(Some(2),net,0.0);
    for (i,delta) in p.close_cash_deltas.iter().enumerate() {
        near(*delta,NATIVE_GROSS[i]+NATIVE_SWAP[i],&format!("deal index {i} immediate cash delta"));
    }
    }
}

#[test]
fn profile_off_preserves_deterministic_cash_and_trade_values() {
    let p=run_with(None,true,0.0);
    assert_eq!(p.broker.history.len(),8);
    assert!(p.broker.positions().is_empty());
    near(p.broker.balance,4975.3107,"profile-off final balance");
    near(p.before_triple_close_balance,4981.9068,"profile-off intermediate balance");
    let expected:[f64;8]=[-8.5,7.640000000001237,79.0999999999949,-82.10999999999694,
        22.599999999998545,-23.459999999999127,38.039999999998145,-39.32999999999902];
    for (actual, expected) in p.close_cash_deltas.iter().zip(expected) {
        near(*actual, expected, "profile-off close cash delta");
    }
}

#[test]
fn profile_validation_is_atomic_pristine_only_and_not_enabled_by_net_or_price() {
    let mut b=SimBroker::z_ustawien(5000.0,&settings(true,0.0));b.price_digits=Some(2);
    assert_eq!(b.native_swap_cash_digits(),None);
    for invalid in [9,u32::MAX] {assert!(b.set_native_swap_cash_digits(Some(invalid)).is_err());assert_eq!(b.native_swap_cash_digits(),None);}
    b.set_native_swap_cash_digits(Some(0)).unwrap();b.set_native_swap_cash_digits(Some(8)).unwrap();
    b.set_native_swap_cash_digits(Some(2)).unwrap();b.mark(OPEN);
    assert!(b.set_native_swap_cash_digits(None).is_err());assert!(b.set_native_swap_cash_digits(Some(3)).is_err());
    b.set_native_swap_cash_digits(Some(2)).unwrap();assert_eq!(b.native_swap_cash_digits(),Some(2));
    let mut old=SimBroker::new(5000.0,0.2,0.0);old.mark(OPEN);
    assert!(old.set_native_swap_cash_digits(Some(2)).is_err());assert_eq!(old.native_swap_cash_digits(),None);
    let mut epoch=SimBroker::new(5000.0,0.2,0.0);epoch.mark(Quote{ts:0,bid:4000.0,ask:4000.2});
    assert!(epoch.set_native_swap_cash_digits(Some(2)).is_err());assert_eq!(epoch.native_swap_cash_digits(),None);
}

#[test]
fn native_swap_profile_does_not_double_charge_entry_commission_pool() {
    let p=run_with(Some(2),true,7.0);
    near(p.broker.history.iter().map(|t|t.commission).sum(),-1.96,"four entries .07*7");
    near(p.broker.history.iter().map(|t|t.swap).sum(),-18.67,"native swaps");
    near(p.broker.balance,4975.31-1.96,"cash charges entry commission once");
    near(p.broker.history.iter().map(|t|t.canonical_net().unwrap()).sum(),p.broker.balance-5000.0,"canonical net");
    assert!(p.broker.cost_reconciliation_required().is_none());
}

#[test]
fn explicit_precision_boundaries_preserve_residual_conservation() {
    // Synthetic precision controls, not additional native-currency evidence.
    for digits in [0,8] {
        let p=run_with(Some(digits),true,0.0);
        let factor=10f64.powi(digits as i32);
        for tr in &p.broker.history {near(tr.swap*factor,(tr.swap*factor).round(),"configured swap quantum");}
        near(p.broker.history.iter().map(|tr|tr.canonical_net().unwrap()).sum(),p.broker.balance-5000.0,"precision residual conservation");
        assert!(p.broker.cost_reconciliation_required().is_none());
    }
}

fn one(net:bool,side:Side)->(SimBroker,Ticket) {
    let mut b=SimBroker::z_ustawien(5000.0,&settings(net,0.0));b.set_native_swap_cash_digits(Some(2)).unwrap();
    b.mark(Quote {ts:OPEN.ts,bid:4000.0,ask:4000.2});
    let ticket=b.open_market(OrderReq {side,volume:0.07,sl:None,tp:None,basket:None,level:0,is_toucher:false,comment:"settlement".into()}).unwrap();
    (b,ticket)
}

#[test]
fn zero_and_disabled_swap_preserve_cash_and_do_not_fabricate_cost() {
    for enabled in [false,true] {for net in [false,true] {
        let (mut b,t)=one(net,Side::Buy);b.swap_enabled=enabled;b.swap_long_points=0.0;
        b.mark(Quote{ts:TRIPLE.ts,bid:4001.0,ask:4001.2});near(b.balance,5000.0,"no accrued cash");
        b.close_position(t,CloseReason::Manual).unwrap();near(b.history[0].swap,0.0,"zero swap");
        near(b.history[0].profit,b.balance-5000.0,"closed cash");
    }}
}

#[test]
fn sl_tp_stopout_and_eod_use_the_same_native_swap_settlement() {
    for net in [false,true] {for side in [Side::Buy,Side::Sell] {for exit in 0..4 {
        let (mut b,t)=one(net,side);
        b.mark(Quote{ts:TRIPLE.ts,bid:4000.0,ask:4000.2});
        near(b.balance,5000.0,"swap before automatic exit stays accrued");
        match exit {
            0=>{
                b.modify_position(t,Some(if side==Side::Buy{3999.0}else{4001.0}),None).unwrap();
                let bid=if side==Side::Buy{3998.0}else{4002.0};
                b.on_quote(Quote{ts:TRIPLE.ts+1,bid,ask:bid+0.2});
                assert_eq!(b.history[0].reason,CloseReason::Sl);
            },
            1=>{
                b.modify_position(t,None,Some(if side==Side::Buy{4001.0}else{3999.0})).unwrap();
                let bid=if side==Side::Buy{4002.0}else{3998.0};
                b.on_quote(Quote{ts:TRIPLE.ts+1,bid,ask:bid+0.2});
                assert_eq!(b.history[0].reason,CloseReason::Tp);
            },
            2=>{
                b.stop_out_level_pct=1e12;
                b.on_quote(Quote{ts:TRIPLE.ts+1,bid:4000.0,ask:4000.2});
                assert_eq!(b.history[0].reason,CloseReason::MaxDd);
            },
            _=>{b.close_position(t,CloseReason::EodFlat).unwrap();}
        }
        assert!(b.positions().is_empty());assert_eq!(b.history.len(),1);
        near(b.history[0].swap,if side==Side::Buy{-16.91}else{6.86},"one native swap");
        near(b.history[0].profit,b.balance-5000.0,"exit cash vs receipt");
        near(b.account().equity,b.balance,"no residual swap after flat");
        assert!(b.cost_reconciliation_required().is_none());
    }}}
}

#[test]
fn invalid_native_rate_holds_entries_but_keeps_protective_close_available() {
    for net in [false,true] {
    let (mut b,t)=one(net,Side::Buy);b.swap_long_points=f64::NAN;
    b.mark(Quote{ts:TRIPLE.ts,bid:4000.0,ask:4000.2});
    assert!(b.cost_reconciliation_required().is_some());assert!(b.balance.is_finite());
    assert!(b.close_position(t,CloseReason::Manual).is_ok());
    assert!(b.history.is_empty());assert_eq!(b.quarantined_cost_trades().len(),1);
    assert!(b.open_market(OrderReq{side:Side::Buy,volume:0.01,sl:None,tp:None,basket:None,level:0,is_toucher:false,comment:"fault".into()}).is_err());
    }
}
