//! Zadanie: TRENING MODELU AI.
//!
//! Ta sama funkcja co CLI `train.exe` — `conduit_ai::train::train_with`.
//! Dokładamy tylko trzy rzeczy, wszystkie po stronie serwera:
//!  * meldunek po każdym pokoleniu (wykres fitness rośnie na oczach),
//!  * postęp WEWNĄTRZ pokolenia z licznika ocenionych osobników,
//!  * token przerwania — sprawdzany po zapisie punktu kontrolnego, więc
//!    „PRZERWIJ" nigdy nie kosztuje więcej niż jedno pokolenie.
//!
//! Punkty kontrolne i wznawianie są własnością `crates/ai` (`TrainCfg.checkpoint`
//! / `.resume`) — tutaj tylko wskazujemy plik i pokazujemy, czy da się wznowić.
//! Wynik na oknach TRENINGOWYCH nie mówi nic poza tym, że optymalizator działa;
//! jedyną liczbą, która cokolwiek znaczy, jest ocena na walidacji — i tę
//! pokazujemy osobno, nigdy zamiast.

use super::{dzien, parse_dzien, JobCtx, LabGen, LabTrain};
use crate::state::StateHandle;
use crate::store::Workspace;
use conduit_ai::policy::{Model, TrainMeta};
use conduit_ai::reward::{summarize, RewardWeights};
use conduit_ai::rollout::{
    day_label, run_windows, split_alternating, split_windows, training_settings_with, RunCfg,
};
use conduit_ai::safety::SafetyCfg;
use conduit_ai::train::{train_with, Algo, TrainCfg};
use conduit_backtest::{load_messages, TickData};
use conduit_core::settings::TpSchedule;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

// ============================================================
//  ZLECENIE
// ============================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainReq {
    /// okno treningowe
    pub from: String,
    pub to: String,
    /// okno walidacyjne — MUSI być rozłączne z treningowym
    pub valid_from: String,
    pub valid_to: String,
    #[serde(default = "d_gen")]
    pub generations: usize,
    #[serde(default = "d_pop")]
    pub pop: usize,
    #[serde(default = "d_seed")]
    pub seed: u64,
    #[serde(default = "d_algo")]
    pub algo: String,
    #[serde(default = "d_sigma")]
    pub sigma: f32,
    #[serde(default = "d_lr")]
    pub lr: f32,
    #[serde(default = "d_windows")]
    pub windows: usize,
    #[serde(default = "d_window_days")]
    pub window_days: f64,
    #[serde(default = "d_valid_windows")]
    pub valid_windows: usize,
    #[serde(default = "d_balance")]
    pub balance: f64,
    #[serde(default = "d_interval")]
    pub interval: f64,
    /// `chrono` albo `interleave`
    #[serde(default = "d_split")]
    pub split: String,
    #[serde(default = "d_blocks")]
    pub blocks: usize,
    /// nazwa pliku modelu wyjściowego (bez ścieżki)
    #[serde(default)]
    pub name: String,
    /// wznów z punktu kontrolnego zamiast zaczynać od zera
    #[serde(default)]
    pub resume: bool,
}

fn d_gen() -> usize {
    30
}
fn d_pop() -> usize {
    32
}
fn d_seed() -> u64 {
    1
}
fn d_algo() -> String {
    "es".into()
}
fn d_sigma() -> f32 {
    0.05
}
fn d_lr() -> f32 {
    0.05
}
fn d_windows() -> usize {
    6
}
fn d_window_days() -> f64 {
    3.0
}
fn d_valid_windows() -> usize {
    5
}
fn d_balance() -> f64 {
    1000.0
}
fn d_interval() -> f64 {
    2.0
}
fn d_split() -> String {
    "chrono".into()
}
fn d_blocks() -> usize {
    10
}

