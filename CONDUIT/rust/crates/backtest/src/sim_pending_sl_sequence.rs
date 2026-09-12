//! Explicit broker execution profile, not an optimiser strategy axis.
use crate::{SimBroker, RunConfig, KonfOkien, TickData, run};
use conduit_core::{broker::*, settings::*, types::*};
const T:i64=1_800_000_000_000;
fn q(ts:i64,bid:f64,ask:f64)->Quote{Quote{ts,bid,ask}}
fn order(side:Side,sl:Option<f64>)->OrderReq{OrderReq{side,volume:0.01,sl,tp:None,
    basket:Some(1),level:0,is_toucher:false,comment:"B15-old".into()}}
fn pending(kind:PendingKind,sl:Option<f64>,tp:Option<f64>)->PendingReq{PendingReq{
    kind,volume:0.01,price:4000.0,sl,tp,basket:Some(1),level:1,
    is_toucher:false,is_topup:false,no_market_fallback:false,comment:"B15-pending".into()}}

#[test]
fn new_pending_sl_waits_one_invocation_old_sl_does_not_and_equal_ms_is_a_new_quote(){
    let cases=[
        (PendingKind::BuyLimit,q(T,4010.0,4010.2),3995.0,q(T+1000,3990.0,3990.2)),
        (PendingKind::SellLimit,q(T,3990.0,3990.2),4005.0,q(T+1000,4010.0,4010.2)),
        // STOP + widening spread: a legal pre-existing SL and the new SL are
        // both crossed while the opposite quote side activates the stop entry.
        (PendingKind::BuyStop,q(T,3990.0,3990.2),3985.0,q(T+1000,3980.0,4001.0)),
        (PendingKind::SellStop,q(T,4010.0,4010.2),4015.0,q(T+1000,3999.0,4020.0)),
    ];
    for (kind,initial,sl,fill) in cases {for enabled in [false,true]{
        let mut b=SimBroker::new(1000.0,0.2,0.0);assert!(!b.defer_new_pending_sl);
        b.defer_new_pending_sl=enabled;b.on_quote(initial);
        let old=b.open_market(order(kind.side(),Some(sl))).unwrap();
        b.place_pending(pending(kind,Some(sl),None)).unwrap();
        assert_eq!(b.on_quote(fill),(1,if enabled{1}else{2}),"{kind:?}, ON={enabled}");
        assert_eq!(b.history[0].ticket,old);assert_eq!(b.history[0].reason,CloseReason::Sl);
        assert_eq!(b.history[0].close_ts,fill.ts);assert!(b.pendings().is_empty());
        if enabled {
            let fresh=b.positions()[0].ticket;assert_ne!(fresh,old);
            assert_eq!(b.positions()[0].open_ts,fill.ts,"entry itself is not delayed");
            assert_eq!(b.on_quote(fill),(0,1),"a second observation with the SAME timestamp must execute SL");
            assert_eq!(b.history[1].ticket,fresh);assert_eq!(b.history[1].close_ts,fill.ts);
        }
        assert!(b.positions().is_empty());assert_eq!(b.history.len(),2);
    }}
}

#[test]
fn pending_profile_does_not_defer_tp_or_a_market_entry_with_the_same_timestamp(){
    for (kind,initial,fill,tp) in [
        (PendingKind::BuyStop,q(T,3990.,3990.2),q(T+1000,4010.,4010.2),4005.),
        (PendingKind::SellStop,q(T,4010.,4010.2),q(T+1000,3990.,3990.2),3995.),
    ] {
        let mut b=SimBroker::new(1000.,0.2,0.);b.defer_new_pending_sl=true;b.on_quote(initial);
        b.place_pending(pending(kind,None,Some(tp))).unwrap();
        assert_eq!(b.on_quote(fill),(1,1));assert_eq!(b.history[0].reason,CloseReason::Tp);
        assert_eq!(b.history[0].open_ts,b.history[0].close_ts);
    }
    let mut b=SimBroker::new(1000.,0.2,0.);b.defer_new_pending_sl=true;b.on_quote(q(T,4000.,4000.2));
    b.open_market(order(Side::Buy,Some(3999.))).unwrap();
    assert_eq!(b.on_quote(q(T,3998.,3998.2)),(0,1));assert_eq!(b.history[0].reason,CloseReason::Sl);
}

