
use conduit_backtest::sim::SimBroker;
use conduit_core::broker::Broker;
use conduit_core::engine::{Engine, IncomingMessage};
use conduit_core::settings::*;
use conduit_core::types::*;

const T0: Ts = 1_700_000_000_000;
const SALDO: f64 = 1_000.0;

/// Sygnał LIMIT: strefa 4000–4005 POD rynkiem, cele 4010/4020/4030 NAD nim.
/// Rynek startuje na 4008, czyli MIĘDZY strefą a pierwszym celem — to jest
/// jedyny układ, w którym da się rozstrzygnąć wyścig „najpierw cel czy
/// najpierw cofka do strefy".
const SYGNAL: &str = "BUY LIMITS GOLD @ 4005/4000 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3995";

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

fn edycja(ts: Ts, id: i64, edytowana: i64, tekst: &str) -> IncomingMessage {
    let mut m = wiadomosc(ts, id, tekst);
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

/// Ustawienia bazowe: siatka trzech limitów po 0,01 lota, bez żadnego
/// dławika, który mógłby uciąć szczeble i zmienić liczby pod testem.
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
    c
}

/// Sygnał + trzy tiki w GÓRĘ, przez wszystkie trzy cele, ANI RAZU nie
/// schodząc do strefy. Po tym przebiegu koszyk nie ma prawa mieć ani jednego
/// wypełnienia — i to jest przesłanka wszystkich testów D1–D3.
fn przejscie_przez_cele_bez_wejscia(zmien: impl FnOnce(&mut Settings)) -> (Engine, SimBroker) {
    let mut cfg = cfg_bazowa();
    zmien(&mut cfg);
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert_eq!(b.pendings().len(), 3, "siatka limitów musi się rozstawić");

    for (i, bid) in [4011.0, 4021.0, 4031.0].iter().enumerate() {
        tik(&mut e, &mut b, T0 + 1_000 * (i as i64 + 1), *bid);
    }
    assert_eq!(
        b.filled_pendings, 0,
        "przesłanka testu: cena nie schodziła do strefy, więc NIC nie mogło się wypełnić"
    );
    (e, b)
}

// ============================================================
//  D1 — ETAP KOSZYKA BEZ POZYCJI
// ============================================================

/// D1a: cena mija WSZYSTKIE cele, koszyk nie ma ani jednego lota → `tp_stage`
/// zostaje na zerze przy każdej kombinacji ustawień życia siatki.
///
/// ŚWIECI NA CZERWONO, gdy: w `drop_grid_on_target` przywrócić
/// `bk.tp_stage = stage` (sprawdzone — 4 z 8 przypadków padają, po jednym na
/// każdą kombinację z `pending_drop_on_target = true`).
#[test]
fn d1_koszyk_bez_pozycji_nie_awansuje_etapu_z_ceny() {
    for zycie in [
        PendingLifetime::UntilTp1,
        PendingLifetime::UntilTp2,
        PendingLifetime::UntilTp3,
        PendingLifetime::Never,
    ] {
        for kasuj in [true, false] {
            let (e, _b) = przejscie_przez_cele_bez_wejscia(|c| {
                c.pending_lifetime = zycie;
                c.pending_drop_on_target = kasuj;
            });
            let bk = &e.baskets[0];
            assert_eq!(
                bk.tp_stage, 0,
                "pending_lifetime={zycie:?} drop={kasuj}: koszyk BEZ POZYCJI awansował \
                 etap do {} — to jest dokładnie ta liczba, którą `reentry_pass` \
                 zamieniała na nielimitowane wejścia po rynku",
                bk.tp_stage
            );
            assert!(
                !bk.had_positions,
                "pending_lifetime={zycie:?} drop={kasuj}: koszyk nigdy nie wszedł, \
                 więc nie ma prawa się meldować jako handlujący"
            );
            assert_eq!(
                bk.state,
                BasketState::Armed,
                "pending_lifetime={zycie:?} drop={kasuj}: bez wypełnienia stan zostaje Armed"
            );
        }
    }
}

