//! EA-CORE — BRAMKA AKCEPTACJI FALI 0 (`wiedza/EA_PLAN_WDROZENIA.md`).
//!
//! Testy stoją w `tests/`, a nie w `engine.rs`, z dwóch powodów. Pierwszy jest
//! organizacyjny: `engine.rs` ma 650 kB i pracuje na nim kilka zespołów naraz,
//! więc każdy blok testowy dołożony w środku to konflikt. Drugi jest
//! merytoryczny i ważniejszy: **test integracyjny widzi WYŁĄCZNIE publiczne
//! API** — dokładnie to, co widzi warstwa żywa i backtest. Gdyby kontrakt zera
//! dało się udowodnić tylko z dostępem do pól prywatnych, nie byłby dowodem na
//! nic.
//!
//! Zawartość:
//!
//! | test | co dowodzi |
//! |---|---|
//! | `kontrakt_zera_a_*` | `ea_enabled = false` ⇒ przebieg identyczny **i konto nie czytane ani razu więcej** |
//! | `kontrakt_zera_b_*` | `ea_enabled = true` + same zera ⇒ przebieg identyczny co do bitu |
//! | `kontrakt_zera_c_*` | tryb AUTO-EA ⇒ identyczny jak AUTO, także przy WŁĄCZONEJ warstwie |
//! | `z13_zamrozenie_*` | warstwa żyje z własnego zegara przy ZEROWEJ liczbie tików |
//! | `n15_*` | dozór SL: wykrycie (dziś) i egzekucja (za `ea_dozor_sl`) |
//! | `n10_*` | zapadka stempla wykrywa wzrost ryzyka po zawiązaniu koszyka |
//! | `n18_*` | bilans `rozpatrzone = obsłużone + pominięte_z_kodem` domyka się zawsze |
//! | `n19_*` | przed odtworzeniem warstwa ODMAWIA; odtworzenie wraca stemple |

use conduit_core::broker::{BResult, Broker, BrokerError, OrderReq, PendingReq};
use conduit_core::ea::{EtapKoszyka, KodPominiecia, ZrodloPulsu};
use conduit_core::engine::IncomingMessage;
use conduit_core::settings::Settings;
use conduit_core::types::*;
use conduit_core::Engine;

const TS0: Ts = 1_755_000_000_000;

// ---------------------------------------------------------------------------
//  ATRAPA BROKERA
// ---------------------------------------------------------------------------

/// Minimalna atrapa z trzema własnościami, których wymagają testy tej fali:
///
/// 1. **liczy odczyty rachunku** (`odczyty_konta`) — bez tego punkt (a)
///    kontraktu zera jest niesprawdzalny: przebieg może być identyczny co do
///    centa, a warstwa i tak wali w terminal `account()` na każdym ticku;
/// 2. **wypełnia zlecenia oczekujące**, gdy cena przez nie przejdzie — bo
///    koszyk bez wypełnienia nie ma pozycji, a bez pozycji nie ma czego
///    pilnować dozorowi SL;
/// 3. **pozwala otworzyć pozycję BEZ stop-lossa** — nie „naprawia" tego po
///    cichu, bo właśnie brak takiej naprawy jest znaleziskiem N15.
struct Atrapa {
    q: Quote,
    positions: Vec<Position>,
    pendings: Vec<PendingOrder>,
    closed: Vec<ClosedTrade>,
    next: Ticket,
    odczyty_konta: std::cell::Cell<u64>,
    /// broker odmawia KAŻDEJ modyfikacji (test gałęzi `OdmowaBrokera`)
    odmawiaj_modyfikacji: bool,
}

impl Atrapa {
    fn nowa() -> Self {
        Atrapa {
            q: Quote {
                ts: TS0,
                bid: 3998.0,
                ask: 3998.3,
            },
            positions: Vec::new(),
            pendings: Vec::new(),
            closed: Vec::new(),
            next: 1,
            odczyty_konta: std::cell::Cell::new(0),
            odmawiaj_modyfikacji: false,
        }
    }

    /// Nowe kwotowanie + wypełnienie zleceń, przez które cena przeszła.
    fn cena(&mut self, ts: Ts, bid: Px) {
        self.q = Quote {
            ts,
            bid,
            ask: bid + 0.3,
        };
        let mut fill: Vec<PendingOrder> = Vec::new();
        self.pendings.retain(|o| {
            let trafione = match o.kind {
                PendingKind::BuyLimit => self.q.ask <= o.price,
                PendingKind::SellLimit => self.q.bid >= o.price,
                PendingKind::BuyStop => self.q.ask >= o.price,
                PendingKind::SellStop => self.q.bid <= o.price,
            };
            if trafione {
                fill.push(o.clone());
            }
            !trafione
        });
        for o in fill {
            let side = match o.kind {
                PendingKind::BuyLimit | PendingKind::BuyStop => Side::Buy,
                PendingKind::SellLimit | PendingKind::SellStop => Side::Sell,
            };
            self.positions.push(Position {
                ticket: o.ticket,
                side,
                volume: o.volume,
                open_price: o.price,
                open_ts: ts,
                sl: o.sl,
                tp: o.tp,
                vsl: None,
                basket: o.basket,
                level: o.level,
                frozen: false,
                peak_pts: 0.0,
                last_peak_ts: 0,
                is_runner: false,
                is_toucher: o.is_toucher,
                comment: o.comment,
            });
        }
    }

    /// Pełny obraz stanu brokera — podstawa porównań „co do bitu".
    fn obraz(&self) -> String {
        format!("{:?}|{:?}|{:?}", self.positions, self.pendings, self.closed)
    }
}

