
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_700_000_000_000;
const SALDO: f64 = 10_000.0;

const CZAT_A: i64 = -1_000_000_000_301;
const CZAT_B: i64 = -1_000_000_000_302;

const SYGNAL_A: &str = "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3995";
const SYGNAL_B: &str = "BUY LIMITS GOLD @ 3905/3900 AREA\nTP 3910\nTP 3920\nTP 3930\nSL 3895";

fn zrodlo(chat: i64) -> SourceKey {
    SourceKey::new(chat, None)
}

fn kwotowanie(ts: Ts, bid: f64) -> Quote {
    Quote {
        ts,
        bid,
        ask: bid + 0.20,
    }
}

fn wiad(chat: i64, ts: Ts, id: i64, tekst: &str) -> IncomingMessage {
    IncomingMessage {
        ts,
        source: zrodlo(chat),
        source_name: "TEST".into(),
        msg_id: id,
        reply_to: None,
        edit_of: None,
        text: tekst.into(),
    }
}

fn odpowiedz(chat: i64, ts: Ts, id: i64, na: i64, tekst: &str) -> IncomingMessage {
    let mut m = wiad(chat, ts, id, tekst);
    m.reply_to = Some(na);
    m
}

fn cfg_restart() -> Settings {
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
    c.basket_hint_tolerance = 0.0;
    c
}

fn tik(e: &mut Engine, b: &mut SimBroker, ts: Ts, bid: f64) {
    let q = kwotowanie(ts, bid);
    b.on_quote(q);
    e.on_tick(b, &q);
}

/// ZRZUT — dokładnie ta droga, którą idzie `koszyki.json`.
fn zrzut(e: &Engine) -> String {
    serde_json::to_string(&e.baskets).expect("koszyki muszą się serializować")
}

fn z_zrzutu(json: &str) -> Vec<Basket> {
    serde_json::from_str(json).expect("zrzut koszyków musi się wczytać")
}

/// Wyłączenie bota: silnik ginie, BROKER ZOSTAJE. Zwraca świeży silnik
/// z koszykami odtworzonymi ze zrzutu.
fn restart(cfg: Settings, stary: &Engine) -> Engine {
    let json = zrzut(stary);
    let mut nowy = Engine::new(cfg, SALDO);
    nowy.adopt_baskets(z_zrzutu(&json));
    nowy
}

/// Stanowisko przed restartem: jeden koszyk, siatka limitów, bez wypełnień.
fn przed_restartem_siatka() -> (Engine, SimBroker) {
    let cfg = cfg_restart();
    let mut b = SimBroker::z_ustawien(SALDO, &cfg);
    b.on_quote(kwotowanie(T0, 4008.0));
    let mut e = Engine::new(cfg, SALDO);
    e.on_message(&mut b, &wiad(CZAT_A, T0, 501, SYGNAL_A));
    assert_eq!(b.pendings().len(), 3, "przesłanka: trzy limity u brokera");
    (e, b)
}

/// Stanowisko przed restartem: koszyk W RYNKU, po trafionym TP1.
fn przed_restartem_w_rynku() -> (Engine, SimBroker) {
    let mut cfg = cfg_restart();
    cfg.tp_schedule = TpSchedule::AllRunners;
    let mut b = SimBroker::z_ustawien(SALDO, &cfg);
    b.on_quote(kwotowanie(T0, 4008.0));
    let mut e = Engine::new(cfg, SALDO);
    e.on_message(&mut b, &wiad(CZAT_A, T0, 501, SYGNAL_A));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0);
    tik(&mut e, &mut b, T0 + 2_000, 4011.0);
    assert_eq!(b.positions().len(), 3, "przesłanka: trzy pozycje");
    assert_eq!(
        e.baskets[0].tp_stage, 1,
        "przesłanka: TP1 zaliczone Z POZYCJĄ"
    );
    (e, b)
}

// ============================================================
//  1. TREŚĆ KOSZYKA PRZEŻYWA RESTART
// ============================================================

