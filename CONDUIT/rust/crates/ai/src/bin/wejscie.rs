//! WEJŚCIE — oś, której model jeszcze nie dostał, i jedyna, która została.
//!
//! # Dlaczego ten program powstał zamiast kopii BETAZERO
//!
//! Stary model (BETAZERO, `TRADE COPIER/bot/rust_seed/betazero.py`) uczył się
//! ZARZĄDZANIA otwartą pozycją: co 2 s wybierał `add_lot / close_frac / trail /
//! sl`, a jedyną informacją zwrotną był **jeden skalar na cały przebieg**
//! (średni dzienny log-zwrot). Przy 263 wagach i 43 200 krokach decyzyjnych na
//! ścieżkę daje to przypisanie zasługi bliskie zeru — po 30 pokoleniach fitness
//! wynosił 0,062 przy rozrzucie populacji ±0,05.
//!
//! Nam wiadomo dziś dwie rzeczy, których tamten projekt nie wiedział:
//!
//! 1. **W punkcie decyzyjnym informacji o „biegaczu" NIE MA** (AUC 0,50;
//!    `PLAN_AI.md` §2). Oś wyjścia jest więc pusta dla modelu.
//! 2. **W 73 % koszyków cena dochodzi do TP1, zanim głębsze warstwy siatki się
//!    wypełnią.** Reguły wyjścia zarządzają jedną warstwą zamiast trzech.
//!
//! Wniosek: pytanie modelu brzmi nie „kiedy wyjść", tylko **„jak wejść"** —
//! ile warstw i jak głęboko postawić, żeby koszyk w ogóle istniał.
//!
//! # Konstrukcja
//!
//! * **Jedna decyzja na sygnał**, podjęta w chwili `t0` z cech znanych PRZED
//!   jakimkolwiek wypełnieniem. To usuwa artefakt etykietowania po próbkach
//!   u źródła — nie ma jak policzyć 360 próbek dla ścieżki, która biegnie 6 h.
//! * **Pełna informacja zwrotna**: symulator liczy wynik WSZYSTKICH wariantów
//!   wejścia dla każdego sygnału. To nie jest RL — to kontekstowy bandyta
//!   z widocznymi wszystkimi ramionami, więc uczenie jest regresją, nie
//!   ewolucją po jednym skalarze.
//! * **Reguła wyjścia STAŁA** we wszystkich wariantach (wszystko na TP1, SL
//!   sygnału, twardy czas życia). Jedyne, co się zmienia, to geometria wejścia.
//! * **SL liczony ze strefy SUROWEJ**, wspólny dla wszystkich wariantów —
//!   inaczej zmiana geometrii przesuwałaby też stop i nie dałoby się rozdzielić
//!   przyczyn.
//!
//! # Kontrole (bez nich liczby są bezwartościowe)
//!
//! | kontrola | co obala |
//! |---|---|
//! | placebo (`--placebo-h`) | zysk z dryfu próbki zamiast z sygnału |
//! | najlepsza STAŁA akcja | „model wybiera" tam, gdzie wystarczy jedna geometria |
//! | model liniowy (ridge) | przeuczenie sieci |
//! | cechy przetasowane | wyciek przez konstrukcję pipeline'u |
//! | stała ekspozycja | „przewaga", która jest tylko większą dźwignią |
//!
//! ```text
//! cargo run --release -p conduit-ai --bin wejscie -- --zycie-min 60
//! ```

use anyhow::{bail, Result};
use conduit_ai::obs::MarketWindow;
use conduit_backtest::{load_signals, RawSignal, TickData};
use conduit_core::types::*;
use rayon::prelude::*;

// ============================================================
//  AKCJE — geometria wejścia
// ============================================================

/// Wariant wejścia: ile warstw i jak rozciągnięta strefa.
///
/// `deep` pogłębia krawędź LEPSZĄ (dla kupna: w dół), `tol` przesuwa krawędź
/// GORSZĄ (dla kupna: w górę; wartość ujemna zwęża strefę). Silnikowy czempion
/// to `(3, 3.0, -2.0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Akcja {
    jedn: usize,
    deep: f64,
    tol: f64,
}

impl Akcja {
    fn nazwa(&self) -> String {
        format!("{}w/gl{:.1}/tol{:+.0}", self.jedn, self.deep, self.tol)
    }
}

/// Siatka 4×4: liczba warstw × głębokość. `tol` stały (−2, jak w silniku),
/// żeby siatka miała SĄSIEDZTWO — konfiguracja wygrywająca samotnie,
/// z ujemnymi sąsiadami, to dopasowanie do szumu (PROMPT0 §4).
fn siatka_akcji() -> Vec<Akcja> {
    let mut v = Vec::new();
    for jedn in [1usize, 2, 3, 4] {
        for deep in [0.0f64, 1.5, 3.0, 5.0] {
            v.push(Akcja {
                jedn,
                deep,
                tol: -2.0,
            });
        }
    }
    v
}

/// Geometria ODNIESIENIA — dokładnie ta, którą liczy silnik (`KRATA`).
/// Od niej liczona jest podłoga SL, wspólna dla wszystkich wariantów.
const GEOM_ODN: Akcja = Akcja {
    jedn: 3,
    deep: 3.0,
    tol: -2.0,
};

/// Indeks akcji odpowiadającej geometrii silnika (`KRATA`/czempion).
fn idx_czempiona(akcje: &[Akcja]) -> usize {
    akcje
        .iter()
        .position(|a| a.jedn == 3 && (a.deep - 3.0).abs() < 1e-9 && (a.tol + 2.0).abs() < 1e-9)
        .unwrap_or(0)
}

/// Poziomy siatki. Poziom 0 jest NAJGŁĘBSZY (najlepsza cena).
fn poziomy(side: Side, lo: f64, hi: f64, a: &Akcja) -> Vec<f64> {
    let (mut zlo, mut zhi) = (lo, hi);
    match side {
        Side::Buy => {
            zlo -= a.deep;
            zhi += a.tol;
        }
        Side::Sell => {
            zhi += a.deep;
            zlo -= a.tol;
        }
    }
    let (zlo, zhi) = (zlo.min(zhi), zlo.max(zhi));
    let u = a.jedn.max(1);
    if u == 1 {
        // jedna warstwa staje na krawędzi GORSZEJ — czyli wypełnia się jako
        // pierwsza. To jest wariant „wejdź od razu", nie „czekaj na cofnięcie".
        return vec![side.worse_edge(zlo, zhi)];
    }
    (0..u)
        .map(|i| {
            let f = i as f64 / (u - 1) as f64;
            match side {
                Side::Buy => zlo + f * (zhi - zlo),
                Side::Sell => zhi - f * (zhi - zlo),
            }
        })
        .collect()
}

// ============================================================
//  WYNIK JEDNEGO KOSZYKA
// ============================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Powod {
    Brak,
    Tp1,
    Sl,
    Czas,
    Horyzont,
}

#[derive(Clone)]
struct Wynik {
    /// czy koszyk w ogóle się wypełnił (inaczej wkład = 0 $, ale sygnał istnieje)
    wypelnil: bool,
    pnl: f64,
    dzien: i64,
    ts_otw: Ts,
    ts_zam: Ts,
    warstw: u8,
    /// ile warstw było otwartych w chwili dotknięcia TP1 (0 = TP1 nie padł)
    warstw_w_tp1: u8,
    powod: Powod,
    /// wycena mark-to-market co `krok_s` — wyłącznie do min. equity
    marks: Vec<(Ts, f32)>,
}

impl Default for Wynik {
    fn default() -> Self {
        Wynik {
            wypelnil: false,
            pnl: 0.0,
            dzien: 0,
            ts_otw: 0,
            ts_zam: 0,
            warstw: 0,
            warstw_w_tp1: 0,
            powod: Powod::Brak,
            marks: Vec::new(),
        }
    }
}

/// Cechy znane w chwili `t0`, PRZED jakimkolwiek wypełnieniem.
const F: usize = 28;

