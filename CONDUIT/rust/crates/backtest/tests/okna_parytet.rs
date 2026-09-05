//! KONTRAKT MODUŁU `okna`: `--reset-co 1` bez ZZN to DOKŁADNIE `--daily-reset`.
//!
//! Okna przesuwane mają własną pętlę (N torów naraz), bo jednotorowa pętla
//! `runner.rs` fizycznie tego nie udźwignie. Duplikat pętli zawsze prędzej czy
//! później odjeżdża od oryginału — i wtedy nikt tego nie zauważa, bo obie
//! strony dają „jakieś" liczby. Ten plik jest jedynym zabezpieczeniem: jeżeli
//! którakolwiek pętla się zmieni, a druga nie, testy tutaj puszczą alarm.
//!
//! Testy poniżej używają wyłącznie syntetycznych danych i porównują obie
//! ścieżki bez odwołania do prywatnego korpusu lub wyników historycznych.
//! Tu odtwarzamy ją na danych syntetycznych, żeby test był samowystarczalny.

use conduit_backtest::data::{ReplayMessage, TickData};
use conduit_backtest::okna::{uruchom, KonfOkien};
use conduit_backtest::runner::{run, RunConfig};
use conduit_core::settings::Settings;
use conduit_core::types::Ts;

fn zapisz_ticki(sciezka: &std::path::Path, ticki: &[(Ts, f32, f32)]) {
    let mut buf = vec![0u8; 64];
    buf[0..4].copy_from_slice(&0x4B54_4443u32.to_le_bytes());
    buf[8..16].copy_from_slice(&(ticki.len() as u64).to_le_bytes());
    for (ts, bid, ask) in ticki {
        buf.extend_from_slice(&ts.to_le_bytes());
        buf.extend_from_slice(&bid.to_le_bytes());
        buf.extend_from_slice(&ask.to_le_bytes());
    }
    std::fs::write(sciezka, buf).unwrap();
}

fn ustawienia() -> Settings {
    let mut s = Settings::default();
    s.session_filter = false;
    s.exec_latency_ms = 0;
    s.entry_units = 1;
    s.skip_if_sl_breached = false;
    s.journal_enabled = false;
    s.max_open_baskets = 0;
    s.max_open_positions = 0;
    s.streak_pause_n = 0;
    s.oae_timeout_min = 0.0;
    s
}

/// DNI: 10 dób po minucie ticka, cena faluje sinusoidą o amplitudzie 8 $.
/// Codziennie o 10:00 wpada sygnał kupna w strefie wokół ceny bieżącej, z celem
/// i stopem na tyle blisko, żeby część dni kończyła się zyskiem, a część stratą.
fn dane(dir: &std::path::Path, dni: i64) -> (TickData, Vec<ReplayMessage>, Ts) {
    let plik = dir.join("ticks.bin");
    let t0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;
    let n = dni * 24 * 60;
    let ticki: Vec<(Ts, f32, f32)> = (0..n)
        .map(|i| {
            let ts = t0 + i * 60_000;
            let f = i as f32 / 240.0;
            let px = 4000.0 + 8.0 * f.sin() + (i as f32) * 0.002;
            (ts, px, px + 0.20)
        })
        .collect();
    zapisz_ticki(&plik, &ticki);
    let dane = TickData::open(&plik).unwrap();

    let mut msgs: Vec<ReplayMessage> = Vec::new();
    for d in 0..dni {
        let ts_tick = t0 + d * 86_400_000 + 10 * 3_600_000;
        let i = (ts_tick - t0) / 60_000;
        let f = i as f64 / 240.0;
        let px = 4000.0 + 8.0 * f.sin() + (i as f64) * 0.002;
        msgs.push(ReplayMessage {
            kanal: String::new(),
            // znacznik wiadomości jest w czasie Telegrama (UTC) — runner i okna
            // dodają do niego tę samą strefę serwera
            ts: ts_tick - 3 * 3_600_000,
            telegram_published_ts: None,
            msg_id: 1000 + d,
            reply_to: None,
            edit_of: None,
            text: format!(
                "BUY GOLD @ {:.2}/{:.2}\nTP {:.2}\nSL {:.2}",
                px + 0.6,
                px - 0.6,
                px + 3.0,
                px - 3.0
            ),
        });
    }
    (dane, msgs, t0)
}

