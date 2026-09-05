//! Trening modelu AI na realnych danych.
//!
//! ```text
//! cargo run --release -p conduit-ai --bin train -- \
//!     --ticks data/ticks.bin --signals data/signals.json \
//!     --from 2026-04-01 --to 2026-06-01 \
//!     --valid-from 2026-06-01 --valid-to 2026-07-27 \
//!     --generations 30 --pop 32 --seed 1 --out models/atfx_v1.json
//! ```
//!
//! Okna treningowe i walidacyjne są ROZŁĄCZNE i raportowane osobno. Wynik na
//! oknach treningowych mówi tylko tyle, że optymalizator działa; jedyną liczbą,
//! która cokolwiek znaczy, jest wynik na walidacji.

use anyhow::{bail, Context, Result};
use conduit_ai::policy::{ActionMode, Model, TrainMeta};
use conduit_ai::reward::{aggregate_mar, bootstrap_ci, min_capital_for, ruin_probability};
use conduit_ai::reward::{summarize, RewardWeights, Summary, ACTION_NAMES};
use conduit_ai::rollout::{
    day_label, deep_settings, pole_compound, pole_daily, run_windows, split_alternating,
    split_windows, training_settings_with, Pole, RunCfg, Window,
};
use conduit_ai::safety::SafetyCfg;
use conduit_ai::train::{train_with, Algo, TrainCfg};
use conduit_backtest::{load_messages, ReplayMessage, TickData};
use conduit_core::settings::TpSchedule;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// ============================================================
//  ARGUMENTY
// ============================================================

struct Args {
    ticks: String,
    signals: String,
    from: Option<String>,
    to: Option<String>,
    valid_from: Option<String>,
    valid_to: Option<String>,
    out: String,
    generations: usize,
    pop: usize,
    seed: u64,
    algo: String,
    sigma: f32,
    lr: f32,
    balance: f64,
    windows: usize,
    window_days: f64,
    valid_windows: usize,
    interval: f64,
    hidden_pos: Vec<usize>,
    hidden_bsk: Vec<usize>,
    threads: usize,
    k_dd: f64,
    k_risk: f64,
    k_hold: f64,
    floor_pct: f64,
    max_margin: f64,
    max_lots: f64,
    msg_offset_min: f64,
    tp_mode: String,
    split: String,
    blocks: usize,
    valid_days: f64,
    patience: usize,
    load: Option<String>,
    w_min: f64,
    checkpoint: Option<String>,
    resume: bool,
    preset: Option<String>,
    engine: String,
    swap_folds: bool,
    actions: String,
    regime_split: String,
    relative: bool,
    stress_from: String,
    stress_to: String,
    six: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            ticks: "data/ticks.bin".into(),
            signals: "data/signals.json".into(),
            from: None,
            to: None,
            valid_from: None,
            valid_to: None,
            out: "models/ai.json".into(),
            generations: 30,
            pop: 32,
            seed: 1,
            algo: "es".into(),
            sigma: 0.05,
            lr: 0.05,
            balance: 1000.0,
            windows: 6,
            window_days: 1.0,
            valid_windows: 5,
            interval: 2.0,
            hidden_pos: vec![48, 32],
            hidden_bsk: vec![32, 24],
            threads: 0,
            k_dd: -1.0,
            k_risk: -1.0,
            k_hold: -1.0,
            floor_pct: 60.0,
            max_margin: 30.0,
            max_lots: 1.0,
            msg_offset_min: 180.0,
            tp_mode: "last".into(),
            split: "chrono".into(),
            blocks: 10,
            valid_days: 0.0,
            patience: 8,
            load: None,
            w_min: -1.0,
            checkpoint: None,
            resume: false,
            preset: None,
            engine: "deep".into(),
            swap_folds: false,
            actions: "entry".into(),
            regime_split: "2026-06-01".into(),
            relative: true,
            stress_from: "2026-07-22".into(),
            stress_to: "2026-07-26".into(),
            six: false,
        }
    }
}

fn parse_dims(s: &str) -> Vec<usize> {
    s.split(',')
        .filter_map(|x| x.trim().parse::<usize>().ok())
        .filter(|x| *x > 0)
        .collect()
}

/// „2026-04-15" albo „2026-04-15T13:00" → ms epoki (UTC).
fn parse_date(s: &str) -> Result<i64> {
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
    let t = s.trim();
    let dt: NaiveDateTime = if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        d.and_hms_opt(0, 0, 0).unwrap()
    } else if let Ok(d) = NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M") {
        d
    } else {
        bail!("nie rozumiem daty '{s}' (oczekuję RRRR-MM-DD)");
    };
    Ok(Utc.from_utc_datetime(&dt).timestamp_millis())
}

