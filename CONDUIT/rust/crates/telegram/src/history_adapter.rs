//! Current Telegram history -> real core parser -> pure LIMIT validator.
//! This adapter intentionally has NO Engine, Broker, MessageSink or order API.

use std::collections::{BTreeMap, BTreeSet};

use conduit_core::parser::{self, EntrySignal, OpcjeParsera, ProfitUpdateKind, Signal};
use conduit_server::history_import::{
    HistoryPreviewDecision, HistoryPreviewResponse, HistorySnapshot,
};

use crate::history_import::{
    self as validator, Fact, HistoricalMessage, HistoryCoverage, ImportRequest, ImportWindow,
    LimitPlan, ManagementKind, MessageKey, Status,
};

/// A future planner must supply the ACTUAL rounded broker plan and exact parsed
/// entry it was built from. A zone endpoint is not a substitute for that plan.
pub struct PlannedLimit {
    pub parsed_entry: EntrySignal,
    pub broker_plan: LimitPlan,
}

pub struct AdaptedHistory {
    pub messages: Vec<HistoricalMessage>,
    pub coverage: HistoryCoverage,
}

fn facts(text: &str, plan: Option<&PlannedLimit>) -> Vec<Fact> {
    // Use production grammar. Broader recognition here only disqualifies import;
    // it does not change the live parser axes or reinterpret management as orders.
    let parsed = parser::parse_z_opcjami(
        text,
        OpcjeParsera {
            geometryczny: false,
            min_pewnosc: 1.0,
            rf_wymaga_wykonania: true,
            partials_jako_komenda: true,
            luz_interpunkcyjny: true,
            recap_guard: true,
        },
    );
    let mut out = Vec::new();
    for signal in parsed {
        out.push(match signal {
            Signal::Entry(entry) if entry.is_limit && !entry.is_stop => {
                if let Some(p) = plan.filter(|p| p.parsed_entry == entry && p.broker_plan.valid()) {
                    Fact::Limit(p.broker_plan.clone())
                } else {
                    Fact::LimitWithoutBrokerPlan
                }
            }
            Signal::Entry(_) | Signal::MarketOpen { .. } => Fact::MarketEntry,
            Signal::TpHit { .. } => Fact::Management(ManagementKind::TpHit),
            Signal::SlHit => Fact::Management(ManagementKind::SlHit),
            Signal::RiskFree { .. } => Fact::Management(ManagementKind::RiskFree),
            Signal::SecuringPartial { .. } | Signal::TakePartials => {
                Fact::Management(ManagementKind::SecuringPartial)
            }
            Signal::OutAtEntry => Fact::Management(ManagementKind::OutAtEntry),
            Signal::CloseAll => Fact::Management(ManagementKind::Close),
            Signal::Cancel => Fact::Management(ManagementKind::Cancel),
            Signal::SetSl { .. } => Fact::Management(ManagementKind::SetSl),
            Signal::BreakEven => Fact::Management(ManagementKind::BreakEven),
            // Correction proves later activity; conservative no-import policy.
            Signal::TpCorrection { .. } => Fact::Management(ManagementKind::ActivationEvidence),
            Signal::Info => Fact::Informational,
        });
    }
    match parser::profit_update_kind(text) {
        ProfitUpdateKind::AtTpProximity => out.push(Fact::Management(ManagementKind::AtTp)),
        ProfitUpdateKind::UnindexedPips | ProfitUpdateKind::RunningPips => {
            out.push(Fact::Management(ManagementKind::PipsHit))
        }
        ProfitUpdateKind::ConfirmedPriceLevel | ProfitUpdateKind::ConfirmedIndexedTp => {
            out.push(Fact::Management(ManagementKind::TpHit))
        }
        ProfitUpdateKind::Other => {}
    }
    // NOT a second trading parser: unrecognized trading-like prose is uncertainty.
    // False positives only prevent import. Never manufacture side/levels/actions.
    if out.iter().all(|f| matches!(f, Fact::Informational)) {
        let upper = text.to_uppercase();
        let suspicious = parser::wyglada_na_wejscie(text)
            || upper.split(|c: char| !c.is_alphanumeric()).any(|w| {
                matches!(
                    w,
                    "BUY"
                        | "SELL"
                        | "LIMIT"
                        | "STOP"
                        | "TP"
                        | "TP1"
                        | "TP2"
                        | "TP3"
                        | "SL"
                        | "PIPS"
                        | "RUNNER"
                        | "RUNNERS"
                        | "CANCEL"
                        | "CANCELLED"
                        | "CLOSE"
                        | "CLOSED"
                        | "FILLED"
                        | "ACTIVATED"
                )
            });
        if suspicious {
            out.push(Fact::UnrecognizedTradingText);
        }
    }
    if out.is_empty() {
        out.push(Fact::Informational);
    }
    out
}

