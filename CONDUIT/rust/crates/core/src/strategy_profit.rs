//! Strategy accounting is distinct from the broker cash ledger and its display.
//! The legacy XAU/USD strategy uses confirmed price movement plus allocated swap.
use crate::cost_receipt::ProfitBasis;
use crate::types::{ClosedTrade, XAU_CONTRACT};

pub const STRATEGY_REALIZED_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyProfitError {
    MissingGeometry,
    InvalidValue,
    CanonicalReceipt,
}

/// Only this verified live contract is comparable with the existing simulator.
pub fn legacy_scope_issue(symbol: &str, currency: &str, contract: f64) -> Option<&'static str> {
    if !matches!(symbol, "XAUUSD" | "XAUUSD.s")
        || currency != "USD"
        || !contract.is_finite()
        || contract != XAU_CONTRACT
    {
        Some("STRATEGY P/L HOLD: verified XAUUSD, USD account and 100-unit contract required")
    } else {
        None
    }
}

impl ClosedTrade {
    /// Never changes profit/net/cash. A live producer must separately verify the
    /// USD/100-unit contract and provenance of this exact closed tranche.
    pub fn strategy_realized_profit(
        &self,
        canonical_net: bool,
    ) -> Result<f64, StrategyProfitError> {
        if canonical_net {
            self.canonical_net()
                .map_err(|_| StrategyProfitError::CanonicalReceipt)?;
            return Ok(self.profit);
        }
        let value = match self.profit_basis {
            Some(ProfitBasis::PriceOnlyGross) => {
                if !self.open_price.is_finite()
                    || self.open_price <= 0.0
                    || !self.close_price.is_finite()
                    || self.close_price <= 0.0
                    || !self.volume.is_finite()
                    || self.volume <= 0.0
                    || self.open_ts <= 0
                    || self.close_ts < self.open_ts
                {
                    return Err(StrategyProfitError::MissingGeometry);
                }
                if !self.swap.is_finite() {
                    return Err(StrategyProfitError::InvalidValue);
                }
                (self.close_price - self.open_price) * self.side.sign() * XAU_CONTRACT * self.volume
                    + self.swap
            }
            // Preserve every historical simulator floating operation, including
            // legacy source-defined fixtures/archives. No inferred reinterpretation.
            _ => self.profit,
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err(StrategyProfitError::InvalidValue)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CloseReason, Side};
    fn trade(open: f64, close: f64, profit: f64) -> ClosedTrade {
        ClosedTrade {
            ticket: 1,
            side: Side::Sell,
            volume: 0.01,
            open_price: open,
            close_price: close,
            open_ts: 1000,
            close_ts: 2000,
            profit,
            commission: -0.07,
            swap: 0.0,
            reason: CloseReason::RiskFree,
            basket: Some(1),
            profit_basis: Some(ProfitBasis::PriceOnlyGross),
            cost_receipt: None,
        }
    }
    #[test]
    fn five_confirmed_rf_closes_use_same_zero_boundary_as_historical_price_arithmetic() {
        let opens = [4591.13, 4592.19, 4593.44, 4594.43, 4595.8];
        let profits = [-2.90, -1.84, -0.59, 0.40, 1.77];
        let mut strategy = 0.0;
        let mut cash_gross = 0.0;
        for (open, gross) in opens.into_iter().zip(profits) {
            let t = trade(open, 4594.03, gross);
            let before = serde_json::to_vec(&t).unwrap();
            strategy += t.strategy_realized_profit(false).unwrap();
            cash_gross += t.profit;
            assert_eq!(serde_json::to_vec(&t).unwrap(), before);
        }
        let keeper = (4593.57 - 4596.73) * -1.0 * 100.0 * 0.01;
        assert!(strategy + keeper >= 0.0);
        assert!(cash_gross + keeper < 0.0);
        assert_eq!(strategy.to_bits(), (-3.159999999998945_f64).to_bits());
    }
    #[test]
    fn swap_partial_volume_and_restart_preserve_strategy_but_not_replace_broker_net() {
        let mut t = trade(4050.01, 4040.03, 9.98);
        t.volume = 0.03;
        t.swap = -1.23;
        let expected = (t.close_price - t.open_price) * -1.0 * 100.0 * 0.03 - 1.23;
        let a = t.strategy_realized_profit(false).unwrap();
        assert_eq!(a.to_bits(), expected.to_bits());
        let restored: ClosedTrade =
            serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
        assert_eq!(
            restored.strategy_realized_profit(false).unwrap().to_bits(),
            a.to_bits()
        );
        assert_eq!(restored.profit, 9.98);
        assert_eq!(restored.net_profit(), Some(9.98 - 0.07 - 1.23));
    }
    #[test]
    fn simulator_price_plus_swap_is_bit_exact_even_with_different_geometry() {
        for profit in [0.0, -0.0, 1e-13, -3.159999999998945, 1522404.6699999697] {
            let mut t = trade(4000.0, 4001.0, profit);
            t.profit_basis = Some(ProfitBasis::PricePlusSwap);
            t.swap = 4.0;
            assert_eq!(
                t.strategy_realized_profit(false).unwrap().to_bits(),
                profit.to_bits()
            );
        }
    }
    #[test]
    fn missing_geometry_or_wrong_account_contract_is_never_inferred() {
        let t = trade(4000.0, 4001.0, -1.0);
        for variant in 0..5 {
            let mut bad = t.clone();
            match variant {
                0 => bad.open_price = 0.0,
                1 => bad.volume = 0.0,
                2 => bad.open_ts = 0,
                3 => bad.close_ts = 500,
                _ => bad.swap = f64::NAN,
            }
            assert!(bad.strategy_realized_profit(false).is_err());
        }
        assert!(legacy_scope_issue("XAUUSD.s", "USD", 100.0).is_none());
        assert!(legacy_scope_issue("XAUUSD", "USD", 100.0).is_none());
        for (s, c, n) in [
            ("XAUUSD", "EUR", 100.0),
            ("XAUUSD", "USD", 1.0),
            ("BTCUSD", "USD", 100.0),
        ] {
            assert!(legacy_scope_issue(s, c, n).is_some());
        }
        assert!(t.strategy_realized_profit(true).is_err());
    }
}
