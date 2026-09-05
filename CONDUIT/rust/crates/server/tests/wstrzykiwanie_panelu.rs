
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;
use conduit_server::proto::{ClientMsg, Command};
use conduit_server::wstrzykniecie;

const T0: Ts = 1_700_000_000_000;
const SALDO: f64 = 10_000.0;

/// Prawdziwy `chat_id` — taki, jaki operator wybiera w panelu, gdy chce
/// wstrzyknąć wiadomość do istniejącego kanału.
const CZAT: i64 = -1_000_000_000_301;
const CZAT_DRUGI: i64 = -1_000_000_000_302;

const WEJSCIE: &str = "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3995";
const WEJSCIE_NIZEJ: &str = "BUY LIMITS GOLD @ 3905/3900 AREA\nTP 3910\nTP 3920\nTP 3930\nSL 3895";
const WEJSCIE_PRZESUNIETE: &str =
    "BUY LIMITS GOLD @ 4001/3996 AREA\nTP 4006\nTP 4016\nTP 4026\nSL 3991";

// ============================================================
//  STANOWISKO
// ============================================================

fn kwotowanie(ts: Ts, bid: f64) -> Quote {
    Quote {
        ts,
        bid,
        ask: bid + 0.20,
    }
}

fn ustawienia() -> Settings {
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
    // Wskazówki cenowe wyłączone: mierzymy ADRESOWANIE wiadomości, a nie to,
    // czy parser znalazł w treści liczbę pasującą do którejś strefy.
    c.basket_hint_tolerance = 0.0;
    // „SL HIT" ma być jednoznacznie destrukcyjny — inaczej nie da się odróżnić
    // „trafiło we właściwy koszyk" od „nie zrobiło nic".
    c.sl_hit_mode = SlHitMode::CloseAll;
    c
}

fn stanowisko(bid: f64) -> (Engine, SimBroker) {
    let cfg = ustawienia();
    let mut b = SimBroker::z_ustawien(SALDO, &cfg);
    b.on_quote(kwotowanie(T0, bid));
    let e = Engine::new(cfg, SALDO);
    (e, b)
}

/// Parsuje PRAWDZIWY komunikat WebSocket i wyciąga z niego komendę.
///
/// Świadomie idziemy przez `ClientMsg`, a nie przez `Command` wprost: nazwy
/// pól po drucie są w camelCase (`msgId`, `editOf`, `replyTo`, `topicId`)
/// i to jest dokładnie ten szew, na którym pola gubią się najczęściej.
fn komenda(json: &str) -> Command {
    match serde_json::from_str::<ClientMsg>(json).expect("poprawny komunikat klienta") {
        ClientMsg::Command { cmd, .. } => cmd,
        inny => panic!("zły wariant: {inny:?}"),
    }
}

fn wiadomosc(json: &str, ts: Ts) -> IncomingMessage {
    wstrzykniecie::wiadomosc(&komenda(json), ts, wstrzykniecie::ZRODLO_PANELU)
        .expect("to jest simulateMessage")
}

fn koszyk<'a>(e: &'a Engine, zr: &SourceKey) -> &'a conduit_core::types::Basket {
    e.baskets
        .iter()
        .find(|x| x.source == *zr)
        .unwrap_or_else(|| panic!("brak koszyka źródła {zr:?}"))
}

// ============================================================
//  1. KONTRAKT ZERA
// ============================================================

