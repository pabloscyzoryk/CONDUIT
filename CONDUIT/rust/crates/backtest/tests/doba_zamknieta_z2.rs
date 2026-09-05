//! Z-2: strażnik kończy dzień — bramka wejść MUSI o tym wiedzieć.
//!
//! Defekt: `check_guards` zamykał wszystko po stopie dnia, po godzinie EOD
//! i przed weekendem, a `entry_gate` nie miała o tym pojęcia. Pierwszy sygnał
//! po zamknięciu otwierał pozycje od nowa, strażnik ścinał je na następnym
//! ticku — i tak do północy silnika. Każda taka para płaci pełny spread,
//! a w dzienniku wygląda jak normalny handel.
//!
//! Testy sprawdzają SKUTEK na koncie (czy powstała pozycja), a nie stan pola
//! wewnętrznego — bo to skutek kosztował pieniądze.

use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

/// Poniedziałek, równa doba UTC — żeby godziny w teście były godzinami
/// silnika, a piątkowa reguła nie odpalała się przypadkiem.
const PON: Ts = 1_700_524_800_000; // 2023-11-21 00:00:00 UTC (wtorek)

fn o_godzinie(h: i64) -> Ts {
    PON + h * 3_600_000
}

fn zrodlo() -> SourceKey {
    SourceKey::new(-1_000_000_000_301, None)
}

fn kwotowanie(ts: Ts, bid: f64) -> Quote {
    Quote {
        ts,
        bid,
        ask: bid + 0.20,
    }
}

fn wiadomosc(ts: Ts, id: i64, tekst: &str) -> IncomingMessage {
    IncomingMessage {
        ts,
        source: zrodlo(),
        source_name: "TEST".into(),
        msg_id: id,
        reply_to: None,
        edit_of: None,
        text: tekst.into(),
    }
}

/// Sygnał z celami tak daleko, że nie zostaną trafione — mierzymy bramkę,
/// a nie take-profit.
const SYGNAL: &str = "BUY GOLD @ 4005/4000\nTP 4200\nTP 4300\nTP 4400\nSL 3990";

/// Cena leży NAD strefą, więc wejście rynkowe wypełnia się od razu i test
/// mierzy bramkę, a nie to, czy cena zdążyła dojść do zlecenia oczekującego.
const CENA: f64 = 4012.0;

/// Ustawienia bez żadnej reguły kończącej dzień: baza do porównań.
fn baza() -> Settings {
    let mut cfg = Settings::default();
    cfg.session_filter = false;
    cfg.auto_limit = false; // wejście po rynku — natychmiastowe wypełnienie
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.entry_units = 1;
    cfg.max_open_positions = 5;
    cfg
}

fn stanowisko(cfg: Settings) -> (Engine, SimBroker) {
    let stops = cfg.stops_level;
    let mut b = SimBroker::new(1000.0, stops, 0.0);
    b.on_quote(kwotowanie(o_godzinie(1), CENA));
    let e = Engine::new(cfg, 1000.0);
    (e, b)
}

fn tik(e: &mut Engine, b: &mut SimBroker, ts: Ts, bid: f64) {
    let q = kwotowanie(ts, bid);
    b.on_quote(q);
    e.on_tick(b, &q);
}

fn sygnal(e: &mut Engine, b: &mut SimBroker, ts: Ts, id: i64) {
    let q = kwotowanie(ts, CENA);
    b.on_quote(q);
    e.on_message(b, &wiadomosc(ts, id, SYGNAL));
    e.on_tick(b, &q);
}

// ============================================================
//  GODZINA EOD
// ============================================================

#[test]
fn po_godzinie_eod_sygnal_nie_otwiera_juz_nic() {
    let mut cfg = baza();
    cfg.eod_flat_hour = 18.0;
    let (mut e, mut b) = stanowisko(cfg);

    sygnal(&mut e, &mut b, o_godzinie(10), 1);
    assert!(
        !b.positions().is_empty(),
        "sygnał przed EOD ma otworzyć pozycję"
    );

    // wchodzimy w godzinę EOD — strażnik opróżnia rachunek
    tik(&mut e, &mut b, o_godzinie(18), CENA);
    assert!(b.positions().is_empty(), "godzina EOD ma zamknąć wszystko");

    // ...i to jest koniec dnia, a nie zaproszenie do kolejnej rundy
    sygnal(&mut e, &mut b, o_godzinie(18) + 60_000, 2);
    assert!(
        b.positions().is_empty(),
        "Z-2: sygnał po godzinie EOD otworzył pozycję — strażnik zamknie ją \
         natychmiast, a konto zapłaci spread"
    );

    // godzina 19 to nadal ta sama doba: blokada obowiązuje do północy silnika
    sygnal(&mut e, &mut b, o_godzinie(19), 3);
    assert!(
        b.positions().is_empty(),
        "Z-2: blokada zeszła przed końcem doby"
    );
}

