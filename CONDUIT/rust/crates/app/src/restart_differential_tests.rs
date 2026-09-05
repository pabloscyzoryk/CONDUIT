//! Deliberate equality contracts: actual app memory/adoption helpers and the
//! same SimBroker continue across restart. No terminal, account or filesystem IO.
use super::*;
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::{Broker,BResult,OrderReq,PendingReq,ExecutionSession,ReceiptBarrier};
use conduit_core::formaty::Lancuch;
use conduit_core::engine::Gate;
use conduit_core::settings::*;
use conduit_core::types::*;
use std::collections::BTreeMap;
const T:Ts=1_700_000_000_000;
const TEXT:&str="BUY GOLD @ 4005/4000\nTP 5000\nTP 5100\nSL 3900";
const LOGIN:i64=123456;
const MAGIC:i64=770077;
fn cfg()->Settings{Settings{auto_limit:false,entry_units:1,lot_fixed:0.03,lot_max:0.0,
    risk_per_basket_pct:0.0,max_portfolio_risk_pct:0.0,pending_lifetime:PendingLifetime::Never,
    pending_drop_on_target:false,tp_source:TpSource::PriceOnly,tp_price_only_strict:true,
    swap_enabled:false,max_dd_pct:0.0,max_dd_usd:0.0,equity_floor_pct:0.0,
    basket_realized_broker_only:true,confirmed_exit_retry:true,sltp_retry_s:3.0,
    restore_strategy_continuation:true,
    ..Settings::default()}}
struct RetryBroker{inner:SimBroker,reject_next_modify:bool,unknown_id:bool,
    id_override:Option<u64>,scope:String,generation:u64,receipt:ReceiptBarrier}
impl Broker for RetryBroker {
    fn quote(&self)->Quote{self.inner.quote()}
    fn account(&self)->Account{self.inner.account()}
    fn stops_level(&self)->f64{self.inner.stops_level()}
    fn volume_min(&self)->f64{self.inner.volume_min()}
    fn volume_step(&self)->f64{self.inner.volume_step()}
    fn volume_max(&self)->f64{self.inner.volume_max()}
    fn execution_session(&self)->Option<ExecutionSession>{Some(ExecutionSession{scope:self.scope.clone(),generation:self.generation})}
    fn position_identifier(&self,t:Ticket)->Option<u64>{
        if self.unknown_id {None} else {self.inner.find_position(t).map(|p|self.id_override.unwrap_or(p.ticket))}
    }
    fn receipt_barrier(&self)->ReceiptBarrier{self.receipt}
    fn positions(&self)->&[Position]{self.inner.positions()}
    fn pendings(&self)->&[PendingOrder]{self.inner.pendings()}
    fn positions_mut(&mut self)->&mut Vec<Position>{self.inner.positions_mut()}
    fn pendings_mut(&mut self)->&mut Vec<PendingOrder>{self.inner.pendings_mut()}
    fn open_market(&mut self,r:OrderReq)->BResult<Ticket>{self.inner.open_market(r)}
    fn place_pending(&mut self,r:PendingReq)->BResult<Ticket>{self.inner.place_pending(r)}
    fn cancel_pending(&mut self,t:Ticket)->BResult<()>{self.inner.cancel_pending(t)}
    fn modify_position(&mut self,t:Ticket,s:Option<Px>,p:Option<Px>)->BResult<()>{
        if self.reject_next_modify{self.reject_next_modify=false;return Err(conduit_core::BrokerError::Rejected);}
        self.inner.modify_position(t,s,p)
    }
    fn modify_pending(&mut self,t:Ticket,x:Px,s:Option<Px>,p:Option<Px>)->BResult<()>{self.inner.modify_pending(t,x,s,p)}
    fn close_position(&mut self,t:Ticket,r:CloseReason)->BResult<f64>{self.inner.close_position(t,r)}
    fn close_partial(&mut self,t:Ticket,v:f64,r:CloseReason)->BResult<f64>{self.inner.close_partial(t,v,r)}
    fn drain_closed(&mut self)->Vec<ClosedTrade>{self.inner.drain_closed()}
}
fn team(c:&Settings,balance:f64)->routing::Silniki {
    let mut chain=Lancuch{nazwa:"restart-proof".into(),..Default::default()};
    chain.presety.insert("Synergy".into(),"RESTART-PROOF".into());
    let presets=BTreeMap::from([("RESTART-PROOF".into(),c.clone())]);
    let(s,missing)=routing::Silniki::zbuduj(&chain,&presets,c,balance);
    assert!(missing.is_empty());assert_eq!(s.lista.len(),1);s
}
fn msg(ts:Ts,id:i64,text:&str,reply:Option<i64>)->IncomingMessage{IncomingMessage{ts,
    source:SourceKey::new(-990004,None),source_name:"restart-differential".into(),
    msg_id:id,reply_to:reply,edit_of:None,text:text.into()}}
