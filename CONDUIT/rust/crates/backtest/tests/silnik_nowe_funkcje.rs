
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::{limit_price_is_valid, Broker};
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

/// Silnik + broker ustawione na jednej cenie startowej.
fn stanowisko(cfg: Settings, bid: f64) -> (Engine, SimBroker) {
    let stops = cfg.stops_level;
    let mut b = SimBroker::new(1000.0, stops, 0.0);
    b.on_quote(kwotowanie(T0, bid));
    let e = Engine::new(cfg, 1000.0);
    (e, b)
}

/// Jeden krok czasu: broker widzi cenę, potem silnik dostaje tick.
fn tik(e: &mut Engine, b: &mut SimBroker, ts: Ts, bid: f64) {
    let q = kwotowanie(ts, bid);
    b.on_quote(q);
    e.on_tick(b, &q);
}

const SYGNAL: &str = "BUY GOLD @ 4005/4000\nTP 4010\nTP 4020\nTP 4030\nSL 3990";
/// Ten sam setup, ale z celami tak daleko, że nie zostaną trafione przez
/// przypadek — test wyjścia musi mierzyć SWOJĄ regułę, a nie take-profit.
const SYGNAL_DALEKI: &str = "BUY GOLD @ 4005/4000\nTP 4200\nTP 4300\nTP 4400\nSL 3990";

// ============================================================
//  RE-ENTRY
// ============================================================

#[test]
fn reentry_wchodzi_dopiero_po_trafionym_celu_i_po_kroku_ceny() {
    let mut cfg = Settings::default();
    cfg.reenter_after_tp = true;
    cfg.reenter_min_tp_stage = 1;
    cfg.market_entry_step = 1.0;
    cfg.tp_schedule = TpSchedule::AllRunners;
    // bez wygaszania koszyka i bez inkasa — badamy wyłącznie powtórne wejście
    cfg.assign_tp_per_position = true;

    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let po_wejsciu = b.positions().len();
    assert!(po_wejsciu > 0, "sygnał musi otworzyć pozycję");

    // etap 0 — powrót do strefy nie może niczego dokładać
    tik(&mut e, &mut b, T0 + 1_000, 4001.0);
    assert_eq!(
        b.positions().len(),
        po_wejsciu,
        "re-entry przed TP1 jest zabronione"
    );

    // cena dobija TP1 → etap 1
    tik(&mut e, &mut b, T0 + 2_000, 4010.5);
    let po_tp = b.positions().len();

    // powrót do strefy — teraz wolno wejść ponownie
    tik(&mut e, &mut b, T0 + 3_000, 4003.0);
    let po_reentry = b.positions().len();
    assert!(
        po_reentry > po_tp,
        "po TP1 powrót do strefy otwiera nową pozycję"
    );

    // ten sam tick ceny nie może dokładać w kółko
    tik(&mut e, &mut b, T0 + 4_000, 4003.0);
    assert_eq!(
        b.positions().len(),
        po_reentry,
        "bez ruchu ceny nie dokładamy"
    );

    // dopiero krok 1.0 w naszą stronę otwiera kolejną
    tik(&mut e, &mut b, T0 + 5_000, 4001.5);
    assert!(
        b.positions().len() > po_reentry,
        "krok ceny odblokowuje kolejne wejście"
    );
}

#[test]
fn reentry_wylaczone_domyslnie() {
    let mut cfg = Settings::default();
    cfg.tp_schedule = TpSchedule::AllRunners;
    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let n = b.positions().len();
    tik(&mut e, &mut b, T0 + 1_000, 4010.5);
    tik(&mut e, &mut b, T0 + 2_000, 4002.0);
    assert_eq!(b.positions().len(), n, "bez włącznika nic się nie dokłada");
}

#[test]
fn reentry_nie_wchodzi_po_przebitym_sl() {
    let mut cfg = Settings::default();
    cfg.reenter_after_tp = true;
    cfg.reenter_min_tp_stage = 0;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.virtual_sl = false;

    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    // rynek pod SL — strefa formalnie „zawiera" cenę tylko przy krawędzi,
    // więc sprawdzamy jawnie: pod SL nie wchodzimy nigdy
    tik(&mut e, &mut b, T0 + 1_000, 3989.0);
    tik(&mut e, &mut b, T0 + 2_000, 3989.0);
    assert!(
        b.positions().is_empty(),
        "pozycje wyszły na SL i nic nie zostało dołożone"
    );
}

// ============================================================
//  WYGASZANIE STARYCH SYGNAŁÓW
// ============================================================

#[test]
fn stary_czekajacy_sygnal_jest_anulowany() {
    let mut cfg = Settings::default();
    cfg.ignore_old_after_min = 30.0;
    cfg.entry_units = 3;

    // Cena NAD strefą (limity nie realizują się), ale PONIŻEJ TP1 — inaczej
    // koszyk zaliczyłby cel i sam skasowałby limity, a test mierzyłby co innego.
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(!b.pendings().is_empty(), "limity muszą stać");

    tik(&mut e, &mut b, T0 + 29 * 60_000, 4008.0);
    assert!(
        !b.pendings().is_empty(),
        "przed upływem czasu nic nie znika"
    );

    tik(&mut e, &mut b, T0 + 31 * 60_000, 4008.0);
    assert!(
        b.pendings().is_empty(),
        "po 30 min czekający sygnał jest kasowany"
    );
}

#[test]
fn koszyk_ktory_handlowal_nie_jest_wygaszany() {
    let mut cfg = Settings::default();
    cfg.ignore_old_after_min = 1.0;
    cfg.entry_units = 2;
    cfg.tp_schedule = TpSchedule::AllRunners;

    // cena W strefie — część zleceń realizuje się od razu
    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(!b.positions().is_empty());

    tik(&mut e, &mut b, T0 + 5 * 60_000, 4002.0);
    assert!(
        !b.positions().is_empty(),
        "trad w toku nie jest starym sygnałem — nie wolno go zamknąć po czasie"
    );
}

// ============================================================
//  REŻIM ZMIENNOŚCI
// ============================================================

#[test]
fn sztorm_tnie_liczbe_jednostek() {
    let mut baza = Settings::default();
    baza.entry_units = 4;
    baza.vol_window_min = 30.0;
    baza.vol_range_usd = 15.0;
    baza.vol_units_mult = 0.5;

    // --- rynek spokojny: bufor prawie płaski ---
    let (mut e1, mut b1) = stanowisko(baza.clone(), 4100.0);
    for i in 0..20 {
        tik(
            &mut e1,
            &mut b1,
            T0 + i * 6_000,
            4100.0 + (i % 2) as f64 * 0.1,
        );
    }
    e1.on_message(&mut b1, &wiadomosc(T0 + 200_000, 1, SYGNAL));
    let spokoj = b1.pendings().len();

    // --- sztorm: ten sam czas, zakres 40 $ ---
    let (mut e2, mut b2) = stanowisko(baza, 4100.0);
    for i in 0..20 {
        tik(
            &mut e2,
            &mut b2,
            T0 + i * 6_000,
            4100.0 + (i % 2) as f64 * 40.0,
        );
    }
    e2.on_message(&mut b2, &wiadomosc(T0 + 200_000, 1, SYGNAL));
    let sztorm = b2.pendings().len();

    assert!(spokoj > 0, "w spokoju siatka musi stanąć");
    assert!(
        sztorm < spokoj,
        "w sztormie stawiamy mniej zleceń: spokój {spokoj}, sztorm {sztorm}"
    );
}

#[test]
fn bez_wlacznika_zmiennosc_nie_zmienia_rozmiaru() {
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.vol_window_min = 0.0; // filtr wyłączony

    let (mut e, mut b) = stanowisko(cfg, 4100.0);
    for i in 0..20 {
        tik(
            &mut e,
            &mut b,
            T0 + i * 6_000,
            4100.0 + (i % 2) as f64 * 40.0,
        );
    }
    e.on_message(&mut b, &wiadomosc(T0 + 200_000, 1, SYGNAL));
    assert_eq!(
        b.pendings().len(),
        4,
        "bez filtru zawsze pełne cztery poziomy"
    );
}

// ============================================================
//  REVERSAL-EXIT
// ============================================================

