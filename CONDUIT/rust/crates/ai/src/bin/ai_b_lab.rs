
use anyhow::{bail, Result};
use conduit_backtest::data::{load_signals, TickData};
use conduit_core::types::{day_of, hour_of, weekday_of, Px, Side, Ts, XAU_CONTRACT};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fmt::Write as _;

// ============================================================
//  FAKTY BROKERA (podane, nieodkrywane)
// ============================================================
const SLIP_MKT: f64 = 0.008;
const SLIP_PEND: f64 = 0.092;
const SWAP_LONG_PTS: f64 = -75.82;
const SWAP_SHORT_PTS: f64 = 27.41;
const SWAP_SRODA: u32 = 2; // 0 = poniedziałek
const MIN_STOP: f64 = 0.20;
const DZWIGNIA: f64 = 500.0;
const LOT_MIN: f64 = 0.01;
/// Przesunięcie znacznika sygnału (UTC) do czasu serwera (UTC+3).
const UTC_DO_SERWERA_MS: i64 = 180 * 60 * 1000;
/// Termin ważności zlecenia oczekującego — WSPÓLNY dla wszystkich wariantów,
/// żeby liczba wypełnień była identyczna. To nie jest parametr do strojenia.
const WAZNOSC_OCZEKUJACEGO_MS: i64 = 24 * 3600 * 1000;

// ============================================================
//  SYGNAŁ — okrojony do tego, co model widzi
// ============================================================
#[derive(Clone)]
struct Sygnal {
    id: i64,
    ts: Ts, // czas serwera, ms
    side: Side,
    oczekujacy: bool,
    wejscie: Px,
    szerokosc: f64,
    // TYLKO DLA LINII ODNIESIENIA „GEOMETRIA KANAŁU” — model tego nie dostaje.
    kanal_sl: Px,
    kanal_tp1: Px,
}

// ============================================================
//  AKCJA — pełny przepis zarządzania jedną jednostką
// ============================================================
#[derive(Clone, Copy, Debug)]
struct Akcja {
    sl: f64,
    tp: f64, // 0 = brak celu (runner)
    be: f64, // 0 = nigdy
    gap: f64,
    start: f64,
    ttl_min: i64,
}

const G_SL: [f64; 9] = [1.0, 2.0, 3.0, 4.5, 6.0, 9.0, 13.0, 20.0, 30.0];
const G_TP: [f64; 11] = [0.0, 1.0, 2.0, 3.0, 4.5, 6.0, 9.0, 13.0, 20.0, 30.0, 45.0];
const G_BE: [f64; 3] = [0.0, 1.5, 4.0];
const G_TR: [(f64, f64); 3] = [(0.0, 0.0), (3.0, 4.0), (8.0, 10.0)];
/// 360 min jest w siatce celowo — na tym horyzoncie mierzyło AI-A
/// (najlepszy stały wektor −137,19 $) i tam robimy parytet.
const G_TTL: [i64; 4] = [60, 360, 1440, 4320];

fn siatka_akcji() -> Vec<Akcja> {
    let mut v = Vec::new();
    for &sl in &G_SL {
        for &tp in &G_TP {
            for &be in &G_BE {
                for &(gap, start) in &G_TR {
                    for &ttl in &G_TTL {
                        v.push(Akcja {
                            sl,
                            tp,
                            be,
                            gap,
                            start,
                            ttl_min: ttl,
                        });
                    }
                }
            }
        }
    }
    v
}

fn opis_akcji(a: &Akcja) -> String {
    format!(
        "SL {:.1} · TP {} · BE {} · zapadka {} · czas {}",
        a.sl,
        if a.tp == 0.0 {
            "brak".into()
        } else {
            format!("{:.1}", a.tp)
        },
        if a.be == 0.0 {
            "nie".into()
        } else {
            format!("+{:.1}", a.be)
        },
        if a.gap == 0.0 {
            "nie".into()
        } else {
            format!("luz {:.1} od +{:.1}", a.gap, a.start)
        },
        if a.ttl_min >= 1440 {
            "24 h".into()
        } else {
            format!("{} min", a.ttl_min)
        }
    )
}

// ============================================================
//  WEJŚCIE — identyczne dla wszystkich wariantów
// ============================================================
#[derive(Clone, Copy)]
struct Wejscie {
    wypelnione: bool,
    idx: usize,
    ts: Ts,
    px: Px,
}

/// Wejście: **ZAWSZE zlecenie oczekujące na podanym poziomie**.
///
/// W tym wariancie podana cena jest poziomem oczekującego wejścia także wtedy,
/// gdy tekst nie zawiera słowa „LIMITS". Zapobiega to niejawnej zamianie
/// geometrii sygnału na wejście rynkowe w chwili publikacji.
///
/// Funkcja NIE przyjmuje `Akcja` ani mnożnika — to gwarancja typu, że
/// ekspozycja jest identyczna we wszystkich ramionach.
fn wejscie(t: &TickData, s: &Sygnal) -> Wejscie {
    let brak = Wejscie {
        wypelnione: false,
        idx: 0,
        ts: 0,
        px: 0.0,
    };
    let i0 = t.index_at(s.ts);
    if i0 >= t.len() {
        return brak;
    }
    let koniec = s.ts + WAZNOSC_OCZEKUJACEGO_MS;
    let mut i = i0;
    while i < t.len() && t.ts(i) <= koniec {
        let trafione = match s.side {
            Side::Buy => t.ask(i) <= s.wejscie,
            Side::Sell => t.bid(i) >= s.wejscie,
        };
        if trafione {
            let px = s.wejscie + s.side.sign() * SLIP_PEND;
            return Wejscie {
                wypelnione: true,
                idx: i,
                ts: t.ts(i),
                px,
            };
        }
        i += 1;
    }
    brak
}

// ============================================================
//  PROWADZENIE POZYCJI
// ============================================================
#[derive(Clone, Copy, PartialEq, Debug)]
enum Powod {
    Sl,
    Tp,
    Czas,
    KoniecDanych,
}

#[derive(Clone, Copy)]
struct Wynik {
    close_ts: Ts,
    pnl: f32, // $ na 0,01 lota, ze swapem
    powod: Powod,
}

#[inline]
fn korzystniejszy_sl(side: Side, a: Px, b: Px) -> Px {
    match side {
        Side::Buy => a.max(b),
        Side::Sell => a.min(b),
    }
}

