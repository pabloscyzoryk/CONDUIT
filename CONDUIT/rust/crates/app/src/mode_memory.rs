//! Account-bound strategy namespaces. No network, clock reads or order APIs.
use super::*;
use std::collections::BTreeMap;

const MAX_EVENTS: usize = 8192;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone,serde::Serialize,serde::Deserialize)]
pub(super) enum PassiveEvent {
    Message {message:IncomingMessage,received_utc:i64},
    UnverifiedMessage {message:IncomingMessage,received_utc:i64},
    Gap {reason:String,observed_utc:i64},
}

#[derive(Default,Clone,serde::Serialize,serde::Deserialize)]
#[serde(default)]
pub(super) struct ModeBucket {
    pub silniki:BTreeMap<String,TrwalySilnik>,
    pub koszyki:Vec<Basket>,
    pub initialized:bool,
    pub context_events:Vec<PassiveEvent>,
    pub context_overflow:u64,
}

#[derive(Default,Clone)]
pub(super) struct ModeMemory {
    pub auto:ModeBucket,
    pub auto_ea:Option<ModeBucket>,
    pub active_auto_ea:bool,
    pub allocator:BTreeMap<u32,u32>,
    pub revision:u64,
    pub telegram_generation:Option<u32>,
    pub telegram_unavailable:bool,
    pub source_gap_utc:Option<i64>,
}

impl ModeMemory {
    pub fn active(&self)->&ModeBucket {
        if self.active_auto_ea {self.auto_ea.as_ref().expect("selected EA bucket exists")} else {&self.auto}
    }
    pub fn capture(&mut self,team:&routing::Silniki,diagnosis:&str) {
        for slot in &team.lista {
            let next=self.allocator.entry(slot.slot).or_insert(slot.engine.next_basket_id());
            *next=(*next).max(slot.engine.next_basket_id());
        }
        let previous=if self.active_auto_ea {self.auto_ea.take().unwrap_or_default()} else {std::mem::take(&mut self.auto)};
        let bucket=ModeBucket {silniki:capture_engines(team,diagnosis),koszyki:team.koszyki(),initialized:true,
            context_events:previous.context_events,context_overflow:previous.context_overflow};
        if self.active_auto_ea {self.auto_ea=Some(bucket);} else {self.auto=bucket;}
        self.revision=self.revision.wrapping_add(1);
    }
    pub fn queue(&mut self,event:PassiveEvent) {
        if let PassiveEvent::Gap{observed_utc,..}=&event {
            self.source_gap_utc=Some(self.source_gap_utc.map_or(*observed_utc,|old|old.max(*observed_utc)));
        }
        let event=match event {
            PassiveEvent::Message{message,received_utc} if self.source_gap_utc.is_some_and(|gap|received_utc<=gap)=>
                PassiveEvent::UnverifiedMessage{message,received_utc},
            other=>other,
        };
        let Some(bucket)=self.auto_ea.as_mut() else{return;};
        if !bucket.silniki.values().any(|m|m.t100.is_some()) {return;}
        let size=serde_json::to_vec(&event).map_or(MAX_BYTES+1,|v|v.len());
        let total=serde_json::to_vec(&bucket.context_events).map_or(MAX_BYTES+1,|v|v.len());
        if bucket.context_events.len()>=MAX_EVENTS || total.saturating_add(size)>MAX_BYTES {
            bucket.context_overflow+=bucket.context_events.len() as u64+1;
            bucket.context_events.clear();
            bucket.context_events.push(PassiveEvent::Gap{reason:"passive context queue overflow".into(),observed_utc:0});
        } else {bucket.context_events.push(event);}
        self.revision=self.revision.wrapping_add(1);
    }
}

pub(super) fn capture_engines(team:&routing::Silniki,diagnosis:&str)->BTreeMap<String,TrwalySilnik> {
    team.lista.iter().map(|s| {
        let halted=s.engine.halted.as_deref().and_then(|r| {
            let rest=if diagnosis.is_empty(){r}else if r==diagnosis{""}else{
                r.strip_prefix(diagnosis).and_then(|v|v.strip_prefix(ui::HALT_SEP)).unwrap_or(r)};
            (!rest.is_empty()).then(||rest.to_string())
        });
        (s.format.clone(),TrwalySilnik {t100:s.engine.t100_checkpoint(),
            strategy_realized:Some(crate::strategy_realized_memory::StrategyRealizedMemory::capture(&s.engine)),
            stats:Some(s.engine.stats.clone()),halted,risk_override:s.engine.risk_override,
            closed_today:s.engine.closed_today.clone(),stopped_trading_day:s.engine.stopped_trading_day(),
            profit_budget_anchor:(s.engine.cfg.profit_budget_arm_pct!=0.0).then(||(&s.engine.stats).into()),
            pending_sources:s.engine.export_pending_source_memory(),rearm_reconcile:s.engine.rearm_reconcile_state(),
            entry_sources:s.engine.export_entry_source_memory(),continuation:s.engine.export_strategy_continuation()})
    }).collect()
}

