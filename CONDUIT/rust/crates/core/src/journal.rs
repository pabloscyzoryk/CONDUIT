
use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Wersja schematu zdarzenia. Rośnie, gdy zmienia się ZNACZENIE pola.
/// Analizator odmawia liczenia na pliku z nowszym schematem, zamiast po cichu
/// interpretować liczby po swojemu.
pub const JOURNAL_SCHEMA: u32 = 1;

// ============================================================
//  POZIOM I KATEGORIA
// ============================================================

/// Waga zdarzenia. Kolejność wariantów JEST istotna — po niej działa filtr
/// `min_level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EventLevel {
    /// szczegóły przydatne wyłącznie przy śledzeniu błędu
    Debug,
    #[default]
    Info,
    /// coś się udało i warto to widzieć w skrócie dnia
    Ok,
    Warn,
    Error,
}

impl EventLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            EventLevel::Debug => "debug",
            EventLevel::Info => "info",
            EventLevel::Ok => "ok",
            EventLevel::Warn => "warn",
            EventLevel::Error => "error",
        }
    }

    /// Odwzorowanie starych poziomów `LogLine` (0 info, 1 ok, 2 warn, 3 error).
    pub fn from_legacy(v: u8) -> Self {
        match v {
            1 => EventLevel::Ok,
            2 => EventLevel::Warn,
            3 => EventLevel::Error,
            _ => EventLevel::Info,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "debug" => Some(EventLevel::Debug),
            "info" => Some(EventLevel::Info),
            "ok" | "success" => Some(EventLevel::Ok),
            "warn" | "warning" => Some(EventLevel::Warn),
            "error" | "err" => Some(EventLevel::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EventCategory {
    /// wiadomość z kanału, jej parsowanie i odrzucenia
    Signal,
    /// decyzja silnika: wejść / nie wejść / zignorować komunikat
    Decision,
    /// zlecenia i modyfikacje u brokera
    Order,
    /// otwarcie i zamknięcie pozycji — to jest sedno pliku
    Trade,
    /// koszyk: powstanie, etapy celów, zakończenie
    Basket,
    /// strażnicy kapitału, blokady, awaryjne zamknięcia
    Risk,
    /// saldo, equity, granica doby
    Account,
    /// stan procesu, połączenia, poczta — czyli to, co wolno odsiać
    #[default]
    System,
}

impl EventCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            EventCategory::Signal => "signal",
            EventCategory::Decision => "decision",
            EventCategory::Order => "order",
            EventCategory::Trade => "trade",
            EventCategory::Basket => "basket",
            EventCategory::Risk => "risk",
            EventCategory::Account => "account",
            EventCategory::System => "system",
        }
    }
}

/// Rodzaj zdarzenia. Też zamknięta lista — po niej działa całe łączenie
/// strumieni w analizatorze.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// przyjęto wiadomość z kanału
    MessageReceived,
    /// wiadomość rozpoznana jako konkretna akcja
    SignalParsed,
    /// akcja z wiadomości ODRZUCONA — `reason` obowiązkowy
    SignalRejected,
    BasketCreated,
    BasketUpdated,
    BasketClosed,
    OrderPlaced,
    OrderRejected,
    PendingCancelled,
    PositionOpened,
    PositionClosed,
    StopsModified,
    /// trafiony cel koszyka (z ceny, z kanału albo z realizacji brokera)
    TargetHit,
    /// komunikat celu ZIGNOROWANY — `reason` obowiązkowy
    TargetIgnored,
    /// awaryjne zamknięcie od strażnika kapitału (dawne `EMERGENCY_STOP`)
    RiskStop,
    /// blokada nowych wejść
    GuardBlocked,
    /// zmiana doby handlowej serwera
    DayRollover,
    #[default]
    Note,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::MessageReceived => "message_received",
            EventKind::SignalParsed => "signal_parsed",
            EventKind::SignalRejected => "signal_rejected",
            EventKind::BasketCreated => "basket_created",
            EventKind::BasketUpdated => "basket_updated",
            EventKind::BasketClosed => "basket_closed",
            EventKind::OrderPlaced => "order_placed",
            EventKind::OrderRejected => "order_rejected",
            EventKind::PendingCancelled => "pending_cancelled",
            EventKind::PositionOpened => "position_opened",
            EventKind::PositionClosed => "position_closed",
            EventKind::StopsModified => "stops_modified",
            EventKind::TargetHit => "target_hit",
            EventKind::TargetIgnored => "target_ignored",
            EventKind::RiskStop => "risk_stop",
            EventKind::GuardBlocked => "guard_blocked",
            EventKind::DayRollover => "day_rollover",
            EventKind::Note => "note",
        }
    }
}

// ============================================================
//  POWODY — ZAMKNIĘTA LISTA
// ============================================================

