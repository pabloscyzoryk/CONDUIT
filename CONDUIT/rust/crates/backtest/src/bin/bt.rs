//! CLI backtestów.
//!
//! Przykłady:
//!   bt --period all --preset presets/ATFX-BAZA.json
//!   bt --period last-month --sweep presets/ --daily-reset
//!   bt --from 2026-06-01 --to 2026-06-08 --sweep presets/ --balance 200
//!
//! GRANICE OKNA: `--from` WŁĄCZNIE, `--to` WYŁĄCZNIE — mierzone jest
//! `[--from, --to)`. Przykład wyżej obejmuje 01…07 czerwca; 08 czerwca już nie.
//! Pojedynczy dzień pisze się `--from 2026-08-06 --to 2026-08-07`.

use anyhow::{bail, Result};
use conduit_backtest::chart;
use conduit_backtest::data::{load_messages_with_time_offset, TickData};
use conduit_backtest::metrics::Metrics;
use conduit_backtest::runner::{
    run, run_with_progress, zbuduj_drabinke, FormatCfg, RunConfig, SzczebelCfg,
};
use conduit_core::formaty::PulapyGlobalne;
use conduit_core::settings::{Preset, Settings};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Jedyny pisarz pliku oglądanego na żywo przez `postep.exe`.
///
/// Wszystkie wątki Rayona kończą presety niezależnie, ale mapa, serializacja i
/// podmiana pliku przechodzą przez TEN SAM mutex. Dzięki temu dwa kończące się
/// naraz przebiegi nie nadpiszą sobie wyników ani wspólnego pliku `.tmp`.
/// Trzymamy wyłącznie [`Metrics`], nigdy transakcje, koszyki ani krzywe — także
/// `--summary-only` dostaje więc pełne liczby do selekcji bez dużych artefaktów.
struct PisarzWynikowCzastkowych {
    cel: PathBuf,
    wyniki: HashMap<String, Metrics>,
    quick_tick_stride: usize,
    blad_zgloszony: bool,
}

impl PisarzWynikowCzastkowych {
    fn nowy(katalog: &Path, quick_tick_stride: usize) -> Self {
        Self {
            cel: katalog.join("wyniki_czastkowe.json"),
            wyniki: HashMap::new(),
            quick_tick_stride,
            blad_zgloszony: false,
        }
    }

    /// Usuwa wyniki z poprzedniego uruchomienia, zanim pierwszy preset ruszy.
    fn wyzeruj(&mut self) -> std::io::Result<()> {
        self.wyniki.clear();
        self.zapisz()
    }

    /// Dodaje JEDEN ukończony preset i od razu publikuje pełną mapę.
    fn dodaj(&mut self, nazwa: String, metryki: Metrics) -> std::io::Result<()> {
        self.wyniki.insert(nazwa, metryki);
        self.zapisz()
    }

    fn zapisz(&self) -> std::io::Result<()> {
        let tmp = self.cel.with_file_name("wyniki_czastkowe.json.tmp");
        let txt = if self.quick_tick_stride > 1 {
            serde_json::to_vec(&serde_json::json!({
                "schema": "conduit.quick-sweep-partial.v1",
                "approximate": true,
                "coronation_eligible": false,
                "quick_tick_stride": self.quick_tick_stride,
                "warning": "APPROXIMATE SCREENING ONLY — rerun finalists with N=1",
                "results": &self.wyniki,
            }))
        } else {
            // Exact N=1 retains the historical direct name -> Metrics shape.
            serde_json::to_vec(&self.wyniki)
        }
        .map_err(std::io::Error::other)?;
        std::fs::write(&tmp, txt)?;
        match std::fs::rename(&tmp, &self.cel) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Czytający nadal ma poprzedni, kompletny dokument; nie
                // zostawiamy śmiecia, którego ktoś mógłby uznać za wynik.
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    /// Błąd dysku ma być głośny, ale nie 1200 razy i nie może przerwać sweepu.
    fn dodaj_z_raportem(&mut self, nazwa: String, metryki: Metrics) {
        match self.dodaj(nazwa, metryki) {
            Ok(()) => self.blad_zgloszony = false,
            Err(e) if !self.blad_zgloszony => {
                eprintln!("  !! nie udało się zapisać wyników cząstkowych na żywo: {e}");
                self.blad_zgloszony = true;
            }
            Err(_) => {}
        }
    }
}

fn days_from_ymd(y: i64, m: i64, d: i64) -> i64 {
    let yy = if m <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_date(s: &str) -> Result<i64> {
    let p: Vec<&str> = s.split('-').collect();
    if p.len() != 3 {
        bail!("data musi mieć format RRRR-MM-DD, dostałem: {s}");
    }
    Ok(days_from_ymd(p[0].parse()?, p[1].parse()?, p[2].parse()?) * 86_400_000)
}

/// Kontrakt czasu jest liczony z EFEKTYWNYCH ustawień rachunku, dokładnie
/// tych, które czyta runner (również przy wielu formatach i drabince).
/// CLI i preset nie mogą po cichu wykonać tej samej normalizacji dwa razy.
#[derive(serde::Serialize)]
struct ReplayClockContract {
    variant: String,
    cli_signal_offset_ms: i64,
    preset_server_tz_offset_ms: i64,
    preset_msg_clock_offset_ms: Option<i64>,
    preset_msg_offset_ms: i64,
    effective_clock_offset_ms: i64,
    exec_latency_ms: i64,
    effective_dispatch_offset_ms: i64,
    live_tick_order_strict: bool,
}

fn replay_clock_contract(
    cli_offset_min: i64,
    variant: &str,
    settings: &Settings,
) -> Result<ReplayClockContract> {
    let cli_offset_ms = cli_offset_min
        .checked_mul(60_000)
        .ok_or_else(|| anyhow::anyhow!("offset CLI nie mieści się w i64: {cli_offset_min} min"))?;
    let preset_offset_ms = settings.msg_offset();
    let total_ms = cli_offset_ms
        .checked_add(preset_offset_ms)
        .ok_or_else(|| anyhow::anyhow!("suma offsetów zegara nie mieści się w i64: {variant}"))?;
    if cli_offset_ms != 0 && preset_offset_ms != 0 {
        bail!(
            "PODWÓJNE PRZESUNIĘCIE ZEGARA REPLAYU: wariant „{variant}”: \
             --signal-time-offset-min {cli_offset_min:+} min + efektywny \
             Settings::msg_offset() {:+} ms ({:+.3} min) = suma {:+} ms ({:+.3} min). \
             Runner dodaje offset presetu PO normalizacji CLI. Użyj oryginalnego \
             korpusu UTC z --signal-time-offset-min 0 (GOD-X4 już dodaje +180 min) \
             albo jawnie ustaw msg_clock_offset_ms=0 w efektywnym presecie/rachunku \
             i normalizuj zegar wyłącznie przez CLI. Nie uruchomiono backtestu.",
            preset_offset_ms,
            preset_offset_ms as f64 / 60_000.0,
            total_ms,
            total_ms as f64 / 60_000.0,
        );
    }
    let dispatch_ms = total_ms
        .checked_add(settings.exec_latency_ms)
        .ok_or_else(|| anyhow::anyhow!("offset z opóźnieniem nie mieści się w i64: {variant}"))?;
    Ok(ReplayClockContract {
        variant: variant.to_string(),
        cli_signal_offset_ms: cli_offset_ms,
        preset_server_tz_offset_ms: settings.server_tz_offset_ms,
        preset_msg_clock_offset_ms: settings.msg_clock_offset_ms,
        preset_msg_offset_ms: preset_offset_ms,
        effective_clock_offset_ms: total_ms,
        exec_latency_ms: settings.exec_latency_ms,
        effective_dispatch_offset_ms: dispatch_ms,
        live_tick_order_strict: settings.live_tick_order_strict,
    })
}

struct Args {
    ticks: PathBuf,
    signals: PathBuf,
    /// Jawna normalizacja zegara strumienia replayu. To właściwość
    /// korpusu danych, nie strategii; 0 zachowuje historyczne zachowanie.
    signal_time_offset_min: i64,
    signal_time_offset_explicit: bool,
    sim_limit_price_improvement: bool,
    sim_new_pending_sl_next_tick: bool,
    sim_native_swap_cash_digits: Option<u32>,
    sim_trade_sessions: Option<conduit_backtest::trade_sessions::TradeSessionProfile>,
    /// Raw Telegram replay uses the exact VPS content-dedup ingress.
    live_telegram_ingress: bool,
    /// Explicit approximate screening stride; 1 is the exact legacy path.
    quick_tick_stride: usize,
    sim_price_digits: Option<u32>,
    /// początek okna WŁĄCZNIE (północka tego dnia)
    from: Option<i64>,
    /// Koniec okna WYŁĄCZNIE (północ tego dnia). Mierzymy `[from, to)` — dzień
    /// podany w `--to` NIE jest mierzony. Tak było od początku (`--to` bez
    /// wartości daje `t_last + 1`, czyli granicę otwartą) i tak zostaje: 190
    /// wywołań w skryptach i raportach (m.in. bramka `PARYTET.md`) mierzy dziś
    /// dokładnie to, co mierzyło, a przesunięcie granicy o dobę unieważniłoby
    /// każdą archiwalną liczbę zapisaną obok jej polecenia.
    to: Option<i64>,
    period: String,
    preset: Option<PathBuf>,
    sweep: Option<PathBuf>,
    balance: f64,
    daily_reset: bool,
    /// TRYB AUTO-EA (`--auto-ea`) — patrz `RunConfig::auto_ea`.
    ///
    /// Istnieje po to i wyłącznie po to, żeby punkt (c) potrójnego kontraktu
    /// zera („AUTO-EA bez osi = AUTO co do centa") dało się UDOWODNIĆ na
    /// korpusie, a nie tylko na atrapie brokera.
    auto_ea: bool,
    /// „dzień po dniu, ale saldo przechodzi" — patrz `RunConfig::flat_na_dobie`
    flat_na_dobie: bool,
    out: PathBuf,
    top: usize,
    quiet: bool,
    walk_forward: i64,
    dump_trades: bool,
    /// pomiń zapis wykresów SVG — przy przemiataniu dziesiątek tysięcy
    /// konfiguracji wykresy zajmują pół giga na tysiąc przebiegów i nikt
    /// ich nie ogląda; liczby w `wyniki_*.json` są nietknięte
    no_charts: bool,
    /// Masowy sweep: zachowaj pełne metryki, ale po ich policzeniu zwolnij
    /// ciężkie artefakty diagnostyczne i nie twórz pliku per preset.
    /// Decyzje silnika i arytmetyka metryk pozostają identyczne.
    summary_only: bool,
    /// ścieżka pliku `.jsonl` z dziennikiem zdarzeń (tylko dla przebiegu
    /// pojedynczego presetu — sto presetów piszących do jednego pliku dawałoby
    /// przeplecione linie i bezużyteczny raport)
    journal: Option<PathBuf>,
    /// OKNA PRZESUWANE („n-ki"): długość okna w dniach handlowych. 0 = tryb
    /// wyłączony, 1 = to samo co `--daily-reset`. Patrz `NKI_ZZN.md`.
    reset_co: u32,
    /// ZZN — Z ZACHOWANIEM NOCNYM: po granicy okna nie bierzemy nowych
    /// sygnałów, ale koszyki z okna dochodzą do naturalnego końca.
    zzn: bool,
    /// sufit ogona ZZN w dobach
    zzn_max_dni: u32,
    /// CO ILE MILISEKUND PRÓBKOWAĆ KRZYWĄ KAPITAŁU.
    ///
    /// Domyślne 5 minut wystarcza do oglądania miesięcy, ale przy zejściu na
    /// poziom minuty (Conduit Graph) gubi cały kształt dnia — a to właśnie
    /// wtedy chce się zobaczyć, co dokładnie stało się o 14:03. Zejście do
    /// 60 000 zwiększa plik danych mniej więcej pięciokrotnie i nie wpływa
    /// na wynik przebiegu ani o cent: to wyłącznie częstość ZAPISU.
    krzywa_ms: i64,

    // ---------- WIELE PRESETÓW NARAZ ----------
    /// Mapa `format → plik presetu` z `--preset-format NAZWA=plik.json`.
    ///
    /// Kolejność JEST znacząca: pierwszy podany format opisuje RACHUNEK
    /// (patrz `rachunek` niżej). Poza tym sloty i tak rozdaje rdzeń po nazwie.
    formaty: Vec<(String, PathBuf)>,
    /// Warianty pułapów globalnych: `(etykieta, pułapy)`.
    ///
    /// Każdy wariant to OSOBNY przebieg, liczony równolegle z pozostałymi —
    /// dokładnie tak, jak sweep po presetach. Pusta lista = jeden przebieg
    /// bez pułapów.
    pulapy: Vec<(String, PulapyGlobalne)>,
    /// Skąd wziąć POLA RACHUNKU (dźwignia, swap, poślizg, opóźnienie, karta
    /// lota) przy wielu formatach. `None` = z pierwszego `--preset-format`
    /// (przy drabince: z pierwszej nogi NAJNIŻSZEGO szczebla).
    rachunek: Option<PathBuf>,

    // ---------- DRABINKA ŁAŃCUCHÓW ----------
    /// `--drabinka "0=ZENONLY5,500=ZENONLY3,1000=SENTINEL-0,1500=SENTINEL-0A"`
    /// — progi SALDA (nie equity) i nazwy ŁAŃCUCHÓW WBUDOWANYCH
    /// (`formaty.rs::lancuchy_wbudowane`). Presety nóg ładują się z
    /// `--presety-dir` po nazwie z łańcucha.
    ///
    /// Szczebel wolno też podać jako POJEDYNCZY preset: `preset:NAZWA`
    /// (plik `--presety-dir/NAZWA.json`) albo `plik:ŚCIEŻKA`. Wtedy nogą jest
    /// pole `format` presetu, a pułapy globalne są puste. Patrz
    /// `runner::zbuduj_drabinke`.
    drabinka: Option<String>,
    /// histereza schodzenia w dół, w % progu (0 = bez histerezy)
    drabinka_histereza_pct: f64,
    /// katalog z presetami dla nóg łańcuchów drabinki
    presety_dir: PathBuf,
    /// ROZGRZEWKA HISTORII RYNKU: ile godzin ceny SPRZED `--from` wsypać do
    /// silnika, zanim zacznie handlować. **0 = ZIMNY START i to jest domyślna**,
    /// bo na niej stoi bramka parytetu (1936,47 / 1178,02 / 2527,07).
    ///
    /// # Po co ta flaga powstała (SENTINEL-0, 03.08.2026)
    ///
    /// Bez niej okno `--from 2026-07-01` mierzy co innego niż okno
    /// `--from 2026-06-01`, i to nie dlatego, że lipiec był gorszy, tylko
    /// dlatego, że przy `regime_ma_hours = 72` silnik startujący 1 lipca ma
    /// przez pierwsze trzy doby filtr reżimu ŚLEPY. Zmierzone na tym samym
    /// presecie i tych samych dniach: noga ZEN w oknie lipcowym schodziła na
    /// dno 2,75 $ przy RYZ 1153 %, a w oknie pełnym te same dni dawały dno
    /// 112,00 $ przy RYZ 314 %. Różnica NIE była własnością lipca.
    ///
    /// `live.rs::rozgrzej_historie` i `lotto --rozgrzewka-h` mają to od
    /// 03.08.2026; backtest był ostatnim miejscem, w którym okno krótsze niż
    /// `regime_ma_hours` cicho mierzyło ślepy filtr.
    rozgrzewka_h: usize,
}

/// Wczytuje preset z pliku.
fn wczytaj_preset(p: &Path) -> Result<Preset> {
    let txt = std::fs::read_to_string(p)
        .map_err(|e| anyhow::anyhow!("nie mogę wczytać presetu {}: {e}", p.display()))?;
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
        let obce = conduit_core::nieznane_pola_ustawien(&v);
        if !obce.is_empty() {
            eprintln!(
                "  !! PRESET MA POLA, KTORYCH TA BINARKA NIE ZNA: {} -- sa POMIJANE, wiec przebieg opisuje INNY preset niz podany; przebuduj btp/conduit z biezacego silnika",
                obce.join(", ")
            );
        }
    }
    serde_json::from_str::<Preset>(&txt)
        .map_err(|e| anyhow::anyhow!("{} nie jest presetem: {e}", p.display()))
}

/// Parsuje `--pulapy` — JSON pułapów z opcjonalnym polem `nazwa`.
///
/// `nazwa` NIE jest polem [`PulapyGlobalne`]; służy wyłącznie do podpisania
/// wiersza w tabeli, żeby przemiatanie kilkunastu wariantów dało się czytać.
/// serde ignoruje nieznane klucze, więc wystarczy wyjąć ją wcześniej.
fn parsuj_pulapy(s: &str, nr: usize) -> Result<(String, PulapyGlobalne)> {
    let v: serde_json::Value = serde_json::from_str(s)
        .map_err(|e| anyhow::anyhow!("--pulapy: to nie jest poprawny JSON ({e}): {s}"))?;
    let nazwa = v
        .get("nazwa")
        .and_then(|x| x.as_str())
        .map(|x| x.to_string())
        .unwrap_or_else(|| format!("pulapy{nr}"));
    let p: PulapyGlobalne = serde_json::from_value(v)
        .map_err(|e| anyhow::anyhow!("--pulapy: nieznane pole albo zły typ ({e}): {s}"))?;
    Ok((nazwa, p))
}

struct Wariant {
    nazwa: String,
    /// ustawienia przebiegu jednoformatowego ALBO pola rachunku przy wielu
    settings: Settings,
    formaty: Vec<FormatCfg>,
    pulapy: PulapyGlobalne,
    /// plakietka źródła do `*_dane.json`
    format: String,
    /// szczeble drabinki — puste poza trybem `--drabinka`
    drabinka: Vec<SzczebelCfg>,
    /// konfiguracja warstwy EA z pola `ea` presetu (`None` = nie ruszaj)
    ea: Option<String>,
}

// Jeden wariant używany przez MAIN, walk-forward oraz tryb okien.
impl WariantOkien for Wariant {
    fn etykieta(&self) -> &str {
        &self.nazwa
    }
    fn ustawienia(&self) -> &Settings {
        &self.settings
    }
    fn nogi(&self) -> &[FormatCfg] {
        &self.formaty
    }
    fn sufity(&self) -> &PulapyGlobalne {
        &self.pulapy
    }
}


/// Konstruktor MAIN wyodrębniony bez zmiany istniejących wartości.
fn main_run_config(a: &Args, p: &Wariant, from: i64, to: i64,
    journal: Option<PathBuf>) -> RunConfig {
    RunConfig {
        from,
        to,
        start_balance: a.balance,
        sim_limit_price_improvement: a.sim_limit_price_improvement,
        sim_new_pending_sl_next_tick: a.sim_new_pending_sl_next_tick,
        sim_native_swap_cash_digits: a.sim_native_swap_cash_digits,
        sim_trade_sessions: a.sim_trade_sessions.clone(),
        live_telegram_ingress: a.live_telegram_ingress,
        quick_tick_stride: a.quick_tick_stride,
        settings: p.settings.clone(),
        ea_konfig: p.ea.clone(),
        formaty: p.formaty.clone(),
        pulapy: p.pulapy.clone(),
        daily_reset: a.daily_reset,
        auto_ea: a.auto_ea,
        flat_na_dobie: a.flat_na_dobie,
        source_name: "ATFX VIP SIGNALS".into(),
        curve_interval_ms: a.krzywa_ms,
        journal_path: journal.clone(),
        // 0 = zimny start = ścieżka parytetu; patrz `Args::rozgrzewka_h`.
        rozgrzewka_h: a.rozgrzewka_h,
        // puste poza `--drabinka` = zero nowego kodu w pętli
        drabinka: p.drabinka.clone(),
        drabinka_histereza_pct: a.drabinka_histereza_pct,
        // „Kredyt odliczony" ma znaczyć odliczony TAKŻE od progów —
        // inaczej 300 $ + 300 $ bonusu startuje od razu na szczeblu
        // 500, choć własnych pieniędzy jest 300. Ta sama reguła co
        // w `Engine::kredyt_skuteczny`: ręczna kwota nadpisuje,
        // 0 przy włączonym odliczaniu = automat z terminala (którego
        // w backteście nie ma, więc 0).
        drabinka_kredyt: if p.settings.odlicz_kredyt && !p.settings.credit_balance_separate {
            p.settings.kredyt_reczny.max(0.0)
        } else {
            0.0
        },
        // Reszta pól z domyślnych. `..Default::default()` jest tu
        // ŚWIADOME: `RunConfig` dostał w ciągu jednego dnia trzy nowe
        // pola (`formaty`, `pulapy`, `rozgrzewka_h`) i za każdym razem
        // przewracał wszystkie pięć wyliczeń wprost — w tym `lotto.exe`
        // w OSOBNYM workspace, którego `cargo check --workspace` nie
        // łapie. Domyślne wartości są ścieżką parytetu (zero pułapów,
        // zimny start), więc dopisanie pola nie może tu niczego zmienić
        // po cichu.
        ..Default::default()
    }
}

/// TRAIN i OOS dziedziczą cały konfigurator MAIN, w tym przyszłe pola.
/// Zmieniają wyłącznie zakres i sposób zapisu diagnostyki. Ograniczenie
/// drabinka + walk-forward nadal egzekwuje parser wariantów przed przebiegiem.
fn walk_forward_run_config(main: &RunConfig, from: i64, to: i64) -> RunConfig {
    let mut cfg = main.clone();
    cfg.from = from;
    cfg.to = to;
    cfg.journal_path = None;
    cfg.curve_interval_ms = main.curve_interval_ms.max(600_000);
    cfg
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        ticks: "data/ticks.bin".into(),
        signals: "data/signals.json".into(),
        signal_time_offset_min: 0,
        signal_time_offset_explicit: false,
        sim_limit_price_improvement: false,
        sim_new_pending_sl_next_tick: false,
        sim_native_swap_cash_digits: None,
        sim_trade_sessions: None,
        live_telegram_ingress: false,
        quick_tick_stride: 1,
        sim_price_digits: None,
        from: None,
        to: None,
        period: "all".into(),
        preset: None,
        sweep: None,
        balance: 200.0,
        daily_reset: false,
        auto_ea: false,
        flat_na_dobie: false,
        out: "out".into(),
        top: 20,
        quiet: false,
        walk_forward: 0,
        dump_trades: false,
        no_charts: false,
        summary_only: false,
        journal: None,
        reset_co: 0,
        zzn: false,
        zzn_max_dni: 7,
        krzywa_ms: 300_000,
        formaty: Vec::new(),
        pulapy: Vec::new(),
        rachunek: None,
        // ZIMNY START — domyślna ścieżki parytetu. Nie zmieniać bez przeliczenia
        // 1936,47 / 1178,02 / 2527,07.
        rozgrzewka_h: 0,
        drabinka: None,
        drabinka_histereza_pct: 0.0,
        presety_dir: "../PACKAGE/presets".into(),
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < v.len() {
        let k = v[i].as_str();
        let mut next = || -> Result<String> {
            i += 1;
            v.get(i)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("brak wartości dla {k}"))
        };
        match k {
            "--ticks" => a.ticks = next()?.into(),
            "--sim-limit-price-improvement" => a.sim_limit_price_improvement = true,
            "--sim-new-pending-sl-next-tick" => a.sim_new_pending_sl_next_tick = true,
            "--sim-trade-sessions" => {
                let path = PathBuf::from(next()?);
                a.sim_trade_sessions = Some(conduit_backtest::trade_sessions::TradeSessionProfile::load(&path)?);
            }
            "--sim-native-swap-cash-digits" => {
                let digits: u32 = next()?.parse()?;
                if digits > 8 { bail!("--sim-native-swap-cash-digits requires 0..=8"); }
                a.sim_native_swap_cash_digits = Some(digits);
            }
            "--live-telegram-ingress" => a.live_telegram_ingress = true,
            "--quick-tick-stride" => {
                a.quick_tick_stride = next()?.parse()?;
                if a.quick_tick_stride == 0 {
                    bail!("--quick-tick-stride musi być >= 1 (1 = dokładny backtest)");
                }
            }
            "--sim-price-digits" => a.sim_price_digits = Some(next()?.parse()?),
            "--signals" => a.signals = next()?.into(),
            "--signal-time-offset-min" => {
                a.signal_time_offset_min = next()?.parse()?;
                a.signal_time_offset_explicit = true;
            }
            "--journal" => a.journal = Some(next()?.into()),
            "--from" => a.from = Some(parse_date(&next()?)?),
            "--to" => a.to = Some(parse_date(&next()?)?),
            "--period" => a.period = next()?,
            "--preset" => a.preset = Some(next()?.into()),
            "--preset-format" => {
                let v = next()?;
                let Some((f, p)) = v.split_once('=') else {
                    bail!("--preset-format oczekuje FORMAT=plik.json, dostałem: {v}");
                };
                if f.trim().is_empty() {
                    bail!("--preset-format: pusta nazwa formatu w „{v}”");
                }
                a.formaty.push((f.trim().to_string(), p.into()));
            }
            "--pulapy" => {
                let v = next()?;
                let nr = a.pulapy.len() + 1;
                a.pulapy.push(parsuj_pulapy(&v, nr)?);
            }
            "--pulapy-plik" => {
                // Lista wariantów w pliku — wygodniejsza niż dwadzieścia
                // `--pulapy` w wierszu polecenia, a przy przemiataniu pułapów
                // dokładnie o to chodzi. Plik to TABLICA obiektów.
                let v = next()?;
                let txt = std::fs::read_to_string(&v)
                    .map_err(|e| anyhow::anyhow!("nie mogę wczytać {v}: {e}"))?;
                let lista: Vec<serde_json::Value> = serde_json::from_str(&txt)
                    .map_err(|e| anyhow::anyhow!("{v} ma być TABLICĄ obiektów pułapów: {e}"))?;
                for (i, x) in lista.into_iter().enumerate() {
                    let nr = a.pulapy.len() + 1;
                    a.pulapy.push(
                        parsuj_pulapy(&x.to_string(), nr)
                            .map_err(|e| anyhow::anyhow!("{v}, pozycja {}: {e}", i + 1))?,
                    );
                }
            }
            "--rachunek" => a.rachunek = Some(next()?.into()),
            "--rozgrzewka-h" => a.rozgrzewka_h = next()?.parse()?,
            "--drabinka" => a.drabinka = Some(next()?),
            "--drabinka-histereza-pct" => a.drabinka_histereza_pct = next()?.parse()?,
            "--presety-dir" => a.presety_dir = next()?.into(),
            "--sweep" => a.sweep = Some(next()?.into()),
            "--balance" => a.balance = next()?.parse()?,
            "--daily-reset" => a.daily_reset = true,
            "--auto-ea" => a.auto_ea = true,
            "--flat-na-dobie" => a.flat_na_dobie = true,
            "--reset-co" => a.reset_co = next()?.parse()?,
            "--zzn" => a.zzn = true,
            "--krzywa-ms" => a.krzywa_ms = next()?.parse()?,
            "--zzn-max-dni" => a.zzn_max_dni = next()?.parse()?,
            "--out" => a.out = next()?.into(),
            "--top" => a.top = next()?.parse()?,
            "--quiet" => a.quiet = true,
            "--dump-trades" => a.dump_trades = true,
            "--no-charts" => a.no_charts = true,
            "--summary-only" => a.summary_only = true,
            "--walk-forward" => a.walk_forward = next()?.parse()?,
            "-h" | "--help" => {
                println!(
                    "bt — backtest bota CONDUIT\n\n\
                     --ticks <plik>        domyślnie data/ticks.bin\n\
                     --sim-limit-price-improvement  jawny model lepszej ceny LIMIT z bieżącego ticka\n\
                     --sim-new-pending-sl-next-tick  SL nowego pendingu od następnego fizycznego ticka\n\
                     --sim-native-swap-cash-digits <0..8>  jawny model: swap w equity, w saldzie przy close\n\
                     \x20                     Precyzja waluty rachunku; brak flagi zachowuje legacy.\n\
                     --live-telegram-ingress  raw replay przechodzi przez te sama bramke odbiorcza\n\
                     \x20                     Telegrama co aplikacja LIVE (zwykly backtest: OFF).\n\
                     --quick-tick-stride <N>  PRZYBLIŻONE sito: bloki N ticków zachowują\n\
                     \x20                     first/last + min/max BID/ASK i granice zdarzeń.\n\
                     \x20                     N=1 (domyślne) = dokładny stary przebieg. Wyniku\n\
                     \x20                     N>1 NIE WOLNO koronować; finaliści muszą przejść N=1.\n\
                     --signals <plik>      domyślnie data/signals.json\n\
                     --signal-time-offset-min <N>  JAWNE przesunięcie całego strumienia\n\
                     \x20                     replayu o N minut (np. 180 dla UTC → UTC+3).\n\
                     \x20                     Domyślnie 0: GOD-X4 już przesuwa +180 min.\n\
                     \x20                     N != 0 wymaga efektywnego msg_clock_offset_ms=0;\n\
                     \x20                     CLI + niezerowy msg_offset presetu = BŁĄD. Kronika\n\
                     \x20                     received_at_ms też przesuwa się tylko wtedy,\n\
                     \x20                     gdy operator jawnie poda tę flagę.\n\
                     --period <zakres>     all | last-month | last-week | last-N-days\n\
                     --from RRRR-MM-DD     początek okna WŁĄCZNIE (nadpisuje --period)\n\
                     --to RRRR-MM-DD       koniec okna WYŁĄCZNIE — dzień podany tutaj\n\
                     \x20                     JUŻ NIE JEST mierzony. Mierzymy [--from, --to).\n\
                     \x20                     Jeden dzień 2026-08-06 to zatem:\n\
                     \x20                     --from 2026-08-06 --to 2026-08-07\n\
                     \x20                     (--from X --to X wybiera ZERO ticków i kończy\n\
                     \x20                     się błędem, nie tabelą zer)\n\
                     --preset <plik.json>  pojedyncza konfiguracja\n\
                     --sweep <katalog>     przebadaj wszystkie presety z katalogu\n\
                     --preset-format F=P   WIELE PRESETÓW NARAZ: format F gra presetem P.\n\
                     \x20                     Można podać kilka razy. Sygnał trafia do silnika\n\
                     \x20                     po polu `kanal` ze zbioru sygnałów; sygnał bez\n\
                     \x20                     pasującego formatu jest POLICZONY jako pominięty.\n\
                     --pulapy '<json>'     pułapy ponad presetami (limit skuteczny =\n\
                     \x20                     min(preset, pułap), 0 = brak pułapu). Można podać\n\
                     \x20                     kilka razy — każdy to osobny przebieg. Pole\n\
                     \x20                     „nazwa\" podpisuje wiersz w tabeli.\n\
                     \x20                     np. '{{\"nazwa\":\"P18\",\"maxPozycji\":18}}'\n\
                     --pulapy-plik <plik>  TABLICA takich obiektów w jednym pliku\n\
                     --rachunek <plik>     skąd wziąć pola RACHUNKU przy wielu formatach\n\
                     \x20                     (domyślnie: pierwszy --preset-format)\n\
                     --drabinka <spec>     DRABINKA po progach SALDA, np.\n\
                     \x20                     \"0=ZENONLY5,500=ZENONLY3,1000=SENTINEL-0,\n\
                     \x20                     1500=SENTINEL-0A\". Goła nazwa = łańcuch\n\
                     \x20                     wbudowany (formaty.rs), presety nóg\n\
                     \x20                     z --presety-dir. POJEDYNCZY preset:\n\
                     \x20                     \"preset:NAZWA\" (z --presety-dir) albo\n\
                     \x20                     \"plik:C:/…/PRESET.json\" — wtedy format nogi\n\
                     \x20                     bierze się z pola `format` presetu,\n\
                     \x20                     a pułapów globalnych NIE MA (sufitem są\n\
                     \x20                     własne limity presetu).\n\
                     \x20                     Przełączenie W TRAKCIE przebiegu adoptuje\n\
                     \x20                     koszyki (kontrakt jak restart na żywo).\n\
                     \x20                     Nie łączy się z --preset/--preset-format/\n\
                     \x20                     --pulapy/--sweep/--reset-co.\n\
                     --drabinka-histereza-pct <P>  schodź dopiero pod prog·(1−P/100), domyślnie 0\n\
                     --presety-dir <dir>   katalog presetów drabinki (domyślnie ../PACKAGE/presets)\n\
                     --rozgrzewka-h <N>    ile godzin ceny SPRZED --from dostaje silnik,\n\
                     \x20                     zanim zacznie handlować. 0 = ZIMNY START\n\
                     \x20                     i taka jest domyślna (ścieżka parytetu).\n\
                     \x20                     Okno krótsze niż regime_ma_hours BEZ tej\n\
                     \x20                     flagi mierzy ŚLEPY filtr reżimu.\n\
                     --balance <kwota>     kapitał startowy (domyślnie 200)\n\
                     --daily-reset         każdy dzień liczony osobno od kwoty startowej\n\
                     --auto-ea             tryb AUTO-EA (flaga Engine::tryb_auto_ea).\n\
                     \x20                     Dziś nie czyta jej ŻADNA oś, więc przebieg ma\n\
                     \x20                     wyjść co do centa jak bez niej — to jest punkt\n\
                     \x20                     (c) potrójnego kontraktu zera EA-CORE i po to\n\
                     \x20                     ta flaga istnieje.\n\
                     --reset-co <N>        OKNA PRZESUWANE o jeden dzień, długość N dni\n\
                     \x20                     handlowych, compounding WEWNĄTRZ okna.\n\
                     \x20                     N=1 to dokładnie --daily-reset (i tak jest\n\
                     \x20                     sprawdzane). Patrz NKI_ZZN.md\n\
                     --zzn                 Z ZACHOWANIEM NOCNYM: po granicy okna nie\n\
                     \x20                     bierzemy nowych sygnałów, ale koszyki z okna\n\
                     \x20                     dochodzą do naturalnego końca (tylko z --reset-co)\n\
                     --zzn-max-dni <N>     sufit ogona ZZN w dobach (domyślnie 7)\n\
                     --out <katalog>       gdzie zapisać wyniki i wykresy\n\
                     --no-charts           nie zapisuj wykresów SVG ani ich danych\n\
                     --summary-only        masowy sweep: pełne metryki zbiorcze, ale\n\
                     \x20                     bez transakcji/krzywych/koszyków per preset\n\
                     --top <n>             ile najlepszych pokazać w podsumowaniu
\n                     --walk-forward <dni>  walidacja poza próbą: ucz na N dniach, sprawdź na kolejnych N"
                );
                std::process::exit(0);
            }
            other => bail!("nieznany argument: {other}"),
        }
        i += 1;
    }
    Ok(a)
}