/// D1b: OBSERWACJA rynku ma trafiać do WŁASNEJ wielkości, nie do `tp_stage`.
///
/// Sam fakt „rynek przeszedł drogę bez nas" jest potrzebny: bez niego
/// `pending_lifetime = UntilTp2/UntilTp3` nigdy by nie dojrzało, skoro
/// `tp_stage` ma teraz stać w miejscu. Ten test pilnuje POŁOWY, która da się
/// sprawdzić bez znajomości nazwy pola: reguła kasowania siatki musi DZIAŁAĆ
/// przy `UntilTp2`, mimo że etap koszyka nie rośnie.
///
/// ŚWIECI NA CZERWONO, gdy: obserwacja przestanie być gdziekolwiek zapisywana
/// (siatka przeżyje drugi cel i pendingi zostaną).
#[test]
fn d1_obserwacja_rynku_dziala_mimo_stojacego_etapu() {
    let (_e, b) = przejscie_przez_cele_bez_wejscia(|c| {
        c.pending_lifetime = PendingLifetime::UntilTp2;
        c.pending_drop_on_target = true;
    });
    assert!(
        b.pendings().is_empty(),
        "rynek przeszedł DWA cele bez nas, więc `UntilTp2` ma skasować siatkę — \
         zostało {} zleceń. Jeśli etap stoi (i słusznie), obserwacja musi być \
         liczona osobno, inaczej ta reguła nigdy nie dojrzeje",
        b.pendings().len()
    );
}

#[test]
fn d1_komunikat_z_kanalu_nie_awansuje_etapu_bez_pozycji() {
    for kasuj in [true, false] {
        let mut cfg = cfg_bazowa();
        cfg.pending_drop_on_target = kasuj;
        cfg.pending_lifetime = PendingLifetime::Never;
        let (mut e, mut b) = stanowisko(cfg, 4008.0);
        e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
        assert_eq!(b.pendings().len(), 3);

        // Kanał melduje trafienie DRUGIEGO celu, a my nie mamy ani jednego lota.
        e.on_message(&mut b, &wiadomosc(T0 + 1_000, 2, "✅ TP2 HIT +200 PIPS"));

        let bk = &e.baskets[0];
        assert_eq!(
            bk.tp_stage, 0,
            "drop={kasuj}: komunikat „TP2 HIT\" przewinął etap koszyka bez pozycji do {}",
            bk.tp_stage
        );
        assert_eq!(bk.realized, 0.0, "drop={kasuj}: nie ma czego bankować");
        assert!(
            !bk.secured,
            "drop={kasuj}: meldunek o celu nie zabezpiecza pustego koszyka"
        );
    }
}

// ============================================================
//  D2 — ŻYCIE SIATKI
// ============================================================

/// D2: przy `pending_lifetime = Never` siatka PRZEŻYWA przejście ceny przez
/// wszystkie cele — i jest nadal SPRAWNA, czyli wypełnia się po powrocie.
///
/// „Przeżywa" bez drugiej połowy nic nie znaczy: zlecenie może stać u brokera
/// i mieć zaznaczony szczebel jako `cancelled`/`filled`, przez co ani się nie
/// wypełni, ani nie odbuduje. Dlatego test sprawdza WYNIK, nie stan pola.
///
/// ŚWIECI NA CZERWONO, gdy: w `drop_grid_on_target` przenieść odczyt
/// `pending_lifetime` za `cancel_pendings_keep` — czyli wrócić do stanu,
/// w którym „Never" wyciszało kasowanie, ale nie stan (sprawdzone).
#[test]
fn d2_siatka_przezywa_przejscie_ceny_przez_cele() {
    let (mut e, mut b) = przejscie_przez_cele_bez_wejscia(|c| {
        c.pending_lifetime = PendingLifetime::Never;
        c.pending_drop_on_target = true; // reguła WŁĄCZONA, wyciszać ma ją `Never`
    });
    assert_eq!(
        b.pendings().len(),
        3,
        "przy `Never` żaden szczebel nie ma prawa zniknąć — cena szła sama"
    );

    // …a teraz cofka przez całą strefę: siatka ma być SPRAWNA, nie tylko obecna
    for (i, bid) in [4006.0, 4004.0, 4002.0, 3999.0].iter().enumerate() {
        tik(&mut e, &mut b, T0 + 10_000 + 1_000 * (i as i64), *bid);
    }
    assert_eq!(
        b.filled_pendings, 3,
        "siatka przeżyła, więc musi się wypełnić po powrocie ceny — inaczej \
         „przeżyła\" znaczy tylko „wisi martwa\""
    );
}