/// Dlaczego coś zostało pominięte, odrzucone albo zignorowane.
///
/// To jest odpowiedź na brak nr 6: `TARGET_ignore_B` wystąpiło w logu 72 razy
/// i ANI RAZU nie podało powodu. Kod z zamkniętej listy da się policzyć
/// (`jq -r .reason | sort | uniq -c`); zdanie po polsku — nie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectCode {
    /// handel wstrzymany przez strażnika (`halted`)
    Halted,
    /// sygnał rynkowy przy ustawieniu „tylko LIMIT"
    MarketSignalWhileLimitOnly,
    /// filtr tagów treści (np. HIGH RISK)
    TagFilter,
    /// filtr kierunku (tylko BUY / tylko SELL)
    SideFilter,
    /// filtr reżimu rynku (sygnał pod prąd średniej)
    RegimeFilter,
    /// SL z sygnału był już przebity, zanim powstało zlecenie
    SlBreached,
    /// cena uciekła za strefę dalej niż `max_chase_beyond_zone`
    ChaseTooFar,
    /// odległość strefa–SL poza limitem
    SlDistanceTooBig,
    /// GEOMETRIA SYGNAŁU NIE TRZYMA SIĘ KUPY — literówka w treści.
    ///
    /// Treść wejściowa może zawierać literówkę w strefie, stopie albo celu.
    /// Taki błąd potrafi umieścić cel po niewłaściwej stronie rynku albo bardzo
    /// daleko od strefy, pozostawiając pozycję bez sensownego planu wyjścia.
    SignalGeometryInsane,
    /// bramka wejścia zamknięta (ekspozycja, sesja, przerwa po stratach…)
    EntryGateBlocked,
    /// limit liczby otwartych pozycji
    MaxOpenPositions,
    /// limit liczby żywych koszyków
    MaxOpenBaskets,
    /// limit sumy wolumenu w jedną stronę
    MaxDirectionalLots,
    /// rachunkowy bezpiecznik `expo_cap_pct` usunął istniejące zlecenie
    /// oczekujące albo pozycję. To nie jest odrzucenie nowego sygnału ani
    /// brak marginesu u brokera: użytkownik musi widzieć, że już wystawiona
    /// ekspozycja została zmieniona przez nadpisanie konfiguracji rachunku.
    ExposureCap,
    /// equity poniżej twardej podłogi
    EquityFloor,
    /// poza godzinami sesji
    SessionClosed,
    /// przerwa po serii WŁASNYCH strat (`streak_pause_*`)
    StreakPause,
    /// HAMULEC SL-HIT — kanał ogłosił w dobie N własnych stopów
    /// (`slhit_pause_n`), więc wejścia są wstrzymane.
    ///
    /// OSOBNY KOD, a nie `StreakPause`, i to nie jest kosmetyka. Obie reguły
    /// są pauzami, ale czytają DWA RÓŻNE ŹRÓDŁA: `StreakPause` liczy nasz
    /// własny zrealizowany wynik, hamulec czyta REŻIM Z KANAŁU, zanim strata
    /// zdąży się u nas zmaterializować. Dopóki kod był wspólny, panel pisał
    /// „pauza po serii strat" botowi, który nie miał ANI JEDNEJ straty —
    /// czyli podawał użytkownikowi fałszywą przyczynę, po której nie dało się
    /// znaleźć prawdziwej. Osobny kod daje też osobny licznik w lejku.
    SlHitBrake,
    /// sygnał starszy niż `ignore_old_after_min`
    StaleSignal,
    /// cena sięgnęła celu, zanim siatka limitów zdążyła się wypełnić
    TargetReachedWithoutEntry,
    /// koszyk nie mieści się w limicie ryzyka — pusty plan siatki
    RiskBudgetExhausted,
    /// nie da się wskazać koszyka, którego dotyczy komunikat
    NoTargetBasket,
    /// komunikat celu niepotwierdzony ceną
    TpNotConfirmedByPrice,
    /// poziom z komunikatu zarządzającego jest absurdalnie daleko od rynku
    /// i całej geometrii jednoznacznie adresowanego koszyka
    ManagementLevelInsane,
    /// ten etap drabinki już padł
    TpStageAlreadyPassed,
    /// numer celu poza drabinką
    TpIndexOutOfRange,
    /// koszyk o podanym identyfikatorze nie istnieje
    BasketNotFound,
    /// koszyk jest już zakończony
    BasketDone,
    /// SECURING PARTIAL PROFITS na koszyku starszym niż `spp_max_age_h`
    SppTooOld,
    /// edycja wiadomości powtarza akcję już wykonaną
    DuplicateEditedAction,
    /// edycja wiadomości NIEZNANEJ mapie `msg_to_basket` niesie wejście —
    /// po restarcie mapa zna tylko koszyki żywe, więc edycja wczorajszego
    /// sygnału otwierałaby świeży koszyk na starych cenach (oś
    /// `edycja_sieroty_nie_otwiera`, Pakiet A5)
    EditOrphan,
    /// broker odrzucił zlecenie
    BrokerRejected,
    /// broker: za mało depozytu
    NotEnoughMargin,
    /// broker: SL/TP bliżej ceny niż stops level
    InvalidStops,
    /// broker: rynek zamknięty
    MarketClosed,
    Manual,
    /// ustawienie każe ignorować ten rodzaj komunikatu
    DisabledBySetting,
    /// wyczerpany dzienny budżet transakcji (`daily_signal_budget`)
    DailyBudgetSpent,
    /// sygnał poniżej progu jakości: R:R albo szerokość strefy
    SignalQualityTooLow,
    /// sygnał scalony z żywym koszykiem tego samego kierunku zamiast otwarcia
    /// drugiego — to NIE jest odrzucenie, tylko inna droga wykonania
    MergedIntoBasket,
    /// wyjście czeka na drugą stronę spreadu zamiast płacić go od razu
    ExitWaitingForLimit,
    /// czas oczekiwania na lepszą cenę minął — wyjście awaryjne po rynku
    ExitLimitTimeout,
    /// siatka wystawiona ponownie po powrocie ceny do strefy
    GridRearmed,
    /// sygnał idzie pod trend wyższego rzędu (`trend_filter_*`)
    TrendFilter,
    /// koszyk uwolniony od ryzyka regułą silnika: część zysku zabankowana,
    /// runner ze stopem na średniej ważonej cenie wejścia
    RiskFreeArmed,
    /// poziom marginesu ≤ `margin_call_level_pct` — wezwanie do uzupełnienia.
    ///
    /// Osobny kod, nie `EquityFloor`: pod jednym kodem siedziały CZTERY
    /// mechanizmy (podłoga equity presetu, podłoga łańcucha, margin-call,
    /// `ml_min_wejscie`) i w rejestrze nie dało się ich rozróżnić —
    /// precedens F2a (SlHitBrake vs StreakPause).
    MarginCall,
    /// poziom marginesu ≤ `ml_min_wejscie` (bramka wcześniejsza i szersza
    /// niż margin-call; przy `ml_licz_wiszace` liczy też leżące zlecenia)
    MarginLevel,
}

impl RejectCode {
    pub fn as_str(self) -> &'static str {
        match self {
            RejectCode::Halted => "halted",
            RejectCode::MarketSignalWhileLimitOnly => "market_signal_while_limit_only",
            RejectCode::TagFilter => "tag_filter",
            RejectCode::SideFilter => "side_filter",
            RejectCode::RegimeFilter => "regime_filter",
            RejectCode::SlBreached => "sl_breached",
            RejectCode::ChaseTooFar => "chase_too_far",
            RejectCode::SlDistanceTooBig => "sl_distance_too_big",
            RejectCode::SignalGeometryInsane => "signal_geometry_insane",
            RejectCode::EntryGateBlocked => "entry_gate_blocked",
            RejectCode::MaxOpenPositions => "max_open_positions",
            RejectCode::MaxOpenBaskets => "max_open_baskets",
            RejectCode::MaxDirectionalLots => "max_directional_lots",
            RejectCode::ExposureCap => "exposure_cap",
            RejectCode::EquityFloor => "equity_floor",
            RejectCode::SessionClosed => "session_closed",
            RejectCode::StreakPause => "streak_pause",
            RejectCode::SlHitBrake => "sl_hit_brake",
            RejectCode::StaleSignal => "stale_signal",
            RejectCode::TargetReachedWithoutEntry => "target_reached_without_entry",
            RejectCode::RiskBudgetExhausted => "risk_budget_exhausted",
            RejectCode::NoTargetBasket => "no_target_basket",
            RejectCode::TpNotConfirmedByPrice => "tp_not_confirmed_by_price",
            RejectCode::ManagementLevelInsane => "management_level_insane",
            RejectCode::TpStageAlreadyPassed => "tp_stage_already_passed",
            RejectCode::TpIndexOutOfRange => "tp_index_out_of_range",
            RejectCode::BasketNotFound => "basket_not_found",
            RejectCode::BasketDone => "basket_done",
            RejectCode::SppTooOld => "spp_too_old",
            RejectCode::DuplicateEditedAction => "duplicate_edited_action",
            RejectCode::EditOrphan => "edit_orphan",
            RejectCode::BrokerRejected => "broker_rejected",
            RejectCode::NotEnoughMargin => "not_enough_margin",
            RejectCode::InvalidStops => "invalid_stops",
            RejectCode::MarketClosed => "market_closed",
            RejectCode::Manual => "manual",
            RejectCode::DisabledBySetting => "disabled_by_setting",
            RejectCode::DailyBudgetSpent => "daily_budget_spent",
            RejectCode::SignalQualityTooLow => "signal_quality_too_low",
            RejectCode::MergedIntoBasket => "merged_into_basket",
            RejectCode::ExitWaitingForLimit => "exit_waiting_for_limit",
            RejectCode::ExitLimitTimeout => "exit_limit_timeout",
            RejectCode::GridRearmed => "grid_rearmed",
            RejectCode::TrendFilter => "trend_filter",
            RejectCode::RiskFreeArmed => "risk_free_armed",
            RejectCode::MarginCall => "margin_call",
            RejectCode::MarginLevel => "margin_level",
        }
    }
}

