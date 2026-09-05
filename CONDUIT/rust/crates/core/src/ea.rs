
use std::fmt;

use crate::broker::Broker;
use crate::settings::{EaRatchet, EaStateSrc, Settings};
use crate::types::{Basket, BasketState, Px, Ticket, Ts, XAU_CONTRACT};

// ============================================================================
//  STAN WARSTWY
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EaStan {
    /// mniej ryzyka niż zwykle — węższy trailing, brak dokładek
    Obrona,
    /// zachowanie dzisiejsze, wszystkie modulatory = 1,0
    Neutral,
    /// więcej luzu, ale WYŁĄCZNIE dla koszyka już zabezpieczonego
    Agresja,
}

#[inline]
fn minut_do_zamkniecia_tygodnia(ts: Ts, godzina_utc: f64) -> f64 {
    // Epoka Uniksa zaczyna sie w CZWARTEK, wiec przesuniecie o 4 dni daje
    // tydzien liczony od poniedzialku — i dopiero wtedy `dzien` 4 to piatek.
    let sekundy = (ts / 1000) as f64;
    let dni_od_epoki = (sekundy / 86_400.0).floor();
    let dzien = ((dni_od_epoki as i64) + 4).rem_euclid(7);
    let sek_w_dobie = sekundy - dni_od_epoki * 86_400.0;
    let do_zamkniecia_dzis = godzina_utc * 3600.0 - sek_w_dobie;
    match dzien {
        4 if do_zamkniecia_dzis > 0.0 => do_zamkniecia_dzis / 60.0,
        3 => (do_zamkniecia_dzis + 86_400.0) / 60.0,
        _ => f64::INFINITY,
    }
}

impl EaStan {
    /// Porządek OSTROŻNOŚCI: Obrona jest najostrożniejsza.
    ///
    /// Potrzebny zapadce: „przejście w mniej ostrożny stan NIE przywraca
    /// parametrów w koszyku, który już jest otwarty".
    #[inline]
    pub fn ostroznosc(self) -> u8 {
        match self {
            EaStan::Obrona => 2,
            EaStan::Neutral => 1,
            EaStan::Agresja => 0,
        }
    }

    /// Stabilny kod do dziennika i do testów. Nie zmieniać po wydaniu.
    #[inline]
    pub fn kod(self) -> &'static str {
        match self {
            EaStan::Obrona => "OBRONA",
            EaStan::Neutral => "NEUTRAL",
            EaStan::Agresja => "AGRESJA",
        }
    }
}

impl fmt::Display for EaStan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kod())
    }
}

/// Skąd przyszedł puls zarządzania.
///
/// Rozróżnienie nie jest kosmetyczne: to JEDYNY sposób udowodnienia, że
/// warstwa żyje przy zamrożonym strumieniu. Licznik [`EaRdzen::pulsy_zegar`]
/// rosnący bez ani jednego ticka jest treścią testu Z13.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZrodloPulsu {
    /// obudził nas tick z rynku
    Tick,
    /// obudził nas własny zegar warstwy (`ea_tick_s`)
    Zegar,
}

impl ZrodloPulsu {
    #[inline]
    pub fn kod(self) -> &'static str {
        match self {
            ZrodloPulsu::Tick => "TICK",
            ZrodloPulsu::Zegar => "ZEGAR",
        }
    }
}


/// Wektor stanu czytany RAZ na puls zarządzania.
///
/// Raz — bo sześć rodzin czytających konto osobno zbudowałoby sześć różnych
/// migawek jednej chwili i progi zaczęłyby ze sobą migotać. To jest właśnie
/// powód, dla którego EA-CORE musi powstać PRZED rodzinami B–G.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WektorStanu {
    /// znacznik czasu pulsu (ms)
    pub ts: Ts,
    /// `E` — equity
    pub equity: f64,
    /// `B` — saldo
    pub balance: f64,
    /// `C` — surowy kredyt bonusowy; MT5 raportuje go oddzielnie od salda.
    pub credit: f64,
    /// `K` — equity własne: ON respektuje ręczne nadpisanie C; OFF legacy E−C.
    pub wlasne: f64,
    /// `M` — margines użyty
    pub margines: f64,
    /// `FM` — wolny margines
    pub wolny_margines: f64,
    /// `ML` — poziom marginesu w % (`f64::INFINITY`, gdy `M = 0`)
    pub ml: f64,
    /// `F` — floating = `E − B − C` w modelu MT5, historycznie `E − B`.
    pub floating: f64,
    /// `F_pct` — floating jako % equity
    pub floating_pct: f64,
    /// `n_pos` / `n_bask` / `n_pend`
    pub pozycje: usize,
    pub koszyki: usize,
    pub zlecenia: usize,
    pub ryzyko_usd: f64,
    /// `R_pct` — ryzyko portfela jako % equity
    pub ryzyko_pct: f64,
}

impl WektorStanu {
    /// Buduje wektor z JEDNEGO odczytu rachunku.
    ///
    /// ⚠ Wołane WYŁĄCZNIE spod `ea_enabled = true`. Przy wyłączonej warstwie
    /// rachunek nie ma prawa zostać odczytany ani razu więcej niż dziś —
    /// to jest punkt (c) kontraktu zera i sprawdza go osobny test
    /// (`atrapa` licząca wywołania `account()`).
    pub fn zbierz<B: Broker>(b: &B, cfg: &Settings, koszyki: usize, ts: Ts) -> Self {
        let a = b.account();
        let ml = if a.margin > 0.0 {
            a.equity / a.margin * 100.0
        } else {
            f64::INFINITY
        };
        let wlasne = if cfg.credit_balance_separate {
            // EA risk equity intentionally remains Equity-based, independent
            // from the strategy's Balance/Equity lot selector. Respect manual C.
            a.equity - cfg.kredyt_skuteczny_z(a.credit)
        } else if cfg.odlicz_kredyt {
            a.equity - a.credit
        } else {
            a.equity
        };
        let floating = if cfg.credit_balance_separate {
            // Actual floating uses raw broker credit, not a sizing override.
            a.equity - a.balance - a.credit
        } else { a.equity - a.balance };
        let mut ryzyko = 0.0_f64;
        for p in b.positions() {
            if p.frozen {
                continue;
            }
            match p.sl.or(p.vsl) {
                Some(sl) => ryzyko += (p.open_price - sl).abs() * XAU_CONTRACT * p.volume,
                None => ryzyko = f64::INFINITY,
            }
        }
        for o in b.pendings() {
            if o.frozen {
                continue;
            }
            match o.sl {
                Some(sl) => ryzyko += (o.price - sl).abs() * XAU_CONTRACT * o.volume,
                None => ryzyko = f64::INFINITY,
            }
        }
        WektorStanu {
            ts,
            equity: a.equity,
            balance: a.balance,
            credit: a.credit,
            wlasne,
            margines: a.margin,
            wolny_margines: a.free_margin,
            ml,
            floating,
            floating_pct: if a.equity != 0.0 {
                floating / a.equity * 100.0
            } else {
                0.0
            },
            pozycje: b.positions().len(),
            koszyki,
            zlecenia: b.pendings().len(),
            ryzyko_usd: ryzyko,
            ryzyko_pct: if a.equity > 0.0 {
                ryzyko / a.equity * 100.0
            } else {
                0.0
            },
        }
    }
}

// ============================================================================
//  MASZYNA STANU KOSZYKA  (jawna, bo przeskok etapu ma być NIEMOŻLIWY)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EtapKoszyka {
    /// plan jest, ale u brokera nie leży jeszcze nic
    Planowany,
    /// zlecenia albo plan siatki istnieją, nic się nie wypełniło
    Uzbrojony,
    /// koszyk KIEDYKOLWIEK miał pozycję (`had_positions` — zatrzask)
    Pracuje,
    /// koszyk uwolniony od ryzyka (`secured` / RISK FREE — zatrzask)
    Zabezpieczony,
    /// zamknięty definitywnie (`BasketState::Done` — stan końcowy)
    Zamkniety,
}

impl EtapKoszyka {
    /// Od tego szczebla w górę zejście jest AWARIĄ, nie zdarzeniem.
    pub const PIERWSZY_ZATRZASK: EtapKoszyka = EtapKoszyka::Pracuje;

    #[inline]
    pub fn kod(self) -> &'static str {
        match self {
            EtapKoszyka::Planowany => "PLANOWANY",
            EtapKoszyka::Uzbrojony => "UZBROJONY",
            EtapKoszyka::Pracuje => "PRACUJE",
            EtapKoszyka::Zabezpieczony => "ZABEZPIECZONY",
            EtapKoszyka::Zamkniety => "ZAMKNIETY",
        }
    }

    /// Odczyt etapu z migawki koszyka. Czysta funkcja — bez efektów ubocznych,
    /// żeby dała się testować bez brokera.
    ///
    /// Kolejność warunków jest wiążąca i idzie OD KOŃCA życia: koszyk `Done`
    /// jest zamknięty niezależnie od tego, co jeszcze wisi w wektorach,
    /// a koszyk `secured` jest zabezpieczony niezależnie od tego, ile ma
    /// pozycji. Odwrotna kolejność dawałaby „zabezpieczony po zamknięciu".
    ///
    /// `Pracuje` czyta `had_positions`, **nie** `!tickets.is_empty()` — i to
    /// jest cała różnica między drabiną a migotaniem: koszyk między
    /// zamknięciem transzy a ponownym wejściem ma zero biletów, a pracować
    /// nie przestał.
    pub fn z_koszyka(bk: &Basket) -> Self {
        if bk.state == BasketState::Done {
            return EtapKoszyka::Zamkniety;
        }
        if bk.secured || bk.state == BasketState::RiskFree {
            return EtapKoszyka::Zabezpieczony;
        }
        if bk.had_positions || !bk.tickets.is_empty() {
            return EtapKoszyka::Pracuje;
        }
        if !bk.pendings.is_empty() || !bk.levels.is_empty() {
            return EtapKoszyka::Uzbrojony;
        }
        EtapKoszyka::Planowany
    }
}

// ============================================================================
//  ZAPADKA STEMPLA  (N10)
// ============================================================================

/// STEMPEL koszyka — budżet ryzyka obowiązujący **w chwili zawiązania**.
///
/// Każda decyzja dotycząca istniejącego koszyka — dokładka, relot, piramida,
/// rearm, re-entry — ma czytać STEMPEL, nie stan bieżący. Podniesienie
/// alokacji działa tylko na koszyki zawiązane PO podniesieniu; obniżenie
/// działa natychmiast i na wszystko.
///
/// ⚠ **Pułapka nazwana wprost:** `pending_relot_up = true` (włączone dziś
/// w STORM-1 i TYLER-1) przelicza wolumen wiszących zleceń po wzroście salda —
/// to jest podniesienie ryzyka w trakcie otwartego koszyka, tylnymi drzwiami.
/// Detektor niżej ma to WIDZIEĆ; egzekucja (relot w górę czyta stempel)
/// należy do fali, która dotknie ścieżki relotu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StempelKoszyka {
    pub id: u32,
    /// chwila ostemplowania (pierwsze zobaczenie koszyka przez warstwę)
    pub ts: Ts,
    /// ryzyko PEŁNEGO planu w chwili zawiązania ($)
    pub ryzyko_stempla: f64,
    /// stan warstwy w chwili zawiązania — do zapadki `NieLuzujWKoszyku`
    pub stan: EaStan,
    /// najwyższe ryzyko, jakie ten koszyk kiedykolwiek pokazał ($)
    pub ryzyko_szczyt: f64,
    /// ostatni zaobserwowany etap — do wykrywania regresji
    pub etap: EtapKoszyka,
    pub zapadka_zgloszona: bool,
}

/// Ryzyko PLANOWANE koszyka: to, co jeszcze może się zdarzyć.
///
/// Suma po żywych warstwach siatki (niewypełnionych i nieanulowanych) oraz po
/// otwartych pozycjach, licząc odległość do stop-lossa koszyka. Koszyk bez SL
/// nie ma mierzalnego ryzyka i zwraca `None` — **nie zero**. To rozróżnienie
/// jest treścią znaleziska: dziś bramki odrzucającej sygnał bez SL nie ma
/// nigdzie, więc taki koszyk omija limit koszykowy, portfelowy i skalę
/// rynkową NARAZ. Zliczenie go jako „ryzyko 0" ukryłoby to.
pub fn ryzyko_planowane<B: Broker>(bk: &Basket, b: &B) -> Option<f64> {
    let sl = bk.sl?;
    let mut r = ryzyko_planu(bk, sl);
    for p in b.positions() {
        if p.basket != Some(bk.id) {
            continue;
        }
        r += ryzyko_pozycji(p.open_price, p.sl.unwrap_or(sl), p.volume);
    }
    Some(r)
}

#[inline]
fn ryzyko_planu(bk: &Basket, sl: Px) -> f64 {
    let mut r = 0.0;
    for g in &bk.levels {
        if g.filled || g.cancelled {
            continue;
        }
        let vol = if g.volume > 0.0 { g.volume } else { 0.0 };
        r += (g.price - sl).abs() * XAU_CONTRACT * vol * g.base_units.max(1) as f64;
    }
    r
}

#[inline]
fn ryzyko_pozycji(open_price: Px, sl: Px, volume: f64) -> f64 {
    (open_price - sl).abs() * XAU_CONTRACT * volume
}

// ============================================================================
//  BILANS PULSU  (N18 — nic nie ginie bez śladu)
// ============================================================================

/// Kod pominięcia. Każda pozycja, której warstwa NIE obsłużyła, ma dokładnie
/// jeden taki kod — inaczej bilans się nie domyka i test pęka.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KodPominiecia {
    /// rdzeń nie odtworzył jeszcze stanu po restarcie (N19)
    NiegotowyRdzen,
    /// pozycja zamrożona ręczną edycją — bot jej nie rusza
    Zamrozona,
    /// pozycja nie należy do żadnego koszyka tej nogi (sierota / bilet ręczny)
    Sierota,
    /// pozycja bez SL, a jej koszyk też nie ma z czego go wziąć (N15)
    BrakZrodlaSl,
    /// pozycja bez SL, dozór wyłączony (`ea_dozor_sl = false`) — sam ślad
    DozorWylaczony,
    /// broker odmówił modyfikacji
    OdmowaBrokera,
    /// koszyk zniknął między zebraniem migawki a decyzją
    KoszykZnikl,
    /// etap koszyka cofnął się — stan obserwowany nie jest monotoniczny
    RegresjaEtapu,
    /// ryzyko koszyka wzrosło po zawiązaniu (N10)
    ZapadkaZlamana,
}

impl KodPominiecia {
    pub const WSZYSTKIE: [KodPominiecia; 9] = [
        KodPominiecia::NiegotowyRdzen,
        KodPominiecia::Zamrozona,
        KodPominiecia::Sierota,
        KodPominiecia::BrakZrodlaSl,
        KodPominiecia::DozorWylaczony,
        KodPominiecia::OdmowaBrokera,
        KodPominiecia::KoszykZnikl,
        KodPominiecia::RegresjaEtapu,
        KodPominiecia::ZapadkaZlamana,
    ];

    #[inline]
    pub fn kod(self) -> &'static str {
        match self {
            KodPominiecia::NiegotowyRdzen => "NiegotowyRdzen",
            KodPominiecia::Zamrozona => "Zamrozona",
            KodPominiecia::Sierota => "Sierota",
            KodPominiecia::BrakZrodlaSl => "BrakZrodlaSl",
            KodPominiecia::DozorWylaczony => "DozorWylaczony",
            KodPominiecia::OdmowaBrokera => "OdmowaBrokera",
            KodPominiecia::KoszykZnikl => "KoszykZnikl",
            KodPominiecia::RegresjaEtapu => "RegresjaEtapu",
            KodPominiecia::ZapadkaZlamana => "ZapadkaZlamana",
        }
    }

    #[inline]
    fn idx(self) -> usize {
        match self {
            KodPominiecia::NiegotowyRdzen => 0,
            KodPominiecia::Zamrozona => 1,
            KodPominiecia::Sierota => 2,
            KodPominiecia::BrakZrodlaSl => 3,
            KodPominiecia::DozorWylaczony => 4,
            KodPominiecia::OdmowaBrokera => 5,
            KodPominiecia::KoszykZnikl => 6,
            KodPominiecia::RegresjaEtapu => 7,
            KodPominiecia::ZapadkaZlamana => 8,
        }
    }
}

/// Księgowość jednego pulsu: `rozpatrzone = obsłużone + pominięte_z_kodem`.
///
/// To ta sama reguła, która złapała rozjazd lejka (`zgubione_bez_sladu = −102`
/// jako sygnał alarmowy). Bilans jest liczony NARASTAJĄCO przez cały przebieg,
/// bo pojedynczy puls domyka się trywialnie, a interesuje nas, czy domyka się
/// **zawsze**.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BilansPulsu {
    /// ile bytów (pozycji + koszyków) warstwa w ogóle wzięła pod uwagę
    pub rozpatrzone: u64,
    /// ile z nich obsłużyła (dziś: zaobserwowała i zaksięgowała)
    pub obsluzone: u64,
    /// ile pominęła, każde z kodem
    pub pominiete: u64,
    /// rozbicie pominięć po kodach
    pub kody: [u64; 9],
}

impl BilansPulsu {
    #[inline]
    fn obsluz(&mut self) {
        self.rozpatrzone += 1;
        self.obsluzone += 1;
    }

    #[inline]
    fn pomin(&mut self, k: KodPominiecia) {
        self.rozpatrzone += 1;
        self.pominiete += 1;
        self.kody[k.idx()] += 1;
    }

    /// Sam kod, bez zwiększania `rozpatrzone` — dla zdarzeń, które są
    /// OBSERWACJĄ o bycie już policzonym (regresja etapu, złamana zapadka).
    #[inline]
    fn odnotuj(&mut self, k: KodPominiecia) {
        self.kody[k.idx()] += 1;
    }

    /// N18: bilans musi się domykać DOKŁADNIE, bez reszty.
    #[inline]
    pub fn domyka_sie(&self) -> bool {
        self.rozpatrzone == self.obsluzone + self.pominiete
    }

    /// Ile razy padł dany kod.
    #[inline]
    pub fn ile(&self, k: KodPominiecia) -> u64 {
        self.kody[k.idx()]
    }

    /// Ile bytów zginęło bez śladu (musi być 0).
    #[inline]
    pub fn zgubione_bez_sladu(&self) -> i64 {
        self.rozpatrzone as i64 - self.obsluzone as i64 - self.pominiete as i64
    }
}

// ============================================================================
//  DZIENNIK ZMIAN STANU
// ============================================================================

/// Jeden wpis dziennika warstwy — zmiana stanu z POWODEM i wartościami wejść.
#[derive(Debug, Clone, PartialEq)]
pub struct WpisEa {
    pub ts: Ts,
    pub z: EaStan,
    pub na: EaStan,
    /// sygnał `x` wg `ea_state_src` w chwili przejścia
    pub x: f64,
    /// źródło pulsu, który wywołał przejście
    pub zrodlo: ZrodloPulsu,
    pub powod: &'static str,
}

/// Ile wpisów dziennika warstwa trzyma w pamięci. Sufit jest twardy, bo rdzeń
/// nie ma dostępu do dysku — a przebieg 24 mln ticków bez sufitu zjadłby RAM.
const SUFIT_DZIENNIKA: usize = 512;

// ============================================================================
//  MODULATORY  (haki dla rodzin A–G — DZIŚ PUSTE)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Modulatory {
    /// mnożnik jednostek nowego koszyka (punkt 2 kolejności)
    pub jednostki: f64,
    /// mnożnik odstępu trailingu (< 1 = ciaśniej)
    pub trailing: f64,
    /// czy wolno DOKŁADAĆ do koszyka (dokładka, re-entry, piramida, rearm)
    pub dokladki: bool,
}

impl Modulatory {
    /// Stan neutralny: same jedynki i zgoda. To jest wartość, którą warstwa
    /// zwraca ZAWSZE, dopóki rodziny B–G nie wejdą — i to jest połowa dowodu
    /// podwójnego zera.
    #[inline]
    pub const fn neutralne() -> Self {
        Modulatory {
            jednostki: 1.0,
            trailing: 1.0,
            dokladki: true,
        }
    }

    /// Czy modulatory są neutralne co do bitu.
    #[inline]
    pub fn sa_neutralne(&self) -> bool {
        self.jednostki == 1.0 && self.trailing == 1.0 && self.dokladki
    }
}

impl Default for Modulatory {
    fn default() -> Self {
        Modulatory::neutralne()
    }
}

// ============================================================================
//  RODZINA A — EKSPOZYCJA WOBEC STANU RACHUNKU  (FALA 1)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SufitEa {
    /// **A1** — absolutny sufit jednostek z wolnego marginesu i sufitu slotów.
    /// `u32::MAX` = oś A1 wyłączona (brak sufitu absolutnego).
    pub jednostki_max: u32,
    /// **A3 × A4** — mnożnik liczby jednostek planu. ZAWSZE `<= 1,0`;
    /// pilnuje tego [`SufitEa::nowy`], nie opis.
    pub mult: f64,
}

impl SufitEa {
    /// Konstruktor, który jest jedynym miejscem egzekwującym niezmiennik
    /// kierunku: **rodzina A nigdy nie podnosi ekspozycji.**
    #[inline]
    pub fn nowy(jednostki_max: u32, mult: f64) -> Self {
        let m = if mult.is_finite() {
            mult.clamp(0.0, 1.0)
        } else {
            1.0
        };
        SufitEa {
            jednostki_max,
            mult: m,
        }
    }

    /// Sufit, który nic nie robi — wartość odniesienia dla kontraktu zera.
    #[inline]
    pub const fn obojetny() -> Self {
        SufitEa {
            jednostki_max: u32::MAX,
            mult: 1.0,
        }
    }

    /// Czy sufit jest obojętny CO DO BITU (nie przycina ani jednej jednostki).
    #[inline]
    pub fn jest_obojetny(&self) -> bool {
        self.jednostki_max == u32::MAX && self.mult == 1.0
    }

    /// Ile jednostek zostaje z planu o `suma` jednostkach.
    ///
    /// Podłoga na JEDNEJ jednostce jest istotna i jest tą samą podłogą, którą
    /// ma `regime_soft_units_mult`: siatka o zerowej liczbie szczebli to
    /// koszyk, którego nie ma — czyli twarda blokada wejścia pod inną nazwą,
    /// a cała rodzina A ma pokrycie sygnałów zostawić nietknięte.
    ///
    /// Odmowa całego koszyka jest możliwa, ale zapada WYŻEJ i z własnym kodem
    /// (`jednostki_max == 0` w [`crate::Engine::place_grid`]) — żeby dało się
    /// odróżnić „margines nie pozwala na nic" od „plan przycięty".
    #[inline]
    pub fn docelowe_jednostki(&self, suma: u32) -> u32 {
        if suma == 0 {
            return 0;
        }
        let po_mult = if self.mult < 1.0 {
            ((suma as f64) * self.mult).floor().max(1.0) as u32
        } else {
            suma
        };
        po_mult.min(self.jednostki_max).max(1)
    }
}

impl Default for SufitEa {
    fn default() -> Self {
        SufitEa::obojetny()
    }
}

/// STAN I LICZNIKI RODZINY A. Zerowe u silnika, który osi nie używa.
///
/// Wszystko, co rodzina A pamięta między pulsami, siedzi TUTAJ i nigdzie
/// indziej — inaczej cztery osie zbudowałyby cztery własne migawki jednej
/// chwili, czyli dokładnie ten błąd, dla którego powstał EA-CORE.
#[derive(Debug, Clone, Default)]
pub struct StanRodzinyA {
    /// **A4** — ile STRATNYCH stopów padło w bieżącej dobie.
    ///
    /// Licznik jest BEZWARUNKOWY (rośnie także przy zamkniętej bramie): to
    /// jedno porównanie na zamkniętą transakcję, nie czyta rachunku i nie
    /// zmienia żadnej decyzji, a bez niego nie dałoby się powiedzieć, ile dni
    /// oś A4 W OGÓLE by uzbroiła — czyli czy zero jej wpływu to „nie działa",
    /// czy „nie miała okazji".
    pub stopy_dnia: u32,
    /// **A4** — najwięcej stratnych stopów w jednej dobie w całym przebiegu.
    pub stopy_dnia_max: u32,
    /// **A4** — ile dób uzbroiło stan dnia (liczone raz na dobę).
    pub dni_uzbrojone: u32,
    /// **A2** — koszyki z ZATRZAŚNIĘTYM wetem dokładek (histereza).
    pub stop_dokladek: std::collections::HashSet<u32>,
    /// ile razy weto A2 / A4 odmówiło dokładki
    pub weta_a2: u64,
    pub weta_a4: u64,
    /// ile koszyków rodzina A przycięła i o ile jednostek łącznie
    pub przyciete_koszyki: u64,
    pub sciete_jednostki: u64,
    /// ile koszyków ODMÓWIONO, bo nawet jedna jednostka nie mieściła się
    /// w budżecie marginesu (A1)
    pub odmowy_margines: u64,
    /// najmniejszy sufit A1 zaobserwowany w przebiegu (diagnostyka kalibracji)
    pub sufit_min: Option<u32>,
}

impl StanRodzinyA {
    /// Nowa doba: licznik stopów startuje od zera, tak samo jak `slhit_dnia`.
    /// Szczyt i liczba uzbrojonych dób ZOSTAJĄ — to są miary przebiegu.
    #[inline]
    pub fn nowa_doba(&mut self) {
        self.stopy_dnia = 0;
    }

    /// Czy oś A4 jest uzbrojona przy danym progu. Wydzielone, bo czyta to
    /// i planer (mnożnik jednostek), i weto dokładek — a dwie kopie warunku
    /// to dwie okazje, żeby się rozjechały.
    #[inline]
    pub fn dzien_uzbrojony(&self, prog: u32) -> bool {
        self.stopy_dnia >= prog
    }

    /// Czy rodzina A cokolwiek zrobiła w tym przebiegu. Jedna liczba do
    /// odpowiedzi na pytanie „czy tryb AUTO-EA robi już realną różnicę".
    #[inline]
    pub fn cokolwiek_zrobila(&self) -> bool {
        self.przyciete_koszyki > 0
            || self.odmowy_margines > 0
            || self.weta_a2 > 0
            || self.weta_a4 > 0
    }
}

// ============================================================================
//  BETA EA — PĘTLA DECYZYJNA (szkielet)
// ============================================================================
//

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::broker::{
    clamp_limit_price, limit_price_is_valid, pending_sl_is_valid, pending_tp_is_valid,
    stop_price_is_valid, tp_is_valid, OrderReq, PendingReq,
};
use crate::types::{CloseReason, PendingKind, Quote, Side};

/// Krok wolumenu brokera. Ta sama liczba, którą stosuje `Engine::round_lot` —
/// przepisana, a nie zaimportowana, bo tamta jest prywatna w `engine.rs`,
/// a ten moduł nie ma prawa tamtego pliku dotykać.
#[inline]
fn krok_lota(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ----------------------------------------------------------------- konfiguracja

/// WARIANT TAKTYKI BETY.
///
/// Szkielet nie przesądza treści wariantów `Z` i `P` — buduje je dwóch
/// osobnych autorów, każdy w swojej funkcji ([`EaRdzen::taktyka_z`],
/// [`EaRdzen::taktyka_p`]). Tu jest wyłącznie przełącznik, żeby oba warianty
/// dały się porównać JEDNYM przebiegiem różniącym się jedną literą.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WariantBety {
    /// beta wyłączona — pętla decyzyjna nie rusza ani jednego bajtu
    #[default]
    #[serde(alias = "OFF", alias = "Off", alias = "wylaczony", alias = "none")]
    Off,
    /// wariant Z
    #[serde(alias = "Z")]
    Z,
    /// wariant P
    #[serde(alias = "P")]
    P,
    /// wariant W — WETO
    #[serde(alias = "W", alias = "weto")]
    W,
}