impl Broker for Atrapa {
    fn quote(&self) -> Quote {
        self.q
    }
    fn account(&self) -> Account {
        self.odczyty_konta.set(self.odczyty_konta.get() + 1);
        Account {
            balance: 400.0,
            equity: 400.0,
            margin: 0.0,
            free_margin: 400.0,
            leverage: 500,
            credit: 0.0,
        }
    }
    fn stops_level(&self) -> f64 {
        0.0
    }
    fn positions(&self) -> &[Position] {
        &self.positions
    }
    fn pendings(&self) -> &[PendingOrder] {
        &self.pendings
    }
    fn positions_mut(&mut self) -> &mut Vec<Position> {
        &mut self.positions
    }
    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
        &mut self.pendings
    }
    fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> {
        let t = self.next;
        self.next += 1;
        self.positions.push(Position {
            ticket: t,
            side: r.side,
            volume: r.volume,
            open_price: self.q.entry(r.side),
            open_ts: self.q.ts,
            sl: r.sl,
            tp: r.tp,
            vsl: None,
            basket: r.basket,
            level: r.level,
            frozen: false,
            peak_pts: 0.0,
            last_peak_ts: 0,
            is_runner: false,
            is_toucher: r.is_toucher,
            comment: r.comment,
        });
        Ok(t)
    }
    fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> {
        let t = self.next;
        self.next += 1;
        self.pendings.push(PendingOrder {
            ticket: t,
            kind: r.kind,
            volume: r.volume,
            price: r.price,
            sl: r.sl,
            tp: r.tp,
            placed_ts: self.q.ts,
            basket: r.basket,
            level: r.level,
            frozen: false,
            is_toucher: r.is_toucher,
            is_topup: r.is_topup,
            comment: r.comment,
        });
        Ok(t)
    }
    fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
        if self.odmawiaj_modyfikacji {
            return Err(BrokerError::InvalidStops);
        }
        match self.positions.iter_mut().find(|p| p.ticket == t) {
            Some(p) => {
                p.sl = sl;
                p.tp = tp;
                Ok(())
            }
            None => Err(BrokerError::NoSuchTicket),
        }
    }
    fn modify_pending(
        &mut self,
        t: Ticket,
        price: Px,
        sl: Option<Px>,
        tp: Option<Px>,
    ) -> BResult<()> {
        match self.pendings.iter_mut().find(|o| o.ticket == t) {
            Some(o) => {
                o.price = price;
                o.sl = sl;
                o.tp = tp;
                Ok(())
            }
            None => Err(BrokerError::NoSuchTicket),
        }
    }
    fn close_position(&mut self, t: Ticket, _r: CloseReason) -> BResult<f64> {
        let przed = self.positions.len();
        self.positions.retain(|p| p.ticket != t);
        if self.positions.len() < przed {
            Ok(0.0)
        } else {
            Err(BrokerError::NoSuchTicket)
        }
    }
    fn close_partial(&mut self, _t: Ticket, _v: f64, _r: CloseReason) -> BResult<f64> {
        Ok(0.0)
    }
    fn cancel_pending(&mut self, t: Ticket) -> BResult<()> {
        let przed = self.pendings.len();
        self.pendings.retain(|o| o.ticket != t);
        if self.pendings.len() < przed {
            Ok(())
        } else {
            Err(BrokerError::NoSuchTicket)
        }
    }
    fn drain_closed(&mut self) -> Vec<ClosedTrade> {
        std::mem::take(&mut self.closed)
    }
}

// ---------------------------------------------------------------------------
//  POMOCNICZE
// ---------------------------------------------------------------------------

fn wiad(msg_id: i64, text: &str) -> IncomingMessage {
    IncomingMessage {
        ts: TS0,
        source: SourceKey::new(1, None),
        source_name: "TEST".into(),
        msg_id,
        reply_to: None,
        edit_of: None,
        text: text.into(),
    }
}

const WEJSCIE: &str = "BUY LIMITS GOLD @ 3996/3990\nTP 4010\nTP 4020\nSL 3980";
/// Ten sam setup BEZ stop-lossa — parser go przyjmuje, a silnik nie ma dziś
/// żadnej bramki, która by go odrzuciła (`cap_basket_risk` wychodzi na
/// `let Some(slv) = sl else { return }`). To jest materiał dowodowy dla N15.
const WEJSCIE_BEZ_SL: &str = "BUY LIMITS GOLD @ 3996/3990\nTP 4010\nTP 4020";

/// Jeden i ten sam scenariusz dla każdego wariantu ustawień: sygnał, spadek
/// ceny (wypełnienie limitów), wzrost, komunikat TP1, dalszy wzrost.
///
/// Zwraca (obraz brokera, obraz koszyków, liczba odczytów rachunku).
fn przebieg(zmien: impl FnOnce(&mut Settings), auto_ea: bool) -> (String, String, u64) {
    let mut c = Settings::default();
    zmien(&mut c);
    let mut e = Engine::new(c, 400.0);
    e.tryb_auto_ea = auto_ea;
    let mut b = Atrapa::nowa();

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    for (i, px) in [3997.0, 3995.0, 3993.0, 3989.0, 3992.0, 3998.0, 4004.0]
        .iter()
        .enumerate()
    {
        b.cena(TS0 + (i as i64 + 1) * 60_000, *px);
        let q = b.quote();
        e.on_tick(&mut b, &q);
    }
    e.on_message(&mut b, &wiad(2, "✅ TP1 HIT +48 PIPS"));
    for (i, px) in [4008.0, 4012.0, 4016.0].iter().enumerate() {
        b.cena(TS0 + (i as i64 + 10) * 60_000, *px);
        let q = b.quote();
        e.on_tick(&mut b, &q);
    }
    let odczyty = b.odczyty_konta.get();
    (b.obraz(), format!("{:?}", e.baskets), odczyty)
}

// ===========================================================================
//  KONTRAKT ZERA — POTRÓJNY
// ===========================================================================

