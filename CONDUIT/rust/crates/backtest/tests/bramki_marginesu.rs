
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_786_015_440_000;

fn zrodlo() -> SourceKey {
    SourceKey::new(-1_000_000_000_101, Some(9))
}

fn kwotowanie(ts: Ts, bid: f64) -> Quote {
    Quote {
        ts,
        bid,
        ask: bid + 0.30,
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

/// Sygnał wejścia RYNKOWEGO ze strefą — dziesięć jednostek wchodzi w jednym
/// ticku, więc margines skacze natychmiast i widać go w tym samym przebiegu.
const SYGNAL: &str = "BUY GOLD @ 4005/4000\nTP 4010\nTP 4020\nTP 4030\nSL 3990";
const SYGNAL2: &str = "BUY GOLD @ 4006/4001\nTP 4011\nTP 4021\nTP 4031\nSL 3991";

/// Konfiguracja, w której 10 jednostek wchodzi po rynku od razu.
fn baza() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 10;
    c.auto_limit = false;
    c.market_entry_mode = MarketEntryMode::GridAtOnce;
    c.grid_anchor_absolute = false;
    c.ppm_enabled = false;
    c.max_open_positions = 0;
    c.max_open_baskets = 0;
    c.lot_mode_percent = false;
    c.lot_fixed = 0.10;
    c.lot_min = 0.10;
    c.lot_max = 0.10;
    c.risk_per_basket_pct = 0.0;
    c.session_filter = false;
    c.regime_filter = RegimeFilter::Off;
    c.trend_filter_enabled = false;
    c.stops_level = 0.2;
    // Bramka brokerska stoi na 50 %, żeby nie mieszała się z mierzoną osią.
    c.margin_call_level_pct = 50.0;
    c
}

/// Rachunek celowo MAŁY wobec lota: 10 × 0,10 lota = 1,0 lota po ~4000 $
/// przy dźwigni 500 to 800 $ marginesu. Przy equity 2000 $ daje to poziom
/// marginesu 250 % — czyli progi 300 % i 100 % leżą po dwóch stronach.
fn stanowisko(cfg: Settings) -> (Engine, SimBroker) {
    let stops = cfg.stops_level;
    let mut b = SimBroker::new(2000.0, stops, 0.0);
    b.on_quote(kwotowanie(T0, 4002.0));
    let e = Engine::new(cfg, 2000.0);
    (e, b)
}

fn poziom(b: &SimBroker) -> f64 {
    b.margin_level_pct()
}

// ============================================================
//  PARYTET — próg 0 nie zmienia NICZEGO
// ============================================================

#[test]
fn prog_zero_nie_zmienia_nic() {
    let c = Settings::default();
    assert_eq!(c.ml_min_wejscie, 0.0);
    assert_eq!(c.ml_min_warstwa, 0.0);
    assert_eq!(c.ml_min_reentry, 0.0);
    assert_eq!(c.ml_min_rearm, 0.0);
    assert_eq!(c.ml_min_piramida, 0.0);
    assert_eq!(c.ml_min_fast_addon, 0.0);
    assert_eq!(c.ml_min_relot_up, 0.0);
    assert_eq!(c.ml_min_drabina, 0.0);
    assert_eq!(c.konto_dzwignia, 0.0);
    assert!(!c.ml_licz_wiszace);

    // …i że przebieg z nimi na zerze daje ten sam stan co przebieg
    // konfiguracją, która o nich nie wie (bo `Default` to ta sama struktura).
    let (mut e, mut b) = stanowisko(baza());
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(!b.positions().is_empty(), "baza musi w ogóle handlować");
    let ile = b.positions().len();
    let wol: f64 = b.positions().iter().map(|p| p.volume).sum();

    let (mut e2, mut b2) = stanowisko(baza());
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(b2.positions().len(), ile);
    assert!((b2.positions().iter().map(|p| p.volume).sum::<f64>() - wol).abs() < 1e-12);
}

// ============================================================
//  BRAMKA WEJŚCIA
// ============================================================

/// Próg WYŻSZY od bieżącego poziomu marginesu musi zatrzymać DRUGI koszyk,
/// a pierwszy przepuścić — bo przed pierwszym ekspozycja jest zerowa,
/// czyli poziom nieskończony.
#[test]
fn wejscie_blokowane_dopiero_gdy_jest_co_mierzyc() {
    let mut c = baza();
    c.ml_min_wejscie = 300.0;
    let (mut e, mut b) = stanowisko(c);

    // pierwszy koszyk: margines zero → poziom nieskończony → przechodzi
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let po_pierwszym = b.positions().len();
    assert!(
        po_pierwszym > 0,
        "pierwszy koszyk musi wejść — nie ma jeszcze ekspozycji"
    );
    let ml = poziom(&b);
    assert!(
        ml < 300.0,
        "test bez sensu, jeśli poziom po pierwszym koszyku nie zszedł pod próg (jest {ml:.0} %)"
    );

    // drugi koszyk: poziom już pod progiem → bramka zamyka
    e.on_message(&mut b, &wiadomosc(T0 + 60_000, 2, SYGNAL2));
    assert_eq!(
        b.positions().len(),
        po_pierwszym,
        "drugi koszyk wszedł mimo poziomu marginesu {ml:.0} % poniżej progu 300 %"
    );
}

/// Ten sam przebieg z progiem zerowym MUSI wpuścić drugi koszyk — inaczej
/// test wyżej mierzyłby cokolwiek innego niż nową oś.
#[test]
fn bez_progu_drugi_koszyk_wchodzi() {
    let (mut e, mut b) = stanowisko(baza());
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let po_pierwszym = b.positions().len();
    e.on_message(&mut b, &wiadomosc(T0 + 60_000, 2, SYGNAL2));
    assert!(
        b.positions().len() > po_pierwszym,
        "bez progu drugi koszyk powinien dołożyć pozycje"
    );
}

// ============================================================
//  BRAMKA WARSTWY — przerywa siatkę W POŁOWIE
// ============================================================

/// To jest ta bramka, której dotąd nie było wcale: plan siatki powstawał raz
/// i był rozstawiany do końca bez patrzenia na rachunek.
#[test]
fn warstwa_ucina_siatke_w_polowie() {
    let mut c = baza();
    // Próg dobrany tak, żeby zadziałał PO kilku jednostkach, a nie przed
    // pierwszą: 10 jednostek daje ~250 %, więc 400 % utnie po drodze.
    c.ml_min_warstwa = 400.0;
    let (mut e, mut b) = stanowisko(c);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));

    let ile = b.positions().len();
    assert!(
        ile > 0,
        "pierwsza warstwa musi wejść — przed nią ekspozycja jest zerowa"
    );
    assert!(
        ile < 10,
        "siatka rozstawiła się w całości ({ile} jednostek) mimo progu warstwy 400 %"
    );
    assert!(
        poziom(&b) >= 400.0 || ile < 10,
        "po ucięciu poziom marginesu powinien zostać nad progiem albo siatka krótsza"
    );
}

