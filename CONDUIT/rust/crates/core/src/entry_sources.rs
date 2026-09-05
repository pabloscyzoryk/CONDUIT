//! Durable source identity; this ledger does not change a preset's order lifetime.
use super::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct EntrySourceRecord {
    pub source: SourceKey,
    pub msg_id: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basket_id: Option<u32>,
    #[serde(default)]
    pub first_entry_was_edit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_entry_edit_ts: Option<Ts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_ts: Option<Ts>,
}

/// Recovery is permitted only for a protected, complete intent. Further preset
/// geometry and risk gates still execute normally inside handle_entry.
pub(super) fn complete_recoverable_entry(e: &EntrySignal) -> bool {
    let positive = |x: f64| x.is_finite() && x > 0.0;
    if !positive(e.lo) || !positive(e.hi) || e.lo > e.hi || (e.is_limit && e.is_stop) {
        return false;
    }
    let Some(sl) = e.sl.filter(|v| positive(*v)) else {
        return false;
    };
    if match e.side {
        Side::Buy => sl >= e.lo,
        Side::Sell => sl <= e.hi,
    } {
        return false;
    }
    if e.tps.is_empty() && !e.tp_open {
        return false;
    }
    e.tps
        .iter()
        .all(|tp| positive(*tp) && (*tp - e.side.better_edge(e.lo, e.hi)) * e.side.sign() > 0.0)
        && e.warstwy_offset.is_none_or(|v| v.is_finite() && v >= 0.0)
}

impl Engine {
    pub fn entry_source_memory_revision(&self) -> u64 {
        self.entry_source_revision
    }

    pub fn export_entry_source_memory(&self) -> Vec<EntrySourceRecord> {
        let mut records: Vec<_> = self.entry_sources.values().cloned().collect();
        records.sort_by_key(|r| (r.source.chat_id, r.source.topic_id, r.msg_id));
        records
    }

    pub fn restore_entry_source_memory(&mut self, records: &[EntrySourceRecord]) {
        for record in records {
            let key = (record.source.clone(), record.msg_id);
            let mut merged = record.clone();
            if let Some(old) = self.entry_sources.get(&key) {
                merged.cancelled_ts = old.cancelled_ts.or(merged.cancelled_ts);
                merged.first_entry_was_edit |= old.first_entry_was_edit;
                merged.last_entry_edit_ts = old.last_entry_edit_ts.max(merged.last_entry_edit_ts);
                merged.basket_id = merged.basket_id.or(old.basket_id);
                for a in &old.aliases {
                    if !merged.aliases.contains(a) {
                        merged.aliases.push(*a);
                    }
                }
            }
            merged.aliases.sort_unstable();
            merged.aliases.dedup();
            if self.entry_sources.get(&key) != Some(&merged) {
                self.entry_sources.insert(key.clone(), merged.clone());
                self.entry_source_revision = pending_validity::next_source_revision();
            }
            self.entry_source_aliases.insert(key.clone(), record.msg_id);
            for a in &merged.aliases {
                self.entry_source_aliases
                    .insert((record.source.clone(), *a), record.msg_id);
            }
            if let Some(id) = merged.basket_id {
                self.next_basket_id = self.next_basket_id.max(id.saturating_add(1));
                self.msg_to_basket.insert(key, id);
                for a in &merged.aliases {
                    self.msg_to_basket.insert((record.source.clone(), *a), id);
                }
            }
        }
    }

    pub(super) fn entry_source_record(
        &self,
        source: &SourceKey,
        msg_id: i64,
    ) -> Option<&EntrySourceRecord> {
        let root = self
            .entry_source_aliases
            .get(&(source.clone(), msg_id))
            .copied()
            .unwrap_or(msg_id);
        self.entry_sources.get(&(source.clone(), root))
    }

    pub(super) fn entry_source_withdrawn(&self, m: &IncomingMessage) -> bool {
        self.entry_source_record(&m.source, m.edit_of.unwrap_or(m.msg_id))
            .is_some_and(|r| r.cancelled_ts.is_some())
    }