fn tick(s:&mut routing::Silniki,b:&mut RetryBroker,ts:Ts,bid:f64){
    let q=Quote{ts,bid,ask:bid+0.2};b.inner.on_quote(q);s.glowny_mut().engine.on_tick(b,&q);
}
fn rig(c:Settings)->(routing::Silniki,RetryBroker){
    let mut b=RetryBroker{inner:SimBroker::z_ustawien(1000.0,&c),reject_next_modify:false,unknown_id:false,
        id_override:None,scope:"synthetic-restart-account".into(),generation:1,receipt:ReceiptBarrier::Clear};
    let mut s=team(&c,1000.0);tick(&mut s,&mut b,T,4004.0);
    let reports=restore_strategy_memory(&mut s,&Trwale::default(),&b,ContinuationOrigin::Fresh);
    assert!(reports.iter().all(|(_,r)|r.review.is_none()));
    s.glowny_mut().engine.on_message(&mut b,&msg(T,1,TEXT,None));
    tick(&mut s,&mut b,T+1,4004.0);assert_eq!(b.positions().len(),1);(s,b)
}

/// Actual app projection, not a manually copied list of Engine fields. Memory
/// and basket JSON are round-tripped separately, like the two current files;
/// this fixture deliberately makes NO atomicity/exactly-once claim.
fn production_restart(s:&mut routing::Silniki,b:&RetryBroker)->routing::Silniki {
    let c=s.glowny().engine.cfg.clone();let acc=b.account();
    let mut memory=Trwale::default();zapamietaj_silniki(&mut memory,s,acc.equity,"");
    memory.silniki=serde_json::from_slice(&serde_json::to_vec(&memory.silniki).unwrap()).unwrap();
    let (new,reports)=restore_memory(c,memory,b,ContinuationOrigin::Memory);
    assert!(reports.iter().all(|(_,r)|r.review.is_none()),"{reports:?}");new
}
fn restore_memory(c:Settings,mut memory:Trwale,b:&RetryBroker,origin:ContinuationOrigin)
    ->(routing::Silniki,Vec<(String,ContinuationImportReport)>) {
    let acc=b.account();
    let dump=wznowienie::Zrzut{wersja:wznowienie::WERSJA,zapisano:b.quote().ts,login:LOGIN,
        magic:MAGIC,symbol:"XAUUSD".into(),next_basket_id:memory.next_basket_id,koszyki:memory.koszyki.clone()};
    let dump=serde_json::from_slice(&serde_json::to_vec(&dump).unwrap()).unwrap();
    let mut new=team(&c,acc.balance);przenies_pamiec(&mut new,&mut memory,acc.balance,acc.credit);
    let recovered=wznowienie::odtworz(Some(dump),b.positions(),b.pendings(),LOGIN,MAGIC,"XAUUSD","CD");
    new.rozdaj_koszyki(recovered.koszyki);
    let reports=restore_strategy_memory(&mut new,&memory,b,origin);
    (new,reports)
}

fn rejected_be()->(routing::Silniki,RetryBroker){
    let(mut s,mut b)=rig(cfg());tick(&mut s,&mut b,T+1000,4008.0);b.reject_next_modify=true;
    s.glowny_mut().engine.on_message(&mut b,&msg(T+1000,2,"BREAK EVEN",Some(1)));
    assert!(!b.reject_next_modify);assert_eq!(b.positions()[0].sl,Some(3900.0));(s,b)
}
fn memory_of(s:&mut routing::Silniki,b:&RetryBroker)->Trwale {
    let mut m=Trwale::default();zapamietaj_silniki(&mut m,s,b.account().equity,"");
    m.silniki=serde_json::from_slice(&serde_json::to_vec(&m.silniki).unwrap()).unwrap();m
}

