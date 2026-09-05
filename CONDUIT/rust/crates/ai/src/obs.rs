//! Obserwacja — cechy wejściowe modelu.
//!
//! Trzy bloki cech, składane w jeden wektor:
//!
//! ```text
//!   [ GLOBAL (rynek + konto) | KOSZYK | POZYCJA ]
//!   |<------- wejście sieci koszyka ---->|
//!   |<-------------- wejście sieci pozycji -------------->|
//! ```
//!
//! Dzięki temu układowi jeden bufor obsługuje obie sieci: sieć koszyka czyta
//! prefiks `[0..BSK_IN]`, sieć pozycji cały wektor `[0..POS_IN]`. Zero alokacji
//! w gorącej pętli.
//!
//! **Normalizacja.** Model ma działać przy złocie po 4700 i po 2000, więc żadna
//! cecha nie może być ceną. Wszystko jest dzielone przez jedną z trzech skal:
//!  * `atr` — zmienność minutowa (odległości cenowe),
//!  * szerokość strefy sygnału (geometria wejścia),
//!  * kapitał startowy (pieniądze).
//!
//! Każda cecha przechodzi na końcu przez `norm()`, które zamienia NaN/±inf na 0
//! i przycina do ±8. To jest twarda gwarancja, że sieć nigdy nie dostanie NaN —
//! niezależnie od tego, jak dziwne dane przyjdą z rynku.

use conduit_core::types::*;

// ============================================================
//  WYMIARY
// ============================================================

pub const G_DIM: usize = 29;
pub const B_DIM: usize = 22;
pub const P_DIM: usize = 18;

/// Wejście sieci koszyka.
pub const BSK_IN: usize = G_DIM + B_DIM;
/// Wejście sieci pozycji.
pub const POS_IN: usize = G_DIM + B_DIM + P_DIM;

pub const GLOBAL_NAMES: [&str; G_DIM] = [
    "spread_atr",
    "atr_rel",
    "ret_1m",
    "ret_5m",
    "ret_15m",
    "ret_60m",
    "vol_5m",
    "vol_60m",
    "equity_ret",
    "balance_ret",
    "float_pnl",
    "dd_now",
    "dd_max",
    "margin_util",
    "n_pos",
    "n_pend",
    "net_lots",
    "gross_lots",
    "hour_sin",
    "hour_cos",
    "day_pnl",
    "room_to_floor",
    "open_risk",
    "ret_4h",
    "ret_24h",
    "ret_72h",
    "vol_24h",
    "loss_streak",
    "since_win",
];

pub const BASKET_NAMES: [&str; B_DIM] = [
    "bk_side",
    "bk_adv_zone",
    "bk_adv_atr",
    "bk_sl_dist_atr",
    "bk_has_sl",
    "bk_tp_dist_atr",
    "bk_has_tp",
    "bk_tp_span_atr",
    "bk_stage_frac",
    "bk_n_tps",
    "bk_age",
    "bk_n_open",
    "bk_n_pend",
    "bk_realized",
    "bk_floating",
    "bk_is_limit",
    "bk_armed",
    "bk_riskfree",
    "bk_sl_width_atr",
    "bk_rr_shallow",
    "bk_rr_deep",
    "bk_width_atr",
];

pub const POSITION_NAMES: [&str; P_DIM] = [
    "p_pnl_atr",
    "p_pnl_rel",
    "p_peak_atr",
    "p_give_atr",
    "p_retrace",
    "p_age",
    "p_since_peak",
    "p_sl_dist_atr",
    "p_has_sl",
    "p_locked_atr",
    "p_tp_dist_atr",
    "p_has_tp",
    "p_vol_rel",
    "p_level",
    "p_is_toucher",
    "p_is_runner",
    "p_entry_depth",
    "p_risk_rel",
];

/// Wszystkie nazwy cech w kolejności wektora wejściowego sieci pozycji.
pub fn feature_names() -> Vec<String> {
    GLOBAL_NAMES
        .iter()
        .chain(BASKET_NAMES.iter())
        .chain(POSITION_NAMES.iter())
        .map(|s| s.to_string())
        .collect()
}

/// Jedyna droga, którą liczba trafia do sieci. Bez wyjątków.
///
/// NaN → 0 (brak informacji), ±∞ → ±8 (informacja o znaku zostaje zachowana).
#[inline]
pub fn norm(x: f64) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(-8.0, 8.0) as f32
    }
}

