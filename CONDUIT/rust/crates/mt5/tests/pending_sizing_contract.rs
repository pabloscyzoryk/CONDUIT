//! Real Bridge/Transport and synthetic loopback responses. No Python, terminal or broker.
use conduit_core::{broker::{Broker, OrderReq, PendingReq}, types::*};
use conduit_mt5::{Mt5Bridge, SidecarConfig};
use serde_json::{json, Value};
use std::{io::{BufRead, BufReader, Write},net::{TcpListener,TcpStream},
    sync::{Arc,Mutex,MutexGuard},thread,time::{Duration,Instant}};
static PORT_LOCK:Mutex<()>=Mutex::new(());
const TS:i64=1_800_000_000_000;
#[derive(Default)]
struct State { orders:Vec<Value>, positions:Vec<Value>, requests:Vec<Value>, error:Option<i64> }
struct Fixture { bridge:Mt5Bridge,state:Arc<Mutex<State>>,worker:Option<thread::JoinHandle<()>>,_lock:MutexGuard<'static,()> }
impl Fixture {
 fn new(orders:Vec<Value>,error:Option<i64>)->Self {
  let lock=PORT_LOCK.lock().unwrap_or_else(|e|e.into_inner());
  let listener=TcpListener::bind(("127.0.0.1",0)).unwrap();let port=listener.local_addr().unwrap().port();drop(listener);
  let state=Arc::new(Mutex::new(State{orders,error,..Default::default()}));let worker_state=state.clone();
  let worker=thread::spawn(move||{
   let deadline=Instant::now()+Duration::from_secs(5);
   let mut stream=loop{match TcpStream::connect(("127.0.0.1",port)){Ok(s)=>break s,
    Err(_) if Instant::now()<deadline=>thread::sleep(Duration::from_millis(5)),Err(e)=>panic!("fixture connect {e}")}};
   stream.set_nodelay(true).unwrap();
   writeln!(stream,"{}",json!({"ev":"hello","proto":1,"ready":true,"sidecar":"SYNTHETIC-PENDING-SIZING"})).unwrap();stream.flush().unwrap();
   let mut reader=BufReader::new(stream.try_clone().unwrap());
   loop {
    let mut line=String::new();if reader.read_line(&mut line).unwrap()==0{break;}
    let req:Value=serde_json::from_str(&line).unwrap();let cmd=req["cmd"].as_str().unwrap();let args=&req["args"];
    let mut s=worker_state.lock().unwrap();s.requests.push(req.clone());
    let result=match cmd {
     "account"=>json!({"login":42,"server":"SYNTHETIC-DEMO","trade_mode":0,"currency":"USD","balance":1000.,"equity":1000.,"margin":0.,"margin_free":1000.,"leverage":500}),
     "symbol_info"=>json!({"symbol":"XAUUSD","digits":2,"point":0.01,"stops_level_points":0.,"volume_min":0.01,"volume_max":100.,"volume_step":0.01,"contract_size":100.,"trade_mode":4}),
     "quote"=>json!({"bid":2100.,"ask":2100.2,"ts":TS}),
     "positions"=>json!(s.positions),"orders"=>json!(s.orders),
     "place_pending"=>{
      let ticket=201+s.orders.len() as u64;
      s.orders.push(json!({"ticket":ticket,"kind":args["kind"],"volume":args["volume"],"price_open":args["price"],"time_msc":TS,
        "sl":args["sl"],"tp":args["tp"],"magic":777,"symbol":"XAUUSD","comment":args["comment"]}));
      json!({"retcode":10009,"order":ticket,"volume":args["volume"]})
     },
     "open_market"=>{
      let ticket=101+s.positions.len() as u64;
      s.positions.push(json!({"ticket":ticket,"identifier":ticket+1000,"kind":if args["side"]=="buy"{0}else{1},"volume":args["volume"],
       "price_open":2100.2,"time_msc":TS,"sl":args["sl"],"tp":args["tp"],"magic":777,"symbol":"XAUUSD","comment":args["comment"]}));
      json!({"retcode":10009,"position":ticket,"position_identifier":ticket+1000,"order":ticket,"deal":ticket+2000,"price":2100.2,"volume":args["volume"]})
     },
     "subscribe_ticks"|"probe_finish"=>json!({}),
     other=>panic!("unexpected offline command {other}"),
    };
    let response=if matches!(cmd,"open_market"|"place_pending") && s.error.is_some(){json!({"id":req["id"],"ok":false,"error":{"code":s.error.unwrap(),"msg":"synthetic ambiguous send"}})}
      else{json!({"id":req["id"],"ok":true,"result":result})};
    writeln!(stream,"{response}").unwrap();stream.flush().unwrap();if cmd=="probe_finish"{break;}
   }
  });
  let bridge=Mt5Bridge::connect(SidecarConfig{autostart:false,port,magic:777,close_receipt_reconcile:true,
    request_timeout:Duration::from_secs(3),connect_timeout:Duration::from_secs(3),..Default::default()}).unwrap();
  Self{bridge,state,worker:Some(worker),_lock:lock}
 }
 fn sent(&self,command:&str)->usize{self.state.lock().unwrap().requests.iter().filter(|r|r["cmd"]==command).count()}
}
impl Drop for Fixture{fn drop(&mut self){let _=self.bridge.transport().call("probe_finish",json!({}));if let Some(worker)=self.worker.take(){worker.join().unwrap();}}}
fn pending(topup:bool)->PendingReq{PendingReq{kind:PendingKind::BuyLimit,volume:0.02,price:2090.,sl:Some(2080.),tp:Some(2120.),basket:Some(1),level:0,is_toucher:false,is_topup:topup,no_market_fallback:false,comment:"B1".into()}}
fn market()->OrderReq{OrderReq{side:Side::Buy,volume:0.01,sl:Some(2080.),tp:Some(2120.),basket:Some(1),level:0,is_toucher:false,comment:"B1".into()}}

#[test]
fn pending_topup_survives_immediate_cache_poll_and_same_bridge_reconcile(){
 let mut f=Fixture::new(vec![],None);let t=f.bridge.place_pending(pending(true)).unwrap();
 assert!(f.bridge.pendings().iter().find(|p|p.ticket==t).unwrap().is_topup,"immediate ACK lost topup");
 f.bridge.refresh_state().unwrap();assert!(f.bridge.pendings()[0].is_topup,"poll lost topup");
 f.bridge.reconcile().unwrap();assert!(f.bridge.pendings()[0].is_topup,"reconcile lost topup");
}
#[test]
fn fresh_bridge_adopts_full_topup_comment_without_guessing_legacy_base(){
 let raw=|ticket,comment|json!({"ticket":ticket,"kind":2,"volume":0.01,"price_open":2090.,"time_msc":TS,"sl":2080.,"tp":2120.,"magic":777,"symbol":"XAUUSD","comment":comment});
 let f=Fixture::new(vec![raw(201,"CD1.0u!~1!-B1"),raw(202,"CD1.0~2!-B1")],None);
 assert!(f.bridge.pendings().iter().find(|p|p.ticket==201).unwrap().is_topup);
 assert!(!f.bridge.pendings().iter().find(|p|p.ticket==202).unwrap().is_topup);
}
fn unknown_open(code:i64){
 let mut f=Fixture::new(vec![],Some(code));assert!(f.bridge.open_market(market()).is_err());
 assert_eq!(f.sent("open_market"),1,"ambiguous broker result may not resend OPEN");
 assert!(f.bridge.unconfirmed_open().is_some(),"accepted possibility must remain reconcilable");
 let evidence=f.bridge.operation_evidence().unwrap();assert_eq!(evidence.attempts.len(),1);
 assert_eq!(evidence.attempts[0].retcode,Some(code));assert_ne!(format!("{:?}",evidence.outcome),"RemoteRefusal");
 f.bridge.refresh_state().unwrap();assert_eq!(f.bridge.positions().len(),1);assert_eq!(f.sent("open_market"),1);
}
#[test]fn broker_timeout_is_unknown_not_a_retry(){unknown_open(10012);}
#[test]fn broker_connection_is_unknown_not_a_retry(){unknown_open(10031);}
#[test]fn broker_processing_error_is_unknown_not_a_definite_refusal(){unknown_open(10011);}
#[test]
fn ambiguous_pending_send_retains_entry_barrier_and_no_retry(){
 let mut f=Fixture::new(vec![],Some(10012));assert!(f.bridge.place_pending(pending(true)).is_err());
 assert_eq!(f.sent("place_pending"),1);assert!(f.bridge.place_pending(pending(false)).is_err());
 assert_eq!(f.sent("place_pending"),1,"unknown pending must retain shared entry hold");
}

#[test]
fn explicitly_sized_crossed_pending_is_not_repriced_or_sent(){
 let mut f=Fixture::new(vec![],None);
 for (kind,price) in [(PendingKind::BuyLimit,2100.211),(PendingKind::SellLimit,2099.989),
                     (PendingKind::BuyStop,2100.19),(PendingKind::SellStop,2100.01)] {
  let mut r=pending(false);r.kind=kind;r.price=price;r.no_market_fallback=true;
  assert!(f.bridge.place_pending(r).is_err());
 }
 assert_eq!(f.sent("open_market"),0);assert_eq!(f.sent("place_pending"),0);
 assert!(f.bridge.positions().is_empty());assert!(f.bridge.pendings().is_empty());
}

#[test]
fn legacy_crossed_pending_keeps_market_fallback_but_strict_valid_pending_is_accepted(){
 let mut f=Fixture::new(vec![],None);let mut r=pending(false);r.price=2100.21;
 assert!(!r.no_market_fallback);f.bridge.place_pending(r).unwrap();
 assert_eq!(f.sent("open_market"),1);
 let mut valid=pending(true);valid.no_market_fallback=true;let ticket=f.bridge.place_pending(valid).unwrap();
 assert!(f.bridge.pendings().iter().any(|p|p.ticket==ticket&&p.is_topup));
 assert_eq!(f.sent("place_pending"),1);assert_eq!(f.sent("open_market"),1);
}
