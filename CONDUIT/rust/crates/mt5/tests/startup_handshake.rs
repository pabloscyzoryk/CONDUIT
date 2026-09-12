//! Real loopback transport and the production Python loop with an offline MT5 stub.
//! No terminal, account login, external network or trading API is used.
use conduit_mt5::{SidecarConfig, transport::{CallError, Transport}};
use serde_json::{json, Value};
use std::{io::{BufRead, BufReader, Write}, net::TcpStream, path::PathBuf,
    thread, time::Duration};

fn config() -> SidecarConfig {
    SidecarConfig { autostart: false, connect_timeout: Duration::from_millis(500),
        request_timeout: Duration::from_millis(100), restart_backoff: Duration::from_millis(10),
        ..Default::default() }
}
fn send(stream: &mut TcpStream, value: Value) {
    writeln!(stream, "{value}").unwrap(); stream.flush().unwrap();
}

#[test]
fn bare_tcp_is_not_terminal_readiness_and_cannot_send_orders() {
    let mut transport = Transport::start(config()).unwrap();
    let mut peer = TcpStream::connect(("127.0.0.1", transport.local_port())).unwrap();
    peer.set_read_timeout(Some(Duration::from_millis(120))).unwrap();
    thread::sleep(Duration::from_millis(30));
    assert!(!transport.is_connected(), "TCP accept is not MT5 initialization");
    assert_eq!(transport.call("open_market", json!({})), Err(CallError::Disconnected));
    let mut line = String::new();
    assert!(BufReader::new(peer.try_clone().unwrap()).read_line(&mut line).is_err());
    send(&mut peer, json!({"ev":"hello", "proto":1, "ready":true}));
    assert!(transport.wait_connected(Duration::from_secs(1)));
    drop(peer);
    transport.shutdown();
}

#[test]
fn native_not_initialized_reply_keeps_its_actual_detail() {
    let mut transport = Transport::start(config()).unwrap();
    let port = transport.local_port();
    let peer = thread::spawn(move || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        send(&mut s, json!({"ev":"hello","proto":1,"ready":true}));
        let mut line = String::new();
        BufReader::new(s.try_clone().unwrap()).read_line(&mut line).unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        send(&mut s, json!({"id":request["id"], "ok":false,
            "error":{"code":-1,"msg":"fixture account unavailable"}}));
        thread::sleep(Duration::from_millis(30));
    });
    assert!(transport.wait_connected(Duration::from_secs(1)));
    let error = transport.call("account", Value::Null).unwrap_err();
    assert_eq!(error.retcode(), Some(-1));
    assert!(error.to_string().contains("fixture account unavailable"));
    peer.join().unwrap(); transport.shutdown();
}

fn python_config(mode: &str) -> SidecarConfig {
    SidecarConfig {
        python: std::env::var_os("CONDUIT_TEST_PYTHON").map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("python")),
        script: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/startup_sidecar.py"),
        symbol: mode.into(), connect_timeout: Duration::from_secs(22),
        follow_terminal_account: mode.starts_with("FOLLOW"),
        request_timeout: Duration::from_millis(100), restart_backoff: Duration::from_millis(10),
        ..Default::default()
    }
}

#[test]
fn actual_python_startup_failure_survives_process_exit_and_retry_can_succeed() {
    let error = match Transport::start(python_config("FAIL_INIT")) {
        Ok(mut t) => { t.shutdown(); panic!("failed initialize must not return a ready transport") },
        Err(e) => e.to_string(),
    };
    assert!(error.contains("fixture initialize rejected"), "{error}");
    let mut t = Transport::start(python_config("READY")).unwrap();
    assert!(t.is_connected());
    assert_eq!(t.call("ping", Value::Null).unwrap()["pong"], true);
    t.shutdown();
}

#[test]
fn actual_python_slow_init_exceeds_rpc_and_ready_keepalive_deadlines() {
    let mut t = Transport::start(python_config("SLOW_INIT")).unwrap();
    assert!(t.is_connected());
    assert_eq!(t.call("ping", Value::Null).unwrap()["pong"], true);
    t.shutdown();
}

#[test]
fn actual_python_no_terminal_then_terminal_and_terminal_before_bot() {
    let error = match Transport::start(python_config("FOLLOW_ABSENT")) {
        Ok(mut t) => { t.shutdown(); panic!("no terminal must not initialize") },
        Err(e) => e.to_string(),
    };
    assert!(error.contains("terminal_discovery"), "{error}");
    assert!(error.contains("MT5 nie jest uruchomiony"), "{error}");
    for _ in 0..2 {
        let mut t = Transport::start(python_config("FOLLOW_READY")).unwrap();
        assert!(t.is_connected());
        assert_eq!(t.call("ping", Value::Null).unwrap()["pong"], true);
        t.shutdown();
    }
}

#[test]
fn startup_python_exit_before_tcp_is_reported_without_waiting_connect_timeout() {
    let mut cfg = python_config("IMPORT_FAIL");
    cfg.password = Some("fixture-password".into());
    let started = std::time::Instant::now();
    let error = match Transport::start(cfg) {
        Ok(mut t) => { t.shutdown(); panic!("missing script cannot become ready") },
        Err(e) => e.to_string(),
    };
    assert!(error.contains("python_exit"), "{error}");
    assert!(error.contains("ImportError: synthetic unavailable dependency"), "{error}");
    assert!(error.contains("[redacted]"), "{error}");
    assert!(!error.contains("fixture-password"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn transparent_retry_does_not_treat_previous_failure_as_current_failure() {
    let mut t = Transport::start(config()).unwrap();
    let mut first = TcpStream::connect(("127.0.0.1", t.local_port())).unwrap();
    send(&mut first, json!({"ev":"startup_error","stage":"initialize","code":-1,"msg":"first attempt"}));
    assert!(t.wait_ready(Duration::from_secs(1)).is_err());
    drop(first);
    let mut second = TcpStream::connect(("127.0.0.1", t.local_port())).unwrap();
    send(&mut second, json!({"ev":"keepalive"}));
    thread::sleep(Duration::from_millis(80));
    send(&mut second, json!({"ev":"hello","proto":1,"ready":true}));
    for _ in 0..50 {
        if t.is_connected() { break; }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(t.is_connected());
    assert!(t.wait_ready(Duration::from_millis(1)).unwrap());
    drop(second); t.shutdown();
}

#[test]
fn explicit_startup_error_reaches_caller_and_legacy_failed_hello_never_qualifies() {
    for hello in [
        json!({"ev":"startup_error","stage":"symbol","code":-1,"msg":"fixture contract refused"}),
        json!({"ev":"hello","proto":1,"mt5_version":""}),
        json!({"ev":"hello","proto":99,"ready":true}),
    ] {
        let mut t = Transport::start(config()).unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", t.local_port())).unwrap();
        send(&mut s, hello);
        let error = t.wait_ready(Duration::from_secs(1)).unwrap_err();
        assert!(matches!(error, CallError::Startup(_)));
        assert!(!t.is_connected());
        assert!(t.call("open_market", json!({})).is_err());
        drop(s); t.shutdown();
    }
}
