//! TEST WYKONALNOŚCI WERSJI 2 — czy stan rynku różnicuje optymalne ustawienia.
//!
//! # Pytanie
//!
//! `PLAN_AI_2.md` wersja 2: model ma być funkcją **stan rynku → wektor ustawień**,
//! wywoływaną przy każdym nowym sygnale. Warunek konieczny, żeby to miało sens:
//! **optymalny wektor ustawień musi być RÓŻNY w różnych stanach rynku.**
//! Jeśli jedno ustawienie jest najlepsze wszędzie — nie ma czego dostrajać.
//!
//! # Dlaczego rozkład zerowy MUSI pochodzić z losowania
//!
//! „Najlepszy wektor per kubełek" jest **z definicji** nie gorszy od najlepszego
//! globalnego — maksimum po podzbiorach jest zawsze ≥ maksimum całości. Sama
//! dodatnia różnica nie dowodzi więc niczego. Rozstrzyga wyłącznie porównanie
//! z **losowym podziałem na kubełki tej samej wielkości**: tam przewaga też
//! wychodzi dodatnia, bo bierze się z dopasowania do szumu.
//!
//! Dwa zespoły popełniły dziś tę pomyłkę w dwóch wariantach — stąd ten komentarz
//! jest tutaj, a nie w raporcie.
//!
//! # Poprzeczka
//!
//! Najlepszy **stały wektor w tej samej przestrzeni**, nie `KRATA` i nie preset
//! z dysku. Inaczej zmierzyłbym przewagę przestrzeni ustawień, a nie modelu.
//!
//! ```text
//! cargo run --release -p conduit-ai --bin ustawienia -- --horyzont-h 6
//! ```

use anyhow::{bail, Result};
use conduit_ai::peak::*;
use conduit_backtest::{load_signals, TickData};

// ============================================================
//  PRZESTRZEŃ USTAWIEŃ
// ============================================================

/// Jeden wektor ustawień. Odpowiada temu, co model wersji 2 ma WYPISYWAĆ.
#[derive(Clone, Copy, PartialEq)]
struct Ustawienia {
    sl_min_dist: f64,
    deep_off: f64,
    jednostki: usize,
    /// `true` = wyjście na TP1, `false` = bez celu
    tp1: bool,
    /// `basket_max_age_min`; 0 = bez limitu
    zycie_min: i64,
}

impl Ustawienia {
    fn opis(&self) -> String {
        format!(
            "sl{:.1} gl{:.0} u{} {} {}",
            self.sl_min_dist,
            self.deep_off,
            self.jednostki,
            if self.tp1 { "TP1" } else { "bez" },
            if self.zycie_min == 0 {
                "∞".to_string()
            } else {
                format!("{}m", self.zycie_min)
            }
        )
    }
}

/// Osie generatora — każda wymaga OSOBNEGO przebiegu po tickach.
fn osie_generatora() -> Vec<(f64, f64, usize)> {
    let mut v = Vec::new();
    for sl in [1.5f64, 3.0, 6.0] {
        for gl in [0.0f64, 3.0, 6.0] {
            for u in [1usize, 3] {
                v.push((sl, gl, u));
            }
        }
    }
    v
}

/// Osie wyjścia — liczone na już wygenerowanych ścieżkach, za darmo.
fn osie_wyjscia() -> Vec<(bool, i64)> {
    let mut v = Vec::new();
    for tp1 in [true, false] {
        for zm in [0i64, 60, 240] {
            v.push((tp1, zm));
        }
    }
    v
}

/// Wypłata jednego wariantu wyjścia na gotowej ścieżce, po kosztach.
fn wyplata(s: &Sciezka, tp1: bool, zycie_min: i64) -> f64 {
    let ts0 = s.ts0();
    let kres = if zycie_min > 0 {
        ts0 + zycie_min * 60_000
    } else {
        i64::MAX
    };
    let swap = |t: i64| swap_usd(ts0, t, s.side_buy) * s.wypelnionych as f64;

    if tp1 {
        if let (Some(t), Some(v)) = (s.ts_tp1, s.wych_tp1) {
            if t <= kres {
                return v as f64 + swap(t);
            }
        }
    }
    let ostatnia = s.probki[s.probki.len() - 1].ts;
    if kres >= ostatnia {
        return s.wych_koniec as f64 + swap(ostatnia);
    }
    match s.probki.iter().rev().find(|p| p.ts <= kres) {
        Some(p) => p.wych as f64 + swap(p.ts),
        None => s.probki[0].wych as f64,
    }
}