#[inline]
fn ln_age(minutes: f64) -> f64 {
    // 0 min → 0, 1 doba → 1; dalej rośnie powoli
    (1.0 + minutes.max(0.0)).ln() / 1441.0f64.ln()
}

// ============================================================
//  OKNO RYNKOWE
// ============================================================

const RING: usize = 128;
/// Ile ostatnich GODZIN trzymamy — 96 h wystarcza na kontekst tygodnia handlowego.
const HRING: usize = 96;
/// Poniżej tej wartości ATR nie schodzi — inaczej dzielenie przez zmienność
/// martwego rynku (np. w niedzielę) wysadza wszystkie cechy w kosmos.
const ATR_FLOOR: f64 = 0.03;
const ATR_ALPHA: f64 = 1.0 / 14.0;

/// Pamięć rynku: zamknięcia minutowe + wygładzona zmienność.
///
/// Trzymamy tylko `RING` ostatnich ZAREJESTROWANYCH minut, nie minut
/// kalendarzowych — dzięki temu weekendowa dziura nie zjada całej historii.
/// Skok o więcej niż 10 minut (weekend, przerwa w danych) nie aktualizuje ATR,
/// żeby luka nie została policzona jako zmienność.
#[derive(Clone)]
pub struct MarketWindow {
    closes: [f64; RING],
    /// zmiany minuta-do-minuty, do liczenia zmienności zrealizowanej
    diffs: [f64; RING],
    head: usize,
    filled: usize,
    cur_min: i64,
    last_mid: f64,
    prev_close: f64,
    atr: f64,
    started: bool,
    // --- druga, wolniejsza skala: zamknięcia godzinowe (kontekst reżimu) ---
    hours: [f64; HRING],
    h_head: usize,
    h_filled: usize,
    cur_hour: i64,
}

impl Default for MarketWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketWindow {
    pub fn new() -> Self {
        MarketWindow {
            closes: [0.0; RING],
            diffs: [0.0; RING],
            head: 0,
            filled: 0,
            cur_min: i64::MIN,
            last_mid: 0.0,
            prev_close: 0.0,
            atr: 0.0,
            started: false,
            hours: [0.0; HRING],
            h_head: 0,
            h_filled: 0,
            cur_hour: i64::MIN,
        }
    }

    /// Wołane na KAŻDYM ticku (także w rozgrzewce przed oknem testowym).
    #[inline]
    pub fn on_tick(&mut self, q: &Quote) {
        let mid = q.mid();
        let m = q.ts.div_euclid(60_000);
        let h = q.ts.div_euclid(3_600_000);
        if !self.started {
            self.started = true;
            self.cur_hour = h;
            self.cur_min = m;
            self.last_mid = mid;
            self.prev_close = mid;
            self.atr = ATR_FLOOR;
            return;
        }
        if m != self.cur_min {
            let gap = m.saturating_sub(self.cur_min);
            let c = self.last_mid;
            let d = c - self.prev_close;
            self.closes[self.head] = c;
            self.diffs[self.head] = if gap <= 10 { d } else { 0.0 };
            self.head = (self.head + 1) % RING;
            if self.filled < RING {
                self.filled += 1;
            }
            if gap <= 10 {
                self.atr = self.atr * (1.0 - ATR_ALPHA) + d.abs() * ATR_ALPHA;
            }
            self.prev_close = c;
            self.cur_min = m;
        }
        if h != self.cur_hour {
            self.hours[self.h_head] = self.last_mid;
            self.h_head = (self.h_head + 1) % HRING;
            if self.h_filled < HRING {
                self.h_filled += 1;
            }
            self.cur_hour = h;
        }
        self.last_mid = mid;
    }

    /// Zmienność odniesienia (średnia bezwzględna zmiana minutowa).
    #[inline]
    pub fn atr(&self) -> f64 {
        self.atr.max(ATR_FLOOR)
    }

    #[inline]
    pub fn price(&self) -> f64 {
        self.last_mid
    }

    /// Zamknięcie sprzed `k` zarejestrowanych minut (`k = 1` → ostatnie).
    #[inline]
    fn close_back(&self, k: usize) -> Option<f64> {
        if k == 0 || k > self.filled {
            return None;
        }
        Some(self.closes[(self.head + RING - k) % RING])
    }

