
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_700_000_000_000;

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

fn stanowisko(cfg: Settings, bid: f64) -> (Engine, SimBroker) {
    let stops = cfg.stops_level;
    let mut b = SimBroker::new(1000.0, stops, 0.0);
    b.on_quote(kwotowanie(T0, bid));
    let e = Engine::new(cfg, 1000.0);
    (e, b)
}

fn tik(e: &mut Engine, b: &mut SimBroker, ts: Ts, bid: f64) {
    let q = kwotowanie(ts, bid);
    b.on_quote(q);
    e.on_tick(b, &q);
}

/// Strefa LIMITOWA pod rynkiem: przy cenie startowej 4008 wszystkie szczeble
/// leżą poniżej, więc kładą się jako zlecenia oczekujące i NIC nie wchodzi.
const LIMITY: &str = "BUY LIMITS GOLD @ 4005/4000\nTP 4010\nTP 4020\nTP 4030\nSL 3990";
/// Ten sam setup jako wejście RYNKOWE — potrzebny do punktu (b).
const RYNEK: &str = "BUY GOLD @ 4005/4000\nTP 4010\nTP 4020\nTP 4030\nSL 3990";

/// Wspólna baza: siatka limitów, żadnych sprzątaczy tła, cele w jednym miejscu.
fn baza() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3;
    c.tp_schedule = TpSchedule::AllAtTp1;
    c.assign_tp_per_position = true;
    // szczebel, na którym limit już się nie położy, ma być POMINIĘTY, a nie
    // zamieniony w wejście rynkowe — inaczej testy mierzyłyby dwie rzeczy naraz
    c.pending_cross_policy = PendingCrossPolicy::Skip;
    c
}

/// Baza dla punktu (b): sygnał RYNKOWY musi naprawdę wejść, więc szczebel
/// nie do położenia limitem ma iść po rynku (`Market` = zachowanie domyślne),
/// a nie zostać pominięty.
fn baza_rynkowa() -> Settings {
    let mut c = baza();
    c.pending_cross_policy = PendingCrossPolicy::Market;
    c.auto_limit = false;
    c
}

// ============================================================
//  (a) KOSZYK BEZ POZYCJI NIE AWANSUJE ETAPU
// ============================================================

/// Cena mija WSZYSTKIE trzy cele, a siatka ani razu się nie wypełnia.
///
/// Przed naprawą: `tp_stage` kończy na 3. Po naprawie: `tp_stage` stoi na 0,
/// a przebytą drogę notuje osobne pole `plan_wykonany_do`.
#[test]
fn bez_pozycji_etap_stoi_choc_cena_minela_wszystkie_cele() {
    let mut cfg = baza();
    // `Never` po to, żeby siatka dożyła do ostatniego celu i dało się zobaczyć
    // WSZYSTKIE trzy przejścia. Kasowanie siatki testuje punkt (c).
    cfg.pending_lifetime = PendingLifetime::Never;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, LIMITY));
    assert!(
        !b.pendings().is_empty(),
        "test wymaga wiszącej siatki limitów"
    );
    assert!(
        b.positions().is_empty(),
        "test wymaga, żeby NIC się nie wypełniło"
    );

    // Cena idzie W GÓRĘ, czyli od strefy, i po kolei mija TP1, TP2, TP3.
    // Jeden tick na cel — pętla wykrywania sprawdza jeden poziom na przebieg.
    for (i, bid) in [4011.0, 4021.0, 4031.0].into_iter().enumerate() {
        tik(&mut e, &mut b, T0 + (i as i64 + 1) * 1_000, bid);
    }

    assert!(b.positions().is_empty(), "nic nie miało prawa się wypełnić");
    let k = &e.baskets[0];
    assert_eq!(
        k.tp_stage, 0,
        "etap koszyka BEZ POZYCJI musi stać na zerze — cele zlecenia oczekującego są martwe"
    );
    assert_eq!(
        k.plan_wykonany_do, 3,
        "przebyta bez nas droga ma być policzona, ale w OSOBNYM polu"
    );
    assert!(
        !k.secured,
        "koszyk bez ani jednego lota nie jest „zabezpieczony”"
    );
}