// ============================================================
//  NARZĘDZIA
// ============================================================

struct Rng(u64);
impl Rng {
    fn nast(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn ile(&mut self, n: usize) -> usize {
        (self.nast() % n.max(1) as u64) as usize
    }
}

/// Najlepszy wektor w obrębie zbioru indeksów + jego wynik.
fn najlepszy(m: &[Vec<f64>], idx: &[usize], nk: usize) -> (usize, f64) {
    let mut naj = (0usize, f64::NEG_INFINITY);
    for k in 0..nk {
        let s: f64 = idx.iter().map(|i| m[*i][k]).sum();
        if s > naj.1 {
            naj = (k, s);
        }
    }
    naj
}

/// Suma wyników przy strategii „najlepszy wektor osobno w każdym kubełku".
fn suma_per_kubelek(m: &[Vec<f64>], kubelki: &[Vec<usize>], nk: usize) -> (f64, Vec<usize>) {
    let mut suma = 0.0;
    let mut wybory = Vec::with_capacity(kubelki.len());
    for kub in kubelki {
        if kub.is_empty() {
            wybory.push(usize::MAX);
            continue;
        }
        let (k, s) = najlepszy(m, kub, nk);
        suma += s;
        wybory.push(k);
    }
    (suma, wybory)
}

// ============================================================
//  MAIN
// ============================================================

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    let mut ticks = "data/ticks.bin".to_string();
    let mut signals = "data/signals.json".to_string();
    let mut horyzont_h = 6i64;
    let mut krok_s = 30i64;
    let mut n_los = 200usize;
    let mut seed = 7u64;
    let mut frac = 0.60f64;
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < v.len() {
        match v[i].as_str() {
            "--ticks" => {
                i += 1;
                ticks = v[i].clone();
            }
            "--signals" => {
                i += 1;
                signals = v[i].clone();
            }
            "--horyzont-h" => {
                i += 1;
                horyzont_h = v[i].parse()?;
            }
            "--krok-s" => {
                i += 1;
                krok_s = v[i].parse()?;
            }
            "--losowan" => {
                i += 1;
                n_los = v[i].parse()?;
            }
            "--seed" => {
                i += 1;
                seed = v[i].parse()?;
            }
            "--frac" => {
                i += 1;
                frac = v[i].parse()?;
            }
            other => bail!("nieznany argument: {other}"),
        }
        i += 1;
    }

    let td = TickData::open(&ticks)?;
    let syg = load_signals(&signals)?;
    println!(
        "dane: {} ticków · {} sygnałów · horyzont {} h",
        td.len(),
        syg.len(),
        horyzont_h
    );

    let og = osie_generatora();
    let ow = osie_wyjscia();
    let nk = og.len() * ow.len();
    println!(
        "przestrzeń ustawień: {} osi generatora × {} osi wyjścia = {} wektorów",
        og.len(),
        ow.len(),
        nk
    );

    // --- macierz [sygnał][wektor ustawień] ---
    // Sygnał, który przy danych ustawieniach się nie wypełnił, dostaje 0 —
    // „nie wszedłem" to realny skutek ustawień, nie brak danych.
    let n_syg = syg.len();
    let mut m: Vec<Vec<f64>> = vec![vec![0.0; nk]; n_syg];
    // cechy stanu rynku bierzemy z konfiguracji BAZOWEJ, żeby kubełki nie
    // zmieniały się razem z ustawieniami
    let mut atr_wzgl: Vec<f64> = vec![f64::NAN; n_syg];
    let mut godzina: Vec<u32> = vec![99; n_syg];
    let mut dzien_syg: Vec<i64> = vec![0; n_syg];