/// Polecenie z SAMYM tekstem — dokładnie to, co panel wysyłał przed F5.
///
/// Brak pola ma znaczyć „jak dotąd", a nie „domyślna wartość, która coś
/// zmienia". Gdyby np. `topic_id` zaczęło domyślnie wskazywać temat, każdy
/// dzisiejszy sygnał ręczny wylądowałby w innym źródle niż wczoraj — czyli
/// w innym koszyku.
#[test]
fn kontrakt_zera_samo_tekst_dziala_jak_dotad() {
    let cmd = komenda(r#"{"type":"command","reqId":1,"cmd":"simulateMessage","text":"CLOSE ALL"}"#);
    match &cmd {
        Command::SimulateMessage {
            text,
            channel_id,
            topic_id,
            msg_id,
            reply_to,
            edit_of,
        } => {
            assert_eq!(text, "CLOSE ALL");
            assert!(
                channel_id.is_none(),
                "brak `channelId` = brak, nie zero z panelu"
            );
            assert!(topic_id.is_none());
            assert!(msg_id.is_none());
            assert!(reply_to.is_none());
            assert!(edit_of.is_none());
        }
        inny => panic!("zły wariant: {inny:?}"),
    }

    let im = wstrzykniecie::wiadomosc(&cmd, T0, wstrzykniecie::ZRODLO_PANELU).unwrap();
    assert_eq!(im.source, SourceKey::new(wstrzykniecie::KANAL_PANELU, None));
    assert_eq!(im.source_name, "PANEL");
    assert!(im.reply_to.is_none());
    assert!(im.edit_of.is_none());
    assert!(
        wstrzykniecie::numer_wstrzykniety(im.msg_id),
        "numer nadany automatycznie musi zostać w zakresie panelu, jest {}",
        im.msg_id
    );
}

/// …i taka wiadomość zakłada koszyk tak samo, jak zakładała przed zmianą.
#[test]
fn kontrakt_zera_wstrzykniecie_zaklada_koszyk() {
    let (mut e, mut b) = stanowisko(4008.0);
    let json = format!(
        r#"{{"type":"command","reqId":1,"cmd":"simulateMessage","text":{}}}"#,
        serde_json::to_string(WEJSCIE).unwrap()
    );
    e.on_message(&mut b, &wiadomosc(&json, T0));

    assert_eq!(e.baskets.len(), 1, "sam tekst ma założyć koszyk, jak dotąd");
    let bk = koszyk(&e, &SourceKey::new(wstrzykniecie::KANAL_PANELU, None));
    assert_eq!((bk.zone_lo, bk.zone_hi), (4000.0, 4005.0));
    assert_eq!(b.pendings().len(), 3, "trzy szczeble siatki");
}

// ============================================================
//  2. MOST — pola z panelu DOJEŻDŻAJĄ do wiadomości
// ============================================================

/// SEDNO F5. Wszystkie cztery pola naraz, po drucie, w camelCase.
///
/// Ten test jest tanią polisą na najdroższy błąd tej klasy: pole widoczne
/// w panelu i w protokole, którego silnik nigdy nie zobaczył.
#[test]
fn wszystkie_pola_dojezdzaja_z_drutu_do_wiadomosci() {
    let im = wiadomosc(
        r#"{"type":"command","reqId":9,"cmd":"simulateMessage","text":"TP1 HIT",
            "channelId":-1000000000301,"topicId":77,"msgId":4321,"replyTo":1000,"editOf":4321}"#,
        T0,
    );
    assert_eq!(im.text, "TP1 HIT");
    assert_eq!(
        im.source,
        SourceKey::new(CZAT, Some(77)),
        "kanał ORAZ temat forum"
    );
    assert_eq!(im.msg_id, 4321, "podany numer jest brany dosłownie");
    assert_eq!(im.reply_to, Some(1000));
    assert_eq!(im.edit_of, Some(4321));
}

/// Nazwy po drucie są w camelCase. Gdyby serwer oczekiwał `msg_id`, panel
/// wysyłałby `msgId`, serde wstawiłby `None` — i wszystko wyglądałoby na
/// działające, tylko edycja nigdy by nie doszła.
#[test]
fn nazwy_pol_ida_w_camelcase() {
    let cmd = Command::SimulateMessage {
        text: "x".into(),
        channel_id: Some(-1),
        topic_id: Some(2),
        msg_id: Some(3),
        reply_to: Some(4),
        edit_of: Some(5),
    };
    let j = serde_json::to_value(&cmd).unwrap();
    for k in ["channelId", "topicId", "msgId", "replyTo", "editOf"] {
        assert!(j.get(k).is_some(), "pole `{k}` nie wyszło po drucie: {j}");
    }
    assert_eq!(j["cmd"], "simulateMessage");
}

// ============================================================
//  3. EDYCJA WSTRZYKNIĘTA
// ============================================================

