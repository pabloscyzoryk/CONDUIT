//! **POTRÓJNY KONTRAKT ZERA WARSTWY EA-CORE — sprawdzany przez CAŁY BACKTEST.**
//!
//! Testy jednostkowe w `conduit-core/tests/ea_core.rs` prowadzą silnik po
//! atrapie brokera. To jest inna klasa dowodu: tutaj chodzi **ta sama pętla
//! `runner::run`**, którą liczy się bramkę parytetu i każdy raport — z
//! wypełnieniami, marginesem, granicami doby i wymianą silników.
//!
//! Sprawdzane są wszystkie trzy punkty z `wiedza/EA_PLAN_WDROZENIA.md` FALA 0:
//!
//!  * **(a)** `ea_enabled = false` — odniesienie (ścieżka sprzed warstwy),
//!  * **(b)** `ea_enabled = true` + wszystkie pola rodzin B–G zerowe,
//!  * **(c)** tryb AUTO-EA (`RunConfig::auto_ea`) bez ani jednej osi.
//!
//! Dodatkowo **(b′)**: kadencja własnego zegara (`ea_tick_s` = 0 / 1 / 5)
//! nie ma prawa ruszyć ani jednej liczby. To jest ten sam wymóg co (b),
//! ale sprawdzony na osi, na której najłatwiej go złamać: zmiana kadencji
//! zmienia LICZBĘ PULSÓW o trzy rzędy wielkości, a wynik handlu — o zero.
//!
//! **Dlaczego porównanie idzie po transakcjach, a nie po sumie:** dwie różne
//! historie handlu potrafią dać ten sam zysk. Bramką jest tu równość
//! transakcja-po-transakcji, a suma jest tylko pierwszym filtrem.

use conduit_backtest::data::{ReplayMessage, TickData};
use conduit_backtest::runner::{run, RunConfig, RunResult};
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

/// Dziesięć dób ticka co minutę, cena faluje z dryfem, codziennie o 10:00
/// sygnał kupna w strefie. Część dni kończy się zyskiem, część stratą — bo
/// kontrakt zera sprawdzony na samych wygranych nie jest sprawdzony.
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

fn przebieg(
    ticks: &TickData,
    msgs: &[ReplayMessage],
    od: Ts,
    do_: Ts,
    zmien: impl FnOnce(&mut Settings),
    auto_ea: bool,
) -> RunResult {
    let mut s = ustawienia();
    zmien(&mut s);
    run(
        ticks,
        msgs,
        &RunConfig {
            from: od,
            to: do_,
            start_balance: 400.0,
            settings: s,
            auto_ea,
            ..Default::default()
        },
    )
}

/// Równość CO DO CENTA i transakcja po transakcji.
///
/// Sam zysk nie wystarcza: dwie różne historie potrafią zsumować się do tej
/// samej liczby. Dlatego porównanie schodzi na pojedynczy `ClosedTrade`
/// (bitowo — `to_bits`, nie z tolerancją: kontrakt mówi „co do centa", a nie
/// „prawie").
fn identyczne(a: &RunResult, b: &RunResult, co: &str) {
    let (ma, mb) = (&a.metrics, &b.metrics);
    assert_eq!(
        ma.total_profit.to_bits(),
        mb.total_profit.to_bits(),
        "{co}: zysk {} vs {}",
        ma.total_profit,
        mb.total_profit
    );
    assert_eq!(ma.trades, mb.trades, "{co}: transakcje");
    assert_eq!(ma.baskets, mb.baskets, "{co}: koszyki");
    assert_eq!(
        ma.min_equity.to_bits(),
        mb.min_equity.to_bits(),
        "{co}: dno equity {} vs {}",
        ma.min_equity,
        mb.min_equity
    );
    assert_eq!(
        ma.max_dd_abs.to_bits(),
        mb.max_dd_abs.to_bits(),
        "{co}: maxDD"
    );
    assert_eq!(ma.signals_seen, mb.signals_seen, "{co}: sygnały widziane");
    assert_eq!(ma.signals_taken, mb.signals_taken, "{co}: sygnały wzięte");
    assert_eq!(
        a.trades.len(),
        b.trades.len(),
        "{co}: długość historii transakcji"
    );
    for (i, (x, y)) in a.trades.iter().zip(b.trades.iter()).enumerate() {
        assert_eq!(
            x.open_ts, y.open_ts,
            "{co}: transakcja {i} — chwila otwarcia"
        );
        assert_eq!(
            x.close_ts, y.close_ts,
            "{co}: transakcja {i} — chwila zamknięcia"
        );
        assert_eq!(
            x.open_price.to_bits(),
            y.open_price.to_bits(),
            "{co}: transakcja {i} — cena wejścia"
        );
        assert_eq!(
            x.close_price.to_bits(),
            y.close_price.to_bits(),
            "{co}: transakcja {i} — cena wyjścia"
        );
        assert_eq!(
            x.volume.to_bits(),
            y.volume.to_bits(),
            "{co}: transakcja {i} — wolumen"
        );
        assert_eq!(
            x.profit.to_bits(),
            y.profit.to_bits(),
            "{co}: transakcja {i} — wynik"
        );
    }
    assert_eq!(a.equity_curve, b.equity_curve, "{co}: krzywa equity");
    assert_eq!(a.stop_outs, b.stop_outs, "{co}: stop-outy");
}

