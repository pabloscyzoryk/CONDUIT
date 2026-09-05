//! Zadanie: BACKTEST (pojedynczy preset, przemiał katalogu, walk-forward).
//!
//! Ta sama ścieżka kodu, co CLI `bt.exe` — `conduit_backtest::runner::run`.
//! Różnica jest jedna: zamiast wypisywać tabelę na końcu, meldujemy postęp
//! po drodze i pozwalamy się przerwać.
//!
//! **Zrównoleglenie i postęp.** Przebiegi lecą przez rayona (tak samo jak
//! w CLI — 40 presetów na 24 rdzeniach to 40 niezależnych backtestów). Postęp
//! liczymy w TICKACH, nie w przebiegach: przebieg trwa dziesiątki sekund,
//! więc pasek oparty na przebiegach stałby nieruchomo, a potem skakał.
//! Ticki są wspólnym mianownikiem także dla walk-forward, gdzie okna mają
//! różne długości.

use super::{
    bezpieczna_nazwa, dzien, parse_dzien, zapisz_json, JobCtx, LabCell, LabMonth, LabQuad, LabRow,
    Tryb, PODLOGA_RUINY_PCT, TRYBY,
};
use crate::state::StateHandle;
use conduit_backtest::chart;
use conduit_backtest::data::{load_messages, TickData};
use conduit_backtest::metrics::{DayStat, Metrics};
use conduit_backtest::runner::{run_with_progress, RunConfig, RunResult};
use conduit_core::settings::{Preset, Settings};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================
//  ZLECENIE
// ============================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestReq {
    /// `all` | `last-month` | `last-week` | `last-N-days` | `range`
    #[serde(default = "domyslny_okres")]
    pub period: String,
    /// używane tylko przy `period = "range"`
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    /// NAZWA katalogu presetów z listy `/api/lab/info` (nie ścieżka!)
    #[serde(default)]
    pub preset_dir: String,
    /// pojedynczy preset z tego katalogu; puste = cały katalog (przemiał)
    #[serde(default)]
    pub preset: String,
    #[serde(default = "domyslny_kapital")]
    pub balance: f64,
    #[serde(default)]
    pub daily_reset: bool,
    /// długość okna walidacji poza próbą w dniach; 0 = bez walk-forward
    #[serde(default)]
    pub walk_forward: i64,
    #[serde(default)]
    pub four_modes: bool,
}

fn domyslny_okres() -> String {
    "last-month".into()
}
fn domyslny_kapital() -> f64 {
    200.0
}

impl Default for BacktestReq {
    fn default() -> Self {
        BacktestReq {
            period: domyslny_okres(),
            from: String::new(),
            to: String::new(),
            preset_dir: String::new(),
            preset: String::new(),
            balance: domyslny_kapital(),
            daily_reset: false,
            walk_forward: 0,
            four_modes: false,
        }
    }
}

/// Tryby, w których policzymy każdy preset.
///
/// Bez oceny czterotrybowej jest to JEDEN tryb-atrapa: `compounding: None`
/// znaczy „nie dotykaj lota presetu". To rozróżnienie jest istotne — podmiana
/// lota nawet na „równoważną" wartość zmieniłaby wyniki wszystkich dotychczasowych
/// przebiegów.
#[derive(Debug, Clone, Copy)]
pub struct Wariant {
    pub key: &'static str,
    pub label: &'static str,
    pub daily_reset: bool,
    /// `None` = zostaw ustawienia lota presetu nietknięte
    pub compounding: Option<bool>,
}

impl Wariant {
    fn z_trybu(t: &Tryb) -> Wariant {
        Wariant {
            key: t.key,
            label: t.label,
            daily_reset: t.daily_reset,
            compounding: Some(t.compounding),
        }
    }
}

pub fn warianty(req: &BacktestReq) -> Vec<Wariant> {
    if req.four_modes {
        return TRYBY.iter().map(Wariant::z_trybu).collect();
    }
    vec![Wariant {
        key: if req.daily_reset { "daily" } else { "compound" },
        label: if req.daily_reset {
            "każdy dzień osobno"
        } else {
            "compounding"
        },
        daily_reset: req.daily_reset,
        compounding: None,
    }]
}

/// Okno czasowe ze zlecenia. Wydzielone, bo testuje się bez plików z danymi.
pub fn okno(req: &BacktestReq, first_ts: i64, last_ts: i64) -> anyhow::Result<(i64, i64)> {
    let dzien_ms = 86_400_000i64;
    let koniec = last_ts + 1;
    let (from, to) = match req.period.as_str() {
        "all" => (first_ts, koniec),
        "last-month" => (koniec - 30 * dzien_ms, koniec),
        "last-week" => (koniec - 7 * dzien_ms, koniec),
        "range" => {
            let a = parse_dzien(&req.from)?;
            let b = if req.to.trim().is_empty() {
                koniec
            } else {
                parse_dzien(&req.to)? + dzien_ms
            };
            (a, b)
        }
        p if p.starts_with("last-") && p.ends_with("-days") => {
            let n: i64 = p
                .trim_start_matches("last-")
                .trim_end_matches("-days")
                .parse()?;
            (koniec - n.max(1) * dzien_ms, koniec)
        }
        inne => anyhow::bail!("nieznany okres „{inne}”"),
    };
    let from = from.max(first_ts);
    if to <= from {
        anyhow::bail!("puste okno czasowe: {} … {}", dzien(from), dzien(to));
    }
    Ok((from, to))
}

// ============================================================
//  OCENA (ta sama reguła co w CLI)
// ============================================================

/// Czy przebieg doprowadził konto do ruiny.
///
/// Ruina to zero ALBO poziom, z którego minimalny lot już nie odrobi
/// ([`PODLOGA_RUINY_PCT`]). Mierzymy po `min_equity`, czyli po najniższym
/// equity NA CAŁEJ ŚCIEŻCE, z otwartymi pozycjami — a nie po saldzie na koniec
/// dnia. Konto, które chwilowo spadło do 12 $, nie „przeżyło, bo się odbiło":
/// przy takim equity broker już domyka pozycje, a minimalny lot nie ma czym
/// odrobić.
pub fn ruina(m: &Metrics) -> bool {
    m.blown || m.min_equity <= m.start_balance * PODLOGA_RUINY_PCT / 100.0
}

pub fn score(m: &Metrics) -> f64 {
    if ruina(m) {
        // wciąż monotonicznie po zysku, żeby „mniej katastrofalny" był niżej,
        // ale zawsze pod każdym wariantem, który konta nie zabił
        return -1e9 + m.total_profit.clamp(-1e6, 1e6) * 1e-6;
    }
    m.total_profit
}

/// Porządek tabeli: najpierw przebiegi POLICZONE DO KOŃCA (malejąco po ocenie),
/// dopiero za nimi cząstkowe. Przebieg przerwany po tygodniu potrafi mieć
/// świetną ocenę wyłącznie dlatego, że nie zdążył zobaczyć złego reżimu —
/// mieszanie go z pełnymi w jednym rankingu zamienia tabelę w pułapkę.
fn porownaj(a: &LabRow, b: &LabRow) -> std::cmp::Ordering {
    a.partial.cmp(&b.partial).then(
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal),
    )
}

// ============================================================
//  KOMÓRKA OCENY CZTEROTRYBOWEJ
// ============================================================

/// Miesiące osobno — §4 wymaga ich wprost, bo średnia z całości ukrywa reżim.
fn miesiace(daily: &[DayStat]) -> Vec<LabMonth> {
    let mut out: Vec<LabMonth> = Vec::new();
    for d in daily {
        if d.trades == 0 {
            continue;
        }
        let m = d.date.get(..7).unwrap_or(&d.date).to_string();
        match out.last_mut() {
            Some(x) if x.month == m => {
                x.profit += d.profit;
                x.days += 1;
                if d.profit < 0.0 {
                    x.loss_days += 1;
                }
            }
            _ => out.push(LabMonth {
                month: m,
                profit: d.profit,
                days: 1,
                loss_days: u32::from(d.profit < 0.0),
            }),
        }
    }
    out
}

