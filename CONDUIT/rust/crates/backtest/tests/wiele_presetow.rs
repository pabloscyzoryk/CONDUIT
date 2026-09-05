
use conduit_backtest::data::{ReplayMessage, TickData};
use conduit_backtest::runner::{run, FormatCfg, RunConfig};
use conduit_core::formaty::PulapyGlobalne;
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

/// Ustawienia bez ani jednego filtra — test ma mierzyć ROUTING i PUŁAPY,
/// a nie to, czy bramka sesji akurat przepuściła godzinę.
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
    s.regime_filter = conduit_core::settings::RegimeFilter::Off;
    s
}

const T0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;
const N: usize = 3 * 24 * 60;

/// Trzy doby ticków co minutę, cena PŁASKA — żadne zlecenie nie ma prawa
/// zamknąć się samo, więc liczba otwartych pozycji jest sterowana wyłącznie
/// przez bramki wejścia. O to chodzi w teście pułapu.
fn dane(dir: &str) -> (TickData, std::path::PathBuf) {
    let d = std::env::temp_dir().join(dir);
    std::fs::create_dir_all(&d).unwrap();
    let plik = d.join("ticks.bin");
    let ticki: Vec<(Ts, f32, f32)> = (0..N)
        .map(|i| (T0 + i as i64 * 60_000, 4000.0f32, 4000.20f32))
        .collect();
    zapisz_ticki(&plik, &ticki);
    let t = TickData::open(&plik).unwrap();
    (t, plik)
}

/// Wejście rynkowe w cenę bieżącą, ze stopem i celem daleko poza zasięgiem
/// płaskiego rynku. Pozycja zostaje otwarta do końca przebiegu.
fn sygnal(nr: i64, minuta: i64, kanal: &str) -> ReplayMessage {
    ReplayMessage {
        // znacznik jest w czasie Telegrama; runner dodaje strefę serwera
        ts: T0 + minuta * 60_000 - 3 * 3_600_000,
        telegram_published_ts: None,
        msg_id: nr,
        reply_to: None,
        edit_of: None,
        text: "BUY GOLD @ 4001.00/3999.00\nTP 4200.00\nSL 3800.00".to_string(),
        kanal: kanal.to_string(),
    }
}

fn cfg_wielu(formaty: Vec<(&str, Settings)>, pulapy: PulapyGlobalne) -> RunConfig {
    RunConfig {
        from: T0,
        to: T0 + 3 * 86_400_000,
        start_balance: 100_000.0,
        settings: ustawienia(),
        formaty: formaty
            .into_iter()
            .map(|(f, s)| FormatCfg {
                format: f.to_string(),
                preset: format!("P-{f}"),
                settings: s,
            })
            .collect(),
        pulapy,
        ..Default::default()
    }
}

// ============================================================
//  1. ROUTING PO KANALE
// ============================================================

/// SEDNO: wiadomość idzie do silnika SWOJEGO formatu i do żadnego innego.
///
/// Trzy sygnały `Synergy` i jeden `ZEN`. Gdyby routing nie działał, każdy
/// silnik zobaczyłby wszystkie cztery — czyli ten sam strumień policzony
/// dwa razy, na jednym rachunku, bez śladu w logach.
#[test]
fn kazdy_format_dostaje_wylacznie_swoje_sygnaly() {
    let (t, plik) = dane("conduit_test_wiele_routing");
    let msgs = vec![
        sygnal(1, 100, "Synergy"),
        sygnal(2, 200, "Synergy"),
        sygnal(3, 300, "Synergy"),
        sygnal(4, 400, "ZEN"),
    ];
    let cfg = cfg_wielu(
        vec![("Synergy", ustawienia()), ("ZEN", ustawienia())],
        PulapyGlobalne::default(),
    );
    let r = run(&t, &msgs, &cfg);

    assert_eq!(r.formaty.len(), 2, "dwa formaty = dwa silniki");
    let syn = r.formaty.iter().find(|f| f.format == "Synergy").unwrap();
    let zen = r.formaty.iter().find(|f| f.format == "ZEN").unwrap();
    assert_eq!(
        syn.koszyki, 3,
        "Synergy ma dostać DOKŁADNIE swoje trzy sygnały"
    );
    assert_eq!(zen.koszyki, 1, "ZEN ma dostać DOKŁADNIE swój jeden sygnał");
    assert_ne!(
        syn.slot, zen.slot,
        "dwa formaty nie mogą dzielić slotu koszyków"
    );
    assert!(
        r.bez_trasy.is_empty(),
        "oba kanały mają silnik, nic nie miało przepaść"
    );

    let _ = std::fs::remove_file(&plik);
}