    let t0 = std::time::Instant::now();
    for (gi, (sl, gl, u)) in og.iter().enumerate() {
        let cfg = GenCfg {
            krok_s,
            horyzont_h,
            rozgrzewka_min: 120,
            msg_offset_ms: 180 * 60_000,
            jednostki: *u,
            deep_off: *gl,
            tol_off: -2.0,
            sl_min_dist: *sl,
            n_r: 6.0,
            sesja: None,
            placebo_h: 0,
        };
        let sc = generuj(&td, &syg, &cfg);
        for s in &sc {
            let si = s.sygnal as usize;
            for (wi, (tp1, zm)) in ow.iter().enumerate() {
                m[si][gi * ow.len() + wi] = wyplata(s, *tp1, *zm);
            }
            if gi == 4 {
                // konfiguracja bazowa: sl 3,0 · gl 3,0 · u 3
                atr_wzgl[si] = s.probki[0].x[16] as f64;
                godzina[si] = ((s.ts0() / 3_600_000) % 24) as u32;
                dzien_syg[si] = s.dzien;
            }
        }
    }
    let zywe: Vec<usize> = (0..n_syg).filter(|i| !atr_wzgl[*i].is_nan()).collect();
    println!(
        "wygenerowano {} przebiegów w {:.1} s · {} sygnałów z wypełnieniem w konfiguracji bazowej",
        og.len(),
        t0.elapsed().as_secs_f64(),
        zywe.len()
    );
    if zywe.len() < 200 {
        bail!("za mało sygnałów");
    }

    // ==================================================================
    //  1. NAJLEPSZY STAŁY WEKTOR — poprzeczka
    // ==================================================================
    let (naj_glob, wyn_glob) = najlepszy(&m, &zywe, nk);
    let ust_glob = {
        let (sl, gl, u) = og[naj_glob / ow.len()];
        let (tp1, zm) = ow[naj_glob % ow.len()];
        Ustawienia {
            sl_min_dist: sl,
            deep_off: gl,
            jednostki: u,
            tp1,
            zycie_min: zm,
        }
    };
    println!("\n╔════════════════════════════════════════════════════════════════════════╗");
    println!("║ 1. POPRZECZKA — najlepszy STAŁY wektor w tej samej przestrzeni         ║");
    println!("╚════════════════════════════════════════════════════════════════════════╝");
    println!(
        "  [{}]  {:+.2} $  ({:+.4} $ na sygnał)",
        ust_glob.opis(),
        wyn_glob,
        wyn_glob / zywe.len() as f64
    );
    // pięć najlepszych, żeby zobaczyć, czy to płaskowyż czy szpilka
    let mut ranking: Vec<(usize, f64)> = (0..nk)
        .map(|k| (k, zywe.iter().map(|i| m[*i][k]).sum()))
        .collect();
    ranking.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    println!("\n  pięć najlepszych stałych wektorów (test na szpilkę):");
    for (k, s) in ranking.iter().take(5) {
        let (sl, gl, u) = og[k / ow.len()];
        let (tp1, zm) = ow[k % ow.len()];
        let uu = Ustawienia {
            sl_min_dist: sl,
            deep_off: gl,
            jednostki: u,
            tp1,
            zycie_min: zm,
        };
        println!("    {:<28} {:>+10.2} $", uu.opis(), s);
    }

    // ==================================================================
    //  2. CZY STAN RYNKU RÓŻNICUJE OPTIMUM
    // ==================================================================
    println!("\n╔════════════════════════════════════════════════════════════════════════╗");
    println!("║ 2. CZY STAN RYNKU RÓŻNICUJE OPTIMUM — z rozkładem zerowym z LOSOWANIA  ║");
    println!("╚════════════════════════════════════════════════════════════════════════╝");

    let kubelkuj = |klucz: &dyn Fn(usize) -> usize, ile: usize| -> Vec<Vec<usize>> {
        let mut k = vec![Vec::new(); ile];
        for i in &zywe {
            k[klucz(*i).min(ile - 1)].push(*i);
        }
        k
    };

