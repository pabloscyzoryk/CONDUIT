//! Public GOD-X7 Settings, public Engine -> Widok -> actual Bridge/TCP.
//! Synthetic loopback only: no Python, terminal, Telegram, credentials or broker.
//! Original 2 PASS / 4 safety RED preserved in the timestamped evidence directory.
//! ON tests require a sticky guard; OFF tests explicitly document unsafe compatibility.
use conduit_core::{broker::{Broker, ReceiptBarrier}, engine::{Engine, IncomingMessage},
    routing::{Widok, Wlasnosc}, settings::Settings, types::*};
use conduit_mt5::{Mt5Bridge, SidecarConfig};
use serde_json::{json, Value};
use std::{io::{BufRead, BufReader, Write}, net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, MutexGuard, atomic::{AtomicBool, Ordering}},
    thread, time::{Duration, Instant}};

const T0: i64 = 1_788_177_000_000;
const BALANCE: f64 = 600.0;
const FOREIGN_TICKET: u64 = 990_001;
const FOREIGN_IDENTIFIER: u64 = 99_099_001;
// This regression originated with GOD-X4. GOD-X7 intentionally inherits the
// pending-grid and receipt-safety semantics exercised below (including eight
// entry units), so the public-source test pins the public preset that users
// actually receive instead of depending on an omitted private historical file.
// `settings()` and the order-count assertions below fail loudly if that
// inheritance ever changes.
const PRESET: &str = include_str!("../../../../config/presets/GOD-X7.json");
static PORT_LOCK: Mutex<()> = Mutex::new(());

fn send(s: &mut TcpStream, value: &Value) { writeln!(s, "{value}").unwrap(); s.flush().unwrap(); }
fn raw_position(ticket: u64, identifier: u64, side: Side, volume: f64,
    price: f64, comment: &str, ts: i64, sl: f64, tp: f64) -> Value {
    json!({"ticket":ticket,"identifier":identifier,"kind":if side==Side::Buy{0}else{1},
        "volume":volume,"price_open":price,"time_msc":ts,"sl":sl,"tp":tp,
        "symbol":"XAUUSD","magic":777,"comment":comment})
}

