
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::Engine;
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_700_000_000_000;

fn kwotowanie(ts: Ts, bid: f64) -> Quote {
    Quote {
        ts,
        bid,
        ask: bid + 0.20,
    }
}

/// Ustawienia liczące lot PROCENTEM kapitału — tylko przy nich podstawa
/// w ogóle ma znaczenie. Przy `lot_fixed` wolumen nie zależy od salda.
fn cfg_procentowy() -> Settings {
    let mut c = Settings::default();
    c.lot_mode_percent = true;
    c.lot_percent = 1.0; // 1 % → 0,01 lota na każde 100 $
    c.lot_min = 0.01;
    c.lot_max = 0.0; // bez sufitu, żeby nie zamaskował różnicy
    c
}

/// Silnik podpięty do symulowanego rachunku o zadanym saldzie i kredycie.
fn stanowisko(cfg: Settings, saldo: f64) -> (Engine, SimBroker) {
    let mut b = SimBroker::z_ustawien(saldo, &cfg);
    b.on_quote(kwotowanie(T0, 4000.0));
    let mut e = Engine::new(cfg, saldo);
    let q = kwotowanie(T0, 4000.0);
    e.on_tick(&mut b, &q);
    (e, b)
}

// ============================================================
//  1. RÓWNOWAŻNOŚĆ: 600 z kredytem 300 == 300 bez kredytu
// ============================================================

#[test]
fn lot_z_600_minus_kredyt_300_rowna_sie_lotowi_z_300() {
    // --- konto z bonusem: saldo 600, kredyt 300, odliczanie WŁĄCZONE ---
    let mut z_bonusem = cfg_procentowy();
    z_bonusem.odlicz_kredyt = true;
    z_bonusem.kredyt_reczny = 300.0;
    let (e_bonus, b_bonus) = stanowisko(z_bonusem, 600.0);

    // --- konto bez bonusu: saldo 300, żadnego kredytu ---
    let (e_goly, _) = stanowisko(cfg_procentowy(), 300.0);

    assert_eq!(
        b_bonus.account().balance,
        600.0,
        "broker ma raportować PEŁNE saldo — kredytu nie ukrywamy przed nikim"
    );
    assert_eq!(
        b_bonus.account().credit,
        300.0,
        "kredyt musi dojść z brokera"
    );

    assert_eq!(
        e_bonus.podstawa_lota(),
        300.0,
        "podstawa lota na koncie 600 $ z bonusem 300 $ to 300 $, a nie 600 $"
    );
    assert_eq!(
        e_bonus.lot_size(e_bonus.podstawa_lota()),
        e_goly.lot_size(e_goly.podstawa_lota()),
        "lot z salda 600 $ przy kredycie 300 $ MUSI być równy lotowi z salda 300 $ \
         bez kredytu — to jest cała treść tej funkcji"
    );

    // I dla porządku: bez odliczania byłby dwa razy większy.
    let (e_bez, _) = stanowisko(cfg_procentowy(), 600.0);
    assert!(
        e_bez.lot_size(e_bez.podstawa_lota()) > e_bonus.lot_size(e_bonus.podstawa_lota()),
        "gdyby odliczanie nic nie zmieniało, ten test niczego by nie pilnował"
    );
}

// ============================================================
//  2. BRAK REGRESU: konto bez bonusu zachowuje się jak dotąd
// ============================================================

#[test]
fn bez_kredytu_i_bez_przelacznika_podstawa_to_pelne_saldo() {
    let (e, _) = stanowisko(cfg_procentowy(), 437.19);
    assert_eq!(e.kredyt_skuteczny(), 0.0);
    assert_eq!(
        e.podstawa_lota(),
        437.19,
        "przy wyłączonym przełączniku podstawa MUSI być pełnym saldem — \
         inaczej nowe pola ruszyłyby bramkę parytetu"
    );
}

#[test]
fn wlaczony_przelacznik_bez_kredytu_niczego_nie_zmienia() {
    // Najczęstszy przypadek na koncie bez promocji: ktoś włączył odliczanie
    // „na wszelki wypadek". Terminal raportuje `credit = 0`, więc nie ma
    // czego odejmować i wynik nie ma prawa się różnić.
    let mut c = cfg_procentowy();
    c.odlicz_kredyt = true;
    let (e, b) = stanowisko(c, 437.19);
    assert_eq!(b.account().credit, 0.0);
    assert_eq!(e.kredyt_skuteczny(), 0.0);
    assert_eq!(e.podstawa_lota(), 437.19);
}

// ============================================================
//  3. KONWENCJA ZERA: 0 = AUTOMAT, nie „kredytu nie ma"
// ============================================================

#[test]
fn zero_w_polu_recznym_znaczy_automat_czyli_odczyt_z_terminala() {
    // Symulujemy rachunek, na którym terminal RAPORTUJE bonus, a pole ręczne
    // zostało puste. Poprawna odpowiedź: bierz z terminala.
    let mut c = cfg_procentowy();
    c.odlicz_kredyt = true;
    c.kredyt_reczny = 0.0; // AUTOMAT

    let mut b = SimBroker::z_ustawien(600.0, &c);
    b.credit = 300.0; // to, co powiedziałby terminal
    b.on_quote(kwotowanie(T0, 4000.0));
    let mut e = Engine::new(c, 600.0);
    e.on_tick(&mut b, &kwotowanie(T0, 4000.0));

    assert_eq!(
        e.kredyt_skuteczny(),
        300.0,
        "zero w polu ręcznym to AUTOMAT — gdyby czytać je jako „kredytu nie ma\", \
         bot grałby lotem od 600 $ mimo włączonego odliczania"
    );
    assert_eq!(e.podstawa_lota(), 300.0);
}