/// Mówi głośno, w których POLACH RACHUNKU presety formatów się różnią.
///
/// Rachunek jest jeden: nie da się mieć dwóch dźwigni, dwóch swapów ani dwóch
/// opóźnień realizacji naraz. Warstwa żywa rozstrzyga to dokumentem panelu;
/// backtest bierze pierwszy podany format (albo `--rachunek`). Cisza w tym
/// miejscu znaczyłaby, że drugi preset po cichu mierzy inny świat, niż mierzył
/// sam — i że nikt się o tym nie dowie.
///
/// # Ta sama funkcja co na żywo (EA-21, 24.08.2026)
///
/// Samo porównanie robi [`conduit_core::formaty::rozjazd_rachunku`] w rdzeniu,
/// bo warstwa żywa musi mówić DOKŁADNIE to samo. Do 24.08.2026 ta funkcja była
/// jedynym miejscem, które o rozjeździe w ogóle wspominało — pomiar widział
/// ostrzeżenie, konto nie widziało nic.
fn ostrzez_o_rozjezdzie_rachunku(fmt: &[FormatCfg], rach: &Settings) {
    let nogi: Vec<(&str, &Settings)> = fmt
        .iter()
        .map(|f| (f.format.as_str(), &f.settings))
        .collect();
    let lista = conduit_core::formaty::rozjazd_rachunku(&nogi, rach);
    if lista.is_empty() {
        return;
    }
    eprintln!(
        "⚠ POLA RACHUNKU RÓŻNIĄ SIĘ MIĘDZY PRESETAMI — rachunek jest JEDEN, więc\n\
         \x20 poniższe wartości zostały NADPISANE wartością rachunku (pierwszy\n\
         \x20 --preset-format albo --rachunek). Preset gra tu inaczej, niż grał sam:"
    );
    for z in &lista {
        eprintln!("{z}");
    }
    // Rozjazd, po którym noga ma MNIEJ ochrony, niż deklarował jej preset, nie
    // jest kosmetyką — to jest ta sama klasa co „preset niezwiązany gra
    // dokumentem". Na żywo zatrzymuje handel (`live.rs`); tutaj musi
    // przynajmniej dać się znaleźć `grep`-em w logu przebiegu.
    let ile = lista.iter().filter(|x| x.oslabia).count();
    if ile > 0 {
        eprintln!(
            "\x20 ⛔ {ile} z nich ZDEJMUJE NODZE BEZPIECZNIK, który niósł jej preset.\n\
             \x20    Ten przebieg mierzy INNY układ bezpieczników niż deklarują presety nóg."
        );
    }
    eprintln!();
}

