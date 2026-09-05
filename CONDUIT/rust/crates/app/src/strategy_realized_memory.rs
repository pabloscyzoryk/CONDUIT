//! Internal persistence of strategy arithmetic, never broker cash accounting.
use conduit_core::{engine::Engine, types::Basket};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const HOLD: &str = "STRATEGY P/L HOLD: saved management result has an unverified basis";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StrategyRealizedMemory {
    version: u32,
    canonical_net: bool,
    review: bool,
    day: i64,
    realized_today_bits: u64,
    gross_win_bits: u64,
    gross_loss_bits: u64,
    closed_today_bits: Vec<u64>,
    closed_observations: Vec<(i64, u64)>,
    baskets: BTreeMap<u32, BasketResult>,
}

#[derive(Clone, Serialize, Deserialize)]
struct BasketResult {
    msg_id: i64,
    created_ts: i64,
    realized_bits: u64,
}

impl StrategyRealizedMemory {
    pub(crate) fn capture(e: &Engine) -> Self {
        Self {
            version: conduit_core::strategy_profit::STRATEGY_REALIZED_VERSION,
            canonical_net: e.cfg.closed_profit_net_costs,
            review: e
                .cost_reconciliation_required
                .as_deref()
                .is_some_and(|s| s.contains("STRATEGY P/L")),
            day: e.stats.day,
            realized_today_bits: e.stats.realized_today.to_bits(),
            gross_win_bits: e.stats.gross_win.to_bits(),
            gross_loss_bits: e.stats.gross_loss.to_bits(),
            closed_today_bits: e.closed_today.iter().map(|x| x.to_bits()).collect(),
            closed_observations: e.obs.closed_strategy_observation_bits(),
            baskets: e
                .baskets
                .iter()
                .map(|b| {
                    (
                        b.id,
                        BasketResult {
                            msg_id: b.msg_id,
                            created_ts: b.created_ts,
                            realized_bits: b.realized.to_bits(),
                        },
                    )
                })
                .collect(),
        }
    }
}

fn carries_strategy_result(b: &Basket) -> bool {
    b.alive() && b.realized != 0.0
}

fn projection_matches(value: f64, bits: u64) -> bool {
    let exact = f64::from_bits(bits);
    if !exact.is_finite() {
        return false;
    }
    let Ok(encoded) = serde_json::to_string(&exact) else {
        return false;
    };
    let Ok(projected) = serde_json::from_str::<f64>(&encoded) else {
        return false;
    };
    value == projected || value.to_bits() == bits
}