// ============================================================
//  2. SYGNAŁ BEZ FORMATU — POLICZONY, NIE ZNIKNIĘTY
// ============================================================

/// Kanał bez presetu to stan POPRAWNY (nasłuchujemy, nie handlujemy) — ale
/// musi być WIDOCZNY. Cicha strata gałęzi sygnałów wygląda w tabeli dokładnie
/// tak samo jak zbiór, w którym tych sygnałów nigdy nie było.
#[test]
fn sygnal_bez_pasujacego_formatu_jest_policzony() {
    let (t, plik) = dane("conduit_test_wiele_brak");
    let msgs = vec![
        sygnal(1, 100, "Synergy"),
        sygnal(2, 200, "ZEN"),
        sygnal(3, 300, "ZEN"),
        sygnal(4, 400, ""),
    ];
    // Łańcuch handluje WYŁĄCZNIE Synergy.
    let cfg = cfg_wielu(vec![("Synergy", ustawienia())], PulapyGlobalne::default());
    let r = run(&t, &msgs, &cfg);

    assert_eq!(
        r.bez_trasy.get("ZEN"),
        Some(&2),
        "oba sygnały ZEN muszą być policzone"
    );
    assert_eq!(
        r.bez_trasy.get("(bez kanału)"),
        Some(&1),
        "sygnał bez kanału też jest pominięciem, a nie „zerem sygnałów”"
    );
    assert_eq!(
        r.metrics.odrzuty.get("BrakFormatu:ZEN"),
        Some(&2),
        "licznik musi trafić do metryk — inaczej nie zobaczy go nikt poza ekranem"
    );
    assert_eq!(r.formaty[0].koszyki, 1, "handluje tylko Synergy");

    let _ = std::fs::remove_file(&plik);
}

// ============================================================
//  3. PUŁAP GLOBALNY LICZY WSZYSTKIE SILNIKI RAZEM
// ============================================================

#[test]
fn pulap_globalny_ogranicza_sume_pozycji_obu_formatow() {
    let (t, plik) = dane("conduit_test_wiele_pulap");
    let mut msgs = Vec::new();
    for k in 0..3i64 {
        msgs.push(sygnal(10 + k, 100 + k * 20, "Synergy"));
        msgs.push(sygnal(20 + k, 110 + k * 20, "ZEN"));
    }
    let mut s = ustawienia();
    s.max_open_positions = 3;

    let bez = run(
        &t,
        &msgs,
        &cfg_wielu(
            vec![("Synergy", s.clone()), ("ZEN", s.clone())],
            PulapyGlobalne::default(),
        ),
    );
    let z = run(
        &t,
        &msgs,
        &cfg_wielu(
            vec![("Synergy", s.clone()), ("ZEN", s.clone())],
            PulapyGlobalne {
                max_pozycji: 4,
                ..Default::default()
            },
        ),
    );

    assert_eq!(
        bez.metrics.max_open_positions, 6,
        "bez pułapu dwa presety po 3 dają 6 pozycji na jednym rachunku — \
         to jest ta liczba, przed którą warstwa pułapów ma chronić"
    );
    assert_eq!(
        z.metrics.max_open_positions, 4,
        "pułap 4 ma zatrzymać piątą pozycję, choć ŻADEN preset nie łamie \
         swojego limitu 3"
    );

    let _ = std::fs::remove_file(&plik);
}

/// Pułap `0` znaczy BRAK PUŁAPU, a nie „limit wynosi zero".
///
/// Ta sama pułapka co przy `reenter_max`, która kosztowała projekt realny
/// rozjazd wyników. Tu: pułap zerowy musi dać DOKŁADNIE ten sam wynik, co brak
/// pułapów — inaczej domyślny `PulapyGlobalne::default()` cicho zamknąłby handel.
#[test]
fn pulap_zero_nie_zamyka_handlu() {
    let (t, plik) = dane("conduit_test_wiele_zero");
    let msgs = vec![sygnal(1, 100, "Synergy"), sygnal(2, 200, "ZEN")];
    let a = run(
        &t,
        &msgs,
        &cfg_wielu(
            vec![("Synergy", ustawienia()), ("ZEN", ustawienia())],
            PulapyGlobalne::default(),
        ),
    );
    let b = run(
        &t,
        &msgs,
        &cfg_wielu(
            vec![("Synergy", ustawienia()), ("ZEN", ustawienia())],
            PulapyGlobalne {
                max_pozycji: 0,
                max_koszykow: 0,
                ..Default::default()
            },
        ),
    );
    assert_eq!(a.metrics.max_open_positions, b.metrics.max_open_positions);
    assert!(a.metrics.max_open_positions >= 2, "oba formaty muszą wejść");

    let _ = std::fs::remove_file(&plik);
}

