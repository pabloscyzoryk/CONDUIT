//! Real Bridge + Transport + Engine, synthetic loopback only. No Python/MT5/Telegram.
use conduit_core::{broker::{Broker, OrderReq}, engine::Engine, settings::Settings, types::*};
use conduit_mt5::{Mt5Bridge, SidecarConfig};
use serde_json::{json, Value};
use std::{io::{BufRead, BufReader, Write}, net::{TcpListener, TcpStream}, thread,
    sync::{Arc, Mutex, MutexGuard, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant}};

const TS: i64 = 8_683_201_000;
const ID: u64 = 9001;
// The test's external connector needs a port before Bridge::connect starts its
// listener. Serialize the reserve/drop/bind fixture lifetime so one fixture's
// ephemeral client cannot steal another fixture's just-reserved port on Windows.
static FIXTURE_PORT_LOCK: Mutex<()> = Mutex::new(());

fn account() -> Value { json!({"login":42,"server":"fixture-demo","trade_mode":0}) }
fn write(s: &mut TcpStream, v: &Value) {
    writeln!(s, "{}", serde_json::to_string(v).unwrap()).unwrap();
    s.flush().unwrap();
}

struct Fixture { bridge: Mt5Bridge, worker: Option<thread::JoinHandle<()>>, stop: Arc<AtomicBool>, _port_lock: MutexGuard<'static, ()> }
impl Fixture {
    fn new(on: bool, initial_volume: f64) -> Self {
        Self::with_timeout(on,initial_volume,Duration::from_secs(3))
    }
    fn with_timeout(on: bool, initial_volume: f64, timeout:Duration) -> Self {
        Self::with_costs(on,initial_volume,timeout,false)
    }
    fn with_costs(on: bool, initial_volume: f64, timeout:Duration, net_costs:bool) -> Self {
        let port_lock=FIXTURE_PORT_LOCK.lock().unwrap_or_else(|e|e.into_inner());
        let reservation = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(s) => break s,
                    Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                    Err(e) => panic!("fixture connection: {e}"),
                }
            };
            stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            write(&mut stream, &json!({"ev":"hello","proto":1,"sidecar":"OFFLINE-CLOSE-RECEIPT-FIXTURE"}));
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            let mut volume = initial_volume;
            let mut snapshot_volume_override = None::<f64>;
            let mut ticket = ID;
            let mut identifier = ID;
            let mut deal = 9100u64;
            let mut balance = 1000.0 - if net_costs {initial_volume*8.0} else {0.0};
            let mut frames = Vec::<Value>::new();
            let mut last_frames = Vec::<Value>::new();
            let mut fail_positions = false;
            let mut partial_cap = None::<f64>;
            let mut profit_per_lot = 50.0;
            let mut pending_orders = Vec::<Value>::new();
            let mut extra_positions = Vec::<Value>::new();
            let mut position_kind=0i64;
            let mut drop_next_response=false;
            let mut reject_next_trade=false;
            let mut valid_open=false;
            let mut malformed_next_response=false;
            let mut disconnect_next_response=false;
            let mut next_open_ack_patch=None::<Value>;
            let mut next_close_ack_patch=None::<Value>;
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {},
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                        if stop_worker.load(Ordering::Relaxed) { break; }
                        continue;
                    },
                    Err(e) => panic!("fixture read: {e}"),
                }
                let req: Value = serde_json::from_str(&line).unwrap();
                let args = &req["args"];
                let trade=matches!(req["cmd"].as_str(),Some("open_market"|"place_pending"|"close_position"|"close_partial"));
                if trade && reject_next_trade {
                    reject_next_trade=false;
                    write(&mut stream,&json!({"id":req["id"],"ok":false,"error":{"code":10006,"msg":"fixture explicit rejection"}}));
                    continue;
                }
                let mut result = match req["cmd"].as_str().unwrap() {
                    "account" => json!({"login":42,"server":"fixture-demo","trade_mode":0,
                        "balance":balance,"equity":balance,"margin":0.,"margin_free":balance,"leverage":1000,
                        "currency":if net_costs {"USD"}else{""}}),
                    "symbol_info" => json!({"symbol":"XAUUSD","digits":2,"point":0.01,
                        "stops_level_points":0.,"volume_min":0.01,"volume_max":100.,"volume_step":0.01,
                        "contract_size":100.,"trade_mode":4}),
                    "quote" => json!({"bid":4000.5,"ask":4000.5,"ts":TS}),
                    "positions" if fail_positions => json!("SIMULATED BAD SNAPSHOT; NOT EMPTY POSITIONS"),
                    "positions" => {
                        let mut rows=extra_positions.clone();
                        let observed_volume=snapshot_volume_override.unwrap_or(volume);
                        if observed_volume>1e-9 { rows.push(json!({"ticket":ticket,"identifier":identifier,
                            "kind":position_kind,"volume":observed_volume,"price_open":4000.,"time_msc":TS-1000,
                            "magic":777,"comment":"CD1.0","symbol":"XAUUSD"})); }
                        json!(rows)
                    }
                    "orders" => json!(pending_orders),
                    "subscribe_ticks" => json!({}),
                    "open_market" if valid_open => {
                        let ticket=ID+10+extra_positions.len() as u64;
                        extra_positions.push(json!({"ticket":ticket,"identifier":ticket,
                            "kind":0,"volume":0.01,"price_open":4000.5,"time_msc":TS,
                            "magic":777,"comment":args["comment"],"symbol":"XAUUSD"}));
                        json!({"retcode":10009,"deal":8888,"order":ticket,"position":ticket,
                            "position_identifier":ticket,"volume":0.01,"price":4000.5})
                    }
                    "open_market" => json!({"retcode":10009,"deal":8888,"order":ID,
                        "position":0,"position_identifier":0,"volume":0.01,"price":4000.5}),
                    "close_position" | "close_partial" | "probe_sl" | "probe_tp" => {
                        let mut cut = args.get("volume").and_then(Value::as_f64).unwrap_or(volume);
                        if let Some(cap) = partial_cap { cut = cut.min(cap); }
                        assert!(cut > 0. && cut <= volume + 1e-9);
                        volume = (volume - cut).max(0.); balance += cut * profit_per_lot;
                        let sl = req["cmd"] == "probe_sl";
                        let tp = req["cmd"] == "probe_tp";
                        let mut frame=json!({"ev":"closed","account":account(),"deal":deal,
                            "position":identifier,"deal_type":1,"volume":cut,"price":4000.5,
                            "time_msc":TS+deal as i64,"profit":cut*profit_per_lot,"commission":0.,"swap":0.,
                            "reason":if sl {4}else if tp {5}else{3},"magic":777,"comment":"close","symbol":"XAUUSD",
                            "price_open":4000.,"time_open_msc":TS-1000});
                        if net_costs {
                            let commission=-3.0*cut;let swap=-75.0*cut;let fee=-0.125*cut;
                            balance+=commission+swap+fee;
                            frame["commission"]=json!(commission);frame["swap"]=json!(swap);
                            frame["cost_receipt"]=fixture_cost_payload(deal,identifier,cut,cut*profit_per_lot,commission,swap);
                        }
                        frames.push(frame);
                        let result = json!({"retcode":10009,"deal":deal,"position":ticket,
                            "position_identifier":identifier,"volume":cut,"price":4000.5,"profit":cut*profit_per_lot});
                        deal += 1;
                        result
                    }
                    "probe_emit" => {
                        for f in &frames { write(&mut stream, f); }
                        last_frames = std::mem::take(&mut frames);
                        json!({})
                    }
                    "probe_duplicate" => {
                        for f in &last_frames { write(&mut stream, f); }
                        json!({})
                    }
                    "probe_duplicate_cost" => {
                        for f in &last_frames {
                            let mut copy=f.clone();copy["cost_receipt"]=args["value"].clone();
                            write(&mut stream,&copy);
                        }
                        json!({})
                    }
                    "probe_duplicate_as_owned" => {
                        for f in &last_frames {
                            let mut copy=f.clone(); copy["ev"]=json!("closed");
                            write(&mut stream,&copy);
                        }
                        json!({})
                    }
                    "probe_conflict" => {
                        let mut f = last_frames[0].clone(); f["profit"] = json!(999.);
                        write(&mut stream, &f); json!({})
                    }
                    "probe_corrupt" => {
                        let field = args["field"].as_str().unwrap();
                        for f in &mut frames { f[field] = args["value"].clone(); }
                        json!({})
                    }
                    "probe_remove_field" => {
                        for f in &mut frames { f.as_object_mut().unwrap().remove(args["field"].as_str().unwrap()); }
                        json!({})
                    }
                    "probe_fail_positions" => { fail_positions = args["enabled"].as_bool().unwrap(); json!({}) }
                    "probe_ticket" => { ticket = args["ticket"].as_u64().unwrap(); json!({}) }
                    "probe_snapshot_volume" => {snapshot_volume_override=args["value"].as_f64();json!({})}
                    "probe_pending_roundtrip_unobserved" => {
                        let order=pending_orders.pop().expect("pending required");
                        frames.push(json!({"ev":"closed","account":account(),"deal":9901,
                            "position":order["ticket"],"deal_type":1,"volume":0.01,"price":4000.5,
                            "time_msc":TS+9901,"profit":0.5,"commission":0.0,"swap":0.0,"reason":5,
                            "magic":777,"symbol":"XAUUSD","price_open":4000.0,"time_open_msc":TS+9800}));
                        json!({})
                    }
                    "probe_foreign_close" => {
                        // A foreign trade must not secretly remove our observed
                        // position. The old fixture rewrote an OWN close's ID,
                        // leaving an independent, legitimately unexplained gap.
                        frames.push(json!({"ev":"closed_foreign","account":account(),"deal":9902,
                            "position":7777,"deal_type":1,"volume":0.08,"price":4000.5,
                            "time_msc":TS+9902,"profit":4.0,"commission":0.0,"swap":0.0,
                            "reason":0,"magic":0,"symbol":"XAUUSD","price_open":4000.0,
                            "time_open_msc":TS-1000}));
                        balance+=4.0;json!({})
                    }
                    "probe_identifier" => { identifier = args["identifier"].as_u64().unwrap(); json!({}) }
                    "probe_partial_cap" => { partial_cap = args["cap"].as_f64(); json!({}) }
                    "probe_profit_per_lot" => { profit_per_lot=args["value"].as_f64().unwrap(); json!({}) }
                    "probe_position_kind" => { position_kind=args["value"].as_i64().unwrap(); json!({}) }
                    "probe_drop_next_response" => { drop_next_response=true; json!({}) }
                    "probe_reject_next_trade" => { reject_next_trade=true; json!({}) }
                    "probe_valid_open" => { valid_open=true; json!({}) }
                    "probe_next_open_ack_patch" => {next_open_ack_patch=Some(args.clone());json!({})}
                    "probe_next_close_ack_patch" => {next_close_ack_patch=Some(args.clone());json!({})}
                    "probe_malformed_next_response" => { malformed_next_response=true; json!({}) }
                    "probe_disconnect_next_response" => { disconnect_next_response=true; json!({}) }
                    "probe_add_second_slot" => {
                        extra_positions.push(json!({"ticket":ID+1,"identifier":ID+1,
                            "kind":0,"volume":0.04,"price_open":4000.,"time_msc":TS-1000,
                            "magic":777,"comment":"CD100001.0","symbol":"XAUUSD"})); json!({})
                    }
                    "probe_close_second_slot" => {
                        assert_eq!(extra_positions.len(),1); extra_positions.clear();
                        frames.push(json!({"ev":"closed","account":account(),"deal":9200,
                            "position":ID+1,"deal_type":1,"volume":0.04,"price":4000.5,
                            "time_msc":TS+9200,"profit":2.,"commission":0.,"swap":0.,
                            "reason":4,"magic":777,"comment":"close","symbol":"XAUUSD",
                            "price_open":4000.,"time_open_msc":TS-1000})); json!({})
                    }
                    "place_pending" => {
                        let ticket=20_000+pending_orders.len() as u64;
                        let mut order=args.clone(); order["ticket"]=json!(ticket);
                        order["magic"]=json!(777); order["time_msc"]=json!(TS);
                        order["price_open"]=args["price"].clone();
                        // MT5 snapshots encode absent SL/TP as numeric zero,
                        // unlike optional RPC request fields (JSON null).
                        for field in ["sl","tp"] {if order[field].is_null(){order[field]=json!(0.0);}}
                        pending_orders.push(order);
                        json!({"retcode":10009,"order":ticket})
                    }
                    "cancel_pending" => {
                        pending_orders.retain(|p|p["ticket"]!=args["ticket"]); json!({})
                    }
                    "modify_position" | "modify_pending" => json!({}),
                    "probe_finish" => {
                        write(&mut stream, &json!({"id":req["id"],"ok":true,"result":{}})); break;
                    }
                    unexpected => panic!("Unexpected command (no real orders): {unexpected}"),
                };
                if req["cmd"]=="open_market" {
                    if let Some(patch)=next_open_ack_patch.take() {
                        for (field,value) in patch.as_object().unwrap() {
                            if value.is_null(){result.as_object_mut().unwrap().remove(field);}
                            else{result[field]=value.clone();}
                        }
                    }
                }
                if matches!(req["cmd"].as_str(),Some("close_position"|"close_partial")) {
                    if let Some(patch)=next_close_ack_patch.take() {
                        for (field,value) in patch.as_object().unwrap() {
                            if value.is_null(){result.as_object_mut().unwrap().remove(field);}
                            else{result[field]=value.clone();}
                        }
                    }
                }
                if trade && drop_next_response { drop_next_response=false; continue; }
                if trade && disconnect_next_response { break; }
                if trade && malformed_next_response {
                    malformed_next_response=false;
                    write(&mut stream,&json!({"id":req["id"],"ok":true,"result":"NOT A TRADE ACK"}));
                    continue;
                }
                write(&mut stream, &json!({"id":req["id"],"ok":true,"result":result}));
            }
        });
        let bridge = Mt5Bridge::connect(SidecarConfig { autostart:false, port, magic:777,
            close_receipt_reconcile:on, request_timeout:timeout,
            closed_profit_net_costs:net_costs,
            connect_timeout:Duration::from_secs(3), restart_backoff:Duration::from_millis(50),
            ..Default::default() }).unwrap();
        assert_eq!(bridge.positions().len(), usize::from(initial_volume>0.0));
        if initial_volume>0.0 {near(bridge.positions()[0].volume, initial_volume);}
        Self {bridge, worker:Some(worker), stop, _port_lock:port_lock}
    }
    fn call(&self, cmd: &'static str, args: Value) {
        self.bridge.transport().call(cmd, args).unwrap();
    }
    fn emit(&mut self) { self.call("probe_emit", json!({})); self.bridge.poll_state(); }
}
fn fixture_cost_payload(deal:u64,position:u64,volume:f64,gross:f64,commission:f64,swap:f64)->Value {
    json!({"schema":1,"deal_id":deal,"position_id":position,"volume":volume,"currency":"USD",
        "complete":true,"incomplete_reason":null,"cutoff_time_msc":TS+deal as i64,
        "history_fingerprint":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "history_query_complete":true,"first_entry_time_msc":TS-1000,
        "gross_profit":gross,"entry_commission_alloc":-7.0*volume,"exit_commission":commission,
        "entry_fee_alloc":-1.0*volume,"exit_fee":-0.125*volume,"swap":swap})
}