#[test]
fn edycja_wstrzyknieta_zmienia_koszyk_zamiast_tworzyc_nowy() {
    let (mut e, mut b) = stanowisko(4008.0);
    let zr = SourceKey::new(CZAT, None);

    let wejscie = format!(
        r#"{{"type":"command","reqId":1,"cmd":"simulateMessage","text":{},"channelId":{CZAT},"msgId":7000}}"#,
        serde_json::to_string(WEJSCIE).unwrap()
    );
    e.on_message(&mut b, &wiadomosc(&wejscie, T0));
    assert_eq!(e.baskets.len(), 1);
    assert_eq!(
        (koszyk(&e, &zr).zone_lo, koszyk(&e, &zr).zone_hi),
        (4000.0, 4005.0)
    );

    let edycja = format!(
        r#"{{"type":"command","reqId":2,"cmd":"simulateMessage","text":{},"channelId":{CZAT},"msgId":7000,"editOf":7000}}"#,
        serde_json::to_string(WEJSCIE_PRZESUNIETE).unwrap()
    );
    e.on_message(&mut b, &wiadomosc(&edycja, T0 + 1_000));

    assert_eq!(
        e.baskets.len(),
        1,
        "edycja NIE jest nową wiadomością — drugi koszyk znaczy, że `editOf` \
         nie dojechało do silnika"
    );
    let bk = koszyk(&e, &zr);
    assert_eq!(
        (bk.zone_lo, bk.zone_hi),
        (3996.0, 4001.0),
        "strefa po edycji"
    );
    assert_eq!(bk.sl, Some(3991.0), "stop po edycji");
}

/// Kontrola negatywna do testu wyżej: bez `editOf` (a więc pod nowym numerem)
/// ta sama treść zakłada DRUGI koszyk. Gdyby jej nie było, test edycji
/// przechodziłby także wtedy, gdyby silnik po prostu ignorował powtórki.
#[test]
fn bez_pola_editof_powstaje_drugi_koszyk() {
    let (mut e, mut b) = stanowisko(4008.0);
    let wejscie = format!(
        r#"{{"type":"command","reqId":1,"cmd":"simulateMessage","text":{},"channelId":{CZAT},"msgId":7000}}"#,
        serde_json::to_string(WEJSCIE).unwrap()
    );
    e.on_message(&mut b, &wiadomosc(&wejscie, T0));

    let druga = format!(
        r#"{{"type":"command","reqId":2,"cmd":"simulateMessage","text":{},"channelId":{CZAT},"msgId":7001}}"#,
        serde_json::to_string(WEJSCIE_PRZESUNIETE).unwrap()
    );
    e.on_message(&mut b, &wiadomosc(&druga, T0 + 1_000));

    assert_eq!(e.baskets.len(), 2, "nowy numer bez `editOf` to nowy sygnał");
}

// ============================================================
//  4. ODPOWIEDŹ WSTRZYKNIĘTA
// ============================================================

/// Odpowiedź ma trafić do koszyka wiadomości, NA KTÓRĄ odpowiada — a nie do
/// najświeższego koszyka kanału.
///
/// Dwa koszyki w TYM SAMYM źródle, więc rozstrzyga wyłącznie `replyTo`.
#[test]
fn odpowiedz_wstrzyknieta_trafia_do_wskazanego_koszyka() {
    let (mut e, mut b) = stanowisko(4008.0);

    for (numer, tekst) in [(100, WEJSCIE), (200, WEJSCIE_NIZEJ)] {
        let j = format!(
            r#"{{"type":"command","reqId":1,"cmd":"simulateMessage","text":{},"channelId":{CZAT},"msgId":{numer}}}"#,
            serde_json::to_string(tekst).unwrap()
        );
        e.on_message(&mut b, &wiadomosc(&j, T0));
    }
    assert_eq!(
        e.baskets.len(),
        2,
        "przesłanka: dwa koszyki w jednym kanale"
    );
    let starszy = koszyk(&e, &SourceKey::new(CZAT, None)).id;

    // odpowiedź NA WIADOMOŚĆ 100 — czyli na ten starszy koszyk
    e.on_message(
        &mut b,
        &wiadomosc(
            &format!(
                r#"{{"type":"command","reqId":2,"cmd":"simulateMessage","text":"❌ SL HIT -100 PIPS","channelId":{CZAT},"msgId":900,"replyTo":100}}"#
            ),
            T0 + 1_000,
        ),
    );

    let zamkniete: Vec<u32> = e
        .baskets
        .iter()
        .filter(|x| x.state == BasketState::Done)
        .map(|x| x.id)
        .collect();
    assert_eq!(
        zamkniete,
        vec![starszy],
        "„SL HIT\" w odpowiedzi na wiadomość 100 ma zamknąć DOKŁADNIE koszyk \
         wiadomości 100 — inaczej `replyTo` nie dojechało"
    );
}

// ============================================================
//  5. NUMERACJA — wstrzyknięcie nie zderza się z Telegramem
// ============================================================