/// Wariant danych dla ZZN: sygnał o 22:00, cele i stopy poza zasięgiem ruchu.
/// Koszyk nie ma jak się domknąć przed północą.
fn dane_przez_polnoc(dir: &std::path::Path, dni: i64) -> (TickData, Vec<ReplayMessage>, Ts) {
    let plik = dir.join("ticks.bin");
    let t0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;
    let n = dni * 24 * 60;
    let ticki: Vec<(Ts, f32, f32)> = (0..n)
        .map(|i| {
            let ts = t0 + i * 60_000;
            let f = i as f32 / 240.0;
            let px = 4000.0 + 8.0 * f.sin();
            (ts, px, px + 0.20)
        })
        .collect();
    zapisz_ticki(&plik, &ticki);
    let dane = TickData::open(&plik).unwrap();

    let mut msgs: Vec<ReplayMessage> = Vec::new();
    for d in 0..dni {
        let ts_tick = t0 + d * 86_400_000 + 22 * 3_600_000;
        let i = (ts_tick - t0) / 60_000;
        let f = i as f64 / 240.0;
        let px = 4000.0 + 8.0 * f.sin();
        msgs.push(ReplayMessage {
            kanal: String::new(),
            ts: ts_tick - 3 * 3_600_000,
            telegram_published_ts: None,
            msg_id: 2000 + d,
            reply_to: None,
            edit_of: None,
            text: format!(
                "BUY GOLD @ {:.2}/{:.2}\nTP {:.2}\nSL {:.2}",
                px + 0.6,
                px - 0.6,
                px + 400.0,
                px - 400.0
            ),
        });
    }
    (dane, msgs, t0)
}

#[test]
fn reset_co_1_to_dokladnie_daily_reset() {
    let dir = std::env::temp_dir().join("conduit_test_okna_parytet");
    std::fs::create_dir_all(&dir).unwrap();
    let dni = 10i64;
    let (ticks, msgs, t0) = dane(&dir, dni);
    let to = t0 + dni * 86_400_000;

    let r = run(
        &ticks,
        &msgs,
        &RunConfig {
            from: t0,
            to,
            start_balance: 200.0,
            settings: ustawienia(),
            daily_reset: true,
            ..Default::default()
        },
    );
    let w = uruchom(
        &ticks,
        &msgs,
        &KonfOkien {
            from: t0,
            to,
            start_balance: 200.0,
            settings: ustawienia(),
            n_dni: 1,
            zzn: false,
            ..Default::default()
        },
    );

    assert_eq!(
        w.okna.len(),
        r.daily.len(),
        "liczba okien przy n=1 musi się równać liczbie dni handlowych z runnera"
    );
    // Dzień po dniu, nie tylko suma: zgodna suma przy rozjechanych dniach
    // znaczyłaby, że dwa błędy się znoszą.
    for (o, d) in w.okna.iter().zip(r.daily.iter()) {
        assert_eq!(o.od, d.date, "okno {} opisuje inny dzień niż runner", o.nr);
        assert!(
            (o.zysk - d.profit).abs() < 1e-9,
            "dzień {}: okna {:.10} $, runner {:.10} $",
            d.date,
            o.zysk,
            d.profit
        );
    }
    assert!(
        (w.suma - r.metrics.total_profit).abs() < 1e-9,
        "suma okien {:.6} ≠ zysk runnera {:.6}",
        w.suma,
        r.metrics.total_profit
    );

    let _ = std::fs::remove_file(dir.join("ticks.bin"));
}

