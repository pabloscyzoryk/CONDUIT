//! Public Engine/Sim source-edit proofs. No MT5, network or live orders.
use crate::sim::SimBroker;
use conduit_core::broker::*;
use conduit_core::engine::{Engine,IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;
const T:Ts=1_700_000_000_000;
const BUY:&str="BUY GOLD @ 4005/4000\nTP 4030\nTP 4060\nTP 4090\nSL 3990";
const SELL:&str="SELL GOLD @ 4000/4005\nTP 3970\nTP 3940\nTP 3910\nSL 4015";
fn cfg()->Settings {Settings{entry_edit_geometry_v2:true,auto_limit:true,entry_units:3,
    lot_fixed:0.03,lot_max:0.0,pending_lifetime:PendingLifetime::Never,
    pending_drop_on_target:false,tp_source:TpSource::PriceOnly,tp_price_only_strict:true,
    swap_enabled:false,max_dd_pct:0.0,max_dd_usd:0.0,equity_floor_pct:0.0,..Settings::default()}}
fn msg(ts:Ts,text:&str,edit:bool)->IncomingMessage{IncomingMessage{ts,
    source:SourceKey::new(-990003,None),source_name:"entry-edit-contract".into(),
    msg_id:1,reply_to:None,edit_of:edit.then_some(1),text:text.into()}}
fn q(ts:Ts,bid:f64)->Quote{Quote{ts,bid,ask:bid+0.2}}
struct Tape {inner:SimBroker,proof:bool,barrier:bool,cancels:usize,mods:usize,opens:usize,
    reject_mod:Option<usize>,reject_cancel:Option<usize>,ack_keep:bool,fill_cancel:bool,
    high:f64,generation:u64,mutate_after_place:u8,addon_calls:usize,
    ambiguous_addon_once:bool,delayed_addons:Vec<Position>}
impl Broker for Tape {
    fn quote(&self)->Quote{self.inner.quote()}
    fn account(&self)->Account{self.inner.account()}
    fn stops_level(&self)->f64{self.inner.stops_level()}
    fn volume_min(&self)->f64{self.inner.volume_min()}
    fn volume_step(&self)->f64{self.inner.volume_step()}
    fn volume_max(&self)->f64{self.inner.volume_max()}
    fn pending_cancel_snapshot_authoritative(&self)->bool{self.proof}
    fn close_receipts_pending(&self)->bool{self.barrier}
    fn execution_session(&self)->Option<ExecutionSession>{Some(ExecutionSession{scope:"synthetic-C".into(),generation:self.generation})}
    fn positions(&self)->&[Position]{self.inner.positions()}
    fn pendings(&self)->&[PendingOrder]{self.inner.pendings()}
    fn positions_mut(&mut self)->&mut Vec<Position>{self.inner.positions_mut()}
    fn pendings_mut(&mut self)->&mut Vec<PendingOrder>{self.inner.pendings_mut()}
    fn open_market(&mut self,r:OrderReq)->BResult<Ticket>{
        self.opens+=1;
        if r.level == -4 {
            self.addon_calls+=1;
            if self.ambiguous_addon_once {
                self.ambiguous_addon_once=false;
                let ticket=self.inner.open_market(r)?;
                let idx=self.inner.positions().iter().position(|p|p.ticket==ticket).unwrap();
                self.delayed_addons.push(self.inner.positions_mut().remove(idx));
                self.mark();
                return Err(BrokerError::Rejected);
            }
        }
        let out=self.inner.open_market(r);self.mark();out
    }
    fn place_pending(&mut self,r:PendingReq)->BResult<Ticket>{self.opens+=1;let out=self.inner.place_pending(r);
        if let Ok(t)=out {match self.mutate_after_place {
            1=>self.generation+=1,2=>self.barrier=true,
            3=>{self.inner.pendings_mut().iter_mut().find(|p|p.ticket==t).unwrap().sl=None;},_=>{}
        }self.mutate_after_place=0;}self.mark();out}
    fn modify_position(&mut self,t:Ticket,s:Option<Px>,p:Option<Px>)->BResult<()>{
        self.mods+=1;if self.reject_mod==Some(self.mods){return Err(BrokerError::Rejected);}self.inner.modify_position(t,s,p)}
    fn modify_pending(&mut self,t:Ticket,x:Px,s:Option<Px>,p:Option<Px>)->BResult<()>{
        self.mods+=1;if self.reject_mod==Some(self.mods){return Err(BrokerError::Rejected);}self.inner.modify_pending(t,x,s,p)}
    fn close_position(&mut self,t:Ticket,r:CloseReason)->BResult<f64>{self.inner.close_position(t,r)}
    fn close_partial(&mut self,t:Ticket,v:f64,r:CloseReason)->BResult<f64>{self.inner.close_partial(t,v,r)}
    fn drain_closed(&mut self)->Vec<ClosedTrade>{self.inner.drain_closed()}
    fn cancel_pending(&mut self,t:Ticket)->BResult<()> {
        self.cancels+=1;if self.reject_cancel==Some(self.cancels){return Err(BrokerError::Rejected);}
        if self.ack_keep{return Ok(());}
        if self.fill_cancel {
            self.fill_cancel=false;let p=self.pendings().iter().find(|p|p.ticket==t).unwrap().clone();
            self.inner.open_market(OrderReq{side:p.kind.side(),volume:p.volume,sl:p.sl,tp:p.tp,
                basket:p.basket,level:p.level,is_toucher:p.is_toucher,comment:p.comment})?;
        }
        self.inner.cancel_pending(t)
    }
}
impl Tape {fn mark(&mut self){self.high=self.high.max(self.positions().iter().map(|p|p.volume).sum::<f64>()
    +self.pendings().iter().map(|p|p.volume).sum::<f64>());}
    fn reveal_delayed_addons(&mut self){
        self.inner.positions_mut().append(&mut self.delayed_addons);
    }}
fn rig(c:Settings,text:&str,bid:f64)->(Engine,Tape){
    let mut b=Tape{inner:SimBroker::z_ustawien(1000.0,&c),proof:true,barrier:false,cancels:0,
        mods:0,opens:0,reject_mod:None,reject_cancel:None,ack_keep:false,fill_cancel:false,
        high:0.0,generation:0,mutate_after_place:0,addon_calls:0,
        ambiguous_addon_once:false,delayed_addons:Vec::new()};
    let mut e=Engine::new(c,1000.0);tick(&mut e,&mut b,T,bid);e.on_message(&mut b,&msg(T,text,false));
    assert_eq!(e.baskets.len(),1);(e,b)
}
fn tick(e:&mut Engine,b:&mut Tape,ts:Ts,bid:f64){let q=q(ts,bid);b.inner.on_quote(q);e.on_tick(b,&q);}
fn edit(e:&mut Engine,b:&mut Tape,text:&str){e.on_message(b,&msg(T+2000,text,true));}
fn review(e:&Engine)->&str{&e.baskets[0].entry_edit_state.as_ref().unwrap().review.as_ref().unwrap().reason}
fn near(a:f64,b:f64){assert!((a-b).abs()<1e-8,"{a} != {b}");}

#[test]
fn source_snapshot_is_optional_off_and_versioned_on(){
    let mut c=cfg();c.entry_edit_geometry_v2=false;let(e,_)=rig(c,BUY,4012.0);
    let j=serde_json::to_string(&e.baskets).unwrap();assert!(!j.contains("entry_edit_state"));
    let old:Vec<Basket>=serde_json::from_str(&j).unwrap();assert!(old[0].entry_edit_state.is_none());
    let(e,_)=rig(cfg(),BUY,4012.0);let state=e.baskets[0].entry_edit_state.as_ref().unwrap();
    assert_eq!((state.schema_version,state.revision),(1,1));assert!(state.source.is_some());
}
#[test]
fn tp_only_edit_confirms_broker_targets_inplace(){
    let mut c=cfg();c.tp_schedule=TpSchedule::AllAtTp1;c.assign_tp_per_position=true;
    let(mut e,mut b)=rig(c,BUY,4012.0);let tickets:Vec<_>=b.pendings().iter().map(|p|p.ticket).collect();
    edit(&mut e,&mut b,&BUY.replace("TP 4030","TP 4031"));
    assert_eq!(e.baskets[0].tps[0],4031.0);assert!(b.pendings().iter().all(|p|p.tp==Some(4031.0)));
    assert_eq!(tickets,b.pendings().iter().map(|p|p.ticket).collect::<Vec<_>>());assert_eq!(b.cancels,0);
    assert_eq!(e.baskets[0].entry_edit_state.as_ref().unwrap().revision,2);
}
#[test]
fn cosmetic_runner_edit_does_not_replace_grid_or_reset_progress(){
    let mut c=cfg();c.runner_cele_n=2;c.runner_cele_krok=5.0;
    let(mut e,mut b)=rig(c,BUY,4012.0);e.baskets[0].tp_stage=1;e.baskets[0].tp_touch_ts=vec![T+1];
    let before=serde_json::to_vec(b.pendings()).unwrap();let calls=(b.cancels,b.mods,b.opens);
    b.inner.balance=5000.0;b.inner.on_quote(q(T+2000,4020.0));
    edit(&mut e,&mut b,&format!("{BUY}\nUPDATED NOTE"));
    assert_eq!(before,serde_json::to_vec(b.pendings()).unwrap());assert_eq!(calls,(b.cancels,b.mods,b.opens));
    assert_eq!(e.baskets[0].tp_stage,1);assert_eq!(e.baskets[0].tp_touch_ts,vec![T+1]);
}
#[test]
fn cosmetic_edit_preserves_actual_better_stop_buy_and_sell(){
    for (text,bid,next,better) in [(BUY,4004.0,4008.0,4006.0),(SELL,4001.0,3997.0,3999.0)] {
        let mut c=cfg();c.entry_units=1;c.auto_limit=false;c.be_never_loosen=true;
        let(mut e,mut b)=rig(c,text,bid);let p=b.positions()[0].clone();
        b.inner.on_quote(q(T+1000,next));b.inner.modify_position(p.ticket,Some(better),p.tp).unwrap();
        let calls=b.mods;edit(&mut e,&mut b,&format!("{text}\nUPDATED NOTE"));
        assert_eq!(b.positions()[0].sl,Some(better));assert_eq!(b.mods,calls);
    }
}
#[test]
fn three_to_four_pips_updates_both_mirrored_grids_with_proof(){
    for (text,bid,word) in [(BUY,4012.0,"ADDING"),(SELL,3993.0,"SUBTRACTING")] {
        let mut c=cfg();c.entry_warstwy_z_tekstu=true;
        let s=format!("{text}\n{word} 3 PIPS TO EACH LIMIT ORDER");
        let(mut e,mut b)=rig(c,&s,bid);let old:Vec<_>=b.pendings().iter().map(|p|p.price).collect();
        edit(&mut e,&mut b,&s.replace(&format!("{word} 3"),&format!("{word} 4")));
        near(e.baskets[0].warstwy_offset.unwrap(),0.4);
        assert_ne!(old,b.pendings().iter().map(|p|p.price).collect::<Vec<_>>());
        assert!(e.baskets[0].entry_edit_state.as_ref().unwrap().review.is_none());near(b.high,0.09);
    }
}
#[test]
fn text_offset_disabled_is_metadata_only_no_order_rpc(){
    let s=format!("{BUY}\nADDING 3 PIPS TO EACH LIMIT ORDER");
    let(mut e,mut b)=rig(cfg(),&s,4012.0);let before=serde_json::to_vec(b.pendings()).unwrap();
    edit(&mut e,&mut b,&s.replace("ADDING 3","ADDING 4"));
    near(e.baskets[0].warstwy_offset.unwrap(),0.4);assert_eq!(b.cancels,0);assert_eq!(b.mods,0);
    assert_eq!(before,serde_json::to_vec(b.pendings()).unwrap());
}
#[test]
fn done_edit_cannot_mutate_history_or_broker(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);e.baskets[0].state=BasketState::Done;e.baskets[0].tp_stage=2;
    let old=e.baskets[0].tps.clone();let n=b.opens;
    edit(&mut e,&mut b,&BUY.replace("TP 4030","TP 4031"));
    assert_eq!(e.baskets[0].tps,old);assert_eq!(e.baskets[0].tp_stage,2);assert_eq!((b.cancels,b.mods,b.opens),(0,0,n));
}
#[test]
fn tp_change_after_observed_stage_is_review_not_reset(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);e.baskets[0].tp_stage=1;
    edit(&mut e,&mut b,&BUY.replace("TP 4030","TP 4031"));
    assert_eq!(review(&e),"ObservedTargetsNeedProgressMigration");assert_eq!(e.baskets[0].tp_stage,1);assert_eq!(b.mods,0);
}
#[test]
fn missing_source_does_not_guess_original_stop(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);e.baskets[0].entry_edit_state=None;
    edit(&mut e,&mut b,BUY);assert_eq!(review(&e),"MissingSourceSnapshot");assert_eq!(b.mods,0);
}
#[test]
fn partial_modify_failure_does_not_commit_desired_source(){
    let mut c=cfg();c.tp_schedule=TpSchedule::AllAtTp1;
    let(mut e,mut b)=rig(c,BUY,4012.0);b.reject_mod=Some(2);
    edit(&mut e,&mut b,&BUY.replace("TP 4030","TP 4031"));
    assert_eq!(review(&e),"PendingModifyUnconfirmed");assert_eq!(e.baskets[0].tps[0],4030.0);
    assert_eq!(b.pendings().iter().filter(|p|p.tp==Some(4031.0)).count(),1);
    assert_eq!(e.baskets[0].entry_edit_state.as_ref().unwrap().revision,1);
}
#[test]
fn unavailable_cancel_proof_preserves_all_old_pending_orders(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);b.proof=false;let old=serde_json::to_vec(b.pendings()).unwrap();
    edit(&mut e,&mut b,&BUY.replace("4005/4000","4006/4000"));
    assert_eq!(review(&e),"CancelFillProofUnavailable");assert_eq!(old,serde_json::to_vec(b.pendings()).unwrap());assert_eq!(b.cancels,0);
}
#[test]
fn cancel_failure_and_ack_still_present_never_replace(){
    for keep in [false,true]{let(mut e,mut b)=rig(cfg(),BUY,4012.0);let opens=b.opens;
        b.ack_keep=keep;b.reject_cancel=(!keep).then_some(2);
        edit(&mut e,&mut b,&BUY.replace("4005/4000","4006/4000"));
        assert_eq!(review(&e),"CancelUnconfirmed");assert_eq!(b.opens,opens);assert!(!b.pendings().is_empty());}
}
#[test]
fn fill_during_cancel_halts_replacement_without_closing_the_fill(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);let opens=b.opens;b.fill_cancel=true;
    edit(&mut e,&mut b,&BUY.replace("4005/4000","4006/4000"));
    assert_eq!(review(&e),"FillOrSessionChangedDuringCancel");assert_eq!(b.opens,opens);assert_eq!(b.positions().len(),1);
}
#[test]
fn receipt_review_survives_clear_restart_and_blocks_only_its_basket(){
    let mut c=cfg();c.rearm_grid_on_return=true;c.rearm_bez_pozycji=true;c.rearm_min_gap_min=0.0;
    let(mut e,mut b)=rig(c,BUY,4012.0);b.barrier=true;
    edit(&mut e,&mut b,&BUY.replace("4005/4000","4006/4000"));assert_eq!(review(&e),"ReceiptBarrier");
    let saved=serde_json::to_string(&e.baskets).unwrap();e.baskets=serde_json::from_str(&saved).unwrap();
    b.barrier=false;let ids:Vec<_>=b.pendings().iter().map(|p|p.ticket).collect();
    for t in ids {b.inner.cancel_pending(t).unwrap();}e.baskets[0].pendings.clear();
    tick(&mut e,&mut b,T+5000,4003.0);assert!(b.positions().is_empty()&&b.pendings().is_empty());
    b.inner.on_quote(q(T+6000,4012.0));let mut new=msg(T+6000,BUY,false);new.msg_id=2;
    e.on_message(&mut b,&new);assert_eq!(e.baskets.len(),2);assert!(!b.pendings().is_empty());
    assert!(b.pendings().iter().all(|p|p.basket==Some(e.baskets[1].id)));
}
#[test]
fn actual_fill_even_with_armed_metadata_forbids_geometry_replacement(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);let p=b.pendings()[0].clone();
    b.inner.open_market(OrderReq{side:Side::Buy,volume:p.volume,sl:p.sl,tp:p.tp,basket:p.basket,
        level:p.level,is_toucher:false,comment:String::new()}).unwrap();
    assert_eq!(e.baskets[0].state,BasketState::Armed);
    edit(&mut e,&mut b,&BUY.replace("4005/4000","4006/4000"));
    assert_eq!(review(&e),"WorkingGeometryNeedsFillRevisionProof");assert_eq!(b.cancels,0);
}
#[test]
fn real_stop_edit_never_loosens_working_position(){
    let mut c=cfg();c.entry_units=1;c.auto_limit=false;
    let(mut e,mut b)=rig(c,BUY,4004.0);let p=b.positions()[0].clone();b.inner.on_quote(q(T+1000,4008.0));
    b.inner.modify_position(p.ticket,Some(4006.0),p.tp).unwrap();
    edit(&mut e,&mut b,&BUY.replace("SL 3990","SL 3989"));assert_eq!(b.positions()[0].sl,Some(4006.0));
}
#[test]
fn pending_exit_has_priority_over_edit(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);e.baskets[0].pending_exit=Some(PendingBasketExit{reason:CloseReason::Manual,last_attempt_ts:T});
    let old=e.baskets[0].tps.clone();edit(&mut e,&mut b,&BUY.replace("TP 4030","TP 4031"));
    assert_eq!(e.baskets[0].tps,old);assert_eq!(b.mods,0);
}
#[test]
fn malformed_source_state_schema_is_not_used_as_valid_snapshot(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);e.baskets[0].entry_edit_state.as_mut().unwrap().schema_version=99;
    edit(&mut e,&mut b,BUY);assert_eq!(review(&e),"MissingSourceSnapshot");assert_eq!(b.mods,0);
}

