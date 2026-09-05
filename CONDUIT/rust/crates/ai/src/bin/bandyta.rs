
use anyhow::{bail, Result};
use conduit_ai::peak::*;
use conduit_backtest::{load_signals, TickData};
use conduit_core::types::Ts;

// ============================================================
//  ARGUMENTY
// ============================================================

struct Args {
    ticks: String,
    signals: String,
    krok_s: i64,
    horyzont_h: i64,
    msg_offset_min: f64,
    jednostki: usize,
    seed: u64,
    frac_ucz: f64,
    n_losowych: usize,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            ticks: "data/ticks.bin".into(),
            signals: "data/signals.json".into(),
            krok_s: 30,
            horyzont_h: 6,
            msg_offset_min: 180.0,
            jednostki: 3,
            seed: 7,
            frac_ucz: 0.60,
            n_losowych: 20,
        }
    }
}

fn parse() -> Result<Args> {
    let mut a = Args::default();
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
            "--krok-s" => a.krok_s = nast!().parse()?,
            "--horyzont-h" => a.horyzont_h = nast!().parse()?,
            "--msg-offset-min" => a.msg_offset_min = nast!().parse()?,
            "--jednostki" => a.jednostki = nast!().parse()?,
            "--seed" => a.seed = nast!().parse()?,
            "--frac-ucz" => a.frac_ucz = nast!().parse()?,
            other => bail!("nieznany argument: {other}"),
        }
        i += 1;
    }
    Ok(a)
}

// ============================================================
//  PRZESTRZEŃ DECYZJI
// ============================================================

/// Wariant decyzji podejmowanej RAZ, w chwili `t0`.
///
/// Oś „nie bierz" jest tu celowo: `NAUKOWIEC.md` wskazuje selekcję sygnału jako
/// jednego z czterech kandydatów, a bez wariantu zerowego model nie ma jak
/// odmówić gry.
#[derive(Clone, Copy, PartialEq)]
struct Wariant {
    /// `true` = wyjście na TP1, `false` = bez celu (do SL albo do limitu czasu)
    tp1: bool,
    /// limit trzymania w minutach; `0` = bez limitu
    zycie_min: i64,
    /// `true` = w ogóle nie bierzemy tego sygnału
    pas: bool,
}

fn warianty() -> Vec<(String, Wariant)> {
    let mut v = vec![(
        "NIE BIERZ".to_string(),
        Wariant {
            tp1: true,
            zycie_min: 0,
            pas: true,
        },
    )];
    for zm in [15i64, 30, 60, 120, 240, 0] {
        let ozn = if zm == 0 {
            "bez limitu".to_string()
        } else {
            format!("{zm} min")
        };
        v.push((
            format!("TP1 · {ozn}"),
            Wariant {
                tp1: true,
                zycie_min: zm,
                pas: false,
            },
        ));
        v.push((
            format!("bez celu · {ozn}"),
            Wariant {
                tp1: false,
                zycie_min: zm,
                pas: false,
            },
        ));
    }
    v
}

/// Wypłata wariantu dla jednego koszyka, w dolarach, PO KOSZTACH.
///
/// Poślizg wejścia siedzi już w cenie wejścia (generator). Tutaj dochodzi
/// **swap** — naliczany od każdej wypełnionej warstwy za każdą przekroczoną
/// północ. Bez tego wariant trzymający przez noc jest sztucznie lepszy od
/// wariantu, który zamyka przed północą.
fn wyplata(s: &Sciezka, w: Wariant) -> f64 {
    if w.pas {
        return 0.0;
    }
    let ts_start = s.ts0();
    let koszt_swap = |ts_koniec: Ts| -> f64 {
        swap_usd(ts_start, ts_koniec, s.side_buy) * s.wypelnionych as f64
    };
    let ts0 = s.ts0();
    let kres = if w.zycie_min > 0 {
        ts0 + w.zycie_min * 60_000
    } else {
        Ts::MAX
    };

    // TP1 liczy się tylko wtedy, gdy padł PRZED upływem limitu czasu
    if w.tp1 {
        if let (Some(t), Some(v)) = (s.ts_tp1, s.wych_tp1) {
            if t <= kres {
                return v as f64 + koszt_swap(t);
            }
        }
    }
    let ostatnia = s.probki[s.probki.len() - 1].ts;
    if kres >= ostatnia {
        return s.wych_koniec as f64 + koszt_swap(ostatnia);
    }
    match s.probki.iter().rev().find(|p| p.ts <= kres) {
        Some(p) => p.wych as f64 + koszt_swap(p.ts),
        None => s.probki[0].wych as f64,
    }
}

// ============================================================
//  NARZĘDZIA STATYSTYCZNE
// ============================================================

