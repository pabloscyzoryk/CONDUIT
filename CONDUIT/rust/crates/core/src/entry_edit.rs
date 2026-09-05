//! Opt-in source-aware edits. Review is a per-basket persisted HOLD, not an
//! automatic retry queue or a claim of atomic broker/consumer checkpointing.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryEditOutcome { LegacyHandled, NoOp, Applied, Rejected, RequiresReview }

impl EntryEditOutcome {
    pub(super) fn source_handled(self)->bool {
        matches!(self,Self::LegacyHandled|Self::NoOp|Self::Applied)
    }
}

pub(super) fn same_source(a:&EntrySignal,b:&EntrySignal)->bool {
    a.side==b.side && a.is_limit==b.is_limit && a.is_stop==b.is_stop
        && a.lo==b.lo && a.hi==b.hi && a.sl==b.sl && a.tps==b.tps
        && a.tp_open==b.tp_open && a.warstwy_offset==b.warstwy_offset
}
fn valid_source(e:&EntrySignal)->bool {
    let px=|v:f64|v.is_finite()&&v>0.0;
    px(e.lo)&&px(e.hi)&&e.lo<=e.hi&&e.sl.is_none_or(px)
        &&e.tps.iter().all(|v|px(*v))
        &&e.warstwy_offset.is_none_or(|v|v.is_finite()&&v>=0.0)
        &&!(e.is_limit&&e.is_stop)
}
fn protect(side:Side,current:Option<Px>,wanted:Option<Px>)->Option<Px> {
    match (current,wanted) {
        (Some(a),Some(b))=>Some(if side==Side::Buy {a.max(b)}else{a.min(b)}),
        (Some(a),None)=>Some(a), (_,b)=>b,
    }
}
fn same_price(a:Option<Px>,b:Option<Px>)->bool {
    match(a,b){(Some(a),Some(b))=>(a-b).abs()<1e-8,(None,None)=>true,_=>false}
}
fn stage_plan(bk:&mut Basket,candidate:&Basket,levels:Vec<GridLevel>) {
    bk.is_limit=candidate.is_limit;bk.is_stop=candidate.is_stop;
    bk.entry_lo=candidate.entry_lo;bk.entry_hi=candidate.entry_hi;
    bk.zone_lo=candidate.zone_lo;bk.zone_hi=candidate.zone_hi;
    bk.sl=candidate.sl;bk.tps=candidate.tps.clone();
    bk.tp_open=candidate.tp_open;bk.warstwy_offset=candidate.warstwy_offset;
    bk.levels=levels;
}

impl Engine {
    pub(super) fn entry_edit_blocks(&self,id:Option<u32>)->bool {
        id.and_then(|id|self.basket(id)).is_some_and(|bk| {
            // A rollback toggle cannot erase an already persisted uncertainty.
            self.pending_source_cancelled(bk.id)
                || bk.entry_edit_state.as_ref().is_some_and(|s|s.review.is_some())
                || (self.cfg.entry_edit_geometry_v2 && bk.entry_edit_state.as_ref()
                    .is_none_or(|s|s.schema_version!=1||s.source.is_none()))
        })
    }

    pub(super) fn edit_review(&mut self,id:u32,e:&EntrySignal,ts:Ts,reason:&str)->EntryEditOutcome {
        if let Some(bk)=self.basket_mut(id) {
            let state=bk.entry_edit_state.get_or_insert_with(||Box::new(EntryEditState {
                schema_version:1,revision:0,source:None,applied_ts:0,cancelled_by_source_ts:None,review:None,
            }));
            state.review=Some(EntryEditReview{desired_source:e.clone(),received_ts:ts,reason:reason.into()});
        }
        *self.odrzuty.entry(format!("EntryEditV2::{reason}")).or_insert(0)+=1;
        self.basket_note(id,ts,format!("ENTRY EDIT V2 RequiresReview: {reason}; only this basket's new risk is blocked"));
        EntryEditOutcome::RequiresReview
    }