/// Ile razy konto spadło do poziomu ruiny.
///
/// Liczymy INACZEJ w obu trybach i to nie jest niekonsekwencja:
///  * „dzień po dniu" — każda doba to OSOBNE konto od nowa, więc każdy dzień
///    zakończony w ruinie jest osobnym wyzerowaniem;
///  * „długoterminowo" — konto jest jedno i ciągłe, więc liczymy WEJŚCIA
///    w ruinę. Bez tego jeden upadek na 40 $ dawałby tyle „wyzerowań", ile
///    zostało dni do końca okna.
fn wyzerowania(daily: &[DayStat], m: &Metrics, daily_reset: bool, balance: f64) -> u32 {
    let prog = balance * PODLOGA_RUINY_PCT / 100.0;
    let n = if daily_reset {
        daily.iter().filter(|d| d.end_equity <= prog).count() as u32
    } else {
        let mut n = 0u32;
        let mut byla = false;
        for d in daily {
            let teraz = d.end_equity <= prog;
            if teraz && !byla {
                n += 1;
            }
            byla = teraz;
        }
        n
    };
    if (m.blown || m.min_equity <= prog) && n == 0 {
        1
    } else {
        n
    }
}

fn komorka(
    w: &Wariant,
    m: &Metrics,
    daily: &[DayStat],
    ostatni_dzien_okna: &str,
    chart: &str,
    partial: bool,
    balance: f64,
) -> LabCell {
    let aktywne: Vec<&DayStat> = daily.iter().filter(|d| d.trades > 0).collect();
    let stratne = aktywne.iter().filter(|d| d.profit < 0.0).count();
    let cut_short = m.blown
        && daily
            .last()
            .map(|d| d.date.as_str() < ostatni_dzien_okna)
            .unwrap_or(false);
    LabCell {
        mode: w.key.to_string(),
        profit: m.total_profit,
        loss_days_pct: if aktywne.is_empty() {
            0.0
        } else {
            stratne as f64 / aktywne.len() as f64 * 100.0
        },
        worst_day: m.worst_day,
        min_equity: m.min_equity,
        ruins: wyzerowania(daily, m, w.daily_reset, balance),
        blown: m.blown,
        units: m.trades,
        known_entry_sources: (m.entry_source_observation_version >= 1).then_some(m.known_entry_sources),
        known_full_entry_sources: (m.entry_source_observation_version >= 1).then_some(m.known_full_entry_sources),
        entry_sources_first_seen_as_edit: (m.entry_source_observation_version >= 1).then_some(m.entry_sources_first_seen_as_edit),
        max_dd: m.max_dd_abs,
        max_open_risk_pct: m.max_open_risk_pct,
        end_equity: m.end_equity,
        trading_days: m.trading_days,
        months: miesiace(daily),
        partial,
        cut_short,
        chart: chart.to_string(),
    }
}

fn wiersz(name: &str, m: &Metrics, chart: &str, partial: bool) -> LabRow {
    LabRow {
        name: name.to_string(),
        profit: m.total_profit,
        per_day: m.avg_per_day,
        max_dd: m.max_dd_abs,
        max_dd_pct: m.max_dd_pct,
        risk: m.max_open_risk,
        risk_pct: m.max_open_risk_pct,
        profit_factor: if m.profit_factor.is_finite() {
            Some(m.profit_factor)
        } else {
            None
        },
        win_days_pct: m.win_days_pct,
        win_rate: m.win_rate,
        trades: m.trades,
        known_entry_sources: (m.entry_source_observation_version >= 1).then_some(m.known_entry_sources),
        known_full_entry_sources: (m.entry_source_observation_version >= 1).then_some(m.known_full_entry_sources),
        entry_sources_first_seen_as_edit: (m.entry_source_observation_version >= 1).then_some(m.entry_sources_first_seen_as_edit),
        max_open_positions: m.max_open_positions,
        blown: m.blown,
        score: score(m),
        chart: chart.to_string(),
        partial,
    }
}

// ============================================================
//  URUCHOMIENIE
// ============================================================

fn wczytaj_presety(dir: &Path, tylko: &str) -> anyhow::Result<Vec<Preset>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        if !tylko.is_empty() && p.file_stem().and_then(|x| x.to_str()) != Some(tylko) {
            continue;
        }
        let txt = std::fs::read_to_string(&p)?;
        match serde_json::from_str::<Preset>(&txt) {
            Ok(pr) => out.push(pr),
            // jeden zepsuty plik nie może zablokować całego przemiału
            Err(err) => tracing::warn!(plik = %p.display(), blad = %err, "pomijam preset"),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Publiczne wejście: startuje zadanie i wraca z jego identyfikatorem.
pub fn start(st: &StateHandle, req: BacktestReq) -> anyhow::Result<String> {
    // wszystko, co da się sprawdzić PRZED uruchomieniem wątku, sprawdzamy
    // teraz — żeby błąd konfiguracji wrócił jako odpowiedź HTTP, a nie jako
    // zadanie, które zaraz padnie
    let (ticks_path, signals_path) = super::data_paths(&st.workspace);
    if !ticks_path.is_file() {
        anyhow::bail!("{}", super::gdzie_szukalem(&st.workspace));
    }
    if !signals_path.is_file() {
        anyhow::bail!("nie znalazłem pliku sygnałów: {}", signals_path.display());
    }

    let presety = if req.preset_dir.trim().is_empty() {
        // bez wskazanego katalogu liczymy same ustawienia domyślne
        vec![Preset {
            name: "domyślne".into(),
            description: String::new(),
            format: "ATFX".into(),
            settings: Settings::default(),
            ea: None,
        }]
    } else {
        let d = super::resolve_preset_dir(&st.workspace, &req.preset_dir)?;
        let p = wczytaj_presety(&d, req.preset.trim())?;
        if p.is_empty() {
            anyhow::bail!(
                "w katalogu „{}” nie ma presetu „{}”",
                req.preset_dir,
                if req.preset.is_empty() {
                    "(żadnego)"
                } else {
                    &req.preset
                }
            );
        }
        p
    };

    let tytul = format!(
        "{} · {} · kapitał {:.0} $ · {}",
        if presety.len() == 1 {
            presety[0].name.clone()
        } else {
            format!("przemiał {} presetów", presety.len())
        },
        opis_okresu(&req),
        req.balance,
        if req.four_modes {
            "OCENA CZTEROTRYBOWA".to_string()
        } else if req.daily_reset {
            "każdy dzień osobno".to_string()
        } else {
            "compounding".to_string()
        }
    );

    super::spawn_job(st, "backtest", tytul, move |ctx| licz(ctx, req, presety))
}

fn opis_okresu(req: &BacktestReq) -> String {
    match req.period.as_str() {
        "all" => "całość danych".into(),
        "last-month" => "ostatni miesiąc".into(),
        "last-week" => "ostatni tydzień".into(),
        "range" => format!(
            "{} … {}",
            req.from,
            if req.to.is_empty() { "koniec" } else { &req.to }
        ),
        p => p.to_string(),
    }
}

/// Granica między diagnostycznym RunResult a publikowanymi metrykami Lab.
/// HOLD nie jest anulowaniem użytkownika ani zerowym wynikiem. Pierwszy błąd
/// zatrzymuje współbieżne przebiegi przez callback, ale kończy job jako FAILED.
#[derive(Default)]
struct LabRunGate {
    inflight: parking_lot::Mutex<Vec<String>>,
    runs_done: AtomicU64,
    hold: parking_lot::Mutex<Option<String>>,
}

impl LabRunGate {
    fn held(&self) -> bool {
        self.hold.lock().is_some()
    }

    fn begin(&self, name: &str) -> anyhow::Result<()> {
        let hold = self.hold.lock();
        if let Some(reason) = hold.as_ref() {
            anyhow::bail!("{reason}");
        }
        self.inflight.lock().push(name.to_string());
        drop(hold);
        Ok(())
    }

    fn complete(&self, name: &str, stage: &str, result: RunResult) -> anyhow::Result<RunResult> {
        let mut inflight = self.inflight.lock();
        if let Some(i) = inflight.iter().position(|x| x == name) {
            inflight.remove(i);
        }
        drop(inflight);
        self.runs_done.fetch_add(1, Ordering::Relaxed);

        let mut hold = self.hold.lock();
        if let Some((kind, reason)) = result.reconciliation_hold() {
            hold.get_or_insert_with(|| format!(
                "HOLD {kind} · {stage} · preset {name}: {reason}. \
                 Wynik nie jest kompletny; nie opublikowano go ani nie dopuszczono do rankingu."
            ));
        }
        if let Some(reason) = hold.as_ref() {
            anyhow::bail!("{reason}");
        }
        Ok(result)
    }
}

/// Wspólny licznik postępu dla wszystkich równoległych przebiegów.
struct Postep<'a> {
    ctx: &'a JobCtx,
    ticks_done: AtomicU64,
    ticks_total: u64,
    runs_total: u64,
    results: LabRunGate,
    faza: parking_lot::Mutex<String>,
    /// ile ticków było zrobione, gdy zaczynał się bieżący tryb
    baza_trybu: AtomicU64,
    /// ile ticków przypada na jeden tryb (0 = nie dzielimy na tryby)
    na_tryb: AtomicU64,
    /// podpis cienkiego paska w okienku postępu
    podpis_trybu: parking_lot::Mutex<String>,
    /// ile przebiegów doprowadziło konto do ruiny — licznik do okienka
    zerowe: AtomicU64,
}