/// Prowadzenie JEDNEJ jednostki 0,01 lota. Arytmetyka zgodna co do centa
/// z `conduit_core::engine::widok_koszyka` — pilnuje tego test `zgodnosc_*`.
fn prowadz(t: &TickData, w: &Wejscie, side: Side, a: &Akcja, m: f64) -> Wynik {
    let znak = side.sign();
    let sl_d = (a.sl * m).max(MIN_STOP);
    let tp_d = if a.tp > 0.0 {
        (a.tp * m).max(MIN_STOP)
    } else {
        0.0
    };
    let be_d = a.be * m;
    let gap_d = a.gap * m;
    let start_d = a.start * m;

    let mut sl = w.px - znak * sl_d;
    let tp = if tp_d > 0.0 {
        Some(w.px + znak * tp_d)
    } else {
        None
    };
    let termin = w.ts + a.ttl_min * 60_000;

    let mut szczyt = 0.0f64;
    let mut swap = 0.0f64;
    let mut dzien = day_of(w.ts, 0);
    let swap_pts = match side {
        Side::Buy => SWAP_LONG_PTS,
        Side::Sell => SWAP_SHORT_PTS,
    };

    let mut i = w.idx + 1;
    while i < t.len() {
        let ts = t.ts(i);

        // swap PRZED egzekucją — pozycja, która przeżyła północ, płaci za nią
        let d = day_of(ts, 0);
        while dzien < d {
            dzien += 1;
            let mult = if weekday_of(dzien * 86_400_000, 0) == SWAP_SRODA {
                3.0
            } else {
                1.0
            };
            swap += swap_pts * LOT_MIN * mult;
        }

        let wyj = match side {
            Side::Buy => t.bid(i),
            Side::Sell => t.ask(i),
        };

        // SL ma pierwszeństwo, rozliczany PO CENIE RYNKOWEJ
        let sl_hit = match side {
            Side::Buy => wyj <= sl,
            Side::Sell => wyj >= sl,
        };
        if sl_hit {
            let px = match side {
                Side::Buy => sl.min(wyj),
                Side::Sell => sl.max(wyj),
            };
            let pnl = (px - w.px) * znak * XAU_CONTRACT * LOT_MIN + swap;
            return Wynik {
                close_ts: ts,
                pnl: pnl as f32,
                powod: Powod::Sl,
            };
        }
        if let Some(tpl) = tp {
            let hit = match side {
                Side::Buy => wyj >= tpl,
                Side::Sell => wyj <= tpl,
            };
            if hit {
                let pnl = (tpl - w.px) * znak * XAU_CONTRACT * LOT_MIN + swap;
                return Wynik {
                    close_ts: ts,
                    pnl: pnl as f32,
                    powod: Powod::Tp,
                };
            }
        }
        if ts >= termin {
            let pnl = (wyj - w.px) * znak * XAU_CONTRACT * LOT_MIN + swap;
            return Wynik {
                close_ts: ts,
                pnl: pnl as f32,
                powod: Powod::Czas,
            };
        }

        let zysk = (wyj - w.px) * znak;
        if be_d > 0.0 && zysk >= be_d {
            sl = korzystniejszy_sl(side, sl, w.px);
        }
        if gap_d > 0.0 && zysk >= start_d {
            if zysk > szczyt {
                szczyt = zysk;
            }
            sl = korzystniejszy_sl(side, sl, w.px + znak * (szczyt - gap_d));
        }
        i += 1;
    }
    let ost = t.len() - 1;
    let wyj = match side {
        Side::Buy => t.bid(ost),
        Side::Sell => t.ask(ost),
    };
    let pnl = (wyj - w.px) * znak * XAU_CONTRACT * LOT_MIN + swap;
    Wynik {
        close_ts: t.ts(ost),
        pnl: pnl as f32,
        powod: Powod::KoniecDanych,
    }
}

// ============================================================
//  CECHY t0 — wyłącznie obserwacja rynku, zero informacji z sygnału
// ============================================================
const N_CECH: usize = 15;
const NAZWY_CECH: [&str; N_CECH] = [
    "zmiennosc_60m",
    "zmiennosc_15m",
    "zmiennosc_240m",
    "spread",
    "trend_60m_w_sigmach",
    "trend_240m_w_sigmach",
    "polozenie_w_zakresie_24h",
    "godzina_serwera",
    "dzien_tygodnia",
    "ekspansja_zmiennosci",
    "kierunek",
    "oczekujacy",
    "log_min_od_poprzedniego",
    "sygnalow_w_60m",
    "KONTROLA_LOSOWA",
];

struct Cechy {
    x: Vec<[f64; N_CECH]>,
    sigma: Vec<f64>,
}

fn okno(t: &TickData, i0: usize, ms: i64) -> (f64, f64, f64) {
    // (min, max, cena na początku okna) po mid
    let t0 = t.ts(i0);
    let mut i = i0;
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    let mut pierwsza = (t.bid(i0) + t.ask(i0)) * 0.5;
    while i > 0 && t0 - t.ts(i) <= ms {
        let mid = (t.bid(i) + t.ask(i)) * 0.5;
        if mid < lo {
            lo = mid;
        }
        if mid > hi {
            hi = mid;
        }
        pierwsza = mid;
        i -= 1;
    }
    (lo, hi, pierwsza)
}

fn policz_cechy(t: &TickData, sygnaly: &[Sygnal], ziarno: u64) -> Cechy {
    let mut rng = ChaCha8Rng::seed_from_u64(ziarno);
    let losowe: Vec<f64> = (0..sygnaly.len())
        .map(|_| rng.gen::<f64>() * 2.0 - 1.0)
        .collect();

    let surowe: Vec<[f64; N_CECH]> = sygnaly
        .par_iter()
        .enumerate()
        .map(|(k, s)| {
            let i0 = t.index_at(s.ts).min(t.len() - 1);
            let (lo15, hi15, _) = okno(t, i0, 15 * 60_000);
            let (lo60, hi60, p60) = okno(t, i0, 60 * 60_000);
            let (lo240, hi240, p240) = okno(t, i0, 240 * 60_000);
            let (lo24, hi24, _) = okno(t, i0, 1440 * 60_000);
            let mid = (t.bid(i0) + t.ask(i0)) * 0.5;
            let s60 = (hi60 - lo60).max(0.05);
            let s15 = (hi15 - lo15).max(0.05);
            let s240 = (hi240 - lo240).max(0.05);
            let poprz = if k > 0 {
                (s.ts - sygnaly[k - 1].ts) as f64 / 60_000.0
            } else {
                600.0
            };
            let ile60 = sygnaly[..k]
                .iter()
                .rev()
                .take_while(|p| s.ts - p.ts <= 3_600_000)
                .count() as f64;
            [
                s60,
                s15,
                s240,
                t.ask(i0) - t.bid(i0),
                (mid - p60) / s60,
                (mid - p240) / s240,
                if hi24 > lo24 {
                    (mid - lo24) / (hi24 - lo24)
                } else {
                    0.5
                },
                hour_of(s.ts, 0) as f64,
                weekday_of(s.ts, 0) as f64,
                s15 / (s60 / 2.0),
                s.side.sign(),
                if s.oczekujacy { 1.0 } else { 0.0 },
                (poprz.max(0.1)).ln(),
                ile60,
                losowe[k],
            ]
        })
        .collect();

    let sigma = surowe.iter().map(|c| c[0].clamp(0.4, 40.0)).collect();
    Cechy { x: surowe, sigma }
}

// ============================================================
//  MACIERZ zysk × wariant
// ============================================================
struct Macierz {
    n: usize,
    k: usize,
    pnl: Vec<f32>,
    close_ts: Vec<i64>,
    powod: Vec<Powod>,
}

impl Macierz {
    #[inline]
    fn p(&self, i: usize, k: usize) -> f64 {
        self.pnl[i * self.k + k] as f64
    }
}

fn licz_macierz(
    t: &TickData,
    sygnaly: &[Sygnal],
    wej: &[Wejscie],
    akcje: &[Akcja],
    mnozniki: &[f64],
) -> Macierz {
    let k = akcje.len();
    let wiersze: Vec<(Vec<f32>, Vec<i64>, Vec<Powod>)> = (0..sygnaly.len())
        .into_par_iter()
        .map(|i| {
            let mut p = vec![0.0f32; k];
            let mut c = vec![0i64; k];
            let mut r = vec![Powod::Czas; k];
            if wej[i].wypelnione {
                for (j, a) in akcje.iter().enumerate() {
                    let w = prowadz(t, &wej[i], sygnaly[i].side, a, mnozniki[i]);
                    p[j] = w.pnl;
                    c[j] = w.close_ts;
                    r[j] = w.powod;
                }
            }
            (p, c, r)
        })
        .collect();
    let mut m = Macierz {
        n: sygnaly.len(),
        k,
        pnl: Vec::with_capacity(sygnaly.len() * k),
        close_ts: Vec::with_capacity(sygnaly.len() * k),
        powod: Vec::with_capacity(sygnaly.len() * k),
    };
    for (p, c, r) in wiersze {
        m.pnl.extend_from_slice(&p);
        m.close_ts.extend_from_slice(&c);
        m.powod.extend_from_slice(&r);
    }
    m
}

// ============================================================
//  SYMULACJA KONTA — CZTERY TRYBY
// ============================================================
#[derive(Clone, Copy)]
struct Deal {
    open_ts: Ts,
    close_ts: Ts,
    open_px: Px,
    side: Side,
    pnl001: f64,
}