struct Fixture {
    bridge: Mt5Bridge, worker: Option<thread::JoinHandle<()>>, stop: Arc<AtomicBool>,
    _lock: MutexGuard<'static, ()>,
}
impl Fixture {
    fn new(side: Side, receipt_on: bool) -> Self {
        let lock = PORT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let reservation = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = reservation.local_addr().unwrap().port(); drop(reservation);
        let stop = Arc::new(AtomicBool::new(false)); let stop_worker = stop.clone();
        let worker = thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(5);
            let mut stream = loop { match TcpStream::connect(("127.0.0.1", port)) {
                Ok(s) => break s, Err(_) if Instant::now()<until => thread::sleep(Duration::from_millis(5)),
                Err(e) => panic!("loopback connect: {e}"),
            }};
            stream.set_nodelay(true).unwrap();
            stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            send(&mut stream, &json!({"ev":"hello","proto":1,"sidecar":"SYNTHETIC-G4-CANCEL-UNKNOWN"}));
            let mut reader = BufReader::new(stream.try_clone().unwrap()); let mut line=String::new();
            let mut ts=T0; let mut bid=if side==Side::Buy {4012.0}else{3988.0};
            let mut orders=Vec::<Value>::new(); let mut history=Vec::<Value>::new();
            let mut deals=Vec::<Value>::new(); let mut positions=vec![raw_position(
                FOREIGN_TICKET,FOREIGN_IDENTIFIER,Side::Buy,0.01,bid,"CD100001.0",T0-1000,0.0,0.0)];
            let mut trace=Vec::<Value>::new(); let mut seq=0u64;
            let mut placed=0u64; let mut opens=0u64; let mut cancels=0u64;
            let close_calls=0u64; let mut history_reads=0u64;
            let mut drop_cancel_ticket=None::<u64>; let mut unknown_order=None::<Value>;
            let mut disconnect_cancel=false;
            loop {
                line.clear(); match reader.read_line(&mut line) {
                    Ok(0)=>break, Ok(_)=>{},
                    Err(e) if matches!(e.kind(),std::io::ErrorKind::TimedOut|std::io::ErrorKind::WouldBlock)=>{
                        if stop_worker.load(Ordering::Relaxed){break;} continue;
                    }, Err(e)=>panic!("loopback read: {e}"),
                }
                let request:Value=serde_json::from_str(&line).unwrap();
                let command=request["cmd"].as_str().unwrap(); let a=&request["args"];
                eprintln!("G4_FAKE_RPC {side:?} receipt={receipt_on} {command}");
                let mut omit_response=false;
                let result=match command {
                    "account"=>json!({"login":42,"server":"G4-CANCEL-SYNTHETIC-DEMO","trade_mode":0,
                        "balance":BALANCE,"equity":BALANCE,"margin":0.0,"margin_free":BALANCE,
                        "leverage":1000,"currency":"USD"}),
                    "symbol_info"=>json!({"symbol":"XAUUSD","digits":2,"point":0.01,
                        "stops_level_points":0.0,"volume_min":0.01,"volume_max":100.0,
                        "volume_step":0.01,"contract_size":100.0,"trade_mode":4}),
                    "quote"=>json!({"ts":ts,"bid":bid,"ask":bid+0.20}),
                    "positions"=>json!(positions), "orders"=>json!(orders), "subscribe_ticks"=>json!({}),
                    "place_pending"=>{
                        placed+=1; let ticket=700_000+placed; let mut row=a.clone();
                        row["ticket"]=json!(ticket); row["magic"]=json!(777);
                        row["price_open"]=a["price"].clone(); row["time_msc"]=json!(ts);
                        for k in ["sl","tp"] {if row[k].is_null(){row[k]=json!(0.0);}}
                        orders.push(row.clone()); seq+=1;
                        trace.push(json!({"seq":seq,"event":"place_pending","order":ticket,
                            "price":row["price_open"],"volume":row["volume"],"comment":row["comment"],
                            "unknown_old_order":unknown_order.as_ref().map(|o|o["ticket"].clone()),
                            "pending_count":orders.len()}));
                        json!({"retcode":10009,"order":ticket})
                    },
                    "cancel_pending"=>{
                        cancels+=1; let t=a["ticket"].as_u64().unwrap();
                        let row=orders.iter().find(|o|o["ticket"]==t).cloned()
                            .expect("cancel must name a current own pending, never a position");
                        assert_ne!(t,FOREIGN_TICKET,"other engine position cannot become a cancel target");
                        seq+=1;
                        if drop_cancel_ticket==Some(t) {
                            drop_cancel_ticket=None; unknown_order=Some(row.clone()); omit_response=true;
                            // Request reached the server; outcome/response is unavailable.
                            // The old pending still exists and can legally fill later.
                            trace.push(json!({"seq":seq,"event":"cancel_response_lost_order_still_live","order":t}));
                            if disconnect_cancel {
                                eprintln!("G4_FAKE_DISCONNECT_AFTER_CANCEL order={t}");
                                stream.shutdown(std::net::Shutdown::Both).unwrap();
                                break;
                            }
                        } else {
                            orders.retain(|o|o["ticket"]!=t);
                            history.push(json!({"ticket":t,"state":2,"position_id":0,
                                "volume_initial":row["volume"],"volume_current":row["volume"],
                                "time_done_msc":ts,"comment":row["comment"]}));
                            trace.push(json!({"seq":seq,"event":"cancel_ack","order":t}));
                        }
                        json!({"retcode":10009,"order":t})
                    },
                    "open_market"=>{
                        opens+=1; let ticket=800_000+opens; let identifier=ticket+90_000_000;
                        let buy=a["side"]=="buy"; let px=if buy{bid+0.20}else{bid};
                        positions.push(raw_position(ticket,identifier,if buy{Side::Buy}else{Side::Sell},
                            a["volume"].as_f64().unwrap(),px,a["comment"].as_str().unwrap(),ts,
                            a["sl"].as_f64().unwrap_or(0.0),a["tp"].as_f64().unwrap_or(0.0)));
                        seq+=1;trace.push(json!({"seq":seq,"event":"open_market","ticket":ticket,
                            "unknown_old_order":unknown_order.as_ref().map(|o|o["ticket"].clone())}));
                        json!({"retcode":10009,"deal":900_000+opens,"order":ticket,"position":ticket,
                            "position_identifier":identifier,"volume":a["volume"],"price":px})
                    },
                    "modify_position"=>{
                        let t=a["ticket"].as_u64().unwrap();
                        let p=positions.iter_mut().find(|p|p["ticket"]==t).unwrap();
                        for k in ["sl","tp"]{p[k]=a[k].as_f64().map_or(json!(0.0),|v|json!(v));}json!({})
                    },
                    "modify_pending"=>{
                        let t=a["ticket"].as_u64().unwrap();
                        let p=orders.iter_mut().find(|p|p["ticket"]==t).unwrap();
                        p["price_open"]=a["price"].clone();
                        for k in ["sl","tp"]{p[k]=a[k].as_f64().map_or(json!(0.0),|v|json!(v));}json!({})
                    },
                    "close_position"|"close_partial"=>{
                        panic!("unexpected close in this pending-only chronology: {}",a["ticket"]);
                    },
                    "history_orders"|"history_deals"|"pending_reconcile"=>{
                        history_reads+=1;json!({"orders":history,"deals":deals})
                    },
                    "probe_arm_fault"=>{drop_cancel_ticket=Some(a["ticket"].as_u64().unwrap());
                        disconnect_cancel=a["disconnect"].as_bool().unwrap_or(false);json!({})},
                    "probe_quote"=>{ts=a["ts"].as_i64().unwrap();bid=a["bid"].as_f64().unwrap();json!({})},
                    "probe_late_fill"=>{
                        let row=unknown_order.clone().expect("fault must precede delayed fill");
                        let order=row["ticket"].as_u64().unwrap();
                        assert!(orders.iter().any(|o|o["ticket"]==order));
                        orders.retain(|o|o["ticket"]!=order);
                        // Distinct namespaces are intentional: order != deal != physical ticket != identifier.
                        let ticket=880_123u64;let identifier=98_880_123u64;let deal=980_777u64;
                        let buy=row["kind"]==2;let price=row["price_open"].as_f64().unwrap();
                        ts+=1000;bid=if buy{price-0.20}else{price};
                        positions.push(raw_position(ticket,identifier,if buy{Side::Buy}else{Side::Sell},
                            row["volume"].as_f64().unwrap(),price,row["comment"].as_str().unwrap(),ts,
                            row["sl"].as_f64().unwrap_or(0.0),row["tp"].as_f64().unwrap_or(0.0)));
                        history.push(json!({"ticket":order,"state":4,"position_id":identifier,
                            "volume_initial":row["volume"],"volume_current":0.0,"time_done_msc":ts}));
                        deals.push(json!({"ticket":deal,"order":order,"position_id":identifier,
                            "entry":0,"type":if buy{0}else{1},"volume":row["volume"],"price":price,"time_msc":ts}));
                        seq+=1;trace.push(json!({"seq":seq,"event":"late_old_pending_fill",
                            "order":order,"deal":deal,"position_ticket":ticket,"position_identifier":identifier,
                            "price":price,"volume":row["volume"]}));json!({"ticket":ticket,"identifier":identifier,"order":order})
                    },
                    "probe_snapshot"=>json!({"placed":placed,"opens":opens,"cancels":cancels,
                        "close_calls":close_calls,"history_reads":history_reads,"orders":orders,
                        "positions":positions,"history_orders":history,"entry_deals":deals,"trace":trace}),
                    "probe_finish"|"shutdown"=>{send(&mut stream,&json!({"id":request["id"],"ok":true,"result":{}}));break;},
                    other=>panic!("unexpected fake-only RPC: {other}"),
                };
                if !omit_response {send(&mut stream,&json!({"id":request["id"],"ok":true,"result":result}));}
            }
        });
        let bridge=Mt5Bridge::connect(SidecarConfig{autostart:false,port,magic:777,
            close_receipt_reconcile:receipt_on,request_timeout:Duration::from_secs(2),
            connect_timeout:Duration::from_secs(3),restart_backoff:Duration::from_millis(50),
            ..Default::default()}).unwrap();
        Self{bridge,worker:Some(worker),stop,_lock:lock}
    }
    fn call(&self,cmd:&'static str,args:Value)->Value {self.bridge.transport().call(cmd,args).unwrap()}
    fn quote(&mut self,ts:i64,bid:f64){self.call("probe_quote",json!({"ts":ts,"bid":bid}));self.bridge.refresh_quote().unwrap();}
}
impl Drop for Fixture {fn drop(&mut self){let _=self.bridge.transport().call("probe_finish",json!({}));
    self.stop.store(true,Ordering::Relaxed);if let Some(w)=self.worker.take(){w.join().unwrap();}}}

