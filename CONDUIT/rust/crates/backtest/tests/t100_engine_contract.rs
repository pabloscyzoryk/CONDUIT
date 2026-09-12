//! Synthetic execution qualification: actual shared Engine and SimBroker.
//! No terminal, network, prices from a provider, or profitability selection.
use conduit_backtest::sim::SimBroker;
use conduit_core::{Broker, Engine, IncomingMessage, Quote, Settings, SourceKey};

fn settings(enabled:bool)->Settings {
    let mut c=Settings::default();
    c.t100.enabled=enabled;c.t100.score_threshold=0.35;c.t100.risk_pct=4.0;
    c.t100.portfolio_risk_pct=8.0;c.t100.daily_loss_pct=40.0;
    c.t100.session_start_utc=0;c.t100.session_end_utc=24;c.t100.friday_flat_utc=24;
    c.t100.min_atr=0.05;c.t100.spread_atr_max=1.0;c.t100.daily_profit_lock_pct=0.0;
    c.lot_max=0.01;c.session_filter=false;c.exec_latency_ms=0;
    c.journal_enabled=false;c.ea_enabled=false;c.ai_enabled=false;
    c.streak_pause_n=0;c.max_portfolio_risk_pct=0.0;c.profit_budget_arm_pct=0.0;
    c.swap_enabled=false;c.basket_realized_broker_only=true;
    c
}
fn quotes()->Vec<Quote>{
    (0..360).flat_map(|minute|{
        (0..3).map(move|part|{
            let x=minute as f64;
            let price=2400.0+0.09*x+(x*0.31).sin()*1.2+[-0.30,0.0,0.40][part];
            Quote{ts:1_783_320_000_000+minute*60_000+part as i64*20_000,bid:price,ask:price+0.1}
        })
    }).collect()
}
fn replay(restart:bool,seed:&conduit_core::engine::ReplayBootstrap)->(Engine,SimBroker){
    let mut e=Engine::from_replay_bootstrap(seed).unwrap();let settings=e.cfg.clone();
    let mut b=SimBroker::z_ustawien(600.0,&settings);
    let quotes=quotes();
    for (i,q) in quotes.iter().enumerate(){
        b.on_quote(*q);e.on_tick(&mut b,q);
        if restart && i==quotes.len()/2 {
            let exact=e.export_replay_bootstrap().unwrap();
            let bytes=serde_json::to_vec(&exact).unwrap();
            e=Engine::from_replay_bootstrap(&serde_json::from_slice(&bytes).unwrap()).unwrap();
        }
    }
    (e,b)
}

#[test]
fn autonomous_shared_engine_executes_without_telegram_and_resumes_exactly(){
    // Both processes start from the same source-revision allocator state.
    let mut initial=Engine::new(settings(true),600.0);initial.tryb_auto_ea=true;
    let seed=initial.export_replay_bootstrap().unwrap();
    let (a,ba)=replay(false,&seed);let (b,bb)=replay(true,&seed);
    assert!(a.t100.diagnostics.opened>0,"fixture must exercise autonomous execution: {:?}",a.t100.diagnostics);
    assert!(!ba.history.is_empty(),"fixture must exercise actual SL/TP/strategy closes");
    assert!(a.t100_entry_hold_reason().is_none(),"verified simulator cost basis must not cause spurious hold");
    assert!(ba.history.iter().all(|t|t.volume<=0.01));
    assert_eq!(serde_json::to_value(&ba.history).unwrap(),serde_json::to_value(&bb.history).unwrap());
    assert_eq!(ba.account().equity.to_bits(),bb.account().equity.to_bits());
    assert_eq!(ba.account().balance.to_bits(),bb.account().balance.to_bits());
    assert_eq!(a.t100_checkpoint(),b.t100_checkpoint());
    let sa=a.export_replay_bootstrap().unwrap();let sb=b.export_replay_bootstrap().unwrap();
    let changed:Vec<_>=sa.fields.iter().filter(|(k,v)|sb.fields.get(*k)!=Some(*v)).map(|(k,_)|k).collect();
    assert!(changed.is_empty(),"checkpoint fields differ: {changed:?}");
}

#[test]
fn disabled_t100_preserves_legacy_execution_even_with_nondefault_policy_parameters(){
    let mut cfg=settings(false);let mut other=cfg.clone();other.t100=Default::default();
    cfg.entry_units=1;other.entry_units=1;
    let mut a=Engine::new(cfg.clone(),600.0);let mut b=Engine::new(other.clone(),600.0);
    let mut ba=SimBroker::z_ustawien(600.0,&cfg);let mut bb=SimBroker::z_ustawien(600.0,&other);
    for (i,q) in quotes().iter().enumerate(){
        ba.on_quote(*q);bb.on_quote(*q);
        if i==10 {
            let msg=IncomingMessage{ts:q.ts,source:SourceKey::new(1,None),source_name:"Synergy".into(),
                msg_id:1,reply_to:None,edit_of:None,text:format!("BUY GOLD @ {}/{}\nTP {}\nSL {}",q.ask+0.1,q.ask-0.1,q.ask+2.0,q.ask-2.0)};
            a.on_message(&mut ba,&msg);b.on_message(&mut bb,&msg);
        }
        a.on_tick(&mut ba,q);b.on_tick(&mut bb,q);
    }
    assert!(!ba.history.is_empty(),"legacy control must trade");
    assert_eq!(serde_json::to_value(&ba.history).unwrap(),serde_json::to_value(&bb.history).unwrap());
    assert_eq!(ba.account().equity.to_bits(),bb.account().equity.to_bits());
    assert_eq!(serde_json::to_value(&a.stats).unwrap(),serde_json::to_value(&b.stats).unwrap());
    assert_eq!(a.t100,Default::default());assert_eq!(b.t100,Default::default());
}

#[test]
fn runner_rejects_t100_in_auto_before_processing_any_market_data(){
    use std::io::Write;
    let path=std::env::temp_dir().join(format!("conduit_t100_wrong_mode_{}_{}.bin",std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(&path).unwrap();
    let mut header=vec![0u8;64];header[..4].copy_from_slice(&0x4B54_4443u32.to_le_bytes());
    file.write_all(&header).unwrap();drop(file);
    let ticks=conduit_backtest::data::TickData::open(&path).unwrap();
    let cfg=conduit_backtest::runner::RunConfig{settings:settings(true),auto_ea:false,..Default::default()};
    let result=conduit_backtest::runner::run(&ticks,&[],&cfg);
    assert_eq!(result.ticks_processed,0);assert!(result.trades.is_empty());
    assert!(result.t100[0].entry_hold.as_deref().unwrap().contains("AUTO-EA"));
    drop(ticks);std::fs::remove_file(path).unwrap();
}