#[test]
fn tp_open_updates_actual_runner_target_without_cancel(){
    let mut c=cfg();c.tp_schedule=TpSchedule::AllRunners;c.tp_open_extra=true;c.tp_open_offset=20.0;
    let(mut e,mut b)=rig(c,BUY,4012.0);assert!(b.pendings().iter().all(|p|p.tp==Some(4090.0)));
    edit(&mut e,&mut b,&format!("{BUY}\nTP OPEN"));
    assert!(e.baskets[0].tp_open);assert!(b.pendings().iter().all(|p|p.tp==Some(4110.0)));
    assert_eq!(b.cancels,0);let calls=b.mods;
    edit(&mut e,&mut b,&format!("{BUY}\nTP OPEN\nUPDATED NOTE"));assert_eq!(b.mods,calls);
}

#[test]
fn direction_edit_never_reverses_existing_basket(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);let before=serde_json::to_vec(b.pendings()).unwrap();
    edit(&mut e,&mut b,SELL);assert_eq!(e.baskets[0].side,Side::Buy);
    assert_eq!(before,serde_json::to_vec(b.pendings()).unwrap());assert_eq!((b.cancels,b.mods),(0,0));
}

#[test]
fn frozen_order_prevents_partial_modify_before_first_rpc(){
    let(mut e,mut b)=rig(cfg(),BUY,4012.0);b.pendings_mut()[1].frozen=true;
    edit(&mut e,&mut b,&BUY.replace("TP 4030","TP 4031"));
    assert_eq!(review(&e),"FrozenExposure");assert_eq!(b.mods,0);
}