/// Select only after a flat-account proof. Global identity, risk/UI and broker
/// state remain outside this function; no source event moves between accounts.
pub(super) fn select_mode(memory:&mut Trwale,auto_ea:bool) {
    if memory.modes.active_auto_ea==auto_ea {return;}
    if auto_ea && memory.modes.auto_ea.is_none(){memory.modes.auto_ea=Some(ModeBucket::default());}
    memory.modes.active_auto_ea=auto_ea;
    let selected=memory.modes.active().clone();
    memory.mode_fresh=!selected.initialized;
    memory.mode_baskets_known=true;
    memory.silniki=selected.silniki;memory.koszyki=selected.koszyki;
    memory.next_basket_id=memory.modes.allocator.values().copied().max().unwrap_or(1);
    memory.modes.revision=memory.modes.revision.wrapping_add(1);
}

pub(super) fn restore_allocator(team:&mut routing::Silniki,memory:&ModeMemory) {
    for slot in &mut team.lista {
        if let Some(floor)=memory.allocator.get(&slot.slot) {
            if let Err(reason)=slot.engine.ensure_next_basket_id_floor(*floor) {
                slot.engine.hold_strategy_continuation(ContinuationReviewScope::Account,reason);
            }
        }
    }
}

/// Same production parser/options as Engine::on_message, but no Engine call,
/// broker access, execution or synthetic message-statistic increment.
pub(super) fn apply_context(engine:&mut Engine,event:&PassiveEvent) {
    match event {
        PassiveEvent::Gap{..}=>engine.t100.context.quarantine_existing(),
        PassiveEvent::Message{message,..}|PassiveEvent::UnverifiedMessage{message,..}=> {
            let cfg=&engine.cfg;
            let mut signals=conduit_core::parser::parse_z_opcjami(&message.text,conduit_core::parser::OpcjeParsera {
                geometryczny:cfg.parser_geometryczny,min_pewnosc:cfg.parser_min_pewnosc,
                rf_wymaga_wykonania:cfg.rf_wymaga_wykonania,partials_jako_komenda:cfg.partials_wykonuj,
                luz_interpunkcyjny:cfg.parser_luz_interpunkcyjny,recap_guard:cfg.recap_guard});
            if cfg.profit_update_telemetry_only {conduit_core::parser::suppress_at_tp_hits(&mut signals,&message.text);}
            engine.t100.observe_context(message,&signals);
            if matches!(event,PassiveEvent::UnverifiedMessage{..}) {engine.t100.context.quarantine_source_version(message);}
        }
    }
}

pub(super) fn restore_context(team:&mut routing::Silniki,memory:&mut Trwale) {
    if !memory.modes.active_auto_ea || !team.lista.iter().any(|s|s.engine.cfg.t100.enabled) {return;}
    let events=memory.modes.auto_ea.as_mut().map(|b|std::mem::take(&mut b.context_events)).unwrap_or_default();
    for slot in team.lista.iter_mut().filter(|s|s.engine.cfg.t100.enabled) {
        for event in &events {apply_context(&mut slot.engine,event);}
    }
    if !events.is_empty(){memory.modes.revision=memory.modes.revision.wrapping_add(1);}
}

pub(super) fn context_gap(memory:&mut Trwale,team:&mut routing::Silniki,reason:&str,utc:i64) {
    memory.modes.source_gap_utc=Some(memory.modes.source_gap_utc.map_or(utc,|old|old.max(utc)));
    let event=PassiveEvent::Gap{reason:reason.into(),observed_utc:utc};
    if memory.modes.active_auto_ea {
        for slot in team.lista.iter_mut().filter(|s|s.engine.cfg.t100.enabled) {apply_context(&mut slot.engine,&event);}
        memory.modes.revision=memory.modes.revision.wrapping_add(1);
    } else {memory.modes.queue(event);}
}

/// Import only at activation/recovery. The shared policy evaluates subsequent
/// samples itself, exactly as in backtests. Strategy counters remain separate.
pub(super) fn preserve_account_risk(team:&mut routing::Silniki,stats:&ui::Stats,day:i64,equity:f64) {
    for slot in team.lista.iter_mut().filter(|s|s.engine.cfg.t100.enabled) {
        import_account_risk(&mut slot.engine,stats,day,equity);
    }
}

pub(super) fn import_account_risk(engine:&mut Engine,stats:&ui::Stats,day:i64,equity:f64) {
    if stats.day_key!=day || !stats.day_start_equity.is_finite() || stats.day_start_equity<=0.0 {return;}
    let start=stats.day_start_equity;
    let mut minimum=equity.min(start);
    if stats.real_drawdown_day.day==Some(day) {
        if let Some(min)=stats.real_drawdown_day.min_equity.filter(|v|v.is_finite()) {minimum=minimum.min(min);}
    }
    let peak=stats.peak_equity_today.max(start).max(equity);
    if let Err(reason)=engine.t100.import_account_day(day,start,peak,minimum,equity,&engine.cfg.t100) {
        engine.hold_strategy_continuation(ContinuationReviewScope::Account,reason);
    }
}

pub(super) fn preserve_account_halt(team:&mut routing::Silniki,reason:&str) {
    if reason.is_empty(){return;}
    for slot in &mut team.lista {
        if slot.engine.halted.as_deref().is_none_or(|old|!old.contains(reason)) {
            slot.engine.halted=Some(match slot.engine.halted.take(){
                Some(old)=>format!("{old}{}{reason}",ui::HALT_SEP),None=>reason.into()});
        }
    }
}
