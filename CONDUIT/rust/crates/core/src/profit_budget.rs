//! Opt-in daily profit reserve for NEW exposure only. All inputs are observed
//! now; it is not a stop order and cannot promise a floor across gaps or costs.
use crate::{Broker, Settings, Side, Stats};
use crate::types::{day_of, XAU_CONTRACT};
use crate::volume_contract::{normalize_open_volume, StrategyVolumeLimits, VolumeSpec};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BudgetAnchor { pub day: i64, pub start: f64, pub peak: f64 }
impl Default for BudgetAnchor {
    fn default() -> Self { Self { day: i64::MIN, start: 0.0, peak: 0.0 } }
}
/// Exact private snapshot: serde_json's default decimal parser may move an
/// f64 by one ULP, enough to alter a boundary-sized order after restart.
#[derive(Clone,Copy,Debug,serde::Serialize,serde::Deserialize)]
pub struct BudgetAnchorBits {pub day:i64,pub start:u64,pub peak:u64}
impl From<&Stats> for BudgetAnchorBits {
    fn from(s:&Stats)->Self {Self {day:s.day,start:s.day_start_equity.to_bits(),peak:s.day_peak_equity.to_bits()}}
}
impl BudgetAnchorBits {
    pub fn restore(self,s:&mut Stats) {s.day=self.day;s.day_start_equity=f64::from_bits(self.start);
        s.day_peak_equity=f64::from_bits(self.peak);}
}
impl From<&Stats> for BudgetAnchor {
    fn from(s: &Stats) -> Self { Self { day:s.day, start:s.day_start_equity, peak:s.day_peak_equity } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetError { InvalidSettings, UnknownDayAnchor, InvalidAccount, InvalidQuote,
    InvalidExposure, MissingStop, InvalidNewStop, InvalidVolume, Exhausted }
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvailableBudget { pub floor: f64, pub capacity: f64, pub downside: f64, pub remaining: f64 }
fn positive(x:f64)->bool {x.is_finite() && x>0.0}

/// Remaining marked-to-stop downside. A protected historical profit is not
/// counted from its old entry price. Missing exposure protection is unknown.
fn downside(side:Side, current:f64, stop:Option<f64>, volume:f64)->Result<f64,BudgetError> {
    if !positive(current) || !positive(volume) {return Err(BudgetError::InvalidExposure);}
    let stop=stop.filter(|s|positive(*s)).ok_or(BudgetError::MissingStop)?;
    let risk=((current-stop)*side.sign()).max(0.0)*XAU_CONTRACT*volume;
    if !risk.is_finite(){Err(BudgetError::InvalidExposure)}else{Ok(risk)}
}

/// None = disabled/not armed, preserving the legacy portfolio arithmetic.
/// `reclaim_pending` is exclusively a PLANNING allowance for modifiable orders
/// of one basket; actual sends MUST always pass None and remeasure the broker.
pub fn available<B:Broker>(cfg:&Settings, anchor:BudgetAnchor, b:&B,
    reclaim_pending:Option<u32>)->Result<Option<AvailableBudget>,BudgetError> {
    if cfg.profit_budget_arm_pct==0.0 {return Ok(None);}
    if !positive(cfg.profit_budget_arm_pct)
        || !cfg.profit_budget_keep_pct.is_finite() || !(0.0..=100.0).contains(&cfg.profit_budget_keep_pct)
        || !cfg.profit_budget_deploy_pct.is_finite() || !(0.0..=100.0).contains(&cfg.profit_budget_deploy_pct) {
        return Err(BudgetError::InvalidSettings);
    }
    let q=b.quote(); let equity=b.account().equity;
    if !equity.is_finite() {return Err(BudgetError::InvalidAccount);}
    if anchor.day!=day_of(q.ts,cfg.session_offset()) || !positive(anchor.start)
        || !positive(anchor.peak) {return Err(BudgetError::UnknownDayAnchor);}
    let peak=anchor.peak.max(equity); let profit=(peak-anchor.start).max(0.0);
    if profit<=0.0 || profit<anchor.start*cfg.profit_budget_arm_pct/100.0 {return Ok(None);}
    if !positive(q.bid) || !positive(q.ask) || q.ask<q.bid {return Err(BudgetError::InvalidQuote);}
    let floor=anchor.start+profit*cfg.profit_budget_keep_pct/100.0;
    let mut capacity=(equity-floor).max(0.0)*cfg.profit_budget_deploy_pct/100.0;
    if !cfg.max_portfolio_risk_pct.is_finite(){return Err(BudgetError::InvalidSettings);}
    if cfg.max_portfolio_risk_pct>0.0 {
        capacity=capacity.min(equity.max(0.0)*cfg.max_portfolio_risk_pct/100.0);
    }
    let mut used=0.0;
    for p in b.positions().iter().chain(b.ukryte_pozycje()) {
        used+=downside(p.side,q.exit(p.side),p.sl.or(p.vsl),p.volume)?;
    }
    for p in b.pendings().iter().chain(b.ukryte_zlecenia()) {
        if reclaim_pending.is_some() && p.basket==reclaim_pending && !p.frozen {continue;}
        used+=downside(p.kind.side(),p.price,p.sl,p.volume)?;
    }
    if ![floor,capacity,used].into_iter().all(f64::is_finite) {return Err(BudgetError::InvalidExposure);}
    Ok(Some(AvailableBudget{floor,capacity,downside:used,remaining:(capacity-used).max(0.0)}))
}

/// Re-evaluate immediately before EVERY broker send, then floor to its actual
/// lot step. No rounding to lot_min, no reservation reclaimed before cancel ACK.
pub fn limit_open_volume<B:Broker>(cfg:&Settings,anchor:BudgetAnchor,b:&B,
    side:Side,entry:f64,stop:Option<f64>,requested:f64)->Result<f64,BudgetError> {
    let Some(budget)=available(cfg,anchor,b,None)? else {return Ok(requested)};
    if budget.remaining<=0.0{return Err(BudgetError::Exhausted);}
    let entry=b.normalize_order_price(entry);
    let stop=stop.map(|s|b.normalize_order_price(s)).filter(|s|positive(*s)).ok_or(BudgetError::MissingStop)?;
    if !positive(entry) || (entry-stop)*side.sign()<=0.0 {return Err(BudgetError::InvalidNewStop);}
    if !positive(requested){return Err(BudgetError::InvalidVolume);}
    let per_lot=(entry-stop)*side.sign()*XAU_CONTRACT;
    let capped=requested.min(budget.remaining/per_lot);
    let a=b.account();
    let volume=normalize_open_volume(capped,
        VolumeSpec{minimum:b.volume_min(),step:b.volume_step(),maximum:b.volume_max()},
        StrategyVolumeLimits{minimum:cfg.lot_min,maximum:cfg.lot_max,
            capital_per_lot:cfg.lot_max_z_salda,capital:cfg.podstawa_lota_z_konta(a.balance,a.equity,a.credit)})
        .map_err(|e|match e {crate::volume_contract::VolumeError::BelowMinimum=>BudgetError::Exhausted,
            _=>BudgetError::InvalidVolume})?;
    let risk=per_lot*volume;
    if !risk.is_finite() || risk>budget.remaining+16.0*f64::EPSILON*budget.remaining.max(1.0) {
        return Err(BudgetError::Exhausted);
    }
    Ok(volume)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::broker::{BResult,OrderReq,PendingReq};
    use crate::{Account,Quote,Position,PendingOrder,Ticket,Px,CloseReason,ClosedTrade,PendingKind};
    pub(crate) struct TestBroker {
        pub q:Quote,pub equity:f64,pub positions:Vec<Position>,pub pendings:Vec<PendingOrder>,
        pub hidden_positions:Vec<Position>,pub hidden_pendings:Vec<PendingOrder>,
        pub minimum:f64,pub step:f64,pub maximum:f64,pub sends:usize,pub price_digits:Option<i32>,
    }
    pub(crate) fn broker()->TestBroker {TestBroker {q:Quote{ts:1_700_000_000_000,bid:4000.0,ask:4000.2},
        equity:700.0,positions:vec![],pendings:vec![],hidden_positions:vec![],hidden_pendings:vec![],
        minimum:0.01,step:0.01,maximum:100.0,sends:0,price_digits:None}}
    pub(crate) fn cfg()->Settings {Settings{profit_budget_arm_pct:10.0,profit_budget_keep_pct:50.0,
        profit_budget_deploy_pct:100.0,max_portfolio_risk_pct:0.0,lot_min:0.01,lot_max:0.0,
        risk_per_basket_pct:0.0,..Settings::default()}}
    pub(crate) fn anchor(b:&TestBroker,c:&Settings)->BudgetAnchor {
        BudgetAnchor{day:day_of(b.q.ts,c.session_offset()),start:600.0,peak:700.0}
    }
    pub(crate) fn position(side:Side,open:f64,sl:Option<f64>,volume:f64)->Position {
        Position {ticket:1,side,volume,open_price:open,open_ts:1,sl,tp:None,vsl:None,basket:Some(1),
            level:0,frozen:false,peak_pts:0.0,last_peak_ts:0,is_runner:false,is_toucher:false,comment:String::new()}
    }
    pub(crate) fn pending(id:u32,side:Side,entry:f64,sl:Option<f64>,volume:f64)->PendingOrder {
        PendingOrder{ticket:1,kind:PendingKind::limit(side),volume,price:entry,sl,tp:None,placed_ts:1,
            basket:Some(id),level:0,frozen:false,is_toucher:false,is_topup:false,comment:String::new()}
    }
    impl Broker for TestBroker {
        fn quote(&self)->Quote{self.q}
        fn account(&self)->Account{Account{balance:self.equity,equity:self.equity,margin:0.0,
            free_margin:self.equity,leverage:500,credit:0.0}}
        fn stops_level(&self)->f64{0.0}
        fn volume_min(&self)->f64{self.minimum} fn volume_step(&self)->f64{self.step}
        fn volume_max(&self)->f64{self.maximum}
        fn normalize_order_price(&self,p:f64)->f64 {self.price_digits.map_or(p,|d|{let f=10f64.powi(d);(p*f).round()/f})}
        fn positions(&self)->&[Position]{&self.positions}
        fn pendings(&self)->&[PendingOrder]{&self.pendings}
        fn positions_mut(&mut self)->&mut Vec<Position>{&mut self.positions}
        fn pendings_mut(&mut self)->&mut Vec<PendingOrder>{&mut self.pendings}
        fn ukryte_pozycje(&self)->&[Position]{&self.hidden_positions}
        fn ukryte_zlecenia(&self)->&[PendingOrder]{&self.hidden_pendings}
        fn open_market(&mut self,r:OrderReq)->BResult<Ticket>{
            self.sends+=1;let ticket=self.sends as u64;
            let mut p=position(r.side,self.q.entry(r.side),r.sl,r.volume);
            p.ticket=ticket;p.basket=r.basket;p.level=r.level;p.tp=r.tp;
            self.equity-=self.q.spread()*XAU_CONTRACT*r.volume;self.positions.push(p);Ok(ticket)
        }
        fn place_pending(&mut self,r:PendingReq)->BResult<Ticket>{
            self.sends+=1;let ticket=self.sends as u64;
            let mut p=pending(r.basket.unwrap_or(0),r.kind.side(),r.price,r.sl,r.volume);
            p.ticket=ticket;p.kind=r.kind;p.basket=r.basket;p.level=r.level;p.tp=r.tp;
            self.pendings.push(p);Ok(ticket)
        }
        fn modify_position(&mut self,t:Ticket,sl:Option<Px>,tp:Option<Px>)->BResult<()>{
            let p=self.positions.iter_mut().find(|p|p.ticket==t).ok_or(crate::BrokerError::NoSuchTicket)?;
            p.sl=sl;p.tp=tp;Ok(())
        }
        fn modify_pending(&mut self,t:Ticket,price:Px,sl:Option<Px>,tp:Option<Px>)->BResult<()>{
            let p=self.pendings.iter_mut().find(|p|p.ticket==t).ok_or(crate::BrokerError::NoSuchTicket)?;
            p.price=price;p.sl=sl;p.tp=tp;Ok(())
        }
        fn cancel_pending(&mut self,t:Ticket)->BResult<()>{self.pendings.retain(|p|p.ticket!=t);Ok(())}
        fn close_position(&mut self,t:Ticket,_r:CloseReason)->BResult<f64>{self.positions.retain(|p|p.ticket!=t);Ok(0.0)}
        fn close_partial(&mut self,_t:Ticket,_v:f64,_r:CloseReason)->BResult<f64>{Ok(0.0)}
        fn drain_closed(&mut self)->Vec<ClosedTrade>{vec![]}
    }
    fn near(a:f64,b:f64){assert!((a-b).abs()<1e-8,"{a} != {b}");}
    #[test]
    fn profit_budget_disabled_and_unarmed_are_exact_legacy_passthrough() {
        let mut c=cfg();let mut b=broker();let a=anchor(&b,&c);
        c.profit_budget_arm_pct=0.0;c.profit_budget_keep_pct=f64::NAN;b.maximum=f64::NAN;
        assert_eq!(limit_open_volume(&c,BudgetAnchor::default(),&b,Side::Buy,f64::NAN,None,5.123),Ok(5.123));
        c=cfg();c.profit_budget_arm_pct=20.0;
        assert_eq!(available(&c,a,&b,None),Ok(None));
        assert_eq!(limit_open_volume(&c,a,&b,Side::Buy,4000.0,None,5.123),Ok(5.123));
    }
    #[test]
    fn profit_budget_marks_both_sides_and_counts_hidden_exposure_without_old_entry_bias() {
        let c=cfg();let mut b=broker();let a=anchor(&b,&c);
        b.positions.push(position(Side::Buy,3900.0,Some(3999.0),0.1));
        b.positions.push(position(Side::Sell,4100.0,Some(4001.0),0.1));
        b.pendings.push(pending(1,Side::Buy,3990.0,Some(3980.0),0.01));
        b.hidden_pendings.push(pending(2,Side::Sell,4010.0,Some(4020.0),0.01));
        b.hidden_positions.push(position(Side::Buy,3800.0,Some(3999.0),0.01));
        let v=available(&c,a,&b,None).unwrap().unwrap();
        near(v.floor,650.0);near(v.capacity,50.0);near(v.downside,39.0);near(v.remaining,11.0);
        b.positions[0].open_price=100.0;b.positions[1].open_price=9000.0;
        near(available(&c,a,&b,None).unwrap().unwrap().remaining,11.0);
    }
    #[test]
    fn profit_budget_arm_boundary_floor_and_portfolio_share_the_same_downside_pool() {
        let mut c=cfg();let mut b=broker();let mut a=anchor(&b,&c);
        c.profit_budget_arm_pct=20.0;b.equity=719.0;a.peak=719.0;
        assert_eq!(available(&c,a,&b,None).unwrap(),None);
        b.equity=720.0;a.peak=720.0;
        near(available(&c,a,&b,None).unwrap().unwrap().remaining,60.0);
        c.max_portfolio_risk_pct=5.0;
        near(available(&c,a,&b,None).unwrap().unwrap().remaining,36.0);
        b.positions.push(position(Side::Buy,3000.0,Some(3999.0),0.1));
        near(available(&c,a,&b,None).unwrap().unwrap().remaining,26.0);
        c.profit_budget_deploy_pct=10.0;
        near(available(&c,a,&b,None).unwrap().unwrap().remaining,0.0);
        c.profit_budget_keep_pct=100.0;c.profit_budget_deploy_pct=100.0;
        assert_eq!(limit_open_volume(&c,a,&b,Side::Buy,4000.2,Some(3990.0),1.0),Err(BudgetError::Exhausted));
    }
    #[test]
    fn profit_budget_missing_stop_invalid_anchor_and_nonfinite_inputs_fail_closed() {
        let mut c=cfg();let mut b=broker();let a=anchor(&b,&c);
        assert_eq!(available(&c,BudgetAnchor::default(),&b,None),Err(BudgetError::UnknownDayAnchor));
        b.positions.push(position(Side::Buy,3900.0,None,0.01));
        assert_eq!(available(&c,a,&b,None),Err(BudgetError::MissingStop));
        b.positions[0].vsl=Some(3990.0);assert!(available(&c,a,&b,None).is_ok());
        b.positions.clear();b.pendings.push(pending(1,Side::Buy,3990.0,None,0.01));
        assert_eq!(available(&c,a,&b,None),Err(BudgetError::MissingStop));b.pendings.clear();
        assert_eq!(limit_open_volume(&c,a,&b,Side::Buy,4000.2,None,1.0),Err(BudgetError::MissingStop));
        c.profit_budget_keep_pct=101.0;assert_eq!(available(&c,a,&b,None),Err(BudgetError::InvalidSettings));
        c=cfg();b.q.ask=f64::NAN;assert_eq!(available(&c,a,&b,None),Err(BudgetError::InvalidQuote));
    }
    #[test]
    fn profit_budget_lots_only_floor_to_broker_step_never_round_up_to_minimum() {
        let c=cfg();let mut b=broker();let a=anchor(&b,&c);
        near(limit_open_volume(&c,a,&b,Side::Buy,4000.2,Some(3990.0),1.0).unwrap(),0.04);
        near(limit_open_volume(&c,a,&b,Side::Sell,4000.0,Some(4010.0),1.0).unwrap(),0.05);
        b.step=0.03;b.minimum=0.03;
        near(limit_open_volume(&c,a,&b,Side::Buy,4000.2,Some(3990.0),1.0).unwrap(),0.03);
        b.step=0.1;b.minimum=0.1;
        assert_eq!(limit_open_volume(&c,a,&b,Side::Buy,4000.2,Some(3990.0),1.0),Err(BudgetError::Exhausted));
        b.step=0.01;b.minimum=0.01;b.maximum=f64::NAN;
        assert_eq!(limit_open_volume(&c,a,&b,Side::Buy,4000.2,Some(3990.0),1.0),Err(BudgetError::InvalidVolume));
    }
    #[test]
    fn profit_budget_values_the_rounded_broker_stop_before_choosing_the_lot_step() {
        let c=cfg();let mut b=broker();let a=anchor(&b,&c);b.equity=699.99;
        near(limit_open_volume(&c,a,&b,Side::Buy,4000.0,Some(3990.004),1.0).unwrap(),0.05);
        b.price_digits=Some(2);
        near(limit_open_volume(&c,a,&b,Side::Buy,4000.0,Some(3990.004),1.0).unwrap(),0.04);
        // 0.05 lots would consume $50 after broker rounding, above $49.99.
    }
    #[test]
    fn profit_budget_relot_reclaim_is_planning_only_and_frozen_orders_remain_counted() {
        let c=cfg();let mut b=broker();let a=anchor(&b,&c);
        b.pendings.push(pending(7,Side::Buy,3990.0,Some(3980.0),0.04));
        near(available(&c,a,&b,Some(7)).unwrap().unwrap().remaining,50.0);
        near(available(&c,a,&b,None).unwrap().unwrap().remaining,10.0);
        near(limit_open_volume(&c,a,&b,Side::Buy,3990.0,Some(3980.0),1.0).unwrap(),0.01);
        b.pendings[0].frozen=true;near(available(&c,a,&b,Some(7)).unwrap().unwrap().remaining,10.0);
        b.pendings.clear();near(limit_open_volume(&c,a,&b,Side::Buy,3990.0,Some(3980.0),1.0).unwrap(),0.05);
    }
}
