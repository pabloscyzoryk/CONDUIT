
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_700_000_000_000;
const SALDO: f64 = 10_000.0;

/// Kanał A (Synergy) i kanał B (ZEN) — RÓŻNE czaty.
const CZAT_A: i64 = -1_000_000_000_301;
const CZAT_B: i64 = -1_000_000_000_302;

/// Ten sam czat, ale osobny TEMAT forum. Dla silnika to trzecie źródło.
fn zrodlo_a() -> SourceKey {
    SourceKey::new(CZAT_A, None)
}
fn zrodlo_b() -> SourceKey {
    SourceKey::new(CZAT_B, None)
}
fn zrodlo_a_temat() -> SourceKey {
    SourceKey::new(CZAT_A, Some(77))
}

/// Sygnały celowo o RÓŻNEJ geometrii — inaczej „koszyk B nietknięty" dałoby
/// się spełnić przez pomyłkę, bo oba koszyki wyglądałyby tak samo.
const SYGNAL_A: &str = "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3995";
const SYGNAL_B: &str = "BUY LIMITS GOLD @ 3905/3900 AREA\nTP 3910\nTP 3920\nTP 3930\nSL 3895";

/// Wariant dla testów DEDUPU: oba kanały nadają setup o TEJ SAMEJ geometrii.
///
/// To nie jest wygoda, tylko konieczność. Żeby sprawdzić dedup akcji, oba
/// koszyki muszą mieć JEDNOCZEŚNIE otwarte pozycje — a przy rozsuniętych
/// strefach zejście do strefy B przebija stop koszyka A i zostawia go bez
/// pozycji. Rozróżnialność bierze się wtedy z WARTOŚCI w komunikatach, a nie
/// z geometrii sygnału.
const SYGNAL_ROWNOLEGLY: &str = SYGNAL_A;

fn kwotowanie(ts: Ts, bid: f64) -> Quote {
    Quote {
        ts,
        bid,
        ask: bid + 0.20,
    }
}

fn wiad(zr: SourceKey, ts: Ts, id: i64, tekst: &str) -> IncomingMessage {
    IncomingMessage {
        ts,
        source: zr,
        source_name: "TEST".into(),
        msg_id: id,
        reply_to: None,
        edit_of: None,
        text: tekst.into(),
    }
}

fn wiad_edycja(zr: SourceKey, ts: Ts, id: i64, edytowana: i64, tekst: &str) -> IncomingMessage {
    let mut m = wiad(zr, ts, id, tekst);
    m.edit_of = Some(edytowana);
    m
}

fn wiad_odpowiedz(zr: SourceKey, ts: Ts, id: i64, na: i64, tekst: &str) -> IncomingMessage {
    let mut m = wiad(zr, ts, id, tekst);
    m.reply_to = Some(na);
    m
}

fn cfg_wielokanalowa() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3;
    c.lot_mode_percent = false;
    c.lot_fixed = 0.01;
    c.lot_min = 0.01;
    c.risk_per_basket_pct = 0.0;
    c.max_portfolio_risk_pct = 0.0;
    c.max_open_baskets = 0;
    c.max_open_positions = 0;
    c.tp_source = TpSource::Either;
    c.pending_lifetime = PendingLifetime::Never;
    c.pending_drop_on_target = false;
    c.ignore_old_after_min = 0.0;
    c.pending_ttl_h = 0.0;
    // Wskazówki cenowe wyłączone: chcemy mierzyć klucz ŹRÓDŁA, a nie to, czy
    // parser znalazł w treści liczbę pasującą do strefy któregoś koszyka.
    c.basket_hint_tolerance = 0.0;
    // „SL HIT" ma być jednoznacznie DESTRUKCYJNY, żeby dało się odróżnić
    // „trafiło we właściwy koszyk" od „nie zrobiło nic".
    c.sl_hit_mode = SlHitMode::CloseAll;
    c
}

