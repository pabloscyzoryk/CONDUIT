//! Mapowanie retcodes MT5 na `BrokerError` z rdzenia.
//!
//! Dwie osobne rzeczy, których nie wolno mylić:
//!
//!  * **klasyfikacja** — na jaki `BrokerError` przekłada się dany kod,
//!  * **ponawialność** — czy w ogóle warto próbować jeszcze raz.
//!
//! Requote albo „cena się zmieniła" to nie jest odmowa brokera, tylko wyścig z
//! rynkiem: ponawiamy z odświeżoną ceną. Natomiast „invalid stops" albo „brak
//! marginu" ponawiane w pętli nigdy się nie uda i tylko zaleje serwer.

use conduit_core::broker::BrokerError;

use crate::proto::{local_code, WireError};

/// Kody `TRADE_RETCODE_*` z MQL5. Trzymane jawnie, żeby dało się je czytać w logu.
pub mod retcode {
    pub const REQUOTE: i64 = 10004;
    pub const REJECT: i64 = 10006;
    pub const CANCEL: i64 = 10007;
    pub const PLACED: i64 = 10008;
    pub const DONE: i64 = 10009;
    pub const DONE_PARTIAL: i64 = 10010;
    pub const ERROR: i64 = 10011;
    pub const TIMEOUT: i64 = 10012;
    pub const INVALID: i64 = 10013;
    pub const INVALID_VOLUME: i64 = 10014;
    pub const INVALID_PRICE: i64 = 10015;
    pub const INVALID_STOPS: i64 = 10016;
    pub const TRADE_DISABLED: i64 = 10017;
    pub const MARKET_CLOSED: i64 = 10018;
    pub const NO_MONEY: i64 = 10019;
    pub const PRICE_CHANGED: i64 = 10020;
    pub const PRICE_OFF: i64 = 10021;
    pub const INVALID_EXPIRATION: i64 = 10022;
    pub const ORDER_CHANGED: i64 = 10023;
    pub const TOO_MANY_REQUESTS: i64 = 10024;
    pub const NO_CHANGES: i64 = 10025;
    pub const SERVER_DISABLES_AT: i64 = 10026;
    pub const CLIENT_DISABLES_AT: i64 = 10027;
    pub const LOCKED: i64 = 10028;
    pub const FROZEN: i64 = 10029;
    pub const INVALID_FILL: i64 = 10030;
    pub const CONNECTION: i64 = 10031;
    pub const ONLY_REAL: i64 = 10032;
    pub const LIMIT_ORDERS: i64 = 10033;
    pub const LIMIT_VOLUME: i64 = 10034;
    pub const INVALID_ORDER: i64 = 10035;
    pub const POSITION_CLOSED: i64 = 10036;
    pub const INVALID_CLOSE_VOLUME: i64 = 10038;
    pub const CLOSE_ORDER_EXIST: i64 = 10039;
    pub const LIMIT_POSITIONS: i64 = 10040;
    pub const REJECT_CANCEL: i64 = 10041;
    pub const LONG_ONLY: i64 = 10042;
    pub const SHORT_ONLY: i64 = 10043;
    pub const CLOSE_ONLY: i64 = 10044;
    pub const FIFO_CLOSE: i64 = 10045;
    pub const HEDGE_PROHIBITED: i64 = 10046;
}

/// Czy `retcode` oznacza sukces? (`DONE`, `PLACED`, `DONE_PARTIAL`)
#[inline]
pub fn is_success(code: i64) -> bool {
    matches!(
        code,
        retcode::DONE | retcode::PLACED | retcode::DONE_PARTIAL
    )
}

