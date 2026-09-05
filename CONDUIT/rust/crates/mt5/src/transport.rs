//! Transport do sidecara: nadzorowany proces potomny + gniazdo TCP na pętli lokalnej.
//!
//! Podział ról jest odwrotny, niż podpowiada intuicja: **serwerem jest Rust**,
//! a sidecar Pythona łączy się do niego jako klient. Dzięki temu port wybiera
//! system operacyjny (`bind` na porcie 0), nie ma wyścigu „kto pierwszy wstanie"
//! i nie trzeba zgadywać, ile czekać na terminal.
//!
//! Nadzór: osobny wątek trzyma cykl `spawn → accept → pętla czytania → ubij →
//! odczekaj → od nowa`. Kiedy połączenie padnie, wszystkie żądania w locie
//! dostają `Disconnected` NATYCHMIAST, zamiast czekać na timeout. To jest
//! istotne: bot, który przez 30 sekund „myśli", że wysłał zlecenie, jest
//! groźniejszy od bota, który wie, że go nie wysłał.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::Value;
use tracing::{debug, error, info, warn};

use crate::proto::{self, Frame, Hello, ProtoError, RawClosed, RawTick, Request, WireError};

/// Ile kwotowań trzymamy, zanim zaczniemy gubić najstarsze.
const TICK_BUFFER: usize = 4096;

#[derive(Debug, Clone)]
pub struct SidecarConfig {
    /// interpreter Pythona (musi mieć zainstalowany pakiet `MetaTrader5`)
    pub python: PathBuf,
    /// ścieżka do `mt5_sidecar.py`
    pub script: PathBuf,
    /// czy Rust ma sam uruchamiać proces (false = sidecar startuje ktoś inny)
    pub autostart: bool,
    /// adres nasłuchu; ZAWSZE pętla lokalna — nigdy 0.0.0.0
    pub host: String,
    /// 0 = niech port wybierze system
    pub port: u16,

    /// katalog z `terminal64.exe`; `None` = domyślny terminal z rejestru
    pub terminal_path: Option<PathBuf>,
    pub login: Option<i64>,
    pub password: Option<String>,
    pub server: Option<String>,
    /// Attach only to an already running terminal; never submit stored credentials.
    pub follow_terminal_account: bool,
    /// Explicit consent for REAL in follow mode (DEMO remains the safe default).
    pub allow_real_account: bool,
    /// Opt-in exact in-session close receipt attribution. Separate from login policy.
    pub close_receipt_reconcile: bool,
    /// Versioned complete closed-net receipt producer. Requires reconciliation.
    pub closed_profit_net_costs: bool,

    pub symbol: String,
    pub magic: i64,
    /// Minimalny dystans SL/TP, którego spodziewa się SYMULATOR (z presetu),
    /// w jednostkach ceny. Most porównuje to z wartością z serwera brokera
    /// i przy rozjeździe krzyczy — patrz [`crate::bridge::StopsMismatch`].
    /// `None` = nie sprawdzaj (nikt nie podał wartości odniesienia).
    pub expected_stops_level_price: Option<f64>,
    /// co ile sidecar odpytuje o tick
    pub tick_interval_ms: u64,
    /// co ile sidecar przegląda historię dealów
    pub deal_poll_ms: u64,
    /// dopuszczalne odchylenie ceny przy zleceniu rynkowym, w punktach
    pub deviation_points: u32,

