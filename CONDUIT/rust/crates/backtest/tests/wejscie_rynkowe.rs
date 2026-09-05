
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

/// Sygnał RYNKOWY (bez słowa LIMIT). Strefa 4000–4005, SL 3990.
const SYGNAL: &str = "BUY GOLD @ 4005/4000\nTP 4010\nTP 4020\nTP 4030\nSL 3990";

/// Σ |cena wejścia − SL| × 100 × wolumen po wszystkim, co koszyk trzyma.
fn ryzyko_koszyka(b: &SimBroker) -> f64 {
    b.pendings()
        .iter()
        .filter_map(|o| o.sl.map(|s| (o.price - s).abs() * 100.0 * o.volume))
        .sum::<f64>()
        + b.positions()
            .iter()
            .filter_map(|p| {
                p.sl.or(p.vsl)
                    .map(|s| (p.open_price - s).abs() * 100.0 * p.volume)
            })
            .sum::<f64>()
}

fn stanowisko_rynkowe(mut cfg: Settings, bid: f64) -> (Engine, SimBroker) {
    cfg.auto_limit = false;
    cfg.tp_schedule = TpSchedule::AllRunners;
    stanowisko(cfg, bid)
}

// ============================================================
//  BŁĄD 1 — cap liczony od ceny szczebla zamiast od ceny wypełnienia
// ============================================================

#[test]
fn cap_ryzyka_liczy_wejscie_rynkowe_od_ceny_wypelnienia() {
    // Plan wycenia ryzyko przy cenach 4000–4005, a wszystkie jednostki wchodzą
    // po ~4012. Dystans do SL 3990 jest wtedy o ponad połowę większy, niż
    // zakładał plan — i o tyle właśnie ryzyko przekraczało cap.
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.lot_fixed = 0.10;
    cfg.lot_min = 0.01;
    cfg.risk_per_basket_pct = 3.0; // 3 % z 1000 $ = 30 $

    let (mut e, mut b) = stanowisko_rynkowe(cfg, 4012.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(!b.positions().is_empty(), "wejście rynkowe musi się odbyć");

    let ryzyko = ryzyko_koszyka(&b);
    assert!(
        ryzyko <= 30.0 + 1e-6,
        "ryzyko po CENIE WYPEŁNIENIA {ryzyko:.2} $ przekracza cap 30 $ — to jest \
         dokładnie ta dziura, przez którą koszyk brał 15,5× własnego limitu"
    );
}

#[test]
fn cap_rynkowy_tnie_liczbe_jednostek_gdy_lot_lezy_na_podlodze() {
    // Przy locie równym podłodze brokera skalowanie wolumenu nie ma czego
    // ściąć — jedynym dławikiem zostaje LICZBA jednostek. To nie jest
    // przypadek teoretyczny: 0,5 % kapitału z 200 $ to 0,01 lota, czyli
    // dokładnie minimum, i tak gra konto produkcyjne.
    let mut ciasno = Settings::default();
    ciasno.entry_units = 5;
    ciasno.lot_fixed = 0.01;
    ciasno.lot_min = 0.01;
    ciasno.grid_anchor_absolute = true; // syntetyczna siatka: 5 poziomów × 5 sztuk
    ciasno.risk_per_basket_pct = 1.0; // 10 $ na koncie 1000 $

    let mut luzno = ciasno.clone();
    luzno.risk_per_basket_pct = 50.0;

    let (mut e1, mut b1) = stanowisko_rynkowe(ciasno, 4012.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, SYGNAL));
    let (mut e2, mut b2) = stanowisko_rynkowe(luzno, 4012.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));

    assert!(
        b1.positions().len() < b2.positions().len(),
        "ciasny cap musi otworzyć MNIEJ pozycji ({} vs {})",
        b1.positions().len(),
        b2.positions().len()
    );
    assert!(
        ryzyko_koszyka(&b1) <= 10.0 + 1e-6,
        "ryzyko {:.2} $ ponad cap 10 $",
        ryzyko_koszyka(&b1)
    );
}

#[test]
fn cap_rynkowy_nie_dotyka_sciezki_limitowej() {
    // BRAMKA REGRESJI CAŁEJ NAPRAWY. Każdy wydany preset ma `auto_limit = true`;
    // jeśli tu drgnie choć jedna liczba, naprawa przestaje być no-opem.
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.lot_fixed = 0.10;
    cfg.risk_per_basket_pct = 0.0;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(b.pendings().len(), 4, "siatka limitów bez zmian");
    for o in b.pendings() {
        assert!(
            (o.volume - 0.10).abs() < 1e-9,
            "wolumen nietknięty: {}",
            o.volume
        );
    }
}

// ============================================================
//  BŁĄD 3 — `market_entry_step` nie dotyczył pierwszego wejścia
// ============================================================