impl From<crate::broker::BrokerError> for RejectCode {
    fn from(e: crate::broker::BrokerError) -> Self {
        use crate::broker::BrokerError as E;
        match e {
            E::InvalidStops | E::WrongSide => RejectCode::InvalidStops,
            E::NotEnoughMargin => RejectCode::NotEnoughMargin,
            E::MarketClosed => RejectCode::MarketClosed,
            _ => RejectCode::BrokerRejected,
        }
    }
}

/// Nazwa powodu zamknięcia w postaci, którą da się grupować w `jq`.
pub fn close_reason_str(r: CloseReason) -> &'static str {
    match r {
        CloseReason::Tp => "Tp",
        CloseReason::Sl => "Sl",
        CloseReason::VirtualSl => "VirtualSl",
        CloseReason::Manual => "Manual",
        CloseReason::Partial => "Partial",
        CloseReason::RiskFree => "RiskFree",
        CloseReason::OutAtEntry => "OutAtEntry",
        CloseReason::Harvest => "Harvest",
        CloseReason::Stale => "Stale",
        CloseReason::Trail => "Trail",
        CloseReason::BasketClose => "BasketClose",
        CloseReason::EodFlat => "EodFlat",
        CloseReason::DayTarget => "DayTarget",
        CloseReason::MaxDd => "MaxDd",
        CloseReason::Ai => "Ai",
        CloseReason::Expired => "Expired",
        CloseReason::RevExit => "RevExit",
    }
}

// ============================================================
//  MIGAWKA STANU
// ============================================================