// ============================================================
//  D3 — ETAP PO SPÓŹNIONYM WYPEŁNIENIU
// ============================================================

/// D3: pozycja wypełniona PO tym, jak rynek sam przeszedł całą drabinkę,
/// zaczyna liczenie od TP1 — nie od TP3 i nie od „po ostatnim celu".
///
/// To jest najdroższa konsekwencja skażonego `tp_stage`. Przy `tp_stage = 3`
/// pętla cenowa pyta o `tps.get(3)`, dostaje `None` i **nie robi nic**: cel
/// trafiony przez naszą pozycję przechodzi bez zainkasowania transzy, bez
/// drabinki stopów i bez wpisu w dzienniku.
///
/// ŚWIECI NA CZERWONO, gdy: przywrócić `bk.tp_stage = stage` w
/// `drop_grid_on_target` (sprawdzone — `tp_stage` zostaje 3, a `realized`
/// zero).
#[test]
fn d3_pozycja_po_powrocie_ceny_dostaje_tp1_a_nie_tp3() {
    let (mut e, mut b) = przejscie_przez_cele_bez_wejscia(|c| {
        c.pending_lifetime = PendingLifetime::Never;
        c.tp_schedule = TpSchedule::AllRunners; // pozycja nie zniknie u brokera na TP1
        c.assign_tp_per_position = true;
    });
    assert_eq!(e.baskets[0].tp_stage, 0);

    // cofka do strefy — siatka się wypełnia
    for (i, bid) in [4006.0, 4004.0, 4002.0, 3999.0].iter().enumerate() {
        tik(&mut e, &mut b, T0 + 10_000 + 1_000 * (i as i64), *bid);
    }
    assert!(
        !b.positions().is_empty(),
        "przesłanka: koszyk musi mieć pozycje"
    );
    assert_eq!(
        e.baskets[0].tp_stage, 0,
        "samo wypełnienie nie jest trafionym celem"
    );

    // …i dopiero teraz rynek idzie do PIERWSZEGO celu — z nami w rynku
    tik(&mut e, &mut b, T0 + 20_000, 4011.0);

    let bk = &e.baskets[0];
    assert_eq!(
        bk.tp_stage, 1,
        "pierwszy cel trafiony NASZĄ pozycją to TP1 — etap {} znaczy, że koszyk \
         zaczął liczyć od miejsca, do którego rynek doszedł bez nas",
        bk.tp_stage
    );
    assert!(
        bk.tp_touch_ts.first().copied().unwrap_or(0) > 0,
        "TP1 ma zostawić znacznik dotknięcia — bez niego okna \
         tp_signal_max_lead_s/lag_s mierzą pustkę"
    );
    assert!(
        bk.last_tp_ts > 0,
        "TP1 zaliczone NASZĄ pozycją musi zostawić chwilę trafienia — na niej \
         stoi anty-piła re-entry (`reenter_min_return_s`). Sam licznik etapu \
         bez tego znacznika jest ozdobą"
    );
    // Świadomie NIE żądamy tu zainkasowanej transzy: przy `AllRunners` każda
    // pozycja celuje w ostatni cel, więc na TP1 nie ma czego bankować i tak
    // ma być. Harmonogramy inkasa mają własne testy w `silnik_nowe_funkcje.rs`.
}

// ============================================================
//  D4 — RISK FREE I STAN „ZABEZPIECZONY"
// ============================================================

/// Stanowisko z koszykiem, który MA otwarte pozycje na plusie.
fn koszyk_na_plusie(zmien: impl FnOnce(&mut Settings)) -> (Engine, SimBroker) {
    let mut cfg = cfg_bazowa();
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.pending_lifetime = PendingLifetime::Never;
    zmien(&mut cfg);
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    // cofka do strefy — wypełniamy całą siatkę
    for (i, bid) in [4004.0, 4001.0, 3999.0].iter().enumerate() {
        tik(&mut e, &mut b, T0 + 1_000 * (i as i64 + 1), *bid);
    }
    assert_eq!(b.positions().len(), 3, "przesłanka: trzy otwarte pozycje");
    // …i rynek idzie na naszą korzyść, ale JESZCZE nie do celu
    tik(&mut e, &mut b, T0 + 8_000, 4008.0);
    (e, b)
}