/// Snapshot observation time is the time the whole snapshot was assembled,
/// never publication time. Individual RPC fetch times remain in HistoryRecord.
/// No attempt is made to invent missing pre-edit versions or deletion events.
pub fn adapt(
    snapshot: &HistorySnapshot,
    plans: &BTreeMap<MessageKey, PlannedLimit>,
) -> AdaptedHistory {
    let messages = snapshot
        .records
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let key = MessageKey {
                chat_id: row.chat_id,
                msg_id: row.msg_id,
            };
            let mut parsed = facts(&row.text, plans.get(&key));
            if row.outgoing
                || row.reply_peer_id.is_some_and(|id| id != row.chat_id)
                || row
                    .edited_ms
                    .is_some_and(|ts| ts > snapshot.request.cutoff_ms)
            {
                parsed.push(Fact::UnrecognizedTradingText);
            }
            HistoricalMessage {
                key,
                reply_to: row.reply_to.map(|msg_id| MessageKey {
                    chat_id: row.reply_peer_id.unwrap_or(row.chat_id),
                    msg_id,
                }),
                published_ms: row.published_ms,
                observed_ms: snapshot.completed_ms.max(row.fetched_ms),
                edited_ms: row.edited_ms,
                receive_seq: index as u64 + 1,
                // Collision-free content identity (not a cryptographic hash). Exact
                // text is retained internally; no fabricated historical event hash.
                content_hash: format!("current-text:{}:{}", row.text.len(), row.text),
                facts: parsed,
            }
        })
        .collect();
    AdaptedHistory {
        messages,
        coverage: HistoryCoverage {
            chat_id: snapshot.request.chat_id,
            from_ms: snapshot.request.entry_bounds().map(|b| b.0).unwrap_or(0),
            through_ms: snapshot.request.cutoff_ms,
            all_pages_complete: snapshot.visible_pages_complete && snapshot.reply_parents_complete,
            prior_edits_complete: false,
            deletions_complete: false,
            data_limit_hit: snapshot.data_limit_hit(),
            capture_gap: !snapshot.atomic_at_cutoff || !snapshot.issues.is_empty(),
        },
    }
}