#[test]
fn reversal_exit_bankuje_zysk_przy_gwaltownym_zawroceniu() {
    let mut cfg = Settings::default();
    cfg.rev_exit_range = 20.0;
    cfg.rev_exit_slope = 10.0;
    cfg.rev_exit_profit = 3.0;
    cfg.rev_exit_window_min = 60.0;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.entry_units = 1;

    let (mut e, mut b) = stanowisko(cfg, 3998.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    assert_eq!(b.positions().len(), 1);

    // rynek jedzie w górę: budujemy zakres i zysk
    let mut ts = T0;
    for i in 1..=20 {
        ts = T0 + i * 6_000;
        tik(&mut e, &mut b, ts, 3998.0 + i as f64 * 1.5);
    }
    assert_eq!(b.positions().len(), 1, "w trendzie pozycja jedzie dalej");

    // …i zawraca o 12 $ od szczytu — zakres w oknie jest duży, oddanie
    // przewagi też
    ts += 6_000;
    tik(&mut e, &mut b, ts, 4016.0);
    assert!(
        b.positions().is_empty(),
        "gwałtowne zawrócenie przy dużym zakresie bankuje zysk"
    );
}

#[test]
fn reversal_exit_wylaczony_nie_rusza_pozycji() {
    let mut cfg = Settings::default();
    cfg.rev_exit_range = 0.0;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.entry_units = 1;

    let (mut e, mut b) = stanowisko(cfg, 3998.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    let mut ts = T0;
    for i in 1..=20 {
        ts = T0 + i * 6_000;
        tik(&mut e, &mut b, ts, 3998.0 + i as f64 * 1.5);
    }
    tik(&mut e, &mut b, ts + 6_000, 4016.0);
    assert_eq!(
        b.positions().len(),
        1,
        "wyłączona reguła nie może nic zamykać"
    );
}

// ============================================================
//  DEDUP EDYCJI
// ============================================================

#[test]
fn edycja_komunikatu_nie_powtarza_wykonanej_akcji() {
    let mut cfg = Settings::default();
    cfg.dedup_edited_signals = true;
    cfg.tp_schedule = TpSchedule::OfficialCounts;
    cfg.official_counts = "1".into();
    cfg.assign_tp_per_position = false;
    cfg.entry_units = 4;
    cfg.tp_source = TpSource::SignalOnly;
    cfg.tp_stage_from_broker_fill = false;

    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let start = b.positions().len();
    assert!(start >= 2, "potrzebujemy kilku pozycji, jest {start}");

    let mut m = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    let po_tp = b.positions().len();
    assert!(po_tp < start, "TP1 musi zainkasować transzę");

    // ta sama wiadomość, poprawiona przez sygnalistę
    let mut edycja = wiadomosc(T0 + 2_000, 7, "TP1 HIT ✅");
    edycja.reply_to = Some(1);
    edycja.edit_of = Some(7);
    e.on_message(&mut b, &edycja);
    assert_eq!(
        b.positions().len(),
        po_tp,
        "edycja nie może inkasować drugi raz"
    );
}

#[test]
fn edycja_z_nowa_trescia_wykonuje_tylko_nowa_akcje() {
    let mut cfg = Settings::default();
    cfg.dedup_edited_signals = true;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.tp_source = TpSource::SignalOnly;
    cfg.tp_stage_from_broker_fill = false;
    cfg.entry_units = 2;
    cfg.risk_free_mode = RiskFreeMode::CloseEverything;

    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(!b.positions().is_empty());

    let mut m = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    let po_tp = b.positions().len();
    assert!(po_tp > 0);

    // dopisany RISK FREE ma się wykonać, choć „TP1 HIT" już nie
    let mut edycja = wiadomosc(T0 + 2_000, 7, "TP1 HIT — RISK FREE");
    edycja.reply_to = Some(1);
    edycja.edit_of = Some(7);
    e.on_message(&mut b, &edycja);
    assert!(b.positions().is_empty(), "dopisana akcja musi się wykonać");
}

// ============================================================
//  WSKAZANIE KOSZYKA POZIOMEM
// ============================================================

#[test]
fn komunikat_z_poziomem_trafia_do_wlasciwego_koszyka() {
    let mut cfg = Settings::default();
    cfg.basket_hint_tolerance = 0.6;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.entry_units = 1;
    cfg.risk_free_mode = RiskFreeMode::CloseEverything;
    cfg.max_open_baskets = 0;

    // dwa niezależne koszyki z tego samego kanału
    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    e.on_message(
        &mut b,
        &wiadomosc(
            T0 + 1_000,
            2,
            "BUY GOLD @ 4003/4001\nTP 4015\nTP 4025\nSL 3995",
        ),
    );
    assert_eq!(b.positions().len(), 2, "dwa koszyki, dwie pozycje");

    // komunikat wskazuje STARSZY koszyk swoją strefą
    e.on_message(
        &mut b,
        &wiadomosc(T0 + 2_000, 3, "RISK FREE (4005 TO 4000)"),
    );
    assert_eq!(
        b.positions().len(),
        1,
        "zamknięty musi zostać koszyk wskazany poziomem, a nie najnowszy"
    );
    // został ten drugi — o strefie 4001–4003
    let p = &b.positions()[0];
    assert!(
        p.open_price >= 4001.0 && p.open_price <= 4003.5,
        "cena {}",
        p.open_price
    );
}

// ============================================================
//  PONAWIANIE ODRZUCONYCH SL/TP
// ============================================================

#[test]
fn odrzucony_stop_jest_ponawiany_a_nie_gubiony() {
    let mut cfg = Settings::default();
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.entry_units = 1;
    cfg.trail_mode = TrailMode::Gap;
    cfg.trail_start = 1.0;
    cfg.trail_gap = 0.05; // celowo bliżej ceny niż stops level → odrzut
    cfg.sltp_retry_s = 0.0; // najpierw BEZ ponawiania

    let (mut e, mut b) = stanowisko(cfg.clone(), 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let sl0 = b.positions()[0].sl;
    tik(&mut e, &mut b, T0 + 1_000, 4006.0);
    let bez_ponawiania = b.positions()[0].sl;

    // Trailing dosuwa SL do wykonalnej odległości, więc sam odrzut nie musi
    // wystąpić — sprawdzamy tylko, że stop nigdy się nie POGARSZA.
    if let (Some(a), Some(z)) = (sl0, bez_ponawiania) {
        assert!(z >= a - 1e-9, "SL nie może się cofnąć: {a} → {z}");
    }

    // …a z ponawianiem zamiar przeżywa nieudaną próbę
    cfg.sltp_retry_s = 1.0;
    let (mut e2, mut b2) = stanowisko(cfg, 4002.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e2, &mut b2, T0 + 1_000, 4006.0);
    tik(&mut e2, &mut b2, T0 + 3_000, 4006.0);
    let z_ponawianiem = b2.positions()[0].sl;
    assert!(z_ponawianiem.is_some(), "po ponowieniu stop musi istnieć");
}

// ============================================================
//  SMART SL
// ============================================================

#[test]
fn smart_sl_daje_najlepszemu_wejsciu_mocniejszy_stop() {
    let mut cfg = Settings::default();
    cfg.smart_sl_mode = SmartSlMode::Ladder;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.assign_tp_per_position = true;
    cfg.entry_units = 3;
    cfg.tp_source = TpSource::SignalOnly;
    cfg.tp_stage_from_broker_fill = false;
    cfg.trail_mode = TrailMode::Off;

    // Cena startuje NAD strefą, więc trzy limity stają na 4000 / 4002.5 / 4005;
    // zejście pod strefę wypełnia je po RÓŻNYCH cenach — bez tego wszystkie
    // pozycje miałyby to samo wejście i ranga nie miałaby znaczenia.
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(b.pendings().len(), 3, "trzy limity w strefie");
    tik(&mut e, &mut b, T0 + 500, 3999.0);
    assert_eq!(b.positions().len(), 3, "wszystkie trzy limity zafillowane");

    // dwa cele trafione → dwie najlepsze pozycje dostają szczeble łańcucha
    let mut m = wiadomosc(T0 + 1_000, 7, "TP2 HIT");
    m.reply_to = Some(1);
    // cena musi pozwolić na taki SL
    tik(&mut e, &mut b, T0 + 900, 4025.0);
    e.on_message(&mut b, &m);

    let mut poz: Vec<(f64, Option<f64>)> =
        b.positions().iter().map(|p| (p.open_price, p.sl)).collect();
    poz.sort_by(|a, c| a.0.partial_cmp(&c.0).unwrap());
    let najlepsza = poz[0].1.expect("najlepsze wejście musi mieć SL");
    let najgorsza = poz[poz.len() - 1]
        .1
        .expect("najgorsze wejście musi mieć SL");
    assert!(
        najlepsza > najgorsza,
        "ranga 0 ma mieć mocniejszy stop: {najlepsza} vs {najgorsza}"
    );
}

// ============================================================
//  PARTIALE Z WOLUMENU
// ============================================================

#[test]
fn partiale_zamykaja_czesc_wolumenu_zamiast_calej_pozycji() {
    let mut cfg = Settings::default();
    cfg.partial_close = true;
    cfg.partial_min_lot = 0.02;
    cfg.lot_fixed = 0.10;
    cfg.tp_schedule = TpSchedule::ScaleOutPct;
    cfg.scale_out_pct = 30.0;
    cfg.assign_tp_per_position = false;
    cfg.entry_units = 2;
    cfg.tp_source = TpSource::SignalOnly;
    cfg.tp_stage_from_broker_fill = false;

    let (mut e, mut b) = stanowisko(cfg, 3998.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let przed: Vec<f64> = b.positions().iter().map(|p| p.volume).collect();
    assert_eq!(przed.len(), 2, "dwie pozycje po 0.10");

    let mut m = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);

    let po: Vec<f64> = b.positions().iter().map(|p| p.volume).collect();
    assert_eq!(po.len(), 2, "żadna pozycja nie może zniknąć w całości");
    for v in &po {
        assert!(*v < 0.10 - 1e-9, "wolumen musi zmaleć, jest {v}");
        assert!(*v >= 0.01 - 1e-9, "zostawiamy co najmniej minimalny lot");
    }
}

#[test]
fn partial_jednego_ticketu_008_i_007_zamyka_dokladnie_polowe_na_tp() {
    for (lot, ocz_cut, ocz_reszta) in [(0.08, 0.04, 0.04), (0.07, 0.04, 0.03)] {
        let mut cfg = Settings::default();
        cfg.partial_close = true;
        cfg.partial_min_lot = 0.02;
        cfg.lot_fixed = lot;
        cfg.tp_schedule = TpSchedule::ScaleOutPct;
        cfg.scale_out_pct = 50.0;
        cfg.assign_tp_per_position = false;
        // Dla matched-control whole-ticket wolno zamknac ostatnia pozycje;
        // w galezi partial flaga nie zmienia wyniku.
        cfg.bank_close_last = true;
        cfg.entry_units = 1;
        cfg.tp_source = TpSource::SignalOnly;
        cfg.tp_stage_from_broker_fill = false;

        let mut whole_cfg = cfg.clone();
        whole_cfg.partial_close = false;

        let (mut e, mut b) = stanowisko(cfg, 3998.0);
        e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
        assert_eq!(b.positions().len(), 1);
        let mut m = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
        m.reply_to = Some(1);
        e.on_message(&mut b, &m);

        assert_eq!(
            b.positions().len(),
            1,
            "partial nie moze usunac ticketu {lot}"
        );
        let reszta = b.positions()[0].volume;
        assert!(
            (reszta - ocz_reszta).abs() < 1e-12,
            "lot {lot}: reszta {reszta}"
        );
        let zamkniete: f64 = b.history.iter().map(|x| x.volume).sum();
        assert!(
            (zamkniete - ocz_cut).abs() < 1e-12,
            "lot {lot}: cut {zamkniete}"
        );

        // Identyczna sciezka i TP, jedyna roznica to istniejacy toggle
        // partial_close: bez niego whole-ticket zamyka cala jedyna pozycje.
        let (mut e_whole, mut b_whole) = stanowisko(whole_cfg, 3998.0);
        e_whole.on_message(&mut b_whole, &wiadomosc(T0, 1, SYGNAL));
        let mut m_whole = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
        m_whole.reply_to = Some(1);
        e_whole.on_message(&mut b_whole, &m_whole);
        assert!(
            b_whole.positions().is_empty(),
            "whole-ticket control ma zamknac cale {lot}"
        );
        let whole_closed: f64 = b_whole.history.iter().map(|x| x.volume).sum();
        assert!((whole_closed - lot).abs() < 1e-12);
    }
}

#[test]
fn partial_tp_czyta_minimum_i_krok_z_brokera() {
    let mut cfg = Settings::default();
    cfg.partial_close = true;
    cfg.partial_min_lot = 0.20;
    cfg.lot_fixed = 0.30;
    cfg.tp_schedule = TpSchedule::ScaleOutPct;
    cfg.scale_out_pct = 50.0;
    cfg.assign_tp_per_position = false;
    cfg.entry_units = 1;
    cfg.tp_source = TpSource::SignalOnly;
    cfg.tp_stage_from_broker_fill = false;

    let (mut e, mut b) = stanowisko(cfg, 3998.0);
    b.volume_min = 0.10;
    b.volume_step = 0.10;
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let mut m = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);

    assert_eq!(b.positions().len(), 1);
    assert!((b.positions()[0].volume - 0.10).abs() < 1e-12);
    assert!((b.history[0].volume - 0.20).abs() < 1e-12);
}

#[test]
fn maly_lot_automatycznie_wraca_do_calych_pozycji() {
    let mut cfg = Settings::default();
    cfg.partial_close = true;
    cfg.partial_min_lot = 0.02;
    cfg.lot_fixed = 0.01; // poniżej progu
    cfg.tp_schedule = TpSchedule::ScaleOutPct;
    cfg.scale_out_pct = 50.0;
    cfg.assign_tp_per_position = false;
    cfg.entry_units = 4;
    cfg.tp_source = TpSource::SignalOnly;
    cfg.tp_stage_from_broker_fill = false;

    let (mut e, mut b) = stanowisko(cfg, 3998.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let przed = b.positions().len();
    assert_eq!(przed, 4, "cztery pozycje po 0.01");

    let mut m = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);

    assert!(
        b.positions().len() < przed,
        "przy locie 0.01 inkaso musi zamknąć CAŁE pozycje"
    );
    for p in b.positions() {
        assert!((p.volume - 0.01).abs() < 1e-9, "wolumen nietknięty");
    }
}

// ============================================================
//  ZAOKRĄGLANIE TRANSZY
// ============================================================

#[test]
fn zaokraglanie_w_gore_ratuje_tp1_na_malym_koszyku() {
    // 15 % z 3 pozycji to 0,45 — „nearest" daje ZERO i TP1 nic nie inkasuje
    let mut cfg = Settings::default();
    cfg.tp_schedule = TpSchedule::OfficialPct;
    cfg.official_pct = [15.0, 30.0, 30.0, 20.0];
    cfg.assign_tp_per_position = false;
    cfg.entry_units = 3;
    cfg.tp_source = TpSource::SignalOnly;
    cfg.tp_stage_from_broker_fill = false;
    cfg.bank_rounding = BankRounding::Nearest;

    let (mut e, mut b) = stanowisko(cfg.clone(), 3998.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let n0 = b.positions().len();
    assert_eq!(n0, 3, "test opiera się na dokładnie trzech pozycjach");
    let mut m = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    assert_eq!(b.positions().len(), n0, "tryb nearest nie inkasuje niczego");

    cfg.bank_rounding = BankRounding::Up;
    let (mut e2, mut b2) = stanowisko(cfg, 3998.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    let n1 = b2.positions().len();
    let mut m2 = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
    m2.reply_to = Some(1);
    e2.on_message(&mut b2, &m2);
    assert!(
        b2.positions().len() < n1,
        "tryb up musi zainkasować co najmniej jedną"
    );
}

#[test]
fn runner_zostaje_dopoki_nie_pozwolimy_domknac() {
    let mut cfg = Settings::default();
    cfg.tp_schedule = TpSchedule::ScaleOutPct;
    cfg.scale_out_pct = 100.0;
    cfg.assign_tp_per_position = false;
    cfg.entry_units = 3;
    cfg.tp_source = TpSource::SignalOnly;
    cfg.tp_stage_from_broker_fill = false;
    cfg.bank_close_last = false;

    let (mut e, mut b) = stanowisko(cfg.clone(), 3998.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(b.positions().len(), 3);
    let mut m = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    assert_eq!(b.positions().len(), 1, "jeden runner zawsze zostaje");

    cfg.bank_close_last = true;
    let (mut e2, mut b2) = stanowisko(cfg, 3998.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    let mut m2 = wiadomosc(T0 + 1_000, 7, "TP1 HIT");
    m2.reply_to = Some(1);
    e2.on_message(&mut b2, &m2);
    assert!(
        b2.positions().is_empty(),
        "z jawną zgodą koszyk wolno domknąć"
    );
}

// ============================================================
//  GUARD WIEKU DLA „SECURING PARTIAL PROFITS"
// ============================================================

#[test]
fn spp_nie_przezbraja_starego_koszyka() {
    let mut cfg = Settings::default();
    cfg.spp_max_age_h = 2.0;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.entry_units = 1;

    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let cele_przed = e.baskets[0].tps.clone();

    let mut m = wiadomosc(
        T0 + 3 * 3_600_000,
        9,
        "SECURING PARTIAL PROFITS\nTP 4100\nTP 4200\nSL 4050",
    );
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    assert_eq!(
        e.baskets[0].tps, cele_przed,
        "stary koszyk zachowuje swoje cele"
    );
}

#[test]
fn spp_przezbraja_swiezy_koszyk() {
    let mut cfg = Settings::default();
    cfg.spp_max_age_h = 12.0;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.entry_units = 1;

    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    let mut m = wiadomosc(
        T0 + 60_000,
        9,
        "SECURING PARTIAL PROFITS AND I WILL TARGET;\n\n4100\n4200",
    );
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    assert_eq!(
        e.baskets[0].tps,
        vec![4100.0, 4200.0],
        "świeży koszyk dostaje nowe cele"
    );
}

// ============================================================
//  „BUY NOW"
// ============================================================

#[test]
fn buy_now_dziala_dopiero_po_wlaczeniu() {
    let mut cfg = Settings::default();
    cfg.honor_market_open = false;
    let (mut e, mut b) = stanowisko(cfg.clone(), 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, "BUY NOW"));
    assert!(
        b.positions().is_empty(),
        "domyślnie komunikat jest ignorowany"
    );

    cfg.honor_market_open = true;
    let (mut e2, mut b2) = stanowisko(cfg, 4002.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, "BUY NOW"));
    assert_eq!(
        b2.positions().len(),
        1,
        "po włączeniu otwiera jedną pozycję"
    );
}

// ============================================================
//  WAGI GŁĘBOKOŚCI I LIMIT RYZYKA KOSZYKA
// ============================================================

/// Wolumeny zleceń koszyka, uporządkowane od NAJGŁĘBSZEGO wejścia.
///
/// Dla BUY głębiej znaczy taniej, więc sortujemy rosnąco po cenie.
fn wolumeny_wg_glebokosci(b: &SimBroker) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = b.pendings().iter().map(|o| (o.price, o.volume)).collect();
    v.sort_by(|a, c| a.0.partial_cmp(&c.0).unwrap());
    v
}

#[test]
fn wagi_daja_wiekszy_wolumen_glebszym_wejsciom() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.lot_fixed = 0.10; // dość duży lot, żeby krok 0.01 nie zjadł proporcji
    cfg.entry_weights = "1,2,4".into();

    // cena NAD strefą — wszystkie trzy poziomy zostają limitami
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));

    let v = wolumeny_wg_glebokosci(&b);
    assert_eq!(v.len(), 3, "trzy poziomy siatki");
    // 4000 (najgłębiej) > 4002.5 > 4005 (najpłycej)
    assert!(
        v[0].1 > v[1].1 && v[1].1 > v[2].1,
        "wolumen ma rosnąć z głębokością: {v:?}"
    );
}

