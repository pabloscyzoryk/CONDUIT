
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::parser::{self, OpcjeParsera, Signal};
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_700_000_000_000;
const SALDO: f64 = 1_000.0;

/// Sygnał kanoniczny: strefa 4000–4005 POD rynkiem, cele 4010/4020/4030 NAD.
/// Rynek startuje na 4008 — MIĘDZY strefą a pierwszym celem.
const SYGNAL: &str =
    "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nTP OPEN\nSL 3995";

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

fn odpowiedz(ts: Ts, id: i64, do_kogo: i64, tekst: &str) -> IncomingMessage {
    let mut m = wiadomosc(ts, id, tekst);
    m.reply_to = Some(do_kogo);
    m
}

/// EDYCJA wiadomości — ten sam `msg_id` co oryginał, `edit_of` wskazuje jego.
fn edycja(ts: Ts, edytowana: i64, tekst: &str) -> IncomingMessage {
    let mut m = wiadomosc(ts, edytowana, tekst);
    m.edit_of = Some(edytowana);
    m
}

fn stanowisko(cfg: Settings, bid: f64) -> (Engine, SimBroker) {
    let mut b = SimBroker::z_ustawien(SALDO, &cfg);
    b.on_quote(kwotowanie(T0, bid));
    let e = Engine::new(cfg, SALDO);
    (e, b)
}

fn tik(e: &mut Engine, b: &mut SimBroker, ts: Ts, bid: f64) {
    let q = kwotowanie(ts, bid);
    b.on_quote(q);
    e.on_tick(b, &q);
}

/// Ustawienia bazowe — siatka trzech limitów po 0,01 lota, bez dławików.
fn cfg_bazowa() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3;
    c.lot_mode_percent = false;
    c.lot_fixed = 0.01;
    c.lot_min = 0.01;
    c.risk_per_basket_pct = 0.0;
    c.max_portfolio_risk_pct = 0.0;
    c.tp_source = TpSource::Either;
    c.assign_tp_per_position = true;
    c.ignore_old_after_min = 0.0;
    c.pending_ttl_h = 0.0;
    c.pending_drop_grace_min = 0.0;
    // Siatka ma PRZEŻYĆ dojście ceny do celu — inaczej połowa scenariuszy
    // testowałaby kasowanie siatki zamiast reakcji na komunikat.
    c.pending_lifetime = PendingLifetime::Never;
    c.pending_drop_on_target = false;
    c
}

/// Koszyk z SYGNAŁU, z cofnięciem ceny do strefy — po tym są POZYCJE.
///
/// Zwraca `(engine, broker)`; koszyk ma identyfikator 1, a wiadomość
/// źródłowa numer 1.
fn koszyk_z_pozycjami(zmien: impl FnOnce(&mut Settings)) -> (Engine, SimBroker) {
    let mut cfg = cfg_bazowa();
    zmien(&mut cfg);
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(
        b.pendings().len(),
        3,
        "siatka trzech limitów ma się rozstawić"
    );
    // cofka DO strefy — wypełnia wszystkie trzy szczeble
    tik(&mut e, &mut b, T0 + 1_000, 3_999.0);
    assert!(
        !b.positions().is_empty(),
        "przesłanka: cena weszła w strefę, więc muszą być pozycje"
    );
    // powrót nad strefę, ale POD pierwszy cel — nic się samo nie zamknie
    tik(&mut e, &mut b, T0 + 2_000, 4_008.0);
    (e, b)
}

fn koszyk(e: &Engine) -> &Basket {
    e.baskets.first().expect("koszyk musi istnieć")
}

/// Ile lotów żyje w koszyku (pozycje, nie zlecenia).
fn wolumen(b: &SimBroker) -> f64 {
    (b.positions().iter().map(|p| p.volume).sum::<f64>() * 100.0).round() / 100.0
}

// ================================================================
//  B01–B02 — WEJŚCIA
// ================================================================

/// B01: WEJŚCIE STREFOWE (LIMIT) rozstawia siatkę w strefie, z celem i stopem
/// z treści — i NIE otwiera nic po rynku.
#[test]
fn b01_wejscie_limitowe_rozstawia_siatke_w_strefie() {
    let mut cfg = cfg_bazowa();
    cfg.entry_units = 3;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));

    assert_eq!(b.positions().len(), 0, "limit nie otwiera pozycji po rynku");
    assert_eq!(b.pendings().len(), 3, "trzy szczeble siatki");
    let bk = koszyk(&e);
    assert_eq!(bk.side, Side::Buy);
    assert!(bk.is_limit, "[BUY LIMITS] to zlecenie oczekujące");
    assert_eq!(bk.sl, Some(3_995.0), "stop z treści");
    assert_eq!(
        bk.tps,
        vec![4_010.0, 4_020.0, 4_030.0],
        "trzy cele z treści"
    );
    assert!(
        bk.tp_open,
        "[TP OPEN] — czwarty cel bez poziomu, runner zostaje"
    );
    for o in b.pendings() {
        assert!(
            o.price >= 3_999.99 && o.price <= 4_005.01,
            "szczebel {} poza strefą 4000–4005",
            o.price
        );
    }
}

/// B02: WEJŚCIE RYNKOWE („BUY NOW") — działa TYLKO za przełącznikiem
/// `honor_market_open`, bo komunikat nie niesie ani stopu, ani celów.
///
/// Domyślnie WYŁĄCZONE i to jest poprawne: samo „BUY NOW" bez SL to pozycja
/// bez zabezpieczenia. Test pilnuje OBU stron przełącznika.
#[test]
fn b02_wejscie_rynkowe_buy_now_za_przelacznikiem() {
    for wolno in [false, true] {
        let mut cfg = cfg_bazowa();
        cfg.honor_market_open = wolno;
        let (mut e, mut b) = stanowisko(cfg, 4008.0);
        e.on_message(&mut b, &wiadomosc(T0, 1, "BUY NOW"));
        if wolno {
            assert_eq!(
                b.positions().len(),
                1,
                "[BUY NOW] ma otworzyć pozycję po rynku"
            );
            assert_eq!(
                e.baskets.len(),
                1,
                "pozycja ma należeć do koszyka, nie wisieć luzem"
            );
        } else {
            assert_eq!(
                b.positions().len(),
                0,
                "bez przełącznika [BUY NOW] nie wchodzi"
            );
            assert!(e.baskets.is_empty(), "…i nie zakłada koszyka");
        }
    }
}

// ================================================================
//  B03–B05 — CELE
// ================================================================

