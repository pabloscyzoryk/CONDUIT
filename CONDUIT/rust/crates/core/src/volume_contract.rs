//! Opt-in opening-volume contract. No orders, account mutation or rounding-up.
//! A broker max of NaN is unknown, not permission for unlimited exposure.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeSpec {
    pub minimum: f64,
    pub step: f64,
    pub maximum: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrategyVolumeLimits {
    pub minimum: f64,
    /// Zero disables this user cap, never the broker cap.
    pub maximum: f64,
    /// Zero disables the dynamic cap; otherwise cap = capital / divisor.
    pub capital_per_lot: f64,
    pub capital: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeError {
    InvalidRequested,
    InvalidBrokerMinimum,
    InvalidBrokerStep,
    /// The current live adapter serializes volume on an eight-decimal lattice.
    UnsupportedBrokerStepPrecision,
    InvalidBrokerMaximum,
    InvalidStrategyMinimum,
    InvalidStrategyMaximum,
    InvalidDynamicDivisor,
    InvalidCapital,
    ConflictingBounds,
    BelowMinimum,
    UnrepresentableUnits,
    OffGrid,
}

fn positive(x: f64) -> bool { x.is_finite() && x > 0.0 }
fn nonnegative(x: f64) -> bool { x.is_finite() && x >= 0.0 }

impl StrategyVolumeLimits {
    /// Validate and derive user bounds without inventing a broker model.
    pub fn bounds(self) -> Result<(f64,f64),VolumeError> {
        if !positive(self.minimum) { return Err(VolumeError::InvalidStrategyMinimum); }
        if !nonnegative(self.maximum) { return Err(VolumeError::InvalidStrategyMaximum); }
        if !nonnegative(self.capital_per_lot) { return Err(VolumeError::InvalidDynamicDivisor); }
        let mut maximum=if self.maximum>0.0 {self.maximum} else {f64::MAX};
        if self.capital_per_lot>0.0 {
            if !nonnegative(self.capital) { return Err(VolumeError::InvalidCapital); }
            let dynamic=self.capital/self.capital_per_lot;
            if !dynamic.is_finite() { return Err(VolumeError::InvalidCapital); }
            maximum=maximum.min(dynamic);
        }
        if maximum<self.minimum { return Err(VolumeError::ConflictingBounds); }
        Ok((self.minimum,maximum))
    }
}

impl VolumeSpec {
    pub fn validate(self) -> Result<(),VolumeError> {
        if !positive(self.minimum) { return Err(VolumeError::InvalidBrokerMinimum); }
        if !positive(self.step) { return Err(VolumeError::InvalidBrokerStep); }
        let wire_units = self.step * 1e8;
        if !wire_units.is_finite() || wire_units < 1.0
            || (wire_units - wire_units.round()).abs()
                > 8.0 * f64::EPSILON * wire_units.abs().max(1.0) {
            return Err(VolumeError::UnsupportedBrokerStepPrecision);
        }
        if !positive(self.maximum) { return Err(VolumeError::InvalidBrokerMaximum); }
        if self.maximum<self.minimum { return Err(VolumeError::ConflictingBounds); }
        Ok(())
    }
}

/// Only float-representation noise at an integer quotient may be snapped.
/// A real fractional lot is always floored; this does not permit 0.015→0.02.
fn units(x:f64,step:f64,up:bool)->Result<f64,VolumeError> {
    let n=x/step;
    if !n.is_finite() || n>4_503_599_627_370_496.0 { return Err(VolumeError::UnrepresentableUnits); }
    let rounded=n.round();
    let tolerance=8.0*f64::EPSILON*n.abs().max(1.0);
    Ok(if (n-rounded).abs()<=tolerance {rounded} else if up {n.ceil()} else {n.floor()})
}

pub fn volume_epsilon(value:f64,step:f64)->f64 {
    16.0*f64::EPSILON*value.abs().max(step.abs()).max(1.0)
}

/// Normalize an opening request, after every strategy multiplier.
/// Never increases requested volume or any cap except roundoff tolerance.
pub fn normalize_open_volume(requested:f64, spec:VolumeSpec, limits:StrategyVolumeLimits)
    ->Result<f64,VolumeError> {
    if !positive(requested) { return Err(VolumeError::InvalidRequested); }
    spec.validate()?;
    let (user_min,user_max)=limits.bounds()?;
    let minimum=user_min.max(spec.minimum);
    let maximum=user_max.min(spec.maximum);
    if minimum>maximum { return Err(VolumeError::ConflictingBounds); }
    let low=units(minimum,spec.step,true)?;
    let high=units(requested.min(maximum),spec.step,false)?;
    if high<low || high<1.0 { return Err(VolumeError::BelowMinimum); }
    let result=high*spec.step;
    let eps=volume_epsilon(result,spec.step);
    if !positive(result) || result>requested+eps || result>maximum+eps || result+eps<minimum {
        return Err(VolumeError::UnrepresentableUnits);
    }
    debug_assert!((result/spec.step-(result/spec.step).round()).abs()
        <=16.0*f64::EPSILON*(result/spec.step).abs().max(1.0));
    Ok(result)
}

/// Simulator/broker boundary: validate the request, do not silently re-size it.
pub fn validate_broker_volume(volume:f64,spec:VolumeSpec)->Result<(),VolumeError> {
    if !positive(volume) {return Err(VolumeError::InvalidRequested);}
    spec.validate()?;
    let eps=volume_epsilon(volume,spec.step);
    if volume+eps<spec.minimum {return Err(VolumeError::BelowMinimum);}
    if volume>spec.maximum+eps {return Err(VolumeError::ConflictingBounds);}
    let floored=units(volume,spec.step,false)?*spec.step;
    if (floored-volume).abs()>eps {return Err(VolumeError::OffGrid);}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec()->VolumeSpec {VolumeSpec{minimum:0.01,step:0.01,maximum:100.0}}
    fn limits()->StrategyVolumeLimits {StrategyVolumeLimits{minimum:0.01,maximum:0.0,capital_per_lot:0.0,capital:1000.0}}
    fn n(x:f64,s:VolumeSpec,l:StrategyVolumeLimits)->f64 {normalize_open_volume(x,s,l).unwrap()}
    #[test] fn cap_between_steps_floors_not_nearest(){let mut l=limits();l.maximum=0.015;assert_eq!(n(0.10,spec(),l),0.01);}
    #[test] fn request_between_steps_floors(){assert_eq!(n(0.019,spec(),limits()),0.01);}
    #[test] fn never_promotes_small_topup(){let mut l=limits();l.minimum=0.03;assert_eq!(normalize_open_volume(0.01,spec(),l),Err(VolumeError::BelowMinimum));}
    #[test] fn inverted_user_bounds_fail(){let mut l=limits();l.minimum=0.10;l.maximum=0.05;assert_eq!(normalize_open_volume(0.10,spec(),l),Err(VolumeError::ConflictingBounds));}
    #[test] fn dynamic_cap_survives_final_multiplier(){let mut l=limits();l.capital_per_lot=10000.0;assert_eq!(n(0.20,spec(),l),0.10);}
    #[test] fn zero_user_cap_does_not_disable_broker_cap(){let mut s=spec();s.maximum=0.07;assert_eq!(n(20.0,s,limits()),0.07);}
    #[test] fn zero_dynamic_capital_does_not_fall_back_to_unlimited(){let mut l=limits();l.capital_per_lot=10000.0;l.capital=0.0;assert_eq!(normalize_open_volume(0.2,spec(),l),Err(VolumeError::ConflictingBounds));}
    #[test] fn nonstep_minimum_is_ceiled(){let mut s=spec();s.minimum=0.015;assert_eq!(n(0.029,s,limits()),0.02);assert!(normalize_open_volume(0.019,s,limits()).is_err());}
    #[test] fn broker_step_point_one(){let mut s=spec();s.minimum=0.10;s.step=0.10;assert_eq!(n(0.29,s,limits()),0.20);}
    #[test] fn broker_step_point_zero_zero_one(){let mut s=spec();s.minimum=0.001;s.step=0.001;let mut l=limits();l.minimum=0.001;assert!((n(0.0079,s,l)-0.007).abs()<1e-12);}
    #[test] fn exact_decimal_boundary_is_not_one_step_short(){for x in [0.03,0.07,0.14,0.29,0.58,1.16]{assert!((n(x,spec(),limits())-x).abs()<1e-12);}}
    #[test] fn invalid_requested_fails(){for x in [0.,-1.,f64::NAN,f64::INFINITY,f64::NEG_INFINITY]{assert_eq!(normalize_open_volume(x,spec(),limits()),Err(VolumeError::InvalidRequested));}}
    #[test] fn invalid_broker_bounds_fail(){for x in [0.,-1.,f64::NAN,f64::INFINITY]{let mut s=spec();s.maximum=x;assert!(normalize_open_volume(0.1,s,limits()).is_err());s=spec();s.minimum=x;assert!(normalize_open_volume(0.1,s,limits()).is_err());s=spec();s.step=x;assert!(normalize_open_volume(0.1,s,limits()).is_err());}}
    #[test] fn invalid_strategy_bounds_fail(){for x in [-1.,f64::NAN,f64::INFINITY]{let mut l=limits();l.maximum=x;assert!(normalize_open_volume(0.1,spec(),l).is_err());l=limits();l.minimum=x;assert!(normalize_open_volume(0.1,spec(),l).is_err());l=limits();l.capital_per_lot=x;assert!(normalize_open_volume(0.1,spec(),l).is_err());}}
    #[test] fn invalid_dynamic_capital_fails(){for x in [-1.,f64::NAN,f64::INFINITY]{let mut l=limits();l.capital_per_lot=100.;l.capital=x;assert!(normalize_open_volume(0.1,spec(),l).is_err());}}
    #[test] fn offgrid_broker_request_is_not_silently_rounded(){assert_eq!(validate_broker_volume(0.015,spec()),Err(VolumeError::OffGrid));assert!(validate_broker_volume(0.03,spec()).is_ok());}
    #[test] fn excessive_units_fail_instead_of_unsafe_integer_arithmetic(){let mut s=spec();s.step=1e-8;s.maximum=1e9;assert_eq!(normalize_open_volume(1e9,s,limits()),Err(VolumeError::UnrepresentableUnits));}
    #[test] fn unsupported_live_wire_precision_fails_closed(){for step in [1e-9,1.5e-8,1e-20]{let mut s=spec();s.step=step;assert_eq!(normalize_open_volume(0.1,s,limits()),Err(VolumeError::UnsupportedBrokerStepPrecision));}}
    #[test] fn accepted_volumes_survive_current_mt5_eight_decimal_boundary(){for step in [1e-8,0.001,0.01,0.015,0.1]{let s=VolumeSpec{minimum:step,step,maximum:100.0};let mut l=limits();l.minimum=step;for req in [0.0079,0.019,0.123456789,0.29,3.17]{if let Ok(v)=normalize_open_volume(req,s,l){let wire=((((v/step).round()*step)*1e8).round()/1e8).clamp(s.minimum,s.maximum);assert!((wire-v).abs()<=volume_epsilon(v,step),"{step}: {v} -> {wire}");}}}}
    #[test] fn invariants_over_steps_requests_and_caps(){for step in [0.001,0.01,0.1]{for req in [0.001,0.009,0.01,0.015,0.03,0.07,0.105,0.29,1.234,100.01]{for cap in [0.0,0.005,0.01,0.015,0.04,0.3,10.0]{let s=VolumeSpec{minimum:step,step,maximum:100.0};let mut l=limits();l.minimum=step;l.maximum=cap;if let Ok(v)=normalize_open_volume(req,s,l){let eps=volume_epsilon(v,step);assert!(v<=req+eps&&v<=s.maximum+eps&&v+eps>=step);assert!(cap==0.0||v<=cap+eps);assert!(validate_broker_volume(v,s).is_ok());}}}}}
}
