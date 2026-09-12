//! Typy domenowe. Wszystko, co przepływa między silnikiem, brokerem i UI.

use serde::{Deserialize, Serialize};

/// Znacznik czasu w milisekundach epoki. NIGDY nie czytany z zegara wewnątrz
/// silnika — zawsze przychodzi w zdarzeniu. To jest warunek determinizmu.
pub type Ts = i64;
pub type Ticket = u64;
pub type Px = f64;

pub const XAU_CONTRACT: f64 = 100.0;
/// 1 pips złota = 0.10 $
pub const PIP: f64 = 0.10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    #[inline]
    pub fn sign(self) -> f64 {
        match self {
            Side::Buy => 1.0,
            Side::Sell => -1.0,
        }
    }

    /// Krawędź strefy po stronie LEPSZYCH wejść (BUY: dół, SELL: góra).
    /// Ta metoda istnieje po to, żeby nie dało się już popełnić błędu
    /// „fallback siatki na złej krawędzi dla SELL" z poprzedniego bota.
    #[inline]
    pub fn better_edge(self, lo: Px, hi: Px) -> Px {
        match self {
            Side::Buy => lo,
            Side::Sell => hi,
        }
    }

    #[inline]
    pub fn worse_edge(self, lo: Px, hi: Px) -> Px {
        match self {
            Side::Buy => hi,
            Side::Sell => lo,
        }
    }

    /// Czy `a` jest korzystniejsze od `b` dla tej strony?
    #[inline]
    pub fn better(self, a: Px, b: Px) -> bool {
        match self {
            Side::Buy => a < b,
            Side::Sell => a > b,
        }
    }

    #[inline]
    pub fn opposite(self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingKind {
    BuyLimit,
    SellLimit,
    BuyStop,
    SellStop,
}

impl PendingKind {
    #[inline]
    pub fn side(self) -> Side {
        match self {
            PendingKind::BuyLimit | PendingKind::BuyStop => Side::Buy,
            PendingKind::SellLimit | PendingKind::SellStop => Side::Sell,
        }
    }

    #[inline]
    pub fn limit(side: Side) -> Self {
        match side {
            Side::Buy => PendingKind::BuyLimit,
            Side::Sell => PendingKind::SellLimit,
        }
    }

    #[inline]
    pub fn stop(side: Side) -> Self {
        match side {
            Side::Buy => PendingKind::BuyStop,
            Side::Sell => PendingKind::SellStop,
        }
    }
}

/// Kwotowanie. Konstruktor pilnuje, żeby nie dało się stworzyć ceny pustej
/// ani odwróconej — jednostronne ticki z eksportu muszą zostać uzupełnione na
/// wejściu, nie w środku silnika.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub ts: Ts,
    pub bid: Px,
    pub ask: Px,
}

impl Quote {
    pub fn new(ts: Ts, bid: Px, ask: Px) -> Option<Self> {
        if bid.is_finite() && ask.is_finite() && bid > 0.0 && ask >= bid {
            Some(Quote { ts, bid, ask })
        } else {
            None
        }
    }

    #[inline]
    pub fn spread(&self) -> f64 {
        self.ask - self.bid
    }

    #[inline]
    pub fn mid(&self) -> Px {
        (self.bid + self.ask) * 0.5
    }

    /// Cena, po której WCHODZI się na daną stronę.
    #[inline]
    pub fn entry(&self, side: Side) -> Px {
        match side {
            Side::Buy => self.ask,
            Side::Sell => self.bid,
        }
    }