/// **(a)** `ea_enabled = false` ⇒ przebieg identyczny co do bitu z przebiegiem
/// silnika, który o warstwie nic nie wie.
///
/// Wariant „silnik bez warstwy" jest tu reprezentowany przez sam `false` —
/// bo przy `false` kod warstwy nie jest w ogóle wykonywany, a jedynym śladem
/// jej istnienia jest pole w strukturze. Test jest więc **regresyjny**: pęknie
/// w dniu, w którym ktoś dopisze do `on_tick` gałąź EA bez bramki
/// `if self.cfg.ea_enabled`.
#[test]
fn kontrakt_zera_a_wylaczona_warstwa_nie_zmienia_ani_bitu() {
    let (b1, k1, _) = przebieg(|_| {}, false);
    let (b2, k2, _) = przebieg(|c| c.ea_enabled = false, false);
    assert_eq!(
        b1, b2,
        "domyślne ustawienia i jawne `ea_enabled = false` muszą być tym samym"
    );
    assert_eq!(k1, k2);
}

/// **(a) — część twardsza: KONTO NIE JEST CZYTANE ANI RAZU WIĘCEJ.**
///
/// To jest ten warunek, który odróżnia „wynik się zgadza" od „warstwa jest
/// naprawdę wyłączona". Wzorzec przepisany z `margines_pozwala`:
/// *„`próg <= 0` = oś wyłączona i funkcja wychodzi PRZED odczytem rachunku.
/// To nie jest mikrooptymalizacja, tylko warunek parytetu."*
///
/// Przy `ea_enabled = true` licznik MUSI urosnąć — inaczej test nie
/// dowodziłby niczego (mierzyłby atrapę, która konta nie czyta wcale).
#[test]
fn kontrakt_zera_a_wylaczona_warstwa_nie_czyta_konta() {
    let (_, _, bez) = przebieg(|c| c.ea_enabled = false, false);
    let (_, _, z_warstwa) = przebieg(|c| c.ea_enabled = true, false);
    assert!(
        z_warstwa > bez,
        "test jest ślepy: warstwa włączona nie odczytała konta ani razu więcej ({z_warstwa} vs {bez})"
    );

    // a teraz właściwa asercja: `false` nie dokłada ANI JEDNEGO odczytu
    // ponad to, co robi silnik z domyślnymi ustawieniami
    let (_, _, domyslne) = przebieg(|_| {}, false);
    assert_eq!(
        bez, domyslne,
        "wyłączona warstwa dołożyła odczyty rachunku: {bez} zamiast {domyslne}"
    );
}

/// **(b) PODWÓJNE ZERO:** `ea_enabled = true` + wszystkie pola rodzin zerowe
/// ⇒ przebieg **co do bitu** identyczny z wyłączoną warstwą.
///
/// To jest test, który oddziela KOSZT SZKIELETU od kosztu polityk. Warstwa
/// pracuje pełną parą — pulsuje, liczy wektor stanu, stempluje koszyki,
/// prowadzi maszynę stanu — i nie zmienia ani jednej liczby.
#[test]
fn kontrakt_zera_b_podwojne_zero_nie_zmienia_ani_bitu() {
    let (b1, k1, _) = przebieg(|c| c.ea_enabled = false, false);
    let (b2, k2, _) = przebieg(|c| c.ea_enabled = true, false);
    assert_eq!(b2, b1, "warstwa z samymi zerami ruszyła stan brokera");
    assert_eq!(k2, k1, "warstwa z samymi zerami ruszyła koszyki");
}

/// **(b) z kadencją zegara.** Kadencja jest polem SZKIELETU, nie polityką —
/// więc `ea_tick_s = 1` i `= 5` też nie mają prawa ruszyć ani jednej liczby.
/// Bez tego testu koszt zegara mierzylibyśmy na przebiegach, które nie są
/// tym samym przebiegiem.
#[test]
fn kontrakt_zera_b_kadencja_zegara_nie_zmienia_ani_bitu() {
    let (wzor, kwzor, _) = przebieg(|c| c.ea_enabled = false, false);
    for kadencja in [0.0, 1.0, 5.0, 60.0] {
        let (b, k, _) = przebieg(
            |c| {
                c.ea_enabled = true;
                c.ea_tick_s = kadencja;
            },
            false,
        );
        assert_eq!(b, wzor, "ea_tick_s = {kadencja} ruszyło stan brokera");
        assert_eq!(k, kwzor, "ea_tick_s = {kadencja} ruszyło koszyki");
    }
}

/// **(b) z pełną maszyną stanu.** Progi USTAWIONE, ale rodziny B–G nie mają
/// jeszcze ani jednego czytelnika modulatorów — więc nawet przełączający się
/// stan nie ma prawa ruszyć wyniku. Ten test jest **bramką na przyszłość**:
/// pęknie w chwili, w której pierwsza oś zacznie czytać stan BEZ własnego
/// wyłącznika w zerze.
#[test]
fn kontrakt_zera_b_maszyna_stanu_bez_osi_jest_bezobjawowa() {
    let (wzor, kwzor, _) = przebieg(|c| c.ea_enabled = false, false);
    let (b, k, _) = przebieg(
        |c| {
            c.ea_enabled = true;
            c.ea_tick_s = 1.0;
            c.ea_state_src = conduit_core::settings::EaStateSrc::FloatPctEquity;
            c.ea_defense_enter = 0.001; // obrona praktycznie od razu
            c.ea_defense_exit = 0.0005;
            c.ea_offense_enter = 0.001;
            c.ea_offense_exit = 0.0005;
            c.ea_state_dwell_s = 1.0;
        },
        false,
    );
    assert_eq!(b, wzor, "maszyna stanu bez osi ruszyła stan brokera");
    assert_eq!(k, kwzor, "maszyna stanu bez osi ruszyła koszyki");
}