#[test]
fn warnings_state_live_geometry_and_restart_limits(){
    let warnings=cfg().pulapki_konfiguracji();assert!(warnings.iter().any(|v|v.contains("entry_edit_geometry_v2")&&v.contains("RequiresReview")));
    let mut c=cfg();c.entry_edit_geometry_v2=false;
    assert!(!c.pulapki_konfiguracji().iter().any(|v|v.contains("entry_edit_geometry_v2")));
}

#[test]
fn metadata_only_offset_with_unmapped_addon_cannot_panic_or_touch_orders(){
    let s=format!("{BUY}\nADDING 3 PIPS TO EACH LIMIT ORDER");
    let(mut e,mut b)=rig(cfg(),&s,4012.0);
    b.inner.open_market(OrderReq{side:Side::Buy,volume:0.03,sl:Some(3990.0),tp:None,
        basket:Some(e.baskets[0].id),level:-4,is_toucher:false,comment:"test-addon".into()}).unwrap();
    let before=serde_json::to_vec(b.positions()).unwrap();let mods=b.mods;
    edit(&mut e,&mut b,&s.replace("ADDING 3","ADDING 4"));
    assert_eq!(before,serde_json::to_vec(b.positions()).unwrap());assert_eq!(b.mods,mods);
    near(e.baskets[0].warstwy_offset.unwrap(),0.4);
}

