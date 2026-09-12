use super::*;
use crate::engine::testy_pakiet_a::{Atrapa, TS0, WEJSCIE, wiad};

fn engine() -> Engine {
    let mut c=Settings::default(); c.t100.enabled=true; c.t100.risk_pct=4.0;
    c.session_hours="0-24".into(); c.max_portfolio_risk_pct=0.0; c.profit_budget_arm_pct=0.0;
    c.risk_per_basket_pct=0.0; c.lot_max=5.0; c.ea_enabled=false;
    let mut e=Engine::new(c,400.0);e.tryb_auto_ea=true;e
}
fn broker() -> Atrapa {let mut b=Atrapa::nowa();b.maximum_volume=100.0;b}
fn plan(id:u64,q:Quote) -> EntryPlan {EntryPlan{decision_id:id,ts:q.ts,side:Side::Buy,
    expert:0,volume:0.01,entry_reference:q.ask,sl:q.ask-5.0,tp:q.ask+10.0,
    approved_budget_usd:6.0,risk_usd:5.0,score:0.8,atr:2.0,context_key:None}}

#[test]
fn source_entry_edit_and_cancel_are_context_not_execution() {
    let mut e=engine();let mut b=broker();
    let mut m=wiad(1,7,None,WEJSCIE);m.source_name="Synergy".into();
    e.on_message(&mut b,&m);
    m.edit_of=Some(7);m.text=WEJSCIE.replace("3990","3980");e.on_message(&mut b,&m);
    m.msg_id=8;m.edit_of=None;m.reply_to=Some(7);m.text="CANCEL BUY LIMITS".into();e.on_message(&mut b,&m);
    assert!(b.positions().is_empty() && b.pendings().is_empty() && e.baskets.is_empty());
    assert!(e.msg_to_basket.is_empty());assert_eq!(e.stats.messages,3);
}

#[test]
fn confirmed_market_intent_creates_real_basket_without_source_alias() {
    let mut e=engine();let mut b=broker();let q=b.quote();
    e.t100_execute(&mut b,&q,Intent::Open(plan(1,q)));
    assert_eq!(e.baskets.len(),1);assert_eq!(b.positions().len(),1);
    assert_eq!(e.baskets[0].tickets,vec![b.positions()[0].ticket]);
    assert_eq!(e.baskets[0].source_name,"T-100");assert!(e.msg_to_basket.is_empty());
    assert_eq!(e.t100.diagnostics.opened,1);
    let before=(b.positions()[0].sl,b.positions()[0].tp);
    e.on_message(&mut b,&wiad(1,7,None,"MOVE SL 3999\nTP1 HIT\nRISK FREE"));
    assert_eq!((b.positions()[0].sl,b.positions()[0].tp),before,"legacy source management must not run");
}

#[test]
fn local_invalid_geometry_and_definite_refusal_do_not_create_baskets_or_holds() {
    let mut e=engine();let mut b=broker();let q=b.quote();
    let mut bad=plan(1,q);bad.sl=q.ask+5.0;e.t100_execute(&mut b,&q,Intent::Open(bad));
    assert_eq!(e.order_submission_sequence,0);
    b.market_failures=1;e.t100_execute(&mut b,&q,Intent::Open(plan(2,q)));
    assert_eq!(e.order_submission_sequence,1);assert!(e.baskets.is_empty());
    assert!(e.t100_entry_hold_reason().is_none());assert_eq!(e.t100.diagnostics.rejected,2);
}

#[test]
fn exact_checkpoint_carries_runtime_and_reserved_ids_and_rejects_missing_state() {
    let mut e=engine();let mut b=broker();let q=b.quote();
    e.t100_execute(&mut b,&q,Intent::Open(plan(1,q)));
    let checkpoint=e.t100_checkpoint().unwrap();
    let disk=serde_json::to_vec(&checkpoint).unwrap();
    let mut restored=engine();restored.adopt_baskets(e.baskets.clone());
    restored.restore_t100_checkpoint(Some(&serde_json::from_slice(&disk).unwrap())).unwrap();
    assert_eq!(restored.t100_checkpoint(),Some(checkpoint));
    let replay=e.export_replay_bootstrap().unwrap();
    assert_eq!(Engine::from_replay_bootstrap(&replay).unwrap().export_replay_bootstrap().unwrap(),replay);
    let mut empty=engine();assert!(empty.restore_t100_checkpoint(None).is_err());
    assert!(empty.entry_gate(&b,TS0).blocked().is_some());
    empty.t100_execute(&mut b,&q,Intent::Open(plan(2,q)));assert_eq!(b.positions().len(),1);
    let mut changed=engine();changed.cfg.t100.risk_pct=3.0;
    assert!(changed.restore_t100_checkpoint(e.t100_checkpoint().as_ref()).is_err());
}

