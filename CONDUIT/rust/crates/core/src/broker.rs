//! Warstwa brokera.
//!
//! Silnik NIGDY nie dotyka MT5 ani symulatora bezpośrednio — rozmawia przez tę
//! cechę. Wspólny interfejs pozwala używać tych samych reguł strategii, ale
//! sam NIE gwarantuje parytetu. Kolejność zdarzeń, opóźnione potwierdzenia,
//! koszty i ograniczenia brokera wymagają osobnych testów zgodności.

use crate::types::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerError {
    /// SL/TP bliżej ceny niż stops level
    InvalidStops,
    /// CENA ZLECENIA OCZEKUJĄCEGO jest niedopuszczalna: albo leży po złej
    /// stronie rynku (limit kupna nad ceną), albo bliżej niej niż stops level.
    ///
    /// To NIE JEST to samo co `InvalidStops`, choć MT5 zwraca oba jako błędy
    /// „ceny". Zlanie ich w jedno kosztowało nas wieczór szukania problemu ze
    /// stop-lossem, którego nie było — odmowa dotyczyła poziomu siatki.
    InvalidPrice,
    /// SL po niewłaściwej stronie ceny
    WrongSide,
    NoSuchTicket,
    NotEnoughMargin,
    InvalidVolume,
    MarketClosed,
    Rejected,
}

pub type BResult<T> = Result<T, BrokerError>;

/// Temporary delivery lag is retryable; uncertain execution requires review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptBarrier { Clear, Temporary, RequiresReview }

/// Verified account scope plus ephemeral transport generation. Never restore
/// this generation from persisted baskets or infer it from a quote timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionSession { pub scope: String, pub generation: u64 }

#[derive(Debug, Clone)]
pub struct OrderReq {
    pub side: Side,
    pub volume: f64,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    pub basket: Option<u32>,
    pub level: i32,
    pub is_toucher: bool,
    pub comment: String,
}

#[derive(Debug, Clone)]
pub struct PendingReq {
    pub kind: PendingKind,
    pub volume: f64,
    pub price: Px,
    pub sl: Option<Px>,
    pub tp: Option<Px>,
    pub basket: Option<u32>,
    pub level: i32,
    pub is_toucher: bool,
    /// patrz `PendingOrder::is_topup`
    pub is_topup: bool,
    pub comment: String,
}

pub trait Broker {
    fn quote(&self) -> Quote;
    fn account(&self) -> Account;
    fn stops_level(&self) -> f64;

    /// Najmniejszy wolumen, ktory broker przyjmuje dla instrumentu.
    ///
    /// Domyslne `0.01` jest jednoczesnie kontraktem zgodnosci dla starych
    /// brokerow/testowych atrap: zanim ta informacja weszla do interfejsu,
    /// caly silnik zakladal siatke 0.01 lota. Implementacja live nadpisuje te
    /// metody danymi `SYMBOL_VOLUME_*` od terminala.
    fn volume_min(&self) -> f64 {
        0.01
    }

    /// Krok siatki wolumenu instrumentu; patrz [`Broker::volume_min`].
    fn volume_step(&self) -> f64 {
        0.01
    }

    /// Maksymalny wolumen jednego zlecenia. NaN oznacza NIEZNANY kontrakt,
    /// którego nowy walidator nie może zgadywać. Stara ścieżka nie odczytuje
    /// tej metody; live podaje rzeczywiste SYMBOL_VOLUME_MAX.
    fn volume_max(&self) -> f64 {
        f64::NAN
    }

    /// Czy broker wymaga potwierdzonego rozliczenia przed kolejnym wejściem.
    /// Capability, nie osobna oś strategii. Opakowania muszą ją przekazywać.
    fn close_receipt_reconciliation_active(&self) -> bool {
        false
    }

    /// Wykonane lub dostarczone zamknięcia nie zostały jeszcze przekazane
    /// właścicielowi do księgowania. Blokuje wyłącznie NOWE wejścia, nigdy
    /// ochronne close/modify/cancel. Sam ACK RPC nie jest rozliczeniem.
    fn close_receipts_pending(&self) -> bool {
        false
    }