impl Postep<'_> {
    /// Wywoływane z wielu wątków rayona. `false` = przerwij przebieg.
    fn tick(&self, delta: u64) -> bool {
        let done = self.ticks_done.fetch_add(delta, Ordering::Relaxed) + delta;
        if self.ctx.cancelled() || self.results.held() {
            return false;
        }
        // składanie napisów tylko wtedy, gdy naprawdę pójdą do interfejsu
        if self.ctx.due() {
            self.opublikuj(done, false);
        }
        true
    }

    fn opublikuj(&self, ticks: u64, force: bool) {
        let elapsed = self.ctx.elapsed_ms().max(1) as f64 / 1000.0;
        let tps = ticks as f64 / elapsed;
        let runs = self.results.runs_done.load(Ordering::Relaxed);
        let rpm = runs as f64 / elapsed * 60.0;
        let biezace = {
            let g = self.results.inflight.lock();
            if g.is_empty() {
                String::new()
            } else if g.len() == 1 {
                format!(" · preset {}", g[0])
            } else {
                format!(" · {} i {} inne", g[0], g.len() - 1)
            }
        };
        let faza = self.faza.lock().clone();
        let progress = (ticks as f64 / self.ticks_total.max(1) as f64).clamp(0.0, 1.0);

        // --- cienki pasek okienka: postęp BIEŻĄCEGO TRYBU ---
        // Przy ocenie czterotrybowej to jedyna liczba, którą da się uczciwie
        // podpisać: przebiegi lecą po kilkanaście naraz, więc „postęp presetu"
        // byłby wymysłem, a „postęp trybu 2/4" jest faktem.
        let na_tryb = self.na_tryb.load(Ordering::Relaxed);
        if na_tryb > 0 {
            let baza = self.baza_trybu.load(Ordering::Relaxed);
            let u = (ticks.saturating_sub(baza)) as f64 / na_tryb as f64;
            self.ctx.biezacy_okna(u, self.podpis_trybu.lock().clone());
        }

        self.ctx.edit(force, |j| {
            j.progress = progress;
            j.done = runs;
            j.total = self.runs_total;
            j.label = format!(
                "{faza} {}/{}{biezace}",
                (runs + 1).min(self.runs_total.max(1)),
                self.runs_total
            );
            j.speed = format!("{:.1} mln ticków/s", tps / 1e6);
            j.speed2 = if rpm >= 1.0 {
                format!("{rpm:.1} przebiegu/min")
            } else {
                format!(
                    "{:.1} min/przebieg",
                    if rpm > 0.0 { 1.0 / rpm } else { 0.0 }
                )
            };
        });
    }
}