impl WariantBety {
    #[inline]
    pub fn kod(self) -> &'static str {
        match self {
            WariantBety::Off => "OFF",
            WariantBety::Z => "Z",
            WariantBety::P => "P",
            WariantBety::W => "W",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KonfigBety {
    /// który wariant taktyki prowadzi ten przebieg
    pub wariant: WariantBety,
    pub format: String,
    /// czy odrzucać koszyki, których `source_name` nie zawiera [`KonfigBety::format`]
    pub dopasuj_source_name: bool,
    /// WŁASNY ZEGAR PĘTLI DECYZYJNEJ w sekundach (0 = na każdym pulsie).
    ///
    /// Osobny od `ea_tick_s`, bo to dwie różne rzeczy: tamten mówi, jak często
    /// warstwa **patrzy** (dozór SL, wektor stanu, zapadka), ten — jak często
    /// **decyduje**. Decyzja jest droższa (przelot po pozycjach i koszykach),
    /// a każda interwencja płaci spread, więc nie ma powodu, żeby chodziła
    /// z częstością ticka.
    pub kadencja_s: f64,
    /// TRYB OBSERWACYJNY: decyzje zapadają i są liczone, ale egzekutor nie
    /// wysyła do brokera ani jednego zlecenia.
    ///
    /// To jest najtańszy sposób odpowiedzi na pytanie „ile razy ta taktyka
    /// w ogóle miałaby co robić" — bez płacenia spreadu i bez zmiany wyniku.
    pub tylko_obserwuj: bool,
    pub max_akcji_koszyk: u32,
    /// najmniejszy odstęp między dwiema interwencjami na TYM SAMYM koszyku (s)
    pub min_odstep_akcji_s: f64,
    /// Czy wolno PRZESUNĄĆ STOP W STRONĘ WIĘKSZEGO RYZYKA.
    ///
    /// Domyślnie **nie** i egzekutor pilnuje tego strukturalnie, tak samo jak
    /// [`SufitEa::nowy`] pilnuje, że rodzina A nie podnosi ekspozycji.
    /// Rozszerzenie stopa to jest podniesienie ryzyka po zawiązaniu koszyka,
    /// czyli dokładnie to, czego zabrania zapadka N10.
    pub pozwol_luzowac_stop: bool,
    /// ile wpisów dziennika decyzji warstwa trzyma w pamięci
    pub dziennik_max: usize,
    /// czy wypisać podsumowanie bety na stderr przy końcu życia silnika
    pub raport: bool,
    /// PARAMETRY LICZBOWE TAKTYK — worek `nazwa → liczba`.
    ///
    /// Celowo nie jest to sztywna lista pól: dwaj autorzy wariantów pracują
    /// równolegle i każde sztywne pole byłoby konfliktem w tym samym pliku.
    /// Odczyt przez [`KonfigBety::p`], z wartością domyślną podawaną w miejscu
    /// użycia — więc parametr nieustawiony ma zachowanie opisane TAM, gdzie
    /// się go czyta, a nie w odległej tabeli.
    pub param: BTreeMap<String, f64>,
    /// parametry tekstowe taktyk (tryby, nazwy)
    pub tekst: BTreeMap<String, String>,
    /// NAZWY 16 POWODÓW TAKTYCZNYCH ([`PowodAkcji::Taktyka`]) do dziennika.
    /// Pusta lista = w raporcie widać `TAKTYKA_0`…`TAKTYKA_15`.
    pub nazwy_powodow: Vec<String>,
}

impl Default for KonfigBety {
    fn default() -> Self {
        KonfigBety {
            wariant: WariantBety::Off,
            format: String::new(),
            dopasuj_source_name: false,
            kadencja_s: 0.0,
            tylko_obserwuj: false,
            max_akcji_koszyk: 0,
            min_odstep_akcji_s: 0.0,
            pozwol_luzowac_stop: false,
            dziennik_max: 512,
            raport: true,
            param: BTreeMap::new(),
            tekst: BTreeMap::new(),
            nazwy_powodow: Vec::new(),
        }
    }
}

impl KonfigBety {
    /// Parametr liczbowy z wartością domyślną podaną W MIEJSCU UŻYCIA.
    #[inline]
    pub fn p(&self, nazwa: &str, domyslnie: f64) -> f64 {
        self.param.get(nazwa).copied().unwrap_or(domyslnie)
    }

    /// Parametr tekstowy z wartością domyślną podaną w miejscu użycia.
    #[inline]
    pub fn t<'a>(&'a self, nazwa: &str, domyslnie: &'a str) -> &'a str {
        self.tekst
            .get(nazwa)
            .map(|s| s.as_str())
            .unwrap_or(domyslnie)
    }

    /// Czy parametr w ogóle ustawiono (odróżnia „zero" od „nie podano").
    #[inline]
    pub fn ma(&self, nazwa: &str) -> bool {
        self.param.contains_key(nazwa)
    }

    /// Nazwa powodu taktycznego do dziennika.
    pub fn nazwa_powodu(&self, n: u8) -> &str {
        self.nazwy_powodow
            .get(n as usize)
            .map(|s| s.as_str())
            .unwrap_or("")
    }
}

/// Nazwa zmiennej środowiskowej. Jedno miejsce, żeby dokumentacja i kod nie
/// mogły się rozjechać.
pub const ZMIENNA_BETY: &str = "CONDUIT_EA_BETA";

static BETA_ENV: OnceLock<Option<KonfigBety>> = OnceLock::new();

/// Konfiguracja bety z otoczenia procesu — czytana i parsowana **raz**.
///
/// `None` znaczy „beta wyłączona" i to jest jedyne znaczenie braku zmiennej.
pub fn konfig_bety_z_env() -> Option<&'static KonfigBety> {
    BETA_ENV.get_or_init(wczytaj_bete).as_ref()
}

fn wczytaj_bete() -> Option<KonfigBety> {
    let surowe = std::env::var(ZMIENNA_BETY).ok()?;
    let s = surowe.trim();
    if s.is_empty() {
        return None;
    }
    match serde_json::from_str::<KonfigBety>(s) {
        Ok(k) if k.wariant == WariantBety::Off => None,
        Ok(k) => Some(k),
        Err(e) => {
            // NIE cichy fallback: zła konfiguracja ma być słyszalna.
            eprintln!(
                "[EA-BETA] BŁĄD KONFIGURACJI {ZMIENNA_BETY}: {e}\n\
                 [EA-BETA] beta ZOSTAJE WYŁĄCZONA, przebieg jest zwykłym przebiegiem."
            );
            None
        }
    }
}

/// Leniwy uchwyt konfiguracji w rdzeniu. `Copy`, bo trzyma referencję
/// `'static` do zawartości [`BETA_ENV`] — dzięki temu odczyt konfiguracji nie
/// pożycza `self` i nie blokuje egzekutora, który potrzebuje `&mut self`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum UchwytBety {
    /// pierwszy puls jeszcze nie sięgnął po otoczenie
    #[default]
    Nieodczytany,
    /// odczytano: bety nie ma
    Brak,
    /// odczytano: beta działa
    Jest(&'static KonfigBety),
}

// ------------------------------------------------------------------- akcje

/// Rodzaj akcji BEZ ładunku — do indeksowania liczników i do dziennika.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KodAkcji {
    Trzymaj,
    ZamknijCalosc,
    ZamknijCzesc,
    PrzesunStop,
    PrzesunCel,
    DolozPozycje,
    AnulujOczekujace,
    WejdzPonownie,
}

pub const LICZBA_AKCJI: usize = 8;

impl KodAkcji {
    pub const WSZYSTKIE: [KodAkcji; LICZBA_AKCJI] = [
        KodAkcji::Trzymaj,
        KodAkcji::ZamknijCalosc,
        KodAkcji::ZamknijCzesc,
        KodAkcji::PrzesunStop,
        KodAkcji::PrzesunCel,
        KodAkcji::DolozPozycje,
        KodAkcji::AnulujOczekujace,
        KodAkcji::WejdzPonownie,
    ];

    #[inline]
    pub fn idx(self) -> usize {
        match self {
            KodAkcji::Trzymaj => 0,
            KodAkcji::ZamknijCalosc => 1,
            KodAkcji::ZamknijCzesc => 2,
            KodAkcji::PrzesunStop => 3,
            KodAkcji::PrzesunCel => 4,
            KodAkcji::DolozPozycje => 5,
            KodAkcji::AnulujOczekujace => 6,
            KodAkcji::WejdzPonownie => 7,
        }
    }

    #[inline]
    pub fn kod(self) -> &'static str {
        match self {
            KodAkcji::Trzymaj => "TRZYMAJ",
            KodAkcji::ZamknijCalosc => "ZAMKNIJ_CALOSC",
            KodAkcji::ZamknijCzesc => "ZAMKNIJ_CZESC",
            KodAkcji::PrzesunStop => "PRZESUN_STOP",
            KodAkcji::PrzesunCel => "PRZESUN_CEL",
            KodAkcji::DolozPozycje => "DOLOZ_POZYCJE",
            KodAkcji::AnulujOczekujace => "ANULUJ_OCZEKUJACE",
            KodAkcji::WejdzPonownie => "WEJDZ_PONOWNIE",
        }
    }
}

/// AKCJA EA — jedyny język, którym taktyka rozmawia ze światem.
///
/// Taktyka **nie dotyka brokera**. Zwraca akcję, a wykonuje ją jeden
/// egzekutor ([`EaRdzen::wykonaj_akcje`]) — dzięki temu obsługa odmowy,
/// `stops_level` i księgowość są w JEDNYM miejscu i nie da się ich pominąć
/// przez zapomnienie.
fn source_allows_action(basket: &Basket, action: &AkcjaEa) -> bool {
    !basket.entry_edit_state.as_ref().is_some_and(|s| s.cancelled_by_source_ts.is_some())
        || !matches!(action, AkcjaEa::DolozPozycje { .. } | AkcjaEa::WejdzPonownie { .. })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AkcjaEa {
    /// nic nie rób — i to jest decyzja, nie brak decyzji
    Trzymaj,
    /// zamknij wszystkie wskazane pozycje koszyka po rynku
    ZamknijCalosc,
    /// zainkasuj UŁAMEK (0..1) wolumenu każdej wskazanej pozycji
    ZamknijCzesc(f64),
    /// przesuń stop wskazanych pozycji na ten poziom
    PrzesunStop(Px),
    /// przesuń cel (TP) wskazanych pozycji na ten poziom
    PrzesunCel(Px),
    /// dołóż do ŻYJĄCEGO koszyka: `limit = None` znaczy po rynku
    DolozPozycje {
        side: Side,
        volume: f64,
        limit: Option<Px>,
        sl: Option<Px>,
        tp: Option<Px>,
        level: i32,
    },
    /// skasuj wszystkie oczekujące zlecenia koszyka
    AnulujOczekujace,
    /// wejdź PONOWNIE po tym, jak koszyk stracił pozycje (osobno od dokładki,
    /// bo to inna decyzja i musi być osobno policzalna)
    WejdzPonownie {
        side: Side,
        volume: f64,
        limit: Option<Px>,
        sl: Option<Px>,
        tp: Option<Px>,
    },
}

impl AkcjaEa {
    #[inline]
    pub fn kod(&self) -> KodAkcji {
        match self {
            AkcjaEa::Trzymaj => KodAkcji::Trzymaj,
            AkcjaEa::ZamknijCalosc => KodAkcji::ZamknijCalosc,
            AkcjaEa::ZamknijCzesc(_) => KodAkcji::ZamknijCzesc,
            AkcjaEa::PrzesunStop(_) => KodAkcji::PrzesunStop,
            AkcjaEa::PrzesunCel(_) => KodAkcji::PrzesunCel,
            AkcjaEa::DolozPozycje { .. } => KodAkcji::DolozPozycje,
            AkcjaEa::AnulujOczekujace => KodAkcji::AnulujOczekujace,
            AkcjaEa::WejdzPonownie { .. } => KodAkcji::WejdzPonownie,
        }
    }

    /// Czy akcja w ogóle rusza rachunek. `Trzymaj` nie rusza — i tylko dlatego
    /// wolno ją podejmować na każdym pulsie bez płacenia spreadu.
    #[inline]
    pub fn jest_bierna(&self) -> bool {
        matches!(self, AkcjaEa::Trzymaj)
    }
}

/// Ile jest powodów TAKTYCZNYCH do rozdania autorom wariantów.
pub const POWODY_TAKTYK: usize = 16;
/// Rozmiar tablicy liczników powodów.
pub const LICZBA_POWODOW: usize = 8 + POWODY_TAKTYK;

const NAZWY_TAKTYK: [&str; POWODY_TAKTYK] = [
    "TAKTYKA_0",
    "TAKTYKA_1",
    "TAKTYKA_2",
    "TAKTYKA_3",
    "TAKTYKA_4",
    "TAKTYKA_5",
    "TAKTYKA_6",
    "TAKTYKA_7",
    "TAKTYKA_8",
    "TAKTYKA_9",
    "TAKTYKA_10",
    "TAKTYKA_11",
    "TAKTYKA_12",
    "TAKTYKA_13",
    "TAKTYKA_14",
    "TAKTYKA_15",
];

/// POWÓD AKCJI — bez niego decyzja jest nie do obronienia po przebiegu.
///
/// Osiem powodów wspólnych plus szesnaście numerowanych slotów dla autorów
/// wariantów. Sloty są numerowane, a nie nazwane w kodzie, dokładnie po to,
/// żeby dwa równoległe warianty mogły dopisywać powody bez konfliktu w tym
/// samym pliku; nazwę do raportu podaje konfiguracja
/// ([`KonfigBety::nazwy_powodow`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowodAkcji {
    /// nic się nie dzieje (towarzyszy `Trzymaj`)
    Brak,
    /// akcja próbna szkieletu — dowód, że pętla żyje
    Szkielet,
    /// ochrona kapitału / ryzyko koszyka
    DozorRyzyka,
    /// posłuszeństwo komunikatowi z kanału
    KomendaKanalu,
    /// cofka do celu / zachowanie ceny wokół wejścia
    Cofka,
    /// zasięg ruchu od zawiązania koszyka
    Zasieg,
    /// wiek koszyka / czas w rynku
    Wiek,
    /// stan rachunku (margines, floating, dzień)
    Rachunek,
    /// slot autora wariantu (0..15)
    Taktyka(u8),
}

impl PowodAkcji {
    #[inline]
    pub fn idx(self) -> usize {
        match self {
            PowodAkcji::Brak => 0,
            PowodAkcji::Szkielet => 1,
            PowodAkcji::DozorRyzyka => 2,
            PowodAkcji::KomendaKanalu => 3,
            PowodAkcji::Cofka => 4,
            PowodAkcji::Zasieg => 5,
            PowodAkcji::Wiek => 6,
            PowodAkcji::Rachunek => 7,
            PowodAkcji::Taktyka(n) => 8 + (n as usize).min(POWODY_TAKTYK - 1),
        }
    }

    #[inline]
    pub fn kod(self) -> &'static str {
        match self {
            PowodAkcji::Brak => "BRAK",
            PowodAkcji::Szkielet => "SZKIELET",
            PowodAkcji::DozorRyzyka => "DOZOR_RYZYKA",
            PowodAkcji::KomendaKanalu => "KOMENDA_KANALU",
            PowodAkcji::Cofka => "COFKA",
            PowodAkcji::Zasieg => "ZASIEG",
            PowodAkcji::Wiek => "WIEK",
            PowodAkcji::Rachunek => "RACHUNEK",
            PowodAkcji::Taktyka(n) => NAZWY_TAKTYK[(n as usize).min(POWODY_TAKTYK - 1)],
        }
    }

    /// Powód po indeksie — do wypisania raportu z gołej tablicy liczników.
    pub fn z_idx(i: usize) -> PowodAkcji {
        match i {
            0 => PowodAkcji::Brak,
            1 => PowodAkcji::Szkielet,
            2 => PowodAkcji::DozorRyzyka,
            3 => PowodAkcji::KomendaKanalu,
            4 => PowodAkcji::Cofka,
            5 => PowodAkcji::Zasieg,
            6 => PowodAkcji::Wiek,
            7 => PowodAkcji::Rachunek,
            _ => PowodAkcji::Taktyka((i.saturating_sub(8)).min(POWODY_TAKTYK - 1) as u8),
        }
    }
}

/// KTÓRYCH POZYCJI dotyczy akcja.
///
/// Zamknięty zbiór, a nie dowolny predykat — bo wybór biletów jest tym
/// miejscem, w którym najłatwiej zbudować dwie różne definicje „najgłębszej
/// pozycji" i potem porównywać jabłka z gruszkami.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WyborBiletow {
    /// wszystkie niezamrożone pozycje koszyka
    #[default]
    Wszystkie,
    /// pozycja o NAJGORSZYM bieżącym wyniku (najgłębsze wejście)
    Najglebszy,
    /// pozycja o NAJLEPSZYM bieżącym wyniku
    Najplytszy,
    /// najstarsza pozycja koszyka
    Najstarszy,
    /// najmłodsza pozycja koszyka
    Najmlodszy,
    /// pozycje oznaczone jako runner
    Runner,
    /// konkretny bilet
    Jeden(Ticket),
    /// N pozycji o NAJGORSZYM biezacym wyniku — uogolnienie [`WyborBiletow::Najglebszy`].
    ///
    /// Istnieje dla redukcji CZESCIOWEJ (np. przed luka weekendowa): zejscie
    /// do polowy ekspozycji wymaga zamkniecia kilku nog naraz, a nie jednej.
    /// Zamykamy najgorsze, bo maja najmniejszy zapas do stopa — luka uderza
    /// we wszystkie nogi tak samo co do ceny, wiec pierwsza wypada ta,
    /// ktora byla najblizej stopa.
    ///
    /// Liczba, nie lista, bo [`Rozstrzygniecie`] jest `Copy` — a to jest
    /// wlasnosc warta utrzymania: decyzja bez alokacji nie ma jak zawiesc
    /// w polowie.
    NajgorszeN(u8),
}

/// ROZSTRZYGNIĘCIE TAKTYKI — akcja + powód + wybór biletów.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rozstrzygniecie {
    pub akcja: AkcjaEa,
    pub powod: PowodAkcji,
    pub bilety: WyborBiletow,
}

impl Rozstrzygniecie {
    /// Nic nie robimy — i to jest pełnoprawna decyzja.
    #[inline]
    pub const fn trzymaj(powod: PowodAkcji) -> Self {
        Rozstrzygniecie {
            akcja: AkcjaEa::Trzymaj,
            powod,
            bilety: WyborBiletow::Wszystkie,
        }
    }

    #[inline]
    pub const fn nowe(akcja: AkcjaEa, powod: PowodAkcji) -> Self {
        Rozstrzygniecie {
            akcja,
            powod,
            bilety: WyborBiletow::Wszystkie,
        }
    }

    /// Zawęża akcję do wybranych biletów.
    #[inline]
    pub fn na(self, bilety: WyborBiletow) -> Self {
        Rozstrzygniecie { bilety, ..self }
    }
}

/// DECYZJA GOTOWA DO WYKONANIA: rozstrzygnięcie z rozwiązanymi już biletami.
///
/// Bilety są rozwiązywane w pętli, a nie w egzekutorze, bo egzekutor pracuje
/// na `&mut B` i nie ma już wtedy dostępu do koszyków — a plan musi dać się
/// obejrzeć i policzyć ZANIM cokolwiek pójdzie do brokera.
#[derive(Debug, Clone, PartialEq)]
pub struct Decyzja {
    pub ts: Ts,
    pub koszyk: u32,
    pub akcja: AkcjaEa,
    pub powod: PowodAkcji,
    /// bilety POZYCJI, których akcja dotyczy
    pub bilety: Vec<Ticket>,
    /// bilety ZLECEŃ OCZEKUJĄCYCH (dla `AnulujOczekujace`)
    pub zlecenia: Vec<Ticket>,
}

/// CO SIĘ Z AKCJĄ STAŁO. Suma po tych wariantach musi się równać liczbie
/// decyzji — inaczej coś zniknęło bez śladu (N18).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WynikAkcji {
    /// broker przyjął wszystko, co poszło
    Wykonana,
    /// część biletów przeszła, część nie
    CzesciowoWykonana,
    /// nie było czego robić (`Trzymaj`, pusty wybór, ułamek poniżej kroku lota)
    BezPracy,
    /// NASZ walidator nie wypuścił poziomu (stops_level, zła strona rynku,
    /// luzowanie stopa) — do brokera nie poszło nic
    OdmowaPoziomu,
    /// broker odmówił
    OdmowaBrokera,
    /// tryb obserwacyjny — decyzja policzona, nic nie wysłane
    Obserwacja,
}

impl WynikAkcji {
    #[inline]
    pub fn kod(self) -> &'static str {
        match self {
            WynikAkcji::Wykonana => "WYKONANA",
            WynikAkcji::CzesciowoWykonana => "CZESCIOWO",
            WynikAkcji::BezPracy => "BEZ_PRACY",
            WynikAkcji::OdmowaPoziomu => "ODMOWA_POZIOMU",
            WynikAkcji::OdmowaBrokera => "ODMOWA_BROKERA",
            WynikAkcji::Obserwacja => "OBSERWACJA",
        }
    }
}

/// KOD POMINIĘCIA KOSZYKA przez pętlę decyzyjną.
///
/// Osobny enum od [`KodPominiecia`], a nie dopisane warianty — bo tamten jest
/// czytany przez `runner.rs` i przez bramki innych torów, a jego tablica ma
/// dokładnie dziewięć pól. Beta ma zero prawa ruszać cudzą miarę.
/// Odmowy BROKERA idą do OBU liczników: tu i do
/// [`KodPominiecia::OdmowaBrokera`] w [`BilansPulsu`], bo tak każe kontrakt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KodPominieciaBety {
    /// rdzeń nie odtworzył jeszcze stanu (N19)
    NiegotowyRdzen,
    /// koszyk z innego kanału niż `format` konfiguracji
    ObcyFormat,
    /// broker nie podał sensownego kwotowania
    BrakKwotowania,
    /// wszystkie pozycje koszyka zamrożone ręczną edycją
    Zamrozony,
    /// koszyk wyczerpał sufit interwencji (`max_akcji_koszyk`)
    SufitAkcji,
    /// za wcześnie po poprzedniej interwencji (`min_odstep_akcji_s`)
    OdstepAkcji,
    /// wariant nie ma jeszcze taktyki
    WariantBezTaktyki,
    /// walidator poziomów odmówił
    PoziomOdrzucony,
    /// broker odmówił
    OdmowaBrokera,
    /// decyzja zapadła, ale nie było czego zrobić
    BezPracy,
}

pub const LICZBA_KODOW_BETY: usize = 10;

impl KodPominieciaBety {
    pub const WSZYSTKIE: [KodPominieciaBety; LICZBA_KODOW_BETY] = [
        KodPominieciaBety::NiegotowyRdzen,
        KodPominieciaBety::ObcyFormat,
        KodPominieciaBety::BrakKwotowania,
        KodPominieciaBety::Zamrozony,
        KodPominieciaBety::SufitAkcji,
        KodPominieciaBety::OdstepAkcji,
        KodPominieciaBety::WariantBezTaktyki,
        KodPominieciaBety::PoziomOdrzucony,
        KodPominieciaBety::OdmowaBrokera,
        KodPominieciaBety::BezPracy,
    ];

    #[inline]
    pub fn idx(self) -> usize {
        match self {
            KodPominieciaBety::NiegotowyRdzen => 0,
            KodPominieciaBety::ObcyFormat => 1,
            KodPominieciaBety::BrakKwotowania => 2,
            KodPominieciaBety::Zamrozony => 3,
            KodPominieciaBety::SufitAkcji => 4,
            KodPominieciaBety::OdstepAkcji => 5,
            KodPominieciaBety::WariantBezTaktyki => 6,
            KodPominieciaBety::PoziomOdrzucony => 7,
            KodPominieciaBety::OdmowaBrokera => 8,
            KodPominieciaBety::BezPracy => 9,
        }
    }

    #[inline]
    pub fn kod(self) -> &'static str {
        match self {
            KodPominieciaBety::NiegotowyRdzen => "NiegotowyRdzen",
            KodPominieciaBety::ObcyFormat => "ObcyFormat",
            KodPominieciaBety::BrakKwotowania => "BrakKwotowania",
            KodPominieciaBety::Zamrozony => "Zamrozony",
            KodPominieciaBety::SufitAkcji => "SufitAkcji",
            KodPominieciaBety::OdstepAkcji => "OdstepAkcji",
            KodPominieciaBety::WariantBezTaktyki => "WariantBezTaktyki",
            KodPominieciaBety::PoziomOdrzucony => "PoziomOdrzucony",
            KodPominieciaBety::OdmowaBrokera => "OdmowaBrokera",
            KodPominieciaBety::BezPracy => "BezPracy",
        }
    }
}

/// Licznik JEDNEGO rodzaju akcji. Domyka się tak samo jak bilans pulsu:
/// `zapadla = wykonana + czesciowa + bez_pracy + odmowa_poziomu +
/// odmowa_brokera + obserwacja`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LicznikAkcji {
    pub zapadla: u64,
    pub wykonana: u64,
    pub czesciowa: u64,
    pub bez_pracy: u64,
    pub odmowa_poziomu: u64,
    pub odmowa_brokera: u64,
    pub obserwacja: u64,
}

impl LicznikAkcji {
    #[inline]
    fn dopisz(&mut self, w: WynikAkcji) {
        match w {
            WynikAkcji::Wykonana => self.wykonana += 1,
            WynikAkcji::CzesciowoWykonana => self.czesciowa += 1,
            WynikAkcji::BezPracy => self.bez_pracy += 1,
            WynikAkcji::OdmowaPoziomu => self.odmowa_poziomu += 1,
            WynikAkcji::OdmowaBrokera => self.odmowa_brokera += 1,
            WynikAkcji::Obserwacja => self.obserwacja += 1,
        }
    }

    #[inline]
    pub fn rozliczone(&self) -> u64 {
        self.wykonana
            + self.czesciowa
            + self.bez_pracy
            + self.odmowa_poziomu
            + self.odmowa_brokera
            + self.obserwacja
    }

    #[inline]
    pub fn domyka_sie(&self) -> bool {
        self.zapadla == self.rozliczone()
    }
}

/// KSIĘGOWOŚĆ PĘTLI DECYZYJNEJ — ta sama zasada domu co [`BilansPulsu`]:
/// `rozpatrzone = obsłużone + pominięte_z_kodem`, a każde pominięcie ma KOD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BilansDecyzji {
    /// ile koszyków pętla w ogóle wzięła pod uwagę
    pub rozpatrzone: u64,
    /// ile z nich dostało decyzję (także `Trzymaj`)
    pub obsluzone: u64,
    /// ile pominięto, każdy z kodem
    pub pominiete: u64,
    /// rozbicie pominięć
    pub kody: [u64; LICZBA_KODOW_BETY],
    /// liczniki po rodzajach akcji
    pub akcje: [LicznikAkcji; LICZBA_AKCJI],
    /// ile razy padł który powód
    pub powody: [u64; LICZBA_POWODOW],
    /// ile POJEDYNCZYCH zleceń poszło do brokera i ile z nich odbiło
    pub zlecen_wyslanych: u64,
    pub zlecen_odrzuconych: u64,
}

impl BilansDecyzji {
    #[inline]
    fn obsluz(&mut self) {
        self.rozpatrzone += 1;
        self.obsluzone += 1;
    }

    #[inline]
    fn pomin(&mut self, k: KodPominieciaBety) {
        self.rozpatrzone += 1;
        self.pominiete += 1;
        self.kody[k.idx()] += 1;
    }

    /// Sam kod, bez ruszania bilansu — dla zdarzeń dotyczących bytu już
    /// policzonego (odmowa poziomu przy wykonywaniu decyzji).
    #[inline]
    fn odnotuj(&mut self, k: KodPominieciaBety) {
        self.kody[k.idx()] += 1;
    }

    #[inline]
    pub fn domyka_sie(&self) -> bool {
        self.rozpatrzone == self.obsluzone + self.pominiete
            && self.akcje.iter().all(|a| a.domyka_sie())
    }

