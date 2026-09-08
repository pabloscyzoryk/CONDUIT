
use anyhow::{Context, Result};
use conduit_backtest::data::{load_messages, TickData};
use conduit_backtest::metrics::{compute, DayStat, Metrics};
use conduit_backtest::runner::{run, RunConfig, RunResult};
use conduit_core::settings::Preset;
use conduit_core::types::{CloseReason, ClosedTrade, Side, Ts, XAU_CONTRACT};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Przesunięcie zegara wiadomości względem zegara ticków: +180 minut.
const MSGOFF: i64 = 180 * 60_000;
const H: i64 = 3_600_000;
const D: i64 = 86_400_000;
/// Kapitał startowy — ten sam, na którym mierzymy czempiona.
const START: f64 = 200.0;
/// Ile jednostek na sygnał. OSTRZE gra trzema, więc wyrocznia też trzema —
/// inaczej „luka w dolarach" porównywałaby różne rozmiary pozycji.
const UNITS: f64 = 3.0;
/// Lot jako procent kapitału — dokładnie reguła OSTRZA (`lot_percent: 0.5`).
const LOT_PCT: f64 = 0.5;
/// Wolumen odniesienia przy kapitale startowym: 3 × 0,01 lota = 3 $ na punkt.
const VOL_REF: f64 = 0.03;

/// Górny limit lota na jedno zlecenie — pole `lot_max` z presetu OSTRZE.
/// KRYTYCZNE dla compoundingu wyroczni: wyrocznia ma 100 % trafień, więc
/// obstawianie stałym ułamkiem kapitału rośnie geometrycznie i bez tego
/// limitu daje liczby rzędu 1e+99, czyli nic nie znaczące. Silnik ten limit
/// nakłada (`lot_max: 100.0`), więc wyrocznia musi nakładać go tak samo.
const LOT_MAX: f64 = 100.0;

/// Jak dobierany jest lot pojedynczej jednostki.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TrybLotu {
    /// 0,01 lota — bez compoundingu. To JEDYNA miara „ile pieniędzy leży na
    /// stole", która nie zależy od tempa dorabiania kapitału.
    Staly,
    /// procent kapitału, dokładnie reguła OSTRZA, bez zaokrąglania
    Procent,
    /// procent kapitału z zaokrągleniem i podłogą 0,01 lota (wykonalność)
    ProcentReal,
}

fn lot_jednostki(balance: f64, tryb: TrybLotu) -> f64 {
    match tryb {
        TrybLotu::Staly => 0.01,
        TrybLotu::Procent => (balance * LOT_PCT / 100.0 / 100.0).clamp(1e-9, LOT_MAX),
        TrybLotu::ProcentReal => {
            let l = balance * LOT_PCT / 100.0 / 100.0;
            (l.clamp(0.01, LOT_MAX) * 100.0).round() / 100.0
        }
    }
}

// ============================================================
//  SYGNAŁY — własny odczyt, bo potrzebujemy pól, których nie ma
//  `RawSignal` (tp_open, tag_first_entry).
// ============================================================

#[derive(Debug, Clone, Deserialize)]
struct JsonSig {
    id: i64,
    ts: i64,
    #[serde(default)]
    edited: Option<i64>,
    dir: String,
    #[serde(default)]
    limit: bool,
    lo: f64,
    hi: f64,
    sl: f64,
    tps: Vec<f64>,
    #[serde(default)]
    tp_open: bool,
    #[serde(default)]
    tag_high_risk: bool,
    #[serde(default)]
    tag_may_not: bool,
    #[serde(default)]
    tag_first_entry: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct JsonPlik {
    signals: Vec<JsonSig>,
}

#[derive(Debug, Clone)]
struct Sig {
    id: i64,
    ts_utc: i64,
    /// znacznik w zegarze SERWERA (ms)
    ts: i64,
    buy: bool,
    zlo: f64,
    zhi: f64,
    sl: f64,
    tps: Vec<f64>,
    limit: bool,
    tp_open: bool,
    high_risk: bool,
    may_not: bool,
    first_entry: bool,
    edited: bool,
}

impl Sig {
    /// Cena, po której WCHODZIMY (BUY → ask, SELL → bid).
    #[inline]
    fn cena_wejscia(&self, bid: f64, ask: f64) -> f64 {
        if self.buy {
            ask
        } else {
            bid
        }
    }
    /// Cena, po której WYCHODZIMY (BUY → bid, SELL → ask).
    #[inline]
    fn cena_wyjscia(&self, bid: f64, ask: f64) -> f64 {
        if self.buy {
            bid
        } else {
            ask
        }
    }
    /// Zysk w punktach ceny przy wyjściu po `px` z wejścia `pin`.
    #[inline]
    fn pts(&self, pin: f64, px: f64) -> f64 {
        if self.buy {
            px - pin
        } else {
            pin - px
        }
    }
    /// Czy cena wejściowa mieści się w strefie (albo jest lepsza)?
    #[inline]
    fn w_strefie(&self, ep: f64) -> bool {
        if self.buy {
            ep <= self.zhi
        } else {
            ep >= self.zlo
        }
    }
}

// ============================================================
//  SKANY TICKÓW
// ============================================================

const NIGDY: i64 = i64::MAX;

/// Siatka limitów czasu trzymania (minuty) dla polityk wykonalnych.
const CZASY_MIN: [i64; 8] = [15, 30, 60, 120, 240, 480, 720, 1440];
/// Progi uzbrojenia zapadki (dolary zysku przy wolumenie odniesienia → punkty).
const ZAP_START: [f64; 5] = [2.0, 4.0, 8.0, 16.0, 32.0];
/// Ile procent szczytu zapadka blokuje.
const ZAP_PCT: [f64; 4] = [30.0, 50.0, 70.0, 90.0];

/// Wynik skanu fazy WYJŚCIA z konkretnego wejścia.
#[derive(Debug, Clone, Default)]
struct Wyj {
    /// najlepsze wychylenie PRZED dotknięciem SL (punkty ceny)
    mfe: f64,
    mfe_ts: i64,
    mfe_px: f64,
    /// najlepsze wychylenie bez cięcia na SL — ile rynek dał w ogóle
    mfe_raw: f64,
    mfe_raw_ts: i64,
    /// najgorsze wychylenie w oknie
    mae: f64,
    mae_ts: i64,
    sl_ts: i64,
    sl_fill: f64,
    tp_ts: [i64; 4],
    last_ts: i64,
    last_px: f64,
    /// POLITYKI WYKONALNE (bez znajomości przyszłości), wynik w punktach ceny.
    /// `czas[k]` — trzymaj do `CZASY_MIN[k]` minut albo do SL, bez celu zysku.
    czas: [f64; 8],
    /// `zapadka[x][s]` — zapadka blokująca `ZAP_PCT[x]` % szczytu, uzbrajana
    /// po przekroczeniu `ZAP_START[s]` punktów; poza tym SL i horyzont.
    zapadka: [[f64; 5]; 4],
    /// bez celu zysku: tylko SL albo koniec horyzontu
    bez_celu: f64,
}

impl Wyj {
    fn nowy() -> Self {
        Wyj {
            mfe: f64::NEG_INFINITY,
            mfe_ts: NIGDY,
            mfe_px: 0.0,
            mfe_raw: f64::NEG_INFINITY,
            mfe_raw_ts: NIGDY,
            mae: f64::INFINITY,
            mae_ts: NIGDY,
            sl_ts: NIGDY,
            sl_fill: 0.0,
            tp_ts: [NIGDY; 4],
            last_ts: 0,
            last_px: 0.0,
            czas: [0.0; 8],
            zapadka: [[0.0; 5]; 4],
            bez_celu: 0.0,
        }
    }