/// **(c) TRYB AUTO-EA ⇒ co do bitu jak AUTO** — także przy WŁĄCZONEJ warstwie.
///
/// Flaga `tryb_auto_ea` jest bramą dla przyszłych osi; dziś nie czyta jej ani
/// jedna. Test pilnuje, żeby tak zostało do chwili, w której oś dostanie
/// własny wyłącznik.
#[test]
fn kontrakt_zera_c_auto_ea_rowna_sie_auto() {
    for wlaczona in [false, true] {
        let (b1, k1, _) = przebieg(|c| c.ea_enabled = wlaczona, false);
        let (b2, k2, _) = przebieg(|c| c.ea_enabled = wlaczona, true);
        assert_eq!(
            b2, b1,
            "AUTO-EA rozjechało się z AUTO (ea_enabled = {wlaczona})"
        );
        assert_eq!(
            k2, k1,
            "AUTO-EA rozjechało się z AUTO na koszykach (ea_enabled = {wlaczona})"
        );
    }
}

// ===========================================================================
//  Z13 — TEST ZAMROŻENIA STRUMIENIA
// ===========================================================================

/// **Warstwa EA musi żyć, gdy tiki PRZESTAJĄ PRZYCHODZIĆ.**
///
/// Klasa błędu, którą ten test zamyka, ma w projekcie jeden znany przypadek
/// i kosztował 12 h martwego `rev_exit` oraz rozjazd −80 % wobec backtestu.
/// **Backtest tej klasy NIE WIDZI** — `sim_clock_strict` zawsze karmi silnik
/// tikiem, więc reguła zależna od strumienia wygląda tam na zdrową.
///
/// Scenariusz: sygnał, jeden tik (żeby siatka stanęła), a potem **godzina
/// ciszy** prowadzona wyłącznie przez `Engine::ea_zegar`. Asercje:
///  * puls z zegara faktycznie się odbywa (`pulsy_zegar` rośnie),
///  * kadencja jest respektowana (60 pulsów przy 60 s przez godzinę,
///    nie 3600 i nie 1),
///  * ani jeden tik nie był potrzebny (`pulsy_tick` nie rośnie w tej fazie).
#[test]
fn z13_zamrozenie_strumienia_warstwa_zyje_z_wlasnego_zegara() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    c.ea_tick_s = 60.0;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 1_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    let tikow_po_starcie = e.ea.pulsy_tick;
    assert!(
        tikow_po_starcie > 0,
        "puls ze źródła TICK w ogóle nie zadziałał"
    );

    // --- STRUMIEŃ ZAMARZA: ani jednego `on_tick` przez godzinę ---
    let start = TS0 + 1_000;
    let mut odbytych = 0u64;
    for s in 1..=3600i64 {
        if e.ea_zegar(&mut b, start + s * 1000) {
            odbytych += 1;
        }
    }

    assert_eq!(
        e.ea.pulsy_tick, tikow_po_starcie,
        "w fazie ciszy nie było ANI JEDNEGO ticka, a licznik tickowy urósł"
    );
    assert_eq!(
        e.ea.pulsy_zegar, odbytych,
        "licznik pulsów z zegara nie zgadza się z liczbą zwróconych `true`"
    );
    assert_eq!(
        odbytych, 60,
        "kadencja 60 s przez godzinę ciszy = 60 pulsów, a nie {odbytych} \
         (3600 = zegar zignorowany, 0 = straż śpi — obie awarie są zabójcze)"
    );
    assert!(e.ea.gotowy(), "warstwa nie odtworzyła stanu mimo 60 pulsów");
}

/// Zegar mierzy CZAS ZDARZENIA, nie liczbę wywołań.
///
/// `rev_exit` zamarzł, bo klucz zależał od `len()` bufora. Tutaj: tysiąc
/// wywołań z TYM SAMYM znacznikiem czasu daje **jeden** puls, a jedno
/// wywołanie po przeskoku czasu — też jeden. Reguła nie ma jak się zapętlić
/// ani zamrozić.
#[test]
fn z13_zegar_liczy_czas_a_nie_wywolania() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    c.ea_tick_s = 10.0;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    let mut odbytych = 0;
    for _ in 0..1000 {
        if e.ea_zegar(&mut b, TS0 + 50_000) {
            odbytych += 1;
        }
    }
    assert_eq!(
        odbytych, 1,
        "tysiąc wywołań w tej samej chwili to JEDEN puls"
    );
    assert!(e.ea_zegar(&mut b, TS0 + 60_000), "po 10 s puls musi wejść");
}

/// Kontrakt zera dla samego wejścia zegarowego: przy `ea_enabled = false`
/// `ea_zegar` wychodzi PRZED czymkolwiek i nie czyta rachunku.
#[test]
fn z13_zegar_przy_wylaczonej_warstwie_nie_czyta_konta() {
    let mut e = Engine::new(Settings::default(), 400.0);
    let mut b = Atrapa::nowa();
    let przed = b.odczyty_konta.get();
    for s in 0..1000 {
        assert!(
            !e.ea_zegar(&mut b, TS0 + s * 1000),
            "wyłączona warstwa zrobiła puls"
        );
    }
    assert_eq!(
        b.odczyty_konta.get(),
        przed,
        "wyłączona warstwa odczytała rachunek"
    );
    assert_eq!(e.ea.pulsy(), 0);
}

// ===========================================================================
//  N15 — KAŻDA POZYCJA MA SL U BROKERA
// ===========================================================================