#[test]
fn cost_receipt_complete_projects_net_with_fees_and_never_charges_terminal_balance_again() {
    let mut f=Fixture::with_costs(true,0.08,Duration::from_secs(3),true);
    assert!(f.bridge.cost_net_supported());
    f.bridge.close_position(ID,CloseReason::BasketClose).unwrap();f.emit();
    let cash=f.bridge.account().balance;near(cash,997.11);
    let closed=f.bridge.drain_closed();assert_eq!(closed.len(),1);
    near(closed[0].profit,-2.89);near(closed[0].canonical_net().unwrap(),-2.89);
    near(closed[0].commission,-0.8);near(closed[0].swap,-6.);
    let receipt=closed[0].cost_receipt.as_ref().unwrap();
    assert_eq!(receipt.key.deal_id,9100);assert_eq!(receipt.position_identifier,ID);
    assert!(receipt.key.scope_id.contains("fixture-demo"));near(receipt.exit_fee.unwrap(),-0.01);
    f.call("probe_duplicate",json!({}));f.bridge.poll_state();assert!(f.bridge.drain_closed().is_empty());
    near(f.bridge.account().balance,cash);
}

#[test]
fn cost_receipt_three_partials_book_exact_closed_net_once_in_actual_engine() {
    let mut f=Fixture::with_costs(true,0.08,Duration::from_secs(3),true);
    let mut e=engine();e.cfg.closed_profit_net_costs=true;let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
    for cut in [0.02,0.02,0.04] {f.bridge.close_partial(ID,cut,CloseReason::Partial).unwrap();}
    assert!(e.wejscie_zablokowane(&f.bridge,TS));e.on_tick(&mut f.bridge,&q);near(e.baskets[0].realized,0.);
    f.emit();let cash=f.bridge.account().balance;e.on_tick(&mut f.bridge,&q);
    near(e.baskets[0].realized,-2.89);near(f.bridge.account().balance,cash);near(cash,997.11);
    f.call("probe_duplicate",json!({}));f.bridge.poll_state();e.on_tick(&mut f.bridge,&q);
    near(e.baskets[0].realized,-2.89);near(f.bridge.account().balance,cash);
}

#[test]
fn cost_receipt_missing_or_malformed_payload_is_quarantined_before_seen_or_engine_credit() {
    for value in [None,Some(Value::Null),Some(json!("not a receipt"))] {
        let mut f=Fixture::with_costs(true,0.08,Duration::from_secs(3),true);
        let mut e=engine();e.cfg.closed_profit_net_costs=true;let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
        f.bridge.close_position(ID,CloseReason::BasketClose).unwrap();
        match value {None=>f.call("probe_remove_field",json!({"field":"cost_receipt"})),
            Some(v)=>f.call("probe_corrupt",json!({"field":"cost_receipt","value":v}))};
        f.emit();assert!(f.bridge.drain_closed().is_empty());assert!(!f.bridge.cost_net_supported());
        assert!(f.bridge.close_receipts_pending());assert!(f.bridge.open_market(req()).is_err());
        e.on_tick(&mut f.bridge,&q);near(e.baskets[0].realized,0.);
    }
}

#[test]
fn cost_receipt_each_missing_component_is_not_certified_by_legacy_serde_zero() {
    for field in ["gross_profit","entry_commission_alloc","exit_commission","entry_fee_alloc","exit_fee","swap"] {
        let mut f=Fixture::with_costs(true,0.08,Duration::from_secs(3),true);
        f.bridge.close_position(ID,CloseReason::BasketClose).unwrap();
        let mut proof=fixture_cost_payload(9100,ID,0.08,4.,-0.24,-6.);proof[field]=Value::Null;
        f.call("probe_corrupt",json!({"field":"cost_receipt","value":proof}));f.emit();
        assert!(f.bridge.drain_closed().is_empty(),"{field}");assert!(!f.bridge.cost_net_supported(),"{field}");
    }
}

#[test]
fn cost_receipt_wrong_identity_currency_cutoff_or_truncated_genesis_cannot_book() {
    for (field,value) in [("deal_id",json!(9999)),("position_id",json!(9999)),("currency",json!("EUR")),
        ("cutoff_time_msc",json!(TS+9999)),("first_entry_time_msc",json!(TS)),
        ("history_query_complete",json!(false)),("history_fingerprint",json!("fake")),
        ("complete",json!(false)),("schema",json!(2))] {
        let mut f=Fixture::with_costs(true,0.08,Duration::from_secs(3),true);
        f.bridge.close_position(ID,CloseReason::BasketClose).unwrap();
        let mut proof=fixture_cost_payload(9100,ID,0.08,4.,-0.24,-6.);proof[field]=value;
        f.call("probe_corrupt",json!({"field":"cost_receipt","value":proof}));f.emit();
        assert!(f.bridge.drain_closed().is_empty(),"{field}");assert!(!f.bridge.cost_net_supported(),"{field}");
    }
}

#[test]
fn cost_receipt_known_manual_close_still_credits_owner() {
    let mut f=Fixture::with_costs(true,0.08,Duration::from_secs(3),true);
    let mut e=engine();e.cfg.closed_profit_net_costs=true;let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
    f.call("probe_sl",json!({"volume":0.08}));
    f.call("probe_corrupt",json!({"field":"magic","value":0}));
    f.call("probe_corrupt",json!({"field":"ev","value":"closed_foreign"}));
    f.emit();e.on_tick(&mut f.bridge,&q);near(e.baskets[0].realized,-2.89);
}

#[test]
fn cost_receipt_unknown_foreign_position_is_not_adopted_or_certified() {
    let mut f=Fixture::with_costs(true,0.08,Duration::from_secs(3),true);
    f.call("probe_foreign_close",json!({}));
    f.emit();assert!(f.bridge.drain_closed().is_empty());
    assert_eq!(f.bridge.foreign_closed().len(),1);
    assert!(f.bridge.close_receipt_issue().is_none());
}