/// Zrzut → wczytanie → adopcja musi dać koszyk IDENTYCZNY co do pola.
///
/// To nie jest test serde dla samego serde: `koszyki.json` jest JEDYNYM
/// źródłem drabinki celów, planu siatki i `msg_id`. Konto zna tylko ceny.
#[test]
fn tresc_koszyka_przezywa_pelny_obieg_przez_json() {
    let (e, _b) = przed_restartem_w_rynku();
    let przed = e.baskets[0].clone();

    let po = z_zrzutu(&zrzut(&e)).remove(0);

    assert_eq!(po.id, przed.id);
    assert_eq!(po.source, przed.source, "kanał źródłowy przeżywa restart");
    assert_eq!(
        po.msg_id, przed.msg_id,
        "wiązanie odpowiedzi przeżywa restart"
    );
    assert_eq!(po.tps, przed.tps, "drabinka celów przeżywa restart");
    assert_eq!(po.sl, przed.sl);
    assert_eq!((po.zone_lo, po.zone_hi), (przed.zone_lo, przed.zone_hi));
    assert_eq!(po.tp_stage, przed.tp_stage, "etap celów przeżywa restart");
    assert_eq!(po.tickets, przed.tickets);
    assert_eq!(
        po.levels.len(),
        przed.levels.len(),
        "plan siatki przeżywa restart"
    );
    assert_eq!(po.state, przed.state);
    assert!(po.had_positions);
}

/// KONTRAKT ZRZUTU: brak JEDNEGO pola w JEDNYM koszyku kasuje CAŁY plik.
///
/// `wznowienie::wczytaj` robi `serde_json::from_str::<Zrzut>(…).ok()?`, więc
/// błąd na dowolnym polu dowolnego koszyka znaczy „zrzutu nie ma" — czyli
/// WSZYSTKIE koszyki lecą ścieżką awaryjną „zlep z samego konta", bez
/// drabinki celów i bez `msg_id`. To jest dokładnie ten sposób, w jaki
/// dołożenie pola bez `#[serde(default)]` produkuje sieroty po aktualizacji
/// binarki.
///
/// Test jest tu po to, żeby ta cena była WIDOCZNA przy każdym nowym polu.
#[test]
fn nowe_pole_bez_serde_default_kasuje_caly_zrzut() {
    let (e, _b) = przed_restartem_w_rynku();
    let json = zrzut(&e);

    // starsza binarka nie zapisała pola `tp_stage`
    let stary: serde_json::Value = serde_json::from_str(&json).unwrap();
    let mut tab = stary.as_array().unwrap().clone();
    tab[0].as_object_mut().unwrap().remove("tp_stage");
    let okrojony = serde_json::to_string(&serde_json::Value::Array(tab)).unwrap();

    assert!(
        serde_json::from_str::<Vec<Basket>>(&okrojony).is_err(),
        "pole BEZ `#[serde(default)]` jest wymagane — a to znaczy, że stary \
         zrzut przestaje się wczytywać W CAŁOŚCI. Każde nowe pole `Basket` \
         musi mieć `#[serde(default)]`, inaczej pierwszy restart po \
         aktualizacji binarki robi z żywych pozycji sieroty"
    );

    // …i druga strona: pole NIEZNANE nowej binarce jest po cichu pomijane,
    // więc cofnięcie się do starszej wersji jest bezpieczne
    let mut tab = serde_json::from_str::<serde_json::Value>(&json)
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    tab[0]
        .as_object_mut()
        .unwrap()
        .insert("pole_z_przyszlosci".into(), serde_json::json!(7));
    let z_nadmiarem = serde_json::to_string(&serde_json::Value::Array(tab)).unwrap();
    assert!(
        serde_json::from_str::<Vec<Basket>>(&z_nadmiarem).is_ok(),
        "nieznane pole nie ma prawa wywalić zrzutu"
    );
}

// ============================================================
//  2. ADOPCJA W SILNIKU
// ============================================================

/// Po adopcji koszyk musi być OSIĄGALNY dla komunikatów z kanału — inaczej
/// wraca do panelu, ale kanał nim nie rządzi i mamy sierotę z ładnym wierszem.
#[test]
fn po_restarcie_komunikat_z_kanalu_trafia_w_odtworzony_koszyk() {
    let (stary, mut b) = przed_restartem_w_rynku();
    let mut e = restart(cfg_restart_z_runnerami(), &stary);
    assert_eq!(e.baskets.len(), 1, "koszyk wrócił");

    // ODPOWIEDŹ na wiadomość, która ten koszyk założyła — najmocniejsze
    // z możliwych wiązań, i jedyne, które przeżywa wyłączenie procesu
    e.on_message(
        &mut b,
        &odpowiedz(CZAT_A, T0 + 10_000, 900, 501, "MOVE SL TO 4001"),
    );
    assert_eq!(
        e.baskets[0].sl,
        Some(4001.0),
        "odpowiedź na `msg_id` koszyka musi go znaleźć po restarcie — \
         `adopt_baskets` odbudowuje mapę `msg_to_basket`"
    );

    // …i zwykły komunikat zarządzający z tego samego źródła też
    e.on_message(
        &mut b,
        &wiad(CZAT_A, T0 + 11_000, 901, "✅ TP2 HIT +200 PIPS"),
    );
    assert_eq!(
        e.baskets[0].tp_stage, 2,
        "etap idzie dalej od miejsca, w którym stanął"
    );
}

