//! Backtest: dane historyczne, symulator brokera, przebieg replay, metryki, wykresy.
pub mod chart;
pub mod data;
pub mod journal_dump;
pub mod loganaliza;
pub mod metrics;
pub mod okna;
pub mod runner;
pub mod sim;
pub mod sr_warmup;
mod continuation;
mod sim_costs;
pub mod statystyki;

pub use data::{
    load_messages, load_messages_with_time_offset, load_signals, RawSignal, ReplayMessage, TickData,
};
pub use metrics::{DayStat, Metrics};
pub use okna::{uruchom as uruchom_okna, KonfOkien, WynikOkien, WynikOkna};
pub use runner::{run, run_with_progress, ProgressFn, RunConfig, RunResult};
pub use sim::SimBroker;
pub use statystyki::StatSygnalow;

#[cfg(test)]
mod axis_sizing_audit;
#[cfg(test)]
mod axis_relot_contracts;
#[cfg(test)]
mod axis_entry_edit_contracts;
#[cfg(test)]
mod cost_net_contracts;
#[cfg(test)]
mod sr_warmup_v2_tests;
#[cfg(test)]
mod continuation_fresh_tests;
#[cfg(test)]
mod sim_pending_sl_sequence;
#[cfg(test)]
mod sim_observation_review_tests;
#[cfg(test)]
mod native_swap_edge_tests;
#[cfg(test)]
mod native_swap_driver_tests;