    /// Normalne wyjście: TP1 albo SL, co pierwsze. Gdy nic — po horyzoncie.
    /// SL ma pierwszeństwo w tym samym ticku, dokładnie jak `SimBroker`.
    fn normalne(&self, s: &Sig) -> (i64, f64, &'static str) {
        let t1 = self.tp_ts[0];
        if t1 != NIGDY && t1 < self.sl_ts {
            (t1, s.tps[0], "TP1")
        } else if self.sl_ts != NIGDY {
            (self.sl_ts, self.sl_fill, "SL")
        } else {
            (self.last_ts, self.last_px, "HORYZONT")
        }
    }
}

/// Skan fazy wyjścia od indeksu `i0` (pierwszy tick PO wejściu) do `koniec`.
fn skan_wyjscia(t: &TickData, s: &Sig, i0: usize, px_in: f64, koniec: i64) -> Wyj {
    let mut w = Wyj::nowy();
    let n = t.len();
    let mut i = i0;
    let mut sl_juz = false;
    // --- stan polityk wykonalnych ---
    let t_in = if i0 < n { t.ts(i0) } else { 0 };
    let mut ti = 0usize; // ile limitów czasu już rozliczonych
    let mut czas_v = [f64::NAN; 8];
    let mut zap_v: [[Option<f64>; 5]; 4] = [[None; 5]; 4];
    let mut zap_done = [0usize; 4];
    let mut pk = f64::NEG_INFINITY; // szczyt zysku od wejścia
    let mut pol_koniec = false;
    while i < n {
        let ts = t.ts(i);
        if ts > koniec {
            break;
        }
        let x = s.cena_wyjscia(t.bid(i), t.ask(i));
        let p = s.pts(px_in, x);

        if p > w.mfe_raw {
            w.mfe_raw = p;
            w.mfe_raw_ts = ts;
        }
        if !sl_juz && p > w.mfe {
            w.mfe = p;
            w.mfe_ts = ts;
            w.mfe_px = x;
        }
        if p < w.mae {
            w.mae = p;
            w.mae_ts = ts;
        }

        // dotknięcia celów
        for (k, tp) in s.tps.iter().take(4).enumerate() {
            if w.tp_ts[k] == NIGDY {
                let hit = if s.buy { x >= *tp } else { x <= *tp };
                if hit {
                    w.tp_ts[k] = ts;
                }
            }
        }
        // dotknięcie stopa
        if !sl_juz {
            let hit = if s.buy { x <= s.sl } else { x >= s.sl };
            if hit {
                sl_juz = true;
                w.sl_ts = ts;
                // stop realizuje się po cenie rynkowej, jeśli rynek przeskoczył poziom
                w.sl_fill = if s.buy { s.sl.min(x) } else { s.sl.max(x) };
            }
        }

        // ---------- POLITYKI WYKONALNE ----------
        // Liczone w tym samym przebiegu, bo inaczej trzeba by skanować ticki
        // po raz drugi dla każdego zestawu parametrów. Stop ma pierwszeństwo
        // w obrębie ticku — dokładnie jak w silniku.
        if !pol_koniec {
            if sl_juz {
                let psl = s.pts(px_in, w.sl_fill);
                while ti < CZASY_MIN.len() {
                    czas_v[ti] = psl;
                    ti += 1;
                }
                for xi in 0..ZAP_PCT.len() {
                    for si in 0..ZAP_START.len() {
                        if zap_v[xi][si].is_none() {
                            zap_v[xi][si] = Some(psl);
                        }
                    }
                }
                pol_koniec = true;
            } else {
                // limit czasu trzymania — wskaźnik tylko rośnie, koszt O(1)
                let wiek = ts - t_in;
                while ti < CZASY_MIN.len() && wiek >= CZASY_MIN[ti] * 60_000 {
                    czas_v[ti] = p;
                    ti += 1;
                }
                // zapadka: szczyt jest wspólny, więc próg naruszenia też —
                // różne progi uzbrojenia zmieniają tylko MOMENT uzbrojenia
                if p > pk {
                    pk = p;
                }
                for xi in 0..ZAP_PCT.len() {
                    if zap_done[xi] == ZAP_START.len() {
                        continue;
                    }
                    if p <= pk * ZAP_PCT[xi] / 100.0 {
                        for si in 0..ZAP_START.len() {
                            if zap_v[xi][si].is_none() && pk >= ZAP_START[si] {
                                zap_v[xi][si] = Some(p);
                                zap_done[xi] += 1;
                            }
                        }
                    }
                }
            }
        }

        w.last_ts = ts;
        w.last_px = x;
        i += 1;
    }
    if !w.mfe.is_finite() {
        w.mfe = 0.0;
    }
    if !w.mfe_raw.is_finite() {
        w.mfe_raw = 0.0;
    }
    if !w.mae.is_finite() {
        w.mae = 0.0;
    }
    // ---------- domknięcie polityk po ostatniej cenie w oknie ----------
    if w.last_ts != 0 {
        let plast = s.pts(px_in, w.last_px);
        while ti < CZASY_MIN.len() {
            czas_v[ti] = plast;
            ti += 1;
        }
        for xi in 0..ZAP_PCT.len() {
            for si in 0..ZAP_START.len() {
                if zap_v[xi][si].is_none() {
                    zap_v[xi][si] = Some(plast);
                }
            }
        }
        w.bez_celu = if sl_juz {
            s.pts(px_in, w.sl_fill)
        } else {
            plast
        };
    }
    for k in 0..CZASY_MIN.len() {
        w.czas[k] = if czas_v[k].is_nan() { 0.0 } else { czas_v[k] };
    }
    for xi in 0..ZAP_PCT.len() {
        for si in 0..ZAP_START.len() {
            w.zapadka[xi][si] = zap_v[xi][si].unwrap_or(0.0);
        }
    }
    w
}

/// NAJLEPSZA PARA (wejście, wyjście) — pełna wyrocznia.
///
/// Dlaczego para, a nie „najlepsze wejście" i osobno „najlepsze wyjście":
/// dobierane niezależnie dają bzdurę. Najniższa cena w strefie wypada zwykle
/// na samym dnie wybicia, często o włos nad stopem — i po niej rynek już
/// tylko idzie w stop, więc z tego wejścia NIE MA korzystnego wyjścia.
/// Wyrocznia musi maksymalizować RÓŻNICĘ, a nie dwa końce osobno.
///
/// Ograniczenia (żeby sufit nie był absurdem):
///  * wejście tylko w strefie i tylko w oknie `okno_we` od sygnału,
///  * wejście tylko ZANIM cena po raz pierwszy dotknie SL — dokładnie tak,
///    jak działa `skip_if_sl_breached` w OSTRZU,
///  * pozycja nie może przeżyć dotknięcia SL,
///  * czas trzymania nie dłuższy niż `horyzont`.
///
/// Algorytm: monotoniczna kolejka kandydatów na wejście (rosnąco po cenie),
/// czyszczona przy dotknięciu SL i przycinana od czoła, gdy kandydat jest
/// starszy niż horyzont. Czoło kolejki to najlepsze dostępne wejście dla
/// wyjścia „tutaj". Jedno przejście po tickach.
fn najlepsza_para(
    t: &TickData,
    s: &Sig,
    t0: i64,
    okno_we: i64,
    horyzont: i64,
    ciecie_dobowe: bool,
) -> Option<(i64, f64, i64, f64)> {
    let n = t.len();
    let polnoc = (t0 / D + 1) * D;
    let mut kon_we = t0 + okno_we;
    let mut kon = t0 + okno_we + horyzont;
    if ciecie_dobowe {
        kon_we = kon_we.min(polnoc);
        kon = kon.min(polnoc);
    }
    let mut dq: std::collections::VecDeque<(i64, f64)> = std::collections::VecDeque::new();
    let mut best: Option<(i64, f64, i64, f64)> = None;
    let mut bestp = 0.0f64;
    let mut sl_byl = false;

    let mut i = t.index_at(t0);
    while i < n {
        let ts = t.ts(i);
        if ts > kon {
            break;
        }
        let bid = t.bid(i);
        let ask = t.ask(i);
        let ep = s.cena_wejscia(bid, ask);
        let xe = s.cena_wyjscia(bid, ask);

        // 1) dotknięcie stopa unieważnia wszystko, co było wcześniej
        let sl_hit = if s.buy { xe <= s.sl } else { xe >= s.sl };
        if sl_hit {
            dq.clear();
            sl_byl = true;
            if ts > kon_we {
                break;
            }
            i += 1;
            continue;
        }
        // 2) nowy kandydat na wejście
        if !sl_byl && ts <= kon_we && s.w_strefie(ep) {
            while let Some(&(_, p)) = dq.back() {
                let gorszy = if s.buy { p >= ep } else { p <= ep };
                if gorszy {
                    dq.pop_back();
                } else {
                    break;
                }
            }
            dq.push_back((ts, ep));
        }
        // 3) kandydaci starsi niż horyzont odpadają
        while let Some(&(t1, _)) = dq.front() {
            if ts - t1 > horyzont {
                dq.pop_front();
            } else {
                break;
            }
        }
        // 4) najlepsza para kończąca się w tym ticku
        if let Some(&(t1, p1)) = dq.front() {
            let pr = s.pts(p1, xe);
            if pr > bestp {
                bestp = pr;
                best = Some((t1, p1, ts, xe));
            }
        }
        if sl_byl && dq.is_empty() && ts > kon_we {
            break;
        }
        i += 1;
    }
    if bestp > 0.0 {
        best
    } else {
        None
    }
}

/// Cechy OBSERWOWALNE w chwili decyzji — wyłącznie z przeszłości.
#[derive(Debug, Clone, Default, Serialize)]
struct Cechy {
    godzina: i64,
    dzien_tyg: i64,
    szer_strefy: f64,
    dyst_sl: f64,
    dyst_tp1: f64,
    r_mult: f64,
    /// zakres ceny w 60 minutach PRZED sygnałem
    zmiennosc_60m: f64,
    /// cena minus średnia 72-godzinna (dodatnia = nad średnią)
    nad_ma72: f64,
    /// czy sygnał gra POD prąd średniej 72 h (kryterium OSTRZA)
    kontra_ma: bool,
    /// ile cena musi przejechać, żeby dotknąć strefy (dodatnia = jeszcze daleko)
    dyst_do_strefy: f64,
    sygnalow_24h: i64,
    /// wynik poprzedniego sygnału w normalnej grze (punkty ceny)
    poprz_wynik: f64,
}

/// Kompletna analiza jednego sygnału.
#[derive(Debug, Clone)]
struct Analiza {
    s: Sig,
    px0: f64,
    strefa_dotknieta: bool,
    /// pierwsze naruszenie SL licząc OD SYGNAŁU (blokuje wejście — tak jak
    /// `skip_if_sl_breached` w OSTRZU)
    sl_pre_ts: i64,
    // --- wejście normalne: pierwsze dotknięcie strefy ---
    t_in_n: i64,
    px_in_n: f64,
    i_in_n: usize,
    // --- wejście idealne: najlepsza cena w oknie ---
    t_in_b: i64,
    px_in_b: f64,
    i_in_b: usize,
    // --- wychylenia liczone OD MOMENTU SYGNAŁU (do etykiet) ---
    mfe_sig: f64,
    mfe_sig_ts: i64,
    mae_sig: f64,
    mae_sig_ts: i64,
    tp_ts_sig: [i64; 4],
    sl_ts_sig: i64,
    // --- wyjścia ---
    wn: Wyj,
    wb: Wyj,
    /// pełna wyrocznia: (wejście ts, wejście cena, wyjście ts, wyjście cena)
    para: Option<(i64, f64, i64, f64)>,
    cechy: Cechy,
}

impl Analiza {
    /// W1 — normalne wejście, normalne wyjście (bez selekcji).
    fn w1_surowy(&self) -> Option<(i64, f64, i64, f64, &'static str)> {
        if !self.strefa_dotknieta {
            return None;
        }
        let (tx, px, powod) = self.wn.normalne(&self.s);
        Some((self.t_in_n, self.px_in_n, tx, px, powod))
    }
    fn w1_pts(&self) -> Option<f64> {
        self.w1_surowy()
            .map(|(_, pin, _, px, _)| self.s.pts(pin, px))
    }
}

/// Skan pełny jednego sygnału.
fn analizuj(
    t: &TickData,
    s: &Sig,
    okno_wejscia: i64,
    horyzont: i64,
    opoznienie: i64,
    ciecie_dobowe: bool,
) -> Option<Analiza> {
    let t0 = s.ts + opoznienie;
    let n = t.len();
    let i0 = t.index_at(t0);
    if i0 >= n {
        return None;
    }
    let px0 = s.cena_wejscia(t.bid(i0), t.ask(i0));

    // koniec okna wejścia; przy cięciu dobowym nie przekraczamy północy serwera
    let mut kon_we = t0 + okno_wejscia;
    if ciecie_dobowe {
        let polnoc = (t0 / D + 1) * D;
        kon_we = kon_we.min(polnoc);
    }

    // ---------- faza 1: wejście + wychylenia od sygnału ----------
    let kon_sig = {
        let mut k = t0 + horyzont;
        if ciecie_dobowe {
            k = k.min((t0 / D + 1) * D);
        }
        k
    };

    let mut sl_pre_ts = NIGDY;
    let mut touch: Option<(i64, f64, usize)> = None;
    let mut best: Option<(i64, f64, usize)> = None;
    let mut mfe_sig = f64::NEG_INFINITY;
    let mut mfe_sig_ts = NIGDY;
    let mut mae_sig = f64::INFINITY;
    let mut mae_sig_ts = NIGDY;
    let mut tp_ts_sig = [NIGDY; 4];
    let mut sl_ts_sig = NIGDY;

    let kon_max = kon_we.max(kon_sig);
    let mut i = i0;
    while i < n {
        let ts = t.ts(i);
        if ts > kon_max {
            break;
        }
        let bid = t.bid(i);
        let ask = t.ask(i);
        let xe = s.cena_wyjscia(bid, ask);

        // --- wychylenia i cele od momentu sygnału ---
        if ts <= kon_sig {
            let p = s.pts(px0, xe);
            if p > mfe_sig {
                mfe_sig = p;
                mfe_sig_ts = ts;
            }
            if p < mae_sig {
                mae_sig = p;
                mae_sig_ts = ts;
            }
            for (k, tp) in s.tps.iter().take(4).enumerate() {
                if tp_ts_sig[k] == NIGDY {
                    let hit = if s.buy { xe >= *tp } else { xe <= *tp };
                    if hit {
                        tp_ts_sig[k] = ts;
                    }
                }
            }
            if sl_ts_sig == NIGDY {
                let hit = if s.buy { xe <= s.sl } else { xe >= s.sl };
                if hit {
                    sl_ts_sig = ts;
                }
            }
        }

        // --- faza wejścia (kończy się z chwilą naruszenia SL) ---
        if ts <= kon_we && sl_pre_ts == NIGDY {
            let ep = s.cena_wejscia(bid, ask);
            if s.w_strefie(ep) {
                if touch.is_none() {
                    touch = Some((ts, ep, i));
                }
                let lepsze = match best {
                    None => true,
                    Some((_, bp, _)) => {
                        if s.buy {
                            ep < bp
                        } else {
                            ep > bp
                        }
                    }
                };
                if lepsze {
                    best = Some((ts, ep, i));
                }
            }
            // naruszenie stopa PRZED wejściem — dalej już nie szukamy
            let hit = if s.buy { xe <= s.sl } else { xe >= s.sl };
            if hit {
                sl_pre_ts = ts;
            }
        }
        i += 1;
    }

    let strefa_dotknieta = touch.is_some();
    let (t_in_n, px_in_n, i_in_n) = touch.unwrap_or((NIGDY, px0, i0));
    let (t_in_b, px_in_b, i_in_b) = best.unwrap_or((NIGDY, px0, i0));

    // ---------- faza 2: wyjścia ----------
    let kon_wy = |te: i64| -> i64 {
        let mut k = te + horyzont;
        if ciecie_dobowe {
            k = k.min((te / D + 1) * D);
        }
        k
    };
    let wn = if strefa_dotknieta {
        skan_wyjscia(t, s, i_in_n, px_in_n, kon_wy(t_in_n))
    } else {
        Wyj::nowy()
    };
    let wb = if best.is_some() {
        skan_wyjscia(t, s, i_in_b, px_in_b, kon_wy(t_in_b))
    } else {
        Wyj::nowy()
    };
    let para = najlepsza_para(t, s, t0, okno_wejscia, horyzont, ciecie_dobowe);

    Some(Analiza {
        s: s.clone(),
        px0,
        strefa_dotknieta,
        sl_pre_ts,
        t_in_n,
        px_in_n,
        i_in_n,
        t_in_b,
        px_in_b,
        i_in_b,
        mfe_sig: if mfe_sig.is_finite() { mfe_sig } else { 0.0 },
        mfe_sig_ts,
        mae_sig: if mae_sig.is_finite() { mae_sig } else { 0.0 },
        mae_sig_ts,
        tp_ts_sig,
        sl_ts_sig,
        wn,
        wb,
        para,
        cechy: Cechy::default(),
    })
}

// ============================================================
//  PORTFEL — jeden przebieg po tickach, wspólny dla wszystkich wariantów
// ============================================================

#[derive(Debug, Clone)]
struct Plan {
    sid: i64,
    buy: bool,
    t_in: i64,
    px_in: f64,
    t_out: i64,
    px_out: f64,
}

struct Podsumowanie {
    m: Metrics,
    dni: Vec<DayStat>,
}

/// Odtwarza portfel wyroczni tick po ticku. Lot liczony z BIEŻĄCEGO salda —
/// dokładnie tak, jak robi to silnik (`Engine::lot_size`).
fn symuluj(
    t: &TickData,
    plany: &[Plan],
    od: i64,
    do_: i64,
    dzienny_reset: bool,
    max_poz: usize,
    tryb_lotu: TrybLotu,
) -> Podsumowanie {
    let mut p: Vec<&Plan> = plany
        .iter()
        .filter(|x| x.t_in >= od && x.t_in < do_ && x.t_out != NIGDY)
        .collect();
    p.sort_by_key(|x| x.t_in);

    let i0 = t.index_at(od);
    let i1 = t.index_at(do_).min(t.len());

    let mut saldo = START;
    let mut otwarte: Vec<(usize, f64)> = Vec::new(); // (indeks planu, wolumen)
    let mut pi = 0usize;
    let mut trades: Vec<ClosedTrade> = Vec::new();
    let mut krzywa: Vec<(Ts, f64)> = Vec::new();
    let mut dni: Vec<DayStat> = Vec::new();
    let mut ticket = 1u64;

    let mut cur_day = i64::MIN;
    let mut day_start = saldo;
    let mut day_peak = saldo;
    let mut day_dd: f64 = 0.0;
    let mut day_trades = 0u32;
    let mut day_sig = 0u32;
    let mut last_curve = 0i64;
    let mut min_eq = saldo;

    for i in i0..i1 {
        let ts = t.ts(i);
        let bid = t.bid(i);
        let ask = t.ask(i);

        // ---------- granica doby ----------
        let day = ts.div_euclid(D);
        if day != cur_day {
            if cur_day != i64::MIN {
                // kapitał na granicy doby MUSI zawierać pozycje otwarte —
                // inaczej dzień, w którym coś przechodzi przez północ,
                // dostaje zysk przypisany do złej doby
                let mut eq = saldo;
                for (idx, vol) in &otwarte {
                    let pl = p[*idx];
                    let px = if pl.buy { bid } else { ask };
                    eq += pts_usd(pl.buy, pl.px_in, px, *vol);
                }
                dni.push(DayStat {
                    day: cur_day,
                    date: fmt_day(cur_day),
                    start_equity: day_start,
                    end_equity: eq,
                    profit: eq - day_start,
                    max_dd: day_dd,
                    min_equity: None, real_dd: None, real_dd_pct: None, equity_observation_basis: None,
                    trades: day_trades,
                    signals: day_sig,
                });
            }
            cur_day = day;
            if dzienny_reset && !dni.is_empty() {
                // domknij wszystko po rynku i wróć do kwoty startowej
                for (k, vol) in otwarte.drain(..) {
                    let pl = p[k];
                    let px = if pl.buy { bid } else { ask };
                    let zysk = pts_usd(pl.buy, pl.px_in, px, vol);
                    saldo += zysk;
                    trades.push(mk_trade(&mut ticket, pl, px, ts, vol, zysk));
                }
                saldo = START;
            }
            let mut eq0 = saldo;
            for (idx, vol) in &otwarte {
                let pl = p[*idx];
                let px = if pl.buy { bid } else { ask };
                eq0 += pts_usd(pl.buy, pl.px_in, px, *vol);
            }
            day_start = eq0;
            day_peak = day_start;
            day_dd = 0.0;
            day_trades = 0;
            day_sig = 0;
        }

        // ---------- zamknięcia ----------
        let mut k = 0;
        while k < otwarte.len() {
            let (idx, vol) = otwarte[k];
            if p[idx].t_out <= ts {
                let pl = p[idx];
                let zysk = pts_usd(pl.buy, pl.px_in, pl.px_out, vol);
                saldo += zysk;
                trades.push(mk_trade(&mut ticket, pl, pl.px_out, pl.t_out, vol, zysk));
                otwarte.swap_remove(k);
                day_trades += 1;
            } else {
                k += 1;
            }
        }

        // ---------- otwarcia ----------
        while pi < p.len() && p[pi].t_in <= ts {
            let idx = pi;
            pi += 1;
            if otwarte.len() * (UNITS as usize) >= max_poz {
                continue; // limit jednocześnie otwartych pozycji
            }
            let vol = UNITS * lot_jednostki(saldo.max(1.0), tryb_lotu);
            otwarte.push((idx, vol));
            day_sig += 1;
        }

        // ---------- kapitał ----------
        let mut eq = saldo;
        for (idx, vol) in &otwarte {
            let pl = p[*idx];
            let px = if pl.buy { bid } else { ask };
            eq += pts_usd(pl.buy, pl.px_in, px, *vol);
        }
        if eq < min_eq {
            min_eq = eq;
        }
        if eq > day_peak {
            day_peak = eq;
        }
        let dd = day_peak - eq;
        if dd > day_dd {
            day_dd = dd;
        }
        if ts - last_curve >= 60_000 {
            last_curve = ts;
            krzywa.push((ts, eq));
        }
    }

    // domknięcie reszty po ostatniej cenie
    if i1 > i0 {
        let ts = t.ts(i1 - 1);
        let bid = t.bid(i1 - 1);
        let ask = t.ask(i1 - 1);
        for (idx, vol) in otwarte.drain(..) {
            let pl = p[idx];
            let px = if pl.buy { bid } else { ask };
            let zysk = pts_usd(pl.buy, pl.px_in, px, vol);
            saldo += zysk;
            trades.push(mk_trade(&mut ticket, pl, px, ts, vol, zysk));
            day_trades += 1;
        }
        if cur_day != i64::MIN {
            dni.push(DayStat {
                day: cur_day,
                date: fmt_day(cur_day),
                start_equity: day_start,
                end_equity: saldo,
                profit: saldo - day_start,
                max_dd: day_dd,
                min_equity: None, real_dd: None, real_dd_pct: None, equity_observation_basis: None,
                trades: day_trades,
                signals: day_sig,
            });
        }
        krzywa.push((ts, saldo));
    }

    trades.sort_by_key(|x| x.close_ts);
    // Wyrocznia liczy wykonanie IDEALNE, nie preset — progu BE nie ma skąd
    // wziąć i nie ma po co: zero zostawia stary podział co do bitu.
    let mut m = compute(START, &krzywa, &dni, &trades, min_eq, saldo <= 0.0, 0.0);
    if dzienny_reset {
        let total: f64 = dni.iter().map(|d| d.profit).sum();
        m.total_profit = total;
        m.end_equity = START + total;
        m.end_balance = m.end_equity;
        m.return_pct = total / START * 100.0;
        m.max_dd_abs = dni.iter().map(|d| d.max_dd).fold(0.0, f64::max);
        m.max_dd_pct = m.max_dd_abs / START * 100.0;
    }
    Podsumowanie { m, dni }
}

#[inline]
fn pts_usd(buy: bool, pin: f64, px: f64, vol: f64) -> f64 {
    let p = if buy { px - pin } else { pin - px };
    p * XAU_CONTRACT * vol
}

fn mk_trade(ticket: &mut u64, pl: &Plan, px: f64, ts: i64, vol: f64, zysk: f64) -> ClosedTrade {
    let t = *ticket;
    *ticket += 1;
    ClosedTrade {
        profit_basis: None, cost_receipt: None,
        ticket: t,
        side: if pl.buy { Side::Buy } else { Side::Sell },
        volume: vol,
        open_price: pl.px_in,
        close_price: px,
        open_ts: pl.t_in,
        close_ts: ts,
        profit: zysk,
        commission: 0.0,
        swap: 0.0,
        reason: CloseReason::Manual,
        basket: Some(pl.sid as u32),
    }
}

fn fmt_day(day: i64) -> String {
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn dni_od_ymd(y: i64, m: i64, d: i64) -> i64 {
    let yy = if m <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ============================================================
//  RAPORT
// ============================================================

#[derive(Debug, Clone, Serialize)]
struct Wiersz {
    wariant: String,
    okno: String,
    tryb: String,
    zysk: f64,
    koncowe: f64,
    pf: f64,
    max_dd_usd: f64,
    max_dd_pct: f64,
    dni_plus_pct: f64,
    dni_handlowe: u32,
    transakcje: u32,
    srednio_na_dzien: f64,
    win_rate: f64,
}

fn wiersz(nazwa: &str, okno: &str, tryb: &str, p: &Podsumowanie) -> Wiersz {
    Wiersz {
        wariant: nazwa.into(),
        okno: okno.into(),
        tryb: tryb.into(),
        zysk: p.m.total_profit,
        koncowe: p.m.end_equity,
        pf: if p.m.profit_factor.is_finite() {
            p.m.profit_factor
        } else {
            999.0
        },
        max_dd_usd: p.m.max_dd_abs,
        max_dd_pct: p.m.max_dd_pct,
        dni_plus_pct: p.m.win_days_pct,
        dni_handlowe: p.m.trading_days,
        transakcje: p.m.trades,
        srednio_na_dzien: p.m.avg_per_day,
        win_rate: p.m.win_rate,
    }
}

// ============================================================
//  ETYKIETY
// ============================================================

#[derive(Debug, Clone, Serialize)]
struct Etykieta {
    id: i64,
    ts_utc: i64,
    ts_serwer_ms: i64,
    data: String,
    dir: String,
    limit: bool,
    tp_open: bool,
    lo: f64,
    hi: f64,
    sl: f64,
    tps: Vec<f64>,
    tag_high_risk: bool,
    tag_may_not: bool,
    tag_first_entry: bool,
    edited: bool,

    cena_w_chwili_sygnalu: f64,
    strefa_dotknieta: bool,
    sl_naruszony_przed_wejsciem: bool,

    wziety_w1: bool,
    wziety_w2: bool,
    wziety_w3: bool,

    wejscie_normalne_ts: Option<i64>,
    wejscie_normalne_cena: Option<f64>,
    wejscie_idealne_ts: Option<i64>,
    wejscie_idealne_cena: Option<f64>,
    wyjscie_normalne_ts: Option<i64>,
    wyjscie_normalne_cena: Option<f64>,
    wyjscie_normalne_powod: String,
    wyjscie_idealne_ts: Option<i64>,
    wyjscie_idealne_cena: Option<f64>,

    /// wychylenia liczone od CENY W CHWILI SYGNAŁU (punkty ceny)
    mfe_od_sygnalu: f64,
    mfe_od_sygnalu_ts: Option<i64>,
    mae_od_sygnalu: f64,
    mae_od_sygnalu_ts: Option<i64>,
    /// wychylenie od NORMALNEGO WEJŚCIA, ucięte na dotknięciu stopa — czyli
    /// tyle, ile dało się z tej pozycji WYJĄĆ. To jest etykieta wyroczni.
    mfe_od_wejscia: f64,
    /// wychylenie od wejścia BEZ ucięcia na stopie: dokąd rynek doszedł
    /// w ogóle, także po tym, jak pozycja byłaby już zamknięta. NIE nadaje
    /// się na cel uczenia — pozycji już wtedy nie ma.
    mfe_od_wejscia_pelne: f64,
    mae_od_wejscia: f64,

    /// UWAGA: `tp*_ts` i `sl_ts` liczone są od CHWILI SYGNAŁU, nie od wejścia.
    /// Dla zleceń limit cena stoi zwykle po drugiej stronie celu, więc TP1
    /// bywa „trafiony" w tej samej sekundzie, w której sygnał przyszedł —
    /// TP1 może więc pojawić się przed własnym wejściem. Jako etykieta wyniku
    /// te pola są bezużyteczne; służą wyłącznie do opisu
    /// położenia ceny względem poziomów. Do uczenia brać `*_po_wejsciu`.
    tp1_osiagniety: bool,
    tp1_ts: Option<i64>,
    tp2_osiagniety: bool,
    tp2_ts: Option<i64>,
    tp3_osiagniety: bool,
    tp3_ts: Option<i64>,
    sl_osiagniety: bool,
    sl_ts: Option<i64>,

    /// Trafienia liczone OD WEJŚCIA — to są prawdziwe zdarzenia pozycji.
    tp1_ts_po_wejsciu: Option<i64>,
    tp2_ts_po_wejsciu: Option<i64>,
    tp3_ts_po_wejsciu: Option<i64>,
    sl_ts_po_wejsciu: Option<i64>,

    /// Wyniki polityk WYKONALNYCH (bez wyroczni), punkty ceny, od wejścia
    /// normalnego. Odniesienia dla modelu: musi bić te liczby.
    pol_bez_celu_pts: Option<f64>,
    pol_czas_pts: Option<Vec<f64>>,

    /// wyniki w punktach ceny (× 3 $ = dolary przy wolumenie odniesienia)
    wynik_normalny_pts: Option<f64>,
    wynik_w2_pts: Option<f64>,
    wynik_w3_pts: Option<f64>,

    cechy: Cechy,
}

fn opt_ts(t: i64) -> Option<i64> {
    if t == NIGDY {
        None
    } else {
        Some(t)
    }
}

// ============================================================
//  ANALIZA ODZYSKIWALNOŚCI
// ============================================================

#[derive(Debug, Clone, Serialize)]
struct WynikCechy {
    cecha: String,
    kubelki: usize,
    /// dolary bez reguły na drugiej połowie
    baza_2p: f64,
    /// dolary z regułą wyuczoną na pierwszej połowie, zastosowaną na drugiej
    regula_2p: f64,
    /// odwrotny podział: ucz na drugiej, sprawdź na pierwszej
    baza_1p: f64,
    regula_1p: f64,
    /// suma poza próbą
    zysk_oos: f64,
    /// ile sygnałów reguła przepuszcza (druga połowa)
    przepuszczone_2p: usize,
    wszystkie_2p: usize,
}

/// Uczy progu „bierz tylko kubełki o dodatniej średniej" na jednej połowie
/// i sprawdza na drugiej.
fn ocen_cehe(
    nazwa: &str,
    dane: &[(i64, usize, f64)], // (ts, kubełek, wynik $)
    n_kub: usize,
    granica: i64,
) -> WynikCechy {
    let mut sum1 = vec![0.0; n_kub];
    let mut cnt1 = vec![0usize; n_kub];
    let mut sum2 = vec![0.0; n_kub];
    let mut cnt2 = vec![0usize; n_kub];
    for (ts, k, v) in dane {
        if *ts < granica {
            sum1[*k] += v;
            cnt1[*k] += 1;
        } else {
            sum2[*k] += v;
            cnt2[*k] += 1;
        }
    }
    // reguła z pierwszej połowy → wynik na drugiej
    let mut baza2 = 0.0;
    let mut reg2 = 0.0;
    let mut prz2 = 0usize;
    let mut all2 = 0usize;
    for (ts, k, v) in dane {
        if *ts >= granica {
            baza2 += v;
            all2 += 1;
            let ok = cnt1[*k] >= 8 && sum1[*k] / cnt1[*k] as f64 > 0.0;
            if ok {
                reg2 += v;
                prz2 += 1;
            }
        }
    }
    // reguła z drugiej połowy → wynik na pierwszej
    let mut baza1 = 0.0;
    let mut reg1 = 0.0;
    for (ts, k, v) in dane {
        if *ts < granica {
            baza1 += v;
            let ok = cnt2[*k] >= 8 && sum2[*k] / cnt2[*k] as f64 > 0.0;
            if ok {
                reg1 += v;
            }
        }
    }
    WynikCechy {
        cecha: nazwa.into(),
        kubelki: n_kub,
        baza_2p: baza2,
        regula_2p: reg2,
        baza_1p: baza1,
        regula_1p: reg1,
        zysk_oos: (reg2 - baza2) + (reg1 - baza1),
        przepuszczone_2p: prz2,
        wszystkie_2p: all2,
    }
}

// ============================================================
//  MAIN
// ============================================================

struct Arg {
    ticks: PathBuf,
    signals: PathBuf,
    out: PathBuf,
    preset: PathBuf,
    okno_wejscia_h: f64,
    horyzont_h: f64,
    bez_ostrza: bool,
}

fn parse_args() -> Result<Arg> {
    let mut a = Arg {
        ticks: "data/ticks.bin".into(),
        signals: "data/signals.json".into(),
        out: "../analiza".into(),
        preset: "presets/OSTRZE.json".into(),
        okno_wejscia_h: 4.0,
        horyzont_h: 24.0,
        bez_ostrza: false,
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < v.len() {
        let k = v[i].clone();
        let mut nast = |i: &mut usize| -> String {
            *i += 1;
            v.get(*i).cloned().unwrap_or_default()
        };
        match k.as_str() {
            "--ticks" => a.ticks = nast(&mut i).into(),
            "--signals" => a.signals = nast(&mut i).into(),
            "--out" => a.out = nast(&mut i).into(),
            "--preset" => a.preset = nast(&mut i).into(),
            "--okno-wejscia" => a.okno_wejscia_h = nast(&mut i).parse()?,
            "--horyzont" => a.horyzont_h = nast(&mut i).parse()?,
            "--bez-ostrza" => a.bez_ostrza = true,
            other => anyhow::bail!("nieznany argument: {other}"),
        }
        i += 1;
    }
    Ok(a)
}

fn main() -> Result<()> {
    let a = parse_args()?;
    let okno_wejscia = (a.okno_wejscia_h * H as f64) as i64;
    let horyzont = (a.horyzont_h * H as f64) as i64;

    eprintln!("wczytuję ticki…");
    let t = TickData::open(&a.ticks)?;
    eprintln!(
        "ticki: {} ({} … {})",
        t.len(),
        fmt_ts(t.first_ts()),
        fmt_ts(t.last_ts())
    );

    let txt = std::fs::read_to_string(&a.signals)
        .with_context(|| format!("nie mogę wczytać {}", a.signals.display()))?;
    let plik: JsonPlik = serde_json::from_str(&txt)?;
    let sygnaly: Vec<Sig> = plik
        .signals
        .iter()
        .filter(|s| !s.tps.is_empty() && s.sl > 0.0)
        .map(|s| Sig {
            id: s.id,
            ts_utc: s.ts,
            ts: s.ts * 1000 + MSGOFF,
            buy: s.dir.eq_ignore_ascii_case("BUY"),
            zlo: s.lo.min(s.hi),
            zhi: s.lo.max(s.hi),
            sl: s.sl,
            tps: s.tps.clone(),
            limit: s.limit,
            tp_open: s.tp_open,
            high_risk: s.tag_high_risk,
            may_not: s.tag_may_not,
            first_entry: s.tag_first_entry,
            edited: s.edited.is_some(),
        })
        .collect();
    eprintln!(
        "sygnały: {} (z {} w pliku)",
        sygnaly.len(),
        plik.signals.len()
    );

    // ---------- godzinowa historia ceny (do średniej 72 h) ----------
    eprintln!("buduję historię godzinową…");
    let mut godz: Vec<(i64, f64)> = Vec::new();
    {
        let mut cur = i64::MIN;
        for i in 0..t.len() {
            let ts = t.ts(i);
            let h = ts.div_euclid(H);
            if h != cur {
                if cur != i64::MIN {
                    let last = godz.last_mut().unwrap();
                    *last = (cur * H, last.1);
                }
                cur = h;
                godz.push((h * H, (t.bid(i) + t.ask(i)) * 0.5));
            } else {
                godz.last_mut().unwrap().1 = (t.bid(i) + t.ask(i)) * 0.5;
            }
        }
    }

    // ---------- skany ----------
    let skan = |opoznienie: i64, dobowe: bool| -> Vec<Option<Analiza>> {
        sygnaly
            .par_iter()
            .map(|s| analizuj(&t, s, okno_wejscia, horyzont, opoznienie, dobowe))
            .collect()
    };

    eprintln!("skan A: pełny horyzont…");
    let mut a_full = skan(0, false);
    eprintln!("skan B: cięcie dobowe…");
    let a_daily = skan(0, true);
    eprintln!("skan C: opóźnienie 250 ms…");
    let a_lat = skan(250, false);
    eprintln!("skan D: opóźnienie 250 ms + cięcie dobowe…");
    let a_lat_daily = skan(250, true);

    // ---------- cechy obserwowalne ----------
    {
        // wynik normalnej gry per sygnał (do „poprzedniego wyniku")
        let norm: Vec<f64> = a_full
            .iter()
            .map(|o| o.as_ref().and_then(|x| x.w1_pts()).unwrap_or(0.0))
            .collect();
        for (i, o) in a_full.iter_mut().enumerate() {
            let Some(an) = o.as_mut() else { continue };
            let s = an.s.clone();
            let ts = s.ts;
            // zmienność 60 minut przed sygnałem
            let ia = t.index_at(ts - 60 * 60_000);
            let ib = t.index_at(ts).min(t.len());
            let mut mn = f64::INFINITY;
            let mut mx = f64::NEG_INFINITY;
            for k in ia..ib {
                let b = t.bid(k);
                if b < mn {
                    mn = b;
                }
                if b > mx {
                    mx = b;
                }
            }
            let zm = if mx.is_finite() && mn.is_finite() {
                mx - mn
            } else {
                0.0
            };
            // średnia 72-godzinna
            let poz = godz.partition_point(|(h, _)| *h < ts);
            let ma = if poz >= 72 {
                godz[poz - 72..poz].iter().map(|(_, p)| *p).sum::<f64>() / 72.0
            } else if poz > 0 {
                godz[..poz].iter().map(|(_, p)| *p).sum::<f64>() / poz as f64
            } else {
                an.px0
            };
            let kontra = if s.buy { an.px0 < ma } else { an.px0 > ma };
            let dyst_sl = if s.buy { s.zlo - s.sl } else { s.sl - s.zhi };
            let dyst_tp1 = if s.buy {
                s.tps[0] - s.zhi
            } else {
                s.zlo - s.tps[0]
            };
            let dyst_str = if s.buy {
                an.px0 - s.zhi
            } else {
                s.zlo - an.px0
            };
            let ile24 = sygnaly
                .iter()
                .filter(|x| x.ts < ts && x.ts >= ts - D)
                .count() as i64;
            an.cechy = Cechy {
                godzina: ts.div_euclid(H).rem_euclid(24),
                dzien_tyg: (ts.div_euclid(D) + 4).rem_euclid(7),
                szer_strefy: s.zhi - s.zlo,
                dyst_sl,
                dyst_tp1,
                r_mult: if dyst_sl.abs() > 1e-9 {
                    dyst_tp1 / dyst_sl
                } else {
                    0.0
                },
                zmiennosc_60m: zm,
                nad_ma72: an.px0 - ma,
                kontra_ma: kontra,
                dyst_do_strefy: dyst_str,
                sygnalow_24h: ile24,
                poprz_wynik: if i > 0 { norm[i - 1] } else { 0.0 },
            };
        }
    }

    // ---------- budowa planów wariantów ----------
    // L0 — ODNIESIENIE. Bierz każdy sygnał, który dał się wypełnić; wyjście
    // TP1 albo SL, co pierwsze. Zero wiedzy o przyszłości. To jest zero na
    // skali: tyle daje ślepe kopiowanie kanału.
    let plan_l0 = |aa: &[Option<Analiza>]| -> Vec<Plan> {
        aa.iter()
            .filter_map(|o| {
                let an = o.as_ref()?;
                let (tin, pin, tout, pout, _) = an.w1_surowy()?;
                Some(Plan {
                    sid: an.s.id,
                    buy: an.s.buy,
                    t_in: tin,
                    px_in: pin,
                    t_out: tout,
                    px_out: pout,
                })
            })
            .collect()
    };
    // L2 — IDEALNE WYJŚCIE, BEZ SELEKCJI. Wejście normalne, wyjście w szczycie
    // korzystnego wychylenia. Sygnały, które poszły prosto w stop (szczyt ≤ 0),
    // NIE są pomijane — kończą stratą na stopie. Bez tego „idealne wyjście"
    // po cichu niosłoby też selekcję i teza „selekcja nic nie dodaje" byłaby
    // sprawdzana sama na sobie.
    let plan_l2 = |aa: &[Option<Analiza>]| -> Vec<Plan> {
        aa.iter()
            .filter_map(|o| {
                let an = o.as_ref()?;
                if !an.strefa_dotknieta {
                    return None;
                }
                if an.wn.mfe > 0.0 && an.wn.mfe_ts != NIGDY {
                    Some(Plan {
                        sid: an.s.id,
                        buy: an.s.buy,
                        t_in: an.t_in_n,
                        px_in: an.px_in_n,
                        t_out: an.wn.mfe_ts,
                        px_out: an.wn.mfe_px,
                    })
                } else {
                    let (tin, pin, tout, pout, _) = an.w1_surowy()?;
                    Some(Plan {
                        sid: an.s.id,
                        buy: an.s.buy,
                        t_in: tin,
                        px_in: pin,
                        t_out: tout,
                        px_out: pout,
                    })
                }
            })
            .collect()
    };
    let plan_w1 = |aa: &[Option<Analiza>]| -> Vec<Plan> {
        aa.iter()
            .filter_map(|o| {
                let an = o.as_ref()?;
                let (tin, pin, tout, pout, _) = an.w1_surowy()?;
                if an.s.pts(pin, pout) <= 0.0 {
                    return None; // DOSKONAŁA SELEKCJA
                }
                Some(Plan {
                    sid: an.s.id,
                    buy: an.s.buy,
                    t_in: tin,
                    px_in: pin,
                    t_out: tout,
                    px_out: pout,
                })
            })
            .collect()
    };
    let plan_w2 = |aa: &[Option<Analiza>]| -> Vec<Plan> {
        aa.iter()
            .filter_map(|o| {
                let an = o.as_ref()?;
                if !an.strefa_dotknieta || an.wn.mfe <= 0.0 || an.wn.mfe_ts == NIGDY {
                    return None;
                }
                Some(Plan {
                    sid: an.s.id,
                    buy: an.s.buy,
                    t_in: an.t_in_n,
                    px_in: an.px_in_n,
                    t_out: an.wn.mfe_ts,
                    px_out: an.wn.mfe_px,
                })
            })
            .collect()
    };
    let plan_w3 = |aa: &[Option<Analiza>]| -> Vec<Plan> {
        aa.iter()
            .filter_map(|o| {
                let an = o.as_ref()?;
                let (ti, pi, to, po) = an.para?;
                Some(Plan {
                    sid: an.s.id,
                    buy: an.s.buy,
                    t_in: ti,
                    px_in: pi,
                    t_out: to,
                    px_out: po,
                })
            })
            .collect()
    };

    let t_first = t.first_ts();
    let t_last = t.last_ts();
    let od_czerwca = dni_od_ymd(2026, 6, 1) * D;
    let ost_mies = t_last - 30 * D;
    let okna: Vec<(&str, i64, i64)> = vec![
        ("całość", t_first, t_last + 1),
        ("od 1 czerwca", od_czerwca, t_last + 1),
        ("ostatni miesiąc", ost_mies, t_last + 1),
    ];

    let mut wiersze: Vec<Wiersz> = Vec::new();
    eprintln!("symulacje wariantów…");
    // DRABINA. Nazwy zgodne z zadaniem: L0 odniesienie, L1 selekcja,
    // L2 wyjście (bez selekcji), L2+L1 wyjście i selekcja razem,
    // L3 pełna wyrocznia, L4 wyrocznia realistyczna.
    for (nazwa, planer, aa_f, aa_d, maxp, tryb_pct) in [
        (
            "L0",
            0usize,
            &a_full,
            &a_daily,
            usize::MAX,
            TrybLotu::Procent,
        ),
        ("L1", 1, &a_full, &a_daily, usize::MAX, TrybLotu::Procent),
        ("L2", 2, &a_full, &a_daily, usize::MAX, TrybLotu::Procent),
        ("L2+L1", 4, &a_full, &a_daily, usize::MAX, TrybLotu::Procent),
        ("L3", 3, &a_full, &a_daily, usize::MAX, TrybLotu::Procent),
        ("L4", 3, &a_lat, &a_lat_daily, 9usize, TrybLotu::ProcentReal),
    ] {
        let wybierz = |aa: &[Option<Analiza>]| -> Vec<Plan> {
            match planer {
                0 => plan_l0(aa),
                1 => plan_w1(aa),
                2 => plan_l2(aa),
                4 => plan_w2(aa),
                _ => plan_w3(aa),
            }
        };
        let pf = wybierz(aa_f);
        let pd = wybierz(aa_d);
        for (on, od, do_) in &okna {
            let s0 = symuluj(&t, &pf, *od, *do_, false, maxp, TrybLotu::Staly);
            wiersze.push(wiersz(nazwa, on, "lot stały 0,03", &s0));
            let c = symuluj(&t, &pf, *od, *do_, false, maxp, tryb_pct);
            wiersze.push(wiersz(nazwa, on, "compounding", &c));
            let dd = symuluj(&t, &pd, *od, *do_, true, maxp, tryb_pct);
            wiersze.push(wiersz(nazwa, on, "dzień-po-dniu", &dd));
        }
    }

    // ---------- ETYKIETY ----------
    eprintln!("zapisuję etykiety…");
    std::fs::create_dir_all(&a.out)?;
    let etyk: Vec<Etykieta> = a_full
        .iter()
        .filter_map(|o| o.as_ref())
        .map(|an| {
            let s = &an.s;
            let (wn_ts, wn_px, wn_pow) = if an.strefa_dotknieta {
                let (a1, b1, c1) = an.wn.normalne(s);
                (Some(a1), Some(b1), c1.to_string())
            } else {
                (None, None, "BRAK WEJŚCIA".to_string())
            };
            let w1p = an.w1_pts();
            Etykieta {
                id: s.id,
                ts_utc: s.ts_utc,
                ts_serwer_ms: s.ts,
                data: fmt_ts(s.ts),
                dir: if s.buy { "BUY".into() } else { "SELL".into() },
                limit: s.limit,
                tp_open: s.tp_open,
                lo: s.zlo,
                hi: s.zhi,
                sl: s.sl,
                tps: s.tps.clone(),
                tag_high_risk: s.high_risk,
                tag_may_not: s.may_not,
                tag_first_entry: s.first_entry,
                edited: s.edited,
                cena_w_chwili_sygnalu: an.px0,
                strefa_dotknieta: an.strefa_dotknieta,
                sl_naruszony_przed_wejsciem: an.sl_pre_ts != NIGDY
                    && (an.t_in_n == NIGDY || an.sl_pre_ts < an.t_in_n),
                wziety_w1: w1p.map(|v| v > 0.0).unwrap_or(false),
                wziety_w2: an.strefa_dotknieta && an.wn.mfe > 0.0,
                wziety_w3: an.para.is_some(),
                wejscie_normalne_ts: opt_ts(an.t_in_n),
                wejscie_normalne_cena: if an.strefa_dotknieta {
                    Some(an.px_in_n)
                } else {
                    None
                },
                wejscie_idealne_ts: an.para.map(|p| p.0),
                wejscie_idealne_cena: an.para.map(|p| p.1),
                wyjscie_normalne_ts: wn_ts,
                wyjscie_normalne_cena: wn_px,
                wyjscie_normalne_powod: wn_pow,
                wyjscie_idealne_ts: an.para.map(|p| p.2),
                wyjscie_idealne_cena: an.para.map(|p| p.3),
                mfe_od_sygnalu: an.mfe_sig,
                mfe_od_sygnalu_ts: opt_ts(an.mfe_sig_ts),
                mae_od_sygnalu: an.mae_sig,
                mae_od_sygnalu_ts: opt_ts(an.mae_sig_ts),
                mfe_od_wejscia: if an.strefa_dotknieta { an.wn.mfe } else { 0.0 },
                mfe_od_wejscia_pelne: if an.strefa_dotknieta {
                    an.wn.mfe_raw
                } else {
                    0.0
                },
                mae_od_wejscia: if an.strefa_dotknieta { an.wn.mae } else { 0.0 },
                tp1_ts_po_wejsciu: if an.strefa_dotknieta {
                    opt_ts(an.wn.tp_ts[0])
                } else {
                    None
                },
                tp2_ts_po_wejsciu: if an.strefa_dotknieta {
                    opt_ts(an.wn.tp_ts[1])
                } else {
                    None
                },
                tp3_ts_po_wejsciu: if an.strefa_dotknieta {
                    opt_ts(an.wn.tp_ts[2])
                } else {
                    None
                },
                sl_ts_po_wejsciu: if an.strefa_dotknieta {
                    opt_ts(an.wn.sl_ts)
                } else {
                    None
                },
                pol_bez_celu_pts: if an.strefa_dotknieta {
                    Some(an.wn.bez_celu)
                } else {
                    None
                },
                pol_czas_pts: if an.strefa_dotknieta {
                    Some(an.wn.czas.to_vec())
                } else {
                    None
                },
                tp1_osiagniety: an.tp_ts_sig[0] != NIGDY,
                tp1_ts: opt_ts(an.tp_ts_sig[0]),
                tp2_osiagniety: an.tp_ts_sig[1] != NIGDY,
                tp2_ts: opt_ts(an.tp_ts_sig[1]),
                tp3_osiagniety: an.tp_ts_sig[2] != NIGDY,
                tp3_ts: opt_ts(an.tp_ts_sig[2]),
                sl_osiagniety: an.sl_ts_sig != NIGDY,
                sl_ts: opt_ts(an.sl_ts_sig),
                wynik_normalny_pts: w1p,
                wynik_w2_pts: if an.strefa_dotknieta && an.wn.mfe > 0.0 {
                    Some(an.wn.mfe)
                } else {
                    None
                },
                wynik_w3_pts: an.para.map(|p| an.s.pts(p.1, p.3)),
                cechy: an.cechy.clone(),
            }
        })
        .collect();
    let p_etyk = a.out.join("etykiety_wyroczni.json");
    std::fs::write(&p_etyk, serde_json::to_string_pretty(&etyk)?)?;
    eprintln!("→ {}", p_etyk.display());

    // ============================================================
    //  ROZLICZENIE LUKI WOBEC CZEMPIONA
    // ============================================================
    let mut luka = serde_json::Map::new();
    let mut ostrze_sig: HashMap<i64, (f64, f64, i64, f64)> = HashMap::new(); // sid → (usd, vol, t_in, vwap)
                                                                             // Nazwa czempiona bierze się z presetu podanego w `--preset`, żeby raport
                                                                             // nie kłamał, gdy odniesieniem jest RUNNER, a nie OSTRZE.
    let czempion: String = a
        .preset
        .file_stem()
        .map(|x| x.to_string_lossy().to_string())
        .unwrap_or_else(|| "CZEMPION".into());
    if !a.bez_ostrza {
        eprintln!("przebieg odniesienia: {czempion}…");
        let ptxt = std::fs::read_to_string(&a.preset)?;
        let preset: Preset = serde_json::from_str(&ptxt)?;
        let msgs = load_messages(&a.signals)?;

        // wersja o STAŁYM locie — do rozliczenia luki w dolarach bez efektu
        // rosnącej pozycji
        let mut st = preset.settings.clone();
        st.lot_mode_percent = false;
        st.lot_fixed = 0.01;
        // Dziennik JEST tu potrzebny. `baskets_dump` niesie tylko koszyki żywe
        // na koniec przebiegu (silnik przycina stare, `baskets.retain`), więc
        // mapowanie transakcja → sygnał zbudowane z niego gubi 3/4 handlu.
        // Zdarzenie `basket_created` wiąże `basket_id` z `msg_id` na trwałe.
        st.journal_enabled = true;
        st.journal_text_mirror = false;
        st.journal_snapshots = false;
        st.journal_excursions = false;
        // Dziennik to plik ROBOCZY tego narzędzia (rzędu gigabajta) —
        // kasowany od razu po zbudowaniu mapy koszyk→sygnał.
        let dz = a.out.join("_dziennik_tmp.jsonl");
        let cfg = RunConfig {
            from: t_first,
            to: t_last + 1,
            start_balance: START,
            settings: st,
            formaty: Vec::new(),
            pulapy: Default::default(),
            rozgrzewka_h: 0,
            daily_reset: false,
            source_name: "ATFX VIP SIGNALS".into(),
            curve_interval_ms: 60_000,
            journal_path: Some(dz.clone()),
            // Reszta pól z domyślnych — domyślne wartości są ścieżką parytetu
            // (pusta drabinka = zero nowego kodu w pętli), więc dopisanie pola
            // do RunConfig nie może tu niczego zmienić po cichu.
            ..Default::default()
        };
        let r = run(&t, &msgs, &cfg);
        let r = require_complete_reference(r, &czempion, "lot stały 0.01 / rozliczenie luki")?;
        eprintln!(
            "{czempion} (lot stały 0.01): {:+.2} $, PF {:.2}, {} transakcji",
            r.metrics.total_profit, r.metrics.profit_factor, r.metrics.trades
        );
        let mut b2m: HashMap<u32, i64> = HashMap::new();
        for b in &r.baskets_dump {
            b2m.insert(b.id, b.msg_id);
        }
        // mapowanie z dziennika — kompletne, bo zapisane w chwili powstania koszyka
        if let Ok(f) = std::fs::File::open(&dz) {
            use std::io::BufRead;
            let rd = std::io::BufReader::with_capacity(1 << 20, f);
            for l in rd.lines().map_while(|x| x.ok()) {
                if !l.contains("basket_created") {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&l) else {
                    continue;
                };
                if v.get("kind").and_then(|x| x.as_str()) != Some("basket_created") {
                    continue;
                }
                let (Some(bid), Some(mid)) = (
                    v.get("basket_id").and_then(|x| x.as_u64()),
                    v.get("msg_id").and_then(|x| x.as_i64()),
                ) else {
                    continue;
                };
                b2m.insert(bid as u32, mid);
            }
        }
        eprintln!("mapowanie koszyk→sygnał: {} pozycji", b2m.len());
        let _ = std::fs::remove_file(&dz);
        luka.insert(
            "czempion_mediana_trzymania_min".into(),
            serde_json::json!(r.metrics.median_hold_min),
        );
        luka.insert(
            "czempion_srednia_trzymania_min".into(),
            serde_json::json!(r.metrics.avg_hold_min),
        );
        for tr in &r.trades {
            let Some(bid) = tr.basket else { continue };
            let Some(mid) = b2m.get(&bid) else { continue };
            let e = ostrze_sig.entry(*mid).or_insert((0.0, 0.0, i64::MAX, 0.0));
            e.0 += tr.profit;
            e.1 += tr.volume;
            e.2 = e.2.min(tr.open_ts);
            e.3 += tr.open_price * tr.volume;
        }
        for v in ostrze_sig.values_mut() {
            if v.1 > 0.0 {
                v.3 /= v.1;
            }
        }
        luka.insert(
            "czempion_lot_staly_zysk".into(),
            serde_json::json!(r.metrics.total_profit),
        );
        luka.insert(
            "czempion_lot_staly_pf".into(),
            serde_json::json!(r.metrics.profit_factor),
        );
        luka.insert(
            "czempion_transakcje".into(),
            serde_json::json!(r.metrics.trades),
        );
        luka.insert(
            "czempion_sygnaly_wziete".into(),
            serde_json::json!(ostrze_sig.len()),
        );

        // pełne przebiegi odniesienia dla trzech okien × dwa tryby
        for (on, od, do_) in &okna {
            for tryb in [false, true] {
                let mut c2 = RunConfig {
                    from: *od,
                    to: *do_,
                    start_balance: START,
                    settings: preset.settings.clone(),
                    formaty: Vec::new(),
                    pulapy: Default::default(),
                    rozgrzewka_h: 0,
                    daily_reset: tryb,
                    source_name: "ATFX VIP SIGNALS".into(),
                    curve_interval_ms: 60_000,
                    journal_path: None,
                    // jak wyżej: domyślne = ścieżka parytetu
                    ..Default::default()
                };
                c2.settings.journal_enabled = false;
                let rr = run(&t, &msgs, &c2);
                let rr = require_complete_reference(rr, &czempion, &format!(
                    "okno {on} / {}", if tryb { "dzień-po-dniu" } else { "compounding" }
                ))?;
                wiersze.push(Wiersz {
                    wariant: czempion.clone(),
                    okno: (*on).into(),
                    tryb: if tryb {
                        "dzień-po-dniu".into()
                    } else {
                        "compounding".into()
                    },
                    zysk: rr.metrics.total_profit,
                    koncowe: rr.metrics.end_equity,
                    pf: if rr.metrics.profit_factor.is_finite() {
                        rr.metrics.profit_factor
                    } else {
                        999.0
                    },
                    max_dd_usd: rr.metrics.max_dd_abs,
                    max_dd_pct: rr.metrics.max_dd_pct,
                    dni_plus_pct: rr.metrics.win_days_pct,
                    dni_handlowe: rr.metrics.trading_days,
                    transakcje: rr.metrics.trades,
                    srednio_na_dzien: rr.metrics.avg_per_day,
                    win_rate: rr.metrics.win_rate,
                });
            }
        }

        // ---------- drabina L0 … L4 ----------
        let idx: HashMap<i64, usize> = sygnaly.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        let mut l0 = 0.0; // OSTRZE tak, jak zagrał
        let mut l1 = 0.0; // + doskonała selekcja
        let mut l2 = 0.0; // + doskonałe wyjście (z wejścia OSTRZA)
        let mut l3 = 0.0; // + doskonałe wejście
        let mut n_zlych = 0usize;
        let mut szczegoly: Vec<serde_json::Value> = Vec::new();

        for (mid, (usd, vol, tin, vwap)) in &ostrze_sig {
            l0 += *usd;
            if *usd > 0.0 {
                l1 += *usd;
            } else {
                n_zlych += 1;
            }
            let Some(i) = idx.get(mid) else { continue };
            let Some(an) = a_full[*i].as_ref() else {
                continue;
            };
            let s = &an.s;
            // doskonałe wyjście z wejścia OSTRZA
            let i_in = t.index_at(*tin);
            let w = skan_wyjscia(&t, s, i_in, *vwap, *tin + horyzont);
            let usd2 = (w.mfe * XAU_CONTRACT * *vol).max(0.0);
            l2 += usd2;
            // doskonałe wejście + doskonałe wyjście, ten sam wolumen
            let usd3 = match an.para {
                Some(p) => (an.s.pts(p.1, p.3) * XAU_CONTRACT * *vol).max(usd2),
                None => usd2,
            };
            l3 += usd3;
            szczegoly.push(serde_json::json!({
                "id": mid, "usd_czempion": usd, "vol": vol,
                "usd_idealne_wyjscie": usd2, "usd_idealne_wejscie_i_wyjscie": usd3,
            }));
        }

        // L4: sygnały, których OSTRZE w ogóle nie zagrał. Rozdzielone na dwie
        // przyczyny, bo to zupełnie różne problemy do naprawy:
        //   * FILTR — silnik nawet nie założył koszyka (godziny, reżim, limity),
        //   * BRAK WYPEŁNIENIA — koszyk był, ale zlecenia nigdy się nie zafillowały
        //     (strefa zawężona o 2 $, siatka w głąb o 3 $, cena nie wróciła).
        let koszyki: std::collections::HashSet<i64> = b2m.values().cloned().collect();
        let mut l4_dod_sufit = 0.0;
        let mut l4_dod_realnie = 0.0;
        let mut n_pominietych = 0usize;
        let mut n_pominietych_zysk = 0usize;
        let mut filtr_n = 0usize;
        let mut filtr_sufit = 0.0;
        let mut filtr_realnie = 0.0;
        let mut nofill_n = 0usize;
        let mut nofill_sufit = 0.0;
        let mut nofill_realnie = 0.0;
        for (i, s) in sygnaly.iter().enumerate() {
            if ostrze_sig.contains_key(&s.id) {
                continue;
            }
            n_pominietych += 1;
            let byl_koszyk = koszyki.contains(&s.id);
            if byl_koszyk {
                nofill_n += 1;
            } else {
                filtr_n += 1;
            }
            let Some(an) = a_full[i].as_ref() else {
                continue;
            };
            if let Some(p) = an.para {
                let u = an.s.pts(p.1, p.3) * XAU_CONTRACT * VOL_REF;
                l4_dod_sufit += u;
                if byl_koszyk {
                    nofill_sufit += u;
                } else {
                    filtr_sufit += u;
                }
            }
            if let Some(p) = an.w1_pts() {
                if p > 0.0 {
                    let u = p * XAU_CONTRACT * VOL_REF;
                    l4_dod_realnie += u;
                    n_pominietych_zysk += 1;
                    if byl_koszyk {
                        nofill_realnie += u;
                    } else {
                        filtr_realnie += u;
                    }
                }
            }
        }
        let l4 = l3 + l4_dod_sufit;
        luka.insert(
            "czempion_koszykow_sygnalow".into(),
            serde_json::json!(koszyki.len()),
        );
        luka.insert("filtr_liczba".into(), serde_json::json!(filtr_n));
        luka.insert("filtr_sufit".into(), serde_json::json!(filtr_sufit));
        luka.insert("filtr_realnie".into(), serde_json::json!(filtr_realnie));
        luka.insert("brak_fill_liczba".into(), serde_json::json!(nofill_n));
        luka.insert("brak_fill_sufit".into(), serde_json::json!(nofill_sufit));
        luka.insert(
            "brak_fill_realnie".into(),
            serde_json::json!(nofill_realnie),
        );

        luka.insert("L0_czempion".into(), serde_json::json!(l0));
        luka.insert("L1_selekcja".into(), serde_json::json!(l1));
        luka.insert("L2_selekcja_wyjscie".into(), serde_json::json!(l2));
        luka.insert("L3_selekcja_wejscie_wyjscie".into(), serde_json::json!(l3));
        luka.insert("L4_plus_pominiete".into(), serde_json::json!(l4));
        luka.insert("skladnik_zle_sygnaly".into(), serde_json::json!(l1 - l0));
        luka.insert("skladnik_gorsze_wyjscie".into(), serde_json::json!(l2 - l1));
        luka.insert("skladnik_gorsze_wejscie".into(), serde_json::json!(l3 - l2));
        luka.insert(
            "skladnik_filtry_sufit".into(),
            serde_json::json!(l4_dod_sufit),
        );
        luka.insert(
            "skladnik_filtry_realnie".into(),
            serde_json::json!(l4_dod_realnie),
        );
        luka.insert("liczba_zlych_sygnalow".into(), serde_json::json!(n_zlych));
        luka.insert(
            "liczba_pominietych".into(),
            serde_json::json!(n_pominietych),
        );
        luka.insert(
            "liczba_pominietych_zyskownych".into(),
            serde_json::json!(n_pominietych_zysk),
        );
        luka.insert("szczegoly".into(), serde_json::json!(szczegoly));
    }

    // ============================================================
    //  CO DA SIĘ ODZYSKAĆ BEZ ZNAJOMOŚCI PRZYSZŁOŚCI
    // ============================================================
    eprintln!("analiza odzyskiwalności…");
    let granica = {
        let mut v: Vec<i64> = a_full
            .iter()
            .filter_map(|o| o.as_ref())
            .map(|x| x.s.ts)
            .collect();
        v.sort_unstable();
        v[v.len() / 2]
    };

    // wartość normalnej gry per sygnał w dolarach przy wolumenie odniesienia
    let baza: Vec<(i64, f64, &Analiza)> = a_full
        .iter()
        .filter_map(|o| o.as_ref())
        .filter_map(|an| {
            an.w1_pts()
                .map(|p| (an.s.ts, p * XAU_CONTRACT * VOL_REF, an))
        })
        .collect();

    let mut cechy_out: Vec<WynikCechy> = Vec::new();
    let kub_kwantyl = |vals: &[f64], n: usize| -> Vec<f64> {
        let mut v = vals.to_vec();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (1..n).map(|k| v[v.len() * k / n]).collect()
    };
    let ind = |progi: &[f64], x: f64| -> usize { progi.iter().filter(|p| x >= **p).count() };

    // --- cechy kategoryczne ---
    cechy_out.push(ocen_cehe(
        "godzina serwera",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, an.cechy.godzina as usize, *v))
            .collect::<Vec<_>>(),
        24,
        granica,
    ));
    cechy_out.push(ocen_cehe(
        "dzień tygodnia",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, an.cechy.dzien_tyg as usize, *v))
            .collect::<Vec<_>>(),
        7,
        granica,
    ));
    cechy_out.push(ocen_cehe(
        "kierunek",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, if an.s.buy { 0 } else { 1 }, *v))
            .collect::<Vec<_>>(),
        2,
        granica,
    ));
    cechy_out.push(ocen_cehe(
        "limit vs rynek",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, if an.s.limit { 0 } else { 1 }, *v))
            .collect::<Vec<_>>(),
        2,
        granica,
    ));
    cechy_out.push(ocen_cehe(
        "HIGH RISK",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, an.s.high_risk as usize, *v))
            .collect::<Vec<_>>(),
        2,
        granica,
    ));
    cechy_out.push(ocen_cehe(
        "MAY NOT BE AROUND",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, an.s.may_not as usize, *v))
            .collect::<Vec<_>>(),
        2,
        granica,
    ));
    cechy_out.push(ocen_cehe(
        "FIRST ENTRY",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, an.s.first_entry as usize, *v))
            .collect::<Vec<_>>(),
        2,
        granica,
    ));
    cechy_out.push(ocen_cehe(
        "sygnał edytowany",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, an.s.edited as usize, *v))
            .collect::<Vec<_>>(),
        2,
        granica,
    ));
    cechy_out.push(ocen_cehe(
        "kontra-trend MA72",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, an.cechy.kontra_ma as usize, *v))
            .collect::<Vec<_>>(),
        2,
        granica,
    ));
    cechy_out.push(ocen_cehe(
        "TP OPEN",
        &baza
            .iter()
            .map(|(ts, v, an)| (*ts, an.s.tp_open as usize, *v))
            .collect::<Vec<_>>(),
        2,
        granica,
    ));

    // --- cechy ciągłe, kwantylowe kubełki po 5 ---
    let ciagle: Vec<(&str, Box<dyn Fn(&Analiza) -> f64>)> = vec![
        (
            "szerokość strefy",
            Box::new(|a: &Analiza| a.cechy.szer_strefy),
        ),
        ("dystans SL", Box::new(|a: &Analiza| a.cechy.dyst_sl)),
        ("dystans TP1", Box::new(|a: &Analiza| a.cechy.dyst_tp1)),
        ("R = TP1/SL", Box::new(|a: &Analiza| a.cechy.r_mult)),
        (
            "zmienność 60 min",
            Box::new(|a: &Analiza| a.cechy.zmiennosc_60m),
        ),
        (
            "odchylenie od MA72",
            Box::new(|a: &Analiza| a.cechy.nad_ma72),
        ),
        (
            "dystans do strefy",
            Box::new(|a: &Analiza| a.cechy.dyst_do_strefy),
        ),
        (
            "sygnałów w 24 h",
            Box::new(|a: &Analiza| a.cechy.sygnalow_24h as f64),
        ),
        (
            "wynik poprzedniego",
            Box::new(|a: &Analiza| a.cechy.poprz_wynik),
        ),
    ];
    for (nazwa, f) in &ciagle {
        let vals: Vec<f64> = baza.iter().map(|(_, _, an)| f(an)).collect();
        let progi = kub_kwantyl(&vals, 5);
        let dane: Vec<(i64, usize, f64)> = baza
            .iter()
            .map(|(ts, v, an)| (*ts, ind(&progi, f(an)).min(4), *v))
            .collect();
        cechy_out.push(ocen_cehe(nazwa, &dane, 5, granica));
    }

    // --- sweep stałego celu (odzyskiwalność „lepszego wyjścia") ---
    #[derive(Serialize)]
    struct SweepTp {
        cel_usd: f64,
        calosc: f64,
        pierwsza_polowa: f64,
        druga_polowa: f64,
        trafien: usize,
    }
    let mut sweep_tp: Vec<SweepTp> = Vec::new();
    for k in 1..=40 {
        let cel = k as f64;
        let mut c = 0.0;
        let mut p1 = 0.0;
        let mut p2 = 0.0;
        let mut hit = 0usize;
        for an in a_full.iter().filter_map(|o| o.as_ref()) {
            if !an.strefa_dotknieta {
                continue;
            }
            let s = &an.s;
            // czy cel osiągnięty przed SL?
            let osiagniety = an.wn.mfe >= cel;
            let pts = if osiagniety {
                hit += 1;
                cel
            } else {
                let (_, px, _) = an.wn.normalne(s);
                // gdy cel nie padł, wychodzimy tak jak normalnie (SL/horyzont),
                // ale bez TP1 — więc bierzemy SL albo cenę końcową
                if an.wn.sl_ts != NIGDY {
                    s.pts(an.px_in_n, an.wn.sl_fill)
                } else {
                    let _ = px;
                    s.pts(an.px_in_n, an.wn.last_px)
                }
            };
            let usd = pts * XAU_CONTRACT * VOL_REF;
            c += usd;
            if s.ts < granica {
                p1 += usd;
            } else {
                p2 += usd;
            }
        }
        sweep_tp.push(SweepTp {
            cel_usd: cel,
            calosc: c,
            pierwsza_polowa: p1,
            druga_polowa: p2,
            trafien: hit,
        });
    }

    // --- sweep głębszego wejścia (odzyskiwalność „lepszego wejścia") ---
    #[derive(Serialize)]
    struct SweepWe {
        glebokosc_usd: f64,
        wypelnien: usize,
        calosc: f64,
        pierwsza_polowa: f64,
        druga_polowa: f64,
    }
    let mut sweep_we: Vec<SweepWe> = Vec::new();
    for k in 0..=16 {
        let gl = k as f64 * 0.5;
        let wyniki: Vec<(i64, f64, bool)> = sygnaly
            .par_iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let an = a_full[i].as_ref()?;
                let cel = if s.buy { s.zlo - gl } else { s.zhi + gl };
                // szukamy wypełnienia w oknie wejścia, przed naruszeniem SL
                let i0 = t.index_at(s.ts);
                let kon = s.ts + okno_wejscia;
                let mut j = i0;
                let mut fill: Option<(i64, f64, usize)> = None;
                while j < t.len() {
                    let ts = t.ts(j);
                    if ts > kon {
                        break;
                    }
                    let ep = s.cena_wejscia(t.bid(j), t.ask(j));
                    let xe = s.cena_wyjscia(t.bid(j), t.ask(j));
                    let ok = if s.buy { ep <= cel } else { ep >= cel };
                    if ok {
                        fill = Some((ts, ep, j));
                        break;
                    }
                    let sl_hit = if s.buy { xe <= s.sl } else { xe >= s.sl };
                    if sl_hit {
                        break;
                    }
                    j += 1;
                }
                let (tin, pin, iin) = fill?;
                let w = skan_wyjscia(&t, s, iin, pin, tin + horyzont);
                let (_, px, _) = w.normalne(s);
                let _ = an;
                Some((s.ts, s.pts(pin, px) * XAU_CONTRACT * VOL_REF, true))
            })
            .collect();
        let c: f64 = wyniki.iter().map(|(_, v, _)| v).sum();
        let p1: f64 = wyniki
            .iter()
            .filter(|(ts, _, _)| *ts < granica)
            .map(|(_, v, _)| v)
            .sum();
        let p2: f64 = wyniki
            .iter()
            .filter(|(ts, _, _)| *ts >= granica)
            .map(|(_, v, _)| v)
            .sum();
        sweep_we.push(SweepWe {
            glebokosc_usd: gl,
            wypelnien: wyniki.len(),
            calosc: c,
            pierwsza_polowa: p1,
            druga_polowa: p2,
        });
    }

    #[derive(Serialize)]
    struct PolitykaOOS {
        polityka: String,
        /// najlepszy parametr wg pierwszej połowy i jego wynik na drugiej
        param_1p: String,
        wynik_2p: f64,
        param_2p: String,
        wynik_1p: f64,
        /// suma poza próbą — TA liczba się liczy
        oos: f64,
        /// najlepszy wynik na całości przy wyborze parametru po fakcie
        in_sample: f64,
        param_in_sample: String,
        /// pełna siatka: (parametr, pierwsza połowa, druga połowa, całość)
        siatka: Vec<(String, f64, f64, f64)>,
    }

    /// Wybiera wariant po jednej połowie, rozlicza na drugiej.
    let oceń_polityke = |nazwa: &str, opisy: &[String], dane: &[(i64, Vec<f64>)]| -> PolitykaOOS {
        let n = opisy.len();
        let mut s1 = vec![0.0; n];
        let mut s2 = vec![0.0; n];
        for (ts, v) in dane {
            for k in 0..n {
                if *ts < granica {
                    s1[k] += v[k];
                } else {
                    s2[k] += v[k];
                }
            }
        }
        let arg = |s: &[f64]| -> usize {
            let mut b = 0usize;
            for k in 1..n {
                if s[k] > s[b] {
                    b = k;
                }
            }
            b
        };
        let b1 = arg(&s1);
        let b2 = arg(&s2);
        let calosc: Vec<f64> = (0..n).map(|k| s1[k] + s2[k]).collect();
        let bc = arg(&calosc);
        PolitykaOOS {
            polityka: nazwa.into(),
            param_1p: opisy[b1].clone(),
            wynik_2p: s2[b1],
            param_2p: opisy[b2].clone(),
            wynik_1p: s1[b2],
            oos: s2[b1] + s1[b2],
            in_sample: calosc[bc],
            param_in_sample: opisy[bc].clone(),
            siatka: (0..n)
                .map(|k| (opisy[k].clone(), s1[k], s2[k], calosc[k]))
                .collect(),
        }
    };

    let mut polityki: Vec<PolitykaOOS> = Vec::new();
    {
        let wsad: Vec<&Analiza> = a_full
            .iter()
            .filter_map(|o| o.as_ref())
            .filter(|an| an.strefa_dotknieta)
            .collect();
        let usd = |pts: f64| pts * XAU_CONTRACT * VOL_REF;

        // 1) odniesienie bez parametru: TP1 albo SL
        let d: Vec<(i64, Vec<f64>)> = wsad
            .iter()
            .map(|an| {
                let (_, px, _) = an.wn.normalne(&an.s);
                (an.s.ts, vec![usd(an.s.pts(an.px_in_n, px))])
            })
            .collect();
        polityki.push(oceń_polityke(
            "TP1 albo SL (odniesienie)",
            &["—".to_string()],
            &d,
        ));

        // 2) bez celu zysku: tylko SL albo koniec horyzontu
        let d: Vec<(i64, Vec<f64>)> = wsad
            .iter()
            .map(|an| (an.s.ts, vec![usd(an.wn.bez_celu)]))
            .collect();
        polityki.push(oceń_polityke("bez celu, tylko SL", &["—".to_string()], &d));

        // 3) limit czasu trzymania
        let opisy: Vec<String> = CZASY_MIN.iter().map(|m| format!("{m} min")).collect();
        let d: Vec<(i64, Vec<f64>)> = wsad
            .iter()
            .map(|an| (an.s.ts, an.wn.czas.iter().map(|p| usd(*p)).collect()))
            .collect();
        polityki.push(oceń_polityke("limit czasu trzymania", &opisy, &d));

        // 4) zapadka blokująca procent szczytu
        let mut opisy: Vec<String> = Vec::new();
        for x in ZAP_PCT {
            for s in ZAP_START {
                opisy.push(format!("od {s} $, blokuj {x} %"));
            }
        }
        let d: Vec<(i64, Vec<f64>)> = wsad
            .iter()
            .map(|an| {
                let mut v = Vec::with_capacity(20);
                for xi in 0..ZAP_PCT.len() {
                    for si in 0..ZAP_START.len() {
                        v.push(usd(an.wn.zapadka[xi][si]));
                    }
                }
                (an.s.ts, v)
            })
            .collect();
        polityki.push(oceń_polityke("zapadka od szczytu", &opisy, &d));

        // 5) stały cel zysku w dolarach
        let opisy: Vec<String> = (1..=40).map(|k| format!("{k} $")).collect();
        let d: Vec<(i64, Vec<f64>)> = wsad
            .iter()
            .map(|an| {
                let v = (1..=40)
                    .map(|k| {
                        let cel = k as f64;
                        if an.wn.mfe >= cel {
                            usd(cel)
                        } else {
                            usd(an.wn.bez_celu)
                        }
                    })
                    .collect();
                (an.s.ts, v)
            })
            .collect();
        polityki.push(oceń_polityke("stały cel zysku", &opisy, &d));

        // 6) SUFIT: wyjście w szczycie (wyrocznia) — dla skali
        let d: Vec<(i64, Vec<f64>)> = wsad
            .iter()
            .map(|an| {
                let p = if an.wn.mfe > 0.0 {
                    an.wn.mfe
                } else {
                    an.wn.bez_celu
                };
                (an.s.ts, vec![usd(p)])
            })
            .collect();
        polityki.push(oceń_polityke(
            "SUFIT: szczyt (wyrocznia)",
            &["—".to_string()],
            &d,
        ));
    }

    // ============================================================
    //  CZY WINNY JEST CEL ZYSKU, CZY SZEROKOŚĆ STOPA?
    // ============================================================
    // Wcześniejsza analiza (STAN.md §2) orzekła, że „cel zysku niszczy
    // strategię": bez celu +429 $, wyjście po 240 min +581 $, wobec −164 $
    // dla TP1-albo-SL. Mierzono to jednak przy stopie rozszerzonym dwukrotnie.
    // Stop tego kanału jest bardzo ciasny (mediana 1 $ od krawędzi strefy),
    // więc „trzymaj dłużej bez celu" może po prostu oznaczać „daj się wynieść
    // na stopie". Mnożnik stopa musi być OSOBNĄ OSIĄ, a nie milczącym
    // założeniem — inaczej nie wiadomo, który z dwóch wniosków jest prawdziwy.
    #[derive(Serialize)]
    struct MnoznikSL {
        mnoznik: f64,
        tp1_albo_sl: f64,
        bez_celu: f64,
        czas_oos: f64,
        czas_param: String,
        zapadka_oos: f64,
        zapadka_param: String,
        sufit_szczyt: f64,
    }
    let mut mnozniki: Vec<MnoznikSL> = Vec::new();
    {
        let opisy_czas: Vec<String> = CZASY_MIN.iter().map(|m| format!("{m} min")).collect();
        let mut opisy_zap: Vec<String> = Vec::new();
        for x in ZAP_PCT {
            for s in ZAP_START {
                opisy_zap.push(format!("od {s} $, blokuj {x} %"));
            }
        }
        let usd = |pts: f64| pts * XAU_CONTRACT * VOL_REF;
        for mn in [1.0f64, 1.5, 2.0, 3.0, 4.0] {
            let dane: Vec<(i64, f64, f64, [f64; 8], [[f64; 5]; 4], f64)> = a_full
                .par_iter()
                .filter_map(|o| {
                    let an = o.as_ref()?;
                    if !an.strefa_dotknieta {
                        return None;
                    }
                    let mut s2 = an.s.clone();
                    // stop odsuwany od KRAWĘDZI STREFY o wielokrotność
                    // pierwotnego dystansu — tak, jak robi to `sl_min_dist`
                    let baza = if s2.buy {
                        s2.zlo - s2.sl
                    } else {
                        s2.sl - s2.zhi
                    };
                    s2.sl = if s2.buy {
                        s2.zlo - baza * mn
                    } else {
                        s2.zhi + baza * mn
                    };
                    let i_in = t.index_at(an.t_in_n);
                    let w = skan_wyjscia(&t, &s2, i_in, an.px_in_n, an.t_in_n + horyzont);
                    let (_, px, _) = w.normalne(&s2);
                    let szczyt = if w.mfe > 0.0 { w.mfe } else { w.bez_celu };
                    Some((
                        an.s.ts,
                        s2.pts(an.px_in_n, px),
                        w.bez_celu,
                        w.czas,
                        w.zapadka,
                        szczyt,
                    ))
                })
                .collect();

            let tp1: f64 = dane.iter().map(|x| usd(x.1)).sum();
            let bc: f64 = dane.iter().map(|x| usd(x.2)).sum();
            let suf: f64 = dane.iter().map(|x| usd(x.5)).sum();
            let d_czas: Vec<(i64, Vec<f64>)> = dane
                .iter()
                .map(|x| (x.0, x.3.iter().map(|p| usd(*p)).collect()))
                .collect();
            let o_czas = oceń_polityke("czas", &opisy_czas, &d_czas);
            let d_zap: Vec<(i64, Vec<f64>)> = dane
                .iter()
                .map(|x| {
                    let mut v = Vec::with_capacity(20);
                    for xi in 0..ZAP_PCT.len() {
                        for si in 0..ZAP_START.len() {
                            v.push(usd(x.4[xi][si]));
                        }
                    }
                    (x.0, v)
                })
                .collect();
            let o_zap = oceń_polityke("zapadka", &opisy_zap, &d_zap);
            mnozniki.push(MnoznikSL {
                mnoznik: mn,
                tp1_albo_sl: tp1,
                bez_celu: bc,
                czas_oos: o_czas.oos,
                czas_param: o_czas.param_1p.clone(),
                zapadka_oos: o_zap.oos,
                zapadka_param: o_zap.param_1p.clone(),
                sufit_szczyt: suf,
            });
        }
    }

    // --- WRAŻLIWOŚĆ NA HORYZONT I OKNO WEJŚCIA ---
    // Sufit W2/W3 to w dużej mierze funkcja tego, jak długo wolno trzymać.
    // Bez tej tabeli liczba „ile leży na stole" jest nieinterpretowalna.
    #[derive(Serialize)]
    struct Wrazliwosc {
        horyzont_h: f64,
        okno_wejscia_h: f64,
        sufit_w2_usd: f64,
        sufit_w3_usd: f64,
        wziete_w2: usize,
        wziete_w3: usize,
    }
    let mut wrazliwosc: Vec<Wrazliwosc> = Vec::new();
    for (hh, ww) in [
        (0.5, 4.0),
        (1.0, 4.0),
        (2.0, 4.0),
        (4.0, 4.0),
        (8.0, 4.0),
        (24.0, 4.0),
        (24.0, 0.5),
        (24.0, 1.0),
        (24.0, 2.0),
        (24.0, 8.0),
    ] {
        let hz = (hh * H as f64) as i64;
        let ow = (ww * H as f64) as i64;
        let res: Vec<(f64, f64)> = sygnaly
            .par_iter()
            .map(|s| {
                let an = analizuj(&t, s, ow, hz, 0, false);
                let w2 = an
                    .as_ref()
                    .filter(|x| x.strefa_dotknieta && x.wn.mfe > 0.0)
                    .map(|x| x.wn.mfe * XAU_CONTRACT * VOL_REF)
                    .unwrap_or(0.0);
                let w3 = an
                    .as_ref()
                    .and_then(|x| x.para.map(|p| x.s.pts(p.1, p.3) * XAU_CONTRACT * VOL_REF))
                    .unwrap_or(0.0);
                (w2, w3)
            })
            .collect();
        wrazliwosc.push(Wrazliwosc {
            horyzont_h: hh,
            okno_wejscia_h: ww,
            sufit_w2_usd: res.iter().map(|x| x.0).sum(),
            sufit_w3_usd: res.iter().map(|x| x.1).sum(),
            wziete_w2: res.iter().filter(|x| x.0 > 0.0).count(),
            wziete_w3: res.iter().filter(|x| x.1 > 0.0).count(),
        });
    }

    // --- rozkłady pomocnicze ---
    let mut roz = serde_json::Map::new();
    {
        let mut mfe: Vec<f64> = Vec::new();
        let mut mae: Vec<f64> = Vec::new();
        let mut zapas_we: Vec<f64> = Vec::new();
        let mut tp1_cel: Vec<f64> = Vec::new();
        for an in a_full.iter().filter_map(|o| o.as_ref()) {
            if !an.strefa_dotknieta {
                continue;
            }
            mfe.push(an.wn.mfe_raw);
            mae.push(an.wn.mae);
            if an.t_in_b != NIGDY {
                zapas_we.push(an.s.pts(an.px_in_n, an.px_in_b).abs());
            }
            tp1_cel.push(an.s.pts(an.px_in_n, an.s.tps[0]));
        }
        let kw = |v: &mut Vec<f64>, q: f64| -> f64 {
            if v.is_empty() {
                return 0.0;
            }
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[((v.len() - 1) as f64 * q) as usize]
        };
        roz.insert(
            "mfe_mediana".into(),
            serde_json::json!(kw(&mut mfe.clone(), 0.5)),
        );
        roz.insert(
            "mfe_p25".into(),
            serde_json::json!(kw(&mut mfe.clone(), 0.25)),
        );
        roz.insert(
            "mfe_p75".into(),
            serde_json::json!(kw(&mut mfe.clone(), 0.75)),
        );
        roz.insert(
            "mfe_p90".into(),
            serde_json::json!(kw(&mut mfe.clone(), 0.90)),
        );
        roz.insert(
            "mae_mediana".into(),
            serde_json::json!(kw(&mut mae.clone(), 0.5)),
        );
        roz.insert(
            "mae_p10".into(),
            serde_json::json!(kw(&mut mae.clone(), 0.10)),
        );
        roz.insert(
            "zapas_wejscia_mediana".into(),
            serde_json::json!(kw(&mut zapas_we.clone(), 0.5)),
        );
        roz.insert(
            "zapas_wejscia_p90".into(),
            serde_json::json!(kw(&mut zapas_we.clone(), 0.90)),
        );
        roz.insert(
            "zapas_wejscia_srednia".into(),
            serde_json::json!(zapas_we.iter().sum::<f64>() / zapas_we.len().max(1) as f64),
        );
        roz.insert(
            "tp1_dystans_mediana".into(),
            serde_json::json!(kw(&mut tp1_cel.clone(), 0.5)),
        );
    }

    // --- podstawowe liczniki ---
    let mut licz = serde_json::Map::new();
    {
        let n = a_full.iter().filter(|o| o.is_some()).count();
        let dotk = a_full
            .iter()
            .filter_map(|o| o.as_ref())
            .filter(|a| a.strefa_dotknieta)
            .count();
        let zysk = baza.iter().filter(|(_, v, _)| *v > 0.0).count();
        let strat = baza.iter().filter(|(_, v, _)| *v <= 0.0).count();
        let suma: f64 = baza.iter().map(|(_, v, _)| v).sum();
        let suma_plus: f64 = baza
            .iter()
            .filter(|(_, v, _)| *v > 0.0)
            .map(|(_, v, _)| v)
            .sum();
        let suma_minus: f64 = baza
            .iter()
            .filter(|(_, v, _)| *v <= 0.0)
            .map(|(_, v, _)| v)
            .sum();
        let sufit_w3: f64 = a_full
            .iter()
            .filter_map(|o| o.as_ref())
            .filter_map(|a| a.para.map(|p| a.s.pts(p.1, p.3) * XAU_CONTRACT * VOL_REF))
            .sum();
        let sufit_w2: f64 = a_full
            .iter()
            .filter_map(|o| o.as_ref())
            .filter(|a| a.strefa_dotknieta && a.wn.mfe > 0.0)
            .map(|a| a.wn.mfe * XAU_CONTRACT * VOL_REF)
            .sum();
        licz.insert("sygnalow".into(), serde_json::json!(n));
        licz.insert("strefa_dotknieta".into(), serde_json::json!(dotk));
        licz.insert("normalna_gra_zyskownych".into(), serde_json::json!(zysk));
        licz.insert("normalna_gra_stratnych".into(), serde_json::json!(strat));
        licz.insert("normalna_gra_suma_usd".into(), serde_json::json!(suma));
        licz.insert(
            "normalna_gra_zyski_usd".into(),
            serde_json::json!(suma_plus),
        );
        licz.insert(
            "normalna_gra_straty_usd".into(),
            serde_json::json!(suma_minus),
        );
        licz.insert("sufit_w1_usd".into(), serde_json::json!(suma_plus));
        licz.insert("sufit_w2_usd".into(), serde_json::json!(sufit_w2));
        licz.insert("sufit_w3_usd".into(), serde_json::json!(sufit_w3));
        licz.insert("granica_polowy_ms".into(), serde_json::json!(granica));
        licz.insert("granica_polowy".into(), serde_json::json!(fmt_ts(granica)));
    }

    let raport = serde_json::json!({
        "parametry": {
            "czempion": czempion,
            "preset_sciezka": a.preset.display().to_string(),
            "okno_wejscia_h": a.okno_wejscia_h,
            "horyzont_h": a.horyzont_h,
            "jednostki": UNITS,
            "lot_percent": LOT_PCT,
            "wolumen_odniesienia": VOL_REF,
            "przesuniecie_zegara_min": MSGOFF / 60_000,
        },
        "liczniki": licz,
        "wyniki": wiersze,
        "luka": luka,
        "cechy": cechy_out,
        "polityki_wyjscia": polityki,
        "mnoznik_sl": mnozniki,
        "sweep_celu": sweep_tp,
        "sweep_wejscia": sweep_we,
        "wrazliwosc": wrazliwosc,
        "rozklady": roz,
    });
    let p_rap = a.out.join("wyrocznia_raport.json");
    std::fs::write(&p_rap, serde_json::to_string_pretty(&raport)?)?;
    eprintln!("→ {}", p_rap.display());

    // ---------- podsumowanie na ekran ----------
    println!(
        "\n{:<8} {:<16} {:<14} {:>10} {:>8} {:>9} {:>7} {:>7}",
        "WARIANT", "OKNO", "TRYB", "ZYSK $", "PF", "maxDD $", "DNI+%", "TRANS."
    );
    for w in &wiersze {
        println!(
            "{:<8} {:<16} {:<14} {:>10.2} {:>8.2} {:>9.2} {:>6.1}% {:>7}",
            w.wariant, w.okno, w.tryb, w.zysk, w.pf, w.max_dd_usd, w.dni_plus_pct, w.transakcje
        );
    }
    println!("\nPOLITYKI WYJŚCIA (dolary, wolumen odniesienia {VOL_REF}, poza próbą)");
    println!(
        "{:<28} {:>10} {:>22} {:>12}",
        "POLITYKA", "OOS $", "PARAM (1p→2p)", "W PRÓBIE $"
    );
    for p in &polityki {
        println!(
            "{:<28} {:>10.2} {:>22} {:>12.2}",
            p.polityka, p.oos, p.param_1p, p.in_sample
        );
    }
    println!("\nMNOŻNIK STOPA × POLITYKA WYJŚCIA (dolary)");
    println!(
        "{:>6} {:>12} {:>12} {:>12} {:>12} {:>12}",
        "×SL", "TP1/SL", "BEZ CELU", "CZAS oos", "ZAPADKA oos", "SUFIT"
    );
    for m in &mnozniki {
        println!(
            "{:>6.1} {:>12.2} {:>12.2} {:>12.2} {:>12.2} {:>12.2}",
            m.mnoznik, m.tp1_albo_sl, m.bez_celu, m.czas_oos, m.zapadka_oos, m.sufit_szczyt
        );
    }
    Ok(())
}

