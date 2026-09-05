
use anyhow::{bail, Result};
use conduit_ai::peak::*;
use conduit_backtest::{load_signals, TickData};
use conduit_core::types::Ts;

// ============================================================
//  OBSERWACJA W CHWILI T PO WEJŚCIU
// ============================================================

/// Stan koszyka po `t_min` minutach od pierwszego wypełnienia.
struct Obs {
    /// ile warstw wypełniło się do tej chwili
    warstw: usize,
    /// wychylenie koszyka w tej chwili, w dolarach
    wych: f64,
    /// tempo: warstwy na minutę
    tempo: f64,
    /// odstęp między pierwszym a ostatnim wypełnieniem do tej chwili, w minutach
    rozpietosc_min: f64,
    /// ile warstw koszyk miał NA KONIEC (etykieta pomocnicza)
    warstw_koniec: usize,
    /// wynik koszyka na koniec, w dolarach
    wynik: f64,
    /// wynik, gdyby zamknąć DOKŁADNIE w tej chwili
    wynik_teraz: f64,
    dzien: i64,
}

fn obserwuj(s: &Sciezka, t_min: i64) -> Option<Obs> {
    let ts0 = s.ts0();
    let kres = ts0 + t_min * 60_000;
    let ostatnia = s.probki[s.probki.len() - 1].ts;
    // koszyk musi jeszcze żyć w chwili obserwacji, inaczej nie ma decyzji
    if kres >= ostatnia {
        return None;
    }
    let warstw = s.ts_wyp.iter().filter(|t| **t <= kres).count();
    if warstw == 0 {
        return None;
    }
    let p = s.probki.iter().rev().find(|p| p.ts <= kres)?;
    let pierwsze = s.ts_wyp[0];
    let ostatnie_wyp = *s.ts_wyp.iter().filter(|t| **t <= kres).next_back()?;
    let rozp = (ostatnie_wyp - pierwsze) as f64 / 60_000.0;
    Some(Obs {
        warstw,
        wych: p.wych as f64,
        tempo: warstw as f64 / (t_min as f64).max(0.5),
        rozpietosc_min: rozp,
        warstw_koniec: s.wypelnionych as usize,
        wynik: s.wych_koniec as f64,
        wynik_teraz: p.wych as f64,
        dzien: s.dzien,
    })
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

fn korelacja(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let mut num = 0.0;
    let (mut da, mut db) = (0.0, 0.0);
    for i in 0..a.len() {
        let (x, y) = (a[i] - ma, b[i] - mb);
        num += x * y;
        da += x * x;
        db += y * y;
    }
    if da < 1e-12 || db < 1e-12 {
        0.0
    } else {
        num / (da * db).sqrt()
    }
}

/// Bootstrap po DNIACH dla różnicy dwóch polityk na tych samych koszykach.
fn boot_roznicy(a: &[(i64, f64)], b: &[(i64, f64)], seed: u64) -> (f64, f64, f64) {
    let mut m: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for (x, y) in a.iter().zip(b) {
        *m.entry(x.0).or_insert(0.0) += x.1 - y.1;
    }
    let v: Vec<f64> = m.values().cloned().collect();
    if v.len() < 3 {
        return (v.iter().sum(), f64::NAN, f64::NAN);
    }
    let mut r = Rng(seed | 1);
    let mut s = Vec::with_capacity(4000);
    for _ in 0..4000 {
        let mut acc = 0.0;
        for _ in 0..v.len() {
            acc += v[r.ile(v.len())];
        }
        s.push(acc);
    }
    s.sort_by(|x, y| x.partial_cmp(y).unwrap());
    (v.iter().sum(), s[100], s[3899])
}

/// Regresja grzbietowa na małej liczbie cech (własna, bo `Ridge` chce F cech).
fn ridge_maly(x: &[Vec<f64>], y: &[f64], alpha: f64) -> Vec<f64> {
    let d = x[0].len() + 1;
    let mut a = vec![0.0f64; d * d];
    let mut rhs = vec![0.0f64; d];
    for k in 0..x.len() {
        let mut v = x[k].clone();
        v.push(1.0);
        for i in 0..d {
            for j in 0..d {
                a[i * d + j] += v[i] * v[j];
            }
            rhs[i] += v[i] * y[k];
        }
    }
    for i in 0..d - 1 {
        a[i * d + i] += alpha;
    }
    // eliminacja Gaussa
    for col in 0..d {
        let mut piv = col;
        for r in col + 1..d {
            if a[r * d + col].abs() > a[piv * d + col].abs() {
                piv = r;
            }
        }
        if piv != col {
            for c in 0..d {
                a.swap(col * d + c, piv * d + c);
            }
            rhs.swap(col, piv);
        }
        let dg = a[col * d + col];
        if dg.abs() < 1e-12 {
            continue;
        }
        for r in col + 1..d {
            let f = a[r * d + col] / dg;
            if f == 0.0 {
                continue;
            }
            for c in col..d {
                a[r * d + c] -= f * a[col * d + c];
            }
            rhs[r] -= f * rhs[col];
        }
    }
    let mut w = vec![0.0f64; d];
    for r in (0..d).rev() {
        let mut sm = rhs[r];
        for c in r + 1..d {
            sm -= a[r * d + c] * w[c];
        }
        let dg = a[r * d + r];
        w[r] = if dg.abs() < 1e-12 { 0.0 } else { sm / dg };
    }
    w
}

fn pred(w: &[f64], x: &[f64]) -> f64 {
    let mut s = w[w.len() - 1];
    for i in 0..x.len() {
        s += w[i] * x[i];
    }
    s
}

// ============================================================
//  MAIN
// ============================================================

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    let mut ticks = "data/ticks.bin".to_string();
    let mut signals = "data/signals.json".to_string();
    let mut horyzont_h = 6i64;
    let mut jednostki = 9usize;
    let mut seed = 7u64;
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
            "--jednostki" => {
                i += 1;
                jednostki = v[i].parse()?;
            }
            "--seed" => {
                i += 1;
                seed = v[i].parse()?;
            }
            other => bail!("nieznany argument: {other}"),
        }
        i += 1;
    }

    let td = TickData::open(&ticks)?;
    let syg = load_signals(&signals)?;
    let cfg = GenCfg {
        krok_s: 15,
        horyzont_h,
        rozgrzewka_min: 120,
        msg_offset_ms: 180 * 60_000,
        jednostki,
        deep_off: 3.0,
        tol_off: -2.0,
        sl_min_dist: 3.0,
        n_r: 6.0,
        sesja: None,
        placebo_h: 0,
    };
    let sc = generuj(&td, &syg, &cfg);
    println!(
        "dane: {} ticków · {} sygnałów · {} koszyków · siatka {} warstw · horyzont {} h",
        td.len(),
        syg.len(),
        sc.len(),
        jednostki,
        horyzont_h
    );
    if sc.len() < 200 {
        bail!("za mało koszyków");
    }

    // ==================================================================
    //  1. CZY GŁĘBOKOŚĆ KOŃCOWA W OGÓLE RÓŻNICUJE WYNIK
    // ==================================================================
    println!("\n╔════════════════════════════════════════════════════════════════════════╗");
    println!("║ 1. GŁĘBOKOŚĆ KOŃCOWA A WYNIK — odtworzenie obserwacji SWEEP-a          ║");
    println!("╚════════════════════════════════════════════════════════════════════════╝");
    let mut wg_gl: Vec<(usize, f64, usize)> = Vec::new();
    for g in 1..=jednostki {
        let grupa: Vec<&Sciezka> = sc.iter().filter(|s| s.wypelnionych as usize == g).collect();
        if grupa.is_empty() {
            continue;
        }
        let suma: f64 = grupa.iter().map(|s| s.wych_koniec as f64).sum();
        wg_gl.push((g, suma, grupa.len()));
    }
    println!(
        "  {:<10} {:>10} {:>14} {:>12}",
        "warstw", "koszyków", "suma $", "na koszyk $"
    );
    for (g, s, n) in &wg_gl {
        println!(
            "  {:<10} {:>10} {:>+14.2} {:>+12.3}",
            g,
            n,
            s,
            s / *n as f64
        );
    }

    // ==================================================================
    //  2. CZY WCZESNE TEMPO PRZEWIDUJE GŁĘBOKOŚĆ KOŃCOWĄ
    // ==================================================================
    println!("\n╔════════════════════════════════════════════════════════════════════════╗");
    println!("║ 2. CZY TEMPO W PIERWSZYCH MINUTACH PRZEWIDUJE GŁĘBOKOŚĆ KOŃCOWĄ        ║");
    println!("╚════════════════════════════════════════════════════════════════════════╝");
    for t_min in [1i64, 2, 5] {
        let obs: Vec<Obs> = sc.iter().filter_map(|s| obserwuj(s, t_min)).collect();
        if obs.len() < 100 {
            println!("  t = {t_min} min: za mało obserwacji ({})", obs.len());
            continue;
        }
        let a: Vec<f64> = obs.iter().map(|o| o.warstw as f64).collect();
        let b: Vec<f64> = obs.iter().map(|o| o.warstw_koniec as f64).collect();
        let c: Vec<f64> = obs.iter().map(|o| o.wych).collect();
        println!(
            "  t = {:>2} min · {} koszyków żyje · korelacja(warstw teraz, warstw na koniec) = {:+.4} \
             · korelacja(wychylenie, warstw na koniec) = {:+.4}",
            t_min,
            obs.len(),
            korelacja(&a, &b),
            korelacja(&c, &b)
        );
    }

    // ==================================================================
    //  3. CZY TEMPO DODAJE PONAD SAMO WYCHYLENIE — kontrola bez mechanizmu
    // ==================================================================
    println!("\n╔════════════════════════════════════════════════════════════════════════╗");
    println!("║ 3. CZY TEMPO DODAJE PONAD WYCHYLENIE — kontrola bez mechanizmu         ║");
    println!("╚════════════════════════════════════════════════════════════════════════╝");
    println!(
        "  „Pięć warstw w dwie minuty\" znaczy tyle samo co „cena poszła przeciw mnie\".\n\
         \x20 Model na SAMYM WYCHYLENIU jest kontrolą: tempo liczy się tylko, jeśli ją bije."
    );

    let t_min = 2i64;
    let obs: Vec<Obs> = sc.iter().filter_map(|s| obserwuj(s, t_min)).collect();
    if obs.len() < 200 {
        bail!("za mało obserwacji w t = {t_min} min");
    }
    // podział CHRONOLOGICZNY po dniu
    let mut idx: Vec<usize> = (0..obs.len()).collect();
    idx.sort_by_key(|i| obs[*i].dzien);
    let gr = (idx.len() as f64 * 0.60) as usize;
    let (ucz, test) = (&idx[..gr], &idx[gr..]);

    // etykieta: zysk z DALSZEGO trzymania (koniec − teraz)
    let cel = |o: &Obs| o.wynik - o.wynik_teraz;

    let buduj = |wybor: &dyn Fn(&Obs) -> Vec<f64>| -> (Vec<f64>, f64) {
        let xu: Vec<Vec<f64>> = ucz.iter().map(|i| wybor(&obs[*i])).collect();
        let yu: Vec<f64> = ucz.iter().map(|i| cel(&obs[*i])).collect();
        let w = ridge_maly(&xu, &yu, 1.0);
        // dolary: trzymaj, gdy model przewiduje dodatni zysk z trzymania
        let d: f64 = test
            .iter()
            .map(|i| {
                if pred(&w, &wybor(&obs[*i])) > 0.0 {
                    cel(&obs[*i])
                } else {
                    0.0
                }
            })
            .sum();
        (w, d)
    };

    let (_, d_wych) = buduj(&|o| vec![o.wych]);
    let (_, d_tempo) = buduj(&|o| vec![o.wych, o.warstw as f64, o.tempo, o.rozpietosc_min]);
    let (_, d_sam_tempo) = buduj(&|o| vec![o.warstw as f64, o.tempo, o.rozpietosc_min]);
    let d_zawsze: f64 = test.iter().map(|i| cel(&obs[*i])).sum();

    // kontrola: cechy losowe
    let mut rng = Rng(seed | 3);
    let losowe: Vec<Vec<f64>> = (0..obs.len())
        .map(|_| {
            (0..4)
                .map(|_| rng.ile(2000) as f64 / 1000.0 - 1.0)
                .collect()
        })
        .collect();
    let d_los = {
        let xu: Vec<Vec<f64>> = ucz.iter().map(|i| losowe[*i].clone()).collect();
        let yu: Vec<f64> = ucz.iter().map(|i| cel(&obs[*i])).collect();
        let w = ridge_maly(&xu, &yu, 1.0);
        test.iter()
            .map(|i| {
                if pred(&w, &losowe[*i]) > 0.0 {
                    cel(&obs[*i])
                } else {
                    0.0
                }
            })
            .sum::<f64>()
    };

    println!(
        "\n  {} obserwacji w t = {} min · uczenie {} · test {}",
        obs.len(),
        t_min,
        ucz.len(),
        test.len()
    );
    println!(
        "\n  {:<46} {:>12}",
        "polityka (TEST, zysk z dalszego trzymania)", "suma $"
    );
    println!("  {:<46} {:>+12.2}", "ZAWSZE TRZYMAJ", d_zawsze);
    println!("  {:<46} {:>+12.2}", "NIGDY (zamknij w t = 2 min)", 0.0);
    println!(
        "  {:<46} {:>+12.2}",
        "KONTROLA: model na SAMYM WYCHYLENIU", d_wych
    );
    println!(
        "  {:<46} {:>+12.2}",
        "MODEL: wychylenie + tempo wypełniania", d_tempo
    );
    println!(
        "  {:<46} {:>+12.2}",
        "MODEL: samo tempo, bez wychylenia", d_sam_tempo
    );
    println!("  {:<46} {:>+12.2}", "KONTROLA: cechy losowe", d_los);

    let par = |naz: &str, x: f64, y: f64, kx: &dyn Fn(&Obs) -> bool, ky: &dyn Fn(&Obs) -> bool| {
        let a: Vec<(i64, f64)> = test
            .iter()
            .map(|i| {
                (
                    obs[*i].dzien,
                    if kx(&obs[*i]) { cel(&obs[*i]) } else { 0.0 },
                )
            })
            .collect();
        let b: Vec<(i64, f64)> = test
            .iter()
            .map(|i| {
                (
                    obs[*i].dzien,
                    if ky(&obs[*i]) { cel(&obs[*i]) } else { 0.0 },
                )
            })
            .collect();
        let (d, lo, hi) = boot_roznicy(&a, &b, seed);
        let werdykt = if lo > 0.0 {
            "DODAJE"
        } else if hi < 0.0 {
            "SZKODZI"
        } else {
            "nierozstrzygnięte"
        };
        println!(
            "    {:<44} {:>+9.2} $  [{:+.0} … {:+.0}]  {}",
            naz, d, lo, hi, werdykt
        );
        let _ = (x, y);
    };

    let w_wych = {
        let xu: Vec<Vec<f64>> = ucz.iter().map(|i| vec![obs[*i].wych]).collect();
        let yu: Vec<f64> = ucz.iter().map(|i| cel(&obs[*i])).collect();
        ridge_maly(&xu, &yu, 1.0)
    };
    let w_tempo = {
        let xu: Vec<Vec<f64>> = ucz
            .iter()
            .map(|i| {
                vec![
                    obs[*i].wych,
                    obs[*i].warstw as f64,
                    obs[*i].tempo,
                    obs[*i].rozpietosc_min,
                ]
            })
            .collect();
        let yu: Vec<f64> = ucz.iter().map(|i| cel(&obs[*i])).collect();
        ridge_maly(&xu, &yu, 1.0)
    };
    println!("\n  CZY TEMPO DOKŁADA (różnica sparowana, bootstrap po dniach):");
    par(
        "tempo+wychylenie − samo wychylenie",
        d_tempo,
        d_wych,
        &|o| {
            pred(
                &w_tempo,
                &[o.wych, o.warstw as f64, o.tempo, o.rozpietosc_min],
            ) > 0.0
        },
        &|o| pred(&w_wych, &[o.wych]) > 0.0,
    );
    par(
        "tempo+wychylenie − zawsze trzymaj",
        d_tempo,
        d_zawsze,
        &|o| {
            pred(
                &w_tempo,
                &[o.wych, o.warstw as f64, o.tempo, o.rozpietosc_min],
            ) > 0.0
        },
        &|_| true,
    );

    println!(
        "\n  → {}",
        if d_tempo > d_wych && d_tempo > d_los && d_tempo > d_zawsze {
            "TEMPO DODAJE — sprawdzić płaskowyż i placebo przed ogłoszeniem"
        } else {
            "TEMPO NIE DODAJE ponad wychylenie — to ta sama informacja w innym przebraniu"
        }
    );

    println!(
        "\nUWAGA: pomiar NA ŚCIEŻKACH, lot stały 0,01/warstwę, poślizg i swap w cenach.\n\
         Obserwacja w t = 2 min od PIERWSZEGO WYPEŁNIENIA; koszyki, które do tej chwili\n\
         już nie żyją, są wykluczone (nie ma w nich czego decydować)."
    );
    Ok(())
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(ts: Ts, w: f32) -> Probka {
        Probka {
            ts,
            x: vec![0.0; F],
            y: [0.0; 3],
            y_kl: 0.0,
            szczyt_od_teraz: 0.0,
            wych: w,
            wych_run: w,
            ryzyko: 6.0,
            otw: 1,
            sciezka: 0,
        }
    }

    fn sc(probki: Vec<Probka>, ts_wyp: Vec<Ts>, koniec: f32) -> Sciezka {
        Sciezka {
            sygnal: 0,
            side_buy: true,
            id_sygnalu: 1,
            dzien: 0,
            wypelnionych: ts_wyp.len() as u8,
            ts_wyp,
            probki,
            wych_tp1: None,
            ts_tp1: None,
            wych_koniec: koniec,
            wych_runner: koniec,
            szczyt: 0.0,
            ryzyko_wej: 6.0,
            powod: Powod::Horyzont,
            rf: vec![0.0; RF_WARIANTY.len()],
            rf_ok: vec![false; RF_WARIANTY.len()],
        }
    }

    const M: Ts = 60_000;

    #[test]
    fn obserwacja_liczy_tylko_warstwy_do_chwili_t() {
        let s = sc(
            vec![pr(0, 0.0), pr(M, -5.0), pr(3 * M, -20.0), pr(10 * M, -40.0)],
            vec![0, M, 3 * M],
            -40.0,
        );
        let o = obserwuj(&s, 2).unwrap();
        assert_eq!(o.warstw, 2, "do 2. minuty wypełniły się dwie warstwy");
        assert_eq!(o.warstw_koniec, 3);
        assert!(
            (o.wych + 5.0).abs() < 1e-9,
            "wychylenie z próbki <= 2 min: {}",
            o.wych
        );
    }

    #[test]
    fn koszyk_juz_zamkniety_nie_daje_obserwacji() {
        // ostatnia próbka w 1. minucie — w 2. minucie nie ma czego decydować
        let s = sc(vec![pr(0, 0.0), pr(M, -9.0)], vec![0], -9.0);
        assert!(obserwuj(&s, 2).is_none());
    }

    #[test]
    fn zysk_z_trzymania_jest_roznica_koniec_minus_teraz() {
        let s = sc(
            vec![pr(0, 0.0), pr(M, -5.0), pr(10 * M, 12.0)],
            vec![0, M],
            12.0,
        );
        let o = obserwuj(&s, 2).unwrap();
        assert!((o.wynik - o.wynik_teraz - 17.0).abs() < 1e-9);
    }

    #[test]
    fn ridge_maly_odtwarza_zaleznosc_liniowa() {
        let mut x = Vec::new();
        let mut y = Vec::new();
        for k in 0..200 {
            let a = (k as f64 * 0.03).sin();
            let b = (k as f64 * 0.07).cos();
            x.push(vec![a, b]);
            y.push(3.0 * a - 2.0 * b + 0.5);
        }
        let w = ridge_maly(&x, &y, 1e-9);
        assert!((w[0] - 3.0).abs() < 0.05, "w0 {}", w[0]);
        assert!((w[1] + 2.0).abs() < 0.05, "w1 {}", w[1]);
        assert!((w[2] - 0.5).abs() < 0.05, "b {}", w[2]);
    }

    #[test]
    fn korelacja_zachowuje_sie_sensownie() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        assert!((korelacja(&a, &a) - 1.0).abs() < 1e-9);
        let b = vec![4.0, 3.0, 2.0, 1.0];
        assert!((korelacja(&a, &b) + 1.0).abs() < 1e-9);
    }
}