/// **ZNALEZISKO: N15 JEST DZIŚ ZŁAMANY.**
///
/// Sygnał bez stop-lossa przechodzi przez cały silnik i produkuje pozycję
/// z `sl = None`. Nie ma dziś ANI JEDNEJ bramki, która by to złapała:
/// `cap_basket_risk` wychodzi na `let Some(slv) = sl else { return }`,
/// a `sl_min_dist` tylko ODSUWA stop, który już jest (`compute_sl` zaczyna
/// od `e.sl?`). Taki koszyk omija limit koszykowy, portfelowy i skalę rynkową
/// NARAZ.
///
/// Test jest napisany tak, żeby **pękł w dniu naprawy** — i to jest jego
/// druga funkcja: wtedy trzeba tu wpisać nowy stan świata, a nie skasować
/// asercję.
#[test]
fn n15_znalezisko_pozycja_bez_sl_powstaje_dzis_bez_przeszkod() {
    let mut e = Engine::new(Settings::default(), 400.0);
    let mut b = Atrapa::nowa();
    e.on_message(&mut b, &wiad(1, WEJSCIE_BEZ_SL));
    for (i, px) in [3997.0, 3995.0, 3993.0, 3989.0].iter().enumerate() {
        b.cena(TS0 + (i as i64 + 1) * 60_000, *px);
        let q = b.quote();
        e.on_tick(&mut b, &q);
    }
    let bez_sl = b.positions().iter().filter(|p| p.sl.is_none()).count();
    assert!(
        bez_sl > 0,
        "sygnał bez SL nie wyprodukował pozycji bez SL — jeśli to naprawiono, \
         zaktualizuj ten test i wpisz nowy stan N15 w RAPORT.md"
    );
    assert!(
        e.baskets.iter().any(|x| x.sl.is_none()),
        "koszyk bez SL nie powstał — scenariusz testu przestał być tym scenariuszem"
    );
}

#[test]
fn n15_dozor_widzi_pozycje_bez_sl_nie_dotykajac_jej() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();
    e.on_message(&mut b, &wiad(1, WEJSCIE_BEZ_SL));
    for (i, px) in [3997.0, 3995.0, 3993.0, 3989.0].iter().enumerate() {
        b.cena(TS0 + (i as i64 + 1) * 60_000, *px);
        let q = b.quote();
        e.on_tick(&mut b, &q);
    }
    assert!(
        e.ea.widziane_bez_sl > 0,
        "dozór nie zauważył pozycji bez SL"
    );
    assert_eq!(
        e.ea.dostawione_sl, 0,
        "dozór wyłączony, a jednak coś dostawił"
    );
    let bl = e.ea.bilans();
    assert!(
        bl.ile(KodPominiecia::BrakZrodlaSl) > 0,
        "brak kodu `BrakZrodlaSl`: {:?}",
        bl
    );
    assert!(
        b.positions().iter().all(|p| p.sl.is_none()),
        "dozór ruszył stop mimo wyłącznika"
    );
}

/// **EGZEKUCJA N15:** przy `ea_dozor_sl = true` pozycja bez SL dostaje SL
/// SWOJEGO KOSZYKA — i ani grosza fantazji.
///
/// Scenariusz jest ten realny, nie wydumany: koszyk MA stop z sygnału,
/// a pozycja go nie ma. Tak wygląda późny fill i tak wygląda pozycja przejęta
/// po restarcie.
#[test]
fn n15_dozor_dostawia_stop_z_koszyka_gdy_wlaczony() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    c.ea_dozor_sl = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    // koszyk ma SL z sygnału
    let sl_koszyka = e.baskets[0].sl.expect("koszyk z sygnału ma stop");

    // pozycja wchodzi z pominięciem stopu (tak wygląda późny fill u brokera,
    // który przyjął zlecenie, ale odrzucił jego SL)
    b.positions.push(Position {
        ticket: 999,
        side: Side::Buy,
        volume: 0.01,
        open_price: 3993.0,
        open_ts: TS0 + 90_000,
        sl: None,
        tp: None,
        vsl: None,
        basket: Some(e.baskets[0].id),
        level: 0,
        frozen: false,
        peak_pts: 0.0,
        last_peak_ts: 0,
        is_runner: false,
        is_toucher: false,
        comment: String::new(),
    });

    b.cena(TS0 + 120_000, 3996.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);

    let p = b
        .positions()
        .iter()
        .find(|p| p.ticket == 999)
        .expect("pozycja żyje");
    assert_eq!(p.sl, Some(sl_koszyka), "dozór nie dostawił stopu z koszyka");
    assert!(
        e.ea.dostawione_sl >= 1,
        "licznik dostawionych stopów nie urósł"
    );
}

#[test]
fn n15_dozor_nie_rusza_sierot_ani_zamrozonych() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    c.ea_dozor_sl = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    let poz = |ticket: Ticket, basket: Option<u32>, frozen: bool| Position {
        ticket,
        side: Side::Buy,
        volume: 0.01,
        open_price: 3990.0,
        open_ts: TS0,
        sl: None,
        tp: None,
        vsl: None,
        basket,
        level: 0,
        frozen,
        peak_pts: 0.0,
        last_peak_ts: 0,
        is_runner: false,
        is_toucher: false,
        comment: String::new(),
    };
    b.positions.push(poz(101, None, false)); // sierota / bilet ręczny
    b.positions.push(poz(102, Some(1), true)); // zamrożona ręczną edycją

    b.cena(TS0 + 1_000, 3998.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);

    assert!(
        b.positions().iter().all(|p| p.sl.is_none()),
        "dozór ruszył cudzą pozycję"
    );
    let bl = e.ea.bilans();
    assert!(
        bl.ile(KodPominiecia::Sierota) > 0,
        "brak kodu Sierota: {bl:?}"
    );
    assert!(
        bl.ile(KodPominiecia::Zamrozona) > 0,
        "brak kodu Zamrozona: {bl:?}"
    );
    assert!(bl.domyka_sie(), "bilans się nie domyka: {bl:?}");
}

/// Odmowa brokera nie jest ciszą — ma swój kod i nie udaje sukcesu.
#[test]
fn n15_odmowa_brokera_ma_kod() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    c.ea_dozor_sl = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();
    b.odmawiaj_modyfikacji = true;

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    b.positions.push(Position {
        ticket: 999,
        side: Side::Buy,
        volume: 0.01,
        open_price: 3993.0,
        open_ts: TS0 + 90_000,
        sl: None,
        tp: None,
        vsl: None,
        basket: Some(e.baskets[0].id),
        level: 0,
        frozen: false,
        peak_pts: 0.0,
        last_peak_ts: 0,
        is_runner: false,
        is_toucher: false,
        comment: String::new(),
    });
    b.cena(TS0 + 120_000, 3996.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);

    let bl = e.ea.bilans();
    assert!(
        bl.ile(KodPominiecia::OdmowaBrokera) > 0,
        "odmowa brokera bez kodu: {bl:?}"
    );
    assert_eq!(e.ea.dostawione_sl, 0, "odmowa policzona jako sukces");
    assert!(bl.domyka_sie(), "bilans się nie domyka: {bl:?}");
}