fn retry_stop_path(restart:bool)->Option<f64>{
    let(mut s,mut b)=rig(cfg());tick(&mut s,&mut b,T+1000,4008.0);
    b.reject_next_modify=true;
    s.glowny_mut().engine.on_message(&mut b,&msg(T+1000,2,"BREAK EVEN",Some(1)));
    assert!(!b.reject_next_modify,"fixture must actually reject the requested BE modification");
    assert_eq!(b.positions()[0].sl,Some(3900.0));
    if restart{s=production_restart(&mut s,&b);}
    tick(&mut s,&mut b,T+5000,4008.0);b.positions()[0].sl
}
#[test]
fn restart_must_preserve_pending_protective_stop_retry(){
    let uninterrupted=retry_stop_path(false);let restarted=retry_stop_path(true);
    assert_eq!(restarted,uninterrupted,"same broker and ticks, actual app memory/adopt must not lose pending protection");
}

fn queued_exit_path(restart:bool)->(usize,f64){
    let mut c=cfg();c.basket_target_usd=1.0;c.exit_via_limit=true;c.exit_limit_wait_s=5.0;
    let(mut s,mut b)=rig(c);tick(&mut s,&mut b,T+1000,4005.0);
    assert_eq!(b.positions().len(),1,"exit should be queued at ask, not yet closed");
    if restart{s=production_restart(&mut s,&b);}
    tick(&mut s,&mut b,T+7000,4004.5);(b.positions().len(),b.account().balance)
}
#[test]
fn restart_must_preserve_discretionary_exit_deadline(){
    // General axis contract; GOD-X4-LIVE currently has exit_via_limit=false.
    let uninterrupted=queued_exit_path(false);let restarted=queued_exit_path(true);
    assert_eq!(restarted,uninterrupted,"restart cannot erase an already committed exit deadline");
}

fn day_stop_path(restart:bool)->usize{
    let mut c=cfg();c.day_trail_stop_pct=30.0;c.day_trail_arm_pct=12.0;
    let(mut s,mut b)=rig(c);tick(&mut s,&mut b,T+1000,4105.0);
    tick(&mut s,&mut b,T+2000,3970.0);tick(&mut s,&mut b,T+2100,3970.0);
    assert!(b.positions().is_empty(),"day trail must actually close the basket");
    // Explicit cash deposit removes the instantaneous drawdown condition. It
    // does NOT remove the already latched stop-until-midnight policy.
    b.inner.balance+=400.0;
    if restart{s=production_restart(&mut s,&b);}
    tick(&mut s,&mut b,T+5000,4004.0);
    s.glowny_mut().engine.on_message(&mut b,&msg(T+5000,3,TEXT,None));
    b.positions().len()
}
#[test]
fn restart_must_preserve_latched_day_stop_after_funds_change(){
    let uninterrupted=day_stop_path(false);let restarted=day_stop_path(true);
    assert_eq!(uninterrupted,0,"control must keep the day closed");
    assert_eq!(restarted,uninterrupted,"account-memory stats are not the private day_stop latch");
}

#[test]
fn day_stop_survives_account_memory_without_optional_order_continuation() {
    let mut c = cfg();
    c.restore_strategy_continuation = false;
    c.day_trail_stop_pct = 1.0;
    c.day_trail_arm_pct = 0.0;
    let (mut s, mut b) = rig(c.clone());
    tick(&mut s, &mut b, T + 1000, 4020.0);
    tick(&mut s, &mut b, T + 2000, 4004.0);
    assert!(b.positions().is_empty());
    let stopped = s.glowny().engine.stopped_trading_day();
    assert!(stopped.is_some());
    b.inner.balance += 400.0;
    let memory = memory_of(&mut s, &b);
    let (mut restored, reports) = restore_memory(c, memory, &b, ContinuationOrigin::Memory);
    assert!(reports.is_empty(), "the order-continuation axis remains OFF");
    assert_eq!(restored.glowny().engine.stopped_trading_day(), stopped);
    tick(&mut restored, &mut b, T + 3000, 4004.0);
    restored.glowny_mut().engine.on_message(&mut b, &msg(T + 3000, 4, TEXT, None));
    assert!(b.positions().is_empty(), "cash deposit/reconnect erased the day latch");
    let next_day = T + 86_400_000;
    tick(&mut restored, &mut b, next_day, 4004.0);
    restored.glowny_mut().engine.on_message(&mut b, &msg(next_day, 5, TEXT, None));
    assert!(!b.positions().is_empty(), "yesterday's latch blocked the new trading day");
}

