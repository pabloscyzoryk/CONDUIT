use std::collections::{BTreeMap,BTreeSet};
use serde::{Serialize,Deserialize};
use crate::{IncomingMessage,Signal};
use crate::types::{ClosedTrade,CloseReason,Position,Quote,Side,Ticket,Ts,XAU_CONTRACT};
use super::{Config,ContextBook,Diagnostics,EntryPlan,ExecutionOutcome,Features,Intent,MarketState,PortfolioView,REVISION};

#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
struct TradeMemory { plan:EntryPlan, remaining:f64, realized:f64 }

#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct Runtime {
    pub revision:String,
    pub market:MarketState,
    pub context:ContextBook,
    pub diagnostics:Diagnostics,
    trades:BTreeMap<Ticket,TradeMemory>,
    confirmed:BTreeSet<u64>, uncertain:BTreeSet<u64>,
    expert_mean_r:[f64;4], expert_samples:[u64;4],
    next_decision:u64, last_open_bar:Option<u64>, last_quote_ts:Option<Ts>,
    day:Option<i64>, day_start:f64, day_peak:f64, day_locked:bool,
}
impl Default for Runtime {
    fn default()->Self {Self {revision:REVISION.into(),market:MarketState::default(),context:ContextBook::default(),
        diagnostics:Diagnostics::default(),trades:BTreeMap::new(),confirmed:BTreeSet::new(),uncertain:BTreeSet::new(),
        expert_mean_r:[0.0;4],expert_samples:[0;4],next_decision:1,last_open_bar:None,last_quote_ts:None,
        day:None,day_start:0.0,day_peak:0.0,day_locked:false}}
}