// ===========================================================================
//  N10 — ZAPADKA STEMPLA
// ===========================================================================

/// Stempel powstaje przy PIERWSZYM zobaczeniu koszyka i nosi chwilę
/// ZAWIĄZANIA (`created_ts`), nie chwilę stemplowania.
#[test]
fn n10_stempel_powstaje_z_chwila_zawiazania_koszyka() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);

    let id = e.baskets[0].id;
    let created = e.baskets[0].created_ts;
    let s = e.ea.stempel(id).expect("koszyk musi być ostemplowany");
    assert_eq!(
        s.ts, created,
        "stempel udaje, że koszyk powstał w chwili pulsu"
    );
    assert!(
        s.ryzyko_stempla > 0.0,
        "koszyk z SL ma mierzalne ryzyko: {s:?}"
    );
    assert_eq!(
        s.etap,
        EtapKoszyka::Uzbrojony,
        "siatka stoi, nic nie zafillowane: {s:?}"
    );
}

/// **N10 — DETEKTOR ZAPADKI.** Podniesienie ryzyka po zawiązaniu koszyka
/// (dokładnie to, co robi `pending_relot_up` po wzroście salda) zostaje
/// wykryte i zaksięgowane kodem `ZapadkaZlamana`.
///
/// Test symuluje podniesienie wprost — zwiększając wolumen leżących warstw —
/// bo ścieżka relotu ma własne bramki czasowe i saldowe, a badanym tu
/// zjawiskiem jest sam **wzrost ryzyka**, nie droga do niego.
#[test]
fn n10_wzrost_ryzyka_po_zawiazaniu_jest_wykrywany() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    assert_eq!(
        e.ea.bilans().ile(KodPominiecia::ZapadkaZlamana),
        0,
        "zapadka krzyczy, zanim cokolwiek urosło"
    );

    // PODNIESIENIE RYZYKA W TRAKCIE OTWARTEGO KOSZYKA — tylnymi drzwiami
    for g in e.baskets[0].levels.iter_mut() {
        g.volume = if g.volume > 0.0 { g.volume * 3.0 } else { 0.03 };
    }

    b.cena(TS0 + 120_000, 3996.5);
    let q = b.quote();
    e.on_tick(&mut b, &q);

    assert!(
        e.ea.bilans().ile(KodPominiecia::ZapadkaZlamana) > 0,
        "wzrost ryzyka po zawiązaniu przeszedł niezauważony: {:?}",
        e.ea.bilans()
    );
    assert_eq!(
        e.ea.koszyki_zapadka, 1,
        "jeden koszyk ponad stemplem = jeden koszyk w liczniku"
    );
    assert!(
        e.ea.zapadka_max_nadwyzka_usd > 0.0,
        "nadwyżka nie została zmierzona"
    );
}

/// **MIARA AKCEPTACJI N10 LICZY KOSZYKI, NIE PULSY.**
///
/// `EA_ARBITER_SPEC` N10 mówi wprost: metryką jest „licznik koszyków, których
/// ryzyko wzrosło po zawiązaniu = 0". Licznik zdarzeń tego nie mierzy — jest
/// funkcją KADENCJI: ten sam handel na STORM 2408b dał **26 800 zdarzeń przy
/// `ea_tick_s = 0` i 463 przy `ea_tick_s = 5`**. Ten test pilnuje, żeby liczba
/// raportowana jako naruszenie niezmiennika nie zależała od zegara.
#[test]
fn n10_licznik_koszykow_nie_zalezy_od_kadencji_pulsu() {
    let policz = |tick_s: f64| -> (u64, u64) {
        let mut c = Settings::default();
        c.ea_enabled = true;
        c.ea_tick_s = tick_s;
        let mut e = Engine::new(c, 400.0);
        let mut b = Atrapa::nowa();
        e.on_message(&mut b, &wiad(1, WEJSCIE));
        b.cena(TS0 + 60_000, 3997.0);
        let q = b.quote();
        e.on_tick(&mut b, &q);
        for g in e.baskets[0].levels.iter_mut() {
            g.volume = if g.volume > 0.0 { g.volume * 3.0 } else { 0.03 };
        }
        // dwadzieścia ticków po sekundzie: przy `ea_tick_s = 0` puls leci
        // dwadzieścia razy, przy `5` — cztery razy
        for i in 2..22i64 {
            b.cena(TS0 + i * 1_000, 3996.5);
            let q = b.quote();
            e.on_tick(&mut b, &q);
        }
        (
            e.ea.koszyki_zapadka,
            e.ea.bilans().ile(KodPominiecia::ZapadkaZlamana),
        )
    };
    let (kosz0, zdarz0) = policz(0.0);
    let (kosz5, zdarz5) = policz(5.0);
    assert_eq!(kosz0, 1, "koszyk ponad stemplem policzony inaczej niż raz");
    assert_eq!(
        kosz0, kosz5,
        "miara N10 zależy od kadencji: {kosz0} przy ea_tick_s=0 kontra {kosz5} przy 5"
    );
    assert!(
        zdarz0 > zdarz5,
        "licznik ZDARZEŃ ma zależeć od kadencji ({zdarz0} vs {zdarz5}) — inaczej ten test \
         nie sprawdza tego, o co chodzi"
    );
}

