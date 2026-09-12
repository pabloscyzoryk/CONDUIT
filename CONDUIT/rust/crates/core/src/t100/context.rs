use std::collections::{BTreeMap,BTreeSet};
use serde::{Deserialize,Serialize};
use crate::{IncomingMessage,Signal,EntrySignal};
use crate::types::{Side,Ts};
use super::Config;

#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
pub struct SignalContext {
    pub key:String, pub available_ts:Ts, pub side:Side, pub lo:f64, pub hi:f64,
    pub sl:Option<f64>, pub tps:Vec<f64>, pub pending:bool, pub cancelled:bool,
    pub uses:u64, pub edited:bool,
}

#[derive(Debug,Clone,Default,PartialEq,Serialize,Deserialize)]
pub struct ContextBook {
    pub contexts:BTreeMap<String,SignalContext>,
    aliases:BTreeMap<String,String>,
    withdrawals:BTreeMap<String,Ts>,
    #[serde(default,skip_serializing_if="BTreeSet::is_empty")]
    quarantined:BTreeSet<String>,
}
impl ContextBook {
    fn root_key(&self,key:&str)->Option<String> {
        let mut current=key;let mut visited=BTreeSet::new();
        for _ in 0..64 {
            if !visited.insert(current) {return None;}
            match self.aliases.get(current) {Some(next)=>current=next,None=>return Some(current.to_owned())}
        }
        None
    }
    pub fn observe(&mut self,m:&IncomingMessage,signals:&[Signal]) {
        // Source attribution is a routing fact. Never scrape a channel name from text.
        if !m.source_name.to_lowercase().contains("synergy") {return;}
        let prefix=m.source.as_string();let id=m.edit_of.unwrap_or(m.msg_id);
        let key=format!("{prefix}:{id}");
        let target=m.reply_to.map(|id|format!("{prefix}:{id}"));
        let target=target.and_then(|k|self.root_key(&k));
        if let Some(target)=&target {self.aliases.insert(key.clone(),target.clone());}
        for signal in signals {
            match signal {
                Signal::Entry(entry) if valid_entry(entry) => {
                    // An edit is usable when first received, never at original publish time.
                    if self.contexts.get(&key).is_some_and(|c|m.ts<c.available_ts || (c.edited && m.edit_of.is_none())) {continue;}
                    let uses=self.contexts.get(&key).map(|c|c.uses).unwrap_or(0);
                    self.contexts.insert(key.clone(),SignalContext { key:key.clone(),available_ts:m.ts,
                        side:entry.side,lo:entry.lo.min(entry.hi),hi:entry.lo.max(entry.hi),sl:entry.sl,
                        tps:entry.tps.clone(),pending:entry.is_limit||entry.is_stop,
                        cancelled:self.withdrawals.contains_key(&key),uses,edited:m.edit_of.is_some() });
                    // Only a complete, accepted version revalidates this setup.
                    // Partial SL/TP corrections cannot prove source continuity.
                    self.quarantined.remove(&key);
                },
                Signal::Cancel => {if let Some(target)=&target {
                    self.withdrawals.insert(target.clone(),m.ts);
                    if let Some(c)=self.contexts.get_mut(target) {c.cancelled=true;}
                }},
                Signal::SetSl{value} if value.is_finite() && *value>0.0 => {if let Some(c)=target.as_ref().and_then(|k|self.contexts.get_mut(k)) {
                    if m.ts>=c.available_ts {c.sl=Some(*value);c.available_ts=m.ts;}
                }},
                Signal::TpCorrection{index,value} if value.is_finite() && *value>0.0 => {if let Some(c)=target.as_ref().and_then(|k|self.contexts.get_mut(k)) {
                    if *index>0 && *index<=c.tps.len() && m.ts>=c.available_ts {c.tps[*index-1]=*value;c.available_ts=m.ts;}
                }},
                // TP/RF/SL announcements are observations, not T-100 execution commands.
                _=>{},
            }
        }
        // A CANCEL may arrive before the intervening reply. Resolve that
        // previously unknown edge when it is actually observed, without
        // applying the cancellation to an unrelated last-seen setup.
        let resolved:Vec<_>=self.withdrawals.iter().filter_map(|(key,ts)|self.root_key(key).map(|root|(root,*ts))).collect();
        for (key,ts) in resolved {
            self.withdrawals.entry(key.clone()).or_insert(ts);
            if let Some(c)=self.contexts.get_mut(&key){c.cancelled=true;}
        }
    }
    pub fn best<'a>(&'a self,cfg:&Config,ts:Ts,price:f64,atr:f64,side:Side)->Option<(&'a SignalContext,f64)> {
        self.contexts.values().filter_map(|c|{
            if c.cancelled || self.quarantined.contains(&c.key) || c.available_ts>ts {return None;}
            let age=(ts-c.available_ts) as f64/60_000.0;
            if !c.pending && age>cfg.market_context_max_age_min {return None;}
            // Source geometry is advisory evidence, not a permanent directional
            // vote after its stop or final target is already behind the price.
            // This never cancels the source setup: a pending remains remembered
            // and can become relevant again on a later return into its geometry.
            let direction=c.side.sign();
            if c.sl.is_some_and(|sl|(price-sl)*direction<=0.0) {return None;}
            let anchor=if c.side==Side::Buy {c.hi} else {c.lo};
            let remaining=c.tps.iter().copied().filter(|tp|(*tp-anchor)*direction>0.0)
                .map(|tp|(tp-price)*direction).max_by(f64::total_cmp);
            if remaining.is_some_and(|distance|distance<=0.0) {return None;}
            let runway=remaining.map(|distance|(distance/(2.0*atr)).clamp(0.0,1.0)).unwrap_or(1.0);
            let distance=if price<c.lo {c.lo-price} else if price>c.hi {price-c.hi} else {0.0};
            if distance>8.0*atr {return None;}
            let fresh=2.0_f64.powf(-age/cfg.signal_half_life_min).max(if c.pending{0.25}else{0.0});
            let proximity=(-distance/(2.0*atr)).exp();
            let align=if c.side==side {1.0} else {-1.0};
            Some((c,align*fresh*proximity*runway))
        }).max_by(|(a,x),(b,y)|x.abs().total_cmp(&y.abs()).then(a.available_ts.cmp(&b.available_ts)).then(a.key.cmp(&b.key)))
    }
    pub fn used(&mut self,key:&str) {if let Some(c)=self.contexts.get_mut(key){c.uses+=1;}}
    /// A known observation gap removes influence, not the pending or its history.
    pub fn quarantine_existing(&mut self) {
        self.quarantined.extend(self.contexts.values().filter(|c|!c.cancelled).map(|c|c.key.clone()));
    }
    /// A queued version received before a known observation gap cannot renew
    /// continuity when it is dispatched later. Unrelated new keys stay usable.
    pub fn quarantine_source_version(&mut self,message:&IncomingMessage) {
        let key=format!("{}:{}",message.source.as_string(),message.edit_of.unwrap_or(message.msg_id));
        if self.contexts.get(&key).is_some_and(|c|!c.cancelled) {self.quarantined.insert(key);}
    }
    pub fn quarantined_count(&self)->usize {
        self.contexts.values().filter(|c|!c.cancelled && self.quarantined.contains(&c.key)).count()
    }
    pub fn active_count(&self)->usize {self.contexts.values().filter(|c|!c.cancelled).count()}
    pub fn used_count(&self)->usize {self.contexts.values().filter(|c|c.uses>0).count()}
}
fn valid_entry(e:&EntrySignal)->bool {
    e.lo.is_finite() && e.hi.is_finite() && e.lo>0.0 && e.hi>0.0
        && e.sl.is_none_or(|v|v.is_finite()&&v>0.0) && e.tps.iter().all(|v|v.is_finite()&&*v>0.0)
}