#[test]
fn bez_progu_siatka_rozstawia_sie_cala() {
    let (mut e, mut b) = stanowisko(baza());
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(
        b.positions().len(),
        10,
        "bez progu wszystkie dziesięć jednostek ma wejść — inaczej test wyżej nie mierzy osi"
    );
}

// ============================================================
//  LICZENIE WISZĄCYCH — najgorszy scenariusz zamiast bieżącego
// ============================================================

/// `ml_licz_wiszace` ma odpowiadać na pytanie „co, jeśli wszystko, co leży,
/// się wypełni". Przy wejściach LIMITOWYCH bramka bez tej flagi widzi ZERO
/// ekspozycji, bo pozycji jeszcze nie ma — i przepuszcza kolejny koszyk.
#[test]
fn wiszace_licza_sie_dopiero_z_flaga() {
    // wejście limitowe: zlecenia leżą, pozycji nie ma
    let mut c = baza();
    c.auto_limit = true;
    c.ml_min_wejscie = 300.0;
    let (mut e, mut b) = stanowisko(c.clone());
    // cena ponad strefą, żeby limity zostały limitami
    let q = kwotowanie(T0, 4020.0);
    b.on_quote(q);
    e.on_tick(&mut b, &q);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        b.positions().is_empty(),
        "to miały być zlecenia, nie pozycje"
    );
    let wiszace = b.pendings().len();
    assert!(wiszace > 0, "brak zleceń — test nie ma czego mierzyć");

    // BEZ flagi: margines pozycji = 0 → poziom nieskończony → drugi koszyk wchodzi
    e.on_message(&mut b, &wiadomosc(T0 + 60_000, 2, SYGNAL2));
    assert!(
        b.pendings().len() > wiszace,
        "bez `ml_licz_wiszace` drugi koszyk powinien dołożyć zlecenia (widzi zerową ekspozycję)"
    );

    // Z FLAGĄ: ten sam scenariusz, ale bramka liczy to, co się wypełni
    let mut c2 = c;
    c2.ml_licz_wiszace = true;
    let (mut e2, mut b2) = stanowisko(c2);
    let q2 = kwotowanie(T0, 4020.0);
    b2.on_quote(q2);
    e2.on_tick(&mut b2, &q2);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    let wiszace2 = b2.pendings().len();
    assert!(wiszace2 > 0);
    e2.on_message(&mut b2, &wiadomosc(T0 + 60_000, 2, SYGNAL2));
    assert_eq!(
        b2.pendings().len(),
        wiszace2,
        "z `ml_licz_wiszace` drugi koszyk NIE ma prawa wejść — leżące zlecenia \
         zabierają margines, którego bramka bez flagi nie widzi"
    );
}