    /// Zwrot za ostatnie `k` minut, w jednostkach ceny.
    #[inline]
    pub fn ret(&self, k: usize) -> f64 {
        match self.close_back(k) {
            Some(c) => self.last_mid - c,
            None => 0.0,
        }
    }

    /// Zamknięcie sprzed `k` zarejestrowanych GODZIN.
    #[inline]
    fn hour_back(&self, k: usize) -> Option<f64> {
        if k == 0 || k > self.h_filled {
            return None;
        }
        Some(self.hours[(self.h_head + HRING - k) % HRING])
    }

    /// Zwrot za ostatnie `k` godzin, w jednostkach ceny.
    ///
    /// To jest sygnał REŻIMU, nie sygnał wejścia. Czerwiec i lipiec są ujemne dla
    /// każdego presetu; jeśli model ma nauczyć się zmniejszać ekspozycję w złym
    /// reżimie, musi widzieć rynek w skali dni, a nie minut.
    #[inline]
    pub fn ret_h(&self, k: usize) -> f64 {
        match self.hour_back(k) {
            Some(c) => self.last_mid - c,
            None => 0.0,
        }
    }

    /// Odchylenie standardowe zmian GODZINOWYCH z ostatnich `k` godzin.
    pub fn vol_h(&self, k: usize) -> f64 {
        let n = k.min(self.h_filled);
        if n < 3 {
            return 0.0;
        }
        let mut s = 0.0;
        let mut s2 = 0.0;
        for i in 1..n {
            let a = self.hours[(self.h_head + HRING - i) % HRING];
            let b = self.hours[(self.h_head + HRING - i - 1) % HRING];
            let d = a - b;
            s += d;
            s2 += d * d;
        }
        let m = s / (n - 1) as f64;
        (s2 / (n - 1) as f64 - m * m).max(0.0).sqrt()
    }

    /// Odchylenie standardowe zmian minutowych z ostatnich `k` minut.
    pub fn vol(&self, k: usize) -> f64 {
        let n = k.min(self.filled);
        if n < 2 {
            return 0.0;
        }
        let mut s = 0.0;
        let mut s2 = 0.0;
        for i in 1..=n {
            let d = self.diffs[(self.head + RING - i) % RING];
            s += d;
            s2 += d * d;
        }
        let m = s / n as f64;
        (s2 / n as f64 - m * m).max(0.0).sqrt()
    }
}

// ============================================================
//  KONTEKST GLOBALNY
// ============================================================

/// Wszystko, co model wie o świecie poza konkretną pozycją.
pub struct GlobalCtx<'a> {
    pub q: &'a Quote,
    pub mw: &'a MarketWindow,
    pub acc: &'a Account,
    pub start_balance: f64,
    pub peak_equity: f64,
    pub max_dd_abs: f64,
    pub day_start_equity: f64,
    pub n_pos: usize,
    pub n_pend: usize,
    pub buy_lots: f64,
    pub sell_lots: f64,
    pub base_lot: f64,
    pub equity_floor: f64,
    pub tz_offset_ms: i64,
    /// ile strat z rzędu (najtańsza cecha rozpoznająca zły dzień)
    pub loss_streak: u32,
    /// godziny od ostatniej wygranej transakcji
    pub since_win_h: f64,
    /// łączne OTWARTE ryzyko: Σ |wejście − SL| × 100 × wolumen (patrz [`open_risk`])
    pub open_risk: f64,
}

/// Skale normalizacji — jedyne, czego potrzebują cechy koszyka i pozycji.
///
/// Świadomie NIE przekazujemy tam `GlobalCtx`: cechy lokalne nie mają prawa
/// zależeć od stanu konta inaczej niż przez skalę pieniężną, a przy okazji
/// runtime nie musi trzymać przy życiu pożyczki na `MarketWindow`.
#[derive(Clone, Copy)]
pub struct Scale<'a> {
    pub q: &'a Quote,
    pub atr: f64,
    /// kapitał startowy — skala pieniężna
    pub cap: f64,
    pub base_lot: f64,
}

impl<'a> Scale<'a> {
    pub fn of(g: &GlobalCtx<'a>) -> Self {
        Scale {
            q: g.q,
            atr: g.mw.atr(),
            cap: g.start_balance.max(1.0),
            base_lot: g.base_lot.max(0.01),
        }
    }
}