/// D4a: „RISK FREE" na koszyku z pozycjami na plusie przesuwa stop na
/// breakeven powiększony o `be_offset` i oznacza koszyk jako zabezpieczony.
///
/// ŚWIECI NA CZERWONO, gdy: w `handle_risk_free` zamienić `be` na `open`
/// bez `be_offset` albo pominąć `modify_position` (sprawdzone).
#[test]
fn d4_risk_free_przesuwa_stop_na_be_gdy_pozycja_jest_na_plusie() {
    let (mut e, mut b) = koszyk_na_plusie(|c| {
        c.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
        c.be_offset = 0.30;
        c.risk_free_runner_target = RiskFreeRunnerTarget::KeepTp;
    });
    let przed: Vec<(Ticket, Px, Option<Px>)> = b
        .positions()
        .iter()
        .map(|p| (p.ticket, p.open_price, p.sl))
        .collect();
    for (_, open, sl) in &przed {
        assert!(
            sl.map(|s| s < *open).unwrap_or(true),
            "przesłanka: stop z sygnału leży POD ceną wejścia ({sl:?} vs {open})"
        );
    }

    e.on_message(&mut b, &wiadomosc(T0 + 9_000, 2, "🔒 RISK FREE 4005"));

    assert_eq!(
        b.positions().len(),
        3,
        "tryb MoveSlToBeOnly nie zamyka niczego — ma tylko przesunąć stopy"
    );
    for (t, open, _) in &przed {
        let p = b
            .find_position(*t)
            .expect("pozycja nie miała prawa zniknąć");
        let be = open + 0.30;
        assert!(
            p.sl.map(|s| (s - be).abs() < 1e-6).unwrap_or(false),
            "stop pozycji {t} miał wylądować na BE {be:.2}, a jest {:?}",
            p.sl
        );
    }
    let bk = &e.baskets[0];
    assert!(
        bk.secured,
        "koszyk z pozycjami po RISK FREE jest zabezpieczony"
    );
    assert_eq!(bk.state, BasketState::RiskFree);
}

#[test]
fn d4_koszyk_bez_pozycji_nie_jest_zabezpieczony() {
    for tekst in [
        "🔒 RISK FREE 4005",
        "SECURING PARTIAL PROFITS AND I WILL TARGET; 4010 4020 4030\n⛔ SL IS SET TO BE AT 4005",
    ] {
        let mut cfg = cfg_bazowa();
        cfg.risk_free_mode = RiskFreeMode::MoveSlToBeOnly;
        cfg.be_offset = 0.30;
        cfg.pending_lifetime = PendingLifetime::Never;
        let (mut e, mut b) = stanowisko(cfg, 4008.0);
        e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
        assert_eq!(b.pendings().len(), 3);
        assert!(
            b.positions().is_empty(),
            "przesłanka: sama siatka, zero pozycji"
        );

        e.on_message(&mut b, &wiadomosc(T0 + 1_000, 2, tekst));

        let bk = &e.baskets[0];
        assert!(
            !bk.secured,
            "„{tekst}\" oznaczył koszyk BEZ POZYCJI jako uwolniony od ryzyka — \
             a ryzyko ma wtedy PEŁNE, bo siatka wciąż czeka"
        );
        assert_eq!(
            bk.secured_ts, 0,
            "bez zabezpieczenia nie wolno też uzbrajać zegara runnera"
        );
        assert_eq!(
            bk.tp_stage, 0,
            "SPP na pustym koszyku to nie jest trafiony cel"
        );
    }
}

// ============================================================
//  D5 — STRAŻ EKSPOZYCJI
// ============================================================