fn settings()->Settings {
    let root:Value=serde_json::from_str(PRESET).unwrap();
    let cfg:Settings=serde_json::from_value(root["settings"].clone()).unwrap();
    assert!(!cfg.entry_edit_geometry_v2 && !cfg.pending_relot_reconcile_target);
    assert!(cfg.confirmed_exit_retry && cfg.basket_realized_broker_only);
    cfg // No strategy-axis overrides: exact typed source preset, plus serde defaults.
}
fn message(side:Side,edit:bool)->IncomingMessage {
    let text=match(side,edit){
        (Side::Buy,false)=>"BUY LIMIT GOLD @ 4005/4000\nTP 4020\nTP 4030\nTP 4040\nSL 3990",
        (Side::Buy,true)=>"BUY LIMIT GOLD @ 4004/3999\nTP 4020\nTP 4030\nTP 4040\nSL 3990",
        (Side::Sell,false)=>"SELL LIMIT GOLD @ 4000/4005\nTP 3980\nTP 3970\nTP 3960\nSL 4015",
        (Side::Sell,true)=>"SELL LIMIT GOLD @ 4001/4006\nTP 3980\nTP 3970\nTP 3960\nSL 4015",
    };
    IncomingMessage{ts:if edit{T0+2000}else{T0},source:SourceKey::new(-990001,None),
        source_name:"G4 synthetic cancel regression".into(),msg_id:1,reply_to:None,
        edit_of:edit.then_some(1),text:text.into()}
}
fn owner()->Wlasnosc{Wlasnosc{slot:0,zapasowy:true,znane_sloty:vec![0,1]}}
fn tick(e:&mut Engine,f:&mut Fixture,waiting:&mut Vec<ClosedTrade>){
    let q=f.bridge.quote();let mut b=Widok::nowy(&mut f.bridge,owner(),waiting);e.on_tick(&mut b,&q);
}
fn run(side:Side,receipt_on:bool,fault:bool){
    let mut f=Fixture::new(side,receipt_on);let mut e=Engine::new(settings(),BALANCE);
    let mut waiting=Vec::new();tick(&mut e,&mut f,&mut waiting);
    {let mut b=Widok::nowy(&mut f.bridge,owner(),&mut waiting);e.on_message(&mut b,&message(side,false));}
    f.quote(T0+1000,if side==Side::Buy{4012.0}else{3988.0});tick(&mut e,&mut f,&mut waiting);
    assert_eq!(e.baskets.len(),1,"initial G4 signal must actually create a basket; rejects={:?}",e.odrzuty);
    let basket=e.baskets[0].id;assert_eq!(e.baskets[0].state,BasketState::Armed);
    let before=f.call("probe_snapshot",json!({}));
    let old=before["orders"].as_array().unwrap().len();
    assert!(old>=2,"fixture needs a real multi-order G4 grid, got {old}; {before}");
    assert_eq!(before["opens"],0,"initial grid must not be filled yet");
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::Clear);
    let foreign=f.bridge.positions().iter().find(|p|p.ticket==FOREIGN_TICKET).unwrap().clone();
    f.quote(T0+2000,if side==Side::Buy{4012.0}else{3988.0});
    if fault {
        // Choose the shallow edge: after the edit moves the grid away, a
        // quote can fill this stale order without crossing any replacement.
        let target=before["orders"].as_array().unwrap().iter().max_by(|a,b| {
            let ord=a["price_open"].as_f64().unwrap().total_cmp(&b["price_open"].as_f64().unwrap());
            if side==Side::Buy {ord}else{ord.reverse()}
        }).unwrap()["ticket"].clone();
        f.call("probe_arm_fault",json!({"ticket":target}));
    }
    {let mut b=Widok::nowy(&mut f.bridge,owner(),&mut waiting);
        assert!(b.positions().is_empty(),"Widok must hide the other engine's actual position");
        e.on_message(&mut b,&message(side,true));}
    let after=f.call("probe_snapshot",json!({}));
    let new_placements=after["placed"].as_u64().unwrap()-before["placed"].as_u64().unwrap();
    let immediate_barrier=format!("{:?}",f.bridge.receipt_barrier());
    let late=if fault {
        let late=f.call("probe_late_fill",json!({}));
        f.bridge.refresh_state().unwrap();
        let t=late["ticket"].as_u64().unwrap();
        assert!(f.bridge.positions().iter().any(|p|p.ticket==t&&p.basket==Some(basket)),
            "late old fill must remain owned/protectable; it is not a canceled order");
        Some(late)
    }else{None};
    let unchanged=f.bridge.positions().iter().find(|p|p.ticket==FOREIGN_TICKET).unwrap();
    assert_eq!(serde_json::to_value(unchanged).unwrap(),serde_json::to_value(&foreign).unwrap(),
        "other engine position must be unchanged by the first engine's edit");
    let mut protective_proof=Value::Null;
    if fault {
        f.bridge.refresh_quote().unwrap();
        tick(&mut e,&mut f,&mut waiting); // Reconcile actual late fill; the entry hold must not stop protection.
        let own_stop=if side==Side::Buy{3991.0}else{4014.0};
        let own_ticket=late.as_ref().unwrap()["ticket"].as_u64().unwrap();
        let protection=IncomingMessage{ts:f.bridge.quote().ts+1,source:SourceKey::new(-990001,None),
            source_name:"G4 synthetic cancel regression".into(),msg_id:2,reply_to:Some(1),
            edit_of:None,text:format!("MOVE SL TO {own_stop}")};
        {let mut b=Widok::nowy(&mut f.bridge,owner(),&mut waiting);e.on_message(&mut b,&protection);}
        assert_eq!(f.bridge.positions().iter().find(|p|p.ticket==own_ticket).unwrap().sl,Some(own_stop),
            "public Engine protection must work despite the cancel entry hold");
        let other_stop=f.bridge.quote().bid-2.0;
        let mut other=Engine::new(settings(),BALANCE);
        let other_basket:Basket=serde_json::from_value(json!({"id":100001,
            "source":SourceKey::new(-990002,None),"source_name":"other synthetic owner","msg_id":100001,
            "side":"Buy","is_limit":false,"entry_lo":foreign.open_price,"entry_hi":foreign.open_price,
            "zone_lo":foreign.open_price,"zone_hi":foreign.open_price,"sl":null,"tps":[],
            "tp_stage":0,"created_ts":T0-1000,"state":"Working","tickets":[FOREIGN_TICKET],
            "pendings":[],"realized":0.0,"events":[],"had_positions":true})).unwrap();
        other.adopt_baskets(vec![other_basket]);
        let protect_other=IncomingMessage{ts:f.bridge.quote().ts+2,source:SourceKey::new(-990002,None),
            source_name:"other synthetic owner".into(),msg_id:100002,reply_to:Some(100001),edit_of:None,
            text:format!("MOVE SL TO {other_stop}")};
        {let mut b=Widok::nowy(&mut f.bridge,Wlasnosc{slot:1,zapasowy:false,znane_sloty:vec![0,1]},&mut waiting);
            other.on_message(&mut b,&protect_other);}
        assert_eq!(f.bridge.positions().iter().find(|p|p.ticket==FOREIGN_TICKET).unwrap().sl,Some(other_stop),
            "the other owner must retain its own protective authority");
        assert_eq!(f.bridge.receipt_barrier(),if receipt_on {ReceiptBarrier::RequiresReview}else{ReceiptBarrier::Clear});
        if receipt_on {
            assert!(f.bridge.close_receipt_issue().is_some(),"refresh and SL must not erase unknown cancel");
        }
        assert!(e.baskets[0].entry_edit_state.as_ref().unwrap().review.is_some(),
            "local edit review remains sticky even when the optional receipt pipeline is off");
        protective_proof=json!({"public_engine_own_stop":own_stop,"public_engine_other_stop":other_stop,
            "other_owner_authorized_only_after_original_edit":true,"sticky_review":true});
    }
    let final_wire=f.call("probe_snapshot",json!({}));
    let own_pending_volume:f64=f.bridge.pendings().iter().filter(|p|p.basket==Some(basket)).map(|p|p.volume).sum();
    let own_open_volume:f64=f.bridge.positions().iter().filter(|p|p.basket==Some(basket)).map(|p|p.volume).sum();
    let basket_snapshot=serde_json::to_value(&e.baskets[0]).unwrap();
    let journal=e.drain_journal();
    let record=json!({"case":format!("{side:?}"),"fault":fault,"receipt_on":receipt_on,
        "strategy_overrides":{},"source_preset":"GOD-X4-LIVE DE5867056331BB4C...",
        "before":before,"after_edit":after,"after_late_fill":final_wire,
        "new_placements_before_any_history_proof":new_placements,
        "barrier_after_edit":immediate_barrier,"barrier_after_late_snapshot":format!("{:?}",f.bridge.receipt_barrier()),
        "unknown_sends":f.bridge.unknown_sends,"late_fill":late,
        "protective_proof":protective_proof,
        "owned_pending_volume":own_pending_volume,"owned_open_volume":own_open_volume,
        "basket":basket_snapshot,"journal":journal});
    println!("G4_CANCEL_TRACE={record}");
    assert_eq!(final_wire["close_calls"],0);
    assert_eq!(final_wire["history_reads"],0,"the current bridge has no typed pending evidence RPC");
    if fault {
        assert_eq!(f.bridge.unknown_sends,1,"test must exercise actual Transport timeout");
        assert_eq!(new_placements,0,
            "unknown cancellation must not send replacement with either receipt mode");
        assert_eq!(final_wire["placed"],before["placed"],"later tick/protection must not replay replacement");
        assert!(e.baskets[0].entry_edit_state.as_ref().unwrap().review.is_some());
        if receipt_on {
            assert_eq!(immediate_barrier,"RequiresReview");
            assert_eq!(final_wire["placed"],before["placed"],"later tick/protection must not replay replacement");
        } else {
            assert_eq!(immediate_barrier,"Clear","OFF leaves the optional global receipt pipeline off; local basket review supplies the hold");
        }
    }else{
        assert_eq!(after["cancels"].as_u64().unwrap() as usize,old);
        assert!(new_placements>0,"no-fault control must exercise actual cancel -> replacement");
        assert!(final_wire["orders"].as_array().unwrap().iter().all(|n|
            !before["orders"].as_array().unwrap().iter().any(|o|o["ticket"]==n["ticket"])));
        assert_eq!(f.bridge.unknown_sends,0);
    }
}