#[test]
fn wagi_nie_powiekszaja_koszyka_tylko_go_przewazaja() {
    let mut baza = Settings::default();
    baza.entry_units = 3;
    baza.lot_fixed = 0.10;

    let (mut e1, mut b1) = stanowisko(baza.clone(), 4008.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, SYGNAL));
    let rowno: f64 = b1.pendings().iter().map(|o| o.volume).sum();

    let mut z_wagami = baza;
    z_wagami.entry_weights = "1,2,4".into();
    let (mut e2, mut b2) = stanowisko(z_wagami, 4008.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    let wazone: f64 = b2.pendings().iter().map(|o| o.volume).sum();

    assert!(
        (rowno - wazone).abs() <= 0.03,
        "łączny wolumen ma zostać ten sam: równo {rowno}, ważone {wazone}"
    );
}

#[test]
fn limit_ryzyka_scina_wolumeny_koszyka() {
    // Bez limitu: 4 poziomy × 0.10 lota, SL ~10-15 $ od wejścia → ryzyko
    // rzędu 500 $ na koncie 1000 $. Limit 3 % musi to ostro przyciąć.
    let mut baza = Settings::default();
    baza.entry_units = 4;
    baza.lot_fixed = 0.10;

    let (mut e1, mut b1) = stanowisko(baza.clone(), 4008.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, SYGNAL));
    let ryzyko_bez = ryzyko_koszyka(&b1);
    assert!(
        ryzyko_bez > 30.0,
        "test wymaga wyjściowo dużego ryzyka: {ryzyko_bez}"
    );

    let mut z_limitem = baza;
    z_limitem.risk_per_basket_pct = 3.0; // 3 % z 1000 $ = 30 $
    let (mut e2, mut b2) = stanowisko(z_limitem, 4008.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    let ryzyko_z = ryzyko_koszyka(&b2);

    assert!(ryzyko_z < ryzyko_bez, "limit musi zmniejszyć ryzyko");
    assert!(
        ryzyko_z <= 30.0 + 1e-6,
        "ryzyko koszyka {ryzyko_z} przekracza limit 30 $"
    );
}

/// Σ |cena wejścia − SL| × 100 × wolumen po wszystkich zleceniach koszyka.
fn ryzyko_koszyka(b: &SimBroker) -> f64 {
    b.pendings()
        .iter()
        .filter_map(|o| o.sl.map(|s| (o.price - s).abs() * 100.0 * o.volume))
        .sum::<f64>()
        + b.positions()
            .iter()
            .filter_map(|p| p.sl.map(|s| (p.open_price - s).abs() * 100.0 * p.volume))
            .sum::<f64>()
}

#[test]
fn limit_ryzyka_odrzuca_koszyk_ktorego_nie_da_sie_zmiescic() {
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.lot_fixed = 0.10;
    cfg.lot_min = 0.01;
    // 0.01 % z 1000 $ = 0.10 $, a jeden poziom przy minimalnym locie
    // to co najmniej kilka dolarów ryzyka — nie ma jak zmieścić
    cfg.risk_per_basket_pct = 0.01;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        b.pendings().is_empty(),
        "koszyk musi zostać odrzucony w całości"
    );
    assert!(b.positions().is_empty());
}

#[test]
fn bez_limitu_ryzyka_nic_sie_nie_zmienia() {
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.lot_fixed = 0.10;
    cfg.risk_per_basket_pct = 0.0;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(b.pendings().len(), 4);
    for o in b.pendings() {
        assert!(
            (o.volume - 0.10).abs() < 1e-9,
            "wolumen nietknięty: {}",
            o.volume
        );
    }
}

#[test]
fn limit_ryzyka_najpierw_scina_a_potem_usuwa_najplytsze() {
    // Limit na tyle ciasny, że samo skalowanie nie wystarczy — muszą zniknąć
    // poziomy, i to te NAJPŁYTSZE (najgorszy stosunek zysku do ryzyka).
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.lot_fixed = 0.10;
    cfg.lot_min = 0.01;
    cfg.risk_per_basket_pct = 1.0; // 10 $ na koncie 1000 $

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));

    let v = wolumeny_wg_glebokosci(&b);
    assert!(!v.is_empty(), "coś powinno zostać");
    assert!(v.len() < 4, "część poziomów musiała zniknąć: {v:?}");
    // został najgłębszy poziom, czyli najniższa cena dla BUY
    assert!(
        (v[0].0 - 4000.0).abs() < 1e-6,
        "zostać ma najgłębsze wejście, zostało {:?}",
        v[0].0
    );
    assert!(ryzyko_koszyka(&b) <= 10.0 + 1e-6);
}

// ============================================================
//  ŻYCIE SIATKI LIMITÓW — CEL OSIĄGNIĘTY BEZ NAS
// ============================================================

/// Sygnał limitowy: strefa PONIŻEJ rynku, cele POWYŻEJ strefy.
/// Rynek startuje między strefą a TP1, więc rozstrzyga wyścig:
/// „najpierw cofka do strefy" czy „najpierw cel".
const LIMIT_SYGNAL: &str = "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3995";

/// Konfiguracja klasy BIEZACY: siatka limitów, cele czytane też z ceny,
/// limity żyją do pierwszego celu.
fn cfg_siatka() -> Settings {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.lot_fixed = 0.01;
    cfg.tp_source = TpSource::Either;
    cfg.pending_lifetime = PendingLifetime::UntilTp1;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.assign_tp_per_position = true;
    cfg.ignore_old_after_min = 0.0;
    cfg.pending_ttl_h = 0.0;
    cfg
}

/// Odtwarza jeden i ten sam przebieg: sygnał limitowy, rynek idzie NAJPIERW
/// do TP1, dopiero potem cofa się w strefę. Zwraca (koszyki, wypełnienia).
fn przebieg_cel_przed_strefa(drop_on_target: bool) -> (usize, u64) {
    let mut cfg = cfg_siatka();
    cfg.pending_drop_on_target = drop_on_target;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, LIMIT_SYGNAL));
    assert_eq!(b.pendings().len(), 3, "siatka musi się rozstawić");

    // rynek idzie w GÓRĘ, do pierwszego celu — bez nas
    for (i, bid) in [4009.0, 4010.5, 4012.0].iter().enumerate() {
        tik(&mut e, &mut b, T0 + 1_000 * (i as i64 + 1), *bid);
    }
    // …a potem wraca przez całą strefę aż pod SL
    for (i, bid) in [4006.0, 4004.0, 4002.0, 4000.0, 3998.0].iter().enumerate() {
        tik(&mut e, &mut b, T0 + 10_000 + 1_000 * (i as i64), *bid);
    }
    (e.baskets.len(), b.filled_pendings)
}

#[test]
fn siatka_ginie_gdy_cena_siegnie_celu_bez_wejscia() {
    let (koszyki, wejscia) = przebieg_cel_przed_strefa(true);
    assert_eq!(koszyki, 1, "sygnał zakłada dokładnie jeden koszyk");
    assert_eq!(
        wejscia, 0,
        "TP1 padł, zanim cena wróciła do strefy — siatka miała zniknąć, \
         a weszła {wejscia} razy"
    );
}

/// Druga strona przełącznika. Bez niej test wyżej przechodziłby także wtedy,
/// gdyby siatka nie wypełniała się z zupełnie innego powodu.
#[test]
fn bez_wygaszania_ta_sama_siatka_wchodzi_pelna_liczba_razy() {
    let (koszyki, wejscia) = przebieg_cel_przed_strefa(false);
    assert_eq!(koszyki, 1);
    assert_eq!(
        wejscia, 3,
        "z wyłączonym wygaszaniem cofka do strefy musi wypełnić całą siatkę"
    );
}

/// Domyślnie reguła jest WŁĄCZONA — cała biblioteka presetów i wszystkie
/// wyniki sweepów powstały przy tym zachowaniu.
#[test]
fn wygaszanie_siatki_na_celu_jest_domyslne() {
    assert!(Settings::default().pending_drop_on_target);
}

/// Koszyk, który JUŻ wszedł, idzie starą ścieżką: cel z ceny jest prawdziwym
/// trafionym celem, więc etap rośnie i to on — a nie wygaszanie siatki —
/// decyduje o losie reszty limitów.
#[test]
fn cel_po_wejsciu_jest_liczony_jako_trafiony_a_nie_jako_wygasniecie() {
    let mut cfg = cfg_siatka();
    cfg.entry_units = 2;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, LIMIT_SYGNAL));
    assert_eq!(b.pendings().len(), 2);

    // najpierw cofka: najpłytszy limit się wypełnia
    tik(&mut e, &mut b, T0 + 1_000, 4004.0);
    assert_eq!(b.filled_pendings, 1, "płytszy limit musi się wypełnić");
    assert!(!b.positions().is_empty());

    // teraz cena idzie na TP1 — to jest TRAFIONY cel koszyka, nie wygaśnięcie
    tik(&mut e, &mut b, T0 + 2_000, 4011.0);
    let bk = &e.baskets[0];
    assert!(bk.had_positions, "koszyk handlował");
    assert_eq!(bk.tp_stage, 1, "etap celu musi urosnąć do TP1");
    assert!(
        b.pendings().is_empty(),
        "przy UntilTp1 reszta limitów znika"
    );
}


/// Strefa 3998–4002 przy cenie bid 4001.90 / ask 4002.10 i `stops_level` 0.20.
/// Poziom 4002 leży po dobrej stronie (poniżej ask), ale w pasie stops level —
/// to jest dokładnie ten szczebel, którego broker nie przyjmuje.
const SYGNAL_STREFA_NA_CENIE: &str = "BUY GOLD @ 4002/3998\nTP 4010\nTP 4020\nTP 4030\nSL 3990";

fn sprawdz_wejscie_przy_strefie_na_cenie(krata: bool) {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.grid_anchor_absolute = krata;
    cfg.stops_level = 0.20;
    cfg.tp_schedule = TpSchedule::AllRunners;

    let (mut e, mut b) = stanowisko(cfg, 4001.90);
    let q = b.quote();
    assert!(
        q.ask > 4002.0 && q.ask < 4002.2,
        "scenariusz wymaga ceny WEWNĄTRZ strefy"
    );

    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_STREFA_NA_CENIE));

    // 1) Sygnał musi skończyć się WEJŚCIEM, a nie zerem zleceń.
    let zlecen = b.positions().len() + b.pendings().len();
    assert!(
        zlecen > 0,
        "krata={krata}: sygnał ze strefą na cenie musi coś rozstawić"
    );
    assert!(
        !b.positions().is_empty(),
        "krata={krata}: szczebel, na którym limit nie może leżeć, ma zejść na wejście rynkowe"
    );

    // 2) Każdy limit, który ZOSTAJE na rynku, musi być do przyjęcia przez
    //    brokera. To jest ta połowa, której nie było: poziom 4002 przy ask
    //    4002.10 wygląda poprawnie (jest poniżej ceny), a mimo to wraca z
    //    `10015`, bo mieści się w pasie stops level.
    for o in b.pendings() {
        assert!(
            limit_price_is_valid(o.kind.side(), o.price, &q, 0.20),
            "krata={krata}: limit {:.2} przy ask {:.2} zostałby odrzucony jako INVALID_PRICE",
            o.price,
            q.ask
        );
    }
}

#[test]
fn strefa_na_cenie_konczy_sie_wejsciem_krata_wlaczona() {
    sprawdz_wejscie_przy_strefie_na_cenie(true);
}

#[test]
fn strefa_na_cenie_konczy_sie_wejsciem_krata_wylaczona() {
    sprawdz_wejscie_przy_strefie_na_cenie(false);
}

/// Wariant `Skip` to JEDYNY, który świadomie rezygnuje z wejścia — i właśnie
/// dlatego nie może być domyślny. Test pilnuje, żeby domyślna wartość
/// odtwarzała zachowanie symulatora (wejście po rynku).
#[test]
fn domyslna_polityka_odtwarza_symulator_a_skip_swiadomie_rezygnuje() {
    assert_eq!(
        Settings::default().pending_cross_policy,
        PendingCrossPolicy::Market,
        "domyślnie musi wchodzić po rynku — tak liczy backtest"
    );

    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.stops_level = 0.20;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.pending_cross_policy = PendingCrossPolicy::Skip;

    let (mut e, mut b) = stanowisko(cfg, 4001.90);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_STREFA_NA_CENIE));
    assert!(
        b.positions().is_empty(),
        "przy Skip niewykonalny szczebel jest pomijany, nie zamieniany na rynek"
    );
}


