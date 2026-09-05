//! Actual Engine + SimBroker equivalence, not a second S/R implementation.
use crate::{TickData, SimBroker, RunConfig, run};
use crate::sr_warmup::snapshot_from_tickdata;
use conduit_core::{Broker, Engine, IncomingMessage, Settings};
use conduit_core::engine::{SrWarmupAppliedV2, SrWarmupSnapshotV2};
use conduit_core::settings::*;
use conduit_core::types::*;
use serde_json::json;
use std::{fs, path::PathBuf, sync::atomic::{AtomicU64,Ordering}};
const T:i64 = 1_800_000_000_000;
static SEQ:AtomicU64=AtomicU64::new(0);
struct DataFile(PathBuf);
impl DataFile {
    fn new(quotes:&[Quote])->Self {
        let p=std::env::temp_dir().join(format!("conduit-sr-v2-{}-{}.cdtk",std::process::id(),SEQ.fetch_add(1,Ordering::Relaxed)));
        let mut out=vec![0u8;64];out[..4].copy_from_slice(&0x4B544443u32.to_le_bytes());
        out[8..16].copy_from_slice(&(quotes.len()as u64).to_le_bytes());
        for q in quotes {out.extend(q.ts.to_le_bytes());out.extend((q.bid as f32).to_le_bytes());out.extend((q.ask as f32).to_le_bytes());}
        fs::write(&p,out).unwrap();Self(p)
    }
    fn open(&self,digits:Option<u32>)->TickData {let mut d=TickData::open(&self.0).unwrap();d.set_price_digits(digits).unwrap();d}
}
impl Drop for DataFile {fn drop(&mut self){let _=fs::remove_file(&self.0);}}
fn q(ts:i64,bid:f64,spread:f64)->Quote {Quote{ts,bid,ask:bid+spread}}
fn stream()->Vec<Quote> {
    let mut out=Vec::new();
    for i in 0..480 {
        // Gaps include a weekend-sized interval; no synthetic candles inserted.
        if (80..85).contains(&i) {continue;}
        let ts=T+i*60_000+if i>=210 {2*86_400_000}else{0};
        let base=4000.0+((i*7)%17) as f64;
        out.extend([q(ts+1234+i%701,base+2.017,0.823),q(ts+23_000,base-3.029,0.257),
            q(ts+23_000,base-1.035,0.613),q(ts+58_000,base+0.019,0.137)]);
    }
    out
}
fn config(tf:u32)->Settings {
    let mut c=Settings::default();
    c.sr_warmup_exact_ticks=true;c.trail_sr_enabled=true;c.trail_sr_tf_min=tf;
    c.trail_sr_fractal_n=1;c.trail_sr_atr_period=3;c.trail_sr_struct_window_h=1;
    c.trail_sr_offset_atr_mult=0.25;c.trail_sr_offset_spread_mult=2.0;
    c.trail_sr_scope=TrailSrScope::All;c.trail_sr_activation=TrailSrActivation::Entry;
    c.trail_sr_min_gain=0.0;c.trail_sr_min_dist_price=0.0;c.trail_sr_min_dist_tp=0.0;
    c.entry_units=3;c.lot_mode_percent=false;c.lot_fixed=0.04;
    c.risk_per_basket_pct=0.0;c.max_portfolio_risk_pct=0.0;
    c.tp_source=TpSource::SignalOnly;c.tp_schedule=TpSchedule::AllRunners;
    c.pending_lifetime=PendingLifetime::Never;c.pending_ttl_h=0.0;
    c.trail_mode=TrailMode::Off;c.trail_split=false;c.riskfree_enabled=false;c.risk_free_trail=false;
    c.rearm_grid_on_return=false;c.rearm_bez_pozycji=false;c
}
fn apply(e:&mut Engine,s:&SrWarmupSnapshotV2)->Result<SrWarmupAppliedV2,String> {
    e.rozgrzej_sr_v2(s,&s.context,s.cutoff_exclusive)
}
fn step(e:&mut Engine,b:&mut SimBroker,q:Quote){b.on_quote(q);e.on_tick(b,&q);}

