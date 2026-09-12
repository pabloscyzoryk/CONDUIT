//! Real Bridge/Transport with a synthetic loopback sidecar; no terminal or Python.
use conduit_core::{broker::{Broker, OrderReq, ReceiptBarrier, UnconfirmedOpen},
    engine::Engine, settings::{Settings, PendingCrossPolicy}, types::*};
use conduit_mt5::{Mt5Bridge, SidecarConfig};
use serde_json::{json, Value};
use std::{io::{BufRead, BufReader, Write}, net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, MutexGuard, atomic::{AtomicBool, AtomicUsize, Ordering}},
    thread, time::{Duration, Instant}};

const TS: i64 = 1_800_000_000_000;
const TICKET: u64 = 8101;
static PORT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct State {
    positions: Vec<Value>, reject: bool, valid_ack: bool, disconnect: bool,
    post_send_not_initialized: bool,
    fail_positions: bool, fail_orders: bool, partial: Option<f64>, opens: usize,
}
struct Fixture {
    bridge: Mt5Bridge, state: Arc<Mutex<State>>, requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>, worker: Option<thread::JoinHandle<()>>,
    _lock: MutexGuard<'static, ()>,
}
fn send(stream: &mut TcpStream, value: Value) {
    writeln!(stream, "{value}").unwrap(); stream.flush().unwrap();
}
impl Fixture {
    fn new() -> Self {
        let lock = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let reservation = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = reservation.local_addr().unwrap().port(); drop(reservation);
        let state = Arc::new(Mutex::new(State::default())); let worker_state = state.clone();
        let stop = Arc::new(AtomicBool::new(false)); let worker_stop = stop.clone();
        let requests = Arc::new(AtomicUsize::new(0)); let worker_requests = requests.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop { match TcpStream::connect(("127.0.0.1", port)) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                Err(e) => panic!("synthetic sidecar connect: {e}"),
            }};
            stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            stream.set_nodelay(true).unwrap();
            send(&mut stream, json!({"ev":"hello","proto":1,"ready":true,"sidecar":"SYNTHETIC-OPEN-IDENTITY"}));
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break, Ok(_) => {},
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                        if worker_stop.load(Ordering::Relaxed) { break; } continue;
                    }, Err(e) => panic!("synthetic sidecar read: {e}"),
                }
                worker_requests.fetch_add(1, Ordering::SeqCst);
                let req: Value = serde_json::from_str(&line).unwrap();
                let command = req["cmd"].as_str().unwrap(); let args = &req["args"];
                let mut s = worker_state.lock().unwrap();
                if command == "open_market" && s.reject {
                    s.opens += 1;
                    send(&mut stream, json!({"id":req["id"],"ok":false,
                        "error":{"code":10006,"msg":"synthetic definite refusal"}})); continue;
                }
                let result = match command {
                    "account" => json!({"login":42,"server":"SYNTHETIC-OPEN-DEMO","trade_mode":0,
                        "balance":600.0,"equity":600.0,"margin":0.0,"margin_free":600.0,
                        "leverage":500,"currency":"USD"}),
                    "symbol_info" => json!({"symbol":"XAUUSD","digits":2,"point":0.01,
                        "stops_level_points":0.0,"volume_min":0.01,"volume_max":100.0,
                        "volume_step":0.01,"contract_size":100.0,"trade_mode":4}),
                    "quote" => json!({"bid":4000.0,"ask":4000.2,"ts":TS}),
                    "positions" if s.fail_positions => json!("synthetic invalid snapshot"),
                    "positions" => json!(s.positions),
                    "orders" if s.fail_orders => json!("synthetic invalid orders"),
                    "orders" => json!([]),
                    "subscribe_ticks" => json!({}),
                    "modify_position" | "cancel_pending" => json!({"retcode":10009}),
                    "open_market" => {
                        s.opens += 1;
                        let volume = s.partial.unwrap_or(args["volume"].as_f64().unwrap());
                        let ticket = TICKET + s.positions.len() as u64;
                        s.positions.push(json!({"ticket":ticket,"identifier":ticket + 1000,
                            "kind":if args["side"] == "buy" {0} else {1}, "volume":volume,
                            "price_open":4000.2,"time_msc":TS,"sl":args["sl"].as_f64().unwrap_or(0.0),
                            "tp":args["tp"].as_f64().unwrap_or(0.0),"symbol":"XAUUSD",
                            "magic":777,"comment":args["comment"]}));
                        if s.disconnect { break; }
                        if s.post_send_not_initialized {
                            send(&mut stream, json!({"id":req["id"],"ok":false,
                                "error":{"code":-1,"msg":"synthetic account missing after send"}}));
                            continue;
                        }
                        json!({"retcode":if volume < args["volume"].as_f64().unwrap() {10010} else {10009},
                            "order":ticket,"deal":ticket+2000,"position":if s.valid_ack {ticket} else {0},
                            "position_identifier":if s.valid_ack {ticket+1000} else {0},
                            "volume":volume,"price":4000.2})
                    },
                    other => panic!("unexpected synthetic RPC: {other}"),
                };
                send(&mut stream, json!({"id":req["id"],"ok":true,"result":result}));
            }
        });
        let bridge = Mt5Bridge::connect(SidecarConfig { autostart:false, port, magic:777,
            close_receipt_reconcile:true, request_timeout:Duration::from_millis(300),
            connect_timeout:Duration::from_secs(3), ..Default::default() }).unwrap();
        Self { bridge, state, requests, stop, worker:Some(worker), _lock:lock }
    }
    fn unknown(&mut self) -> UnconfirmedOpen {
        assert!(self.bridge.open_market(request()).is_err());
        self.bridge.unconfirmed_open().expect("submitted incomplete ACK must retain identity")
    }
    fn resolved(&mut self) -> UnconfirmedOpen {
        let intent = self.unknown(); self.bridge.refresh_state().unwrap();
        assert_eq!(self.bridge.receipt_barrier(), ReceiptBarrier::Clear);
        assert_eq!(self.bridge.confirmed_open(&intent), Some(TICKET)); intent
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() { worker.join().unwrap(); }
    }
}
fn request() -> OrderReq {
    OrderReq { side:Side::Buy, volume:0.08, sl:Some(3990.0), tp:Some(4020.0),
        basket:Some(1), level:2, is_toucher:false, comment:"synthetic".into() }
}

