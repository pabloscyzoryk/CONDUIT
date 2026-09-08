//! CONDUIT MT5 — most do terminala MetaTrader 5.
//!
//! MetaTrader 5 nie ma API dla Rusta. Są dokładnie trzy drogi i każda coś kosztuje:
//!
//!  1. **Sidecar w Pythonie** (`sidecar/mt5_sidecar.py`) — pakiet `MetaTrader5`
//!     firmy MetaQuotes rozmawia z terminalem po IPC. My rozmawiamy z sidecarem
//!     po TCP na pętli lokalnej, protokołem liniowym JSON. To jest ścieżka
//!     wdrożona tutaj: działa od razu, kosztuje jeden proces Pythona i ułamek
//!     milisekundy na wywołanie.
//!  2. **Expert Advisor w MQL5** (`mql5/ConduitBridge.mq5`) — kod działa
//!     wewnątrz terminala, więc odpada cała warstwa IPC. Szkic jest
//!     w repozytorium jako ścieżka docelowa (nazwany potok Windows).
//!  3. Wstrzykiwanie do procesu terminala — odpada, nie ma o czym mówić.
//!
//! # Układ
//!
//! ```text
//!   Engine ──(cecha Broker)──▶ Mt5Bridge ──▶ Transport ──TCP──▶ sidecar.py ──▶ terminal
//! ```
//!
//! `Mt5Bridge` jest **synchroniczny**, bo taka jest cecha `Broker`. Cała
//! asynchroniczność (strumień ticków, nadzór procesu) siedzi w `Transport`
//! i w wątku nadzorcy; do silnika dociera przez `Mt5Bridge::poll()`.
//!
//! # Czego ten kod NIE robi
//!
//! Nie zgaduje parametrów instrumentu. `digits`, `point`, `stops_level`, krok
//! wolumenu i tryb wypełnienia są pobierane **z serwera** przy starcie.
//! Zaszycie `stops_level = 20` w kodzie działa u jednego brokera i cicho
//! niszczy wyniki u każdego innego.

pub mod bridge;
pub mod comment;
mod cost_adapter;
pub mod errors;
pub mod market;
pub mod operation_evidence;
pub mod proto;
pub mod transport;
pub mod watchdog;

pub use bridge::{
    ForeignClosed, ForeignOrder, ForeignPosition, Mt5Bridge, Origin, ReconcileReport, StopsMismatch,
};
pub use errors::{classify, is_retryable, retcode};
pub use market::MarketData;
pub use proto::{
    AccountIdent, Bar, BrokerSymbol, Candles, Costs, Deal, Deals, Hello, SlipStats, SymbolInfo,
    SymbolsList,
};
pub use transport::{CallError, SidecarConfig, Transport, TransportHandle};
pub use watchdog::{
    discover_terminal, Recovery, RetryPlan, TerminalControl, Watchdog, WatchdogConfig,
    WatchdogEvent, WindowsTerminal,
};

/// Ścieżka do skryptu sidecara względem katalogu tego crate'a.
///
/// Przydaje się aplikacji, która chce zbudować `SidecarConfig` bez zgadywania,
/// gdzie leży plik.
pub fn default_sidecar_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("sidecar")
        .join("mt5_sidecar.py")
}

#[cfg(test)]
mod tests {
    #[test]
    fn skrypt_sidecara_lezy_tam_gdzie_obiecujemy() {
        let p = super::default_sidecar_path();
        assert!(p.exists(), "brak pliku sidecara pod {}", p.display());
    }
}