    pub(super) fn remember_entry_source(&mut self, m: &IncomingMessage, id: u32) {
        let source_id = m.edit_of.unwrap_or(m.msg_id);
        let mut r = self
            .entry_source_record(&m.source, source_id)
            .cloned()
            .unwrap_or(EntrySourceRecord {
                source: m.source.clone(),
                msg_id: source_id,
                aliases: Vec::new(),
                basket_id: None,
                first_entry_was_edit: false,
                last_entry_edit_ts: None,
                cancelled_ts: None,
            });
        if r.basket_id.is_none() {
            r.first_entry_was_edit = m.edit_of.is_some();
        }
        r.basket_id = Some(id);
        if m.edit_of.is_some() {
            r.last_entry_edit_ts = Some(m.ts);
        }
        if m.msg_id != r.msg_id {
            r.aliases.push(m.msg_id);
        }
        self.restore_entry_source_memory(&[r]);
    }

    pub(super) fn remember_adopted_entry_source(&mut self, bk: &Basket) {
        self.restore_entry_source_memory(&[EntrySourceRecord {
            source: bk.source.clone(),
            msg_id: bk.msg_id,
            aliases: bk.msg_aliases.clone(),
            basket_id: Some(bk.id),
            first_entry_was_edit: false,
            last_entry_edit_ts: None,
            cancelled_ts: bk
                .entry_edit_state
                .as_ref()
                .and_then(|s| s.cancelled_by_source_ts),
        }]);
    }

    pub(super) fn remember_entry_edit(&mut self, m: &IncomingMessage) {
        if let Some(mut r) = self
            .entry_source_record(&m.source, m.edit_of.unwrap_or(m.msg_id))
            .cloned()
        {
            r.last_entry_edit_ts = Some(m.ts);
            self.restore_entry_source_memory(&[r]);
        }
    }

    pub(super) fn remember_entry_alias(&mut self, id: u32, alias: i64) {
        let Some(mut r) = self
            .entry_sources
            .values()
            .find(|r| r.basket_id == Some(id))
            .cloned()
        else {
            return;
        };
        if alias != r.msg_id && !r.aliases.contains(&alias) {
            r.aliases.push(alias);
            self.restore_entry_source_memory(&[r]);
        }
    }

