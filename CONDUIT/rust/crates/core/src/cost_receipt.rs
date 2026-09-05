//! Pure, versioned closed-cost contract. ClosedTrade may carry this metadata;
//! producer integration is opt-in and does not authorize live cost accounting.
//!
//! This module never reads an account, charges balance, deduplicates broker deals,
//! persists acknowledgements or authorizes an entry. F18A owns position identity
//! and attribution. A future adapter must supply its existing stable account-state
//! scope key; the UI accountSession nonce is deliberately not used here.
//!
//! Incomplete receipts remain incomplete: `net()` returns an error, never gross
//! or zero as a substitute. Adding this unused module changes no strategy mode.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const COST_SCHEMA_VERSION: u16 = 1;
/// Beyond this bound an integer cannot be represented exactly by f64 allocation.
pub const MAX_EXACT_VOLUME_UNITS: u64 = 1 << 53;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct CostSchema(u16);

impl CostSchema {
    pub const V1: Self = Self(COST_SCHEMA_VERSION);
}

impl TryFrom<u16> for CostSchema {
    type Error = CostError;
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value == COST_SCHEMA_VERSION {
            Ok(Self(value))
        } else {
            Err(CostError::UnsupportedSchema(value))
        }
    }
}

impl From<CostSchema> for u16 {
    fn from(value: CostSchema) -> Self {
        value.0
    }
}

