//! Pure nominal RF-floor evaluator, NOT connected to Engine or live trading.
//!
//! An adapter supplies independently confirmed exposure and valued stop legs.
//! This module checks their declared scope/revisions/completeness; it cannot
//! authenticate an adapter assertion or contact a broker. RPC acceptance and
//! optimistic cache are deliberately not accepted as confirmed stop evidence.
//! No price path, gap protection, cash booking, retries or strategy choices are
//! implemented here. `VerifiedNominal` is conditional on the stated valuation
//! model, not a lower bound guaranteed for future real executions or fees.

use crate::cost_receipt::ProfitBasis;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const RF_FLOOR_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct RfFloorSchema(u16);
impl RfFloorSchema {
    pub const V1: Self = Self(RF_FLOOR_SCHEMA_VERSION);
}
impl TryFrom<u16> for RfFloorSchema {
    type Error = String;
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        if v == RF_FLOOR_SCHEMA_VERSION {
            Ok(Self(v))
        } else {
            Err(format!("unsupported RF floor schema {v}"))
        }
    }
}
impl From<RfFloorSchema> for u16 {
    fn from(v: RfFloorSchema) -> Self {
        v.0
    }
}

/// Reuses the adapter's opaque account-state scope; never encode login/server
/// again or substitute the UI session nonce. Revisions are nonzero producer
/// tokens, not broker quote timestamps. A changed policy invalidates old proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RfOwner {
    pub scope_id: String,
    pub session_generation: u64,
    pub basket_id: u32,
    pub setup_revision: u64,
    pub policy_revision: u64,
}
impl RfOwner {
    fn valid(&self) -> bool {
        !self.scope_id.trim().is_empty()
            && self.session_generation > 0
            && self.basket_id > 0
            && self.setup_revision > 0
            && self.policy_revision > 0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RfStopEvidence {
    /// Actual observed price, including any broker normalization. No new modify
    /// is required if an already existing tighter stop has this same proof.
    Confirmed { price: f64, snapshot_revision: u64 },
    /// An authoritative observation proves that no broker SL exists (StopOff).
    Absent,
    /// Includes rejected requests and optimistic cache; not zero or no-stop.
    Unconfirmed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RfPosition {
    pub owner: RfOwner,
    pub position_identifier: u64,
    pub remaining_volume: f64,
    pub stop: RfStopEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RfPendingEvidence {
    /// A fresh empty exposure projection WITH reconciled cancellation/fill
    /// races. A cancel ACK or an empty stale orders cache cannot construct it.
    NoneConfirmed {
        snapshot_revision: u64,
        cancellation_and_fills_reconciled: bool,
    },
    /// V1 does not value possible pending fills; known pending risk is unsecured.
    Present,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RfSnapshot {
    pub owner: RfOwner,
    pub revision: u64,
    pub authoritative: bool,
    /// Includes all owned positions, including frozen/unmanaged ones. The
    /// producer must not silently send only the chosen runners.
    pub all_owned_positions_complete: bool,
    pub positions: Vec<RfPosition>,
    pub pending: RfPendingEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RfRealized {
    pub owner: RfOwner,
    pub ledger_revision: u64,
    pub currency: String,
    pub profit_basis: ProfitBasis,
    /// All owned close receipts through required_ledger_revision were credited
    /// by the owner exactly once. The calculator does not perform that credit.
    pub receipts_complete_and_consumed: bool,
    pub net: Option<f64>,
}

/// Signed remaining-position costs, NOT amounts already allocated to closed
/// tranches. Zero must be explicit. Exit values are declared MODEL inputs, not
/// a promise about future broker tariffs. No implicit XAU size/currency exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RfResidualCosts {
    pub entry_commission: Option<f64>,
    pub entry_fee: Option<f64>,
    pub accrued_swap: Option<f64>,
    pub modelled_exit_commission: Option<f64>,
    pub modelled_exit_fee: Option<f64>,
}
impl RfResidualCosts {
    fn values(&self) -> [Option<f64>; 5] {
        [
            self.entry_commission,
            self.entry_fee,
            self.accrued_swap,
            self.modelled_exit_commission,
            self.modelled_exit_fee,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RfLegValuation {
    pub owner: RfOwner,
    pub position_identifier: u64,
    pub snapshot_revision: u64,
    /// Binds the remaining entry-cost allocation to the same closed ledger.
    pub ledger_revision: u64,
    pub currency: String,
    pub model_id: String,
    pub for_volume: f64,
    pub at_stop_price: f64,
    pub allocation_complete: bool,
    /// Gross profit for EXACTLY for_volume at at_stop_price, calculated by the
    /// named valuation model. Do not supply per-lot profit or current floating.
    pub gross_profit_at_stop: Option<f64>,
    pub costs: RfResidualCosts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RfFloorInput {
    pub schema: RfFloorSchema,
    pub expected_owner: RfOwner,
    pub currency: String,
    pub valuation_model_id: String,
    pub minimum_snapshot_revision: u64,
    pub required_ledger_revision: u64,
    pub snapshot: RfSnapshot,
    pub realized: RfRealized,
    pub legs: Vec<RfLegValuation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RfIncomplete {
    InvalidContext,
    OwnerMismatch,
    SnapshotNotAuthoritative,
    SnapshotStale,
    PositionInventoryIncomplete,
    InvalidPosition { position_identifier: u64 },
    DuplicatePosition { position_identifier: u64 },
    PendingUnconfirmed,
    StopUnconfirmed { position_identifier: u64 },
    StopStaleOrInvalid { position_identifier: u64 },
    RealizedNotCanonicalOrUnconsumed,
    RealizedRevisionMismatch,
    RealizedMissingOrNonfinite,
    CurrencyMismatch,
    UnexpectedValuation { position_identifier: u64 },
    DuplicateValuation { position_identifier: u64 },
    MissingValuation { position_identifier: u64 },
    ValuationBindingMismatch { position_identifier: u64 },
    ValuationOrCostIncomplete { position_identifier: u64 },
    ArithmeticOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RfUnsecured {
    NegativeNominalFloor,
    NoBrokerStop { position_identifier: u64 },
    PendingExposureNotValued,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RfNominalFloor {
    pub owner: RfOwner,
    pub currency: String,
    pub valuation_model_id: String,
    pub snapshot_revision: u64,
    pub ledger_revision: u64,
    pub realized_net: f64,
    pub remaining_gross_at_stops: f64,
    pub remaining_signed_costs: f64,
    pub nominal_net: f64,
    pub position_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RfFloorOutcome {
    VerifiedNominal {
        calculation: RfNominalFloor,
    },
    Unsecured {
        calculation: Option<RfNominalFloor>,
        reasons: Vec<RfUnsecured>,
    },
    /// No numeric partial sum is exposed as a floor when evidence is incomplete.
    Incomplete {
        issues: Vec<RfIncomplete>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RfFloorResult {
    pub schema: RfFloorSchema,
    pub outcome: RfFloorOutcome,
}

fn result(outcome: RfFloorOutcome) -> RfFloorResult {
    RfFloorResult {
        schema: RfFloorSchema::V1,
        outcome,
    }
}
fn finite(v: Option<f64>) -> bool {
    v.is_some_and(f64::is_finite)
}
fn positive(v: f64) -> bool {
    v.is_finite() && v > 0.0
}
fn currency_valid(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase())
}

/// No mutations, I/O, hidden fees, price guesses or brokerage-specific math.
/// Declared proof is validated, not manufactured. Numeric valuation comes from
/// the named producer; its economic correctness still needs adapter tests.
pub fn evaluate_nominal_floor(input: &RfFloorInput) -> RfFloorResult {
    use RfIncomplete as I;
    let mut issues = Vec::new();
    let mut unsecured = Vec::new();
    let owner = &input.expected_owner;
    let snapshot = &input.snapshot;
    let realized = &input.realized;
    if !owner.valid()
        || !currency_valid(&input.currency)
        || input.valuation_model_id.trim().is_empty()
        || input.minimum_snapshot_revision == 0
        || input.required_ledger_revision == 0
    {
        issues.push(I::InvalidContext);
    }
    if snapshot.owner != *owner || realized.owner != *owner {
        issues.push(I::OwnerMismatch);
    }
    if !snapshot.authoritative {
        issues.push(I::SnapshotNotAuthoritative);
    }
    if snapshot.revision < input.minimum_snapshot_revision || snapshot.revision == 0 {
        issues.push(I::SnapshotStale);
    }
    if !snapshot.all_owned_positions_complete {
        issues.push(I::PositionInventoryIncomplete);
    }
    match snapshot.pending {
        RfPendingEvidence::NoneConfirmed {
            snapshot_revision,
            cancellation_and_fills_reconciled,
        } if snapshot_revision == snapshot.revision && cancellation_and_fills_reconciled => {}
        RfPendingEvidence::Present => unsecured.push(RfUnsecured::PendingExposureNotValued),
        _ => issues.push(I::PendingUnconfirmed),
    }
    if realized.profit_basis != ProfitBasis::CanonicalClosedNetV1
        || !realized.receipts_complete_and_consumed
    {
        issues.push(I::RealizedNotCanonicalOrUnconsumed);
    }
    if realized.ledger_revision != input.required_ledger_revision {
        issues.push(I::RealizedRevisionMismatch);
    }
    if realized.currency != input.currency {
        issues.push(I::CurrencyMismatch);
    }
    if !finite(realized.net) {
        issues.push(I::RealizedMissingOrNonfinite);
    }

    let mut positions = BTreeMap::new();
    for p in &snapshot.positions {
        let id = p.position_identifier;
        if p.owner != *owner {
            issues.push(I::OwnerMismatch);
        }
        if id == 0 || !positive(p.remaining_volume) {
            issues.push(I::InvalidPosition {
                position_identifier: id,
            });
        }
        if positions.insert(id, p).is_some() {
            issues.push(I::DuplicatePosition {
                position_identifier: id,
            });
        }
        match p.stop {
            RfStopEvidence::Confirmed {
                price,
                snapshot_revision,
            } => {
                if !positive(price) || snapshot_revision != snapshot.revision {
                    issues.push(I::StopStaleOrInvalid {
                        position_identifier: id,
                    });
                }
            }
            RfStopEvidence::Absent => unsecured.push(RfUnsecured::NoBrokerStop {
                position_identifier: id,
            }),
            RfStopEvidence::Unconfirmed => issues.push(I::StopUnconfirmed {
                position_identifier: id,
            }),
        }
    }
    let mut legs = BTreeMap::new();
    let mut valued = BTreeSet::new();
    for leg in &input.legs {
        let id = leg.position_identifier;
        if legs.insert(id, leg).is_some() {
            issues.push(I::DuplicateValuation {
                position_identifier: id,
            });
        }
        if !positions.contains_key(&id) {
            issues.push(I::UnexpectedValuation {
                position_identifier: id,
            });
        }
        if leg.owner != *owner {
            issues.push(I::OwnerMismatch);
        }
        if leg.currency != input.currency {
            issues.push(I::CurrencyMismatch);
        }
        if leg.model_id != input.valuation_model_id
            || leg.snapshot_revision != snapshot.revision
            || leg.ledger_revision != realized.ledger_revision
            || !positive(leg.for_volume)
            || !positive(leg.at_stop_price)
        {
            issues.push(I::ValuationBindingMismatch {
                position_identifier: id,
            });
        }
        if !leg.allocation_complete
            || !finite(leg.gross_profit_at_stop)
            || !leg.costs.values().into_iter().all(finite)
        {
            issues.push(I::ValuationOrCostIncomplete {
                position_identifier: id,
            });
        }
    }
    for (&id, p) in &positions {
        if let RfStopEvidence::Confirmed { price, .. } = p.stop {
            match legs.get(&id) {
                None => issues.push(I::MissingValuation {
                    position_identifier: id,
                }),
                Some(leg) => {
                    // Exact binding, not a tolerance that could reuse the old
                    // full-volume valuation after a partial. Producer canonicalizes.
                    if leg.for_volume != p.remaining_volume || leg.at_stop_price != price {
                        issues.push(I::ValuationBindingMismatch {
                            position_identifier: id,
                        });
                    }
                    valued.insert(id);
                }
            }
        }
    }
    if !issues.is_empty() {
        return result(RfFloorOutcome::Incomplete { issues });
    }
    if !unsecured.is_empty() {
        return result(RfFloorOutcome::Unsecured {
            calculation: None,
            reasons: unsecured,
        });
    }

    // Deterministic identifier order. No epsilon can turn a negative floor into
    // a zero; adapters must specify their currency/rounding model explicitly.
    let mut gross = 0.0;
    let mut costs = 0.0;
    for id in valued {
        let leg = legs[&id];
        gross += leg.gross_profit_at_stop.expect("validated above");
        for value in leg.costs.values() {
            costs += value.expect("validated above");
            if !costs.is_finite() {
                return result(RfFloorOutcome::Incomplete {
                    issues: vec![I::ArithmeticOverflow],
                });
            }
        }
        if !gross.is_finite() {
            return result(RfFloorOutcome::Incomplete {
                issues: vec![I::ArithmeticOverflow],
            });
        }
    }
    let realized_net = realized.net.expect("validated above");
    let nominal_net = (realized_net + gross) + costs;
    if !nominal_net.is_finite() {
        return result(RfFloorOutcome::Incomplete {
            issues: vec![I::ArithmeticOverflow],
        });
    }
    let calculation = RfNominalFloor {
        owner: owner.clone(),
        currency: input.currency.clone(),
        valuation_model_id: input.valuation_model_id.clone(),
        snapshot_revision: snapshot.revision,
        ledger_revision: realized.ledger_revision,
        realized_net,
        remaining_gross_at_stops: gross,
        remaining_signed_costs: costs,
        nominal_net,
        position_count: positions.len(),
    };
    if nominal_net >= 0.0 {
        result(RfFloorOutcome::VerifiedNominal { calculation })
    } else {
        result(RfFloorOutcome::Unsecured {
            calculation: Some(calculation),
            reasons: vec![RfUnsecured::NegativeNominalFloor],
        })
    }
}

#[cfg(test)]
#[path = "rf_protection_tests.rs"]
mod tests;