#[test]
fn domyslny_tryb_rynkowy_to_dokladnie_dawne_zachowanie() {
    assert_eq!(
        Settings::default().market_entry_mode,
        MarketEntryMode::GridAtOnce
    );
}

#[test]
fn tryb_single_wchodzi_jedna_pozycja_o_lacznym_rozmiarze() {
    let mut baza = Settings::default();
    baza.entry_units = 4;
    baza.lot_fixed = 0.02;
    baza.risk_per_basket_pct = 0.0; // bez capu — mierzymy SAM rozkład

    let (mut e1, mut b1) = stanowisko_rynkowe(baza.clone(), 4012.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, SYGNAL));
    let siatka: f64 = b1.positions().iter().map(|p| p.volume).sum();
    assert!(b1.positions().len() > 1, "GridAtOnce otwiera wiele pozycji");

    let mut single = baza;
    single.market_entry_mode = MarketEntryMode::Single;
    let (mut e2, mut b2) = stanowisko_rynkowe(single, 4012.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));

    assert_eq!(
        b2.positions().len(),
        1,
        "Single otwiera DOKŁADNIE jedną pozycję"
    );
    let jedna = b2.positions()[0].volume;
    assert!(
        (jedna - siatka).abs() < 1e-6,
        "łączny rozmiar musi zostać zachowany: {jedna} wobec {siatka}"
    );
}

#[test]
fn tryb_laddered_uwalnia_jednostki_co_krok_ceny() {
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.lot_fixed = 0.02;
    cfg.risk_per_basket_pct = 0.0;
    cfg.market_entry_mode = MarketEntryMode::Laddered;
    cfg.market_entry_step = 1.0;

    let (mut e, mut b) = stanowisko_rynkowe(cfg, 4012.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let po_pierwszym = b.positions().len();
    assert!(po_pierwszym > 0, "pierwszy szczebel wchodzi od razu");

    // pół kroku to za mało
    tik(&mut e, &mut b, T0 + 1_000, 4011.5);
    assert_eq!(
        b.positions().len(),
        po_pierwszym,
        "0,5 $ nie uwalnia szczebla"
    );

    // pełny krok NA KORZYŚĆ WEJŚCIA (dla kupna: w dół) uwalnia kolejny
    tik(&mut e, &mut b, T0 + 2_000, 4010.5);
    let po_drugim = b.positions().len();
    assert!(
        po_drugim > po_pierwszym,
        "krok 1,0 $ musi uwolnić kolejny szczebel"
    );

    // sam upływ czasu nie dokłada niczego
    tik(&mut e, &mut b, T0 + 3_000, 4010.5);
    assert_eq!(
        b.positions().len(),
        po_drugim,
        "bez ruchu ceny nie dokładamy"
    );

    // drabina wchodzi ŁAGODNIEJ niż jednorazowy wystrzał całego planu
    let mut naraz = Settings::default();
    naraz.entry_units = 4;
    naraz.lot_fixed = 0.02;
    naraz.risk_per_basket_pct = 0.0;
    let (mut e2, mut b2) = stanowisko_rynkowe(naraz, 4012.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        po_pierwszym < b2.positions().len(),
        "pierwszy tick drabiny ma otworzyć mniej niż GridAtOnce ({po_pierwszym} vs {})",
        b2.positions().len()
    );
}

// ============================================================
//  BŁĄD 2 — odwrócony znak `max_chase_beyond_zone`
// ============================================================

#[test]
fn gonienie_liczone_od_gorszej_krawedzi_zatrzymuje_wejscie_rynkowe() {
    // Strefa 4000–4005, gorsza krawędź dla kupna to 4005. Cena 4012 leży 7 $
    // za nią — klasyczne gonienie, którego stary znak w ogóle nie widział.
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.max_chase_beyond_zone = 3.0;

    let (mut e, mut b) = stanowisko_rynkowe(cfg.clone(), 4012.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        b.positions().is_empty(),
        "gonienie 7 $ ponad próg 3 $ musi zostać odrzucone"
    );
    assert!(e.baskets.is_empty(), "koszyk nie ma prawa powstać");

    // cena tuż za strefą mieści się w progu i wchodzi normalnie
    let (mut e2, mut b2) = stanowisko_rynkowe(cfg, 4006.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        !b2.positions().is_empty(),
        "1 $ za strefą jest w granicach progu"
    );
}

#[test]
fn gonienie_nie_dotyka_sciezki_limitowej_ani_domyslnego_progu() {
    // Dla sygnału obsługiwanego limitami cena leży za gorszą krawędzią
    // Z DEFINICJI, więc po naprawie znaku próg działałby tam jak wyłącznik
    // całego handlu. Straż jest świadomie zawężona do wejść rynkowych.
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.max_chase_beyond_zone = 3.0;
    let (mut e, mut b) = stanowisko(cfg, 4012.0); // auto_limit = true (domyślnie)
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        !b.pendings().is_empty(),
        "siatka limitów wchodzi mimo progu gonienia"
    );

    // domyślna 0,0 wyłącza straż w całości — dlatego naprawa znaku nie rusza
    // ani jednej liczby żadnego wydanego presetu
    assert_eq!(Settings::default().max_chase_beyond_zone, 0.0);
    let mut luz = Settings::default();
    luz.entry_units = 2;
    let (mut e2, mut b2) = stanowisko_rynkowe(luz, 4012.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        !b2.positions().is_empty(),
        "przy domyślnym progu nic nie jest blokowane"
    );
}