fn licz(ctx: &JobCtx, req: BacktestReq, presety: Vec<Preset>) -> anyhow::Result<String> {
    let (ticks_path, signals_path) = super::data_paths(ctx.workspace());

    ctx.edit(true, |j| j.label = "wczytywanie ticków…".into());
    let ticks = TickData::open(&ticks_path)?;
    ctx.edit(true, |j| j.label = "wczytywanie sygnałów…".into());
    let messages = load_messages(&signals_path)?;

    let (from, to) = okno(&req, ticks.first_ts(), ticks.last_ts())?;
    let i0 = ticks.index_at(from);
    let i1 = ticks.index_at(to).min(ticks.len());
    let ticks_okna = (i1.saturating_sub(i0)) as u64;

    let tryby = warianty(&req);

    // --- ile ticków przemielimy łącznie (dokładnie, nie w przybliżeniu) ---
    let na_tryb = ticks_okna * presety.len() as u64;
    let mut plan_ticks = na_tryb * tryby.len() as u64;
    let mut wf_okna: Vec<(i64, i64, i64, i64)> = Vec::new();
    // Walk-forward liczymy tylko dla przemiału JEDNOTRYBOWEGO. Przy ocenie
    // czterotrybowej „najlepszy na oknie uczenia" nie ma jednoznacznego
    // znaczenia — a wybieranie zwycięzcy w jednym trybie i egzaminowanie go
    // w innym byłoby po prostu innym eksperymentem niż ten, o który proszono.
    if req.walk_forward > 0 && presety.len() > 1 && !req.four_modes {
        let win = req.walk_forward * 86_400_000;
        let mut t = from;
        while t + 2 * win <= to {
            wf_okna.push((t, t + win, t + win, t + 2 * win));
            t += win;
        }
        for (ta, tb, va, vb) in &wf_okna {
            let uczenie = (ticks.index_at(*tb) - ticks.index_at(*ta)) as u64;
            let egzamin = (ticks.index_at(*vb) - ticks.index_at(*va)) as u64;
            plan_ticks += uczenie * presety.len() as u64 + egzamin;
        }
    }
    let runs_total = presety.len() as u64 * tryby.len() as u64
        + wf_okna.len() as u64 * (presety.len() as u64 + 1);

    let tag = if req.four_modes {
        "ocena4"
    } else if req.daily_reset {
        "daily"
    } else {
        "compound"
    };
    let naglowek = format!("{} … {} · {tag}", dzien(from), dzien(to));
    let ostatni_dzien = dzien(to - 1);
    ctx.edit(true, |j| {
        j.total = runs_total;
        j.label = format!(
            "{} przebiegów × {} mln ticków",
            runs_total,
            ticks_okna / 1_000_000
        );
    });

    // --- osobne okienko postępu (LAB\postep.exe) ---
    ctx.skala_okna(plan_ticks.max(1) as f64, "ticków", "ticków/s");
    let staty_bazowe: Vec<(String, String)> = vec![
        (
            "okres".into(),
            format!("{} … {}", dzien(from), ostatni_dzien.clone()),
        ),
        ("kapitał startowy".into(), format!("{:.0} $", req.balance)),
        (
            "tryb".into(),
            if req.four_modes {
                "ocena czterotrybowa (4 przebiegi na preset)".into()
            } else {
                tryby[0].label.to_string()
            },
        ),
        ("presetów".into(), presety.len().to_string()),
    ];
    ctx.staty_okna(staty_bazowe.clone());

    let p = Postep {
        ctx,
        ticks_done: AtomicU64::new(0),
        ticks_total: plan_ticks.max(1),
        runs_total,
        results: LabRunGate::default(),
        faza: parking_lot::Mutex::new("przebieg".into()),
        baza_trybu: AtomicU64::new(0),
        na_tryb: AtomicU64::new(if tryby.len() > 1 { na_tryb } else { 0 }),
        podpis_trybu: parking_lot::Mutex::new(String::new()),
        zerowe: AtomicU64::new(0),
    };

    // ================= PRZEMIAŁ =================
    //
    // Tryby idą PO KOLEI, presety równolegle w środku każdego. Odwrotna
    // kolejność (jedna wielka pula par preset×tryb) obciążyłaby rdzenie
    // odrobinę równiej, ale wtedy nie da się powiedzieć, co się właściwie
    // liczy: cienki pasek okienka i etykieta „tryb 2/4" opisywałyby wtedy
    // mieszankę czterech różnych eksperymentów.
    let mut wyniki: Vec<(String, LabRow, Metrics)> = Vec::new();
    let mut komorki: BTreeMap<String, Vec<LabCell>> = BTreeMap::new();

    for (mi, w) in tryby.iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        p.baza_trybu
            .store(p.ticks_done.load(Ordering::Relaxed), Ordering::Relaxed);
        if tryby.len() > 1 {
            *p.faza.lock() = format!("tryb {}/{} · przebieg", mi + 1, tryby.len());
            *p.podpis_trybu.lock() = format!("tryb {}/{} · {}", mi + 1, tryby.len(), w.label);
        }

        let partia: Vec<(String, LabRow, Metrics, LabCell)> = presety
            .par_iter()
            .map(|pr| {
                p.results.begin(&pr.name)?;
                // Podmiana lota TYLKO przy ocenie czterotrybowej. Przy zwykłym
                // backteście `compounding == None` i ustawienia presetu idą
                // do silnika nietknięte — ta ścieżka ma zostać co do bitu taka
                // sama jak przed dodaniem czterech trybów.
                let mut settings = pr.settings.clone();
                if let Some(c) = w.compounding {
                    super::ustaw_compounding(&mut settings, c, req.balance);
                }
                let cfg = RunConfig {
                    from,
                    to,
                    start_balance: req.balance,
                    settings,
                    formaty: Vec::new(),
                    pulapy: Default::default(),
                    daily_reset: w.daily_reset,
                    source_name: "ATFX VIP SIGNALS".into(),
                    curve_interval_ms: 300_000,
                    journal_path: None,
                    ..Default::default()
                };
                let cb = |d: u64| p.tick(d);
                let r = run_with_progress(&ticks, &messages, &cfg, Some(&cb));
                let r = p.results.complete(&pr.name, w.label, r)?;

                // wykres powstaje od razu po przebiegu — krzywa equity potrafi mieć
                // kilkadziesiąt tysięcy punktów i nie ma powodu trzymać jej dłużej
                let safe = bezpieczna_nazwa(&pr.name);
                let plik = format!("{safe}_{}.svg", w.key);
                if !r.cancelled {
                    let svg = chart::run_chart(
                        &format!("{} · {} · {naglowek}", pr.name, w.label),
                        &r.metrics,
                        &r.equity_curve,
                        &r.daily,
                    );
                    let _ = std::fs::write(ctx.out_dir.join(&plik), svg);
                }

                if ruina(&r.metrics) {
                    p.zerowe.fetch_add(1, Ordering::Relaxed);
                }
                let chart_name = if r.cancelled { "" } else { plik.as_str() };
                let row = wiersz(&pr.name, &r.metrics, chart_name, r.cancelled);
                let cell = komorka(
                    w,
                    &r.metrics,
                    &r.daily,
                    &ostatni_dzien,
                    chart_name,
                    r.cancelled,
                    req.balance,
                );
                p.opublikuj(p.ticks_done.load(Ordering::Relaxed), true);
                // liczby dopisujemy do widoku na bieżąco — tabela rośnie w oczach
                let pierwszy = mi == 0;
                ctx.edit(true, |j| {
                    if pierwszy {
                        j.rows.push(row.clone());
                        j.rows.sort_by(|a, b| porownaj(a, b));
                    }
                    if !plik.is_empty() && !r.cancelled {
                        j.charts.push(plik.clone());
                    }
                });
                Ok((pr.name.clone(), row, r.metrics, cell))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        // Licznik ruiny do okienka — §4 mówi wprost, że to jedyny próg
        // bezwzględny, więc ma być widoczny w trakcie, nie dopiero w raporcie.
        let mut staty = staty_bazowe.clone();
        let z = p.zerowe.load(Ordering::Relaxed);
        let g = p.results.runs_done.load(Ordering::Relaxed);
        staty.push((
            "konto w ruinie".into(),
            if z == 0 {
                format!("0 z {g} — żaden")
            } else {
                format!("{z} z {g}")
            },
        ));
        ctx.staty_okna(staty);

        for (n, row, m, cell) in partia {
            komorki.entry(n.clone()).or_default().push(cell);
            if mi == 0 {
                wyniki.push((n, row, m));
            }
        }

        // Komplet komórek trafia do widoku po każdym trybie — użytkownik
        // widzi wypełniającą się tabelę 2×2, a nie pustkę przez cztery minuty.
        let quads = zbuduj_quady(&komorki, tryby.len());
        ctx.edit(true, |j| j.quads = quads);
    }

    let przerwane = ctx.cancelled();

    // ================= WALK-FORWARD =================
    // Wybór najlepszego presetu na całym oknie to selekcja W PRÓBIE i zawsze
    // wygląda dobrze. Tutaj uczciwie: zwycięzcę wybieramy na oknie treningowym,
    // a wynik liczymy na NASTĘPNYM, nietkniętym.
    let mut wf: Vec<WfRow> = Vec::new();
    if !przerwane && !wf_okna.is_empty() {
        *p.faza.lock() = "walk-forward".into();
        for (ta, tb, va, vb) in wf_okna.clone() {
            if ctx.cancelled() {
                break;
            }
            let training: Vec<(String, f64)> = presety
                .par_iter()
                .map(|pr| {
                    p.results.begin(&pr.name)?;
                    let cfg = RunConfig {
                        from: ta,
                        to: tb,
                        start_balance: req.balance,
                        settings: pr.settings.clone(),
                        formaty: Vec::new(),
                        pulapy: Default::default(),
                        daily_reset: req.daily_reset,
                        source_name: "ATFX VIP SIGNALS".into(),
                        curve_interval_ms: 600_000,
                        journal_path: None,
                        ..Default::default()
                    };
                    let cb = |d: u64| p.tick(d);
                    let r = run_with_progress(&ticks, &messages, &cfg, Some(&cb));
                    let r = p.results.complete(&pr.name, "walk-forward TRAIN", r)?;
                    Ok((pr.name.clone(), score(&r.metrics)))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let best = training.into_iter()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

            if ctx.cancelled() {
                break;
            }
            // Nazwa pochodzi z tej samej listy, więc `find` zawsze trafia —
            // ale profil `release` ma `panic = "abort"`, a pomyłka w tym
            // rozumowaniu ubiłaby cały serwer razem z oknem. Pominięcie okna
            // jest tu jedyną rozsądną reakcją.
            let Some((name, _)) = best else { continue };
            let Some(pr) = presety.iter().find(|x| x.name == name) else {
                continue;
            };
            let cfg = RunConfig {
                from: va,
                to: vb,
                start_balance: req.balance,
                settings: pr.settings.clone(),
                formaty: Vec::new(),
                pulapy: Default::default(),
                daily_reset: req.daily_reset,
                source_name: "ATFX VIP SIGNALS".into(),
                curve_interval_ms: 600_000,
                journal_path: None,
                ..Default::default()
            };
            p.results.begin(&name)?;
            let cb = |d: u64| p.tick(d);
            let r = run_with_progress(&ticks, &messages, &cfg, Some(&cb));
            let r = p.results.complete(&name, "walk-forward OOS", r)?;
            wf.push(WfRow {
                from: dzien(va),
                to: dzien(vb),
                preset: name,
                profit: r.metrics.total_profit,
                max_dd: r.metrics.max_dd_abs,
                risk_pct: r.metrics.max_open_risk_pct,
                win_days_pct: r.metrics.win_days_pct,
            });
            p.opublikuj(p.ticks_done.load(Ordering::Relaxed), true);
        }
    }

    // ================= ZAPIS =================
    let mut ranking: Vec<(String, LabRow)> = wyniki
        .iter()
        .map(|(n, r, _)| (n.clone(), r.clone()))
        .collect();
    ranking.sort_by(|a, b| porownaj(&a.1, &b.1));

    let mut summary: BTreeMap<String, Metrics> = BTreeMap::new();
    for (n, _, m) in &wyniki {
        summary.insert(n.clone(), m.clone());
    }
    // Nazwa pliku niesie TRYB, którego dotyczą metryki w środku — a `summary`
    // zbiera tylko pierwszy tryb. Przy zwykłym backteście `tryby[0].key` to
    // dokładnie stare „daily"/„compound", więc nazwy plików się nie zmieniają.
    zapisz_json(
        &ctx.out_dir,
        &format!("wyniki_{}.json", tryby[0].key),
        &summary,
    )?;

    let quads = zbuduj_quady(&komorki, tryby.len());
    if req.four_modes {
        zapisz_json(&ctx.out_dir, "ocena4.json", &quads)?;
    }

    let raport = Raport {
        job: ctx
            .out_dir
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default(),
        okno: naglowek.clone(),
        from: dzien(from),
        to: dzien(to),
        balance: req.balance,
        daily_reset: req.daily_reset,
        walk_forward: req.walk_forward,
        cancelled: przerwane,
        four_modes: req.four_modes,
        rows: ranking.iter().map(|(_, r)| r.clone()).collect(),
        quads: quads.clone(),
        wf: wf.clone(),
    };
    zapisz_json(&ctx.out_dir, "raport.json", &raport)?;

    // Wykres porównawczy tylko z przebiegów DOKOŃCZONYCH — zestawienie, w którym
    // jeden słupek jest z pełnego okna, a drugi z jego trzeciej części, kłamie
    // bardziej niż brak wykresu.
    let pelne: Vec<(String, Metrics)> = ranking
        .iter()
        .filter(|(_, r)| !r.partial)
        .filter_map(|(n, _)| {
            wyniki
                .iter()
                .find(|(x, _, _)| x == n)
                .map(|(_, _, m)| (n.clone(), m.clone()))
        })
        .collect();
    if pelne.len() > 1 {
        let svg = chart::compare_chart(&format!("Porównanie konfiguracji · {naglowek}"), &pelne);
        let _ = std::fs::write(ctx.out_dir.join("porownanie.svg"), svg);
        ctx.edit(true, |j| j.charts.insert(0, "porownanie.svg".into()));
    }

    let najlepszy = ranking
        .iter()
        .find(|(n, r)| !r.partial && !summary.get(n.as_str()).map(ruina).unwrap_or(r.blown))
        .or_else(|| ranking.iter().find(|(_, r)| !r.partial && !r.blown));
    let mut note = match najlepszy {
        Some((n, r)) => format!(
            "najlepszy: {n} · {:+.0} $ ({:+.2} $/dz) · dni+ {:.0}% · maxDD {:.0} $ (informacyjnie) · ryzyko {:.0}%",
            r.profit, r.per_day, r.win_days_pct, r.max_dd, r.risk_pct
        ),
        None if ranking.iter().any(|(_, r)| !r.partial) => {
            "ŻADEN wariant nie przeszedł progu bezwzględnego — każdy pełny przebieg \
             doprowadził konto do ruiny"
                .into()
        }
        None if !ranking.is_empty() => {
            "brak przebiegu policzonego do końca — wyniki są wyłącznie cząstkowe".into()
        }
        None => "brak wyników".into(),
    };
    if req.four_modes {
        let przeszlo = quads.iter().filter(|q| q.survives).count();
        note.push_str(&format!(
            " · CZTERY TRYBY: {przeszlo} z {} presetów przeżyło we wszystkich czterech",
            quads.len()
        ));
    }
    if !wf.is_empty() {
        let suma: f64 = wf.iter().map(|x| x.profit).sum();
        note.push_str(&format!(" · POZA PRÓBĄ {suma:+.0} $ w {} oknach", wf.len()));
    }
    if przerwane {
        note.push_str(" · PRZERWANE — wyniki cząstkowe zapisane");
    }

    ctx.edit(true, |j| {
        j.rows = ranking.iter().map(|(_, r)| r.clone()).collect();
        j.quads = quads.clone();
        j.progress = if przerwane { j.progress } else { 1.0 };
        j.label = if przerwane {
            "przerwane — zapisano".into()
        } else {
            "gotowe".into()
        };
    });

    Ok(note)
}

fn zbuduj_quady(komorki: &BTreeMap<String, Vec<LabCell>>, ile_trybow: usize) -> Vec<LabQuad> {
    let mut out: Vec<LabQuad> = komorki
        .iter()
        .map(|(name, cells)| {
            let komplet = cells.len() == ile_trybow;
            let czyste = cells.iter().all(|c| !c.partial);
            LabQuad {
                name: name.clone(),
                survives: komplet && czyste && cells.iter().all(|c| c.ruins == 0 && !c.blown),
                sum_profit: cells.iter().map(|c| c.profit).sum(),
                worst_profit: cells.iter().map(|c| c.profit).fold(f64::INFINITY, f64::min),
                cells: cells.clone(),
            }
        })
        .collect();
    // porządek: najpierw te, które przeżyły wszędzie, potem po NAJSŁABSZYM
    // trybie — §4 wymaga, żeby preset działał we wszystkich czterech naraz,
    // więc o miejscu decyduje najgorsza komórka, nie suma
    out.sort_by(|a, b| {
        b.survives.cmp(&a.survives).then(
            b.worst_profit
                .partial_cmp(&a.worst_profit)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WfRow {
    pub from: String,
    pub to: String,
    pub preset: String,
    pub profit: f64,
    pub max_dd: f64,
    pub risk_pct: f64,
    pub win_days_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raport {
    job: String,
    okno: String,
    from: String,
    to: String,
    balance: f64,
    daily_reset: bool,
    walk_forward: i64,
    cancelled: bool,
    four_modes: bool,
    rows: Vec<LabRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    quads: Vec<LabQuad>,
    wf: Vec<WfRow>,
}

#[cfg(test)]
mod reconciliation_gate_tests {
    use super::*;

    fn result() -> RunResult {
        serde_json::from_value(serde_json::json!({
            "metrics": Metrics {start_balance:600.0, end_equity:1000600.0,
                total_profit:1000000.0, min_equity:600.0, ..Metrics::default()},
            "equity_curve": [], "daily": [], "ticks_processed":2, "elapsed_ms":1
        })).unwrap()
    }

    fn set_hold(result: &mut RunResult, kind: usize, reason: &str) {
        match kind {
            0 => result.cost_reconciliation_required = Some(reason.into()),
            1 => result.sr_warmup_reconciliation_required = Some(reason.into()),
            2 => result.sim_execution_reconciliation_required = Some(reason.into()),
            3 => result.continuation_reconciliation_required = Some(reason.into()),
            _ => unreachable!(),
        }
    }

    #[test]
    fn legacy_metric_conversion_would_publish_held_run_as_a_complete_winner() {
        let mut r = result();
        r.cost_reconciliation_required = Some("missing receipt".into());
        let old = wiersz("held", &r.metrics, "held.svg", r.cancelled);
        assert!(!old.partial && old.score > 0.0,
            "source defect: metric-only conversion loses HOLD before ranking");
        let gate = LabRunGate::default();
        gate.begin("held").unwrap();
        assert!(gate.complete("held", "main", r).is_err());
    }

    #[test]
    fn every_hold_stage_stops_before_chart_row_cell_or_wf_selection() {
        for stage in ["main", "four modes", "walk-forward TRAIN", "walk-forward OOS"] {
            for kind in 0..4 {
                for cancelled in [false, true] {
                    let gate = LabRunGate::default();
                    let mut r = result();
                    set_hold(&mut r, kind, "unresolved evidence");
                    r.cancelled = cancelled;
                    gate.begin("candidate").unwrap();
                    let mut published = 0;
                    let answer = gate.complete("candidate", stage, r).map(|r| {
                        published += 1;
                        (wiersz("candidate", &r.metrics, "chart.svg", r.cancelled), score(&r.metrics))
                    });
                    let error = answer.unwrap_err().to_string();
                    assert_eq!(published, 0);
                    assert!(error.contains("HOLD") && error.contains(stage)
                        && error.contains("candidate") && error.contains("unresolved evidence"));
                    assert!(gate.held());
                    assert!(gate.inflight.lock().is_empty());
                    assert_eq!(gate.runs_done.load(Ordering::Relaxed), 1);
                }
            }
        }
    }

    #[test]
    fn complete_and_user_cancelled_results_remain_byte_identical_without_hold() {
        for cancelled in [false, true] {
            let gate = LabRunGate::default();
            let mut r = result();
            r.cancelled = cancelled;
            let before = serde_json::to_vec(&r).unwrap();
            gate.begin("legacy").unwrap();
            let r = gate.complete("legacy", "main", r).unwrap();
            assert_eq!(serde_json::to_vec(&r).unwrap(), before);
            assert_eq!(wiersz("legacy", &r.metrics, "", r.cancelled).partial, cancelled);
            assert!(!gate.held());
            assert!(gate.inflight.lock().is_empty());
        }
    }

    #[test]
    fn first_hold_stops_new_work_and_drains_existing_inflight_without_replacing_reason() {
        let gate = LabRunGate::default();
        gate.begin("faulty").unwrap();
        gate.begin("already-running").unwrap();
        let mut r = result();
        set_hold(&mut r, 2, "first physical observation fault");
        let first = gate.complete("faulty", "main", r).unwrap_err().to_string();
        assert!(gate.begin("must-not-start").is_err());
        assert_eq!(*gate.inflight.lock(), vec!["already-running"]);
        let mut sibling = result();
        sibling.cancelled = true; // p.tick returns false after gate.held()
        set_hold(&mut sibling, 0, "secondary fault");
        assert_eq!(gate.complete("already-running", "main", sibling).unwrap_err().to_string(), first);
        assert!(gate.inflight.lock().is_empty());
        assert_eq!(gate.runs_done.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn actual_runner_cost_hold_is_not_a_lab_zero_or_winner() {
        // Two local synthetic quotes; no MT5, Telegram, network or UI startup.
        let path = std::env::temp_dir().join(format!("conduit-lab-hold-{}.bin", std::process::id()));
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
            from: ticks.first_ts(), to: ticks.last_ts() + 1, start_balance:600.0,
            settings: Settings {closed_profit_net_costs:true,
                basket_realized_broker_only:false, ..Settings::default()},
            ..RunConfig::default()
        };
        let r = run_with_progress(&ticks, &[], &cfg, None);
        drop(ticks);
        std::fs::remove_file(&path).unwrap();
        assert!(r.cost_reconciliation_required.is_some(), "expected actual runner dependency HOLD");
        assert!(!r.cancelled, "HOLD is not a user cancellation");
        let gate = LabRunGate::default();
        gate.begin("invalid-cost-config").unwrap();
        assert!(gate.complete("invalid-cost-config", "main", r).is_err());
    }

    #[test]
    fn all_three_lab_runner_adapters_gate_before_any_metric_consumer() {
        // Wiring guard complements executed gate/runner tests, not a semantic proof by itself.
        let source = include_str!("backtests.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let calls: Vec<_> = production.split("let r = run_with_progress(").skip(1).collect();
        assert_eq!(calls.len(), 3);
        for call in calls {
            let gate = call.find("p.results.complete(").unwrap();
            let metric = call.find("r.metrics").unwrap();
            assert!(gate < metric, "main, TRAIN and OOS must reject HOLD before losing metadata");
        }
        assert!(production.contains("self.ctx.cancelled() || self.results.held()"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DZIEN: i64 = 86_400_000;

    fn req(period: &str) -> BacktestReq {
        BacktestReq {
            period: period.into(),
            ..Default::default()
        }
    }

    #[test]
    fn okresy_licza_sie_od_konca_danych() {
        let first = 0;
        let last = 100 * DZIEN;
        let (a, b) = okno(&req("all"), first, last).unwrap();
        assert_eq!(a, first);
        assert_eq!(b, last + 1);

        let (a, _) = okno(&req("last-week"), first, last).unwrap();
        assert_eq!(a, last + 1 - 7 * DZIEN);

        let (a, _) = okno(&req("last-month"), first, last).unwrap();
        assert_eq!(a, last + 1 - 30 * DZIEN);

        let (a, _) = okno(&req("last-3-days"), first, last).unwrap();
        assert_eq!(a, last + 1 - 3 * DZIEN);
    }

    /// Okno nie może wyjść przed pierwszy tick — inaczej pasek postępu
    /// obiecywałby przemiał danych, których nie ma.
    #[test]
    fn okno_nie_wychodzi_przed_dane() {
        let first = 90 * DZIEN;
        let last = 100 * DZIEN;
        let (a, _) = okno(&req("all"), first, last).unwrap();
        assert_eq!(a, first);
        let (a, _) = okno(&req("last-month"), first, last).unwrap();
        assert_eq!(
            a, first,
            "30 dni wstecz sięga poza dane — obcinamy do pierwszego ticku"
        );
    }

    #[test]
    fn zakres_dat_obejmuje_dzien_koncowy() {
        let r = BacktestReq {
            period: "range".into(),
            from: "2026-06-01".into(),
            to: "2026-06-02".into(),
            ..Default::default()
        };
        let (a, b) = okno(&r, 0, 100_000 * DZIEN).unwrap();
        assert_eq!(
            b - a,
            2 * DZIEN,
            "zakres „do 2 czerwca” musi zawierać cały 2 czerwca"
        );
    }

    #[test]
    fn zly_okres_i_puste_okno_to_bledy() {
        assert!(okno(&req("wczoraj"), 0, DZIEN).is_err());
        let r = BacktestReq {
            period: "range".into(),
            from: "2026-06-10".into(),
            to: "2026-06-01".into(),
            ..Default::default()
        };
        assert!(okno(&r, 0, 100_000 * DZIEN).is_err());
    }

    /// REGRESJA: po przerwaniu przemiału czołówkę rankingu potrafiła zająć
    /// konfiguracja, która przemieliła tylko kawałek okna — i to ona trafiała
    /// do podsumowania jako „najlepsza".
    #[test]
    fn przebiegi_czastkowe_ida_pod_pelne() {
        let pelny = LabRow {
            name: "pelny".into(),
            score: 0.5,
            partial: false,
            ..Default::default()
        };
        let czastkowy = LabRow {
            name: "czastkowy".into(),
            score: 9.9,
            partial: true,
            ..Default::default()
        };
        let slabszy = LabRow {
            name: "slabszy".into(),
            score: 0.1,
            partial: false,
            ..Default::default()
        };

        let mut v = vec![czastkowy, slabszy, pelny];
        v.sort_by(porownaj);
        assert_eq!(
            v[0].name, "pelny",
            "pierwszy musi być przebieg policzony do końca"
        );
        assert_eq!(v[1].name, "slabszy");
        assert_eq!(
            v[2].name, "czastkowy",
            "cząstkowy nie wygrywa mimo najwyższej oceny"
        );
    }

    /// Metryki „zdrowego" przebiegu: konto przeżyło, equity nie zeszło do ruiny.
    fn zdrowy(profit: f64, max_dd: f64, min_eq: f64) -> Metrics {
        Metrics {
            start_balance: 200.0,
            total_profit: profit,
            max_dd_abs: max_dd,
            min_equity: min_eq,
            ..Default::default()
        }
    }

    #[test]
    fn wyzerowane_konto_ladue_na_koncu_rankingu() {
        let dobry = zdrowy(100.0, 50.0, 150.0);
        let zly = Metrics {
            total_profit: 5000.0,
            blown: true,
            ..zdrowy(5000.0, 10.0, 150.0)
        };
        assert!(score(&dobry) > score(&zly));
    }

    #[test]
    fn obsuniecie_nie_decyduje_o_kolejnosci() {
        let spokojny = zdrowy(414.0, 42.0, 160.0);
        let zyskowny = zdrowy(764.0, 130.0, 120.0);
        assert!(
            score(&zyskowny) > score(&spokojny),
            "większy zysk ma wygrywać mimo większego maxDD — inaczej rangujemy po obsunięciu"
        );

        // otwarte ryzyko ponad połowę kapitału TEŻ nie jest powodem odrzucenia,
        // dopóki konto przeżyło
        let ryzykant = Metrics {
            max_open_risk_pct: 200.0,
            ..zdrowy(9999.0, 10.0, 90.0)
        };
        assert!(score(&ryzykant) > score(&spokojny));
    }

    /// Jedyny próg bezwzględny: konto nie może dojść do zera ANI do poziomu,
    /// z którego minimalny lot już nie odrobi.
    #[test]
    fn ruina_to_zero_albo_podloga_nie_obsuniecie() {
        assert!(
            !ruina(&zdrowy(100.0, 190.0, 41.0)),
            "40,5 $ na koncie 200 $ jeszcze nie jest ruiną"
        );
        assert!(
            ruina(&zdrowy(100.0, 190.0, 40.0)),
            "równo na progu 20 % = ruina"
        );
        assert!(
            ruina(&zdrowy(100.0, 190.0, -23.0)),
            "ujemne equity to ruina"
        );
        assert!(ruina(&Metrics {
            blown: true,
            ..zdrowy(100.0, 1.0, 199.0)
        }));
        // samo wielkie obsunięcie, przy zdrowym dnie equity, ruiną NIE JEST
        assert!(!ruina(&zdrowy(-50.0, 307.0, 150.0)));
    }

    #[test]
    fn wyzerowany_nie_zostaje_najlepszy() {
        let ruina_row = LabRow {
            name: "SWEEP-A-5".into(),
            profit: -200.0,
            blown: true,
            ..Default::default()
        };
        let dobry_ale_czastkowy = LabRow {
            name: "MAKS".into(),
            profit: 376.0,
            partial: true,
            ..Default::default()
        };
        let mut v = vec![ruina_row, dobry_ale_czastkowy];
        v.sort_by(porownaj);
        // stara reguła: pierwszy niecząstkowy → SWEEP-A-5
        assert_eq!(
            v.iter().find(|r| !r.partial).map(|r| r.name.as_str()),
            Some("SWEEP-A-5")
        );
        // nowa: pierwszy niecząstkowy I nie-ruina → nie ma takiego
        assert!(v.iter().find(|r| !r.partial && !r.blown).is_none());
    }

    fn dzien_stat(date: &str, profit: f64, end_eq: f64, trades: u32) -> DayStat {
        DayStat {
            day: 0,
            date: date.into(),
            start_equity: 200.0,
            end_equity: end_eq,
            profit,
            max_dd: 0.0,
            trades,
            signals: 0,
        }
    }

    #[test]
    fn miesiace_licza_sie_osobno_i_tylko_z_dni_handlowych() {
        let d = vec![
            dzien_stat("2026-06-01", 10.0, 210.0, 3),
            dzien_stat("2026-06-02", -4.0, 206.0, 2),
            dzien_stat("2026-06-03", 0.0, 206.0, 0), // dzień bez transakcji — pomijamy
            dzien_stat("2026-07-01", 25.0, 231.0, 5),
        ];
        let m = miesiace(&d);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].month, "2026-06");
        assert!((m[0].profit - 6.0).abs() < 1e-9);
        assert_eq!(
            m[0].days, 2,
            "dzień bez transakcji nie jest dniem handlowym"
        );
        assert_eq!(m[0].loss_days, 1);
        assert_eq!(m[1].month, "2026-07");
        assert!((m[1].profit - 25.0).abs() < 1e-9);
    }

    /// Wyzerowania liczymy INACZEJ w obu trybach i to jest celowe.
    #[test]
    fn wyzerowania_zalezne_od_trybu() {
        let m = zdrowy(0.0, 0.0, 5.0);
        // trzy doby pod progiem 40 $ z rzędu
        let d = vec![
            dzien_stat("2026-06-01", -170.0, 30.0, 4),
            dzien_stat("2026-06-02", -175.0, 25.0, 4),
            dzien_stat("2026-06-03", -180.0, 20.0, 4),
        ];
        assert_eq!(
            wyzerowania(&d, &m, true, 200.0),
            3,
            "dzień po dniu: każda doba to osobne konto, więc trzy wyzerowania"
        );
        assert_eq!(
            wyzerowania(&d, &m, false, 200.0),
            1,
            "długoterminowo: konto padło RAZ, a nie raz na każdy pozostały dzień"
        );
        // dwa osobne wejścia w ruinę z odbiciem pośrodku
        let d2 = vec![
            dzien_stat("2026-06-01", -170.0, 30.0, 4),
            dzien_stat("2026-06-02", 170.0, 200.0, 4),
            dzien_stat("2026-06-03", -170.0, 30.0, 4),
        ];
        assert_eq!(wyzerowania(&d2, &m, false, 200.0), 2);
    }

    #[test]
    fn ruina_w_srodku_doby_tez_sie_liczy() {
        // dzień zaczyna się i kończy zdrowo, ale equity zjechało do 16 $
        let dni = vec![dzien_stat("2026-06-01", 12.0, 212.0, 40)];
        let m = zdrowy(12.0, 190.0, 16.42);
        assert_eq!(
            wyzerowania(&dni, &m, false, 200.0),
            1,
            "dno 16,42 $ przy progu 40 $ to ruina, choćby doba skończyła się na +12 $"
        );
        assert_eq!(wyzerowania(&dni, &m, true, 200.0), 1);

        // i kontrola w drugą stronę: zdrowe dno to nadal zero wyzerowań
        let zdrowe = zdrowy(12.0, 60.0, 141.0);
        assert_eq!(wyzerowania(&dni, &zdrowe, false, 200.0), 0);

        // spójność z `ruina()` — obie drogi muszą dawać ten sam werdykt
        for min_eq in [16.42, 27.9, 39.9, 40.0, 40.1, 150.0] {
            let mm = zdrowy(12.0, 190.0, min_eq);
            assert_eq!(
                wyzerowania(&dni, &mm, false, 200.0) > 0,
                ruina(&mm),
                "licznik wyzerowań i próg ruiny rozjeżdżają się przy dnie {min_eq}"
            );
        }
    }

    /// Compounding musi być OSIĄ, a nie zmianą wielkości pozycji: oba ramiona
    /// startują od tego samego lota (NAUKOWIEC §3C — kontrola ekspozycji).
    #[test]
    fn oba_ramiona_compoundingu_startuja_od_tego_samego_lota() {
        use conduit_core::engine::Engine;
        // DWIE bazy: bez bonusu i z bonusem odliczanym od podstawy lota.
        // Druga jest tu dlatego, że `ustaw_compounding` liczyło `lot0`
        // z SUROWEGO salda — na koncie 600 $ z bonusem 300 $ zamrażało lot
        // dwa razy za duży, a w ramieniu compoundingu rozjeżdżało pierwsze
        // wejście o stosunek podstawa/saldo. Sam `Settings::default()` tego
        // nie łapie, bo ma kredyt wyłączony.
        let mut z_bonusem = Settings::default();
        z_bonusem.odlicz_kredyt = true;
        z_bonusem.kredyt_reczny = 300.0;

        for baza in [Settings::default(), z_bonusem] {
            let kredyt = if baza.odlicz_kredyt {
                baza.kredyt_reczny
            } else {
                0.0
            };
            for balance in [400.0, 1000.0, 5000.0] {
                let podstawa = (balance - kredyt).max(0.0);
                let mut bez = baza.clone();
                let mut z = baza.clone();
                super::super::ustaw_compounding(&mut bez, false, balance);
                super::super::ustaw_compounding(&mut z, true, balance);
                // Lot liczymy TĄ SAMĄ drogą co silnik: od podstawy, nie od salda.
                let l_bez = Engine::new(bez.clone(), balance).lot_size(podstawa);
                let l_z = Engine::new(z.clone(), balance).lot_size(podstawa);
                assert!(
                    (l_bez - l_z).abs() < 1e-9,
                    "przy {balance} $ (kredyt {kredyt}) ramiona startują różnym lotem: \
                     {l_bez} vs {l_z} — to mierzyłoby wielkość pozycji, nie compounding"
                );
                assert!(!bez.lot_mode_percent && bez.lot_scale_step == 0.0);
                assert!(z.lot_mode_percent && z.lot_scale_step == 0.0);
                // i sedno osi: przy DWUKROTNIE większym koncie tylko jedno ramię rośnie
                let p2 = (balance * 2.0 - kredyt).max(0.0);
                let l_bez2 = Engine::new(bez, balance * 2.0).lot_size(p2);
                let l_z2 = Engine::new(z, balance * 2.0).lot_size(p2);
                assert!(
                    (l_bez2 - l_bez).abs() < 1e-9,
                    "lot stały nie ma prawa urosnąć"
                );
                assert!(l_z2 > l_z, "compounding ma rosnąć razem z kontem");
            }
        }
    }

    /// Zwykły backtest ma iść DOKŁADNIE tą samą ścieżką co przed dodaniem
    /// czterech trybów: jeden wariant i ZERO podmian w ustawieniach presetu.
    #[test]
    fn bez_czterech_trybow_nie_dotykamy_ustawien_presetu() {
        let r = BacktestReq {
            daily_reset: true,
            ..Default::default()
        };
        let w = warianty(&r);
        assert_eq!(w.len(), 1);
        assert!(w[0].daily_reset);
        assert!(
            w[0].compounding.is_none(),
            "brak podmiany lota = wynik co do bitu jak dotąd"
        );

        let r4 = BacktestReq {
            four_modes: true,
            ..Default::default()
        };
        let w4 = warianty(&r4);
        assert_eq!(w4.len(), 4);
        assert_eq!(
            w4.iter().filter(|x| x.daily_reset).count(),
            2,
            "dwa tryby dzienne i dwa długoterminowe"
        );
        assert_eq!(w4.iter().filter(|x| x.compounding == Some(true)).count(), 2);
    }

    fn cela(mode: &str, profit: f64, ruins: u32) -> LabCell {
        LabCell {
            mode: mode.into(),
            profit,
            ruins,
            ..Default::default()
        }
    }

    #[test]
    fn quad_przechodzi_tylko_przy_komplecie_czterech_zdrowych() {
        let mut k: BTreeMap<String, Vec<LabCell>> = BTreeMap::new();
        k.insert(
            "DOBRY".into(),
            vec![
                cela("dpd-staly", 100.0, 0),
                cela("dpd-comp", 120.0, 0),
                cela("dlugo-staly", 300.0, 0),
                cela("dlugo-comp", 900.0, 0),
            ],
        );
        k.insert(
            "ZERUJE".into(),
            vec![
                cela("dpd-staly", 500.0, 0),
                cela("dpd-comp", 900.0, 0),
                cela("dlugo-staly", 800.0, 0),
                cela("dlugo-comp", 2000.0, 1), // zeruje w jednym trybie
            ],
        );
        k.insert(
            "NIEPELNY".into(),
            vec![cela("dpd-staly", 999.0, 0), cela("dpd-comp", 999.0, 0)],
        );

        let q = zbuduj_quady(&k, 4);
        assert_eq!(q.len(), 3);
        assert_eq!(
            q[0].name, "DOBRY",
            "tylko ten przeżył we wszystkich czterech"
        );
        assert!(q[0].survives);
        assert!(
            (q[0].worst_profit - 100.0).abs() < 1e-9,
            "o miejscu decyduje NAJSŁABSZY tryb"
        );

        let zeruje = q.iter().find(|x| x.name == "ZERUJE").unwrap();
        assert!(
            !zeruje.survives,
            "jedno wyzerowanie dyskwalifikuje mimo +4200 $ łącznie"
        );
        let niepelny = q.iter().find(|x| x.name == "NIEPELNY").unwrap();
        assert!(
            !niepelny.survives,
            "dwie zmierzone komórki to nie „przechodzi w połowie”"
        );
    }
}


#[cfg(test)]
mod source_telemetry_tests {
    use super::*;

    #[test]
    fn entry_sources_and_closed_transactions_stay_separate_in_lab_rows() {
        let metrics = Metrics { entry_source_observation_version: 1, known_entry_sources: 7, known_full_entry_sources: 4,
            entry_sources_first_seen_as_edit: 3, trades: 31, ..Metrics::default() };
        let row = wiersz("synthetic", &metrics, "", false);
        assert_eq!(row.trades, 31);
        assert_eq!(row.known_entry_sources, Some(7));
        assert_eq!(row.known_full_entry_sources, Some(4));
        assert_eq!(row.entry_sources_first_seen_as_edit, Some(3));
        let legacy = wiersz("legacy", &Metrics::default(), "", false);
        assert_eq!(legacy.known_entry_sources, None);
        assert_eq!(legacy.known_full_entry_sources, None);
        assert_eq!(legacy.entry_sources_first_seen_as_edit, None);
        let measured_zero = wiersz("measured zero", &Metrics { entry_source_observation_version: 1, ..Metrics::default() }, "", false);
        assert_eq!(measured_zero.known_entry_sources, Some(0));
    }

    #[test]
    fn archived_lab_rows_keep_missing_source_counts_unknown() {
        let mut json = serde_json::to_value(LabRow::default()).unwrap();
        for key in ["knownEntrySources", "knownFullEntrySources", "entrySourcesFirstSeenAsEdit"] {
            json.as_object_mut().unwrap().remove(key);
        }
        let archived: LabRow = serde_json::from_value(json).unwrap();
        assert_eq!(archived.known_entry_sources, None);
        assert_eq!(archived.known_full_entry_sources, None);
        assert_eq!(archived.entry_sources_first_seen_as_edit, None);
        let serialized = serde_json::to_value(archived).unwrap();
        assert!(serialized.get("knownEntrySources").is_none());
        let cell: LabCell = serde_json::from_value(serde_json::to_value(LabCell::default()).unwrap()).unwrap();
        assert_eq!(cell.known_entry_sources, None);
    }
}