fn parse_args() -> Result<Args> {
    let mut a = Args::default();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let k = argv[i].as_str();
        let mut val = || -> Result<String> {
            i += 1;
            argv.get(i)
                .cloned()
                .with_context(|| format!("brak wartości dla {k}"))
        };
        match k {
            "--ticks" => a.ticks = val()?,
            "--signals" => a.signals = val()?,
            "--from" => a.from = Some(val()?),
            "--to" => a.to = Some(val()?),
            "--valid-from" => a.valid_from = Some(val()?),
            "--valid-to" => a.valid_to = Some(val()?),
            "--out" => a.out = val()?,
            "--generations" | "--gens" => a.generations = val()?.parse()?,
            "--pop" => a.pop = val()?.parse()?,
            "--seed" => a.seed = val()?.parse()?,
            "--algo" => a.algo = val()?,
            "--sigma" => a.sigma = val()?.parse()?,
            "--lr" => a.lr = val()?.parse()?,
            "--balance" => a.balance = val()?.parse()?,
            "--windows" => a.windows = val()?.parse()?,
            "--window-days" => a.window_days = val()?.parse()?,
            "--valid-windows" => a.valid_windows = val()?.parse()?,
            "--interval" => a.interval = val()?.parse()?,
            "--hidden-pos" => a.hidden_pos = parse_dims(&val()?),
            "--hidden-bsk" => a.hidden_bsk = parse_dims(&val()?),
            "--threads" => a.threads = val()?.parse()?,
            "--k-dd" => a.k_dd = val()?.parse()?,
            "--k-risk" => a.k_risk = val()?.parse()?,
            "--k-hold" => a.k_hold = val()?.parse()?,
            "--floor-pct" => a.floor_pct = val()?.parse()?,
            "--max-margin" => a.max_margin = val()?.parse()?,
            "--max-lots" => a.max_lots = val()?.parse()?,
            "--msg-offset-min" => a.msg_offset_min = val()?.parse()?,
            "--tp-mode" => a.tp_mode = val()?,
            "--split" => a.split = val()?,
            "--blocks" => a.blocks = val()?.parse()?,
            "--valid-days" => a.valid_days = val()?.parse()?,
            "--patience" => a.patience = val()?.parse()?,
            "--load" => a.load = Some(val()?),
            "--w-min" => a.w_min = val()?.parse()?,
            "--checkpoint" => a.checkpoint = Some(val()?),
            "--resume" => a.resume = true,
            "--preset" => a.preset = Some(val()?),
            "--engine" => a.engine = val()?,
            "--swap-folds" => a.swap_folds = true,
            "--actions" => a.actions = val()?,
            "--regime-split" => a.regime_split = val()?,
            "--absolute" => a.relative = false,
            "--six" => a.six = true,
            "--stress-from" => a.stress_from = val()?,
            "--stress-to" => a.stress_to = val()?,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => bail!("nieznany argument: {other}"),
        }
        i += 1;
    }
    Ok(a)
}

fn print_help() {
    println!(
        "\ntrening modelu zarządzania pozycjami (CONDUIT AI)\n\n\
         DANE\n  \
           --ticks PLIK            plik ticków .bin        (data/ticks.bin)\n  \
           --signals PLIK          eksport sygnałów .json  (data/signals.json)\n  \
           --from RRRR-MM-DD       początek zakresu TRENINGOWEGO\n  \
           --to RRRR-MM-DD         koniec zakresu treningowego\n  \
           --valid-from/--valid-to zakres WALIDACYJNY (musi być rozłączny)\n\n\
         OKNA\n  \
           --windows N             liczba okien treningowych      (6)\n  \
           --window-days D         długość okna w dniach          (3)\n  \
           --valid-windows N       liczba okien walidacyjnych     (5)\n\n\
         EWOLUCJA\n  \
           --algo es|cem           algorytm                       (es)\n  \
           --generations N         liczba pokoleń                 (30)\n  \
           --pop N                 liczebność populacji           (32)\n  \
           --sigma X --lr X        siła szumu i krok uczenia      (0.05 / 0.05)\n  \
           --seed N                ziarno (pełny determinizm)     (1)\n  \
           --threads N             liczba wątków, 0 = wszystkie   (0)\n\n\
         MODEL I RYZYKO\n  \
           --hidden-pos 48,32      warstwy ukryte sieci pozycji\n  \
           --hidden-bsk 32,24      warstwy ukryte sieci koszyka\n  \
           --balance X             kapitał startowy               (1000)\n  \
           --interval S            interwał decyzji w sekundach   (2)\n  \
           --floor-pct X           podłoga equity w % kapitału    (60)\n  \
           --max-margin X          sufit margin/equity w %        (30)\n  \
           --max-lots X            sufit łącznego wolumenu        (1.0)\n  \
           --k-dd --k-risk --k-hold  wagi kar w funkcji nagrody\n\n\
         WYNIK\n  \
           --out PLIK              gdzie zapisać model            (models/ai.json)\n"
    );
}

// ============================================================
//  RAPORT
// ============================================================

fn fmt_pf(pf: f64) -> String {
    if pf.is_finite() {
        format!("{pf:.2}")
    } else {
        "∞".into()
    }
}