pub fn global_features(out: &mut [f32], c: &GlobalCtx) {
    debug_assert!(out.len() >= G_DIM);
    let atr = c.mw.atr();
    let px = c.q.mid().max(1.0);
    let cap = c.start_balance.max(1.0);
    let lot = c.base_lot.max(0.01);
    let eq = c.acc.equity;

    let hour = hour_of(c.q.ts, c.tz_offset_ms) as f64
        + ((c.q.ts + c.tz_offset_ms).rem_euclid(3_600_000) as f64) / 3_600_000.0;
    let ang = hour / 24.0 * std::f64::consts::TAU;

    out[0] = norm(c.q.spread() / atr);
    out[1] = norm(atr / px * 1000.0);
    out[2] = norm(c.mw.ret(1) / atr);
    out[3] = norm(c.mw.ret(5) / atr);
    out[4] = norm(c.mw.ret(15) / atr);
    out[5] = norm(c.mw.ret(60) / atr);
    out[6] = norm(c.mw.vol(5) / atr);
    out[7] = norm(c.mw.vol(60) / atr);
    out[8] = norm(eq / cap - 1.0);
    out[9] = norm(c.acc.balance / cap - 1.0);
    out[10] = norm((eq - c.acc.balance) / cap);
    out[11] = norm((c.peak_equity - eq) / c.peak_equity.max(1.0));
    out[12] = norm(c.max_dd_abs / cap);
    out[13] = norm(c.acc.margin / eq.max(1.0));
    out[14] = norm(c.n_pos as f64 / 10.0);
    out[15] = norm(c.n_pend as f64 / 10.0);
    out[16] = norm((c.buy_lots - c.sell_lots) / (10.0 * lot));
    out[17] = norm((c.buy_lots + c.sell_lots) / (10.0 * lot));
    out[18] = norm(ang.sin());
    out[19] = norm(ang.cos());
    out[20] = norm((eq - c.day_start_equity) / cap);
    out[21] = norm((eq - c.equity_floor) / cap);
    out[22] = norm(c.open_risk / cap);

    // --- REŻIM: skala godzinowa, normalizacja pierwiastkiem czasu ---
    // Zwrot za N minut skaluje się jak ATR × √N przy błądzeniu losowym, więc
    // dzielenie przez ATR×√N trzyma wszystkie horyzonty w tym samym rzędzie
    // wielkości niezależnie od tego, czy patrzymy na godzinę, czy na trzy doby.
    let sc = |minutes: f64| atr * minutes.sqrt();
    out[23] = norm(c.mw.ret_h(4) / sc(240.0));
    out[24] = norm(c.mw.ret_h(24) / sc(1440.0));
    out[25] = norm(c.mw.ret_h(72) / sc(4320.0));
    out[26] = norm(c.mw.vol_h(24) / (atr * 60.0f64.sqrt()));
    out[27] = norm(c.loss_streak as f64 / 5.0);
    out[28] = norm(ln_age(c.since_win_h * 60.0));
}

// ============================================================
//  OTWARTE RYZYKO
// ============================================================

/// Łączne ryzyko otwartych pozycji: ile stracimy, jeśli KAŻDA z nich wyjdzie po
/// swoim stop-lossie.
///
/// Szeroki SL może wyglądać świetnie w próbce, w której rzadko zostaje
/// zrealizowany: zrealizowane obsunięcie jest wtedy małe mimo dużego otwartego
/// ryzyka. Ta funkcja mierzy ryzyko wprost, niezależnie od tego, czy historia
/// akurat je zrealizowała.
///
/// Pozycja BEZ stop-lossa ma ryzyko nieograniczone. Wyceniamy je zastępczo na
/// `no_sl_atr × ATR`, żeby model nie mógł „schować" ryzyka przez zdjęcie SL.
pub fn open_risk(positions: &[Position], atr: f64, no_sl_atr: f64) -> f64 {
    let mut r = 0.0;
    for p in positions {
        let dist = match p.sl {
            Some(s) => ((p.open_price - s) * p.side.sign()).max(0.0),
            None => no_sl_atr * atr,
        };
        r += dist * XAU_CONTRACT * p.volume;
    }
    r
}