/// Zaokrąglenie do 4 miejsc. Robimy je RAZ, przy budowie zdarzenia — dzięki
/// temu zapis i odczyt dają bit w bit tę samą liczbę, a plik nie puchnie od
/// `0.30000000000000004`.
#[inline]
pub fn r4(v: f64) -> f64 {
    if v.is_finite() {
        (v * 10_000.0).round() / 10_000.0
    } else {
        0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct MarketSnapshot {
    pub bid: f64,
    pub ask: f64,
    pub spread: f64,
    pub equity: f64,
    pub balance: f64,
    /// wykorzystany depozyt zabezpieczający
    pub margin_used: f64,
    pub free_margin: f64,
    /// liczba otwartych pozycji
    pub open_positions: u32,
    /// liczba niezafillowanych zleceń oczekujących
    pub open_pendings: u32,
    /// suma wolumenu otwartych pozycji (loty)
    pub open_volume: f64,
    /// suma wolumenu netto ze znakiem (BUY dodatnio)
    pub net_volume: f64,
    /// niezrealizowany wynik otwartych pozycji
    pub floating: f64,
    /// bieżące obsunięcie od szczytu equity, w dolarach
    pub dd_abs: f64,
    /// to samo w procentach szczytu
    pub dd_pct: f64,
    /// wynik zrealizowany od początku doby handlowej
    pub realized_today: f64,
}

impl MarketSnapshot {
    /// Buduje migawkę z kwotowania, konta i listy pozycji.
    pub fn build(
        q: &Quote,
        acc: &Account,
        positions: &[Position],
        pendings_n: usize,
        peak_equity: f64,
        realized_today: f64,
    ) -> Self {
        let mut vol = 0.0;
        let mut net = 0.0;
        let mut floating = 0.0;
        for p in positions {
            vol += p.volume;
            net += p.volume * p.side.sign();
            floating += p.profit_usd(q);
        }
        let dd = (peak_equity - acc.equity).max(0.0);
        MarketSnapshot {
            bid: r4(q.bid),
            ask: r4(q.ask),
            spread: r4(q.spread()),
            equity: r4(acc.equity),
            balance: r4(acc.balance),
            margin_used: r4(acc.margin),
            free_margin: r4(acc.free_margin),
            open_positions: positions.len() as u32,
            open_pendings: pendings_n as u32,
            open_volume: r4(vol),
            net_volume: r4(net),
            floating: r4(floating),
            dd_abs: r4(dd),
            dd_pct: r4(dd / peak_equity.max(1.0) * 100.0),
            realized_today: r4(realized_today),
        }
    }
}

// ============================================================
//  WYCHYLENIA CENY (MFE / MAE)
// ============================================================

/// Największe korzystne i największe niekorzystne wychylenie ceny w czasie
/// życia pozycji.
///
/// Bez tego nie da się odpowiedzieć na pytanie „czy dało się zamknąć lepiej" —
/// a to jest jedyne pytanie, które mówi, ile kosztuje dana reguła zarządzania.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Excursion {
    /// najlepsza cena WYJŚCIA, jaka wystąpiła w trakcie życia pozycji
    pub mfe_price: f64,
    /// najgorsza cena wyjścia
    pub mae_price: f64,
    /// szczyt w punktach ceny, na LOT — miara pierwotna, niezależna od
    /// wolumenu; dzięki temu częściowe zamknięcie nie psuje rachunku
    pub mfe_pts: f64,
    pub mae_pts: f64,
    /// wynik, jaki dałoby zamknięcie w najlepszym momencie (brutto, w $),
    /// przeliczony na WOLUMEN TEJ TRANSZY
    pub mfe_usd: f64,
    /// wynik przy zamknięciu w najgorszym momencie (ujemny, w $)
    pub mae_usd: f64,
    /// kiedy padł szczyt (zegar serwera brokera, ms)
    pub mfe_ts: Ts,
    pub mae_ts: Ts,
    /// ile ticków objęła obserwacja — 0 znaczy „nie mierzono", i wtedy
    /// analizator NIE liczy tej pozycji do „zostawionego na stole"
    pub samples: u64,
}

/// Stan śledzenia jednej pozycji między tickami.
///
/// Mierzymy w PUNKTACH CENY, nie w dolarach. Powodem jest częściowe
/// zamknięcie: pozycja zachowuje wtedy ten sam numer, ale kurczy się co do
/// wolumenu. Gdyby szczyt był zapamiętany w dolarach dla starego rozmiaru,
/// „zostawione na stole” dla transzy wychodziłoby zawyżone o już zamkniętą
/// część. Punkty przelicza się na dolary dopiero przy zapisie — wolumenem
/// tej konkretnej transakcji.
#[derive(Debug, Clone, Copy)]
struct ExcState {
    mfe_pts: f64,
    mae_pts: f64,
    mfe_price: f64,
    mae_price: f64,
    mfe_ts: Ts,
    mae_ts: Ts,
    samples: u64,
}

// ============================================================
//  ZDARZENIE
// ============================================================

/// Jedna zamknięta noga w awaryjnym zamknięciu — „co dokładnie i po ile".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosedLeg {
    pub ticket: Ticket,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basket_id: Option<u32>,
    pub side: String,
    pub volume: f64,
    pub open_price: f64,
    pub close_price: f64,
    pub profit: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloseDetail {
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub profit_basis: Option<crate::cost_receipt::ProfitBasis>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub cost_receipt: Option<Box<crate::cost_receipt::CostReceipt>>,
    pub side: String,
    pub volume: f64,
    pub open_price: f64,
    pub close_price: f64,
    /// zegar serwera brokera, ms epoki
    pub open_ts: Ts,
    pub close_ts: Ts,
    /// czas trzymania pozycji w sekundach
    pub hold_s: f64,
    /// wynik brutto (bez prowizji i swapu)
    pub gross: f64,
    pub commission: f64,
    pub swap: f64,
    /// wynik netto — to, co naprawdę weszło na saldo
    pub net: f64,
    pub reason: String,
    pub excursion: Excursion,
    /// `mfe_usd − net`, obcięte do zera: ile pieniędzy zostało na stole
    pub left_on_table: f64,
}

impl CloseDetail {
    pub fn new(t: &ClosedTrade, exc: Excursion) -> Result<Self,crate::cost_receipt::CostError> {
        let (gross,net)=if t.profit_basis==Some(crate::cost_receipt::ProfitBasis::CanonicalClosedNetV1) {
            (r4(t.cost_receipt.as_ref().ok_or(crate::cost_receipt::CostError::MissingReceipt)?.gross_profit
                .ok_or(crate::cost_receipt::CostError::MissingComponent(crate::cost_receipt::CostComponent::GrossProfit))?),
             r4(t.canonical_net()?))
        } else {
            // Historical output preserved byte-for-byte. Source-defined, not a
            // declaration that these old fields represented complete costs.
            (r4(t.profit),r4(t.profit-t.commission.abs()+t.swap))
        };

        let mut exc = exc;
        if exc.samples > 0 && gross > exc.mfe_usd {
            exc.mfe_usd = gross;
            exc.mfe_price = r4(t.close_price);
            exc.mfe_pts = r4((t.close_price - t.open_price) * t.side.sign());
            exc.mfe_ts = t.close_ts;
        }

        let left = if exc.samples > 0 {
            r4((exc.mfe_usd - net).max(0.0))
        } else {
            0.0
        };
        Ok(CloseDetail {
            profit_basis:t.profit_basis,cost_receipt:t.cost_receipt.clone(),
            side: match t.side {
                Side::Buy => "Buy".to_string(),
                Side::Sell => "Sell".to_string(),
            },
            volume: r4(t.volume),
            open_price: r4(t.open_price),
            close_price: r4(t.close_price),
            open_ts: t.open_ts,
            close_ts: t.close_ts,
            hold_s: r4((t.close_ts - t.open_ts) as f64 / 1000.0),
            gross,
            commission: r4(t.commission),
            swap: r4(t.swap),
            net,
            reason: close_reason_str(t.reason).to_string(),
            excursion: exc,
            left_on_table: left,
        })
    }
}

/// Jedno zdarzenie dziennika = jedna linia pliku `.jsonl`.
///
/// Pola opcjonalne znikają z zapisu, więc linia „przyszła wiadomość" nie waży
/// tyle co linia zamknięcia pozycji. To nie jest oszczędność dla samej
/// oszczędności: plik, którego nie da się przewinąć wzrokiem, przestaje być
/// czytelny dla człowieka.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEvent {
    /// wersja schematu — analizator sprawdza ją przed liczeniem
    pub v: u32,
    /// stabilny identyfikator zdarzenia: `{run_id}#{numer}`
    pub event_id: String,
    /// **Zegar ścienny** procesu, ISO 8601 z datą i strefą.
    /// Stempluje go warstwa zapisu; rdzeń nie ma prawa czytać zegara.
    pub ts: String,
    /// ten sam znacznik w ms epoki UTC — do sortowania bez parsowania
    pub ts_ms: i64,
    /// **Zegar serwera brokera** (czas ticka), ISO 8601 z datą i strefą.
    /// To DRUGI, niezależny zegar. Mylenie go z zegarem ściennym raz już
    /// kosztowało nas cały wynik backtestu, więc ma osobne pole.
    pub ts_broker: String,
    /// surowy znacznik ticka w ms — dokładnie to, co dostał silnik
    pub ts_broker_ms: Ts,
    pub level: EventLevel,
    pub category: EventCategory,
    pub kind: EventKind,
    /// doba handlowa serwera w postaci `YYYY-MM-DD` — klucz grupowania
    pub session_day: String,

    // ---- identyfikatory (zawsze, gdy zdarzenie ich dotyczy) ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basket_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<Ticket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    // ---- treść ----
    /// jedno zdanie po polsku — to jest wersja „dla oka"
    pub text: String,
    /// powód z ZAMKNIĘTEJ listy; obowiązkowy przy odrzuceniach i pominięciach
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<RejectCode>,
    /// migawka stanu w chwili decyzji
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<MarketSnapshot>,
    /// komplet danych o zamknięciu pozycji
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<CloseDetail>,
    /// co dokładnie zamknął strażnik kapitału
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legs: Vec<ClosedLeg>,
    /// pola dodatkowe, zależne od rodzaju zdarzenia
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub data: serde_json::Map<String, serde_json::Value>,
}