#[test]
fn kwota_reczna_nadpisuje_odczyt_z_terminala() {
    let mut c = cfg_procentowy();
    c.odlicz_kredyt = true;
    c.kredyt_reczny = 250.0;

    let mut b = SimBroker::z_ustawien(600.0, &c);
    b.credit = 300.0; // terminal mówi co innego
    b.on_quote(kwotowanie(T0, 4000.0));
    let mut e = Engine::new(c, 600.0);
    e.on_tick(&mut b, &kwotowanie(T0, 4000.0));

    assert_eq!(e.kredyt_skuteczny(), 250.0, "dodatnia kwota ręczna wygrywa");
    assert_eq!(e.podstawa_lota(), 350.0);
}

#[test]
fn wylaczony_przelacznik_ignoruje_nawet_wpisana_kwote() {
    // Kwota w polu NIE jest sama w sobie zgodą na odliczanie. Zostaje
    // w ustawieniach po wyłączeniu przełącznika i nie ma prawa działać.
    let mut c = cfg_procentowy();
    c.odlicz_kredyt = false;
    c.kredyt_reczny = 300.0;
    let (e, _) = stanowisko(c, 600.0);
    assert_eq!(e.kredyt_skuteczny(), 0.0);
    assert_eq!(e.podstawa_lota(), 600.0);
}

// ============================================================
//  4. PODUSZKA ZOSTAJE PODUSZKĄ
// ============================================================

#[test]
fn kredyt_schodzi_tylko_z_podstawy_lota_a_nie_z_marginesu() {
    // Rozstrzygnięcie użytkownika, wprost: „będzie większy margines i będzie
    // można otworzyć więcej i to jest OKEJ, bo od tego służy credit".
    // Odjęcie bonusu z equity albo z wolnego depozytu zabrałoby dokładnie tę
    // zdolność, dla której bonus się bierze.
    let mut c = cfg_procentowy();
    c.odlicz_kredyt = true;
    c.kredyt_reczny = 300.0;
    let (e, b) = stanowisko(c, 600.0);

    let acc = b.account();
    assert_eq!(
        acc.balance, 600.0,
        "saldo widziane przez brokera zostaje pełne"
    );
    assert_eq!(acc.equity, 600.0, "equity NIE jest pomniejszane o bonus");
    assert_eq!(
        acc.free_margin, 600.0,
        "wolny depozyt NIE jest pomniejszany o bonus"
    );
    assert_eq!(
        e.stats.balance, 600.0,
        "statystyki konta pokazują saldo, nie podstawę"
    );
    assert_eq!(
        e.podstawa_lota(),
        300.0,
        "pomniejszona jest WYŁĄCZNIE podstawa lota"
    );
}

#[test]
fn kredyt_wiekszy_od_salda_nie_daje_ujemnej_podstawy() {
    // Strata zjadła własne pieniądze: saldo 180 $, bonus 300 $. Podstawa nie
    // ma prawa wyjść ujemna — lot schodzi na podłogę `lot_min` i tyle.
    let mut c = cfg_procentowy();
    c.odlicz_kredyt = true;
    c.kredyt_reczny = 300.0;
    let (e, _) = stanowisko(c, 180.0);
    assert_eq!(e.podstawa_lota(), 0.0);
    assert_eq!(
        e.lot_size(e.podstawa_lota()),
        0.01,
        "podstawa zero daje lot z podłogi, a nie NaN ani wartość ujemną"
    );
}

// ============================================================
//  5. POLE RACHUNKU, NIE POLE PRESETU
// ============================================================

#[test]
fn oba_pola_kredytu_sa_polami_rachunku() {
    // Bonus daje broker JEDNEMU kontu, jedną kwotą. Gdyby te pola były per
    // format, dwa silniki liczyłyby lot od dwóch różnych podstaw tego samego
    // salda — i nikt by nie wiedział, który ma rację.
    use conduit_core::wielosilnik::POLA_RACHUNKU;
    for k in ["odlicz_kredyt", "kredyt_reczny"] {
        assert!(
            POLA_RACHUNKU.contains(&k),
            "{k} musi być polem RACHUNKU, nie presetu"
        );
    }

    // I sprawdzenie, że składanie ustawień faktycznie je przenosi.
    let mut preset = Settings::default();
    preset.odlicz_kredyt = false;
    preset.kredyt_reczny = 0.0;
    let mut rachunek = Settings::default();
    rachunek.odlicz_kredyt = true;
    rachunek.kredyt_reczny = 300.0;

    let zlozone = conduit_core::wielosilnik::ustawienia_formatu(&preset, &rachunek);
    assert!(
        zlozone.odlicz_kredyt,
        "przełącznik z rachunku musi nadpisać preset"
    );
    assert_eq!(zlozone.kredyt_reczny, 300.0);
}