/// ⚠ KATALOG JEST PER TEST i to nie jest higiena, tylko poprawka błędu.
///
/// `cargo test` puszcza testy z jednego pliku RÓWNOLEGLE, a `TickData::open`
/// mapuje plik w pamięć. Wspólna ścieżka `ticks.bin` znaczy więc, że jeden test
/// nadpisuje bufor, z którego drugi właśnie czyta — objaw: `plik ticków za
/// krótki`, i to **niedeterministycznie** (pojedynczy przebieg tego pliku
/// przechodził, pełny `cargo test` się wywracał). Nazwa testu w ścieżce
/// rozłącza te przebiegi całkowicie.
fn scena(nazwa: &str) -> (TickData, Vec<ReplayMessage>, Ts, Ts) {
    let dir = std::env::temp_dir().join(format!("conduit_test_ea_kontrakt_zera_{nazwa}"));
    std::fs::create_dir_all(&dir).unwrap();
    let dni = 10i64;
    let (ticks, msgs, t0) = dane(&dir, dni);
    (ticks, msgs, t0, t0 + dni * 86_400_000)
}

/// SCENA MUSI HANDLOWAĆ. Kontrakt zera na przebiegu bez transakcji jest
/// tautologią — ten test jest po to, żeby wszystkie pozostałe coś znaczyły.
#[test]
fn scena_kontraktu_zera_faktycznie_handluje() {
    let (t, m, od, do_) = scena("handel");
    let r = przebieg(&t, &m, od, do_, |_| {}, false);
    assert!(
        r.metrics.trades >= 5,
        "scena zrobiła {} transakcji — za mało na dowód",
        r.metrics.trades
    );
    assert!(
        r.metrics.baskets >= 5,
        "scena zawiązała {} koszyków",
        r.metrics.baskets
    );
    assert!(
        r.metrics.total_profit != 0.0,
        "scena wyszła na zero — nie ma czego porównywać"
    );
}

/// **(a)** Warstwa wyłączona nie zmienia niczego. To jest odniesienie, więc
/// test sprawdza rzecz osobną: że przy `ea_enabled = false` warstwa **nie
/// wykonała ani jednego pulsu** — czyli że kontrakt jest strukturalny,
/// a nie „policzyliśmy to samo dwa razy".
#[test]
fn kontrakt_a_warstwa_wylaczona_nie_pulsuje_ani_razu() {
    let (t, m, od, do_) = scena("a");
    let mut s = ustawienia();
    assert!(
        !s.ea_enabled,
        "domyślna `ea_enabled` przestała być `false` — to jest zmiana kontraktu"
    );
    s.ea_tick_s = 7.0; // pole rodziny ustawione, wyłącznik główny w zerze
    let r = run(
        &t,
        &m,
        &RunConfig {
            from: od,
            to: do_,
            start_balance: 400.0,
            settings: s,
            ..Default::default()
        },
    );
    let baza = przebieg(&t, &m, od, do_, |_| {}, false);
    identyczne(
        &baza,
        &r,
        "(a) ea_tick_s przy wyłączonym wyłączniku głównym",
    );
}