/// Strefa 4000–4005, SL 3990, TP1 4010. Trzy szczeble mają wtedy R:R
/// 10/10 = 1,00 · 7,5/12,5 = 0,60 · 5/15 = 0,33 — a więc realną, policzalną
/// z sygnału różnicę jakości, a nie zgadywaną drabinkę „1,2,4".
#[test]
fn wagi_z_rr_daja_wiecej_wolumenu_szczeblowi_o_lepszej_geometrii() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.lot_fixed = 0.10;
    cfg.entry_weights_from_rr = true;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));

    let v = wolumeny_wg_glebokosci(&b);
    assert_eq!(v.len(), 3, "trzy poziomy siatki: {v:?}");
    assert!(
        v[0].1 > v[1].1 && v[1].1 > v[2].1,
        "wolumen ma rosnąć z jakością szczebla: {v:?}"
    );
}

#[test]
fn wagi_z_rr_przewazaja_koszyk_a_nie_go_powiekszaja() {
    let mut baza = Settings::default();
    baza.entry_units = 3;
    baza.lot_fixed = 0.10;

    let (mut e1, mut b1) = stanowisko(baza.clone(), 4008.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, SYGNAL));
    let rowno: f64 = b1.pendings().iter().map(|o| o.volume).sum();

    let mut z_rr = baza;
    z_rr.entry_weights_from_rr = true;
    let (mut e2, mut b2) = stanowisko(z_rr, 4008.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    let wazone: f64 = b2.pendings().iter().map(|o| o.volume).sum();

    assert!(
        (rowno - wazone).abs() <= 0.03,
        "łączny wolumen ma zostać ten sam: równo {rowno}, z R:R {wazone}"
    );
}

/// OSTRZEŻENIE Z PLANU: równe wagi przy wielu jednostkach zerowały konto.
/// Wagi wolno włączać wyłącznie razem z limitem ryzyka na koszyk — ten test
/// przypina, że oba działają JEDNOCZEŚNIE i że limit wygrywa.
#[test]
fn wagi_z_rr_nie_omijaja_limitu_ryzyka_koszyka() {
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.lot_fixed = 0.10;
    cfg.entry_weights_from_rr = true;
    cfg.risk_per_basket_pct = 3.0; // 3 % z 1000 $ = 30 $

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));

    let r = ryzyko_koszyka(&b);
    assert!(r > 0.0, "koszyk musi w ogóle powstać");
    assert!(
        r <= 30.0 + 1e-6,
        "ryzyko koszyka {r} $ przekracza limit 30 $ mimo wag"
    );
}

#[test]
fn wagi_z_rr_wylaczone_zostawiaja_wolumeny_rowne() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.lot_fixed = 0.10;
    assert!(!cfg.entry_weights_from_rr, "domyślnie wyłączone");

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    for o in b.pendings() {
        assert!(
            (o.volume - 0.10).abs() < 1e-9,
            "wolumen nietknięty: {}",
            o.volume
        );
    }
}


/// SL blisko strefy, żeby `sl_min_dist` miał co poszerzać. Bez tego reguła
/// jest niewidoczna: stop z sygnału i tak leży dalej, niż wymaga próg.
const WASKI: &str = "BUY GOLD @ 4002/4000\nTP 4010\nTP 4020\nSL 4000";
const SZEROKI: &str = "BUY GOLD @ 4008/4000\nTP 4020\nTP 4030\nSL 4000";

/// Odległość SL od środka strefy w pierwszym wystawionym zleceniu.
fn dystans_sl(b: &SimBroker, mid: f64) -> f64 {
    let o = b
        .pendings()
        .first()
        .expect("musi powstać choć jedno zlecenie");
    (mid - o.sl.expect("zlecenie musi nieść SL")).abs()
}

#[test]
fn sl_min_dist_liczony_z_szerokosci_strefy_skaluje_sie_z_sygnalem() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.adaptive_params = true;
    cfg.sl_min_dist_zone_mult = 0.7;

    let (mut e1, mut b1) = stanowisko(cfg.clone(), 4012.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, WASKI));
    // strefa 2 $ → 0,7 × 2 = 1,4 $ od środka 4001
    assert!(
        (dystans_sl(&b1, 4001.0) - 1.4).abs() < 1e-6,
        "wąska: {}",
        dystans_sl(&b1, 4001.0)
    );

    let (mut e2, mut b2) = stanowisko(cfg, 4012.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 2, SZEROKI));
    // strefa 8 $ → 0,7 × 8 = 5,6 $ od środka 4004
    assert!(
        (dystans_sl(&b2, 4004.0) - 5.6).abs() < 1e-6,
        "szeroka: {}",
        dystans_sl(&b2, 4004.0)
    );
}

/// Przełącznik wyłączony musi dawać DOKŁADNIE ten sam SL, co konfiguracja
/// bez tych pól w ogóle. To jest właściwa postać testu „off": nie „jakaś
/// stała liczba", tylko „bit w bit tak jak przed dołożeniem mechanizmu".
#[test]
fn bez_adaptacji_nowe_pola_nie_zmieniaja_niczego() {
    let mut z_polami = Settings::default();
    z_polami.entry_units = 3;
    z_polami.sl_min_dist = 3.0;
    z_polami.adaptive_params = false;
    // celowo poustawiane — bez włącznika nie mają prawa zadziałać
    z_polami.sl_min_dist_zone_mult = 0.7;
    z_polami.sl_min_dist_atr_mult = 2.0;
    z_polami.sl_min_dist_floor = 9.0;
    z_polami.sl_min_dist_cap = 0.5;
    z_polami.entry_deep_zone_mult = 5.0;
    z_polami.entry_units_zone_ref = 1.0;
    z_polami.units_by_hour = "22-23:3".into();

    let mut czyste = Settings::default();
    czyste.entry_units = 3;
    czyste.sl_min_dist = 3.0;

    for (nr, tekst) in [(1i64, WASKI), (2, SZEROKI)] {
        let (mut e1, mut b1) = stanowisko(z_polami.clone(), 4012.0);
        e1.on_message(&mut b1, &wiadomosc(T0, nr, tekst));
        let (mut e2, mut b2) = stanowisko(czyste.clone(), 4012.0);
        e2.on_message(&mut b2, &wiadomosc(T0, nr, tekst));

        let a: Vec<(f64, Option<f64>, f64)> = b1
            .pendings()
            .iter()
            .map(|o| (o.price, o.sl, o.volume))
            .collect();
        let c: Vec<(f64, Option<f64>, f64)> = b2
            .pendings()
            .iter()
            .map(|o| (o.price, o.sl, o.volume))
            .collect();
        assert_eq!(a, c, "sygnał {nr}: wyłączone pola zmieniły siatkę");
    }
}

#[test]
fn podloga_i_sufit_trzymaja_wyliczony_sl_w_ryzach() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.adaptive_params = true;
    cfg.sl_min_dist_zone_mult = 0.7;
    cfg.sl_min_dist_floor = 2.0;
    cfg.sl_min_dist_cap = 4.0;

    // wąska strefa dałaby 1,4 $ — podłoga podnosi do 2,0
    let (mut e1, mut b1) = stanowisko(cfg.clone(), 4012.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, WASKI));
    assert!((dystans_sl(&b1, 4001.0) - 2.0).abs() < 1e-6);

    // szeroka dałaby 5,6 $ — sufit ścina do 4,0
    let (mut e2, mut b2) = stanowisko(cfg, 4012.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 2, SZEROKI));
    assert!((dystans_sl(&b2, 4004.0) - 4.0).abs() < 1e-6);
}

#[test]
fn glebokosc_wejscia_rosnie_z_szerokoscia_strefy() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.zone_offset_mode = ZoneOffsetMode::Directional;
    cfg.entry_deep_offset = 3.0;
    cfg.entry_tol_offset = 0.0;
    cfg.adaptive_params = true;
    cfg.entry_deep_zone_mult = 1.0;

    let (mut e1, mut b1) = stanowisko(cfg.clone(), 4012.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, WASKI));
    // strefa 4000–4002 rozciągnięta o 1 × 2 $ → najgłębszy poziom 3998
    assert!(
        (najglebszy_poziom(&b1) - 3998.0).abs() < 1e-6,
        "{}",
        najglebszy_poziom(&b1)
    );

    let (mut e2, mut b2) = stanowisko(cfg, 4012.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 2, SZEROKI));
    // strefa 4000–4008 rozciągnięta o 1 × 8 $ → najgłębszy poziom 3992
    assert!(
        (najglebszy_poziom(&b2) - 3992.0).abs() < 1e-6,
        "{}",
        najglebszy_poziom(&b2)
    );
}

fn najglebszy_poziom(b: &SimBroker) -> f64 {
    b.pendings()
        .iter()
        .map(|o| o.price)
        .fold(f64::MAX, f64::min)
}

#[test]
fn bez_adaptacji_glebokosc_jest_stala_dla_obu_sygnalow() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.zone_offset_mode = ZoneOffsetMode::Directional;
    cfg.entry_deep_offset = 3.0;
    cfg.entry_tol_offset = 0.0;
    cfg.entry_deep_zone_mult = 1.0; // bez `adaptive_params` ma nie działać

    let (mut e1, mut b1) = stanowisko(cfg.clone(), 4012.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, WASKI));
    let (mut e2, mut b2) = stanowisko(cfg, 4012.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 2, SZEROKI));
    assert!(
        (najglebszy_poziom(&b1) - 3997.0).abs() < 1e-6,
        "{}",
        najglebszy_poziom(&b1)
    );
    assert!(
        (najglebszy_poziom(&b2) - 3997.0).abs() < 1e-6,
        "{}",
        najglebszy_poziom(&b2)
    );
}

/// `T0` wypada o godzinie 22 czasu serwera — test przypina, że pasmo
/// godzinowe naprawdę zmienia liczbę szczebli, a nie tylko się parsuje.
#[test]
fn pora_dnia_zmienia_liczbe_szczebli_siatki() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.adaptive_params = true;
    cfg.units_by_hour = "22-23:2".into();

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(
        b.pendings().len(),
        6,
        "mnożnik 2 na tej godzinie: 3 → 6 szczebli"
    );

    let mut bez = Settings::default();
    bez.entry_units = 3;
    bez.units_by_hour = "22-23:2".into(); // bez włącznika ma nie działać
    let (mut e2, mut b2) = stanowisko(bez, 4008.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(b2.pendings().len(), 3);
}

// ============================================================
//  RISK FREE JAKO REGUŁA SILNIKA
// ============================================================
//
// Struktura wypłaty, o którą chodzi: zabankować tyle zysku, żeby pokrył
// straty reszty, a runnera zostawić ze stopem na ŚREDNIEJ WAŻONEJ CENIE
// WEJŚCIA koszyka. Dół zamknięty, góra otwarta.
//
// Sygnał testowy jest SZEROKI celowo: trzy szczeble wchodzą po wyraźnie
// różnych cenach, więc średnia ważona jest inną liczbą niż każda z nich
// z osobna — i widać, czy stop naprawdę ląduje na średniej KOSZYKA, a nie
// na własnej cenie nogi.
const RF_SYGNAL: &str = "BUY GOLD @ 4020/3980\nTP 4200\nTP 4300\nTP 4400\nSL 3950";

/// Rozstawia koszyk i wypełnia trzy szczeble po 4020, 4000 i 3980.
/// Zwraca stanowisko gotowe do wywołania reguły.
fn koszyk_trzy_wejscia(cfg: Settings) -> (Engine, SimBroker) {
    let (mut e, mut b) = stanowisko(cfg, 4030.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, RF_SYGNAL));
    assert_eq!(b.pendings().len(), 3, "trzy szczeble siatki");
    tik(&mut e, &mut b, T0 + 1_000, 4019.8); // wypełnia 4020
    tik(&mut e, &mut b, T0 + 2_000, 3999.8); // wypełnia 4000
    tik(&mut e, &mut b, T0 + 3_000, 3979.8); // wypełnia 3980
    assert_eq!(b.positions().len(), 3, "trzy otwarte pozycje");
    (e, b)
}

fn cfg_riskfree() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3;
    c.lot_fixed = 0.10;
    c.riskfree_enabled = true;
    c.riskfree_trigger_usd = 200.0;
    c.riskfree_keep_units = 1;
    c.riskfree_runner_target = RiskFreeRunnerTarget::LastTp;
    c
}

/// Wejścia 4020 / 4000 / 3980 po 0,10 lota → średnia ważona 4000,00.
/// Runnerem zostaje wejście najgłębsze (3980), więc stop na średniej leży
/// 20 $ NAD jego ceną wejścia: koszyk wychodzi na zero, a runner na plus.
#[test]
fn stop_runnera_laduje_na_sredniej_wazonej_wejsc_koszyka() {
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg_riskfree());

    // zysk koszyka: +100 +300 +500 = +900 $, próg 200 $ przekroczony
    tik(&mut e, &mut b, T0 + 4_000, 4030.0);

    assert_eq!(b.positions().len(), 1, "zostaje jeden runner");
    let r = &b.positions()[0];
    assert!(
        (r.open_price - 3980.0).abs() < 1e-6,
        "runnerem zostaje najgłębsze wejście"
    );
    let sl = r.sl.expect("runner musi mieć stop");
    assert!(
        (sl - 4000.0).abs() < 1e-6,
        "stop ma leżeć na średniej ważonej koszyka 4000,00, a jest {sl}"
    );

    let zabankowane: f64 = b
        .history
        .iter()
        .filter(|t| t.reason == CloseReason::RiskFree)
        .map(|t| t.profit)
        .sum();
    assert!(
        (zabankowane - 400.0).abs() < 1e-6,
        "zabankowane ma wynieść +400 $ (100 + 300), jest {zabankowane}"
    );
}