fn cfg_restart_z_runnerami() -> Settings {
    let mut c = cfg_restart();
    c.tp_schedule = TpSchedule::AllRunners;
    c
}

/// Numer koszyka po restarcie NIE MOŻE się powtórzyć. Dwa koszyki o tym samym
/// numerze znaczą, że komentarz w MT5 (`B7`) wskazuje na dwa różne byty —
/// a to jest ta sama klasa błędu co kolizja `msg_id`, tylko po stronie konta.
#[test]
fn numeracja_koszykow_nie_zaczyna_sie_od_nowa() {
    let (stary, mut b) = przed_restartem_w_rynku();
    let id_przed = stary.baskets[0].id;
    let mut e = restart(cfg_restart_z_runnerami(), &stary);
    assert!(
        e.next_basket_id() > id_przed,
        "licznik koszyków ma przeskoczyć ponad odtworzone numery ({} vs {id_przed})",
        e.next_basket_id()
    );

    // świeży sygnał po restarcie dostaje WŁASNY numer
    b.on_quote(kwotowanie(T0 + 20_000, 3908.0));
    e.on_message(&mut b, &wiad(CZAT_B, T0 + 20_000, 1, SYGNAL_B));
    assert_eq!(e.baskets.len(), 2);
    let numery: Vec<u32> = e.baskets.iter().map(|x| x.id).collect();
    assert_ne!(
        numery[0], numery[1],
        "dwa koszyki nie mogą dzielić numeru: {numery:?}"
    );
}

#[test]
fn koszyki_dwoch_kanalow_wracaja_osobno() {
    let cfg = cfg_restart();
    let mut b = SimBroker::z_ustawien(SALDO, &cfg);
    b.on_quote(kwotowanie(T0, 4008.0));
    let mut stary = Engine::new(cfg, SALDO);
    stary.on_message(&mut b, &wiad(CZAT_A, T0, 1, SYGNAL_A));
    stary.on_message(&mut b, &wiad(CZAT_B, T0 + 100, 1, SYGNAL_B));
    assert_eq!(stary.baskets.len(), 2);

    let mut e = restart(cfg_restart(), &stary);
    assert_eq!(e.baskets.len(), 2, "oba koszyki wracają");

    let przed_b = e
        .baskets
        .iter()
        .find(|x| x.source == zrodlo(CZAT_B))
        .cloned()
        .expect("koszyk kanału B");

    // odpowiedź kanału A na „wiadomość 1" — numer koliduje z kanałem B
    e.on_message(
        &mut b,
        &odpowiedz(CZAT_A, T0 + 10_000, 900, 1, "MOVE SL TO 3998"),
    );

    let po_a = e
        .baskets
        .iter()
        .find(|x| x.source == zrodlo(CZAT_A))
        .unwrap();
    let po_b = e
        .baskets
        .iter()
        .find(|x| x.source == zrodlo(CZAT_B))
        .unwrap();
    assert_eq!(po_a.sl, Some(3998.0), "adresat to kanał A");
    assert_eq!(
        po_b.sl, przed_b.sl,
        "koszyk kanału B ma zostać nietknięty — po restarcie tak samo jak przed"
    );
}

// ============================================================
//  3. RESTART NIE MA PRAWA WEJŚĆ W RYNEK
// ============================================================

/// Adopcja wstrzykuje GOTOWY stan. Sam ten fakt nie może otworzyć ani jednego
/// zlecenia — bot po starcie ma ZASTAĆ rynek, a nie w niego wejść.
#[test]
fn adopcja_sama_z_siebie_nie_sklada_zlecen() {
    let (stary, mut b) = przed_restartem_siatka();
    let bilety_przed: Vec<Ticket> = b.pendings().iter().map(|o| o.ticket).collect();
    let pozycji_przed = b.positions().len();

    let mut e = restart(cfg_restart(), &stary);
    // kilka ticków „na sucho", bez ruchu w strefę
    for i in 0..5 {
        tik(
            &mut e,
            &mut b,
            T0 + 30_000 + 1_000 * i,
            4008.0 + i as f64 * 0.1,
        );
    }

    let bilety_po: Vec<Ticket> = b.pendings().iter().map(|o| o.ticket).collect();
    assert_eq!(
        bilety_po, bilety_przed,
        "po restarcie u brokera mają leżeć DOKŁADNIE te same zlecenia"
    );
    assert_eq!(
        b.positions().len(),
        pozycji_przed,
        "…i ani jedna nowa pozycja"
    );
}