#[test]
fn pending_profile_never_blocks_explicit_protective_close_or_stop_out(){
    let mut b=SimBroker::new(1000.,0.2,0.);b.defer_new_pending_sl=true;b.on_quote(q(T,4010.,4010.2));
    b.place_pending(pending(PendingKind::BuyLimit,Some(3995.),None)).unwrap();
    assert_eq!(b.on_quote(q(T+1000,3990.,3990.2)),(1,0));
    let fresh=b.positions()[0].ticket;b.close_position(fresh,CloseReason::RiskFree).unwrap();
    assert_eq!(b.history[0].open_ts,b.history[0].close_ts);assert_eq!(b.history[0].reason,CloseReason::RiskFree);
    let mut b=SimBroker::new(100.,0.2,0.);b.defer_new_pending_sl=true;b.on_quote(q(T,4010.,4010.2));
    let mut p=pending(PendingKind::BuyLimit,Some(3995.),None);p.volume=0.1;b.place_pending(p).unwrap();
    assert_eq!(b.on_quote(q(T+1000,3990.,3990.2)),(1,1));
    assert_eq!(b.stop_outs,1);assert_eq!(b.history[0].reason,CloseReason::MaxDd);
}

fn config()->Settings {Settings{auto_limit:true,entry_units:1,lot_fixed:0.01,
    pending_lifetime:PendingLifetime::Never,pending_drop_on_target:false,
    server_tz_offset_ms:0,msg_clock_offset_ms:Some(0),exec_latency_ms:0,
    live_tick_order_strict:true,runner_ksiegowanie_v2:true,
    session_filter:false,skip_if_sl_breached:false,rearm_grid_on_return:false,
    swap_enabled:false,max_dd_pct:0.,max_dd_usd:0.,equity_floor_pct:0.,
    ..Settings::default()}}

#[test]
fn pending_profile_reaches_single_multi_runner_and_windows(){
    use crate::data::ReplayMessage;
    use crate::runner::FormatCfg;
    let path=std::env::temp_dir().join(format!("conduit-B15-model-{}.cdtk",std::process::id()));
    let quotes=[q(T,4010.,4010.2),q(T+1000,3990.,3990.2),q(T+2000,3990.,3990.2)];
    let mut data=vec![0u8;64];data[..4].copy_from_slice(&0x4B544443u32.to_le_bytes());
    data[8..16].copy_from_slice(&(quotes.len()as u64).to_le_bytes());
    for quote in quotes {data.extend(quote.ts.to_le_bytes());data.extend((quote.bid as f32).to_le_bytes());data.extend((quote.ask as f32).to_le_bytes());}
    std::fs::write(&path,data).unwrap();let ticks=TickData::open(&path).unwrap();
    let messages=[ReplayMessage{kanal:"B15".into(),ts:T,telegram_published_ts:None,msg_id:1,reply_to:None,edit_of:None,
        text:"BUY LIMITS GOLD @ 4000/4000 AREA\nTP 4030\nTP 4060\nTP 4090\nSL 3995".into()}];
    for enabled in [false,true] {for multi in [false,true]{
        let settings=config();let formaty=if multi{vec![FormatCfg{format:"B15".into(),preset:"fixture".into(),settings:settings.clone()}]}else{vec![]};
        let cfg=RunConfig{from:T,to:T+3000,start_balance:1000.,sim_new_pending_sl_next_tick:enabled,
            settings:settings.clone(),formaty:formaty.clone(),..Default::default()};
        let r=run(&ticks,&messages,&cfg);assert_eq!(r.trades.len(),1,"enabled={enabled}, multi={multi}");
        assert_eq!(r.trades[0].reason,CloseReason::Sl);
        assert_eq!(r.trades[0].open_ts,T+1000);assert_eq!(r.trades[0].close_ts,T+if enabled{2000}else{1000});
        let w=crate::okna::uruchom(&ticks,&messages,&KonfOkien{from:T,to:T+3000,start_balance:1000.,
            settings,formaty,sim_new_pending_sl_next_tick:enabled,..Default::default()});
        assert_eq!(w.okna.len(),1);assert_eq!(w.okna[0].trejdy,1);
        assert_eq!(w.suma,r.metrics.total_profit,"single/multi/windows profile must use the same broker execution");
    }}
    drop(ticks);std::fs::remove_file(path).unwrap();
}

