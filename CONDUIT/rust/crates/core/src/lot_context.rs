//! Ten causal volume attenuators. They never choose signals, SL, TP or rearm.
//!
//! Each enabled axis proposes a cap in [0.25, 1]. The smallest cap wins;
//! multiplying ten penalties would silently compound the risk reduction.
//! This cap is not a guarantee on future drawdown or broker execution.

pub const AXIS_COUNT: usize = 10;
pub const FIELD_NAMES: [&str; AXIS_COUNT] = [
    "lot_growth_equity_stress_strength",
    "lot_growth_portfolio_load_strength",
    "lot_growth_direction_load_strength",
    "lot_growth_basket_count_strength",
    "lot_growth_spread_stress_strength",
    "lot_growth_tp1_deficit_strength",
    "lot_growth_stop_width_strength",
    "lot_growth_age_decay_strength",
    "lot_growth_rearm_decay_strength",
    "lot_growth_day_dd_strength",
];

/// Current broker observations and the currently accepted basket geometry.
/// None is unknown, not zero risk. TP1 reward is signed in the entry direction.
/// Age is since basket creation, and rearms are already completed batches.
#[derive(Debug, Clone, Copy, Default)]
pub struct LotStressContext {
    pub equity: Option<f64>,
    pub balance: Option<f64>,
    pub portfolio_sl_risk: Option<f64>,
    pub same_side_sl_risk: Option<f64>,
    pub active_baskets: Option<f64>,
    pub spread: Option<f64>,
    pub stop_distance: Option<f64>,
    pub tp1_reward_distance: Option<f64>,
    pub zone_width: Option<f64>,
    pub basket_age_secs: Option<f64>,
    pub completed_rearms: Option<f64>,
    pub day_start_equity: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LotStressOutcome {
    pub multiplier: f64,
    pub factors: [f64; AXIS_COUNT],
    pub dominant_axis: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LotStressError {
    InvalidStrength(usize),
    MissingOrInvalidInput { axis: usize, field: &'static str },
    NonFiniteStress(usize),
}

fn value(v: Option<f64>, axis: usize, field: &'static str) -> Result<f64, LotStressError> {
    v.filter(|x| x.is_finite()).ok_or(LotStressError::MissingOrInvalidInput { axis, field })
}
fn positive(v: Option<f64>, axis: usize, field: &'static str) -> Result<f64, LotStressError> {
    let x = value(v, axis, field)?;
    if x > 0.0 { Ok(x) } else { Err(LotStressError::MissingOrInvalidInput { axis, field }) }
}
fn nonnegative(v: Option<f64>, axis: usize, field: &'static str) -> Result<f64, LotStressError> {
    let x = value(v, axis, field)?;
    if x >= 0.0 { Ok(x) } else { Err(LotStressError::MissingOrInvalidInput { axis, field }) }
}

/// Strength 0 disables an axis; valid strengths are finite 0..=2.
/// OFF returns 1 without requiring any account, quote or basket observation.
pub fn evaluate(strengths: [f64; AXIS_COUNT], c: &LotStressContext)
    -> Result<LotStressOutcome, LotStressError>
{
    let mut result = LotStressOutcome { multiplier: 1.0, factors: [1.0; AXIS_COUNT], dominant_axis: None };
    for (i, strength) in strengths.into_iter().enumerate() {
        if !strength.is_finite() || !(0.0..=2.0).contains(&strength) {
            return Err(LotStressError::InvalidStrength(i));
        }
        if strength == 0.0 { continue; }
        let stress = match i {
            0 => (1.0 - positive(c.equity, i, "equity")?
                / positive(c.balance, i, "balance")?).max(0.0) / 0.10,
            1 => nonnegative(c.portfolio_sl_risk, i, "portfolio_sl_risk")?
                / (positive(c.equity, i, "equity")? * 0.20),
            2 => nonnegative(c.same_side_sl_risk, i, "same_side_sl_risk")?
                / (positive(c.equity, i, "equity")? * 0.10),
            3 => (nonnegative(c.active_baskets, i, "active_baskets")? - 1.0).max(0.0) / 3.0,
            4 => nonnegative(c.spread, i, "spread")?
                / (positive(c.stop_distance, i, "stop_distance")? * 0.05),
            5 => (1.0 - value(c.tp1_reward_distance, i, "tp1_reward_distance")?
                / positive(c.stop_distance, i, "stop_distance")?).max(0.0),
            6 => (positive(c.stop_distance, i, "stop_distance")?
                / positive(c.zone_width, i, "zone_width")? - 1.0).max(0.0),
            7 => nonnegative(c.basket_age_secs, i, "basket_age_secs")? / 86_400.0,
            8 => nonnegative(c.completed_rearms, i, "completed_rearms")? / 2.0,
            9 => (1.0 - positive(c.equity, i, "equity")?
                / positive(c.day_start_equity, i, "day_start_equity")?).max(0.0) / 0.10,
            _ => unreachable!(),
        };
        if !stress.is_finite() || stress < 0.0 { return Err(LotStressError::NonFiniteStress(i)); }
        let scaled = strength * stress;
        // Overflow cannot be silently interpreted as an infinite permitted stress.
        if !scaled.is_finite() { return Err(LotStressError::NonFiniteStress(i)); }
        let factor = (1.0 / (1.0 + scaled)).max(0.25);
        result.factors[i] = factor;
        if factor < result.multiplier {
            result.multiplier = factor;
            result.dominant_axis = Some(i);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> LotStressContext {
        LotStressContext { equity: Some(900.), balance: Some(1000.), portfolio_sl_risk: Some(180.),
            same_side_sl_risk: Some(90.), active_baskets: Some(4.), spread: Some(0.5),
            stop_distance: Some(10.), tp1_reward_distance: Some(0.), zone_width: Some(5.),
            basket_age_secs: Some(86400.), completed_rearms: Some(2.), day_start_equity: Some(1000.) }
    }
    #[test]
    fn off_needs_no_context() {
        assert_eq!(evaluate([0.;10], &LotStressContext::default()).unwrap(),
            LotStressOutcome { multiplier:1., factors:[1.;10], dominant_axis:None });
    }
    #[test]
    fn ten_independent_unit_stress_witnesses() {
        for i in 0..10 {
            let mut strengths=[0.;10];strengths[i]=1.;
            let r=evaluate(strengths,&context()).unwrap();
            assert!((r.multiplier-0.5).abs()<1e-14, "axis {i}: {r:?}");
            assert_eq!(r.dominant_axis,Some(i));
            assert_eq!(r.factors.iter().filter(|&&v|v==1.).count(),9);
        }
    }
    #[test]
    fn ten_axes_take_smallest_cap_not_product() {
        let r=evaluate([1.;10],&context()).unwrap();
        assert!((r.multiplier-0.5).abs()<1e-14);
        assert!(r.multiplier > r.factors.iter().product::<f64>());
    }
    #[test]
    fn absent_enabled_input_is_hold_but_absent_disabled_is_unused() {
        let mut c=context();c.tp1_reward_distance=None;
        let mut s=[0.;10];s[4]=1.;assert!(evaluate(s,&c).is_ok());
        s[5]=1.;assert_eq!(evaluate(s,&c),Err(LotStressError::MissingOrInvalidInput { axis:5,field:"tp1_reward_distance" }));
    }
    #[test]
    fn loss_or_profit_never_increases_lot() {
        let mut c=context(); c.equity=Some(2000.);c.tp1_reward_distance=Some(20.);
        let mut s=[0.;10];for i in [0,5,9]{s[i]=2.;}
        assert_eq!(evaluate(s,&c).unwrap().multiplier,1.);
        c.equity=Some(1.);assert_eq!(evaluate(s,&c).unwrap().multiplier,0.25);
    }
    #[test]
    fn signal_age_does_not_expire_signal_or_rearm_permission() {
        let mut c=context();c.basket_age_secs=Some(21.*86400.);
        let mut s=[0.;10];s[7]=1.;
        assert_eq!(evaluate(s,&c).unwrap().multiplier,0.25);
    }
    #[test]
    fn adverse_tp1_geometry_is_a_reduction_not_a_future_fill_assumption() {
        let mut c=context();c.tp1_reward_distance=Some(-10.);
        let mut s=[0.;10];s[5]=1.;
        assert!((evaluate(s,&c).unwrap().multiplier-1./3.).abs()<1e-15);
    }
    #[test]
    fn strength_monotonicity_and_floor() {
        for i in 0..10 {
            let mut last=1.;
            for strength in [0.,0.25,0.5,1.,2.] {
                let mut s=[0.;10];s[i]=strength;
                let next=evaluate(s,&context()).unwrap().multiplier;
                assert!(next<=last && (0.25..=1.).contains(&next));last=next;
            }
        }
    }
    #[test]
    fn invalid_strengths_and_nonfinite_observations_fail_closed() {
        for x in [-1.,2.01,f64::NAN,f64::INFINITY] {
            let mut s=[0.;10];s[0]=x;
            assert_eq!(evaluate(s,&context()),Err(LotStressError::InvalidStrength(0)));
        }
        let mut c=context();c.equity=Some(f64::NAN);
        let mut s=[0.;10];s[0]=1.;assert!(evaluate(s,&c).is_err());
        c=context();c.stop_distance=Some(0.);s=[0.;10];s[4]=1.;assert!(evaluate(s,&c).is_err());
    }
}
