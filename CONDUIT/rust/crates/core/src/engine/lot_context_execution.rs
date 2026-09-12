//! The ten sizing axes observe the broker at the final new-order boundary.
//! They share no future bars, do not move day anchors and never edit exits.
use super::*;
use crate::lot_context::{LotStressContext,AXIS_COUNT,FIELD_NAMES};

impl Engine {
    fn growth_strengths(&self)->[f64;AXIS_COUNT] {
        let c=&self.cfg;
        [c.lot_growth_equity_stress_strength,c.lot_growth_portfolio_load_strength,
            c.lot_growth_direction_load_strength,c.lot_growth_basket_count_strength,
            c.lot_growth_spread_stress_strength,c.lot_growth_tp1_deficit_strength,
            c.lot_growth_stop_width_strength,c.lot_growth_age_decay_strength,
            c.lot_growth_rearm_decay_strength,c.lot_growth_day_dd_strength]
    }

    fn growth_stress_context<B:Broker>(&self,b:&B,basket:Option<u32>,side:Side,
        entry:Px,sl:Option<Px>,s:[f64;AXIS_COUNT])->LotStressContext {
        let mut c=LotStressContext::default();
        if [0,1,2,9].iter().any(|i|s[*i]!=0.0) {
            let a=b.account();c.equity=Some(a.equity);c.balance=Some(a.balance);
        }
        let q=b.quote();
        let valid_quote=q.bid.is_finite()&&q.ask.is_finite()&&q.bid>0.0&&q.ask>=q.bid;
        if s[4]!=0.0 && valid_quote {c.spread=Some(q.ask-q.bid);}
        let entry=b.normalize_order_price(entry);
        if [4,5,6].iter().any(|i|s[*i]!=0.0) && entry.is_finite()&&entry>0.0 {
            c.stop_distance=sl.map(|v|b.normalize_order_price(v))
                .filter(|v|v.is_finite()&&*v>0.0).map(|v|(entry-v)*side.sign());
        }
        if let Some(bk)=basket.and_then(|id|self.basket(id)) {
            if bk.entry_lo.is_finite()&&bk.entry_hi.is_finite()&&bk.entry_lo>0.0&&bk.entry_hi>=bk.entry_lo {
                c.zone_width=Some(bk.entry_hi-bk.entry_lo);
            }
            if entry.is_finite()&&entry>0.0 {
                c.tp1_reward_distance=bk.tps.first().copied()
                    .filter(|v|v.is_finite()&&*v>0.0)
                    .map(|v|(b.normalize_order_price(v)-entry)*side.sign());
            }
            if bk.created_ts<=q.ts {c.basket_age_secs=Some((q.ts-bk.created_ts) as f64/1000.0);}
            c.completed_rearms=Some(bk.rearms as f64);
        }
        if s[9]!=0.0 && self.stats.day==day_of(q.ts,self.cfg.session_offset()) {
            c.day_start_equity=Some(self.stats.day_start_equity);
        }
        if s[1]!=0.0 || s[2]!=0.0 || s[3]!=0.0 {
            let mut ids=std::collections::HashSet::new();
            let (mut total,mut same)=(0.0,0.0);
            let (mut identity_ok,mut total_ok,mut same_ok)=(true,valid_quote,valid_quote);
            let mut row=|owner:Option<u32>,row_side:Side,price:f64,stop:Option<f64>,volume:f64| {
                if let Some(id)=owner {ids.insert(id);} else {identity_ok=false;}
                if s[1]==0.0 && (s[2]==0.0 || row_side!=side) {return;}
                let risk=if price.is_finite()&&price>0.0&&volume.is_finite()&&volume>0.0 {
                    stop.filter(|v|v.is_finite()&&*v>0.0).map(|v|((price-v)*row_side.sign()).max(0.0)*XAU_CONTRACT*volume)
                        .filter(|v|v.is_finite())
                } else {None};
                if let Some(risk)=risk {total+=risk;if row_side==side {same+=risk;}}
                else {total_ok=false;if row_side==side {same_ok=false;}}
            };
            for p in b.positions().iter().chain(b.ukryte_pozycje()) {
                row(p.basket,p.side,q.exit(p.side),p.sl.or(p.vsl),p.volume);
            }
            for p in b.pendings().iter().chain(b.ukryte_zlecenia()) {
                row(p.basket,p.kind.side(),p.price,p.sl,p.volume);
            }
            if identity_ok {
                c.active_baskets=Some(ids.len() as f64);
                if total_ok&&total.is_finite() {c.portfolio_sl_risk=Some(total);}
                if same_ok&&same.is_finite() {c.same_side_sl_risk=Some(same);}
            }
        }
        c
    }