const NAZWY: [&str; F] = [
    "kierunek",    // +1 kupno / −1 sprzedaż
    "szer_strefy", // (hi−lo) / ATR
    "dyst_tp1",    // (TP1 − środek strefy)·sgn / ATR
    "dyst_sl",     // (środek strefy − SL)·sgn / ATR
    "rr",          // dyst_tp1 / dyst_sl
    "liczba_celow",
    "limit", // sygnał ze zleceniami oczekującymi
    "tag_high_risk",
    "tag_may_not",
    "strefa_od_ceny", // (środek strefy − cena)·sgn / ATR  (ujemne = trzeba cofnięcia)
    "krawedz_od_ceny", // (krawędź gorsza − cena)·sgn / ATR
    "atr_wzgl",
    "zmiennosc_5m",
    "zmiennosc_60m",
    "ret_1m",
    "ret_5m",
    "ret_15m",
    "ret_60m",
    "spread",
    "spread_do_mediany",
    "godz_sin",
    "godz_cos",
    "aktywnosc_24h",
    "od_poprz_sygnalu",
    "od_tp_hit",
    "nr_w_dniu",
    "trend_24h",
    "do_konca_sesji",
];

#[inline]
fn norm(x: f64) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(-8.0, 8.0) as f32
    }
}

#[inline]
fn ln_min(m: f64) -> f64 {
    (1.0 + m.max(0.0)).ln() / 1441.0f64.ln()
}

struct Kontekst {
    tp_hity: Vec<Ts>,
    czasy_syg: Vec<Ts>,
}

// ============================================================
//  SYMULACJA JEDNEGO SYGNAŁU — WSZYSTKIE AKCJE W JEDNYM PRZEBIEGU
// ============================================================

/// Co zrobić z poziomem siatki, który w chwili sygnału jest już przebity.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Przez {
    /// wejście rynkowe po bieżącej cenie — zachowanie `sim.rs` (domyślne)
    Rynek,
    /// zlecenie odrzucone, warstwa nie powstaje — zachowanie `bridge.rs` (10015)
    Pomin,
    Cena,
}

struct Cfg {
    przez: Przez,
    horyzont_h: i64,
    zycie_min: i64,
    krok_s: i64,
    msg_offset_ms: i64,
    rozgrzewka_min: i64,
    sl_min_dist: f64,
    placebo_h: i64,
}

struct Rekord {
    x: Vec<f32>,
    /// wynik każdej akcji; indeksy zgodne z `siatka_akcji()`
    wyn: Vec<Wynik>,
    dzien_syg: i64,
    ts0: Ts,
}

