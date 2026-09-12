//! Opt-in nominal lot curves and allocation of a NEW order. The optional
//! basket limit is a pre-send marked-to-stop budget, not a continuous DD stop.
use crate::{Broker, Settings, Side};
use crate::types::XAU_CONTRACT;
use crate::volume_contract::{normalize_open_volume, StrategyVolumeLimits, VolumeSpec};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LotGrowthMode { #[default] Off, Power, ThresholdLinear, GeometricSteps }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LotGrowthAllocation {
    #[default] Uniform, EqualSLRisk, Depth, EqualSLRiskDepth, ExposureAwareRisk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrowthError {
    InvalidSettings, InvalidAccount, InvalidQuote, InvalidExposure, UnknownBasket,
    MissingStop, InvalidNewStop, InvalidVolume, Exhausted, UnconfirmedExposure, UnsupportedEngine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationFallback { SpecialEntry, MissingZone, MissingStop, NonAdverseStop, ZeroReferenceRisk }

pub fn enabled(c: &Settings) -> bool { c.lot_growth_mode != LotGrowthMode::Off }
fn positive(x: f64) -> bool { x.is_finite() && x > 0.0 }

/// Only for a proportionally scaled initial plan: accumulated floating-point
/// noise must not remove an entire level. This is not an order-volume or cash
/// budget tolerance; those final pre-send checks retain their own contracts.
pub fn plan_risk_exceeds(now:f64,cap:f64)->bool {
    if !now.is_finite() || !cap.is_finite() {return true;}
    now>cap && now-cap>32.0*f64::EPSILON*now.abs().max(cap.abs()).max(1.0)
}

pub fn validate(c: &Settings) -> Result<(), GrowthError> {
    if !enabled(c) { return Ok(()); }
    if !c.lot_growth_reference_lot.is_finite() || c.lot_growth_reference_lot < 0.01
        || !positive(c.lot_growth_reference_balance)
        || !c.lot_growth_basket_risk_pct.is_finite()
        || !(0.0..=100.0).contains(&c.lot_growth_basket_risk_pct) {
        return Err(GrowthError::InvalidSettings);
    }
    let valid = match c.lot_growth_mode {
        LotGrowthMode::Off => true,
        LotGrowthMode::Power => positive(c.lot_growth_power) && c.lot_growth_power <= 1.0,
        LotGrowthMode::ThresholdLinear => c.lot_growth_rate_pct.is_finite() && c.lot_growth_rate_pct >= 0.0,
        LotGrowthMode::GeometricSteps => c.lot_growth_capital_multiple.is_finite()
            && c.lot_growth_capital_multiple > 1.0 && c.lot_growth_lot_multiple.is_finite()
            && c.lot_growth_lot_multiple >= 1.0,
    };
    if valid { Ok(()) } else { Err(GrowthError::InvalidSettings) }
}

/// No broker rounding here: all later strategy multipliers are retained, and
/// the actual order is floored once to the broker step immediately before send.
pub fn nominal(c: &Settings, capital: f64) -> Result<f64, GrowthError> {
    validate(c)?;
    if !positive(capital) { return Err(GrowthError::InvalidAccount); }
    let ratio = (capital / c.lot_growth_reference_balance).max(1.0);
    let lot = match c.lot_growth_mode {
        LotGrowthMode::Off => return Err(GrowthError::InvalidSettings),
        LotGrowthMode::Power => c.lot_growth_reference_lot * ratio.powf(c.lot_growth_power),
        LotGrowthMode::ThresholdLinear => c.lot_growth_reference_lot
            + (capital-c.lot_growth_reference_balance).max(0.0)*c.lot_growth_rate_pct/10000.0,
        LotGrowthMode::GeometricSteps => {
            let exponent=ratio.ln()/c.lot_growth_capital_multiple.ln();
            let nearest=exponent.round();
            // Snap only quotient representation noise at an exact power.
            // Real capital below a threshold still belongs to the lower step.
            let tolerance=8.0*f64::EPSILON*exponent.abs().max(1.0);
            let steps=if (exponent-nearest).abs()<=tolerance {nearest} else {exponent.floor()};
            c.lot_growth_reference_lot*c.lot_growth_lot_multiple.powf(steps)
        }
    };
    if positive(lot) { Ok(lot) } else { Err(GrowthError::InvalidVolume) }
}

/// An unresolved submission can consume budget even before it appears in a
/// position/pending snapshot. Absence alone never releases that reservation.
pub fn ready<B: Broker>(c: &Settings, b: &B) -> Result<(), GrowthError> {
    if !enabled(c) { return Ok(()); }
    validate(c)?;
    if c.t100.enabled { return Err(GrowthError::UnsupportedEngine); }
    if b.receipt_barrier() != crate::broker::ReceiptBarrier::Clear
        || b.unconfirmed_open().is_some_and(|intent| b.confirmed_open(&intent).is_none()) {
        return Err(GrowthError::UnconfirmedExposure);
    }
    Ok(())
}

/// Allocation and attenuation shape only the surplus above the first legal
/// lot. A request already below that minimum stays below it and will be refused
/// by the final floor. A later cash-risk cap can still refuse the whole order.
pub fn apply_surplus<B:Broker>(c:&Settings,b:&B,requested:f64,factor:f64)->Result<f64,GrowthError> {
    if !positive(requested)||!positive(factor) {return Err(GrowthError::InvalidVolume);}
    if factor==1.0 {return Ok(requested);}
    let a=b.account();
    let minimum=crate::volume_contract::minimum_legal_open_volume(
        VolumeSpec{minimum:b.volume_min(),step:b.volume_step(),maximum:b.volume_max()},
        StrategyVolumeLimits{minimum:c.lot_min,maximum:c.lot_max,capital_per_lot:c.lot_max_z_salda,
            capital:c.podstawa_lota_z_konta(a.balance,a.equity,a.credit)})
        .map_err(|_|GrowthError::InvalidVolume)?;
    if requested<=minimum {return Ok(requested);}
    let volume=minimum+(requested-minimum)*factor;
    if positive(volume) {Ok(volume)} else {Err(GrowthError::InvalidVolume)}
}

fn downside(side: Side, price: f64, sl: Option<f64>, volume: f64) -> Result<f64, GrowthError> {
    if !positive(price) || !positive(volume) { return Err(GrowthError::InvalidExposure); }
    let sl = sl.filter(|v| positive(*v)).ok_or(GrowthError::MissingStop)?;
    let value = ((price-sl)*side.sign()).max(0.0)*XAU_CONTRACT*volume;
    if value.is_finite() { Ok(value) } else { Err(GrowthError::InvalidExposure) }
}

/// Effective SL: broker stop first, cached virtual stop only when absent.
/// Hidden/frozen exposure remains counted. A row without an owner cannot be
/// proved to belong elsewhere and therefore cannot create available capacity.
pub fn basket_downside<B: Broker>(b: &B, basket: u32) -> Result<f64, GrowthError> {
    let q = b.quote();
    if !positive(q.bid) || !positive(q.ask) || q.ask < q.bid { return Err(GrowthError::InvalidQuote); }
    let mut used = 0.0;
    for p in b.positions().iter().chain(b.ukryte_pozycje()) {
        let id = p.basket.ok_or(GrowthError::UnknownBasket)?;
        if id == basket { used += downside(p.side, q.exit(p.side), p.sl.or(p.vsl), p.volume)?; }
    }
    for p in b.pendings().iter().chain(b.ukryte_zlecenia()) {
        let id = p.basket.ok_or(GrowthError::UnknownBasket)?;
        if id == basket { used += downside(p.kind.side(), p.price, p.sl, p.volume)?; }
    }
    if used.is_finite() { Ok(used) } else { Err(GrowthError::InvalidExposure) }
}

/// `zone` is the CURRENT accepted entry zone, never a future fill set.
/// Special entries and unusable reference geometry fall back explicitly to
/// Uniform. Invalid numeric order/exposure inputs are errors, not fallbacks.
pub fn allocate<B: Broker>(c: &Settings, b: &B, basket: Option<u32>, level: i32,
    side: Side, entry: f64, stop: Option<f64>, zone: Option<(f64,f64)>, requested: f64)
    -> Result<(f64, Option<AllocationFallback>), GrowthError> {
    if !enabled(c) { return Ok((requested,None)); }
    ready(c,b)?;
    if !positive(requested) { return Err(GrowthError::InvalidVolume); }
    let entry = b.normalize_order_price(entry);
    if !positive(entry) { return Err(GrowthError::InvalidQuote); }
    if c.lot_growth_allocation == LotGrowthAllocation::Uniform { return Ok((requested,None)); }
    let fallback = |why| Ok((requested,Some(why)));
    if level < 0 { return fallback(AllocationFallback::SpecialEntry); }
    let Some((lo,hi)) = zone else { return fallback(AllocationFallback::MissingZone); };
    if !positive(lo) || !positive(hi) || lo > hi { return Err(GrowthError::InvalidExposure); }
    let Some(stop) = stop else { return fallback(AllocationFallback::MissingStop); };
    let stop = b.normalize_order_price(stop);
    if !positive(stop) { return Err(GrowthError::MissingStop); }
    let d_entry = (entry-stop)*side.sign();
    let d_ref = (lo+(hi-lo)*0.5-stop)*side.sign();
    if d_entry <= 0.0 { return fallback(AllocationFallback::NonAdverseStop); }
    if d_ref <= 0.0 { return fallback(AllocationFallback::ZeroReferenceRisk); }
    let depth = if hi == lo {0.5} else if side == Side::Buy {(hi-entry)/(hi-lo)} else {(entry-lo)/(hi-lo)}.clamp(0.0,1.0);
    let risk_weight = d_ref/d_entry;
    let weight = match c.lot_growth_allocation {
        LotGrowthAllocation::Uniform => 1.0,
        LotGrowthAllocation::EqualSLRisk => risk_weight,
        LotGrowthAllocation::Depth => 0.5+depth,
        LotGrowthAllocation::EqualSLRiskDepth => risk_weight*(0.75+0.5*depth),
        LotGrowthAllocation::ExposureAwareRisk => {
            let equity = b.account().equity;
            if !positive(equity) { return Err(GrowthError::InvalidAccount); }
            if !c.risk_per_basket_pct.is_finite() || c.risk_per_basket_pct < 0.0 { return Err(GrowthError::InvalidSettings); }
            let capacity = equity*c.risk_per_basket_pct/100.0;
            if capacity == 0.0 { return fallback(AllocationFallback::ZeroReferenceRisk); }
            if !positive(capacity) { return Err(GrowthError::InvalidAccount); }
            risk_weight/(1.0+basket_downside(b,basket.ok_or(GrowthError::UnknownBasket)?)?/capacity).sqrt()
        }
    };
    if !positive(weight) { return Err(GrowthError::InvalidVolume); }
    let volume = apply_surplus(c,b,requested,weight.clamp(0.5,1.5))?;
    if positive(volume) { Ok((volume,None)) } else { Err(GrowthError::InvalidVolume) }
}

/// Final optional per-basket cap, after every legacy multiplier and any
/// portfolio/profit cap. Zero disables only this additional budget.
pub fn limit<B: Broker>(c: &Settings, b: &B, basket: Option<u32>, side: Side,
    entry: f64, stop: Option<f64>, requested: f64) -> Result<f64, GrowthError> {
    if !enabled(c) { return Ok(requested); }
    ready(c,b)?;
    if c.lot_growth_basket_risk_pct == 0.0 { return Ok(requested); }
    let basket = basket.ok_or(GrowthError::UnknownBasket)?;
    let equity = b.account().equity;
    if !positive(equity) { return Err(GrowthError::InvalidAccount); }
    let capacity = equity*c.lot_growth_basket_risk_pct/100.0;
    if !positive(capacity) { return Err(GrowthError::InvalidAccount); }
    let remaining = (capacity-basket_downside(b,basket)?).max(0.0);
    if remaining == 0.0 { return Err(GrowthError::Exhausted); }
    let entry = b.normalize_order_price(entry);
    let stop = stop.map(|v| b.normalize_order_price(v)).filter(|v| positive(*v)).ok_or(GrowthError::MissingStop)?;
    if !positive(entry) || (entry-stop)*side.sign() <= 0.0 { return Err(GrowthError::InvalidNewStop); }
    if !positive(requested) { return Err(GrowthError::InvalidVolume); }
    let per_lot = (entry-stop)*side.sign()*XAU_CONTRACT;
    let a = b.account();
    let volume = normalize_open_volume(requested.min(remaining/per_lot),
        VolumeSpec{minimum:b.volume_min(),step:b.volume_step(),maximum:b.volume_max()},
        StrategyVolumeLimits{minimum:c.lot_min,maximum:c.lot_max,capital_per_lot:c.lot_max_z_salda,
            capital:c.podstawa_lota_z_konta(a.balance,a.equity,a.credit)})
        .map_err(|e| if e == crate::volume_contract::VolumeError::BelowMinimum {GrowthError::Exhausted} else {GrowthError::InvalidVolume})?;
    if per_lot*volume > remaining+16.0*f64::EPSILON*remaining.max(1.0) { return Err(GrowthError::Exhausted); }
    Ok(volume)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profit_budget::tests::{broker,position,pending};
    fn cfg()->Settings {Settings{lot_growth_mode:LotGrowthMode::Power,lot_max:0.0,
        risk_per_basket_pct:20.0,..Settings::default()}}
    fn near(a:f64,b:f64){assert!((a-b).abs()<1e-10,"{a} != {b}");}
    #[test]
    fn lot_growth_curves_plateau_shape_and_domains() {
        let mut c=cfg();c.lot_growth_power=0.5;
        near(nominal(&c,300.0).unwrap(),0.01);near(nominal(&c,4000.0).unwrap(),0.02);
        c.lot_growth_mode=LotGrowthMode::ThresholdLinear;c.lot_growth_rate_pct=0.5;
        near(nominal(&c,1000.0).unwrap(),0.01);near(nominal(&c,2000.0).unwrap(),0.06);
        c.lot_growth_mode=LotGrowthMode::GeometricSteps;
        near(nominal(&c,1999.99).unwrap(),0.01);near(nominal(&c,2000.0).unwrap(),0.015);
        near(nominal(&c,4000.0).unwrap(),0.0225);
        c.lot_growth_capital_multiple=1.0;assert_eq!(nominal(&c,2000.0),Err(GrowthError::InvalidSettings));
        c=cfg();c.lot_growth_power=1.1;assert!(validate(&c).is_err());
        c.lot_growth_mode=LotGrowthMode::Off;c.lot_growth_reference_lot=f64::NAN;
        assert_eq!(validate(&c),Ok(()));
    }
    #[test]
    fn lot_growth_surplus_preserves_legal_minimum_but_never_promotes_a_small_request() {
        let c=cfg();let mut b=broker();
        for factor in [0.25,0.5,1.0,1.5] {
            assert_eq!(apply_surplus(&c,&b,0.01,factor),Ok(0.01));
            assert_eq!(apply_surplus(&c,&b,0.005,factor),Ok(0.005));
        }
        near(apply_surplus(&c,&b,0.03,0.5).unwrap(),0.02);
        b.minimum=0.015;
        assert_eq!(apply_surplus(&c,&b,0.02,0.25),Ok(0.02));
        assert_eq!(apply_surplus(&c,&b,0.015,1.5),Ok(0.015));
        b.maximum=0.019;assert!(apply_surplus(&c,&b,0.03,0.5).is_err());
    }
    #[test]
    fn lot_growth_geometric_exact_boundaries_and_real_cent_brackets() {
        let mut c=cfg();c.lot_growth_mode=LotGrowthMode::GeometricSteps;
        for anchor in [600.0,1000.0,1500.0,2500.0] {
            c.lot_growth_reference_balance=anchor;
            for multiple in [1.5_f64,1.75,2.0,2.5,3.0] {
                c.lot_growth_capital_multiple=multiple;
                for n in 1..=12 {
                    let capital=anchor*multiple.powf(n as f64);
                    let want=0.01*1.5_f64.powf(n as f64);
                    let actual=nominal(&c,capital).unwrap();
                    assert!((actual-want).abs()<1e-12,"anchor={anchor} multiple={multiple} n={n}: {actual} != {want}");
                    near(nominal(&c,capital-0.01).unwrap(),want/1.5);
                    near(nominal(&c,capital+0.01).unwrap(),want);
                }
            }
        }
    }
    #[test]
    fn lot_growth_off_and_zero_budget_are_exact_passthrough() {
        let mut c=cfg();let mut b=broker();c.lot_growth_mode=LotGrowthMode::Off;
        c.lot_growth_basket_risk_pct=f64::NAN;b.q.ask=f64::NAN;
        assert_eq!(allocate(&c,&b,None,0,Side::Buy,f64::NAN,None,None,0.0157),Ok((0.0157,None)));
        assert_eq!(limit(&c,&b,None,Side::Buy,f64::NAN,None,0.0157),Ok(0.0157));
        c=cfg();assert_eq!(limit(&c,&b,None,Side::Buy,f64::NAN,None,0.0157),Ok(0.0157));
    }
    #[test]
    fn lot_growth_allocation_is_side_symmetric_bounded_and_no_zero_division() {
        let mut c=cfg();let b=broker();
        for mode in [LotGrowthAllocation::Uniform,LotGrowthAllocation::EqualSLRisk,
            LotGrowthAllocation::Depth,LotGrowthAllocation::EqualSLRiskDepth,LotGrowthAllocation::ExposureAwareRisk] {
            c.lot_growth_allocation=mode;
            let buy=allocate(&c,&b,Some(1),0,Side::Buy,3980.0,Some(3970.0),Some((3980.0,3990.0)),0.02).unwrap().0;
            let sell=allocate(&c,&b,Some(1),0,Side::Sell,4020.0,Some(4030.0),Some((4010.0,4020.0)),0.02).unwrap().0;
            near(buy,sell);assert!((0.01..=0.03).contains(&buy));
        }
        c.lot_growth_allocation=LotGrowthAllocation::EqualSLRisk;
        assert_eq!(allocate(&c,&b,Some(1),0,Side::Buy,3980.0,Some(3980.0),Some((3980.0,3990.0)),0.02),
            Ok((0.02,Some(AllocationFallback::NonAdverseStop))));
        assert_eq!(allocate(&c,&b,Some(1),-2,Side::Buy,3980.0,Some(3970.0),None,0.02),
            Ok((0.02,Some(AllocationFallback::SpecialEntry))));
    }
    #[test]
    fn lot_growth_whole_basket_marks_protected_profit_counts_hidden_frozen_and_other_side() {
        let mut c=cfg();c.lot_growth_basket_risk_pct=10.0;let mut b=broker();
        b.positions.push(position(Side::Buy,3900.0,Some(3999.0),0.1));
        b.hidden_positions.push(position(Side::Sell,4100.0,Some(4001.0),0.1));
        let mut p=pending(1,Side::Buy,3990.0,Some(3980.0),0.02);p.frozen=true;b.hidden_pendings.push(p);
        b.pendings.push(pending(2,Side::Buy,3990.0,None,1.0));
        near(basket_downside(&b,1).unwrap(),38.0);
        near(limit(&c,&b,Some(1),Side::Buy,3990.0,Some(3980.0),1.0).unwrap(),0.03);
        b.positions[0].open_price=100.0;near(basket_downside(&b,1).unwrap(),38.0);
        b.equity=389.99;assert_eq!(limit(&c,&b,Some(1),Side::Buy,3990.0,Some(3980.0),1.0),Err(GrowthError::Exhausted));
        b.positions[0].sl=None;assert_eq!(basket_downside(&b,1),Err(GrowthError::MissingStop));
        b.positions[0].vsl=Some(3999.0);assert!(basket_downside(&b,1).is_ok());
        b.positions[0].basket=None;assert_eq!(basket_downside(&b,1),Err(GrowthError::UnknownBasket));
    }
    #[test]
    fn lot_growth_risk_uses_normalized_stop_and_never_promotes_minimum() {
        let mut c=cfg();let mut b=broker();c.lot_growth_basket_risk_pct=10.0;b.equity=499.99;
        near(limit(&c,&b,Some(1),Side::Buy,4000.0,Some(3990.004),1.0).unwrap(),0.05);
        b.price_digits=Some(2);near(limit(&c,&b,Some(1),Side::Buy,4000.0,Some(3990.004),1.0).unwrap(),0.04);
        b.minimum=0.1;b.step=0.1;
        assert_eq!(limit(&c,&b,Some(1),Side::Buy,4000.0,Some(3990.0),1.0),Err(GrowthError::Exhausted));
    }
}