impl Default for JournalEvent {
    fn default() -> Self {
        JournalEvent {
            v: JOURNAL_SCHEMA,
            event_id: String::new(),
            ts: String::new(),
            ts_ms: 0,
            ts_broker: String::new(),
            ts_broker_ms: 0,
            level: EventLevel::Info,
            category: EventCategory::System,
            kind: EventKind::Note,
            session_day: String::new(),
            basket_id: None,
            ticket: None,
            msg_id: None,
            signal_id: None,
            source: None,
            text: String::new(),
            reason: None,
            market: None,
            close: None,
            legs: Vec::new(),
            data: serde_json::Map::new(),
        }
    }
}

impl JournalEvent {
    /// Dokleja zegar ścienny. Wołane WYŁĄCZNIE przez warstwę zapisu — rdzeń
    /// zostawia tu zera, bo nie ma dostępu do zegara.
    pub fn stamp_wall(&mut self, wall_utc_ms: i64, local_offset_ms: i64) {
        self.ts_ms = wall_utc_ms;
        self.ts = iso8601(wall_utc_ms, local_offset_ms);
    }

    /// Linia dla człowieka — to, co ląduje w lustrzanym pliku `.log`.
    ///
    /// **Jedno zdarzenie = jedna linia, bez wyjątków.** Treść wiadomości
    /// z Telegrama bywa wielolinijkowa; w `.jsonl` chroni nas escapowanie
    /// JSON-a, ale w pliku tekstowym surowy `\n` rozbiłby jedno zdarzenie na
    /// kilka wierszy i `grep -c` przestałby liczyć zdarzenia. Dlatego łamania
    /// linii zamieniamy na widoczny znak ⏎.
    pub fn human(&self) -> String {
        let mut s = format!(
            "{} [{}/{}] {}",
            self.ts_broker,
            self.category.as_str(),
            self.level.as_str(),
            self.kind.as_str()
        );
        if let Some(b) = self.basket_id {
            s.push_str(&format!(" B{b}"));
        }
        if let Some(t) = self.ticket {
            s.push_str(&format!(" #{t}"));
        }
        if let Some(m) = self.msg_id {
            s.push_str(&format!(" msg{m}"));
        }
        if let Some(r) = self.reason {
            s.push_str(&format!(" reason={}", r.as_str()));
        }
        s.push_str(" · ");
        s.push_str(&jedna_linia(&self.text));
        if let Some(c) = &self.close {
            s.push_str(&format!(
                " | {} {:.2} lota {:.2}→{:.2} netto {:+.2} $ ({}) trzymane {:.0} s · MFE {:+.2} $ · zostawione {:.2} $",
                c.side, c.volume, c.open_price, c.close_price, c.net, c.reason, c.hold_s,
                c.excursion.mfe_usd, c.left_on_table
            ));
        }
        if let Some(m) = &self.market {
            s.push_str(&format!(
                " | bid {:.2} ask {:.2} spread {:.2} eq {:.2} $ poz {} vol {:.2} DD {:.2} $",
                m.bid, m.ask, m.spread, m.equity, m.open_positions, m.open_volume, m.dd_abs
            ));
        }
        s
    }
}

/// Spłaszcza tekst do jednej linii (dla lustrzanego pliku `.log`).
fn jedna_linia(t: &str) -> String {
    let mut s = String::with_capacity(t.len());
    let mut poprzedni_lamacz = false;
    for c in t.chars() {
        match c {
            '\n' | '\r' => {
                if !poprzedni_lamacz {
                    s.push('⏎');
                    poprzedni_lamacz = true;
                }
            }
            '\t' => {
                s.push(' ');
                poprzedni_lamacz = false;
            }
            _ => {
                s.push(c);
                poprzedni_lamacz = false;
            }
        }
    }
    s
}

// ============================================================
//  FORMATOWANIE CZASU
// ============================================================

pub fn iso8601(instant_utc_ms: i64, offset_ms: i64) -> String {
    use chrono::{DateTime, FixedOffset, TimeZone};
    let secs = offset_ms.div_euclid(1000) as i32;
    let tz = FixedOffset::east_opt(secs).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    match DateTime::from_timestamp_millis(instant_utc_ms) {
        Some(dt) => tz
            .from_utc_datetime(&dt.naive_utc())
            .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
            .to_string(),
        None => String::from("1970-01-01T00:00:00.000+00:00"),
    }
}

/// Znacznik ticka (już wyrażony w czasie serwera) jako ISO 8601 z tą strefą.
///
/// `ticks.bin` trzyma czas serwera brokera „udający epokę", więc prawdziwy
/// moment to `broker_ms − offset`. Renderujemy go w strefie serwera, dzięki
/// czemu w pliku widać dokładnie tę godzinę, którą pokazuje MetaTrader —
/// i jednocześnie widać, że to NIE jest UTC.
pub fn iso8601_broker(broker_ms: Ts, server_offset_ms: i64) -> String {
    iso8601(broker_ms - server_offset_ms, server_offset_ms)
}

/// Doba handlowa serwera jako `YYYY-MM-DD`.
///
/// Liczona z surowego znacznika ticka i `session_offset` silnika (który dla
/// naszych danych wynosi 0, bo ticki są już w czasie serwera). To jest klucz,
/// po którym rotuje się plik i po którym analizator grupuje wyniki.
pub fn session_day_str(broker_ms: Ts, session_offset_ms: i64) -> String {
    use chrono::DateTime;
    let d = day_of(broker_ms, session_offset_ms);
    match DateTime::from_timestamp(d * 86_400, 0) {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => String::from("1970-01-01"),
    }
}

// ============================================================
//  BUFOR
// ============================================================

/// Konfiguracja dziennika widziana przez rdzeń.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalConfig {
    pub enabled: bool,
    pub min_level: EventLevel,
    /// dołączaj migawkę stanu do decyzji (koszt: ~200 B na zdarzenie)
    pub snapshots: bool,
    /// mierz MFE/MAE każdej pozycji
    pub excursions: bool,
    /// offset strefy serwera brokera (do renderowania `ts_broker`)
    pub server_offset_ms: i64,
    /// offset doby handlowej (do rotacji i grupowania)
    pub session_offset_ms: i64,
    /// ile zdarzeń wolno trzymać w buforze, zanim najstarsze przepadną
    pub cap: usize,
}

impl Default for JournalConfig {
    fn default() -> Self {
        JournalConfig {
            enabled: false,
            min_level: EventLevel::Info,
            snapshots: true,
            excursions: true,
            server_offset_ms: 3 * 3_600_000,
            session_offset_ms: 0,
            cap: 20_000,
        }
    }
}