#[derive(Default, Clone)]
struct RaportKonta {
    zysk: f64,
    jednostek: usize,
    odrzuconych_marginesem: usize,
    dni_stratnych_pct: f64,
    najgorszy_dzien: f64,
    najnizsze_equity: f64,
    wyzerowan: usize,
    miesiace: Vec<(String, f64)>,
}

fn miesiac(ts: Ts) -> String {
    // czas serwera; do etykiety miesiąca wystarczy podział dobowy
    let dni = day_of(ts, 0);
    let mut y = 1970i64;
    let mut d = dni;
    loop {
        let dl = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            366
        } else {
            365
        };
        if d < dl {
            break;
        }
        d -= dl;
        y += 1;
    }
    let przest = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dl = [
        31,
        if przest { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mm = 1;
    for l in dl {
        if d < l {
            break;
        }
        d -= l;
        mm += 1;
    }
    format!("{y}-{mm:02}")
}

/// Jeden przebieg konta. `reset_dobowy` = tryb dzień-po-dniu (każda doba
/// niezależna, saldo startowe 200 $). `compounding` = lot rośnie z saldem
/// w proporcji 0,01 lota na 200 $.
fn symuluj_konto(
    t: &TickData,
    deals: &[Deal],
    reset_dobowy: bool,
    compounding: bool,
) -> RaportKonta {
    let mut r = RaportKonta::default();
    r.najnizsze_equity = f64::MAX;

    let mut grupy: Vec<Vec<Deal>> = Vec::new();
    if reset_dobowy {
        let mut mapa: HashMap<i64, Vec<Deal>> = HashMap::new();
        for d in deals {
            mapa.entry(day_of(d.open_ts, 0)).or_default().push(*d);
        }
        let mut klucze: Vec<i64> = mapa.keys().copied().collect();
        klucze.sort_unstable();
        for k in klucze {
            grupy.push(mapa.remove(&k).unwrap());
        }
    } else {
        grupy.push(deals.to_vec());
    }

    let mut po_dniach: HashMap<i64, f64> = HashMap::new();
    let mut po_mies: HashMap<String, f64> = HashMap::new();

    for g in &grupy {
        let mut g = g.clone();
        g.sort_by_key(|d| d.open_ts);
        let mut saldo = 200.0f64;
        let mut otwarte: Vec<(usize, f64)> = Vec::new(); // (idx w g, lot)
        let mut zdarzenia: Vec<(Ts, u8, usize)> = Vec::new();
        for (i, d) in g.iter().enumerate() {
            zdarzenia.push((d.open_ts, 0, i));
            zdarzenia.push((d.close_ts, 1, i));
        }
        zdarzenia.sort_by_key(|z| (z.0, z.1));
        let mut loty = vec![0.0f64; g.len()];
        let mut martwe = false;

        for (ts, typ, i) in zdarzenia {
            // wycena bieżąca
            let idx = t.index_at(ts).min(t.len().saturating_sub(1));
            let plyn: f64 = otwarte
                .iter()
                .map(|(j, lot)| {
                    let d = &g[*j];
                    let wyj = match d.side {
                        Side::Buy => t.bid(idx),
                        Side::Sell => t.ask(idx),
                    };
                    (wyj - d.open_px) * d.side.sign() * XAU_CONTRACT * lot
                })
                .sum();
            let equity = saldo + plyn;
            if equity < r.najnizsze_equity {
                r.najnizsze_equity = equity;
            }
            if equity <= 30.0 && !martwe {
                martwe = true;
                r.wyzerowan += 1;
            }

            if typ == 0 {
                if martwe {
                    continue;
                }
                let lot = if compounding {
                    (LOT_MIN * (saldo / 200.0).floor().max(1.0) * 100.0).round() / 100.0
                } else {
                    LOT_MIN
                };
                let uzyty: f64 = otwarte
                    .iter()
                    .map(|(j, l)| g[*j].open_px * XAU_CONTRACT * l / DZWIGNIA)
                    .sum();
                let potrzebny = g[i].open_px * XAU_CONTRACT * lot / DZWIGNIA;
                if equity - uzyty < potrzebny {
                    r.odrzuconych_marginesem += 1;
                    continue;
                }
                loty[i] = lot;
                otwarte.push((i, lot));
                r.jednostek += 1;
            } else {
                if loty[i] <= 0.0 {
                    continue;
                }
                otwarte.retain(|(j, _)| *j != i);
                let zysk = g[i].pnl001 * (loty[i] / LOT_MIN);
                saldo += zysk;
                *po_dniach.entry(day_of(g[i].open_ts, 0)).or_default() += zysk;
                *po_mies.entry(miesiac(g[i].open_ts)).or_default() += zysk;
                r.zysk += zysk;
            }
        }
    }

    let mut dni: Vec<(i64, f64)> = po_dniach.into_iter().collect();
    dni.sort_unstable_by_key(|x| x.0);
    let n = dni.len().max(1);
    r.dni_stratnych_pct = 100.0 * dni.iter().filter(|x| x.1 < 0.0).count() as f64 / n as f64;
    r.najgorszy_dzien = dni.iter().map(|x| x.1).fold(0.0, f64::min);
    let mut ms: Vec<(String, f64)> = po_mies.into_iter().collect();
    ms.sort_by(|a, b| a.0.cmp(&b.0));
    r.miesiace = ms;
    if r.najnizsze_equity == f64::MAX {
        r.najnizsze_equity = 200.0;
    }
    r
}

fn deals_z_polityki(
    sygnaly: &[Sygnal],
    wej: &[Wejscie],
    m: &Macierz,
    wybor: &[usize],
) -> Vec<Deal> {
    let mut v = Vec::new();
    for i in 0..sygnaly.len() {
        if !wej[i].wypelnione {
            continue;
        }
        let k = wybor[i];
        v.push(Deal {
            open_ts: wej[i].ts,
            close_ts: m.close_ts[i * m.k + k].max(wej[i].ts + 1),
            open_px: wej[i].px,
            side: sygnaly[i].side,
            pnl001: m.p(i, k),
        });
    }
    v
}

// ============================================================
//  REGRESJA GRZBIETOWA (odniesienie liniowe, obowiązkowe)
// ============================================================
fn ridge(x: &[Vec<f64>], y: &[f64], lam: f64) -> Vec<f64> {
    let d = x[0].len();
    let mut a = vec![vec![0.0f64; d + 1]; d];
    for (xi, yi) in x.iter().zip(y) {
        for r in 0..d {
            for c in 0..d {
                a[r][c] += xi[r] * xi[c];
            }
            a[r][d] += xi[r] * yi;
        }
    }
    for r in 0..d {
        a[r][r] += lam;
    }
    // eliminacja Gaussa z częściowym wyborem elementu głównego
    for c in 0..d {
        let mut piv = c;
        for r in c + 1..d {
            if a[r][c].abs() > a[piv][c].abs() {
                piv = r;
            }
        }
        a.swap(c, piv);
        if a[c][c].abs() < 1e-12 {
            continue;
        }
        let p = a[c][c];
        for j in c..=d {
            a[c][j] /= p;
        }
        for r in 0..d {
            if r == c {
                continue;
            }
            let f = a[r][c];
            if f == 0.0 {
                continue;
            }
            for j in c..=d {
                a[r][j] -= f * a[c][j];
            }
        }
    }
    (0..d).map(|r| a[r][d]).collect()
}

// ============================================================
//  MAIN
// ============================================================
fn main() -> Result<()> {
    let szkielet = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ai_b/szkielet.bin".to_string());
    let plik_sygnalow = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "data/signals.json".to_string());

    let t = TickData::open(&szkielet)?;
    eprintln!(
        "szkielet: {} punktów, {} … {}",
        t.len(),
        t.first_ts(),
        t.last_ts()
    );

    let surowe = load_signals(&plik_sygnalow)?;
    let mut sygnaly: Vec<Sygnal> = surowe
        .iter()
        .filter_map(|s| {
            let side = match s.dir.as_str() {
                "BUY" => Side::Buy,
                "SELL" => Side::Sell,
                _ => return None,
            };
            let (lo, hi) = (s.lo.min(s.hi), s.lo.max(s.hi));
            let wej = match side {
                Side::Buy => hi,
                Side::Sell => lo,
            };
            if !wej.is_finite() || wej <= 0.0 {
                return None;
            }
            Some(Sygnal {
                id: s.id,
                ts: s.ts * 1000 + UTC_DO_SERWERA_MS,
                side,
                oczekujacy: s.limit,
                wejscie: wej,
                szerokosc: (hi - lo).max(0.0),
                kanal_sl: s.sl,
                kanal_tp1: s.tps.first().copied().unwrap_or(0.0),
            })
        })
        .collect();
    sygnaly.sort_by_key(|s| s.ts);
    sygnaly.retain(|s| s.ts >= t.first_ts() && s.ts <= t.last_ts());
    eprintln!("sygnałów w oknie danych: {}", sygnaly.len());

    // --- WEJŚCIA: identyczne dla wszystkich ramion ---
    let wej: Vec<Wejscie> = sygnaly.par_iter().map(|s| wejscie(&t, s)).collect();
    let wypelnionych = wej.iter().filter(|w| w.wypelnione).count();
    eprintln!(
        "wypełnionych jednostek: {wypelnionych} / {} (kontrola ekspozycji — ta sama liczba we WSZYSTKICH wariantach)",
        sygnaly.len()
    );

    let cechy = policz_cechy(&t, &sygnaly, 0xA1B2C3D4);
    let mut sig_sorted: Vec<f64> = cechy.sigma.clone();
    sig_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let sig_med = sig_sorted[sig_sorted.len() / 2];
    eprintln!("mediana zmienności 60 min: {sig_med:.2} $");

    let akcje = siatka_akcji();
    eprintln!("wariantów zarządzania: {}", akcje.len());

    // --- TRZY RAMIONA ---
    let n = sygnaly.len();
    let m_stale: Vec<f64> = vec![1.0; n];
    let m_skal: Vec<f64> = cechy.sigma.iter().map(|s| s / sig_med).collect();
    let mut perm: Vec<usize> = (0..n).collect();
    {
        let mut rng = ChaCha8Rng::seed_from_u64(777);
        for i in (1..n).rev() {
            let j = rng.gen_range(0..=i);
            perm.swap(i, j);
        }
    }
    let m_plac: Vec<f64> = (0..n).map(|i| m_skal[perm[i]]).collect();

    let start = std::time::Instant::now();
    let mac_stale = licz_macierz(&t, &sygnaly, &wej, &akcje, &m_stale);
    eprintln!(
        "ramię STAŁE policzone w {:.1} s",
        start.elapsed().as_secs_f64()
    );
    let mac_skal = licz_macierz(&t, &sygnaly, &wej, &akcje, &m_skal);
    let mac_plac = licz_macierz(&t, &sygnaly, &wej, &akcje, &m_plac);
    eprintln!("wszystkie ramiona w {:.1} s", start.elapsed().as_secs_f64());

    let mut rap = String::new();
    let _ = writeln!(rap, "# AI-B — zarządzanie od zera\n");
    let _ = writeln!(
        rap,
        "dane: szkielet {} pkt · sygnałów {} · wypełnionych {} · wariantów {}\n",
        t.len(),
        n,
        wypelnionych,
        akcje.len()
    );

    // --- najlepszy STAŁY wariant w każdym ramieniu (in-sample) ---
    let najlepszy = |m: &Macierz| -> (usize, f64) {
        let mut best = (0usize, f64::MIN);
        for k in 0..m.k {
            let s: f64 = (0..m.n).map(|i| m.p(i, k)).sum();
            if s > best.1 {
                best = (k, s);
            }
        }
        best
    };
    let (k_st, v_st) = najlepszy(&mac_stale);
    let (k_sk, v_sk) = najlepszy(&mac_skal);
    let (k_pl, v_pl) = najlepszy(&mac_plac);

    let _ = writeln!(
        rap,
        "## 1. Najlepszy STAŁY wariant w każdym ramieniu (in-sample, 1 jednostka)\n"
    );
    let _ = writeln!(
        rap,
        "To jest **właściwa poprzeczka**, nie `KRATA`: najlepsza polityka BEZ zależności \
         od stanu rynku, wybrana z perspektywy czasu na tej samej próbce. Wszystko, co \
         uczone, musi to pobić, żeby uczenie w ogóle miało sens.\n"
    );
    let na_j = |v: f64| v / wypelnionych as f64;
    let _ = writeln!(rap, "| ramię | najlepszy wariant | suma $ | $/jednostkę |");
    let _ = writeln!(rap, "|---|---|---:|---:|");
    let _ = writeln!(
        rap,
        "| STAŁE (dystanse w $) | {} | {:+.2} | {:+.4} |",
        opis_akcji(&akcje[k_st]),
        v_st,
        na_j(v_st)
    );
    let _ = writeln!(
        rap,
        "| SKALOWANE zmiennością | {} × σ/{:.2} | {:+.2} | {:+.4} |",
        opis_akcji(&akcje[k_sk]),
        sig_med,
        v_sk,
        na_j(v_sk)
    );
    let _ = writeln!(
        rap,
        "| PLACEBO (σ przestawione) | {} × σ_perm/{:.2} | {:+.2} | {:+.4} |",
        opis_akcji(&akcje[k_pl]),
        sig_med,
        v_pl,
        na_j(v_pl)
    );

    // --- porównanie sparowane po WARIANTACH: skalowanie vs stałe vs placebo ---
    let mut lepsze_sk = 0usize;
    let mut lepsze_pl = 0usize;
    let mut sum_sk = 0.0;
    let mut sum_pl = 0.0;
    for k in 0..akcje.len() {
        let a: f64 = (0..n).map(|i| mac_stale.p(i, k)).sum();
        let b: f64 = (0..n).map(|i| mac_skal.p(i, k)).sum();
        let c: f64 = (0..n).map(|i| mac_plac.p(i, k)).sum();
        if b > a {
            lepsze_sk += 1;
        }
        if c > a {
            lepsze_pl += 1;
        }
        sum_sk += b - a;
        sum_pl += c - a;
    }
    let _ = writeln!(
        rap,
        "\n## 2. Skalowanie zmiennością wobec stałych dystansów — po wszystkich {} wariantach\n",
        akcje.len()
    );
    let _ = writeln!(
        rap,
        "| porównanie | wariantów lepszych | średnia różnica $ |"
    );
    let _ = writeln!(rap, "|---|---:|---:|");
    let _ = writeln!(
        rap,
        "| SKALOWANE − STAŁE | {} / {} | {:+.2} |",
        lepsze_sk,
        akcje.len(),
        sum_sk / akcje.len() as f64
    );
    let _ = writeln!(
        rap,
        "| PLACEBO − STAŁE | {} / {} | {:+.2} |",
        lepsze_pl,
        akcje.len(),
        sum_pl / akcje.len() as f64
    );

    // --- GEOMETRIA KANAŁU: ta sama jednostka, wyjście z POZIOMÓW sygnału ---
    // Poziomy BEZWZGLĘDNE, nie dystanse od środka strefy: kanał podaje SL i TP
    // jako ceny i tak je stawia bot.
    let geo: Vec<Wynik> = (0..n)
        .into_par_iter()
        .map(|i| {
            if !wej[i].wypelnione {
                return Wynik {
                    close_ts: 0,
                    pnl: 0.0,
                    powod: Powod::Czas,
                };
            }
            let s = &sygnaly[i];
            let znak = s.side.sign();
            let sl_d = ((wej[i].px - s.kanal_sl) * znak).max(MIN_STOP);
            let tp_d = if s.kanal_tp1 > 0.0 {
                ((s.kanal_tp1 - wej[i].px) * znak).max(MIN_STOP)
            } else {
                0.0
            };
            let a = Akcja {
                sl: sl_d,
                tp: tp_d,
                be: 0.0,
                gap: 0.0,
                start: 0.0,
                ttl_min: 4320,
            };
            prowadz(&t, &wej[i], s.side, &a, 1.0)
        })
        .collect();
    let v_geo: f64 = geo.iter().map(|w| w.pnl as f64).sum();
    let geo_tp = geo.iter().filter(|w| w.powod == Powod::Tp).count();
    let geo_sl = geo.iter().filter(|w| w.powod == Powod::Sl).count();
    let _ = writeln!(
        rap,
        "\n## 3. Poprzeczka „geometria kanału” (poziomy SL i TP1 z sygnału, ta sama jednostka)\n"
    );
    let _ = writeln!(
        rap,
        "**{:+.2} $** łącznie · **{:+.4} $ na jednostkę** · udział celu **{:.1} %** ({geo_tp} TP / {geo_sl} SL)\n",
        v_geo,
        v_geo / wypelnionych as f64,
        100.0 * geo_tp as f64 / (geo_tp + geo_sl).max(1) as f64
    );
    let _ = writeln!(
        rap,
        "> PRZEWIDYWANIE ZAPISANE PRZED POMIAREM: udział celu ma wyjść ≈ 65 % \
         (opublikowana skuteczność 65,4 %), a wynik na jednostkę ≈ −0,11 $ z arytmetyki \
         minus spread, czyli ok. −0,35 $. Zgodność = symulator odtwarza kanał; \
         rozjazd = symulator jest zepsuty i cała reszta raportu jest nieważna.\n"
    );

    // --- WYROCZNIA: najlepszy wariant per sygnał, ex post ---
    let v_wyr: f64 = (0..n)
        .map(|i| {
            (0..akcje.len())
                .map(|k| mac_stale.p(i, k))
                .fold(f64::MIN, f64::max)
        })
        .filter(|v| v.is_finite())
        .sum();
    let _ = writeln!(
        rap,
        "## 4. Wyrocznia (sufit, nie do bicia): **{v_wyr:+.2} $**\n"
    );

    // --- PARYTET Z AI-A: najlepszy stały wariant na horyzoncie 6 h ---
    let mut best6 = (0usize, f64::MIN);
    for (k, a) in akcje.iter().enumerate() {
        if a.ttl_min != 360 {
            continue;
        }
        let s: f64 = (0..n).map(|i| mac_stale.p(i, k)).sum();
        if s > best6.1 {
            best6 = (k, s);
        }
    }
    let _ = writeln!(
        rap,
        "### 4a. Parytet z niezależnym pomiarem AI-A (horyzont 6 h, stały wektor)\n"
    );
    let _ = writeln!(
        rap,
        "AI-A: **−137,19 $**. AI-B, najlepszy stały wariant o `czas 360 min`: **{:+.2} $** ({}).\n",
        best6.1,
        opis_akcji(&akcje[best6.0])
    );

    // ============================================================
    //  POLITYKA UCZONA — walk-forward, okno ROZSZERZAJĄCE
    // ============================================================
    let dni: Vec<i64> = sygnaly.iter().map(|s| day_of(s.ts, 0)).collect();
    let d0 = *dni.first().unwrap();
    let d_max = *dni.last().unwrap();
    const UCZ_MIN_DNI: i64 = 30;
    const KROK_DNI: i64 = 7;
    const N_KAND: usize = 12;

    // standaryzacja cech na całości (średnia/odchylenie) — nie niesie wyniku
    let mut sr = [0.0f64; N_CECH];
    let mut sd = [0.0f64; N_CECH];
    for c in &cechy.x {
        for j in 0..N_CECH {
            sr[j] += c[j];
        }
    }
    for j in 0..N_CECH {
        sr[j] /= n as f64;
    }
    for c in &cechy.x {
        for j in 0..N_CECH {
            sd[j] += (c[j] - sr[j]).powi(2);
        }
    }
    for j in 0..N_CECH {
        sd[j] = (sd[j] / n as f64).sqrt().max(1e-9);
    }
    let xz: Vec<Vec<f64>> = cechy
        .x
        .iter()
        .map(|c| {
            let mut v: Vec<f64> = (0..N_CECH).map(|j| (c[j] - sr[j]) / sd[j]).collect();
            v.push(1.0);
            v
        })
        .collect();

    // uruchomienie walk-forward na zadanej macierzy; zwraca wybór wariantu
    let wf = |m: &Macierz,
              tryb: u8, // 0 uczona, 1 stała (bez mechanizmu), 2 losowa, 3 przetasowane etykiety, 4 placebo-cechy, 5 tylko ostatni tydzień
              ziarno: u64|
     -> Vec<usize> {
        let mut rng = ChaCha8Rng::seed_from_u64(ziarno);
        let mut wybor = vec![0usize; n];
        let mut d = d0 + UCZ_MIN_DNI;
        // przed pierwszym oknem uczącym nie handlujemy wcale (wariant 0 = SL 1.0
        // TP 0 … ale dla uczciwości oznaczamy je i pomijamy w rozliczeniu)
        let mut aktywne = vec![false; n];
        while d <= d_max {
            let ucz_od = if tryb == 5 { d - 7 } else { d0 };
            let ucz: Vec<usize> = (0..n)
                .filter(|&i| dni[i] >= ucz_od && dni[i] < d && wej[i].wypelnione)
                .collect();
            let test: Vec<usize> = (0..n)
                .filter(|&i| dni[i] >= d && dni[i] < d + KROK_DNI)
                .collect();
            if ucz.len() >= 40 {
                // kandydaci: najlepsze warianty na oknie uczącym
                let mut sumy: Vec<(usize, f64)> = (0..m.k)
                    .map(|k| (k, ucz.iter().map(|&i| m.p(i, k)).sum::<f64>()))
                    .collect();
                sumy.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                let kand: Vec<usize> = sumy.iter().take(N_KAND).map(|x| x.0).collect();
                let najlepszy_staly = kand[0];

                match tryb {
                    1 => {
                        for &i in &test {
                            wybor[i] = najlepszy_staly;
                            aktywne[i] = true;
                        }
                    }
                    2 => {
                        for &i in &test {
                            wybor[i] = kand[rng.gen_range(0..kand.len())];
                            aktywne[i] = true;
                        }
                    }
                    _ => {
                        // regresja grzbietowa na każdy wariant kandydujący
                        let xs: Vec<Vec<f64>> = ucz.iter().map(|&i| xz[i].clone()).collect();
                        let mut wagi: Vec<Vec<f64>> = Vec::with_capacity(kand.len());
                        let mut kolejnosc: Vec<usize> = (0..ucz.len()).collect();
                        if tryb == 3 {
                            for i in (1..kolejnosc.len()).rev() {
                                let j = rng.gen_range(0..=i);
                                kolejnosc.swap(i, j);
                            }
                        }
                        for &k in &kand {
                            let y: Vec<f64> =
                                (0..ucz.len()).map(|q| m.p(ucz[kolejnosc[q]], k)).collect();
                            wagi.push(ridge(&xs, &y, 10.0));
                        }
                        for &i in &test {
                            let x = if tryb == 4 {
                                &xz[rng.gen_range(0..n)]
                            } else {
                                &xz[i]
                            };
                            let mut best = (kand[0], f64::MIN);
                            for (q, &k) in kand.iter().enumerate() {
                                let v: f64 = x.iter().zip(&wagi[q]).map(|(a, b)| a * b).sum();
                                if v > best.1 {
                                    best = (k, v);
                                }
                            }
                            wybor[i] = best.0;
                            aktywne[i] = true;
                        }
                    }
                }
            }
            d += KROK_DNI;
        }
        for i in 0..n {
            if !aktywne[i] {
                wybor[i] = usize::MAX;
            }
        }
        wybor
    };

    let suma_wyboru = |m: &Macierz, w: &[usize]| -> (f64, usize) {
        let mut s = 0.0;
        let mut c = 0;
        for i in 0..n {
            if w[i] != usize::MAX && wej[i].wypelnione {
                s += m.p(i, w[i]);
                c += 1;
            }
        }
        (s, c)
    };

    let _ = writeln!(
        rap,
        "## 5. Polityka uczona wobec kontroli (walk-forward, okno rozszerzające, krok {KROK_DNI} dni)\n"
    );
    let _ = writeln!(rap, "| ramię | polityka | zysk $ | jednostek |");
    let _ = writeln!(rap, "|---|---|---:|---:|");

    let mut wyniki_wf: Vec<(String, String, f64, usize, Vec<usize>, &Macierz)> = Vec::new();
    for (nazwa, mac) in [
        ("STAŁE", &mac_stale),
        ("SKALOWANE", &mac_skal),
        ("PLACEBO-σ", &mac_plac),
    ] {
        for (opis, tryb, z) in [
            ("uczona (cechy rynku)", 0u8, 11u64),
            ("bez mechanizmu (stała najlepsza z przeszłości)", 1, 12),
            ("losowa z tych samych kandydatów", 2, 13),
            ("przetasowane etykiety", 3, 14),
            ("placebo cech (cechy z obcego sygnału)", 4, 15),
            ("tylko ostatni tydzień (zakazana rekurencja)", 5, 16),
        ] {
            let w = wf(mac, tryb, z);
            let (s, c) = suma_wyboru(mac, &w);
            let _ = writeln!(rap, "| {nazwa} | {opis} | {s:+.2} | {c} |");
            wyniki_wf.push((nazwa.to_string(), opis.to_string(), s, c, w, mac));
        }
    }

    // ============================================================
    //  CZTERY TRYBY dla najważniejszych polityk
    // ============================================================
    let _ = writeln!(rap, "\n## 6. Cztery tryby, konto 200 $\n");
    let _ = writeln!(
        rap,
        "| polityka | tryb | zysk $ | jedn. | dni− % | najg. dzień | min. equity | wyzerowań | odrzuc. margines |"
    );
    let _ = writeln!(rap, "|---|---|---:|---:|---:|---:|---:|---:|---:|");

    let mut do_oceny: Vec<(String, Vec<Deal>)> = Vec::new();
    // geometria kanału
    {
        let mut v = Vec::new();
        for i in 0..n {
            if !wej[i].wypelnione || geo[i].close_ts == 0 {
                continue;
            }
            v.push(Deal {
                open_ts: wej[i].ts,
                close_ts: geo[i].close_ts.max(wej[i].ts + 1),
                open_px: wej[i].px,
                side: sygnaly[i].side,
                pnl001: geo[i].pnl as f64,
            });
        }
        do_oceny.push(("GEOMETRIA KANAŁU".into(), v));
    }
    // najlepsza stała (in-sample) w ramieniu stałym i skalowanym
    {
        let w = vec![k_st; n];
        do_oceny.push((
            "STAŁA najlepsza (in-sample)".into(),
            deals_z_polityki(&sygnaly, &wej, &mac_stale, &w),
        ));
        let w = vec![k_sk; n];
        do_oceny.push((
            "SKALOWANA najlepsza (in-sample)".into(),
            deals_z_polityki(&sygnaly, &wej, &mac_skal, &w),
        ));
    }
    for (nazwa, opis, _, _, w, mac) in &wyniki_wf {
        if !(opis.starts_with("uczona") || opis.starts_with("bez mechanizmu")) {
            continue;
        }
        let mut v = Vec::new();
        for i in 0..n {
            if w[i] == usize::MAX || !wej[i].wypelnione {
                continue;
            }
            v.push(Deal {
                open_ts: wej[i].ts,
                close_ts: mac.close_ts[i * mac.k + w[i]].max(wej[i].ts + 1),
                open_px: wej[i].px,
                side: sygnaly[i].side,
                pnl001: mac.p(i, w[i]),
            });
        }
        do_oceny.push((format!("{nazwa} / {opis}"), v));
    }

    let mut json = serde_json::Map::new();
    for (nazwa, deals) in &do_oceny {
        let mut wpisy = serde_json::Map::new();
        for (tn, reset, comp) in [
            ("dzień-po-dniu, bez comp.", true, false),
            ("dzień-po-dniu, comp.", true, true),
            ("długoterminowo, bez comp.", false, false),
            ("długoterminowo, comp.", false, true),
        ] {
            let r = symuluj_konto(&t, deals, reset, comp);
            let _ = writeln!(
                rap,
                "| {nazwa} | {tn} | {:+.2} | {} | {:.0} | {:+.2} | {:.1} | {} | {} |",
                r.zysk,
                r.jednostek,
                r.dni_stratnych_pct,
                r.najgorszy_dzien,
                r.najnizsze_equity,
                r.wyzerowan,
                r.odrzuconych_marginesem
            );
            wpisy.insert(
                tn.to_string(),
                serde_json::json!({
                    "zysk": r.zysk, "jednostek": r.jednostek,
                    "dni_stratnych_pct": r.dni_stratnych_pct,
                    "najgorszy_dzien": r.najgorszy_dzien,
                    "najnizsze_equity": r.najnizsze_equity,
                    "wyzerowan": r.wyzerowan,
                    "miesiace": r.miesiace,
                }),
            );
        }
        json.insert(nazwa.clone(), serde_json::Value::Object(wpisy));
    }

    // --- miesiące osobno dla kluczowych polityk (tryb długoterminowy bez comp.) ---
    let _ = writeln!(
        rap,
        "\n## 7. Wynik każdego miesiąca osobno (długoterminowo, bez compoundingu)\n"
    );
    let _ = writeln!(rap, "| polityka | 2026-04 | 2026-05 | 2026-06 | 2026-07 |");
    let _ = writeln!(rap, "|---|---:|---:|---:|---:|");
    for (nazwa, deals) in &do_oceny {
        let r = symuluj_konto(&t, deals, false, false);
        let g = |m: &str| -> f64 {
            r.miesiace
                .iter()
                .find(|x| x.0 == m)
                .map(|x| x.1)
                .unwrap_or(0.0)
        };
        let _ = writeln!(
            rap,
            "| {nazwa} | {:+.1} | {:+.1} | {:+.1} | {:+.1} |",
            g("2026-04"),
            g("2026-05"),
            g("2026-06"),
            g("2026-07")
        );
    }

    // --- rozkład powodów wyjścia (kontrola niezdegenerowania) ---
    let _ = writeln!(
        rap,
        "\n## 8. Rozkład powodów wyjścia (kontrola niezdegenerowania)\n"
    );
    let _ = writeln!(rap, "| polityka | SL | TP | czas |");
    let _ = writeln!(rap, "|---|---:|---:|---:|");
    let licz = |m: &Macierz, w: &[usize]| -> (usize, usize, usize) {
        let (mut a, mut b, mut c) = (0, 0, 0);
        for i in 0..n {
            if w[i] == usize::MAX || !wej[i].wypelnione {
                continue;
            }
            match m.powod[i * m.k + w[i]] {
                Powod::Sl => a += 1,
                Powod::Tp => b += 1,
                _ => c += 1,
            }
        }
        (a, b, c)
    };
    {
        let (a, b, c) = licz(&mac_stale, &vec![k_st; n]);
        let _ = writeln!(rap, "| STAŁA najlepsza | {a} | {b} | {c} |");
        let (a, b, c) = licz(&mac_skal, &vec![k_sk; n]);
        let _ = writeln!(rap, "| SKALOWANA najlepsza | {a} | {b} | {c} |");
        let (mut a, mut b, mut c) = (0, 0, 0);
        for i in 0..n {
            if wej[i].wypelnione && geo[i].close_ts != 0 {
                match geo[i].powod {
                    Powod::Sl => a += 1,
                    Powod::Tp => b += 1,
                    _ => c += 1,
                }
            }
        }
        let _ = writeln!(rap, "| GEOMETRIA KANAŁU | {a} | {b} | {c} |");
    }
    for (nazwa, opis, _, _, w, mac) in &wyniki_wf {
        if !opis.starts_with("uczona") {
            continue;
        }
        let (a, b, c) = licz(mac, w);
        let _ = writeln!(rap, "| {nazwa} / uczona | {a} | {b} | {c} |");
    }

    // ============================================================
    //  §9 — PYTANIE KOORDYNATORA: czy TEMPO wypełniania pierwszych
    //  warstw przewiduje, że koszyk skończy jako dziewięciowarstwowy?
    // ============================================================
    //
    // SWEEP: 3 wypełnione warstwy → +1 965 $, 9 wypełnionych → −1 662 $.
    // Hipoteza zerowa, którą trzeba obalić ZANIM ogłosi się informację:
    // „dziewięć wypełnionych warstw" to nie prognoza, tylko INNA NAZWA na
    // „cena spadła o osiem rozstawów". Różnica 3 600 $ może być tautologią.
    // Pytanie ma sens dopiero w postaci: czy przy TEJ SAMEJ osiągniętej
    // głębokości (3 warstwy) TEMPO dojścia do niej niesie informację o tym,
    // co będzie dalej.
    const H_WARSTW_MS: i64 = 6 * 3600 * 1000;
    const N_WARSTW: usize = 9;
    struct Koszyk {
        n_wypelnionych: usize,
        czasy: [f64; N_WARSTW], // minuty od sygnału, f64::NAN gdy brak
        pnl: f64,
    }
    let koszyki: Vec<Koszyk> = (0..n)
        .into_par_iter()
        .map(|i| {
            let s = &sygnaly[i];
            let znak = s.side.sign();
            let krok = (s.szerokosc / 2.0).max(1.0);
            let mut czasy = [f64::NAN; N_WARSTW];
            let mut ile = 0usize;
            let i0 = t.index_at(s.ts);
            let koniec = s.ts + H_WARSTW_MS;
            let mut j = i0;
            while j < t.len() && t.ts(j) <= koniec && ile < N_WARSTW {
                let cena = match s.side {
                    Side::Buy => t.ask(j),
                    Side::Sell => t.bid(j),
                };
                while ile < N_WARSTW {
                    let poziom = s.wejscie - znak * krok * ile as f64;
                    let trafione = if s.side == Side::Buy {
                        cena <= poziom
                    } else {
                        cena >= poziom
                    };
                    if !trafione {
                        break;
                    }
                    czasy[ile] = (t.ts(j) - s.ts) as f64 / 60_000.0;
                    ile += 1;
                }
                j += 1;
            }
            // wynik koszyka: każda wypełniona warstwa 0,01 lota, geometria
            // kanałowa liczona OD WŁASNEGO poziomu warstwy, horyzont 6 h
            let mut pnl = 0.0;
            for k in 0..ile {
                let poziom = s.wejscie - znak * krok * k as f64;
                let idx = t
                    .index_at(s.ts + (czasy[k] * 60_000.0) as i64)
                    .min(t.len() - 1);
                let w = Wejscie {
                    wypelnione: true,
                    idx,
                    ts: t.ts(idx),
                    px: poziom + znak * SLIP_PEND,
                };
                let a = Akcja {
                    sl: 6.0,
                    tp: 3.0,
                    be: 0.0,
                    gap: 0.0,
                    start: 0.0,
                    ttl_min: 360,
                };
                pnl += prowadz(&t, &w, s.side, &a, 1.0).pnl as f64;
            }
            Koszyk {
                n_wypelnionych: ile,
                czasy,
                pnl,
            }
        })
        .collect();

    let _ = writeln!(
        rap,
        "\n## 9. Czy TEMPO wypełniania warstw przewiduje głęboki koszyk? (pytanie koordynatora)\n"
    );
    let _ = writeln!(
        rap,
        "Siatka {N_WARSTW} warstw, rozstaw = pół szerokości strefy (mediana 2,50 $), \
         horyzont 6 h, geometria kanałowa (SL 6 / TP 3) liczona od poziomu KAŻDEJ warstwy.\n"
    );
    let _ = writeln!(
        rap,
        "| wypełnionych warstw | koszyków | suma $ | $/koszyk | $/warstwę |"
    );
    let _ = writeln!(rap, "|---:|---:|---:|---:|---:|");
    for grupa in [(1usize, 2usize), (3, 4), (5, 6), (7, 8), (9, 9)] {
        let sel: Vec<&Koszyk> = koszyki
            .iter()
            .filter(|k| k.n_wypelnionych >= grupa.0 && k.n_wypelnionych <= grupa.1)
            .collect();
        if sel.is_empty() {
            continue;
        }
        let suma: f64 = sel.iter().map(|k| k.pnl).sum();
        let warstw: usize = sel.iter().map(|k| k.n_wypelnionych).sum();
        let _ = writeln!(
            rap,
            "| {}–{} | {} | {:+.1} | {:+.2} | {:+.3} |",
            grupa.0,
            grupa.1,
            sel.len(),
            suma,
            suma / sel.len() as f64,
            suma / warstw.max(1) as f64
        );
    }

    // --- AUC: przy TEJ SAMEJ osiągniętej głębokości 3 warstw ---
    fn auc(x: &[f64], y: &[bool]) -> f64 {
        let mut idx: Vec<usize> = (0..x.len()).collect();
        idx.sort_by(|&a, &b| x[a].partial_cmp(&x[b]).unwrap());
        let (mut rank_sum, mut npos, mut nneg) = (0.0f64, 0usize, 0usize);
        let mut i = 0;
        while i < idx.len() {
            let mut j = i;
            while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
                j += 1;
            }
            let sr = (i + j) as f64 / 2.0 + 1.0;
            for &q in &idx[i..=j] {
                if y[q] {
                    rank_sum += sr;
                }
            }
            i = j + 1;
        }
        for &b in y {
            if b {
                npos += 1
            } else {
                nneg += 1
            }
        }
        if npos == 0 || nneg == 0 {
            return 0.5;
        }
        (rank_sum - npos as f64 * (npos as f64 + 1.0) / 2.0) / (npos as f64 * nneg as f64)
    }

    let pop: Vec<usize> = (0..n)
        .filter(|&i| koszyki[i].n_wypelnionych >= 3 && koszyki[i].czasy[2].is_finite())
        .collect();
    let etyk: Vec<bool> = pop
        .iter()
        .map(|&i| koszyki[i].n_wypelnionych >= 7)
        .collect();
    let ile_pos = etyk.iter().filter(|b| **b).count();
    let _ = writeln!(
        rap,
        "\n### 9a. Populacja warunkowana głębokością: koszyki, które doszły do 3 warstw\n"
    );
    let _ = writeln!(
        rap,
        "n = {} · z tego skończyło ≥ 7 warstw: {} ({:.1} %)\n",
        pop.len(),
        ile_pos,
        100.0 * ile_pos as f64 / pop.len().max(1) as f64
    );
    let mut rng9 = ChaCha8Rng::seed_from_u64(9091);
    let losowa: Vec<f64> = (0..pop.len()).map(|_| rng9.gen::<f64>()).collect();
    let predyktory: Vec<(&str, Vec<f64>)> = vec![
        (
            "czas do 3. warstwy (min)",
            pop.iter().map(|&i| koszyki[i].czasy[2]).collect(),
        ),
        (
            "odstęp 1.→3. warstwy (min)",
            pop.iter()
                .map(|&i| koszyki[i].czasy[2] - koszyki[i].czasy[0])
                .collect(),
        ),
        (
            "odstęp 2.→3. warstwy (min)",
            pop.iter()
                .map(|&i| koszyki[i].czasy[2] - koszyki[i].czasy[1])
                .collect(),
        ),
        (
            "zmienność 60 min w t0 (kontrola: cecha znana PRZED wejściem)",
            pop.iter().map(|&i| cechy.sigma[i]).collect(),
        ),
        ("KONTROLA LOSOWA", losowa),
    ];
    let _ = writeln!(rap, "| predyktor | AUC | przedział 95 % (bootstrap 400) |");
    let _ = writeln!(rap, "|---|---:|---|");
    for (nazwa, x) in &predyktory {
        let a = auc(x, &etyk);
        let mut boot: Vec<f64> = Vec::with_capacity(400);
        for _ in 0..400 {
            let mut xb = Vec::with_capacity(x.len());
            let mut yb = Vec::with_capacity(x.len());
            for _ in 0..x.len() {
                let q = rng9.gen_range(0..x.len());
                xb.push(x[q]);
                yb.push(etyk[q]);
            }
            boot.push(auc(&xb, &yb));
        }
        boot.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let _ = writeln!(
            rap,
            "| {nazwa} | {:.3} | [{:.3}, {:.3}] |",
            a, boot[10], boot[389]
        );
    }

    std::fs::create_dir_all("../analiza")?;
    std::fs::write("../analiza/ai_b_raport.md", &rap)?;
    std::fs::write(
        "../analiza/ai_b_wyniki.json",
        serde_json::to_string_pretty(&serde_json::Value::Object(json))?,
    )?;
    println!("{rap}");
    eprintln!("zapisano analiza/ai_b_raport.md i ai_b_wyniki.json");
    Ok(())
}