/// Konfiguracja z WŁĄCZONYM przezbrojeniem siatki — jedyną regułą, która woła
/// `sync_grid(.., only_existing = false)`, czyli naprawdę odbudowuje szczeble
/// bez żywego zlecenia. `resize_pendings` i budzenie sesyjne wołają wariant
/// `true` i szczebla nieobecnego u brokera nie ruszą.
fn cfg_restart_z_przezbrojeniem() -> Settings {
    let mut c = cfg_restart_z_runnerami();
    c.rearm_grid_on_return = true;
    c.rearm_max_times = 0; // bez limitu powtórzeń
    c.rearm_min_gap_min = 0.0;
    c.rearm_min_basket_profit = 0.0;
    c
}

/// Koszyk po restarcie: dwa szczeble weszły, trzeci ma w PLANIE `filled=false`,
/// a u brokera nie stoi (zlecenie wygasło albo skasował je człowiek w terminalu).
fn przed_restartem_z_dziura() -> (Engine, SimBroker) {
    let cfg = cfg_restart_z_przezbrojeniem();
    let mut b = SimBroker::z_ustawien(SALDO, &cfg);
    b.on_quote(kwotowanie(T0, 4008.0));
    let mut e = Engine::new(cfg, SALDO);
    e.on_message(&mut b, &wiad(CZAT_A, T0, 501, SYGNAL_A));
    // cofka TYLKO do 4001: wchodzą dwa płytsze szczeble, najgłębszy (4000) czeka
    tik(&mut e, &mut b, T0 + 1_000, 4001.0);
    assert_eq!(b.positions().len(), 2, "przesłanka: dwa szczeble weszły");
    assert_eq!(
        b.pendings().len(),
        1,
        "przesłanka: najgłębszy szczebel czeka"
    );
    // rynek idzie w górę, koszyk na plusie (warunek przezbrojenia)
    tik(&mut e, &mut b, T0 + 2_000, 4008.0);
    // …i broker kasuje ostatni limit, gdy bota nie ma
    let ofiara = b.pendings()[0].ticket;
    b.cancel_pending(ofiara).unwrap();
    assert!(b.pendings().is_empty());
    let niewypelnione = e.baskets[0].levels.iter().filter(|g| !g.filled).count();
    assert_eq!(
        niewypelnione, 1,
        "przesłanka: jeden szczebel planu ma `filled = false`"
    );
    (e, b)
}

/// TO JEST POWÓD, dla którego `wznowienie::dopasuj_do_konta` zamyka furtkę
/// znacznikiem `filled = true`.
///
/// Po restarcie NIE DA SIĘ odróżnić szczebla SKASOWANEGO (wolno odtworzyć) od
/// ZREALIZOWANEGO I ZAMKNIĘTEGO (odtworzenie to ciche, nieproszone wejście).
/// Ten test dowodzi, że reguła przezbrojenia NAPRAWDĘ odstawia taki szczebel —
/// czyli że tamta jedna linijka jest jedyną rzeczą, która nas przed tym broni.
///
/// ŚWIECI NA CZERWONO, gdy: `sync_grid` przestanie odbudowywać szczeble
/// `!filled` przy `only_existing = false` — a wtedy `wznowienie` można
/// uprościć. Dopóki jest zielony, tamtej linijki NIE WOLNO ruszyć.
#[test]
fn szczebel_odblokowany_po_restarcie_naprawde_dostawia_zlecenie() {
    let (stary, mut b) = przed_restartem_z_dziura();
    let mut e = restart(cfg_restart_z_przezbrojeniem(), &stary);
    assert!(
        e.baskets[0].levels.iter().any(|g| !g.filled),
        "adoptujemy SUROWY zrzut — tak wyglądałby koszyk bez blokady szczebli"
    );

    // cena wraca do strefy, koszyk na plusie → przezbrojenie
    tik(&mut e, &mut b, T0 + 30_000, 4004.0);

    assert_eq!(
        b.pendings().len(),
        1,
        "silnik odbudował szczebel, którego u brokera nie było — i dokładnie \
         dlatego `wznowienie` musi go najpierw oznaczyć jako `filled`"
    );
}

