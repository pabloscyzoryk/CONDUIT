//! Reporting only. This state never authorizes or changes an Engine action.
use super::*;
use std::collections::{HashMap, HashSet};

type Key = (String, i64);

#[derive(Default)]
pub(super) struct SourceFunnel {
    offered: HashSet<Key>,
    accepted: HashSet<Key>,
    rejected: HashSet<Key>,
    aliases: HashMap<Key, Key>,
    names: HashMap<(usize, i64), HashSet<Key>>,
    revisions: HashMap<usize, u64>,
    rejection_cursors: HashMap<usize, usize>,
    ambiguous: HashSet<(usize, i64)>,
    unattributed: u32,
}

impl SourceFunnel {
    pub fn observe(
        &mut self,
        engine_index: usize,
        message: &ReplayMessage,
        cfg: &Settings,
    ) -> Option<Key> {
        let signals = conduit_core::parser::parse_z_opcjami(
            &message.text,
            conduit_core::parser::OpcjeParsera {
                geometryczny: cfg.parser_geometryczny,
                min_pewnosc: cfg.parser_min_pewnosc,
                rf_wymaga_wykonania: cfg.rf_wymaga_wykonania,
                partials_jako_komenda: cfg.partials_wykonuj,
                luz_interpunkcyjny: cfg.parser_luz_interpunkcyjny,
                recap_guard: cfg.recap_guard,
            },
        );
        if !signals.iter().any(|s| {
            matches!(
                s,
                conduit_core::parser::Signal::Entry(_)
                    | conduit_core::parser::Signal::MarketOpen { .. }
            )
        }) {
            return None;
        }
        // Legacy replay has no real chat/topic metadata. Preserve its namespace
        // rather than pretending that a synthetic Engine SourceKey is Telegram.
        let key = (
            message.kanal.trim().to_ascii_lowercase(),
            message.edit_of.unwrap_or(message.msg_id),
        );
        if self.aliases.get(&key).is_some_and(|root| root != &key) {
            return None;
        }
        self.offered.insert(key.clone());
        for id in [message.msg_id, message.edit_of.unwrap_or(message.msg_id)] {
            self.names
                .entry((engine_index, id))
                .or_default()
                .insert(key.clone());
        }
        Some(key)
    }

    pub fn reject_before_engine(&mut self, key: Option<Key>) {
        if let Some(key) = key {
            self.rejected.insert(key);
        }
    }

    pub fn collect(&mut self, index: usize, engine: &Engine) {
        let cursor = self.rejection_cursors.entry(index).or_default();
        if *cursor > engine.odrzucone_wejscia.len() {
            *cursor = 0;
        }
        for rejection in &engine.odrzucone_wejscia[*cursor..] {
            match self.names.get(&(index, rejection.msg_id)) {
                Some(keys) if keys.len() == 1 => self.rejected.extend(keys.iter().cloned()),
                Some(_) => {
                    if self.ambiguous.insert((index, rejection.msg_id)) {
                        self.unattributed = self.unattributed.saturating_add(1);
                    }
                }
                None if self.aliases.iter().any(|((_, id), root)| {
                    *id == rejection.msg_id
                        && self
                            .names
                            .get(&(index, root.1))
                            .is_some_and(|keys| keys.contains(root))
                }) => {}
                None => self.unattributed = self.unattributed.saturating_add(1),
            }
        }
        *cursor = engine.odrzucone_wejscia.len();
        let revision = engine.entry_source_memory_revision();
        if self.revisions.get(&index) == Some(&revision) {
            return;
        }
        self.revisions.insert(index, revision);
        let records = engine.export_entry_source_memory();
        let independent: HashSet<_> = records
            .iter()
            .filter(|r| r.basket_id.is_some())
            .map(|r| r.msg_id)
            .collect();
        for record in records.iter().filter(|r| r.basket_id.is_some()) {
            let Some(keys) = self.names.get(&(index, record.msg_id)) else {
                continue;
            };
            if keys.len() != 1 {
                if self.ambiguous.insert((index, record.msg_id)) {
                    self.unattributed = self.unattributed.saturating_add(1);
                }
                continue;
            }
            for key in keys {
                self.accepted.insert(key.clone());
                for alias in record.aliases.iter().filter(|id| !independent.contains(id)) {
                    self.aliases.insert((key.0.clone(), *alias), key.clone());
                }
            }
        }
    }