#[test]fn g4_buy_edit_no_fault_control(){run(Side::Buy,true,false);}
#[test]fn g4_sell_edit_no_fault_control(){run(Side::Sell,true,false);}
#[test]fn g4_buy_cancel_timeout_receipt_on_must_not_replace(){run(Side::Buy,true,true);}
#[test]fn g4_sell_cancel_timeout_receipt_on_must_not_replace(){run(Side::Sell,true,true);}
#[test]fn g4_buy_cancel_timeout_receipt_off_requires_basket_review(){run(Side::Buy,false,true);}
#[test]fn g4_sell_cancel_timeout_receipt_off_requires_basket_review(){run(Side::Sell,false,true);}

#[test]
fn g4_cancel_disconnect_after_send_requires_explicit_sticky_review(){
    let mut f=Fixture::new(Side::Buy,true);let mut e=Engine::new(settings(),BALANCE);
    let mut waiting=Vec::new();tick(&mut e,&mut f,&mut waiting);
    {let mut b=Widok::nowy(&mut f.bridge,owner(),&mut waiting);e.on_message(&mut b,&message(Side::Buy,false));}
    f.quote(T0+1000,4012.0);tick(&mut e,&mut f,&mut waiting);
    let before=f.call("probe_snapshot",json!({}));
    assert_eq!(before["orders"].as_array().unwrap().len(),8);
    let target=before["orders"].as_array().unwrap().last().unwrap()["ticket"].clone();
    f.call("probe_arm_fault",json!({"ticket":target,"disconnect":true}));
    {let mut b=Widok::nowy(&mut f.bridge,owner(),&mut waiting);e.on_message(&mut b,&message(Side::Buy,true));}
    println!("G4_DISCONNECT_TRACE={}",json!({"connected":f.bridge.transport().is_connected(),
        "issue":f.bridge.close_receipt_issue(),"barrier":format!("{:?}",f.bridge.receipt_barrier()),
        "unknown_sends":f.bridge.unknown_sends,"cached_pending_count":f.bridge.pendings().len()}));
    assert!(!f.bridge.transport().is_connected());
    assert_eq!(f.bridge.receipt_barrier(),ReceiptBarrier::RequiresReview);
    assert!(f.bridge.close_receipt_issue().is_some(),
        "G4_CANCEL_DISCONNECT_UNLATCHED: disconnected is not a durable receipt fault for a request already sent");
}