/// SEKCJE „SYGNAŁY", „KOSZYKI" i „TRANSAKCJE" (Pakiet E5).
///
/// Trzy pytania, na które dotychczasowy wydruk nie odpowiadał:
/// * ile sygnałów kanału w ogóle zamieniło się w handel i co zjadło resztę —
///   a przy każdym filtrze ILE TO KOSZTOWAŁO w dolarach, nie w sztukach;
/// * jak wygląda KOSZYK jako całość (transakcja to szczebel, nie sygnał —
///   przy ośmiu szczeblach próbka jest ośmiokrotnie zawyżona);
/// * czy wynik nie stoi na jednej stronie rynku i ile zjadł sam spread.
fn wypisz_statystyki_e(r: &conduit_backtest::runner::RunResult) {
    let s = &r.metrics.stat_sygnalow;
    let l = &s.lejek;
    // Cisza przy pustym przebiegu jest zamierzona — drukowanie samych zer
    // sugerowałoby pomiar, którego nie było.
    if l.sygnaly_widziane == 0 && l.koszyki == 0 {
        return;
    }

    println!("\nSYGNAŁY (lejek):");
    println!(
        "  widziane {} → odrzucone {} → koszyki {} (bez wypełnienia {}) → z handlem {}   \
         WYKONANYCH: {:.1} %",
        l.sygnaly_widziane,
        l.odrzucone_razem,
        l.koszyki,
        l.koszyk_bez_fillu,
        l.koszyk_z_handlem,
        l.wykonanych_pct
    );
    if !l.koszt_filtrow.is_empty() {
        // WYCENA FILTRÓW. Model najprostszy z możliwych (0,01 lota, TP1 albo
        // SL) — nie po to, żeby przewidzieć zysk presetu bez filtra, tylko
        // żeby dało się PORÓWNAĆ kody między sobą wspólną miarką. Znak
        // dodatni znaczy „odrzucone sygnały były zyskowne", czyli filtr
        // KOSZTOWAŁ.
        let mut v: Vec<_> = l.koszt_filtrow.iter().collect();
        v.sort_by(|a, b| b.1.usd.abs().partial_cmp(&a.1.usd.abs()).unwrap());
        println!(
            "  koszt filtrów (0,01 lota, TP1 albo SL; + = filtr kosztował): {:+.2} $ \
             z {} wycenionych odrzutów",
            l.koszt_filtrow_usd, l.wycenionych
        );
        for (kod, k) in v.into_iter().take(5) {
            println!(
                "    {kod:<26} n={:<5} wycenione={:<5} TP1/SL {}/{}   {:+10.2} $",
                k.n, k.wycenione, k.tp1, k.sl, k.usd
            );
        }
    }

    let k = &s.koszyki;
    if k.total > 0 {
        println!("\nKOSZYKI (jednostka = wykonany sygnał):");
        println!(
            "  {} koszyków · z handlem {} · bez wypełnienia {} · W/L/BE {}/{}/{} ({:.1} % wygranych)",
            k.total, k.z_pozycjami, k.bez_fillu, k.win, k.loss, k.be, k.win_pct
        );
        println!(
            "  suma {:+.2} $ · średnia {:+.2} $ · mediana {:+.2} $ · TP1/TP2/TP3 {}/{}/{} \
             · mediana do TP1 {:.1} min",
            k.suma_usd, k.srednia_usd, k.mediana_usd, k.tp1, k.tp2, k.tp3, k.med_do_tp1_min
        );
        if k.fill_udzial > 0.0 {
            println!(
                "  wypełnionych szczebli: {:.1} % planu · rozkład {:?}",
                k.fill_udzial * 100.0,
                k.fill_hist
            );
        }
        if !k.powody.is_empty() {
            print!("  powody:");
            for (nazwa, kom) in &k.powody {
                print!("  {nazwa}={} ({:+.0} $)", kom.n, kom.usd);
            }
            println!();
        }
        // Godziny i dni tylko NIEPUSTE — 24 wiersze zer nie są informacją.
        let mut godz: Vec<(usize, &conduit_backtest::statystyki::StatOkna)> = k
            .godziny
            .iter()
            .enumerate()
            .filter(|(_, o)| o.n > 0)
            .collect();
        godz.sort_by(|a, b| b.1.usd.partial_cmp(&a.1.usd).unwrap());
        if !godz.is_empty() {
            print!("  najlepsze godziny:");
            for (h, o) in godz.iter().take(4) {
                print!("  {h:02}h n={} {:+.0} $ ({:.0} %)", o.n, o.usd, o.win_pct);
            }
            println!();
            print!("  najgorsze godziny:");
            for (h, o) in godz.iter().rev().take(4) {
                print!("  {h:02}h n={} {:+.0} $ ({:.0} %)", o.n, o.usd, o.win_pct);
            }
            println!();
        }
        const DNI: [&str; 7] = ["pn", "wt", "śr", "cz", "pt", "sb", "nd"];
        print!("  dni:");
        for (i, o) in k.dni.iter().enumerate() {
            if o.n > 0 {
                print!("  {}={} ({:+.0} $)", DNI[i], o.n, o.usd);
            }
        }
        println!();
    }

    let t = &s.transakcje;
    if t.buy.n + t.sell.n > 0 {
        println!("\nTRANSAKCJE (rozszerzenia):");
        println!(
            "  BUY  n={:<5} win {:.1} % · PF {:.2} · {:+.2} $ · swap {:+.2} $",
            t.buy.n, t.buy.win_pct, t.buy.profit_factor, t.buy.usd, t.buy.swap
        );
        println!(
            "  SELL n={:<5} win {:.1} % · PF {:.2} · {:+.2} $ · swap {:+.2} $",
            t.sell.n, t.sell.win_pct, t.sell.profit_factor, t.sell.usd, t.sell.swap
        );
        println!(
            "  remisy (BE) {} · win% bez remisów {:.1} · trzymanie p90 {:.1} min",
            r.metrics.bes, r.metrics.win_rate_bez_be, t.hold_p90_min
        );
        println!(
            "  spread zapłacony {:.2} $ · swap {:+.2} $ · SL i TP w tym samym ticku: {}",
            t.spread_usd, t.swap_usd, t.sl_tp_same_tick
        );
        if t.r_n > 0 {
            print!(
                "  R-multiple (jednostka = mediana ryzyka stopów), średnia {:.2}:",
                t.r_srednie
            );
            for (nazwa, n) in t.r_nazwy.iter().zip(t.r_hist.iter()) {
                print!("  {nazwa}={n}");
            }
            println!();
        }
    }
}

/// ROZBICIE PRZEBIEGU NA FORMATY.
///
/// Bez tego „portfel dwóch kanałów zarobił X" nie odpowiada na jedyne pytanie,
/// które się liczy: czy drugi format DOKŁADA, czy tylko zjada margines
/// pierwszemu. Zysk formatu liczy się z historii brokera po slocie numeru
/// koszyka, więc jest to ten sam dolar, który wchodzi do sumy.
fn wypisz_formaty(r: &conduit_backtest::runner::RunResult) {
    if r.formaty.is_empty() {
        return;
    }
    println!("\nROZBICIE NA FORMATY (zysk z historii brokera, po slocie koszyka):");
    println!(
        "  {:<10}{:<14}{:>7}{:>12}{:>9}{:>9}{:>9}",
        "format", "preset", "slot", "zysk $", "trejdy", "sygnały", "koszyki"
    );
    let mut suma = 0.0;
    for f in &r.formaty {
        suma += f.zysk;
        println!(
            "  {:<10}{:<14}{:>7}{:>12.2}{:>9}{:>9}{:>9}",
            trunc(&f.format, 9),
            trunc(&f.preset, 13),
            f.slot,
            f.zysk,
            f.trejdy,
            f.sygnaly,
            f.koszyki
        );
    }
    println!("  {:<31}{:>12.2}", "RAZEM zrealizowane", suma);
    if !r.bez_trasy.is_empty() {
        let mut v: Vec<_> = r.bez_trasy.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        print!("  ⚠ wiadomości BEZ PASUJĄCEGO FORMATU (pominięte):");
        for (k, n) in v {
            print!("  {k}={n}");
        }
        println!(
            "\n    To jest stan POPRAWNY tylko wtedy, gdy tak został ustawiony — \
             kanał bez\n    presetu jest nasłuchiwany, ale nie handlowany."
        );
    }
    if !r.szczeble.is_empty() {
        println!("\nDRABINKA — rozbicie na szczeble (zysk = transakcje ZAMKNIĘTE, gdy szczebel był aktywny):");
        println!(
            "  {:<16}{:>8}{:>12}{:>9}{:>9}",
            "łańcuch", "próg $", "zysk $", "trejdy", "wejścia"
        );
        let mut suma = 0.0;
        for s in &r.szczeble {
            suma += s.zysk;
            println!(
                "  {:<16}{:>8.0}{:>12.2}{:>9}{:>9}",
                trunc(&s.nazwa, 15),
                s.prog,
                s.zysk,
                s.trejdy,
                s.wejscia
            );
        }
        println!("  {:<24}{:>12.2}", "RAZEM zrealizowane", suma);
        println!("  przełączeń szczebla: {}", r.przelaczenia.len());
        // Pierwsze wejście na każdy WYŻSZY szczebel — to jest odpowiedź na
        // pytanie „po ilu dniach drabinka dojechała na górę".
        let mut widziane: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for p in &r.przelaczenia {
            if widziane.insert(p.na.as_str()) {
                let ms = p.ts.rem_euclid(86_400_000);
                println!(
                    "    pierwsze wejście na {:<16} {} {:02}:{:02}  (saldo {:.2})",
                    p.na,
                    fmt_ts(p.ts),
                    ms / 3_600_000,
                    ms % 3_600_000 / 60_000,
                    p.balance
                );
            }
        }
    }
}