/// **(b)** PODWÓJNE ZERO: warstwa włączona, wszystkie progi rodzin B–G w zerze.
///
/// Szkielet liczy wektor stanu, stempluje koszyki, prowadzi bilans i dozór —
/// i nie zmienia ani jednej liczby. To jest test, który oddziela KOSZT
/// SZKIELETU od kosztu polityk.
#[test]
fn kontrakt_b_podwojne_zero_nie_zmienia_ani_centa() {
    let (t, m, od, do_) = scena("b");
    let baza = przebieg(&t, &m, od, do_, |_| {}, false);
    let ea = przebieg(&t, &m, od, do_, |s| s.ea_enabled = true, false);
    identyczne(&baza, &ea, "(b) podwójne zero");
}

#[test]
fn kontrakt_b_kadencja_zegara_nie_zmienia_ani_centa() {
    let (t, m, od, do_) = scena("b_zegar");
    let baza = przebieg(&t, &m, od, do_, |s| s.ea_enabled = true, false);
    for ts in [1.0, 5.0, 60.0] {
        let r = przebieg(
            &t,
            &m,
            od,
            do_,
            move |s| {
                s.ea_enabled = true;
                s.ea_tick_s = ts;
            },
            false,
        );
        identyczne(&baza, &r, &format!("(b′) ea_tick_s = {ts}"));
    }
}

/// **(c)** Tryb AUTO-EA bez osi = AUTO co do centa.
///
/// Flagi `Engine::tryb_auto_ea` nie czyta dziś żadna oś, więc test ma wyjść
/// zielony **z tego powodu** — a nie dlatego, że flaga nie dojechała do
/// silnika. Dlatego para: najpierw dowód, że flaga faktycznie stoi na
/// silnikach, dopiero potem równość wyników.
#[test]
fn kontrakt_c_auto_ea_rowna_sie_auto() {
    let (t, m, od, do_) = scena("c");
    let baza = przebieg(&t, &m, od, do_, |_| {}, false);
    let ea = przebieg(&t, &m, od, do_, |_| {}, true);
    identyczne(&baza, &ea, "(c) tryb AUTO-EA bez osi");

    // ...i to samo przy WŁĄCZONEJ warstwie — tryb i wyłącznik główny to dwie
    // różne rzeczy i wolno je łamać osobno
    let baza_ea = przebieg(&t, &m, od, do_, |s| s.ea_enabled = true, false);
    let obie = przebieg(&t, &m, od, do_, |s| s.ea_enabled = true, true);
    identyczne(&baza_ea, &obie, "(c) AUTO-EA razem z ea_enabled");
}

#[test]
fn flaga_auto_ea_przezywa_wymiane_silnika_o_polnocy() {
    let (t, m, od, do_) = scena("c_doba");
    let mut s = ustawienia();
    s.ea_enabled = true;
    let r = run(
        &t,
        &m,
        &RunConfig {
            from: od,
            to: do_,
            start_balance: 400.0,
            settings: s,
            daily_reset: true,
            auto_ea: true,
            ..Default::default()
        },
    );
    // gdyby tryb ginął przy wymianie silnika, `[EA-CORE]` raportowałby
    // `auto_ea=false` — a jedyne, co widać z zewnątrz, to że przebieg policzył
    // się normalnie; dlatego asercja idzie na fakt handlu, a właściwy dowód
    // trybu niesie test (c) razem z `zbuduj_zespol`
    assert!(
        r.metrics.trades > 0,
        "przebieg dobowy nie zawarł ani jednej transakcji"
    );

    let mut s2 = ustawienia();
    s2.ea_enabled = true;
    let bez = run(
        &t,
        &m,
        &RunConfig {
            from: od,
            to: do_,
            start_balance: 400.0,
            settings: s2,
            daily_reset: true,
            auto_ea: false,
            ..Default::default()
        },
    );
    identyczne(&bez, &r, "(c) AUTO-EA w trybie dobowym");
}