#[allow(clippy::too_many_lines)]
fn jeden_sygnal(
    td: &TickData,
    s: &RawSignal,
    cfg: &Cfg,
    akcje: &[Akcja],
    ctx: &Kontekst,
) -> Option<Rekord> {
    let side = if s.dir.eq_ignore_ascii_case("BUY") {
        Side::Buy
    } else {
        Side::Sell
    };
    let sgn = side.sign();
    let (mut lo, mut hi) = (s.lo.min(s.hi), s.lo.max(s.hi));
    let mut tp1 = *s.tps.first()?;
    let mut sl_syg = s.sl;

    // --- KONTROLA PLACEBO: ta sama geometria, inna chwila ---
    let mut przesuniecie_ms = 0i64;
    if cfg.placebo_h != 0 {
        let t_a = s.ts * 1000 + cfg.msg_offset_ms;
        let t_b = t_a + cfg.placebo_h * 3_600_000;
        let (i_a, i_b) = (td.index_at(t_a), td.index_at(t_b));
        if i_a >= td.len() || i_b >= td.len() {
            return None;
        }
        let delta = td.quote(i_b).mid() - td.quote(i_a).mid();
        lo += delta;
        hi += delta;
        tp1 += delta;
        sl_syg += delta;
        przesuniecie_ms = cfg.placebo_h * 3_600_000;
    }

    let mid_odn = match side {
        Side::Buy => ((lo - GEOM_ODN.deep) + (hi + GEOM_ODN.tol)) * 0.5,
        Side::Sell => ((hi + GEOM_ODN.deep) + (lo - GEOM_ODN.tol)) * 0.5,
    };
    let sl = match side {
        Side::Buy => sl_syg.min(mid_odn - cfg.sl_min_dist),
        Side::Sell => sl_syg.max(mid_odn + cfg.sl_min_dist),
    };
    if (lo - sl) * sgn <= 0.0 && (hi - sl) * sgn <= 0.0 {
        return None;
    }
    if (tp1 - hi) * sgn <= 0.0 && (tp1 - lo) * sgn <= 0.0 {
        return None;
    }

    let t0 = s.ts * 1000 + cfg.msg_offset_ms + przesuniecie_ms;
    let i_sig = td.index_at(t0);
    if i_sig >= td.len() {
        return None;
    }

    // --- rozgrzewka: pamięć rynku przed sygnałem ---
    let mut mw = MarketWindow::new();
    let i_warm = td.index_at(t0 - cfg.rozgrzewka_min * 60_000);
    let mut spread_buf: Vec<f64> = Vec::with_capacity(1024);
    for i in i_warm..i_sig {
        let q = td.quote(i);
        if q.bid > 0.0 && q.ask >= q.bid {
            mw.on_tick(&q);
            spread_buf.push(q.ask - q.bid);
        }
    }
    if spread_buf.len() < 30 {
        return None;
    }
    spread_buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let spread_med = spread_buf[spread_buf.len() / 2];

    let q0 = td.quote(i_sig);
    if !(q0.bid > 0.0 && q0.ask >= q0.bid) {
        return None;
    }
    let atr = mw.atr().max(1e-6);
    let cena0 = q0.mid();
    let godz = hour_of(t0, 0) as f64;
    let ang = godz / 24.0 * std::f64::consts::TAU;
    let mid_zone = (lo + hi) * 0.5;
    let kraw_gorsza = side.worse_edge(lo, hi);
    let d_tp1 = (tp1 - mid_zone) * sgn;
    let d_sl = (mid_zone - sl) * sgn;

    let od_tp = match ctx.tp_hity.partition_point(|t| *t <= t0) {
        0 => 1440.0,
        k => ((t0 - ctx.tp_hity[k - 1]) as f64 / 60_000.0).min(1440.0),
    };
    let od_syg = match ctx.czasy_syg.partition_point(|t| *t < t0) {
        0 => 1440.0,
        k => ((t0 - ctx.czasy_syg[k - 1]) as f64 / 60_000.0).min(1440.0),
    };
    let akt = {
        let a = ctx.czasy_syg.partition_point(|t| *t < t0 - 86_400_000);
        let b = ctx.czasy_syg.partition_point(|t| *t <= t0);
        (b - a) as f64
    };
    let nr_w_dniu = {
        let d0 = t0.div_euclid(86_400_000) * 86_400_000;
        let a = ctx.czasy_syg.partition_point(|t| *t < d0);
        let b = ctx.czasy_syg.partition_point(|t| *t <= t0);
        (b - a) as f64
    };
    // trend rynku: gdzie byliśmy dobę temu
    let trend24 = {
        let i = td.index_at(t0 - 86_400_000);
        if i < td.len() {
            (cena0 - td.quote(i).mid()) / atr
        } else {
            0.0
        }
    };

    let mut x = vec![0.0f32; F];
    x[0] = sgn as f32;
    x[1] = norm((hi - lo) / atr);
    x[2] = norm(d_tp1 / atr);
    x[3] = norm(d_sl / atr);
    x[4] = norm(if d_sl.abs() > 1e-9 { d_tp1 / d_sl } else { 0.0 });
    x[5] = norm(s.tps.len() as f64 / 3.0);
    x[6] = if s.limit { 1.0 } else { 0.0 };
    x[7] = if s.tag_high_risk { 1.0 } else { 0.0 };
    x[8] = if s.tag_may_not { 1.0 } else { 0.0 };
    x[9] = norm((mid_zone - cena0) * sgn / atr);
    x[10] = norm((kraw_gorsza - cena0) * sgn / atr);
    x[11] = norm(atr / cena0.max(1.0) * 1000.0);
    x[12] = norm(mw.vol(5) / atr);
    x[13] = norm(mw.vol(60) / atr);
    x[14] = norm(mw.ret(1) / atr);
    x[15] = norm(mw.ret(5) / atr);
    x[16] = norm(mw.ret(15) / atr);
    x[17] = norm(mw.ret(60) / atr);
    x[18] = norm(q0.spread() / atr);
    x[19] = norm(if spread_med > 1e-9 {
        q0.spread() / spread_med - 1.0
    } else {
        0.0
    });
    x[20] = norm(ang.sin());
    x[21] = norm(ang.cos());
    x[22] = norm((1.0 + akt).ln() / 30.0f64.ln());
    x[23] = norm(ln_min(od_syg));
    x[24] = norm(ln_min(od_tp));
    x[25] = norm(nr_w_dniu / 20.0);
    x[26] = norm(trend24);
    x[27] = norm((18.0 - godz) / 10.0);

    // ==========================================================
    //  SYMULACJA — wszystkie akcje naraz, jeden przebieg po tickach
    // ==========================================================
    let i_end = td.index_at(t0 + cfg.horyzont_h * 3_600_000).min(td.len());
    let krok_ms = cfg.krok_s * 1000;
    let zycie_ms = if cfg.zycie_min > 0 {
        cfg.zycie_min * 60_000
    } else {
        i64::MAX / 4
    };

    let k = akcje.len();
    let poz: Vec<Vec<f64>> = akcje.iter().map(|a| poziomy(side, lo, hi, a)).collect();

    let przebity: Vec<Vec<bool>> = poz
        .iter()
        .map(|p| {
            p.iter()
                .map(|lv| match side {
                    Side::Buy => q0.ask <= *lv,
                    Side::Sell => q0.bid >= *lv,
                })
                .collect()
        })
        .collect();
    let mut czy_wyp: Vec<Vec<bool>> = poz.iter().map(|p| vec![false; p.len()]).collect();
    let mut wej: Vec<Vec<f64>> = vec![Vec::new(); k];
    let mut zamk = vec![false; k];
    let mut wyn: Vec<Wynik> = vec![Wynik::default(); k];
    let mut nast_mark = vec![0i64; k];
    let mut zywych = k;

    for i in i_sig..i_end {
        if zywych == 0 {
            break;
        }
        let q = td.quote(i);
        if !(q.bid > 0.0 && q.ask >= q.bid) {
            continue;
        }
        let cena = q.exit(side);
        let tp1_dotk = match side {
            Side::Buy => q.bid >= tp1,
            Side::Sell => q.ask <= tp1,
        };
        let sl_dotk = match side {
            Side::Buy => q.bid <= sl,
            Side::Sell => q.ask >= sl,
        };

        for a in 0..k {
            if zamk[a] {
                continue;
            }
            // --- wypełnienia limitów (fill po SWOJEJ cenie) ---
            for (lv, p) in poz[a].iter().enumerate() {
                if czy_wyp[a][lv] {
                    continue;
                }
                // poziom przebity już w chwili postawienia — patrz komentarz wyżej
                if przebity[a][lv] {
                    match cfg.przez {
                        Przez::Pomin => {
                            czy_wyp[a][lv] = true; // „wypełniony” = zdjęty z siatki, bez pozycji
                            continue;
                        }
                        Przez::Rynek => {
                            czy_wyp[a][lv] = true;
                            wej[a].push(match side {
                                Side::Buy => q.ask,
                                Side::Sell => q.bid,
                            });
                            if wej[a].len() == 1 {
                                wyn[a].wypelnil = true;
                                wyn[a].ts_otw = q.ts;
                                wyn[a].dzien = q.ts.div_euclid(86_400_000);
                                nast_mark[a] = q.ts;
                            }
                            continue;
                        }
                        Przez::Cena => {}
                    }
                }
                let dotyk = match side {
                    Side::Buy => q.ask <= *p,
                    Side::Sell => q.bid >= *p,
                };
                if dotyk {
                    czy_wyp[a][lv] = true;
                    wej[a].push(*p);
                    if wej[a].len() == 1 {
                        wyn[a].wypelnil = true;
                        wyn[a].ts_otw = q.ts;
                        wyn[a].dzien = q.ts.div_euclid(86_400_000);
                        nast_mark[a] = q.ts;
                    }
                }
            }
            if wej[a].is_empty() {
                continue;
            }
            let wych: f64 = wej[a].iter().map(|e| (cena - e) * sgn).sum();

            // --- mark-to-market ---
            if q.ts >= nast_mark[a] {
                nast_mark[a] = q.ts + krok_ms;
                wyn[a].marks.push((q.ts, wych as f32));
            }

            // --- wyjście: TP1 · SL · twardy czas życia (od PIERWSZEGO wypełnienia) ---
            let po_czasie = q.ts - wyn[a].ts_otw >= zycie_ms;
            let powod = if sl_dotk {
                Powod::Sl
            } else if tp1_dotk {
                Powod::Tp1
            } else if po_czasie {
                Powod::Czas
            } else {
                Powod::Brak
            };
            if powod != Powod::Brak {
                if powod == Powod::Tp1 {
                    wyn[a].warstw_w_tp1 = wej[a].len() as u8;
                }
                wyn[a].pnl = wych;
                wyn[a].ts_zam = q.ts;
                wyn[a].warstw = wej[a].len() as u8;
                wyn[a].powod = powod;
                zamk[a] = true;
                zywych -= 1;
            }
        }
    }

    // --- domknięcie po horyzoncie ---
    let i_last = i_end.saturating_sub(1).min(td.len().saturating_sub(1));
    let q_last = td.quote(i_last);
    let cena_k = q_last.exit(side);
    for a in 0..k {
        if zamk[a] || wej[a].is_empty() {
            continue;
        }
        wyn[a].pnl = wej[a].iter().map(|e| (cena_k - e) * sgn).sum();
        wyn[a].ts_zam = q_last.ts;
        wyn[a].warstw = wej[a].len() as u8;
        wyn[a].powod = Powod::Horyzont;
    }
    // sygnały niewypełnione zostają z pnl = 0 i `wypelnil = false`

    Some(Rekord {
        x,
        wyn,
        dzien_syg: t0.div_euclid(86_400_000),
        ts0: t0,
    })
}

// ============================================================
//  AGREGACJA DZIENNA I MIARY
// ============================================================

struct Miary {
    pnl: f64,
    dni: usize,
    dni_str: usize,
    najgorszy_dzien: f64,
    min_skum: f64,
    min_equity: f64,
    zerowan: usize,
    wypelnionych: usize,
    /// łączna liczba wypełnionych warstw — mianownik przeliczenia „na jednostkę”
    jedn_lacznie: f64,
    sr_warstw: f64,
    tp1_z_1_warstwa: f64,
    udzial_tp1: f64,
}

/// Dzienne sumy wyniku — podstawa każdego bootstrapu.
fn dni_z(wyniki: &[(i64, f64)]) -> Vec<(i64, f64)> {
    let mut m: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for (d, v) in wyniki {
        *m.entry(*d).or_insert(0.0) += v;
    }
    m.into_iter().collect()
}