    /// ile czekamy na odpowiedź na pojedyncze żądanie
    pub request_timeout: Duration,
    /// ile czekamy na pierwsze połączenie po starcie procesu
    pub connect_timeout: Duration,
    /// przerwa przed restartem po padzie
    pub restart_backoff: Duration,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        SidecarConfig {
            python: PathBuf::from("python"),
            script: PathBuf::from("crates/mt5/sidecar/mt5_sidecar.py"),
            autostart: true,
            host: "127.0.0.1".to_string(),
            port: 0,
            terminal_path: None,
            login: None,
            password: None,
            server: None,
            follow_terminal_account: false,
            allow_real_account: false,
            close_receipt_reconcile: false,
            closed_profit_net_costs: false,
            symbol: "XAUUSD".to_string(),
            magic: 770_077,
            expected_stops_level_price: None,
            tick_interval_ms: 50,
            deal_poll_ms: 500,
            deviation_points: 30,
            request_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(90),
            restart_backoff: Duration::from_secs(3),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CallError {
    #[error("sidecar niepodłączony")]
    Disconnected,
    #[error("przekroczony czas odpowiedzi ({0:?})")]
    Timeout(Duration),
    #[error("błąd protokołu: {0}")]
    Proto(#[from] ProtoError),
    #[error("broker odmówił: {} ({})", .0.code, .0.msg)]
    Broker(WireError),
    #[error("odpowiedź w złym kształcie: {0}")]
    Shape(String),
}

impl CallError {
    /// Retcode MT5, jeśli odmowa przyszła od brokera.
    pub fn retcode(&self) -> Option<i64> {
        match self {
            CallError::Broker(e) => Some(e.code),
            _ => None,
        }
    }
}

struct Pending {
    tx: mpsc::Sender<Result<Value, WireError>>,
    /// pokolenie połączenia — odpowiedź ze starego połączenia jest bez wartości
    gen: u64,
}

struct Shared {
    cfg: SidecarConfig,
    /// port, który przydzielił system — sidecar dostaje go w argumentach
    local_port: u16,
    writer: Mutex<Option<TcpStream>>,
    pending: Mutex<HashMap<u64, Pending>>,
    ticks: Mutex<VecDeque<RawTick>>,
    closed: Mutex<Vec<RawClosed>>,
    /// Zamknięcia SPOZA bota. Osobna kolejka, bo `closed` karmi statystyki
    /// silnika — wrzucenie tam cudzych transakcji zafałszowałoby wynik bota.
    /// Panel czyta obie i pokazuje je z etykietą źródła.
    foreign_closed: Mutex<Vec<RawClosed>>,
    hello: Mutex<Option<Hello>>,
    next_id: AtomicU64,
    generation: AtomicU64,
    execution_generation: AtomicU64,
    connected: AtomicBool,
    stopping: AtomicBool,
    restarts: AtomicU64,
    /// ile ramek nie dało się sparsować — sygnał rozjazdu wersji
    proto_errors: AtomicU64,
    bound_account: Mutex<Option<Value>>,
    account_changed: AtomicBool,
    resolved_symbol: Mutex<Option<String>>,
    /// Malformed own close frames cannot disappear as a warning under the strict receipt contract.
    unparsed_closed: Mutex<Vec<Value>>,
    receipt_decode_issue: Mutex<Option<String>>,
}

impl Shared {
    /// Zrywa wszystkie żądania w locie. Wołane przy padzie połączenia.
    fn fail_all_pending(&self) {
        let mut p = self.pending.lock();
        for (_, pend) in p.drain() {
            // odbiorcy mogło już nie być (timeout) — to nie jest błąd
            let _ = pend.tx.send(Err(WireError {
                code: proto::local_code::NOT_INITIALIZED,
                msg: "połączenie z sidecarem zerwane".into(),
            }));
        }
    }

    fn push_tick(&self, t: RawTick) {
        let mut q = self.ticks.lock();
        if q.len() >= TICK_BUFFER {
            q.pop_front();
        }
        q.push_back(t);
    }
}

pub struct Transport {
    sh: Arc<Shared>,
    sup: Option<JoinHandle<()>>,
}

impl Transport {
    /// Uruchamia nasłuch, startuje sidecar i czeka na jego powitanie.
    pub fn start(cfg: SidecarConfig) -> anyhow::Result<Transport> {
        let listener = TcpListener::bind((cfg.host.as_str(), cfg.port))?;
        let port = listener.local_addr()?.port();
        info!(port, symbol = %cfg.symbol, "MT5: nasłuch dla sidecara");

        let connect_timeout = cfg.connect_timeout;
        let sh = Arc::new(Shared {
            cfg,
            local_port: port,
            writer: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            ticks: Mutex::new(VecDeque::with_capacity(TICK_BUFFER)),
            closed: Mutex::new(Vec::new()),
            foreign_closed: Mutex::new(Vec::new()),
            hello: Mutex::new(None),
            next_id: AtomicU64::new(1),
            generation: AtomicU64::new(0),
            execution_generation: AtomicU64::new(0),
            connected: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            restarts: AtomicU64::new(0),
            proto_errors: AtomicU64::new(0),
            bound_account: Mutex::new(None),
            account_changed: AtomicBool::new(false),
            resolved_symbol: Mutex::new(None),
            unparsed_closed: Mutex::new(Vec::new()),
            receipt_decode_issue: Mutex::new(None),
        });

        let sh2 = Arc::clone(&sh);
        let sup = thread::Builder::new()
            .name("mt5-sidecar".into())
            .spawn(move || supervise(sh2, listener, port))?;

        // czekamy na pierwsze połączenie — bez tego pierwsze `call` i tak padnie
        let deadline = Instant::now() + connect_timeout;
        while Instant::now() < deadline {
            if sh.connected.load(Ordering::Acquire) {
                return Ok(Transport { sh, sup: Some(sup) });
            }
            thread::sleep(Duration::from_millis(25));
        }
        // nie przerywamy nadzoru — terminal potrafi wstawać minutami; oddajemy
        // uchwyt niepodłączony, a wywołujący zobaczy `Disconnected`
        warn!(
            ?connect_timeout,
            "MT5: sidecar nie zdążył się połączyć, nadzór działa dalej"
        );
        Ok(Transport { sh, sup: Some(sup) })
    }

    pub fn is_connected(&self) -> bool {
        self.sh.connected.load(Ordering::Acquire)
            && !self.sh.account_changed.load(Ordering::Acquire)
    }

    pub fn execution_generation(&self) -> u64 { self.sh.execution_generation.load(Ordering::Acquire) }

    /// Pin survives transparent sidecar restarts. Only a NEW bridge may bind a new account.
    pub fn bind_account(&self, login: i64, server: &str, trade_mode: i32) {
        if self.sh.cfg.follow_terminal_account || self.sh.cfg.close_receipt_reconcile {
            *self.sh.bound_account.lock() = Some(serde_json::json!({
                "login": login, "server": server, "trade_mode": trade_mode
            }));
        }
    }

    pub fn bind_symbol(&self, symbol: &str) {
        *self.sh.resolved_symbol.lock() = Some(symbol.to_string());
    }

    /// Port, na którym czekamy na sidecar (istotny przy `autostart = false`).
    pub fn local_port(&self) -> u16 {
        self.sh.local_port
    }

    pub fn restarts(&self) -> u64 {
        self.sh.restarts.load(Ordering::Relaxed)
    }

    pub fn proto_errors(&self) -> u64 {
        self.sh.proto_errors.load(Ordering::Relaxed)
    }

    pub fn receipt_decode_issue(&self) -> Option<String> {
        self.sh.receipt_decode_issue.lock().clone()
    }

    /// Reader-thread events may arrive between two Engine calls. Keep the entry
    /// barrier up until Bridge has classified them and their owner has consumed
    /// the resulting ledger entries. Foreign events only wait for classification.
    pub fn close_receipts_waiting(&self) -> bool {
        !self.sh.closed.lock().is_empty()
            || !self.sh.foreign_closed.lock().is_empty()
            || !self.sh.unparsed_closed.lock().is_empty()
    }

    pub fn hello(&self) -> Option<Hello> {
        self.sh.hello.lock().clone()
    }

    pub fn config(&self) -> &SidecarConfig {
        &self.sh.cfg
    }

    /// Czeka (do `timeout`) aż sidecar się podłączy.
    pub fn wait_connected(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.is_connected() {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.is_connected()
    }

    /// Synchroniczne żądanie/odpowiedź.
    pub fn call(&self, cmd: &'static str, args: Value) -> Result<Value, CallError> {
        wywolaj(&self.sh, cmd, args)
    }

    /// Typowana wersja `call`.
    pub fn call_as<T: serde::de::DeserializeOwned>(
        &self,
        cmd: &'static str,
        args: Value,
    ) -> Result<T, CallError> {
        let v = self.call(cmd, args)?;
        serde_json::from_value(v).map_err(|e| CallError::Shape(e.to_string()))
    }

    /// Lekki, klonowalny uchwyt do ODCZYTÓW z innego wątku.
    ///
    /// Powstał dla jednej potrzeby: serwer HTTP musi umieć zapytać terminal
    /// o świece, a `Mt5Bridge` należy na wyłączność do pętli handlowej i nie
    /// wolno go stamtąd zabrać. Uchwyt nie daje dostępu do mostu ani do stanu
    /// silnika — daje wyłącznie `call` po tym samym gnieździe.
    ///
    /// Bezpieczeństwo współbieżne wynika z konstrukcji protokołu: żądania są
    /// numerowane, a odpowiedzi trafiają do właściciela numeru. Dwa wątki
    /// pytające naraz nie mogą dostać swoich odpowiedzi na krzyż. Sidecar
    /// obsługuje je po kolei, więc długi odczyt OPÓŹNIA to, co stoi za nim
    /// w kolejce — dlatego liczba świec na żądanie jest ograniczona.
    pub fn handle(&self) -> TransportHandle {
        TransportHandle {
            sh: Arc::clone(&self.sh),
            symbol: self.sh.resolved_symbol.lock().clone().unwrap_or_else(|| self.sh.cfg.symbol.clone()),
        }
    }

    /// Zabiera wszystkie kwotowania odebrane od ostatniego wywołania.
    pub fn drain_ticks(&self) -> Vec<RawTick> {
        let mut q = self.sh.ticks.lock();
        q.drain(..).collect()
    }

    /// Zabiera wszystkie zamknięcia odebrane od ostatniego wywołania.
    pub fn drain_closed(&self) -> Vec<RawClosed> {
        std::mem::take(&mut *self.sh.closed.lock())
    }

    /// Zabiera zamknięcia SPOZA bota (cudzy `magic`). Do prezentacji w panelu —
    /// statystyki silnika ich nie widzą i widzieć nie powinny.
    pub fn drain_foreign_closed(&self) -> Vec<RawClosed> {
        std::mem::take(&mut *self.sh.foreign_closed.lock())
    }

    /// Prosi sidecar o zamknięcie i zatrzymuje nadzór.
    pub fn shutdown(&mut self) {
        self.sh.stopping.store(true, Ordering::Release);
        let _ = self.call("shutdown", Value::Null);
        if let Some(s) = self.sh.writer.lock().as_ref() {
            let _ = s.shutdown(Shutdown::Both);
        }
        if let Some(h) = self.sup.take() {
            let _ = h.join();
        }
    }
}

/// Klonowalny uchwyt do odczytów — patrz [`Transport::handle`].
#[derive(Clone)]
pub struct TransportHandle {
    sh: Arc<Shared>,
    symbol: String,
}

impl TransportHandle {
    pub fn call(&self, cmd: &'static str, args: Value) -> Result<Value, CallError> {
        wywolaj(&self.sh, cmd, args)
    }

    pub fn call_as<T: serde::de::DeserializeOwned>(
        &self,
        cmd: &'static str,
        args: Value,
    ) -> Result<T, CallError> {
        let v = self.call(cmd, args)?;
        serde_json::from_value(v).map_err(|e| CallError::Shape(e.to_string()))
    }

    pub fn is_connected(&self) -> bool {
        self.sh.connected.load(Ordering::Acquire)
            && !self.sh.account_changed.load(Ordering::Acquire)
    }

    /// Symbol, którym handluje silnik — domyślny instrument zapytań.
    pub fn symbol(&self) -> &str {
        &self.symbol
    }
}

/// Wspólne ciało `call` dla [`Transport`] i [`TransportHandle`].
fn wywolaj(sh: &Arc<Shared>, cmd: &'static str, mut args: Value) -> Result<Value, CallError> {
    if cmd != "shutdown" && sh.account_changed.load(Ordering::Acquire) {
        return Err(CallError::Disconnected);
    }
    if let Some(account) = sh.bound_account.lock().clone() {
        if !args.is_object() {
            args = serde_json::json!({});
        }
        args["_expected_account"] = account;
    }
    let id = sh.next_id.fetch_add(1, Ordering::Relaxed);
    let gen = sh.generation.load(Ordering::Acquire);
    let (tx, rx) = mpsc::channel();

    sh.pending.lock().insert(id, Pending { tx, gen });

    let line = Request::new(id, cmd, args).to_line();
    let sent = {
        let mut w = sh.writer.lock();
        match w.as_mut() {
            Some(s) => s.write_all(line.as_bytes()).and_then(|_| s.flush()),
            None => {
                drop(w);
                sh.pending.lock().remove(&id);
                return Err(CallError::Disconnected);
            }
        }
    };
    if let Err(e) = sent {
        warn!(%e, cmd, "MT5: zapis do sidecara nieudany");
        sh.pending.lock().remove(&id);
        return Err(CallError::Disconnected);
    }

    match rx.recv_timeout(sh.cfg.request_timeout) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) if e.code == -6 => {
            sh.account_changed.store(true, Ordering::Release);
            sh.ticks.lock().clear();
            sh.closed.lock().clear();
            sh.foreign_closed.lock().clear();
            Err(CallError::Broker(e))
        }
        Ok(Err(e)) if e.code == proto::local_code::NOT_INITIALIZED => Err(CallError::Disconnected),
        Ok(Err(e)) => Err(CallError::Broker(e)),
        Err(RecvTimeoutError::Timeout) => {
            sh.pending.lock().remove(&id);
            Err(CallError::Timeout(sh.cfg.request_timeout))
        }
        Err(RecvTimeoutError::Disconnected) => {
            sh.pending.lock().remove(&id);
            Err(CallError::Disconnected)
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        if self.sup.is_some() {
            self.shutdown();
        }
    }
}

// ============================================================
//  NADZÓR
// ============================================================

fn supervise(sh: Arc<Shared>, listener: TcpListener, port: u16) {
    if listener.set_nonblocking(true).is_err() {
        error!("MT5: nie da się ustawić nasłuchu nieblokującego");
        return;
    }

    while !sh.stopping.load(Ordering::Acquire) {
        let mut child = if sh.cfg.autostart {
            match spawn_sidecar(&sh.cfg, port) {
                Ok(c) => Some(c),
                Err(e) => {
                    error!(%e, "MT5: nie udało się uruchomić sidecara");
                    thread::sleep(sh.cfg.restart_backoff);
                    continue;
                }
            }
        } else {
            info!(
                port,
                "MT5: autostart wyłączony — czekam, aż sidecar sam się połączy"
            );
            None
        };

        match accept_with_timeout(&listener, sh.cfg.connect_timeout, &sh.stopping) {
            Some(stream) => {
                run_session(&sh, stream);
            }
            None => {
                warn!("MT5: sidecar się nie zgłosił");
            }
        }

        sh.connected.store(false, Ordering::Release);
        *sh.writer.lock() = None;
        sh.fail_all_pending();

        if let Some(c) = child.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if sh.stopping.load(Ordering::Acquire) {
            break;
        }
        sh.restarts.fetch_add(1, Ordering::Relaxed);
        warn!(
            restarts = sh.restarts.load(Ordering::Relaxed),
            "MT5: restart sidecara"
        );
        thread::sleep(sh.cfg.restart_backoff);
    }
    debug!("MT5: nadzór zakończony");
}

fn spawn_sidecar(cfg: &SidecarConfig, port: u16) -> std::io::Result<Child> {
    let mut c = sidecar_command(cfg, port);
    let mut child = c.spawn()?;
    if let Some(err) = child.stderr.take() {
        thread::Builder::new()
            .name("mt5-sidecar-log".into())
            .spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    warn!(target: "mt5_sidecar", "{line}");
                }
            })
            .ok();
    }
    Ok(child)
}