/// To samo, ale ruch melduje KANAŁ, nie cena. Ścieżka `signal_tp_check` nie
/// miała ani jednego odwołania do pozycji, a przy `tp_source = Either`
/// (domyślne i we wszystkich wydanych presetach) przepuszczała każdy komunikat.
#[test]
fn bez_pozycji_komunikat_z_kanalu_tez_nie_rusza_etapu() {
    let mut cfg = baza();
    cfg.pending_lifetime = PendingLifetime::Never;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, LIMITY));
    assert!(b.positions().is_empty());

    e.on_message(&mut b, &wiadomosc(T0 + 1_000, 2, "✅ TP1 HIT +48 PIPS"));
    e.on_message(&mut b, &wiadomosc(T0 + 2_000, 3, "✅ TP2 HIT +90 PIPS"));

    assert_eq!(
        e.baskets[0].tp_stage, 0,
        "meldunek z kanału opisuje RUCH, nie naszą transakcję"
    );
}

/// SPP („SECURING PARTIAL PROFITS") oznaczał koszyk jako zabezpieczony
/// BEZWARUNKOWO. Koszyk bez wypełnienia zostawał więc „uwolniony od ryzyka",
/// mając ryzyko pełne — a skutki wychodziły dopiero po późniejszym wejściu:
/// `riskfree_pass` taki koszyk pomija, a podłoga SMART SL skacze na breakeven.
#[test]
fn bez_pozycji_spp_nie_oznacza_koszyka_jako_zabezpieczonego() {
    let mut cfg = baza();
    cfg.pending_lifetime = PendingLifetime::Never;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, LIMITY));
    assert!(b.positions().is_empty());

    e.on_message(
        &mut b,
        &wiadomosc(T0 + 1_000, 2, "SECURING PARTIAL PROFITS\nTP 4012\nTP 4022"),
    );

    let k = &e.baskets[0];
    assert!(
        !k.secured,
        "nie ma czego zabezpieczać — koszyk nie trzyma ani jednego lota"
    );
    assert_eq!(k.secured_ts, 0, "zegar runnera nie ma od czego ruszyć");
    assert_eq!(k.tp_stage, 0, "SPP nie awansuje etapu koszyka bez pozycji");
}

// ============================================================
//  (b) PO WYPEŁNIENIU ETAPY LICZĄ SIĘ NORMALNIE
// ============================================================

/// Kontrola dodatnia całej naprawy: gdy koszyk JEST w rynku, wszystko liczy
/// się jak dotąd. Bez tego testu „nic nie awansuje" dałoby się spełnić
/// wyłączając mechanizm.
#[test]
fn po_wypelnieniu_etapy_licza_sie_normalnie() {
    let mut cfg = baza_rynkowa();
    cfg.pending_lifetime = PendingLifetime::Never;
    // pozycje mają dożyć do kolejnych celów, a nie zamknąć się na TP1
    cfg.tp_schedule = TpSchedule::AllRunners;
    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, RYNEK));
    assert!(
        !b.positions().is_empty(),
        "wejście rynkowe musi otworzyć pozycję"
    );
    assert_eq!(e.baskets[0].tp_stage, 0, "przed ruchem etap jest zerowy");

    tik(&mut e, &mut b, T0 + 1_000, 4011.0);
    assert_eq!(e.baskets[0].tp_stage, 1, "TP1 z pozycją w rynku LICZY SIĘ");

    tik(&mut e, &mut b, T0 + 2_000, 4021.0);
    assert_eq!(e.baskets[0].tp_stage, 2, "TP2 z pozycją w rynku LICZY SIĘ");
}

