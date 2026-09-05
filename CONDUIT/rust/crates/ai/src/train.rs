//! Trening ewolucyjny — bez gradientów, bez autogradu, równolegle po rdzeniach.
//!
//! **Dlaczego ewolucja, a nie uczenie ze wzmocnieniem z gradientem.** Nagroda
//! jest tu z natury epizodyczna i mocno nieciągła: zamknięcie pozycji, kara za
//! obsunięcie, kara za otwarte ryzyko. Gradient przez taki symulator nie
//! przechodzi — trzeba by go estymować i tak. Skoro i tak estymujemy, to metody
//! ewolucyjne robią to wprost, są odporne na nieciągłości i skalują się
//! liniowo z liczbą rdzeni: 24 rdzenie = 24 równoległe backtesty, bez
//! synchronizacji, bez współdzielonego stanu.
//!
//! **Determinizm.** Cały szum losowany jest sekwencyjnie z `ChaCha8Rng`
//! zasianego ziarnem pokolenia, ZANIM ruszy równoległa ocena. Kolejność
//! wykonania wątków nie wpływa więc na nic — ten sam `--seed` daje ten sam
//! model, co do bitu.
//!
//! Dwa algorytmy do wyboru:
//!  * **ES** (OpenAI-ES): szum lustrzany, ranking zamiast surowej nagrody,
//!    Adam na estymacie gradientu. Domyślny — dobrze znosi kilka tysięcy wag.
//!  * **CEM**: przekrój elit, diagonalna gaussowska. Prostszy, szybciej zbiega
//!    przy małej liczbie parametrów, gorzej przy dużej.

use crate::policy::Model;
use crate::reward::{
    aggregate, aggregate_mar, aggregate_vs, summarize, RewardWeights, Summary, WindowOutcome,
};
use crate::rollout::{run_window, RunCfg, Window};
use conduit_backtest::{ReplayMessage, TickData};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algo {
    Es,
    Cem,
}

