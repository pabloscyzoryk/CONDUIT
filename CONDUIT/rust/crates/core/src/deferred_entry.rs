//! RAM-only, opt-in receipt-wait lifecycle. No order retries, no durable replay,
//! no interpretation of historical TP as profit of a position not yet opened.
use super::*;

type Key = (SourceKey, i64);
const MAX_ACTIVE: usize = 128;
const MAX_SESSION_RECORDS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DeferredEntryState {
    Waiting, NoEntry, Cancelled, Expired, RequiresReview, Attempted, Executed, Rejected,
}
impl DeferredEntryState {
    fn active(self) -> bool { matches!(self, Self::Waiting | Self::NoEntry) }
    /// Intencja może już nie nadawać się do automatycznego wykonania, ale do
    /// końca TTL nadal musi być obserwowana i ostatecznie zamknięta.  Dawniej
    /// `RequiresReview` wypadało z pętli na zawsze, więc pięciominutowa oś
    /// `deferred_entry_max_age_s` nie działała, a stary rekord zatruwał kolejne
    /// NEW/EDIT przez całą sesję live.
    fn awaiting_terminal(self) -> bool {
        matches!(self, Self::Waiting | Self::NoEntry | Self::RequiresReview)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeferredEntryStatus {
    pub action_id: String,
    pub state: DeferredEntryState,
    pub reason: String,
    pub first_received_utc: Ts,
    pub last_received_utc: Ts,
}

struct PendingEntry {
    status: DeferredEntryStatus,
    message: IncomingMessage,
    entry: Option<EntrySignal>,
    session: Option<ExecutionSession>,
    initial_quote_ts: Ts,
    last_quote_ts: Ts,
    sequence: u64,
    protection_done: bool,
}

#[derive(Default)]
pub(super) struct DeferredEntries {
    records: HashMap<Key, PendingEntry>,
    clock_utc: Option<Ts>,
    maximum_clock_utc: Ts,
    sequence: u64,
    releasing: Option<Key>,
}

impl DeferredEntries {
    pub(super) fn is_replay(&self, m: &IncomingMessage) -> bool {
        self.releasing.as_ref().is_some_and(|key| key.0 == m.source && key.1 == m.edit_of.unwrap_or(m.msg_id))
    }
    pub(super) fn protection_done(&self, m: &IncomingMessage) -> bool {
        self.is_replay(m) && self.records.get(&(m.source.clone(),m.edit_of.unwrap_or(m.msg_id)))
            .is_some_and(|p|p.protection_done)
    }
    pub(super) fn blocks_entry_done(&self, key: &Key) -> bool {
        self.records.get(key).is_some_and(|p| !matches!(p.status.state, DeferredEntryState::Attempted | DeferredEntryState::Executed))
    }
}

impl Engine {
    /// Explicit dispatch/receipt UTC clock. `m.ts` keeps its old broker-clock
    /// meaning; publication time must NOT be passed as `received_utc`.
    pub fn on_message_received<B: Broker>(&mut self, b: &mut B, m: &IncomingMessage, received_utc: Ts) {
        let old = self.deferred_entries.clock_utc.replace(received_utc);
        self.on_message(b, m);
        self.deferred_entries.clock_utc = old;
    }

    /// TTL uses this injected UTC clock, never the possibly UTC+3 quote epoch.
    /// A fresh price event is required to release; a wall-clock pulse can only
    /// expire/invalidate an intention, not create an order from a stale quote.
    pub fn on_tick_received<B: Broker>(&mut self, b: &mut B, q: &Quote, dispatch_utc: Ts) {
        let old = self.deferred_entries.clock_utc.replace(dispatch_utc);
        self.on_tick(b, q);
        self.deferred_release(b, q);
        self.deferred_entries.clock_utc = old;
    }

    pub fn deferred_entry_status(&self, source: &SourceKey, message_id: i64) -> Option<DeferredEntryStatus> {
        self.deferred_entries.records.get(&(source.clone(), message_id)).map(|p| p.status.clone())
    }

    fn deferred_emit(&mut self, key: &Key, ts: Ts) {
        if let Some(p) = self.deferred_entries.records.get(key) {
            let status = p.status.clone();
            self.log(ts, 1, format!("ENTRY {}: {:?} — {}", status.action_id, status.state, status.reason));
            if self.journal.wants(EventLevel::Info) {
                self.journal.push(Ev::new(ts, EventLevel::Info, EventCategory::Signal, EventKind::Note)
                    .msg(key.1).text(format!("Deferred ENTRY: {:?}: {}", status.state, status.reason))
                    .put("action", "deferred_entry").put("action_id", status.action_id)
                    .put("state", format!("{:?}", status.state)).put("reason", status.reason)
                    .put("first_received_utc", status.first_received_utc)
                    .put("last_received_utc", status.last_received_utc).build());
            }
        }
    }

    fn deferred_transition(&mut self, key: &Key, state: DeferredEntryState, reason: &str, ts: Ts) {
        if let Some(p) = self.deferred_entries.records.get_mut(key) {
            if p.status.state == state && p.status.reason == reason { return; }
            p.status.state = state;
            p.status.reason = reason.to_string();
            self.deferred_emit(key, ts);
        }
    }

    pub(super) fn deferred_observe<B: Broker>(&mut self, b: &B, q: &Quote) {
        if self.deferred_entries.records.is_empty() { return; }
        let clock = self.deferred_entries.clock_utc.filter(|t| *t > 0);
        if let Some(now) = clock {
            self.deferred_entries.maximum_clock_utc = self.deferred_entries.maximum_clock_utc.max(now);
        }
        let now = self.deferred_entries.maximum_clock_utc;
        let session = b.execution_session();
        let barrier = b.receipt_barrier();
        let max_age = self.cfg.deferred_entry_max_age_s;
        let keys: Vec<Key> = self.deferred_entries.records.iter()
            .filter(|(_, p)| p.status.state.awaiting_terminal()).map(|(k, _)| k.clone()).collect();
        for key in keys {
            let p = &self.deferred_entries.records[&key];
            // TTL jest nadrzędnym końcem życia intencji.  W szczególności
            // trwały fault receipt nie może utrzymywać starego Telegramowego
            // ENTRY w stanie RequiresReview bez końca.  Expired nigdy nie jest
            // automatycznie wznawiane ani wykonywane.
            let failure = if clock.is_some() && max_age.is_finite() && max_age > 0.0
                && now.saturating_sub(p.status.first_received_utc) as f64 >= max_age * 1000.0 {
                Some((DeferredEntryState::Expired, "MaximumReceiptAgeExceeded"))
            } else if !self.cfg.defer_entry_until_receipts {
                Some((DeferredEntryState::RequiresReview, "AxisDisabledWithPendingIntent"))
            } else if clock.is_none() {
                Some((DeferredEntryState::RequiresReview, "MissingExplicitReceiptClock"))
            } else if session.is_none() || p.session != session {
                Some((DeferredEntryState::RequiresReview, "ExecutionSessionChangedOrUnverified"))
            } else if barrier == ReceiptBarrier::RequiresReview {
                Some((DeferredEntryState::RequiresReview, "UncertainBrokerExecution"))
            } else if !max_age.is_finite() || max_age <= 0.0 {
                Some((DeferredEntryState::RequiresReview, "InvalidMaximumAge"))
            } else if let Some(entry) = &p.entry {
                let exit = q.exit(entry.side);
                if !exit.is_finite() || exit <= 0.0 {
                    Some((DeferredEntryState::RequiresReview, "InvalidQuote"))
                } else if entry.sl.map(|sl| (exit - sl) * entry.side.sign() <= 0.0).unwrap_or(false) {
                    Some((DeferredEntryState::Cancelled, "StopReachedBeforeEntry"))
                } else if entry.tps.iter().any(|tp| (exit - tp) * entry.side.sign() >= 0.0) {
                    Some((DeferredEntryState::RequiresReview, "TargetReachedBeforeEntry"))
                } else { None }
            } else { None };
            let exact_open_reconciled = failure.is_none()
                && p.status.state == DeferredEntryState::RequiresReview
                && p.status.reason == "UncertainBrokerExecution"
                && p.entry.is_some();
            if let Some((state, reason)) = failure {
                self.deferred_transition(&key, state, reason, q.ts);
            } else if exact_open_reconciled {
                // Bridge usuwa RequiresReview samoczynnie wyłącznie wtedy, gdy
                // niepewne OPEN ma kompletny, jednoznaczny odpowiednik w
                // autorytatywnym snapshotcie MT5. CLOSE, rozłączenie,
                // sprzeczna tożsamość i ręczny latch pozostają zablokowane.
                // Intencja przechwycona w krótkim oknie między timeoutem ACK a
                // snapshotem może więc bez retry wrócić do kolejki i wykonać
                // się dokładnie raz na następnym świeżym ticku.
                self.deferred_transition(
                    &key,
                    DeferredEntryState::Waiting,
                    "ExactOpenSnapshotReconciled",
                    q.ts,
                );
            }
        }
    }

    pub(super) fn deferred_message<B: Broker>(&mut self, b: &B, m: &IncomingMessage, signals: &[Signal]) -> bool {
        self.deferred_message_impl(b,m,signals,false)
    }

    pub(super) fn deferred_after_protection<B: Broker>(&mut self, b: &B, m: &IncomingMessage, entry: &EntrySignal) -> bool {
        if !self.cfg.defer_entry_until_receipts || b.receipt_barrier() == ReceiptBarrier::Clear { return false; }
        let key=(m.source.clone(),m.edit_of.unwrap_or(m.msg_id));
        if self.deferred_entries.is_replay(m) {
            if let Some(p)=self.deferred_entries.records.get_mut(&key) { p.protection_done=true; }
            let (state,reason)=if b.receipt_barrier()==ReceiptBarrier::Temporary {
                (DeferredEntryState::Waiting,"ProtectiveCloseAwaitingReceipts")
            } else { (DeferredEntryState::RequiresReview,"ProtectiveCloseExecutionUncertain") };
            self.deferred_transition(&key,state,reason,m.ts);
            return true;
        }
        self.deferred_message_impl(b,m,&[Signal::Entry(entry.clone())],true)
    }

    fn deferred_message_impl<B: Broker>(&mut self, b: &B, m: &IncomingMessage, signals: &[Signal], after_protection: bool) -> bool {
        if !self.cfg.defer_entry_until_receipts { return false; }
        let direct = (m.source.clone(), m.edit_of.unwrap_or(m.msg_id));
        if self.deferred_entries.releasing.as_ref() == Some(&direct) { return false; }
        self.deferred_observe(b, &b.quote());
        let linked = m.reply_to.map(|id| (m.source.clone(), id))
            .filter(|key| self.deferred_entries.records.contains_key(key));
        let key = if self.deferred_entries.records.contains_key(&direct) { direct.clone() }
            else { linked.unwrap_or_else(|| direct.clone()) };
        // Already-executed intentions use the ordinary basket editor/router.
        if self.msg_to_basket.contains_key(&key) { return false; }
        let known = self.deferred_entries.records.contains_key(&key);
        let entry = signals.iter().find_map(|s| if let Signal::Entry(e) = s { Some(e.clone()) } else { None });
        let only_info = signals.iter().all(|s| matches!(s, Signal::Info));
        let market = signals.iter().any(|s| matches!(s, Signal::MarketOpen { .. }));
        let prepare = only_info && {
            let text = m.text.to_ascii_uppercase();
            text.contains("PREPARE") && (text.contains("BUY") || text.contains("SELL"))
        };
        let barrier = b.receipt_barrier();
        // A genuinely global emergency exit still protects live positions.
        // It invalidates queued entries, but is never swallowed by their queue.
        if signals.iter().any(|s| matches!(s, Signal::CloseAll))
            && self.cfg.honor_close_all && self.cfg.close_all_scope == CloseAllScope::Global {
            let keys: Vec<Key> = self.deferred_entries.records.iter()
                .filter(|(_,p)|p.status.state.active()).map(|(k,_)|k.clone()).collect();
            for k in keys { self.deferred_transition(&k, DeferredEntryState::Cancelled, "GlobalCloseAllBeforeEntry", m.ts); }
            return false;
        }
        // An unthreaded status can concern the new, not-yet-open entry. Never
        // silently attach it to the older live basket just because it exists.
        if !known && m.edit_of.is_none() && m.reply_to.is_none() && entry.is_none() && !only_info && !market {
            let keys: Vec<Key> = self.deferred_entries.records.iter()
                .filter(|(k,p)| k.0 == m.source && p.status.state.active()).map(|(k,_)|k.clone()).collect();
            if !keys.is_empty() {
                for k in keys { self.deferred_transition(&k, DeferredEntryState::RequiresReview, "UnthreadedManagementWhileEntryDeferred", m.ts); }
                return true;
            }
        }
        if !known {
            // Zwykłe INFO (recap, celebracja, komentarz) nie jest intencją
            // wejścia i nie może tworzyć rekordu blokującego tylko dlatego,
            // że broker akurat rozlicza poprzednie zamknięcie.  Zachowujemy
            // wyłącznie PREPARE, bo jego późniejszy EDIT może zawierać ENTRY
            // pod tym samym msg_id.
            if barrier == ReceiptBarrier::Clear
                || (entry.is_none() && !market && (!only_info || !prepare)) { return false; }
            // A pure new entry first executes its protective opposite-close
            // prefix. Capture the still-unexecuted ENTRY at its own gate.
            if !after_protection && entry.is_some()
                && signals.iter().all(|s| matches!(s,Signal::Entry(_)|Signal::Info)) { return false; }
            // Never turn an unknown old EDIT into a new intention after restart.
            if m.edit_of.is_some() || m.reply_to.is_some() { return false; }
            if self.deferred_entries.records.len() >= MAX_SESSION_RECORDS
                || self.deferred_entries.records.values().filter(|p| p.status.state.active()).count() >= MAX_ACTIVE {
                self.jreject(b, m, "entry", RejectCode::EntryGateBlocked, "DeferredQueueCapacityExceeded");
                return true;
            }
            let received = self.deferred_entries.clock_utc.unwrap_or(0);
            self.deferred_entries.sequence += 1;
            let sequence = self.deferred_entries.sequence;
            self.deferred_entries.records.insert(key.clone(), PendingEntry {
                status: DeferredEntryStatus {
                    action_id: format!("{}:{}:entry", key.0.as_string(), key.1),
                    state: DeferredEntryState::NoEntry, reason: "ObservedWhileReceiptBlocked".into(),
                    first_received_utc: received, last_received_utc: received,
                }, message: m.clone(), entry: None, session: b.execution_session(),
                initial_quote_ts: b.quote().ts, last_quote_ts: b.quote().ts, sequence,
                protection_done: after_protection,
            });
        }
        let p = &self.deferred_entries.records[&key];
        if !p.status.state.active() { self.deferred_emit(&key, m.ts); return true; }
        let received = self.deferred_entries.clock_utc.unwrap_or(0);
        if received < p.status.last_received_utc {
            self.log(m.ts, 1, "Deferred ENTRY: older receipt ignored; age and latest payload retained");
            return true;
        }
        if known && m.edit_of.is_none() && m.reply_to.is_none() {
            // A redelivered NEW cannot overwrite the latest EDIT.
            self.deferred_emit(&key, m.ts);
            return true;
        }
        // A reply saying "thanks" is not an edit deleting the original entry.
        if known && m.reply_to.is_some() && only_info { return true; }
        let cancel = signals.iter().any(|s| matches!(s, Signal::Cancel | Signal::CloseAll | Signal::SlHit));
        let mixed = signals.iter().any(|s| !matches!(s, Signal::Entry(_) | Signal::Info));
        let (state, reason) = if cancel { (DeferredEntryState::Cancelled, "CancelledBeforeEntry") }
            // Gołe BUY/SELL NOW nie ma własnej geometrii SL/TP i pozostaje
            // niejednoznaczne.  Pełny Signal::Entry — także market/stop — ma
            // natomiast po receipt wrócić do zwykłej ścieżki, gdzie jawne osie
            // only_limit_signals / honor_stop_orders / market_entry_mode
            // rozstrzygają jego wykonanie identycznie w live i backteście.
            else if market {
                (DeferredEntryState::RequiresReview, "MarketOpenWithoutEntryGeometry")
            } else if mixed || (m.reply_to.is_some() && !only_info) {
                (DeferredEntryState::RequiresReview, "ManagementBeforeEntryRequiresReview")
            } else if entry.is_some() { (DeferredEntryState::Waiting, "WaitingForReceiptBooking") }
            else { (DeferredEntryState::NoEntry, "LatestEditContainsNoEntry") };
        let p = self.deferred_entries.records.get_mut(&key).unwrap();
        p.message = m.clone(); p.message.msg_id = key.1; p.message.edit_of = None; p.message.reply_to = None;
        p.entry = entry; p.status.last_received_utc = received;
        self.deferred_transition(&key, state, reason, m.ts);
        self.deferred_observe(b, &b.quote());
        true
    }

    fn deferred_release<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if !self.cfg.defer_entry_until_receipts || self.deferred_entries.records.is_empty() { return; }
        self.deferred_observe(b, q);
        if b.receipt_barrier() != ReceiptBarrier::Clear || b.quote() != *q { return; }
        let now = self.deferred_entries.clock_utc.unwrap_or(0);
        let key = self.deferred_entries.records.iter()
            .filter(|(_, p)| p.status.state == DeferredEntryState::Waiting
                && q.ts > p.initial_quote_ts && q.ts > p.last_quote_ts && now >= p.status.last_received_utc)
            .min_by_key(|(_, p)| p.sequence).map(|(key, _)| key.clone());
        let Some(key) = key else { return; };
        if self.wejscie_zablokowane(b, q.ts) {
            self.deferred_transition(&key, DeferredEntryState::Rejected, "CurrentEntryGateRejected", q.ts);
            return;
        }
        let p = self.deferred_entries.records.get_mut(&key).unwrap();
        p.last_quote_ts = q.ts;
        let mut message = p.message.clone(); message.ts = q.ts;
        self.deferred_transition(&key, DeferredEntryState::Attempted, "SingleValidatedDispatchAttempt", q.ts);
        self.deferred_entries.releasing = Some(key.clone());
        self.on_message(b, &message);
        self.deferred_entries.releasing = None;
        // Protective close may itself have created a new receipt barrier.
        // It was not an ENTRY execution attempt and is never repeated.
        if self.deferred_entries.records[&key].status.state == DeferredEntryState::Waiting { return; }
        let (state, reason) = if b.receipt_barrier() == ReceiptBarrier::RequiresReview {
            (DeferredEntryState::RequiresReview, "ExecutionUncertainNoAutomaticRetry")
        } else if self.msg_to_basket.contains_key(&key) {
            (DeferredEntryState::Executed, "BasketCreatedByNormalEntryPipeline")
        } else { (DeferredEntryState::Rejected, "NormalEntryPipelineDidNotCreateBasket") };
        self.deferred_transition(&key, state, reason, q.ts);
    }
}
