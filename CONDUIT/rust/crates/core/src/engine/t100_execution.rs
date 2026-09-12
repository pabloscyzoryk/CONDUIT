//! Execution adapter for the autonomous policy. No SDK, clock or I/O here.
use super::*;
use crate::recorded_broker::exact::{decode, encode, Exact};
use crate::t100::{Config, EntryPlan, ExecutionOutcome, Intent, PortfolioView, REVISION};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingOpen {
    plan: EntryPlan,
    basket: u32,
    proof: Option<UnconfirmedOpen>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExecutionState {
    revision: String,
    config: Config,
    pending: Option<PendingOpen>,
    review: Option<String>,
    last_ts: Ts,
    mutation: u64,
}
impl ExecutionState {
    pub(super) fn new(config: &Config) -> Self {
        Self { revision: REVISION.into(), config: config.clone(), pending: None, review: None, last_ts: 0, mutation: 0 }
    }
}

/// Exact float encoding is shared with forensic replay; JSON cannot round the
/// candle or risk state when a live process restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct T100Checkpoint {
    pub revision: String,
    next_basket_id: u32,
    created_baskets_count: u32,
    runtime: Exact,
    execution: Exact,
}

impl Engine {
    /// Explicit settings apply on a confirmed flat account. Preserve all past
    /// learning, observations and risk anchors; this is never a fresh reset.
    pub fn apply_t100_configuration(&mut self, next: &Config, broker_flat: bool) -> Result<(), &'static str> {
        if *next == self.cfg.t100 { return Ok(()); }
        if !next.valid() && next.enabled {return Err("T-100 configuration is invalid");}
        if !broker_flat || self.baskets.iter().any(|b|b.alive())
            || self.rearm_entry_hold_reason().is_some() || self.continuation_entry_blocked()
            || self.t100_execution.pending.is_some() || self.t100_execution.review.is_some() {
            return Err("T-100 configuration change requires confirmed flat account and reconciled decision state");
        }
        self.cfg.t100 = next.clone();
        self.t100_execution.config = next.clone();
        self.t100_execution.mutation = self.t100_execution.mutation.wrapping_add(1);
        Ok(())
    }
    pub fn t100_checkpoint(&self) -> Option<T100Checkpoint> {
        (self.cfg.t100.enabled || self.t100_execution.mutation > 0).then(|| T100Checkpoint {
            revision: REVISION.into(),
            next_basket_id: self.next_basket_id,
            created_baskets_count: self.created_baskets_count,
            runtime: encode(&self.t100).expect("T100 runtime is serializable"),
            execution: encode(&self.t100_execution).expect("T100 execution is serializable"),
        })
    }

    /// None is a missing checkpoint, not evidence of a fresh autonomous state.
    /// A deliberately fresh Engine is initialized by Engine::new instead.
    pub fn restore_t100_checkpoint(&mut self, saved: Option<&T100Checkpoint>) -> Result<(), String> {
        if !self.cfg.t100.enabled && saved.is_none() { return Ok(()); }
        let parsed: Result<(crate::t100::Runtime, ExecutionState, u32, u32), String> = (|| {
            let saved = saved.ok_or("missing T100 checkpoint")?;
            if saved.revision != REVISION { return Err("unsupported T100 checkpoint revision".into()); }
            let runtime: crate::t100::Runtime = decode(&saved.runtime).map_err(|e| format!("T100 runtime: {e}"))?;
            let execution: ExecutionState = decode(&saved.execution).map_err(|e| format!("T100 execution: {e}"))?;
            if !runtime.valid_state() || execution.revision != REVISION || execution.config != self.cfg.t100 {
                return Err("T100 checkpoint configuration differs".into());
            }
            Ok((runtime, execution, saved.next_basket_id, saved.created_baskets_count))
        })();
        match parsed {
            Ok((runtime, execution, next, count)) => {
                self.t100 = runtime; self.t100_execution = execution;
                self.next_basket_id = self.next_basket_id.max(next);
                self.created_baskets_count = self.created_baskets_count.max(count);
                Ok(())
            }
            Err(reason) => { self.t100_hold(&reason); Err(reason) }
        }
    }

    pub(super) fn t100_hold(&mut self, reason: &str) {
        if self.t100_execution.review.is_none() {
            self.t100_execution.mutation = self.t100_execution.mutation.wrapping_add(1);
            self.t100_execution.review = Some(reason.into());
            self.log(self.t100_execution.last_ts, 2, format!("T-100 REVIEW: {reason}; new exposure blocked, protection remains active"));
        }
    }

    pub fn t100_memory_revision(&self) -> Option<u64> {
        (self.cfg.t100.enabled || self.t100_execution.mutation > 0).then_some(self.t100_execution.mutation)
    }

    pub(super) fn t100_context(&mut self, m: &IncomingMessage, signals: &[Signal]) {
        if !self.tryb_auto_ea { return; }
        self.t100_execution.last_ts = m.ts;
        self.t100_execution.mutation = self.t100_execution.mutation.wrapping_add(1);
        self.t100.observe_context(m, signals);
    }

    pub(super) fn t100_closed(&mut self, c: &ClosedTrade) {
        self.t100_execution.mutation = self.t100_execution.mutation.wrapping_add(1);
        if let Some(net) = c.net_profit() { self.t100.on_closed(c, net); }
        else { self.t100_hold("closed receipt has no validated net basis"); }
    }

    pub fn t100_entry_hold_reason(&self) -> Option<&'static str> {
        if !self.cfg.t100.enabled && self.t100_execution.pending.is_none() { return None; }
        if self.cfg.t100.enabled && !self.tryb_auto_ea {
            return Some("T-100 HOLD: AUTO-EA mode is required; AUTO cannot use this preset");
        }
        if self.t100_execution.review.is_some()
            || !self.cfg.t100.valid()
            || self.t100.revision != REVISION
            || self.t100_execution.revision != REVISION
            || self.t100_execution.config != self.cfg.t100 {
            Some("T-100 REVIEW: incomplete decision state; restore the matching checkpoint before new exposure")
        } else if self.t100_execution.pending.is_some() {
            Some("T-100 HOLD: submitted entry awaits exact broker confirmation")
        } else { None }
    }

    pub(super) fn t100_reconcile<B: Broker>(&mut self, b: &B) {
        self.t100_execution.last_ts = b.quote().ts;
        let Some(pending) = self.t100_execution.pending.clone() else { return; };
        let Some(proof) = pending.proof.as_ref() else { return; };
        if b.receipt_barrier() != ReceiptBarrier::Clear { return; }
        let Some(ticket) = b.confirmed_open(proof) else { return; };
        if self.t100_confirm(b, &pending.plan, pending.basket, ticket) {
            self.t100_execution.pending = None;
        }
    }

    fn t100_confirm<B: Broker>(&mut self, b: &B, plan: &EntryPlan, basket: u32, ticket: Ticket) -> bool {
        let Some(position) = b.positions().iter().find(|p| p.ticket == ticket
            && p.basket == Some(basket) && p.side == plan.side) else {
            self.t100_hold("acknowledged entry has no matching position snapshot");
            return false;
        };
        if !position.volume.is_finite() || position.volume <= 0.0
            || !position.open_price.is_finite() || position.open_price <= 0.0 {
            self.t100_hold("confirmed entry geometry is invalid"); return false;
        }
        if self.baskets.iter().any(|b| b.id == basket) {
            self.t100_hold("autonomous basket identity is already occupied"); return false;
        }
        let mut actual = plan.clone();
        actual.volume = position.volume;
        actual.entry_reference = position.open_price;
        actual.sl = position.sl.unwrap_or(plan.sl);
        actual.tp = position.tp.unwrap_or(plan.tp);
        actual.risk_usd = ((actual.entry_reference - actual.sl) * actual.side.sign()).max(0.0)
            * XAU_CONTRACT * actual.volume;
        let bk = Basket {
            warstwy_offset: None, id: basket, source: SourceKey::new(0, None),
            source_name: "T-100".into(), msg_id: 0, pending_exit: None,
            pending_relot_review: Vec::new(), entry_edit_state: None,
            msg_aliases: Vec::new(), persisted_done_actions: Vec::new(), side: actual.side,
            is_limit: false, is_stop: false, entry_lo: actual.entry_reference,
            entry_hi: actual.entry_reference, zone_lo: actual.entry_reference, zone_hi: actual.entry_reference,
            sl: Some(actual.sl), tps: vec![actual.tp], tp_stage: 0, plan_wykonany_do: 0,
            created_ts: position.open_ts, drop_po_ts: 0, state: BasketState::Working,
            tickets: vec![ticket], pendings: Vec::new(), realized: 0.0, events: Vec::new(),
            levels: Vec::new(), reentries: 0, last_entry_px: Some(actual.entry_reference),
            secured: false, rearm_blocked_by_spp: false, secured_by_rule: false,
            had_positions: true, rearms: 0, last_rearm_ts: 0, wol_pierwotny: vec![(ticket,actual.volume)],
            tp_touch_ts: Vec::new(), tp_touch_px: Vec::new(), sl_touch_ts: 0, sl_touch_px: 0.0,
            adverse_since: 0, age_limit_min: 0.0, last_tp_ts: 0, tempo_fast: false,
            tempo_checked: false, pyramided: false, fast_addons: 0, last_addon_ts: 0,
            peak_pl_usd: 0.0, risk_initial_usd: actual.risk_usd, secured_ts: 0,
            zone_touched: true, tp_open: false, be_ts: 0, drop_armed: false,
        };
        self.baskets.push(bk);
        self.t100_execution.mutation = self.t100_execution.mutation.wrapping_add(1);
        self.created_baskets_count += 1;
        self.refresh_basket_slots();
        self.t100.on_open_result(&actual, ExecutionOutcome::Confirmed, Some(ticket));
        self.log(position.open_ts, 1, format!("T-100 ENTRY: decision {} confirmed in basket {}", plan.decision_id, basket));
        true
    }

    pub(super) fn t100_tick<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        self.t100_execution.last_ts = q.ts;
        if !self.tryb_auto_ea { self.t100.diagnostics.last_reason = "requires_auto_ea".into(); return; }
        if !b.t100_contract_supported() { self.t100_hold("unsupported broker contract or account currency"); }
        if !self.cfg.t100.valid() { self.t100_hold("invalid T100 configuration"); }
        if self.t100_execution.config != self.cfg.t100 { self.t100_hold("configuration changed without matching decision state"); }
        let offset = self.cfg.server_tz_offset_ms;
        if offset % 3_600_000 != 0 || !(-14..=14).contains(&(offset / 3_600_000)) {
            self.t100_hold("broker UTC offset is not an explicit supported whole hour");
        }
        let own: Vec<_> = b.positions().iter().filter(|p| p.basket.is_some_and(|id|
            self.basket(id).is_some_and(|bk| bk.source_name == "T-100"))).cloned().collect();
        let foreign = b.positions().iter().filter(|p| !own.iter().any(|o| o.ticket == p.ticket))
            .chain(b.ukryte_pozycje());
        let other_risk_usd = foreign.map(|p| downside(p.side, q.exit(p.side), p.sl, p.volume))
            .chain(b.pendings().iter().chain(b.ukryte_zlecenia()).map(|p| downside(p.kind.side(),p.price,p.sl,p.volume))).sum();
        let account = b.account();
        let view = PortfolioView {
            account: &account, positions: &own,
            entry_allowed: self.t100_entry_hold_reason().is_none() && !self.wygaszanie
                && !self.wejscie_zablokowane(b, q.ts) && b.receipt_barrier() == ReceiptBarrier::Clear,
            other_risk_usd, lot_min: b.volume_min(), lot_step: b.volume_step(), lot_max: b.volume_max(),
            lot_cap: self.cfg.lot_max, stops_level: b.stops_level(),
            broker_utc_offset_hours: (offset / 3_600_000) as i32,
            completed_bars: b.complete_m1_bars(self.t100.market.completed_bars_after()),
        };
        let plans = self.t100.on_tick(&self.cfg.t100, q, view);
        for intent in plans { self.t100_execute(b, q, intent); }
        for bk in &mut self.baskets {
            if bk.source_name == "T-100" && bk.tickets.is_empty() && bk.pendings.is_empty()
                && bk.pending_exit.is_none() { bk.state = BasketState::Done; }
        }
    }

    fn t100_execute<B: Broker>(&mut self, b: &mut B, q: &Quote, intent: Intent) {
        match intent {
            Intent::Modify {ticket, sl, tp} => {
                if b.positions().iter().any(|p| p.ticket == ticket && !p.frozen) {
                    self.try_modify(b, ticket, Some(sl), tp, q.ts);
                }
            }
            Intent::Close {ticket, reason} => {
                if b.positions().iter().any(|p| p.ticket == ticket && !p.frozen) {
                    let _ = b.close_position(ticket, reason);
                }
            }
            Intent::Open(mut plan) => {
                if !b.t100_contract_supported() { self.t100_hold("unsupported broker contract or account currency"); }
                if self.t100_entry_hold_reason().is_some() || self.wygaszanie
                    || self.wejscie_zablokowane(b,q.ts) || b.receipt_barrier()!=ReceiptBarrier::Clear {
                    self.t100.on_open_result(&plan,ExecutionOutcome::Rejected,None); return;
                }
                plan.sl=b.normalize_order_price(plan.sl); plan.tp=b.normalize_order_price(plan.tp);
                if !sl_is_valid(plan.side,plan.sl,q,b.stops_level())
                    || !plan.tp.is_finite() || (plan.tp-q.entry(plan.side))*plan.side.sign()<=b.stops_level() {
                    self.t100.on_open_result(&plan,ExecutionOutcome::Rejected,None); return;
                }
                // Preserve the policy's approved USD budget after broker price
                // rounding. Volume is floored and never lifted to the minimum.
                let risk_per_lot=(q.entry(plan.side)-plan.sl)*plan.side.sign()*XAU_CONTRACT;
                let spec=crate::volume_contract::VolumeSpec{minimum:b.volume_min(),step:b.volume_step(),maximum:b.volume_max()};
                let risk_lot=plan.approved_budget_usd/risk_per_lot;
                let limits=crate::volume_contract::StrategyVolumeLimits{minimum:self.cfg.lot_min,
                    maximum:self.cfg.lot_max,capital_per_lot:self.cfg.lot_max_z_salda,capital:self.podstawa_lota()};
                let volume=crate::volume_contract::normalize_open_volume(plan.volume.min(risk_lot),spec,limits);
                if !plan.approved_budget_usd.is_finite() || plan.approved_budget_usd<=0.0
                    || !plan.volume.is_finite() || plan.volume<=0.0
                    || !plan.risk_usd.is_finite() || plan.risk_usd<=0.0
                    || !risk_per_lot.is_finite() || risk_per_lot<=0.0 || volume.is_err() {
                    self.t100.on_open_result(&plan,ExecutionOutcome::Rejected,None);return;
                }
                plan.volume=volume.unwrap();
                plan.risk_usd=risk_per_lot*plan.volume;
                let basket=self.next_basket_id;
                let Some(next)=basket.checked_add(1) else {self.t100_hold("basket sequence exhausted");return;};
                if crate::wielosilnik::slot_koszyka(next)!=self.slot {self.t100_hold("basket slot exhausted");return;}
                self.next_basket_id=next;
                self.t100_execution.mutation = self.t100_execution.mutation.wrapping_add(1);
                let request=OrderReq{side:plan.side,volume:plan.volume,sl:Some(plan.sl),tp:Some(plan.tp),
                    basket:Some(basket),level:0,is_toucher:false,comment:"T100".into()};
                let before=self.order_submission_sequence;
                match self.open_market_order(b,request) {
                    Ok(ticket)=>{self.t100_confirm(b,&plan,basket,ticket);}
                    Err(_)=>{
                        let submitted=self.order_submission_sequence!=before;
                        let proof=if submitted {b.unconfirmed_open()}else{None};
                        let uncertain=submitted && (proof.is_some() || b.receipt_barrier()!=ReceiptBarrier::Clear);
                        if uncertain {
                            self.t100.on_open_result(&plan,ExecutionOutcome::Uncertain,None);
                            if proof.is_none(){self.t100_hold("submitted entry has no exact confirmation identity");}
                            self.t100_execution.pending=Some(PendingOpen{plan,basket,proof});
                        } else {self.t100.on_open_result(&plan,ExecutionOutcome::Rejected,None);}
                    }
                }
            }
        }
    }
}

fn downside(side:Side, entry:f64, sl:Option<f64>, volume:f64)->f64 {
    match sl {
        Some(stop) if entry.is_finite() && entry>0.0 && stop.is_finite() && stop>0.0
            && volume.is_finite() && volume>0.0 => ((entry-stop)*side.sign()).max(0.0)*XAU_CONTRACT*volume,
        _=>f64::INFINITY,
    }
}

#[cfg(test)]
#[path = "t100_execution_tests.rs"]
mod tests;