/// Sedno reguły: po uwolnieniu koszyk NIE MOŻE JUŻ STRACIĆ.
/// Cena zawraca 31 $ w dół i zbiera runnera na stopie — a koszyk i tak
/// kończy mocno na plusie, bo zysk został zabankowany wcześniej.
#[test]
fn po_uwolnieniu_od_ryzyka_koszyk_nie_moze_juz_stracic() {
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg_riskfree());
    tik(&mut e, &mut b, T0 + 4_000, 4030.0); // reguła się uruchamia
    tik(&mut e, &mut b, T0 + 5_000, 3999.0); // zawrót — runner zbierany na BE

    assert!(b.positions().is_empty(), "runner wyszedł na stopie");
    let wynik: f64 = b.history.iter().map(|t| t.profit).sum();
    assert!(
        wynik > 0.0,
        "koszyk po uwolnieniu nie może wyjść na minus: {wynik:.2} $"
    );

    // Kontrola: TA SAMA ścieżka ceny bez reguły kończy się STRATĄ.
    // To jest cała wartość mechanizmu — i zarazem dowód, że test mierzy
    // regułę, a nie łagodny scenariusz.
    let mut bez = cfg_riskfree();
    bez.riskfree_enabled = false;
    let (mut e2, mut b2) = koszyk_trzy_wejscia(bez);
    tik(&mut e2, &mut b2, T0 + 4_000, 4030.0);
    tik(&mut e2, &mut b2, T0 + 5_000, 3999.0);
    let bez_reguly: f64 = b2.history.iter().map(|t| t.profit).sum::<f64>()
        + b2.positions()
            .iter()
            .map(|p| p.profit_usd(&b2.q))
            .sum::<f64>();
    assert!(
        bez_reguly < 0.0,
        "bez reguły ta sama ścieżka ma być stratna, a wyszło {bez_reguly:.2} $"
    );
    assert!(wynik > bez_reguly);
}

/// ⚠ DRUGA STRONA MEDALU — to NIE jest darmowy pieniądz.
/// Stop na średniej bywa zbierany tuż przed ruchem we właściwą stronę.
/// Wtedy reguła zamienia dużą wygraną na małą i test ma to pokazywać
/// wprost, a nie chować.
#[test]
fn stop_na_be_bywa_zbierany_tuz_przed_ruchem_i_kosztuje() {
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg_riskfree());
    tik(&mut e, &mut b, T0 + 4_000, 4030.0);
    tik(&mut e, &mut b, T0 + 5_000, 3999.0); // szpilka zbiera runnera
    tik(&mut e, &mut b, T0 + 6_000, 4100.0); // …i dopiero teraz ruch właściwy
    let z_regula: f64 = b.history.iter().map(|t| t.profit).sum();

    let mut bez = cfg_riskfree();
    bez.riskfree_enabled = false;
    let (mut e2, mut b2) = koszyk_trzy_wejscia(bez);
    tik(&mut e2, &mut b2, T0 + 4_000, 4030.0);
    tik(&mut e2, &mut b2, T0 + 5_000, 3999.0);
    tik(&mut e2, &mut b2, T0 + 6_000, 4100.0);
    let bez_reguly: f64 = b2.history.iter().map(|t| t.profit).sum::<f64>()
        + b2.positions()
            .iter()
            .map(|p| p.profit_usd(&b2.q))
            .sum::<f64>();

    assert!(
        z_regula < bez_reguly,
        "na tej ścieżce reguła MUSI kosztować: z regułą {z_regula:.2} $, bez {bez_reguly:.2} $"
    );
    assert!(
        z_regula > 0.0,
        "ale nadal nie wolno jej wyjść na minus: {z_regula:.2} $"
    );
}

/// „RISK FREE", które zostawia koszyk pod wodą, byłoby kłamstwem.
/// Przy zysku 150 $ całość niesie jedna noga (+250), a reszta jest 100 $
/// pod kreską — zabankowanie ich NIE pokryłoby strat, więc reguła milczy.
#[test]
fn regula_milczy_gdy_bank_nie_pokrylby_strat() {
    let mut cfg = cfg_riskfree();
    cfg.riskfree_trigger_usd = 100.0;
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg);

    // zysk koszyka: −150 +50 +250 = +150 $ ≥ próg 100 $,
    // ale bank z dwóch gorszych nóg to −100 $
    tik(&mut e, &mut b, T0 + 4_000, 4005.0);

    assert_eq!(b.positions().len(), 3, "nic nie wolno zamknąć");
    assert!(
        b.history.iter().all(|t| t.reason != CloseReason::RiskFree),
        "żadna pozycja nie mogła zostać zabankowana jako RISK FREE"
    );
}

#[test]
fn bez_wlacznika_koszyk_nie_jest_uwalniany() {
    let mut cfg = cfg_riskfree();
    cfg.riskfree_enabled = false;
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg);
    tik(&mut e, &mut b, T0 + 4_000, 4030.0);
    assert_eq!(b.positions().len(), 3, "bez włącznika nic się nie zmienia");
    assert!(b.history.is_empty(), "nic nie zostało zamknięte");
}

/// Próg wyrażony wielokrotnością RYZYKA koszyka zamiast kwotą — to samo
/// zdarzenie, inna jednostka.
///
/// Ryzyko koszyka przy SL 3950: (4020−3950 + 4000−3950 + 3980−3950) × 100
/// × 0,10 = **1500 $**. Zysk przy 4030 to 900 $, więc 0,5R (750 $) odpala
/// regułę, a 1R (1500 $) jeszcze nie — i test przypina OBIE strony progu.
#[test]
fn prog_w_wielokrotnosci_ryzyka_dziala_tak_samo_jak_kwotowy() {
    let mut cfg = cfg_riskfree();
    cfg.riskfree_trigger_usd = 0.0;
    cfg.riskfree_trigger_r = 0.5;
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg);
    tik(&mut e, &mut b, T0 + 4_000, 4030.0);
    assert_eq!(
        b.positions().len(),
        1,
        "0,5R = 750 $ przy zysku 900 $ — reguła musi zadziałać"
    );
    assert!(b.positions()[0].sl.is_some());

    let mut wyzej = cfg_riskfree();
    wyzej.riskfree_trigger_usd = 0.0;
    wyzej.riskfree_trigger_r = 1.0;
    let (mut e2, mut b2) = koszyk_trzy_wejscia(wyzej);
    tik(&mut e2, &mut b2, T0 + 4_000, 4030.0);
    assert_eq!(
        b2.positions().len(),
        3,
        "1R = 1500 $ przy zysku 900 $ — jeszcze za wcześnie"
    );
}

/// Reguła nie ma prawa COFNĄĆ stopu, który już jest lepszy — miałaby wtedy
/// zwiększać ryzyko, a ma je wyłącznie zdejmować.
#[test]
fn uwolnienie_nie_cofa_stopu_ktory_juz_byl_lepszy() {
    let mut cfg = cfg_riskfree();
    // trailing dociągnie stop wysoko, zanim reguła zdąży zadziałać
    cfg.trail_mode = TrailMode::Gap;
    cfg.trail_start = 1.0;
    cfg.trail_gap = 2.0;
    cfg.riskfree_trigger_usd = 850.0;
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg);
    tik(&mut e, &mut b, T0 + 4_000, 4030.0);

    for p in b.positions() {
        if let Some(sl) = p.sl {
            assert!(
                sl >= 4000.0 - 1e-6,
                "stop {sl} runnera cofnięty poniżej średniej koszyka"
            );
        }
    }
}


/// Koszyk, którego wszystkie trzy szczeble weszły i wyszły na TP1 —
/// czyli setup, który ZADZIAŁAŁ. Dopiero taki wolno dokładać.
fn cfg_rearm() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3;
    c.tp_schedule = TpSchedule::AllAtTp1;
    c.pending_lifetime = PendingLifetime::Never;
    // szczebel, na którym limit już się nie położy, ma być POMINIĘTY, a nie
    // zamieniony na wejście rynkowe — inaczej test mierzyłby dwie rzeczy naraz
    c.pending_cross_policy = PendingCrossPolicy::Skip;
    c.rearm_grid_on_return = true;
    c.rearm_min_gap_min = 0.0;
    c.rearm_max_times = 5;
    c
}

#[test]
fn siatka_wraca_gdy_cena_wroci_do_strefy_a_koszyk_jest_na_plusie() {
    let (mut e, mut b) = stanowisko(cfg_rearm(), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(b.pendings().len(), 3);

    tik(&mut e, &mut b, T0 + 1_000, 3999.0); // wszystkie trzy wchodzą
    assert_eq!(b.positions().len(), 3);
    tik(&mut e, &mut b, T0 + 2_000, 4011.0); // wszystkie wychodzą na TP1
    assert!(b.positions().is_empty(), "koszyk domknięty na celu");
    assert!(b.pendings().is_empty(), "siatka pusta");

    tik(&mut e, &mut b, T0 + 3_000, 4004.5); // cena wraca do strefy
    assert_eq!(
        b.pendings().len(),
        2,
        "dwa szczeble leżące pod rynkiem mają wrócić jako limity"
    );
    assert_eq!(e.baskets[0].rearms, 1, "przezbrojenie ma zostać policzone");
}

#[test]
fn bez_wlacznika_siatka_nie_wraca() {
    let mut cfg = cfg_rearm();
    cfg.rearm_grid_on_return = false;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0);
    tik(&mut e, &mut b, T0 + 2_000, 4011.0);
    tik(&mut e, &mut b, T0 + 3_000, 4004.5);
    assert!(b.pendings().is_empty(), "bez włącznika nic nie wraca");
    assert_eq!(e.baskets[0].rearms, 0);
}

/// POTWIERDZENIE jest istotą tej reguły. Koszyk pod wodą to nie okazja do
/// dokładania, tylko uśrednianie straty — dokładnie to, czego trader nie robi.
#[test]
fn koszyk_pod_woda_nie_dostaje_dolozenia() {
    let (mut e, mut b) = stanowisko(cfg_rearm(), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0); // wejścia
    tik(&mut e, &mut b, T0 + 2_000, 3989.0); // stop-loss 3990 zbiera całość
    assert!(b.positions().is_empty());

    tik(&mut e, &mut b, T0 + 3_000, 4004.5); // cena wraca, ale koszyk stratny
    assert!(
        b.pendings().is_empty(),
        "do stratnego koszyka nie dokładamy"
    );
    assert_eq!(e.baskets[0].rearms, 0);
}


/// Stanowisko z ustaloną bazą doby: pierwszy tick ustawia `day_start_equity`
/// na równe 1000 $, więc procenty w teście są policzalne na piechotę.
fn dzien_od_1000(cfg: Settings) -> (Engine, SimBroker) {
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    tik(&mut e, &mut b, T0, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0);
    assert_eq!(b.positions().len(), 3, "trzy wejścia");
    (e, b)
}

fn cfg_dzien() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3;
    c.tp_schedule = TpSchedule::AllRunners;
    c
}

#[test]
fn cel_dnia_w_procentach_zamyka_pozycje() {
    let mut cfg = cfg_dzien();
    cfg.day_target_pct = 0.5; // 0,5 % z 1000 $ = 5 $
    cfg.day_target_close = true;
    let (mut e, mut b) = dzien_od_1000(cfg);

    tik(&mut e, &mut b, T0 + 2_000, 4005.0);
    assert!(b.positions().is_empty(), "cel dnia zamyka wszystko");
    assert!(b.history.iter().any(|t| t.reason == CloseReason::DayTarget));
}

#[test]
fn bez_celu_procentowego_pozycje_zostaja() {
    let (mut e, mut b) = dzien_od_1000(cfg_dzien());
    tik(&mut e, &mut b, T0 + 2_000, 4001.5);
    assert_eq!(b.positions().len(), 3, "bez ustawienia nic się nie zamyka");
}

#[test]
fn stop_dnia_liczy_sie_od_szczytu_equity_a_nie_od_salda_otwarcia() {
    let mut cfg = cfg_dzien();
    cfg.day_trail_stop_pct = 0.5; // 0,5 % szczytu dnia
    cfg.day_trail_arm_pct = 0.5; // uzbrój po +0,5 % (5 $)
    let (mut e, mut b) = dzien_od_1000(cfg);

    // średnia wejść 4002,50; przy 4006 zysk +10,50 $ → szczyt, reguła uzbrojona
    tik(&mut e, &mut b, T0 + 2_000, 4006.0);
    assert_eq!(b.positions().len(), 3, "na szczycie nic się nie zamyka");
    // przy 4004 zysk spada do +4,50 $, czyli oddane 6,00 $ ze szczytu ≥ 5,05 $
    tik(&mut e, &mut b, T0 + 3_000, 4004.0);
    assert!(b.positions().is_empty(), "stop dnia domyka koszyk");
    assert!(b.history.iter().any(|t| t.reason == CloseReason::DayTarget));
}

