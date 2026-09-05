//! Experimental, PURE historical LIMIT eligibility validator.
//!
//! Registered for read-only preview, never connected to the live message sink.
//! It consumes normalized parser facts and certified history/quote coverage.
//! It does not fetch Telegram, open MT5, return orders, or mutate dedup state.
//! `EligibleForReview` is NOT authorization to place orders.
//!
//! Standalone tests need no Cargo/dependencies:
//! rustc --edition=2021 --test history_import.rs -o history_import_tests.exe

use std::collections::{BTreeMap, BTreeSet};

pub const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MessageKey {
    pub chat_id: i64,
    pub msg_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitPlan {
    /// Hash must cover actual planned/rounded broker limits, symbol and all levels.
    pub fingerprint: String,
    pub symbol: String,
    pub side: Side,
    pub limits: Vec<f64>,
    pub sl: f64,
    pub tp1: f64,
}

impl LimitPlan {
    pub fn valid(&self) -> bool {
        if self.fingerprint.is_empty()
            || self.symbol.is_empty()
            || self.limits.is_empty()
            || !self.sl.is_finite()
            || !self.tp1.is_finite()
            || self.sl <= 0.0
            || self.tp1 <= 0.0
            || self.limits.iter().any(|x| !x.is_finite() || *x <= 0.0)
        {
            return false;
        }
        self.limits.iter().all(|level| match self.side {
            Side::Buy => self.sl < *level && *level < self.tp1,
            Side::Sell => self.tp1 < *level && *level < self.sl,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagementKind {
    AtTp,
    TpHit,
    PipsHit,
    Runner,
    RiskFree,
    SecuringPartial,
    BreakEven,
    SetSl,
    OutAtEntry,
    Close,
    Cancel,
    SlHit,
    ActivationEvidence,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Fact {
    Limit(LimitPlan),
    /// Parser recognizes a LIMIT, but no actual rounded broker plan is supplied.
    LimitWithoutBrokerPlan,
    MarketEntry,
    Management(ManagementKind),
    /// Includes a source deletion/tombstone when its identity is known.
    Deleted,
    /// A parser ambiguity must not be silently downgraded to ordinary chat.
    UnrecognizedTradingText,
    Informational,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoricalMessage {
    pub key: MessageKey,
    pub reply_to: Option<MessageKey>,
    pub published_ms: i64,
    pub observed_ms: i64,
    pub edited_ms: Option<i64>,
    pub receive_seq: u64,
    pub content_hash: String,
    /// Adapter must use the real parser plus conservative ambiguity detection.
    pub facts: Vec<Fact>,
}

#[derive(Debug, Clone)]
pub struct HistoryCoverage {
    pub chat_id: i64,
    pub from_ms: i64,
    pub through_ms: i64,
    pub all_pages_complete: bool,
    /// GetHistory's final text + edit_date alone does NOT satisfy this.
    pub prior_edits_complete: bool,
    /// A present-day history page does NOT prove no prior deletion/cancel.
    pub deletions_complete: bool,
    pub data_limit_hit: bool,
    pub capture_gap: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum ImportWindow {
    Today,
    TwoDays,
    ThreeDays,
    OneWeek,
    TwoWeeks,
    FourWeeks,
    All,
    Custom { from_ms: i64, to_ms: i64 },
}

#[derive(Debug, Clone)]
pub struct ImportRequest {
    pub experimental_enabled: bool,
    pub startup_ms: i64,
    /// Caller supplies midnight in the explicitly selected timezone.
    pub today_start_ms: i64,
    pub window: ImportWindow,
    pub expected_broker_identity: String,
    pub expected_symbol: String,
    /// Persisted import IDs AND existing/adopted basket message aliases.
    pub already_imported_or_owned: BTreeSet<MessageKey>,
}

impl ImportRequest {
    fn bounds(&self) -> Option<(i64, i64)> {
        let n = match self.window {
            ImportWindow::Today => {
                return (self.today_start_ms <= self.startup_ms)
                    .then_some((self.today_start_ms, self.startup_ms))
            }
            ImportWindow::TwoDays => 2,
            ImportWindow::ThreeDays => 3,
            ImportWindow::OneWeek => 7,
            ImportWindow::TwoWeeks => 14,
            ImportWindow::FourWeeks => 28,
            ImportWindow::All => return Some((i64::MIN, self.startup_ms)),
            ImportWindow::Custom { from_ms, to_ms } => {
                return (from_ms <= to_ms && to_ms <= self.startup_ms).then_some((from_ms, to_ms))
            }
        };
        Some((self.startup_ms.saturating_sub(n * DAY_MS), self.startup_ms))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Quote {
    pub ts: i64,
    pub bid: f64,
    pub ask: f64,
}

#[derive(Debug, Clone)]
pub struct PriceCoverage {
    pub broker_identity: String,
    pub symbol: String,
    pub from_ms: i64,
    pub through_ms: i64,
    /// Producer certifies every broker tick/page for this interval, including gaps.
    pub complete: bool,
    pub source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriceState {
    Untouched,
    LimitTouched,
    Tp1Crossed,
    SlCrossed,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct BrokerPriceProof {
    key: MessageKey,
    plan_fingerprint: String,
    plan: LimitPlan,
    broker_identity: String,
    symbol: String,
    from_ms: i64,
    through_ms: i64,
    source_hash: String,
    state: PriceState,
    pub ticks_checked: usize,
}

impl BrokerPriceProof {
    pub fn state(&self) -> &PriceState {
        &self.state
    }
}

/// Quote proof checks executable sides, not mid/Bid-only approximations.
/// `coverage.complete` must come from a bounded, fully verified tick fetch.
/// It is NOT inferred merely from first and last timestamps.
pub fn prove_price_path(
    key: MessageKey,
    plan: &LimitPlan,
    publication_ms: i64,
    startup_ms: i64,
    ticks: &[Quote],
    coverage: &PriceCoverage,
) -> BrokerPriceProof {
    let mut proof = BrokerPriceProof {
        key,
        plan_fingerprint: plan.fingerprint.clone(),
        plan: plan.clone(),
        broker_identity: coverage.broker_identity.clone(),
        symbol: coverage.symbol.clone(),
        from_ms: publication_ms,
        through_ms: startup_ms,
        source_hash: coverage.source_hash.clone(),
        state: PriceState::Unknown,
        ticks_checked: 0,
    };
    if !plan.valid()
        || publication_ms > startup_ms
        || !coverage.complete
        || coverage.from_ms > publication_ms
        || coverage.through_ms < startup_ms
        || coverage.symbol != plan.symbol
        || coverage.broker_identity.is_empty()
        || coverage.source_hash.is_empty()
        || ticks.is_empty()
        || ticks[0].ts > publication_ms
        || ticks.iter().any(|q| {
            !q.bid.is_finite()
                || !q.ask.is_finite()
                || q.bid <= 0.0
                || q.ask < q.bid
                || q.ts > startup_ms
        })
        || ticks.windows(2).any(|q| q[1].ts < q[0].ts)
    {
        return proof;
    }
    // At publication the currently known quote is the last quote at/before it.
    let first = ticks.iter().rposition(|q| q.ts <= publication_ms).unwrap();
    let mut touched = false;
    let mut tp = false;
    let mut sl = false;
    for q in &ticks[first..] {
        proof.ticks_checked += 1;
        match plan.side {
            Side::Buy => {
                touched |= plan.limits.iter().any(|level| q.ask <= *level);
                tp |= q.bid >= plan.tp1;
                sl |= q.bid <= plan.sl;
            }
            Side::Sell => {
                touched |= plan.limits.iter().any(|level| q.bid >= *level);
                tp |= q.ask <= plan.tp1;
                sl |= q.ask >= plan.sl;
            }
        }
    }
    proof.state = if touched {
        PriceState::LimitTouched
    } else if tp {
        PriceState::Tp1Crossed
    } else if sl {
        PriceState::SlCrossed
    } else {
        PriceState::Untouched
    };
    proof
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Disabled,
    Rejected,
    Unknown,
    EligibleForReview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    ExperimentalDisabled,
    InvalidRequest,
    OutsideSelectedWindow,
    AlreadyOwnedOrImported,
    NotLimit,
    InvalidPlan,
    ObservedManagement,
    SourceDeleted,
    PlanWasChanged,
    MissingHistoryCoverage,
    IncompletePages,
    UnknownPreviousEdits,
    UnknownDeletions,
    DataLimitReached,
    CaptureGap,
    ConflictingDuplicate,
    InvalidMessageVersion,
    MissingReplyOrAmbiguousManagement,
    UnrecognizedTradingText,
    MissingPriceProof,
    UnknownPricePath,
    WrongBrokerOrSymbol,
    PriceProofDoesNotCoverPlan,
    HistoricalLimitTouched,
    HistoricalTp1Crossed,
    MissingBrokerPlan,
    HistoricalSlCrossed,
    VerifiedUntouchedLimit,
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub key: MessageKey,
    pub status: Status,
    pub reasons: Vec<Reason>,
    pub evidence_ids: Vec<MessageKey>,
    pub plan: Option<LimitPlan>,
}

impl Decision {
    /// There is intentionally no execution authorization in this module.
    pub fn can_submit_orders(&self) -> bool {
        false
    }
}

/// Pure validation. Input is not mutated; decisions never reserve/import IDs.
pub fn validate(
    request: &ImportRequest,
    messages: &[HistoricalMessage],
    coverage: &[HistoryCoverage],
    proofs: &[BrokerPriceProof],
) -> Vec<Decision> {
    let mut by_key: BTreeMap<MessageKey, Vec<&HistoricalMessage>> = BTreeMap::new();
    for msg in messages
        .iter()
        .filter(|m| m.observed_ms <= request.startup_ms && m.published_ms <= request.startup_ms)
    {
        by_key.entry(msg.key).or_default().push(msg);
    }
    for versions in by_key.values_mut() {
        versions.sort_by_key(|m| (m.observed_ms, m.receive_seq));
    }
    // Malformed version metadata in a channel cannot silently remove an old
    // cancellation or move it to a different entry. This is input uncertainty,
    // not evidence that the trade itself activated.
    let invalid_chats: BTreeSet<i64> = by_key
        .iter()
        .filter(|(_, versions)| {
            let publication = versions[0].published_ms;
            versions.iter().any(|m| {
                m.published_ms != publication
                    || m.published_ms > m.observed_ms
                    || m.edited_ms
                        .is_some_and(|t| t < m.published_ms || t > m.observed_ms)
                    || m.content_hash.is_empty()
                    || m.facts.is_empty()
            })
        })
        .map(|(key, _)| key.chat_id)
        .collect();
    let conflicting_chats: BTreeSet<i64> = by_key
        .iter()
        .filter(|(_, versions)| {
            versions.iter().enumerate().any(|(i, m)| {
                versions[..i].iter().any(|old| {
                    old.receive_seq == m.receive_seq
                        && (old.content_hash != m.content_hash
                            || old.facts != m.facts
                            || old.reply_to != m.reply_to)
                })
            })
        })
        .map(|(key, _)| key.chat_id)
        .collect();
    let is_entry = |versions: &Vec<&HistoricalMessage>| {
        versions.iter().any(|m| {
            m.facts.iter().any(|f| {
                matches!(
                    f,
                    Fact::Limit(_) | Fact::LimitWithoutBrokerPlan | Fact::MarketEntry
                )
            })
        })
    };
    let mut result = Vec::new();
    for (&key, versions) in by_key.iter().filter(|(_, versions)| is_entry(versions)) {
        let latest = versions.last().unwrap();
        let plan = latest.facts.iter().find_map(|f| {
            if let Fact::Limit(p) = f {
                Some(p.clone())
            } else {
                None
            }
        });
        let mut decision = Decision {
            key,
            status: Status::Unknown,
            reasons: vec![],
            evidence_ids: vec![],
            plan,
        };
        let finish = |mut d: Decision, status: Status, reason: Reason| {
            d.status = status;
            d.reasons.push(reason);
            d
        };
        if !request.experimental_enabled {
            result.push(finish(
                decision,
                Status::Disabled,
                Reason::ExperimentalDisabled,
            ));
            continue;
        }
        let Some((from, to)) = request.bounds().filter(|_| {
            !request.expected_broker_identity.is_empty() && !request.expected_symbol.is_empty()
        }) else {
            result.push(finish(decision, Status::Unknown, Reason::InvalidRequest));
            continue;
        };
        let publication = versions.iter().map(|m| m.published_ms).min().unwrap();
        if publication < from || publication > to {
            result.push(finish(
                decision,
                Status::Rejected,
                Reason::OutsideSelectedWindow,
            ));
            continue;
        }
        if request.already_imported_or_owned.contains(&key) {
            result.push(finish(
                decision,
                Status::Rejected,
                Reason::AlreadyOwnedOrImported,
            ));
            continue;
        }
        if versions
            .iter()
            .any(|m| m.facts.contains(&Fact::MarketEntry))
        {
            result.push(finish(decision, Status::Rejected, Reason::NotLimit));
            continue;
        }
        if decision.plan.as_ref().is_some_and(|p| !p.valid()) {
            result.push(finish(decision, Status::Unknown, Reason::InvalidPlan));
            continue;
        }
        if versions.iter().any(|m| m.facts.contains(&Fact::Deleted)) {
            result.push(finish(decision, Status::Rejected, Reason::SourceDeleted));
            continue;
        }
        let mut changed = false;
        let mut conflict = false;
        for (i, msg) in versions.iter().enumerate() {
            changed |= msg
                .facts
                .iter()
                .any(|f| matches!(f, Fact::Limit(p) if Some(p) != decision.plan.as_ref()));
            conflict |= versions[..i].iter().any(|old| {
                old.receive_seq == msg.receive_seq
                    && (old.content_hash != msg.content_hash
                        || old.facts != msg.facts
                        || old.reply_to != msg.reply_to)
            });
        }
        if changed {
            decision.reasons.push(Reason::PlanWasChanged);
        }
        if conflict || conflicting_chats.contains(&key.chat_id) {
            decision.reasons.push(Reason::ConflictingDuplicate);
        }
        if invalid_chats.contains(&key.chat_id) {
            decision.reasons.push(Reason::InvalidMessageVersion);
        }
        let mut unresolved = false;
        let mut unknown_text = false;
        // Any observed management in a transitive descendant, even later erased,
        // disqualifies an untouched-history import under the conservative policy.
        for (&message_key, history) in by_key
            .iter()
            .filter(|(other, _)| other.chat_id == key.chat_id)
        {
            let relevant: Vec<_> = history
                .iter()
                .filter(|m| {
                    m.observed_ms >= publication
                        && m.facts.iter().any(|f| {
                            matches!(
                                f,
                                Fact::Management(_) | Fact::Deleted | Fact::UnrecognizedTradingText
                            )
                        })
                })
                .collect();
            if relevant.is_empty() {
                continue;
            }
            for observed in relevant {
                let mut cursor = message_key;
                let mut seen = BTreeSet::new();
                let root = loop {
                    if !seen.insert(cursor) {
                        break None;
                    }
                    let Some(chain) = by_key.get(&cursor) else {
                        break None;
                    };
                    // Every ancestor is read AS OF this management version,
                    // not from its final edited text. A later reply edit must
                    // never retroactively reroute an earlier cancellation.
                    let as_of: Vec<_> = chain
                        .iter()
                        .copied()
                        .filter(|m| {
                            (m.observed_ms, m.receive_seq)
                                <= (observed.observed_ms, observed.receive_seq)
                        })
                        .collect();
                    if as_of.is_empty() {
                        break None;
                    }
                    if is_entry(&as_of) {
                        break Some(cursor);
                    }
                    // The relevant observed message's own reply is version-specific.
                    let parent = if cursor == message_key {
                        observed.reply_to
                    } else {
                        as_of.last().unwrap().reply_to
                    };
                    let Some(parent) = parent else {
                        break None;
                    };
                    if parent.chat_id != key.chat_id {
                        break None;
                    }
                    cursor = parent;
                };
                if root == Some(key) {
                    if observed
                        .facts
                        .iter()
                        .any(|f| matches!(f, Fact::Management(_) | Fact::Deleted))
                    {
                        decision.evidence_ids.push(message_key);
                    }
                    unknown_text |= observed.facts.contains(&Fact::UnrecognizedTradingText);
                } else if root.is_none() {
                    unresolved = true;
                }
            }
        }
        if !decision.evidence_ids.is_empty() {
            decision.evidence_ids.sort();
            decision.evidence_ids.dedup();
            result.push(finish(
                decision,
                Status::Rejected,
                Reason::ObservedManagement,
            ));
            continue;
        }
        if unresolved {
            decision
                .reasons
                .push(Reason::MissingReplyOrAmbiguousManagement);
        }
        if unknown_text {
            decision.reasons.push(Reason::UnrecognizedTradingText);
        }
        match coverage.iter().find(|c| {
            c.chat_id == key.chat_id
                && c.from_ms <= publication
                && c.through_ms >= request.startup_ms
        }) {
            None => decision.reasons.push(Reason::MissingHistoryCoverage),
            Some(c) => {
                if !c.all_pages_complete {
                    decision.reasons.push(Reason::IncompletePages);
                }
                if !c.prior_edits_complete {
                    decision.reasons.push(Reason::UnknownPreviousEdits);
                }
                if !c.deletions_complete {
                    decision.reasons.push(Reason::UnknownDeletions);
                }
                if c.data_limit_hit {
                    decision.reasons.push(Reason::DataLimitReached);
                }
                if c.capture_gap {
                    decision.reasons.push(Reason::CaptureGap);
                }
            }
        }
        let Some(plan) = decision.plan.as_ref() else {
            result.push(finish(decision, Status::Unknown, Reason::MissingBrokerPlan));
            continue;
        };
        let candidates: Vec<_> = proofs.iter().filter(|p| p.key == key).collect();
        if candidates.len() != 1 {
            decision.reasons.push(Reason::MissingPriceProof);
        } else {
            let proof = candidates[0];
            let mut applicable = true;
            if proof.broker_identity != request.expected_broker_identity
                || proof.symbol != request.expected_symbol
                || plan.symbol != request.expected_symbol
            {
                decision.reasons.push(Reason::WrongBrokerOrSymbol);
                applicable = false;
            }
            if proof.plan_fingerprint != plan.fingerprint
                || proof.plan != *plan
                || proof.from_ms > publication
                || proof.through_ms < request.startup_ms
                || proof.source_hash.is_empty()
            {
                decision.reasons.push(Reason::PriceProofDoesNotCoverPlan);
                applicable = false;
            }
            // A different broker or plan is UNKNOWN, even if its own prices
            // crossed a level. Do not attribute that event to this account.
            match &proof.state {
                _ if !applicable => {}
                PriceState::Unknown => decision.reasons.push(Reason::UnknownPricePath),
                PriceState::Untouched => {}
                PriceState::LimitTouched => {
                    result.push(finish(
                        decision,
                        Status::Rejected,
                        Reason::HistoricalLimitTouched,
                    ));
                    continue;
                }
                PriceState::Tp1Crossed => {
                    result.push(finish(
                        decision,
                        Status::Rejected,
                        Reason::HistoricalTp1Crossed,
                    ));
                    continue;
                }
                PriceState::SlCrossed => {
                    result.push(finish(
                        decision,
                        Status::Rejected,
                        Reason::HistoricalSlCrossed,
                    ));
                    continue;
                }
            }
        }
        if decision.reasons.is_empty() {
            decision.status = Status::EligibleForReview;
            decision.reasons.push(Reason::VerifiedUntouchedLimit);
        }
        result.push(decision);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(id: i64) -> MessageKey {
        MessageKey {
            chat_id: -10042,
            msg_id: id,
        }
    }
    fn plan() -> LimitPlan {
        LimitPlan {
            fingerprint: "plan-v1".into(),
            symbol: "XAUUSD".into(),
            side: Side::Buy,
            limits: vec![100.0, 99.0],
            sl: 98.0,
            tp1: 104.0,
        }
    }
    fn entry() -> HistoricalMessage {
        HistoricalMessage {
            key: key(1),
            reply_to: None,
            published_ms: 1000,
            observed_ms: 1000,
            edited_ms: None,
            receive_seq: 1,
            content_hash: "entry1".into(),
            facts: vec![Fact::Limit(plan())],
        }
    }
    fn req() -> ImportRequest {
        ImportRequest {
            experimental_enabled: true,
            startup_ms: 2000,
            today_start_ms: 0,
            window: ImportWindow::All,
            expected_broker_identity: "42@BrokerDemo".into(),
            expected_symbol: "XAUUSD".into(),
            already_imported_or_owned: BTreeSet::new(),
        }
    }
    fn cov() -> HistoryCoverage {
        HistoryCoverage {
            chat_id: key(1).chat_id,
            from_ms: 0,
            through_ms: 2000,
            all_pages_complete: true,
            prior_edits_complete: true,
            deletions_complete: true,
            data_limit_hit: false,
            capture_gap: false,
        }
    }
    fn pcov() -> PriceCoverage {
        PriceCoverage {
            broker_identity: "42@BrokerDemo".into(),
            symbol: "XAUUSD".into(),
            from_ms: 0,
            through_ms: 2000,
            complete: true,
            source_hash: "ticks-sha".into(),
        }
    }
    fn ticks() -> Vec<Quote> {
        vec![
            Quote {
                ts: 1000,
                bid: 101.0,
                ask: 101.2,
            },
            Quote {
                ts: 2000,
                bid: 102.0,
                ask: 102.2,
            },
        ]
    }
    fn proof() -> BrokerPriceProof {
        prove_price_path(key(1), &plan(), 1000, 2000, &ticks(), &pcov())
    }
    fn decision(msg: Vec<HistoricalMessage>, c: HistoryCoverage, p: BrokerPriceProof) -> Decision {
        validate(&req(), &msg, &[c], &[p]).remove(0)
    }
    fn mgmt(id: i64, parent: Option<MessageKey>, fact: Fact) -> HistoricalMessage {
        HistoricalMessage {
            key: key(id),
            reply_to: parent,
            published_ms: 1500,
            observed_ms: 1500,
            edited_ms: None,
            receive_seq: id as u64,
            content_hash: format!("m{id}"),
            facts: vec![fact],
        }
    }
    #[test]
    fn valid_requires_both_histories_and_is_not_an_order() {
        let d = decision(vec![entry()], cov(), proof());
        assert_eq!(d.status, Status::EligibleForReview);
        assert!(!d.can_submit_orders());
    }
    #[test]
    fn experimental_flag_off() {
        let mut r = req();
        r.experimental_enabled = false;
        assert_eq!(
            validate(&r, &[entry()], &[cov()], &[proof()])[0].status,
            Status::Disabled
        );
    }
    #[test]
    fn every_management_kind_disqualifies() {
        for kind in [
            ManagementKind::AtTp,
            ManagementKind::TpHit,
            ManagementKind::PipsHit,
            ManagementKind::Runner,
            ManagementKind::RiskFree,
            ManagementKind::SecuringPartial,
            ManagementKind::BreakEven,
            ManagementKind::SetSl,
            ManagementKind::OutAtEntry,
            ManagementKind::Close,
            ManagementKind::Cancel,
            ManagementKind::SlHit,
            ManagementKind::ActivationEvidence,
        ] {
            let d = decision(
                vec![entry(), mgmt(2, Some(key(1)), Fact::Management(kind))],
                cov(),
                proof(),
            );
            assert_eq!(d.status, Status::Rejected);
            assert!(d.reasons.contains(&Reason::ObservedManagement));
        }
    }
    #[test]
    fn transitive_reply_management() {
        let d = decision(
            vec![
                entry(),
                mgmt(2, Some(key(1)), Fact::Informational),
                mgmt(3, Some(key(2)), Fact::Management(ManagementKind::TpHit)),
            ],
            cov(),
            proof(),
        );
        assert_eq!(d.evidence_ids, vec![key(3)]);
        assert_eq!(d.status, Status::Rejected);
    }
    #[test]
    fn erased_management_is_not_forgotten() {
        let old = mgmt(2, Some(key(1)), Fact::Management(ManagementKind::Cancel));
        let mut new = old.clone();
        new.observed_ms = 1800;
        new.receive_seq = 4;
        new.content_hash = "changed".into();
        new.facts = vec![Fact::Informational];
        assert_eq!(
            decision(vec![entry(), old, new], cov(), proof()).status,
            Status::Rejected
        );
    }
    #[test]
    fn duplicate_same_content_is_idempotent() {
        let mut other = entry();
        other.receive_seq = 9;
        other.observed_ms = 1600;
        let v = validate(&req(), &[entry(), other], &[cov()], &[proof()]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].status, Status::EligibleForReview);
    }
    #[test]
    fn contradictory_duplicate_unknown() {
        let mut other = entry();
        other.content_hash = "different".into();
        assert!(decision(vec![entry(), other], cov(), proof())
            .reasons
            .contains(&Reason::ConflictingDuplicate));
    }
    #[test]
    fn already_owned_skipped() {
        let mut r = req();
        r.already_imported_or_owned.insert(key(1));
        assert_eq!(
            validate(&r, &[entry()], &[cov()], &[proof()])[0].reasons,
            vec![Reason::AlreadyOwnedOrImported]
        );
    }
    #[test]
    fn same_id_other_chat_not_owned() {
        let mut r = req();
        r.already_imported_or_owned.insert(MessageKey {
            chat_id: 7,
            msg_id: 1,
        });
        assert_eq!(
            validate(&r, &[entry()], &[cov()], &[proof()])[0].status,
            Status::EligibleForReview
        );
    }
    #[test]
    fn old_market_edited_to_limit_is_not_unactivated() {
        let mut old = entry();
        old.facts = vec![Fact::MarketEntry];
        let mut new = entry();
        new.observed_ms = 1500;
        new.receive_seq = 2;
        assert_eq!(
            decision(vec![old, new], cov(), proof()).reasons,
            vec![Reason::NotLimit]
        );
    }
    #[test]
    fn changed_plan_needs_segmented_historical_proof() {
        let mut old = entry();
        if let Fact::Limit(p) = &mut old.facts[0] {
            p.limits[0] = 100.5;
        }
        let mut new = entry();
        new.observed_ms = 1500;
        new.receive_seq = 2;
        assert!(decision(vec![old, new], cov(), proof())
            .reasons
            .contains(&Reason::PlanWasChanged));
    }
    #[test]
    fn missing_reply_unknown() {
        assert_eq!(
            decision(
                vec![
                    entry(),
                    mgmt(2, Some(key(999)), Fact::Management(ManagementKind::TpHit))
                ],
                cov(),
                proof()
            )
            .status,
            Status::Unknown
        );
    }
    #[test]
    fn no_reply_management_unknown() {
        assert_eq!(
            decision(
                vec![
                    entry(),
                    mgmt(2, None, Fact::Management(ManagementKind::TpHit))
                ],
                cov(),
                proof()
            )
            .status,
            Status::Unknown
        );
    }
    #[test]
    fn reply_cycle_unknown_not_hang() {
        assert_eq!(
            decision(
                vec![
                    entry(),
                    mgmt(2, Some(key(3)), Fact::Informational),
                    mgmt(3, Some(key(2)), Fact::Management(ManagementKind::TpHit))
                ],
                cov(),
                proof()
            )
            .status,
            Status::Unknown
        );
    }
    #[test]
    fn unrelated_known_entry_management_does_not_poison_candidate() {
        let mut e2 = entry();
        e2.key = key(10);
        let v = validate(
            &req(),
            &[
                entry(),
                e2,
                mgmt(11, Some(key(10)), Fact::Management(ManagementKind::TpHit)),
            ],
            &[cov()],
            &[proof()],
        );
        assert_eq!(v[0].status, Status::EligibleForReview);
        assert_eq!(v[1].status, Status::Rejected);
    }
    #[test]
    fn full_history_requirements_independent() {
        for n in 0..6 {
            let mut c = cov();
            match n {
                0 => c.all_pages_complete = false,
                1 => c.prior_edits_complete = false,
                2 => c.deletions_complete = false,
                3 => c.data_limit_hit = true,
                4 => c.capture_gap = true,
                _ => c.from_ms = 1001,
            }
            assert_eq!(decision(vec![entry()], c, proof()).status, Status::Unknown);
        }
    }
    #[test]
    fn malformed_plan_unknown() {
        let mut e = entry();
        if let Fact::Limit(p) = &mut e.facts[0] {
            p.sl = f64::NAN;
        }
        assert_eq!(
            decision(vec![e], cov(), proof()).reasons,
            vec![Reason::InvalidPlan]
        );
    }
    #[test]
    fn deletion_rejected() {
        let mut e = entry();
        e.facts.push(Fact::Deleted);
        assert_eq!(
            decision(vec![e], cov(), proof()).reasons,
            vec![Reason::SourceDeleted]
        );
    }
    #[test]
    fn parser_ambiguity_unknown() {
        assert_eq!(
            decision(
                vec![
                    entry(),
                    mgmt(2, Some(key(1)), Fact::UnrecognizedTradingText)
                ],
                cov(),
                proof()
            )
            .status,
            Status::Unknown
        );
    }
    #[test]
    fn no_price_proof_unknown() {
        assert!(validate(&req(), &[entry()], &[cov()], &[])[0]
            .reasons
            .contains(&Reason::MissingPriceProof));
    }
    #[test]
    fn wrong_symbol_or_broker_unknown() {
        let mut p = proof();
        p.broker_identity = "42@OtherLive".into();
        assert!(decision(vec![entry()], cov(), p)
            .reasons
            .contains(&Reason::WrongBrokerOrSymbol));
        let mut p = proof();
        p.symbol = "XAUUSD.s".into();
        assert!(decision(vec![entry()], cov(), p)
            .reasons
            .contains(&Reason::WrongBrokerOrSymbol));
    }
    #[test]
    fn forged_or_stale_plan_proof_unknown() {
        let mut p = proof();
        p.plan_fingerprint = "other".into();
        assert!(decision(vec![entry()], cov(), p)
            .reasons
            .contains(&Reason::PriceProofDoesNotCoverPlan));
    }
    #[test]
    fn buy_limit_uses_ask_not_bid() {
        let q = [Quote {
            ts: 1000,
            bid: 99.9,
            ask: 100.1,
        }];
        let p = prove_price_path(key(1), &plan(), 1000, 2000, &q, &pcov());
        assert_eq!(p.state, PriceState::Untouched);
    }
    #[test]
    fn buy_touch_equal_is_activation() {
        let q = [Quote {
            ts: 1000,
            bid: 99.8,
            ask: 100.0,
        }];
        let p = prove_price_path(key(1), &plan(), 1000, 2000, &q, &pcov());
        assert_eq!(p.state, PriceState::LimitTouched);
        assert_eq!(decision(vec![entry()], cov(), p).status, Status::Rejected);
    }
    #[test]
    fn tp1_without_entry_still_disqualifies() {
        let q = [Quote {
            ts: 1000,
            bid: 104.0,
            ask: 104.2,
        }];
        let p = prove_price_path(key(1), &plan(), 1000, 2000, &q, &pcov());
        assert_eq!(p.state, PriceState::Tp1Crossed);
        assert_eq!(decision(vec![entry()], cov(), p).status, Status::Rejected);
    }
    #[test]
    fn sell_limit_uses_bid() {
        let mut p = plan();
        p.side = Side::Sell;
        p.limits = vec![104.0, 105.0];
        p.tp1 = 100.0;
        p.sl = 106.0;
        let q = [Quote {
            ts: 1000,
            bid: 103.9,
            ask: 104.1,
        }];
        assert_eq!(
            prove_price_path(key(1), &p, 1000, 2000, &q, &pcov()).state,
            PriceState::Untouched
        );
        let q = [Quote {
            ts: 1000,
            bid: 104.0,
            ask: 104.2,
        }];
        assert_eq!(
            prove_price_path(key(1), &p, 1000, 2000, &q, &pcov()).state,
            PriceState::LimitTouched
        );
    }
    #[test]
    fn first_last_quotes_do_not_prove_no_gap() {
        let mut c = pcov();
        c.complete = false;
        assert_eq!(
            prove_price_path(key(1), &plan(), 1000, 2000, &ticks(), &c).state,
            PriceState::Unknown
        );
    }
    #[test]
    fn future_or_unsorted_quotes_unknown() {
        let mut q = ticks();
        q[1].ts = 2001;
        assert_eq!(
            prove_price_path(key(1), &plan(), 1000, 2000, &q, &pcov()).state,
            PriceState::Unknown
        );
        let mut q = ticks();
        q.reverse();
        assert_eq!(
            prove_price_path(key(1), &plan(), 1000, 2000, &q, &pcov()).state,
            PriceState::Unknown
        );
    }
    #[test]
    fn no_quote_at_or_before_publication_unknown() {
        let mut q = ticks();
        q[0].ts = 1001;
        assert_eq!(
            prove_price_path(key(1), &plan(), 1000, 2000, &q, &pcov()).state,
            PriceState::Unknown
        );
    }
    #[test]
    fn malformed_quote_unknown() {
        let q = [Quote {
            ts: 1000,
            bid: 102.0,
            ask: 101.0,
        }];
        assert_eq!(
            prove_price_path(key(1), &plan(), 1000, 2000, &q, &pcov()).state,
            PriceState::Unknown
        );
    }
    #[test]
    fn all_windows_have_explicit_cutoffs() {
        let mut r = req();
        r.startup_ms = 30 * DAY_MS;
        r.today_start_ms = r.startup_ms - 1000;
        for (w, n) in [
            (ImportWindow::TwoDays, 2),
            (ImportWindow::ThreeDays, 3),
            (ImportWindow::OneWeek, 7),
            (ImportWindow::TwoWeeks, 14),
            (ImportWindow::FourWeeks, 28),
        ] {
            r.window = w;
            assert_eq!(r.bounds(), Some(((30 - n) * DAY_MS, 30 * DAY_MS)));
        }
        r.window = ImportWindow::Today;
        assert_eq!(r.bounds(), Some((r.today_start_ms, r.startup_ms)));
        r.window = ImportWindow::All;
        assert_eq!(r.bounds(), Some((i64::MIN, r.startup_ms)));
        r.window = ImportWindow::Custom {
            from_ms: 20,
            to_ms: 10,
        };
        assert!(r.bounds().is_none());
    }
    #[test]
    fn custom_end_does_not_shorten_management_validation() {
        let mut r = req();
        r.window = ImportWindow::Custom {
            from_ms: 500,
            to_ms: 1200,
        };
        let v = validate(
            &r,
            &[
                entry(),
                mgmt(2, Some(key(1)), Fact::Management(ManagementKind::TpHit)),
            ],
            &[cov()],
            &[proof()],
        );
        assert_eq!(v[0].status, Status::Rejected);
    }
    #[test]
    fn future_management_not_used_as_evidence() {
        let mut m = mgmt(2, Some(key(1)), Fact::Management(ManagementKind::TpHit));
        m.observed_ms = 2001;
        assert_eq!(
            decision(vec![entry(), m], cov(), proof()).status,
            Status::EligibleForReview
        );
    }
    #[test]
    fn ancestor_reply_edit_cannot_rewrite_old_management() {
        let mut other = entry();
        other.key = key(10);
        let mut parent = mgmt(2, Some(key(1)), Fact::Informational);
        parent.published_ms = 1200;
        parent.observed_ms = 1200;
        let old_hit = mgmt(3, Some(key(2)), Fact::Management(ManagementKind::TpHit));
        let mut changed = parent.clone();
        changed.observed_ms = 1800;
        changed.edited_ms = Some(1800);
        changed.receive_seq = 12;
        changed.reply_to = Some(key(10));
        changed.content_hash = "redirect".into();
        let other_proof = prove_price_path(key(10), &plan(), 1000, 2000, &ticks(), &pcov());
        let v = validate(
            &req(),
            &[entry(), other, parent, old_hit, changed],
            &[cov()],
            &[proof(), other_proof],
        );
        assert_eq!(v[0].status, Status::Rejected);
        assert_eq!(v[0].evidence_ids, vec![key(3)]);
        assert_eq!(v[1].status, Status::EligibleForReview);
    }
    #[test]
    fn ancestor_not_observed_at_event_time_is_unknown() {
        let mut parent = mgmt(2, Some(key(1)), Fact::Informational);
        parent.observed_ms = 1800;
        let hit = mgmt(3, Some(key(2)), Fact::Management(ManagementKind::TpHit));
        let d = decision(vec![entry(), parent, hit], cov(), proof());
        assert_eq!(d.status, Status::Unknown);
        assert!(d
            .reasons
            .contains(&Reason::MissingReplyOrAmbiguousManagement));
    }
    #[test]
    fn future_or_prepublication_edit_metadata_is_unknown() {
        for edit in [999, 2001] {
            let mut e = entry();
            e.edited_ms = Some(edit);
            let d = decision(vec![e], cov(), proof());
            assert_eq!(d.status, Status::Unknown);
            assert!(d.reasons.contains(&Reason::InvalidMessageVersion));
        }
    }
    #[test]
    fn altered_publication_time_is_unknown() {
        let mut edited = entry();
        edited.observed_ms = 1500;
        edited.published_ms = 1400;
        edited.receive_seq = 2;
        let d = decision(vec![entry(), edited], cov(), proof());
        assert_eq!(d.status, Status::Unknown);
        assert!(d.reasons.contains(&Reason::InvalidMessageVersion));
    }
    #[test]
    fn price_proof_binds_levels_not_just_caller_hash() {
        let mut other_plan = plan();
        other_plan.limits = vec![100.5];
        // Deliberately retain the same claimed fingerprint: complete plan equality wins.
        let p = prove_price_path(key(1), &other_plan, 1000, 2000, &ticks(), &pcov());
        let d = decision(vec![entry()], cov(), p);
        assert_eq!(d.status, Status::Unknown);
        assert!(d.reasons.contains(&Reason::PriceProofDoesNotCoverPlan));
    }
    #[test]
    fn foreign_broker_touch_is_not_attributed_to_this_broker() {
        let q = [Quote {
            ts: 1000,
            bid: 99.8,
            ask: 100.0,
        }];
        let mut c = pcov();
        c.broker_identity = "Other@Broker".into();
        let p = prove_price_path(key(1), &plan(), 1000, 2000, &q, &c);
        assert_eq!(p.state(), &PriceState::LimitTouched);
        let d = decision(vec![entry()], cov(), p);
        assert_eq!(d.status, Status::Unknown);
        assert!(!d.reasons.contains(&Reason::HistoricalLimitTouched));
    }
    #[test]
    fn duplicate_price_proofs_are_unknown() {
        let d = &validate(&req(), &[entry()], &[cov()], &[proof(), proof()])[0];
        assert_eq!(d.status, Status::Unknown);
        assert!(d.reasons.contains(&Reason::MissingPriceProof));
    }
    #[test]
    fn intermediate_conflicting_duplicate_cannot_hide_management() {
        let mut e2 = entry();
        e2.key = key(10);
        let parent = mgmt(2, Some(key(1)), Fact::Informational);
        let mut conflict = parent.clone();
        conflict.reply_to = Some(key(10));
        let hit = mgmt(3, Some(key(2)), Fact::Management(ManagementKind::TpHit));
        let d = decision(vec![entry(), e2, parent, conflict, hit], cov(), proof());
        assert_ne!(d.status, Status::EligibleForReview);
        assert!(d.reasons.contains(&Reason::ConflictingDuplicate));
    }
    #[test]
    fn source_before_observation_is_required() {
        let mut e = entry();
        e.observed_ms = 999;
        assert!(decision(vec![e], cov(), proof())
            .reasons
            .contains(&Reason::InvalidMessageVersion));
    }
}