#[test]
fn live_risk_signature_tracks_source_alias_withdrawal_day_stop_and_engine_replacement() {
    let mut c = cfg(); c.explicit_pending_until_cancel = true; c.reply_graph_transitive = true;
    let (mut s, mut b) = rig(c);
    let original = sygnatura_ryzyka(&s);
    s.glowny_mut().engine.on_message(&mut b,
        &msg(T+10,10,"BUY LIMITS GOLD @ 3990/3985\nTP 4050\nSL 3970",None));
    let accepted = sygnatura_ryzyka(&s); assert_ne!(accepted, original);
    s.glowny_mut().engine.on_message(&mut b, &msg(T+20,11,"TP1 HIT",Some(10)));
    let aliased = sygnatura_ryzyka(&s); assert_ne!(aliased, accepted);
    assert_eq!(aliased, sygnatura_ryzyka(&s), "unchanged loop must not dirty the source ledger");
    s.glowny_mut().engine.on_message(&mut b, &msg(T+30,12,"CANCEL",Some(11)));
    let withdrawn = sygnatura_ryzyka(&s); assert_ne!(withdrawn, aliased);
    s.glowny_mut().engine.restore_stopped_trading_day(Some(123));
    let stopped = sygnatura_ryzyka(&s); assert_ne!(stopped, withdrawn);
    s.glowny_mut().engine.risk_override = true;
    let overridden = sygnatura_ryzyka(&s); assert_ne!(overridden, stopped);
    s.glowny_mut().engine.halted = Some("synthetic risk stop".into());
    let halted = sygnatura_ryzyka(&s); assert_ne!(halted, overridden);
    let restored = production_restart(&mut s, &b);
    assert_ne!(sygnatura_ryzyka(&restored), halted,
        "new engine instance must force complete durable snapshot even with equal source count");
    assert!(restored.glowny().engine.export_pending_source_memory().iter()
        .any(|r|r.msg_id==10 && r.aliases.contains(&11) && r.cancelled_ts.is_some()));
}

#[test]
fn publisher_pending_validity_and_withdrawal_survive_actual_app_restart_projection() {
    let mut c = cfg(); c.explicit_pending_until_cancel = true;
    c.restore_strategy_continuation = false; c.reply_graph_transitive = true;
    let (mut s, mut b) = rig(c);
    let pending = "BUY LIMITS GOLD @ 3990/3985\nTP 4050\nTP 4060\nSL 3970";
    s.glowny_mut().engine.on_message(&mut b, &msg(T+10,10,pending,None));
    assert!(!b.pendings().is_empty());
    s.glowny_mut().engine.on_message(&mut b, &msg(T+20,11,"TP1 HIT",Some(10)));
    let mut restored = production_restart(&mut s, &b);
    tick(&mut restored, &mut b, T + 14*86_400_000, 4004.0);
    assert!(!b.pendings().is_empty(), "source limits survive two weeks and restart");
    let before_positions: Vec<_> = b.positions().iter().map(|p|p.ticket).collect();
    restored.glowny_mut().engine.on_message(&mut b,
        &msg(T+14*86_400_000+1,12,"NO LONGER VALID",Some(11)));
    assert!(b.pendings().is_empty(), "persisted alias must resolve to the old source setup");
    assert_eq!(before_positions,b.positions().iter().map(|p|p.ticket).collect::<Vec<_>>());
    let ledger = restored.glowny().engine.export_pending_source_memory();
    assert!(ledger.iter().any(|r|r.msg_id==10 && r.cancelled_ts.is_some()));
    let mut after_cancel = production_restart(&mut restored, &b);
    assert!(after_cancel.glowny().engine.export_pending_source_memory()
        .iter().any(|r|r.msg_id==10 && r.cancelled_ts.is_some()));
    after_cancel.glowny_mut().engine.on_message(&mut b,
        &msg(T+28*86_400_000,10,pending,None));
    assert!(b.pendings().is_empty(), "no-exposure recovery must preserve the source tombstone");
    assert_eq!(before_positions,b.positions().iter().map(|p|p.ticket).collect::<Vec<_>>());
}

