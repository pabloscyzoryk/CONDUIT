//! Boundary validation for a broker's complete causal closed-cost proof.
//! Does not book cash, query MT5, deduplicate deals, or infer absent costs.
use crate::proto::RawClosed;
use conduit_core::cost_receipt::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCostReceipt {
    schema: u16,
    deal_id: u64,
    position_id: u64,
    volume: f64,
    currency: String,
    complete: bool,
    incomplete_reason: Option<String>,
    cutoff_time_msc: i64,
    history_fingerprint: Option<String>,
    history_query_complete: bool,
    first_entry_time_msc: Option<i64>,
    gross_profit: Option<f64>,
    entry_commission_alloc: Option<f64>,
    exit_commission: Option<f64>,
    entry_fee_alloc: Option<f64>,
    exit_fee: Option<f64>,
    swap: Option<f64>,
}

pub(crate) fn closed_receipt(raw: &RawClosed, scope: &str, currency: &str, known_open_ts: i64)
    -> Result<CostReceipt, String> {
    let value = raw.cost_receipt.as_ref().ok_or("missing separate cost receipt proof")?;
    let proof: RawCostReceipt = serde_json::from_value(value.clone())
        .map_err(|e|format!("malformed cost receipt: {e}"))?;
    if !proof.complete || !proof.history_query_complete || proof.incomplete_reason.is_some() {
        return Err(format!("incomplete cost history: {}", proof.incomplete_reason.as_deref().unwrap_or("no complete-history proof")));
    }
    let schema = CostSchema::try_from(proof.schema).map_err(|e|e.to_string())?;
    if proof.deal_id != raw.deal || proof.position_id != raw.position
        || proof.cutoff_time_msc != raw.time_msc || proof.volume != raw.volume {
        return Err("cost receipt identity/volume/cutoff differs from closed deal".into());
    }
    if currency.is_empty() || proof.currency != currency {
        return Err("cost receipt account currency mismatch".into());
    }
    // Separate optional fields prove the original source actually supplied them.
    if proof.gross_profit != Some(raw.profit) || proof.exit_commission != Some(raw.commission)
        || proof.swap != Some(raw.swap) {
        return Err("missing or contradictory original profit/commission/swap".into());
    }
    let first = proof.first_entry_time_msc.ok_or("missing first entry history timestamp")?;
    if first < 0 || first > raw.time_msc || (known_open_ts > 0 && first > known_open_ts) {
        return Err("position history starts after the independently observed position opening".into());
    }
    let fingerprint = proof.history_fingerprint.ok_or("missing causal history fingerprint")?;
    if fingerprint.len() != 64 || !fingerprint.bytes().all(|b|b.is_ascii_hexdigit()) {
        return Err("invalid causal history fingerprint".into());
    }
    let receipt = CostReceipt {
        schema, key: CostReceiptKey {scope_id:scope.to_owned(),deal_id:raw.deal},
        position_identifier:raw.position, volume:raw.volume, currency:proof.currency,
        source:CostSource::BrokerDeals,
        gross_profit:proof.gross_profit,entry_commission_alloc:proof.entry_commission_alloc,
        exit_commission:proof.exit_commission,entry_fee_alloc:proof.entry_fee_alloc,
        exit_fee:proof.exit_fee,swap:proof.swap,completeness:CostCompleteness::Complete,
        entry_allocation:Some(EntryAllocationProof {method:EntryAllocationMethod::ResidualProportionalV1,
            cutoff_time_msc:raw.time_msc,cutoff_deal_id:raw.deal,history_query_complete:true,
            history_fingerprint:fingerprint}),
    };
    receipt.net().map_err(|e|e.to_string())?;
    Ok(receipt)
}
