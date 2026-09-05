//! Actual Sim + public Engine cost contract. No terminal, TCP or real orders.
use crate::SimBroker;
use conduit_core::{broker::*,cost_receipt::*,engine::{Engine,IncomingMessage},settings::*,types::*};
const DAY:i64=86_400_000;
const T:i64=(1_700_000_000_000/DAY)*DAY+3_600_000;
fn q(ts:i64,bid:f64)->Quote{Quote{ts,bid,ask:bid+0.2}}
fn near(a:f64,b:f64){assert!((a-b).abs()<1e-8,"{a} != {b}");}
fn config()->Settings{Settings{closed_profit_net_costs:true,basket_realized_broker_only:true,
    commission_per_lot:7.0,swap_enabled:false,server_tz_offset_ms:0,exec_latency_ms:0,
    max_dd_pct:0.0,max_dd_usd:0.0,equity_floor_pct:0.0,..Settings::default()}}
fn sim()->SimBroker{let mut b=SimBroker::z_ustawien(1000.0,&config());b.mark(q(T,4000.0));b}
fn req(volume:f64)->OrderReq{OrderReq{side:Side::Buy,volume,sl:None,tp:None,basket:None,
    level:0,is_toucher:false,comment:"cost-test".into()}}
fn receipt()->CostReceipt{CostReceipt{schema:CostSchema::V1,key:CostReceiptKey{scope_id:"test".into(),deal_id:1},
    position_identifier:1,volume:0.07,currency:"USD".into(),source:CostSource::SimulatorLedger{run_id:"test".into(),cost_spec_hash:"test-spec".into()},
    gross_profit:Some(10.0),entry_commission_alloc:Some(-0.49),exit_commission:Some(0.2),
    entry_fee_alloc:Some(-0.03),exit_fee:Some(-0.04),swap:Some(-0.7),completeness:CostCompleteness::Complete,entry_allocation:None}}
fn legacy()->ClosedTrade{ClosedTrade{ticket:1,side:Side::Buy,volume:0.07,open_price:4000.0,close_price:4001.0,
    open_ts:T,close_ts:T+1,profit:10.0,commission:0.0,swap:0.0,reason:CloseReason::Manual,basket:None,
    profit_basis:None,cost_receipt:None}}