fn sidecar_command(cfg: &SidecarConfig, port: u16) -> Command {
    let mut c = Command::new(&cfg.python);
    c.arg(&cfg.script)
        .arg("--host")
        .arg(&cfg.host)
        .arg("--port")
        .arg(port.to_string())
        .arg("--symbol")
        .arg(&cfg.symbol)
        .arg("--magic")
        .arg(cfg.magic.to_string())
        .arg("--tick-ms")
        .arg(cfg.tick_interval_ms.to_string())
        .arg("--deal-ms")
        .arg(cfg.deal_poll_ms.to_string())
        .arg("--deviation")
        .arg(cfg.deviation_points.to_string());
    if let Some(p) = &cfg.terminal_path {
        c.arg("--terminal").arg(p);
    }
    if cfg.follow_terminal_account {
        c.arg("--follow-terminal-account");
        if cfg.allow_real_account {
            c.arg("--allow-real-account");
        }
        c.env_remove("CONDUIT_MT5_PASSWORD");
    }
    if cfg.close_receipt_reconcile {
        c.arg("--close-receipt-reconcile");
    }
    if cfg.closed_profit_net_costs { c.arg("--closed-profit-net-costs"); }
    if let Some(l) = cfg.login.filter(|_| !cfg.follow_terminal_account) {
        c.arg("--login").arg(l.to_string());
    }
    if let Some(s) = cfg.server.as_ref().filter(|_| !cfg.follow_terminal_account) {
        c.arg("--server").arg(s);
    }
    // hasło NIGDY w argumentach — trafiłoby do listy procesów
    if let Some(p) = cfg.password.as_ref().filter(|_| !cfg.follow_terminal_account) {
        c.env("CONDUIT_MT5_PASSWORD", p);
    }
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW: background helper only.
    }

    c
}

fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
    stopping: &AtomicBool,
) -> Option<TcpStream> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if stopping.load(Ordering::Acquire) {
            return None;
        }
        match listener.accept() {
            Ok((s, addr)) => {
                // tylko pętla lokalna — cokolwiek innego to nie nasz sidecar
                if !addr.ip().is_loopback() {
                    warn!(%addr, "MT5: odrzucone połączenie spoza pętli lokalnej");
                    let _ = s.shutdown(Shutdown::Both);
                    continue;
                }
                return Some(s);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                error!(%e, "MT5: accept nieudany");
                return None;
            }
        }
    }
    None
}

fn run_session(sh: &Arc<Shared>, stream: TcpStream) {
    // KONIECZNE: na Windows gniazdo zwrócone przez `accept()` dziedziczy tryb
    // nieblokujący po nasłuchu. Bez tego pierwszy `read_line` wraca z
    // `WouldBlock` i nadzór uznaje żywego sidecara za martwego — w kółko.
    if let Err(e) = stream.set_nonblocking(false) {
        error!(%e, "MT5: nie da się przełączyć gniazda w tryb blokujący");
        return;
    }
    let _ = stream.set_nodelay(true);
    // sidecar wysyła keepalive co sekundę; dłuższa cisza = zawieszony terminal
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));

    let wr = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            error!(%e, "MT5: nie da się sklonować gniazda");
            return;
        }
    };
    sh.generation.fetch_add(1, Ordering::AcqRel);
    // Unique across new Transport instances as well as reconnects in this
    // process. RAM intentions cannot mistake a new bridge's generation=1 for
    // the old bridge's generation=1. This token is deliberately not durable.
    static NEXT_EXECUTION_GENERATION: AtomicU64 = AtomicU64::new(1);
    sh.execution_generation.store(NEXT_EXECUTION_GENERATION.fetch_add(1, Ordering::AcqRel), Ordering::Release);
    *sh.writer.lock() = Some(wr);
    sh.connected.store(true, Ordering::Release);
    info!("MT5: sidecar podłączony");

    let mut rdr = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match rdr.read_line(&mut line) {
            Ok(0) => {
                warn!("MT5: sidecar zamknął połączenie");
                break;
            }
            Ok(_) => {}
            Err(e) => {
                warn!(%e, "MT5: odczyt z sidecara przerwany");
                break;
            }
        }
        match proto::parse_line(&line) {
            Ok(None) => {}
            Ok(Some(Frame::Response { id, result })) => {
                let gen = sh.generation.load(Ordering::Acquire);
                if let Some(p) = sh.pending.lock().remove(&id) {
                    if p.gen == gen {
                        let _ = p.tx.send(result);
                    } else {
                        debug!(id, "MT5: odpowiedź ze starego połączenia — pominięta");
                    }
                } else {
                    debug!(id, "MT5: odpowiedź bez oczekującego żądania (timeout?)");
                }
            }
            Ok(Some(Frame::Event { kind, body })) => handle_event(sh, &kind, body),
            Err(e) => {
                sh.proto_errors.fetch_add(1, Ordering::Relaxed);
                error!(%e, "MT5: ramka nie do sparsowania");
            }
        }
        if sh.stopping.load(Ordering::Acquire) {
            break;
        }
    }
}