#[test]
fn cost_runtime_hold_preserves_legacy_ledger_and_protection_without_reconnect() {
    for receipts in [false,true] {
        let mut f=Fixture::new(receipts,0.08);
        let generation=f.bridge.transport().execution_generation();
        let pending=f.bridge.place_pending(conduit_core::broker::PendingReq {kind:PendingKind::BuyLimit,
            price:3980.,volume:0.01,sl:None,tp:None,basket:Some(1),level:0,is_toucher:false,
            comment:"before hold".into(),is_topup:false}).unwrap();
        f.bridge.hold_new_entries("cost mode transition needs migration");
        assert!(f.bridge.close_receipts_pending());
        assert_eq!(f.bridge.receipt_barrier(),conduit_core::broker::ReceiptBarrier::RequiresReview);
        assert!(f.bridge.open_market(req()).is_err());
        assert!(f.bridge.place_pending(conduit_core::broker::PendingReq {kind:PendingKind::BuyLimit,
            price:3980.,volume:0.01,sl:None,tp:None,basket:Some(1),level:0,is_toucher:false,
            comment:"blocked".into(),is_topup:false}).is_err());
        f.bridge.modify_position(ID,Some(3990.),None).unwrap();
        f.bridge.cancel_pending(pending).unwrap();assert!(f.bridge.pendings().is_empty());
        f.bridge.close_position(ID,CloseReason::Manual).unwrap();f.emit();
        let c=f.bridge.drain_closed();assert_eq!(c.len(),1);near(c[0].profit,4.);
        assert!(c[0].cost_receipt.is_none());assert!(c[0].profit_basis.is_none());
        assert!(!f.bridge.transport().config().closed_profit_net_costs);
        assert_eq!(f.bridge.transport().execution_generation(),generation);
        assert!(f.bridge.transport().is_connected());
        assert!(f.bridge.close_receipts_pending(),"a completed close cannot clear the runtime latch");
    }
}

#[test]
fn cost_receipt_off_ignores_unexpected_cost_payload_and_keeps_original_profit() {
    let mut f=Fixture::new(true,0.08);f.bridge.close_position(ID,CloseReason::BasketClose).unwrap();
    f.call("probe_corrupt",json!({"field":"cost_receipt","value":"ignored while OFF"}));f.emit();
    let c=f.bridge.drain_closed();assert_eq!(c.len(),1);near(c[0].profit,4.);
    assert!(c[0].profit_basis.is_none());assert!(c[0].cost_receipt.is_none());
    f.call("probe_duplicate_cost",json!({"value":{"different":"still ignored while OFF"}}));
    f.bridge.poll_state();assert!(f.bridge.drain_closed().is_empty());
    assert!(f.bridge.close_receipt_issue().is_none(),"OFF metadata must not alter the economic duplicate fingerprint");
}

#[test]
fn cost_consumer_fault_latches_actual_broker_entry_halt_but_leaves_protection_available() {
    let mut f=Fixture::with_costs(true,0.08,Duration::from_secs(3),true);
    f.bridge.report_cost_consumer_fault("fixture receipt mismatch");assert!(!f.bridge.cost_net_supported());
    assert!(f.bridge.open_market(req()).is_err());
    f.bridge.modify_position(ID,Some(3990.),None).unwrap();
    f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();f.emit();
    assert!(!f.bridge.cost_net_supported(),"a later valid receipt cannot erase earlier fault");
}

#[test]
fn cost_on_without_receipt_identity_is_rejected_before_any_transport_start() {
    let err=Mt5Bridge::connect(SidecarConfig{autostart:false,closed_profit_net_costs:true,
        close_receipt_reconcile:false,..Default::default()}).err().unwrap();
    assert!(err.to_string().contains("requires close_receipt_reconcile"));
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.bridge.transport().call("probe_finish", json!({}));
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.worker.take() { h.join().unwrap(); }
    }
}
fn near(a:f64,b:f64) { assert!((a-b).abs()<1e-8, "{a} != {b}"); }
fn req() -> OrderReq { OrderReq { side:Side::Buy, volume:0.01, sl:None, tp:None,
    basket:Some(1), level:0, is_toucher:false, comment:"fixture".into() } }
fn engine() -> Engine {
    let mut cfg = Settings::default();
    cfg.basket_realized_broker_only = true; cfg.confirmed_exit_retry = true;
    cfg.server_tz_offset_ms = 0; cfg.exec_latency_ms = 0; cfg.rearm_grid_on_return = false;
    let mut e = Engine::new(cfg, 1000.);
    let basket:Basket = serde_json::from_value(json!({"id":1,"source":SourceKey::new(1,None),
        "source_name":"fixture","msg_id":1,"side":"Buy","is_limit":false,
        "entry_lo":4000.,"entry_hi":4000.,"zone_lo":4000.,"zone_hi":4000.,"sl":null,
        "tps":[],"tp_stage":0,"created_ts":TS-1000,"state":"Working","tickets":[ID],
        "pendings":[],"realized":0.,"events":[],"had_positions":true})).unwrap();
    e.adopt_baskets(vec![basket]); e
}

fn rearm_engine(bridge:&mut Mt5Bridge) -> Engine {
    let mut e=engine(); let q=bridge.quote(); e.on_tick(bridge,&q);
    e.cfg.rearm_grid_on_return=true;
    e.cfg.rearm_min_gap_min=0.; e.cfg.rearm_min_basket_profit=0.;
    e.cfg.rearm_block_after_secured=false; e.cfg.spp_blocks_rearm_when_flat=false;
    e.cfg.auto_limit=true;
    let bk=&mut e.baskets[0];
    bk.zone_lo=3999.; bk.zone_hi=4001.; bk.entry_lo=3999.; bk.entry_hi=4001.;
    bk.sl=Some(3990.); bk.state=BasketState::RiskFree; bk.secured=true;
    bk.realized=3.80;
    bk.levels=vec![GridLevel{price:4000.,base_units:1,volume:0.01,
        sl:Some(3990.),tp:Some(4100.),level:0,is_toucher:false,
        fill_ts:TS-1000,fill_px:4000.,cancelled:false,filled:true}];
    assert!(!e.wejscie_zablokowane(bridge,TS),"fixture must be eligible before close");
    e
}

#[test]
fn riskfree_negative_close_ack_must_block_rearm_before_delayed_deal() {
    let mut f=Fixture::new(true,0.08); let mut e=rearm_engine(&mut f.bridge);
    f.call("probe_profit_per_lot",json!({"value":-201.}));
    f.bridge.close_position(ID,CloseReason::RiskFree).unwrap();
    let q=f.bridge.quote(); e.on_tick(&mut f.bridge,&q);
    assert_eq!(e.baskets[0].rearms,0,"ACK is not realized profit; +3.80 omits the -16.08 close");
    assert!(f.bridge.pendings().is_empty());
    assert!(e.wejscie_zablokowane(&f.bridge,TS));
    f.emit();
    assert!(e.wejscie_zablokowane(&f.bridge,TS),"decoded receipt is not yet engine credit");
    e.on_tick(&mut f.bridge,&q);
    near(e.baskets[0].realized,-12.28);
    assert!(!e.wejscie_zablokowane(&f.bridge,TS));
    assert_eq!(e.baskets[0].rearms,0); assert!(f.bridge.pendings().is_empty());
}

#[test]
fn positive_close_allows_rearm_only_after_engine_consumes_receipt() {
    let mut f=Fixture::new(true,0.01); let mut e=rearm_engine(&mut f.bridge);
    f.bridge.close_position(ID,CloseReason::RiskFree).unwrap();
    let q=f.bridge.quote(); e.on_tick(&mut f.bridge,&q);
    assert_eq!(e.baskets[0].rearms,0); assert!(f.bridge.close_receipts_pending());
    f.emit(); assert!(f.bridge.close_receipts_pending());
    assert!(f.bridge.open_market(req()).is_err());
    e.on_tick(&mut f.bridge,&q);
    near(e.baskets[0].realized,4.3);
    assert!(!f.bridge.close_receipts_pending());
    assert_eq!(e.baskets[0].rearms,1); assert_eq!(f.bridge.pendings().len(),1);
}

#[test]
fn two_slots_first_drain_does_not_release_shared_receipt_barrier() {
    use conduit_core::routing::{Widok,Wlasnosc};
    let mut f=Fixture::new(true,0.01); f.call("probe_add_second_slot",json!({}));
    f.bridge.refresh_state().unwrap(); assert_eq!(f.bridge.positions().len(),2);
    let mut e0=engine(); let mut e1=engine();
    e1.baskets[0].id=100001; e1.baskets[0].tickets=vec![ID+1];
    let owner=|slot|Wlasnosc{slot,zapasowy:slot==0,znane_sloty:vec![0,1]};
    let mut waiting=Vec::new(); let q=f.bridge.quote();
    for (slot,e) in [(0,&mut e0),(1,&mut e1)] {
        let mut view=Widok::nowy(&mut f.bridge,owner(slot),&mut waiting); e.on_tick(&mut view,&q);
    }
    f.bridge.close_position(ID,CloseReason::RiskFree).unwrap();
    f.call("probe_close_second_slot",json!({})); f.emit();
    {
        let mut view=Widok::nowy(&mut f.bridge,owner(0),&mut waiting);
        e0.on_tick(&mut view,&q); near(e0.baskets[0].realized,0.5);
        assert!(view.close_receipts_pending()); assert!(e0.wejscie_zablokowane(&view,TS));
        assert!(view.open_market(req()).is_err(),"inner was drained, but another owner has not booked its receipt");
    }
    assert_eq!(waiting.len(),1);
    {
        let mut view=Widok::nowy(&mut f.bridge,owner(1),&mut waiting);
        assert!(view.close_receipts_pending()); e1.on_tick(&mut view,&q);
        near(e1.baskets[0].realized,2.); assert!(!view.close_receipts_pending());
    }
    assert!(waiting.is_empty()); assert!(!f.bridge.close_receipts_pending());
}

#[test]
fn receipt_wait_does_not_disable_protective_modify_cancel_or_further_close() {
    use conduit_core::broker::PendingReq;
    let mut f=Fixture::new(true,0.08);
    let pending=f.bridge.place_pending(PendingReq{kind:PendingKind::BuyLimit,volume:0.01,
        price:3999.,sl:Some(3990.),tp:Some(4100.),basket:Some(1),level:1,
        is_toucher:false,is_topup:false,comment:"fixture".into()}).unwrap();
    f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();
    assert!(f.bridge.close_receipts_pending());
    f.bridge.modify_position(ID,Some(3998.),Some(4100.)).unwrap();
    f.bridge.modify_pending(pending,3998.,Some(3990.),Some(4100.)).unwrap();
    f.bridge.cancel_pending(pending).unwrap();
    f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();
    f.bridge.close_position(ID,CloseReason::Manual).unwrap();
    f.emit(); assert_eq!(f.bridge.drain_closed().len(),3);
    assert!(!f.bridge.close_receipts_pending());
}