// ============================================================
//  OSTATNIA DZIURA — dokładki po trafionym celu (`reentry_pass`)
// ============================================================

/// Konfiguracja dokładająca po TP1, wzorowana na ULTRA-X5: `reenter_max = 0`
/// NIE znaczy „wyłącz", tylko „bez limitu".
fn cfg_dokladki(cap_pct: f64) -> Settings {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.lot_fixed = 0.10;
    cfg.lot_min = 0.01;
    cfg.reenter_after_tp = true;
    cfg.reenter_min_tp_stage = 1;
    cfg.reenter_max = 0; // bez limitu LICZBY dokładek — jak X5
    cfg.market_entry_step = 1.0;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.assign_tp_per_position = true;
    cfg.risk_per_basket_pct = cap_pct;
    cfg.reenter_respect_cap = true;
    cfg
}

/// Przepuszcza koszyk przez TP1, a potem kilkakrotnie sprowadza cenę do strefy
/// krokami co 1 $ — czyli robi dokładnie to, na czym żyje `reenter_max = 0`.
fn ile_pozycji_po_dokladkach(cfg: Settings) -> usize {
    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 4010.5); // TP1 → etap 1
    let mut t = T0 + 2_000;
    for krok in 0..6 {
        // powrót do strefy coraz głębiej: 4004, 4003, 4002 …
        tik(&mut e, &mut b, t, 4004.0 - krok as f64);
        t += 1_000;
    }
    b.positions().len()
}

#[test]
fn dokladki_pytaja_o_budzet_ryzyka_koszyka() {
    let bez_capu = ile_pozycji_po_dokladkach(cfg_dokladki(0.0));
    let z_capem = ile_pozycji_po_dokladkach(cfg_dokladki(3.0)); // 30 $ z 1000 $

    assert!(bez_capu > 0, "test wymaga, żeby dokładki w ogóle wchodziły");
    assert!(
        z_capem < bez_capu,
        "limit ryzyka musi ograniczyć dokładki ({z_capem} wobec {bez_capu} bez limitu)"
    );
}

#[test]
fn dokladki_nie_przekraczaja_capu_koszyka() {
    let cfg = cfg_dokladki(3.0); // 30 $ na koncie 1000 $
    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 4010.5);
    let mut t = T0 + 2_000;
    for krok in 0..6 {
        tik(&mut e, &mut b, t, 4004.0 - krok as f64);
        t += 1_000;
        // po KAŻDYM kroku ryzyko koszyka musi mieścić się w limicie —
        // sprawdzanie tylko na końcu przegapiłoby chwilowe przekroczenie
        let r = ryzyko_koszyka(&b);
        assert!(
            r <= 30.0 + 1e-6,
            "po kroku {krok} ryzyko {r:.2} $ ponad cap 30 $"
        );
    }
}

#[test]
fn bez_przelacznika_dokladki_ida_jak_dawniej() {
    let mut cfg = cfg_dokladki(3.0);
    cfg.reenter_respect_cap = false;
    let bez_przelacznika = ile_pozycji_po_dokladkach(cfg);
    let bez_capu = ile_pozycji_po_dokladkach(cfg_dokladki(0.0));
    assert_eq!(
        bez_przelacznika, bez_capu,
        "wyłączony przełącznik musi dawać dokładnie zachowanie sprzed reguły"
    );
}

#[test]
fn bez_limitu_ryzyka_dokladki_zachowuja_sie_jak_dotad() {
    // `risk_per_basket_pct = 0` to „bez limitu" i tak ma zostać: cała naprawa
    // jest wtedy no-opem, bo `market_risk_cap` zwraca `None`.
    let mut cfg = cfg_dokladki(0.0);
    cfg.lot_fixed = 0.10;
    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 4010.5);
    tik(&mut e, &mut b, T0 + 2_000, 4003.0);
    let dokladki: Vec<f64> = b
        .positions()
        .iter()
        .filter(|p| p.level == -2)
        .map(|p| p.volume)
        .collect();
    assert!(!dokladki.is_empty(), "dokładka musi wejść");
    for v in dokladki {
        assert!((v - 0.10).abs() < 1e-9, "wolumen dokładki nietknięty: {v}");
    }
}