#[cfg(test)] mod tests {
    use super::*;use crate::SourceKey;
    fn msg(id:i64,reply:Option<i64>,edit:bool,ts:i64)->IncomingMessage {IncomingMessage {ts,source:SourceKey::new(1,None),source_name:"Synergy".into(),msg_id:id,reply_to:reply,edit_of:edit.then_some(id),text:String::new()}}
    fn entry()->Signal {Signal::Entry(EntrySignal{side:Side::Buy,is_limit:true,is_stop:false,lo:100.0,hi:101.0,sl:Some(95.0),tps:vec![110.0],tp_open:false,warstwy_offset:None,tag_high_risk:false,tag_may_not_be_around:false,tag_first_entry:false})}
    #[test] fn edit_first_available_is_causal_and_pending_survives_weeks() {
        let mut book=ContextBook::default();book.observe(&msg(5,None,true,120_000),&[entry()]);
        let cfg=Config::default();assert!(book.best(&cfg,119_999,100.5,2.0,Side::Buy).is_none());
        assert!(book.best(&cfg,30*86_400_000,100.5,2.0,Side::Buy).is_some());
        book.observe(&msg(6,Some(5),false,31*86_400_000),&[Signal::Cancel]);
        assert!(book.best(&cfg,32*86_400_000,100.5,2.0,Side::Buy).is_none());
    }
    #[test] fn quarantine_preserves_pending_and_revalidates_only_complete_own_version() {
        let mut book=ContextBook::default();let cfg=Config::default();
        book.observe(&msg(5,None,true,1),&[entry()]);let original=book.contexts.clone();
        book.quarantine_existing();assert_eq!(book.contexts,original);assert_eq!(book.quarantined_count(),1);
        assert!(book.best(&cfg,100,100.5,2.0,Side::Buy).is_none());
        book.observe(&msg(6,Some(5),false,2),&[Signal::SetSl{value:96.0}]);
        assert_eq!(book.quarantined_count(),1);
        book.observe(&msg(7,Some(5),false,3),&[Signal::TpCorrection{index:1,value:112.0}]);
        assert_eq!(book.quarantined_count(),1);
        book.observe(&msg(5,None,false,4),&[entry()]); // rejected late original is not new evidence
        assert_eq!(book.quarantined_count(),1);
        book.observe(&msg(8,None,false,5),&[entry()]);
        assert_eq!(book.best(&cfg,100,100.5,2.0,Side::Buy).unwrap().0.key,msg(8,None,false,5).source.as_string()+":8");
        assert_eq!(book.quarantined_count(),1);
        let saved=serde_json::to_vec(&book).unwrap();book=serde_json::from_slice(&saved).unwrap();
        assert_eq!(book.quarantined_count(),1);
        book.observe(&msg(5,None,true,6),&[entry()]);assert_eq!(book.quarantined_count(),0);
        assert!(book.contexts.values().all(|c|c.pending && !c.cancelled));
        book.quarantine_existing();book.observe(&msg(9,Some(5),false,7),&[Signal::Cancel]);
        assert_eq!(book.quarantined_count(),1);assert_eq!(book.active_count(),1);
    }
    #[test] fn normal_context_encoding_remains_legacy_compatible() {
        let mut book=ContextBook::default();book.observe(&msg(5,None,false,1),&[entry()]);
        let value=serde_json::to_value(&book).unwrap();assert!(value.get("quarantined").is_none());
        let restored:ContextBook=serde_json::from_value(value.clone()).unwrap();assert_eq!(restored,book);
        assert_eq!(serde_json::to_value(restored).unwrap(),value);
    }
    #[test] fn cancel_before_entry_tombstone_and_reply_chain() {
        let mut book=ContextBook::default();book.observe(&msg(6,Some(5),false,100),&[Signal::Info]);
        book.observe(&msg(7,Some(6),false,101),&[Signal::Cancel]);book.observe(&msg(5,None,true,102),&[entry()]);
        assert_eq!(book.active_count(),0);
    }
    #[test] fn unrelated_source_and_late_original_do_not_change_context() {
        let mut book=ContextBook::default();let mut m=msg(5,None,false,1);m.source_name="Other".into();book.observe(&m,&[entry()]);assert_eq!(book.active_count(),0);
        book.observe(&msg(5,None,true,10),&[entry()]);let before=book.contexts.clone();
        book.observe(&msg(5,None,false,11),&[entry()]);assert_eq!(book.contexts,before);
    }
    #[test] fn cancellation_waits_for_late_reply_edge_without_guessing_target() {
        let mut book=ContextBook::default();book.observe(&msg(5,None,false,1),&[entry()]);
        book.observe(&msg(7,Some(6),false,10),&[Signal::Cancel]);assert_eq!(book.active_count(),1);
        book.observe(&msg(6,Some(5),false,11),&[Signal::Info]);assert_eq!(book.active_count(),0);
    }
    #[test] fn pending_geometry_changes_influence_without_cancelling_its_lifetime() {
        let mut book=ContextBook::default();book.observe(&msg(5,None,false,0),&[entry()]);
        let cfg=Config::default();
        assert!(book.best(&cfg,100,94.0,2.0,Side::Buy).is_none());
        assert!(book.best(&cfg,100,111.0,2.0,Side::Buy).is_none());
        assert_eq!(book.active_count(),1);
        let near=book.best(&cfg,100,109.0,2.0,Side::Buy).unwrap().1;
        let at_zone=book.best(&cfg,100,100.5,2.0,Side::Buy).unwrap().1;
        assert!(near>0.0 && near<at_zone);
        assert!(book.best(&cfg,30*86_400_000,100.5,2.0,Side::Buy).is_some());
    }
    #[test] fn sell_geometry_is_symmetric_and_a_later_correction_is_not_backdated() {
        let mut setup=match entry(){Signal::Entry(e)=>e,_=>unreachable!()};
        setup.side=Side::Sell;setup.sl=Some(106.0);setup.tps=vec![90.0];
        let mut book=ContextBook::default();book.observe(&msg(5,None,false,0),&[Signal::Entry(setup)]);
        let cfg=Config::default();
        assert!(book.best(&cfg,100,107.0,2.0,Side::Sell).is_none());
        assert!(book.best(&cfg,100,89.0,2.0,Side::Sell).is_none());
        assert!(book.best(&cfg,100,100.5,2.0,Side::Sell).is_some());
        book.observe(&msg(6,Some(5),false,1000),&[Signal::SetSl{value:99.0}]);
        assert!(book.best(&cfg,1000,100.5,2.0,Side::Sell).is_none());
        assert!(book.best(&cfg,1000,98.5,2.0,Side::Sell).is_some());
    }
}