#[derive(Debug)]
pub struct JournalBuf {
    pub cfg: JournalConfig,
    /// prefiks identyfikatorów zdarzeń — jeden przebieg = jeden prefiks
    pub run_id: String,
    seq: u64,
    events: Vec<JournalEvent>,
    exc: HashMap<Ticket, ExcState>,
    /// ile zdarzeń przepadło przez przepełnienie bufora — jawnie, bo cicha
    /// utrata linii dziennika jest gorsza niż jej brak
    pub dropped: u64,
}

impl Default for JournalBuf {
    fn default() -> Self {
        JournalBuf::new(JournalConfig::default(), "run")
    }
}

impl JournalBuf {
    pub fn new(cfg: JournalConfig, run_id: impl Into<String>) -> Self {
        JournalBuf {
            cfg,
            run_id: run_id.into(),
            seq: 0,
            events: Vec::new(),
            exc: HashMap::new(),
            dropped: 0,
        }
    }

    #[inline]
    pub fn enabled(&self) -> bool {
        self.cfg.enabled
    }

    /// Czy zdarzenie o tym poziomie w ogóle powstanie?
    /// Sprawdzane PRZED zbudowaniem treści, żeby wyłączony dziennik nie
    /// kosztował ani jednej alokacji.
    #[inline]
    pub fn wants(&self, level: EventLevel) -> bool {
        self.cfg.enabled && level >= self.cfg.min_level
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Nadaje `event_id`, uzupełnia znaczniki i wkłada zdarzenie do bufora.
    pub fn push(&mut self, mut ev: JournalEvent) {
        if !self.wants(ev.level) {
            return;
        }
        self.seq += 1;
        ev.v = JOURNAL_SCHEMA;
        ev.event_id = format!("{}#{:08}", self.run_id, self.seq);
        ev.ts_broker = iso8601_broker(ev.ts_broker_ms, self.cfg.server_offset_ms);
        ev.session_day = session_day_str(ev.ts_broker_ms, self.cfg.session_offset_ms);
        if !self.cfg.snapshots {
            ev.market = None;
        }
        if self.events.len() >= self.cfg.cap {
            let drop = self.cfg.cap / 4 + 1;
            self.events.drain(0..drop);
            self.dropped += drop as u64;
        }
        self.events.push(ev);
    }

    /// Zabiera wszystko, co się nazbierało. Pętla woła to i oddaje do zapisu.
    pub fn drain(&mut self) -> Vec<JournalEvent> {
        std::mem::take(&mut self.events)
    }

    /// Podgląd bez zabierania — do testów i do panelu.
    pub fn peek(&self) -> &[JournalEvent] {
        &self.events
    }

    // -------- MFE / MAE --------

    /// Aktualizuje wychylenia wszystkich otwartych pozycji.
    ///
    /// Wołane raz na tick. Cena brana jest po stronie WYJŚCIA (BUY wychodzi po
    /// bid), więc `mfe_usd` to naprawdę „tyle dałoby się wyjąć", a nie liczba
    /// z połowy spreadu.
    pub fn track(&mut self, positions: &[Position], q: &Quote) {
        if !self.cfg.enabled || !self.cfg.excursions {
            return;
        }
        for p in positions {
            let px = q.exit(p.side);
            let pts = (px - p.open_price) * p.side.sign();
            let st = self.exc.entry(p.ticket).or_insert(ExcState {
                mfe_pts: pts,
                mae_pts: pts,
                mfe_price: px,
                mae_price: px,
                mfe_ts: q.ts,
                mae_ts: q.ts,
                samples: 0,
            });
            st.samples += 1;
            if pts > st.mfe_pts {
                st.mfe_pts = pts;
                st.mfe_price = px;
                st.mfe_ts = q.ts;
            }
            if pts < st.mae_pts {
                st.mae_pts = pts;
                st.mae_price = px;
                st.mae_ts = q.ts;
            }
        }
    }

    /// Odczytuje i USUWA wychylenia zamkniętej pozycji.
    ///
    /// Zwraca `Excursion` z `samples = 0`, gdy pozycji nie śledzono (np.
    /// otwarcie i zamknięcie w tym samym ticku). Analizator odróżnia ten
    /// przypadek od „zerowego wychylenia" i nie wlicza go do statystyk.
    pub fn take_excursion(&mut self, t: Ticket, volume: f64) -> Excursion {
        match self.exc.remove(&t) {
            Some(st) => finish(st, volume),
            None => Excursion::default(),
        }
    }

    pub fn peek_excursion(&self, t: Ticket, volume: f64) -> Excursion {
        match self.exc.get(&t) {
            Some(st) => finish(*st, volume),
            None => Excursion::default(),
        }
    }

    /// Czy ta pozycja jest już śledzona? Po tym poznajemy NOWE otwarcie.
    #[inline]
    pub fn knows(&self, t: Ticket) -> bool {
        self.exc.contains_key(&t)
    }

    /// Zapomina pozycje, których broker już nie ma — inaczej mapa rośnie bez
    /// końca przy pozycjach zamkniętych poza silnikiem (ręcznie w MT5).
    pub fn forget_missing(&mut self, live: &[Position]) {
        if self.exc.len() < 256 {
            return;
        }
        let set: std::collections::HashSet<Ticket> = live.iter().map(|p| p.ticket).collect();
        self.exc.retain(|t, _| set.contains(t));
    }

    pub fn note_open(&mut self, p: &Position, q: &Quote) {
        if !self.cfg.enabled || !self.cfg.excursions {
            return;
        }
        let px = q.exit(p.side);
        let pts = (px - p.open_price) * p.side.sign();
        self.exc.insert(
            p.ticket,
            ExcState {
                mfe_pts: pts,
                mae_pts: pts,
                mfe_price: px,
                mae_price: px,
                mfe_ts: q.ts,
                mae_ts: q.ts,
                samples: 1,
            },
        );
    }
}

fn finish(st: ExcState, volume: f64) -> Excursion {
    let k = XAU_CONTRACT * volume;
    Excursion {
        mfe_price: r4(st.mfe_price),
        mae_price: r4(st.mae_price),
        mfe_pts: r4(st.mfe_pts),
        mae_pts: r4(st.mae_pts),
        mfe_usd: r4(st.mfe_pts * k),
        mae_usd: r4(st.mae_pts * k),
        mfe_ts: st.mfe_ts,
        mae_ts: st.mae_ts,
        samples: st.samples,
    }
}

// ============================================================
//  BUDOWANIE ZDARZEŃ — skróty
// ============================================================

/// Konstruktor zdarzenia. Wymusza podanie czasu ticka, poziomu, kategorii
/// i rodzaju — czyli tego, bez czego linia dziennika jest bezużyteczna.
pub struct Ev(JournalEvent);

impl Ev {
    pub fn new(
        ts_broker_ms: Ts,
        level: EventLevel,
        category: EventCategory,
        kind: EventKind,
    ) -> Self {
        Ev(JournalEvent {
            ts_broker_ms,
            level,
            category,
            kind,
            ..Default::default()
        })
    }
    pub fn text(mut self, t: impl Into<String>) -> Self {
        self.0.text = t.into();
        self
    }
    pub fn basket(mut self, id: u32) -> Self {
        self.0.basket_id = Some(id);
        self
    }
    pub fn basket_opt(mut self, id: Option<u32>) -> Self {
        self.0.basket_id = id;
        self
    }
    pub fn ticket(mut self, t: Ticket) -> Self {
        self.0.ticket = Some(t);
        self
    }
    pub fn msg(mut self, id: i64) -> Self {
        self.0.msg_id = Some(id);
        self
    }
    pub fn signal(mut self, id: impl Into<String>) -> Self {
        self.0.signal_id = Some(id.into());
        self
    }
    pub fn source(mut self, s: impl Into<String>) -> Self {
        self.0.source = Some(s.into());
        self
    }
    pub fn reason(mut self, r: RejectCode) -> Self {
        self.0.reason = Some(r);
        self
    }
    pub fn market(mut self, m: Option<MarketSnapshot>) -> Self {
        self.0.market = m;
        self
    }
    pub fn close(mut self, c: CloseDetail) -> Self {
        self.0.close = Some(c);
        self
    }
    pub fn legs(mut self, l: Vec<ClosedLeg>) -> Self {
        self.0.legs = l;
        self
    }
    pub fn put(mut self, k: &str, v: impl Into<serde_json::Value>) -> Self {
        self.0.data.insert(k.to_string(), v.into());
        self
    }
    pub fn put_f(self, k: &str, v: f64) -> Self {
        self.put(k, r4(v))
    }
    pub fn build(self) -> JournalEvent {
        self.0
    }
}

/// Stabilny identyfikator akcji z wiadomości: `msg_id:action_key`.
/// Pozwala złączyć „przyszła wiadomość" z „utworzono koszyk" i z „odrzucono"
/// bez zgadywania po czasie.
#[inline]
pub fn signal_id(msg_id: i64, action_key: &str) -> String {
    format!("{msg_id}:{action_key}")
}

// ============================================================
//  TESTY
// ============================================================

#[cfg(test)]
mod testy {
    use super::*;