#[test]
fn sr_v2_prefix_and_future_ledger_exact_tf_1_2_5_15() {
    let f=DataFile::new(&stream());
    for digits in [None,Some(2)] {let ticks=f.open(digits);
        for tf in [1,2,5,15] {for partial in [false,true] {
            let cutoff=T+2*86_400_000+360*60_000+if partial {30_000}else{0};
            let s=snapshot_from_tickdata(&ticks,T,cutoff).unwrap();
            let c=config(tf);let mut continuous=Engine::new(c.clone(),1000.0);
            let mut cb=SimBroker::z_ustawien(1000.0,&c);cb.price_digits=digits;
            for i in 0..ticks.index_at(cutoff){step(&mut continuous,&mut cb,ticks.quote(i));}
            let mut warm=Engine::new(c.clone(),1000.0);
            let stats_before=serde_json::to_vec(&warm.stats).unwrap();
            assert!(matches!(apply(&mut warm,&s).unwrap(),SrWarmupAppliedV2::Applied{dynamic_ready:true,..}));
            assert_eq!(stats_before,serde_json::to_vec(&warm.stats).unwrap(),"warmup changed non-SR stats");
            assert!(warm.baskets.is_empty());
            assert_eq!(serde_json::to_vec(&continuous.stan_sr()).unwrap(),serde_json::to_vec(&warm.stan_sr()).unwrap(),"prefix tf={tf} partial={partial} digits={digits:?}");
            let mut wb=SimBroker::z_ustawien(1000.0,&c);wb.price_digits=digits;
            let first=ticks.quote(ticks.index_at(cutoff));cb.on_quote(first);wb.on_quote(first);
            let msg=IncomingMessage{ts:first.ts,source:SourceKey::new(-10031,None),source_name:"SR-V2-SYNTHETIC".into(),
                msg_id:1,reply_to:None,edit_of:None,
                text:"BUY LIMITS GOLD @ 3998/3992 AREA\nTP 4100\nTP 4200\nTP 4300\nSL 3900".into()};
            continuous.on_message(&mut cb,&msg);warm.on_message(&mut wb,&msg);
            let fill=q(first.ts+1,3991.0,0.2);step(&mut continuous,&mut cb,fill);step(&mut warm,&mut wb,fill);
            assert_eq!(cb.positions().len(),3,"nonvacuous entry tf={tf}");
            let mut sl_changed=false;
            for i in ticks.index_at(cutoff)..ticks.len() {
                let quote=ticks.quote(i);if quote.ts<fill.ts{continue;}
                step(&mut continuous,&mut cb,quote);step(&mut warm,&mut wb,quote);
                sl_changed|=cb.positions().iter().any(|p|p.sl.is_some_and(|v|v>3900.0));
                assert_eq!(continuous.stan_sr(),warm.stan_sr(),"future SR tf={tf} i={i}");
                let ledger=|e:&Engine,b:&SimBroker|serde_json::to_vec(&json!({"baskets":e.baskets,"positions":b.positions(),
                    "pending":b.pendings(),"history":b.history,"balance":b.account().balance})).unwrap();
                assert_eq!(ledger(&continuous,&cb),ledger(&warm,&wb),"future ledger tf={tf} i={i}");
            }
            assert!(sl_changed,"must observe real S/R stop changes tf={tf}");
            // Explicitly cross the tightened stop as the final causal quote;
            // a larger TF can legitimately retain every runner in the oscillations.
            let stop_quote=q(ticks.last_ts()+1000,3890.0,0.2);
            step(&mut continuous,&mut cb,stop_quote);step(&mut warm,&mut wb,stop_quote);
            assert_eq!(serde_json::to_vec(&cb.history).unwrap(),serde_json::to_vec(&wb.history).unwrap());
            assert_eq!(cb.account().balance,wb.account().balance);
            assert_eq!(continuous.stan_sr(),warm.stan_sr());
            assert!(!cb.history.is_empty(),"must observe real closed trade tf={tf}");
        }}
    }
}

#[test]
fn sr_v2_invalid_snapshots_are_atomic_and_identity_bound() {
    let f=DataFile::new(&stream());let d=f.open(None);let s=snapshot_from_tickdata(&d,T,T+40*60_000).unwrap();
    let mut e=Engine::new(config(1),1000.0);apply(&mut e,&s).unwrap();
    let before=serde_json::to_vec(&e.stan_sr()).unwrap();
    let mut mutations:Vec<Box<dyn Fn(&mut SrWarmupSnapshotV2)>>=vec![
        Box::new(|x|x.version=1),Box::new(|x|x.complete_query_coverage=false),Box::new(|x|x.source_hash.clear()),
        Box::new(|x|x.minutes[3].first_tick_ts=x.cutoff_exclusive),Box::new(|x|x.minutes[3].last_tick_ts=x.cutoff_exclusive),
        Box::new(|x|x.minutes[3].low_mid=f64::NAN),Box::new(|x|x.minutes[3].last_spread=-0.1),
        Box::new(|x|x.minutes[3].close_mid=x.minutes[3].high_mid+1.0),Box::new(|x|x.minutes.swap(2,3)),
        Box::new(|x|x.minutes[3].tick_count=0),Box::new(|x|x.minutes[3].bucket_open_ts+=1),
        Box::new(|x|x.context.account_scope="other".into()),Box::new(|x|x.context.symbol="other".into()),
        Box::new(|x|x.context.runtime_generation="reconnected".into()),Box::new(|x|x.context.clock_domain="UTC".into()),
        Box::new(|x|x.context.explicit_offset_ms=10_800_000),Box::new(|x|x.context.price_normalization="other".into()),
        Box::new(|x|x.cutoff_exclusive+=1),Box::new(|x|x.from_inclusive=x.cutoff_exclusive),
    ];
    for (i,mutate) in mutations.drain(..).enumerate(){let mut bad=s.clone();mutate(&mut bad);
        assert!(e.rozgrzej_sr_v2(&bad,&s.context,s.cutoff_exclusive).is_err(),"mutation {i}");
        assert_eq!(before,serde_json::to_vec(&e.stan_sr()).unwrap(),"partial mutation {i}");}
}

