//! CONDUIT AI — model, który przejmuje 100 % zarządzania pozycjami.
//!
//! # Podział odpowiedzialności
//!
//! Kierunku NIE wymyśla model. Kierunek i strefa wejścia przychodzą z kanału
//! Telegram i obsługuje je `Engine`. Model dostaje to, co z sygnału wynikło —
//! koszyk pozycji i zleceń — i od tego momentu decyduje o wszystkim: kiedy
//! wyjść, ile zamknąć, gdzie przesunąć SL i TP, które limity skasować, czy
//! dołożyć wejście i jakim wolumenem.
//!
//! # Moduły
//!
//! | moduł | rola |
//! |---|---|
//! | [`obs`] | cechy obserwacji + pamięć rynku (ATR, zwroty, zmienność) |
//! | [`policy`] | dwie sieci MLP, dekodowanie akcji, format modelu (JSON) |
//! | [`safety`] | twarde ograniczenia wykonania — model NIE MOŻE wyzerować konta |
//! | [`runtime`] | inferencja w pętli bota; funkcja wołana przez silnik |
//! | [`reward`] | funkcja nagrody i agregacja po oknach |
//! | [`rollout`] | przebieg backtestu z polityką w miejscu `manage_positions` |
//! | [`train`] | ewolucja (OpenAI-ES / CEM), równolegle przez `rayon` |
//!
//! # Trzy niezależne warstwy ochrony kapitału
//!
//! 1. **Nagroda** karze obsunięcie, otwarte ryzyko i wyzerowanie konta —
//!    to uczy model, czego unikać.
//! 2. **Warstwa bezpieczeństwa** ([`safety`]) blokuje akcje podnoszące
//!    wykorzystanie marginu ponad próg i pilnuje zapadki SL — to sprawia, że
//!    model nie ma jak zrobić szkody, nawet gdyby chciał.
//! 3. **Podłoga equity** likwiduje pozycje i zatrzymuje silnik — ostatnia
//!    linia, sprawdzana na każdym ticku.
//!
//! Warstwy 2 i 3 żyją w kodzie wykonania, nie w funkcji celu. Kara w nagrodzie
//! podlega optymalizacji i optymalizator prędzej czy później znajdzie jej
//! obejście; ograniczenie w kodzie obejścia nie ma.
//!
//! # Użycie w pętli bota
//!
//! ```ignore
//! let mut ai = AiRuntime::load("models/atfx_v1.json", balance, cfg.ai_decision_interval_s)?;
//! // …
//! engine.on_tick(&mut broker, &quote);
//! ai.on_tick(&mut engine, &mut broker, &quote);   // ← zarządzanie pozycjami
//! ```

pub mod obs;
pub mod peak;
pub mod policy;
pub mod reward;
pub mod rollout;
pub mod runtime;
pub mod safety;
pub mod train;

pub use obs::{MarketWindow, B_DIM, G_DIM, POS_IN, P_DIM};
pub use policy::{BasketDecision, Exit, Model, Policy, PosDecision, SlAction, TpAction};
pub use reward::{RewardWeights, Summary, WindowOutcome};
pub use rollout::{run_window, split_windows, training_settings, RunCfg, Window};
pub use runtime::AiRuntime;
pub use safety::{Safety, SafetyCfg};
pub use train::{train, Algo, TrainCfg};