// ============================================================
//  WYMUSZONA DŹWIGNIA — oś stresu
// ============================================================

/// `konto_dzwignia` nie jest naprawą (wzór zgadza się z MetaTraderem co do
/// centa przy 1:500), tylko osią pytania „co, gdyby broker ściął dźwignię".
/// Przy 1:100 ten sam wolumen zabiera pięć razy więcej marginesu.
#[test]
fn wymuszona_dzwignia_zaostrza_bramke() {
    let mut c = baza();
    c.ml_min_wejscie = 300.0;
    c.konto_dzwignia = 100.0;
    let (mut e, mut b) = stanowisko(c);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let po_pierwszym = b.positions().len();
    assert!(po_pierwszym > 0);

    // Przy 1:100 poziom po pierwszym koszyku jest ~5× niższy niż przy 1:500,
    // więc drugi koszyk musi odpaść tym pewniej.
    e.on_message(&mut b, &wiadomosc(T0 + 60_000, 2, SYGNAL2));
    assert_eq!(
        b.positions().len(),
        po_pierwszym,
        "przy wymuszonej dźwigni 1:100 drugi koszyk nie ma prawa wejść"
    );
}

// ============================================================
//  KONTO BEZ EQUITY — bramka ma się ZAMKNĄĆ, nie otworzyć
// ============================================================

/// Dzielenie przez zero w liczniku poziomu marginesu jest tą klasą błędu,
/// która przepuszcza wszystko dokładnie wtedy, gdy nie wolno przepuścić nic.
#[test]
fn zerowe_equity_zamyka_bramke() {
    let mut c = baza();
    c.ml_min_wejscie = 100.0;
    let stops = c.stops_level;
    let mut b = SimBroker::new(0.0, stops, 0.0);
    b.on_quote(kwotowanie(T0, 4002.0));
    let mut e = Engine::new(c, 0.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        b.positions().is_empty() && b.pendings().is_empty(),
        "rachunek bez equity nie ma poziomu marginesu, tylko problem — bramka \
         ma się zamknąć, a nie przepuścić wszystko"
    );
}