    pub fn apply(&self, funnel: &mut crate::statystyki::Lejek) {
        // Aliases discovered later are management versions of their established
        // source. An independently accepted merged NEW has its own record and
        // is deliberately not folded into its shared basket's original source.
        let independent = |key: &&Key| !self.aliases.get(*key).is_some_and(|root| root != *key);
        let count = |n: usize| n.min(u32::MAX as usize) as u32;
        funnel.source_observation_version = 1;
        funnel.source_identity_semantics =
            "channel namespace + original message ID; raw chat/topic unavailable".into();
        funnel.unattributed_entry_outcomes = self.unattributed;
        funnel.sygnaly_wejsciowe = count(self.offered.iter().filter(independent).count());
        funnel.koszyki_sygnaly = count(self.accepted.iter().filter(independent).count());
        funnel.odrzucone_sygnaly = count(
            self.rejected
                .difference(&self.accepted)
                .filter(independent)
                .count(),
        );
        funnel.zgubione_bez_sladu = funnel.sygnaly_wejsciowe as i64
            - funnel.koszyki_sygnaly as i64
            - funnel.odrzucone_sygnaly as i64;
        funnel.wykonanych_pct = if funnel.sygnaly_wejsciowe == 0 {
            0.
        } else {
            funnel.koszyk_z_handlem as f64 / funnel.sygnaly_wejsciowe as f64 * 100.
        };
        funnel.accepted_entry_sources_pct = if funnel.sygnaly_wejsciowe == 0 {
            0.
        } else {
            funnel.koszyki_sygnaly as f64 / funnel.sygnaly_wejsciowe as f64 * 100.
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ENTRY: &str = "BUY LIMIT GOLD @ 3999/3998\nSL 3990\nTP 4020";
    fn config() -> Settings {
        Settings {
            edycja_sieroty_nie_otwiera: false,
            entry_idempotencja: true,
            merge_same_side: true,
            merge_window_min: 60.,
            merge_min_overlap: 0.5,
            reply_graph_transitive: true,
            side_filter: conduit_core::settings::SideFilter::BuyOnly,
            lot_fixed: 0.01,
            lot_max: 0.01,
            equity_floor_pct: 0.,
            max_dd_pct: 0.,
            max_open_positions: 0,
            max_open_baskets: 0,
            swap_enabled: false,
            ..Default::default()
        }
    }
    fn rig() -> (Engine, SimBroker) {
        let cfg = config();
        let mut broker = SimBroker::z_ustawien(600., &cfg);
        let q = Quote {
            ts: 1_700_000_000_000,
            bid: 4005.,
            ask: 4005.2,
        };
        broker.on_quote(q);
        let mut engine = Engine::new(cfg, 600.);
        engine.on_tick(&mut broker, &q);
        (engine, broker)
    }
    fn deliver(
        f: &mut SourceFunnel,
        index: usize,
        e: &mut Engine,
        b: &mut SimBroker,
        id: i64,
        edit: bool,
        reply: Option<i64>,
        text: &str,
        namespace: &str,
    ) {
        let m = ReplayMessage {
            ts: b.quote().ts,
            msg_id: id,
            edit_of: edit.then_some(id),
            reply_to: reply,
            text: text.into(),
            kanal: namespace.into(),
            ..Default::default()
        };
        f.observe(index, &m, &e.cfg);
        e.on_message(
            b,
            &IncomingMessage {
                ts: m.ts,
                source: SourceKey::new(-990001, None),
                source_name: namespace.into(),
                msg_id: id,
                edit_of: m.edit_of,
                reply_to: reply,
                text: text.into(),
            },
        );
        f.collect(index, e);
    }
    #[test]
    fn causal_sources_cover_first_edit_draft_duplicates_aliases_merged_new_and_rejections() {
        let (mut e, mut b) = rig();
        let mut f = SourceFunnel::default();
        deliver(
            &mut f,
            0,
            &mut e,
            &mut b,
            10,
            false,
            None,
            "Preparing a signal",
            "Synergy",
        );
        deliver(&mut f, 0, &mut e, &mut b, 10, true, None, ENTRY, "Synergy");
        deliver(&mut f, 0, &mut e, &mut b, 10, true, None, ENTRY, "Synergy");
        deliver(&mut f, 0, &mut e, &mut b, 10, false, None, ENTRY, "Synergy");
        assert_eq!(e.created_baskets_count(), 1);
        deliver(
            &mut f,
            0,
            &mut e,
            &mut b,
            20,
            false,
            Some(10),
            "Signal update follows",
            "Synergy",
        );
        deliver(
            &mut f,
            0,
            &mut e,
            &mut b,
            20,
            true,
            Some(10),
            ENTRY,
            "Synergy",
        );
        assert_eq!(
            e.created_baskets_count(),
            1,
            "known reply alias edits the existing source"
        );
        deliver(
            &mut f,
            0,
            &mut e,
            &mut b,
            30,
            false,
            Some(10),
            ENTRY,
            "Synergy",
        );
        assert_eq!(
            e.created_baskets_count(),
            1,
            "independent accepted NEW can merge into one basket"
        );
        deliver(
            &mut f,
            0,
            &mut e,
            &mut b,
            40,
            true,
            None,
            "SELL GOLD @ 4010/4009\nSL 4020\nTP 3990",
            "Synergy",
        );
        let mut l = crate::statystyki::Lejek {
            koszyki: 1,
            koszyk_z_handlem: 1,
            ..Default::default()
        };
        f.apply(&mut l);
        assert_eq!(
            l.sygnaly_wejsciowe, 3,
            "two independent accepted sources plus rejected first EDIT"
        );
        assert_eq!(l.koszyki_sygnaly, 2);
        assert_eq!(l.odrzucone_sygnaly, 1);
        assert_eq!(l.zgubione_bez_sladu, 0);
        assert_eq!(l.unattributed_entry_outcomes, 0);
        assert!((l.wykonanych_pct - 100. / 3.).abs() < 1e-10);
        assert!((l.accepted_entry_sources_pct - 200. / 3.).abs() < 1e-10);
    }
    #[test]
    fn same_message_id_in_two_namespaces_is_two_sources() {
        let (mut a, mut ba) = rig();
        let (mut b, mut bb) = rig();
        let mut f = SourceFunnel::default();
        deliver(
            &mut f, 0, &mut a, &mut ba, 10, false, None, ENTRY, "Synergy",
        );
        deliver(
            &mut f,
            1,
            &mut b,
            &mut bb,
            10,
            true,
            None,
            ENTRY,
            "SyntheticSecond",
        );
        let mut l = crate::statystyki::Lejek::default();
        f.apply(&mut l);
        assert_eq!(l.sygnaly_wejsciowe, 2);
        assert_eq!(l.koszyki_sygnaly, 2);
        assert_eq!(l.zgubione_bez_sladu, 0);
    }
    #[test]
    fn missing_route_counts_only_entry_sources_and_dedupes_rejected_revisions() {
        let mut f = SourceFunnel::default();
        let cfg = config();
        for (text, edit) in [("TP1 HIT", false), (ENTRY, true), (ENTRY, true)] {
            let m = ReplayMessage {
                msg_id: 10,
                edit_of: edit.then_some(10),
                text: text.into(),
                kanal: "Synergy".into(),
                ..Default::default()
            };
            let key = f.observe(usize::MAX, &m, &cfg);
            f.reject_before_engine(key);
        }
        let mut l = crate::statystyki::Lejek::default();
        f.apply(&mut l);
        assert_eq!(l.sygnaly_wejsciowe, 1);
        assert_eq!(l.odrzucone_sygnaly, 1);
        assert_eq!(l.zgubione_bez_sladu, 0);
    }
}