    /// Cena, po której WYCHODZI się z danej strony.
    #[inline]
    pub fn exit(&self, side: Side) -> Px {
        match side {
            Side::Buy => self.bid,
            Side::Sell => self.ask,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub ticket: Ticket,
    pub side: Side,
    pub volume: f64,
    pub open_price: Px,
    pub open_ts: Ts,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    /// SL trzymany u bota — broker go nie widzi
    pub vsl: Option<Px>,
    pub basket: Option<u32>,
    /// indeks poziomu w siatce (0 = pierwszy postawiony)
    pub level: i32,
    /// pozycja zamrożona ręczną edycją — bot jej nie rusza
    pub frozen: bool,
    /// szczyt zysku w punktach ceny
    pub peak_pts: f64,
    pub last_peak_ts: Ts,
    pub is_runner: bool,
    pub is_toucher: bool,
    pub comment: String,
}

impl Position {
    #[inline]
    pub fn profit_pts(&self, q: &Quote) -> f64 {
        (q.exit(self.side) - self.open_price) * self.side.sign()
    }

    #[inline]
    pub fn profit_usd(&self, q: &Quote) -> f64 {
        self.profit_pts(q) * XAU_CONTRACT * self.volume
    }

    #[inline]
    pub fn profit_at(&self, px: Px) -> f64 {
        (px - self.open_price) * self.side.sign() * XAU_CONTRACT * self.volume
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOrder {
    pub ticket: Ticket,
    pub kind: PendingKind,
    pub volume: f64,
    pub price: Px,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    pub placed_ts: Ts,
    pub basket: Option<u32>,
    pub level: i32,
    pub frozen: bool,
    pub is_toucher: bool,
    /// Zlecenie DOŁOŻONE do szczebla, żeby podnieść jego łączny wolumen po
    /// wzroście salda. Liczy się do wolumenu, ale NIE do liczby sztuk na
    /// szczeblu — inaczej `sync_grid` zobaczyłby nadmiar i je skasował.
    #[serde(default)]
    pub is_topup: bool,
    pub comment: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseReason {
    Tp,
    Sl,
    VirtualSl,
    Manual,
    Partial,
    RiskFree,
    OutAtEntry,
    Harvest,
    Stale,
    Trail,
    BasketClose,
    EodFlat,
    DayTarget,
    MaxDd,
    Ai,
    Expired,
    /// REVERSAL-EXIT (`rev_exit_*`). Osobny kod, nie `Harvest`: ta reguła ma
    /// historię 12-godzinnego zamarznięcia (cache po długości bufora) i musi
    /// być widoczna w forensyce osobno — pod `Harvest` rozjazd −80 % był
    /// nieodróżnialny od zwykłych żniw.
    RevExit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedTrade {
    pub ticket: Ticket,
    pub side: Side,
    pub volume: f64,
    pub open_price: Px,
    pub close_price: Px,
    pub open_ts: Ts,
    pub close_ts: Ts,
    pub profit: f64,
    pub commission: f64,
    pub swap: f64,
    pub reason: CloseReason,
    pub basket: Option<u32>,
    /// None preserves the historical, source-defined profit and JSON shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profit_basis: Option<crate::cost_receipt::ProfitBasis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_receipt: Option<Box<crate::cost_receipt::CostReceipt>>,
}

impl ClosedTrade {
    /// Display/export net from an explicit producer contract. No cash mutation
    /// and no inference from the size or sign of historical costs.
    pub fn net_profit(&self) -> Option<f64> {
        use crate::cost_receipt::ProfitBasis;
        if !self.profit.is_finite() || !self.commission.is_finite() || !self.swap.is_finite() {
            return None;
        }
        let net = match self.profit_basis? {
            ProfitBasis::PriceOnlyGross => self.profit + self.commission + self.swap,
            ProfitBasis::PricePlusSwap => self.profit + self.commission,
            ProfitBasis::CanonicalClosedNetV1 => self.canonical_net().ok()?,
            ProfitBasis::LegacySourceDefined => return None,
        };
        net.is_finite().then_some(net)
    }

    /// Canonical closed-net only. Never guesses costs on a historical record.
    pub fn canonical_net(&self) -> Result<f64, crate::cost_receipt::CostError> {
        use crate::cost_receipt::{CostError, ProfitBasis};
        if self.profit_basis != Some(ProfitBasis::CanonicalClosedNetV1) {
            return Err(CostError::LegacyBasisUnknown);
        }
        let receipt = self.cost_receipt.as_ref().ok_or(CostError::MissingReceipt)?;
        if !self.volume.is_finite() || self.volume<=0.0
            || (receipt.volume-self.volume).abs() > self.volume.abs().max(1.0)*1e-12 {
            return Err(CostError::ReceiptMismatch);
        }
        let net = receipt.net()?;
        if !self.profit.is_finite() || (self.profit-net).abs() > net.abs().max(1.0)*1e-12 {
            return Err(CostError::ReceiptMismatch);
        }
        let commission=receipt.entry_commission_alloc.unwrap()+receipt.exit_commission.unwrap();
        let swap=receipt.swap.unwrap();
        if !commission.is_finite() || !self.commission.is_finite() || !self.swap.is_finite()
            || (self.commission-commission).abs()>commission.abs().max(1.0)*1e-12
            || (self.swap-swap).abs()>swap.abs().max(1.0)*1e-12 {
            return Err(CostError::ReceiptMismatch);
        }
        Ok(net)
    }

    /// Projects already-booked costs into a management ledger; never charges cash.
    pub fn with_cost_receipt(mut self, receipt: crate::cost_receipt::CostReceipt)
        -> Result<Self, crate::cost_receipt::CostError> {
        use crate::cost_receipt::{CostError, ProfitBasis};
        let net = receipt.net()?;
        let commission = receipt.entry_commission_alloc.unwrap() + receipt.exit_commission.unwrap();
        if !commission.is_finite() { return Err(CostError::ArithmeticOverflow); }
        self.profit = net;
        self.commission = commission;
        self.swap = receipt.swap.unwrap();
        self.profit_basis = Some(ProfitBasis::CanonicalClosedNetV1);
        self.cost_receipt = Some(Box::new(receipt));
        self.canonical_net()?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Account {
    pub balance: f64,
    pub equity: f64,
    pub margin: f64,
    pub free_margin: f64,
    pub leverage: u32,
    /// KREDYT BONUSOWY brokera (`ACCOUNT_CREDIT`), ODDZIELNY od MT5 Balance.
    /// Wpłata 300 $ + bonus 300 $ daje B=300, C=300, E=600 bez pozycji.
    ///
    /// Czy kredyt schodzi z podstawy lota, decyduje `Settings::odlicz_kredyt`;
    /// samo pole jest wyłącznie ODCZYTEM z terminala i przy braku bonusu
    /// (albo przy sidecarze bez tej wartości) wynosi 0.
    ///
    /// Historyczny symulator zachowuje stary kontrakt dopóki
    /// `Settings::credit_balance_separate` jest OFF.
    #[serde(default)]
    pub credit: f64,
}

/// Źródło sygnału: kanał + opcjonalny temat forum.
/// Temat traktowany jest jako NIEZALEŻNY kanał — własne koszyki, własne presety.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceKey {
    pub chat_id: i64,
    pub topic_id: Option<i64>,
}

impl SourceKey {
    pub fn new(chat_id: i64, topic_id: Option<i64>) -> Self {
        Self { chat_id, topic_id }
    }
    pub fn as_string(&self) -> String {
        match self.topic_id {
            Some(t) => format!("{}:{}", self.chat_id, t),
            None => self.chat_id.to_string(),
        }
    }
}

/// Stan koszyka — jawny automat, żeby nie dało się przeskoczyć etapu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BasketState {
    /// limity rozstawione, nic jeszcze nie zafillowane
    Armed,
    /// co najmniej jedna pozycja otwarta
    Working,
    /// po RISK FREE — biegnie tylko runner
    RiskFree,
    /// zakończony
    Done,
}

/// Zaplanowany poziom siatki — „jak ma wyglądać ten szczebel".
///
/// Bez zapamiętanego planu nie da się zrobić dwóch rzeczy, które poprzedni bot
/// robił: dostawić brakujących zleceń po zafillowaniu części poziomu oraz
/// przeliczyć rozmiar niezafillowanych limitów, gdy zmienił się reżim
/// zmienności. Rekonstruowanie tego z żywych zleceń jest niemożliwe — po
/// skasowaniu zlecenia informacja przepada.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridLevel {
    pub price: Px,
    /// liczba jednostek PRZED mnożnikiem reżimu zmienności
    pub base_units: u32,
    /// wolumen JEDNEGO zlecenia na tym szczeblu (po wagach głębokości
    /// i po ewentualnym ścięciu przez limit ryzyka koszyka)
    ///
    /// 0 = wartość z czasów przed wagami; wtedy bierzemy bieżący lot.
    #[serde(default)]
    pub volume: f64,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    pub level: i32,
    pub is_toucher: bool,
    /// Chwila i CENA faktycznego wypełnienia tego szczebla (0 = niewypełniony).
    ///
    /// Cena, a nie poziom zlecenia: przy luce broker realizuje po cenie
    /// rynkowej i to ją trzeba rozliczyć. Odczyt z realizacji zlecenia jest
    /// jedynym źródłem prawdy — rekonstrukcja z biegu ekstremów myli się
    /// dokładnie tam, gdzie rynek przeskakuje poziom.
    #[serde(default)]
    pub fill_ts: Ts,
    #[serde(default)]
    pub fill_px: Px,
    /// Czy szczebel został ANULOWANY, zamiast się wypełnić.
    ///
    /// Bez tej flagi „niewypełniony" znaczy dwie różne rzeczy naraz: zlecenie
    /// nadal czeka albo zostało skasowane i już nigdy nie wejdzie.
    #[serde(default)]
    pub cancelled: bool,
    /// czy ten szczebel kiedykolwiek się zrealizował
    ///
    /// Bez tego znacznika nie da się odróżnić szczebla, który został skasowany
    /// (i wolno go odtworzyć), od szczebla, którego pozycja została otwarta
    /// i zamknięta (odtworzenie byłoby cichym, nieproszonym wejściem).
    #[serde(default)]
    pub filled: bool,
}

/// Akcje zarzadzajace rzeczywiscie wykonane z jednej wiadomosci Telegrama.
///
/// To jest mala, deterministyczna migawka pamieci dedupu `Engine`. Trzymamy
/// ja przy koszyku, bo `koszyki.json` jest jedynym stanem decyzji, ktory live
/// odtwarza przez `Engine::adopt_baskets` po restarcie procesu. Brak pola w
/// starszym zrzucie oznacza pusty wektor i zachowuje kompatybilnosc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedDoneActions {
    pub msg_id: i64,
    #[serde(default)]
    pub actions: Vec<String>,
}

/// Mandatory basket exit not yet confirmed against broker exposure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingBasketExit {
    pub reason: CloseReason,
    #[serde(default)]
    pub last_attempt_ts: Ts,
}

/// Audit/recovery latch, NOT a queued order. It never auto-executes after restart.
/// The target is a diagnostic snapshot; any eventual release needs a NEW plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingRelotReview {
    pub level: i32,
    pub created_ts: Ts,
    pub target_at_decision: f64,
    pub reason: String,
}

/// Source identity, not mutable management TP/SL. Unknown old snapshots remain
/// unknown. Review is persisted as a HOLD, never auto-replayed after restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntryEditState {
    pub schema_version: u8,
    pub revision: u64,
    pub source: Option<crate::parser::EntrySignal>,
    pub applied_ts: Ts,
    /// Explicit publisher withdrawal; cannot be undone by cosmetic/source edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_by_source_ts: Option<Ts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<EntryEditReview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntryEditReview {
    pub desired_source: crate::parser::EntrySignal,
    pub received_ts: Ts,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Basket {
    pub id: u32,
    pub source: SourceKey,
    pub source_name: String,
    /// id wiadomości, z której powstał — do wiązania odpowiedzi i edycji
    pub msg_id: i64,
    /// Durable exit intent; absent in older snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_exit: Option<PendingBasketExit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_relot_review: Vec<PendingRelotReview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_edit_state: Option<Box<EntryEditState>>,
    /// Wiadomości zarządzające już jednoznacznie przypisane do koszyka.
    ///
    /// Dzięki temu odpowiedź na odpowiedź (`7549 → 7464 → 7438`) zachowuje
    /// adresata także po restarcie. Lista jest częścią migawki koszyka; brak
    /// pola w starszym JSON oznacza pustą listę i zachowuje kompatybilność.
    #[serde(default)]
    pub msg_aliases: Vec<i64>,
    /// Trwala pamiec dedupu komunikatow zarzadzajacych.
    ///
    /// Pole jest zapisywane tylko przy osi `dedup_management_po_restarcie`.
    /// `skip_serializing_if` sprawia, ze os OFF zachowuje stary zrzut bez
    /// dodatkowego klucza (kontrakt legacy/zera).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub persisted_done_actions: Vec<PersistedDoneActions>,
    pub side: Side,
    pub is_limit: bool,
    #[serde(default)]
    pub is_stop: bool,
    /// strefa z sygnału (przed offsetami)
    pub entry_lo: Px,
    pub entry_hi: Px,
    /// strefa po zastosowaniu offsetów z konfiguracji
    pub zone_lo: Px,
    pub zone_hi: Px,
    pub sl: Option<Px>,
    pub tps: Vec<Px>,
    pub tp_stage: usize,
    /// ILE CELÓW OSIĄGNĄŁ RYNEK BEZ NASZEJ POZYCJI (0 = żadnego).
    ///
    /// Osobne pole od `tp_stage`, bo to są DWIE RÓŻNE RZECZY: `tp_stage` mówi
    /// „tyle transz zainkasowaliśmy", a to mówi „tyle celów plan setupu
    /// wykonał, a nas w rynku nie było". Pierwsze jest księgowością naszej
    /// transakcji, drugie — obserwacją rynku.
    ///
    /// Czytelnik jest DOKŁADNIE JEDEN: reguła `pending_lifetime` („limity żyją
    /// do pierwszego/drugiego/trzeciego celu"). Żaden inny mechanizm — ani
    /// re-entry, ani drabinka SL, ani `retarget`, ani cechy modelu, ani panel
    /// — tego pola nie widzi i widzieć nie może.
    #[serde(default)]
    pub plan_wykonany_do: usize,
    pub created_ts: Ts,
    pub state: BasketState,
    pub tickets: Vec<Ticket>,
    pub pendings: Vec<Ticket>,
    /// suma zrealizowana na tym koszyku
    pub realized: f64,
    pub events: Vec<BasketEvent>,

    /// plan siatki — do dostawiania i przeliczania zleceń oczekujących
    #[serde(default)]
    pub levels: Vec<GridLevel>,
    /// ile razy koszyk wszedł ponownie po trafionym celu
    #[serde(default)]
    pub reentries: u32,
    /// cena ostatniego wejścia rynkowego — próg dla kolejnego kroku
    #[serde(default)]
    pub last_entry_px: Option<Px>,
    /// czy koszyk był już zabezpieczony (RISK FREE / SPP) — podnosi podłogę
    /// schodkowego SL do breakeven
    #[serde(default)]
    pub secured: bool,
    /// Jawny komunikat SPP zabronił ponownego uzbrojenia już płaskiego
    /// koszyka. Osobne od `secured`, bo bez pozycji nie ma ryzyka do
    /// zabezpieczenia; pole dotyczy wyłącznie przyszłego rearmu.
    #[serde(default)]
    pub rearm_blocked_by_spp: bool,
    /// Z-10: czy koszyk zabezpieczyła REGUŁA SILNIKA (`riskfree_pass`),
    /// a nie komunikat z kanału ani SPP.
    ///
    /// `riskfree_runner_max_hold_min` jest opisane jako zegar runnera
    /// utworzonego przez regułę, ale sweep filtrował po samym `secured_ts` —
    /// więc domykał też koszyki uwolnione SŁOWEM sygnalisty, a koszyki po
    /// `SECURING PARTIAL PROFITS` (te bez `secured_ts`) omijał zupełnie.
    #[serde(default)]
    pub secured_by_rule: bool,
    /// czy koszyk kiedykolwiek miał otwartą pozycję (blokuje wygaszanie
    /// „starego sygnału" — to już nie jest sygnał czekający, tylko trade)
    #[serde(default)]
    pub had_positions: bool,

    /// Sygnał miał w drabince pozycję „TP OPEN" — czyli cel bez liczby.
    ///
    /// Parser to zapisywał od zawsze, ale silnik nigdy z tego nie korzystał.
    /// Poprzedni bot traktuje „TP OPEN" jak PEŁNOPRAWNY kolejny cel o wartości
    /// `ostatni ± tp_open_offset` (bot.py `resolve_tp`, `final_tp`) — i to jest
    /// cel, który dostaje u brokera runner.
    #[serde(default)]
    pub tp_open: bool,
    /// Przesuniecie warstw PODANE W TRESCI sygnalu (dolary, ku pierwszemu
    /// wejsciu). `None` = kanal nic nie powiedzial. Patrz
    /// `EntrySignal::warstwy_offset` i `entry_warstwy_z_tekstu`.
    #[serde(default)]
    pub warstwy_offset: Option<f64>,

    #[serde(default)]
    pub rearms: u32,
    /// Kiedy ostatnio przezbrojono siatkę — do odstępu `rearm_min_gap_min`.
    #[serde(default)]
    pub last_rearm_ts: Ts,

    #[serde(default)]
    pub wol_pierwotny: Vec<(Ticket, f64)>,

    #[serde(default)]
    pub tp_touch_ts: Vec<Ts>,

    /// CENA w chwili pierwszego dotknięcia każdego celu (0 = nie dotknięty).
    ///
    /// Równoległa do `tp_touch_ts`. Sam poziom celu nie wystarczy: wyjście
    /// rozlicza się po cenie rynkowej, a ta przy luce bywa daleko za poziomem.
    #[serde(default)]
    pub tp_touch_px: Vec<Px>,

    /// Chwila i cena pierwszego dotknięcia stop-lossa koszyka (0 = nie dotknięty).
    #[serde(default)]
    pub sl_touch_ts: Ts,
    #[serde(default)]
    pub sl_touch_px: Px,

    #[serde(default)]
    pub adverse_since: Ts,

    /// INDYWIDUALNY kres życia koszyka w minutach (0 = obowiązuje globalny
    /// `basket_max_age_min`).
    ///
    /// Ustawiany przez filtr tempa w trybie MIĘKKIM: koszyk, przez który cena
    /// przeleciała, nie jest zabijany — traci tylko prawo do długiego trzymania.
    /// Krótszy z dwóch limitów wygrywa.
    #[serde(default)]
    pub age_limit_min: f64,

    #[serde(default)]
    pub drop_po_ts: Ts,

    /// Koszyk został oznaczony przez filtr tempa jako PRZELOT — cena przeszła
    /// przez strefę zamiast o nią zaczepić.
    ///
    /// Ustawiane w OBU trybach filtra (twardym i miękkim). Reguły DOKŁADAJĄCE
    /// ekspozycję sprawdzają to i odmawiają: koszyk-przelot nie dostaje dokładek.
    #[serde(default)]
    pub tempo_fast: bool,

    /// Czy filtr tempa OCENIŁ już ten koszyk (ocena jest JEDNORAZOWA).
    ///
    /// ⚠ Bez tej flagi `reject_fast_filled_baskets` — wołany z `on_tick` —
    /// dopisywał do `regime_hist` KAŻDY kwalifikujący się koszyk na KAŻDYM
    /// ticku. Koszyk w trybie miękkim żyje jeszcze 30 minut i przez ten czas
    /// był liczony tysiące razy, więc udział w oknie odzwierciedlał
    /// **czas życia razy częstość ticków**, a nie częstość zjawiska.
    ///
    /// Skutek: przy suficie 100 wpisów okno historii obejmowało ostatnie
    /// kilkadziesiąt ticków, czyli SEKUNDY zamiast dni — bramka reżimu nie
    /// mierzyła reżimu, tylko migawkę „co teraz żyje". Stąd fałszywy wniosek
    /// „ponad 90 % koszyków to przeloty" i pozorna binarność bramki.
    #[serde(default)]
    pub tempo_checked: bool,

    /// Czy piramida już dołożyła do tego koszyka (dokładka jest jednorazowa).
    #[serde(default)]
    pub pyramided: bool,

    /// Chwila ostatniej dokładki tempowej (0 = żadnej). Podstawa dla
    /// `fast_addon_cooldown_s`.
    #[serde(default)]
    pub last_addon_ts: Ts,

    /// Ile DOKŁADEK TEMPOWYCH dostał już ten koszyk (`fast_addon_*`).
    ///
    /// Osobny licznik od `pyramided`, bo to inna reguła: piramida czeka na
    /// POTWIERDZONY cel i dokłada limitem na cofnięciu, dokładka tempowa
    /// wchodzi rynkiem W TRAKCIE szybkiego ruchu. Mogą działać naraz.
    #[serde(default)]
    pub fast_addons: u32,

    /// Chwila OSTATNIEGO trafionego celu (0 = żaden).
    ///
    /// Podstawa dla `reenter_min_return_s`: bez opóźnienia bot dokłada
    /// natychmiast po celu i piłuje w kółko ten sam poziom.
    #[serde(default)]
    pub last_tp_ts: Ts,

    /// Szczyt ŁĄCZNEGO wyniku koszyka (zrealizowany + otwarty) w dolarach.
    ///
    /// Śledzony na bieżąco, bo z migawki nie da się go odtworzyć. To jest
    /// koszykowy odpowiednik `Position::peak_pts` — i jedyna podstawa, na
    /// której da się powiedzieć „koszyk oddał połowę tego, co miał".
    #[serde(default)]
    pub peak_pl_usd: f64,

    /// Ryzyko PEŁNEGO planu siatki w chwili rozstawienia ($).
    ///
    /// Zapamiętane, bo ryzyko bieżące maleje w miarę domykania warstw, a do
    /// porównywania koszyków między sobą potrzebna jest stała jednostka.
    #[serde(default)]
    pub risk_initial_usd: f64,

    /// Kiedy koszyk został uwolniony od ryzyka (RISK FREE).
    ///
    /// Od tej chwili — nie od sygnału i nie od pierwszego wypełnienia — biegnie
    /// zegar `riskfree_runner_max_hold_h`. Koszyk potrafi czekać na wypełnienie
    /// wiele godzin, a runnerem staje się dopiero tutaj; mieszanie tych dwóch
    /// zegarów dawałoby „72 h", które w rzeczywistości znaczy 76 h.
    #[serde(default)]
    pub secured_ts: Ts,

    #[serde(default)]
    pub zone_touched: bool,

    #[serde(default)]
    pub be_ts: Ts,

    /// Czy reguła „cel osiągnięty bez nas" jest już UZBROJONA.
    ///
    /// Uzbraja się w chwili, gdy cena stoi po WEJŚCIOWEJ stronie najbliższego
    /// celu — czyli gdy setup rzeczywiście czeka na przejście drogi strefa→cel.
    /// Bez tego rozróżnienia sygnał opublikowany „na pułapkę" (rynek już za
    /// celem) kasuje własną siatkę w tej samej sekundzie, w której ją wystawia.
    #[serde(default)]
    pub drop_armed: bool,
}

impl Basket {
    #[inline]
    pub fn mid(&self) -> Px {
        (self.zone_lo + self.zone_hi) * 0.5
    }
    #[inline]
    pub fn width(&self) -> f64 {
        self.zone_hi - self.zone_lo
    }
    #[inline]
    pub fn alive(&self) -> bool {
        !matches!(self.state, BasketState::Done)
    }

    #[inline]
    pub fn ma_pozycje(&self) -> bool {
        !self.tickets.is_empty()
    }

    /// Ile celów wolno rozpatrywać jako „już za nami" przy WYBORZE poziomu
    /// do sprawdzenia w pętli cenowej.
    ///
    /// Dla koszyka z pozycją to po prostu jego etap. Dla koszyka bez pozycji —
    /// większa z dwóch liczb, bo obie są prawdziwe: koszyk mógł zainkasować
    /// TP1 z pozycji, stracić ją, i dopiero potem rynek sam poszedł do TP2.
    /// Bez `max` licznik cofałby się i reguła `pending_lifetime` nigdy by nie
    /// dojrzała do `UntilTp2`/`UntilTp3`.
    #[inline]
    pub fn etap_obserwowany(&self) -> usize {
        self.tp_stage.max(self.plan_wykonany_do)
    }

    pub fn zeruj_postep(&mut self) {
        self.tp_stage = 0;
        self.plan_wykonany_do = 0;
        self.zone_touched = false;
        self.drop_armed = false;
        self.drop_po_ts = 0;
        self.last_tp_ts = 0;
        self.tp_touch_ts.clear();
        self.tp_touch_px.clear();
        self.sl_touch_ts = 0;
        self.sl_touch_px = 0.0;
    }
}

/// Migawka KOSZYKA jako całości — wielkości pierwszej klasy.
///
/// Powstała z zamówienia właściciela: „brakuje liczenia, ile w sumie są warte
/// wszystkie pozycje z poszczególnego koszyka, zamiast patrzenia na pozycje
/// osobno". Silnik podejmował dotąd decyzje per pozycja, a kanał ATFX zarządza
/// KOSZYKIEM: jego „risk free" to domknięcie części warstw tak, żeby całość
/// wyszła na zero, liczone od średniej ważonej ceny wejścia.
///
/// **To jest JEDYNE miejsce, w którym te wielkości się liczą.** Reguła RISK
/// FREE, przezbrojenie siatki i moduł cech biorą je stąd — inaczej powstałyby
/// trzy definicje „wyniku koszyka", które rozjechałyby się o parę centów
/// dokładnie wtedy, gdy ktoś porównałby decyzję bota z cechą modelu.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasketView {
    pub id: u32,
    pub side: Side,
    /// zrealizowany + otwarty wynik CAŁEGO koszyka w dolarach
    pub pl_usd: f64,
    /// wynik w wielokrotności ryzyka BIEŻĄCEGO (ile jeszcze można stracić).
    /// Właściwa jednostka do decyzji „mam 2R, uwalniam koszyk".
    pub pl_r_current: f64,
    /// wynik w wielokrotności ryzyka POCZĄTKOWEGO (stała jednostka).
    /// Właściwa do porównywania koszyków między sobą.
    pub pl_r_initial: f64,
    /// średnia cena wejścia WAŻONA WOLUMENEM — cena, przy której na zero
    /// wychodzi całość. Tak definiuje breakeven autor kanału.
    pub avg_entry: Px,
    /// Σ |cena wejścia − SL| × 100 × wolumen po OTWARTYCH pozycjach
    pub risk_usd: f64,
    /// to samo, ale policzone dla PEŁNEGO planu siatki w chwili rozstawienia
    pub risk_initial_usd: f64,
    /// szczyt łącznego wyniku koszyka i spadek od niego
    pub peak_pl_usd: f64,
    pub drawdown_from_peak: f64,
    pub filled_layers: u32,
    pub pending_layers: u32,
    /// jaka CZĘŚĆ zaplanowanego ryzyka jest już w rynku (0–1)
    pub planned_risk_in_market: f64,
    pub age_min: f64,
    pub secured: bool,
    pub tp_stage: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasketEvent {
    pub ts: Ts,
    pub text: String,
}

/// Statystyki liczone przyrostowo — bez przeglądania historii co tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    // ---- MIERNIK PRZELICZANIA LOTA W ZLECENIACH OCZEKUJĄCYCH ----
    // Zbierane zawsze, gdy reguła jest włączona; na razie NIC nie sterują.
    // `relot_dyst_min` i `_blisko` mają odpowiedzieć na pytanie, czy warto
    // wprowadzić próg „za blisko, nie ruszaj" — patrz `relot_pendings`.
    #[serde(default)]
    pub relot_prob: u32,
    #[serde(default)]
    pub relot_udane: u32,
    #[serde(default)]
    pub relot_odmowy: u32,
    #[serde(default)]
    pub relot_blisko: u32,
    #[serde(default)]
    pub relot_dyst_suma: f64,
    #[serde(default)]
    pub relot_dyst_min: f64,
    // ---- KIERUNKI OSOBNO ----
    // Bez rozdzielenia nie da się odpowiedzieć na pytanie użytkownika, czy
    // strona RYZYKA (w dół) sama z siebie podnosi dno. Liczymy zdarzenia
    // i WOLUMEN, bo sto dokładek po 0,01 to co innego niż jedna po 1,00.
    #[serde(default)]
    pub relot_up_zdarzen: u32,
    #[serde(default)]
    pub relot_down_zdarzen: u32,
    #[serde(default)]
    pub relot_up_lotow: f64,
    #[serde(default)]
    pub relot_down_lotow: f64,
    /// ile razy cel zmalał, choć saldo NIE zmalało — czyli ile redukcji bierze
    /// się wyłącznie ze spłaszczania wag RR, a nie ze spadku kapitału
    #[serde(default)]
    pub relot_down_bez_spadku: u32,
    /// ile dokładek podnosi szczebel POWYŻEJ tego, na co pozwala plan
    /// przeliczony na bieżące saldo — czyli ile razy relot omija
    /// `cap_basket_risk` (liczony od EQUITY) i sufit portfela
    #[serde(default)]
    pub relot_up_ponad_plan: u32,
    /// ile razy plan przeliczony na bieżące saldo wyszedł PUSTY (nic się nie
    /// mieści). Bez tej liczby nie da się odróżnić „cel wg planu nic nie
    /// zmienia, bo szczeble są w porządku" od „nic nie zmienia, bo planu
    /// w ogóle nie było" — a to dwie zupełnie różne rzeczy.
    #[serde(default)]
    pub relot_plan_pusty: u32,
    /// ile razy plan wyszedł policzalny
    #[serde(default)]
    pub relot_plan_ok: u32,
    /// suma |cel wg planu − cel wg gołego lota| po wszystkich zdarzeniach:
    /// dolarowy rozmiar rozjazdu obu definicji celu
    #[serde(default)]
    pub relot_rozjazd_lotow: f64,
    #[serde(default)]
    pub relot_szczebli: u32,
    /// Ile koszyków POMINIĘTO, bo przeliczony plan miał inny KSZTAŁT niż plan
    /// zapisany w koszyku (inny zestaw szczebli albo spłaszczone wagi R:R).
    ///
    /// `rr_multipliers` wraca do równych wag przy szczeblu o zerowym ryzyku
    /// albo bez drogi do celu — i robi to dla CAŁEGO koszyka. Relot ma
    /// zmieniać wyłącznie SKALĘ; gdyby przepuścił zdegenerowany plan, po cichu
    /// przepisałby drabinkę na płaską, czyli zrobiłby dokładnie to, co robił
    /// stary tryb `wg_planu = false`.
    #[serde(default)]
    pub relot_ksztalt_odmowa: u32,
    // ---- REDUKCJA EKSPOZYCJI (`expo_cap_pct`) ----
    /// NAJWYŻSZA zaobserwowana ekspozycja POTENCJALNA w % equity.
    ///
    /// Liczona ZAWSZE, gdy reguła jest włączona — także wtedy, gdy próg jest
    /// ustawiony tak wysoko, że nigdy nie wiąże. Dzięki temu jeden przebieg
    /// kontrolny (próg nieosiągalny, zero zadziałań, wynik co do centa jak
    /// baza) mówi, w jakim ZAKRESIE próg w ogóle ma prawo coś zrobić —
    /// zamiast zgadywania płaskowyżu w ciemno.
    #[serde(default)]
    pub expo_max_pct: f64,
    /// ile razy próg został przekroczony (tick z zadziałaniem)
    #[serde(default)]
    pub expo_zdarzen: u32,
    /// ile leżących zleceń skasowano — wariant (a)
    #[serde(default)]
    pub expo_pend_skasowane: u32,
    /// ile LOTÓW zdjęto kasowaniem zleceń. Sto szczebli po 0,01 to co innego
    /// niż jeden po 1,00, a licznik zdarzeń tego nie rozróżnia.
    #[serde(default)]
    pub expo_lotow: f64,
    /// ile pozycji domknięto — wariant (b), tylko przy `expo_cap_close`
    #[serde(default)]
    pub expo_poz_domkniete: u32,
    /// ile razy reguła zeszła do ZERA leżących zleceń i nadal była nad progiem
    /// (czyli sam wariant (a) nie wystarczył). Bez tej liczby „(a) wystarcza"
    /// jest nieodróżnialne od „(a) nie miał czego kasować".
    #[serde(default)]
    pub expo_niedosyt: u32,
    /// Neutral sizing observations/fallbacks, never counted as rejected orders.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub lot_sizing_diagnostics: std::collections::BTreeMap<String,u64>,
    pub balance: f64,
    #[serde(default)]
    pub credit: f64,
    pub equity: f64,
    pub start_balance: f64,
    pub peak_equity: f64,
    pub day_start_equity: f64,
    pub day_peak_equity: f64,
    pub max_dd_abs: f64,
    pub max_dd_pct: f64,
    pub day_max_dd: f64,
    pub realized_today: f64,
    pub trades: u32,
    pub wins: u32,
    pub losses: u32,
    pub gross_win: f64,
    pub gross_loss: f64,
    pub messages: u64,
    pub signals: u64,
    /// dzień kalendarzowy (dni od epoki) — do resetu statystyk dziennych
    pub day: i64,
}

impl Stats {
    pub fn new(balance: f64) -> Self {
        Stats {
            relot_prob: 0,
            relot_udane: 0,
            relot_odmowy: 0,
            relot_blisko: 0,
            relot_dyst_suma: 0.0,
            relot_dyst_min: 0.0,
            relot_up_zdarzen: 0,
            relot_down_zdarzen: 0,
            relot_up_lotow: 0.0,
            relot_down_lotow: 0.0,
            relot_down_bez_spadku: 0,
            relot_up_ponad_plan: 0,
            relot_plan_pusty: 0,
            relot_plan_ok: 0,
            relot_rozjazd_lotow: 0.0,
            relot_szczebli: 0,
            relot_ksztalt_odmowa: 0,
            expo_max_pct: 0.0,
            expo_zdarzen: 0,
            expo_pend_skasowane: 0,
            expo_lotow: 0.0,
            expo_poz_domkniete: 0,
            expo_niedosyt: 0,
            lot_sizing_diagnostics: std::collections::BTreeMap::new(),
            balance,
            credit: 0.0,
            equity: balance,
            start_balance: balance,
            peak_equity: balance,
            day_start_equity: balance,
            day_peak_equity: balance,
            max_dd_abs: 0.0,
            max_dd_pct: 0.0,
            day_max_dd: 0.0,
            realized_today: 0.0,
            trades: 0,
            wins: 0,
            losses: 0,
            gross_win: 0.0,
            gross_loss: 0.0,
            messages: 0,
            signals: 0,
            day: i64::MIN,
        }
    }

    #[inline]
    pub fn profit_factor(&self) -> f64 {
        if self.gross_loss > 0.0 {
            self.gross_win / self.gross_loss
        } else if self.gross_win > 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    }

    #[inline]
    pub fn win_rate(&self) -> f64 {
        if self.trades == 0 {
            0.0
        } else {
            self.wins as f64 / self.trades as f64
        }
    }
}

/// Dni od epoki dla znacznika czasu, w strefie serwera brokera.
#[inline]
pub fn day_of(ts: Ts, tz_offset_ms: i64) -> i64 {
    (ts + tz_offset_ms).div_euclid(86_400_000)
}

/// Godzina (0-23) w strefie serwera brokera.
#[inline]
pub fn hour_of(ts: Ts, tz_offset_ms: i64) -> u32 {
    (((ts + tz_offset_ms).rem_euclid(86_400_000)) / 3_600_000) as u32
}

/// Dzień tygodnia, 0 = poniedziałek.
#[inline]
pub fn weekday_of(ts: Ts, tz_offset_ms: i64) -> u32 {
    // 1970-01-01 to czwartek (=3 przy poniedziałku 0)
    ((day_of(ts, tz_offset_ms) + 3).rem_euclid(7)) as u32
}