#[test]
fn three_partial_delivery_snapshot_replay_permutations_are_once_and_barriered() {
    // Exhaustive small schedule: each ACK can be followed by a snapshot, and
    // each of the first two events can be delivered immediately or batched.
    for snapshot_mask in 0..8 {
        for delivery_mask in 0..4 {
            let mut f=Fixture::new(true,0.08); let mut total=0.; let mut count=0;
            for n in 0..3 {
                f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();
                assert!(f.bridge.close_receipts_pending());
                if snapshot_mask & (1<<n)!=0 { f.bridge.refresh_state().unwrap(); }
                assert!(f.bridge.close_receipts_pending());
                if n==2 || delivery_mask & (1<<n)!=0 {
                    f.emit(); assert!(f.bridge.close_receipts_pending());
                    let cs=f.bridge.drain_closed(); total+=cs.iter().map(|c|c.profit).sum::<f64>(); count+=cs.len();
                    assert!(cs.iter().all(|c|c.basket==Some(1)&&c.reason==CloseReason::Partial));
                    assert!(!f.bridge.close_receipts_pending());
                    f.call("probe_duplicate",json!({})); f.bridge.poll_state();
                    assert!(f.bridge.drain_closed().is_empty()); assert!(!f.bridge.close_receipts_pending());
                }
            }
            near(total,3.); assert_eq!(count,3); near(f.bridge.positions()[0].volume,0.02);
        }
    }
}

#[test]
fn off_close_ack_does_not_enable_receipt_barrier_or_change_legacy_rearm() {
    let mut f=Fixture::new(false,0.08); let mut e=rearm_engine(&mut f.bridge);
    f.call("probe_profit_per_lot",json!({"value":-201.}));
    f.bridge.close_position(ID,CloseReason::RiskFree).unwrap();
    assert!(!f.bridge.close_receipt_reconciliation_active()); assert!(!f.bridge.close_receipts_pending());
    let q=f.bridge.quote(); e.on_tick(&mut f.bridge,&q);
    assert_eq!(e.baskets[0].rearms,1); near(e.baskets[0].realized,3.80);
}

#[test]
fn netting_reversal_is_detected_instead_of_retaining_old_buy_metadata() {
    for new_ticket in [false,true] {
        let mut f=Fixture::new(true,0.08);
        f.call("probe_position_kind",json!({"value":1}));
        if new_ticket { f.call("probe_ticket",json!({"ticket":9901})); }
        f.bridge.refresh_state().unwrap();
        assert!(f.bridge.close_receipt_issue().is_some(),"same stable identifier changed side");
        assert!(!f.bridge.positions().iter().any(|p|p.side==Side::Buy),"must not manage a reversed SELL using old BUY metadata");
        assert_eq!(f.bridge.quarantined_positions().count(),1);
        assert_eq!(f.bridge.foreign_positions()[0].side,Side::Sell);
        let account=f.bridge.account(); near(account.balance,1000.); near(account.equity,1000.);
        f.call("probe_position_kind",json!({"value":0})); f.bridge.refresh_state().unwrap();
        assert!(f.bridge.positions().is_empty(),"stale old BUY must not unlatch quarantine");
        assert!(f.bridge.close_receipts_pending());
        f.bridge.reconcile().unwrap(); assert!(f.bridge.positions().is_empty());
    }
}

#[test]
fn unknown_position_kind_never_silently_becomes_sell_and_off_stays_legacy() {
    for on in [false,true] {
        let mut f=Fixture::new(on,0.08);
        f.call("probe_position_kind",json!({"value":99})); f.bridge.refresh_state().unwrap();
        if on {
            assert!(f.bridge.positions().is_empty()); assert!(f.bridge.foreign_positions().is_empty());
            assert_eq!(f.bridge.quarantined_positions().next().unwrap().kind,99);
            assert!(f.bridge.close_receipts_pending());
            f.bridge.reconcile().unwrap(); assert!(f.bridge.positions().is_empty());
        } else {
            assert_eq!(f.bridge.positions()[0].side,Side::Buy); // exact old merge behavior
            assert!(!f.bridge.close_receipts_pending());
        }
    }
}

#[test]
fn close_timeout_actual_execution_and_empty_snapshot_cannot_unlatch_uncertainty() {
    let mut f=Fixture::with_timeout(true,0.08,Duration::from_millis(150));
    f.call("probe_drop_next_response",json!({}));
    assert!(f.bridge.close_position(ID,CloseReason::RiskFree).is_err());
    assert!(f.bridge.close_receipts_pending());
    f.bridge.refresh_state().unwrap(); assert!(f.bridge.positions().is_empty());
    assert!(f.bridge.close_receipts_pending(),"snapshot is not proof of exact economic result");
    f.emit(); assert_eq!(f.bridge.drain_closed().len(),1);
    assert!(f.bridge.close_receipts_pending(),"stage A does not invent durable unknown-RPC recovery");
}

#[test]
fn open_timeout_actual_fill_is_not_retried_and_exact_snapshot_clears_only_the_hold() {
    let mut f=Fixture::with_timeout(true,0.08,Duration::from_millis(150));
    f.call("probe_valid_open",json!({})); f.call("probe_drop_next_response",json!({}));
    assert!(f.bridge.open_market(req()).is_err()); assert!(f.bridge.close_receipts_pending());
    assert!(f.bridge.open_market(req()).is_err()); // blocked locally, no duplicate RPC
    f.bridge.refresh_state().unwrap(); assert_eq!(f.bridge.positions().len(),2);
    assert!(!f.bridge.close_receipts_pending());
    f.bridge.modify_position(ID,Some(3999.),None).unwrap();
}

#[test]
fn explicit_broker_rejection_is_not_ambiguous_execution_or_permanent_halt() {
    let mut f=Fixture::new(true,0.08); f.call("probe_reject_next_trade",json!({}));
    assert!(f.bridge.close_position(ID,CloseReason::RiskFree).is_err());
    assert!(!f.bridge.close_receipts_pending()); near(f.bridge.positions()[0].volume,0.08);
    f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap(); f.emit();
    assert_eq!(f.bridge.drain_closed().len(),1); assert!(!f.bridge.close_receipts_pending());
}

#[test]
fn close_ack_missing_identifier_waits_for_exact_history_and_then_unblocks_entries() {
    let mut f=Fixture::new(true,0.08);
    f.call("probe_next_close_ack_patch",json!({"position_identifier":0}));
    f.bridge.close_position(ID,CloseReason::RiskFree).unwrap();
    assert_eq!(f.bridge.receipt_barrier(),conduit_core::broker::ReceiptBarrier::Temporary);
    assert!(f.bridge.open_market(req()).is_err(),"new risk waits for the exact closed deal");
    f.emit();
    let closed=f.bridge.drain_closed();
    assert_eq!(closed.len(),1);near(closed[0].profit,4.0);
    assert!(!f.bridge.close_receipts_pending(),"exact history receipt must release the live gate");
}

#[test]
fn close_ack_conflicting_nonzero_identifier_remains_a_review_fault() {
    let mut f=Fixture::new(true,0.08);
    f.call("probe_next_close_ack_patch",json!({"position_identifier":ID+999}));
    f.bridge.close_position(ID,CloseReason::RiskFree).unwrap();
    assert_eq!(f.bridge.receipt_barrier(),conduit_core::broker::ReceiptBarrier::RequiresReview);
    f.emit();
    assert!(f.bridge.close_receipts_pending(),"later history cannot erase contradictory identity");
}

#[test]
fn current_receipt_barrier_rejects_valid_entry_without_replay() {
    use conduit_core::IncomingMessage;
    let message=IncomingMessage{ts:TS,source:SourceKey::new(1,None),source_name:"fixture".into(),
        msg_id:55,reply_to:None,edit_of:None,
        text:"BUY LIMITS GOLD @ 3999/3997 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3990".into()};
    // Control proves the same parsed entry is valid and executable without the barrier.
    {
        let mut f=Fixture::new(true,0.08); let mut e=engine(); let q=f.bridge.quote();
        e.on_tick(&mut f.bridge,&q); e.on_message(&mut f.bridge,&message);
        assert!(e.baskets.iter().any(|bk|bk.msg_id==55)); assert!(!f.bridge.pendings().is_empty());
    }
    let mut f=Fixture::new(true,0.08); let mut e=engine(); let q=f.bridge.quote();
    e.on_tick(&mut f.bridge,&q); f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();
    e.on_message(&mut f.bridge,&message);
    assert!(!e.baskets.iter().any(|bk|bk.msg_id==55));
    assert!(e.logs.iter().any(|l|l.text.contains("wejścia zablokowane")));
    f.emit(); e.on_tick(&mut f.bridge,&q);
    assert!(!f.bridge.close_receipts_pending());
    assert!(!e.baskets.iter().any(|bk|bk.msg_id==55),"legacy has no deferred intent to replay");
    assert!(f.bridge.pendings().is_empty());
}

fn deferred_message(id:i64) -> conduit_core::IncomingMessage {
    conduit_core::IncomingMessage { ts:TS,source:SourceKey::new(1,None),source_name:"fixture".into(),
        msg_id:id,reply_to:None,edit_of:None,
        text:"BUY LIMITS GOLD @ 3999/3997 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3990".into() }
}
fn deferred_engine(f:&mut Fixture)->Engine {
    let mut e=engine(); e.cfg.defer_entry_until_receipts=true;
    let q=f.bridge.quote(); e.on_tick_received(&mut f.bridge,&q,TS);
    f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap(); e
}
fn deferred_state(e:&Engine,m:&conduit_core::IncomingMessage)->conduit_core::engine::DeferredEntryState {
    e.deferred_entry_status(&m.source,m.edit_of.unwrap_or(m.msg_id)).unwrap().state
}
fn deferred_tick(e:&mut Engine,f:&mut Fixture,quote_ts:i64,utc:i64,price:f64) {
    let q=Quote::new(quote_ts,price,price+0.2).unwrap(); f.bridge.set_replay_quote(q);
    e.on_tick_received(&mut f.bridge,&q,utc);
}

#[test]
fn deferred_entry_books_receipt_then_runs_full_pipeline_once_and_edits_do_not_reopen() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08); let mut e=deferred_engine(&mut f); let m=deferred_message(55);
    e.on_message_received(&mut f.bridge,&m,TS);
    assert_eq!(deferred_state(&e,&m),Waiting); assert_eq!(e.baskets.len(),1);
    // Same NEW and identical EDIT remain a single intention; neither is done_actions.
    e.on_message_received(&mut f.bridge,&m,TS+1);
    let mut edit=m.clone(); edit.edit_of=Some(55);
    e.on_message_received(&mut f.bridge,&edit,TS+2);
    let messages_before=e.stats.messages;
    let signals_before=e.stats.signals;
    f.emit(); assert!(f.bridge.close_receipts_pending());
    deferred_tick(&mut e,&mut f,TS+10,TS+10,4000.5);
    assert_eq!(deferred_state(&e,&m),Executed); near(e.baskets[0].realized,1.0);
    assert_eq!(e.baskets.iter().filter(|b|b.msg_id==55).count(),1);
    assert_eq!(e.stats.messages,messages_before,"internal dispatch is not a second Telegram delivery");
    assert_eq!(e.stats.signals,signals_before);
    e.on_message_received(&mut f.bridge,&m,TS+11);
    e.on_message_received(&mut f.bridge,&edit,TS+12);
    assert_eq!(e.baskets.iter().filter(|b|b.msg_id==55).count(),1);
}