    /// A direct reply proves the publisher's source identity even when NEW was
    /// missed. Non-entry replies preserve a bounded transitive reply chain.
    pub(super) fn observe_source_reply(&mut self, m: &IncomingMessage, signals: &[Signal]) {
        let Some(reply) = m.reply_to else { return };
        if signals
            .iter()
            .any(|s| matches!(s, Signal::Entry(_) | Signal::MarketOpen { .. }))
        {
            return;
        }
        let cancel = signals.iter().any(|s| matches!(s, Signal::Cancel));
        // Extending provenance through informational replies belongs to the
        // selected recovery policy; legacy orphan-block mode keeps its graph.
        if !cancel && (self.cfg.edycja_sieroty_nie_otwiera || !self.cfg.reply_graph_transitive) {
            return;
        }
        let mut r = self
            .entry_source_record(&m.source, reply)
            .cloned()
            .unwrap_or(EntrySourceRecord {
                source: m.source.clone(),
                msg_id: reply,
                aliases: Vec::new(),
                basket_id: self.msg_to_basket.get(&(m.source.clone(), reply)).copied(),
                first_entry_was_edit: false,
                last_entry_edit_ts: None,
                cancelled_ts: None,
            });
        if cancel {
            r.cancelled_ts.get_or_insert(m.ts);
        }
        if m.msg_id != r.msg_id {
            r.aliases.push(m.msg_id);
        }
        self.restore_entry_source_memory(&[r]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testy_pakiet_a::{wiad, Atrapa, TS0, WEJSCIE};
    fn engine() -> Engine {
        Engine::new(
            Settings {
                edycja_sieroty_nie_otwiera: false,
                entry_idempotencja: false,
                explicit_pending_until_cancel: false,
                reply_graph_transitive: true,
                risk_per_basket_pct: 0.0,
                max_portfolio_risk_pct: 0.0,
                max_open_positions: 0,
                max_open_baskets: 0,
                lot_mode_percent: false,
                lot_fixed: 0.01,
                ..Settings::default()
            },
            400.0,
        )
    }
    #[test]
    fn first_complete_edit_opens_once_at_receive_and_late_new_never_rolls_back() {
        let mut b = Atrapa::nowa();
        let mut e = engine();
        let mut edit = wiad(1, 50, Some(999), WEJSCIE);
        edit.ts = TS0 + 1234;
        e.on_message(&mut b, &edit);
        assert_eq!(e.created_baskets_count(), 1);
        assert_eq!(e.baskets[0].created_ts, edit.ts);
        let n = b.positions().len() + b.pendings().len();
        assert!(n > 0);
        e.on_message(&mut b, &edit);
        let changed = WEJSCIE.replace("SL 3990", "SL 3991");
        e.on_message(&mut b, &wiad(1, 50, Some(999), &changed));
        assert_eq!(e.baskets[0].sl, Some(3991.0));
        e.on_message(&mut b, &wiad(1, 999, None, WEJSCIE));
        assert_eq!(e.created_baskets_count(), 1);
        assert_eq!(e.baskets[0].sl, Some(3991.0));
        assert_eq!(b.positions().len() + b.pendings().len(), n);
        let r = e
            .entry_source_record(&SourceKey::new(1, None), 999)
            .unwrap();
        assert!(r.first_entry_was_edit);
        assert!(r.aliases.contains(&50));
    }
    #[test]
    fn known_information_then_complete_edit_uses_the_same_recovery_path() {
        let mut b = Atrapa::nowa();
        let mut e = engine();
        e.on_message(&mut b, &wiad(1, 100, None, "Preparing the next setup"));
        assert!(e.baskets.is_empty());
        e.on_message(&mut b, &wiad(1, 100, Some(100), WEJSCIE));
        assert_eq!(e.created_baskets_count(), 1);
    }
    #[test]
    fn recovery_requires_complete_protected_geometry_and_honors_legacy_policy() {
        for text in [
            "BUY NOW",
            "BUY GOLD @ 4000",
            "BUY GOLD @ 4000 SL 3990",
            "BUY GOLD @ 4000 TP 4010",
            "BUY GOLD @ 4000 SL 4001 TP 4010",
        ] {
            let mut b = Atrapa::nowa();
            let mut e = engine();
            e.cfg.honor_market_open = true;
            e.on_message(&mut b, &wiad(1, 100, Some(100), text));
            assert!(e.baskets.is_empty(), "incomplete orphan opened: {text}");
        }
        let mut b = Atrapa::nowa();
        let mut e = engine();
        e.cfg.edycja_sieroty_nie_otwiera = true;
        e.on_message(&mut b, &wiad(1, 100, Some(100), WEJSCIE));
        assert!(e.baskets.is_empty());
        assert_eq!(e.odrzuty.get("EditOrphan"), Some(&1));
    }
    #[test]
    fn risk_rejection_is_not_consumption_and_a_later_complete_edit_can_be_evaluated() {
        let mut b = Atrapa::nowa();
        let mut e = engine();
        e.halted = Some("synthetic risk stop".into());
        e.on_message(&mut b, &wiad(1, 100, Some(100), WEJSCIE));
        assert!(e.baskets.is_empty());
        assert!(e.entry_sources.is_empty());
        e.halted = None;
        e.on_message(&mut b, &wiad(1, 100, Some(100), WEJSCIE));
        assert_eq!(e.created_baskets_count(), 1);
    }
    #[test]
    fn unknown_bound_cancel_persists_and_resolves_transitive_replies_by_source() {
        let mut b = Atrapa::nowa();
        let mut e = engine();
        e.cfg.honor_cancel = false;
        let mut info = wiad(1, 101, None, "Setup update");
        info.reply_to = Some(100);
        e.on_message(&mut b, &info);
        let mut cancel = wiad(1, 102, None, "NO LONGER VALID");
        cancel.reply_to = Some(101);
        e.on_message(&mut b, &cancel);
        let state = serde_json::to_vec(&e.export_entry_source_memory()).unwrap();
        let records: Vec<EntrySourceRecord> = serde_json::from_slice(&state).unwrap();
        let mut restored = engine();
        restored.restore_entry_source_memory(&records);
        for id in [100, 101] {
            restored.on_message(&mut b, &wiad(1, id, Some(id), WEJSCIE));
            restored.on_message(&mut b, &wiad(1, id, None, WEJSCIE));
        }
        assert!(restored.baskets.is_empty());
        restored.on_message(&mut b, &wiad(2, 100, Some(100), WEJSCIE));
        assert_eq!(
            restored.created_baskets_count(),
            1,
            "same ID in another source is independent"
        );
    }
    #[test]
    fn consumed_source_survives_basket_pruning_and_policy_toggles() {
        let mut b = Atrapa::nowa();
        let mut e = engine();
        e.on_message(&mut b, &wiad(1, 100, Some(100), WEJSCIE));
        let records = e.export_entry_source_memory();
        // No retained basket and no volatile action cache after restart/pruning.
        let mut restored = engine();
        restored.restore_entry_source_memory(&records);
        let before = (b.positions().len(), b.pendings().len());
        restored.cfg.edycja_sieroty_nie_otwiera = true;
        restored.on_message(&mut b, &wiad(1, 100, Some(100), WEJSCIE));
        restored.cfg.edycja_sieroty_nie_otwiera = false;
        restored.on_message(&mut b, &wiad(1, 100, Some(100), WEJSCIE));
        restored.on_message(&mut b, &wiad(1, 100, None, WEJSCIE));
        assert_eq!(restored.created_baskets_count(), 0);
        assert_eq!((b.positions().len(), b.pendings().len()), before);
    }
    #[test]
    fn a_reply_cancel_after_recovery_targets_only_its_pending_basket() {
        let mut b = Atrapa::nowa();
        let mut e = engine();
        e.cfg.honor_cancel = true;
        b.ustaw_cene(TS0, 4008.0, 4008.2);
        e.on_message(&mut b, &wiad(1, 100, Some(100), WEJSCIE));
        e.on_message(&mut b, &wiad(1, 200, Some(200), WEJSCIE));
        let first = e.baskets[0].id;
        let second = e.baskets[1].id;
        assert!(
            b.pendings().iter().any(|p| p.basket == Some(second)),
            "fixture needs another live grid"
        );
        let mut alias = wiad(1, 101, None, "Setup update");
        alias.reply_to = Some(100);
        e.on_message(&mut b, &alias);
        let mut cancel = wiad(1, 102, None, "CANCEL");
        cancel.reply_to = Some(101);
        e.on_message(&mut b, &cancel);
        assert!(!b.pendings().iter().any(|p| p.basket == Some(first)));
        assert!(b.pendings().iter().any(|p| p.basket == Some(second)));
        let after = b.positions().len();
        e.on_message(&mut b, &wiad(1, 100, None, WEJSCIE));
        assert_eq!(e.created_baskets_count(), 2);
        assert_eq!(b.positions().len(), after);
    }
    #[test]
    fn legacy_orphan_block_keeps_informational_reply_graph_unchanged() {
        let mut e = engine();
        let mut b = Atrapa::nowa();
        e.cfg.edycja_sieroty_nie_otwiera = true;
        e.on_message(&mut b, &wiad(1, 100, None, WEJSCIE));
        let mut info = wiad(1, 101, None, "Setup update");
        info.reply_to = Some(100);
        e.on_message(&mut b, &info);
        assert!(!e
            .msg_to_basket
            .contains_key(&(SourceKey::new(1, None), 101)));
        assert!(e
            .entry_source_record(&SourceKey::new(1, None), 101)
            .is_none());
    }

    #[test]
    fn source_memory_revision_changes_for_alias_and_tombstone_not_duplicate_restore() {
        let mut e = engine();
        let mut b = Atrapa::nowa();
        let empty = e.entry_source_memory_revision();
        e.on_message(&mut b, &wiad(1, 100, Some(100), WEJSCIE));
        let accepted = e.entry_source_memory_revision();
        assert_ne!(empty, accepted);
        let records = e.export_entry_source_memory();
        e.restore_entry_source_memory(&records);
        assert_eq!(accepted, e.entry_source_memory_revision());
        let mut m = wiad(1, 101, None, "CANCEL");
        m.reply_to = Some(100);
        e.on_message(&mut b, &m);
        assert_ne!(accepted, e.entry_source_memory_revision());
    }
}
