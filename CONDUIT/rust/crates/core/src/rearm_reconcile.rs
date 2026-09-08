//! A rearm batch is counted once, including when its OPEN is adopted later.
//! Definite refusals create no state. Unconfirmed exposure is never guessed.
use super::*;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RearmReconcileState {
    batches: Vec<RearmBatch>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testy_pakiet_a::{Atrapa, TS0, WEJSCIE, silnik, wiad};
    use std::collections::VecDeque;

    #[derive(Clone, Copy)]
    enum Reply { Ack, Pending, Refused }

    struct AckBroker {
        inner: Atrapa,
        replies: VecDeque<Reply>,
        last: Option<UnconfirmedOpen>,
        waiting: Vec<(UnconfirmedOpen, Ticket)>,
        released: bool,
        calls: usize,
        session: ExecutionSession,
    }

    impl AckBroker {
        fn new() -> Self { Self { inner: Atrapa::nowa(), replies: VecDeque::new(), last: None,
            waiting: Vec::new(), released: false, calls: 0,
            session: ExecutionSession {scope: "synthetic-account".into(), generation: 1} } }
    }

    impl Broker for AckBroker {
        fn quote(&self)->Quote {self.inner.quote()}
        fn account(&self)->Account {self.inner.account()}
        fn stops_level(&self)->f64 {self.inner.stops_level()}
        fn execution_session(&self)->Option<ExecutionSession>{Some(self.session.clone())}
        fn positions(&self)->&[Position]{self.inner.positions()}
        fn positions_mut(&mut self)->&mut Vec<Position>{self.inner.positions_mut()}
        fn pendings(&self)->&[PendingOrder]{self.inner.pendings()}
        fn pendings_mut(&mut self)->&mut Vec<PendingOrder>{self.inner.pendings_mut()}
        fn close_receipts_pending(&self)->bool{!self.waiting.is_empty() && !self.released}
        fn unconfirmed_open(&self)->Option<UnconfirmedOpen>{self.last.clone()}
        fn confirmed_open(&self, intent:&UnconfirmedOpen)->Option<Ticket>{
            if !self.released || intent.session.scope!=self.session.scope{return None;}
            self.waiting.iter().find(|(saved,t)| saved==intent && self.positions().iter().any(|p|p.ticket==*t)).map(|(_,t)|*t)
        }
        fn open_market(&mut self, mut r:OrderReq)->BResult<Ticket>{
            self.last=None;
            if self.close_receipts_pending(){return Err(BrokerError::Rejected);}
            self.calls+=1;
            let reply=self.replies.pop_front().unwrap_or(Reply::Ack);
            if matches!(reply,Reply::Refused){return Err(BrokerError::InvalidStops);}
            r.comment=format!("synthetic-ordinal-{}",self.calls);
            let intent=UnconfirmedOpen{session:self.session.clone(),side:r.side,requested_volume:r.volume,
                basket:r.basket,level:r.level,is_toucher:r.is_toucher,submitted_quote_ts:self.quote().ts,machine_comment:r.comment.clone()};
            let ticket=self.inner.open_market(r)?;
            if matches!(reply,Reply::Pending){self.last=Some(intent.clone());self.waiting.push((intent,ticket));Err(BrokerError::Rejected)}
            else{Ok(ticket)}
        }
        fn place_pending(&mut self,r:PendingReq)->BResult<Ticket>{self.last=None;self.inner.place_pending(r)}
        fn modify_position(&mut self,t:Ticket,sl:Option<Px>,tp:Option<Px>)->BResult<()>{self.inner.modify_position(t,sl,tp)}
        fn modify_pending(&mut self,t:Ticket,p:Px,sl:Option<Px>,tp:Option<Px>)->BResult<()>{self.inner.modify_pending(t,p,sl,tp)}
        fn close_position(&mut self,t:Ticket,why:CloseReason)->BResult<f64>{self.inner.close_position(t,why)}
        fn close_partial(&mut self,t:Ticket,v:f64,why:CloseReason)->BResult<f64>{self.inner.close_partial(t,v,why)}
        fn cancel_pending(&mut self,t:Ticket)->BResult<()>{self.inner.cancel_pending(t)}
        fn drain_closed(&mut self)->Vec<ClosedTrade>{self.inner.drain_closed()}
    }

    fn fixture(levels:usize,max_times:u32,gap:f64)->(Engine,AckBroker){
        let mut b=AckBroker::new();
        let mut e=silnik(|c|{c.auto_limit=true;c.entry_units=levels as u32;
            c.risk_per_basket_pct=0.0;c.max_portfolio_risk_pct=0.0;c.rearm_grid_on_return=true;
            c.rearm_block_after_secured=false;c.rearm_min_gap_min=gap;c.rearm_max_times=max_times;
            c.rearm_min_basket_profit=0.0;c.pending_cross_policy=PendingCrossPolicy::Market;
            c.fast_addon_move_usd=0.0;});
        e.on_message(&mut b,&wiad(1,1,None,WEJSCIE));
        b.positions_mut().clear();b.pendings_mut().clear();
        b.inner.ustaw_cene(TS0+60_000,3997.25,3997.45);
        let bk=&mut e.baskets[0];
        let prototype=bk.levels[0].clone();
        bk.levels=(0..levels).map(|n|{let mut g=prototype.clone();g.level=n as i32;g.price=3999.0;
            g.base_units=1;g.volume=0.01;g.filled=false;g.fill_ts=0;g}).collect();
        bk.tickets.clear();bk.pendings.clear();bk.had_positions=true;bk.realized=10.0;bk.state=BasketState::Working;
        b.calls=0;(e,b)
    }

    #[test]
    fn confirmed_adopted_rearm_obeys_count_and_cooldown_like_complete_ack(){
        for (cap,gap) in [(1,0.0),(0,15.0),(1,15.0)] {
            for reply in [Reply::Ack,Reply::Pending] {
                let (mut e,mut b)=fixture(1,cap,gap);b.replies.push_back(reply);
                let q=b.quote();e.rearm_pass(&mut b,&q);assert_eq!(b.calls,1);
                if matches!(reply,Reply::Pending){
                    assert_eq!(e.baskets[0].rearms,0);assert!(e.wejscie_zablokowane(&b,q.ts));
                    e.rearm_pass(&mut b,&q);assert_eq!(b.calls,1,"no resend while waiting");b.released=true;
                }
                b.inner.ustaw_cene(q.ts+2,q.bid,q.ask);let tick=b.quote();e.on_tick(&mut b,&tick);
                assert_eq!(e.baskets[0].rearms,1);assert_eq!(e.baskets[0].last_rearm_ts,q.ts);
                assert!(!e.rearm_confirmation_pending());
                e.reconcile_rearm_batches(&b);assert_eq!(e.baskets[0].rearms,1,"idempotent repeated snapshot");
                b.positions_mut().clear();b.inner.ustaw_cene(q.ts+1_000,q.bid,q.ask);let tick=b.quote();
                e.rearm_pass(&mut b,&tick);assert_eq!(b.calls,1,"both configured limits remain effective");
            }
        }
    }

    #[test]
    fn partial_ack_batch_is_counted_once_and_definite_refusal_never_counts(){
        for second in [Reply::Ack,Reply::Pending,Reply::Refused] {
            let (mut e,mut b)=fixture(2,0,0.0);b.replies.extend([Reply::Ack,second]);
            let q=b.quote();e.rearm_pass(&mut b,&q);assert_eq!(b.calls,2);assert_eq!(e.baskets[0].rearms,1);
            b.released=true;e.reconcile_rearm_batches(&b);assert_eq!(e.baskets[0].rearms,1);
            assert_eq!(e.baskets[0].last_rearm_ts,q.ts);assert!(!e.rearm_confirmation_pending());
        }
        let (mut e,mut b)=fixture(1,1,15.0);b.replies.push_back(Reply::Refused);
        let q=b.quote();e.rearm_pass(&mut b,&q);assert_eq!(e.baskets[0].rearms,0);
        assert_eq!(e.baskets[0].last_rearm_ts,0);assert!(!e.rearm_confirmation_pending());
        e.rearm_pass(&mut b,&q);assert_eq!(e.baskets[0].rearms,1,"a definite refusal did not consume capacity");
    }

    #[test]
    fn restart_retains_unconfirmed_rearm_and_requires_matching_scope_and_position(){
        let (mut e,mut b)=fixture(1,1,15.0);b.replies.push_back(Reply::Pending);
        let q=b.quote();e.rearm_pass(&mut b,&q);
        let state:RearmReconcileState=serde_json::from_slice(&serde_json::to_vec(&e.rearm_reconcile_state()).unwrap()).unwrap();
        let baskets=e.baskets.clone();let mut restored=Engine::new(e.cfg.clone(),400.0);
        restored.adopt_baskets(baskets);restored.restore_rearm_reconcile_state(state);
        assert!(restored.wejscie_zablokowane(&b,q.ts));
        b.released=true;b.session.scope="other-account".into();restored.reconcile_rearm_batches(&b);
        assert_eq!(restored.baskets[0].rearms,0);assert!(restored.rearm_confirmation_pending());
        b.session.scope="synthetic-account".into();b.session.generation+=1;
        let positions=std::mem::take(b.positions_mut());restored.reconcile_rearm_batches(&b);
        assert_eq!(restored.baskets[0].rearms,0,"no position is not proof of acceptance");
        *b.positions_mut()=positions;restored.reconcile_rearm_batches(&b);
        assert_eq!(restored.baskets[0].rearms,1);assert_eq!(restored.baskets[0].last_rearm_ts,q.ts);
        assert!(!restored.rearm_confirmation_pending());
    }

    #[test]
    fn persisted_batch_recognizes_exact_before_after_counters_without_double_count(){
        let (mut e,mut b)=fixture(1,1,15.0);b.replies.push_back(Reply::Pending);
        let q=b.quote();e.rearm_pass(&mut b,&q);let saved=e.rearm_reconcile_state();
        b.released=true;e.reconcile_rearm_batches(&b);assert_eq!(e.baskets[0].rearms,1);
        e.restore_rearm_reconcile_state(saved.clone());e.reconcile_rearm_batches(&b);
        assert_eq!(e.baskets[0].rearms,1,"older pending memory plus newer basket never counts twice");
        e.restore_rearm_reconcile_state(saved);e.baskets[0].rearms=7;e.reconcile_rearm_batches(&b);
        assert!(e.rearm_confirmation_pending(),"unexplained counter divergence requires review");
        assert_eq!(e.baskets[0].rearms,7);
    }

    #[test]
    fn recorded_broker_roundtrip_captures_pending_and_confirmed_rearm_identity(){
        use crate::recorded_broker::{Recorder,ReplayBroker,Trace};
        let (mut engine,mut broker)=fixture(1,1,15.0);broker.replies.push_back(Reply::Pending);
        for confirmed in [false,true] {
            broker.released=confirmed;
            let q=broker.quote();
            let bootstrap=engine.export_replay_bootstrap().unwrap();
            let mut recorder=Recorder::new(&mut broker);
            engine.on_tick(&mut recorder,&q);
            let trace=recorder.finish();drop(recorder);
            let trace:Trace=serde_json::from_slice(&serde_json::to_vec(&trace).unwrap()).unwrap();
            let mut replay_broker=ReplayBroker::new(trace).unwrap();
            let mut replay=Engine::from_replay_bootstrap(&bootstrap).unwrap();
            replay.on_tick(&mut replay_broker,&q);replay_broker.finish().unwrap();
            assert_eq!(replay.export_replay_bootstrap().unwrap(),engine.export_replay_bootstrap().unwrap());
        }
        assert_eq!(engine.baskets[0].rearms,1);assert!(!engine.rearm_confirmation_pending());
    }

    #[test]
    fn another_slot_hold_blocks_new_sends_but_not_protection_and_clears_after_proof() {
        use crate::routing::Silniki;
        let (mut owner, mut b) = fixture(1,1,15.0);
        b.replies.push_back(Reply::Pending);
        let q=b.quote();owner.rearm_pass(&mut b,&q);
        let accepted=b.positions_mut().pop().unwrap();
        b.released=true; // Transport is clear, but there is no current position proof.
        owner.reconcile_rearm_batches(&b);
        assert!(owner.rearm_entry_hold_reason().unwrap().contains("REVIEW"));
        let mut chain=crate::formaty::Lancuch::default();
        chain.presety.insert("ATFX".into(),"A".into());
        chain.presety.insert("Synergy".into(),"B".into());
        let configs=[("A".into(),owner.cfg.clone()),("B".into(),owner.cfg.clone())].into_iter().collect();
        let (mut engines, missing)=Silniki::zbuduj(&chain,&configs,&owner.cfg,400.0);
        assert!(missing.is_empty());
        let a=engines.lista.iter().position(|s|s.zapasowy).unwrap();
        let other=1-a;
        engines.lista[a].engine=owner;
        let basket=engines.lista[other].engine.next_basket_id();
        let market=OrderReq{side:Side::Buy,volume:0.01,sl:Some(3980.0),tp:Some(4010.0),
            basket:Some(basket),level:0,is_toucher:false,comment:"synthetic existing exposure".into()};
        let pending=PendingReq{kind:PendingKind::BuyLimit,volume:0.01,price:3990.0,
            sl:Some(3980.0),tp:Some(4010.0),basket:Some(basket),level:1,
            is_toucher:false,is_topup:false,comment:"synthetic existing pending".into()};
        let ticket=b.inner.open_market(market.clone()).unwrap();
        let order=b.inner.place_pending(pending.clone()).unwrap();
        let calls=b.calls;
        engines.przelicz_obce(&b,None);
        engines.z_widokiem(other,&mut b,|e,w| {
            assert!(e.obce.rearm_entry_hold);
            assert!(e.rearm_entry_hold_reason().is_none(),"foreign state cannot feed routing back");
            let bootstrap=e.export_replay_bootstrap().unwrap();
            assert!(Engine::from_replay_bootstrap(&bootstrap).unwrap().obce.rearm_entry_hold);
            let sequence=e.order_submission_sequence;
            assert!(e.open_market_order(w,market.clone()).is_err());
            assert!(e.place_pending_order(w,pending.clone()).is_err());
            assert_eq!(e.order_submission_sequence,sequence,"blocked before the Broker boundary");
            w.modify_position(ticket,Some(3981.0),Some(4011.0)).unwrap();
            w.cancel_pending(order).unwrap();
            w.close_position(ticket,CloseReason::RiskFree).unwrap();
        });
        assert_eq!(b.calls,calls);
        assert!(b.positions().is_empty() && b.pendings().is_empty());
        b.positions_mut().push(accepted);
        engines.z_widokiem(a,&mut b,|e,w| e.reconcile_rearm_batches(w));
        assert!(engines.lista[a].engine.rearm_entry_hold_reason().is_none());
        engines.z_widokiem(other,&mut b,|e,w| {
            assert!(!e.obce.rearm_entry_hold,"no sticky halt after confirmation");
            e.open_market_order(w,market).unwrap();
            e.place_pending_order(w,pending).unwrap();
        });
        assert_eq!(b.calls,calls+1);
        assert!(engines.lista.iter().all(|s|s.engine.halted.is_none()));
        assert_eq!(engines.lista[a].engine.baskets[0].rearms,1);
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RearmBatch {
    basket: u32,
    submitted_ts: Ts,
    count_before: u32,
    last_before: Ts,
    counted: bool,
    review: Option<String>,
    intents: Vec<UnconfirmedOpen>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct RearmReconcile {
    state: RearmReconcileState,
    building: Option<RearmBatch>,
    revision: u64,
}

impl Engine {
    pub fn rearm_reconcile_state(&self) -> RearmReconcileState {
        self.rearm_reconcile.state.clone()
    }

    pub fn restore_rearm_reconcile_state(&mut self, state: RearmReconcileState) {
        self.rearm_reconcile.state = state;
        self.rearm_reconcile.building = None;
        self.rearm_reconcile.revision = self.rearm_reconcile.revision.wrapping_add(1);
    }

    pub fn rearm_reconcile_revision(&self) -> u64 { self.rearm_reconcile.revision }

    /// Read-only application guard for manual new exposure. Protective
    /// close/cancel/modify operations must not be routed through this guard.
    pub fn rearm_entry_hold_reason(&self) -> Option<&'static str> {
        // Own state only: routing derives other-slot holds from this accessor.
        // Including obce here would make a cleared account hold feed itself.
        (!self.rearm_reconcile.state.batches.is_empty()).then(|| self.rearm_hold_reason())
    }

    pub(super) fn rearm_confirmation_pending(&self) -> bool {
        !self.rearm_reconcile.state.batches.is_empty() || self.obce.rearm_entry_hold
    }

    pub(super) fn rearm_hold_reason(&self) -> &'static str {
        if self.rearm_reconcile.state.batches.iter().any(|b| b.review.is_some()) {
            "REARM REVIEW: brak pełnego dowodu wysłanej intencji; wymagane uzgodnienie historii brokera i pamięci koszyka"
        } else {
            "REARM HOLD: oczekiwanie na potwierdzenie wysłanego zlecenia; licznik nie jest zgadywany"
        }
    }

    pub(super) fn begin_rearm_batch(&mut self, basket: u32, submitted_ts: Ts) {
        debug_assert!(self.rearm_reconcile.building.is_none());
        let Some(bk) = self.basket(basket) else { return };
        let count_before = bk.rearms;
        let last_before = bk.last_rearm_ts;
        self.rearm_reconcile.building = Some(RearmBatch {
            basket, submitted_ts, count_before, last_before, counted: false, review: None, intents: Vec::new(),
        });
    }

    pub(super) fn remember_rearm_unconfirmed<B: Broker>(&mut self, broker: &B, basket: u32) {
        let Some(batch) = self.rearm_reconcile.building.as_mut().filter(|b| b.basket == basket) else { return };
        let Some(intent) = broker.unconfirmed_open() else { return };
        if intent.basket != Some(basket) || intent.session.scope.is_empty()
            || intent.machine_comment.is_empty() || !intent.requested_volume.is_finite()
            || intent.requested_volume <= 0.0 { return; }
        if !batch.intents.contains(&intent) { batch.intents.push(intent); }
    }

    pub(super) fn finish_rearm_batch(&mut self, placed: usize) {
        let Some(mut batch) = self.rearm_reconcile.building.take() else { return };
        if batch.intents.is_empty() { return; }
        batch.counted = placed > 0;
        let basket = batch.basket;
        let ts = batch.submitted_ts;
        self.rearm_reconcile.state.batches.push(batch);
        self.rearm_reconcile.revision = self.rearm_reconcile.revision.wrapping_add(1);
        self.basket_note(basket, ts, "REARM HOLD: oczekiwanie na potwierdzenie wysłanego zlecenia; licznik nie jest zgadywany".into());
    }

    /// Runs before another entry can use the rearm count/cooldown. The adapter
    /// supplies the confirmation; a similar-looking cached position is not proof.
    pub(super) fn reconcile_rearm_batches<B: Broker>(&mut self, broker: &B) {
        if !self.rearm_confirmation_pending() { return; }
        let batches = std::mem::take(&mut self.rearm_reconcile.state.batches);
        let mut remaining = Vec::new();
        let mut changed = false;
        for mut batch in batches {
            let Some(slot) = self.baskets.iter().position(|b| b.id == batch.basket) else {
                if batch.review.is_none() {
                    batch.review=Some("MissingBasketMemory".into()); changed=true;
                    self.log(broker.quote().ts,2,"REARM REVIEW: brak pełnego dowodu wysłanej intencji; wymagane uzgodnienie historii brokera i pamięci koszyka");
                }
                remaining.push(batch); continue;
            };
            let basket = &self.baskets[slot];
            let expected_count = batch.count_before.saturating_add(1);
            let at_before = basket.rearms == batch.count_before && basket.last_rearm_ts == batch.last_before;
            let at_after = basket.rearms == expected_count && basket.last_rearm_ts == batch.submitted_ts;
            // Basket and risk files may be saved independently. Recognize only
            // the exact before/after states of this batch; never guess a merge.
            if !at_before && !at_after {
                if batch.review.as_deref() != Some("CounterStateMismatch") {
                    batch.review=Some("CounterStateMismatch".into()); changed=true;
                    self.log(broker.quote().ts,2,"REARM REVIEW: stan licznika nie odpowiada zapisanej próbie; wymagane uzgodnienie pamięci koszyka");
                    self.basket_note(batch.basket,broker.quote().ts,
                        "REARM REVIEW: stan licznika nie odpowiada zapisanej próbie; wymagane uzgodnienie pamięci koszyka".into());
                }
                remaining.push(batch); continue;
            }
            let mut confirmed = Vec::new();
            batch.intents.retain(|intent| {
                let ticket = broker.confirmed_open(intent).filter(|ticket| {
                    broker.positions().iter().any(|p| p.ticket == *ticket && p.basket == Some(batch.basket)
                        && p.side == intent.side && p.level == intent.level && p.is_toucher == intent.is_toucher)
                });
                if let Some(ticket) = ticket { confirmed.push(ticket); false } else { true }
            });
            if !confirmed.is_empty() {
                changed = true;
                if at_before {
                    let basket = &mut self.baskets[slot];
                    // Control path records the quote at submission, not the later
                    // receipt delivery. No retroactive order is created here.
                    basket.rearms = expected_count;
                    basket.last_rearm_ts = batch.submitted_ts;
                    batch.counted = true;
                    let count = basket.rearms;
                    self.basket_note(batch.basket, broker.quote().ts,
                        format!("REARM potwierdzony po uzgodnieniu z brokerem: przezbrojenie {count}; zachowano czas wysłania"));
                    if self.journal.wants(EventLevel::Info) {
                        self.journal.push(Ev::new(broker.quote().ts, EventLevel::Info, EventCategory::Order, EventKind::OrderPlaced)
                            .basket(batch.basket).reason(RejectCode::GridRearmed)
                            .text("REARM: potwierdzone wcześniej wysłane zlecenie; bez ponownej wysyłki")
                            .put("reconciled", true).put("rearms", count as u64)
                            .put("submitted_ts", batch.submitted_ts).put("confirmed_tickets", confirmed).build());
                    }
                }
                batch.counted = true;
            }
            if !batch.intents.is_empty() {
                if broker.receipt_barrier()==ReceiptBarrier::Clear && batch.review.is_none() {
                    batch.review=Some("MissingPositionProof".into()); changed=true;
                    self.log(broker.quote().ts,2,"REARM REVIEW: brak potwierdzonej pozycji o pełnej tożsamości wysłanego zlecenia; wymagane uzgodnienie historii brokera");
                    self.basket_note(batch.basket,broker.quote().ts,
                        "REARM REVIEW: brak potwierdzonej pozycji o pełnej tożsamości wysłanego zlecenia; wymagane uzgodnienie historii brokera".into());
                }
                remaining.push(batch);
            }
        }
        self.rearm_reconcile.state.batches = remaining;
        if changed { self.rearm_reconcile.revision = self.rearm_reconcile.revision.wrapping_add(1); }
    }
}