    fn receipt_barrier(&self) -> ReceiptBarrier {
        if self.close_receipts_pending() { ReceiptBarrier::Temporary } else { ReceiptBarrier::Clear }
    }

    fn execution_session(&self) -> Option<ExecutionSession> { None }

    /// Verified stable broker position identity. A ticket is only an execution
    /// alias; unknown is None, never a guessed ticket-to-identifier mapping.
    fn position_identifier(&self, _ticket: Ticket) -> Option<u64> { None }

    /// ACK of cancellation alone does not prove that no fill raced with it.
    fn pending_cancel_snapshot_authoritative(&self) -> bool { false }

    /// True only for an ACTIVE, complete canonical-net pipeline in this instance.
    /// An unsupported/disabled/quarantined source must never impersonate net.
    /// This is not proof of durable restart or real-broker tariff fidelity.
    fn cost_net_supported(&self) -> bool { false }
    /// Consumer detected an invalid receipt. Supporting adapters latch an entry
    /// halt account-wide; protective exits remain available. Not a durable ACK.
    fn report_cost_consumer_fault(&mut self, _reason: &str) {}

    fn positions(&self) -> &[Position];
    fn pendings(&self) -> &[PendingOrder];
    fn positions_mut(&mut self) -> &mut Vec<Position>;
    /// Zlecenia oczekujące do modyfikacji NA MIEJSCU.
    ///
    /// Silnik sam z siebie tego nie potrzebuje — składa i kasuje zlecenia
    /// przez `place_pending` / `cancel_pending`. Metoda istnieje dla warstwy
    /// żywej, która przy kilku silnikach na jednym rachunku musi każdemu
    /// pokazać **wyłącznie jego** zlecenia (`crates/app/src/routing.rs`):
    /// bez dostępu do wektora nie da się tego zrobić inaczej niż kopią,
    /// a kopia rozjeżdża się w chwili, gdy silnik dostawi zlecenie w środku
    /// tego samego ticku.
    fn pendings_mut(&mut self) -> &mut Vec<PendingOrder>;

    fn ukryte_pozycje(&self) -> &[Position] {
        &[]
    }

    /// Zlecenia oczekujące schowane przed tym silnikiem — patrz
    /// [`Broker::ukryte_pozycje`].
    fn ukryte_zlecenia(&self) -> &[PendingOrder] {
        &[]
    }

    fn open_market(&mut self, r: OrderReq) -> BResult<Ticket>;
    fn place_pending(&mut self, r: PendingReq) -> BResult<Ticket>;
    fn modify_position(&mut self, t: Ticket, sl: Option<Px>, tp: Option<Px>) -> BResult<()>;
    fn modify_pending(
        &mut self,
        t: Ticket,
        price: Px,
        sl: Option<Px>,
        tp: Option<Px>,
    ) -> BResult<()>;
    fn close_position(&mut self, t: Ticket, reason: CloseReason) -> BResult<f64>;
    fn close_partial(&mut self, t: Ticket, volume: f64, reason: CloseReason) -> BResult<f64>;
    fn cancel_pending(&mut self, t: Ticket) -> BResult<()>;

    /// Transakcje zamknięte od ostatniego odczytu (silnik konsumuje i czyści).
    fn drain_closed(&mut self) -> Vec<ClosedTrade>;

    fn find_position(&self, t: Ticket) -> Option<&Position> {
        self.positions().iter().find(|p| p.ticket == t)
    }
}