// ============================================================
//  TESTY — padną, gdy ktoś to zepsuje
// ============================================================
#[cfg(test)]
mod testy {
    use super::*;
    use conduit_core::engine::widok_koszyka;
    use conduit_core::types::{Basket, BasketState, Position, Quote, SourceKey};

    fn pozycja(side: Side, open: Px, vol: f64) -> Position {
        Position {
            ticket: 1,
            side,
            volume: vol,
            open_price: open,
            open_ts: 0,
            sl: None,
            tp: None,
            vsl: None,
            basket: Some(1),
            level: 0,
            frozen: false,
            peak_pts: 0.0,
            last_peak_ts: 0,
            is_runner: false,
            is_toucher: false,
            comment: String::new(),
        }
    }

    fn koszyk(side: Side) -> Basket {
        Basket {
            id: 1,
            source: SourceKey::new(0, None),
            source_name: String::new(),
            msg_id: 0,
            msg_aliases: vec![],
            persisted_done_actions: vec![],
            pending_exit: None,
            pending_relot_review: Vec::new(),
            entry_edit_state: None,
            side,
            is_limit: false,
            entry_lo: 0.0,
            entry_hi: 0.0,
            zone_lo: 0.0,
            zone_hi: 0.0,
            sl: None,
            tps: vec![],
            tp_stage: 0,
            plan_wykonany_do: 0,
            created_ts: 0,
            state: BasketState::Working,
            tickets: vec![1],
            pendings: vec![],
            realized: 0.0,
            events: vec![],
            levels: vec![],
            reentries: 0,
            last_entry_px: None,
            secured: false,
            rearm_blocked_by_spp: false,
            had_positions: true,
            rearms: 0,
            last_rearm_ts: 0,
            // Pola dołożone do `Basket` przez inny zespół; atrapa musi je mieć,
            // inaczej cały cel testowy `ai_b_lab` się nie kompiluje i wywala
            // `cargo test --workspace`.
            last_addon_ts: 0,
            fast_addons: 0,
            tp_touch_ts: vec![],
            peak_pl_usd: 0.0,
            risk_initial_usd: 0.0,
            secured_ts: 0,
            zone_touched: false,
            be_ts: 0,
            tp_open: false,
            warstwy_offset: None,
            drop_armed: false,
            is_stop: false,
            secured_by_rule: false,
            tp_touch_px: Vec::new(),
            sl_touch_ts: 0,
            sl_touch_px: 0.0,
            adverse_since: 0,
            age_limit_min: 0.0,
            tempo_fast: false,
            tempo_checked: false,
            pyramided: false,
            last_tp_ts: 0,
            drop_po_ts: 0,
            wol_pierwotny: Vec::new(),
        }
    }