#[test]
fn incomplete_ack_then_unique_snapshot_confirms_without_rpc_or_resend() {
    let mut f = Fixture::new(); let intent = f.unknown();
    assert!(intent.machine_comment.contains("~1!"));
    assert_eq!(intent.session, f.bridge.execution_session().unwrap());
    assert_eq!(intent.requested_volume, 0.08);
    assert_eq!(f.bridge.confirmed_open(&intent), None);
    f.bridge.refresh_state().unwrap(); let count = f.requests.load(Ordering::SeqCst);
    for _ in 0..3 { assert_eq!(f.bridge.confirmed_open(&intent), Some(TICKET)); }
    assert_eq!(f.requests.load(Ordering::SeqCst), count);
    assert_eq!(f.state.lock().unwrap().opens, 1);
}

#[test]
fn not_initialized_after_send_retains_unknown_outcome_and_exact_adoption() {
    let mut f = Fixture::new();
    f.state.lock().unwrap().post_send_not_initialized = true;
    let intent = f.unknown();
    let evidence = f.bridge.operation_evidence().unwrap();
    assert_eq!(evidence.outcome, conduit_mt5::operation_evidence::Outcome::ConfirmationPending);
    assert_eq!(evidence.attempts[0].status, "transport_error");
    assert_eq!(evidence.attempts[0].retcode, Some(-1));
    assert!(f.bridge.open_market(request()).is_err());
    assert_eq!(f.state.lock().unwrap().opens, 1);
    f.bridge.refresh_state().unwrap();
    assert_eq!(f.bridge.confirmed_open(&intent), Some(TICKET));
    assert_eq!(f.state.lock().unwrap().opens, 1);
}

#[test]
fn definite_refusal_and_local_no_send_never_publish_previous_intent() {
    let mut f = Fixture::new(); f.state.lock().unwrap().reject = true;
    assert!(f.bridge.open_market(request()).is_err());
    assert!(f.bridge.unconfirmed_open().is_none());
    f.state.lock().unwrap().reject = false;
    let intent = f.unknown();
    let count = f.state.lock().unwrap().opens;
    // Existing unknown-open HOLD rejects locally, without another RPC.
    assert!(f.bridge.open_market(request()).is_err());
    assert!(f.bridge.unconfirmed_open().is_none());
    assert_eq!(f.state.lock().unwrap().opens, count);
    f.bridge.refresh_state().unwrap();
    assert_eq!(f.bridge.confirmed_open(&intent), Some(TICKET));
    let mut invalid = request(); invalid.volume = 0.0;
    assert!(f.bridge.open_market(invalid).is_err());
    assert!(f.bridge.unconfirmed_open().is_none());
}

#[test]
fn next_successful_or_protective_operation_clears_last_descriptor() {
    let mut f = Fixture::new(); let intent = f.resolved();
    f.bridge.modify_position(TICKET, Some(3991.0), Some(4020.0)).unwrap();
    assert!(f.bridge.unconfirmed_open().is_none());
    assert_eq!(f.bridge.confirmed_open(&intent), Some(TICKET));
    f.state.lock().unwrap().valid_ack = true;
    assert!(f.bridge.open_market(request()).is_ok());
    assert!(f.bridge.unconfirmed_open().is_none());
}