/// Absence of a future optional field means LegacySourceDefined, NOT net zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfitBasis {
    LegacySourceDefined,
    CanonicalClosedNetV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostReceiptKey {
    /// Opaque stable key from the existing account-state/F18A adapter. No second
    /// login/server/symbol encoder is introduced by this module.
    pub scope_id: String,
    pub deal_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CostSource {
    BrokerDeals,
    SimulatorLedger {
        run_id: String,
        cost_spec_hash: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostComponent {
    GrossProfit,
    EntryCommission,
    ExitCommission,
    EntryFee,
    ExitFee,
    Swap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CostIssue {
    MissingEntryHistory,
    IncompleteHistoryQuery,
    MissingComponent { component: CostComponent },
    UnsupportedEntryKind { entry: i32 },
    UnresolvedPositionIdentity,
    ConflictingRawDeal,
    UnknownCostAllocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CostCompleteness {
    Complete,
    Incomplete { issues: Vec<CostIssue> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryAllocationMethod {
    ResidualProportionalV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryAllocationProof {
    pub method: EntryAllocationMethod,
    pub cutoff_time_msc: i64,
    pub cutoff_deal_id: u64,
    pub history_query_complete: bool,
    /// Adapter-provided provenance digest. It is not a credential or a proof
    /// manufactured by the calculator that the broker history was complete.
    pub history_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostReceipt {
    pub schema: CostSchema,
    pub key: CostReceiptKey,
    pub position_identifier: u64,
    pub volume: f64,
    pub currency: String,
    pub source: CostSource,
    pub gross_profit: Option<f64>,
    pub entry_commission_alloc: Option<f64>,
    pub exit_commission: Option<f64>,
    pub entry_fee_alloc: Option<f64>,
    pub exit_fee: Option<f64>,
    /// Exact exit-deal swap in live, allocated accrued pool share in a simulator.
    pub swap: Option<f64>,
    pub completeness: CostCompleteness,
    pub entry_allocation: Option<EntryAllocationProof>,
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum CostError {
    #[error("historical profit has no canonical net basis")]
    LegacyBasisUnknown,
    #[error("canonical profit is missing its receipt")]
    MissingReceipt,
    #[error("receipt does not match the closed transche")]
    ReceiptMismatch,
    #[error("unsupported cost schema {0}")]
    UnsupportedSchema(u16),
    #[error("missing stable account scope")]
    EmptyScope,
    #[error("invalid deal identity")]
    InvalidDealId,
    #[error("invalid stable position identity")]
    InvalidPositionIdentifier,
    #[error("invalid closed volume")]
    InvalidVolume,
    #[error("missing account currency")]
    InvalidCurrency,
    #[error("missing simulator run/cost specification identity")]
    InvalidSource,
    #[error("invalid causal entry allocation proof")]
    InvalidAllocationProof,
    #[error("complete broker receipt requires complete entry history proof")]
    MissingAllocationProof,
    #[error("cost receipt is incomplete: {issues:?}")]
    Incomplete { issues: Vec<CostIssue> },
    #[error("missing cost component {0:?}")]
    MissingComponent(CostComponent),
    #[error("nonfinite cost component {0:?}")]
    NonFiniteComponent(CostComponent),
    #[error("cost arithmetic overflow")]
    ArithmeticOverflow,
    #[error("invalid volume unit step")]
    InvalidVolumeStep,
    #[error("invalid number of volume units")]
    InvalidVolumeUnits,
    #[error("volume units exceed exact floating allocation range")]
    VolumeUnitOverflow,
    #[error("exit requests {requested} units, only {available} remain")]
    ExitExceedsPool { requested: u64, available: u64 },
    #[error("empty volume pool has unallocated costs")]
    InvalidPoolState,
}

impl CostReceipt {
    fn components(&self) -> [(CostComponent, Option<f64>); 6] {
        [
            (CostComponent::GrossProfit, self.gross_profit),
            (CostComponent::EntryCommission, self.entry_commission_alloc),
            (CostComponent::ExitCommission, self.exit_commission),
            (CostComponent::EntryFee, self.entry_fee_alloc),
            (CostComponent::ExitFee, self.exit_fee),
            (CostComponent::Swap, self.swap),
        ]
    }

    /// Structural validity allows honest missing fields in an incomplete receipt.
    /// This checks adapter assertions, but does not contact the source to prove them.
    pub fn validate(&self) -> Result<(), CostError> {
        if self.key.scope_id.trim().is_empty() {
            return Err(CostError::EmptyScope);
        }
        if self.key.deal_id == 0 {
            return Err(CostError::InvalidDealId);
        }
        if self.position_identifier == 0 {
            return Err(CostError::InvalidPositionIdentifier);
        }
        if !self.volume.is_finite() || self.volume <= 0.0 {
            return Err(CostError::InvalidVolume);
        }
        if self.currency.trim().is_empty() {
            return Err(CostError::InvalidCurrency);
        }
        if let CostSource::SimulatorLedger {
            run_id,
            cost_spec_hash,
        } = &self.source
        {
            if run_id.trim().is_empty() || cost_spec_hash.trim().is_empty() {
                return Err(CostError::InvalidSource);
            }
        }
        let complete = matches!(self.completeness, CostCompleteness::Complete);
        for (component, value) in self.components() {
            match value {
                Some(v) if !v.is_finite() => return Err(CostError::NonFiniteComponent(component)),
                None if complete => return Err(CostError::MissingComponent(component)),
                _ => {}
            }
        }
        if let Some(proof) = &self.entry_allocation {
            if proof.cutoff_time_msc < 0
                || proof.cutoff_deal_id != self.key.deal_id
                || proof.history_fingerprint.trim().is_empty()
            {
                return Err(CostError::InvalidAllocationProof);
            }
        }
        if complete
            && matches!(self.source, CostSource::BrokerDeals)
            && !self
                .entry_allocation
                .as_ref()
                .is_some_and(|p| p.history_query_complete)
        {
            return Err(CostError::MissingAllocationProof);
        }
        Ok(())
    }

    /// Canonical closed-transche net, using one fixed component order.
    /// No cash is booked and no fallback to gross/zero is permitted.
    pub fn net(&self) -> Result<f64, CostError> {
        self.validate()?;
        if let CostCompleteness::Incomplete { issues } = &self.completeness {
            return Err(CostError::Incomplete {
                issues: issues.clone(),
            });
        }
        let mut sum = 0.0;
        for (component, value) in self.components() {
            sum += value.ok_or(CostError::MissingComponent(component))?;
            if !sum.is_finite() {
                return Err(CostError::ArithmeticOverflow);
            }
        }
        Ok(sum)
    }

    /// Diagnostic subtotal only. No known values => None, never fabricated zero.
    pub fn known_components_sum(&self) -> Result<Option<f64>, CostError> {
        let mut sum = None;
        for (component, value) in self.components() {
            if let Some(v) = value {
                if !v.is_finite() {
                    return Err(CostError::NonFiniteComponent(component));
                }
                let next = sum.unwrap_or(0.0) + v;
                if !next.is_finite() {
                    return Err(CostError::ArithmeticOverflow);
                }
                sum = Some(next);
            }
        }
        Ok(sum)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocatedCosts {
    pub entry_commission: f64,
    pub entry_fee: f64,
    pub swap: f64,
}

/// Explicit serializable snapshot. Deserializing ResidualCostPool validates it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostPoolSnapshot {
    pub schema: CostSchema,
    pub volume_step: f64,
    pub remaining_units: u64,
    pub entry_commission: f64,
    pub entry_fee: f64,
    pub swap: f64,
}

/// Pure pool for already-posted costs. Units are integer broker volume quanta.
/// The adapter validates/converts actual volume; the pool never rounds an order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CostPoolSnapshot", into = "CostPoolSnapshot")]
pub struct ResidualCostPool {
    snapshot: CostPoolSnapshot,
}

impl TryFrom<CostPoolSnapshot> for ResidualCostPool {
    type Error = CostError;
    fn try_from(s: CostPoolSnapshot) -> Result<Self, Self::Error> {
        if !s.volume_step.is_finite() || s.volume_step <= 0.0 {
            return Err(CostError::InvalidVolumeStep);
        }
        if s.remaining_units > MAX_EXACT_VOLUME_UNITS {
            return Err(CostError::VolumeUnitOverflow);
        }
        for (component, value) in [
            (CostComponent::EntryCommission, s.entry_commission),
            (CostComponent::EntryFee, s.entry_fee),
            (CostComponent::Swap, s.swap),
        ] {
            if !value.is_finite() {
                return Err(CostError::NonFiniteComponent(component));
            }
        }
        if s.remaining_units == 0
            && (s.entry_commission != 0.0 || s.entry_fee != 0.0 || s.swap != 0.0)
        {
            return Err(CostError::InvalidPoolState);
        }
        Ok(Self { snapshot: s })
    }
}

impl From<ResidualCostPool> for CostPoolSnapshot {
    fn from(pool: ResidualCostPool) -> Self {
        pool.snapshot
    }
}

impl ResidualCostPool {
    pub fn new(volume_step: f64) -> Result<Self, CostError> {
        Self::try_from(CostPoolSnapshot {
            schema: CostSchema::V1,
            volume_step,
            remaining_units: 0,
            entry_commission: 0.0,
            entry_fee: 0.0,
            swap: 0.0,
        })
    }

    pub fn snapshot(&self) -> CostPoolSnapshot {
        self.snapshot.clone()
    }

    /// Caller has already charged these signed costs to the simulated cash ledger.
    /// Invalid input/overflow leaves every pool field unchanged.
    pub fn add_entry(&mut self, units: u64, commission: f64, fee: f64) -> Result<(), CostError> {
        if units == 0 {
            return Err(CostError::InvalidVolumeUnits);
        }
        if !commission.is_finite() {
            return Err(CostError::NonFiniteComponent(
                CostComponent::EntryCommission,
            ));
        }
        if !fee.is_finite() {
            return Err(CostError::NonFiniteComponent(CostComponent::EntryFee));
        }
        let mut next = self.snapshot();
        next.remaining_units = next
            .remaining_units
            .checked_add(units)
            .ok_or(CostError::VolumeUnitOverflow)?;
        if next.remaining_units > MAX_EXACT_VOLUME_UNITS {
            return Err(CostError::VolumeUnitOverflow);
        }
        next.entry_commission += commission;
        next.entry_fee += fee;
        if !next.entry_commission.is_finite() || !next.entry_fee.is_finite() {
            return Err(CostError::ArithmeticOverflow);
        }
        *self = Self::try_from(next)?;
        Ok(())
    }

    pub fn accrue_swap(&mut self, signed_swap: f64) -> Result<(), CostError> {
        if self.snapshot.remaining_units == 0 {
            return Err(CostError::InvalidVolumeUnits);
        }
        if !signed_swap.is_finite() {
            return Err(CostError::NonFiniteComponent(CostComponent::Swap));
        }
        let next = self.snapshot.swap + signed_swap;
        if !next.is_finite() {
            return Err(CostError::ArithmeticOverflow);
        }
        self.snapshot.swap = next;
        Ok(())
    }

    /// Last exit takes the exact residual costs. No cash/balance mutation.
    pub fn allocate_exit(&mut self, units: u64) -> Result<AllocatedCosts, CostError> {
        if units == 0 {
            return Err(CostError::InvalidVolumeUnits);
        }
        let before = &self.snapshot;
        if units > before.remaining_units {
            return Err(CostError::ExitExceedsPool {
                requested: units,
                available: before.remaining_units,
            });
        }
        let last = units == before.remaining_units;
        let fraction = if last {
            1.0
        } else {
            units as f64 / before.remaining_units as f64
        };
        let allocated = AllocatedCosts {
            entry_commission: before.entry_commission * fraction,
            entry_fee: before.entry_fee * fraction,
            swap: before.swap * fraction,
        };
        let mut next = self.snapshot();
        next.remaining_units -= units;
        if last {
            next.entry_commission = 0.0;
            next.entry_fee = 0.0;
            next.swap = 0.0;
        } else {
            next.entry_commission -= allocated.entry_commission;
            next.entry_fee -= allocated.entry_fee;
            next.swap -= allocated.swap;
        }
        *self = Self::try_from(next)?;
        Ok(allocated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn receipt() -> CostReceipt {
        CostReceipt {
            schema: CostSchema::V1,
            key: CostReceiptKey {
                scope_id: "synthetic-existing-account-scope".into(),
                deal_id: 1001,
            },
            position_identifier: 90,
            volume: 0.01,
            currency: "USD".into(),
            source: CostSource::BrokerDeals,
            gross_profit: Some(0.5),
            entry_commission_alloc: Some(-0.07),
            exit_commission: Some(-0.03),
            entry_fee_alloc: Some(0.0),
            exit_fee: Some(-0.01),
            swap: Some(-0.7582),
            completeness: CostCompleteness::Complete,
            entry_allocation: Some(EntryAllocationProof {
                method: EntryAllocationMethod::ResidualProportionalV1,
                cutoff_time_msc: 1000,
                cutoff_deal_id: 1001,
                history_query_complete: true,
                history_fingerprint: "synthetic-history-proof".into(),
            }),
        }
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-11, "{a} != {b}");
    }
    fn pool(units: u64, commission: f64, fee: f64) -> ResidualCostPool {
        let mut p = ResidualCostPool::new(0.01).unwrap();
        p.add_entry(units, commission, fee).unwrap();
        p
    }

    #[test]
    fn full_signed_net() {
        near(receipt().net().unwrap(), -0.3682);
    }
    #[test]
    fn net_is_pure_and_does_not_sum_flat_cost_aliases_again() {
        let r = receipt();
        let original = r.clone();
        assert_eq!(r.net(), r.net());
        assert_eq!(r, original);
        assert_ne!(
            r.net().unwrap(),
            r.net().unwrap() + r.exit_commission.unwrap() + r.swap.unwrap()
        );
    }
    #[test]
    fn honest_incomplete_can_persist_but_is_not_net() {
        let mut r = receipt();
        r.entry_commission_alloc = None;
        r.completeness = CostCompleteness::Incomplete {
            issues: vec![CostIssue::MissingEntryHistory],
        };
        assert!(r.validate().is_ok());
        near(r.known_components_sum().unwrap().unwrap(), -0.2982);
        assert!(matches!(r.net(), Err(CostError::Incomplete { .. })));
        let restored: CostReceipt =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(restored, r);
        assert!(restored.net().is_err());
    }
    #[test]
    fn all_unknown_is_not_fabricated_zero() {
        let mut r = receipt();
        r.gross_profit = None;
        r.entry_commission_alloc = None;
        r.exit_commission = None;
        r.entry_fee_alloc = None;
        r.exit_fee = None;
        r.swap = None;
        r.completeness = CostCompleteness::Incomplete {
            issues: vec![CostIssue::UnknownCostAllocation],
        };
        assert_eq!(r.known_components_sum().unwrap(), None);
        assert!(r.net().is_err());
    }
    #[test]
    fn claimed_complete_requires_every_component() {
        let mut r = receipt();
        r.exit_fee = None;
        assert_eq!(
            r.net(),
            Err(CostError::MissingComponent(CostComponent::ExitFee))
        );
    }
    #[test]
    fn nonfinite_and_overflow_never_become_net() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut r = receipt();
            r.swap = Some(bad);
            assert!(r.net().is_err());
            assert!(r.known_components_sum().is_err());
        }
        let mut r = receipt();
        r.gross_profit = Some(f64::MAX);
        r.entry_commission_alloc = Some(f64::MAX);
        assert_eq!(r.net(), Err(CostError::ArithmeticOverflow));
    }
    #[test]
    fn schema_v1_roundtrip_rejects_future_or_missing_version() {
        let r = receipt();
        let value = serde_json::to_value(&r).unwrap();
        assert_eq!(value["schema"], 1);
        assert_eq!(
            serde_json::from_value::<CostReceipt>(value.clone()).unwrap(),
            r
        );
        let mut future = value.clone();
        future["schema"] = json!(2);
        assert!(serde_json::from_value::<CostReceipt>(future).is_err());
        let mut absent = value;
        absent.as_object_mut().unwrap().remove("schema");
        assert!(serde_json::from_value::<CostReceipt>(absent).is_err());
    }
    #[test]
    fn broker_complete_requires_adapter_history_evidence() {
        let mut r = receipt();
        r.entry_allocation = None;
        assert_eq!(r.net(), Err(CostError::MissingAllocationProof));
        r = receipt();
        r.entry_allocation.as_mut().unwrap().history_query_complete = false;
        assert_eq!(r.net(), Err(CostError::MissingAllocationProof));
    }
    #[test]
    fn causal_proof_matches_exit_and_has_valid_cutoff() {
        let mut r = receipt();
        r.entry_allocation.as_mut().unwrap().cutoff_deal_id = 1002;
        assert_eq!(r.net(), Err(CostError::InvalidAllocationProof));
        r = receipt();
        r.entry_allocation.as_mut().unwrap().cutoff_time_msc = -1;
        assert_eq!(r.net(), Err(CostError::InvalidAllocationProof));
    }
    #[test]
    fn simulator_requires_run_and_cost_spec_not_fake_broker_proof() {
        let mut r = receipt();
        r.entry_allocation = None;
        r.source = CostSource::SimulatorLedger {
            run_id: "run-A".into(),
            cost_spec_hash: "known-model-A".into(),
        };
        assert!(r.net().is_ok());
        r.source = CostSource::SimulatorLedger {
            run_id: String::new(),
            cost_spec_hash: "x".into(),
        };
        assert_eq!(r.net(), Err(CostError::InvalidSource));
    }
    #[test]
    fn identity_currency_and_volume_are_checked() {
        let mut r = receipt();
        r.key.scope_id = " ".into();
        assert_eq!(r.net(), Err(CostError::EmptyScope));
        r = receipt();
        r.key.deal_id = 0;
        assert_eq!(r.net(), Err(CostError::InvalidDealId));
        r = receipt();
        r.position_identifier = 0;
        assert_eq!(r.net(), Err(CostError::InvalidPositionIdentifier));
        r = receipt();
        r.volume = 0.0;
        assert_eq!(r.net(), Err(CostError::InvalidVolume));
        r = receipt();
        r.currency.clear();
        assert_eq!(r.net(), Err(CostError::InvalidCurrency));
    }
    #[test]
    fn receipts_distinguish_partial_deals_and_account_scopes() {
        let a = receipt().key;
        let mut b = a.clone();
        b.deal_id += 1;
        let mut c = a.clone();
        c.scope_id = "other-account".into();
        let keys: std::collections::HashSet<_> = [a, b, c].into_iter().collect();
        assert_eq!(keys.len(), 3);
    }
    #[test]
    fn zero_cost_and_rebates_are_valid() {
        let mut r = receipt();
        r.gross_profit = Some(-0.1);
        r.entry_commission_alloc = Some(0.1);
        r.exit_commission = Some(0.03);
        r.entry_fee_alloc = Some(0.02);
        r.exit_fee = Some(0.01);
        r.swap = Some(0.2741);
        near(r.net().unwrap(), 0.3341);
        r.gross_profit = Some(0.0);
        r.entry_commission_alloc = Some(0.0);
        r.exit_commission = Some(0.0);
        r.entry_fee_alloc = Some(0.0);
        r.exit_fee = Some(0.0);
        r.swap = Some(0.0);
        assert_eq!(r.net().unwrap(), 0.0);
    }
    #[test]
    fn optional_fixture_preserves_legacy_json_bytes() {
        #[derive(Serialize, Deserialize)]
        struct Fixture {
            ticket: u64,
            profit: f64,
            commission: f64,
            swap: f64,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            profit_basis: Option<ProfitBasis>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            cost_receipt: Option<Box<CostReceipt>>,
        }
        let raw = r#"{"ticket":90,"profit":0.5,"commission":-0.03,"swap":-0.7582}"#;
        let mut f: Fixture = serde_json::from_str(raw).unwrap();
        assert_eq!(serde_json::to_string(&f).unwrap(), raw);
        f.profit_basis = Some(ProfitBasis::CanonicalClosedNetV1);
        f.cost_receipt = Some(Box::new(receipt()));
        f.profit = f.cost_receipt.as_ref().unwrap().net().unwrap();
        let round: Fixture = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        near(round.profit, -0.3682);
        // The workspace serde_json parser can shift an f64 by one ULP on
        // reload. Do not change its global features (legacy semantics) here.
        // A future exact durable consumer checkpoint needs its own bit contract.
        near(round.cost_receipt.unwrap().net().unwrap(), round.profit);
    }
    #[test]
    fn pool_full_allocation_is_exact_residual() {
        let mut p = pool(8, -0.56, -0.08);
        p.accrue_swap(-6.0).unwrap();
        let before = p.snapshot();
        let share = p.allocate_exit(8).unwrap();
        assert_eq!(share.entry_commission, before.entry_commission);
        assert_eq!(share.entry_fee, before.entry_fee);
        assert_eq!(share.swap, before.swap);
        assert_eq!(p.snapshot().remaining_units, 0);
        assert_eq!(p.snapshot().volume_step, 0.01);
    }
    #[test]
    fn pool_odd_volume_partials_conserve_costs() {
        let mut p = pool(7, -0.49, -0.03);
        let a = p.allocate_exit(2).unwrap();
        let b = p.allocate_exit(2).unwrap();
        let before_last = p.snapshot();
        let c = p.allocate_exit(3).unwrap();
        near(
            a.entry_commission + b.entry_commission + c.entry_commission,
            -0.49,
        );
        near(a.entry_fee + b.entry_fee + c.entry_fee, -0.03);
        assert_eq!(c.entry_fee, before_last.entry_fee);
        assert_eq!(p.snapshot().remaining_units, 0);
    }
    #[test]
    fn pool_later_entry_does_not_change_previous_share() {
        let mut p = pool(4, -0.28, 0.0);
        let first = p.allocate_exit(2).unwrap();
        p.add_entry(2, -0.42, 0.0).unwrap();
        near(first.entry_commission, -0.14);
        near(p.allocate_exit(4).unwrap().entry_commission, -0.56);
    }
    #[test]
    fn pool_later_swap_only_belongs_to_remaining_units() {
        let mut p = pool(8, 0.0, 0.0);
        p.accrue_swap(-8.0).unwrap();
        near(p.allocate_exit(4).unwrap().swap, -4.0);
        p.accrue_swap(-4.0).unwrap();
        near(p.allocate_exit(4).unwrap().swap, -8.0);
    }
    #[test]
    fn pool_restart_snapshot_preserves_residue_not_consumer_ack() {
        let mut p = pool(7, -0.49, -0.03);
        p.accrue_swap(-0.7582).unwrap();
        p.allocate_exit(2).unwrap();
        let mut q: ResidualCostPool =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p.allocate_exit(2), q.allocate_exit(2));
        assert_eq!(p.allocate_exit(3), q.allocate_exit(3));
        // This proves pure state recovery, not atomic Engine/consumed-ID persistence.
    }
    #[test]
    fn pool_rejects_malformed_empty_or_future_snapshot() {
        let p = pool(1, -0.07, 0.0);
        let mut value = serde_json::to_value(p).unwrap();
        value["remaining_units"] = json!(0);
        assert!(serde_json::from_value::<ResidualCostPool>(value.clone()).is_err());
        value["remaining_units"] = json!(1);
        value["schema"] = json!(2);
        assert!(serde_json::from_value::<ResidualCostPool>(value).is_err());
    }
    #[test]
    fn pool_bad_exit_is_transactional() {
        let mut p = pool(1, -0.07, 0.0);
        let before = p.clone();
        assert!(p.allocate_exit(0).is_err());
        assert_eq!(p, before);
        assert!(p.allocate_exit(2).is_err());
        assert_eq!(p, before);
    }
    #[test]
    fn pool_bad_entry_and_overflow_are_transactional() {
        let mut p = pool(1, f64::MAX, 0.0);
        let before = p.clone();
        for (units, commission, fee) in [
            (0, 0.0, 0.0),
            (1, f64::NAN, 0.0),
            (1, 0.0, f64::INFINITY),
            (1, f64::MAX, 0.0),
            (u64::MAX, 0.0, 0.0),
        ] {
            assert!(p.add_entry(units, commission, fee).is_err());
            assert_eq!(p, before);
        }
    }
    #[test]
    fn pool_bad_swap_is_transactional() {
        let mut p = pool(1, 0.0, 0.0);
        p.accrue_swap(f64::MAX).unwrap();
        let before = p.clone();
        assert!(p.accrue_swap(f64::MAX).is_err());
        assert_eq!(p, before);
        assert!(p.accrue_swap(f64::NAN).is_err());
        assert_eq!(p, before);
        p.allocate_exit(1).unwrap();
        assert!(p.accrue_swap(0.1).is_err());
    }
    #[test]
    fn pool_validates_step_and_integer_precision_bound() {
        for step in [0.0, -0.01, f64::NAN, f64::INFINITY] {
            assert!(ResidualCostPool::new(step).is_err());
        }
        let mut p = pool(MAX_EXACT_VOLUME_UNITS, 0.0, 0.0);
        let before = p.clone();
        assert_eq!(p.add_entry(1, 0.0, 0.0), Err(CostError::VolumeUnitOverflow));
        assert_eq!(p, before);
    }
}