fn fmt_ts(ms: i64) -> String {
    let s = ms.div_euclid(1000);
    let day = s.div_euclid(86_400);
    let sec = s.rem_euclid(86_400);
    format!(
        "{} {:02}:{:02}:{:02}",
        fmt_day(day),
        sec / 3600,
        (sec % 3600) / 60,
        sec % 60
    )
}

/// The theoretical oracle and its simulated strategy reference are different
/// evidence. An incomplete RunResult is not a zero reference/profit gap.
/// Preserve valid results unchanged; propagate a diagnostic error before any
/// reference metric, basket/trade conversion, or comparison is published.
fn require_complete_reference(result: RunResult, preset: &str, stage: &str) -> Result<RunResult> {
    if let Some((kind, reason)) = result.reconciliation_hold() {
        anyhow::bail!(
            "HOLD {kind} · wyrocznia / preset {preset} / {stage}: {reason}. \
             Symulacja odniesienia nie jest kompletna; nie publikuję jej wyniku ani luki jako zweryfikowanych."
        );
    }
    Ok(result)
}

#[cfg(test)]
mod reconciliation_gate_tests {
    use super::*;

    fn result() -> RunResult {
        serde_json::from_value(serde_json::json!({
            "metrics": Metrics {start_balance:200.0, end_equity:1000200.0,
                total_profit:1000000.0, min_equity:200.0, ..Metrics::default()},
            "equity_curve": [], "daily": [], "ticks_processed":2, "elapsed_ms":1
        })).unwrap()
    }