#[test]
fn unknown_risk_is_infinite_and_both_sides_use_current_equity_downside() {
    assert_eq!(downside(Side::Buy,4005.0,Some(4001.0),0.02),8.0);
    assert_eq!(downside(Side::Sell,3995.0,Some(3999.0),0.02),8.0);
    assert!(downside(Side::Buy,4000.0,None,0.01).is_infinite());
    assert!(downside(Side::Buy,4000.0,Some(f64::NAN),0.01).is_infinite());
}

struct Delayed {
    inner:Atrapa, last:Option<UnconfirmedOpen>, ticket:Option<Ticket>, released:bool, calls:u32,
    bars: Option<Vec<crate::t100::Bar>>,
    immediate_ack: bool, round_prices: bool,
}
impl Delayed {fn new()->Self{Self{inner:broker(),last:None,ticket:None,released:false,calls:0,bars:None,
    immediate_ack:false,round_prices:false}}}
impl Broker for Delayed {
    fn t100_contract_supported(&self) -> bool { true }
    fn complete_m1_bars(&self, after: Option<Ts>) -> Option<&[crate::t100::Bar]> {
        self.bars.as_ref().map(|bars| &bars[bars.partition_point(|b|after.is_some_and(|ts|b.ts<=ts))..])
    }
    fn quote(&self)->Quote{self.inner.quote()} fn account(&self)->Account{self.inner.account()}
    fn stops_level(&self)->f64{0.0} fn volume_max(&self)->f64{100.0}
    fn normalize_order_price(&self,p:f64)->f64{if self.round_prices{(p*100.0).round()/100.0}else{p}}
    fn positions(&self)->&[Position]{self.inner.positions()} fn positions_mut(&mut self)->&mut Vec<Position>{self.inner.positions_mut()}
    fn pendings(&self)->&[PendingOrder]{self.inner.pendings()} fn pendings_mut(&mut self)->&mut Vec<PendingOrder>{self.inner.pendings_mut()}
    fn close_receipts_pending(&self)->bool{self.ticket.is_some()&&!self.released}
    fn unconfirmed_open(&self)->Option<UnconfirmedOpen>{self.last.clone()}
    fn confirmed_open(&self,p:&UnconfirmedOpen)->Option<Ticket>{
        (self.released && self.last.as_ref()==Some(p)).then_some(self.ticket).flatten()
    }
    fn open_market(&mut self,r:OrderReq)->BResult<Ticket>{
        self.calls+=1;if self.immediate_ack{return self.inner.open_market(r);}
        self.last=Some(UnconfirmedOpen{session:ExecutionSession{scope:"synthetic".into(),generation:1},
            side:r.side,requested_volume:r.volume,basket:r.basket,level:r.level,is_toucher:r.is_toucher,
            submitted_quote_ts:self.quote().ts,machine_comment:format!("synthetic-{}",self.calls)});
        self.ticket=Some(self.inner.open_market(r)?);Err(BrokerError::Rejected)
    }
    fn place_pending(&mut self,r:PendingReq)->BResult<Ticket>{self.inner.place_pending(r)}
    fn modify_position(&mut self,t:Ticket,sl:Option<Px>,tp:Option<Px>)->BResult<()>{self.inner.modify_position(t,sl,tp)}
    fn modify_pending(&mut self,t:Ticket,p:Px,sl:Option<Px>,tp:Option<Px>)->BResult<()>{self.inner.modify_pending(t,p,sl,tp)}
    fn close_position(&mut self,t:Ticket,r:CloseReason)->BResult<f64>{self.inner.close_position(t,r)}
    fn close_partial(&mut self,t:Ticket,v:f64,r:CloseReason)->BResult<f64>{self.inner.close_partial(t,v,r)}
    fn cancel_pending(&mut self,t:Ticket)->BResult<()>{self.inner.cancel_pending(t)}
    fn drain_closed(&mut self)->Vec<ClosedTrade>{self.inner.drain_closed()}
}