/// B03: „TP1 HIT" z kanału podnosi etap koszyka i PRZESTAWIA CEL pozycji
/// na następny szczebel drabinki.
///
/// # Skąd bierze się zysk na celu — dwie różne drogi
///
/// To jest miejsce, w którym łatwo postawić złą tezę. Przy
/// `assign_tp_per_position = true` (tak stoi ta baza i tak stoją presety
/// produkcyjne) transza NIE pada z komunikatu: każda pozycja ma WŁASNY
/// take-profit u brokera i to broker ją zamyka, gdy cena tam dojdzie
/// (`bank_on_tp` zwraca wtedy udział 0 %). Komunikat kanału robi coś innego
/// i równie ważnego — PRZESUWA CEL POZOSTAŁYCH pozycji na kolejny szczebel.
///
/// Drugą drogę — inkaso z komunikatu — włącza się jawnie
/// (`bank_all_at_stage`); jej dowód stoi w drugiej połowie testu.
#[test]
fn b03_tp1_hit_przesuwa_cel_i_moze_bankowac() {
    // --- (a) domyślna droga: cele u brokera, komunikat RETARGETUJE ---
    let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
    let przed = b.positions().len();
    assert!(przed >= 2, "do testu potrzeba kilku pozycji, jest {przed}");
    // przy `tp_schedule = Ladder` szczeble dostają RÓŻNE cele z drabinki —
    // to jest stan wyjściowy, nie teza testu
    let cele_przed: Vec<Option<Px>> = b.positions().iter().map(|p| p.tp).collect();
    assert!(
        cele_przed.iter().all(|t| t.is_some()),
        "przesłanka: każda pozycja ma cel u brokera, cele: {cele_przed:?}"
    );

    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "TP1 HIT"));

    assert_eq!(koszyk(&e).tp_stage, 1, "etap koszyka ma wzrosnąć do 1");
    assert_eq!(
        b.positions().len(),
        przed,
        "przy celach u brokera komunikat NIE zamyka pozycji sam z siebie"
    );
    for p in b.positions() {
        assert_eq!(
            p.tp,
            Some(4_020.0),
            "po TP1 pozostałe pozycje mają celować w TP2 — inaczej drabinka \
             stoi w miejscu i cały koszyk czeka na cel, który już padł"
        );
    }

    // --- (b) droga jawna: `bank_all_at_stage = 1` inkasuje CAŁOŚĆ na TP1 ---
    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.bank_all_at_stage = 1);
    let przed = b.positions().len();
    assert!(przed >= 2);
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "TP1 HIT"));
    assert_eq!(b.positions().len(), 0, "bank całości na TP1 zamyka pozycje");
    assert_eq!(
        b.history.len(),
        przed,
        "…i wszystkie mają trafić do historii"
    );
    assert_eq!(koszyk(&e).state, BasketState::Done, "…a koszyk się kończy");
}

/// B04: kolejne cele przesuwają etap DALEJ, a nie od nowa.
#[test]
fn b04_tp2_i_tp3_przesuwaja_etap_dalej() {
    let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "TP1 HIT"));
    assert_eq!(koszyk(&e).tp_stage, 1);
    e.on_message(&mut b, &odpowiedz(T0 + 4_000, 3, 1, "TP2 HIT"));
    assert_eq!(koszyk(&e).tp_stage, 2, "drugi cel = etap 2");
    e.on_message(&mut b, &odpowiedz(T0 + 5_000, 4, 1, "TP3 HIT"));
    assert_eq!(koszyk(&e).tp_stage, 3, "trzeci cel = etap 3");
}

#[test]
fn f3_pips_hit_wymaga_ceny_tylko_za_przelacznikiem() {
    for (ochrona, oczekiwany_etap) in [(false, 1usize), (true, 0usize)] {
        let (mut e, mut b) = koszyk_z_pozycjami(|c| {
            c.tp_unindexed_pips_require_price = ochrona;
        });
        // Bieżący bid 4008 jest poniżej TP1=4010.
        e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "+50 PIPS HIT"));
        assert_eq!(
            koszyk(&e).tp_stage,
            oczekiwany_etap,
            "ochrona={ochrona}: meldunek bez potwierdzonej ceny"
        );
    }

    let (mut e, mut b) = koszyk_z_pozycjami(|c| {
        c.tp_unindexed_pips_require_price = true;
    });
    // Cena jest już przy/za TP1 (tolerancja pozostaje ustawieniem presetu).
    b.on_quote(kwotowanie(T0 + 3_000, 4_010.0));
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "+50 PIPS HIT"));
    assert_eq!(
        koszyk(&e).tp_stage,
        1,
        "potwierdzony ceną meldunek ma działać"
    );
}

/// B05: „TP OPEN" / „TP4 Open" — czwarty cel BEZ POZIOMU. Koszyk ma to
/// zapamiętać jako `tp_open`, żeby po ostatnim numerowanym celu został
/// runner zamiast domknięcia całości.
#[test]
fn b05_tp_open_zapamietane_jako_runner() {
    let (e, _b) = koszyk_z_pozycjami(|_| {});
    assert!(
        koszyk(&e).tp_open,
        "[TP OPEN] z treści ma trafić do koszyka"
    );

    // ten sam sygnał BEZ „TP OPEN" — flaga ma być zgaszona
    let mut cfg = cfg_bazowa();
    cfg.entry_units = 1;
    let (mut e2, mut b2) = stanowisko(cfg, 4008.0);
    e2.on_message(
        &mut b2,
        &wiadomosc(
            T0,
            1,
            "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nSL 3995",
        ),
    );
    assert!(!koszyk(&e2).tp_open, "bez [TP OPEN] runner się nie zbroi");
}

// ================================================================
//  B06 — STOP
// ================================================================

/// B06: „SL HIT" z kanału — zachowanie zależy od `sl_hit_mode` i test
/// pilnuje KAŻDEGO trybu, bo to jest komunikat, po którym nie ma odwrotu.
///
/// Domyślny `CancelPendings` NIE zamyka pozycji: kanał melduje SWÓJ stop,
/// a nasze wejścia były głębsze. `CloseAll` zamyka wszystko.
#[test]
fn b06_sl_hit_wg_trybu() {
    // --- CancelPendings (domyślny): siatka znika, pozycje zostają ---
    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.sl_hit_mode = SlHitMode::CancelPendings);
    let poz_przed = b.positions().len();
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "SL HIT"));
    assert_eq!(
        b.pendings().len(),
        0,
        "CancelPendings kasuje niewypełnione limity"
    );
    assert_eq!(
        b.positions().len(),
        poz_przed,
        "…i NIE rusza otwartych pozycji"
    );

    // --- CloseAll: koszyk kończy się na miejscu ---
    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.sl_hit_mode = SlHitMode::CloseAll);
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "SL HIT"));
    assert_eq!(b.positions().len(), 0, "CloseAll zamyka pozycje");
    assert_eq!(b.pendings().len(), 0, "…i kasuje siatkę");
    assert_eq!(koszyk(&e).state, BasketState::Done, "…i zamyka koszyk");

    // --- Ignore: nic ---
    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.sl_hit_mode = SlHitMode::Ignore);
    let poz_przed = b.positions().len();
    let pend_przed = b.pendings().len();
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "SL HIT"));
    assert_eq!(b.positions().len(), poz_przed, "Ignore nie rusza pozycji");
    assert_eq!(b.pendings().len(), pend_przed, "Ignore nie rusza siatki");
    assert_ne!(
        koszyk(&e).state,
        BasketState::Done,
        "Ignore nie zamyka koszyka"
    );
}