#[test]
fn continuation_off_keeps_legacy_projection_without_new_key(){
    let mut c=cfg();c.restore_strategy_continuation=false;
    let(mut s,mut b)=rig(c);tick(&mut s,&mut b,T+1000,4008.0);b.reject_next_modify=true;
    s.glowny_mut().engine.on_message(&mut b,&msg(T+1000,2,"BREAK EVEN",Some(1)));
    let m=memory_of(&mut s,&b);let json=serde_json::to_value(&m.silniki).unwrap();
    assert!(json.as_object().unwrap().values().all(|v|v.get("continuation").is_none()));
    let c=s.glowny().engine.cfg.clone();let(mut new,reports)=restore_memory(c,m,&b,ContinuationOrigin::Memory);
    assert!(reports.is_empty());tick(&mut new,&mut b,T+5000,4008.0);
    assert_eq!(b.positions()[0].sl,Some(3900.0),"OFF retains the documented old loss of retry");
}

#[test]
fn confirmed_tighter_broker_sl_is_not_loosened_on_import_or_retry(){
    let(mut s,mut b)=rejected_be();let p=b.positions()[0].clone();
    b.inner.modify_position(p.ticket,Some(4006.0),p.tp).unwrap();
    s=production_restart(&mut s,&b);tick(&mut s,&mut b,T+5000,4008.0);
    assert_eq!(b.positions()[0].sl,Some(4006.0));
}

#[test]
fn new_alias_for_same_stable_position_and_new_transport_generation_is_supported(){
    let(mut s,mut b)=rejected_be();let stable=b.positions()[0].ticket;
    let c=s.glowny().engine.cfg.clone();let m=memory_of(&mut s,&b);
    b.inner.positions_mut()[0].ticket=999;b.id_override=Some(stable);b.generation+=1;
    let(mut new,reports)=restore_memory(c,m,&b,ContinuationOrigin::Memory);
    assert!(reports.iter().all(|(_,r)|r.review.is_none()),"{reports:?}");
    tick(&mut new,&mut b,T+5000,4008.0);assert_eq!(b.positions()[0].sl,Some(4004.2));
}

#[test]
fn same_ticket_different_stable_position_or_missing_identity_requires_review(){
    for unknown in [false,true] {
        let(mut s,mut b)=rejected_be();let c=s.glowny().engine.cfg.clone();let m=memory_of(&mut s,&b);
        if unknown {b.unknown_id=true;} else {b.id_override=Some(991122);}
        let(mut new,reports)=restore_memory(c,m,&b,ContinuationOrigin::Memory);
        assert!(reports[0].1.review.is_some());
        let before=new.glowny().engine.export_strategy_continuation().unwrap();
        tick(&mut new,&mut b,T+5000,4008.0);
        assert_eq!(b.positions()[0].sl,Some(3900.0),"unknown={unknown}; reports={reports:?}; imported desired={:?}; logs={:?}",before.desired,new.glowny().engine.logs);
        new.glowny_mut().engine.resume_trading(T+5001);
        assert!(!matches!(new.glowny().engine.entry_gate(&b,T+5001),Gate::Open));
    }
}

#[test]
fn changed_settings_and_changed_setup_do_not_borrow_old_intent_authority(){
    for settings_change in [false,true] {
        let(mut s,mut b)=rejected_be();let mut c=s.glowny().engine.cfg.clone();let mut m=memory_of(&mut s,&b);
        if settings_change {c.be_lock_pts+=1.0;} else {m.koszyki[0].entry_hi+=1.0;}
        let(mut new,reports)=restore_memory(c,m,&b,ContinuationOrigin::Memory);
        assert!(reports[0].1.review.is_some());
        assert_eq!(reports[0].1.imported_stops,0);
        assert_eq!(reports[0].1.imported_exits,0);
        let before=new.glowny().engine.export_strategy_continuation().unwrap();
        assert!(before.desired.is_empty());
        tick(&mut new,&mut b,T+5000,4008.0);
        // The changed configuration activates the independent current BE-lock
        // rule. Review blocks new risk, not valid protective RPCs. That SL is
        // not evidence that a rejected old continuation was imported.
        let expected_sl=if settings_change {4004.2} else {3900.0};
        assert_eq!(b.positions()[0].sl,Some(expected_sl),"settings_change={settings_change}; reports={reports:?}");
    }
}

#[test]
fn failed_replacement_cannot_reuse_previous_intent_guard_when_identifier_returns(){
    let(mut s,mut b)=rejected_be();b.unknown_id=true;b.reject_next_modify=true;
    s.glowny_mut().engine.on_message(&mut b,&msg(T+1100,3,"MOVE SL TO 4006",Some(1)));
    assert!(!b.reject_next_modify,"fixture must request a second actual modify");
    let snap=s.glowny().engine.export_strategy_continuation().unwrap();
    assert_eq!(snap.desired.len(),1);assert!(snap.desired[0].guard.is_none());
    b.unknown_id=false;tick(&mut s,&mut b,T+5000,4008.0);
    assert_eq!(b.positions()[0].sl,Some(3900.0));assert!(s.glowny().engine.continuation_review().is_some());
}