    // kwartyle ATR względnego
    let mut sort_atr: Vec<f64> = zywe.iter().map(|i| atr_wzgl[*i]).collect();
    sort_atr.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let prog = |q: f64| sort_atr[((sort_atr.len() - 1) as f64 * q) as usize];
    let (q1, q2, q3) = (prog(0.25), prog(0.50), prog(0.75));

    let podzialy: Vec<(&str, Vec<Vec<usize>>)> = vec![
        (
            "ZMIENNOŚĆ (kwartyle ATR)",
            kubelkuj(
                &|i| {
                    let a = atr_wzgl[i];
                    if a <= q1 {
                        0
                    } else if a <= q2 {
                        1
                    } else if a <= q3 {
                        2
                    } else {
                        3
                    }
                },
                4,
            ),
        ),
        (
            "SESJA (bloki 6 h czasu serwera)",
            kubelkuj(&|i| (godzina[i] / 6) as usize, 4),
        ),
        (
            "ZMIENNOŚĆ × SESJA",
            kubelkuj(
                &|i| {
                    let a = atr_wzgl[i];
                    let z = if a <= q2 { 0 } else { 1 };
                    let s = (godzina[i] / 6) as usize;
                    z * 4 + s
                },
                8,
            ),
        ),
    ];

    for (nazwa, kub) in &podzialy {
        let rozmiary: Vec<usize> = kub.iter().map(|k| k.len()).collect();
        let (suma_r, wybory) = suma_per_kubelek(&m, kub, nk);
        let przewaga = suma_r - wyn_glob;

        // ROZKŁAD ZEROWY: losowe kubełki o TYCH SAMYCH rozmiarach
        let mut rng = Rng(seed | 1);
        let mut lepsze = 0usize;
        let mut przewagi_los: Vec<f64> = Vec::with_capacity(n_los);
        for _ in 0..n_los {
            let mut miesz = zywe.clone();
            for j in (1..miesz.len()).rev() {
                let t = rng.ile(j + 1);
                miesz.swap(j, t);
            }
            let mut los_kub: Vec<Vec<usize>> = Vec::with_capacity(rozmiary.len());
            let mut off = 0usize;
            for r in &rozmiary {
                los_kub.push(miesz[off..off + r].to_vec());
                off += r;
            }
            let (s_los, _) = suma_per_kubelek(&m, &los_kub, nk);
            let p_los = s_los - wyn_glob;
            przewagi_los.push(p_los);
            if p_los >= przewaga {
                lepsze += 1;
            }
        }
        przewagi_los.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = przewagi_los[(n_los as f64 * 0.95) as usize];
        let sr_los = przewagi_los.iter().sum::<f64>() / n_los as f64;
        let p_wart = (lepsze + 1) as f64 / (n_los + 1) as f64;

        let roznych = {
            let mut w: Vec<usize> = wybory
                .iter()
                .cloned()
                .filter(|v| *v != usize::MAX)
                .collect();
            w.sort_unstable();
            w.dedup();
            w.len()
        };

        println!(
            "\n  ── {} ── {} kubełków, rozmiary {:?}",
            nazwa,
            kub.len(),
            rozmiary
        );
        println!(
            "  przewaga „optimum per kubełek\" nad stałym: {:+.2} $ · \
             LOSOWO: średnio {:+.2} $, p95 {:+.2} $",
            przewaga, sr_los, p95
        );
        println!(
            "  różnych wektorów wybranych: {} z {} kubełków · p = {:.3}   [{}]",
            roznych,
            kub.len(),
            p_wart,
            if p_wart < 0.05 {
                "STAN RYNKU RÓŻNICUJE OPTIMUM"
            } else {
                "NIE RÓŻNI SIĘ od losowego podziału — nie ma czego dostrajać"
            }
        );
        print!("  wybory per kubełek: ");
        for (bi, w) in wybory.iter().enumerate() {
            if *w == usize::MAX {
                continue;
            }
            let (sl, gl, u) = og[w / ow.len()];
            let (tp1, zm) = ow[w % ow.len()];
            let uu = Ustawienia {
                sl_min_dist: sl,
                deep_off: gl,
                jednostki: u,
                tp1,
                zycie_min: zm,
            };
            print!("[{}]{} ", bi, uu.opis());
        }
        println!();
    }