    #[inline]
    pub fn ile(&self, k: KodPominieciaBety) -> u64 {
        self.kody[k.idx()]
    }

    #[inline]
    pub fn akcja(&self, k: KodAkcji) -> LicznikAkcji {
        self.akcje[k.idx()]
    }

    #[inline]
    pub fn powod(&self, p: PowodAkcji) -> u64 {
        self.powody[p.idx()]
    }

    /// Ile decyzji faktycznie RUSZYŁO rachunek.
    #[inline]
    pub fn interwencji(&self) -> u64 {
        self.akcje
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != KodAkcji::Trzymaj.idx())
            .map(|(_, a)| a.wykonana + a.czesciowa)
            .sum()
    }
}

/// Jeden wpis DZIENNIKA DECYZJI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WpisDecyzji {
    pub ts: Ts,
    pub koszyk: u32,
    pub akcja: KodAkcji,
    pub powod: PowodAkcji,
    pub wynik: WynikAkcji,
    /// liczba opisująca akcję (ułamek, poziom, wolumen) — do zrozumienia wpisu
    /// bez zaglądania w kod
    pub szczegol: f64,
    /// ilu biletów dotyczyła
    pub biletow: u32,
}

// ------------------------------------------------------- widok decyzyjny

/// MIGAWKA POZYCJI na potrzeby decyzji. Kopia, nie referencja — bo egzekutor
/// potrzebuje `&mut B`, a widok musi przeżyć koniec pożyczki `b.positions()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MigawkaPozycji {
    pub ticket: Ticket,
    pub koszyk: u32,
    pub side: Side,
    pub volume: f64,
    pub open_price: Px,
    pub open_ts: Ts,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    pub level: i32,
    pub frozen: bool,
    pub is_runner: bool,
    pub is_toucher: bool,
    /// wynik otwarty w dolarach, liczony po cenie WYJŚCIA (czyli po spreadzie)
    pub wynik_usd: f64,
    /// wynik otwarty w punktach ceny
    pub wynik_pts: f64,
    /// szczyt zysku tej pozycji w punktach (`Position::peak_pts`)
    pub szczyt_pts: f64,
    /// wiek pozycji w minutach
    pub wiek_min: f64,
}

/// PAMIĘĆ EA O KOSZYKU — to, czego nie ma ani w koszyku, ani u brokera.
///
/// To jest ta część, która odróżnia EA od zestawu reguł: program pamięta,
/// co widział, i decyduje na podstawie własnej historii obserwacji, a nie
/// wyłącznie na podstawie migawki „teraz".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PamiecKoszyka {
    pub id: u32,
    /// kiedy pętla decyzyjna zobaczyła ten koszyk PIERWSZY RAZ
    ///
    /// ⚠ To nie to samo co `Basket::created_ts`: po restarcie (i po wymianie
    /// silnika w backteście dobowym) koszyk jest przejęty, a nie zawiązany.
    /// Nazwa mówi dokładnie to, co pole znaczy, żeby nikt nie policzył z niego
    /// „wieku setupu".
    pub pierwszy_ts: Ts,
    /// cena w chwili pierwszego zobaczenia
    pub cena_pierwsza: Px,
    /// najwyższa i najniższa cena widziana od tamtej chwili
    pub cena_max: Px,
    pub cena_min: Px,
    /// szczyt i DNO łącznego wyniku koszyka ($) widziane przez pętlę
    pub pl_szczyt: f64,
    pub pl_dno: f64,
    /// ile interwencji EA wykonał na tym koszyku i kiedy ostatnią
    pub akcje: u32,
    pub ostatnia_akcja_ts: Ts,
    /// ostatni stop, który EA sam ustawił na tym koszyku
    pub ostatni_stop: Option<Px>,
    /// ile razy EA inkasował część wolumenu
    pub inkasa: u32,
}

impl PamiecKoszyka {
    fn nowa(id: u32, ts: Ts, cena: Px, pl: f64) -> Self {
        PamiecKoszyka {
            id,
            pierwszy_ts: ts,
            cena_pierwsza: cena,
            cena_max: cena,
            cena_min: cena,
            pl_szczyt: pl,
            pl_dno: pl,
            akcje: 0,
            ostatnia_akcja_ts: 0,
            ostatni_stop: None,
            inkasa: 0,
        }
    }

    fn dopisz(&mut self, cena: Px, pl: f64) {
        if cena > self.cena_max {
            self.cena_max = cena;
        }
        if cena < self.cena_min {
            self.cena_min = cena;
        }
        if pl > self.pl_szczyt {
            self.pl_szczyt = pl;
        }
        if pl < self.pl_dno {
            self.pl_dno = pl;
        }
    }
}

/// SYTUACJA KOSZYKA — wszystko, czego potrzeba do decyzji, w jednej strukturze.
///
/// Cztery warstwy, w tej kolejności: **rama sygnału** (co obiecali traderzy),
/// **stan pozycji** (co mamy w rynku), **stan koszyka** (co już się w nim
/// zdarzyło), **kontekst rynku i rachunku** (gdzie stoimy). Taktyka nie ma
/// prawa sięgać poza tę strukturę — bo drugie źródło tej samej liczby to
/// drugie miejsce, w którym da się je rozjechać.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SytuacjaKoszyka<'a> {
    // ---------------------------------------------------------- rama sygnału
    pub koszyk: u32,
    pub format: &'a str,
    pub side: Side,
    pub is_limit: bool,
    pub is_stop: bool,
    /// strefa wejścia PO offsetach konfiguracji
    pub zone_lo: Px,
    pub zone_hi: Px,
    /// strefa surowa, prosto z sygnału
    pub entry_lo: Px,
    pub entry_hi: Px,
    pub sl: Option<Px>,
    pub tps: &'a [Px],
    pub tp_open: bool,
    /// wiek koszyka od ZAWIĄZANIA w minutach
    pub wiek_min: f64,

    // ----------------------------------------------------------- stan koszyka
    pub etap: EtapKoszyka,
    pub stan: BasketState,
    /// ile celów zainkasowaliśmy MAJĄC pozycję
    pub tp_stage: usize,
    /// ile celów rynek wykonał BEZ nas
    pub plan_wykonany_do: usize,
    pub secured: bool,
    pub secured_by_rule: bool,
    /// czy kanał kazał postawić SL na breakeven (`Basket::be_ts > 0`)
    pub bylo_be: bool,
    pub be_ts: Ts,
    pub zone_touched: bool,
    pub reentries: u32,
    pub rearms: u32,
    pub fast_addons: u32,
    pub pyramided: bool,
    /// szczeble siatki: żywe (do wypełnienia) i wypełnione
    pub szczeble_zywe: u32,
    pub szczeble_wypelnione: u32,
    /// bilety oczekujących zleceń koszyka
    pub zlecenia: &'a [Ticket],

    // ---------------------------------------------------------- stan pozycji
    pub pozycje: &'a [MigawkaPozycji],
    pub wolumen: f64,
    /// średnia cena wejścia WAŻONA WOLUMENEM (cena wyjścia na zero)
    pub srednia_cena: Px,
    /// wynik OTWARTY ($), po cenie wyjścia
    pub otwarty_usd: f64,
    /// wynik ZREALIZOWANY na tym koszyku ($)
    pub zrealizowany_usd: f64,
    /// suma obu — jedyna liczba, którą warto porównywać z dnem i szczytem
    pub laczny_usd: f64,
    /// ile jeszcze można stracić na otwartych pozycjach ($); `None` = brak SL
    pub ryzyko_usd: Option<f64>,
    pub zamrozonych: usize,

    // -------------------------------------------------------- kontekst rynku
    pub q: Quote,
    pub spread: f64,
    /// cena, po której WCHODZI się na stronę koszyka
    pub cena_wejscia: Px,
    /// cena, po której WYCHODZI się ze strony koszyka
    pub cena_wyjscia: Px,
    /// ile ceny przeszło NA KORZYŚĆ koszyka od pierwszego zobaczenia
    pub zasieg_za: f64,
    /// ile ceny przeszło PRZECIW koszykowi od pierwszego zobaczenia
    pub zasieg_przeciw: f64,
    /// odległość ceny wyjścia od stopa koszyka (dodatnia = stop jeszcze przed nami)
    pub dystans_do_sl: Option<f64>,
    /// najbliższy niezainkasowany cel i odległość do niego
    pub nastepny_cel: Option<Px>,
    pub dystans_do_celu: Option<f64>,

    // -------------------------------------------------------------- pamięć EA
    pub pamiec: PamiecKoszyka,

    // --------------------------------------------------------------- rachunek
    pub w: WektorStanu,
    /// stan warstwy EFEKTYWNY dla tego koszyka (z zapadką)
    pub stan_ea: EaStan,
    pub ts: Ts,
}

impl SytuacjaKoszyka<'_> {
    /// Czy koszyk trzyma TERAZ jakąkolwiek pozycję.
    #[inline]
    pub fn ma_pozycje(&self) -> bool {
        !self.pozycje.is_empty()
    }

    /// Wynik łączny w wielokrotności ryzyka bieżącego (`None` bez SL).
    #[inline]
    pub fn pl_r(&self) -> Option<f64> {
        match self.ryzyko_usd {
            Some(r) if r > 1e-9 => Some(self.laczny_usd / r),
            _ => None,
        }
    }

    /// Najgorszy łączny wynik, jaki ten koszyk pokazał ($).
    #[inline]
    pub fn dno_usd(&self) -> f64 {
        self.pamiec.pl_dno
    }

    /// Ile koszyk ODDAŁ od swojego szczytu ($).
    #[inline]
    pub fn oddane_od_szczytu(&self) -> f64 {
        (self.pamiec.pl_szczyt - self.laczny_usd).max(0.0)
    }

    /// Ile procent drogi od średniego wejścia do najbliższego celu jest za nami.
    pub fn droga_do_celu_pct(&self) -> Option<f64> {
        let cel = self.nastepny_cel?;
        let baza = (cel - self.srednia_cena) * self.side.sign();
        if baza <= 1e-9 {
            return None;
        }
        Some(((self.cena_wyjscia - self.srednia_cena) * self.side.sign() / baza * 100.0).max(0.0))
    }

    /// Wolumen odpowiadający ułamkowi łącznego wolumenu koszyka.
    #[inline]
    pub fn ulamek_wolumenu(&self, u: f64) -> f64 {
        krok_lota(self.wolumen * u.clamp(0.0, 1.0))
    }
}

// ============================================================================
//  RDZEŃ
// ============================================================================

/// EA-CORE: jeden stan, jeden zegar, jedna histereza, jedna zapadka, jeden
/// dziennik.
///
/// Cała struktura jest **zerowa u silnika, który jej nie używa**: `Default`
/// daje `Neutral`, puste wektory i `gotowy = false`. Silnik z `ea_enabled =
/// false` nie woła ani jednej metody, więc koszt to `size_of` w `Engine`
/// i nic więcej.
#[derive(Debug, Clone, Default)]
pub struct EaRdzen {
    /// Engine-scoped restore latch. Never suppresses protective EA actions.
    continuation_entry_hold: bool,
    profit_budget_anchor: crate::profit_budget::BudgetAnchor,
    /// bieżący stan warstwy
    stan: EaStan,
    /// odkąd trwa bieżący stan (0 = od zawsze)
    stan_od_ts: Ts,
    /// kandydat na nowy stan i chwila, w której warunek zaczął obowiązywać —
    /// przejście wymaga przetrzymania `ea_state_dwell_s`
    kandydat: Option<(EaStan, Ts)>,
    /// chwila ostatniego pulsu (dowolnego źródła)
    ostatni_puls_ts: Ts,
    /// czy rdzeń odtworzył stan po restarcie (N19). Dopóki `false`, warstwa
    /// ODMAWIA działania kodem [`KodPominiecia::NiegotowyRdzen`].
    gotowy: bool,
    stemple: Vec<StempelKoszyka>,
    /// narastający bilans (N18)
    bilans: BilansPulsu,
    /// dziennik zmian stanu (sufit [`SUFIT_DZIENNIKA`])
    dziennik: Vec<WpisEa>,
    /// ostatni policzony wektor stanu — do podglądu z panelu i z testów
    ostatni_wektor: WektorStanu,
    /// OCZY MÓZGU — wielookresowy obraz rynku (tiki, 1m … 1M).
    ///
    /// Aktualizowane TYLKO wtedy, gdy mózg jest włączony zmienną `MOZG`.
    /// Przy wyłączonym mózgu nie kosztują ani jednego dodania — a to jest
    /// warunek parytetu, bo agregacja świec na 31 mln tików nie jest darmowa.
    oczy: conduit_mozg::oczy::Oczy,
    /// obserwacje trybu Cień — ile razy i dlaczego mózg CHCIAŁBY coś zrobić
    cien_mozgu: conduit_mozg::cien::Obserwacje,
    /// liczniki pulsów wg źródła — dowód, że warstwa żyje bez ticków
    pub pulsy_tick: u64,
    pub pulsy_zegar: u64,
    /// ile razy dozór SL faktycznie DOSTAWIŁ stop (N15)
    pub dostawione_sl: u64,
    /// ile pozycji bez SL warstwa w ogóle zobaczyła (N15 — znalezisko)
    pub widziane_bez_sl: u64,
    /// **MIARA AKCEPTACJI N10:** ile RÓŻNYCH koszyków podniosło ryzyko po
    /// zawiązaniu. Każdy koszyk liczy się raz, niezależnie od kadencji pulsu.
    /// Cel niezmiennika: **0**.
    pub koszyki_zapadka: u64,
    /// ile RÓŻNYCH koszyków cofnęło etap (regresja od pierwszego zatrzasku
    /// w górę). Cel: **0**.
    pub koszyki_regresja: u64,
    /// Największe przekroczenie stempla w całym przebiegu ($) i jego udział
    /// w stemplu. Bez tej pary „ryzyko wzrosło" nie da się odróżnić od
    /// „ryzyko wzrosło o grosz na zaokrągleniu".
    pub zapadka_max_nadwyzka_usd: f64,
    pub zapadka_max_nadwyzka_pct: f64,

    // ---------------------------- BETA EA (pętla decyzyjna) -----------------
    //
    // KONTRAKT ZERA: dopóki `beta` jest `Brak`, żadne z tych pól nie jest
    // ruszane i pętla decyzyjna wychodzi PRZED odczytem czegokolwiek.
    //
    /// leniwy uchwyt konfiguracji ze zmiennej `CONDUIT_EA_BETA`
    beta: UchwytBety,
    /// chwila ostatniego przebiegu pętli decyzyjnej (własny zegar bety)
    beta_ostatni_ts: Ts,
    /// ile razy pętla decyzyjna faktycznie przebiegła
    pub beta_pulsy: u64,
    /// PAMIĘĆ EA o koszykach — to, czego nie ma ani w koszyku, ani u brokera
    pamiec: Vec<PamiecKoszyka>,
    /// księgowość pętli decyzyjnej (`rozpatrzone = obsłużone + pominięte`)
    bilans_decyzji: BilansDecyzji,
    /// dziennik podjętych akcji (sufit z konfiguracji)
    dziennik_decyzji: Vec<WpisDecyzji>,
    /// bufory wielokrotnego użytku — puls nie ma prawa alokować w pętli
    bufor_poz: Vec<MigawkaPozycji>,
    bufor_plan: Vec<Decyzja>,
}

impl Default for EaStan {
    fn default() -> Self {
        EaStan::Neutral
    }
}

impl EaRdzen {
    pub(crate) fn set_profit_budget_anchor(&mut self, anchor:crate::profit_budget::BudgetAnchor) {
        self.profit_budget_anchor=anchor;
    }
    pub(crate) fn profit_budget_peak(&self)->f64 {self.profit_budget_anchor.peak}
    fn profit_budget_volume<B:Broker>(&mut self,cfg:&Settings,b:&B,side:Side,entry:Px,
        sl:Option<Px>,volume:f64,r:&mut Rachuba)->Option<f64> {
        if cfg.profit_budget_arm_pct!=0.0 {
            let equity=b.account().equity;
            if equity.is_finite(){self.profit_budget_anchor.peak=self.profit_budget_anchor.peak.max(equity);}
        }
        match crate::profit_budget::limit_open_volume(cfg,self.profit_budget_anchor,b,side,entry,sl,volume) {
            Ok(v)=>Some(v), Err(reason)=>{
                eprintln!("[EA-BETA][ProfitBudget::{reason:?}] new order withheld");
                r.odmowa_brokera(&mut self.bilans,&mut self.bilans_decyzji);None
            }
        }
    }

    pub(crate) fn set_continuation_entry_hold(&mut self, hold: bool) {
        self.continuation_entry_hold=hold;
    }
    // ---------------------------------------------------------------- odczyt

    #[inline]
    pub fn stan(&self) -> EaStan {
        self.stan
    }
    #[inline]
    pub fn stan_od(&self) -> Ts {
        self.stan_od_ts
    }
    #[inline]
    pub fn gotowy(&self) -> bool {
        self.gotowy
    }
    #[inline]
    pub fn bilans(&self) -> BilansPulsu {
        self.bilans
    }
    #[inline]
    pub fn dziennik(&self) -> &[WpisEa] {
        &self.dziennik
    }
    #[inline]
    pub fn stemple(&self) -> &[StempelKoszyka] {
        &self.stemple
    }
    #[inline]
    pub fn wektor(&self) -> WektorStanu {
        self.ostatni_wektor
    }
    #[inline]
    pub fn pulsy(&self) -> u64 {
        self.pulsy_tick + self.pulsy_zegar
    }
    /// Stempel konkretnego koszyka (do egzekucji zapadki przez osie FALI 1+).
    #[inline]
    pub fn stempel(&self, id: u32) -> Option<&StempelKoszyka> {
        self.stemple.iter().find(|s| s.id == id)
    }

    /// **N19 — brama wejściowa.** Dopóki rdzeń nie odtworzył stanu, wolno mu
    /// wyłącznie odtwarzać: nie przydziela, nie modyfikuje, nie decyduje.
    #[inline]
    pub fn wolno_dzialac(&self) -> Result<(), KodPominiecia> {
        if self.gotowy {
            Ok(())
        } else {
            Err(KodPominiecia::NiegotowyRdzen)
        }
    }

    #[inline]
    pub fn modulatory(&self, _cfg: &Settings, _koszyk: u32) -> Modulatory {
        // ZAPADKA (`NieLuzujWKoszyku`) jest już tutaj gotowa do użycia:
        // stan efektywny koszyka to NAJOSTROŻNIEJSZY ze stanu bieżącego
        // i stanu ze stempla. Powrót do Neutral nie rozszerza trailingu
        // i nie odblokowuje dokładek w koszyku zawiązanym w Obronie.
        // Póki rodziny B–G nie mają pól, wynik i tak jest neutralny.
        Modulatory::neutralne()
    }

    /// Stan EFEKTYWNY dla koszyka, czyli z uwzględnieniem zapadki.
    ///
    /// To jest ta jedna linijka, dla której zapadka w ogóle istnieje.
    /// Wystawiona publicznie i przetestowana ZANIM pojawi się pierwszy
    /// czytelnik — inaczej pierwsza oś rodziny B zbudowałaby własną wersję.
    pub fn stan_efektywny(&self, cfg: &Settings, koszyk: u32) -> EaStan {
        match cfg.ea_state_ratchet {
            EaRatchet::Swobodny => self.stan,
            EaRatchet::NieLuzujWKoszyku => match self.stempel(koszyk) {
                Some(s) if s.stan.ostroznosc() > self.stan.ostroznosc() => s.stan,
                _ => self.stan,
            },
        }
    }

    // ------------------------------------------------------------- odtwarzanie

    /// **N19 — RESTART NIE GUBI OCHRONY.**
    ///
    /// Odtwarza stemple z koszyków przejętych po restarcie i dopiero wtedy
    /// otwiera bramę [`EaRdzen::wolno_dzialac`]. Kolejność źródeł jest ta
    /// z niezmiennika: broker (pozycje i zlecenia = prawda) → koszyki.
    ///
    /// Woła się SAMO, na pierwszym pulsie — backtest nie musi o tym wiedzieć,
    /// a warstwa żywa dostaje to za darmo razem z przejęciem koszyków.
    pub fn odtworz<B: Broker>(&mut self, baskets: &[Basket], b: &B, ts: Ts) {
        self.stemple.clear();
        for bk in baskets {
            let r = ryzyko_planowane(bk, b).unwrap_or(0.0);
            self.stemple.push(StempelKoszyka {
                id: bk.id,
                // stempel odtworzony NIE UDAJE, że powstał w chwili
                // zawiązania: bierze `created_ts` koszyka, bo to jest
                // jedyna prawdziwa chwila zawiązania, jaka przetrwała
                // restart. Wpisanie `ts` restartu zerowałoby zapadkę.
                ts: if bk.created_ts > 0 { bk.created_ts } else { ts },
                ryzyko_stempla: if bk.risk_initial_usd > 0.0 {
                    bk.risk_initial_usd
                } else {
                    r
                },
                stan: self.stan,
                ryzyko_szczyt: r,
                etap: EtapKoszyka::z_koszyka(bk),
                zapadka_zgloszona: false,
            });
        }
        self.gotowy = true;
    }

    #[inline]
    pub fn uniewaznij(&mut self) {
        self.gotowy = false;
    }

    // ------------------------------------------------------------------ zegar

    /// Czy zegar warstwy właśnie wybił.
    ///
    /// `ea_tick_s = 0` znaczy **brak własnego zegara**: warstwa budzi się
    /// wtedy z każdym tickiem, czyli dokładnie tak, jak działają dzisiejsze
    /// reguły. `> 0` włącza kadencję: puls leci, gdy od ostatniego minęło
    /// `ea_tick_s` sekund — **niezależnie od tego, czy obudził nas tick, czy
    /// zegar**. To jest cała treść „straż budzi się z OBU źródeł".
    ///
    /// ⚠ Zegar mierzy się CZASEM ZDARZENIA, nigdy długością bufora. Klucz
    /// cache liczony z `len()` dequeue zamroził `rev_exit` na 12 h — ta
    /// funkcja jest odpowiedzią na tamten błąd i nie wolno jej zmienić na
    /// licznik.
    #[inline]
    pub fn zegar_wybija(&self, cfg: &Settings, ts: Ts) -> bool {
        if cfg.ea_tick_s <= 0.0 {
            return true;
        }
        if self.ostatni_puls_ts == 0 {
            return true;
        }
        let delta = ts - self.ostatni_puls_ts;
        // Zegar cofnięty (reset dobowy backtestu, korekta czasu na maszynie)
        // MUSI dać puls, a nie ciszę do czasu, aż czas dogoni starą wartość.
        // Straż zamilkła na godziny to jest dokładnie ta awaria, dla której
        // ten zegar w ogóle powstał — i nie wolno jej wpuścić tylnymi drzwiami.
        delta < 0 || delta as f64 >= cfg.ea_tick_s * 1000.0
    }

    // ------------------------------------------------------------ maszyna stanu

    /// Sygnał `x` maszyny stanu wg `ea_state_src` (ujemny = strata).
    fn sygnal(&self, cfg: &Settings, w: &WektorStanu, baskets: &[Basket]) -> f64 {
        match cfg.ea_state_src {
            EaStateSrc::FloatPctEquity => w.floating_pct,
            EaStateSrc::FloatR => {
                // floating / R pierwotne — po CAŁYM portfelu, bo Obrona jest
                // stanem portfela. Koszyki bez zapamiętanego R nie wchodzą do
                // mianownika: dzielenie przez „0 zamiast braku" dawałoby
                // nieskończoność i migotanie progu.
                let r: f64 = baskets
                    .iter()
                    .map(|x| x.risk_initial_usd)
                    .filter(|v| *v > 0.0)
                    .sum();
                if r > 0.0 {
                    w.floating / r
                } else {
                    0.0
                }
            }
        }
    }

    /// Docelowy stan wg progów, BEZ histerezy czasowej.
    ///
    /// # Konwencja zera (obowiązkowa, patrz `KONWENCJA_ZERA.md`)
    ///
    /// `ea_defense_enter = 0` znaczy **obrona nigdy**, a nie „obrona przy
    /// zerowej stracie". Tak samo `ea_offense_enter = 0` znaczy **agresja
    /// nigdy**. To jest połowa podwójnego zera: preset z samymi zerami stoi
    /// w `Neutral` na zawsze, cokolwiek robi rynek.
    fn ocen_stan(&self, cfg: &Settings, x: f64) -> EaStan {
        // wyjście ze stanu bieżącego ma PIERWSZEŃSTWO nad wejściem do nowego —
        // inaczej przy progach ustawionych na krzyż stan skakałby co puls
        match self.stan {
            EaStan::Obrona => {
                // wychodzimy dopiero, gdy strata cofnie się do progu WYJŚCIA
                // (mniej dotkliwego niż próg wejścia) — histereza dwustronna
                if cfg.ea_defense_exit > 0.0 && x >= -cfg.ea_defense_exit {
                    EaStan::Neutral
                } else if cfg.ea_defense_enter <= 0.0 {
                    // oś wyłączono w trakcie — nie trzymamy stanu na siłę
                    EaStan::Neutral
                } else {
                    EaStan::Obrona
                }
            }
            EaStan::Agresja => {
                if cfg.ea_offense_exit > 0.0 && x <= cfg.ea_offense_exit {
                    EaStan::Neutral
                } else if cfg.ea_offense_enter <= 0.0 {
                    EaStan::Neutral
                } else {
                    EaStan::Agresja
                }
            }
            EaStan::Neutral => {
                if cfg.ea_defense_enter > 0.0 && x <= -cfg.ea_defense_enter {
                    EaStan::Obrona
                } else if cfg.ea_offense_enter > 0.0 && x >= cfg.ea_offense_enter {
                    EaStan::Agresja
                } else {
                    EaStan::Neutral
                }
            }
        }
    }

    /// Stosuje przejście z minimalnym czasem trwania (`ea_state_dwell_s`).
    ///
    /// **Asymetria, ta sama co w drabince i w arbitrze:** REDUKCJA RYZYKA
    /// DZIAŁA NATYCHMIAST, luz wymaga potwierdzenia. Przejście w stan
    /// bardziej ostrożny (Neutral → Obrona, Agresja → Neutral) omija dwell;
    /// przejście w mniej ostrożny musi przetrzymać warunek.
    fn przejdz(&mut self, cfg: &Settings, cel: EaStan, x: f64, ts: Ts, zr: ZrodloPulsu) {
        if cel == self.stan {
            self.kandydat = None;
            return;
        }
        let ostrozniej = cel.ostroznosc() > self.stan.ostroznosc();
        let dwell_ms = (cfg.ea_state_dwell_s.max(0.0) * 1000.0) as i64;
        if !ostrozniej && dwell_ms > 0 {
            match self.kandydat {
                Some((k, od)) if k == cel => {
                    if ts - od < dwell_ms {
                        return; // jeszcze nie dojrzało
                    }
                }
                _ => {
                    self.kandydat = Some((cel, ts));
                    return;
                }
            }
        }
        let powod = if ostrozniej {
            "zaciskanie"
        } else {
            "luzowanie"
        };
        let wpis = WpisEa {
            ts,
            z: self.stan,
            na: cel,
            x,
            zrodlo: zr,
            powod,
        };
        if cfg.ea_state_journal {
            if self.dziennik.len() >= SUFIT_DZIENNIKA {
                self.dziennik.remove(0);
            }
            self.dziennik.push(wpis);
        }
        self.stan = cel;
        self.stan_od_ts = ts;
        self.kandydat = None;
    }

    // -------------------------------------------------------------------- puls