// ================================================================
//  B07–B08 — ZABEZPIECZENIE
// ================================================================

/// B07: „SL IS SET TO BE" / „set BE" przesuwa stop KAŻDEJ pozycji na jej
/// własną cenę wejścia. Nie na cenę rynkową i nie na średnią.
#[test]
fn b07_break_even_przesuwa_stop_na_cene_wejscia() {
    for tekst in ["SL IS SET TO BE", "Set BE", "BREAK EVEN"] {
        let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
        // przesłanka: przed komunikatem stop stoi na poziomie sygnału
        for p in b.positions() {
            assert_eq!(
                p.sl,
                Some(3_995.0),
                "przed BE stop ma być na 3995 ({tekst})"
            );
        }
        e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, tekst));
        for p in b.positions() {
            assert_eq!(
                p.sl,
                Some(p.open_price),
                "[{tekst}]: stop pozycji ma stanąć na JEJ cenie wejścia"
            );
        }
    }
}

/// B08: „RISK FREE" — koszyk zostaje oznaczony jako zabezpieczony i część
/// pozycji jest zabankowana (domyślny tryb `CloseAllKeepNearest`).
#[test]
fn b08_risk_free_zabezpiecza_koszyk() {
    let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
    let przed = b.positions().len();
    assert!(przed >= 2);
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "RISK FREE 4008"));
    assert!(
        koszyk(&e).secured,
        "koszyk ma być oznaczony jako zabezpieczony"
    );
    assert!(
        b.positions().len() < przed,
        "CloseAllKeepNearest ma zostawić JEDNĄ pozycję, zostało {}",
        b.positions().len()
    );
    for p in b.positions() {
        assert!(p.sl.is_some(), "runner po RISK FREE musi mieć stop");
    }
}

#[test]
fn f4_risk_free_odrzuca_absurdalny_poziom_za_progiem() {
    for (prog, ma_zabezpieczyc) in [(0.0, true), (20.0, false)] {
        let (mut e, mut b) = koszyk_z_pozycjami(|c| c.rf_level_sanity_max_usd = prog);
        e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "RISK FREE 4967"));
        assert_eq!(
            koszyk(&e).secured,
            ma_zabezpieczyc,
            "próg={prog}: stara semantyka przy 0, sanity-check przy >0"
        );
    }

    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.rf_level_sanity_max_usd = 20.0);
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "RISK FREE 4008"));
    assert!(
        koszyk(&e).secured,
        "prawidłowy poziom przy rynku musi przejść"
    );
}

// ================================================================
//  B09–B11 — WYJŚCIA
// ================================================================

/// B09: „OUT AT ENTRY ON THE REST" — domyślnie zamyka WSZYSTKO i kończy
/// koszyk; tryb `Ignore` nie robi nic.
#[test]
fn b09_out_at_entry_wg_trybu() {
    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.out_at_entry_mode = OutAtEntryMode::CloseAll);
    e.on_message(
        &mut b,
        &odpowiedz(T0 + 3_000, 2, 1, "OUT AT ENTRY ON THE REST"),
    );
    assert_eq!(b.positions().len(), 0, "CloseAll zamyka pozycje");
    assert_eq!(b.pendings().len(), 0, "…i kasuje siatkę");
    assert_eq!(koszyk(&e).state, BasketState::Done);

    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.out_at_entry_mode = OutAtEntryMode::Ignore);
    let przed = b.positions().len();
    e.on_message(
        &mut b,
        &odpowiedz(T0 + 3_000, 2, 1, "OUT AT ENTRY ON THE REST"),
    );
    assert_eq!(b.positions().len(), przed, "Ignore nie rusza pozycji");
}

/// B10: „CLOSE ALL" — i jego ZASIĘG. `Global` zamyka wszystkie koszyki
/// wszystkich źródeł, `Basket` tylko ten, do którego komunikat jest
/// adresowany. To jedyny komunikat w dispatchu z zasięgiem globalnym,
/// więc test trzyma OBIE ścieżki.
#[test]
fn b10_close_all_zasieg() {
    for zasieg in [CloseAllScope::Global, CloseAllScope::Basket] {
        let mut cfg = cfg_bazowa();
        cfg.close_all_scope = zasieg;
        cfg.max_open_baskets = 0;
        let (mut e, mut b) = stanowisko(cfg, 4008.0);
        // dwa koszyki z tego samego źródła
        e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
        e.on_message(
            &mut b,
            &wiadomosc(
                T0 + 10,
                2,
                "BUY LIMITS GOLD @ 4004/3999 AREA\nTP 4011\nTP 4021\nSL 3994",
            ),
        );
        assert_eq!(e.baskets.len(), 2, "przesłanka: dwa koszyki");
        tik(&mut e, &mut b, T0 + 1_000, 3_998.0);
        tik(&mut e, &mut b, T0 + 2_000, 4_008.0);
        assert!(
            !b.positions().is_empty(),
            "przesłanka: obydwa koszyki mają pozycje"
        );

        e.on_message(&mut b, &odpowiedz(T0 + 3_000, 3, 1, "CLOSE ALL"));
        match zasieg {
            CloseAllScope::Global => {
                assert_eq!(b.positions().len(), 0, "Global zamyka WSZYSTKO");
                assert_eq!(b.pendings().len(), 0);
            }
            CloseAllScope::Basket => {
                assert!(
                    !b.positions().is_empty(),
                    "Basket ma zamknąć TYLKO koszyk 1 — drugi zostaje"
                );
                assert_eq!(
                    e.baskets[0].state,
                    BasketState::Done,
                    "…ale adresat ma być zamknięty"
                );
            }
        }
    }
}