/// Klasyfikacja retcode → `BrokerError`.
///
/// `NO_CHANGES` celowo NIE jest błędem z punktu widzenia silnika: prosiliśmy o
/// stan, który już obowiązuje. Zwracamy `Rejected`, a wywołujący (modyfikacja
/// SL/TP) traktuje go jako sukces — patrz `bridge::modify_position`.
pub fn classify(code: i64) -> BrokerError {
    use retcode as r;
    match code {
        r::INVALID_STOPS => BrokerError::InvalidStops,

        // `10015` mówi o CENIE ZLECENIA, nie o stopach. Wrzucony do jednego
        // worka z `INVALID_STOPS` kazał szukać problemu ze stop-lossem, gdy
        // naprawdę chodziło o poziom siatki po złej stronie rynku.
        r::INVALID_PRICE | r::INVALID_EXPIRATION => BrokerError::InvalidPrice,

        r::INVALID_VOLUME | r::INVALID_CLOSE_VOLUME | r::LIMIT_VOLUME => BrokerError::InvalidVolume,

        r::NO_MONEY => BrokerError::NotEnoughMargin,

        r::MARKET_CLOSED
        | r::TRADE_DISABLED
        | r::SERVER_DISABLES_AT
        | r::CLIENT_DISABLES_AT
        | r::CLOSE_ONLY
        | r::LONG_ONLY
        | r::SHORT_ONLY
        | r::ONLY_REAL => BrokerError::MarketClosed,

        r::POSITION_CLOSED | r::INVALID_ORDER => BrokerError::NoSuchTicket,
        local_code::NO_TICKET => BrokerError::NoSuchTicket,

        _ => BrokerError::Rejected,
    }
}

/// Czy ma sens ponowienie? Tylko wyścigi z rynkiem i chwilowe zatory.
///
/// `INVALID_FILL` jest ponawialny, ale w sposób szczególny: sidecar sam
/// przełącza tryb wypełnienia i próbuje jeszcze raz, więc do Rusta zwykle
/// w ogóle nie dolatuje.
pub fn is_execution_unknown(code: i64) -> bool {
    matches!(code,retcode::TIMEOUT | retcode::CONNECTION | retcode::ERROR)
}

pub fn is_retryable(code: i64) -> bool {
    use retcode as r;
    matches!(
        code,
        r::REQUOTE
            | r::PRICE_CHANGED
            | r::PRICE_OFF
            | r::TOO_MANY_REQUESTS
            | r::ORDER_CHANGED
            | r::INVALID_FILL
            | r::FROZEN
    )
}

/// Czy błąd znaczy „nic nie trzeba było robić"?
#[inline]
pub fn is_no_op(code: i64) -> bool {
    code == retcode::NO_CHANGES
}

/// Czytelna nazwa kodu do logu.
pub fn name(code: i64) -> &'static str {
    use retcode as r;
    match code {
        r::REQUOTE => "REQUOTE",
        r::REJECT => "REJECT",
        r::CANCEL => "CANCEL",
        r::PLACED => "PLACED",
        r::DONE => "DONE",
        r::DONE_PARTIAL => "DONE_PARTIAL",
        r::ERROR => "ERROR",
        r::TIMEOUT => "TIMEOUT",
        r::INVALID => "INVALID",
        r::INVALID_VOLUME => "INVALID_VOLUME",
        r::INVALID_PRICE => "INVALID_PRICE",
        r::INVALID_STOPS => "INVALID_STOPS",
        r::TRADE_DISABLED => "TRADE_DISABLED",
        r::MARKET_CLOSED => "MARKET_CLOSED",
        r::NO_MONEY => "NO_MONEY",
        r::PRICE_CHANGED => "PRICE_CHANGED",
        r::PRICE_OFF => "PRICE_OFF",
        r::INVALID_EXPIRATION => "INVALID_EXPIRATION",
        r::ORDER_CHANGED => "ORDER_CHANGED",
        r::TOO_MANY_REQUESTS => "TOO_MANY_REQUESTS",
        r::NO_CHANGES => "NO_CHANGES",
        r::SERVER_DISABLES_AT => "SERVER_DISABLES_AT",
        r::CLIENT_DISABLES_AT => "CLIENT_DISABLES_AT",
        r::LOCKED => "LOCKED",
        r::FROZEN => "FROZEN",
        r::INVALID_FILL => "INVALID_FILL",
        r::CONNECTION => "CONNECTION",
        r::ONLY_REAL => "ONLY_REAL",
        r::LIMIT_ORDERS => "LIMIT_ORDERS",
        r::LIMIT_VOLUME => "LIMIT_VOLUME",
        r::INVALID_ORDER => "INVALID_ORDER",
        r::POSITION_CLOSED => "POSITION_CLOSED",
        r::INVALID_CLOSE_VOLUME => "INVALID_CLOSE_VOLUME",
        r::CLOSE_ORDER_EXIST => "CLOSE_ORDER_EXIST",
        r::LIMIT_POSITIONS => "LIMIT_POSITIONS",
        r::REJECT_CANCEL => "REJECT_CANCEL",
        r::LONG_ONLY => "LONG_ONLY",
        r::SHORT_ONLY => "SHORT_ONLY",
        r::CLOSE_ONLY => "CLOSE_ONLY",
        r::FIFO_CLOSE => "FIFO_CLOSE",
        r::HEDGE_PROHIBITED => "HEDGE_PROHIBITED",
        local_code::NOT_INITIALIZED => "SIDECAR_NOT_INITIALIZED",
        local_code::UNKNOWN_CMD => "SIDECAR_UNKNOWN_CMD",
        local_code::BAD_ARGS => "SIDECAR_BAD_ARGS",
        local_code::NO_TICKET => "SIDECAR_NO_TICKET",
        local_code::EXCEPTION => "SIDECAR_EXCEPTION",
        _ => "?",
    }
}