// ============================================================
//  KONTEKST KOSZYKA
// ============================================================

/// Kopia danych koszyka wystarczająca do policzenia cech.
///
/// Świadomie NIE bierzemy `Basket::tickets` — silnik nie dopisuje tam pozycji
/// powstałych z realizacji limitów (ticket zlecenia ≠ ticket pozycji). Prawdę o
/// tym, co jest otwarte, ma wyłącznie broker, więc liczniki przychodzą z niego.
#[derive(Clone, Copy)]
pub struct BasketCtx {
    pub side: Side,
    pub zone_lo: Px,
    pub zone_hi: Px,
    pub sl: Option<Px>,
    pub first_tp: Option<Px>,
    pub next_tp: Option<Px>,
    pub last_tp: Option<Px>,
    pub tp_stage: usize,
    pub n_tps: usize,
    pub created_ts: Ts,
    pub is_limit: bool,
    pub armed: bool,
    pub risk_free: bool,
    pub realized: f64,
    /// niezrealizowany wynik wszystkich pozycji koszyka
    pub floating: f64,
    pub n_open: usize,
    pub n_pend: usize,
}

impl BasketCtx {
    /// Kontekst zastępczy dla pozycji-sieroty (koszyk zamknięty lub nieznany).
    pub fn orphan(side: Side, price: Px) -> Self {
        BasketCtx {
            side,
            zone_lo: price,
            zone_hi: price,
            sl: None,
            first_tp: None,
            next_tp: None,
            last_tp: None,
            tp_stage: 0,
            n_tps: 0,
            created_ts: 0,
            is_limit: false,
            armed: false,
            risk_free: false,
            realized: 0.0,
            floating: 0.0,
            n_open: 1,
            n_pend: 0,
        }
    }

    #[inline]
    pub fn mid(&self) -> Px {
        (self.zone_lo + self.zone_hi) * 0.5
    }
    #[inline]
    pub fn width(&self) -> f64 {
        self.zone_hi - self.zone_lo
    }
}

pub fn basket_features(out: &mut [f32], sc: &Scale, bk: &BasketCtx) {
    debug_assert!(out.len() >= B_DIM);
    let atr = sc.atr;
    let cap = sc.cap;
    let s = bk.side.sign();
    let px = sc.q.mid();
    let w = bk.width().max(atr);
    let age_min = if bk.created_ts > 0 {
        sc.q.ts.saturating_sub(bk.created_ts) as f64 / 60_000.0
    } else {
        0.0
    };

    out[0] = norm(s);
    out[1] = norm((px - bk.mid()) * s / w);
    out[2] = norm((px - bk.mid()) * s / atr);
    match bk.sl {
        Some(v) => {
            out[3] = norm((px - v) * s / atr);
            out[4] = 1.0;
            out[18] = norm((bk.mid() - v).abs() / atr);
        }
        None => {
            out[3] = 0.0;
            out[4] = 0.0;
            out[18] = 0.0;
        }
    }
    match bk.next_tp {
        Some(v) => {
            out[5] = norm((v - px) * s / atr);
            out[6] = 1.0;
        }
        None => {
            out[5] = 0.0;
            out[6] = 0.0;
        }
    }
    out[7] = match bk.last_tp {
        Some(v) => norm((v - bk.mid()) * s / atr),
        None => 0.0,
    };
    out[8] = norm(bk.tp_stage as f64 / bk.n_tps.max(1) as f64);
    out[9] = norm(bk.n_tps as f64 / 5.0);
    out[10] = norm(ln_age(age_min));
    out[11] = norm(bk.n_open as f64 / 5.0);
    out[12] = norm(bk.n_pend as f64 / 5.0);
    out[13] = norm(bk.realized / cap);
    out[14] = norm(bk.floating / cap);
    out[15] = if bk.is_limit { 1.0 } else { 0.0 };
    out[16] = if bk.armed { 1.0 } else { 0.0 };
    out[17] = if bk.risk_free { 1.0 } else { 0.0 };

    // --- GEOMETRIA STREFY: to jest cała ekonomia tego sygnału ---
    //
    // Strefa ma medianę 5 $ szerokości. Wejście przy krawędzi PŁYTKIEJ daje
    // SL ~6 $ i TP1 ~3 $, czyli R:R 0.5. Wejście przy krawędzi GŁĘBOKIEJ daje
    // SL ~1 $ i TP1 ~8 $, czyli R:R 8.0 — szesnastokrotnie lepiej, przy tym
    // samym sygnale. Silnik przy jednej jednostce stawia limit domyślnie na
    // `worse_edge`, czyli w najgorszym możliwym miejscu. Model bez tych cech
    // nie ma jak zobaczyć, że w ogóle jest o co grać.
    let (rr_sh, rr_dp) = match (bk.first_tp, bk.sl) {
        (Some(tp), Some(slv)) => {
            let worse = bk.side.worse_edge(bk.zone_lo, bk.zone_hi);
            let better = bk.side.better_edge(bk.zone_lo, bk.zone_hi);
            let rr = |edge: Px| {
                let reward = (tp - edge) * s;
                let risk = (edge - slv) * s;
                if risk > 1e-6 {
                    reward / risk
                } else {
                    0.0
                }
            };
            (rr(worse), rr(better))
        }
        _ => (0.0, 0.0),
    };
    out[19] = norm(rr_sh);
    out[20] = norm(rr_dp);
    out[21] = norm(bk.width() / atr);
}