/// B11: „CANCEL THE LIMITS" kasuje niewypełnione zlecenia i NIE zamyka
/// otwartych pozycji; `honor_cancel = false` wyłącza to w całości.
#[test]
fn b11_cancel_kasuje_tylko_limity() {
    let mut cfg = cfg_bazowa();
    cfg.entry_units = 3;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    // wypełniamy TYLKO najpłytszy szczebel
    tik(&mut e, &mut b, T0 + 1_000, 4_004.0);
    let poz = b.positions().len();
    assert!(poz >= 1, "przesłanka: jeden szczebel wszedł");
    assert!(!b.pendings().is_empty(), "przesłanka: reszta siatki wisi");

    e.on_message(&mut b, &odpowiedz(T0 + 2_000, 2, 1, "CANCEL THE LIMITS"));
    assert_eq!(b.pendings().len(), 0, "CANCEL kasuje wiszące zlecenia");
    assert_eq!(b.positions().len(), poz, "…i NIE rusza otwartych pozycji");

    // wyłączone ustawieniem
    let mut cfg = cfg_bazowa();
    cfg.honor_cancel = false;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let pend = b.pendings().len();
    e.on_message(&mut b, &odpowiedz(T0 + 2_000, 2, 1, "CANCEL THE LIMITS"));
    assert_eq!(
        b.pendings().len(),
        pend,
        "honor_cancel = false — siatka zostaje"
    );
}

// ================================================================
//  B12–B14 — ZMIANA PLANU
// ================================================================

/// B12: „USE 4015 AS TP1" / „TP2 ADJUSTED TO 4025" — korekta celu wchodzi
/// do drabinki koszyka.
#[test]
fn b12_korekta_celu_wchodzi_do_drabinki() {
    let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
    assert_eq!(koszyk(&e).tps, vec![4_010.0, 4_020.0, 4_030.0]);
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "USE 4015 AS TP1"));
    assert_eq!(koszyk(&e).tps[0], 4_015.0, "TP1 ma się zmienić na 4015");
    e.on_message(&mut b, &odpowiedz(T0 + 4_000, 3, 1, "TP2 ADJUSTED TO 4025"));
    assert_eq!(koszyk(&e).tps[1], 4_025.0, "TP2 ma się zmienić na 4025");
}

/// B13: „SECURING PARTIAL PROFITS" z nową drabinką celów — koszyk dostaje
/// NOWY PLAN, a postęp jest zerowany (stary etap wskazywał w inną tablicę).
#[test]
fn b13_spp_przezbraja_cele() {
    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.spp_keep_tp = false);
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "TP1 HIT"));
    assert_eq!(koszyk(&e).tp_stage, 1, "przesłanka: etap podniesiony");

    e.on_message(
        &mut b,
        &odpowiedz(
            T0 + 4_000,
            3,
            1,
            "SECURING PARTIAL PROFITS\nTARGETS: 4040 4050 4060\nSL IS SET TO BE AT 4002",
        ),
    );
    let bk = koszyk(&e);
    assert_eq!(
        bk.tps,
        vec![4_040.0, 4_050.0, 4_060.0],
        "nowa drabinka z treści SPP"
    );
    assert!(
        bk.secured,
        "SPP przy żywych pozycjach oznacza koszyk jako zabezpieczony"
    );
}

/// B14: „Take partials" / „CLOSE 3 LAYERS" — oś `partials_wykonuj`.
///
/// Przy WYŁĄCZONEJ osi parser nie produkuje wariantu w ogóle (kontrakt zera),
/// przy włączonej i niezerowej transzy koszyk oddaje część wolumenu.
#[test]
fn b14_partials_za_osia() {
    // oś WYŁĄCZONA — wariant nie powstaje nawet w parserze
    let sygnaly = parser::parse_z_opcjami("CLOSE 3 LAYERS NOW", OpcjeParsera::default());
    assert!(
        !sygnaly.iter().any(|s| matches!(s, Signal::TakePartials)),
        "kontrakt zera: bez osi nie ma wariantu TakePartials"
    );

    // oś WŁĄCZONA, transza 50 % — wolumen ma spaść
    let (mut e, mut b) = koszyk_z_pozycjami(|c| {
        c.partials_wykonuj = true;
        c.partials_pct = 50.0;
        c.entry_units = 3;
    });
    let vol_przed = wolumen(&b);
    assert!(
        vol_przed >= 0.03,
        "przesłanka: trzy szczeble po 0,01 = {vol_przed}"
    );
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "CLOSE 3 LAYERS NOW"));
    assert!(
        wolumen(&b) < vol_przed,
        "polecenie warstwowe ma odebrać część wolumenu: było {vol_przed}, jest {}",
        wolumen(&b)
    );
}

/// B15: WARIANT WARUNKOWY („YOU CAN CLOSE 3 LAYERS … OR YOU CAN HOLD") to
/// PROPOZYCJA, nie polecenie — nie wolno go wykonać nawet przy włączonej osi.
#[test]
fn b15_warunkowe_close_layers_nie_jest_poleceniem() {
    const TEKST: &str = "YOU CAN CLOSE 3 LAYERS NOW TO COVER THE SL OR YOU CAN HOLD FOR 200 PIPS";
    let l = parser::close_layers(TEKST).expect("to JEST polecenie warstwowe co do kształtu");
    assert!(l.optional, "…ale WARUNKOWE");

    let opcje = OpcjeParsera {
        partials_jako_komenda: true,
        ..OpcjeParsera::default()
    };
    let sygnaly = parser::parse_z_opcjami(TEKST, opcje);
    assert!(
        !sygnaly.iter().any(|s| matches!(s, Signal::TakePartials)),
        "wariant warunkowy nie ma prawa stać się poleceniem"
    );

    let (mut e, mut b) = koszyk_z_pozycjami(|c| {
        c.partials_wykonuj = true;
        c.partials_pct = 50.0;
    });
    let vol = wolumen(&b);
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, TEKST));
    assert_eq!(wolumen(&b), vol, "propozycja nie rusza wolumenu");
}

// ================================================================
//  B16–B19 — ADRESOWANIE, POWTÓRZENIA, RETROSPEKCJE
// ================================================================

/// B16: RELACJA, NIE POLECENIE. „TP2 hit for anyone that held this" opowiada
/// o cudzej pozycji — wykonanie tego inkasuje transzę bez powodu.
#[test]
fn b16_retrospekcja_nie_rusza_koszyka() {
    let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
    let etap = koszyk(&e).tp_stage;
    let poz = b.positions().len();
    e.on_message(
        &mut b,
        &odpowiedz(T0 + 3_000, 2, 1, "TP2 Hit for anyone that held this trade"),
    );
    assert_eq!(koszyk(&e).tp_stage, etap, "retrospekcja nie podnosi etapu");
    assert_eq!(b.positions().len(), poz, "…i nie zamyka pozycji");
}

