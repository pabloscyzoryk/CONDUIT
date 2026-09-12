//! Explicit broker execution hours. Quotes/messages retain their original clock.
//! MQL5 SymbolInfoSessionTrade returns weekday-local seconds, including 86400.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDay {
    pub day_sun0: u32,
    pub trade: Vec<[u32; 2]>,
    /// Audit metadata only: never used to filter physical observations.
    pub quote: Vec<[u32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradeSessionProfile {
    pub schema: String,
    pub clock: String,
    pub days: Vec<SessionDay>,
    /// Native TP executes at the improved available quote after a gap.
    /// Explicit rather than silently changing historical fixed-TP arithmetic.
    pub take_profit_price_improvement: bool,
}

impl TradeSessionProfile {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != "conduit.trade-sessions.v1" || self.clock != "broker" {
            return Err("execution sessions require conduit.trade-sessions.v1 and broker clock".into());
        }
        let mut seen = [false; 7];
        for day in &self.days {
            if day.day_sun0 > 6 || seen[day.day_sun0 as usize] {
                return Err("each execution-profile weekday must occur exactly once".into());
            }
            seen[day.day_sun0 as usize] = true;
            for intervals in [&day.trade, &day.quote] {
                let mut previous_end = 0;
                for &[from, to] in intervals {
                    if from >= 86400 || to <= from || to > 172800 || from < previous_end {
                        return Err("invalid, unsorted or overlapping session seconds".into());
                    }
                    previous_end = to;
                }
            }
        }
        if seen.iter().any(|day| !day) {
            return Err("all seven weekdays are required, including explicitly closed days".into());
        }
        // Also reject spill-over overlap with the following day's first session.
        for day in &self.days {
            let next = self.days.iter().find(|x| x.day_sun0 == (day.day_sun0 + 1) % 7).unwrap();
            for (today, tomorrow) in [(&day.trade, &next.trade), (&day.quote, &next.quote)] {
                if let (Some(last), Some(first)) = (today.last(), tomorrow.first()) {
                    if last[1] > 86400 + first[0] {
                        return Err("session overlaps the following weekday".into());
                    }
                }
            }
        }
        Ok(())
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let raw = std::fs::read(path)?;
        let profile: Self = serde_json::from_slice(&raw)?;
        profile.validate().map_err(anyhow::Error::msg)?;
        Ok(profile)
    }

    pub fn is_open(&self, broker_ts_ms: i64) -> bool {
        let date = broker_ts_ms.div_euclid(86_400_000);
        let weekday = (date + 4).rem_euclid(7) as u32;
        let second_ms = broker_ts_ms.rem_euclid(86_400_000) as u64;
        self.days.iter().any(|day| {
            let elapsed = if day.day_sun0 == weekday { second_ms }
                else if (day.day_sun0 + 1) % 7 == weekday { second_ms + 86_400_000 }
                else { return false; };
            day.trade.iter().any(|&[from, to]| elapsed >= from as u64 * 1000 && elapsed < to as u64 * 1000)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn profile() -> TradeSessionProfile {
        TradeSessionProfile {schema:"conduit.trade-sessions.v1".into(), clock:"broker".into(),
            days:(0..7).map(|day_sun0| SessionDay {day_sun0,
                trade:if (1..=5).contains(&day_sun0) {vec![[3660, if day_sun0==5 {86220} else {86280}]]} else {vec![]},
                quote:vec![]}).collect(), take_profit_price_improvement:true}
    }
    #[test]
    fn native_weekday_boundaries_use_broker_time_without_dropping_quotes() {
        let p=profile();p.validate().unwrap();let midnight=1785196800000;
        assert!(!p.is_open(midnight+3_660_000-1));
        assert!(p.is_open(midnight+3_660_000));
        assert!(p.is_open(midnight+86_280_000-1));
        assert!(!p.is_open(midnight+86_280_000));
        assert!(!p.is_open(1785628800000+10_000_000)); // Sunday
    }
    #[test]
    fn overnight_intervals_keep_86400_and_wrap_week_boundary() {
        let mut p=profile();for day in &mut p.days {day.trade.clear();}
        p.days[6].trade=vec![[82800,90000]];p.validate().unwrap();
        let sunday=3*86_400_000; // 1970-01-04
        assert!(p.is_open(sunday+3_599_999));assert!(!p.is_open(sunday+3_600_000));
    }
    #[test]
    fn malformed_or_incomplete_profile_never_becomes_always_open() {
        let mut p=profile();p.days.pop();assert!(p.validate().is_err());
        let mut p=profile();p.clock="utc".into();assert!(p.validate().is_err());
        let mut p=profile();p.days[1].trade=vec![[0,0]];assert!(p.validate().is_err());
    }

    #[test]
    fn native_july_28_quote_only_minute_defers_fills_and_server_protection() {
        use crate::sim::SimBroker;
        use conduit_core::{broker::*, types::*};
        let mut b=SimBroker::new(300.0,0.20,0.0);b.price_digits=Some(2);
        b.limit_price_improvement=true;b.defer_new_pending_sl=true;
        b.set_trade_sessions(Some(profile())).unwrap();
        b.on_quote(Quote{ts:1785196675021,bid:4076.60,ask:4076.98});
        let request=|side,sl,tp| OrderReq{side,sl,tp,volume:0.01,basket:None,level:0,is_toucher:false,comment:"native-session".into()};
        let sell=b.open_market(request(Side::Sell,Some(4078.0),None)).unwrap();
        let buy=b.open_market(request(Side::Buy,None,Some(4078.0))).unwrap();
        let pending=|kind,price| PendingReq{kind,price,volume:0.01,sl:None,tp:None,basket:None,level:0,
            is_toucher:false,is_topup:false,no_market_fallback:false,comment:"native-session".into()};
        b.place_pending(pending(PendingKind::BuyStop,4078.0)).unwrap();
        b.place_pending(pending(PendingKind::SellLimit,4078.0)).unwrap();
        let idle=b.place_pending(pending(PendingKind::BuyLimit,4000.0)).unwrap();
        assert_eq!(b.on_quote(Quote{ts:1785200406021,bid:4079.60,ask:4080.00}),(0,0));
        assert_eq!((b.positions().len(),b.pendings().len()),(2,3));
        assert_eq!(b.quote().ts,1785200406021,"closed session still publishes the observation");
        assert_eq!(b.open_market(request(Side::Buy,None,None)),Err(BrokerError::MarketClosed));
        assert_eq!(b.place_pending(pending(PendingKind::BuyLimit,4078.0)),Err(BrokerError::MarketClosed));
        assert_eq!(b.modify_position(sell,Some(4079.0),None),Err(BrokerError::InvalidStops));
        assert_eq!(b.modify_position(sell,Some(4081.0),None),Err(BrokerError::MarketClosed));
        assert_eq!(b.modify_pending(idle,3999.0,None,None),Err(BrokerError::MarketClosed));
        assert_eq!(b.close_position(sell,CloseReason::Manual),Err(BrokerError::MarketClosed));
        assert_eq!(b.close_partial(buy,0.01,CloseReason::Partial),Err(BrokerError::MarketClosed));
        assert_eq!(b.cancel_pending(idle),Err(BrokerError::MarketClosed));
        assert_eq!((b.positions().len(),b.pendings().len()),(2,3));
        assert_eq!(b.on_quote(Quote{ts:1785200460244,bid:4081.83,ask:4082.22}),(2,2));
        let closed=b.drain_closed();
        assert_eq!(closed.len(),2);
        assert_eq!(closed[0].close_price,4082.22);assert_eq!(closed[0].reason,CloseReason::Sl);
        assert_eq!(closed[1].close_price,4081.83);assert_eq!(closed[1].reason,CloseReason::Tp);
        assert!(closed.iter().all(|t|t.close_ts==1785200460244));
        assert_eq!((b.positions().len(),b.pendings().len()),(2,1));
    }
}
