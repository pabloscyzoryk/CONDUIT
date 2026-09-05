//! Tests call the actual standard runner; synthetic identity is not an MT5 proof.
use crate::{run,RunConfig,ReplayMessage,SimBroker,TickData};
use crate::runner::{FormatCfg,SzczebelCfg};
use conduit_core::{Broker,Settings};
use conduit_core::formaty::PulapyGlobalne;
use conduit_core::settings::*;
use conduit_core::types::*;
use std::{fs,path::PathBuf,sync::atomic::{AtomicU64,Ordering}};
const T:i64=1_800_000_000_000;
static SEQ:AtomicU64=AtomicU64::new(0);
struct DataFile(PathBuf);
impl DataFile {
    fn new()->Self {
        let p=std::env::temp_dir().join(format!("conduit-fresh-continuation-{}-{}.cdtk",std::process::id(),SEQ.fetch_add(1,Ordering::Relaxed)));
        let rows=[(T-3_600_000,4004.0),(T-1000,4004.0),(T,4004.0),(T+1000,4008.0),
            (T+2000,4008.0),(T+6000,4003.0),(T+10_000,4004.0),(T+11_000,4011.0)];
        let mut out=vec![0u8;64];out[..4].copy_from_slice(&0x4B544443u32.to_le_bytes());
        out[8..16].copy_from_slice(&(rows.len()as u64).to_le_bytes());
        for (ts,bid) in rows {out.extend(ts.to_le_bytes());out.extend((bid as f32).to_le_bytes());out.extend(((bid+0.2)as f32).to_le_bytes());}
        fs::write(&p,out).unwrap();Self(p)
    }
    fn open(&self)->TickData{let mut d=TickData::open(&self.0).unwrap();d.set_price_digits(Some(2)).unwrap();d}
}
impl Drop for DataFile{fn drop(&mut self){let _=fs::remove_file(&self.0);}}
fn config()->RunConfig {
    let settings=Settings{auto_limit:false,entry_units:1,lot_fixed:0.03,lot_max:0.0,
        risk_per_basket_pct:0.0,max_portfolio_risk_pct:0.0,swap_enabled:false,
        pending_lifetime:PendingLifetime::Never,pending_drop_on_target:false,
        tp_source:TpSource::PriceOnly,tp_price_only_strict:true,
        basket_realized_broker_only:true,confirmed_exit_retry:true,live_tick_order_strict:true,
        exec_latency_ms:0,session_filter:false,msg_clock_offset_ms:Some(0),
        ..Settings::default()};
    RunConfig{from:T,to:T+12_000,start_balance:1000.0,settings,..Default::default()}
}
fn messages()->Vec<ReplayMessage>{
    [(T,1,None,"BUY GOLD @ 4005/4000\nTP 4010\nTP 4020\nSL 3900"),
        (T+1000,2,Some(1),"BREAK EVEN"),
        (T+10_000,3,None,"BUY GOLD @ 4005/4000\nTP 4010\nTP 4020\nSL 3900")]
        .into_iter().map(|(ts,msg_id,reply_to,text)|ReplayMessage{ts,msg_id,reply_to,text:text.into(),kanal:"Synergy".into(),..Default::default()}).collect()
}
fn audit(r:&crate::RunResult)->serde_json::Value{serde_json::json!({"metrics":r.metrics,"trades":r.trades,
    "baskets":r.baskets_dump,"days":r.daily,"equity":r.equity_curve,"balance":r.balance_curve,"ticks":r.ticks_processed})}

#[test]
fn fresh_static_on_off_actions_money_and_baskets_match_with_and_without_warmup(){
    let f=DataFile::new();let d=f.open();
    for warmup in [0,1] {for routed in [false,true] {
        let mut cfg=config();cfg.rozgrzewka_h=warmup;
        if routed {cfg.formaty=vec![FormatCfg{format:"Synergy".into(),preset:"synthetic".into(),settings:cfg.settings.clone()}];}
        let off=run(&d,&messages(),&cfg);assert!(!off.trades.is_empty(),"fixture must really trade: {}",audit(&off));
        cfg.settings.restore_strategy_continuation=true;
        let on=run(&d,&messages(),&cfg);
        assert!(on.continuation_reconciliation_required.is_none(),"{:?}",on.continuation_reconciliation_required);
        assert!(on.continuation_scope.as_ref().unwrap().starts_with("SYNTHETIC_SIM_NOT_MT5:"));
        assert_eq!(audit(&off),audit(&on),"warmup={warmup}, routed={routed}");
    }}
}

#[test]
fn unsupported_reset_and_chain_are_explicitly_nonrankable_before_any_ticks(){
    let f=DataFile::new();let d=f.open();
    for mode in 0..3 {let mut cfg=config();cfg.settings.restore_strategy_continuation=true;
        match mode {0=>cfg.daily_reset=true,1=>cfg.flat_na_dobie=true,_=>cfg.drabinka=vec![SzczebelCfg{
            prog:0.0,nazwa:"initial".into(),formaty:vec![FormatCfg{format:"Synergy".into(),preset:"leg".into(),settings:cfg.settings.clone()}],pulapy:PulapyGlobalne::default()}]}
        let r=run(&d,&messages(),&cfg);assert!(r.continuation_reconciliation_required.is_some());assert_eq!(r.ticks_processed,0);assert!(r.trades.is_empty());
    }
}

#[test]
fn account_off_shadows_strategy_on_and_retains_legacy_result_serialization(){
    let f=DataFile::new();let d=f.open();let mut cfg=config();let mut leg=cfg.settings.clone();leg.restore_strategy_continuation=true;
    cfg.formaty=vec![FormatCfg{format:"Synergy".into(),preset:"shadowed".into(),settings:leg}];
    let r=run(&d,&messages(),&cfg);assert!(!r.trades.is_empty());
    let j=serde_json::to_value(r).unwrap();assert!(j.get("continuation_scope").is_none());assert!(j.get("continuation_reconciliation_required").is_none());
}

#[test]
fn synthetic_scope_is_explicit_unique_and_refuses_rebinding_or_existing_history(){
    let mut a=SimBroker::new(1000.0,0.0,0.0);let mut b=SimBroker::new(1000.0,0.0,0.0);
    assert!(a.execution_session().is_none());
    let sa=a.bind_synthetic_continuation_scope().unwrap();let sb=b.bind_synthetic_continuation_scope().unwrap();
    assert_ne!(sa.scope,sb.scope);assert!(a.bind_synthetic_continuation_scope().is_err());
    let mut dirty=SimBroker::new(1000.0,0.0,0.0);dirty.q=Quote{ts:T,bid:4004.0,ask:4004.2};
    let ticket=dirty.open_market(conduit_core::broker::OrderReq{side:Side::Buy,volume:0.01,sl:None,tp:None,basket:Some(1),level:0,is_toucher:false,comment:String::new()}).unwrap();
    assert!(dirty.bind_synthetic_continuation_scope().is_err());dirty.close_position(ticket,CloseReason::Manual).unwrap();dirty.drain_closed();
    assert!(dirty.bind_synthetic_continuation_scope().is_err(),"flat after trading is not Fresh");
}

#[test]
fn separate_windows_pipeline_does_not_publish_silent_zero_for_uninitialized_continuation(){
    let f=DataFile::new();let d=f.open();let mut cfg=crate::okna::KonfOkien::default();cfg.from=T;cfg.to=T+12_000;cfg.settings.restore_strategy_continuation=true;
    let result=crate::okna::uruchom(&d,&messages(),&cfg);
    assert!(result.continuation_reconciliation_required.is_some());assert_eq!(result.tickow,0);
}