/// Wylicza wykonalny wolumen CZESCIOWEGO zamkniecia.
///
/// Wynik jest najblizszym krokiem brokera (remisy w gore), ale nigdy nie
/// zamyka calej pozycji i nigdy nie zostawia reszty mniejszej niz `min`.
/// `None` oznacza, ze tej pozycji nie da sie juz podzielic na dwa wykonalne
/// kawalki. Dla historycznej siatki `min = step = 0.01` pierwsza galaz jest
/// doslownie starym wzorem silnika, wiec zachowanie dotychczasowych presetow
/// pozostaje identyczne.
#[inline]
pub fn partial_close_volume(current: f64, desired: f64, min: f64, step: f64) -> Option<f64> {
    if !current.is_finite()
        || !desired.is_finite()
        || !min.is_finite()
        || !step.is_finite()
        || current <= 0.0
        || desired <= 0.0
    {
        return None;
    }

    let min = if min > 0.0 { min } else { 0.01 };
    let step = if step > 0.0 { step } else { 0.01 };

    // Kontrakt bitowej zgodnosci ze starym XAUUSD (0.01/0.01).
    if (min - 0.01).abs() <= 1e-12 && (step - 0.01).abs() <= 1e-12 {
        let want = (desired * 100.0).round() / 100.0;
        let cut = want.max(0.01).min((current - 0.01).max(0.0));
        return (cut >= 0.01 - 1e-9).then_some(cut);
    }

    // Dodatnie wolumeny: floor(x + 0.5) daje deterministyczny remis w gore.
    let nearest = ((desired / step) + 0.5 + 1e-12).floor() * step;
    let max_cut = (((current - min).max(0.0) / step) + 1e-12).floor() * step;
    let mut cut = nearest.max(min).min(max_cut);
    cut = (cut * 1e8).round() / 1e8;

    // Ostatnia obrona przed niedokladnoscia f64 i nietypowa kombinacja
    // minimum/kroku. Cofamy po jednym kroku, az reszta bedzie handlowalna.
    while cut >= min - 1e-9 && current - cut < min - 1e-9 {
        cut = ((cut - step).max(0.0) * 1e8).round() / 1e8;
    }
    if cut < min - 1e-9 || cut >= current - 1e-9 || current - cut < min - 1e-9 {
        None
    } else {
        Some(cut)
    }
}

/// Czy poziom SL jest wykonalny dla brokera przy danej cenie?
///
/// Dwa warunki, oba realne u Vantage:
///  * SL musi leżeć po WŁAŚCIWEJ stronie ceny (poniżej dla BUY),
///  * nie bliżej niż `stops_level`.
///
/// Bez tego sprawdzenia symulacja „zamyka" pozycję po cenie lepszej od
/// rynkowej — czyli po cenie, której nigdy nie było.
#[inline]
pub fn sl_is_valid(side: Side, sl: Px, q: &Quote, stops_level: f64) -> bool {
    match side {
        Side::Buy => sl <= q.bid - stops_level,
        Side::Sell => sl >= q.ask + stops_level,
    }
}

#[inline]
pub fn tp_is_valid(side: Side, tp: Px, q: &Quote, stops_level: f64) -> bool {
    match side {
        Side::Buy => tp >= q.bid + stops_level,
        Side::Sell => tp <= q.ask - stops_level,
    }
}

/// Czy zlecenie OCZEKUJĄCE typu LIMIT może w ogóle leżeć na tej cenie?
///
/// MT5 stawia dwa warunki naraz i odrzuca oba tym samym kodem `10015`:
/// limit musi być po właściwej stronie rynku ORAZ nie bliżej niż `stops_level`.
/// Sam warunek „po właściwej stronie" to za mało — poziom oddalony o grosz od
/// ceny wygląda poprawnie, a broker go nie przyjmie.
#[inline]
pub fn limit_price_is_valid(side: Side, price: Px, q: &Quote, stops_level: f64) -> bool {
    match side {
        Side::Buy => price <= q.ask - stops_level,
        Side::Sell => price >= q.bid + stops_level,
    }
}

/// To samo dla zleceń STOP — lustrzane odbicie: stop leży ZA rynkiem.
#[inline]
pub fn stop_price_is_valid(side: Side, price: Px, q: &Quote, stops_level: f64) -> bool {
    match side {
        Side::Buy => price >= q.ask + stops_level,
        Side::Sell => price <= q.bid - stops_level,
    }
}

/// Najbliższa cena, na której limit jeszcze się położy.
#[inline]
pub fn clamp_limit_price(side: Side, price: Px, q: &Quote, stops_level: f64) -> Px {
    match side {
        Side::Buy => price.min(q.ask - stops_level),
        Side::Sell => price.max(q.bid + stops_level),
    }
}

