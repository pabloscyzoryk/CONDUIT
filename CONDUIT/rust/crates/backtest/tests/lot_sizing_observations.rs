//! Neutral sizing telemetry survives fresh-day account resets without becoming
//! a rejection count or disappearing from the final backtest result.
use conduit_backtest::{ReplayMessage,TickData,RunConfig,run};
use conduit_core::settings::Settings;
use conduit_core::lot_growth::LotGrowthMode;

#[test]
fn neutral_sizing_observations_accumulate_across_independent_days() {
    let day=86_400_000i64;
    let start=1_775_000_000_000i64/day*day;
    let path=std::env::temp_dir().join(format!("conduit_sizing_observation_{}.bin",std::process::id()));
    let n=3*24*60usize;
    let mut bytes=vec![0u8;64];bytes[..4].copy_from_slice(&0x4B54_4443u32.to_le_bytes());
    bytes[8..16].copy_from_slice(&(n as u64).to_le_bytes());
    for i in 0..n {
        bytes.extend_from_slice(&(start+i as i64*60_000).to_le_bytes());
        let minute=i%(24*60);
        let bid=if minute>=12*60 {4021.5f32}else if minute>=10*60 {3999.5f32}else{4000f32};
        bytes.extend_from_slice(&bid.to_le_bytes());
        bytes.extend_from_slice(&(bid+0.2).to_le_bytes());
    }
    std::fs::write(&path,bytes).unwrap();
    let tape=TickData::open(&path).unwrap();
    let messages:Vec<_>=(0..3).map(|d|ReplayMessage {kanal:String::new(),
        ts:start+d*day+9*3_600_000,telegram_published_ts:None,msg_id:100+d,
        reply_to:None,edit_of:None,text:"BUY GOLD @ 4000.50/3999.50\nTP 4020\nSL 3990".into()}).collect();
    let cfg=Settings {session_filter:false,exec_latency_ms:0,entry_units:1,
        msg_clock_offset_ms:Some(0),server_tz_offset_ms:0,
        auto_limit:false,skip_if_sl_breached:false,journal_enabled:false,max_open_baskets:0,
        max_open_positions:0,streak_pause_n:0,oae_timeout_min:0.,swap_enabled:false,
        regime_filter:conduit_core::settings::RegimeFilter::Off,
        lot_growth_mode:LotGrowthMode::Power,lot_growth_reference_lot:0.04,
        lot_growth_spread_stress_strength:1.,..Settings::default()};
    let run_days=|days|run(&tape,&messages,&RunConfig {from:start,to:start+days*day,
        start_balance:1000.,settings:cfg.clone(),daily_reset:true,flat_na_dobie:true,
        ..RunConfig::default()});
    let one=run_days(1);let three=run_days(3);
    let key="LotContext::Observed::lot_growth_spread_stress_strength";
    let n1=*one.metrics.lot_sizing_diagnostics.get(key).expect("axis must be observed");
    assert!(n1>0 && one.metrics.trades>0,"observations={n1}, trades={}, rejections={:?}",one.metrics.trades,one.metrics.odrzuty);
    assert_eq!(three.metrics.lot_sizing_diagnostics[key],3*n1);
    assert!(!three.metrics.odrzuty.contains_key(key));
    let serialized=serde_json::to_value(&three.metrics).unwrap();
    assert_eq!(serialized["lot_sizing_diagnostics"][key],3*n1);
    let empty=serde_json::to_value(conduit_backtest::Metrics::default()).unwrap();
    assert!(empty.get("lot_sizing_diagnostics").is_none(),"OFF retains old serialized schema");
    drop(tape);std::fs::remove_file(path).unwrap();
}