    pub fn puls<B: Broker>(
        &mut self,
        cfg: &Settings,
        baskets: &mut [Basket],
        b: &mut B,
        ts: Ts,
        zr: ZrodloPulsu,
    ) -> bool {
        if !self.zegar_wybija(cfg, ts) {
            return false;
        }
        self.ostatni_puls_ts = ts;
        match zr {
            ZrodloPulsu::Tick => self.pulsy_tick += 1,
            ZrodloPulsu::Zegar => self.pulsy_zegar += 1,
        }

        // (1) N19 — dopóki rdzeń nie odtworzył stanu, jedyne, co mu wolno,
        //     to odtworzyć stan. Pierwszy puls po starcie robi to sam.
        if !self.gotowy {
            self.bilans.pomin(KodPominiecia::NiegotowyRdzen);
            self.odtworz(baskets, b, ts);
        }

        // (2) N15 — dozór SL, przed czymkolwiek innym
        self.dozor_sl(cfg, baskets, b);

        // (3) wektor stanu — JEDEN odczyt rachunku na puls
        let w = WektorStanu::zbierz(b, cfg, baskets.len(), ts);
        self.ostatni_wektor = w;

        // (4) maszyna stanu
        let x = self.sygnal(cfg, &w, baskets);
        let cel = self.ocen_stan(cfg, x);
        self.przejdz(cfg, cel, x, ts, zr);

        // (5) stemple, zapadka i monotoniczność etapu
        self.sprawdz_zapadke(baskets, b, ts);

        // (6) PĘTLA DECYZYJNA BETY — ten moment, w którym program pyta „czy to
        //     nadal ma sens" (patrz [`EaRdzen::petla_decyzyjna`]).
        //
        // ⚠ KONTRAKT ZERA BETY: bez zmiennej `CONDUIT_EA_BETA` funkcja wychodzi
        // na PIERWSZEJ linijce — przed kwotowaniem, przed pozycjami, przed
        // jakąkolwiek decyzją. Kroki (1)–(5) zostają dokładnie takie, jakie
        // były, bo pętla stoi ZA nimi i niczego im nie odbiera.
        self.petla_decyzyjna(cfg, baskets, b, ts);
        true
    }

    // ------------------------------------------------------------- N15 dozór SL

    fn dozor_sl<B: Broker>(&mut self, cfg: &Settings, baskets: &[Basket], b: &mut B) {
        // Zebranie najpierw, modyfikacja potem: `positions()` pożycza brokera
        // niemutowalnie, a `modify_position` mutowalnie.
        let mut braki: Vec<(Ticket, Option<Px>, Option<Px>)> = Vec::new();
        for p in b.positions() {
            if p.sl.is_some() {
                self.bilans.obsluz();
                continue;
            }
            self.widziane_bez_sl += 1;
            if p.frozen {
                self.bilans.pomin(KodPominiecia::Zamrozona);
                continue;
            }
            let Some(bid) = p.basket else {
                self.bilans.pomin(KodPominiecia::Sierota);
                continue;
            };
            let Some(bk) = baskets.iter().find(|x| x.id == bid) else {
                self.bilans.pomin(KodPominiecia::KoszykZnikl);
                continue;
            };
            if cfg.confirmed_exit_retry && bk.pending_exit.is_some() {
                continue; // the committed exit owns this basket until broker-flat
            }
            let Some(sl) = bk.sl else {
                self.bilans.pomin(KodPominiecia::BrakZrodlaSl);
                continue;
            };
            if !cfg.ea_dozor_sl {
                self.bilans.pomin(KodPominiecia::DozorWylaczony);
                continue;
            }
            braki.push((p.ticket, Some(sl), p.tp));
        }
        for (t, sl, tp) in braki {
            match b.modify_position(t, sl, tp) {
                Ok(()) => {
                    self.dostawione_sl += 1;
                    self.bilans.obsluz();
                }
                Err(_) => self.bilans.pomin(KodPominiecia::OdmowaBrokera),
            }
        }
    }

    // -------------------------------------------------- N10 zapadka + etapy

    /// **N10 — ZAPADKA STEMPLA** oraz monotoniczność maszyny stanu koszyka.
    ///
    /// Dla każdego żywego koszyka: stempluje go przy pierwszym zobaczeniu,
    /// a potem pilnuje dwóch rzeczy naraz:
    ///
    ///  * `ryzyko_planowane(b, t) <= ryzyko_ze_stempla(b)` — wzrost znaczy,
    ///    że ktoś podniósł ryzyko po zawiązaniu koszyka (kanoniczny sprawca:
    ///    `pending_relot_up`, dziś włączony w STORM-1 i TYLER-1),
    ///  * etap koszyka nie cofa się — regresja znaczy, że jakaś ścieżka
    ///    skasowała historię koszyka.
    ///
    /// Oba są dziś WYKRYWANE i księgowane; egzekucja (odmowa relotu w górę)
    /// należy do fali, która dotknie ścieżki relotu, i musi mieć własny
    /// kontrakt zera.
    fn sprawdz_zapadke<B: Broker>(&mut self, baskets: &[Basket], b: &B, ts: Ts) {
        self.stemple
            .retain(|s| baskets.iter().any(|bk| bk.id == s.id));

        // JEDEN przelot po pozycjach, kubełkowany po koszyku. Wersja
        // „pozycje w pętli po koszykach" liczyła to samo, ale kwadratowo —
        // a przy `ea_tick_s = 0` puls leci na każdym z 24 mln tików.
        let mut ryz_poz: Vec<(u32, f64)> = Vec::with_capacity(baskets.len());
        for p in b.positions() {
            let Some(bid) = p.basket else { continue };
            let Some(bk) = baskets.iter().find(|x| x.id == bid) else {
                continue;
            };
            let Some(sl_bk) = bk.sl else { continue };
            let r = ryzyko_pozycji(p.open_price, p.sl.unwrap_or(sl_bk), p.volume);
            match ryz_poz.iter_mut().find(|(id, _)| *id == bid) {
                Some(w) => w.1 += r,
                None => ryz_poz.push((bid, r)),
            }
        }

        for bk in baskets {
            let etap = EtapKoszyka::z_koszyka(bk);
            let r = bk.sl.map(|sl| {
                ryzyko_planu(bk, sl)
                    + ryz_poz
                        .iter()
                        .find(|(id, _)| *id == bk.id)
                        .map_or(0.0, |(_, v)| *v)
            });
            match self.stemple.iter_mut().find(|s| s.id == bk.id) {
                Some(s) => {
                    // regresja liczy się WYŁĄCZNIE od pierwszego zatrzasku
                    // w górę — niżej cofnięcie jest normalnym życiem
                    // (skasowana siatka), a nie awarią
                    if s.etap >= EtapKoszyka::PIERWSZY_ZATRZASK && etap < s.etap {
                        self.bilans.odnotuj(KodPominiecia::RegresjaEtapu);
                        self.koszyki_regresja += 1;
                    }
                    s.etap = s.etap.max(etap);
                    if let Some(r) = r {
                        if r > s.ryzyko_szczyt {
                            s.ryzyko_szczyt = r;
                        }
                        let tol = (s.ryzyko_stempla.abs() * 0.01).max(0.01);
                        if r > s.ryzyko_stempla + tol {
                            self.bilans.odnotuj(KodPominiecia::ZapadkaZlamana);
                            // KOSZYK liczy się RAZ (miara akceptacji N10),
                            // zdarzenie liczy się za każdym pulsem (diagnostyka).
                            if !s.zapadka_zgloszona {
                                s.zapadka_zgloszona = true;
                                self.koszyki_zapadka += 1;
                            }
                            let nad = r - s.ryzyko_stempla;
                            if nad > self.zapadka_max_nadwyzka_usd {
                                self.zapadka_max_nadwyzka_usd = nad;
                                self.zapadka_max_nadwyzka_pct = if s.ryzyko_stempla > 1e-9 {
                                    nad / s.ryzyko_stempla * 100.0
                                } else {
                                    f64::INFINITY
                                };
                            }
                        }
                    }
                    self.bilans.obsluz();
                }
                None => {
                    self.stemple.push(StempelKoszyka {
                        id: bk.id,
                        ts: if bk.created_ts > 0 { bk.created_ts } else { ts },
                        ryzyko_stempla: if bk.risk_initial_usd > 0.0 {
                            bk.risk_initial_usd
                        } else {
                            r.unwrap_or(0.0)
                        },
                        stan: self.stan,
                        ryzyko_szczyt: r.unwrap_or(0.0),
                        etap,
                        zapadka_zgloszona: false,
                    });
                    self.bilans.obsluz();
                }
            }
        }
    }
    // ======================================================================
    //  BETA EA — PĘTLA DECYZYJNA
    // ======================================================================