fn stanowisko(cfg: Settings, bid: f64) -> (Engine, SimBroker) {
    let mut b = SimBroker::z_ustawien(SALDO, &cfg);
    b.on_quote(kwotowanie(T0, bid));
    let e = Engine::new(cfg, SALDO);
    (e, b)
}

/// Migawka koszyka do porównań „nietknięty co do pola".
#[derive(Debug, Clone, PartialEq)]
struct Odcisk {
    side: Side,
    zone: (Px, Px),
    entry: (Px, Px),
    sl: Option<Px>,
    tps: Vec<Px>,
    tp_stage: usize,
    secured: bool,
    state: BasketState,
    pendingow: usize,
}

fn odcisk(e: &Engine, zr: &SourceKey) -> Odcisk {
    let bk = e
        .baskets
        .iter()
        .find(|x| x.source == *zr)
        .unwrap_or_else(|| panic!("brak koszyka źródła {zr:?}"));
    Odcisk {
        side: bk.side,
        zone: (bk.zone_lo, bk.zone_hi),
        entry: (bk.entry_lo, bk.entry_hi),
        sl: bk.sl,
        tps: bk.tps.clone(),
        tp_stage: bk.tp_stage,
        secured: bk.secured,
        state: bk.state,
        pendingow: bk.pendings.len(),
    }
}

/// Dwa kanały nadają wejście pod TYM SAMYM `msg_id`.
fn dwa_kanaly_z_kolizja(msg_id: i64) -> (Engine, SimBroker) {
    let cfg = cfg_wielokanalowa();
    // cena między obiema strefami: obie siatki zostają limitami
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiad(zrodlo_a(), T0, msg_id, SYGNAL_A));
    e.on_message(&mut b, &wiad(zrodlo_b(), T0 + 100, msg_id, SYGNAL_B));
    assert_eq!(
        e.baskets.len(),
        2,
        "ten sam `msg_id` w DWÓCH kanałach to dwa różne sygnały — muszą powstać \
         dwa koszyki. Jeden koszyk znaczy, że któraś mapa ma kluczem gołe i64"
    );
    (e, b)
}

// ============================================================
//  1. SAMO ZAŁOŻENIE KOSZYKÓW
// ============================================================

#[test]
fn ten_sam_msgid_w_dwoch_kanalach_daje_dwa_koszyki() {
    let (e, b) = dwa_kanaly_z_kolizja(1);
    let a = odcisk(&e, &zrodlo_a());
    let z_b = odcisk(&e, &zrodlo_b());
    assert_eq!(a.zone, (4000.0, 4005.0));
    assert_eq!(z_b.zone, (3900.0, 3905.0));
    assert_eq!(
        b.pendings().len(),
        6,
        "każdy kanał ma własną siatkę trzech szczebli"
    );
}

/// A6 (`entry_idempotencja`) broni przed RE-DELIVERY — powtórką TEJ SAMEJ
/// wiadomości. Nie wolno jej mylić z kolizją: powtórka w kanale A i pierwsza
/// dostawa w kanale B mają identyczny `msg_id`, a znaczą co innego.
#[test]
fn idempotencja_nie_zjada_wiadomosci_drugiego_kanalu() {
    assert!(
        Settings::default().entry_idempotencja,
        "oś A6 jest domyślnie włączona — właśnie dlatego kolizja musi być tu sprawdzona"
    );
    let (mut e, mut b) = dwa_kanaly_z_kolizja(1);
    let przed_b = odcisk(&e, &zrodlo_b());

    // RE-DELIVERY w kanale A: ten sam `msg_id`, `edit_of = None`
    e.on_message(&mut b, &wiad(zrodlo_a(), T0 + 1_000, 1, SYGNAL_A));

    assert_eq!(e.baskets.len(), 2, "powtórka nie ma prawa dołożyć koszyka");
    assert_eq!(
        odcisk(&e, &zrodlo_b()),
        przed_b,
        "re-delivery w kanale A nie ma prawa ruszyć koszyka kanału B"
    );
}

// ============================================================
//  2. EDYCJA
// ============================================================