    #[test]
    fn all_holds_reject_reference_before_publication_or_comparison() {
        for stage in ["fixed lot", "window / compounding", "window / daily reset"] {
            for kind in 0..4 {
                let mut r = result();
                let reason = Some("missing evidence".to_string());
                match kind {
                    0 => r.cost_reconciliation_required = reason,
                    1 => r.sr_warmup_reconciliation_required = reason,
                    2 => r.sim_execution_reconciliation_required = reason,
                    _ => r.continuation_reconciliation_required = reason,
                }
                let mut published = false;
                let answer = require_complete_reference(r, "candidate", stage).map(|r| {
                    published = true;
                    r.metrics.total_profit
                });
                let error = answer.unwrap_err().to_string();
                assert!(!published);
                assert!(error.contains("HOLD") && error.contains("candidate")
                    && error.contains(stage) && error.contains("missing evidence"));
            }
        }
    }

    #[test]
    fn no_hold_preserves_legacy_result_bytes_without_mutating_cancellation() {
        for cancelled in [false, true] {
            let mut r = result();
            r.cancelled = cancelled;
            let before = serde_json::to_vec(&r).unwrap();
            let r = require_complete_reference(r, "legacy", "fixed lot").unwrap();
            assert_eq!(serde_json::to_vec(&r).unwrap(), before);
        }
    }