#[test]
fn nowa_doba_zdejmuje_blokade_sama() {
    let mut cfg = baza();
    cfg.eod_flat_hour = 18.0;
    let (mut e, mut b) = stanowisko(cfg);

    sygnal(&mut e, &mut b, o_godzinie(10), 1);
    tik(&mut e, &mut b, o_godzinie(18), CENA);
    sygnal(&mut e, &mut b, o_godzinie(19), 2);
    assert!(b.positions().is_empty(), "doba miała być zamknięta");

    // następny dzień, godzina 10 — handel wraca bez żadnego „wznów"
    tik(&mut e, &mut b, o_godzinie(24 + 9), CENA);
    sygnal(&mut e, &mut b, o_godzinie(24 + 10), 3);
    assert!(
        !b.positions().is_empty(),
        "blokada doby przeżyła granicę doby — dzień po EOD byłby martwy"
    );
}

// ============================================================
//  PIĄTKOWE WYPŁASZCZENIE
// ============================================================

#[test]
fn po_flat_weekend_piatek_jest_zamkniety_do_konca() {
    let mut cfg = baza();
    cfg.flat_weekend = true;
    cfg.flat_weekend_hour = 20.0;
    let (mut e, mut b) = stanowisko(cfg);

    // PON to wtorek; piątek = +3 doby
    let piatek = |h: i64| o_godzinie(3 * 24 + h);
    assert_eq!(weekday_of(piatek(0), 0), 4, "test celuje w piątek");

    tik(&mut e, &mut b, piatek(9), CENA);
    sygnal(&mut e, &mut b, piatek(10), 1);
    assert!(
        !b.positions().is_empty(),
        "piątek przed 20 handluje normalnie"
    );

    tik(&mut e, &mut b, piatek(20), CENA);
    assert!(b.positions().is_empty(), "wypłaszczenie przed weekendem");

    // ⚠ TU NIE WOLNO PATRZEĆ NA `positions()`.
    //
    // Po godzinie `flat_weekend_hour` strażnik zamyka wszystko NA KAŻDYM
    // ticku, a `sygnal()` woła `on_tick` zaraz po `on_message`. Pozycja
    // otwarta i ścięta w tym samym kroku zostawia rachunek pusty — czyli
    // dokładnie tak samo, jak gdyby bramka jej nie wpuściła. Sprawdzone:
    // ta asercja przechodziła RÓWNIEŻ z wyłączoną poprawką, czyli nie
    // mierzyła niczego.
    //
    // Młócka spreadowa zostawia ślad w HISTORII, nie w stanie — i to jest
    // jedyna rzecz, która odróżnia „nie wpuszczono" od „wpuszczono
    // i natychmiast zamknięto ze stratą spreadu".
    let przed = b.history.len();
    sygnal(&mut e, &mut b, piatek(21), 2);
    assert_eq!(
        b.history.len(),
        przed,
        "Z-2: sygnał po piątkowym wypłaszczeniu wszedł na rachunek i został \
         ścięty w tym samym kroku — to jest para otwórz-zamknij za pełny spread"
    );
}

// ============================================================
//  PARYTET — bez tych reguł nic się nie zmienia
// ============================================================

#[test]
fn bez_regul_konczacych_dzien_bramka_zostaje_otwarta() {
    // Dokładnie ta konfiguracja, którą mają wszystkie presety bramki
    // parytetu: `eod_flat_hour = 0`, `flat_weekend = false`,
    // `day_trail_stop_pct = 0`. Żadna z trzech gałęzi nie ma jak się odpalić.
    let mut cfg = baza();
    cfg.eod_flat_hour = 0.0;
    cfg.flat_weekend = false;
    cfg.day_trail_stop_pct = 0.0;
    let (mut e, mut b) = stanowisko(cfg);

    for h in [10, 18, 19, 22] {
        tik(&mut e, &mut b, o_godzinie(h), CENA);
    }
    sygnal(&mut e, &mut b, o_godzinie(23), 1);
    assert!(
        !b.positions().is_empty(),
        "poprawka Z-2 zamknęła dobę presetowi, który o żadną taką regułę \
         nie prosił — to byłby rozjazd parytetu"
    );
}