impl Algo {
    pub fn parse(s: &str) -> Option<Algo> {
        match s.to_ascii_lowercase().as_str() {
            "es" | "openai-es" => Some(Algo::Es),
            "cem" => Some(Algo::Cem),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Algo::Es => "es",
            Algo::Cem => "cem",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TrainCfg {
    pub algo: Algo,
    pub generations: usize,
    /// liczebność populacji; przy ES zaokrąglana w dół do parzystej (pary lustrzane)
    pub pop: usize,
    pub seed: u64,
    pub sigma: f32,
    pub lr: f32,
    /// mnożnik sigmy po każdym pokoleniu
    pub sigma_decay: f32,
    /// rozpad wag — trzyma politykę blisko „nic nie rób", dopóki nie ma powodu
    pub l2: f32,
    /// ułamek populacji uznawany za elitę (CEM)
    pub elite_frac: f64,
    /// Ile pokoleń bez poprawy środka tolerujemy, zanim rozdmuchamy sigmę.
    ///
    /// Ten mechanizm nie jest ozdobnikiem. W tym zadaniu istnieje bardzo silny
    /// atraktor „nic nie rób / skasuj wszystkie limity": ma ocenę bliską zeru,
    /// a jego otoczenie jest PŁASKIE — wszyscy sąsiedzi też nic nie robią, więc
    /// estymata gradientu wynosi zero i ewolucja staje na dobre. Okresowe
    /// podniesienie sigmy jest jedynym sposobem, żeby populacja sięgnęła poza
    /// ten płaskowyż i w ogóle zobaczyła, że aktywne zarządzanie istnieje.
    pub patience: usize,
    pub sigma_boost: f32,
    pub reward: RewardWeights,
    /// Gdzie zapisywać stan po KAŻDYM pokoleniu.
    pub checkpoint: Option<std::path::PathBuf>,
    /// Wznów z punktu kontrolnego zamiast zaczynać od zera.
    pub resume: bool,
    /// Oceniaj WZGLĘDEM linii bazowej (preset na tych samych oknach).
    pub relative: bool,

    // ---------- hooki dla interfejsu okienkowego ----------
    /// Token przerwania. Ustawienie na `true` kończy trening po zakończeniu
    /// AKTUALNEGO pokolenia — czyli już PO zapisie punktu kontrolnego, więc
    /// przerwanie nie kosztuje nic ponad bieżące pokolenie. `train` zwraca
    /// wtedy najlepszy dotychczasowy model, a nie model pusty.
    ///
    /// Świadomie nie przerywamy w środku oceny populacji: pokolenie jest
    /// najmniejszą jednostką, po której stan optymalizatora jest SPÓJNY
    /// (theta, momenty Adama, sigma, licznik stagnacji). Zatrzymanie w połowie
    /// dałoby punkt kontrolny, z którego wznowienie nie byłoby deterministyczne.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Licznik zakończonych par (kandydat, okno) w bieżącym pokoleniu.
    /// Pozwala pokazać postęp WEWNĄTRZ pokolenia, a nie tylko między nimi —
    /// przy 30-sekundowym pokoleniu to różnica między paskiem, który stoi,
    /// a paskiem, który płynie.
    pub eval_done: Option<Arc<AtomicU64>>,
}

impl TrainCfg {
    /// Czy poproszono o przerwanie.
    #[inline]
    pub fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(false)
    }
}

impl Default for TrainCfg {
    fn default() -> Self {
        TrainCfg {
            algo: Algo::Es,
            generations: 30,
            pop: 32,
            seed: 1,
            sigma: 0.05,
            lr: 0.05,
            sigma_decay: 0.995,
            l2: 0.002,
            elite_frac: 0.25,
            patience: 8,
            sigma_boost: 1.7,
            reward: RewardWeights::default(),
            checkpoint: None,
            resume: false,
            relative: true,
            cancel: None,
            eval_done: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GenReport {
    pub gen: usize,
    /// ocena środka rozkładu — to jest model, który faktycznie zapisujemy
    pub center: f64,
    pub best: f64,
    pub mean: f64,
    pub median: f64,
    pub worst: f64,
    pub center_summary: Summary,
    pub elapsed_s: f64,
    pub sigma: f32,
}

/// Ocena jednego zestawu wag na wszystkich oknach.
pub fn evaluate(
    td: &TickData,
    msgs: &[ReplayMessage],
    ws: &[Window],
    rc: &RunCfg,
    model: &Model,
    w: &RewardWeights,
) -> (f64, Vec<WindowOutcome>) {
    let outs: Vec<WindowOutcome> = ws
        .iter()
        .map(|x| run_window(td, msgs, x, rc, model))
        .collect();
    (aggregate(&outs, w), outs)
}

// ============================================================
//  PUNKT KONTROLNY
// ============================================================

/// Jeden wiersz historii treningu — to, co widać na ekranie, w formie trwałej.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GenLine {
    pub gen: usize,
    pub center: f64,
    pub best: f64,
    pub mean: f64,
    pub median: f64,
    pub worst: f64,
    pub pnl: f64,
    pub max_dd: f64,
    pub max_open_risk: f64,
    pub trades: u32,
    pub sigma: f32,
    pub elapsed_s: f64,
}

/// Pełny stan treningu po zakończonym pokoleniu.
///
/// Zapisywany po KAŻDYM pokoleniu, atomowo (zapis do pliku tymczasowego +
/// zmiana nazwy), więc przerwany zapis nie zostawia uszkodzonego punktu
/// kontrolnego. Przerwanie treningu — celowe czy przypadkowe — kosztuje
/// najwyżej jedno pokolenie.
///
/// Determinizm wznowienia bierze się stąd, że szum każdego pokolenia losowany
/// jest z ziarna `(seed, gen)`, a nie ze strumienia ciągnącego się przez cały
/// przebieg. Wznowione pokolenie `k` dostaje dokładnie ten sam szum, co
/// dostałoby w przebiegu nieprzerwanym.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub version: u32,
    /// odcisk konfiguracji — wznowienie z niepasującym stanem jest odrzucane
    pub fingerprint: String,
    /// ile pokoleń JUŻ wykonano (następne ma numer `gen`)
    pub gen: usize,
    pub seed: u64,
    pub algo: String,
    pub theta: Vec<f32>,
    pub sigma: f32,
    pub adam_m: Vec<f32>,
    pub adam_v: Vec<f32>,
    pub adam_t: i32,
    pub cem_sigma: Vec<f32>,
    pub best_theta: Vec<f32>,
    pub best_center: f64,
    pub best_score: crate::policy::TrainScore,
    pub stale: usize,
    pub history: Vec<GenLine>,
}

pub const CHECKPOINT_VERSION: u32 = 1;

/// Sentynel zamiast `−∞`, którego JSON nie potrafi zapisać.
const NEG_HUGE: f64 = -1e18;

impl Checkpoint {
    pub fn load(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let txt = std::fs::read_to_string(path.as_ref())?;
        Ok(serde_json::from_str(&txt)?)
    }

    /// Zapis atomowy: najpierw plik tymczasowy, potem podmiana nazwy.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
        let p = path.as_ref();
        if let Some(d) = p.parent() {
            if !d.as_os_str().is_empty() {
                std::fs::create_dir_all(d)?;
            }
        }
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        // Windows nie nadpisuje przy rename, więc kasujemy cel jawnie
        let _ = std::fs::remove_file(p);
        std::fs::rename(&tmp, p)?;
        Ok(())
    }
}

/// Odcisk konfiguracji: wszystko, co zmienia znaczenie zapisanych wag.
pub fn fingerprint(ws: &[Window], rc: &RunCfg, proto: &Model, tc: &TrainCfg) -> String {
    let w = &tc.reward;
    format!(
        "v{CHECKPOINT_VERSION}|pos{:?}|bsk{:?}|okna{}:{}|kap{}|int{}|tp{}|nagroda{},{},{},{},{},{},{}|pop{}|sigma{}|lr{}|algo{}",
        proto.policy.pos.dims,
        proto.policy.bsk.dims,
        ws.len(),
        ws.iter().map(|x| format!("{}-{}", x.from, x.to)).collect::<Vec<_>>().join(","),
        rc.start_balance,
        rc.settings.ai_decision_interval_s,
        rc.settings.skip_if_sl_breached,
        w.k_dd,
        w.k_risk,
        w.no_sl_atr,
        w.k_hold,
        w.k_blow,
        w.w_mean,
        w.w_min,
        tc.pop,
        tc.sigma,
        tc.lr,
        tc.algo.name(),
    )
}

/// Zmienna normalna N(0,1) metodą Boxa–Mullera.
/// Własna implementacja, żeby nie ciągnąć `rand_distr` dla jednej formuły.
#[inline]
fn gauss<R: Rng>(rng: &mut R) -> f32 {
    let u1: f32 = rng.gen_range(1e-7f32..1.0);
    let u2: f32 = rng.gen_range(0.0f32..1.0);
    (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
}

/// Kopia wektora z NaN/±inf zamienionymi na 0 — JSON nie zna takich literałów.
fn sane(v: &[f32]) -> Vec<f32> {
    v.iter()
        .map(|x| if x.is_finite() { *x } else { 0.0 })
        .collect()
}

/// Adam — stabilizuje krok przy bardzo zaszumionej estymacie gradientu.
struct Adam {
    m: Vec<f32>,
    v: Vec<f32>,
    t: i32,
}

impl Adam {
    fn new(d: usize) -> Self {
        Adam {
            m: vec![0.0; d],
            v: vec![0.0; d],
            t: 0,
        }
    }
    fn step(&mut self, theta: &mut [f32], grad: &[f32], lr: f32, l2: f32) {
        const B1: f32 = 0.9;
        const B2: f32 = 0.999;
        const EPS: f32 = 1e-8;
        self.t += 1;
        let c1 = 1.0 - B1.powi(self.t);
        let c2 = 1.0 - B2.powi(self.t);
        for i in 0..theta.len() {
            let g = grad[i];
            self.m[i] = B1 * self.m[i] + (1.0 - B1) * g;
            self.v[i] = B2 * self.v[i] + (1.0 - B2) * g * g;
            let mh = self.m[i] / c1;
            let vh = self.v[i] / c2;
            theta[i] += lr * mh / (vh.sqrt() + EPS) - l2 * theta[i];
        }
    }
}

/// Ranking wyśrodkowany: zamienia surowe wyniki na wagi z (−0.5, 0.5).
///
/// Bez tego jedno okno z gigantyczną stratą (albo z wyzerowaniem konta i karą
/// −10) zdominowałoby estymatę gradientu i pociągnęło całą populację w losową
/// stronę. Ranking sprawia, że liczy się KOLEJNOŚĆ, a nie skala.
fn centered_ranks(f: &[f64]) -> Vec<f32> {
    let n = f.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_unstable_by(|a, b| {
        f[*a]
            .partial_cmp(&f[*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut r = vec![0.0f32; n];
    for (rank, &i) in idx.iter().enumerate() {
        r[i] = (rank as f32 / (n - 1).max(1) as f32) - 0.5;
    }
    r
}

/// Równoległa ocena całej populacji.
///
/// Zadania to pary (kandydat, okno) — przy 32 kandydatach i 6 oknach daje to
/// 192 niezależne backtesty, co na 24 rdzeniach wyrównuje obciążenie dużo lepiej
/// niż zrównoleglanie po samych kandydatach.
fn eval_population(
    td: &TickData,
    msgs: &[ReplayMessage],
    ws: &[Window],
    rc: &RunCfg,
    proto: &Model,
    thetas: &[Vec<f32>],
    w: &RewardWeights,
    base: Option<&[WindowOutcome]>,
    done: Option<&AtomicU64>,
) -> (Vec<f64>, Vec<Vec<WindowOutcome>>) {
    let models: Vec<Model> = thetas
        .iter()
        .map(|t| {
            let mut m = proto.clone();
            m.policy.set_params(t);
            m
        })
        .collect();

    let nw = ws.len();
    let jobs: Vec<(usize, usize)> = (0..models.len())
        .flat_map(|c| (0..nw).map(move |k| (c, k)))
        .collect();

    let mut flat: Vec<(usize, usize, WindowOutcome)> = jobs
        .par_iter()
        .map(|&(c, k)| {
            let out = run_window(td, msgs, &ws[k], rc, &models[c]);
            // licznik postępu: jeden `fetch_add` na backtest okna, więc koszt
            // jest niemierzalny wobec samego backtestu
            if let Some(d) = done {
                d.fetch_add(1, Ordering::Relaxed);
            }
            (c, k, out)
        })
        .collect();
    flat.sort_unstable_by_key(|(c, k, _)| (*c, *k));

    let mut outs: Vec<Vec<WindowOutcome>> = vec![Vec::with_capacity(nw); models.len()];
    for (c, _, o) in flat {
        outs[c].push(o);
    }
    let fit: Vec<f64> = outs
        .iter()
        .map(|o| match base {
            // MAR × margines do zera × skala zysku; baza służy już tylko bramce aktywności
            Some(b) => aggregate_mar(o, Some(b), w),
            None => aggregate_mar(o, None, w),
        })
        .collect();
    (fit, outs)
}

/// Główna pętla treningu.
pub fn train(
    td: &TickData,
    msgs: &[ReplayMessage],
    ws: &[Window],
    rc: &RunCfg,
    proto: &Model,
    tc: &TrainCfg,
    on_gen: impl FnMut(&GenReport),
) -> (Model, Vec<GenReport>) {
    train_with(td, msgs, ws, rc, proto, tc, on_gen, |_, _| {}, |_| {})
}

/// Wariant z powiadomieniami o wznowieniu — używany przez binarkę.
#[allow(clippy::too_many_arguments)]
pub fn train_with(
    td: &TickData,
    msgs: &[ReplayMessage],
    ws: &[Window],
    rc: &RunCfg,
    proto: &Model,
    tc: &TrainCfg,
    mut on_gen: impl FnMut(&GenReport),
    mut on_resume: impl FnMut(usize, f64),
    mut on_resume_failed: impl FnMut(&str),
) -> (Model, Vec<GenReport>) {
    let d = proto.policy.n_params();
    let mut theta: Vec<f32> = proto.policy.get_params();
    let mut sigma = tc.sigma;
    let half = (tc.pop / 2).max(1);
    let pop = match tc.algo {
        Algo::Es => half * 2,
        Algo::Cem => tc.pop.max(4),
    };

    let mut adam = Adam::new(d);
    // CEM utrzymuje własną, per-wymiarową sigmę
    let mut cem_sigma: Vec<f32> = vec![tc.sigma; d];

    let mut best_theta = theta.clone();
    let mut best_center = NEG_HUGE;
    let mut best_score = crate::policy::TrainScore::default();
    let mut reports = Vec::with_capacity(tc.generations);

    let mut noise: Vec<Vec<f32>> = Vec::new();
    let mut cand: Vec<Vec<f32>> = Vec::new();
    let sigma0 = tc.sigma;
    let mut stale = 0usize;

    // Linia bazowa liczona RAZ: preset bez żadnej ingerencji modelu. To jest
    // punkt odniesienia dla każdej oceny, więc musi być policzony dokładnie na
    // tych samych oknach i tym samym silniku.
    let base_outs: Option<Vec<WindowOutcome>> = if tc.relative {
        let mut zero = proto.clone();
        zero.actions = crate::policy::ActionMode::EntryOnly;
        // model startowy nie wykonuje żadnej akcji, więc jego przebieg JEST presetem
        Some(
            ws.iter()
                .map(|x| crate::rollout::run_window(td, msgs, x, rc, &zero))
                .collect(),
        )
    } else {
        None
    };

    // --- wznowienie z punktu kontrolnego ---
    let fp = fingerprint(ws, rc, proto, tc);
    let mut history: Vec<GenLine> = Vec::new();
    let mut start_gen = 0usize;
    if tc.resume {
        if let Some(path) = &tc.checkpoint {
            match Checkpoint::load(path) {
                Ok(cp) if cp.fingerprint == fp && cp.theta.len() == d => {
                    theta = cp.theta;
                    sigma = cp.sigma;
                    adam.m = cp.adam_m;
                    adam.v = cp.adam_v;
                    adam.t = cp.adam_t;
                    if cp.cem_sigma.len() == d {
                        cem_sigma = cp.cem_sigma;
                    }
                    best_theta = cp.best_theta;
                    best_center = cp.best_center;
                    best_score = cp.best_score;
                    stale = cp.stale;
                    history = cp.history;
                    start_gen = cp.gen;
                    on_resume(start_gen, best_center);
                }
                Ok(_) => on_resume_failed("punkt kontrolny nie pasuje do bieżącej konfiguracji"),
                Err(e) => on_resume_failed(&format!("nie mogę wczytać punktu kontrolnego: {e}")),
            }
        }
    }

    for gen in start_gen..tc.generations {
        let t0 = std::time::Instant::now();
        // ziarno zależne od pokolenia — powtarzalne, ale nieskorelowane
        let mut rng =
            ChaCha8Rng::seed_from_u64(tc.seed.wrapping_mul(0x9E37_79B9).wrapping_add(gen as u64));

        cand.clear();
        match tc.algo {
            Algo::Es => {
                noise.clear();
                for _ in 0..half {
                    let mut e = vec![0.0f32; d];
                    for x in e.iter_mut() {
                        *x = gauss(&mut rng);
                    }
                    noise.push(e);
                }
                for e in &noise {
                    cand.push(theta.iter().zip(e).map(|(t, n)| t + sigma * n).collect());
                    cand.push(theta.iter().zip(e).map(|(t, n)| t - sigma * n).collect());
                }
            }
            Algo::Cem => {
                for _ in 0..pop {
                    let mut c = vec![0.0f32; d];
                    for i in 0..d {
                        c[i] = theta[i] + cem_sigma[i] * gauss(&mut rng);
                    }
                    cand.push(c);
                }
            }
        }
        // środek rozkładu oceniamy razem z populacją — to on jest zapisywany
        cand.push(theta.clone());

        let center_theta = theta.clone();
        // licznik zeruje się na starcie pokolenia — interfejs pokazuje
        // „osobnik k z N" w obrębie bieżącego pokolenia, nie od początku świata
        if let Some(c) = &tc.eval_done {
            c.store(0, Ordering::Relaxed);
        }
        let (fit, outs) = eval_population(
            td,
            msgs,
            ws,
            rc,
            proto,
            &cand,
            &tc.reward,
            base_outs.as_deref(),
            tc.eval_done.as_deref(),
        );
        let center_fit = *fit.last().unwrap();
        let center_sum = summarize(outs.last().unwrap(), &tc.reward);
        let pf = &fit[..fit.len() - 1];

        // --- aktualizacja ---
        match tc.algo {
            Algo::Es => {
                let ranks = centered_ranks(pf);
                let mut grad = vec![0.0f32; d];
                for (j, e) in noise.iter().enumerate() {
                    let wgt = ranks[2 * j] - ranks[2 * j + 1];
                    if wgt != 0.0 {
                        for i in 0..d {
                            grad[i] += wgt * e[i];
                        }
                    }
                }
                let scale = 1.0 / (pf.len() as f32 * sigma);
                for g in grad.iter_mut() {
                    *g *= scale;
                }
                adam.step(&mut theta, &grad, tc.lr, tc.l2);
            }
            Algo::Cem => {
                let k = ((pf.len() as f64 * tc.elite_frac).round() as usize).clamp(2, pf.len());
                let mut idx: Vec<usize> = (0..pf.len()).collect();
                idx.sort_unstable_by(|a, b| {
                    pf[*b]
                        .partial_cmp(&pf[*a])
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                let elite = &idx[..k];
                for i in 0..d {
                    let mut m = 0.0f32;
                    for &e in elite {
                        m += cand[e][i];
                    }
                    m /= k as f32;
                    let mut v = 0.0f32;
                    for &e in elite {
                        let dx = cand[e][i] - m;
                        v += dx * dx;
                    }
                    let s = (v / k as f32).sqrt();
                    theta[i] = m * (1.0 - tc.l2);
                    cem_sigma[i] = s.max(tc.sigma * 0.05) * tc.sigma_decay;
                }
            }
        }
        // krok mógł rozbiec parametry — wtedy cofamy się do najlepszego stanu
        if theta.iter().any(|x| !x.is_finite()) {
            theta.copy_from_slice(&best_theta);
            adam = Adam::new(d);
            sigma = sigma0;
        }
        sigma = (sigma * tc.sigma_decay).max(tc.sigma * 0.1);

        // poprawa liczona PRZED aktualizacją — `center_fit` dotyczy θ sprzed kroku
        if center_fit > best_center + 1e-9 {
            best_center = center_fit;
            best_theta = center_theta;
            best_score = crate::policy::TrainScore {
                fitness: center_fit,
                pnl: center_sum.pnl,
                max_dd: center_sum.max_dd_abs,
                profit_factor: center_sum.profit_factor,
                trades: center_sum.trades,
                win_rate: center_sum.win_rate,
                note: String::new(),
            };
            stale = 0;
        } else {
            stale += 1;
        }

        // wyjście z płaskowyżu: rozdmuchanie sigmy i skasowanie momentów Adama
        // (stary moment ciągnąłby z powrotem w to samo miejsce)
        if tc.patience > 0 && stale >= tc.patience {
            sigma = (sigma * tc.sigma_boost).min(sigma0 * 4.0);
            adam = Adam::new(d);
            stale = 0;
        }

        let mut sorted: Vec<f64> = pf.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let rep = GenReport {
            gen,
            center: center_fit,
            best: *sorted.last().unwrap(),
            mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
            median: sorted[sorted.len() / 2],
            worst: sorted[0],
            center_summary: center_sum,
            elapsed_s: t0.elapsed().as_secs_f64(),
            sigma,
        };
        on_gen(&rep);
        history.push(GenLine {
            gen,
            center: center_fit,
            best: rep.best,
            mean: rep.mean,
            median: rep.median,
            worst: rep.worst,
            pnl: rep.center_summary.pnl,
            max_dd: rep.center_summary.max_dd_abs,
            max_open_risk: rep.center_summary.max_open_risk,
            trades: rep.center_summary.trades,
            sigma: rep.sigma,
            elapsed_s: rep.elapsed_s,
        });
        reports.push(rep);

        // --- punkt kontrolny PO KAŻDYM pokoleniu ---
        if let Some(path) = &tc.checkpoint {
            let cp = Checkpoint {
                version: CHECKPOINT_VERSION,
                fingerprint: fp.clone(),
                gen: gen + 1,
                seed: tc.seed,
                algo: tc.algo.name().to_string(),
                theta: sane(&theta),
                sigma,
                adam_m: sane(&adam.m),
                adam_v: sane(&adam.v),
                adam_t: adam.t,
                cem_sigma: sane(&cem_sigma),
                best_theta: sane(&best_theta),
                best_center: if best_center.is_finite() {
                    best_center
                } else {
                    NEG_HUGE
                },
                best_score: best_score.clone(),
                stale,
                history: history.clone(),
            };
            if let Err(e) = cp.save(path) {
                eprintln!("UWAGA: nie udało się zapisać punktu kontrolnego: {e}");
            }
        }

        // Przerwanie sprawdzamy DOPIERO TERAZ — po zapisie punktu kontrolnego.
        // Dzięki tej kolejności „PRZERWIJ" nigdy nie kosztuje więcej niż jedno
        // pokolenie, a wznowienie ruszy z pokolenia `gen + 1`.
        if tc.cancelled() {
            break;
        }
    }

    // zwracamy NAJLEPSZY oceniony środek, nie ostatni — ES potrafi przestrzelić
    let mut out = proto.clone();
    out.policy.set_params(&best_theta);
    out.score = best_score;
    (out, reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_jest_wysrodkowany_i_monotoniczny() {
        let f = vec![-100.0, 0.0, 5.0, 1.0];
        let r = centered_ranks(&f);
        assert!((r[0] - -0.5).abs() < 1e-6);
        assert!((r[2] - 0.5).abs() < 1e-6);
        assert!(r[1] < r[3]);
        // skala wyniku nie ma znaczenia, tylko kolejność
        let f2 = vec![-1e9, 0.0, 5.0, 1.0];
        assert_eq!(centered_ranks(&f2), r);
    }

    #[test]
    fn punkt_kontrolny_zapisuje_sie_atomowo_i_wraca_bez_strat() {
        let d = 32;
        let cp = Checkpoint {
            version: CHECKPOINT_VERSION,
            fingerprint: "test".into(),
            gen: 7,
            seed: 42,
            algo: "es".into(),
            theta: (0..d).map(|i| i as f32 * 0.25).collect(),
            sigma: 0.061,
            adam_m: vec![0.5; d],
            adam_v: vec![0.25; d],
            adam_t: 7,
            cem_sigma: vec![0.1; d],
            best_theta: (0..d).map(|i| -(i as f32)).collect(),
            best_center: -0.0031,
            best_score: crate::policy::TrainScore::default(),
            stale: 3,
            history: vec![GenLine {
                gen: 0,
                center: -1.0,
                ..Default::default()
            }],
        };
        let p = std::env::temp_dir().join("conduit_ai_cp_test.json");
        cp.save(&p).unwrap();
        let back = Checkpoint::load(&p).unwrap();
        assert_eq!(back.gen, 7);
        assert_eq!(back.theta, cp.theta);
        assert_eq!(back.best_theta, cp.best_theta);
        assert_eq!(back.adam_m, cp.adam_m);
        assert_eq!(back.adam_t, 7);
        assert_eq!(back.stale, 3);
        assert_eq!(back.history.len(), 1);
        assert!((back.sigma - 0.061).abs() < 1e-9);
        // zapis dwukrotny musi się udać (na Windows rename nie nadpisuje)
        cp.save(&p).unwrap();
        // po zapisie nie zostaje plik tymczasowy
        assert!(!p.with_extension("tmp").exists());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn punkt_kontrolny_nie_przyjmuje_niepoprawnych_liczb() {
        // −∞ i NaN nie mają reprezentacji w JSON-ie; `sane()` musi je wyczyścić
        let v = vec![1.0f32, f32::NAN, f32::INFINITY, -2.5];
        let s = sane(&v);
        assert_eq!(s, vec![1.0, 0.0, 0.0, -2.5]);
        assert!(serde_json::to_string(&s).is_ok());
    }

    #[test]
    fn odcisk_konfiguracji_wykrywa_zmiane_okien() {
        use crate::safety::SafetyCfg;
        let m = Model::fresh(&[8], &[8], 1, SafetyCfg::default());
        let rc = RunCfg::default();
        let tc = TrainCfg::default();
        let a = fingerprint(
            &[Window {
                from: 0,
                to: 10,
                label: "a".into(),
            }],
            &rc,
            &m,
            &tc,
        );
        let b = fingerprint(
            &[Window {
                from: 0,
                to: 20,
                label: "a".into(),
            }],
            &rc,
            &m,
            &tc,
        );
        assert_ne!(a, b, "inne okna muszą dać inny odcisk");

        let mut tc2 = tc.clone();
        tc2.reward.k_risk += 0.1;
        let c = fingerprint(
            &[Window {
                from: 0,
                to: 10,
                label: "a".into(),
            }],
            &rc,
            &m,
            &tc2,
        );
        assert_ne!(a, c, "inna nagroda musi dać inny odcisk");
    }

    #[test]
    fn adam_idzie_w_strone_gradientu() {
        let mut a = Adam::new(3);
        let mut th = vec![0.0f32; 3];
        for _ in 0..10 {
            a.step(&mut th, &[1.0, -1.0, 0.0], 0.1, 0.0);
        }
        assert!(th[0] > 0.5, "{th:?}");
        assert!(th[1] < -0.5);
        assert!(th[2].abs() < 1e-3);
    }
}