/// Koszyk, w którym część szczebli leży POD ceną (zostaje limitami), a część
/// NAD nią (wchodzi po rynku) — czyli jedyny układ, w którym da się odróżnić
/// „kasowanie zleceń" od „domykania pozycji".
fn stanowisko_mieszane(zmien: impl FnOnce(&mut Settings)) -> (Engine, SimBroker) {
    let mut cfg = cfg_bazowa();
    cfg.entry_units = 4;
    cfg.lot_fixed = 0.10;
    cfg.pending_lifetime = PendingLifetime::Never;
    cfg.pending_drop_on_target = false;
    zmien(&mut cfg);
    // cena W ŚRODKU strefy 4000–4005
    let (mut e, mut b) = stanowisko(cfg, 4002.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    assert!(
        !b.pendings().is_empty(),
        "przesłanka: część szczebli została limitami"
    );
    assert!(
        !b.positions().is_empty(),
        "przesłanka: część szczebli weszła po rynku"
    );
    (e, b)
}

/// D5a: gdy same skasowane limity nie wystarczą, straż DOMYKA POZYCJE.
///
/// To jest cała różnica między polisą a bramką wejścia: pułap globalny
/// blokuje nowe sygnały, ale nigdy nie redukuje tego, co już leży — po
/// pierwszym wybuchu ekspozycji księga zostaje ZAMROŻONA w stanie stratnym.
///
/// ŚWIECI NA CZERWONO, gdy: usunąć gałąź (b) `DOMYKANIE POZYCJI` z
/// `redukuj_ekspozycje` (sprawdzone).
#[test]
fn d5_straz_ekspozycji_domyka_pozycje_a_nie_tylko_kasuje_zlecenia() {
    let (mut e, mut b) = stanowisko_mieszane(|c| {
        // 1 % z 1000 $ = 10 $ potencjalnego marginesu; jeden szczebel 0,10 lota
        // przy 4000 $ i dźwigni 500 kosztuje 80 $ — próg wiąże z zapasem.
        c.expo_cap_pct = 1.0;
        c.expo_cap_s = 0.0;
        c.expo_cap_close = true;
    });
    let pozycji_przed = b.positions().len();

    tik(&mut e, &mut b, T0 + 1_000, 4002.0);

    assert!(
        b.pendings().is_empty(),
        "wariant (a): najpierw giną leżące zlecenia — zostało {}",
        b.pendings().len()
    );
    assert!(
        b.positions().len() < pozycji_przed,
        "wariant (b): po zejściu do zera zleceń straż MUSI domknąć pozycje — \
         było {pozycji_przed}, jest {}",
        b.positions().len()
    );
    assert!(
        e.stats.expo_poz_domkniete > 0,
        "domknięcia mają być POLICZONE — bez licznika reguła jest niewidzialna"
    );
}

/// D5b: druga strona przełącznika. Bez niej test wyżej przechodziłby także
/// wtedy, gdyby pozycje znikały z zupełnie innego powodu (stop, margin call).
#[test]
fn d5_bez_zgody_na_domykanie_pozycje_zostaja() {
    let (mut e, mut b) = stanowisko_mieszane(|c| {
        c.expo_cap_pct = 1.0;
        c.expo_cap_s = 0.0;
        c.expo_cap_close = false;
    });
    let pozycji_przed = b.positions().len();

    tik(&mut e, &mut b, T0 + 1_000, 4002.0);

    assert!(b.pendings().is_empty(), "zlecenia giną tak samo");
    assert_eq!(
        b.positions().len(),
        pozycji_przed,
        "bez `expo_cap_close` nie wolno zrealizować straty — pozycje zostają"
    );
    assert!(
        e.stats.expo_niedosyt > 0,
        "sytuacja „zeszliśmy do zera zleceń i wciąż jesteśmy nad progiem\" \
         musi zostawić ślad w liczniku"
    );
    assert_eq!(e.stats.expo_poz_domkniete, 0);
}

/// D5c: bramka nieruszalności. Preset, który o regule nie wie, nie płaci za
/// nią nawet jednym odczytem konta.
#[test]
fn d5_wylaczona_straz_nie_rusza_niczego() {
    let (mut e, mut b) = stanowisko_mieszane(|_| {});
    assert_eq!(Settings::default().expo_cap_pct, 0.0, "domyślnie WYŁĄCZONE");
    let (p, z) = (b.positions().len(), b.pendings().len());
    tik(&mut e, &mut b, T0 + 1_000, 4002.0);
    assert_eq!((b.positions().len(), b.pendings().len()), (p, z));
    assert_eq!(e.stats.expo_zdarzen, 0);
}

// ============================================================
//  D6 — KONWENCJA ZERA W `reenter_max`
// ============================================================

/// Ile razy koszyk dołożył się po trafionym celu przy danym `reenter_max`.
///
/// Przebieg: siatka się wypełnia, rynek idzie na TP1 (etap 1 — z POZYCJĄ,
/// więc etap jest prawdziwy), a potem cena wraca do strefy i schodzi po niej
/// krokami po 1 $ (`market_entry_step`), dając kolejne okazje do dokładki.
fn dokladki_przy_limicie(reenter_max: u32) -> u32 {
    let mut cfg = cfg_bazowa();
    cfg.entry_units = 1;
    cfg.tp_schedule = TpSchedule::AllRunners; // pozycja żyje po TP1
    cfg.pending_lifetime = PendingLifetime::Never;
    cfg.pending_drop_on_target = false;
    cfg.reenter_after_tp = true;
    cfg.reenter_min_tp_stage = 1;
    cfg.reenter_min_return_s = 0.0;
    cfg.no_reenter_from_stage = 0;
    cfg.reenter_stop_after_riskfree = false;
    cfg.market_entry_step = 1.0;
    cfg.reenter_max = reenter_max;
    cfg.max_open_positions = 0;
    cfg.max_open_baskets = 0;

    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    // wypełnienie
    tik(&mut e, &mut b, T0 + 1_000, 4001.0);
    assert!(!b.positions().is_empty(), "przesłanka: koszyk musi wejść");
    // TP1 z NASZĄ pozycją → etap 1 jest prawdziwy
    tik(&mut e, &mut b, T0 + 2_000, 4011.0);
    assert_eq!(e.baskets[0].tp_stage, 1, "przesłanka: prawdziwy etap 1");

    // powroty do strefy, co 1 $ w dół (krok `market_entry_step`)
    for (i, bid) in [4004.5, 4003.4, 4002.3, 4001.2, 4000.1].iter().enumerate() {
        tik(&mut e, &mut b, T0 + 10_000 + 1_000 * (i as i64), *bid);
    }
    e.baskets[0].reentries
}

/// D6: `0` w `reenter_max` znaczy **BEZ LIMITU**, a nie „wyłączone".
///
/// Ta dwuznaczność kosztowała już realny rozjazd wyników i jest osobno
/// zapisana w kontrakcie pola (`settings.rs`: „⚠ 0 znaczy BEZ LIMITU, a nie
/// »wyłącz«"). Wyłącznikiem CAŁEJ reguły jest `reenter_after_tp` i tylko on.
///
/// ŚWIECI NA CZERWONO, gdy: zamienić `reenter_lim == 0 || x.reentries < …`
/// na samo `x.reentries < reenter_lim` (sprawdzone — zero dokładek przy `0`).
#[test]
fn d6_reenter_max_zero_znaczy_bez_limitu() {
    let bez_limitu = dokladki_przy_limicie(0);
    assert!(
        bez_limitu >= 3,
        "`reenter_max = 0` ma znaczyć BEZ LIMITU — dokładek było {bez_limitu}, \
         a okazji pięć. Odczytanie zera jako „wyłączone\" jest zmierzonym \
         źródłem rozjazdu wyników"
    );

    let z_limitem = dokladki_przy_limicie(2);
    assert_eq!(
        z_limitem, 2,
        "przy jawnym limicie 2 mają być dokładnie dwie dokładki, nie {z_limitem}"
    );
    assert!(
        bez_limitu > z_limitem,
        "gdyby `0` znaczyło „wyłączone\", ta nierówność byłaby odwrotna"
    );
}

/// D6b: wyłącznikiem jest `reenter_after_tp`, i to on ma być domyślnie
/// wyłączony — konwencja zera nie może być JEDYNYM sposobem wyciszenia reguły.
#[test]
fn d6_wylacznikiem_calej_reguly_jest_osobne_pole() {
    let d = Settings::default();
    assert!(!d.reenter_after_tp, "cała rodzina domyślnie milczy");
    assert_eq!(
        d.reenter_max, 0,
        "a domyślna wartość limitu to „bez limitu\""
    );

    let mut cfg = cfg_bazowa();
    cfg.reenter_after_tp = false;
    cfg.reenter_max = 0;
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.pending_lifetime = PendingLifetime::Never;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 4001.0);
    tik(&mut e, &mut b, T0 + 2_000, 4011.0);
    for (i, bid) in [4004.5, 4003.4, 4002.3].iter().enumerate() {
        tik(&mut e, &mut b, T0 + 10_000 + 1_000 * (i as i64), *bid);
    }
    assert_eq!(
        e.baskets[0].reentries, 0,
        "przy wyłączonym `reenter_after_tp` zero w limicie nie ma prawa niczego otworzyć"
    );
}