#[test]
fn physical_ids_allow_multi_day_profile_without_forcing_bookkeeping_v2(){
    let day=T.div_euclid(86_400_000)*86_400_000;
    let quotes=[q(day-1000,4010.,4010.2),q(day+1000,3990.,3990.2),q(day+2000,3990.,3990.2)];
    let path=std::env::temp_dir().join(format!("conduit-B15-boundary-{}.cdtk",std::process::id()));
    let mut data=vec![0u8;64];data[..4].copy_from_slice(&0x4B544443u32.to_le_bytes());
    data[8..16].copy_from_slice(&(quotes.len()as u64).to_le_bytes());
    for quote in quotes {data.extend(quote.ts.to_le_bytes());data.extend((quote.bid as f32).to_le_bytes());data.extend((quote.ask as f32).to_le_bytes());}
    std::fs::write(&path,data).unwrap();let ticks=TickData::open(&path).unwrap();
    for enabled in [false,true] {for v2 in [false,true] {for daily_reset in [false,true] {
        let mut settings=config();settings.runner_ksiegowanie_v2=v2;
        let cfg=RunConfig{from:day-1000,to:day+3000,start_balance:1000.,settings:settings.clone(),
            sim_new_pending_sl_next_tick:enabled,daily_reset,rozgrzewka_h:72,..Default::default()};
        let result=run(&ticks,&[],&cfg);
        assert!(result.sim_execution_reconciliation_required.is_none());
        assert_eq!(result.ticks_processed,3);assert!(result.trades.is_empty());
        assert_eq!(cfg.settings.runner_ksiegowanie_v2,v2,"driver must not flip D4");
        assert!(serde_json::to_value(&result).unwrap().get("sim_execution_reconciliation_required").is_none());
        for n in [1,2] {
            let windows=crate::okna::uruchom(&ticks,&[],&KonfOkien{from:cfg.from,to:cfg.to,
                settings:settings.clone(),n_dni:n,sim_new_pending_sl_next_tick:enabled,..Default::default()});
            assert!(windows.sim_execution_reconciliation_required.is_none());
            assert_eq!(windows.tickow,3);assert!(!windows.okna.is_empty());
        }
        // The previous day's observation is now exclusively warmup: allowed.
        let single=RunConfig{from:day+1000,..cfg};
        assert!(run(&ticks,&[],&single).sim_execution_reconciliation_required.is_none());
        let windows=crate::okna::uruchom(&ticks,&[],&KonfOkien{from:day+1000,to:day+3000,
            settings:settings.clone(),sim_new_pending_sl_next_tick:enabled,..Default::default()});
        assert!(windows.sim_execution_reconciliation_required.is_none());
        // A nominal one-day window with a trading ZZN tail still spans days.
        let tail=crate::okna::uruchom(&ticks,&[],&KonfOkien{from:day-1000,to:day,
            settings,zzn:true,sim_new_pending_sl_next_tick:enabled,..Default::default()});
        assert!(tail.sim_execution_reconciliation_required.is_none());
        assert_eq!(tail.tickow,3);
    }}}
    drop(ticks);std::fs::remove_file(path).unwrap();
}

#[test]
fn bookkeeping_marks_are_not_new_market_observations(){
    let mut b=SimBroker::new(1000.,0.2,0.);b.defer_new_pending_sl=true;b.on_quote(q(T,4010.,4010.2));
    b.place_pending(pending(PendingKind::BuyLimit,Some(3995.),None)).unwrap();
    let fill=q(T+1000,3990.,3990.2);assert_eq!(b.on_quote(fill),(1,0));
    b.mark(fill);b.mark(fill);assert_eq!(b.positions().len(),1);assert!(b.history.is_empty());
    // A new real observation may have exactly the same ms and prices. The
    // caller supplies one on_quote per observation; the indexed driver now
    // ensures repeated day bookkeeping cannot impersonate another quote.
    assert_eq!(b.on_quote(fill),(0,1));
}

#[test]
fn physical_observation_id_never_executes_the_same_tick_twice(){
    let mut b=SimBroker::new(1000.,0.2,0.);b.defer_new_pending_sl=true;
    b.on_observation(40,q(T,4010.,4010.2)).unwrap();
    b.place_pending(pending(PendingKind::BuyLimit,Some(3995.),None)).unwrap();
    let fill=q(T+1000,3990.,3990.2);
    assert_eq!(b.on_observation(41,fill).unwrap(),(1,0));
    b.mark(fill);
    assert_eq!(b.on_observation(41,fill).unwrap(),(0,0),
        "bookkeeping re-entry for one physical tick must not activate its new SL");
    assert_eq!(b.positions().len(),1);assert!(b.history.is_empty());
    assert_eq!(b.on_observation(42,fill).unwrap(),(0,1),
        "a distinct tape row with identical timestamp AND prices is a new observation");
    assert_eq!(b.history.len(),1);assert_eq!(b.history[0].reason,CloseReason::Sl);
}

#[test]
fn physical_observation_id_conflicting_or_reversed_identity_is_explicit_error(){
    let mut b=SimBroker::new(1000.,0.2,0.);b.defer_new_pending_sl=true;
    let first=q(T,4010.,4010.2);b.on_observation(50,first).unwrap();
    let balance=b.balance;
    assert!(b.on_observation(50,q(T,4000.,4000.2)).is_err());
    assert!(b.on_observation(49,first).is_err());
    assert_eq!(b.balance,balance);assert_eq!(b.quote().bid,first.bid);
    assert_eq!(b.on_observation(51,first).unwrap(),(0,0));
}