/// B17: KOMUNIKAT DO KOSZYKA JUŻ ZAMKNIĘTEGO — ma nie zrobić nic i nie
/// wskrzesić koszyka. Adresat jest jednoznaczny (`reply_to`), więc trafia,
/// ale nie ma czego ruszyć.
#[test]
fn b17_komunikat_do_zamknietego_koszyka() {
    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.close_all_scope = CloseAllScope::Basket);
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "CLOSE ALL"));
    assert_eq!(
        koszyk(&e).state,
        BasketState::Done,
        "przesłanka: koszyk zamknięty"
    );
    let zamkniec = b.history.len();

    for tekst in [
        "TP1 HIT",
        "RISK FREE 4008",
        "SL IS SET TO BE",
        "OUT AT ENTRY",
        "CANCEL THE LIMITS",
    ] {
        e.on_message(&mut b, &odpowiedz(T0 + 4_000, 3, 1, tekst));
        assert_eq!(
            b.positions().len(),
            0,
            "[{tekst}] nie ma prawa otworzyć pozycji"
        );
        assert_eq!(
            b.pendings().len(),
            0,
            "[{tekst}] nie ma prawa wystawić zlecenia"
        );
        assert_eq!(
            b.history.len(),
            zamkniec,
            "[{tekst}] nie ma prawa nic domknąć"
        );
        assert_eq!(
            koszyk(&e).state,
            BasketState::Done,
            "[{tekst}] nie wskrzesza koszyka"
        );
    }
}

/// B18: KOMUNIKAT BEZ `reply_to` — kanał często odpowiada „luzem". Adresata
/// wskazuje wtedy WSKAZÓWKA CENOWA z treści, a gdy jej nie ma, reguła
/// „najnowszy żywy koszyk z tego źródła".
#[test]
fn b18_komunikat_bez_reply_to_trafia_po_wskazowce() {
    // (a) wskazówka cenowa wskazuje STARSZY koszyk, choć reguła zapasowa
    //     („najnowszy żywy") wskazałaby nowszy — to jest cała treść testu.
    //
    //     OBIE strefy muszą leżeć POD rynkiem, inaczej „BUY LIMITS" nad ceną
    //     jest geometrycznie niemożliwy i drugi koszyk w ogóle nie powstaje.
    //     Stopy schodzą nisko, żeby wspólna cofka nie zabiła pierwszego.
    const SYG_A: &str = "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3970";
    const SYG_B: &str = "BUY LIMITS GOLD @ 3995/3990 AREA\nTP 4011\nTP 4021\nTP 4031\nSL 3960";
    let mut cfg = cfg_bazowa();
    cfg.max_open_baskets = 0;
    cfg.basket_hint_tolerance = 0.6;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYG_A));
    e.on_message(&mut b, &wiadomosc(T0 + 10, 2, SYG_B));
    assert_eq!(e.baskets.len(), 2, "przesłanka: dwa koszyki");
    tik(&mut e, &mut b, T0 + 1_000, 3_989.0);
    tik(&mut e, &mut b, T0 + 2_000, 4_008.0);
    assert!(
        e.baskets.iter().all(|bk| bk.had_positions),
        "przesłanka: obydwa koszyki się wypełniły"
    );

    // „(4002 TO 4004)" wskazuje strefę PIERWSZEGO koszyka — mimo że reguła
    // zapasowa wybrałaby drugi (nowszy, też z pozycjami)
    e.on_message(&mut b, &wiadomosc(T0 + 3_000, 3, "TP1 HIT (4002 TO 4004)"));
    assert_eq!(e.baskets[0].tp_stage, 1, "wskazówka ma trafić w koszyk 1");
    assert_eq!(e.baskets[1].tp_stage, 0, "…i nie ruszyć koszyka 2");

    // (b) bez wskazówki — reguła „najnowszy żywy z pozycjami"
    let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
    e.on_message(&mut b, &wiadomosc(T0 + 3_000, 9, "TP1 HIT"));
    assert_eq!(
        koszyk(&e).tp_stage,
        1,
        "bez adresata trafia w jedyny żywy koszyk"
    );
}

/// B19: EDYCJA POWTARZAJĄCA AKCJĘ — dostawca może ponownie dostarczyć tę samą
/// końcową treść wiadomości. Bez deduplikacji koszyk inkasowałby transzę na
/// tym samym celu dwa razy.
#[test]
fn b19_edycja_nie_wykonuje_akcji_drugi_raz() {
    for dedup in [true, false] {
        let (mut e, mut b) = koszyk_z_pozycjami(|c| c.dedup_edited_signals = dedup);
        e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "TP1 HIT"));
        assert_eq!(koszyk(&e).tp_stage, 1, "przesłanka: pierwszy raz wykonany");

        let mut m = edycja(T0 + 4_000, 2, "TP1 HIT");
        m.reply_to = Some(1);
        e.on_message(&mut b, &m);

        if dedup {
            assert_eq!(
                koszyk(&e).tp_stage,
                1,
                "edycja powtarzająca [TP1 HIT] ma zostać pominięta"
            );
        } else {
            assert!(
                koszyk(&e).tp_stage >= 1,
                "bez dedupu akcja idzie ponownie — to jest właśnie to, co dedup blokuje"
            );
        }
    }
}

/// B20: EDYCJA ZMIENIAJĄCA TREŚĆ SYGNAŁU po rozstawieniu siatki —
/// przezbrojenie koszyka, a nie drugi koszyk.
#[test]
fn b20_edycja_wejscia_przezbraja_koszyk() {
    let mut cfg = cfg_bazowa();
    let (mut e, mut b) = stanowisko(cfg.clone(), 4008.0);
    cfg.entry_units = 3;
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(e.baskets.len(), 1);
    assert_eq!(koszyk(&e).tps, vec![4_010.0, 4_020.0, 4_030.0]);

    e.on_message(
        &mut b,
        &edycja(
            T0 + 1_000,
            1,
            "BUY LIMITS GOLD @ 4003/3998 AREA\nTP 4012\nTP 4022\nTP 4032\nSL 3993",
        ),
    );
    assert_eq!(e.baskets.len(), 1, "edycja NIE zakłada drugiego koszyka");
    let bk = koszyk(&e);
    assert_eq!(
        bk.tps,
        vec![4_012.0, 4_022.0, 4_032.0],
        "nowe cele z edycji"
    );
    assert_eq!(bk.sl, Some(3_993.0), "nowy stop z edycji");
    assert_eq!(bk.tp_stage, 0, "nowy plan zeruje postęp");
}

// ================================================================
//  B21–B23 — KOLEJNOŚĆ I CISZA
// ================================================================