// ============================================================
//  D7 — EDYCJA ZMIENIAJĄCA WARTOŚĆ
// ============================================================

#[test]
fn d7_edycja_zmieniajaca_wartosc_nie_ginie_jako_duplikat() {
    for (os, oczekiwany) in [(true, 4002.0), (false, 4000.0)] {
        let mut cfg = cfg_bazowa();
        cfg.tp_schedule = TpSchedule::AllRunners;
        cfg.pending_lifetime = PendingLifetime::Never;
        cfg.dedup_klucz_z_wartoscia = os;
        let (mut e, mut b) = stanowisko(cfg, 4008.0);
        e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
        tik(&mut e, &mut b, T0 + 1_000, 4001.0);
        assert!(!b.positions().is_empty(), "przesłanka: koszyk w rynku");
        // Rynek wraca nad strefę: OBA poziomy stopu (4000 i 4002) muszą leżeć
        // pod ceną, inaczej broker odrzuci je jako niewykonalne i test mierzyłby
        // `stops_level`, a nie dedup.
        tik(&mut e, &mut b, T0 + 1_500, 4008.0);

        e.on_message(&mut b, &wiadomosc(T0 + 2_000, 2, "MOVE SL TO 4000"));
        assert_eq!(
            e.baskets[0].sl,
            Some(4000.0),
            "oś A4 = {os}: pierwsza zmiana stopu"
        );

        // TA SAMA wiadomość, poprawiona: inny POZIOM, ta sama akcja
        e.on_message(&mut b, &edycja(T0 + 3_000, 2, 2, "MOVE SL TO 4002"));

        assert_eq!(
            e.baskets[0].sl,
            Some(oczekiwany),
            "oś A4 = {os}: edycja poziomu {}",
            if os {
                "MUSI przejść"
            } else {
                "ginie jako duplikat (stare zachowanie)"
            }
        );
        for p in b.positions() {
            assert!(
                p.sl.map(|s| (s - oczekiwany).abs() < 1e-6).unwrap_or(false),
                "oś A4 = {os}: stop u BROKERA to {:?}, a miał być {oczekiwany} — \
                 pole koszyka bez zlecenia u brokera nie chroni ani centa",
                p.sl
            );
        }
    }
}

