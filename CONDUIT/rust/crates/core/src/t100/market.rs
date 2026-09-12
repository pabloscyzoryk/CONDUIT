use std::collections::VecDeque;
use serde::{Deserialize, Serialize};
use crate::types::{Quote, Ts};

const MINUTE: i64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub ts: Ts, pub open:f64, pub high:f64, pub low:f64, pub close:f64,
    pub max_spread:f64, pub observations:u64,
}
impl Bar {
    fn new(q:&Quote)->Self { Self {ts:q.ts.div_euclid(MINUTE)*MINUTE,
        open:q.bid,high:q.bid,low:q.bid,close:q.bid,max_spread:0.0,observations:0} }
    fn add(&mut self,q:&Quote) {
        self.high=self.high.max(q.bid); self.low=self.low.min(q.bid); self.close=q.bid;
    }
    fn merge(&mut self,b:Bar) {
        self.high=self.high.max(b.high); self.low=self.low.min(b.low); self.close=b.close;
        self.max_spread=self.max_spread.max(b.max_spread); self.observations+=b.observations;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Aggregate { current:Option<Bar>, count:usize, last_ts:Option<Ts> }
impl Aggregate {
    fn push(&mut self,b:Bar,minutes:i64)->Option<Bar> {
        let bucket=b.ts.div_euclid(MINUTE*minutes)*MINUTE*minutes;
        if self.current.is_none_or(|v|v.ts!=bucket) || self.last_ts.is_some_and(|t|b.ts-t!=MINUTE) {
            self.current=Some(Bar {ts:bucket,..b}); self.count=0;
        } else { self.current.as_mut().unwrap().merge(b); }
        self.count+=1; self.last_ts=Some(b.ts);
        if b.ts+MINUTE==bucket+MINUTE*minutes {
            let done=self.current.take();
            // A partial first bucket or any missing minute is not a full bar.
            if self.count==minutes as usize { return done; }
        }
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MarketState {
    current:Option<Bar>, last_ts:Option<Ts>, m1:VecDeque<Bar>, m5:VecDeque<Bar>, m15:VecDeque<Bar>,
    agg5:Aggregate, agg15:Aggregate, pub bar_sequence:u64,
    first_quote_ts:Option<Ts>,
}
fn append(bars:&mut VecDeque<Bar>,b:Bar,limit:usize) { bars.push_back(b); if bars.len()>limit {bars.pop_front();} }

impl MarketState {
    pub fn last_completed_ts(&self)->Option<Ts> {self.m1.back().map(|bar|bar.ts)}
    /// Native suffix request also excludes unusable history during cold warmup.
    pub fn completed_bars_after(&self)->Option<Ts> {
        self.last_completed_ts().or_else(||self.first_quote_ts.map(|t|t.saturating_sub(1)))
    }
    /// Exactly one completed M1 bar per boundary; no synthetic gap candles.
    /// Returns None for duplicate/out-of-order observations or an incomplete bar.
    pub fn observe(&mut self,q:&Quote)->Option<Bar> {
        if !super::valid_quote(q) || self.last_ts.is_some_and(|t|q.ts<t) {return None;}
        self.first_quote_ts.get_or_insert(q.ts);
        let gap=self.last_ts.is_some_and(|t|q.ts-t>120_000);
        self.last_ts=Some(q.ts);
        if gap {
            self.current=Some(Bar::new(q)); self.m1.clear(); self.m5.clear(); self.m15.clear();
            self.agg5=Aggregate::default(); self.agg15=Aggregate::default(); return None;
        }
        let Some(current)=self.current.as_mut() else {self.current=Some(Bar::new(q));return None;};
        if current.ts==q.ts.div_euclid(MINUTE)*MINUTE {current.add(q);return None;}
        let done=*current; self.current=Some(Bar::new(q));
        // A cold start in the middle of a minute cannot reconstruct its OHLC.
        if self.first_quote_ts.is_some_and(|ts|done.ts<ts) {return None;}
        self.accept_completed(done);
        Some(done)
    }
    fn accept_completed(&mut self,done:Bar) {
        if self.m1.back().is_some_and(|b|done.ts-b.ts>MINUTE) {
            self.m1.clear();self.m5.clear();self.m15.clear();self.agg5=Aggregate::default();self.agg15=Aggregate::default();
        }
        append(&mut self.m1,done,256);
        if let Some(b)=self.agg5.push(done,5) {append(&mut self.m5,b,128);}
        if let Some(b)=self.agg15.push(done,15) {append(&mut self.m15,b,96);}
        self.bar_sequence+=1;
    }
    /// Native MT5 input contract: complete BID M1 bars, delivered after close.
    /// Older corrected candles never rewrite a decision already made.
    pub fn observe_complete(&mut self,bars:&[Bar],q:&Quote)->Option<Bar> {
        if !super::valid_quote(q) || self.last_ts.is_some_and(|ts|q.ts<ts) {return None;}
        self.first_quote_ts.get_or_insert(q.ts);self.last_ts=Some(q.ts);
        let mut newest=None;
        for raw in bars {
            if raw.ts.rem_euclid(MINUTE)!=0 || raw.ts<0 || raw.ts>q.ts-MINUTE
                || self.first_quote_ts.is_some_and(|ts|raw.ts<ts)
                || self.m1.back().is_some_and(|b|raw.ts<=b.ts) {continue;}
            if ![raw.open,raw.high,raw.low,raw.close].iter().all(|v|v.is_finite()&&*v>0.0)
                || raw.high<raw.open.max(raw.close) || raw.low>raw.open.min(raw.close) || raw.high<raw.low {continue;}
            let bar=Bar{max_spread:0.0,observations:0,..*raw};
            self.accept_completed(bar);newest=Some(bar);
        }
        newest
    }
    pub fn features(&self)->Option<Features> {
        if self.m1.len()<40 || self.m5.len()<12 || self.m15.len()<4 {return None;}
        let last=*self.m1.back()?;
        let atr=average_true_range(&self.m1,20).max(1e-8);
        let slow_atr=average_true_range(&self.m5,12).max(atr);
        let fast=ema(&self.m1,8); let slow=ema(&self.m1,21);
        let tf5=(ema(&self.m5,5)-ema(&self.m5,12))/slow_atr;
        let tf15=(self.m15.back()?.close-self.m15[self.m15.len()-4].close)/(slow_atr*2.0);
        let trend=(0.40*(fast-slow)/atr+0.40*tf5+0.20*tf15).clamp(-2.0,2.0);
        let recent:Vec<_>=self.m1.iter().rev().take(20).collect();
        let mean=recent.iter().map(|b|b.close).sum::<f64>()/20.0;
        let variance=recent.iter().map(|b|(b.close-mean).powi(2)).sum::<f64>()/20.0;
        let std=variance.sqrt().max(atr*0.25);
        let prev_high=self.m1.iter().rev().skip(1).take(20).map(|b|b.high).fold(f64::NEG_INFINITY,f64::max);
        let prev_low=self.m1.iter().rev().skip(1).take(20).map(|b|b.low).fold(f64::INFINITY,f64::min);
        let net=(last.close-self.m1[self.m1.len()-21].close).abs();
        let mut path=0.0;
        for i in self.m1.len()-20..self.m1.len() {path+=(self.m1[i].close-self.m1[i-1].close).abs();}
        let range=(last.high-last.low).max(1e-8);
        Some(Features {bar:last,atr,slow_atr,fast,slow,trend,efficiency:(net/path.max(1e-8)).clamp(0.0,1.0),
            zscore:(last.close-mean)/std,prev_high,prev_low,body:(last.close-last.open)/range,
            close_location:(last.close-last.low)/range,
            compression:atr/slow_atr,
            last_move:(last.close-self.m1[self.m1.len()-2].close)/atr,
            range_atr:range/atr})
    }
}

fn ema(bars:&VecDeque<Bar>,period:usize)->f64 {
    let n=bars.len().min(period*4);let k=2.0/(period as f64+1.0);
    let mut v=bars[bars.len()-n].close;
    for b in bars.iter().skip(bars.len()-n+1) {v+=k*(b.close-v);}
    v
}
fn average_true_range(bars:&VecDeque<Bar>,period:usize)->f64 {
    let start=bars.len().saturating_sub(period).max(1); let mut sum=0.0;
    for i in start..bars.len() {let b=bars[i];let prev=bars[i-1].close;
        sum+=(b.high-b.low).max((b.high-prev).abs()).max((b.low-prev).abs());}
    sum/(bars.len()-start).max(1) as f64
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Features {
    pub bar:Bar, pub atr:f64, pub slow_atr:f64, pub fast:f64, pub slow:f64,
    pub trend:f64, pub efficiency:f64, pub zscore:f64, pub prev_high:f64, pub prev_low:f64,
    pub body:f64, pub close_location:f64, pub compression:f64, pub last_move:f64, pub range_atr:f64,
}

#[cfg(test)] mod tests {
    use super::*;
    fn q(t:i64,p:f64)->Quote{Quote{ts:t,bid:p,ask:p+0.2}}
    #[test] fn boundary_tick_is_not_in_previous_bar() {
        let mut m=MarketState::default();m.observe(&q(0,100.0));m.observe(&q(59_999,102.0));
        let b=m.observe(&q(60_000,999.0)).unwrap();assert_eq!(b.high,102.0);assert_eq!(b.close,102.0);
        assert_eq!(m.current.unwrap().close,999.0);
    }
    #[test] fn late_quote_cannot_rewrite_features() {
        let mut m=MarketState::default();m.observe(&q(60_000,100.0));let before=m.clone();
        assert!(m.observe(&q(59_999,999.0)).is_none());assert_eq!(m,before);
    }
    #[test] fn gap_does_not_invent_liquidity_or_warmup() {
        let mut m=MarketState::default();for i in 0..80 {m.observe(&q(i*60_000,100.0+i as f64));}
        assert!(m.features().is_some());m.observe(&q(86_400_000,200.0));assert!(m.features().is_none());
        assert!(m.m1.is_empty());
    }
    #[test] fn serialize_resume_matches_continuous_closed_bars() {
        let mut a=MarketState::default();for i in 0..81 {a.observe(&q(i*60_000,100.0+i as f64%7.0));}
        let mut b:MarketState=serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        for i in 81..150 {let tick=q(i*60_000,100.0+i as f64%9.0);assert_eq!(a.observe(&tick),b.observe(&tick));assert_eq!(a.features(),b.features());}
    }
}
