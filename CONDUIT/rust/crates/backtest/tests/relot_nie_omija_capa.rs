
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_700_000_000_000;
/// Cena startowa NAD strefą, żeby siatka BUY LIMIT została na rynku zamiast
/// wypełnić się od razu. Cały test dotyczy zleceń OCZEKUJĄCYCH.
const CENA: f64 = 4012.0;
const SYGNAL: &str = "BUY GOLD @ 4005/4000\nTP 4030\nTP 4060\nTP 4090\nSL 3990";

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

/// Ustawienia, przy których limit ryzyka koszyka NAPRAWDĘ WIĄŻE.
///
/// Bez tego test byłby pusty: gdy cap nie wiąże, plan i goły lot dają to samo,
/// więc obie wersje kodu przechodzą. `lot_percent = 3` przy 300 $ daje lot
/// bazowy 0,09, czyli ryzyko planu grubo ponad 25 % kapitału.
fn cfg_relot(wg_planu: bool) -> Settings {
    let mut c = Settings::default();
    c.auto_limit = true; // siatka limitów, nie wejście rynkowe
    c.lot_mode_percent = true;
    c.lot_percent = 3.0;
    c.lot_min = 0.01;
    c.lot_max = 0.0;
    c.entry_units = 5;
    c.entry_weights_from_rr = true;
    c.entry_weights_rr_cap = 3.0;
    c.entry_weights_rr_power = 1.0;
    c.risk_per_basket_pct = 25.0;
    c.max_portfolio_risk_pct = 0.0;
    c.pending_relot_on_balance = true;
    c.pending_relot_topup = true;
    c.pending_relot_up = true;
    c.pending_relot_down = true;
    c.pending_relot_wg_planu = wg_planu;
    c.pending_resize_s = 1.0; // kadencja relotu
    c
}

fn stanowisko(cfg: Settings, saldo: f64) -> (Engine, SimBroker) {
    let mut b = SimBroker::z_ustawien(saldo, &cfg);
    b.on_quote(kwotowanie(T0, CENA));
    let e = Engine::new(cfg, saldo);
    (e, b)
}

fn tik(e: &mut Engine, b: &mut SimBroker, ts: Ts, bid: f64) {
    let q = kwotowanie(ts, bid);
    b.on_quote(q);
    e.on_tick(b, &q);
}

/// Ryzyko wszystkich ŻYWYCH zleceń oczekujących: Σ |cena − SL| × 100 × wolumen.
/// Ta sama definicja, którą liczy `cap_basket_risk`.
fn ryzyko_pendingow(b: &SimBroker) -> f64 {
    b.pendings()
        .iter()
        .filter_map(|p| p.sl.map(|s| (p.price - s).abs() * XAU_CONTRACT * p.volume))
        .sum()
}

fn wolumen_pendingow(b: &SimBroker) -> f64 {
    b.pendings().iter().map(|p| p.volume).sum()
}

/// Wspólny przebieg: sygnał → siatka → SKOK SALDA → kilka cykli relotu.
///
/// Zwraca `(ryzyko przed, ryzyko po, wolumen przed, wolumen po, cap po)`.
fn przebieg(wg_planu: bool) -> (f64, f64, f64, f64, f64) {
    let cfg = cfg_relot(wg_planu);
    let pct = cfg.risk_per_basket_pct;
    let (mut e, mut b) = stanowisko(cfg, 300.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        !b.pendings().is_empty(),
        "siatka limitów musi stanąć na rynku"
    );
    assert!(
        b.positions().is_empty(),
        "nic nie może się wypełnić — cena jest nad strefą"
    );
    let ryz_przed = ryzyko_pendingow(&b);
    let vol_przed = wolumen_pendingow(&b);

    // SKOK SALDA — dokładnie sytuacja, o którą chodzi w compoundingu:
    // konto urosło, a siatka leży z lotem sprzed wzrostu.
    b.balance = 1200.0;

    // kilka cykli ponad kadencję `pending_resize_s`
    for k in 1..=6 {
        tik(&mut e, &mut b, T0 + k * 2_000, CENA);
    }
    let cap = b.equity().max(0.0) * pct / 100.0;
    (
        ryz_przed,
        ryzyko_pendingow(&b),
        vol_przed,
        wolumen_pendingow(&b),
        cap,
    )
}

// ============================================================
//  1. NIEZMIENNIK RYZYKA — ten test świeci na czerwono bez poprawki
// ============================================================

#[test]
fn dokladka_nie_przekracza_limitu_ryzyka_koszyka() {
    let (ryz_przed, ryz_po, _, _, cap) = przebieg(true);
    assert!(
        ryz_przed <= cap + 1e-6,
        "kontrola wejściowa: sam plan ma się mieścić w capie ({ryz_przed:.2} vs {cap:.2})"
    );
    assert!(
        ryz_po <= cap + 1e-6,
        "PO RELOCIE ryzyko zleceń koszyka wyszło ponad limit: {ryz_po:.2} $ przy capie \
         {cap:.2} $. Dokładka nie może omijać `cap_basket_risk` — to jest dokładnie \
         ta usterka, przez którą HYPER-X1 dawał 2,6× ekspozycji."
    );
}

/// TEST TESTU. Z wyłączoną poprawką (`pending_relot_wg_planu = false`)
/// niezmiennik MUSI zostać złamany — inaczej powyższy test nie ma czego
/// pilnować i jest gorszy niż jego brak.
#[test]
fn bramka_czerwienieje_przy_wylaczonej_poprawce() {
    let (_, ryz_po, _, _, cap) = przebieg(false);
    assert!(
        ryz_po > cap + 1e-6,
        "stary tryb (cel = sztuki × goły lot) MIAŁ przekraczać cap; jeśli już nie \
         przekracza, to test `dokladka_nie_przekracza_limitu_ryzyka_koszyka` nie \
         potrafi zaświecić na czerwono i trzeba go przepisać ({ryz_po:.2} vs {cap:.2})"
    );
}

// ============================================================
//  2. MECHANIZM ŻYJE — relot wg planu reaguje na wzrost kapitału
// ============================================================

#[test]
fn relot_wg_planu_podnosi_wolumen_po_wzroscie_salda() {
    let (_, _, vol_przed, vol_po, _) = przebieg(true);
    assert!(
        vol_po > vol_przed + 1e-9,
        "po czterokrotnym wzroście salda siatka ma urosnąć, a nie stać w miejscu \
         ({vol_przed:.2} → {vol_po:.2} lota)"
    );
}

// ============================================================
//  3. DOMYŚLNA — kto włącza relot, dostaje wersję z planem
// ============================================================

#[test]
fn domyslnie_relot_liczy_cel_z_planu() {
    assert!(
        Settings::default().pending_relot_wg_planu,
        "domyślne `false` znaczyło: włącz relot i po cichu dostań 2,6× dźwigni \
         przez ominięcie własnego limitu ryzyka"
    );
}
