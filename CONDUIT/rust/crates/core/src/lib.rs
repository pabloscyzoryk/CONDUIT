
pub mod broker;
/// Versioned pure cost arithmetic only; not wired into ClosedTrade or trading.
pub mod cost_receipt;
/// Pure conditional nominal-floor evaluator; not connected to trading yet.
pub mod rf_protection;
/// Pure session-only pending no-fill evidence; no live capability is enabled.
pub mod pending_cancel_proof;
/// EA-CORE — szkielet warstwy EA (FALA 0 z `wiedza/EA_PLAN_WDROZENIA.md`):
/// jeden wektor stanu, jeden zegar niezależny od strumienia tików, maszyna
/// Obrona/Neutral/Agresja z dwustronną histerezą i zapadką, jawna maszyna
/// stanu koszyka oraz PUSTE haki dla osi adaptacyjnych rodzin A–G.
///
/// Nie jest osią handlową i nie ma prawa zmienić ani jednej liczby przy
/// wyłącznikach w zerze — kontrakt zera jest tu POTRÓJNY i to jest cała
/// bramka akceptacji tej fali.
pub mod ea;
pub mod engine;
pub mod formaty;
pub mod journal;
pub mod most_mozgu;
/// Moduł obserwacji — cechy stanu rynku, sygnału i koszyka dla modeli AI.
/// Właściciel: zespół CECHY. Rdzeń tylko woła haki i nie liczy tu niczego.
pub mod obserwacje;
pub mod parser;
pub mod routing;
pub mod settings;
pub mod telegram_ingress;
pub mod types;
pub mod volume_contract;
pub mod profit_budget;
/// Arytmetyka wielu silników na jednym rachunku: rozłączne numery koszyków,
/// obciążenie pozostałych silników i podział ustawień na rachunkowe i handlowe.
/// Właściciel: zespół ROUTING.
pub mod wielosilnik;

pub use broker::{Broker, BrokerError, OrderReq, PendingReq};
pub use ea::{
    BilansPulsu, EaRdzen, EaStan, EtapKoszyka, KodPominiecia, Modulatory, StempelKoszyka,
    WektorStanu, ZrodloPulsu,
};
pub use engine::{odsiew_sita, Engine, IncomingMessage, LogLine, StanZmiennosci};
pub use journal::{
    EventCategory, EventKind, EventLevel, JournalBuf, JournalConfig, JournalEvent, RejectCode,
};
pub use obserwacje::{
    Komunikat, KonfigObserwacji, KontekstKoszyka, KontekstSygnalu, Obserwacje, Obserwator,
};
pub use parser::{EntrySignal, Signal};
pub use settings::{nieznane_pola_ustawien, Preset, Settings};
pub use types::*;