    pub(super) fn growth_context_volume<B:Broker>(&mut self,b:&B,basket:Option<u32>,side:Side,
        entry:Px,sl:Option<Px>,requested:f64)->BResult<f64> {
        if !crate::lot_growth::enabled(&self.cfg) {return Ok(requested);}
        let strengths=self.growth_strengths();
        // Do not even ask for a broker snapshot when all axes are disabled.
        if strengths.iter().all(|v|*v==0.0) {return Ok(requested);}
        let context=self.growth_stress_context(b,basket,side,entry,sl,strengths);
        match crate::lot_context::evaluate(strengths,&context) {
            Ok(outcome)=>{
                if let Some(i)=outcome.dominant_axis {
                    *self.stats.lot_sizing_diagnostics.entry(format!("LotContext::Dominant::{}",FIELD_NAMES[i])).or_insert(0)+=1;
                }
                for (i,strength) in strengths.iter().enumerate() {if *strength>0.0 {
                    *self.stats.lot_sizing_diagnostics.entry(format!("LotContext::Observed::{}",FIELD_NAMES[i])).or_insert(0)+=1;
                }}
                if self.journal.wants(EventLevel::Info) {
                    let mut ev=Ev::new(b.quote().ts,EventLevel::Info,EventCategory::Risk,EventKind::Note)
                        .text("LOT CONTEXT: causal pre-send sizing observation")
                        .put_f("requested_volume",requested).put_f("multiplier",outcome.multiplier)
                        .put("strengths",serde_json::json!(strengths)).put("factors",serde_json::json!(outcome.factors))
                        .put("context",serde_json::json!({"equity":context.equity,"balance":context.balance,
                            "portfolio_sl_risk":context.portfolio_sl_risk,"same_side_sl_risk":context.same_side_sl_risk,
                            "active_baskets":context.active_baskets,"spread":context.spread,"stop_distance":context.stop_distance,
                            "tp1_reward_distance":context.tp1_reward_distance,"zone_width":context.zone_width,
                            "basket_age_secs":context.basket_age_secs,"completed_rearms":context.completed_rearms,
                            "day_start_equity":context.day_start_equity}));
                    if let Some(id)=basket {ev=ev.basket(id);}
                    self.journal.push(ev.build());
                }
                crate::lot_growth::apply_surplus(&self.cfg,b,requested,outcome.multiplier)
                    .map_err(|reason|self.growth_error(b,reason))
            }
            Err(reason)=>{
                *self.odrzuty.entry(format!("LotContext::{reason:?}")).or_insert(0)+=1;
                self.log(b.quote().ts,1,format!("LOT CONTEXT HOLD: {reason:?}; new order withheld"));
                Err(BrokerError::Rejected)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profit_budget::tests::{broker,position,pending};
    fn engine()->Engine {Engine::new(Settings{lot_growth_mode:crate::lot_growth::LotGrowthMode::Power,
        lot_growth_allocation:crate::lot_growth::LotGrowthAllocation::Uniform,..Settings::default()},700.0)}
    #[test]
    fn lot_growth_context_zero_ignores_missing_snapshots_and_off_ignores_strengths() {
        let mut e=engine();let mut b=broker();b.q.ask=f64::NAN;b.equity=f64::NAN;
        assert_eq!(e.growth_context_volume(&b,None,Side::Buy,f64::NAN,None,0.123),Ok(0.123));
        assert!(e.odrzuty.is_empty());
        e.cfg.lot_growth_mode=crate::lot_growth::LotGrowthMode::Off;
        e.cfg.lot_growth_portfolio_load_strength=f64::NAN;
        assert_eq!(e.growth_context_volume(&b,None,Side::Buy,f64::NAN,None,0.123),Ok(0.123));
    }
    #[test]
    fn lot_growth_context_counts_actual_distinct_hidden_owners_and_marked_side_risk() {
        let e=engine();let mut b=broker();
        b.positions.push(position(Side::Buy,3900.0,Some(3999.0),0.1));
        b.hidden_positions.push(position(Side::Sell,4100.0,Some(4001.0),0.1));
        b.pendings.push(pending(2,Side::Buy,3990.0,Some(3980.0),0.02));
        b.hidden_pendings.push(pending(3,Side::Sell,4010.0,Some(4020.0),0.01));
        let mut strengths=[0.0;10];strengths[1]=1.0;strengths[2]=1.0;strengths[3]=1.0;
        let c=e.growth_stress_context(&b,Some(1),Side::Buy,4000.2,Some(3990.0),strengths);
        assert_eq!(c.active_baskets,Some(3.0));
        assert!((c.portfolio_sl_risk.unwrap()-48.0).abs()<1e-9);
        assert!((c.same_side_sl_risk.unwrap()-30.0).abs()<1e-9);
        b.hidden_positions[0].sl=None;
        let c=e.growth_stress_context(&b,Some(1),Side::Buy,4000.2,Some(3990.0),strengths);
        assert_eq!(c.portfolio_sl_risk,None);assert_eq!(c.same_side_sl_risk,Some(30.0));
    }
    #[test]
    fn lot_growth_context_current_day_anchor_required_without_mutating_it() {
        let mut e=engine();let b=broker();e.cfg.lot_growth_day_dd_strength=1.0;
        e.stats.day=day_of(b.q.ts,0)-1;e.stats.day_start_equity=1000.0;
        assert!(e.growth_context_volume(&b,Some(1),Side::Buy,4000.2,Some(3990.0),0.08).is_err());
        e.stats.day=day_of(b.q.ts,0);
        let volume=e.growth_context_volume(&b,Some(1),Side::Buy,4000.2,Some(3990.0),0.08).unwrap();
        assert!((volume-0.0275).abs()<1e-12);
        assert_eq!(e.stats.day_start_equity,1000.0);
    }
    #[test]
    fn lot_growth_context_missing_enabled_geometry_is_hold_and_steps_are_floor_only() {
        let mut e=engine();let mut b=broker();e.cfg.lot_growth_tp1_deficit_strength=1.0;
        assert!(e.growth_context_volume(&b,None,Side::Buy,4000.2,Some(3990.0),0.08).is_err());
        e.cfg.lot_growth_tp1_deficit_strength=0.0;e.cfg.lot_growth_spread_stress_strength=1.0;
        let p=PendingReq{kind:PendingKind::BuyLimit,price:3990.0,volume:0.01,sl:Some(3980.0),
            tp:Some(4010.0),basket:Some(1),level:0,is_toucher:false,is_topup:false,no_market_fallback:false,comment:"synthetic".into()};
        e.place_pending_order(&mut b,p.clone()).unwrap();assert_eq!(b.sends,1);
        let mut below=p;below.volume=0.005;
        assert!(e.place_pending_order(&mut b,below).is_err());assert_eq!(b.sends,1);
        assert!(e.stats.lot_sizing_diagnostics.contains_key("LotContext::Observed::lot_growth_spread_stress_strength"));
    }
}