#[test]
fn sr_v2_off_parent_off_and_static_sr_do_not_read_or_modify_bad_payload() {
    let f=DataFile::new(&stream());let d=f.open(None);let mut s=snapshot_from_tickdata(&d,T,T+20*60_000).unwrap();
    s.version=99;s.minutes[2].high_mid=f64::NAN;
    for mode in 0..3 {let mut c=config(1);match mode {0=>c.sr_warmup_exact_ticks=false,1=>c.trail_sr_enabled=false,
        _=>{c.trail_sr_offset_atr_mult=0.0;c.trail_sr_offset_spread_mult=0.0;c.trail_sr_min_prominence_atr=0.0;}}
        let mut e=Engine::new(c,1000.0);let before=serde_json::to_vec(&e.stan_sr()).unwrap();
        assert_eq!(apply(&mut e,&s).unwrap(),SrWarmupAppliedV2::Inactive);assert_eq!(before,serde_json::to_vec(&e.stan_sr()).unwrap());}
}

#[test]
fn sr_v2_producer_preserves_equal_timestamp_ticks_precision_and_cutoff() {
    let rows=[q(T+1234,4000.013,0.211),q(T+1234,4001.027,0.823),q(T+29_000,3999.041,0.117),q(T+30_000,9000.0,0.2)];
    let f=DataFile::new(&rows);let d=f.open(None);let rounded=f.open(Some(2));
    let s=snapshot_from_tickdata(&d,T,T+30_000).unwrap();let r=snapshot_from_tickdata(&rounded,T,T+30_000).unwrap();
    assert_eq!(s.minutes.len(),1);let m=&s.minutes[0];assert_eq!(m.tick_count,3);
    assert_eq!(m.first_tick_ts,T+1234);assert_eq!(m.last_tick_ts,T+29_000);
    assert_eq!(m.high_mid,d.quote(1).mid());assert_eq!(m.close_mid,d.quote(2).mid());
    assert_eq!(m.last_spread,d.quote(2).ask-d.quote(2).bid);assert_ne!(s.source_hash,r.source_hash);
    assert_eq!(r.minutes[0].close_mid,rounded.quote(2).mid());
    assert_eq!(s.context.explicit_offset_ms,0);assert!(s.context.clock_domain.contains("unchanged"));
}

#[test]
fn sr_v2_producer_rejects_bad_quotes_and_backward_delivery_instead_of_sorting() {
    for rows in [vec![q(T+1000,4000.0,0.2),q(T+3000,4001.0,0.2),q(T+2000,4002.0,0.2)],
        vec![q(T+1000,4000.0,-0.1)],vec![q(T+1000,f64::NAN,0.2)]] {
        let f=DataFile::new(&rows);let d=f.open(None);assert!(snapshot_from_tickdata(&d,T,T+60_000).is_err());
    }
}

#[test]
fn sr_v2_runner_unavailable_prefix_is_explicitly_nonrankable_and_off_json_unchanged() {
    let f=DataFile::new(&stream());let d=f.open(None);
    let mut cfg=RunConfig{from:T,to:T+60_000,start_balance:1000.0,settings:config(1),..Default::default()};
    for hours in [0,1] {cfg.rozgrzewka_h=hours;let r=run(&d,&[],&cfg);
        assert!(r.sr_warmup_reconciliation_required.is_some());assert_eq!(r.ticks_processed,0);}
    cfg.settings.sr_warmup_exact_ticks=false;let r=run(&d,&[],&cfg);
    assert!(!serde_json::to_value(r).unwrap().as_object().unwrap().contains_key("sr_warmup_reconciliation_required"));
}

#[test]
fn sr_v2_future_chain_leg_cannot_bypass_unsupported_mode_gate() {
    use crate::runner::{SzczebelCfg,FormatCfg};
    let f=DataFile::new(&stream());let d=f.open(None);
    let mut off=config(1);off.sr_warmup_exact_ticks=false;
    let leg=|c|FormatCfg{format:"Synergy".into(),preset:"SR-test".into(),settings:c};
    let cfg=RunConfig{from:T,to:T+60_000,settings:off.clone(),drabinka:vec![
        SzczebelCfg{prog:0.0,nazwa:"initial-OFF".into(),formaty:vec![leg(off)],pulapy:Default::default()},
        SzczebelCfg{prog:10000.0,nazwa:"future-ON".into(),formaty:vec![leg(config(1))],pulapy:Default::default()}],..Default::default()};
    let r=run(&d,&[],&cfg);assert!(r.sr_warmup_reconciliation_required.unwrap().contains("chain"));
    assert_eq!(r.ticks_processed,0);
}