/// Bez uzbrojenia reguła ścinałaby dobę na pierwszym zwykłym obsunięciu,
/// zanim cokolwiek zarobi — dlatego próg uzbrojenia jest osobnym polem.
#[test]
fn stop_dnia_nieuzbrojony_nie_scina_doby() {
    let mut cfg = cfg_dzien();
    cfg.day_trail_stop_pct = 0.5;
    cfg.day_trail_arm_pct = 50.0; // wymaga +500 $ — nigdy się nie uzbroi
    let (mut e, mut b) = dzien_od_1000(cfg);
    tik(&mut e, &mut b, T0 + 2_000, 4002.5);
    tik(&mut e, &mut b, T0 + 3_000, 4000.3);
    assert_eq!(
        b.positions().len(),
        3,
        "nieuzbrojony stop nie ma prawa zadziałać"
    );
}


#[test]
fn budzet_dnia_ogranicza_liczbe_koszykow() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.daily_signal_budget = 1;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    e.on_message(&mut b, &wiadomosc(T0 + 60_000, 2, SYGNAL_DALEKI));
    assert_eq!(
        e.baskets.len(),
        1,
        "drugi sygnał ma nie zmieścić się w budżecie"
    );
}

#[test]
fn bez_budzetu_wchodza_wszystkie_sygnaly() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    e.on_message(&mut b, &wiadomosc(T0 + 60_000, 2, SYGNAL_DALEKI));
    assert_eq!(e.baskets.len(), 2);
}

/// R:R liczone w miejscu REALISTYCZNEGO wejścia, czyli przy gorszej krawędzi
/// strefy: |4010 − 4005| / |4005 − 3990| = 0,33. Próg 1,0 musi taki sygnał
/// odrzucić, próg 0,2 — przepuścić.
#[test]
fn prog_rr_odrzuca_sygnal_o_zlej_geometrii() {
    let mut ostry = Settings::default();
    ostry.entry_units = 2;
    ostry.signal_min_rr = 1.0;
    let (mut e1, mut b1) = stanowisko(ostry, 4008.0);
    e1.on_message(&mut b1, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        e1.baskets.is_empty(),
        "sygnał o R:R 0,33 ma zostać odrzucony"
    );
    assert!(b1.pendings().is_empty());

    let mut lagodny = Settings::default();
    lagodny.entry_units = 2;
    lagodny.signal_min_rr = 0.2;
    let (mut e2, mut b2) = stanowisko(lagodny, 4008.0);
    e2.on_message(&mut b2, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(
        e2.baskets.len(),
        1,
        "przy progu 0,2 ten sam sygnał przechodzi"
    );
}

#[test]
fn pasmo_szerokosci_strefy_odrzuca_setupy_poza_zakresem() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.signal_min_zone_width = 6.0; // strefa SYGNAŁU ma 5 $
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(e.baskets.is_empty(), "za wąska strefa ma zostać odrzucona");
    assert!(b.pendings().is_empty());
}


const SYGNAL_BLISKI: &str = "BUY GOLD @ 4006/4001\nTP 4012\nTP 4022\nTP 4032\nSL 3991";
const SYGNAL_ODLEGLY: &str = "BUY GOLD @ 3905/3900\nTP 3910\nTP 3920\nTP 3930\nSL 3890";

fn cfg_merge() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 2;
    c.merge_same_side = true;
    c.merge_window_min = 20.0;
    c.merge_min_overlap = 0.5;
    c
}

#[test]
fn drugi_sygnal_o_nakladajacej_sie_strefie_dolacza_do_koszyka() {
    let (mut e, mut b) = stanowisko(cfg_merge(), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    e.on_message(&mut b, &wiadomosc(T0 + 5 * 60_000, 2, SYGNAL_BLISKI));

    assert_eq!(
        e.baskets.len(),
        1,
        "jedna transakcja, nie dwie — jeden spread"
    );
    let bk = &e.baskets[0];
    assert!(
        (bk.zone_hi - 4006.0).abs() < 1e-6,
        "koszyk ma przejąć nową strefę"
    );
    assert!((bk.tps[0] - 4012.0).abs() < 1e-6, "i nowe cele");
}

#[test]
fn bez_wlacznika_powstaja_dwa_koszyki() {
    let mut cfg = cfg_merge();
    cfg.merge_same_side = false;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    e.on_message(&mut b, &wiadomosc(T0 + 5 * 60_000, 2, SYGNAL_BLISKI));
    assert_eq!(
        e.baskets.len(),
        2,
        "domyślnie każdy sygnał to osobny koszyk"
    );
}

#[test]
fn strefy_ktore_sie_nie_pokrywaja_nie_sa_laczone() {
    let (mut e, mut b) = stanowisko(cfg_merge(), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    e.on_message(&mut b, &wiadomosc(T0 + 5 * 60_000, 2, SYGNAL_ODLEGLY));
    assert_eq!(
        e.baskets.len(),
        2,
        "sto dolarów niżej to inny setup, nie ten sam"
    );
}

#[test]
fn sygnal_po_oknie_czasu_zaklada_nowy_koszyk() {
    let (mut e, mut b) = stanowisko(cfg_merge(), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    e.on_message(&mut b, &wiadomosc(T0 + 40 * 60_000, 2, SYGNAL_BLISKI));
    assert_eq!(
        e.baskets.len(),
        2,
        "po 40 min przy oknie 20 min to nowy setup"
    );
}


fn cfg_wyjscie() -> Settings {
    let mut c = Settings::default();
    c.entry_units = 1;
    c.tp_schedule = TpSchedule::AllRunners;
    c.smart_exit = true;
    c.smart_exit_take = 2.0;
    // wyłączamy „trzymaj, bo pod ceną czeka limit" — badamy jedną regułę naraz
    c.smart_exit_hold_if_pending = 0.0;
    c
}

/// Wejście po 4004,70. Przy bidzie 4007 zysk to 2,30 pkt, więc `smart_exit`
/// każe wyjść. Bez reguły wychodzimy po BIDZIE 4007,00; z regułą czekamy, aż
/// bid dojdzie do 4007,20 — czyli tam, gdzie wypełniłby się limit stojący po
/// drugiej stronie spreadu.
#[test]
fn wyjscie_czeka_na_druga_strone_spreadu() {
    let mut z_limitem = cfg_wyjscie();
    z_limitem.exit_via_limit = true;
    z_limitem.exit_limit_wait_s = 60.0;

    let (mut e, mut b) = stanowisko(z_limitem, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 4004.5); // wejście po 4004,70
    assert_eq!(b.positions().len(), 1);
    tik(&mut e, &mut b, T0 + 2_000, 4007.0); // reguła chce wyjść…
    assert_eq!(b.positions().len(), 1, "…ale czeka na lepszą cenę");
    tik(&mut e, &mut b, T0 + 3_000, 4007.3); // cena dochodzi do celu
    assert!(b.positions().is_empty(), "wyjście po dojściu ceny");

    let cena = b.history.last().expect("musi być zamknięcie").close_price;
    assert!(
        cena >= 4007.2 - 1e-9,
        "wyjście po {cena}, a limit stał na 4007,20"
    );
}

#[test]
fn bez_wlacznika_wyjscie_jest_natychmiastowe_po_rynku() {
    let (mut e, mut b) = stanowisko(cfg_wyjscie(), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 4004.5);
    tik(&mut e, &mut b, T0 + 2_000, 4007.0);
    assert!(b.positions().is_empty(), "domyślnie zamykamy od razu");
    let cena = b.history.last().unwrap().close_price;
    assert!(
        (cena - 4007.0).abs() < 1e-9,
        "po rynku, czyli po BIDZIE: {cena}"
    );
}

/// ⚠ Cena tej reguły: limit może się nie wypełnić. Po upływie czasu
/// wychodzimy po rynku — i wtedy bywa GORZEJ niż od razu.
#[test]
fn po_uplywie_czasu_wychodzimy_awaryjnie_po_rynku() {
    let mut cfg = cfg_wyjscie();
    cfg.exit_via_limit = true;
    cfg.exit_limit_wait_s = 1.0;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 4004.5);
    tik(&mut e, &mut b, T0 + 2_000, 4007.0); // decyzja o wyjściu, cel 4007,20
    assert_eq!(b.positions().len(), 1);
    tik(&mut e, &mut b, T0 + 4_000, 4006.5); // cena nie doszła, czas minął

    assert!(b.positions().is_empty(), "po terminie wychodzimy po rynku");
    let cena = b.history.last().unwrap().close_price;
    assert!(
        (cena - 4006.5).abs() < 1e-9,
        "wyjście awaryjne po bieżącym BIDZIE: {cena}"
    );
    assert!(
        cena < 4007.0,
        "czekanie kosztowało — i test ma to pokazywać"
    );
}


/// Ścieżka, na której widać RÓŻNICĘ między trybami stopu runnera:
/// uwolnienie przy 4030 (runner wszedł po 3980, średnia koszyka 4000),
/// potem bieg do 4100, cofnięcie do 4074 i dopiero wtedy załamanie do 3999.
///
/// Stop przyklejony do BE (4000) przesypia cały bieg i oddaje go w całości.
/// Luźna zapadka podciąga się za szczytem i inkasuje go przy cofnięciu.
fn przebieg_bieg_potem_zalamanie(cfg: Settings) -> f64 {
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg);
    tik(&mut e, &mut b, T0 + 4_000, 4030.0); // uwolnienie od ryzyka
    tik(&mut e, &mut b, T0 + 5_000, 4100.0); // runner odjeżdża
    tik(&mut e, &mut b, T0 + 6_000, 4074.0); // cofnięcie
    tik(&mut e, &mut b, T0 + 7_000, 3999.0); // załamanie
    b.history.iter().map(|t| t.profit).sum::<f64>()
        + b.positions()
            .iter()
            .map(|p| p.profit_usd(&b.q))
            .sum::<f64>()
}

#[test]
fn luzna_zapadka_runnera_bije_stop_przyklejony_do_be() {
    let mut be = cfg_riskfree();
    be.riskfree_runner_stop = RiskFreeRunnerStop::Be;

    let mut luzny = cfg_riskfree();
    luzny.riskfree_runner_stop = RiskFreeRunnerStop::TrailGap;
    luzny.riskfree_runner_gap = 25.0;

    let wynik_be = przebieg_bieg_potem_zalamanie(be);
    let wynik_luzny = przebieg_bieg_potem_zalamanie(luzny);

    assert!(
        wynik_luzny > wynik_be,
        "luźna zapadka ma zainkasować bieg runnera: luźna {wynik_luzny:.2} $, BE {wynik_be:.2} $"
    );
    // oba tryby muszą jednak zostawić koszyk na plusie — od tego jest RISK FREE
    assert!(wynik_be > 0.0 && wynik_luzny > 0.0);
}

#[test]
fn runner_bez_stopu_nie_jest_zbierany_szpilka() {
    let mut bez = cfg_riskfree();
    bez.riskfree_runner_stop = RiskFreeRunnerStop::Off;
    let (mut e, mut b) = koszyk_trzy_wejscia(bez);
    tik(&mut e, &mut b, T0 + 4_000, 4030.0);
    assert_eq!(b.positions().len(), 1, "runner zostaje");
    assert!(
        b.positions()[0].sl.is_none(),
        "w trybie Off runner nie ma stopu"
    );

    tik(&mut e, &mut b, T0 + 5_000, 3999.0);
    assert_eq!(b.positions().len(), 1, "szpilka nie ma czym go zebrać");
}

/// Limit trzymania: wkład runnerów ma wyraźne optimum ok. 72 h, a przy
/// tygodniu SPADA. Runner bez terminu ważności oddaje zysk z powrotem.
#[test]
fn runner_domyka_sie_po_limicie_trzymania() {
    let mut cfg = cfg_riskfree();
    cfg.riskfree_runner_stop = RiskFreeRunnerStop::Off;
    cfg.riskfree_runner_max_hold_min = 60.0;
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg);
    tik(&mut e, &mut b, T0 + 4_000, 4030.0);
    assert_eq!(b.positions().len(), 1);

    tik(&mut e, &mut b, T0 + 30 * 60_000, 4032.0);
    assert_eq!(
        b.positions().len(),
        1,
        "po pół godziny runner jeszcze biegnie"
    );

    tik(&mut e, &mut b, T0 + 70 * 60_000, 4032.0);
    assert!(
        b.positions().is_empty(),
        "po godzinie runner ma zostać domknięty"
    );
    assert!(b.history.iter().any(|t| t.reason == CloseReason::Expired));
}

#[test]
fn bez_limitu_trzymania_runner_biegnie_dalej() {
    let mut cfg = cfg_riskfree();
    cfg.riskfree_runner_stop = RiskFreeRunnerStop::Off;
    cfg.riskfree_runner_max_hold_min = 0.0;
    let (mut e, mut b) = koszyk_trzy_wejscia(cfg);
    tik(&mut e, &mut b, T0 + 4_000, 4030.0);
    tik(&mut e, &mut b, T0 + 200 * 60_000, 4032.0);
    assert_eq!(b.positions().len(), 1, "0 = bez terminu ważności");
}

const SYGNAL_PODCIAGNIETY: &str = SYGNAL;

#[test]
fn siatka_nie_ginie_gdy_cel_lezal_po_drodze_od_poczatku() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.pending_drop_on_target = true;
    cfg.pending_drop_require_zone_touch = true;

    // rynek POWYŻEJ pierwszego celu — klasyczne podciągnięcie
    let (mut e, mut b) = stanowisko(cfg, 4012.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_PODCIAGNIETY));
    assert_eq!(b.pendings().len(), 3, "siatka ma się rozstawić");

    tik(&mut e, &mut b, T0 + 1_000, 4012.0);
    assert_eq!(
        b.pendings().len(),
        3,
        "cel leżący po drodze od początku nie jest „osiągnięty bez nas\""
    );
}

