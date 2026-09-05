//! Lossless M1 sufficient statistics for dynamic S/R, from the same canonical
//! TickData::quote path as trading. No sorting, timezone inference or future bars.
use crate::TickData;
use conduit_core::engine::{SrWarmupContextV2, SrWarmupMinuteV2, SrWarmupSnapshotV2, SrWarmupSourceV2};
use sha2::{Digest, Sha256};

pub fn tickdata_context(ticks: &TickData) -> SrWarmupContextV2 {
    SrWarmupContextV2 {
        account_scope: "backtest-synthetic-account".into(),
        symbol: "CDTK-instrument".into(), runtime_generation: "backtest-initial-prefix".into(),
        source_kind: SrWarmupSourceV2::BacktestTickData,
        clock_domain: "CDTK-ts-unchanged".into(), explicit_offset_ms: 0,
        price_normalization: format!("TickData::quote/price_digits={:?}", ticks.price_digits()),
    }
}

/// CDTK has the same globally ordered-input contract as the replay binary search.
/// We additionally validate EVERY selected quote and both selection boundaries;
/// malformed data is an error, not silently dropped/normalized here.
pub fn snapshot_from_tickdata(ticks: &TickData, from: i64, cutoff: i64)
    -> Result<SrWarmupSnapshotV2, String> {
    if from < 0 || from >= cutoff { return Err("invalid SR producer range".into()); }
    let start = ticks.index_at(from);
    let end = ticks.index_at(cutoff);
    if start > end || (start > 0 && ticks.ts(start - 1) >= from)
        || (end < ticks.len() && ticks.ts(end) < cutoff) {
        return Err("SR CDTK boundary/order mismatch".into());
    }
    let mut minutes: Vec<SrWarmupMinuteV2> = Vec::new();
    let mut hash = Sha256::new();
    let mut previous = None;
    for i in start..end {
        let q = ticks.quote(i);
        let mid = q.mid(); let spread = q.ask - q.bid;
        if q.ts < from || q.ts >= cutoff || previous.is_some_and(|t| q.ts < t)
            || !q.bid.is_finite() || !q.ask.is_finite() || !mid.is_finite()
            || q.bid <= 0.0 || q.ask < q.bid || !spread.is_finite() {
            return Err(format!("invalid/out-of-order SR quote at CDTK index {i}"));
        }
        previous = Some(q.ts);
        hash.update(q.ts.to_le_bytes());
        hash.update(q.bid.to_bits().to_le_bytes()); hash.update(q.ask.to_bits().to_le_bytes());
        let bucket_open_ts = q.ts.div_euclid(60_000) * 60_000;
        if let Some(m) = minutes.last_mut().filter(|m| m.bucket_open_ts == bucket_open_ts) {
            m.last_tick_ts = q.ts; m.high_mid = m.high_mid.max(mid); m.low_mid = m.low_mid.min(mid);
            m.close_mid = mid; m.last_spread = spread; m.tick_count += 1;
        } else {
            minutes.push(SrWarmupMinuteV2 { bucket_open_ts, first_tick_ts: q.ts, last_tick_ts: q.ts,
                high_mid: mid, low_mid: mid, close_mid: mid, last_spread: spread, tick_count: 1 });
        }
    }
    let out = SrWarmupSnapshotV2 { version: 2, context: tickdata_context(ticks),
        source_hash: format!("{:x}", hash.finalize()), from_inclusive: from,
        cutoff_exclusive: cutoff, complete_query_coverage: true, minutes };
    out.validate(&out.context, cutoff)?;
    Ok(out)
}