    /// Uchwyt konfiguracji bety — czytany z otoczenia **raz na proces**
    /// i zapamiętywany w rdzeniu.
    ///
    /// To jest ZAWÓR KONTRAKTU ZERA: dopóki zwraca `None`, pętla decyzyjna
    /// wychodzi PRZED odczytem kwotowania, rachunku i pozycji. Jeden odczyt
    /// `std::env::var` na proces to jedyny koszt bety wyłączonej.
    #[inline]
    fn uchwyt_bety(&mut self) -> Option<&'static KonfigBety> {
        match self.beta {
            UchwytBety::Jest(k) => Some(k),
            UchwytBety::Brak => None,
            UchwytBety::Nieodczytany => {
                self.beta = match konfig_bety_z_env() {
                    Some(k) => UchwytBety::Jest(k),
                    None => UchwytBety::Brak,
                };
                match self.beta {
                    UchwytBety::Jest(k) => Some(k),
                    _ => None,
                }
            }
        }
    }

    /// Konfiguracja bety BEZ sięgania do otoczenia — do podglądu i raportu.
    #[inline]
    pub fn beta(&self) -> Option<&'static KonfigBety> {
        match self.beta {
            UchwytBety::Jest(k) => Some(k),
            _ => None,
        }
    }

    /// WSTRZYKNIĘCIE KONFIGURACJI z pominięciem otoczenia.
    ///
    /// Dla testów i dla warstwy żywej, która woli podać konfigurację wprost.
    /// Przecieka jedną małą strukturę na wywołanie (`Box::leak`), bo uchwyt
    /// jest `'static` — wołać RAZ na proces, nie w pętli.
    pub fn wstrzyknij_bete(&mut self, k: KonfigBety) {
        self.beta = if k.wariant == WariantBety::Off {
            UchwytBety::Brak
        } else {
            UchwytBety::Jest(Box::leak(Box::new(k)))
        };
    }

    /// Wyłącza betę w tym rdzeniu, nie pytając otoczenia (testy parytetu).
    #[inline]
    pub fn wylacz_bete(&mut self) {
        self.beta = UchwytBety::Brak;
    }

    #[inline]
    pub fn bilans_decyzji(&self) -> BilansDecyzji {
        self.bilans_decyzji
    }

    #[inline]
    pub fn dziennik_decyzji(&self) -> &[WpisDecyzji] {
        &self.dziennik_decyzji
    }

    /// Pamięć EA o konkretnym koszyku (`None` = pętla go jeszcze nie widziała).
    #[inline]
    pub fn pamiec_koszyka(&self, id: u32) -> Option<&PamiecKoszyka> {
        self.pamiec.iter().find(|p| p.id == id)
    }

    /// Ustawia licznik interwencji koszyka — WYŁĄCZNIE do testów budżetu.
    #[cfg(test)]
    fn pamiec_test_ustaw(&mut self, id: u32, akcje: u32, ts: Ts) {
        if let Some(p) = self.pamiec.iter_mut().find(|p| p.id == id) {
            p.akcje = akcje;
            p.ostatnia_akcja_ts = ts;
        }
    }

    /// Zegar pętli decyzyjnej. Mierzy CZASEM ZDARZENIA, tak samo jak
    /// [`EaRdzen::zegar_wybija`] — i tak samo daje puls przy zegarze
    /// cofniętym, bo cisza straży to jest ta awaria, dla której to wszystko
    /// powstało.
    #[inline]
    fn zegar_bety(&self, k: &KonfigBety, ts: Ts) -> bool {
        if k.kadencja_s <= 0.0 {
            return true;
        }
        if self.beta_ostatni_ts == 0 {
            return true;
        }
        let delta = ts - self.beta_ostatni_ts;
        delta < 0 || delta as f64 >= k.kadencja_s * 1000.0
    }

    /// Aktualizuje pamięć EA o koszyku i zwraca jej KOPIĘ.
    ///
    /// Kopię, a nie referencję, bo sytuacja idzie potem do taktyki, a taktyka
    /// jest wołana na `&self` — pożyczka mutowalna nie miałaby jak się
    /// skończyć w tym samym wyrażeniu.
    fn pamiec_aktualizuj(&mut self, id: u32, ts: Ts, cena: Px, pl: f64) -> PamiecKoszyka {
        match self.pamiec.iter_mut().find(|p| p.id == id) {
            Some(p) => {
                p.dopisz(cena, pl);
                *p
            }
            None => {
                let p = PamiecKoszyka::nowa(id, ts, cena, pl);
                self.pamiec.push(p);
                p
            }
        }
    }

    /// **PĘTLA DECYZYJNA** — ten moment, w którym program pyta „czy to nadal
    /// ma sens".
    ///
    /// Dzisiejszy silnik układa PLAN w chwili sygnału i wykonuje go stałymi
    /// regułami; pola dzielące margines zmieniają KSZTAŁT planu, ale nie dają
    /// prawa do zmiany zdania. Ta pętla daje to prawo: budzi się na pulsie,
    /// zbiera [`SytuacjaKoszyka`] dla każdego żywego koszyka, pyta taktykę
    /// o [`Rozstrzygniecie`] i oddaje je jednemu egzekutorowi.
    ///
    /// Kolejność jest wiążąca i wygląda tak:
    /// 1. **kontrakt zera** — brak konfiguracji ⇒ wyjście PRZED czymkolwiek,
    /// 2. własny zegar bety (decyzja jest droższa niż obserwacja),
    /// 3. brama N19 (rdzeń niegotowy nie decyduje),
    /// 4. JEDNA migawka pozycji na cały puls, kubełkowana po koszyku,
    /// 5. dla każdego żywego koszyka: pamięć → sytuacja → taktyka → plan,
    /// 6. egzekucja planu, z pełną obsługą odmowy.
    ///
    /// Rozdzielenie 5 i 6 nie jest kosmetyką: plan da się obejrzeć i policzyć
    /// ZANIM cokolwiek pójdzie do brokera, więc `tylko_obserwuj` mierzy
    /// dokładnie tę samą taktykę, którą potem się wykonuje.
    fn petla_decyzyjna<B: Broker>(
        &mut self,
        cfg: &Settings,
        baskets: &[Basket],
        b: &mut B,
        ts: Ts,
    ) {
        // (1) KONTRAKT ZERA — jedyna linijka, którą wykonuje przebieg bez bety.
        let Some(k) = self.uchwyt_bety() else { return };

        // (2) własny zegar
        if !self.zegar_bety(k, ts) {
            return;
        }
        self.beta_ostatni_ts = ts;
        self.beta_pulsy += 1;

        // (3) N19
        if self.wolno_dzialac().is_err() {
            self.bilans_decyzji.pomin(KodPominieciaBety::NiegotowyRdzen);
            return;
        }

        let q = b.quote();
        if !(q.bid.is_finite() && q.ask.is_finite() && q.bid > 0.0 && q.ask >= q.bid) {
            self.bilans_decyzji.pomin(KodPominieciaBety::BrakKwotowania);
            return;
        }

        // OCZY MÓZGU — jedyne miejsce, w którym karmimy wielookresowy obraz
        // rynku. Kadencja pętli jest tu zaletą: świece agregują się co puls,
        // a nie co tik, więc koszt jest ograniczony `kadencja_s`, a nie liczbą
        // kwotowań. Przy 2 s to ~30 tys. aktualizacji na dwa tygodnie zamiast
        // 6,4 mln — i to jest różnica między „mózg kosztuje" a „mózg zabija
        // przemiat".
        self.oczy.tik(ts, q.bid, q.ask);
        let stops = b.stops_level();

        let mut poz = std::mem::take(&mut self.bufor_poz);
        poz.clear();
        for p in b.positions() {
            // Pozycja bez koszyka to SIEROTA — nie należy do żadnego setupu
            // tej nogi, więc żadna sytuacja koszyka nie ma prawa jej widzieć.
            if p.basket.is_none() {
                continue;
            }
            poz.push(migawka_pozycji(p, &q, ts));
        }
        poz.sort_unstable_by(|a, c| (a.koszyk, a.ticket).cmp(&(c.koszyk, c.ticket)));

        // (5) koszyk po koszyku: pamięć → sytuacja → taktyka → plan
        let mut plan = std::mem::take(&mut self.bufor_plan);
        plan.clear();
        self.pamiec
            .retain(|p| baskets.iter().any(|bk| bk.id == p.id && bk.alive()));

        for bk in baskets {
            if !bk.alive() || (cfg.confirmed_exit_retry && bk.pending_exit.is_some()) {
                continue; // koszyk zamknięty nie jest bytem do rozpatrzenia
            }
            if k.dopasuj_source_name
                && !k.format.is_empty()
                && !bk
                    .source_name
                    .to_ascii_lowercase()
                    .contains(&k.format.to_ascii_lowercase())
            {
                self.bilans_decyzji.pomin(KodPominieciaBety::ObcyFormat);
                continue;
            }

            // wycinek migawki należący do tego koszyka (bufor jest posortowany)
            let od = poz.partition_point(|m| m.koszyk < bk.id);
            let doo = poz.partition_point(|m| m.koszyk <= bk.id);
            let moje = &poz[od..doo];

            // ŁĄCZNY WYNIK KOSZYKA — liczony PRZED pamięcią, bo to on karmi
            // szczyt i dno. Reszta agregatów powstaje w `zbierz_sytuacje`.
            let laczny = moje.iter().map(|m| m.wynik_usd).sum::<f64>() + bk.realized;

            // ---- PAMIĘĆ EA (własna historia obserwacji) ----
            let pam = self.pamiec_aktualizuj(bk.id, ts, q.mid(), laczny);

            // ---- WIDOK DECYZYJNY ----
            let syt = zbierz_sytuacje(
                bk,
                moje,
                pam,
                &q,
                self.ostatni_wektor,
                self.stan_efektywny(cfg, bk.id),
                ts,
            );

            // ---- BUDŻET INTERWENCJI (spread płaci się za każdą) ----
            if k.max_akcji_koszyk > 0 && pam.akcje >= k.max_akcji_koszyk {
                self.bilans_decyzji.pomin(KodPominieciaBety::SufitAkcji);
                continue;
            }
            let od_akcji = ts - pam.ostatnia_akcja_ts;
            if k.min_odstep_akcji_s > 0.0
                && pam.ostatnia_akcja_ts > 0
                && od_akcji >= 0
                && (od_akcji as f64) < k.min_odstep_akcji_s * 1000.0
            {
                self.bilans_decyzji.pomin(KodPominieciaBety::OdstepAkcji);
                continue;
            }

            // ---- TAKTYKA ----
            let mut r = self.rozstrzygnij(k, &syt);
            if !source_allows_action(bk, &r.akcja) {
                r = Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
            }
            let (bilety, zlecenia) = match r.akcja {
                AkcjaEa::Trzymaj => (Vec::new(), Vec::new()),
                AkcjaEa::AnulujOczekujace => (Vec::new(), bk.pendings.clone()),
                AkcjaEa::DolozPozycje { .. } | AkcjaEa::WejdzPonownie { .. } => {
                    (Vec::new(), Vec::new())
                }
                _ => (wybierz_bilety(moje, r.bilety), Vec::new()),
            };
            self.bilans_decyzji.obsluz();
            self.bilans_decyzji.akcje[r.akcja.kod().idx()].zapadla += 1;
            self.bilans_decyzji.powody[r.powod.idx()] += 1;
            plan.push(Decyzja {
                ts,
                koszyk: bk.id,
                akcja: r.akcja,
                powod: r.powod,
                bilety,
                zlecenia,
            });
        }

        // (6) EGZEKUCJA — dopiero tutaj kończy się pożyczka koszyków
        for i in 0..plan.len() {
            let wynik = self.wykonaj_akcje(k, cfg, b, &q, stops, &plan[i]);
            let d = &plan[i];
            self.bilans_decyzji.akcje[d.akcja.kod().idx()].dopisz(wynik);
            if !d.akcja.jest_bierna()
                && matches!(wynik, WynikAkcji::Wykonana | WynikAkcji::CzesciowoWykonana)
            {
                if let Some(p) = self.pamiec.iter_mut().find(|p| p.id == d.koszyk) {
                    p.akcje += 1;
                    p.ostatnia_akcja_ts = d.ts;
                    if let AkcjaEa::PrzesunStop(px) = d.akcja {
                        p.ostatni_stop = Some(px);
                    }
                    if matches!(d.akcja, AkcjaEa::ZamknijCzesc(_)) {
                        p.inkasa += 1;
                    }
                }
            }
            // DZIENNIK: `Trzymaj` NIE wchodzi (padłoby na każdym pulsie
            // i utopiło przypadki prawdziwe) — jest za to policzone w bilansie.
            if !d.akcja.jest_bierna() && k.dziennik_max > 0 {
                if self.dziennik_decyzji.len() >= k.dziennik_max {
                    self.dziennik_decyzji.remove(0);
                }
                let szczegol = match d.akcja {
                    AkcjaEa::ZamknijCzesc(u) => u,
                    AkcjaEa::PrzesunStop(px) | AkcjaEa::PrzesunCel(px) => px,
                    AkcjaEa::DolozPozycje { volume, .. }
                    | AkcjaEa::WejdzPonownie { volume, .. } => volume,
                    _ => 0.0,
                };
                self.dziennik_decyzji.push(WpisDecyzji {
                    ts: d.ts,
                    koszyk: d.koszyk,
                    akcja: d.akcja.kod(),
                    powod: d.powod,
                    wynik,
                    szczegol,
                    biletow: (d.bilety.len() + d.zlecenia.len()) as u32,
                });
            }
        }

        plan.clear();
        self.bufor_poz = poz;
        self.bufor_plan = plan;
    }

    // ---------------------------------------------------------------- taktyki

    /// ROZSTRZYGNIĘCIE — rozjazd na warianty. Jedyne miejsce, w którym
    /// szkielet wie, że warianty w ogóle istnieją.
    fn rozstrzygnij(&self, k: &KonfigBety, s: &SytuacjaKoszyka<'_>) -> Rozstrzygniecie {
        match k.wariant {
            WariantBety::Off => Rozstrzygniecie::trzymaj(PowodAkcji::Brak),
            WariantBety::Z => {
                self.zrzut_uczenia(s);
                self.taktyka_z(k, s)
            }
            WariantBety::P => {
                self.zrzut_uczenia(s);
                self.taktyka_p(k, s)
            }
            WariantBety::W => self.taktyka_w(k, s),
        }
    }

    /// **TAKTYKA WARIANTU Z — MIEJSCE AUTORA WARIANTU Z.**
    ///
    /// Dziś zwraca jedną AKCJĘ PRÓBNĄ [`AkcjaEa::Trzymaj`] z powodem
    /// [`PowodAkcji::Szkielet`]. To nie jest zaślepka „na później": to jest
    /// dowód, że pętla przebiega, że sytuacja się składa i że księgowość się
    /// domyka — a wynik przebiegu jest identyczny co do centa, bo `Trzymaj`
    /// nie rusza rachunku.
    ///
    /// Autor wariantu zmienia WYŁĄCZNIE ciało tej funkcji. Wszystko, czego
    /// potrzebuje, jest w `s` ([`SytuacjaKoszyka`]) i w `k.param` / `k.tekst`.
    /// Broker jest niedostępny z premedytacją.
    fn taktyka_z(&self, k: &KonfigBety, s: &SytuacjaKoszyka<'_>) -> Rozstrzygniecie {

        use conduit_mozg::oczy::Okres;

        let prog = k.p("prog_inkasa_usd", 0.0);
        let zapas = k.p("zapas_be", 0.0);
        // Ile świec wstecz uznajemy za „bieżący ruch". Trzy to kompromis:
        // jedna jest szumem, dziesięć to już inny ruch.
        let n = k.p("swiec_mikro", 3.0).max(1.0) as usize;
        let n_makro = k.p("swiec_makro", 5.0).max(1.0) as usize;
        // Próg wyczerpania: gdzie w zakresie ostatnich świec siedzi cena.
        // 0 = przy dnie, 1 = przy szczycie. Dla BUY „wyczerpany" znaczy
        // „wysoko w zakresie i przestał iść".
        let prog_wyczerpania = k.p("prog_wyczerpania", 0.80).clamp(0.0, 1.0);

        let zywe: Vec<&MigawkaPozycji> = s.pozycje.iter().filter(|p| !p.frozen).collect();
        if zywe.is_empty() {
            return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
        }

        let mid = (s.q.bid + s.q.ask) * 0.5;
        let mikro = self.oczy.okno(Okres::M5);
        let makro = self.oczy.okno(Okres::H1);

        let poz_mikro = mikro.polozenie_w_zakresie(n, mid);
        let poz_makro = makro.polozenie_w_zakresie(n_makro, mid);
        if !poz_mikro.is_finite() || !poz_makro.is_finite() {
            return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
        }

        // Dla SELL wszystko jest lustrzane: „wysoko w zakresie" znaczy dla
        // nas źle, więc odbijamy położenie zamiast pisać dwie gałęzie reguł.
        let (poz_mikro, poz_makro) = match s.side {
            Side::Buy => (poz_mikro, poz_makro),
            Side::Sell => (1.0 - poz_mikro, 1.0 - poz_makro),
        };

        let mikro_wyczerpany = poz_mikro >= prog_wyczerpania;
        let makro_niesie = poz_makro >= 0.5;

        // ---- ocalały: stop na wejściu, gdy makro przestał nieść -----------
        if zywe.len() == 1 {
            let o = zywe[0];
            if o.wynik_usd <= 0.0 {
                return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
            }
            let poziom = o.open_price + s.side.sign() * zapas;
            let juz = match (o.sl, s.side) {
                (Some(sl), Side::Buy) => sl >= poziom - 1e-9,
                (Some(sl), Side::Sell) => sl <= poziom + 1e-9,
                (None, _) => false,
            };
            if juz {
                return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
            }
            return Rozstrzygniecie::nowe(AkcjaEa::PrzesunStop(poziom), PowodAkcji::Taktyka(2))
                .na(WyborBiletow::Jeden(o.ticket));
        }

        // ---- inkaso płytkiej nogi: TYLKO gdy mikro się wyczerpał ----------
        //
        // Trzy warunki naraz, każdy z własnego powodu:
        //  * koszyk na plusie ponad próg — inaczej inkasujemy stratę;
        //  * mikro wyczerpany — inaczej oddajemy resztę ruchu;
        //  * makro nadal niesie — inaczej nie ma po co zostawiać runnera
        //    i lepiej zabezpieczyć go stopem niż dokładać mu towarzystwa.
        if s.laczny_usd <= prog || !mikro_wyczerpany || !makro_niesie {
            return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
        }
        let najplytszy = zywe
            .iter()
            .min_by(|a, b| match s.side {
                Side::Buy => b
                    .open_price
                    .partial_cmp(&a.open_price)
                    .unwrap_or(std::cmp::Ordering::Equal),
                Side::Sell => a
                    .open_price
                    .partial_cmp(&b.open_price)
                    .unwrap_or(std::cmp::Ordering::Equal),
            })
            .copied();
        match najplytszy {
            Some(p) if p.wynik_usd > 0.0 => {
                Rozstrzygniecie::nowe(AkcjaEa::ZamknijCalosc, PowodAkcji::Taktyka(1))
                    .na(WyborBiletow::Jeden(p.ticket))
            }
            _ => Rozstrzygniecie::trzymaj(PowodAkcji::Brak),
        }
    }

    /// Widok koszyka w postaci, ktorej oczekuje mozg — zeby cechy liczyla
    /// **ta sama funkcja** w treningu i w decyzji. Dwie kopie tego wzoru
    /// rozjechalyby sie przy pierwszej poprawce, a model uczylby sie czegos
    /// innego, niz potem widzi.
    fn koszyk_mozgu(&self, s: &SytuacjaKoszyka<'_>) -> conduit_mozg::wejscie::Koszyk {
        use conduit_mozg::rama::{GeometriaPomyslu, Strona};
        use conduit_mozg::wejscie::{Koszyk, Szczebel};
        let strona = match s.side {
            Side::Buy => Strona::Buy,
            Side::Sell => Strona::Sell,
        };
        let zywe: Vec<&MigawkaPozycji> = s.pozycje.iter().filter(|p| !p.frozen).collect();
        // Glebokosc: 0 = najplytszy, czyli o NAJGORSZEJ cenie wejscia.
        let mut kolejnosc: Vec<usize> = (0..zywe.len()).collect();
        kolejnosc.sort_by(|&a, &b| match s.side {
            Side::Buy => zywe[b]
                .open_price
                .partial_cmp(&zywe[a].open_price)
                .unwrap_or(std::cmp::Ordering::Equal),
            Side::Sell => zywe[a]
                .open_price
                .partial_cmp(&zywe[b].open_price)
                .unwrap_or(std::cmp::Ordering::Equal),
        });
        let mut gl = vec![0u16; zywe.len()];
        for (r, &i) in kolejnosc.iter().enumerate() {
            gl[i] = r as u16;
        }
        let szczeble = zywe
            .iter()
            .enumerate()
            .map(|(i, p)| Szczebel {
                ticket: p.ticket as u64,
                strona,
                cena_wejscia: p.open_price,
                wolumen: p.volume,
                sl: p.sl,
                tp: p.tp,
                glebokosc: gl[i],
                ts_otwarcia: p.open_ts,
                wynik_usd: p.wynik_usd,
                szczyt_usd: p.wynik_usd.max(0.0),
                dno_usd: f64::NAN,
            })
            .collect();
        Koszyk {
            id: s.koszyk,
            rama_id: s.koszyk,
            geometria: GeometriaPomyslu {
                strona,
                krawedz_blizsza: s.entry_hi,
                krawedz_dalsza: s.entry_lo,
                sl: s.sl,
                cele: s.tps.to_vec(),
            },
            ts_zawiazania: s.ts - (s.wiek_min * 60_000.0) as Ts,
            szczeble,
            oczekujacych: s.zlecenia.len() as u16,
            etap_celu: s.tp_stage.min(u8::MAX as usize) as u8,
            rf_ogloszony: s.secured,
            budzet_wydany_usd: 0.0,
        }
    }

    fn zrzut_uczenia(&self, s: &SytuacjaKoszyka<'_>) {
        use std::io::Write;

        // BUFOR, NIE PISANIE PO WIERSZU.
        //
        // Pierwsza wersja otwierala plik bez bufora i brala muteks na KAZDY
        // wiersz. Przy kadencji 5 s na dwumiesiecznym oknie to sa miliony
        // wywolan `write` — i przebieg zszedl do 146 tikow na sekunde wobec
        // normalnych 1–2 milionow. Dziesiec tysiecy razy wolniej, i to nie
        // z powodu liczenia cech, tylko z powodu dysku.
        //
        // Bufor 8 MB zamienia miliony malych zapisow w kilkanascie duzych.
        // Muteks zostaje, bo silnikow w lancuchu bywa kilka, ale brany jest
        // teraz na operacje w pamieci, a nie na wejscie-wyjscie.
        static PLIK: std::sync::OnceLock<
            Option<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>,
        > = std::sync::OnceLock::new();
        let f = PLIK.get_or_init(|| {
            let p = std::env::var("EA_UCZENIE").ok()?;
            let f = std::fs::File::create(&p).ok()?;
            let mut w = std::io::BufWriter::with_capacity(8 << 20, f);
            let mut naglowek = String::from("ts;koszyk;wynik_teraz;wolumen;nog");
            for n in conduit_mozg::cechy::NAZWY.iter() {
                naglowek.push(';');
                naglowek.push_str(n);
            }
            let _ = writeln!(w, "{naglowek}");
            Some(std::sync::Mutex::new(w))
        });
        let Some(m) = f.as_ref() else { return };

        let k = self.koszyk_mozgu(s);
        let rach = conduit_mozg::wejscie::Rachunek {
            saldo: s.w.balance,
            equity: s.w.equity,
            margines_uzyty: s.w.margines,
            margines_wolny: s.w.wolny_margines,
            // s.w.ml = poziom marginesu w procentach; zero znaczy BRAK
            // EKSPOZYCJI, a nie „na krawedzi" — i to rozroznienie musi przejsc
            // do mozgu, bo inaczej pusty rachunek wygladalby jak rachunek
            // przed wezwaniem do uzupelnienia.
            poziom_marginesu: if s.w.ml > 0.0 { Some(s.w.ml) } else { None },
            dzwignia: 500.0,
            wynik_dnia_usd: 0.0,
            seria_stopow: 0,
        };
        let ryzyko = match (s.sl, s.pozycje.first()) {
            (Some(sl), Some(p)) => (p.open_price - sl).abs(),
            _ => f64::NAN,
        };
        let c = conduit_mozg::cechy::cechy(&self.oczy, &k, &rach, s.ts, ryzyko);

        if let Ok(mut w) = m.lock() {
            // Jeden `write!` na wiersz zamiast jednego na liczbe: `format!`
            // alokuje, a alokacja w petli po 6,4 mln pulsow jest widoczna
            // w profilu.
            let _ = write!(
                w,
                "{};{};{:.4};{:.4};{}",
                s.ts,
                s.koszyk,
                s.laczny_usd,
                s.wolumen,
                k.szczeble.len()
            );
            for v in c.iter() {
                let _ = write!(w, ";{v:.6}");
            }
            let _ = writeln!(w);
        }
    }

    fn taktyka_w(&self, k: &KonfigBety, s: &SytuacjaKoszyka<'_>) -> Rozstrzygniecie {
        // Koszyk, ktory juz gra, nie podlega wetu — patrz doc.
        if s.pozycje.iter().any(|p| !p.frozen) || s.zlecenia.is_empty() {
            return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
        }
        // Jedno weto na koszyk. Bez tego licznika kazdy puls probowalby
        // kasowac te same zlecenia i dziennik zalalby sie powtorkami.
        if s.pamiec.akcje > 0 {
            return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
        }

        let prog = k.p("weto_min_zgodnosc", 0.0);
        if prog > 0.0 {
            let zgod = self.oczy.wlasny_kierunek() * s.side.sign();
            if !(zgod >= prog) {
                return Rozstrzygniecie::nowe(AkcjaEa::AnulujOczekujace, PowodAkcji::Taktyka(20));
            }
        }

        // ODCHYLENIE od sredniej: zakaz kupowania daleko nad nia.
        let prog_odch = k.p("weto_max_odchylenie", 0.0);
        if prog_odch > 0.0 {
            use conduit_mozg::oczy::Okres;
            let mid = (s.q.bid + s.q.ask) * 0.5;
            let d = self.oczy.okno(Okres::H1).odchylenie(10, mid) * s.side.sign();
            if d.is_finite() && d > prog_odch {
                return Rozstrzygniecie::nowe(AkcjaEa::AnulujOczekujace, PowodAkcji::Taktyka(21));
            }
        }

        // RYZYKO PORTFELA: nowy koszyk nie wchodzi, gdy rachunek juz niesie
        // wiecej, niz wolno. Ta sama arytmetyka co w `taktyka_p`.
        let limit_pct = k.p("weto_portfel_ryzyko_pct", 0.0);
        if limit_pct > 0.0 && s.w.equity > 0.0 && s.w.ryzyko_pct >= limit_pct {
            return Rozstrzygniecie::nowe(AkcjaEa::AnulujOczekujace, PowodAkcji::Taktyka(22));
        }

        Rozstrzygniecie::trzymaj(PowodAkcji::Brak)
    }

    fn taktyka_p(&self, k: &KonfigBety, s: &SytuacjaKoszyka<'_>) -> Rozstrzygniecie {
        use conduit_mozg::oczy::Okres;

        let zywe: Vec<&MigawkaPozycji> = s.pozycje.iter().filter(|p| !p.frozen).collect();
        let mid = (s.q.bid + s.q.ask) * 0.5;

        // ---- 1. PRZEJECIE KONTROLI --------------------------------------
        //
        // Dopoki wisza zlecenia presetu, a my nie mamy ani jednej pozycji,
        // koszyk jest cudzym planem. Kasujemy go — to jest pierwsza i jedyna
        // rzecz, jaka EA robi z geometria presetu.
        if zywe.is_empty() && !s.zlecenia.is_empty() {
            return Rozstrzygniecie::nowe(AkcjaEa::AnulujOczekujace, PowodAkcji::Taktyka(10));
        }

        let dokladaj = k.p("dokladaj_gdy_otwarte", 0.0) >= 0.5;
        let wolno_wejsc = if zywe.is_empty() {
            true
        } else if !dokladaj {
            false
        } else {
            let wejsc_max = k.p("wejsc_max", 4.0).max(1.0) as usize;
            if zywe.len() >= wejsc_max {
                false
            } else {
                let krok = k.p("krok_dokladki_usd", 0.0);
                if krok <= 0.0 {
                    true
                } else {
                    // najgorsze dotychczasowe wejscie = to, od ktorego mierzymy
                    // krok; inaczej po odbiciu ceny doklada sie natychmiast
                    let odn =
                        zywe.iter()
                            .map(|p| p.open_price)
                            .fold(f64::NAN, |a, b| match s.side {
                                Side::Buy => {
                                    if a.is_nan() {
                                        b
                                    } else {
                                        a.min(b)
                                    }
                                }
                                Side::Sell => {
                                    if a.is_nan() {
                                        b
                                    } else {
                                        a.max(b)
                                    }
                                }
                            });
                    odn.is_finite() && (odn - mid) * s.side.sign() >= krok
                }
            }
        };
        if wolno_wejsc {
            // ILE RAZY WOLNO WEJSC W JEDEN POMYSL.
            //
            // Pierwsza wersja pozwalala RAZ — i to bylo za malo z dwoch
            // powodow. Praktycznego: 172 wejscia na dwa tygodnie to 190
            // transakcji, gdy preset robi 649, a przy tak malej probie kazdy
            // wynik jest opowiescia o kilkunastu zdarzeniach. I zasadniczego:
            // kanal SAM rekomenduje siatke, zeby zlapac wiecej okazji — jeden
            // strzal w strefe to rezygnacja z tego, co ta strefa oferuje.
            //
            // Roznica wobec presetu zostaje jednak istotna: preset rozstawia
            // szczeble GEOMETRIA (strefa dzielona przez krok, niezaleznie od
            // tego, co robi rynek), a EA doklada, gdy SAMO uzna — i przestaje,
            // gdy przestaje uznawac.
            let wejsc_max = k.p("wejsc_max", 4.0).max(1.0) as u32;
            if s.pamiec.akcje > wejsc_max {
                return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
            }

            // ZLECENIE OCZEKUJACE, NIE CZEKANIE NA SPOTKANIE Z CENA.
            //
            // Pierwsza wersja wymagala, zeby cena byla W STREFIE w chwili
            // pulsu — i weszla 22 razy na 173 przejete koszyki. Powod byl
            // wlasny: kasujemy limity presetu, ktore CZEKALY przy strefie,
            // a potem pytamy, czy cena akurat tam jest. Zwykle nie jest —
            // bo po to wlasnie tam czekaly.
            //
            // EA ma wiec wybrac POZIOM i tam polozyc wlasne zlecenie. To jest
            // decyzja „gdzie", ktorej preset nie podejmuje: on rozstawia
            // szczeble geometria strefy, my kladziemy JEDEN w miejscu
            // wybranym z glebokosci strefy.
            let (lo, hi) = (s.entry_lo.min(s.entry_hi), s.entry_lo.max(s.entry_hi));
            let szer = hi - lo;
            if szer <= 0.0 {
                return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
            }
            let juz = s.pamiec.akcje.saturating_sub(1).min(8) as f64;
            let g = (k.p("glebokosc_wejscia", 0.2) + juz * k.p("krok_glebokosci", 0.25))
                .clamp(0.0, 1.0);
            let poziom = match s.side {
                Side::Buy => hi - szer * g,
                Side::Sell => lo + szer * g,
            };
            // Zlecenie MUSI byc po wlasciwej stronie rynku, inaczej broker je
            // odrzuci albo — gorzej — polityka `pending_cross_policy` zamieni
            // je w wejscie rynkowe, czyli dokladnie odwrotnie do intencji.
            let po_stronie = match s.side {
                Side::Buy => poziom < s.q.ask,
                Side::Sell => poziom > s.q.bid,
            };
            let limit = if po_stronie { Some(poziom) } else { None };
            // ILE: z wolnego marginesu i z odleglosci do stopa, liczone TERAZ.
            // Bez stopa nie wchodzimy — pozycja bez zdefiniowanego ryzyka nie
            // jest transakcja, tylko zakladem.
            let sl = match s.sl {
                Some(v) => v,
                None => return Rozstrzygniecie::trzymaj(PowodAkcji::Brak),
            };
            // Ryzyko liczymy od POZIOMU, na ktorym stanie zlecenie — a nie
            // od ceny biezacej. Inaczej wielkosc pozycji opisywalaby chwile
            // zlozenia zlecenia, a nie chwile wejscia w rynek.
            let dyst = (limit.unwrap_or(mid) - sl).abs();
            if dyst <= 0.0 {
                return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
            }
            let ryzyko_usd = s.w.equity * k.p("ryzyko_pct", 1.0) / 100.0;
            let mut lot = (ryzyko_usd / (dyst * XAU_CONTRACT)).clamp(0.01, k.p("lot_max", 1.0));

            let limit_pct = k.p("portfel_ryzyko_pct", 0.0);
            if limit_pct > 0.0 && s.w.equity > 0.0 {
                let budzet = s.w.equity * limit_pct / 100.0 - s.w.ryzyko_usd;
                // `ryzyko_usd` bywa nieskonczone (pozycja bez stopa) — wtedy
                // budzet jest ujemny albo NaN i wejscie odpada, co jest
                // zachowaniem zamierzonym: nie dokladamy do rachunku, ktorego
                // ryzyka nie da sie policzyc.
                if !(budzet > 0.0) {
                    return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
                }
                let sufit = budzet / (dyst * XAU_CONTRACT);
                // TRYB 1 (domyslny): PRZYTNIJ lot do budzetu. Wchodzimy mniejsi,
                //   ale wchodzimy — liczba koszykow zostaje, a o nia wlasciciel
                //   prosil wprost („wykorzystac jeszcze wiecej koszykow").
                // TRYB 0: ODMOW wejscia w calosci. Ostrzejsze, ale traci koszyk.
                if k.p("portfel_tryb", 1.0) >= 0.5 {
                    lot = lot.min(sufit);
                    // Ponizej kroku brokera nie ma czego przycinac — wejscie
                    // 0,004 lota nie istnieje, wiec zostaje odmowa.
                    if lot < 0.01 {
                        return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
                    }
                } else if lot > sufit {
                    return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
                }
            }

            let prog_zgod = k.p("wejscie_min_zgodnosc", 0.0);
            if prog_zgod > 0.0 {
                let zgod = self.oczy.wlasny_kierunek() * s.side.sign();
                if !(zgod >= prog_zgod) {
                    return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
                }
            }
            // ODCHYLENIE: zakaz kupowania daleko nad srednia (i sprzedawania
            // daleko pod nia). Rdzen wszystkich kopert — Bollingera,
            // Nadaraya-Watsona, Kijun-Sen. 0 = brak progu.
            let prog_odch = k.p("wejscie_max_odchylenie", 0.0);
            if prog_odch > 0.0 {
                let d = self.oczy.okno(Okres::H1).odchylenie(10, mid) * s.side.sign();
                if d.is_finite() && d > prog_odch {
                    return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
                }
            }
            return Rozstrzygniecie::nowe(
                AkcjaEa::WejdzPonownie {
                    side: s.side,
                    volume: lot,
                    limit,
                    sl: Some(sl),
                    tp: None,
                },
                PowodAkcji::Taktyka(11),
            );
        }

        // ---- 2b. LUKA WEEKENDOWA ----------------------------------------
        //
        // Decyzja Z POWODU CZASU, nie z powodu ceny — i dlatego preset jej
        // nie podejmuje. Dzialamy TYLKO gdy koszyk jest na plusie albo gdy
        // prog kaze wyjsc bezwarunkowo: zamykanie stratnej pozycji przed
        // weekendem to realizowanie straty w zamian za usuniecie ryzyka,
        // a to jest osobna decyzja i osobna os.
        let okno_min = k.p("przed_weekendem_min", 0.0);
        if okno_min > 0.0 {
            let do_zam =
                minut_do_zamkniecia_tygodnia(s.w.ts, k.p("weekend_zamkniecie_utc_h", 21.0));
            if do_zam <= okno_min {
                // 0 = zamknij calosc; 0,5 = zejdz do polowy ekspozycji.
                // Ulamek realizujemy przez zamkniecie NAJGLEBSZYCH nog —
                // te maja najlepsza cene wejscia, wiec zostawiamy je jako
                // ostatnie... nie: zostawiamy WLASNIE je, bo maja najwiekszy
                // zapas do stopa i najmniej ucierpia na luce.
                let ulamek = k.p("przed_weekendem_redukcja", 0.0).clamp(0.0, 1.0);
                let tylko_zysk = k.p("przed_weekendem_tylko_zysk", 1.0) >= 0.5;
                if !tylko_zysk || s.laczny_usd > 0.0 {
                    if ulamek <= 0.0 {
                        return Rozstrzygniecie::nowe(
                            AkcjaEa::ZamknijCalosc,
                            PowodAkcji::Taktyka(14),
                        )
                        .na(WyborBiletow::Wszystkie);
                    }
                    let do_zamkniecia = ((zywe.len() as f64) * (1.0 - ulamek)).round() as usize;
                    if do_zamkniecia > 0 && do_zamkniecia < zywe.len() {
                        return Rozstrzygniecie::nowe(
                            AkcjaEa::ZamknijCalosc,
                            PowodAkcji::Taktyka(14),
                        )
                        .na(WyborBiletow::NajgorszeN(do_zamkniecia.min(255) as u8));
                    }
                }
            }
        }

        // ---- 3. PROWADZENIE POZYCJI -------------------------------------
        //
        // Przy wielu nogach prowadzimy KOSZYK, nie pojedyncza pozycje: jeden
        // pomysl z kanalu to jedna logiczna transakcja, a nogi sa jej czescia.
        // Reprezentantem do liczenia ryzyka jest noga NAJPLYTSZA — ta o
        // najgorszej cenie wejscia, czyli ta, ktora najwczesniej znajdzie sie
        // pod woda. Liczenie ryzyka od najlepszej nogi zawyzaloby bezpieczenstwo.
        let poz_biezaca = zywe
            .iter()
            .min_by(|a, b| match s.side {
                Side::Buy => b
                    .open_price
                    .partial_cmp(&a.open_price)
                    .unwrap_or(std::cmp::Ordering::Equal),
                Side::Sell => a
                    .open_price
                    .partial_cmp(&b.open_price)
                    .unwrap_or(std::cmp::Ordering::Equal),
            })
            .copied()
            .unwrap_or(zywe[0]);
        let mikro = self.oczy.okno(Okres::M5);
        let makro = self.oczy.okno(Okres::H1);
        let pm = mikro.polozenie_w_zakresie(k.p("swiec_mikro", 3.0).max(1.0) as usize, mid);
        let pmakro = makro.polozenie_w_zakresie(k.p("swiec_makro", 5.0).max(1.0) as usize, mid);
        if !pm.is_finite() || !pmakro.is_finite() {
            return Rozstrzygniecie::trzymaj(PowodAkcji::Brak);
        }
        let (pm, pmakro) = match s.side {
            Side::Buy => (pm, pmakro),
            Side::Sell => (1.0 - pm, 1.0 - pmakro),
        };

        // WYJSCIE: mikro wyczerpany I makro sie odwrocil. Jedno bez drugiego
        // nie wystarcza — mikro sam w sobie to zwykla cofka wewnatrz trendu,
        // a makro sam bez wyczerpania mikro znaczy „wychodzisz w srodku ruchu".
        let mikro_wyczerpany = pm >= k.p("prog_wyczerpania", 0.80).clamp(0.0, 1.0);
        let makro_odwrocony = pmakro < k.p("prog_makro", 0.50).clamp(0.0, 1.0);
        if mikro_wyczerpany && makro_odwrocony && s.laczny_usd > 0.0 {
            return Rozstrzygniecie::nowe(AkcjaEa::ZamknijCalosc, PowodAkcji::Taktyka(12))
                .na(WyborBiletow::Wszystkie);
        }

        let sl0 = match s.sl {
            Some(v) => v,
            None => return Rozstrzygniecie::trzymaj(PowodAkcji::Brak),
        };
        let ryzyko_ceny = (poz_biezaca.open_price - sl0).abs();
        let zysk_ceny = (mid - poz_biezaca.open_price) * s.side.sign();
        if ryzyko_ceny > 0.0 && zysk_ceny >= ryzyko_ceny * k.p("mnoznik_be", 1.0) {
            let poziom = poz_biezaca.open_price + s.side.sign() * k.p("zapas_be", 0.0);
            let juz = match (poz_biezaca.sl, s.side) {
                (Some(sl), Side::Buy) => sl >= poziom - 1e-9,
                (Some(sl), Side::Sell) => sl <= poziom + 1e-9,
                (None, _) => false,
            };
            if !juz {
                return Rozstrzygniecie::nowe(
                    AkcjaEa::PrzesunStop(poziom),
                    PowodAkcji::Taktyka(13),
                )
                .na(WyborBiletow::Wszystkie);
            }
        }
        Rozstrzygniecie::trzymaj(PowodAkcji::Brak)
    }

    // -------------------------------------------------------------- egzekutor

    fn wykonaj_akcje<B: Broker>(
        &mut self,
        k: &KonfigBety,
        cfg: &Settings,
        b: &mut B,
        q: &Quote,
        stops: f64,
        d: &Decyzja,
    ) -> WynikAkcji {
        if d.akcja.jest_bierna() {
            return WynikAkcji::BezPracy;
        }
        if k.tylko_obserwuj {
            return WynikAkcji::Obserwacja;
        }
        let mut r = Rachuba::default();
        let opis = format!("EA-BETA/{}/{}", k.wariant.kod(), d.powod.kod());

        match d.akcja {
            AkcjaEa::Trzymaj => return WynikAkcji::BezPracy,

            // ---------------------------------------------------- zamknięcia
            AkcjaEa::ZamknijCalosc => {
                let cele: Vec<Ticket> = d
                    .bilety
                    .iter()
                    .filter(|t| b.find_position(**t).map(|p| !p.frozen).unwrap_or(false))
                    .copied()
                    .collect();
                r.pusto += d.bilety.len() - cele.len();
                for t in cele {
                    match b.close_position(t, CloseReason::BasketClose) {
                        Ok(_) => r.ok += 1,
                        Err(_) => r.odmowa_brokera(&mut self.bilans, &mut self.bilans_decyzji),
                    }
                }
            }

            AkcjaEa::ZamknijCzesc(ulamek) => {
                let u = ulamek.clamp(0.0, 1.0);
                // Ta sama arytmetyka, którą stosuje `bank_on_tp`: krok 0,01
                // lota, minimum 0,01 i zostawiamy 0,01 — inaczej „częściowe"
                // zamknięcie zamyka całą pozycję.
                let mut zlec: Vec<(Ticket, f64)> = Vec::new();
                for t in &d.bilety {
                    let Some(p) = b.find_position(*t) else {
                        r.pusto += 1;
                        continue;
                    };
                    if p.frozen {
                        r.pusto += 1;
                        continue;
                    }
                    let want = krok_lota(p.volume * u);
                    let cut = want.max(0.01).min((p.volume - 0.01).max(0.0));
                    if cut < 0.01 - 1e-9 {
                        r.pusto += 1;
                        continue;
                    }
                    zlec.push((*t, cut));
                }
                for (t, v) in zlec {
                    match b.close_partial(t, v, CloseReason::Partial) {
                        Ok(_) => r.ok += 1,
                        Err(_) => r.odmowa_brokera(&mut self.bilans, &mut self.bilans_decyzji),
                    }
                }
            }

            // ------------------------------------------------------- poziomy
            AkcjaEa::PrzesunStop(cel) => {
                let mut zlec: Vec<(Ticket, Px, Option<Px>)> = Vec::new();
                for t in &d.bilety {
                    let Some(p) = b.find_position(*t) else {
                        r.pusto += 1;
                        continue;
                    };
                    if p.frozen {
                        r.pusto += 1;
                        continue;
                    }
                    // ZAKAZ LUZOWANIA: przesunięcie stopa OD ceny to jest
                    // podniesienie ryzyka po zawiązaniu koszyka, czyli
                    // dokładnie to, czego zabrania zapadka N10.
                    let luzuje = |px: Px| match p.sl {
                        Some(s) => (px - s) * p.side.sign() < -1e-9,
                        None => false,
                    };
                    if luzuje(cel) && !k.pozwol_luzowac_stop {
                        r.odmowa_poziomu(&mut self.bilans_decyzji);
                        continue;
                    }
                    let px = if crate::broker::sl_is_valid(p.side, cel, q, stops) {
                        cel
                    } else {
                        crate::broker::clamp_sl(p.side, cel, q, stops)
                    };
                    // dosunięcie mogło zamienić zacieśnienie w luzowanie
                    if luzuje(px) && !k.pozwol_luzowac_stop {
                        r.odmowa_poziomu(&mut self.bilans_decyzji);
                        continue;
                    }
                    if !crate::broker::sl_is_valid(p.side, px, q, stops) {
                        r.odmowa_poziomu(&mut self.bilans_decyzji);
                        continue;
                    }
                    if p.sl.map(|s| (s - px).abs() < 1e-9).unwrap_or(false) {
                        r.pusto += 1; // stop już tam stoi — nie płacimy za nic
                        continue;
                    }
                    zlec.push((*t, px, p.tp));
                }
                for (t, px, tp) in zlec {
                    match b.modify_position(t, Some(px), tp) {
                        Ok(()) => r.ok += 1,
                        Err(_) => r.odmowa_brokera(&mut self.bilans, &mut self.bilans_decyzji),
                    }
                }
            }

            AkcjaEa::PrzesunCel(cel) => {
                let mut zlec: Vec<(Ticket, Option<Px>, Px)> = Vec::new();
                for t in &d.bilety {
                    let Some(p) = b.find_position(*t) else {
                        r.pusto += 1;
                        continue;
                    };
                    if p.frozen {
                        r.pusto += 1;
                        continue;
                    }
                    if !tp_is_valid(p.side, cel, q, stops) {
                        r.odmowa_poziomu(&mut self.bilans_decyzji);
                        continue;
                    }
                    if p.tp.map(|t0| (t0 - cel).abs() < 1e-9).unwrap_or(false) {
                        r.pusto += 1;
                        continue;
                    }
                    zlec.push((*t, p.sl, cel));
                }
                for (t, sl, tp) in zlec {
                    match b.modify_position(t, sl, Some(tp)) {
                        Ok(()) => r.ok += 1,
                        Err(_) => r.odmowa_brokera(&mut self.bilans, &mut self.bilans_decyzji),
                    }
                }
            }

            // -------------------------------------------------------- wejścia
            AkcjaEa::DolozPozycje {
                side,
                volume,
                limit,
                sl,
                tp,
                level,
            } => {
                self.zloz_wejscie(
                    cfg, b, q, stops, side, volume, limit, sl, tp, level, &opis, &mut r,
                );
            }
            AkcjaEa::WejdzPonownie {
                side,
                volume,
                limit,
                sl,
                tp,
            } => {
                self.zloz_wejscie(
                    cfg, b, q, stops, side, volume, limit, sl, tp, 0, &opis, &mut r,
                );
            }

            // ------------------------------------------------------- zlecenia
            AkcjaEa::AnulujOczekujace => {
                let cele: Vec<Ticket> = d
                    .zlecenia
                    .iter()
                    .filter(|t| b.pendings().iter().any(|p| p.ticket == **t && !p.frozen))
                    .copied()
                    .collect();
                r.pusto += d.zlecenia.len() - cele.len();
                for t in cele {
                    match b.cancel_pending(t) {
                        Ok(()) => r.ok += 1,
                        Err(_) => r.odmowa_brokera(&mut self.bilans, &mut self.bilans_decyzji),
                    }
                }
            }
        }

        self.bilans_decyzji.zlecen_wyslanych += r.ok + r.blad;
        self.bilans_decyzji.zlecen_odrzuconych += r.blad;
        if r.pusto > 0 {
            self.bilans_decyzji.odnotuj(KodPominieciaBety::BezPracy);
        }
        r.wynik()
    }

    /// Wspólna droga dla [`AkcjaEa::DolozPozycje`] i [`AkcjaEa::WejdzPonownie`]:
    /// `limit = None` idzie po rynku, `Some(px)` kładzie zlecenie oczekujące.
    ///
    /// Zlecenie oczekujące ma DWA walidatory, nie jeden, i to nie jest
    /// nadgorliwość: cena aktywacji mierzy się od rynku
    /// ([`limit_price_is_valid`] / [`stop_price_is_valid`]), a jego stop —
    /// od CENY AKTYWACJI ([`pending_sl_is_valid`], MT5 kod 10016). Bez tego
    /// drugiego powstaje „szczebel-widmo": poziom, którego żywy broker nie
    /// przyjmie, a symulacja liczy z niego wynik.
    #[allow(clippy::too_many_arguments)]
    fn zloz_wejscie<B: Broker>(
        &mut self,
        cfg: &Settings,
        b: &mut B,
        q: &Quote,
        stops: f64,
        side: Side,
        volume: f64,
        limit: Option<Px>,
        sl: Option<Px>,
        tp: Option<Px>,
        level: i32,
        opis: &str,
        r: &mut Rachuba,
    ) {
        if self.continuation_entry_hold {
            r.odmowa_brokera(&mut self.bilans,&mut self.bilans_decyzji);
            return;
        }
        if cfg.closed_profit_net_costs && (!cfg.basket_realized_broker_only || !b.cost_net_supported()) {
            b.report_cost_consumer_fault("EA canonical-net dependency/pipeline requires review");
            eprintln!("[EA-BETA][COST HOLD] unsupported/inactive canonical-net pipeline; new entry rejected");
            r.odmowa_brokera(&mut self.bilans,&mut self.bilans_decyzji);
            return;
        }
        let dol = if cfg.lot_min.is_finite() && cfg.lot_min > 0.0 {
            cfg.lot_min
        } else {
            0.01
        };
        let vol = if cfg.order_volume_contract_v2 {
            let a = b.account();
            let spec = crate::volume_contract::VolumeSpec {
                minimum: b.volume_min(), step: b.volume_step(), maximum: b.volume_max(),
            };
            let limits = crate::volume_contract::StrategyVolumeLimits {
                minimum: cfg.lot_min, maximum: cfg.lot_max,
                capital_per_lot: cfg.lot_max_z_salda,
                capital: cfg.podstawa_lota_z_konta(a.balance, a.equity, a.credit),
            };
            match crate::volume_contract::normalize_open_volume(volume, spec, limits) {
                Ok(v) => v,
                Err(reason) => {
                    eprintln!("[EA-BETA][VolumeContract::{reason:?}] opening rejected; requested={volume:?}, broker={spec:?}, limits={limits:?}");
                    r.odmowa_brokera(&mut self.bilans, &mut self.bilans_decyzji);
                    return;
                }
            }
        } else {
            krok_lota(volume.max(dol))
        };
        if !vol.is_finite() || vol < dol - 1e-9 {
            r.pusto += 1;
            return;
        }
        match limit {
            None => {
                let sl_ok = match sl {
                    Some(s) if !crate::broker::sl_is_valid(side, s, q, stops) => {
                        Some(crate::broker::clamp_sl(side, s, q, stops))
                    }
                    inne => inne,
                };
                if let Some(t) = tp {
                    if !tp_is_valid(side, t, q, stops) {
                        r.odmowa_poziomu(&mut self.bilans_decyzji);
                        return;
                    }
                }
                let Some(vol)=self.profit_budget_volume(cfg,b,side,q.entry(side),sl_ok,vol,r) else{return;};
                match b.open_market(OrderReq {
                    side,
                    volume: vol,
                    sl: sl_ok,
                    tp,
                    basket: None,
                    level,
                    is_toucher: false,
                    comment: opis.to_string(),
                }) {
                    Ok(_) => r.ok += 1,
                    Err(_) => r.odmowa_brokera(&mut self.bilans, &mut self.bilans_decyzji),
                }
            }
            Some(px0) => {
                // Po WŁAŚCIWEJ stronie rynku decyduje, czy to limit czy stop:
                // limit leży PRZED rynkiem, stop ZA nim. Zgadywanie kierunku
                // z samej ceny jest tu jedyną uczciwą drogą, bo taktyka podaje
                // poziom, a nie typ zlecenia.
                let po_rynku = (px0 - q.entry(side)) * side.sign() > 0.0;
                let (kind, px) = if po_rynku {
                    (PendingKind::stop(side), px0)
                } else {
                    (
                        PendingKind::limit(side),
                        clamp_limit_price(side, px0, q, stops),
                    )
                };
                let ok_ceny = match kind {
                    PendingKind::BuyStop | PendingKind::SellStop => {
                        stop_price_is_valid(side, px, q, stops)
                    }
                    _ => limit_price_is_valid(side, px, q, stops),
                };
                if !ok_ceny {
                    r.odmowa_poziomu(&mut self.bilans_decyzji);
                    return;
                }
                if let Some(s) = sl {
                    if !pending_sl_is_valid(side, s, px, stops) {
                        r.odmowa_poziomu(&mut self.bilans_decyzji);
                        return;
                    }
                }
                if let Some(t) = tp {
                    if !pending_tp_is_valid(side, t, px, stops) {
                        r.odmowa_poziomu(&mut self.bilans_decyzji);
                        return;
                    }
                }
                let Some(vol)=self.profit_budget_volume(cfg,b,side,px,sl,vol,r) else{return;};
                match b.place_pending(PendingReq {
                    kind,
                    volume: vol,
                    price: px,
                    sl,
                    tp,
                    basket: None,
                    level,
                    is_toucher: false,
                    is_topup: false,
                    comment: opis.to_string(),
                }) {
                    Ok(_) => r.ok += 1,
                    Err(_) => r.odmowa_brokera(&mut self.bilans, &mut self.bilans_decyzji),
                }
            }
        }
    }

    // ----------------------------------------------------------------- raport

    /// Jedno zdanie o tym, co beta zrobiła w tym przebiegu. Wołane samo
    /// z [`Drop`], żeby liczby dało się odczytać bez dotykania `runner.rs`.
    pub fn raport_bety(&self) -> Option<String> {
        let k = self.beta()?;
        let bd = self.bilans_decyzji;
        let mut akcje: Vec<String> = Vec::new();
        for a in KodAkcji::WSZYSTKIE {
            let l = bd.akcje[a.idx()];
            if l.zapadla == 0 {
                continue;
            }
            akcje.push(format!(
                "{}: zapadla={} wykonana={} czesciowa={} bez_pracy={} odm_poziomu={} odm_brokera={} obserwacja={}",
                a.kod(),
                l.zapadla,
                l.wykonana,
                l.czesciowa,
                l.bez_pracy,
                l.odmowa_poziomu,
                l.odmowa_brokera,
                l.obserwacja,
            ));
        }
        let kody: Vec<String> = KodPominieciaBety::WSZYSTKIE
            .iter()
            .filter(|c| bd.ile(**c) > 0)
            .map(|c| format!("{}={}", c.kod(), bd.ile(*c)))
            .collect();
        let powody: Vec<String> = (0..LICZBA_POWODOW)
            .filter(|i| bd.powody[*i] > 0)
            .map(|i| {
                let p = PowodAkcji::z_idx(i);
                let nazwa = match p {
                    PowodAkcji::Taktyka(n) if !k.nazwa_powodu(n).is_empty() => {
                        k.nazwa_powodu(n).to_string()
                    }
                    inny => inny.kod().to_string(),
                };
                format!("{nazwa}={}", bd.powody[i])
            })
            .collect();
        let mut t = String::new();
        t.push_str(&format!(
            "[EA-BETA {}] wariant={} kadencja_s={} tylko_obserwuj={} | pulsy_petli={} | koszyki: rozpatrzone={} obsluzone={} pominiete={} domyka_sie={}\n",
            if k.format.is_empty() { "-" } else { k.format.as_str() },
            k.wariant.kod(),
            k.kadencja_s,
            k.tylko_obserwuj,
            self.beta_pulsy,
            bd.rozpatrzone,
            bd.obsluzone,
            bd.pominiete,
            bd.domyka_sie(),
        ));
        t.push_str(&format!(
            "[EA-BETA] interwencje={} zlecen_wyslanych={} zlecen_odrzuconych={} | kody: {}\n",
            bd.interwencji(),
            bd.zlecen_wyslanych,
            bd.zlecen_odrzuconych,
            if kody.is_empty() {
                "brak".to_string()
            } else {
                kody.join(" ")
            },
        ));
        t.push_str(&format!(
            "[EA-BETA] powody: {}\n",
            if powody.is_empty() {
                "brak".to_string()
            } else {
                powody.join(" ")
            },
        ));
        for a in akcje {
            t.push_str(&format!("[EA-BETA] {a}\n"));
        }
        t.push_str(&format!(
            "[EA-BETA] dziennik decyzji: {} wpisow (sufit {})  (liczniki opisuja ZYCIE TEGO SILNIKA — przy --daily-reset i drabince silnik jest wymieniany)",
            self.dziennik_decyzji.len(),
            k.dziennik_max,
        ));
        Some(t)
    }
}