// ============================================================
//  KONTEKST POZYCJI
// ============================================================

/// Kopia pozycji bez `String` — żeby przepisanie do cech nie alokowało.
#[derive(Clone, Copy)]
pub struct PosSnap {
    pub ticket: Ticket,
    pub basket: Option<u32>,
    pub side: Side,
    pub volume: f64,
    pub open_price: Px,
    pub open_ts: Ts,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    pub level: i32,
    pub peak_pts: f64,
    pub last_peak_ts: Ts,
    pub is_runner: bool,
    pub is_toucher: bool,
    pub frozen: bool,
}

impl PosSnap {
    pub fn of(p: &Position) -> Self {
        PosSnap {
            ticket: p.ticket,
            basket: p.basket,
            side: p.side,
            volume: p.volume,
            open_price: p.open_price,
            open_ts: p.open_ts,
            sl: p.sl,
            tp: p.tp,
            level: p.level,
            peak_pts: p.peak_pts,
            last_peak_ts: p.last_peak_ts,
            is_runner: p.is_runner,
            is_toucher: p.is_toucher,
            frozen: p.frozen,
        }
    }

    /// Klucz grupowania — sieroty (bez koszyka) lądują na końcu.
    #[inline]
    pub fn basket_key(&self) -> u32 {
        self.basket.unwrap_or(u32::MAX)
    }

    #[inline]
    pub fn profit_pts(&self, q: &Quote) -> f64 {
        (q.exit(self.side) - self.open_price) * self.side.sign()
    }
    #[inline]
    pub fn profit_usd(&self, q: &Quote) -> f64 {
        self.profit_pts(q) * XAU_CONTRACT * self.volume
    }
}

