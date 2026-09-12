
use crate::broker::*;
use crate::journal::{
    self, CloseDetail, ClosedLeg, Ev, EventCategory, EventKind, EventLevel, JournalBuf,
    MarketSnapshot, RejectCode,
};
use crate::parser::{self, EntrySignal, Signal};
use crate::settings::*;
use crate::types::*;
use std::collections::HashMap;

#[path = "engine/t100_execution.rs"]
mod t100_execution;
pub use t100_execution::T100Checkpoint;

#[path = "relot_reconcile.rs"]
mod relot_reconcile;

#[cfg(test)]
#[path = "lot_growth_engine_tests.rs"]
mod lot_growth_engine_tests;

#[path = "engine/lot_context_execution.rs"]
mod lot_context_execution;

#[path = "rearm_reconcile.rs"]
mod rearm_reconcile;
pub use rearm_reconcile::RearmReconcileState;

#[path = "entry_edit.rs"]
mod entry_edit;
#[path = "pending_validity.rs"]
mod pending_validity;
pub use pending_validity::PendingSourceRecord;
#[path = "entry_sources.rs"]
mod entry_sources;
pub use entry_sources::EntrySourceRecord;
pub use entry_edit::EntryEditOutcome;

#[path = "strategy_continuation.rs"]
mod strategy_continuation;
#[path = "replay_bootstrap.rs"]
mod replay_bootstrap;
pub use replay_bootstrap::ReplayBootstrap;
pub use strategy_continuation::{
    ContinuationImportReport, ContinuationOrigin, ContinuationReview, ContinuationReviewScope,
    EngineContinuationV1,
};

#[path = "deferred_entry.rs"]
mod deferred_entry;
pub use deferred_entry::{DeferredEntryState, DeferredEntryStatus};

#[path = "sr_state.rs"]
mod sr_state;
#[path = "sr_warmup.rs"]
mod sr_warmup;
pub use sr_warmup::{
    SrWarmupAppliedV2, SrWarmupContextV2, SrWarmupMinuteV2, SrWarmupSnapshotV2, SrWarmupSourceV2,
};

use conduit_mozg_cien::aktuator as cakt;
use conduit_mozg_cien::diag as cien;
use conduit_mozg_cien::zrodlo as czr;

#[inline]
fn round_lot(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TrailAdaptiveSnapshot {
    price_efficiency: f64,
    vol_ratio: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct MovementAccumulator {
    first_ts: Option<Ts>,
    first_px: Px,
    last_ts: Ts,
    last_px: Px,
    path: f64,
    samples: u32,
}

impl MovementAccumulator {
    #[inline]
    fn push(&mut self, ts: Ts, px: Px) {
        if self.samples == 0 {
            self.first_ts = Some(ts);
            self.first_px = px;
        } else {
            self.path += (px - self.last_px).abs();
        }
        self.last_ts = ts;
        self.last_px = px;
        self.samples += 1;
    }

    #[inline]
    fn path_rate(&self) -> Option<f64> {
        let dt_s = (self.last_ts - self.first_ts?) as f64 / 1000.0;
        (self.samples >= 3 && dt_s > 0.0 && self.path > 0.0).then_some(self.path / dt_s)
    }
}

static DIAG_KRAWEDZ: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> =
    std::sync::OnceLock::new();

fn diag_plik() -> Option<&'static std::sync::Mutex<std::fs::File>> {
    DIAG_KRAWEDZ
        .get_or_init(|| {
            let p = std::env::var("KRAWEDZ_DIAG").ok()?;
            let f = std::fs::File::create(p).ok()?;
            Some(std::sync::Mutex::new(f))
        })
        .as_ref()
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn diag_wejscie(
    ts: Ts,
    id: u32,
    sciezka: &str,
    side: Side,
    sig_lo: Px,
    sig_hi: Px,
    cena: Px,
    rynek: Px,
    poziom: i32,
) {
    let Some(m) = diag_plik() else { return };
    let w = (sig_hi - sig_lo).abs();
    let d = if w > 1e-9 {
        match side {
            Side::Buy => (sig_hi - cena) / w,
            Side::Sell => (cena - sig_lo) / w,
        }
    } else {
        f64::NAN
    };
    if let Ok(mut f) = m.lock() {
        use std::io::Write;
        let _ = writeln!(
            f,
            "{ts};{id};{sciezka};{side:?};{sig_lo:.2};{sig_hi:.2};{cena:.2};{rynek:.2};{poziom};{d:.4}"
        );
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StanZmiennosci {
    pub sezon_suma: [f64; 24],
    pub sezon_ile: [u32; 24],
    pub sezon_klucz: i64,
    pub sezon_hi: Px,
    pub sezon_lo: Px,
    pub proby: Vec<f64>,
    pub proba_klucz: i64,
    pub mult: f64,
    pub ile_policzono: u64,
    pub suma_mult: f64,
    pub min_mult: f64,
    pub max_mult: f64,
    pub ile_odsezonowano: u64,
}

impl Default for StanZmiennosci {
    fn default() -> Self {
        StanZmiennosci {
            sezon_suma: [0.0; 24],
            sezon_ile: [0; 24],
            sezon_klucz: i64::MIN,
            sezon_hi: 0.0,
            sezon_lo: 0.0,
            proby: Vec::new(),
            proba_klucz: i64::MIN,
            mult: 1.0,
            ile_policzono: 0,
            suma_mult: 0.0,
            min_mult: f64::MAX,
            max_mult: f64::MIN,
            ile_odsezonowano: 0,
        }
    }
}


const TRAIL_SR_TF_MS: i64 = 60_000;
const TRAIL_SR_FRACTAL_N: usize = 3;
const TRAIL_SR_OFFSET: f64 = 0.5;
const TRAIL_SR_MIN_DIST_TP: f64 = 2.0;
const TRAIL_SR_STRUCT_WINDOW_MS: i64 = 24 * 3_600_000;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct StanSr {
    kubelek: i64,
    high: Px,
    low: Px,
    close: Px,
    spread_close: Px,
    zamkniete: std::collections::VecDeque<(Px, Px)>,
    swingi_low: std::collections::VecDeque<(Px, Ts)>,
    swingi_high: std::collections::VecDeque<(Px, Ts)>,
    prominence_low: std::collections::VecDeque<(Px, Ts, f64)>,
    prominence_high: std::collections::VecDeque<(Px, Ts, f64)>,
    prev_close: Option<Px>,
    atr_true_ranges: std::collections::VecDeque<f64>,
    spread_closed: std::collections::VecDeque<f64>,
    atr: Option<f64>,
    spread_ref: Option<f64>,
    nowa_swieca: bool,
}

impl Default for StanSr {
    fn default() -> Self {
        StanSr {
            kubelek: i64::MIN,
            high: 0.0,
            low: 0.0,
            close: 0.0,
            spread_close: 0.0,
            zamkniete: std::collections::VecDeque::new(),
            swingi_low: std::collections::VecDeque::new(),
            swingi_high: std::collections::VecDeque::new(),
            prominence_low: std::collections::VecDeque::new(),
            prominence_high: std::collections::VecDeque::new(),
            prev_close: None,
            atr_true_ranges: std::collections::VecDeque::new(),
            spread_closed: std::collections::VecDeque::new(),
            atr: None,
            spread_ref: None,
            nowa_swieca: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SrWarmupBar {
    pub ts: Ts,
    pub high: Px,
    pub low: Px,
    pub close: Px,
    pub spread: Px,
}

#[allow(clippy::too_many_arguments)]
fn new_basket(
    id: u32,
    m: &IncomingMessage,
    side: Side,
    is_limit: bool,
    entry_lo: Px,
    entry_hi: Px,
    zone_lo: Px,
    zone_hi: Px,
    sl: Option<Px>,
    tps: Vec<Px>,
    ts: Ts,
) -> Basket {
    Basket {
        warstwy_offset: None,
        id,
        source: m.source.clone(),
        source_name: m.source_name.clone(),
        msg_id: m.msg_id,
        pending_exit: None,
        pending_relot_review: Vec::new(),
        entry_edit_state: None,
        msg_aliases: Vec::new(),
        persisted_done_actions: Vec::new(),
        side,
        is_limit,
        is_stop: false,
        entry_lo,
        entry_hi,
        zone_lo,
        zone_hi,
        sl,
        tps,
        tp_stage: 0,
        plan_wykonany_do: 0,
        created_ts: ts,
        drop_po_ts: 0,
        state: BasketState::Armed,
        tickets: Vec::new(),
        pendings: Vec::new(),
        realized: 0.0,
        events: Vec::new(),
        levels: Vec::new(),
        reentries: 0,
        last_entry_px: None,
        secured: false,
        rearm_blocked_by_spp: false,
        secured_by_rule: false,
        had_positions: false,
        rearms: 0,
        last_rearm_ts: 0,
        wol_pierwotny: Vec::new(),
        tp_touch_ts: Vec::new(),
        tp_touch_px: Vec::new(),
        sl_touch_ts: 0,
        sl_touch_px: 0.0,
        adverse_since: 0,
        age_limit_min: 0.0,
        last_tp_ts: 0,
        tempo_fast: false,
        tempo_checked: false,
        pyramided: false,
        fast_addons: 0,
        last_addon_ts: 0,
        peak_pl_usd: 0.0,
        risk_initial_usd: 0.0,
        secured_ts: 0,
        zone_touched: false,
        tp_open: false,
        be_ts: 0,
        drop_armed: false,
    }
}

#[inline]
fn zone_overlap(a_lo: Px, a_hi: Px, b_lo: Px, b_hi: Px) -> f64 {
    let wspolne = (a_hi.min(b_hi) - a_lo.max(b_lo)).max(0.0);
    let wezsza = (a_hi - a_lo).min(b_hi - b_lo);
    if wezsza <= 1e-9 {
        if wspolne > 0.0 || (a_lo >= b_lo && a_hi <= b_hi) {
            1.0
        } else {
            0.0
        }
    } else {
        (wspolne / wezsza).min(1.0)
    }
}

pub fn widok_koszyka(bk: &Basket, pozycje: &[Position], q: &Quote) -> BasketView {
    let poz: Vec<&Position> = pozycje.iter().filter(|p| p.basket == Some(bk.id)).collect();

    let otwarte: f64 = poz.iter().map(|p| p.profit_usd(q)).sum();
    let pl_usd = bk.realized + otwarte;

    let suma_wol: f64 = poz.iter().map(|p| p.volume).sum();
    let avg_entry = if suma_wol > 0.0 {
        poz.iter().map(|p| p.open_price * p.volume).sum::<f64>() / suma_wol
    } else {
        bk.mid()
    };

    let risk_usd: f64 = poz
        .iter()
        .filter_map(|p| {
            p.sl.map(|s| (p.open_price - s).abs() * XAU_CONTRACT * p.volume)
        })
        .sum();
    let risk_initial_usd = if bk.risk_initial_usd > 0.0 {
        bk.risk_initial_usd
    } else {
        risk_usd
    };

    let plan_risk: f64 = bk
        .levels
        .iter()
        .filter_map(|g| {
            g.sl.map(|s| (g.price - s).abs() * XAU_CONTRACT * g.volume * g.base_units.max(1) as f64)
        })
        .sum();
    let planned_risk_in_market = if plan_risk > 1e-9 {
        (risk_usd / plan_risk).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let wiek = q.ts.saturating_sub(bk.created_ts) as f64 / 60_000.0;
    BasketView {
        id: bk.id,
        side: bk.side,
        pl_usd,
        pl_r_current: if risk_usd > 1e-9 {
            pl_usd / risk_usd
        } else {
            0.0
        },
        pl_r_initial: if risk_initial_usd > 1e-9 {
            pl_usd / risk_initial_usd
        } else {
            0.0
        },
        avg_entry,
        risk_usd,
        risk_initial_usd,
        peak_pl_usd: bk.peak_pl_usd,
        drawdown_from_peak: (bk.peak_pl_usd - pl_usd).max(0.0),
        filled_layers: poz.len() as u32,
        pending_layers: 0, // uzupełniane przez `Engine::basket_view`, patrz niżej
        planned_risk_in_market,
        age_min: wiek.max(0.0),
        secured: bk.secured,
        tp_stage: bk.tp_stage,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Gate {
    Open,
    Halted(String),
    Blocked(String, RejectCode),
}

#[allow(dead_code)]
struct Szczebel {
    t: Ticket,
    vol: f64,
    topup: bool,
    kind: PendingKind,
    price: Px,
    sl: Option<Px>,
    tp: Option<Px>,
    touch: bool,
    com: String,
}

impl Gate {
    pub fn blocked(&self) -> Option<(&str, RejectCode)> {
        match self {
            Gate::Open => None,
            Gate::Halted(r) => Some((r.as_str(), RejectCode::Halted)),
            Gate::Blocked(r, c) => Some((r.as_str(), *c)),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogLine {
    pub ts: Ts,
    pub level: u8, // 0 info, 1 ok, 2 warn, 3 error
    pub text: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IncomingMessage {
    pub ts: Ts,
    pub source: SourceKey,
    pub source_name: String,
    pub msg_id: i64,
    pub reply_to: Option<i64>,
    pub edit_of: Option<i64>,
    pub text: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OdrzuconeWejscie {
    pub ts: Ts,
    pub msg_id: i64,
    pub kod: String,
    pub side: Side,
    pub lo: Px,
    pub hi: Px,
    pub sl: Option<Px>,
    pub tp1: Option<Px>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct DesiredStops {
    sl: Option<Px>,
    tp: Option<Px>,
    last_try: Ts,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct QueuedExit {
    target: Px,
    deadline: Ts,
    reason: CloseReason,
    market_at_decision: Px,
}

/// Reporting-only observations of the configured parser, never entry authority.
#[derive(Clone,Copy,Debug,serde::Serialize,serde::Deserialize)]
pub struct EntrySourceObservation {pub has_full_entry:bool,pub first_seen_as_edit:bool}

pub struct Engine {
    pub cfg: Settings,
    pub t100: crate::t100::Runtime,
    t100_execution: t100_execution::ExecutionState,
    pending_sources: std::collections::BTreeMap<u32, PendingSourceRecord>,
    entry_sources: HashMap<(SourceKey,i64),EntrySourceRecord>,
    entry_source_aliases: HashMap<(SourceKey,i64),i64>,
    entry_source_revision: u64,
    pending_source_revision: u64,

    pub regime_hist: Vec<(Ts, bool)>,

    pub baskets: Vec<Basket>,
    basket_slots: HashMap<u32, usize>,
    basket_slots_len: usize,
    pub stats: Stats,
    pub logs: Vec<LogLine>,
    pub halted: Option<String>,
    pub cost_reconciliation_required: Option<String>,
    pub cost_quarantine: Vec<ClosedTrade>,
    pub risk_override: bool,
    halt_ms: i64,
    halt_prev_ts: Ts,
    halt_min_zapisane: i64,
    next_basket_id: u32,
    created_baskets_count: u32,
    entry_source_observations: HashMap<(SourceKey,i64),EntrySourceObservation>,
    // Diagnostic dedup only; a local precheck has not consumed a broker intent.
    fast_addon_invalid_tp_note: HashMap<u32,u64>,
    // Engine-to-Broker call boundary, not a claim of terminal acceptance.
    order_submission_sequence: u64,
    rearm_reconcile: rearm_reconcile::RearmReconcile,
    loss_streak: u32,
    paused_until: Ts,
    day_stop: i64,
    kredyt_rozjazd_dzien: i64,
    last_vsl_eval: Ts,
    last_tp_hit_ts: Ts,
    spread_med: Px,
    spread_buf: Vec<Px>,
    zmiennosc: StanZmiennosci,
    sr: StanSr,
    price_hist: Vec<(Ts, Px)>,
    rezim_miekki: bool,
    slhit_dnia: u32,
    slhit_pauza_do: Ts,
    slhit_miekki: bool,
    vol_hist: Vec<(Ts, Px)>,
    desired: HashMap<Ticket, DesiredStops>,
    last_resize: Ts,
    last_relot: Ts,
    last_expo: Ts,
    limit_cancelled: Vec<(u32, i32)>,
    done_actions: HashMap<(SourceKey, i64), Vec<String>>,
    deferred_entries: deferred_entry::DeferredEntries,
    msg_to_basket: HashMap<(SourceKey, i64), u32>,
    opened_today: u32,
    budget_day: i64,
    queued_exits: HashMap<Ticket, QueuedExit>,
    continuation: strategy_continuation::ContinuationRuntime,
    pub obs: crate::obserwacje::Obserwator,
    pub closed_today: Vec<f64>,
    pub journal: JournalBuf,
    pub odrzuty: std::collections::BTreeMap<String, u64>,
    pub odrzucone_wejscia: Vec<OdrzuconeWejscie>,
    wejscie_w_obrobce: Option<OdrzuconeWejscie>,
    zignorowane: u64,
    pub wygaszanie: bool,

    pub tryb_auto_ea: bool,

    pub ea: crate::ea::EaRdzen,

    pub ea_a: crate::ea::StanRodzinyA,

    slot: u32,
    pub pulapy: crate::formaty::PulapyGlobalne,
    pub obce: crate::wielosilnik::ObceObciazenie,
}

impl Engine {
    pub fn new(cfg: Settings, start_balance: f64) -> Self {
        let jcfg = cfg.journal_config();
        Engine {
            t100: Default::default(),
            t100_execution: t100_execution::ExecutionState::new(&cfg.t100),
            pending_sources: Default::default(),
            entry_sources: Default::default(),
            entry_source_aliases: Default::default(),
            entry_source_revision: pending_validity::next_source_revision(),
            pending_source_revision: pending_validity::next_source_revision(),
            journal: JournalBuf::new(jcfg, "eng"),
            cfg,
            regime_hist: Vec::new(),
            baskets: Vec::new(),
            basket_slots: HashMap::new(),
            basket_slots_len: 0,
            stats: Stats::new(start_balance),
            logs: Vec::new(),
            halted: None,
            cost_reconciliation_required: None,
            cost_quarantine: Vec::new(),
            risk_override: false,
            halt_ms: 0,
            halt_prev_ts: 0,
            halt_min_zapisane: 0,
            next_basket_id: 1,
            created_baskets_count: 0,
            entry_source_observations: HashMap::new(),
            fast_addon_invalid_tp_note: HashMap::new(),
            order_submission_sequence: 0,
            rearm_reconcile: Default::default(),
            loss_streak: 0,
            paused_until: 0,
            day_stop: i64::MIN,
            kredyt_rozjazd_dzien: i64::MIN,
            last_vsl_eval: 0,
            last_tp_hit_ts: 0,
            spread_med: 0.0,
            spread_buf: Vec::with_capacity(512),
            zmiennosc: StanZmiennosci::default(),
            sr: StanSr::default(),
            price_hist: Vec::new(),
            rezim_miekki: false,
            slhit_dnia: 0,
            slhit_pauza_do: i64::MIN,
            slhit_miekki: false,
            vol_hist: Vec::new(),
            desired: HashMap::new(),
            limit_cancelled: Vec::new(),
            last_resize: 0,
            last_relot: 0,
            last_expo: 0,
            done_actions: HashMap::new(),
            deferred_entries: Default::default(),
            msg_to_basket: HashMap::new(),
            opened_today: 0,
            budget_day: i64::MIN,
            queued_exits: HashMap::new(),
            continuation: Default::default(),
            obs: crate::obserwacje::Obserwator::default(),
            closed_today: Vec::new(),
            odrzuty: std::collections::BTreeMap::new(),
            odrzucone_wejscia: Vec::new(),
            wejscie_w_obrobce: None,
            zignorowane: 0,
            wygaszanie: false,
            tryb_auto_ea: false,
            ea: crate::ea::EaRdzen::default(),
            ea_a: crate::ea::StanRodzinyA::default(),
            slot: crate::wielosilnik::SLOT_STARY,
            pulapy: crate::formaty::PulapyGlobalne::default(),
            obce: crate::wielosilnik::ObceObciazenie::default(),
        }
    }

    pub fn przypisz_slot(&mut self, slot: u32) {
        debug_assert!(
            self.baskets.is_empty(),
            "slot wolno przypisać tylko silnikowi bez koszyków"
        );
        if !self.baskets.is_empty() {
            return;
        }
        self.slot = slot;
        self.next_basket_id = crate::wielosilnik::pierwszy_numer(slot);
    }

    pub fn slot(&self) -> u32 {
        self.slot
    }


    #[track_caller]
    fn try_modify<B: Broker>(
        &mut self,
        b: &mut B,
        t: Ticket,
        sl: Option<Px>,
        tp: Option<Px>,
        ts: Ts,
    ) -> bool {
        if cien::czynny() {
            let l = std::panic::Location::caller().line();
            let (csl, ctp) = b
                .find_position(t)
                .map(|p| (p.sl, p.tp))
                .unwrap_or((None, None));
            if cien::rozne_px(sl, csl) {
                cien::z(cakt::A_STOP, t, czr::L_TRY_MODIFY, l);
            }
            if cien::rozne_px(tp, ctp) {
                cien::z(cakt::A_CEL, t, czr::L_TRY_MODIFY, l);
            }
        }
        let continuation_guard = self
            .cfg
            .restore_strategy_continuation
            .then(|| self.capture_continuation_guard(b, t));
        match b.modify_position(t, sl, tp) {
            Ok(()) => {
                self.desired.remove(&t);
                self.forget_continuation_guard(t, false);
                true
            }
            Err(_) => {
                if self.cfg.sltp_retry_s > 0.0 {
                    if let Some(guard) = continuation_guard {
                        self.remember_continuation_guard(t, guard, false);
                    }
                    self.desired.insert(
                        t,
                        DesiredStops {
                            sl,
                            tp,
                            last_try: ts,
                        },
                    );
                }
                false
            }
        }
    }

    fn retry_stops<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        if self.cfg.sltp_retry_s <= 0.0 || self.desired.is_empty() {
            return;
        }
        let gap = (self.cfg.sltp_retry_s * 1000.0) as i64;
        let due: Vec<(Ticket, DesiredStops)> = self
            .desired
            .iter()
            .filter(|(t, d)| ts - d.last_try >= gap && b.find_position(**t).is_some())
            .map(|(t, d)| (*t, *d))
            .collect();
        self.desired.retain(|t, _| b.find_position(*t).is_some());
        for (t, d) in due {
            let Some(d) = self.continuation_retry_stops(b, t, d) else {
                continue;
            };
            if cien::czynny() {
                let (csl, ctp) = b
                    .find_position(t)
                    .map(|p| (p.sl, p.tp))
                    .unwrap_or((None, None));
                if cien::rozne_px(d.sl, csl) {
                    cien::z(cakt::A_STOP, t, czr::Z_RETRY_STOPS, 0);
                }
                if cien::rozne_px(d.tp, ctp) {
                    cien::z(cakt::A_CEL, t, czr::Z_RETRY_STOPS, 0);
                }
            }
            if b.modify_position(t, d.sl, d.tp).is_ok() {
                self.desired.remove(&t);
                self.forget_continuation_guard(t, false);
            } else if let Some(e) = self.desired.get_mut(&t) {
                e.last_try = ts;
            }
        }
    }


    fn vol_factor(&self, ts: Ts) -> f64 {
        if self.cfg.vol_window_min <= 0.0 {
            return 1.0;
        }
        let t0 = ts - (self.cfg.vol_window_min * 60_000.0) as i64;
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        let mut n = 0usize;
        for (t, p) in self.vol_hist.iter().rev() {
            if *t < t0 {
                break;
            }
            lo = lo.min(*p);
            hi = hi.max(*p);
            n += 1;
        }
        if n < 5 {
            return 1.0;
        }
        if hi - lo >= self.cfg.vol_range_usd {
            self.cfg.vol_units_mult.max(0.01)
        } else {
            1.0
        }
    }


    fn atr_proxy(&self, ts: Ts) -> Option<f64> {
        let win = if self.cfg.adaptive_atr_window_min > 0.0 {
            self.cfg.adaptive_atr_window_min
        } else {
            60.0
        };
        let t0 = ts - (win * 60_000.0) as i64;
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        let mut n = 0usize;
        for (t, p) in self.vol_hist.iter().rev() {
            if *t < t0 {
                break;
            }
            lo = lo.min(*p);
            hi = hi.max(*p);
            n += 1;
        }
        if n < 10 {
            return None;
        }
        let r = hi - lo;
        if r > 0.0 {
            Some(r)
        } else {
            None
        }
    }


    fn sezon_mnoznik(&self, ts: Ts) -> f64 {
        let h = hour_of(ts, self.cfg.server_tz_offset_ms) as usize;
        let z = &self.zmiennosc;
        if z.sezon_ile[h] < 3 {
            return 1.0;
        }
        let mut suma = 0.0;
        let mut ile = 0.0;
        for i in 0..24 {
            if z.sezon_ile[i] >= 3 {
                suma += z.sezon_suma[i] / z.sezon_ile[i] as f64;
                ile += 1.0;
            }
        }
        if ile < 6.0 || suma <= 0.0 {
            return 1.0;
        }
        let sredni = suma / ile;
        let moj = z.sezon_suma[h] / z.sezon_ile[h] as f64;
        let m = moj / sredni;
        if m.is_finite() && m > 0.10 {
            m
        } else {
            1.0
        }
    }

    fn ranga_percentylowa(proby: &[f64], x: f64) -> f64 {
        if proby.is_empty() {
            return 0.5;
        }
        let mniejszych = proby.iter().filter(|p| **p < x).count() as f64;
        mniejszych / proby.len() as f64
    }

    fn aktualizuj_mnoznik_zmiennosci(&mut self, q: &Quote) {
        if self.cfg.vol_size_mode == VolSizeMode::Off {
            return;
        }
        let ts = q.ts;
        let mid = q.mid();

        let klucz = (ts + self.cfg.server_tz_offset_ms).div_euclid(3_600_000);
        if self.zmiennosc.sezon_klucz == i64::MIN {
            self.zmiennosc.sezon_klucz = klucz;
            self.zmiennosc.sezon_hi = mid;
            self.zmiennosc.sezon_lo = mid;
        } else if klucz == self.zmiennosc.sezon_klucz {
            if mid > self.zmiennosc.sezon_hi {
                self.zmiennosc.sezon_hi = mid;
            }
            if mid < self.zmiennosc.sezon_lo {
                self.zmiennosc.sezon_lo = mid;
            }
        } else {
            let zakres = self.zmiennosc.sezon_hi - self.zmiennosc.sezon_lo;
            if zakres > 0.0 && zakres.is_finite() {
                let h = self.zmiennosc.sezon_klucz.rem_euclid(24) as usize;
                self.zmiennosc.sezon_suma[h] += zakres;
                self.zmiennosc.sezon_ile[h] += 1;
            }
            self.zmiennosc.sezon_klucz = klucz;
            self.zmiennosc.sezon_hi = mid;
            self.zmiennosc.sezon_lo = mid;
        }

        let Some(surowa) = self.atr_proxy(ts) else {
            return;
        };
        let sez = if self.cfg.vol_size_odsezonuj {
            self.sezon_mnoznik(ts)
        } else {
            1.0
        };
        if sez != 1.0 {
            self.zmiennosc.ile_odsezonowano += 1;
        }
        let odsez = surowa / sez;
        if !odsez.is_finite() || odsez <= 0.0 {
            return;
        }

        let k5 = (ts + self.cfg.server_tz_offset_ms).div_euclid(300_000);
        if k5 != self.zmiennosc.proba_klucz {
            self.zmiennosc.proba_klucz = k5;
            let okno = self.cfg.vol_size_percentile_okno.max(1) as usize;
            self.zmiennosc.proby.push(odsez);
            if self.zmiennosc.proby.len() > okno {
                let nadmiar = self.zmiennosc.proby.len() - okno;
                self.zmiennosc.proby.drain(0..nadmiar);
            }
        }

        let dol = if self.cfg.vol_size_min_mult > 0.0 {
            self.cfg.vol_size_min_mult
        } else {
            0.05
        };
        let gora = if self.cfg.vol_size_max_mult > 0.0 {
            self.cfg.vol_size_max_mult
        } else {
            20.0
        };
        let (dol, gora) = if dol <= gora {
            (dol, gora)
        } else {
            (gora, dol)
        };

        let m = match self.cfg.vol_size_mode {
            VolSizeMode::Off => 1.0,
            VolSizeMode::Target => {
                if self.cfg.vol_size_target <= 0.0 {
                    1.0
                } else {
                    self.cfg.vol_size_target / odsez
                }
            }
            VolSizeMode::Percentile => {
                let okno = self.cfg.vol_size_percentile_okno as usize;
                if okno == 0 || self.zmiennosc.proby.len() < (okno / 2).max(10) {
                    1.0
                } else {
                    let p = Self::ranga_percentylowa(&self.zmiennosc.proby, odsez);
                    gora - p * (gora - dol)
                }
            }
        };
        let m = if m.is_finite() {
            m.clamp(dol, gora)
        } else {
            1.0
        };
        self.zmiennosc.mult = m;
        self.zmiennosc.ile_policzono += 1;
        self.zmiennosc.suma_mult += m;
        if m < self.zmiennosc.min_mult {
            self.zmiennosc.min_mult = m;
        }
        if m > self.zmiennosc.max_mult {
            self.zmiennosc.max_mult = m;
        }
    }

    pub fn stan_zmiennosci(&self) -> StanZmiennosci {
        self.zmiennosc.clone()
    }

    pub fn set_stan_zmiennosci(&mut self, s: StanZmiennosci) {
        self.zmiennosc = s;
    }

    pub fn stan_sr(&self) -> StanSr {
        self.sr.clone()
    }

    pub fn set_stan_sr(&mut self, s: StanSr) {
        self.sr = s;
    }

    pub fn profil_godzinowy(&self) -> ([f64; 24], [u32; 24]) {
        let z = &self.zmiennosc;
        let mut suma = 0.0;
        let mut ile = 0.0;
        for i in 0..24 {
            if z.sezon_ile[i] >= 3 {
                suma += z.sezon_suma[i] / z.sezon_ile[i] as f64;
                ile += 1.0;
            }
        }
        let sredni = if ile > 0.0 { suma / ile } else { 0.0 };
        let mut out = [0.0f64; 24];
        for i in 0..24 {
            if z.sezon_ile[i] > 0 && sredni > 0.0 {
                out[i] = (z.sezon_suma[i] / z.sezon_ile[i] as f64) / sredni;
            }
        }
        (out, z.sezon_ile)
    }


    #[inline]
    fn kap_male(&self, mult: f64) -> bool {
        mult > 0.0 && self.stats.balance < self.stats.start_balance * mult
    }

    #[inline]
    fn kap_f(&self, duze: f64, male: f64, mult: f64) -> f64 {
        if self.kap_male(mult) {
            male
        } else {
            duze
        }
    }

    #[inline]
    fn kap_u(&self, duze: u32, male: u32, mult: f64) -> u32 {
        if self.kap_male(mult) {
            male
        } else {
            duze
        }
    }

    #[inline]
    fn units_base(&self, is_limit: bool) -> u32 {
        let u = self.kap_u(
            self.cfg.units_for(is_limit),
            self.cfg.entry_units_small,
            self.cfg.entry_units_small_mult,
        );
        if self.rezim_miekki && self.cfg.regime_soft_units_mult != 1.0 {
            return ((u as f64) * self.cfg.regime_soft_units_mult)
                .floor()
                .max(1.0) as u32;
        }
        u
    }

    #[inline]
    fn risk_per_basket_pct_eff(&self) -> f64 {
        let r = self.kap_f(
            self.cfg.risk_per_basket_pct,
            self.cfg.risk_per_basket_pct_small,
            self.cfg.risk_per_basket_pct_small_mult,
        );
        if self.rezim_miekki && r > 0.0 && self.cfg.regime_soft_risk_mult != 1.0 {
            return r * self.cfg.regime_soft_risk_mult;
        }
        r
    }

    #[inline]
    fn max_open_positions_eff(&self) -> u32 {
        if self.rezim_miekki && self.cfg.regime_soft_max_positions > 0 {
            return self.cfg.regime_soft_max_positions;
        }
        self.kap_u(
            self.cfg.max_open_positions,
            self.cfg.max_open_positions_small,
            self.cfg.max_open_positions_small_mult,
        )
    }

    #[inline]
    fn max_open_baskets_eff(&self) -> u32 {
        self.kap_u(
            self.cfg.max_open_baskets,
            self.cfg.max_open_baskets_small,
            self.cfg.max_open_baskets_small_mult,
        )
    }

    #[inline]
    fn bonus_ekspozycji<B: Broker>(&self, b: &B) -> (u32, u32) {
        if self.cfg.exposure_bonus_profit_pct <= 0.0
            || (self.cfg.exposure_bonus_positions == 0 && self.cfg.exposure_bonus_baskets == 0)
        {
            return (0, 0);
        }
        let prog = self.stats.balance * self.cfg.exposure_bonus_profit_pct / 100.0;
        if prog <= 0.0 {
            return (0, 0);
        }
        let q = b.quote();
        let plywajacy: f64 = b.positions().iter().map(|p| p.profit_usd(&q)).sum();
        if plywajacy >= prog {
            (
                self.cfg.exposure_bonus_positions,
                self.cfg.exposure_bonus_baskets,
            )
        } else {
            (0, 0)
        }
    }

    #[inline]
    fn basket_max_age_eff(&self) -> f64 {
        self.kap_f(
            self.cfg.basket_max_age_min,
            self.cfg.basket_max_age_min_small,
            self.cfg.basket_max_age_min_small_mult,
        )
    }

    #[inline]
    fn market_step_eff(&self) -> f64 {
        if self.kap_male(self.cfg.market_entry_step_small_mult) {
            self.cfg.market_step_from(self.cfg.market_entry_step_small)
        } else {
            self.cfg.market_step()
        }
    }

    fn adaptive_sl_min_dist(&self, sig_width: f64, ts: Ts) -> f64 {
        let baza = self.kap_f(
            self.cfg.sl_min_dist,
            self.cfg.sl_min_dist_small,
            self.cfg.sl_min_dist_small_mult,
        );
        if !self.cfg.adaptive_params {
            return baza;
        }
        let mut v = 0.0f64;
        let mut zrodlo = false;
        if self.cfg.sl_min_dist_zone_mult > 0.0 && sig_width > 0.0 {
            v = v.max(self.cfg.sl_min_dist_zone_mult * sig_width);
            zrodlo = true;
        }
        if self.cfg.sl_min_dist_atr_mult > 0.0 {
            if let Some(atr) = self.atr_proxy(ts) {
                v = v.max(self.cfg.sl_min_dist_atr_mult * atr);
                zrodlo = true;
            }
        }
        if !zrodlo {
            return baza;
        }
        if self.cfg.sl_min_dist_floor > 0.0 {
            v = v.max(self.cfg.sl_min_dist_floor);
        }
        if self.cfg.sl_min_dist_cap > 0.0 {
            v = v.min(self.cfg.sl_min_dist_cap);
        }
        v
    }

    #[inline]
    fn dystans_do_sl(e: &EntrySignal) -> Option<f64> {
        let sl = e.sl?;
        let krawedz = e.side.better_edge(e.lo.min(e.hi), e.lo.max(e.hi));
        let d = (krawedz - sl) * e.side.sign();
        if d > 0.0 {
            Some(d)
        } else {
            None
        }
    }

    #[inline]
    fn adaptive_deep_offset(&self, sig_width: f64, dyst_do_sl: Option<f64>) -> f64 {
        if self.cfg.entry_deep_frac_to_sl > 0.0 {
            if let Some(d) = dyst_do_sl {
                return d * self.cfg.entry_deep_frac_to_sl;
            }
        }
        if self.cfg.adaptive_params && self.cfg.entry_deep_zone_mult > 0.0 && sig_width > 0.0 {
            self.cfg.entry_deep_zone_mult * sig_width
        } else {
            self.cfg.entry_deep_offset
        }
    }

    fn adaptive_units(&self, base: u32, sig_width: f64, ts: Ts) -> u32 {
        if !self.cfg.adaptive_params {
            return base;
        }
        let mut f = 1.0f64;
        if self.cfg.entry_units_zone_ref > 0.0 && sig_width > 0.0 {
            f *= sig_width / self.cfg.entry_units_zone_ref;
        }
        f *= self
            .cfg
            .hour_units_mult(hour_of(ts, self.cfg.session_offset()));
        if (f - 1.0).abs() < 1e-9 {
            return base;
        }
        let n = (base as f64 * f).round().max(1.0);
        (n as u32).clamp(1, base.saturating_mul(3).max(1))
    }

    #[inline]
    fn scaled_units(&self, base: u32, ts: Ts) -> u32 {
        let f = self.vol_factor(ts);
        if (f - 1.0).abs() < 1e-9 {
            return base.max(1);
        }
        ((base as f64 * f).round() as u32).max(1)
    }

    pub fn market_history(&self) -> (Vec<(Ts, Px)>, Vec<(Ts, Px)>) {
        (self.price_hist.clone(), self.vol_hist.clone())
    }

    pub fn set_market_history(&mut self, price: Vec<(Ts, Px)>, vol: Vec<(Ts, Px)>) {
        self.price_hist = price;
        self.vol_hist = vol;
    }

    pub fn adopt_baskets(&mut self, baskets: Vec<Basket>) {
        for bk in baskets {
            self.remember_pending_source(&bk);
            self.remember_adopted_entry_source(&bk);
            self.next_basket_id = self.next_basket_id.max(bk.id.saturating_add(1));
            self.msg_to_basket
                .insert((bk.source.clone(), bk.msg_id), bk.id);
            for alias in &bk.msg_aliases {
                self.msg_to_basket
                    .insert((bk.source.clone(), *alias), bk.id);
            }
            if self.cfg.dedup_management_po_restarcie {
                for zapis in &bk.persisted_done_actions {
                    let done = self
                        .done_actions
                        .entry((bk.source.clone(), zapis.msg_id))
                        .or_default();
                    for akcja in &zapis.actions {
                        if !done.contains(akcja) {
                            done.push(akcja.clone());
                        }
                    }
                }
            }
            match self.baskets.iter_mut().find(|x| x.id == bk.id) {
                Some(stary) => *stary = bk,
                None => self.baskets.push(bk),
            }
        }
        self.rebuild_basket_slots();
        self.sync_source_tombstones();
    }

    fn persist_done_action(&mut self, basket_id: u32, msg_id: i64, action: &str) {
        let Some(bk) = self.baskets.iter_mut().find(|x| x.id == basket_id) else {
            return;
        };
        let zapis = match bk
            .persisted_done_actions
            .iter_mut()
            .find(|x| x.msg_id == msg_id)
        {
            Some(zapis) => zapis,
            None => {
                bk.persisted_done_actions.push(PersistedDoneActions {
                    msg_id,
                    actions: Vec::new(),
                });
                bk.persisted_done_actions
                    .last_mut()
                    .expect("rekord trwalego dedupu zostal wlasnie dodany")
            }
        };
        if !zapis.actions.iter().any(|x| x == action) {
            zapis.actions.push(action.to_string());
        }
    }

    pub fn entry_source_observations(&self)->&HashMap<(SourceKey,i64),EntrySourceObservation> {
        &self.entry_source_observations
    }
    pub fn take_entry_source_observations(&mut self)->HashMap<(SourceKey,i64),EntrySourceObservation> {
        std::mem::take(&mut self.entry_source_observations)
    }
    pub fn restore_entry_source_observations(&mut self,records:HashMap<(SourceKey,i64),EntrySourceObservation>) {
        self.entry_source_observations=records;
    }

    /// Cumulative newly created baskets; pruning retained history cannot reduce it.
    pub fn created_baskets_count(&self) -> u32 {
        self.created_baskets_count
    }

    pub fn next_basket_id(&self) -> u32 {
        self.next_basket_id
    }

    /// Restore an account-wide allocator floor without adopting another
    /// strategy's baskets or borrowing its performance counters.
    pub fn ensure_next_basket_id_floor(&mut self, floor: u32) -> Result<(), &'static str> {
        if floor < crate::wielosilnik::pierwszy_numer(self.slot)
            || crate::wielosilnik::slot_koszyka(floor) != self.slot {
            return Err("basket allocator floor is outside the engine slot");
        }
        self.next_basket_id = self.next_basket_id.max(floor);
        Ok(())
    }


    pub fn basket_view<B: Broker>(&self, b: &B, id: u32) -> Option<BasketView> {
        let bk = self.basket(id)?;
        let mut v = widok_koszyka(bk, b.positions(), &b.quote());
        v.pending_layers = b
            .pendings()
            .iter()
            .filter(|o| o.basket == Some(bk.id))
            .count() as u32;
        Some(v)
    }

    pub fn basket_views<B: Broker>(&self, b: &B) -> Vec<BasketView> {
        self.baskets
            .iter()
            .filter(|x| x.alive())
            .filter_map(|x| self.basket_view(b, x.id))
            .collect()
    }

    fn update_basket_peaks<B: Broker>(&mut self, b: &B, q: &Quote) {
        if self.baskets.is_empty() {
            return;
        }
        let mut otwarte: HashMap<u32, f64> = HashMap::new();
        for p in b.positions() {
            if let Some(id) = p.basket {
                *otwarte.entry(id).or_insert(0.0) += p.profit_usd(q);
            }
        }
        for bk in self.baskets.iter_mut().filter(|x| x.alive()) {
            let pl = bk.realized + otwarte.get(&bk.id).copied().unwrap_or(0.0);
            if pl > bk.peak_pl_usd {
                bk.peak_pl_usd = pl;
            }
            if bk.tp_touch_ts.len() < bk.tps.len() {
                bk.tp_touch_ts.resize(bk.tps.len(), 0);
            }
            if bk.tp_touch_px.len() < bk.tps.len() {
                bk.tp_touch_px.resize(bk.tps.len(), 0.0);
            }
            for (i, cel) in bk.tps.iter().enumerate() {
                if bk.tp_touch_ts[i] != 0 {
                    continue;
                }
                let dotkniety = match bk.side {
                    Side::Buy => q.bid >= *cel,
                    Side::Sell => q.ask <= *cel,
                };
                if dotkniety {
                    bk.tp_touch_ts[i] = q.ts;
                    bk.tp_touch_px[i] = q.exit(bk.side);
                }
            }
            if bk.sl_touch_ts == 0 {
                if let Some(s) = bk.sl {
                    let przebity = match bk.side {
                        Side::Buy => q.bid <= s,
                        Side::Sell => q.ask >= s,
                    };
                    if przebity {
                        bk.sl_touch_ts = q.ts;
                        bk.sl_touch_px = q.exit(bk.side);
                    }
                }
            }
        }
    }

    fn log(&mut self, ts: Ts, level: u8, text: impl Into<String>) {
        self.logs.push(LogLine {
            ts,
            level,
            text: text.into(),
        });
        if self.logs.len() > 5000 {
            self.logs.drain(0..2500);
        }
    }


    pub fn set_run_id(&mut self, id: impl Into<String>) {
        self.journal.run_id = id.into();
    }

    pub fn drain_journal(&mut self) -> Vec<journal::JournalEvent> {
        self.journal.drain()
    }

    fn jsnap<B: Broker>(&self, b: &B) -> Option<MarketSnapshot> {
        if !self.journal.cfg.snapshots {
            return None;
        }
        Some(MarketSnapshot::build(
            &b.quote(),
            &b.account(),
            b.positions(),
            b.pendings().len(),
            self.stats.peak_equity,
            self.stats.realized_today,
        ))
    }

    fn jreject<B: Broker>(
        &mut self,
        b: &B,
        m: &IncomingMessage,
        action: &str,
        code: RejectCode,
        text: impl Into<String>,
    ) {
        let kod = format!("{code:?}");
        *self.odrzuty.entry(kod.clone()).or_insert(0) += 1;
        if action == "entry" {
            if let Some(w) = &self.wejscie_w_obrobce {
                if w.msg_id == m.msg_id {
                    let mut w = w.clone();
                    w.kod = kod;
                    w.ts = m.ts;
                    self.odrzucone_wejscia.push(w);
                }
            }
        }
        if !self.journal.wants(EventLevel::Warn) {
            return;
        }
        let snap = self.jsnap(b);
        self.journal.push(
            Ev::new(
                m.ts,
                EventLevel::Warn,
                EventCategory::Decision,
                EventKind::SignalRejected,
            )
            .text(text)
            .msg(m.msg_id)
            .signal(journal::signal_id(m.msg_id, action))
            .source(m.source_name.clone())
            .reason(code)
            .market(snap)
            .build(),
        );
    }

    fn jignore<B: Broker>(
        &mut self,
        b: &B,
        m: &IncomingMessage,
        action: &str,
        basket: Option<u32>,
        code: RejectCode,
        text: impl Into<String>,
    ) {
        self.zignorowane += 1;
        if !self.journal.wants(EventLevel::Warn) {
            return;
        }
        let snap = self.jsnap(b);
        self.journal.push(
            Ev::new(
                m.ts,
                EventLevel::Warn,
                EventCategory::Decision,
                EventKind::TargetIgnored,
            )
            .text(text)
            .msg(m.msg_id)
            .signal(journal::signal_id(m.msg_id, action))
            .source(m.source_name.clone())
            .basket_opt(basket)
            .reason(code)
            .market(snap)
            .build(),
        );
    }

    #[allow(dead_code)]
    fn jnote(&mut self, ts: Ts, level: EventLevel, cat: EventCategory, text: impl Into<String>) {
        if !self.journal.wants(level) {
            return;
        }
        self.journal
            .push(Ev::new(ts, level, cat, EventKind::Note).text(text).build());
    }

    fn jguard<B: Broker>(
        &mut self,
        b: &B,
        ts: Ts,
        code: RejectCode,
        text: String,
        wartosc: f64,
        dd_pct: f64,
        prog: f64,
    ) {
        if !self.journal.wants(EventLevel::Error) {
            return;
        }
        let snap = self.jsnap(b);
        self.journal.push(
            Ev::new(
                ts,
                EventLevel::Error,
                EventCategory::Risk,
                EventKind::GuardBlocked,
            )
            .text(text)
            .reason(code)
            .market(snap)
            .put_f("value", wartosc)
            .put_f("threshold", prog)
            .put_f("dd_pct", dd_pct)
            .put_f("peak_equity", self.stats.peak_equity)
            .put_f("day_peak_equity", self.stats.day_peak_equity)
            .put_f("day_start_equity", self.stats.day_start_equity)
            .build(),
        );
    }


    #[inline]
    pub fn kredyt_skuteczny(&self) -> f64 {
        self.cfg.kredyt_skuteczny_z(self.stats.credit)
    }

    #[inline]
    pub fn podstawa_lota(&self) -> f64 {
        self.cfg
            .podstawa_lota_z_konta(self.stats.balance, self.stats.equity, self.stats.credit)
    }

    pub fn kredyt_odliczony_od_podstawy(&self) -> f64 {
        if !self.cfg.credit_balance_separate {
            return self.kredyt_skuteczny();
        }
        let raw = match self.cfg.lot_base {
            crate::settings::PodstawaLota::Balance => self.stats.balance,
            crate::settings::PodstawaLota::Equity => self.stats.equity,
            crate::settings::PodstawaLota::MinOfBoth => self.stats.balance.min(self.stats.equity),
        }
        .max(0.0);
        (raw - self.podstawa_lota()).max(0.0)
    }

    #[inline]
    fn sufit_lota(&self) -> f64 {
        let staly = if self.cfg.lot_max.is_finite() && self.cfg.lot_max > 0.0 {
            self.cfg.lot_max
        } else {
            f64::MAX
        };
        let dz = self.cfg.lot_max_z_salda;
        if dz <= 0.0 {
            return staly;
        }
        let z_salda = self.podstawa_lota() / dz;
        if !z_salda.is_finite() || z_salda <= 0.0 {
            return staly;
        }
        staly.min(z_salda)
    }

    #[inline]
    fn wolumen_zlecenia(&self, v: f64) -> f64 {
        if self.strict_open_volume() {
            return match self.volume_limits().bounds() {
                Ok((_, maximum)) if v.is_finite() && v > 0.0 => v.min(maximum),
                _ => 0.0,
            };
        }
        let c = &self.cfg;
        let dol = if c.lot_min.is_finite() && c.lot_min > 0.0 {
            c.lot_min
        } else {
            0.01
        };
        let gora = self.sufit_lota();
        let (dol, gora) = if dol <= gora {
            (dol, gora)
        } else {
            (gora, dol)
        };
        round_lot(v.max(dol).min(gora))
    }

    fn volume_limits(&self) -> crate::volume_contract::StrategyVolumeLimits {
        crate::volume_contract::StrategyVolumeLimits {
            minimum: self.cfg.lot_min,
            maximum: self.cfg.lot_max,
            capital_per_lot: self.cfg.lot_max_z_salda,
            capital: self.podstawa_lota(),
        }
    }

    fn strict_open_volume(&self) -> bool {
        self.cfg.order_volume_contract_v2 || crate::lot_growth::enabled(&self.cfg)
    }

    fn final_open_volume<B: Broker>(&mut self, b: &B, requested: f64) -> BResult<f64> {
        if !self.strict_open_volume() {
            return Ok(requested);
        }
        let spec = crate::volume_contract::VolumeSpec {
            minimum: b.volume_min(),
            step: b.volume_step(),
            maximum: b.volume_max(),
        };
        match crate::volume_contract::normalize_open_volume(requested, spec, self.volume_limits()) {
            Ok(volume) => Ok(volume),
            Err(reason) => {
                *self
                    .odrzuty
                    .entry(format!("VolumeContract::{reason:?}"))
                    .or_insert(0) += 1;
                self.log(b.quote().ts, 2, format!(
                    "VOLUME CONTRACT: odmowa nowego zlecenia ({reason:?}); requested={requested:?}, \
                     broker min={:?}/step={:?}/max={:?}, strategy min={:?}/max={:?}/capital_per_lot={:?}",
                    spec.minimum, spec.step, spec.maximum,
                    self.cfg.lot_min, self.cfg.lot_max, self.cfg.lot_max_z_salda,
                ));
                Err(BrokerError::InvalidVolume)
            }
        }
    }

    fn cost_entry_blocked<B: Broker>(&self, b: &B) -> Option<&str> {
        if let Some(reason) = self.cost_reconciliation_required.as_deref() {
            return Some(reason);
        }
        if !self.cfg.closed_profit_net_costs {
            return None;
        }
        if !self.cfg.basket_realized_broker_only {
            return Some("canonical net requires basket_realized_broker_only");
        }
        if !b.cost_net_supported() {
            return Some("canonical net pipeline unsupported, inactive or requires review");
        }
        None
    }

    fn latch_cost_fault<B: Broker>(&mut self, b: &mut B, ts: Ts, reason: String) {
        if self.cost_reconciliation_required.is_none() {
            self.log(
                ts,
                2,
                format!("COST HOLD: {reason}; nowe ryzyko zablokowane, wyjścia pozostają czynne"),
            );
            self.cost_reconciliation_required = Some(reason.clone());
        }
        self.halted = Some(format!("COST HOLD: {reason}"));
        b.report_cost_consumer_fault(&reason);
    }

    fn profit_budget_volume<B: Broker>(&mut self, b:&B, basket:Option<u32>, side:Side,
        entry:Px, sl:Option<Px>, requested:f64)->BResult<f64> {
        if self.cfg.profit_budget_arm_pct!=0.0 && self.stats.day==day_of(b.quote().ts,self.cfg.session_offset()) {
            let equity=b.account().equity;
            if equity.is_finite(){self.stats.day_peak_equity=self.stats.day_peak_equity.max(equity);}
        }
        match crate::profit_budget::limit_open_volume(&self.cfg,(&self.stats).into(),b,side,entry,sl,requested) {
            Ok(volume)=>Ok(volume),
            Err(reason)=>{
                let code=format!("ProfitBudget::{reason:?}");
                *self.odrzuty.entry(code.clone()).or_insert(0)+=1;
                self.log(b.quote().ts,1,format!("RISK BUDGET: {reason:?}; new order withheld"));
                if self.journal.wants(EventLevel::Info) {
                    let snap=self.jsnap(b);
                    let mut ev=Ev::new(b.quote().ts,EventLevel::Warn,EventCategory::Risk,EventKind::OrderRejected)
                        .text(format!("RISK BUDGET: {reason:?}; new order withheld"))
                        .reason(RejectCode::RiskBudgetExhausted).market(snap).put("code",code)
                        .put_f("requested_volume",requested).put_f("entry",entry);
                    if let Some(id)=basket {ev=ev.basket(id);}
                    self.journal.push(ev.build());
                }
                Err(BrokerError::Rejected)
            }
        }
    }

    fn growth_error<B: Broker>(&mut self, b:&B, reason:crate::lot_growth::GrowthError) -> BrokerError {
        *self.odrzuty.entry(format!("LotGrowth::{reason:?}")).or_insert(0)+=1;
        self.log(b.quote().ts,1,format!("LOT GROWTH HOLD: {reason:?}; new order withheld"));
        BrokerError::Rejected
    }

    /// A relot target is allocated before subtracting existing exposure. Its
    /// resulting delta/replacement bypasses allocation only, never final caps.
    fn growth_allocated_volume<B: Broker>(&mut self,b:&B,basket:Option<u32>,level:i32,
        side:Side,entry:Px,sl:Option<Px>,requested:f64,already_allocated:bool)->BResult<f64> {
        if !crate::lot_growth::enabled(&self.cfg) {return Ok(requested);}
        if self.ea.beta().is_some() {
            return Err(self.growth_error(b,crate::lot_growth::GrowthError::UnsupportedEngine));
        }
        if let Err(reason)=crate::lot_growth::ready(&self.cfg,b) {return Err(self.growth_error(b,reason));}
        if already_allocated {return Ok(requested);}
        let zone=basket.and_then(|id|self.basket(id)).map(|bk|(bk.entry_lo,bk.entry_hi));
        match crate::lot_growth::allocate(&self.cfg,b,basket,level,side,entry,sl,zone,requested) {
            Ok((volume,fallback))=>{
                if let Some(reason)=fallback {
                    *self.stats.lot_sizing_diagnostics.entry(format!("LotGrowth::UniformFallback::{reason:?}")).or_insert(0)+=1;
                }
                self.growth_context_volume(b,basket,side,entry,sl,volume)
            }
            Err(reason)=>Err(self.growth_error(b,reason)),
        }
    }

    fn growth_budget_volume<B: Broker>(&mut self,b:&B,basket:Option<u32>,side:Side,
        entry:Px,sl:Option<Px>,requested:f64)->BResult<f64> {
        match crate::lot_growth::limit(&self.cfg,b,basket,side,entry,sl,requested) {
            Ok(v)=>Ok(v),Err(reason)=>Err(self.growth_error(b,reason)),
        }
    }

    fn growth_sync_volume<B: Broker>(&mut self,b:&B,id:u32,level:i32,side:Side,
        entry:Px,sl:Option<Px>,raw:f64,want:usize,remaining_slots:usize)->BResult<(f64,bool)> {
        if !crate::lot_growth::enabled(&self.cfg) || !self.cfg.pending_relot_reconcile_target {
            return Ok((raw,false));
        }
        let weighted=self.growth_allocated_volume(b,Some(id),level,side,entry,sl,raw,false)?;
        let per_order=self.final_open_volume(b,weighted)?;
        let positions:f64=b.positions().iter().chain(b.ukryte_pozycje())
            .filter(|p|p.basket==Some(id)&&p.level==level).map(|p|p.volume).sum();
        let pending:f64=b.pendings().iter().chain(b.ukryte_zlecenia())
            .filter(|p|p.basket==Some(id)&&p.level==level).map(|p|p.volume).sum();
        if !positions.is_finite() || !pending.is_finite() || positions<0.0 || pending<0.0 {
            return Err(self.growth_error(b,crate::lot_growth::GrowthError::InvalidExposure));
        }
        let remaining=(per_order*want as f64-positions-pending).max(0.0);
        Ok((per_order.min(remaining/remaining_slots.max(1) as f64),true))
    }

    fn open_market_order<B: Broker>(&mut self, b: &mut B, r: OrderReq) -> BResult<Ticket> {
        self.open_market_order_allocated(b,r,false)
    }

    fn open_market_order_allocated<B: Broker>(&mut self, b: &mut B, mut r: OrderReq,
        already_allocated:bool) -> BResult<Ticket> {
        if self.rearm_confirmation_pending() { return Err(BrokerError::Rejected); }
        if self.continuation_entry_blocked() {
            return Err(BrokerError::Rejected);
        }
        if self.entry_edit_blocks(r.basket) {
            return Err(BrokerError::Rejected);
        }
        if let Some(reason) = self.cost_entry_blocked(b).map(str::to_owned) {
            self.latch_cost_fault(b, b.quote().ts, reason);
            return Err(BrokerError::Rejected);
        }
        if self.relot_entry_requires_review(r.basket, r.level) {
            if let Some(id) = r.basket {
                self.relot_note(id, b.quote().ts, "RequiresReviewEntryBlocked");
            }
            return Err(BrokerError::Rejected);
        }
        r.volume = self.growth_allocated_volume(b,r.basket,r.level,r.side,b.quote().entry(r.side),r.sl,r.volume,already_allocated)?;
        r.volume = self.final_open_volume(b, r.volume)?;
        r.volume = self.profit_budget_volume(b,r.basket,r.side,b.quote().entry(r.side),r.sl,r.volume)?;
        r.volume = self.growth_budget_volume(b,r.basket,r.side,b.quote().entry(r.side),r.sl,r.volume)?;
        self.order_submission_sequence=self.order_submission_sequence.wrapping_add(1);
        b.open_market(r)
    }

    fn place_pending_order<B: Broker>(&mut self, b: &mut B, r: PendingReq) -> BResult<Ticket> {
        self.place_pending_order_allocated(b,r,false)
    }

    fn place_pending_order_allocated<B: Broker>(&mut self, b: &mut B, mut r: PendingReq,
        already_allocated:bool) -> BResult<Ticket> {
        if self.rearm_confirmation_pending() { return Err(BrokerError::Rejected); }
        if self.continuation_entry_blocked() {
            return Err(BrokerError::Rejected);
        }
        if self.entry_edit_blocks(r.basket) {
            return Err(BrokerError::Rejected);
        }
        if let Some(reason) = self.cost_entry_blocked(b).map(str::to_owned) {
            self.latch_cost_fault(b, b.quote().ts, reason);
            return Err(BrokerError::Rejected);
        }
        if self.relot_entry_requires_review(r.basket, r.level) {
            if let Some(id) = r.basket {
                self.relot_note(id, b.quote().ts, "RequiresReviewEntryBlocked");
            }
            return Err(BrokerError::Rejected);
        }
        r.volume = self.growth_allocated_volume(b,r.basket,r.level,r.kind.side(),r.price,r.sl,r.volume,already_allocated)?;
        r.volume = self.final_open_volume(b, r.volume)?;
        r.volume = self.profit_budget_volume(b,r.basket,r.kind.side(),r.price,r.sl,r.volume)?;
        r.volume = self.growth_budget_volume(b,r.basket,r.kind.side(),r.price,r.sl,r.volume)?;
        r.no_market_fallback |= crate::lot_growth::enabled(&self.cfg);
        self.order_submission_sequence=self.order_submission_sequence.wrapping_add(1);
        b.place_pending(r)
    }

    #[inline]
    fn slhit_hamuje(&self, ts: Ts) -> bool {
        self.cfg.slhit_pause_n > 0 && ts < self.slhit_pauza_do
    }

    pub fn lot_size(&self, balance: f64) -> f64 {
        let c = &self.cfg;
        let pct = self.kap_f(c.lot_percent, c.lot_percent_small, c.lot_percent_small_mult);
        let growth=crate::lot_growth::enabled(c);
        let mut lot = if growth {
            match crate::lot_growth::nominal(c,balance) {Ok(v)=>v,Err(_)=>return 0.0}
        } else if c.lot_mode_percent {
            balance * pct / 100.0 / 100.0
        } else {
            c.lot_fixed
        };
        if !growth && c.lot_scale_step > 0.0 {
            let steps = (balance / c.lot_scale_step).floor().max(1.0);
            lot = lot.max(0.01 * steps);
        }
        if c.vol_size_mode != VolSizeMode::Off {
            lot *= self.zmiennosc.mult;
        }
        if self.rezim_miekki && c.regime_soft_lot_mult != 1.0 {
            lot *= c.regime_soft_lot_mult;
        }
        if self.slhit_miekki && c.slhit_pause_lot_mult > 0.0 {
            lot *= c.slhit_pause_lot_mult;
        }
        if self.strict_open_volume() {
            return match self.volume_limits().bounds() {
                Ok((minimum, maximum)) if lot.is_finite() && lot > 0.0 => {
                    if growth {lot.min(maximum)} else {lot.max(minimum).min(maximum)}
                }
                _ => 0.0,
            };
        }
        let dol = if c.lot_min.is_finite() && c.lot_min > 0.0 {
            c.lot_min
        } else {
            0.01
        };
        let gora = self.sufit_lota();
        let (dol, gora) = if dol <= gora {
            (dol, gora)
        } else {
            (gora, dol)
        };
        if !lot.is_finite() {
            lot = dol;
        }
        ((lot.max(dol).min(gora)) * 100.0).round() / 100.0
    }

    pub fn poziomy_wejscia_planowane(&self) -> u32 {
        self.units_base(true).max(self.units_base(false)).max(1)
    }

    pub fn lot_koszyka_planowany(&self, balance: f64) -> f64 {
        let lot = self.lot_size(balance);
        let n = self.poziomy_wejscia_planowane() as usize;
        let na_poziomie = if n > 1 && self.cfg.grid_anchor_absolute && self.cfg.units_per_level {
            n as f64
        } else {
            1.0
        };
        let mult = if self.cfg.entry_weights_from_rr {
            vec![1.0; n]
        } else {
            self.cfg.depth_multipliers(n)
        };
        let mut sztuki: Vec<f64> = Vec::with_capacity(n * na_poziomie as usize);
        for m in &mult {
            let v = self.wolumen_zlecenia(lot * m);
            for _ in 0..(na_poziomie as usize) {
                sztuki.push(v);
            }
        }
        let limit = self.max_open_positions_eff() as usize;
        if self.cfg.enforce_position_limit_on_fill && limit > 0 && sztuki.len() > limit {
            sztuki.sort_by(|a, b| b.partial_cmp(a).unwrap_or(core::cmp::Ordering::Equal));
            sztuki.truncate(limit);
        }
        let suma: f64 = sztuki.iter().sum();
        (suma * 100.0).round() / 100.0
    }


    pub fn on_message<B: Broker>(&mut self, b: &mut B, m: &IncomingMessage) {
        self.reconcile_rearm_batches(b);
        // A rejection may occur before handle_entry; never reuse the previous payload.
        self.wejscie_w_obrobce=None;
        self.continuation_observe(b);
        self.refresh_basket_slots();
        let deferred_replay = self.deferred_entries.is_replay(m);
        if !deferred_replay {
            self.stats.messages += 1;
        }
        cien::puls(m.ts, conduit_mozg_cien::cien::PULS_WIADOMOSC);
        let mut signals = parser::parse_z_opcjami(
            &m.text,
            parser::OpcjeParsera {
                geometryczny: self.cfg.parser_geometryczny,
                min_pewnosc: self.cfg.parser_min_pewnosc,
                rf_wymaga_wykonania: self.cfg.rf_wymaga_wykonania,
                partials_jako_komenda: self.cfg.partials_wykonuj,
                luz_interpunkcyjny: self.cfg.parser_luz_interpunkcyjny,
                recap_guard: self.cfg.recap_guard,
            },
        );
        let telemetry_tp_suppressed = if self.cfg.profit_update_telemetry_only {
            parser::suppress_at_tp_hits(&mut signals, &m.text)
        } else {
            0
        };
        if telemetry_tp_suppressed > 0 {
            self.log(
                m.ts,
                0,
                format!(
                    "AT TP telemetry: pominięto {telemetry_tp_suppressed} wykonawczą akcję TP, zachowano pozostałe intencje"
                ),
            );
        }
        let full_entry=signals.iter().any(|s|matches!(s,Signal::Entry(_)));
        if full_entry || signals.iter().any(|s|matches!(s,Signal::MarketOpen{..})) {
            let observed=self.entry_source_observations.entry((m.source.clone(),m.edit_of.unwrap_or(m.msg_id)))
                .or_insert(EntrySourceObservation{has_full_entry:false,first_seen_as_edit:m.edit_of.is_some()});
            observed.has_full_entry|=full_entry;
        }
        // Every early entry gate uses this CURRENT parsed geometry, including
        // EditOrphan before dispatch. This is diagnostic state only.
        if let Some(e)=signals.iter().find_map(|s|if let Signal::Entry(e)=s{Some(e)}else{None}) {
            self.wejscie_w_obrobce=Some(OdrzuconeWejscie{ts:m.ts,msg_id:m.msg_id,kod:String::new(),
                side:e.side,lo:e.lo,hi:e.hi,sl:e.sl,tp1:e.tps.first().copied()});
        }
        let actionable = !signals.iter().all(|s| matches!(s, Signal::Info));
        if actionable && !deferred_replay {
            self.stats.signals += 1;
        }

        if self.journal.wants(EventLevel::Info) && !deferred_replay {
            let akcje: Vec<String> = signals
                .iter()
                .filter(|s| !matches!(s, Signal::Info))
                .map(|s| s.action_key())
                .collect();
            let snap = self.jsnap(b);
            let skrot: String = m.text.chars().take(280).collect();
            self.journal.push(
                Ev::new(
                    m.ts,
                    EventLevel::Info,
                    EventCategory::Signal,
                    EventKind::MessageReceived,
                )
                .text(skrot)
                .msg(m.msg_id)
                .source(m.source_name.clone())
                .market(snap)
                .put("actions", akcje.join(","))
                .put("actionable", actionable)
                .put("telemetry_tp_suppressed", telemetry_tp_suppressed)
                .put("edit_of", m.edit_of.unwrap_or(0))
                .put("reply_to", m.reply_to.unwrap_or(0))
                .put("chat", m.source.as_string())
                .build(),
            );
        }

        if self.cfg.t100.enabled {
            self.t100_context(m, &signals);
            return;
        }
        self.observe_source_reply(m, &signals);
        let withdrawn=self.entry_source_withdrawn(m);
        let late_new=m.edit_of.is_none() && self.entry_source_record(&m.source,m.msg_id)
            .is_some_and(|r|r.basket_id.is_some() && r.last_entry_edit_ts.is_some());
        if withdrawn || late_new {
            let entries=signals.iter().filter(|s|matches!(s,Signal::Entry(_) | Signal::MarketOpen{..})).count();
            for _ in 0..entries {
                if withdrawn {
                    self.jreject(b,m,"entry",RejectCode::EntryGateBlocked,
                        "ENTRY SOURCE: publisher withdrawal prevents reactivation");
                } else {
                    self.jignore(b,m,"entry",None,RejectCode::DuplicateEditedAction,
                        "ENTRY SOURCE: late NEW cannot replace an already received entry edit");
                }
            }
            signals.retain(|s|!matches!(s,Signal::Entry(_) | Signal::MarketOpen{..}));
        }

        if self.deferred_message(b, m, &signals) {
            return;
        }

        if let Some(orig) = m.edit_of {
            if let Some(&bid) = self.msg_to_basket.get(&(m.source.clone(), orig)) {
                if let Some(poz) = signals.iter().position(|s| matches!(s, Signal::Entry(_))) {
                    let entry = match &signals[poz] {
                        Signal::Entry(e) => e.clone(),
                        _ => unreachable!("position() wskazało Entry"),
                    };
                    let edit_outcome = self.apply_entry_edit(b, bid, &entry, m.ts);
                    if edit_outcome.source_handled() {self.remember_entry_edit(m);}
                    if !self.cfg.edycja_wykonuje_reszte_akcji {
                        return;
                    }
                    signals.remove(poz);
                    let done = self
                        .done_actions
                        .entry((m.source.clone(), orig))
                        .or_default();
                    if edit_outcome.source_handled() && !done.iter().any(|k| k == "entry") {
                        done.push("entry".into());
                    }
                }
            } else {
                let mut filtered=Vec::with_capacity(signals.len());
                for signal in signals {
                    let is_entry=matches!(&signal,Signal::Entry(_) | Signal::MarketOpen{..});
                    let complete=matches!(&signal,Signal::Entry(e) if entry_sources::complete_recoverable_entry(e));
                    if is_entry && (self.cfg.edycja_sieroty_nie_otwiera || !complete) {
                        self.jreject(b,m,"entry",RejectCode::EditOrphan,
                            format!("ENTRY EDIT: source {orig} has no basket; recovery requires the preset policy and a complete protected entry"));
                    } else {
                        if complete {self.log(m.ts,0,"ENTRY EDIT: first complete protected source evaluated at receive time");}
                        filtered.push(signal);
                    }
                }
                signals=filtered;
            }
            if self.cfg.dedup_edited_signals {
                let z_wartoscia = self.cfg.dedup_klucz_z_wartoscia;
                let klucz_akcji = |s: &Signal| -> String {
                    if z_wartoscia {
                        s.action_key_v2()
                    } else {
                        s.action_key()
                    }
                };
                if let Some(done) = self.done_actions.get(&(m.source.clone(), orig)) {
                    let before = signals.len();
                    let pominiete: Vec<String> = signals
                        .iter()
                        .map(&klucz_akcji)
                        .filter(|k| done.contains(k))
                        .collect();
                    signals.retain(|s| !done.contains(&klucz_akcji(s)));
                    if signals.len() < before {
                        self.log(
                            m.ts,
                            0,
                            format!(
                                "edycja wiadomości {orig}: pominięto {} akcji już wykonanych",
                                before - signals.len()
                            ),
                        );
                        for k in &pominiete {
                            self.jreject(
                                b,
                                m,
                                k,
                                RejectCode::DuplicateEditedAction,
                                format!(
                                    "edycja wiadomości {orig} powtarza akcję „{k}\", \
                                     która została już WYKONANA"
                                ),
                            );
                        }
                    }
                }
            }
        }

        if m.edit_of.is_none() && self.cfg.dedup_management_po_restarcie {
            if let Some(done) = self
                .done_actions
                .get(&(m.source.clone(), m.msg_id))
                .cloned()
            {
                let z_wartoscia = self.cfg.dedup_klucz_z_wartoscia;
                let klucz_akcji = |s: &Signal| -> String {
                    if z_wartoscia {
                        s.action_key_v2()
                    } else {
                        s.action_key()
                    }
                };
                let jest_zarzadzaniem = |s: &Signal| {
                    !matches!(
                        s,
                        Signal::Info | Signal::Entry(_) | Signal::MarketOpen { .. }
                    )
                };
                let pominiete: Vec<String> = signals
                    .iter()
                    .filter(|s| jest_zarzadzaniem(s))
                    .map(&klucz_akcji)
                    .filter(|k| done.contains(k))
                    .collect();
                if !pominiete.is_empty() {
                    signals.retain(|s| !jest_zarzadzaniem(s) || !done.contains(&klucz_akcji(s)));
                    self.log(
                        m.ts,
                        0,
                        format!(
                            "ponowna dostawa wiadomości {}: pominięto {} wykonanych akcji zarządzających",
                            m.msg_id,
                            pominiete.len()
                        ),
                    );
                    for k in &pominiete {
                        self.jreject(
                            b,
                            m,
                            k,
                            RejectCode::DuplicateEditedAction,
                            format!(
                                "ponowna dostawa wiadomości {} powtarza akcję „{k}”, która została już WYKONANA",
                                m.msg_id
                            ),
                        );
                    }
                }
            }
        }

        let key = (m.source.clone(), m.edit_of.unwrap_or(m.msg_id));
        for s in signals {
            let persistent_target = if self.cfg.dedup_management_po_restarcie
                && !matches!(
                    &s,
                    Signal::Info | Signal::Entry(_) | Signal::MarketOpen { .. }
                ) {
                self.target_basket(m)
            } else {
                None
            };
            let klucz = (!matches!(s, Signal::Info)).then(|| {
                if self.cfg.dedup_klucz_z_wartoscia {
                    s.action_key_v2()
                } else {
                    s.action_key()
                }
            });
            let odrzuconych_przed: u64 = self.odrzuty.values().sum();
            let zignorowanych_przed = self.zignorowane;
            let is_entry = matches!(&s, Signal::Entry(_));
            self.dispatch(b, m, s);
            if let Some(k) = klucz {
                let wykonana = self.odrzuty.values().sum::<u64>() == odrzuconych_przed
                    && (!self.cfg.dedup_pelny_status || self.zignorowane == zignorowanych_przed)
                    && !(is_entry && self.deferred_entries.blocks_entry_done(&key));
                if wykonana {
                    if self.cfg.dedup_management_po_restarcie {
                        let done = self.done_actions.entry(key.clone()).or_default();
                        if !done.contains(&k) {
                            done.push(k.clone());
                        }
                        if let Some(basket_id) = persistent_target {
                            self.persist_done_action(basket_id, key.1, &k);
                        }
                    } else {
                        self.done_actions.entry(key.clone()).or_default().push(k);
                    }
                }
            }
        }
        if self.done_actions.len() > 2000 {
            let mut keys: Vec<(SourceKey, i64)> = self.done_actions.keys().cloned().collect();
            keys.sort_unstable_by_key(|k| (k.1, k.0.chat_id, k.0.topic_id));
            for k in keys.into_iter().take(1000) {
                self.done_actions.remove(&k);
            }
        }
    }

    fn cel_ze_strefy_przeciwnej<B: Broker>(
        &mut self,
        b: &mut B,
        m: &IncomingMessage,
        e: &EntrySignal,
    ) {
        use crate::settings::CelZPrzeciwnego as C;
        let ids: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && x.source == m.source && x.side != e.side)
            .filter(|x| !self.basket_exit_pending(x.id))
            .map(|x| x.id)
            .collect();
        if ids.is_empty() {
            return;
        }
        let (lo, hi) = (e.lo.min(e.hi), e.lo.max(e.hi));
        let zapas = self.cfg.cel_z_przeciwnego_zapas.max(0.0);
        for id in ids {
            let Some(bk) = self.basket(id) else { continue };
            let strona = bk.side;
            let cel = match (self.cfg.cel_z_przeciwnego, strona) {
                (C::Off, _) => continue,
                (C::BliższaKrawedz, Side::Buy) => lo - zapas,
                (C::BliższaKrawedz, Side::Sell) => hi + zapas,
                (C::DalszaKrawedz, Side::Buy) => hi - zapas,
                (C::DalszaKrawedz, Side::Sell) => lo + zapas,
                (C::Srodek, Side::Buy) => (lo + hi) / 2.0 - zapas,
                (C::Srodek, Side::Sell) => (lo + hi) / 2.0 + zapas,
            };
            let q = b.quote();
            let sensowny = match strona {
                Side::Buy => cel > q.ask,
                Side::Sell => cel < q.bid,
            };
            if !sensowny {
                self.basket_note(
                    id,
                    m.ts,
                    format!("cel ze strefy przeciwnej {cel:.2} odrzucony — po złej stronie rynku"),
                );
                continue;
            }
            let tickety: Vec<Ticket> = self
                .basket(id)
                .map(|x| x.tickets.clone())
                .unwrap_or_default();
            let mut n = 0usize;
            for t in tickety {
                let sl = b.find_position(t).and_then(|p| p.sl);
                cien::z(cakt::A_CEL, t, czr::Z_CEL_ZE_STREFY_PRZECIWNEJ, 0);
                if b.modify_position(t, sl, Some(cel)).is_ok() {
                    n += 1;
                }
            }
            if n > 0 {
                self.basket_note(
                    id,
                    m.ts,
                    format!(
                        "CEL ZE STREFY PRZECIWNEJ: {n} poz. → TP {cel:.2} \
                         (strefa nadawcy {lo:.2}–{hi:.2})"
                    ),
                );
            }
        }
    }

    fn dispatch<B: Broker>(&mut self, b: &mut B, m: &IncomingMessage, s: Signal) {
        match s {
            Signal::Entry(e) => {
                if self.cfg.exit_on_opposite_signal && !self.deferred_entries.protection_done(m) {
                    self.close_opposite_baskets(b, m, e.side);
                }
                if self.cfg.cel_z_przeciwnego != crate::settings::CelZPrzeciwnego::Off
                    && !self.deferred_entries.protection_done(m)
                {
                    self.cel_ze_strefy_przeciwnej(b, m, &e);
                }
                self.handle_entry(b, m, e);
                self.rezim_miekki = false;
                self.slhit_miekki = false;
            }
            Signal::TpHit { index } => {
                let key = match index {
                    Some(i) => format!("tp{i}"),
                    None => "tp".to_string(),
                };
                if let Some(id) = self.target_or_note(b, m, &key) {
                    self.obs
                        .na_komunikacie(id, m.ts, crate::obserwacje::Komunikat::TpHit);
                    let mut index = index;
                    if self.cfg.tp_hit_match_level && index.is_none() {
                        if let Some(v) = parser::hit_level(&m.text) {
                            let tol = self.cfg.basket_hint_tolerance.max(0.5);
                            if let Some(bk) = self.basket(id) {
                                if let Some(i) = bk
                                    .tps
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, t)| (**t - v).abs() <= tol)
                                    .map(|(i, _)| i + 1)
                                    .max()
                                {
                                    index = Some(i);
                                }
                            }
                        }
                    }
                    let sprawdzenie = if self.cfg.tp_price_only_strict
                        && self.cfg.tp_source == TpSource::PriceOnly
                    {
                        Err(RejectCode::DisabledBySetting)
                    } else if self.cfg.tp_unindexed_pips_require_price
                        && index.is_none()
                        && parser::unindexed_pips_hit(&m.text)
                    {
                        self.signal_tp_price_check(b, id, index)
                    } else {
                        self.signal_tp_check(b, id, index)
                    };
                    match sprawdzenie {
                        Ok(()) => self.handle_tp_hit(b, id, index, m.ts, "kanał"),
                        Err(code) => self.jignore(
                            b,
                            m,
                            &key,
                            Some(id),
                            code,
                            "komunikat o trafionym celu nieprzyjęty",
                        ),
                    }
                }
            }
            Signal::SlHit => {
                self.slhit_dnia += 1;
                if self.cfg.slhit_pause_n > 0
                    && self.slhit_dnia >= self.cfg.slhit_pause_n
                    && m.ts >= self.slhit_pauza_do
                {
                    self.slhit_pauza_do = if self.cfg.slhit_pause_min > 0.0 {
                        m.ts + (self.cfg.slhit_pause_min * 60_000.0) as i64
                    } else {
                        i64::MAX // do końca doby — zdejmowane na granicy doby
                    };
                    self.log(
                        m.ts,
                        1,
                        format!(
                            "HAMULEC SL-HIT: {} stopów kanału w dobie — pauza wejść {}",
                            self.slhit_dnia,
                            if self.cfg.slhit_pause_min > 0.0 {
                                format!("{:.0} min", self.cfg.slhit_pause_min)
                            } else {
                                "do końca doby".into()
                            }
                        ),
                    );
                }
                if let Some(id) = self.target_or_note(b, m, "sl") {
                    self.obs
                        .na_komunikacie(id, m.ts, crate::obserwacje::Komunikat::SlHit);
                    self.handle_sl_hit(b, id, m.ts);
                }
            }
            Signal::RiskFree { level } => {
                if let Some(id) = self.target_or_note(b, m, "rf") {
                    let absurd = level.is_some_and(|v| {
                        !self.rf_level_plausible(b, id, v, self.cfg.rf_level_sanity_max_usd)
                    });
                    if absurd {
                        self.jignore(
                            b,
                            m,
                            "rf",
                            Some(id),
                            RejectCode::ManagementLevelInsane,
                            "RISK FREE odrzucony: poziom rażąco daleko od rynku i koszyka",
                        );
                    } else {
                        self.obs
                            .na_komunikacie(id, m.ts, crate::obserwacje::Komunikat::RiskFree);
                        self.handle_risk_free(b, id, level, m.ts);
                    }
                }
            }
            Signal::OutAtEntry => {
                if let Some(id) = self.target_or_note(b, m, "oae") {
                    self.obs
                        .na_komunikacie(id, m.ts, crate::obserwacje::Komunikat::OutAtEntry);
                    self.handle_out_at_entry(b, id, m.ts);
                }
            }
            Signal::Cancel => {
                if let Some(reply)=m.reply_to {
                    if self.entry_source_record(&m.source,reply).is_some_and(|r|r.basket_id.is_none()) {
                        self.jignore(b,m,"cancel",None,RejectCode::NoTargetBasket,
                            "ENTRY SOURCE: bound CANCEL saved before its source entry; no unrelated basket selected");
                        return;
                    }
                }
                let explicit_target = self.target_basket(m).filter(|id| self.explicit_pending_source(*id));
                let known_reply = m.reply_to.is_some_and(|r| self.msg_to_basket.contains_key(&(m.source.clone(), r)));
                if explicit_target.is_some() && !known_reply {
                    self.jignore(b, m, "cancel", explicit_target, RejectCode::NoTargetBasket,
                        "AMBIGUOUS CANCEL: explicit pending withdrawal needs a reply to its known source message");
                    return;
                }
                if self.cfg.honor_cancel || explicit_target.is_some() {
                    if let Some(id) = self.target_or_note(b, m, "cancel") {
                        self.obs
                            .na_komunikacie(id, m.ts, crate::obserwacje::Komunikat::Cancel);
                        let n = if self.explicit_pending_source(id) {
                            self.withdraw_pending_source(b, id, m.ts)
                        } else { self.cancel_pendings(b, id) };
                        self.basket_note(
                            id,
                            m.ts,
                            format!("CANCEL z kanału — skasowano {n} limitów"),
                        );
                        if self.journal.wants(EventLevel::Info) {
                            let snap = self.jsnap(b);
                            self.journal.push(
                                Ev::new(
                                    m.ts,
                                    EventLevel::Info,
                                    EventCategory::Order,
                                    EventKind::PendingCancelled,
                                )
                                .text(format!("CANCEL z kanału — skasowano {n} limitów"))
                                .basket(id)
                                .msg(m.msg_id)
                                .signal(journal::signal_id(m.msg_id, "cancel"))
                                .market(snap)
                                .put("cancelled", n as u64)
                                .build(),
                            );
                        }
                    }
                } else {
                    self.jignore(
                        b,
                        m,
                        "cancel",
                        None,
                        RejectCode::DisabledBySetting,
                        "CANCEL z kanału wyłączony ustawieniem",
                    );
                }
            }
            Signal::CloseAll => {
                if self.cfg.honor_close_all {
                    match self.cfg.close_all_scope {
                        crate::settings::CloseAllScope::Global => {
                            self.close_everything(b, m.ts, CloseReason::BasketClose);
                            self.log(m.ts, 2, "CLOSE ALL z kanału");
                        }
                        crate::settings::CloseAllScope::Basket => {
                            if let Some(id) = self.target_or_note(b, m, "closeall") {
                                let (zamkniete, skasowane, wynik) =
                                    self.close_basket(b, id, m.ts, CloseReason::BasketClose);
                                self.log(
                                    m.ts,
                                    2,
                                    format!(
                                        "CLOSE ALL z kanału — koszyk B{id}: {zamkniete} poz. {wynik:+.2} $, skasowano {skasowane} limitów"
                                    ),
                                );
                            }
                        }
                    }
                } else {
                    self.jignore(
                        b,
                        m,
                        "closeall",
                        None,
                        RejectCode::DisabledBySetting,
                        "CLOSE ALL z kanału wyłączony ustawieniem",
                    );
                }
            }
            Signal::TakePartials => {
                if let Some(id) = self.target_or_note(b, m, "partials") {
                    self.inkasuj_partials(b, id, m.ts);
                }
            }
            Signal::SecuringPartial {
                targets,
                sl,
                spp_be_level,
            } => {
                if let Some(id) = self.target_or_note(b, m, "spp") {
                    self.obs
                        .na_komunikacie(id, m.ts, crate::obserwacje::Komunikat::Spp);
                    let too_old = self.cfg.spp_max_age_h > 0.0
                        && self
                            .basket(id)
                            .map(|bk| {
                                (m.ts - bk.created_ts) as f64 > self.cfg.spp_max_age_h * 3_600_000.0
                            })
                            .unwrap_or(false);
                    if too_old {
                        self.basket_note(
                            id,
                            m.ts,
                            format!(
                                "SPP pominięty — koszyk starszy niż {:.0} h",
                                self.cfg.spp_max_age_h
                            ),
                        );
                        self.jignore(
                            b,
                            m,
                            "spp",
                            Some(id),
                            RejectCode::SppTooOld,
                            format!(
                                "SPP pominięty — koszyk starszy niż {:.0} h",
                                self.cfg.spp_max_age_h
                            ),
                        );
                        return;
                    }
                    if !targets.is_empty() && !self.cfg.spp_keep_tp {
                        let strona = self.basket(id).map(|bk| bk.side);
                        if strona.map(|sd| self.cele_spojne(sd, &targets)) == Some(true) {
                            if let Some(bk) = self.basket_mut(id) {
                                let stara = std::mem::replace(&mut bk.tps, targets);
                                if stara != bk.tps {
                                    bk.zeruj_postep();
                                }
                            }
                        } else {
                            self.basket_note(
                                id,
                                m.ts,
                                format!(
                                    "CELE ODRZUCONE — {targets:?} niespójne z kierunkiem {strona:?};                                      to zapowiedź nadawcy dla JEGO pozycji, nie nasze cele"
                                ),
                            );
                        }
                    }
                    if let Some(v) = sl {
                        self.set_basket_sl(b, id, v, m.ts);
                    }
                    if self.cfg.spp_sl_mode != SppSlMode::Off {
                        if let Some(lvl) = spp_be_level {
                            self.zastosuj_spp_be(b, id, lvl, m.ts);
                        }
                    }
                    let ma_poz = self.basket(id).map(|x| x.ma_pozycje()).unwrap_or(false);
                    if ma_poz {
                        if let Some(bk) = self.basket_mut(id) {
                            bk.secured = true;
                        }
                        if self.cfg.spp_arms_runner_clock {
                            if let Some(bk) = self.basket_mut(id) {
                                if bk.secured_ts == 0 {
                                    bk.secured_ts = m.ts;
                                }
                            }
                        }
                    } else {
                        if self.cfg.spp_blocks_rearm_when_flat {
                            if let Some(bk) = self.basket_mut(id) {
                                bk.rearm_blocked_by_spp = true;
                            }
                        }
                        self.basket_note(
                            id,
                            m.ts,
                            "SPP przyjęty jako NOWY PLAN, ale koszyk nie ma pozycji — \
                             nie ma czego zabezpieczać (`secured` bez zmian)"
                                .into(),
                        );
                    }
                    self.handle_tp_hit(b, id, None, m.ts, "SPP");
                }
            }
            Signal::MarketOpen { side } => {
                if self.cfg.honor_market_open {
                    self.handle_market_open(b, m, side);
                    self.slhit_miekki = false;
                } else {
                    self.log(
                        m.ts,
                        0,
                        format!("„{side:?} NOW\" zignorowane — brak SL i celów w komunikacie"),
                    );
                    self.jreject(
                        b,
                        m,
                        &format!("mkt{side:?}"),
                        RejectCode::DisabledBySetting,
                        format!("„{side:?} NOW\" zignorowane — brak SL i celów w komunikacie"),
                    );
                }
            }
            Signal::TpCorrection { index, value } => {
                if let Some(id) = self.target_or_note(b, m, &format!("corr{index}")) {
                    if let Some(bk) = self.basket_mut(id) {
                        if index >= 1 && index <= bk.tps.len() {
                            bk.tps[index - 1] = value;
                        } else if index == bk.tps.len() + 1 {
                            bk.tps.push(value);
                        }
                    }
                    if self.cfg.tp_correction_to_broker {
                        self.retarget(b, id, m.ts);
                        let cel = self.basket(id).and_then(|bk| {
                            bk.tps
                                .get(bk.tp_stage)
                                .copied()
                                .or_else(|| bk.tps.last().copied())
                        });
                        if let Some(v) = cel {
                            let do_zmiany: Vec<(Ticket, Px, Option<Px>)> = b
                                .pendings()
                                .iter()
                                .filter(|o| o.basket == Some(id))
                                .map(|o| (o.ticket, o.price, o.sl))
                                .collect();
                            for (t, price, sl) in do_zmiany {
                                cien::z(
                                    cakt::A_KSZTALT_ZLEC,
                                    t,
                                    czr::Z_KANAL_TP_KOREKTA_PENDING,
                                    0,
                                );
                                let _ = b.modify_pending(t, price, sl, Some(v));
                            }
                        }
                    }
                    self.basket_note(id, m.ts, format!("korekta TP{index} → {value:.2}"));
                }
            }
            Signal::SetSl { value } => {
                if let Some(id) = self.target_or_note(b, m, "setsl") {
                    self.set_basket_sl(b, id, value, m.ts);
                }
            }
            Signal::BreakEven => {
                if let Some(id) = self.target_or_note(b, m, "be") {
                    self.obs
                        .na_komunikacie(id, m.ts, crate::obserwacje::Komunikat::Be);
                    self.move_basket_to_be(b, id, m.ts);
                }
            }
            Signal::Info => {}
        }
    }

    fn signal_tp_check<B: Broker>(
        &self,
        b: &B,
        basket_id: u32,
        index: Option<usize>,
    ) -> Result<(), RejectCode> {
        match self.cfg.tp_source {
            TpSource::PriceOnly => Err(RejectCode::DisabledBySetting),
            TpSource::SignalOnly | TpSource::Either => Ok(()),
            TpSource::SignalConfirmedByPrice => self.signal_tp_price_check(b, basket_id, index),
            TpSource::PriceFirstSignalWindow => {
                let bk = self.basket(basket_id).ok_or(RejectCode::BasketNotFound)?;
                let stage = index.unwrap_or(bk.tp_stage + 1).max(1);
                let tp = *bk.tps.get(stage - 1).ok_or(RejectCode::TpIndexOutOfRange)?;
                let q = b.quote();
                let dotkniecie = bk.tp_touch_ts.get(stage - 1).copied().unwrap_or(0);

                if dotkniecie > 0 {
                    let lag = self.cfg.tp_signal_max_lag_s;
                    if lag > 0.0 {
                        let spoznienie = (q.ts - dotkniecie) as f64 / 1000.0;
                        if spoznienie > lag {
                            return Err(RejectCode::TpNotConfirmedByPrice);
                        }
                    }
                    return Ok(());
                }

                if self.cfg.tp_signal_max_lead_s <= 0.0 {
                    return Err(RejectCode::TpNotConfirmedByPrice);
                }
                let tol = self.cfg.tp_price_tolerance;
                let blisko = match bk.side {
                    Side::Buy => q.bid >= tp - tol,
                    Side::Sell => q.ask <= tp + tol,
                };
                if blisko {
                    Ok(())
                } else {
                    Err(RejectCode::TpNotConfirmedByPrice)
                }
            }
        }
    }

    fn signal_tp_price_check<B: Broker>(
        &self,
        b: &B,
        basket_id: u32,
        index: Option<usize>,
    ) -> Result<(), RejectCode> {
        let bk = self.basket(basket_id).ok_or(RejectCode::BasketNotFound)?;
        let stage = index.unwrap_or(bk.tp_stage + 1).max(1);
        let tp = *bk.tps.get(stage - 1).ok_or(RejectCode::TpIndexOutOfRange)?;
        let q = b.quote();
        let tol = self.cfg.tp_price_tolerance;
        let ok = match bk.side {
            Side::Buy => q.bid >= tp - tol,
            Side::Sell => q.ask <= tp + tol,
        };
        if ok {
            Ok(())
        } else {
            Err(RejectCode::TpNotConfirmedByPrice)
        }
    }

    fn rf_level_plausible<B: Broker>(
        &self,
        b: &B,
        basket_id: u32,
        level: Px,
        max_gap: f64,
    ) -> bool {
        if max_gap <= 0.0 {
            return true;
        }
        let Some(bk) = self.basket(basket_id) else {
            return false;
        };
        let q = b.quote();
        let mut best = (level - q.bid).abs().min((level - q.ask).abs());
        best = best
            .min((level - bk.zone_lo).abs())
            .min((level - bk.zone_hi).abs());
        if let Some(sl) = bk.sl {
            best = best.min((level - sl).abs());
        }
        for tp in &bk.tps {
            best = best.min((level - *tp).abs());
        }
        best <= max_gap
    }

    fn target_or_note<B: Broker>(
        &mut self,
        b: &B,
        m: &IncomingMessage,
        action: &str,
    ) -> Option<u32> {
        match self.target_basket(m) {
            Some(id) => {
                if self.basket_exit_pending(id) {
                    self.jignore(b, m, action, Some(id), RejectCode::Halted,
                        "koszyk potwierdza obowiązkowe wyjście; zarządzanie nie może odwołać zamknięcia");
                    return None;
                }
                if self.cfg.reply_graph_transitive {
                    self.msg_to_basket.insert((m.source.clone(), m.msg_id), id);
                    if let Some(bk) = self.baskets.iter_mut().find(|x| x.id == id) {
                        if m.msg_id != bk.msg_id && !bk.msg_aliases.contains(&m.msg_id) {
                            bk.msg_aliases.push(m.msg_id);
                        }
                    }
                    self.remember_pending_source_alias(id, m.msg_id);
                    self.remember_entry_alias(id, m.msg_id);
                }
                Some(id)
            }
            None => {
                self.jignore(
                    b,
                    m,
                    action,
                    None,
                    RejectCode::NoTargetBasket,
                    "brak żywego koszyka z tego źródła — komunikat nie ma do czego się odnieść",
                );
                None
            }
        }
    }

    fn target_basket(&self, m: &IncomingMessage) -> Option<u32> {
        if let Some(r) = m.reply_to {
            if let Some(&id) = self.msg_to_basket.get(&(m.source.clone(), r)) {
                return Some(id);
            }
            if self.cfg.reply_veto {
                return None;
            }
        }

        let mine: Vec<&Basket> = self
            .baskets
            .iter()
            .filter(|x| x.source == m.source && x.alive())
            .collect();
        if mine.is_empty() {
            return None;
        }

        let tol = self.cfg.basket_hint_tolerance;
        if tol > 0.0 {
            let hints = parser::basket_hints(&m.text);
            for h in &hints {
                if let Some(bk) = mine.iter().rev().find(|bk| {
                    let in_zone = *h >= bk.zone_lo - tol && *h <= bk.zone_hi + tol;
                    let is_target = bk.tps.iter().any(|t| (t - h).abs() <= tol);
                    let is_sl = bk.sl.map(|s| (s - h).abs() <= tol).unwrap_or(false);
                    in_zone || is_target || is_sl
                }) {
                    return Some(bk.id);
                }
            }
            if self.cfg.hint_veto && !hints.is_empty() {
                return None;
            }
        }

        mine.iter()
            .rev()
            .find(|x| !x.tickets.is_empty())
            .or_else(|| mine.last())
            .map(|x| x.id)
    }

    fn handle_market_open<B: Broker>(&mut self, b: &mut B, m: &IncomingMessage, side: Side) {
        // A market command is an entry too. Its source identity survives the
        // volatile action/content caches through the persisted basket map.
        let source_id = m.edit_of.unwrap_or(m.msg_id);
        if self.entry_source_withdrawn(m) || self.msg_to_basket.get(&(m.source.clone(), source_id))
            .is_some_and(|id| self.pending_source_cancelled(*id)) {
            self.jreject(b, m, "market", RejectCode::EntryGateBlocked,
                "publisher cancelled this source; changing it to MARKET cannot reactivate it");
            return;
        }

        if (self.cfg.entry_idempotencja || m.edit_of.is_some()
            || self.entry_source_record(&m.source,source_id).is_some_and(|r|r.first_entry_was_edit))
            && self.msg_to_basket.contains_key(&(m.source.clone(), source_id))
        {
            self.jignore(b, m, &format!("mkt{side:?}"),
                self.msg_to_basket.get(&(m.source.clone(), source_id)).copied(),
                RejectCode::DuplicateEditedAction,
                "market command already belongs to a basket; edit/re-delivery cannot open again");
            return;
        }
        self.slhit_miekki = self.slhit_hamuje(m.ts) && self.cfg.slhit_pause_lot_mult > 0.0;
        if self.wygaszanie {
            *self.odrzuty.entry("Wygaszanie".to_string()).or_insert(0) += 1;
            return;
        }
        let gate = self.entry_gate(b, m.ts);
        if let Some((r, code)) = gate.blocked() {
            let (r, code) = (r.to_string(), code);
            self.log(m.ts, 2, format!("„{side:?} NOW\" pominięte: {r}"));
            self.jreject(
                b,
                m,
                &format!("mkt{side:?}"),
                code,
                format!("„{side:?} NOW\" pominięte: {r}"),
            );
            return;
        }
        let q = b.quote();
        let px = q.entry(side);
        let (tps, sl) = self
            .baskets
            .iter()
            .rev()
            .find(|x| x.source == m.source && x.side == side && x.alive())
            .map(|x| (x.tps.clone(), x.sl))
            .unwrap_or_default();

        let id = self.next_basket_id;
        self.next_basket_id += 1;
        self.created_baskets_count += 1;
        let mut bk = new_basket(id, m, side, false, px, px, px, px, sl, tps.clone(), m.ts);
        bk.events.push(BasketEvent {
            ts: m.ts,
            text: format!("otwarcie rynkowe z komunikatu „{side:?} NOW\" @ {px:.2}"),
        });
        self.baskets.push(bk);
        self.msg_to_basket.insert((m.source.clone(), m.msg_id), id);
        self.remember_entry_source(m,id);
        self.obs.na_sygnale(m.ts, side);

        let lot = self.lot_size(self.podstawa_lota());
        let tp = tps.last().copied();
        cien::z(cakt::A_PLAN_PROBY, id as u64, czr::Z_MARKET_OPEN, 0);
        if let Ok(t) = self.open_market_order(
            b,
            OrderReq {
                side,
                volume: lot,
                sl: self.broker_sl(sl, side),
                tp,
                basket: Some(id),
                level: 0,
                is_toucher: false,
                comment: format!("B{id}"),
            },
        ) {
            if let Some(bk) = self.basket_mut(id) {
                bk.tickets.push(t);
                bk.state = BasketState::Working;
                bk.had_positions = true;
                bk.last_entry_px = Some(px);
            }
            self.apply_virtual_sl(b, t, sl);
            diag_wejscie(m.ts, id, "sygnal-NOW", side, px, px, px, px, 0);
        }
    }

    #[inline]
    fn basket_mut(&mut self, id: u32) -> Option<&mut Basket> {
        if let Some(slot) = self.cached_basket_slot(id) {
            return self.baskets.get_mut(slot);
        }
        self.baskets.iter_mut().find(|x| x.id == id)
    }
    #[inline]
    fn basket(&self, id: u32) -> Option<&Basket> {
        if let Some(slot) = self.cached_basket_slot(id) {
            return self.baskets.get(slot);
        }
        self.baskets.iter().find(|x| x.id == id)
    }

    #[inline]
    fn cached_basket_slot(&self, id: u32) -> Option<usize> {
        self.basket_slots.get(&id).copied().filter(|&slot| {
            self.baskets
                .get(slot)
                .map(|bk| bk.id == id)
                .unwrap_or(false)
        })
    }

    #[inline]
    fn refresh_basket_slots(&mut self) {
        if self.basket_slots_len != self.baskets.len() {
            self.rebuild_basket_slots();
        }
    }

    fn rebuild_basket_slots(&mut self) {
        self.basket_slots.clear();
        for (slot, bk) in self.baskets.iter().enumerate() {
            if self.basket_slots.insert(bk.id, slot).is_some() {
                self.basket_slots.clear();
                self.basket_slots_len = self.baskets.len();
                return;
            }
        }
        self.basket_slots_len = self.baskets.len();
    }

    #[inline]
    fn price_tp_slots(&self) -> Vec<(usize, u32)> {
        self.baskets
            .iter()
            .enumerate()
            .filter(|(_, bk)| bk.alive())
            .filter(|(_, bk)| !self.cfg.confirmed_exit_retry || bk.pending_exit.is_none())
            .map(|(slot, bk)| (slot, bk.id))
            .collect()
    }

    #[inline]
    fn basket_exit_pending(&self, id: u32) -> bool {
        self.cfg.confirmed_exit_retry
            && self
                .basket(id)
                .map(|b| b.pending_exit.is_some())
                .unwrap_or(false)
    }

    #[inline]
    fn book_command_profit_legacy(&mut self, id: u32, profit: f64) {
        if !self.cfg.basket_realized_broker_only && !self.cfg.closed_profit_net_costs {
            if let Some(bk) = self.basket_mut(id) {
                bk.realized += profit;
            }
        }
    }

    fn basket_note(&mut self, id: u32, ts: Ts, text: String) {
        if let Some(bk) = self.basket_mut(id) {
            bk.events.push(BasketEvent { ts, text });
            if bk.events.len() > 200 {
                bk.events.drain(0..100);
            }
        }
    }


    fn close_opposite_baskets<B: Broker>(&mut self, b: &mut B, m: &IncomingMessage, nowa: Side) {
        let przeciwne: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && x.side != nowa && x.source == m.source)
            .map(|x| x.id)
            .collect();
        if przeciwne.is_empty() {
            return;
        }
        for id in przeciwne {
            if self.cfg.confirmed_exit_retry {
                self.request_confirmed_exit(b, id, m.ts, CloseReason::BasketClose);
                continue;
            }
            let tickety: Vec<Ticket> = b
                .positions()
                .iter()
                .filter(|p| p.basket == Some(id) && !p.frozen)
                .map(|p| p.ticket)
                .collect();
            let mut wynik = 0.0;
            let mut zamkniete = 0usize;
            for t in tickety {
                cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_CLOSE_OPPOSITE, 0);
                if let Ok(z) = b.close_position(t, CloseReason::BasketClose) {
                    wynik += z;
                    zamkniete += 1;
                }
            }
            let skasowane = self.cancel_pendings(b, id);
            self.book_command_profit_legacy(id, wynik);
            if let Some(bk) = self.basket_mut(id) {
                bk.state = BasketState::Done;
            }
            self.basket_note(
                id,
                m.ts,
                format!(
                    "koszyk zamknięty sygnałem przeciwnym ({nowa:?}) — {zamkniete} poz. \
                     {wynik:+.2} $, skasowano {skasowane} limitów"
                ),
            );
            if self.journal.wants(EventLevel::Warn) {
                let snap = self.jsnap(b);
                self.journal.push(
                    Ev::new(
                        m.ts,
                        EventLevel::Warn,
                        EventCategory::Basket,
                        EventKind::BasketClosed,
                    )
                    .text(format!(
                        "koszyk B{id} zamknięty — kanał wysłał sygnał przeciwny ({nowa:?})"
                    ))
                    .basket(id)
                    .msg(m.msg_id)
                    .signal(journal::signal_id(m.msg_id, "entry"))
                    .source(m.source_name.clone())
                    .market(snap)
                    .put("closed", zamkniete as u64)
                    .put("cancelled", skasowane as u64)
                    .put_f("realized", wynik)
                    .build(),
                );
            }
        }
    }

    fn handle_entry<B: Broker>(&mut self, b: &mut B, m: &IncomingMessage, e: EntrySignal) {
        let ts = m.ts;
        if self.entry_source_withdrawn(m) || self.msg_to_basket.get(&(m.source.clone(), m.edit_of.unwrap_or(m.msg_id)))
            .is_some_and(|id| self.pending_source_cancelled(*id)) {
            self.jreject(b, m, "entry", RejectCode::EntryGateBlocked,
                "publisher cancelled this explicit pending source; re-delivery cannot reactivate it");
            return;
        }


        self.slhit_miekki = self.slhit_hamuje(ts) && self.cfg.slhit_pause_lot_mult > 0.0;

        self.wejscie_w_obrobce = Some(OdrzuconeWejscie {
            ts,
            msg_id: m.msg_id,
            kod: String::new(),
            side: e.side,
            lo: e.lo,
            hi: e.hi,
            sl: e.sl,
            tp1: e.tps.first().copied(),
        });

        if (self.cfg.entry_idempotencja || self.entry_source_record(&m.source,m.msg_id)
            .is_some_and(|r|r.first_entry_was_edit)) && m.edit_of.is_none() {
            if let Some(&bid) = self.msg_to_basket.get(&(m.source.clone(), m.msg_id)) {
                self.log(
                    ts,
                    1,
                    format!(
                        "wiadomość {} już otworzyła koszyk B{bid} — powtórka \
                         (re-delivery) potraktowana jak edycja",
                        m.msg_id
                    ),
                );
                self.apply_entry_edit(b, bid, &e, ts);
                return;
            }
        }

        if self.wygaszanie {
            *self.odrzuty.entry("Wygaszanie".to_string()).or_insert(0) += 1;
            return;
        }

        if let Some(r) = &self.halted {
            let r = r.clone();
            self.log(ts, 2, format!("sygnał pominięty — handel wstrzymany: {r}"));
            self.jreject(
                b,
                m,
                "entry",
                RejectCode::Halted,
                format!("sygnał pominięty — handel wstrzymany: {r}"),
            );
            return;
        }
        if self.cfg.only_limit_signals && !e.is_limit {
            self.log(ts, 0, "sygnał rynkowy pominięty (tylko LIMIT)");
            self.jreject(
                b,
                m,
                "entry",
                RejectCode::MarketSignalWhileLimitOnly,
                "sygnał rynkowy pominięty — ustawienie „tylko LIMIT\"",
            );
            return;
        }
        if let Some(reason) = self.tag_blocked(&m.text) {
            self.log(ts, 0, format!("sygnał odrzucony filtrem tagów: {reason}"));
            self.jreject(
                b,
                m,
                "entry",
                RejectCode::TagFilter,
                format!("sygnał odrzucony filtrem tagów: {reason}"),
            );
            return;
        }
        if self.deferred_after_protection(b, m, &e) {
            return;
        }
        if let Gate::Blocked(r, code) = self.entry_gate(b, ts) {
            self.log(ts, 2, format!("wejścia zablokowane: {r}"));
            self.jreject(b, m, "entry", code, format!("wejścia zablokowane: {r}"));
            return;
        }
        let side_ok = match self.cfg.side_filter {
            SideFilter::Both => true,
            SideFilter::BuyOnly => e.side == Side::Buy,
            SideFilter::SellOnly => e.side == Side::Sell,
        };
        if !side_ok {
            self.log(ts, 0, format!("kierunek {:?} odfiltrowany", e.side));
            self.jreject(
                b,
                m,
                "entry",
                RejectCode::SideFilter,
                format!("kierunek {:?} odfiltrowany", e.side),
            );
            return;
        }
        if self.pulapy.blokuj_przeciwne_kierunki && self.obce.przeciwny_kierunek {
            self.log(
                ts,
                2,
                format!("kierunek {:?} przeciwny do pozycji innego formatu", e.side),
            );
            self.jreject(
                b,
                m,
                "entry",
                RejectCode::SideFilter,
                format!(
                    "pułap łańcucha: inny format stoi już po stronie przeciwnej do {:?}; \
                     otwarcie obu naraz to spread w obie strony i wyzerowana ekspozycja",
                    e.side
                ),
            );
            return;
        }
        self.rezim_miekki = false;
        let cena_wejscia = match e.side {
            Side::Buy => e.lo,
            Side::Sell => e.hi,
        };
        let rezim_przeszedl = match self.cfg.regime_cena {
            crate::settings::RegimeCena::Rynkowa => self.regime_ok(e.side, b.quote().mid()),
            crate::settings::RegimeCena::Wejscia => self.regime_ok(e.side, cena_wejscia),
            crate::settings::RegimeCena::Obie => {
                self.regime_ok(e.side, b.quote().mid()) && self.regime_ok(e.side, cena_wejscia)
            }
        };
        if self.cfg.regime_gdy_rozerwany == crate::settings::RegimeGdyRozerwany::Miekko
            && self.rezim_wyciszony()
        {
            self.rezim_miekki = true;
            self.log(
                ts,
                0,
                "reżim bez zdania (okno rozerwane) — wejście mniejszym rozmiarem",
            );
        }
        if !rezim_przeszedl {
            if !self.cfg.regime_soft {
                self.log(ts, 0, "sygnał niezgodny z reżimem rynku — pominięty");
                self.jreject(
                    b,
                    m,
                    "entry",
                    RejectCode::RegimeFilter,
                    "sygnał niezgodny z reżimem rynku",
                );
                return;
            }
            self.rezim_miekki = true;
            self.log(
                ts,
                0,
                "sygnał niezgodny z reżimem — wejście w trybie miękkim",
            );
        }
        if self.trend_adverse(e.side, ts) == Some(true)
            && self.cfg.trend_filter_mode == TrendFilterMode::Block
        {
            self.log(ts, 0, "sygnał pod trend wyższego rzędu — pominięty");
            self.jreject(
                b,
                m,
                "entry",
                RejectCode::TrendFilter,
                format!(
                    "sygnał {:?} pod trend: rynek zmienił się o ponad {:.2} % w oknie {:.0} h",
                    e.side, self.cfg.trend_filter_drop_pct, self.cfg.trend_filter_window_h
                ),
            );
            return;
        }

        let (zone_lo, zone_hi) = self.compute_zone(&e);
        let sl = self.compute_sl(&e, zone_lo, zone_hi, ts);
        let sig_w = (e.hi - e.lo).abs();

        if self.cfg.adaptive_params && self.journal.wants(EventLevel::Debug) {
            let d = self.adaptive_sl_min_dist(sig_w, ts);
            let deep = self.adaptive_deep_offset(sig_w, Self::dystans_do_sl(&e));
            let u = self.adaptive_units(self.units_base(e.is_limit).max(1), sig_w, ts);
            self.journal.push(
                Ev::new(
                    ts,
                    EventLevel::Debug,
                    EventCategory::Decision,
                    EventKind::Note,
                )
                .text(format!(
                    "parametry z sygnału: strefa {sig_w:.2} $ → sl_min_dist {d:.2} $ \
                         (preset {:.2}), deep {deep:.2} $ (preset {:.2}), szczebli {u} (preset {})",
                    self.cfg.sl_min_dist,
                    self.cfg.entry_deep_offset,
                    self.cfg.units_for(e.is_limit).max(1)
                ))
                .msg(m.msg_id)
                .signal(journal::signal_id(m.msg_id, "entry"))
                .put_f("zone_width", sig_w)
                .put_f("sl_min_dist", d)
                .put_f("deep_offset", deep)
                .put("units", u as u64)
                .put("atr", self.atr_proxy(ts).map(journal::r4))
                .build(),
            );
        }

        if self.cfg.signal_min_rr > 0.0 {
            let krawedz = e.side.worse_edge(zone_lo, zone_hi);
            if let (Some(slv), Some(tp1)) = (sl, e.tps.first().copied()) {
                let ryzyko = (krawedz - slv).abs();
                let nagroda = (tp1 - krawedz).abs();
                let rr = if ryzyko > 1e-9 {
                    nagroda / ryzyko
                } else {
                    f64::INFINITY
                };
                if rr < self.cfg.signal_min_rr {
                    self.log(
                        ts,
                        0,
                        format!("sygnał odrzucony — R:R {rr:.2} poniżej progu"),
                    );
                    self.jreject(
                        b,
                        m,
                        "entry",
                        RejectCode::SignalQualityTooLow,
                        format!(
                            "R:R {rr:.2} przy krawędzi {krawedz:.2} poniżej progu {:.2} \
                             (ryzyko {ryzyko:.2} $, nagroda {nagroda:.2} $)",
                            self.cfg.signal_min_rr
                        ),
                    );
                    return;
                }
            }
        }
        if (self.cfg.signal_min_zone_width > 0.0 && sig_w < self.cfg.signal_min_zone_width)
            || (self.cfg.signal_max_zone_width > 0.0 && sig_w > self.cfg.signal_max_zone_width)
        {
            self.log(
                ts,
                0,
                format!("sygnał odrzucony — strefa {sig_w:.2} $ poza pasmem"),
            );
            self.jreject(
                b,
                m,
                "entry",
                RejectCode::SignalQualityTooLow,
                format!(
                    "szerokość strefy {sig_w:.2} $ poza pasmem {:.2}–{:.2} $",
                    self.cfg.signal_min_zone_width, self.cfg.signal_max_zone_width
                ),
            );
            return;
        }

        let q = b.quote();
        if self.cfg.skip_if_sl_breached {
            if let Some(slv) = e.sl {
                let breached = match e.side {
                    Side::Buy => q.ask <= slv,
                    Side::Sell => q.bid >= slv,
                };
                if breached {
                    self.log(
                        ts,
                        2,
                        format!(
                            "sygnał pominięty — SL {slv:.2} przebity zanim powstało zlecenie (cena {:.2})",
                            q.mid()
                        ),
                    );
                    self.jreject(
                        b,
                        m,
                        "entry",
                        RejectCode::SlBreached,
                        format!(
                            "SL {slv:.2} przebity zanim powstało zlecenie (cena {:.2})",
                            q.mid()
                        ),
                    );
                    return;
                }
            }
        }
        let wejdzie_po_rynku = !e.is_limit && !self.cfg.auto_limit;
        if self.cfg.max_chase_beyond_zone > 0.0 && wejdzie_po_rynku {
            let worst = e.side.worse_edge(zone_lo, zone_hi);
            let beyond = match e.side {
                Side::Buy => q.ask - worst,
                Side::Sell => worst - q.bid,
            };
            if beyond > self.cfg.max_chase_beyond_zone {
                self.log(
                    ts,
                    2,
                    format!("sygnał pominięty — cena {beyond:.2} $ za strefą"),
                );
                self.jreject(
                    b,
                    m,
                    "entry",
                    RejectCode::ChaseTooFar,
                    format!(
                        "cena uciekła {beyond:.2} $ za strefę (limit {:.2} $)",
                        self.cfg.max_chase_beyond_zone
                    ),
                );
                return;
            }
        }

        {
            let szer = zone_hi - zone_lo;
            let mut powod: Option<String> = None;
            if self.cfg.sanity_zone_max > 0.0 && szer > self.cfg.sanity_zone_max {
                powod = Some(format!(
                    "strefa {szer:.2} $ ponad limit {:.2} $",
                    self.cfg.sanity_zone_max
                ));
            }
            if powod.is_none()
                && (self.cfg.sanity_tp_max > 0.0
                    || self.cfg.sanity_tp_rosnace
                    || self.cfg.sanity_tp_strona)
            {
                let baza = e.side.better_edge(zone_lo, zone_hi);
                let mut poprz = f64::NEG_INFINITY;
                for (i, tp) in e.tps.iter().enumerate() {
                    let od = (tp - baza) * e.side.sign();
                    if self.cfg.sanity_tp_strona && od <= 0.0 {
                        powod = Some(format!("TP{} = {tp:.2} po złej stronie strefy", i + 1));
                        break;
                    }
                    if self.cfg.sanity_tp_max > 0.0 && od.abs() > self.cfg.sanity_tp_max {
                        powod = Some(format!(
                            "TP{} oddalony o {:.2} $ ponad limit {:.2} $",
                            i + 1,
                            od.abs(),
                            self.cfg.sanity_tp_max
                        ));
                        break;
                    }
                    if self.cfg.sanity_tp_rosnace && od <= poprz {
                        powod = Some(format!(
                            "TP{} = {tp:.2} nie jest dalej niż poprzedni cel",
                            i + 1
                        ));
                        break;
                    }
                    poprz = od;
                }
            }
            if let Some(p) = powod {
                self.log(ts, 0, format!("sygnał pominięty — geometria: {p}"));
                self.jreject(b, m, "entry", RejectCode::SignalGeometryInsane, p);
                return;
            }
        }

        if self.cfg.entry_sl_dist_limit > 0.0 {
            if let Some(slv) = sl {
                let worst = e.side.worse_edge(zone_lo, zone_hi);
                if (worst - slv).abs() > self.cfg.entry_sl_dist_limit {
                    self.log(ts, 0, "wejście pominięte — zbyt daleko od SL");
                    self.jreject(
                        b,
                        m,
                        "entry",
                        RejectCode::SlDistanceTooBig,
                        format!(
                            "odległość strefa–SL {:.2} $ ponad limit {:.2} $",
                            (worst - slv).abs(),
                            self.cfg.entry_sl_dist_limit
                        ),
                    );
                    return;
                }
            }
        }

        if self.cfg.merge_same_side {
            let okno = (self.cfg.merge_window_min.max(0.0) * 60_000.0) as i64;
            let prog = self.cfg.merge_min_overlap;
            let kandydat = self
                .baskets
                .iter()
                .rev()
                .find(|x| {
                    x.alive()
                        && x.side == e.side
                        && x.source == m.source
                        && ts >= x.created_ts
                        && ts - x.created_ts <= okno
                        && zone_overlap(x.zone_lo, x.zone_hi, zone_lo, zone_hi) >= prog
                })
                .map(|x| (x.id, x.zone_lo, x.zone_hi));
            if let Some((id, old_lo, old_hi)) = kandydat {
                let pokrycie = zone_overlap(old_lo, old_hi, zone_lo, zone_hi);
                let wiek =
                    (ts - self.basket(id).map(|x| x.created_ts).unwrap_or(ts)) as f64 / 60_000.0;
                self.apply_entry_edit(b, id, &e, ts);
                self.msg_to_basket.insert((m.source.clone(), m.msg_id), id);
                self.remember_entry_source(m,id);
                self.basket_note(
                    id,
                    ts,
                    format!(
                        "sygnał dołączony do tego koszyka zamiast nowego \
                         (pokrycie stref {:.0} %, wiek koszyka {wiek:.0} min) — jeden spread zamiast dwóch",
                        pokrycie * 100.0
                    ),
                );
                if self.journal.wants(EventLevel::Info) {
                    let snap = self.jsnap(b);
                    self.journal.push(
                        Ev::new(ts, EventLevel::Info, EventCategory::Decision, EventKind::BasketUpdated)
                            .text(format!(
                                "koszyk B{id}: nowy sygnał {:?} scalony (pokrycie {:.0} %, wiek {wiek:.0} min)",
                                e.side,
                                pokrycie * 100.0
                            ))
                            .basket(id)
                            .msg(m.msg_id)
                            .signal(journal::signal_id(m.msg_id, "entry"))
                            .source(m.source_name.clone())
                            .reason(RejectCode::MergedIntoBasket)
                            .market(snap)
                            .put_f("overlap", pokrycie)
                            .put_f("age_min", wiek)
                            .put_f("zone_lo", zone_lo)
                            .put_f("zone_hi", zone_hi)
                            .build(),
                    );
                }
                return;
            }
        }

        if self.cfg.daily_signal_budget > 0 {
            let dzien = day_of(ts, self.cfg.session_offset());
            if dzien != self.budget_day {
                self.budget_day = dzien;
                self.opened_today = 0;
            }
            if self.opened_today >= self.cfg.daily_signal_budget {
                self.log(
                    ts,
                    0,
                    format!(
                        "sygnał pominięty — budżet dnia wyczerpany ({} koszyków)",
                        self.opened_today
                    ),
                );
                self.jreject(
                    b,
                    m,
                    "entry",
                    RejectCode::DailyBudgetSpent,
                    format!(
                        "budżet dnia wyczerpany: {} z {} koszyków",
                        self.opened_today, self.cfg.daily_signal_budget
                    ),
                );
                return;
            }
            self.opened_today += 1;
        }

        let id = self.next_basket_id;
        self.next_basket_id += 1;
        self.created_baskets_count += 1;
        let mut bk = new_basket(
            id,
            m,
            e.side,
            e.is_limit,
            e.lo,
            e.hi,
            zone_lo,
            zone_hi,
            sl,
            self.cele_z_runnerem_od(e.side, &e.tps, Some(b.quote().mid())),
            ts,
        );
        bk.tp_open = e.tp_open;
        bk.warstwy_offset = e.warstwy_offset;
        bk.is_stop = e.is_stop;
        // Retain the source even under the legacy editor: comparing an edit
        // with the currently trailed stop mistakes unchanged text for new risk.
        bk.entry_edit_state = Some(Box::new(EntryEditState {
                schema_version: 1,
                revision: 1,
                source: Some(e.clone()),
                applied_ts: ts,
                cancelled_by_source_ts: None,
                review: None,
            }));
        self.remember_pending_source(&bk);
        bk.events.push(BasketEvent {
            ts,
            text: format!(
                "koszyk B{id} · {:?}{} · strefa {:.2}–{:.2} · SL {} · cele {}",
                e.side,
                if e.is_limit { " LIMIT" } else { "" },
                zone_lo,
                zone_hi,
                sl.map(|x| format!("{x:.2}")).unwrap_or_else(|| "—".into()),
                e.tps
                    .iter()
                    .map(|t| format!("{t:.2}"))
                    .collect::<Vec<_>>()
                    .join(" / ")
            ),
        });
        self.baskets.push(bk);
        self.msg_to_basket.insert((m.source.clone(), m.msg_id), id);
        self.remember_entry_source(m,id);
        self.obs.na_sygnale(ts, e.side);

        if self.journal.wants(EventLevel::Info) {
            let snap = self.jsnap(b);
            self.journal.push(
                Ev::new(
                    ts,
                    EventLevel::Info,
                    EventCategory::Basket,
                    EventKind::BasketCreated,
                )
                .text(format!(
                    "koszyk B{id} {:?}{} strefa {zone_lo:.2}–{zone_hi:.2}",
                    e.side,
                    if e.is_limit { " LIMIT" } else { "" }
                ))
                .basket(id)
                .msg(m.msg_id)
                .signal(journal::signal_id(m.msg_id, "entry"))
                .source(m.source_name.clone())
                .market(snap)
                .put("side", format!("{:?}", e.side))
                .put("is_limit", e.is_limit)
                .put_f("zone_lo", zone_lo)
                .put_f("zone_hi", zone_hi)
                .put_f("signal_lo", e.lo)
                .put_f("signal_hi", e.hi)
                .put("sl", sl.map(journal::r4))
                .put(
                    "tps",
                    e.tps.iter().map(|t| journal::r4(*t)).collect::<Vec<f64>>(),
                )
                .build(),
            );
        }

        self.place_grid(b, id, ts);
    }

    fn cele_spojne(&self, side: Side, tps: &[Px]) -> bool {
        if tps.len() < 2 {
            return true; // jeden cel nie ma kierunku
        }
        let rosnace = tps.windows(2).all(|w| w[1] >= w[0]);
        let malejace = tps.windows(2).all(|w| w[1] <= w[0]);
        match side {
            Side::Buy => rosnace,
            Side::Sell => malejace,
        }
    }

    fn apply_entry_edit<B: Broker>(
        &mut self,
        b: &mut B,
        id: u32,
        e: &EntrySignal,
        ts: Ts,
    ) -> EntryEditOutcome {
        if self.pending_source_cancelled(id) {
            self.basket_note(id, ts, "EDYCJA pominięta: źródło anulowało ten sygnał".into());
            return EntryEditOutcome::Rejected;
        }

        if self.cfg.entry_edit_geometry_v2 {
            return self.apply_entry_edit_v2(b, id, e, ts);
        }
        if self.basket(id).and_then(|bk| bk.entry_edit_state.as_ref())
            .is_some_and(|state| state.review.is_some())
        {
            self.basket_note(id, ts, "EDYCJA wstrzymana: poprzednia rewizja wymaga uzgodnienia".into());
            return EntryEditOutcome::RequiresReview;
        }
        if self.basket(id).and_then(|bk| bk.entry_edit_state.as_ref())
            .and_then(|state| state.source.as_ref())
            .is_some_and(|source| entry_edit::same_source(source, e))
        {
            self.basket_note(id, ts, "EDYCJA bez zmiany źródłowego planu — postęp i zlecenia pozostają".into());
            return EntryEditOutcome::NoOp;
        }
        self.apply_entry_edit_legacy(b, id, e, ts);
        if self.basket(id).and_then(|bk| bk.entry_edit_state.as_ref())
            .is_some_and(|state| state.review.is_some()) {
            EntryEditOutcome::RequiresReview
        } else { EntryEditOutcome::LegacyHandled }
    }

    fn apply_entry_edit_legacy<B: Broker>(&mut self, b: &mut B, id: u32, e: &EntrySignal, ts: Ts) {
        if self.basket_exit_pending(id) {
            self.basket_note(
                id,
                ts,
                "EDYCJA pominięta: trwa potwierdzanie zamknięcia koszyka".into(),
            );
            return;
        }
        let (zone_lo, zone_hi) = self.compute_zone(e);
        let sl = self.compute_sl(e, zone_lo, zone_hi, ts);
        let (old_lo, old_hi, old_sl, old_tps, mozna_przestawic) = match self.basket(id) {
            Some(bk) => (
                bk.zone_lo,
                bk.zone_hi,
                bk.sl,
                bk.tps.clone(),
                bk.state == BasketState::Armed && bk.tickets.is_empty(),
            ),
            None => return,
        };

        if self.basket(id).map(|bk| bk.side) != Some(e.side) {
            self.basket_note(
                id,
                ts,
                format!(
                    "EDYCJA ODRZUCONA — sygnal {:?} nie pasuje do koszyka {:?};                      przeciwny kierunek to nowy setup, nie poprawka",
                    e.side,
                    self.basket(id).map(|bk| bk.side)
                ),
            );
            return;
        }

        if !self.cele_spojne(e.side, &e.tps) {
            self.basket_note(
                id,
                ts,
                format!(
                    "CELE ODRZUCONE przy edycji — {:?} niespójne z {:?}",
                    e.tps, e.side
                ),
            );
            return;
        }
        if mozna_przestawic && (b.close_receipts_pending()
            || b.receipt_barrier() != crate::broker::ReceiptBarrier::Clear) {
            self.edit_review(id, e, ts, "LegacyEditReceiptBarrier");
            return;
        }
        let before_edit = self.basket(id).cloned().expect("legacy edit basket was validated");
        let before_session = b.execution_session();
        let position_signature = |broker: &B| -> Vec<(Ticket, f64, Px)> {
            broker.positions().iter().filter(|p| p.basket == Some(id))
                .map(|p| (p.ticket, p.volume, p.open_price)).collect()
        };
        let before_positions = position_signature(b);
        let cele_nowe = self.cele_z_runnerem_od(e.side, &e.tps, Some(b.quote().mid()));
        let cele_te_same = old_tps == cele_nowe;
        if let Some(bk) = self.basket_mut(id) {
            let plan_inny = bk.tps != cele_nowe
                || bk.zone_lo != zone_lo
                || bk.zone_hi != zone_hi
                || bk.sl != sl;
            bk.tps = cele_nowe.clone();
            bk.sl = sl;
            bk.zone_lo = zone_lo;
            bk.zone_hi = zone_hi;
            bk.entry_lo = e.lo;
            bk.entry_hi = e.hi;
            // This is a source snapshot, not a certificate of broker ACK.
            // Any uncertain cancellation remains governed by the broker gate.
            let revision = bk.entry_edit_state.as_ref().map_or(1, |s| s.revision.saturating_add(1));
            bk.entry_edit_state = Some(Box::new(EntryEditState {
                schema_version: 1, revision, source: Some(e.clone()),
                applied_ts: ts, cancelled_by_source_ts: None, review: None,
            }));
            if plan_inny {
                bk.zeruj_postep();
            }
        }
        self.basket_note(
            id,
            ts,
            format!("EDYCJA sygnału: strefa {old_lo:.2}–{old_hi:.2} → {zone_lo:.2}–{zone_hi:.2}"),
        );

        if let Some(v) = sl {
            self.set_basket_sl(b, id, v, ts);
        }
        let bez_zmian = (old_lo - zone_lo).abs() < 1e-9
            && (old_hi - zone_hi).abs() < 1e-9
            && old_sl.map(|x| (x * 1e9) as i64) == sl.map(|x| (x * 1e9) as i64)
            && cele_te_same;
        if mozna_przestawic && !bez_zmian {
            let pending_before = b.pendings().iter().filter(|o| o.basket == Some(id)).count();
            let cancelled = self.cancel_pendings(b, id);
            let remaining: Vec<_> = b.pendings().iter().filter(|o| o.basket == Some(id))
                .map(|o| o.ticket).collect();
            let unconfirmed = cancelled != pending_before || !remaining.is_empty()
                || before_positions != position_signature(b)
                || before_session != b.execution_session()
                || b.close_receipts_pending()
                || b.receipt_barrier() != crate::broker::ReceiptBarrier::Clear;
            if unconfirmed {
                // Cancellation/modify may have partly executed. Preserve the last
                // source revision and actual broker exposure; do not manufacture
                // a replacement or pretend the changed plan was fully committed.
                let positions: Vec<_> = b.positions().iter().filter(|p| p.basket == Some(id))
                    .cloned().collect();
                if let Some(bk) = self.basket_mut(id) {
                    *bk = before_edit;
                    bk.pendings = remaining;
                    bk.tickets = positions.iter().map(|p| p.ticket).collect();
                    if !positions.is_empty() {
                        bk.had_positions = true;
                        bk.state = BasketState::Working;
                        for position in &positions {
                            if let Some(level) = bk.levels.iter_mut().find(|g| g.level == position.level) {
                                level.filled = true;
                                level.fill_ts = position.open_ts;
                            }
                        }
                    }
                }
                self.edit_review(id, e, ts, "LegacyCancelOrFillUnconfirmed");
                return;
            }
            self.place_grid(b, id, ts);
        } else if mozna_przestawic {
            self.basket_note(
                id,
                ts,
                "edycja bez różnicy — siatka zostaje na rynku".into(),
            );
        }
    }

    fn compute_zone(&self, e: &EntrySignal) -> (Px, Px) {
        let (mut lo, mut hi) = (e.lo, e.hi);
        let deep = self.adaptive_deep_offset((e.hi - e.lo).abs(), Self::dystans_do_sl(e));
        match self.cfg.zone_offset_mode {
            ZoneOffsetMode::None => {}
            ZoneOffsetMode::Price => {
                hi += self.cfg.entry_hi_offset;
                lo += self.cfg.entry_lo_offset;
            }
            ZoneOffsetMode::Directional => {
                match e.side {
                    Side::Buy => {
                        lo -= deep;
                        hi += self.cfg.entry_tol_offset;
                    }
                    Side::Sell => {
                        hi += deep;
                        lo -= self.cfg.entry_tol_offset;
                    }
                }
            }
        }
        (lo.min(hi), lo.max(hi))
    }

    fn compute_sl(&self, e: &EntrySignal, lo: Px, hi: Px, ts: Ts) -> Option<Px> {
        let mut sl = e.sl?;
        let min_dist = self.adaptive_sl_min_dist((e.hi - e.lo).abs(), ts);
        if min_dist > 0.0 {
            let mid = (lo + hi) * 0.5;
            let want = match e.side {
                Side::Buy => mid - min_dist,
                Side::Sell => mid + min_dist,
            };
            sl = match e.side {
                Side::Buy => sl.min(want),
                Side::Sell => sl.max(want),
            };
        }
        if self.cfg.sl_max_dist > 0.0 {
            let mid = (lo + hi) * 0.5;
            let cap = match e.side {
                Side::Buy => mid - self.cfg.sl_max_dist,
                Side::Sell => mid + self.cfg.sl_max_dist,
            };
            sl = match e.side {
                Side::Buy => sl.max(cap),
                Side::Sell => sl.min(cap),
            };
        }
        Some(sl)
    }


    fn plan_grid(
        &self,
        id: u32,
        ts: Ts,
        wolne_portfela: Option<f64>,
        sufit_ea: Option<crate::ea::SufitEa>,
    ) -> (Vec<GridLevel>, bool, u32) {
        match self.basket(id) {
            Some(bk) => self.plan_grid_for(bk, ts, wolne_portfela, sufit_ea),
            None => (Vec::new(), false, 0),
        }
    }

    fn plan_grid_for(
        &self,
        basket: &Basket,
        ts: Ts,
        wolne_portfela: Option<f64>,
        sufit_ea: Option<crate::ea::SufitEa>,
    ) -> (Vec<GridLevel>, bool, u32) {
        let (side, is_limit, lo, hi, sl, tps, tp_open, sig_lo, sig_hi, warstwy_txt) =
            match Some(basket) {
                Some(bk) => (
                    bk.side,
                    bk.is_limit,
                    bk.zone_lo,
                    bk.zone_hi,
                    bk.sl,
                    bk.tps.clone(),
                    bk.tp_open,
                    bk.entry_lo,
                    bk.entry_hi,
                    bk.warstwy_offset,
                ),
                None => return (Vec::new(), false, 0),
            };
        let sig_w = (sig_hi - sig_lo).abs();
        let mut units = self.adaptive_units(self.units_base(is_limit).max(1), sig_w, ts);
        if self.cfg.trend_filter_mode == TrendFilterMode::Shrink
            && self.cfg.trend_filter_shrink > 0.0
            && self.trend_adverse(side, ts) == Some(true)
        {
            units = ((units as f64 * self.cfg.trend_filter_shrink).round() as u32).max(1);
        }
        let units = units;
        let uklad = self.cfg.uklad_drabinki();
        let trzymaj_uklad = !self.cfg.entry_uklad_kotwica.eq_ignore_ascii_case("ocalaly");
        let mut uklad_sztuk: Vec<u32> = Vec::new();

        let step = self.cfg.grid_step();
        let mut prices: Vec<Px> = Vec::new();
        if self.cfg.grid_anchor_absolute && step > 0.0 {
            let mut v = (lo / step - 1e-9).ceil() * step;
            while v <= hi + 1e-9 {
                prices.push((v * 100.0).round() / 100.0);
                v += step;
            }
            prices.sort_by(|a, b| match side {
                Side::Buy => a.partial_cmp(b).unwrap(),
                Side::Sell => b.partial_cmp(a).unwrap(),
            });
            if prices.is_empty() {
                prices.push(side.better_edge(lo, hi));
            }
        } else if step > 0.0 && hi - lo > step * 0.5 {
            let n = (((hi - lo) / step).floor() as usize).max(1);
            for i in 0..=n {
                let p = match side {
                    Side::Buy => lo + i as f64 * step,
                    Side::Sell => hi - i as f64 * step,
                };
                if p >= lo - 1e-9 && p <= hi + 1e-9 {
                    prices.push(p);
                }
            }
        } else if units == 1 {
            prices.push(if self.cfg.entry_jeden_na_glebokiej {
                side.better_edge(lo, hi)
            } else {
                side.worse_edge(lo, hi)
            });
        } else if !uklad.is_empty() {
            let n = uklad.len();
            for (i, &szt) in uklad.iter().enumerate().rev() {
                if szt == 0 {
                    continue;
                }
                let f = (n - 1 - i) as f64 / (n - 1).max(1) as f64;
                prices.push(match side {
                    Side::Buy => lo + f * (hi - lo),
                    Side::Sell => hi - f * (hi - lo),
                });
                uklad_sztuk.push(szt);
            }
        } else {
            for i in 0..units {
                let f = i as f64 / (units - 1).max(1) as f64;
                let k = self.cfg.entry_depth_curve;
                let f = if k > 0.0 && (k - 1.0).abs() > 1e-12 {
                    f.powf(k)
                } else {
                    f
                };
                prices.push(match side {
                    Side::Buy => lo + f * (hi - lo),
                    Side::Sell => hi - f * (hi - lo),
                });
            }
        }
        if prices.is_empty() {
            prices.push(side.better_edge(lo, hi));
        }

        let warstwy = if self.cfg.entry_warstwy_z_tekstu {
            warstwy_txt.unwrap_or(self.cfg.entry_warstwy_offset)
        } else {
            self.cfg.entry_warstwy_offset
        };
        if warstwy != 0.0 && prices.len() > 1 {
            let ostatni = prices.len() - 1;
            let o = side.sign() * warstwy;
            for (i, p) in prices.iter_mut().enumerate() {
                if i != ostatni {
                    *p += o;
                }
            }
        }

        let plan_n = prices.len();
        let mut idx_plan: Vec<usize> = (0..plan_n).collect();

        if self.cfg.zakaz_ponizej_krawedzi && sig_w > 1e-9 {
            let krawedz = Self::dalsza_krawedz(side, sig_lo, sig_hi);
            let zostaw: Vec<bool> = prices
                .iter()
                .map(|p| !self.za_dalsza_krawedzia(side, *p, sig_lo, sig_hi))
                .collect();
            Self::przesiej_rownolegle(
                &mut prices,
                &mut uklad_sztuk,
                &mut idx_plan,
                &zostaw,
                trzymaj_uklad,
            );
            if prices.is_empty() {
                prices.push(krawedz);
                uklad_sztuk.clear();
                idx_plan.clear();
                idx_plan.push(0);
            }
        }

        if self.cfg.drop_unplaceable_levels {
            if let Some(slv) = sl {
                let stops = self.cfg.stops_level;
                let zostaw: Vec<bool> = prices
                    .iter()
                    .map(|p| match side {
                        Side::Buy => slv <= *p - stops,
                        Side::Sell => slv >= *p + stops,
                    })
                    .collect();
                Self::przesiej_rownolegle(
                    &mut prices,
                    &mut uklad_sztuk,
                    &mut idx_plan,
                    &zostaw,
                    trzymaj_uklad,
                );
                if prices.is_empty() {
                    return (Vec::new(), true, 0);
                }
            }
        }

        let total = prices.len();
        let lot = self.lot_size(self.podstawa_lota());
        let krzywa_planowa = self
            .cfg
            .entry_krzywa_kotwica
            .eq_ignore_ascii_case("planowany");
        let tp_planowy = self
            .cfg
            .tp_drabinka_kotwica
            .eq_ignore_ascii_case("planowany");
        let (mult, mapuj_mult) = if self.cfg.entry_weights_from_rr {
            (
                self.cfg.rr_multipliers(&prices, sl, tps.first().copied()),
                false,
            )
        } else if krzywa_planowa && plan_n > 0 {
            (self.cfg.depth_multipliers(plan_n), true)
        } else {
            (self.cfg.depth_multipliers(total), false)
        };
        let mut out: Vec<GridLevel> = Vec::with_capacity(total + 4);
        for (i, &price) in prices.iter().enumerate() {
            let sl_szczebla = if self.cfg.sl_wlasny_na_pozycje > 0.0 {
                let wlasny = price - side.sign() * self.cfg.sl_wlasny_na_pozycje;
                match sl {
                    Some(k) => Some(match side {
                        Side::Buy => wlasny.max(k),
                        Side::Sell => wlasny.min(k),
                    }),
                    None => Some(wlasny),
                }
            } else {
                sl
            };
            out.push(GridLevel {
                price,
                base_units: if uklad_sztuk.is_empty() {
                    self.units_for_level(
                        price,
                        sl_szczebla,
                        tps.first().copied(),
                        units,
                        total,
                        is_limit,
                    )
                } else {
                    uklad_sztuk.get(i).copied().unwrap_or(1).max(1)
                },
                volume: self.wolumen_zlecenia(
                    lot * mult[Self::miejsce(mapuj_mult, &idx_plan, i, mult.len())],
                ),
                sl: sl_szczebla,
                tp: self.target_for_ex(
                    &tps,
                    side,
                    if tp_planowy {
                        idx_plan.get(i).copied().unwrap_or(i)
                    } else {
                        i
                    },
                    if tp_planowy {
                        plan_n.max(1)
                    } else {
                        total.max(1)
                    },
                    tp_open,
                ),
                level: i as i32,
                is_toucher: false,
                fill_ts: 0,
                fill_px: 0.0,
                cancelled: false,
                filled: false,
            });
        }

        for (k, (off, tunits, tp_idx)) in self.cfg.parse_toucher_bands().into_iter().enumerate() {
            let base = side.worse_edge(lo, hi);
            let cena_touchera = match side {
                Side::Buy => base - off,
                Side::Sell => base + off,
            };
            if self.za_dalsza_krawedzia(side, cena_touchera, sig_lo, sig_hi) {
                continue;
            }
            out.push(GridLevel {
                price: cena_touchera,
                base_units: tunits,
                volume: lot,
                sl,
                tp: tps.get(tp_idx).copied().or_else(|| tps.last().copied()),
                level: 1000 + k as i32,
                is_toucher: true,
                fill_ts: 0,
                fill_px: 0.0,
                cancelled: false,
                filled: false,
            });
        }

        if self.cfg.entry_allowance_usd > 0.0 && self.cfg.entry_allowance_units > 0 {
            let poza = side.worse_edge(lo, hi) + side.sign() * self.cfg.entry_allowance_usd;
            let mieści = match sl {
                Some(slv) => match side {
                    Side::Buy => slv <= poza - self.cfg.stops_level,
                    Side::Sell => slv >= poza + self.cfg.stops_level,
                },
                None => true,
            };
            if !self.cfg.drop_unplaceable_levels || mieści {
                out.push(GridLevel {
                    price: (poza * 100.0).round() / 100.0,
                    base_units: self.cfg.entry_allowance_units,
                    volume: self.wolumen_zlecenia(lot),
                    sl,
                    tp: self.target_for_ex(&tps, side, total.max(1) - 1, total.max(1), tp_open),
                    level: 2000,
                    is_toucher: false,
                    fill_ts: 0,
                    fill_px: 0.0,
                    cancelled: false,
                    filled: false,
                });
            }
        }

        if self.wejscie_rynkowe(is_limit)
            && self.cfg.market_entry_mode == MarketEntryMode::Single
            && !out.is_empty()
        {
            let lacznie: f64 = out
                .iter()
                .map(|g| g.volume * g.base_units.max(1) as f64)
                .sum();
            let jeden = GridLevel {
                price: side.worse_edge(lo, hi),
                base_units: 1,
                volume: self.wolumen_zlecenia(lacznie),
                sl,
                tp: self.target_for_ex(&tps, side, 0, 1, tp_open),
                level: 0,
                is_toucher: false,
                fill_ts: 0,
                fill_px: 0.0,
                cancelled: false,
                filled: false,
            };
            out.clear();
            out.push(jeden);
        }

        if self.cfg.market_entry_units > 0
            && self.cfg.market_hybrid_now_units == 0
            && self.sygnal_rynkowy(is_limit)
        {
            let mut budzet = self.cfg.market_entry_units;
            out.retain_mut(|g| {
                if budzet == 0 {
                    return false;
                }
                let u = g.base_units.max(1);
                if u <= budzet {
                    budzet -= u;
                } else {
                    g.base_units = budzet;
                    budzet = 0;
                }
                true
            });
        }

        let mut sciete = 0u32;
        if let Some(s) = sufit_ea {
            if !s.jest_obojetny() {
                let suma: u32 = out.iter().map(|g| g.base_units.max(1)).sum();
                let cel = s.docelowe_jednostki(suma);
                if cel < suma {
                    sciete = suma - cel;
                    let mut budzet = cel;
                    out.retain_mut(|g| {
                        if budzet == 0 {
                            return false;
                        }
                        let u = g.base_units.max(1);
                        if u <= budzet {
                            budzet -= u;
                        } else {
                            g.base_units = budzet;
                            budzet = 0;
                        }
                        true
                    });
                }
            }
        }

        self.cap_basket_risk(&mut out, sl, wolne_portfela);
        (out, false, sciete)
    }

    fn cap_basket_risk(
        &self,
        levels: &mut Vec<GridLevel>,
        sl: Option<Px>,
        wolne_portfela: Option<f64>,
    ) {
        let pct = self.risk_per_basket_pct_eff();
        let mult = self.dlawik_mult();
        if (pct <= 0.0 && wolne_portfela.is_none()) || levels.is_empty() {
            return;
        }
        let Some(slv) = sl else { return };
        let mut cap = if pct > 0.0 {
            self.stats.equity.max(0.0) * pct * mult / 100.0
        } else {
            f64::MAX
        };
        if let Some(w) = wolne_portfela {
            cap = cap.min(w);
        }
        if cap <= 0.0 {
            levels.clear();
            return;
        }

        let risk_of = |g: &GridLevel| {
            (g.price - slv).abs() * XAU_CONTRACT * g.volume * g.base_units.max(1) as f64
        };
        let total = |ls: &Vec<GridLevel>| ls.iter().map(risk_of).sum::<f64>();
        let growth = crate::lot_growth::enabled(&self.cfg);
        let exceeds = |value| if growth {crate::lot_growth::plan_risk_exceeds(value,cap)} else {value>cap};

        let mut now = total(levels);
        if if growth {!exceeds(now)} else {now<=cap} {
            return;
        }

        let factor = cap / now;
        for g in levels.iter_mut() {
            g.volume = if growth {
                // Preserve G7's minimum-leg selection. The complete plan is
                // pruned/rechecked below; the final broker cap never promotes.
                (g.volume * factor).max(self.cfg.lot_min)
            } else if self.cfg.order_volume_contract_v2 {
                g.volume * factor
            } else {
                round_lot((g.volume * factor).max(self.cfg.lot_min))
            };
        }
        now = total(levels);

        while exceeds(now) && levels.len() > 1 {
            let idx = levels
                .iter()
                .position(|g| g.is_toucher)
                .unwrap_or(levels.len() - 1);
            levels.remove(idx);
            now = total(levels);
        }

        if exceeds(now) {
            levels.clear();
        }
    }

    fn dlawik_mult(&self) -> f64 {
        if self.cfg.dd_soft_pct <= 0.0 && self.cfg.dd_hard_pct <= 0.0 {
            return 1.0;
        }
        let base = match self.cfg.dd_guard_scope {
            DdGuardScope::Daily => self.stats.day_peak_equity,
            DdGuardScope::Lifetime | DdGuardScope::LifetimePeakDailyReset => self.stats.peak_equity,
        };
        if base <= 0.0 {
            return 1.0;
        }
        let dd_pct = (base - self.stats.equity) / base * 100.0;
        let mut m = 1.0;
        if self.cfg.dd_soft_pct > 0.0 && dd_pct >= self.cfg.dd_soft_pct {
            m = self.cfg.dd_soft_mult;
        }
        if self.cfg.dd_hard_pct > 0.0 && dd_pct >= self.cfg.dd_hard_pct {
            m = m.min(self.cfg.dd_hard_mult);
        }
        if m.is_finite() {
            m.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    fn wolny_budzet_portfela<B: Broker>(&self, b: &B) -> Option<f64> {
        match crate::profit_budget::available(&self.cfg,(&self.stats).into(),b,None) {
            Ok(Some(v))=>return Some(v.remaining),
            Err(_)=>return Some(0.0),
            Ok(None)=>{}
        }
        let pct = self.cfg.max_portfolio_risk_pct;
        if pct <= 0.0 {
            return None;
        }
        let cap = self.stats.equity.max(0.0) * pct / 100.0;
        let poz: f64 = b
            .positions()
            .iter()
            .chain(b.ukryte_pozycje())
            .filter_map(|p| {
                p.sl.or(p.vsl)
                    .map(|s| (p.open_price - s).abs() * XAU_CONTRACT * p.volume)
            })
            .sum();
        let pend: f64 = b
            .pendings()
            .iter()
            .chain(b.ukryte_zlecenia())
            .filter_map(|p| p.sl.map(|s| (p.price - s).abs() * XAU_CONTRACT * p.volume))
            .sum();
        Some((cap - poz - pend).max(0.0))
    }

    #[inline]
    fn wejscie_rynkowe(&self, is_limit: bool) -> bool {
        !is_limit && !self.cfg.auto_limit
    }

    #[inline]
    fn sygnal_rynkowy(&self, is_limit: bool) -> bool {
        !is_limit
    }

    #[inline]
    fn przesiej_rownolegle(
        prices: &mut Vec<Px>,
        sztuki: &mut Vec<u32>,
        idx_plan: &mut Vec<usize>,
        zostaw: &[bool],
        trzymaj_przy_cenie: bool,
    ) {
        {
            let ile = zostaw.iter().filter(|z| !**z).count();
            if ile > 0 {
                ODSIANE_SZCZEBLE.fetch_add(ile as u64, std::sync::atomic::Ordering::Relaxed);
                KOSZYKI_Z_ODSIEWEM.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        {
            let mut k = 0;
            idx_plan.retain(|_| {
                let z = zostaw.get(k).copied().unwrap_or(true);
                k += 1;
                z
            });
        }
        let mut i = 0;
        prices.retain(|_| {
            let z = zostaw.get(i).copied().unwrap_or(true);
            i += 1;
            z
        });
        if sztuki.is_empty() || !trzymaj_przy_cenie {
            return;
        }
        let mut j = 0;
        sztuki.retain(|_| {
            let z = zostaw.get(j).copied().unwrap_or(true);
            j += 1;
            z
        });
    }

    fn miejsce(mapuj: bool, idx_plan: &[usize], i: usize, dl: usize) -> usize {
        let m = if mapuj {
            idx_plan.get(i).copied().unwrap_or(i)
        } else {
            i
        };
        m.min(dl.saturating_sub(1))
    }

    fn dalsza_krawedz(side: Side, sig_lo: Px, sig_hi: Px) -> Px {
        side.better_edge(sig_lo.min(sig_hi), sig_lo.max(sig_hi))
    }

    #[inline]
    fn za_dalsza_krawedzia(&self, side: Side, cena: Px, sig_lo: Px, sig_hi: Px) -> bool {
        if !self.cfg.zakaz_ponizej_krawedzi {
            return false;
        }
        if (sig_hi - sig_lo).abs() < 1e-9 {
            return false;
        }
        let k = Self::dalsza_krawedz(side, sig_lo, sig_hi);
        match side {
            Side::Buy => cena < k - 1e-9,
            Side::Sell => cena > k + 1e-9,
        }
    }

    fn market_risk_cap<B: Broker>(&self, id: u32, b: &B) -> Option<f64> {
        let pct = self.risk_per_basket_pct_eff();
        if pct <= 0.0 {
            return None;
        }
        let cap = self.stats.equity.max(0.0) * pct / 100.0;
        let zajete: f64 = b
            .positions()
            .iter()
            .filter(|p| p.basket == Some(id))
            .filter_map(|p| {
                p.sl.or(p.vsl).map(|s| {
                    let adwersja = (p.open_price - s) * p.side.sign();
                    if adwersja > 0.0 {
                        adwersja * XAU_CONTRACT * p.volume
                    } else {
                        0.0
                    }
                })
            })
            .sum();
        Some((cap - zajete).max(0.0))
    }

    fn market_risk_scale(
        &self,
        lots_rynkowe: f64,
        px: Px,
        sl: Option<Px>,
        cap: Option<f64>,
    ) -> f64 {
        let Some(cap) = cap else { return 1.0 };
        if lots_rynkowe <= 0.0 || cap <= 0.0 {
            return 1.0;
        }
        let Some(slv) = sl else { return 1.0 };
        let ryzyko = (px - slv).abs() * XAU_CONTRACT * lots_rynkowe;
        if ryzyko <= cap {
            return 1.0;
        }
        cap / ryzyko
    }

    fn place_grid<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts) {
        if self.basket_exit_pending(id) {
            return;
        }
        let wolne = self.wolny_budzet_portfela(b);
        let sufit_ea = self.ea_sufit_jednostek(b);

        if sufit_ea.is_some_and(|s| s.jednostki_max == 0) {
            self.ea_a.odmowy_margines += 1;
            *self.odrzuty.entry("EaMargines".to_string()).or_insert(0) += 1;
            if let Some(bk) = self.basket_mut(id) {
                bk.state = BasketState::Done;
            }
            let opis = format!(
                "odrzucony — wolny margines nie starcza na ani jedną nogę \
                 (A1 = {:.1} % FM)",
                self.cfg.ea_lot_z_wolnego_marginesu
            );
            self.basket_note(id, ts, format!("koszyk {opis}"));
            if self.journal.wants(EventLevel::Warn) {
                let snap = self.jsnap(b);
                self.journal.push(
                    Ev::new(
                        ts,
                        EventLevel::Warn,
                        EventCategory::Decision,
                        EventKind::SignalRejected,
                    )
                    .text(format!("koszyk B{id} {opis}"))
                    .basket(id)
                    .reason(RejectCode::RiskBudgetExhausted)
                    .market(snap)
                    .build(),
                );
            }
            return;
        }

        let (plan, pusty_przez_stops, sciete_ea) = self.plan_grid(id, ts, wolne, sufit_ea);
        if sciete_ea > 0 {
            self.ea_a.przyciete_koszyki += 1;
            self.ea_a.sciete_jednostki += sciete_ea as u64;
        }
        if plan.is_empty() {
            let profit_block=match crate::profit_budget::available(&self.cfg,(&self.stats).into(),b,None) {
                Err(reason)=>Some(reason), Ok(Some(_))=>Some(crate::profit_budget::BudgetError::Exhausted),
                Ok(None)=>None,
            };
            let (licznik, kod, opis) = if pusty_przez_stops {
                (
                    "StopsLevel".to_string(),
                    RejectCode::InvalidStops,
                    format!(
                        "odrzucony — wszystkie szczeble bliżej SL niż stops_level {:.2}",
                        self.cfg.stops_level
                    ),
                )
            } else if let Some(reason)=profit_block {
                (format!("ProfitBudget::{reason:?}"),RejectCode::RiskBudgetExhausted,
                    format!("RISK BUDGET: {reason:?}; no legal grid fits the current reserve"))
            } else {
                (
                    "BudzetRyzyka".to_string(),
                    RejectCode::RiskBudgetExhausted,
                    format!(
                        "odrzucony — nie mieści się w limicie ryzyka {:.2}% kapitału",
                        self.risk_per_basket_pct_eff()
                    ),
                )
            };
            *self.odrzuty.entry(licznik.to_string()).or_insert(0) += 1;
            if let Some(bk) = self.basket_mut(id) {
                bk.state = BasketState::Done;
            }
            self.basket_note(id, ts, format!("koszyk {opis}"));
            if self.journal.wants(EventLevel::Warn) {
                let snap = self.jsnap(b);
                self.journal.push(
                    Ev::new(
                        ts,
                        EventLevel::Warn,
                        EventCategory::Decision,
                        EventKind::SignalRejected,
                    )
                    .text(format!("koszyk B{id} {opis}"))
                    .basket(id)
                    .reason(kod)
                    .market(snap)
                    .build(),
                );
            }
            return;
        }
        let planned_risk = self.plan_risk(&plan);
        if let Some(bk) = self.basket_mut(id) {
            bk.levels = plan;
            if bk.risk_initial_usd <= 0.0 {
                bk.risk_initial_usd = planned_risk;
            }
        }
        let placed = self.sync_grid(b, id, ts, false);
        let lot = self.lot_size(self.podstawa_lota());
        self.basket_note(
            id,
            ts,
            format!("rozstawiono {placed} zleceń · lot bazowy {lot:.2} · ryzyko koszyka {planned_risk:.2} $"),
        );
        if self.journal.wants(EventLevel::Info) {
            let snap = self.jsnap(b);
            let msg_id = self.basket(id).map(|x| x.msg_id);
            let mut ev = Ev::new(ts, EventLevel::Info, EventCategory::Order, EventKind::OrderPlaced)
                .text(format!(
                    "rozstawiono {placed} zleceń koszyka B{id} · lot bazowy {lot:.2} · ryzyko {planned_risk:.2} $"
                ))
                .basket(id)
                .market(snap)
                .put("placed", placed as u64)
                .put_f("lot", lot)
                .put_f("planned_risk", planned_risk);
            if let Some(mid) = msg_id {
                ev = ev.msg(mid).signal(journal::signal_id(mid, "entry"));
            }
            self.journal.push(ev.build());
        }
    }

    fn plan_ma_ten_sam_ksztalt(&self, id: u32, plan: &[GridLevel]) -> bool {
        let Some(bk) = self.basket(id) else {
            return false;
        };
        if bk.levels.is_empty() {
            return true;
        }
        let wspolne: Vec<i32> = plan
            .iter()
            .filter(|g| !g.is_toucher)
            .map(|g| g.level)
            .filter(|l| bk.levels.iter().any(|g| g.level == *l))
            .collect();
        if wspolne.len() < 2 {
            return true;
        }
        let rozpietosc = |ls: &[GridLevel]| -> f64 {
            let mut lo = f64::MAX;
            let mut hi: f64 = 0.0;
            for g in ls.iter().filter(|g| wspolne.contains(&g.level)) {
                if g.volume > 0.0 {
                    lo = lo.min(g.volume);
                    hi = hi.max(g.volume);
                }
            }
            if lo == f64::MAX || lo <= 0.0 {
                1.0
            } else {
                hi / lo
            }
        };
        let r_stary = rozpietosc(&bk.levels);
        let r_nowy = rozpietosc(plan);
        !(r_stary > 1.05 && r_nowy <= 1.001)
    }

    fn plan_risk(&self, levels: &[GridLevel]) -> f64 {
        levels
            .iter()
            .filter_map(|g| {
                g.sl.map(|s| {
                    (g.price - s).abs() * XAU_CONTRACT * g.volume * g.base_units.max(1) as f64
                })
            })
            .sum()
    }

    #[track_caller]
    fn sync_grid<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts, only_existing: bool) -> usize {
        if self.entry_edit_blocks(Some(id)) {
            return 0;
        }
        if self.basket_exit_pending(id) || b.close_receipts_pending() {
            return 0;
        }
        let (
            side,
            is_limit,
            sl_koszyka,
            mut levels,
            koszyk_stop,
            sig_lo,
            sig_hi,
            had_positions,
            tps,
        ) = match self.basket(id) {
            Some(bk) => (
                bk.side,
                bk.is_limit,
                bk.sl,
                bk.levels.clone(),
                bk.is_stop,
                bk.entry_lo,
                bk.entry_hi,
                bk.had_positions,
                bk.tps.clone(),
            ),
            None => return 0,
        };
        if self.cfg.pending_relot_reconcile_target
            && !self.revalidate_relot_sync_levels(b, id, ts, &mut levels)
        {
            return 0;
        }
        if cien::czynny() {
            cien::z(
                cakt::A_PLAN_PROBY,
                id as u64,
                czr::L_SYNC_GRID,
                std::panic::Location::caller().line(),
            );
        }
        let zlecenie_stop = koszyk_stop && self.cfg.honor_stop_orders;
        if levels.is_empty() {
            return 0;
        }
        let lot = self.lot_size(self.podstawa_lota());
        let q = b.quote();
        let mut placed = 0usize;
        let mut removed = 0usize;

        let f = self.vol_factor(ts);
        let core = levels.iter().filter(|g| !g.is_toucher).count();
        let allowed = if f < 1.0 {
            ((core as f64 * f).round() as usize).max(1)
        } else {
            core
        };
        let mut core_seen = 0usize;

        struct Zadanie {
            gl: GridLevel,
            want: usize,
            have: usize,
            filled: bool,
            live_pend: Vec<Ticket>,
            market_now: usize,
        }
        let mut zadania: Vec<Zadanie> = Vec::with_capacity(levels.len());

        for gl in &levels {
            let live_pend: Vec<Ticket> = b
                .pendings()
                .iter()
                .filter(|o| o.basket == Some(id) && o.level == gl.level && !o.is_topup)
                .map(|o| o.ticket)
                .collect();
            let live_pos = b
                .positions()
                .iter()
                .filter(|p| p.basket == Some(id) && p.level == gl.level)
                .count();
            if live_pos > 0 {
                if let Some(bk) = self.basket_mut(id) {
                    if let Some(x) = bk.levels.iter_mut().find(|x| x.level == gl.level) {
                        x.filled = true;
                    }
                }
            }
            if only_existing && (gl.filled || live_pos > 0) {
                if !gl.is_toucher {
                    core_seen += 1;
                }
                continue;
            }
            if only_existing
                && self.cfg.sync_only_live_levels
                && (live_pend.is_empty() || gl.cancelled)
            {
                if !gl.is_toucher {
                    core_seen += 1;
                }
                continue;
            }

            let mut want = self.scaled_units(gl.base_units, ts) as usize;
            if !gl.is_toucher {
                core_seen += 1;
                if core_seen > allowed {
                    want = 0;
                }
            }
            let have = live_pend.len() + live_pos;
            let mut planned_level = gl.clone();
            if self.cfg.pending_relot_reconcile_target {
                let (budgeted_want, volume) =
                    self.relot_sync_addition_budget(b, id, gl, want, have);
                want = budgeted_want;
                planned_level.volume = volume;
            }
            zadania.push(Zadanie {
                gl: planned_level,
                want,
                have,
                filled: gl.filled || live_pos > 0,
                live_pend,
                market_now: 0,
            });
        }

        let chce_rynek_koszyk = self.wejscie_rynkowe(is_limit) && !zlecenie_stop;
        let drabina = chce_rynek_koszyk && self.cfg.market_entry_mode == MarketEntryMode::Laddered;

        if drabina {
            let wybrany = zadania.iter().rposition(|z| z.want > z.have && !z.filled);
            for (i, z) in zadania.iter_mut().enumerate() {
                if Some(i) != wybrany {
                    z.want = z.have;
                }
            }
        }

        let px_rynek = q.entry(side);
        let chase = match side {
            Side::Buy => (px_rynek - sig_hi.max(sig_lo)).max(0.0),
            Side::Sell => (sig_hi.min(sig_lo) - px_rynek).max(0.0),
        };
        let chase_ok = self.cfg.market_hybrid_max_chase_usd <= 0.0
            || chase <= self.cfg.market_hybrid_max_chase_usd + 1e-9;
        let hybryda_aktywna = self.cfg.market_hybrid_now_units > 0
            && self.sygnal_rynkowy(is_limit)
            && self.cfg.auto_limit
            && !zlecenie_stop
            && !had_positions
            && !only_existing
            && chase_ok;
        if hybryda_aktywna {
            let mut budzet = self.cfg.market_hybrid_now_units as usize;
            for z in zadania.iter_mut().rev() {
                if budzet == 0 {
                    break;
                }
                let nowe = z.want.saturating_sub(z.have);
                z.market_now = nowe.min(budzet);
                budzet -= z.market_now;
            }
            if self.cfg.market_hybrid_pending_units > 0 {
                let mut pending_budget = self.cfg.market_hybrid_pending_units as usize;
                for z in zadania.iter_mut() {
                    let nowe = z.want.saturating_sub(z.have);
                    let pending_new = nowe.saturating_sub(z.market_now);
                    let keep = pending_new.min(pending_budget);
                    pending_budget -= keep;
                    z.want = z.have + z.market_now + keep;
                }
            }
        }

        let rynek_za_krawedzia = self.za_dalsza_krawedzia(side, px_rynek, sig_lo, sig_hi);
        let mut lots_rynkowe = 0.0f64;
        if chce_rynek_koszyk
            || hybryda_aktywna
            || self.cfg.pending_cross_policy == PendingCrossPolicy::Market
        {
            let stops = b.stops_level();
            for z in &zadania {
                if z.want <= z.have {
                    continue;
                }
                let mozliwe = if zlecenie_stop {
                    stop_price_is_valid(side, z.gl.price, &q, stops)
                } else {
                    limit_price_is_valid(side, z.gl.price, &q, stops)
                };
                let nowych = z.want - z.have;
                let hybrydowych = z.market_now.min(nowych);
                let awaryjnych = if chce_rynek_koszyk || !mozliwe {
                    nowych.saturating_sub(hybrydowych)
                } else {
                    0
                };
                let rynkowych = hybrydowych + awaryjnych;
                if rynkowych == 0 {
                    continue;
                }
                if rynek_za_krawedzia {
                    continue;
                }
                let vol = if z.gl.volume > 0.0 { z.gl.volume } else { lot };
                let hybrid_mult = if self.cfg.market_hybrid_lot_mult > 0.0 {
                    self.cfg.market_hybrid_lot_mult
                } else {
                    1.0
                };
                lots_rynkowe += vol * (hybrydowych as f64 * hybrid_mult + awaryjnych as f64);
            }
        }
        let cap_rynek = self.market_risk_cap(id, b);
        let skala_rynek = self.market_risk_scale(lots_rynkowe, px_rynek, sl_koszyka, cap_rynek);
        let mut ryzyko_rynkowe = 0.0f64;
        let mut budzet_wyczerpany = false;
        let mut zgloszona_krawedz = false;

        for z in &zadania {
            let gl = &z.gl;
            let want = z.want;
            let have = z.have;
            let live_pend = z.live_pend.clone();

            if have < want {
                for unit_idx in 0..(want - have) {
                    let stops = b.stops_level();
                    let zmiesci_sie = if zlecenie_stop {
                        stop_price_is_valid(side, gl.price, &q, stops)
                    } else {
                        limit_price_is_valid(side, gl.price, &q, stops)
                    };
                    let vol = if gl.volume > 0.0 { gl.volume } else { lot };
                    let chce_rynek_hybryda = unit_idx < z.market_now;
                    let chce_rynek = chce_rynek_koszyk || chce_rynek_hybryda;

                    let zastepczy = if zmiesci_sie {
                        None
                    } else {
                        Some(self.cfg.pending_cross_policy)
                    };
                    if zastepczy == Some(PendingCrossPolicy::Skip) {
                        self.basket_note(
                            id,
                            ts,
                            format!(
                                "poziom {:.2} pominięty — limit nie mieści się przy cenie {:.2} (stops {stops:.2})",
                                gl.price,
                                q.entry(side)
                            ),
                        );
                        continue;
                    }

                    let jako_rynek = chce_rynek || zastepczy == Some(PendingCrossPolicy::Market);

                    if jako_rynek && rynek_za_krawedzia {
                        *self
                            .odrzuty
                            .entry("PonizejKrawedzi".to_string())
                            .or_insert(0) += 1;
                        let pierwsza = !zgloszona_krawedz;
                        zgloszona_krawedz = true;
                        if pierwsza {
                            self.basket_note(
                                id,
                                ts,
                                format!(
                                    "wejście rynkowe wstrzymane — cena {px_rynek:.2} \
                                     jest za dalszą krawędzią strefy {:.2} \
                                     (zakaz_ponizej_krawedzi)",
                                    Self::dalsza_krawedz(side, sig_lo, sig_hi)
                                ),
                            );
                        }
                        if pierwsza && self.journal.wants(EventLevel::Info) {
                            let snap = self.jsnap(b);
                            self.journal.push(
                                Ev::new(
                                    ts,
                                    EventLevel::Info,
                                    EventCategory::Decision,
                                    EventKind::SignalRejected,
                                )
                                .text(format!(
                                    "koszyk B{id}: wejście rynkowe odrzucone — cena \
                                     {px_rynek:.2} za dalszą krawędzią strefy {:.2}",
                                    Self::dalsza_krawedz(side, sig_lo, sig_hi)
                                ))
                                .basket(id)
                                .reason(RejectCode::ChaseTooFar)
                                .market(snap)
                                .put_f("price", px_rynek)
                                .put_f("krawedz", Self::dalsza_krawedz(side, sig_lo, sig_hi))
                                .put_f("poziom", gl.price)
                                .build(),
                            );
                        }
                        continue;
                    }

                    let hybrid_mult = if chce_rynek_hybryda && self.cfg.market_hybrid_lot_mult > 0.0
                    {
                        self.cfg.market_hybrid_lot_mult
                    } else {
                        1.0
                    };
                    let vol_rynek = if jako_rynek {
                        if crate::lot_growth::enabled(&self.cfg) {
                            // The sequential sum-risk check below decides
                            // whether this minimum leg can still be afforded.
                            (vol * hybrid_mult * skala_rynek).max(self.cfg.lot_min)
                        } else if self.cfg.order_volume_contract_v2 {
                            vol * hybrid_mult * skala_rynek
                        } else {
                            round_lot((vol * hybrid_mult * skala_rynek).max(self.cfg.lot_min))
                        }
                    } else {
                        vol
                    };
                    if jako_rynek {
                        if budzet_wyczerpany {
                            break;
                        }
                        if let (Some(cap), Some(slv)) = (cap_rynek, sl_koszyka) {
                            let r = (px_rynek - slv).abs() * XAU_CONTRACT * vol_rynek;
                            if ryzyko_rynkowe + r > cap + 1e-9 {
                                budzet_wyczerpany = true;
                                self.basket_note(
                                    id,
                                    ts,
                                    format!(
                                        "wejście rynkowe ucięte — budżet ryzyka {cap:.2} $ \
                                         wyczerpany przy cenie wypełnienia {px_rynek:.2}"
                                    ),
                                );
                                break;
                            }
                            ryzyko_rynkowe += r;
                        }
                    }

                    if !self.margines_pozwala(b, self.cfg.ml_min_warstwa) {
                        self.basket_note(
                            id,
                            ts,
                            format!(
                                "siatka ucięta — poziom marginesu poniżej {:.0} %",
                                self.cfg.ml_min_warstwa
                            ),
                        );
                        break;
                    }

                    let mut cena_wejscia = px_rynek;
                    let submission_before = self.order_submission_sequence;
                    let res = if jako_rynek {
                        let tp_rynek = if chce_rynek_hybryda {
                            match self.cfg.market_hybrid_tp_stage {
                                0 => gl.tp,
                                255 => None,
                                n => tps
                                    .get(n.saturating_sub(1) as usize)
                                    .copied()
                                    .or_else(|| tps.last().copied()),
                            }
                        } else {
                            gl.tp
                        };
                        let stop=self.broker_sl(gl.sl,side);
                        let (volume,allocated)=match self.growth_sync_volume(b,id,gl.level,side,
                            px_rynek,stop,vol_rynek,want,want-have-unit_idx) {
                            Ok(v)=>v,Err(_)=>continue,
                        };
                        self.open_market_order_allocated(
                            b,
                            OrderReq {
                                side,
                                volume,
                                sl: stop,
                                tp: tp_rynek,
                                basket: Some(id),
                                level: gl.level,
                                is_toucher: gl.is_toucher,
                                comment: format!("B{id}"),
                            },
                            allocated,
                        )
                        .map(|t| (t, true))
                    } else {
                        let (kind, price) = match zastepczy {
                            _ if zlecenie_stop && zmiesci_sie => {
                                (PendingKind::stop(side), gl.price)
                            }
                            Some(PendingCrossPolicy::Stop)
                                if stop_price_is_valid(side, gl.price, &q, stops) =>
                            {
                                (PendingKind::stop(side), gl.price)
                            }
                            Some(PendingCrossPolicy::Stop) | Some(PendingCrossPolicy::Shift) => (
                                PendingKind::limit(side),
                                clamp_limit_price(side, gl.price, &q, stops),
                            ),
                            _ => (PendingKind::limit(side), gl.price),
                        };
                        cena_wejscia = price;
                        if self.za_dalsza_krawedzia(side, price, sig_lo, sig_hi) {
                            self.basket_note(
                                id,
                                ts,
                                format!(
                                    "poziom {:.2} pominięty — po dosunięciu do {price:.2} \
                                     leżałby za dalszą krawędzią strefy {:.2}",
                                    gl.price,
                                    Self::dalsza_krawedz(side, sig_lo, sig_hi)
                                ),
                            );
                            *self
                                .odrzuty
                                .entry("PonizejKrawedzi".to_string())
                                .or_insert(0) += 1;
                            continue;
                        }
                        let stop=self.broker_sl(gl.sl,side);
                        let (volume,allocated)=match self.growth_sync_volume(b,id,gl.level,side,
                            price,stop,vol,want,want-have-unit_idx) {
                            Ok(v)=>v,Err(_)=>continue,
                        };
                        self.place_pending_order_allocated(
                            b,
                            PendingReq {
                                kind,
                                volume,
                                price,
                                sl: stop,
                                tp: gl.tp,
                                basket: Some(id),
                                level: gl.level,
                                is_toucher: gl.is_toucher,
                                is_topup: false,
                                no_market_fallback: false,
                                comment: format!("B{id}"),
                            },
                            allocated,
                        )
                        .map(|t| (t, false))
                    };
                    match res {
                        Ok((t, true)) => {
                            if let Some(bk) = self.basket_mut(id) {
                                bk.tickets.push(t);
                                bk.state = BasketState::Working;
                                bk.had_positions = true;
                                if drabina || chce_rynek_hybryda {
                                    if let Some(x) =
                                        bk.levels.iter_mut().find(|x| x.level == gl.level)
                                    {
                                        x.filled = true;
                                    }
                                    bk.last_entry_px = Some(px_rynek);
                                }
                            }
                            self.apply_virtual_sl(b, t, gl.sl);
                            diag_wejscie(
                                ts,
                                id,
                                if chce_rynek_hybryda {
                                    "hybryda-market"
                                } else if chce_rynek {
                                    "koszyk-rynkowy"
                                } else {
                                    "rynek-zamiast-limitu"
                                },
                                side,
                                sig_lo,
                                sig_hi,
                                px_rynek,
                                px_rynek,
                                gl.level,
                            );
                            placed += 1;
                        }
                        Ok((t, false)) => {
                            if let Some(bk) = self.basket_mut(id) {
                                bk.pendings.push(t);
                            }
                            diag_wejscie(
                                ts,
                                id,
                                "limit",
                                side,
                                sig_lo,
                                sig_hi,
                                cena_wejscia,
                                px_rynek,
                                gl.level,
                            );
                            placed += 1;
                        }
                        Err(_) => {
                            if self.order_submission_sequence != submission_before {
                                self.remember_rearm_unconfirmed(b, id);
                            }
                            break;
                        }
                    }
                }
            } else if have > want {
                for t in live_pend.into_iter().take(have - want) {
                    if cien::czynny() {
                        cien::z(
                            cakt::A_ZYCIE_ZLEC,
                            t,
                            czr::Z_SYNC_GRID_KASUJ,
                            std::panic::Location::caller().line(),
                        );
                    }
                    if b.cancel_pending(t).is_ok() {
                        if let Some(bk) = self.basket_mut(id) {
                            bk.pendings.retain(|x| *x != t);
                        }
                        removed += 1;
                    }
                }
            }
        }
        if only_existing && (placed > 0 || removed > 0) {
            self.basket_note(
                id,
                ts,
                format!(
                    "reżim zmienności ×{:.2}: +{placed} / −{removed} limitów",
                    self.vol_factor(ts)
                ),
            );
        }
        placed
    }

    #[inline]
    fn broker_sl(&self, sl: Option<Px>, side: Side) -> Option<Px> {
        match sl {
            Some(v) if self.cfg.virtual_sl_all && self.cfg.vsl_broker_offset > 0.0 => {
                Some(v - side.sign() * self.cfg.vsl_broker_offset)
            }
            other => other,
        }
    }

    fn apply_virtual_sl<B: Broker>(&self, b: &mut B, t: Ticket, sl: Option<Px>) {
        if !self.cfg.virtual_sl_all || sl.is_none() {
            return;
        }
        cien::z(cakt::A_STOP, t, czr::Z_VIRTUAL_SL_ZAPIS, 0);
        if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
            p.vsl = sl;
        }
    }

    fn units_for_level(
        &self,
        level: Px,
        sl: Option<Px>,
        tp1: Option<Px>,
        base_units: u32,
        n_levels: usize,
        is_limit: bool,
    ) -> u32 {
        let mnoznik_kraty = self.cfg.grid_anchor_absolute
            && self.cfg.units_per_level
            && (is_limit || self.cfg.units_per_level_zone);
        let mut u = if n_levels > 1 && !mnoznik_kraty {
            1
        } else {
            base_units
        };

        if self.cfg.entry_risk_budget > 0.0 {
            if let Some(s) = sl {
                let d = (level - s).abs();
                if d > 0.0 {
                    u = u.min((self.cfg.entry_risk_budget / d).round().max(1.0) as u32);
                }
            }
        }
        if self.cfg.entry_tp1_budget > 0.0 {
            if let Some(t) = tp1 {
                let d = (t - level).abs();
                if d > 0.0 {
                    u = u.min((self.cfg.entry_tp1_budget / d).round().max(1.0) as u32);
                }
            }
        }
        u.max(1)
    }

    fn cele_z_runnerem(&self, side: Side, tps: &[Px]) -> Vec<Px> {
        self.cele_z_runnerem_od(side, tps, None)
    }

    fn cele_z_runnerem_od(&self, side: Side, tps: &[Px], rynek: Option<Px>) -> Vec<Px> {
        let mut out: Vec<Px> = match (self.cfg.cele_pomin_za_cena, rynek) {
            (true, Some(r)) => {
                let przed: Vec<Px> = tps
                    .iter()
                    .copied()
                    .filter(|t| match side {
                        Side::Buy => *t > r,
                        Side::Sell => *t < r,
                    })
                    .collect();
                if przed.is_empty() {
                    tps.last().copied().into_iter().collect()
                } else {
                    przed
                }
            }
            _ => tps.to_vec(),
        };
        let n = self.cfg.runner_cele_n;
        if n == 0 || self.cfg.runner_cele_krok <= 0.0 {
            return out;
        }
        let Some(&ostatni) = out.last() else {
            return out;
        };
        for i in 1..=n {
            out.push(ostatni + side.sign() * self.cfg.runner_cele_krok * i as f64);
        }
        out
    }

    fn target_for(&self, tps: &[Px], side: Side, idx: usize, total: usize) -> Option<Px> {
        self.target_for_ex(tps, side, idx, total, false)
    }

    fn target_for_ex(
        &self,
        tps: &[Px],
        side: Side,
        idx: usize,
        total: usize,
        tp_open: bool,
    ) -> Option<Px> {
        if tps.is_empty() {
            return None;
        }
        let open_extra = tp_open && self.cfg.tp_open_extra && self.cfg.tp_open_offset != 0.0;
        let last = if open_extra {
            *tps.last().unwrap() + side.sign() * self.cfg.tp_open_offset
        } else {
            *tps.last().unwrap()
        };
        if self.cfg.cele_na_ostatnim {
            return Some(last);
        }
        match self.cfg.tp_schedule {
            TpSchedule::AllRunners => Some(last),
            TpSchedule::AllAtTp1 => Some(tps[0]),
            TpSchedule::Ladder | TpSchedule::OfficialPct | TpSchedule::ScaleOutPct => {
                let tier = (idx * tps.len()) / total.max(1);
                Some(tps[tier.min(tps.len() - 1)])
            }
            TpSchedule::OfficialCounts => {
                let counts = self.cfg.parse_counts();
                let mut acc = 0usize;
                for (i, c) in counts.iter().enumerate() {
                    acc += *c as usize;
                    if idx < acc {
                        return Some(tps[i.min(tps.len() - 1)]);
                    }
                }
                match self.cfg.last_runner {
                    LastRunner::NoTp => None,
                    LastRunner::NextTp => Some(tps[counts.len().min(tps.len() - 1)]),
                    LastRunner::Runner => {
                        if self.cfg.tp_freeze_after_ladder {
                            Some(last)
                        } else {
                            Some(last + side.sign() * self.cfg.tp_open_offset)
                        }
                    }
                }
            }
        }
    }


    fn handle_tp_hit<B: Broker>(
        &mut self,
        b: &mut B,
        id: u32,
        index: Option<usize>,
        ts: Ts,
        src: &str,
    ) {
        if !self.basket(id).map(|x| x.ma_pozycje()).unwrap_or(false) {
            self.cel_bez_pozycji(b, id, index, ts, src);
            return;
        }
        self.handle_tp_hit_z_pozycja(b, id, index, ts, src);
    }

    fn cel_bez_pozycji<B: Broker>(
        &mut self,
        b: &mut B,
        id: u32,
        index: Option<usize>,
        ts: Ts,
        src: &str,
    ) {
        if self.basket_exit_pending(id) {
            return;
        }
        let (tps, obserwowany) = match self.basket(id) {
            Some(bk) => (bk.tps.clone(), bk.etap_obserwowany()),
            None => return,
        };
        let stage = index.unwrap_or(obserwowany + 1).max(1);
        if stage <= obserwowany {
            return;
        }
        self.basket_note(
            id,
            ts,
            format!("TP{stage} bez naszej pozycji ({src}) — etap koszyka NIE rusza"),
        );
        if self.keep_explicit_pending(id) { return; }
        if !self.cfg.pending_drop_on_target {
            if let Some(bk) = self.basket_mut(id) {
                bk.plan_wykonany_do = bk.plan_wykonany_do.max(stage);
            }
            return;
        }
        let level = tps
            .get(stage - 1)
            .copied()
            .or_else(|| tps.last().copied())
            .unwrap_or(0.0);
        self.drop_grid_on_target(b, id, stage, level, ts);
    }

    fn handle_tp_hit_z_pozycja<B: Broker>(
        &mut self,
        b: &mut B,
        id: u32,
        index: Option<usize>,
        ts: Ts,
        src: &str,
    ) {
        if self.basket_exit_pending(id) {
            return;
        }
        self.last_tp_hit_ts = ts;
        let (side, tps, stage_now) = match self.basket(id) {
            Some(bk) => (bk.side, bk.tps.clone(), bk.tp_stage),
            None => return,
        };
        let target_stage = index.unwrap_or(stage_now + 1).max(1);
        if target_stage <= stage_now {
            return;
        }
        if let Some(bk) = self.basket_mut(id) {
            bk.last_tp_ts = ts;
        }

        let steps: Vec<usize> = if self.cfg.tp_hit_fill_stages {
            (stage_now + 1..=target_stage).collect()
        } else {
            vec![target_stage]
        };

        for st in steps {
            if let Some(bk) = self.basket_mut(id) {
                bk.tp_stage = st;
                bk.plan_wykonany_do = bk.plan_wykonany_do.max(st);
            }
            self.basket_note(id, ts, format!("TP{st} osiągnięty ({src})"));
            self.bank_on_tp(b, id, st, ts);
        }

        if self.cfg.bank_all_at_stage > 0 && target_stage >= self.cfg.bank_all_at_stage as usize {
            if self.cfg.confirmed_exit_retry {
                self.request_confirmed_exit(b, id, ts, CloseReason::Tp);
                return;
            }
            let zywe: Vec<Ticket> = self
                .basket(id)
                .map(|x| x.tickets.clone())
                .unwrap_or_default();
            let mut n = 0usize;
            let mut suma = 0.0;
            for t in zywe {
                if b.find_position(t).is_none() {
                    continue;
                }
                cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_TP_ZAMKNIJ, 0);
                if let Ok(p) = b.close_position(t, CloseReason::Tp) {
                    suma += p;
                    n += 1;
                }
            }
            let sk = self.cancel_pendings(b, id);
            self.book_command_profit_legacy(id, suma);
            if let Some(bk) = self.basket_mut(id) {
                bk.state = BasketState::Done;
            }
            self.basket_note(
                id,
                ts,
                format!(
                    "BANK CAŁOŚCI na TP{target_stage} — zamknięto {n} ({suma:+.2} $), \
                     skasowano {sk} limitów"
                ),
            );
            return;
        }

        if self.cfg.no_tp_after_stage > 0 && target_stage >= self.cfg.no_tp_after_stage as usize {
            let zywe: Vec<Ticket> = self
                .basket(id)
                .map(|bk| {
                    bk.tickets
                        .iter()
                        .copied()
                        .filter(|t| b.find_position(*t).is_some())
                        .collect()
                })
                .unwrap_or_default();
            let mut zdjete = 0usize;
            for t in zywe {
                let (sl, tp) = match b.find_position(t) {
                    Some(p) => (p.sl, p.tp),
                    None => continue,
                };
                let bez_celu = match tp {
                    Some(_) => {
                        cien::z(cakt::A_CEL, t, czr::Z_TP_ZDEJMIJ_CEL_RUNNERA, 0);
                        let ok = b.modify_position(t, sl, None).is_ok();
                        if ok {
                            zdjete += 1;
                        }
                        ok
                    }
                    None => true,
                };
                if bez_celu {
                    if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
                        p.is_runner = true;
                    }
                }
            }
            if zdjete > 0 {
                self.basket_note(
                    id,
                    ts,
                    format!("ZDJĘTY SUFIT po TP{target_stage} — {zdjete} pozycji bez celu"),
                );
            }
        }

        if self.cfg.pyramid_after_stage > 0
            && self.margines_pozwala(b, self.cfg.ml_min_piramida)
            && target_stage >= self.cfg.pyramid_after_stage as usize
        {
            let dane = self.basket(id).map(|bk| {
                (
                    bk.pyramided,
                    bk.tempo_fast,
                    bk.sl,
                    bk.tps.first().copied(),
                    bk.tps.last().copied(),
                )
            });
            if let Some((zrobiona, przelot, bsl, tp1, ostatni)) = dane {
                let regime_ok = if self.cfg.pyramid_regime_lookback == 0 {
                    true
                } else {
                    let n = self.cfg.pyramid_regime_lookback as usize;
                    let ogon = &self.regime_hist[self.regime_hist.len().saturating_sub(n)..];
                    if ogon.len() < n.min(8) {
                        true
                    } else {
                        let przeloty = ogon.iter().filter(|(_, p)| *p).count();
                        100.0 * przeloty as f64 / ogon.len() as f64
                            <= self.cfg.pyramid_regime_max_fast_pct
                    }
                };

                let kapital_ok = self.cfg.pyramid_min_equity_mult <= 0.0
                    || self.stats.balance
                        >= self.stats.start_balance * self.cfg.pyramid_min_equity_mult;

                if !zrobiona && !przelot && regime_ok && kapital_ok {
                    if let Some(px) = tp1 {
                        let vol = (self.lot_size(self.podstawa_lota())
                            * self.cfg.pyramid_lot_mult.max(0.0))
                        .max(self.cfg.lot_min);
                        cien::z(cakt::A_DOLOZENIE, id as u64, czr::Z_PIRAMIDA_PO_TP, 0);
                        let res = self.place_pending_order(
                            b,
                            PendingReq {
                                kind: PendingKind::limit(side),
                                volume: vol,
                                price: px,
                                sl: self.broker_sl(bsl, side),
                                tp: ostatni,
                                basket: Some(id),
                                level: -3,
                                is_toucher: false,
                                is_topup: false,
                                no_market_fallback: false,
                                comment: format!("B{id}"),
                            },
                        );
                        match res {
                            Ok(t) => {
                                if let Some(bk) = self.basket_mut(id) {
                                    bk.pendings.push(t);
                                    bk.pyramided = true;
                                }
                                self.basket_note(
                                    id,
                                    ts,
                                    format!(
                                        "PIRAMIDA: TP{target_stage} potwierdził ruch — dokładka \
                                         {vol:.2} lota limitem na {px:.2} (cofnięcie do TP1)"
                                    ),
                                );
                            }
                            Err(e) => {
                                if let Some(bk) = self.basket_mut(id) {
                                    bk.pyramided = true;
                                }
                                self.basket_note(
                                    id,
                                    ts,
                                    format!("PIRAMIDA odrzucona przez brokera: {e:?}"),
                                );
                            }
                        }
                    }
                }
            }
        }

        let cancel_stage = if self.keep_explicit_pending(id) { None } else { match self.cfg.pending_lifetime {
            PendingLifetime::UntilTp1 => Some(1),
            PendingLifetime::UntilTp2 => Some(2),
            PendingLifetime::UntilTp3 => Some(3),
            PendingLifetime::Never => None,
        }};
        if let Some(cs) = cancel_stage {
            if target_stage >= cs {
                let n = self.cancel_pendings(b, id);
                if n > 0 {
                    self.basket_note(id, ts, format!("skasowano {n} niezafillowanych limitów"));
                }
            }
        }

        let prog_be = if self.cfg.be_od_etapu > 0 {
            Some(self.cfg.be_od_etapu)
        } else if self.cfg.be_at_tp1 {
            Some(1)
        } else {
            None
        };
        if let Some(pr) = prog_be {
            let dosc_pozycji = self.cfg.be_min_pozycji == 0
                || self
                    .basket(id)
                    .map(|bk| {
                        bk.tickets
                            .iter()
                            .filter(|t| b.find_position(**t).is_some())
                            .count()
                            >= self.cfg.be_min_pozycji as usize
                    })
                    .unwrap_or(false);
            if target_stage >= pr as usize && dosc_pozycji {
                self.move_basket_to_be(b, id, ts);
            }
        }

        if self.cfg.sl_po_tp1_na_krawedz && target_stage >= 1 {
            if let Some((sl_lo, sl_hi)) = self.basket(id).map(|bk| (bk.entry_lo, bk.entry_hi)) {
                let (a, b2) = (sl_lo.min(sl_hi), sl_lo.max(sl_hi));
                if b2 - a > 1e-9 {
                    let krawedz = side.worse_edge(a, b2);
                    self.set_basket_sl(b, id, krawedz, ts);
                }
            }
        }

        if self.cfg.sl_polowa_od_konca > 0 {
            let prog = tps.len().saturating_sub(self.cfg.sl_polowa_od_konca);
            if prog > 0 && target_stage >= prog {
                self.sl_polowa_drogi(b, id, ts);
            }
        }

        if self.cfg.ladder_from_tp > 0 && target_stage >= self.cfg.ladder_from_tp {
            let li = target_stage.saturating_sub(1 + self.cfg.ladder_lag);
            if let Some(&anchor) = tps.get(li) {
                let v = anchor - side.sign() * self.cfg.ladder_offset;
                self.set_basket_sl(b, id, v, ts);
            }
        }

        self.apply_smart_sl(b, id, ts);

        self.retarget(b, id, ts);
    }

    fn drop_grid_on_target<B: Broker>(
        &mut self,
        b: &mut B,
        id: u32,
        stage: usize,
        level: Px,
        ts: Ts,
    ) {
        if self.basket_exit_pending(id) || self.keep_explicit_pending(id) {
            return;
        }
        if let Some(bk) = self.basket_mut(id) {
            bk.plan_wykonany_do = bk.plan_wykonany_do.max(stage);
        }
        let market_cancel_stage = self.cfg.market_unfilled_cancel_stage;
        let market_bez_fillu = market_cancel_stage > 0
            && market_cancel_stage < 255
            && self
                .basket(id)
                .map(|bk| self.sygnal_rynkowy(bk.is_limit))
                .unwrap_or(false);
        let cancel_stage = if market_bez_fillu {
            market_cancel_stage as usize
        } else {
            match self.cfg.pending_lifetime {
                PendingLifetime::UntilTp1 => 1,
                PendingLifetime::UntilTp2 => 2,
                PendingLifetime::UntilTp3 => 3,
                PendingLifetime::Never => return,
            }
        };
        if stage < cancel_stage {
            return;
        }
        let laska = (self.cfg.pending_drop_grace_min.max(0.0) * 60_000.0) as i64;
        if laska > 0 && self.blisko_strefy(b, id) {
            if let Some(bk) = self.basket_mut(id) {
                if bk.drop_po_ts == 0 {
                    bk.drop_po_ts = ts + laska;
                }
            }
            return;
        }
        let n = self.cancel_pendings_keep(b, id, self.cfg.pending_drop_keep_n as usize);
        if n == 0 {
            return;
        }
        let tekst =
            format!("TP{stage} ({level:.2}) osiągnięty bez wejścia — skasowano {n} limitów");
        self.basket_note(id, ts, tekst.clone());
        if self.journal.wants(EventLevel::Warn) {
            let snap = self.jsnap(b);
            self.journal.push(
                Ev::new(
                    ts,
                    EventLevel::Warn,
                    EventCategory::Order,
                    EventKind::PendingCancelled,
                )
                .text(tekst)
                .basket(id)
                .reason(RejectCode::TargetReachedWithoutEntry)
                .market(snap)
                .put("cancelled", n as u64)
                .put_f("level", level)
                .put("stage", stage as u64)
                .build(),
            );
        }
    }

    fn apply_smart_sl<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts) {
        if self.cfg.smart_sl_mode == SmartSlMode::Off {
            return;
        }
        let (side, tps, stage, base_sl, secured) = match self.basket(id) {
            Some(bk) => (bk.side, bk.tps.clone(), bk.tp_stage, bk.sl, bk.secured),
            None => return,
        };
        if self.cfg.smart_sl_only_after_rf && !secured {
            return;
        }
        let tickets: Vec<Ticket> = self
            .basket(id)
            .map(|x| x.tickets.clone())
            .unwrap_or_default();

        let mut live: Vec<(Ticket, Px, Option<Px>, Option<Px>)> = tickets
            .iter()
            .filter_map(|t| b.find_position(*t))
            .filter(|p| !p.frozen)
            .map(|p| (p.ticket, p.open_price, p.sl, p.tp))
            .collect();
        if live.is_empty() {
            return;
        }
        live.sort_by(|a, c| match side {
            Side::Buy => a.1.partial_cmp(&c.1).unwrap(),
            Side::Sell => c.1.partial_cmp(&a.1).unwrap(),
        });

        let use_be = matches!(
            self.cfg.smart_sl_mode,
            SmartSlMode::BreakevenOnly | SmartSlMode::LadderWithBe
        );
        let use_ladder = matches!(
            self.cfg.smart_sl_mode,
            SmartSlMode::Ladder | SmartSlMode::LadderWithBe
        );
        let q = b.quote();
        let sign = side.sign();
        let mut moved = 0usize;

        for (rank, (t, open, cur_sl, tp)) in live.into_iter().enumerate() {
            let be_px = open + sign * self.cfg.be_offset;
            let step = stage as isize - rank as isize - self.cfg.smart_sl_delay as isize;

            let want: Option<Px> = if step <= 0 {
                base_sl
            } else if use_be && step == 1 {
                Some(be_px)
            } else if use_ladder {
                let idx = step - 1 - if use_be { 1 } else { 0 };
                if idx < 0 {
                    base_sl
                } else {
                    tps.get(idx as usize)
                        .copied()
                        .or_else(|| tps.last().copied())
                }
            } else {
                Some(be_px)
            };

            let floor = if secured && self.cfg.smart_sl_floor_be_after_rf {
                Some(match base_sl {
                    None => be_px,
                    Some(s) => {
                        if side == Side::Buy {
                            s.max(be_px)
                        } else {
                            s.min(be_px)
                        }
                    }
                })
            } else {
                base_sl
            };

            let v = match (want, floor) {
                (Some(w), Some(f)) => Some(if side == Side::Buy {
                    w.max(f)
                } else {
                    w.min(f)
                }),
                (a, c) => a.or(c),
            };
            let Some(v) = v else { continue };

            let tighter = cur_sl.map(|s| side.better(s, v)).unwrap_or(true);
            if !tighter {
                continue;
            }
            if sl_is_valid(side, v, &q, b.stops_level()) && self.try_modify(b, t, Some(v), tp, ts) {
                moved += 1;
            }
        }
        if moved > 0 {
            self.basket_note(
                id,
                ts,
                format!("SMART SL: przesunięto {moved} stopów (etap {stage})"),
            );
        }
    }

    fn bank_on_tp<B: Broker>(&mut self, b: &mut B, id: u32, stage: usize, ts: Ts) {
        let live: Vec<Ticket> = match self.basket(id) {
            Some(bk) => bk
                .tickets
                .iter()
                .copied()
                .filter(|t| b.find_position(*t).map(|p| !p.frozen).unwrap_or(false))
                .collect(),
            None => return,
        };
        if live.is_empty() {
            return;
        }
        let n = live.len();

        if self.cfg.partial_pct_od_pierwotnego {
            let teraz: Vec<(Ticket, f64)> = live
                .iter()
                .filter_map(|t| b.find_position(*t).map(|p| (*t, p.volume)))
                .collect();
            if let Some(bk) = self.basket_mut(id) {
                for (t, v) in teraz {
                    if !bk.wol_pierwotny.iter().any(|(x, _)| *x == t) {
                        bk.wol_pierwotny.push((t, v));
                    }
                }
            }
        }

        let pct = match self.cfg.tp_schedule {
            TpSchedule::AllRunners | TpSchedule::Ladder | TpSchedule::AllAtTp1
                if self.cfg.assign_tp_per_position =>
            {
                0.0
            }
            TpSchedule::OfficialCounts => {
                let counts = self.cfg.parse_counts();
                let c = counts
                    .get(stage - 1)
                    .copied()
                    .unwrap_or(if self.cfg.official_spp {
                        counts.last().copied().unwrap_or(0)
                    } else {
                        0
                    });
                c as f64 / n as f64 * 100.0
            }
            TpSchedule::OfficialPct => *self.cfg.official_pct.get(stage - 1).unwrap_or(&if self
                .cfg
                .official_spp
            {
                self.cfg.official_pct[3]
            } else {
                0.0
            }),
            TpSchedule::ScaleOutPct => self.cfg.scale_out_pct,
            _ => 0.0,
        };
        if pct <= 0.0 {
            return;
        }

        let q = b.quote();
        let mut sorted = live.clone();
        sorted.sort_by(|a, c| {
            let pa = b.find_position(*a).map(|p| p.profit_usd(&q)).unwrap_or(0.0);
            let pc = b.find_position(*c).map(|p| p.profit_usd(&q)).unwrap_or(0.0);
            match self.cfg.bank_from {
                BankFrom::Worst => pa.partial_cmp(&pc).unwrap(),
                BankFrom::Best => pc.partial_cmp(&pa).unwrap(),
            }
        });

        let vols: Vec<f64> = live
            .iter()
            .filter_map(|t| b.find_position(*t).map(|p| p.volume))
            .collect();
        if self.cfg.partials_allowed(&vols) {
            let mut done = 0usize;
            for t in &sorted {
                let vol = match b.find_position(*t) {
                    Some(p) => p.volume,
                    None => continue,
                };
                let baza = if self.cfg.partial_pct_od_pierwotnego {
                    self.basket(id)
                        .and_then(|bk| {
                            bk.wol_pierwotny
                                .iter()
                                .find(|(x, _)| x == t)
                                .map(|(_, v)| *v)
                        })
                        .unwrap_or(vol)
                } else {
                    vol
                };
                let Some(cut) =
                    partial_close_volume(vol, baza * pct / 100.0, b.volume_min(), b.volume_step())
                else {
                    continue;
                };
                cien::z(cakt::A_WOLUMEN_POZ, *t, czr::Z_BANK_ON_TP_CZESC, 0);
                if b.close_partial(*t, cut, CloseReason::Partial).is_ok() {
                    done += 1;
                }
            }
            if done > 0 {
                self.basket_note(
                    id,
                    ts,
                    format!("zainkasowano {pct:.0}% wolumenu z {done} poz. na TP{stage}"),
                );
            }
            return;
        }

        let close_n = self.cfg.bank_count(n, pct);
        if close_n == 0 {
            return;
        }
        let limit = if self.cfg.bank_close_last {
            close_n.min(n)
        } else {
            close_n.min(n.saturating_sub(1))
        };
        if limit == 0 {
            return;
        }

        for t in sorted.into_iter().take(limit) {
            cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_BANK_ON_TP_CALOSC, 0);
            let _ = b.close_position(t, CloseReason::Partial);
        }
        self.basket_note(id, ts, format!("zainkasowano {limit} poz. na TP{stage}"));
    }

    #[track_caller]
    fn close_basket<B: Broker>(
        &mut self,
        b: &mut B,
        id: u32,
        ts: Ts,
        reason: CloseReason,
    ) -> (usize, usize, f64) {
        if self.cfg.confirmed_exit_retry {
            return self.request_confirmed_exit(b, id, ts, reason);
        }
        let tickety: Vec<Ticket> = b
            .positions()
            .iter()
            .filter(|p| p.basket == Some(id) && !p.frozen)
            .map(|p| p.ticket)
            .collect();
        let ile_bylo = tickety.len();
        let mut wynik = 0.0;
        let mut zamkniete = 0usize;
        for t in tickety {
            if cien::czynny() {
                cien::z(
                    cakt::A_ZYCIE_POZ,
                    t,
                    czr::L_CLOSE_BASKET,
                    std::panic::Location::caller().line(),
                );
            }
            if let Ok(z) = b.close_position(t, reason) {
                wynik += z;
                zamkniete += 1;
            }
        }
        let skasowane = self.cancel_pendings(b, id);
        self.book_command_profit_legacy(id, wynik);
        if let Some(bk) = self.basket_mut(id) {
            if zamkniete == ile_bylo {
                bk.state = BasketState::Done;
            }
        }
        self.basket_note(
            id,
            ts,
            format!(
                "koszyk zamknięty ({}) — {zamkniete}/{ile_bylo} poz. {wynik:+.2} $, skasowano {skasowane} limitów",
                journal::close_reason_str(reason)
            ),
        );
        (zamkniete, skasowane, wynik)
    }

    fn request_confirmed_exit<B: Broker>(
        &mut self,
        b: &mut B,
        id: u32,
        ts: Ts,
        reason: CloseReason,
    ) -> (usize, usize, f64) {
        let Some(bk) = self.basket_mut(id) else {
            return (0, 0, 0.0);
        };
        let first = bk.pending_exit.is_none();
        if first {
            bk.pending_exit = Some(PendingBasketExit {
                reason,
                last_attempt_ts: ts,
            });
        }
        self.attempt_confirmed_exit(b, id, ts, first)
    }

    fn attempt_confirmed_exit<B: Broker>(
        &mut self,
        b: &mut B,
        id: u32,
        ts: Ts,
        first: bool,
    ) -> (usize, usize, f64) {
        let Some(intent) = self.basket(id).and_then(|x| x.pending_exit.clone()) else {
            return (0, 0, 0.0);
        };
        let exposed = b.positions().iter().any(|p| p.basket == Some(id))
            || b.pendings().iter().any(|p| p.basket == Some(id));
        if !exposed {
            if let Some(bk) = self.basket_mut(id) {
                bk.tickets.clear();
                bk.pendings.clear();
                bk.state = BasketState::Done;
                bk.pending_exit = None;
            }
            return (0, 0, 0.0);
        }
        if !first && ts.saturating_sub(intent.last_attempt_ts) < 1_000 {
            return (0, 0, 0.0);
        }
        if let Some(bk) = self.basket_mut(id) {
            bk.state = BasketState::Working;
            bk.pending_exit.as_mut().unwrap().last_attempt_ts = ts;
        }
        let cancelled = self.cancel_pendings_now(b, id);
        let tickets: Vec<Ticket> = b
            .positions()
            .iter()
            .filter(|p| p.basket == Some(id))
            .map(|p| p.ticket)
            .collect();
        let mut closed = 0;
        let mut profit = 0.0;
        for t in tickets {
            self.queued_exits.remove(&t);
            self.desired.remove(&t);
            if b.find_position(t).map(|p| p.frozen).unwrap_or(true) {
                continue;
            }
            if let Ok(z) = b.close_position(t, intent.reason) {
                closed += 1;
                profit += z;
            }
        }
        self.book_command_profit_legacy(id, profit);
        let positions: Vec<Ticket> = b
            .positions()
            .iter()
            .filter(|p| p.basket == Some(id))
            .map(|p| p.ticket)
            .collect();
        let pendings: Vec<Ticket> = b
            .pendings()
            .iter()
            .filter(|p| p.basket == Some(id))
            .map(|p| p.ticket)
            .collect();
        let left_pos = positions.len();
        let left_pending = pendings.len();
        if let Some(bk) = self.basket_mut(id) {
            bk.tickets = positions;
            bk.pendings = pendings;
            if left_pos == 0 && left_pending == 0 {
                bk.state = BasketState::Done;
                bk.pending_exit = None;
            }
        }
        self.basket_note(id, ts, format!(
            "wyjście {} ({}): zamknięto {closed}, anulowano {cancelled}, wynik {profit:+.2} $; pozostaje {left_pos} pozycji / {left_pending} zleceń",
            if left_pos == 0 && left_pending == 0 { "potwierdzone" } else { "oczekuje na brokera" },
            journal::close_reason_str(intent.reason)));
        (closed, cancelled, profit)
    }

    fn retry_confirmed_exits<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        if !self.cfg.confirmed_exit_retry {
            return;
        }
        let ids: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.pending_exit.is_some())
            .map(|x| x.id)
            .collect();
        for id in ids {
            self.attempt_confirmed_exit(b, id, ts, false);
        }
    }

    fn inkasuj_partials<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts) {
        let pct = self.cfg.partials_pct;
        if pct <= 0.0 {
            self.basket_note(
                id,
                ts,
                "„take partials\" bez transzy (partials_pct = 0)".into(),
            );
            return;
        }
        let live: Vec<Ticket> = match self.basket(id) {
            Some(bk) => bk
                .tickets
                .iter()
                .copied()
                .filter(|t| b.find_position(*t).map(|p| !p.frozen).unwrap_or(false))
                .collect(),
            None => return,
        };
        if live.is_empty() {
            self.basket_note(
                id,
                ts,
                "„take partials\" — koszyk nie ma czym inkasować".into(),
            );
            return;
        }
        let n = live.len();

        let q = b.quote();
        let mut sorted = live.clone();
        sorted.sort_by(|a, c| {
            let pa = b.find_position(*a).map(|p| p.profit_usd(&q)).unwrap_or(0.0);
            let pc = b.find_position(*c).map(|p| p.profit_usd(&q)).unwrap_or(0.0);
            match self.cfg.bank_from {
                BankFrom::Worst => pa.partial_cmp(&pc).unwrap(),
                BankFrom::Best => pc.partial_cmp(&pa).unwrap(),
            }
        });

        let vols: Vec<f64> = live
            .iter()
            .filter_map(|t| b.find_position(*t).map(|p| p.volume))
            .collect();
        if self.cfg.partials_allowed(&vols) {
            let mut done = 0usize;
            for t in &sorted {
                let vol = match b.find_position(*t) {
                    Some(p) => p.volume,
                    None => continue,
                };
                let Some(cut) =
                    partial_close_volume(vol, vol * pct / 100.0, b.volume_min(), b.volume_step())
                else {
                    continue;
                };
                cien::z(cakt::A_WOLUMEN_POZ, *t, czr::Z_INKASO_CZESC, 0);
                if b.close_partial(*t, cut, CloseReason::Partial).is_ok() {
                    done += 1;
                }
            }
            if done > 0 {
                self.basket_note(
                    id,
                    ts,
                    format!("„take partials\" z kanału: {pct:.0}% wolumenu z {done} poz."),
                );
            }
            return;
        }

        let close_n = self.cfg.bank_count(n, pct);
        let limit = if self.cfg.bank_close_last {
            close_n.min(n)
        } else {
            close_n.min(n.saturating_sub(1))
        };
        if limit == 0 {
            self.basket_note(
                id,
                ts,
                format!("„take partials\" bez skutku — {pct:.0}% z {n} poz. to zero warstw"),
            );
            return;
        }
        for t in sorted.into_iter().take(limit) {
            cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_INKASO_CALOSC, 0);
            let _ = b.close_position(t, CloseReason::Partial);
        }
        self.basket_note(id, ts, format!("„take partials\" z kanału: {limit} poz."));
    }

    fn retarget<B: Broker>(&mut self, b: &mut B, id: u32, _ts: Ts) {
        if self.basket_exit_pending(id) {
            return;
        }
        let (side, tps, stage, tp_open) = match self.basket(id) {
            Some(bk) => (bk.side, bk.tps.clone(), bk.tp_stage, bk.tp_open),
            None => return,
        };
        let tickets: Vec<Ticket> = self
            .basket(id)
            .map(|x| x.tickets.clone())
            .unwrap_or_default();
        let next = tps.get(stage).copied();
        let last = tps.last().copied();
        let q = b.quote();

        for t in tickets {
            let (cur_tp, frozen, sl) = match b.find_position(t) {
                Some(p) => (p.tp, p.frozen, p.sl),
                None => continue,
            };
            if frozen || cur_tp.is_none() {
                continue;
            }
            let keep_final = self.cfg.retarget_respects_final_target && self.cfg.cele_na_ostatnim;
            let newtp = if keep_final {
                self.target_for_ex(&tps, side, 0, 1, tp_open)
            } else {
                match self.cfg.tp_schedule {
                    TpSchedule::AllRunners => last,
                    _ => next.or(last),
                }
            };
            let newtp = match newtp {
                Some(v) => v,
                None => continue,
            };
            let newtp = if !keep_final && next.is_none() && !self.cfg.tp_freeze_after_ladder {
                cur_tp.unwrap() + side.sign() * self.cfg.tp_open_offset
            } else {
                newtp
            };
            if tp_is_valid(side, newtp, &q, b.stops_level()) {
                self.try_modify(b, t, sl, Some(newtp), _ts);
            }
        }
    }

    fn handle_risk_free<B: Broker>(&mut self, b: &mut B, id: u32, level: Option<Px>, ts: Ts) {
        if self.basket_exit_pending(id) {
            return;
        }
        use RiskFreeMode::*;
        if self.cfg.risk_free_mode == Ignore {
            self.basket_note(id, ts, "RISK FREE zignorowany (konfiguracja)".into());
            return;
        }

        let (side, lo, hi, tps) = match self.basket(id) {
            Some(bk) => (bk.side, bk.zone_lo, bk.zone_hi, bk.tps.clone()),
            None => return,
        };

        if self.cfg.pending_cancel_on_riskfree && !self.keep_explicit_pending(id) {
            let n = self.cancel_pendings(b, id);
            if n > 0 {
                self.basket_note(
                    id,
                    ts,
                    format!("RISK FREE — skasowano {n} wiszących limitów"),
                );
            }
        }

        let live: Vec<Ticket> = self
            .basket(id)
            .map(|bk| {
                bk.tickets
                    .iter()
                    .copied()
                    .filter(|t| b.find_position(*t).is_some())
                    .collect()
            })
            .unwrap_or_default();
        if live.is_empty() {
            self.basket_note(id, ts, "RISK FREE bez otwartych pozycji".into());
            return;
        }

        let q = b.quote();
        let reference = level.unwrap_or_else(|| side.better_edge(lo, hi));
        let keep_n = self.cfg.risk_free_runners.max(1) as usize;

        let mut order = live.clone();
        match self.cfg.risk_free_mode {
            CloseAllKeepNearest => order.sort_by(|a, c| {
                let da = b
                    .find_position(*a)
                    .map(|p| (p.open_price - reference).abs())
                    .unwrap_or(f64::MAX);
                let dc = b
                    .find_position(*c)
                    .map(|p| (p.open_price - reference).abs())
                    .unwrap_or(f64::MAX);
                da.partial_cmp(&dc).unwrap()
            }),
            CloseAllKeepBest => order.sort_by(|a, c| {
                let pa = b.find_position(*a).map(|p| p.profit_usd(&q)).unwrap_or(0.0);
                let pc = b.find_position(*c).map(|p| p.profit_usd(&q)).unwrap_or(0.0);
                pc.partial_cmp(&pa).unwrap()
            }),
            _ => {}
        }

        let keepers: Vec<Ticket> = match self.cfg.risk_free_mode {
            CloseAllKeepNearest | CloseAllKeepBest => order.iter().copied().take(keep_n).collect(),
            MoveSlToBeOnly => live.clone(),
            CloseProfitableOnly | CloseEverything => Vec::new(),
            Ignore => return,
        };

        let mut closed = 0usize;
        let mut realized = 0.0;
        for t in &live {
            if keepers.contains(t) {
                continue;
            }
            let should_close = match self.cfg.risk_free_mode {
                CloseProfitableOnly => b
                    .find_position(*t)
                    .map(|p| p.profit_usd(&q) > 0.0)
                    .unwrap_or(false),
                MoveSlToBeOnly => false,
                _ => true,
            };
            if should_close {
                cien::z(cakt::A_ZYCIE_POZ, *t, czr::Z_RISK_FREE_ZAMKNIJ, 0);
                if let Ok(p) = b.close_position(*t, CloseReason::RiskFree) {
                    realized += p;
                    closed += 1;
                }
            }
        }

        let mut armed = 0usize;
        let be_targets: Vec<Ticket> = if self.cfg.risk_free_mode == MoveSlToBeOnly {
            live.clone()
        } else {
            keepers.clone()
        };
        for t in be_targets {
            let (open, cur_sl, cur_tp) = match b.find_position(t) {
                Some(p) => (p.open_price, p.sl, p.tp),
                None => continue,
            };
            let be = open + side.sign() * self.cfg.be_offset;
            let newtp = match self.cfg.risk_free_runner_target {
                RiskFreeRunnerTarget::KeepTp => cur_tp,
                RiskFreeRunnerTarget::LastTp => tps.last().copied(),
                RiskFreeRunnerTarget::NoTpTrailOnly => None,
                RiskFreeRunnerTarget::NextTp => {
                    let st = self.basket(id).map(|x| x.tp_stage).unwrap_or(0);
                    tps.get(st).copied().or_else(|| tps.last().copied())
                }
            };
            let prog_ok = self.cfg.risk_free_be_min_profit <= 0.0
                || b.find_position(t)
                    .map(|p| p.profit_pts(&q) >= self.cfg.risk_free_be_min_profit)
                    .unwrap_or(false);
            let loosens_stop =
                self.cfg.be_never_loosen && cur_sl.map(|s| side.better(be, s)).unwrap_or(false);
            if !loosens_stop && prog_ok && sl_is_valid(side, be, &q, b.stops_level()) {
                if self.try_modify(b, t, Some(be), newtp, ts) {
                    armed += 1;
                }
            } else if newtp != cur_tp {
                let sl_teraz = b.find_position(t).and_then(|p| p.sl);
                self.try_modify(b, t, sl_teraz, newtp, ts);
            }
            if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
                p.is_runner = newtp.is_none();
            }
        }

        let zabezpieczony = self.cfg.risk_free_mode != CloseProfitableOnly;
        self.book_command_profit_legacy(id, realized);
        if let Some(bk) = self.basket_mut(id) {
            bk.state = BasketState::RiskFree;
            if zabezpieczony {
                bk.secured = true;
                bk.secured_ts = ts;
            }
        }
        self.basket_note(
            id,
            ts,
            format!(
                "RISK FREE @ {reference:.2} ({:?}) · zamknięto {closed} ({realized:+.2} $) · SL na BE: {armed}",
                self.cfg.risk_free_mode
            ),
        );
    }

    #[inline]
    fn bramka_dnia_czynna(&self) -> bool {
        let e = self.stats.day_start_equity;
        if self.cfg.day_gate_od_salda > 0.0 && e < self.cfg.day_gate_od_salda {
            return false;
        }
        if self.cfg.day_gate_do_salda > 0.0 && e >= self.cfg.day_gate_do_salda {
            return false;
        }
        true
    }

    fn handle_out_at_entry<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts) {
        if self.basket_exit_pending(id) {
            return;
        }
        use OutAtEntryMode::*;
        if self.cfg.out_at_entry_mode == Ignore {
            self.basket_note(id, ts, "OUT AT ENTRY zignorowany (konfiguracja)".into());
            return;
        }
        if self.cfg.oae_skip_after_riskfree && self.basket(id).map(|bk| bk.secured).unwrap_or(false)
        {
            self.basket_note(
                id,
                ts,
                "OUT AT ENTRY pominięty — koszyk już zabezpieczony (RISK FREE)".into(),
            );
            return;
        }
        if self.cfg.confirmed_exit_retry && self.cfg.out_at_entry_mode == CloseAll {
            self.request_confirmed_exit(b, id, ts, CloseReason::OutAtEntry);
            return;
        }
        let q = b.quote();
        let side = match self.basket(id) {
            Some(bk) => bk.side,
            None => return,
        };
        let live: Vec<Ticket> = self
            .basket(id)
            .map(|x| x.tickets.clone())
            .unwrap_or_default();
        let mut n = 0;
        for t in live {
            let p = match b.find_position(t) {
                Some(p) => p.clone(),
                None => continue,
            };
            let pnl = p.profit_usd(&q);
            let pts = p.profit_pts(&q);
            let act = match self.cfg.out_at_entry_mode {
                CloseAll => true,
                CloseLosersOnly => pnl < 0.0,
                CloseFlatOnly => pts.abs() <= self.cfg.oae_band_pts,
                MoveSlToBe => false,
                Ignore => false,
            };
            if act {
                cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_OUT_AT_ENTRY_ZAMKNIJ, 0);
                if b.close_position(t, CloseReason::OutAtEntry).is_ok() {
                    n += 1;
                }
            } else if self.cfg.out_at_entry_mode == MoveSlToBe {
                let be = p.open_price + side.sign() * self.cfg.be_offset;
                if sl_is_valid(side, be, &q, b.stops_level()) {
                    self.try_modify(b, t, Some(be), p.tp, ts);
                } else {
                    match self.cfg.oae_pod_woda {
                        OaePodWoda::NicNieRob => {}
                        OaePodWoda::Zamknij => {
                            cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_OUT_AT_ENTRY_ZAMKNIJ, 0);
                            if b.close_position(t, CloseReason::OutAtEntry).is_ok() {
                                n += 1;
                            }
                        }
                        OaePodWoda::DociagnijStop => {
                            let stops = b.stops_level();
                            let kres = match side {
                                Side::Buy => q.bid - stops,
                                Side::Sell => q.ask + stops,
                            };
                            let cel = match side {
                                Side::Buy => be.min(kres),
                                Side::Sell => be.max(kres),
                            };
                            let ciasniej = match (p.sl, side) {
                                (Some(stary), Side::Buy) => cel > stary,
                                (Some(stary), Side::Sell) => cel < stary,
                                (None, _) => true,
                            };
                            if ciasniej && sl_is_valid(side, cel, &q, stops) {
                                self.try_modify(b, t, Some(cel), p.tp, ts);
                            }
                        }
                    }
                }
            }
        }
        self.cancel_pendings(b, id);
        if self.cfg.out_at_entry_mode == CloseAll {
            if let Some(bk) = self.basket_mut(id) {
                bk.state = BasketState::Done;
            }
        }
        self.basket_note(
            id,
            ts,
            format!(
                "OUT AT ENTRY ({:?}) — zamknięto {n}",
                self.cfg.out_at_entry_mode
            ),
        );
    }

    fn handle_sl_hit<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts) {
        if self.basket_exit_pending(id) {
            return;
        }
        use SlHitMode::*;
        match self.cfg.sl_hit_mode {
            Ignore => {}
            CancelPendings => {
                let n = self.cancel_pendings(b, id);
                self.basket_note(id, ts, format!("SL HIT z kanału — skasowano {n} limitów"));
            }
            CloseAll => {
                if self.cfg.confirmed_exit_retry {
                    self.request_confirmed_exit(b, id, ts, CloseReason::Sl);
                    return;
                }
                let live: Vec<Ticket> = self
                    .basket(id)
                    .map(|x| x.tickets.clone())
                    .unwrap_or_default();
                for t in live {
                    cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_SL_HIT_ZAMKNIJ, 0);
                    let _ = b.close_position(t, CloseReason::Sl);
                }
                self.cancel_pendings(b, id);
                if let Some(bk) = self.basket_mut(id) {
                    bk.state = BasketState::Done;
                }
                self.basket_note(id, ts, "SL HIT z kanału — zamknięto koszyk".into());
            }
            VerifyByPrice => {
                let q = b.quote();
                let (side, sl) = match self.basket(id) {
                    Some(bk) => (bk.side, bk.sl),
                    None => return,
                };
                if let Some(slv) = sl {
                    let dist = (q.mid() - slv) * side.sign();
                    if dist > self.cfg.sl_hit_verify_tol {
                        self.basket_note(
                            id,
                            ts,
                            format!("SL HIT odrzucony — rynek {dist:.2} $ po stronie zysku"),
                        );
                        return;
                    }
                }
                let n = self.cancel_pendings(b, id);
                self.basket_note(
                    id,
                    ts,
                    format!("SL HIT zweryfikowany — skasowano {n} limitów"),
                );
            }
        }
    }

    fn move_basket_to_be<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts) {
        if self.basket_exit_pending(id) {
            return;
        }
        let q = b.quote();
        let side = match self.basket(id) {
            Some(bk) => bk.side,
            None => return,
        };
        if let Some(bk) = self.basket_mut(id) {
            bk.be_ts = ts;
        }
        let live: Vec<Ticket> = self
            .basket(id)
            .map(|x| x.tickets.clone())
            .unwrap_or_default();
        let mut n = 0;
        for t in live {
            let (open, sl, tp, frozen) = match b.find_position(t) {
                Some(p) => (p.open_price, p.sl, p.tp, p.frozen),
                None => continue,
            };
            if frozen {
                continue;
            }
            let be = open + side.sign() * self.cfg.be_offset;
            let luzuje = (self.cfg.trail_sr_enabled || self.cfg.be_never_loosen)
                && sl.map(|s| side.better(be, s)).unwrap_or(false);
            if !luzuje
                && sl_is_valid(side, be, &q, b.stops_level())
                && self.try_modify(b, t, Some(be), tp, ts)
            {
                n += 1;
            }
        }
        if n > 0 {
            self.basket_note(id, ts, format!("SL na breakeven: {n} poz."));
        }
    }

    fn sl_polowa_drogi<B: Broker>(&mut self, b: &mut B, id: u32, ts: Ts) {
        let ulamek = if self.cfg.sl_polowa_ulamek > 0.0 {
            self.cfg.sl_polowa_ulamek
        } else {
            0.5
        };
        let q = b.quote();
        let side = match self.basket(id) {
            Some(bk) => bk.side,
            None => return,
        };
        let live: Vec<Ticket> = self
            .basket(id)
            .map(|x| x.tickets.clone())
            .unwrap_or_default();
        let mut n = 0;
        for t in live {
            let (open, tp, frozen, cur) = match b.find_position(t) {
                Some(p) => (p.open_price, p.tp, p.frozen, p.sl),
                None => continue,
            };
            if frozen {
                continue;
            }
            let cena = q.exit(side);
            let ruch = (cena - open) * side.sign();
            if ruch <= 0.0 {
                continue;
            }
            let want = open + side.sign() * ruch * ulamek;
            let lepszy = match cur {
                Some(s) => (want - s) * side.sign() > 0.0,
                None => true,
            };
            if !lepszy {
                continue;
            }
            if sl_is_valid(side, want, &q, b.stops_level())
                && self.try_modify(b, t, Some(want), tp, ts)
            {
                n += 1;
            }
        }
        if n > 0 {
            self.basket_note(
                id,
                ts,
                format!("SL na {:.0} % drogi wejście→cena: {n} poz.", ulamek * 100.0),
            );
        }
    }

    fn set_basket_sl<B: Broker>(&mut self, b: &mut B, id: u32, sl: Px, ts: Ts) {
        let q = b.quote();
        let side = match self.basket(id) {
            Some(bk) => bk.side,
            None => return,
        };
        if let Some(bk) = self.basket_mut(id) {
            bk.sl = Some(sl);
        }
        let live: Vec<Ticket> = self
            .basket(id)
            .map(|x| x.tickets.clone())
            .unwrap_or_default();
        for t in live {
            let (tp, frozen) = match b.find_position(t) {
                Some(p) => (p.tp, p.frozen),
                None => continue,
            };
            if !frozen && sl_is_valid(side, sl, &q, b.stops_level()) {
                self.try_modify(b, t, Some(sl), tp, ts);
            }
        }
        if self.cfg.sl_edit_reaches_pendings {
            if let Some(bk) = self.basket_mut(id) {
                for gl in bk.levels.iter_mut() {
                    gl.sl = Some(sl);
                }
            }
            let broker_sl = self.broker_sl(Some(sl), side);
            let do_zmiany: Vec<(Ticket, Px, Option<Px>)> = b
                .pendings()
                .iter()
                .filter(|o| o.basket == Some(id))
                .map(|o| (o.ticket, o.price, o.tp))
                .collect();
            let mut zmienione = 0usize;
            for (t, price, tp) in do_zmiany {
                cien::z(cakt::A_KSZTALT_ZLEC, t, czr::Z_SET_BASKET_SL_PENDING, 0);
                if b.modify_pending(t, price, broker_sl, tp).is_ok() {
                    zmienione += 1;
                }
            }
            if zmienione > 0 {
                self.basket_note(
                    id,
                    ts,
                    format!("SL przepisany na {zmienione} zleceń oczekujących"),
                );
            }
        }
        self.basket_note(id, ts, format!("SL koszyka → {sl:.2}"));
    }

    fn zastosuj_spp_be<B: Broker>(&mut self, b: &mut B, id: u32, poziom: Px, ts: Ts) {
        let q = b.quote();
        let side = match self.basket(id) {
            Some(bk) => bk.side,
            None => return,
        };
        let tryb = self.cfg.spp_sl_mode;
        let cel = poziom - side.sign() * self.cfg.spp_sl_pad;
        let tylko_runnery = matches!(
            tryb,
            SppSlMode::RunnersOnly | SppSlMode::RunnersOnlyIfBetter
        );
        let tylko_bankujace = tryb == SppSlMode::BankersOnly;
        let tylko_lepszy = matches!(
            tryb,
            SppSlMode::OnlyIfBetter | SppSlMode::RunnersOnlyIfBetter
        );

        let live: Vec<Ticket> = self
            .basket(id)
            .map(|x| x.tickets.clone())
            .unwrap_or_default();
        let mut n = 0usize;
        for t in live {
            let (tp, cur_sl, frozen, runner) = match b.find_position(t) {
                Some(p) => (p.tp, p.sl, p.frozen, p.is_runner),
                None => continue,
            };
            if frozen || (tylko_runnery && !runner) || (tylko_bankujace && runner) {
                continue;
            }
            if tylko_lepszy && !cur_sl.map(|s| side.better(s, cel)).unwrap_or(true) {
                continue;
            }
            if sl_is_valid(side, cel, &q, b.stops_level())
                && self.try_modify(b, t, Some(cel), tp, ts)
            {
                n += 1;
            }
        }
        if !tylko_runnery && !tylko_bankujace && n > 0 {
            if let Some(bk) = self.basket_mut(id) {
                bk.sl = Some(cel);
            }
        }
        if n > 0 {
            self.basket_note(id, ts, format!("SL z komunikatu SPP → {cel:.2} ({n} poz.)"));
        }
    }

    #[track_caller]
    fn cancel_pendings_keep<B: Broker>(&mut self, b: &mut B, id: u32, n: usize) -> usize {
        if self.basket_exit_pending(id) {
            return 0;
        }
        if n == 0 {
            return self.cancel_pendings(b, id);
        }
        let list: Vec<Ticket> = self
            .basket(id)
            .map(|x| x.pendings.clone())
            .unwrap_or_default();
        let Some(bk) = self.baskets.iter().find(|x| x.id == id) else {
            return 0;
        };
        let strona = bk.side;
        let mut moje: Vec<(Ticket, Px)> = b
            .pendings()
            .iter()
            .filter(|o| list.contains(&o.ticket))
            .map(|o| (o.ticket, o.price))
            .collect();
        moje.sort_by(|x, y| match strona {
            Side::Buy => y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal),
            Side::Sell => x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal),
        });
        if moje.len() <= n {
            return 0;
        }
        let do_kasacji: Vec<Ticket> = moje[n..].iter().map(|x| x.0).collect();
        let poziomy: Vec<i32> = b
            .pendings()
            .iter()
            .filter(|o| do_kasacji.contains(&o.ticket))
            .map(|o| o.level)
            .collect();
        let mut k = 0;
        let mut confirmed = Vec::new();
        for t in &do_kasacji {
            if cien::czynny() {
                cien::z(
                    cakt::A_ZYCIE_ZLEC,
                    *t,
                    czr::L_CANCEL_KEEP,
                    std::panic::Location::caller().line(),
                );
            }
            if b.cancel_pending(*t).is_ok() {
                k += 1;
                confirmed.push(*t);
            }
        }
        let do_kasacji = if self.cfg.confirmed_exit_retry {
            confirmed
        } else {
            do_kasacji
        };
        let remaining_levels: Vec<i32> = if self.cfg.confirmed_exit_retry {
            b.pendings()
                .iter()
                .filter(|o| o.basket == Some(id))
                .map(|o| o.level)
                .collect()
        } else {
            Vec::new()
        };
        if let Some(bk) = self.basket_mut(id) {
            bk.pendings.retain(|t| !do_kasacji.contains(t));
            for lv in poziomy {
                if let Some(g) = bk.levels.iter_mut().find(|g| g.level == lv) {
                    if g.fill_ts == 0 && !remaining_levels.contains(&lv) {
                        g.cancelled = true;
                    }
                }
            }
        }
        k
    }

    #[track_caller]
    fn cancel_pendings<B: Broker>(&mut self, b: &mut B, id: u32) -> usize {
        if self.basket_exit_pending(id) {
            return 0;
        }
        self.cancel_pendings_now(b, id)
    }

    #[track_caller]
    fn cancel_pendings_now<B: Broker>(&mut self, b: &mut B, id: u32) -> usize {
        if self.cfg.confirmed_exit_retry {
            let orders: Vec<(Ticket, i32)> = b
                .pendings()
                .iter()
                .filter(|o| o.basket == Some(id) && !o.frozen)
                .map(|o| (o.ticket, o.level))
                .collect();
            let mut cancelled = 0;
            let mut levels = Vec::new();
            for (ticket, level) in orders {
                if b.cancel_pending(ticket).is_ok() {
                    cancelled += 1;
                    levels.push(level);
                }
            }
            let remaining: Vec<Ticket> = b
                .pendings()
                .iter()
                .filter(|o| o.basket == Some(id))
                .map(|o| o.ticket)
                .collect();
            if let Some(bk) = self.basket_mut(id) {
                bk.pendings = remaining;
                for g in &mut bk.levels {
                    if g.fill_ts == 0 && levels.contains(&g.level) {
                        g.cancelled = true;
                    }
                }
            }
            return cancelled;
        }
        let list: Vec<Ticket> = self
            .basket(id)
            .map(|x| x.pendings.clone())
            .unwrap_or_default();
        let poziomy: Vec<i32> = b
            .pendings()
            .iter()
            .filter(|o| list.contains(&o.ticket))
            .map(|o| o.level)
            .collect();
        let mut n = 0;
        for t in list {
            if cien::czynny() {
                cien::z(
                    cakt::A_ZYCIE_ZLEC,
                    t,
                    czr::L_CANCEL_PENDINGS,
                    std::panic::Location::caller().line(),
                );
            }
            if b.cancel_pending(t).is_ok() {
                n += 1;
            }
        }
        if let Some(bk) = self.basket_mut(id) {
            bk.pendings.clear();
            for lv in poziomy {
                if let Some(g) = bk.levels.iter_mut().find(|g| g.level == lv) {
                    if g.fill_ts == 0 {
                        g.cancelled = true;
                    }
                }
            }
        }
        n
    }

    #[track_caller]
    pub fn close_everything<B: Broker>(&mut self, b: &mut B, ts: Ts, reason: CloseReason) {
        self.queued_exits.clear();
        if self.cfg.confirmed_exit_retry {
            let ids: Vec<u32> = self.baskets.iter().map(|x| x.id).collect();
            for id in ids {
                self.request_confirmed_exit(b, id, ts, reason);
            }
            let unowned = b
                .positions()
                .iter()
                .filter(|p| !self.baskets.iter().any(|x| p.basket == Some(x.id)))
                .count();
            let unowned_pending = b
                .pendings()
                .iter()
                .filter(|p| !self.baskets.iter().any(|x| p.basket == Some(x.id)))
                .count();
            if unowned + unowned_pending > 0 {
                self.log(ts, 2, format!("zamknięcie zbiorcze: pominięto {unowned} pozycji i {unowned_pending} zleceń bez własnego koszyka; wymagają ręcznej rekoncyliacji"));
            }
            return;
        }
        let spisuj = self.journal.wants(EventLevel::Warn);
        let q = b.quote();
        let przed: Vec<Position> = if spisuj {
            b.positions().to_vec()
        } else {
            Vec::new()
        };

        let tickets: Vec<Ticket> = b.positions().iter().map(|p| p.ticket).collect();
        let mut nogi: Vec<ClosedLeg> = Vec::new();
        let mut suma = 0.0;
        for t in tickets {
            if cien::czynny() {
                cien::z(
                    cakt::A_ZYCIE_POZ,
                    t,
                    czr::L_CLOSE_EVERYTHING,
                    std::panic::Location::caller().line(),
                );
            }
            let zysk = b.close_position(t, reason);
            if !spisuj {
                continue;
            }
            let Some(p) = przed.iter().find(|x| x.ticket == t) else {
                continue;
            };
            let profit = zysk.unwrap_or(0.0);
            suma += profit;
            nogi.push(ClosedLeg {
                ticket: t,
                basket_id: p.basket,
                side: format!("{:?}", p.side),
                volume: journal::r4(p.volume),
                open_price: journal::r4(p.open_price),
                close_price: journal::r4(q.exit(p.side)),
                profit: journal::r4(profit),
            });
        }
        let pend: Vec<Ticket> = b.pendings().iter().map(|p| p.ticket).collect();
        let skasowane = pend.len();
        for t in pend {
            if cien::czynny() {
                cien::z(
                    cakt::A_ZYCIE_ZLEC,
                    t,
                    czr::L_CLOSE_EVERYTHING,
                    std::panic::Location::caller().line(),
                );
            }
            let _ = b.cancel_pending(t);
        }
        for bk in self.baskets.iter_mut() {
            bk.state = BasketState::Done;
            bk.pendings.clear();
        }

        if spisuj && (!nogi.is_empty() || skasowane > 0) {
            let vol: f64 = nogi.iter().map(|l| l.volume).sum();
            let n = nogi.len();
            let snap = self.jsnap(b);
            self.journal.push(
                Ev::new(ts, EventLevel::Error, EventCategory::Risk, EventKind::RiskStop)
                    .text(format!(
                        "zamknięcie zbiorcze ({}) · {n} pozycji · {vol:.2} lota · wynik {suma:+.2} $ · skasowano {skasowane} limitów",
                        journal::close_reason_str(reason)
                    ))
                    .reason(match reason {
                        CloseReason::MaxDd => RejectCode::Halted,
                        CloseReason::Manual => RejectCode::Manual,
                        _ => RejectCode::DisabledBySetting,
                    })
                    .market(snap)
                    .legs(nogi)
                    .put("close_reason", journal::close_reason_str(reason))
                    .put("positions", n as u64)
                    .put_f("volume", vol)
                    .put_f("realized", suma)
                    .put("pendings_cancelled", skasowane as u64)
                    .build(),
            );
        }
    }


    pub fn ea_zegar<B: Broker>(&mut self, b: &mut B, ts: Ts) -> bool {
        if !self.cfg.ea_enabled || self.cfg.t100.enabled {
            return false;
        }
        self.ea
            .set_continuation_entry_hold(self.continuation_entry_blocked() || self.rearm_confirmation_pending());
        self.ea.set_profit_budget_anchor((&self.stats).into());
        let pulsed=self.ea.puls(
            &self.cfg,
            &mut self.baskets,
            b,
            ts,
            crate::ea::ZrodloPulsu::Zegar,
        );
        self.stats.day_peak_equity=self.stats.day_peak_equity.max(self.ea.profit_budget_peak());
        pulsed
    }

    pub fn on_tick<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if self.cfg.t100.enabled { self.t100_reconcile(b); }
        self.reconcile_rearm_batches(b);
        self.continuation_observe(b);
        self.deferred_observe(b, q);
        self.refresh_basket_slots();
        self.retry_source_cancellations(b);
        let ts = q.ts;
        cien::puls(ts, conduit_mozg_cien::cien::PULS_TICK);

        if self.cfg.trail_sr_enabled {
            self.sr_na_ticku(q);
        }

        if self.halt_prev_ts > 0 && self.halted.is_some() {
            self.halt_ms += (ts - self.halt_prev_ts).clamp(0, 60_000);
            let m = self.halt_ms / 60_000;
            if m != self.halt_min_zapisane {
                self.halt_min_zapisane = m;
                *self.odrzuty.entry("HamulecMinut".to_string()).or_insert(0) = m as u64;
            }
        }
        self.halt_prev_ts = ts;

        if self.journal.enabled() {
            if self.journal.wants(EventLevel::Info) {
                let nowe: Vec<Position> = b
                    .positions()
                    .iter()
                    .filter(|p| !self.journal.knows(p.ticket))
                    .cloned()
                    .collect();
                if !nowe.is_empty() {
                    let snap = self.jsnap(b);
                    let msg_of: HashMap<u32, i64> =
                        self.baskets.iter().map(|x| (x.id, x.msg_id)).collect();
                    for p in &nowe {
                        let mut ev = Ev::new(
                            ts,
                            EventLevel::Info,
                            EventCategory::Trade,
                            EventKind::PositionOpened,
                        )
                        .text(format!(
                            "otwarto {:?} {:.2} lota po {:.2}",
                            p.side, p.volume, p.open_price
                        ))
                        .ticket(p.ticket)
                        .basket_opt(p.basket)
                        .market(snap)
                        .put("side", format!("{:?}", p.side))
                        .put_f("volume", p.volume)
                        .put_f("open_price", p.open_price)
                        .put("sl", p.sl.map(journal::r4))
                        .put("tp", p.tp.map(journal::r4))
                        .put("vsl", p.vsl.map(journal::r4))
                        .put("level", p.level as i64)
                        .put("open_ts", p.open_ts);
                        if let Some(mid) = p.basket.and_then(|bid| msg_of.get(&bid).copied()) {
                            ev = ev.msg(mid).signal(journal::signal_id(mid, "entry"));
                        }
                        self.journal.push(ev.build());
                    }
                }
            }
            let poz: Vec<Position> = b.positions().to_vec();
            self.journal.track(&poz, q);
        }

        if let Some(reason) = self.cost_entry_blocked(b).map(str::to_owned) {
            self.latch_cost_fault(b, ts, reason);
        }
        let closed = b.drain_closed();
        let closed = if self.cfg.closed_profit_net_costs {
            let mut accepted = Vec::with_capacity(closed.len());
            for trade in closed {
                if !self.cfg.basket_realized_broker_only {
                    self.cost_quarantine.push(trade);
                    continue;
                }
                match trade.canonical_net() {
                    Ok(_) => accepted.push(trade),
                    Err(error) => {
                        self.latch_cost_fault(
                            b,
                            ts,
                            format!("invalid closed receipt #{}: {error}", trade.ticket),
                        );
                        self.cost_quarantine.push(trade);
                    }
                }
            }
            accepted
        } else {
            closed
        };

        if self.journal.wants(EventLevel::Info) && !closed.is_empty() {
            let snap = self.jsnap(b);
            let msg_of: HashMap<u32, i64> = self.baskets.iter().map(|x| (x.id, x.msg_id)).collect();
            let zywe: std::collections::HashSet<Ticket> =
                b.positions().iter().map(|p| p.ticket).collect();
            for c in &closed {
                let exc = if zywe.contains(&c.ticket) {
                    self.journal.peek_excursion(c.ticket, c.volume)
                } else {
                    self.journal.take_excursion(c.ticket, c.volume)
                };
                let Ok(detail) = CloseDetail::new(c, exc) else {
                    self.log(
                        ts,
                        2,
                        format!("COST JOURNAL HOLD: invalid receipt #{}", c.ticket),
                    );
                    continue;
                };
                let poziom = if detail.net >= 0.0 {
                    EventLevel::Ok
                } else {
                    EventLevel::Warn
                };
                let mut ev = Ev::new(
                    c.close_ts,
                    poziom,
                    EventCategory::Trade,
                    EventKind::PositionClosed,
                )
                .text(format!(
                    "zamknięto #{} · {} · {:+.2} $",
                    c.ticket,
                    journal::close_reason_str(c.reason),
                    detail.net
                ))
                .ticket(c.ticket)
                .basket_opt(c.basket)
                .market(snap)
                .close(detail)
                .put("partial", zywe.contains(&c.ticket));
                if let Some(mid) = c.basket.and_then(|bid| msg_of.get(&bid).copied()) {
                    ev = ev.msg(mid).signal(journal::signal_id(mid, "entry"));
                }
                self.journal.push(ev.build());
            }
        }

        for c in &closed {
            let profit = match c.strategy_realized_profit(self.cfg.closed_profit_net_costs) {
                Ok(value) => value,
                Err(error) => {
                    self.latch_cost_fault(b, ts, format!("STRATEGY P/L: unverified closed tranche ({error:?})"));
                    self.cost_quarantine.push(c.clone());
                    continue;
                }
            };
            self.stats.trades += 1;
            if profit > 0.0 {
                self.stats.wins += 1;
                self.stats.gross_win += profit;
            } else {
                self.stats.losses += 1;
                self.stats.gross_loss += -profit;
            }
            self.stats.realized_today += profit;
            self.closed_today.push(profit);
            self.obs.na_zamknieciu(c.close_ts, profit);
            if self.cfg.t100.enabled {
                self.t100_closed(c);
            }
            if let Some(bid) = c.basket {
                if let Some(bk) = self.basket_mut(bid) {
                    bk.realized += profit;
                    bk.tickets.retain(|t| *t != c.ticket);
                }
            }
        }
        for p in b.positions() {
            let Some(bid) = p.basket else { continue };
            if let Some(bk) = self.baskets.iter_mut().find(|x| x.id == bid) {
                if !bk.tickets.contains(&p.ticket) {
                    bk.tickets.push(p.ticket);
                }
                if let Some(g) = bk.levels.iter_mut().find(|g| g.level == p.level) {
                    if g.fill_ts == 0 {
                        g.fill_ts = p.open_ts;
                        g.fill_px = p.open_price;
                        g.filled = true;
                    }
                }
                bk.pendings.retain(|t| *t != p.ticket);
                bk.had_positions = true;
                if bk.state == BasketState::Armed {
                    bk.state = BasketState::Working;
                }
            }
        }
        if self.cfg.be_covers_late_fills && !self.cfg.t100.enabled {
            let mut do_krycia: Vec<(Ticket, Px, Option<Px>)> = Vec::new();
            for p in b.positions() {
                let Some(bid) = p.basket else { continue };
                if p.frozen {
                    continue;
                }
                let Some(bk) = self.baskets.iter().find(|x| x.id == bid) else {
                    continue;
                };
                if (self.cfg.confirmed_exit_retry && bk.pending_exit.is_some())
                    || bk.be_ts == 0
                    || p.open_ts < bk.be_ts
                {
                    continue;
                }
                let be = p.open_price + bk.side.sign() * self.cfg.be_offset;
                let warto = p.sl.map(|cur| bk.side.better(cur, be)).unwrap_or(true);
                if warto && sl_is_valid(bk.side, be, q, b.stops_level()) {
                    do_krycia.push((p.ticket, be, p.tp));
                }
            }
            let mut ile = 0usize;
            for (t, be, tp) in do_krycia {
                if self.try_modify(b, t, Some(be), tp, ts) {
                    ile += 1;
                }
            }
            if ile > 0 {
                self.log(
                    ts,
                    1,
                    format!("BE kryje {ile} pozycji wypełnionych PO komendzie kanału"),
                );
            }
        }

        {
            let live_pend: std::collections::HashSet<Ticket> =
                b.pendings().iter().map(|o| o.ticket).collect();
            let live_pos: std::collections::HashSet<Ticket> =
                b.positions().iter().map(|p| p.ticket).collect();
            for bk in self.baskets.iter_mut() {
                bk.pendings.retain(|t| live_pend.contains(t));
                bk.tickets.retain(|t| live_pos.contains(t));
            }
        }

        self.retry_confirmed_exits(b, ts);

        if self.cfg.tp_stage_from_broker_fill && !self.cfg.t100.enabled {
            let hits: Vec<(u32, Px)> = closed
                .iter()
                .filter(|c| c.reason == CloseReason::Tp)
                .filter_map(|c| c.basket.map(|b| (b, c.close_price)))
                .collect();
            for (bid, px) in hits {
                let (side, tps, stage) = match self.basket(bid) {
                    Some(bk) => (bk.side, bk.tps.clone(), bk.tp_stage),
                    None => continue,
                };
                let reached = tps
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| match side {
                        Side::Buy => px >= **t - 1e-9,
                        Side::Sell => px <= **t + 1e-9,
                    })
                    .map(|(i, _)| i + 1)
                    .max()
                    .unwrap_or(0);
                if reached > stage {
                    self.handle_tp_hit_z_pozycja(b, bid, Some(reached), ts, "realizacja brokera");
                }
            }
        }

        if !closed.is_empty() {
            let last_neg = closed.iter().map(|c| c.profit).sum::<f64>() < 0.0;
            if last_neg {
                self.loss_streak += 1;
                if self.cfg.streak_pause_n > 0 && self.loss_streak >= self.cfg.streak_pause_n {
                    self.paused_until = ts + (self.cfg.streak_pause_min * 60_000.0) as i64;
                    self.loss_streak = 0;
                }
            } else {
                self.loss_streak = 0;
            }
        }

        let acc = b.account();
        self.stats.balance = acc.balance;
        self.stats.credit = acc.credit;
        self.stats.equity = acc.equity;

        if self.cfg.odlicz_kredyt
            && self.cfg.kredyt_reczny > 0.0
            && (self.cfg.kredyt_reczny - acc.credit).abs() > 0.01
            && self.journal.wants(EventLevel::Warn)
        {
            let dzien = day_of(ts, self.cfg.session_offset());
            if dzien != self.kredyt_rozjazd_dzien {
                self.kredyt_rozjazd_dzien = dzien;
                self.journal.push(
                    Ev::new(
                        ts,
                        EventLevel::Warn,
                        EventCategory::Account,
                        EventKind::Note,
                    )
                    .text(format!(
                        "ROZJAZD KREDYTU · ręcznie {:.2} $ · terminal {:.2} $ · \
                             lot liczony od {:.2} $ (kwota ręczna wygrywa). \
                             Jeśli broker zdjął bonus, wyzeruj pole ręczne — zero znaczy AUTOMAT.",
                        self.cfg.kredyt_reczny,
                        acc.credit,
                        self.cfg
                            .podstawa_lota_z_konta(acc.balance, acc.equity, acc.credit)
                    ))
                    .put_f("kredyt_reczny", self.cfg.kredyt_reczny)
                    .put_f("kredyt_terminal", acc.credit)
                    .put_f("balance", acc.balance)
                    .build(),
                );
            }
        }

        let day = day_of(ts, self.cfg.session_offset());
        if day != self.stats.day {
            if self.journal.wants(EventLevel::Info) && self.stats.day != i64::MIN {
                self.journal.push(
                    Ev::new(ts, EventLevel::Info, EventCategory::Account, EventKind::DayRollover)
                        .text(format!(
                            "nowa doba handlowa · saldo {:.2} $ · equity {:.2} $ · poprzedni dzień {:+.2} $",
                            acc.balance, acc.equity, self.stats.realized_today
                        ))
                        .put_f("balance", acc.balance)
                        .put_f("equity", acc.equity)
                        .put_f("prev_day_realized", self.stats.realized_today)
                        .put_f("prev_day_max_dd", self.stats.day_max_dd)
                        .put("prev_day", journal::session_day_str(self.stats.day * 86_400_000, 0))
                        .build(),
                );
            }
            self.stats.day = day;
            self.stats.day_start_equity = acc.equity;
            self.stats.day_peak_equity = acc.equity;
            self.stats.day_max_dd = 0.0;
            self.stats.realized_today = 0.0;
            self.closed_today.clear();
            self.slhit_dnia = 0;
            self.slhit_pauza_do = i64::MIN;
            if !matches!(self.cfg.dd_guard_scope, DdGuardScope::Lifetime) {
                if self
                    .halted
                    .as_deref()
                    .map(|r| r.starts_with("MAX DRAWDOWN"))
                    .unwrap_or(false)
                {
                    self.halted = None;
                    self.log(ts, 1, "nowa doba — blokada obsunięcia zdjęta".to_string());
                }
            }
        }
        if acc.equity > self.stats.peak_equity {
            self.stats.peak_equity = acc.equity;
        }
        if acc.equity > self.stats.day_peak_equity {
            self.stats.day_peak_equity = acc.equity;
        }
        let dd = self.stats.peak_equity - acc.equity;
        if dd > self.stats.max_dd_abs {
            self.stats.max_dd_abs = dd;
            self.stats.max_dd_pct = dd / self.stats.peak_equity.max(1.0) * 100.0;
        }
        let ddd = self.stats.day_peak_equity - acc.equity;
        if ddd > self.stats.day_max_dd {
            self.stats.day_max_dd = ddd;
        }

        if self
            .price_hist
            .last()
            .map(|(t, _)| ts - *t >= 3_600_000)
            .unwrap_or(true)
        {
            self.price_hist.push((ts, q.mid()));
            if self.price_hist.len() > 24 * 30 {
                self.price_hist.remove(0);
            }
        }

        if self
            .vol_hist
            .last()
            .map(|(t, _)| ts - *t >= 5_000)
            .unwrap_or(true)
        {
            self.vol_hist.push((ts, q.mid()));
            let horizon = ts
                - ((self
                    .cfg
                    .vol_window_min
                    .max(self.cfg.rev_exit_window_min)
                    .max(60.0)
                    * 2.0)
                    * 60_000.0) as i64;
            if self.vol_hist.len() > 64 {
                self.vol_hist.retain(|(t, _)| *t >= horizon);
            }
        }

        self.aktualizuj_mnoznik_zmiennosci(q);

        self.obs.na_ticku(q, &self.baskets, b.positions());

        self.update_basket_peaks(b, q);

        if self.cfg.ea_enabled && !self.cfg.t100.enabled {
            self.ea
                .set_continuation_entry_hold(self.continuation_entry_blocked() || self.rearm_confirmation_pending());
            self.ea.set_profit_budget_anchor((&self.stats).into());
            self.ea.puls(
                &self.cfg,
                &mut self.baskets,
                b,
                ts,
                crate::ea::ZrodloPulsu::Tick,
            );
            self.stats.day_peak_equity=self.stats.day_peak_equity.max(self.ea.profit_budget_peak());
        }

        self.check_guards(b, q);

        self.retry_stops(b, ts);

        if self.cfg.t100.enabled {
            self.t100_tick(b, q);
            return;
        }

        let reguly_wlaczone = !(self.cfg.ai_enabled && self.cfg.ai_replaces_management);
        if reguly_wlaczone {
            self.riskfree_pass(b, q);
        }

        if reguly_wlaczone {
            self.manage_positions(b, q);
        }

        let price_source_enabled = matches!(
            self.cfg.tp_source,
            TpSource::PriceOnly
                | TpSource::Either
                | TpSource::SignalConfirmedByPrice
                | TpSource::PriceFirstSignalWindow
        );
        let tp_front_run = self.cfg.tp_price_front_run_usd.max(0.0);
        if price_source_enabled || tp_front_run > 0.0 {
            let slots = self.price_tp_slots();
            for (slot, id) in slots {
                let bk = self
                    .baskets
                    .get(slot)
                    .filter(|bk| bk.id == id)
                    .or_else(|| self.basket(id));
                let (side, next_tp, stage, ma_poz, armed, lo, hi, dotknieta) = match bk {
                    Some(bk) => {
                        let stage = if bk.ma_pozycje() {
                            bk.tp_stage
                        } else {
                            bk.etap_obserwowany()
                        };
                        (
                            bk.side,
                            bk.tps.get(stage).copied(),
                            stage,
                            bk.ma_pozycje(),
                            bk.drop_armed,
                            bk.zone_lo,
                            bk.zone_hi,
                            bk.zone_touched,
                        )
                    }
                    None => continue,
                };
                if !dotknieta {
                    let w_strefie = match side {
                        Side::Buy => q.ask <= hi + 1e-9,
                        Side::Sell => q.bid >= lo - 1e-9,
                    };
                    if w_strefie {
                        if let Some(bk) = self.basket_mut(id) {
                            bk.zone_touched = true;
                        }
                    }
                }
                if let Some(next) = next_tp {
                    if !ma_poz && !price_source_enabled {
                        continue;
                    }
                    let offset = if ma_poz { tp_front_run } else { 0.0 };
                    let reached = match side {
                        Side::Buy => q.bid >= next - offset,
                        Side::Sell => q.ask <= next + offset,
                    };
                    if !reached {
                        if !armed {
                            if let Some(bk) = self.basket_mut(id) {
                                bk.drop_armed = true;
                            }
                        }
                        continue;
                    }
                    if self.cfg.pending_drop_require_zone_touch
                        && !ma_poz
                        && !self.basket(id).map(|x| x.zone_touched).unwrap_or(false)
                    {
                        continue;
                    }
                    if ma_poz {
                        let src = if offset > 0.0 {
                            "cena MT5 front-run"
                        } else {
                            "cena"
                        };
                        self.handle_tp_hit(b, id, Some(stage + 1), ts, src);
                    } else if self.cfg.pending_drop_on_target
                        && (armed || !self.cfg.pending_drop_arm)
                    {
                        self.drop_grid_on_target(b, id, stage + 1, next, ts);
                    }
                }
            }
        }

        if self.cfg.pending_ttl_h > 0.0 {
            let max_age = (self.cfg.pending_ttl_h * 3_600_000.0) as i64;
            let ages: HashMap<u32, Ts> =
                self.baskets.iter().map(|x| (x.id, x.created_ts)).collect();
            let from_basket = self.cfg.pending_ttl_from_basket;
            let expired: Vec<Ticket> = b
                .pendings()
                .iter()
                .filter(|o| {
                    !o.basket
                        .map(|id| self.basket_exit_pending(id))
                        .unwrap_or(false)
                })
                .filter(|o| !o.basket.is_some_and(|id| self.keep_explicit_pending(id)))
                .filter(|o| {
                    let born = if from_basket {
                        o.basket
                            .and_then(|bid| ages.get(&bid).copied())
                            .unwrap_or(o.placed_ts)
                    } else {
                        o.placed_ts
                    };
                    ts - born > max_age
                })
                .map(|o| o.ticket)
                .collect();
            let poziomy: Vec<(Option<u32>, i32)> = expired
                .iter()
                .filter_map(|t| b.pendings().iter().find(|o| o.ticket == *t))
                .map(|o| (o.basket, o.level))
                .collect();
            for t in expired {
                cien::z(cakt::A_ZYCIE_ZLEC, t, czr::Z_PENDING_TTL, 0);
                let _ = b.cancel_pending(t);
                for bk in self.baskets.iter_mut() {
                    bk.pendings.retain(|x| *x != t);
                }
            }
            for (bid, lvl) in poziomy {
                if let Some(id) = bid {
                    if let Some(bk) = self.baskets.iter_mut().find(|x| x.id == id) {
                        if let Some(g) = bk.levels.iter_mut().find(|g| g.level == lvl) {
                            g.cancelled = true;
                        }
                    }
                }
            }
        }

        let with_pos: Vec<u32> = b.positions().iter().filter_map(|p| p.basket).collect();
        for bk in self.baskets.iter_mut() {
            if !bk.tickets.is_empty() || with_pos.contains(&bk.id) {
                bk.had_positions = true;
            }
        }

        if self.cfg.virtual_sl_all {
            let sl_of: HashMap<u32, Option<Px>> =
                self.baskets.iter().map(|x| (x.id, x.sl)).collect();
            for p in b.positions_mut().iter_mut() {
                if p.vsl.is_none() {
                    if let Some(v) = p.basket.and_then(|bid| sl_of.get(&bid).copied()).flatten() {
                        p.vsl = Some(v);
                    }
                }
            }
        }

        self.expire_stale_baskets(b, ts);

        self.dokoncz_odroczone_kasowanie(b, ts);
        self.expire_old_baskets(b, ts);
        self.reject_fast_filled_baskets(b, ts);
        self.enforce_position_limit(b, ts);
        self.zone_exit_adverse_sweep(b, q);
        self.fast_addon_sweep(b, q);

        self.resize_pendings(b, ts);
        self.relot_pendings(b, ts);

        self.redukuj_ekspozycje(b, ts);

        self.sesja_limity_pass(b, q);
        self.rezim_limity_pass(b, q);
        self.rearm_pass(b, q);

        self.market_ladder_pass(b, q);

        self.reentry_pass(b, q);

        self.rev_exit_sweep(b, q);

        let mut domkniete: Vec<u32> = Vec::new();
        for bk in self.baskets.iter_mut() {
            let czeka_na_rearm = self.cfg.rearm_grid_on_return
                && self.cfg.rearm_keep_empty_alive
                && bk.had_positions;
            if bk.alive()
                && !(self.cfg.confirmed_exit_retry && bk.pending_exit.is_some())
                && !czeka_na_rearm
                && bk.tickets.is_empty()
                && bk.pendings.is_empty()
                && ts - bk.created_ts > 60_000
            {
                bk.state = BasketState::Done;
                domkniete.push(bk.id);
            }
        }
        for id in domkniete {
            self.obs.zapomnij(id);
        }
        if self.baskets.len() > 500 {
            let cutoff = ts - 7 * 86_400_000;
            self.baskets.retain(|x| {
                x.alive()
                    || x.created_ts > cutoff
                    || (self.cfg.confirmed_exit_retry && x.pending_exit.is_some())
            });
        }
    }

    fn check_guards<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if self.halted.is_some() || self.risk_override {
            return;
        }
        // A latched stop remains an exit obligation even if equity recovers or
        // the EOD trigger hour has passed. Broker rejection cannot leave an
        // old pending ladder free to fill later in the stopped trading day.
        if self.stopped_trading_day() == Some(day_of(q.ts, self.cfg.session_offset())) {
            if !b.positions().is_empty() || !b.pendings().is_empty() {
                self.close_everything(b, q.ts, CloseReason::DayTarget);
            }
            return;
        }
        let lot = self.lot_size(self.podstawa_lota());
        let scale = self.cfg.usd_scale(lot);
        let eq = self.stats.equity;
        let base = match self.cfg.dd_guard_scope {
            DdGuardScope::Daily => self.stats.day_peak_equity,
            DdGuardScope::Lifetime | DdGuardScope::LifetimePeakDailyReset => self.stats.peak_equity,
        };
        let dd = base - eq;
        let dd_pct = dd / base.max(1.0) * 100.0;

        if self.pulapy.podloga_equity_usd > 0.0
            || self.pulapy.max_dd_pct > 0.0
            || self.pulapy.max_dd_usd > 0.0
        {
            let acc = b.account();
            let szczyt = self.stats.peak_equity.max(acc.equity);
            let dd_konta = szczyt - acc.equity;
            let dd_konta_pct = dd_konta / szczyt.max(1.0) * 100.0;
            let powod = if self.pulapy.podloga_equity_usd > 0.0
                && acc.equity <= self.pulapy.podloga_equity_usd
            {
                Some(format!(
                    "PODŁOGA EQUITY ŁAŃCUCHA: {:.2} $ ≤ {:.2} $",
                    acc.equity, self.pulapy.podloga_equity_usd
                ))
            } else if self.pulapy.max_dd_pct > 0.0 && dd_konta_pct >= self.pulapy.max_dd_pct {
                Some(format!(
                    "OBSUNIĘCIE RACHUNKU {dd_konta_pct:.1}% ≥ {:.1}% (pułap łańcucha)",
                    self.pulapy.max_dd_pct
                ))
            } else if self.pulapy.max_dd_usd > 0.0 && dd_konta >= self.pulapy.max_dd_usd {
                Some(format!(
                    "OBSUNIĘCIE RACHUNKU {dd_konta:.2} $ ≥ {:.2} $ (pułap łańcucha)",
                    self.pulapy.max_dd_usd
                ))
            } else {
                None
            };
            if let Some(r) = powod {
                self.close_everything(b, q.ts, CloseReason::MaxDd);
                *self.odrzuty.entry("HamulecStop".to_string()).or_insert(0) += 1;
                self.halted = Some(r.clone());
                self.log(q.ts, 3, r.clone());
                self.jguard(
                    b,
                    q.ts,
                    RejectCode::Halted,
                    r,
                    dd_konta,
                    dd_konta_pct,
                    self.pulapy.max_dd_pct,
                );
                return;
            }
        }
        if self.cfg.max_dd_pct > 0.0 && dd_pct >= self.cfg.max_dd_pct {
            self.close_everything(b, q.ts, CloseReason::MaxDd);
            let r = format!("MAX DRAWDOWN {dd_pct:.1}% ≥ {:.1}%", self.cfg.max_dd_pct);
            *self.odrzuty.entry("HamulecStop".to_string()).or_insert(0) += 1;
            self.halted = Some(r.clone());
            self.log(q.ts, 3, r.clone());
            self.jguard(
                b,
                q.ts,
                RejectCode::Halted,
                r,
                dd,
                dd_pct,
                self.cfg.max_dd_pct,
            );
            return;
        }
        if self.cfg.max_dd_usd > 0.0 && dd >= self.cfg.max_dd_usd * scale {
            self.close_everything(b, q.ts, CloseReason::MaxDd);
            let r = format!(
                "MAX DRAWDOWN {dd:.2} $ ≥ {:.2} $",
                self.cfg.max_dd_usd * scale
            );
            *self.odrzuty.entry("HamulecStop".to_string()).or_insert(0) += 1;
            self.halted = Some(r.clone());
            self.log(q.ts, 3, r.clone());
            self.jguard(
                b,
                q.ts,
                RejectCode::Halted,
                r,
                dd,
                dd_pct,
                self.cfg.max_dd_usd * scale,
            );
            return;
        }
        if self.cfg.day_trail_stop_usd > 0.0 {
            let d = self.stats.day_peak_equity - eq;
            if d >= self.cfg.day_trail_stop_usd * scale && (!b.positions().is_empty() || !b.pendings().is_empty()) {
                self.close_everything(b, q.ts, CloseReason::DayTarget);
                self.log(q.ts, 2, format!("DAY-TRAIL: equity −{d:.2} $ od piku dnia"));
                self.jguard(
                    b,
                    q.ts,
                    RejectCode::EntryGateBlocked,
                    format!("DAY-TRAIL: equity −{d:.2} $ od piku dnia"),
                    d,
                    dd_pct,
                    self.cfg.day_trail_stop_usd * scale,
                );
            }
        }
        if self.cfg.day_target_usd > 0.0 && self.cfg.day_target_close {
            let scale2 = if self.cfg.day_target_scale_lot {
                (lot / 0.01).max(1.0)
            } else {
                scale
            };
            let today = eq - self.stats.day_start_equity;
            if today >= self.cfg.day_target_usd * scale2 && (!b.positions().is_empty() || !b.pendings().is_empty()) {
                self.close_everything(b, q.ts, CloseReason::DayTarget);
                self.log(q.ts, 1, format!("CEL DZIENNY osiągnięty: +{today:.2} $"));
                self.jguard(
                    b,
                    q.ts,
                    RejectCode::EntryGateBlocked,
                    format!("CEL DZIENNY osiągnięty: +{today:.2} $"),
                    today,
                    dd_pct,
                    self.cfg.day_target_usd * scale2,
                );
            }
        }
        if self.cfg.day_target_pct > 0.0 && self.cfg.day_target_close && self.bramka_dnia_czynna() {
            let baza = self.stats.day_start_equity.max(1.0);
            let prog = baza * self.cfg.day_target_pct / 100.0;
            let dzis = eq - self.stats.day_start_equity;
            if dzis >= prog && (!b.positions().is_empty() || !b.pendings().is_empty()) {
                self.close_everything(b, q.ts, CloseReason::DayTarget);
                let r = format!(
                    "CEL DZIENNY {:.2} % osiągnięty: +{dzis:.2} $ z {baza:.2} $ (próg {prog:.2} $)",
                    self.cfg.day_target_pct
                );
                self.log(q.ts, 1, r.clone());
                self.jguard(b, q.ts, RejectCode::EntryGateBlocked, r, dzis, dd_pct, prog);
            }
        }
        if self.cfg.day_trail_stop_pct > 0.0 && self.bramka_dnia_czynna() {
            let threshold = self.cfg.day_trail_threshold(
                self.stats.day_start_equity, self.stats.day_peak_equity);
            let uzbrojony = threshold.is_some();
            let (szczyt, prog) = threshold.unwrap_or((1.0, 0.0));
            let oddane = self.stats.day_peak_equity - eq;
            if uzbrojony && oddane >= prog {
                self.zatrzymaj_dobe(q.ts);
            }
            if uzbrojony && oddane >= prog && (!b.positions().is_empty() || !b.pendings().is_empty()) {
                self.close_everything(b, q.ts, CloseReason::DayTarget);
                let r = format!(
                    "STOP DNIA: oddane {oddane:.2} $, podstawa {:?} {szczyt:.2} $ \
                     ({:.2} % ≥ {:.2} %)",
                    self.cfg.day_trail_basis,
                    oddane / szczyt * 100.0,
                    self.cfg.day_trail_stop_pct
                );
                self.log(q.ts, 2, r.clone());
                self.jguard(
                    b,
                    q.ts,
                    RejectCode::EntryGateBlocked,
                    r,
                    oddane,
                    dd_pct,
                    prog,
                );
            }
        }
        let hour = hour_of(q.ts, self.cfg.session_offset());
        if self.cfg.eod_flat_hour > 0.0 && hour == self.cfg.eod_flat_hour as u32 {
            self.zatrzymaj_dobe(q.ts);
            if !b.positions().is_empty() || !b.pendings().is_empty() {
                self.close_everything(b, q.ts, CloseReason::EodFlat);
                self.log(q.ts, 0, "EOD-FLAT");
            }
        }
        if self.cfg.flat_weekend {
            let wd = weekday_of(q.ts, self.cfg.session_offset());
            if wd == 4 && hour >= self.cfg.flat_weekend_hour as u32 {
                self.zatrzymaj_dobe(q.ts);
                if !b.positions().is_empty() || !b.pendings().is_empty() {
                    self.close_everything(b, q.ts, CloseReason::EodFlat);
                    self.log(q.ts, 0, "FLAT przed weekendem");
                }
            }
        }
    }

    fn zatrzymaj_dobe(&mut self, ts: Ts) {
        let d = day_of(ts, self.cfg.session_offset());
        if self.day_stop == d {
            return;
        }
        self.day_stop = d;
        self.log(
            ts,
            1,
            "DOBA ZAMKNIĘTA dla wejść — do północy silnika".to_string(),
        );
    }

    /// Account/strategy-scoped persistence for the day latch, independent of
    /// optional order-continuation replay. Restoring it can only retain a stop.
    pub fn stopped_trading_day(&self) -> Option<i64> {
        (self.day_stop != i64::MIN).then_some(self.day_stop)
    }

    pub fn restore_stopped_trading_day(&mut self, day: Option<i64>) {
        if let Some(day) = day { self.day_stop = self.day_stop.max(day); }
    }


    #[track_caller]
    fn close_or_queue<B: Broker>(&mut self, b: &mut B, t: Ticket, reason: CloseReason, q: &Quote) {
        if b.find_position(t)
            .and_then(|p| p.basket)
            .map(|id| self.basket_exit_pending(id))
            .unwrap_or(false)
        {
            return;
        }
        if cien::czynny() {
            cien::z(
                cakt::A_ZYCIE_POZ,
                t,
                czr::L_CLOSE_OR_QUEUE,
                std::panic::Location::caller().line(),
            );
        }
        if !self.cfg.exit_via_limit {
            let _ = b.close_position(t, reason);
            return;
        }
        if self.queued_exits.contains_key(&t) {
            return;
        }
        let p = match b.find_position(t) {
            Some(p) => p.clone(),
            None => return,
        };
        let zysk = p.profit_pts(q);
        if zysk < self.cfg.exit_limit_min_profit {
            let _ = b.close_position(t, reason);
            return;
        }
        let spread = (q.ask - q.bid).max(0.0);
        let zapas = spread + self.cfg.exit_limit_offset.max(0.0);
        if zapas <= 0.0 {
            let _ = b.close_position(t, reason);
            return;
        }
        let rynek = q.exit(p.side);
        let target = match p.side {
            Side::Buy => rynek + zapas,
            Side::Sell => rynek - zapas,
        };
        let czekaj = (self.cfg.exit_limit_wait_s.max(0.0) * 1000.0) as i64;
        if self.cfg.restore_strategy_continuation {
            let guard = self.capture_continuation_guard(b, t);
            self.remember_continuation_guard(t, guard, true);
        }
        self.queued_exits.insert(
            t,
            QueuedExit {
                target,
                deadline: q.ts + czekaj,
                reason,
                market_at_decision: rynek,
            },
        );
        if self.journal.wants(EventLevel::Debug) {
            self.journal.push(
                Ev::new(
                    q.ts,
                    EventLevel::Debug,
                    EventCategory::Decision,
                    EventKind::Note,
                )
                .text(format!(
                    "#{t}: wyjście czeka na {target:.2} zamiast rynku {rynek:.2} \
                         (spread {spread:.2} $, do {} s)",
                    self.cfg.exit_limit_wait_s
                ))
                .ticket(t)
                .basket_opt(p.basket)
                .reason(RejectCode::ExitWaitingForLimit)
                .put_f("target", target)
                .put_f("market", rynek)
                .put_f("spread", spread)
                .build(),
            );
        }
    }

    fn sweep_queued_exits<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if self.queued_exits.is_empty() {
            return;
        }
        let lista: Vec<(Ticket, QueuedExit)> =
            self.queued_exits.iter().map(|(k, v)| (*k, *v)).collect();
        for (t, qe) in lista {
            let p = match b.find_position(t) {
                Some(p) => p.clone(),
                None => {
                    self.queued_exits.remove(&t);
                    continue;
                }
            };
            if p.basket
                .map(|id| self.basket_exit_pending(id))
                .unwrap_or(false)
            {
                self.queued_exits.remove(&t);
                self.forget_continuation_guard(t, true);
                continue;
            }
            if !self.continuation_exit_proved(b, t) {
                continue;
            }
            let osiagniete = match p.side {
                Side::Buy => q.bid >= qe.target - 1e-9,
                Side::Sell => q.ask <= qe.target + 1e-9,
            };
            let spozniony = q.ts >= qe.deadline;
            if !osiagniete && !spozniony {
                continue;
            }
            let cena = q.exit(p.side);
            let zysk_ze_zwloki =
                (cena - qe.market_at_decision) * p.side.sign() * XAU_CONTRACT * p.volume;
            cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_KOLEJKA_WYJSC, 0);
            if b.close_position(t, qe.reason).is_ok() && self.journal.wants(EventLevel::Info) {
                let (poziom, kod, opis) = if osiagniete {
                    (
                        EventLevel::Ok,
                        RejectCode::ExitWaitingForLimit,
                        "wypełnione po lepszej cenie",
                    )
                } else {
                    (
                        EventLevel::Warn,
                        RejectCode::ExitLimitTimeout,
                        "czas minął — wyjście po rynku",
                    )
                };
                self.journal.push(
                    Ev::new(q.ts, poziom, EventCategory::Decision, EventKind::Note)
                        .text(format!(
                            "#{t}: {opis} — {cena:.2} wobec {:.2} przy decyzji ({zysk_ze_zwloki:+.2} $)",
                            qe.market_at_decision
                        ))
                        .ticket(t)
                        .basket_opt(p.basket)
                        .reason(kod)
                        .put_f("exit_price", cena)
                        .put_f("market_at_decision", qe.market_at_decision)
                        .put_f("gain_from_waiting", zysk_ze_zwloki)
                        .put_f("target", qe.target)
                        .build(),
                );
            }
            self.queued_exits.remove(&t);
            self.forget_continuation_guard(t, true);
        }
    }

    fn manage_positions<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        let ts = q.ts;

        self.sweep_queued_exits(b, q);

        if self.cfg.exit_spread_mult > 0.0 {
            self.spread_buf.push(q.ask - q.bid);
            if self.spread_buf.len() >= 512 {
                let mut v = std::mem::take(&mut self.spread_buf);
                v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                self.spread_med = v[v.len() / 2];
                v.clear();
                self.spread_buf = v;
            }
        }
        let stops = b.stops_level();
        let vsl_due = self.cfg.vsl_eval_s <= 0.0
            || ts - self.last_vsl_eval >= (self.cfg.vsl_eval_s * 1000.0) as i64;
        if vsl_due {
            self.last_vsl_eval = ts;
        }

        for p in b.positions_mut().iter_mut() {
            let pts = (q.exit(p.side) - p.open_price) * p.side.sign();
            if pts > p.peak_pts {
                p.peak_pts = pts;
                p.last_peak_ts = ts;
            }
        }

        if self.cfg.basket_target_usd > 0.0 {
            let mut zysk: HashMap<u32, f64> = HashMap::new();
            for p in b.positions() {
                if let Some(bid) = p.basket {
                    *zysk.entry(bid).or_insert(0.0) += p.profit_usd(q);
                }
            }
            let do_zamkniecia: Vec<u32> = zysk
                .into_iter()
                .filter(|(_, v)| *v >= self.cfg.basket_target_usd)
                .map(|(k, _)| k)
                .collect();
            for bid in do_zamkniecia {
                if self.basket_exit_pending(bid) {
                    continue;
                }
                let tickety: Vec<Ticket> = b
                    .positions()
                    .iter()
                    .filter(|p| p.basket == Some(bid))
                    .map(|p| p.ticket)
                    .collect();
                for t in tickety {
                    self.close_or_queue(b, t, CloseReason::Harvest, q);
                }
                self.basket_note(
                    bid,
                    ts,
                    format!("cel koszyka {:.2} $ osiągnięty", self.cfg.basket_target_usd),
                );
            }
        }

        let runnerzy = self.runnerzy_wg_glebokosci(b);
        let snapshot: Vec<Position> = b.positions().to_vec();
        let trail_adaptive = self.trail_adaptive_snapshot(q);

        let sr_swieca = self.cfg.trail_sr_enabled && self.sr.nowa_swieca;
        if sr_swieca {
            self.sr.nowa_swieca = false;
        }

        for p in snapshot {
            if p.frozen
                || p.basket
                    .map(|id| self.basket_exit_pending(id))
                    .unwrap_or(false)
            {
                continue;
            }
            let pts = (q.exit(p.side) - p.open_price) * p.side.sign();

            if self.cfg.virtual_sl && vsl_due {
                if let Some(v) = p.vsl {
                    let hit = match p.side {
                        Side::Buy => q.bid <= v,
                        Side::Sell => q.ask >= v,
                    };
                    if hit {
                        cien::z(cakt::A_ZYCIE_POZ, p.ticket, czr::Z_VIRTUAL_SL, 0);
                        let _ = b.close_position(p.ticket, CloseReason::VirtualSl);
                        continue;
                    }
                }
            }

            if self.cfg.exit_via_limit && self.queued_exits.contains_key(&p.ticket) {
                continue;
            }

            let wiek_min = (ts - p.open_ts) as f64 / 60_000.0;
            let za_swieza =
                self.cfg.exit_min_hold_min > 0.0 && wiek_min < self.cfg.exit_min_hold_min;
            let za_maly_zysk = self.cfg.exit_min_profit > 0.0 && pts < self.cfg.exit_min_profit;
            let po_tp_hit = self.cfg.hold_after_tp_hit_min > 0.0
                && self.last_tp_hit_ts > 0
                && (ts - self.last_tp_hit_ts) as f64 / 60_000.0 < self.cfg.hold_after_tp_hit_min;
            let reguly_wolne = !za_swieza && !za_maly_zysk && !po_tp_hit;

            if reguly_wolne && pts > 0.0 {
                if self.cfg.exit_r_multiple > 0.0 {
                    if let Some(sl) = p.sl {
                        let ryzyko = (p.open_price - sl).abs();
                        if ryzyko > 1e-9 && pts >= ryzyko * self.cfg.exit_r_multiple {
                            self.close_or_queue(b, p.ticket, CloseReason::Harvest, q);
                            continue;
                        }
                    }
                }
                if self.cfg.exit_round_dist > 0.0 && self.cfg.exit_round_step > 0.0 {
                    let cena = q.exit(p.side);
                    let krok = self.cfg.exit_round_step;
                    let najblizszy = (cena / krok).round() * krok;
                    let przed = match p.side {
                        Side::Buy => najblizszy >= cena,
                        Side::Sell => najblizszy <= cena,
                    };
                    if przed && (najblizszy - cena).abs() <= self.cfg.exit_round_dist {
                        self.close_or_queue(b, p.ticket, CloseReason::Harvest, q);
                        continue;
                    }
                }
                if self.cfg.exit_spread_mult > 0.0 && self.spread_med > 0.0 {
                    if (q.ask - q.bid) >= self.spread_med * self.cfg.exit_spread_mult {
                        self.close_or_queue(b, p.ticket, CloseReason::Harvest, q);
                        continue;
                    }
                }
            }

            if self.cfg.smart_exit && pts > 0.0 && reguly_wolne {
                let trzymaj_bo_siatka = if self.cfg.smart_exit_hold_if_pending > 0.0 {
                    let prog = self.cfg.smart_exit_hold_if_pending;
                    let blisko = self.cfg.smart_exit_pending_min_dist;
                    let ten_sam = self.cfg.smart_exit_pending_scope == PendingScope::SameBasket;
                    let ile = b
                        .pendings()
                        .iter()
                        .filter(|o| o.kind.side() == p.side)
                        .filter(|o| !ten_sam || o.basket == p.basket)
                        .filter(|o| {
                            let d = match p.side {
                                Side::Buy => q.bid - o.price,
                                Side::Sell => o.price - q.ask,
                            };
                            d > blisko && d <= prog
                        })
                        .count() as u32;
                    ile >= self.cfg.smart_exit_min_pendings.max(1)
                } else {
                    false
                };

                if !trzymaj_bo_siatka {
                    if self.cfg.smart_exit_take > 0.0 && pts >= self.cfg.smart_exit_take {
                        self.close_or_queue(b, p.ticket, CloseReason::Harvest, q);
                        continue;
                    }
                    if self.cfg.smart_exit_giveback > 0.0
                        && p.peak_pts >= self.cfg.smart_exit_min_peak
                        && p.peak_pts - pts >= p.peak_pts * self.cfg.smart_exit_giveback
                    {
                        self.close_or_queue(b, p.ticket, CloseReason::Harvest, q);
                        continue;
                    }
                    if self.cfg.smart_exit_drop_speed > 0.0 {
                        let okno = (self.cfg.smart_exit_speed_window_s * 1000.0) as i64;
                        let wiek = ts - p.last_peak_ts;
                        if wiek > 0 && wiek <= okno {
                            let spadek = p.peak_pts - pts;
                            let na_min = spadek / (wiek as f64 / 60_000.0).max(1e-6);
                            if na_min >= self.cfg.smart_exit_drop_speed {
                                self.close_or_queue(b, p.ticket, CloseReason::Harvest, q);
                                continue;
                            }
                        }
                    }
                }
            }

            if self.cfg.harvest_retrace_pct > 0.0 && p.peak_pts >= self.cfg.harvest_start {
                if p.peak_pts - pts >= p.peak_pts * self.cfg.harvest_retrace_pct / 100.0 {
                    self.close_or_queue(b, p.ticket, CloseReason::Harvest, q);
                    continue;
                }
            }

            if self.cfg.oae_timeout_min > 0.0 {
                let age_min = (ts - p.open_ts) as f64 / 60_000.0;
                if age_min >= self.cfg.oae_timeout_min && pts < self.cfg.oae_profit_min {
                    cien::z(cakt::A_ZYCIE_POZ, p.ticket, czr::Z_OAE_TIMEOUT, 0);
                    let _ = b.close_position(p.ticket, CloseReason::OutAtEntry);
                    continue;
                }
            }

            let stag_min = (ts - p.last_peak_ts) as f64 / 60_000.0;
            let s1 = self.cfg.stale_take_min > 0.0
                && stag_min >= self.cfg.stale_take_min
                && pts >= self.cfg.stale_take_profit;
            let s2 = self.cfg.stale_take_min2 > 0.0
                && stag_min >= self.cfg.stale_take_min2
                && pts >= self.cfg.stale_take_profit2;
            if s1 || s2 {
                self.close_or_queue(b, p.ticket, CloseReason::Stale, q);
                continue;
            }

            if self.cfg.be_lock_pts > 0.0 && pts >= self.cfg.be_lock_pts {
                let be = p.open_price + p.side.sign() * self.cfg.be_offset;
                let luzuje = p.sl.map(|s| p.side.better(be, s)).unwrap_or(false);
                if !luzuje && sl_is_valid(p.side, be, q, stops) {
                    self.try_modify(b, p.ticket, Some(be), p.tp, ts);
                }
            }

            let runner_teraz = if self.cfg.trail_runners_by_depth {
                runnerzy.contains(&p.ticket)
            } else {
                p.is_runner
            };
            if let Some(cand) = self.trail_candidate(&p, q, runner_teraz, trail_adaptive.as_ref()) {
                let improves = p.sl.map(|s| !p.side.better(cand, s)).unwrap_or(true);
                let above_be = !p.side.better(cand, p.open_price);
                if improves && above_be {
                    let min_d = self.cfg.trail_min_dist.max(stops);
                    let safe = match p.side {
                        Side::Buy => cand.min(q.bid - min_d),
                        Side::Sell => cand.max(q.ask + min_d),
                    };
                    let still = p.sl.map(|s| !p.side.better(safe, s)).unwrap_or(true);
                    if still && sl_is_valid(p.side, safe, q, stops) {
                        if self.cfg.virtual_sl && !self.cfg.virtual_sl_only_when_rejected {
                            if let Some(pp) =
                                b.positions_mut().iter_mut().find(|x| x.ticket == p.ticket)
                            {
                                pp.vsl = Some(cand);
                            }
                        } else {
                            self.try_modify(b, p.ticket, Some(safe), p.tp, ts);
                        }
                    }
                }
            }

            if sr_swieca {
                let w_zakresie = match self.cfg.trail_sr_scope {
                    TrailSrScope::All => true,
                    TrailSrScope::Runner => runner_teraz,
                    TrailSrScope::Tp3Up => self.sr_warstwa_tp3_up(&p),
                };
                let (stage, next_tp) = p
                    .basket
                    .and_then(|id| self.basket(id))
                    .map(|bk| (bk.tp_stage, bk.tps.get(bk.tp_stage).copied()))
                    .unwrap_or((0, None));
                let aktywna = match self.cfg.trail_sr_activation {
                    TrailSrActivation::Entry => true,
                    TrailSrActivation::Gain => pts >= self.cfg.trail_sr_min_gain,
                    TrailSrActivation::Tp1 => stage >= 1,
                    TrailSrActivation::Tp2 => stage >= 2,
                    TrailSrActivation::Tp3 => stage >= 3,
                };
                if w_zakresie && aktywna {
                    if let Some(poziom) = self.sr_kandydat(p.side, q.mid(), next_tp, ts) {
                        let sl_prop = poziom - p.side.sign() * self.sr_effective_offset();
                        if let Some((sl_teraz, vsl_teraz, tp_teraz)) =
                            b.find_position(p.ticket).map(|x| (x.sl, x.vsl, x.tp))
                        {
                            let poprawia = sl_teraz
                                .map(|s| !p.side.better(sl_prop, s) && (sl_prop - s).abs() > 1e-9)
                                .unwrap_or(true);
                            if poprawia && sl_is_valid(p.side, sl_prop, q, stops) {
                                if self.cfg.virtual_sl && !self.cfg.virtual_sl_only_when_rejected {
                                    let vsl_poprawia = vsl_teraz
                                        .map(|v| {
                                            !p.side.better(sl_prop, v) && (sl_prop - v).abs() > 1e-9
                                        })
                                        .unwrap_or(true);
                                    if vsl_poprawia {
                                        if let Some(pp) = b
                                            .positions_mut()
                                            .iter_mut()
                                            .find(|x| x.ticket == p.ticket)
                                        {
                                            pp.vsl = Some(sl_prop);
                                        }
                                    }
                                } else {
                                    self.try_modify(b, p.ticket, Some(sl_prop), tp_teraz, ts);
                                }
                            }
                        }
                    }
                }
            }
        }
    }


    #[inline]
    fn sr_dynamic_active(&self) -> bool {
        self.cfg.trail_sr_min_prominence_atr > 0.0
            || self.cfg.trail_sr_offset_atr_mult > 0.0
            || self.cfg.trail_sr_offset_spread_mult > 0.0
    }

    #[inline]
    fn sr_dynamic_ready(&self) -> bool {
        if !self.sr_dynamic_active() {
            return true;
        }
        let n = self.cfg.trail_sr_atr_period.max(1) as usize;
        self.sr.atr_true_ranges.len() >= n
            && self.sr.spread_closed.len() >= n
            && self.sr.atr.is_some()
            && self.sr.spread_ref.is_some()
    }

    #[inline]
    fn sr_effective_offset(&self) -> f64 {
        if !self.sr_dynamic_active() {
            return self.cfg.trail_sr_offset;
        }
        let mut out = self.cfg.trail_sr_offset;
        if self.cfg.trail_sr_offset_atr_mult > 0.0 {
            out = out.max(self.sr.atr.unwrap_or(0.0) * self.cfg.trail_sr_offset_atr_mult);
        }
        if self.cfg.trail_sr_offset_spread_mult > 0.0 {
            out = out.max(self.sr.spread_ref.unwrap_or(0.0) * self.cfg.trail_sr_offset_spread_mult);
        }
        out
    }

    pub fn rozgrzej_sr_z_m1(&mut self, bars: &[SrWarmupBar]) -> bool {
        if !self.cfg.trail_sr_enabled || !self.sr_dynamic_active() {
            return true;
        }
        self.sr = StanSr::default();
        let tf_ms = 60_000 * self.cfg.trail_sr_tf_min.max(1) as i64;
        let mut ostatni_ts = i64::MIN;
        for bar in bars {
            if bar.ts <= ostatni_ts
                || !bar.high.is_finite()
                || !bar.low.is_finite()
                || !bar.close.is_finite()
                || bar.high < bar.low
            {
                continue;
            }
            ostatni_ts = bar.ts;
            let kubelek = bar.ts.div_euclid(tf_ms);
            if self.sr.kubelek == i64::MIN {
                self.sr.kubelek = kubelek;
                self.sr.high = bar.high;
                self.sr.low = bar.low;
                self.sr.close = bar.close;
                self.sr.spread_close = bar.spread.max(0.0);
            } else if kubelek == self.sr.kubelek {
                self.sr.high = self.sr.high.max(bar.high);
                self.sr.low = self.sr.low.min(bar.low);
                self.sr.close = bar.close;
                self.sr.spread_close = bar.spread.max(0.0);
            } else if kubelek > self.sr.kubelek {
                self.sr_zamknij_swiece(bar.ts);
                self.sr.kubelek = kubelek;
                self.sr.high = bar.high;
                self.sr.low = bar.low;
                self.sr.close = bar.close;
                self.sr.spread_close = bar.spread.max(0.0);
            }
        }
        self.sr.nowa_swieca = false;
        self.sr_dynamic_ready()
    }

    fn sr_na_ticku(&mut self, q: &Quote) {
        let kubelek =
            q.ts.div_euclid(60_000 * self.cfg.trail_sr_tf_min.max(1) as i64);
        let mid = q.mid();
        let dynamiczny = self.sr_dynamic_active();
        if self.sr.kubelek == i64::MIN {
            self.sr.kubelek = kubelek;
            self.sr.high = mid;
            self.sr.low = mid;
            if dynamiczny {
                self.sr.close = mid;
                self.sr.spread_close = (q.ask - q.bid).max(0.0);
            }
            return;
        }
        if kubelek != self.sr.kubelek {
            self.sr_zamknij_swiece(q.ts);
            self.sr.kubelek = kubelek;
            self.sr.high = mid;
            self.sr.low = mid;
            if dynamiczny {
                self.sr.close = mid;
                self.sr.spread_close = (q.ask - q.bid).max(0.0);
            }
        } else {
            if mid > self.sr.high {
                self.sr.high = mid;
            }
            if mid < self.sr.low {
                self.sr.low = mid;
            }
            if dynamiczny {
                self.sr.close = mid;
                self.sr.spread_close = (q.ask - q.bid).max(0.0);
            }
        }
    }


    fn sr_zamknij_swiece(&mut self, ts: Ts) {
        sr_state::SrStateMath {
            cfg: &self.cfg,
            sr: &mut self.sr,
        }
        .sr_zamknij_swiece(ts);
    }

    fn sr_kandydat(&self, side: Side, mid: Px, next_tp: Option<Px>, ts: Ts) -> Option<Px> {
        if !self.sr_dynamic_ready() {
            return None;
        }
        let swingi = match side {
            Side::Buy => &self.sr.swingi_low,
            Side::Sell => &self.sr.swingi_high,
        };
        let mdp = self.cfg.trail_sr_min_dist_price;
        let min_prom = self.cfg.trail_sr_min_prominence_atr;
        swingi
            .iter()
            .filter(|&&(_, t)| t <= ts)
            .filter(|&&(lvl, _)| match side {
                Side::Buy => lvl < mid,
                Side::Sell => lvl > mid,
            })
            .filter(|&&(lvl, _)| mdp <= 0.0 || (mid - lvl).abs() >= mdp)
            .filter(|&&(lvl, t)| {
                if min_prom <= 0.0 {
                    return true;
                }
                let p = match side {
                    Side::Buy => &self.sr.prominence_low,
                    Side::Sell => &self.sr.prominence_high,
                };
                p.iter()
                    .find(|&&(pl, pt, _)| pl == lvl && pt == t)
                    .map(|&(_, _, v)| v >= min_prom)
                    .unwrap_or(false)
            })
            .filter(|&&(lvl, _)| {
                next_tp
                    .map(|t| (lvl - t).abs() >= self.cfg.trail_sr_min_dist_tp)
                    .unwrap_or(true)
            })
            .min_by(|a, b| {
                (mid - a.0)
                    .abs()
                    .partial_cmp(&(mid - b.0).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|&(lvl, _)| lvl)
    }

    fn sr_warstwa_tp3_up(&self, p: &Position) -> bool {
        let tp = match p.tp {
            Some(t) => t,
            None => return true,
        };
        match p.basket.and_then(|id| self.basket(id)) {
            Some(bk) => bk
                .tps
                .iter()
                .position(|&t| (t - tp).abs() < 0.01)
                .map(|i| i >= 2)
                .unwrap_or(false),
            None => false,
        }
    }

    fn runnerzy_wg_glebokosci<B: Broker>(&self, b: &B) -> std::collections::HashSet<Ticket> {
        let mut out = std::collections::HashSet::new();
        if !self.cfg.trail_split || !self.cfg.trail_runners_by_depth {
            return out;
        }
        let n = self.cfg.trail_runners_n.max(1) as usize;
        for bk in self.baskets.iter().filter(|x| x.alive()) {
            let mut poz: Vec<&Position> = b
                .positions()
                .iter()
                .filter(|p| p.basket == Some(bk.id))
                .collect();
            poz.sort_by(|a, c| {
                if bk.side.better(a.open_price, c.open_price) {
                    std::cmp::Ordering::Less
                } else if bk.side.better(c.open_price, a.open_price) {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            });
            for p in poz.into_iter().take(n) {
                out.insert(p.ticket);
            }
        }
        out
    }

    fn trail_adaptive_snapshot(&self, q: &Quote) -> Option<TrailAdaptiveSnapshot> {
        if !self.cfg.trail_adaptive_enabled {
            return None;
        }

        let er_ms = (self.cfg.trail_adaptive_window_s.max(0.0) * 1000.0) as i64;
        let fast_ms = (self.cfg.trail_adaptive_fast_vol_s.max(0.0) * 1000.0) as i64;
        let slow_ms = (self.cfg.trail_adaptive_slow_vol_s.max(0.0) * 1000.0) as i64;
        if er_ms <= 0 {
            return None;
        }
        let oldest = q.ts - er_ms.max(fast_ms).max(slow_ms);
        let first = self.vol_hist.partition_point(|(ts, _)| *ts < oldest);
        let er_from = q.ts - er_ms;
        let fast_from = q.ts - fast_ms;
        let slow_from = q.ts - slow_ms;
        let mut er = MovementAccumulator::default();
        let mut fast = MovementAccumulator::default();
        let mut slow = MovementAccumulator::default();
        let mid = q.mid();
        let current_already_sampled = self
            .vol_hist
            .last()
            .is_some_and(|(ts, px)| *ts == q.ts && px.to_bits() == mid.to_bits());

        let mut feed = |ts: Ts, px: Px| {
            if ts >= er_from {
                er.push(ts, px);
            }
            if fast_ms > 0 && ts >= fast_from {
                fast.push(ts, px);
            }
            if slow_ms > 0 && ts >= slow_from {
                slow.push(ts, px);
            }
        };
        for &(ts, px) in &self.vol_hist[first..] {
            if ts <= q.ts {
                feed(ts, px);
            }
        }
        if !current_already_sampled {
            feed(q.ts, mid);
        }

        let min_samples = self.cfg.trail_adaptive_min_samples.max(2);
        if er.samples < min_samples || er.path <= 1e-12 {
            return None;
        }
        let price_efficiency = ((er.last_px - er.first_px) / er.path).clamp(-1.0, 1.0);
        let vol_ratio = match (fast.path_rate(), slow.path_rate()) {
            (Some(a), Some(b)) if b > 1e-12 => Some(a / b),
            _ => None,
        };
        Some(TrailAdaptiveSnapshot {
            price_efficiency,
            vol_ratio,
        })
    }

    fn trail_adaptive_gap(
        &self,
        p: &Position,
        jest_runnerem: bool,
        base_gap: f64,
        snap: Option<&TrailAdaptiveSnapshot>,
    ) -> f64 {
        let Some(snap) = snap else { return base_gap };
        if !self.cfg.trail_adaptive_enabled
            || (self.cfg.trail_adaptive_runners_only && !jest_runnerem)
            || p.peak_pts < self.cfg.trail_adaptive_min_peak.max(0.0)
        {
            return base_gap;
        }

        let signed_er = snap.price_efficiency * p.side.sign();
        let trend_thr = self.cfg.trail_adaptive_trend_er.abs().clamp(0.0, 1.0);
        let reversal_thr = self.cfg.trail_adaptive_reversal_er.abs().clamp(0.0, 1.0);
        let mut mult = if signed_er >= trend_thr {
            self.cfg.trail_adaptive_trend_gap_mult
        } else if signed_er <= -reversal_thr {
            self.cfg.trail_adaptive_reversal_gap_mult
        } else {
            self.cfg.trail_adaptive_chop_gap_mult
        };
        if !mult.is_finite() {
            mult = 1.0;
        }
        mult = mult.max(0.0);

        if self.cfg.trail_adaptive_vol_ratio > 0.0
            && snap
                .vol_ratio
                .is_some_and(|r| r >= self.cfg.trail_adaptive_vol_ratio)
        {
            let vol_mult = if signed_er >= 0.0 {
                self.cfg.trail_adaptive_vol_favorable_mult
            } else {
                self.cfg.trail_adaptive_vol_adverse_mult
            };
            if vol_mult.is_finite() {
                mult *= vol_mult.max(0.0);
            }
        }

        let mut gap = base_gap.max(0.0) * mult;
        let lo = self.cfg.trail_adaptive_min_gap.max(0.0);
        if lo > 0.0 {
            gap = gap.max(lo);
        }
        if self.cfg.trail_adaptive_max_gap > 0.0 {
            gap = gap.min(self.cfg.trail_adaptive_max_gap.max(lo));
        }
        gap
    }

    fn trail_candidate(
        &self,
        p: &Position,
        q: &Quote,
        jest_runnerem: bool,
        adaptive: Option<&TrailAdaptiveSnapshot>,
    ) -> Option<Px> {
        if self.cfg.riskfree_enabled
            && self.cfg.riskfree_runner_stop == RiskFreeRunnerStop::TrailGap
            && p.basket
                .and_then(|id| self.basket(id))
                .map(|bk| bk.secured)
                .unwrap_or(false)
        {
            let luz_bazowy = if self.cfg.riskfree_runner_gap > 0.0 {
                self.cfg.riskfree_runner_gap
            } else {
                25.0
            };
            let luz = self.trail_adaptive_gap(p, true, luz_bazowy, adaptive);
            let peak = p.peak_pts;
            if peak <= 0.0 {
                return None;
            }
            return Some(p.open_price + p.side.sign() * (peak - luz));
        }
        let zabezpieczony = p
            .basket
            .and_then(|id| self.basket(id))
            .map(|bk| bk.secured)
            .unwrap_or(false);
        if self.cfg.risk_free_trail
            && !self.cfg.riskfree_enabled
            && zabezpieczony
            && self.cfg.trail_mode == TrailMode::Off
            && self.cfg.trail_runner_mode != TrailMode::Off
        {
            return self.trail_z_parametrow(
                p,
                q,
                self.cfg.trail_runner_mode,
                self.cfg.trail_runner_start,
                self.cfg.trail_runner_gap,
                self.cfg.trail_runner_lock_pct,
                &self.cfg.trail_runner_tiers,
                true,
                adaptive,
            );
        }
        let runner = self.cfg.trail_split && jest_runnerem;
        let (mode, start, gap, lock, tiers) = if runner {
            (
                self.cfg.trail_runner_mode,
                self.cfg.trail_runner_start,
                self.cfg.trail_runner_gap,
                self.cfg.trail_runner_lock_pct,
                &self.cfg.trail_runner_tiers,
            )
        } else {
            (
                self.cfg.trail_mode,
                self.cfg.trail_start,
                self.cfg.trail_gap,
                self.cfg.trail_lock_pct,
                &self.cfg.trail_tiers,
            )
        };
        self.trail_z_parametrow(p, q, mode, start, gap, lock, tiers, jest_runnerem, adaptive)
    }

    fn trail_z_parametrow(
        &self,
        p: &Position,
        q: &Quote,
        mode: TrailMode,
        start: f64,
        gap: f64,
        lock: f64,
        tiers: &str,
        jest_runnerem: bool,
        adaptive: Option<&TrailAdaptiveSnapshot>,
    ) -> Option<Px> {
        if mode == TrailMode::Off || p.peak_pts < start {
            return None;
        }
        let s = p.side.sign();
        match mode {
            TrailMode::Gap => {
                let gap = self.trail_adaptive_gap(p, jest_runnerem, gap, adaptive);
                Some(q.exit(p.side) - s * gap)
            }
            TrailMode::LockPct => Some(p.open_price + s * p.peak_pts * lock / 100.0),
            TrailMode::Tiered => {
                let mut best = None;
                for (thr, keep) in Settings::parse_tiers(tiers) {
                    if p.peak_pts >= thr {
                        best = Some(keep);
                    }
                }
                best.map(|k| p.open_price + s * k)
            }
            TrailMode::Atr | TrailMode::Chandelier => {
                let mult = self.cfg.trail_atr_mult;
                if mult <= 0.0 {
                    return None;
                }
                let atr = self.atr_proxy(q.ts)?;
                let luka = self.trail_adaptive_gap(p, jest_runnerem, mult * atr, adaptive);
                let kotwica = match mode {
                    TrailMode::Chandelier => p.open_price + s * p.peak_pts,
                    _ => q.exit(p.side),
                };
                Some(kotwica - s * luka)
            }
            TrailMode::Off => None,
        }
    }


    fn expire_stale_baskets<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        if self.cfg.ignore_old_after_min <= 0.0 {
            return;
        }
        let limit = (self.cfg.ignore_old_after_min * 60_000.0) as i64;
        let stale: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| {
                x.alive() && !self.keep_explicit_pending(x.id)
                    && !x.had_positions && x.tickets.is_empty() && ts - x.created_ts > limit
            })
            .map(|x| x.id)
            .collect();
        for id in stale {
            if self.cfg.confirmed_exit_retry {
                self.request_confirmed_exit(b, id, ts, CloseReason::Expired);
                continue;
            }
            let n = self.cancel_pendings(b, id);
            if let Some(bk) = self.basket_mut(id) {
                bk.state = BasketState::Done;
            }
            self.basket_note(
                id,
                ts,
                format!(
                    "sygnał anulowany — czekał ponad {:.0} min bez wejścia (skasowano {n} limitów)",
                    self.cfg.ignore_old_after_min
                ),
            );
        }
    }

    fn resize_pendings<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        if !self.cfg.pending_resize_on_vol || self.cfg.vol_window_min <= 0.0 {
            return;
        }
        let gap = (self.cfg.pending_resize_s.max(0.0) * 1000.0) as i64;
        if ts - self.last_resize < gap {
            return;
        }
        self.last_resize = ts;
        let ids: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && !x.pendings.is_empty())
            .map(|x| x.id)
            .collect();
        for id in ids {
            self.sync_grid(b, id, ts, true);
        }
    }

    fn relot_pendings<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        if !self.cfg.pending_relot_on_balance {
            return;
        }
        if self.cfg.pending_relot_reconcile_target {
            self.relot_pendings_reconciled(b, ts);
            return;
        }
        let v2 = self.strict_open_volume();
        let delta_epsilon = if v2 {
            b.volume_step().abs() * 0.5
        } else {
            0.005
        };
        let amount = |v: f64| if v2 { v } else { (v * 100.0).round() / 100.0 };
        let gap = (self.cfg.pending_resize_s.max(0.0) * 1000.0) as i64;
        if ts - self.last_relot < gap {
            return;
        }
        self.last_relot = ts;

        let cel = self.lot_size(self.podstawa_lota());
        let zywe: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive())
            .filter(|x| !self.basket_exit_pending(x.id))
            .filter(|x| !self.entry_edit_blocks(Some(x.id)))
            .map(|x| x.id)
            .collect();

        let wolne = self.wolny_budzet_portfela(b);
        let mut plany: HashMap<u32, Vec<(i32, f64)>> = HashMap::new();
        for id in &zywe {
            let (p, _, _) = self.plan_grid(*id, ts, wolne, None);
            if !p.is_empty() && !self.plan_ma_ten_sam_ksztalt(*id, &p) {
                self.stats.relot_ksztalt_odmowa += 1;
                continue;
            }
            if p.is_empty() {
                self.stats.relot_plan_pusty += 1;
                continue;
            }
            self.stats.relot_plan_ok += 1;
            plany.insert(*id, p.iter().map(|g| (g.level, g.volume)).collect());
        }

        let mut szczeble: Vec<(u32, i32)> = b
            .pendings()
            .iter()
            .filter(|p| !p.frozen)
            .filter_map(|p| {
                p.basket
                    .filter(|id| zywe.contains(id))
                    .map(|id| (id, p.level))
            })
            .collect();
        szczeble.sort_unstable();
        szczeble.dedup();

        let q = b.quote();
        let dokladka = self.cfg.pending_relot_topup;

        for (id, poziom) in szczeble {
            let na_szczeblu: Vec<Szczebel> = b
                .pendings()
                .iter()
                .filter(|p| p.basket == Some(id) && p.level == poziom && !p.frozen)
                .map(|p| Szczebel {
                    t: p.ticket,
                    vol: p.volume,
                    topup: p.is_topup,
                    kind: p.kind,
                    price: p.price,
                    sl: p.sl,
                    tp: p.tp,
                    touch: p.is_toucher,
                    com: p.comment.clone(),
                })
                .collect();
            if na_szczeblu.is_empty() {
                continue;
            }
            self.stats.relot_szczebli += 1;
            let sztuki = na_szczeblu.iter().filter(|x| !x.topup).count().max(1) as f64;
            let cel_plaski = amount(sztuki * cel);
            let cel_planu = plany.get(&id).map(|lv| {
                let v = lv
                    .iter()
                    .find(|(l, _)| *l == poziom)
                    .map(|(_, v)| *v)
                    .unwrap_or(0.0);
                amount(sztuki * v)
            });
            let mut cel_szczebla = if self.cfg.pending_relot_wg_planu {
                match cel_planu {
                    Some(v) => v,
                    None => continue,
                }
            } else {
                cel_plaski
            };
            if crate::lot_growth::enabled(&self.cfg) {
                let proto=&na_szczeblu[0];
                cel_szczebla=match self.growth_allocated_volume(b,Some(id),poziom,
                    proto.kind.side(),proto.price,proto.sl,cel_szczebla/sztuki,false)
                    .and_then(|v|self.final_open_volume(b,v)) {
                    Ok(v)=>v*sztuki,Err(_)=>continue,
                };
            }
            let suma = amount(na_szczeblu.iter().map(|x| x.vol).sum::<f64>());
            if let Some(p) = cel_planu {
                self.stats.relot_rozjazd_lotow += (cel_plaski - p).abs();
            }
            let roznica = cel_szczebla - suma;
            if roznica.abs() < delta_epsilon {
                continue;
            }

            if roznica < 0.0 {
                self.stats.relot_down_zdarzen += 1;
                self.stats.relot_down_lotow += -roznica;
                if let Some(p) = cel_planu {
                    if p >= suma - delta_epsilon {
                        self.stats.relot_down_bez_spadku += 1;
                    }
                }
                if !self.cfg.pending_relot_down {
                    continue;
                }
            } else {
                self.stats.relot_up_zdarzen += 1;
                self.stats.relot_up_lotow += roznica;
                if let Some(p) = cel_planu {
                    if cel_szczebla > p + delta_epsilon {
                        self.stats.relot_up_ponad_plan += 1;
                    }
                }
                let wolno_w_gore = self.cfg.pending_relot_up
                    && self.stats.balance >= self.cfg.pending_relot_up_od_salda;
                if !wolno_w_gore {
                    continue;
                }
                if !self.margines_pozwala(b, self.cfg.ml_min_relot_up) {
                    continue;
                }
            }

            let wzor = &na_szczeblu[0];
            let ref_px = if wzor.kind.side() == Side::Buy {
                q.ask
            } else {
                q.bid
            };
            let dyst = (ref_px - wzor.price).abs();
            self.stats.relot_prob += 1;
            self.stats.relot_dyst_suma += dyst;
            if self.stats.relot_dyst_min == 0.0 || dyst < self.stats.relot_dyst_min {
                self.stats.relot_dyst_min = dyst;
            }
            if dyst < 1.0 {
                self.stats.relot_blisko += 1;
            }

            if roznica < 0.0 {
                let mut nadmiar = -roznica;
                for s in na_szczeblu.iter().filter(|x| x.topup) {
                    if nadmiar < delta_epsilon {
                        break;
                    }
                    cien::z(cakt::A_ZYCIE_ZLEC, s.t, czr::Z_RELOT_KASUJ, 0);
                    if b.cancel_pending(s.t).is_ok() {
                        let tt = s.t;
                        if let Some(bk) = self.basket_mut(id) {
                            bk.pendings.retain(|x| *x != tt);
                        }
                        nadmiar -= s.vol;
                        self.stats.relot_udane += 1;
                    }
                }
                if nadmiar >= delta_epsilon {
                    if let Some(s) = na_szczeblu.iter().find(|x| !x.topup) {
                        let nowy = self.wolumen_zlecenia(s.vol - nadmiar);
                        let nowy = if v2 {
                            match self.final_open_volume(b, nowy) {
                                Ok(v) => v,
                                Err(_) => continue,
                            }
                        } else {
                            nowy
                        };
                        cien::z(cakt::A_KSZTALT_ZLEC, s.t, czr::Z_RELOT_ZMNIEJSZ, 0);
                        if (nowy - s.vol).abs() >= delta_epsilon && b.cancel_pending(s.t).is_ok() {
                            let tt = s.t;
                            if let Some(bk) = self.basket_mut(id) {
                                bk.pendings.retain(|x| *x != tt);
                            }
                            match self.place_pending_order_allocated(
                                b,
                                PendingReq {
                                    kind: s.kind,
                                    volume: nowy,
                                    price: s.price,
                                    sl: s.sl,
                                    tp: s.tp,
                                    basket: Some(id),
                                    level: poziom,
                                    is_toucher: s.touch,
                                    is_topup: false,
                                    no_market_fallback: false,
                                    comment: s.com.clone(),
                                },
                                true,
                            ) {
                                Ok(n) => {
                                    if let Some(bk) = self.basket_mut(id) {
                                        bk.pendings.push(n);
                                    }
                                    self.stats.relot_udane += 1;
                                }
                                Err(e) => {
                                    self.stats.relot_odmowy += 1;
                                    self.basket_note(
                                        id,
                                        ts,
                                        format!(
                                            "ZMNIEJSZANIE LOTA: szczebel UTRACONY - broker \
                                             odrzucil zlozenie {nowy:.2} lota: {e:?}"
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
                continue;
            }

            if dokladka {
                let vol = self.wolumen_zlecenia(roznica);
                if !v2 && vol < 0.01 {
                    continue;
                }
                let s = &na_szczeblu[0];
                cien::z(cakt::A_KSZTALT_ZLEC, s.t, czr::Z_RELOT_DOSTAW, 0);
                match self.place_pending_order_allocated(
                    b,
                    PendingReq {
                        kind: s.kind,
                        volume: vol,
                        price: s.price,
                        sl: s.sl,
                        tp: s.tp,
                        basket: Some(id),
                        level: poziom,
                        is_toucher: s.touch,
                        is_topup: true,
                        no_market_fallback: false,
                        comment: s.com.clone(),
                    },
                    true,
                ) {
                    Ok(n) => {
                        if let Some(bk) = self.basket_mut(id) {
                            bk.pendings.push(n);
                        }
                        self.stats.relot_udane += 1;
                    }
                    Err(_) => self.stats.relot_odmowy += 1,
                }
            } else {
                if let Some(s) = na_szczeblu.iter().find(|x| !x.topup) {
                    let jednostka = if crate::lot_growth::enabled(&self.cfg) {
                        cel_szczebla / sztuki.max(1.0)
                    } else if self.cfg.pending_relot_wg_planu {
                        if v2 {
                            cel_szczebla / sztuki.max(1.0)
                        } else {
                            (cel_szczebla / sztuki.max(1.0)).max(0.01)
                        }
                    } else {
                        if v2 {
                            cel
                        } else {
                            cel.max(0.01)
                        }
                    };
                    let baza = amount(jednostka);
                    let baza = if v2 {
                        match self.final_open_volume(b, baza) {
                            Ok(v) => v,
                            Err(_) => continue,
                        }
                    } else {
                        baza
                    };
                    if (baza - s.vol).abs() < delta_epsilon {
                        continue;
                    }
                    cien::z(cakt::A_KSZTALT_ZLEC, s.t, czr::Z_RELOT_PRZESTAW, 0);
                    if b.cancel_pending(s.t).is_err() {
                        continue;
                    }
                    let tt = s.t;
                    if let Some(bk) = self.basket_mut(id) {
                        bk.pendings.retain(|x| *x != tt);
                    }
                    match self.place_pending_order_allocated(
                        b,
                        PendingReq {
                            kind: s.kind,
                            volume: baza,
                            price: s.price,
                            sl: s.sl,
                            tp: s.tp,
                            basket: Some(id),
                            level: poziom,
                            is_toucher: s.touch,
                            is_topup: false,
                            no_market_fallback: false,
                            comment: s.com.clone(),
                        },
                        true,
                    ) {
                        Ok(n) => {
                            if let Some(bk) = self.basket_mut(id) {
                                bk.pendings.push(n);
                            }
                            self.stats.relot_udane += 1;
                        }
                        Err(e) => {
                            self.stats.relot_odmowy += 1;
                            self.basket_note(
                                id,
                                ts,
                                format!(
                                    "PRZELICZENIE LOTA: szczebel UTRACONY - broker odrzucil \
                                     ponowne zlozenie {baza:.2} lota: {e:?}"
                                ),
                            );
                        }
                    }
                }
            }
        }
    }


    #[inline]
    fn dzwignia_efektywna(&self, acc: &Account) -> f64 {
        if self.cfg.konto_dzwignia > 0.0 {
            self.cfg.konto_dzwignia
        } else {
            acc.leverage.max(1) as f64
        }
    }

    fn poziom_marginesu<B: Broker>(&self, b: &B) -> (Option<f64>, Option<f64>) {
        let acc = b.account();
        let eq = acc.equity;
        if eq <= 0.0 {
            return (Some(0.0), Some(0.0));
        }
        let lev = self.dzwignia_efektywna(&acc);
        let mar = |vol: f64, px: Px| vol * XAU_CONTRACT * px / lev;
        let m_poz: f64 = b
            .positions()
            .iter()
            .chain(b.ukryte_pozycje())
            .map(|p| mar(p.volume, p.open_price))
            .sum();
        let m_pend: f64 = b
            .pendings()
            .iter()
            .chain(b.ukryte_zlecenia())
            .filter(|p| !p.frozen)
            .map(|p| mar(p.volume, p.price))
            .sum();
        let teraz = if m_poz > 0.0 {
            Some(eq / m_poz * 100.0)
        } else {
            None
        };
        let razem = m_poz + m_pend;
        let docelowy = if razem > 0.0 {
            Some(eq / razem * 100.0)
        } else {
            None
        };
        (teraz, docelowy)
    }

    #[inline]
    fn margines_pozwala<B: Broker>(&self, b: &B, prog: f64) -> bool {
        if prog <= 0.0 {
            return true;
        }
        let (teraz, docelowy) = self.poziom_marginesu(b);
        let ml = if self.cfg.ml_licz_wiszace {
            docelowy
        } else {
            teraz
        };
        match ml {
            None => true, // brak ekspozycji = poziom nieskończony
            Some(x) => x > prog,
        }
    }


    #[inline]
    fn ea_osie_a(&self) -> bool {
        self.cfg.ea_enabled || self.tryb_auto_ea
    }

    fn ea_sufit_jednostek<B: Broker>(&mut self, b: &B) -> Option<crate::ea::SufitEa> {
        if !self.ea_osie_a() {
            return None;
        }
        let a1 = self.cfg.ea_lot_z_wolnego_marginesu;
        let a3 = self.cfg.ea_redukcja_przy_zageszczeniu;
        let a4 = !matches!(self.cfg.ea_stan_dnia, crate::settings::EaStanDnia::Off)
            && self.ea_a.dzien_uzbrojony(self.cfg.ea_stan_dnia_prog_sl);
        if a1 <= 0.0 && a3 <= 0.0 && !a4 {
            return None;
        }

        let mut jednostki_max = u32::MAX;
        if a1 > 0.0 {
            let acc = b.account();
            let q = b.quote();
            let lev = self.dzwignia_efektywna(&acc);
            let px = (q.bid + q.ask) / 2.0;
            let lot = self.lot_size(self.podstawa_lota()).max(self.cfg.lot_min);
            let margines_jednostki = lot * XAU_CONTRACT * px / lev.max(1.0);
            if margines_jednostki > 0.0 {
                let budzet = acc.free_margin.max(0.0) * a1 / 100.0;
                let ile = (budzet / margines_jednostki).floor();
                jednostki_max = if ile >= u32::MAX as f64 {
                    u32::MAX
                } else {
                    ile.max(0.0) as u32
                };
            }
            let limit = self.max_open_positions_eff();
            if limit > 0 {
                let zajete = b.positions().len() as u32;
                jednostki_max = jednostki_max.min(limit.saturating_sub(zajete));
            }
            self.ea_a.sufit_min = Some(match self.ea_a.sufit_min {
                Some(m) => m.min(jednostki_max),
                None => jednostki_max,
            });
        }

        let mut mult = 1.0_f64;
        if a3 > 0.0 {
            let zywe = self.baskets.iter().filter(|x| x.alive()).count() as f64;
            let podloga = self.cfg.ea_zageszczenie_podloga.clamp(0.0, 1.0);
            mult *= (1.0 - a3 * zywe).clamp(podloga, 1.0);
        }
        if a4 {
            mult *= self.cfg.ea_stan_dnia_jednostki_mult;
        }

        Some(crate::ea::SufitEa::nowy(jednostki_max, mult))
    }

    fn ea_dokladki_wolno<B: Broker>(&mut self, b: &B, q: &Quote, id: u32) -> bool {
        if self.basket_exit_pending(id) {
            return false;
        }
        if !self.ea_osie_a() {
            return true;
        }
        let prog = self.cfg.ea_stop_dokladek_przy_stracie;
        let a4 = !matches!(self.cfg.ea_stan_dnia, crate::settings::EaStanDnia::Off)
            && self.ea_a.dzien_uzbrojony(self.cfg.ea_stan_dnia_prog_sl);
        if prog <= 0.0 && !a4 {
            return true;
        }
        if a4 {
            self.ea_a.weta_a4 += 1;
            return false;
        }

        let mut fl = 0.0;
        for p in b.positions() {
            if p.basket == Some(id) {
                fl += p.profit_usd(q);
            }
        }
        let x = match self.cfg.ea_state_src {
            EaStateSrc::FloatPctEquity => {
                let e = b.account().equity;
                if e != 0.0 {
                    fl / e * 100.0
                } else {
                    0.0
                }
            }
            EaStateSrc::FloatR => {
                let r = self.basket(id).map(|k| k.risk_initial_usd).unwrap_or(0.0);
                if r > 0.0 {
                    fl / r
                } else {
                    0.0
                }
            }
        };

        let powrot = if self.cfg.ea_stop_dokladek_powrot > 0.0 {
            self.cfg.ea_stop_dokladek_powrot
        } else {
            prog
        };
        if self.ea_a.stop_dokladek.contains(&id) {
            if x >= -powrot {
                self.ea_a.stop_dokladek.remove(&id);
                return true;
            }
            self.ea_a.weta_a2 += 1;
            return false;
        }
        if x <= -prog {
            self.ea_a.stop_dokladek.insert(id);
            self.ea_a.weta_a2 += 1;
            return false;
        }
        true
    }

    fn redukuj_ekspozycje<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        if self.cfg.expo_cap_pct <= 0.0 && self.cfg.expo_cap_ml_pct <= 0.0 {
            return;
        }
        let gap = (self.cfg.expo_cap_s.max(0.0) * 1000.0) as i64;
        if gap > 0 && ts - self.last_expo < gap {
            return;
        }
        self.last_expo = ts;

        let acc = b.account();
        let eq = acc.equity;
        if eq <= 0.0 {
            return;
        }
        let lev = self.dzwignia_efektywna(&acc);
        let mar = |vol: f64, px: Px| vol * XAU_CONTRACT * px / lev;

        let m_poz: f64 = b
            .positions()
            .iter()
            .chain(b.ukryte_pozycje())
            .map(|p| mar(p.volume, p.open_price))
            .sum();
        let m_pend: f64 = b
            .pendings()
            .iter()
            .chain(b.ukryte_zlecenia())
            .filter(|p| !p.frozen)
            .map(|p| mar(p.volume, p.price))
            .sum();
        let razem = m_poz + m_pend;

        let pct = razem / eq * 100.0;
        if pct > self.stats.expo_max_pct {
            self.stats.expo_max_pct = pct;
        }

        if self.cfg.expo_cap_ml_pct > 0.0 {
            let mut zadzialalo = false;
            loop {
                let m: f64 = b
                    .positions()
                    .iter()
                    .chain(b.ukryte_pozycje())
                    .map(|p| mar(p.volume, p.open_price))
                    .sum();
                if m <= 0.0 {
                    break;
                }
                let e = b.account().equity;
                if e / m * 100.0 >= self.cfg.expo_cap_ml_pct {
                    break;
                }
                let q2 = b.quote();
                let Some((_, t)) = b
                    .positions()
                    .iter()
                    .filter(|p| !p.frozen)
                    .filter(|p| {
                        !p.basket
                            .map(|id| self.basket_exit_pending(id))
                            .unwrap_or(false)
                    })
                    .map(|p| (p.profit_usd(&q2), p.ticket))
                    .min_by(|x, y| {
                        x.0.partial_cmp(&y.0)
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then(x.1.cmp(&y.1))
                    })
                else {
                    break;
                };
                cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_REDUKCJA_POZ, 0);
                if b.close_position(t, CloseReason::Manual).is_err() {
                    break;
                }
                for bk in self.baskets.iter_mut() {
                    bk.tickets.retain(|x| *x != t);
                }
                self.stats.expo_poz_domkniete += 1;
                zadzialalo = true;
            }
            if zadzialalo {
                self.stats.expo_zdarzen += 1;
            }
        }

        if self.cfg.expo_cap_pct <= 0.0 {
            return;
        }
        let acc = b.account();
        let eq = acc.equity;
        if eq <= 0.0 {
            return;
        }
        let m_poz: f64 = b
            .positions()
            .iter()
            .chain(b.ukryte_pozycje())
            .map(|p| mar(p.volume, p.open_price))
            .sum();
        let m_pend: f64 = b
            .pendings()
            .iter()
            .chain(b.ukryte_zlecenia())
            .filter(|p| !p.frozen)
            .map(|p| mar(p.volume, p.price))
            .sum();
        let razem = m_poz + m_pend;
        let limit = eq * self.cfg.expo_cap_pct / 100.0;
        if razem <= limit {
            return;
        }
        self.stats.expo_zdarzen += 1;
        let mut nadmiar = razem - limit;

        let q = b.quote();
        let mid = (q.bid + q.ask) / 2.0;
        let mut ofiary: Vec<(f64, Ticket, f64, f64, Option<u32>, i32)> = b
            .pendings()
            .iter()
            .filter(|p| !p.frozen)
            .filter(|p| {
                !p.basket
                    .map(|id| self.basket_exit_pending(id))
                    .unwrap_or(false)
            })
            .map(|p| {
                (
                    (p.price - mid).abs(),
                    p.ticket,
                    mar(p.volume, p.price),
                    p.volume,
                    p.basket,
                    p.level,
                )
            })
            .collect();
        ofiary.sort_by(|x, y| {
            y.0.partial_cmp(&x.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(x.1.cmp(&y.1))
        });

        for (_, t, m, vol, bid, lvl) in ofiary {
            if nadmiar <= 0.0 {
                break;
            }
            let exposure_before = limit + nadmiar;
            cien::z(cakt::A_ZYCIE_ZLEC, t, czr::Z_REDUKCJA_PEND, 0);
            if b.cancel_pending(t).is_err() {
                continue;
            }
            nadmiar -= m;
            self.stats.expo_pend_skasowane += 1;
            self.stats.expo_lotow += vol;
            for bk in self.baskets.iter_mut() {
                bk.pendings.retain(|x| *x != t);
            }
            if let Some(id) = bid {
                if let Some(bk) = self.baskets.iter_mut().find(|x| x.id == id) {
                    if let Some(g) = bk.levels.iter_mut().find(|g| g.level == lvl) {
                        g.cancelled = true;
                    }
                }
            }
            if self.journal.wants(EventLevel::Warn) {
                let snap = self.jsnap(b);
                self.journal.push(
                    Ev::new(
                        ts,
                        EventLevel::Warn,
                        EventCategory::Account,
                        EventKind::PendingCancelled,
                    )
                    .text(format!(
                        "expo_cap_pct {:.4}%: anulowano ticket {t} koszyka {} poziom {lvl}; \
                         ekspozycja {:.4} -> {:.4} $, limit {:.4} $",
                        self.cfg.expo_cap_pct,
                        bid.map(|x| format!("B{x}"))
                            .unwrap_or_else(|| "bez koszyka".into()),
                        exposure_before,
                        (limit + nadmiar.max(0.0)),
                        limit,
                    ))
                    .reason(RejectCode::ExposureCap)
                    .ticket(t)
                    .basket_opt(bid)
                    .market(snap)
                    .put_f("expo_cap_pct", self.cfg.expo_cap_pct)
                    .put_f("equity_basis", eq)
                    .put_f("exposure_before", exposure_before)
                    .put_f("cancelled_margin", m)
                    .put_f("exposure_after", limit + nadmiar.max(0.0))
                    .put_f("exposure_limit", limit)
                    .put_f("cancelled_volume", vol)
                    .put("level", lvl as i64)
                    .put("decision_schema", 1_u64)
                    .build(),
                );
            }
        }

        if nadmiar <= 0.0 {
            return;
        }
        self.stats.expo_niedosyt += 1;
        if !self.cfg.expo_cap_close {
            return;
        }

        let mut poz: Vec<(f64, Ticket, f64)> = b
            .positions()
            .iter()
            .filter(|p| !p.frozen)
            .filter(|p| {
                !p.basket
                    .map(|id| self.basket_exit_pending(id))
                    .unwrap_or(false)
            })
            .map(|p| (p.profit_usd(&q), p.ticket, mar(p.volume, p.open_price)))
            .collect();
        poz.sort_by(|x, y| {
            x.0.partial_cmp(&y.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(x.1.cmp(&y.1))
        });
        for (_, t, m) in poz {
            if nadmiar <= 0.0 {
                break;
            }
            cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_REDUKCJA_POZ_LEWAR, 0);
            if b.close_position(t, CloseReason::Manual).is_ok() {
                nadmiar -= m;
                self.stats.expo_poz_domkniete += 1;
                for bk in self.baskets.iter_mut() {
                    bk.tickets.retain(|x| *x != t);
                }
            }
        }
    }


    fn limit_trzymania_runnera<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        let ts = q.ts;
        if self.cfg.riskfree_runner_max_hold_min > 0.0 {
            let max_age = (self.cfg.riskfree_runner_max_hold_min * 60_000.0) as i64;
            let zabezpieczone: HashMap<u32, Ts> = self
                .baskets
                .iter()
                .filter(|x| x.secured && x.secured_ts > 0)
                .filter(|x| !self.basket_exit_pending(x.id))
                .filter(|x| !self.cfg.runner_max_hold_rule_only || x.secured_by_rule)
                .map(|x| (x.id, x.secured_ts))
                .collect();
            let przeterminowane: Vec<Ticket> = b
                .positions()
                .iter()
                .filter(|p| !p.frozen)
                .filter(|p| {
                    p.basket
                        .and_then(|id| zabezpieczone.get(&id).copied())
                        .map(|od| ts - od > max_age)
                        .unwrap_or(false)
                })
                .map(|p| p.ticket)
                .collect();
            for t in przeterminowane {
                let wiek_min = b
                    .find_position(t)
                    .and_then(|p| p.basket)
                    .and_then(|id| zabezpieczone.get(&id).copied())
                    .map(|od| (ts - od) as f64 / 60_000.0)
                    .unwrap_or(0.0);
                let koszyk = b.find_position(t).and_then(|p| p.basket);
                cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_LIMIT_RUNNERA, 0);
                if b.close_position(t, CloseReason::Expired).is_ok() {
                    if let Some(id) = koszyk {
                        self.basket_note(
                            id,
                            ts,
                            format!(
                                "runner domknięty po {wiek_min:.0} min OD UWOLNIENIA — limit {:.0} min \
                                 (przewaga sygnału zmienia znak po ok. 90 min)",
                                self.cfg.riskfree_runner_max_hold_min
                            ),
                        );
                    }
                }
            }
        }
    }

    fn riskfree_pass<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if self.cfg.riskfree_enabled || self.cfg.runner_max_hold_bez_reguly {
            self.limit_trzymania_runnera(b, q);
        }
        if !self.cfg.riskfree_enabled {
            return;
        }
        let ts = q.ts;

        if self.cfg.riskfree_trigger_usd <= 0.0 && self.cfg.riskfree_trigger_r <= 0.0 {
            return;
        }
        let ids: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && !x.secured && !x.tickets.is_empty())
            .filter(|x| !self.basket_exit_pending(x.id))
            .map(|x| x.id)
            .collect();

        for id in ids {
            let (side, tps, stage, zrealizowane) = match self.basket(id) {
                Some(bk) => (bk.side, bk.tps.clone(), bk.tp_stage, bk.realized),
                None => continue,
            };
            let zywe: Vec<Position> = b
                .positions()
                .iter()
                .filter(|p| p.basket == Some(id) && !p.frozen)
                .cloned()
                .collect();
            if zywe.is_empty() {
                continue;
            }

            let otwarte: f64 = zywe.iter().map(|p| p.profit_usd(q)).sum();
            let wynik = zrealizowane + otwarte;
            let ryzyko: f64 = zywe
                .iter()
                .filter_map(|p| {
                    p.sl.map(|s| (p.open_price - s).abs() * XAU_CONTRACT * p.volume)
                })
                .sum();
            let prog_kwota =
                self.cfg.riskfree_trigger_usd > 0.0 && wynik >= self.cfg.riskfree_trigger_usd;
            let prog_r = self.cfg.riskfree_trigger_r > 0.0
                && ryzyko > 0.0
                && wynik >= ryzyko * self.cfg.riskfree_trigger_r;
            if !prog_kwota && !prog_r {
                continue;
            }

            let mut wg_zysku = zywe.clone();
            wg_zysku.sort_by(|a, c| {
                c.profit_usd(q)
                    .partial_cmp(&a.profit_usd(q))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let ile_runnerow = (self.cfg.riskfree_keep_units.max(1) as usize).min(wg_zysku.len());
            let runnery: Vec<Ticket> = wg_zysku
                .iter()
                .take(ile_runnerow)
                .map(|p| p.ticket)
                .collect();
            let do_zamkniecia: Vec<Ticket> = wg_zysku
                .iter()
                .skip(ile_runnerow)
                .map(|p| p.ticket)
                .collect();

            let bank: f64 = wg_zysku
                .iter()
                .skip(ile_runnerow)
                .map(|p| p.profit_usd(q))
                .sum();
            if zrealizowane + bank < 0.0 {
                continue;
            }

            let suma_wol: f64 = zywe.iter().map(|p| p.volume).sum();
            if suma_wol <= 0.0 {
                continue;
            }
            let srednia: Px = zywe.iter().map(|p| p.open_price * p.volume).sum::<f64>() / suma_wol;

            let mut zamkniete = 0usize;
            let mut zabankowane = 0.0;
            for t in &do_zamkniecia {
                cien::z(cakt::A_ZYCIE_POZ, *t, czr::Z_RISKFREE_PASS_ZAMKNIJ, 0);
                if let Ok(z) = b.close_position(*t, CloseReason::RiskFree) {
                    zabankowane += z;
                    zamkniete += 1;
                }
            }
            let zostalo = do_zamkniecia.len() - zamkniete;
            if zostalo > 0 {
                self.book_command_profit_legacy(id, zabankowane);
                self.basket_note(
                    id,
                    ts,
                    format!(
                        "RISK FREE (reguła) PRZERWANY — broker odmówił zamknięcia {zostalo} \
                         z {} pozycji (zabankowano {zabankowane:+.2} $); koszyk NIE jest \
                         zabezpieczony, reguła ponowi na następnym przebiegu",
                        do_zamkniecia.len()
                    ),
                );
                continue;
            }

            let be = srednia + side.sign() * self.cfg.riskfree_be_offset;
            let stops = b.stops_level();
            let mut uzbrojone = 0usize;
            for t in &runnery {
                let (biezacy_sl, biezacy_tp, wlasne_wejscie) = match b.find_position(*t) {
                    Some(p) => (p.sl, p.tp, p.open_price),
                    None => continue,
                };
                let be = if self.cfg.riskfree_runner_stop == RiskFreeRunnerStop::BeOwn {
                    wlasne_wejscie + side.sign() * self.cfg.riskfree_be_offset
                } else {
                    be
                };
                let nowy_tp = match self.cfg.riskfree_runner_target {
                    RiskFreeRunnerTarget::KeepTp => biezacy_tp,
                    RiskFreeRunnerTarget::LastTp => tps.last().copied(),
                    RiskFreeRunnerTarget::NoTpTrailOnly => None,
                    RiskFreeRunnerTarget::NextTp => {
                        tps.get(stage).copied().or_else(|| tps.last().copied())
                    }
                };
                if self.cfg.riskfree_runner_stop == RiskFreeRunnerStop::Off {
                    self.try_modify(b, *t, None, nowy_tp, ts);
                    uzbrojone += 1;
                    if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == *t) {
                        p.is_runner = nowy_tp.is_none();
                        p.vsl = None;
                    }
                    continue;
                }
                let lepszy = biezacy_sl.map(|s| side.better(s, be)).unwrap_or(true);
                if lepszy && sl_is_valid(side, be, q, stops) {
                    self.try_modify(b, *t, Some(be), nowy_tp, ts);
                    uzbrojone += 1;
                } else if nowy_tp != biezacy_tp {
                    self.try_modify(b, *t, biezacy_sl, nowy_tp, ts);
                }
                if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == *t) {
                    p.is_runner = nowy_tp.is_none();
                    if self.cfg.riskfree_runner_stop == RiskFreeRunnerStop::TrailGap {
                        p.peak_pts = (q.exit(p.side) - p.open_price) * p.side.sign();
                        p.last_peak_ts = ts;
                    }
                }
            }

            self.book_command_profit_legacy(id, zabankowane);
            if let Some(bk) = self.basket_mut(id) {
                bk.state = BasketState::RiskFree;
                bk.secured = true;
                bk.secured_ts = ts;
                bk.secured_by_rule = true;
            }
            let powod = if prog_kwota {
                format!(
                    "zysk koszyka {wynik:.2} $ ≥ {:.2} $",
                    self.cfg.riskfree_trigger_usd
                )
            } else {
                format!(
                    "zysk koszyka {wynik:.2} $ ≥ {:.1}R ({:.2} $ przy ryzyku {ryzyko:.2} $)",
                    self.cfg.riskfree_trigger_r,
                    ryzyko * self.cfg.riskfree_trigger_r
                )
            };
            let opis_stopu = match self.cfg.riskfree_runner_stop {
                RiskFreeRunnerStop::Off => "BEZ stopu (tryb Off)".to_string(),
                RiskFreeRunnerStop::BeOwn => {
                    "ze stopem na własnym wejściu każdej warstwy (BeOwn)".to_string()
                }
                _ => format!("ze stopem {be:.2}"),
            };
            self.basket_note(
                id,
                ts,
                format!(
                    "RISK FREE (reguła): {powod} · zabankowano {zamkniete} poz. \
                     {zabankowane:+.2} $ · {uzbrojone} runner(ów) {opis_stopu} \
                     (średnia ważona wejść {srednia:.2})"
                ),
            );
            if self.journal.wants(EventLevel::Info) {
                let snap = self.jsnap(b);
                self.journal.push(
                    Ev::new(ts, EventLevel::Ok, EventCategory::Basket, EventKind::BasketUpdated)
                        .text(format!(
                            "koszyk B{id} uwolniony od ryzyka: {powod}, zabankowano {zabankowane:+.2} $, \
                             runner {opis_stopu}"
                        ))
                        .basket(id)
                        .reason(RejectCode::RiskFreeArmed)
                        .market(snap)
                        .put_f("basket_pl", wynik)
                        .put_f("basket_risk", ryzyko)
                        .put_f("banked", zabankowane)
                        .put("closed", zamkniete as u64)
                        .put("runners", uzbrojone as u64)
                        .put_f("avg_entry", srednia)
                        .put_f("runner_sl", be)
                        .build(),
                );
            }
        }
    }

    fn blisko_strefy<B: Broker>(&self, b: &B, id: u32) -> bool {
        let prog = self.cfg.pending_drop_grace_max_dist;
        if prog <= 0.0 {
            return true;
        }
        let Some(bk) = self.baskets.iter().find(|x| x.id == id) else {
            return true;
        };
        let q = b.quote();
        let cena = match bk.side {
            Side::Buy => q.ask,
            Side::Sell => q.bid,
        };
        let d = if cena > bk.zone_hi {
            cena - bk.zone_hi
        } else if cena < bk.zone_lo {
            bk.zone_lo - cena
        } else {
            0.0
        };
        d <= prog
    }

    fn dokoncz_odroczone_kasowanie<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        if self.cfg.pending_drop_grace_min <= 0.0 {
            return;
        }
        let czekajace: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && x.drop_po_ts > 0 && !self.keep_explicit_pending(x.id))
            .map(|x| x.id)
            .collect();
        let gotowe: Vec<(u32, bool)> = czekajace
            .into_iter()
            .filter_map(|id| {
                let po = self
                    .baskets
                    .iter()
                    .find(|x| x.id == id)
                    .map(|x| x.drop_po_ts);
                let przez_czas = matches!(po, Some(t) if ts >= t);
                if przez_czas || !self.blisko_strefy(b, id) {
                    Some((id, przez_czas))
                } else {
                    None
                }
            })
            .collect();
        for (id, przez_czas) in gotowe {
            let n = self.cancel_pendings_keep(b, id, self.cfg.pending_drop_keep_n as usize);
            if let Some(bk) = self.basket_mut(id) {
                bk.drop_po_ts = 0;
            }
            if n > 0 {
                let powod = if przez_czas {
                    format!(
                        "okno łaski {:.0} min minęło",
                        self.cfg.pending_drop_grace_min
                    )
                } else {
                    format!(
                        "cena dalej niż {:.0} $ od strefy",
                        self.cfg.pending_drop_grace_max_dist
                    )
                };
                self.basket_note(id, ts, format!("{powod} — skasowano {n} limitów"));
            }
        }
    }

    fn expire_old_baskets<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        let limit_wieku = self.basket_max_age_eff();
        if limit_wieku <= 0.0
            && !self
                .baskets
                .iter()
                .any(|x| x.alive() && x.age_limit_min > 0.0)
        {
            return;
        }
        let stare: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && !self.keep_explicit_pending(x.id))
            .filter(|x| {
                let mut lim = f64::INFINITY;
                if limit_wieku > 0.0 {
                    lim = limit_wieku;
                }
                if x.age_limit_min > 0.0 {
                    lim = lim.min(x.age_limit_min);
                }
                let od = if self.cfg.wiek_od_wypelnienia {
                    match x
                        .levels
                        .iter()
                        .filter(|g| g.fill_ts > 0)
                        .map(|g| g.fill_ts)
                        .min()
                    {
                        Some(t) => t,
                        None => return false,
                    }
                } else {
                    x.created_ts
                };
                lim.is_finite() && ts - od > (lim * 60_000.0) as i64
            })
            .map(|x| x.id)
            .collect();
        for id in stare {
            if self.cfg.confirmed_exit_retry {
                self.request_confirmed_exit(b, id, ts, CloseReason::Expired);
                continue;
            }
            let tickety: Vec<Ticket> = b
                .positions()
                .iter()
                .filter(|p| p.basket == Some(id) && !p.frozen)
                .map(|p| p.ticket)
                .collect();
            let mut wynik = 0.0;
            let mut zamkniete = 0usize;
            let ile_bylo = tickety.len();
            for t in tickety {
                cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_EXPIRE_OLD, 0);
                if let Ok(z) = b.close_position(t, CloseReason::Expired) {
                    wynik += z;
                    zamkniete += 1;
                }
            }
            let skasowane = self.cancel_pendings(b, id);
            let wiek = self
                .basket(id)
                .map(|x| (ts - x.created_ts) as f64 / 60_000.0)
                .unwrap_or(0.0);

            let zostalo = ile_bylo - zamkniete;
            self.book_command_profit_legacy(id, wynik);
            if let Some(bk) = self.basket_mut(id) {
                if zostalo == 0 {
                    bk.state = BasketState::Done;
                }
            }
            if zostalo > 0 {
                self.basket_note(
                    id,
                    ts,
                    format!(
                        "wygaśnięcie ODRZUCONE przez brokera — {zostalo} z {ile_bylo} pozycji                          nadal otwarte (zamknięto {zamkniete}); koszyk zostaje żywy i spróbuje                          ponownie. Najczęstsza przyczyna: poziom zamrożenia u brokera"
                    ),
                );
                if self.journal.wants(EventLevel::Warn) {
                    let snap = self.jsnap(b);
                    self.journal.push(
                        Ev::new(ts, EventLevel::Warn, EventCategory::Basket, EventKind::OrderRejected)
                            .text(format!(
                                "koszyk B{id}: broker odmówił zamknięcia {zostalo} z {ile_bylo}                                  pozycji przy wygaśnięciu po {wiek:.0} min"
                            ))
                            .market(snap)
                            .build(),
                    );
                }
                continue;
            }
            if zamkniete == 0 && skasowane == 0 {
                continue;
            }
            self.basket_note(
                id,
                ts,
                format!(
                    "koszyk wygasł po {wiek:.0} min (limit {:.0}) — {zamkniete} poz. {wynik:+.2} $, \
                     skasowano {skasowane} limitów; przewaga sygnału nie żyje tak długo",
                    limit_wieku
                ),
            );
            if self.journal.wants(EventLevel::Warn) {
                let snap = self.jsnap(b);
                self.journal.push(
                    Ev::new(
                        ts,
                        EventLevel::Warn,
                        EventCategory::Basket,
                        EventKind::BasketClosed,
                    )
                    .text(format!(
                        "koszyk B{id} wygasł po {wiek:.0} min — twardy limit wieku {:.0} min",
                        limit_wieku
                    ))
                    .basket(id)
                    .reason(RejectCode::StaleSignal)
                    .market(snap)
                    .put("closed", zamkniete as u64)
                    .put("cancelled", skasowane as u64)
                    .put_f("age_min", wiek)
                    .put_f("realized", wynik)
                    .build(),
                );
            }
        }
    }

    fn enforce_position_limit<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        let limit_poz = self.max_open_positions_eff();
        if !self.cfg.enforce_position_limit_on_fill || limit_poz == 0 {
            return;
        }
        let ile = b.positions().iter().filter(|p| !p.frozen).count();
        if self.cfg.limit_kasuje_tylko_nadmiar {
            self.limit_kasuj_nadmiar(b, ts, limit_poz as usize, ile);
            return;
        }
        if ile < limit_poz as usize {
            return;
        }
        let z_koszykami: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && !x.pendings.is_empty())
            .map(|x| x.id)
            .collect();
        let mut razem = 0usize;
        for id in z_koszykami {
            razem += self.cancel_pendings(b, id);
        }
        if razem == 0 {
            return;
        }
        if self.journal.wants(EventLevel::Warn) {
            let snap = self.jsnap(b);
            self.journal.push(
                Ev::new(
                    ts,
                    EventLevel::Warn,
                    EventCategory::Order,
                    EventKind::PendingCancelled,
                )
                .text(format!(
                    "limit pozycji osiągnięty ({ile}/{}) — skasowano {razem} \
                         niewypełnionych zleceń, żeby limit obowiązywał także po wypełnieniach",
                    limit_poz
                ))
                .reason(RejectCode::MaxOpenPositions)
                .market(snap)
                .put("open", ile as u64)
                .put("cancelled", razem as u64)
                .build(),
            );
        }
    }

    fn limit_kasuj_nadmiar<B: Broker>(&mut self, b: &mut B, ts: Ts, limit: usize, ile: usize) {
        let moje: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive())
            .filter(|x| !self.basket_exit_pending(x.id))
            .map(|x| x.id)
            .collect();
        let q = b.quote();
        let mid = (q.bid + q.ask) / 2.0;
        let wiszace: Vec<(f64, Ticket, u32, i32)> = b
            .pendings()
            .iter()
            .filter(|o| !o.frozen)
            .filter_map(|o| {
                o.basket
                    .filter(|id| moje.contains(id))
                    .map(|id| ((o.price - mid).abs(), o.ticket, id, o.level))
            })
            .collect();
        let luz = limit.saturating_sub(ile);

        if wiszace.len() <= luz {
            let wolne = luz - wiszace.len();
            if wolne == 0 || self.limit_cancelled.is_empty() {
                return;
            }
            let baskets = &self.baskets;
            self.limit_cancelled
                .retain(|(id, _)| baskets.iter().any(|x| x.id == *id && x.alive()));
            let mut odznaczone = 0usize;
            while odznaczone < wolne {
                let Some((id, lvl)) = self.limit_cancelled.first().copied() else {
                    break;
                };
                self.limit_cancelled.remove(0);
                if let Some(bk) = self.baskets.iter_mut().find(|x| x.id == id) {
                    if let Some(g) = bk
                        .levels
                        .iter_mut()
                        .find(|g| g.level == lvl && g.cancelled && !g.filled && g.fill_ts == 0)
                    {
                        g.cancelled = false;
                        odznaczone += 1;
                    }
                }
            }
            if odznaczone > 0 {
                self.log(
                    ts,
                    2,
                    format!(
                        "limit pozycji ma luz ({ile}+{}/{limit}) — odznaczono {odznaczone} \
                         szczebli skasowanych limitem",
                        wiszace.len()
                    ),
                );
            }
            return;
        }

        let mut ofiary = wiszace;
        ofiary.sort_by(|x, y| {
            y.0.partial_cmp(&x.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(x.1.cmp(&y.1))
        });
        let nadmiar = ofiary.len() - luz;
        let mut skasowane = 0usize;
        for (_, t, id, lvl) in ofiary.into_iter().take(nadmiar) {
            cien::z(cakt::A_ZYCIE_ZLEC, t, czr::Z_LIMIT_KASUJ_NADMIAR, 0);
            if b.cancel_pending(t).is_err() {
                continue;
            }
            skasowane += 1;
            if let Some(bk) = self.baskets.iter_mut().find(|x| x.id == id) {
                bk.pendings.retain(|x| *x != t);
                if let Some(g) = bk.levels.iter_mut().find(|g| g.level == lvl) {
                    if g.fill_ts == 0 {
                        g.cancelled = true;
                    }
                }
            }
            if !self.limit_cancelled.contains(&(id, lvl)) {
                self.limit_cancelled.push((id, lvl));
            }
        }
        if skasowane == 0 {
            return;
        }
        if self.journal.wants(EventLevel::Warn) {
            let snap = self.jsnap(b);
            self.journal.push(
                Ev::new(
                    ts,
                    EventLevel::Warn,
                    EventCategory::Order,
                    EventKind::PendingCancelled,
                )
                .text(format!(
                    "limit pozycji: {ile} pozycji + wiszące ponad {limit} — skasowano \
                         {skasowane} zleceń nadmiaru (najdalsze od ceny), reszta zostaje"
                ))
                .reason(RejectCode::MaxOpenPositions)
                .market(snap)
                .put("open", ile as u64)
                .put("cancelled", skasowane as u64)
                .build(),
            );
        }
    }

    fn reject_fast_filled_baskets<B: Broker>(&mut self, b: &mut B, ts: Ts) {
        if self.cfg.fast_fill_reject_s <= 0.0 {
            return;
        }
        let potrzeba = self.cfg.fast_fill_layers.max(2) as usize;
        let prog = (self.cfg.fast_fill_reject_s * 1000.0) as i64;

        let ocenione: Vec<(u32, i64, bool)> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && !x.tempo_checked)
            .filter_map(|x| {
                let mut czasy: Vec<Ts> = x
                    .levels
                    .iter()
                    .map(|l| l.fill_ts)
                    .filter(|t| *t > 0)
                    .collect();
                if czasy.len() < potrzeba {
                    return None;
                }
                czasy.sort_unstable();
                let rozpietosc = czasy[potrzeba - 1] - czasy[0];
                Some((x.id, rozpietosc, rozpietosc < prog))
            })
            .collect();
        for (id, _, przelot) in &ocenione {
            self.regime_hist.push((ts, *przelot));
            if let Some(bk) = self.basket_mut(*id) {
                bk.tempo_checked = true;
            }
        }
        if self.regime_hist.len() > 100 {
            let ile = self.regime_hist.len() - 100;
            self.regime_hist.drain(..ile);
        }
        let szybkie: Vec<(u32, i64)> = ocenione
            .into_iter()
            .filter(|(_, _, p)| *p)
            .map(|(i, r, _)| (i, r))
            .collect();

        for (id, rozpietosc) in szybkie {
            if self.basket_exit_pending(id) {
                continue;
            }
            let sek = rozpietosc as f64 / 1000.0;

            let miekki = self.kap_f(
                self.cfg.fast_fill_soft_age_min,
                self.cfg.fast_fill_soft_age_min_small,
                self.cfg.fast_fill_soft_age_min_small_mult,
            );
            if miekki > 0.0 {
                let skrocone = miekki;
                let skasowane = self.cancel_pendings(b, id);
                if let Some(bk) = self.basket_mut(id) {
                    bk.age_limit_min = skrocone;
                    bk.tempo_fast = true;
                }
                self.basket_note(
                    id,
                    ts,
                    format!(
                        "FILTR TEMPA (miękki): {potrzeba} warstw w {sek:.1} s (próg {:.0} s) \
                         — cena przeleciała przez strefę; siatka skasowana ({skasowane} \
                         limitów), życie skrócone do {skrocone:.0} min, pozycje zostają",
                        self.cfg.fast_fill_reject_s
                    ),
                );
                continue;
            }

            if self.cfg.confirmed_exit_retry {
                if let Some(bk) = self.basket_mut(id) {
                    bk.tempo_fast = true;
                }
                self.request_confirmed_exit(b, id, ts, CloseReason::Expired);
                continue;
            }
            let tickety: Vec<Ticket> = b
                .positions()
                .iter()
                .filter(|p| p.basket == Some(id) && !p.frozen)
                .map(|p| p.ticket)
                .collect();
            let ile_bylo = tickety.len();
            let mut wynik = 0.0;
            let mut zamkniete = 0usize;
            for t in tickety {
                cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_REJECT_FAST_FILLED, 0);
                if let Ok(z) = b.close_position(t, CloseReason::Expired) {
                    wynik += z;
                    zamkniete += 1;
                }
            }
            let skasowane = self.cancel_pendings(b, id);
            let zostalo = ile_bylo - zamkniete;
            self.book_command_profit_legacy(id, wynik);
            if let Some(bk) = self.basket_mut(id) {
                if zostalo == 0 {
                    bk.state = BasketState::Done;
                }
                bk.tempo_fast = true;
            }
            if zostalo > 0 {
                self.basket_note(
                    id,
                    ts,
                    format!(
                        "odrzut tempa PRZERWANY — broker odmówił zamknięcia {zostalo} \
                         z {ile_bylo} pozycji; koszyk zostaje żywy (pozycje pilnowane dalej)"
                    ),
                );
            }
            if zamkniete == 0 && skasowane == 0 {
                continue;
            }
            let sek = rozpietosc as f64 / 1000.0;
            self.basket_note(
                id,
                ts,
                format!(
                    "koszyk odrzucony: {potrzeba} warstw w {sek:.1} s (próg {:.0} s) — cena \
                     przeleciała przez strefę zamiast o nią zaczepić; {zamkniete} poz. \
                     {wynik:+.2} $, skasowano {skasowane} limitów",
                    self.cfg.fast_fill_reject_s
                ),
            );
            if self.journal.wants(EventLevel::Warn) {
                let snap = self.jsnap(b);
                self.journal.push(
                    Ev::new(
                        ts,
                        EventLevel::Warn,
                        EventCategory::Basket,
                        EventKind::BasketClosed,
                    )
                    .text(format!(
                        "koszyk B{id} odrzucony — {potrzeba} warstw w {sek:.1} s, \
                             próg {:.0} s",
                        self.cfg.fast_fill_reject_s
                    ))
                    .basket(id)
                    .reason(RejectCode::StaleSignal)
                    .market(snap)
                    .put("closed", zamkniete as u64)
                    .put("cancelled", skasowane as u64)
                    .put_f("fill_span_s", sek)
                    .put_f("realized", wynik)
                    .build(),
                );
            }
        }
    }

    fn fast_addon_sweep<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if self.cfg.fast_addon_move_usd <= 0.0
            || self.cfg.fast_addon_max == 0
            || self.cfg.fast_addon_window_s <= 0.0
        {
            return;
        }
        let ts = q.ts;

        let mut observed_addons: HashMap<u32, u32> = HashMap::new();
        for p in b.positions().iter().filter(|p| p.level == -4) {
            if let Some(id) = p.basket {
                *observed_addons.entry(id).or_insert(0) += 1;
            }
        }
        let mut recovered = Vec::new();
        for bk in &mut self.baskets {
            let observed = observed_addons.get(&bk.id).copied().unwrap_or(0);
            if observed > bk.fast_addons {
                bk.fast_addons = observed;
                recovered.push((bk.id, observed));
            }
        }
        for (id, observed) in recovered {
            self.basket_note(
                id,
                ts,
                format!("DOKŁADKA TEMPOWA: licznik odzyskany z migawki brokera → {observed}"),
            );
        }

        let t0 = ts - (self.cfg.fast_addon_window_s * 1000.0) as i64;
        let mut baza: Option<Px> = None;
        let mut n = 0usize;
        for (t, p) in self.vol_hist.iter().rev() {
            if *t < t0 {
                break;
            }
            baza = Some(*p);
            n += 1;
        }
        if n < 3 {
            return;
        }
        let baza = match baza {
            Some(p) => p,
            None => return,
        };
        let mid = q.mid();

        let prog = self.cfg.fast_addon_move_usd;
        let ostyg = (self.cfg.fast_addon_cooldown_s * 1000.0) as i64;
        let min_stage = self.cfg.fast_addon_min_stage as usize;
        let maks = self.cfg.fast_addon_max;
        let mut kandydaci: Vec<(u32, Side, Option<Px>, Option<Px>, Px, Px)> = Vec::new();
        for bk in self.baskets.iter().filter(|x| x.alive() && x.ma_pozycje()) {
            if self.basket_exit_pending(bk.id) {
                continue;
            }
            if bk.fast_addons >= maks || bk.tp_stage < min_stage {
                continue;
            }
            if ostyg > 0 && bk.last_addon_ts > 0 && ts - bk.last_addon_ts < ostyg {
                continue;
            }
            let ruch = (mid - baza) * bk.side.sign();
            if ruch < prog {
                continue;
            }
            kandydaci.push((
                bk.id,
                bk.side,
                bk.sl,
                bk.tps.last().copied(),
                bk.entry_lo,
                bk.entry_hi,
            ));
        }
        if kandydaci.is_empty() {
            return;
        }

        let limit = self.max_open_positions_eff();
        for (id, side, bsl, tp_ost, sig_lo, sig_hi) in kandydaci {
            if limit > 0 && b.positions().len() >= limit as usize {
                break;
            }
            if self.za_dalsza_krawedzia(side, q.entry(side), sig_lo, sig_hi) {
                continue;
            }
            if !self.margines_pozwala(b, self.cfg.ml_min_fast_addon) {
                break;
            }
            let mut vol = (self.lot_size(self.podstawa_lota())
                * self.cfg.fast_addon_lot_mult.max(0.0))
            .max(self.cfg.lot_min);
            if self.cfg.lot_max > 0.0 {
                vol = vol.min(self.cfg.lot_max);
            }

            if let Some(tp) = tp_ost {
                if !tp_is_valid(side, tp, q, b.stops_level()) {
                    // No request has reached the broker: the slot remains free.
                    // Preserve the preset's attempt cooldown, as in the archived
                    // precheck-free broker-rejection path, before trying again.
                    if let Some(bk)=self.basket_mut(id) {bk.last_addon_ts=ts;}
                    // Deduplicate the note, not the executable opportunity.
                    if self.fast_addon_invalid_tp_note.insert(id,tp.to_bits()) != Some(tp.to_bits()) {
                        self.basket_note(id,ts,format!(
                            "FAST ADDON: TP {tp:.2} is beyond the market; no order sent and capacity remains available"));
                    }
                    continue;
                }
            }
            self.fast_addon_invalid_tp_note.remove(&id);
            cien::z(cakt::A_DOLOZENIE, id as u64, czr::Z_FAST_ADDON, 0);
            let submitted_before=self.order_submission_sequence;
            let res = self.open_market_order(
                b,
                OrderReq {
                    side,
                    volume: vol,
                    sl: self.broker_sl(bsl, side),
                    tp: tp_ost,
                    basket: Some(id),
                    level: -4,
                    is_toucher: false,
                    comment: format!("B{id}"),
                },
            );
            match res {
                Ok(_) => {
                    if let Some(bk) = self.basket_mut(id) {
                        bk.fast_addons += 1;
                        bk.last_addon_ts = ts;
                    }
                    diag_wejscie(
                        ts,
                        id,
                        "dokladka-tempowa",
                        side,
                        sig_lo,
                        sig_hi,
                        q.entry(side),
                        q.entry(side),
                        -4,
                    );
                    self.basket_note(
                        id,
                        ts,
                        format!(
                            "DOKŁADKA TEMPOWA: cena przebiegła {:.2} $ w {:.0} s — \
                             dokładka {vol:.2} lota rynkiem",
                            (mid - baza) * side.sign(),
                            self.cfg.fast_addon_window_s
                        ),
                    );
                }
                Err(e) => {
                    // Only an Engine->Broker submission can leave an uncertain
                    // acknowledgement. A local risk/volume gate sent nothing.
                    if self.order_submission_sequence!=submitted_before {
                        if let Some(bk) = self.basket_mut(id) {
                            bk.fast_addons = bk.fast_addons.saturating_add(1).min(maks);
                            bk.last_addon_ts = ts;
                        }
                        self.basket_note(id, ts, format!("DOKŁADKA TEMPOWA odrzucona: {e:?}"));
                    }
                }
            }
        }
    }

    fn zone_exit_adverse_sweep<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if self.cfg.zone_exit_adverse_s == 0.0 {
            return;
        }
        let lustro = self.cfg.zone_exit_adverse_s < 0.0;
        let prog = (self.cfg.zone_exit_adverse_s.abs() * 1000.0) as i64;
        let mut dojrzale: Vec<u32> = Vec::new();
        for bk in self
            .baskets
            .iter_mut()
            .filter(|x| x.alive() && x.ma_pozycje())
            .filter(|x| !(self.cfg.confirmed_exit_retry && x.pending_exit.is_some()))
        {
            let (lo, hi) = if bk.zone_lo <= bk.zone_hi {
                (bk.zone_lo, bk.zone_hi)
            } else {
                (bk.zone_hi, bk.zone_lo)
            };
            let przeciw = match (bk.side, lustro) {
                (Side::Buy, false) | (Side::Sell, true) => q.bid < lo,
                (Side::Sell, false) | (Side::Buy, true) => q.ask > hi,
            };
            if !przeciw {
                bk.adverse_since = 0;
                continue;
            }
            if bk.adverse_since == 0 {
                bk.adverse_since = q.ts;
                continue;
            }
            if q.ts - bk.adverse_since >= prog {
                dojrzale.push(bk.id);
            }
        }
        let zamykaj = self.cfg.zone_exit_adverse_close;
        for id in dojrzale {
            if self.cfg.confirmed_exit_retry && zamykaj {
                self.request_confirmed_exit(b, id, q.ts, CloseReason::BasketClose);
                continue;
            }
            let mut zamkniete = 0usize;
            let mut ile_bylo = 0usize;
            let mut wynik = 0.0;
            if zamykaj {
                let tickety: Vec<Ticket> = b
                    .positions()
                    .iter()
                    .filter(|p| p.basket == Some(id) && !p.frozen)
                    .map(|p| p.ticket)
                    .collect();
                ile_bylo = tickety.len();
                for t in tickety {
                    cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_ZONE_EXIT_ADVERSE, 0);
                    if let Ok(z) = b.close_position(t, CloseReason::BasketClose) {
                        wynik += z;
                        zamkniete += 1;
                    }
                }
            }
            let skasowane = self.cancel_pendings(b, id);
            let zostalo = ile_bylo - zamkniete;
            self.book_command_profit_legacy(id, wynik);
            if let Some(bk) = self.basket_mut(id) {
                bk.adverse_since = 0;
                if zamykaj && zostalo == 0 {
                    bk.state = BasketState::Done;
                }
            }
            if zostalo > 0 {
                self.basket_note(
                    id,
                    q.ts,
                    format!(
                        "wyjście ze strefy PRZERWANE — broker odmówił zamknięcia {zostalo} \
                         z {ile_bylo} pozycji; koszyk zostaje żywy, ponowienie po {:.0} s",
                        self.cfg.zone_exit_adverse_s.abs()
                    ),
                );
            }
            if zamkniete == 0 && skasowane == 0 {
                continue;
            }
            self.basket_note(
                id,
                q.ts,
                format!(
                    "WYJŚCIE ZE STREFY {}: cena poza strefą {:.0} s \
                     — {zamkniete} poz. {wynik:+.2} $, skasowano {skasowane} limitów",
                    if lustro {
                        "ZA NAMI (kontrola lustrzana)"
                    } else {
                        "PRZECIW"
                    },
                    self.cfg.zone_exit_adverse_s.abs()
                ),
            );
            if self.journal.wants(EventLevel::Warn) {
                let snap = self.jsnap(b);
                self.journal.push(
                    Ev::new(
                        q.ts,
                        EventLevel::Warn,
                        EventCategory::Basket,
                        EventKind::BasketClosed,
                    )
                    .text(format!(
                        "koszyk B{id} — trwałe wyjście ze strefy {} ({:.0} s)",
                        if lustro { "ZA NAMI" } else { "PRZECIW" },
                        self.cfg.zone_exit_adverse_s.abs()
                    ))
                    .basket(id)
                    .reason(RejectCode::StaleSignal)
                    .market(snap)
                    .put("closed", zamkniete as u64)
                    .put("cancelled", skasowane as u64)
                    .put_f("realized", wynik)
                    .build(),
                );
            }
        }
    }

    fn trend_adverse(&self, side: Side, ts: Ts) -> Option<bool> {
        if !self.cfg.trend_filter_enabled || self.cfg.trend_filter_drop_pct <= 0.0 {
            return None;
        }
        let okno = (self.cfg.trend_filter_window_h.max(1.0) * 3_600_000.0) as i64;
        let t0 = ts - okno;
        let teraz = self.price_hist.last()?.1;
        let dawniej = self.price_hist.iter().find(|(t, _)| *t >= t0)?;
        if self
            .price_hist
            .first()
            .map(|(t, _)| *t > t0)
            .unwrap_or(true)
        {
            return None;
        }
        if dawniej.1 <= 0.0 {
            return None;
        }
        let zmiana_pct = (teraz - dawniej.1) / dawniej.1 * 100.0;
        Some(match side {
            Side::Buy => zmiana_pct <= -self.cfg.trend_filter_drop_pct,
            Side::Sell => zmiana_pct >= self.cfg.trend_filter_drop_pct,
        })
    }

    fn sesja_limity_pass<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if self.cfg.sesja_bramka == crate::settings::SesjaBramka::Sygnal || !self.cfg.session_filter
        {
            return;
        }
        let wolno = self.cfg.hours_ok(hour_of(q.ts, self.cfg.session_offset()));
        let ids: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && !x.levels.is_empty())
            .map(|x| x.id)
            .collect();
        for id in ids {
            let wiszace = self.basket(id).map(|x| x.pendings.len()).unwrap_or(0);
            if !wolno && wiszace > 0 {
                let n = self.cancel_pendings(b, id);
                if n > 0 {
                    self.basket_note(
                        id,
                        q.ts,
                        format!(
                            "SESJA USYPIA {n} limitów (poza oknem {})",
                            self.cfg.session_hours
                        ),
                    );
                }
            } else if wolno && wiszace == 0 {
                let n = self.sync_grid(b, id, q.ts, true);
                if n > 0 {
                    self.basket_note(
                        id,
                        q.ts,
                        format!("SESJA BUDZI {n} limitów (okno {})", self.cfg.session_hours),
                    );
                }
            }
        }
    }

    fn rezim_limity_pass<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if !self.cfg.regime_pilnuj_limitow
            || self.cfg.regime_filter == crate::settings::RegimeFilter::Off
        {
            return;
        }
        let ids: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && !x.levels.is_empty())
            .map(|x| x.id)
            .collect();
        for id in ids {
            let (side, poziomy) = match self.basket(id) {
                Some(bk) => (bk.side, bk.levels.clone()),
                None => continue,
            };
            let odniesienie = match side {
                Side::Buy => poziomy
                    .iter()
                    .map(|l| l.price)
                    .fold(f64::INFINITY, f64::min),
                Side::Sell => poziomy
                    .iter()
                    .map(|l| l.price)
                    .fold(f64::NEG_INFINITY, f64::max),
            };
            if !odniesienie.is_finite() {
                continue;
            }
            let wolno = self.regime_ok(side, odniesienie);
            let wiszace = self.basket(id).map(|x| x.pendings.len()).unwrap_or(0);
            if !wolno && wiszace > 0 {
                let n = self.cancel_pendings(b, id);
                if n > 0 {
                    self.basket_note(
                        id,
                        q.ts,
                        format!(
                            "REŻIM USYPIA {n} limitów (próg przy cenie szczebla {odniesienie:.2})"
                        ),
                    );
                }
            } else if wolno && wiszace == 0 {
                let n = self.sync_grid(b, id, q.ts, true);
                if n > 0 {
                    self.basket_note(
                        id,
                        q.ts,
                        format!("REŻIM PRZYWRACA {n} limitów (cena szczebla {odniesienie:.2})"),
                    );
                }
            }
        }
    }

    fn rearm_pass<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if !self.cfg.rearm_grid_on_return {
            return;
        }
        if !self.margines_pozwala(b, self.cfg.ml_min_rearm) {
            return;
        }
        if self.wejscie_zablokowane(b, q.ts) {
            return;
        }
        let odstep = (self.cfg.rearm_min_gap_min.max(0.0) * 60_000.0) as i64;
        let ids: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| {
                if !x.alive()
                    || x.levels.is_empty()
                    || self.basket_exit_pending(x.id)
                    || self.entry_edit_blocks(Some(x.id))
                {
                    return false;
                }
                if x.had_positions {
                    return true;
                }
                if !self.cfg.rearm_bez_pozycji {
                    return false;
                }
                let sufit = self.cfg.rearm_bez_pozycji_max_h;
                if sufit <= 0.0 {
                    return true;
                }
                let wiek_ms = q.ts.saturating_sub(x.created_ts);
                (wiek_ms as f64) <= sufit * 3_600_000.0
            })
            .filter(|x| !self.cfg.rearm_block_after_secured || !x.secured)
            .filter(|x| !self.cfg.spp_blocks_rearm_when_flat || !x.rearm_blocked_by_spp)
            .filter(|x| self.cfg.rearm_max_times == 0 || x.rearms < self.cfg.rearm_max_times)
            .filter(|x| x.last_rearm_ts == 0 || q.ts - x.last_rearm_ts >= odstep)
            .map(|x| x.id)
            .collect();

        for id in ids {
            let (side, lo, hi, sl, zrealizowane, sig_lo, sig_hi, mial_pozycje) =
                match self.basket(id) {
                    Some(bk) => (
                        bk.side,
                        bk.zone_lo,
                        bk.zone_hi,
                        bk.sl,
                        bk.realized,
                        bk.entry_lo,
                        bk.entry_hi,
                        bk.had_positions,
                    ),
                    None => continue,
                };
            let px = q.entry(side);
            if px < lo - 1e-9 || px > hi + 1e-9 {
                continue;
            }
            if self.za_dalsza_krawedzia(side, px, sig_lo, sig_hi) {
                continue;
            }
            if let Some(s) = sl {
                let przebity = match side {
                    Side::Buy => px <= s,
                    Side::Sell => px >= s,
                };
                if przebity {
                    continue;
                }
            }
            let otwarte: f64 = b
                .positions()
                .iter()
                .filter(|p| p.basket == Some(id))
                .map(|p| p.profit_usd(q))
                .sum();
            let wynik = zrealizowane + otwarte;
            let uspiony = !mial_pozycje;
            if !uspiony && wynik < self.cfg.rearm_min_basket_profit {
                continue;
            }

            let audit_rearm = self.journal.wants(EventLevel::Info).then(|| {
                (
                    self.jsnap(b),
                    self.basket(id).map(|bk| bk.secured).unwrap_or(false),
                    b.close_receipt_reconciliation_active(),
                    b.close_receipts_pending(),
                )
            });
            self.begin_rearm_batch(id, q.ts);
            let dostawione = self.sync_grid(b, id, q.ts, false);
            self.finish_rearm_batch(dostawione);
            if dostawione == 0 {
                continue;
            }
            if let Some(bk) = self.basket_mut(id) {
                bk.rearms += 1;
                bk.last_rearm_ts = q.ts;
            }
            let ile = self.basket(id).map(|x| x.rearms).unwrap_or(0);
            self.basket_note(
                id,
                q.ts,
                format!(
                    "siatka przezbrojona ({dostawione} zleceń) — cena wróciła do strefy przy {px:.2}, \
                     koszyk {wynik:+.2} $ (przezbrojenie {ile})"
                ),
            );
            if let Some((snap, secured_before, receipts_active, receipts_pending)) = audit_rearm {
                self.journal.push(
                    Ev::new(q.ts, EventLevel::Info, EventCategory::Order, EventKind::OrderPlaced)
                        .text(format!(
                            "koszyk B{id}: siatka przezbrojona ({dostawione} zleceń) — powrót ceny do strefy \
                             przy wyniku {wynik:+.2} $"
                        ))
                        .basket(id)
                        .reason(RejectCode::GridRearmed)
                        .market(snap)
                        .put("placed", dostawione as u64)
                        .put("rearms", ile as u64)
                        .put_f("basket_pl", wynik)
                        .put_f("price", px)
                        .put("decision_schema", 1_u64)
                        .put("snapshot_phase", "before_rearm")
                        .put("pnl_basis", "basket_realized_plus_position_profit_usd")
                        .put_f("basket_realized_before", zrealizowane)
                        .put_f("basket_open_pl_before", otwarte)
                        .put_f("required_basket_pl", self.cfg.rearm_min_basket_profit)
                        .put("pnl_gate_bypassed_no_previous_fill", uspiony)
                        .put("secured_before", secured_before)
                        .put("rearm_block_after_secured", self.cfg.rearm_block_after_secured)
                        .put("close_receipt_reconcile_active", receipts_active)
                        .put("close_receipts_pending_before", receipts_pending)
                        .put("decision_inputs_f64_bits", serde_json::json!({
                            "bid": format!("{:016x}", q.bid.to_bits()),
                            "ask": format!("{:016x}", q.ask.to_bits()),
                            "realized": format!("{:016x}", zrealizowane.to_bits()),
                            "floating": format!("{:016x}", otwarte.to_bits()),
                            "total": format!("{:016x}", wynik.to_bits()),
                            "threshold": format!("{:016x}", self.cfg.rearm_min_basket_profit.to_bits()),
                        }))
                        .build(),
                );
            }
        }
    }

    fn market_ladder_pass<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if !self.margines_pozwala(b, self.cfg.ml_min_drabina) {
            return;
        }
        if self.cfg.market_entry_mode != MarketEntryMode::Laddered || self.cfg.auto_limit {
            return;
        }
        if self.wejscie_zablokowane(b, q.ts) {
            return;
        }
        let step = self.market_step_eff();
        let ids: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && !x.is_limit && !x.levels.is_empty())
            .filter(|x| !self.basket_exit_pending(x.id))
            .filter(|x| x.levels.iter().any(|g| !g.filled && !g.cancelled))
            .map(|x| x.id)
            .collect();

        for id in ids {
            let (side, sl, last_px) = match self.basket(id) {
                Some(bk) => (bk.side, bk.sl, bk.last_entry_px),
                None => continue,
            };
            let px = q.entry(side);
            if let Some(s) = sl {
                let przebity = match side {
                    Side::Buy => px <= s,
                    Side::Sell => px >= s,
                };
                if przebity {
                    continue;
                }
            }
            if let Some(prev) = last_px {
                if (prev - px) * side.sign() < step {
                    continue;
                }
            }
            if self.sync_grid(b, id, q.ts, false) > 0 {
                self.basket_note(
                    id,
                    q.ts,
                    format!("szczebel rynkowy uwolniony @ {px:.2} (krok {step:.2} $)"),
                );
            }
        }
    }

    fn reentry_pass<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if !self.margines_pozwala(b, self.cfg.ml_min_reentry) {
            return;
        }
        if !self.cfg.reenter_after_tp {
            return;
        }
        if self.wejscie_zablokowane(b, q.ts) {
            return;
        }
        let reenter_lim = self.kap_u(
            self.cfg.reenter_max,
            self.cfg.reenter_max_small,
            self.cfg.reenter_max_small_mult,
        );
        let ids: Vec<u32> = self
            .baskets
            .iter()
            .filter(|x| x.alive() && x.had_positions)
            .filter(|x| !self.basket_exit_pending(x.id))
            .filter(|x| x.tp_stage >= self.cfg.reenter_min_tp_stage)
            .filter(|x| {
                self.cfg.no_reenter_from_stage == 0
                    || x.tp_stage < self.cfg.no_reenter_from_stage as usize
            })
            .filter(|x| !self.cfg.reenter_stop_after_riskfree || !x.secured)
            .filter(|x| reenter_lim == 0 || x.reentries < reenter_lim)
            .filter(|x| {
                self.cfg.reenter_min_return_s <= 0.0
                    || x.last_tp_ts == 0
                    || q.ts - x.last_tp_ts >= (self.cfg.reenter_min_return_s * 1000.0) as i64
            })
            .map(|x| x.id)
            .collect();

        let step = self.market_step_eff();
        let lot = self.lot_size(self.podstawa_lota());

        for id in ids {
            let (side, lo, hi, sl, tps, stage, last_px, sig_lo, sig_hi) = match self.basket(id) {
                Some(bk) => (
                    bk.side,
                    bk.zone_lo,
                    bk.zone_hi,
                    bk.sl,
                    bk.tps.clone(),
                    bk.tp_stage,
                    bk.last_entry_px,
                    bk.entry_lo,
                    bk.entry_hi,
                ),
                None => continue,
            };
            let px = q.entry(side);
            if px < lo - 1e-9 || px > hi + 1e-9 {
                continue;
            }
            if self.za_dalsza_krawedzia(side, px, sig_lo, sig_hi) {
                continue;
            }
            if let Some(s) = sl {
                let breached = match side {
                    Side::Buy => px <= s,
                    Side::Sell => px >= s,
                };
                if breached {
                    continue;
                }
            }
            if let Some(prev) = last_px {
                let moved = (prev - px) * side.sign();
                if moved < step {
                    continue;
                }
            }
            if self.cfg.entry_sl_dist_limit > 0.0 {
                if let Some(s) = sl {
                    if (px - s).abs() > self.cfg.entry_sl_dist_limit {
                        continue;
                    }
                }
            }
            let tp = match self.cfg.tp_schedule {
                TpSchedule::AllRunners => tps.last().copied(),
                _ => tps.get(stage).copied().or_else(|| tps.last().copied()),
            };

            let cap = if self.cfg.reenter_respect_cap {
                self.market_risk_cap(id, b)
            } else {
                None
            };
            let skala = self.market_risk_scale(lot, px, sl, cap);
            let vol = if crate::lot_growth::enabled(&self.cfg) {
                // The remaining-cap check below can still reject this leg.
                (lot * skala).max(self.cfg.lot_min)
            } else if self.cfg.order_volume_contract_v2 {
                lot * skala
            } else {
                round_lot((lot * skala).max(self.cfg.lot_min))
            };
            if let (Some(c), Some(s)) = (cap, sl) {
                let ryzyko = (px - s).abs() * XAU_CONTRACT * vol;
                if ryzyko > c + 1e-9 {
                    self.basket_note(
                        id,
                        q.ts,
                        format!(
                            "dokładka pominięta — budżet ryzyka koszyka wyczerpany \
                             (zostało {c:.2} $, dokładka kosztuje {ryzyko:.2} $)"
                        ),
                    );
                    continue;
                }
            }

            cien::z(cakt::A_DOLOZENIE, id as u64, czr::Z_REENTRY, 0);
            if let Ok(t) = self.open_market_order(
                b,
                OrderReq {
                    side,
                    volume: vol,
                    sl: self.broker_sl(sl, side),
                    tp,
                    basket: Some(id),
                    level: -2,
                    is_toucher: false,
                    comment: format!("B{id}R"),
                },
            ) {
                self.apply_virtual_sl(b, t, sl);
                if let Some(bk) = self.basket_mut(id) {
                    bk.tickets.push(t);
                    bk.reentries += 1;
                    bk.last_entry_px = Some(px);
                    bk.state = BasketState::Working;
                    bk.had_positions = true;
                }
                diag_wejscie(q.ts, id, "re-entry", side, sig_lo, sig_hi, px, px, -2);
                self.basket_note(
                    id,
                    q.ts,
                    format!("RE-ENTRY @ {px:.2} (etap {stage}, powrót do strefy)"),
                );
            }
        }
    }

    fn rev_exit_sweep<B: Broker>(&mut self, b: &mut B, q: &Quote) {
        if self.cfg.rev_exit_range <= 0.0 {
            return;
        }
        let win = if self.cfg.rev_exit_window_min > 0.0 {
            self.cfg.rev_exit_window_min
        } else {
            60.0
        };
        let t0 = q.ts - (win * 60_000.0) as i64;
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        let mut n = 0usize;
        for (t, p) in self.vol_hist.iter().rev() {
            if *t < t0 {
                break;
            }
            lo = lo.min(*p);
            hi = hi.max(*p);
            n += 1;
        }
        if n < 10 {
            return;
        }
        let range = hi - lo;
        if range < self.cfg.rev_exit_range {
            return;
        }

        let mut closed = 0usize;
        let tickets: Vec<Ticket> = b.positions().iter().map(|p| p.ticket).collect();
        for t in tickets {
            let p = match b.find_position(t) {
                Some(p) => p.clone(),
                None => continue,
            };
            if p.frozen
                || p.basket
                    .map(|id| self.basket_exit_pending(id))
                    .unwrap_or(false)
            {
                continue;
            }
            let against = match p.side {
                Side::Buy => hi - q.mid(),
                Side::Sell => q.mid() - lo,
            };
            if against < self.cfg.rev_exit_slope {
                continue;
            }
            if p.profit_pts(q) < self.cfg.rev_exit_profit {
                continue;
            }
            cien::z(cakt::A_ZYCIE_POZ, t, czr::Z_REV_EXIT, 0);
            if b.close_position(t, CloseReason::RevExit).is_ok() {
                closed += 1;
            }
        }
        if closed > 0 {
            self.log(
                q.ts,
                2,
                format!(
                    "REVERSAL-EXIT: zakres {range:.2} $ w {win:.0} min i oddanie przewagi — zbankowano {closed} poz."
                ),
            );
        }
    }


    pub fn entry_gate<B: Broker>(&self, b: &B, ts: Ts) -> Gate {
        self.bramka_wejscia::<B, true>(b, ts)
    }

    #[inline]
    pub fn wejscie_zablokowane<B: Broker>(&self, b: &B, ts: Ts) -> bool {
        !matches!(self.bramka_wejscia::<B, false>(b, ts), Gate::Open)
    }

    fn bramka_wejscia<B: Broker, const MSG: bool>(&self, b: &B, ts: Ts) -> Gate {
        #[inline(always)]
        fn zd<const MSG: bool>(f: impl FnOnce() -> String) -> String {
            if MSG {
                f()
            } else {
                String::new()
            }
        }
        if self.rearm_confirmation_pending() {
            return Gate::Blocked(zd::<MSG>(|| self.rearm_hold_reason().into()), RejectCode::EntryGateBlocked);
        }
        if self.continuation_entry_blocked() {
            return Gate::Halted(zd::<MSG>(|| {
                "CONTINUATION REVIEW: strategy state is unbound or incomplete".into()
            }));
        }
        if let Some(reason) = self.cost_entry_blocked(b) {
            return Gate::Halted(zd::<MSG>(|| format!("COST HOLD: {reason}")));
        }
        if let Some(r) = &self.halted {
            return Gate::Halted(zd::<MSG>(|| r.clone()));
        }
        if b.close_receipts_pending() {
            return Gate::Blocked(
                zd::<MSG>(|| {
                    "oczekiwanie na zaksięgowanie potwierdzonych zamknięć; nowe wejścia wstrzymane"
                        .into()
                }),
                RejectCode::EntryGateBlocked,
            );
        }
        if self.slhit_hamuje(ts) && self.cfg.slhit_pause_lot_mult <= 0.0 {
            return Gate::Blocked(
                zd::<MSG>(|| format!("hamulec SL-HIT: {} stopów kanału w dobie", self.slhit_dnia)),
                RejectCode::SlHitBrake,
            );
        }
        if ts < self.paused_until {
            let left = (self.paused_until - ts) / 60_000;
            return Gate::Blocked(
                zd::<MSG>(|| format!("pauza po serii strat ({left} min)")),
                RejectCode::StreakPause,
            );
        }
        if self.day_stop != i64::MIN && day_of(ts, self.cfg.session_offset()) == self.day_stop {
            return Gate::Blocked(
                zd::<MSG>(|| "doba zamknięta przez strażnika".into()),
                RejectCode::EntryGateBlocked,
            );
        }
        if self.cfg.sesja_bramka != crate::settings::SesjaBramka::Wypelnienie {
            let hour = hour_of(ts, self.cfg.session_offset());
            if !self.cfg.hours_ok(hour) {
                return Gate::Blocked(
                    zd::<MSG>(|| format!("poza sesją ({})", self.cfg.session_hours)),
                    RejectCode::SessionClosed,
                );
            }
        }
        let lot = self.lot_size(self.podstawa_lota());
        if self.cfg.day_target_usd > 0.0 {
            let scale = if self.cfg.day_target_scale_lot {
                (lot / 0.01).max(1.0)
            } else {
                self.cfg.usd_scale(lot)
            };
            if self.stats.equity - self.stats.day_start_equity >= self.cfg.day_target_usd * scale {
                return Gate::Blocked(
                    zd::<MSG>(|| "cel dzienny osiągnięty".into()),
                    RejectCode::EntryGateBlocked,
                );
            }
        }
        if self.cfg.day_target_pct > 0.0 && self.bramka_dnia_czynna() {
            let prog = self.stats.day_start_equity.max(1.0) * self.cfg.day_target_pct / 100.0;
            if self.stats.equity - self.stats.day_start_equity >= prog {
                return Gate::Blocked(
                    zd::<MSG>(|| {
                        format!("cel dzienny {:.2} % osiągnięty", self.cfg.day_target_pct)
                    }),
                    RejectCode::EntryGateBlocked,
                );
            }
        }
        let (bonus_poz, bonus_kosz) = self.bonus_ekspozycji(b);
        let wlasny_poz = match self.max_open_positions_eff() {
            0 => 0,
            n => n + bonus_poz,
        };
        if wlasny_poz > 0 || self.pulapy.max_pozycji > 0 {
            let wlasne = b.positions().len()
                + if self.cfg.exposure_count_pendings {
                    b.pendings().len()
                } else {
                    0
                };
            if wlasny_poz > 0 && wlasne >= wlasny_poz as usize {
                return Gate::Blocked(
                    zd::<MSG>(|| format!("limit ekspozycji ({wlasne})")),
                    RejectCode::MaxOpenPositions,
                );
            }
            if self.pulapy.max_pozycji > 0 {
                let razem = wlasne + self.obce.pozycje as usize;
                if razem >= self.pulapy.max_pozycji as usize {
                    return Gate::Blocked(
                        zd::<MSG>(|| format!("pułap pozycji rachunku ({razem})")),
                        RejectCode::MaxOpenPositions,
                    );
                }
            }
        }
        let wlasny_kosz = match self.max_open_baskets_eff() {
            0 => 0,
            n => n + bonus_kosz,
        };
        if wlasny_kosz > 0 || self.pulapy.max_koszykow > 0 {
            let wlasne = self.baskets.iter().filter(|x| x.alive()).count();
            if wlasny_kosz > 0 && wlasne >= wlasny_kosz as usize {
                return Gate::Blocked(
                    zd::<MSG>(|| format!("limit koszyków ({wlasne})")),
                    RejectCode::MaxOpenBaskets,
                );
            }
            if self.pulapy.max_koszykow > 0 {
                let razem = wlasne + self.obce.koszyki as usize;
                if razem >= self.pulapy.max_koszykow as usize {
                    return Gate::Blocked(
                        zd::<MSG>(|| format!("pułap koszyków rachunku ({razem})")),
                        RejectCode::MaxOpenBaskets,
                    );
                }
            }
        }
        let limit_kier = self.cfg.max_directional_lots;
        let limit_lotow = self.pulapy.max_lotow;
        if limit_kier > 0.0 || limit_lotow > 0.0 || self.pulapy.max_lotow_kierunkowo > 0.0 {
            let mut buy = 0.0;
            let mut sell = 0.0;
            for p in b.positions() {
                match p.side {
                    Side::Buy => buy += p.volume,
                    Side::Sell => sell += p.volume,
                }
            }
            let (obce_buy, obce_sell) = (self.obce.loty_buy, self.obce.loty_sell);
            if limit_kier > 0.0 && buy.max(sell) >= limit_kier {
                return Gate::Blocked(
                    zd::<MSG>(|| "limit ekspozycji kierunkowej".into()),
                    RejectCode::MaxDirectionalLots,
                );
            }
            if self.pulapy.max_lotow_kierunkowo > 0.0
                && (buy + obce_buy).max(sell + obce_sell) >= self.pulapy.max_lotow_kierunkowo
            {
                return Gate::Blocked(
                    zd::<MSG>(|| "pułap ekspozycji kierunkowej rachunku".into()),
                    RejectCode::MaxDirectionalLots,
                );
            }
            if limit_lotow > 0.0 && buy + sell + obce_buy + obce_sell >= limit_lotow {
                return Gate::Blocked(
                    zd::<MSG>(|| {
                        format!(
                            "pułap łańcucha: {:.2} lota na rachunku",
                            buy + sell + obce_buy + obce_sell
                        )
                    }),
                    RejectCode::MaxDirectionalLots,
                );
            }
        }
        if self.pulapy.max_ryzyko_pct > 0.0 {
            let eq = b.account().equity;
            if eq > 0.0 {
                let mut ryzyko = 0.0_f64;
                let mut licz_poz = |p: &Position| {
                    if p.frozen {
                        return;
                    }
                    match p.sl.or(p.vsl) {
                        Some(sl) => ryzyko += (p.open_price - sl).abs() * XAU_CONTRACT * p.volume,
                        None => ryzyko = f64::INFINITY,
                    }
                };
                for p in b.positions() {
                    licz_poz(p);
                }
                for p in b.ukryte_pozycje() {
                    licz_poz(p);
                }
                let mut licz_zlec = |o: &PendingOrder| {
                    if o.frozen {
                        return;
                    }
                    match o.sl {
                        Some(sl) => ryzyko += (o.price - sl).abs() * XAU_CONTRACT * o.volume,
                        None => ryzyko = f64::INFINITY,
                    }
                };
                for o in b.pendings() {
                    licz_zlec(o);
                }
                for o in b.ukryte_zlecenia() {
                    licz_zlec(o);
                }
                let sufit = eq * self.pulapy.max_ryzyko_pct / 100.0;
                if ryzyko >= sufit {
                    return Gate::Blocked(
                        zd::<MSG>(|| {
                            format!(
                                "pułap ryzyka łańcucha: {ryzyko:.2} $ ≥ {sufit:.2} $ ({:.1} % equity)",
                                self.pulapy.max_ryzyko_pct
                            )
                        }),
                        RejectCode::MaxDirectionalLots,
                    );
                }
            }
        }
        if self.cfg.equity_floor_pct > 0.0 {
            let floor = self.stats.start_balance * self.cfg.equity_floor_pct / 100.0;
            if self.stats.equity <= floor {
                return Gate::Blocked(
                    zd::<MSG>(|| format!("podłoga equity {floor:.2} $")),
                    RejectCode::EquityFloor,
                );
            }
        }
        if self.pulapy.podloga_equity_usd > 0.0 {
            let eq = b.account().equity;
            if eq <= self.pulapy.podloga_equity_usd {
                return Gate::Blocked(
                    zd::<MSG>(|| {
                        format!(
                            "podłoga equity łańcucha: {eq:.2} $ ≤ {:.2} $",
                            self.pulapy.podloga_equity_usd
                        )
                    }),
                    RejectCode::EquityFloor,
                );
            }
        }
        if self.pulapy.cel_dnia_usd > 0.0 || self.pulapy.cel_dnia_pct > 0.0 {
            let dzis =
                self.stats.equity - self.stats.day_start_equity + self.obce.zrealizowane_dzis;
            if self.pulapy.cel_dnia_usd > 0.0 && dzis >= self.pulapy.cel_dnia_usd {
                return Gate::Blocked(
                    zd::<MSG>(|| format!("cel dnia łańcucha: +{dzis:.2} $")),
                    RejectCode::EntryGateBlocked,
                );
            }
            if self.pulapy.cel_dnia_pct > 0.0 {
                let prog = self.stats.day_start_equity.max(1.0) * self.pulapy.cel_dnia_pct / 100.0;
                if dzis >= prog {
                    return Gate::Blocked(
                        zd::<MSG>(|| format!("cel dnia łańcucha: +{dzis:.2} $ ≥ {prog:.2} $")),
                        RejectCode::EntryGateBlocked,
                    );
                }
            }
        }
        if self.pulapy.limit_straty_dnia_usd > 0.0 || self.pulapy.limit_straty_dnia_pct > 0.0 {
            let strata =
                self.stats.day_start_equity - self.stats.equity - self.obce.zrealizowane_dzis;
            if self.pulapy.limit_straty_dnia_usd > 0.0
                && strata >= self.pulapy.limit_straty_dnia_usd
            {
                return Gate::Blocked(
                    zd::<MSG>(|| format!("dzienny limit straty łańcucha: −{strata:.2} $")),
                    RejectCode::EntryGateBlocked,
                );
            }
            if self.pulapy.limit_straty_dnia_pct > 0.0 {
                let prog = self.stats.day_start_equity.max(1.0) * self.pulapy.limit_straty_dnia_pct
                    / 100.0;
                if strata >= prog {
                    return Gate::Blocked(
                        zd::<MSG>(|| {
                            format!("dzienny limit straty łańcucha: −{strata:.2} $ ≥ {prog:.2} $")
                        }),
                        RejectCode::EntryGateBlocked,
                    );
                }
            }
        }
        if self.cfg.margin_call_level_pct > 0.0 {
            let acc = b.account();
            if acc.margin > 0.0 {
                let poziom = acc.equity / acc.margin * 100.0;
                if poziom <= self.cfg.margin_call_level_pct {
                    return Gate::Blocked(
                        zd::<MSG>(|| {
                            format!(
                                "poziom marginu {poziom:.0} % ≤ {:.0} % (wezwanie do uzupełnienia)",
                                self.cfg.margin_call_level_pct
                            )
                        }),
                        RejectCode::MarginCall,
                    );
                }
            }
        }
        if !self.margines_pozwala(b, self.cfg.ml_min_wejscie) {
            let (teraz, docelowy) = self.poziom_marginesu(b);
            let ml = if self.cfg.ml_licz_wiszace {
                docelowy
            } else {
                teraz
            };
            return Gate::Blocked(
                zd::<MSG>(|| {
                    format!(
                        "poziom marginesu {:.0} % ≤ {:.0} %{}",
                        ml.unwrap_or(f64::INFINITY),
                        self.cfg.ml_min_wejscie,
                        if self.cfg.ml_licz_wiszace {
                            " (po wypełnieniu wiszących)"
                        } else {
                            ""
                        }
                    )
                }),
                RejectCode::MarginLevel,
            );
        }
        Gate::Open
    }

    fn tag_blocked(&self, text: &str) -> Option<String> {
        if !self.cfg.signal_filter {
            return None;
        }
        let up = text.to_uppercase();
        for t in self
            .cfg
            .skip_tags
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
        {
            if up.contains(&t) {
                return Some(format!("tag pominięty: {t}"));
            }
        }
        let req: Vec<String> = self
            .cfg
            .require_tags
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
            .collect();
        if !req.is_empty() && !req.iter().any(|t| up.contains(t)) {
            return Some("brak wymaganego tagu".into());
        }
        None
    }

    fn rezim_wyciszony(&self) -> bool {
        if self.cfg.regime_filter == RegimeFilter::Off || self.cfg.regime_zmiennosc_max <= 0.0 {
            return false;
        }
        matches!(self.zakres_okna_glownego(), Some(z) if z > self.cfg.regime_zmiennosc_max)
    }

    fn zakres_okna_glownego(&self) -> Option<f64> {
        let n = (self.cfg.regime_ma_hours as usize).max(2);
        if self.price_hist.len() < n {
            return None;
        }
        let okno = &self.price_hist[self.price_hist.len() - n..];
        let lo = okno.iter().map(|(_, p)| *p).fold(f64::INFINITY, f64::min);
        let hi = okno
            .iter()
            .map(|(_, p)| *p)
            .fold(f64::NEG_INFINITY, f64::max);
        Some(hi - lo)
    }

    fn regime_prog(&self, godzin: usize, okno_awaryjne: bool) -> Option<f64> {
        let n = godzin.max(2);
        if self.price_hist.len() < n {
            return None;
        }
        let okno: Vec<f64> = self.price_hist[self.price_hist.len() - n..]
            .iter()
            .map(|(_, p)| *p)
            .collect();
        let lo = okno.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = okno.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        if self.cfg.regime_zmiennosc_min > 0.0 || self.cfg.regime_zmiennosc_max > 0.0 {
            let zakres = self.zakres_okna_glownego()?;
            if self.cfg.regime_zmiennosc_min > 0.0 && zakres < self.cfg.regime_zmiennosc_min {
                return None;
            }
            if !okno_awaryjne
                && self.cfg.regime_zmiennosc_max > 0.0
                && zakres > self.cfg.regime_zmiennosc_max
            {
                return None;
            }
        }
        let prog = match self.cfg.regime_miara {
            crate::settings::RegimeMiara::Srednia => okno.iter().sum::<f64>() / n as f64,
            crate::settings::RegimeMiara::Mediana => {
                let mut v = okno.clone();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                if v.len() % 2 == 1 {
                    v[v.len() / 2]
                } else {
                    (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0
                }
            }
            crate::settings::RegimeMiara::Kanal => (lo + hi) / 2.0,
            crate::settings::RegimeMiara::Wykladnicza => {
                let tau = (n as f64 / 2.0).max(1.0);
                let mut suma = 0.0;
                let mut wagi = 0.0;
                for (i, p) in okno.iter().enumerate() {
                    let wiek = (n - 1 - i) as f64;
                    let w = (-wiek / tau).exp();
                    suma += p * w;
                    wagi += w;
                }
                suma / wagi.max(1e-9)
            }
            crate::settings::RegimeMiara::Percentyl => {
                let mut v = okno.clone();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let q =
                    (self.cfg.regime_percentyl.clamp(0.0, 100.0) / 100.0) * (v.len() - 1) as f64;
                let i = q.floor() as usize;
                let j = (i + 1).min(v.len() - 1);
                v[i] + (v[j] - v[i]) * (q - i as f64)
            }
        };
        Some(prog)
    }

    fn regime_okno_ok(
        &self,
        side: Side,
        price: Px,
        godzin: usize,
        okno_awaryjne: bool,
    ) -> Option<bool> {
        let prog = self.regime_prog(godzin, okno_awaryjne)?;
        if self.cfg.regime_strefa_martwa > 0.0
            && (price - prog).abs() < self.cfg.regime_strefa_martwa
        {
            return None;
        }
        let with = match side {
            Side::Buy => price > prog,
            Side::Sell => price < prog,
        };
        Some(if self.cfg.regime_filter == RegimeFilter::TrendMa {
            with
        } else {
            !with
        })
    }

    fn regime_ok(&self, side: Side, price: Px) -> bool {
        match self.cfg.regime_filter {
            RegimeFilter::Off => true,
            RegimeFilter::TrendMa | RegimeFilter::CounterMa => {
                let mut g1 = self.cfg.regime_ma_hours as usize;
                let mut awaryjne = false;
                if self.cfg.regime_gdy_rozerwany == crate::settings::RegimeGdyRozerwany::KrotkieOkno
                    && self.cfg.regime_zmiennosc_max > 0.0
                    && self.cfg.regime_okno2_h > 0.0
                {
                    if let Some(z) = self.zakres_okna_glownego() {
                        if z > self.cfg.regime_zmiennosc_max {
                            g1 = self.cfg.regime_okno2_h as usize;
                            awaryjne = true;
                        }
                    }
                }
                let o1 = self.regime_okno_ok(side, price, g1, awaryjne);
                if self.cfg.regime_okno2_h > 0.0 {
                    let o2 =
                        self.regime_okno_ok(side, price, self.cfg.regime_okno2_h as usize, false);
                    return match (o1, o2) {
                        (Some(a), Some(b)) => a && b,
                        (Some(a), None) => a,
                        (None, Some(b)) => b,
                        (None, None) => true,
                    };
                }
                o1.unwrap_or(true)
            }
        }
    }

    pub fn resume_trading(&mut self, ts: Ts) {
        if self.continuation_entry_blocked() {
            self.log(ts,2,"nie wznowiono: CONTINUATION REVIEW wymaga uzgodnienia stanu; strażnik ryzyka pozostaje bez zmian");
            return;
        }
        let r = self.halted.take().unwrap_or_default();
        self.risk_override = true;
        self.stats.peak_equity = self.stats.equity;
        if self.cfg.profit_budget_arm_pct==0.0 {
            self.stats.day_peak_equity = self.stats.equity;
        }
        self.log(
            ts,
            2,
            format!("wznowiono handel mimo: {r} (strażnik wyłączony)"),
        );
    }

    pub fn rearm_guard(&mut self, ts: Ts) {
        self.risk_override = false;
        self.stats.peak_equity = self.stats.equity;
        self.log(ts, 1, "strażnik ryzyka uzbrojony ponownie");
    }
}

#[cfg(test)]
mod testy_price_tp_exact_performance {
    use super::testy_pakiet_a::{silnik, wiad, Atrapa, WEJSCIE};
    use super::*;

    #[test]
    fn price_tp_slots_match_legacy_cohort_stage_and_target_after_restore_order() {
        let mut broker = Atrapa::nowa();
        let mut engine = silnik(|_| {});
        engine.on_message(&mut broker, &wiad(1, 1, None, WEJSCIE));
        let template = engine.baskets[0].clone();
        engine.baskets.clear();
        for n in 0..180 {
            let mut bk = template.clone();
            bk.id = 17 + n * 7;
            bk.state = match n % 3 {
                0 => BasketState::Done,
                1 => BasketState::Armed,
                _ => BasketState::Working,
            };
            bk.pending_exit = (n % 5 == 0).then_some(PendingBasketExit {
                reason: CloseReason::Tp,
                last_attempt_ts: 123,
            });
            bk.tickets = if n % 2 == 0 {
                vec![]
            } else {
                vec![n as u64 + 1]
            };
            bk.tp_stage = n as usize % 5;
            bk.plan_wykonany_do = (n as usize / 3) % 5;
            bk.tps = vec![4010.125, 4020.75, 4030.03125];
            engine.baskets.push(bk);
        }
        engine.baskets.reverse();
        engine.baskets.rotate_left(31);
        for retry in [false, true] {
            engine.cfg.confirmed_exit_retry = retry;
            let legacy_ids: Vec<u32> = engine
                .baskets
                .iter()
                .filter(|bk| bk.alive())
                .filter(|bk| !engine.basket_exit_pending(bk.id))
                .map(|bk| bk.id)
                .collect();
            let slots = engine.price_tp_slots();
            assert_eq!(
                legacy_ids,
                slots.iter().map(|(_, id)| *id).collect::<Vec<_>>()
            );
            for (slot, id) in slots {
                let old = engine.basket(id).unwrap();
                let new = &engine.baskets[slot];
                assert_eq!(new.id, old.id);
                let old_stage = if old.ma_pozycje() {
                    old.tp_stage
                } else {
                    old.etap_obserwowany()
                };
                let new_stage = if new.ma_pozycje() {
                    new.tp_stage
                } else {
                    new.etap_obserwowany()
                };
                let cloned_targets = old.tps.clone();
                assert_eq!(new_stage, old_stage);
                assert_eq!(
                    new.tps.get(new_stage).copied(),
                    cloned_targets.get(old_stage).copied()
                );
            }
        }
    }

    fn cache_fixture() -> Engine {
        let mut broker = Atrapa::nowa();
        let mut engine = silnik(|_| {});
        engine.on_message(&mut broker, &wiad(1, 1, None, WEJSCIE));
        let template = engine.baskets[0].clone();
        engine.baskets.clear();
        for (i, id) in [3, 17, 1_000_003, u32::MAX - 1].into_iter().enumerate() {
            let mut bk = template.clone();
            bk.id = id;
            bk.tp_stage = i;
            engine.baskets.push(bk);
        }
        engine.refresh_basket_slots();
        engine
    }

    fn assert_cache_matches_linear(engine: &Engine) {
        let ids = engine.baskets.iter().map(|bk| bk.id).chain([
            0,
            3,
            17,
            1_000_003,
            u32::MAX - 1,
            u32::MAX,
        ]);
        for id in ids {
            let expected = engine.baskets.iter().find(|bk| bk.id == id);
            let actual = engine.basket(id);
            assert_eq!(
                actual.map(|bk| (bk.id, bk.tp_stage, bk.sl)),
                expected.map(|bk| (bk.id, bk.tp_stage, bk.sl)),
                "ID={id}"
            );
        }
    }

    #[test]
    fn basket_index_checks_id_after_reorder_replace_append_and_truncate() {
        let mut engine = cache_fixture();
        assert_eq!(engine.basket_slots.len(), 4);
        assert_cache_matches_linear(&engine);
        engine.baskets.reverse();
        assert_cache_matches_linear(&engine);
        engine.basket_mut(17).unwrap().sl = Some(1234.125);
        assert_eq!(
            engine.baskets.iter().find(|bk| bk.id == 17).unwrap().sl,
            Some(1234.125)
        );
        let mut replacement = engine.baskets[0].clone();
        replacement.id = 941;
        replacement.tp_stage = 9;
        engine.baskets[1] = replacement.clone(); // ta sama długość, nowe ID
        assert_cache_matches_linear(&engine);
        replacement.id = 942;
        engine.baskets.push(replacement);
        assert_cache_matches_linear(&engine); // jeszcze przed refresh
        engine.refresh_basket_slots();
        assert_eq!(engine.basket_slots_len, 5);
        assert_cache_matches_linear(&engine);
        engine.baskets.truncate(2);
        assert_cache_matches_linear(&engine); // stary slot poza wektorem
        engine.refresh_basket_slots();
        assert_cache_matches_linear(&engine);
        engine.baskets.clear();
        assert_cache_matches_linear(&engine);
    }

    #[test]
    fn basket_index_restore_adoption_and_duplicate_fallback_preserve_find() {
        let engine = cache_fixture();
        let saved = serde_json::to_vec(&engine.baskets).unwrap();
        let mut restored = silnik(|_| {});
        restored.adopt_baskets(serde_json::from_slice(&saved).unwrap());
        assert_eq!(restored.basket_slots.len(), 4);
        assert_cache_matches_linear(&restored);
        assert_eq!(serde_json::to_vec(&restored.baskets).unwrap(), saved);
        let mut changed = restored.baskets[1].clone();
        changed.tp_stage = 99;
        restored.adopt_baskets(vec![changed]); // replace, bez zmiany długości
        assert_eq!(restored.basket(17).unwrap().tp_stage, 99);
        assert_cache_matches_linear(&restored);
        let mut duplicate = restored.baskets[0].clone();
        duplicate.tp_stage = 777;
        restored.baskets.push(duplicate);
        restored.refresh_basket_slots();
        assert!(restored.basket_slots.is_empty());
        assert_cache_matches_linear(&restored);
        restored.baskets.reverse();
        assert_cache_matches_linear(&restored);
    }
}

#[cfg(test)]
mod testy_rozmiar_zmiennoscia {
    use super::*;

    fn silnik(c: Settings, saldo: f64, mult: f64) -> Engine {
        let mut e = Engine::new(c, saldo);
        let mut z = e.stan_zmiennosci();
        z.mult = mult;
        e.set_stan_zmiennosci(z);
        e
    }

    #[test]
    fn przy_off_mnoznik_jest_niewidoczny() {
        for (proc, fixed, maxlot) in [
            (0.5, 0.01, 10.0),
            (1.0, 0.05, 0.01),
            (0.4, 0.02, 0.0),
            (2.0, 0.10, 3.0),
        ] {
            let mut c = Settings::default();
            c.lot_mode_percent = true;
            c.lot_percent = proc;
            c.lot_fixed = fixed;
            c.lot_max = maxlot;
            assert_eq!(
                c.vol_size_mode,
                VolSizeMode::Off,
                "domyślnie musi być wyłączone"
            );
            for saldo in [300.0, 512.37, 1000.0, 4321.0, 25_000.0] {
                let odn = Engine::new(c.clone(), saldo).lot_size(saldo);
                for m in [0.0, 0.13, 1.0, 7.5, f64::NAN] {
                    let e = silnik(c.clone(), saldo, m);
                    assert_eq!(
                        e.lot_size(saldo),
                        odn,
                        "Off z mnożnikiem {m} ruszył lot przy saldzie {saldo}"
                    );
                }
            }
        }
    }

    #[test]
    fn wlaczony_tryb_mnozy_lot_bazowy() {
        let mut c = Settings::default();
        c.lot_mode_percent = false;
        c.lot_fixed = 1.00;
        c.lot_max = 100.0;
        c.vol_size_mode = VolSizeMode::Target;
        assert_eq!(silnik(c.clone(), 5000.0, 1.0).lot_size(5000.0), 1.00);
        assert_eq!(silnik(c.clone(), 5000.0, 2.0).lot_size(5000.0), 2.00);
        assert_eq!(silnik(c.clone(), 5000.0, 0.5).lot_size(5000.0), 0.50);
    }

    #[test]
    fn sufit_lota_wygrywa_z_mnoznikiem() {
        let mut c = Settings::default();
        c.lot_mode_percent = false;
        c.lot_fixed = 0.02;
        c.lot_max = 0.03;
        c.lot_min = 0.01;
        c.vol_size_mode = VolSizeMode::Target;
        assert_eq!(
            silnik(c.clone(), 300.0, 10.0).lot_size(300.0),
            0.03,
            "sufit musi uciąć"
        );
        assert_eq!(
            silnik(c.clone(), 300.0, 0.01).lot_size(300.0),
            0.01,
            "podłoga musi podnieść"
        );
    }

    #[test]
    fn lot_koszyka_rosnie_z_liczba_szczebli() {
        let mut c = Settings::default();
        c.lot_mode_percent = false;
        c.lot_fixed = 0.02;
        c.lot_max = 0.0;
        c.lot_min = 0.01;
        c.entry_weights = String::new();

        c.entry_units = 1;
        let e1 = Engine::new(c.clone(), 675.0);
        assert_eq!(e1.poziomy_wejscia_planowane(), 1);
        assert!(
            (e1.lot_koszyka_planowany(675.0) - 0.02).abs() < 1e-9,
            "jeden szczebel = lot zlecenia"
        );

        c.entry_units = 5;
        let e5 = Engine::new(c.clone(), 675.0);
        assert_eq!(e5.poziomy_wejscia_planowane(), 5);
        assert!(
            (e5.lot_koszyka_planowany(675.0) - 0.10).abs() < 1e-9,
            "pięć szczebli = pięć razy lot zlecenia, a nie lot zlecenia"
        );

        c.grid_anchor_absolute = true;
        let ek = Engine::new(c.clone(), 675.0);
        assert!(
            (ek.lot_koszyka_planowany(675.0) - 0.50).abs() < 1e-9,
            "krata bezwzględna mnoży szczeble przez jednostki"
        );

        c.grid_anchor_absolute = false;
        c.entry_units = 10;
        c.max_open_positions = 1;
        c.enforce_position_limit_on_fill = true;
        let ez = Engine::new(c.clone(), 675.0);
        assert_eq!(ez.poziomy_wejscia_planowane(), 10);
        assert!(
            (ez.lot_koszyka_planowany(675.0) - 0.02).abs() < 1e-9,
            "limit 1 pozycji = koszyk jednego zlecenia, nie dziesięciu"
        );

        c.enforce_position_limit_on_fill = false;
        let ezn = Engine::new(c.clone(), 675.0);
        assert!(
            (ezn.lot_koszyka_planowany(675.0) - 0.20).abs() < 1e-9,
            "bez egzekwowania limit nie jest sufitem pierwszej fali"
        );

        c.entry_units = 5;
        c.max_open_positions = 0;
        c.lot_max = 0.01;
        let es = Engine::new(c.clone(), 675.0);
        assert!(
            (es.lot_koszyka_planowany(675.0) - 0.05).abs() < 1e-9,
            "sufit 0,01 tnie ZLECENIE do 0,01, więc koszyk to 5 × 0,01"
        );
    }

    #[test]
    fn ranga_percentylowa_jest_w_przedziale() {
        let p: Vec<f64> = (0..100).map(|i| i as f64).collect();
        assert_eq!(Engine::ranga_percentylowa(&p, -1.0), 0.0);
        assert_eq!(Engine::ranga_percentylowa(&p, 1000.0), 1.0);
        assert!((Engine::ranga_percentylowa(&p, 50.0) - 0.5).abs() < 0.02);
        assert_eq!(Engine::ranga_percentylowa(&[], 3.0), 0.5);
    }

    #[test]
    fn profil_sezonowy_przenosi_sie_przez_wymiane_silnika() {
        let mut c = Settings::default();
        c.vol_size_mode = VolSizeMode::Target;
        let mut e = Engine::new(c.clone(), 300.0);
        let mut z = e.stan_zmiennosci();
        for h in 0..24 {
            z.sezon_suma[h] = 10.0 + h as f64;
            z.sezon_ile[h] = 5;
        }
        e.set_stan_zmiennosci(z.clone());
        let (profil_przed, ile_przed) = e.profil_godzinowy();

        let mut nowy = Engine::new(c, 300.0);
        nowy.set_stan_zmiennosci(e.stan_zmiennosci());
        let (profil_po, ile_po) = nowy.profil_godzinowy();
        assert_eq!(profil_przed, profil_po);
        assert_eq!(ile_przed, ile_po);
        assert!(profil_po[23] > profil_po[0]);
    }
}

#[cfg(test)]
mod testy_trail_adaptive {
    use super::*;

    const T0: Ts = 1_755_000_000_000;

    fn pozycja(side: Side, peak: f64, runner: bool) -> Position {
        Position {
            ticket: 1,
            side,
            volume: 0.01,
            open_price: 4000.0,
            open_ts: T0 - 120_000,
            sl: Some(3990.0),
            tp: None,
            vsl: None,
            basket: None,
            level: 0,
            frozen: false,
            peak_pts: peak,
            last_peak_ts: T0,
            is_runner: runner,
            is_toucher: false,
            comment: String::new(),
        }
    }

    fn silnik() -> Engine {
        let mut c = Settings::default();
        c.trail_adaptive_enabled = true;
        c.trail_adaptive_runners_only = false;
        c.trail_adaptive_window_s = 60.0;
        c.trail_adaptive_min_samples = 4;
        c.trail_adaptive_fast_vol_s = 15.0;
        c.trail_adaptive_slow_vol_s = 60.0;
        c.trail_adaptive_vol_ratio = 0.0;
        c.trail_adaptive_trend_er = 0.6;
        c.trail_adaptive_reversal_er = 0.6;
        c.trail_adaptive_trend_gap_mult = 2.0;
        c.trail_adaptive_chop_gap_mult = 0.8;
        c.trail_adaptive_reversal_gap_mult = 0.5;
        Engine::new(c, 600.0)
    }

    #[test]
    fn signed_er_rozroznia_trend_i_odwrocenie_dla_strony() {
        let mut e = silnik();
        e.vol_hist = vec![
            (T0 - 20_000, 4000.0),
            (T0 - 15_000, 4001.0),
            (T0 - 10_000, 4002.0),
            (T0 - 5_000, 4003.0),
        ];
        let q = Quote {
            ts: T0,
            bid: 4003.9,
            ask: 4004.1,
        };
        let snap = e.trail_adaptive_snapshot(&q).expect("wystarczają próbki");
        assert!((snap.price_efficiency - 1.0).abs() < 1e-12);
        assert_eq!(
            e.trail_adaptive_gap(&pozycja(Side::Buy, 20.0, true), true, 10.0, Some(&snap)),
            20.0
        );
        assert_eq!(
            e.trail_adaptive_gap(&pozycja(Side::Sell, 20.0, true), true, 10.0, Some(&snap)),
            5.0
        );
    }

    #[test]
    fn szum_uzywa_osobnego_mnoznika_a_klamry_sa_twarde() {
        let mut e = silnik();
        e.vol_hist = vec![
            (T0 - 20_000, 4000.0),
            (T0 - 15_000, 4002.0),
            (T0 - 10_000, 3999.0),
            (T0 - 5_000, 4001.0),
        ];
        let q = Quote {
            ts: T0,
            bid: 3999.9,
            ask: 4000.1,
        };
        let snap = e.trail_adaptive_snapshot(&q).unwrap();
        let p = pozycja(Side::Buy, 20.0, true);
        assert_eq!(e.trail_adaptive_gap(&p, true, 10.0, Some(&snap)), 8.0);

        e.cfg.trail_adaptive_min_gap = 9.0;
        assert_eq!(e.trail_adaptive_gap(&p, true, 10.0, Some(&snap)), 9.0);
        e.cfg.trail_adaptive_max_gap = 7.0; // odwrócone klamry normalizują się do 9
        assert_eq!(e.trail_adaptive_gap(&p, true, 10.0, Some(&snap)), 9.0);
    }

    #[test]
    fn przyszla_probka_nie_przecieka_do_decyzji() {
        let mut e = silnik();
        e.vol_hist = vec![
            (T0 - 15_000, 4000.0),
            (T0 - 10_000, 4001.0),
            (T0 - 5_000, 4002.0),
        ];
        let q = Quote {
            ts: T0,
            bid: 4002.9,
            ask: 4003.1,
        };
        let przed = e.trail_adaptive_snapshot(&q).unwrap();
        e.vol_hist.push((T0 + 5_000, 3900.0));
        let po = e.trail_adaptive_snapshot(&q).unwrap();
        assert_eq!(przed, po, "punkt z przyszłości zmienił decyzję");
    }

    #[test]
    fn brak_wiedzy_i_wylacznik_sa_scislym_noopem() {
        let mut e = silnik();
        e.vol_hist = vec![(T0 - 5_000, 4000.0)];
        let q = Quote {
            ts: T0,
            bid: 4000.9,
            ask: 4001.1,
        };
        assert!(e.trail_adaptive_snapshot(&q).is_none());

        e.cfg.trail_adaptive_enabled = false;
        e.cfg.trail_adaptive_trend_gap_mult = 99.0;
        e.vol_hist = vec![
            (T0 - 15_000, 4000.0),
            (T0 - 10_000, 4001.0),
            (T0 - 5_000, 4002.0),
        ];
        assert!(e.trail_adaptive_snapshot(&q).is_none());
        assert_eq!(
            e.trail_adaptive_gap(&pozycja(Side::Buy, 20.0, true), true, 10.0, None),
            10.0
        );
    }

    #[test]
    fn zakres_runner_only_nie_dotyka_zwyklej_pozycji() {
        let mut e = silnik();
        e.cfg.trail_adaptive_runners_only = true;
        let snap = TrailAdaptiveSnapshot {
            price_efficiency: 1.0,
            vol_ratio: None,
        };
        let p = pozycja(Side::Buy, 20.0, false);
        assert_eq!(e.trail_adaptive_gap(&p, false, 10.0, Some(&snap)), 10.0);
        assert_eq!(e.trail_adaptive_gap(&p, true, 10.0, Some(&snap)), 20.0);
    }

    #[test]
    fn ekspansja_ma_inny_mnoznik_zgodnie_i_przeciw() {
        let mut e = silnik();
        e.cfg.trail_adaptive_vol_ratio = 1.5;
        e.cfg.trail_adaptive_vol_favorable_mult = 1.25;
        e.cfg.trail_adaptive_vol_adverse_mult = 0.4;
        let favorable = TrailAdaptiveSnapshot {
            price_efficiency: 0.8,
            vol_ratio: Some(2.0),
        };
        let adverse = TrailAdaptiveSnapshot {
            price_efficiency: -0.8,
            vol_ratio: Some(2.0),
        };
        let p = pozycja(Side::Buy, 20.0, true);
        assert_eq!(e.trail_adaptive_gap(&p, true, 10.0, Some(&favorable)), 25.0);
        assert_eq!(e.trail_adaptive_gap(&p, true, 10.0, Some(&adverse)), 2.0);
    }

    #[test]
    fn adaptacja_jest_wpięta_w_rzeczywisty_kandydat_gap() {
        let mut e = silnik();
        e.cfg.trail_mode = TrailMode::Gap;
        e.cfg.trail_start = 0.0;
        e.cfg.trail_gap = 10.0;
        e.cfg.trail_adaptive_trend_gap_mult = 2.0;
        let p = pozycja(Side::Buy, 30.0, false);
        let q = Quote {
            ts: T0,
            bid: 4030.0,
            ask: 4030.2,
        };
        let favorable = TrailAdaptiveSnapshot {
            price_efficiency: 1.0,
            vol_ratio: None,
        };

        assert_eq!(
            e.trail_candidate(&p, &q, false, Some(&favorable)),
            Some(4010.0),
            "favorable trend must widen the live Gap candidate from 10 to 20"
        );
        e.cfg.trail_adaptive_enabled = false;
        assert_eq!(
            e.trail_candidate(&p, &q, false, Some(&favorable)),
            Some(4020.0),
            "OFF must restore the exact legacy Gap candidate"
        );
    }
}

#[cfg(test)]
mod testy_pakiet_a {
    use super::*;

    pub(super) const TS0: Ts = 1_755_000_000_000; // stały punkt czasu — determinizm

    pub(super) struct Atrapa {
        q: Quote,
        positions: Vec<Position>,
        pendings: Vec<PendingOrder>,
        pub(super) closed: Vec<ClosedTrade>,
        next: Ticket,
        pub(super) stops: f64,
        cancel_failures: usize,
        pub(super) market_failures: usize,
        pub(super) maximum_volume: f64,
    }

    impl Atrapa {
        pub(super) fn nowa() -> Self {
            Atrapa {
                q: Quote {
                    ts: TS0,
                    bid: 3998.0,
                    ask: 3998.3,
                },
                positions: Vec::new(),
                pendings: Vec::new(),
                closed: Vec::new(),
                next: 1,
                stops: 0.0,
                cancel_failures: 0,
                market_failures: 0,
                maximum_volume: f64::NAN,
            }
        }

        pub(super) fn ustaw_cene(&mut self, ts: Ts, bid: Px, ask: Px) {
            self.q = Quote { ts, bid, ask };
        }
    }

    impl Broker for Atrapa {
        fn t100_contract_supported(&self) -> bool { true }
        fn quote(&self) -> Quote {
            self.q
        }
        fn account(&self) -> Account {
            Account {
                balance: 400.0,
                equity: 400.0,
                margin: 0.0,
                free_margin: 400.0,
                leverage: 500,
                credit: 0.0,
            }
        }
        fn stops_level(&self) -> f64 {
            self.stops
        }
        fn volume_max(&self)->f64 {self.maximum_volume}
        fn positions(&self) -> &[Position] {
            &self.positions
        }
        fn pendings(&self) -> &[PendingOrder] {
            &self.pendings
        }
        fn positions_mut(&mut self) -> &mut Vec<Position> {
            &mut self.positions
        }
        fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
            &mut self.pendings
        }
        fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> {
            if self.market_failures>0 {self.market_failures-=1;return Err(BrokerError::Rejected);}
            let t = self.next;
            self.next += 1;
            self.positions.push(Position {
                ticket: t,
                side: r.side,
                volume: r.volume,
                open_price: self.q.entry(r.side),
                open_ts: self.q.ts,
                sl: r.sl,
                tp: r.tp,
                vsl: None,
                basket: r.basket,
                level: r.level,
                frozen: false,
                peak_pts: 0.0,
                last_peak_ts: 0,
                is_runner: false,
                is_toucher: r.is_toucher,
                comment: r.comment,
            });
            Ok(t)
        }
        fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> {
            let t = self.next;
            self.next += 1;
            self.pendings.push(PendingOrder {
                ticket: t,
                kind: r.kind,
                volume: r.volume,
                price: r.price,
                sl: r.sl,
                tp: r.tp,
                placed_ts: self.q.ts,
                basket: r.basket,
                level: r.level,
                frozen: false,
                is_toucher: r.is_toucher,
                is_topup: r.is_topup,
                comment: r.comment,
            });
            Ok(t)
        }
        fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
            match self.positions.iter_mut().find(|p| p.ticket == t) {
                Some(p) => {
                    p.sl = sl;
                    p.tp = tp;
                    Ok(())
                }
                None => Err(BrokerError::NoSuchTicket),
            }
        }
        fn modify_pending(
            &mut self,
            t: Ticket,
            price: Px,
            sl: Option<Px>,
            tp: Option<Px>,
        ) -> BResult<()> {
            match self.pendings.iter_mut().find(|o| o.ticket == t) {
                Some(o) => {
                    o.price = price;
                    o.sl = sl;
                    o.tp = tp;
                    Ok(())
                }
                None => Err(BrokerError::NoSuchTicket),
            }
        }
        fn close_position(&mut self, t: Ticket, _reason: CloseReason) -> BResult<f64> {
            let przed = self.positions.len();
            self.positions.retain(|p| p.ticket != t);
            if self.positions.len() < przed {
                Ok(0.0)
            } else {
                Err(BrokerError::NoSuchTicket)
            }
        }
        fn close_partial(&mut self, _t: Ticket, _v: f64, _r: CloseReason) -> BResult<f64> {
            Ok(0.0)
        }
        fn cancel_pending(&mut self, t: Ticket) -> BResult<()> {
            if self.cancel_failures > 0 {
                self.cancel_failures -= 1;
                return Err(BrokerError::Rejected);
            }
            let przed = self.pendings.len();
            self.pendings.retain(|o| o.ticket != t);
            if self.pendings.len() < przed {
                Ok(())
            } else {
                Err(BrokerError::NoSuchTicket)
            }
        }
        fn drain_closed(&mut self) -> Vec<ClosedTrade> {
            std::mem::take(&mut self.closed)
        }
    }

    pub(super) fn zrodlo(chat: i64) -> SourceKey {
        SourceKey::new(chat, None)
    }

    pub(super) fn wiad(
        chat: i64,
        msg_id: i64,
        edit_of: Option<i64>,
        text: &str,
    ) -> IncomingMessage {
        IncomingMessage {
            ts: TS0,
            source: zrodlo(chat),
            source_name: format!("TEST{chat}"),
            msg_id,
            reply_to: None,
            edit_of,
            text: text.into(),
        }
    }

    pub(super) const WEJSCIE: &str = "BUY LIMITS GOLD @ 4000/3995\nTP 4010\nTP 4020\nSL 3990";

    pub(super) fn silnik(zmien: impl FnOnce(&mut Settings)) -> Engine {
        let mut c = Settings::default();
        zmien(&mut c);
        Engine::new(c, 400.0)
    }

    #[test]
    fn t1_akcja_zignorowana_nie_trafia_do_done_actions() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|_| {});
        assert!(e.cfg.dedup_pelny_status, "oś A2 ma być domyślnie włączona");
        e.on_message(&mut b, &wiad(1, 7, None, "✅ TP1 HIT +48 PIPS"));
        assert!(
            e.done_actions.get(&(zrodlo(1), 7)).is_none(),
            "zignorowana akcja zapisała się jako wykonana: {:?}",
            e.done_actions.get(&(zrodlo(1), 7))
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| c.dedup_pelny_status = false);
        e.on_message(&mut b, &wiad(1, 7, None, "✅ TP1 HIT +48 PIPS"));
        assert_eq!(
            e.done_actions.get(&(zrodlo(1), 7)).map(|v| v.as_slice()),
            Some(&["tp1".to_string()][..]),
            "przy wyłączonej osi musi zostać stare zachowanie 1:1"
        );
    }

    #[test]
    fn t2_kolizja_msg_id_dwoch_zrodel() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|_| {});
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        e.on_message(&mut b, &wiad(2, 2, None, WEJSCIE));
        assert_eq!(e.baskets.len(), 2, "dwa źródła → dwa koszyki");

        e.on_message(&mut b, &wiad(1, 5, None, "✅ TP1 HIT +48 PIPS"));
        assert!(e.done_actions.contains_key(&(zrodlo(1), 5)));

        e.on_message(&mut b, &wiad(2, 5, Some(5), "✅ TP1 HIT +48 PIPS"));
        let koszyk2 = e.baskets.iter().find(|x| x.source == zrodlo(2)).unwrap();
        assert!(
            koszyk2.tp_stage >= 1,
            "TP1 kanału 2 zginęło w dedupie przez kolizję msg_id (etap {})",
            koszyk2.tp_stage
        );
        assert!(e.done_actions.contains_key(&(zrodlo(2), 5)));
    }

    #[test]
    fn t3_edycja_z_wejsciem_nie_polyka_reszty_akcji() {
        let edycja = "BUY LIMITS GOLD @ 4000/3995\nTP 4010\nTP 4020\nSL 3988\nMOVE SL TO 3993";

        for (os, oczekiwany_sl) in [(true, 3993.0), (false, 3988.0)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| c.edycja_wykonuje_reszte_akcji = os);
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            assert_eq!(e.baskets.len(), 1);
            e.on_message(&mut b, &wiad(1, 1, Some(1), edycja));
            assert_eq!(e.baskets.len(), 1, "edycja nie ma prawa dołożyć koszyka");
            assert_eq!(
                e.baskets[0].sl,
                Some(oczekiwany_sl),
                "oś A3 = {os}: SetSl z edycji {}",
                if os {
                    "ma się wykonać"
                } else {
                    "ma przepaść (stare zachowanie)"
                }
            );
            if os {
                assert!(e
                    .done_actions
                    .get(&(zrodlo(1), 1))
                    .is_some_and(|d| d.iter().any(|k| k == "entry")));
            }
        }
    }

    #[test]
    fn auto_ea_dzis_rowna_sie_auto_co_do_bitu() {
        let przebieg = |flaga: bool| {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|_| {});
            e.tryb_auto_ea = flaga;
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            e.on_tick(
                &mut b,
                &Quote {
                    ts: TS0 + 1_000,
                    bid: 3998.0,
                    ask: 3998.3,
                },
            );
            e.on_message(&mut b, &wiad(1, 2, None, "✅ TP1 HIT +48 PIPS"));
            e.on_tick(
                &mut b,
                &Quote {
                    ts: TS0 + 2_000,
                    bid: 4005.0,
                    ask: 4005.3,
                },
            );
            e.on_message(&mut b, &wiad(1, 3, None, "MOVE SL TO 3995"));
            e.on_tick(
                &mut b,
                &Quote {
                    ts: TS0 + 3_000,
                    bid: 4008.0,
                    ask: 4008.3,
                },
            );
            (
                format!("{:?}", e.baskets),
                format!("{:?}", b.pendings),
                format!("{:?}", b.positions),
                format!("{:?}", b.closed),
                format!("{:?}", e.done_actions.get(&(zrodlo(1), 1))),
            )
        };
        assert_eq!(
            przebieg(false),
            przebieg(true),
            "AUTO-EA musi dziś być AUTO co do bitu"
        );
    }

    #[test]
    fn t4_klucz_z_wartoscia_przepuszcza_nowy_poziom() {
        for (os, oczekiwany_sl) in [(true, 3993.0), (false, 3992.0)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| c.dedup_klucz_z_wartoscia = os);
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            e.on_message(&mut b, &wiad(1, 2, None, "MOVE SL TO 3992"));
            assert_eq!(e.baskets[0].sl, Some(3992.0));
            e.on_message(&mut b, &wiad(1, 2, Some(2), "MOVE SL TO 3993"));
            assert_eq!(
                e.baskets[0].sl,
                Some(oczekiwany_sl),
                "oś A4 = {os}: edycja poziomu {}",
                if os {
                    "musi przejść"
                } else {
                    "ginie jako duplikat (stare zachowanie)"
                }
            );
        }
    }

    #[test]
    fn t5_edycja_sierota_nie_otwiera_koszyka() {
        for (os, koszykow) in [(true, 0), (false, 1)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| c.edycja_sieroty_nie_otwiera = os);
            e.on_message(&mut b, &wiad(1, 50, Some(999), WEJSCIE));
            assert_eq!(
                e.baskets.len(),
                koszykow,
                "oś A5 = {os}: edycja nieznanej wiadomości"
            );
            if os {
                assert_eq!(e.odrzuty.get("EditOrphan"), Some(&1));
            }
        }
    }

    #[test]
    fn t6_powtorna_wiadomosc_nie_tworzy_drugiego_koszyka() {
        for (os, koszykow) in [(true, 1), (false, 2)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| c.entry_idempotencja = os);
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            assert_eq!(
                e.baskets.len(),
                koszykow,
                "oś A6 = {os}: re-delivery tej samej wiadomości"
            );
        }
    }

    #[test]
    fn market_now_orphan_edit_respects_the_entry_guard() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.honor_market_open = true;
            c.edycja_sieroty_nie_otwiera = true;
        });
        e.on_message(&mut b, &wiad(1, 50, Some(50), "BUY NOW"));
        assert!(e.baskets.is_empty(), "orphan market edit opened new risk");
        assert!(b.positions.is_empty());
        assert_eq!(e.odrzuty.get("EditOrphan"), Some(&1));
    }

    #[test]
    fn market_now_redelivery_is_idempotent_even_after_restore() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| c.honor_market_open = true);
        let original = wiad(1, 50, None, "BUY NOW");
        e.on_message(&mut b, &original);
        assert_eq!(e.baskets.len(), 1);
        assert_eq!(b.positions.len(), 1);
        e.done_actions.clear(); // replay after volatile dedup memory was lost
        e.on_message(&mut b, &original);
        assert_eq!(e.baskets.len(), 1, "market re-delivery created another basket");
        assert_eq!(b.positions.len(), 1);
    }

    #[test]
    fn market_now_edit_cannot_reopen_a_known_entry_with_a_changed_direction() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| c.honor_market_open = true);
        e.on_message(&mut b, &wiad(1, 50, None, "BUY NOW"));
        e.on_message(&mut b, &wiad(1, 50, Some(50), "SELL NOW"));
        assert_eq!(e.baskets.len(), 1, "changed market edit opened opposite risk");
        assert_eq!(b.positions.len(), 1);
    }

    #[test]
    fn cosmetic_entry_edit_preserves_management_progress_and_stop() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|_| {});
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        e.on_message(&mut b, &wiad(1, 2, None, "MOVE SL TO 3998"));
        e.baskets[0].tp_stage = 1;
        e.baskets[0].plan_wykonany_do = 1;
        e.baskets[0].tp_touch_ts = vec![TS0, 0];
        let before = (e.baskets[0].sl, e.baskets[0].tp_stage,
            e.baskets[0].plan_wykonany_do, e.baskets[0].tp_touch_ts.clone());
        let orders = format!("{:?}", b.pendings);
        e.on_message(&mut b, &wiad(1, 1, Some(1), &format!("{WEJSCIE}\nGood luck")));
        assert_eq!((e.baskets[0].sl, e.baskets[0].tp_stage,
            e.baskets[0].plan_wykonany_do, e.baskets[0].tp_touch_ts.clone()), before,
            "cosmetic edit rolled back the active stop or TP progress");
        assert_eq!(format!("{:?}", b.pendings), orders, "cosmetic edit changed broker orders");
    }

    #[test]
    fn target_only_entry_edit_updates_the_armed_broker_plan() {
        let mut b = Atrapa::nowa();
        b.ustaw_cene(TS0, 4004.0, 4004.2);
        let mut e = silnik(|_| {});
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        let before: Vec<_> = b.pendings.iter().map(|p| p.tp).collect();
        assert!(!before.is_empty());
        e.on_message(&mut b, &wiad(1, 1, Some(1), &WEJSCIE.replace("4010", "4012").replace("4020", "4022")));
        let after: Vec<_> = b.pendings.iter().map(|p| p.tp).collect();
        assert_ne!(before, after, "engine targets changed but broker targets did not");
        assert!(after.iter().all(|tp| tp.is_none_or(|v| v == 4012.0 || v == 4022.0)));
    }

    #[test]
    fn switching_off_v2_cannot_clear_an_existing_edit_review() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|_| {});
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        let state = e.baskets[0].entry_edit_state.as_mut().unwrap();
        state.review = Some(EntryEditReview {
            desired_source: state.source.clone().unwrap(), received_ts: TS0,
            reason: "SyntheticUnconfirmedModify".into(),
        });
        let before = e.baskets[0].sl;
        e.on_message(&mut b, &wiad(1, 1, Some(1), &WEJSCIE.replace("3990", "3988")));
        assert!(e.baskets[0].entry_edit_state.as_ref().unwrap().review.is_some());
        assert_eq!(e.baskets[0].sl, before);
    }

    #[test]
    fn explicit_pending_validity_survives_14_days_age_ttl_and_target_touch() {
        for explicit in [false, true] {
            for enabled in [false, true] {
                let mut b = Atrapa::nowa();
                b.ustaw_cene(TS0, 4004.0, 4004.2);
                let mut e = silnik(|c| {
                    c.explicit_pending_until_cancel = enabled;
                    c.ignore_old_after_min = 1.0;
                    c.pending_ttl_h = 0.01;
                    c.basket_max_age_min = 1.0;
                    c.pending_drop_on_target = true;
                    c.pending_drop_arm = false;
                    c.pending_lifetime = PendingLifetime::UntilTp1;
                });
                let text = if explicit { WEJSCIE.to_string() } else { WEJSCIE.replace("LIMITS ", "") };
                e.on_message(&mut b, &wiad(1, 1, None, &text));
                assert!(!b.pendings.is_empty(), "fixture must place pending orders");
                let old: Vec<_> = b.pendings.iter().map(|o| o.ticket).collect();
                b.ustaw_cene(TS0 + 14 * 86_400_000, 4011.0, 4011.2);
                let q = b.quote();
                e.on_tick(&mut b, &q);
                if enabled && explicit {
                    assert_eq!(old, b.pendings.iter().map(|o| o.ticket).collect::<Vec<_>>());
                    assert_eq!(e.export_pending_source_memory().len(), 1);
                    assert_eq!(e.baskets[0].plan_wykonany_do, 0,
                        "unfilled source limits cannot consume their plan at a historical TP touch");
                    let mut tp = wiad(1, 2, None, "TP1 HIT"); tp.reply_to = Some(1); tp.ts = q.ts + 1;
                    e.on_message(&mut b, &tp);
                    assert_eq!(old, b.pendings.iter().map(|o| o.ticket).collect::<Vec<_>>());
                    let mut rf = wiad(1, 3, None, "RISK FREE"); rf.reply_to = Some(1); rf.ts = q.ts + 2;
                    e.cfg.pending_cancel_on_riskfree = true;
                    e.on_message(&mut b, &rf);
                    assert_eq!(old, b.pendings.iter().map(|o| o.ticket).collect::<Vec<_>>());
                } else {
                    assert!(b.pendings.is_empty(), "ordinary grid/legacy validity must keep its configured expiry");
                }
            }
        }
    }

    #[test]
    fn source_cancel_variants_route_to_14_day_old_message_not_the_newest_grid() {
        for text in ["CANCEL", "CANCELLED", "CANCELED", "NO LONGER VALID",
            "THIS SETUP IS NO LONGER VALID", "DELETE ALL LIMITS", "INVALID"] {
            let mut b = Atrapa::nowa(); b.ustaw_cene(TS0, 4004.0, 4004.2);
            let mut e = silnik(|c| c.explicit_pending_until_cancel = true);
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            let old_id = e.baskets[0].id;
            let mut second = wiad(1, 2, None, &WEJSCIE.replace("4000/3995", "3998/3993"));
            second.ts += 14 * 86_400_000;
            e.on_message(&mut b, &second);
            assert_eq!(e.baskets.len(), 2);
            let latest = e.baskets[1].id;
            let newer_orders: Vec<_> = b.pendings.iter().filter(|p| p.basket == Some(latest))
                .map(|p| p.ticket).collect();
            assert!(!newer_orders.is_empty());
            let mut cancel = wiad(1, 3, None, text); cancel.reply_to = Some(1); cancel.ts = second.ts + 1;
            e.on_message(&mut b, &cancel);
            assert!(e.pending_source_cancelled(old_id), "{text}");
            assert!(!b.pendings.iter().any(|p| p.basket == Some(old_id)), "{text}");
            assert_eq!(newer_orders, b.pendings.iter().map(|p| p.ticket).collect::<Vec<_>>(), "{text}");
            assert!(!e.pending_source_cancelled(latest));
        }
    }

    #[test]
    fn explicit_pending_requires_known_reply_and_honors_it_when_generic_cancel_is_off() {
        let mut b = Atrapa::nowa(); b.ustaw_cene(TS0, 4004.0, 4004.2);
        let mut e = silnik(|c| { c.explicit_pending_until_cancel = true; c.honor_cancel = false; });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        let id = e.baskets[0].id;
        e.on_message(&mut b, &wiad(1, 2, None, "NO LONGER VALID"));
        assert!(!b.pendings.is_empty()); assert!(!e.pending_source_cancelled(id));
        let mut unknown = wiad(1, 3, None, "CANCEL"); unknown.reply_to = Some(9999);
        e.on_message(&mut b, &unknown);
        assert!(!b.pendings.is_empty()); assert!(!e.pending_source_cancelled(id));
        let mut known = wiad(1, 4, None, "CANCEL"); known.reply_to = Some(1);
        e.on_message(&mut b, &known);
        assert!(b.pendings.is_empty()); assert!(e.pending_source_cancelled(id));
    }

    #[test]
    fn pending_source_cancel_retries_after_restart_without_closing_filled_positions() {
        let mut b = Atrapa::nowa(); b.ustaw_cene(TS0, 4004.0, 4004.2);
        let mut e = silnik(|c| c.explicit_pending_until_cancel = true);
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        let id = e.baskets[0].id;
        let ticket = b.open_market(OrderReq { side:Side::Buy, volume:0.01,
            sl:Some(3990.0), tp:Some(4020.0), basket:Some(id), level:99,
            is_toucher:false, comment:String::new() }).unwrap();
        b.cancel_failures = 100;
        let mut cancel = wiad(1, 9, None, "NO LONGER VALID"); cancel.reply_to = Some(1);
        e.on_message(&mut b, &cancel);
        assert!(!b.pendings.is_empty(), "fixture rejection must retain real broker orders");
        let source_json = serde_json::to_vec(&e.export_pending_source_memory()).unwrap();
        let baskets_json = serde_json::to_vec(&e.baskets).unwrap();
        let mut restarted = silnik(|c| { c.explicit_pending_until_cancel = true; c.entry_idempotencja = false; });
        restarted.restore_pending_source_memory(&serde_json::from_slice::<Vec<PendingSourceRecord>>(&source_json).unwrap());
        restarted.adopt_baskets(serde_json::from_slice(&baskets_json).unwrap());
        b.cancel_failures = 0;
        b.ustaw_cene(TS0 + 14 * 86_400_000, 4004.0, 4004.2);
        let q = b.quote(); restarted.on_tick(&mut b, &q);
        assert!(b.pendings.is_empty());
        assert_eq!(b.positions.iter().map(|p| p.ticket).collect::<Vec<_>>(), vec![ticket]);
        assert!(restarted.pending_source_cancelled(id));
        let mut repeated = wiad(1, 1, None, WEJSCIE); repeated.ts = q.ts;
        restarted.on_message(&mut b, &repeated);
        repeated.edit_of = Some(1); repeated.text = WEJSCIE.replace("4000/3995", "4001/3994");
        restarted.on_message(&mut b, &repeated);
        assert!(b.pendings.is_empty(), "re-delivery/edit cannot rearm a withdrawn source");
        assert_eq!(b.positions.len(), 1);
    }

    #[test]
    fn pending_source_revision_tracks_all_mutations_without_false_dirty_repeats() {
        let mut b = Atrapa::nowa(); b.ustaw_cene(TS0, 4004.0, 4004.2);
        let mut e = silnik(|c| { c.explicit_pending_until_cancel = true; c.reply_graph_transitive = true; });
        let initial = e.pending_source_memory_revision();
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        let accepted = e.pending_source_memory_revision(); assert_ne!(accepted, initial);
        let records = e.export_pending_source_memory();
        e.restore_pending_source_memory(&records);
        assert_eq!(accepted, e.pending_source_memory_revision(), "identical restore dirtied the ledger");
        let mut alias = wiad(1, 9, None, "TP1 HIT"); alias.reply_to = Some(1);
        e.on_message(&mut b, &alias);
        let aliased = e.pending_source_memory_revision(); assert_ne!(aliased, accepted);
        e.on_message(&mut b, &alias);
        assert_eq!(aliased, e.pending_source_memory_revision());
        let mut cancel = wiad(1, 10, None, "CANCEL"); cancel.reply_to = Some(9);
        e.on_message(&mut b, &cancel);
        let withdrawn = e.pending_source_memory_revision(); assert_ne!(withdrawn, aliased);
        e.on_message(&mut b, &cancel);
        assert_eq!(withdrawn, e.pending_source_memory_revision());
        let mut replacement = silnik(|_| {});
        assert_ne!(replacement.pending_source_memory_revision(), withdrawn,
            "engine replacement must invalidate the old persistence signature");
        replacement.restore_pending_source_memory(&e.export_pending_source_memory());
        let loaded = replacement.pending_source_memory_revision();
        replacement.restore_pending_source_memory(&e.export_pending_source_memory());
        assert_eq!(loaded, replacement.pending_source_memory_revision());
        assert_eq!(replacement.export_pending_source_memory(), e.export_pending_source_memory());
        // A restart after toggling the axis off can still adopt a newly known
        // alias from a compatible basket snapshot without dropping its source.
        let mut adopted = e.baskets[0].clone(); adopted.msg_aliases.push(99);
        replacement.adopt_baskets(vec![adopted]);
        assert_ne!(loaded, replacement.pending_source_memory_revision());
        assert!(replacement.export_pending_source_memory()[0].aliases.contains(&99));
    }

    #[test]
    fn pending_source_alias_survives_snapshot_then_actual_history_prune_and_restart() {
        let mut b = Atrapa::nowa(); b.ustaw_cene(TS0, 4004.0, 4004.2);
        let mut e = silnik(|c| { c.explicit_pending_until_cancel = true; c.reply_graph_transitive = true; });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        let id = e.baskets[0].id;
        let mut alias = wiad(1, 9, None, "TP1 HIT"); alias.reply_to = Some(1);
        e.on_message(&mut b, &alias);
        assert!(e.export_pending_source_memory()[0].aliases.contains(&9));
        e.close_everything(&mut b, TS0 + 1, CloseReason::MaxDd);
        let old = e.baskets[0].clone();
        for n in 2..=501 {
            let mut historical = old.clone(); historical.id = n; historical.msg_id = 1000 + n as i64;
            historical.msg_aliases.clear();
            e.baskets.push(historical);
        }
        b.ustaw_cene(TS0 + 14 * 86_400_000, 4004.0, 4004.2);
        let q = b.quote(); e.on_tick(&mut b, &q);
        assert!(e.baskets.is_empty(), "fixture must exercise real seven-day history pruning");
        let saved = e.export_pending_source_memory();
        assert!(saved[0].aliases.contains(&9), "later snapshot lost alias after pruning its basket");
        let mut restored = silnik(|c| c.explicit_pending_until_cancel = true);
        restored.restore_pending_source_memory(&saved);
        let mut cancel = wiad(1, 10, None, "CANCEL"); cancel.reply_to = Some(9); cancel.ts = q.ts + 1;
        restored.on_message(&mut b, &cancel);
        assert!(restored.pending_source_cancelled(id));
    }

    #[test]
    fn risk_exit_keeps_source_validity_and_disk_tombstone_outlives_broker_exposure() {
        let mut b = Atrapa::nowa(); b.ustaw_cene(TS0, 4004.0, 4004.2);
        let mut e = silnik(|c| c.explicit_pending_until_cancel = true);
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        let id = e.baskets[0].id;
        e.close_everything(&mut b, TS0 + 1, CloseReason::MaxDd);
        assert!(b.pendings.is_empty());
        assert!(e.keep_explicit_pending(id), "risk exit is not a publisher withdrawal");
        let mut restored = silnik(|c| c.explicit_pending_until_cancel = true);
        restored.restore_pending_source_memory(&e.export_pending_source_memory());
        assert!(restored.baskets.is_empty(), "no broker exposure is invented on restart");
        let mut cancel = wiad(1, 9, None, "CANCEL"); cancel.reply_to = Some(1);
        cancel.ts += 28 * 86_400_000;
        restored.on_message(&mut b, &cancel);
        assert!(restored.pending_source_cancelled(id));
        let mut final_restore = silnik(|c| { c.explicit_pending_until_cancel = false; c.entry_idempotencja = false; });
        final_restore.restore_pending_source_memory(&restored.export_pending_source_memory());
        let mut again = wiad(1, 1, None, WEJSCIE); again.ts = cancel.ts + 1;
        final_restore.on_message(&mut b, &again);
        assert!(b.pendings.is_empty(), "turning off a setting cannot clear a persisted source withdrawal");
        assert!(final_restore.baskets.is_empty());
    }

    #[test]
    fn close_guards_cancel_pending_only_exposure() {
        for guard in ["day-trail-pct", "day-trail-usd", "day-target-pct",
            "day-target-usd", "eod", "weekend"] {
            let mut b = Atrapa::nowa();
            // A Friday quote with no entry touch leaves a broker pending ladder.
            let ts = 1788537600000i64; // 2026-09-04 16:00 UTC
            b.ustaw_cene(ts, 4004.0, 4004.2);
            let mut e = silnik(|c| {
                match guard {
                    "day-trail-pct" => { c.day_trail_stop_pct = 1.0; c.day_trail_arm_pct = 0.0; }
                    "day-trail-usd" => c.day_trail_stop_usd = 1.0,
                    "day-target-pct" => { c.day_target_pct = 1.0; c.day_target_close = true; }
                    "day-target-usd" => { c.day_target_usd = 1.0; c.day_target_close = true; }
                    "eod" => c.eod_flat_hour = 16.0,
                    "weekend" => { c.flat_weekend = true; c.flat_weekend_hour = 16.0; }
                    _ => unreachable!(),
                }
            });
            let mut message = wiad(1, 1, None, WEJSCIE);
            message.ts = ts;
            e.on_message(&mut b, &message);
            assert!(b.positions.is_empty(), "{guard}");
            assert!(!b.pendings.is_empty(), "{guard}: fixture needs broker pending exposure");
            e.stats.day_start_equity = 400.0;
            e.stats.day_peak_equity = 500.0;
            e.stats.equity = 450.0;
            let q = b.quote();
            e.check_guards(&mut b, &q);
            assert!(b.pendings.is_empty(), "{guard}: pending orders survived a close guard");
        }
    }

    #[test]
    fn stopped_day_retries_pending_cancel_after_trigger_hour_passes() {
        for confirmed in [false, true] {
            let ts = 1788537600000i64; // Friday 16:00 UTC
            let mut b = Atrapa::nowa();
            b.ustaw_cene(ts, 4004.0, 4004.2);
            let mut e = silnik(|c| { c.eod_flat_hour = 16.0; c.confirmed_exit_retry = confirmed; });
            let mut message = wiad(1, 1, None, WEJSCIE);
            message.ts = ts;
            e.on_message(&mut b, &message);
            assert!(!b.pendings.is_empty());
            b.cancel_failures = 100;
            let q = b.quote();
            e.on_tick(&mut b, &q);
            assert!(!b.pendings.is_empty(), "fixture cancel must fail");
            assert_eq!(e.stopped_trading_day(), Some(day_of(ts, 0)));
            b.cancel_failures = 0;
            b.ustaw_cene(ts + 3_600_000, 4004.0, 4004.2);
            let q = b.quote();
            assert_ne!(hour_of(q.ts, 0), e.cfg.eod_flat_hour as u32);
            e.on_tick(&mut b, &q);
            assert!(b.pendings.is_empty(), "day stop forgot unconfirmed pending cancellation (confirmed={confirmed})");
        }
    }

    #[test]
    fn profit_peak_guard_closes_positions_and_stops_the_day_at_shared_threshold() {
        for (basis, closes) in [(DayTrailBasis::EquityPeak, false), (DayTrailBasis::ProfitPeak, true)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.honor_market_open = true;
                c.day_trail_stop_pct = 30.0;
                c.day_trail_arm_pct = 12.0;
                c.day_trail_basis = basis;
            });
            e.on_message(&mut b, &wiad(1, 1, None, "BUY NOW"));
            assert_eq!(b.positions.len(), 1);
            e.stats.day_start_equity = 400.0;
            e.stats.day_peak_equity = 480.0;
            e.stats.equity = 455.0; // gave back 25; ProfitPeak threshold is 24
            let q = b.quote();
            e.check_guards(&mut b, &q);
            assert_eq!(b.positions.is_empty(), closes);
            assert_eq!(e.day_stop == day_of(q.ts, e.cfg.session_offset()), closes);
        }
    }
}

#[cfg(test)]
mod testy_pakiet_b {
    use super::testy_pakiet_a::{silnik, wiad, Atrapa, TS0, WEJSCIE};
    use super::*;

    const WEJSCIE_MARKET: &str = "BUY GOLD @ 4000/3995\nTP 4010\nTP 4020\nSL 3990";

    #[test]
    fn b2_market_entry_units_scina_plan() {
        for (os, jednostek) in [(1u32, 1usize), (0u32, 5usize)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.auto_limit = false;
                c.entry_units = 5;
                c.market_entry_units = os;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_MARKET));
            assert_eq!(e.baskets.len(), 1, "sygnał rynkowy ma otworzyć koszyk");
            assert_eq!(
                b.positions().len() + b.pendings().len(),
                jednostek,
                "oś B2 = {os}: liczba jednostek wejścia rynkowego"
            );
        }
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.auto_limit = false;
            c.entry_units = 5;
            c.market_entry_units = 1;
        });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        assert_eq!(
            b.positions().len() + b.pendings().len(),
            5,
            "sygnał LIMITS musi zachować pełną siatkę mimo osi B2"
        );
    }

    #[test]
    fn b2_market_entry_units_dziala_takze_przy_auto_limit() {
        for (os, jednostek) in [(1u32, 1usize), (2u32, 2usize), (0u32, 5usize)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.auto_limit = true;
                c.entry_units = 5;
                c.market_entry_units = os;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_MARKET));
            assert_eq!(e.baskets.len(), 1, "sygnał rynkowy ma otworzyć koszyk");
            assert_eq!(
                b.positions().len() + b.pendings().len(),
                jednostek,
                "oś B2 przy auto_limit = true, market_entry_units = {os}"
            );
        }
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.auto_limit = true;
            c.entry_units = 5;
            c.market_entry_units = 1;
        });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        assert_eq!(
            b.positions().len() + b.pendings().len(),
            5,
            "sygnał LIMITS musi zachować pełną siatkę mimo osi B2 (auto_limit)"
        );
    }

    #[test]
    fn b2h_market_hybrid_otwiera_teraz_i_zostawia_dolne_limity() {
        let mut b = Atrapa::nowa();
        b.ustaw_cene(TS0, 4000.40, 4000.60);
        let mut e = silnik(|c| {
            c.auto_limit = true;
            c.entry_units = 5;
            c.market_entry_units = 1;
            c.market_hybrid_now_units = 1;
        });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_MARKET));
        assert_eq!(
            b.positions().len(),
            1,
            "hybryda ma wejść jedną nogą natychmiast"
        );
        assert_eq!(
            b.pendings().len(),
            4,
            "pozostałe cztery nogi mają czekać niżej"
        );
        assert_eq!(
            b.positions()[0].level,
            4,
            "rynek ma dostać pierwsze/płytkie wejście"
        );
        assert!(
            b.pendings()
                .iter()
                .all(|o| o.price < b.positions()[0].open_price),
            "po BUY natychmiastowym mają zostać wyłącznie niższe limity"
        );

        let mut b0 = Atrapa::nowa();
        b0.ustaw_cene(TS0, 4000.40, 4000.60);
        let mut e0 = silnik(|c| {
            c.auto_limit = true;
            c.entry_units = 5;
            c.market_entry_units = 1;
            c.market_hybrid_now_units = 0;
        });
        e0.on_message(&mut b0, &wiad(1, 1, None, WEJSCIE_MARKET));
        assert_eq!(b0.positions().len(), 0);
        assert_eq!(b0.pendings().len(), 1);

        let mut bl = Atrapa::nowa();
        bl.ustaw_cene(TS0, 4000.40, 4000.60);
        let mut el = silnik(|c| {
            c.auto_limit = true;
            c.entry_units = 5;
            c.market_entry_units = 1;
            c.market_hybrid_now_units = 1;
        });
        el.on_message(&mut bl, &wiad(1, 1, None, WEJSCIE));
        assert_eq!(
            bl.positions().len(),
            0,
            "jawne LIMITS nie mogą wejść hybrydą"
        );
        assert_eq!(bl.pendings().len(), 5);
    }

    #[test]
    fn b2c_tp1_bez_fillu_kasuje_tylko_pending_sygnalu_market() {
        for (os, zostaje) in [(true, false), (false, true)] {
            let mut b = Atrapa::nowa();
            b.ustaw_cene(TS0, 4000.40, 4000.60);
            let mut e = silnik(|c| {
                c.auto_limit = true;
                c.entry_units = 5;
                c.market_entry_units = 1;
                c.pending_lifetime = PendingLifetime::UntilTp2;
                c.market_unfilled_cancel_stage = if os { 1 } else { 0 };
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_MARKET));
            assert_eq!(b.positions().len(), 0);
            assert_eq!(b.pendings().len(), 1);
            e.on_message(&mut b, &wiad(1, 2, None, "TP1 HIT"));
            assert_eq!(
                !b.pendings().is_empty(),
                zostaje,
                "market_unfilled_cancel_stage={}",
                if os { 1 } else { 0 }
            );
        }

        let mut b = Atrapa::nowa();
        b.ustaw_cene(TS0, 4000.40, 4000.60);
        let mut e = silnik(|c| {
            c.auto_limit = true;
            c.entry_units = 5;
            c.pending_lifetime = PendingLifetime::UntilTp2;
            c.market_unfilled_cancel_stage = 1;
        });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
        assert!(!b.pendings().is_empty());
        e.on_message(&mut b, &wiad(1, 2, None, "TP1 HIT"));
        assert!(!b.pendings().is_empty(), "jawne LIMITS nadal żyją do TP2");
    }

    #[test]
    fn b2h_parametry_stroja_liczbe_lot_chase_i_cel() {
        let mut b = Atrapa::nowa();
        b.ustaw_cene(TS0, 4000.40, 4000.60); // chase BUY = 0,60
        let mut e = silnik(|c| {
            c.auto_limit = true;
            c.entry_units = 5;
            c.lot_fixed = 0.10;
            c.market_hybrid_now_units = 2;
            c.market_hybrid_pending_units = 2;
            c.market_hybrid_lot_mult = 0.5;
            c.market_hybrid_max_chase_usd = 1.0;
            c.market_hybrid_tp_stage = 2;
        });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_MARKET));
        assert_eq!(b.positions().len(), 2);
        assert_eq!(b.pendings().len(), 2);
        assert!(b.positions().iter().all(|p| (p.volume - 0.05).abs() < 1e-9));
        assert!(b.positions().iter().all(|p| p.tp == Some(4020.0)));

        let mut b2 = Atrapa::nowa();
        b2.ustaw_cene(TS0, 4000.40, 4000.60);
        let mut e2 = silnik(|c| {
            c.auto_limit = true;
            c.entry_units = 5;
            c.market_hybrid_now_units = 2;
            c.market_hybrid_pending_units = 2;
            c.market_hybrid_max_chase_usd = 0.5;
        });
        e2.on_message(&mut b2, &wiad(1, 1, None, WEJSCIE_MARKET));
        assert_eq!(b2.positions().len(), 0);
        assert_eq!(b2.pendings().len(), 5);
    }

    #[test]
    fn b3_risk_free_kasuje_pendingi() {
        for os in [true, false] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.auto_limit = true; // wszystkie szczeble jako limity
                c.entry_units = 4;
                c.pending_lifetime = PendingLifetime::Never;
                c.pending_cancel_on_riskfree = os;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            assert!(!b.pendings().is_empty(), "test wymaga wiszących limitów");
            assert!(!b.positions().is_empty(), "test wymaga otwartych pozycji");
            e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE 3999"));
            if os {
                assert!(
                    b.pendings().is_empty(),
                    "oś B3: po RISK FREE pendingi mają zniknąć"
                );
                assert!(
                    e.baskets[0].levels.iter().any(|g| g.cancelled),
                    "skasowane szczeble mają nosić znacznik `cancelled` (Z-7)"
                );
            } else {
                assert!(
                    !b.pendings().is_empty(),
                    "bez osi B3 pendingi żyją po RF — stare zachowanie"
                );
            }
        }
    }

    #[test]
    fn b4_bank_all_at_stage_zamyka_calosc() {
        for (os, pusto) in [(2u8, true), (0u8, false)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.auto_limit = false;
                c.entry_units = 3;
                c.bank_all_at_stage = os;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_MARKET));
            assert!(!b.positions().is_empty());
            e.on_message(&mut b, &wiad(1, 2, None, "TP1 HIT"));
            if os > 0 {
                assert!(
                    !b.positions().is_empty(),
                    "TP1 < N — bank całości jeszcze nie działa"
                );
            }
            e.on_message(&mut b, &wiad(1, 3, None, "TP2 HIT"));
            assert_eq!(
                b.positions().is_empty(),
                pusto,
                "oś B4 = {os}: stan pozycji po TP2"
            );
            if os > 0 {
                assert!(b.pendings().is_empty(), "bank całości kasuje też pendingi");
                assert!(
                    matches!(e.baskets[0].state, BasketState::Done),
                    "koszyk po banku całości ma być zakończony"
                );
            }
        }
    }

    #[test]
    fn rearm_decision_records_truthful_components_and_explicit_loss_threshold() {
        for (realized, threshold, block_secured, expected) in [
            (0.0, 0.0, false, true),      // incomplete legacy ledger
            (-16.08, 0.0, false, false),  // correct ledger, original threshold
            (-16.08, -13.0, false, true), // deliberate, bounded-by-threshold choice
            (-16.08, -12.0, false, false),
            (-16.08, -13.0, true, false), // explicit RF veto still wins
        ] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.auto_limit = true;
                c.entry_units = 4;
                c.risk_per_basket_pct = 0.0;
                c.max_portfolio_risk_pct = 0.0;
                c.rearm_grid_on_return = true;
                c.rearm_block_after_secured = block_secured;
                c.rearm_min_gap_min = 0.0;
                c.rearm_min_basket_profit = threshold;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            b.positions_mut().clear();
            b.pendings_mut().clear();
            b.ustaw_cene(TS0 + 60_000, 3997.25, 3997.45);
            let id = e.baskets[0].id;
            let ticket = b
                .open_market(OrderReq {
                    side: Side::Buy,
                    volume: 0.04,
                    sl: Some(3996.30),
                    tp: None,
                    basket: Some(id),
                    level: 0,
                    is_toucher: false,
                    comment: "causal fixture".into(),
                })
                .unwrap();
            b.positions_mut()[0].open_price = 3996.30;
            let bk = &mut e.baskets[0];
            bk.tickets = vec![ticket];
            bk.pendings.clear();
            bk.had_positions = true;
            bk.realized = realized;
            bk.secured = true;
            bk.state = BasketState::RiskFree;
            e.drain_journal();
            let q = b.quote();
            e.rearm_pass(&mut b, &q);
            assert_eq!(
                e.baskets[0].rearms,
                u32::from(expected),
                "realized={realized}, threshold={threshold}, secured_veto={block_secured}"
            );
            let events: Vec<_> = e
                .drain_journal()
                .into_iter()
                .filter(|ev| ev.reason == Some(RejectCode::GridRearmed))
                .collect();
            assert_eq!(events.len(), usize::from(expected));
            if let Some(ev) = events.first() {
                let number = |key: &str| ev.data[key].as_f64().unwrap();
                assert!((number("basket_realized_before") - realized).abs() < 1e-8);
                assert!((number("basket_open_pl_before") - 3.80).abs() < 1e-8);
                assert!((number("basket_pl") - realized - 3.80).abs() < 1e-8);
                assert_eq!(number("required_basket_pl"), threshold);
                assert_eq!(ev.data["snapshot_phase"], "before_rearm");
                assert_eq!(ev.data["secured_before"], true);
                assert_eq!(ev.data["pnl_gate_bypassed_no_previous_fill"], false);
                assert_eq!(ev.data["close_receipt_reconcile_active"], false);
                let bits = &ev.data["decision_inputs_f64_bits"];
                assert_eq!(bits["realized"], format!("{:016x}", realized.to_bits()));
                assert_eq!(bits["threshold"], format!("{:016x}", threshold.to_bits()));
                assert_eq!(bits["bid"], format!("{:016x}", q.bid.to_bits()));
                let snapshot = ev.market.unwrap();
                assert_eq!(snapshot.open_positions, 1, "snapshot predates new entries");
                assert_eq!(snapshot.open_pendings, 0);
                assert_eq!(snapshot.open_volume, 0.04);
            }
        }
    }

    #[test]
    fn confirmed_strategy_profit_is_shared_by_basket_day_and_observation_consumers() {
        let mut b=Atrapa::nowa();
        let mut e=silnik(|c| {c.auto_limit=true;c.entry_units=4;c.basket_realized_broker_only=true;});
        e.on_tick(&mut b,&Quote {ts:TS0,bid:3998.0,ask:3998.3});
        e.on_message(&mut b,&wiad(1,1,None,WEJSCIE));
        let basket=e.baskets[0].id;
        let mut expected=0.0;
        for (i,(open,profit)) in [4591.13,4592.19,4593.44,4594.43,4595.8].into_iter()
            .zip([-2.90,-1.84,-0.59,0.40,1.77]).enumerate() {
            let trade=ClosedTrade {ticket:100+i as u64,side:Side::Sell,volume:0.01,
                open_price:open,close_price:4594.03,open_ts:TS0-1000,close_ts:TS0+1,
                profit,commission:0.0,swap:0.0,reason:CloseReason::RiskFree,basket:Some(basket),
                profit_basis:Some(crate::cost_receipt::ProfitBasis::PriceOnlyGross),cost_receipt:None};
            expected+=trade.strategy_realized_profit(false).unwrap(); b.closed.push(trade);
        }
        b.ustaw_cene(TS0+1,3998.0,3998.3);let q=b.quote();e.on_tick(&mut b,&q);
        assert_eq!(e.basket(basket).unwrap().realized.to_bits(),expected.to_bits());
        assert_eq!(e.stats.realized_today.to_bits(),expected.to_bits());
        assert_eq!(e.closed_today.iter().sum::<f64>().to_bits(),expected.to_bits());
        assert_eq!(e.stats.trades,5);assert!(e.cost_reconciliation_required.is_none());
        assert_eq!(b.account().balance,400.0,"strategy bookkeeping never mutates broker cash");
    }

    #[test]
    fn missing_confirmed_strategy_geometry_latches_entry_hold_without_invented_profit() {
        let mut b=Atrapa::nowa();let mut e=silnik(|c|c.basket_realized_broker_only=true);
        let q=b.quote();e.on_tick(&mut b,&q);
        b.closed.push(ClosedTrade {ticket:100,side:Side::Sell,volume:0.01,open_price:0.0,
            close_price:4000.0,open_ts:0,close_ts:TS0,profit:2.0,commission:0.0,swap:0.0,
            reason:CloseReason::RiskFree,basket:None,
            profit_basis:Some(crate::cost_receipt::ProfitBasis::PriceOnlyGross),cost_receipt:None});
        e.on_tick(&mut b,&q);
        assert!(e.cost_reconciliation_required.as_deref().unwrap().contains("STRATEGY P/L"));
        assert_eq!(e.cost_quarantine.len(),1);assert_eq!(e.stats.realized_today,0.0);
        e.on_message(&mut b,&wiad(1,1,None,WEJSCIE));
        assert!(b.positions().is_empty() && b.pendings().is_empty());
    }

    #[test]
    fn rearm_po_risk_free_jest_przelaczalnie_zablokowany() {
        for (blokada, ma_przezbroic) in [(false, true), (true, false)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.auto_limit = true;
                c.entry_units = 4;
                c.rearm_grid_on_return = true;
                c.rearm_block_after_secured = blokada;
                c.rearm_min_gap_min = 0.0;
                c.rearm_min_basket_profit = 0.0;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            assert!(!e.baskets[0].levels.is_empty(), "test wymaga planu siatki");

            b.positions_mut().clear();
            b.pendings_mut().clear();
            let (lo, hi) = (e.baskets[0].zone_lo, e.baskets[0].zone_hi);
            {
                let bk = &mut e.baskets[0];
                bk.had_positions = true;
                bk.realized = 1.0;
                bk.secured = true;
                bk.state = BasketState::RiskFree;
            }
            let mid = (lo + hi) / 2.0;
            b.ustaw_cene(TS0 + 60_000, mid - 0.1, mid + 0.1);
            let q = b.quote();
            e.rearm_pass(&mut b, &q);

            let wystawione = b.positions().len() + b.pendings().len();
            assert_eq!(
                wystawione > 0,
                ma_przezbroic,
                "rearm_block_after_secured={blokada}: nieprawidłowa decyzja rearm"
            );
            assert_eq!(e.baskets[0].rearms, u32::from(ma_przezbroic));
        }
    }

    #[test]
    fn pusty_koszyk_czeka_na_powrot_tylko_z_jawnym_toggle_rearm() {
        for (rearm, zachowaj, ma_przezyc, ma_przezbroic) in [
            (true, false, false, false),
            (true, true, true, true),
            (false, true, false, false),
        ] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.auto_limit = true;
                c.entry_units = 4;
                c.rearm_grid_on_return = rearm;
                c.rearm_keep_empty_alive = zachowaj;
                c.rearm_min_gap_min = 0.0;
                c.rearm_min_basket_profit = 0.0;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            assert!(!e.baskets[0].levels.is_empty(), "test wymaga planu siatki");

            b.positions_mut().clear();
            b.pendings_mut().clear();
            {
                let bk = &mut e.baskets[0];
                bk.tickets.clear();
                bk.pendings.clear();
                bk.had_positions = true;
                bk.realized = 1.0;
                bk.state = BasketState::Working;
            }
            b.ustaw_cene(TS0 + 61_000, 4004.0, 4004.3);
            let q_poza = b.quote();
            e.on_tick(&mut b, &q_poza);
            assert_eq!(
                e.baskets[0].alive(),
                ma_przezyc,
                "rearm={rearm}, rearm_keep_empty_alive={zachowaj}: życie po pustym ticku"
            );

            b.ustaw_cene(TS0 + 62_000, 3997.8, 3998.1);
            let q_powrot = b.quote();
            e.on_tick(&mut b, &q_powrot);
            let wystawione = b.positions().len() + b.pendings().len();
            assert_eq!(
                wystawione > 0,
                ma_przezbroic,
                "rearm={rearm}, rearm_keep_empty_alive={zachowaj}: powrót do strefy"
            );
        }
    }

    #[test]
    fn spp_na_plaskim_koszyku_moze_jawnie_zablokowac_rearm() {
        const SPP: &str = "TP3 HIT +90 PIPS\n\nIM SECURING PARTIAL PROFITS HERE. \
                              SL IS SET TO BE AT 3998 AND I WILL TARGET;\n\n4010\n4020\n4030\n4040\n\n\
                              DO NOT UNDER ANY CIRCUMSTANCES ENTER THE MARKET AGAIN";
        for (blokada, ma_przezbroic) in [(false, true), (true, false)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.auto_limit = false;
                c.entry_units = 1;
                c.rearm_grid_on_return = true;
                c.rearm_keep_empty_alive = true;
                c.rearm_block_after_secured = true;
                c.spp_blocks_rearm_when_flat = blokada;
                c.rearm_min_gap_min = 0.0;
                c.rearm_min_basket_profit = 0.0;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE));
            b.positions_mut().clear();
            b.pendings_mut().clear();
            {
                let bk = &mut e.baskets[0];
                bk.tickets.clear();
                bk.pendings.clear();
                bk.had_positions = true;
                bk.realized = 1.0;
                bk.state = BasketState::Working;
            }
            e.on_message(&mut b, &wiad(1, 2, None, SPP));
            assert!(
                !e.baskets[0].secured,
                "płaski koszyk nie może udawać secured"
            );
            assert_eq!(e.baskets[0].rearm_blocked_by_spp, blokada);

            b.ustaw_cene(TS0 + 61_000, 3997.8, 3998.1);
            let q = b.quote();
            e.on_tick(&mut b, &q);
            assert_eq!(
                b.positions().len() + b.pendings().len() > 0,
                ma_przezbroic,
                "spp_blocks_rearm_when_flat={blokada}"
            );
        }
    }
}

#[cfg(test)]
mod testy_pakiet_f {
    use super::testy_pakiet_a::{silnik, wiad, zrodlo, Atrapa, TS0, WEJSCIE};
    use super::*;

    fn odpowiedz(chat: i64, msg_id: i64, reply_to: i64, text: &str) -> IncomingMessage {
        IncomingMessage {
            ts: TS0,
            source: zrodlo(chat),
            source_name: format!("TEST{chat}"),
            msg_id,
            reply_to: Some(reply_to),
            edit_of: None,
            text: text.into(),
        }
    }

    #[test]
    fn f1_odpowiedz_do_nieznanego_id_nie_rusza_koszyka() {
        for (os, oczekiwany_sl) in [(true, 3990.0), (false, 3993.0)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| c.reply_veto = os);
            e.on_message(&mut b, &wiad(1, 100, None, WEJSCIE));
            assert_eq!(e.baskets.len(), 1, "test wymaga jednego żywego koszyka");
            assert_eq!(e.baskets[0].sl, Some(3990.0), "SL z sygnału");

            assert!(
                !e.msg_to_basket.contains_key(&(zrodlo(1), 6160)),
                "test wymaga, żeby adresat był NIEZNANY"
            );
            e.on_message(&mut b, &odpowiedz(1, 6163, 6160, "MOVE SL TO 3993"));

            assert_eq!(
                e.baskets[0].sl,
                Some(oczekiwany_sl),
                "oś reply_veto = {os}: SL koszyka po cudzej odpowiedzi"
            );
            if os {
                assert_eq!(
                    e.odrzuty.get("NoTargetBasket").copied().unwrap_or(0)
                        + *e.odrzuty.get("no_target_basket").unwrap_or(&0),
                    0,
                    "weto odpowiedzi miało być ciche (bez wpisu w odrzuty) — \
                     jeśli dostało ślad, zaktualizuj kontrakt tego testu"
                );
            }
        }
    }

    #[test]
    fn f1_odpowiedz_do_znanego_id_dziala_normalnie() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| c.reply_veto = true);
        e.on_message(&mut b, &wiad(1, 100, None, WEJSCIE));
        assert!(
            e.msg_to_basket.contains_key(&(zrodlo(1), 100)),
            "sygnał ma zostawić ślad w mapie wiadomość→koszyk"
        );
        e.on_message(&mut b, &odpowiedz(1, 101, 100, "MOVE SL TO 3993"));
        assert_eq!(
            e.baskets[0].sl,
            Some(3993.0),
            "odpowiedź na WŁASNY sygnał musi przejść mimo weta"
        );
    }

    #[test]
    fn f5_reply_graph_jest_przechodni_i_trwaly() {
        for os in [false, true] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.reply_veto = true;
                c.reply_graph_transitive = os;
            });
            e.on_message(&mut b, &wiad(1, 100, None, WEJSCIE));
            e.on_message(&mut b, &odpowiedz(1, 101, 100, "MOVE SL TO 3993"));
            assert_eq!(e.baskets[0].sl, Some(3993.0));

            e.on_message(&mut b, &odpowiedz(1, 102, 101, "MOVE SL TO 3994"));
            assert_eq!(
                e.baskets[0].sl,
                Some(if os { 3994.0 } else { 3993.0 }),
                "przechodnia odpowiedź przy osi={os}"
            );

            let migawka = e.baskets[0].clone();
            let mut po = silnik(|c| {
                c.reply_veto = true;
                c.reply_graph_transitive = os;
            });
            po.adopt_baskets(vec![migawka]);
            po.on_message(&mut b, &odpowiedz(1, 103, 101, "MOVE SL TO 3995"));
            assert_eq!(
                po.baskets[0].sl,
                Some(if os { 3995.0 } else { 3993.0 }),
                "alias wiadomości ma przeżyć restart przy osi={os}"
            );
        }
    }

    #[test]
    fn f2a_hamulec_ma_wlasny_kod_odrzutu() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.slhit_pause_n = 1;
            c.slhit_pause_min = 60.0;
        });
        e.on_message(&mut b, &wiad(1, 1, None, "❌ SL HIT -30 PIPS"));

        let gate = e.entry_gate(&b, TS0 + 60_000);
        let (zdanie, kod) = gate.blocked().expect("hamulec ma zamknąć bramkę");
        assert_eq!(
            kod,
            RejectCode::SlHitBrake,
            "hamulec SL-HIT nie ma prawa dzielić kodu z pauzą po serii strat"
        );
        assert_ne!(kod, RejectCode::StreakPause);
        assert!(
            zdanie.contains("hamulec SL-HIT") && zdanie.contains('1'),
            "tekst bramki ma podać liczbę stopów kanału: {zdanie}"
        );
        assert_eq!(kod.as_str(), "sl_hit_brake");
    }

    #[test]
    fn f2b_miekki_hamulec_wpuszcza_z_polowa_lota() {
        let bazowy = |c: &mut Settings| {
            c.lot_mode_percent = false;
            c.lot_fixed = 0.10;
            c.lot_min = 0.01;
            c.auto_limit = false;
            c.entry_units = 1;
            c.risk_per_basket_pct = 0.0;
            c.slhit_pause_n = 1;
            c.slhit_pause_min = 60.0;
        };

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| bazowy(c));
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_MKT));
        let pelny: f64 = b.positions().iter().map(|p| p.volume).sum();
        assert!(pelny > 0.0, "test wymaga otwartej pozycji odniesienia");

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| bazowy(c));
        e.on_message(&mut b, &wiad(1, 1, None, "❌ SL HIT -30 PIPS"));
        e.on_message(&mut b, &wiad(1, 2, None, WEJSCIE_MKT));
        assert!(
            b.positions().is_empty() && b.pendings().is_empty(),
            "przy mnożniku 0 hamulec ma BLOKOWAĆ, jak przed 18.08"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            bazowy(c);
            c.slhit_pause_lot_mult = 0.5;
        });
        e.on_message(&mut b, &wiad(1, 1, None, "❌ SL HIT -30 PIPS"));
        e.on_message(&mut b, &wiad(1, 2, None, WEJSCIE_MKT));
        let miekki: f64 = b.positions().iter().map(|p| p.volume).sum();
        assert!(
            miekki > 0.0,
            "miękki hamulec ma WPUŚCIĆ wejście, nie zablokować"
        );
        assert!(
            (miekki - pelny * 0.5).abs() < 1e-9,
            "miękki hamulec: {miekki} zamiast połowy z {pelny}"
        );

        assert!(
            !e.slhit_miekki,
            "flaga miękkiego hamulca musi zgasnąć po wejściu"
        );
    }

    const WEJSCIE_MKT: &str = "BUY GOLD @ 4000/3995\nTP 4010\nTP 4020\nSL 3990";

    #[test]
    fn f1_komunikat_bez_odpowiedzi_trafia_do_najnowszego() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| c.reply_veto = true);
        e.on_message(&mut b, &wiad(1, 100, None, WEJSCIE));
        e.on_message(&mut b, &wiad(1, 200, None, "MOVE SL TO 3993"));
        assert_eq!(
            e.baskets[0].sl,
            Some(3993.0),
            "komunikat bez odpowiedzi ma iść do najnowszego koszyka mimo weta"
        );
    }
}

#[cfg(test)]
mod testy_pakiet_g {
    use super::testy_pakiet_a::{silnik, wiad, Atrapa, TS0};
    use super::*;

    const WEJSCIE_NISKO: &str = "BUY LIMITS GOLD @ 3990/3985\nTP 4010\nTP 4020\nSL 3980";

    fn krata(c: &mut Settings) {
        c.lot_mode_percent = false;
        c.lot_fixed = 0.01;
        c.lot_min = 0.01;
        c.auto_limit = false;
        c.risk_per_basket_pct = 0.0;
        c.max_open_positions = 0;
        c.entry_units = 3;
        c.entry_units_limit = 3;
        c.ppm = 1.0;
        c.ppm_enabled = true;
        c.ppm_for_limits = true;
        c.grid_anchor_absolute = true;
    }

    #[test]
    fn g1_units_per_level_rozdziela_kotwice_od_mnoznika() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(krata);
        assert!(
            e.cfg.units_per_level,
            "oś G1 ma być domyślnie włączona (parytet)"
        );
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_NISKO));
        let poziomy = e.baskets[0].levels.iter().filter(|g| !g.is_toucher).count();
        let z_mnoznikiem = b.pendings().len();
        assert!(
            poziomy > 1,
            "test wymaga kraty o wielu poziomach, mam {poziomy}"
        );
        assert_eq!(
            z_mnoznikiem,
            poziomy * 3,
            "przy domyślnym `units_per_level` na poziomie ma leżeć komplet jednostek"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            krata(c);
            c.units_per_level = false;
        });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_NISKO));
        let poziomy2 = e.baskets[0].levels.iter().filter(|g| !g.is_toucher).count();
        assert_eq!(
            poziomy2, poziomy,
            "kotwica ma zostać nietknięta — te same poziomy"
        );
        assert_eq!(
            b.pendings().len(),
            poziomy,
            "przy `units_per_level = false` na poziomie ma leżeć DOKŁADNIE jedno zlecenie"
        );
    }

    #[test]
    fn g1_bez_kotwicy_os_nic_nie_zmienia() {
        let bez_kotwicy = |c: &mut Settings| {
            krata(c);
            c.grid_anchor_absolute = false;
        };
        let mut b1 = Atrapa::nowa();
        let mut e1 = silnik(bez_kotwicy);
        e1.on_message(&mut b1, &wiad(1, 1, None, WEJSCIE_NISKO));

        let mut b2 = Atrapa::nowa();
        let mut e2 = silnik(|c| {
            bez_kotwicy(c);
            c.units_per_level = false;
        });
        e2.on_message(&mut b2, &wiad(1, 1, None, WEJSCIE_NISKO));

        assert_eq!(
            b1.pendings().len(),
            b2.pendings().len(),
            "bez kotwicy oś G1 nie ma prawa niczego zmienić"
        );
    }

    const WEJSCIE_NISKO_STREFA: &str = "BUY GOLD @ 3990/3985\nTP 4010\nTP 4020\nSL 3980";

    #[test]
    fn bug38r_strefa_bez_limit_nie_dostaje_kraty() {
        for os in [true, false] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                krata(c);
                c.units_per_level_zone = os;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_NISKO));
            let poziomy = e.baskets[0].levels.iter().filter(|g| !g.is_toucher).count();
            assert!(
                poziomy > 1,
                "test wymaga kraty o wielu poziomach, mam {poziomy}"
            );
            assert_eq!(
                b.pendings().len(),
                poziomy * 3,
                "oś = {os}: jawny LIMIT ma dostawać kratę historyczną — komplet \
                 jednostek na poziom"
            );
        }

        for (os, mnoznik) in [(true, 3), (false, 1)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                krata(c);
                c.auto_limit = true;
                c.units_per_level_zone = os;
            });
            if os {
                assert!(
                    e.cfg.units_per_level_zone,
                    "kontrakt zera: oś ma być domyślnie włączona (stare zachowanie)"
                );
            }
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_NISKO_STREFA));
            assert_eq!(e.baskets.len(), 1, "sygnał strefowy ma otworzyć koszyk");
            let poziomy = e.baskets[0].levels.iter().filter(|g| !g.is_toucher).count();
            assert!(
                poziomy > 1,
                "test wymaga wielu poziomów w strefie, mam {poziomy}"
            );
            assert_eq!(
                b.pendings().len(),
                poziomy * mnoznik,
                "oś = {os}: strefa bez słowa LIMIT — {} (koszyk B1 z 24.08 to \
                 dokładnie mnożnik na strefie)",
                if os {
                    "stare zachowanie, komplet na poziom"
                } else {
                    "jedno zlecenie na poziom, mnożnik wyłączony"
                }
            );
        }

        for os in [true, false] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                krata(c);
                c.auto_limit = true;
                c.grid_anchor_absolute = false;
                c.units_per_level_zone = os;
            });
            e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_NISKO_STREFA));
            let poziomy = e.baskets[0].levels.iter().filter(|g| !g.is_toucher).count();
            assert_eq!(
                b.pendings().len(),
                poziomy,
                "oś = {os}: bez kotwicy zawsze było jedno zlecenie na poziom — \
                 naprawa nie ma prawa tego ruszyć"
            );
        }
    }
}

#[cfg(test)]
mod testy_sekcja_m_1808 {
    use super::testy_pakiet_a::{silnik, wiad, Atrapa, TS0};
    use super::*;

    const WEJSCIE_M3: &str = "BUY LIMITS GOLD @ 4000/3995\nTP 4010\nTP 4020\nSL 3994";

    fn geometria(c: &mut Settings) {
        c.zone_offset_mode = ZoneOffsetMode::Directional;
        c.entry_deep_offset = 3.0;
        c.entry_tol_offset = 0.0;
        c.sl_min_dist = 3.0;
        c.sl_max_dist = 0.0;
        c.adaptive_params = false;
    }

    #[test]
    fn m3_glebokosc_jako_ulamek_wyprowadza_dno_siatki_znad_stopu() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(geometria);
        assert_eq!(
            e.cfg.entry_deep_frac_to_sl, 0.0,
            "oś M3 ma być domyślnie wyłączona"
        );
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_M3));
        let bk = &e.baskets[0];
        let (dno_przed, sl_przed) = (bk.zone_lo, bk.sl.expect("sygnał ma stop"));
        assert!(
            dno_przed < sl_przed,
            "test ma odtwarzać BŁĄD M3: dno {dno_przed:.2} powinno leżeć pod stopem {sl_przed:.2}"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            geometria(c);
            c.entry_deep_frac_to_sl = 0.667;
        });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_M3));
        let bk = &e.baskets[0];
        let (dno_po, sl_po) = (bk.zone_lo, bk.sl.expect("sygnał ma stop"));
        assert!(
            dno_po > sl_po,
            "po włączeniu osi dno {dno_po:.3} musi leżeć NAD stopem {sl_po:.3}"
        );
        assert!(
            (dno_po - (3995.0 - 0.667)).abs() < 1e-9,
            "głębokość ma być ułamkiem dystansu do stopu, mam dno {dno_po:.4}"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            geometria(c);
            c.entry_deep_frac_to_sl = 0.667;
        });
        e.on_message(
            &mut b,
            &wiad(1, 1, None, "BUY LIMITS GOLD @ 4000/3995\nTP 4010\nTP 4020"),
        );
        assert!(
            (e.baskets[0].zone_lo - 3992.0).abs() < 1e-9,
            "bez stopu w sygnale ma obowiązywać stałe `entry_deep_offset`, mam {:.4}",
            e.baskets[0].zone_lo
        );
    }

    fn koszyk_z_pozycja(b: &mut Atrapa, e: &mut Engine, open: Px) -> Ticket {
        e.on_message(
            b,
            &wiad(
                1,
                1,
                None,
                "BUY LIMITS GOLD @ 4000/3995\nTP 4010\nTP 4020\nSL 3990",
            ),
        );
        let id = e.baskets[0].id;
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.10,
                sl: None,
                tp: Some(4010.0),
                basket: Some(id),
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .expect("atrapa nie odmawia");
        if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
            p.open_price = open;
        }
        e.baskets[0].tickets.push(t);
        t
    }

    #[test]
    fn m4_prog_zysku_oddziela_miejsce_stopu_od_posluszenstwa() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.be_offset = 21.0;
            c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
        });
        assert_eq!(
            e.cfg.risk_free_be_min_profit, 0.0,
            "oś M4 ma być domyślnie wyłączona"
        );
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE NOW"));
        assert_eq!(
            b.find_position(t).and_then(|p| p.sl),
            None,
            "to jest BŁĄD M4: przy be_offset 21 breakeven leżałby 4011, \
             czyli nad rynkiem 3998 — `sl_is_valid` odrzuca i stop zostaje pusty"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.be_offset = 0.3;
            c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
        });
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE NOW"));
        assert_eq!(
            b.find_position(t).and_then(|p| p.sl),
            Some(3990.3),
            "przy be_offset 0,3 stop ma wylądować 0,30 $ nad wejściem"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.be_offset = 0.3;
            c.risk_free_be_min_profit = 21.0;
            c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
        });
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE NOW"));
        assert_eq!(
            b.find_position(t).and_then(|p| p.sl),
            None,
            "przy zysku 8 $ i progu 21 $ stop nie ma prawa drgnąć"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.be_offset = 0.3;
            c.risk_free_be_min_profit = 21.0;
            c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
        });
        let t = koszyk_z_pozycja(&mut b, &mut e, 3968.0);
        e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE NOW"));
        assert_eq!(
            b.find_position(t).and_then(|p| p.sl),
            Some(3968.3),
            "przy 30 $ zysku próg 21 $ jest spełniony i stop musi się przesunąć"
        );
    }

    #[test]
    fn m15_limit_trzymania_runnera_dziala_takze_bez_reguly() {
        let jak_produkcja = |c: &mut Settings| {
            c.riskfree_enabled = false;
            c.riskfree_runner_max_hold_min = 90.0;
            c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
            c.risk_free_runner_target = RiskFreeRunnerTarget::NoTpTrailOnly;
            c.be_offset = 21.0;
            c.trail_mode = TrailMode::Off;
            c.trail_runner_mode = TrailMode::Off;
            c.basket_max_age_min = 0.0;
            c.oae_timeout_min = 0.0;
            c.stale_take_min = 0.0;
            c.stale_take_min2 = 0.0;
            c.harvest_retrace_pct = 0.0;
            c.smart_exit = false;
        };
        let pozniej = Quote {
            ts: TS0 + 100 * 60_000,
            bid: 3998.0,
            ask: 3998.3,
        };

        let mut b = Atrapa::nowa();
        let mut e = silnik(jak_produkcja);
        assert!(
            !e.cfg.runner_max_hold_bez_reguly,
            "oś M15 ma być domyślnie wyłączona"
        );
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE NOW"));
        assert!(
            e.baskets[0].secured,
            "komunikat ma oznaczyć koszyk jako zabezpieczony"
        );
        assert_eq!(
            b.find_position(t).and_then(|p| p.tp),
            None,
            "`NoTpTrailOnly` ma zdjąć cel — to jest runner bez sufitu"
        );
        e.on_tick(&mut b, &pozniej);
        assert!(
            b.find_position(t).is_some(),
            "to jest BŁĄD M15: bez celu, bez trailingu, bez BE i bez terminu \
             pozycja zostaje otwarta 100 minut po RISK FREE"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            jak_produkcja(c);
            c.runner_max_hold_bez_reguly = true;
        });
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE NOW"));
        e.on_tick(&mut b, &pozniej);
        assert!(
            b.find_position(t).is_none(),
            "po włączeniu `runner_max_hold_bez_reguly` runner ma zniknąć po 90 minutach"
        );

        let wczesniej = Quote {
            ts: TS0 + 80 * 60_000,
            bid: 3998.0,
            ask: 3998.3,
        };
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            jak_produkcja(c);
            c.runner_max_hold_bez_reguly = true;
        });
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE NOW"));
        e.on_tick(&mut b, &wczesniej);
        assert!(
            b.find_position(t).is_some(),
            "limit 90 min nie ma prawa domykać po 80 minutach"
        );
    }

    #[test]
    fn m15_konfiguracja_bez_wyjscia_jest_zglaszana() {
        let mut c = Settings::default();
        c.risk_free_runner_target = RiskFreeRunnerTarget::NoTpTrailOnly;
        c.trail_mode = TrailMode::Off;
        c.trail_runner_mode = TrailMode::Off;
        c.be_offset = 21.0;
        c.riskfree_enabled = false;
        c.riskfree_runner_max_hold_min = 90.0;
        assert!(
            c.pulapki_konfiguracji()
                .iter()
                .any(|s| s.contains("RUNNER NIE MA WYJŚCIA")),
            "preset produkcyjny z 18.08 musi być zgłoszony jako pułapka"
        );
        assert!(
            c.martwe_ustawienia()
                .iter()
                .any(|m| m.pole == "riskfree_runner_max_hold_min"),
            "limit trzymania zbramkowany regułą musi być zgłoszony jako martwy"
        );

        c.runner_max_hold_bez_reguly = true;
        assert!(
            !c.pulapki_konfiguracji()
                .iter()
                .any(|s| s.contains("RUNNER NIE MA WYJŚCIA")),
            "po włączeniu limitu runner ma wyjście i ostrzeżenie ma zniknąć"
        );
        assert!(
            !c.martwe_ustawienia()
                .iter()
                .any(|m| m.pole == "riskfree_runner_max_hold_min"),
            "limit przestaje być martwy"
        );

        c.runner_max_hold_rule_only = true;
        assert!(
            c.martwe_ustawienia()
                .iter()
                .any(|m| m.pole == "riskfree_runner_max_hold_min"),
            "`rule_only` przy wyłączonej regule zostawia zegar bez koszyków — to też martwe pole"
        );
        assert!(
            c.pulapki_konfiguracji()
                .iter()
                .any(|s| s.contains("RUNNER NIE MA WYJŚCIA")),
            "zegar, który nie ma czego domykać, nie jest wyjściem dla runnera"
        );
    }
}

#[cfg(test)]
mod testy_fala_audyt_2408 {
    use super::testy_pakiet_a::{silnik, wiad, Atrapa, TS0};
    use super::*;

    const WEJSCIE_POD: &str = "BUY LIMITS GOLD @ 3990/3985\nTP 4010\nTP 4020\nSL 3980";

    struct Kaprys {
        w: Atrapa,
        mody: u32,
        zamk: u32,
        anul: u32,
        tryb_modify: u8,
        tryb_close: u8,
        tryb_cancel: u8,
    }

    impl Kaprys {
        fn nowy(tryb_modify: u8, tryb_close: u8) -> Self {
            Kaprys {
                w: Atrapa::nowa(),
                mody: 0,
                zamk: 0,
                anul: 0,
                tryb_modify,
                tryb_close,
                tryb_cancel: 0,
            }
        }
        fn odmowa(tryb: u8, licznik: u32) -> bool {
            match tryb {
                1 => true,
                2 => licznik % 2 == 1,
                _ => false,
            }
        }
    }

    impl Broker for Kaprys {
        fn quote(&self) -> Quote {
            self.w.quote()
        }
        fn account(&self) -> Account {
            self.w.account()
        }
        fn stops_level(&self) -> f64 {
            self.w.stops_level()
        }
        fn positions(&self) -> &[Position] {
            self.w.positions()
        }
        fn pendings(&self) -> &[PendingOrder] {
            self.w.pendings()
        }
        fn positions_mut(&mut self) -> &mut Vec<Position> {
            self.w.positions_mut()
        }
        fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
            self.w.pendings_mut()
        }
        fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> {
            self.w.open_market(r)
        }
        fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> {
            self.w.place_pending(r)
        }
        fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
            self.mody += 1;
            if Self::odmowa(self.tryb_modify, self.mody) {
                return Err(BrokerError::InvalidStops);
            }
            self.w.modify_position(t, sl, tp)
        }
        fn modify_pending(
            &mut self,
            t: Ticket,
            price: Px,
            sl: Option<Px>,
            tp: Option<Px>,
        ) -> BResult<()> {
            self.w.modify_pending(t, price, sl, tp)
        }
        fn close_position(&mut self, t: Ticket, r: CloseReason) -> BResult<f64> {
            self.zamk += 1;
            if Self::odmowa(self.tryb_close, self.zamk) {
                return Err(BrokerError::MarketClosed);
            }
            self.w.close_position(t, r)
        }
        fn close_partial(&mut self, t: Ticket, v: f64, r: CloseReason) -> BResult<f64> {
            self.w.close_partial(t, v, r)
        }
        fn cancel_pending(&mut self, t: Ticket) -> BResult<()> {
            self.anul += 1;
            if Self::odmowa(self.tryb_cancel, self.anul) {
                return Err(BrokerError::MarketClosed);
            }
            self.w.cancel_pending(t)
        }
        fn drain_closed(&mut self) -> Vec<ClosedTrade> {
            self.w.drain_closed()
        }
    }

    fn bez_zamykaczy(c: &mut Settings) {
        c.basket_max_age_min = 0.0;
        c.oae_timeout_min = 0.0;
        c.stale_take_min = 0.0;
        c.stale_take_min2 = 0.0;
        c.harvest_retrace_pct = 0.0;
        c.smart_exit = false;
        c.trail_mode = TrailMode::Off;
        c.trail_runner_mode = TrailMode::Off;
    }

    fn koszyk_z_pozycja<B: Broker>(b: &mut B, e: &mut Engine, open: Px) -> Ticket {
        if e.baskets.is_empty() {
            e.on_message(b, &wiad(1, 1, None, WEJSCIE_POD));
        }
        let id = e.baskets[0].id;
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.10,
                sl: None,
                tp: Some(4010.0),
                basket: Some(id),
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .expect("otwarcie rynkowe w atrapie nie odmawia");
        if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
            p.open_price = open;
        }
        e.baskets[0].tickets.push(t);
        t
    }


    #[test]
    fn rf_z_kanalu_ponawia_be_po_odmowie_brokera() {
        let mut b = Kaprys::nowy(2, 0);
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.be_offset = 0.3;
            c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
        });
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE NOW"));
        assert_eq!(
            b.find_position(t).and_then(|p| p.sl),
            None,
            "pierwsza modyfikacja miała zostać odrzucona (broker-kapryśnik)"
        );
        assert!(
            e.desired.contains_key(&t),
            "odrzucone BE musi zostać ZAMIAREM (desired), nie zniknąć bez śladu"
        );

        e.on_tick(
            &mut b,
            &Quote {
                ts: TS0 + 4_000,
                bid: 3998.0,
                ask: 3998.3,
            },
        );
        assert_eq!(
            b.find_position(t).and_then(|p| p.sl),
            Some(3990.3),
            "retry musi dowieźć BE mimo pierwszej odmowy brokera"
        );
        assert!(
            e.desired.is_empty(),
            "dowieziony zamiar ma zniknąć z kolejki"
        );
    }

    #[test]
    fn is_runner_dopiero_po_ok_brokera() {
        let mut b = Kaprys::nowy(1, 0);
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.no_tp_after_stage = 1;
        });
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.on_message(&mut b, &wiad(1, 2, None, "TP1 HIT +48 PIPS"));
        let p = b.find_position(t).expect("pozycja zyje").clone();
        assert!(
            p.tp.is_some(),
            "broker odrzucił każdą modyfikację — cel musiał zostać na pozycji"
        );
        assert!(
            !p.is_runner,
            "is_runner przy wiszącym TP to rozjazd stanu: silnik prowadziłby \
             runnera, którego broker zamknie na pierwszym dotknięciu celu"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.no_tp_after_stage = 1;
        });
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.on_message(&mut b, &wiad(1, 2, None, "TP1 HIT +48 PIPS"));
        let p = b.find_position(t).expect("pozycja zyje").clone();
        assert!(
            p.is_runner,
            "po przyjętym zdjęciu celu pozycja jest runnerem"
        );
    }

    #[test]
    fn close_profitable_only_bez_falszywego_secured() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.risk_free_mode = RiskFreeMode::CloseProfitableOnly;
        });
        let _zyskowna = koszyk_z_pozycja(&mut b, &mut e, 3990.0); // +8 $
        let stratna = koszyk_z_pozycja(&mut b, &mut e, 4006.0); // -8 $
        e.on_message(&mut b, &wiad(1, 2, None, "RISK FREE NOW"));

        assert_eq!(
            b.positions().len(),
            1,
            "zyskowna zamknięta, stratna zostaje"
        );
        assert_eq!(b.positions()[0].ticket, stratna);
        let bk = &e.baskets[0];
        assert_eq!(bk.state, BasketState::RiskFree, "stan koszyka jak dotąd");
        assert!(
            !bk.secured,
            "secured przy żywej stratnej pozycji kłamie: OAE pominąłby koszyk \
             jako zabezpieczony, a schodkowy SL wziąłby BE za podłogę"
        );
    }

    #[test]
    fn riskfree_pass_sekuruje_dopiero_po_realnych_zamknieciach() {
        let mut b = Kaprys::nowy(0, 2);
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.riskfree_enabled = true;
            c.riskfree_trigger_usd = 50.0;
            c.riskfree_keep_units = 1;
        });
        for _ in 0..3 {
            koszyk_z_pozycja(&mut b, &mut e, 3990.0); // po +8 $ każda
        }
        let tik = |ts: Ts| Quote {
            ts,
            bid: 3998.0,
            ask: 3998.3,
        };

        e.on_tick(&mut b, &tik(TS0 + 1_000));
        assert_eq!(
            b.positions().len(),
            2,
            "jedno zamknięcie przeszło, jedno odrzucone"
        );
        assert!(
            !e.baskets[0].secured,
            "bank niepełny (broker odmówił) — koszyk NIE jest wolny od ryzyka"
        );

        e.on_tick(&mut b, &tik(TS0 + 2_000));
        assert!(!e.baskets[0].secured, "kolejna odmowa = dalej bez secured");

        e.on_tick(&mut b, &tik(TS0 + 3_000));
        assert_eq!(b.positions().len(), 1, "został sam runner");
        assert!(
            e.baskets[0].secured,
            "po domknięciu CAŁEGO banku koszyk jest zabezpieczony"
        );
    }


    #[test]
    fn bank_all_rejected_exit_is_not_lost_after_reconciliation() {
        let mut b = Kaprys::nowy(0, 1);
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.tp_schedule = TpSchedule::AllRunners;
            c.bank_all_at_stage = 3;
            c.tp_hit_fill_stages = false;
            c.tp_source = TpSource::PriceOnly;
            c.basket_realized_broker_only = true;
            c.confirmed_exit_retry = true;
        });
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        let id = e.baskets[0].id;
        e.baskets[0].tps = vec![4010.0, 4020.0, 4030.0];
        e.baskets[0].state = BasketState::Working;
        e.baskets[0].had_positions = true;
        b.positions_mut()
            .iter_mut()
            .find(|p| p.ticket == t)
            .unwrap()
            .tp = None;
        b.w.ustaw_cene(TS0 + 1_000, 4030.0, 4030.3);
        e.handle_tp_hit(&mut b, id, Some(3), TS0 + 1_000, "price");
        assert!(
            b.find_position(t).is_some(),
            "rejected close leaves a live position"
        );

        let q = Quote {
            ts: TS0 + 2_000,
            bid: 4031.0,
            ask: 4031.3,
        };
        b.w.ustaw_cene(q.ts, q.bid, q.ask);
        e.on_tick(&mut b, &q);
        eprintln!(
            "P0 bank rejection: after reconciliation state={:?}, close_attempts={}, live={}",
            e.baskets[0].state,
            b.zamk,
            b.find_position(t).is_some()
        );
        assert!(
            e.baskets[0].alive(),
            "rejected final TP must not leave a Done basket with live exposure"
        );

        b.tryb_close = 0;
        let q = Quote {
            ts: TS0 + 4_000,
            bid: 4028.0,
            ask: 4028.3,
        };
        b.w.ustaw_cene(q.ts, q.bid, q.ask);
        e.on_tick(&mut b, &q);
        assert!(
            b.find_position(t).is_none(),
            "accepted retry must honor the already committed exit even after pullback"
        );
        assert_eq!(e.baskets[0].state, BasketState::Done);
        assert!(e.baskets[0].pending_exit.is_none());
    }

    fn exit_retry_engine() -> Engine {
        silnik(|c| {
            bez_zamykaczy(c);
            c.confirmed_exit_retry = true;
            c.basket_realized_broker_only = true;
            c.tp_schedule = TpSchedule::AllRunners;
            c.bank_all_at_stage = 3;
            c.tp_hit_fill_stages = false;
            c.tp_source = TpSource::PriceOnly;
            c.rearm_grid_on_return = true;
            c.rearm_min_gap_min = 0.0;
            c.reenter_after_tp = true;
            c.no_reenter_from_stage = 0;
        })
    }

    #[test]
    fn confirmed_exit_permanent_failure_has_bounded_retry_and_no_reentry() {
        let mut b = Kaprys::nowy(0, 1);
        let mut e = exit_retry_engine();
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        let id = e.baskets[0].id;
        e.baskets[0].tp_stage = 3;
        e.baskets[0].had_positions = true;
        e.baskets[0].realized = 100.0; // rearm's profit gate would otherwise pass
        e.close_basket(&mut b, id, TS0, CloseReason::Tp);
        assert!(e.baskets[0].pending_exit.is_some());
        for n in 1..=20 {
            let q = Quote {
                ts: TS0 + n * 100,
                bid: 3989.0,
                ask: 3989.3,
            };
            b.w.ustaw_cene(q.ts, q.bid, q.ask);
            e.on_tick(&mut b, &q);
            assert_eq!(e.sync_grid(&mut b, id, q.ts, false), 0);
            e.place_grid(&mut b, id, q.ts);
            assert!(!e.ea_dokladki_wolno(&b, &q, id));
            assert_eq!(
                b.positions().iter().map(|p| p.ticket).collect::<Vec<_>>(),
                vec![t]
            );
            assert!(b.pendings().is_empty());
            assert!(e.baskets[0].alive());
        }
        assert_eq!(b.zamk, 3, "one initial attempt plus one per elapsed second");
        let before = e.baskets[0].tps.clone();
        let mut edit = wiad(
            1,
            1,
            Some(1),
            "BUY LIMITS GOLD @ 3991/3980\nTP 4100\nTP 4200\nSL 3970",
        );
        edit.ts = TS0 + 2100;
        e.on_message(&mut b, &edit);
        assert_eq!(
            e.baskets[0].tps, before,
            "entry edits cannot replace the committed exit"
        );
        e.handle_tp_hit(&mut b, id, Some(4), TS0 + 2200, "late TP");
        assert!(b.pendings().is_empty(), "no pyramiding from late TP");
    }

    #[test]
    fn confirmed_exit_partial_success_only_done_after_remaining_close() {
        let mut b = Kaprys::nowy(0, 2);
        let mut e = exit_retry_engine();
        for _ in 0..3 {
            koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        }
        let id = e.baskets[0].id;
        let (closed, _, _) = e.close_basket(&mut b, id, TS0, CloseReason::BasketClose);
        assert_eq!(closed, 1);
        assert_eq!(b.positions().len(), 2);
        assert_eq!(e.baskets[0].tickets.len(), 2);
        assert!(e.baskets[0].alive());
        b.tryb_close = 0;
        e.retry_confirmed_exits(&mut b, TS0 + 1000);
        assert!(b.positions().is_empty());
        assert_eq!(e.baskets[0].state, BasketState::Done);
        assert!(e.baskets[0].pending_exit.is_none());
    }

    #[test]
    fn confirmed_exit_cancel_failure_and_late_fill_are_reconciled_from_broker() {
        let mut b = Kaprys::nowy(0, 0);
        let mut e = exit_retry_engine();
        koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        let id = e.baskets[0].id;
        assert!(
            !b.pendings().is_empty(),
            "fixture must contain entry exposure"
        );
        b.tryb_cancel = 1;
        e.close_basket(&mut b, id, TS0, CloseReason::Tp);
        assert!(b.positions().is_empty());
        assert!(!e.baskets[0].pendings.is_empty());
        assert!(e.baskets[0].alive(), "pending-only exposure is not Done");
        b.pendings_mut().clear();
        let fill = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.01,
                sl: None,
                tp: None,
                basket: Some(id),
                level: 99,
                is_toucher: false,
                comment: format!("B{id}"),
            })
            .unwrap();
        e.baskets[0].tickets.clear();
        e.baskets[0].pendings.clear();
        b.tryb_cancel = 0;
        e.retry_confirmed_exits(&mut b, TS0 + 1000);
        assert!(
            b.find_position(fill).is_none(),
            "retry discovers late fill absent from cached tickets"
        );
        assert_eq!(e.baskets[0].state, BasketState::Done);
    }

    #[test]
    fn confirmed_exit_pending_only_cannot_bypass_retry_cadence_via_tp_ttl_or_exposure() {
        let mut b = Kaprys::nowy(0, 0);
        let mut e = exit_retry_engine();
        koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        let id = e.baskets[0].id;
        e.cfg.pending_ttl_h = 1.0;
        e.cfg.pending_ttl_from_basket = false;
        e.cfg.pending_drop_on_target = true;
        e.cfg.pending_lifetime = PendingLifetime::UntilTp1;
        e.cfg.pending_drop_arm = false;
        e.cfg.pending_drop_require_zone_touch = false;
        e.cfg.expo_cap_pct = 0.01;
        for p in b.pendings_mut() {
            p.placed_ts = TS0 - 2 * 3_600_000;
        }
        b.tryb_cancel = 1;
        e.close_basket(&mut b, id, TS0, CloseReason::Tp);
        let pending_count = b.pendings().len();
        let attempts = b.anul;
        assert!(pending_count > 0 && b.positions().is_empty());
        for n in 1..=9 {
            let q = Quote {
                ts: TS0 + n * 100,
                bid: 4050.0,
                ask: 4050.3,
            };
            b.w.ustaw_cene(q.ts, q.bid, q.ask);
            e.on_tick(&mut b, &q);
            assert_eq!(
                b.anul, attempts,
                "TP/TTL/exposure cannot issue an extra cancellation before retry"
            );
            assert_eq!(b.pendings().len(), pending_count);
            assert_eq!(e.baskets[0].pendings.len(), pending_count);
            assert!(e.baskets[0].alive() && e.baskets[0].pending_exit.is_some());
        }
        e.retry_confirmed_exits(&mut b, TS0 + 1000);
        assert_eq!(b.anul, attempts + pending_count as u32);
    }

    #[test]
    fn confirmed_exit_restart_roundtrip_and_legacy_basket_serde() {
        let mut b = Kaprys::nowy(0, 1);
        let mut e = exit_retry_engine();
        koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        let id = e.baskets[0].id;
        e.close_basket(&mut b, id, TS0, CloseReason::Tp);
        let saved = serde_json::to_vec(&e.baskets).unwrap();
        let mut restored = Engine::new(e.cfg.clone(), 400.0);
        restored.baskets = serde_json::from_slice(&saved).unwrap();
        b.tryb_close = 0;
        let q = Quote {
            ts: TS0 + 3000,
            bid: 3975.0,
            ask: 3975.3,
        };
        b.w.ustaw_cene(q.ts, q.bid, q.ask);
        restored.on_tick(&mut b, &q);
        assert!(b.positions().is_empty());
        assert_eq!(restored.baskets[0].state, BasketState::Done);
        let mut old = serde_json::to_value(&restored.baskets[0]).unwrap();
        old.as_object_mut().unwrap().remove("pending_exit");
        let legacy: Basket = serde_json::from_value(old).unwrap();
        assert!(legacy.pending_exit.is_none());
    }

    #[test]
    fn confirmed_exit_global_owns_only_known_baskets_and_waits_for_frozen() {
        let mut b = Kaprys::nowy(0, 0);
        let mut e = exit_retry_engine();
        let own = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        let other = b
            .open_market(OrderReq {
                side: Side::Sell,
                volume: 0.01,
                sl: None,
                tp: None,
                basket: Some(999_999),
                level: 0,
                is_toucher: false,
                comment: "foreign".into(),
            })
            .unwrap();
        let manual = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.01,
                sl: None,
                tp: None,
                basket: None,
                level: 0,
                is_toucher: false,
                comment: "manual".into(),
            })
            .unwrap();
        e.queued_exits.insert(
            own,
            QueuedExit {
                target: 3980.0,
                deadline: TS0,
                reason: CloseReason::Harvest,
                market_at_decision: 3990.0,
            },
        );
        b.positions_mut()
            .iter_mut()
            .find(|p| p.ticket == own)
            .unwrap()
            .frozen = true;
        e.close_everything(&mut b, TS0, CloseReason::Manual);
        assert!(e.baskets[0].alive());
        assert!(e.baskets[0].pending_exit.is_some());
        assert!(
            !e.queued_exits.contains_key(&own),
            "old discretionary exit loses authority even for frozen tickets"
        );
        assert_eq!(b.positions().len(), 3);
        b.positions_mut()
            .iter_mut()
            .find(|p| p.ticket == own)
            .unwrap()
            .frozen = false;
        let q = Quote {
            ts: TS0 + 100,
            bid: 3998.0,
            ask: 3998.3,
        };
        e.close_or_queue(&mut b, own, CloseReason::Harvest, &q);
        assert!(
            b.find_position(own).is_some(),
            "other exits cannot bypass pending retry cadence"
        );
        e.retry_confirmed_exits(&mut b, TS0 + 1000);
        assert!(b.find_position(own).is_none());
        assert!(b.find_position(other).is_some());
        assert!(b.find_position(manual).is_some());
        assert_eq!(e.baskets[0].state, BasketState::Done);
    }

    #[test]
    fn confirmed_exit_off_preserves_legacy_rejected_bank_behaviour() {
        let mut b = Kaprys::nowy(0, 1);
        let mut e = exit_retry_engine();
        e.cfg.confirmed_exit_retry = false;
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        let id = e.baskets[0].id;
        e.baskets[0].tps = vec![4010.0, 4020.0, 4030.0];
        e.handle_tp_hit(&mut b, id, Some(3), TS0, "price");
        assert!(b.find_position(t).is_some());
        assert_eq!(e.baskets[0].state, BasketState::Done);
        assert!(e.baskets[0].pending_exit.is_none());
        let before = b.zamk;
        e.retry_confirmed_exits(&mut b, TS0 + 2000);
        assert_eq!(before, b.zamk);
    }

    #[test]
    fn odrzut_tempa_nie_robi_sierot() {
        let zbuduj = |c: &mut Settings| {
            bez_zamykaczy(c);
            c.entry_units = 2;
            c.fast_fill_reject_s = 300.0;
            c.fast_fill_layers = 2;
            c.fast_fill_soft_age_min = 0.0;
        };

        let mut b = Kaprys::nowy(0, 1);
        let mut e = silnik(zbuduj);
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_POD));
        let id = e.baskets[0].id;
        koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        koszyk_z_pozycja(&mut b, &mut e, 3989.0);
        e.baskets[0].levels[0].fill_ts = TS0 + 1_000;
        e.baskets[0].levels[1].fill_ts = TS0 + 5_000; // 4 s < próg 300 s
        e.reject_fast_filled_baskets(&mut b, TS0 + 10_000);
        assert_eq!(b.positions().len(), 2, "broker odmówił — pozycje żyją");
        assert!(
            e.baskets.iter().find(|x| x.id == id).unwrap().alive(),
            "Done przy odrzuconych zamknięciach robi SIEROTY — koszyk ma zostać żywy"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(zbuduj);
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_POD));
        let id = e.baskets[0].id;
        koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        koszyk_z_pozycja(&mut b, &mut e, 3989.0);
        e.baskets[0].levels[0].fill_ts = TS0 + 1_000;
        e.baskets[0].levels[1].fill_ts = TS0 + 5_000;
        e.reject_fast_filled_baskets(&mut b, TS0 + 10_000);
        assert!(b.positions().is_empty(), "zgodny broker zamyka wszystko");
        assert!(
            !e.baskets.iter().find(|x| x.id == id).unwrap().alive(),
            "po potwierdzonych zamknięciach koszyk przechodzi w Done jak dotąd"
        );
    }

    #[test]
    fn wyjscie_ze_strefy_nie_robi_sierot() {
        let zbuduj = |c: &mut Settings| {
            bez_zamykaczy(c);
            c.zone_exit_adverse_s = 1.0;
            c.zone_exit_adverse_close = true;
        };
        let pod_strefa = Quote {
            ts: TS0 + 5_000,
            bid: 3980.0,
            ask: 3980.3,
        };

        let mut b = Kaprys::nowy(0, 1);
        let mut e = silnik(zbuduj);
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.baskets[0].adverse_since = TS0;
        e.zone_exit_adverse_sweep(&mut b, &pod_strefa);
        assert!(
            b.find_position(t).is_some(),
            "broker odmówił — pozycja żyje"
        );
        assert!(e.baskets[0].alive(), "koszyk nie ma prawa przejść w Done");
        assert_eq!(
            e.baskets[0].adverse_since, 0,
            "znacznik zerowany = kadencja ponowienia"
        );

        let mut b = Atrapa::nowa();
        let mut e = silnik(zbuduj);
        let t = koszyk_z_pozycja(&mut b, &mut e, 3990.0);
        e.baskets[0].adverse_since = TS0;
        e.zone_exit_adverse_sweep(&mut b, &pod_strefa);
        assert!(b.find_position(t).is_none(), "zgodny broker zamyka pozycję");
        assert!(
            !e.baskets[0].alive(),
            "po potwierdzonym zamknięciu koszyk kończy się"
        );
    }


    fn otworz_lot<B: Broker>(b: &mut B, vol: f64, lvl: i32) -> Ticket {
        b.open_market(OrderReq {
            side: Side::Buy,
            volume: vol,
            sl: None,
            tp: None,
            basket: None,
            level: lvl,
            is_toucher: false,
            comment: String::new(),
        })
        .expect("atrapa nie odmawia otwarcia")
    }

    #[test]
    fn redukcja_ekspozycji_omija_zamrozone() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.expo_cap_ml_pct = 200.0;
        });
        let zwykla = otworz_lot(&mut b, 1.0, 0);
        let mrozona = otworz_lot(&mut b, 1.0, 1);
        if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == mrozona) {
            p.frozen = true;
        }
        e.redukuj_ekspozycje(&mut b, TS0 + 1_000);
        assert!(
            b.find_position(zwykla).is_none(),
            "niezamrożona pozycja jest ofiarą strażnika"
        );
        assert!(
            b.find_position(mrozona).is_some(),
            "zamrożona pozycja przeżywa — nawet gdy poziom marginesu dalej pod progiem"
        );
    }

    #[test]
    fn redukcja_ekspozycji_honoruje_konto_dzwignia() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.expo_cap_ml_pct = 200.0;
            c.konto_dzwignia = 1_000_000.0;
        });
        otworz_lot(&mut b, 1.0, 0);
        e.redukuj_ekspozycje(&mut b, TS0 + 1_000);
        assert_eq!(
            b.positions().len(),
            1,
            "przy dźwigni z `konto_dzwignia` margines jest pomijalny — strażnik nie \
             strzela; licząc surowym `acc.leverage` zamknąłby pozycję"
        );
    }

    #[test]
    fn redukcja_ekspozycji_spisuje_kazda_ofiare_do_kroniki() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.entry_units = 4;
            c.expo_cap_pct = 0.01;
            c.expo_cap_close = false;
            c.journal_enabled = true;
            c.journal_min_level = EventLevel::Debug;
        });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_POD));
        let przed = b.pendings().len();
        assert!(przed > 0, "fixture musi wystawić pendingi");

        e.redukuj_ekspozycje(&mut b, TS0 + 1_000);
        let events: Vec<_> = e
            .journal
            .peek()
            .iter()
            .filter(|x| x.kind == EventKind::PendingCancelled)
            .filter(|x| x.reason == Some(RejectCode::ExposureCap))
            .collect();
        assert_eq!(events.len(), przed - b.pendings().len());
        assert!(events.iter().all(|x| x.ticket.is_some()));
        assert!(events.iter().all(|x| {
            x.data.contains_key("exposure_before")
                && x.data.contains_key("exposure_after")
                && x.data.contains_key("exposure_limit")
        }));
    }


    #[test]
    fn limit_kasuje_tylko_nadmiar_i_odznacza_po_luzie() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.entry_units = 4;
            c.max_open_positions = 3;
            c.enforce_position_limit_on_fill = true;
            c.limit_kasuje_tylko_nadmiar = true;
        });
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_POD));
        assert_eq!(
            b.pendings().len(),
            4,
            "test wymaga czterech wiszących szczebli"
        );

        otworz_lot(&mut b, 0.10, 10);
        otworz_lot(&mut b, 0.10, 11);
        e.enforce_position_limit(&mut b, TS0 + 1_000);
        assert_eq!(
            b.pendings().len(),
            1,
            "zostaje dokładnie tyle, ile limit mieści"
        );
        let najplytszy = b.pendings()[0].price;
        assert!(
            (najplytszy - 3990.0).abs() < 1e-9,
            "zostać miał szczebel NAJBLIŻEJ ceny (3990), został {najplytszy:.2}"
        );
        assert_eq!(
            e.baskets[0].levels.iter().filter(|g| g.cancelled).count(),
            3,
            "ofiary nadmiaru dostają znacznik `cancelled`"
        );

        b.positions_mut().clear();
        e.enforce_position_limit(&mut b, TS0 + 2_000);
        assert_eq!(
            e.baskets[0].levels.iter().filter(|g| g.cancelled).count(),
            1,
            "odznaczone najwyżej tyle szczebli, ile limit znów mieści (3 - 1 wiszący = 2)"
        );
    }

    #[test]
    fn limit_bez_osi_kasuje_wszystko_jak_dotad() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            bez_zamykaczy(c);
            c.entry_units = 4;
            c.max_open_positions = 3;
            c.enforce_position_limit_on_fill = true;
        });
        assert!(
            !e.cfg.limit_kasuje_tylko_nadmiar,
            "oś ma być domyślnie wyłączona"
        );
        e.on_message(&mut b, &wiad(1, 1, None, WEJSCIE_POD));
        assert_eq!(b.pendings().len(), 4);

        otworz_lot(&mut b, 0.10, 10);
        otworz_lot(&mut b, 0.10, 11);
        e.enforce_position_limit(&mut b, TS0 + 1_000);
        assert_eq!(
            b.pendings().len(),
            4,
            "poniżej limitu stara ścieżka nie rusza niczego — mimo że 2+4 > 3"
        );

        otworz_lot(&mut b, 0.10, 12);
        e.enforce_position_limit(&mut b, TS0 + 2_000);
        assert!(
            b.pendings().is_empty(),
            "przy pełnym liczniku stara ścieżka kasuje WSZYSTKIE wiszące"
        );
    }


    #[test]
    fn krotkie_okno_daje_inna_odpowiedz_niz_milcz() {
        let zbuduj = |tryb: crate::settings::RegimeGdyRozerwany| {
            let mut e = silnik(|c| {
                c.regime_filter = RegimeFilter::TrendMa;
                c.regime_miara = crate::settings::RegimeMiara::Srednia;
                c.regime_ma_hours = 72.0;
                c.regime_okno2_h = 6.0;
                c.regime_zmiennosc_max = 10.0;
            });
            e.cfg.regime_gdy_rozerwany = tryb;
            for i in 0..66 {
                let px = if i % 2 == 0 { 4000.0 } else { 4100.0 };
                e.price_hist.push((TS0 + i as i64 * 3_600_000, px));
            }
            for i in 66..72 {
                e.price_hist.push((TS0 + i as i64 * 3_600_000, 4100.0));
            }
            e
        };

        let milcz = zbuduj(crate::settings::RegimeGdyRozerwany::Milcz);
        assert!(
            milcz.regime_ok(Side::Buy, 4050.0),
            "Milcz: oba okna uciszone bramką zakresu — sygnał przechodzi"
        );

        let krotkie = zbuduj(crate::settings::RegimeGdyRozerwany::KrotkieOkno);
        assert!(
            !krotkie.regime_ok(Side::Buy, 4050.0),
            "KrotkieOkno: okno awaryjne 6 h MÓWI (próg 4100), kupno po 4050 \
             idzie pod prąd — dotąd tryb był w 100 % tożsamy z Milcz"
        );
    }

    #[test]
    fn rozgrzewka_milczy_zamiast_liczyc_krotki_zakres() {
        let mut e = silnik(|c| {
            c.regime_filter = RegimeFilter::TrendMa;
            c.regime_ma_hours = 72.0;
            c.regime_zmiennosc_min = 5.0;
        });
        for i in 0..10 {
            let px = if i % 2 == 0 { 4000.0 } else { 4008.0 };
            e.price_hist.push((TS0 + i as i64 * 3_600_000, px));
        }
        assert!(
            e.regime_prog(6, false).is_none(),
            "bez kompletu okna głównego nie ma miary rozerwania — okno ma milczeć, \
             a nie mierzyć widełki na zakresie 6 h (inna wielkość niż 72 h)"
        );
    }
}

#[cfg(test)]
mod testy_trail_sr {
    use super::testy_pakiet_a::{silnik, wiad, Atrapa, TS0};
    use super::*;

    fn karm_swiece(e: &mut Engine, b: &mut Atrapa, t0: Ts, swiece: &[(f64, f64)]) -> Ts {
        for (i, &(hi, lo)) in swiece.iter().enumerate() {
            let baza = t0 + i as i64 * 60_000;
            for (dt, px) in [(1_000, hi), (30_000, lo)] {
                b.ustaw_cene(baza + dt, px, px);
                let q = b.quote();
                e.on_tick(b, &q);
            }
        }
        t0 + swiece.len() as i64 * 60_000 + 1_000
    }

    fn tick(e: &mut Engine, b: &mut Atrapa, ts: Ts, px: f64) {
        b.ustaw_cene(ts, px, px);
        let q = b.quote();
        e.on_tick(b, &q);
    }

    fn pozycja(
        b: &mut Atrapa,
        side: Side,
        open: Px,
        sl: Option<Px>,
        tp: Option<Px>,
        runner: bool,
    ) -> Ticket {
        let t = b
            .open_market(OrderReq {
                side,
                volume: 0.10,
                sl,
                tp,
                basket: None,
                level: 0,
                is_toucher: false,
                comment: String::new(),
            })
            .expect("atrapa nie odmawia");
        if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
            p.open_price = open;
            p.is_runner = runner;
        }
        t
    }

    fn sl(b: &Atrapa, t: Ticket) -> Option<Px> {
        b.find_position(t).and_then(|p| p.sl)
    }

    const SWIECE_3995: [(f64, f64); 7] = [
        (4002.0, 4000.0),
        (4002.0, 3999.0),
        (4002.0, 3998.0),
        (4002.0, 3995.0),
        (4002.0, 3998.0),
        (4002.0, 3999.0),
        (4002.0, 4000.0),
    ];

    #[test]
    fn geometria_swingow_i_anty_lookahead() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.trail_sr_enabled = true;
            c.trail_sr_min_dist_price = 0.0;
        });
        let po = karm_swiece(&mut e, &mut b, TS0, &SWIECE_3995);
        tick(&mut e, &mut b, po, 4000.0); // zamyka 7. świecę → potwierdzenie
        assert_eq!(e.sr.swingi_low.len(), 1, "dokładnie jeden swing low");
        assert_eq!(
            e.sr.swingi_low[0].0, 3995.0,
            "poziom = minimum świecy środkowej"
        );
        assert!(
            e.sr.swingi_high.is_empty(),
            "płaskie szczyty (4002 wszędzie) mają być odrzucone — wymóg OSTRego ekstremum"
        );
        let t_potw = e.sr.swingi_low[0].1;
        assert_eq!(
            t_potw, po,
            "potwierdzenie = zamknięcie 3. świecy PO szczytowej"
        );
        assert_eq!(
            e.sr_kandydat(Side::Buy, 4000.0, None, t_potw - 1),
            None,
            "zapytanie sprzed potwierdzenia nie ma prawa widzieć swinga"
        );
        assert_eq!(e.sr_kandydat(Side::Buy, 4000.0, None, t_potw), Some(3995.0));
    }

    #[test]
    fn plaskie_dno_odrzucone() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| c.trail_sr_enabled = true);
        let swiece = [
            (4002.0, 4000.0),
            (4002.0, 3999.0),
            (4002.0, 3995.0),
            (4002.0, 3995.0),
            (4002.0, 3998.0),
            (4002.0, 3999.0),
            (4002.0, 4000.0),
            (4002.0, 4001.0),
            (4002.0, 4001.5),
        ];
        let po = karm_swiece(&mut e, &mut b, TS0, &swiece);
        tick(&mut e, &mut b, po, 4001.0);
        assert!(
            e.sr.swingi_low.is_empty(),
            "równe dna 3995/3995 nie są ostrym ekstremum w żadnym z okien: {:?}",
            e.sr.swingi_low
        );
    }

    #[test]
    fn zapadka_sl_nigdy_sie_nie_cofa() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.trail_sr_enabled = true;
            c.trail_sr_activation = TrailSrActivation::Entry;
            c.trail_sr_min_dist_price = 0.0;
        });
        let t = pozycja(&mut b, Side::Buy, 3990.0, None, None, true);
        let po = karm_swiece(&mut e, &mut b, TS0, &SWIECE_3995);
        tick(&mut e, &mut b, po, 4000.0);
        assert_eq!(
            sl(&b, t),
            Some(3994.5),
            "pozycja bez SL dostaje pierwszy poziom − offset"
        );

        let nizsze = [
            (3993.0, 3992.0),
            (3991.0, 3990.0),
            (3989.0, 3988.0),
            (3986.0, 3985.0),
            (3989.0, 3988.0),
            (3991.0, 3990.0),
            (3993.0, 3992.0),
        ];
        let po2 = karm_swiece(&mut e, &mut b, po + 60_000, &nizsze);
        tick(&mut e, &mut b, po2, 3992.0);
        assert!(
            e.sr.swingi_low.iter().any(|&(l, _)| l == 3985.0),
            "niższy swing ma być wykryty (inaczej test nie mierzy zapadki)"
        );
        assert_eq!(sl(&b, t), Some(3994.5), "SL ani razu nie spada");
    }

    #[test]
    fn kontrakt_zera_wylaczona_os_nie_dotyka_niczego() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|_| {});
        assert!(!e.cfg.trail_sr_enabled, "oś ma być domyślnie wyłączona");
        let t = pozycja(&mut b, Side::Buy, 3990.0, Some(3980.0), None, true);
        let po = karm_swiece(&mut e, &mut b, TS0, &SWIECE_3995);
        tick(&mut e, &mut b, po, 4000.0);
        assert_eq!(
            e.sr.kubelek,
            i64::MIN,
            "agregator nietknięty (parytet strukturalny)"
        );
        assert!(e.sr.zamkniete.is_empty() && e.sr.swingi_low.is_empty());
        assert_eq!(sl(&b, t), Some(3980.0), "SL bajt w bajt jak przed osią");
    }

    #[test]
    fn przypadek_wlasciciela_4644() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.trail_sr_enabled = true;
            c.trail_sr_activation = TrailSrActivation::Entry;
            c.trail_sr_min_dist_price = 0.0;
        });
        let t = pozycja(&mut b, Side::Buy, 4640.0, None, Some(4652.0), true);
        let swiece = [
            (4647.0, 4646.0),
            (4646.5, 4645.5),
            (4646.0, 4645.0),
            (4645.5, 4644.5),
            (4646.0, 4645.0),
            (4646.5, 4645.5),
            (4647.0, 4646.0),
        ];
        let po = karm_swiece(&mut e, &mut b, TS0, &swiece);
        tick(&mut e, &mut b, po, 4647.0);
        assert_eq!(
            sl(&b, t),
            Some(4644.0),
            "SL = swing low 4644.5 − offset 0.5 = 4644 — idealne miejsce właściciela"
        );
        assert_eq!(
            b.find_position(t).and_then(|p| p.tp),
            Some(4652.0),
            "TP 4652 nietknięty"
        );
    }

    #[test]
    fn filtry_oddechu_i_wybor_najblizszego() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.trail_sr_enabled = true;
            c.trail_sr_min_dist_price = 0.0;
        });
        let po = karm_swiece(&mut e, &mut b, TS0, &SWIECE_3995);
        let nizsze = [
            (3993.0, 3992.0),
            (3991.0, 3990.0),
            (3989.0, 3988.0),
            (3986.0, 3985.0),
            (3989.0, 3988.0),
            (3991.0, 3990.0),
            (3993.0, 3992.0),
        ];
        let po2 = karm_swiece(&mut e, &mut b, po + 60_000, &nizsze);
        tick(&mut e, &mut b, po2, 4000.0);
        assert_eq!(
            e.sr.swingi_low.len(),
            2,
            "korpus testu: dwa swingi 3995 i 3985"
        );
        let ts = po2 + 1;
        assert_eq!(e.sr_kandydat(Side::Buy, 4000.0, None, ts), Some(3995.0));
        e.cfg.trail_sr_min_dist_price = 6.0;
        assert_eq!(e.sr_kandydat(Side::Buy, 4000.0, None, ts), Some(3985.0));
        e.cfg.trail_sr_min_dist_price = 0.0;
        assert_eq!(
            e.sr_kandydat(Side::Buy, 4000.0, Some(3994.0), ts),
            Some(3985.0)
        );
        assert_eq!(e.sr_kandydat(Side::Buy, 3980.0, None, ts), None);
    }

    #[test]
    fn stops_level_odmowa_pomija_swiece_bez_clampowania() {
        let mut b = Atrapa::nowa();
        b.stops = 10.0;
        let mut e = silnik(|c| {
            c.trail_sr_enabled = true;
            c.trail_sr_activation = TrailSrActivation::Entry;
            c.trail_sr_min_dist_price = 0.0;
        });
        let t = pozycja(&mut b, Side::Buy, 3985.0, Some(3980.0), None, true);
        let po = karm_swiece(&mut e, &mut b, TS0, &SWIECE_3995);
        tick(&mut e, &mut b, po, 4000.0);
        assert_eq!(
            sl(&b, t),
            Some(3980.0),
            "odmowa brokera = pominięcie świecy; SL przycięty do widełek to INNY mechanizm"
        );
        tick(&mut e, &mut b, po + 60_000, 4010.0);
        assert_eq!(
            sl(&b, t),
            Some(3994.5),
            "ponowna próba przechodzi na PEŁNY poziom"
        );
    }

    #[test]
    fn scope_runner_nie_dotyka_warstw() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.trail_sr_enabled = true;
            c.trail_sr_activation = TrailSrActivation::Entry;
            c.trail_sr_min_dist_price = 0.0;
        });
        let warstwa = pozycja(&mut b, Side::Buy, 3990.0, Some(3985.0), Some(4010.0), false);
        let runner = pozycja(&mut b, Side::Buy, 3990.0, Some(3985.0), None, true);
        let po = karm_swiece(&mut e, &mut b, TS0, &SWIECE_3995);
        tick(&mut e, &mut b, po, 4000.0);
        assert_eq!(
            sl(&b, warstwa),
            Some(3985.0),
            "warstwa TP1 nietknięta (scope=Runner)"
        );
        assert_eq!(
            sl(&b, runner),
            Some(3994.5),
            "runner dostaje poziom struktury"
        );
    }

    #[test]
    fn aktywacja_tp2_czeka_na_drugi_cel() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.trail_sr_enabled = true;
            c.trail_sr_min_dist_price = 0.0;
        });
        assert_eq!(
            e.cfg.trail_sr_activation,
            TrailSrActivation::Tp2,
            "default ze spec"
        );
        e.on_message(
            &mut b,
            &wiad(
                1,
                1,
                None,
                "BUY LIMITS GOLD @ 3990/3985\nTP 4010\nTP 4020\nSL 3980",
            ),
        );
        let id = e.baskets[0].id;
        let t = pozycja(&mut b, Side::Buy, 3990.0, None, None, true);
        if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
            p.basket = Some(id);
        }
        e.baskets[0].tickets.push(t);
        e.baskets[0].had_positions = true;

        let po = karm_swiece(&mut e, &mut b, TS0, &SWIECE_3995);
        tick(&mut e, &mut b, po, 4000.0);
        assert_eq!(sl(&b, t), None, "etap 0 — przed TP2 zero modyfikacji S/R");

        e.baskets[0].tp_stage = 1;
        tick(&mut e, &mut b, po + 60_000, 4000.0);
        assert_eq!(sl(&b, t), None, "etap 1 — nadal przed progiem Tp2");

        e.baskets[0].tp_stage = 2;
        tick(&mut e, &mut b, po + 120_000, 4000.0);
        assert_eq!(
            sl(&b, t),
            Some(3994.5),
            "po dotknięciu TP2 pierwsza świeca podbija"
        );
    }

    #[test]
    fn arbitraz_z_be_lepszy_dla_zysku_wygrywa() {
        let zaloz = |wlaczona: bool| {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                c.trail_sr_enabled = wlaczona;
                c.trail_sr_activation = TrailSrActivation::Entry;
                c.trail_sr_min_dist_price = 0.0;
                c.be_at_tp1 = true;
                c.be_offset = 0.0;
            });
            e.on_message(
                &mut b,
                &wiad(
                    1,
                    1,
                    None,
                    "BUY LIMITS GOLD @ 3990/3985\nTP 4010\nTP 4020\nSL 3980",
                ),
            );
            let id = e.baskets[0].id;
            let t = pozycja(&mut b, Side::Buy, 3990.0, None, None, true);
            if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
                p.basket = Some(id);
            }
            e.baskets[0].tickets.push(t);
            e.baskets[0].had_positions = true;
            (b, e, id, t)
        };

        let (mut b, mut e, id, t) = zaloz(true);
        let po = karm_swiece(&mut e, &mut b, TS0, &SWIECE_3995);
        tick(&mut e, &mut b, po, 4000.0);
        assert_eq!(sl(&b, t), Some(3994.5));
        e.move_basket_to_be(&mut b, id, po + 1);
        assert_eq!(
            sl(&b, t),
            Some(3994.5),
            "BE nie cofa stopu podbitego przez S/R"
        );
        let (mut b, mut e, id, t) = zaloz(true);
        tick(&mut e, &mut b, TS0 + 500, 4000.0);
        e.move_basket_to_be(&mut b, id, TS0 + 600);
        assert_eq!(sl(&b, t), Some(3990.0), "BE stawia podłogę");
        let po = karm_swiece(&mut e, &mut b, TS0 + 60_000, &SWIECE_3995);
        tick(&mut e, &mut b, po, 4000.0);
        assert_eq!(
            sl(&b, t),
            Some(3994.5),
            "S/R nad BE wygrywa — niezależnie od kolejności"
        );

        let nizszy = [
            (4002.0, 3992.0),
            (4002.0, 3991.0),
            (4002.0, 3990.0),
            (4002.0, 3987.0),
            (4002.0, 3990.0),
            (4002.0, 3991.0),
            (4002.0, 3992.0),
        ];
        let (mut b, mut e, id, t) = zaloz(true);
        tick(&mut e, &mut b, TS0 + 500, 4000.0);
        e.move_basket_to_be(&mut b, id, TS0 + 600);
        let po = karm_swiece(&mut e, &mut b, TS0 + 60_000, &nizszy);
        tick(&mut e, &mut b, po, 4000.0);
        assert_eq!(sl(&b, t), Some(3990.0), "S/R pod BE przegrywa z zapadką");
        let (mut b, mut e, id, t) = zaloz(true);
        let po = karm_swiece(&mut e, &mut b, TS0, &nizszy);
        tick(&mut e, &mut b, po, 4000.0);
        assert_eq!(
            sl(&b, t),
            Some(3986.5),
            "S/R stawia, bo BE jeszcze nie było"
        );
        e.move_basket_to_be(&mut b, id, po + 1);
        assert_eq!(
            sl(&b, t),
            Some(3990.0),
            "BE nadpisuje w górę — lepszy wygrywa"
        );

        let (mut b, mut e, id, t) = zaloz(false);
        tick(&mut e, &mut b, TS0 + 500, 4000.0);
        if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
            p.sl = Some(3994.5); // jakby postawił go ktokolwiek inny
        }
        e.move_basket_to_be(&mut b, id, TS0 + 600);
        assert_eq!(
            sl(&b, t),
            Some(3990.0),
            "stara ścieżka CO DO BITU: bez osi BE nadpisuje bezwarunkowo"
        );
    }

    #[test]
    fn dynamiczne_zera_sa_bit_identical_z_legacy() {
        let mut b1 = Atrapa::nowa();
        let mut b2 = Atrapa::nowa();
        let ustaw = |c: &mut Settings| {
            c.trail_sr_enabled = true;
            c.trail_sr_activation = TrailSrActivation::Entry;
            c.trail_sr_min_dist_price = 0.0;
        };
        let mut legacy = silnik(ustaw);
        let mut zero = silnik(ustaw);
        zero.cfg.trail_sr_atr_period = 99;
        let t1 = pozycja(&mut b1, Side::Buy, 3990.0, None, None, true);
        let t2 = pozycja(&mut b2, Side::Buy, 3990.0, None, None, true);
        let po1 = karm_swiece(&mut legacy, &mut b1, TS0, &SWIECE_3995);
        let po2 = karm_swiece(&mut zero, &mut b2, TS0, &SWIECE_3995);
        tick(&mut legacy, &mut b1, po1, 4000.0);
        tick(&mut zero, &mut b2, po2, 4000.0);
        assert_eq!(legacy.stan_sr(), zero.stan_sr());
        assert_eq!(sl(&b1, t1), sl(&b2, t2));
        assert!(legacy.sr.atr_true_ranges.is_empty());
        assert!(zero.sr.atr_true_ranges.is_empty());
    }

    #[test]
    fn prominence_i_atr_sa_causalne_i_zamrozone_przy_potwierdzeniu() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            c.trail_sr_enabled = true;
            c.trail_sr_fractal_n = 1;
            c.trail_sr_atr_period = 3;
            c.trail_sr_min_prominence_atr = 0.1;
            c.trail_sr_min_dist_price = 0.0;
        });
        let swiece = [(10.0, 9.0), (10.0, 5.0), (11.0, 9.0)];
        let po = karm_swiece(&mut e, &mut b, TS0, &swiece);
        assert!(
            e.sr.prominence_low.is_empty(),
            "prawa świeca nie jest jeszcze domknięta"
        );
        tick(&mut e, &mut b, po - 1_001, 11.0);
        assert!(
            e.sr.prominence_low.is_empty(),
            "tick przed granicą nie potwierdza swinga"
        );
        tick(&mut e, &mut b, po, 12.0);
        let (_, t, prom) = e.sr.prominence_low[0];
        assert!((prom - 1.25).abs() < 1e-12, "prominence/ATR={prom}");
        assert_eq!(e.sr_kandydat(Side::Buy, 12.0, None, t), Some(5.0));
        e.cfg.trail_sr_min_prominence_atr = 1.26;
        assert_eq!(e.sr_kandydat(Side::Buy, 12.0, None, t), None);
        let zapisany = e.sr.prominence_low[0].2;
        tick(&mut e, &mut b, po + 60_000, 100.0);
        assert_eq!(e.sr.prominence_low[0].2, zapisany);
    }

    #[test]
    fn effective_offset_bierze_max_fixed_atr_i_spread_p90() {
        let mut e = silnik(|c| {
            c.trail_sr_enabled = true;
            c.trail_sr_atr_period = 3;
            c.trail_sr_offset = 0.5;
            c.trail_sr_offset_atr_mult = 0.5;
            c.trail_sr_offset_spread_mult = 3.0;
        });
        let bars = [
            SrWarmupBar {
                ts: TS0,
                high: 11.0,
                low: 9.0,
                close: 10.0,
                spread: 0.20,
            },
            SrWarmupBar {
                ts: TS0 + 60_000,
                high: 11.0,
                low: 9.0,
                close: 10.0,
                spread: 0.40,
            },
            SrWarmupBar {
                ts: TS0 + 120_000,
                high: 11.0,
                low: 9.0,
                close: 10.0,
                spread: 0.30,
            },
            SrWarmupBar {
                ts: TS0 + 180_000,
                high: 10.0,
                low: 10.0,
                close: 10.0,
                spread: 9.0,
            },
        ];
        assert!(e.rozgrzej_sr_z_m1(&bars));
        assert_eq!(e.sr.atr, Some(2.0));
        assert_eq!(e.sr.spread_ref, Some(0.4));
        assert!((e.sr_effective_offset() - 1.2).abs() < 1e-12);
    }

    #[test]
    fn stan_dynamiczny_przezywa_serializacje_i_restart_bez_rozjazdu() {
        let pusty_stary: StanSr = serde_json::from_str("{}").unwrap();
        assert_eq!(
            pusty_stary,
            StanSr::default(),
            "snapshot sprzed nowych pól ma dostać neutralne defaulty"
        );
        let cfg = |c: &mut Settings| {
            c.trail_sr_enabled = true;
            c.trail_sr_fractal_n = 1;
            c.trail_sr_atr_period = 3;
            c.trail_sr_min_prominence_atr = 0.2;
            c.trail_sr_offset_atr_mult = 0.3;
            c.trail_sr_offset_spread_mult = 2.0;
            c.trail_sr_min_dist_price = 0.0;
        };
        let mut b1 = Atrapa::nowa();
        let mut ciagly = silnik(cfg);
        let pierwsze = [(10.0, 9.0), (10.0, 5.0), (11.0, 9.0), (12.0, 10.0)];
        let po = karm_swiece(&mut ciagly, &mut b1, TS0, &pierwsze);
        let json = serde_json::to_string(&ciagly.stan_sr()).unwrap();
        let stan: StanSr = serde_json::from_str(&json).unwrap();
        let mut wznowiony = silnik(cfg);
        wznowiony.set_stan_sr(stan);
        let mut b2 = Atrapa::nowa();
        for (dt, px) in [(0, 12.0), (60_000, 11.0), (120_000, 13.0)] {
            tick(&mut ciagly, &mut b1, po + dt, px);
            tick(&mut wznowiony, &mut b2, po + dt, px);
        }
        assert_eq!(ciagly.stan_sr(), wznowiony.stan_sr());
        assert_eq!(
            ciagly.sr_effective_offset(),
            wznowiony.sr_effective_offset()
        );
        assert_eq!(
            ciagly.sr_kandydat(Side::Buy, 20.0, None, po + 120_000),
            wznowiony.sr_kandydat(Side::Buy, 20.0, None, po + 120_000)
        );
    }

    #[test]
    fn warmup_m1_jest_identyczny_z_nieprzerwanym_strumieniem() {
        let cfg = |c: &mut Settings| {
            c.trail_sr_enabled = true;
            c.trail_sr_tf_min = 2;
            c.trail_sr_fractal_n = 1;
            c.trail_sr_atr_period = 3;
            c.trail_sr_min_prominence_atr = 0.1;
            c.trail_sr_offset_atr_mult = 0.25;
            c.trail_sr_offset_spread_mult = 2.0;
            c.trail_sr_min_dist_price = 0.0;
        };
        let bars: Vec<SrWarmupBar> = (0..12)
            .map(|i| {
                let base = 100.0 + (i % 5) as f64;
                SrWarmupBar {
                    ts: TS0 + i * 60_000,
                    high: base + 1.0,
                    low: base - 1.0,
                    close: base,
                    spread: 0.25 + (i % 3) as f64 * 0.25,
                }
            })
            .collect();
        let mut ciagly = silnik(cfg);
        let mut broker = Atrapa::nowa();
        for bar in &bars {
            for (dt, mid) in [(0, bar.high), (30_000, bar.low), (59_000, bar.close)] {
                broker.ustaw_cene(bar.ts + dt, mid - bar.spread / 2.0, mid + bar.spread / 2.0);
                let q = broker.quote();
                ciagly.on_tick(&mut broker, &q);
            }
        }
        let mut po_restarcie = silnik(cfg);
        assert!(po_restarcie.rozgrzej_sr_z_m1(&bars));
        assert_eq!(ciagly.stan_sr(), po_restarcie.stan_sr());
        assert_eq!(
            ciagly.sr_effective_offset(),
            po_restarcie.sr_effective_offset()
        );
    }
}

#[cfg(test)]
mod testy_odzysk_storm {
    use super::testy_pakiet_a::{silnik, wiad, zrodlo, Atrapa, TS0};
    use super::*;

    const KUPNO: &str = "BUY LIMITS GOLD @ 3990/3985\nTP 4010\nTP 4020\nSL 3980";
    const SPRZEDAZ: &str = "SELL LIMITS GOLD @ 4010/4005\nTP 3990\nTP 3980\nSL 4020";

    fn siatka(c: &mut Settings) {
        c.lot_mode_percent = false;
        c.lot_fixed = 0.01;
        c.lot_min = 0.01;
        c.auto_limit = false;
        c.risk_per_basket_pct = 0.0;
        c.max_open_positions = 0;
        c.entry_units = 5;
        c.entry_units_limit = 5;
        c.grid_anchor_absolute = false;
        c.ppm_enabled = false;
    }

    fn wiad_ts(chat: i64, msg_id: i64, reply: Option<i64>, ts: Ts, text: &str) -> IncomingMessage {
        IncomingMessage {
            ts,
            source: zrodlo(chat),
            source_name: format!("TEST{chat}"),
            msg_id,
            reply_to: reply,
            edit_of: None,
            text: text.into(),
        }
    }


    #[test]
    fn w30_kontrakt_zera_wymaga_obu_pol() {
        let odniesienie = {
            let mut b = Atrapa::nowa();
            let mut e = silnik(siatka);
            assert_eq!(e.cfg.entry_allowance_usd, 0.0, "kontrakt zera: kwota 0");
            assert_eq!(e.cfg.entry_allowance_units, 0, "kontrakt zera: jednostki 0");
            e.on_message(&mut b, &wiad(1, 1, None, KUPNO));
            (e.baskets[0].levels.len(), b.pendings().len())
        };
        assert!(odniesienie.1 > 0, "test wymaga rozstawionej siatki");

        for (usd, units) in [(1.0, 0u32), (0.0, 2u32)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                siatka(c);
                c.entry_allowance_usd = usd;
                c.entry_allowance_units = units;
            });
            e.on_message(&mut b, &wiad(1, 1, None, KUPNO));
            assert_eq!(
                (e.baskets[0].levels.len(), b.pendings().len()),
                odniesienie,
                "usd={usd}, units={units}: jedno pole bez drugiego nie ma prawa \
                 postawić warstwy"
            );
        }
    }

    #[test]
    fn w30_geometria_allowance_nad_strefa_i_pod_strefa() {
        for (tekst, strona) in [(KUPNO, Side::Buy), (SPRZEDAZ, Side::Sell)] {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                siatka(c);
                c.entry_allowance_usd = 1.0;
                c.entry_allowance_units = 2;
            });
            e.on_message(&mut b, &wiad(1, 1, None, tekst));
            let bk = &e.baskets[0];
            assert_eq!(bk.side, strona, "test wymaga sygnału {strona:?}");
            let w = bk
                .levels
                .iter()
                .find(|g| g.level == 2000)
                .unwrap_or_else(|| panic!("{strona:?}: brak warstwy allowance"));

            let krawedz = strona.worse_edge(bk.zone_lo, bk.zone_hi);
            assert!(
                (w.price - (krawedz + strona.sign())).abs() < 1e-9,
                "{strona:?}: warstwa ma leżeć 1 $ za krawędzią {krawedz:.2}, leży {:.2}",
                w.price
            );
            assert!(
                strona.better(krawedz, w.price),
                "{strona:?}: warstwa allowance musi być GORSZYM wejściem niż krawędź"
            );
            assert!(
                w.price > bk.zone_hi + 1e-9 || w.price < bk.zone_lo - 1e-9,
                "{strona:?}: warstwa allowance musi leżeć POZA strefą [{:.2};{:.2}], \
                 leży {:.2}",
                bk.zone_lo,
                bk.zone_hi,
                w.price
            );
            assert_eq!(w.base_units, 2, "{strona:?}: jednostki wprost z pola");
            assert!(!w.is_toucher, "{strona:?}: to nie jest toucher");
        }
    }

    #[test]
    fn w30_allowance_doklada_zlecenia_ponad_siatke() {
        let mut b0 = Atrapa::nowa();
        let mut e0 = silnik(siatka);
        e0.on_message(&mut b0, &wiad(1, 1, None, KUPNO));
        let bez = b0.pendings().len();

        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            siatka(c);
            c.entry_allowance_usd = 1.0;
            c.entry_allowance_units = 2;
        });
        e.on_message(&mut b, &wiad(1, 1, None, KUPNO));
        assert_eq!(
            b.pendings().len(),
            bez + 2,
            "warstwa allowance ma DOŁOŻYĆ 2 zlecenia ponad siatkę w strefie \
             (nie dzieli się budżetem `entry_units`)"
        );
    }


    fn scenariusz_b56(os: bool) -> (Atrapa, Engine, u32, Ticket) {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            siatka(c);
            c.be_offset = 0.3; // dokładnie jak w presecie STORM-1
            c.be_covers_late_fills = os;
        });
        e.on_message(&mut b, &wiad(1, 1, None, KUPNO));
        let id = e.baskets[0].id;

        e.on_message(
            &mut b,
            &wiad_ts(1, 2, Some(1), TS0 + 60_000, "SL IS SET TO BE"),
        );
        assert!(
            e.baskets[0].be_ts > 0,
            "komenda BE ma zostawić znacznik chwili"
        );

        let o = b.pendings()[0].clone();
        let _ = b.cancel_pending(o.ticket);
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: o.volume,
                sl: o.sl,
                tp: o.tp,
                basket: Some(id),
                level: o.level,
                is_toucher: false,
                comment: format!("B{id}"),
            })
            .expect("atrapa otwiera pozycję");
        let ts_fill = TS0 + 240_000;
        if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
            p.open_ts = ts_fill;
            p.open_price = o.price;
        }
        let q = Quote {
            ts: ts_fill,
            bid: 3998.0,
            ask: 3998.3,
        };
        b.ustaw_cene(ts_fill, q.bid, q.ask);
        e.on_tick(&mut b, &q);
        (b, e, id, t)
    }

    #[test]
    fn w31a_kontrakt_zera_pozny_fill_nosi_stary_sl() {
        let (b, e, _, t) = scenariusz_b56(false);
        assert!(!e.cfg.be_covers_late_fills, "oś ma być domyślnie wyłączona");
        let p = b.find_position(t).expect("pozycja żyje");
        assert_eq!(
            p.sl,
            Some(3980.0),
            "bez osi późny fill musi nosić stop Z SYGNAŁU — stan sprzed naprawy"
        );
    }

    #[test]
    fn w31a_be_kryje_fill_po_komendzie() {
        let (b, _, _, t) = scenariusz_b56(true);
        let p = b.find_position(t).expect("pozycja żyje");
        let oczekiwany = p.open_price + 0.3;
        assert_eq!(
            p.sl.map(|v| (v * 100.0).round() / 100.0),
            Some((oczekiwany * 100.0).round() / 100.0),
            "późny fill ma dostać BE liczone od WŁASNEJ ceny wejścia {:.2}",
            p.open_price
        );
    }

    #[test]
    fn w31a_wczesna_pozycja_nie_jest_ruszana_ponownie() {
        let mut b = Atrapa::nowa();
        let mut e = silnik(|c| {
            siatka(c);
            c.be_offset = 0.3;
            c.be_covers_late_fills = true;
        });
        e.on_message(&mut b, &wiad(1, 1, None, KUPNO));
        let id = e.baskets[0].id;
        let t = b
            .open_market(OrderReq {
                side: Side::Buy,
                volume: 0.01,
                sl: Some(3980.0),
                tp: None,
                basket: Some(id),
                level: 0,
                is_toucher: false,
                comment: format!("B{id}"),
            })
            .expect("atrapa otwiera pozycję");
        e.baskets[0].tickets.push(t);
        e.on_message(
            &mut b,
            &wiad_ts(1, 2, Some(1), TS0 + 60_000, "SL IS SET TO BE"),
        );
        let po_komendzie = b.find_position(t).and_then(|p| p.sl);
        if let Some(p) = b.positions_mut().iter_mut().find(|p| p.ticket == t) {
            p.sl = Some(3999.0);
        }
        let q = Quote {
            ts: TS0 + 240_000,
            bid: 3998.0,
            ask: 3998.3,
        };
        b.ustaw_cene(q.ts, q.bid, q.ask);
        e.on_tick(&mut b, &q);
        assert_eq!(
            b.find_position(t).and_then(|p| p.sl),
            Some(3999.0),
            "pozycja sprzed komendy (BE dało jej {po_komendzie:?}) nie ma prawa \
             zostać cofnięta przez krycie późnych filli"
        );
    }


    #[test]
    fn w31b_parser_kontrakt_zera_i_wariant_warunkowy() {
        let rozkaz = "Take partials.";
        let warunkowy = "YOU CAN CLOSE 3 LAYERS HERE OR YOU CAN HOLD FOR 200 PIPS";

        for t in [rozkaz, warunkowy] {
            assert!(
                !parser::parse(t)
                    .iter()
                    .any(|s| matches!(s, Signal::TakePartials)),
                "kontrakt zera: bez osi ta wiadomość nie ma prawa dać polecenia: {t}"
            );
        }

        let opcje = |v: bool| parser::OpcjeParsera {
            partials_jako_komenda: v,
            ..Default::default()
        };
        assert!(
            parser::parse_z_opcjami(rozkaz, opcje(true))
                .iter()
                .any(|s| matches!(s, Signal::TakePartials)),
            "z osią rozkaz ma być poleceniem (132 takie wiadomości STORM)"
        );
        assert!(
            !parser::parse_z_opcjami(warunkowy, opcje(true))
                .iter()
                .any(|s| matches!(s, Signal::TakePartials)),
            "wariant z wyborem (you can … or hold) zostaje informacją"
        );
    }

    #[test]
    fn w31b_inkaso_na_komende_i_kontrakt_zera() {
        let zaloz = |wykonuj: bool, pct: f64| {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                siatka(c);
                c.partials_wykonuj = wykonuj;
                c.partials_pct = pct;
                c.partial_close = false; // 0,01 lota jest niepodzielny
                c.bank_close_last = false; // runner zostaje
            });
            e.on_message(&mut b, &wiad(1, 1, None, KUPNO));
            let id = e.baskets[0].id;
            for _ in 0..5 {
                let t = b
                    .open_market(OrderReq {
                        side: Side::Buy,
                        volume: 0.01,
                        sl: Some(3980.0),
                        tp: None,
                        basket: Some(id),
                        level: 0,
                        is_toucher: false,
                        comment: format!("B{id}"),
                    })
                    .unwrap();
                e.baskets[0].tickets.push(t);
            }
            e.on_message(
                &mut b,
                &wiad_ts(1, 2, Some(1), TS0 + 60_000, "Take partials."),
            );
            (b, e)
        };

        let (b, e) = zaloz(false, 40.0);
        assert_eq!(
            b.positions().len(),
            5,
            "bez osi komenda partials nic nie zamyka"
        );
        assert_eq!(e.baskets[0].tp_stage, 0, "bez osi etap nie drga");

        let (b, _) = zaloz(true, 0.0);
        assert_eq!(
            b.positions().len(),
            5,
            "transza 0 % nie ma prawa nic zamknąć"
        );

        let (b, e) = zaloz(true, 40.0);
        assert_eq!(
            b.positions().len(),
            3,
            "40 % z 5 pozycji to 2 warstwy do inkasa, reszta biegnie dalej"
        );
        assert_eq!(
            e.baskets[0].tp_stage, 0,
            "transza partials NIE jest szczeblem drabinki — etap koszyka \
             musi zostać nietknięty (patrz `Basket::tp_stage`)"
        );
    }


    #[test]
    fn w33_close_all_adresowanie() {
        let zaloz = |scope: crate::settings::CloseAllScope| {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                siatka(c);
                c.close_all_scope = scope;
            });
            e.on_message(&mut b, &wiad(1, 1, None, KUPNO));
            e.on_message(&mut b, &wiad(1, 2, None, SPRZEDAZ));
            assert_eq!(e.baskets.len(), 2, "test wymaga DWÓCH żywych koszyków");
            for i in 0..2 {
                let id = e.baskets[i].id;
                let side = e.baskets[i].side;
                let t = b
                    .open_market(OrderReq {
                        side,
                        volume: 0.01,
                        sl: None,
                        tp: None,
                        basket: Some(id),
                        level: 0,
                        is_toucher: false,
                        comment: format!("B{id}"),
                    })
                    .unwrap();
                e.baskets[i].tickets.push(t);
            }
            e.on_message(
                &mut b,
                &wiad_ts(1, 3, Some(1), TS0 + 60_000, "CLOSE ALL NOW"),
            );
            (b, e)
        };

        let (b, e) = zaloz(crate::settings::CloseAllScope::Global);
        assert_eq!(
            b.positions().len(),
            0,
            "Global zamyka WSZYSTKO — stan sprzed 24.08"
        );
        assert!(
            e.baskets.iter().all(|x| x.state == BasketState::Done),
            "Global kończy oba koszyki"
        );

        let (b, e) = zaloz(crate::settings::CloseAllScope::Basket);
        assert_eq!(
            b.positions().len(),
            1,
            "Basket zamyka WYŁĄCZNIE koszyk-adresata — drugi zostaje żywy"
        );
        let adresat = e.baskets[0].id;
        assert_eq!(
            b.positions()[0].basket,
            Some(e.baskets[1].id),
            "zostać ma koszyk, do którego komunikat NIE był adresowany (adresat: B{adresat})"
        );
        assert_eq!(e.baskets[0].state, BasketState::Done, "adresat domknięty");
        assert!(e.baskets[1].alive(), "cudzy koszyk nietknięty");
    }

    #[test]
    fn w33_domyslny_zasieg_to_global() {
        assert_eq!(
            Settings::default().close_all_scope,
            crate::settings::CloseAllScope::Global,
            "kontrakt zera: domyślny zasięg close all to Global"
        );
    }
}

#[cfg(test)]
mod testy_krawedz_strefy {
    use super::testy_pakiet_a::{silnik, wiad, Atrapa, TS0};
    use super::*;

    const BUY6: &str = "BUY LIMITS GOLD @ 3900/3894\nTP 3910\nTP 3920\nSL 3880";
    const SELL6: &str = "SELL LIMITS GOLD @ 4094/4100\nTP 4084\nTP 4074\nSL 4101";
    const BUY_NAD_RYNKIEM: &str = "BUY LIMITS GOLD @ 4005/3999\nTP 4010\nTP 4020\nSL 3997";

    fn kanon(c: &mut Settings) {
        c.zone_offset_mode = ZoneOffsetMode::Directional;
        c.entry_deep_offset = -0.3;
        c.entry_tol_offset = 1.3;
        c.ppm_enabled = true;
        c.ppm_for_limits = true;
        c.ppm = 1.0;
        c.entry_units = 12;
        c.lot_mode_percent = false;
        c.lot_fixed = 0.01;
    }

    fn glebokie(c: &mut Settings) {
        kanon(c);
        c.entry_deep_offset = 3.0;
    }

    fn poziomy(e: &Engine) -> Vec<Px> {
        e.baskets[0]
            .levels
            .iter()
            .filter(|g| !g.is_toucher && g.level < 1000)
            .map(|g| g.price)
            .collect()
    }

    fn postaw(zmien: impl FnOnce(&mut Settings), tekst: &str) -> (Atrapa, Engine) {
        let mut b = Atrapa::nowa();
        let mut e = silnik(zmien);
        e.on_message(&mut b, &wiad(1, 100, None, tekst));
        (b, e)
    }

    #[test]
    fn a_zaden_poziom_ponizej_dolnej_krawedzi() {
        let (_, e0) = postaw(kanon, BUY6);
        let p0 = poziomy(&e0);
        assert_eq!(e0.baskets[0].entry_lo, 3894.0);
        assert_eq!(e0.baskets[0].entry_hi, 3900.0);
        assert_eq!(p0.len(), 8, "kanon Synergy: {p0:?}");
        assert!(
            p0.iter().all(|p| *p >= 3894.0 - 1e-9),
            "kanon i tak nie schodzi pod lo: {p0:?}"
        );

        let (_, e1) = postaw(glebokie, BUY6);
        let p1 = poziomy(&e1);
        let pod1 = p1.iter().filter(|p| **p < 3894.0 - 1e-9).count();
        assert_eq!(
            p1.len(),
            11,
            "strefa robocza 3891 … 3901,3, krok 1 $: {p1:?}"
        );
        assert_eq!(
            pod1, 3,
            "bez osi mają zostać trzy poziomy pod krawędzią: {p1:?}"
        );

        let (_, e2) = postaw(
            |c| {
                glebokie(c);
                c.zakaz_ponizej_krawedzi = true;
            },
            BUY6,
        );
        let p2 = poziomy(&e2);
        assert!(
            p2.iter().all(|p| *p >= 3894.0 - 1e-9),
            "oś włączona, a poziom pod dolną krawędzią został: {p2:?}"
        );
        assert_eq!(p2.len(), 8, "zostaje osiem poziomów 3894 … 3901: {p2:?}");
        assert!(
            (p2[0] - 3894.0).abs() < 1e-9,
            "najgłębszy poziom leży NA krawędzi: {p2:?}"
        );
    }

    #[test]
    fn b_sell_symetrycznie() {
        let (_, e1) = postaw(glebokie, SELL6);
        assert_eq!(
            e1.baskets[0].side,
            Side::Sell,
            "test wymaga sygnału sprzedaży"
        );
        assert_eq!(e1.baskets[0].entry_hi, 4100.0);
        let p1 = poziomy(&e1);
        let nad1 = p1.iter().filter(|p| **p > 4100.0 + 1e-9).count();
        assert!(
            nad1 > 0,
            "bez osi sprzedaż ma szczeble nad górną krawędzią: {p1:?}"
        );

        let (_, e2) = postaw(
            |c| {
                glebokie(c);
                c.zakaz_ponizej_krawedzi = true;
            },
            SELL6,
        );
        let p2 = poziomy(&e2);
        assert!(
            p2.iter().all(|p| *p <= 4100.0 + 1e-9),
            "oś włączona, a szczebel nad górną krawędzią został: {p2:?}"
        );
        assert!(
            (p2[0] - 4100.0).abs() < 1e-9,
            "najgłębszy poziom leży NA krawędzi: {p2:?}"
        );
    }

    #[test]
    fn c_wejscie_rynkowe_pod_strefa_odmowa() {
        let (b0, e0) = postaw(kanon, BUY_NAD_RYNKIEM);
        assert!(
            !b0.positions().is_empty(),
            "test wymaga, żeby bez osi ścieżka rynkowa naprawdę otworzyła pozycje"
        );
        assert!(
            b0.positions().iter().all(|p| p.open_price < 3999.0 - 1e-9),
            "pozycje mają powstać po cenie rynku (3998,3), a nie po cenie szczebla"
        );
        assert_eq!(
            e0.odrzuty.get("PonizejKrawedzi"),
            None,
            "bez osi nie ma odmów"
        );

        let (b1, e1) = postaw(
            |c| {
                kanon(c);
                c.zakaz_ponizej_krawedzi = true;
            },
            BUY_NAD_RYNKIEM,
        );
        assert!(
            b1.positions().is_empty(),
            "oś włączona, a wejście rynkowe pod strefą przeszło: {:?}",
            b1.positions()
                .iter()
                .map(|p| p.open_price)
                .collect::<Vec<_>>()
        );
        assert!(
            e1.odrzuty.get("PonizejKrawedzi").copied().unwrap_or(0) > 0,
            "odmowa musi zostawić ślad w liczniku odrzutów"
        );
        let slad = e1.baskets[0]
            .events
            .iter()
            .any(|z| z.text.contains("za dalszą krawędzią strefy"));
        assert!(
            slad,
            "odmowa musi zostawić wpis w dzienniku koszyka: {:?}",
            e1.baskets[0].events
        );
    }

    #[test]
    fn d_reentry_nie_wchodzi_pod_krawedzia() {
        let przygotuj = |zakaz: bool| {
            let mut b = Atrapa::nowa();
            let mut e = silnik(|c| {
                glebokie(c);
                c.reenter_after_tp = true;
                c.reenter_min_tp_stage = 1;
                c.reenter_max = 0;
                c.zakaz_ponizej_krawedzi = zakaz;
            });
            e.on_message(&mut b, &wiad(1, 100, None, BUY6));
            let id = e.baskets[0].id;
            b.pendings_mut().retain(|o| o.basket != Some(id));
            {
                let bk = e.basket_mut(id).unwrap();
                bk.tp_stage = 1;
                bk.had_positions = true;
                bk.last_entry_px = None;
            }
            b.ustaw_cene(TS0 + 60_000, 3891.9, 3892.0);
            let q = b.quote();
            e.reentry_pass(&mut b, &q);
            b.positions().len()
        };
        assert_eq!(
            przygotuj(false),
            1,
            "bez osi re-entry wchodzi pod krawędzią (stan sprzed naprawy)"
        );
        assert_eq!(
            przygotuj(true),
            0,
            "z osią re-entry pod krawędzią nie ma prawa powstać"
        );
    }

    #[test]
    fn e_kontrakt_zera() {
        assert!(
            !Settings::default().zakaz_ponizej_krawedzi,
            "kontrakt zera: oś ma startować wyłączona"
        );

        for tekst in [BUY6, SELL6, BUY_NAD_RYNKIEM] {
            for przygotuj in [kanon as fn(&mut Settings), glebokie as fn(&mut Settings)] {
                let (b0, e0) = postaw(przygotuj, tekst);
                let (b1, e1) = postaw(
                    |c| {
                        przygotuj(c);
                        c.zakaz_ponizej_krawedzi = false;
                    },
                    tekst,
                );
                assert_eq!(
                    poziomy(&e0),
                    poziomy(&e1),
                    "plan siatki ruszył się przy osi = false"
                );
                assert_eq!(
                    b0.positions().len(),
                    b1.positions().len(),
                    "liczba pozycji ruszyła się przy osi = false"
                );
                assert_eq!(
                    b0.pendings().len(),
                    b1.pendings().len(),
                    "liczba zleceń ruszyła się przy osi = false"
                );
                assert_eq!(
                    e1.odrzuty.get("PonizejKrawedzi"),
                    None,
                    "przy osi = false licznik odrzutów tej rodziny musi zostać pusty"
                );
            }
        }

        let e = silnik(|c| c.zakaz_ponizej_krawedzi = false);
        assert!(!e.za_dalsza_krawedzia(Side::Buy, 3800.0, 3894.0, 3900.0));
        assert!(!e.za_dalsza_krawedzia(Side::Sell, 4200.0, 4094.0, 4100.0));
        let e = silnik(|c| c.zakaz_ponizej_krawedzi = true);
        assert!(e.za_dalsza_krawedzia(Side::Buy, 3800.0, 3894.0, 3900.0));
        assert!(
            !e.za_dalsza_krawedzia(Side::Buy, 3894.0, 3894.0, 3900.0),
            "NA krawędzi wolno"
        );
        assert!(e.za_dalsza_krawedzia(Side::Sell, 4200.0, 4094.0, 4100.0));
        assert!(
            !e.za_dalsza_krawedzia(Side::Sell, 4100.0, 4094.0, 4100.0),
            "NA krawędzi wolno"
        );
        assert!(!e.za_dalsza_krawedzia(Side::Buy, 3800.0, 3900.0, 3900.0));
    }
}

#[cfg(test)]
mod testy_margines_lancuch {
    use super::testy_pakiet_a::{silnik, wiad, Atrapa, TS0};
    use super::*;
    use crate::routing::{Widok, Wlasnosc};
    use crate::wielosilnik;

    const SLOT_A: u32 = 1;
    const SLOT_B: u32 = 2;

    fn kto(slot: u32, zapasowy: bool) -> Wlasnosc {
        Wlasnosc {
            slot,
            zapasowy,
            znane_sloty: vec![SLOT_A, SLOT_B],
        }
    }

    fn otworz(b: &mut Atrapa, id: u32, vol: f64) {
        b.open_market(OrderReq {
            side: Side::Buy,
            volume: vol,
            sl: None,
            tp: None,
            basket: Some(id),
            level: 0,
            is_toucher: false,
            comment: String::new(),
        })
        .expect("atrapa nie odmawia otwarcia");
    }

    fn prawda(b: &Atrapa) -> f64 {
        let acc = b.account();
        let lev = acc.leverage.max(1) as f64;
        let m: f64 = b
            .positions()
            .iter()
            .map(|p| p.volume * XAU_CONTRACT * p.open_price / lev)
            .sum();
        acc.equity / m * 100.0
    }

    #[test]
    fn obie_nogi_licza_ten_sam_poziom_marginesu_calego_rachunku() {
        let mut b = Atrapa::nowa();
        otworz(&mut b, wielosilnik::baza_slotu(SLOT_A) + 1, 0.10);
        otworz(&mut b, wielosilnik::baza_slotu(SLOT_B) + 1, 0.02);
        let oczekiwany = prawda(&b);

        let mut ea = silnik(|_| {});
        ea.przypisz_slot(SLOT_A);
        let mut eb = silnik(|_| {});
        eb.przypisz_slot(SLOT_B);

        let mut poczekalnia = Vec::new();
        let a = {
            let w = Widok::nowy(&mut b, kto(SLOT_A, false), &mut poczekalnia);
            ea.poziom_marginesu(&w).0.expect("noga A ma ekspozycję")
        };
        let bb = {
            let w = Widok::nowy(&mut b, kto(SLOT_B, false), &mut poczekalnia);
            eb.poziom_marginesu(&w)
                .0
                .expect("noga B widzi margines rachunku")
        };

        assert!(
            (a - oczekiwany).abs() < 1e-9,
            "noga A liczy {a:.2} % zamiast {oczekiwany:.2} % (margines JEDNEJ nogi w mianowniku)"
        );
        assert!(
            (bb - oczekiwany).abs() < 1e-9,
            "noga B liczy {bb:.2} % zamiast {oczekiwany:.2} % (margines JEDNEJ nogi w mianowniku)"
        );
        assert_eq!(
            a.to_bits(),
            bb.to_bits(),
            "dwie nogi jednego rachunku podały różny poziom marginesu: {a} vs {bb}"
        );
    }

    #[test]
    fn jeden_silnik_poziom_marginesu_bit_w_bit_bez_zmian() {
        let mut b = Atrapa::nowa();
        otworz(&mut b, 1, 0.07);
        otworz(&mut b, wielosilnik::baza_slotu(7) + 3, 0.03);
        b.place_pending(PendingReq {
            kind: PendingKind::BuyLimit,
            volume: 0.05,
            price: 3990.0,
            sl: None,
            tp: None,
            basket: Some(1),
            level: 1,
            is_toucher: false,
            is_topup: false,
            no_market_fallback: false,
            comment: String::new(),
        })
        .expect("atrapa nie odmawia");

        let mut e = silnik(|_| {});
        let (t_bez, d_bez) = e.poziom_marginesu(&b);

        let mut poczekalnia = Vec::new();
        let kto = Wlasnosc {
            slot: wielosilnik::SLOT_STARY,
            zapasowy: true,
            znane_sloty: vec![0],
        };
        let w = Widok::nowy(&mut b, kto, &mut poczekalnia);
        let (t_widok, d_widok) = e.poziom_marginesu(&w);
        drop(w);

        assert_eq!(
            t_bez.map(f64::to_bits),
            t_widok.map(f64::to_bits),
            "jeden silnik: poziom marginesu TERAZ drgnął przez widok"
        );
        assert_eq!(
            d_bez.map(f64::to_bits),
            d_widok.map(f64::to_bits),
            "jeden silnik: poziom marginesu DOCELOWY drgnął przez widok"
        );
        assert!(t_bez.is_some_and(|x| x > 0.0));
    }

    #[test]
    fn ml_min_wejscie_blokuje_noge_b_gdy_margines_zjadla_noga_a() {
        let mut b = Atrapa::nowa();
        otworz(&mut b, wielosilnik::baza_slotu(SLOT_A) + 1, 0.5);
        assert!(
            prawda(&b) < 150.0,
            "test wymaga rachunku pod progiem: {:.1} %",
            prawda(&b)
        );

        let mut eb = silnik(|c| c.ml_min_wejscie = 150.0);
        eb.przypisz_slot(SLOT_B);

        let mut poczekalnia = Vec::new();
        {
            let mut w = Widok::nowy(&mut b, kto(SLOT_B, false), &mut poczekalnia);
            eb.on_message(
                &mut w,
                &wiad(
                    1,
                    1,
                    None,
                    "BUY LIMITS GOLD @ 4000/3995\nTP 4010\nTP 4020\nSL 3990",
                ),
            );
        }

        assert_eq!(
            eb.odrzuty.get("MarginLevel").copied(),
            Some(1),
            "noga B weszła mimo marginesu zjedzonego przez nogę A (odrzuty: {:?})",
            eb.odrzuty
        );
        assert!(
            eb.baskets.is_empty(),
            "zablokowane wejście nie ma prawa zostawić koszyka"
        );
        let _ = TS0;
    }
}

pub static ODSIANE_SZCZEBLE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static KOSZYKI_Z_ODSIEWEM: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn odsiew_sita() -> (u64, u64) {
    (
        ODSIANE_SZCZEBLE.load(std::sync::atomic::Ordering::Relaxed),
        KOSZYKI_Z_ODSIEWEM.load(std::sync::atomic::Ordering::Relaxed),
    )
}

#[cfg(test)]
mod profit_budget_send_tests {
    use super::*;
    use crate::profit_budget::tests::{broker,cfg,anchor};
    fn rig()->(Engine,crate::profit_budget::tests::TestBroker) {
        let c=cfg();let b=broker();let a=anchor(&b,&c);let mut e=Engine::new(c,600.0);
        e.stats.day=a.day;e.stats.day_start_equity=a.start;e.stats.day_peak_equity=a.peak;
        e.stats.equity=b.account().equity;(e,b)
    }
    fn market(level:i32)->OrderReq {OrderReq{side:Side::Buy,volume:1.0,sl:Some(3990.0),tp:Some(4100.0),
        basket:Some(1),level,is_toucher:false,comment:"synthetic-profit-budget".into()}}
    fn pending_req(level:i32)->PendingReq {PendingReq {kind:PendingKind::BuyLimit,price:3990.0,
        volume:1.0,sl:Some(3980.0),tp:Some(4100.0),basket:Some(1),level,
        is_toucher:false,is_topup:false,no_market_fallback:false,comment:"synthetic-profit-budget".into()}}
    #[test]
    fn profit_budget_every_market_send_remeasures_budget_including_reentry_levels() {
        for level in [0,-2,-6] {
            let (mut e,mut b)=rig();
            e.open_market_order(&mut b,market(level)).unwrap();
            assert_eq!(b.positions[0].volume,0.04);
            assert!(e.open_market_order(&mut b,market(level)).is_err());
            assert_eq!(b.sends,1,"second send cannot reuse the first order's budget");
            assert_eq!(e.odrzuty.get("ProfitBudget::Exhausted"),Some(&1));
        }
    }
    #[test]
    fn profit_budget_pending_rearm_topup_and_market_cannot_bypass_live_exposure() {
        for level in [0,2,-2] {
            let (mut e,mut b)=rig();e.place_pending_order(&mut b,pending_req(level)).unwrap();
            assert_eq!(b.pendings[0].volume,0.05);
            assert!(e.place_pending_order(&mut b,pending_req(level)).is_err());
            assert!(e.open_market_order(&mut b,market(level)).is_err());
            assert_eq!(b.sends,1);
            // Only actual disappearance of the old order releases its budget.
            b.cancel_pending(1).unwrap();e.place_pending_order(&mut b,pending_req(level)).unwrap();
            assert_eq!(b.sends,2);
        }
    }
    #[test]
    fn configured_portfolio_cap_remeasures_every_market_and_pending_send_before_profit_arm() {
        for arm in [0.0,100.0] {
            for level in [0,-2,-4,-6] {
                let (mut e,mut b)=rig();e.cfg.profit_budget_arm_pct=arm;e.cfg.max_portfolio_risk_pct=5.0;
                e.open_market_order(&mut b,market(level)).unwrap();
                assert_eq!(b.positions[0].volume,0.03);
                assert!(e.open_market_order(&mut b,market(level)).is_err());assert_eq!(b.sends,1);
                let (mut e,mut b)=rig();e.cfg.profit_budget_arm_pct=arm;e.cfg.max_portfolio_risk_pct=5.0;
                e.place_pending_order(&mut b,pending_req(level)).unwrap();
                assert_eq!(b.pendings[0].volume,0.03);
                assert!(e.place_pending_order(&mut b,pending_req(level)).is_err());assert_eq!(b.sends,1);
            }
        }
    }

    #[test]
    fn profit_budget_manual_risk_resume_keeps_the_daily_profit_anchor() {
        let (mut e,mut b)=rig();e.stats.day_peak_equity=800.0;e.stats.equity=700.0;
        e.halted=Some("synthetic guard".into());e.resume_trading(b.q.ts);
        assert!(e.risk_override);assert_eq!(e.stats.day_peak_equity,800.0);
        assert!(e.open_market_order(&mut b,market(0)).is_err(),"manual guard override erased the profit reserve");
        e.cfg.profit_budget_arm_pct=0.0;e.resume_trading(b.q.ts);
        assert_eq!(e.stats.day_peak_equity,700.0,"OFF keeps legacy resume behavior");
    }
}

#[cfg(test)]
mod entry_observation_tests {
    use super::*;
    use crate::profit_budget::tests::broker;
    fn msg(chat:i64,id:i64,edit:bool,text:&str)->IncomingMessage {IncomingMessage{ts:1_700_000_000_000,
        source:SourceKey::new(chat,None),source_name:"synthetic-observation".into(),msg_id:id,
        reply_to:None,edit_of:edit.then_some(id),text:text.into()}}
    #[test]
    fn early_orphan_rejections_record_current_geometry_and_never_leak_previous_entry_context() {
        let mut e=Engine::new(Settings{edycja_sieroty_nie_otwiera:true,..Settings::default()},600.0);let mut b=broker();
        e.on_message(&mut b,&msg(-900001,10,false,"BUY LIMIT GOLD @ 3990/3985\nSL 3980\nTP 4050"));
        let created=e.created_baskets_count();
        e.on_message(&mut b,&msg(-900002,10,true,"SELL LIMIT GOLD @ 4020/4025\nSL 4030\nTP 3990"));
        let w=e.odrzucone_wejscia.last().expect("complete orphan has measurable rejection geometry");
        assert_eq!(w.kod,"EditOrphan");assert_eq!(w.side,Side::Sell);assert_eq!(w.lo,4020.0);
        assert_eq!(w.hi,4025.0);assert_eq!(w.sl,Some(4030.0));assert_eq!(e.created_baskets_count(),created);
        let rows=e.odrzucone_wejscia.len();
        // Same ID on another source must not borrow the prior Entry's geometry.
        e.on_message(&mut b,&msg(-900003,10,true,"BUY NOW"));
        assert_eq!(e.odrzucone_wejscia.len(),rows);
        assert_eq!(e.entry_source_observations().len(),3,"source identity includes the channel");
        assert_eq!(e.entry_source_observations().values().filter(|o|o.has_full_entry).count(),2);
        assert_eq!(e.entry_source_observations().values().filter(|o|o.first_seen_as_edit).count(),2);
        e.on_message(&mut b,&msg(-900002,10,true,"SELL LIMIT GOLD @ 4021/4026\nSL 4031\nTP 3990"));
        assert_eq!(e.entry_source_observations().len(),3,"material revisions are not new source IDs");
    }
}

#[cfg(test)]
mod fast_addon_precheck_tests {
    use super::*;
    use super::testy_pakiet_a::{Atrapa,wiad,WEJSCIE,TS0};
    fn prepared()->(Engine,Atrapa,Quote) {
        let mut e=Engine::new(Settings {fast_addon_move_usd:1.0,fast_addon_max:1,
            fast_addon_window_s:60.0,fast_addon_cooldown_s:0.0,fast_addon_min_stage:0,
            fast_addon_lot_mult:3.0,lot_mode_percent:false,lot_fixed:0.05,lot_max:0.1,
            max_open_positions:0,max_open_baskets:0,risk_per_basket_pct:0.0,
            max_portfolio_risk_pct:0.0,order_volume_contract_v2:false,..Settings::default()},400.0);
        let mut b=Atrapa::nowa();b.maximum_volume=100.0;
        e.on_message(&mut b,&wiad(1,100,None,WEJSCIE));
        assert!(b.positions().iter().any(|p|p.basket==Some(e.baskets[0].id)));
        e.vol_hist=vec![(TS0+10_000,4000.0),(TS0+20_000,4001.0),(TS0+25_000,4002.0)];
        b.ustaw_cene(TS0+30_000,4005.0,4005.2);let q=b.quote();
        (e,b,q)
    }
    #[test]
    fn invalid_tp_without_submission_does_not_consume_slot_and_later_revision_sends_once() {
        let (mut e,mut b,q)=prepared();let id=e.baskets[0].id;
        e.basket_mut(id).unwrap().tps=vec![4004.0];
        let before=b.positions().len();
        e.fast_addon_sweep(&mut b,&q);e.fast_addon_sweep(&mut b,&q);
        assert_eq!(b.positions().len(),before);assert_eq!(e.baskets[0].fast_addons,0);
        assert_eq!(e.baskets[0].last_addon_ts,q.ts,"invalid attempt preserves configured cooldown");
        assert_eq!(e.baskets[0].events.iter().filter(|x|x.text.contains("capacity remains available")).count(),1);
        e.basket_mut(id).unwrap().tps=vec![4030.0];
        e.fast_addon_sweep(&mut b,&q);e.fast_addon_sweep(&mut b,&q);
        let addons:Vec<_>=b.positions().iter().filter(|p|p.level==-4).collect();
        assert_eq!(addons.len(),1);assert_eq!(addons[0].tp,Some(4030.0));
        assert_eq!(addons[0].volume,0.1,"multiplier cannot exceed lot_max even with volume V2 off");
        assert_eq!(e.baskets[0].fast_addons,1);assert_eq!(e.baskets[0].last_addon_ts,q.ts);
    }
    #[test]
    fn submitted_broker_failure_keeps_the_existing_no_duplicate_retry_guard() {
        let (mut e,mut b,q)=prepared();let before=b.positions().len();b.market_failures=1;
        e.fast_addon_sweep(&mut b,&q);
        assert_eq!(b.market_failures,0,"the broker must have received the request");
        assert_eq!(e.baskets[0].fast_addons,1);assert_eq!(e.baskets[0].last_addon_ts,q.ts);
        e.fast_addon_sweep(&mut b,&q);
        assert_eq!(b.positions().len(),before,"ambiguous broker error cannot cause a second request");
    }
    #[test]
    fn local_portfolio_rejection_preserves_addon_slot_until_existing_risk_is_released() {
        let (mut e,mut b,q)=prepared();e.cfg.max_portfolio_risk_pct=10.0;
        let before=b.positions().len();let submissions=e.order_submission_sequence;
        e.fast_addon_sweep(&mut b,&q);
        assert_eq!(e.baskets[0].fast_addons,0);assert_eq!(e.baskets[0].last_addon_ts,0);
        assert_eq!(e.order_submission_sequence,submissions,"local budget gate called the broker");
        assert_eq!(b.positions().len(),before);
        for p in b.positions_mut(){p.sl=Some(4004.0);}
        for p in b.pendings().to_vec(){b.cancel_pending(p.ticket).unwrap();}
        let available=crate::profit_budget::available(&e.cfg,(&e.stats).into(),&b,None).unwrap().unwrap();
        assert!(available.remaining>15.2,"fixture must free at least one new minimum lot: {available:?}");
        e.fast_addon_sweep(&mut b,&q);e.fast_addon_sweep(&mut b,&q);
        assert_eq!(e.order_submission_sequence,submissions.wrapping_add(1));
        assert_eq!(b.positions().len(),before+1);assert_eq!(e.baskets[0].fast_addons,1);
        assert_eq!(b.positions().last().unwrap().volume,0.02,"remaining live downside determines the safe lot step");
    }

    #[test]
    fn local_invalid_tp_keeps_the_configured_attempt_cooldown_without_consuming_capacity() {
        let (mut e,mut b,q)=prepared();e.cfg.fast_addon_cooldown_s=60.0;
        let id=e.baskets[0].id;e.basket_mut(id).unwrap().tps=vec![4004.0];
        let before=e.order_submission_sequence;e.fast_addon_sweep(&mut b,&q);
        assert_eq!(e.baskets[0].fast_addons,0);assert_eq!(e.baskets[0].last_addon_ts,q.ts);
        e.basket_mut(id).unwrap().tps=vec![4030.0];
        b.ustaw_cene(q.ts+10_000,4005.0,4005.2);let next=b.quote();
        e.fast_addon_sweep(&mut b,&next);
        assert_eq!(e.order_submission_sequence,before,"valid TP cannot bypass configured cooldown");
        e.vol_hist=vec![(q.ts+40_000,4000.0),(q.ts+50_000,4001.0),(q.ts+55_000,4002.0)];
        b.ustaw_cene(q.ts+60_000,4005.0,4005.2);let later=b.quote();
        e.fast_addon_sweep(&mut b,&later);
        assert_eq!(e.order_submission_sequence,before.wrapping_add(1));
        assert_eq!(e.baskets[0].fast_addons,1);assert_eq!(e.baskets[0].last_addon_ts,later.ts);
    }

}