impl From<&WireError> for BrokerError {
    fn from(e: &WireError) -> Self {
        classify(e.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_stops_ma_wlasna_kategorie() {
        assert_eq!(classify(retcode::INVALID_STOPS), BrokerError::InvalidStops);
        // to jest ten sam błąd, przez który poprzedni bot drukował darmowy zysk
        // w backteście — tutaj musi wracać jawnie, nie jako ogólne „Rejected"
        assert_ne!(classify(retcode::INVALID_STOPS), BrokerError::Rejected);
    }

    /// `10015` to odmowa CENY ZLECENIA, a nie stop-lossa. Dopóki oba kody
    /// wracały jako `InvalidStops`, log i mail kazały sprawdzać stops level,
    /// podczas gdy naprawy wymagał poziom siatki.
    #[test]
    fn invalid_price_to_nie_invalid_stops() {
        assert_eq!(classify(retcode::INVALID_PRICE), BrokerError::InvalidPrice);
        assert_ne!(classify(retcode::INVALID_PRICE), BrokerError::InvalidStops);
        assert!(!is_retryable(retcode::INVALID_PRICE));
    }

    #[test]
    fn brak_marginu_i_wolumen() {
        assert_eq!(classify(retcode::NO_MONEY), BrokerError::NotEnoughMargin);
        assert_eq!(
            classify(retcode::INVALID_VOLUME),
            BrokerError::InvalidVolume
        );
        assert_eq!(
            classify(retcode::INVALID_CLOSE_VOLUME),
            BrokerError::InvalidVolume
        );
    }

    #[test]
    fn rynek_zamkniety_zbiera_caly_kubelek() {
        for c in [
            retcode::MARKET_CLOSED,
            retcode::TRADE_DISABLED,
            retcode::SERVER_DISABLES_AT,
            retcode::CLIENT_DISABLES_AT,
        ] {
            assert_eq!(classify(c), BrokerError::MarketClosed, "kod {c}");
        }
    }

    #[test]
    fn brak_tiketu() {
        assert_eq!(
            classify(retcode::POSITION_CLOSED),
            BrokerError::NoSuchTicket
        );
        assert_eq!(classify(local_code::NO_TICKET), BrokerError::NoSuchTicket);
    }

    #[test]
    fn requote_jest_ponawialny_invalid_stops_nie() {
        assert!(is_retryable(retcode::REQUOTE));
        assert!(is_retryable(retcode::PRICE_CHANGED));
        assert!(is_retryable(retcode::PRICE_OFF));
        for code in [retcode::TIMEOUT,retcode::CONNECTION,retcode::ERROR] {
            assert!(is_execution_unknown(code));
            assert!(!is_retryable(code));
        }
        assert!(!is_retryable(retcode::INVALID_STOPS));
        assert!(!is_retryable(retcode::NO_MONEY));
        assert!(!is_retryable(retcode::MARKET_CLOSED));
    }

    #[test]
    fn sukcesy_rozpoznane() {
        assert!(is_success(retcode::DONE));
        assert!(is_success(retcode::PLACED));
        assert!(is_success(retcode::DONE_PARTIAL));
        assert!(!is_success(retcode::REQUOTE));
    }

    #[test]
    fn brak_zmian_to_nie_awaria() {
        assert!(is_no_op(retcode::NO_CHANGES));
        assert!(!is_no_op(retcode::REJECT));
    }

    #[test]
    fn nieznany_kod_nie_wywala_tylko_odrzuca() {
        assert_eq!(classify(99999), BrokerError::Rejected);
        assert_eq!(name(99999), "?");
        assert!(!is_retryable(99999));
    }
}