impl Runtime {
    /// Import observed account-day anchors on activation or return from an
    /// inactive mode. Continuous trading still uses `on_tick` unchanged.
    /// Minimum proves a past loss from the day's start; without ordered equity
    /// observations it cannot prove that a giveback occurred after the peak.
    pub fn import_account_day(&mut self,day:i64,start:f64,peak:f64,minimum:f64,
        current_equity:f64,cfg:&Config)->Result<(),String> {
        if !cfg.valid() || ![start,peak,minimum,current_equity].iter().all(|v|v.is_finite())
            || start<=0.0 || current_equity<=0.0 || peak<start || peak<current_equity
            || minimum>start || minimum>current_equity {
            return Err("invalid_account_day_anchors".into());
        }
        if self.day.is_some_and(|old|day<old) {
            return Err("stale_account_day_anchors".into());
        }
        let same_day=self.day==Some(day);
        let merged_peak=if same_day {peak.max(self.day_peak)} else {peak};
        let loss=(start-minimum).max(0.0)/start*100.0;
        let profit=(merged_peak-start).max(0.0);
        let armed=cfg.daily_profit_lock_pct>0.0 && profit>=start*cfg.daily_profit_lock_pct/100.0;
        let floor=start+profit*(1.0-cfg.daily_giveback_pct/100.0);
        let locked=(same_day && self.day_locked) || loss>=cfg.daily_loss_pct
            || (armed && current_equity<floor);
        self.day=Some(day);self.day_start=start;self.day_peak=merged_peak;self.day_locked=locked;
        Ok(())
    }
    pub fn valid_state(&self)->bool {
        self.revision==REVISION && self.next_decision>0
            && self.confirmed.iter().chain(self.uncertain.iter()).all(|id|*id<self.next_decision)
            && self.last_open_bar.is_none_or(|v|v<=self.market.bar_sequence)
            && [self.day_start,self.day_peak].iter().chain(self.expert_mean_r.iter()).all(|v|v.is_finite())
            && self.trades.values().all(|m|m.plan.expert<4 && m.plan.volume.is_finite() && m.plan.volume>0.0
                && m.plan.approved_budget_usd.is_finite() && m.plan.approved_budget_usd>0.0
                && m.plan.risk_usd.is_finite() && m.plan.risk_usd>0.0 && m.realized.is_finite()
                && m.remaining.is_finite() && m.remaining>=0.0 && m.remaining<=m.plan.volume+1e-8
                && [m.plan.entry_reference,m.plan.sl,m.plan.tp,m.plan.atr].iter().all(|v|v.is_finite()&&*v>0.0))
    }
    pub fn observe_context(&mut self,m:&IncomingMessage,signals:&[Signal]) {
        self.context.observe(m,signals);self.diagnostics.contexts=self.context.contexts.len() as u64;
    }
    pub fn on_open_result(&mut self,plan:&EntryPlan,outcome:ExecutionOutcome,ticket:Option<Ticket>) {
        if self.confirmed.contains(&plan.decision_id) {return;}
        self.next_decision=self.next_decision.max(plan.decision_id.saturating_add(1));
        match outcome {
            ExecutionOutcome::Uncertain=>{if self.uncertain.insert(plan.decision_id){self.diagnostics.uncertain+=1;}},
            ExecutionOutcome::Rejected=>{self.uncertain.remove(&plan.decision_id);self.diagnostics.rejected+=1;},
            ExecutionOutcome::Confirmed=>{let Some(ticket)=ticket else{return;};
                self.confirmed.insert(plan.decision_id);self.uncertain.remove(&plan.decision_id);
                self.trades.insert(ticket,TradeMemory{plan:plan.clone(),remaining:plan.volume,realized:0.0});
                self.last_open_bar=Some(self.market.bar_sequence);self.diagnostics.opened+=1;
                if (plan.expert as usize)<4 {self.diagnostics.expert_entries[plan.expert as usize]+=1;}
                if let Some(key)=&plan.context_key {self.context.used(key);self.diagnostics.context_entries+=1;}
                else {self.diagnostics.market_only_entries+=1;}
            },
        }
    }
    /// Called once per verified broker receipt, including partial closes.
    /// Never learn from a backtest-only future label or an unconfirmed ACK.
    pub fn on_closed(&mut self,c:&ClosedTrade,net_profit:f64) {
        if !net_profit.is_finite() || !c.volume.is_finite() || c.volume<=0.0 {return;}
        let Some(memory)=self.trades.get_mut(&c.ticket) else{return;};
        memory.realized+=net_profit;memory.remaining=(memory.remaining-c.volume).max(0.0);
        if memory.remaining>1e-8 {return;}
        let memory=self.trades.remove(&c.ticket).unwrap();let index=memory.plan.expert as usize;
        if index>=4 || memory.plan.risk_usd<=0.0 {return;}
        let reward=(memory.realized/memory.plan.risk_usd).clamp(-2.0,3.0);
        // Fixed, slow adaptation with 20 neutral prior observations.
        self.expert_samples[index]+=1;let alpha=1.0/(20+self.expert_samples[index].min(80)) as f64;
        self.expert_mean_r[index]+=alpha*(reward-self.expert_mean_r[index]);
        self.diagnostics.expert_closed[index]+=1;
    }
    pub fn on_tick(&mut self,cfg:&Config,q:&Quote,p:PortfolioView<'_>)->Vec<Intent> {
        if !cfg.enabled {return Vec::new();}
        if !cfg.valid() || self.revision!=REVISION {self.diagnostics.last_reason="invalid_config_or_revision".into();return Vec::new();}
        self.diagnostics.quotes+=1;
        if !super::valid_quote(q) || self.last_quote_ts.is_some_and(|t|q.ts<t) {
            self.diagnostics.invalid_quotes+=1;return Vec::new();
        }
        self.last_quote_ts=Some(q.ts);
        let completed=match p.completed_bars {Some(bars)=>self.market.observe_complete(bars,q),None=>self.market.observe(q)};
        let account=p.account;
        if !account.equity.is_finite() || !account.credit.is_finite() || account.equity<=0.0 {
            self.diagnostics.last_reason="invalid_equity".into();return Vec::new();
        }
        // RDD anchor uses the first observed equity of the broker trading day.
        let day=q.ts.div_euclid(86_400_000);
        if self.day!=Some(day) {self.day=Some(day);self.day_start=account.equity;self.day_peak=account.equity;self.day_locked=false;}
        self.day_peak=self.day_peak.max(account.equity);
        let daily_loss=(self.day_start-account.equity).max(0.0)/self.day_start.max(1e-8)*100.0;
        let peak_profit=(self.day_peak-self.day_start).max(0.0);
        let lock_armed=cfg.daily_profit_lock_pct>0.0 && peak_profit>=self.day_start*cfg.daily_profit_lock_pct/100.0;
        let floor=self.day_start+peak_profit*(1.0-cfg.daily_giveback_pct/100.0);
        let utc=q.ts-i64::from(p.broker_utc_offset_hours)*3_600_000;
        let hour=utc.rem_euclid(86_400_000)/3_600_000;
        let weekday=(utc.div_euclid(86_400_000)+3).rem_euclid(7);
        let weekend=weekday>=5 || (weekday==4 && hour>=i64::from(cfg.friday_flat_utc));
        if daily_loss>=cfg.daily_loss_pct || (lock_armed && account.equity<floor) {self.day_locked=true;}
        if self.day_locked || weekend {
            self.diagnostics.last_reason=if weekend{"weekend_flat"}else{"daily_risk_stop"}.into();
            return p.positions.iter().filter(|v|!v.frozen).map(|v|Intent::Close{ticket:v.ticket,
                reason:if weekend{CloseReason::EodFlat}else{CloseReason::MaxDd}}).collect();
        }
        let Some(_)=completed else{return Vec::new();};
        self.diagnostics.closed_bars+=1;
        let Some(f)=self.market.features() else{
            self.diagnostics.blocked_warmup+=1;self.diagnostics.last_reason="market_warmup".into();return Vec::new();
        };
        self.diagnostics.last_atr=f.atr;
        if q.ts-f.bar.ts>120_000 {
            self.diagnostics.blocked_warmup+=1;self.diagnostics.last_reason="completed_bar_not_current".into();return Vec::new();
        }
        let mut intents=self.manage(cfg,q,&p,&f);
        // A pending close must settle before sizing another entry.
        if intents.iter().any(|i|matches!(i,Intent::Close{..})) {return intents;}
        if !p.entry_allowed || !self.uncertain.is_empty() {
            self.diagnostics.blocked_entry+=1;self.diagnostics.last_reason="execution_or_engine_hold".into();return intents;
        }
        if hour<i64::from(cfg.session_start_utc) || hour>=i64::from(cfg.session_end_utc) {
            self.diagnostics.blocked_session+=1;self.diagnostics.last_reason="outside_entry_session".into();return intents;
        }
        if f.atr<cfg.min_atr || q.spread()>cfg.spread_abs_max || q.spread()>cfg.spread_atr_max*f.atr || f.range_atr>cfg.shock_atr {
            self.diagnostics.blocked_spread+=1;self.diagnostics.last_reason="spread_or_volatility".into();return intents;
        }
        if p.positions.len()>=cfg.max_positions || self.last_open_bar.is_some_and(|bar|self.market.bar_sequence-bar<u64::from(cfg.cooldown_bars)) {
            self.diagnostics.blocked_risk+=1;self.diagnostics.last_reason="capacity_or_cooldown".into();return intents;
        }
        let mut candidates=Vec::new();
        for side in [Side::Buy,Side::Sell] {
            let sign=side.sign();let aligned=f.trend*sign;
            let context=self.context.best(cfg,q.ts,q.mid(),f.atr,side);
            let support=context.map(|(_,s)|s).unwrap_or(0.0);
            if cfg.signal_required && support<=0.0 {continue;}
            let influence=cfg.signal_weight*support;
            if cfg.experts&1!=0 && aligned>cfg.trend_threshold && f.body*sign>0.05
                && (f.bar.close-f.fast).abs()<f.atr*1.5
                && (if side==Side::Buy{f.bar.low<=f.fast+f.atr*0.3}else{f.bar.high>=f.fast-f.atr*0.3}) {
                candidates.push((0,side,0.50+0.16*aligned.min(1.5)+0.12*f.efficiency+influence,context));
            }
            let breakout=if side==Side::Buy {f.bar.close>f.prev_high+0.02*f.atr}else{f.bar.close<f.prev_low-0.02*f.atr};
            if cfg.experts&2!=0 && breakout && f.body*sign>0.20 && aligned> -0.10 {
                candidates.push((1,side,0.58+0.15*f.body.abs()+0.10*f.efficiency+influence,context));
            }
            let range_edge=f.zscore*sign< -1.25;
            let rejection=if side==Side::Buy{f.close_location>0.45}else{f.close_location<0.55};
            if cfg.experts&4!=0 && f.trend.abs()<cfg.range_threshold && f.efficiency<0.35 && range_edge && rejection {
                candidates.push((2,side,0.48+0.10*f.zscore.abs().min(2.5)+0.10*(1.0-f.efficiency)+influence,context));
            }
            if cfg.experts&8!=0 && support>0.0 && cfg.signal_weight>0.0 {
                if let Some((c,_))=context {
                    let touches=f.bar.low<=c.hi+0.35*f.atr && f.bar.high>=c.lo-0.35*f.atr;
                    if touches && rejection && f.body*sign> -0.05 && aligned> -0.7 {
                        candidates.push((3,side,0.55+0.12*f.body.abs()+influence,context));
                    }
                }
            }
        }
        for (expert,_,score,_) in &mut candidates {
            *score+=(self.expert_mean_r[*expert]*cfg.adaptation).clamp(-0.15,0.15);
        }
        candidates.sort_by(|a,b|b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)).then((a.1==Side::Sell).cmp(&(b.1==Side::Sell))));
        let Some((expert,side,score,context))=candidates.first().copied() else{
            self.diagnostics.blocked_score+=1;self.diagnostics.last_reason="no_setup".into();return intents;
        };
        self.diagnostics.last_score=score;
        if score<cfg.score_threshold {
            self.diagnostics.blocked_score+=1;self.diagnostics.last_reason="setup_below_threshold".into();return intents;
        }
        // Avoid opposing inventory and crowded entries at virtually identical prices.
        if p.positions.iter().any(|v|v.side!=side || (v.open_price-q.entry(side)).abs()<f.atr*0.75) {
            self.diagnostics.blocked_risk+=1;self.diagnostics.last_reason="correlated_inventory".into();return intents;
        }
        let entry=q.entry(side);let sign=side.sign();
        let stop_dist=(f.atr*cfg.stop_atr).max(q.spread()+p.stops_level+0.02);
        let mut sl=entry-sign*stop_dist;
        let attributed=context.filter(|(_,support)|*support>0.0 && cfg.signal_weight>0.0);
        if let Some((c,_))=attributed {if let Some(level)=c.sl {
            let d=(entry-level)*sign;
            // Source geometry is advisory; reject wrong-side or implausibly distant SL.
            if d>=stop_dist*0.8 && d<=stop_dist*1.5 && (q.exit(side)-level)*sign>p.stops_level {
                sl=level-sign*0.05*f.atr;
            }
        }}
        let risk_points=(entry-sl)*sign;
        let mut tp=entry+sign*risk_points*cfg.reward_risk;
        if let Some((c,_))=attributed {
            if let Some(level)=c.tps.iter().copied().filter(|t|(*t-entry)*sign>=risk_points*0.8 && (*t-entry)*sign<=risk_points*cfg.reward_risk*1.25)
                .min_by(|a,b|((a-entry)*sign-risk_points*cfg.reward_risk).abs().total_cmp(&((b-entry)*sign-risk_points*cfg.reward_risk).abs())) {
                tp=level-sign*0.05*f.atr;
            }
        }
        let equity=(account.equity-account.credit).max(0.0);
        let risk_existing=p.other_risk_usd+p.positions.iter().map(|v|position_risk(v,q)).sum::<f64>();
        let free_risk=(equity*cfg.portfolio_risk_pct/100.0-risk_existing).max(0.0);
        let dd_mult=(1.0-0.75*(daily_loss/cfg.daily_loss_pct)).clamp(0.25,1.0);
        let skill_mult=(1.0+cfg.adaptation*self.expert_mean_r[expert]).clamp(0.6,1.2);
        let confidence=(0.7+0.3*(score-cfg.score_threshold+0.5)).clamp(0.5,1.1);
        let budget=(equity*cfg.risk_pct/100.0*dd_mult*skill_mult*confidence).min(free_risk);
        let raw_lot=budget/(risk_points*XAU_CONTRACT);
        let max_margin=account.free_margin.max(0.0)*cfg.margin_budget_pct/100.0;
        let margin_lot=max_margin*f64::from(account.leverage.max(1))/(entry*XAU_CONTRACT);
        let cap=if p.lot_cap>0.0{p.lot_cap}else{f64::MAX};
        let grid_ok=[p.lot_min,p.lot_step,p.lot_max,p.stops_level,p.lot_cap].iter().all(|x|x.is_finite()&&*x>=0.0)
            && p.lot_min>0.0 && p.lot_step>0.0 && p.lot_max>=p.lot_min;
        let volume=if grid_ok {((raw_lot.min(margin_lot).min(cap).min(p.lot_max)+1e-10)/p.lot_step).floor()*p.lot_step}else{0.0};
        if !budget.is_finite() || !risk_existing.is_finite() || volume<p.lot_min-1e-9 || volume<=0.0
            || (q.exit(side)-sl)*sign<=p.stops_level || (tp-q.exit(side))*sign<=p.stops_level {
            self.diagnostics.blocked_risk+=1;self.diagnostics.last_reason="risk_below_broker_minimum".into();return intents;
        }
        let plan=EntryPlan {decision_id:self.next_decision,ts:q.ts,side,expert:expert as u8,volume,
            entry_reference:entry,sl,tp,approved_budget_usd:budget,
            risk_usd:risk_points*XAU_CONTRACT*volume,score,atr:f.atr,
            context_key:attributed.map(|(c,_)|c.key.clone())};
        self.next_decision+=1;self.diagnostics.decisions+=1;self.diagnostics.last_reason=format!("expert_{expert}_entry");
        intents.push(Intent::Open(plan));intents
    }
    fn manage(&self,cfg:&Config,q:&Quote,p:&PortfolioView<'_>,f:&Features)->Vec<Intent> {
        let mut intents=Vec::new();
        for v in p.positions.iter().filter(|v|!v.frozen) {
            let Some(memory)=self.trades.get(&v.ticket) else{continue;};
            let original=(memory.plan.entry_reference-memory.plan.sl).abs().max(1e-8);
            let profit=v.profit_pts(q);let r=profit/original;
            let age=(q.ts-v.open_ts).max(0)/60_000;
            let adverse=f.trend*v.side.sign()< -0.65 && f.body*v.side.sign()< -0.4;
            if age>=i64::from(cfg.max_hold_min) || (adverse && r<0.25 && age>=5) {
                intents.push(Intent::Close{ticket:v.ticket,reason:if adverse{CloseReason::RevExit}else{CloseReason::Stale}});continue;
            }
            let sign=v.side.sign();let mut stop=v.sl.unwrap_or(memory.plan.sl);
            if r>=cfg.break_even_r && cfg.break_even_r>0.0 {
                let be=v.open_price+sign*(q.spread()+0.02);
                if (be-stop)*sign>0.0 {stop=be;}
            }
            if r>=cfg.trail_start_r {
                let trend_mult=if f.trend*sign>cfg.trend_threshold && f.efficiency>0.4 {1.25}else{0.85};
                let target=q.exit(v.side)-sign*f.atr*cfg.trail_atr*trend_mult;
                if (target-stop)*sign>0.0 {stop=target;}
            }
            if v.sl.is_none_or(|old|(stop-old)*sign>(0.1*f.atr).max(0.02))
                && (q.exit(v.side)-stop)*sign>p.stops_level+0.01 {
                intents.push(Intent::Modify{ticket:v.ticket,sl:stop,tp:v.tp});
            }
        }
        intents
    }
}
fn position_risk(v:&Position,q:&Quote)->f64 {
    v.sl.or(v.vsl).filter(|sl|sl.is_finite()&&*sl>0.0)
        .map(|sl|((q.exit(v.side)-sl)*v.side.sign()).max(0.0)*XAU_CONTRACT*v.volume)
        .unwrap_or(f64::INFINITY)
}

