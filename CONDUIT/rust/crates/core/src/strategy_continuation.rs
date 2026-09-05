//! Bounded continuation A. Not a durable receipt/checkpoint protocol.
//! Kept as an Engine child so only explicit export/import exposes private state.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContinuationReviewScope { Engine, Account }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuationReview {
    pub scope: ContinuationReviewScope,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationOrigin {
    /// New local bot instance: no prior local projections and no owned exposure.
    /// This is not proof that the brokerage account has never traded before.
    Fresh,
    /// Complete in-process projection from the same account and owner.
    Memory,
    /// Saved projections have no common atomic generation in stage A.
    UnverifiedDisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionIntentGuardV1 {
    pub position_identifier: u64,
    pub basket: u32,
    pub side: Side,
    pub level: i32,
    pub open_ts: Ts,
    pub open_price: Px,
    pub maximum_volume: f64,
    pub geometry_revision: String,
    pub previous_sl: Option<Px>,
    pub previous_tp: Option<Px>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesiredContinuationV1 {
    pub ticket: Ticket,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    pub last_try: Ts,
    pub guard: Option<PositionIntentGuardV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedExitContinuationV1 {
    pub ticket: Ticket,
    pub target: Px,
    pub deadline: Ts,
    pub reason: CloseReason,
    pub market_at_decision: Px,
    pub guard: Option<PositionIntentGuardV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineContinuationV1 {
    pub schema_version: u32,
    pub scope: Option<String>,
    pub owner: String,
    pub session_offset_ms: i64,
    /// Exact canonical typed Settings contract; contains no authentication data.
    pub settings_contract_json: String,
    pub day_stop: i64,
    pub desired: Vec<DesiredContinuationV1>,
    pub queued_exits: Vec<QueuedExitContinuationV1>,
    pub review: Option<ContinuationReview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationImportReport {
    pub imported_stops: usize,
    pub imported_exits: usize,
    pub review: Option<ContinuationReview>,
}

#[derive(Default)]
pub(super) struct ContinuationRuntime {
    scope: Option<String>,
    owner: String,
    settings_contract_json: String,
    ready: bool,
    review: Option<ContinuationReview>,
    desired_guards: HashMap<Ticket, PositionIntentGuardV1>,
    exit_guards: HashMap<Ticket, PositionIntentGuardV1>,
}

fn same_px(a: Option<Px>, b: Option<Px>) -> bool {
    match (a,b) { (None,None)=>true, (Some(a),Some(b))=>a.is_finite()&&b.is_finite()&&(a-b).abs()<=1e-8, _=>false }
}
fn finite_px(p: Option<Px>) -> bool { p.is_none_or(|x|x.is_finite()&&x>0.0) }

impl Engine {
    fn continuation_settings_contract(&self)->String {
        serde_json::to_string(&self.cfg).unwrap_or_default()
    }
    pub fn continuation_review(&self) -> Option<&ContinuationReview> { self.continuation.review.as_ref() }

    /// Ordinary ResumeTrading must not erase an unproved continuation.
    pub fn hold_strategy_continuation(&mut self, scope: ContinuationReviewScope, reason: impl Into<String>) {
        let reason=reason.into();
        let replace=self.continuation.review.as_ref().is_none_or(|old|
            old.scope==ContinuationReviewScope::Engine && scope==ContinuationReviewScope::Account);
        if replace { self.continuation.review=Some(ContinuationReview{scope,reason:reason.clone()}); }
        if replace || self.halted.is_none() {
            let marker=format!("CONTINUATION REVIEW: {reason}");
            self.halted=Some(match self.halted.take() {
                Some(previous) if !previous.contains("CONTINUATION REVIEW:")=>format!("{previous}; {marker}"),
                Some(previous)=>previous,
                None=>marker,
            });
        }
    }

    pub fn continuation_entry_blocked(&self) -> bool {
        self.continuation.review.is_some() || (self.cfg.restore_strategy_continuation && !self.continuation.ready)
    }

    /// Call after an explicit configuration replacement, never every tick.
    /// A different policy is not authority to reuse old pending intentions.
    pub fn continuation_configuration_changed(&mut self) {
        if self.continuation.ready {
            if self.continuation.settings_contract_json!=self.continuation_settings_contract() {
                self.hold_strategy_continuation(ContinuationReviewScope::Engine,"effective Settings changed in runtime; continuation needs review");
            }
        } else if self.cfg.restore_strategy_continuation {
            self.hold_strategy_continuation(ContinuationReviewScope::Account,"continuation enabled in runtime without Fresh/restore proof");
        }
    }

    pub(super) fn continuation_observe<B:Broker>(&mut self,b:&B) {
        if !self.cfg.restore_strategy_continuation { return; }
        if self.halted.is_none() {
            if let Some(review)=self.continuation.review.clone() {self.hold_strategy_continuation(review.scope,review.reason);}
        }
        if self.continuation.ready && b.execution_session().map(|s|s.scope)!=self.continuation.scope {
            self.hold_strategy_continuation(ContinuationReviewScope::Account,"broker scope changed or is no longer verified");
        }
    }

    fn continuation_geometry(&self, basket:u32)->Result<String,String> {
        let bk=self.basket(basket).ok_or("position owner was not adopted")?;
        if !bk.alive() || bk.pending_exit.is_some() || self.entry_edit_blocks(Some(basket)) {
            return Err("basket is terminal, exiting or its edit requires review".into());
        }
        if ![bk.entry_lo,bk.entry_hi,bk.zone_lo,bk.zone_hi].iter().all(|v|v.is_finite())
            || !finite_px(bk.sl) || !bk.tps.iter().all(|v|v.is_finite()) { return Err("nonfinite basket geometry".into()); }
        // Setup revision only: do not confuse a changing fill count/peak with an edit.
        serde_json::to_string(&(&bk.source,bk.msg_id,bk.side,bk.created_ts,
            bk.entry_lo,bk.entry_hi,bk.zone_lo,bk.zone_hi,bk.sl,&bk.tps,
            bk.entry_edit_state.as_ref().map(|s|s.revision)))
            .map_err(|e|format!("cannot encode setup revision: {e}"))
    }

    pub(super) fn capture_continuation_guard<B:Broker>(&self,b:&B,t:Ticket)->Result<PositionIntentGuardV1,ContinuationReview> {
        let review=|scope,reason:&str|ContinuationReview{scope,reason:reason.into()};
        if !self.continuation.ready { return Err(review(ContinuationReviewScope::Account,"continuation has not been bound to an explicit fresh/restore origin")); }
        if self.continuation.settings_contract_json!=self.continuation_settings_contract() {
            return Err(review(ContinuationReviewScope::Engine,"effective Settings changed after continuation binding"));
        }
        if b.execution_session().map(|s|s.scope)!=self.continuation.scope { return Err(review(ContinuationReviewScope::Account,"unverified account scope")); }
        let p=b.find_position(t).ok_or_else(||review(ContinuationReviewScope::Account,"position and its owner are missing"))?;
        if p.frozen || !p.volume.is_finite() || p.volume<=0.0 || !p.open_price.is_finite() || p.open_price<=0.0 {
            return Err(review(ContinuationReviewScope::Engine,"frozen or invalid position"));
        }
        let basket=p.basket.ok_or_else(||review(ContinuationReviewScope::Account,"position has no proved basket owner"))?;
        if self.basket(basket).is_none() {return Err(review(ContinuationReviewScope::Account,"position owner was not adopted"));}
        let id=b.position_identifier(t).filter(|id|*id!=0).ok_or_else(||review(ContinuationReviewScope::Engine,"stable position identifier is unknown"))?;
        Ok(PositionIntentGuardV1{position_identifier:id,basket,side:p.side,level:p.level,
            open_ts:p.open_ts,open_price:p.open_price,maximum_volume:p.volume,
            geometry_revision:self.continuation_geometry(basket).map_err(|reason|ContinuationReview{scope:ContinuationReviewScope::Engine,reason})?,previous_sl:p.sl,previous_tp:p.tp})
    }

    pub(super) fn remember_continuation_guard(&mut self,t:Ticket,guard:Result<PositionIntentGuardV1,ContinuationReview>,exit:bool) {
        if !self.cfg.restore_strategy_continuation { return; }
        match guard {
            Ok(g)=>{if exit {self.continuation.exit_guards.insert(t,g);} else {self.continuation.desired_guards.insert(t,g);} }
            Err(review)=>{
                // A replacement must never borrow the previous intent's proof.
                self.forget_continuation_guard(t,exit);
                self.hold_strategy_continuation(review.scope,format!("intent #{t}: {}",review.reason));
            },
        }
    }

    pub(super) fn forget_continuation_guard(&mut self,t:Ticket,exit:bool) {
        if exit {self.continuation.exit_guards.remove(&t);} else {self.continuation.desired_guards.remove(&t);}
    }

    fn prove_continuation_position<B:Broker>(&self,b:&B,g:&PositionIntentGuardV1)->Result<Position,String> {
        if self.continuation.settings_contract_json!=self.continuation_settings_contract() {return Err("effective Settings contract changed".into());}
        if b.execution_session().map(|s|s.scope)!=self.continuation.scope {return Err("account scope is not verified".into());}
        if b.receipt_barrier()!=ReceiptBarrier::Clear {return Err("receipt barrier is not clear".into());}
        let mut matches=b.positions().iter().filter(|p| b.position_identifier(p.ticket)==Some(g.position_identifier));
        let p=matches.next().ok_or("stable position is missing")?;
        if matches.next().is_some() {return Err("ambiguous stable position identifier".into());}
        if p.frozen || p.basket!=Some(g.basket) || p.side!=g.side || p.level!=g.level
            || p.open_ts!=g.open_ts || !same_px(Some(p.open_price),Some(g.open_price))
            || !p.volume.is_finite() || p.volume<=0.0 || p.volume>g.maximum_volume+1e-9 {
            return Err("position ownership/shape changed or position is frozen".into());
        }
        if self.continuation_geometry(g.basket)?!=g.geometry_revision {return Err("basket setup revision changed".into());}
        Ok(p.clone())
    }

    fn confirmed_stops(&self,p:&Position,g:&PositionIntentGuardV1,sl:Option<Px>,tp:Option<Px>)->Result<(Option<Px>,Option<Px>),String> {
        if !finite_px(sl)||!finite_px(tp) {return Err("invalid intended stops".into());}
        if !same_px(p.tp,g.previous_tp) && !same_px(p.tp,tp) {return Err("broker TP superseded the stored intent".into());}
        let tightened=match (p.sl,sl) {
            (Some(now),Some(want))=>now.is_finite() && (now-want)*p.side.sign()>1e-8,
            (Some(now),None)=>now.is_finite(),
            _=>false,
        };
        if tightened {return Ok((p.sl,tp));}
        if !same_px(p.sl,g.previous_sl) && !same_px(p.sl,sl) {return Err("broker SL changed without a confirmed dominating protection".into());}
        Ok((sl,tp))
    }

    /// Validate immediately before every restored retry too, not just import.
    pub(super) fn continuation_retry_stops<B:Broker>(&mut self,b:&B,t:Ticket,d:DesiredStops)->Option<DesiredStops> {
        if !self.cfg.restore_strategy_continuation {return Some(d);}
        let result=self.continuation.desired_guards.get(&t).cloned().ok_or_else(||"missing intent identity/revision".to_owned())
            .and_then(|g| {let p=self.prove_continuation_position(b,&g)?;
                if p.ticket!=t {return Err("position alias changed after import".into());}
                let(sl,tp)=self.confirmed_stops(&p,&g,d.sl,d.tp)?;
                Ok(DesiredStops{sl,tp,last_try:d.last_try})});
        match result {Ok(d)=>Some(d),Err(e)=>{self.hold_strategy_continuation(ContinuationReviewScope::Engine,format!("SL/TP retry #{t}: {e}"));None}}
    }

    pub(super) fn continuation_exit_proved<B:Broker>(&mut self,b:&B,t:Ticket)->bool {
        if !self.cfg.restore_strategy_continuation {return true;}
        let result=self.continuation.exit_guards.get(&t).cloned().ok_or_else(||"missing exit identity/revision".to_owned())
            .and_then(|g|self.prove_continuation_position(b,&g));
        match result {Ok(p) if p.ticket==t=>true,_=>{self.hold_strategy_continuation(ContinuationReviewScope::Engine,format!("exit #{t} lacks current ownership/revision proof"));false}}
    }

    pub fn export_strategy_continuation(&self)->Option<EngineContinuationV1> {
        if !self.cfg.restore_strategy_continuation {return None;}
        let mut desired:Vec<_>=self.desired.iter().map(|(&ticket,d)|DesiredContinuationV1{ticket,sl:d.sl,tp:d.tp,
            last_try:d.last_try,guard:self.continuation.desired_guards.get(&ticket).cloned()}).collect();
        let mut queued_exits:Vec<_>=self.queued_exits.iter().map(|(&ticket,e)|QueuedExitContinuationV1{ticket,target:e.target,
            deadline:e.deadline,reason:e.reason,market_at_decision:e.market_at_decision,
            guard:self.continuation.exit_guards.get(&ticket).cloned()}).collect();
        desired.sort_by_key(|d|d.ticket);queued_exits.sort_by_key(|e|e.ticket);
        Some(EngineContinuationV1{schema_version:1,scope:self.continuation.scope.clone(),owner:self.continuation.owner.clone(),
            session_offset_ms:self.cfg.session_offset(),settings_contract_json:self.continuation.settings_contract_json.clone(),day_stop:self.day_stop,desired,queued_exits,
            review:self.continuation.review.clone()})
    }

    pub fn import_strategy_continuation<B:Broker>(&mut self,b:&B,owner:&str,
        snapshot:Option<&EngineContinuationV1>,origin:ContinuationOrigin)->ContinuationImportReport {
        let mut result=ContinuationImportReport{imported_stops:0,imported_exits:0,review:None};
        if !self.cfg.restore_strategy_continuation {return result;}
        if self.continuation.ready || owner.is_empty() {
            self.hold_strategy_continuation(ContinuationReviewScope::Account,"restore into initialized engine or missing owner is not permitted");
            result.review=self.continuation.review.clone();return result;
        }
        let session=b.execution_session();
        let Some(session)=session.filter(|s|!s.scope.is_empty()) else {
            self.hold_strategy_continuation(ContinuationReviewScope::Account,"no verified broker account scope");
            result.review=self.continuation.review.clone();return result;
        };
        self.continuation.scope=Some(session.scope.clone());self.continuation.owner=owner.into();
        if origin==ContinuationOrigin::Fresh {
            if snapshot.is_some() || !self.baskets.is_empty() || !b.positions().is_empty() || !b.pendings().is_empty() {
                self.hold_strategy_continuation(ContinuationReviewScope::Account,"Fresh contradicted by history or broker exposure");
            } else if b.receipt_barrier()!=ReceiptBarrier::Clear {
                self.hold_strategy_continuation(ContinuationReviewScope::Account,"Fresh has unresolved receipts");
            } else {self.continuation.ready=true;self.continuation.settings_contract_json=self.continuation_settings_contract();}
            result.review=self.continuation.review.clone();return result;
        }
        let Some(s)=snapshot else {
            self.hold_strategy_continuation(ContinuationReviewScope::Account,"restore continuation is missing; not a fresh start");
            result.review=self.continuation.review.clone();return result;
        };
        if s.schema_version!=1 || s.scope.as_deref()!=Some(session.scope.as_str()) || s.owner!=owner
            || s.session_offset_ms!=self.cfg.session_offset()
            || (s.day_stop!=i64::MIN && (b.quote().ts<=0 || s.day_stop>day_of(b.quote().ts,self.cfg.session_offset()))) {
            self.hold_strategy_continuation(ContinuationReviewScope::Account,"continuation schema/scope/owner/day-clock mismatch");
            result.review=self.continuation.review.clone();return result;
        }
        self.continuation.ready=true;
        self.continuation.settings_contract_json=s.settings_contract_json.clone();
        // A proved prior latch cannot be reset merely because an intent fails.
        self.day_stop=s.day_stop;
        if let Some(r)=&s.review {self.hold_strategy_continuation(r.scope,r.reason.clone());}
        if origin==ContinuationOrigin::UnverifiedDisk {
            self.hold_strategy_continuation(ContinuationReviewScope::Account,"cold-disk projections have no atomic generation proof (stage A)");
        }
        if s.settings_contract_json.is_empty() || s.settings_contract_json!=self.continuation_settings_contract() {
            self.hold_strategy_continuation(ContinuationReviewScope::Engine,"effective Settings contract mismatch; previous day stop retained");
            result.review=self.continuation.review.clone();return result;
        }
        for d in &s.desired {
            if d.guard.as_ref().is_some_and(|g|s.desired.iter().filter(|other|
                other.guard.as_ref().is_some_and(|x|x.position_identifier==g.position_identifier)).count()!=1) {
                self.hold_strategy_continuation(ContinuationReviewScope::Engine,"duplicate SL/TP stable identity in snapshot");continue;
            }
            let proof=d.guard.as_ref().ok_or_else(||"missing stable identity/revision".to_owned())
                .and_then(|g|{let p=self.prove_continuation_position(b,g)?;let(sl,tp)=self.confirmed_stops(&p,g,d.sl,d.tp)?;Ok((p.ticket,sl,tp,g.clone()))});
            match proof {
                Ok((t,sl,tp,g))=>{
                    if self.desired.contains_key(&t) {self.hold_strategy_continuation(ContinuationReviewScope::Engine,"duplicate SL/TP continuation identity");continue;}
                    self.desired.insert(t,DesiredStops{sl,tp,last_try:d.last_try});self.continuation.desired_guards.insert(t,g);result.imported_stops+=1;
                },
                Err(e)=>self.hold_strategy_continuation(ContinuationReviewScope::Engine,format!("SL/TP import #{}: {e}",d.ticket)),
            }
        }
        // Cold-disk stage A restores verified protection only, not discretion.
        if origin==ContinuationOrigin::Memory {for e in &s.queued_exits {
            if e.guard.as_ref().is_some_and(|g|s.queued_exits.iter().filter(|other|
                other.guard.as_ref().is_some_and(|x|x.position_identifier==g.position_identifier)).count()!=1) {
                self.hold_strategy_continuation(ContinuationReviewScope::Engine,"duplicate exit stable identity in snapshot");continue;
            }
            let proof=e.guard.as_ref().ok_or_else(||"missing stable identity/revision".to_owned())
                .and_then(|g|{let p=self.prove_continuation_position(b,g)?;
                    if !e.target.is_finite()||e.target<=0.0||!e.market_at_decision.is_finite()||e.deadline<0 {return Err("invalid exit fields".into());}
                    Ok((p.ticket,g.clone()))});
            match proof {
                Ok((t,g))=>{
                    if self.queued_exits.contains_key(&t) {self.hold_strategy_continuation(ContinuationReviewScope::Engine,"duplicate exit continuation identity");continue;}
                    self.queued_exits.insert(t,QueuedExit{target:e.target,deadline:e.deadline,reason:e.reason,market_at_decision:e.market_at_decision});
                    self.continuation.exit_guards.insert(t,g);result.imported_exits+=1;
                },
                Err(error)=>self.hold_strategy_continuation(ContinuationReviewScope::Engine,format!("exit import #{}: {error}",e.ticket)),
            }
        }}
        if b.execution_session()!=Some(session) {
            self.hold_strategy_continuation(ContinuationReviewScope::Account,"broker session changed during continuation import");
        }
        result.review=self.continuation.review.clone();result
    }
}