fn print_summary(tag: &str, s: &Summary, cap: f64) {
    println!(
        "  {tag:<12} PnL {:+9.2} $ ({:+6.2} %) · MaxDD {:7.2} $ ({:5.2} %) · maxRyzyko {:7.2} $ · \
         PF {:>5} · {:4} trans · WR {:5.1} % · dni+ {}/{} · akcje {}",
        s.pnl,
        s.pnl / cap * 100.0,
        s.max_dd_abs,
        s.max_dd_pct,
        s.max_open_risk,
        fmt_pf(s.profit_factor),
        s.trades,
        s.win_rate * 100.0,
        s.positive_windows,
        s.windows,
        s.actions
    );
    println!(
        "  {:<12} sygnały {} · koszyki {} · zlecenia rynkowe {}/{} · limity {}/{} · zafillowane {} · odrzuty SL/TP {}",
        "", s.signals, s.baskets, s.orders[0], s.orders[1], s.orders[2], s.orders[3],
        s.filled_pendings, s.rejected_stops
    );
    println!(
        "  {:<12} MAR {:5.2} · min. equity {:5.1} % kapitału · okien stratnych {}/{} · najgorsze {:+.2} %",
        "",
        s.mar,
        s.min_equity_ratio * 100.0,
        s.losing_windows,
        s.windows,
        s.worst_window_pct
    );
    if s.repositions > 0 {
        println!(
            "  {:<12} przestawień wejścia: {} ({} jednostek, średnia głębokość {:.2} szer. strefy)",
            "",
            s.repositions,
            s.repos_units,
            s.depth_sum / s.repositions as f64
        );
    }
    if s.actions > 0 {
        let br: Vec<String> = ACTION_NAMES
            .iter()
            .zip(s.acts.iter())
            .filter(|(_, n)| **n > 0)
            .map(|(k, n)| format!("{k} {n}"))
            .collect();
        println!("  {:<12} akcje: {}", "", br.join(" · "));
    }
    if s.blown > 0 {
        println!(
            "  {:<12} !! {} okien z wyzerowanym kontem / przebitą podłogą",
            "", s.blown
        );
    }
}

fn wiersz(nazwa: &str, c: &Pole, d: &Pole) {
    println!(
        "  {nazwa:<14} │ {:+9.2} $  ×{:5.2}  PF {:>5}  DD {:6.2}  dni+ {:4.0} %  min.eq {:5.1} %  najg.dz {:+6.2} % │ {:+9.2} $  PF {:>5}  DD {:6.2}  dni+ {:4.0} %",
        c.pnl, c.mult, fmt_pf(c.pf), c.max_dd, c.pct_dni_plus(), c.min_equity_pct, c.worst_day_pct,
        d.pnl, fmt_pf(d.pf), d.max_dd, d.pct_dni_plus()
    );
}

/// Raport sześciopolowy: {całość, od 1 czerwca, ostatni miesiąc} × {compounding, dzień-po-dniu}.
fn raport_szesciopolowy(
    td: &TickData,
    msgs: &[ReplayMessage],
    rc: &conduit_ai::rollout::RunCfg,
    model: &Model,
    tytul: &str,
) -> anyhow::Result<()> {
    let koniec = td.last_ts();
    let zakresy = [
        ("całość", td.first_ts().max(parse_date("2026-04-01")?)),
        ("od 1 czerwca", parse_date("2026-06-01")?),
        ("ostatni miesiąc", koniec - 30 * 86_400_000),
    ];
    println!(
        "
── {tytul} ──"
    );
    println!(
        "  {:<14} │ {:^76} │ {:^46}",
        "pole", "COMPOUNDING", "DZIEŃ-PO-DNIU"
    );
    for (nazwa, od) in zakresy {
        if od >= koniec {
            continue;
        }
        let c = pole_compound(td, msgs, od, koniec, rc, model);
        let d = pole_daily(td, msgs, od, koniec, rc, model);
        wiersz(nazwa, &c, &d);
    }
    Ok(())
}

fn windows_label(ws: &[Window]) -> Vec<String> {
    ws.iter()
        .map(|w| format!("{}..{}", w.label, day_label(w.to)))
        .collect()
}

// ============================================================
//  MAIN
// ============================================================