#[test]
fn credit_okna_no_trades_bonus_is_not_profit_or_drawdown() {
    let dir = std::env::temp_dir().join(format!("conduit_okna_credit_empty_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (ticks, _, t0) = dane(&dir, 3);
    for zzn in [false, true] {
        for n_dni in [1, 2] {
            for odlicz in [false, true] {
                let mut s = ustawienia();
                s.credit_balance_separate = true;
                s.kredyt_reczny = 300.0;
                s.odlicz_kredyt = odlicz;
                let w = uruchom(&ticks, &[], &KonfOkien {
                    from: t0, to: t0 + 3 * 86_400_000, start_balance: 600.0,
                    settings: s, n_dni, zzn, ..Default::default()
                });
                assert_eq!(w.okna.len(), 4 - n_dni as usize);
                assert_eq!(w.suma, 0.0);
                assert_eq!(w.min_equity_globalne, 600.0);
                for o in &w.okna {
                    assert_eq!(o.zysk, 0.0);
                    assert_eq!(o.zysk_do_granicy, 0.0);
                    assert_eq!(o.wklad_ogona(), 0.0);
                    assert_eq!(o.min_equity, 600.0);
                    assert_eq!(o.max_dd, 0.0);
                    assert_eq!(o.trejdy, 0);
                    assert_eq!(o.initial_credit, Some(300.0));
                    assert_eq!(o.raw_broker_boundary_equity, Some(900.0));
                    assert_eq!(o.raw_broker_end_equity, Some(900.0));
                    assert_eq!(o.raw_broker_min_equity, Some(900.0));
                    assert_eq!(o.reporting_equity_basis.as_deref(), Some("own_equity_excluding_constant_credit"));
                }
            }
        }
    }
    drop(ticks);
    std::fs::remove_file(dir.join("ticks.bin")).unwrap();
}

#[test]
fn credit_okna_daily_reset_parity_with_real_trades() {
    let dir = std::env::temp_dir().join(format!("conduit_okna_credit_daily_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (ticks, msgs, t0) = dane(&dir, 4);
    for odlicz in [false, true] {
        let mut s = ustawienia();
        s.credit_balance_separate = true;
        s.kredyt_reczny = 300.0;
        s.odlicz_kredyt = odlicz;
        let r = run(&ticks, &msgs, &RunConfig {
            from: t0, to: t0 + 4 * 86_400_000, start_balance: 600.0,
            settings: s.clone(), daily_reset: true, ..Default::default()
        });
        let w = uruchom(&ticks, &msgs, &KonfOkien {
            from: t0, to: t0 + 4 * 86_400_000, start_balance: 600.0,
            settings: s, n_dni: 1, ..Default::default()
        });
        assert!(r.metrics.trades > 0, "fixture must really trade");
        assert_eq!(w.okna.len(), r.daily.len());
        for (o, d) in w.okna.iter().zip(&r.daily) {
            assert_eq!(o.od, d.date);
            assert!((o.zysk - d.profit).abs() < 1e-9, "{}: {} != {}", o.od, o.zysk, d.profit);
            assert_eq!(o.initial_credit, Some(300.0));
            assert!((o.raw_broker_boundary_equity.unwrap() - 300.0 - 600.0 - o.zysk).abs() < 1e-9);
        }
        assert!((w.suma - r.metrics.total_profit).abs() < 1e-9);
    }
    drop(ticks);
    std::fs::remove_file(dir.join("ticks.bin")).unwrap();
}

#[test]
fn credit_okna_zero_credit_on_off_identical_and_legacy_shape() {
    let dir = std::env::temp_dir().join(format!("conduit_okna_credit_zero_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (ticks, msgs, t0) = dane(&dir, 3);
    for zzn in [false, true] {
        let mut cfg = KonfOkien {
            from: t0, to: t0 + 3 * 86_400_000, start_balance: 600.0,
            settings: ustawienia(), n_dni: 1, zzn, ..Default::default()
        };
        cfg.settings.kredyt_reczny = 0.0;
        let off = uruchom(&ticks, &msgs, &cfg);
        cfg.settings.credit_balance_separate = true;
        let on = uruchom(&ticks, &msgs, &cfg);
        assert_eq!(serde_json::to_value(&on.okna).unwrap(), serde_json::to_value(&off.okna).unwrap());
        assert_eq!(on.suma.to_bits(), off.suma.to_bits());
        assert!(!serde_json::to_string(&off.okna).unwrap().contains("initial_credit"));
        assert!(!serde_json::to_string(&off.okna).unwrap().contains("raw_broker"));
        // Old JSON still deserializes to absent metadata.
        let old: Vec<conduit_backtest::okna::WynikOkna> =
            serde_json::from_value(serde_json::to_value(&off.okna).unwrap()).unwrap();
        assert!(old.iter().all(|o| o.reporting_equity_basis.is_none()));
    }
    drop(ticks);
    std::fs::remove_file(dir.join("ticks.bin")).unwrap();
}

#[test]
fn credit_okna_global_account_overlay_matches_single_format() {
    use conduit_backtest::runner::FormatCfg;
    let dir = std::env::temp_dir().join(format!("conduit_okna_credit_chain_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (ticks, mut msgs, t0) = dane(&dir, 3);
    for m in &mut msgs { m.kanal = "Synergy".into(); }
    let mut s = ustawienia();
    s.credit_balance_separate = true;
    s.kredyt_reczny = 300.0;
    let mut cfg = KonfOkien {
        from: t0, to: t0 + 3 * 86_400_000, start_balance: 600.0,
        settings: s.clone(), n_dni: 1, ..Default::default()
    };
    let single = uruchom(&ticks, &msgs, &cfg);
    // Deliberately contradictory preset: account contract must win.
    s.credit_balance_separate = false;
    s.kredyt_reczny = 0.0;
    cfg.formaty.push(FormatCfg { format: "Synergy".into(), preset: "FIXTURE".into(), settings: s });
    let chain = uruchom(&ticks, &msgs, &cfg);
    assert!(single.okna.iter().map(|o| o.trejdy).sum::<u32>() > 0);
    assert_eq!(serde_json::to_value(&single.okna).unwrap(), serde_json::to_value(&chain.okna).unwrap());
    drop(ticks);
    std::fs::remove_file(dir.join("ticks.bin")).unwrap();
}

#[test]
fn credit_okna_first_loss_boundary_and_zzn_tail_use_same_own_basis() {
    use conduit_core::settings::{MarketEntryMode, TpSchedule};
    let dir = std::env::temp_dir().join(format!("conduit_okna_credit_tail_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ticks.bin");
    let day = 86_400_000;
    let t0 = 1_775_000_000_000 - 1_775_000_000_000 % day;
    zapisz_ticki(&file, &[
        (t0 + 1_000, 4000.0, 4000.2),
        (t0 + 2_000, 3998.0, 3998.2),
        (t0 + day, 3996.0, 3996.2),
        (t0 + day + 1_000, 4002.0, 4002.2),
        (t0 + day + 2_000, 4002.0, 4002.2),
    ]);
    let ticks = TickData::open(&file).unwrap();
    let msgs = vec![
        ReplayMessage { ts: t0 + 1_000, telegram_published_ts: None, msg_id: 1, reply_to: None, edit_of: None,
            text: "BUY GOLD @ 4001/3999\nTP 4050\nSL 3990".into(), kanal: String::new() },
        ReplayMessage { ts: t0 + day + 1_000, telegram_published_ts: None, msg_id: 2, reply_to: Some(1), edit_of: None,
            text: "CLOSE ALL".into(), kanal: String::new() },
    ];
    let mut s = ustawienia();
    s.server_tz_offset_ms = 0;
    s.credit_balance_separate = true;
    s.kredyt_reczny = 300.0;
    s.runner_ksiegowanie_v2 = true;
    s.market_entry_mode = MarketEntryMode::Single;
    s.tp_schedule = TpSchedule::AllAtTp1;
    s.lot_mode_percent = false;
    s.lot_fixed = 0.01;
    s.swap_enabled = false;
    s.commission_per_lot = 0.0;
    s.slippage_pts = 0.0;
    let cfg = KonfOkien { from: t0, to: t0 + day, start_balance: 600.0,
        settings: s, zzn: true, ..Default::default() };
    let w = uruchom(&ticks, &msgs, &cfg);
    assert_eq!(w.okna.len(), 1);
    let o = &w.okna[0];
    assert_eq!(o.trejdy, 1, "fixture must open and close exactly one position");
    assert_eq!(o.pozycje_na_granicy, 1, "{o:?}");
    assert!(o.zysk_do_granicy < -4.0 && o.zysk_do_granicy > -4.3);
    assert!(o.zysk > 1.7 && o.zysk < 2.0);
    assert!((o.wklad_ogona() - 6.0).abs() < 1e-8);
    assert!((o.max_dd + o.zysk_do_granicy).abs() < 1e-8);
    assert!((o.min_equity - (600.0 + o.zysk_do_granicy)).abs() < 1e-8);
    assert!((o.raw_broker_boundary_equity.unwrap() - 900.0 - o.zysk_do_granicy).abs() < 1e-8);
    assert!((o.raw_broker_end_equity.unwrap() - 900.0 - o.zysk).abs() < 1e-8);
    drop(ticks);
    std::fs::remove_file(file).unwrap();
}

/// Na ticku przebijającym TP nadchodzi też CLOSE. Strict musi najpierw
/// rozliczyć brokerski TP, legacy zamyka po bieżącej cenie komunikatem.
/// Różne wyniki trybów dowodzą, że fixture naprawdę mierzy kolejność.
#[test]
fn reset_co_1_respektuje_strict_tick_order_jak_runner() {
    use conduit_core::settings::{MarketEntryMode, TpSchedule};

    let dir = std::env::temp_dir().join(format!("conduit_okna_strict_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ticks.bin");
    let t0: Ts = 1_775_100_000_000;
    zapisz_ticki(&file, &[
        (t0, 4000.0, 4000.2),
        (t0 + 1_000, 4005.0, 4005.2),
        (t0 + 2_000, 4005.0, 4005.2),
    ]);
    let ticks = TickData::open(&file).unwrap();
    let msgs = vec![
        ReplayMessage {
            ts: t0, telegram_published_ts: None, msg_id: 1, reply_to: None, edit_of: None,
            text: "BUY GOLD @ 4001/3999\nTP 4002\nSL 3990".into(),
            kanal: String::new(),
        },
        ReplayMessage {
            ts: t0 + 1_000, telegram_published_ts: None, msg_id: 2, reply_to: Some(1), edit_of: None,
            text: "CLOSE ALL".into(), kanal: String::new(),
        },
    ];
    let mut results = Vec::new();
    for strict in [false, true] {
        let mut s = ustawienia();
        s.server_tz_offset_ms = 0;
        s.live_tick_order_strict = strict;
        s.runner_ksiegowanie_v2 = true;
        s.market_entry_mode = MarketEntryMode::Single;
        s.tp_schedule = TpSchedule::AllAtTp1;
        let r = run(&ticks, &msgs, &RunConfig {
            from: t0, to: t0 + 3_000, start_balance: 200.0,
            settings: s.clone(), daily_reset: true, ..Default::default()
        });
        let w = uruchom(&ticks, &msgs, &KonfOkien {
            from: t0, to: t0 + 3_000, start_balance: 200.0,
            settings: s, n_dni: 1, zzn: false, ..Default::default()
        });
        assert_eq!(w.okna.len(), 1);
        assert!(r.metrics.trades > 0, "fixture musi rzeczywiście handlować");
        assert_eq!(w.okna[0].trejdy, r.metrics.trades);
        assert!((w.suma - r.metrics.total_profit).abs() < 1e-9,
            "strict={strict}: okna={} runner={}", w.suma, r.metrics.total_profit);
        results.push(r.metrics.total_profit);
    }
    assert!((results[0] - results[1]).abs() > 0.01,
        "fixture musi rozróżniać legacy/strict, inaczej test kolejności jest pusty");
    drop(ticks);
    std::fs::remove_file(file).unwrap();
}

/// Okna są PRZESUWANE o jeden dzień, nie rozłączne: przy D dniach i oknie N
/// pełnych okien jest D − N + 1, a każde zaczyna się dzień po poprzednim.
#[test]
fn okna_sa_przesuwane_o_jeden_dzien() {
    let dir = std::env::temp_dir().join("conduit_test_okna_przesuw");
    std::fs::create_dir_all(&dir).unwrap();
    let dni = 10i64;
    let (ticks, msgs, t0) = dane(&dir, dni);
    let to = t0 + dni * 86_400_000;

    let dniowe = uruchom(
        &ticks,
        &msgs,
        &KonfOkien {
            from: t0,
            to,
            start_balance: 200.0,
            settings: ustawienia(),
            n_dni: 1,
            ..Default::default()
        },
    );
    let d = dniowe.okna.len();
    assert_eq!(d, dni as usize, "10 dób syntetycznych = 10 dni handlowych");

    for n in 2..=5u32 {
        let w = uruchom(
            &ticks,
            &msgs,
            &KonfOkien {
                from: t0,
                to,
                start_balance: 200.0,
                settings: ustawienia(),
                n_dni: n,
                ..Default::default()
            },
        );
        assert_eq!(
            w.okna.len(),
            d - n as usize + 1,
            "n={n}: przy {d} dniach pełnych okien przesuwanych ma być {}",
            d - n as usize + 1
        );
        // pierwsze okno zaczyna się pierwszego dnia, kolejne dzień po dniu
        assert_eq!(
            w.okna[0].od, dniowe.okna[0].od,
            "n={n}: pierwsze okno startuje pierwszego dnia"
        );
        for (k, o) in w.okna.iter().enumerate() {
            assert_eq!(
                o.od,
                dniowe.okna[k].od,
                "n={n}: okno {} startuje o jeden dzień później",
                k + 1
            );
            assert_eq!(
                o.do_dnia,
                dniowe.okna[k + n as usize - 1].od,
                "n={n}: okno {} ma się kończyć {n}. dnia od startu",
                k + 1
            );
        }
    }

    let _ = std::fs::remove_file(dir.join("ticks.bin"));
}

/// Compounding DZIAŁA wewnątrz okna: suma okien N-dniowych nie może być
/// zwykłym przepisaniem sumy dni. Test przypina sam fakt, nie kwotę.
#[test]
fn wewnatrz_okna_dziala_compounding() {
    let dir = std::env::temp_dir().join("conduit_test_okna_compound");
    std::fs::create_dir_all(&dir).unwrap();
    let dni = 10i64;
    let (ticks, msgs, t0) = dane(&dir, dni);
    let to = t0 + dni * 86_400_000;

    let mut s = ustawienia();
    // lot proporcjonalny do kapitału — bez tego compounding nie ma jak się
    // objawić i test przechodziłby także dla zepsutej implementacji
    s.lot_mode_percent = true;
    s.lot_percent = 2.0;

    let w3 = uruchom(
        &ticks,
        &msgs,
        &KonfOkien {
            from: t0,
            to,
            start_balance: 200.0,
            settings: s.clone(),
            n_dni: 3,
            ..Default::default()
        },
    );
    assert!(!w3.okna.is_empty(), "okna 3-dniowe muszą powstać");
    // okno 3-dniowe MUSI widzieć więcej niż jeden dzień
    assert!(
        w3.okna.iter().any(|o| o.trejdy > 1),
        "okno 3-dniowe nie zebrało ani jednego dnia z więcej niż jedną transakcją"
    );
    let _ = std::fs::remove_file(dir.join("ticks.bin"));
}

/// ZZN nie może dokładać nowych koszyków w ogonie: `Engine::wygaszanie`
/// blokuje wejścia, a nie zarządzanie.
#[test]
fn zzn_nie_otwiera_nic_nowego_w_ogonie() {
    let dir = std::env::temp_dir().join("conduit_test_okna_zzn");
    std::fs::create_dir_all(&dir).unwrap();
    let dni = 10i64;
    // Osobne dane: sygnał o 22:00 z celem i stopem TAK DALEKO, że nic ich nie
    // trafia. Koszyk może więc skończyć wyłącznie na wygaśnięciu wieku — czyli
    // GWARANTOWANIE przechodzi przez północ. Bez tego warunku test przechodziłby
    // także dla ZZN, które nic nie robi.
    let (ticks, msgs, t0) = dane_przez_polnoc(&dir, dni);
    let to = t0 + dni * 86_400_000;

    let mut s = ustawienia();
    s.basket_max_age_min = 1800.0;

    let bez = uruchom(
        &ticks,
        &msgs,
        &KonfOkien {
            from: t0,
            to,
            start_balance: 200.0,
            settings: s.clone(),
            n_dni: 1,
            zzn: false,
            ..Default::default()
        },
    );
    let z = uruchom(
        &ticks,
        &msgs,
        &KonfOkien {
            from: t0,
            to,
            start_balance: 200.0,
            settings: s.clone(),
            n_dni: 1,
            zzn: true,
            ..Default::default()
        },
    );

    assert_eq!(bez.okna.len(), z.okna.len(), "ZZN nie zmienia LICZBY okien");
    assert!(
        bez.ucietych_koszykow > 0 || bez.ucietych_pozycji > 0,
        "warunek testu: bez ZZN coś MUSI zostać ucięte na granicy \
         (koszyków {}, pozycji {})",
        bez.ucietych_koszykow,
        bez.ucietych_pozycji
    );
    // liczba koszyków okna nie może urosnąć przez sam ogon
    for (a, b) in bez.okna.iter().zip(z.okna.iter()) {
        assert!(
            b.koszyki <= a.koszyki,
            "okno {}: ogon ZZN założył NOWE koszyki ({} → {})",
            a.nr,
            a.koszyki,
            b.koszyki
        );
    }
    let _ = std::fs::remove_file(dir.join("ticks.bin"));
}