#[test]
fn physical_observation_same_id_cannot_fill_an_order_created_after_that_quote(){
    // A zero-distance model accepts this as a real pending. With 0.2 the
    // intentional market fallback executes inside place_pending instead.
    let mut b=SimBroker::new(1000.,0.,0.);b.defer_new_pending_sl=true;
    let quote=q(T,4000.,4000.2);b.on_observation(7,quote).unwrap();
    let mut req=pending(PendingKind::BuyStop,Some(3990.),None);
    req.price=4000.2;b.place_pending(req).unwrap();
    assert_eq!(b.market_instead_of_limit,0);
    assert_eq!(b.on_observation(7,quote).unwrap(),(0,0));
    assert_eq!(b.pendings().len(),1);assert!(b.positions().is_empty());
    assert_eq!(b.on_observation(8,quote).unwrap(),(1,0));
    assert_eq!(b.positions().len(),1);
}

#[test]
fn physical_observation_repeat_restores_quote_after_older_message_context(){
    let mut b=SimBroker::new(1000.,0.2,0.);b.defer_new_pending_sl=true;
    b.price_digits=Some(2);
    let current=q(T,4000.123,4000.327);b.on_observation(9,current).unwrap();
    let balance=b.balance;
    // D3 supplies a prior quote while dispatching a message between ticks.
    b.q=q(T-1000,4010.,4010.2);
    assert_eq!(b.on_observation(9,current).unwrap(),(0,0));
    assert_eq!(b.quote().ts,T);assert_eq!(b.quote().bid,4000.12);
    assert_eq!(b.quote().ask,4000.33);assert_eq!(b.balance,balance);
}

#[test]
fn physical_ids_execute_sl_on_next_real_row_across_midnight_in_runner_and_windows(){
    use crate::data::ReplayMessage;
    use crate::runner::FormatCfg;
    let day=T.div_euclid(86_400_000)*86_400_000;
    let quotes=[q(day-1000,4010.,4010.2),q(day+1000,3990.,3990.2),
        q(day+1000,3980.,3980.2),q(day+2000,4000.,4000.2)];
    let path=std::env::temp_dir().join(format!("conduit-B15-indexed-real-{}.cdtk",std::process::id()));
    let mut data=vec![0u8;64];data[..4].copy_from_slice(&0x4B544443u32.to_le_bytes());
    data[8..16].copy_from_slice(&(quotes.len()as u64).to_le_bytes());
    for quote in quotes {data.extend(quote.ts.to_le_bytes());data.extend((quote.bid as f32).to_le_bytes());data.extend((quote.ask as f32).to_le_bytes());}
    std::fs::write(&path,data).unwrap();let ticks=TickData::open(&path).unwrap();
    let messages=[ReplayMessage{kanal:"B15".into(),ts:day-1000,telegram_published_ts:None,msg_id:11,reply_to:None,edit_of:None,
        text:"BUY LIMITS GOLD @ 4000/4000 AREA\nTP 4030\nTP 4060\nTP 4090\nSL 3995".into()}];
    for enabled in [false,true] {for v2 in [false,true] {for strict in [false,true] {for multi in [false,true] {
        let mut settings=config();settings.runner_ksiegowanie_v2=v2;settings.live_tick_order_strict=strict;
        let formaty=if multi {vec![
            FormatCfg{format:"B15".into(),preset:"trading".into(),settings:settings.clone()},
            FormatCfg{format:"Other".into(),preset:"idle".into(),settings:settings.clone()},
        ]} else {vec![]};
        let cfg=RunConfig{from:day-1000,to:day+3000,start_balance:1000.,
            sim_new_pending_sl_next_tick:enabled,settings:settings.clone(),formaty:formaty.clone(),
            ..Default::default()};
        let r=run(&ticks,&messages,&cfg);
        assert!(r.sim_execution_reconciliation_required.is_none());
        assert_eq!(r.trades.len(),1,"ON={enabled} D4={v2} strict={strict} multi={multi}");
        let trade=&r.trades[0];assert_eq!(trade.reason,CloseReason::Sl);
        assert_eq!(trade.open_ts,day+1000);assert_eq!(trade.close_ts,day+1000);
        assert_eq!(trade.close_price,if enabled {3980.} else {3990.},
            "same millisecond does not erase the second physical observation");
        let w=crate::okna::uruchom(&ticks,&messages,&KonfOkien{
            from:cfg.from,to:cfg.to,start_balance:1000.,n_dni:2,
            sim_new_pending_sl_next_tick:enabled,settings,formaty,..Default::default()});
        assert!(w.sim_execution_reconciliation_required.is_none());
        assert_eq!(w.okna.len(),1);assert_eq!(w.okna[0].trejdy,1);
        assert_eq!(w.suma,r.metrics.total_profit);
    }}}}
    drop(ticks);std::fs::remove_file(path).unwrap();
}
