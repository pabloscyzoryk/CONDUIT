//! Opt-in relot of existing pending exposure. No implicit re-entry, no replay
//! of saved order quantities. A review marker is a HOLD, never an executable queue.
use super::*;
use crate::volume_contract::{normalize_open_volume, StrategyVolumeLimits, VolumeError, VolumeSpec};

fn sum_pending<B: Broker>(b: &B, id: u32, level: i32) -> f64 {
    b.pendings().iter().filter(|p| p.basket == Some(id) && p.level == level && !p.frozen)
        .map(|p| p.volume).sum()
}
fn sum_positions<B: Broker>(b: &B, id: u32, level: i32) -> f64 {
    b.positions().iter().filter(|p| p.basket == Some(id) && p.level == level)
        .map(|p| p.volume).sum()
}
fn valid_px(x: f64) -> bool { x.is_finite() && x > 0.0 }

impl Engine {
    pub(super) fn relot_entry_requires_review(&self, id: Option<u32>, level: i32) -> bool {
        self.cfg.pending_relot_reconcile_target && id.and_then(|id|self.basket(id))
            .is_some_and(|bk|bk.pending_relot_review.iter().any(|r|r.level==level))
    }

    pub(super) fn relot_note(&mut self, id: u32, ts: Ts, code: &str) {
        *self.odrzuty.entry(format!("RelotReconcile::{code}")).or_insert(0) += 1;
        self.basket_note(id, ts, format!("RELOT RECONCILE: {code}"));
    }

    fn relot_review(&mut self, id: u32, level: i32, ts: Ts, target: f64, reason: &str) {
        if let Some(bk) = self.basket_mut(id) {
            if !bk.pending_relot_review.iter().any(|x| x.level == level) {
                bk.pending_relot_review.push(PendingRelotReview {
                    level, created_ts: ts, target_at_decision: target,
                    reason: format!("RequiresReview:{reason}"),
                });
            }
        }
        self.relot_note(id, ts, reason);
    }

    /// Replanning may reclaim only this basket's modifiable PENDING risk.
    /// Open positions, frozen orders and other baskets still consume the cap.
    pub(super) fn relot_portfolio_budget<B: Broker>(&self, b: &B, id: u32) -> Option<f64> {
        match crate::profit_budget::available(&self.cfg,(&self.stats).into(),b,Some(id)) {
            Ok(Some(v))=>return Some(v.remaining), Err(_)=>return Some(0.0), Ok(None)=>{}
        }
        if self.cfg.max_portfolio_risk_pct <= 0.0 { return None; }
        let cap = self.stats.equity.max(0.0) * self.cfg.max_portfolio_risk_pct / 100.0;
        let pos: f64 = b.positions().iter().chain(b.ukryte_pozycje())
            .filter_map(|p| p.sl.or(p.vsl).map(|s| (p.open_price-s).abs()*XAU_CONTRACT*p.volume)).sum();
        let pending: f64 = b.pendings().iter().chain(b.ukryte_zlecenia())
            .filter(|p| p.basket != Some(id) || p.frozen)
            .filter_map(|p| p.sl.map(|s| (p.price-s).abs()*XAU_CONTRACT*p.volume)).sum();
        Some((cap-pos-pending).max(0.0))
    }

    fn relot_spec<B: Broker>(&self, b: &B) -> VolumeSpec {
        VolumeSpec { minimum: b.volume_min(), step: b.volume_step(), maximum: b.volume_max() }
    }

