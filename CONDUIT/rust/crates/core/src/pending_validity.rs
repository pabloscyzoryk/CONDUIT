//! Publisher validity is independent of broker exposure and strategy risk exits.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

// Process-local change token: replacing an Engine cannot reuse another ledger's
// revision. Durable snapshots still contain the complete source records.
static NEXT_SOURCE_REVISION: AtomicU64 = AtomicU64::new(1);
pub(super) fn next_source_revision() -> u64 {
    crate::recorded_broker::revisions::token(|| NEXT_SOURCE_REVISION.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PendingSourceRecord {
    pub source: SourceKey,
    pub msg_id: i64,
    pub basket_id: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_ts: Option<Ts>,
}

impl Engine {
    /// O(1) invalidation token for the live persistence cache. Every source
    /// mutation, including future fields, must advance this token.
    pub fn pending_source_memory_revision(&self) -> u64 { self.pending_source_revision }

    fn source_memory_changed(&mut self) {
        self.pending_source_revision = next_source_revision();
    }

    pub fn export_pending_source_memory(&self) -> Vec<PendingSourceRecord> {
        self.pending_sources.values().cloned().collect()
    }

    pub fn restore_pending_source_memory(&mut self, records: &[PendingSourceRecord]) {
        let mut changed = false;
        for record in records {
            let before = self.pending_sources.get(&record.basket_id).cloned();
            self.msg_to_basket.insert((record.source.clone(), record.msg_id), record.basket_id);
            for alias in &record.aliases {
                self.msg_to_basket.insert((record.source.clone(), *alias), record.basket_id);
            }
            self.next_basket_id = self.next_basket_id.max(record.basket_id.saturating_add(1));
            let saved = self.pending_sources.entry(record.basket_id).or_insert_with(|| record.clone());
            if saved.source == record.source && saved.msg_id == record.msg_id {
                saved.cancelled_ts = saved.cancelled_ts.or(record.cancelled_ts);
                for alias in &record.aliases { if !saved.aliases.contains(alias) { saved.aliases.push(*alias); } }
            }
            changed |= before.as_ref() != Some(saved);
        }
        if changed { self.source_memory_changed(); }
        self.sync_source_tombstones();
    }

    pub(super) fn sync_source_tombstones(&mut self) {
        for bk in &mut self.baskets {
            if let Some(ts) = self.pending_sources.get(&bk.id).and_then(|r| r.cancelled_ts) {
                let state = bk.entry_edit_state.get_or_insert_with(|| Box::new(EntryEditState {
                    schema_version: 1, revision: 0, source: None, applied_ts: ts,
                    cancelled_by_source_ts: None, review: None,
                }));
                state.cancelled_by_source_ts.get_or_insert(ts);
            }
        }
    }

    pub(super) fn remember_pending_source(&mut self, bk: &Basket) {
        if self.pending_sources.contains_key(&bk.id)
            || (self.cfg.explicit_pending_until_cancel && (bk.is_limit || bk.is_stop))
            || bk.entry_edit_state.as_ref().is_some_and(|s| s.cancelled_by_source_ts.is_some()) {
            let record = PendingSourceRecord { source: bk.source.clone(), msg_id: bk.msg_id,
                basket_id: bk.id, aliases: bk.msg_aliases.clone(), cancelled_ts: bk.entry_edit_state.as_ref()
                    .and_then(|s| s.cancelled_by_source_ts) };
            self.restore_pending_source_memory(&[record]);
        }
    }

    pub(super) fn remember_pending_source_alias(&mut self, id: u32, alias: i64) {
        let Some(record) = self.pending_sources.get_mut(&id) else { return };
        if alias != record.msg_id && !record.aliases.contains(&alias) {
            record.aliases.push(alias);
            self.source_memory_changed();
        }
    }

    pub(super) fn explicit_pending_source(&self, id: u32) -> bool {
        self.pending_sources.contains_key(&id) || (self.cfg.explicit_pending_until_cancel
            && self.basket(id).is_some_and(|bk| bk.is_limit || bk.is_stop))
    }

    pub(super) fn pending_source_cancelled(&self, id: u32) -> bool {
        self.pending_sources.get(&id).is_some_and(|r| r.cancelled_ts.is_some())
            || self.basket(id).and_then(|b| b.entry_edit_state.as_ref())
                .is_some_and(|s| s.cancelled_by_source_ts.is_some())
    }

    pub(super) fn keep_explicit_pending(&self, id: u32) -> bool {
        self.explicit_pending_source(id) && !self.pending_source_cancelled(id)
    }

    pub(super) fn withdraw_pending_source<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts) -> usize {
        if let Some(bk) = self.basket(id).cloned() { self.remember_pending_source(&bk); }
        if let Some(record) = self.pending_sources.get_mut(&id) {
            if record.cancelled_ts.is_none() {
                record.cancelled_ts = Some(ts);
                self.source_memory_changed();
            }
        }
        if let Some(bk) = self.basket_mut(id) {
            let state = bk.entry_edit_state.get_or_insert_with(|| Box::new(EntryEditState {
                schema_version: 1, revision: 0, source: None, applied_ts: ts,
                cancelled_by_source_ts: None, review: None,
            }));
            state.cancelled_by_source_ts.get_or_insert(ts);
            bk.drop_po_ts = 0;
        }
        self.cancel_source_orders(b, id)
    }

    pub(super) fn cancel_source_orders<B: Broker>(&mut self, b: &mut B, id: u32) -> usize {
        let tickets: Vec<_> = b.pendings().iter().filter(|o| o.basket == Some(id))
            .map(|o| o.ticket).collect();
        let mut cancelled = 0;
        for ticket in tickets { if b.cancel_pending(ticket).is_ok() { cancelled += 1; } }
        let remaining: Vec<_> = b.pendings().iter().filter(|o| o.basket == Some(id))
            .map(|o| o.ticket).collect();
        let has_positions = b.positions().iter().any(|p| p.basket == Some(id));
        if let Some(bk) = self.basket_mut(id) {
            bk.pendings = remaining;
            for level in &mut bk.levels { if !level.filled { level.cancelled = true; } }
            if !has_positions && bk.pendings.is_empty() { bk.state = BasketState::Done; }
        }
        cancelled
    }

    pub(super) fn retry_source_cancellations<B: Broker>(&mut self, b: &mut B) {
        if self.pending_sources.is_empty() { return; }
        let ids: std::collections::BTreeSet<_> = b.pendings().iter().filter_map(|o| o.basket)
            .filter(|id| self.pending_source_cancelled(*id)).collect();
        for id in ids { self.cancel_source_orders(b, id); }
    }
}