#[test]
fn deferred_latest_edit_replaces_payload_but_not_first_receipt_or_source_owner() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08); let mut e=deferred_engine(&mut f); let m=deferred_message(55);
    e.on_message_received(&mut f.bridge,&m,TS);
    let mut edit=m.clone(); edit.edit_of=Some(55); edit.text=edit.text.replace("3999/3997","3998/3996");
    e.on_message_received(&mut f.bridge,&edit,TS+20);
    e.on_message_received(&mut f.bridge,&m,TS+30); // old NEW is not latest edit
    let mut stale=m.clone(); stale.edit_of=Some(55);
    e.on_message_received(&mut f.bridge,&stale,TS+10);
    let mut other=m.clone(); other.source=SourceKey::new(2,Some(9));
    e.on_message_received(&mut f.bridge,&other,TS+21);
    assert_eq!(e.deferred_entry_status(&m.source,55).unwrap().first_received_utc,TS);
    f.emit(); deferred_tick(&mut e,&mut f,TS+40,TS+40,4000.5);
    assert_eq!(deferred_state(&e,&m),Executed); assert_eq!(deferred_state(&e,&other),Waiting);
    let b=e.baskets.iter().find(|b|b.msg_id==55&&b.source==m.source).unwrap(); near(b.entry_lo,3996.);
    deferred_tick(&mut e,&mut f,TS+50,TS+50,4000.5);
    assert_eq!(deferred_state(&e,&other),Executed);
}

#[test]
fn deferred_entry_info_entry_edits_and_info_entry_conversion_are_explicit() {
    use conduit_core::engine::DeferredEntryState::*;
    for initially_info in [false,true] {
        let mut f=Fixture::new(true,0.08); let mut e=deferred_engine(&mut f); let mut m=deferred_message(55);
        if initially_info {m.text="PREPARE FOR BUY LIMITS".into();}
        e.on_message_received(&mut f.bridge,&m,TS);
        let mut info=m.clone();info.edit_of=Some(55);info.text="Still waiting".into();
        e.on_message_received(&mut f.bridge,&info,TS+1);
        assert_eq!(deferred_state(&e,&m),NoEntry);
        let mut valid=deferred_message(55);valid.edit_of=Some(55);
        e.on_message_received(&mut f.bridge,&valid,TS+2);
        assert_eq!(deferred_state(&e,&m),Waiting);
        f.emit();deferred_tick(&mut e,&mut f,TS+10,TS+10,4000.5);
        assert_eq!(deferred_state(&e,&m),Executed);
    }
}

#[test]
fn deferred_unrelated_info_never_creates_an_entry_intention_or_poison_record() {
    let mut f=Fixture::new(true,0.08); let mut e=deferred_engine(&mut f);
    let mut info=deferred_message(55); info.text="UNSTRUCTURED COMMUNITY ANNOUNCEMENT".into();
    e.on_message_received(&mut f.bridge,&info,TS);
    assert!(e.deferred_entry_status(&info.source,info.msg_id).is_none());
}

#[test]
fn deferred_requires_review_reaches_terminal_expired_state_at_the_same_utc_ttl() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08); let mut e=deferred_engine(&mut f); let m=deferred_message(55);
    e.cfg.deferred_entry_max_age_s=2.;
    e.on_message_received(&mut f.bridge,&m,TS);
    f.call("probe_malformed_next_response",json!({}));
    assert!(f.bridge.close_partial(ID,0.01,CloseReason::Partial).is_err());
    deferred_tick(&mut e,&mut f,TS+1,TS+1,4000.5);
    assert_eq!(deferred_state(&e,&m),RequiresReview);
    deferred_tick(&mut e,&mut f,TS+2000,TS+2000,4000.5);
    assert_eq!(deferred_state(&e,&m),Expired);
    f.emit(); deferred_tick(&mut e,&mut f,TS+3000,TS+3000,4000.5);
    assert_eq!(deferred_state(&e,&m),Expired,"expired intention can never revive");
    assert!(!e.baskets.iter().any(|b|b.msg_id==55));
}

#[test]
fn deferred_full_market_entry_is_replayed_to_normal_strategy_axes_not_rejected_by_receipts() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08); let mut e=deferred_engine(&mut f); let mut m=deferred_message(55);
    m.text="BUY GOLD @ 4000/3997 AREA\nTP 4010\nTP 4020\nTP 4030\nSL 3990".into();
    e.on_message_received(&mut f.bridge,&m,TS);
    assert_eq!(deferred_state(&e,&m),Waiting,
        "receipt lifecycle must not silently impose an extra LIMIT-only strategy");
    f.emit();
    deferred_tick(&mut e,&mut f,TS+10,TS+10,4000.5);
    assert_eq!(deferred_state(&e,&m),Executed);
    assert_eq!(e.baskets.iter().filter(|b|b.msg_id==55).count(),1);
    assert!(!e.logs.iter().any(|line|line.text.contains("OnlyLimitEntrySupported")));
}

#[test]
fn deferred_entry_arriving_during_unknown_open_is_released_once_after_exact_snapshot() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::with_timeout(true,0.08,Duration::from_millis(150));
    let mut e=engine(); e.cfg.defer_entry_until_receipts=true;
    let q=f.bridge.quote(); e.on_tick_received(&mut f.bridge,&q,TS);

    // Live reproduction: transport loses the OPEN response, while MT5 did in
    // fact open exactly the requested position.  A subsequent Telegram ENTRY
    // arrives before the authoritative snapshot has reconciled that outcome.
    f.call("probe_valid_open",json!({}));
    f.call("probe_drop_next_response",json!({}));
    assert!(f.bridge.open_market(req()).is_err());
    assert_eq!(f.bridge.receipt_barrier(),conduit_core::broker::ReceiptBarrier::RequiresReview);

    let m=deferred_message(55);
    e.on_message_received(&mut f.bridge,&m,TS+1);
    assert_eq!(deferred_state(&e,&m),RequiresReview);
    assert!(!e.baskets.iter().any(|b|b.msg_id==55));

    f.bridge.refresh_state().unwrap();
    assert_eq!(f.bridge.positions().len(),2);
    assert_eq!(f.bridge.receipt_barrier(),conduit_core::broker::ReceiptBarrier::Clear);
    deferred_tick(&mut e,&mut f,TS+10,TS+10,4000.5);
    assert_eq!(deferred_state(&e,&m),Executed);
    assert_eq!(e.baskets.iter().filter(|b|b.msg_id==55).count(),1);
    deferred_tick(&mut e,&mut f,TS+20,TS+20,4000.6);
    assert_eq!(e.baskets.iter().filter(|b|b.msg_id==55).count(),1,
        "exact snapshot reconciliation must release, never retry, the Telegram intention");
}

#[test]
fn deferred_cancel_and_management_reply_never_route_to_older_basket() {
    use conduit_core::engine::DeferredEntryState::*;
    for text in ["CANCEL", "RISK FREE @4000", "SECURING PARTIAL PROFITS", "TP1 HIT"] {
        let mut f=Fixture::new(true,0.08); let mut e=deferred_engine(&mut f);let m=deferred_message(55);
        e.on_message_received(&mut f.bridge,&m,TS);
        let before=(e.baskets[0].state.clone(),e.baskets[0].tp_stage,e.baskets[0].secured,
            f.bridge.positions()[0].sl,f.bridge.positions()[0].volume);
        let mut reply=m.clone();reply.msg_id=56;reply.reply_to=Some(55);reply.text=text.into();
        e.on_message_received(&mut f.bridge,&reply,TS+1);
        assert_eq!(deferred_state(&e,&m),if text=="CANCEL" {Cancelled}else{RequiresReview},"{text}");
        assert_eq!(before,(e.baskets[0].state.clone(),e.baskets[0].tp_stage,e.baskets[0].secured,
            f.bridge.positions()[0].sl,f.bridge.positions()[0].volume));
        let mut edit=m.clone();edit.edit_of=Some(55);e.on_message_received(&mut f.bridge,&edit,TS+2);
        f.emit();deferred_tick(&mut e,&mut f,TS+10,TS+10,4000.5);
        assert!(!e.baskets.iter().any(|b|b.msg_id==55),"terminal intent cannot revive: {text}");
    }
}

#[test]
fn deferred_receipt_age_is_utc_not_broker_or_publication_and_edits_do_not_extend_it() {
    use conduit_core::engine::DeferredEntryState::*;
    for offset in [-10_800_000,0,10_800_000] {
        let mut f=Fixture::new(true,0.08); let mut e=deferred_engine(&mut f);
        e.cfg.deferred_entry_max_age_s=2.;
        let mut m=deferred_message(55);m.ts=TS-999_000_000; // publication irrelevant
        e.on_message_received(&mut f.bridge,&m,TS);
        let mut edit=m.clone();edit.edit_of=Some(55);
        e.on_message_received(&mut f.bridge,&edit,TS+1999);
        assert_eq!(deferred_state(&e,&m),Waiting);
        deferred_tick(&mut e,&mut f,TS+offset,TS+2000,4000.5);
        assert_eq!(deferred_state(&e,&m),Expired,"offset {offset}");
    }
}

#[test]
fn deferred_stale_quote_clock_regression_missing_clock_and_invalid_age_fail_closed() {
    use conduit_core::engine::DeferredEntryState::*;
    {
        let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);let m=deferred_message(55);
        e.cfg.deferred_entry_max_age_s=2.;e.on_message_received(&mut f.bridge,&m,TS);
        let mut edit=m.clone();edit.edit_of=Some(55);e.on_message_received(&mut f.bridge,&edit,TS+1900);
        e.on_message_received(&mut f.bridge,&edit,TS+500);
        f.emit();deferred_tick(&mut e,&mut f,TS,TS+1999,4000.5); // receipt clear, quote stale
        assert_eq!(deferred_state(&e,&m),Waiting);
        deferred_tick(&mut e,&mut f,TS,TS+2000,4000.5);assert_eq!(deferred_state(&e,&m),Expired);
    }
    for missing_clock in [true,false] {
        let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);let m=deferred_message(55);
        if missing_clock {e.on_message(&mut f.bridge,&m);} else {e.cfg.deferred_entry_max_age_s=0.;e.on_message_received(&mut f.bridge,&m,TS);}
        assert_eq!(deferred_state(&e,&m),RequiresReview);
    }
}

#[test]
fn deferred_same_account_new_transport_generation_cannot_release_old_ram_intent() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut e;
    let old_session;
    let m=deferred_message(55);
    {
        let mut f=Fixture::new(true,0.08);e=deferred_engine(&mut f);
        old_session=f.bridge.execution_session().unwrap();e.on_message_received(&mut f.bridge,&m,TS);
    }
    let mut f=Fixture::new(true,0.08);
    let new_session=f.bridge.execution_session().unwrap();assert_eq!(old_session.scope,new_session.scope);
    assert_ne!(old_session.generation,new_session.generation);
    deferred_tick(&mut e,&mut f,TS+10,TS+10,4000.5);
    assert_eq!(deferred_state(&e,&m),RequiresReview);assert!(!e.baskets.iter().any(|b|b.msg_id==55));
}