pub fn position_features(out: &mut [f32], sc: &Scale, bk: &BasketCtx, p: &PosSnap) {
    debug_assert!(out.len() >= P_DIM);
    let atr = sc.atr;
    let cap = sc.cap;
    // jednostka odniesienia dla pieniędzy: 1 % kapitału startowego
    let unit = (cap * 0.01).max(0.01);
    let s = p.side.sign();
    let exit = sc.q.exit(p.side);
    let pts = (exit - p.open_price) * s;
    let peak = p.peak_pts.max(pts);
    let lot = sc.base_lot.max(0.01);

    out[0] = norm(pts / atr);
    out[1] = norm(pts * XAU_CONTRACT * p.volume / unit);
    out[2] = norm(peak / atr);
    out[3] = norm((peak - pts) / atr);
    out[4] = norm(if peak > 1e-9 {
        ((peak - pts) / peak).clamp(0.0, 2.0)
    } else {
        0.0
    });
    out[5] = norm(ln_age(sc.q.ts.saturating_sub(p.open_ts) as f64 / 60_000.0));
    out[6] = norm(ln_age(
        sc.q.ts.saturating_sub(p.last_peak_ts) as f64 / 60_000.0,
    ));
    match p.sl {
        Some(v) => {
            out[7] = norm((exit - v) * s / atr);
            out[8] = 1.0;
            out[9] = norm((v - p.open_price) * s / atr);
            out[17] = norm(((p.open_price - v) * s).max(0.0) * XAU_CONTRACT * p.volume / unit);
        }
        None => {
            out[7] = 0.0;
            out[8] = 0.0;
            out[9] = 0.0;
            // brak SL = ryzyko nieograniczone; wskazujemy to jawnie maksymalną wartością
            out[17] = 8.0;
        }
    }
    match p.tp {
        Some(v) => {
            out[10] = norm((v - exit) * s / atr);
            out[11] = 1.0;
        }
        None => {
            out[10] = 0.0;
            out[11] = 0.0;
        }
    }
    out[12] = norm(p.volume / lot / 5.0);
    out[13] = norm(p.level as f64 / 5.0);
    out[14] = if p.is_toucher { 1.0 } else { 0.0 };
    out[15] = if p.is_runner { 1.0 } else { 0.0 };
    // jak głęboko w strefie weszliśmy: 0 = przy gorszej krawędzi, 1 = przy lepszej
    let worse = bk.side.worse_edge(bk.zone_lo, bk.zone_hi);
    out[16] = norm((worse - p.open_price) * s / bk.width().max(atr));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(eq: f64) -> Account {
        Account {
            balance: eq,
            equity: eq,
            margin: 0.0,
            free_margin: eq,
            leverage: 500,
            credit: 0.0,
        }
    }

    #[test]
    fn norm_zabija_nan_i_nieskonczonosc() {
        assert_eq!(norm(f64::NAN), 0.0);
        assert_eq!(norm(f64::INFINITY), 8.0);
        assert_eq!(norm(f64::NEG_INFINITY), -8.0);
        assert_eq!(norm(1e300), 8.0);
        assert_eq!(norm(0.5), 0.5);
    }

    #[test]
    fn okno_rynkowe_liczy_zwroty_i_atr() {
        let mut mw = MarketWindow::new();
        // 200 minut po jednym ticku, cena rośnie o 1.0 na minutę
        for i in 0..200i64 {
            let px = 4000.0 + i as f64;
            mw.on_tick(&Quote {
                ts: i * 60_000,
                bid: px - 0.1,
                ask: px + 0.1,
            });
        }
        assert!((mw.atr() - 1.0).abs() < 0.2, "atr {}", mw.atr());
        assert!((mw.ret(5) - 5.0).abs() < 1e-6, "ret5 {}", mw.ret(5));
        // skala godzinowa: 200 minut to ponad 3 godziny historii
        assert!(mw.ret_h(1) > 0.0, "zwrot godzinowy: {}", mw.ret_h(1));
        assert!((mw.ret(60) - 60.0).abs() < 1e-6);
        // stały przyrost → zerowa zmienność zwrotów
        assert!(mw.vol(30) < 1e-6);
    }

    #[test]
    fn atr_ma_podloge_na_martwym_rynku() {
        let mut mw = MarketWindow::new();
        for i in 0..100i64 {
            mw.on_tick(&Quote {
                ts: i * 60_000,
                bid: 4000.0,
                ask: 4000.2,
            });
        }
        assert!(mw.atr() >= 0.03);
    }

    /// Kluczowy test odporności: SKRAJNE i uszkodzone wejścia nie mogą
    /// wyprodukować NaN-a w obserwacji. Sieć ma tanh w warstwach ukrytych, ale
    /// NaN przeszedłby przez nią bez oporu i zatruł całą decyzję.
    #[test]
    fn obserwacja_nigdy_nie_zawiera_nan() {
        let mut mw = MarketWindow::new();
        // rynek bez historii — najgorszy możliwy przypadek dla ATR i zwrotów
        let pusty = MarketWindow::new();
        mw.on_tick(&Quote {
            ts: 0,
            bid: 1e-6,
            ask: 1e12,
        });

        let dziwne_konta = [
            Account {
                balance: 0.0,
                equity: 0.0,
                margin: 0.0,
                free_margin: 0.0,
                leverage: 0,
                credit: 0.0,
            },
            Account {
                balance: -500.0,
                equity: -500.0,
                margin: 1e9,
                free_margin: -1e9,
                leverage: 1,
                credit: 0.0,
            },
            Account {
                balance: 1e12,
                equity: 1e12,
                margin: 0.0,
                free_margin: 1e12,
                leverage: 500,
                credit: 0.0,
            },
        ];
        let dziwne_ceny = [
            Quote {
                ts: 0,
                bid: 1e-9,
                ask: 1e-9,
            },
            Quote {
                ts: i64::MAX / 4,
                bid: 1e9,
                ask: 1e9,
            },
            Quote {
                ts: -1,
                bid: 2000.0,
                ask: 2000.3,
            },
        ];
        let dziwne_kapitaly = [0.0, -1.0, 1e-9, 1e12];

        let mut g_out = [0.0f32; G_DIM];
        let mut b_out = [0.0f32; B_DIM];
        let mut p_out = [0.0f32; P_DIM];

        for a in &dziwne_konta {
            for q in &dziwne_ceny {
                for &cap in &dziwne_kapitaly {
                    for market in [&mw, &pusty] {
                        let g = GlobalCtx {
                            q,
                            mw: market,
                            acc: a,
                            start_balance: cap,
                            peak_equity: 0.0,
                            max_dd_abs: f64::INFINITY,
                            day_start_equity: f64::NAN,
                            n_pos: usize::MAX,
                            n_pend: 0,
                            buy_lots: f64::NAN,
                            sell_lots: 0.0,
                            base_lot: 0.0,
                            equity_floor: f64::NEG_INFINITY,
                            tz_offset_ms: i64::MIN / 4,
                            loss_streak: u32::MAX,
                            since_win_h: f64::NAN,
                            open_risk: f64::INFINITY,
                        };
                        global_features(&mut g_out, &g);
                        assert!(
                            g_out.iter().all(|v| v.is_finite() && v.abs() <= 8.0),
                            "cechy globalne: {g_out:?}"
                        );

                        let sc = Scale::of(&g);
                        // koszyk o zerowej szerokości, z SL po złej stronie i bez celów
                        let bk = BasketCtx {
                            side: Side::Sell,
                            zone_lo: 0.0,
                            zone_hi: 0.0,
                            sl: Some(f64::NAN),
                            first_tp: Some(f64::NAN),
                            next_tp: Some(f64::INFINITY),
                            last_tp: None,
                            tp_stage: usize::MAX,
                            n_tps: 0,
                            created_ts: i64::MAX,
                            is_limit: true,
                            armed: true,
                            risk_free: false,
                            realized: f64::NAN,
                            floating: f64::INFINITY,
                            n_open: usize::MAX,
                            n_pend: usize::MAX,
                        };
                        basket_features(&mut b_out, &sc, &bk);
                        assert!(
                            b_out.iter().all(|v| v.is_finite() && v.abs() <= 8.0),
                            "cechy koszyka: {b_out:?}"
                        );

                        let p = PosSnap {
                            ticket: 1,
                            basket: None,
                            side: Side::Buy,
                            volume: 0.0,
                            open_price: 0.0,
                            open_ts: i64::MAX,
                            sl: None,
                            tp: Some(f64::NAN),
                            level: i32::MIN,
                            peak_pts: f64::NAN,
                            last_peak_ts: i64::MIN,
                            is_runner: true,
                            is_toucher: false,
                            frozen: false,
                        };
                        position_features(&mut p_out, &sc, &bk, &p);
                        assert!(
                            p_out.iter().all(|v| v.is_finite() && v.abs() <= 8.0),
                            "cechy pozycji: {p_out:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn otwarte_ryzyko_nie_znika_po_zdjeciu_sl() {
        let mk = |sl: Option<Px>| Position {
            ticket: 1,
            side: Side::Buy,
            volume: 0.01,
            open_price: 4000.0,
            open_ts: 0,
            sl,
            tp: None,
            vsl: None,
            basket: None,
            level: 0,
            frozen: false,
            peak_pts: 0.0,
            last_peak_ts: 0,
            is_runner: false,
            is_toucher: false,
            comment: String::new(),
        };
        // SL 10 $ pod wejściem → ryzyko 10 × 100 × 0.01 = 10 $
        assert!((open_risk(&[mk(Some(3990.0))], 0.5, 40.0) - 10.0).abs() < 1e-9);
        // SL POWYŻEJ wejścia blokuje zysk → ryzyka nie ma
        assert_eq!(open_risk(&[mk(Some(4010.0))], 0.5, 40.0), 0.0);
        // brak SL → ryzyko zastępcze, nie zero
        assert!(open_risk(&[mk(None)], 0.5, 40.0) > 0.0);
    }
}