fn miary(rek: &[Rekord], a: usize, kapital: f64) -> Miary {
    let mut pary: Vec<(i64, f64)> = Vec::with_capacity(rek.len());
    let mut wyp = 0usize;
    let mut sw = 0.0f64;
    let mut tp1_1w = 0usize;
    let mut tp1_all = 0usize;
    // zdarzenia mark-to-market do equity portfela
    let mut zd: Vec<(Ts, u8, usize, f64)> = Vec::with_capacity(rek.len() * 24);
    for (i, r) in rek.iter().enumerate() {
        let w = &r.wyn[a];
        // dzień księgowany po chwili OTWARCIA koszyka (niewypełnione → dzień sygnału)
        let d = if w.wypelnil { w.dzien } else { r.dzien_syg };
        pary.push((d, w.pnl));
        if w.wypelnil {
            wyp += 1;
            sw += w.warstw as f64;
            if w.warstw_w_tp1 > 0 {
                tp1_all += 1;
                if w.warstw_w_tp1 == 1 {
                    tp1_1w += 1;
                }
            }
            for (t, v) in &w.marks {
                zd.push((*t, 0, i, *v as f64));
            }
            zd.push((w.ts_zam, 1, i, w.pnl));
        }
    }
    zd.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut otw: std::collections::HashMap<usize, f64> = std::collections::HashMap::new();
    let (mut suma_otw, mut zreal, mut min_eq) = (0.0f64, 0.0f64, 0.0f64);
    let mut zerowan = 0usize;
    let mut pod_woda = false;
    for (_, typ, i, v) in zd {
        if typ == 0 {
            let stary = otw.insert(i, v).unwrap_or(0.0);
            suma_otw += v - stary;
        } else {
            if let Some(stary) = otw.remove(&i) {
                suma_otw -= stary;
            }
            zreal += v;
        }
        let eq = zreal + suma_otw;
        min_eq = min_eq.min(eq);
        if eq <= -kapital {
            if !pod_woda {
                zerowan += 1;
                pod_woda = true;
            }
        } else {
            pod_woda = false;
        }
    }

    let dni = dni_z(&pary);
    let (mut e, mut mins) = (0.0f64, 0.0f64);
    let mut najg = 0.0f64;
    let mut dni_str = 0usize;
    for (_, v) in &dni {
        e += v;
        mins = mins.min(e);
        najg = najg.min(*v);
        if *v < 0.0 {
            dni_str += 1;
        }
    }
    Miary {
        pnl: pary.iter().map(|p| p.1).sum(),
        dni: dni.len(),
        dni_str,
        najgorszy_dzien: najg,
        min_skum: mins,
        min_equity: min_eq,
        zerowan,
        wypelnionych: wyp,
        jedn_lacznie: sw,
        sr_warstw: if wyp > 0 { sw / wyp as f64 } else { 0.0 },
        tp1_z_1_warstwa: if tp1_all > 0 {
            tp1_1w as f64 / tp1_all as f64 * 100.0
        } else {
            0.0
        },
        udzial_tp1: if wyp > 0 {
            tp1_all as f64 / wyp as f64 * 100.0
        } else {
            0.0
        },
    }
}

/// Bootstrap PO DNIACH na RÓŻNICY dwóch szeregów. Zwraca `(dolny, górny)` 5–95 %.
fn bootstrap_roznicy(a: &[(i64, f64)], b: &[(i64, f64)], losowan: usize, seed: u64) -> (f64, f64) {
    // BŁĄD, KTÓRY TU BYŁ: `a.iter().cloned().collect()` do mapy zostawia OSTATNI
    // koszyk dnia zamiast sumy wszystkich. Przedział ufności wychodził wtedy
    // rozłączny z punktową różnicą — i to była jedyna oznaka pomyłki.
    let ma: std::collections::BTreeMap<i64, f64> = dni_z(a).into_iter().collect();
    let mb: std::collections::BTreeMap<i64, f64> = dni_z(b).into_iter().collect();
    let mut dni: Vec<i64> = ma.keys().chain(mb.keys()).cloned().collect();
    dni.sort_unstable();
    dni.dedup();
    let d: Vec<f64> = dni
        .iter()
        .map(|k| ma.get(k).copied().unwrap_or(0.0) - mb.get(k).copied().unwrap_or(0.0))
        .collect();
    if d.is_empty() {
        return (0.0, 0.0);
    }
    let mut st = seed | 1;
    let mut next = || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        st
    };
    let mut sumy: Vec<f64> = Vec::with_capacity(losowan);
    for _ in 0..losowan {
        let mut s = 0.0;
        for _ in 0..d.len() {
            s += d[(next() as usize) % d.len()];
        }
        sumy.push(s);
    }
    sumy.sort_by(|x, y| x.partial_cmp(y).unwrap());
    (sumy[losowan / 20], sumy[losowan * 19 / 20])
}

// ============================================================
//  MODELE
// ============================================================

/// Regresja grzbietowa, rozwiązanie normalne z eliminacją Gaussa.
struct Ridge {
    w: Vec<f64>,
    b: f64,
}

impl Ridge {
    fn ucz(x: &[&[f32]], y: &[f64], alpha: f64) -> Ridge {
        let n = x.len();
        let f = if n > 0 { x[0].len() } else { 0 };
        if n == 0 || f == 0 {
            return Ridge {
                w: vec![0.0; f],
                b: 0.0,
            };
        }
        let sr_y: f64 = y.iter().sum::<f64>() / n as f64;
        let mut sr_x = vec![0.0f64; f];
        for r in x {
            for j in 0..f {
                sr_x[j] += r[j] as f64;
            }
        }
        for v in &mut sr_x {
            *v /= n as f64;
        }
        let mut a = vec![0.0f64; f * f];
        let mut bv = vec![0.0f64; f];
        for (i, r) in x.iter().enumerate() {
            let dy = y[i] - sr_y;
            for j in 0..f {
                let xj = r[j] as f64 - sr_x[j];
                bv[j] += xj * dy;
                for k in j..f {
                    a[j * f + k] += xj * (r[k] as f64 - sr_x[k]);
                }
            }
        }
        for j in 0..f {
            for k in 0..j {
                a[j * f + k] = a[k * f + j];
            }
            a[j * f + j] += alpha;
        }
        // eliminacja Gaussa z częściowym wyborem elementu głównego
        for c in 0..f {
            let mut p = c;
            for r in c + 1..f {
                if a[r * f + c].abs() > a[p * f + c].abs() {
                    p = r;
                }
            }
            if a[p * f + c].abs() < 1e-12 {
                continue;
            }
            if p != c {
                for k in 0..f {
                    a.swap(c * f + k, p * f + k);
                }
                bv.swap(c, p);
            }
            let d = a[c * f + c];
            for k in c..f {
                a[c * f + k] /= d;
            }
            bv[c] /= d;
            for r in 0..f {
                if r == c {
                    continue;
                }
                let m = a[r * f + c];
                if m.abs() < 1e-15 {
                    continue;
                }
                for k in c..f {
                    a[r * f + k] -= m * a[c * f + k];
                }
                bv[r] -= m * bv[c];
            }
        }
        let w = bv;
        let b = sr_y - w.iter().zip(&sr_x).map(|(a, b)| a * b).sum::<f64>();
        Ridge { w, b }
    }
    fn pred(&self, x: &[f32]) -> f64 {
        self.b
            + self
                .w
                .iter()
                .zip(x)
                .map(|(w, v)| w * *v as f64)
                .sum::<f64>()
    }
}

/// Mała sieć F → H → K, wspólny trzon, K wyjść regresyjnych. Adam + L2.
struct Mlp {
    w1: Vec<f64>,
    b1: Vec<f64>,
    w2: Vec<f64>,
    b2: Vec<f64>,
    h: usize,
    k: usize,
    f: usize,
}