/// Kolizję MIĘDZY KANAŁAMI rozstrzyga `SourceKey` i pilnuje tego
/// `kolizje_msgid_wielokanalowe.rs`. Tutaj sprawdzamy tylko, że wstrzyknięcie
/// z panelu **umie** wskazać kanał — czyli że kontrakt „klucz zawiera źródło"
/// nie został po drodze spłaszczony do samego numeru.
#[test]
fn ten_sam_numer_w_dwoch_kanalach_to_dwa_zrodla() {
    let a = wiadomosc(
        &format!(
            r#"{{"type":"command","reqId":1,"cmd":"simulateMessage","text":"x","channelId":{CZAT},"msgId":1}}"#
        ),
        T0,
    );
    let z_b = wiadomosc(
        &format!(
            r#"{{"type":"command","reqId":2,"cmd":"simulateMessage","text":"x","channelId":{CZAT_DRUGI},"msgId":1}}"#
        ),
        T0,
    );
    assert_eq!(a.msg_id, z_b.msg_id, "numery celowo takie same");
    assert_ne!(
        a.source, z_b.source,
        "…a źródła różne — na tym stoi cały dedup"
    );
}

/// Automatyczny numer nie ma prawa wejść w przestrzeń Telegrama (dodatnią)
/// ani powtórzyć się przy szybkim wstrzykiwaniu.
///
/// REGRESJA: reguła sprzed F5 brzmiała `-(ts % 1e9)` na milisekundach.
/// Skrypt chaosu wysyłający dwie wiadomości w tej samej milisekundzie dostawał
/// ten sam numer, a `entry_idempotencja` (domyślnie włączona) po cichu zjadała
/// drugą — bez wpisu w logu.
#[test]
fn automatyczne_numery_sa_ujemne_i_nigdy_sie_nie_powtarzaja() {
    assert!(
        Settings::default().entry_idempotencja,
        "właśnie dlatego powtórzony numer jest groźny: silnik zjada duplikat"
    );
    let mut widziane = std::collections::HashSet::new();
    for _ in 0..5_000 {
        // ten sam `ts` dla wszystkich — tak wygląda wstrzykiwanie ze skryptu
        let n = wiadomosc(
            r#"{"type":"command","reqId":1,"cmd":"simulateMessage","text":"x"}"#,
            T0,
        )
        .msg_id;
        assert!(n < 0, "numer {n} wszedł w przestrzeń numerów Telegrama");
        assert!(widziane.insert(n), "numer {n} wydany dwa razy");
    }
}

/// I dowód, że to nie jest teoria: dwa wstrzyknięcia „samego tekstu" pod tym
/// samym znacznikiem czasu zakładają DWA koszyki, a nie jeden.
///
/// Test jest SAMODOWODZĄCY: najpierw pokazuje, co robi silnik, gdy numer się
/// powtórzy (drugi sygnał ginie — to była stara reguła `-(ts % 1e9)`), a
/// dopiero potem, że automatyczna numeracja tego nie robi. Bez pierwszej
/// połowy nie byłoby wiadomo, czy druga cokolwiek mierzy.
#[test]
fn dwa_szybkie_wstrzykniecia_daja_dwa_koszyki() {
    let tresc = serde_json::to_string(WEJSCIE).unwrap();

    // ---------- tak wyglądała AWARIA: numer powtórzony ----------
    let (mut e, mut b) = stanowisko(4008.0);
    let powtorzony = format!(
        r#"{{"type":"command","reqId":1,"cmd":"simulateMessage","text":{tresc},"msgId":55}}"#
    );
    e.on_message(&mut b, &wiadomosc(&powtorzony, T0));
    e.on_message(&mut b, &wiadomosc(&powtorzony, T0));
    assert_eq!(
        e.baskets.len(),
        1,
        "przesłanka: ten sam numer w tym samym źródle silnik zjada jako \
         powtórkę — i to jest cena za kolizję numerów"
    );

    // ---------- a tak wygląda teraz: numer nadawany automatycznie ----------
    let (mut e, mut b) = stanowisko(4008.0);
    let json = format!(r#"{{"type":"command","reqId":1,"cmd":"simulateMessage","text":{tresc}}}"#);
    e.on_message(&mut b, &wiadomosc(&json, T0));
    e.on_message(&mut b, &wiadomosc(&json, T0));
    assert_eq!(
        e.baskets.len(),
        2,
        "drugie wstrzyknięcie zginęło w idempotencji wejścia — numery się \
         powtórzyły"
    );
}