#[test]
fn deferred_temporary_to_permanent_fault_never_auto_resumes_or_retries() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);let m=deferred_message(55);
    e.on_message_received(&mut f.bridge,&m,TS);
    f.call("probe_malformed_next_response",json!({}));
    assert!(f.bridge.close_partial(ID,0.01,CloseReason::Partial).is_err());
    deferred_tick(&mut e,&mut f,TS+1,TS+1,4000.5);assert_eq!(deferred_state(&e,&m),RequiresReview);
    f.emit();deferred_tick(&mut e,&mut f,TS+2,TS+2,4000.5);
    assert_eq!(deferred_state(&e,&m),RequiresReview);assert!(f.bridge.pendings().is_empty());
}

#[test]
fn deferred_observed_tp_sl_market_now_and_current_price_are_revalidated() {
    use conduit_core::engine::DeferredEntryState::*;
    for (price,want) in [(4011.,RequiresReview),(3989.,Cancelled)] {
        let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);let m=deferred_message(55);
        e.on_message_received(&mut f.bridge,&m,TS);
        deferred_tick(&mut e,&mut f,TS+1,TS+1,price);assert_eq!(deferred_state(&e,&m),want);
        f.emit();deferred_tick(&mut e,&mut f,TS+2,TS+2,4000.5);
        assert_eq!(deferred_state(&e,&m),want);assert!(!e.baskets.iter().any(|b|b.msg_id==55));
    }
    let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);let mut m=deferred_message(55);
    m.text="BUY NOW".into();e.on_message_received(&mut f.bridge,&m,TS);
    assert_eq!(deferred_state(&e,&m),RequiresReview);
}

#[test]
fn deferred_unknown_execution_during_release_is_single_attempt_not_automatic_retry() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);let m=deferred_message(55);
    e.on_message_received(&mut f.bridge,&m,TS);f.emit();
    f.call("probe_malformed_next_response",json!({}));
    deferred_tick(&mut e,&mut f,TS+1,TS+1,4000.5);
    assert_eq!(deferred_state(&e,&m),RequiresReview);
    assert_eq!(f.bridge.unknown_sends,1);
    deferred_tick(&mut e,&mut f,TS+2,TS+2,4000.5);
    assert_eq!(f.bridge.unknown_sends,1);
}

#[test]
fn deferred_disabled_axis_with_pending_intent_does_not_resume_when_reenabled() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);let m=deferred_message(55);
    e.on_message_received(&mut f.bridge,&m,TS);e.cfg.defer_entry_until_receipts=false;
    deferred_tick(&mut e,&mut f,TS+1,TS+1,4000.5);
    assert_eq!(deferred_state(&e,&m),RequiresReview);
    e.cfg.defer_entry_until_receipts=true;f.emit();
    deferred_tick(&mut e,&mut f,TS+2,TS+2,4000.5);
    assert_eq!(deferred_state(&e,&m),RequiresReview);
}

#[test]
fn deferred_capacity_is_bounded_and_does_not_evict_an_existing_intent() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);
    for id in 100..228 {let m=deferred_message(id);e.on_message_received(&mut f.bridge,&m,TS);}
    let overflow=deferred_message(228);e.on_message_received(&mut f.bridge,&overflow,TS);
    assert!(e.deferred_entry_status(&overflow.source,228).is_none());
    assert_eq!(e.deferred_entry_status(&overflow.source,100).unwrap().state,Waiting);
    assert_eq!(e.baskets.len(),1);assert!(f.bridge.pendings().is_empty());
}

#[test]
fn deferred_unthreaded_management_is_ambiguous_not_older_basket_management() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);let m=deferred_message(55);
    e.on_message_received(&mut f.bridge,&m,TS);
    let mut rf=m.clone();rf.msg_id=56;rf.text="RISK FREE @4000".into();
    let before=f.bridge.positions()[0].volume;
    e.on_message_received(&mut f.bridge,&rf,TS+1);
    assert_eq!(deferred_state(&e,&m),RequiresReview);near(f.bridge.positions()[0].volume,before);
    assert!(!e.baskets[0].secured);
}

#[test]
fn deferred_global_close_all_still_protects_live_positions_and_cancels_queue() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08);let mut e=deferred_engine(&mut f);let m=deferred_message(55);
    e.cfg.honor_close_all=true;e.cfg.close_all_scope=conduit_core::settings::CloseAllScope::Global;
    e.on_message_received(&mut f.bridge,&m,TS);
    let mut close=m.clone();close.msg_id=56;close.text="CLOSE ALL".into();close.reply_to=Some(55);
    e.on_message_received(&mut f.bridge,&close,TS+1);
    assert_eq!(deferred_state(&e,&m),Cancelled);assert!(f.bridge.positions().is_empty());
}

#[test]
fn deferred_opposite_close_creates_barrier_after_dispatch_without_losing_new_entry() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08);let mut e=engine();
    e.cfg.defer_entry_until_receipts=true;e.cfg.exit_on_opposite_signal=true;
    let q=f.bridge.quote();e.on_tick_received(&mut f.bridge,&q,TS);
    let mut m=deferred_message(55);
    m.text="SELL LIMITS GOLD @ 4002/4004 AREA\nTP 3990\nTP 3980\nTP 3970\nSL 4010".into();
    assert!(!f.bridge.close_receipts_pending());
    e.on_message_received(&mut f.bridge,&m,TS);
    assert!(f.bridge.positions().is_empty(),"protective opposite close must execute immediately");
    assert!(f.bridge.close_receipts_pending());
    assert_eq!(deferred_state(&e,&m),Waiting,"entry must be captured AFTER the protective close creates its barrier");
    assert_eq!(e.deferred_entry_status(&m.source,55).unwrap().first_received_utc,TS);
    // A later, independently-owned BUY must not be closed by replaying the
    // OLD entry's already-executed protective prefix.
    f.call("probe_add_second_slot",json!({}));f.bridge.refresh_state().unwrap();
    let mut later=e.baskets[0].clone();later.id=100001;later.msg_id=44;
    later.state=BasketState::Working;later.pending_exit=None;later.tickets=vec![ID+1];
    later.realized=0.;e.baskets.push(later);
    f.emit();deferred_tick(&mut e,&mut f,TS+10,TS+10,4000.5);
    assert_eq!(deferred_state(&e,&m),Executed);
    assert_eq!(e.baskets.iter().filter(|b|b.msg_id==55).count(),1);
    assert!(f.bridge.positions().iter().any(|p|p.ticket==ID+1),"protective prefix must not replay against later positions");
}

#[test]
fn deferred_opposite_unknown_close_is_review_not_temporary_wait() {
    use conduit_core::engine::DeferredEntryState::*;
    let mut f=Fixture::new(true,0.08);let mut e=engine();e.cfg.defer_entry_until_receipts=true;
    e.cfg.exit_on_opposite_signal=true;let q=f.bridge.quote();e.on_tick_received(&mut f.bridge,&q,TS);
    f.call("probe_malformed_next_response",json!({}));
    let mut m=deferred_message(55);m.text="SELL LIMITS GOLD @ 4002/4004 AREA\nTP 3990\nTP 3980\nTP 3970\nSL 4010".into();
    e.on_message_received(&mut f.bridge,&m,TS);assert_eq!(deferred_state(&e,&m),RequiresReview);
    f.emit();deferred_tick(&mut e,&mut f,TS+10,TS+10,4000.5);
    assert_eq!(deferred_state(&e,&m),RequiresReview);assert!(!e.baskets.iter().any(|b|b.msg_id==55));
}

#[test]
fn malformed_open_ack_reconciles_only_from_exact_snapshot_while_close_or_disconnect_stays_latched() {
    for cmd in ["probe_malformed_next_response","probe_disconnect_next_response"] {
        for open in [false,true] {
            let mut f=Fixture::with_timeout(true,0.08,Duration::from_millis(150));
            if open { f.call("probe_valid_open",json!({})); }
            f.call(cmd,json!({}));
            let result=if open { f.bridge.open_market(req()).map(|_|0.) }
                else { f.bridge.close_position(ID,CloseReason::RiskFree) };
            assert!(result.is_err()); assert!(f.bridge.close_receipts_pending(),"{cmd}, open={open}");
            assert!(f.bridge.open_market(req()).is_err());
            if cmd=="probe_malformed_next_response" {
                f.bridge.refresh_state().unwrap();
                assert_eq!(f.bridge.positions().len(),if open {2}else{0});
                assert_eq!(f.bridge.close_receipts_pending(),!open,
                    "only an exact adopted OPEN has complete snapshot proof");
            }
        }
    }
}

#[test]
fn full_rpc_delayed_deal_keeps_basket_and_real_engine_credits_once() {
    let mut f = Fixture::new(true,0.01); let mut e = engine();
    // Live first publishes/anchors the account before it can request a close.
    let q=f.bridge.quote(); e.on_tick(&mut f.bridge,&q);
    e.close_everything(&mut f.bridge, TS, CloseReason::Manual);
    assert!(f.bridge.positions().is_empty());
    f.emit(); let q=f.bridge.quote(); e.on_tick(&mut f.bridge,&q);
    near(e.baskets[0].realized,0.5); near(e.stats.realized_today,0.5);
    f.call("probe_duplicate",json!({})); f.bridge.poll_state(); e.on_tick(&mut f.bridge,&q);
    near(e.baskets[0].realized,0.5); assert_eq!(e.stats.trades,1);
    assert!(f.bridge.close_receipt_issue().is_none());
}

#[test]
fn broker_sl_attribution_survives_snapshot_before_or_after_event() {
    for refresh_first in [false,true] {
        let mut f=Fixture::new(true,0.01);
        f.call("probe_sl",json!({}));
        if refresh_first { f.bridge.refresh_state().unwrap(); }
        f.emit(); let closed=f.bridge.drain_closed();
        assert_eq!(closed.len(),1); assert_eq!(closed[0].basket,Some(1));
        assert_eq!(closed[0].reason,CloseReason::Sl); near(closed[0].profit,0.5);
        assert!(f.bridge.positions().is_empty());
    }
}