#[test]
fn different_scope_or_full_ordinal_is_not_identity_but_generation_may_differ() {
    let mut f = Fixture::new(); let intent = f.resolved();
    let mut wrong = intent.clone(); wrong.session.scope.push_str("-OTHER");
    assert_eq!(f.bridge.confirmed_open(&wrong), None);
    wrong = intent.clone(); wrong.machine_comment = wrong.machine_comment.replace("~1!", "~2!");
    assert_eq!(f.bridge.confirmed_open(&wrong), None);
    wrong = intent.clone(); wrong.session.generation = wrong.session.generation.wrapping_add(10);
    assert_eq!(f.bridge.confirmed_open(&wrong), Some(TICKET));
    let restored:UnconfirmedOpen = serde_json::from_str(&serde_json::to_string(&intent).unwrap()).unwrap();
    assert_eq!(f.bridge.confirmed_open(&restored), Some(TICKET));
}

#[test]
fn persisted_intent_resolves_after_fresh_bridge_reconstructs_authoritative_identity() {
    let mut f = Fixture::new(); let intent = f.unknown();
    let saved = serde_json::to_string(&intent).unwrap();
    let rows = f.state.lock().unwrap().positions.clone();
    drop(f);
    let mut restarted = Fixture::new();
    restarted.state.lock().unwrap().positions = rows;
    restarted.bridge.refresh_state().unwrap();
    let intent:UnconfirmedOpen = serde_json::from_str(&saved).unwrap();
    assert!(restarted.bridge.unconfirmed_open().is_none());
    assert_ne!(intent.session.generation, restarted.bridge.execution_session().unwrap().generation);
    assert_eq!(restarted.bridge.confirmed_open(&intent), Some(TICKET));
    assert_eq!(restarted.state.lock().unwrap().opens, 0);
}

#[test]
fn descriptor_uses_normalized_wire_volume_not_unrounded_request() {
    let mut f = Fixture::new(); let mut order = request(); order.volume = 0.0849;
    assert!(f.bridge.open_market(order).is_err());
    let intent = f.bridge.unconfirmed_open().unwrap();
    assert_eq!(intent.requested_volume, 0.08);
    assert_eq!(intent.requested_volume, f.state.lock().unwrap().positions[0]["volume"].as_f64().unwrap());
    f.bridge.refresh_state().unwrap();
    assert_eq!(f.bridge.confirmed_open(&intent), Some(TICKET));
}

#[test]
fn exact_raw_comment_required_even_if_legacy_cache_reconstructs_it() {
    for suffix in ["CD1.2", "CD1.2~1", "CD1.2~1!", "CD1.2~2!-synthetic"] {
        let mut f = Fixture::new(); let intent = f.resolved();
        f.state.lock().unwrap().positions[0]["comment"] = json!(suffix);
        f.bridge.refresh_state().unwrap();
        assert_eq!(f.bridge.confirmed_open(&intent), None, "literal comment {suffix}");
    }
}

#[test]
fn ambiguous_matching_positions_or_missing_identifier_remain_unconfirmed() {
    for missing_id in [false,true] {
        let mut f = Fixture::new(); let intent = f.unknown();
        { let mut s = f.state.lock().unwrap();
          if missing_id { s.positions[0]["identifier"] = json!(0); }
          else { let mut other = s.positions[0].clone(); other["ticket"] = json!(TICKET+1);
            other["identifier"] = json!(TICKET+1001); s.positions.push(other); }
        }
        f.bridge.refresh_state().unwrap();
        assert_eq!(f.bridge.confirmed_open(&intent), None);
        assert_ne!(f.bridge.receipt_barrier(), ReceiptBarrier::Clear);
    }
}

#[test]
fn partial_fill_is_valid_but_geometry_or_volume_mismatch_is_not() {
    let mut f = Fixture::new(); f.state.lock().unwrap().partial = Some(0.03);
    let intent = f.resolved();
    for change in 0..5 { let mut wrong = intent.clone(); match change {
        0 => wrong.requested_volume = 0.02,
        1 => wrong.side = Side::Sell,
        2 => wrong.level += 1,
        3 => wrong.basket = Some(2),
        _ => wrong.submitted_quote_ts = TS + 2001,
    }; assert_eq!(f.bridge.confirmed_open(&wrong), None); }
}