#[test]
fn n10_szum_ksiegowy_ponizej_procenta_nie_jest_zlamaniem() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    let stempel =
        e.ea.stempel(e.baskets[0].id)
            .expect("koszyk ostemplowany")
            .ryzyko_stempla;
    assert!(stempel > 0.0);

    for g in e.baskets[0].levels.iter_mut() {
        g.volume *= 1.005;
    }
    b.cena(TS0 + 120_000, 3996.5);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    assert_eq!(
        e.ea.koszyki_zapadka, 0,
        "pół procenta rozjazdu księgowego zgłoszone jako złamanie zapadki (stempel {stempel:.2} $)"
    );

    // ...a dziesięć procent to już jest sygnał, nie szum
    for g in e.baskets[0].levels.iter_mut() {
        g.volume *= 1.10;
    }
    b.cena(TS0 + 180_000, 3996.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    assert_eq!(
        e.ea.koszyki_zapadka, 1,
        "dziesięć procent ponad stempel przeszło niezauważone"
    );
}

/// Zapadka **nie krzyczy przy redukcji** — obniżenie ryzyka jest zawsze wolne
/// i ma działać natychmiast. Bez tego testu detektor byłby alarmem na wszystko.
#[test]
fn n10_redukcja_ryzyka_nie_jest_zlamaniem_zapadki() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);

    for g in e.baskets[0].levels.iter_mut() {
        g.volume = if g.volume > 0.0 { g.volume / 3.0 } else { 0.0 };
    }
    b.cena(TS0 + 120_000, 3996.5);
    let q = b.quote();
    e.on_tick(&mut b, &q);

    assert_eq!(
        e.ea.bilans().ile(KodPominiecia::ZapadkaZlamana),
        0,
        "redukcja ryzyka zgłoszona jako złamanie zapadki: {:?}",
        e.ea.bilans()
    );
}

// ===========================================================================
//  N18 — NIC NIE GINIE BEZ ŚLADU
// ===========================================================================

/// Bilans domyka się DOKŁADNIE na całym przebiegu, w każdym wariancie
/// ustawień. `zgubione_bez_sladu` to ta sama liczba, która złapała rozjazd
/// lejka (−102) — tutaj musi być zerem zawsze.
#[test]
fn n18_bilans_domyka_sie_na_calym_przebiegu() {
    for (etykieta, dozor) in [("dozór wyłączony", false), ("dozór włączony", true)] {
        let mut c = Settings::default();
        c.ea_enabled = true;
        c.ea_dozor_sl = dozor;
        let mut e = Engine::new(c, 400.0);
        let mut b = Atrapa::nowa();

        e.on_message(&mut b, &wiad(1, WEJSCIE));
        e.on_message(&mut b, &wiad(2, WEJSCIE_BEZ_SL));
        for (i, px) in [3997.0, 3995.0, 3993.0, 3989.0, 3992.0, 3998.0, 4004.0]
            .iter()
            .enumerate()
        {
            b.cena(TS0 + (i as i64 + 1) * 60_000, *px);
            let q = b.quote();
            e.on_tick(&mut b, &q);
        }
        let bl = e.ea.bilans();
        assert!(
            bl.domyka_sie(),
            "{etykieta}: bilans się nie domyka — {bl:?}"
        );
        assert_eq!(bl.zgubione_bez_sladu(), 0, "{etykieta}: {bl:?}");
        assert!(
            bl.rozpatrzone > 0,
            "{etykieta}: bilans pusty, test nic nie mierzy"
        );
    }
}

// ===========================================================================
//  N19 — RESTART NIE GUBI OCHRONY
// ===========================================================================

/// **Przed odtworzeniem warstwa ODMAWIA.** Świeży rdzeń nie jest gotowy
/// i mówi to kodem `NiegotowyRdzen`, a nie ciszą.
#[test]
fn n19_przed_odtworzeniem_warstwa_odmawia() {
    let e = Engine::new(Settings::default(), 400.0);
    assert!(!e.ea.gotowy(), "świeży rdzeń nie ma prawa być gotowy");
    assert_eq!(e.ea.wolno_dzialac(), Err(KodPominiecia::NiegotowyRdzen));
}

/// **Restart odtwarza ochronę.** Silnik podniesiony na koszykach przejętych
/// od brokera (tak wygląda wznowienie) stempluje je na PIERWSZYM pulsie —
/// razem z ryzykiem i etapem — i dopiero wtedy otwiera bramę.
///
/// Sedno: stempel nie powstaje „od nowa z dzisiejszym budżetem". Bierze
/// `created_ts` i `risk_initial_usd` koszyka, więc zapadka po restarcie
/// obowiązuje TĘ SAMĄ granicę co przed nim.
#[test]
fn n19_restart_odtwarza_stemple_i_otwiera_brame() {
    // --- faza 1: silnik pracuje i zawiązuje koszyk ---
    let mut c = Settings::default();
    c.ea_enabled = true;
    let mut e = Engine::new(c.clone(), 400.0);
    let mut b = Atrapa::nowa();
    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    let koszyki_przed = e.baskets.clone();
    let stempel_przed = *e
        .ea
        .stempel(koszyki_przed[0].id)
        .expect("stempel przed restartem");

    // --- faza 2: RESTART. Nowy silnik, te same koszyki, ten sam broker ---
    let mut e2 = Engine::new(c, 400.0);
    e2.baskets = koszyki_przed.clone();
    assert!(!e2.ea.gotowy(), "po restarcie rdzeń musi być NIEGOTOWY");
    assert_eq!(e2.ea.wolno_dzialac(), Err(KodPominiecia::NiegotowyRdzen));
    assert!(
        e2.ea.stempel(koszyki_przed[0].id).is_none(),
        "stempel nie ma się wziąć znikąd"
    );

    // pierwszy puls (tu: z zegara — czyli droga, którą idzie warstwa żywa)
    assert!(
        e2.ea_zegar(&mut b, TS0 + 120_000),
        "pierwszy puls po restarcie musi wejść"
    );
    assert!(e2.ea.gotowy(), "puls nie odtworzył stanu");
    assert_eq!(e2.ea.wolno_dzialac(), Ok(()));

    let s = e2
        .ea
        .stempel(koszyki_przed[0].id)
        .expect("stempel po restarcie");
    assert_eq!(
        s.ts, stempel_przed.ts,
        "restart przesunął chwilę zawiązania — zapadka wyzerowałaby się przy każdym restarcie"
    );
    assert_eq!(
        s.ryzyko_stempla, stempel_przed.ryzyko_stempla,
        "restart zmienił granicę zapadki"
    );
    assert_eq!(s.etap, stempel_przed.etap, "restart zgubił etap koszyka");
    // odmowa przed odtworzeniem ma ślad w bilansie
    assert!(
        e2.ea.bilans().ile(KodPominiecia::NiegotowyRdzen) > 0,
        "odmowa przed odtworzeniem bez śladu: {:?}",
        e2.ea.bilans()
    );
}