#[test]
fn approved_budget_survives_stop_price_rounding_without_promoting_volume() {
    for (side,raw_sl,rounded_sl,farther) in [
        (Side::Buy,4300.004,4300.0,true),(Side::Buy,4299.996,4300.0,false),
        (Side::Sell,4309.996,4310.0,true),(Side::Sell,4310.004,4310.0,false),
    ] {
        let mut e=engine();let mut b=Delayed::new();b.immediate_ack=true;b.round_prices=true;
        let (bid,ask)=if side==Side::Buy{(4304.90,4305.00)}else{(4305.00,4305.10)};
        b.inner.ustaw_cene(TS0,bid,ask);let q=b.quote();
        let mut p=plan(1,q);p.side=side;p.entry_reference=q.entry(side);p.sl=raw_sl;
        p.tp=p.entry_reference+side.sign()*10.0;
        p.risk_usd=(p.entry_reference-p.sl)*side.sign()*100.0*p.volume;
        assert_eq!(p.risk_usd<5.0,farther);
        // Spare risk allowance must survive SL rounding, but a closer stop
        // must never increase the volume already limited by policy/margin.
        e.t100_execute(&mut b,&q,Intent::Open(p));
        assert_eq!(b.positions().len(),1,"valid minimum lot must survive price discretization within approved risk");
        assert_eq!(b.positions()[0].volume,0.01);assert_eq!(b.positions()[0].sl,Some(rounded_sl));
        assert_eq!(e.baskets[0].risk_initial_usd,5.0);
        let state=serde_json::to_value(&e.t100).unwrap();
        let memory=&state["trades"][b.positions()[0].ticket.to_string()]["plan"];
        assert_eq!(memory["risk_usd"],5.0);assert_eq!(memory["approved_budget_usd"],6.0);
        let mut restored=engine();restored.adopt_baskets(e.baskets.clone());
        restored.restore_t100_checkpoint(e.t100_checkpoint().as_ref()).unwrap();
        assert_eq!(restored.t100_checkpoint(),e.t100_checkpoint());
    }
}

#[test]
fn rounded_stop_cannot_lift_minimum_lot_above_actual_approved_budget() {
    for budget in [4.998,0.0,-1.0,f64::NAN,f64::INFINITY] {
        let mut e=engine();let mut b=Delayed::new();b.immediate_ack=true;b.round_prices=true;
        b.inner.ustaw_cene(TS0,4304.90,4305.0);let q=b.quote();let mut p=plan(1,q);
        p.sl=4300.004;p.risk_usd=(q.ask-p.sl)*100.0*p.volume;p.approved_budget_usd=budget;
        e.t100_execute(&mut b,&q,Intent::Open(p));
        assert_eq!(b.calls,0);assert!(b.positions().is_empty()&&e.baskets.is_empty());
        assert_eq!(e.t100.diagnostics.rejected,1);
    }
    let mut e=engine();let mut b=Delayed::new();b.immediate_ack=true;b.round_prices=true;
    b.inner.ustaw_cene(TS0,4304.90,4305.0);let q=b.quote();let mut p=plan(1,q);
    p.sl=4300.004;p.risk_usd=(q.ask-p.sl)*100.0*p.volume;p.approved_budget_usd=5.0;
    e.t100_execute(&mut b,&q,Intent::Open(p));
    assert_eq!(b.positions()[0].volume,0.01);assert_eq!(e.baskets[0].risk_initial_usd,5.0);
}

#[test]
fn rounded_stop_floors_larger_volume_when_approved_budget_requires_it() {
    let mut e=engine();let mut b=Delayed::new();b.immediate_ack=true;b.round_prices=true;
    b.inner.ustaw_cene(TS0,4304.90,4305.0);let q=b.quote();let mut p=plan(1,q);
    p.sl=4300.004;p.volume=0.02;p.risk_usd=(q.ask-p.sl)*100.0*p.volume;
    p.approved_budget_usd=9.995;assert!(p.risk_usd<p.approved_budget_usd);
    e.t100_execute(&mut b,&q,Intent::Open(p));
    assert_eq!(b.positions()[0].volume,0.01);assert_eq!(e.baskets[0].risk_initial_usd,5.0);
}

#[test]
fn missing_approved_budget_or_older_checkpoint_cannot_certify_pending_risk() {
    let mut serialized=serde_json::to_value(plan(1,broker().quote())).unwrap();
    serialized.as_object_mut().unwrap().remove("approved_budget_usd");
    assert!(serde_json::from_value::<EntryPlan>(serialized).is_err());
    let mut old=engine().t100_checkpoint().unwrap();old.revision="T-100/2".into();
    let mut e=engine();assert!(e.restore_t100_checkpoint(Some(&old)).is_err());
    assert!(e.t100_entry_hold_reason().is_some());
}