    // ==================================================================
    //  3. TEST POZA PRÓBĄ — czy dopasowanie per kubełek się UOGÓLNIA
    // ==================================================================
    println!("\n╔═══════════════════════════════╗");
    println!("║ 3. TEST POZA PRÓBĄ — to jest jedyna liczba, która coś znaczy");
    println!("╚═══════════════════════════════╝");
    println!(
        "  Sekcja 2 liczy przewagę W PRÓBIE — optimum per kubełek jest wybierane na tych\n\
         \x20 samych danych, na których jest mierzone. Tu wybór zapada na PIERWSZEJ części\n\
         \x20 czasu, a wynik liczy się na DRUGIEJ. Poprzeczka: najlepszy stały wektor\n\
         \x20 wybrany tak samo — na pierwszej części, zastosowany na drugiej."
    );

    // podział CHRONOLOGICZNY po dniu sygnału
    let mut wg_czasu = zywe.clone();
    wg_czasu.sort_by_key(|i| dzien_syg[*i]);
    let gr = (wg_czasu.len() as f64 * frac) as usize;
    let ucz: Vec<usize> = wg_czasu[..gr].to_vec();
    let test: Vec<usize> = wg_czasu[gr..].to_vec();

    // progi kubełków wyznaczone WYŁĄCZNIE na części uczącej
    let mut a_ucz: Vec<f64> = ucz.iter().map(|i| atr_wzgl[*i]).collect();
    a_ucz.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let pu = |q: f64| a_ucz[((a_ucz.len() - 1) as f64 * q) as usize];
    let (u1, u2, u3) = (pu(0.25), pu(0.50), pu(0.75));
    let kub_atr = |i: usize| -> usize {
        let a = atr_wzgl[i];
        if a <= u1 {
            0
        } else if a <= u2 {
            1
        } else if a <= u3 {
            2
        } else {
            3
        }
    };
    let kub_zs = |i: usize| -> usize {
        let z = if atr_wzgl[i] <= u2 { 0 } else { 1 };
        z * 4 + (godzina[i] / 6) as usize
    };

    println!(
        "\n  uczenie {} sygnałów (dni {}–{}) · test {} sygnałów (dni {}–{})",
        ucz.len(),
        dzien_syg[ucz[0]],
        dzien_syg[ucz[ucz.len() - 1]],
        test.len(),
        dzien_syg[test[0]],
        dzien_syg[test[test.len() - 1]]
    );

