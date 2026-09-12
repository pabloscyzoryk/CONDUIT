//! Offline transport fixture: no terminal, MetaTrader package or login.
use conduit_core::{broker::{Broker, OrderReq, PendingReq}, types::*};
use conduit_mt5::{Mt5Bridge, SidecarConfig};
use serde_json::{json, Value};
use std::{io::{BufRead, BufReader, Write}, net::{TcpListener, TcpStream},
    sync::{Arc, Mutex}, thread, time::{Duration, Instant}};

struct Fixture {
    bridge: Mt5Bridge,
    requests: Arc<Mutex<Vec<Value>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let reserve = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = reserve.local_addr().unwrap().port();
        drop(reserve);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(stream) => break stream,
                    Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                    Err(error) => panic!("offline fixture connection: {error}"),
                }
            };
            writeln!(stream, "{}", json!({"ev":"hello","proto":1,"ready":true,"sidecar":"FAKE-PRICE-CACHE"})).unwrap();
            stream.flush().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 { break; }
                let request: Value = serde_json::from_str(&line).unwrap();
                let command = request["cmd"].as_str().unwrap();
                seen.lock().unwrap().push(request.clone());
                let result = match command {
                    "account" => json!({"login":42,"server":"synthetic","trade_mode":0,
                        "currency":"USD","balance":1000.0,"equity":1000.0,
                        "margin":0.0,"margin_free":1000.0,"leverage":500}),
                    "symbol_info" => json!({"symbol":"XAUUSD","digits":2,"point":0.01,
                        "stops_level_points":0.0,"volume_min":0.01,"volume_max":100.0,
                        "volume_step":0.01,"contract_size":100.0,"trade_mode":4}),
                    "quote" => json!({"bid":2100.0,"ask":2100.2,"ts":1700000000000i64}),
                    "positions" | "orders" => json!([]),
                    "open_market" => json!({"retcode":10009,"position":101,"order":101,
                        "deal":1001,"price":2100.2,"volume":0.01}),
                    "place_pending" => json!({"retcode":10009,"order":201,"volume":0.01}),
                    "modify_position" | "modify_pending" => json!({"retcode":10009}),
                    "subscribe_ticks" | "probe_finish" => json!({}),
                    _ => panic!("unexpected offline RPC {command}"),
                };
                writeln!(stream, "{}", json!({"id":request["id"],"ok":true,"result":result})).unwrap();
                stream.flush().unwrap();
                if command == "probe_finish" { break; }
            }
        });
        let bridge = Mt5Bridge::connect(SidecarConfig { autostart:false, port,
            request_timeout:Duration::from_secs(3), connect_timeout:Duration::from_secs(3),
            close_receipt_reconcile:false, ..Default::default() }).unwrap();
        Self { bridge, requests, worker:Some(worker) }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.bridge.transport().call("probe_finish", json!({}));
        if let Some(worker) = self.worker.take() { worker.join().unwrap(); }
    }
}

#[test]
fn cache_and_wire_share_broker_precision_for_all_four_price_operations() {
    let mut fixture = Fixture::new();
    let market = fixture.bridge.open_market(OrderReq { side:Side::Buy, volume:0.01,
        sl:Some(2090.126), tp:Some(2120.124), basket:Some(1), level:0,
        is_toucher:false, comment:"synthetic".into() }).unwrap();
    let position = &fixture.bridge.positions()[0];
    assert_eq!((position.sl, position.tp), (Some(2090.13), Some(2120.12)));
    fixture.bridge.modify_position(market, Some(2091.126), Some(2121.124)).unwrap();
    let position = &fixture.bridge.positions()[0];
    assert_eq!((position.sl, position.tp), (Some(2091.13), Some(2121.12)));

    let pending = fixture.bridge.place_pending(PendingReq { kind:PendingKind::BuyLimit,
        price:2095.126, volume:0.01, sl:Some(2085.126), tp:Some(2115.124), basket:Some(2),
        level:0, is_toucher:false, is_topup:false,no_market_fallback:false, comment:"synthetic".into() }).unwrap();
    let order = &fixture.bridge.pendings()[0];
    assert_eq!((order.price, order.sl, order.tp), (2095.13, Some(2085.13), Some(2115.12)));
    fixture.bridge.modify_pending(pending, 2096.126, Some(2086.126), Some(2116.124)).unwrap();
    let order = &fixture.bridge.pendings()[0];
    assert_eq!((order.price, order.sl, order.tp), (2096.13, Some(2086.13), Some(2116.12)));

    let requests = fixture.requests.lock().unwrap();
    for (command, sl, tp) in [("open_market",2090.13,2120.12),
        ("modify_position",2091.13,2121.12), ("place_pending",2085.13,2115.12),
        ("modify_pending",2086.13,2116.12)] {
        let request = requests.iter().find(|r| r["cmd"] == command).unwrap();
        assert_eq!(request["args"]["sl"], sl);
        assert_eq!(request["args"]["tp"], tp);
    }
}