fn load_presets(dir: &Path) -> Result<Vec<Preset>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let txt = std::fs::read_to_string(&p)?;
        match serde_json::from_str::<Preset>(&txt) {
            Ok(pr) => out.push(pr),
            Err(err) => eprintln!("pomijam {}: {err}", p.display()),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn main() -> Result<()> {
    // `--dump-settings` wypisuje KOMPLET ustawień domyślnych jako JSON i kończy.
    //
    // Istnieje po to, żeby dało się zapisywać presety PEŁNE. Preset niosący
    // tylko kilkadziesiąt pól jest niestabilny: `serde(default)` dopełnia resztę
    // bieżącymi wartościami domyślnymi, więc dopisanie do silnika nowego
    // ustawienia po cichu zmienia zachowanie każdego istniejącego presetu.
    // Zdarzyło się to realnie — ten sam preset dawał +288 $, a po dołożeniu
    // nowych pól i refaktorze silnika −83 $ przy potrojonej liczbie transakcji.
    // Porównywałem wtedy dwie różne konfiguracje pod jedną nazwą.
    if std::env::args().any(|x| x == "--dump-settings") {
        let s = conduit_core::settings::Settings::default();
        println!("{}", serde_json::to_string_pretty(&s)?);
        return Ok(());
    }

    let a = parse_args()?;

    if a.quick_tick_stride > 1 && a.reset_co > 0 {
        bail!("--quick-tick-stride N>1 nie obsługuje osobnego silnika --reset-co; użyj zwykłego sweepu albo N=1");
    }

    let mut ticks = TickData::open(&a.ticks)?;
    ticks.set_price_digits(a.sim_price_digits)?;
    let messages = load_messages_with_time_offset(&a.signals, a.signal_time_offset_min)?;
    std::fs::create_dir_all(&a.out)?;

    let t_first = ticks.first_ts();
    let t_last = ticks.last_ts();

    // ---------- okno czasowe ----------
    let (from, to) = match (a.from, a.to) {
        (Some(f), Some(t)) => (f, t),
        (Some(f), None) => (f, t_last + 1),
        (None, t) => {
            let end = t.unwrap_or(t_last + 1);
            let start = match a.period.as_str() {
                "all" => t_first,
                "last-month" => end - 30 * 86_400_000,
                "last-week" => end - 7 * 86_400_000,
                p if p.starts_with("last-") && p.ends_with("-days") => {
                    let n: i64 = p
                        .trim_start_matches("last-")
                        .trim_end_matches("-days")
                        .parse()?;
                    end - n * 86_400_000
                }
                other => bail!("nieznany okres: {other}"),
            };
            (start.max(t_first), end)
        }
    };

    if !a.quiet {
        println!(
            "ticki: {} ({} … {})   wiadomości: {}   offset sygnałów: {:+} min",
            ticks.len(),
            fmt_ts(t_first),
            fmt_ts(t_last),
            messages.len(),
            a.signal_time_offset_min,
        );
        println!(
            "okno:  {} … {}   kapitał {:.0} $   tryb: {}\n",
            fmt_ts(from),
            fmt_ts(to),
            a.balance,
            if a.daily_reset {
                "każdy dzień osobno"
            } else {
                "compounding"
            }
        );
    }

    // ---------- PUSTE OKNO ----------
    //
    // Do 17.08.2026 okno bez ani jednego ticka kończyło się CICHO: `run`
    // zwracał `Metrics::default()`, tabela pokazywała same zera, a wiersz
    // „equity końcowe 0.00 $ · różnica (pozycje otwarte) −400.00 $" czytało się
    // jak wyzerowane konto — czyli jak NAJGORSZY MOŻLIWY WYNIK — podczas gdy
    // znaczył „nie było czego mierzyć". Najczęstsza droga do tego stanu to
    // `--from X --to X`: `--to` jest granicą WYŁĄCZNĄ, więc takie okno wybiera
    // zero ticków. Pomyłka w wierszu polecenia ma się kończyć błędem, nie
    // liczbą, którą da się wkleić do raportu.
    //
    // Cichy zwrot w `run_with_progress` ZOSTAJE nietknięty i to jest celowe:
    // w trybie okien (`--reset-co`) puste podokno — weekend, święto, luka w
    // danych — jest normalne i nie może przerywać całego przemiatania. Głośne
    // jest CAŁE okno przebiegu, nie jego fragment; dlatego bramka stoi tutaj,
    // w CLI, a nie w bibliotece.
    let pustka = ticks.index_at(to).min(ticks.len()) <= ticks.index_at(from);
    if pustka {
        let podpowiedz = if ticks.is_empty() {
            format!(
                "\n  plik {} nie zawiera ANI JEDNEGO notowania",
                a.ticks.display()
            )
        } else if to <= from {
            format!(
                "\n  --to jest granicą WYŁĄCZNĄ: mierzone jest [--from, --to), \
                 więc `--to {}` nie obejmuje ani jednej sekundy dnia {}.\
                 \n  Jeden dzień {} mierzy się tak:  --from {} --to {}",
                fmt_ts(to),
                fmt_ts(from),
                fmt_ts(from),
                fmt_ts(from),
                fmt_ts(from + 86_400_000),
            )
        } else if from > t_last || to <= t_first {
            "\n  okno leży CAŁKOWICIE poza zakresem pliku ticków".to_string()
        } else {
            "\n  w tym zakresie plik ticków nie ma ani jednego notowania \
             (weekend? święto? luka w danych?)"
                .to_string()
        };
        bail!(
            "PUSTE OKNO: {} … {} nie zawiera ani jednego ticka — nie ma czego mierzyć.\
             \n  plik ticków: {} … {} ({} ticków){}",
            fmt_ts(from),
            fmt_ts(to),
            fmt_ts(t_first),
            fmt_ts(t_last),
            ticks.len(),
            podpowiedz,
        );
    }

    // ---------- co badamy ----------
    let presets: Vec<Preset> = if let Some(dir) = &a.sweep {
        load_presets(dir)?
    } else if let Some(f) = &a.preset {
        vec![serde_json::from_str::<Preset>(&std::fs::read_to_string(
            f,
        )?)?]
    } else {
        vec![Preset {
            name: "domyślne".into(),
            description: String::new(),
            // Preset bez pliku = ustawienia domyślne silnika, a te powstały
            // pod kanał ATFX i tylko pod nim były mierzone.
            format: "ATFX".into(),
            settings: Settings::default(),
            // brak pliku = brak warstwy EA (zmienna srodowiskowa nadal dziala)
            ea: None,
        }]
    };
    if presets.is_empty() && a.formaty.is_empty() {
        bail!("nie znalazłem żadnych presetów");
    }

    // ---------- CO JEST JEDNYM PRZEBIEGIEM ----------
    //
    // Do 03.08.2026 jeden przebieg = jeden preset. Odkąd `--preset-format`
    // pozwala grać kilkoma presetami naraz, oś przemiatania robi się DRUGA:
    // przy jednym łańcuchu presetów chcemy przemieść PUŁAPY. Wariant zbiera
    // jedno i drugie, więc równoległa pętla niżej nie musi wiedzieć, którą oś
    // akurat przemiatamy.
    let warianty: Vec<Wariant> = if let Some(spec) = &a.drabinka {
        if a.preset.is_some() || a.sweep.is_some() || !a.formaty.is_empty() {
            bail!(
                "--drabinka nie łączy się z --preset / --sweep / --preset-format: \
                 szczeble drabinki NOSZĄ własne łańcuchy nóg. Wybierz jedno."
            );
        }
        if !a.pulapy.is_empty() {
            bail!(
                "--drabinka nie łączy się z --pulapy: każdy szczebel nosi pułapy \
                 SWOJEGO łańcucha wbudowanego (formaty.rs). Chcesz inne — zmień łańcuch."
            );
        }
        if a.reset_co > 0 || a.walk_forward > 0 {
            bail!("--drabinka nie obsługuje --reset-co ani --walk-forward");
        }
        let szczeble = zbuduj_drabinke(spec, &a.presety_dir)?;
        // POLA RACHUNKU: jawnie z --rachunek albo z pierwszej nogi
        // NAJNIŻSZEGO szczebla. Rachunek jest JEDEN przez cały przebieg —
        // drabinka wymienia presety, nie konto.
        let rach = match &a.rachunek {
            Some(p) => wczytaj_preset(p)?.settings,
            None => szczeble[0]
                .formaty
                .first()
                .map(|f| f.settings.clone())
                .ok_or_else(|| anyhow::anyhow!("najniższy szczebel nie ma żadnej nogi"))?,
        };
        let wszystkie_nogi: Vec<FormatCfg> = szczeble
            .iter()
            .flat_map(|s| s.formaty.iter().cloned())
            .collect();
        ostrzez_o_rozjezdzie_rachunku(&wszystkie_nogi, &rach);
        let nazwa = szczeble
            .iter()
            .map(|s| format!("{}={}", s.prog, s.nazwa))
            .collect::<Vec<_>>()
            .join(" → ");
        vec![Wariant {
            nazwa,
            settings: rach,
            formaty: Vec::new(),
            pulapy: PulapyGlobalne::default(),
            format: "DRABINKA".into(),
            drabinka: szczeble,
            ea: None,
        }]
    } else if a.formaty.is_empty() {
        if !a.pulapy.is_empty() && a.pulapy.len() > 1 {
            bail!(
                "kilka wariantów --pulapy ma sens tylko z --preset-format \
                 (bez niego jest jeden silnik i nie ma nad czym trzymać sufitu)"
            );
        }
        let pul = a.pulapy.first().map(|(_, p)| p.clone()).unwrap_or_default();
        presets
            .iter()
            .map(|p| Wariant {
                nazwa: p.name.clone(),
                settings: p.settings.clone(),
                formaty: Vec::new(),
                pulapy: pul.clone(),
                format: p.format.clone(),
                drabinka: Vec::new(),
                ea: p.ea.as_ref().map(|v| v.to_string()),
            })
            .collect()
    } else {
        if a.preset.is_some() || a.sweep.is_some() {
            bail!(
                "--preset / --sweep nie łączą się z --preset-format: pierwszy opisuje \
                 CAŁY strumień, drugi rozdziela go między formaty. Wybierz jedno."
            );
        }
        // Presety formatów, w kolejności podanej w wierszu polecenia.
        let mut fmt: Vec<FormatCfg> = Vec::new();
        for (nazwa, plik) in &a.formaty {
            if fmt.iter().any(|f| &f.format == nazwa) {
                bail!("format „{nazwa}” podany dwa razy — kanał ma DOKŁADNIE JEDEN preset");
            }
            let p = wczytaj_preset(plik)?;
            fmt.push(FormatCfg {
                format: nazwa.clone(),
                preset: p.name.clone(),
                settings: p.settings,
            });
        }
        // POLA RACHUNKU. Jedno konto = jedna dźwignia, jeden swap, jedno
        // opóźnienie. Domyślnie bierzemy je z PIERWSZEGO podanego formatu
        // i mówimy głośno, gdzie presety się różnią — cicha wygrana
        // „ostatniego wczytanego" jest dokładnie tą klasą rozjazdu, przed
        // którą chroni `wielosilnik::POLA_RACHUNKU`.
        let rach = match &a.rachunek {
            Some(p) => wczytaj_preset(p)?.settings,
            None => fmt[0].settings.clone(),
        };
        ostrzez_o_rozjezdzie_rachunku(&fmt, &rach);
        let pulapy = if a.pulapy.is_empty() {
            vec![("bez pułapów".to_string(), PulapyGlobalne::default())]
        } else {
            a.pulapy.clone()
        };
        pulapy
            .into_iter()
            .map(|(nazwa, p)| Wariant {
                nazwa,
                settings: rach.clone(),
                formaty: fmt.clone(),
                pulapy: p,
                format: fmt
                    .iter()
                    .map(|f| f.format.as_str())
                    .collect::<Vec<_>>()
                    .join("+"),
                drabinka: Vec::new(),
                ea: None,
            })
            .collect()
    };
    if warianty.is_empty() {
        bail!("nie ma czego liczyć");
    }

    // P0: walidujemy WSZYSTKIE warianty przed pierwszym przebiegiem,
    // zerowaniem tabeli wyników i również przed odgałęzieniem --reset-co.
    // Ustawienia nóg nie sterują zegarem: runner i okna używają rachunku.
    let clock_contracts: Vec<ReplayClockContract> = warianty
        .iter()
        .map(|w| replay_clock_contract(a.signal_time_offset_min, &w.nazwa, &w.settings))
        .collect::<Result<_>>()?;
    if a.signal_time_offset_min != 0 {
        eprintln!(
            "UWAGA: jawna normalizacja zegara replayu: {:+} min z CLI; efektywny offset wiadomości wszystkich presetów/rachunków wynosi 0",
            a.signal_time_offset_min
        );
    }

    // Znaczniki after_offset obejmują WYŁĄCZNIE loader/CLI. Runner dodaje
    // następnie msg_offset() i exec_latency_ms; ich pełna suma jest jawna
    // per wariant. Manifest powstaje także dla --reset-co, nie tylko sweepu.
    let mut replay_manifest = serde_json::json!({
        "schema": "conduit.backtest.replay-clock.v2",
        "sim_limit_price_improvement": a.sim_limit_price_improvement,
        "sim_new_pending_sl_next_tick": a.sim_new_pending_sl_next_tick,
        "sim_new_pending_sl_next_tick_scope": "per immutable source-row observation; repeated bookkeeping of that row executes no market action, equal-time distinct rows still execute; not a full live-parity certificate",
        "sim_price_digits": a.sim_price_digits,
        "ticks_path": a.ticks.display().to_string(),
        "signals_path": a.signals.display().to_string(),
        "signal_time_offset_min": a.signal_time_offset_min,
        "signal_time_offset_explicit": a.signal_time_offset_explicit,
        "legacy_default_unchanged": a.signal_time_offset_min == 0,
        "loader_received_at_shifted_only_by_explicit_cli": true,
        "cli_and_preset_offsets_must_not_stack": true,
        "dispatch_formula": "input.ts + cli_signal_offset_ms + preset_msg_offset_ms + exec_latency_ms",
        "variants": clock_contracts,
        "messages": messages.len(),
        "message_first_ms_after_offset": messages.first().map(|m| m.ts),
        "message_last_ms_after_offset": messages.last().map(|m| m.ts),
        "ticks": ticks.len(),
        "tick_first_ms": t_first,
        "tick_last_ms": t_last,
        "window_from_ms": from,
        "window_to_exclusive_ms": to,
    });
    if a.quick_tick_stride > 1 {
        replay_manifest["approximate"] = serde_json::Value::Bool(true);
        replay_manifest["quick_backtest"] = serde_json::json!({
            "schema": "conduit.quick-backtest.v1",
            "requested_stride": a.quick_tick_stride,
            "method": "causal block extrema: first/last + min/max BID/ASK in original order",
            "forced_boundaries": ["effective message arrival", "broker trading day", "feed gap > 60 seconds"],
            "coronation_eligible": false,
            "warning": "APPROXIMATE SCREENING ONLY; every finalist must be rerun with --quick-tick-stride 1"
        });
        eprintln!(
            "\n╔══ QUICK BACKTEST N={} — WYNIK PRZYBLIŻONY ══\n\
             ║ tylko sito kandydatów; KORONACJA/WYDANIE ZABRONIONE\n\
             ║ finalistów uruchom ponownie z --quick-tick-stride 1\n\
             ╚══════════════════════════════════════════════\n",
            a.quick_tick_stride
        );
        std::fs::write(
            a.out.join("APPROXIMATE_DO_NOT_CROWN.txt"),
            format!(
                "QUICK BACKTEST N={} — APPROXIMATE SCREENING ONLY.\n\
                 Wynik nie kwalifikuje się do koronacji ani wydania.\n\
                 Każdy finalista wymaga niezależnego przebiegu N=1.\n",
                a.quick_tick_stride
            ),
        )?;
    }
    // Preserve the old manifest byte-contract when disabled.  When enabled,
    // record exactly which production Telegram gates the raw replay executed.
    if a.live_telegram_ingress {
        replay_manifest["live_telegram_ingress"] = serde_json::json!({
            "enabled": true,
            "content_dedup": "conduit_core::telegram_ingress::ContentMemory",
            "stale_entry_gate": "conduit_core::telegram_ingress::stale_entry_age_minutes",
            "stale_entry_max_age_min": 5.0,
            "stale_entry_scope": "fresh opening messages only; edits and management always pass",
            "telegram_publication_time_source": "telegram_published_ts or ts-latency_ms",
            "engine_timestamp_after_gate": "latest causal market tick, matching LIVE",
        });
    }
    // OFF does not add new fields to an old replay manifest.
    if let Some(profile) = &a.sim_trade_sessions {
        use sha2::{Digest, Sha256};
        let encoded = serde_json::to_vec(profile)?;
        replay_manifest["sim_trade_sessions"] = serde_json::json!({
            "profile": profile,
            "canonical_profile_sha256": format!("{:x}", Sha256::digest(encoded)),
            "scope": "broker-clock physical execution; quotes, messages, swap and mark-to-market are not filtered",
            "authority": "https://www.mql5.com/en/docs/marketinformation/symbolinfosessiontrade",
        });
    }
    if let Some(digits) = a.sim_native_swap_cash_digits {
        replay_manifest["sim_native_swap_cash"] = serde_json::json!({
            "currency_digits": digits,
            "scope": "explicit broker model: accrued swap in equity, settled to balance on closure; monetary rounding; independent of strategy and closed NET reporting",
            "native_qualification_scope": "see immutable native edge evidence; not a universal broker guarantee",
        });
    }
    std::fs::write(
        a.out.join("manifest_replay.json"),
        serde_json::to_string_pretty(&replay_manifest)?,
    )?;

    if !a.formaty.is_empty() && !a.quiet {
        println!("FORMATY W TYM PRZEBIEGU:");
        for f in &warianty[0].formaty {
            println!("  {:<10} → {}", f.format, f.preset);
        }
        println!(
            "  wariantów pułapów: {}   (routing po polu `kanal` sygnału)\n",
            warianty.len()
        );
    }

    // ---------- OKNA PRZESUWANE („n-ki") + ZZN ----------
    //
    // Osobna gałąź, nie kolejna flaga w głównej ścieżce. Główna ścieżka jest
    // bazą odniesienia całego projektu (1936 / 1178 / 2527) i nie ma powodu,
    // żeby przybywało w niej warunków. Zgodność jest pilnowana liczbą, nie
    // wspólnym kodem: `--reset-co 1` bez ZZN musi dać co do centa to samo, co
    // `--daily-reset`.
    if a.reset_co > 0 {
        // WIELOSILNIK W OKNACH — podpięty 07.08.2026.
        //
        // Do tego dnia okna chodziły jednym silnikiem, więc KAŻDY łańcuch
        // wielonogowy (SENTINEL-0/0A/0C/2) był w tym trybie niemierzalny —
        // a odsetek dodatnich okien i najgorsze okno to jedno z trzech
        // kryteriów oceny presetu. Teraz `okna.rs` buduje `Silniki` tą samą
        // drogą co `runner.rs` i routuje po polu `kanal`.
        //
        // Drabinka nadal nie wchodzi: przełączanie szczebla wymaga adopcji
        // koszyków przy wymianie ustawień, a to jest osobna praca. Blokada
        // niżej (`--drabinka` + `--reset-co`) zostaje świadomie.
        return tryb_okien(&a, &ticks, &messages, &warianty, from, to);
    }
    if a.zzn {
        bail!("--zzn działa tylko razem z --reset-co N");
    }

    // ---------- przebiegi (równolegle) ----------
    use rayon::prelude::*;
    // Dziennik zdarzeń zapisujemy WYŁĄCZNIE przy pojedynczym presecie.
    // Przy sweepie kilkadziesiąt wątków pisałoby do jednego pliku naraz —
    // linie by się przeplotły, a raport liczyłby cudze transakcje.
    let journal = if warianty.len() == 1 {
        a.journal.clone()
    } else {
        None
    };
    if a.journal.is_some() && journal.is_none() {
        eprintln!(
            "dziennik pominięty: --journal działa tylko dla JEDNEGO przebiegu (jest ich {})",
            warianty.len()
        );
    }
    // ---------- okienko postępu ----------
    //
    // Sweep po kilkudziesięciu presetach liczy się minutami i do tej pory
    // terminal milczał aż do końca. Tu wisi cała komunikacja z `postep.exe`:
    // wątek raportujący czyta liczniki atomowe i zapisuje plik stanu, a gdy
    // okno poprosi o przerwanie, ustawia flagę, którą widzi `ProgressFn`.
    //
    // Wątki liczące NIE dotykają dysku ani muteksów w gorącej pętli — robią
    // jeden `fetch_add` na 262 144 ticki. Pomiar czasu przebiegu zostaje więc
    // porównywalny z tym sprzed dołożenia postępu.
    let tickow_na_przebieg = ticks
        .index_at(to)
        .min(ticks.len())
        .saturating_sub(ticks.index_at(from)) as u64;
    let licznik = Arc::new(AtomicU64::new(0));
    let przerwij = Arc::new(AtomicBool::new(false));
    let gotowe = Arc::new(AtomicUsize::new(0));
    let koniec = Arc::new(AtomicBool::new(false));
    /// Najlepszy DOTĄD ukończony przebieg — wyłącznie do okna postępu.
    ///
    /// Komplet liczb, których PROMPT0 §4a wymaga przy każdym wariancie: sam
    /// zysk bez najgorszego dnia i bez najniższego equity nie mówi, czy to
    /// wynik do użycia, czy konto uratowane cudem.
    struct Najlepszy {
        nazwa: String,
        ocena: f64,
        zysk: f64,
        pf: f64,
        najgorszy_dzien: f64,
        dno_equity: f64,
        dni_plus: f64,
        trejdy: u32,
    }
    let najlepszy: Arc<Mutex<Option<Najlepszy>>> = Arc::new(Mutex::new(None));
    // Ile przebiegów WYZEROWAŁO konto. Jedyny próg bezwzględny (PROMPT0 §4a):
    // konto na zerze nie ma już jak odrobić, więc liczba na wierzchu.
    let zerowe = Arc::new(AtomicUsize::new(0));

    // Wyniki do przeglądarki monitora powstają W TRAKCIE sweepu, nie dopiero
    // po `par_iter().collect()`. Pisarz jest wspólny także w `--summary-only`:
    // ten tryb usuwa ciężkie szczegóły przebiegu, lecz pełne `Metrics` są małe
    // i właśnie ich potrzebuje selekcja live.
    let pisarz_czastkowy = Arc::new(Mutex::new(PisarzWynikowCzastkowych::nowy(
        &a.out,
        a.quick_tick_stride,
    )));
    if let Ok(mut p) = pisarz_czastkowy.lock() {
        if let Err(e) = p.wyzeruj() {
            eprintln!("  !! nie udało się wyzerować wyników cząstkowych: {e}");
            p.blad_zgloszony = true;
        }
    }

    // --- postęp POJEDYNCZEGO przebiegu (cienki pasek w oknie) ---
    //
    // Przebiegi lecą równolegle na wszystkich rdzeniach, więc „bieżący przebieg"
    // to w rzeczywistości kilkanaście przebiegów naraz. Trzymamy po jednym
    // slocie na wątek rayona: nazwa przebiegu, który ten wątek właśnie liczy,
    // i jego własny licznik ticków. Wątek raportujący czyta wszystkie sloty
    // i podaje oknu NAJDALEJ zaawansowany — podpisany tak, żeby było widać, że
    // to jeden z wielu, a nie „ten jedyny".
    //
    // Slot indeksujemy `rayon::current_thread_index()`, bo wątek roboczy liczy
    // dokładnie jeden przebieg naraz (`run_with_progress` jest zwykłą pętlą,
    // nie oddaje sterowania w środku). Jeden slot zapasowy na końcu obsługuje
    // przypadek, w którym praca wykona się poza pulą.
    let rdzenie = rayon::current_num_threads();
    let sloty: Arc<Vec<AtomicU64>> = Arc::new((0..=rdzenie).map(|_| AtomicU64::new(0)).collect());
    let nazwy_slotow: Arc<Mutex<Vec<String>>> =
        Arc::new(Mutex::new(vec![String::new(); rdzenie + 1]));

    let watek_postepu = {
        let licznik = licznik.clone();
        let przerwij = przerwij.clone();
        let gotowe = gotowe.clone();
        let najlepszy = najlepszy.clone();
        let zerowe = zerowe.clone();
        let sloty = sloty.clone();
        let nazwy_slotow = nazwy_slotow.clone();
        let koniec = koniec.clone();
        let ile = warianty.len();
        let nazwa = match (&a.sweep, &a.preset) {
            (Some(d), _) => format!("sweep {} ({ile} presetów)", d.display()),
            (None, Some(p)) => format!("preset {}", p.display()),
            _ => "ustawienia domyślne".to_string(),
        };
        let razem = (tickow_na_przebieg as f64) * ile as f64;
        let tryb = if a.daily_reset {
            "dzień po dniu"
        } else {
            "compounding"
        };
        // Account compounding and tick fidelity are independent dimensions.
        // Never infer exact replay merely from the compounding label.
        let tryb_obliczen = if a.quick_tick_stride > 1 { "quick" } else { "full" };
        let limit_lota = {
            let mut caps: Vec<f64> = warianty.iter().map(|p| p.settings.lot_max).collect();
            caps.sort_by(f64::total_cmp);
            caps.dedup();
            if caps.len() == 1 {
                if caps[0] == 0.0 { "bez limitu".to_string() }
                else { conduit_monitor::pl_liczba(caps[0], 2) }
            } else { "różny dla presetów".to_string() }
        };
        // Okno rozdziela trzy różne zakresy, których nie wolno zlewać w jedno
        // „okres": dane na dysku, zdarzenia eksportu i ticki faktycznie
        // mierzone. Koniec pomiaru pokazujemy jako OSTATNI REALNY TICK, nie
        // wyłączną granicę `to`, która przy `t_last + 1` wygląda jak inny czas.
        let i_od = ticks.index_at(from).min(ticks.len());
        let i_do = ticks.index_at(to).min(ticks.len());
        let zakres_mierzony = if i_od < i_do {
            format!(
                "{} … {} (ostatni tick)",
                data_czas_ludzki(ticks.ts(i_od)),
                data_czas_ludzki(ticks.ts(i_do - 1))
            )
        } else {
            "—".to_string()
        };
        let dane_tickow = format!(
            "{} … {}",
            data_czas_ludzki(t_first),
            data_czas_ludzki(t_last)
        );
        let zdarzenia_w_oknie: Vec<i64> = messages
            .iter()
            .filter(|m| m.ts >= from && m.ts < to)
            .map(|m| m.ts)
            .collect();
        let zakres_zdarzen = match (zdarzenia_w_oknie.first(), zdarzenia_w_oknie.last()) {
            (Some(a), Some(b)) => format!(
                "{} … {} · {} zdarzeń",
                data_czas_ludzki(*a),
                data_czas_ludzki(*b),
                zdarzenia_w_oknie.len()
            ),
            _ => "brak zdarzeń w mierzonym zakresie".to_string(),
        };
        // `historia_z_tickow` skanuje ten zapas, żeby weekend i przerwy
        // dobowe nie zamieniły żądanych N godzin historii w kilkanaście próbek.
        // To jest WYŁĄCZNIE stan rynku/SR; handel nadal zaczyna się w `from`.
        let rozgrzewka = if a.rozgrzewka_h == 0 {
            "0 h — zimny start".to_string()
        } else {
            let zapas_h = a.rozgrzewka_h.saturating_mul(2).saturating_add(120);
            let skan_od = from
                .saturating_sub((zapas_h as i64).saturating_mul(3_600_000))
                .max(t_first);
            format!(
                "{} h historii · skan {} … start · bez handlu",
                a.rozgrzewka_h,
                data_czas_ludzki(skan_od)
            )
        };
        let kapital = format!("{} $", conduit_monitor::pl_liczba(a.balance, 0));
        let wykresy = if a.no_charts || a.summary_only {
            "nie"
        } else {
            "tak"
        };
        // Gdzie wylądują liczby. Bez tego po zakończeniu sweepu trzeba szukać
        // po katalogach `out_*`, których jest ponad sto.
        let plik_wynikow = a
            .out
            .join(format!(
                "wyniki_{}.json",
                if a.quick_tick_stride > 1 {
                    format!("APPROX_N{}_{}", a.quick_tick_stride,
                        if a.daily_reset { "daily" } else { "compound" })
                } else {
                    if a.daily_reset { "daily" } else { "compound" }.to_string()
                }
            ))
            .display()
            .to_string();
        // Katalog wyników przekazujemy oknu, żeby umiało pokazać statystyki
        // presetów JUŻ POLICZONYCH, nie czekając na koniec przemiatania.
        let kat_wynikow = a.out.clone();
        std::thread::spawn(move || {
            // znak dopisujemy sami — przy zyskach krążących wokół zera plus
            // niesie tyle samo informacji co minus
            let zn = |x: f64, d: usize| {
                let s = conduit_monitor::pl_liczba(x, d);
                if x >= 0.0 {
                    format!("+{s}")
                } else {
                    s
                }
            };
            let mut r = conduit_monitor::Raport::nowy(nazwa, conduit_monitor::BACKTEST);
            r.calosc(razem, "ticków", "ticków/s");
            r.katalog_wynikow(&kat_wynikow);
            loop {
                let g = gotowe.load(Ordering::Relaxed);

                // --- co się liczy W TEJ CHWILI (może być kilkanaście naraz) ---
                let mut trwa: Vec<(String, f64)> = Vec::new();
                if let Ok(nz) = nazwy_slotow.lock() {
                    for (i, n) in nz.iter().enumerate() {
                        if n.is_empty() {
                            continue;
                        }
                        let u = if tickow_na_przebieg > 0 {
                            (sloty[i].load(Ordering::Relaxed) as f64 / tickow_na_przebieg as f64)
                                .clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        trwa.push((n.clone(), u));
                    }
                }
                // najdalej zaawansowany na początek
                trwa.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
                let k = trwa.len();

                // Podpis głównego paska mówi PRAWDĘ o równoległości: „przebieg
                // 19/22" sugerowałby, że liczy się jeden, a liczą się wszystkie
                // wolne rdzenie naraz.
                let co = match trwa.first() {
                    Some((n, _)) if k > 1 => format!("gotowe {g}/{ile} · liczone {k} naraz — {n}"),
                    Some((n, _)) => format!("gotowe {g}/{ile} · aktualnie liczone — {n}"),
                    None => format!("gotowe {g}/{ile}"),
                };
                // Cienki pasek: JEDEN przebieg, ten najdalej zaawansowany —
                // czyli ten, który skończy się najbliżej. Podpis wprost mówi,
                // ilu innych nie widać i jak daleko jest najsłabszy z nich;
                // udawanie, że ten procent opisuje „bieżący preset", byłoby
                // precyzją, której tu nie ma.
                match trwa.first() {
                    Some((n, u)) if k > 1 => {
                        let najslabszy = trwa.last().map(|x| x.1).unwrap_or(*u);
                        r.biezacy(
                            *u,
                            format!(
                                "aktualnie liczone: {} · najdalej z {k} naraz (najsłabszy {} %)",
                                trunc(n, 22),
                                conduit_monitor::pl_liczba(najslabszy * 100.0, 0)
                            ),
                        );
                    }
                    Some((n, u)) => r.biezacy(*u, format!("aktualnie liczone: {}", trunc(n, 30))),
                    None => r.biezacy(0.0, ""),
                }

                let mut st = conduit_monitor::Statystyki::nowe();
                st.dodaj(
                    "przebiegi",
                    if k > 0 {
                        format!("{g} / {ile} · {k} naraz")
                    } else {
                        format!("{g} / {ile}")
                    },
                );
                if let Ok(n) = najlepszy.lock() {
                    if let Some(b) = n.as_ref() {
                        st.dodaj("najlepszy z ukończonych", format!("{} · {g}/{ile} gotowych", b.nazwa));
                        st.dodaj(
                            "wynik tego presetu",
                            format!(
                                "{} $ · PF {} · {} zamknięć",
                                zn(b.zysk, 0),
                                if b.pf.is_finite() {
                                    conduit_monitor::pl_liczba(b.pf, 2)
                                } else {
                                    "∞".into()
                                },
                                b.trejdy
                            ),
                        );
                        // §4a: zysk bez najgorszego dnia i bez dna equity nie
                        // wystarcza do żadnej decyzji
                        st.dodaj(
                            "ryzyko tego presetu",
                            format!(
                                "najgorszy dzień {} $ · dno {} $ · dni+ {} %",
                                zn(b.najgorszy_dzien, 0),
                                conduit_monitor::pl_liczba(b.dno_equity, 0),
                                conduit_monitor::pl_liczba(b.dni_plus, 0)
                            ),
                        );
                    }
                }
                if ile > 1 {
                    st.dodaj("pozostało", format!("{} presetów", ile.saturating_sub(g)));
                    if g > 0 {
                        let z = zerowe.load(Ordering::Relaxed);
                        st.dodaj(
                            "wyzerowały konto",
                            if z == 0 {
                                format!("0 z {g} — żaden")
                            } else {
                                format!("{z} z {g}")
                            },
                        );
                    }
                }
                st.dodaj("zakres mierzony", zakres_mierzony.clone());
                st.dodaj("zdarzenia eksportu", zakres_zdarzen.clone());
                st.dodaj("pełne dane ticków", dane_tickow.clone());
                st.dodaj("rozgrzewka rynku/SR", rozgrzewka.clone());
                st.dodaj("kapitał startowy", kapital.clone());
                st.dodaj(
                    "ticków na przebieg",
                    conduit_monitor::pl_duza(tickow_na_przebieg as f64),
                );
                st.dodaj("rdzenie", rdzenie.to_string());
                st.dodaj("tryb_obliczen", tryb_obliczen);
                st.dodaj("max_lot", limit_lota.clone());
                st.dodaj("tryb", tryb.to_string());
                st.dodaj("wykresy", wykresy.to_string());
                st.dodaj("wyniki trafią do", plik_wynikow.clone());
                if !r.postep(licznik.load(Ordering::Relaxed) as f64, co, st) {
                    przerwij.store(true, Ordering::Relaxed);
                }
                if koniec.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
            r.zakoncz();
        })
    };

    let results: Vec<(String, conduit_backtest::runner::RunResult)> = warianty
        .par_iter()
        .map(|p| {
            // slot tego wątku roboczego — nazwa przebiegu i jego własny licznik
            let slot = rayon::current_thread_index()
                .unwrap_or(rdzenie)
                .min(rdzenie);
            sloty[slot].store(0, Ordering::Relaxed);
            if let Ok(mut nz) = nazwy_slotow.lock() {
                nz[slot] = p.nazwa.clone();
            }
            let cfg = main_run_config(&a, p, from, to, journal.clone());
            // Sygnalizator postępu. Zwrócenie `false` przerywa przebieg, a
            // runner oddaje wynik z tego, co zdążył policzyć. Dwa `fetch_add`
            // zamiast jednego, oba bez kontencji, raz na 262 144 ticki — czas
            // przebiegu zostaje porównywalny.
            let sygnal = |d: u64| -> bool {
                licznik.fetch_add(d, Ordering::Relaxed);
                sloty[slot].fetch_add(d, Ordering::Relaxed);
                !przerwij.load(Ordering::Relaxed)
            };
            let mut r = run_with_progress(&ticks, &messages, &cfg, Some(&sygnal));
            match publishable_run_metrics(&r, &format!("run {}", p.nazwa)) {
              Err(error) => {
                eprintln!("{error}");
                przerwij.store(true, Ordering::Relaxed);
              }
              Ok(Some(_)) => {
                if r.metrics.blown {
                    zerowe.fetch_add(1, Ordering::Relaxed);
                }
                let oc = score(&r.metrics);
                if let Ok(mut n) = najlepszy.lock() {
                    if n.as_ref().map(|x| oc > x.ocena).unwrap_or(true) {
                        *n = Some(Najlepszy {
                            nazwa: p.nazwa.clone(),
                            ocena: oc,
                            zysk: r.metrics.total_profit,
                            pf: r.metrics.profit_factor,
                            najgorszy_dzien: r.metrics.worst_day,
                            dno_equity: r.metrics.min_equity,
                            dni_plus: r.metrics.win_days_pct,
                            trejdy: r.metrics.trades,
                        });
                    }
                }
                // Only complete, reconciled rows may appear in the live
                // ranking. The final HOLD check below must not be the first.
                if let Ok(mut zapis) = pisarz_czastkowy.lock() {
                    zapis.dodaj_z_raportem(p.nazwa.clone(), r.metrics.clone());
                }
              }
              Ok(None) => {} // explicitly cancelled, not a completed ranking row
            }
            // Publikacja następuje PRZED zwiększeniem `gotowe`, więc licznik
            // w oknie nigdy nie obiecuje wyniku, którego nie ma jeszcze w
            // `wyniki_czastkowe.json`. Jeden mutex obejmuje mapę i atomową
            // podmianę pliku; równolegle kończące się presety nie mają wyścigu.
            if let Ok(mut nz) = nazwy_slotow.lock() {
                nz[slot].clear();
            }
            gotowe.fetch_add(1, Ordering::Relaxed);
            // Przy tysiącach wariantów szczegóły potrafią zajmować wiele GB
            // RAM, chociaż ranking czyta wyłącznie gotowe `metrics`. Czyścimy
            // je DOPIERO po zakończeniu przebiegu i wyliczeniu metryk, więc
            // wynik oraz każda decyzja silnika pozostają bitowo te same.
            // Finalistę uruchamia się ponownie bez `--summary-only`, aby dostać
            // pełny audyt transakcji, koszyków i krzywych.
            if a.summary_only {
                r.trades.clear();
                r.baskets_dump.clear();
                r.equity_curve.clear();
                r.balance_curve.clear();
                r.daily.clear();
            }
            (p.nazwa.clone(), r)
        })
        .collect();

    koniec.store(true, Ordering::Relaxed);
    let _ = watek_postepu.join();
    for (name,result) in &results {
        require_reconciled_run(result, &format!("run {name}"))?;
    }
    let przerwane = results.iter().any(|(_, r)| r.cancelled);
    if przerwane {
        let zrobiono = licznik.load(Ordering::Relaxed) as f64;
        println!(
            "\n╔══ PRZERWANO NA ŻĄDANIE ══\n\
             ║ policzono {} z {} ticków ({} %)\n\
             ║ wyniki poniżej są CZĄSTKOWE — wolno je oglądać, nie wolno\n\
             ║ porównywać z pełnymi przebiegami\n\
             ╚══════════════════════════",
            conduit_monitor::pl_duza(zrobiono),
            conduit_monitor::pl_duza((tickow_na_przebieg as f64) * warianty.len() as f64),
            conduit_monitor::pl_liczba(
                zrobiono / ((tickow_na_przebieg as f64) * warianty.len() as f64).max(1.0) * 100.0,
                1
            )
        );
    }

    if a.quick_tick_stride > 1 {
        let observed: Vec<f64> = results
            .iter()
            .filter_map(|(_, result)| result.approximation.as_ref().map(|info| info.observed_pct))
            .collect();
        if !observed.is_empty() {
            let min_pct = observed.iter().copied().fold(f64::INFINITY, f64::min);
            let max_pct = observed.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            println!(
                "QUICK N={}: zachowano {:.2}%..{:.2}% surowych ticków (first/last + extrema BID/ASK; granice wiadomości/dnia). WYNIK PRZYBLIŻONY — NIE DO KORONACJI.",
                a.quick_tick_stride, min_pct, max_pct
            );
        }
    }

    if let Some(jp) = &journal {
        let linie: u64 = results.iter().map(|(_, r)| r.journal_lines).sum();
        if linie > 0 {
            println!("\ndziennik zdarzeń: {linie} linii → {}", jp.display());
            println!("analiza:           loganaliza {}", jp.display());
        }
    }

    // ---------- tabela ----------
    let mut rows: Vec<_> = results.iter().collect();
    rows.sort_by(|a, b| {
        let ra = score(&a.1.metrics);
        let rb = score(&b.1.metrics);
        rb.partial_cmp(&ra).unwrap()
    });

    // KOLUMNY PAKIETU E DOKLEJONE Z PRAWEJ, przed znacznikiem „!".
    // Istniejące kolumny zostają na swoich miejscach i w swoich szerokościach —
    // po tej tabeli porównuje się przebiegi z całego archiwum i przesunięcie
    // choćby jednej z nich unieważniałoby te porównania.
    // `WYK%` = koszyki, które naprawdę zahandlowały, przez sygnały widziane
    // (cel właściciela: 100 %); `BE%` = udział koszyków na zero;
    // `FILTRY $` = wycena odrzutów, dodatnia znaczy „filtry kosztowały".
    println!(
        "{:<30}{:>9}{:>8}{:>9}{:>10}{:>7}{:>7}{:>7}{:>7}{:>7}{:>6}{:>10}{:>6}",
        "konfiguracja",
        "zysk $",
        "/dzień",
        "maxDD $",
        "RYZYKO $",
        "RYZ%",
        "PF",
        "dni+%",
        "trejdy",
        "WYK%",
        "BE%",
        "FILTRY $",
        "!"
    );
    println!("{}", "─".repeat(137));
    for (name, r) in rows.iter().take(a.top) {
        let m = &r.metrics;
        let s = &r.metrics.stat_sygnalow;
        let be_pct = if s.koszyki.z_pozycjami > 0 {
            s.koszyki.be as f64 / s.koszyki.z_pozycjami as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "{:<30}{:>9.0}{:>8.2}{:>9.0}{:>10.0}{:>7.0}{:>7}{:>7.0}{:>7}{:>7.0}{:>6.0}{:>10.0}{:>6}",
            trunc(name, 29),
            m.total_profit,
            m.avg_per_day,
            m.max_dd_abs,
            m.max_open_risk,
            m.max_open_risk_pct,
            if m.profit_factor.is_finite() { format!("{:.2}", m.profit_factor) } else { "inf".into() },
            m.win_days_pct,
            m.trades,
            s.lejek.wykonanych_pct,
            be_pct,
            s.lejek.koszt_filtrow_usd,
            if m.blown { "ZERO" } else { "" }
        );
    }

    // ---------- WALIDACJA WALK-FORWARD ----------
    // Wybór najlepszego presetu na całym oknie to selekcja W PRÓBIE i zawsze
    // wygląda dobrze. Tu robimy uczciwie: na oknie treningowym wybieramy
    // zwycięzcę, a wynik liczymy na NASTĘPNYM, nietkniętym oknie.
    if a.walk_forward > 0 && warianty.len() > 1 && !przerwane {
        let win = a.walk_forward * 86_400_000;
        let mut t = from;
        let mut oos_total = 0.0;
        let mut oos_rows: Vec<(String, f64, f64)> = Vec::new();
        println!(
            "\n╔══ WALK-FORWARD: uczymy na {} dniach, sprawdzamy na kolejnych {} ══",
            a.walk_forward, a.walk_forward
        );

        while t + 2 * win <= to {
            let (tr_a, tr_b) = (t, t + win);
            let (te_a, te_b) = (t + win, t + 2 * win);

            let training = warianty
                .par_iter()
                .map(|p| {
                    let base = main_run_config(&a, p, from, to, journal.clone());
                    let cfg = walk_forward_run_config(&base, tr_a, tr_b);
                    let result = run(&ticks, &messages, &cfg);
                    Ok((
                        p.nazwa.clone(),
                        score_reconciled_run(&result, &format!("walk-forward training {}", p.nazwa))?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let best = training.into_iter().max_by(|x, y| x.1.partial_cmp(&y.1).unwrap());

            if let Some((name, _)) = best {
                let p = warianty.iter().find(|p| p.nazwa == name).unwrap();
                let base = main_run_config(&a, p, from, to, journal.clone());
                let cfg = walk_forward_run_config(&base, te_a, te_b);
                let r = run(&ticks, &messages, &cfg);
                require_reconciled_run(&r, &format!("walk-forward out-of-sample {name}"))?;
                let m = &r.metrics;
                println!(
                    "  {} → {}  wybrano: {:<24} wynik POZA PRÓBĄ: {:>9.0} $  DD {:>5.0} $  ryzyko {:>3.0}%  dni+ {:>3.0}%",
                    fmt_ts(te_a), fmt_ts(te_b), trunc(&name, 24),
                    m.total_profit, m.max_dd_abs, m.max_open_risk_pct, m.win_days_pct
                );
                oos_total += m.total_profit;
                oos_rows.push((name, m.total_profit, m.max_dd_abs));
            }
            t += win;
        }
        println!(
            "╚══ suma poza próbą: {oos_total:+.0} $ w {} oknach",
            oos_rows.len()
        );
    }

    // ---------- diagnostyka pojedynczego przebiegu ----------
    // Kontrola wiarygodności: rozkład powodów zamknięcia i to, ile pozycji
    // zostało OTWARTYCH na koniec okna. Otwarte pozycje w stracie potrafią
    // udawać świetny wynik, dopóki nie zostaną domknięte.
    if results.len() == 1 && !a.summary_only {
        let r = &results[0].1;
        let mut by: HashMap<String, (u32, f64)> = HashMap::new();
        for t in &r.trades {
            let e = by.entry(format!("{:?}", t.reason)).or_insert((0, 0.0));
            e.0 += 1;
            e.1 += t.profit;
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        println!("\npowody zamknięć:");
        for (k, (n, p)) in v {
            println!("  {k:<14}{n:>6}   {p:>10.2} $");
        }
        let realized: f64 = r.trades.iter().map(|t| t.profit).sum();
        println!(
            "\nzrealizowane: {realized:.2} $   equity końcowe: {:.2} $   różnica (pozycje otwarte): {:.2} $",
            r.metrics.end_equity,
            r.metrics.end_equity - a.balance - realized
        );
        println!(
            "min. equity w całym przebiegu: {:.2} $   odrzuconych SL przez brokera: {}   limit→rynek: {}",
            r.metrics.min_equity, r.metrics.rejected_stops, r.metrics.market_instead_of_limit
        );
        // Swap jest kosztem NIEWIDOCZNYM w rozkładzie powodów zamknięć, a przy
        // 92,8 % udziale kupna — asymetrycznym. Bez tej linijki nie da się
        // odróżnić „swap nic nie zmienia" od „swap się nie nalicza".
        println!(
            "punkty swapowe: {:.2} $   stop outów brokera: {}",
            r.swap_paid, r.stop_outs
        );
        println!(
            "wypełnień SKASOWANYCH z braku marginesu: {}",
            r.metrics.rejected_no_money
        );
        // POZIOM MARGINESU — „prawie-śmierci". Stop-out jest rzadki, więc sam
        // licznik trupów nie ma mocy statystycznej; zejścia pod 200/150/100 %
        // to ten sam mechanizm, tylko słabszy, i jest ich dziesiątki.
        if r.metrics.min_margin_level.is_finite() {
            println!(
                "POZIOM MARGINESU: min {:.1} % · ticków pod 200/150/100 %: {}/{}/{} ·                  szczyt wolumenu {:.2} lota (margines {:.2} $)",
                r.metrics.min_margin_level,
                r.metrics.ml_pod_200,
                r.metrics.ml_pod_150,
                r.metrics.ml_pod_100,
                r.metrics.max_open_volume,
                r.metrics.max_open_margin
            );
        }
        // Cisza przy WYŁĄCZONEJ regule (`expo_cap_pct = 0`) jest zamierzona:
        // licznik jest wtedy martwy i drukowanie zer sugerowałoby, że reguła
        // działa i nic nie robi. Przy włączonej linijka mówi OBIE rzeczy naraz:
        // jak wysoko sięgnęła ekspozycja i ile reguła zdążyła zdjąć.
        if r.metrics.expo_max_pct > 0.0 || r.metrics.expo_zdarzen > 0 {
            println!(
                "EKSPOZYCJA: max {:.1} % equity · zadziałań {} · skasowanych szczebli {} \
                 ({:.2} lota) · domkniętych pozycji {} · niedosyt {}",
                r.metrics.expo_max_pct,
                r.metrics.expo_zdarzen,
                r.metrics.expo_pend_skasowane,
                r.metrics.expo_lotow,
                r.metrics.expo_poz_domkniete,
                r.metrics.expo_niedosyt
            );
        }
        // Zero przy WYŁĄCZONYM `sim_validate_pending_stops` nic nie znaczy —
        // licznik jest wtedy martwy. Przy włączonym mówi, ile szczebli-widm
        // preset zawdzięcza odmowie brokera.
        println!(
            "pendingów odrzuconych 10016 (SL/TP przy cenie zlecenia): {}",
            r.metrics.rejected_pending_stops
        );
        if !r.metrics.odrzuty.is_empty() {
            let mut v: Vec<_> = r.metrics.odrzuty.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1));
            print!("odrzucone sygnały:");
            for (k, n) in v {
                print!("  {k}={n}");
            }
            println!();
        }
        wypisz_formaty(r);
        wypisz_statystyki_e(r);
        if a.dump_trades {
            let path = a.out.join("transakcje.json");
            std::fs::write(&path, serde_json::to_string(&closed_trade_raw_export(&r.trades)?)?)?;
            println!(
                "zapisano {} transakcji → {}",
                r.trades.len(),
                path.display()
            );
            let bp = a.out.join("koszyki.json");
            std::fs::write(&bp, serde_json::to_string(&r.baskets_dump)?)?;
            println!(
                "zapisano {} koszyków → {}",
                r.baskets_dump.len(),
                bp.display()
            );
            // Dni z SILNIKA, nie odtwarzane z `close_ts`. Odtwarzanie myli
            // się o pozycje przechodzące przez północ i o dni bez transakcji,
            // a mediana dnia jest tu wielkością ocenianą — nie wolno jej
            // liczyć z rekonstrukcji.
            let dp = a.out.join("dni.json");
            std::fs::write(&dp, serde_json::to_string(&r.daily)?)?;
            println!("zapisano {} dni → {}", r.daily.len(), dp.display());
        }
    }

    // ---------- zapis ----------
    let tag = if a.quick_tick_stride > 1 {
        format!("APPROX_N{}_{}", a.quick_tick_stride,
            if a.daily_reset { "daily" } else { "compound" })
    } else {
        if a.daily_reset { "daily" } else { "compound" }.to_string()
    };
    // nazwa presetu → format sygnałów; `results` niesie samą nazwę
    let formaty: HashMap<&str, String> = warianty
        .iter()
        .map(|p| (p.nazwa.as_str(), p.format.clone()))
        .collect();
    let mut summary = HashMap::new();
    for (name, r) in &results {
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if (a.dump_trades || !a.no_charts) && !a.summary_only {
            if !a.no_charts {
                let svg = chart::run_chart(
                    &format!("{name} · {} … {} · {tag}", fmt_ts(from), fmt_ts(to)),
                    &r.metrics,
                    &r.equity_curve,
                    &r.daily,
                );
                std::fs::write(a.out.join(format!("{safe}_{tag}.svg")), svg)?;
            }
            // A multi-preset coronation needs the same primary daily and
            // basket evidence as a single run, even when SVG output is off.
            if a.dump_trades {
                std::fs::write(a.out.join(format!("{safe}_{tag}_dni.json")),
                    serde_json::to_string(&r.daily)?)?;
                std::fs::write(a.out.join(format!("{safe}_{tag}_koszyki.json")),
                    serde_json::to_string(&r.baskets_dump)?)?;
            }

            // SUROWE DANE WYKRESU, nie tylko obrazek.
            //
            // SVG jest do oglądania; do analizy trzeba liczb. Bez tego pliku
            // jedyną drogą do wartości dziennych było odczytywanie WSPÓŁRZĘDNYCH
            // PIKSELI z krzywej — próbowaliśmy 31.07 i pierwsza kalibracja
            // pomyliła saldo startowe o rząd wielkości (4 685 $ zamiast 300 $),
            // bo etykieta osi to linia bazowa pisma, nie kreska siatki.
            //
            // Tu wychodzi dokładnie to, co policzył silnik: pełna krzywa
            // kapitału ze znacznikami czasu i statystyki każdego dnia.
            let dane = serde_json::json!({
                "preset": name,
                "od": fmt_ts(from),
                "do": fmt_ts(to),
                "tryb": tag,
                "saldo_start": r.metrics.start_balance,
                // CO ILE ms PRÓBKOWANA JEST KRZYWA. Bez tej liczby Conduit
                // Graph nie umie odróżnić „w tej minucie nic się nie działo"
                // od „ten przebieg w ogóle nie ma rozdzielczości minutowej" —
                // a to dwa zupełnie różne wnioski z tego samego pustego słupka.
                "krok_ms": a.krzywa_ms,
                // ŹRÓDŁO SYGNAŁÓW. Od 03.08 mamy pięć zbiorów (ATFX, Synergy,
                // ZEN, NOVA, PULSEX) i sama nazwa presetu przestała wystarczać
                // do rozpoznania przebiegu: „HYPER-2" na ATFX i „HYPER-2" na
                // NOVA to dwie różne rzeczy, a na wykresie wyglądają tak samo.
                // Bez tych dwóch pól porównanie dwóch krzywych obok siebie
                // wymaga wiedzy spoza pliku — czyli jest nieweryfikowalne.
                "sygnaly": a.signals.file_name().map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                "sygnaly_sciezka": a.signals.display().to_string(),
                "signal_time_offset_min": a.signal_time_offset_min,
                "signal_time_offset_explicit": a.signal_time_offset_explicit,
                "format": formaty.get(name.as_str()).cloned().unwrap_or_default(),
                "krzywa": r.equity_curve
                    .iter()
                    .map(|(t, v)| serde_json::json!([t, v]))
                    .collect::<Vec<_>>(),
                // Saldo osobno. Różnica wobec `krzywa` to pływający wynik
                // otwartych pozycji — bez tej drugiej serii nie da się
                // odróżnić straty zaksięgowanej od chwilowego zanurzenia.
                "saldo": r.balance_curve
                    .iter()
                    .map(|(t, v)| serde_json::json!([t, v]))
                    .collect::<Vec<_>>(),
                "dni": &r.daily,
                "metryki": &r.metrics,
            });
            std::fs::write(
                a.out.join(format!("{safe}_{tag}_dane.json")),
                serde_json::to_string(&dane)?,
            )?;
        }
        // KAZDA POJEDYNCZA TRANSAKCJA — osobny plik, niezalezny od `--no-charts`.
        //
        // Dane leza w `RunResult.trades` od zawsze, ale nigdy nie trafialy na
        // dysk: zapisywalismy metryki, krzywa i statystyki DNI. Przez to pytanie
        // „ktora dokladnie transakcja zjadla ten dzien" bylo bez odpowiedzi,
        // chociaz silnik mial odpowiedz w reku.
        //
        // Saldo narastajaco i czas trzymania liczymy TUTAJ, bo `ClosedTrade`
        // ich nie niesie, a bez nich tabela transakcji jest nieczytelna.
        if !a.summary_only && !r.trades.is_empty() {
            let pozycje = closed_trade_export_rows(&r.trades, r.metrics.start_balance);
            let plik = serde_json::json!({
                "trade_export_version": 2,
                "net_basis": "explicit per-row producer basis; unknown values are null",
                "preset": name,
                "format": formaty.get(name.as_str()).cloned().unwrap_or_default(),
                "od": fmt_ts(from),
                "do": fmt_ts(to),
                "tryb": tag,
                "saldo_start": r.metrics.start_balance,
                "sygnaly": a.signals.display().to_string(),
                "signal_time_offset_min": a.signal_time_offset_min,
                "signal_time_offset_explicit": a.signal_time_offset_explicit,
                "transakcji": pozycje.len(),
                "transakcje": pozycje,
            });
            std::fs::write(
                a.out.join(format!("{safe}_{tag}_transakcje.json")),
                serde_json::to_string(&plik)?,
            )?;
        }
        summary.insert(name.clone(), r.metrics.clone());
    }

    if results.len() > 1 && !a.no_charts && !a.summary_only {
        let cmp: Vec<(String, conduit_backtest::metrics::Metrics)> = rows
            .iter()
            .map(|(n, r)| (n.clone(), r.metrics.clone()))
            .collect();
        let svg = chart::compare_chart(
            &format!(
                "Porównanie konfiguracji · {} … {} · {tag}",
                fmt_ts(from),
                fmt_ts(to)
            ),
            &cmp,
        );
        std::fs::write(a.out.join(format!("porownanie_{tag}.svg")), svg)?;
    }

    let quick_observation = results
        .iter()
        .find_map(|(_, result)| result.approximation.as_ref())
        .cloned();
    let quick_observed_pct: Vec<f64> = results
        .iter()
        .filter_map(|(_, result)| result.approximation.as_ref().map(|info| info.observed_pct))
        .collect();
    let quick_observed_pct_min = quick_observed_pct
        .iter()
        .copied()
        .reduce(f64::min);
    let quick_observed_pct_max = quick_observed_pct
        .iter()
        .copied()
        .reduce(f64::max);
    let summary_json = if a.quick_tick_stride > 1 {
        serde_json::json!({
            "schema": "conduit.quick-sweep-results.v1",
            "approximate": true,
            "coronation_eligible": false,
            "quick_tick_stride": a.quick_tick_stride,
            "quick_backtest": quick_observation,
            "observed_pct_min": quick_observed_pct_min,
            "observed_pct_max": quick_observed_pct_max,
            "warning": "APPROXIMATE SCREENING ONLY — rerun finalists with N=1",
            "results": summary,
        })
    } else {
        // Preserve the exact historical JSON shape byte-for-byte: a direct
        // map name -> Metrics, with no quick wrapper or extra fields.
        serde_json::to_value(&summary)?
    };
    std::fs::write(
        a.out.join(format!("wyniki_{tag}.json")),
        serde_json::to_string_pretty(&summary_json)?,
    )?;

    // Wynik cząstkowy MUSI się sam przedstawiać. Plik `wyniki_*.json` wygląda
    // identycznie jak z pełnego przebiegu, a nie jest tym samym — bez tej
    // notatki po tygodniu nikt (łącznie z autorem) nie odróżni jednego od
    // drugiego i porówna nieporównywalne.
    if przerwane {
        let zrobiono = licznik.load(Ordering::Relaxed) as f64;
        let razem = (tickow_na_przebieg as f64) * warianty.len() as f64;
        std::fs::write(
            a.out.join("PRZERWANE.txt"),
            format!(
                "Przebieg PRZERWANY na żądanie z okna postępu.\n\
                 policzono: {} z {} ticków ({} %)\n\
                 presetów:  {}\n\
                 okno:      {} … {}\n\
                 tryb:      {tag}\n\n\
                 Wyniki w wyniki_{tag}.json są CZĄSTKOWE — dotyczą krótszego\n\
                 fragmentu historii niż zamówiony i nie wolno ich zestawiać\n\
                 z pełnymi przebiegami.\n",
                conduit_monitor::pl_duza(zrobiono),
                conduit_monitor::pl_duza(razem),
                conduit_monitor::pl_liczba(zrobiono / razem.max(1.0) * 100.0, 1),
                warianty.len(),
                fmt_ts(from),
                fmt_ts(to),
            ),
        )?;
        println!(
            "zapisano notatkę o przerwaniu → {}",
            a.out.join("PRZERWANE.txt").display()
        );
    }

    {
        //  SITO BROKERA — patrz `odsiew_sita` (engine.rs). Linia istnieje po
        //  to, zeby pytanie „czy `drop_unplaceable_levels` cokolwiek wycina"
        //  nigdy wiecej nie bylo rozstrzygane rozumowaniem zamiast pomiarem.
        let (szczeble, koszyki) = conduit_core::odsiew_sita();
        eprintln!(
            "sito brokera: odsiano {szczeble} szczebli w {koszyki} koszykach (suma po wszystkich przebiegach)"
        );
    }
    let total_ticks: u64 = results.iter().map(|(_, r)| r.ticks_processed).sum();
    let total_ms: u64 = results.iter().map(|(_, r)| r.elapsed_ms).max().unwrap_or(0);
    println!(
        "\n{} przebiegów · {} mln ticków każdy · {:.1} s · wyniki w {}/",
        results.len(),
        results[0].1.ticks_processed / 1_000_000,
        total_ms as f64 / 1000.0,
        a.out.display()
    );
    let _ = total_ticks;

    // ETAP E0 — ARBITER CIENIOWY. Bez zmiennej `MOZG_CIEN` funkcja zwraca
    // `None` po JEDNYM odczycie `OnceLock` i nie tworzy żadnego pliku.
    if let Some(p) = conduit_mozg_cien::diag::zapisz_raport(&format!(
        "# ticki={total_ticks} · wyniki={}",
        a.out.display()
    )) {
        println!("arbiter cieniowy → {p}");
    }

    Ok(())
}

/// Ocena konfiguracji.
///
/// Sam zysk podzielony przez obsunięcie NIE WYSTARCZA: konfiguracja z bardzo
/// szerokim stop-lossem prawie nigdy nie realizuje straty, więc w krótkiej
/// próbce wygląda jak maszynka do pieniędzy — a naprawdę trzyma otwarte
/// ryzyko wielokrotnie przekraczające kapitał. Dlatego:
///  * wyzerowanie konta = dyskwalifikacja,
///  * otwarte ryzyko powyżej `MAX_RISK_PCT` kapitału = dyskwalifikacja,
///  * w pozostałych przypadkach premiujemy zysk na jednostkę FAKTYCZNIE
///    podjętego ryzyka (większego z: obsunięcie, otwarte ryzyko).
const MAX_RISK_PCT: f64 = 50.0;

/// Tryb OKIEN PRZESUWANYCH („n-ki") — `--reset-co N`, opcjonalnie z `--zzn`.
///
/// Wypisuje rozkład wyników okien i — zawsze, także bez ZZN — SKALĘ UCIĘCIA
/// na granicy okna: ile pozycji, ile dolarów niezrealizowanych, ile zleceń
/// oczekujących i ile żywych koszyków ginie w chwili domknięcia okna. To jest
/// liczba, bez której nie wiadomo, czy metryka mierzy strategię, czy własną
/// ramkę czasową.
/// Tryb okien przesuwanych.
///
/// Bierze WARIANTY, a nie surowe presety: wariant niesie komplet nóg
/// (`formaty`) i pułapy, więc łańcuch wielonogowy da się zmierzyć tym samym
/// kryterium co pojedynczy preset. Przy przebiegu jednoformatowym `formaty`
/// jest puste i `okna.rs` idzie ścieżką parytetu — jeden silnik, slot 0.
/// Co tryb okien musi wiedzieć o konfiguracji, którą ma policzyć.
///
/// Istnieje po to, żeby `tryb_okien` przyjmował ZARÓWNO `Wariant` (który niesie
/// nogi i pułapy), JAK I gołe `Preset` — bez dublowania pętli. Preset oddaje
/// puste nogi i zerowe pułapy, czyli dokładnie ścieżkę parytetu.
trait WariantOkien {
    fn etykieta(&self) -> &str;
    fn ustawienia(&self) -> &Settings;
    fn nogi(&self) -> &[FormatCfg];
    fn sufity(&self) -> &PulapyGlobalne;
}

impl WariantOkien for Preset {
    fn etykieta(&self) -> &str {
        &self.name
    }
    fn ustawienia(&self) -> &Settings {
        &self.settings
    }
    fn nogi(&self) -> &[FormatCfg] {
        &[]
    }
    fn sufity(&self) -> &PulapyGlobalne {
        // Pusty sufit jako stała, żeby dało się oddać referencję.
        static ZERO: std::sync::OnceLock<PulapyGlobalne> = std::sync::OnceLock::new();
        ZERO.get_or_init(PulapyGlobalne::default)
    }
}

/// Reporting projection only. The broker's original profit and account cash are
/// never changed; unknown historical bases remain unknown, including the suffix
/// of the synthetic cumulative closed-net ledger.
fn closed_trade_export_rows(trades: &[conduit_core::types::ClosedTrade], start: f64) -> Vec<serde_json::Value> {
    let mut ledger = start.is_finite().then_some(start);
    trades.iter().enumerate().map(|(i,t)| {
        let net = t.net_profit();
        ledger = ledger.zip(net).map(|(balance,value)|balance+value).filter(|v|v.is_finite());
        let mut row=serde_json::json!({
            "lp":i+1, "ticket":t.ticket, "kierunek":format!("{:?}",t.side),
            "wolumen":t.volume, "cena_otwarcia":t.open_price, "cena_zamkniecia":t.close_price,
            "otwarcie":fmt_ts(t.open_ts), "zamkniecie":fmt_ts(t.close_ts),
            "otwarcie_ms":t.open_ts, "zamkniecie_ms":t.close_ts,
            "trzymanie_min":(t.close_ts-t.open_ts) as f64/60_000.,
            "zysk":t.profit, "prowizja":t.commission, "swap":t.swap,
            "netto":net, "saldo_po":ledger, "powod":format!("{:?}",t.reason), "koszyk":t.basket,
            "profit_basis":t.profit_basis,
            "netto_status":if net.is_some(){"known"}else{"unavailable_unknown_basis_or_invalid_receipt"},
            "saldo_po_basis":"start_plus_closed_net_NOT_actual_cash_balance",
        });
        if let Some(receipt)=&t.cost_receipt { row["cost_receipt"]=serde_json::json!(receipt); }
        row
    }).collect()
}

fn closed_trade_raw_export(trades: &[conduit_core::types::ClosedTrade]) -> Result<serde_json::Value> {
    let mut rows=serde_json::to_value(trades)?;
    for (row,trade) in rows.as_array_mut().expect("serialized trade slice is an array").iter_mut().zip(trades) {
        row["net_profit"]=serde_json::json!(trade.net_profit());
    }
    Ok(rows)
}

#[cfg(test)]
mod trade_export_basis_tests {
    use super::*;
    use conduit_core::{cost_receipt::*,types::{ClosedTrade,Side,CloseReason}};
    fn trade(basis:Option<ProfitBasis>,profit:f64,swap:f64)->ClosedTrade {
        ClosedTrade{ticket:101,side:Side::Buy,volume:0.01,open_price:4000.,close_price:4100.,
            open_ts:1_700_000_000_000,close_ts:1_700_000_060_000,profit,commission:-2.,swap,
            reason:CloseReason::Tp,basket:Some(1),profit_basis:basis,cost_receipt:None}
    }
    #[test]
    fn simulated_price_plus_swap_is_not_charged_a_second_time() {
        let trades=[trade(Some(ProfitBasis::PricePlusSwap),103.,3.),trade(Some(ProfitBasis::PriceOnlyGross),100.,3.)];
        let rows=closed_trade_export_rows(&trades,600.);
        assert_eq!(rows[0]["netto"],101.);assert_eq!(rows[1]["netto"],101.);
        assert_eq!(rows[1]["saldo_po"],802.);assert_eq!(rows[0]["zysk"],103.);
        assert_eq!(trades[0].profit,103.,"export cannot mutate already-booked broker profit");
    }
    #[test]
    fn canonical_receipt_keeps_fee_and_costs_exactly_once() {
        let receipt=CostReceipt{schema:CostSchema::V1,key:CostReceiptKey{scope_id:"synthetic-export".into(),deal_id:1001},
            position_identifier:101,volume:0.01,currency:"USD".into(),
            source:CostSource::SimulatorLedger{run_id:"export-test".into(),cost_spec_hash:"synthetic-spec".into()},
            gross_profit:Some(100.),entry_commission_alloc:Some(-1.),exit_commission:Some(-1.),
            entry_fee_alloc:Some(0.),exit_fee:Some(-0.5),swap:Some(3.),
            completeness:CostCompleteness::Complete,entry_allocation:None};
        let t=trade(None,100.,3.).with_cost_receipt(receipt).unwrap();
        let rows=closed_trade_export_rows(&[t],600.);
        assert_eq!(rows[0]["netto"],100.5);assert_eq!(rows[0]["saldo_po"],700.5);
    }
    #[test]
    fn unknown_legacy_or_invalid_receipt_never_fabricates_a_net_ledger() {
        let rows=closed_trade_export_rows(&[trade(None,100.,3.),trade(Some(ProfitBasis::PricePlusSwap),103.,3.),
            trade(Some(ProfitBasis::CanonicalClosedNetV1),100.,3.)],600.);
        assert!(rows[0]["netto"].is_null());assert!(rows[0]["saldo_po"].is_null());
        assert_eq!(rows[1]["netto"],101.);assert!(rows[1]["saldo_po"].is_null());
        assert!(rows[2]["netto"].is_null());assert!(rows[2]["saldo_po"].is_null());
        let raw=closed_trade_raw_export(&[trade(None,100.,3.),trade(Some(ProfitBasis::PricePlusSwap),103.,3.)]).unwrap();
        assert!(raw[0]["net_profit"].is_null());assert_eq!(raw[1]["net_profit"],101.);
        assert_eq!(raw[1]["profit"],103.);assert_eq!(raw[1]["swap"],3.);
    }
}

fn tryb_okien<W>(
    a: &Args,
    ticks: &conduit_backtest::TickData,
    messages: &[conduit_backtest::ReplayMessage],
    warianty: &[W],
    from: i64,
    to: i64,
) -> Result<()>
where
    W: WariantOkien + Sync,
{
    use conduit_backtest::okna::{uruchom, KonfOkien};
    use rayon::prelude::*;

    if a.daily_reset {
        eprintln!("uwaga: --daily-reset jest ignorowany, gdy podano --reset-co (n=1 to to samo)");
    }

    let wyniki: Vec<(String, conduit_backtest::okna::WynikOkien)> = warianty
        .par_iter()
        .map(|p| {
            let cfg = KonfOkien {
                from,
                to,
                start_balance: a.balance,
                sim_limit_price_improvement: a.sim_limit_price_improvement,
                sim_new_pending_sl_next_tick: a.sim_new_pending_sl_next_tick,
                sim_native_swap_cash_digits: a.sim_native_swap_cash_digits,
                sim_trade_sessions: a.sim_trade_sessions.clone(),
                settings: p.ustawienia().clone(),
                n_dni: a.reset_co,
                zzn: a.zzn,
                zzn_max_dni: a.zzn_max_dni,
                source_name: "ATFX VIP SIGNALS".into(),
                formaty: p.nogi().to_vec(),
                pulapy: p.sufity().clone(),
            };
            (p.etykieta().to_string(), uruchom(ticks, messages, &cfg))
        })
        .collect();
    for (name,result) in &wyniki {
        if let Some(reason)=&result.continuation_reconciliation_required {
            anyhow::bail!("CONTINUATION HOLD — windows {name} are not rankable: {reason}");
        }
        if let Some(reason)=&result.sim_execution_reconciliation_required {
            anyhow::bail!("SIM EXECUTION HOLD — windows {name} are not rankable: {reason}");
        }
    }

    // znak dopisujemy sami; `-0.00` po zaokrągleniu udaje stratę, więc zero
    // pokazujemy jako zero
    let zn = |x: f64, d: usize| {
        let s = format!("{x:.*}", d);
        if s.starts_with('-') && s.trim_start_matches(['-', '0', '.', ',']).is_empty() {
            return s.trim_start_matches('-').to_string();
        }
        if x >= 0.0 {
            format!("+{s}")
        } else {
            s
        }
    };

    println!(
        "\n╔══ OKNA PRZESUWANE · n = {} dni handlowych · {} ══",
        a.reset_co,
        if a.zzn {
            "ZZN (koszyki dochodzą do końca)"
        } else {
            "bez ZZN (twarde ucięcie)"
        }
    );
    println!(
        "║ okno przesuwa się o JEDEN dzień; wewnątrz okna działa compounding od {:.0} $",
        a.balance
    );
    println!("╚══ {} … {}\n", fmt_ts(from), fmt_ts(to));

    println!(
        "{:<26}{:>6}{:>10}{:>10}{:>10}{:>8}{:>10}{:>10}",
        "konfiguracja", "okien", "suma $", "średnia $", "mediana $", "okien+%", "najgorsze", "dno$"
    );
    println!("{}", "─".repeat(90));
    let mut rows: Vec<_> = wyniki.iter().collect();
    rows.sort_by(|x, y| {
        y.1.mediana
            .partial_cmp(&x.1.mediana)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (name, w) in rows.iter().take(a.top) {
        println!(
            "{:<26}{:>6}{:>10.2}{:>10.2}{:>10.2}{:>8.1}{:>10.2}{:>10.2}",
            trunc(name, 25),
            w.okna.len(),
            w.suma,
            w.srednia,
            w.mediana,
            w.pct_dodatnich,
            w.najgorsze_okno,
            w.min_equity_najgorszego
        );
    }

    // ---------- skala ucięcia na granicy ----------
    for (name, w) in &wyniki {
        if wyniki.len() > 1 {
            println!("\n── {name} ──");
        }
        println!(
            "\nUCIĘCIE NA GRANICY OKNA (stan w chwili końca okna, przed domknięciem):\n  \
             pozycji otwartych:        {}\n  \
             ich niezrealizowany wynik: {} $\n  \
             zleceń oczekujących:      {}\n  \
             żywych koszyków:          {}\n  \
             okien dotkniętych:        {} z {}",
            w.ucietych_pozycji,
            zn(w.uciety_floating, 2),
            w.ucietych_pendingow,
            w.ucietych_koszykow,
            w.okien_z_ucieciem,
            w.okna.len()
        );
        if w.suma.abs() > 1e-9 {
            // `-0.0 == 0.0` w Rust jest prawdą, więc to normalizuje minus zero
            let udzial = w.uciety_floating / w.suma * 100.0;
            let udzial = if udzial == 0.0 { 0.0 } else { udzial };
            println!(
                "  udział w wyniku:          {udzial:.2} % sumy okien ({} $)",
                zn(w.suma, 2)
            );
        }
        if a.zzn {
            let wklad: f64 = w.okna.iter().map(|o| o.wklad_ogona()).sum();
            println!(
                "\nOGON ZZN:\n  \
                 wkład ogonów w wynik:     {} $\n  \
                 najdłuższy ogon:          {} min · średni {:.0} min\n  \
                 okien z ogonem UCIĘTYM:   {} z {}",
                zn(wklad, 2),
                w.najdluzszy_ogon_min,
                w.sredni_ogon_min,
                w.okien_z_ucietym_ogonem,
                w.okna.len()
            );
        }
        println!(
            "\nROZKŁAD OKIEN:\n  \
             suma {} $ · średnia {} $ · MEDIANA {} $\n  \
             okien dodatnich {:.1} % · najlepsze {} $ · NAJGORSZE {} $\n  \
             — po oknach Z HANDLEM ({} z {}): mediana {} $ · dodatnich {:.1} %\n    \
               (to jest mianownik, którego używa Metrics::win_days_pct — bez tego\n    \
                nie da się porównać z bazą 1936 / 1178 / 2527)\n  \
             min. equity w najgorszym oknie {:.2} $ · najniższe w ogóle {:.2} $\n  \
             dni handlowych {} · {} okien · {:.1} s",
            zn(w.suma, 2),
            zn(w.srednia, 2),
            zn(w.mediana, 2),
            w.pct_dodatnich,
            zn(w.najlepsze_okno, 2),
            zn(w.najgorsze_okno, 2),
            w.okien_z_handlem,
            w.okna.len(),
            zn(w.mediana_z_handlem, 2),
            w.pct_dodatnich_z_handlem,
            w.min_equity_najgorszego,
            w.min_equity_globalne,
            w.dni_handlowych,
            w.okna.len(),
            w.czas_ms as f64 / 1000.0
        );
        for z in &w.zastrzezenia {
            println!("  ZASTRZEŻENIE: {z}");
        }
    }

    // ---------- zapis ----------
    let tag = format!("okna{}{}", a.reset_co, if a.zzn { "_zzn" } else { "" });
    let mapa: HashMap<&str, &conduit_backtest::okna::WynikOkien> =
        wyniki.iter().map(|(n, w)| (n.as_str(), w)).collect();
    let path = a.out.join(format!("wyniki_{tag}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&mapa)?)?;
    println!("\nwyniki → {}", path.display());
    Ok(())
}

/// Ocena konfiguracji.
///
/// **Obsunięcie samo w sobie nie jest szkodą.** Szkodą jest realna strata.
/// Konto, które z 200 $ robi 600 $ przy obsunięciu 400 $, jest lepsze od konta,
/// które z 200 $ robi 180 $ przy obsunięciu 20 $ — mimo dwudziestokrotnie
/// większego obsunięcia. Wcześniejsza wersja tej funkcji dzieliła zysk przez
/// obsunięcie i systematycznie promowała warianty bezpieczne i jałowe, przez co
/// przez pół sesji odrzucałem konfiguracje o kilkukrotnie wyższym zysku.
///
/// Dlatego liczy się **stosunek zysku do obsunięcia** (MAR), a nie samo
/// obsunięcie, i to on jest osią rankingu. Obsunięcie wchodzi tylko dwoma
/// kanałami, oba dotyczą realnej szkody:
///  * **wyzerowanie konta** — dyskwalifikacja bezwarunkowa, bo tego nie da się
///    odrobić żadnym zyskiem;
///  * **zbliżenie do zera** — im niżej equity zeszło względem kapitału
///    startowego, tym większe prawdopodobieństwo, że kolejny gorszy przebieg
///    tej samej strategii już konta nie oszczędzi.
fn require_reconciled_run(result: &conduit_backtest::runner::RunResult, context: &str) -> Result<()> {
    if let Some((kind, reason)) = result.reconciliation_hold() {
        anyhow::bail!("{kind} HOLD — {context} is not rankable: {reason}");
    }
    Ok(())
}

fn publishable_run_metrics<'a>(result: &'a conduit_backtest::runner::RunResult, context: &str)
    -> Result<Option<&'a Metrics>> {
    require_reconciled_run(result, context)?;
    Ok((!result.cancelled).then_some(&result.metrics))
}

fn score_reconciled_run(result: &conduit_backtest::runner::RunResult, context: &str) -> Result<f64> {
    let metrics = publishable_run_metrics(result, context)?
        .ok_or_else(|| anyhow::anyhow!("CANCELLED — {context} is not a completed ranking candidate"))?;
    Ok(score(metrics))
}

fn score(m: &conduit_backtest::metrics::Metrics) -> f64 {
    if m.blown {
        return f64::MIN;
    }
    if m.max_open_risk_pct > MAX_RISK_PCT {
        // nie odrzucamy całkiem, ale spychamy na koniec rankingu
        return -1e6 + m.total_profit / m.max_open_risk_pct.max(1.0);
    }
    if m.total_profit <= 0.0 {
        // strata: im mniejsza, tym lepiej, ale zawsze poniżej każdego zysku
        return m.total_profit / m.start_balance.max(1.0) - 1000.0;
    }

    let kap = m.start_balance.max(1.0);

    // 1. ZWROT — to jest cel, więc wchodzi wprost, bez pierwiastkowania.
    let zwrot = m.total_profit / kap;

    // 2. MARGINES DO ZERA — jedyna postać obsunięcia, która naprawdę boli.
    //    Equity nigdy nieschodzące poniżej kapitału daje 1,0; zejście do 30 %
    //    kapitału daje 0,3. Konto uratowane cudem nie wygrywa z kontem, które
    //    nigdy nie było blisko krawędzi.
    let margines = (m.min_equity / kap).clamp(0.05, 1.0);

    // 3. REALNA STRATA — najgorszy pojedynczy dzień jako ułamek kapitału,
    //    jakim konto DYSPONOWAŁO, a nie kapitału startowego.
    //    Przy compoundingu najgorszy dzień wypada wtedy, gdy konto jest już
    //    wielokrotnie większe; odnoszenie go do 200 $ startowych zamieniłoby
    //    stratę 15 % w pozorne 750 % i wyrzuciłoby z rankingu każdy wariant
    //    rosnący. Za typową wielkość konta bierzemy średnią z kapitału
    //    startowego i końcowego.
    //    Obsunięcie w środku dnia, które do wieczora się odrobiło, nie kosztuje
    //    nic — dlatego karzemy `worst_day`, a nie `max_dd_abs`.
    let typowe_konto = ((kap + m.end_equity.max(kap)) * 0.5).max(kap);
    let realna_strata = (m.worst_day.min(0.0).abs()) / typowe_konto;

    // 3a. POZIOM MARGINESU — ODLEGŁOŚĆ OD LIKWIDACJI, a nie od zera.
    //
    // # Dlaczego to musiało wejść do rankingu (07.08.2026)
    //
    // Symulator MIERZYŁ `min_margin_level` i liczniki ticków pod 200/150/100 %
    // od dawna, `bt` je DRUKOWAŁ, a ta funkcja ich NIE CZYTAŁA. Skutek jest
    // policzalny: HYPER-2 na korpusie Synergy (714 sygnałów, 23.06–07.08)
    // robi +21 140 $ ze 300 $ i spędza przy tym **176 ticków poniżej 100 %
    // poziomu marginesu**, przy dnie equity 119 $ i szczycie 11,45 lota.
    // Broker wzywa przy 50 % i likwiduje przy 20 % — ten przebieg przeżył,
    // ale ranking nie miał jak odróżnić go od takiego, który nigdy nie zszedł
    // poniżej 800 %. Wygrywał, bo equity się odrobiło.
    //
    // `min_equity` tego nie zastępuje. Equity mówi, ile zostało; poziom
    // marginesu mówi, ile zostało W STOSUNKU DO TEGO, CO TRZYMAMY OTWARTE —
    // i tylko ta druga liczba decyduje, czy broker zamknie pozycje za nas.
    // Konto z 5 000 $ equity i 4 800 $ marginesu jest w śmiertelnym
    // niebezpieczeństwie; konto ze 150 $ equity i 20 $ marginesu nie jest.
    //
    // Kara jest MNOŻNIKIEM, nie odjęciem: przebieg ocierający się o stop-out
    // ma spaść na dno rankingu niezależnie od tego, ile zarobił.
    //
    // `min_margin_level` bez ani jednej otwartej pozycji jest nieskończonością
    // (patrz `metrics.rs:231`) — wtedy człon wynosi 1,0 i nic nie zmienia.
    let margines_ml = {
        let ml = m.min_margin_level;
        if !ml.is_finite() || ml <= 0.0 {
            1.0
        } else {
            // 20 % = stop-out brokera → 0. 500 % i wyżej → 1,0. Między nimi
            // liniowo: 100 % daje 0,17, 200 % daje 0,375, 300 % daje 0,58.
            ((ml - 20.0) / 480.0).clamp(0.0, 1.0)
        }
    };
    // Sam MINIMALNY poziom to jedno zdarzenie; czas spędzony w strefie
    // zagrożenia to drugie. Przebieg, który dotknął 90 % na jednym ticku,
    // i taki, który siedział pod 100 % przez tysiąc ticków, to nie jest
    // to samo ryzyko.
    let czas_w_ryzyku = {
        let t = m.ml_pod_150 as f64;
        if t <= 0.0 {
            1.0
        } else {
            // każde 1000 ticków pod ML 150 % zabiera połowę oceny
            0.5_f64.powf(t / 1000.0)
        }
    };

    // 4. STAŁOŚĆ — udział dni na plusie. Rozstrzyga remisy między wariantami
    //    o podobnym zwrocie: wolimy ten, który zarabia częściej.
    let stalosc = 0.5 + m.win_days_pct / 200.0;

    zwrot * margines * margines_ml * czas_w_ryzyku * stalosc / (1.0 + 3.0 * realna_strata)
}

/// Dokładny czas do rozdzielenia końca eksportu, danych i pomiaru.
fn data_czas_ludzki(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%d.%m.%Y %H:%M:%S").to_string())
        .unwrap_or_else(|| "—".into())
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}

fn fmt_ts(ts: i64) -> String {
    let day = ts.div_euclid(86_400_000);
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


#[cfg(test)]
mod testy_konfiguracji_walk_forward {
    use super::*;

    fn args() -> Args {
    let mut a = Args {
        ticks: "data/ticks.bin".into(),
        signals: "data/signals.json".into(),
        signal_time_offset_min: 0,
        signal_time_offset_explicit: false,
        sim_limit_price_improvement: false,
        sim_new_pending_sl_next_tick: false,
        sim_native_swap_cash_digits: None,
        sim_trade_sessions: None,
        live_telegram_ingress: false,
        quick_tick_stride: 1,
        sim_price_digits: None,
        from: None,
        to: None,
        period: "all".into(),
        preset: None,
        sweep: None,
        balance: 200.0,
        daily_reset: false,
        auto_ea: false,
        flat_na_dobie: false,
        out: "out".into(),
        top: 20,
        quiet: false,
        walk_forward: 0,
        dump_trades: false,
        no_charts: false,
        summary_only: false,
        journal: None,
        reset_co: 0,
        zzn: false,
        zzn_max_dni: 7,
        krzywa_ms: 300_000,
        formaty: Vec::new(),
        pulapy: Vec::new(),
        rachunek: None,
        // ZIMNY START — domyślna ścieżki parytetu. Nie zmieniać bez przeliczenia
        // 1936,47 / 1178,02 / 2527,07.
        rozgrzewka_h: 0,
        drabinka: None,
        drabinka_histereza_pct: 0.0,
        presety_dir: "../PACKAGE/presets".into(),
    };
        a.balance = 712.25;
        a.sim_limit_price_improvement = true;
        a.sim_new_pending_sl_next_tick = true;
        a.sim_native_swap_cash_digits = Some(2);
        a.live_telegram_ingress = true;
        a.sim_price_digits = Some(2);
        a.daily_reset = true;
        a.auto_ea = true;
        a.flat_na_dobie = true;
        a.krzywa_ms = 17;
        a.rozgrzewka_h = 72;
        a.drabinka_histereza_pct = 3.25;
        a
    }

    fn variant() -> Wariant {
        let mut settings = Settings::default();
        settings.credit_balance_separate = true;
        settings.odlicz_kredyt = true;
        settings.kredyt_reczny = 300.0;
        settings.lot_fixed = 0.07;
        settings.close_receipt_reconcile = true;
        settings.closed_profit_net_costs = true;
        let mut leg = settings.clone();
        leg.lot_fixed = 0.03;
        Wariant {
            nazwa: "WF-config-fixture".into(),
            settings,
            formaty: vec![FormatCfg { format: "Synergy".into(),
                preset: "owned-leg".into(), settings: leg }],
            pulapy: serde_json::from_value(serde_json::json!({
                "maxLotow":0.43,"maxPozycji":20,"maxKoszykow":6
            })).unwrap(),
            format: "Synergy".into(),
            drabinka: Vec::new(),
            ea: Some(r#"{"enabled":true,"pulse_every_s":7}"#.into()),
        }
    }

    fn legs(v: &[FormatCfg]) -> serde_json::Value {
        serde_json::Value::Array(v.iter().map(|f| serde_json::json!({
            "format":f.format, "preset":f.preset, "settings":f.settings
        })).collect())
    }

    // Exhaustive destructuring intentionally fails compilation after any new
    // RunConfig field. Every field must enter this contract, not silently default.
    fn all_fields(c: &RunConfig) -> serde_json::Value {
        let RunConfig {
            from,to,start_balance,sim_limit_price_improvement,
            sim_new_pending_sl_next_tick,sim_native_swap_cash_digits,sim_trade_sessions,
            live_telegram_ingress,quick_tick_stride,
            settings,formaty,pulapy,daily_reset,source_name,curve_interval_ms,
            rozgrzewka_h,drabinka,drabinka_histereza_pct,drabinka_kredyt,
            flat_na_dobie,journal_path,auto_ea,ea_konfig,
        } = c;
        serde_json::json!({
            "from":from,"to":to,"start_balance_bits":start_balance.to_bits(),
            "sim_limit_price_improvement":sim_limit_price_improvement,
            "sim_new_pending_sl_next_tick":sim_new_pending_sl_next_tick,
            "sim_native_swap_cash_digits":sim_native_swap_cash_digits,
            "sim_trade_sessions":sim_trade_sessions,
            "live_telegram_ingress":live_telegram_ingress,
            "quick_tick_stride":quick_tick_stride,
            "settings":settings,"formaty":legs(formaty),"pulapy":pulapy,
            "daily_reset":daily_reset,"source_name":source_name,
            "curve_interval_ms":curve_interval_ms,"rozgrzewka_h":rozgrzewka_h,
            "drabinka":drabinka.iter().map(|d| serde_json::json!({
                "prog_bits":d.prog.to_bits(),"nazwa":d.nazwa,
                "formaty":legs(&d.formaty),"pulapy":d.pulapy
            })).collect::<Vec<_>>(),
            "drabinka_histereza_pct_bits":drabinka_histereza_pct.to_bits(),
            "drabinka_kredyt_bits":drabinka_kredyt.to_bits(),
            "flat_na_dobie":flat_na_dobie,"journal_path":journal_path,
            "auto_ea":auto_ea,"ea_konfig":ea_konfig,
        })
    }

    fn assert_stage(main: &RunConfig, actual: &RunConfig, from: i64, to: i64) {
        let mut expected = all_fields(main);
        expected["from"] = from.into();
        expected["to"] = to.into();
        expected["journal_path"] = serde_json::Value::Null;
        expected["curve_interval_ms"] = main.curve_interval_ms.max(600_000).into();
        let got = all_fields(actual);
        let diff: Vec<_> = expected.as_object().unwrap().iter()
            .filter(|(key,value)| got.get(*key) != Some(*value))
            .map(|(key,value)| format!("{key}: expected={value}, actual={}",got[key]))
            .collect();
        assert!(diff.is_empty(),"RunConfig propagation differences:\n{}",diff.join("\n"));
    }

    #[test]
    fn training_preserves_every_main_field_except_explicit_stage_changes() {
        let a = args(); let p = variant();
        let main = main_run_config(&a,&p,1,99,Some("main-only.jsonl".into()));
        let train = walk_forward_run_config(&main,11,22);
        assert_stage(&main,&train,11,22);
    }

    #[test]
    fn oos_preserves_every_main_field_including_selected_ea() {
        let a = args(); let p = variant();
        let main = main_run_config(&a,&p,1,99,Some("main-only.jsonl".into()));
        let oos = walk_forward_run_config(&main,22,33);
        assert_stage(&main,&oos,22,33);
    }

    #[test]
    fn stage_clone_preserves_future_fields_and_does_not_mutate_selected_variant() {
        for model in [false,true] { for separate in [false,true] {
            let mut a=args();let mut p=variant();
            a.sim_limit_price_improvement=model;a.sim_new_pending_sl_next_tick=model;
            a.sim_native_swap_cash_digits=if model {Some(2)} else {None};
            p.settings.credit_balance_separate=separate;
            let mut base=main_run_config(&a,&p,1,99,Some("never-shared.jsonl".into()));
            // Synthetic clone-only sentinel. Actual CLI still disallows WF
            // with a ladder; this checks field preservation, not that workflow.
            base.drabinka=vec![SzczebelCfg {prog:600.5,nazwa:"clone-only".into(),
                formaty:p.formaty.clone(),pulapy:p.pulapy.clone()}];
            base.drabinka_kredyt=123.25;
            base.source_name="explicit-main-source".into();
            base.curve_interval_ms=900_000;
            let before=all_fields(&base);
            let train=walk_forward_run_config(&base,11,22);
            let mut oos=walk_forward_run_config(&base,22,33);
            assert_stage(&base,&train,11,22);assert_stage(&base,&oos,22,33);
            oos.settings.lot_fixed=0.99;
            oos.formaty[0].settings.kredyt_reczny=999.0;
            oos.ea_konfig=Some("different-selected-ea".into());
            oos.drabinka[0].nazwa="different-stage".into();
            assert_eq!(all_fields(&base),before,"stage mutated its MAIN base");
            assert_eq!(p.formaty[0].settings.kredyt_reczny,300.0);
            assert_stage(&base,&train,11,22);
        }}
    }

    #[test]
    fn each_selected_variant_owns_its_settings_legs_and_ea_in_oos() {
        let a=args();
        for id in [1,2] {
            let mut p=variant();
            p.nazwa=format!("selected-{id}");
            p.settings.lot_fixed=id as f64/100.0;
            p.formaty[0].preset=format!("leg-{id}");
            p.ea=Some(format!("{{\"selected\":{id}}}"));
            let base=main_run_config(&a,&p,1,99,None);
            let oos=walk_forward_run_config(&base,22,33);
            assert_stage(&base,&oos,22,33);
            assert_eq!(oos.settings.lot_fixed.to_bits(),p.settings.lot_fixed.to_bits());
            assert_eq!(oos.formaty[0].preset,p.formaty[0].preset);
            assert_eq!(oos.ea_konfig,p.ea);
        }
    }

    struct Tape {
        ticks: Option<TickData>,
        path: PathBuf,
    }
    impl Tape {
        fn new() -> Self {
            let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .unwrap().as_nanos();
            let path=std::env::temp_dir().join(format!("wf-config-{}-{now}.cdtk",std::process::id()));
            let quotes=[(1_787_777_999_000i64,4002.0f32,4002.2f32),
                (1_787_778_000_017,3999.8,4000.0),
                (1_787_796_000_016,4001.0,4001.2),
                (1_787_796_001_016,4001.0,4001.2)];
            let mut bytes=vec![0u8;64];
            bytes[..4].copy_from_slice(&0x4B544443u32.to_le_bytes());
            bytes[8..16].copy_from_slice(&(quotes.len() as u64).to_le_bytes());
            for (ts,bid,ask) in quotes {
                bytes.extend(ts.to_le_bytes());bytes.extend(bid.to_le_bytes());bytes.extend(ask.to_le_bytes());
            }
            use std::io::Write;
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
            file.write_all(&bytes).unwrap();drop(file);
            let mut ticks=TickData::open(&path).unwrap();
            ticks.set_price_digits(Some(2)).unwrap();
            Self {ticks:Some(ticks),path}
        }
    }
    impl Drop for Tape {
        fn drop(&mut self) {self.ticks.take();let _=std::fs::remove_file(&self.path);}
    }

    #[test]
    fn actual_runner_flat_trade_is_not_lost_in_training_or_oos() {
        use conduit_core::settings::PendingLifetime;
        use conduit_core::types::CloseReason;
        let tape=Tape::new();
        let mut a=args();let mut p=variant();
        a.balance=5000.0;a.daily_reset=false;a.auto_ea=false;
        a.flat_na_dobie=true;a.rozgrzewka_h=0;a.krzywa_ms=600_000;
        p.settings=Settings {
            auto_limit:true,entry_units:1,lot_fixed:0.07,
            pending_lifetime:PendingLifetime::Never,pending_drop_on_target:false,
            server_tz_offset_ms:0,msg_clock_offset_ms:Some(0),exec_latency_ms:0,
            live_tick_order_strict:true,runner_ksiegowanie_v2:true,
            session_filter:false,skip_if_sl_breached:false,rearm_grid_on_return:false,
            riskfree_enabled:false,max_dd_pct:0.0,max_dd_usd:0.0,equity_floor_pct:0.0,
            commission_per_lot:0.0,swap_enabled:false,
            ..Settings::default()
        };
        p.ea=None;p.formaty.clear();p.pulapy=PulapyGlobalne::default();
        let from=1_787_777_999_000;let to=1_787_796_001_017;
        let messages=[conduit_backtest::data::ReplayMessage {
            kanal:"ATFX VIP SIGNALS".into(),ts:from,telegram_published_ts:None,
            msg_id:1,reply_to:None,edit_of:None,
            text:"BUY LIMITS GOLD @ 4000/4000 AREA\nTP 4030\nTP 4060\nTP 4090\nSL 3980".into()
        }];
        let base=main_run_config(&a,&p,from,to,None);
        let main=run(tape.ticks.as_ref().unwrap(),&messages,&base);
        assert!(main.reconciliation_hold().is_none());
        assert_eq!(main.trades.len(),1);
        assert_eq!(main.trades[0].reason,CloseReason::EodFlat);
        for (stage,cfg) in [("TRAIN",walk_forward_run_config(&base,from,to)),
                            ("OOS",walk_forward_run_config(&base,from,to))] {
            let result=run(tape.ticks.as_ref().unwrap(),&messages,&cfg);
            assert!(result.reconciliation_hold().is_none(),"{stage}");
            assert_eq!(serde_json::to_value(&result.trades).unwrap(),
                       serde_json::to_value(&main.trades).unwrap(),"{stage} changed trade semantics");
            assert_eq!(serde_json::to_value(&result.metrics).unwrap(),
                       serde_json::to_value(&main.metrics).unwrap(),"{stage} changed metrics");
        }
    }

    // Literal F21 MAIN constructor, kept only as a golden in the test module.
    fn frozen_f21_main(a: &Args, p: &Wariant, from: i64, to: i64,
        journal: Option<PathBuf>) -> RunConfig {
        RunConfig {
        from,
        to,
        start_balance: a.balance,
        sim_limit_price_improvement: a.sim_limit_price_improvement,
        sim_new_pending_sl_next_tick: a.sim_new_pending_sl_next_tick,
        sim_native_swap_cash_digits: a.sim_native_swap_cash_digits,
        sim_trade_sessions: a.sim_trade_sessions.clone(),
        live_telegram_ingress: a.live_telegram_ingress,
        settings: p.settings.clone(),
        ea_konfig: p.ea.clone(),
        formaty: p.formaty.clone(),
        pulapy: p.pulapy.clone(),
        daily_reset: a.daily_reset,
        auto_ea: a.auto_ea,
        flat_na_dobie: a.flat_na_dobie,
        source_name: "ATFX VIP SIGNALS".into(),
        curve_interval_ms: a.krzywa_ms,
        journal_path: journal.clone(),
        // 0 = zimny start = ścieżka parytetu; patrz `Args::rozgrzewka_h`.
        rozgrzewka_h: a.rozgrzewka_h,
        // puste poza `--drabinka` = zero nowego kodu w pętli
        drabinka: p.drabinka.clone(),
        drabinka_histereza_pct: a.drabinka_histereza_pct,
        // „Kredyt odliczony" ma znaczyć odliczony TAKŻE od progów —
        // inaczej 300 $ + 300 $ bonusu startuje od razu na szczeblu
        // 500, choć własnych pieniędzy jest 300. Ta sama reguła co
        // w `Engine::kredyt_skuteczny`: ręczna kwota nadpisuje,
        // 0 przy włączonym odliczaniu = automat z terminala (którego
        // w backteście nie ma, więc 0).
        drabinka_kredyt: if p.settings.odlicz_kredyt && !p.settings.credit_balance_separate {
            p.settings.kredyt_reczny.max(0.0)
        } else {
            0.0
        },
        // Reszta pól z domyślnych. `..Default::default()` jest tu
        // ŚWIADOME: `RunConfig` dostał w ciągu jednego dnia trzy nowe
        // pola (`formaty`, `pulapy`, `rozgrzewka_h`) i za każdym razem
        // przewracał wszystkie pięć wyliczeń wprost — w tym `lotto.exe`
        // w OSOBNYM workspace, którego `cargo check --workspace` nie
        // łapie. Domyślne wartości są ścieżką parytetu (zero pułapów,
        // zimny start), więc dopisanie pola nie może tu niczego zmienić
        // po cichu.
        ..Default::default()
    }
    }

    #[test]
    fn main_constructor_is_identical_to_frozen_f21_all_fields() {
        for model in [false,true] { for separate in [false,true] {
            for credit in [0.0,300.0] {
                let mut a=args(); let mut p=variant();
                a.sim_limit_price_improvement=model;
                a.sim_new_pending_sl_next_tick=model;
                a.sim_native_swap_cash_digits=if model {Some(2)} else {None};
                a.auto_ea=model; a.flat_na_dobie=model; a.daily_reset=model;
                a.rozgrzewka_h=if model {72} else {0};
                p.settings.credit_balance_separate=separate;
                p.settings.kredyt_reczny=credit;
                if !model {p.ea=None;p.formaty.clear();}
                p.drabinka=vec![SzczebelCfg {prog:600.5,nazwa:"identity-only".into(),
                    formaty:p.formaty.clone(),pulapy:p.pulapy.clone()}];
                let actual=main_run_config(&a,&p,11,22,Some("golden.jsonl".into()));
                let legacy=frozen_f21_main(&a,&p,11,22,Some("golden.jsonl".into()));
                assert_eq!(all_fields(&actual),all_fields(&legacy));
            }
        }}
    }
}

#[cfg(test)]
mod testy_bramek_rankingu {
    use super::*;
    use conduit_backtest::runner::RunResult;

    fn result() -> RunResult {
        serde_json::from_value(serde_json::json!({
            "metrics":Metrics::default(),"equity_curve":[],"daily":[],
            "ticks_processed":0,"elapsed_ms":0
        })).unwrap()
    }

    #[test]
    fn all_four_holds_block_progress_training_and_oos_before_metrics() {
        for (kind,field) in [
            ("COST","cost_reconciliation_required"),
            ("SR WARMUP","sr_warmup_reconciliation_required"),
            ("SIM EXECUTION","sim_execution_reconciliation_required"),
            ("CONTINUATION","continuation_reconciliation_required"),
        ] {
            let mut json=serde_json::to_value(result()).unwrap();
            json[field]="proof incomplete".into();
            let mut r:RunResult=serde_json::from_value(json).unwrap();
            // An attractive apparent profit must not override an invalid proof.
            r.metrics.total_profit=5_000_000.0;
            let err=publishable_run_metrics(&r,"progress").unwrap_err().to_string();
            assert!(err.contains(kind)); assert!(err.contains("proof incomplete"));
            assert!(score_reconciled_run(&r,"walk-forward training").is_err());
            assert!(require_reconciled_run(&r,"walk-forward out-of-sample").is_err());
        }
    }

    #[test]
    fn valid_legacy_score_is_exact_and_cancelled_is_never_published_as_complete() {
        let mut r=result();
        r.metrics.start_balance=600.0;
        r.metrics.end_equity=1000.0;
        r.metrics.total_profit=400.0;
        let legacy=score(&r.metrics);
        assert_eq!(score_reconciled_run(&r,"training").unwrap().to_bits(),legacy.to_bits());
        assert!(std::ptr::eq(publishable_run_metrics(&r,"progress").unwrap().unwrap(),&r.metrics));
        r.cancelled=true;
        assert!(publishable_run_metrics(&r,"progress").unwrap().is_none());
        assert!(score_reconciled_run(&r,"training").is_err());
        // Final CLI may still display an explicitly labelled partial diagnostic.
        assert!(require_reconciled_run(&r,"partial diagnostic").is_ok());
    }
}

#[cfg(test)]
mod testy_zegara_replayu {
    use super::*;

    #[test]
    fn cli_zero_zachowuje_offset_god_x4() {
        let s = Settings::default();
        let c = replay_clock_contract(0, "GOD-X4", &s).unwrap();
        assert_eq!(c.cli_signal_offset_ms, 0);
        assert_eq!(c.preset_msg_offset_ms, 10_800_000);
        assert_eq!(c.effective_clock_offset_ms, 10_800_000);
        assert_eq!(c.effective_dispatch_offset_ms, 10_800_000 + s.exec_latency_ms);
    }

    #[test]
    fn cli_offset_wymaga_zerowego_efektywnego_offsetu_preseta() {
        let mut s = Settings::default();
        s.msg_clock_offset_ms = Some(0);
        let c = replay_clock_contract(180, "jawne-UTC-do-serwera", &s).unwrap();
        assert_eq!(c.preset_server_tz_offset_ms, 10_800_000);
        assert_eq!(c.preset_msg_offset_ms, 0);
        assert_eq!(c.effective_clock_offset_ms, 10_800_000);
    }

    #[test]
    fn podwojny_offset_jest_bledem_z_podana_suma() {
        let e = replay_clock_contract(180, "GOD-X4", &Settings::default())
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("GOD-X4"));
        assert!(e.contains("+21600000 ms (+360.000 min)"));
        assert!(e.contains("msg_clock_offset_ms=0"));
    }

    #[test]
    fn przeciwne_offsety_tez_nie_moga_cicho_sie_znosic() {
        let e = replay_clock_contract(-180, "odwrotny", &Settings::default())
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("+0 ms (+0.000 min)"));
    }

    #[test]
    fn jeden_konflikt_blokuje_caly_zestaw_wariantow() {
        let mut zero = Settings::default();
        zero.msg_clock_offset_ms = Some(0);
        let presets = [("zero", zero), ("konflikt", Settings::default())];
        let result = presets
            .iter()
            .map(|(name, s)| replay_clock_contract(180, name, s))
            .collect::<Result<Vec<_>>>();
        assert!(result.err().unwrap().to_string().contains("konflikt"));
    }
}

#[cfg(test)]
mod testy_wynikow_czastkowych {
    use super::*;

    #[test]
    fn jeden_pisarz_publikuje_atomowo_pelne_metryki_z_wielu_watkow() {
        let d = std::env::temp_dir().join(format!("conduit-bt-wyniki-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();

        let mut p = PisarzWynikowCzastkowych::nowy(&d, 1);
        p.wyzeruj().unwrap();
        let p = Arc::new(Mutex::new(p));

        let watki: Vec<_> = (0..8)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    let mut m = Metrics::default();
                    m.total_profit = 1000.0 + i as f64;
                    m.worst_day = -50.0 - i as f64;
                    m.min_equity = 200.0 - i as f64;
                    m.min_margin_level = 150.0 + i as f64;
                    m.max_open_margin = 75.0 + i as f64;
                    m.stop_outs = i as u64;
                    m.trades = 10 + i;
                    p.lock().unwrap().dodaj(format!("preset-{i}"), m).unwrap();
                })
            })
            .collect();
        for w in watki {
            w.join().unwrap();
        }

        let txt = std::fs::read_to_string(d.join("wyniki_czastkowe.json")).unwrap();
        let wyniki: HashMap<String, Metrics> = serde_json::from_str(&txt).unwrap();
        assert_eq!(wyniki.len(), 8);
        let m = &wyniki["preset-3"];
        assert_eq!(m.total_profit, 1003.0);
        assert_eq!(m.worst_day, -53.0);
        assert_eq!(m.min_equity, 197.0);
        assert_eq!(m.min_margin_level, 153.0);
        assert_eq!(m.max_open_margin, 78.0);
        assert_eq!(m.stop_outs, 3);
        assert_eq!(m.trades, 13);
        assert!(!d.join("wyniki_czastkowe.json.tmp").exists());

        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn quick_partial_is_explicitly_wrapped_and_cannot_look_exact() {
        let d = std::env::temp_dir().join(format!(
            "conduit-bt-wyniki-quick-live-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();

        let mut p = PisarzWynikowCzastkowych::nowy(&d, 20);
        p.wyzeruj().unwrap();
        p.dodaj("candidate".into(), Metrics::default()).unwrap();

        let txt = std::fs::read_to_string(d.join("wyniki_czastkowe.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&txt).unwrap();
        assert_eq!(doc["approximate"], true);
        assert_eq!(doc["coronation_eligible"], false);
        assert_eq!(doc["quick_tick_stride"], 20);
        assert!(doc["results"]["candidate"].is_object());
        assert!(doc.get("candidate").is_none());

        let _ = std::fs::remove_dir_all(&d);
    }
}