fn sr(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

fn odch(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = sr(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

/// Prosty generator — używany wyłącznie do kontroli, nie do modelu.
struct Rng(u64);
impl Rng {
    fn nast(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn f32(&mut self) -> f32 {
        (self.nast() >> 40) as f32 / (1u32 << 24) as f32
    }
    fn ile(&mut self, n: usize) -> usize {
        (self.nast() % n.max(1) as u64) as usize
    }
}

/// Sumy dzienne — podstawa bootstrapu i miary szumu.
fn dni(par: &[(i64, f64)]) -> Vec<f64> {
    let mut m: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for (d, v) in par {
        *m.entry(*d).or_insert(0.0) += v;
    }
    m.values().cloned().collect()
}

// ============================================================
//  MAIN
// ============================================================

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    let a = parse()?;
    let td = TickData::open(&a.ticks)?;
    let syg = load_signals(&a.signals)?;
    println!("dane: {} ticków · {} sygnałów", td.len(), syg.len());

    let cfg = GenCfg {
        krok_s: a.krok_s,
        horyzont_h: a.horyzont_h,
        rozgrzewka_min: 120,
        msg_offset_ms: (a.msg_offset_min * 60_000.0) as i64,
        jednostki: a.jednostki,
        deep_off: 3.0,
        tol_off: -2.0,
        sl_min_dist: 3.0,
        n_r: 6.0,
        sesja: None,
        placebo_h: 0,
    };
    let t0 = std::time::Instant::now();
    let sc = generuj(&td, &syg, &cfg);
    println!(
        "ścieżki: {} koszyków ({:.1} s) · horyzont {} h (godzinowy — symulator nie liczy swapu)",
        sc.len(),
        t0.elapsed().as_secs_f64(),
        a.horyzont_h
    );
    if sc.len() < 200 {
        bail!("za mało ścieżek");
    }

    let war = warianty();
    let nw = war.len();
    let n = sc.len();

    // --- MACIERZ WYPŁAT: [sygnał][wariant] ---
    let m: Vec<Vec<f64>> = sc
        .iter()
        .map(|s| war.iter().map(|(_, w)| wyplata(s, *w)).collect())
        .collect();

    // ==================================================================
    //  PYTANIE 1 — CZY WARIANTY SIĘ RÓŻNIĄ W OBRĘBIE SYGNAŁU
    // ==================================================================
    println!("\n╔════════════════════════════════════════════════════════════════════════╗");
    println!("║ 1. CZY JEST CZEGO SIĘ UCZYĆ — rozrzut wartości w obrębie sygnału       ║");
    println!("╚════════════════════════════════════════════════════════════════════════╝");
    println!(
        "  Warunek konieczny: warianty muszą mieć RÓŻNE wartości dla tego samego\n\
         \x20 sygnału. Jeśli sd(wariant | sygnał) jest bliskie zeru, model nie ma\n\
         \x20 czego wybierać i dalsza praca jest bezprzedmiotowa."
    );

    let rozstep: Vec<f64> = m
        .iter()
        .map(|r| {
            let mx = r.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let mn = r.iter().cloned().fold(f64::INFINITY, f64::min);
            mx - mn
        })
        .collect();
    let sd_w: Vec<f64> = m.iter().map(|r| odch(r)).collect();
    // szum dzienny: odchylenie sum dziennych najlepszej stałej
    let sr_war: Vec<f64> = (0..nw)
        .map(|k| m.iter().map(|r| r[k]).sum::<f64>())
        .collect();
    let naj_stala = (0..nw)
        .max_by(|x, y| sr_war[*x].partial_cmp(&sr_war[*y]).unwrap())
        .unwrap();
    let dni_naj = dni(&sc
        .iter()
        .zip(&m)
        .map(|(s, r)| (s.dzien, r[naj_stala]))
        .collect::<Vec<_>>());
    let szum_dz = odch(&dni_naj);

    let mut ro = rozstep.clone();
    ro.sort_by(|x, y| x.partial_cmp(y).unwrap());
    println!(
        "\n  rozstęp (najlepszy − najgorszy wariant) na sygnał: mediana {:.2} $ · \
         średnia {:.2} $ · p90 {:.2} $ · maks {:.2} $",
        ro[ro.len() / 2],
        sr(&rozstep),
        ro[ro.len() * 9 / 10],
        ro[ro.len() - 1]
    );
    println!(
        "  odchylenie wariantów w obrębie sygnału: średnio {:.2} $ · \
         odchylenie WYNIKU DZIENNEGO najlepszej stałej: {:.2} $",
        sr(&sd_w),
        szum_dz
    );
    // ⚠ Nie dzielić rozstępu NA SYGNAŁ przez szum DZIENNY — dzień agreguje
    // kilkanaście sygnałów, więc taki iloraz jest mniejszy o pierwiastek z ich
    // liczby i zawsze wygląda na „pusto". Właściwe porównanie jest w tej samej
    // jednostce: rozstęp na sygnał wobec odchylenia wyniku na sygnał.
    let na_sygnal: Vec<f64> = sc.iter().zip(&m).map(|(_, r)| r[naj_stala]).collect();
    let sd_sygnal = odch(&na_sygnal);
    let iloraz = sr(&rozstep) / sd_sygnal.max(1e-9);
    println!(
        "  odchylenie wyniku NA SYGNAŁ (najlepsza stała): {:.2} $ · \
         rozstęp na sygnał / to odchylenie = {:.2}",
        sd_sygnal, iloraz
    );
    println!(
        "  → {}",
        if iloraz > 0.5 {
            "ROZRZUT ISTNIEJE — warianty naprawdę się różnią w obrębie sygnału"
        } else {
            "ROZRZUT ZNIKOMY — warianty są praktycznie tożsame"
        }
    );
    println!(
        "  UWAGA: istnienie rozrzutu to warunek KONIECZNY, nie wystarczający.\n\
         \x20 Rozrzut może być w całości nieprzewidywalny z chwili t0 — rozstrzyga\n\
         \x20 dopiero sekcja 3, a nie ta liczba."
    );

    // ==================================================================
    //  PYTANIE 2 — ILE LEŻY NA STOLE
    // ==================================================================
    println!("\n╔════════════════════════════════════════════════════════════════════════╗");
    println!("║ 2. ILE LEŻY NA STOLE — stałe warianty, losowy wybór, sufit             ║");
    println!("╚════════════════════════════════════════════════════════════════════════╝");
    println!(
        "\n  {:<26} {:>12} {:>12} {:>10}",
        "wariant STAŁY", "suma $", "na sygnał $", "granych"
    );
    for (k, (nazwa, w)) in war.iter().enumerate() {
        let granych = if w.pas { 0 } else { n };
        println!(
            "  {:<26} {:>+12.2} {:>+12.3} {:>10}",
            nazwa,
            sr_war[k],
            sr_war[k] / n as f64,
            granych
        );
    }
    let sufit: f64 = m
        .iter()
        .map(|r| r.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
        .sum();
    let dno: f64 = m
        .iter()
        .map(|r| r.iter().cloned().fold(f64::INFINITY, f64::min))
        .sum();
    let losowy = sr_war.iter().sum::<f64>() / nw as f64;
    println!("\n  {:<26} {:>+12.2}", "NAJLEPSZY STAŁY", sr_war[naj_stala]);
    println!("  {:<26} {:>+12.2}", "losowy wybór (średnia)", losowy);
    println!("  {:<26} {:>+12.2}", "SUFIT (wyrocznia na sygnał)", sufit);
    println!("  {:<26} {:>+12.2}", "DNO (najgorszy na sygnał)", dno);
    println!(
        "\n  → do wygrania między najlepszą stałą a sufitem: {:+.2} $ ({:.1}× najlepsza stała)",
        sufit - sr_war[naj_stala],
        sufit / sr_war[naj_stala].abs().max(1e-9)
    );
    println!(
        "  UWAGA: sufit jest NIEOSIĄGALNY z definicji — to maksimum po fakcie.\n\
         \x20 Liczy się wyłącznie jako górna granica; sam w sobie nie jest wynikiem."
    );

    // ==================================================================
    //  PYTANIE 3 — CZY MODEL TO BIERZE
    // ==================================================================
    println!("\n╔════════════════════════════════════════════════════════════════════════╗");
    println!("║ 3. CZY MODEL TO BIERZE — z kontrolami obowiązkowymi                    ║");
    println!("╚════════════════════════════════════════════════════════════════════════╝");

    let gr = (n as f64 * a.frac_ucz) as usize;
    println!(
        "  podział CHRONOLOGICZNY po ścieżkach: uczenie {} · test {} \
         (jedna decyzja na sygnał, cechy wyłącznie z t0)",
        gr,
        n - gr
    );

    // cechy w t0 + cechy LOSOWE (próg ważności wyznaczają one, nie intuicja)
    let mut rng = Rng(a.seed | 1);
    let nc = F + a.n_losowych;
    let x: Vec<Vec<f32>> = sc
        .iter()
        .map(|s| {
            let mut v = s.probki[0].x.clone();
            for _ in 0..a.n_losowych {
                v.push(rng.f32() * 2.0 - 1.0);
            }
            v
        })
        .collect();

    // --- uczenie: osobna regresja wypłaty dla KAŻDEGO wariantu ---
    let ucz_model = |xs: &[Vec<f32>], mm: &[Vec<f64>]| -> Vec<Ridge> {
        (0..nw)
            .map(|k| {
                let y: Vec<f32> = (0..gr).map(|i| mm[i][k] as f32).collect();
                let xu: Vec<Vec<f32>> = (0..gr).map(|i| xs[i][..F].to_vec()).collect();
                Ridge::ucz(&xu, &y, 10.0)
            })
            .collect()
    };
    let wybierz = |mods: &[Ridge], xs: &[Vec<f32>], i: usize| -> usize {
        let mut naj = (0usize, f64::NEG_INFINITY);
        for (k, md) in mods.iter().enumerate() {
            let p = md.pred(&xs[i][..F]) as f64;
            if p > naj.1 {
                naj = (k, p);
            }
        }
        naj.0
    };

    let modele = ucz_model(&x, &m);
    let wyn_model: Vec<(i64, f64)> = (gr..n)
        .map(|i| (sc[i].dzien, m[i][wybierz(&modele, &x, i)]))
        .collect();

    // --- KONTROLA 1: cechy LOSOWE zamiast prawdziwych ---
    let x_los: Vec<Vec<f32>> = (0..n)
        .map(|_| (0..nc).map(|_| rng.f32() * 2.0 - 1.0).collect::<Vec<f32>>())
        .collect();
    let mod_los = ucz_model(&x_los, &m);
    let wyn_los: Vec<(i64, f64)> = (gr..n)
        .map(|i| (sc[i].dzien, m[i][wybierz(&mod_los, &x_los, i)]))
        .collect();

    // --- KONTROLA 2: PRZETASOWANE wypłaty (cechy prawdziwe, etykiety losowe) ---
    let mut m_tas = m.clone();
    for i in (1..gr).rev() {
        let j = rng.ile(i + 1);
        m_tas.swap(i, j);
    }
    let mod_tas = ucz_model(&x, &m_tas);
    let wyn_tas: Vec<(i64, f64)> = (gr..n)
        .map(|i| (sc[i].dzien, m[i][wybierz(&mod_tas, &x, i)]))
        .collect();

    // --- KONTROLA 3: losowy wybór wariantu ---
    let wyn_rand: Vec<(i64, f64)> = (gr..n).map(|i| (sc[i].dzien, m[i][rng.ile(nw)])).collect();

    // --- odniesienia na TYM SAMYM teście ---
    let wyn_stala: Vec<(i64, f64)> = (gr..n).map(|i| (sc[i].dzien, m[i][naj_stala])).collect();
    let wyn_sufit: Vec<(i64, f64)> = (gr..n)
        .map(|i| {
            (
                sc[i].dzien,
                m[i].iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            )
        })
        .collect();

    let suma = |v: &[(i64, f64)]| v.iter().map(|p| p.1).sum::<f64>();
    println!(
        "\n  {:<44} {:>12} {:>14}",
        "polityka (TEST)", "suma $", "na sygnał $"
    );
    let nt = (n - gr) as f64;
    for (nazwa, v) in [
        ("SUFIT (wyrocznia, nieosiągalny)", &wyn_sufit),
        ("NAJLEPSZY STAŁY wariant", &wyn_stala),
        ("MODEL liniowy na cechach z t0", &wyn_model),
        ("KONTROLA: cechy losowe", &wyn_los),
        ("KONTROLA: przetasowane wypłaty", &wyn_tas),
        ("KONTROLA: losowy wybór wariantu", &wyn_rand),
    ] {
        println!("  {:<44} {:>+12.2} {:>+14.3}", nazwa, suma(v), suma(v) / nt);
    }

    // --- różnice sparowane, bootstrap po dniach ---
    println!("\n  CZY MODEL DOKŁADA (różnica sparowana, bootstrap po dniach):");
    let boot = |x_: &[(i64, f64)], y_: &[(i64, f64)]| -> (f64, f64, f64) {
        let d: Vec<(i64, f64)> = x_.iter().zip(y_).map(|(u, v)| (u.0, u.1 - v.1)).collect();
        let dd = dni(&d);
        if dd.len() < 3 {
            return (0.0, f64::NAN, f64::NAN);
        }
        let mut r = Rng(a.seed | 3);
        let mut s = Vec::with_capacity(4000);
        for _ in 0..4000 {
            let mut acc = 0.0;
            for _ in 0..dd.len() {
                acc += dd[r.ile(dd.len())];
            }
            s.push(acc);
        }
        s.sort_by(|p, q| p.partial_cmp(q).unwrap());
        (dd.iter().sum::<f64>(), s[100], s[3899])
    };
    for (nazwa, v) in [
        ("model − najlepszy stały", &wyn_model),
        ("model − cechy losowe", &wyn_model),
        ("model − przetasowane wypłaty", &wyn_model),
    ] {
        let odn: &Vec<(i64, f64)> = match nazwa {
            "model − najlepszy stały" => &wyn_stala,
            "model − cechy losowe" => &wyn_los,
            _ => &wyn_tas,
        };
        let (d, lo, hi) = boot(v, odn);
        let werdykt = if lo > 0.0 {
            "DODAJE"
        } else if hi < 0.0 {
            "SZKODZI"
        } else {
            "nierozstrzygnięte"
        };
        println!(
            "    {:<40} {:>+9.2} $  [{:+.0} … {:+.0}]  {}",
            nazwa, d, lo, hi, werdykt
        );
    }

    // --- co model faktycznie wybiera ---
    let mut ile_w = vec![0usize; nw];
    for i in gr..n {
        ile_w[wybierz(&modele, &x, i)] += 1;
    }
    println!("\n  rozkład wyborów modelu na teście:");
    for (k, (nazwa, _)) in war.iter().enumerate() {
        if ile_w[k] > 0 {
            println!(
                "    {:<26} {:>5} × ({:.0} %)",
                nazwa,
                ile_w[k],
                ile_w[k] as f64 / nt * 100.0
            );
        }
    }
    let zdeg = ile_w.iter().filter(|v| **v > 0).count();
    if zdeg == 1 {
        println!(
            "  ⚠ MODEL ZDEGENEROWANY: wybiera zawsze ten sam wariant — to jest stała, nie model."
        );
    }

    // ==================================================================
    //  4. DECYZJA ŚRÓDPOZYCYJNA — jedyne niezbadane miejsce
    // ==================================================================
    println!("\n╔═══════════════════════════════╗");
    println!("║ 4. CZY W TRAKCIE ŻYCIA KOSZYKA JEST SYGNAŁ — zamknąć czy trzymać");
    println!("╚═══════════════════════════════╝");
    println!(
        "  Cztery podejścia pokazały, że w chwili t0 sygnału nie ma. To jest INNE\n\
         \x20 pytanie i inna populacja: pozycja już żyje, model widzi jej stan.\n\
         \x20 Etykieta: czy wynik NA KOŃCU będzie lepszy niż TERAZ.\n\
         \x20 Ważenie PO ŚCIEŻKACH — co najwyżej cztery chwile na koszyk, nie 360."
    );

    let chwile = [30i64, 60, 120, 240];
    let mut xd: Vec<Vec<f32>> = Vec::new();
    let mut yd: Vec<f32> = Vec::new();
    let mut sd_: Vec<usize> = Vec::new();
    for (i, s) in sc.iter().enumerate() {
        let t0s = s.ts0();
        let ostatnia = s.probki[s.probki.len() - 1].ts;
        for cm in chwile {
            let kres = t0s + cm * 60_000;
            if kres >= ostatnia {
                continue;
            }
            if let Some(p) = s.probki.iter().rev().find(|p| p.ts <= kres) {
                xd.push(p.x.clone());
                yd.push(s.wych_koniec - p.wych);
                sd_.push(i);
            }
        }
    }
    if xd.len() < 200 {
        println!(
            "  za mało chwil decyzyjnych ({}) — pomiar pominięty",
            xd.len()
        );
    } else {
        let gr_d = sd_.partition_point(|i| *i < gr);
        let dodatnich = yd.iter().filter(|v| **v > 0.0).count();
        println!(
            "\n  {} chwil decyzyjnych z {} koszyków ({:.2} na koszyk) - uczenie {} - test {}\n\
             \x20 trzymanie opłaca się w {:.1} % chwil",
            xd.len(),
            sc.len(),
            xd.len() as f64 / sc.len() as f64,
            gr_d,
            xd.len() - gr_d,
            dodatnich as f64 / xd.len() as f64 * 100.0
        );

        let xu: Vec<Vec<f32>> = xd[..gr_d].to_vec();
        let yu: Vec<f32> = yd[..gr_d].to_vec();
        let md = Ridge::ucz(&xu, &yu, 10.0);

        let mut yu_t = yu.clone();
        for i in (1..yu_t.len()).rev() {
            let j = rng.ile(i + 1);
            yu_t.swap(i, j);
        }
        let md_tas = Ridge::ucz(&xu, &yu_t, 10.0);
        let xu_los: Vec<Vec<f32>> = (0..gr_d)
            .map(|_| (0..F).map(|_| rng.f32() * 2.0 - 1.0).collect())
            .collect();
        let md_los = Ridge::ucz(&xu_los, &yu, 10.0);

        let dolary = |wybor: &dyn Fn(usize) -> bool| -> f64 {
            (gr_d..xd.len())
                .map(|k| if wybor(k) { yd[k] as f64 } else { 0.0 })
                .sum()
        };
        let zawsze = dolary(&|_| true);
        let model_d = dolary(&|k| md.pred(&xd[k]) > 0.0);
        let tas_d = dolary(&|k| md_tas.pred(&xd[k]) > 0.0);
        let los_d = dolary(&|k| md_los.pred(&xd[k]) > 0.0);
        let sufit_d = (gr_d..xd.len())
            .map(|k| (yd[k] as f64).max(0.0))
            .sum::<f64>();

        println!("\n  wartość decyzji trzymania na TEŚCIE (suma zysku z trzymania, w $):");
        println!(
            "  {:<46} {:>+12.2}",
            "SUFIT (wyrocznia: trzymaj gdy się opłaci)", sufit_d
        );
        println!("  {:<46} {:>+12.2}", "ZAWSZE TRZYMAJ", zawsze);
        println!(
            "  {:<46} {:>+12.2}",
            "NIGDY NIE TRZYMAJ (zamknij w chwili decyzji)", 0.0
        );
        println!(
            "  {:<46} {:>+12.2}",
            "MODEL liniowy na cechach z tej chwili", model_d
        );
        println!(
            "  {:<46} {:>+12.2}",
            "KONTROLA: przetasowane etykiety", tas_d
        );
        println!("  {:<46} {:>+12.2}", "KONTROLA: cechy losowe", los_d);

        let pr: Vec<f32> = (gr_d..xd.len()).map(|k| md.pred(&xd[k])).collect();
        let et: Vec<f32> = (gr_d..xd.len())
            .map(|k| if yd[k] > 0.0 { 1.0 } else { 0.0 })
            .collect();
        let pr_l: Vec<f32> = (gr_d..xd.len()).map(|k| md_los.pred(&xd[k])).collect();
        println!(
            "\n  AUC znaku (trzymanie się opłaci): model {:.4} - cechy losowe {:.4}",
            auc(&pr, &et),
            auc(&pr_l, &et)
        );
        let naj_stala_d = zawsze.max(0.0);
        let lepszy = model_d - naj_stala_d;
        println!(
            "\n  -> model {} najlepszą stałą decyzję o {:+.2} $   [{}]",
            if lepszy > 0.0 { "BIJE" } else { "PRZEGRYWA z" },
            lepszy,
            if model_d > tas_d && model_d > los_d && lepszy > 0.0 {
                "przechodzi obie kontrole — JEDYNE ŻYWE MIEJSCE"
            } else {
                "nie przechodzi kontroli — to samo co w t0"
            }
        );
    }

    println!("\n╔═══════════════════════════════╗");
    println!("║ 5. ILE RAZY PRZESTAWIA SIĘ RANKING EKSPERTÓW — test wykonalności");
    println!("╚═══════════════════════════════╝");
    println!(
        "  Agregacja ekspertów ma sens tylko wtedy, gdy ranking naprawdę się przestawia.\n\
         \x20 Jeśli lider jest stały — wystarczy wybrać go raz i nie ma czego uczyć.\n\
         \x20 KONTROLA: te same wypłaty z LOSOWO przypisanymi oknami. Przy czystym szumie\n\
         \x20 lider też się zmienia, więc liczy się wyłącznie NADWYŻKA nad kontrolą."
    );

    // okna tygodniowe po dniach kalendarzowych
    let d_min = sc.iter().map(|s| s.dzien).min().unwrap();
    let okno_of = |d: i64| ((d - d_min) / 7) as usize;
    let n_okien = okno_of(sc.iter().map(|s| s.dzien).max().unwrap()) + 1;

    // macierz [okno][ekspert]; pomijamy wariant "NIE BIERŻ" (stałe zero)
    let eks: Vec<usize> = (1..nw).collect();
    let ranking_okien = |przypisanie: &dyn Fn(usize) -> usize| -> Vec<Vec<f64>> {
        let mut t = vec![vec![0.0f64; eks.len()]; n_okien];
        for (i, r) in m.iter().enumerate() {
            let o = przypisanie(i);
            for (j, k) in eks.iter().enumerate() {
                t[o][j] += r[*k];
            }
        }
        t
    };

    // Kendall tau-a między dwoma rankingami (par zgodnych minus niezgodnych)
    let tau = |a: &[f64], b: &[f64]| -> f64 {
        let n_ = a.len();
        let (mut zg, mut nz) = (0i64, 0i64);
        for i in 0..n_ {
            for j in (i + 1)..n_ {
                let x = (a[i] - a[j]).signum();
                let y = (b[i] - b[j]).signum();
                if x * y > 0.0 {
                    zg += 1;
                } else if x * y < 0.0 {
                    nz += 1;
                }
            }
        }
        let par = (n_ * (n_ - 1) / 2) as f64;
        (zg - nz) as f64 / par.max(1.0)
    };

    let policz = |t: &[Vec<f64>]| -> (usize, f64, usize) {
        let mut liderzy: Vec<usize> = Vec::new();
        for w in t.iter() {
            let mut naj = (0usize, f64::NEG_INFINITY);
            for (j, v) in w.iter().enumerate() {
                if *v > naj.1 {
                    naj = (j, *v);
                }
            }
            liderzy.push(naj.0);
        }
        let zmiany = liderzy.windows(2).filter(|p| p[0] != p[1]).count();
        let unikalnych = {
            let mut u = liderzy.clone();
            u.sort_unstable();
            u.dedup();
            u.len()
        };
        let sr_tau = if t.len() < 2 {
            1.0
        } else {
            t.windows(2).map(|p| tau(&p[0], &p[1])).sum::<f64>() / (t.len() - 1) as f64
        };
        (zmiany, sr_tau, unikalnych)
    };

    let t_real = ranking_okien(&|i| okno_of(sc[i].dzien));
    let (zm_r, tau_r, un_r) = policz(&t_real);

    // KONTROLA: te same wypłaty, okna przypisane losowo
    let mut rr = Rng(a.seed | 11);
    let losowe_okna: Vec<usize> = (0..n).map(|_| rr.ile(n_okien)).collect();
    let t_los = ranking_okien(&|i| losowe_okna[i]);
    let (zm_l, tau_l, un_l) = policz(&t_los);

    println!(
        "\n  {} okien tygodniowych, {} ekspertów (warianty zarządzania bez wariantu NIE BIERŻ)",
        n_okien,
        eks.len()
    );
    println!("\n  {:<34} {:>12} {:>12}", "miara", "DANE", "KONTROLA");
    println!(
        "  {:<34} {:>12} {:>12}",
        "zmian lidera między oknami", zm_r, zm_l
    );
    println!(
        "  {:<34} {:>12} {:>12}",
        "różnych ekspertów na czele", un_r, un_l
    );
    println!(
        "  {:<34} {:>12.4} {:>12.4}",
        "śr. Kendall tau kolejnych okien", tau_r, tau_l
    );

    print!("\n  lider w kolejnych oknach: ");
    for w in t_real.iter() {
        let mut naj = (0usize, f64::NEG_INFINITY);
        for (j, v) in w.iter().enumerate() {
            if *v > naj.1 {
                naj = (j, *v);
            }
        }
        print!("{} ", naj.0);
    }
    println!();

    let nadwyzka = zm_r as i64 - zm_l as i64;
    println!(
        "\n  -> nadwyżka zmian nad kontrolą: {:+}   [{}]",
        nadwyzka,
        if tau_r > tau_l + 0.15 {
            "ranking jest STABILNIEJSZY niż szum — jest co śledzić"
        } else if nadwyzka <= 0 {
            "przestawienia NIE RÓŻNIĄ SIĘ od szumu — agregacja nie ma czego śledzić"
        } else {
            "nierozstrzygnięte"
        }
    );
    println!(
        "  Czytanie: wysoka tau w DANYCH przy niskiej w KONTROLI znaczy, że ranking\n\
         \x20 utrzymuje się między oknami i zmienia rzadko, ale REALNIE. Tau bliskie\n\
         \x20 kontroli znaczy, że każde okno daje inną kolejność z powodu szumu."
    );

    println!(
        "\nUWAGA: pomiar NA ŚCIEŻKACH, lot stały 0,01/warstwę, bez limitu jednoczesnych\n\
         pozycji i bez opóźnienia. Horyzont godzinowy, bo symulator nie liczy swapu\n\
         (stawki zależą od brokera, symbolu i kierunku pozycji)."
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

    fn sciezka(probki: Vec<Probka>, tp1: Option<f32>, ts_tp1: Option<Ts>, koniec: f32) -> Sciezka {
        Sciezka {
            sygnal: 0,
            side_buy: true,
            id_sygnalu: 1,
            dzien: 0,
            probki,
            wych_tp1: tp1,
            ts_tp1,
            wych_koniec: koniec,
            wych_runner: koniec,
            szczyt: 0.0,
            ryzyko_wej: 6.0,
            powod: Powod::Horyzont,
            rf: vec![0.0; RF_WARIANTY.len()],
            rf_ok: vec![false; RF_WARIANTY.len()],
            wypelnionych: 1,
            ts_wyp: vec![0],
        }
    }

    const M: Ts = 60_000;

    #[test]
    fn wariant_pas_zawsze_zero() {
        let s = sciezka(vec![pr(0, 0.0), pr(M, -50.0)], Some(5.0), Some(M), -50.0);
        let w = Wariant {
            tp1: true,
            zycie_min: 60,
            pas: true,
        };
        assert_eq!(wyplata(&s, w), 0.0);
    }

    #[test]
    fn tp1_liczy_sie_tylko_gdy_padl_przed_limitem() {
        // TP1 pada w 10. minucie
        let s = sciezka(
            vec![pr(0, 0.0), pr(10 * M, 3.0), pr(30 * M, -40.0)],
            Some(3.0),
            Some(10 * M),
            -40.0,
        );
        // limit 30 min: TP1 zdążył → +3
        let a = wyplata(
            &s,
            Wariant {
                tp1: true,
                zycie_min: 30,
                pas: false,
            },
        );
        assert!((a - 3.0).abs() < 1e-9, "{a}");
        // limit 5 min: TP1 NIE zdążył → wychylenie w 5. minucie (próbka z 0) = 0
        let b = wyplata(
            &s,
            Wariant {
                tp1: true,
                zycie_min: 5,
                pas: false,
            },
        );
        assert!((b - 0.0).abs() < 1e-9, "{b}");
    }

    #[test]
    fn bez_celu_ignoruje_tp1() {
        let s = sciezka(
            vec![pr(0, 0.0), pr(10 * M, 3.0), pr(30 * M, -40.0)],
            Some(3.0),
            Some(10 * M),
            -40.0,
        );
        let a = wyplata(
            &s,
            Wariant {
                tp1: false,
                zycie_min: 0,
                pas: false,
            },
        );
        assert!((a + 40.0).abs() < 1e-9, "bez celu ma zignorować TP1: {a}");
    }

    #[test]
    fn limit_dluzszy_niz_sciezka_nie_gubi_stopu() {
        // ścieżka ginie na SL w 60. minucie; ostatnia PRÓBKA jest w 50. minucie
        // i pokazuje jeszcze +5 $. Limit 240 min wypada PO końcu ścieżki, więc
        // wynikiem musi być realizacja na stopie (−30), a nie stan sprzed niego.
        let s = sciezka(vec![pr(0, 0.0), pr(50 * M, 5.0)], None, None, -30.0);
        let a = wyplata(
            &s,
            Wariant {
                tp1: false,
                zycie_min: 240,
                pas: false,
            },
        );
        assert!(
            (a + 30.0).abs() < 1e-9,
            "limit po końcu ścieżki musi dać wynik końcowy: {a}"
        );
        let b = wyplata(
            &s,
            Wariant {
                tp1: false,
                zycie_min: 0,
                pas: false,
            },
        );
        assert!(
            (a - b).abs() < 1e-9,
            "limit dłuższy niż ścieżka = brak limitu: {a} vs {b}"
        );
    }

    #[test]
    fn limit_czasu_ucina_po_cenie_rynkowej() {
        let s = sciezka(
            vec![pr(0, 0.0), pr(10 * M, 20.0), pr(60 * M, -30.0)],
            None,
            None,
            -30.0,
        );
        let a = wyplata(
            &s,
            Wariant {
                tp1: false,
                zycie_min: 30,
                pas: false,
            },
        );
        assert!(
            (a - 20.0).abs() < 1e-9,
            "ma wziąć ostatnią próbkę w granicy: {a}"
        );
    }

    #[test]
    fn przestrzen_bez_rozrzutu_jest_wykrywalna() {
        // ścieżka, na której wszystkie warianty dają to samo (płaska, bez TP1)
        let s = sciezka(
            vec![pr(0, 0.0), pr(M, 0.0), pr(600 * M, 0.0)],
            None,
            None,
            0.0,
        );
        let w = warianty();
        let r: Vec<f64> = w.iter().map(|(_, v)| wyplata(&s, *v)).collect();
        assert!(
            odch(&r) < 1e-9,
            "płaska ścieżka musi dać zerowy rozrzut: {r:?}"
        );
    }

    #[test]
    fn rozrzut_istnieje_gdy_sciezka_sie_rusza() {
        let s = sciezka(
            vec![pr(0, 0.0), pr(20 * M, 25.0), pr(300 * M, -30.0)],
            Some(3.0),
            Some(5 * M),
            -30.0,
        );
        let w = warianty();
        let r: Vec<f64> = w.iter().map(|(_, v)| wyplata(&s, *v)).collect();
        assert!(
            odch(&r) > 1.0,
            "ruchoma ścieżka musi różnicować warianty: {r:?}"
        );
        let mx = r.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (mx - 25.0).abs() < 1e-9,
            "najlepszy wariant to szczyt 25 $: {mx}"
        );
    }
}