/// B21: PROZA — „good morning traders" ma nie robić NIC. To jest połowa
/// ruchu w kanale ZEN i jedna czwarta w Synergy.
#[test]
fn b21_proza_nie_wywoluje_akcji() {
    let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
    let etap = koszyk(&e).tp_stage;
    let poz = b.positions().len();
    let pend = b.pendings().len();
    for tekst in [
        "UNSTRUCTURED INFORMATION A",
        "UNSTRUCTURED INFORMATION B",
        "UNSTRUCTURED INFORMATION C",
    ] {
        e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, tekst));
        assert!(
            parser::parse(tekst)
                .iter()
                .all(|s| matches!(s, Signal::Info)),
            "[{tekst}] ma być czystą informacją"
        );
    }
    assert_eq!(koszyk(&e).tp_stage, etap);
    assert_eq!(b.positions().len(), poz);
    assert_eq!(b.pendings().len(), pend);
}

/// B22: KOLEJNOŚĆ „BE przed TP1" — komunikat celu NIE MA PRAWA cofnąć stopu
/// z breakeven z powrotem pod wejście. To jest ta pomyłka, po której koszyk
/// zabezpieczony przez kanał wraca do pełnego ryzyka.
#[test]
fn b22_tp1_po_be_nie_cofa_stopu() {
    let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "SL IS SET TO BE"));
    let po_be: Vec<(Ticket, Option<Px>, Px)> = b
        .positions()
        .iter()
        .map(|p| (p.ticket, p.sl, p.open_price))
        .collect();
    assert!(!po_be.is_empty(), "przesłanka: są pozycje po BE");
    for (_, sl, open) in &po_be {
        assert_eq!(*sl, Some(*open), "przesłanka: stop na wejściu");
    }

    e.on_message(&mut b, &odpowiedz(T0 + 4_000, 3, 1, "TP1 HIT"));
    for p in b.positions() {
        let open = p.open_price;
        assert!(
            p.sl.map(|s| s >= open - 1e-9).unwrap_or(false),
            "po [TP1 HIT] stop {:?} zszedł PONIŻEJ wejścia {open} — komunikat celu \
             cofnął zabezpieczenie",
            p.sl
        );
    }
}

/// B23: „RISK FREE" PO FAKCIE — komunikat przychodzi, gdy koszyk już zebrał
/// cel i ma stop nad wejściem. Nie wolno mu POGORSZYĆ stopu.
#[test]
fn b23_risk_free_po_fakcie_nie_pogarsza_stopu() {
    let (mut e, mut b) = koszyk_z_pozycjami(|_| {});
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "TP1 HIT"));
    tik(&mut e, &mut b, T0 + 3_500, 4_012.0);
    let przed: Vec<(Px, Option<Px>)> = b.positions().iter().map(|p| (p.open_price, p.sl)).collect();

    e.on_message(&mut b, &odpowiedz(T0 + 4_000, 3, 1, "RISK FREE 4012"));
    assert!(koszyk(&e).secured, "RISK FREE ma oznaczyć koszyk");
    for p in b.positions() {
        if let Some((_, Some(stary))) = przed.iter().find(|(o, _)| (*o - p.open_price).abs() < 1e-9)
        {
            assert!(
                p.sl.map(|s| s >= *stary - 1e-9).unwrap_or(false),
                "RISK FREE pogorszył stop: było {stary}, jest {:?}",
                p.sl
            );
        }
    }
}

// ================================================================
//  B24 — CZUJNIK „SYGNAŁ, KTÓREGO NIE UMIEM PRZECZYTAĆ"
// ================================================================

/// B24: syntetyczna zapowiedź strefy kontra właściwy sygnał.
///
/// Test pilnuje OBU stron, bo pomyłka w KAŻDĄ stronę jest droga: wejście
/// w zapowiedź to pozycja dużo wcześniej, niż kanał zamierzał, a odrzucenie
/// właściwego sygnału to pominięcie prawidłowego wejścia.
#[test]
fn b24_zapowiedz_strefy_to_nie_sygnal() {
    const ZAPOWIEDZ: &str = "Potential setup: \u{1F447}\nXAUUSD Sell 2110-2115\n\
                             TP1 2107\nTP2 2103\nTP3 2100\nTP Open\nSL 2120";
    const SYGNAL_ZEN: &str = "XAUUSD Sell 2110-2115\nTP1 2107\nTP2 2103\nTP3 2100\n\
                              TP Open\nSL 2120";

    assert!(
        parser::wyglada_na_wejscie(ZAPOWIEDZ),
        "czujnik ma widzieć KSZTAŁT sygnału także w zapowiedzi"
    );
    assert!(
        !parser::parse(ZAPOWIEDZ)
            .iter()
            .any(|s| matches!(s, Signal::Entry(_))),
        "…ale zapowiedź NIE jest wejściem"
    );
    assert!(
        parser::parse(SYGNAL_ZEN)
            .iter()
            .any(|s| matches!(s, Signal::Entry(_))),
        "ten sam blok BEZ wstępu JEST wejściem — inaczej kanał ZEN przestaje handlować"
    );
}

#[test]
fn b25_komunikat_bez_koszyka_zostawia_slad() {
    let komunikaty = [
        "TP1 HIT",
        "TP2 HIT",
        "SL HIT",
        "RISK FREE 4008",
        "OUT AT ENTRY ON THE REST",
        "SECURING PARTIAL PROFITS",
        "CANCEL THE LIMITS",
        "SL IS SET TO BE",
        "USE 4015 AS TP1",
    ];
    for tekst in komunikaty {
        let cfg = cfg_bazowa();
        let (mut e, mut b) = stanowisko(cfg, 4_008.0);
        // Dziennik jest tu JEDYNYM świadkiem: bez żywego koszyka nie zmienia
        // się ani jedna liczba na rachunku, więc dowodem „nie zginęło" może
        // być wyłącznie wpis Z POWODEM.
        e.journal.cfg.enabled = true;
        e.journal.cfg.min_level = conduit_core::journal::EventLevel::Warn;
        e.on_message(&mut b, &wiadomosc(T0, 7, tekst));
        let evs = e.drain_journal();
        let slad = evs
            .iter()
            .find(|ev| ev.reason == Some(conduit_core::journal::RejectCode::NoTargetBasket));
        assert!(
            slad.is_some(),
            "[{tekst}] bez żywego koszyka ma zostawić ŚLAD Z POWODEM (NoTargetBasket), \
             a nie zniknąć w ciszy; dziennik ma {} wpisów: {:?}",
            evs.len(),
            evs.iter().map(|e| (e.kind, e.reason)).collect::<Vec<_>>()
        );
    }
}