/// Dosuwa SL do najbliższego wykonalnego poziomu (zamiast tracić modyfikację).
#[inline]
pub fn clamp_sl(side: Side, sl: Px, q: &Quote, stops_level: f64) -> Px {
    match side {
        Side::Buy => sl.min(q.bid - stops_level),
        Side::Sell => sl.max(q.ask + stops_level),
    }
}

/// Czy SL zlecenia OCZEKUJĄCEGO jest wykonalny WZGLĘDEM CENY TEGO ZLECENIA?
///
/// To jest inne pytanie niż [`sl_is_valid`] i właśnie ta różnica jest tu
/// treścią. Dla pozycji broker mierzy odległość stopu od CENY RYNKOWEJ; dla
/// zlecenia oczekującego — od CENY AKTYWACJI, bo w chwili wypełnienia to ona
/// będzie ceną otwarcia. MT5 odrzuca takie zlecenie kodem **10016**,
/// jeszcze przy składaniu, a nie przy wypełnieniu.
///
/// Bez tego sprawdzenia symulator przyjmuje „szczebel-widmo": poziom siatki,
/// którego SL leży DOKŁADNIE na cenie szczebla (odległość 0 < `stops_level`).
/// Żywy broker takiego zlecenia nie przyjmie, więc szczebel nigdy nie
/// zaistnieje — a backtest liczy z niego wynik.
#[inline]
pub fn pending_sl_is_valid(side: Side, sl: Px, order_price: Px, stops_level: f64) -> bool {
    match side {
        Side::Buy => sl <= order_price - stops_level,
        Side::Sell => sl >= order_price + stops_level,
    }
}

/// To samo dla TP zlecenia oczekującego — lustrzanie, cel leży ZA ceną
/// aktywacji, nie bliżej niż `stops_level`.
#[inline]
pub fn pending_tp_is_valid(side: Side, tp: Px, order_price: Px, stops_level: f64) -> bool {
    match side {
        Side::Buy => tp >= order_price + stops_level,
        Side::Sell => tp <= order_price - stops_level,
    }
}

#[cfg(test)]
mod partial_volume_tests {
    use super::partial_close_volume;

    #[test]
    fn polowa_008_zostawia_004_na_siatce_001() {
        let cut = partial_close_volume(0.08, 0.08 * 0.50, 0.01, 0.01).unwrap();
        assert_eq!(cut, 0.04);
        assert_eq!(0.08 - cut, 0.04);
    }

    #[test]
    fn polowa_007_ma_deterministyczny_remis_w_gore() {
        let cut = partial_close_volume(0.07, 0.07 * 0.50, 0.01, 0.01).unwrap();
        assert_eq!(cut, 0.04);
        assert!(((0.07 - cut) - 0.03).abs() < 1e-12);
    }

    #[test]
    fn nietypowa_siatka_brokera_i_minimum_sa_respektowane() {
        let cut = partial_close_volume(0.30, 0.15, 0.10, 0.10).unwrap();
        assert_eq!(cut, 0.20, "remis 0.15 / 0.10 musi isc w gore");
        assert!(((0.30 - cut) - 0.10).abs() < 1e-12);

        assert_eq!(partial_close_volume(0.10, 0.05, 0.10, 0.10), None);
        assert_eq!(partial_close_volume(0.15, 0.10, 0.10, 0.05), None);
        assert_eq!(partial_close_volume(0.25, 0.10, 0.10, 0.05), Some(0.10));
    }

    #[test]
    fn galaz_001_jest_dokladnie_starym_wzorem() {
        for current_cents in 1..=100 {
            for pct in [1.0, 15.0, 30.0, 33.0, 50.0, 67.0, 99.0] {
                let current = current_cents as f64 / 100.0;
                let desired = current * pct / 100.0;
                let old_want = (desired * 100.0).round() / 100.0;
                let old_cut = old_want.max(0.01).min((current - 0.01).max(0.0));
                let old = (old_cut >= 0.01 - 1e-9).then_some(old_cut);
                assert_eq!(partial_close_volume(current, desired, 0.01, 0.01), old);
            }
        }
    }
}