#[test]
fn cold_disk_review_still_executes_verified_protective_retry_and_broker_stop(){
    let(mut s,mut b)=rejected_be();let c=s.glowny().engine.cfg.clone();let m=memory_of(&mut s,&b);
    let(mut new,reports)=restore_memory(c,m,&b,ContinuationOrigin::UnverifiedDisk);
    assert_eq!(reports[0].1.review.as_ref().unwrap().scope,ContinuationReviewScope::Account);
    tick(&mut new,&mut b,T+5000,4008.0);assert_eq!(b.positions()[0].sl,Some(4004.2));
    new.glowny_mut().engine.resume_trading(T+5001);
    assert!(!matches!(new.glowny().engine.entry_gate(&b,T+5001),Gate::Open));
    tick(&mut new,&mut b,T+6000,4003.0);assert!(b.positions().is_empty());
}

#[test]
fn missing_restore_and_scope_mismatch_are_not_fresh_start(){
    for wrong_scope in [false,true] {
        let(mut s,mut b)=rejected_be();let c=s.glowny().engine.cfg.clone();let mut m=memory_of(&mut s,&b);
        if wrong_scope {b.scope="other-account".into();} else {m.silniki.values_mut().next().unwrap().continuation=None;}
        let(mut new,reports)=restore_memory(c,m,&b,ContinuationOrigin::Memory);
        assert_eq!(reports[0].1.review.as_ref().unwrap().scope,ContinuationReviewScope::Account);
        new.glowny_mut().engine.resume_trading(T+5000);
        assert!(!matches!(new.glowny().engine.entry_gate(&b,T+5000),Gate::Open));
    }
}