#[test]fn canonical_projection_and_signed_rebate_not_charged_again(){
    let c=legacy().with_cost_receipt(receipt()).unwrap();near(c.profit,8.94);near(c.commission,-0.29);near(c.swap,-0.7);
    near(c.canonical_net().unwrap(),8.94);
}
#[test]fn legacy_serde_is_exactly_old_shape_and_not_canonical(){
    let c=legacy();let old=format!("{{\"ticket\":1,\"side\":\"Buy\",\"volume\":0.07,\"open_price\":4000.0,\"close_price\":4001.0,\"open_ts\":{T},\"close_ts\":{},\"profit\":10.0,\"commission\":0.0,\"swap\":0.0,\"reason\":\"Manual\",\"basket\":null}}",T+1);
    assert_eq!(serde_json::to_string(&c).unwrap(),old);assert!(c.canonical_net().is_err());
    let restored:ClosedTrade=serde_json::from_str(&old).unwrap();assert!(restored.profit_basis.is_none());
}
#[test]fn canonical_rejects_missing_incomplete_mismatched_or_tampered_receipt(){
    let mut missing=legacy();missing.profit_basis=Some(ProfitBasis::CanonicalClosedNetV1);assert!(missing.canonical_net().is_err());
    let mut r=receipt();r.entry_fee_alloc=None;r.completeness=CostCompleteness::Incomplete{issues:vec![CostIssue::MissingEntryHistory]};
    assert!(legacy().with_cost_receipt(r).is_err());
    let mut r=receipt();r.volume=0.08;assert!(legacy().with_cost_receipt(r).is_err());
    let mut tr=legacy().with_cost_receipt(receipt()).unwrap();tr.profit+=1.0;assert!(tr.canonical_net().is_err());
    let mut tr=legacy().with_cost_receipt(receipt()).unwrap();tr.commission+=1.0;assert!(tr.canonical_net().is_err());
    let mut tr=legacy().with_cost_receipt(receipt()).unwrap();tr.volume=0.0;assert!(tr.canonical_net().is_err());
    let mut r=receipt();r.gross_profit=Some(-1e308);r.entry_commission_alloc=Some(1e308);r.exit_commission=Some(1e308);r.entry_fee_alloc=Some(-1e308);r.exit_fee=Some(0.0);r.swap=Some(0.0);
    near(r.net().unwrap(),0.0);assert!(legacy().with_cost_receipt(r.clone()).is_err());
    let mut tr=legacy();tr.profit=0.0;tr.profit_basis=Some(ProfitBasis::CanonicalClosedNetV1);tr.cost_receipt=Some(Box::new(r));assert!(tr.canonical_net().is_err());
}
#[test]fn full_close_net_equals_cash_cycle_but_rpc_remains_gross_delta(){
    let mut b=sim();let t=b.open_market(req(0.07)).unwrap();near(b.balance,999.51);
    b.mark(q(T+1,4001.0));let before=b.balance;let rpc=b.close_position(t,CloseReason::Manual).unwrap();
    near(rpc,b.balance-before);near(b.history[0].profit,b.balance-1000.0);
    assert_ne!(rpc,b.history[0].profit);assert!(b.history[0].canonical_net().is_ok());
}
#[test]fn positive_and_negative_swap_are_allocated_once(){
    for rate in [-10.0,10.0]{let mut b=sim();b.swap_enabled=true;b.swap_long_points=rate;b.swap_rollover_mult=1.0;b.mark(q(T,4000.0));
        let t=b.open_market(req(0.07)).unwrap();b.mark(q(T+DAY,4001.0));b.close_position(t,CloseReason::Manual).unwrap();
        near(b.history[0].swap,rate*0.07);near(b.history[0].profit,b.balance-1000.0);near(b.history[0].canonical_net().unwrap(),b.balance-1000.0);}
}
#[test]fn partial_007_002_003_residual_with_rollover_after_first_partial(){
    let mut b=sim();b.swap_enabled=true;b.swap_long_points=-10.0;b.swap_rollover_mult=1.0;b.mark(q(T,4000.0));
    let t=b.open_market(req(0.07)).unwrap();b.mark(q(T+DAY,4001.0));b.close_partial(t,0.02,CloseReason::Partial).unwrap();
    near(b.history[0].swap,-0.2);near(b.history[0].commission,-0.14);
    b.mark(q(T+2*DAY,4001.0));b.close_partial(t,0.03,CloseReason::Partial).unwrap();b.close_position(t,CloseReason::Manual).unwrap();
    assert_eq!(b.history.len(),3);near(b.history.iter().map(|t|t.commission).sum(),-0.49);
    near(b.history.iter().map(|t|t.swap).sum(),-1.2);near(b.history.iter().map(|t|t.profit).sum(),b.balance-1000.0);
    let ids:std::collections::HashSet<_>=b.history.iter().map(|t|t.cost_receipt.as_ref().unwrap().key.deal_id).collect();assert_eq!(ids.len(),3);
}
#[test]fn pending_fill_entry_commission_is_in_closed_receipt(){
    let mut b=sim();b.place_pending(PendingReq{kind:PendingKind::BuyLimit,price:3999.0,volume:0.07,sl:None,tp:None,basket:None,level:0,is_toucher:false,is_topup:false,comment:"p".into()}).unwrap();
    b.on_quote(q(T+1,3998.8));assert_eq!(b.positions().len(),1);let t=b.positions()[0].ticket;
    b.mark(q(T+2,4001.0));b.close_position(t,CloseReason::Manual).unwrap();near(b.history[0].commission,-0.49);near(b.history[0].profit,b.balance-1000.0);
}
#[test]fn off_retains_old_swap_and_commission_contract(){
    let mut c=config();c.closed_profit_net_costs=false;let mut b=SimBroker::z_ustawien(1000.0,&c);
    b.mark(q(T,4000.0));b.swap_enabled=true;b.swap_long_points=-10.0;b.swap_rollover_mult=1.0;b.mark(q(T,4000.0));
    let t=b.open_market(req(0.07)).unwrap();b.mark(q(T+DAY,4001.0));b.close_partial(t,0.02,CloseReason::Partial).unwrap();b.close_position(t,CloseReason::Manual).unwrap();
    near(b.history[0].swap,0.0);near(b.history[1].swap,-0.7);near(b.history.iter().map(|t|t.profit).sum::<f64>()-0.49,b.balance-1000.0);
    for tr in &b.history{assert!(tr.profit_basis.is_none());assert!(!serde_json::to_string(tr).unwrap().contains("cost_receipt"));}
}
#[test]fn mode_switch_existing_state_latches_and_does_not_block_protective_close(){
    let mut b=sim();let t=b.open_market(req(0.07)).unwrap();let mut c=config();c.closed_profit_net_costs=false;b.ustaw(&c);
    assert!(b.cost_reconciliation_required().is_some());assert!(b.open_market(req(0.01)).is_err());
    assert!(b.close_position(t,CloseReason::Manual).is_ok());assert!(b.cost_reconciliation_required().is_some());
}
#[test]fn invalid_commission_or_offgrid_volume_cannot_open_on(){
    let mut b=sim();b.commission_per_lot=f64::NAN;assert!(b.open_market(req(0.07)).is_err());near(b.balance,1000.0);
    assert!(b.cost_reconciliation_required().is_some());
    let mut b=sim();assert!(b.open_market(req(0.015)).is_err());assert!(b.positions().is_empty());assert!(b.cost_reconciliation_required().is_some());
}
#[test]fn journal_uses_canonical_net_once_and_legacy_formula_is_retained(){
    let tr=legacy().with_cost_receipt(receipt()).unwrap();
    let d=conduit_core::journal::CloseDetail::new(&tr,Default::default()).unwrap();near(d.net,8.94);near(d.gross,10.0);assert!(d.cost_receipt.is_some());
    let mut old=legacy();old.swap=-0.7;old.commission=0.2;let d=conduit_core::journal::CloseDetail::new(&old,Default::default()).unwrap();near(d.net,9.1);
    assert!(!serde_json::to_string(&d).unwrap().contains("profit_basis"));
}
fn message(ts:i64,id:i64,reply:Option<i64>,text:&str)->IncomingMessage{IncomingMessage{ts,source:SourceKey::new(1,None),source_name:"TEST".into(),msg_id:id,reply_to:reply,edit_of:None,text:text.into()}}