#[test]
fn broker_generated_snapshot_loss_blocks_rearm_before_delayed_sl_receipt() {
    // Read-only LIVE observation B7: .05 closed -$5.70, next entry attempted
    // before the closed event. Synthetic prices/account, actual Bridge+Engine.
    let mut f=Fixture::new(true,0.05);let mut e=rearm_engine(&mut f.bridge);
    f.call("probe_profit_per_lot",json!({"value":-114.0}));
    f.call("probe_sl",json!({})); // broker close, NO bot close_position ACK
    f.bridge.refresh_state().unwrap();assert!(f.bridge.positions().is_empty());
    let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
    assert_eq!(e.baskets[0].rearms,0,"snapshot disappearance is not yet -$5.70 in the engine ledger");
    assert!(f.bridge.pendings().is_empty());assert!(e.wejscie_zablokowane(&f.bridge,TS));
    f.emit();assert!(e.wejscie_zablokowane(&f.bridge,TS));
    e.on_tick(&mut f.bridge,&q);near(e.baskets[0].realized,-1.90);
    assert!(!e.wejscie_zablokowane(&f.bridge,TS));assert_eq!(e.baskets[0].rearms,0);
}

#[test]
fn broker_generated_partial_snapshot_loss_blocks_all_new_entry_until_credit() {
    let mut f=Fixture::new(true,0.08);let mut e=engine();let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
    f.call("probe_sl",json!({"volume":0.02}));f.bridge.refresh_state().unwrap();
    near(f.bridge.positions()[0].volume,0.06);
    assert!(e.wejscie_zablokowane(&f.bridge,TS),"unexplained partial reduction must gate new risk");
    e.on_message(&mut f.bridge,&deferred_message(991));assert!(!e.baskets.iter().any(|b|b.msg_id==991));
    f.emit();assert!(e.wejscie_zablokowane(&f.bridge,TS));e.on_tick(&mut f.bridge,&q);
    near(e.baskets[0].realized,1.0);near(f.bridge.positions()[0].volume,0.06);
    assert!(!e.wejscie_zablokowane(&f.bridge,TS));
}

#[test]
fn broker_generated_tp_event_first_and_snapshot_first_release_after_accounting_only() {
    for snapshot_first in [false,true] {
        let mut f=Fixture::new(true,0.01);let mut e=rearm_engine(&mut f.bridge);
        f.call("probe_tp",json!({}));
        if snapshot_first {f.bridge.refresh_state().unwrap();assert!(f.bridge.close_receipts_pending());}
        f.emit();assert!(f.bridge.close_receipts_pending());assert!(f.bridge.open_market(req()).is_err());
        let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
        near(e.baskets[0].realized,4.3);assert_eq!(e.baskets[0].rearms,1);
        assert_eq!(f.bridge.pendings().len(),1);assert!(!f.bridge.close_receipts_pending());
        f.call("probe_duplicate",json!({}));f.bridge.poll_state();e.on_tick(&mut f.bridge,&q);
        near(e.baskets[0].realized,4.3);assert_eq!(e.stats.trades,1);assert!(!f.bridge.close_receipts_pending());
    }
}

#[test]
fn broker_generated_three_partial_volume_evidence_survives_snapshot_and_delivery_permutations() {
    for snapshots in 0..8 {for deliveries in 0..4 {
        let mut f=Fixture::new(true,0.08);let mut e=engine();let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
        for n in 0..3 {
            f.call("probe_tp",json!({"volume":0.02}));
            if snapshots&(1<<n)!=0 {f.bridge.refresh_state().unwrap();assert!(f.bridge.close_receipts_pending());}
            if n==2 || deliveries&(1<<n)!=0 {
                f.emit();assert!(f.bridge.close_receipts_pending());e.on_tick(&mut f.bridge,&q);
                near(e.baskets[0].realized,(n+1)as f64);assert!(!f.bridge.close_receipts_pending());
                f.call("probe_duplicate",json!({}));f.bridge.poll_state();e.on_tick(&mut f.bridge,&q);
                near(e.baskets[0].realized,(n+1)as f64);assert!(!f.bridge.close_receipts_pending());
            }
        }
        near(f.bridge.positions()[0].volume,0.02);assert_eq!(e.stats.trades,3);
    }}
}

#[test]
fn broker_generated_snapshot_gap_is_not_a_synthetic_close_and_rollback_cannot_resume() {
    use conduit_core::broker::{PendingReq,ReceiptBarrier};
    let mut f=Fixture::new(true,0.08);
    let pending=f.bridge.place_pending(PendingReq{kind:PendingKind::BuyLimit,volume:0.01,price:3999.0,
        sl:Some(3990.),tp:None,basket:Some(1),level:3,is_toucher:false,is_topup:false,comment:"fixture".into()}).unwrap();
    f.call("probe_tp",json!({"volume":0.02}));f.bridge.refresh_state().unwrap();
    let evidence=f.bridge.pending_volume_receipts();assert_eq!(evidence.len(),1);
    near(evidence[0]["observed_reduction"].as_f64().unwrap(),0.02);
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::Temporary);assert!(f.bridge.drain_closed().is_empty());
    // Contradictory larger row could be a legitimate additional fill OR stale
    // data. Neither is proven; it must not silently erase the missing receipt.
    f.call("probe_snapshot_volume",json!({"value":0.08}));f.bridge.refresh_state().unwrap();
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::RequiresReview);
    f.call("probe_snapshot_volume",json!({"value":null}));f.bridge.refresh_state().unwrap();
    f.emit();assert_eq!(f.bridge.drain_closed().len(),1);
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::RequiresReview);
    assert!(f.bridge.open_market(req()).is_err());
    f.bridge.modify_position(ID,Some(3998.),None).unwrap();f.bridge.cancel_pending(pending).unwrap();
    f.bridge.close_position(ID,CloseReason::Manual).unwrap();f.emit();
    assert_eq!(f.bridge.drain_closed().len(),1);assert!(f.bridge.close_receipts_pending());
}

#[test]
fn broker_generated_snapshot_reduction_off_preserves_legacy_rearm() {
    let mut f=Fixture::new(false,0.05);let mut e=rearm_engine(&mut f.bridge);
    f.call("probe_profit_per_lot",json!({"value":-114.0}));f.call("probe_sl",json!({}));
    f.bridge.refresh_state().unwrap();assert!(!f.bridge.close_receipts_pending());
    let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);assert_eq!(e.baskets[0].rearms,1);
    assert_eq!(f.bridge.pendings().len(),1);assert!(f.bridge.pending_volume_receipts().is_empty());
}

#[test]
fn unobserved_pending_fill_and_close_remains_explicitly_outside_position_evidence() {
    use conduit_core::broker::{PendingReq,ReceiptBarrier};
    let mut f=Fixture::new(true,0.0);
    f.bridge.place_pending(PendingReq{kind:PendingKind::BuyLimit,volume:0.01,price:3999.0,
        sl:Some(3990.),tp:None,basket:Some(1),level:3,is_toucher:false,is_topup:false,comment:"fixture".into()}).unwrap();
    f.call("probe_pending_roundtrip_unobserved",json!({}));f.bridge.refresh_state().unwrap();
    // Missing pending could be cancellation/expiry/fill: no fabricated position.
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::Clear);assert!(f.bridge.pending_volume_receipts().is_empty());
    f.emit();assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::RequiresReview);
    assert!(f.bridge.drain_closed().is_empty(),"unknown opening must not fabricate a closed basket trade");
}

#[test]
fn partial_then_full_and_three_partial_batch_never_double_decrement() {
    for batched in [false,true] {
        let mut f=Fixture::new(true,0.08);
        if batched {
            for _ in 0..3 { f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap(); }
            near(f.bridge.positions()[0].volume,0.02);
            f.emit(); near(f.bridge.positions()[0].volume,0.02);
            let closed=f.bridge.drain_closed(); assert_eq!(closed.len(),3);
            assert!(closed.iter().all(|c|c.basket==Some(1)&&c.reason==CloseReason::Partial));
            near(closed.iter().map(|c|c.profit).sum(),3.);
        } else {
            f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap(); f.emit();
            near(f.bridge.positions()[0].volume,0.06);
            let mut closed=f.bridge.drain_closed();
            f.bridge.close_position(ID,CloseReason::Manual).unwrap(); f.emit();
            closed.extend(f.bridge.drain_closed()); assert_eq!(closed.len(),2);
            assert!(closed.iter().all(|c|c.basket==Some(1)));
            near(closed.iter().map(|c|c.profit).sum(),4.);
            assert!(f.bridge.positions().is_empty());
        }
        assert!(f.bridge.close_receipt_issue().is_none());
    }
}

#[test]
fn acknowledged_partial_execution_uses_executed_not_requested_volume() {
    let mut f=Fixture::new(true,0.08);
    f.call("probe_partial_cap",json!({"cap":0.01}));
    f.bridge.close_partial(ID,0.04,CloseReason::Partial).unwrap();
    near(f.bridge.positions()[0].volume,0.07);
    f.bridge.close_position(ID,CloseReason::Manual).unwrap();
    near(f.bridge.positions()[0].volume,0.06); // full request, partial execution
    f.emit(); near(f.bridge.positions()[0].volume,0.06);
    assert_eq!(f.bridge.drain_closed().len(),2);
}

#[test]
fn physical_ticket_change_keeps_identifier_owner_and_runtime_metadata() {
    let mut f=Fixture::new(true,0.08);
    f.bridge.positions_mut()[0].frozen=true;
    f.bridge.positions_mut()[0].vsl=Some(3999.);
    f.bridge.positions_mut()[0].level=7;
    f.call("probe_ticket",json!({"ticket":9901})); f.bridge.refresh_state().unwrap();
    let p=&f.bridge.positions()[0]; assert_eq!(p.ticket,9901); assert_eq!(p.basket,Some(1));
    assert_eq!(p.level,7); assert!(p.frozen); assert_eq!(p.vsl,Some(3999.));
    f.bridge.close_position(9901,CloseReason::Tp).unwrap(); f.emit();
    let closed=f.bridge.drain_closed(); assert_eq!(closed[0].ticket,9901);
    assert_eq!(closed[0].basket,Some(1)); assert_eq!(closed[0].reason,CloseReason::Tp);
}

#[test]
fn incomplete_identifiers_are_quarantined_not_credited_or_deduplicated_as_zero() {
    for field in ["deal","position"] {
        let mut f=Fixture::new(true,0.08);
        f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();
        f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();
        f.call("probe_corrupt",json!({"field":field,"value":0})); f.emit();
        assert!(f.bridge.drain_closed().is_empty()); assert!(f.bridge.close_receipt_issue().is_some());
        assert!(f.bridge.open_market(req()).is_err());
        // Protective closing remains allowed even when new entries fail closed.
        f.bridge.close_position(ID,CloseReason::Manual).unwrap();
    }
}

#[test]
fn missing_position_snapshot_identifier_blocks_new_entries_without_ticket_guess() {
    let mut f=Fixture::new(true,0.08);
    f.call("probe_identifier",json!({"identifier":0})); f.bridge.refresh_state().unwrap();
    assert!(f.bridge.close_receipt_issue().is_some()); assert!(f.bridge.open_market(req()).is_err());
}

#[test]
fn missing_open_ack_identity_never_invents_physical_ticket_from_colliding_order() {
    let mut f=Fixture::new(true,0.08);
    // The malformed ACK's order deliberately collides with the existing position.
    assert!(f.bridge.open_market(req()).is_err());
    assert_eq!(f.bridge.positions().len(),1);
    near(f.bridge.positions()[0].volume,0.08);
    assert!(f.bridge.close_receipt_issue().is_some());
}