#[test]
fn luka_znacznik_filled_nie_chroni_przed_przezbrojeniem() {
    let (stary, mut b) = przed_restartem_z_dziura();
    let mut e = restart(cfg_restart_z_przezbrojeniem(), &stary);
    // dokładnie to, co robi `wznowienie::dopasuj_do_konta`
    for gl in e.baskets[0].levels.iter_mut() {
        gl.filled = true;
    }

    tik(&mut e, &mut b, T0 + 30_000, 4004.0);

    assert_eq!(
        b.pendings().len(),
        1,
        "LUKA: mimo `filled = true` przezbrojenie odstawiło szczebel. Jeżeli \
         ten test padł, sprawdź `sync_grid` FAZA B — ktoś dołożył tam warunek \
         `!z.filled` albo zepsuł `rearm_pass`"
    );
}

// ============================================================
//  4. KONTRAKT SEMANTYKI PO ADOPCJI
// ============================================================

/// Koszyk odtworzony z SAMEGO KONTA (bez zrzutu) nie zna przebytej drogi.
/// `wznowienie::z_samego_konta` wpisuje mu `tp_stage = 0` — i silnik musi to
/// uszanować: dopóki koszyk nie ma pozycji, etap nie ma prawa ruszyć, choćby
/// cena minęła wszystkie cele.
///
/// UWAGA: ten test dzieli regułę z `semantyka_koszyka_1808.rs` (D1) i
/// `semantyka_wypelnienia.rs`. Stoi tutaj osobno, bo adopcja jest DRUGĄ drogą
/// wejścia stanu do silnika — obok wiadomości — i naprawa jednej z nich nie
/// naprawia drugiej.
#[test]
fn odtworzony_koszyk_bez_pozycji_nie_awansuje_etapu() {
    let (stary, mut b) = przed_restartem_siatka();
    // Reguła kasowania siatki WŁĄCZONA (jak w każdym wydanym presecie), a
    // życie limitów `Never` — czyli ustawienie, które ma ją WYCISZYĆ. Etap
    // koszyka nie ma prawa ruszyć w żadnym z tych dwóch miejsc.
    let mut cfg = cfg_restart();
    cfg.pending_drop_on_target = true;
    let mut e = restart(cfg, &stary);
    assert_eq!(e.baskets[0].tp_stage, 0);
    assert!(e.baskets[0].tickets.is_empty(), "przesłanka: sama siatka");

    // rynek przechodzi całą drabinkę, a my nadal nic nie mamy
    for (i, bid) in [4011.0, 4021.0, 4031.0].iter().enumerate() {
        tik(&mut e, &mut b, T0 + 30_000 + 1_000 * (i as i64), *bid);
    }

    assert_eq!(
        b.filled_pendings, 0,
        "przesłanka: cena nie schodziła do strefy, więc nic się nie wypełniło"
    );
    assert_eq!(
        e.baskets[0].tp_stage, 0,
        "koszyk odtworzony po restarcie awansował etap BEZ POZYCJI do {} — \
         adopcja musi respektować ten sam kontrakt co zwykła praca silnika",
        e.baskets[0].tp_stage
    );
    assert!(!e.baskets[0].had_positions);
}

/// Koszyk odtworzony Z POZYCJAMI liczy dalej OD SWOJEGO etapu — nie od zera
/// i nie od końca drabinki.
#[test]
fn odtworzony_koszyk_z_pozycjami_liczy_dalej_od_swojego_etapu() {
    let (stary, mut b) = przed_restartem_w_rynku();
    let mut e = restart(cfg_restart_z_runnerami(), &stary);
    assert_eq!(e.baskets[0].tp_stage, 1, "etap przeżył restart");

    // rynek idzie do DRUGIEGO celu
    tik(&mut e, &mut b, T0 + 30_000, 4021.0);
    assert_eq!(
        e.baskets[0].tp_stage, 2,
        "po restarcie następnym celem jest TP2 — ani TP1 od nowa, ani przeskok"
    );

    // …i do trzeciego
    tik(&mut e, &mut b, T0 + 31_000, 4031.0);
    assert_eq!(e.baskets[0].tp_stage, 3);
}