#[test]
fn review_does_not_erase_risk_halt_through_projection_and_resume(){
    let(mut s,mut b)=rejected_be();
    s.glowny_mut().engine.halted=Some("max DD: synthetic verified risk latch".into());
    s.glowny_mut().engine.hold_strategy_continuation(ContinuationReviewScope::Engine,"synthetic uncertain intent");
    let c=s.glowny().engine.cfg.clone();let m=memory_of(&mut s,&b);
    let(mut restored,reports)=restore_memory(c,m,&b,ContinuationOrigin::Memory);
    assert!(reports[0].1.review.is_some());
    let before=rozbij_klasy_zatrzymania(&restored,"").1;
    assert!(before.contains("max DD")&&before.contains("CONTINUATION REVIEW"));
    let peaks=(restored.glowny().engine.stats.peak_equity,restored.glowny().engine.stats.day_peak_equity);
    restored.glowny_mut().engine.resume_trading(T+5000);
    assert!(!restored.glowny().engine.risk_override);
    assert_eq!(rozbij_klasy_zatrzymania(&restored,"").1,before);
    assert_eq!((restored.glowny().engine.stats.peak_equity,restored.glowny().engine.stats.day_peak_equity),peaks);
    let dir=std::env::temp_dir().join(format!("conduit-continuation-resume-{}-{}",std::process::id(),conduit_server::now_ms()));
    let st=conduit_server::bootstrap(&conduit_server::ServerConfig{workspace:dir.clone(),..Default::default()},conduit_server::default_auth()).unwrap();
    assert!(wznow_handel(&st,&mut restored,&mut String::new(),T+5001).is_err());
    assert!(!st.read(|s|s.risk_override.active));
    assert_eq!(rozbij_klasy_zatrzymania(&restored,"").1,before);
    // A verified Engine close is still permitted, not merely a broker SL.
    restored.glowny_mut().engine.close_everything(&mut b,T+5002,CloseReason::Manual);
    assert!(b.positions().is_empty());assert!(!b.inner.history.is_empty());
    drop(st);std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn engine_owned_review_is_local_but_account_review_reaches_other_leg(){
    let c=cfg();let(_,mut b)=rig(c.clone());b.inner=SimBroker::z_ustawien(1000.0,&c);
    b.inner.q=Quote{ts:T,bid:4004.0,ask:4004.2};
    let mut chain=Lancuch{nazwa:"scope-proof".into(),..Default::default()};
    chain.presety.insert("Synergy".into(),"SYN".into());chain.presety.insert("ATFX".into(),"ATF".into());
    let presets=BTreeMap::from([("SYN".into(),c.clone()),("ATF".into(),c.clone())]);
    let(mut s,missing)=routing::Silniki::zbuduj(&chain,&presets,&c,1000.0);assert!(missing.is_empty());assert_eq!(s.lista.len(),2);
    for i in 0..s.lista.len(){let owner=s.lista[i].format.clone();
        let r=s.z_widokiem(i,&mut b,|e,w|e.import_strategy_continuation(w,&owner,None,ContinuationOrigin::Fresh));assert!(r.review.is_none());}
    s.z_widokiem(0,&mut b,|e,_|e.hold_strategy_continuation(ContinuationReviewScope::Engine,"local intent"));
    assert!(s.lista[0].engine.continuation_entry_blocked());assert!(!s.lista[1].engine.continuation_entry_blocked());
    assert!(s.z_widokiem(1,&mut b,|e,w|matches!(e.entry_gate(w,T),Gate::Open)));
    s.z_widokiem(0,&mut b,|e,_|e.hold_strategy_continuation(ContinuationReviewScope::Account,"unknown account generation"));
    assert!(s.lista.iter().all(|x|x.engine.continuation_entry_blocked()));
}

#[test]
fn settings_mismatch_retains_historical_day_stop(){
    let mut c=cfg();c.day_trail_stop_pct=30.0;c.day_trail_arm_pct=12.0;
    let(mut s,mut b)=rig(c.clone());tick(&mut s,&mut b,T+1000,4105.0);tick(&mut s,&mut b,T+2000,3970.0);
    tick(&mut s,&mut b,T+2100,3970.0);
    let old=s.glowny().engine.export_strategy_continuation().unwrap().day_stop;
    assert_ne!(old,i64::MIN);let m=memory_of(&mut s,&b);c.be_lock_pts+=1.0;
    let(new,reports)=restore_memory(c,m,&b,ContinuationOrigin::Memory);
    assert!(reports[0].1.review.is_some());assert_eq!(new.glowny().engine.export_strategy_continuation().unwrap().day_stop,old);
}

#[test]
fn actual_runtime_settings_reload_requires_review_without_replacing_old_proof(){
    for enable_late in [false,true] {
        let mut c=cfg();c.restore_strategy_continuation=!enable_late;
        let(mut s,b)=rig(c.clone());s.lista[0].z_pliku=false;
        let saved=s.glowny().engine.export_strategy_continuation();
        let dir=std::env::temp_dir().join(format!("conduit-continuation-reload-{}-{}-{enable_late}",std::process::id(),conduit_server::now_ms()));
        let st=conduit_server::bootstrap(&conduit_server::ServerConfig{workspace:dir.clone(),..Default::default()},conduit_server::default_auth()).unwrap();
        let mut proposed=c.clone();proposed.restore_strategy_continuation=true;proposed.be_lock_pts+=1.0;
        st.update(Sections::all(),|state|state.settings=serde_json::to_value(&proposed).unwrap());
        let stops=c.stops_level;
        przeladuj_ustawienia(&st,&mut s,&mut c,stops,&mut std::collections::HashMap::new());
        let review=s.glowny().engine.continuation_review().unwrap();
        assert_eq!(review.scope,if enable_late{ContinuationReviewScope::Account}else{ContinuationReviewScope::Engine});
        assert!(!matches!(s.glowny().engine.entry_gate(&b,T+1000),Gate::Open));
        if let Some(old)=saved {assert_eq!(s.glowny().engine.export_strategy_continuation().unwrap().settings_contract_json,old.settings_contract_json);}
        drop(st);std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn profit_budget_preserves_day_anchor_without_optional_continuation_and_resets_next_day() {
    let mut c=cfg();c.restore_strategy_continuation=false;c.profit_budget_arm_pct=5.0;
    c.profit_budget_keep_pct=75.0;c.profit_budget_deploy_pct=100.0;
    let (mut s,mut b)=rig(c.clone());
    b.inner.balance+=200.0;tick(&mut s,&mut b,T+1000,4004.0);
    let original=conduit_core::profit_budget::BudgetAnchor::from(&s.glowny().engine.stats);
    assert_eq!(original.start,1000.0);assert!(original.peak>1190.0);
    let signature=sygnatura_ryzyka(&s);
    s.glowny_mut().engine.stats.day_peak_equity+=1.0;
    assert_ne!(sygnatura_ryzyka(&s),signature,"peak advancement must dirty the durable risk snapshot");
    let original=conduit_core::profit_budget::BudgetAnchor::from(&s.glowny().engine.stats);
    let before=conduit_core::profit_budget::available(&c,original,&b,None).unwrap().unwrap();
    let memory=memory_of(&mut s,&b);
    let (mut restored,reports)=restore_memory(c.clone(),memory,&b,ContinuationOrigin::Memory);
    assert!(reports.is_empty());
    let after_anchor=conduit_core::profit_budget::BudgetAnchor::from(&restored.glowny().engine.stats);
    assert_eq!(after_anchor,original);
    assert_eq!(conduit_core::profit_budget::available(&c,after_anchor,&b,None).unwrap(),Some(before));
    let before_count=b.positions().len();
    restored.glowny_mut().engine.on_message(&mut b,&msg(T+1000,101,TEXT,None));
    assert_eq!(b.positions().len(),before_count,"restart cannot erase consumed downside");
    let next_day=T+86_400_000;tick(&mut restored,&mut b,next_day,4004.0);
    let fresh=conduit_core::profit_budget::BudgetAnchor::from(&restored.glowny().engine.stats);
    assert_ne!(fresh.day,original.day);assert_eq!(fresh.start,fresh.peak);
    assert_eq!(conduit_core::profit_budget::available(&c,fresh,&b,None).unwrap(),None);
}

#[test]
fn profit_budget_panel_rejects_oversize_without_sending_or_silently_resizing() {
    let mut c=cfg();c.restore_strategy_continuation=false;c.profit_budget_arm_pct=5.0;
    let (mut s,mut b)=rig(c);b.inner.balance+=200.0;tick(&mut s,&mut b,T+1000,4004.0);
    // Remove existing downside through actual broker close, then book it.
    for p in b.positions().to_vec(){b.close_position(p.ticket,CloseReason::Manual).unwrap();}
    tick(&mut s,&mut b,T+2000,4004.0);
    let before=(b.positions().len(),b.pendings().len(),b.account().equity);
    let error=manual_profit_budget_allowed(&mut s,&b,Side::Buy,4000.0,Some(3990.0),1.0).unwrap_err();
    assert!(error.contains("maximum allowed volume:"));assert!(error.contains("requested volume is unchanged"));
    assert_eq!((b.positions().len(),b.pendings().len(),b.account().equity),before);
    assert!(manual_profit_budget_allowed(&mut s,&b,Side::Buy,4000.0,Some(3990.0),0.01).is_ok());
    assert!(manual_profit_budget_allowed(&mut s,&b,Side::Buy,4000.0,None,0.01).is_err());
    s.glowny_mut().engine.cfg.profit_budget_arm_pct=0.0;
    assert!(manual_profit_budget_allowed(&mut s,&b,Side::Buy,4000.0,None,1.0).is_ok());
}

#[test]
fn configured_portfolio_cap_also_constrains_explicit_manual_volume_before_profit_arm() {
    let mut c=cfg();c.restore_strategy_continuation=false;c.profit_budget_arm_pct=0.0;
    let (mut s,mut b)=rig(c);
    for p in b.positions().to_vec(){b.close_position(p.ticket,CloseReason::Manual).unwrap();}
    tick(&mut s,&mut b,T+1000,4004.0);
    s.glowny_mut().engine.cfg.max_portfolio_risk_pct=5.0;
    s.glowny_mut().engine.stats.day=i64::MIN; // portfolio does not depend on a day anchor
    let before=(b.positions().len(),b.pendings().len(),b.account().equity);
    let error=manual_profit_budget_allowed(&mut s,&b,Side::Buy,4000.0,Some(3990.0),1.0).unwrap_err();
    assert!(error.contains("RISK BUDGET"));assert!(error.contains("maximum allowed volume:"));
    assert!(error.contains("requested volume is unchanged"));
    assert!(manual_profit_budget_allowed(&mut s,&b,Side::Buy,4000.0,Some(3990.0),0.01).is_ok());
    assert!(manual_profit_budget_allowed(&mut s,&b,Side::Buy,4000.0,None,0.01).is_err());
    assert_eq!((b.positions().len(),b.pendings().len(),b.account().equity),before);
    s.glowny_mut().engine.cfg.max_portfolio_risk_pct=0.0;
    assert!(manual_profit_budget_allowed(&mut s,&b,Side::Buy,4000.0,None,1.0).is_ok());
}