// ============================================================================
//  SKŁADANIE WIDOKU DECYZYJNEGO
// ============================================================================

/// MIGAWKA JEDNEJ POZYCJI. Wydzielona, żeby test i pętla liczyły wynik
/// pozycji TĄ SAMĄ arytmetyką — dwa miejsca to dwie okazje do rozjazdu.
pub fn migawka_pozycji(p: &crate::types::Position, q: &Quote, ts: Ts) -> MigawkaPozycji {
    let wynik_pts = p.profit_pts(q);
    MigawkaPozycji {
        ticket: p.ticket,
        koszyk: p.basket.unwrap_or(0),
        side: p.side,
        volume: p.volume,
        open_price: p.open_price,
        open_ts: p.open_ts,
        sl: p.sl,
        tp: p.tp,
        level: p.level,
        frozen: p.frozen,
        is_runner: p.is_runner,
        is_toucher: p.is_toucher,
        wynik_usd: wynik_pts * XAU_CONTRACT * p.volume,
        wynik_pts,
        szczyt_pts: p.peak_pts,
        wiek_min: if p.open_ts > 0 {
            (ts - p.open_ts) as f64 / 60_000.0
        } else {
            0.0
        },
    }
}

/// SKŁADANIE [`SytuacjaKoszyka`] — czysta funkcja, bez brokera i bez `self`.
///
/// Wydzielona z pętli z dwóch powodów. Po pierwsze da się ją sprawdzić testem
/// bez prowadzenia całego pulsu. Po drugie — i ważniejsze — jest JEDNYM
/// miejscem, w którym widać, skąd bierze się każda liczba, na której autorzy
/// wariantów będą decydować. Druga definicja „wyniku koszyka" rozjechałaby się
/// o parę centów dokładnie wtedy, gdy ktoś porównałby decyzję z dziennikiem.
///
/// `moje` musi zawierać **wyłącznie** pozycje tego koszyka.
pub fn zbierz_sytuacje<'a>(
    bk: &'a Basket,
    moje: &'a [MigawkaPozycji],
    pam: PamiecKoszyka,
    q: &Quote,
    w: WektorStanu,
    stan_ea: EaStan,
    ts: Ts,
) -> SytuacjaKoszyka<'a> {
    let mut wolumen = 0.0;
    let mut wazona = 0.0;
    let mut otwarty = 0.0;
    let mut ryzyko = 0.0;
    // Koszyk bez SL nie ma MIERZALNEGO ryzyka i zwraca `None`, nie zero —
    // to jest to samo rozróżnienie, które niesie `ryzyko_planowane`.
    let mut ma_sl = bk.sl.is_some();
    let mut zamrozonych = 0usize;
    for m in moje {
        wolumen += m.volume;
        wazona += m.open_price * m.volume;
        otwarty += m.wynik_usd;
        if m.frozen {
            zamrozonych += 1;
        }
        match m.sl.or(bk.sl) {
            Some(s) => ryzyko += ryzyko_pozycji(m.open_price, s, m.volume),
            None => ma_sl = false,
        }
    }
    let mut zywe = 0u32;
    let mut wypelnione = 0u32;
    for g in &bk.levels {
        if g.filled {
            wypelnione += 1;
        } else if !g.cancelled {
            zywe += 1;
        }
    }
    let znak = bk.side.sign();
    let wyj = q.exit(bk.side);
    let nastepny_cel = bk.tps.get(bk.tp_stage).copied();
    SytuacjaKoszyka {
        koszyk: bk.id,
        format: bk.source_name.as_str(),
        side: bk.side,
        is_limit: bk.is_limit,
        is_stop: bk.is_stop,
        zone_lo: bk.zone_lo,
        zone_hi: bk.zone_hi,
        entry_lo: bk.entry_lo,
        entry_hi: bk.entry_hi,
        sl: bk.sl,
        tps: &bk.tps,
        tp_open: bk.tp_open,
        wiek_min: if bk.created_ts > 0 {
            (ts - bk.created_ts) as f64 / 60_000.0
        } else {
            0.0
        },
        etap: EtapKoszyka::z_koszyka(bk),
        stan: bk.state,
        tp_stage: bk.tp_stage,
        plan_wykonany_do: bk.plan_wykonany_do,
        secured: bk.secured,
        secured_by_rule: bk.secured_by_rule,
        bylo_be: bk.be_ts > 0,
        be_ts: bk.be_ts,
        zone_touched: bk.zone_touched,
        reentries: bk.reentries,
        rearms: bk.rearms,
        fast_addons: bk.fast_addons,
        pyramided: bk.pyramided,
        szczeble_zywe: zywe,
        szczeble_wypelnione: wypelnione,
        zlecenia: &bk.pendings,
        pozycje: moje,
        wolumen,
        srednia_cena: if wolumen > 1e-12 {
            wazona / wolumen
        } else {
            0.0
        },
        otwarty_usd: otwarty,
        zrealizowany_usd: bk.realized,
        laczny_usd: otwarty + bk.realized,
        ryzyko_usd: if ma_sl && !moje.is_empty() {
            Some(ryzyko)
        } else {
            None
        },
        zamrozonych,
        q: *q,
        spread: q.spread(),
        cena_wejscia: q.entry(bk.side),
        cena_wyjscia: wyj,
        // ZASIĘG mierzony od chwili, w której pętla zobaczyła koszyk PIERWSZY
        // RAZ — a nie od `created_ts`, bo po restarcie koszyk jest przejęty,
        // nie zawiązany, i ceny sprzed restartu warstwa po prostu nie widziała.
        zasieg_za: ((if znak > 0.0 {
            pam.cena_max
        } else {
            pam.cena_min
        }) - pam.cena_pierwsza)
            * znak,
        zasieg_przeciw: (pam.cena_pierwsza
            - (if znak > 0.0 {
                pam.cena_min
            } else {
                pam.cena_max
            }))
            * znak,
        dystans_do_sl: bk.sl.map(|s| (wyj - s) * znak),
        nastepny_cel,
        dystans_do_celu: nastepny_cel.map(|c| (c - wyj) * znak),
        pamiec: pam,
        w,
        stan_ea,
        ts,
    }
}

// ============================================================================
//  POMOCNICY EGZEKUTORA
// ============================================================================

/// RACHUBA JEDNEJ AKCJI: ile biletów przeszło, ile odbiło i dlaczego.
///
/// Osobna struktura, bo jedna akcja bywa wielobiletowa (koszyk ma kilkanaście
/// pozycji) i wynik „wykonana" musi znaczyć co innego niż „część przeszła".
/// Bez tego rozróżnienia dziennik pokazywałby sukces tam, gdzie połowa
/// wolumenu została w rynku.
#[derive(Debug, Default, Clone, Copy)]
struct Rachuba {
    ok: u64,
    blad: u64,
    poziom: u64,
    pusto: usize,
}

impl Rachuba {
    /// Odmowa brokera idzie do OBU ksiąg: `BilansPulsu` (kontrakt domu N18)
    /// i `BilansDecyzji` (księga bety).
    #[inline]
    fn odmowa_brokera(&mut self, bp: &mut BilansPulsu, bd: &mut BilansDecyzji) {
        self.blad += 1;
        bp.pomin(KodPominiecia::OdmowaBrokera);
        bd.odnotuj(KodPominieciaBety::OdmowaBrokera);
    }

    /// Poziom odrzucony przez NASZ walidator — do brokera nie poszło nic,
    /// więc `BilansPulsu` się tym nie zajmuje.
    #[inline]
    fn odmowa_poziomu(&mut self, bd: &mut BilansDecyzji) {
        self.poziom += 1;
        bd.odnotuj(KodPominieciaBety::PoziomOdrzucony);
    }

    fn wynik(&self) -> WynikAkcji {
        if self.ok > 0 {
            if self.blad > 0 || self.poziom > 0 || self.pusto > 0 {
                WynikAkcji::CzesciowoWykonana
            } else {
                WynikAkcji::Wykonana
            }
        } else if self.blad > 0 {
            WynikAkcji::OdmowaBrokera
        } else if self.poziom > 0 {
            WynikAkcji::OdmowaPoziomu
        } else {
            WynikAkcji::BezPracy
        }
    }
}

fn wybierz_bilety(moje: &[MigawkaPozycji], w: WyborBiletow) -> Vec<Ticket> {
    let zywe = moje.iter().filter(|m| !m.frozen);
    match w {
        WyborBiletow::Wszystkie => zywe.map(|m| m.ticket).collect(),
        WyborBiletow::Runner => zywe.filter(|m| m.is_runner).map(|m| m.ticket).collect(),
        WyborBiletow::Jeden(t) => {
            if moje.iter().any(|m| m.ticket == t && !m.frozen) {
                vec![t]
            } else {
                Vec::new()
            }
        }
        WyborBiletow::Najglebszy => wybierz_jeden(moje, |a, b| {
            (a.wynik_pts, a.ticket) < (b.wynik_pts, b.ticket)
        }),
        WyborBiletow::Najplytszy => wybierz_jeden(moje, |a, b| {
            (a.wynik_pts, a.ticket) > (b.wynik_pts, b.ticket)
        }),
        WyborBiletow::Najstarszy => {
            wybierz_jeden(moje, |a, b| (a.open_ts, a.ticket) < (b.open_ts, b.ticket))
        }
        WyborBiletow::Najmlodszy => {
            wybierz_jeden(moje, |a, b| (a.open_ts, a.ticket) > (b.open_ts, b.ticket))
        }
        WyborBiletow::NajgorszeN(n) => {
            let mut v: Vec<&MigawkaPozycji> = zywe.collect();
            v.sort_by(|a, b| {
                a.wynik_pts
                    .partial_cmp(&b.wynik_pts)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.ticket.cmp(&b.ticket))
            });
            v.into_iter().take(n as usize).map(|m| m.ticket).collect()
        }
    }
}

fn wybierz_jeden(
    moje: &[MigawkaPozycji],
    lepszy: impl Fn(&MigawkaPozycji, &MigawkaPozycji) -> bool,
) -> Vec<Ticket> {
    let mut naj: Option<&MigawkaPozycji> = None;
    for m in moje.iter().filter(|m| !m.frozen) {
        naj = match naj {
            Some(k) if !lepszy(m, k) => Some(k),
            _ => Some(m),
        };
    }
    naj.map(|m| vec![m.ticket]).unwrap_or_default()
}

/// RAPORT NA KONIEC ŻYCIA SILNIKA.
///
/// Rdzeń nie ma dostępu do plików ani do `runner.rs`, a liczby bety muszą dać
/// się odczytać po przebiegu — więc wychodzą tą samą drogą, którą wychodzi
/// diagnostyka EA-CORE: na stderr. Bez bety nie drukuje się ani jedna linijka
/// i to jest część kontraktu zera (parytet obejmuje też brak hałasu).
impl Drop for EaRdzen {
    fn drop(&mut self) {
        let Some(k) = self.beta() else { return };
        if !k.raport || self.beta_pulsy == 0 {
            return;
        }
        if let Some(t) = self.raport_bety() {
            eprintln!("{t}");
        }
    }
}

// ============================================================================
//  TESTY JEDNOSTKOWE SZKIELETU (bez brokera — czysta logika)
// ============================================================================

#[cfg(test)]
mod testy {
    use super::*;

    fn cfg_zera() -> Settings {
        Settings::default()
    }

    /// PODWÓJNE ZERO, część logiczna: preset z samymi zerami stoi w `Neutral`
    /// przy każdym możliwym sygnale — także skrajnym.
    #[test]
    fn zera_trzymaja_neutral_na_zawsze() {
        let c = cfg_zera();
        let r = EaRdzen::default();
        for x in [-1e9, -100.0, -1.0, 0.0, 1.0, 100.0, 1e9] {
            assert_eq!(
                r.ocen_stan(&c, x),
                EaStan::Neutral,
                "przy samych zerach sygnał {x} nie ma prawa ruszyć stanu"
            );
        }
    }

    /// Modulatory przy zerach są neutralne CO DO BITU — to jest druga połowa
    /// podwójnego zera (pierwsza: stan zostaje `Neutral`).
    #[test]
    fn modulatory_przy_zerach_sa_neutralne() {
        let c = cfg_zera();
        let r = EaRdzen::default();
        let m = r.modulatory(&c, 1);
        assert!(m.sa_neutralne(), "modulatory przy zerach: {m:?}");
        assert_eq!(m.jednostki, 1.0);
        assert_eq!(m.trailing, 1.0);
        assert!(m.dokladki);
    }

    /// Histereza dwustronna: wejście i wyjście na RÓŻNYCH progach, więc
    /// sygnał drgający między nimi nie przełącza stanu.
    #[test]
    fn histereza_nie_migocze_miedzy_progami() {
        let mut c = cfg_zera();
        c.ea_state_src = EaStateSrc::FloatPctEquity;
        c.ea_defense_enter = 5.0; // wchodzimy przy −5 %
        c.ea_defense_exit = 2.0; // wychodzimy dopiero przy −2 %
        let mut r = EaRdzen::default();

        assert_eq!(r.ocen_stan(&c, -4.9), EaStan::Neutral);
        r.stan = r.ocen_stan(&c, -5.0);
        assert_eq!(r.stan, EaStan::Obrona, "próg wejścia");
        // −3 % to POWYŻEJ progu wejścia, ale jeszcze poniżej progu wyjścia
        assert_eq!(
            r.ocen_stan(&c, -3.0),
            EaStan::Obrona,
            "strefa histerezy trzyma obronę"
        );
        assert_eq!(r.ocen_stan(&c, -2.0), EaStan::Neutral, "próg wyjścia");
    }

    /// ASYMETRIA: zaciskanie natychmiast, luzowanie po `dwell`.
    ///
    /// To jest ta sama zasada, którą niesie `ea_ladder_scope` i arbiter:
    /// ochrona nie czeka na zegar, luz wymaga potwierdzenia.
    #[test]
    fn zaciskanie_natychmiast_luzowanie_po_dwell() {
        let mut c = cfg_zera();
        c.ea_state_src = EaStateSrc::FloatPctEquity;
        c.ea_defense_enter = 5.0;
        c.ea_defense_exit = 2.0;
        c.ea_state_dwell_s = 60.0;
        let mut r = EaRdzen::default();

        // zaciskanie: bez czekania
        r.przejdz(&c, EaStan::Obrona, -6.0, 1_000, ZrodloPulsu::Tick);
        assert_eq!(
            r.stan,
            EaStan::Obrona,
            "zaciskanie musi działać natychmiast"
        );

        // luzowanie: pierwsza próba tylko zapisuje kandydata
        r.przejdz(&c, EaStan::Neutral, -1.0, 2_000, ZrodloPulsu::Tick);
        assert_eq!(
            r.stan,
            EaStan::Obrona,
            "luz bez przetrzymania nie przechodzi"
        );
        // po 59 s nadal nie
        r.przejdz(&c, EaStan::Neutral, -1.0, 61_000, ZrodloPulsu::Tick);
        assert_eq!(r.stan, EaStan::Obrona, "59 s to za mało przy dwell 60 s");
        // po 60 s przechodzi
        r.przejdz(&c, EaStan::Neutral, -1.0, 62_000, ZrodloPulsu::Tick);
        assert_eq!(
            r.stan,
            EaStan::Neutral,
            "po przetrzymaniu dwell luz wchodzi"
        );
        assert_eq!(
            r.dziennik().len(),
            2,
            "obie zmiany w dzienniku: {:?}",
            r.dziennik()
        );
    }

    /// Zegar mierzy CZASEM ZDARZENIA, nigdy licznikiem. `ea_tick_s = 0`
    /// znaczy „brak własnego zegara", czyli puls z każdym tickiem.
    #[test]
    fn zegar_liczy_czas_a_nie_wywolania() {
        let mut c = cfg_zera();
        let mut r = EaRdzen::default();
        assert!(r.zegar_wybija(&c, 1_000), "ea_tick_s = 0 → puls zawsze");
        r.ostatni_puls_ts = 1_000;
        assert!(
            r.zegar_wybija(&c, 1_001),
            "ea_tick_s = 0 → puls zawsze, także po 1 ms"
        );

        c.ea_tick_s = 5.0;
        assert!(
            !r.zegar_wybija(&c, 4_999),
            "4,999 s to za mało przy kadencji 5 s"
        );
        assert!(r.zegar_wybija(&c, 6_000), "5 s minęło → puls");

        // ZEGAR COFNIĘTY (reset dobowy backtestu, korekta czasu na maszynie)
        // nie ma prawa zamilczeć straży do chwili, w której czas dogoni
        // starą wartość — to jest ta sama awaria, dla której zegar powstał.
        assert!(
            r.zegar_wybija(&c, 500),
            "cofnięty zegar musi dać puls, nie ciszę"
        );
    }

    /// ZAPADKA `NieLuzujWKoszyku`: koszyk zawiązany w Obronie dożywa
    /// w Obronie, choćby portfel wrócił do Neutral. Nowe koszyki dostają już
    /// luźniejsze parametry.
    #[test]
    fn zapadka_nie_luzuje_w_otwartym_koszyku() {
        let c = cfg_zera();
        let mut r = EaRdzen::default();
        r.stemple.push(StempelKoszyka {
            id: 7,
            ts: 0,
            ryzyko_stempla: 10.0,
            stan: EaStan::Obrona,
            ryzyko_szczyt: 10.0,
            etap: EtapKoszyka::Pracuje,
            zapadka_zgloszona: false,
        });
        r.stan = EaStan::Neutral; // portfel się uspokoił

        assert_eq!(
            r.stan_efektywny(&c, 7),
            EaStan::Obrona,
            "koszyk ostemplowany w Obronie nie ma prawa dostać luzu"
        );
        assert_eq!(
            r.stan_efektywny(&c, 99),
            EaStan::Neutral,
            "koszyk bez stempla (nowy) bierze stan bieżący"
        );

        // wariant `Swobodny` istnieje wyłącznie po to, żeby dało się zmierzyć
        // KOSZT zapadki — i musi działać dokładnie odwrotnie
        let mut c2 = cfg_zera();
        c2.ea_state_ratchet = EaRatchet::Swobodny;
        assert_eq!(r.stan_efektywny(&c2, 7), EaStan::Neutral);
    }