#[test]
fn failed_state_read_or_runtime_barrier_invalidates_old_snapshot_proof() {
    for orders in [false,true] {
        let mut f = Fixture::new(); let intent = f.resolved();
        { let mut s=f.state.lock().unwrap(); s.fail_orders=orders; s.fail_positions=!orders; }
        assert!(f.bridge.refresh_state().is_err());
        assert_eq!(f.bridge.confirmed_open(&intent), None);
    }
    let mut f = Fixture::new(); let intent = f.resolved();
    f.bridge.hold_new_entries("synthetic review barrier");
    assert_eq!(f.bridge.confirmed_open(&intent), None);
}

#[test]
fn disconnect_preserves_pre_send_account_identity_without_confirmation() {
    let mut f = Fixture::new(); let before = f.bridge.execution_session().unwrap();
    f.state.lock().unwrap().disconnect = true;
    let intent = f.unknown();
    assert_eq!(intent.session, before);
    assert_eq!(f.bridge.confirmed_open(&intent), None);
    assert_eq!(f.state.lock().unwrap().opens, 1);
}

#[test]
fn engine_counts_adopted_rearm_once_and_preserves_configured_count_and_cooldown() {
    let mut f = Fixture::new();
    let mut cfg = Settings::default();
    cfg.server_tz_offset_ms = 0; cfg.exec_latency_ms = 0;
    cfg.auto_limit = true; cfg.entry_units = 1; cfg.risk_per_basket_pct = 0.0;
    cfg.max_portfolio_risk_pct = 0.0; cfg.rearm_grid_on_return = true;
    cfg.rearm_block_after_secured = false; cfg.rearm_min_gap_min = 15.0;
    cfg.rearm_max_times = 1; cfg.rearm_min_basket_profit = 0.0;
    cfg.pending_cross_policy = PendingCrossPolicy::Market; cfg.fast_addon_move_usd = 0.0;
    let mut engine = Engine::new(cfg, 600.0);
    // A restored, previously active basket is eligible for one market rearm.
    // This avoids simulating unrelated initial pending acknowledgements.
    let mut basket:Basket = serde_json::from_value(json!({"id":1,"source":SourceKey::new(1,None),
        "source_name":"synthetic","msg_id":1,"side":"Buy","is_limit":true,
        "entry_lo":3995.0,"entry_hi":4000.0,"zone_lo":3995.0,"zone_hi":4000.0,
        "sl":3990.0,"tps":[4010.0],"tp_stage":0,"created_ts":TS-60000,
        "state":"Working","tickets":[],"pendings":[],"realized":10.0,
        "events":[],"had_positions":true})).unwrap();
    basket.levels = vec![GridLevel {price:3999.0,base_units:1,volume:0.01,
        sl:Some(3990.0),tp:Some(4010.0),level:0,is_toucher:false,
        fill_ts:0,fill_px:0.0,cancelled:false,filled:false}];
    engine.adopt_baskets(vec![basket]);
    let q = Quote {ts:TS,bid:3997.25,ask:3997.45}; f.bridge.set_replay_quote(q);
    engine.on_tick(&mut f.bridge, &q);
    assert_eq!(f.state.lock().unwrap().opens, 1, "real Engine submitted the rearm");
    assert_eq!(engine.baskets[0].rearms, 0, "incomplete ACK does not invent confirmation");
    assert!(engine.wejscie_zablokowane(&f.bridge, q.ts));
    engine.on_tick(&mut f.bridge, &q);
    assert_eq!(f.state.lock().unwrap().opens, 1, "no resend while confirmation is pending");
    f.bridge.refresh_state().unwrap();
    let next = Quote {ts:TS+2,..q}; f.bridge.set_replay_quote(next);
    engine.on_tick(&mut f.bridge, &next);
    assert_eq!(engine.baskets[0].rearms, 1);
    assert_eq!(engine.baskets[0].last_rearm_ts, TS);
    engine.on_tick(&mut f.bridge, &next);
    assert_eq!(engine.baskets[0].rearms, 1, "repeated observation cannot count the batch twice");
    assert_eq!(f.state.lock().unwrap().opens, 1);
    assert!(!engine.wejscie_zablokowane(&f.bridge, next.ts), "receipt HOLD is resolved");
    // Offer another empty level: neither existing occupancy nor a stale HOLD
    // may be the reason that configured rearm limits block the next send.
    let mut extra = engine.baskets[0].levels[0].clone();
    extra.level = 1; extra.price = 3998.0; extra.filled = false; extra.fill_ts = 0;
    engine.baskets[0].levels.push(extra);
    engine.cfg.rearm_max_times = 0; // cooldown alone
    engine.on_tick(&mut f.bridge, &next);
    assert_eq!(f.state.lock().unwrap().opens, 1);
    engine.cfg.rearm_max_times = 1; engine.cfg.rearm_min_gap_min = 0.0; // count alone
    engine.on_tick(&mut f.bridge, &next);
    assert_eq!(f.state.lock().unwrap().opens, 1);
}