    /// None is a COMPLETE empty plan. Err is invalid/unknown, never target zero.
    fn checked_relot_plan<B: Broker>(&self, b: &B, id: u32, ts: Ts)
        -> Result<Option<Vec<GridLevel>>, &'static str> {
        let bk = self.basket(id).ok_or("MissingBasket")?;
        if ![bk.zone_lo,bk.zone_hi,bk.entry_lo,bk.entry_hi].into_iter().all(valid_px)
            || bk.zone_lo > bk.zone_hi || bk.entry_lo > bk.entry_hi
            || bk.sl.is_some_and(|x| !valid_px(x)) || bk.tps.iter().any(|x| !valid_px(*x))
            || bk.warstwy_offset.is_some_and(|x| !x.is_finite()) {
            return Err("InvalidGeometry");
        }
        let c = &self.cfg;
        if ![self.stats.balance,self.stats.equity,self.stats.credit,self.podstawa_lota(),
            c.lot_fixed,c.lot_percent,c.lot_scale_step,c.lot_min,c.lot_max,c.lot_max_z_salda,
            c.entry_depth_curve,c.entry_warstwy_offset,c.entry_allowance_usd,c.stops_level,
            c.risk_per_basket_pct,c.max_portfolio_risk_pct,c.dd_soft_mult,c.dd_hard_mult,
            self.risk_per_basket_pct_eff(),self.dlawik_mult(),self.vol_factor(ts),c.grid_step()]
            .into_iter().all(f64::is_finite) { return Err("NonFinitePlanInput"); }
        let limits = self.volume_limits();
        StrategyVolumeLimits { capital_per_lot: 0.0, ..limits }.bounds()
            .map_err(|_| "InvalidStaticVolumeBounds")?;
        if limits.capital < 0.0 || limits.capital_per_lot < 0.0 { return Err("InvalidCapitalBounds"); }
        self.relot_spec(b).validate().map_err(|_| "UnknownBrokerVolumeSpec")?;
        if limits.capital_per_lot > 0.0 && limits.capital / limits.capital_per_lot < limits.minimum {
            return Ok(None); // valid budget, no legal lot
        }
        let budget = self.relot_portfolio_budget(b, id);
        if budget.is_some_and(|x| !x.is_finite()) { return Err("InvalidPortfolioBudget"); }
        let (plan, _empty_by_stops, _) = self.plan_grid(id, ts, budget, None);
        if plan.is_empty() { return Ok(None); }
        if plan.iter().any(|g| !valid_px(g.price) || !g.volume.is_finite() || g.volume < 0.0
            || g.sl.is_some_and(|x| !valid_px(x)) || g.tp.is_some_and(|x| !valid_px(x))) {
            return Err("InvalidPlanOutput");
        }
        for g in &plan {
            if let Some(old) = bk.levels.iter().find(|old| old.level == g.level) {
                if (old.price-g.price).abs() > 1e-8 { return Err("ChangedLevelGeometry"); }
            }
        }
        if !self.plan_ma_ten_sam_ksztalt(id, &plan) { return Err("ChangedWeightShape"); }
        Ok(Some(plan))
    }

    fn relot_legal_volume<B: Broker>(&self, b: &B, requested: f64) -> Result<f64, VolumeError> {
        normalize_open_volume(requested, self.relot_spec(b), self.volume_limits())
    }

    /// Keep the saved geometry and fill/cancel history. Only a freshly checked
    /// volume/unit budget may be replayed by sync_grid/rearm (both bool modes).
    pub(super) fn revalidate_relot_sync_levels<B: Broker>(&mut self, b: &B, id:u32,
        ts:Ts, levels:&mut Vec<GridLevel>) -> bool {
        let plan=match self.checked_relot_plan(b,id,ts) {
            Ok(Some(p))=>p,
            Ok(None)=>{self.relot_note(id,ts,"SyncPlanEmpty");return false;}
            Err(reason)=>{self.relot_note(id,ts,reason);return false;}
        };
        let mut legal=HashMap::new();
        for g in plan {
            if g.volume<=0.0 {continue;}
            if crate::lot_growth::enabled(&self.cfg) {
                // Keep the raw target: allocation needs the final market or
                // clamped pending entry and must precede broker rounding.
                legal.insert(g.level,(g.volume,g.base_units));
                continue;
            }
            match self.relot_legal_volume(b,g.volume) {
                Ok(v)=>{legal.insert(g.level,(v,g.base_units));}
                Err(VolumeError::BelowMinimum)=>{}
                Err(_)=>{self.relot_note(id,ts,"InvalidSyncVolume");return false;}
            }
        }
        levels.retain_mut(|old|match legal.get(&old.level) {
            Some((volume,units))=>{old.volume=*volume;old.base_units=*units;true}
            None=>false,
        });
        if levels.is_empty(){self.relot_note(id,ts,"SyncPlanBelowMinimum");false}else{true}
    }

    /// Ticket count is not exposure after relot consolidates bases or adds a
    /// top-up. Reserve the aggregate deficit BEFORE sync plans another order.
    pub(super) fn relot_sync_addition_budget<B: Broker>(&self,b:&B,id:u32,g:&GridLevel,
        want:usize,have:usize)->(usize,f64) {
        // Growth weights need the actual final entry price. Its target/delta
        // calculation therefore happens at the send point, after price clamps.
        if crate::lot_growth::enabled(&self.cfg) {return (want,g.volume);}
        let extra=want.saturating_sub(have);
        if extra==0 {return (want,g.volume);}
        let pending:f64=b.pendings().iter().filter(|p|p.basket==Some(id)&&p.level==g.level)
            .map(|p|p.volume).sum();
        let positioned=sum_positions(b,id,g.level);
        let remaining=(g.volume*want as f64-pending-positioned).max(0.0);
        if !pending.is_finite()||!positioned.is_finite()||!remaining.is_finite() {
            return (have,g.volume);
        }
        let per_order=g.volume.min(remaining/extra as f64);
        match self.relot_legal_volume(b,per_order) {
            Ok(v)=>(want,v),
            Err(_)=>(have,g.volume),
        }
    }

    fn relot_send<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts,
        proto: &PendingOrder, volume: f64, topup: bool) -> bool {
        if b.close_receipts_pending() || self.basket_exit_pending(id) { return false; }
        let req = PendingReq { kind: proto.kind, volume, price: proto.price, sl: proto.sl,
            tp: proto.tp, basket: Some(id), level: proto.level, is_toucher: proto.is_toucher,
            is_topup: topup,
            no_market_fallback: false, comment: proto.comment.clone() };
        match self.place_pending_order_allocated(b, req, true) {
            Ok(t) => {
                if let Some(bk) = self.basket_mut(id) { bk.pendings.push(t); }
                self.stats.relot_udane += 1; true
            }
            Err(_) => { self.stats.relot_odmowy += 1; self.relot_note(id,ts,"PlacementRejected"); false }
        }
    }

    /// Cancellation is not atomic with a fill. Only a synchronous authoritative
    /// model may prove the post-cancel delta; a live cache requires manual review.
    fn relot_cancel_replace<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts,
        p: &PendingOrder, target: f64, permit_replace: bool) -> bool {
        let before_positions = sum_positions(b,id,p.level);
        match b.cancel_pending(p.ticket) {
            Err(_) => {
                self.stats.relot_odmowy += 1;
                self.relot_note(id,ts,"CancelRejected");
                if !b.pendings().iter().any(|x| x.ticket == p.ticket) {
                    self.relot_review(id,p.level,ts,target,"CancelOutcomeUnknown");
                }
                return false;
            }
            Ok(()) => {}
        }
        if b.pendings().iter().any(|x| x.ticket == p.ticket) {
            self.relot_review(id,p.level,ts,target,"CancelAckStillPending"); return false;
        }
        if let Some(bk) = self.basket_mut(id) { bk.pendings.retain(|t| *t != p.ticket); }
        self.stats.relot_udane += 1;
        let after = sum_pending(b,id,p.level);
        let now_positions = sum_positions(b,id,p.level);
        if !after.is_finite() || !now_positions.is_finite() || now_positions+1e-12 < before_positions {
            self.relot_review(id,p.level,ts,target,"PostCancelSnapshotUnknown"); return false;
        }
        let newly_filled = (now_positions-before_positions).max(0.0);
        let remainder = (target-after-newly_filled).max(0.0);
        if remainder < b.volume_step()*0.5 { return true; }
        if !permit_replace || !b.pending_cancel_snapshot_authoritative() || b.close_receipts_pending() {
            self.relot_review(id,p.level,ts,target,"ReplacementRequiresReview"); return false;
        }
        match self.relot_legal_volume(b,remainder) {
            Ok(v) => {
                if !self.relot_send(b,id,ts,p,v,p.is_topup) {
                    self.relot_review(id,p.level,ts,target,"ReplacementRejectedRequiresReview"); return false;
                }
            }
            Err(VolumeError::BelowMinimum) => { self.relot_note(id,ts,"ResidualBelowMinimum"); }
            Err(_) => { self.relot_review(id,p.level,ts,target,"InvalidReplacement"); return false; }
        }
        true
    }

    pub(super) fn relot_pendings_reconciled<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        if !self.cfg.pending_resize_s.is_finite() { return; }
        let gap=(self.cfg.pending_resize_s.max(0.0)*1000.0) as i64;
        if ts-self.last_relot < gap { return; }
        self.last_relot=ts;
        let ids: Vec<u32> = self.baskets.iter().filter(|x| x.alive())
            .filter(|x| !self.basket_exit_pending(x.id)).map(|x|x.id).collect();
        for id in ids {
            if self.entry_edit_blocks(Some(id)) {continue;}
            let plan=match self.checked_relot_plan(b,id,ts) {
                Ok(p)=>p,
                Err(reason)=>{self.relot_note(id,ts,reason);continue;}
            };
            if plan.is_none() {self.stats.relot_plan_pusty+=1;} else {self.stats.relot_plan_ok+=1;}
            let mut levels:Vec<i32>=b.pendings().iter().filter(|p|p.basket==Some(id)&&!p.frozen).map(|p|p.level).collect();
            levels.sort_unstable();levels.dedup();
            for level in levels {
                let mut orders:Vec<PendingOrder>=b.pendings().iter()
                    .filter(|p|p.basket==Some(id)&&p.level==level&&!p.frozen).cloned().collect();
                if orders.is_empty() {continue;}
                if orders.iter().any(|p|!p.volume.is_finite()||p.volume<=0.0)
                    || b.pendings().iter().any(|p|p.basket==Some(id)&&p.level==level&&p.frozen) {
                    self.relot_note(id,ts,"FrozenOrInvalidLevel");continue;
                }
                self.stats.relot_szczebli+=1;
                let goal=plan.as_ref().and_then(|p|p.iter().find(|g|g.level==level));
                let complete_level_target=if let Some(g)=goal {
                    if g.volume<=0.0 {0.0} else {
                        let raw=if crate::lot_growth::enabled(&self.cfg) {
                            match self.growth_allocated_volume(b,Some(id),level,orders[0].kind.side(),
                                orders[0].price,orders[0].sl,g.volume,false) {
                                Ok(v)=>v,Err(_)=>{self.relot_note(id,ts,"AllocationTargetUnknown");continue;}
                            }
                        } else {g.volume};
                        match self.relot_legal_volume(b,raw) {
                            Ok(v)=>v*self.scaled_units(g.base_units,ts) as f64,
                            Err(VolumeError::BelowMinimum)=>0.0,
                            Err(_)=>{self.relot_note(id,ts,"InvalidTargetVolume");continue;}
                        }
                    }
                } else {0.0};
                // Count planned units, never surviving tickets: cancelling one
                // base must not shrink the target again on every later cycle.
                // Existing fills consume the aggregate level budget; their
                // geometry may differ, so ambiguous filled levels never grow.
                let filled_volume=sum_positions(b,id,level);
                if !filled_volume.is_finite() || filled_volume<0.0 {self.relot_note(id,ts,"InvalidFilledVolume");continue;}
                let target=(complete_level_target-filled_volume).max(0.0);
                let total: f64=orders.iter().map(|p|p.volume).sum();
                let eps=b.volume_step()*0.5;
                let delta=target-total;
                if delta.abs()<eps {continue;}
                let review=self.basket(id).is_some_and(|bk|bk.pending_relot_review.iter().any(|x|x.level==level));
                let filled=self.basket(id).is_some_and(|bk|bk.levels.iter().any(|g|g.level==level&&g.filled))
                    || sum_positions(b,id,level)>0.0;
                if delta>0.0 {
                    self.stats.relot_up_zdarzen+=1;self.stats.relot_up_lotow+=delta;
                    if !self.cfg.pending_relot_up || self.stats.balance<self.cfg.pending_relot_up_od_salda {continue;}
                    if review || filled {self.relot_note(id,ts,"FilledOrReviewLevelNoUp");continue;}
                    if b.close_receipts_pending() {self.relot_note(id,ts,"ReceiptBarrierNoUp");continue;}
                    if !self.margines_pozwala(b,self.cfg.ml_min_relot_up) {continue;}
                    if self.cfg.pending_relot_topup {
                        if let Ok(v)=self.relot_legal_volume(b,delta) {
                            self.relot_send(b,id,ts,&orders[0],v,true);
                        } else {self.relot_note(id,ts,"ResidualBelowMinimum");}
                    } else {
                        if !b.pending_cancel_snapshot_authoritative() {self.relot_note(id,ts,"NoAuthoritativeReplaceSnapshot");continue;}
                        let p=orders.iter().find(|p|!p.is_topup).unwrap_or(&orders[0]);
                        // Validate BEFORE removing a healthy order.
                        if self.relot_legal_volume(b,p.volume+delta).is_err() {self.relot_note(id,ts,"InvalidReplacement");continue;}
                        self.relot_cancel_replace(b,id,ts,p,target,true);
                    }
                } else {
                    self.stats.relot_down_zdarzen+=1;self.stats.relot_down_lotow+=-delta;
                    if !self.cfg.pending_relot_down {continue;}
                    orders.sort_by_key(|p|(!p.is_topup,p.ticket));
                    for p in orders {
                        let current=sum_pending(b,id,level);
                        if current<=target+eps {break;}
                        if !b.pendings().iter().any(|x|x.ticket==p.ticket) {continue;}
                        let would_need_replacement=current-p.volume<target-eps;
                        let permit=!review && !filled && !b.close_receipts_pending();
                        // Full cancellation needs no authority to OPEN. Partial
                        // replacement is allowed only with a synchronous proof.
                        if !self.relot_cancel_replace(b,id,ts,&p,target,
                            would_need_replacement&&permit) {break;}
                    }
                }
            }
        }
    }
}