    /// N18: bilans domyka się po każdej operacji, a `zgubione_bez_sladu`
    /// zostaje zerem.
    #[test]
    fn bilans_sie_domyka() {
        let mut bl = BilansPulsu::default();
        assert!(bl.domyka_sie());
        bl.obsluz();
        bl.pomin(KodPominiecia::Sierota);
        bl.pomin(KodPominiecia::Zamrozona);
        assert!(bl.domyka_sie(), "{bl:?}");
        assert_eq!(bl.zgubione_bez_sladu(), 0);
        assert_eq!(bl.rozpatrzone, 3);
        assert_eq!(bl.ile(KodPominiecia::Sierota), 1);
        // `odnotuj` NIE zmienia bilansu — to obserwacja o bycie już policzonym
        bl.odnotuj(KodPominiecia::ZapadkaZlamana);
        assert!(
            bl.domyka_sie(),
            "odnotowanie nie ma prawa rozjechać bilansu"
        );
        assert_eq!(bl.ile(KodPominiecia::ZapadkaZlamana), 1);
    }

    /// Każdy kod pominięcia ma unikalny indeks — inaczej dwa różne powody
    /// zlewałyby się w jeden licznik i lejek kłamałby po cichu.
    #[test]
    fn kody_pominiec_maja_rozlaczne_indeksy() {
        let mut v: Vec<usize> = KodPominiecia::WSZYSTKIE.iter().map(|k| k.idx()).collect();
        let ile = v.len();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), ile, "kody pominięć dzielą indeks");
        assert_eq!(ile, 9, "tablica `kody` w BilansPulsu ma dokładnie 9 pól");
    }

    /// Maszyna stanu koszyka jest MONOTONICZNA w odczycie: kolejność wariantów
    /// odpowiada kolejności życia, więc `max` na etapach ma sens — a zatrzask
    /// zaczyna się dokładnie na `Pracuje`.
    #[test]
    fn etapy_koszyka_sa_uporzadkowane() {
        use EtapKoszyka::*;
        let kolejnosc = [Planowany, Uzbrojony, Pracuje, Zabezpieczony, Zamkniety];
        for okno in kolejnosc.windows(2) {
            assert!(
                okno[0] < okno[1],
                "{:?} musi poprzedzać {:?}",
                okno[0],
                okno[1]
            );
        }
        assert_eq!(EtapKoszyka::PIERWSZY_ZATRZASK, Pracuje);
        assert!(
            Uzbrojony < EtapKoszyka::PIERWSZY_ZATRZASK,
            "dolne szczeble wolno cofnąć"
        );
    }

    /// Odczyt etapu z migawki — po jednym przypadku na szczebel, plus dwa
    /// przypadki graniczne, które w poprzedniej wersji dawały FAŁSZYWĄ
    /// regresję: koszyk między transzami (zero biletów, `had_positions`)
    /// i koszyk zabezpieczony, którego silnik przestawił z powrotem na
    /// `Working` (cztery takie ścieżki są w `engine.rs`).
    #[test]
    fn odczyt_etapu_z_migawki() {
        use EtapKoszyka::*;
        // Koszyk składamy przez serde, a nie literałem struktury: `Basket` ma
        // ponad trzydzieści pól, z czego większość z `#[serde(default)]`,
        // więc literał byłby ścianą zer, którą trzeba by poprawiać przy
        // każdym nowym polu — i pierwszy, kto to zrobi, zepsuje test, żeby
        // się skompilował.
        let baza = |f: &dyn Fn(&mut Basket)| {
            let mut bk: Basket = serde_json::from_str(
                r#"{"id":1,"source":{"chat_id":1,"topic_id":null},"source_name":"T",
                    "msg_id":1,"side":"Buy","is_limit":true,"entry_lo":0.0,"entry_hi":0.0,
                    "zone_lo":0.0,"zone_hi":0.0,"sl":null,"tps":[],"tp_stage":0,
                    "created_ts":0,"state":"Armed","tickets":[],"pendings":[],
                    "realized":0.0,"events":[]}"#,
            )
            .expect("migawka koszyka do testu");
            f(&mut bk);
            EtapKoszyka::z_koszyka(&bk)
        };

        assert_eq!(baza(&|_| {}), Planowany, "nic nie ma → Planowany");
        assert_eq!(baza(&|b| b.pendings.push(1)), Uzbrojony);
        assert_eq!(baza(&|b| b.tickets.push(1)), Pracuje);
        assert_eq!(
            baza(&|b| b.had_positions = true),
            Pracuje,
            "koszyk MIĘDZY TRANSZAMI (zero biletów) nie ma prawa spaść z `Pracuje`"
        );
        assert_eq!(baza(&|b| b.secured = true), Zabezpieczony);
        assert_eq!(
            baza(&|b| {
                b.secured = true;
                b.state = BasketState::Working;
            }),
            Zabezpieczony,
            "przestawienie stanu na `Working` po RISK FREE nie cofa zatrzasku"
        );
        assert_eq!(
            baza(&|b| {
                b.secured = true;
                b.state = BasketState::Done;
            }),
            Zamkniety,
            "`Done` wygrywa ze wszystkim"
        );
    }

    // ---------------------------------------------------------------- RODZINA A

    /// KONTRAKT ZERA osi A1/A3/A4 w jednym zdaniu: sufit obojętny nie ścina
    /// ani jednej jednostki, niezależnie od wielkości planu.
    #[test]
    fn sufit_obojetny_nie_scina_ani_jednej_jednostki() {
        let s = SufitEa::obojetny();
        assert!(s.jest_obojetny());
        for n in [1u32, 2, 5, 8, 12, 100, 1000] {
            assert_eq!(
                s.docelowe_jednostki(n),
                n,
                "sufit obojętny ruszył plan o {n} jednostkach"
            );
        }
    }

    /// NIEZMIENNIK KIERUNKU: rodzina A nie ma prawa PODNIEŚĆ ekspozycji.
    /// Mnożnik > 1 jest przycinany w konstruktorze, a nie odradzany w opisie.
    #[test]
    fn mnoznik_nigdy_nie_podnosi_ekspozycji() {
        for m in [1.5, 2.0, 10.0, f64::INFINITY] {
            let s = SufitEa::nowy(u32::MAX, m);
            assert!(
                s.mult <= 1.0,
                "mnożnik {m} przeszedł nieprzycięty: {}",
                s.mult
            );
            assert_eq!(
                s.docelowe_jednostki(8),
                8,
                "mnożnik {m} podniósł liczbę jednostek"
            );
        }
        // NaN nie ma prawa zamienić się w zero jednostek ani w nieskończoność
        let s = SufitEa::nowy(u32::MAX, f64::NAN);
        assert_eq!(s.mult, 1.0);
        // wartości ujemne przycinane do zera, ale podłoga jednej jednostki trzyma
        assert_eq!(SufitEa::nowy(u32::MAX, -3.0).docelowe_jednostki(8), 1);
    }

    /// PODŁOGA JEDNEJ JEDNOSTKI — ta sama reguła co `regime_soft_units_mult`:
    /// „siatka o zerowej liczbie szczebli to koszyk, którego nie ma".
    /// Odmowa całego koszyka zapada wyżej i z własnym kodem.
    #[test]
    fn podloga_jednej_jednostki_trzyma_pokrycie() {
        assert_eq!(SufitEa::nowy(u32::MAX, 0.01).docelowe_jednostki(8), 1);
        assert_eq!(
            SufitEa::nowy(0, 1.0).docelowe_jednostki(8),
            1,
            "sufit 0 obsługuje `place_grid`"
        );
        assert_eq!(
            SufitEa::nowy(u32::MAX, 1.0).docelowe_jednostki(0),
            0,
            "pusty plan zostaje pusty"
        );
    }

    #[test]
    fn sufit_absolutny_i_mnoznik_skladaja_sie_w_tej_kolejnosci() {
        // 12 jednostek, A3 = 0,5 -> 6, A1 = 4 -> 4
        assert_eq!(SufitEa::nowy(4, 0.5).docelowe_jednostki(12), 4);
        // 12 jednostek, A3 = 0,5 -> 6, A1 = 9 (nie wiąże) -> 6
        assert_eq!(SufitEa::nowy(9, 0.5).docelowe_jednostki(12), 6);
        // sam A1
        assert_eq!(SufitEa::nowy(3, 1.0).docelowe_jednostki(12), 3);
        // sam A3, zaokrąglenie W DÓŁ (mniej ryzyka przy remisie)
        assert_eq!(SufitEa::nowy(u32::MAX, 0.7).docelowe_jednostki(5), 3);
    }

    /// A4: doba zeruje licznik stopów, ale NIE kasuje miar przebiegu.
    #[test]
    fn stan_dnia_zeruje_licznik_a_nie_miary() {
        let mut s = StanRodzinyA::default();
        assert!(!s.cokolwiek_zrobila());
        s.stopy_dnia = 3;
        s.stopy_dnia_max = 3;
        s.dni_uzbrojone = 1;
        s.weta_a4 = 7;
        assert!(s.dzien_uzbrojony(2));
        assert!(!s.dzien_uzbrojony(4));
        s.nowa_doba();
        assert_eq!(s.stopy_dnia, 0, "nowa doba nie wyzerowała licznika");
        assert_eq!(
            s.stopy_dnia_max, 3,
            "nowa doba skasowała SZCZYT — to miara przebiegu"
        );
        assert_eq!(s.dni_uzbrojone, 1);
        assert!(s.cokolwiek_zrobila(), "weta A4 to jest realna różnica");
        assert!(
            !s.dzien_uzbrojony(2),
            "po nowej dobie stan dnia musi być rozbrojony"
        );
        // prog 0 znaczy „uzbrojony od poczatku doby"
        assert!(s.dzien_uzbrojony(0));
    }

    // ============================================================ BETA EA

    use crate::broker::{BResult, BrokerError};
    use crate::types::{Account, ClosedTrade, PendingOrder, Position};
    use std::cell::Cell;

    /// Atrapa brokera z LICZNIKAMI ODCZYTÓW.
    ///
    /// Liczniki są tu po to, żeby kontrakt zera dało się udowodnić
    /// STRUKTURALNIE, a nie porównaniem wyniku: warstwa wyłączona ma nie
    /// dotknąć rachunku ani kwotowania ani razu. To jest ten sam wzorzec,
    /// którym sprawdza się `margines_pozwala`.
    struct Atrapa {
        q: Quote,
        konto: Account,
        poz: Vec<Position>,
        pend: Vec<PendingOrder>,
        stops: f64,
        odmawiaj: bool,
        n_quote: Cell<u32>,
        n_account: Cell<u32>,
        n_positions: Cell<u32>,
        modyfikacje: Vec<(Ticket, Option<Px>, Option<Px>)>,
        zamkniete: Vec<Ticket>,
        czesciowe: Vec<(Ticket, f64)>,
        zlozone: Vec<f64>,
        volume_spec: crate::volume_contract::VolumeSpec,
    }

    impl Atrapa {
        fn nowa() -> Self {
            Atrapa {
                q: Quote::new(1_000, 4630.00, 4630.20).expect("kwotowanie"),
                konto: Account {
                    balance: 238.88,
                    equity: 238.88,
                    margin: 9.26,
                    free_margin: 229.62,
                    leverage: 500,
                    credit: 0.0,
                },
                poz: Vec::new(),
                pend: Vec::new(),
                stops: 0.20, // stops_level = 20 punktów u PUPrime
                odmawiaj: false,
                n_quote: Cell::new(0),
                n_account: Cell::new(0),
                n_positions: Cell::new(0),
                modyfikacje: Vec::new(),
                zamkniete: Vec::new(),
                czesciowe: Vec::new(),
                zlozone: Vec::new(),
                volume_spec: crate::volume_contract::VolumeSpec {
                    minimum: 0.01, step: 0.01, maximum: 100.0,
                },
            }
        }

        fn z_pozycja(mut self, ticket: Ticket, koszyk: u32, sl: Option<Px>) -> Self {
            self.poz.push(Position {
                ticket,
                side: Side::Buy,
                volume: 0.10,
                open_price: 4628.00,
                open_ts: 500,
                sl,
                tp: None,
                vsl: None,
                basket: Some(koszyk),
                level: 0,
                frozen: false,
                peak_pts: 2.0,
                last_peak_ts: 900,
                is_runner: false,
                is_toucher: false,
                comment: String::new(),
            });
            self
        }
    }

    impl Broker for Atrapa {
        fn volume_min(&self) -> f64 { self.volume_spec.minimum }
        fn volume_step(&self) -> f64 { self.volume_spec.step }
        fn volume_max(&self) -> f64 { self.volume_spec.maximum }
        fn quote(&self) -> Quote {
            self.n_quote.set(self.n_quote.get() + 1);
            self.q
        }
        fn account(&self) -> Account {
            self.n_account.set(self.n_account.get() + 1);
            self.konto
        }
        fn stops_level(&self) -> f64 {
            self.stops
        }
        fn positions(&self) -> &[Position] {
            self.n_positions.set(self.n_positions.get() + 1);
            &self.poz
        }
        fn pendings(&self) -> &[PendingOrder] {
            &self.pend
        }
        fn positions_mut(&mut self) -> &mut Vec<Position> {
            &mut self.poz
        }
        fn pendings_mut(&mut self) -> &mut Vec<PendingOrder> {
            &mut self.pend
        }
        fn open_market(&mut self, r: OrderReq) -> BResult<Ticket> {
            if self.odmawiaj {
                return Err(BrokerError::Rejected);
            }
            self.zlozone.push(r.volume);
            Ok(9_000)
        }
        fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket> {
            if self.odmawiaj {
                return Err(BrokerError::Rejected);
            }
            self.zlozone.push(r.volume);
            Ok(9_001)
        }
        fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()> {
            if self.odmawiaj {
                return Err(BrokerError::InvalidStops);
            }
            self.modyfikacje.push((t, sl, tp));
            if let Some(p) = self.poz.iter_mut().find(|p| p.ticket == t) {
                p.sl = sl;
                p.tp = tp;
            }
            Ok(())
        }
        fn modify_pending(
            &mut self,
            _t: Ticket,
            _price: Px,
            _sl: Option<Px>,
            _tp: Option<Px>,
        ) -> BResult<()> {
            Ok(())
        }
        fn close_position(&mut self, t: Ticket, _r: CloseReason) -> BResult<f64> {
            if self.odmawiaj {
                return Err(BrokerError::Rejected);
            }
            self.zamkniete.push(t);
            self.poz.retain(|p| p.ticket != t);
            Ok(0.0)
        }
        fn close_partial(&mut self, t: Ticket, volume: f64, _r: CloseReason) -> BResult<f64> {
            if self.odmawiaj {
                return Err(BrokerError::InvalidVolume);
            }
            self.czesciowe.push((t, volume));
            if let Some(p) = self.poz.iter_mut().find(|p| p.ticket == t) {
                p.volume -= volume;
            }
            Ok(0.0)
        }
        fn cancel_pending(&mut self, _t: Ticket) -> BResult<()> {
            if self.odmawiaj {
                return Err(BrokerError::NoSuchTicket);
            }
            Ok(())
        }
        fn drain_closed(&mut self) -> Vec<ClosedTrade> {
            Vec::new()
        }
    }

    /// Koszyk do testów — składany przez serde, tak samo jak w
    /// `odczyt_etapu_z_migawki` i z tego samego powodu.
    fn koszyk_testowy(f: &dyn Fn(&mut Basket)) -> Basket {
        let mut bk: Basket = serde_json::from_str(
            r#"{"id":7,"source":{"chat_id":1,"topic_id":null},"source_name":"Synergy",
                "msg_id":1,"side":"Buy","is_limit":true,"entry_lo":4625.0,"entry_hi":4630.0,
                "zone_lo":4625.0,"zone_hi":4630.0,"sl":4620.0,"tps":[4635.0,4640.0],
                "tp_stage":0,"created_ts":0,"state":"Working","tickets":[1],"pendings":[],
                "realized":0.0,"events":[]}"#,
        )
        .expect("migawka koszyka do testu");
        bk.had_positions = true;
        f(&mut bk);
        bk
    }

    fn konfig(w: WariantBety) -> KonfigBety {
        KonfigBety {
            wariant: w,
            raport: false,
            ..KonfigBety::default()
        }
    }

    fn volume_contract_ea_entry(c: &Settings, b: &mut Atrapa, volume: f64,
        pending: bool, reentry: bool) -> WynikAkcji {
        let limit = pending.then_some(4627.0);
        let akcja = if reentry {
            AkcjaEa::WejdzPonownie { side: Side::Buy, volume, limit, sl: None, tp: None }
        } else {
            AkcjaEa::DolozPozycje { side: Side::Buy, volume, limit, sl: None, tp: None, level: 0 }
        };
        let d = Decyzja { ts: 1_000, koszyk: 7, akcja, powod: PowodAkcji::Szkielet,
            bilety: vec![], zlecenia: vec![] };
        let q = b.q;
        EaRdzen::default().wykonaj_akcje(&konfig(WariantBety::Z), c, b, &q, 0.20, &d)
    }

    #[test]
    fn publisher_withdrawal_blocks_ea_addons_but_keeps_existing_position_management() {
        let bk = koszyk_testowy(&|bk| {
            bk.entry_edit_state = Some(Box::new(crate::types::EntryEditState {
                schema_version:1, revision:1, source:None, applied_ts:1,
                cancelled_by_source_ts:Some(2), review:None,
            }));
        });
        let add = AkcjaEa::DolozPozycje { side:Side::Buy, volume:0.01,
            limit:None, sl:None, tp:None, level:0 };
        let reenter = AkcjaEa::WejdzPonownie { side:Side::Buy, volume:0.01,
            limit:None, sl:None, tp:None };
        assert!(!source_allows_action(&bk, &add));
        assert!(!source_allows_action(&bk, &reenter));
        assert!(source_allows_action(&bk, &AkcjaEa::PrzesunStop(4000.0)));
        assert!(source_allows_action(&bk, &AkcjaEa::ZamknijCzesc(0.5)));
        assert!(source_allows_action(&bk, &AkcjaEa::AnulujOczekujace));
        let fresh = koszyk_testowy(&|_| {});
        assert!(source_allows_action(&fresh, &add));
        assert!(source_allows_action(&fresh, &reenter));
    }

    #[test]
    fn cost_net_ea_market_pending_addon_reentry_reject_unsupported_but_off_unchanged() {
        for enabled in [false,true] {for pending in [false,true] {for reentry in [false,true] {
            let mut c=cfg_zera();c.closed_profit_net_costs=enabled;c.basket_realized_broker_only=true;
            let mut b=Atrapa::nowa();
            let result=volume_contract_ea_entry(&c,&mut b,0.02,pending,reentry);
            assert_eq!(result,if enabled{WynikAkcji::OdmowaBrokera}else{WynikAkcji::Wykonana});
            assert_eq!(b.zlozone.len(),if enabled{0}else{1});
            assert!(b.zamkniete.is_empty()&&b.czesciowe.is_empty());
        }}}
    }

    #[test]
    fn volume_contract_ea_market_pending_addon_reentry_respect_three_steps() {
        for (step, requested, expected) in [(0.001, 0.0079, 0.007), (0.01, 0.019, 0.01), (0.1, 0.29, 0.2)] {
            for pending in [false, true] { for reentry in [false, true] {
                let mut c = cfg_zera(); c.order_volume_contract_v2 = true;
                c.lot_min = step; c.lot_max = 0.0;
                let mut b = Atrapa::nowa(); b.volume_spec.minimum = step; b.volume_spec.step = step;
                assert_eq!(volume_contract_ea_entry(&c, &mut b, requested, pending, reentry), WynikAkcji::Wykonana);
                assert_eq!(b.zlozone.len(), 1);
                assert!((b.zlozone[0] - expected).abs() < 1e-12);
            }}
        }
    }

    #[test]
    fn volume_contract_ea_static_and_dynamic_cap_apply_after_tactical_size() {
        for pending in [false, true] {
            let mut c = cfg_zera(); c.order_volume_contract_v2 = true;
            c.lot_min = 0.01; c.lot_max = 0.015;
            let mut b = Atrapa::nowa();
            assert_eq!(volume_contract_ea_entry(&c, &mut b, 0.7, pending, false), WynikAkcji::Wykonana);
            assert_eq!(b.zlozone, vec![0.01]);
            c.lot_max = 0.0; c.lot_max_z_salda = 10_000.0;
            c.lot_base = crate::settings::PodstawaLota::Balance;
            for (balance, expected) in [(199.0, 0.01), (200.0, 0.02), (300.0, 0.03)] {
                let mut b = Atrapa::nowa(); b.konto.balance = balance;
                assert_eq!(volume_contract_ea_entry(&c, &mut b, 0.7, pending, true), WynikAkcji::Wykonana);
                assert!((b.zlozone[0] - expected).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn volume_contract_ea_minimum_never_promotes_residual_and_inversion_is_rejected() {
        for pending in [false, true] { for reentry in [false, true] {
            let mut c = cfg_zera(); c.order_volume_contract_v2 = true;
            c.lot_min = 0.03; c.lot_max = 0.0;
            let mut b = Atrapa::nowa();
            assert_eq!(volume_contract_ea_entry(&c, &mut b, 0.01, pending, reentry), WynikAkcji::OdmowaBrokera);
            assert!(b.zlozone.is_empty());
            c.lot_min = 0.10; c.lot_max = 0.05;
            assert_eq!(volume_contract_ea_entry(&c, &mut b, 0.20, pending, reentry), WynikAkcji::OdmowaBrokera);
            assert!(b.zlozone.is_empty());
        }}
    }

    #[test]
    fn volume_contract_ea_unknown_broker_max_fails_on_and_preserves_off() {
        for pending in [false, true] {
            let mut c = cfg_zera(); c.lot_min = 0.01; c.lot_max = 0.015;
            let mut b = Atrapa::nowa(); b.volume_spec.maximum = f64::NAN;
            // Legacy ignores cap/unknown metadata and promotes small requests.
            assert_eq!(volume_contract_ea_entry(&c, &mut b, 0.0079, pending, false), WynikAkcji::Wykonana);
            assert_eq!(b.zlozone, vec![0.01]);
            assert_eq!(b.n_account.get(), 0, "OFF must not add account reads");
            c.order_volume_contract_v2 = true;
            assert_eq!(volume_contract_ea_entry(&c, &mut b, 0.02, pending, false), WynikAkcji::OdmowaBrokera);
            assert_eq!(b.zlozone.len(), 1);
        }
    }

    #[test]
    fn confirmed_exit_blocks_ea_on_planning_and_sl_edits_with_off_rollback() {
        for enabled in [false, true] {
            let mut c = cfg_zera();
            c.ea_enabled = true;
            c.ea_dozor_sl = true;
            c.confirmed_exit_retry = enabled;
            let mut b = Atrapa::nowa().z_pozycja(1, 7, None);
            let mut ks = vec![koszyk_testowy(&|bk| {
                bk.pending_exit = Some(crate::types::PendingBasketExit {
                    reason: CloseReason::Tp, last_attempt_ts: 1_000,
                });
            })];
            let mut r = EaRdzen::default();
            r.wstrzyknij_bete(konfig(WariantBety::Z));
            assert!(r.puls(&c, &mut ks, &mut b, 2_000, ZrodloPulsu::Tick));
            assert_eq!(r.beta_pulsy, 1, "EA must genuinely be enabled for this regression");
            assert_eq!(r.bilans_decyzji().rozpatrzone, if enabled { 0 } else { 1 });
            assert_eq!(b.modyfikacje.len(), if enabled { 0 } else { 1 });
            assert!(b.zlozone.is_empty() && b.zamkniete.is_empty() && b.czesciowe.is_empty());
            assert!(ks[0].pending_exit.is_some(), "EA cannot clear another owner's exit intent");
        }
    }

    /// **KONTRAKT ZERA, DOWÓD STRUKTURALNY.**
    ///
    /// Bez konfiguracji bety pętla decyzyjna wychodzi PRZED odczytem
    /// kwotowania — a kwotowania nie czyta żaden inny krok pulsu, więc licznik
    /// `quote()` jest tu miarą czystą: 0 znaczy „beta nie tknęła brokera".
    #[test]
    fn beta_wylaczona_nie_dotyka_brokera() {
        let c = cfg_zera();
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let mut ks = vec![koszyk_testowy(&|_| {})];

        let mut r = EaRdzen::default();
        r.wylacz_bete();
        for i in 0..5 {
            assert!(r.puls(&c, &mut ks, &mut b, 1_000 + i, ZrodloPulsu::Tick));
        }
        assert_eq!(b.n_quote.get(), 0, "beta wyłączona ODCZYTAŁA kwotowanie");
        assert_eq!(
            r.beta_pulsy, 0,
            "pętla decyzyjna przebiegła mimo braku konfiguracji"
        );
        assert_eq!(
            r.bilans_decyzji().rozpatrzone,
            0,
            "beta wyłączona coś policzyła"
        );
        assert!(b.modyfikacje.is_empty() && b.zamkniete.is_empty());
        let odczyty_konta_bez_bety = b.n_account.get();

        // ta sama droga z betą WŁĄCZONĄ musi ruszyć licznik — inaczej test
        // przechodziłby także wtedy, gdyby pętli w ogóle nie było
        let mut b2 = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let mut r2 = EaRdzen::default();
        r2.wstrzyknij_bete(konfig(WariantBety::Z));
        for i in 0..5 {
            assert!(r2.puls(&c, &mut ks, &mut b2, 1_000 + i, ZrodloPulsu::Tick));
        }
        assert_eq!(
            b2.n_quote.get(),
            5,
            "pętla decyzyjna nie czytała kwotowania"
        );
        assert_eq!(r2.beta_pulsy, 5);
        assert_eq!(
            b2.n_account.get(),
            odczyty_konta_bez_bety,
            "beta dołożyła ODCZYT RACHUNKU — wektor stanu ma być czytany RAZ na puls"
        );
        // …i nadal ani jednej modyfikacji, bo akcją próbną jest `Trzymaj`
        assert!(b2.modyfikacje.is_empty() && b2.zamkniete.is_empty() && b2.czesciowe.is_empty());
    }

    /// Pętla przebiega, sytuacja się składa, księgowość się domyka —
    /// i wszystko to przy akcji próbnej `Trzymaj`, czyli bez ruszania centa.
    #[test]
    fn petla_decyzyjna_liczy_i_domyka_bilans() {
        let c = cfg_zera();
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let mut ks = vec![koszyk_testowy(&|_| {})];
        let mut r = EaRdzen::default();
        r.wstrzyknij_bete(konfig(WariantBety::Z));

        r.puls(&c, &mut ks, &mut b, 2_000, ZrodloPulsu::Tick);
        let bd = r.bilans_decyzji();
        assert_eq!(
            bd.rozpatrzone, 1,
            "jeden żywy koszyk = jeden byt rozpatrzony"
        );
        assert_eq!(bd.obsluzone, 1);
        assert_eq!(bd.pominiete, 0);
        assert!(bd.domyka_sie(), "{bd:?}");
        let l = bd.akcja(KodAkcji::Trzymaj);
        assert_eq!(l.zapadla, 1, "akcja próbna nie zapadła");
        assert_eq!(l.bez_pracy, 1, "`Trzymaj` ma się rozliczyć jako BEZ_PRACY");
        assert!(l.domyka_sie(), "{l:?}");
        assert_eq!(bd.powod(PowodAkcji::Brak), 1);
        assert_eq!(
            bd.powod(PowodAkcji::Szkielet),
            0,
            "wariant Z nie jest już szkieletem"
        );
        assert_eq!(
            bd.interwencji(),
            0,
            "`Trzymaj` nie ma prawa ruszyć rachunku"
        );
        assert!(
            r.dziennik_decyzji().is_empty(),
            "`Trzymaj` nie wchodzi do dziennika"
        );

        // pamięć EA powstała i pamięta
        let pam = r.pamiec_koszyka(7).copied().expect("pamięć koszyka");
        assert_eq!(pam.pierwszy_ts, 2_000);
        assert_eq!(pam.akcje, 0);
        assert!((pam.cena_pierwsza - b.q.mid()).abs() < 1e-9);
    }

    /// Koszyk zamknięty (`Done`) nie jest bytem do rozpatrzenia, a pamięć po
    /// nim znika — inaczej wektor pamięci rósłby przez cały przebieg.
    #[test]
    fn koszyk_zamkniety_wypada_z_petli_razem_z_pamiecia() {
        let c = cfg_zera();
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let mut ks = vec![koszyk_testowy(&|_| {})];
        let mut r = EaRdzen::default();
        r.wstrzyknij_bete(konfig(WariantBety::Z));
        r.puls(&c, &mut ks, &mut b, 2_000, ZrodloPulsu::Tick);
        assert!(r.pamiec_koszyka(7).is_some());

        ks[0].state = BasketState::Done;
        r.puls(&c, &mut ks, &mut b, 3_000, ZrodloPulsu::Tick);
        assert_eq!(
            r.bilans_decyzji().rozpatrzone,
            1,
            "zamknięty koszyk został rozpatrzony"
        );
        assert!(
            r.pamiec_koszyka(7).is_none(),
            "pamięć po zamkniętym koszyku została"
        );
    }

    /// Zegar bety mierzy CZASEM ZDARZENIA i budzi się przy zegarze cofniętym —
    /// ta sama zasada, co [`EaRdzen::zegar_wybija`], bo to ta sama awaria.
    #[test]
    fn zegar_bety_liczy_czas_a_nie_wywolania() {
        let mut k = konfig(WariantBety::Z);
        let mut r = EaRdzen::default();
        assert!(
            r.zegar_bety(&k, 1_000),
            "kadencja 0 = decyzja na każdym pulsie"
        );
        k.kadencja_s = 5.0;
        assert!(r.zegar_bety(&k, 1_000), "pierwszy raz zawsze przechodzi");
        r.beta_ostatni_ts = 1_000;
        assert!(
            !r.zegar_bety(&k, 5_999),
            "4,999 s to za mało przy kadencji 5 s"
        );
        assert!(r.zegar_bety(&k, 6_000), "5 s minęło → decyzja");
        assert!(
            r.zegar_bety(&k, 500),
            "cofnięty zegar musi dać puls, nie ciszę"
        );
    }

    /// Kadencja bety jest NIEZALEŻNA od kadencji pulsu: warstwa może patrzeć
    /// na każdym ticku, a decydować raz na sekundę.
    #[test]
    fn kadencja_bety_przerzedza_decyzje_a_nie_obserwacje() {
        let c = cfg_zera();
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let mut ks = vec![koszyk_testowy(&|_| {})];
        let mut r = EaRdzen::default();
        r.wstrzyknij_bete(KonfigBety {
            wariant: WariantBety::Z,
            kadencja_s: 1.0,
            raport: false,
            ..KonfigBety::default()
        });
        for i in 0..21 {
            r.puls(&c, &mut ks, &mut b, 10_000 + i * 100, ZrodloPulsu::Tick);
        }
        // ticki co 100 ms od 10,0 s do 12,0 s: dwadzieścia jeden OBSERWACJI,
        // a decyzje tylko w 10,0 / 11,0 / 12,0 s
        assert_eq!(r.pulsy(), 21, "puls miał lecieć z każdym tickiem");
        assert_eq!(
            r.beta_pulsy, 3,
            "kadencja 1 s przepuściła inną liczbę decyzji"
        );
        assert_eq!(
            r.bilans_decyzji().rozpatrzone,
            3,
            "księgowanie idzie za decyzjami, nie za tickami"
        );
    }

    /// TRYB OBSERWACYJNY: decyzja zapada i jest policzona, ale do brokera nie
    /// idzie nic. To jest sposób na wycenę taktyki bez płacenia spreadu.
    #[test]
    fn tryb_obserwacyjny_nie_wysyla_nic() {
        let c = cfg_zera();
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let k = KonfigBety {
            wariant: WariantBety::Z,
            tylko_obserwuj: true,
            raport: false,
            ..KonfigBety::default()
        };
        let mut r = EaRdzen::default();
        r.wstrzyknij_bete(k.clone());
        r.odtworz(&[], &b, 0);
        let d = Decyzja {
            ts: 1_000,
            koszyk: 7,
            akcja: AkcjaEa::ZamknijCalosc,
            powod: PowodAkcji::Szkielet,
            bilety: vec![1],
            zlecenia: Vec::new(),
        };
        let q = b.q;
        let w = r.wykonaj_akcje(&k, &c, &mut b, &q, 0.20, &d);
        assert_eq!(w, WynikAkcji::Obserwacja);
        assert!(b.zamkniete.is_empty(), "tryb obserwacyjny zamknął pozycję");
    }

    /// **ZAKAZ LUZOWANIA STOPA.** Przesunięcie stopa OD ceny to podniesienie
    /// ryzyka po zawiązaniu koszyka — czyli to, czego zabrania zapadka N10.
    /// Egzekutor pilnuje tego STRUKTURALNIE, a nie w opisie.
    #[test]
    fn egzekutor_nie_luzuje_stopu() {
        let c = cfg_zera();
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4626.0));
        let k = konfig(WariantBety::Z);
        let mut r = EaRdzen::default();
        let q = b.q;

        // BUY ze stopem 4626 — próba zejścia na 4620 jest luzowaniem
        let luz = Decyzja {
            ts: 1_000,
            koszyk: 7,
            akcja: AkcjaEa::PrzesunStop(4620.0),
            powod: PowodAkcji::Szkielet,
            bilety: vec![1],
            zlecenia: Vec::new(),
        };
        assert_eq!(
            r.wykonaj_akcje(&k, &c, &mut b, &q, 0.20, &luz),
            WynikAkcji::OdmowaPoziomu
        );
        assert!(b.modyfikacje.is_empty(), "stop został poluzowany");
        assert_eq!(
            r.bilans_decyzji().ile(KodPominieciaBety::PoziomOdrzucony),
            1
        );

        // to samo w górę PRZECHODZI (zacieśnienie)
        let zacisk = Decyzja {
            akcja: AkcjaEa::PrzesunStop(4629.0),
            ..luz.clone()
        };
        assert_eq!(
            r.wykonaj_akcje(&k, &c, &mut b, &q, 0.20, &zacisk),
            WynikAkcji::Wykonana
        );
        assert_eq!(b.modyfikacje.len(), 1);
        assert_eq!(b.modyfikacje[0].1, Some(4629.0));

        // …a z jawną zgodą luzowanie wolno
        let k2 = KonfigBety {
            pozwol_luzowac_stop: true,
            ..konfig(WariantBety::Z)
        };
        let mut b2 = Atrapa::nowa().z_pozycja(1, 7, Some(4626.0));
        let mut r2 = EaRdzen::default();
        assert_eq!(
            r2.wykonaj_akcje(&k2, &c, &mut b2, &q, 0.20, &luz),
            WynikAkcji::Wykonana
        );
    }

    /// `stops_level` jest realny (20 punktów u PUPrime) i egzekutor DOSUWA
    /// poziom, zamiast tracić modyfikację — a jeśli dosunięcie zamieniłoby
    /// zacieśnienie w luzowanie, odmawia.
    #[test]
    fn egzekutor_dosuwa_stop_do_stops_level() {
        let c = cfg_zera();
        // bid 4630,00; stops_level 0,20 ⇒ najwyższy dopuszczalny SL to 4629,80
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4626.0));
        let k = konfig(WariantBety::Z);
        let mut r = EaRdzen::default();
        let q = b.q;
        let d = Decyzja {
            ts: 1_000,
            koszyk: 7,
            akcja: AkcjaEa::PrzesunStop(4640.0), // absurdalnie blisko/za ceną
            powod: PowodAkcji::Szkielet,
            bilety: vec![1],
            zlecenia: Vec::new(),
        };
        assert_eq!(
            r.wykonaj_akcje(&k, &c, &mut b, &q, 0.20, &d),
            WynikAkcji::Wykonana
        );
        assert_eq!(b.modyfikacje.len(), 1);
        assert!(
            (b.modyfikacje[0].1.expect("sl") - 4629.80).abs() < 1e-9,
            "stop nie został dosunięty do stops_level: {:?}",
            b.modyfikacje[0].1
        );
    }

    /// **NIGDY NIE ZAKŁADAJ, ŻE ZLECENIE PRZESZŁO.** Odmowa brokera ląduje
    /// w OBU księgach: `BilansPulsu` (kontrakt domu) i `BilansDecyzji`.
    #[test]
    fn odmowa_brokera_ląduje_w_obu_ksiegach() {
        let c = cfg_zera();
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        b.odmawiaj = true;
        let k = konfig(WariantBety::Z);
        let mut r = EaRdzen::default();
        let q = b.q;
        let d = Decyzja {
            ts: 1_000,
            koszyk: 7,
            akcja: AkcjaEa::ZamknijCalosc,
            powod: PowodAkcji::Szkielet,
            bilety: vec![1],
            zlecenia: Vec::new(),
        };
        assert_eq!(
            r.wykonaj_akcje(&k, &c, &mut b, &q, 0.20, &d),
            WynikAkcji::OdmowaBrokera
        );
        assert_eq!(
            r.bilans().ile(KodPominiecia::OdmowaBrokera),
            1,
            "brak śladu w BilansPulsu"
        );
        assert_eq!(r.bilans_decyzji().ile(KodPominieciaBety::OdmowaBrokera), 1);
        assert_eq!(r.bilans_decyzji().zlecen_odrzuconych, 1);
        assert!(r.bilans().domyka_sie(), "odmowa rozjechała bilans pulsu");
    }

    /// Inkaso części wolumenu trzyma krok lota i zostawia 0,01 — inaczej
    /// „częściowe" zamknięcie zamyka całą pozycję (ta sama arytmetyka, co
    /// `bank_on_tp`).
    #[test]
    fn ulamek_zamkniecia_trzyma_krok_lota() {
        let c = cfg_zera();
        let k = konfig(WariantBety::Z);
        let q = Quote::new(1_000, 4630.0, 4630.2).expect("kwotowanie");

        // 0,10 lota, 40 % ⇒ 0,04
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let mut r = EaRdzen::default();
        let d = Decyzja {
            ts: 1_000,
            koszyk: 7,
            akcja: AkcjaEa::ZamknijCzesc(0.40),
            powod: PowodAkcji::Szkielet,
            bilety: vec![1],
            zlecenia: Vec::new(),
        };
        assert_eq!(
            r.wykonaj_akcje(&k, &c, &mut b, &q, 0.20, &d),
            WynikAkcji::Wykonana
        );
        assert_eq!(b.czesciowe.len(), 1);
        assert!((b.czesciowe[0].1 - 0.04).abs() < 1e-9, "{:?}", b.czesciowe);

        // 100 % ⇒ zostaje 0,01, bo to jest INKASO, nie zamknięcie
        let mut b2 = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let mut r2 = EaRdzen::default();
        let d2 = Decyzja {
            akcja: AkcjaEa::ZamknijCzesc(1.0),
            ..d.clone()
        };
        assert_eq!(
            r2.wykonaj_akcje(&k, &c, &mut b2, &q, 0.20, &d2),
            WynikAkcji::Wykonana
        );
        assert!(
            (b2.czesciowe[0].1 - 0.09).abs() < 1e-9,
            "{:?}",
            b2.czesciowe
        );

        // pozycja minimalna: nie ma czego inkasować i to NIE jest odmowa
        let mut b3 = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        b3.poz[0].volume = 0.01;
        let mut r3 = EaRdzen::default();
        assert_eq!(
            r3.wykonaj_akcje(&k, &c, &mut b3, &q, 0.20, &d),
            WynikAkcji::BezPracy
        );
        assert!(b3.czesciowe.is_empty());
    }

    /// Pozycja ZAMROŻONA ręczną edycją nie jest niczyja i bot jej nie rusza —
    /// niezależnie od tego, jak mądra jest taktyka.
    #[test]
    fn zamrozona_pozycja_jest_nietykalna() {
        let c = cfg_zera();
        let k = konfig(WariantBety::Z);
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        b.poz[0].frozen = true;
        let q = b.q;
        let mut r = EaRdzen::default();
        let d = Decyzja {
            ts: 1_000,
            koszyk: 7,
            akcja: AkcjaEa::ZamknijCalosc,
            powod: PowodAkcji::Szkielet,
            bilety: vec![1],
            zlecenia: Vec::new(),
        };
        assert_eq!(
            r.wykonaj_akcje(&k, &c, &mut b, &q, 0.20, &d),
            WynikAkcji::BezPracy
        );
        assert!(
            b.zamkniete.is_empty(),
            "zamrożona pozycja została zamknięta"
        );

        // …i nie da się jej nawet WYBRAĆ
        let m = [MigawkaPozycji {
            ticket: 1,
            koszyk: 7,
            side: Side::Buy,
            volume: 0.1,
            open_price: 4628.0,
            open_ts: 100,
            sl: None,
            tp: None,
            level: 0,
            frozen: true,
            is_runner: false,
            is_toucher: false,
            wynik_usd: 0.0,
            wynik_pts: 0.0,
            szczyt_pts: 0.0,
            wiek_min: 1.0,
        }];
        assert!(wybierz_bilety(&m, WyborBiletow::Wszystkie).is_empty());
        assert!(wybierz_bilety(&m, WyborBiletow::Jeden(1)).is_empty());
        assert!(wybierz_bilety(&m, WyborBiletow::Najglebszy).is_empty());
    }

    #[test]
    fn wybor_biletow_jest_deterministyczny() {
        let poz = |t: Ticket, pts: f64, ots: Ts, runner: bool| MigawkaPozycji {
            ticket: t,
            koszyk: 7,
            side: Side::Buy,
            volume: 0.1,
            open_price: 4628.0,
            open_ts: ots,
            sl: None,
            tp: None,
            level: 0,
            frozen: false,
            is_runner: runner,
            is_toucher: false,
            wynik_usd: pts * 10.0,
            wynik_pts: pts,
            szczyt_pts: pts,
            wiek_min: 1.0,
        };
        let m = [
            poz(3, -1.0, 300, false),
            poz(1, -1.0, 100, true),
            poz(2, 5.0, 200, false),
        ];
        assert_eq!(
            wybierz_bilety(&m, WyborBiletow::Najglebszy),
            vec![1],
            "remis → niższy bilet"
        );
        assert_eq!(wybierz_bilety(&m, WyborBiletow::Najplytszy), vec![2]);
        assert_eq!(wybierz_bilety(&m, WyborBiletow::Najstarszy), vec![1]);
        assert_eq!(wybierz_bilety(&m, WyborBiletow::Najmlodszy), vec![3]);
        assert_eq!(wybierz_bilety(&m, WyborBiletow::Runner), vec![1]);
        assert_eq!(wybierz_bilety(&m, WyborBiletow::Wszystkie), vec![3, 1, 2]);
    }

    /// BUDŻET INTERWENCJI: każda dodatkowa interwencja płaci spread, więc
    /// sufit i odstęp muszą działać, zanim taktyka w ogóle zostanie zapytana.
    #[test]
    fn sufit_i_odstep_interwencji_dzialaja_przed_taktyka() {
        let c = cfg_zera();
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let mut ks = vec![koszyk_testowy(&|_| {})];
        let mut r = EaRdzen::default();
        r.wstrzyknij_bete(KonfigBety {
            wariant: WariantBety::Z,
            max_akcji_koszyk: 2,
            raport: false,
            ..KonfigBety::default()
        });
        r.puls(&c, &mut ks, &mut b, 1_000, ZrodloPulsu::Tick);
        // udajemy, że EA zdążył już dwa razy zainterweniować
        r.pamiec_test_ustaw(7, 2, 1_000);
        r.puls(&c, &mut ks, &mut b, 2_000, ZrodloPulsu::Tick);
        assert_eq!(r.bilans_decyzji().ile(KodPominieciaBety::SufitAkcji), 1);
        assert!(r.bilans_decyzji().domyka_sie());

        let mut r2 = EaRdzen::default();
        r2.wstrzyknij_bete(KonfigBety {
            wariant: WariantBety::Z,
            min_odstep_akcji_s: 60.0,
            raport: false,
            ..KonfigBety::default()
        });
        r2.puls(&c, &mut ks, &mut b, 1_000, ZrodloPulsu::Tick);
        r2.pamiec_test_ustaw(7, 1, 1_000);
        r2.puls(&c, &mut ks, &mut b, 30_000, ZrodloPulsu::Tick);
        assert_eq!(
            r2.bilans_decyzji().ile(KodPominieciaBety::OdstepAkcji),
            1,
            "30 s < 60 s"
        );
        r2.puls(&c, &mut ks, &mut b, 70_000, ZrodloPulsu::Tick);
        assert_eq!(
            r2.bilans_decyzji().obsluzone,
            2,
            "po 60 s koszyk wraca do rozpatrywania"
        );
    }

    #[test]
    fn konfiguracja_bety_z_json() {
        let k: KonfigBety = serde_json::from_str(
            r#"{"wariant":"z","format":"Synergy","kadencja_s":1.0,
                "param":{"be_prog_r":0.5,"inkaso_pct":40},"tekst":{"tryb":"cofka"},
                "nazwy_powodow":["COFKA_DO_TP1"]}"#,
        )
        .expect("poprawny JSON bety");
        assert_eq!(k.wariant, WariantBety::Z);
        assert_eq!(k.format, "Synergy");
        assert_eq!(k.kadencja_s, 1.0);
        assert_eq!(k.p("be_prog_r", 9.9), 0.5);
        assert_eq!(
            k.p("czego_nie_ma", 9.9),
            9.9,
            "domyślna wartość podawana w miejscu użycia"
        );
        assert!(!k.ma("czego_nie_ma"));
        assert_eq!(k.t("tryb", "-"), "cofka");
        assert_eq!(k.nazwa_powodu(0), "COFKA_DO_TP1");
        assert!(
            !k.tylko_obserwuj,
            "domyślnie beta WYKONUJE, a nie tylko patrzy"
        );
        assert!(
            !k.pozwol_luzowac_stop,
            "domyślnie luzowanie stopa jest ZABRONIONE"
        );

        // wariant `off` to pełnoprawna konfiguracja — po prostu nic nie robi
        let off: KonfigBety = serde_json::from_str(r#"{"wariant":"off"}"#).expect("off");
        assert_eq!(off.wariant, WariantBety::Off);
        assert_eq!(
            KonfigBety::default().wariant,
            WariantBety::Off,
            "brak pola = OFF"
        );

        // literówka w nazwie pola NIE przechodzi
        assert!(serde_json::from_str::<KonfigBety>(r#"{"wariant":"z","kadencja":1}"#).is_err());
        assert!(serde_json::from_str::<KonfigBety>(r#"{"wariant":"x"}"#).is_err());
    }

    /// `wariant: off` gasi betę tak samo jak brak zmiennej.
    #[test]
    fn wariant_off_gasi_bete_jak_brak_zmiennej() {
        let c = cfg_zera();
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        let mut ks = vec![koszyk_testowy(&|_| {})];
        let mut r = EaRdzen::default();
        r.wstrzyknij_bete(konfig(WariantBety::Off));
        r.puls(&c, &mut ks, &mut b, 1_000, ZrodloPulsu::Tick);
        assert_eq!(b.n_quote.get(), 0);
        assert_eq!(r.beta_pulsy, 0);
        assert!(r.beta().is_none());
    }

    /// Każdy kod, akcja i powód ma ROZŁĄCZNY indeks — inaczej dwa różne
    /// zdarzenia zlewałyby się w jeden licznik i dziennik kłamałby po cichu.
    #[test]
    fn indeksy_ksiegowosci_bety_sa_rozlaczne() {
        let mut v: Vec<usize> = KodPominieciaBety::WSZYSTKIE
            .iter()
            .map(|k| k.idx())
            .collect();
        let ile = v.len();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), ile);
        assert_eq!(ile, LICZBA_KODOW_BETY);

        let mut a: Vec<usize> = KodAkcji::WSZYSTKIE.iter().map(|k| k.idx()).collect();
        let ile_a = a.len();
        a.sort_unstable();
        a.dedup();
        assert_eq!(a.len(), ile_a);
        assert_eq!(ile_a, LICZBA_AKCJI);

        // powody: osiem wspólnych + szesnaście slotów taktycznych
        let mut p: Vec<usize> = vec![
            PowodAkcji::Brak.idx(),
            PowodAkcji::Szkielet.idx(),
            PowodAkcji::DozorRyzyka.idx(),
            PowodAkcji::KomendaKanalu.idx(),
            PowodAkcji::Cofka.idx(),
            PowodAkcji::Zasieg.idx(),
            PowodAkcji::Wiek.idx(),
            PowodAkcji::Rachunek.idx(),
        ];
        for n in 0..POWODY_TAKTYK as u8 {
            p.push(PowodAkcji::Taktyka(n).idx());
        }
        let ile_p = p.len();
        p.sort_unstable();
        p.dedup();
        assert_eq!(p.len(), ile_p);
        assert_eq!(
            ile_p, LICZBA_POWODOW,
            "tablica `powody` ma inny rozmiar niż zbiór powodów"
        );
        // slot spoza zakresu nie ma prawa wyjść poza tablicę
        assert!(PowodAkcji::Taktyka(200).idx() < LICZBA_POWODOW);
        assert_eq!(PowodAkcji::z_idx(1), PowodAkcji::Szkielet);
        assert_eq!(PowodAkcji::z_idx(8), PowodAkcji::Taktyka(0));
    }

    /// Licznik akcji domyka się tak samo jak bilans pulsu — i to jest cała
    /// zasada domu: nic nie ginie bez śladu.
    #[test]
    fn licznik_akcji_domyka_sie() {
        let mut l = LicznikAkcji::default();
        assert!(l.domyka_sie());
        for w in [
            WynikAkcji::Wykonana,
            WynikAkcji::CzesciowoWykonana,
            WynikAkcji::BezPracy,
            WynikAkcji::OdmowaPoziomu,
            WynikAkcji::OdmowaBrokera,
            WynikAkcji::Obserwacja,
        ] {
            l.zapadla += 1;
            l.dopisz(w);
        }
        assert!(l.domyka_sie(), "{l:?}");
        assert_eq!(l.rozliczone(), 6);
        l.zapadla += 1; // decyzja bez rozliczenia = dziura
        assert!(!l.domyka_sie(), "dziura w księgowości ma być WIDOCZNA");
    }

    /// SYTUACJA KOSZYKA składa się z PRAWDZIWYCH liczb, nie z zer.
    ///
    /// To jest test WIDOKU: dwaj autorzy wariantów będą z niego czytali i każda
    /// z tych liczb jest przesłanką decyzji, która kosztuje pieniądze.
    #[test]
    fn sytuacja_koszyka_niesie_prawdziwe_liczby() {
        let mut b = Atrapa::nowa().z_pozycja(1, 7, Some(4620.0));
        b.poz.push(Position {
            ticket: 2,
            side: Side::Buy,
            volume: 0.10,
            open_price: 4632.00,
            open_ts: 600,
            sl: Some(4620.0),
            tp: None,
            vsl: None,
            basket: Some(7),
            level: 1,
            frozen: false,
            peak_pts: 0.0,
            last_peak_ts: 0,
            is_runner: false,
            is_toucher: false,
            comment: String::new(),
        });
        let bk = koszyk_testowy(&|bk| {
            bk.realized = 5.0;
            bk.created_ts = 1_000;
        });
        let ts: Ts = 61_000;
        let q = b.q;
        let moje: Vec<MigawkaPozycji> = b.poz.iter().map(|p| migawka_pozycji(p, &q, ts)).collect();
        let pam = PamiecKoszyka::nowa(7, ts, q.mid(), 5.0);
        let s = zbierz_sytuacje(
            &bk,
            &moje,
            pam,
            &q,
            WektorStanu::default(),
            EaStan::Neutral,
            ts,
        );

        assert_eq!(s.koszyk, 7);
        assert_eq!(s.pozycje.len(), 2);
        assert!((s.wolumen - 0.20).abs() < 1e-9);
        assert!(
            (s.srednia_cena - 4630.00).abs() < 1e-9,
            "średnia ważona: {}",
            s.srednia_cena
        );
        // BUY wychodzi po BID 4630,00 — czyli PO SPREADZIE, bo tak się naprawdę
        // wychodzi: (4630−4628)·100·0,1 + (4630−4632)·100·0,1 = 0
        assert!(s.otwarty_usd.abs() < 1e-9, "otwarty: {}", s.otwarty_usd);
        assert!(
            (s.laczny_usd - 5.0).abs() < 1e-9,
            "łączny: {}",
            s.laczny_usd
        );
        assert!((s.wiek_min - 1.0).abs() < 1e-9, "wiek: {}", s.wiek_min);
        // ryzyko do SL koszyka: (4628−4620)·100·0,1 + (4632−4620)·100·0,1
        assert!((s.ryzyko_usd.expect("ryzyko") - 200.0).abs() < 1e-9);
        assert!((s.dystans_do_sl.expect("do SL") - 10.0).abs() < 1e-9);
        assert!((s.nastepny_cel.expect("cel") - 4635.0).abs() < 1e-9);
        assert!((s.dystans_do_celu.expect("do celu") - 5.0).abs() < 1e-9);
        assert_eq!(s.etap, EtapKoszyka::Pracuje);
        assert_eq!(s.zamrozonych, 0);
        assert!(s.ma_pozycje());
        assert!((s.pl_r().expect("R") - 0.025).abs() < 1e-9);
        assert!((s.ulamek_wolumenu(0.5) - 0.10).abs() < 1e-9);

        // KOSZYK BEZ STOPA nie ma MIERZALNEGO ryzyka — `None`, nie zero.
        // To rozróżnienie jest całą treścią znaleziska o koszykach bez SL.
        let bez_sl = koszyk_testowy(&|bk| bk.sl = None);
        let goly: Vec<MigawkaPozycji> = b
            .poz
            .iter()
            .map(|p| {
                let mut m = migawka_pozycji(p, &q, ts);
                m.sl = None;
                m
            })
            .collect();
        let s2 = zbierz_sytuacje(
            &bez_sl,
            &goly,
            pam,
            &q,
            WektorStanu::default(),
            EaStan::Neutral,
            ts,
        );
        assert!(s2.ryzyko_usd.is_none(), "koszyk bez SL zgłosił ryzyko");
        assert!(s2.pl_r().is_none());
    }
}

#[cfg(test)]
mod profit_budget_send_tests {
    use super::*;
    use crate::profit_budget::tests::{broker,cfg,anchor};
    #[test]
    fn profit_budget_ea_beta_market_and_pending_sends_use_shared_reserve() {
        for limit in [None,Some(3990.0)] {
            let mut c=cfg();c.ea_enabled=true;let mut b=broker();let q=b.quote();
            let mut ea=EaRdzen::default();ea.set_profit_budget_anchor(anchor(&b,&c));
            let sl=if limit.is_some(){3980.0}else{3990.0};let mut r=Rachuba::default();
            ea.zloz_wejscie(&c,&mut b,&q,0.0,Side::Buy,1.0,limit,Some(sl),Some(4100.0),-2,"test",&mut r);
            assert_eq!(r.ok,1);assert_eq!(b.sends,1);
            let volume=if limit.is_some(){b.pendings[0].volume}else{b.positions[0].volume};
            assert_eq!(volume,if limit.is_some(){0.05}else{0.04});
            ea.zloz_wejscie(&c,&mut b,&q,0.0,Side::Buy,1.0,limit,Some(sl),Some(4100.0),-2,"test",&mut r);
            assert_eq!(b.sends,1,"EA repeated the budget after its first send");assert_eq!(r.blad,1);
        }
    }
    #[test]
    fn profit_budget_ea_beta_unbound_anchor_or_missing_stop_cannot_open_new_exposure() {
        for missing_anchor in [false,true] {
            let c=cfg();let mut b=broker();let q=b.quote();let mut ea=EaRdzen::default();
            if !missing_anchor{ea.set_profit_budget_anchor(anchor(&b,&c));}
            let mut r=Rachuba::default();
            ea.zloz_wejscie(&c,&mut b,&q,0.0,Side::Buy,1.0,None,
                if missing_anchor{Some(3990.0)}else{None},Some(4100.0),0,"test",&mut r);
            assert_eq!(b.sends,0);assert_eq!(r.blad,1);
        }
    }
}