#[test]
fn replacement_requires_post_placement_session_receipt_and_stop_proof(){
    for fault in [1,2,3] {
        let mut c=cfg();c.entry_units=1;
        let(mut e,mut b)=rig(c,BUY,4012.0);b.mutate_after_place=fault;
        edit(&mut e,&mut b,&BUY.replace("4005/4000","4006/4000"));
        assert!(e.baskets[0].entry_edit_state.as_ref().unwrap().review.is_some(),"fault={fault}");
        assert_eq!(e.baskets[0].entry_edit_state.as_ref().unwrap().revision,1,"unproven replacement cannot commit");
    }
}

#[test]
fn geometry_edit_cannot_erase_unfilled_plan_progress_or_reenter_after_tp(){
    for touched in [false,true] {
        let(mut e,mut b)=rig(cfg(),BUY,4012.0);
        if touched {e.baskets[0].plan_wykonany_do=1;e.baskets[0].tp_touch_ts=vec![T+1000];}
        else {b.inner.on_quote(q(T+1000,4031.0));}
        edit(&mut e,&mut b,&BUY.replace("4005/4000","4006/4000"));
        assert!(e.baskets[0].entry_edit_state.as_ref().unwrap().review.is_some(),"touched={touched}");
        assert_eq!(b.cancels,0);
        if touched {assert_eq!(e.baskets[0].plan_wykonany_do,1);assert_eq!(e.baskets[0].tp_touch_ts,vec![T+1000]);}
    }
}

