//! B9: Mt5Bridge + Transport + public Engine API.
//! Synthetic loopback only. No terminal, Python, Telegram or network broker.
//! Exact close amounts/open prices, bounded delivery chronology, NOT a full tick replay.
use conduit_core::{broker::{Broker,ReceiptBarrier}, engine::{Engine,IncomingMessage}, settings::{Settings,RiskFreeMode,PendingCrossPolicy},
    journal::EventKind, types::*};
use conduit_mt5::{Mt5Bridge,SidecarConfig};
use serde_json::{json,Value};
use std::{io::{BufRead,BufReader,Write},net::{TcpListener,TcpStream},sync::{Arc,Mutex,MutexGuard,
    atomic::{AtomicBool,Ordering}},thread,time::{Duration,Instant}};

const RECEIVED: i64=1_700_000_000_000;
const TICK: i64=RECEIVED+1_000;
const TICK_BID:f64=2100.00;
const TICK_ASK:f64=2100.20;
const INITIAL_BALANCE:f64=1000.00;
const PROFITS:[f64;5]=[-5.00,-4.00,-3.00,-2.00,1.00];
const TICKETS:[u64;6]=[8_000_006,8_000_005,8_000_004,8_000_003,8_000_002,8_000_001];
// Deliberately synthetic, uneven lower spacing keeps three levels above the
// current ask, two unfilled replacement levels below it and one retained
// runner nearest the risk-free reference.
const OPENS:[f64;6]=[2103.00,2102.00,2101.00,2100.00,2099.50,2099.00];
static PORT_LOCK:Mutex<()>=Mutex::new(());
fn near(a:f64,b:f64){assert!((a-b).abs()<1e-7,"{a} != {b}");}
fn account()->Value{json!({"login":42,"server":"RF-B9-SYNTHETIC-DEMO","trade_mode":0})}
fn write(s:&mut TcpStream,v:&Value){writeln!(s,"{}",v).unwrap();s.flush().unwrap();}
fn position(ticket:u64,level:i32,open:f64,volume:f64)->Value{
    json!({"ticket":ticket,"identifier":ticket+90_000_000_000u64,"kind":0,"volume":volume,
        "price_open":open,"time_msc":RECEIVED-60_000,"sl":2090.0,"tp":2120.0,
        "magic":777,"symbol":"XAUUSD","comment":format!("CD9.{level}")})
}
struct Fixture{bridge:Mt5Bridge,worker:Option<thread::JoinHandle<()>>,stop:Arc<AtomicBool>,_lock:MutexGuard<'static,()>}
impl Fixture{
    fn new(on:bool,one_partial_position:bool)->Self{
        let lock=PORT_LOCK.lock().unwrap_or_else(|e|e.into_inner());
        let reservation=TcpListener::bind(("127.0.0.1",0)).unwrap();
        let port=reservation.local_addr().unwrap().port();drop(reservation);
        let stop=Arc::new(AtomicBool::new(false));let worker_stop=stop.clone();
        let worker=thread::spawn(move||{
            let deadline=Instant::now()+Duration::from_secs(5);
            let mut stream=loop{match TcpStream::connect(("127.0.0.1",port)){
                Ok(s)=>break s,Err(_)if Instant::now()<deadline=>thread::sleep(Duration::from_millis(5)),
                Err(e)=>panic!("synthetic fixture connect: {e}")}};
            stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            write(&mut stream,&json!({"ev":"hello","proto":1,"ready":true,"sidecar":"SYNTHETIC-RF-B9"}));
            let mut reader=BufReader::new(stream.try_clone().unwrap());let mut line=String::new();
            let mut positions:Vec<Value>=if one_partial_position{vec![position(TICKETS[5],1,OPENS[5],0.30)]}
                else{(0..6).map(|i|position(TICKETS[i],6-i as i32,OPENS[i],0.05)).collect()};
            let mut orders=Vec::<Value>::new();let mut undelivered=Vec::<Value>::new();let mut delivered=Vec::<Value>::new();
            let mut balance=INITIAL_BALANCE;let mut closes=0usize;let mut opens=0usize;let mut placed=0usize;
            loop{
                line.clear();match reader.read_line(&mut line){Ok(0)=>break,Ok(_)=>{},
                    Err(e)if matches!(e.kind(),std::io::ErrorKind::TimedOut|std::io::ErrorKind::WouldBlock)=>{
                        if worker_stop.load(Ordering::Relaxed){break;}continue;},Err(e)=>panic!("fixture read: {e}")};
                let req:Value=serde_json::from_str(&line).unwrap();let args=&req["args"];
                let result=match req["cmd"].as_str().unwrap(){
                    "account"=>{let float:f64=positions.iter().map(|p|(TICK_BID-p["price_open"].as_f64().unwrap())*
                        p["volume"].as_f64().unwrap()*100.).sum();json!({"login":42,"server":"RF-B9-SYNTHETIC-DEMO",
                        "trade_mode":0,"balance":balance,"equity":balance+float,"margin":0.0,"margin_free":balance+float,
                        "leverage":1000,"currency":"USD"})},
                    "symbol_info"=>json!({"symbol":"XAUUSD","digits":2,"point":0.01,"stops_level_points":0.0,
                        "volume_min":0.01,"volume_max":100.0,"volume_step":0.01,"contract_size":100.0,"trade_mode":4}),
                    "quote"=>json!({"bid":TICK_BID,"ask":TICK_ASK,"ts":TICK}),
                    "positions"=>json!(positions),"orders"=>json!(orders),"subscribe_ticks"=>json!({}),
                    "close_position"|"close_partial"=>{
                        let ticket=args["ticket"].as_u64().unwrap();let i=positions.iter().position(|p|p["ticket"]==ticket).unwrap();
                        let old=positions[i].clone();let before=old["volume"].as_f64().unwrap();
                        let cut=args["volume"].as_f64().unwrap_or(before);assert!(cut>0.&&cut<=before+1e-9);
                        let profit=PROFITS[closes];let deal=900_000+closes as u64;closes+=1;
                        let px=old["price_open"].as_f64().unwrap()+profit/(cut*100.);
                        if before-cut<1e-9{positions.remove(i);}else{positions[i]["volume"]=json!(before-cut);}
                        balance+=profit;
                        undelivered.push(json!({"ev":"closed","account":account(),"deal":deal,
                            "position":old["identifier"],"deal_type":1,"volume":cut,"price":px,
                            "time_msc":TICK+1,"profit":profit,"commission":0.0,"swap":0.0,"reason":3,
                            "magic":777,"comment":"close","symbol":"XAUUSD","price_open":old["price_open"],
                            "time_open_msc":old["time_msc"]}));
                        // Live SendResult has no guaranteed profit. Never inject known close PnL into the ACK.
                        json!({"retcode":10009,"deal":deal,"position":ticket,"position_identifier":old["identifier"],
                            "volume":cut,"price":px})
                    },
                    "modify_position"=>{let ticket=args["ticket"].as_u64().unwrap();
                        let p=positions.iter_mut().find(|p|p["ticket"]==ticket).unwrap();
                        for key in ["sl","tp"]{p[key]=args[key].as_f64().map_or(json!(0.0),|v|json!(v));}json!({})},
                    "open_market"=>{let ticket=8_100_000+opens as u64;opens+=1;
                        let vol=args["volume"].as_f64().unwrap();let mut p=position(ticket,0,TICK_ASK,vol);
                        p["comment"]=args["comment"].clone();p["sl"]=args["sl"].clone();p["tp"]=args["tp"].clone();
                        p["time_msc"]=json!(TICK);let identifier=p["identifier"].clone();positions.push(p);
                        json!({"retcode":10009,"deal":1_000_000+opens as u64,"order":ticket,"position":ticket,
                            "position_identifier":identifier,"volume":vol,"price":TICK_ASK})},
                    "place_pending"=>{let ticket=1_900_000_000+placed as u64;placed+=1;let mut p=args.clone();
                        p["ticket"]=json!(ticket);p["magic"]=json!(777);p["time_msc"]=json!(TICK);
                        p["price_open"]=args["price"].clone();for key in["sl","tp"]{if p[key].is_null(){p[key]=json!(0.0);}}
                        orders.push(p);json!({"retcode":10009,"order":ticket})},
                    "cancel_pending"=>{orders.retain(|p|p["ticket"]!=args["ticket"]);json!({})},
                    "modify_pending"=>json!({}),
                    "probe_emit"=>{if args["reverse"]==true{undelivered.reverse();}
                        let count=args["count"].as_u64().map(|v|v as usize).unwrap_or(undelivered.len()).min(undelivered.len());
                        for f in undelivered.drain(..count){write(&mut stream,&f);delivered.push(f);}json!({})},
                    "probe_duplicate"=>{for f in&delivered{write(&mut stream,f);}json!({})},
                    "probe_stats"=>json!({"closes":closes,"opens":opens,"placed":placed,"undelivered":undelivered.len(),"balance":balance}),
                    "probe_finish"=>{write(&mut stream,&json!({"id":req["id"],"ok":true,"result":{}}));break;},
                    cmd=>panic!("unexpected offline command: {cmd}"),
                };
                write(&mut stream,&json!({"id":req["id"],"ok":true,"result":result}));
            }
        });
        let bridge=Mt5Bridge::connect(SidecarConfig{autostart:false,port,magic:777,close_receipt_reconcile:on,
            request_timeout:Duration::from_secs(3),connect_timeout:Duration::from_secs(3),
            restart_backoff:Duration::from_millis(50),..Default::default()}).unwrap();
        assert_eq!(bridge.positions().len(),if one_partial_position{1}else{6});
        assert!(bridge.positions().iter().all(|p|p.basket==Some(9)));
        Self{bridge,worker:Some(worker),stop,_lock:lock}
    }
    fn call(&self,cmd:&'static str,args:Value)->Value{self.bridge.transport().call(cmd,args).unwrap()}
    fn emit(&mut self,count:usize,reverse:bool){self.call("probe_emit",json!({"count":count,"reverse":reverse}));self.bridge.poll_state();}
}
impl Drop for Fixture{fn drop(&mut self){let _=self.bridge.transport().call("probe_finish",json!({}));
    self.stop.store(true,Ordering::Relaxed);if let Some(worker)=self.worker.take(){worker.join().unwrap();}}}

fn engine(bridge:&mut Mt5Bridge)->Engine{
    let mut cfg=Settings::default();cfg.basket_realized_broker_only=true;cfg.confirmed_exit_retry=true;
    cfg.server_tz_offset_ms=0;cfg.exec_latency_ms=0;cfg.rearm_grid_on_return=false;
    cfg.rearm_min_gap_min=0.;cfg.rearm_min_basket_profit=0.;cfg.rearm_block_after_secured=false;
    cfg.spp_blocks_rearm_when_flat=false;cfg.risk_free_mode=RiskFreeMode::CloseAllKeepNearest;
    cfg.risk_free_runners=1;cfg.risk_free_trail=false;cfg.be_offset=0.;cfg.auto_limit=true;
    cfg.pending_cross_policy=PendingCrossPolicy::Market;cfg.lot_fixed=0.05;
    let mut e=Engine::new(cfg,INITIAL_BALANCE);
    let mut basket:Basket=serde_json::from_value(json!({"id":9,"source":SourceKey::new(1,None),
        "source_name":"SyntheticFormat","msg_id":1001,"side":"Buy","is_limit":true,
        "entry_lo":2095.,"entry_hi":2101.,"zone_lo":2090.,"zone_hi":2103.,"sl":2090.,
        "tps":[2105.,2110.,2120.],"tp_stage":0,"created_ts":RECEIVED-3_000_000,
        "state":"Working","tickets":bridge.positions().iter().map(|p|p.ticket).collect::<Vec<_>>(),
        "pendings":[],"realized":0.,"events":[],"had_positions":true})).unwrap();
    // Six formerly filled levels isolate the observed five replacement requests.
    // Unrelated B5 and the seventh original pending are intentionally out of scope.
    basket.levels=(0..6).rev().map(|i|GridLevel{price:OPENS[i],base_units:1,volume:0.05,
        sl:Some(2090.),tp:Some(2120.),level:6-i as i32,is_toucher:false,
        fill_ts:RECEIVED-60_000,fill_px:OPENS[i],cancelled:false,filled:true}).collect();
    e.adopt_baskets(vec![basket]);let q=bridge.quote();e.on_tick(bridge,&q);
    e.cfg.rearm_grid_on_return=true;e.drain_journal();
    assert!(!e.wejscie_zablokowane(bridge,TICK),"fixture must not start behind an unrelated gate");e
}
fn rf_message()->IncomingMessage{IncomingMessage{ts:RECEIVED,source:SourceKey::new(1,None),
    source_name:"SyntheticFormat".into(),msg_id:1002,reply_to:Some(1001),edit_of:Some(1002),
    text:"RISK FREE 2099".into()}}
fn rf(f:&mut Fixture,e:&mut Engine){e.on_message(&mut f.bridge,&rf_message());
    assert_eq!(f.call("probe_stats",json!({}))["closes"],5,"public RF handler must actually issue five closes");
    assert_eq!(f.bridge.positions().len(),1);assert_eq!(f.bridge.positions()[0].ticket,TICKETS[5]);
    near(f.bridge.positions()[0].profit_usd(&f.bridge.quote()),5.00);near(e.baskets[0].realized,0.);}
fn no_rearm(f:&mut Fixture,e:&mut Engine){let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
    assert_eq!(e.baskets[0].rearms,0,"RF B9: rearm must wait for all close receipts");
    assert_eq!(f.call("probe_stats",json!({}))["opens"],0);assert!(f.bridge.pendings().is_empty());}
fn allocations(e:&mut Engine)->Vec<conduit_core::journal::JournalEvent>{e.drain_journal().into_iter()
    .filter(|ev|ev.kind==EventKind::PositionClosed).collect()}

#[test]
fn b9_rf_public_message_five_ack_then_tick_then_five_receipts_are_owned_and_no_rearm(){
    // Set only for an explicit expected-RED command; production/test source stays identical.
    let on=std::env::var_os("RF_B9_REQUIRE_SAFE_LEGACY").is_none();
    let mut f=Fixture::new(on,false);let mut e=engine(&mut f.bridge);rf(&mut f,&mut e);
    no_rearm(&mut f,&mut e);assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::Temporary);
    assert!(e.wejscie_zablokowane(&f.bridge,TICK));
    f.bridge.refresh_state().unwrap();no_rearm(&mut f,&mut e);
    f.emit(5,false);assert!(e.wejscie_zablokowane(&f.bridge,TICK),"decoded is not consumed");
    no_rearm(&mut f,&mut e);near(e.baskets[0].realized,-13.00);near(f.bridge.account().balance,987.00);
    near(e.baskets[0].realized+f.bridge.positions()[0].profit_usd(&f.bridge.quote()),-8.00);
    let rows=allocations(&mut e);assert_eq!(rows.len(),5);
    for (row,expected) in rows.iter().zip(PROFITS){assert_eq!(row.basket_id,Some(9));assert_eq!(row.msg_id,Some(1001));
        let close=row.close.as_ref().unwrap();assert_eq!(close.reason,"RiskFree");near(close.volume,0.05);near(close.gross,expected);}
    assert!(!f.bridge.close_receipts_pending());assert_eq!(e.stats.trades,5);
    f.call("probe_duplicate",json!({}));f.bridge.poll_state();no_rearm(&mut f,&mut e);
    assert!(allocations(&mut e).is_empty());near(e.baskets[0].realized,-13.00);assert_eq!(e.stats.trades,5);
}

#[test]
fn b9_rf_legacy_control_reproduces_premature_rearm_and_five_ownerless_closes(){
    let mut f=Fixture::new(false,false);let mut e=engine(&mut f.bridge);rf(&mut f,&mut e);
    let q=f.bridge.quote();e.on_tick(&mut f.bridge,&q);
    assert_eq!(e.baskets[0].rearms,1);assert_eq!(f.call("probe_stats",json!({}))["opens"],3);
    assert_eq!(f.bridge.pendings().len(),2);near(e.baskets[0].realized,0.);
    f.emit(5,false);e.on_tick(&mut f.bridge,&q);let rows=allocations(&mut e);
    assert_eq!(rows.len(),5);assert!(rows.iter().all(|row|row.basket_id.is_none()));
    near(rows.iter().map(|row|row.close.as_ref().unwrap().gross).sum(),-13.00);
    near(e.baskets[0].realized,0.);near(f.bridge.account().balance,987.00);
}

#[test]
fn b9_rf_partial_delivery_reversed_order_and_snapshots_do_not_release_rearm_early(){
    for reverse in [false,true]{
        let mut f=Fixture::new(true,false);let mut e=engine(&mut f.bridge);rf(&mut f,&mut e);
        let mut rows=Vec::new();let mut sum=0.;
        for n in 0..5{f.bridge.refresh_state().unwrap();no_rearm(&mut f,&mut e);
            f.emit(1,reverse&&n==0);no_rearm(&mut f,&mut e);let batch=allocations(&mut e);
            assert_eq!(batch.len(),1);assert_eq!(batch[0].basket_id,Some(9));sum+=batch[0].close.as_ref().unwrap().gross;
            rows.extend(batch);near(e.baskets[0].realized,sum);
            assert_eq!(f.bridge.close_receipts_pending(),n<4);
        }
        assert_eq!(rows.len(),5);near(sum,-13.00);assert_eq!(e.stats.trades,5);
    }
}

#[test]
fn five_manual_partials_of_one_position_keep_exact_residual_volume_and_owner(){
    let mut f=Fixture::new(true,true);let mut e=engine(&mut f.bridge);
    for _ in 0..5{f.bridge.close_partial(TICKETS[5],0.05,CloseReason::RiskFree).unwrap();}
    near(f.bridge.positions()[0].volume,0.05);no_rearm(&mut f,&mut e);
    f.bridge.refresh_state().unwrap();near(f.bridge.positions()[0].volume,0.05);no_rearm(&mut f,&mut e);
    f.emit(5,true);no_rearm(&mut f,&mut e);let rows=allocations(&mut e);
    assert_eq!(rows.len(),5);assert!(rows.iter().all(|r|r.basket_id==Some(9)&&r.data["partial"]==true));
    near(e.baskets[0].realized,-13.00);near(f.bridge.positions()[0].volume,0.05);
    f.call("probe_duplicate",json!({}));f.bridge.poll_state();no_rearm(&mut f,&mut e);
    assert!(allocations(&mut e).is_empty());near(f.bridge.positions()[0].volume,0.05);assert_eq!(e.stats.trades,5);
}

#[test]
fn b9_rf_shared_account_router_keeps_five_receipts_barriered_until_owner_consumes(){
    use conduit_core::routing::{Widok,Wlasnosc};
    let mut f=Fixture::new(true,false);let mut e=engine(&mut f.bridge);
    let mut other=Engine::new(e.cfg.clone(),INITIAL_BALANCE);
    let owner=|slot|Wlasnosc{slot,zapasowy:slot==0,znane_sloty:vec![0,1]};
    let mut waiting=Vec::new();let q=f.bridge.quote();
    {
        let mut view=Widok::nowy(&mut f.bridge,owner(0),&mut waiting);
        e.on_message(&mut view,&rf_message());e.on_tick(&mut view,&q);
        assert!(view.close_receipts_pending());assert_eq!(e.baskets[0].rearms,0);
    }
    f.emit(5,false);
    {
        let mut wrong_owner=Widok::nowy(&mut f.bridge,owner(1),&mut waiting);
        other.on_tick(&mut wrong_owner,&q);
        assert!(wrong_owner.close_receipts_pending(),"draining the bridge is not owner credit");
        assert!(other.wejscie_zablokowane(&wrong_owner,TICK));
    }
    assert_eq!(waiting.len(),5);assert_eq!(other.stats.trades,0);near(e.baskets[0].realized,0.);
    {
        let mut actual_owner=Widok::nowy(&mut f.bridge,owner(0),&mut waiting);
        assert!(e.wejscie_zablokowane(&actual_owner,TICK));
        e.on_tick(&mut actual_owner,&q);
        assert!(!actual_owner.close_receipts_pending());
    }
    assert!(waiting.is_empty());near(e.baskets[0].realized,-13.00);assert_eq!(e.baskets[0].rearms,0);
    assert_eq!(e.stats.trades,5);let rows=allocations(&mut e);
    assert_eq!(rows.len(),5);assert!(rows.iter().all(|row|row.basket_id==Some(9)&&row.msg_id==Some(1001)));
    assert_eq!(f.call("probe_stats",json!({}))["opens"],0);assert!(f.bridge.pendings().is_empty());
}
