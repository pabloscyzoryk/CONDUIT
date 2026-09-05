//! Validated state-only dynamic S/R warmup. No broker, clock or trade replay.
use super::{sr_state::SrStateMath, Engine, StanSr};
use crate::types::Ts;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SrWarmupSourceV2 { FullBrokerTicks, ObservedBotTicks, BacktestTickData }

/// Explicit adapter identity, not inferred from prices or message publication time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrWarmupContextV2 {
    pub account_scope: String,
    pub symbol: String,
    pub runtime_generation: String,
    pub source_kind: SrWarmupSourceV2,
    pub clock_domain: String,
    /// Transformation applied by THIS adapter, once. Existing CDTK ticks need 0.
    pub explicit_offset_ms: i64,
    pub price_normalization: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SrWarmupMinuteV2 {
    pub bucket_open_ts: Ts,
    pub first_tick_ts: Ts,
    pub last_tick_ts: Ts,
    pub high_mid: f64,
    pub low_mid: f64,
    pub close_mid: f64,
    pub last_spread: f64,
    pub tick_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SrWarmupSnapshotV2 {
    pub version: u32,
    pub context: SrWarmupContextV2,
    /// SHA-256 of canonical (timestamp,bid bits,ask bits), in delivery order.
    pub source_hash: String,
    pub from_inclusive: Ts,
    pub cutoff_exclusive: Ts,
    /// A completed range scan, not a claim that a sampled feed saw every market tick.
    pub complete_query_coverage: bool,
    pub minutes: Vec<SrWarmupMinuteV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrWarmupAppliedV2 { Inactive, Applied { dynamic_ready: bool, tick_count: u64 } }

impl SrWarmupSnapshotV2 {
    pub fn validate(&self, expected: &SrWarmupContextV2, cutoff: Ts) -> Result<(), String> {
        if self.version != 2 { return Err("unsupported SR snapshot version".into()); }
        if &self.context != expected { return Err("SR account/symbol/generation/source/clock/precision mismatch".into()); }
        if [&expected.account_scope, &expected.symbol, &expected.runtime_generation,
            &expected.clock_domain, &expected.price_normalization].iter().any(|s| s.trim().is_empty()) {
            return Err("missing SR identity/clock/precision context".into());
        }
        if self.cutoff_exclusive != cutoff || self.from_inclusive < 0 || self.from_inclusive >= cutoff {
            return Err("invalid SR causal range/cutoff".into());
        }
        if !self.complete_query_coverage { return Err("incomplete SR query coverage".into()); }
        if self.source_hash.len() != 64 || !self.source_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid SR canonical source hash".into());
        }
        if self.minutes.len() > 1_000_000 { return Err("SR snapshot exceeds bounded minute capacity".into()); }
        let mut previous = None;
        let mut ticks = 0_u64;
        for m in &self.minutes {
            if m.tick_count == 0 || m.first_tick_ts < self.from_inclusive
                || m.last_tick_ts >= cutoff || m.first_tick_ts > m.last_tick_ts
                || m.bucket_open_ts != m.first_tick_ts.div_euclid(60_000) * 60_000
                || m.bucket_open_ts != m.last_tick_ts.div_euclid(60_000) * 60_000
                || previous.is_some_and(|p| m.bucket_open_ts <= p) {
                return Err("invalid/future/out-of-order SR minute".into());
            }
            if ![m.high_mid,m.low_mid,m.close_mid,m.last_spread].iter().all(|x| x.is_finite())
                || m.low_mid <= 0.0 || m.high_mid < m.low_mid || m.close_mid < m.low_mid
                || m.close_mid > m.high_mid || m.last_spread < 0.0 {
                return Err("invalid SR MID price/spread".into());
            }
            ticks = ticks.checked_add(m.tick_count).ok_or("SR tick count overflow")?;
            previous = Some(m.bucket_open_ts);
        }
        Ok(())
    }
}

impl Engine {
    pub fn sr_warmup_exact_active(&self) -> bool {
        self.cfg.sr_warmup_exact_ticks && self.cfg.trail_sr_enabled && self.sr_dynamic_active()
    }

    /// Build a temporary state. Invalid input never erases or partly mutates live state.
    /// Caller must verify the cutoff precedes its first subsequent tick; this method
    /// does not catch up an already running engine or activate trading retrospectively.
    pub fn rozgrzej_sr_v2(&mut self, snapshot: &SrWarmupSnapshotV2,
        expected: &SrWarmupContextV2, cutoff: Ts) -> Result<SrWarmupAppliedV2, String> {
        if !self.sr_warmup_exact_active() { return Ok(SrWarmupAppliedV2::Inactive); }
        snapshot.validate(expected, cutoff)?;
        let mut next = StanSr::default();
        let tf_ms = 60_000 * self.cfg.trail_sr_tf_min.max(1) as i64;
        for m in &snapshot.minutes {
            let bucket = m.first_tick_ts.div_euclid(tf_ms);
            if next.kubelek != i64::MIN && bucket != next.kubelek {
                SrStateMath { cfg: &self.cfg, sr: &mut next }.sr_zamknij_swiece(m.first_tick_ts);
            }
            if bucket != next.kubelek {
                next.kubelek = bucket;
                next.high = m.high_mid;
                next.low = m.low_mid;
            } else {
                next.high = next.high.max(m.high_mid);
                next.low = next.low.min(m.low_mid);
            }
            next.close = m.close_mid;
            next.spread_close = m.last_spread;
        }
        next.nowa_swieca = false;
        if !finite_state(&next) { return Err("SR arithmetic produced nonfinite state".into()); }
        let tick_count = snapshot.minutes.iter().map(|m| m.tick_count).sum();
        self.sr = next;
        Ok(SrWarmupAppliedV2::Applied { dynamic_ready: self.sr_dynamic_ready(), tick_count })
    }
}

fn finite_state(s: &StanSr) -> bool {
    [s.high,s.low,s.close,s.spread_close].iter().all(|v| v.is_finite())
        && [s.prev_close,s.atr,s.spread_ref].iter().flatten().all(|v| v.is_finite())
        && s.zamkniete.iter().all(|(h,l)| h.is_finite() && l.is_finite())
        && s.swingi_low.iter().chain(&s.swingi_high).all(|(p,_)| p.is_finite())
        && s.prominence_low.iter().chain(&s.prominence_high).all(|(p,_,v)| p.is_finite() && v.is_finite())
        && s.atr_true_ranges.iter().chain(&s.spread_closed).all(|v| v.is_finite())
}
