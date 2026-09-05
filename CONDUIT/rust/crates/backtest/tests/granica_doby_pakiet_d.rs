//! Pakiet D — granica doby i kurs wiadomości (D5, D6, D4, D3).
//!
//! Wszystkie cztery naprawy dotyczą PĘTLI backtestu, nie strategii, więc
//! testują się przez wynik przebiegu na tickach syntetycznych. Dane są
//! celowo trywialne: cena stała przez dzień i JEDNA luka nocna, żeby
//! każda różnica w liczbach dała się przypisać dokładnie jednej naprawie.

use conduit_backtest::data::{ReplayMessage, TickData};
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

/// Ustawienia bez ani jednego filtru — badamy księgowanie, nie strategię.
fn ustawienia() -> Settings {
    let mut s = Settings::default();
    s.session_filter = false;
    s.exec_latency_ms = 0;
    s.entry_units = 1;
    s.auto_limit = false;
    s.skip_if_sl_breached = false;
    s.journal_enabled = false;
    s.max_open_baskets = 0;
    s.max_open_positions = 0;
    s.streak_pause_n = 0;
    s.oae_timeout_min = 0.0;
    s.regime_filter = conduit_core::settings::RegimeFilter::Off;
    s.swap_enabled = false;
    s
}

fn katalog(nazwa: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("conduit_test_{nazwa}"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// D5: transakcje zamknięte NA GRANICY DOBY należą do dnia zamykanego.
///
/// Bez osi zamknięcia z `EodFlat` padają PO `daily.push`, a licznik zaraz
/// potem wraca do zera — więc suma `DayStat::trades` po wszystkich dniach
/// jest MNIEJSZA niż liczba transakcji przebiegu. To psuje tryb dzienny,
/// czyli główne kryterium wyboru presetu.
#[test]
fn d5_transakcje_z_granicy_doby_trafiaja_do_dnia_zamykanego() {
    let dir = katalog("d5_granica");
    let plik = dir.join("ticks.bin");

    // 4 doby ticków co minutę, cena płaska — nic nie zamknie się samo,
    // więc JEDYNYM źródłem zamknięć jest `EodFlat` na granicy doby.
    let t0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;
    let n = 4 * 24 * 60;
    let ticki: Vec<(Ts, f32, f32)> = (0..n)
        .map(|i| (t0 + i as i64 * 60_000, 4000.0, 4000.20))
        .collect();
    zapisz_ticki(&plik, &ticki);
    let dane = TickData::open(&plik).unwrap();

    // jedno wejście rynkowe dziennie, cele i stop daleko poza zasięgiem
    let msgs: Vec<ReplayMessage> = (0..4i64)
        .map(|d| ReplayMessage {
            kanal: String::new(),
            ts: t0 + d * 86_400_000 + 12 * 3_600_000 - 3 * 3_600_000,
            telegram_published_ts: None,
            msg_id: 1000 + d,
            reply_to: None,
            edit_of: None,
            text: "BUY GOLD @ 4000.50/3999.50\nTP 4200\nSL 3800".into(),
        })
        .collect();

    let przebieg = |os: bool| {
        let mut s = ustawienia();
        s.runner_ksiegowanie_v2 = os;
        let cfg = RunConfig {
            from: t0,
            to: t0 + 4 * 86_400_000,
            start_balance: 1000.0,
            settings: s,
            // płaska doba: zamykamy wszystko o północy, saldo przechodzi
            flat_na_dobie: true,
            ..Default::default()
        };
        let r = run(&dane, &msgs, &cfg);
        let suma_dni: u32 = r.daily.iter().map(|d| d.trades).sum();
        (r.metrics.trades, suma_dni)
    };

    let (trejdy_bez, dni_bez) = przebieg(false);
    let (trejdy_z, dni_z) = przebieg(true);

    println!(
        "bez osi: {trejdy_bez} transakcji, {dni_bez} w dniach   \
         z osią: {trejdy_z} transakcji, {dni_z} w dniach"
    );
    assert!(
        trejdy_bez > 0,
        "warunek testu: przebieg musi mieć transakcje"
    );
    assert_eq!(
        trejdy_bez, trejdy_z,
        "sama oś księgowania nie ma prawa zmienić LICZBY transakcji"
    );
    assert!(
        dni_bez < trejdy_bez,
        "warunek testu: bez osi transakcje z granicy doby mają GINĄĆ \
         ({dni_bez} z {trejdy_bez})"
    );
    assert_eq!(
        dni_z, trejdy_z,
        "z osią suma transakcji po dniach musi domykać się do przebiegu \
         ({dni_z} z {trejdy_z})"
    );

    let _ = std::fs::remove_file(&plik);
}

/// D5b: transakcje zamknięte KOMUNIKATEM też należą do dnia.
///
/// `hist_przed` był brany PO pętli wiadomości, więc każde wyjście zlecone
/// komunikatem — „TP1 HIT", „SL HIT", „CLOSE", bank przy RISK FREE — nie
/// trafiało do `DayStat::trades` żadnego dnia. Syntetyczny przypadek poniżej
/// pilnuje, aby taka transakcja należała do właściwej doby.
#[test]
fn d5b_zamkniecie_komunikatem_trafia_do_dnia() {
    let dir = katalog("d5b_komunikat");
    let plik = dir.join("ticks.bin");

    let t0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;
    let n = 3 * 24 * 60;
    let ticki: Vec<(Ts, f32, f32)> = (0..n)
        .map(|i| (t0 + i as i64 * 60_000, 4000.0, 4000.20))
        .collect();
    zapisz_ticki(&plik, &ticki);
    let dane = TickData::open(&plik).unwrap();

    // wejście o 12:00, a o 14:00 KOMUNIKAT zamykający koszyk — nie cena
    let mut msgs: Vec<ReplayMessage> = Vec::new();
    for d in 0..3i64 {
        msgs.push(ReplayMessage {
            kanal: String::new(),
            ts: t0 + d * 86_400_000 + 12 * 3_600_000 - 3 * 3_600_000,
            telegram_published_ts: None,
            msg_id: 3000 + d * 2,
            reply_to: None,
            edit_of: None,
            text: "BUY GOLD @ 4000.50/3999.50\nTP 4200\nSL 3800".into(),
        });
        msgs.push(ReplayMessage {
            kanal: String::new(),
            ts: t0 + d * 86_400_000 + 14 * 3_600_000 - 3 * 3_600_000,
            telegram_published_ts: None,
            msg_id: 3001 + d * 2,
            reply_to: None,
            edit_of: None,
            text: "CLOSE ALL".into(),
        });
    }

    let przebieg = |os: bool| {
        let mut s = ustawienia();
        s.runner_ksiegowanie_v2 = os;
        let cfg = RunConfig {
            from: t0,
            to: t0 + 3 * 86_400_000,
            start_balance: 1000.0,
            settings: s,
            ..Default::default()
        };
        let r = run(&dane, &msgs, &cfg);
        (
            r.metrics.trades,
            r.daily.iter().map(|d| d.trades).sum::<u32>(),
        )
    };

    let (trejdy_bez, dni_bez) = przebieg(false);
    let (trejdy_z, dni_z) = przebieg(true);
    println!("bez osi: {trejdy_bez}/{dni_bez}   z osią: {trejdy_z}/{dni_z}");

    assert!(
        trejdy_bez > 0,
        "warunek testu: komunikat ma zamknąć pozycje"
    );
    assert_eq!(
        trejdy_bez, trejdy_z,
        "oś księgowania nie zmienia LICZBY transakcji"
    );
    assert_eq!(
        dni_bez, 0,
        "warunek testu: bez osi zamknięcia z komunikatu mają GINĄĆ co do jednego"
    );
    assert_eq!(
        dni_z, trejdy_z,
        "z osią wszystkie zamknięcia z komunikatu muszą wejść do dni"
    );

    let _ = std::fs::remove_file(&plik);
}

/// D6: `EodFlat` nie ma prawa trafić do NOWEGO silnika.
///
/// Zamknięcia z `close_everything` czekają w kolejce brokera, a odbiera ją
/// dopiero `on_tick`. Bez drenażu przed wymianą silnika wczorajsze domknięcie
/// odbierał ŚWIEŻY silnik nowej doby, podbijał sobie `loss_streak` i przy
/// `streak_pause_n > 0` stawał na pauzę za cudzą stratę.
///
/// Widać to tak: cena płaska, więc jedyną stratą jest zapłacony spread przy
/// `EodFlat`. Z pauzą 24 h po JEDNEJ stracie przebieg bez osi handluje tylko
/// pierwszego dnia; z osią — każdego.
#[test]
fn d6_eodflat_nie_zatruwa_nowego_silnika() {
    let dir = katalog("d6_eodflat");
    let plik = dir.join("ticks.bin");

    let t0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;
    let n = 5 * 24 * 60;
    let ticki: Vec<(Ts, f32, f32)> = (0..n)
        .map(|i| (t0 + i as i64 * 60_000, 4000.0, 4000.20))
        .collect();
    zapisz_ticki(&plik, &ticki);
    let dane = TickData::open(&plik).unwrap();

    let msgs: Vec<ReplayMessage> = (0..5i64)
        .map(|d| ReplayMessage {
            kanal: String::new(),
            ts: t0 + d * 86_400_000 + 12 * 3_600_000 - 3 * 3_600_000,
            telegram_published_ts: None,
            msg_id: 2000 + d,
            reply_to: None,
            edit_of: None,
            text: "BUY GOLD @ 4000.50/3999.50\nTP 4200\nSL 3800".into(),
        })
        .collect();

    let przebieg = |os: bool| {
        let mut s = ustawienia();
        s.runner_ksiegowanie_v2 = os;
        // JEDNA strata = pauza na całą dobę
        s.streak_pause_n = 1;
        s.streak_pause_min = 1440.0;
        let cfg = RunConfig {
            from: t0,
            to: t0 + 5 * 86_400_000,
            start_balance: 1000.0,
            settings: s,
            flat_na_dobie: true,
            ..Default::default()
        };
        run(&dane, &msgs, &cfg).metrics.trades
    };

    let bez = przebieg(false);
    let z_osia = przebieg(true);
    println!("transakcje: bez osi {bez}, z osią {z_osia}");
    assert!(
        z_osia > bez,
        "bez osi wczorajszy EodFlat ma ZATRUWAĆ nową dobę pauzą po serii \
         strat (bez {bez}, z osią {z_osia})"
    );

    let _ = std::fs::remove_file(&plik);
}

/// D3: wiadomość z PRZERWY między tickami nie ma prawa znać kursu zza luki.
///
/// Sygnał rynkowy przychodzi w środku dziury w danych, po której cena skacze
/// o 40 $. Bez osi pozycja otwiera się po kursie PO luce (wiedza z przyszłości),
/// przy osi — po ostatnim kursie, który naprawdę padł.
#[test]
fn d3_wiadomosc_w_luce_nie_widzi_kursu_z_przyszlosci() {
    let dir = katalog("d3_luka");
    let plik = dir.join("ticks.bin");

    let t0: Ts = 1_775_000_000_000 - 1_775_000_000_000 % 86_400_000;
    // dwa bloki ticków rozdzielone sześciogodzinną dziurą i skokiem 4000 → 4040
    let mut ticki: Vec<(Ts, f32, f32)> = Vec::new();
    for i in 0..120i64 {
        ticki.push((t0 + 6 * 3_600_000 + i * 60_000, 4000.0, 4000.20));
    }
    for i in 0..600i64 {
        ticki.push((t0 + 14 * 3_600_000 + i * 60_000, 4040.0, 4040.20));
    }
    zapisz_ticki(&plik, &ticki);
    let dane = TickData::open(&plik).unwrap();

    // wiadomość w środku dziury (10:00 czasu serwera); strefa obejmuje OBIE
    // ceny, więc wejście rynkowe wykona się niezależnie od tego, którą
    // z nich zobaczy — różni się tylko cena otwarcia
    let msgs = vec![ReplayMessage {
        kanal: String::new(),
        ts: t0 + 10 * 3_600_000 - 3 * 3_600_000,
        telegram_published_ts: None,
        msg_id: 7,
        reply_to: None,
        edit_of: None,
        text: "BUY GOLD @ 4060/3980\nTP 4500\nSL 3500".into(),
    }];

    let przebieg = |os: bool| {
        let mut s = ustawienia();
        s.msg_kurs_sprzed_luki = os;
        let cfg = RunConfig {
            from: t0,
            to: t0 + 86_400_000,
            start_balance: 1000.0,
            settings: s,
            ..Default::default()
        };
        let r = run(&dane, &msgs, &cfg);
        r.baskets_dump.len()
    };
    // sam fakt otwarcia koszyka nie zależy od osi — to warunek testu
    assert_eq!(przebieg(false), 1, "warunek testu: koszyk ma powstać");
    assert_eq!(przebieg(true), 1, "oś nie ma prawa zgubić koszyka");

    let cena = |os: bool| {
        let mut s = ustawienia();
        s.msg_kurs_sprzed_luki = os;
        let cfg = RunConfig {
            from: t0,
            to: t0 + 86_400_000,
            start_balance: 1000.0,
            settings: s,
            ..Default::default()
        };
        let r = run(&dane, &msgs, &cfg);
        r.equity_curve
            .iter()
            .map(|(_, e)| *e)
            .fold(f64::INFINITY, f64::min)
    };
    let bez = cena(false);
    let z_osia = cena(true);
    println!("min equity: bez osi {bez:.4}, z osią {z_osia:.4}");
    assert!(
        (bez - z_osia).abs() > 1e-6,
        "oś D3 musi zmienić cenę wejścia po luce 40 $ — a nie zmieniła nic \
         (bez {bez:.4}, z osią {z_osia:.4})"
    );

    let _ = std::fs::remove_file(&plik);
}
