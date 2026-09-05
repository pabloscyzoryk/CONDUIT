//! Offline diagnostic of the ACTUAL Engine::lot_size implementation.
//! No Broker, Telegram, terminal or production state is constructed.
//! Run only after the main release build finishes:
//! cargo run --offline --locked -p conduit-core --example audit_dynamic_lot_boundaries
//!
//! Assertions below document current boundary behavior, NOT desired safe behavior.
//! If a future explicit sizing policy fixes it, update this audit's expectations.
use conduit_core::{Engine, Settings};
use serde_json::json;

fn case(
    name: &str,
    cfg: Settings,
    balance: f64,
    equity: f64,
    expected: f64,
    intended_cap: Option<f64>,
) {
    let mut engine = Engine::new(cfg, balance);
    engine.stats.equity = equity;
    let basis = engine.podstawa_lota();
    let actual = engine.lot_size(basis);
    assert!(
        (actual - expected).abs() < 1e-10,
        "{name}: actual={actual}, recorded pre-fix behavior={expected}"
    );
    println!(
        "{}",
        json!({
            "case": name, "balance": balance, "equity": equity, "basis": basis,
            "actual_base_lot": actual, "recorded_current_result": expected,
            "configured_min": engine.cfg.lot_min, "configured_static_max": engine.cfg.lot_max,
            "dynamic_divisor": engine.cfg.lot_max_z_salda, "intended_cap": intended_cap,
            "exceeds_intended_cap": intended_cap.map(|cap| actual > cap + 1e-10),
            "below_configured_min": actual + 1e-10 < engine.cfg.lot_min,
            "scope": "Engine::lot_size only; no order execution / no live certification"
        })
    );
}

fn fixed() -> Settings {
    let mut cfg = Settings::default();
    cfg.lot_mode_percent = false;
    cfg.lot_fixed = 1.0;
    cfg.lot_min = 0.01;
    cfg.lot_max = 0.0;
    cfg.lot_scale_step = 0.0;
    cfg.lot_max_z_salda = 0.0;
    cfg
}

fn main() {
    let mut cfg = fixed();
    cfg.lot_max = 0.016;
    case(
        "non_step_static_cap_rounds_up",
        cfg,
        600.0,
        600.0,
        0.02,
        Some(0.016),
    );
    let mut cfg = fixed();
    cfg.lot_max = 0.006;
    case(
        "max_below_min_swaps_bounds",
        cfg,
        600.0,
        600.0,
        0.01,
        Some(0.006),
    );
    let mut cfg = fixed();
    cfg.lot_max = 0.004;
    cfg.lot_fixed = 0.0;
    case(
        "substep_max_can_produce_zero",
        cfg,
        600.0,
        600.0,
        0.0,
        Some(0.004),
    );
    let mut cfg = fixed();
    cfg.lot_min = 0.014;
    cfg.lot_fixed = 0.0;
    case(
        "nonstep_min_can_round_below_min",
        cfg,
        600.0,
        600.0,
        0.01,
        None,
    );
    let mut cfg = fixed();
    cfg.lot_max_z_salda = 25_000.0;
    case(
        "dynamic_400_div_25000_rounds_above_cap",
        cfg,
        400.0,
        400.0,
        0.02,
        Some(0.016),
    );
    let mut cfg = fixed();
    cfg.lot_max_z_salda = 25_000.0;
    case(
        "zero_basis_dynamic_cap_falls_back_to_unlimited",
        cfg,
        0.0,
        0.0,
        1.0,
        Some(0.0),
    );
    let mut cfg = fixed();
    cfg.lot_max_z_salda = 25_000.0;
    cfg.lot_max = 0.05;
    case(
        "zero_basis_dynamic_cap_falls_back_to_static",
        cfg,
        0.0,
        0.0,
        0.05,
        Some(0.0),
    );
    let mut cfg = fixed();
    cfg.lot_mode_percent = true;
    cfg.lot_percent = 0.5;
    cfg.lot_max_z_salda = 25_000.0;
    case(
        "zero_basis_percentage_still_has_minimum",
        cfg,
        0.0,
        0.0,
        0.01,
        Some(0.0),
    );
    for (balance, expected) in [
        (100.0, 0.01),
        (300.0, 0.02),
        (600.0, 0.03),
        (1000.0, 0.05),
        (10_000.0, 0.5),
        (100_000.0, 5.0),
    ] {
        let mut cfg = fixed();
        cfg.lot_mode_percent = true;
        cfg.lot_percent = 0.5;
        case(
            "release_percent_0_5_no_strategy_max",
            cfg,
            balance,
            balance,
            expected,
            None,
        );
    }
    let mut cfg = fixed();
    cfg.lot_mode_percent = true;
    cfg.lot_percent = 0.5;
    cfg.lot_base = conduit_core::settings::PodstawaLota::MinOfBoth;
    case(
        "min_of_balance_equity_changes_basis",
        cfg,
        1000.0,
        600.0,
        0.03,
        None,
    );
    let mut cfg = fixed();
    cfg.lot_mode_percent = true;
    cfg.lot_percent = 0.5;
    cfg.odlicz_kredyt = true;
    cfg.kredyt_reczny = 400.0;
    case(
        "credit_deduction_changes_basis",
        cfg,
        1000.0,
        1000.0,
        0.03,
        None,
    );
}