    for (nazwa, klucz, ile) in [
        ("ZMIENNOŚĆ", &kub_atr as &dyn Fn(usize) -> usize, 4usize),
        (
            "ZMIENNOŚĆ × SESJA",
            &kub_zs as &dyn Fn(usize) -> usize,
            8usize,
        ),
    ] {
        // wybór na części uczącej
        let mut ku: Vec<Vec<usize>> = vec![Vec::new(); ile];
        for i in &ucz {
            ku[klucz(*i).min(ile - 1)].push(*i);
        }
        let (_, wybory) = suma_per_kubelek(&m, &ku, nk);
        let (naj_u, _) = najlepszy(&m, &ucz, nk);

        // zastosowanie na części testowej
        let mut wyn_model = 0.0;
        let mut wyn_stala = 0.0;
        for i in &test {
            let b = klucz(*i).min(ile - 1);
            let w = wybory[b];
            if w != usize::MAX {
                wyn_model += m[*i][w];
            }
            wyn_stala += m[*i][naj_u];
        }
        // kontrola: losowe kubełki tej samej wielkości, ta sama procedura
        let rozm: Vec<usize> = ku.iter().map(|k| k.len()).collect();
        let mut rng = Rng(seed | 5);
        let mut lepsze = 0usize;
        let mut wyniki_los: Vec<f64> = Vec::with_capacity(n_los);
        for _ in 0..n_los {
            let mut miesz = ucz.clone();
            for j in (1..miesz.len()).rev() {
                let t = rng.ile(j + 1);
                miesz.swap(j, t);
            }
            let mut lk: Vec<Vec<usize>> = Vec::with_capacity(ile);
            let mut off = 0usize;
            for r in &rozm {
                lk.push(miesz[off..off + r].to_vec());
                off += r;
            }
            let (_, wyb_los) = suma_per_kubelek(&m, &lk, nk);
            // przypisanie testowych do losowych kubełków — też losowo
            let mut w_los = 0.0;
            for i in &test {
                let b = rng.ile(ile);
                let w = wyb_los[b];
                if w != usize::MAX {
                    w_los += m[*i][w];
                }
            }
            wyniki_los.push(w_los);
            if w_los >= wyn_model {
                lepsze += 1;
            }
        }
        wyniki_los.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let sr_los = wyniki_los.iter().sum::<f64>() / n_los as f64;
        let p_wart = (lepsze + 1) as f64 / (n_los + 1) as f64;

        println!(
            "\n  ── {} ──\n\
             \x20 model (optimum per kubełek z uczenia):  {:>+10.2} $\n\
             \x20 POPRZECZKA (najlepszy stały z uczenia): {:>+10.2} $\n\
             \x20 kontrola losowa: średnio {:>+9.2} $, p95 {:>+9.2} $\n\
             \x20 różnica model − poprzeczka: {:>+10.2} $ · p = {:.3}   [{}]",
            nazwa,
            wyn_model,
            wyn_stala,
            sr_los,
            wyniki_los[(n_los as f64 * 0.95) as usize],
            wyn_model - wyn_stala,
            p_wart,
            if wyn_model > wyn_stala && p_wart < 0.05 {
                "UOGÓLNIA SIĘ"
            } else {
                "NIE UOGÓLNIA — dopasowanie do szumu"
            }
        );
    }

    println!(
        "\nUWAGA: pomiar NA ŚCIEŻKACH, lot stały 0,01/warstwę. Poślizg oczekujących\n\
         (+0,092 $/warstwę) i swap są w cenach. Przewaga „per kubełek\" jest liczona\n\
         W PRÓBIE — to górna granica tego, co model mógłby wyciągnąć, a nie wynik modelu."
    );
    Ok(())
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn najlepszy_wybiera_maksimum_w_podzbiorze() {
        let m = vec![vec![1.0, 5.0], vec![1.0, -9.0], vec![1.0, 0.0]];
        // na całości wygrywa kolumna 0 (3,0 wobec −4,0)
        assert_eq!(najlepszy(&m, &[0, 1, 2], 2).0, 0);
        // na samym wierszu 0 wygrywa kolumna 1
        assert_eq!(najlepszy(&m, &[0], 2).0, 1);
    }

    #[test]
    fn per_kubelek_nigdy_nie_jest_gorszy_od_stalego() {
        // to jest własność, przez którą sama dodatnia przewaga nic nie dowodzi
        let m = vec![vec![2.0, -1.0], vec![-1.0, 2.0], vec![0.5, 0.5]];
        let (_, glob) = najlepszy(&m, &[0, 1, 2], 2);
        let (per, _) = suma_per_kubelek(&m, &[vec![0], vec![1], vec![2]], 2);
        assert!(per >= glob, "per kubełek {per} musi być >= stały {glob}");
        assert!((per - 4.5).abs() < 1e-9, "{per}");
    }

    #[test]
    fn pusty_kubelek_nie_psuje_rachunku() {
        let m = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let (s, w) = suma_per_kubelek(&m, &[vec![0, 1], Vec::new()], 2);
        assert!((s - 1.0).abs() < 1e-9);
        assert_eq!(w[1], usize::MAX);
    }

    #[test]
    fn opis_ustawien_jest_jednoznaczny() {
        let a = Ustawienia {
            sl_min_dist: 3.0,
            deep_off: 3.0,
            jednostki: 3,
            tp1: true,
            zycie_min: 60,
        };
        let b = Ustawienia {
            sl_min_dist: 3.0,
            deep_off: 3.0,
            jednostki: 3,
            tp1: false,
            zycie_min: 60,
        };
        assert_ne!(a.opis(), b.opis());
    }
}
