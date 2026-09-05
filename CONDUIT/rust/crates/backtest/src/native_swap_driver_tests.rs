//! Execution-profile propagation tests, not strategy tuning or native proof.
use crate::{run, KonfOkien, RunConfig, TickData};
use crate::data::ReplayMessage;
use crate::runner::FormatCfg;
use conduit_core::{settings::*, types::*};

const T0: i64 = 1_787_777_999_000;
const FILL: i64 = 1_787_778_000_017;
const NIGHT: i64 = 1_787_796_000_016;

struct Tape { ticks: Option<TickData>, path: std::path::PathBuf }
impl Tape {
    fn new(name: &str) -> Self {
        let quotes = [(T0, 4002.0f32, 4002.2f32), (FILL, 3999.8, 4000.0),
                      (NIGHT, 4001.0, 4001.2), (NIGHT+1000, 3970.0, 3970.2)];
        let path = std::env::temp_dir().join(format!("conduit-swap-driver-{name}-{}.cdtk", std::process::id()));
        let mut bytes = vec![0u8;64];
        bytes[..4].copy_from_slice(&0x4B544443u32.to_le_bytes());
        bytes[8..16].copy_from_slice(&(quotes.len() as u64).to_le_bytes());
        for (ts,bid,ask) in quotes {
            bytes.extend(ts.to_le_bytes()); bytes.extend(bid.to_le_bytes()); bytes.extend(ask.to_le_bytes());
        }
        std::fs::write(&path,bytes).unwrap();
        let ticks = TickData::open(&path).unwrap();
        Self { ticks:Some(ticks), path }
    }
    fn ticks(&self) -> &TickData { self.ticks.as_ref().unwrap() }
}
impl Drop for Tape {
    fn drop(&mut self) { self.ticks.take(); let _ = std::fs::remove_file(&self.path); }
}
fn cfg(net: bool) -> Settings {
    Settings {
        auto_limit:true, entry_units:1, lot_fixed:0.07,
        pending_lifetime:PendingLifetime::Never, pending_drop_on_target:false,
        server_tz_offset_ms:0, msg_clock_offset_ms:Some(0), exec_latency_ms:0,
        live_tick_order_strict:true, runner_ksiegowanie_v2:true,
        session_filter:false, skip_if_sl_breached:false, rearm_grid_on_return:false,
        riskfree_enabled:false, max_dd_pct:0.0, max_dd_usd:0.0, equity_floor_pct:0.0,
        swap_enabled:true, swap_long_points:-80.54, swap_short_points:32.67,
        swap_point_value:1.0, swap_rollover_z_serwera:true, swap_rollover3days_mt5:3,
        swap_rollover_mult:3.0, swap_pomijaj_weekend:false,
        basket_realized_broker_only:true, closed_profit_net_costs:net,
        commission_per_lot:0.0, ..Settings::default()
    }
}
fn near(a:f64,b:f64,label:&str) { assert!((a-b).abs()<1e-8,"{label}: actual{a}, expected{b}"); }

#[test]
fn swap_profile_reaches_single_multi_runner_windows_independent_of_net_and_b15() {
    let tape=Tape::new("propagation");
    let messages=[ReplayMessage {kanal:"SwapDriver".into(),ts:T0,telegram_published_ts:None,msg_id:1,
        reply_to:None,edit_of:None,
        text:"BUY LIMITS GOLD @ 4000/4000 AREA\nTP 4030\nTP 4060\nTP 4090\nSL 3980".into()}];
    for digits in [None,Some(2)] { for net in [false,true] {
        for b15 in [false,true] { for multi in [false,true] { for credit in [0.0,300.0] {
            let mut settings=cfg(net);
            settings.credit_balance_separate=credit>0.0;
            settings.kredyt_reczny=credit;
            let formaty=if multi {vec![
                FormatCfg {format:"SwapDriver".into(),preset:"active".into(),settings:settings.clone()},
                FormatCfg {format:"Idle".into(),preset:"idle".into(),settings:settings.clone()},
            ]} else {vec![]};
            let config=RunConfig {from:T0,to:NIGHT+1001,start_balance:5000.0,
                settings:settings.clone(),formaty:formaty.clone(),curve_interval_ms:1,
                sim_new_pending_sl_next_tick:b15,sim_native_swap_cash_digits:digits,
                ..Default::default()};
            let r=run(tape.ticks(),&messages,&config);
            let label=format!("digits={digits:?} NET={net} B15={b15} multi={multi} credit={credit}");
            assert!(r.sim_execution_reconciliation_required.is_none(),"{label}");
            assert!(r.cost_reconciliation_required.is_none(),"{label}");
            assert_eq!(r.trades.len(),1,"{label}");
            let tr=&r.trades[0];
            assert_eq!(tr.reason,CloseReason::Sl,"{label}");
            near(tr.volume,0.07,&label); near(tr.open_price,4000.0,&label);
            near(tr.close_price,3970.0,&label);
            let swap=if digits.is_some(){-16.91}else{-16.9134};
            near(tr.swap,swap,&label); near(r.swap_paid,swap,&label);
            let before_balance=r.balance_curve.iter().find(|(ts,_)|*ts==NIGHT).unwrap().1;
            near(before_balance,if digits.is_some(){5000.0}else{5000.0+swap},&label);
            near(r.metrics.end_balance,5000.0-210.0+swap,&label);
            let w=crate::okna::uruchom(tape.ticks(),&messages,&KonfOkien {
                from:T0,to:NIGHT+1001,start_balance:5000.0,n_dni:2,
                settings,formaty,sim_new_pending_sl_next_tick:b15,
                sim_native_swap_cash_digits:digits,..Default::default()
            });
            assert!(w.sim_execution_reconciliation_required.is_none(),"{label}");
            assert_eq!(w.okna.len(),1,"{label}"); assert_eq!(w.okna[0].trejdy,1,"{label}");
            near(w.suma,r.metrics.end_balance-5000.0,&label);
        }}}
    }}
}