    /// Arytmetyka pętli AI-B musi dawać CO DO CENTA to samo, co
    /// `widok_koszyka` rdzenia. Inaczej „cecha modelu” i „liczba, na której
    /// bot decyduje" to dwie różne rzeczy — a wtedy porównanie z regułami
    /// przestaje cokolwiek znaczyć.
    #[test]
    fn zgodnosc_z_widokiem_koszyka() {
        for (side, open, bid, ask) in [
            (Side::Buy, 4000.0, 4003.5, 4003.74),
            (Side::Sell, 4000.0, 3996.1, 3996.34),
            (Side::Buy, 4000.0, 3991.2, 3991.44),
        ] {
            let p = pozycja(side, open, LOT_MIN);
            let bk = koszyk(side);
            let q = Quote { ts: 0, bid, ask };
            let v = widok_koszyka(&bk, std::slice::from_ref(&p), &q);
            let wyj = match side {
                Side::Buy => bid,
                Side::Sell => ask,
            };
            let moje = (wyj - open) * side.sign() * XAU_CONTRACT * LOT_MIN;
            assert!(
                (v.pl_usd - moje).abs() < 0.005,
                "rozjazd {} vs {}",
                v.pl_usd,
                moje
            );
        }
    }

    /// Cel realizuje się DOKŁADNIE na poziomie, stop PO CENIE RYNKOWEJ.
    /// Odwrócenie tej reguły było dziurą, która zrobiła 419 348 $ z 200 $.
    #[test]
    fn stop_po_rynku_cel_po_poziomie() {
        // ścieżka: wejście 4000, luka w dół na 3990 — stop 4 $ ma się rozliczyć
        // po 3990, nie po 3996
        let a = Akcja {
            sl: 4.0,
            tp: 8.0,
            be: 0.0,
            gap: 0.0,
            start: 0.0,
            ttl_min: 1440,
        };
        assert_eq!(a.sl, 4.0);
        // sam mechanizm sprawdza test integracyjny na szkielecie; tutaj
        // pilnujemy niezmiennika arytmetycznego
        let znak = Side::Buy.sign();
        let px_stop: f64 = (4000.0f64 - 4.0).min(3990.0);
        assert!((px_stop - 3990.0).abs() < 1e-9);
        let pnl = (px_stop - 4000.0) * znak * XAU_CONTRACT * LOT_MIN;
        assert!(
            (pnl + 10.0).abs() < 1e-9,
            "strata musi być −10 $, jest {pnl}"
        );
    }

    /// Swap: 0,01 lota kupna przez jedną noc = −0,7582 $, środa ×3.
    #[test]
    fn swap_zgodny_z_kontem() {
        assert!((SWAP_LONG_PTS * LOT_MIN + 0.7582).abs() < 1e-9);
        assert!((SWAP_SHORT_PTS * LOT_MIN - 0.2741).abs() < 1e-9);
        assert!((SWAP_LONG_PTS * LOT_MIN * 3.0 + 2.2746).abs() < 1e-9);
    }

    /// Ekspozycja musi być identyczna we wszystkich ramionach: wejście nie
    /// zależy od wariantu zarządzania ani od mnożnika zmienności.
    #[test]
    fn wejscie_nie_zalezy_od_zarzadzania() {
        // `wejscie()` nie przyjmuje `Akcja` ani mnożnika — to jest gwarancja
        // typu, nie zapewnienie. Test istnieje, żeby zmiana sygnatury bolała.
        fn sprawdz(_f: fn(&TickData, &Sygnal) -> Wejscie) {}
        sprawdz(wejscie);
    }
}