impl Default for TrainReq {
    fn default() -> Self {
        TrainReq {
            from: String::new(),
            to: String::new(),
            valid_from: String::new(),
            valid_to: String::new(),
            generations: d_gen(),
            pop: d_pop(),
            seed: d_seed(),
            algo: d_algo(),
            sigma: d_sigma(),
            lr: d_lr(),
            windows: d_windows(),
            window_days: d_window_days(),
            valid_windows: d_valid_windows(),
            balance: d_balance(),
            interval: d_interval(),
            split: d_split(),
            blocks: d_blocks(),
            name: String::new(),
            resume: false,
        }
    }
}

// ============================================================
//  PUNKT KONTROLNY
// ============================================================

/// Jeden punkt kontrolny na katalog roboczy. Trening jest wyłączny (jedno
/// zadanie naraz), więc nie ma po co mnożyć plików — a stała nazwa sprawia,
/// że „wznów" po restarcie programu nie wymaga pamiętania, które to było
/// zadanie.
pub fn checkpoint_path(ws: &Workspace) -> PathBuf {
    ws.lab_dir().join("trening_checkpoint.json")
}

/// Skrót informacji o zapisanym punkcie kontrolnym — do ekranu „wznów".
pub fn checkpoint_info(ws: &Workspace) -> Option<super::CheckpointInfo> {
    let p = checkpoint_path(ws);
    let cp = conduit_ai::train::Checkpoint::load(&p).ok()?;
    let saved_at = std::fs::metadata(&p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some(super::CheckpointInfo {
        gen: cp.gen,
        best_center: if cp.best_center.is_finite() {
            cp.best_center
        } else {
            0.0
        },
        seed: cp.seed,
        algo: cp.algo,
        saved_at,
    })
}

// ============================================================
//  URUCHOMIENIE
// ============================================================

pub fn start(st: &StateHandle, req: TrainReq) -> anyhow::Result<String> {
    let (ticks_path, signals_path) = super::data_paths(&st.workspace);
    if !ticks_path.is_file() {
        anyhow::bail!("{}", super::gdzie_szukalem(&st.workspace));
    }
    if !signals_path.is_file() {
        anyhow::bail!("nie znalazłem pliku sygnałów: {}", signals_path.display());
    }
    if Algo::parse(&req.algo).is_none() {
        anyhow::bail!("nieznany algorytm „{}” (es | cem)", req.algo);
    }
    if req.split != "chrono" && req.split != "interleave" {
        anyhow::bail!("nieznany podział „{}” (chrono | interleave)", req.split);
    }
    if req.generations == 0 {
        anyhow::bail!("liczba pokoleń musi być większa od zera");
    }
    if req.pop < 4 {
        anyhow::bail!("populacja musi mieć co najmniej 4 osobniki");
    }
    // daty sprawdzamy TERAZ — literówka ma wrócić jako błąd formularza,
    // a nie jako zadanie, które umiera sekundę po starcie
    for (etykieta, v) in [
        ("początek treningu", &req.from),
        ("koniec treningu", &req.to),
        ("początek walidacji", &req.valid_from),
        ("koniec walidacji", &req.valid_to),
    ] {
        if !v.trim().is_empty() {
            parse_dzien(v).map_err(|e| anyhow::anyhow!("{etykieta}: {e}"))?;
        }
    }
    if req.resume && !checkpoint_path(&st.workspace).is_file() {
        anyhow::bail!("nie ma punktu kontrolnego do wznowienia");
    }

    let tytul = format!(
        "{} · pop {} · {} pokoleń · ziarno {} · trening {} … {} / walidacja {} … {}",
        req.algo.to_uppercase(),
        req.pop,
        req.generations,
        req.seed,
        pusty_na(&req.from, "początek"),
        pusty_na(&req.to, "połowa"),
        pusty_na(&req.valid_from, "połowa"),
        pusty_na(&req.valid_to, "koniec"),
    );

    super::spawn_job(st, "train", tytul, move |ctx| licz(ctx, req))
}

fn pusty_na<'a>(s: &'a str, zamiast: &'a str) -> &'a str {
    if s.trim().is_empty() {
        zamiast
    } else {
        s
    }
}

fn licz(ctx: &JobCtx, req: TrainReq) -> anyhow::Result<String> {
    let ws = ctx.workspace().clone();
    let (ticks_path, signals_path) = super::data_paths(&ws);

    ctx.edit(true, |j| j.label = "wczytywanie ticków…".into());
    let td = TickData::open(&ticks_path)?;
    ctx.edit(true, |j| j.label = "wczytywanie sygnałów…".into());
    let msgs = load_messages(&signals_path)?;

    // ---------- okna ----------
    let t_from = if req.from.trim().is_empty() {
        td.first_ts()
    } else {
        parse_dzien(&req.from)?
    };
    let t_to = if req.to.trim().is_empty() {
        td.first_ts() + (td.last_ts() - td.first_ts()) / 2
    } else {
        parse_dzien(&req.to)?
    };
    let v_from = if req.valid_from.trim().is_empty() {
        t_to
    } else {
        parse_dzien(&req.valid_from)?
    };
    let v_to = if req.valid_to.trim().is_empty() {
        td.last_ts()
    } else {
        parse_dzien(&req.valid_to)?
    };

    if t_to <= t_from || v_to <= v_from {
        anyhow::bail!("pusty zakres treningowy albo walidacyjny");
    }
    if req.split == "chrono" && v_from < t_to {
        anyhow::bail!(
            "zakres walidacyjny ({}) zachodzi na treningowy (koniec {}) — wyniki byłyby bezwartościowe",
            dzien(v_from),
            dzien(t_to)
        );
    }

    let (train_ws, valid_ws, opis) = match req.split.as_str() {
        "interleave" => {
            let (tr, va) =
                split_alternating(t_from, v_to, req.blocks, req.window_days, req.window_days);
            (
                tr,
                va,
                format!(
                    "przeplatany: {} bloków w zakresie {} … {}",
                    req.blocks,
                    day_label(t_from),
                    day_label(v_to)
                ),
            )
        }
        _ => (
            split_windows(t_from, t_to, req.windows, req.window_days),
            split_windows(v_from, v_to, req.valid_windows, req.window_days),
            format!(
                "chronologiczny: trening {} … {}, walidacja {} … {}",
                day_label(t_from),
                day_label(t_to),
                day_label(v_from),
                day_label(v_to)
            ),
        ),
    };
    if train_ws.is_empty() || valid_ws.is_empty() {
        anyhow::bail!("nie udało się zbudować okien — zakres jest krótszy niż długość okna");
    }
    // niezależnie od trybu: zbiory MUSZĄ być rozłączne
    for t in &train_ws {
        for v in &valid_ws {
            if t.to > v.from && v.to > t.from {
                anyhow::bail!(
                    "okno treningowe {} zachodzi na walidacyjne {}",
                    t.label,
                    v.label
                );
            }
        }
    }

    // ---------- konfiguracja ----------
    let reward = RewardWeights::default();
    let mut settings = training_settings_with(TpSchedule::AllRunners);
    settings.ai_decision_interval_s = req.interval;
    let rc = RunCfg {
        start_balance: req.balance,
        settings,
        warmup_min: 120,
        no_sl_atr: reward.no_sl_atr,
        flat_at_end: true,
        msg_offset_ms: (180.0 * 60_000.0) as i64,
    };
    let safety = SafetyCfg::default();

    let eval_done = Arc::new(AtomicU64::new(0));
    let cancel: Arc<AtomicBool> = ctx.cancel.clone();
    let ckpt = checkpoint_path(&ws);

    let tc = TrainCfg {
        algo: Algo::parse(&req.algo).unwrap_or(Algo::Es),
        generations: req.generations,
        pop: req.pop,
        seed: req.seed,
        sigma: req.sigma,
        lr: req.lr,
        reward: reward.clone(),
        checkpoint: Some(ckpt.clone()),
        resume: req.resume,
        cancel: Some(cancel),
        eval_done: Some(eval_done.clone()),
        ..TrainCfg::default()
    };

    let mut proto = Model::fresh(&[48, 32], &[32, 24], req.seed, safety);
    let nazwa = if req.name.trim().is_empty() {
        format!(
            "{}.json",
            super::bezpieczna_nazwa(&format!("ai_{}", req.seed))
        )
    } else {
        format!(
            "{}.json",
            super::bezpieczna_nazwa(req.name.trim().trim_end_matches(".json"))
        )
    };
    let model_path = ws.models_dir().join(&nazwa);
    proto.name = nazwa.clone();
    proto.train = TrainMeta {
        algo: tc.algo.name().into(),
        generations: tc.generations,
        pop: tc.pop,
        seed: tc.seed,
        sigma: tc.sigma,
        lr: tc.lr,
        windows: train_ws
            .iter()
            .map(|w| format!("{}..{}", w.label, day_label(w.to)))
            .collect(),
        start_balance: req.balance,
        decision_interval_s: req.interval,
        engine_tp_mode: "last".into(),
        split: opis.clone(),
        reward: reward.clone(),
    };

    // Liczba backtestów na pokolenie: (kandydaci + środek rozkładu) × okna.
    // ES pracuje na parach lustrzanych, więc zaokrągla populację w dół do
    // parzystej; CEM bierze ją wprost, ale nie mniej niż 4.
    let kandydatow = match tc.algo {
        Algo::Es => (tc.pop / 2).max(1) * 2,
        Algo::Cem => tc.pop.max(4),
    };
    let na_pokolenie = (kandydatow + 1) as u64 * train_ws.len() as u64;

    ctx.edit(true, |j| {
        j.total = req.generations as u64;
        j.label = "linia bazowa (model startowy nic nie robi)…".into();
        j.train = Some(LabTrain {
            eval_total: na_pokolenie,
            model_path: model_path.display().to_string(),
            checkpoint_path: ckpt.display().to_string(),
            resumable: ckpt.is_file(),
            ..Default::default()
        });
    });

    // ---------- linia bazowa ----------
    // Bez niej wynik treningu jest nieinterpretowalny: nie wiadomo, czy model
    // cokolwiek wniósł, czy tylko powtórzył to, co robi sam sygnał.
    let base_tr = summarize(&run_windows(&td, &msgs, &train_ws, &rc, &proto), &reward);
    let base_va = summarize(&run_windows(&td, &msgs, &valid_ws, &rc, &proto), &reward);
    ctx.edit(true, |j| {
        if let Some(t) = j.train.as_mut() {
            t.baseline_train = base_tr.fitness;
            t.baseline_valid = base_va.fitness;
        }
    });

    if ctx.cancelled() {
        return Ok("przerwane przed pierwszym pokoleniem".into());
    }

    // ---------- ewolucja ----------
    // Zegar pokoleń trzymamy tutaj, bo `elapsed_s` z raportu dotyczy samego
    // liczenia, a użytkownik pyta o czas ścienny.
    let start = std::time::Instant::now();
    let gen_gotowych = Arc::new(AtomicU64::new(0));
    let stop_puls = Arc::new(AtomicBool::new(false));
    let mut gens: Vec<LabGen> = Vec::with_capacity(req.generations);
    let n_gen = req.generations as u64;

    // `thread::scope` pozwala wątkowi pulsu pożyczyć `ctx` bez `'static`
    // i gwarantuje, że wątek skończy się przed wyjściem z tego bloku.
    let best = std::thread::scope(|s| {
        // Postęp WEWNĄTRZ pokolenia. Bez niego pasek stałby nieruchomo przez
        // całe pokolenie — przy 30 sekundach na pokolenie wygląda to jak
        // zawieszenie programu. Licznik `eval_done` podbija `crates/ai` po
        // każdym ocenionym oknie, więc to jest realny postęp, nie animacja.
        let stop = stop_puls.clone();
        let ed = eval_done.clone();
        let gg = gen_gotowych.clone();
        s.spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(300));
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let done = ed.load(Ordering::Relaxed).min(na_pokolenie);
                let g = gg.load(Ordering::Relaxed);
                if g >= n_gen {
                    continue;
                }
                let ulamek = if na_pokolenie > 0 {
                    done as f64 / na_pokolenie as f64
                } else {
                    0.0
                };
                ctx.edit(false, |j| {
                    j.progress = ((g as f64 + ulamek) / n_gen as f64).clamp(0.0, 1.0);
                    j.label = format!(
                        "pokolenie {}/{} · osobnik {}/{}",
                        g + 1,
                        n_gen,
                        done,
                        na_pokolenie
                    );
                    if let Some(t) = j.train.as_mut() {
                        t.eval_done = done;
                    }
                });
            }
        });

        let mut pierwsze: Option<usize> = None;
        let (model, _reports) = train_with(
            &td,
            &msgs,
            &train_ws,
            &rc,
            &proto,
            &tc,
            |r| {
                if pierwsze.is_none() {
                    pierwsze = Some(r.gen);
                }
                let od = pierwsze.unwrap_or(0);
                let zrobione = (r.gen - od + 1) as f64;

                gens.push(LabGen {
                    gen: r.gen,
                    center: r.center,
                    best: r.best,
                    median: r.median,
                    worst: r.worst,
                    pnl: r.center_summary.pnl,
                    max_dd: r.center_summary.max_dd_abs,
                    trades: r.center_summary.trades,
                    sigma: r.sigma as f64,
                    elapsed_s: r.elapsed_s,
                });
                gen_gotowych.store(r.gen as u64 + 1, Ordering::Relaxed);

                let minuty = start.elapsed().as_secs_f64() / 60.0;
                let gpm = if minuty > 0.0 { zrobione / minuty } else { 0.0 };
                let zostalo = n_gen.saturating_sub(r.gen as u64 + 1);
                let najlepszy = gens
                    .iter()
                    .max_by(|a, b| {
                        a.center
                            .partial_cmp(&b.center)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .cloned()
                    .unwrap_or_default();
                let widok = gens.clone();

                ctx.edit(true, |j| {
                    j.progress = ((r.gen as f64 + 1.0) / n_gen as f64).clamp(0.0, 1.0);
                    j.done = r.gen as u64 + 1;
                    j.total = n_gen;
                    j.label = format!("pokolenie {}/{} · gotowe", r.gen + 1, n_gen);
                    j.speed = format!("{gpm:.2} pokolenia/min");
                    j.speed2 = format!("{:.1} s/pokolenie · zostało {zostalo}", r.elapsed_s);
                    j.gens = widok;
                    if let Some(t) = j.train.as_mut() {
                        t.best_fitness = najlepszy.center;
                        t.best_gen = najlepszy.gen;
                        t.median_fitness = r.median;
                        t.best_dd = najlepszy.max_dd;
                        t.best_pnl = najlepszy.pnl;
                        t.eval_done = na_pokolenie;
                        t.resumable = true;
                    }
                });
            },
            |gen, best| {
                ctx.log(
                    "info",
                    "Wznowiono trening z punktu kontrolnego",
                    format!("od pokolenia {gen}, najlepszy fitness {best:+.4}"),
                );
            },
            |powod| {
                ctx.log(
                    "warn",
                    "Nie udało się wznowić — trening rusza od zera",
                    powod.to_string(),
                );
            },
        );
        stop_puls.store(true, Ordering::Relaxed);
        model
    });

    let przerwane = ctx.cancelled();

    // ---------- ocena końcowa ----------
    ctx.edit(true, |j| j.label = "ocena na oknach walidacyjnych…".into());
    let fin_tr = summarize(&run_windows(&td, &msgs, &train_ws, &rc, &best), &reward);
    let fin_va = summarize(&run_windows(&td, &msgs, &valid_ws, &rc, &best), &reward);

    let mut out = best;
    out.created = chrono::Utc::now().to_rfc3339();
    out.score.fitness = fin_va.fitness;
    out.score.pnl = fin_va.pnl;
    out.score.max_dd = fin_va.max_dd_abs;
    out.score.profit_factor = fin_va.profit_factor;
    out.score.trades = fin_va.trades;
    out.score.win_rate = fin_va.win_rate;
    out.score.note = format!(
        "walidacja {} … {}: PnL {:+.2} $, MaxDD {:.2} $; trening: PnL {:+.2} $; linia bazowa (walidacja): PnL {:+.2} ${}",
        dzien(v_from),
        dzien(v_to),
        fin_va.pnl,
        fin_va.max_dd_abs,
        fin_tr.pnl,
        base_va.pnl,
        if przerwane { "; TRENING PRZERWANY" } else { "" }
    );

    std::fs::create_dir_all(ws.models_dir()).ok();
    out.save(&model_path)?;
    // kopia w katalogu zadania — żeby wynik konkretnego przebiegu dało się
    // odtworzyć nawet po nadpisaniu pliku w `models/`
    let _ = out.save(ctx.out_dir.join(&nazwa));
    super::zapisz_json(&ctx.out_dir, "pokolenia.json", &gens)?;

    ctx.edit(true, |j| {
        j.label = if przerwane {
            "przerwane — model i punkt kontrolny zapisane".into()
        } else {
            "gotowe".into()
        };
        if let Some(t) = j.train.as_mut() {
            t.valid_fitness = fin_va.fitness;
            t.valid_pnl = fin_va.pnl;
            t.model_path = model_path.display().to_string();
            t.resumable = ckpt.is_file();
        }
    });

    let mut note = format!(
        "walidacja: fitness {:+.4} (baza {:+.4}) · PnL {:+.2} $ · MaxDD {:.2} $ · model {}",
        fin_va.fitness, base_va.fitness, fin_va.pnl, fin_va.max_dd_abs, nazwa
    );
    if fin_va.fitness < base_va.fitness {
        note.push_str(" · UWAGA: gorszy od linii bazowej — to przeuczenie, nie wynik");
    }
    if przerwane {
        note.push_str(" · PRZERWANE — wznów z punktu kontrolnego");
    }
    Ok(note)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domyslne_zlecenie_jest_sensowne() {
        let r = TrainReq::default();
        assert_eq!(r.algo, "es");
        assert_eq!(r.split, "chrono");
        assert!(r.generations > 0 && r.pop >= 4);
    }

    /// Zlecenie z interfejsu przychodzi w camelCase i nie musi mieć
    /// wszystkich pól — brakujące mają wypełnić się wartościami domyślnymi.
    #[test]
    fn zlecenie_wczytuje_sie_z_czesciowego_json() {
        let r: TrainReq = serde_json::from_str(
            r#"{"from":"2026-04-01","to":"2026-06-01","validFrom":"2026-06-01","validTo":"2026-07-01","generations":5}"#,
        )
        .unwrap();
        assert_eq!(r.generations, 5);
        assert_eq!(r.pop, d_pop());
        assert_eq!(r.valid_from, "2026-06-01");
        assert!(!r.resume);
    }

    #[test]
    fn okna_szkolenia_i_walidacji_musza_byc_rozlaczne() {
        let tr = split_windows(0, 10 * 86_400_000, 3, 1.0);
        let va = split_windows(10 * 86_400_000, 20 * 86_400_000, 3, 1.0);
        for t in &tr {
            for v in &va {
                assert!(
                    !(t.to > v.from && v.to > t.from),
                    "okna nachodzą: {} vs {}",
                    t.label,
                    v.label
                );
            }
        }
    }
}