#[cfg(test)] mod tests {
    use super::*;use crate::types::Account;
    fn account()->Account {Account{balance:600.0,equity:600.0,margin:0.0,free_margin:600.0,leverage:500,credit:0.0}}
    fn view<'a>(a:&'a Account,p:&'a[Position])->PortfolioView<'a> {PortfolioView{account:a,positions:p,entry_allowed:true,other_risk_usd:0.0,
        lot_min:0.01,lot_step:0.01,lot_max:100.0,lot_cap:5.0,stops_level:0.0,broker_utc_offset_hours:3,completed_bars:None}}
    fn q(i:i64)->Quote {let x=i as f64;let price=2400.0+0.06*x+(x*0.7).sin()*1.6;
        Quote{ts:1_783_320_000_000+i*30_000,bid:price,ask:price+0.1}}
    fn cfg()->Config {Config{enabled:true,score_threshold:0.4,session_start_utc:0,session_end_utc:24,friday_flat_utc:24,risk_pct:4.0,portfolio_risk_pct:8.0,..Config::default()}}
    #[test] fn disabled_policy_is_exact_noop() {
        let mut s=Runtime::default();let before=s.clone();let a=account();assert!(s.on_tick(&Config::default(),&q(0),view(&a,&[])).is_empty());assert_eq!(s,before);
    }
    #[test] fn autonomous_entries_do_not_require_any_signal_and_cap_is_hard() {
        let mut s=Runtime::default();let a=account();let mut opened=0;
        for i in 0..1000 {let mut v=view(&a,&[]);v.lot_cap=0.01;
            for intent in s.on_tick(&cfg(),&q(i),v) {if let Intent::Open(p)=intent {opened+=1;assert!(p.volume<=0.01+1e-9);assert!(p.context_key.is_none());assert!(p.sl<p.entry_reference);}}
        }assert!(opened>0,"autonomous setup fixture must exercise real entries");
    }
    #[test] fn no_entry_when_ack_uncertain_or_engine_hold() {
        let mut s=Runtime::default();let a=account();s.uncertain.insert(7);
        for i in 0..200 {assert!(!s.on_tick(&cfg(),&q(i),view(&a,&[])).iter().any(|x|matches!(x,Intent::Open(_))));}
        s.uncertain.clear();for i in 200..300 {let mut v=view(&a,&[]);v.entry_allowed=false;assert!(!s.on_tick(&cfg(),&q(i),v).iter().any(|x|matches!(x,Intent::Open(_))));}
    }
    #[test] fn approved_risk_budget_precedes_lot_floor_and_respects_remaining_portfolio_risk() {
        let mut small=Runtime::default();let mut larger=Runtime::default();let a=account();let c=cfg();
        let mut entries=0;
        for i in 0..1000 {
            let mut low=view(&a,&[]);low.lot_cap=0.01;low.other_risk_usd=35.5;
            let mut high=view(&a,&[]);high.lot_cap=0.02;high.other_risk_usd=35.5;
            let x=small.on_tick(&c,&q(i),low);let y=larger.on_tick(&c,&q(i),high);
            for (left,right) in x.iter().zip(&y) {
                if let (Intent::Open(p),Intent::Open(wider))=(left,right) {
                    entries+=1;assert_eq!(p.approved_budget_usd,wider.approved_budget_usd);
                    assert!(p.approved_budget_usd<=a.equity*c.risk_pct/100.0);
                    assert!(p.approved_budget_usd<=a.equity*c.portfolio_risk_pct/100.0-35.5);
                    assert!(p.risk_usd<p.approved_budget_usd);
                    assert!(p.volume<=0.01 && wider.volume<=0.02);
                    assert_eq!(p.risk_usd,(p.entry_reference-p.sl)*p.side.sign()*XAU_CONTRACT*p.volume);
                }
            }
        }
        assert!(entries>0,"production sizing must exercise the spare-budget case");
    }
    #[test] fn runtime_roundtrip_and_chunking_preserve_intents() {
        let mut a=Runtime::default();let acc=account();for i in 0..111{a.on_tick(&cfg(),&q(i),view(&acc,&[]));}
        let exact=crate::recorded_broker::exact::encode(&a).unwrap();
        let serialized=serde_json::to_string(&exact).unwrap();
        let exact=serde_json::from_str(&serialized).unwrap();
        let mut b:Runtime=crate::recorded_broker::exact::decode(&exact).unwrap();
        for i in 111..500{assert_eq!(a.on_tick(&cfg(),&q(i),view(&acc,&[])),b.on_tick(&cfg(),&q(i),view(&acc,&[])));}
        assert_eq!(a,b);
    }
    #[test] fn adding_future_suffix_cannot_rewrite_prefix() {
        let mut a=Runtime::default();let acc=account();let prefix:Vec<_>=(0..300).map(|i|a.on_tick(&cfg(),&q(i),view(&acc,&[]))).collect();
        let mut b=Runtime::default();let extended:Vec<_>=(0..600).map(|i|b.on_tick(&cfg(),&q(i),view(&acc,&[]))).collect();assert_eq!(prefix,extended[..300]);
    }
    #[test] fn invalid_config_and_unbounded_unknown_risk_cannot_open() {
        let mut a=Runtime::default();let acc=account();let mut c=cfg();c.risk_pct=f64::NAN;
        assert!(a.on_tick(&c,&q(0),view(&acc,&[])).is_empty());
        for i in 0..300{let mut v=view(&acc,&[]);v.other_risk_usd=f64::INFINITY;assert!(!a.on_tick(&cfg(),&q(i),v).iter().any(|x|matches!(x,Intent::Open(_))));}
    }
    #[test] fn acknowledgement_recovery_is_idempotent_and_learning_waits_for_complete_close() {
        let mut s=Runtime::default();let plan=EntryPlan{decision_id:1,ts:q(0).ts,side:Side::Buy,expert:0,
            volume:0.02,entry_reference:2400.0,sl:2390.0,tp:2420.0,approved_budget_usd:25.0,
            risk_usd:20.0,score:0.8,atr:5.0,context_key:None};
        s.on_open_result(&plan,ExecutionOutcome::Uncertain,None);
        s.on_open_result(&plan,ExecutionOutcome::Uncertain,None);
        s.on_open_result(&plan,ExecutionOutcome::Confirmed,Some(42));
        s.on_open_result(&plan,ExecutionOutcome::Confirmed,Some(42));
        assert_eq!(s.diagnostics.uncertain,1);assert_eq!(s.diagnostics.opened,1);assert!(s.uncertain.is_empty());
        let mut c=ClosedTrade{ticket:42,side:Side::Buy,volume:0.01,open_price:2400.0,close_price:2410.0,
            open_ts:plan.ts,close_ts:plan.ts+60_000,profit:10.0,commission:-1.0,swap:0.0,
            reason:CloseReason::Partial,basket:Some(1),profit_basis:None,cost_receipt:None};
        s.on_closed(&c,9.0);assert_eq!(s.expert_samples[0],0);
        c.close_ts+=60_000;s.on_closed(&c,9.0);assert_eq!(s.expert_samples[0],1);
        assert_eq!(s.expert_mean_r[0],0.9/21.0);
    }
    #[test] fn daily_loss_guard_uses_start_equity_not_intraday_peak() {
        let mut s=Runtime::default();let mut a=account();let mut c=cfg();c.daily_loss_pct=10.0;c.daily_profit_lock_pct=0.0;
        a.equity=200.0;s.on_tick(&c,&q(0),view(&a,&[]));
        a.equity=250.0;s.on_tick(&c,&q(1),view(&a,&[]));
        a.equity=230.0;s.on_tick(&c,&q(2),view(&a,&[]));assert!(!s.day_locked);
        a.equity=181.0;s.on_tick(&c,&q(3),view(&a,&[]));assert!(!s.day_locked);
        a.equity=179.0;s.on_tick(&c,&q(4),view(&a,&[]));assert!(s.day_locked);
        a.equity=220.0;s.on_tick(&c,&q(5),view(&a,&[]));assert!(s.day_locked);
    }
    #[test] fn activation_after_account_loss_cannot_restart_the_daily_budget() {
        let mut s=Runtime::default();let c=cfg();let day=q(0).ts.div_euclid(86_400_000);
        // The other mode lost 7%, then recovered before T100 activation.
        s.import_account_day(day,600.0,600.0,558.0,590.0,&c).unwrap();
        assert!(s.day_locked);assert_eq!(s.day_start,600.0);
        let mut a=account();a.equity=590.0;
        assert!(s.on_tick(&c,&q(0),view(&a,&[])).is_empty());
        assert_eq!(s.diagnostics.last_reason,"daily_risk_stop");
        s.import_account_day(day,600.0,610.0,590.0,610.0,&c).unwrap();
        assert!(s.day_locked,"same-day return must preserve a prior lock");
        assert_eq!(s.day_peak,610.0);
        s.import_account_day(day+1,610.0,610.0,610.0,610.0,&c).unwrap();
        assert!(!s.day_locked);assert_eq!(s.day_start,610.0);
    }
    #[test] fn imported_profit_floor_respects_ordering_and_current_equity() {
        let mut c=cfg();c.daily_loss_pct=30.0;
        let mut s=Runtime::default();
        // 200 -> 180 -> 250 -> 245: unordered minimum must not invent a
        // later giveback through the armed 220 floor.
        s.import_account_day(1,200.0,250.0,180.0,245.0,&c).unwrap();
        assert!(!s.day_locked);
        s.import_account_day(1,200.0,250.0,180.0,219.0,&c).unwrap();
        assert!(s.day_locked);
        s.import_account_day(1,200.0,250.0,180.0,240.0,&c).unwrap();
        assert!(s.day_locked);
    }
    #[test] fn invalid_or_older_account_anchors_leave_the_runtime_exact() {
        let mut s=Runtime::default();let c=cfg();
        s.import_account_day(2,600.0,620.0,590.0,610.0,&c).unwrap();
        let before=s.clone();
        for (day,start,peak,min,current) in [
            (1,600.0,620.0,590.0,610.0),(2,f64::NAN,620.0,590.0,610.0),
            (2,600.0,605.0,590.0,610.0),(2,600.0,620.0,601.0,610.0),
        ] {
            assert!(s.import_account_day(day,start,peak,min,current,&c).is_err());
            assert_eq!(s,before);
        }
    }
    #[test] fn minimum_lot_is_never_forced_above_risk_budget() {
        let mut s=Runtime::default();let mut a=account();a.equity=1.0;a.balance=1.0;a.free_margin=1.0;
        for i in 0..1000{assert!(!s.on_tick(&cfg(),&q(i),view(&a,&[])).iter().any(|x|matches!(x,Intent::Open(_))));}
        assert!(s.diagnostics.blocked_risk>0);
    }
    #[test] fn complete_broker_candles_match_full_ticks_despite_sparse_quote_delivery() {
        let mut raw=Runtime::default();let mut native=Runtime::default();let a=account();let config=cfg();
        let mut current:Option<super::super::Bar>=None;let mut delivered=Vec::new();let mut entries=0;
        for i in 0..900 {
            let base=q(i);let tick=Quote {ts:q(0).ts+i*20_000,bid:base.bid,ask:base.ask};
            let bucket=tick.ts.div_euclid(60_000)*60_000;
            let boundary=current.is_none_or(|b|b.ts!=bucket);
            if boundary {
                if let Some(bar)=current.take(){delivered.push(bar);}
                current=Some(super::super::Bar {ts:bucket,open:tick.bid,high:tick.bid,low:tick.bid,close:tick.bid,max_spread:99.0,observations:99});
            } else {
                let b=current.as_mut().unwrap();b.high=b.high.max(tick.bid);b.low=b.low.min(tick.bid);b.close=tick.bid;
            }
            let expected=raw.on_tick(&config,&tick,view(&a,&[]));
            if boundary {
                let mut broker=view(&a,&[]);broker.completed_bars=Some(&delivered);
                let actual=native.on_tick(&config,&tick,broker);
                entries+=actual.iter().filter(|x|matches!(x,Intent::Open(_))).count();
                assert_eq!(actual,expected,"first quote after minute boundary {i}");
                assert_eq!(raw.market.features(),native.market.features());
            } else {assert!(expected.is_empty());}
        }
        assert!(entries>0);
        assert_eq!(raw.diagnostics.opened,native.diagnostics.opened);
    }
    #[test] fn empty_native_bar_feed_does_not_fall_back_to_sampled_ticks() {
        let mut s=Runtime::default();let a=account();
        for i in 0..500{let mut v=view(&a,&[]);v.completed_bars=Some(&[]);assert!(s.on_tick(&cfg(),&q(i),v).is_empty());}
        assert_eq!(s.diagnostics.closed_bars,0);
    }
}
