//! T-100: autonomous, causal policy. Broker execution lives in Engine's adapter.
//! Every clock, observation and receipt is supplied by the caller. No I/O here.
mod market;
mod context;
mod policy;
pub use market::{Bar, MarketState, Features};
pub use context::{ContextBook, SignalContext};
pub use policy::Runtime;
use crate::types::{Account, CloseReason, Position, Quote, Side, Ticket, Ts};
use serde::{Deserialize, Serialize};

pub const REVISION: &str = "T-100/3";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    /// Bit mask: trend pullback=1, breakout=2, range reversion=4, Synergy zone=8.
    pub experts: u8,
    pub signal_weight: f64,
    pub signal_half_life_min: f64,
    pub market_context_max_age_min: f64,
    pub signal_required: bool,
    pub score_threshold: f64,
    pub risk_pct: f64,
    pub portfolio_risk_pct: f64,
    pub margin_budget_pct: f64,
    pub max_positions: usize,
    pub cooldown_bars: u32,
    pub stop_atr: f64,
    pub reward_risk: f64,
    pub trail_start_r: f64,
    pub trail_atr: f64,
    pub break_even_r: f64,
    pub max_hold_min: u32,
    pub daily_loss_pct: f64,
    pub daily_profit_lock_pct: f64,
    pub daily_giveback_pct: f64,
    pub spread_atr_max: f64,
    pub spread_abs_max: f64,
    pub min_atr: f64,
    pub shock_atr: f64,
    pub adaptation: f64,
    pub trend_threshold: f64,
    pub range_threshold: f64,
    pub session_start_utc: u8,
    pub session_end_utc: u8,
    pub friday_flat_utc: u8,
}

impl Default for Config {
    fn default() -> Self { Self {
        enabled:false, experts:15, signal_weight:0.3, signal_half_life_min:180.0,
        market_context_max_age_min:720.0, signal_required:false,
        score_threshold:0.62, risk_pct:1.0, portfolio_risk_pct:5.0,
        margin_budget_pct:25.0, max_positions:5, cooldown_bars:3,
        stop_atr:1.8, reward_risk:2.0, trail_start_r:1.2, trail_atr:2.0,
        break_even_r:1.0, max_hold_min:120, daily_loss_pct:6.0,
        daily_profit_lock_pct:2.0, daily_giveback_pct:60.0,
        spread_atr_max:0.18, spread_abs_max:1.2, min_atr:0.3, shock_atr:4.0,
        adaptation:0.2, trend_threshold:0.28, range_threshold:0.24,
        session_start_utc:5, session_end_utc:21, friday_flat_utc:20,
    }}
}

impl Config {
    /// Invalid experimental settings fail closed; never silently normalize them.
    pub fn valid(&self) -> bool {
        let values=[self.signal_weight,self.signal_half_life_min,self.market_context_max_age_min,
            self.score_threshold,self.risk_pct,self.portfolio_risk_pct,self.margin_budget_pct,
            self.stop_atr,self.reward_risk,self.trail_start_r,self.trail_atr,self.break_even_r,
            self.daily_loss_pct,self.daily_profit_lock_pct,self.daily_giveback_pct,
            self.spread_atr_max,self.spread_abs_max,self.min_atr,self.shock_atr,
            self.adaptation,self.trend_threshold,self.range_threshold];
        values.iter().all(|x|x.is_finite() && *x>=0.0) && self.experts>0 && self.experts<=15
            && self.signal_weight<=1.0 && self.signal_half_life_min>0.0
            && self.risk_pct>0.0 && self.risk_pct<=20.0 && self.portfolio_risk_pct<=30.0
            && self.portfolio_risk_pct>=self.risk_pct && self.margin_budget_pct>0.0
            && self.margin_budget_pct<=80.0 && self.max_positions>0 && self.max_positions<=100
            && self.stop_atr>=0.3 && self.reward_risk>=0.3 && self.trail_atr>=0.3
            && self.daily_loss_pct>0.0 && self.daily_loss_pct<=50.0
            && self.daily_giveback_pct<=100.0 && self.adaptation<=1.0
            && self.session_start_utc<self.session_end_utc && self.session_end_utc<=24
            && self.friday_flat_utc<=24 && self.max_hold_min>0 && self.min_atr>0.0
    }
}

/// One immutable broker snapshot. `positions` must include only this engine's
/// positions; hidden exposure must be included in `other_risk_usd`.
pub struct PortfolioView<'a> {
    pub account: &'a Account,
    pub positions: &'a [Position],
    pub entry_allowed: bool,
    pub other_risk_usd: f64,
    pub lot_min: f64,
    pub lot_step: f64,
    pub lot_max: f64,
    /// 0 means no user cap. Broker volume maximum still applies.
    pub lot_cap: f64,
    pub stops_level: f64,
    /// Quote timestamps use broker wall time. Subtract this for UTC sessions.
    pub broker_utc_offset_hours: i32,
    /// Some = authoritative completed broker BID candles, including an empty
    /// waiting feed. None = complete simulated tick tape, aggregated locally.
    pub completed_bars: Option<&'a [Bar]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryPlan {
    pub decision_id: u64,
    pub ts: Ts,
    pub side: Side,
    pub expert: u8,
    pub volume: f64,
    pub entry_reference: f64,
    pub sl: f64,
    pub tp: f64,
    /// Policy/portfolio risk allowance before volume discretization. The
    /// adapter may consume spare allowance after price rounding, never raise
    /// the planned volume or any margin/volume cap.
    pub approved_budget_usd: f64,
    /// Risk of the planned (then confirmed) geometry, used for learning.
    pub risk_usd: f64,
    pub score: f64,
    pub atr: f64,
    /// Optional context attribution, never a broker order/reply identity.
    pub context_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Intent {
    Open(EntryPlan),
    Modify { ticket:Ticket, sl:f64, tp:Option<f64> },
    Close { ticket:Ticket, reason:CloseReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionOutcome { Confirmed, Rejected, Uncertain }

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Diagnostics {
    pub quotes:u64, pub closed_bars:u64, pub contexts:u64, pub decisions:u64,
    pub opened:u64, pub rejected:u64, pub uncertain:u64, pub invalid_quotes:u64,
    pub blocked_warmup:u64, pub blocked_spread:u64, pub blocked_risk:u64,
    pub blocked_session:u64, pub blocked_entry:u64, pub blocked_score:u64,
    pub context_entries:u64, pub market_only_entries:u64,
    pub expert_entries:[u64;4], pub expert_closed:[u64;4],
    pub last_reason:String, pub last_score:f64, pub last_atr:f64,
}

/// Public shape deliberately small enough for Engine and replay adapters.
pub fn valid_quote(q:&Quote) -> bool {
    q.ts>=0 && q.bid.is_finite() && q.ask.is_finite() && q.bid>0.0 && q.ask>=q.bid
}