impl Mlp {
    fn nowa(f: usize, h: usize, k: usize, seed: u64) -> Mlp {
        let mut st = seed | 1;
        let mut r = || {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            ((st >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 2.0
        };
        let s1 = (1.0 / f as f64).sqrt();
        let s2 = (1.0 / h as f64).sqrt();
        Mlp {
            w1: (0..f * h).map(|_| r() * s1).collect(),
            b1: vec![0.0; h],
            w2: (0..h * k).map(|_| r() * s2).collect(),
            b2: vec![0.0; k],
            h,
            k,
            f,
        }
    }
    fn przod(&self, x: &[f32], hb: &mut [f64], out: &mut [f64]) {
        for j in 0..self.h {
            let mut a = self.b1[j];
            for i in 0..self.f {
                a += x[i] as f64 * self.w1[i * self.h + j];
            }
            hb[j] = a.tanh();
        }
        for c in 0..self.k {
            let mut a = self.b2[c];
            for j in 0..self.h {
                a += hb[j] * self.w2[j * self.k + c];
            }
            out[c] = a;
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn ucz(&mut self, x: &[&[f32]], y: &[Vec<f64>], epok: usize, lr: f64, l2: f64) {
        let n = x.len();
        if n == 0 {
            return;
        }
        let (mut m1, mut v1) = (vec![0.0; self.w1.len()], vec![0.0; self.w1.len()]);
        let (mut mb1, mut vb1) = (vec![0.0; self.h], vec![0.0; self.h]);
        let (mut m2, mut v2) = (vec![0.0; self.w2.len()], vec![0.0; self.w2.len()]);
        let (mut mb2, mut vb2) = (vec![0.0; self.k], vec![0.0; self.k]);
        let mut hb = vec![0.0; self.h];
        let mut out = vec![0.0; self.k];
        let mut t = 0.0f64;
        for _ in 0..epok {
            let mut g1 = vec![0.0f64; self.w1.len()];
            let mut gb1 = vec![0.0f64; self.h];
            let mut g2 = vec![0.0f64; self.w2.len()];
            let mut gb2 = vec![0.0f64; self.k];
            for (i, xr) in x.iter().enumerate() {
                self.przod(xr, &mut hb, &mut out);
                let mut dh = vec![0.0f64; self.h];
                for c in 0..self.k {
                    let e = 2.0 * (out[c] - y[i][c]) / n as f64;
                    gb2[c] += e;
                    for j in 0..self.h {
                        g2[j * self.k + c] += e * hb[j];
                        dh[j] += e * self.w2[j * self.k + c];
                    }
                }
                for j in 0..self.h {
                    let d = dh[j] * (1.0 - hb[j] * hb[j]);
                    gb1[j] += d;
                    for ii in 0..self.f {
                        g1[ii * self.h + j] += d * xr[ii] as f64;
                    }
                }
            }
            t += 1.0;
            let bc1 = 1.0 - 0.9f64.powf(t);
            let bc2 = 1.0 - 0.999f64.powf(t);
            let mut krok =
                |w: &mut Vec<f64>, g: &Vec<f64>, m: &mut Vec<f64>, v: &mut Vec<f64>, reg: bool| {
                    for i in 0..w.len() {
                        let gi = g[i] + if reg { l2 * w[i] } else { 0.0 };
                        m[i] = 0.9 * m[i] + 0.1 * gi;
                        v[i] = 0.999 * v[i] + 0.001 * gi * gi;
                        w[i] -= lr * (m[i] / bc1) / ((v[i] / bc2).sqrt() + 1e-8);
                    }
                };
            krok(&mut self.w1, &g1, &mut m1, &mut v1, true);
            krok(&mut self.b1, &gb1, &mut mb1, &mut vb1, false);
            krok(&mut self.w2, &g2, &mut m2, &mut v2, true);
            krok(&mut self.b2, &gb2, &mut mb2, &mut vb2, false);
        }
    }
    fn pred(&self, x: &[f32]) -> Vec<f64> {
        let mut hb = vec![0.0; self.h];
        let mut out = vec![0.0; self.k];
        self.przod(x, &mut hb, &mut out);
        out
    }
}

// ============================================================
//  WALK-FORWARD
// ============================================================

#[derive(Clone, Copy, PartialEq)]
enum Model {
    Liniowy,
    Siec,
    Losowy,
    Tasowane,
}

/// Polityka modelu w trybie walk-forward: co `co_dni` przeuczenie na CAŁEJ
/// historii wcześniejszej, decyzja wyłącznie na dniach późniejszych.
///
/// `dozwolone` ogranicza zbiór akcji — kontrola „stała ekspozycja" podaje tu
/// wyłącznie warianty o tej samej liczbie warstw.
#[allow(clippy::too_many_arguments)]
fn walk_forward(
    rek: &[Rekord],
    dozwolone: &[usize],
    model: Model,
    min_hist_dni: i64,
    co_dni: i64,
    seed: u64,
) -> Vec<(i64, f64)> {
    let mut idx: Vec<usize> = (0..rek.len()).collect();
    idx.sort_by_key(|i| rek[*i].ts0);
    let d0 = rek[idx[0]].dzien_syg;
    let dk = rek[*idx.last().unwrap()].dzien_syg;
    let mut out: Vec<(i64, f64)> = Vec::new();
    let mut st = seed | 1;
    let mut los = || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        st
    };

    let mut d = d0 + min_hist_dni;
    while d <= dk {
        let kres = d + co_dni;
        let ucz: Vec<usize> = idx
            .iter()
            .cloned()
            .filter(|i| rek[*i].dzien_syg < d)
            .collect();
        let test: Vec<usize> = idx
            .iter()
            .cloned()
            .filter(|i| rek[*i].dzien_syg >= d && rek[*i].dzien_syg < kres)
            .collect();
        if test.is_empty() {
            d = kres;
            continue;
        }
        // wybór akcji dla każdego sygnału testowego
        let wybor: Vec<usize> = match model {
            Model::Losowy => test
                .iter()
                .map(|_| dozwolone[(los() as usize) % dozwolone.len()])
                .collect(),
            Model::Liniowy | Model::Tasowane => {
                // cechy uczące; w wariancie „tasowane" etykiety idą do losowych wierszy
                let mut kolej: Vec<usize> = (0..ucz.len()).collect();
                if model == Model::Tasowane {
                    for i in (1..kolej.len()).rev() {
                        let j = (los() as usize) % (i + 1);
                        kolej.swap(i, j);
                    }
                }
                let xs: Vec<&[f32]> = ucz.iter().map(|i| rek[*i].x.as_slice()).collect();
                let modele: Vec<Ridge> = dozwolone
                    .iter()
                    .map(|a| {
                        let ys: Vec<f64> = kolej.iter().map(|p| rek[ucz[*p]].wyn[*a].pnl).collect();
                        Ridge::ucz(&xs, &ys, 30.0)
                    })
                    .collect();
                test.iter()
                    .map(|i| {
                        let mut naj = 0usize;
                        let mut best = f64::NEG_INFINITY;
                        for (p, m) in modele.iter().enumerate() {
                            let v = m.pred(&rek[*i].x);
                            if v > best {
                                best = v;
                                naj = p;
                            }
                        }
                        dozwolone[naj]
                    })
                    .collect()
            }
            Model::Siec => {
                let xs: Vec<&[f32]> = ucz.iter().map(|i| rek[*i].x.as_slice()).collect();
                let ys: Vec<Vec<f64>> = ucz
                    .iter()
                    .map(|i| dozwolone.iter().map(|a| rek[*i].wyn[*a].pnl).collect())
                    .collect();
                let mut m = Mlp::nowa(F, 12, dozwolone.len(), seed);
                m.ucz(&xs, &ys, 300, 0.02, 1e-3);
                test.iter()
                    .map(|i| {
                        let p = m.pred(&rek[*i].x);
                        let mut naj = 0usize;
                        for c in 1..p.len() {
                            if p[c] > p[naj] {
                                naj = c;
                            }
                        }
                        dozwolone[naj]
                    })
                    .collect()
            }
        };
        for (t, a) in test.iter().zip(&wybor) {
            let w = &rek[*t].wyn[*a];
            let dz = if w.wypelnil {
                w.dzien
            } else {
                rek[*t].dzien_syg
            };
            out.push((dz, w.pnl));
        }
        d = kres;
    }
    out
}

fn stala_na_oknie(rek: &[Rekord], a: usize, min_hist_dni: i64) -> Vec<(i64, f64)> {
    let mut idx: Vec<usize> = (0..rek.len()).collect();
    idx.sort_by_key(|i| rek[*i].ts0);
    let d0 = rek[idx[0]].dzien_syg + min_hist_dni;
    idx.iter()
        .filter(|i| rek[**i].dzien_syg >= d0)
        .map(|i| {
            let w = &rek[*i].wyn[a];
            (
                if w.wypelnil {
                    w.dzien
                } else {
                    rek[*i].dzien_syg
                },
                w.pnl,
            )
        })
        .collect()
}

fn podsumuj(par: &[(i64, f64)]) -> (f64, usize, usize, f64, f64) {
    let dni = dni_z(par);
    let (mut e, mut mins, mut najg) = (0.0f64, 0.0f64, 0.0f64);
    let mut str_ = 0usize;
    for (_, v) in &dni {
        e += v;
        mins = mins.min(e);
        najg = najg.min(*v);
        if *v < 0.0 {
            str_ += 1;
        }
    }
    (e, str_, dni.len(), najg, mins)
}

// ============================================================
//  MAIN
// ============================================================

struct Args {
    ticks: String,
    signals: String,
    zycie_min: i64,
    horyzont_h: i64,
    placebo_h: i64,
    od: Option<String>,
    kapital: f64,
    min_hist: i64,
    co_dni: i64,
    przez: String,
}

fn parse() -> Result<Args> {
    let mut a = Args {
        ticks: "data/ticks.bin".into(),
        signals: "data/signals.json".into(),
        zycie_min: 60,
        horyzont_h: 6,
        placebo_h: 0,
        od: None,
        kapital: 200.0,
        min_hist: 45,
        co_dni: 7,
        przez: "rynek".into(),
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < v.len() {
        let k = v[i].clone();
        macro_rules! nast {
            () => {{
                i += 1;
                v.get(i)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("brak wartości dla {}", k))?
            }};
        }
        match k.as_str() {
            "--ticks" => a.ticks = nast!(),
            "--signals" => a.signals = nast!(),
            "--zycie-min" => a.zycie_min = nast!().parse()?,
            "--horyzont-h" => a.horyzont_h = nast!().parse()?,
            "--placebo-h" => a.placebo_h = nast!().parse()?,
            "--od" => a.od = Some(nast!()),
            "--kapital" => a.kapital = nast!().parse()?,
            "--min-hist" => a.min_hist = nast!().parse()?,
            "--co-dni" => a.co_dni = nast!().parse()?,
            "--przez" => a.przez = nast!(),
            other => bail!("nieznany argument: {other}"),
        }
        i += 1;
    }
    Ok(a)
}

fn dzien_z_daty(s: &str) -> Result<i64> {
    use chrono::NaiveDate;
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")?;
    let ep = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    Ok((d - ep).num_days())
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    let a = parse()?;
    let td = TickData::open(&a.ticks)?;
    let syg = load_signals(&a.signals)?;
    let akcje = siatka_akcji();
    let przez = match a.przez.as_str() {
        "rynek" => Przez::Rynek,
        "pomin" => Przez::Pomin,
        "cena" => Przez::Cena,
        o => bail!("--przez: {o} (dozwolone: rynek | pomin | cena)"),
    };
    let cfg = Cfg {
        przez,
        horyzont_h: a.horyzont_h,
        zycie_min: a.zycie_min,
        krok_s: 60,
        msg_offset_ms: 180 * 60_000,
        rozgrzewka_min: 120,
        sl_min_dist: 3.0,
        placebo_h: a.placebo_h,
    };

    let ctx = {
        let mut czasy: Vec<Ts> = syg
            .iter()
            .map(|s| s.ts * 1000 + cfg.msg_offset_ms)
            .collect();
        czasy.sort_unstable();
        let tp = conduit_ai::peak::znaczniki_tp_hit(&syg, cfg.msg_offset_ms);
        Kontekst {
            tp_hity: tp,
            czasy_syg: czasy,
        }
    };

    println!(
        "dane: {} ticków · {} sygnałów · {} wariantów wejścia",
        td.len(),
        syg.len(),
        akcje.len()
    );
    let t0 = std::time::Instant::now();
    let mut rek: Vec<Rekord> = syg
        .par_iter()
        .filter_map(|s| jeden_sygnal(&td, s, &cfg, &akcje, &ctx))
        .collect();
    rek.sort_by_key(|r| r.ts0);
    if let Some(od) = &a.od {
        let d = dzien_z_daty(od)?;
        rek.retain(|r| r.dzien_syg >= d);
    }
    println!(
        "przebite limity: {} · policzone: {} sygnałów w {:.1} s · życie koszyka {} min · horyzont {} h{}",
        a.przez,
        rek.len(),
        t0.elapsed().as_secs_f64(),
        if a.zycie_min > 0 { a.zycie_min.to_string() } else { "bez limitu".into() },
        a.horyzont_h,
        if a.placebo_h != 0 { format!(" · PLACEBO +{} h", a.placebo_h) } else { String::new() }
    );
    if rek.len() < 200 {
        bail!("za mało sygnałów");
    }

    // ==========================================================
    //  1. DIAGNOZA: CZY GŁĘBSZE WARSTWY W OGÓLE ZDĄŻAJĄ SIĘ WYPEŁNIĆ
    // ==========================================================
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║ 1. CZY SIATKA W OGÓLE POWSTAJE — warstwy w chwili TP1");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!(
        "  {:<16} {:>9} {:>9} {:>10} {:>12} {:>14}",
        "wariant", "wypeł.", "śr.warstw", "TP1 padł", "TP1 przy 1w", "PnL $"
    );
    for (i, ak) in akcje.iter().enumerate() {
        let m = miary(&rek, i, a.kapital);
        println!(
            "  {:<16} {:>9} {:>9.2} {:>9.0} % {:>11.0} % {:>+14.2}",
            ak.nazwa(),
            m.wypelnionych,
            m.sr_warstw,
            m.udzial_tp1,
            m.tp1_z_1_warstwa,
            m.pnl
        );
    }
    println!(
        "\n  ROZBICIE PO POWODZIE WYJŚCIA — bez tego nie wiadomo, czy wynik robi\n\
         \x20 stop, cel czy zegar.\n"
    );
    println!(
        "  {:<16} {:>17} {:>17} {:>17} {:>15} {:>17}",
        "wariant", "TP1  n / $", "SL  n / $", "czas  n / $", "horyz. n / $", "<2 min  n / $"
    );
    for (i, ak) in akcje.iter().enumerate() {
        let mut n = [0usize; 4];
        let mut s = [0.0f64; 4];
        // Kontrola generatora `peak.rs`: odrzuca on ścieżki krótsze niż 3 próbki
        // (czyli 2 minuty życia koszyka). Jeżeli te koszyki niosą duży ujemny
        // wynik, każda liczba policzona tamtym generatorem jest o nie zawyżona.
        let (mut n_kr, mut s_kr) = (0usize, 0.0f64);
        for r in &rek {
            let w = &r.wyn[i];
            let k = match w.powod {
                Powod::Tp1 => 0,
                Powod::Sl => 1,
                Powod::Czas => 2,
                Powod::Horyzont => 3,
                Powod::Brak => continue,
            };
            n[k] += 1;
            s[k] += w.pnl;
            if w.ts_zam - w.ts_otw < 120_000 {
                n_kr += 1;
                s_kr += w.pnl;
            }
        }
        println!(
            "  {:<16} {:>17} {:>17} {:>17} {:>15} {:>17}",
            ak.nazwa(),
            format!("{} / {:+.0}", n[0], s[0]),
            format!("{} / {:+.0}", n[1], s[1]),
            format!("{} / {:+.0}", n[2], s[2]),
            format!("{} / {:+.0}", n[3], s[3]),
            format!("{n_kr} / {s_kr:+.0}")
        );
    }

    // ==========================================================
    //  1b. REPLIKA FILTRU `peak.rs` — ile jest warta jego selekcja przeżycia
    // ==========================================================
    //
    // `peak.rs` odrzuca ścieżki krótsze niż 3 próbki (2 minuty). Jego pętla NIE
    // przerywa się na TP1 — tylko na SL — więc filtr wycina wyłącznie koszyki
    // GINĄCE NA STOPIE w pierwszych dwóch minutach, a szybkich wygranych nie
    // rusza. To jest selekcja przeżycia i podnosi każdą liczbę policzoną tamtym
    // generatorem.
    println!("\n\n╔══════════════════════════════════════════════════════════════╗");
    println!("║ 1b. ILE JEST WART FILTR `peak.rs` — koszyki ginące na SL w < 2 min");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!(
        "  {:<16} {:>12} {:>14} {:>16} {:>14}",
        "wariant", "odrzuca n", "ich wynik $", "PnL z nimi $", "PnL bez nich $"
    );
    for (i, ak) in akcje.iter().enumerate() {
        let (mut n, mut s) = (0usize, 0.0f64);
        for r in &rek {
            let w = &r.wyn[i];
            if w.powod == Powod::Sl && w.ts_zam - w.ts_otw < 120_000 {
                n += 1;
                s += w.pnl;
            }
        }
        let p = miary(&rek, i, a.kapital).pnl;
        println!(
            "  {:<16} {:>12} {:>+14.2} {:>+16.2} {:>+14.2}",
            ak.nazwa(),
            n,
            s,
            p,
            p - s
        );
    }

    // ==========================================================
    //  2. SIATKA WYNIKÓW — płaskowyż czy szpilka
    // ==========================================================
    println!("\n\n╔══════════════════════════════════════════════════════════════╗");
    println!("║ 2. SIATKA WYNIKÓW — PnL $ (wiersz = warstwy, kolumna = głębokość)");
    println!("╚══════════════════════════════════════════════════════════════╝");
    print!("  {:<10}", "warstwy\\gł");
    for deep in [0.0f64, 1.5, 3.0, 5.0] {
        print!("{:>12}", format!("{deep:.1} $"));
    }
    println!();
    for jedn in [1usize, 2, 3, 4] {
        print!("  {jedn:<10}");
        for deep in [0.0f64, 1.5, 3.0, 5.0] {
            let i = akcje
                .iter()
                .position(|x| x.jedn == jedn && (x.deep - deep).abs() < 1e-9)
                .unwrap();
            print!("{:>+12.2}", miary(&rek, i, a.kapital).pnl);
        }
        println!();
    }
    // Ten sam rachunek PODZIELONY PRZEZ EKSPOZYCJĘ. Jeżeli obie siatki są
    // płaskie, to znaczy, że geometria wejścia jest mnożnikiem ekspozycji,
    // a nie pokrętłem przewagi — i cała oś wejścia jest pusta dla modelu.
    println!("\n  TO SAMO NA JEDNĄ WYPEŁNIONĄ WARSTWĘ ($ / jednostkę):");
    print!("  {:<10}", "warstwy\\gł");
    for deep in [0.0f64, 1.5, 3.0, 5.0] {
        print!("{:>12}", format!("{deep:.1} $"));
    }
    println!();
    for jedn in [1usize, 2, 3, 4] {
        print!("  {jedn:<10}");
        for deep in [0.0f64, 1.5, 3.0, 5.0] {
            let i = akcje
                .iter()
                .position(|x| x.jedn == jedn && (x.deep - deep).abs() < 1e-9)
                .unwrap();
            let m = miary(&rek, i, a.kapital);
            print!("{:>+12.4}", m.pnl / m.jedn_lacznie.max(1.0));
        }
        println!();
    }

    // ==========================================================
    //  3. PEŁNA OCENA KAŻDEGO WARIANTU
    // ==========================================================
    println!("\n\n╔══════════════════════════════════════════════════════════════╗");
    println!("║ 3. OCENA WARIANTÓW — straty, dni stratne, zbliżenie do zera");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!(
        "  {:<16} {:>11} {:>9} {:>13} {:>12} {:>12} {:>9}",
        "wariant", "PnL $", "dni str.", "najg. dzień", "min krzywej", "min equity", "do zera"
    );
    let mut ranking: Vec<(usize, f64)> = Vec::new();
    for (i, ak) in akcje.iter().enumerate() {
        let m = miary(&rek, i, a.kapital);
        ranking.push((i, m.pnl));
        println!(
            "  {:<16} {:>+11.2} {:>8.1} % {:>+13.2} {:>12.2} {:>12.2} {:>9}",
            ak.nazwa(),
            m.pnl,
            m.dni_str as f64 / m.dni.max(1) as f64 * 100.0,
            m.najgorszy_dzien,
            m.min_skum,
            m.min_equity,
            if m.zerowan > 0 {
                format!("TAK x{}", m.zerowan)
            } else {
                "nie".into()
            }
        );
    }
    ranking.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap());
    let naj_stala = ranking[0].0;
    let czemp = idx_czempiona(&akcje);
    println!(
        "\n  NAJLEPSZA STAŁA: {} ({:+.2} $) · geometria silnika: {} ({:+.2} $)",
        akcje[naj_stala].nazwa(),
        ranking[0].1,
        akcje[czemp].nazwa(),
        miary(&rek, czemp, a.kapital).pnl
    );

    // ==========================================================
    //  4. MODEL vs NAJLEPSZA STAŁA — walk-forward
    // ==========================================================
    println!("\n\n╔══════════════════════════════════════════════════════════════╗");
    println!("║ 4. MODEL WYBIERAJĄCY WEJŚCIE vs NAJLEPSZA STAŁA AKCJA");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!(
        "  Walk-forward: przeuczenie co {} dni na CAŁEJ historii wcześniejszej,\n\
         \x20 decyzja wyłącznie na dniach późniejszych. Pierwsze {} dni to rozbieg.",
        a.co_dni, a.min_hist
    );
    let wszystkie: Vec<usize> = (0..akcje.len()).collect();
    let odniesienie = stala_na_oknie(&rek, naj_stala, a.min_hist);
    let (op, os, od_, onaj, omin) = podsumuj(&odniesienie);
    println!(
        "\n  {:<34} {:>11} {:>9} {:>13} {:>12} {:>22}",
        "polityka", "PnL $", "dni str.", "najg. dzień", "min krzywej", "różnica vs stała [5–95 %]"
    );
    println!(
        "  {:<34} {:>+11.2} {:>8.1} % {:>+13.2} {:>12.2} {:>22}",
        format!("STAŁA {} (odniesienie)", akcje[naj_stala].nazwa()),
        op,
        os as f64 / od_.max(1) as f64 * 100.0,
        onaj,
        omin,
        "—"
    );
    for (nazwa, m) in [
        ("model LINIOWY (ridge, 16 akcji)", Model::Liniowy),
        ("model SIEĆ (F→12→16)", Model::Siec),
        ("KONTROLA: wybór losowy", Model::Losowy),
        ("KONTROLA: etykiety przetasowane", Model::Tasowane),
    ] {
        let w = walk_forward(&rek, &wszystkie, m, a.min_hist, a.co_dni, 12345);
        let (p, s, d, naj, min) = podsumuj(&w);
        let (lo, hi) = bootstrap_roznicy(&w, &odniesienie, 2000, 777);
        println!(
            "  {:<34} {:>+11.2} {:>8.1} % {:>+13.2} {:>12.2} {:>+10.2} [{:+.0}, {:+.0}]",
            nazwa,
            p,
            s as f64 / d.max(1) as f64 * 100.0,
            naj,
            min,
            p - op,
            lo,
            hi
        );
    }

    // --- kontrola STAŁEJ EKSPOZYCJI: model wybiera tylko głębokość ---
    println!(
        "\n  KONTROLA STAŁEJ EKSPOZYCJI — model wybiera wyłącznie GŁĘBOKOŚĆ przy tej\n\
         \x20 samej liczbie warstw. Przewaga, która znika w tym wierszu, była dźwignią,\n\
         \x20 nie geometrią."
    );
    for jedn in [2usize, 3, 4] {
        let doz: Vec<usize> = akcje
            .iter()
            .enumerate()
            .filter(|(_, x)| x.jedn == jedn)
            .map(|(i, _)| i)
            .collect();
        let naj_w_grupie = *doz
            .iter()
            .max_by(|x, y| {
                miary(&rek, **x, a.kapital)
                    .pnl
                    .partial_cmp(&miary(&rek, **y, a.kapital).pnl)
                    .unwrap()
            })
            .unwrap();
        let odn = stala_na_oknie(&rek, naj_w_grupie, a.min_hist);
        let (op2, ..) = podsumuj(&odn);
        let w = walk_forward(&rek, &doz, Model::Liniowy, a.min_hist, a.co_dni, 12345);
        let (p, s, d, naj, _) = podsumuj(&w);
        let (lo, hi) = bootstrap_roznicy(&w, &odn, 2000, 777);
        println!(
            "  {:<34} {:>+11.2} {:>8.1} % {:>+13.2} {:>12} {:>+10.2} [{:+.0}, {:+.0}]",
            format!("{jedn} warstwy: model vs {}", akcje[naj_w_grupie].nazwa()),
            p,
            s as f64 / d.max(1) as f64 * 100.0,
            naj,
            format!("stała {op2:+.0}"),
            p - op2,
            lo,
            hi
        );
    }

    // ==========================================================
    //  5. CO MODEL W OGÓLE WIDZI — ważność cech liniowych
    // ==========================================================
    println!("\n\n╔══════════════════════════════════════════════════════════════╗");
    println!("║ 5. CZY W CHWILI SYGNAŁU JEST JAKAKOLWIEK INFORMACJA");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!(
        "  Korelacja cechy z RÓŻNICĄ wyniku „najgłębsza siatka − płytkie wejście”.\n\
         \x20 Jeśli żadna nie odstaje od cech kontrolnych, model nie ma z czego wybierać."
    );
    let plytka = akcje
        .iter()
        .position(|x| x.jedn == 1 && x.deep == 0.0)
        .unwrap();
    let gleboka = akcje
        .iter()
        .position(|x| x.jedn == 4 && x.deep == 5.0)
        .unwrap();
    let dy: Vec<f64> = rek
        .iter()
        .map(|r| r.wyn[gleboka].pnl - r.wyn[plytka].pnl)
        .collect();
    let mut kor: Vec<(f64, usize)> = (0..F)
        .map(|j| {
            let xs: Vec<f64> = rek.iter().map(|r| r.x[j] as f64).collect();
            (korelacja(&xs, &dy).abs(), j)
        })
        .collect();
    kor.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap());
    // próg z 20 cech LOSOWYCH — bez niego każda korelacja wygląda na coś
    let mut prog = 0.0f64;
    {
        let mut st = 99u64;
        for _ in 0..20 {
            let xs: Vec<f64> = (0..rek.len())
                .map(|_| {
                    st ^= st << 13;
                    st ^= st >> 7;
                    st ^= st << 17;
                    (st >> 11) as f64 / (1u64 << 53) as f64
                })
                .collect();
            prog = prog.max(korelacja(&xs, &dy).abs());
        }
    }
    for (v, j) in kor.iter().take(8) {
        println!(
            "  {:<22} |r| = {:.4}{}",
            NAZWY[*j],
            v,
            if *v > prog {
                "   ← nad progiem kontrolnym"
            } else {
                ""
            }
        );
    }
    println!("  próg: najwyższa z 20 cech LOSOWYCH |r| = {prog:.4}");