// ================================================================
//  B26–B29 — MAPA DZIUR
//
//  Ta sekcja jest ODWROTNOSCIA poprzednich: kazdy test opisuje regule,
//  ktora dzis NIE JEST spelniona. Stoja tu z `#[ignore]`, zeby nie
//  psuly zielonego przebiegu innym zespolom, ale sa gotowe do
//  uruchomienia (`cargo test -- --ignored`) i zaswieca na zielono
//  dokladnie w dniu, w ktorym dziura zostanie zatkana.
// ================================================================

/// B26 (DZIURA): NO-OP KONFIGURACYJNY NIE ZOSTAWIA SLADU.
///
/// `risk_free_mode = Ignore`, `sl_hit_mode = Ignore`,
/// `out_at_entry_mode = Ignore` i `partials_pct = 0` dochodza do konca
/// trasy — komunikat jest rozpoznany, koszyk wskazany — i tam gina.
/// W dzienniku nie ma ani `signal_rejected`, ani `target_ignored`, wiec
/// mianownik „widziane -> wykonane" liczy je jako WYKONANE.
///
/// Cena tego bledu w liczbach: preset FS-M3-ZEN ma `risk_free_mode =
/// Ignore`, a „risk free" to NAJCZESTSZE polecenie tego kanalu — 119
/// z 465 rozpoznanych akcji (26 %). Osiemnascie z nich policzylo sie
/// jako wykonane, choc silnik nie zrobil nic.
#[test]
#[ignore = "DZIURA: no-op z konfiguracji nie pisze do dziennika"]
fn b26_no_op_konfiguracyjny_zostawia_slad() {
    let przypadki: [(&str, fn(&mut Settings)); 4] = [
        ("RISK FREE 4008", |c: &mut Settings| {
            c.risk_free_mode = RiskFreeMode::Ignore
        }),
        ("SL HIT", |c: &mut Settings| {
            c.sl_hit_mode = SlHitMode::Ignore
        }),
        ("OUT AT ENTRY ON THE REST", |c: &mut Settings| {
            c.out_at_entry_mode = OutAtEntryMode::Ignore
        }),
        ("CLOSE 3 LAYERS NOW", |c: &mut Settings| {
            c.partials_wykonuj = true;
            c.partials_pct = 0.0;
        }),
    ];
    for (tekst, zmien) in przypadki {
        let (mut e, mut b) = koszyk_z_pozycjami(zmien);
        e.journal.cfg.enabled = true;
        e.journal.cfg.min_level = conduit_core::journal::EventLevel::Warn;
        let _ = e.drain_journal();
        e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, tekst));
        let evs = e.drain_journal();
        assert!(
            !evs.is_empty(),
            "[{tekst}] wylaczony ustawieniem ma zostawic slad Z POWODEM \
             (DisabledBySetting), a nie zniknac w notatce koszyka"
        );
    }
}


/// Odczyt z wlaczona osia luzu (reszta przelacznikow domyslna).
fn parse_luz(tekst: &str) -> Vec<Signal> {
    parser::parse_z_opcjami(
        tekst,
        OpcjeParsera {
            luz_interpunkcyjny: true,
            ..Default::default()
        },
    )
}

/// B27: „Set SL to BE" JEST poleceniem break-even (za osia).
///
/// Test używa wyłącznie syntetycznych wariantów składni i pilnuje zarówno
/// trybu rozkazującego, jak i opcjonalnego poziomu liczbowego.
#[test]
fn b27_set_sl_to_be_jest_poleceniem_be() {
    for tekst in [
        "SET SL TO BE ON ALL OF YOUR ENTRIES",
        "Set SL to BE",
        "Set SL to BE to be risk free (2101)",
        "SET YOUR SL TO BE AT 2102",
    ] {
        let z = parse_luz(tekst);
        assert!(
            z.iter()
                .any(|x| matches!(x, Signal::BreakEven | Signal::SetSl { .. })),
            "[{tekst}] to polecenie przesuniecia stopu na breakeven, a parser widzi {z:?}"
        );
        // KONTRAKT ZERA: bez osi ani jeden z tych tekstow nie moze dac BE.
        let bez = parser::parse(tekst);
        assert!(
            !bez.iter().any(|x| matches!(x, Signal::BreakEven)),
            "[{tekst}] przy osi OFF parser ma milczec (parytet), a widzi {bez:?}"
        );
    }
    // OPIS, NIE POLECENIE: bez czasownika rozkazujacego wzorzec ma milczec
    // takze przy wlaczonej osi.
    let opis = parse_luz("price came back to my SL to be honest");
    assert!(
        !opis.iter().any(|x| matches!(x, Signal::BreakEven)),
        "opis stanu nie jest poleceniem, a parser widzi {opis:?}"
    );
}

#[test]
fn b28_out_kropka_at_be_to_out_at_entry() {
    for tekst in ["Out. At BE on the rest", "Out, at entry on the rest"] {
        let z = parse_luz(tekst);
        assert!(
            z.iter().any(|x| matches!(x, Signal::OutAtEntry)),
            "[{tekst}] to polecenie wyjscia po cenie wejscia, a parser widzi {z:?}"
        );
        assert!(
            !parser::parse(tekst)
                .iter()
                .any(|x| matches!(x, Signal::OutAtEntry)),
            "[{tekst}] przy osi OFF parser ma milczec (parytet)"
        );
    }
    // Zapis ze spacja dziala BEZ osi i ma dzialac dalej — to jest te 155
    // wiadomosci Synergy, ktorych oS nie moze ruszyc.
    assert!(
        parser::parse("OUT AT ENTRY ON THE REST")
            .iter()
            .any(|x| matches!(x, Signal::OutAtEntry)),
        "zapis podstawowy musi dzialac niezaleznie od osi"
    );
}

#[test]
fn b29_korekta_proza_zmienia_plan() {
    let z = parse_luz("Correction, TP3 should be 4043 not 4042");
    assert!(
        z.iter().any(|x| matches!(x, Signal::TpCorrection { index: 3, value } if (*value - 4043.0).abs() < 1e-9)),
        "korekta celu podana proza ma trafic do drabinki jako TP3=4043, a parser widzi {z:?}"
    );
    assert!(
        !parser::parse("Correction, TP3 should be 4043 not 4042")
            .iter()
            .any(|x| matches!(x, Signal::TpCorrection { .. })),
        "przy osi OFF korekta prozy ma byc niewidzialna (parytet)"
    );
    // KOREKTA LICZBY PIPSOW TO NIE KOREKTA CELU — nawet przy wlaczonej osi.
    for proza in ["Correction: +25 pips not +30 pips", "Correction: +75 Pips"] {
        let s = parse_luz(proza);
        assert!(
            !s.iter().any(|x| matches!(x, Signal::TpCorrection { .. })),
            "[{proza}] to korekta podsumowania, nie planu, a parser widzi {s:?}"
        );
    }
}