/// D7b: to nie jest „każda edycja przechodzi". Edycja o TEJ SAMEJ wartości
/// nadal ma być zjedzona — inaczej re-delivery po rekonekcie przestawiałoby
/// stopy w kółko.
#[test]
fn d7_edycja_o_tej_samej_wartosci_nadal_jest_duplikatem() {
    let mut cfg = cfg_bazowa();
    cfg.tp_schedule = TpSchedule::AllRunners;
    cfg.pending_lifetime = PendingLifetime::Never;
    cfg.dedup_klucz_z_wartoscia = true;
    let (mut e, mut b) = stanowisko(cfg, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0, 1, SYGNAL));
    tik(&mut e, &mut b, T0 + 1_000, 4001.0);
    tik(&mut e, &mut b, T0 + 1_500, 4008.0);
    e.on_message(&mut b, &wiadomosc(T0 + 2_000, 2, "MOVE SL TO 4000"));
    let odrzuty_przed = e.odrzuty.get("DuplicateEditedAction").copied().unwrap_or(0);

    e.on_message(&mut b, &edycja(T0 + 3_000, 2, 2, "MOVE SL TO 4000"));

    assert_eq!(e.baskets[0].sl, Some(4000.0));
    assert!(
        e.odrzuty.get("DuplicateEditedAction").copied().unwrap_or(0) > odrzuty_przed,
        "powtórka tej samej wartości ma zostać ODNOTOWANA jako duplikat: {:?}",
        e.odrzuty
    );
}