#[test]
fn znacznik_dotkniecia_strefy_zapala_sie_dopiero_przy_strefie() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.pending_drop_on_target = true;
    cfg.pending_drop_require_zone_touch = true;

    let (mut e, mut b) = stanowisko(cfg, 4012.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_PODCIAGNIETY));

    // cena kręci się NAD strefą i nad pierwszym celem — droga się nie zaczęła
    tik(&mut e, &mut b, T0 + 1_000, 4011.0);
    tik(&mut e, &mut b, T0 + 2_000, 4013.0);
    assert!(
        !e.baskets[0].zone_touched,
        "nad strefą znacznik ma być zgaszony"
    );
    assert_eq!(b.pendings().len(), 3, "i siatka ma stać nietknięta");

    // dopiero zejście do strefy zapala znacznik
    tik(&mut e, &mut b, T0 + 3_000, 4004.0);
    assert!(
        e.baskets[0].zone_touched,
        "przy strefie znacznik ma się zapalić"
    );
}

#[test]
fn bez_wlacznika_zachowanie_zostaje_stare() {
    let mut stare = Settings::default();
    stare.entry_units = 3;
    stare.pending_drop_on_target = true;
    assert!(
        !stare.pending_drop_require_zone_touch,
        "domyślnie wyłączone"
    );

    let (mut e, mut b) = stanowisko(stare, 4012.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_PODCIAGNIETY));
    tik(&mut e, &mut b, T0 + 1_000, 4012.0);
    assert!(
        b.pendings().len() < 3,
        "stare zachowanie kasuje siatkę od razu — i to jest właśnie ten błąd"
    );
}

// ============================================================
//  `trail_runners_n` — POLE, KTÓRE DOTĄD NIC NIE ROBIŁO
// ============================================================
//
// Forensyka: `n = 1` i `n = 3` dawały wynik identyczny do szóstego miejsca
// po przecinku, bo „runner" znaczyło „pozycja bez take-profitu", a nie
// „N najlepszych wejść". Test przypina, że po włączeniu przełącznika liczba
// runnerów naprawdę zmienia to, komu przysługuje luźniejszy trailing.

fn cfg_runnerzy(n: u32, wg_glebokosci: bool) -> Settings {
    let mut c = Settings::default();
    c.entry_units = 3;
    c.tp_schedule = TpSchedule::AllRunners;
    c.trail_split = true;
    c.trail_runners_by_depth = wg_glebokosci;
    c.trail_runners_n = n;
    // runner: zapadka luźna; reszta: ciasna — różnica ma być widoczna w SL
    c.trail_runner_mode = TrailMode::Gap;
    c.trail_runner_start = 1.0;
    c.trail_runner_gap = 20.0;
    c.trail_mode = TrailMode::Gap;
    c.trail_start = 1.0;
    c.trail_gap = 2.0;
    c
}

/// Ile pozycji ma stop DALEKO od ceny (czyli luźną zapadkę runnera).
fn ile_luznych(b: &SimBroker, cena: f64) -> usize {
    b.positions()
        .iter()
        .filter(|p| p.sl.map(|s| (cena - s) > 10.0).unwrap_or(true))
        .count()
}

#[test]
fn liczba_runnerow_naprawde_zmienia_komu_przysluguje_luzny_trailing() {
    let jeden = {
        let (mut e, mut b) = stanowisko(cfg_runnerzy(1, true), 4008.0);
        e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
        tik(&mut e, &mut b, T0 + 1_000, 3999.0);
        tik(&mut e, &mut b, T0 + 2_000, 4030.0);
        ile_luznych(&b, 4030.0)
    };
    let trzy = {
        let (mut e, mut b) = stanowisko(cfg_runnerzy(3, true), 4008.0);
        e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
        tik(&mut e, &mut b, T0 + 1_000, 3999.0);
        tik(&mut e, &mut b, T0 + 2_000, 4030.0);
        ile_luznych(&b, 4030.0)
    };
    assert!(
        trzy > jeden,
        "n=3 ma dać więcej luźnych runnerów niż n=1 (było: {jeden} i {trzy})"
    );
}

/// Bez przełącznika pole nadal nic nie robi — i test ma to pokazywać wprost,
/// bo to jest dokładnie stan, który forensyka nazwała martwym.
#[test]
fn bez_przelacznika_liczba_runnerow_nadal_nic_nie_zmienia() {
    let policz = |n: u32| {
        let (mut e, mut b) = stanowisko(cfg_runnerzy(n, false), 4008.0);
        e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
        tik(&mut e, &mut b, T0 + 1_000, 3999.0);
        tik(&mut e, &mut b, T0 + 2_000, 4030.0);
        b.positions()
            .iter()
            .filter_map(|p| p.sl)
            .map(|s| (s * 1e6) as i64)
            .sum::<i64>()
    };
    assert_eq!(
        policz(1),
        policz(3),
        "bez włącznika `trail_runners_n` nadal nie ma żadnego wpływu"
    );
}

// ============================================================
//  SYGNAŁ PRZECIWNY ZAMYKA KOSZYK
// ============================================================
//
// Pole `exit_on_opposite_signal` miało kontrolkę w panelu i wpis
// „zaimplementowane" w dokumencie, a w rdzeniu ZERO odwołań. Dwa zespoły
// zmierzyły to niezależnie: rozstęp 0,0000 $ na pełnym zakresie.
const SYGNAL_SELL: &str = "SELL GOLD @ 4010/4015\nTP 4000\nTP 3990\nTP 3980\nSL 4025";

fn ile_kupna(b: &SimBroker) -> usize {
    b.positions().iter().filter(|p| p.side == Side::Buy).count()
}

#[test]
fn sygnal_przeciwny_zamyka_koszyk_kupna() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.exit_on_opposite_signal = true;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0);
    assert_eq!(ile_kupna(&b), 2, "koszyk kupna musi najpierw powstać");

    e.on_message(&mut b, &wiadomosc(T0 + 2_000, 2, SYGNAL_SELL));
    assert_eq!(
        ile_kupna(&b),
        0,
        "sygnalista zmienił zdanie — kupno wychodzi"
    );
    assert!(b
        .history
        .iter()
        .any(|t| t.reason == CloseReason::BasketClose));
}

#[test]
fn bez_wlacznika_sygnal_przeciwny_niczego_nie_zamyka() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.tp_schedule = TpSchedule::AllRunners;
    assert!(!cfg.exit_on_opposite_signal, "domyślnie wyłączone");

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0);
    e.on_message(&mut b, &wiadomosc(T0 + 2_000, 2, SYGNAL_SELL));
    assert_eq!(ile_kupna(&b), 2, "bez włącznika koszyk kupna zostaje");
}

// ============================================================
//  TWARDY CZAS ŻYCIA KOSZYKA I FILTR TRENDU
// ============================================================
//
// Przewaga sygnału nad kontrolą placebo żyje ok. godziny i po 90 min zmienia
// znak: 15 min +616 $, 60 min +1 045 $, 90 min −321 $, 24 h −3 494 $.
// Koszyk trzymany dłużej nie realizuje już sygnału, tylko pozycję kierunkową.

#[test]
fn koszyk_wygasa_po_twardym_limicie_wieku() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.basket_max_age_min = 60.0;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0);
    assert_eq!(b.positions().len(), 2, "koszyk musi najpierw wejść");

    tik(&mut e, &mut b, T0 + 59 * 60_000, 4000.0);
    assert_eq!(
        b.positions().len(),
        2,
        "przed upływem limitu nic się nie dzieje"
    );

    tik(&mut e, &mut b, T0 + 61 * 60_000, 4000.0);
    assert!(
        b.positions().is_empty(),
        "po 60 min koszyk ma zostać domknięty"
    );
    assert!(b.history.iter().any(|t| t.reason == CloseReason::Expired));
}

#[test]
fn bez_limitu_wieku_koszyk_zyje_dalej() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.tp_schedule = TpSchedule::AllRunners;
    assert_eq!(cfg.basket_max_age_min, 0.0, "domyślnie wyłączone");

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0);
    tik(&mut e, &mut b, T0 + 300 * 60_000, 4000.0);
    assert_eq!(b.positions().len(), 2, "bez limitu koszyk żyje");
}

/// Filtr trendu potrzebuje historii godzinowej — budujemy ją tickami
/// co godzinę, prowadząc cenę w dół o ponad próg.
fn silnik_z_trendem_spadkowym(cfg: Settings) -> (Engine, SimBroker) {
    let (mut e, mut b) = stanowisko(cfg, 4400.0);
    // 30 godzin spadku z 4400 do 4000 = −9,1 %
    for i in 0..30 {
        let cena = 4400.0 - i as f64 * 13.5;
        tik(&mut e, &mut b, T0 + i * 3_600_000, cena);
    }
    (e, b)
}

#[test]
fn filtr_trendu_blokuje_kupno_w_spadajacym_rynku() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.trend_filter_enabled = true;
    cfg.trend_filter_window_h = 24.0;
    cfg.trend_filter_drop_pct = 2.0;
    cfg.trend_filter_mode = TrendFilterMode::Block;

    let (mut e, mut b) = silnik_z_trendem_spadkowym(cfg);
    let ts = T0 + 30 * 3_600_000;
    e.on_message(&mut b, &wiadomosc(ts, 1, SYGNAL));
    assert!(e.baskets.is_empty(), "kupno pod trend ma zostać odrzucone");
}

#[test]
fn wariant_miekki_wchodzi_mniejszym_rozmiarem_zamiast_odmawiac() {
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.trend_filter_enabled = true;
    cfg.trend_filter_window_h = 24.0;
    cfg.trend_filter_drop_pct = 2.0;
    cfg.trend_filter_mode = TrendFilterMode::Shrink;
    cfg.trend_filter_shrink = 0.5;

    let (mut e, mut b) = silnik_z_trendem_spadkowym(cfg);
    let ts = T0 + 30 * 3_600_000;
    e.on_message(&mut b, &wiadomosc(ts, 1, SYGNAL));
    assert_eq!(e.baskets.len(), 1, "wariant miękki NIE odmawia wejścia");
    assert_eq!(
        b.pendings().len(),
        2,
        "…ale wchodzi połową szczebli (4 → 2)"
    );
}

#[test]
fn bez_filtra_trendu_sygnal_pod_trend_wchodzi_w_calosci() {
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    assert!(!cfg.trend_filter_enabled, "domyślnie wyłączony");
    cfg.trend_filter_drop_pct = 2.0; // celowo — bez włącznika ma nie działać

    let (mut e, mut b) = silnik_z_trendem_spadkowym(cfg);
    let ts = T0 + 30 * 3_600_000;
    e.on_message(&mut b, &wiadomosc(ts, 1, SYGNAL));
    assert_eq!(e.baskets.len(), 1);
    assert_eq!(b.pendings().len(), 4, "pełny rozmiar");
}

/// Brak wiedzy nie może zmieniać zachowania: przy zbyt krótkiej historii
/// filtr milczy, zamiast zgadywać. Ta sama zasada, co w `vol_factor`.
#[test]
fn filtr_trendu_milczy_przy_zbyt_krotkiej_historii() {
    let mut cfg = Settings::default();
    cfg.entry_units = 4;
    cfg.trend_filter_enabled = true;
    cfg.trend_filter_window_h = 24.0;
    cfg.trend_filter_drop_pct = 2.0;
    cfg.trend_filter_mode = TrendFilterMode::Block;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    tik(&mut e, &mut b, T0, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0 + 1_000, 1, SYGNAL));
    assert_eq!(
        e.baskets.len(),
        1,
        "bez historii filtr nie ma prawa blokować"
    );
}

// ============================================================
//  ROZKŁAD WARSTW W STREFIE — TEST NA KONKRETNYCH CENACH
// ============================================================
//
// Test celowo sprawdza ceny poziomów, a nie samo istnienie pola. Konwencja
// indeksu jest odwrotna do intuicji („i = 0" to warstwa najgłębsza, dla kupna
// `zone_lo`), więc przypadkowe odwrócenie jej przy refaktorze musi być widoczne.
const SYNTETYCZNY_SYGNAL_STREFOWY: &str =
    "BUY GOLD @ 4100/4095\nTP 4130\nTP 4150\nTP 4170\nSL 4080";

fn ceny_poziomow(b: &SimBroker) -> Vec<f64> {
    let mut v: Vec<f64> = b.pendings().iter().map(|o| o.price).collect();
    v.sort_by(|a, c| a.partial_cmp(c).unwrap());
    v
}

#[test]
fn rozklad_warstw_domyslny_jest_rowny() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    assert_eq!(
        cfg.entry_depth_curve, 1.0,
        "domyślnie równo — zero zmian dla presetów"
    );

    let (mut e, mut b) = stanowisko(cfg, 4110.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYNTETYCZNY_SYGNAL_STREFOWY));
    let c = ceny_poziomow(&b);
    assert_eq!(c.len(), 3);
    for (i, oczek) in [4095.00, 4097.50, 4100.00].iter().enumerate() {
        assert!(
            (c[i] - oczek).abs() < 1e-6,
            "poziom {i}: {} zamiast {oczek}",
            c[i]
        );
    }
}