#[test]
fn invalid_cash_model_is_non_rankable_even_without_executable_ticks() {
    let tape=Tape::new("invalid");
    let r=run(tape.ticks(),&[],&RunConfig {from:0,to:0,
        sim_native_swap_cash_digits:Some(9),..Default::default()});
    assert!(r.sim_execution_reconciliation_required.is_some());
    assert_eq!(r.ticks_processed,0); assert!(r.trades.is_empty());
    let w=crate::okna::uruchom(tape.ticks(),&[],&KonfOkien {from:0,to:0,
        sim_native_swap_cash_digits:Some(u32::MAX),..Default::default()});
    assert!(w.sim_execution_reconciliation_required.is_some());
    assert_eq!(w.tickow,0); assert!(w.okna.is_empty());
}

#[test]
fn cost_hold_reports_only_observed_ticks_and_actual_stop_time() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let tape = Tape::new("hold-progress");
    let messages = [ReplayMessage { kanal:"SwapDriver".into(), ts:T0, telegram_published_ts:None, msg_id:1,
        reply_to:None, edit_of:None,
        text:"BUY LIMITS GOLD @ 4000/4000 AREA\nTP 4030\nTP 4060\nTP 4090\nSL 3980".into() }];
    for net in [false, true] {
        let mut settings = cfg(net);
        // Both values are finite and valid JSON. Their product first overflows
        // when the open position crosses the rollover on the third quote.
        settings.swap_long_points = 1e308;
        settings.swap_point_value = 1e308;
        let counted = AtomicU64::new(0);
        let progress = |delta| { counted.fetch_add(delta, Ordering::Relaxed); true };
        let result = crate::runner::run_with_progress(tape.ticks(), &messages, &RunConfig {
            from:T0, to:NIGHT+1001, start_balance:5000.0, settings,
            sim_native_swap_cash_digits:Some(2), curve_interval_ms:1,
            ..Default::default()
        }, Some(&progress));
        assert!(result.cost_reconciliation_required.as_deref()
            .is_some_and(|reason| reason.contains("native swap accrual/overflow")));
        assert_eq!(result.ticks_processed, 3, "NET={net}: fourth quote was never processed");
        assert_eq!(counted.load(Ordering::Relaxed), 3, "NET={net}: no fictional progress");
        assert_eq!(result.equity_curve.last().unwrap().0, NIGHT, "NET={net}");
        assert_eq!(result.balance_curve.last().unwrap().0, NIGHT, "NET={net}");
        assert!(!result.cancelled, "HOLD is not a user cancellation");
        assert!(result.reconciliation_hold().is_some());
    }
}

