//! Actual Python-sidecar (mock package) ACK -> real Transport/Bridge -> Engine.
//! No real MetaTrader import, initialize/login, terminal process or order_send.
use conduit_core::{broker::{Broker,OrderReq,ReceiptBarrier},engine::Engine,settings::Settings,types::*};
use conduit_mt5::{Mt5Bridge,SidecarConfig};
use serde_json::{json,Value};
use std::{io::{BufRead,BufReader,Write},net::{TcpListener,TcpStream},path::PathBuf,
    process::Command,sync::{Mutex,MutexGuard},thread,time::{Duration,Instant}};

const TS:i64=1_700_000_000_000;
static PORT_LOCK:Mutex<()>=Mutex::new(());
fn write(s:&mut TcpStream,v:&Value){writeln!(s,"{v}").unwrap();s.flush().unwrap();}
fn account()->Value{json!({"login":42,"server":"exact-ack-demo","trade_mode":0})}
fn req()->OrderReq{OrderReq{side:Side::Buy,volume:0.05,sl:Some(3990.),tp:Some(4030.),
    basket:Some(1),level:0,is_toucher:false,comment:"fixture".into()}}
fn python_ack(variant:&str)->Value{
    let fixture=PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/test_sidecar_exact_ack.py");
    let python=std::env::var("CONDUIT_TEST_PYTHON").unwrap_or_else(|_|"python".into());
    let result=Command::new(python).arg("-B")
        .arg(fixture)
        .arg("--emit-bridge-fixture").arg(variant).output().unwrap();
    assert!(result.status.success(),"fake-only Python failed: {}",String::from_utf8_lossy(&result.stderr));
    serde_json::from_slice(&result.stdout).unwrap()
}
struct Fixture{bridge:Mt5Bridge,worker:Option<thread::JoinHandle<()>>,_lock:MutexGuard<'static,()>}
impl Fixture{
    fn new(variant:&str)->Self{
        let ack=python_ack(variant);
        assert_eq!(ack["close_ack"]["profit"],0.0);
        let lock=PORT_LOCK.lock().unwrap_or_else(|e|e.into_inner());
        let reserve=TcpListener::bind(("127.0.0.1",0)).unwrap();
        let port=reserve.local_addr().unwrap().port();drop(reserve);
        let worker=thread::spawn(move||{
            let deadline=Instant::now()+Duration::from_secs(5);
            let mut stream=loop{match TcpStream::connect(("127.0.0.1",port)){
                Ok(s)=>break s,Err(_)if Instant::now()<deadline=>thread::sleep(Duration::from_millis(5)),
                Err(e)=>panic!("fake connector: {e}"),}};
            write(&mut stream,&json!({"ev":"hello","proto":1,"ready":true,"sidecar":"FAKE-PYTHON-EXACT-ACK"}));
            let mut reader=BufReader::new(stream.try_clone().unwrap());let mut line=String::new();
            let mut live=false;let mut closed=false;let mut comment=String::new();
            loop{
                line.clear();if reader.read_line(&mut line).unwrap()==0{break;}
                let r:Value=serde_json::from_str(&line).unwrap();let cmd=r["cmd"].as_str().unwrap();
                let result=match cmd{
                    "account"=>{let b=if closed{1006.4}else{1000.};json!({"login":42,"server":"exact-ack-demo",
                        "trade_mode":0,"currency":"USD","balance":b,"equity":b,"margin":0.,"margin_free":b,"leverage":1000})},
                    "symbol_info"=>json!({"symbol":"XAUUSD","digits":2,"point":0.01,"stops_level_points":0.,
                        "volume_min":0.01,"volume_max":100.,"volume_step":0.01,"contract_size":100.,"trade_mode":4}),
                    "quote"=>json!({"bid":4000.2,"ask":4000.2,"ts":TS}),
                    "positions"=>if live{json!([{"ticket":922,"identifier":433,"kind":0,"volume":0.05,
                        "price_open":4000.,"time_msc":TS,"sl":3990.,"tp":4030.,"magic":770077,
                        "comment":comment,"symbol":"XAUUSD"}])}else{json!([])},
                    "orders"=>json!([]),"subscribe_ticks"=>json!({}),
                    "open_market"=>{live=true;comment=r["args"]["comment"].as_str().unwrap().into();ack["open_ack"].clone()},
                    "close_position"|"close_partial"=>{live=false;closed=true;ack["close_ack"].clone()},
                    "probe_emit"=>{
                        write(&mut stream,&json!({"ev":"closed","account":account(),"deal":881,"position":433,
                            "deal_type":1,"volume":0.05,"price":4002.,"time_msc":TS+1000,"profit":7.,
                            "commission":-0.2,"swap":-0.3,"reason":3,"magic":770077,"comment":"close",
                            "symbol":"XAUUSD","price_open":4000.,"time_open_msc":TS}));json!({})},
                    "probe_finish"=>json!({}),_=>panic!("unexpected fixture RPC {cmd}"),
                };
                write(&mut stream,&json!({"id":r["id"],"ok":true,"result":result}));
                if cmd=="probe_finish"{break;}
            }
        });
        let bridge=Mt5Bridge::connect(SidecarConfig{autostart:false,port,magic:770077,
            close_receipt_reconcile:true,request_timeout:Duration::from_secs(3),
            connect_timeout:Duration::from_secs(3),..Default::default()}).unwrap();
        Self{bridge,worker:Some(worker),_lock:lock}
    }
    fn emit(&mut self){self.bridge.transport().call("probe_emit",json!({})).unwrap();self.bridge.poll_state();}
}
impl Drop for Fixture{
    fn drop(&mut self){let _=self.bridge.transport().call("probe_finish",json!({}));
        if let Some(t)=self.worker.take(){t.join().unwrap();}}
}
fn engine(broker_only:bool)->Engine{
    let cfg=Settings{basket_realized_broker_only:broker_only,confirmed_exit_retry:true,
        close_receipt_reconcile:true,server_tz_offset_ms:0,exec_latency_ms:0,
        rearm_grid_on_return:false,swap_enabled:false,..Settings::default()};
    let mut e=Engine::new(cfg,1000.);
    let basket:Basket=serde_json::from_value(json!({"id":1,"source":SourceKey::new(1,None),
        "source_name":"exact-ack-fixture","msg_id":1,"side":"Buy","is_limit":false,
        "entry_lo":4000.,"entry_hi":4000.,"zone_lo":4000.,"zone_hi":4000.,"sl":3990.,
        "tps":[4030.],"tp_stage":0,"created_ts":TS-1000,"state":"Working","tickets":[922],
        "pendings":[],"realized":0.,"events":[],"had_positions":true})).unwrap();
    e.adopt_baskets(vec![basket]);e
}
#[test]
fn actual_python_ack_then_delayed_receipt_is_owned_and_credited_once_for_both_consumer_modes(){
    for broker_only in [false,true]{
        let mut f=Fixture::new("complete");
        assert_eq!(f.bridge.open_market(req()).unwrap(),922);
        assert_eq!(f.bridge.position_identifier(922),Some(433));
        let mut e=engine(broker_only);let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
        e.close_everything(&mut f.bridge,TS+1,CloseReason::Manual);
        assert_eq!(e.baskets[0].realized,0.0,"ACK must never book realized PnL");
        assert!(f.bridge.close_receipts_pending());assert!(f.bridge.open_market(req()).is_err());
        let pending = f.bridge.operation_evidence().unwrap();
        assert_eq!(pending.outcome, conduit_mt5::operation_evidence::Outcome::LocalReceiptPending);
        assert!(pending.attempts.is_empty(), "local deferred entry never reaches transport");
        f.emit();assert!(f.bridge.close_receipts_pending(),"decoded is not owner-consumed");
        e.on_tick(&mut f.bridge,&q);
        assert_eq!(e.baskets[0].realized,9.7,"confirmed price movement plus swap is booked once for strategy; broker gross remains 7");
        f.emit();e.on_tick(&mut f.bridge,&q);
        assert_eq!(e.baskets[0].realized,9.7,"duplicate receipt cannot book strategy profit again");
        assert_eq!(f.bridge.account().balance,1006.4,"cash remains broker truth, not ACK components");
    }
}
#[test]
fn actual_python_unknown_open_is_not_retried_and_exact_snapshot_recovers_hold(){
    let mut f=Fixture::new("unknown_open");
    assert!(f.bridge.open_market(req()).is_err());
    let ack = f.bridge.operation_evidence().unwrap();
    assert_eq!(ack.outcome, conduit_mt5::operation_evidence::Outcome::ConfirmationPending);
    assert_eq!(ack.attempts.len(), 1);
    assert_eq!(ack.attempts[0].retcode, Some(10009));
    assert!(ack.attempts[0].acknowledgement.get("position").is_some());
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::RequiresReview);
    assert!(f.bridge.positions().is_empty(),"unproven ACK cannot invent owned position");
    let unknown_after_dispatch=f.bridge.unknown_sends;
    assert!(f.bridge.open_market(req()).is_err());
    assert_eq!(f.bridge.unknown_sends,unknown_after_dispatch,
        "receipt gate must reject locally, never dispatch the OPEN twice");
    assert_eq!(f.bridge.operation_evidence().unwrap().outcome,
        conduit_mt5::operation_evidence::Outcome::LocalReceiptReview);
    assert!(f.bridge.operation_evidence().unwrap().attempts.is_empty());

