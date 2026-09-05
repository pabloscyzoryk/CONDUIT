//! Public Engine + real SimBroker, with a narrow broker fault/fill interleaving.
//! No terminal, clock sleeps, IPC or live orders.
use crate::sim::SimBroker;
use conduit_core::broker::*;
use conduit_core::engine::{Engine,IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T: Ts=1_700_000_000_000;
const BUY:&str="BUY GOLD @ 4005/4000\nTP 4030\nTP 4060\nSL 3990";
fn q(ts:Ts)->Quote {Quote{ts,bid:4012.0,ask:4012.2}}
fn cfg()->Settings {Settings {
    auto_limit:true,entry_units:1,lot_mode_percent:true,lot_percent:1.0,lot_max:0.0,
    order_volume_contract_v2:true,pending_relot_on_balance:true,
    pending_relot_reconcile_target:true,pending_relot_wg_planu:true,pending_relot_topup:true,
    pending_resize_s:1.0,pending_lifetime:PendingLifetime::Never,pending_drop_on_target:false,
    tp_source:TpSource::PriceOnly,tp_price_only_strict:true,swap_enabled:false,
    max_dd_pct:0.0,max_dd_usd:0.0,equity_floor_pct:0.0,..Settings::default()
}}
struct FaultBroker {
    inner:SimBroker,barrier:bool,authoritative:bool,cancels:Vec<Ticket>,
    reject_once:Option<Ticket>,ack_keeps_pending:bool,fill_during_cancel:f64,barrier_after_cancel:bool,
    pending_high_water:Vec<(Ts,f64)>,
}
impl Broker for FaultBroker {
    fn quote(&self)->Quote{self.inner.quote()}
    fn account(&self)->Account{self.inner.account()}
    fn stops_level(&self)->f64{self.inner.stops_level()}
    fn volume_min(&self)->f64{self.inner.volume_min()}
    fn volume_step(&self)->f64{self.inner.volume_step()}
    fn volume_max(&self)->f64{self.inner.volume_max()}
    fn pending_cancel_snapshot_authoritative(&self)->bool{self.authoritative}
    fn close_receipts_pending(&self)->bool{self.barrier}
    fn close_receipt_reconciliation_active(&self)->bool{true}
    fn positions(&self)->&[Position]{self.inner.positions()}
    fn pendings(&self)->&[PendingOrder]{self.inner.pendings()}
    fn positions_mut(&mut self)->&mut Vec<Position>{self.inner.positions_mut()}
    fn pendings_mut(&mut self)->&mut Vec<PendingOrder>{self.inner.pendings_mut()}
    fn open_market(&mut self,r:OrderReq)->BResult<Ticket>{if self.barrier{Err(BrokerError::Rejected)}else{self.inner.open_market(r)}}
    fn place_pending(&mut self,r:PendingReq)->BResult<Ticket>{if self.barrier{Err(BrokerError::Rejected)}else{
        let out=self.inner.place_pending(r);
        if out.is_ok(){self.pending_high_water.push((self.quote().ts,self.pendings().iter().map(|p|p.volume).sum()));}
        out
    }}
    fn modify_position(&mut self,t:Ticket,s:Option<Px>,p:Option<Px>)->BResult<()>{self.inner.modify_position(t,s,p)}
    fn modify_pending(&mut self,t:Ticket,x:Px,s:Option<Px>,p:Option<Px>)->BResult<()>{self.inner.modify_pending(t,x,s,p)}
    fn close_position(&mut self,t:Ticket,r:CloseReason)->BResult<f64>{self.inner.close_position(t,r)}
    fn close_partial(&mut self,t:Ticket,v:f64,r:CloseReason)->BResult<f64>{self.inner.close_partial(t,v,r)}
    fn drain_closed(&mut self)->Vec<ClosedTrade>{self.inner.drain_closed()}
    fn cancel_pending(&mut self,t:Ticket)->BResult<()> {
        self.cancels.push(t);
        if self.reject_once==Some(t){self.reject_once=None;return Err(BrokerError::Rejected);}
        if self.ack_keeps_pending{return Ok(());}
        if self.fill_during_cancel>0.0 {
            let p=self.inner.pendings().iter().find(|p|p.ticket==t).unwrap().clone();
            let v=self.fill_during_cancel.min(p.volume);self.fill_during_cancel=0.0;
            // Broker reports a real partial fill in the cancellation interval.
            // The fill price is incidental to these quantity/ownership tests.
            self.inner.open_market(OrderReq{side:p.kind.side(),volume:v,sl:p.sl,tp:p.tp,
                basket:p.basket,level:p.level,is_toucher:p.is_toucher,comment:p.comment.clone()})?;
        }
        self.inner.cancel_pending(t)?;
        if self.barrier_after_cancel{self.barrier=true;}
        Ok(())
    }
}
fn rig(c:Settings,balance:f64)->(Engine,FaultBroker){
    let mut b=FaultBroker{inner:SimBroker::z_ustawien(balance,&c),barrier:false,authoritative:true,
        cancels:vec![],reject_once:None,ack_keeps_pending:false,fill_during_cancel:0.0,barrier_after_cancel:false,pending_high_water:vec![]};
    let mut e=Engine::new(c,balance);tick(&mut e,&mut b,T);
    e.on_message(&mut b,&IncomingMessage{ts:T,source:SourceKey::new(-990002,None),source_name:"relot-audit".into(),
        msg_id:1,reply_to:None,edit_of:None,text:BUY.into()});
    assert_eq!(e.baskets.len(),1);assert!(!b.pendings().is_empty());(e,b)
}
fn tick(e:&mut Engine,b:&mut FaultBroker,ts:Ts){let q=q(ts);b.inner.on_quote(q);e.on_tick(b,&q);}
fn pv(b:&FaultBroker)->f64{b.pendings().iter().map(|p|p.volume).sum()}
fn near(a:f64,b:f64){assert!((a-b).abs()<1e-8,"{a} != {b}");}

#[test]
fn complete_empty_budget_cancels_with_both_volume_contract_modes(){
    for volume_v2 in [false,true]{let mut c=cfg();c.order_volume_contract_v2=volume_v2;c.risk_per_basket_pct=20.0;
        let(mut e,mut b)=rig(c,1000.0);b.inner.balance=10.0;tick(&mut e,&mut b,T+2000);
        assert!(b.pendings().is_empty());assert!(e.baskets[0].pendings.is_empty());}
}
#[test]
fn off_preserves_empty_plan_legacy_and_empty_metadata_bytes(){
    let mut c=cfg();c.pending_relot_reconcile_target=false;c.order_volume_contract_v2=false;c.risk_per_basket_pct=20.0;
    let(mut e,mut b)=rig(c,1000.0);b.inner.balance=10.0;tick(&mut e,&mut b,T+2000);
    near(pv(&b),0.1);assert!(!serde_json::to_string(&e.baskets).unwrap().contains("pending_relot_review"));
}
#[test]
fn invalid_plan_never_means_zero_target(){
    let(mut e,mut b)=rig(cfg(),1000.0);let old=serde_json::to_vec(b.pendings()).unwrap();
    e.cfg.lot_fixed=f64::NAN;b.inner.balance=10.0;tick(&mut e,&mut b,T+2000);
    assert_eq!(old,serde_json::to_vec(b.pendings()).unwrap());assert!(b.cancels.is_empty());
    assert!(e.odrzuty.contains_key("RelotReconcile::NonFinitePlanInput"));
}
#[test]
fn two_bases_plus_topup_reduce_once_and_target_does_not_shrink_each_tick(){
    let mut c=cfg();c.entry_units=3;c.entry_uklad="2,0".into();
    let(mut e,mut b)=rig(c,1000.0);assert_eq!(b.pendings().len(),2);near(pv(&b),0.2);
    b.inner.balance=1500.0;tick(&mut e,&mut b,T+2000);near(pv(&b),0.3);
    assert_eq!(b.pendings().iter().filter(|p|p.is_topup).count(),1);
    b.inner.balance=750.0;
    for ms in [4000,6000,8000,10000]{tick(&mut e,&mut b,T+ms);near(pv(&b),0.14);}
}
#[test]
fn cancel_rejection_keeps_ticket_and_retry_never_loses_accounting(){
    let mut c=cfg();c.entry_units=3;c.entry_uklad="2,0".into();c.risk_per_basket_pct=50.0;
    let(mut e,mut b)=rig(c,1000.0);let t=b.pendings()[0].ticket;b.reject_once=Some(t);
    b.inner.balance=1.0;tick(&mut e,&mut b,T+2000);
    assert!(b.pendings().iter().any(|p|p.ticket==t));assert!(e.baskets[0].pendings.contains(&t));
    tick(&mut e,&mut b,T+4000);assert!(b.pendings().is_empty());
}
#[test]
fn cancel_ack_with_order_still_present_never_sends_replacement(){
    let(mut e,mut b)=rig(cfg(),1000.0);b.ack_keeps_pending=true;b.inner.balance=500.0;
    tick(&mut e,&mut b,T+2000);near(pv(&b),0.1);
    assert_eq!(b.pendings().len(),1);assert!(e.baskets[0].pending_relot_review[0].reason.contains("CancelAckStillPending"));
}
#[test]
fn receipt_barrier_preserves_up_base_and_down_intent_survives_no_pending(){
    let mut c=cfg();c.pending_relot_topup=false;let(mut e,mut b)=rig(c,1000.0);
    b.barrier=true;b.inner.balance=2000.0;tick(&mut e,&mut b,T+2000);
    near(pv(&b),0.1);assert!(b.cancels.is_empty());
    b.inner.balance=500.0;tick(&mut e,&mut b,T+4000);assert!(b.pendings().is_empty());
    let json=serde_json::to_string(&e.baskets).unwrap();let restored:Vec<Basket>=serde_json::from_str(&json).unwrap();
    assert_eq!(restored[0].pending_relot_review.len(),1);near(restored[0].pending_relot_review[0].target_at_decision,0.05);
    b.barrier=false;b.inner.balance=9000.0;tick(&mut e,&mut b,T+6000);
    assert!(b.pendings().is_empty(),"review is not a queued stale order");
    e.baskets=restored;tick(&mut e,&mut b,T+8000);assert!(b.pendings().is_empty());
}
#[test]
fn nonauthoritative_live_style_snapshot_never_cancels_for_up(){
    let mut c=cfg();c.pending_relot_topup=false;let(mut e,mut b)=rig(c,1000.0);
    b.authoritative=false;b.inner.balance=2000.0;tick(&mut e,&mut b,T+2000);
    near(pv(&b),0.1);assert!(b.cancels.is_empty());
}
#[test]
fn partial_fill_during_cancel_is_subtracted_and_cannot_make_negative_delta(){
    for fill in [0.04,0.08]{let(mut e,mut b)=rig(cfg(),1000.0);b.fill_during_cancel=fill;b.inner.balance=500.0;
        tick(&mut e,&mut b,T+2000);near(pv(&b),(0.05_f64-fill).max(0.0));
        near(b.positions().iter().map(|p|p.volume).sum(),fill);
        assert!(b.pendings().iter().all(|p|p.volume>0.0));}
}
#[test]
fn existing_partial_fill_blocks_up_without_closing_the_fill(){
    let(mut e,mut b)=rig(cfg(),1000.0);let p=b.pendings()[0].clone();
    b.inner.open_market(OrderReq{side:Side::Buy,volume:0.04,sl:p.sl,tp:p.tp,basket:p.basket,level:p.level,
        is_toucher:false,comment:String::new()}).unwrap();b.pendings_mut()[0].volume=0.06;
    b.inner.balance=2000.0;tick(&mut e,&mut b,T+2000);
    near(pv(&b),0.06);near(b.positions()[0].volume,0.04);assert!(b.cancels.is_empty());
}
#[test]
fn active_portfolio_cap_does_not_count_own_replaced_pending_twice(){
    let mut c=cfg();c.max_portfolio_risk_pct=20.0;
    let(mut e,mut b)=rig(c,1000.0);let old=pv(&b);
    tick(&mut e,&mut b,T+2000);near(pv(&b),old);assert!(b.cancels.is_empty());
}

#[test]
fn requires_review_blocks_later_rearm_after_receipt_clears(){
    let mut c=cfg();c.rearm_grid_on_return=true;c.rearm_bez_pozycji=true;c.rearm_min_gap_min=0.0;
    let(mut e,mut b)=rig(c,1000.0);b.barrier=true;b.inner.balance=500.0;
    tick(&mut e,&mut b,T+2000);assert!(b.pendings().is_empty());
    b.barrier=false;let q=Quote{ts:T+4000,bid:4003.0,ask:4003.2};b.inner.on_quote(q);e.on_tick(&mut b,&q);
    assert!(b.pendings().is_empty()&&b.positions().is_empty());
    assert!(e.odrzuty.contains_key("RelotReconcile::RequiresReviewEntryBlocked"));
}

#[test]
fn empty_budget_rearm_cannot_restore_stale_frozen_volume(){
    let mut c=cfg();c.entry_units=3;c.risk_per_basket_pct=20.0;
    c.rearm_grid_on_return=true;c.rearm_bez_pozycji=true;c.rearm_min_gap_min=0.0;
    let(mut e,mut b)=rig(c,1000.0);b.inner.balance=10.0;tick(&mut e,&mut b,T+2000);
    assert!(b.pendings().is_empty());
    let q=Quote{ts:T+4000,bid:4003.0,ask:4003.2};b.inner.on_quote(q);e.on_tick(&mut b,&q);
    assert!(b.pendings().is_empty()&&b.positions().is_empty(),"rearm must not resurrect risk rejected by the checked plan");
}

#[test]
fn consolidated_level_sync_cannot_overshoot_aggregate_even_intratick(){
    let mut c=cfg();c.entry_units=3;c.entry_uklad="2,0".into();
    c.pending_resize_on_vol=true;c.vol_window_min=1.0;
    let(mut e,mut b)=rig(c,1000.0);b.inner.balance=300.0;tick(&mut e,&mut b,T+2000);
    near(pv(&b),0.06);assert_eq!(b.pendings().len(),1);
    b.pending_high_water.clear();tick(&mut e,&mut b,T+4000);near(pv(&b),0.06);
    assert!(b.pending_high_water.iter().all(|(_,v)|*v<=0.06+1e-10),"intracycle overexposure: {:?}",b.pending_high_water);
}

#[test]
fn checked_zero_matrix_volume_sync_flag_and_resize_or_rearm(){
    for a in [false,true]{for only_live in [false,true]{for rearm in [false,true]{
        let mut c=cfg();c.order_volume_contract_v2=a;c.sync_only_live_levels=only_live;
        c.entry_units=3;c.risk_per_basket_pct=20.0;
        c.pending_resize_on_vol=!rearm;c.vol_window_min=1.0;
        c.rearm_grid_on_return=rearm;c.rearm_bez_pozycji=true;c.rearm_min_gap_min=0.0;
        let(mut e,mut b)=rig(c,1000.0);b.pending_high_water.clear();b.inner.balance=10.0;
        tick(&mut e,&mut b,T+2000);
        let bid=if rearm{4003.0}else{4012.0};let q=Quote{ts:T+4000,bid,ask:bid+0.2};
        b.inner.on_quote(q);e.on_tick(&mut b,&q);
        assert!(b.pendings().is_empty()&&b.positions().is_empty(),"A={a},sync={only_live},rearm={rearm}");
        assert!(b.pending_high_water.is_empty());
    }}}
}

#[test]
fn checked_positive_aggregate_matrix_never_temporarily_overfills(){
    for a in [false,true]{for only_live in [false,true]{for rearm in [false,true]{
        let mut c=cfg();c.order_volume_contract_v2=a;c.sync_only_live_levels=only_live;
        c.entry_units=3;c.entry_uklad="0,2".into();
        c.pending_resize_on_vol=!rearm;c.vol_window_min=1.0;
        c.rearm_grid_on_return=rearm;c.rearm_bez_pozycji=true;c.rearm_min_gap_min=0.0;
        let(mut e,mut b)=rig(c,1000.0);b.inner.balance=300.0;tick(&mut e,&mut b,T+2000);
        near(pv(&b),0.06);assert_eq!(b.pendings().len(),1);b.pending_high_water.clear();
        let bid=if rearm{4003.0}else{4012.0};let q=Quote{ts:T+4000,bid,ask:bid+0.2};
        b.inner.on_quote(q);e.on_tick(&mut b,&q);near(pv(&b),0.06);
        assert!(b.pending_high_water.iter().all(|(_,v)|*v<=0.06+1e-10),"A={a},sync={only_live},rearm={rearm}: {:?}",b.pending_high_water);
    }}}
}