#[test]
fn real_b41_identical_edit_is_noop_and_material_sl_tp_delta_applies_once(){
    const INITIAL:&str="BUY LIMITS GOLD @ 4100/4094 AREA\nTP 4103\nTP 4107\nTP 4112\nTP OPEN\nSL 4093";
    const MATERIAL:&str="BUY LIMITS GOLD @ 4100/4094 AREA\nTP 4103\nTP 4107\nTP 4120\nTP OPEN\nSL 4090";
    let mut c=cfg();c.entry_units=7;c.lot_fixed=0.01;c.dedup_edited_signals=true;
    let(mut e,mut b)=rig(c,INITIAL,4381.64);
    assert_eq!(b.pendings().len(),7);assert!(b.positions().is_empty());

    let initial_wire=serde_json::to_vec(b.pendings()).unwrap();
    let initial_calls=(b.opens,b.mods,b.cancels);
    edit(&mut e,&mut b,INITIAL);
    assert_eq!(initial_wire,serde_json::to_vec(b.pendings()).unwrap());
    assert_eq!(initial_calls,(b.opens,b.mods,b.cancels),"identical replay must be a semantic no-op");

    edit(&mut e,&mut b,MATERIAL);
    assert_eq!(e.baskets[0].sl,Some(4364.0));assert_eq!(e.baskets[0].tps[2],4394.0);
    assert_eq!(b.pendings().len(),7,"material edit must update, never duplicate, the grid");
    assert!(b.positions().is_empty(),"an edit must not re-enter at market");
    assert!(b.pendings().iter().all(|p|p.sl==Some(4364.0)));

    let material_wire=serde_json::to_vec(b.pendings()).unwrap();
    let material_calls=(b.opens,b.mods,b.cancels);
    edit(&mut e,&mut b,&format!("{MATERIAL}\nFIRST ENTRY CAN BE AT ANY LEVEL OF 4374"));
    assert_eq!(material_wire,serde_json::to_vec(b.pendings()).unwrap());
    assert_eq!(material_calls,(b.opens,b.mods,b.cancels),"cosmetic follow-up must not replay the delta");
}