/// SEDNO ZADANIA I5: edycja z kanału A nie rusza koszyka z kanału B.
///
/// Edycja przesuwa CAŁĄ geometrię sygnału A o 4 $ w dół. Gdyby `msg_to_basket`
/// miało kluczem gołe `msg_id`, przezbrojony zostałby ten koszyk, który wpisał
/// się do mapy JAKO OSTATNI — czyli B — i to jego strefa poleciałaby na
/// wartości z kanału A.
#[test]
fn edycja_z_kanalu_a_nie_rusza_koszyka_z_kanalu_b() {
    let (mut e, mut b) = dwa_kanaly_z_kolizja(1);
    let przed_b = odcisk(&e, &zrodlo_b());

    e.on_message(
        &mut b,
        &wiad_edycja(
            zrodlo_a(),
            T0 + 1_000,
            1,
            1,
            "BUY LIMITS GOLD @ 4001/3996 AREA\nTP 4006\nTP 4016\nTP 4026\nSL 3991",
        ),
    );

    let po_a = odcisk(&e, &zrodlo_a());
    assert_eq!(
        po_a.zone,
        (3996.0, 4001.0),
        "edycja MA przezbroić własny koszyk — inaczej test niczego nie mierzy"
    );
    assert_eq!(po_a.sl, Some(3991.0));
    assert_eq!(
        odcisk(&e, &zrodlo_b()),
        przed_b,
        "koszyk kanału B ma zostać NIETKNIĘTY co do pola"
    );
    assert_eq!(e.baskets.len(), 2, "edycja nie zakłada trzeciego koszyka");
}

/// Odwrotny kierunek. Bez niego test wyżej przechodziłby także wtedy, gdyby
/// edycja trafiała zawsze w koszyk STARSZY zamiast we właściwy.
#[test]
fn edycja_z_kanalu_b_nie_rusza_koszyka_z_kanalu_a() {
    let (mut e, mut b) = dwa_kanaly_z_kolizja(1);
    let przed_a = odcisk(&e, &zrodlo_a());

    e.on_message(
        &mut b,
        &wiad_edycja(
            zrodlo_b(),
            T0 + 1_000,
            1,
            1,
            "BUY LIMITS GOLD @ 3901/3896 AREA\nTP 3906\nTP 3916\nTP 3926\nSL 3891",
        ),
    );

    assert_eq!(odcisk(&e, &zrodlo_b()).zone, (3896.0, 3901.0));
    assert_eq!(odcisk(&e, &zrodlo_a()), przed_a);
}

/// TEMAT FORUM to osobne źródło. Ten sam czat, ten sam `msg_id`, inny temat —
/// i to nadal muszą być dwa niezależne koszyki.
#[test]
fn temat_forum_jest_osobnym_zrodlem() {
    let cfg = cfg_wielokanalowa();
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiad(zrodlo_a(), T0, 1, SYGNAL_A));
    e.on_message(&mut b, &wiad(zrodlo_a_temat(), T0 + 100, 1, SYGNAL_B));
    assert_eq!(
        e.baskets.len(),
        2,
        "czat + temat to inne źródło niż sam czat"
    );

    let przed = odcisk(&e, &zrodlo_a_temat());
    e.on_message(
        &mut b,
        &wiad_edycja(
            zrodlo_a(),
            T0 + 1_000,
            1,
            1,
            "BUY LIMITS GOLD @ 4001/3996 AREA\nTP 4006\nTP 4016\nTP 4026\nSL 3991",
        ),
    );
    assert_eq!(odcisk(&e, &zrodlo_a()).zone, (3996.0, 4001.0));
    assert_eq!(
        odcisk(&e, &zrodlo_a_temat()),
        przed,
        "sygnał z jednego tematu nigdy nie rusza koszyka innego tematu"
    );
}

// ============================================================
//  3. DEDUP AKCJI
// ============================================================