/// Restart pod ZAMROŻONYM strumieniem: ani jednego ticka, a ochrona wraca.
/// To jest złożenie N19 z Z13 — i dokładnie ten przypadek zdarza się realnie
/// (bot wstaje w weekend albo w przerwie kwotowań).
#[test]
fn n19_odtworzenie_dziala_takze_bez_ani_jednego_ticka() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    c.ea_tick_s = 30.0;
    let mut e = Engine::new(c.clone(), 400.0);
    let mut b = Atrapa::nowa();
    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    let koszyki = e.baskets.clone();

    let mut e2 = Engine::new(c, 400.0);
    e2.baskets = koszyki.clone();
    // ANI JEDEN `on_tick` — tylko zegar
    e2.ea_zegar(&mut b, TS0 + 200_000);
    assert!(
        e2.ea.gotowy(),
        "bez tików ochrona nie wróciła — to jest ta klasa błędu z rev_exit"
    );
    assert_eq!(
        e2.ea.pulsy_tick, 0,
        "test przypadkiem użył ticka i przestał mierzyć to, co miał"
    );
    assert!(e2.ea.stempel(koszyki[0].id).is_some());
}

// ===========================================================================
//  MASZYNA STANU KOSZYKA
// ===========================================================================

/// Etap koszyka rośnie razem z jego życiem: `Uzbrojony` (siatka stoi)
/// → `Pracuje` (pierwsza pozycja). Odczyt jest funkcją migawki, więc test
/// mierzy to, co warstwa naprawdę widzi.
#[test]
fn maszyna_stanu_koszyka_idzie_do_przodu() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    let id = e.baskets[0].id;
    assert_eq!(e.ea.stempel(id).unwrap().etap, EtapKoszyka::Uzbrojony);

    // cena schodzi na siatkę — limity się wypełniają
    for (i, px) in [3995.0, 3993.0, 3989.0].iter().enumerate() {
        b.cena(TS0 + (i as i64 + 2) * 60_000, *px);
        let q = b.quote();
        e.on_tick(&mut b, &q);
    }
    assert!(
        !b.positions().is_empty(),
        "siatka się nie wypełniła — scenariusz przestał działać"
    );
    assert_eq!(
        e.ea.stempel(id).unwrap().etap,
        EtapKoszyka::Pracuje,
        "etap nie doszedł do `Pracuje` mimo otwartych pozycji"
    );
}

#[test]
fn stempel_znika_razem_z_koszykiem() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    e.on_message(&mut b, &wiad(1, WEJSCIE));
    b.cena(TS0 + 60_000, 3997.0);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    assert_eq!(e.ea.stemple().len(), 1);

    e.baskets.clear();
    b.cena(TS0 + 120_000, 3997.5);
    let q = b.quote();
    e.on_tick(&mut b, &q);
    assert!(
        e.ea.stemple().is_empty(),
        "stempel przeżył koszyk: {:?}",
        e.ea.stemple()
    );
}

// ===========================================================================
//  DZIENNIK WARSTWY
// ===========================================================================

/// Zmiana stanu zostawia wpis z powodem, wartością sygnału i ŹRÓDŁEM pulsu.
/// Bez źródła nie da się później odpowiedzieć na pytanie „czy to zegar nas
/// obudził, czy rynek" — a to jest pierwsze pytanie przy każdej awarii
/// z rodziny „straż spała".
#[test]
fn dziennik_notuje_zmiane_stanu_ze_zrodlem() {
    let mut c = Settings::default();
    c.ea_enabled = true;
    c.ea_tick_s = 1.0;
    c.ea_state_src = conduit_core::settings::EaStateSrc::FloatPctEquity;
    c.ea_defense_enter = 0.0001;
    c.ea_defense_exit = 0.00001;
    let mut e = Engine::new(c, 400.0);
    let mut b = Atrapa::nowa();

    // atrapa daje equity == balance, więc floating == 0 i stan zostaje
    // Neutral — dziennik ma być PUSTY, a nie „prawie pusty"
    e.on_message(&mut b, &wiad(1, WEJSCIE));
    for s in 1..=5 {
        b.cena(TS0 + s * 60_000, 3997.0);
        let q = b.quote();
        e.on_tick(&mut b, &q);
    }
    assert!(
        e.ea.dziennik().is_empty(),
        "stan zmienił się bez powodu — floating = 0, a progi są dodatnie: {:?}",
        e.ea.dziennik()
    );
    assert_eq!(e.ea.stan(), conduit_core::ea::EaStan::Neutral);
    // źródło pulsów jest rozróżnialne
    assert!(e.ea.pulsy_tick > 0 && e.ea.pulsy_zegar == 0);
    assert!(e.ea_zegar(&mut b, TS0 + 999_000));
    assert_eq!(
        e.ea.pulsy_zegar, 1,
        "puls z zegara nie policzył się jako zegarowy"
    );
    let _ = ZrodloPulsu::Zegar; // typ jest publiczny — warstwa żywa go potrzebuje
}