fn main() -> Result<()> {
    let a = parse_args()?;
    if a.threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(a.threads)
            .build_global()
            .ok();
    }

    // ---------- dane ----------
    let td = TickData::open(&a.ticks)?;
    let msgs: Vec<ReplayMessage> = load_messages(&a.signals)?;
    println!(
        "dane: {} ticków {} … {} · {} wiadomości",
        td.len(),
        day_label(td.first_ts()),
        day_label(td.last_ts()),
        msgs.len()
    );

    let t_from = match &a.from {
        Some(s) => parse_date(s)?,
        None => td.first_ts(),
    };
    let t_to = match &a.to {
        Some(s) => parse_date(s)?,
        None => td.first_ts() + (td.last_ts() - td.first_ts()) / 2,
    };
    let v_from = match &a.valid_from {
        Some(s) => parse_date(s)?,
        None => t_to,
    };
    let v_to = match &a.valid_to {
        Some(s) => parse_date(s)?,
        None => td.last_ts(),
    };
    if t_to <= t_from || v_to <= v_from {
        bail!("pusty zakres treningowy albo walidacyjny");
    }
    if a.split == "chrono" && v_from < t_to {
        bail!(
            "zakres walidacyjny ({}) zachodzi na treningowy (koniec {}) — wyniki byłyby bezwartościowe",
            day_label(v_from),
            day_label(t_to)
        );
    }

    let valid_days = if a.valid_days > 0.0 {
        a.valid_days
    } else {
        a.window_days
    };
    let (train_ws, valid_ws, split_desc) = match a.split.as_str() {
        "chrono" => (
            split_windows(t_from, t_to, a.windows, a.window_days),
            split_windows(v_from, v_to, a.valid_windows, a.window_days),
            format!(
                "chronologiczny: trening {} … {}, walidacja {} … {}",
                day_label(t_from),
                day_label(t_to),
                day_label(v_from),
                day_label(v_to)
            ),
        ),
        "interleave" => {
            // przeplot obejmuje CAŁY zakres: od początku treningu do końca walidacji
            let (tr, va) = split_alternating(t_from, v_to, a.blocks, a.window_days, valid_days);
            (
                tr,
                va,
                format!(
                    "przeplatany: {} bloków w zakresie {} … {}, w każdym {:.1} dnia treningu + {:.1} dnia walidacji",
                    a.blocks,
                    day_label(t_from),
                    day_label(v_to),
                    a.window_days,
                    valid_days
                ),
            )
        }
        other => bail!("nieznany --split '{other}' (chrono|interleave)"),
    };
    // Zamiana fałd. Przy przeplocie zbiory mają tę samą strukturę, więc oba
    // przebiegi razem dają każdemu oknu dokładnie jedno wystąpienie poza próbą —
    // to jest dwukrotna walidacja krzyżowa, a nie wybieranie wygodnego podziału.
    let (train_ws, valid_ws) = if a.swap_folds {
        (valid_ws, train_ws)
    } else {
        (train_ws, valid_ws)
    };
    if train_ws.is_empty() || valid_ws.is_empty() {
        bail!("nie udało się zbudować okien");
    }
    // niezależnie od trybu: zbiory MUSZĄ być rozłączne
    for t in &train_ws {
        for v in &valid_ws {
            if t.to > v.from && v.to > t.from {
                bail!(
                    "okno treningowe {} zachodzi na walidacyjne {}",
                    t.label,
                    v.label
                );
            }
        }
    }

    // ---------- konfiguracja ----------
    let mut reward = RewardWeights::default();
    if a.relative {
        // Ryzyko pilnuje WYŁĄCZNIE warstwa wykonania (podłoga equity, sufit
        // marginu, limit ryzyka na koszyk, sufit wolumenu od kapitału
        // startowego). Kara w nagrodzie zawsze konkuruje z zyskiem i przy naszym
        // stosunku sygnału do szumu wygrywała — model wolał nie handlować.
        reward.k_dd = 0.0;
        reward.k_risk = 0.0;
        reward.k_hold = 0.0;
    }
    if a.k_dd >= 0.0 {
        reward.k_dd = a.k_dd;
    }
    if a.k_risk >= 0.0 {
        reward.k_risk = a.k_risk;
    }
    if a.k_hold >= 0.0 {
        reward.k_hold = a.k_hold;
    }
    if a.w_min >= 0.0 {
        reward.w_min = a.w_min.clamp(0.0, 1.0);
        reward.w_mean = 1.0 - reward.w_min;
    }

    let sched = match a.tp_mode.as_str() {
        "last" => TpSchedule::AllRunners,
        "tp1" => TpSchedule::AllAtTp1,
        "ladder" => TpSchedule::Ladder,
        other => bail!("nieznany --tp-mode '{other}' (last|tp1|ladder)"),
    };
    let mut settings = match &a.preset {
        Some(p) => {
            let txt =
                std::fs::read_to_string(p).with_context(|| format!("nie mogę wczytać {p}"))?;
            let pr: conduit_core::settings::Preset =
                serde_json::from_str(&txt).with_context(|| format!("{p} nie jest presetem"))?;
            println!("preset  : {} — {}", pr.name, pr.description);
            let mut st = pr.settings;
            // preset opisuje SILNIK; tryb AI i interwał należą do nas
            st.ai_enabled = true;
            st
        }
        None => match a.engine.as_str() {
            "deep" => deep_settings(),
            "shallow" => training_settings_with(sched),
            other => bail!("nieznany --engine '{other}' (deep|shallow)"),
        },
    };
    settings.ai_decision_interval_s = a.interval;
    let msg_offset_ms = (a.msg_offset_min * 60_000.0) as i64;
    let rc = RunCfg {
        start_balance: a.balance,
        settings,
        warmup_min: 120,
        no_sl_atr: reward.no_sl_atr,
        flat_at_end: true,
        msg_offset_ms,
    };

    let safety = SafetyCfg {
        equity_floor_pct: a.floor_pct,
        max_margin_util_pct: a.max_margin,
        max_total_lots: a.max_lots,
        ..SafetyCfg::default()
    };

    // Przerwanie i licznik ocen dla okienka postępu. Oba hooki istnieją w
    // `TrainCfg` od początku — tu je tylko podłączamy. `cancel` sprawdzane jest
    // PO zapisie punktu kontrolnego, więc „PRZERWIJ" kosztuje najwyżej jedno
    // pokolenie, a wznowienie (`--resume`) ruszy z następnego.
    let anuluj = Arc::new(AtomicBool::new(false));
    let ocen_zrobionych = Arc::new(AtomicU64::new(0));

    let tc = TrainCfg {
        cancel: Some(anuluj.clone()),
        eval_done: Some(ocen_zrobionych.clone()),
        algo: Algo::parse(&a.algo).with_context(|| format!("nieznany algorytm '{}'", a.algo))?,
        generations: a.generations,
        pop: a.pop,
        seed: a.seed,
        sigma: a.sigma,
        lr: a.lr,
        patience: a.patience,
        relative: a.relative,
        reward: reward.clone(),
        checkpoint: Some(std::path::PathBuf::from(
            a.checkpoint
                .clone()
                .unwrap_or_else(|| format!("{}.checkpoint.json", a.out)),
        )),
        resume: a.resume,
        ..TrainCfg::default()
    };

    let mut proto = match &a.load {
        Some(p) => {
            let m = Model::load(p)?;
            println!("wczytano model: {p}");
            m
        }
        None => Model::fresh(&a.hidden_pos, &a.hidden_bsk, a.seed, safety),
    };
    proto.name = a.out.clone();
    proto.actions = ActionMode::parse(&a.actions)
        .with_context(|| format!("nieznany --actions '{}'", a.actions))?;
    proto.train = TrainMeta {
        algo: tc.algo.name().into(),
        generations: tc.generations,
        pop: tc.pop,
        seed: tc.seed,
        sigma: tc.sigma,
        lr: tc.lr,
        windows: windows_label(&train_ws),
        start_balance: a.balance,
        decision_interval_s: a.interval,
        engine_tp_mode: a.tp_mode.clone(),
        split: split_desc.clone(),
        reward: reward.clone(),
    };

    println!(
        "podział : {split_desc}{}",
        if a.swap_folds {
            " [FAŁDA 2: zbiory zamienione]"
        } else {
            ""
        }
    );
    println!(
        "okna    : trening {} × {:.1} dnia, walidacja {} × {:.1} dnia (ROZŁĄCZNE, sprawdzone)",
        train_ws.len(),
        a.window_days,
        valid_ws.len(),
        valid_days
    );
    println!(
        "model   : pozycja {:?} → 11 wyjść · koszyk {:?} → 7 wyjść · {} wag",
        &proto.policy.pos.dims[1..proto.policy.pos.dims.len() - 1],
        &proto.policy.bsk.dims[1..proto.policy.bsk.dims.len() - 1],
        proto.policy.n_params()
    );
    println!(
        "akcje   : {} ({})",
        proto.actions.name(),
        match proto.actions {
            ActionMode::EntryOnly =>
                "tylko głębokość wejścia i wielkość pozycji; SL/TP/wyjścia prowadzi silnik",
            ActionMode::Full => "pełne zarządzanie pozycją, koszykiem i wejściem",
        }
    );
    println!(
        "ocena   : {}",
        if a.relative {
            "WZGLĘDEM presetu na tych samych oknach; ryzyko = ograniczenie wykonania, nie kara"
        } else {
            "bezwzględna (od zera)"
        }
    );
    println!(
        "nagroda : k_dd {:.2} · k_risk {:.2} · k_hold {:.2} $/lot/h · k_blow {:.0} · średnia/min {:.2}/{:.2}",
        reward.k_dd, reward.k_risk, reward.k_hold, reward.k_blow, reward.w_mean, reward.w_min
    );
    println!(
        "ryzyko  : podłoga equity {:.0} % · margin ≤ {:.0} % · ≤ {:.2} lota łącznie",
        a.floor_pct, a.max_margin, a.max_lots
    );
    println!("rdzenie : {}\n", rayon::current_num_threads());

    if a.six {
        raport_szesciopolowy(&td, &msgs, &rc, &proto, "RAPORT SZEŚCIOPOLOWY")?;
        return Ok(());
    }

    // ---------- okienko postępu ----------
    //
    // Trening trwa godzinami, więc bez podglądu jedyną informacją zwrotną jest
    // wiersz na pokolenie — czyli co kilkadziesiąt sekund. Licznik `eval_done`
    // pokazuje ruch WEWNĄTRZ pokolenia (para kandydat × okno), dzięki czemu
    // pasek płynie zamiast stać.
    let pop_efektywne = match tc.algo {
        Algo::Cem => a.pop.max(4),
        _ => (a.pop / 2).max(1) * 2,
    };
    // +1, bo razem z populacją oceniamy środek rozkładu
    let ocen_na_pokolenie = ((pop_efektywne + 1) * train_ws.len()) as u64;
    let pokolen_zrobionych = Arc::new(AtomicU64::new(0));
    // ostatni wiersz historii: (pokolenie, środek, najlepszy, mediana, PnL, MaxDD, transakcje, sigma)
    type Wiersz = (usize, f64, f64, f64, f64, f64, u32, f32);
    let ostatni: Arc<Mutex<Option<Wiersz>>> = Arc::new(Mutex::new(None));
    let koniec_treningu = Arc::new(AtomicBool::new(false));

    let watek_postepu = {
        let anuluj = anuluj.clone();
        let ocen = ocen_zrobionych.clone();
        let pokolenia = pokolen_zrobionych.clone();
        let ostatni = ostatni.clone();
        let koniec = koniec_treningu.clone();
        let gen_razem = a.generations as u64;
        let nazwa = format!(
            "{} · {} pokoleń × pop {} · {} okien",
            std::path::Path::new(&a.out)
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("model"),
            a.generations,
            pop_efektywne,
            train_ws.len()
        );
        let algo = tc.algo.name().to_string();
        let rdzenie = rayon::current_num_threads();
        std::thread::spawn(move || {
            let mut r = conduit_monitor::Raport::nowy(nazwa, conduit_monitor::TRENING);
            r.calosc((gen_razem * ocen_na_pokolenie) as f64, "ocen", "ocen/s");
            loop {
                let g = pokolenia.load(Ordering::Relaxed);
                let e = ocen.load(Ordering::Relaxed);
                let zrobione = (g * ocen_na_pokolenie + e.min(ocen_na_pokolenie)) as f64;
                let co = if g == 0 && e == 0 {
                    "przygotowanie: linia bazowa na oknach treningowych".to_string()
                } else {
                    format!(
                        "pokolenie {}/{gen_razem} — ocena {}/{ocen_na_pokolenie}",
                        (g + 1).min(gen_razem),
                        e.min(ocen_na_pokolenie)
                    )
                };
                // Cienki pasek: postęp BIEŻĄCEGO POKOLENIA, nie całego treningu.
                // Tu „element bieżący" jest jednoznaczny — pokolenia idą po
                // kolei, oceny wewnątrz jednego liczą się równolegle, ale samo
                // pokolenie jest jedno. Nie ma więc tego problemu, co przy
                // sweepie backtestów, gdzie równolegle biegnie kilkanaście
                // niezależnych przebiegów.
                if g == 0 && e == 0 {
                    r.biezacy(0.0, "");
                } else {
                    let w_pokoleniu = if ocen_na_pokolenie > 0 {
                        e.min(ocen_na_pokolenie) as f64 / ocen_na_pokolenie as f64
                    } else {
                        0.0
                    };
                    r.biezacy(
                        w_pokoleniu,
                        format!(
                            "pokolenie {}/{gen_razem} · {} z {ocen_na_pokolenie} ocen",
                            (g + 1).min(gen_razem),
                            e.min(ocen_na_pokolenie)
                        ),
                    );
                }
                // znak dopisujemy sami: `pl_liczba` daje minus, a przy ocenach
                // krążących wokół zera plus jest równie ważną informacją
                let zn = |x: f64, d: usize| {
                    let s = conduit_monitor::pl_liczba(x, d);
                    if x >= 0.0 {
                        format!("+{s}")
                    } else {
                        s
                    }
                };
                let mut st = conduit_monitor::Statystyki::nowe();
                st.dodaj("pokolenie", format!("{g} / {gen_razem}"));
                if let Ok(o) = ostatni.lock() {
                    if let Some((_, srodek, najl, med, pnl, dd, tr, sig)) = o.as_ref() {
                        st.dodaj("ocena środka", zn(*srodek, 4));
                        st.dodaj("ocena najlepszego", zn(*najl, 4));
                        st.dodaj("mediana populacji", zn(*med, 4));
                        st.dodaj("PnL środka", format!("{} $", zn(*pnl, 2)));
                        st.dodaj(
                            "MaxDD środka",
                            format!("{} $", conduit_monitor::pl_liczba(*dd, 2)),
                        );
                        st.dodaj("transakcje", tr.to_string());
                        st.dodaj("sigma", conduit_monitor::pl_liczba(*sig as f64, 4));
                    }
                }
                st.dodaj("algorytm", algo.clone());
                st.dodaj("rdzenie", rdzenie.to_string());
                if !r.postep(zrobione, co, st) {
                    anuluj.store(true, Ordering::Relaxed);
                }
                if koniec.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            r.zakoncz();
        })
    };

    // ---------- linia bazowa ----------
    println!("── LINIA BAZOWA (model startowy nic nie robi — sygnał gra swoim SL/TP) ──");
    let base_tr = summarize(&run_windows(&td, &msgs, &train_ws, &rc, &proto), &reward);
    let base_va = summarize(&run_windows(&td, &msgs, &valid_ws, &rc, &proto), &reward);
    print_summary("trening", &base_tr, a.balance);
    print_summary("walidacja", &base_va, a.balance);
    println!(
        "  fitness: trening {:+.4} · walidacja {:+.4}\n",
        base_tr.fitness, base_va.fitness
    );

    // ---------- trening ----------
    println!(
        "── EWOLUCJA ({}, pop {}, {} pokoleń) ──",
        tc.algo.name(),
        tc.pop,
        tc.generations
    );
    println!(
        "{:>4} {:>9} {:>9} {:>9} {:>9} {:>10} {:>9} {:>7} {:>6}",
        "pok", "środek", "najlepszy", "mediana", "najgorszy", "PnL $", "MaxDD $", "trans", "s"
    );
    let t0 = std::time::Instant::now();
    let (best, _reports) = train_with(
        &td,
        &msgs,
        &train_ws,
        &rc,
        &proto,
        &tc,
        |r| {
            println!(
                "{:>4} {:>+9.4} {:>+9.4} {:>+9.4} {:>+9.4} {:>10.2} {:>9.2} {:>7} {:>6.1}",
                r.gen,
                r.center,
                r.best,
                r.median,
                r.worst,
                r.center_summary.pnl,
                r.center_summary.max_dd_abs,
                r.center_summary.trades,
                r.elapsed_s
            );
            // to samo do okienka postępu
            pokolen_zrobionych.store(r.gen as u64 + 1, Ordering::Relaxed);
            if let Ok(mut o) = ostatni.lock() {
                *o = Some((
                    r.gen,
                    r.center,
                    r.best,
                    r.median,
                    r.center_summary.pnl,
                    r.center_summary.max_dd_abs,
                    r.center_summary.trades,
                    r.sigma,
                ));
            }
        },
        |gen, best| {
            println!("  wznowiono z punktu kontrolnego: pokolenie {gen}, najlepszy {best:+.4}");
            // pasek ma ruszyć od miejsca wznowienia, a nie od zera
            pokolen_zrobionych.store(gen as u64, Ordering::Relaxed);
        },
        |why| println!("  START OD ZERA — {why}"),
    );
    koniec_treningu.store(true, Ordering::Relaxed);
    let _ = watek_postepu.join();
    println!(
        "\nczas treningu: {:.1} min",
        t0.elapsed().as_secs_f64() / 60.0
    );
    if anuluj.load(Ordering::Relaxed) {
        println!(
            "\n╔══ PRZERWANO NA ŻĄDANIE ══\n\
             ║ punkt kontrolny jest zapisany — `--resume` wznowi z następnego pokolenia\n\
             ║ poniżej ocena NAJLEPSZEGO dotychczasowego modelu; zostanie zapisany normalnie\n\
             ╚══════════════════════════"
        );
    }

    // ---------- ocena końcowa ----------
    let proto_start = proto.clone();
    println!("\n── WYTRENOWANY MODEL ──");
    let fin_tr = summarize(&run_windows(&td, &msgs, &train_ws, &rc, &best), &reward);
    let outs_va = run_windows(&td, &msgs, &valid_ws, &rc, &best);
    let base_outs_va = run_windows(&td, &msgs, &valid_ws, &rc, &proto_start);
    let fin_va = summarize(&outs_va, &reward);
    print_summary("trening", &fin_tr, a.balance);
    print_summary("walidacja", &fin_va, a.balance);
    println!(
        "  fitness: trening {:+.4} · walidacja {:+.4}",
        fin_tr.fitness, fin_va.fitness
    );

    println!("\n── ZMIANA WZGLĘDEM LINII BAZOWEJ ──");
    println!(
        "  trening  : PnL {:+.2} → {:+.2} $ ({:+.2}) · MaxDD {:.2} → {:.2} $ · fitness {:+.4} → {:+.4}",
        base_tr.pnl, fin_tr.pnl, fin_tr.pnl - base_tr.pnl, base_tr.max_dd_abs, fin_tr.max_dd_abs,
        base_tr.fitness, fin_tr.fitness
    );
    println!(
        "  walidacja: PnL {:+.2} → {:+.2} $ ({:+.2}) · MaxDD {:.2} → {:.2} $ · fitness {:+.4} → {:+.4}",
        base_va.pnl, fin_va.pnl, fin_va.pnl - base_va.pnl, base_va.max_dd_abs, fin_va.max_dd_abs,
        base_va.fitness, fin_va.fitness
    );
    if fin_va.fitness < base_va.fitness {
        println!("\n  UWAGA: model jest na walidacji GORSZY od linii bazowej — to przeuczenie,");
        println!("  a nie wynik. Zwiększ liczbę/długość okien albo osłab krok uczenia.");
    }

    // ---------- ROZBICIE WG REŻIMU ----------
    //
    // Kanał zmienił charakter: udział zleceń LIMIT to 7 % w kwietniu i 66 % w
    // lipcu, znacznik FIRST ENTRY 2 % → 36 %, edycje wiadomości 17 % → 44 %.
    // Wynik uśredniony po całym zakresie miesza dwa różne rynki, a model, który
    // wygrywa tylko na kwietniu, jest bezwartościowy.
    let granica = parse_date(&a.regime_split)?;
    let stary: Vec<usize> = (0..valid_ws.len())
        .filter(|i| valid_ws[*i].from < granica)
        .collect();
    let nowy: Vec<usize> = (0..valid_ws.len())
        .filter(|i| valid_ws[*i].from >= granica)
        .collect();
    println!(
        "\n── WYNIK WG REŻIMU (tylko walidacja, granica {}) ──",
        day_label(granica)
    );
    for (nazwa, idx) in [("kwiecień+maj", &stary), ("czerwiec+lipiec", &nowy)] {
        if idx.is_empty() {
            println!("  {nazwa:<16} brak okien po tej stronie granicy");
            continue;
        }
        let m: Vec<_> = idx.iter().map(|i| outs_va[*i].clone()).collect();
        let bl: Vec<_> = idx.iter().map(|i| base_outs_va[*i].clone()).collect();
        let sm = summarize(&m, &reward);
        let sb = summarize(&bl, &reward);
        let (clo, chi) = bootstrap_ci(&sm.window_pnl, 20_000, 4242);
        println!(
            "  {nazwa:<16} model {:+8.2} $  CI [{:+8.2}, {:+8.2}]   baza {:+8.2} $   ({} okien, {} trans, PF {})",
            sm.pnl,
            clo,
            chi,
            sb.pnl,
            sm.windows,
            sm.trades,
            fmt_pf(sm.profit_factor)
        );
    }

    let s_from = parse_date(&a.stress_from)?;
    let s_to = parse_date(&a.stress_to)?;
    let stress = vec![Window {
        from: s_from,
        to: s_to,
        label: day_label(s_from),
    }];
    let sm = summarize(&run_windows(&td, &msgs, &stress, &rc, &best), &reward);
    let sb = summarize(
        &run_windows(&td, &msgs, &stress, &rc, &proto_start),
        &reward,
    );
    println!(
        "
── NAJGORSZE DNI ({} … {}) ──",
        day_label(s_from),
        day_label(s_to)
    );
    println!(
        "  model  {:+8.2} $ ({:+6.2} % kapitału)  MaxDD {:6.2} $  {:3} trans  PF {}",
        sm.pnl,
        sm.pnl / a.balance * 100.0,
        sm.max_dd_abs,
        sm.trades,
        fmt_pf(sm.profit_factor)
    );
    println!(
        "  baza   {:+8.2} $ ({:+6.2} % kapitału)  MaxDD {:6.2} $  {:3} trans  PF {}   → różnica {:+.2} $",
        sb.pnl,
        sb.pnl / a.balance * 100.0,
        sb.max_dd_abs,
        sb.trades,
        fmt_pf(sb.profit_factor),
        sm.pnl - sb.pnl
    );

    // ---------- ocena POZA PRÓBĄ: przedział ufności i ryzyko ruiny ----------
    println!(
        "
── ISTOTNOŚĆ (wyłącznie walidacja, {} rozłącznych okien) ──",
        fin_va.windows
    );
    let (lo95, hi95) = bootstrap_ci(&fin_va.window_pnl, 20_000, 12345);
    let (blo, bhi) = bootstrap_ci(&base_va.window_pnl, 20_000, 12345);
    println!(
        "  model         PnL {:+8.2} $   95 % CI [{:+8.2}, {:+8.2}]   {}",
        fin_va.pnl,
        lo95,
        hi95,
        if lo95 > 0.0 {
            "przewaga ISTOTNA"
        } else {
            "przewaga NIEUSTALONA (przedział obejmuje zero)"
        }
    );
    println!(
        "  linia bazowa  PnL {:+8.2} $   95 % CI [{:+8.2}, {:+8.2}]",
        base_va.pnl, blo, bhi
    );
    let n = fin_va.window_pnl.len().max(1) as f64;
    let mean = fin_va.pnl / n;
    let var = fin_va
        .window_pnl
        .iter()
        .map(|x| (x - mean).powi(2))
        .sum::<f64>()
        / n.max(2.0);
    println!(
        "  na okno: średnia {:+.2} $, odch. std {:.2} $, dni dodatnie {}/{}",
        mean,
        var.sqrt(),
        fin_va.positive_windows,
        fin_va.windows
    );

    if !fin_va.profits.is_empty() {
        println!(
            "
── RYZYKO RUINY (bootstrap na {} transakcjach z walidacji) ──",
            fin_va.profits.len()
        );
        println!(
            "  ruina = zejście equity do podłogi {:.0} % kapitału startowego",
            a.floor_pct
        );
        let kapitaly = [200.0, 300.0, 500.0, 800.0, 1200.0, 2000.0, 3000.0, 5000.0];
        for c in kapitaly {
            let r = ruin_probability(&fin_va.profits, c, a.floor_pct, 20_000, 999);
            println!(
                "    {c:>7.0} $ → {:>6.2} %{}",
                r * 100.0,
                if r < 0.05 { "  ← poniżej 5 %" } else { "" }
            );
        }
        match min_capital_for(&fin_va.profits, &kapitaly, 0.05, a.floor_pct, 999) {
            Some((c, r)) => println!(
                "  minimalny kapitał dla ryzyka ruiny < 5 %: {c:.0} $ (zmierzone {:.2} %)",
                r * 100.0
            ),
            None => {
                println!("  UWAGA: żaden z badanych kapitałów (do 5000 $) nie schodzi poniżej 5 %")
            }
        }
        println!("  (bootstrap losuje transakcje ze zwracaniem — gubi autokorelację serii strat,");
        println!("   więc te liczby są OPTYMISTYCZNE; realne serie bywają dłuższe niż losowe)");
    }

    // ---------- zapis ----------
    let mut out = best;
    out.created = chrono::Utc::now().to_rfc3339();
    out.score.fitness = fin_va.fitness;
    out.score.pnl = fin_va.pnl;
    out.score.max_dd = fin_va.max_dd_abs;
    out.score.profit_factor = fin_va.profit_factor;
    out.score.trades = fin_va.trades;
    out.score.win_rate = fin_va.win_rate;
    let base_desc = if a.load.is_some() {
        "punkt wyjścia"
    } else {
        "linia bazowa"
    };
    out.score.note = format!(
        "walidacja {} … {}: PnL {:+.2} $, MaxDD {:.2} $, maxRyzyko {:.2} $, PF {}; \
         trening: PnL {:+.2} $, MaxDD {:.2} $; {base_desc} (walidacja): PnL {:+.2} $",
        day_label(v_from),
        day_label(v_to),
        fin_va.pnl,
        fin_va.max_dd_abs,
        fin_va.max_open_risk,
        fmt_pf(fin_va.profit_factor),
        fin_tr.pnl,
        fin_tr.max_dd_abs,
        base_va.pnl
    );
    out.save(&a.out)?;
    println!("\nzapisano: {}", a.out);
    Ok(())
}