/// Pamięć wykonanych akcji (`done_actions`) też musi być kluczowana źródłem.
///
/// Scenariusz: obie strony mają koszyk z pozycją. W kanale A wiadomość 500
/// przesuwa stop; w kanale B wiadomość o TYM SAMYM numerze 500 przesuwa stop
/// swojego koszyka. Przy gołym `i64` druga akcja zostałaby zjedzona jako
/// „już wykonana" — stop kanału B nigdy by się nie ruszył.
#[test]
fn dedup_akcji_nie_myli_watkow_dwoch_kanalow() {
    let (mut e, mut b) = dwa_kanaly_w_rynku();

    // TEN SAM numer wiadomości, dwa kanały, dwie różne wartości stopu
    e.on_message(
        &mut b,
        &wiad(zrodlo_a(), T0 + 5_000, 500, "MOVE SL TO 3998"),
    );
    e.on_message(
        &mut b,
        &wiad(zrodlo_b(), T0 + 5_100, 500, "MOVE SL TO 3999"),
    );

    let a = e.baskets.iter().find(|x| x.source == zrodlo_a()).unwrap();
    let z_b = e.baskets.iter().find(|x| x.source == zrodlo_b()).unwrap();
    assert_eq!(a.sl, Some(3998.0), "stop kanału A");
    assert_eq!(
        z_b.sl,
        Some(3999.0),
        "stop kanału B zginął w dedupie przez kolizję `msg_id` — \
         przy gołym i64 zostałby na 3995 z sygnału"
    );
}

/// Dwa kanały, ta sama geometria, OBA koszyki z otwartymi pozycjami.
fn dwa_kanaly_w_rynku() -> (Engine, SimBroker) {
    let cfg = cfg_wielokanalowa();
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiad(zrodlo_a(), T0, 1, SYGNAL_ROWNOLEGLY));
    e.on_message(&mut b, &wiad(zrodlo_b(), T0 + 100, 1, SYGNAL_ROWNOLEGLY));
    assert_eq!(e.baskets.len(), 2);
    // jedno zejście w strefę wypełnia obie siatki, potem powrót nad strefę,
    // żeby oba stopy (3998/3999) były u brokera wykonalne
    for (i, bid) in [3999.0, 4008.0].iter().enumerate() {
        let q = kwotowanie(T0 + 1_000 * (i as i64 + 1), *bid);
        b.on_quote(q);
        e.on_tick(&mut b, &q);
    }
    assert!(
        e.baskets.iter().all(|x| !x.tickets.is_empty()),
        "przesłanka: oba koszyki muszą mieć pozycje"
    );
    (e, b)
}

/// To samo dla EDYCJI komunikatu zarządzającego: edycja wiadomości 500
/// w kanale B nie ma prawa być zjedzona dlatego, że wiadomość 500 kanału A
/// wykonała już taką akcję.
#[test]
fn edycja_komunikatu_nie_dedupuje_sie_przez_kanaly() {
    let (mut e, mut b) = dwa_kanaly_w_rynku();

    // kanał A wykonuje „TP1 HIT" w wiadomości 500
    e.on_message(
        &mut b,
        &wiad(zrodlo_a(), T0 + 5_000, 500, "✅ TP1 HIT +50 PIPS"),
    );
    let a = e.baskets.iter().find(|x| x.source == zrodlo_a()).unwrap();
    assert_eq!(a.tp_stage, 1, "przesłanka: kanał A naprawdę zaliczył TP1");

    // kanał B: EDYCJA wiadomości 500 z tą samą akcją — numer koliduje, źródło NIE
    e.on_message(
        &mut b,
        &wiad_edycja(zrodlo_b(), T0 + 5_100, 500, 500, "✅ TP1 HIT +50 PIPS"),
    );
    let z_b = e.baskets.iter().find(|x| x.source == zrodlo_b()).unwrap();
    assert_eq!(
        z_b.tp_stage, 1,
        "TP1 kanału B zginęło w dedupie przez kolizję `msg_id` (etap {})",
        z_b.tp_stage
    );
}