/// Najtwardszy dowód wypełnienia: broker sam zamknął pozycję na JEJ
/// take-proficie. Etap musi urosnąć, choć `tickets` jest już puste — to jest
/// jedyny dopuszczony wyjątek od bramki „bez pozycji nic się nie rusza".
#[test]
fn realizacja_brokera_awansuje_etap_mimo_pustego_koszyka() {
    let mut cfg = baza_rynkowa();
    cfg.pending_lifetime = PendingLifetime::Never;
    cfg.tp_stage_from_broker_fill = true;
    // każda pozycja celuje w TP1, więc pierwszy ruch domyka koszyk do zera
    cfg.tp_schedule = TpSchedule::AllAtTp1;
    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, RYNEK));
    assert!(!b.positions().is_empty());

    tik(&mut e, &mut b, T0 + 1_000, 4011.0);
    assert!(b.positions().is_empty(), "wszystko wyszło na własnym TP1");
    assert!(
        e.baskets[0].tp_stage >= 1,
        "cel, za który DOSTALIŚMY pieniądze, musi się zaliczyć (etap {})",
        e.baskets[0].tp_stage
    );
}

// ============================================================
//  (c) SIATKA PRZEŻYWA PRZEJŚCIE CENY PRZEZ CELE
// ============================================================

/// `pending_drop_on_target = false` znaczy „siatka czeka na wypełnienie
/// niezależnie od tego, co w tym czasie zrobiła cena". Ścieżka CENOWA to
/// honorowała, ale ścieżka KANAŁOWA nie: `handle_tp_hit` kasował limity
/// własnym kodem w ogonie, nie pytając o tę oś ani razu.
///
/// Po naprawie reguła kasowania siatki ma JEDNO wejście (`drop_grid_on_target`)
/// i JEDNĄ oś, wspólną dla ceny, kanału i SPP.
#[test]
fn wylaczona_regula_zostawia_siatke_takze_przy_meldunku_z_kanalu() {
    let mut cfg = baza();
    cfg.pending_lifetime = PendingLifetime::UntilTp1;
    cfg.pending_drop_on_target = false;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, LIMITY));
    let ile = b.pendings().len();
    assert!(ile > 0, "test wymaga wiszącej siatki");

    // 1) cena mija TP1 — ścieżka cenowa
    tik(&mut e, &mut b, T0 + 1_000, 4011.0);
    assert_eq!(
        b.pendings().len(),
        ile,
        "przy wyłączonej regule cena nie kasuje siatki"
    );

    // 2) kanał melduje TP1 — ścieżka komunikatu
    e.on_message(&mut b, &wiadomosc(T0 + 2_000, 2, "✅ TP1 HIT +48 PIPS"));
    assert_eq!(
        b.pendings().len(),
        ile,
        "kanał kasował siatkę OMIJAJĄC oś `pending_drop_on_target` — to był drugi, ukryty egzemplarz reguły"
    );
    assert_eq!(e.baskets[0].tp_stage, 0, "i przy okazji przewijał etap");
}

/// Przy WŁĄCZONEJ regule (domyślnie) siatka ginie tak samo jak dotąd —
/// naprawa nie jest cichym wyłączeniem `pending_lifetime`.
#[test]
fn wlaczona_regula_dalej_kasuje_siatke_na_celu_bez_wejscia() {
    let mut cfg = baza();
    cfg.pending_lifetime = PendingLifetime::UntilTp1;
    assert!(cfg.pending_drop_on_target, "oś ma być domyślnie włączona");
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, LIMITY));
    assert!(!b.pendings().is_empty());

    tik(&mut e, &mut b, T0 + 1_000, 4011.0);
    assert!(
        b.pendings().is_empty(),
        "TP1 osiągnięty bez nas kończy życie siatki"
    );
    assert_eq!(e.baskets[0].tp_stage, 0, "ale NIE awansuje etapu");
    assert_eq!(
        e.baskets[0].plan_wykonany_do, 1,
        "obserwacja idzie do swojego pola"
    );
}