// ============================================================
//  4. JEDEN FORMAT = ŚCIEŻKA KLASYCZNA, CO DO CENTA
// ============================================================

#[test]
fn jeden_format_daje_to_samo_co_klasyczny_preset() {
    let (t, plik) = dane("conduit_test_wiele_parytet");
    let msgs: Vec<ReplayMessage> = (0..6i64)
        .map(|k| sygnal(k, 100 + k * 30, "Synergy"))
        .collect();

    let klasyczny = RunConfig {
        from: T0,
        to: T0 + 3 * 86_400_000,
        start_balance: 100_000.0,
        settings: ustawienia(),
        ..Default::default()
    };
    let a = run(&t, &msgs, &klasyczny);
    let b = run(
        &t,
        &msgs,
        &cfg_wielu(vec![("Synergy", ustawienia())], PulapyGlobalne::default()),
    );

    assert_eq!(
        format!("{:.6}", a.metrics.total_profit),
        format!("{:.6}", b.metrics.total_profit),
        "jeden format nie ma prawa zmienić wyniku ani o cent"
    );
    assert_eq!(a.metrics.trades, b.metrics.trades);
    assert_eq!(a.metrics.max_open_positions, b.metrics.max_open_positions);
    assert_eq!(
        format!("{:.6}", a.metrics.min_equity),
        format!("{:.6}", b.metrics.min_equity)
    );
    assert!(
        a.formaty.is_empty(),
        "ścieżka klasyczna nie produkuje rozbicia na formaty"
    );
    assert_eq!(b.formaty.len(), 1);

    let _ = std::fs::remove_file(&plik);
}

/// Ten sam parytet, ale w trybie „każdy dzień osobno" — bo to on jest osią
/// rankingu presetów i to w nim silniki są WYMIENIANE co dobę.
///
/// Wymiana silnika przy wielu formatach musi przywrócić każdemu jego SLOT;
/// bez tego po pierwszej północy oba formaty numerowałyby koszyki od `B1`
/// i widok brokera zacząłby pokazywać jednemu pozycje drugiego.
#[test]
fn parytet_jednego_formatu_takze_w_trybie_dziennym() {
    let (t, plik) = dane("conduit_test_wiele_parytet_d");
    let mut msgs: Vec<ReplayMessage> = Vec::new();
    for d in 0..3i64 {
        for k in 0..2i64 {
            msgs.push(sygnal(d * 10 + k, d * 1440 + 300 + k * 60, "Synergy"));
        }
    }
    let mut klasyczny = RunConfig {
        from: T0,
        to: T0 + 3 * 86_400_000,
        start_balance: 100_000.0,
        settings: ustawienia(),
        daily_reset: true,
        ..Default::default()
    };
    let a = run(&t, &msgs, &klasyczny);
    klasyczny.formaty = vec![FormatCfg {
        format: "Synergy".into(),
        preset: "P".into(),
        settings: ustawienia(),
    }];
    let b = run(&t, &msgs, &klasyczny);

    assert_eq!(
        format!("{:.6}", a.metrics.total_profit),
        format!("{:.6}", b.metrics.total_profit),
        "tryb dzienny: wymiana silnika nie ma prawa zmienić wyniku"
    );
    assert_eq!(a.metrics.trades, b.metrics.trades);
    // Slot musi PRZEŻYĆ wymianę silnika — inaczej koszyki drugiego dnia
    // wracają do numeracji od 1 i zderzają się z cudzymi.
    let slot = b.formaty[0].slot;
    assert!(slot > 0, "format dostaje własny slot");
    for k in b.baskets_dump.iter() {
        assert_eq!(
            conduit_core::wielosilnik::slot_koszyka(k.id),
            slot,
            "koszyk {} wypadł ze slotu formatu — wymiana silnika zgubiła numerację",
            k.id
        );
    }

    let _ = std::fs::remove_file(&plik);
}
