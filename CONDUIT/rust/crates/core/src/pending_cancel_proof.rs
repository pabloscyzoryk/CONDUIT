//! Pure, session-only evidence reducer for pending cancellation WITHOUT fills.
//!
//! Not connected to Broker, Engine, RPC, settings, Sim or live trading. It never
//! cancels, places, retries or books anything. Producer completeness assertions
//! are checked for consistency, not authenticated. In particular a local cut
//! is not an atomic freeze of the terminal. A later adapter must provide real
//! order-scoped history and generation proof; an empty orders cache is not one.
//! Quantity units must already be EXACT integer multiples of the stated broker
//! step; conversion from MT5 doubles is an unimplemented producer obligation.
//! A deserialized DTO is not a restored running reducer or consumer checkpoint.

use crate::{broker::ExecutionSession, types::PendingKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct PendingProofSchema(u16);
impl PendingProofSchema {
    pub const V1: Self = Self(1);
}
impl TryFrom<u16> for PendingProofSchema {
    type Error = String;
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        if v == 1 {
            Ok(Self(v))
        } else {
            Err(format!("unsupported pending proof schema {v}"))
        }
    }
}
impl From<PendingProofSchema> for u16 {
    fn from(v: PendingProofSchema) -> Self {
        v.0
    }
}