    fn q(ts: Ts, bid: f64, ask: f64) -> Quote {
        Quote { ts, bid, ask }
    }

    fn poz(ticket: Ticket, side: Side, vol: f64, open: f64) -> Position {
        Position {
            ticket,
            side,
            volume: vol,
            open_price: open,
            open_ts: 0,
            sl: None,
            tp: None,
            vsl: None,
            basket: Some(5),
            level: 0,
            frozen: false,
            peak_pts: 0.0,
            last_peak_ts: 0,
            is_runner: false,
            is_toucher: false,
            comment: String::new(),
        }
    }

    fn cfg_on() -> JournalConfig {
        JournalConfig {
            enabled: true,
            min_level: EventLevel::Debug,
            ..Default::default()
        }
    }

    /// Serializacja w obie strony: co zapisaliśmy, to odczytujemy — bit w bit.
    #[test]
    fn zdarzenie_przezywa_podroz_do_json_i_z_powrotem() {
        let mut j = JournalBuf::new(cfg_on(), "test");
        let trade = ClosedTrade {
            profit_basis: None, cost_receipt: None,
            ticket: 77,
            side: Side::Buy,
            volume: 0.03,
            open_price: 3300.15,
            close_price: 3312.40,
            open_ts: 1_753_000_000_000,
            close_ts: 1_753_000_600_000,
            profit: 36.75,
            commission: 0.9,
            swap: -0.15,
            reason: CloseReason::Tp,
            basket: Some(5),
        };
        let exc = Excursion {
            mfe_price: 3320.0,
            mae_price: 3295.0,
            mfe_pts: 19.85,
            mae_pts: -5.15,
            mfe_usd: 59.55,
            mae_usd: -15.45,
            mfe_ts: 1_753_000_300_000,
            mae_ts: 1_753_000_100_000,
            samples: 412,
        };
        j.push(
            Ev::new(
                1_753_000_600_000,
                EventLevel::Ok,
                EventCategory::Trade,
                EventKind::PositionClosed,
            )
            .text("cel 1 zrealizowany")
            .basket(5)
            .ticket(77)
            .msg(9001)
            .signal(signal_id(9001, "tp1"))
            .source("ATFX VIP SIGNALS")
            .market(Some(MarketSnapshot {
                bid: 3312.40,
                ask: 3312.62,
                spread: 0.22,
                equity: 236.75,
                balance: 236.75,
                margin_used: 19.87,
                free_margin: 216.88,
                open_positions: 2,
                open_pendings: 3,
                open_volume: 0.05,
                net_volume: 0.05,
                floating: -1.25,
                dd_abs: 4.5,
                dd_pct: 1.87,
                realized_today: 36.75,
            }))
            .close(CloseDetail::new(&trade, exc).unwrap())
            .put("stage", 1u64)
            .build(),
        );

        let ev = j.peek()[0].clone();
        let line = serde_json::to_string(&ev).unwrap();
        // jedna linia = jedno zdarzenie: znak nowej linii w środku zabiłby format
        assert!(!line.contains('\n'));
        let back: JournalEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(ev, back);

        // pola, na których stoi cała analiza — sprawdzane wprost
        assert_eq!(back.event_id, "test#00000001");
        assert_eq!(back.kind, EventKind::PositionClosed);
        assert_eq!(back.basket_id, Some(5));
        assert_eq!(back.ticket, Some(77));
        assert_eq!(back.msg_id, Some(9001));
        assert_eq!(back.signal_id.as_deref(), Some("9001:tp1"));
        let c = back.close.unwrap();
        assert_eq!(c.volume, 0.03);
        assert_eq!(c.reason, "Tp");
        // netto = brutto − prowizja + swap
        assert!((c.net - (36.75 - 0.9 - 0.15)).abs() < 1e-9);
        // na stole: MFE 59.55 − netto 35.70
        assert!((c.left_on_table - (59.55 - 35.70)).abs() < 1e-6);
    }

    #[test]
    fn znacznik_ma_date_i_strefe() {
        // 2026-07-23 12:00:00 UTC
        let utc_ms = 1_784_808_000_000i64;
        let s = iso8601(utc_ms, 3 * 3_600_000);
        assert!(s.starts_with("2026-07-23T15:00:00.000"), "{s}");
        assert!(s.ends_with("+03:00"), "{s}");

        // znacznik ticka jest ZAPISANY w czasie serwera, więc renderuje się
        // dokładnie tak, jak pokazuje MetaTrader — ale z jawną strefą
        let broker_ms = utc_ms + 3 * 3_600_000;
        let b = iso8601_broker(broker_ms, 3 * 3_600_000);
        assert!(b.starts_with("2026-07-23T15:00:00.000"), "{b}");
        assert!(b.ends_with("+03:00"), "{b}");
        assert_eq!(session_day_str(broker_ms, 0), "2026-07-23");
    }