/// B31: literowka „RISK FREEE" (za osia).
///
/// Granica `\b` po „FREE" nie wypada miedzy dwoma „E", wiec `RE_RF` milczal na
/// calej wiadomosci — mimo ze niesie poziom. Synergy 3 unikalne tresci.
#[test]
fn b31_literowka_risk_freee() {
    let z = parse_luz("+50 / RISK FREEE 4045");
    assert!(
        z.iter().any(
            |x| matches!(x, Signal::RiskFree { level: Some(v) } if (*v - 4045.0).abs() < 1e-9)
        ),
        "literowka ma dac RiskFree z poziomem 4045, a parser widzi {z:?}"
    );
    assert!(
        !parser::parse("+50 / RISK FREEE 4045")
            .iter()
            .any(|x| matches!(x, Signal::RiskFree { .. })),
        "przy osi OFF literowka ma zostac proza (parytet)"
    );
    // Poprawny zapis dziala bez osi i NIE dubluje sie przy wlaczonej.
    let ile = parse_luz("RISK FREE 4034")
        .iter()
        .filter(|x| matches!(x, Signal::RiskFree { .. }))
        .count();
    assert_eq!(
        ile, 1,
        "poprawny zapis ma dac DOKLADNIE jeden RiskFree, a nie dwa"
    );
}

/// B32: „ALL TP'S HIT" (za osia) — i DLACZEGO apostrof jest wymagany.
///
/// To jest ostrożny wzorzec: podobne zdanie w podsumowaniu dnia musi pozostać
/// prozą, a nie meldunkiem o naszym koszyku. Wykonanie go kasowałoby transzę
/// po każdym zbiorczym podsumowaniu.
///
/// Druga bramka to `RE_RETROSPEKCJA`: wiadomość z apostrofem może mówić
/// o cudzej pozycji („If anyone took this, all TP's hit").
#[test]
fn b32_wszystkie_cele_z_apostrofem() {
    // (a) MELDUNEK o naszym koszyku — ma byc trafieniem.
    for tekst in [
        "ALL TP'S HIT 🔥🔥🔥",
        "Just like that, all TP's hit at once 🔥",
    ] {
        let z = parse_luz(tekst);
        assert!(
            z.iter().any(|x| matches!(x, Signal::TpHit { .. })),
            "[{tekst}] to meldunek o trafieniu, a parser widzi {z:?}"
        );
        assert!(
            !parser::parse(tekst)
                .iter()
                .any(|x| matches!(x, Signal::TpHit { .. })),
            "[{tekst}] przy osi OFF ma zostac proza (parytet)"
        );
    }
    // (b) RECAP DNIA BEZ APOSTROFU — proza W OBU polozeniach przelacznika.
    let recap = "THATS 9 TPS HIT ALREADY TODAY";
    for (etyk, s) in [("OFF", parser::parse(recap)), ("ON", parse_luz(recap))] {
        assert!(
            !s.iter().any(|x| matches!(x, Signal::TpHit { .. })),
            "[os {etyk}] recap dnia NIE jest meldunkiem o koszyku, a parser widzi {s:?}"
        );
    }
    // (c) RETROSPEKCJA z apostrofem — odcina ja istniejaca bramka.
    let retro = parse_luz("If anyone took this, all TP's hit 🔥");
    assert!(
        !retro.iter().any(|x| matches!(x, Signal::TpHit { .. })),
        "relacja o cudzej pozycji nie rusza naszego koszyka, a parser widzi {retro:?}"
    );
}

#[test]
fn b33_uniewaznienie_strefy_bez_drugiego_zdania() {
    let z = parse_luz("The last zone is no longer valid ❌️");
    assert!(
        z.iter().any(|x| matches!(x, Signal::Cancel)),
        "odwolanie strefy ma skasowac wiszace zlecenia, a parser widzi {z:?}"
    );
    assert!(
        !parser::parse("The last zone is no longer valid ❌️")
            .iter()
            .any(|x| matches!(x, Signal::Cancel)),
        "przy osi OFF ma zostac proza (parytet)"
    );
    // ESEJE — proza takze przy WLACZONEJ osi. Podmiotem jest idea/analiza,
    // nie zlecenie; wykonanie ich kasowaloby siatke po wykladzie.
    for esej in [
        "it simply marks where the idea is no longer valid",
        "When we choose to close a trade early, like we did today, it doesn't \
         necessarily mean our original analysis is no longer valid",
    ] {
        let s = parse_luz(esej);
        assert!(
            !s.iter().any(|x| matches!(x, Signal::Cancel)),
            "[{esej}] to wyklad, nie polecenie, a parser widzi {s:?}"
        );
    }
}

/// B30 (DZIURA): KOMUNIKAT DO KOSZYKA JUZ ZAMKNIETEGO tez ginie bez powodu.
///
/// `target_basket` zwraca identyfikator takze dla koszyka martwego („adresat
/// jest jednoznaczny"), wiec `target_or_note` NIE woła `jignore` — a handler
/// wychodzi po cichu, bo nie ma czym ruszac. W rejestrze taka akcja wyglada
/// jak WYKONANA.
///
/// To jest ta sama klasa co B26 i ta sama konsekwencja: mianownik
/// „widziane -> wykonane" zawyza licznik wykonan. Kanaly komentuja setup
/// jeszcze dlugo po jego zamknieciu, wiec skala nie jest marginalna.
#[test]
#[ignore = "DZIURA: komunikat do martwego koszyka nie pisze do dziennika"]
fn b30_komunikat_do_martwego_koszyka_zostawia_slad() {
    let (mut e, mut b) = koszyk_z_pozycjami(|c| c.close_all_scope = CloseAllScope::Basket);
    e.on_message(&mut b, &odpowiedz(T0 + 3_000, 2, 1, "CLOSE ALL"));
    assert_eq!(
        koszyk(&e).state,
        BasketState::Done,
        "przeslanka: koszyk zamkniety"
    );
    e.journal.cfg.enabled = true;
    e.journal.cfg.min_level = conduit_core::journal::EventLevel::Warn;
    let _ = e.drain_journal();
    for tekst in ["TP1 HIT", "RISK FREE 4008", "SL IS SET TO BE"] {
        e.on_message(&mut b, &odpowiedz(T0 + 4_000, 3, 1, tekst));
    }
    let evs = e.drain_journal();
    assert!(
        !evs.is_empty(),
        "trzy komunikaty do martwego koszyka nie zostawily ANI JEDNEGO wpisu \
         — w rejestrze policza sie jako wykonane"
    );
}
