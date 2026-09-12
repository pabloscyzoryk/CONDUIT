//! Production TCP/Bridge cache; the peer is synthetic, never MT5 or Python.
use conduit_core::{broker::Broker, types::Quote};
use conduit_mt5::{Mt5Bridge,SidecarConfig};
use serde_json::{json,Value};
use std::{io::{BufRead,BufReader,Write},net::{TcpStream,TcpListener},thread,
    sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}},time::{Duration,Instant}};
static PORT_LOCK:Mutex<()>=Mutex::new(());
fn send(s:&mut TcpStream,v:Value){writeln!(s,"{v}").unwrap();s.flush().unwrap();}
fn identity()->Value{json!({"login":42,"server":"synthetic-m1","trade_mode":0})}
fn packet()->Value{json!({"ev":"m1_bars","schema":1,"symbol":"XAUUSD","account":identity(),
    "observed_utc_ms":10,"available_at_ms":180000,"complete":true,"error":null,
    "bars":[{"ts":60000,"open":4000.,"high":4004.,"low":3998.,"close":4002.,"max_spread":0.,"observations":0},
    {"ts":120000,"open":4002.,"high":4005.,"low":3999.,"close":4001.,"max_spread":0.,"observations":0}]})}

#[test]
fn complete_m1_bridge_opt_in_exact_cache_no_rpc_on_read_and_failure_clear() {
    let _lock=PORT_LOCK.lock().unwrap();
    let listener=TcpListener::bind(("127.0.0.1",0)).unwrap();let port=listener.local_addr().unwrap().port();drop(listener);
    let calls=Arc::new(AtomicUsize::new(0));let worker_calls=calls.clone();
    let events=Arc::new(Mutex::new(Vec::<Value>::new()));let worker_events=events.clone();
    let handle=thread::spawn(move||{
        let deadline=Instant::now()+Duration::from_secs(5);
        let mut s=loop{match TcpStream::connect(("127.0.0.1",port)){Ok(s)=>break s,Err(_) if Instant::now()<deadline=>thread::sleep(Duration::from_millis(5)),Err(e)=>panic!("fixture: {e}")}};
        s.set_nodelay(true).unwrap();send(&mut s,json!({"ev":"hello","proto":1,"ready":true,"sidecar":"SYNTHETIC-M1"}));
        let mut reader=BufReader::new(s.try_clone().unwrap());
        loop{let mut line=String::new();if reader.read_line(&mut line).unwrap_or(0)==0{break;}
            let r:Value=serde_json::from_str(&line).unwrap();worker_calls.fetch_add(1,Ordering::SeqCst);
            let result=match r["cmd"].as_str().unwrap(){
                "account"=>json!({"login":42,"server":"synthetic-m1","trade_mode":0,"balance":600.,"equity":600.,"margin":0.,"margin_free":600.,"leverage":500,"currency":"USD"}),
                "symbol_info"=>json!({"symbol":"XAUUSD","digits":2,"point":0.01,"stops_level_points":0.,"volume_min":0.01,"volume_max":100.,"volume_step":0.01,"contract_size":100.}),
                "quote"=>json!({"ts":180000,"bid":4001.,"ask":4001.2}),
                "positions"|"orders"=>json!([]),"subscribe_ticks"=>json!({}),
                "shutdown" => break,
                "t100_bars"=>json!({"schema":1,"enabled":r["args"]["enabled"]}),
                "fixture_emit"=>{for e in worker_events.lock().unwrap().drain(..){send(&mut s,e);}json!({})},
                other=>panic!("unexpected RPC: {other}"),
            };send(&mut s,json!({"id":r["id"],"ok":true,"result":result}));
        }
    });
    let mut b=Mt5Bridge::connect(SidecarConfig{autostart:false,port,close_receipt_reconcile:true,
        request_timeout:Duration::from_secs(2),connect_timeout:Duration::from_secs(3),..Default::default()}).unwrap();
    let before=calls.load(Ordering::SeqCst);
    b.configure_t100_bars(false).unwrap();assert_eq!(b.complete_m1_bars(None),Some(&[][..]));
    assert_eq!(before,calls.load(Ordering::SeqCst),"G7 OFF has no new RPC");
    b.configure_t100_bars(true).unwrap();b.configure_t100_bars(true).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst),before+1,"enable is idempotent");
    events.lock().unwrap().push(packet());
    b.transport().call("fixture_emit",json!({})).unwrap();b.poll_ticks();
    let n=calls.load(Ordering::SeqCst);let bars=b.complete_m1_bars(None).unwrap().to_vec();assert_eq!(bars.len(),2);
    assert_eq!(b.complete_m1_bars(Some(60000)).unwrap(),&bars[1..]);
    assert!(b.complete_m1_bars(Some(120000)).unwrap().is_empty());
    assert_eq!(n,calls.load(Ordering::SeqCst),"recording/policy getter is cache-only");
    b.set_replay_quote(Quote{ts:179999,bid:4001.,ask:4001.2});assert!(b.complete_m1_bars(None).unwrap().is_empty());
    b.set_replay_quote(Quote{ts:180000,bid:4001.,ask:4001.2});
    let mut bad=packet();bad["bars"]=json!("malformed");events.lock().unwrap().push(bad);
    b.transport().call("fixture_emit",json!({})).unwrap();b.poll_ticks();assert!(b.complete_m1_bars(None).unwrap().is_empty());
    assert!(b.t100_bars_issue().is_some());
    events.lock().unwrap().push(packet());b.transport().call("fixture_emit",json!({})).unwrap();b.poll_ticks();assert_eq!(b.complete_m1_bars(None).unwrap().len(),2);
    b.configure_t100_bars(false).unwrap();let n=calls.load(Ordering::SeqCst);b.configure_t100_bars(false).unwrap();
    assert!(b.complete_m1_bars(None).unwrap().is_empty());assert_eq!(n,calls.load(Ordering::SeqCst));
    drop(b);handle.join().unwrap();
}