// ============================================================
//  4. ODPOWIEDŹ (`reply_to`)
// ============================================================

/// `reply_to` niesie GOŁY numer wiadomości — bez czatu. Rozstrzygnięcie, czyja
/// to odpowiedź, może więc pochodzić WYŁĄCZNIE ze źródła samej odpowiedzi.
///
/// Scenariusz: oba kanały mają sygnał o `msg_id = 1`. Kanał B odpowiada na
/// „wiadomość 1" komunikatem „SL HIT". Przy gołym kluczu odpowiedź zabiłaby
/// koszyk kanału A.
#[test]
fn odpowiedz_trafia_do_koszyka_wlasnego_zrodla() {
    let (mut e, mut b) = dwa_kanaly_z_kolizja(1);
    let przed_a = odcisk(&e, &zrodlo_a());

    e.on_message(
        &mut b,
        &wiad_odpowiedz(zrodlo_b(), T0 + 1_000, 900, 1, "❌ SL HIT -100 PIPS"),
    );

    let z_b = e.baskets.iter().find(|x| x.source == zrodlo_b()).unwrap();
    assert_eq!(
        z_b.state,
        BasketState::Done,
        "odpowiedź „SL HIT\" ma zamknąć koszyk WŁASNEGO kanału"
    );
    assert_eq!(
        odcisk(&e, &zrodlo_a()),
        przed_a,
        "koszyk kanału A nie ma prawa zginąć od cudzej odpowiedzi"
    );
}

/// Odwrotny kierunek — i przy okazji dowód, że rozstrzyga ŹRÓDŁO, a nie
/// kolejność wpisów do mapy.
#[test]
fn odpowiedz_z_kanalu_a_konczy_koszyk_a() {
    let (mut e, mut b) = dwa_kanaly_z_kolizja(1);
    let przed_b = odcisk(&e, &zrodlo_b());

    e.on_message(
        &mut b,
        &wiad_odpowiedz(zrodlo_a(), T0 + 1_000, 900, 1, "❌ SL HIT -100 PIPS"),
    );

    let a = e.baskets.iter().find(|x| x.source == zrodlo_a()).unwrap();
    assert_eq!(a.state, BasketState::Done);
    assert_eq!(odcisk(&e, &zrodlo_b()), przed_b);
}

/// Odpowiedź na numer, który W TYM ŹRÓDLE nie istnieje (ale istnieje
/// w drugim), nie ma prawa zostać przekierowana do cudzego koszyka. Przy
/// włączonym wecie odpowiedzi (F1) taki komunikat ma zostać ODRZUCONY,
/// a nie spaść na „najnowszy żywy koszyk".
#[test]
fn odpowiedz_na_cudzy_numer_nie_wykonuje_sie_u_nas() {
    let cfg = {
        let mut c = cfg_wielokanalowa();
        c.reply_veto = true;
        c
    };
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    // sygnał TYLKO w kanale A, o numerze 1
    e.on_message(&mut b, &wiad(zrodlo_a(), T0, 1, SYGNAL_A));
    // …a w kanale B mamy własny koszyk pod INNYM numerem
    e.on_message(&mut b, &wiad(zrodlo_b(), T0 + 100, 2, SYGNAL_B));
    let przed_a = odcisk(&e, &zrodlo_a());
    let przed_b = odcisk(&e, &zrodlo_b());

    // kanał B odpowiada na „wiadomość 1", której w kanale B nigdy nie było
    e.on_message(
        &mut b,
        &wiad_odpowiedz(zrodlo_b(), T0 + 1_000, 900, 1, "❌ SL HIT -100 PIPS"),
    );

    assert_eq!(
        odcisk(&e, &zrodlo_a()),
        przed_a,
        "adresat jest podany wprost i to nie jest kanał A"
    );
    assert_eq!(
        odcisk(&e, &zrodlo_b()),
        przed_b,
        "…a w kanale B nie ma wiadomości 1, więc komunikat nie ma do czego się odnieść"
    );
}