// Reuse the existing identity type without changing its production serde/API.
// These are only wire projections of ExecutionSession, not another scope ID.
mod session_wire {
    use super::*;
    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Wire {
        scope: String,
        generation: u64,
    }
    pub fn serialize<S: serde::Serializer>(v: &ExecutionSession, s: S) -> Result<S::Ok, S::Error> {
        Wire {
            scope: v.scope.clone(),
            generation: v.generation,
        }
        .serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<ExecutionSession, D::Error> {
        let v = Wire::deserialize(d)?;
        Ok(ExecutionSession {
            scope: v.scope,
            generation: v.generation,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOperationKey {
    #[serde(with = "session_wire")]
    pub session: ExecutionSession,
    pub owner_engine: String,
    pub basket_id: u32,
    pub operation_seq: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingRevision {
    pub source: u64,
    pub policy: u64,
    pub geometry: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOrderBindingV1 {
    pub order_ticket: u64,
    pub symbol: String,
    pub magic: u64,
    pub kind: PendingKind,
    pub level: i32,
    pub is_topup: bool,
    pub is_toucher: bool,
    pub time_setup_msc: i64,
    pub initial_units: u64,
    pub current_units_at_registration: u64,
    pub price: f64,
    pub sl: Option<f64>,
    pub tp: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingCancelIntentV1 {
    pub schema: PendingProofSchema,
    pub key: PendingOperationKey,
    pub revision: PendingRevision,
    /// Full locally committed observation sequence BEFORE sending cancel.
    pub registered_after_observation: u64,
    /// Same declared broker-time basis as order setup/done (not quote time).
    pub registered_before_cancel_msc: i64,
    pub volume_step: f64,
    pub orders: Vec<PendingOrderBindingV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingOrderState {
    Started,
    Placed,
    Canceled,
    Partial,
    Filled,
    Rejected,
    Expired,
    RequestAdd,
    RequestModify,
    RequestCancel,
}
impl PendingOrderState {
    /// Explicit MT5 ENUM_ORDER_STATE mapping; unknown values are not canceled.
    pub fn from_mt5(code: i32) -> Option<Self> {
        Some(match code {
            0 => Self::Started,
            1 => Self::Placed,
            2 => Self::Canceled,
            3 => Self::Partial,
            4 => Self::Filled,
            5 => Self::Rejected,
            6 => Self::Expired,
            7 => Self::RequestAdd,
            8 => Self::RequestModify,
            9 => Self::RequestCancel,
            _ => return None,
        })
    }
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Canceled | Self::Filled | Self::Rejected | Self::Expired
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOrderRecordV1 {
    pub order_ticket: u64,
    pub symbol: String,
    pub magic: u64,
    pub kind: PendingKind,
    pub state: PendingOrderState,
    pub time_setup_msc: i64,
    pub time_done_msc: i64,
    pub position_identifier: u64,
    pub initial_units: u64,
    pub current_units: u64,
    pub price: f64,
    pub sl: Option<f64>,
    pub tp: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingDealV1 {
    pub deal_ticket: u64,
    pub order_ticket: u64,
    pub position_identifier: u64,
    pub symbol: String,
    pub magic: u64,
    /// Actual MT5 DEAL_ENTRY code. Even OUT/INOUT cannot prove an unfilled order.
    pub entry: u8,
    pub volume_units: u64,
    pub time_msc: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EvidenceRead<T> {
    Complete(T),
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOrderEvidenceV1 {
    /// Both reads MUST use this exact ORDER ticket, never a DEAL ticket/date slice.
    pub queried_order_ticket: u64,
    pub history: EvidenceRead<Vec<PendingOrderRecordV1>>,
    pub deals: EvidenceRead<Vec<PendingDealV1>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelatedPositionV1 {
    /// Producer proved this relation through entry deals, not ticket equality.
    pub order_ticket: u64,
    pub position_identifier: u64,
    pub remaining_units: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum PendingReceiptCutV1 {
    Clear {
        required_owner_revision: u64,
        consumed_owner_revision: u64,
    },
    Temporary,
    RequiresReview,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "model", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoricalVolumeConventionV1 {
    /// No native/model evidence: current==0 or current==initial proves nothing.
    Unverified,
    /// Named producer model: history current is the unfilled remainder even
    /// after cancel. A reference is provenance, NOT authentication of the model.
    RemainingUnfilled { evidence_ref: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingCancelObservationV1 {
    pub schema: PendingProofSchema,
    pub key: PendingOperationKey,
    pub revision: PendingRevision,
    pub observation_seq: u64,
    /// Producer's complete history-read boundary in the order timestamp basis.
    pub history_read_through_msc: i64,
    pub identity_confirmed: bool,
    #[serde(with = "session_wire")]
    pub session_before: ExecutionSession,
    #[serde(with = "session_wire")]
    pub session_after: ExecutionSession,
    /// One publication from complete successful reads, not an atomic market cut.
    pub locally_published_complete: bool,
    pub volume_convention: HistoricalVolumeConventionV1,
    pub current_orders: EvidenceRead<Vec<PendingOrderRecordV1>>,
    pub orders: Vec<PendingOrderEvidenceV1>,
    pub related_positions: EvidenceRead<Vec<RelatedPositionV1>>,
    pub receipt_cut: Option<PendingReceiptCutV1>,
}

/// Current trusted runtime context is supplied afresh for every observation.
/// It is deliberately not a persisted resume token.
#[derive(Debug, Clone)]
pub struct PendingProofContext {
    pub session: Option<ExecutionSession>,
    pub owner_engine: String,
    pub basket_id: u32,
    pub revision: PendingRevision,
    pub operation_still_current: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingRegistrationOrigin {
    RegisteredInCurrentSession,
    RestoredUnverified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingWaitReason {
    NotObserved,
    IncompletePublication,
    ReadUnavailable,
    HistoryNotYetVisible,
    OrderStillCurrent,
    OrderNotTerminal,
    ReceiptsNotConsumed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingReviewReason {
    InvalidIntent,
    ColdRestart,
    SessionMismatch,
    RevisionMismatch,
    OwnerMismatch,
    Superseded,
    StaleObservation,
    ConflictingObservation,
    ReadFailed,
    MalformedEvidence,
    WrongOrderScope,
    DuplicateConflict,
    QuantityConflict,
    GeometryConflict,
    UnsupportedVolumeConvention,
    UnsupportedFinalState,
    FillOrPartialObserved,
    ReceiptReview,
    QuantityOverflow,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoFillOrderV1 {
    pub order_ticket: u64,
    pub initial_units: u64,
    /// Historical canceled remainder, NOT current live exposure.
    pub history_remaining_units: u64,
    pub time_done_msc: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingNoFillProofV1 {
    pub schema: PendingProofSchema,
    pub key: PendingOperationKey,
    pub revision: PendingRevision,
    pub observation_seq: u64,
    pub terminal_orders: Vec<NoFillOrderV1>,
    /// Audit only. This is NOT a replacement budget or permission to reopen.
    pub retired_order_units: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    content = "detail",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PendingCancelStateV1 {
    Waiting(PendingWaitReason),
    RequiresReview(PendingReviewReason),
    VerifiedNoFill(PendingNoFillProofV1),
}

/// RAM-only reducer. Review is sticky; no public reset, serde resume or RPC.
pub struct PendingCancelReducerV1 {
    intent: PendingCancelIntentV1,
    last: Option<PendingCancelObservationV1>,
    state: PendingCancelStateV1,
}
impl PendingCancelReducerV1 {
    pub fn new(intent: PendingCancelIntentV1, origin: PendingRegistrationOrigin) -> Self {
        let state = if origin == PendingRegistrationOrigin::RestoredUnverified {
            PendingCancelStateV1::RequiresReview(PendingReviewReason::ColdRestart)
        } else if !valid_intent(&intent) {
            PendingCancelStateV1::RequiresReview(PendingReviewReason::InvalidIntent)
        } else {
            PendingCancelStateV1::Waiting(PendingWaitReason::NotObserved)
        };
        Self {
            intent,
            last: None,
            state,
        }
    }
    pub fn state(&self) -> &PendingCancelStateV1 {
        &self.state
    }
    pub fn observe(
        &mut self,
        current: &PendingProofContext,
        obs: PendingCancelObservationV1,
    ) -> &PendingCancelStateV1 {
        if matches!(self.state, PendingCancelStateV1::RequiresReview(_)) {
            return &self.state;
        }
        let i = &self.intent;
        let invalid = if current.session.as_ref() != Some(&i.key.session)
            || !obs.identity_confirmed
            || obs.session_before != i.key.session
            || obs.session_after != i.key.session
        {
            Some(PendingReviewReason::SessionMismatch)
        } else if current.owner_engine != i.key.owner_engine || current.basket_id != i.key.basket_id
        {
            Some(PendingReviewReason::OwnerMismatch)
        } else if current.revision != i.revision || obs.revision != i.revision {
            Some(PendingReviewReason::RevisionMismatch)
        } else if !current.operation_still_current || obs.key != i.key {
            Some(PendingReviewReason::Superseded)
        } else if obs.observation_seq <= i.registered_after_observation
            || self
                .last
                .as_ref()
                .is_some_and(|v| obs.observation_seq < v.observation_seq)
        {
            Some(PendingReviewReason::StaleObservation)
        } else if self
            .last
            .as_ref()
            .is_some_and(|v| obs.observation_seq == v.observation_seq && &obs != v)
        {
            Some(PendingReviewReason::ConflictingObservation)
        } else {
            None
        };
        if let Some(reason) = invalid {
            self.state = PendingCancelStateV1::RequiresReview(reason);
        } else if self.last.as_ref() != Some(&obs) {
            self.state = evaluate_no_fill(i, &obs);
            self.last = Some(obs);
        }
        &self.state
    }
}

fn positive(v: f64) -> bool {
    v.is_finite() && v > 0.0
}
fn same_price(a: f64, b: f64) -> bool {
    positive(a) && positive(b) && (a - b).abs() <= 4.0 * f64::EPSILON * a.abs().max(b.abs())
}
fn same_optional_price(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_price(a, b),
        _ => false,
    }
}
fn valid_revision(r: PendingRevision) -> bool {
    r.source > 0 && r.policy > 0 && r.geometry > 0
}
fn valid_intent(i: &PendingCancelIntentV1) -> bool {
    let k = &i.key;
    let mut seen = BTreeSet::new();
    !k.session.scope.trim().is_empty()
        && k.session.generation > 0
        && !k.owner_engine.trim().is_empty()
        && k.basket_id > 0
        && k.operation_seq > 0
        && valid_revision(i.revision)
        && i.registered_after_observation > 0
        && i.registered_before_cancel_msc > 0
        && positive(i.volume_step)
        && !i.orders.is_empty()
        && i.orders.iter().all(|o| {
            o.order_ticket > 0
                && seen.insert(o.order_ticket)
                && !o.symbol.trim().is_empty()
                && o.magic > 0
                && o.time_setup_msc > 0
                && o.time_setup_msc <= i.registered_before_cancel_msc
                && o.initial_units > 0
                && o.current_units_at_registration == o.initial_units
                && positive(o.price)
                && o.sl.is_none_or(positive)
                && o.tp.is_none_or(positive)
        })
}
fn check_order(
    o: &PendingOrderRecordV1,
    b: &PendingOrderBindingV1,
) -> Result<(), PendingReviewReason> {
    if o.order_ticket != b.order_ticket
        || o.symbol != b.symbol
        || o.magic != b.magic
        || o.kind != b.kind
    {
        return Err(PendingReviewReason::WrongOrderScope);
    }
    if o.time_setup_msc != b.time_setup_msc
        || o.initial_units != b.initial_units
        || o.current_units > o.initial_units
    {
        return Err(PendingReviewReason::QuantityConflict);
    }
    if !same_price(o.price, b.price)
        || !same_optional_price(o.sl, b.sl)
        || !same_optional_price(o.tp, b.tp)
    {
        return Err(PendingReviewReason::GeometryConflict);
    }
    Ok(())
}
fn read<'a, T>(
    v: &'a EvidenceRead<T>,
    waiting: &mut Option<PendingWaitReason>,
) -> Result<Option<&'a T>, PendingReviewReason> {
    match v {
        EvidenceRead::Complete(v) => Ok(Some(v)),
        EvidenceRead::Unavailable => {
            waiting.get_or_insert(PendingWaitReason::ReadUnavailable);
            Ok(None)
        }
        EvidenceRead::Failed => Err(PendingReviewReason::ReadFailed),
    }
}
fn evaluate_no_fill(
    i: &PendingCancelIntentV1,
    o: &PendingCancelObservationV1,
) -> PendingCancelStateV1 {
    match evaluate_checked(i, o) {
        Ok(s) => s,
        Err(reason) => PendingCancelStateV1::RequiresReview(reason),
    }
}
fn evaluate_checked(
    i: &PendingCancelIntentV1,
    o: &PendingCancelObservationV1,
) -> Result<PendingCancelStateV1, PendingReviewReason> {
    use PendingReviewReason as R;
    let mut waiting = if o.locally_published_complete {
        None
    } else {
        Some(PendingWaitReason::IncompletePublication)
    };
    if o.history_read_through_msc < i.registered_before_cancel_msc {
        return Err(R::StaleObservation);
    }
    match &o.volume_convention {
        HistoricalVolumeConventionV1::RemainingUnfilled { evidence_ref }
            if !evidence_ref.trim().is_empty() => {}
        _ => return Err(R::UnsupportedVolumeConvention),
    }
    let bindings: BTreeMap<_, _> = i.orders.iter().map(|b| (b.order_ticket, b)).collect();
    let mut observed = BTreeMap::new();
    for row in &o.orders {
        if !bindings.contains_key(&row.queried_order_ticket) {
            return Err(R::WrongOrderScope);
        }
        if let Some(prev) = observed.insert(row.queried_order_ticket, row) {
            if prev != row {
                return Err(R::DuplicateConflict);
            }
        }
    }
    if observed.len() != bindings.len() {
        waiting.get_or_insert(PendingWaitReason::ReadUnavailable);
    }
    if let Some(rows) = read(&o.current_orders, &mut waiting)? {
        let mut seen = BTreeMap::new();
        for row in rows {
            if row.order_ticket == 0 {
                return Err(R::MalformedEvidence);
            }
            if let Some(prev) = seen.insert(row.order_ticket, row) {
                if prev != row {
                    return Err(R::DuplicateConflict);
                }
            }
            if let Some(b) = bindings.get(&row.order_ticket) {
                check_order(row, b)?;
                if row.position_identifier != 0
                    || matches!(
                        row.state,
                        PendingOrderState::Partial | PendingOrderState::Filled
                    )
                    || row.current_units != row.initial_units
                {
                    return Err(R::FillOrPartialObserved);
                }
                waiting.get_or_insert(PendingWaitReason::OrderStillCurrent);
            }
        }
    }
    if let Some(rows) = read(&o.related_positions, &mut waiting)? {
        for p in rows {
            if !bindings.contains_key(&p.order_ticket) || p.position_identifier == 0 {
                return Err(R::WrongOrderScope);
            }
            // Even a zero-volume/already-closed related position disproves no-fill.
            return Err(R::FillOrPartialObserved);
        }
    }
    match &o.receipt_cut {
        Some(PendingReceiptCutV1::Clear {
            required_owner_revision,
            consumed_owner_revision,
        }) if *required_owner_revision > 0
            && consumed_owner_revision >= required_owner_revision => {}
        Some(PendingReceiptCutV1::RequiresReview) => return Err(R::ReceiptReview),
        _ => {
            waiting.get_or_insert(PendingWaitReason::ReceiptsNotConsumed);
        }
    }
    let mut terminal_orders = Vec::new();
    let mut retired = 0u64;
    let mut all_deals = BTreeMap::new();
    for (ticket, b) in &bindings {
        let Some(row) = observed.get(ticket) else {
            continue;
        };
        let mut has_deal = false;
        if let Some(deals) = read(&row.deals, &mut waiting)? {
            for d in deals {
                if d.order_ticket != *ticket
                    || d.deal_ticket == 0
                    || d.position_identifier == 0
                    || d.symbol != b.symbol
                    || d.magic != b.magic
                    || d.entry > 3
                    || d.volume_units == 0
                    || d.time_msc < b.time_setup_msc
                {
                    return Err(R::WrongOrderScope);
                }
                if let Some(prev) = all_deals.insert(d.deal_ticket, d) {
                    if prev != d {
                        return Err(R::DuplicateConflict);
                    }
                }
                has_deal = true;
            }
        }
        let Some(history) = read(&row.history, &mut waiting)? else {
            if has_deal {
                return Err(R::FillOrPartialObserved);
            }
            continue;
        };
        let mut record = None;
        for h in history {
            check_order(h, b)?;
            if let Some(prev) = record {
                if prev != h {
                    return Err(R::DuplicateConflict);
                }
            }
            record = Some(h);
        }
        let Some(h) = record else {
            if has_deal {
                return Err(R::FillOrPartialObserved);
            }
            waiting.get_or_insert(PendingWaitReason::HistoryNotYetVisible);
            continue;
        };
        if has_deal
            || h.position_identifier != 0
            || h.state == PendingOrderState::Partial
            || h.state == PendingOrderState::Filled
            || h.current_units != h.initial_units
        {
            return Err(R::FillOrPartialObserved);
        }
        if !h.state.terminal() {
            waiting.get_or_insert(PendingWaitReason::OrderNotTerminal);
            continue;
        }
        if h.state != PendingOrderState::Canceled {
            return Err(R::UnsupportedFinalState);
        }
        if h.time_done_msc < i.registered_before_cancel_msc
            || h.time_done_msc > o.history_read_through_msc
        {
            return Err(R::MalformedEvidence);
        }
        retired = retired
            .checked_add(h.initial_units)
            .ok_or(R::QuantityOverflow)?;
        terminal_orders.push(NoFillOrderV1 {
            order_ticket: *ticket,
            initial_units: h.initial_units,
            history_remaining_units: h.current_units,
            time_done_msc: h.time_done_msc,
        });
    }
    if let Some(reason) = waiting {
        return Ok(PendingCancelStateV1::Waiting(reason));
    }
    if terminal_orders.len() != bindings.len() {
        return Err(R::MalformedEvidence);
    }
    Ok(PendingCancelStateV1::VerifiedNoFill(PendingNoFillProofV1 {
        schema: PendingProofSchema::V1,
        key: i.key.clone(),
        revision: i.revision,
        observation_seq: o.observation_seq,
        terminal_orders,
        retired_order_units: retired,
    }))
}

#[cfg(test)]
#[path = "pending_cancel_proof_tests.rs"]
mod tests;