    // The fixture exposes, on the next authoritative positions snapshot, the
    // order that MT5 really accepted despite the incomplete/Rejected ACK.
    // Full machine identity (magic+symbol+side+basket+level+comment+volume and
    // stable POSITION_IDENTIFIER) is required before the recoverable hold is
    // released; absence alone would not prove rejection and cannot release it.
    f.bridge.refresh_state().unwrap();
    assert_eq!(f.bridge.positions().len(),1);
    let p=&f.bridge.positions()[0];
    assert_eq!((p.ticket,p.basket,p.level,p.volume),(922,Some(1),0,0.05));
    assert_eq!(f.bridge.position_identifier(922),Some(433));
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::Clear,
        "one exact broker snapshot must recover the hold without resend");
}
#[test]
fn actual_python_unknown_close_waits_for_exact_receipt_before_reenabling_entries(){
    let mut f=Fixture::new("unknown_close");f.bridge.open_market(req()).unwrap();
    assert_eq!(f.bridge.close_position(922,CloseReason::Manual).unwrap(),0.0);
    // PUPrime can omit POSITION_IDENTIFIER even for an executed close.  The
    // accepted ACK may update the physical-position cache, but it is not an
    // economic receipt: new risk remains blocked until the exact history deal
    // proves the stable identifier and is consumed.
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::Temporary);
    assert!(f.bridge.open_market(req()).is_err(),"temporary receipt barrier must block new risk");
    f.emit();
    let closed=f.bridge.drain_closed();
    assert_eq!(closed.len(),1,"exact delayed deal must settle exactly one close");
    assert_eq!(closed[0].ticket,922);assert_eq!(closed[0].basket,Some(1));
    assert_eq!(closed[0].profit,7.0);
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::Clear);
}
#[test]
fn actual_python_conflicting_open_geometry_holds_but_close_waits_for_exact_deal(){
    {
        let mut f=Fixture::new("geometry_open");
        assert!(f.bridge.open_market(req()).is_err());
        assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::RequiresReview);
        assert!(f.bridge.positions().is_empty());
    }
    {
        let mut f=Fixture::new("geometry_close");f.bridge.open_market(req()).unwrap();
        assert_eq!(f.bridge.close_position(922,CloseReason::Manual).unwrap(),0.0);
        // The immediate sidecar ACK cannot prove the close geometry, so the
        // Bridge treats it like PUPrime's missing-identifier ACK: temporary,
        // at-most-once, and blocked until the independently delivered exact
        // history deal arrives.  This is intentionally different from an
        // unknown OPEN, where identity cannot be recovered without risking an
        // invented owned position and therefore RequiresReview remains right.
        assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::Temporary);
        assert!(f.bridge.open_market(req()).is_err(),"no entry before exact close receipt");
        f.emit();
        let closed=f.bridge.drain_closed();
        assert_eq!(closed.len(),1);assert_eq!(closed[0].ticket,922);
        assert_eq!(closed[0].profit,7.0);
        assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::Clear);
    }
}