    /// MFE/MAE liczone po stronie WYJŚCIA i wobec bieżącego wolumenu.
    #[test]
    fn mfe_i_mae_liczone_poprawnie() {
        let mut j = JournalBuf::new(cfg_on(), "t");
        // BUY 0.10 lota otwarte po 3300.00; 1 $ ruchu = 0.10 × 100 = 10 $
        let p = poz(1, Side::Buy, 0.10, 3300.00);
        j.note_open(&p, &q(0, 3300.00, 3300.20));

        // w górę do bid 3305 → +50 $
        j.track(std::slice::from_ref(&p), &q(1000, 3305.00, 3305.20));
        // w dół do bid 3297 → −30 $
        j.track(std::slice::from_ref(&p), &q(2000, 3297.00, 3297.20));
        // z powrotem do 3302 → +20 $ (nie zmienia ani szczytu, ani dna)
        j.track(std::slice::from_ref(&p), &q(3000, 3302.00, 3302.20));

        let e = j.take_excursion(1, 0.10);
        assert!((e.mfe_usd - 50.0).abs() < 1e-6, "MFE = {}", e.mfe_usd);
        assert!((e.mae_usd + 30.0).abs() < 1e-6, "MAE = {}", e.mae_usd);
        assert_eq!(e.mfe_price, 3305.00);
        assert_eq!(e.mae_price, 3297.00);
        assert_eq!(e.mfe_ts, 1000);
        assert_eq!(e.mae_ts, 2000);
        assert_eq!(e.samples, 4);
        // po odczycie pozycja przestaje być śledzona
        assert!(!j.knows(1));

        // SELL: znaki odwrócone, cena wyjścia to ASK
        let s = poz(2, Side::Sell, 0.10, 3300.00);
        j.note_open(&s, &q(0, 3299.80, 3300.00));
        j.track(std::slice::from_ref(&s), &q(1000, 3289.80, 3290.00)); // ask 3290 → +100 $
        j.track(std::slice::from_ref(&s), &q(2000, 3304.80, 3305.00)); // ask 3305 → −50 $
        let e2 = j.take_excursion(2, 0.10);
        assert!(
            (e2.mfe_usd - 100.0).abs() < 1e-6,
            "MFE SELL = {}",
            e2.mfe_usd
        );
        assert!(
            (e2.mae_usd + 50.0).abs() < 1e-6,
            "MAE SELL = {}",
            e2.mae_usd
        );
    }

    /// „Nie mierzono" musi być odróżnialne od „zero wychylenia".
    #[test]
    fn brak_pomiaru_to_nie_zero() {
        let mut j = JournalBuf::new(cfg_on(), "t");
        let e = j.take_excursion(999, 0.01);
        assert_eq!(e.samples, 0);

        let trade = ClosedTrade {
            profit_basis: None, cost_receipt: None,
            ticket: 999,
            side: Side::Buy,
            volume: 0.01,
            open_price: 3300.0,
            close_price: 3290.0,
            open_ts: 0,
            close_ts: 1000,
            profit: -10.0,
            commission: 0.0,
            swap: 0.0,
            reason: CloseReason::Sl,
            basket: None,
        };
        let d = CloseDetail::new(&trade, e).unwrap();
        assert_eq!(d.left_on_table, 0.0);
    }

    /// Wyłączony dziennik nie produkuje niczego, a filtr poziomu działa.
    #[test]
    fn filtr_poziomu_i_wylaczenie() {
        let mut j = JournalBuf::new(JournalConfig::default(), "t"); // enabled = false
        assert!(!j.wants(EventLevel::Error));
        j.push(
            Ev::new(
                0,
                EventLevel::Error,
                EventCategory::Risk,
                EventKind::RiskStop,
            )
            .build(),
        );
        assert!(j.is_empty());

        let mut j = JournalBuf::new(
            JournalConfig {
                enabled: true,
                min_level: EventLevel::Warn,
                ..Default::default()
            },
            "t",
        );
        assert!(!j.wants(EventLevel::Info));
        assert!(j.wants(EventLevel::Warn));
        j.push(Ev::new(0, EventLevel::Info, EventCategory::System, EventKind::Note).build());
        j.push(
            Ev::new(
                0,
                EventLevel::Error,
                EventCategory::Risk,
                EventKind::RiskStop,
            )
            .build(),
        );
        assert_eq!(j.len(), 1);
        assert_eq!(j.peek()[0].kind, EventKind::RiskStop);
    }

    /// Lustro tekstowe MUSI mieć jedno zdarzenie w jednej linii.
    ///
    /// Wiadomości z Telegrama są wielolinijkowe; bez spłaszczenia jedno
    /// zdarzenie rozpadało się na kilkanaście wierszy i `wc -l` na pliku
    /// `.log` pokazywał trzy razy więcej „zdarzeń", niż naprawdę było.
    #[test]
    fn lustro_tekstowe_ma_jedna_linie_na_zdarzenie() {
        let mut j = JournalBuf::new(cfg_on(), "t");
        j.push(
            Ev::new(
                0,
                EventLevel::Info,
                EventCategory::Signal,
                EventKind::MessageReceived,
            )
            .text("BUY GOLD 3300-3305\nSL 3290\nTP1 3310\r\nTP2 3320")
            .msg(42)
            .build(),
        );
        let h = j.peek()[0].human();
        assert!(!h.contains('\n'), "linia zawiera łamanie: {h}");
        assert!(!h.contains('\r'));
        assert!(
            h.contains("BUY GOLD 3300-3305⏎SL 3290⏎TP1 3310⏎TP2 3320"),
            "{h}"
        );
        // a w JSON-ie oryginał zostaje nietknięty
        assert!(j.peek()[0].text.contains('\n'));
    }

    /// Powód odrzucenia jest KODEM, nie zdaniem — da się go zliczyć.
    #[test]
    fn powod_jest_kodem_z_zamknietej_listy() {
        let ev = Ev::new(
            0,
            EventLevel::Warn,
            EventCategory::Decision,
            EventKind::SignalRejected,
        )
        .reason(RejectCode::SlBreached)
        .text("SL 3290.00 przebity zanim powstało zlecenie")
        .build();
        let s = serde_json::to_string(&ev).unwrap();
        assert!(s.contains("\"reason\":\"sl_breached\""), "{s}");
    }
}