fn handle_event(sh: &Arc<Shared>, kind: &str, body: Value) {
    if (sh.cfg.follow_terminal_account || sh.cfg.close_receipt_reconcile)
        && matches!(kind, "tick" | "closed" | "closed_foreign") {
        let bound = sh.bound_account.lock().clone();
        let Some(bound) = bound else { return; }; // no account-bound data before handshake
        if sh.account_changed.load(Ordering::Acquire) || body.get("account") != Some(&bound) {
            sh.account_changed.store(true, Ordering::Release);
            sh.ticks.lock().clear();
            sh.closed.lock().clear();
            sh.foreign_closed.lock().clear();
            return;
        }
    }
    match kind {
        "tick" => match serde_json::from_value::<RawTick>(body) {
            Ok(t) => sh.push_tick(t),
            Err(e) => warn!(%e, "MT5: zły tick"),
        },
        "closed_foreign" => match serde_json::from_value::<RawClosed>(body) {
            Ok(c) => {
                let mut q = sh.foreign_closed.lock();
                // Bufor ograniczony: to jest podgląd, a nie księga rachunkowa.
                if q.len() >= 2000 {
                    q.remove(0);
                }
                q.push(c);
            }
            Err(e) => {
                sh.proto_errors.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(blad = %e, "sidecar: nieczytelne zdarzenie closed_foreign");
            }
        },
        "closed" => match serde_json::from_value::<RawClosed>(body.clone()) {
            Ok(c) => sh.closed.lock().push(c),
            Err(e) => {
                warn!(%e, "MT5: złe zdarzenie zamknięcia");
                if sh.cfg.close_receipt_reconcile {
                    sh.unparsed_closed.lock().push(body);
                    sh.receipt_decode_issue.lock().get_or_insert_with(|| format!("Nieczytelne potwierdzenie zamknięcia: {e}"));
                }
            },
        },
        "hello" => match serde_json::from_value::<Hello>(body) {
            Ok(h) => {
                if h.proto != proto::PROTO_VERSION {
                    error!(
                        got = h.proto,
                        want = proto::PROTO_VERSION,
                        "MT5: ROZJAZD WERSJI PROTOKOŁU — sidecar i bot nie pasują do siebie"
                    );
                }
                info!(sidecar = %h.sidecar, mt5 = %h.mt5_version, "MT5: powitanie sidecara");
                *sh.hello.lock() = Some(h);
            }
            Err(e) => warn!(%e, "MT5: złe powitanie"),
        },
        "log" => {
            let msg = body.get("msg").and_then(|x| x.as_str()).unwrap_or("");
            info!(target: "mt5_sidecar", "{msg}");
        }
        "keepalive" => {}
        other => debug!(other, "MT5: nieznane zdarzenie"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_terminal_never_passes_credentials_even_when_config_contains_them() {
        let mut cfg = SidecarConfig::default();
        cfg.login = Some(123);
        cfg.server = Some("old-real".into());
        cfg.password = Some("fixture-not-a-secret".into());
        cfg.follow_terminal_account = true;
        let c = sidecar_command(&cfg, 12345);
        let args: Vec<_> = c.get_args().map(|x| x.to_string_lossy().into_owned()).collect();
        assert!(args.iter().any(|x| x == "--follow-terminal-account"));
        assert!(!args.iter().any(|x| x == "--login" || x == "--server" || x == "--allow-real-account"));
        assert!(c.get_envs().any(|(k, v)| k == "CONDUIT_MT5_PASSWORD" && v.is_none()));
        cfg.follow_terminal_account = false;
        let legacy = sidecar_command(&cfg, 12345);
        assert!(legacy.get_args().any(|x| x == "--login"));
        assert!(legacy.get_envs().any(|(k, v)| k == "CONDUIT_MT5_PASSWORD" && v.is_some()));
    }

    #[test]
    fn follow_stream_identity_blocks_colliding_ticket_and_latches_across_reconnect() {
        let cfg = SidecarConfig { follow_terminal_account: true, autostart: false,
            connect_timeout: Duration::from_millis(1), restart_backoff: Duration::from_millis(1), ..Default::default() };
        let tr = Transport::start(cfg).unwrap();
        tr.bind_account(42, "demo-server", 0);
        let identity = serde_json::json!({"login":42,"server":"demo-server","trade_mode":0});
        let mut tick = serde_json::json!({"ts":1000,"bid":2000.0,"ask":2000.2,"account":identity});
        handle_event(&tr.sh, "tick", tick.clone());
        assert_eq!(tr.drain_ticks().len(), 1);
        tick["account"]["server"] = serde_json::json!("real-server");
        handle_event(&tr.sh, "tick", tick);
        assert!(tr.sh.account_changed.load(Ordering::Acquire));
        tr.sh.connected.store(true, Ordering::Release); // transparent transport reconnection
        assert!(!tr.is_connected());
        assert!(matches!(tr.call("close_position", serde_json::json!({"ticket":7})), Err(CallError::Disconnected)));
        assert!(tr.drain_ticks().is_empty());
    }

    #[test]
    fn domyslna_konfiguracja_nie_wystawia_sie_na_swiat() {
        let c = SidecarConfig::default();
        assert_eq!(
            c.host, "127.0.0.1",
            "sidecar NIGDY nie może słuchać publicznie"
        );
        assert_eq!(c.port, 0, "port ma wybrać system, żeby nie było wyścigu");
    }

    #[test]
    fn call_bez_polaczenia_zwraca_disconnected_a_nie_wisi() {
        // nasłuch bez sidecara: autostart wyłączony, więc nikt się nie zgłosi
        let cfg = SidecarConfig {
            autostart: false,
            connect_timeout: Duration::from_millis(50),
            request_timeout: Duration::from_millis(50),
            restart_backoff: Duration::from_millis(10),
            ..Default::default()
        };
        let t = Transport::start(cfg).expect("nasłuch musi wstać");
        assert!(!t.is_connected());
        let started = Instant::now();
        let r = t.call("account", Value::Null);
        assert_eq!(r.unwrap_err(), CallError::Disconnected);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "nie wolno czekać na timeout"
        );
    }

    #[test]
    fn retcode_wyciagany_z_bledu_brokera() {
        let e = CallError::Broker(WireError {
            code: 10016,
            msg: "Invalid stops".into(),
        });
        assert_eq!(e.retcode(), Some(10016));
        assert_eq!(CallError::Disconnected.retcode(), None);
    }

    /// Atrapa sidecara: mówi protokołem, ale nie zna MT5. Pozwala przetestować
    /// całą pętlę transportu — powitanie, żądanie/odpowiedź, odmowę brokera
    /// i strumienie zdarzeń — bez terminala i bez Pythona.
    fn atrapa(port: u16) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let s = TcpStream::connect(("127.0.0.1", port)).expect("połączenie do Rusta");
            let mut w = s.try_clone().unwrap();
            let mut send = |v: &str| {
                let _ = w.write_all(v.as_bytes());
                let _ = w.write_all(b"\n");
                let _ = w.flush();
            };
            send(r#"{"ev":"hello","proto":1,"sidecar":"atrapa","mt5_version":"0.0"}"#);
            send(r#"{"ev":"tick","ts":1700000000000,"bid":4000.00,"ask":4000.24}"#);
            send(r#"{"ev":"tick","ts":1700000000100,"bid":4000.10,"ask":4000.34}"#);
            send(
                r#"{"ev":"closed","deal":5,"position":42,"deal_type":1,"volume":0.01,"price":4010.0,"time_msc":1700000001000,"profit":9.8,"magic":770077,"reason":5}"#,
            );

            let rdr = BufReader::new(s);
            for line in rdr.lines().map_while(Result::ok) {
                let v: Value = match serde_json::from_str(line.trim()) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let id = v["id"].as_u64().unwrap_or(0);
                match v["cmd"].as_str().unwrap_or("") {
                    "account" => send(&format!(
                        r#"{{"id":{id},"ok":true,"result":{{"balance":200.0,"equity":205.0,"margin":8.23,"margin_free":196.77,"leverage":500}}}}"#
                    )),
                    "open_market" => send(&format!(
                        r#"{{"id":{id},"ok":false,"error":{{"code":10016,"msg":"Invalid stops"}}}}"#
                    )),
                    "shutdown" => {
                        send(&format!(r#"{{"id":{id},"ok":true,"result":null}}"#));
                        break;
                    }
                    _ => send(&format!(r#"{{"id":{id},"ok":true,"result":null}}"#)),
                }
            }
        })
    }

    #[test]
    fn pelna_petla_z_atrapa_sidecara() {
        let cfg = SidecarConfig {
            autostart: false,
            // krótko, żeby `start` oddał sterowanie i test mógł podpiąć atrapę
            connect_timeout: Duration::from_millis(200),
            restart_backoff: Duration::from_millis(20),
            request_timeout: Duration::from_secs(3),
            ..Default::default()
        };
        let mut t = Transport::start(cfg).unwrap();
        let h = atrapa(t.local_port());
        assert!(
            t.wait_connected(Duration::from_secs(5)),
            "atrapa się nie podłączyła"
        );

        // --- żądanie / odpowiedź ---
        let acc: crate::proto::RawAccount = t.call_as("account", Value::Null).unwrap();
        assert_eq!(acc.balance, 200.0);
        assert_eq!(acc.leverage, 500);

        // --- odmowa brokera niesie retcode, a nie ogólny błąd ---
        let e = t.call("open_market", json_null()).unwrap_err();
        assert_eq!(e.retcode(), Some(10016));

        // --- strumień ticków ---
        let mut ticks = Vec::new();
        for _ in 0..50 {
            ticks.extend(t.drain_ticks());
            if ticks.len() >= 2 {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(ticks.len(), 2, "oba kwotowania muszą dojść");
        assert_eq!(ticks[0].ts, 1_700_000_000_000);
        assert!((ticks[1].ask - 4000.34).abs() < 1e-9);
        assert!(t.drain_ticks().is_empty(), "drain musi opróżniać bufor");

        // --- strumień zamknięć ---
        let closed = t.drain_closed();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].position, 42);
        assert_eq!(closed[0].reason, 5, "DEAL_REASON_TP");

        // --- powitanie odebrane i zgodne wersjami ---
        let hello = t.hello().expect("powitanie");
        assert_eq!(hello.proto, proto::PROTO_VERSION);
        assert_eq!(
            t.proto_errors(),
            0,
            "żadna ramka nie może być nie do sparsowania"
        );

        t.shutdown();
        let _ = h.join();
    }

    fn json_null() -> Value {
        Value::Null
    }
}