    #[test]
    fn actual_runner_dependency_hold_is_not_a_verified_zero_reference() {
        let path = std::env::temp_dir().join(format!("conduit-oracle-hold-{}.bin", std::process::id()));
        let mut bytes = vec![0u8; 64];
        bytes[0..4].copy_from_slice(&0x4B54_4443u32.to_le_bytes());
        bytes[8..16].copy_from_slice(&2u64.to_le_bytes());
        for ts in [1_788_163_200_000i64, 1_788_163_201_000] {
            bytes.extend_from_slice(&ts.to_le_bytes());
            bytes.extend_from_slice(&4400f32.to_le_bytes());
            bytes.extend_from_slice(&4400.2f32.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        let ticks = TickData::open(&path).unwrap();
        let cfg = RunConfig {
            from:ticks.first_ts(), to:ticks.last_ts()+1, start_balance:200.0,
            settings: conduit_core::Settings {closed_profit_net_costs:true,
                basket_realized_broker_only:false, ..Default::default()},
            ..RunConfig::default()
        };
        let r = run(&ticks, &[], &cfg);
        drop(ticks);
        std::fs::remove_file(&path).unwrap();
        assert!(r.cost_reconciliation_required.is_some());
        assert!(!r.cancelled);
        let error = require_complete_reference(r, "invalid-cost", "fixed lot").unwrap_err();
        assert!(error.to_string().contains("HOLD COST"));
    }

    #[test]
    fn both_reference_call_sites_check_before_any_metric_or_trade_use() {
        let source = include_str!("wyrocznia.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        for binding in ["r", "rr"] {
            let call = format!("let {binding} = run(&t, &msgs,");
            let after = production.split(&call).nth(1).expect("reference run call");
            let gate = after.find(&format!("require_complete_reference({binding},")).unwrap();
            let metric = after.find(&format!("{binding}.metrics")).unwrap();
            assert!(gate < metric);
        }
    }
}