fn check_open_ack_proof(field:&str,value:Value) {
    for on in [false,true] {
        let mut f=Fixture::new(on,0.08);f.call("probe_valid_open",json!({}));
        let mut patch=serde_json::Map::new();patch.insert(field.into(),value.clone());
        f.call("probe_next_open_ack_patch",Value::Object(patch));
        // The broker really opens .01; only its acknowledgement is malformed.
        let result=f.bridge.open_market(req());
        if !on {
            assert!(result.is_ok(),"OFF keeps the pre-existing ACK fallback behavior");
            assert!(!f.bridge.close_receipts_pending());continue;
        }
        assert!(result.is_err(),"{field}={value}: incomplete/contradictory execution proof must not be adopted");
        assert_eq!(f.bridge.receipt_barrier(),conduit_core::broker::ReceiptBarrier::RequiresReview);
        assert_eq!(f.bridge.positions().len(),1,"ACK cannot fabricate an owned position from requested volume/current quote");
        f.bridge.refresh_state().unwrap();assert_eq!(f.bridge.positions().len(),2);
        near(f.bridge.positions().iter().find(|p|p.ticket==ID+10).unwrap().volume,0.01);
        assert_eq!(f.bridge.receipt_barrier(),conduit_core::broker::ReceiptBarrier::Clear,
            "one exact MT5 position proves execution and releases only the entry gate");
        f.bridge.refresh_state().unwrap();assert_eq!(f.bridge.positions().len(),2,"no duplicate execution");
        f.bridge.modify_position(ID+10,Some(3999.0),None).unwrap();
        f.bridge.close_position(ID,CloseReason::Manual).unwrap();f.emit();
        assert_eq!(f.bridge.drain_closed().len(),1,"protective closure remains possible");
        assert!(!f.bridge.close_receipts_pending(),"fully proven OPEN must not poison later receipts");
    }
}

macro_rules! open_ack_proof_test {
    ($name:ident,$field:literal,$value:expr)=>{
        #[test] fn $name(){check_open_ack_proof($field,json!($value));}
    };
}
open_ack_proof_test!(proof_open_ack_zero_volume,"volume",0.0);
open_ack_proof_test!(proof_open_ack_negative_volume,"volume",-0.01);
open_ack_proof_test!(proof_open_ack_excess_volume,"volume",0.02);
open_ack_proof_test!(proof_open_ack_missing_volume,"volume",Value::Null);
open_ack_proof_test!(proof_open_ack_missing_deal,"deal",Value::Null);
open_ack_proof_test!(proof_open_ack_zero_price,"price",0.0);
open_ack_proof_test!(proof_open_ack_negative_price,"price",-1.0);
open_ack_proof_test!(proof_open_ack_missing_price,"price",Value::Null);
open_ack_proof_test!(proof_open_ack_not_executed_retcode,"retcode",10008);

#[test]
fn proof_open_ack_confirmed_partial_fill_uses_actual_volume_and_price() {
    let mut f=Fixture::new(true,0.08);f.call("probe_valid_open",json!({}));
    f.call("probe_next_open_ack_patch",json!({"retcode":10010}));
    let mut request=req();request.volume=0.02;
    let ticket=f.bridge.open_market(request).unwrap();
    let p=f.bridge.positions().iter().find(|p|p.ticket==ticket).unwrap();
    near(p.volume,0.01);near(p.open_price,4000.5);assert!(!f.bridge.close_receipts_pending());
    f.bridge.refresh_state().unwrap();assert!(!f.bridge.close_receipts_pending());
    near(f.bridge.positions().iter().find(|p|p.ticket==ticket).unwrap().volume,0.01);
}

#[test]
fn position_identifier_api_requires_current_owned_verified_identity_not_ticket_equality() {
    let mut f=Fixture::new(true,0.08);assert_eq!(f.bridge.position_identifier(ID),Some(ID));
    f.call("probe_ticket",json!({"ticket":9500}));f.bridge.refresh_state().unwrap();
    assert_eq!(f.bridge.position_identifier(ID),None,"an old physical alias is not a current position");
    assert_eq!(f.bridge.position_identifier(9500),Some(ID),"ticket and stable identifier are distinct");
    f.call("probe_position_kind",json!({"value":1}));f.bridge.refresh_state().unwrap();
    assert_eq!(f.bridge.position_identifier(9500),None,"quarantined/reversed identity is not restoration proof");
    drop(f);
    let f=Fixture::new(false,0.08);assert_eq!(f.bridge.position_identifier(ID),None,"OFF has no proven alias ledger");
}

#[test]
fn incomplete_ack_does_not_refresh_foreign_positions_inside_an_engine_view() {
    use conduit_core::routing::{Widok,Wlasnosc};
    let mut f=Fixture::new(true,0.08);
    let mut waiting=Vec::new();
    {
        let owner=Wlasnosc{slot:1,zapasowy:false,znane_sloty:vec![0,1]};
        let mut view=Widok::nowy(&mut f.bridge,owner,&mut waiting);
        assert!(view.positions().is_empty()); // physical position belongs to slot0
        let mut request=req(); request.basket=Some(100001);
        assert!(view.open_market(request).is_err());
        assert!(view.positions().is_empty(),"failed RPC must not refresh another format into this view");
    }
    assert_eq!(f.bridge.positions().len(),1);
    near(f.bridge.positions()[0].volume,0.08);
}

#[test]
fn malformed_close_missing_id_is_not_silently_dropped_by_decoder() {
    for on in [false,true] {
        let mut f=Fixture::new(on,0.08);
        f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();
        f.call("probe_remove_field",json!({"field":"deal"})); f.emit();
        assert!(f.bridge.drain_closed().is_empty());
        assert_eq!(f.bridge.close_receipt_issue().is_some(),on);
        if on { assert!(f.bridge.open_market(req()).is_err()); }
    }
}

#[test]
fn failed_snapshot_retains_receipts_until_success_instead_of_using_empty_state() {
    let mut f=Fixture::new(true,0.08);
    f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();
    f.call("probe_fail_positions",json!({"enabled":true})); f.emit();
    assert!(f.bridge.drain_closed().is_empty()); assert!(f.bridge.close_receipt_issue().is_some());
    assert!(f.bridge.open_market(req()).is_err()); near(f.bridge.positions()[0].volume,0.06);
    f.call("probe_fail_positions",json!({"enabled":false})); f.bridge.poll_state();
    assert_eq!(f.bridge.drain_closed().len(),1); assert!(f.bridge.close_receipt_issue().is_none());
    near(f.bridge.positions()[0].volume,0.06);
}

#[test]
fn duplicate_deal_fingerprint_conflict_is_not_second_profit() {
    let mut f=Fixture::new(true,0.08);
    f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap(); f.emit();
    near(f.bridge.drain_closed()[0].profit,1.);
    f.call("probe_conflict",json!({})); f.bridge.poll_state();
    assert!(f.bridge.drain_closed().is_empty()); assert!(f.bridge.close_receipt_issue().is_some());
}

#[test]
fn manual_close_of_known_bot_position_uses_position_owner_not_close_deal_magic() {
    let mut f=Fixture::new(true,0.08);
    let mut e=engine(); let q=f.bridge.quote(); e.on_tick(&mut f.bridge,&q);
    f.call("probe_sl",json!({}));
    f.call("probe_corrupt",json!({"field":"magic","value":0}));
    f.call("probe_corrupt",json!({"field":"reason","value":0}));
    f.call("probe_corrupt",json!({"field":"ev","value":"closed_foreign"}));
    f.bridge.refresh_state().unwrap(); f.emit(); e.on_tick(&mut f.bridge,&q);
    near(e.baskets[0].realized,4.); assert_eq!(e.stats.trades,1);
    assert!(f.bridge.foreign_closed().is_empty());
    // The event channel is not part of economic identity: cross-channel replay is one deal.
    f.call("probe_duplicate_as_owned",json!({})); f.bridge.poll_state(); e.on_tick(&mut f.bridge,&q);
    near(e.baskets[0].realized,4.); assert_eq!(e.stats.trades,1);
    assert!(f.bridge.close_receipt_issue().is_none());
}

#[test]
fn unknown_manual_position_stays_foreign_and_does_not_credit_or_adopt_basket() {
    let mut f=Fixture::new(true,0.08);
    f.call("probe_foreign_close",json!({}));
    f.emit();
    assert!(f.bridge.drain_closed().is_empty());
    assert_eq!(f.bridge.foreign_closed().len(),1);
    assert!(f.bridge.close_receipt_issue().is_none());
}

#[test]
fn foreign_receipt_cannot_explain_disappearance_of_an_owned_position() {
    let mut f=Fixture::new(true,0.08);
    f.call("probe_sl",json!({}));
    f.call("probe_corrupt",json!({"field":"magic","value":0}));
    f.call("probe_corrupt",json!({"field":"position","value":7777}));
    f.call("probe_corrupt",json!({"field":"ev","value":"closed_foreign"}));
    f.emit();assert!(f.bridge.drain_closed().is_empty());
    assert_eq!(f.bridge.foreign_closed().len(),1);
    assert!(f.bridge.close_receipts_pending(),"an unrelated foreign deal does not settle our missing .08");
    let pending=f.bridge.pending_volume_receipts();assert_eq!(pending.len(),1);
    near(pending[0]["observed_reduction"].as_f64().unwrap(),0.08);
    near(pending[0]["receipted_volume"].as_f64().unwrap(),0.0);
}

#[test]
fn different_account_with_colliding_deal_is_rejected_at_transport_boundary() {
    let mut f=Fixture::new(true,0.08);
    f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap();
    f.call("probe_corrupt",json!({"field":"account","value":{"login":42,"server":"OTHER","trade_mode":0}}));
    f.emit(); assert!(!f.bridge.transport().is_connected());
    assert!(f.bridge.drain_closed().is_empty());
}

#[test]
fn off_preserves_legacy_lost_full_attribution_and_batched_partial_behavior() {
    let mut f=Fixture::new(false,0.01);
    f.bridge.close_position(ID,CloseReason::Manual).unwrap(); f.emit();
    assert_eq!(f.bridge.drain_closed()[0].basket,None);
    drop(f);
    let mut f=Fixture::new(false,0.08);
    for _ in 0..3 { f.bridge.close_partial(ID,0.02,CloseReason::Partial).unwrap(); }
    f.emit(); let closed=f.bridge.drain_closed();
    assert_eq!(closed.iter().map(|c|c.basket).collect::<Vec<_>>(),vec![Some(1),None,None]);
    assert_eq!(closed.iter().map(|c|c.reason).collect::<Vec<_>>(),vec![CloseReason::Partial,CloseReason::Manual,CloseReason::Manual]);
    near(f.bridge.positions()[0].volume,0.02);
}