struct InjectedClose {inner:SimBroker,injected:Vec<ClosedTrade>}
impl Broker for InjectedClose {
    fn quote(&self)->Quote{self.inner.quote()}
    fn account(&self)->Account{self.inner.account()}
    fn stops_level(&self)->f64{self.inner.stops_level()}
    fn positions(&self)->&[Position]{self.inner.positions()}
    fn pendings(&self)->&[PendingOrder]{self.inner.pendings()}
    fn positions_mut(&mut self)->&mut Vec<Position>{self.inner.positions_mut()}
    fn pendings_mut(&mut self)->&mut Vec<PendingOrder>{self.inner.pendings_mut()}
    fn cost_net_supported(&self)->bool{self.inner.cost_net_supported()}
    fn report_cost_consumer_fault(&mut self,r:&str){self.inner.report_cost_consumer_fault(r)}
    fn open_market(&mut self,r:OrderReq)->BResult<Ticket>{self.inner.open_market(r)}
    fn place_pending(&mut self,r:PendingReq)->BResult<Ticket>{self.inner.place_pending(r)}
    fn modify_position(&mut self,t:Ticket,s:Option<Px>,p:Option<Px>)->BResult<()>{self.inner.modify_position(t,s,p)}
    fn modify_pending(&mut self,t:Ticket,p:Px,s:Option<Px>,tp:Option<Px>)->BResult<()>{self.inner.modify_pending(t,p,s,tp)}
    fn close_position(&mut self,t:Ticket,r:CloseReason)->BResult<f64>{self.inner.close_position(t,r)}
    fn close_partial(&mut self,t:Ticket,v:f64,r:CloseReason)->BResult<f64>{self.inner.close_partial(t,v,r)}
    fn cancel_pending(&mut self,t:Ticket)->BResult<()>{self.inner.cancel_pending(t)}
    fn drain_closed(&mut self)->Vec<ClosedTrade>{let mut c=std::mem::take(&mut self.injected);c.extend(self.inner.drain_closed());c}
}
#[test]fn actual_engine_quarantines_invalid_receipt_and_resume_cannot_reopen_risk(){
    let mut b=InjectedClose{inner:sim(),injected:vec![legacy()]};let t=b.open_market(req(0.07)).unwrap();
    let mut e=Engine::new(config(),1000.0);e.on_tick(&mut b,&q(T,4000.0));
    assert_eq!(e.cost_quarantine.len(),1);assert_eq!(e.stats.trades,0);near(e.stats.realized_today,0.0);
    assert!(e.cost_reconciliation_required.is_some());assert!(!b.cost_net_supported());
    e.halted=None;e.risk_override=true;assert!(!matches!(e.entry_gate(&b,T+1),conduit_core::engine::Gate::Open));
    assert!(b.open_market(req(0.01)).is_err());assert!(b.close_position(t,CloseReason::Manual).is_ok());
    assert!(b.positions().is_empty());e.on_tick(&mut b,&q(T+2,4000.0));assert_eq!(e.stats.trades,1);
    near(e.stats.realized_today,b.inner.history[0].profit);assert_eq!(e.cost_quarantine.len(),1);
}
#[test]fn actual_engine_rf_books_net_once_not_cash_rpc_twice(){
    let mut c=config();c.risk_free_mode=RiskFreeMode::CloseEverything;let mut e=Engine::new(c.clone(),1000.0);let mut b=SimBroker::z_ustawien(1000.0,&c);
    b.mark(q(T,4010.0));e.on_message(&mut b,&message(T,1,None,"BUY LIMITS GOLD @ 4000/3995\nTP 4030\nTP 4040\nSL 3980"));assert_eq!(e.baskets.len(),1);
    for t in b.pendings().iter().map(|p|p.ticket).collect::<Vec<_>>(){b.cancel_pending(t).unwrap();}
    let id=e.baskets[0].id;b.mark(q(T+1,4000.0));let mut r=req(0.07);r.basket=Some(id);b.open_market(r).unwrap();e.on_tick(&mut b,&q(T+1,4000.0));
    b.mark(q(T+2,4005.0));e.on_message(&mut b,&message(T+2,2,Some(1),"RISK FREE"));assert!(b.positions().is_empty());near(e.baskets[0].realized,0.0);
    e.on_tick(&mut b,&q(T+2,4005.0));near(e.baskets[0].realized,b.history[0].profit);near(e.stats.realized_today,b.history[0].profit);
    near(b.balance-1000.0,b.history[0].profit);e.on_tick(&mut b,&q(T+3,4005.0));near(e.baskets[0].realized,b.history[0].profit);
}
#[test]fn unsupported_or_missing_dependency_is_fail_closed_before_entry(){
    for unsupported in [true,false]{let mut c=config();if !unsupported{c.basket_realized_broker_only=false;}
        let mut b=if unsupported{SimBroker::new(1000.0,0.2,7.0)}else{SimBroker::z_ustawien(1000.0,&c)};
        b.mark(q(T,4000.0));let mut e=Engine::new(c,1000.0);e.on_tick(&mut b,&q(T,4000.0));
        e.on_message(&mut b,&message(T,1,None,"BUY GOLD @ 4000/3995\nTP 4030\nSL 3980"));
        assert!(b.positions().is_empty()&&b.pendings().is_empty());assert!(e.cost_reconciliation_required.is_some());}
}
#[test]fn broken_dependency_during_open_position_never_books_mixed_gross_and_net(){
    let mut c=config();c.risk_free_mode=RiskFreeMode::CloseEverything;
    let mut e=Engine::new(c.clone(),1000.0);let mut b=SimBroker::z_ustawien(1000.0,&c);
    b.mark(q(T,4010.0));e.on_message(&mut b,&message(T,1,None,"BUY LIMITS GOLD @ 4000/3995\nTP 4030\nTP 4040\nSL 3980"));
    assert_eq!(e.baskets.len(),1);for t in b.pendings().iter().map(|p|p.ticket).collect::<Vec<_>>(){b.cancel_pending(t).unwrap();}
    b.mark(q(T+1,4000.0));let mut r=req(0.07);r.basket=Some(e.baskets[0].id);b.open_market(r).unwrap();e.on_tick(&mut b,&q(T+1,4000.0));
    // Simulate a malformed in-flight configuration. Exit must remain possible,
    // but invalid dependencies cannot reactivate old command-profit booking.
    e.cfg.basket_realized_broker_only=false;b.mark(q(T+2,4005.0));
    e.on_message(&mut b,&message(T+2,2,Some(1),"RISK FREE"));assert!(b.positions().is_empty());
    near(e.baskets[0].realized,0.0);e.on_tick(&mut b,&q(T+2,4005.0));
    near(e.baskets[0].realized,0.0);assert_eq!(e.stats.trades,0);assert_eq!(e.cost_quarantine.len(),1);
    assert!(e.cost_reconciliation_required.is_some());
}
#[test]fn missing_pool_quarantines_close_but_cash_and_protective_exit_still_work(){
    let mut b=SimBroker::new(1000.0,0.2,7.0);b.mark(q(T,4000.0));let t=b.open_market(req(0.07)).unwrap();
    // Deliberate corruption bypasses ustaw's mode-change guard: existing position
    // has no known allocation pool. Never make a fake zero-commission receipt.
    b.closed_profit_net_costs=true;b.mark(q(T+1,4001.0));assert!(b.close_position(t,CloseReason::Manual).is_ok());
    assert!(b.positions().is_empty());assert!(b.history.is_empty());assert!(b.drain_closed().is_empty());
    assert_eq!(b.quarantined_cost_trades().len(),1);assert!(b.cost_reconciliation_required().is_some());assert!(!b.cost_net_supported());
    assert!(b.open_market(req(0.01)).is_err());assert!(b.balance.is_finite());
}