fn fast_addon_cfg()->Settings{
    let mut c=cfg();c.auto_limit=false;c.entry_units=1;c.market_entry_units=1;
    c.market_entry_mode=MarketEntryMode::Single;c.lot_fixed=0.01;
    c.fast_addon_move_usd=1.0;c.fast_addon_window_s=60.0;c.fast_addon_max=1;
    c.fast_addon_lot_mult=1.0;c.fast_addon_cooldown_s=0.0;c.fast_addon_min_stage=0;
    c
}

#[test]
fn ambiguous_fast_addon_ack_and_delayed_snapshot_never_send_twice(){
    let(mut e,mut b)=rig(fast_addon_cfg(),BUY,4004.0);
    b.ambiguous_addon_once=true;
    tick(&mut e,&mut b,T+6000,4004.5);tick(&mut e,&mut b,T+12000,4005.5);
    assert_eq!(b.addon_calls,1);assert_eq!(b.delayed_addons.len(),1);
    assert_eq!(e.baskets[0].fast_addons,1,"a dispatched ambiguous attempt consumes max=1");

    // Reproduce a stale/restored engine snapshot whose counter has not yet
    // learned about the accepted MT5 position. The next authoritative broker
    // snapshot must repair it before eligibility is evaluated.
    e.baskets[0].fast_addons=0;b.reveal_delayed_addons();
    tick(&mut e,&mut b,T+18000,4006.0);
    assert_eq!(e.baskets[0].fast_addons,1,"visible level=-4 position reconciles the counter");
    assert_eq!(b.positions().iter().filter(|p|p.level==-4).count(),1);
    tick(&mut e,&mut b,T+30000,4008.0);
    assert_eq!(b.addon_calls,1,"max=1 must hold after the delayed snapshot");
}

#[test]
fn fast_addon_with_tp_already_behind_market_is_consumed_without_broker_spam(){
    let mut c=fast_addon_cfg();c.tp_source=TpSource::SignalOnly;
    let(mut e,mut b)=rig(c,BUY,4004.0);
    e.baskets[0].tps=vec![4004.0];
    b.positions_mut()[0].tp=None;
    tick(&mut e,&mut b,T+6000,4004.5);tick(&mut e,&mut b,T+12000,4005.5);
    assert_eq!(b.addon_calls,0,"stale TP must be rejected before the broker RPC");
    assert_eq!(e.baskets[0].fast_addons,1,"stale opportunity consumes max=1");
    tick(&mut e,&mut b,T+18000,4007.0);
    assert_eq!(b.addon_calls,0,"stale TP must not retry on later ticks");
}