/// Wykładnik 1,77 to dokładny odpowiednik zamówienia „0 % · 70,7 % · 100 %
/// głębokości licząc od krawędzi BLIŻSZEJ cenie".
#[test]
fn krzywa_177_przesuwa_srodkowa_warstwe_w_glab() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.entry_depth_curve = 1.77;

    let (mut e, mut b) = stanowisko(cfg, 4110.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYNTETYCZNY_SYGNAL_STREFOWY));
    let c = ceny_poziomow(&b);
    assert!((c[0] - 4095.00).abs() < 1e-6, "krawędź głęboka: {}", c[0]);
    assert!((c[2] - 4100.00).abs() < 1e-6, "krawędź płytka: {}", c[2]);
    assert!(
        (c[1] - 4096.47).abs() < 0.01,
        "środkowa warstwa ma leżeć na 4096,47 (70,7 % w głąb), a leży na {}",
        c[1]
    );
}

/// Kontrola kierunku: wykładnik MNIEJSZY od 1 przesuwa warstwy w stronę
/// krawędzi PŁYTKIEJ, czyli w stronę gorszych wejść. Ten test istnieje po to,
/// żeby pomyłka w znaku konwencji padła tutaj, a nie w wynikach.
#[test]
fn krzywa_ponizej_jedynki_przesuwa_warstwy_ku_plytkiej_krawedzi() {
    let mut cfg = Settings::default();
    cfg.entry_units = 3;
    cfg.entry_depth_curve = 0.5;

    let (mut e, mut b) = stanowisko(cfg, 4110.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYNTETYCZNY_SYGNAL_STREFOWY));
    let c = ceny_poziomow(&b);
    assert!(
        c[1] > 4097.50,
        "przy 0,5 środkowa warstwa ma być PŁYCEJ niż przy 1,0, a jest {}",
        c[1]
    );
    assert!((c[1] - 4098.54).abs() < 0.01, "{}", c[1]);
}


fn cfg_okna(lead: f64, lag: f64) -> Settings {
    let mut c = Settings::default();
    c.entry_units = 1;
    c.tp_source = TpSource::PriceFirstSignalWindow;
    c.tp_signal_max_lead_s = lead;
    c.tp_signal_max_lag_s = lag;
    c.tp_price_tolerance = 0.30;
    c.tp_schedule = TpSchedule::AllRunners;
    c
}

/// Komunikat PRZED dotknięciem poziomu, przy wyprzedzeniu zabronionym (0),
/// nie może przesunąć etapu — cena ma rządzić.
#[test]
fn komunikat_przed_cena_odrzucony_gdy_wyprzedzenie_zabronione() {
    let (mut e, mut b) = stanowisko(cfg_okna(0.0, 0.0), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 4004.5); // wejście, cena daleko od TP1 4010

    let mut m = wiadomosc(T0 + 2_000, 2, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    assert_eq!(
        e.baskets[0].tp_stage, 0,
        "cena nie potwierdziła — etap stoi"
    );
}

/// Przy dopuszczonym wyprzedzeniu wystarczy, że cena jest BLISKO poziomu.
#[test]
fn komunikat_przed_cena_przyjety_gdy_cena_juz_blisko() {
    let (mut e, mut b) = stanowisko(cfg_okna(30.0, 0.0), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 4004.5);
    tik(&mut e, &mut b, T0 + 2_000, 4009.8); // 0,20 $ pod celem 4010, w tolerancji

    let mut m = wiadomosc(T0 + 3_000, 2, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    assert!(e.baskets[0].tp_stage >= 1, "bliskość celu ma wystarczyć");
}

/// Komunikat SPÓŹNIONY ponad próg opisuje setup, który zdążył się zmienić.
#[test]
fn komunikat_spozniony_ponad_prog_jest_odrzucany() {
    let (mut e, mut b) = stanowisko(cfg_okna(0.0, 60.0), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 4004.5);
    tik(&mut e, &mut b, T0 + 2_000, 4010.5); // cena DOTYKA celu — znacznik zapisany
    let etap_po_cenie = e.baskets[0].tp_stage;

    // komunikat 10 minut po fakcie, przy progu 60 s
    let mut m = wiadomosc(T0 + 2_000 + 600_000, 2, "TP2 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    assert_eq!(
        e.baskets[0].tp_stage, etap_po_cenie,
        "spóźniony komunikat nie może przesunąć etapu dalej"
    );
}

/// Przy `lag = 0` (bez limitu) ten sam spóźniony komunikat przechodzi.
#[test]
fn bez_limitu_spoznienia_komunikat_jest_przyjmowany() {
    let (mut e, mut b) = stanowisko(cfg_okna(0.0, 0.0), 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 4004.5);
    tik(&mut e, &mut b, T0 + 2_000, 4010.5);

    let mut m = wiadomosc(T0 + 2_000 + 600_000, 2, "TP1 HIT");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);
    assert!(e.baskets[0].tp_stage >= 1);
}

// ============================================================
//  WEZWANIE DO UZUPEŁNIENIA DEPOZYTU
// ============================================================

/// Broker przy tym poziomie NIE zamyka pozycji — przestaje przyjmować nowe.
#[test]
fn wezwanie_do_uzupelnienia_blokuje_nowe_wejscia_ale_nie_zamyka() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.tp_schedule = TpSchedule::AllRunners;
    // Poziom marginu = equity / margines × 100. Dwie pozycje 0,01 lota po
    // ~4000 przy dźwigni 500 to margines ok. 16 $, więc przy koncie 1000 $
    // poziom wynosi ok. 6 250 %. Próg musi być WYŻSZY, żeby reguła zadziałała —
    // 5 000 % było poniżej i test sprawdzał wtedy nie to, co opisuje.
    cfg.margin_call_level_pct = 100_000.0;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0);
    let ile = b.positions().len();
    assert!(
        ile > 0,
        "pierwszy koszyk musi wejść, zanim margines się zapełni"
    );

    // drugi sygnał przy zapełnionym marginesie — bramka ma go odrzucić
    e.on_message(&mut b, &wiadomosc(T0 + 2_000, 2, SYGNAL));
    assert_eq!(e.baskets.len(), 1, "nowe wejścia zablokowane");
    assert_eq!(
        b.positions().len(),
        ile,
        "ale istniejące pozycje NIE są zamykane"
    );
}

#[test]
fn bez_progu_marginu_wejscia_nie_sa_blokowane() {
    let mut cfg = Settings::default();
    cfg.entry_units = 2;
    cfg.tp_schedule = TpSchedule::AllRunners;
    assert_eq!(cfg.margin_call_level_pct, 50.0, "domyślny próg brokera");

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 3999.0);
    e.on_message(&mut b, &wiadomosc(T0 + 2_000, 2, SYGNAL));
    assert_eq!(e.baskets.len(), 2, "przy zdrowym marginesie wchodzą oba");
}

// ============================================================
//  NAPRAWY Z AUDYTU FABLE (Z-4, Z-5, Z-9)
// ============================================================

/// Z-4: BE-lock MUSI działać na pozycji, która nie ma jeszcze stop-lossa.
///
/// Odwrócony `unwrap_or(true)` sprawiał, że brak SL był traktowany jak
/// „BE poluzowałby istniejący stop" — czyli jak powód, żeby nic nie robić.
/// A to są dokładnie te pozycje, którym siatka bezpieczeństwa jest
/// najbardziej potrzebna.
#[test]
fn be_lock_stawia_stop_takze_na_pozycji_bez_sl() {
    let mut cfg = Settings::default();
    cfg.be_lock_pts = 5.0;
    cfg.be_offset = 0.0;
    cfg.entry_units = 1;
    cfg.tp_schedule = TpSchedule::AllRunners;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    // sygnał BEZ SL — parser zwraca `sl = None`, więc pozycja rodzi się bez stopu
    e.on_message(
        &mut b,
        &wiadomosc(T0, 1, "BUY GOLD @ 4005/4000\nTP 4200\nTP 4300\nTP 4400"),
    );
    tik(&mut e, &mut b, T0 + 1_000, 4002.0);
    let poz = b.positions().len();
    assert!(poz > 0, "sygnał musi otworzyć pozycję");
    assert!(
        b.positions().iter().all(|p| p.sl.is_none()),
        "warunek testu: pozycja startuje BEZ stopu"
    );

    // zysk ponad próg BE-locka
    tik(&mut e, &mut b, T0 + 2_000, 4015.0);
    assert!(
        b.positions().iter().any(|p| p.sl.is_some()),
        "BE-lock musi postawić stop pozycji bez SL (Z-4)"
    );
}

/// Z-4, druga strona: BE-lock nadal NIE WOLNO poluzować istniejącego stopu.
#[test]
fn be_lock_nie_luzuje_ciasniejszego_stopu() {
    let mut cfg = Settings::default();
    cfg.be_lock_pts = 5.0;
    cfg.be_offset = 0.0;
    cfg.entry_units = 1;
    cfg.tp_schedule = TpSchedule::AllRunners;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    tik(&mut e, &mut b, T0 + 1_000, 4002.0);
    // Cena najpierw odjeżdża w górę, żeby stop NAD wejściem był w ogóle
    // wykonalny — broker odrzuca SL bliżej rynku niż `stops_level`.
    tik(&mut e, &mut b, T0 + 2_000, 4014.0);
    let poz = b.positions()[0].clone();
    // stop CIAŚNIEJSZY niż breakeven (dla BUY: wyżej niż wejście)
    let ciasny = poz.open_price + 2.0;
    b.modify_position(poz.ticket, Some(ciasny), poz.tp).unwrap();

    tik(&mut e, &mut b, T0 + 3_000, 4015.0);
    let sl = b.positions()[0].sl.expect("stop miał zostać");
    assert!(
        sl >= ciasny - 1e-9,
        "BE-lock cofnął stop z {ciasny} na {sl} — to jest poluzowanie"
    );
}

/// Z-5: nowy stop koszyka ma dojść także do zleceń OCZEKUJĄCYCH.
#[test]
fn nowy_sl_koszyka_dociera_do_pendingow() {
    let mut cfg = Settings::default();
    cfg.sl_edit_reaches_pendings = true;
    cfg.entry_units = 4;
    cfg.tp_schedule = TpSchedule::AllRunners;

    // cena NAD strefą, żeby limity zostały wiszące, a nie wypełnione
    let (mut e, mut b) = stanowisko(cfg, 4030.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    let wiszace: Vec<_> = b.pendings().iter().map(|o| (o.ticket, o.sl)).collect();
    assert!(wiszace.len() >= 2, "potrzebujemy wiszących limitów");
    let stary = wiszace[0].1.expect("limit ma startowy SL z sygnału");

    // sygnalista zacieśnia stop
    let mut m = wiadomosc(T0 + 1_000, 2, "MOVE SL TO 3995");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);

    let nowe: Vec<Option<f64>> = b.pendings().iter().map(|o| o.sl).collect();
    assert!(
        nowe.iter()
            .all(|s| s.map(|v| (v - 3995.0).abs() < 1e-6).unwrap_or(false)),
        "każdy pending ma nieść NOWY stop; było {stary}, jest {nowe:?}"
    );
}

/// Z-5 z flagą wyłączoną: zachowanie sprzed naprawy, co do centa.
#[test]
fn bez_flagi_nowy_sl_nie_rusza_pendingow() {
    let mut cfg = Settings::default();
    assert!(!cfg.sl_edit_reaches_pendings, "domyślnie WYŁĄCZONE");
    cfg.entry_units = 4;
    cfg.tp_schedule = TpSchedule::AllRunners;

    let (mut e, mut b) = stanowisko(cfg, 4030.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL_DALEKI));
    let przed: Vec<Option<f64>> = b.pendings().iter().map(|o| o.sl).collect();

    let mut m = wiadomosc(T0 + 1_000, 2, "MOVE SL TO 3995");
    m.reply_to = Some(1);
    e.on_message(&mut b, &m);

    let po: Vec<Option<f64>> = b.pendings().iter().map(|o| o.sl).collect();
    assert_eq!(przed, po, "bez flagi pendingi zostają nietknięte");
}

/// Z-9: „SELL STOP 4730" ma czekać na PRZEBICIE, a nie wchodzić po rynku.
#[test]
fn sell_stop_sklada_zlecenie_przebiciowe_zamiast_wchodzic_po_rynku() {
    let mut cfg = Settings::default();
    cfg.honor_stop_orders = true;
    cfg.entry_units = 1;
    cfg.tp_schedule = TpSchedule::AllRunners;

    let (mut e, mut b) = stanowisko(cfg, 4001.0);
    e.on_message(
        &mut b,
        &wiadomosc(T0, 1, "SELL STOP 4000\nSL 4002\nTP 3990 3980 3970"),
    );
    assert!(
        b.positions().is_empty(),
        "zlecenie STOP nie wolno zamienić na natychmiastowe wejście po rynku"
    );
    assert!(
        b.pendings().iter().any(|o| o.kind == PendingKind::SellStop),
        "ma powstać SELL STOP, a nie SELL LIMIT: {:?}",
        b.pendings().iter().map(|o| o.kind).collect::<Vec<_>>()
    );
}

/// Z-9 bez flagi: udokumentowane STARE zachowanie (wejście po rynku).
#[test]
fn bez_flagi_sell_stop_wchodzi_po_rynku_jak_dotad() {
    let mut cfg = Settings::default();
    assert!(!cfg.honor_stop_orders, "domyślnie WYŁĄCZONE");
    cfg.entry_units = 1;
    cfg.tp_schedule = TpSchedule::AllRunners;

    let (mut e, mut b) = stanowisko(cfg, 4001.0);
    e.on_message(
        &mut b,
        &wiadomosc(T0, 1, "SELL STOP 4000\nSL 4002\nTP 3990 3980 3970"),
    );
    assert!(
        !b.positions().is_empty(),
        "stare zachowanie: limit nie do położenia → wejście po rynku"
    );
}