/// Called after the existing account-scoped basket restore and before its first
/// engine tick. The exact binary result lives alongside the version, so decimal
/// JSON parsing cannot move a zero-boundary strategy to the other side of zero.
pub(crate) fn restore(
    e: &mut Engine,
    memory: Option<&StrategyRealizedMemory>,
    current_day: i64,
) -> Result<(), &'static str> {
    if e.cfg.closed_profit_net_costs {
        return Ok(());
    }
    let meaningful_day = e.stats.day == current_day
        && (e.stats.realized_today != 0.0 || e.closed_today.iter().any(|x| *x != 0.0));
    let meaningful_baskets = e.baskets.iter().any(carries_strategy_result);
    let Some(m) = memory else {
        return if meaningful_day || meaningful_baskets {
            Err(HOLD)
        } else {
            Ok(())
        };
    };
    if m.review {
        return Err(HOLD);
    }
    if m.version != conduit_core::strategy_profit::STRATEGY_REALIZED_VERSION
        || m.canonical_net != e.cfg.closed_profit_net_costs
    {
        return if meaningful_day || meaningful_baskets {
            Err(HOLD)
        } else {
            Ok(())
        };
    }
    if [m.realized_today_bits, m.gross_win_bits, m.gross_loss_bits]
        .iter()
        .any(|x| !f64::from_bits(*x).is_finite())
        || m.closed_today_bits
            .iter()
            .any(|x| !f64::from_bits(*x).is_finite())
        || m.closed_observations.len() > 50
        || m.closed_observations
            .iter()
            .any(|(ts, bits)| *ts <= 0 || !f64::from_bits(*bits).is_finite())
    {
        return Err(HOLD);
    }
    if e.stats.day != m.day {
        return if meaningful_day || meaningful_baskets {
            Err(HOLD)
        } else {
            Ok(())
        };
    }
    if !projection_matches(e.stats.realized_today, m.realized_today_bits)
        || !projection_matches(e.stats.gross_win, m.gross_win_bits)
        || !projection_matches(e.stats.gross_loss, m.gross_loss_bits)
        || e.closed_today.len() != m.closed_today_bits.len()
        || e.closed_today
            .iter()
            .zip(&m.closed_today_bits)
            .any(|(value, bits)| !projection_matches(*value, *bits))
    {
        return Err(HOLD);
    }
    for b in e.baskets.iter().filter(|b| b.alive()) {
        let Some(saved) = m.baskets.get(&b.id) else {
            if b.realized != 0.0 {
                return Err(HOLD);
            } else {
                continue;
            }
        };
        let value = f64::from_bits(saved.realized_bits);
        if saved.msg_id != b.msg_id || saved.created_ts != b.created_ts || !value.is_finite() {
            return Err(HOLD);
        }
        // Verify the decimal projection before restoring exact bits. The JSON
        // decoder itself is the only accepted transform; no P/L tolerance is used.
        if !projection_matches(b.realized, saved.realized_bits) {
            return Err(HOLD);
        }
    }
    if e.stats.day == m.day {
        e.stats.realized_today = f64::from_bits(m.realized_today_bits);
        e.stats.gross_win = f64::from_bits(m.gross_win_bits);
        e.stats.gross_loss = f64::from_bits(m.gross_loss_bits);
        e.closed_today = m
            .closed_today_bits
            .iter()
            .map(|x| f64::from_bits(*x))
            .collect();
    }
    for b in e.baskets.iter_mut().filter(|b| b.alive()) {
        if let Some(saved) = m.baskets.get(&b.id) {
            b.realized = f64::from_bits(saved.realized_bits);
        }
    }
    if !e
        .obs
        .restore_closed_strategy_observation_bits(&m.closed_observations)
    {
        return Err(HOLD);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use conduit_core::settings::Settings;
    fn basket(realized: f64) -> Basket {
        serde_json::from_value(
            serde_json::json!({"id":1,"source":{"chat_id":1,"topic_id":null},
            "source_name":"Synthetic","msg_id":10,"side":"Sell","is_limit":true,
            "entry_lo":4000.0,"entry_hi":4005.0,"zone_lo":4000.0,"zone_hi":4005.0,
            "sl":4010.0,"tps":[3990.0],"tp_stage":0,"created_ts":1000,
            "state":"Working","tickets":[1],"pendings":[],"realized":realized,"events":[]}),
        )
        .unwrap()
    }
    #[test]
    fn empty_legacy_and_previous_day_archive_do_not_block_fresh_start() {
        let mut e = Engine::new(Settings::default(), 600.0);
        assert!(restore(&mut e, None, 20000).is_ok());
        e.stats.day = 19999;
        e.stats.realized_today = 12.34;
        e.closed_today = vec![12.34];
        assert!(restore(&mut e, None, 20000).is_ok());
        assert_eq!(e.stats.realized_today, 12.34);
    }
    #[test]
    fn old_same_day_nonzero_strategy_memory_holds_without_resetting_cash_or_risk() {
        let mut e = Engine::new(Settings::default(), 600.0);
        e.stats.day = 20000;
        e.stats.realized_today = -3.16;
        e.closed_today = vec![-3.16];
        assert_eq!(restore(&mut e, None, 20000), Err(HOLD));
        assert_eq!(e.stats.balance, 600.0);
        assert_eq!(e.stats.realized_today, -3.16);
    }
    #[test]
    fn versioned_same_day_state_survives_json_restart_with_exact_boundary_bits() {
        let mut e = Engine::new(Settings::default(), 600.0);
        e.stats.day = 20000;
        e.stats.realized_today = -3.159999999998945;
        e.stats.gross_win = 2.1700000000009823;
        e.stats.gross_loss = 5.329999999999927;
        e.closed_today = vec![-2.899999999999636, 1.7700000000004366];
        e.obs.na_zamknieciu(1000, -2.899999999999636);
        e.obs.na_zamknieciu(2000, 1.7700000000004366);
        let m = StrategyRealizedMemory::capture(&e);
        let m: StrategyRealizedMemory =
            serde_json::from_slice(&serde_json::to_vec(&m).unwrap()).unwrap();
        let mut restored = Engine::new(Settings::default(), 600.0);
        restored.stats.day = 20000;
        restored.stats = serde_json::from_slice(&serde_json::to_vec(&e.stats).unwrap()).unwrap();
        restored.closed_today =
            serde_json::from_slice(&serde_json::to_vec(&e.closed_today).unwrap()).unwrap();
        assert!(restore(&mut restored, Some(&m), 20000).is_ok());
        assert_eq!(
            restored.stats.realized_today.to_bits(),
            e.stats.realized_today.to_bits()
        );
        assert_eq!(
            restored
                .closed_today
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>(),
            e.closed_today
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            restored.obs.closed_strategy_observation_bits(),
            e.obs.closed_strategy_observation_bits()
        );
        restored.stats.realized_today = -3.15;
        assert_eq!(
            restore(&mut restored, Some(&m), 20000),
            Err(HOLD),
            "stale stamp cannot overwrite current result"
        );
    }
    #[test]
    fn active_carry_requires_stamp_but_archive_and_zero_state_do_not() {
        let mut e = Engine::new(Settings::default(), 600.0);
        e.stats.day = 19999;
        e.baskets.push(basket(-3.16));
        assert_eq!(restore(&mut e, None, 20000), Err(HOLD));
        assert_eq!(e.baskets[0].realized, -3.16);
        e.baskets[0].state = conduit_core::types::BasketState::Done;
        assert!(restore(&mut e, None, 20000).is_ok());
        e.baskets[0] = basket(0.0);
        assert!(restore(&mut e, None, 20000).is_ok());
    }
    #[test]
    fn stamped_active_basket_restores_exact_bits_and_rejects_wrong_source_or_stale_amount() {
        let mut e = Engine::new(Settings::default(), 600.0);
        e.stats.day = 20000;
        e.baskets.push(basket(-3.159999999998945));
        let m = StrategyRealizedMemory::capture(&e);
        let m: StrategyRealizedMemory =
            serde_json::from_slice(&serde_json::to_vec(&m).unwrap()).unwrap();
        let mut restarted = Engine::new(Settings::default(), 600.0);
        restarted.stats.day = 20000;
        restarted.baskets =
            serde_json::from_slice(&serde_json::to_vec(&e.baskets).unwrap()).unwrap();
        assert!(restore(&mut restarted, Some(&m), 20000).is_ok());
        assert_eq!(
            restarted.baskets[0].realized.to_bits(),
            e.baskets[0].realized.to_bits()
        );
        assert!(
            restore(&mut restarted, Some(&m), 20000).is_ok(),
            "reconnect is idempotent"
        );
        restarted.baskets[0].msg_id = 11;
        assert_eq!(restore(&mut restarted, Some(&m), 20000), Err(HOLD));
        restarted.baskets[0].msg_id = 10;
        restarted.baskets[0].realized = -3.15;
        assert_eq!(restore(&mut restarted, Some(&m), 20000), Err(HOLD));
    }
}