    println!(
        "\nUWAGA: pomiar NA ŚCIEŻKACH. Lot 0,01 na warstwę, bez limitu jednoczesnych\n\
         koszyków, bez marginesu i bez opóźnienia wykonania. Liczby NIE są wprost\n\
         porównywalne z wynikiem silnika — służą do porównań MIĘDZY wariantami."
    );
    Ok(())
}

fn korelacja(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    if n < 3.0 {
        return 0.0;
    }
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..a.len() {
        let (x, y) = (a[i] - ma, b[i] - mb);
        sxy += x * y;
        sxx += x * x;
        syy += y * y;
    }
    if sxx < 1e-12 || syy < 1e-12 {
        0.0
    } else {
        sxy / (sxx * syy).sqrt()
    }
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poziomy_odwzorowuja_geometrie_silnika() {
        let a = Akcja {
            jedn: 3,
            deep: 3.0,
            tol: -2.0,
        };
        let p = poziomy(Side::Buy, 100.0, 110.0, &a);
        assert_eq!(p.len(), 3);
        assert!((p[0] - 97.0).abs() < 1e-9, "najgłębszy {}", p[0]);
        assert!((p[2] - 108.0).abs() < 1e-9, "najpłytszy {}", p[2]);
        // sprzedaż: lustro
        let ps = poziomy(Side::Sell, 100.0, 110.0, &a);
        assert!((ps[0] - 113.0).abs() < 1e-9 && (ps[2] - 102.0).abs() < 1e-9);
    }

    #[test]
    fn jedna_warstwa_staje_na_krawedzi_gorszej() {
        // wariant „wejdź od razu”: dla kupna to GÓRNA krawędź, wypełnia się
        // jako pierwsza. Głębokość nie może na to wpływać.
        let p0 = poziomy(
            Side::Buy,
            100.0,
            110.0,
            &Akcja {
                jedn: 1,
                deep: 0.0,
                tol: -2.0,
            },
        );
        let p5 = poziomy(
            Side::Buy,
            100.0,
            110.0,
            &Akcja {
                jedn: 1,
                deep: 5.0,
                tol: -2.0,
            },
        );
        assert!((p0[0] - 108.0).abs() < 1e-9);
        assert!(
            (p5[0] - 108.0).abs() < 1e-9,
            "głębokość nie rusza jednej warstwy"
        );
    }

    #[test]
    fn glebsza_siatka_stawia_dalszy_poziom() {
        let a = poziomy(
            Side::Buy,
            100.0,
            110.0,
            &Akcja {
                jedn: 3,
                deep: 0.0,
                tol: -2.0,
            },
        );
        let b = poziomy(
            Side::Buy,
            100.0,
            110.0,
            &Akcja {
                jedn: 3,
                deep: 5.0,
                tol: -2.0,
            },
        );
        assert!(
            b[0] < a[0],
            "głębsza siatka ma niższy poziom 0: {} vs {}",
            b[0],
            a[0]
        );
        assert!((a[2] - b[2]).abs() < 1e-9, "krawędź gorsza się nie rusza");
    }

    #[test]
    fn ridge_odtwarza_zaleznosc_liniowa() {
        let x: Vec<Vec<f32>> = (0..200).map(|i| vec![i as f32 / 100.0, 1.0]).collect();
        let xr: Vec<&[f32]> = x.iter().map(|v| v.as_slice()).collect();
        let y: Vec<f64> = x.iter().map(|v| 3.0 * v[0] as f64 + 2.0).collect();
        let m = Ridge::ucz(&xr, &y, 1e-6);
        let p = m.pred(&[1.0, 1.0]);
        assert!((p - 5.0).abs() < 0.05, "przewidziane {p}");
    }

    #[test]
    fn bootstrap_zera_dla_identycznych_szeregow() {
        let a: Vec<(i64, f64)> = (0..40).map(|i| (i, i as f64)).collect();
        let (lo, hi) = bootstrap_roznicy(&a, &a, 500, 1);
        assert!(lo.abs() < 1e-9 && hi.abs() < 1e-9);
    }

    #[test]
    fn bootstrap_wykrywa_stala_przewage() {
        let a: Vec<(i64, f64)> = (0..60).map(|i| (i, 10.0)).collect();
        let b: Vec<(i64, f64)> = (0..60).map(|i| (i, 1.0)).collect();
        let (lo, hi) = bootstrap_roznicy(&a, &b, 1000, 3);
        assert!(lo > 500.0 && hi < 550.0, "[{lo}, {hi}]");
    }

    #[test]
    fn dni_sumuja_po_dacie() {
        let d = dni_z(&[(1, 2.0), (1, 3.0), (2, -1.0)]);
        assert_eq!(d, vec![(1, 5.0), (2, -1.0)]);
    }
}
