
use anyhow::{bail, Result};
use conduit_ai::peak::*;
use conduit_ai::policy::Scratch;
use conduit_backtest::{load_signals, TickData};

// ============================================================
//  ARGUMENTY
// ============================================================

struct Args {
    ticks: String,
    signals: String,
    krok_s: i64,
    horyzont_h: i64,
    epok: usize,
    lr: f32,
    ukryte: Vec<usize>,
    seed: u64,
    msg_offset_min: f64,
    jednostki: usize,
    n_r: f64,
    koszt: f64,
    od: Option<String>,
    wf: usize,
    tylko_rf: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            ticks: "data/ticks.bin".into(),
            signals: "data/signals.json".into(),
            krok_s: 60,
            horyzont_h: 6,
            epok: 40,
            lr: 0.05,
            ukryte: vec![24, 16],
            seed: 7,
            msg_offset_min: 180.0,
            jednostki: 3,
            n_r: 6.0,
            // spread 0,24 $ na jednostkę ceny × 3 jednostki koszyka
            koszt: 0.24 * 3.0,
            od: None,
            wf: 4,
            tylko_rf: false,
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
            "--epok" => a.epok = nast!().parse()?,
            "--lr" => a.lr = nast!().parse()?,
            "--ukryte" => {
                a.ukryte = nast!()
                    .split(',')
                    .filter_map(|x| x.trim().parse().ok())
                    .collect()
            }
            "--seed" => a.seed = nast!().parse()?,
            "--msg-offset-min" => a.msg_offset_min = nast!().parse()?,
            "--jednostki" => a.jednostki = nast!().parse()?,
            "--n-r" => a.n_r = nast!().parse()?,
            "--koszt" => a.koszt = nast!().parse()?,
            "--od" => a.od = Some(nast!()),
            "--wf" => a.wf = nast!().parse()?,
            "--tylko-rf" => a.tylko_rf = true,
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


pub struct Izotonik {
    x: Vec<f32>,
    y: Vec<f32>,
}

impl Izotonik {
    pub fn ucz(p: &[f32], y: &[f32]) -> Izotonik {
        assert_eq!(p.len(), y.len());
        let mut idx: Vec<usize> = (0..p.len()).collect();
        idx.sort_by(|a, b| {
            p[*a]
                .partial_cmp(&p[*b])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut sy: Vec<f64> = Vec::with_capacity(idx.len());
        let mut sx: Vec<f64> = Vec::with_capacity(idx.len());
        let mut n: Vec<f64> = Vec::with_capacity(idx.len());
        for &i in &idx {
            sy.push(y[i] as f64);
            sx.push(p[i] as f64);
            n.push(1.0);
            while sy.len() >= 2 {
                let k = sy.len();
                if sy[k - 2] / n[k - 2] <= sy[k - 1] / n[k - 1] {
                    break;
                }
                let (a, b, c) = (sy.pop().unwrap(), sx.pop().unwrap(), n.pop().unwrap());
                let k = sy.len();
                sy[k - 1] += a;
                sx[k - 1] += b;
                n[k - 1] += c;
            }
        }
        Izotonik {
            x: (0..sy.len()).map(|k| (sx[k] / n[k]) as f32).collect(),
            y: (0..sy.len()).map(|k| (sy[k] / n[k]) as f32).collect(),
        }
    }

    /// Interpolacja liniowa między reprezentantami bloków, na krańcach stała.
    pub fn pred(&self, p: f32) -> f32 {
        if self.x.is_empty() {
            return 0.5;
        }
        if p <= self.x[0] {
            return self.y[0];
        }
        if p >= self.x[self.x.len() - 1] {
            return self.y[self.y.len() - 1];
        }
        let k = self.x.partition_point(|v| *v <= p);
        let (x0, x1) = (self.x[k - 1], self.x[k]);
        let (y0, y1) = (self.y[k - 1], self.y[k]);
        if (x1 - x0).abs() < 1e-9 {
            return y1;
        }
        (y0 + (y1 - y0) * (p - x0) / (x1 - x0)).clamp(0.0, 1.0)
    }
}

/// Kalibracja Platta: `σ(a·logit(p) + b)`.
pub struct Platt {
    a: f64,
    b: f64,
}

#[inline]
fn logit(p: f32) -> f64 {
    let q = (p as f64).clamp(1e-6, 1.0 - 1e-6);
    (q / (1.0 - q)).ln()
}

impl Platt {
    pub fn ucz(p: &[f32], y: &[f32], kroki: usize, lr: f64) -> Platt {
        let (mut a, mut b) = (1.0f64, 0.0f64);
        let n = p.len().max(1) as f64;
        // wygładzenie celu (Platt 1999) — chroni przed rozbieganiem a → ∞
        let np = y.iter().filter(|v| **v > 0.5).count() as f64;
        let nn = n - np;
        let (hi, lo) = ((np + 1.0) / (np + 2.0), 1.0 / (nn + 2.0));
        let z: Vec<f64> = p.iter().map(|v| logit(*v)).collect();
        for _ in 0..kroki {
            let (mut ga, mut gb) = (0.0f64, 0.0f64);
            for k in 0..p.len() {
                let t = if y[k] > 0.5 { hi } else { lo };
                let s = 1.0 / (1.0 + (-(a * z[k] + b)).exp());
                let d = s - t;
                ga += d * z[k];
                gb += d;
            }
            a -= lr * ga / n;
            b -= lr * gb / n;
        }
        Platt { a, b }
    }
    pub fn pred(&self, p: f32) -> f32 {
        (1.0 / (1.0 + (-(self.a * logit(p) + self.b)).exp())) as f32
    }
}

pub fn brier(p: &[f32], y: &[f32]) -> f64 {
    if p.is_empty() {
        return f64::NAN;
    }
    p.iter()
        .zip(y)
        .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
        .sum::<f64>()
        / p.len() as f64
}

pub fn logstrata(p: &[f32], y: &[f32]) -> f64 {
    if p.is_empty() {
        return f64::NAN;
    }
    p.iter()
        .zip(y)
        .map(|(a, b)| {
            let q = (*a as f64).clamp(1e-6, 1.0 - 1e-6);
            if *b > 0.5 {
                -q.ln()
            } else {
                -(1.0 - q).ln()
            }
        })
        .sum::<f64>()
        / p.len() as f64
}

/// Oczekiwany błąd kalibracji — kubełki o RÓWNEJ LICZEBNOŚCI.
pub fn ece(p: &[f32], y: &[f32], k: usize) -> f64 {
    if p.len() < k {
        return f64::NAN;
    }
    let mut idx: Vec<usize> = (0..p.len()).collect();
    idx.sort_by(|a, b| {
        p[*a]
            .partial_cmp(&p[*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let d = p.len() / k;
    let mut suma = 0.0;
    for b in 0..k {
        let s = &idx[b * d..if b + 1 == k { p.len() } else { (b + 1) * d }];
        let sp: f64 = s.iter().map(|i| p[*i] as f64).sum::<f64>() / s.len() as f64;
        let sy: f64 = s.iter().map(|i| y[*i] as f64).sum::<f64>() / s.len() as f64;
        suma += (sp - sy).abs() * s.len() as f64;
    }
    suma / p.len() as f64
}

fn tabela_niezawodnosci(nazwa: &str, p: &[f32], y: &[f32], k: usize) {
    if p.len() < k * 2 {
        println!("  [{nazwa}] za mało próbek na tabelę");
        return;
    }
    let mut idx: Vec<usize> = (0..p.len()).collect();
    idx.sort_by(|a, b| {
        p[*a]
            .partial_cmp(&p[*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let d = p.len() / k;
    print!("  {nazwa:<14}");
    for b in 0..k {
        let s = &idx[b * d..if b + 1 == k { p.len() } else { (b + 1) * d }];
        let sp: f64 = s.iter().map(|i| p[*i] as f64).sum::<f64>() / s.len() as f64;
        let sy: f64 = s.iter().map(|i| y[*i] as f64).sum::<f64>() / s.len() as f64;
        print!(" {sp:.2}/{sy:.2}");
    }
    println!();
}


#[derive(Clone, Copy, PartialEq, Eq)]
enum Cel {
    Suma,
    MedianaDzienna,
    SredniaPrzycieta,
    SharpeDzienny,
}

impl Cel {
    fn nazwa(self) -> &'static str {
        match self {
            Cel::Suma => "suma dolarów (dotychczasowa)",
            Cel::MedianaDzienna => "MEDIANA dziennego wyniku",
            Cel::SredniaPrzycieta => "średnia dzienna PRZYCIĘTA 10 %",
            Cel::SharpeDzienny => "zysk ważony ryzykiem (śr./odch. dz.)",
        }
    }
    fn krotka(self) -> &'static str {
        match self {
            Cel::Suma => "suma",
            Cel::MedianaDzienna => "mediana",
            Cel::SredniaPrzycieta => "przycięta",
            Cel::SharpeDzienny => "ważony",
        }
    }
}

const CELE: [Cel; 4] = [
    Cel::Suma,
    Cel::MedianaDzienna,
    Cel::SredniaPrzycieta,
    Cel::SharpeDzienny,
];

fn dni_sumy(w: &[(i64, f64)]) -> Vec<f64> {
    let mut m: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for (d, v) in w {
        *m.entry(*d).or_insert(0.0) += v;
    }
    m.values().cloned().collect()
}

fn ocena(cel: Cel, w: &[(i64, f64)]) -> f64 {
    let mut v = dni_sumy(w);
    if v.is_empty() {
        return f64::NEG_INFINITY;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    match cel {
        Cel::Suma => v.iter().sum(),
        Cel::MedianaDzienna => {
            let n = v.len();
            if n % 2 == 1 {
                v[n / 2]
            } else {
                (v[n / 2 - 1] + v[n / 2]) * 0.5
            }
        }
        Cel::SredniaPrzycieta => {
            let k = (v.len() as f64 * 0.10).floor() as usize;
            if v.len() <= 2 * k + 1 {
                return v.iter().sum::<f64>() / v.len() as f64;
            }
            let s = &v[k..v.len() - k];
            s.iter().sum::<f64>() / s.len() as f64
        }
        Cel::SharpeDzienny => {
            let n = v.len() as f64;
            let sr = v.iter().sum::<f64>() / n;
            let sd = (v.iter().map(|x| (x - sr).powi(2)).sum::<f64>() / n).sqrt();
            if sd < 1e-9 {
                if sr > 0.0 {
                    99.0
                } else {
                    -99.0
                }
            } else {
                sr / sd * n.sqrt()
            }
        }
    }
}

// ============================================================
//  POLITYKI NA GOTOWYCH PREDYKCJACH
// ============================================================

/// Decyzja JEDNORAZOWA przy wejściu: `p ≥ prog` → puszczamy bez celu, inaczej TP1.
fn pol_wejscie(sc: &[Sciezka], p0: &[f32], prog: f32) -> Vec<(i64, f64)> {
    sc.iter()
        .zip(p0)
        .map(|(s, p)| {
            (
                s.dzien,
                if *p >= prog {
                    s.wych_koniec as f64
                } else {
                    s.tp1() as f64
                },
            )
        })
        .collect()
}

/// Decyzja CIĄGŁA z zatrzaskiem: domyślnie TP1, ale do chwili dotknięcia TP1
/// model może zmienić zdanie na „puść bez celu". Po dotknięciu TP1 pozycji już
/// nie ma, więc decyzja nie może zapaść później — inaczej model zmieniałby
/// zdanie po fakcie.
fn pol_ciagla_zatrzask(sc: &[Sciezka], p: &[Vec<f32>], prog: f32) -> Vec<(i64, f64)> {
    sc.iter()
        .zip(p)
        .map(|(s, pv)| {
            for (k, pr) in pv.iter().enumerate() {
                if let Some(t) = s.ts_tp1 {
                    if s.probki[k].ts > t {
                        break;
                    }
                }
                if *pr >= prog {
                    return (s.dzien, s.wych_koniec as f64);
                }
            }
            (s.dzien, s.tp1() as f64)
        })
        .collect()
}

/// Decyzja podjęta DOKŁADNIE w chwili dotknięcia TP1 (ostatnia próbka przed nim).
/// Wariant bez zatrzasku: liczy się zdanie modelu w momencie, w którym pozycja
/// faktycznie ma wyjść.
fn pol_w_tp1(sc: &[Sciezka], p: &[Vec<f32>], prog: f32) -> Vec<(i64, f64)> {
    sc.iter()
        .zip(p)
        .map(|(s, pv)| {
            let mut ost = pv[0];
            if let Some(t) = s.ts_tp1 {
                for (k, pr) in pv.iter().enumerate() {
                    if s.probki[k].ts > t {
                        break;
                    }
                    ost = *pr;
                }
            }
            (
                s.dzien,
                if ost >= prog {
                    s.wych_koniec as f64
                } else {
                    s.tp1() as f64
                },
            )
        })
        .collect()
}

/// „Zamknij teraz kontra kontynuuj" — wariant, który DEGENERUJE i jest tu po to,
/// żeby to pokazać liczbą, a nie zdaniem.
fn pol_zamknij_teraz(sc: &[Sciezka], p: &[Vec<f32>], prog: f32) -> Vec<(i64, f64)> {
    sc.iter()
        .zip(p)
        .map(|(s, pv)| {
            for (k, pr) in pv.iter().enumerate() {
                if *pr < prog {
                    return (s.dzien, s.probki[k].wych as f64);
                }
            }
            (s.dzien, s.wych_koniec as f64)
        })
        .collect()
}

fn ile_ge(p0: &[f32], prog: f32) -> usize {
    p0.iter().filter(|v| **v >= prog).count()
}

// ============================================================
//  POMOCNICZE
// ============================================================

fn kwantyle(mut v: Vec<f32>, ile: usize) -> Vec<f32> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if v.is_empty() {
        return vec![0.0];
    }
    let mut out: Vec<f32> = (1..ile).map(|k| v[k * v.len() / ile]).collect();
    out.dedup();
    out
}

fn kwantyl(v: &[f32], q: f64) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s[((s.len() as f64 - 1.0) * q).round() as usize]
}

fn nag() {
    println!(
        "  {:<48} {:>9}  {:>6}  {:>7}  {:>7}  {:>8}   {}",
        "polityka", "PnL $", "PF", "trafień", "dni +", "maxDD $", "95 % (bootstrap po dniach)"
    );
}

fn wiersz(nazwa: &str, w: &Wynik, ci: Option<(f64, f64)>) {
    let pf = if w.pf >= 999.0 {
        "∞".to_string()
    } else {
        format!("{:.2}", w.pf)
    };
    match ci {
        Some((lo, hi)) => println!(
            "  {:<48} {:>+9.2}  {:>6}  {:>6.1} %  {:>6.1} %  {:>8.2}   [{:+.0} … {:+.0}]",
            nazwa, w.pnl, pf, w.trafien, w.dni_plus, w.maxdd, lo, hi
        ),
        None => println!(
            "  {:<48} {:>+9.2}  {:>6}  {:>6.1} %  {:>6.1} %  {:>8.2}",
            nazwa, w.pnl, pf, w.trafien, w.dni_plus, w.maxdd
        ),
    }
}

/// Luka wyjścia każdej ścieżki: ile zostawia polityka TP1 wobec szczytu.
fn luki(sc: &[Sciezka]) -> Vec<(usize, f64)> {
    let mut v: Vec<(usize, f64)> = sc
        .iter()
        .enumerate()
        .map(|(i, s)| (i, (s.szczyt - s.tp1()) as f64))
        .collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v
}

/// Ile z dziesięciu największych luk wyjścia trafia do górnego decyla rankingu.
/// Zwraca `(złapane z 10, udział luki w decylu %, suma luki $)`.
fn koncentracja(sc: &[Sciezka], ranking: &[f32]) -> (usize, f64, f64) {
    let lk = luki(sc);
    let suma: f64 = lk.iter().map(|v| v.1.max(0.0)).sum();
    let mut oc: Vec<(usize, f32)> = ranking.iter().cloned().enumerate().collect();
    oc.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let decyl: std::collections::HashSet<usize> = oc
        .iter()
        .take((sc.len() / 10).max(1))
        .map(|v| v.0)
        .collect();
    let zlapane = lk.iter().take(10).filter(|v| decyl.contains(&v.0)).count();
    let w_decylu: f64 = lk
        .iter()
        .filter(|v| decyl.contains(&v.0))
        .map(|v| v.1.max(0.0))
        .sum();
    (zlapane, w_decylu / suma.max(1e-9) * 100.0, suma)
}

/// Ranking oceniany nie przy wejściu, tylko w chwili, gdy koszyk pierwszy raz
/// osiąga `k_r` × ryzyko. Ścieżki, które nigdy tam nie docierają, lądują na dnie.
///
/// To jest pytanie „czy ogon staje się rozpoznawalny, KIEDY ruch już się zaczął",
/// a nie „czy da się go przewidzieć przed startem".
fn ranking_w_punkcie(sc: &[Sciezka], p: &[Vec<f32>], k_r: f32) -> (Vec<f32>, usize) {
    let mut ile = 0;
    let v = sc
        .iter()
        .zip(p)
        .map(|(s, pv)| {
            for (k, pr) in pv.iter().enumerate() {
                if s.probki[k].wych >= k_r * s.probki[k].ryzyko {
                    ile += 1;
                    return *pr;
                }
            }
            -1.0
        })
        .collect();
    (v, ile)
}

/// Predykcja i etykieta DOKŁADNIE w chwili osiągnięcia `k_r` × ryzyko.
/// Bez tego nie da się powiedzieć, czy model ma sygnał w późniejszym punkcie
/// decyzyjnym, czy tylko korzysta z tego, że pozycja dotąd dożyła.
fn auc_w_punkcie(sc: &[Sciezka], p: &[Vec<f32>], k_r: f32) -> (f64, usize) {
    let mut sk = Vec::new();
    let mut y = Vec::new();
    for (s, pv) in sc.iter().zip(p) {
        for (k, pr) in pv.iter().enumerate() {
            if s.probki[k].wych >= k_r * s.probki[k].ryzyko {
                sk.push(*pr);
                y.push(s.probki[k].y_kl);
                break;
            }
        }
    }
    (auc(&sk, &y), sk.len())
}

/// KONTROLA dla rankingu w późniejszym punkcie: sam fakt „doszedł do k·R"
/// przesuwa ścieżkę w stronę dużej luki niezależnie od modelu. Zwraca
/// `(ile z 10 największych luk w ogóle doszło, oczekiwane trafienia przy
/// losowej kolejności wewnątrz tej grupy)`.
fn kontrola_punktu(sc: &[Sciezka], k_r: f32) -> (usize, f64, usize) {
    let doszedl: Vec<bool> = sc
        .iter()
        .map(|s| s.probki.iter().any(|p| p.wych >= k_r * p.ryzyko))
        .collect();
    let n_doszlo = doszedl.iter().filter(|v| **v).count();
    let lk = luki(sc);
    let top10_doszlo = lk.iter().take(10).filter(|v| doszedl[v.0]).count();
    let decyl = (sc.len() / 10).max(1);
    let ocz = if n_doszlo == 0 {
        0.0
    } else {
        top10_doszlo as f64 * (decyl.min(n_doszlo) as f64 / n_doszlo as f64)
    };
    (top10_doszlo, ocz, n_doszlo)
}

/// KONTROLA: model uczony **wyłącznie na punkcie decyzyjnym** (jedna próbka na
/// ścieżkę), a nie na całym przebiegu.
///
/// Bez niej zarzut „AUC przy wejściu jest niskie tylko dlatego, że model widział
/// głównie próbki śródścieżkowe" zostaje nierozstrzygnięty. Tu populacja uczenia
/// i populacja stosowania są IDENTYCZNE, więc jeśli AUC dalej siedzi przy 0,50,
/// to znaczy, że w tym punkcie po prostu nie ma czego przewidywać.
fn model_na_punkcie(
    a: &Args,
    ucz: &[Sciezka],
    test: &[Sciezka],
    k_r: f32,
) -> (f64, f64, usize, usize) {
    let zbierz = |sc: &[Sciezka]| -> (Vec<Vec<f32>>, Vec<f32>) {
        let mut x = Vec::new();
        let mut y = Vec::new();
        for s in sc {
            if k_r < 0.0 {
                x.push(s.probki[0].x.clone());
                y.push(s.probki[0].y_kl);
            } else {
                for p in &s.probki {
                    if p.wych >= k_r * p.ryzyko {
                        x.push(p.x.clone());
                        y.push(p.y_kl);
                        break;
                    }
                }
            }
        }
        (x, y)
    };
    let (xu, yu) = zbierz(ucz);
    let (xt, yt) = zbierz(test);
    if xu.len() < 50 || xt.len() < 50 {
        return (f64::NAN, f64::NAN, xu.len(), xt.len());
    }
    let lin = RegLog::ucz(&xu, &yu, 400, 3.0, 1e-4);
    // mniejsza sieć: przy kilkuset próbkach [24,16] przeucza się z definicji
    let m = ucz_klas(&xu, &yu, &[12, 8], 400, a.lr * 4.0, a.seed + 11);
    let mut sc_ = Scratch::for_net(&m);
    let pn: Vec<f32> = xt.iter().map(|v| pred_klas(&m, &mut sc_, v)).collect();
    let pl: Vec<f32> = xt.iter().map(|v| lin.pred(v)).collect();
    (auc(&pn, &yt), auc(&pl, &yt), xu.len(), xt.len())
}

/// Pełny raport RISK FREE na zadanym zbiorze ścieżek, BEZ MODELU.
fn raport_rf(sc: &[Sciezka], etykieta: &str, ci_seed: u64) {
    let w_tp1 = podsumuj(&pol_tp1(sc));
    let w_run = podsumuj(&pol_runner(sc));
    let w_bez = podsumuj(&pol_bez_celu(sc));
    let w_suf = podsumuj(&pol_szczyt(sc));
    println!(
        "
  ── {etykieta} ({} ścieżek) ──",
        sc.len()
    );
    nag();
    wiersz("SUFIT: wyjście w szczycie", &w_suf, None);
    wiersz(
        "ODNIESIENIE: wszystko na TP1",
        &w_tp1,
        Some(bootstrap_dni(&pol_tp1(sc), 4000, ci_seed)),
    );
    wiersz("ODNIESIENIE: czempion (1×TP1 + runnery)", &w_run, None);
    wiersz("ODNIESIENIE: nic bez celu", &w_bez, None);
    println!("  {:-<48}", "");
    for vi in 0..RF_WARIANTY.len() {
        let v = pol_rf(sc, vi);
        let w = podsumuj(&v);
        let ile = sc.iter().filter(|s| s.rf_ok[vi]).count();
        wiersz(
            &format!("{} ({}/{})", RF_NAZWY[vi], ile, sc.len()),
            &w,
            Some(bootstrap_dni(&v, 4000, ci_seed)),
        );
    }
    println!("  {:-<48}", "");
    println!(
        "  WARIANT Z FALLBACKIEM „reszta na TP1\" — CZYTAĆ Z OSTRZEŻENIEM.
           Jest WYKONALNY tylko dla wyzwalacza „w TP1\": decyzja zapada dokładnie w chwili
           dotknięcia celu, więc pozycja, która celu nie dotknie, faktycznie gra TP1.
           Dla wyzwalaczy +k R jest NIEWYKONALNY i oznaczony ⚠: TP1 leży medianowo 3 $ od
           wejścia, a +3 R to ~18 $, więc pozycja MUSI najpierw porzucić TP1, żeby w ogóle
           dożyć wyzwalacza. Reguła „graj TP1, chyba że ta pozycja doszłaby do +3 R\" wymaga
           wiedzy o przyszłości. Liczby ⚠ są tu WYŁĄCZNIE po to, żeby pokazać skalę złudzenia."
    );
    let tp1v = pol_tp1(sc);
    let runv = pol_runner(sc);
    for vi in 0..RF_WARIANTY.len() {
        let v = pol_rf_lub_tp1(sc, vi);
        let w = podsumuj(&v);
        let znak = if RF_WARIANTY[vi].0 == 0.0 {
            ""
        } else {
            " ⚠NIEWYKONALNY"
        };
        wiersz(
            &format!("{} · reszta TP1{}", RF_NAZWY[vi], znak),
            &w,
            Some(bootstrap_dni(&v, 4000, ci_seed)),
        );
    }
    // Miara samego złudzenia: ile z „przewagi" znika, gdy fallback zostanie
    // odebrany i reguła stanie się wykonalna.
    println!(
        "
  SKALA ZŁUDZENIA (wariant niewykonalny → wykonalny):"
    );
    for vi in 0..RF_WARIANTY.len() {
        if RF_WARIANTY[vi].0 == 0.0 {
            continue;
        }
        let z_fb = podsumuj(&pol_rf_lub_tp1(sc, vi)).pnl;
        let bez = podsumuj(&pol_rf(sc, vi)).pnl;
        println!(
            "    {:<52} {:>+10.2} $ → {:>+9.2} $   (znika {:.0} %)",
            RF_NAZWY[vi],
            z_fb,
            bez,
            (z_fb - bez) / z_fb.abs().max(1e-9) * 100.0
        );
    }
    // RÓŻNICE SPAROWANE wobec najprostszej sensownej alternatywy.
    // Liczone dla wariantu Z FALLBACKIEM, bo to on daje duże liczby — i to on
    // musi udowodnić, że bije zwykłe „wszystko na TP1".
    println!(
        "
  czy RISK FREE Z FALLBACKIEM dokłada (różnica sparowana, bootstrap po dniach):"
    );
    for vi in 0..RF_WARIANTY.len() {
        let v = pol_rf_lub_tp1(sc, vi);
        for (nazwa, odn) in [("− wszystko na TP1", &tp1v), ("− czempion", &runv)] {
            let d: f64 = v.iter().zip(odn).map(|(x, y)| x.1 - y.1).sum();
            let (lo, hi) = bootstrap_roznicy(&v, odn, 4000, ci_seed);
            let werdykt = if lo > 0.0 {
                "DODAJE"
            } else if hi < 0.0 {
                "SZKODZI"
            } else {
                "nierozstrzygnięte"
            };
            println!(
                "    {:<50} {:<20} {:>+9.2} $  [{:+.0} … {:+.0}]  {}",
                RF_NAZWY[vi], nazwa, d, lo, hi, werdykt
            );
        }
    }
    // NAJWAŻNIEJSZE PORÓWNANIE: pełny RISK FREE wobec własnych kontroli
    // mechanizmu. Jeśli różnica jest bliska zeru, przewagi nie daje domykanie
    // warstw, tylko coś prostszego.
    println!(
        "
  co w tym mechanizmie faktycznie pracuje (różnica sparowana):"
    );
    for (a_, b_, opis) in [
        (
            4usize,
            9usize,
            "RF +3 R (domyka + BE)  −  sam SL na BE przy +3 R",
        ),
        (3, 8, "RF +2 R (domyka + BE)  −  sam SL na BE przy +2 R"),
        (
            9,
            11,
            "sam SL na BE przy +3 R  −  samo porzucenie TP1 przy +3 R",
        ),
        (
            8,
            10,
            "sam SL na BE przy +2 R  −  samo porzucenie TP1 przy +2 R",
        ),
        (4, 0, "RF +3 R  −  RF w TP1"),
    ] {
        let x = pol_rf_lub_tp1(sc, a_);
        let y = pol_rf_lub_tp1(sc, b_);
        let d: f64 = x.iter().zip(&y).map(|(u, v)| u.1 - v.1).sum();
        let (lo, hi) = bootstrap_roznicy(&x, &y, 4000, ci_seed);
        let werdykt = if lo > 0.0 {
            "DODAJE"
        } else if hi < 0.0 {
            "SZKODZI"
        } else {
            "nierozstrzygnięte"
        };
        println!(
            "    {:<58} {:>+9.2} $  [{:+.0} … {:+.0}]  {}",
            opis, d, lo, hi, werdykt
        );
    }
}

// ============================================================
//  STRUKTURA WYNIKU FOLDU
// ============================================================

struct WynikFoldu {
    progi: Vec<(Cel, f32, f64, usize, f64)>, // cel, próg, PnL testu, puszczonych, pokrycie
    ev_prog: f32,
    ev_prog_lin: f32,
    ev_pnl: f64,
    ev_pnl_lin: f64,
    ev_ciagla_pnl: f64,
    ev_w_tp1_pnl: f64,
    ev_zamknij_pnl: f64,
    ev_g: f64,
    ev_l: f64,
    czempion: f64,
    tp1: f64,
    bez: f64,
    ogon_zlapane: usize,
    ogon_zlapane_lin: usize,
    ogon_reg_zlapane: usize,
    biegacz_zlapane: usize,
    najlepsza_cecha: (String, usize),
    zlapane_w_1r: usize,
    zlapane_w_2r: usize,
    ocz_w_1r: f64,
    ocz_w_2r: f64,
    auc_we: f64,
    auc_we_lin: f64,
    auc_probki: f64,
    auc_1r: f64,
    auc_2r: f64,
    ogon_pnl: f64,
    auc_ogon: f64,
    auc_ogon_lin: f64,
    brier_sur: f64,
    brier_izo: f64,
    brier_plt: f64,
    ece_sur: f64,
    ece_izo: f64,
    ece_plt: f64,
    brier_we_sur: f64,
    brier_we_izo: f64,
    ece_we_sur: f64,
    ece_we_izo: f64,
}

// ============================================================
//  JEDEN FOLD
// ============================================================

#[allow(clippy::too_many_lines)]
fn fold(
    a: &Args,
    ucz: &[Sciezka],
    kal: &[Sciezka],
    test: &[Sciezka],
    etykieta: &str,
) -> WynikFoldu {
    println!(
        "\n╔═══════════════════════════════════════════════════════════════════════════════════╗"
    );
    println!("║ {etykieta:<81} ║");
    println!(
        "╚═══════════════════════════════════════════════════════════════════════════════════╝"
    );
    let pu = plaskie(ucz);
    let pk = plaskie(kal);
    let pt = plaskie(test);
    let po_sc = |v: &[Sciezka]| {
        v.iter().filter(|z| z.probki[0].y_kl > 0.5).count() as f64 / v.len().max(1) as f64 * 100.0
    };
    println!(
        "  uczenie {} ścieżek / {} próbek · KALIBRACJA+PRÓG {} / {} · TEST {} / {}",
        ucz.len(),
        pu.len(),
        kal.len(),
        pk.len(),
        test.len(),
        pt.len()
    );
    println!(
        "  udział biegaczy ≥ {:.0} R: po PRÓBKACH ucz {:.1} % / test {:.1} % · \
         po ŚCIEŻKACH przy wejściu ucz {:.1} % / test {:.1} %",
        a.n_r,
        pu.iter().map(|p| p.y_kl).sum::<f32>() as f64 / pu.len().max(1) as f64 * 100.0,
        pt.iter().map(|p| p.y_kl).sum::<f32>() as f64 / pt.len().max(1) as f64 * 100.0,
        po_sc(ucz),
        po_sc(test)
    );

    let xu: Vec<Vec<f32>> = pu.iter().map(|p| p.x.clone()).collect();
    let yu_k: Vec<f32> = pu.iter().map(|p| p.y_kl).collect();

    // ---------- GŁOWA „BIEGACZ ≥ N·R" + odniesienie liniowe ----------
    let m_k = ucz_klas(&xu, &yu_k, &a.ukryte, a.epok, a.lr * 4.0, a.seed + 1);
    let lin_k = RegLog::ucz(&xu, &yu_k, 300, 3.0, 1e-4);

    let sur = |sc: &[Sciezka]| -> Vec<Vec<f32>> {
        let mut s = Scratch::for_net(&m_k);
        sc.iter()
            .map(|z| {
                z.probki
                    .iter()
                    .map(|p| pred_klas(&m_k, &mut s, &p.x))
                    .collect()
            })
            .collect()
    };
    let sur_kal = sur(kal);
    let sur_test = sur(test);
    let sur_kal_pl: Vec<f32> = sur_kal.iter().flatten().cloned().collect();
    let sur_test_pl: Vec<f32> = sur_test.iter().flatten().cloned().collect();
    let sur_kal_we: Vec<f32> = sur_kal.iter().map(|v| v[0]).collect();
    let sur_test_we: Vec<f32> = sur_test.iter().map(|v| v[0]).collect();
    let yk_kal: Vec<f32> = pk.iter().map(|p| p.y_kl).collect();
    let yk_test: Vec<f32> = pt.iter().map(|p| p.y_kl).collect();
    let yw_kal: Vec<f32> = kal.iter().map(|z| z.probki[0].y_kl).collect();
    let yw_test: Vec<f32> = test.iter().map(|z| z.probki[0].y_kl).collect();

    println!("\n───── §3.3 KALIBRACJA PRAWDOPODOBIEŃSTW ─────");
    // ROZBICIE AUC NA PUNKTY DECYZYJNE. AUC liczone po WSZYSTKICH próbkach
    // mierzy w większości rozpoznawanie ruchu JUŻ TRWAJĄCEGO. Decyzja hybrydy
    // zapada przy wejściu — i tam trzeba je zmierzyć osobno, inaczej całą
    // rodzinę polityk ocenia się liczbą, która ich nie dotyczy.
    let auc_lin_we: f64 = auc(
        &test
            .iter()
            .map(|z| lin_k.pred(&z.probki[0].x))
            .collect::<Vec<_>>(),
        &yw_test,
    );
    println!(
        "  AUC po WSZYSTKICH próbkach: sieć {:.4} · liniowy {:.4}",
        auc(&sur_test_pl, &yk_test),
        auc(
            &pt.iter().map(|p| lin_k.pred(&p.x)).collect::<Vec<_>>(),
            &yk_test
        )
    );
    println!(
        "  AUC PRZY WEJŚCIU (tam, gdzie faktycznie zapada decyzja): sieć {:.4} · liniowy {:.4}",
        auc(&sur_test_we, &yw_test),
        auc_lin_we
    );
    {
        let surtest_v = &sur_test;
        for kr in [0.5f32, 1.0, 2.0] {
            let (au, il) = auc_w_punkcie(test, surtest_v, kr);
            println!("  AUC W CHWILI +{kr} R: sieć {au:.4} ({il} ścieżek doszło)");
        }
    }

    // DWA kalibratory: śródścieżkowy i wejściowy. Rozkłady są różne, bo długa
    // ścieżka daje 360 próbek, a krótka pięć.
    let izo = Izotonik::ucz(&sur_kal_pl, &yk_kal);
    let plt = Platt::ucz(&sur_kal_pl, &yk_kal, 400, 2.0);
    let izo_we = Izotonik::ucz(&sur_kal_we, &yw_kal);
    let plt_we = Platt::ucz(&sur_kal_we, &yw_kal, 400, 2.0);

    let izo_test: Vec<f32> = sur_test_pl.iter().map(|p| izo.pred(*p)).collect();
    let plt_test: Vec<f32> = sur_test_pl.iter().map(|p| plt.pred(*p)).collect();
    let izo_test_we: Vec<f32> = sur_test_we.iter().map(|p| izo_we.pred(*p)).collect();
    let plt_test_we: Vec<f32> = sur_test_we.iter().map(|p| plt_we.pred(*p)).collect();

    let (b0, b1, b2) = (
        brier(&sur_test_pl, &yk_test),
        brier(&izo_test, &yk_test),
        brier(&plt_test, &yk_test),
    );
    let (e0, e1, e2) = (
        ece(&sur_test_pl, &yk_test, 10),
        ece(&izo_test, &yk_test, 10),
        ece(&plt_test, &yk_test, 10),
    );
    let (bw0, bw1, bw2) = (
        brier(&sur_test_we, &yw_test),
        brier(&izo_test_we, &yw_test),
        brier(&plt_test_we, &yw_test),
    );
    let (ew0, ew1, ew2) = (
        ece(&sur_test_we, &yw_test, 10),
        ece(&izo_test_we, &yw_test, 10),
        ece(&plt_test_we, &yw_test, 10),
    );

    println!(
        "  {:<34} {:>9} {:>10} {:>9}",
        "populacja / wariant", "Brier", "log-strata", "ECE"
    );
    for (n_, p_, b_, e_) in [
        ("PO PRÓBKACH · surowy", &sur_test_pl, b0, e0),
        ("PO PRÓBKACH · izotoniczny", &izo_test, b1, e1),
        ("PO PRÓBKACH · Platt", &plt_test, b2, e2),
    ] {
        println!(
            "  {:<34} {:>9.4} {:>10.4} {:>9.4}",
            n_,
            b_,
            logstrata(p_, &yk_test),
            e_
        );
    }
    for (n_, p_, b_, e_) in [
        ("PRZY WEJŚCIU · surowy", &sur_test_we, bw0, ew0),
        ("PRZY WEJŚCIU · izotoniczny", &izo_test_we, bw1, ew1),
        ("PRZY WEJŚCIU · Platt", &plt_test_we, bw2, ew2),
    ] {
        println!(
            "  {:<34} {:>9.4} {:>10.4} {:>9.4}",
            n_,
            b_,
            logstrata(p_, &yw_test),
            e_
        );
    }
    println!(
        "  tabela niezawodności PO PRÓBKACH (10 kubełków równolicznych, przewidz./zaobserw.):"
    );
    tabela_niezawodnosci("surowy", &sur_test_pl, &yk_test, 10);
    tabela_niezawodnosci("izotoniczny", &izo_test, &yk_test, 10);
    println!("  tabela niezawodności PRZY WEJŚCIU:");
    tabela_niezawodnosci("surowy", &sur_test_we, &yw_test, 10);
    tabela_niezawodnosci("izotoniczny", &izo_test_we, &yw_test, 10);
    println!(
        "  → po próbkach: Brier {:.1} % lepszy, ECE {:.1} % lepszy · \
         przy wejściu: Brier {:.1} % lepszy, ECE {:.1} % lepszy",
        (b0 - b1.min(b2)) / b0.max(1e-9) * 100.0,
        (e0 - e1.min(e2)) / e0.max(1e-9) * 100.0,
        (bw0 - bw1.min(bw2)) / bw0.max(1e-9) * 100.0,
        (ew0 - ew1.min(ew2)) / ew0.max(1e-9) * 100.0
    );

    // kalibrowane predykcje używane dalej
    let kal_we = |v: &[f32]| -> Vec<f32> { v.iter().map(|p| izo_we.pred(*p)).collect() };
    let kal_pl = |v: &[Vec<f32>]| -> Vec<Vec<f32>> {
        v.iter()
            .map(|z| z.iter().map(|p| izo.pred(*p)).collect())
            .collect()
    };
    let p0_kal = kal_we(&sur_kal_we);
    let p0_test = kal_we(&sur_test_we);
    let pp_test = kal_pl(&sur_test);

    // ---------- ODNIESIENIA ----------
    let tp1_t = pol_tp1(test);
    let bez_t = pol_bez_celu(test);
    let run_t = pol_runner(test);
    let w_tp1 = podsumuj(&tp1_t);
    let w_bez = podsumuj(&bez_t);
    let w_run = podsumuj(&run_t);
    let w_sufit = podsumuj(&pol_szczyt(test));
    let w_wyr = podsumuj(&pol_wyrocznia_hybryda(test));

    // ==================================================================
    //  KONTROLA: MODEL UCZONY WYŁĄCZNIE NA PUNKCIE DECYZYJNYM
    // ==================================================================
    println!(
        "
───── KONTROLA: model uczony TYLKO na punkcie decyzyjnym ─────"
    );
    println!(
        "  {:<28} {:>10} {:>10} {:>12} {:>10}",
        "punkt decyzyjny", "AUC sieć", "AUC lin.", "próbek ucz.", "test"
    );
    for (n_, kr) in [("WEJŚCIE", -1.0f32), ("+1 R", 1.0), ("+2 R", 2.0)] {
        let (an, al, nu, nt_) = model_na_punkcie(a, ucz, test, kr);
        println!(
            "  {:<28} {:>10.4} {:>10.4} {:>12} {:>10}",
            n_, an, al, nu, nt_
        );
    }
    println!(
        "  → populacja uczenia = populacja stosowania. Jeśli AUC dalej siedzi przy 0,50,
             niskie AUC przy wejściu NIE jest skutkiem doboru próbek do uczenia."
    );

    // ==================================================================
    //  RISK FREE STEROWANY MODELEM
    // ==================================================================
    println!(
        "
───── RISK FREE + model (czy ranking modelu cokolwiek dokłada) ─────"
    );
    let p0_kal_rf = kal_we(&sur_kal_we);
    let p0_test_rf = kal_we(&sur_test_we);
    for vi in [0usize, 2] {
        let czysty = podsumuj(&pol_rf(test, vi)).pnl;
        // model decyduje, KTÓRE koszyki grać w trybie RISK FREE, a które na TP1
        let kand_rf = kwantyle(p0_kal_rf.clone(), 20);
        let mut naj = (kand_rf[0], f64::NEG_INFINITY);
        for pr in &kand_rf {
            let w: Vec<(i64, f64)> = kal
                .iter()
                .zip(&p0_kal_rf)
                .map(|(s, q)| {
                    // decyzja zapada PRZY WEJŚCIU: albo ta pozycja idzie w tryb
                    // RISK FREE (i wtedy `rf` niesie własny fallback „bez celu"),
                    // albo gra TP1. Warunku `rf_ok` tu być NIE MOŻE — to wiedza
                    // o tym, czy wyzwalacz w ogóle zajdzie, czyli o przyszłości.
                    (
                        s.dzien,
                        if *q >= *pr {
                            s.rf[vi] as f64
                        } else {
                            s.tp1() as f64
                        },
                    )
                })
                .collect();
            let o = ocena(Cel::SredniaPrzycieta, &w);
            if o > naj.1 {
                naj = (*pr, o);
            }
        }
        let sterowany: Vec<(i64, f64)> = test
            .iter()
            .zip(&p0_test_rf)
            .map(|(s, q)| {
                (
                    s.dzien,
                    if *q >= naj.0 {
                        s.rf[vi] as f64
                    } else {
                        s.tp1() as f64
                    },
                )
            })
            .collect();
        let ws = podsumuj(&sterowany);
        let rfv = pol_rf(test, vi);
        let (lo, hi) = bootstrap_roznicy(&sterowany, &rfv, 4000, a.seed);
        println!(
            "  {:<44} czysty {:>+9.2} $ · sterowany progiem {:.3} {:>+9.2} $ ·              różnica {:>+8.2} $ [{:+.0} … {:+.0}]",
            RF_NAZWY[vi],
            czysty,
            naj.0,
            ws.pnl,
            ws.pnl - czysty,
            lo,
            hi
        );
    }

    println!("\n───── §3.1 STROJENIE PROGU — cztery kryteria ─────");
    println!(
        "  rozkład KALIBROWANEGO p przy wejściu na części strojącej: \
         p10 = {:.4} · mediana = {:.4} · p90 = {:.4} · max = {:.4}",
        kwantyl(&p0_kal, 0.10),
        kwantyl(&p0_kal, 0.50),
        kwantyl(&p0_kal, 0.90),
        kwantyl(&p0_kal, 1.0)
    );
    let kand = kwantyle(p0_kal.clone(), 40);
    let mut progi = Vec::new();
    println!(
        "  {:<38} {:>8} {:>11} {:>13} {:>12}",
        "kryterium doboru progu", "próg", "pokrycie", "PnL TEST $", "puszczonych"
    );
    for cel in CELE {
        let mut naj = (kand[0], f64::NEG_INFINITY);
        for pr in &kand {
            let o = ocena(cel, &pol_wejscie(kal, &p0_kal, *pr));
            if o > naj.1 {
                naj = (*pr, o);
            }
        }
        let w = podsumuj(&pol_wejscie(test, &p0_test, naj.0));
        let ip = ile_ge(&p0_test, naj.0);
        let pokr = ile_ge(&p0_kal, naj.0) as f64 / kal.len().max(1) as f64 * 100.0;
        println!(
            "  {:<38} {:>8.4} {:>10.1} % {:>+13.2} {:>8}/{}",
            cel.nazwa(),
            naj.0,
            pokr,
            w.pnl,
            ip,
            test.len()
        );
        progi.push((cel, naj.0, w.pnl, ip, pokr));
    }

    println!("\n───── §3.2 DECYZJA PRZEZ WARTOŚĆ OCZEKIWANĄ ─────");
    let mut g_sum = 0.0f64;
    let mut g_n = 0.0f64;
    let mut l_sum = 0.0f64;
    let mut l_n = 0.0f64;
    for s in ucz {
        let d = (s.wych_koniec - s.tp1()) as f64;
        if s.probki[0].y_kl > 0.5 {
            g_sum += d;
            g_n += 1.0;
        } else {
            l_sum -= d;
            l_n += 1.0;
        }
    }
    let g = g_sum / g_n.max(1.0);
    let l = l_sum / l_n.max(1.0);
    let prog_ev = ((l + a.koszt) / (g + l)) as f32;
    println!(
        "  stałe z części UCZĄCEJ (nic poza nią): G = E[koniec − TP1 | biegacz] = {:+.2} $ \
         ({} ścieżek) · L = E[TP1 − koniec | nie-biegacz] = {:+.2} $ ({} ścieżek)",
        g, g_n as usize, l, l_n as usize
    );
    println!(
        "  koszt c = {:.2} $ (spread 0,24 $ × {} jedn.) → p* = (L + c)/(G + L) = {:.4}   \
         [ARYTMETYKA, zero strojenia]",
        a.koszt, a.jednostki, prog_ev
    );
    let ev_t = pol_wejscie(test, &p0_test, prog_ev);
    let w_ev = podsumuj(&ev_t);
    let ev_c_t = pol_ciagla_zatrzask(test, &pp_test, prog_ev);
    let w_ev_c = podsumuj(&ev_c_t);
    let ev_tp1_t = pol_w_tp1(test, &pp_test, prog_ev);
    let w_ev_tp1 = podsumuj(&ev_tp1_t);
    let ev_z_t = pol_zamknij_teraz(test, &pp_test, prog_ev);
    let w_ev_z = podsumuj(&ev_z_t);

    // ta sama arytmetyka na modelu LINIOWYM — obowiązkowe odniesienie
    let lin_we_kal: Vec<f32> = kal.iter().map(|z| lin_k.pred(&z.probki[0].x)).collect();
    let lin_we_test: Vec<f32> = test.iter().map(|z| lin_k.pred(&z.probki[0].x)).collect();
    let izo_lin = Izotonik::ucz(&lin_we_kal, &yw_kal);
    let p0l_test: Vec<f32> = lin_we_test.iter().map(|p| izo_lin.pred(*p)).collect();
    let ev_lin_t = pol_wejscie(test, &p0l_test, prog_ev);
    let w_ev_lin = podsumuj(&ev_lin_t);
    println!(
        "  puszczonych: EV wejście {}/{} · EV ciągła (zatrzask) {}/{} · liniowy {}/{}",
        ile_ge(&p0_test, prog_ev),
        test.len(),
        pol_ciagla_zatrzask(test, &pp_test, prog_ev)
            .iter()
            .zip(test)
            .filter(|(w, s)| (w.1 - s.wych_koniec as f64).abs() < 1e-6)
            .count(),
        test.len(),
        ile_ge(&p0l_test, prog_ev),
        test.len()
    );

    println!("\n───── §3.4 ETYKIETA CELOWANA W OGON ─────");
    let rel_ucz: Vec<f32> = pu
        .iter()
        .map(|p| p.szczyt_od_teraz / p.ryzyko.max(1e-6))
        .collect();
    let prog_ogon = kwantyl(&rel_ucz, 0.90);
    let yo = |p: &Probka| {
        if p.szczyt_od_teraz / p.ryzyko.max(1e-6) >= prog_ogon {
            1.0
        } else {
            0.0
        }
    };
    let yu_o: Vec<f32> = pu.iter().map(|p| yo(p)).collect();
    let yt_o: Vec<f32> = pt.iter().map(|p| yo(p)).collect();
    println!(
        "  próg ogona = 90. percentyl szczytu w krotności ryzyka na UCZENIU: {:.2} R \
         (etykieta biegacza to {:.0} R = {:.1} % próbek)",
        prog_ogon,
        a.n_r,
        yu_k.iter().sum::<f32>() as f64 / yu_k.len().max(1) as f64 * 100.0
    );
    let m_o = ucz_klas(&xu, &yu_o, &a.ukryte, a.epok, a.lr * 4.0, a.seed + 2);
    let lin_o = RegLog::ucz(&xu, &yu_o, 300, 3.0, 1e-4);
    let po_all: Vec<Vec<f32>> = {
        let mut s = Scratch::for_net(&m_o);
        test.iter()
            .map(|z| {
                z.probki
                    .iter()
                    .map(|p| pred_klas(&m_o, &mut s, &p.x))
                    .collect()
            })
            .collect()
    };
    let po0_test: Vec<f32> = po_all.iter().map(|v| v[0]).collect();
    let po_test_pl: Vec<f32> = po_all.iter().flatten().cloned().collect();
    let po_lin: Vec<f32> = pt.iter().map(|p| lin_o.pred(&p.x)).collect();
    let po_lin0: Vec<f32> = test.iter().map(|z| lin_o.pred(&z.probki[0].x)).collect();
    let auc_o = auc(&po_test_pl, &yt_o);
    let auc_o_l = auc(&po_lin, &yt_o);
    println!("  AUC etykiety ogonowej (po próbkach): sieć {auc_o:.4} · liniowy {auc_o_l:.4}");
    if auc_o <= auc_o_l {
        println!("  → sieć NIE bije liniowego poza próbą");
    }

    // regresja logarytmu szczytu WYŁĄCZNIE NA WYGRANYCH
    let wyg: Vec<&Probka> = ucz
        .iter()
        .filter(|s| s.tp1() > 0.0)
        .flat_map(|s| s.probki.iter())
        .collect();
    let xw: Vec<Vec<f32>> = wyg.iter().map(|p| p.x.clone()).collect();
    let yw: Vec<f32> = wyg
        .iter()
        .map(|p| (1.0 + p.szczyt_od_teraz.max(0.0)).ln())
        .collect();
    let ridge_w = Ridge::ucz(&xw, &yw, 1.0);
    let yw3: Vec<[f32; 3]> = yw.iter().map(|v| [*v, *v, *v]).collect();
    let (m_w, sk_w) = ucz_siec(&xw, &yw3, &a.ukryte, a.epok, a.lr, a.seed + 3);
    let reg0: Vec<f32> = {
        let mut s = Scratch::for_net(&m_w);
        test.iter()
            .map(|z| pred_siec(&m_w, &sk_w, &mut s, &z.probki[0].x, 0))
            .collect()
    };
    let reg0_lin: Vec<f32> = test.iter().map(|z| ridge_w.pred(&z.probki[0].x)).collect();
    // R² liczony na TYCH SAMYCH warunkach, na jakich model był uczony:
    // tylko próbki ścieżek wygrywających. Liczenie go na wszystkich mierzyłoby
    // przesunięcie rozkładu, a nie jakość modelu.
    let (r2_w, r2_w_l) = {
        let idx: Vec<usize> = (0..test.len()).filter(|i| test[*i].tp1() > 0.0).collect();
        let mut s = Scratch::for_net(&m_w);
        let pr: Vec<f32> = idx
            .iter()
            .map(|i| pred_siec(&m_w, &sk_w, &mut s, &test[*i].probki[0].x, 0))
            .collect();
        let prl: Vec<f32> = idx
            .iter()
            .map(|i| ridge_w.pred(&test[*i].probki[0].x))
            .collect();
        let y: Vec<f32> = idx
            .iter()
            .map(|i| (1.0 + test[*i].probki[0].szczyt_od_teraz.max(0.0)).ln())
            .collect();
        (r2(&pr, &y), r2(&prl, &y))
    };
    println!(
        "  regresja ln(1+szczyt) uczona TYLKO na wygranych ({} z {} ścieżek, {} próbek): \
         R² na wygranych z testu — sieć {:+.4} · liniowy {:+.4}",
        ucz.iter().filter(|s| s.tp1() > 0.0).count(),
        ucz.len(),
        wyg.len(),
        r2_w,
        r2_w_l
    );

    let (z_bieg, u_bieg, suma_luk) = koncentracja(test, &p0_test);
    let (z_ogon, u_ogon, _) = koncentracja(test, &po0_test);
    let (z_ogon_l, u_ogon_l, _) = koncentracja(test, &po_lin0);
    let (z_reg, u_reg, _) = koncentracja(test, &reg0);
    let (z_reg_l, u_reg_l, _) = koncentracja(test, &reg0_lin);
    println!(
        "\n  KONCENTRACJA — {} ścieżek testowych, łączna luka wyjścia {:.0} $, \
         górny decyl = {} ścieżek",
        test.len(),
        suma_luk,
        (test.len() / 10).max(1)
    );
    let lk = luki(test);
    println!(
        "  dziesięć największych luk to {:.0} $ ({:.0} % całości); największa {:.0} $",
        lk.iter().take(10).map(|v| v.1).sum::<f64>(),
        lk.iter().take(10).map(|v| v.1).sum::<f64>() / suma_luk.max(1e-9) * 100.0,
        lk[0].1
    );
    println!(
        "  {:<48} {:>14} {:>18}",
        "ranking PRZY WEJŚCIU", "luka w decylu", "z 10 największych"
    );
    for (n_, z, u) in [
        ("biegacz ≥ N·R (sieć) — stan dotychczasowy", z_bieg, u_bieg),
        ("OGON p90 (sieć)", z_ogon, u_ogon),
        ("OGON p90 (liniowy — odniesienie)", z_ogon_l, u_ogon_l),
        ("ln(szczyt) na wygranych (sieć)", z_reg, u_reg),
        ("ln(szczyt) na wygranych (liniowy)", z_reg_l, u_reg_l),
    ] {
        println!("  {:<48} {:>13.0} % {:>15}/10", n_, u, z);
    }

    // Czy ogon jest w ogóle przewidywalny PRZY WEJŚCIU? Najlepsza pojedyncza
    // cecha (i jej odwrotność) jako granica dolna tego, co da się osiągnąć
    // bez modelu. Bez tej kontroli „model łapie 1/10" nie ma z czym porównać.
    let mut naj_cecha = (String::new(), 0usize, 0.0f64);
    for f in 0..F {
        for zn in [1.0f32, -1.0] {
            let r: Vec<f32> = test.iter().map(|z| z.probki[0].x[f] * zn).collect();
            let (z, u, _) = koncentracja(test, &r);
            if z > naj_cecha.1 || (z == naj_cecha.1 && u > naj_cecha.2) {
                naj_cecha = (
                    format!("{}{}", if zn < 0.0 { "−" } else { "+" }, NAZWY_CECH[f]),
                    z,
                    u,
                );
            }
        }
    }
    println!(
        "  KONTROLA: najlepsza z {} pojedynczych cech przy wejściu ({}) łapie {}/10 \
         i trzyma {:.0} % luki — to jest granica „bez modelu\"",
        F * 2,
        naj_cecha.0,
        naj_cecha.1,
        naj_cecha.2
    );

    // Ranking w PÓŹNIEJSZYM punkcie decyzyjnym: kiedy ruch już się zaczął.
    let (r1, n1) = ranking_w_punkcie(test, &pp_test, 1.0);
    let (r2_, n2) = ranking_w_punkcie(test, &pp_test, 2.0);
    let (z1, u1, _) = koncentracja(test, &r1);
    let (z2, u2, _) = koncentracja(test, &r2_);
    let (t1, o1, _) = kontrola_punktu(test, 1.0);
    let (t2, o2, _) = kontrola_punktu(test, 2.0);
    println!(
        "  ranking W CHWILI +1 R ({} z {} ścieżek): {}/10, {:.0} % luki  \
         [KONTROLA: {} z 10 największych w ogóle doszło; losowa kolejność dałaby {:.1}/10]",
        n1,
        test.len(),
        z1,
        u1,
        t1,
        o1
    );
    println!(
        "  ranking W CHWILI +2 R ({} ścieżek): {}/10, {:.0} % luki  \
         [KONTROLA: {} z 10 największych doszło; losowa kolejność dałaby {:.1}/10]",
        n2, z2, u2, t2, o2
    );

    // dolary na głowie ogonowej — próg dobrany kryterium przyciętej średniej
    let po0_kal: Vec<f32> = {
        let mut s = Scratch::for_net(&m_o);
        kal.iter()
            .map(|z| pred_klas(&m_o, &mut s, &z.probki[0].x))
            .collect()
    };
    let yo_kal_we: Vec<f32> = kal.iter().map(|z| yo(&z.probki[0])).collect();
    let izo_o = Izotonik::ucz(&po0_kal, &yo_kal_we);
    let po0_kal_c: Vec<f32> = po0_kal.iter().map(|p| izo_o.pred(*p)).collect();
    let po0_test_c: Vec<f32> = po0_test.iter().map(|p| izo_o.pred(*p)).collect();
    let kand_o = kwantyle(po0_kal_c.clone(), 40);
    let mut naj_o = (kand_o[0], f64::NEG_INFINITY);
    for pr in &kand_o {
        let o = ocena(Cel::SredniaPrzycieta, &pol_wejscie(kal, &po0_kal_c, *pr));
        if o > naj_o.1 {
            naj_o = (*pr, o);
        }
    }
    let ogon_t = pol_wejscie(test, &po0_test_c, naj_o.0);
    let w_ogon = podsumuj(&ogon_t);

    // ==================================================================
    //  DOLARY
    // ==================================================================
    println!(
        "\n───── DOLARY NA TEŚCIE (0,01 lota na jednostkę, {} jednostek) ─────",
        a.jednostki
    );
    let ci = |v: &Vec<(i64, f64)>| Some(bootstrap_dni(v, 4000, a.seed));
    nag();
    wiersz("SUFIT: wyjście w szczycie", &w_sufit, None);
    wiersz("SUFIT hybrydy (wie, co puścić)", &w_wyr, None);
    println!("  {:-<48}", "");
    wiersz("ODNIESIENIE: wszystko na TP1", &w_tp1, ci(&tp1_t));
    wiersz("ODNIESIENIE: nic bez celu (do SL)", &w_bez, ci(&bez_t));
    wiersz(
        "ODNIESIENIE: czempion (1×TP1 + runnery)",
        &w_run,
        ci(&run_t),
    );
    println!("  {:-<48}", "");
    for (cel, pr, _, ip, _) in &progi {
        let w = podsumuj(&pol_wejscie(test, &p0_test, *pr));
        wiersz(
            &format!(
                "§3.1 próg wg [{}] = {:.4} ({}/{})",
                cel.krotka(),
                pr,
                ip,
                test.len()
            ),
            &w,
            None,
        );
    }
    println!("  {:-<48}", "");
    wiersz(
        &format!(
            "§3.2 EV wejście p* = {:.4} ({}/{})",
            prog_ev,
            ile_ge(&p0_test, prog_ev),
            test.len()
        ),
        &w_ev,
        ci(&ev_t),
    );
    wiersz(
        "§3.2 EV wejście — model LINIOWY (odniesienie)",
        &w_ev_lin,
        None,
    );
    wiersz(
        "§3.2 EV ciągła z zatrzaskiem (do chwili TP1)",
        &w_ev_c,
        ci(&ev_c_t),
    );
    wiersz(
        "§3.2 EV oceniana w chwili dotknięcia TP1",
        &w_ev_tp1,
        ci(&ev_tp1_t),
    );
    wiersz(
        "§3.2 EV „zamknij teraz\" (wariant degenerujący)",
        &w_ev_z,
        None,
    );
    wiersz(
        &format!(
            "§3.4 głowa OGONOWA, próg {:.4} ({}/{})",
            naj_o.0,
            ile_ge(&po0_test_c, naj_o.0),
            test.len()
        ),
        &w_ogon,
        ci(&ogon_t),
    );

    println!("\n  CZY MODEL DOKŁADA (różnica sparowana, bootstrap po dniach):");
    let poroj = |nazwa: &str, x: &Vec<(i64, f64)>, y: &Vec<(i64, f64)>| {
        let d: f64 = x.iter().zip(y).map(|(u, v)| u.1 - v.1).sum();
        let (lo, hi) = bootstrap_roznicy(x, y, 4000, a.seed);
        let werdykt = if lo > 0.0 {
            "DODAJE"
        } else if hi < 0.0 {
            "SZKODZI"
        } else {
            "nierozstrzygnięte"
        };
        println!(
            "    {:<52} {:>+9.2} $  [{:+.0} … {:+.0}]  {}",
            nazwa, d, lo, hi, werdykt
        );
    };
    poroj("§3.2 EV wejście − czempion", &ev_t, &run_t);
    poroj("§3.2 EV wejście − wszystko na TP1", &ev_t, &tp1_t);
    poroj("§3.2 EV wejście − EV wejście LINIOWA", &ev_t, &ev_lin_t);
    poroj("§3.2 EV ciągła − czempion", &ev_c_t, &run_t);
    poroj("§3.2 EV ciągła − wszystko na TP1", &ev_c_t, &tp1_t);
    poroj("§3.2 EV w chwili TP1 − wszystko na TP1", &ev_tp1_t, &tp1_t);
    poroj("§3.4 ogon − czempion", &ogon_t, &run_t);

    WynikFoldu {
        progi,
        ev_prog: prog_ev,
        ev_prog_lin: prog_ev,
        ev_pnl: w_ev.pnl,
        ev_pnl_lin: w_ev_lin.pnl,
        ev_ciagla_pnl: w_ev_c.pnl,
        ev_w_tp1_pnl: w_ev_tp1.pnl,
        ev_zamknij_pnl: w_ev_z.pnl,
        ev_g: g,
        ev_l: l,
        czempion: w_run.pnl,
        tp1: w_tp1.pnl,
        bez: w_bez.pnl,
        ogon_zlapane: z_ogon,
        ogon_zlapane_lin: z_ogon_l,
        ogon_reg_zlapane: z_reg,
        biegacz_zlapane: z_bieg,
        najlepsza_cecha: (naj_cecha.0, naj_cecha.1),
        zlapane_w_1r: z1,
        zlapane_w_2r: z2,
        ocz_w_1r: o1,
        ocz_w_2r: o2,
        auc_we: auc(&sur_test_we, &yw_test),
        auc_we_lin: auc_lin_we,
        auc_probki: auc(&sur_test_pl, &yk_test),
        auc_1r: auc_w_punkcie(test, &sur_test, 1.0).0,
        auc_2r: auc_w_punkcie(test, &sur_test, 2.0).0,
        ogon_pnl: w_ogon.pnl,
        auc_ogon: auc_o,
        auc_ogon_lin: auc_o_l,
        brier_sur: b0,
        brier_izo: b1,
        brier_plt: b2,
        ece_sur: e0,
        ece_izo: e1,
        ece_plt: e2,
        brier_we_sur: bw0,
        brier_we_izo: bw1,
        ece_we_sur: ew0,
        ece_we_izo: ew1,
    }
}

// ============================================================
//  WALK-FORWARD — jedyna liczba CHRONOLOGICZNA
// ============================================================

/// Cicha wersja: uczy, kalibruje, dobiera próg i zwraca dolary kluczowych polityk.
fn wf_krok(
    a: &Args,
    ucz: &[Sciezka],
    kal: &[Sciezka],
    test: &[Sciezka],
) -> (f64, f64, f64, f64, f64, f64) {
    let pu = plaskie(ucz);
    let pk = plaskie(kal);
    let xu: Vec<Vec<f32>> = pu.iter().map(|p| p.x.clone()).collect();
    let yu_k: Vec<f32> = pu.iter().map(|p| p.y_kl).collect();
    let m_k = ucz_klas(&xu, &yu_k, &a.ukryte, a.epok, a.lr * 4.0, a.seed + 1);

    let sur = |sc: &[Sciezka]| -> Vec<Vec<f32>> {
        let mut s = Scratch::for_net(&m_k);
        sc.iter()
            .map(|z| {
                z.probki
                    .iter()
                    .map(|p| pred_klas(&m_k, &mut s, &p.x))
                    .collect()
            })
            .collect()
    };
    let sk = sur(kal);
    let st = sur(test);
    let izo_we = Izotonik::ucz(
        &sk.iter().map(|v| v[0]).collect::<Vec<_>>(),
        &kal.iter().map(|z| z.probki[0].y_kl).collect::<Vec<_>>(),
    );
    let izo = Izotonik::ucz(
        &sk.iter().flatten().cloned().collect::<Vec<_>>(),
        &pk.iter().map(|p| p.y_kl).collect::<Vec<_>>(),
    );
    let p0k: Vec<f32> = sk.iter().map(|v| izo_we.pred(v[0])).collect();
    let p0t: Vec<f32> = st.iter().map(|v| izo_we.pred(v[0])).collect();
    let ppt: Vec<Vec<f32>> = st
        .iter()
        .map(|z| z.iter().map(|p| izo.pred(*p)).collect())
        .collect();

    let mut g_sum = 0.0f64;
    let mut g_n = 0.0f64;
    let mut l_sum = 0.0f64;
    let mut l_n = 0.0f64;
    for s in ucz {
        let d = (s.wych_koniec - s.tp1()) as f64;
        if s.probki[0].y_kl > 0.5 {
            g_sum += d;
            g_n += 1.0;
        } else {
            l_sum -= d;
            l_n += 1.0;
        }
    }
    let (g, l) = (g_sum / g_n.max(1.0), l_sum / l_n.max(1.0));
    let prog_ev = ((l + a.koszt) / (g + l)) as f32;

    let kand = kwantyle(p0k.clone(), 40);
    let mut naj_s = (kand[0], f64::NEG_INFINITY);
    let mut naj_p = (kand[0], f64::NEG_INFINITY);
    for pr in &kand {
        let w = pol_wejscie(kal, &p0k, *pr);
        let os = ocena(Cel::Suma, &w);
        if os > naj_s.1 {
            naj_s = (*pr, os);
        }
        let op = ocena(Cel::SredniaPrzycieta, &w);
        if op > naj_p.1 {
            naj_p = (*pr, op);
        }
    }
    (
        podsumuj(&pol_tp1(test)).pnl,
        podsumuj(&pol_runner(test)).pnl,
        podsumuj(&pol_wejscie(test, &p0t, naj_s.0)).pnl,
        podsumuj(&pol_wejscie(test, &p0t, naj_p.0)).pnl,
        podsumuj(&pol_wejscie(test, &p0t, prog_ev)).pnl,
        podsumuj(&pol_ciagla_zatrzask(test, &ppt, prog_ev)).pnl,
    )
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
        n_r: a.n_r,
        sesja: None,
        placebo_h: 0,
    };
    let t0 = std::time::Instant::now();
    let mut sc = generuj(&td, &syg, &cfg);
    if let Some(od) = &a.od {
        let d = dzien_z_daty(od)?;
        sc.retain(|s| s.dzien >= d);
    }
    let n_prob: usize = sc.iter().map(|s| s.probki.len()).sum();
    let sl = sc.iter().filter(|s| s.powod == Powod::Sl).count();
    println!(
        "ścieżki: {} koszyków · {} próbek ({:.1} s) · {} zakończonych na SL ({:.0} %)",
        sc.len(),
        n_prob,
        t0.elapsed().as_secs_f64(),
        sl,
        sl as f64 / sc.len().max(1) as f64 * 100.0
    );
    if sc.len() < 200 {
        bail!("za mało ścieżek — sprawdź zegar wiadomości i zakres ticków");
    }

    // ==================================================================
    //  RISK FREE — STRUKTURA, KTÓRA NIE POTRZEBUJE MODELU
    // ==================================================================
    println!(
        "

╔═══════════════════════════════════════════════════════════════════════════════════╗"
    );
    println!(
        "║ RISK FREE — mechanizm kanału. Zero uczenia, zero strojenia, zero podziału danych  ║"
    );
    println!(
        "╚═══════════════════════════════════════════════════════════════════════════════════╝"
    );
    println!(
        "  Domykamy część warstw koszyka z nieujemnym zyskiem, a resztę zostawiamy ze STOPEM
           NA ŚREDNIEJ CENIE WEJŚCIA tej reszty. Runner nie może stracić. Wszystko liczone
           CO TICK — na siatce minutowej stop na BE dałby darmowy zysk (pułapka nr 2)."
    );
    {
        let roz: Vec<usize> = (1..=a.jednostki)
            .map(|k| sc.iter().filter(|s| s.wypelnionych as usize == k).count())
            .collect();
        println!(
            "  wypełnionych warstw w koszyku: {} · mediana {} · koszyków z ≥2 warstwami: {} z {} ({:.0} %)",
            roz.iter()
                .enumerate()
                .map(|(i, n_)| format!("{}→{}", i + 1, n_))
                .collect::<Vec<_>>()
                .join(" "),
            {
                let mut v: Vec<u8> = sc.iter().map(|s| s.wypelnionych).collect();
                v.sort_unstable();
                v[v.len() / 2]
            },
            sc.iter().filter(|s| s.wypelnionych >= 2).count(),
            sc.len(),
            sc.iter().filter(|s| s.wypelnionych >= 2).count() as f64 / sc.len() as f64 * 100.0
        );
        // NIEZMIENNIK: wyzwolony RISK FREE nie ma prawa stracić więcej niż poślizg
        for vi in 0..RF_WARIANTY.len() {
            let zle: Vec<f32> = sc
                .iter()
                .filter(|s| s.rf_ok[vi] && s.rf[vi] < -0.5)
                .map(|s| s.rf[vi])
                .collect();
            if !zle.is_empty() {
                println!(
                    "  ⚠ NIEZMIENNIK ZŁAMANY [{}]: {} ścieżek poniżej −0,50 $, najgorsza {:.2} $",
                    RF_NAZWY[vi],
                    zle.len(),
                    zle.iter().cloned().fold(f32::INFINITY, f32::min)
                );
            }
        }
        let vi0 = 0usize;
        let naj = sc
            .iter()
            .filter(|s| s.rf_ok[vi0])
            .map(|s| s.rf[vi0])
            .fold(f32::INFINITY, f32::min);
        println!(
            "  kontrola niezmiennika „runner nie może stracić\": najgorszy wyzwolony RISK FREE              (wariant 1) = {:.2} $ (to poślizg w ramach ticka, nie strata konstrukcyjna)",
            naj
        );
    }
    raport_rf(&sc, "CAŁOŚĆ", a.seed);
    {
        let nbl2 = 4usize;
        let bl = sc.len() / nbl2;
        for k in 0..nbl2 {
            let cz = if k + 1 == nbl2 {
                &sc[k * bl..]
            } else {
                &sc[k * bl..(k + 1) * bl]
            };
            let w_tp1 = podsumuj(&pol_tp1(cz)).pnl;
            let w_run = podsumuj(&pol_runner(cz)).pnl;
            print!(
                "
  blok {}/{} (wariant Z FALLBACKIEM): TP1 {:>+8.2} · czempion {:>+8.2}",
                k + 1,
                nbl2,
                w_tp1,
                w_run
            );
            for vi in 0..RF_WARIANTY.len() {
                print!(
                    " · RF{} {:>+8.2}",
                    vi + 1,
                    podsumuj(&pol_rf_lub_tp1(cz, vi)).pnl
                );
            }
        }
        println!();
    }

    // Sekcja RISK FREE nie wymaga ŻADNEGO uczenia, więc da się ją policzyć na
    // horyzoncie 48–72 h, na którym uczenie byłoby nie do udźwignięcia.
    if a.tylko_rf {
        return Ok(());
    }

    let n = sc.len();
    let h = n / 2;
    let a1 = (n as f64 * 0.30) as usize;
    let b1 = h;
    let a2 = h + (n as f64 * 0.30) as usize;

    let f_a = fold(
        &a,
        &sc[..a1],
        &sc[a1..b1],
        &sc[b1..],
        "FOLD A — uczenie 0–30 %, kalibracja+próg 30–50 %, TEST 50–100 % (chronologiczny)",
    );
    let f_b = fold(
        &a,
        &sc[h..a2],
        &sc[a2..],
        &sc[..h],
        "FOLD B — uczenie 50–80 %, kalibracja+próg 80–100 %, TEST 0–50 % (odwrócony: sonda stabilności)",
    );

    // ==================================================================
    //  WALK-FORWARD CHRONOLOGICZNY
    // ==================================================================
    println!(
        "\n\n╔═══════════════════════════════════════════════════════════════════════════════════╗"
    );
    println!(
        "║ WALK-FORWARD CHRONOLOGICZNY — jedyna liczba, której nie da się przestroić         ║"
    );
    println!(
        "╚═══════════════════════════════════════════════════════════════════════════════════╝"
    );
    let nbl = a.wf + 2;
    let blok = sc.len() / nbl;
    let mut sumy = [0.0f64; 6];
    println!(
        "  {:<8} {:>7} {:>11} {:>11} {:>13} {:>13} {:>13} {:>13}",
        "krok", "test", "TP1", "czempion", "próg[suma]", "próg[przyc.]", "EV wejście", "EV ciągła"
    );
    for k in 0..a.wf {
        let ucz = &sc[..(k + 1) * blok];
        let kal = &sc[(k + 1) * blok..(k + 2) * blok];
        let test = if k + 3 >= nbl {
            &sc[(k + 2) * blok..]
        } else {
            &sc[(k + 2) * blok..(k + 3) * blok]
        };
        let r = wf_krok(&a, ucz, kal, test);
        let v = [r.0, r.1, r.2, r.3, r.4, r.5];
        for i in 0..6 {
            sumy[i] += v[i];
        }
        println!(
            "  {:<8} {:>7} {:>+11.2} {:>+11.2} {:>+13.2} {:>+13.2} {:>+13.2} {:>+13.2}",
            k + 1,
            test.len(),
            v[0],
            v[1],
            v[2],
            v[3],
            v[4],
            v[5]
        );
    }
    println!(
        "  {:<8} {:>7} {:>+11.2} {:>+11.2} {:>+13.2} {:>+13.2} {:>+13.2} {:>+13.2}",
        "SUMA", "", sumy[0], sumy[1], sumy[2], sumy[3], sumy[4], sumy[5]
    );

    // ==================================================================
    //  WERDYKTY
    // ==================================================================
    println!(
        "\n\n╔═══════════════════════════════════════════════════════════════════════════════════╗"
    );
    println!(
        "║ WERDYKTY — kryteria sukcesu z PLAN_AI.md                                          ║"
    );
    println!(
        "╚═══════════════════════════════════════════════════════════════════════════════════╝"
    );

    println!("\n§3.1 — próg z fold A w ±30 % progu z fold B, ZNAK wyniku ten sam w obu foldach");
    println!(
        "  {:<38} {:>8} {:>8} {:>9} {:>9} {:>9} {:>11} {:>11}  {}",
        "kryterium",
        "próg A",
        "próg B",
        "rozbież.",
        "pokr. A",
        "pokr. B",
        "PnL test A",
        "PnL test B",
        "werdykt"
    );
    let mut spelnione: Vec<(Cel, f64)> = Vec::new();
    for k in 0..f_a.progi.len() {
        let (cel, pa, va, _, ka) = f_a.progi[k];
        let (_, pb, vb, _, kb) = f_b.progi[k];
        let sr = ((pa + pb) as f64 / 2.0).max(1e-9);
        let roz = (pa - pb).abs() as f64 / sr * 100.0;
        let znak_ok = (va > 0.0) == (vb > 0.0);
        let prog_ok = roz <= 30.0;
        let w = match (prog_ok, znak_ok) {
            (true, true) => "SPEŁNIONE",
            (true, false) => "próg OK, znak NIE",
            (false, true) => "znak OK, próg NIE",
            (false, false) => "NIESPEŁNIONE",
        };
        println!(
            "  {:<38} {:>8.4} {:>8.4} {:>8.0} % {:>8.1} % {:>8.1} % {:>+11.2} {:>+11.2}  {}",
            cel.nazwa(),
            pa,
            pb,
            roz,
            ka,
            kb,
            va,
            vb,
            w
        );
        if prog_ok && znak_ok {
            spelnione.push((cel, va + vb));
        }
    }
    if spelnione.is_empty() {
        println!("  → ŻADNE kryterium nie spełnia obu warunków naraz");
    } else {
        spelnione.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        println!(
            "  → kryterium [{}] spełnia OBA warunki, suma obu foldów {:+.2} $",
            spelnione[0].0.nazwa(),
            spelnione[0].1
        );
    }

    println!("\n§3.3 — kalibracja na osobnej części danych (mniej = lepiej)");
    println!(
        "  {:<26} {:>10} {:>10} {:>11} {:>10} {:>10}",
        "populacja / fold", "Brier sur.", "Brier izo", "Brier Platt", "ECE sur.", "ECE izo"
    );
    println!(
        "  {:<26} {:>10.4} {:>10.4} {:>11.4} {:>10.4} {:>10.4}",
        "po próbkach · A", f_a.brier_sur, f_a.brier_izo, f_a.brier_plt, f_a.ece_sur, f_a.ece_izo
    );
    println!(
        "  {:<26} {:>10.4} {:>10.4} {:>11.4} {:>10.4} {:>10.4}",
        "po próbkach · B", f_b.brier_sur, f_b.brier_izo, f_b.brier_plt, f_b.ece_sur, f_b.ece_izo
    );
    println!(
        "  {:<26} {:>10.4} {:>10.4} {:>11} {:>10.4} {:>10.4}",
        "przy wejściu · A", f_a.brier_we_sur, f_a.brier_we_izo, "—", f_a.ece_we_sur, f_a.ece_we_izo
    );
    println!(
        "  {:<26} {:>10.4} {:>10.4} {:>11} {:>10.4} {:>10.4}",
        "przy wejściu · B", f_b.brier_we_sur, f_b.brier_we_izo, "—", f_b.ece_we_sur, f_b.ece_we_izo
    );
    let ok = f_a.brier_izo < f_a.brier_sur
        && f_b.brier_izo < f_b.brier_sur
        && f_a.ece_izo < f_a.ece_sur
        && f_b.ece_izo < f_b.ece_sur;
    println!(
        "  → kalibracja izotoniczna {} w OBU foldach i OBU populacjach: {}",
        if ok {
            "POPRAWIA Brier i ECE"
        } else {
            "NIE poprawia wszędzie"
        },
        if ok {
            "SPEŁNIONE"
        } else {
            "sprawdzić tabelę"
        }
    );

    println!("\n§3.2 — reguła wartości oczekiwanej (zero strojenia progu)");
    println!(
        "  {:<7} {:>8} {:>8} {:>8} {:>12} {:>12} {:>12} {:>12} {:>11}",
        "fold", "G $", "L $", "p*", "EV wejście", "EV liniowa", "EV ciągła", "EV w TP1", "czempion"
    );
    for (n_, f) in [("A", &f_a), ("B", &f_b)] {
        println!(
            "  {:<7} {:>+8.2} {:>+8.2} {:>8.4} {:>+12.2} {:>+12.2} {:>+12.2} {:>+12.2} {:>+11.2}",
            n_,
            f.ev_g,
            f.ev_l,
            f.ev_prog,
            f.ev_pnl,
            f.ev_pnl_lin,
            f.ev_ciagla_pnl,
            f.ev_w_tp1_pnl,
            f.czempion
        );
    }
    let ev_rozb = (f_a.ev_prog - f_b.ev_prog).abs() as f64
        / (((f_a.ev_prog + f_b.ev_prog) / 2.0) as f64).max(1e-9)
        * 100.0;
    let str_rozb = (f_a.progi[0].1 - f_b.progi[0].1).abs() as f64
        / (((f_a.progi[0].1 + f_b.progi[0].1) / 2.0) as f64).max(1e-9)
        * 100.0;
    println!(
        "  ROZBIEŻNOŚĆ PROGU między foldami: arytmetyczny {:.0} % · strojony na sumie {:.0} %",
        ev_rozb, str_rozb
    );
    println!(
        "  suma obu foldów: EV wejście {:+.2} · EV ciągła {:+.2} · EV w TP1 {:+.2} · \
         EV „zamknij teraz\" {:+.2} · czempion {:+.2} · wszystko na TP1 {:+.2} · nic bez celu {:+.2}",
        f_a.ev_pnl + f_b.ev_pnl,
        f_a.ev_ciagla_pnl + f_b.ev_ciagla_pnl,
        f_a.ev_w_tp1_pnl + f_b.ev_w_tp1_pnl,
        f_a.ev_zamknij_pnl + f_b.ev_zamknij_pnl,
        f_a.czempion + f_b.czempion,
        f_a.tp1 + f_b.tp1,
        f_a.bez + f_b.bez
    );

    println!("\nGDZIE NAPRAWDĘ JEST SYGNAŁ — AUC rozbite na PUNKTY DECYZYJNE");
    println!(
        "  {:<8} {:>18} {:>16} {:>12} {:>12}",
        "fold", "wszystkie próbki", "PRZY WEJŚCIU", "w +1 R", "w +2 R"
    );
    for (n_, f) in [("A", &f_a), ("B", &f_b)] {
        println!(
            "  {:<8} {:>18.4} {:>16.4} {:>12.4} {:>12.4}",
            n_, f.auc_probki, f.auc_we, f.auc_1r, f.auc_2r
        );
    }
    println!(
        "  liniowy przy wejściu: {:.4} / {:.4}",
        f_a.auc_we_lin, f_b.auc_we_lin
    );
    println!(
        "  → AUC po wszystkich próbkach mierzy ROZPOZNAWANIE RUCHU JUŻ TRWAJĄCEGO: ścieżka\n\
         \x20   biegnąca sześć godzin daje 360 próbek etykietowanych 1, ścieżka ginąca na SL\n\
         \x20   po pięciu minutach daje pięć. W punktach, w których zapada decyzja, AUC jest\n\
         \x20   przy 0,50 — czyli PRZY WEJŚCIU RANKINGU NIE MA."
    );

    println!("\n§3.4 — górny decyl ma złapać ≥ 4 z 10 największych luk wyjścia");
    println!(
        "  {:<48} {:>8} {:>8}  {}",
        "ranking", "fold A", "fold B", "werdykt"
    );
    for (n_, x, y) in [
        (
            "biegacz ≥ N·R przy wejściu (stan dotychczasowy)",
            f_a.biegacz_zlapane,
            f_b.biegacz_zlapane,
        ),
        (
            "OGON p90 przy wejściu — sieć",
            f_a.ogon_zlapane,
            f_b.ogon_zlapane,
        ),
        (
            "OGON p90 przy wejściu — liniowy",
            f_a.ogon_zlapane_lin,
            f_b.ogon_zlapane_lin,
        ),
        (
            "ln(szczyt) na wygranych — sieć",
            f_a.ogon_reg_zlapane,
            f_b.ogon_reg_zlapane,
        ),
        (
            "ten sam model W CHWILI +1 R",
            f_a.zlapane_w_1r,
            f_b.zlapane_w_1r,
        ),
        (
            "ten sam model W CHWILI +2 R",
            f_a.zlapane_w_2r,
            f_b.zlapane_w_2r,
        ),
    ] {
        let w = if x >= 4 && y >= 4 {
            "SPEŁNIONE"
        } else if x >= 4 || y >= 4 {
            "połowicznie"
        } else {
            "NIESPEŁNIONE"
        };
        println!("  {:<48} {:>6}/10 {:>6}/10  {}", n_, x, y, w);
    }
    println!(
        "  KONTROLA bez modelu — najlepsza pojedyncza cecha: fold A [{}] {}/10 · fold B [{}] {}/10",
        f_a.najlepsza_cecha.0, f_a.najlepsza_cecha.1, f_b.najlepsza_cecha.0, f_b.najlepsza_cecha.1
    );
    println!(
        "  KONTROLA punktu +1 R (losowa kolejność wewnątrz grupy, która doszła): {:.1}/10 i {:.1}/10 ·          punktu +2 R: {:.1}/10 i {:.1}/10 — model musi bić TE liczby, nie zero",
        f_a.ocz_w_1r, f_b.ocz_w_1r, f_a.ocz_w_2r, f_b.ocz_w_2r
    );
    println!(
        "  AUC etykiety ogonowej: sieć {:.4} / {:.4} · liniowy {:.4} / {:.4} (fold A / B)",
        f_a.auc_ogon, f_b.auc_ogon, f_a.auc_ogon_lin, f_b.auc_ogon_lin
    );
    println!(
        "  dolary na głowie ogonowej: {:+.2} / {:+.2} (suma {:+.2}) wobec czempiona {:+.2}",
        f_a.ogon_pnl,
        f_b.ogon_pnl,
        f_a.ogon_pnl + f_b.ogon_pnl,
        f_a.czempion + f_b.czempion
    );

    println!(
        "\nUWAGA: pomiar NA ŚCIEŻKACH, nie w silniku. Brak limitu jednoczesnych pozycji,\n\
         brak zarządzania kapitałem, brak opóźnienia wykonania, lot stały. Liczby są\n\
         porównywalne MIĘDZY SOBĄ, ale nie z wynikiem presetu w silniku. Fold B jest\n\
         odwrócony w czasie i służy WYŁĄCZNIE do sprawdzenia stabilności progu —\n\
         jedyną liczbą chronologiczną jest walk-forward."
    );
    Ok(())
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn probka(w: f32, ts: i64) -> Probka {
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

    fn sciezka(
        probki: Vec<Probka>,
        tp1: Option<f32>,
        ts_tp1: Option<i64>,
        koniec: f32,
        szczyt: f32,
    ) -> Sciezka {
        Sciezka {
            sygnal: 0,
            side_buy: true,
            id_sygnalu: 1,
            dzien: 0,
            probki,
            wych_tp1: tp1,
            ts_tp1,
            wych_koniec: koniec,
            wych_runner: 0.0,
            szczyt,
            ryzyko_wej: 6.0,
            powod: Powod::Horyzont,
            rf: vec![0.0; RF_WARIANTY.len()],
            rf_ok: vec![false; RF_WARIANTY.len()],
            wypelnionych: 1,
            ts_wyp: vec![0],
        }
    }

    #[test]
    fn izotonik_odtwarza_monotoniczna_zaleznosc() {
        let mut p = Vec::new();
        let mut y = Vec::new();
        for k in 0..2000 {
            let q = (k % 100) as f32 / 100.0;
            p.push(q);
            y.push(if ((k / 100) as f32 / 20.0) < q * q {
                1.0
            } else {
                0.0
            });
        }
        let iz = Izotonik::ucz(&p, &y);
        assert!(iz.pred(0.1) < iz.pred(0.9), "kalibrator musi być rosnący");
        assert!(
            iz.pred(0.5) < 0.5,
            "surowe 0,5 przy prawdzie 0,25 ma spaść: {}",
            iz.pred(0.5)
        );
        let mut poprz = -1.0;
        for k in 0..=100 {
            let v = iz.pred(k as f32 / 100.0);
            assert!(v >= poprz - 1e-6, "spadek w {k}: {v} < {poprz}");
            poprz = v;
        }
    }

    #[test]
    fn izotonik_poprawia_brier_na_rozdetych_prawdopodobienstwach() {
        let mut p = Vec::new();
        let mut y = Vec::new();
        for k in 0..4000 {
            let q = ((k * 37) % 100) as f32 / 100.0;
            p.push(q);
            y.push(if (((k * 53) % 1000) as f32 / 1000.0) < q * q {
                1.0
            } else {
                0.0
            });
        }
        let iz = Izotonik::ucz(&p[..2000], &y[..2000]);
        let pk: Vec<f32> = p[2000..].iter().map(|v| iz.pred(*v)).collect();
        let b0 = brier(&p[2000..], &y[2000..]);
        let b1 = brier(&pk, &y[2000..]);
        assert!(
            b1 < b0,
            "kalibracja nie poprawiła Briera: {b0:.4} → {b1:.4}"
        );
    }

    #[test]
    fn platt_prostuje_przesuniecie() {
        let mut p = Vec::new();
        let mut y = Vec::new();
        for k in 0..3000 {
            let z = ((k % 61) as f64 - 30.0) / 10.0;
            let praw = 1.0 / (1.0 + (-z).exp());
            let sur = 1.0 / (1.0 + (-(z + 1.5)).exp());
            p.push(sur as f32);
            y.push(if (((k * 89) % 997) as f64 / 997.0) < praw {
                1.0
            } else {
                0.0
            });
        }
        let pl = Platt::ucz(&p, &y, 2000, 4.0);
        let pk: Vec<f32> = p.iter().map(|v| pl.pred(*v)).collect();
        assert!(brier(&pk, &y) < brier(&p, &y), "Platt nie poprawił Briera");
    }

    #[test]
    fn cele_reaguja_na_ogon() {
        // dziewięć dni po −1 $ i jeden dzień +100 $: suma dodatnia, mediana ujemna
        let mut w: Vec<(i64, f64)> = (0..9).map(|d| (d, -1.0)).collect();
        w.push((9, 100.0));
        assert!(ocena(Cel::Suma, &w) > 0.0);
        assert!(
            ocena(Cel::MedianaDzienna, &w) < 0.0,
            "mediana musi widzieć ogon inaczej niż suma"
        );
        assert!(ocena(Cel::SredniaPrzycieta, &w) < 0.0);
    }

    #[test]
    fn ece_zeruje_sie_dla_idealnej_kalibracji() {
        let mut p = Vec::new();
        let mut y = Vec::new();
        for k in 0..10000 {
            let q = ((k / 1000) as f32 + 0.5) / 10.0;
            p.push(q);
            y.push(if (k % 1000) < (q * 1000.0) as usize {
                1.0
            } else {
                0.0
            });
        }
        assert!(
            ece(&p, &y, 10) < 0.02,
            "ECE idealnej kalibracji: {}",
            ece(&p, &y, 10)
        );
    }

    #[test]
    fn polityka_wejscia_wybiera_wlasciwa_galaz() {
        let sc = vec![sciezka(vec![probka(0.0, 0)], Some(5.0), Some(0), 2.0, 20.0)];
        assert_eq!(podsumuj(&pol_wejscie(&sc, &[0.9], 0.5)).pnl, 2.0);
        assert_eq!(podsumuj(&pol_wejscie(&sc, &[0.1], 0.5)).pnl, 5.0);
    }

    #[test]
    fn zatrzask_nie_zmienia_zdania_po_tp1() {
        // TP1 padł w chwili 60 000; pewność rośnie DOPIERO potem — decyzja
        // musi zostać przy TP1, inaczej model handluje wiedzą z przyszłości
        let pr = vec![probka(0.0, 0), probka(3.0, 60_000), probka(9.0, 120_000)];
        let sc = vec![sciezka(pr, Some(5.0), Some(60_000), 1.0, 12.0)];
        let p = vec![vec![0.1f32, 0.1, 0.99]];
        assert_eq!(podsumuj(&pol_ciagla_zatrzask(&sc, &p, 0.5)).pnl, 5.0);
        // pewność wysoka PRZED TP1 → puszczamy
        let p2 = vec![vec![0.9f32, 0.1, 0.1]];
        assert_eq!(podsumuj(&pol_ciagla_zatrzask(&sc, &p2, 0.5)).pnl, 1.0);
    }

    #[test]
    fn decyzja_w_chwili_tp1_bierze_ostatnie_zdanie() {
        let pr = vec![probka(0.0, 0), probka(3.0, 60_000), probka(9.0, 120_000)];
        let sc = vec![sciezka(pr, Some(5.0), Some(60_000), 1.0, 12.0)];
        // ostatnia próbka PRZED TP1 (ts = 60 000) ma p = 0,9 → puszczamy
        let p = vec![vec![0.1f32, 0.9, 0.1]];
        assert_eq!(podsumuj(&pol_w_tp1(&sc, &p, 0.5)).pnl, 1.0);
        // a tu ostatnie zdanie przed TP1 to 0,1 → TP1, mimo że później rośnie
        let p2 = vec![vec![0.9f32, 0.1, 0.99]];
        assert_eq!(podsumuj(&pol_w_tp1(&sc, &p2, 0.5)).pnl, 5.0);
    }

    #[test]
    fn zamknij_teraz_degeneruje_na_niskim_p() {
        let pr = vec![probka(-0.7, 0), probka(7.0, 60_000)];
        let sc = vec![sciezka(pr, Some(5.0), Some(60_000), 3.0, 7.0)];
        // p poniżej progu już na wejściu → wychodzimy po −0,7 $ (sam spread)
        let p = vec![vec![0.05f32, 0.9]];
        assert!((podsumuj(&pol_zamknij_teraz(&sc, &p, 0.2)).pnl + 0.7).abs() < 1e-6);
    }

    #[test]
    fn koncentracja_liczy_gorny_decyl() {
        let sc: Vec<Sciezka> = (0..20)
            .map(|i| {
                let mut s = sciezka(vec![probka(0.0, 0)], Some(0.0), Some(0), 0.0, i as f32);
                s.dzien = i as i64;
                s
            })
            .collect();
        let idealny: Vec<f32> = (0..20).map(|i| i as f32).collect();
        assert_eq!(
            koncentracja(&sc, &idealny).0,
            2,
            "górny decyl z 20 ścieżek to 2 pozycje"
        );
        let odwrotny: Vec<f32> = (0..20).map(|i| -(i as f32)).collect();
        assert_eq!(koncentracja(&sc, &odwrotny).0, 0);
    }

    #[test]
    fn ranking_w_punkcie_omija_sciezki_bez_ruchu() {
        // pierwsza ścieżka dochodzi do +1 R (6 $), druga nie
        let a = sciezka(
            vec![probka(0.0, 0), probka(7.0, 60_000)],
            None,
            None,
            7.0,
            7.0,
        );
        let b = sciezka(
            vec![probka(0.0, 0), probka(1.0, 60_000)],
            None,
            None,
            1.0,
            1.0,
        );
        let sc = vec![a, b];
        let p = vec![vec![0.2f32, 0.8], vec![0.9f32, 0.9]];
        let (r, ile) = ranking_w_punkcie(&sc, &p, 1.0);
        assert_eq!(ile, 1);
        assert!(
            (r[0] - 0.8).abs() < 1e-6,
            "ma wziąć p z chwili osiągnięcia +1 R"
        );
        assert!(r[1] < 0.0, "ścieżka bez ruchu ląduje na dnie rankingu");
    }
}