// ============================================================
//  (d) POZYCJA WYPEŁNIONA PO POWROCIE CENY CELUJE W TP1
// ============================================================

/// Pełna droga z opisu właściciela: sygnał, cena ucieka w górę przez cele,
/// kanał to melduje, a POTEM cena wraca i siatka się wypełnia.
///
/// Przed naprawą działy się dwie rzeczy naraz i obie złe: meldunek z kanału
/// kasował siatkę (więc wypełnienia nie było wcale), a gdyby siatka przeżyła —
/// etap stał na 2 i wejście celowałoby od razu w TP3.
#[test]
fn wypelnienie_po_powrocie_ceny_celuje_w_tp1_a_nie_w_tp3() {
    let mut cfg = baza();
    cfg.pending_lifetime = PendingLifetime::UntilTp1;
    cfg.pending_drop_on_target = false; // siatka ma dożyć powrotu
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, LIMITY));
    assert!(b.positions().is_empty());

    // rynek ucieka przez TP1 i TP2, kanał to melduje
    tik(&mut e, &mut b, T0 + 1_000, 4011.0);
    tik(&mut e, &mut b, T0 + 2_000, 4021.0);
    e.on_message(&mut b, &wiadomosc(T0 + 3_000, 2, "✅ TP1 HIT +48 PIPS"));
    e.on_message(&mut b, &wiadomosc(T0 + 4_000, 3, "✅ TP2 HIT +90 PIPS"));
    assert_eq!(e.baskets[0].tp_stage, 0, "nadal nie mamy ani jednego lota");

    // cena WRACA do strefy — limity się wypełniają
    tik(&mut e, &mut b, T0 + 5_000, 4001.0);
    assert!(
        !b.positions().is_empty(),
        "siatka miała przeżyć i wypełnić się po powrocie ceny"
    );

    let tp1 = 4010.0;
    for p in b.positions() {
        let cel = p.tp.expect("pozycja z siatki dostaje cel");
        assert!(
            (cel - tp1).abs() < 1e-6,
            "wejście po powrocie celuje w TP1 ({tp1}), a nie w cel przewinięty ruchem bez nas (dostało {cel})"
        );
    }
    assert_eq!(
        e.baskets[0].tp_stage, 0,
        "etap rusza dopiero z NASZEGO wejścia, a nie z ruchu sprzed niego"
    );
}

#[test]
fn fantomowy_etap_nie_otwiera_wejsc_rynkowych() {
    let mut cfg = baza();
    cfg.pending_lifetime = PendingLifetime::UntilTp1; // siatka ginie na TP1
    cfg.reenter_after_tp = true;
    cfg.reenter_min_tp_stage = 1;
    cfg.reenter_max = 0; // bez limitu — jak w wydanych presetach
    cfg.market_entry_step = 0.0;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, LIMITY));
    assert!(b.positions().is_empty());

    // rynek idzie do TP1 bez nas — siatka ginie, koszyk zostaje pusty
    tik(&mut e, &mut b, T0 + 1_000, 4011.0);
    assert!(
        b.pendings().is_empty(),
        "siatka skasowana regułą celu bez wejścia"
    );

    // cena wraca do strefy — nie ma już limitów, więc jedyne, co może tu
    // otworzyć pozycję, to re-entry napędzane etapem
    for i in 0..5 {
        tik(&mut e, &mut b, T0 + 5_000 + i * 1_000, 4002.0);
    }

    assert!(
        b.positions().is_empty(),
        "koszyk, który NIGDY nie wszedł, nie ma prawa dokładać po rynku ({} pozycji)",
        b.positions().len()
    );
    assert_eq!(e.baskets[0].reentries, 0, "żadnego powtórnego wejścia");
}