#[test]
fn delayed_ack_restart_adoption_counts_once_and_keeps_account_entry_barrier() {
    let mut e=engine();let mut b=Delayed::new();let q=b.quote();
    e.t100_execute(&mut b,&q,Intent::Open(plan(1,q)));
    assert_eq!(b.calls,1);assert!(e.baskets.is_empty());assert!(e.rearm_entry_hold_reason().is_some());
    e.t100_execute(&mut b,&q,Intent::Open(plan(2,q)));assert_eq!(b.calls,1);
    let mut restored=engine();restored.restore_t100_checkpoint(e.t100_checkpoint().as_ref()).unwrap();
    let saved_next=restored.next_basket_id;restored.t100_reconcile(&b);assert!(restored.baskets.is_empty());
    b.released=true;restored.t100_reconcile(&b);
    assert_eq!(restored.baskets.len(),1);assert_eq!(restored.t100.diagnostics.opened,1);
    assert!(restored.rearm_entry_hold_reason().is_none());assert_eq!(restored.next_basket_id,saved_next);
    restored.t100_reconcile(&b);assert_eq!(restored.baskets.len(),1);assert_eq!(b.calls,1);
}

#[test]
fn invalid_enabled_config_blocks_exposure_instead_of_legacy_fallback() {
    let mut c=Settings::default();c.t100.enabled=true;c.t100.experts=0;
    let mut e=Engine::new(c,400.0);e.tryb_auto_ea=true;let mut b=broker();let q=b.quote();
    e.on_tick(&mut b,&q);e.on_message(&mut b,&wiad(1,7,None,WEJSCIE));
    assert!(e.t100_entry_hold_reason().is_some());assert!(e.baskets.is_empty());
    assert!(b.positions().is_empty()&&b.pendings().is_empty());
}

#[test]
fn authoritative_m1_transcript_preserves_absence_empty_and_suffix_exactly() {
    use crate::recorded_broker::{Recorder,ReplayBroker};
    let bar=crate::t100::Bar{ts:60_000,open:4000.0,high:4003.0,low:3998.0,close:4001.25,max_spread:0.0,observations:0};
    for bars in [None,Some(vec![]),Some(vec![bar])] {
        let mut b=Delayed::new();b.bars=bars.clone();
        let mut recording=Recorder::new(&mut b);
        assert_eq!(recording.complete_m1_bars(None),bars.as_deref());
        assert_eq!(recording.complete_m1_bars(Some(60_000)),bars.as_ref().map(|_|&[][..]));
        let trace=recording.finish();
        let disk=serde_json::to_vec(&trace).unwrap();
        let replay=ReplayBroker::new(serde_json::from_slice(&disk).unwrap()).unwrap();
        assert_eq!(replay.complete_m1_bars(None),bars.as_deref());
        assert_eq!(replay.complete_m1_bars(Some(60_000)),bars.as_ref().map(|_|&[][..]));
        replay.finish().unwrap();
    }
}

#[test]
fn auto_and_auto_ea_do_not_fall_back_to_each_other() {
    let mut e=engine();e.tryb_auto_ea=false;let mut b=broker();let q=b.quote();
    e.on_message(&mut b,&wiad(1,7,None,WEJSCIE));e.on_tick(&mut b,&q);
    assert!(e.t100_entry_hold_reason().unwrap().contains("AUTO-EA"));
    assert!(e.baskets.is_empty()&&b.positions().is_empty()&&b.pendings().is_empty());
    assert_eq!(e.t100.diagnostics.quotes,0);assert_eq!(e.t100.diagnostics.contexts,0);
    e.tryb_auto_ea=true;assert!(e.t100_entry_hold_reason().is_none());
    e.on_tick(&mut b,&q);assert_eq!(e.t100.diagnostics.quotes,1);
}

#[test]
fn flat_configuration_apply_preserves_runtime_and_rejects_exposure_or_review() {
    let mut e=engine();let mut b=broker();let q=b.quote();e.on_tick(&mut b,&q);
    e.on_message(&mut b,&wiad(1,7,None,WEJSCIE));
    let runtime=encode(&e.t100).unwrap();let mut next=e.cfg.t100.clone();next.risk_pct=3.0;
    assert!(e.apply_t100_configuration(&next,false).is_err());
    e.apply_t100_configuration(&next,true).unwrap();assert_eq!(encode(&e.t100).unwrap(),runtime);
    e.t100_execute(&mut b,&q,Intent::Open(plan(1,q)));let old=e.cfg.t100.clone();
    next.enabled=false;assert!(e.apply_t100_configuration(&next,true).is_err());assert_eq!(e.cfg.t100,old);
    let mut clean=engine();clean.t100_hold("synthetic missing receipt");assert!(clean.apply_t100_configuration(&next,true).is_err());
}