#[test]
fn unknown_native_swap_cost_cannot_be_ranked_by_windows() {
    let tape=Tape::new("unknown-cost");
    let messages=[ReplayMessage {kanal:"SwapDriver".into(),ts:T0,telegram_published_ts:None,msg_id:1,
        reply_to:None,edit_of:None,
        text:"BUY LIMITS GOLD @ 4000/4000 AREA\nTP 4030\nTP 4060\nTP 4090\nSL 3980".into()}];
    for net in [false,true] { for zzn in [false,true] { for multi in [false,true] {
        let mut settings=cfg(net);
        settings.swap_long_points=f64::NAN;
        let formaty=if multi { vec![
            FormatCfg {format:"SwapDriver".into(),preset:"active".into(),settings:settings.clone()},
            FormatCfg {format:"Idle".into(),preset:"idle".into(),settings:settings.clone()},
        ] } else { vec![] };
        let r=run(tape.ticks(),&messages,&RunConfig {from:T0,to:NIGHT+1001,
            start_balance:5000.0,settings:settings.clone(),formaty:formaty.clone(),
            sim_native_swap_cash_digits:Some(2),..Default::default()});
        assert!(r.cost_reconciliation_required.is_some(),"runner NET={net} multi={multi}");
        let w=crate::okna::uruchom(tape.ticks(),&messages,&KonfOkien {
            from:T0,to:NIGHT+1001,start_balance:5000.0,n_dni:2,settings,formaty,zzn,
            sim_native_swap_cash_digits:Some(2),..Default::default()
        });
        assert!(w.sim_execution_reconciliation_required.is_some(),
            "unknown swap was presented as a ranked window: NET={net} ZZN={zzn} multi={multi}, sum={}",w.suma);
        assert!(w.okna.is_empty(),"unreconciled windows must not publish ranking rows");
    }}}
}

#[test]
fn daily_reset_and_flat_settle_native_swap_once_with_or_without_credit() {
    let tape=Tape::new("reset-flat-credit");
    let messages=[ReplayMessage {kanal:"SwapDriver".into(),ts:T0,telegram_published_ts:None,msg_id:1,
        reply_to:None,edit_of:None,
        text:"BUY LIMITS GOLD @ 4000/4000 AREA\nTP 4030\nTP 4060\nTP 4090\nSL 3980".into()}];
    for digits in [None,Some(2)] { for net in [false,true] {
        for b15 in [false,true] { for reset in [false,true] { for credit in [0.0,300.0] {
            let mut settings=cfg(net);
            settings.credit_balance_separate=credit>0.0;
            settings.kredyt_reczny=credit;
            let r=run(tape.ticks(),&messages,&RunConfig {from:T0,to:NIGHT+1001,
                start_balance:5000.0,settings,curve_interval_ms:1,
                daily_reset:reset,flat_na_dobie:!reset,
                sim_native_swap_cash_digits:digits,sim_new_pending_sl_next_tick:b15,
                ..Default::default()});
            let label=format!("digits={digits:?} NET={net} B15={b15} reset={reset} credit={credit}");
            assert!(r.reconciliation_hold().is_none(),"{label}: {:?}",r.reconciliation_hold());
            assert_eq!(r.trades.len(),1,"{label}");
            let tr=&r.trades[0];
            assert_eq!(tr.reason,CloseReason::EodFlat,"{label}");
            assert_eq!(tr.close_ts,NIGHT,"{label}");
            near(tr.close_price,4001.0,&label);
            let swap=if digits.is_some(){-16.91}else{-16.9134};
            near(tr.swap,swap,&label); near(r.swap_paid,swap,&label);
            near(tr.profit,7.0+swap,&label);
            near(r.metrics.end_balance,5000.0+7.0+swap,&label);
            let cash=r.balance_curve.iter().find(|(ts,_)|*ts==NIGHT).unwrap().1;
            near(cash,if reset{5000.0}else{5000.0+7.0+swap},&label);
        }}}
    }}
}

#[test]
fn shared_reconciliation_gate_keeps_all_fault_kinds_out_of_ranking() {
    let tape=Tape::new("ranking-gate");
    let clean=run(tape.ticks(),&[],&RunConfig {from:0,to:0,..Default::default()});
    assert!(clean.reconciliation_hold().is_none());
    for kind in ["COST","SR WARMUP","SIM EXECUTION","CONTINUATION"] {
        let mut r=clean.clone();
        match kind {
            "COST"=>r.cost_reconciliation_required=Some("incomplete".into()),
            "SR WARMUP"=>r.sr_warmup_reconciliation_required=Some("incomplete".into()),
            "SIM EXECUTION"=>r.sim_execution_reconciliation_required=Some("incomplete".into()),
            _=>r.continuation_reconciliation_required=Some("incomplete".into()),
        }
        assert_eq!(r.reconciliation_hold(),Some((kind,"incomplete")));
        // The shared gate is not a new result field or a synthetic strategy.
        let restored:crate::runner::RunResult=serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(restored.reconciliation_hold(),r.reconciliation_hold());
    }
}