    fn edit_commit(&mut self,id:u32,e:&EntrySignal,ts:Ts,candidate:&Basket,levels:Vec<GridLevel>) {
        if let Some(bk)=self.basket_mut(id) {
            stage_plan(bk,candidate,levels);
            let state=bk.entry_edit_state.as_mut().expect("validated source snapshot");
            state.revision=state.revision.saturating_add(1);
            state.source=Some(e.clone());state.applied_ts=ts;state.review=None;
        }
        self.basket_note(id,ts,"ENTRY EDIT V2 Applied: source revision and confirmed plan updated".into());
    }

    fn edit_plan<B:Broker>(&mut self,b:&B,candidate:&Basket,ts:Ts)->Result<Vec<GridLevel>,&'static str> {
        if ![candidate.zone_lo,candidate.zone_hi,self.stats.balance,self.stats.equity,
            self.podstawa_lota(),self.cfg.grid_step(),self.cfg.entry_depth_curve]
            .into_iter().all(f64::is_finite) {return Err("InvalidPlanInput");}
        let budget=self.relot_portfolio_budget(b,candidate.id);
        let ea_cap=self.ea_sufit_jednostek(b);
        let (mut plan,_,_)=self.plan_grid_for(candidate,ts,budget,ea_cap);
        for gl in &mut plan {
            if !gl.price.is_finite()||gl.price<=0.0||!gl.volume.is_finite()||gl.volume<=0.0
                ||gl.sl.is_some_and(|v|!v.is_finite()||v<=0.0)
                ||gl.tp.is_some_and(|v|!v.is_finite()||v<=0.0) {return Err("InvalidPlanOutput");}
            // C never invokes broker-specific silent rounding for a new plan.
            gl.volume=crate::volume_contract::normalize_open_volume(gl.volume,
                crate::volume_contract::VolumeSpec{minimum:b.volume_min(),step:b.volume_step(),maximum:b.volume_max()},
                self.volume_limits()).map_err(|_|"InvalidPlanVolume")?;
        }
        Ok(plan)
    }

    pub(super) fn apply_entry_edit_v2<B:Broker>(&mut self,b:&mut B,id:u32,e:&EntrySignal,ts:Ts)->EntryEditOutcome {
        let old=match self.basket(id){Some(bk)=>bk.clone(),None=>return EntryEditOutcome::Rejected};
        if old.pending_exit.is_some() || old.state==BasketState::Done {
            self.basket_note(id,ts,"ENTRY EDIT V2 Rejected: terminal basket or committed exit".into());
            return EntryEditOutcome::Rejected;
        }
        if e.side!=old.side || !valid_source(e) || !self.cele_spojne(e.side,&e.tps) {
            self.basket_note(id,ts,"ENTRY EDIT V2 Rejected: invalid source/direction".into());
            return EntryEditOutcome::Rejected;
        }
        let state=match old.entry_edit_state.as_ref() {
            Some(s) if s.schema_version==1 && s.source.is_some()=>s,
            _=>return self.edit_review(id,e,ts,"MissingSourceSnapshot"),
        };
        if state.review.is_some() {return self.edit_review(id,e,ts,"ExistingReviewRequiresReconciliation");}
        let source=state.source.as_ref().unwrap();
        if same_source(source,e) {
            self.basket_note(id,ts,"ENTRY EDIT V2 NoOp: canonical source unchanged; no order RPC".into());
            return EntryEditOutcome::NoOp;
        }
        if b.receipt_barrier()!=ReceiptBarrier::Clear {
            return self.edit_review(id,e,ts,"ReceiptBarrier");
        }
        if self.cost_entry_blocked(b).is_some() || !old.pending_relot_review.is_empty() {
            return self.edit_review(id,e,ts,"OtherReconciliationBarrier");
        }
        let positions:Vec<_>=b.positions().iter().filter(|p|p.basket==Some(id)).cloned().collect();
        let pending:Vec<_>=b.pendings().iter().filter(|p|p.basket==Some(id)).cloned().collect();
        if positions.iter().any(|p|p.frozen)||pending.iter().any(|p|p.frozen) {
            return self.edit_review(id,e,ts,"FrozenExposure");
        }
        let working=old.state!=BasketState::Armed||old.had_positions||!positions.is_empty()
            ||old.levels.iter().any(|g|g.filled||g.fill_ts!=0)||old.realized!=0.0;
        let zone_changed=source.lo!=e.lo||source.hi!=e.hi;
        let source_geometry=zone_changed||source.is_limit!=e.is_limit||source.is_stop!=e.is_stop
            ||(source.warstwy_offset!=e.warstwy_offset&&self.cfg.entry_warstwy_z_tekstu);
        let sl_changed=source.sl!=e.sl||zone_changed;
        let tp_changed=source.tps!=e.tps||source.tp_open!=e.tp_open;
        if (source.sl.is_some()&&e.sl.is_none()) || (tp_changed&&e.tps.is_empty()) {
            return self.edit_review(id,e,ts,"RemovedProtectionNeedsExplicitCommand");
        }
        if working && source_geometry {return self.edit_review(id,e,ts,"WorkingGeometryNeedsFillRevisionProof");}
        if tp_changed && (old.tp_stage>0||old.plan_wykonany_do>0||old.tp_touch_ts.iter().any(|t|*t!=0)) {
            return self.edit_review(id,e,ts,"ObservedTargetsNeedProgressMigration");
        }
        if self.cfg.virtual_sl_all && sl_changed {
            return self.edit_review(id,e,ts,"VirtualStopNeedsDualConfirmation");
        }
        let mut candidate=old.clone();
        candidate.is_limit=e.is_limit;candidate.is_stop=e.is_stop;
        candidate.entry_lo=e.lo;candidate.entry_hi=e.hi;
        candidate.tp_open=e.tp_open;candidate.warstwy_offset=e.warstwy_offset;
        if zone_changed {(candidate.zone_lo,candidate.zone_hi)=self.compute_zone(e);}
        if sl_changed {
            let next=self.compute_sl(e,candidate.zone_lo,candidate.zone_hi,ts);
            candidate.sl=if working {protect(old.side,old.sl,next)}else{next};
        }
        if tp_changed {candidate.tps=self.cele_z_runnerem_od(e.side,&e.tps,Some(b.quote().mid()));}
        if !source_geometry && !sl_changed && !tp_changed {
            // An inactive text-offset/tag change cannot touch execution. In
            // particular an add-on outside the original grid needs no lookup.
            self.edit_commit(id,e,ts,&candidate,old.levels.clone());
            return EntryEditOutcome::Applied;
        }
        let fresh=match self.edit_plan(b,&candidate,ts){Ok(p)=>p,Err(r)=>return self.edit_review(id,e,ts,r)};
        let same_grid=!source_geometry && fresh.len()==old.levels.len()
            &&old.levels.iter().all(|g|fresh.iter().any(|n|n.level==g.level&&(n.price-g.price).abs()<1e-8&&n.is_toucher==g.is_toucher));
        if !same_grid {return self.edit_replace_armed(b,id,e,ts,&old,candidate,working);}
        if sl_changed && pending.iter().any(|p|fresh.iter().find(|g|g.level==p.level)
            .is_some_and(|g|g.volume*g.base_units.max(1) as f64+1e-8
                < pending.iter().filter(|x|x.level==p.level).map(|x|x.volume).sum::<f64>())) {
            return self.edit_review(id,e,ts,"StopEditNeedsVolumeReductionProof");
        }
        let mut levels=old.levels.clone(); // retain fill history, quantities and prices
        for g in &mut levels {
            let n=fresh.iter().find(|n|n.level==g.level).unwrap();
            if sl_changed {g.sl=if working {protect(old.side,g.sl,n.sl)}else{n.sl};}
            if tp_changed {g.tp=n.tp;}
        }
        // Validate ownership/assignment before the first RPC. Add-on levels not
        // represented by the original grid need an explicit TP policy.
        if positions.iter().any(|p|(sl_changed||tp_changed)&&!levels.iter().any(|g|g.level==p.level))
            ||pending.iter().any(|p|!levels.iter().any(|g|g.level==p.level)) {
            return self.edit_review(id,e,ts,"UnmappedExecutionLevel");
        }
        let session=b.execution_session();
        let pending_shape:Vec<_>=pending.iter().map(|p|(p.ticket,p.volume)).collect();
        let position_shape:Vec<_>=positions.iter().map(|p|(p.ticket,p.volume)).collect();
        for p in pending {
            let g=levels.iter().find(|g|g.level==p.level).unwrap();
            let sl=if sl_changed {self.broker_sl(g.sl,old.side)}else{p.sl};
            let tp=if tp_changed {g.tp}else{p.tp};
            if same_price(sl,p.sl)&&same_price(tp,p.tp){continue;}
            if b.receipt_barrier()!=ReceiptBarrier::Clear||b.execution_session()!=session {
                return self.edit_review(id,e,ts,"SessionOrReceiptChangedDuringModify");
            }
            if b.modify_pending(p.ticket,p.price,sl,tp).is_err()
                ||!b.pendings().iter().any(|q|q.ticket==p.ticket&&same_price(q.sl,sl)&&same_price(q.tp,tp)) {
                return self.edit_review(id,e,ts,"PendingModifyUnconfirmed");
            }
        }
        for p in positions {
            let g=levels.iter().find(|g|g.level==p.level).unwrap();
            let sl=if sl_changed {protect(old.side,p.sl,self.broker_sl(g.sl,old.side))}else{p.sl};
            let tp=if tp_changed {g.tp}else{p.tp};
            if same_price(sl,p.sl)&&same_price(tp,p.tp){continue;}
            if b.receipt_barrier()!=ReceiptBarrier::Clear||b.execution_session()!=session {
                return self.edit_review(id,e,ts,"SessionOrReceiptChangedDuringModify");
            }
            // Do not place this partial edit into the unversioned legacy retry
            // map: a later source revision could otherwise replay stale SL/TP.
            if b.modify_position(p.ticket,sl,tp).is_err()
                ||!b.find_position(p.ticket).is_some_and(|q|same_price(q.sl,sl)&&same_price(q.tp,tp)) {
                return self.edit_review(id,e,ts,"PositionModifyUnconfirmed");
            }
            self.desired.remove(&p.ticket);
        }
        let current_pending:Vec<_>=b.pendings().iter().filter(|p|p.basket==Some(id)).map(|p|(p.ticket,p.volume)).collect();
        let current_positions:Vec<_>=b.positions().iter().filter(|p|p.basket==Some(id)).map(|p|(p.ticket,p.volume)).collect();
        if pending_shape!=current_pending||position_shape!=current_positions
            ||b.receipt_barrier()!=ReceiptBarrier::Clear||b.execution_session()!=session {
            return self.edit_review(id,e,ts,"ExecutionChangedDuringModify");
        }
        self.edit_commit(id,e,ts,&candidate,levels);
        EntryEditOutcome::Applied
    }

    fn edit_replace_armed<B:Broker>(&mut self,b:&mut B,id:u32,e:&EntrySignal,ts:Ts,
        old:&Basket,candidate:Basket,working:bool)->EntryEditOutcome {
        if working {return self.edit_review(id,e,ts,"WorkingGeometryNeedsFillRevisionProof");}
        if old.tp_stage>0 || old.plan_wykonany_do>0 || old.sl_touch_ts!=0
            ||old.tp_touch_ts.iter().any(|t|*t!=0) {
            return self.edit_review(id,e,ts,"GeometryNeedsProgressMigration");
        }
        let quote=b.quote();let exit=quote.exit(old.side);
        if !quote.bid.is_finite()||!quote.ask.is_finite()||quote.bid<=0.0||quote.ask<quote.bid
            ||candidate.sl.is_some_and(|s|(exit-s)*old.side.sign()<=0.0)
            ||e.tps.first().is_some_and(|tp|(exit-tp)*old.side.sign()>=0.0) {
            return self.edit_review(id,e,ts,"GeometryAlreadyInvalidAtPrice");
        }
        // Live's cached post-cancel view currently cannot provide this proof.
        // Keep the old orders intact, rather than cancel first and guess later.
        if !b.pending_cancel_snapshot_authoritative() {
            return self.edit_review(id,e,ts,"CancelFillProofUnavailable");
        }
        let session=b.execution_session();
        let tickets:Vec<_>=b.pendings().iter().filter(|p|p.basket==Some(id)).map(|p|p.ticket).collect();
        for ticket in tickets {
            if b.receipt_barrier()!=ReceiptBarrier::Clear||b.execution_session()!=session
                ||b.positions().iter().any(|p|p.basket==Some(id)) {
                return self.edit_review(id,e,ts,"FillOrSessionChangedDuringCancel");
            }
            if b.cancel_pending(ticket).is_err()||b.pendings().iter().any(|p|p.ticket==ticket) {
                return self.edit_review(id,e,ts,"CancelUnconfirmed");
            }
            if let Some(bk)=self.basket_mut(id){bk.pendings.retain(|t|*t!=ticket);}
        }
        if !b.pending_cancel_snapshot_authoritative()||b.receipt_barrier()!=ReceiptBarrier::Clear
            ||b.execution_session()!=session||b.positions().iter().any(|p|p.basket==Some(id))
            ||b.pendings().iter().any(|p|p.basket==Some(id)) {
            return self.edit_review(id,e,ts,"PostCancelFillProofFailed");
        }
        // Recompute after cancelling; pending budget is reclaimed exactly once.
        let fresh=match self.edit_plan(b,&candidate,ts){Ok(p)=>p,Err(r)=>return self.edit_review(id,e,ts,r)};
        // Stage only the prospective geometry for synchronous sync execution;
        // do NOT claim the source revision applied until all exposure is proven.
        if let Some(bk)=self.basket_mut(id){stage_plan(bk,&candidate,fresh.clone());}
        // This is a virgin Armed plan; history must never be copied onto a new
        // geometry. Initial risk belongs to the basket's initial lifetime.
        if let Some(bk)=self.basket_mut(id){bk.zeruj_postep();}
        if fresh.is_empty() {self.edit_commit(id,e,ts,&candidate,fresh);return EntryEditOutcome::Applied;}
        self.sync_grid(b,id,ts,false);
        if b.execution_session()!=session||b.receipt_barrier()!=ReceiptBarrier::Clear
            ||!b.pending_cancel_snapshot_authoritative() {
            return self.edit_review(id,e,ts,"PostPlacementSessionOrReceiptUnproven");
        }
        // Volume equality alone cannot certify a different price/stop/target
        // or an unplanned level. Real fills need price/slippage evidence that
        // this component does not yet provide; retain a review instead.
        if b.positions().iter().any(|p|p.basket==Some(id)) {
            return self.edit_review(id,e,ts,"ReplacementFillNeedsPriceProof");
        }
        if b.pendings().iter().filter(|p|p.basket==Some(id)).any(|p| {
            !fresh.iter().any(|g|g.level==p.level &&(g.price-p.price).abs()<1e-8
                &&p.kind==if candidate.is_stop&&self.cfg.honor_stop_orders {PendingKind::stop(old.side)}else{PendingKind::limit(old.side)}
                &&same_price(p.sl,self.broker_sl(g.sl,old.side))&&same_price(p.tp,g.tp)
                &&p.is_toucher==g.is_toucher&&!p.frozen)
        }) {
            return self.edit_review(id,e,ts,"ReplacementFieldsUnconfirmed");
        }
        let complete=fresh.iter().all(|g| {
            let volume:f64=b.pendings().iter().filter(|p|p.basket==Some(id)&&p.level==g.level).map(|p|p.volume).sum::<f64>()
                +b.positions().iter().filter(|p|p.basket==Some(id)&&p.level==g.level).map(|p|p.volume).sum::<f64>();
            (volume-g.volume*self.scaled_units(g.base_units,ts) as f64).abs()<1e-8
        });
        if !complete {return self.edit_review(id,e,ts,"ReplacementIncomplete");}
        // Keep any actual fill markers written by sync_grid; never copy the
        // unfilled candidate over just-observed execution history.
        let actual_levels=self.basket(id).unwrap().levels.clone();
        self.edit_commit(id,e,ts,&candidate,actual_levels);
        EntryEditOutcome::Applied
    }
}