/// Server preview supplies neither a broker plan nor a quote certificate. Thus
/// current history alone can show Rejected/Unknown, never executable eligibility.
pub fn preview(snapshot: HistorySnapshot) -> HistoryPreviewResponse {
    let adapted = adapt(&snapshot, &BTreeMap::new());
    // The pure historical validator examines candidate x management relations.
    // Large/adversarial snapshots must not launch unbounded CPU work. Keeping
    // evidence and explicitly returning UNKNOWN is preferable to dropping it.
    let entries: Vec<_> = adapted
        .messages
        .iter()
        .filter(|m| {
            m.facts.iter().any(|f| {
                matches!(
                    f,
                    Fact::Limit(_) | Fact::LimitWithoutBrokerPlan | Fact::MarketEntry
                )
            })
        })
        .collect();
    if adapted.messages.len() > 10_000
        || entries.len().saturating_mul(adapted.messages.len()) > 10_000_000
    {
        let decisions = entries
            .iter()
            .filter(|m| {
                snapshot
                    .records
                    .iter()
                    .any(|r| r.msg_id == m.key.msg_id && !r.parent_context_only)
            })
            .map(|m| HistoryPreviewDecision {
                chat_id: m.key.chat_id,
                msg_id: m.key.msg_id,
                published_ms: m.published_ms,
                status: "unknown".into(),
                reasons: vec![
                    "PreviewValidationBudgetExceeded".into(),
                    "ReadOnlyNoExecution".into(),
                ],
                evidence_ids: vec![],
            })
            .collect();
        return HistoryPreviewResponse {
            read_only: true,
            can_submit_orders: false,
            snapshot,
            decisions,
        };
    }
    let (from_ms, to_ms) = snapshot.request.entry_bounds().unwrap_or((0, 0));
    let request = ImportRequest {
        experimental_enabled: snapshot.request.experimental_enabled,
        // Evaluate fetched evidence when it actually became available, but keep
        // original entry window. Coverage after cutoff is deliberately unknown.
        startup_ms: snapshot.completed_ms.max(
            snapshot
                .records
                .iter()
                .map(|r| r.fetched_ms)
                .max()
                .unwrap_or(0),
        ),
        today_start_ms: snapshot.request.today_start_ms,
        window: ImportWindow::Custom { from_ms, to_ms },
        expected_broker_identity: "NOT_CONNECTED_READ_ONLY_PREVIEW".into(),
        expected_symbol: "NOT_PLANNED".into(),
        already_imported_or_owned: BTreeSet::new(),
    };
    let decisions = validator::validate(&request, &adapted.messages, &[adapted.coverage], &[])
        .into_iter()
        .filter(|d| {
            snapshot
                .records
                .iter()
                .any(|r| r.msg_id == d.key.msg_id && !r.parent_context_only)
        })
        .map(|d| HistoryPreviewDecision {
            chat_id: d.key.chat_id,
            msg_id: d.key.msg_id,
            published_ms: snapshot
                .records
                .iter()
                .find(|r| r.msg_id == d.key.msg_id)
                .map(|r| r.published_ms)
                .unwrap_or(0),
            status: match d.status {
                Status::Disabled => "disabled",
                Status::Rejected => "rejected",
                Status::Unknown => "unknown",
                Status::EligibleForReview => "eligible_for_review",
            }
            .into(),
            reasons: d
                .reasons
                .iter()
                .map(|r| format!("{r:?}"))
                .chain([
                    "ReadOnlyNoOwnershipDedupCertificate".into(),
                    "CurrentHistoryDoesNotContainPriorEditsOrDeletions".into(),
                ])
                .collect(),
            evidence_ids: d.evidence_ids.iter().map(|key| key.msg_id).collect(),
        })
        .collect();
    HistoryPreviewResponse {
        read_only: true,
        can_submit_orders: false,
        snapshot,
        decisions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conduit_server::history_import::{HistoryFetchRequest, HistoryRecord, HistoryWindow};
    const ENTRY: &str = "🟢 BUY LIMITS GOLD @ 4118/4112 AREA\n🎯 TP1 4122\n🎯 TP2 4127\n⛔️ SL 4104";
    fn record(id: i64, text: &str, parent: Option<i64>) -> HistoryRecord {
        HistoryRecord {
            chat_id: -100123,
            topic_id: None,
            msg_id: id,
            reply_to: parent,
            reply_peer_id: None,
            published_ms: id * 1000,
            edited_ms: None,
            fetched_ms: 10000,
            text: text.into(),
            outgoing: false,
            service_message: false,
            parent_context_only: false,
        }
    }
    fn snapshot(records: Vec<HistoryRecord>) -> HistorySnapshot {
        HistorySnapshot {
            request: HistoryFetchRequest {
                experimental_enabled: true,
                chat_id: -100123,
                topic_id: None,
                cutoff_ms: 9000,
                today_start_ms: 1000,
                window: HistoryWindow::Today,
                max_messages: 100,
                max_pages: 10,
                max_parent_messages: 10,
                max_text_bytes: 100000,
                page_timeout_ms: 100,
                total_timeout_ms: 1000,
            },
            started_ms: 9000,
            completed_ms: 10000,
            records,
            pages_fetched: 1,
            parent_batches_fetched: 0,
            raw_messages_scanned: 0,
            duplicate_records: 0,
            other_topic_records: 0,
            publications_after_cutoff: 0,
            scanned_text_bytes: 0,
            visible_pages_complete: true,
            reply_parents_complete: true,
            prior_edits_complete: false,
            deletions_complete: false,
            atomic_at_cutoff: false,
            library_retry_may_be_hidden: true,
            issues: vec![],
        }
    }
    #[test]
    fn parser_recognizes_actual_limit_but_never_fabricates_broker_plan() {
        let f = facts(ENTRY, None);
        assert!(f.contains(&Fact::LimitWithoutBrokerPlan));
        let p = preview(snapshot(vec![record(1, ENTRY, None)]));
        assert_eq!(p.decisions.len(), 1);
        assert_eq!(p.decisions[0].status, "unknown");
        assert!(p.decisions[0].reasons.contains(&"MissingBrokerPlan".into()));
        assert!(!p.can_submit_orders);
    }
    #[test]
    fn parser_market_entry_rejected() {
        let text = ENTRY.replace("BUY LIMITS", "BUY");
        let p = preview(snapshot(vec![record(1, &text, None)]));
        assert_eq!(p.decisions.len(), 1);
        assert_eq!(p.decisions[0].status, "rejected");
        assert!(p.decisions[0].reasons.contains(&"NotLimit".into()));
    }
    #[test]
    fn final_text_tp_chain_disqualifies_without_claiming_real_fill() {
        let p = preview(snapshot(vec![
            record(1, ENTRY, None),
            record(2, "TP1 HIT", Some(1)),
            record(3, "TP2 HIT", Some(2)),
        ]));
        assert_eq!(p.decisions[0].status, "rejected");
        assert!(p.decisions[0]
            .reasons
            .contains(&"ObservedManagement".into()));
        assert_eq!(p.decisions[0].evidence_ids, vec![2, 3]);
    }
    #[test]
    fn management_fetch_before_parent_still_forms_snapshot_graph() {
        let mut e = record(1, ENTRY, None);
        e.fetched_ms = 11000;
        let mut s = snapshot(vec![e, record(2, "TP1 HIT", Some(1))]);
        s.completed_ms = 11000;
        let p = preview(s);
        assert_eq!(p.decisions[0].status, "rejected");
    }
    #[test]
    fn at_tp_and_running_pips_are_evidence_not_importable_fresh_limits() {
        for text in [
            "AT TP1",
            "+50 PIPS RUNNING",
            "+50 PIPS HIT",
            "RISK FREE",
            "OUT AT ENTRY",
            "SL HIT",
            "CANCEL THIS TRADE",
        ] {
            let p = preview(snapshot(vec![
                record(1, ENTRY, None),
                record(2, text, Some(1)),
            ]));
            assert_ne!(p.decisions[0].status, "eligible_for_review", "{text}");
            assert!(
                facts(text, None)
                    .iter()
                    .any(|f| matches!(f, Fact::Management(_) | Fact::UnrecognizedTradingText)),
                "{text}"
            );
        }
    }
    #[test]
    fn unknown_trading_text_remains_unknown_not_informational_only() {
        let f = facts("FILLED MY LIMIT AFTER A TYPPO", None);
        assert!(f.contains(&Fact::UnrecognizedTradingText));
    }
    #[test]
    fn changed_parser_entry_cannot_use_stale_plan() {
        let parsed = parser::parse(ENTRY)
            .into_iter()
            .find_map(|s| {
                if let Signal::Entry(e) = s {
                    Some(e)
                } else {
                    None
                }
            })
            .unwrap();
        let planned = PlannedLimit {
            parsed_entry: parsed,
            broker_plan: LimitPlan {
                fingerprint: "cert".into(),
                symbol: "XAUUSD".into(),
                side: validator::Side::Buy,
                limits: vec![4118.0, 4112.0],
                sl: 4104.0,
                tp1: 4122.0,
            },
        };
        assert!(facts(ENTRY, Some(&planned))
            .iter()
            .any(|f| matches!(f, Fact::Limit(_))));
        assert!(facts(&ENTRY.replace("4118", "4117"), Some(&planned))
            .contains(&Fact::LimitWithoutBrokerPlan));
    }
    #[test]
    fn cutoff_metadata_not_mapped_to_fake_original_observation() {
        let mut e = record(1, ENTRY, None);
        e.edited_ms = Some(9500);
        let s = snapshot(vec![e]);
        let a = adapt(&s, &BTreeMap::new());
        assert_eq!(a.messages[0].observed_ms, 10000);
        assert_eq!(a.messages[0].edited_ms, Some(9500));
        assert!(!a.coverage.prior_edits_complete);
        assert!(!a.coverage.deletions_complete);
        assert!(a.coverage.capture_gap);
        assert!(a.messages[0].facts.contains(&Fact::UnrecognizedTradingText));
    }
    #[test]
    fn parent_context_never_becomes_preview_candidate() {
        let mut e = record(1, ENTRY, None);
        e.parent_context_only = true;
        let p = preview(snapshot(vec![e, record(2, "TP1 HIT", Some(1))]));
        assert!(p.decisions.is_empty());
    }
}
