//! Private, offline-verifiable Engine boundary recording. This does not claim
//! to replay MTProto transport, GUI policy, AI/EA or unobserved broker ticks.
use anyhow::{bail, Context, Result};
use conduit_core::{
    broker::Broker,
    engine::{Engine, IncomingMessage, ReplayBootstrap},
    recorded_broker::{
        exact::{self, Exact},
        Recorder, ReplayBroker, Trace,
    },
    types::Quote,
};
use conduit_server::replay_capture::{Capture, Event};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

#[cfg(feature = "capture-benchmark")]
pub mod benchmark {
    use std::{cell::RefCell, time::Instant};
    thread_local! { static SAMPLES: RefCell<Vec<[f64; 4]>> = const { RefCell::new(Vec::new()) }; }
    pub struct Phases {
        clock: Instant,
        elapsed: [f64; 4],
    }
    impl Phases {
        pub fn new() -> Self {
            Self {
                clock: Instant::now(),
                elapsed: [0.0; 4],
            }
        }
        pub fn mark(&mut self, phase: usize) {
            self.elapsed[phase] = self.clock.elapsed().as_secs_f64() * 1e6;
            self.clock = Instant::now();
        }
        pub fn finish(mut self) {
            self.mark(3);
            SAMPLES.with(|v| v.borrow_mut().push(self.elapsed));
        }
    }
    pub fn take() -> Vec<[f64; 4]> {
        SAMPLES.with(|v| std::mem::take(&mut *v.borrow_mut()))
    }
}

#[derive(Clone, Serialize, Deserialize)]
enum Action {
    Tick {
        quote: Quote,
        received_utc: i64,
    },
    Message {
        message: IncomingMessage,
        received_utc: i64,
    },
}
#[derive(Serialize, Deserialize)]
struct Frame {
    schema: u32,
    external_patch_contract: String,
    engine: String,
    action_seq: u64,
    bootstrap: Option<ReplayBootstrap>,
    pre_patch: BTreeMap<String, Exact>,
    action: Exact,
    broker: PooledTrace,
    source_revision_tokens: Vec<u64>,
    post_sha256: String,
}
#[derive(Serialize, Deserialize)]
struct PooledTrace {
    epoch: u64,
    base: usize,
    reset: bool,
    new_values: Vec<Exact>,
    calls: Vec<conduit_core::recorded_broker::Call>,
    incomplete: bool,
}
#[derive(Default)]
struct ValuePool {
    epoch: u64,
    values: Vec<Exact>,
    buckets: HashMap<u64, Vec<usize>>,
    bytes: usize,
}
impl ValuePool {
    fn encode(&mut self, trace: Trace) -> PooledTrace {
        use std::hash::{Hash, Hasher};
        let reset = self.epoch == 0 || self.values.len() > 4096 || self.bytes > 8 * 1024 * 1024;
        if reset {
            self.epoch += 1;
            self.values.clear();
            self.buckets.clear();
            self.bytes = 0;
        }
        let base = self.values.len();
        let mut new_values = Vec::new();
        let mut refs = Vec::with_capacity(trace.values.len());
        for value in trace.values {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut h);
            let hash = h.finish();
            let existing = self
                .buckets
                .get(&hash)
                .and_then(|ids| ids.iter().find(|&&i| self.values[i] == value))
                .copied();
            let id = match existing {
                Some(i) => i,
                None => {
                    let i = self.values.len();
                    self.bytes += value.estimated_bytes();
                    self.buckets.entry(hash).or_default().push(i);
                    new_values.push(value.clone());
                    self.values.push(value);
                    i
                }
            };
            refs.push(id);
        }
        let calls = trace
            .calls
            .into_iter()
            .map(|mut c| {
                c.args = refs[c.args];
                c.result = refs[c.result];
                c
            })
            .collect();
        PooledTrace {
            epoch: self.epoch,
            base,
            reset,
            new_values,
            calls,
            incomplete: trace.incomplete,
        }
    }
    fn decode(&mut self, tape: PooledTrace) -> Result<Trace> {
        if tape.reset {
            if tape.epoch != self.epoch + 1 || tape.base != 0 {
                bail!("invalid value dictionary reset");
            }
            self.epoch = tape.epoch;
            self.values.clear();
            self.bytes = 0;
        }
        if tape.epoch != self.epoch || tape.base != self.values.len() {
            bail!("missing value dictionary prefix");
        }
        for v in tape.new_values {
            self.bytes += v.estimated_bytes();
            if self.bytes > 32 * 1024 * 1024 || self.values.len() > 20_000 {
                bail!("value dictionary exceeds bounded memory");
            }
            self.values.push(v);
        }
        let mut local = Trace::default();
        local.incomplete = tape.incomplete;
        let mut refs = HashMap::new();
        for mut call in tape.calls {
            for key in [&mut call.args, &mut call.result] {
                let global = *key;
                let local_index = if let Some(&v) = refs.get(&global) {
                    v
                } else {
                    let v = local.values.len();
                    local.values.push(
                        self.values
                            .get(global)
                            .context("missing exact value reference")?
                            .clone(),
                    );
                    refs.insert(global, v);
                    v
                };
                *key = local_index;
            }
            local.calls.push(call);
        }
        Ok(local)
    }
}
struct Memory {
    engines: HashMap<usize, (String, ReplayBootstrap)>,
    next: u64,
    values: ValuePool,
}
#[derive(Clone)]
pub struct Session {
    pub tape: Capture,
    memory: Arc<Mutex<Memory>>,
}
fn fingerprint<T: Serialize>(value: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
pub fn binary_sha256() -> Result<String> {
    static HASH: OnceLock<Result<String, String>> = OnceLock::new();
    HASH.get_or_init(|| {
        let path = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut h = Sha256::new();
        {
            use std::io::Read;
            let mut buf = [0u8; 65536];
            loop {
                let n = f.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
        }
        Ok(format!("{:x}", h.finalize()))
    })
    .clone()
    .map_err(anyhow::Error::msg)
}
impl Session {
    pub fn start(root: &Path, run_id: &str) -> Result<Self> {
        Self::start_at(
            root,
            run_id,
            "before_engine_construction_and_application_restore",
            None,
            None,
        )
    }
    pub fn checkpoint(root: &Path, run_id: &str, predecessor: &str, reason: &str) -> Result<Self> {
        Self::start_at(
            root,
            run_id,
            "complete_engine_checkpoint_at_next_decision",
            Some(predecessor),
            Some(reason),
        )
    }
    fn start_at(
        root: &Path,
        run_id: &str,
        origin: &str,
        predecessor: Option<&str>,
        boundary: Option<&str>,
    ) -> Result<Self> {
        let binary = binary_sha256()?;
        let tape = Capture::start(root, run_id, &binary)?;
        tape.append("session_origin",&json!({"schema":"conduit.engine-capture.origin.v1","binary_sha256":binary,"package_version":env!("CARGO_PKG_VERSION"),"source_sha256":env!("CONDUIT_REPLAY_SOURCE_SHA256"),"scope":"actual_rules_engine_calls_with_exact_broker_transcripts_and_observed_external_state_patches","origin":origin,"predecessor_run_id":predecessor,"boundary_reason":boundary,"application_interval_rederived":false,"clock":"explicit_quote_broker_wall_and_received_utc","privacy":"private_messages_trades_and_account_identity; no_auth_configuration","excluded":["mtproto_transport","gui_routing_policy_verification","unobserved_mt5_ticks","ai_ea"]}));
        Ok(Self {
            tape,
            memory: Arc::new(Mutex::new(Memory {
                engines: HashMap::new(),
                next: 0,
                values: ValuePool::default(),
            })),
        })
    }
    pub fn tick<B: Broker>(&self, e: &mut Engine, b: &mut B, q: &Quote, received_utc: i64) {
        self.execute(
            e,
            b,
            Action::Tick {
                quote: *q,
                received_utc,
            },
        );
    }
    pub fn message<B: Broker>(
        &self,
        e: &mut Engine,
        b: &mut B,
        message: &IncomingMessage,
        received_utc: i64,
    ) {
        self.execute(
            e,
            b,
            Action::Message {
                message: message.clone(),
                received_utc,
            },
        );
    }
    fn execute<B: Broker>(&self, e: &mut Engine, b: &mut B, action: Action) {
        if !self.tape.active() {
            apply(e, b, &action);
            return;
        }
        #[cfg(feature = "capture-benchmark")]
        let mut phases = benchmark::Phases::new();
        let before = match e.export_replay_bootstrap() {
            Ok(v) => v,
            Err(_) => {
                self.tape.invalidate(
                    "unsupported or unencodable engine bootstrap; AI/EA outside capture v1",
                );
                apply(e, b, &action);
                return;
            }
        };
        let address = e as *const Engine as usize;
        let (id, seq, bootstrap, patch) = {
            let mut m = self.memory.lock().unwrap_or_else(|p| p.into_inner());
            m.next += 1;
            let seq = m.next;
            match m.engines.get(&address) {
                Some((id, last)) => (
                    id.clone(),
                    seq,
                    None,
                    before
                        .fields
                        .iter()
                        .filter(|(k, v)| last.fields.get(*k) != Some(*v))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
                None => (
                    format!("engine-{}", m.engines.len() + 1),
                    seq,
                    Some(before.clone()),
                    BTreeMap::new(),
                ),
            }
        };
        if patch.keys().any(|k| !external_field(k)) {
            self.tape
                .invalidate("unclassified external engine mutation; prefix not qualified");
            apply(e, b, &action);
            return;
        }
        self.tape
            .append("engine_begin", &json!({"engine":id,"action_seq":seq}));
        #[cfg(feature = "capture-benchmark")]
        phases.mark(0);
        let revisions = conduit_core::recorded_broker::revisions::Scope::record();
        let mut recorded = Recorder::new(b);
        apply(e, &mut recorded, &action);
        let source_revision_tokens = match revisions.finish() {
            Ok(v) => v,
            Err(_) => {
                self.tape.invalidate("source revision token capture failed");
                return;
            }
        };
        let trace = recorded.finish();
        #[cfg(feature = "capture-benchmark")]
        phases.mark(1);
        if trace.incomplete {
            self.tape
                .invalidate("broker transcript bounded limit reached");
            return;
        }
        match (e.export_replay_bootstrap(), exact::encode(&action)) {
            (Ok(after), Ok(action)) => match fingerprint(&after) {
                Ok(post_sha256) => {
                    #[cfg(feature = "capture-benchmark")]
                    phases.mark(2);
                    let frame = Frame {
                        schema: 1,
                        external_patch_contract: "reviewed_application_mutations_v1".into(),
                        engine: id.clone(),
                        action_seq: seq,
                        bootstrap,
                        pre_patch: patch,
                        action,
                        broker: self
                            .memory
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .values
                            .encode(trace),
                        source_revision_tokens,
                        post_sha256,
                    };
                    self.tape.append("engine_frame", &frame);
                    self.memory
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .engines
                        .insert(address, (id, after));
                    #[cfg(feature = "capture-benchmark")]
                    phases.finish();
                }
                Err(_) => self.tape.invalidate("engine post-state digest unavailable"),
            },
            _ => self
                .tape
                .invalidate("engine post-state or action encoding unavailable"),
        }
    }
}
// Production application mutations between calls, audited against live.rs and
// routing.rs. Manual commands delimit runs; the next run has an explicit hot
// checkpoint. Unknown private decision-state mutation never passes.
fn external_field(k: &str) -> bool {
    matches!(
        k,
        "cfg"
            | "obce"
            | "tryb_auto_ea"
            | "journal"
            | "logs"
            | "odrzuty"
            | "halted"
            | "continuation"
    )
}
fn apply<B: Broker>(e: &mut Engine, b: &mut B, a: &Action) {
    match a {
        Action::Tick {
            quote,
            received_utc,
        } => e.on_tick_received(b, quote, *received_utc),
        Action::Message {
            message,
            received_utc,
        } => e.on_message_received(b, message, *received_utc),
    }
}

#[derive(Default)]
struct Verify {
    engines: BTreeMap<String, Engine>,
    pending: BTreeMap<u64, String>,
    frames: u64,
    last_action: u64,
    source_sha256: Option<String>,
    values: ValuePool,
}
impl Verify {
    fn event(&mut self, event: Event, report: &Path) -> Result<()> {
        match event.kind.as_str() {
            "session_origin" => {
                if event.seq != 1 || event.payload["schema"] != "conduit.engine-capture.origin.v1" {
                    bail!("invalid recorded engine origin");
                }
                self.source_sha256 = event.payload["source_sha256"].as_str().map(str::to_string);
            }
            "engine_begin" => {
                let seq = event.payload["action_seq"]
                    .as_u64()
                    .context("action sequence missing")?;
                let engine = event.payload["engine"]
                    .as_str()
                    .context("engine identity missing")?
                    .to_string();
                if seq != self.last_action + 1 || self.pending.insert(seq, engine).is_some() {
                    bail!("duplicate or unordered engine action");
                }
                self.last_action = seq;
            }
            "engine_frame" => {
                let frame: Frame = serde_json::from_value(event.payload)?;
                if frame.schema != 1
                    || frame.external_patch_contract != "reviewed_application_mutations_v1"
                    || self.pending.remove(&frame.action_seq).as_deref() != Some(&frame.engine)
                {
                    bail!("engine frame without matching begin");
                }
                if let Some(seed) = frame.bootstrap {
                    if self.engines.contains_key(&frame.engine) {
                        bail!("duplicate engine bootstrap");
                    }
                    self.engines.insert(
                        frame.engine.clone(),
                        Engine::from_replay_bootstrap(&seed).map_err(anyhow::Error::msg)?,
                    );
                }
                let engine = self
                    .engines
                    .get_mut(&frame.engine)
                    .context("engine bootstrap missing")?;
                if frame.pre_patch.keys().any(|k| !external_field(k)) {
                    bail!("unclassified external engine state patch");
                }
                engine
                    .apply_replay_patch(&frame.pre_patch, false)
                    .map_err(anyhow::Error::msg)?;
                let action: Action =
                    exact::decode(&frame.action).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let path = report.to_path_buf();
                let action_seq = frame.action_seq;
                let engine_id = frame.engine.clone();
                let mut broker = ReplayBroker::new(self.values.decode(frame.broker)?)
                    .map_err(anyhow::Error::msg)?;
                broker.on_mismatch(move|m|{let _=write_report(&path,&json!({"schema":"conduit.engine-replay.report.v1","status":"FAIL","action_seq":action_seq,"engine":engine_id,"mismatch":m,"network_or_services_started":false}));});
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let revisions = conduit_core::recorded_broker::revisions::Scope::replay(
                        frame.source_revision_tokens,
                    );
                    apply(engine, &mut broker, &action);
                    revisions.finish()?;
                    broker.finish()
                }));
                match run {
                    Ok(v) => v.map_err(anyhow::Error::msg)?,
                    Err(_) => bail!(
                        "recorded broker transcript mismatch at action {}",
                        frame.action_seq
                    ),
                };
                let after = engine
                    .export_replay_bootstrap()
                    .map_err(anyhow::Error::msg)?;
                if fingerprint(&after)? != frame.post_sha256 {
                    bail!("engine post-state mismatch at action {}", frame.action_seq);
                }
                self.frames += 1;
            }
            "application_bootstrap"
            | "application_provenance"
            | "broker_operation"
            | "session_end" => {}
            _ => bail!("unknown event kind: {}", event.kind),
        }
        Ok(())
    }
}
fn write_report(path: &Path, value: &Value) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    conduit_server::store::write_json_atomic(path, value)
}
pub fn verify_directory(dir: &Path, report: &Path) -> Result<Value> {
    let mut v = Verify::default();
    let m = conduit_server::replay_capture::read_events(dir, |e| v.event(e, report))?;
    if !v.pending.is_empty() || v.frames == 0 {
        bail!("capture prefix has an unfinished action or no engine frames");
    }
    let current = binary_sha256()?;
    Ok(
        json!({"schema":"conduit.engine-replay.report.v1","status":"PASS","scope":m.qualification_scope,"run_id":m.run_id,"frames":v.frames,"engines":v.engines.len(),"verified_through_seq":m.last_seq,"session_closed":m.status=="closed","capture_binary_sha256":m.binary_sha256,"verifier_binary_sha256":current,"same_binary":current==m.binary_sha256,"capture_source_sha256":v.source_sha256,"verifier_source_sha256":env!("CONDUIT_REPLAY_SOURCE_SHA256"),"same_source":v.source_sha256.as_deref()==Some(env!("CONDUIT_REPLAY_SOURCE_SHA256")),"meaning":"all captured Engine calls, broker call order/results and resulting exact state matched; external application state changes were supplied, not rederived","network_or_services_started":false}),
    )
}

/// Extract only the explicit lossless frames. Never interpret Telegram text as
/// filenames, options, commands or instructions. No authentication is loaded.
struct ExtractedRuns {
    directories: Vec<PathBuf>,
    skipped_runs: Vec<String>,
    skipped_retention: bool,
}
fn extract_alllogs(
    input: &Path,
    destination: &Path,
    selected: Option<&str>,
) -> Result<ExtractedRuns> {
    use std::io::{BufRead, Read};
    let f = std::fs::File::open(input)?;
    if f.metadata()?.len() > 1024 * 1024 * 1024 {
        bail!("allLogs exceeds bounded input size");
    }
    std::fs::create_dir(destination)?;
    let mut runs = std::collections::BTreeSet::new();
    let mut files = std::collections::BTreeSet::new();
    let mut skipped = std::collections::BTreeSet::new();
    let mut skipped_retention = false;
    let mut total = 0u64;
    let mut reader = std::io::BufReader::new(f);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = (&mut reader)
            .take(96 * 1024 * 1024)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if n >= 96 * 1024 * 1024 {
            bail!("allLogs frame line exceeds bounded size");
        }
        let line = std::str::from_utf8(&line)?;
        if let Some(raw) = line.strip_prefix("CONDUIT_REPLAY_EXPORT_STATUS_V1 ") {
            let status: Value = serde_json::from_str(raw)?;
            if let Some(selected) = selected {
                let run = status["run_id"]
                    .as_str()
                    .context("export status run id missing")?;
                if !conduit_server::replay_capture::safe_component(run) {
                    bail!("unsafe export status run identifier");
                }
                if run != selected {
                    if run == "_retention" {
                        skipped_retention = true;
                    } else {
                        skipped.insert(run.to_string());
                    }
                    continue;
                }
            }
            if status["status"] != "committed_prefix_exported" {
                bail!("allLogs reports an incomplete capture export");
            }
            continue;
        }
        let Some(raw) = line.strip_prefix("CONDUIT_REPLAY_FILE_V1 ") else {
            continue;
        };
        let value: Value = serde_json::from_str(raw)?;
        let run = value["run_id"].as_str().context("run id missing")?;
        let name = value["file"].as_str().context("file name missing")?;
        if !conduit_server::replay_capture::safe_component(run) {
            bail!("unsafe capture run identifier");
        }
        if run == "_retention" && name == "retention.json" {
            skipped_retention = true;
            continue;
        }
        if selected.is_some_and(|wanted| run != wanted) {
            skipped.insert(run.to_string());
            continue;
        }
        if name != "manifest.json" && !conduit_server::replay_capture::safe_segment(name) {
            bail!("unsafe capture frame path");
        }
        if !files.insert((run.to_string(), name.to_string())) {
            bail!("duplicate capture frame");
        }
        let content = value["content"]
            .as_str()
            .context("frame content missing")?
            .as_bytes();
        if value["bytes"].as_u64() != Some(content.len() as u64)
            || value["sha256"].as_str() != Some(&format!("{:x}", Sha256::digest(content)))
        {
            bail!("allLogs frame byte/hash mismatch");
        }
        total = total
            .checked_add(content.len() as u64)
            .context("allLogs size overflow")?;
        if total > conduit_server::replay_capture::RUN_BUDGET {
            bail!("capture extraction exceeds total budget");
        }
        let dir = destination.join(run);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(name), content)?;
        runs.insert(dir);
    }
    if runs.is_empty() {
        if selected.is_some() {
            bail!("selected replay run has no capture frames in allLogs");
        }
        bail!("allLogs contains no lossless replay capture frames");
    }
    Ok(ExtractedRuns {
        directories: runs.into_iter().collect(),
        skipped_runs: skipped.into_iter().collect(),
        skipped_retention,
    })
}
/// Must run before the ordinary CLI parser or any runtime/window initialization.
pub fn maybe_cli(argv: &[String]) -> Result<bool> {
    let Some(index) = argv.iter().position(|a| a == "--replay-capture") else {
        return Ok(false);
    };
    let input = argv
        .get(index + 1)
        .context("--replay-capture requires a directory or allLogs file")?;
    let selectors: Vec<_> = argv
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "--replay-run")
        .collect();
    if selectors.len() > 1 {
        bail!("--replay-run may be specified only once");
    }
    let selected = if let Some((i, _)) = selectors.first() {
        let value = argv
            .get(i + 1)
            .context("--replay-run requires an exact run identifier")?;
        if value.starts_with("--")
            || value == "_retention"
            || !conduit_server::replay_capture::safe_component(value)
        {
            bail!("--replay-run requires a safe, non-reserved run identifier");
        }
        Some(value.as_str())
    } else {
        None
    };
    let report = argv
        .iter()
        .position(|a| a == "--replay-report")
        .and_then(|i| argv.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("replay-report.json"));
    if report.exists() {
        bail!("replay report already exists; choose a new --replay-report path");
    }
    let mut skipped_run_ids = Vec::<String>::new();
    let mut retention_record_not_qualified = false;
    let mut selection_inventory_complete = false;
    let result = (|| -> Result<Value> {
        let p = Path::new(input);
        if p.is_dir() {
            if selected.is_some() {
                bail!("--replay-run applies only to an allLogs file");
            }
            verify_directory(p, &report)
        } else if p.is_file() {
            let extracted = report.with_extension(format!("capture-{}", std::process::id()));
            let extracted = extract_alllogs(p, &extracted, selected)?;
            skipped_run_ids = extracted.skipped_runs.clone();
            retention_record_not_qualified = extracted.skipped_retention;
            selection_inventory_complete = true;
            let results = extracted
                .directories
                .iter()
                .map(|d| {
                    let result = verify_directory(d, &report)?;
                    if selected.is_some_and(|wanted| result["run_id"].as_str() != Some(wanted)) {
                        bail!("selected run differs from captured manifest identity");
                    }
                    Ok(result)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(
                json!({"schema":"conduit.engine-replay.report.v1","status":"PASS","runs":results,
                    "selected_run":selected,"selection_applied":selected.is_some(),
                    "skipped_run_ids":extracted.skipped_runs,"skipped_run_count":extracted.skipped_runs.len(),
                    "skipped_runs_not_verified":true,"retention_record_not_qualified":extracted.skipped_retention,
                    "selection_inventory_complete":selection_inventory_complete,
                    "selection_scope":if selected.is_some(){"only the explicitly selected run was verified; omitted runs were not qualified"}else{"all extracted runs; first failure aborts verification"},
                    "network_or_services_started":false}),
            )
        } else {
            bail!("capture input does not exist");
        }
    })();
    match result {
        Ok(value) => {
            write_report(&report, &value)?;
            println!("CONDUIT_REPLAY_RESULT PASS");
            Ok(true)
        }
        Err(e) => {
            let value = json!({"schema":"conduit.engine-replay.report.v1","status":"FAIL","reason":e.to_string(),
                "selected_run":selected,"selection_applied":selected.is_some(),
                "skipped_run_count":skipped_run_ids.len(),"skipped_run_ids":skipped_run_ids,
                "skipped_runs_not_verified":true,"selection_inventory_complete":selection_inventory_complete,
                "retention_record_not_qualified":retention_record_not_qualified,
                "network_or_services_started":false});
            if !report.exists() {
                write_report(&report, &value)?;
            } else if selected.is_some() {
                // The verifier can have already written its detailed first
                // broker mismatch. Keep every diagnostic field and attach the
                // explicit selection context rather than replacing that report.
                use std::io::Read;
                const REPORT_LIMIT: u64 = 16 * 1024 * 1024;
                let file = std::fs::File::open(&report)?;
                let mut bytes = Vec::new();
                file.take(REPORT_LIMIT + 1).read_to_end(&mut bytes)?;
                if bytes.len() as u64 > REPORT_LIMIT {
                    bail!("selected replay failed; detailed report exceeds context-merge budget: {e}");
                }
                let mut detailed: Value = serde_json::from_slice(&bytes)?;
                let fields = detailed.as_object_mut().context("invalid detailed replay report")?;
                for key in ["selected_run", "selection_applied", "skipped_run_count", "skipped_run_ids",
                    "skipped_runs_not_verified", "selection_inventory_complete", "retention_record_not_qualified"] {
                    fields.insert(key.into(), value[key].clone());
                }
                write_report(&report, &detailed)?;
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod selector_tests {
    use super::*;

    fn root() -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "conduit-replay-selector-{}-{}-{}",
            std::process::id(),
            conduit_server::now_ms(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        p
    }
    fn status(run: &str, value: &str) -> String {
        format!(
            "CONDUIT_REPLAY_EXPORT_STATUS_V1 {}\n",
            json!({"run_id":run,"status":value})
        )
    }
    fn frame(run: &str, file: &str, content: &[u8]) -> String {
        format!(
            "CONDUIT_REPLAY_FILE_V1 {}\n",
            json!({"run_id":run,"file":file,
            "bytes":content.len(),"sha256":format!("{:x}",Sha256::digest(content)),
            "content":std::str::from_utf8(content).unwrap()})
        )
    }
    fn args(input: &Path, report: &Path, run: Option<&str>) -> Vec<String> {
        let mut a = vec![
            "conduit".into(),
            "--replay-capture".into(),
            input.display().to_string(),
            "--replay-report".into(),
            report.display().to_string(),
        ];
        if let Some(run) = run {
            a.extend(["--replay-run".into(), run.into()]);
        }
        a
    }
    fn actual_run(p: &Path, id: &str, envelope_id: &str) -> String {
        use conduit_backtest::sim::SimBroker;
        let session = Session::start(p, id).unwrap();
        let mut cfg = conduit_core::Settings::default();
        cfg.ai_enabled = false;
        cfg.ea_enabled = false;
        let mut engine = Engine::new(cfg, 600.0);
        let mut broker = SimBroker::new(600.0, 0.0, 0.0);
        let quote = Quote {
            ts: 1782400000000,
            bid: 4000.0,
            ask: 4000.2,
        };
        broker.on_quote(quote);
        session.tick(&mut engine, &mut broker, &quote, quote.ts - 10_800_000);
        finished_run(p, id, envelope_id, &session.tape)
    }
    fn finished_run(p: &Path, id: &str, envelope_id: &str, tape: &Capture) -> String {
        tape.finish();
        let dir = p.join(id);
        let began = std::time::Instant::now();
        let manifest = loop {
            if let Ok(m) = conduit_server::replay_capture::read_manifest(&dir) {
                if m.writer_closed {
                    break m;
                }
            }
            assert!(began.elapsed() < std::time::Duration::from_secs(30));
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let mut out = frame(
            envelope_id,
            "manifest.json",
            &std::fs::read(dir.join("manifest.json")).unwrap(),
        );
        for segment in manifest.segments {
            let bytes = std::fs::read(dir.join(&segment.file)).unwrap();
            out.push_str(&frame(
                envelope_id,
                &segment.file,
                &bytes[..segment.bytes as usize],
            ));
        }
        out.push_str(&status(envelope_id, "committed_prefix_exported"));
        out
    }
    #[test]
    fn explicit_selection_verifies_actual_engine_run_and_reports_skipped_incomplete_run() {
        let p = root();
        let input = p.join("alllogs.txt");
        let text = status("older", "incomplete")
            + &frame("_retention", "retention.json", b"{\"deleted_runs\":[]}")
            + &actual_run(&p, "healthy", "healthy");
        std::fs::write(&input, text).unwrap();
        let report = p.join("selected.json");
        assert!(maybe_cli(&args(&input, &report, Some("healthy"))).unwrap());
        let result: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
        assert_eq!(result["status"], "PASS");
        assert_eq!(result["selected_run"], "healthy");
        assert_eq!(result["runs"].as_array().unwrap().len(), 1);
        assert_eq!(result["runs"][0]["frames"], 1);
        assert_eq!(result["skipped_run_ids"], json!(["older"]));
        assert_eq!(result["skipped_run_count"], 1);
        assert_eq!(result["skipped_runs_not_verified"], true);
        assert_eq!(result["retention_record_not_qualified"], true);
        assert_eq!(result["network_or_services_started"], false);
        assert!(maybe_cli(&args(&input, &p.join("default.json"), None)).is_err());
        let failure: Value =
            serde_json::from_slice(&std::fs::read(p.join("default.json")).unwrap()).unwrap();
        assert_eq!(failure["status"], "FAIL");
        assert_eq!(failure["selection_applied"], false);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn selected_incomplete_or_absent_run_is_not_qualified() {
        let p = root();
        let input = p.join("alllogs.txt");
        std::fs::write(
            &input,
            status("broken", "incomplete") + &frame("broken", "manifest.json", b"{}"),
        )
        .unwrap();
        assert!(extract_alllogs(&input, &p.join("selected"), Some("broken")).is_err());
        assert!(extract_alllogs(&input, &p.join("absent"), Some("missing")).is_err());
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn selected_bytes_are_checked_while_unselected_bytes_are_explicitly_unverified() {
        let p = root();
        let input = p.join("alllogs.txt");
        let corrupt = format!(
            "CONDUIT_REPLAY_FILE_V1 {}\n",
            json!({"run_id":"other","file":"manifest.json",
            "bytes":999,"sha256":"invalid","content":"{}"})
        );
        std::fs::write(&input, corrupt + &frame("selected", "manifest.json", b"{}")).unwrap();
        let extracted = extract_alllogs(&input, &p.join("good"), Some("selected")).unwrap();
        assert_eq!(extracted.skipped_runs, vec!["other"]);
        assert!(extract_alllogs(&input, &p.join("bad"), Some("other")).is_err());
        assert!(extract_alllogs(&input, &p.join("default"), None).is_err());
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn selected_paths_and_duplicates_remain_fail_closed() {
        let p = root();
        let input = p.join("alllogs.txt");
        std::fs::write(&input, frame("selected", "../escape", b"{}")).unwrap();
        assert!(extract_alllogs(&input, &p.join("unsafe"), Some("selected")).is_err());
        let valid = frame("selected", "manifest.json", b"{}");
        std::fs::write(&input, valid.clone() + &valid).unwrap();
        assert!(extract_alllogs(&input, &p.join("duplicate"), Some("selected")).is_err());
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn selected_envelope_cannot_alias_a_different_manifest_identity() {
        let p = root();
        let input = p.join("alllogs.txt");
        std::fs::write(
            &input,
            status("older", "incomplete") + &actual_run(&p, "actual", "alias"),
        )
        .unwrap();
        let error = maybe_cli(&args(&input, &p.join("report.json"), Some("alias"))).unwrap_err();
        assert!(error.to_string().contains("manifest identity"));
        let result: Value =
            serde_json::from_slice(&std::fs::read(p.join("report.json")).unwrap()).unwrap();
        assert_eq!(result["status"], "FAIL");
        assert_eq!(result["selected_run"], "alias");
        assert_eq!(result["skipped_run_ids"], json!(["older"]));
        assert_eq!(result["skipped_runs_not_verified"], true);
        assert_eq!(result["selection_inventory_complete"], true);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn selected_broker_mismatch_keeps_detailed_diagnostic_and_selection_context() {
        let p = root();
        actual_run(&p, "original", "original");
        let tape = Capture::start(&p, "mismatch", &binary_sha256().unwrap()).unwrap();
        let mut modified = false;
        conduit_server::replay_capture::read_events(&p.join("original"), |mut event| {
            if event.kind == "engine_frame" {
                event.payload["broker"]["calls"][0]["method"] = json!("synthetic_mismatch");
                modified = true;
            }
            tape.append(&event.kind, &event.payload);
            Ok(())
        }).unwrap();
        assert!(modified);
        let input = p.join("alllogs.txt");
        std::fs::write(&input, status("older", "incomplete") + &finished_run(&p, "mismatch", "mismatch", &tape)).unwrap();
        let report = p.join("mismatch-report.json");
        assert!(maybe_cli(&args(&input, &report, Some("mismatch"))).is_err());
        let result: Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
        assert_eq!(result["status"], "FAIL");
        assert!(result["mismatch"].is_object(), "retain detailed broker mismatch");
        assert_eq!(result["action_seq"], 1);
        assert_eq!(result["selected_run"], "mismatch");
        assert_eq!(result["skipped_run_ids"], json!(["older"]));
        assert_eq!(result["selection_inventory_complete"], true);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn selector_is_exact_single_nonreserved_and_file_only() {
        let p = root();
        for (i, value) in ["../escape", "_retention", "--replay-report"]
            .iter()
            .enumerate()
        {
            assert!(
                maybe_cli(&args(&p, &p.join(format!("invalid-{i}.json")), Some(value))).is_err()
            );
        }
        let mut duplicate = args(&p, &p.join("duplicate.json"), Some("one"));
        duplicate.extend(["--replay-run".into(), "two".into()]);
        assert!(maybe_cli(&duplicate).is_err());
        let mut missing = args(&p, &p.join("missing.json"), None);
        missing.push("--replay-run".into());
        assert!(maybe_cli(&missing).is_err());
        let error = maybe_cli(&args(&p, &p.join("directory.json"), Some("one"))).unwrap_err();
        assert!(error.to_string().contains("only to an allLogs file"));
        std::fs::remove_dir_all(p).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conduit_backtest::sim::SimBroker;
    fn finish_capture(session: &Session, dir: &Path) {
        session.tape.finish();
        let start = std::time::Instant::now();
        loop {
            if conduit_server::replay_capture::read_manifest(dir).is_ok_and(|m| m.writer_closed) {
                break;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(30),
                "offline writer finalization deadline"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    fn root() -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "conduit-engine-tape-{}-{}-{}",
            std::process::id(),
            conduit_server::now_ms(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn actual_engine_tick_message_and_external_patch_roundtrip() {
        let p = root();
        let session = Session::start(&p, "fixture").unwrap();
        let mut cfg = conduit_core::Settings::default();
        cfg.ai_enabled = false;
        cfg.ea_enabled = false;
        let mut e = Engine::new(cfg, 600.0);
        let mut b = SimBroker::new(600.0, 0.0, 0.0);
        let q = Quote {
            ts: 1782400000000,
            bid: 4000.0,
            ask: 4000.2,
        };
        b.on_quote(q);
        session.tick(&mut e, &mut b, &q, 1782389200000);
        e.halted = Some("synthetic external diagnosis".into());
        let q = Quote {
            ts: q.ts + 250,
            bid: 3999.0,
            ask: 3999.2,
        };
        b.on_quote(q);
        session.tick(&mut e, &mut b, &q, 1782389200250);
        finish_capture(&session, &p.join("fixture"));
        let report = p.join("report.json");
        let result = verify_directory(&p.join("fixture"), &report).unwrap();
        assert_eq!(result["frames"], 2);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn offline_cli_missing_input_fails_without_startup() {
        let p = root();
        let args = vec![
            "conduit".into(),
            "--replay-capture".into(),
            p.join("missing").display().to_string(),
            "--replay-report".into(),
            p.join("report.json").display().to_string(),
        ];
        assert!(maybe_cli(&args).is_err());
        let v: Value =
            serde_json::from_slice(&std::fs::read(p.join("report.json")).unwrap()).unwrap();
        assert_eq!(v["network_or_services_started"], false);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn cross_process_reader() {
        let Ok(path) = std::env::var("CONDUIT_SYNTHETIC_REPLAY_DIR") else {
            return;
        };
        let p = PathBuf::from(path);
        let result = verify_directory(&p, &p.join("child-report.json")).unwrap();
        assert!(result["frames"].as_u64().unwrap() > 1000);
    }
    #[test]
    fn busy_multibasket_tape_matches_in_independent_process_and_reports_cost() {
        let p = root();
        let session = Session::start(&p, "busy").unwrap();
        let mut cfg = conduit_core::Settings::default();
        cfg.ai_enabled = false;
        cfg.ea_enabled = false;
        cfg.lot_mode_percent = false;
        cfg.lot_fixed = 0.01;
        cfg.lot_max = 0.01;
        cfg.max_open_baskets = 0;
        let baseline_cfg = cfg.clone();
        let mut messages = Vec::new();
        let mut engine = Engine::new(cfg, 10_000.0);
        let mut broker = SimBroker::new(10_000.0, 0.0, 0.0);
        let start = 1782400000000;
        let q = Quote {
            ts: start,
            bid: 4000.0,
            ask: 4000.2,
        };
        broker.on_quote(q);
        for id in 1..=40 {
            let message = IncomingMessage {
                ts: start,
                source: conduit_core::types::SourceKey::new(id, None),
                source_name: "Synthetic fixture".into(),
                msg_id: id,
                reply_to: None,
                edit_of: None,
                text: "GOLD BUY 3999-4001 SL 3990 TP1 4010 TP2 4020 TP3 4030".into(),
            };
            session.message(&mut engine, &mut broker, &message, start - 10_800_000);
            messages.push(message);
        }
        assert!(
            engine.baskets.len() >= 30,
            "fixture must exercise multiple real engine baskets"
        );
        let mut samples = Vec::new();
        let began = std::time::Instant::now();
        for n in 1..=1200 {
            let p = 3999.0 + (n % 31) as f64 * 0.08;
            let q = Quote {
                ts: start + n * 250,
                bid: p,
                ask: p + 0.2,
            };
            broker.on_quote(q);
            let tick_began = std::time::Instant::now();
            session.tick(&mut engine, &mut broker, &q, q.ts - 10_800_000);
            samples.push(tick_began.elapsed().as_micros() as f64);
        }
        let elapsed = began.elapsed();
        finish_capture(&session, &p.join("busy"));
        assert_eq!(
            session.tape.take_warning(),
            None,
            "capture must be complete"
        );
        let m = conduit_server::replay_capture::read_manifest(&p.join("busy")).unwrap();
        let bytes: u64 = m.segments.iter().map(|s| s.bytes).sum();
        let mut baseline_engine = Engine::new(baseline_cfg, 10_000.0);
        let mut baseline_broker = SimBroker::new(10_000.0, 0.0, 0.0);
        baseline_broker.on_quote(q);
        for message in &messages {
            baseline_engine.on_message_received(&mut baseline_broker, message, start - 10_800_000);
        }
        let mut baseline_samples = Vec::new();
        for n in 1..=1200 {
            let price = 3999.0 + (n % 31) as f64 * 0.08;
            let q = Quote {
                ts: start + n * 250,
                bid: price,
                ask: price + 0.2,
            };
            baseline_broker.on_quote(q);
            let began = std::time::Instant::now();
            baseline_engine.on_tick_received(&mut baseline_broker, &q, q.ts - 10_800_000);
            baseline_samples.push(began.elapsed().as_micros() as f64);
        }
        assert_eq!(
            broker.account().balance.to_bits(),
            baseline_broker.account().balance.to_bits()
        );
        assert_eq!(
            broker.account().equity.to_bits(),
            baseline_broker.account().equity.to_bits()
        );
        let baseline_mean = baseline_samples.iter().sum::<f64>() / baseline_samples.len() as f64;
        baseline_samples.sort_by(f64::total_cmp);
        samples.sort_by(f64::total_cmp);
        let perf = json!({"baseline_mean_us":baseline_mean,"baseline_p95_us":baseline_samples[baseline_samples.len()*95/100],"incremental_mean_us":samples.iter().sum::<f64>()/samples.len()as f64-baseline_mean,"mean_us":samples.iter().sum::<f64>()/samples.len()as f64,"p95_us":samples[samples.len()*95/100],"estimated_MiB_per_hour_at_10_ticks_s":bytes as f64/1200.0*36_000.0/1_048_576.0,"schema":"conduit.capture.synthetic-performance.v1","profile":"debug","ticks":1200,"baskets":engine.baskets.len(),"elapsed_ms":elapsed.as_millis(),"microseconds_per_tick":elapsed.as_micros()as f64/1200.0,"bytes":bytes,"bytes_per_tick_including_bootstrap":bytes as f64/1200.0});
        eprintln!("CONDUIT_CAPTURE_PERF {perf}");
        if let Ok(output) = std::env::var("CONDUIT_REPLAY_TEST_ARTIFACT_DIR") {
            let output = PathBuf::from(output);
            std::fs::create_dir_all(&output).unwrap();
            let retained = output.join(format!(
                "synthetic_busy_capture_{}",
                conduit_server::now_ms()
            ));
            std::fs::create_dir(&retained).unwrap();
            for entry in std::fs::read_dir(p.join("busy")).unwrap() {
                let entry = entry.unwrap();
                std::fs::copy(entry.path(), retained.join(entry.file_name())).unwrap();
            }
            std::fs::write(output.join("synthetic_capture_location.json"),serde_json::to_vec_pretty(&json!({"directory":retained,"capture_binary_sha256":binary_sha256().unwrap(),"capture_source_sha256":env!("CONDUIT_REPLAY_SOURCE_SHA256")})).unwrap()).unwrap();
            std::fs::write(
                output.join("synthetic_capture_performance.json"),
                serde_json::to_vec_pretty(&perf).unwrap(),
            )
            .unwrap();
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "replay_capture::tests::cross_process_reader",
                "--nocapture",
            ])
            .env("CONDUIT_SYNTHETIC_REPLAY_DIR", p.join("busy"))
            .status()
            .unwrap();
        assert!(
            status.success(),
            "independent process must reproduce all broker calls and private state"
        );
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn stale_pass_report_is_never_reused_for_a_new_bad_input() {
        let p = root();
        let report = p.join("report.json");
        std::fs::write(&report, b"{\"status\":\"PASS\"}").unwrap();
        let args = vec![
            "conduit".into(),
            "--replay-capture".into(),
            p.join("missing").display().to_string(),
            "--replay-report".into(),
            report.display().to_string(),
        ];
        assert!(maybe_cli(&args)
            .unwrap_err()
            .to_string()
            .contains("already exists"));
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn all_logs_exact_frames_roundtrip_and_corrupt_input_fail_offline() {
        let p = root();
        let session = Session::start(&p, "export_fixture").unwrap();
        let mut cfg = conduit_core::Settings::default();
        cfg.ai_enabled = false;
        cfg.ea_enabled = false;
        let mut e = Engine::new(cfg, 600.0);
        let mut b = SimBroker::new(600.0, 0.0, 0.0);
        let q = Quote {
            ts: 1782400000000,
            bid: 4000.0,
            ask: 4000.2,
        };
        b.on_quote(q);
        session.tick(&mut e, &mut b, &q, q.ts - 10_800_000);
        finish_capture(&session, &p.join("export_fixture"));
        let mut text = String::from("Synthetic allLogs header\n");
        for item in std::fs::read_dir(p.join("export_fixture")).unwrap() {
            let item = item.unwrap();
            let data = std::fs::read(item.path()).unwrap();
            let frame = json!({"run_id":"export_fixture","file":item.file_name().to_str().unwrap(),"bytes":data.len(),"sha256":format!("{:x}",Sha256::digest(&data)),"content":String::from_utf8(data).unwrap()});
            text.push_str("CONDUIT_REPLAY_FILE_V1 ");
            text.push_str(&frame.to_string());
            text.push('\n');
        }
        let input = p.join("alllogs.txt");
        std::fs::write(&input, &text).unwrap();
        let argv = |path: &Path, out: &str| {
            vec![
                "conduit".into(),
                "--replay-capture".into(),
                path.display().to_string(),
                "--replay-report".into(),
                p.join(out).display().to_string(),
            ]
        };
        assert!(maybe_cli(&argv(&input, "export-report.json")).unwrap());
        std::fs::write(
            &input,
            text.replace("CONDUIT_REPLAY_FILE_V1 ", "CONDUIT_REPLAY_FILE_V1 {invalid"),
        )
        .unwrap();
        assert!(maybe_cli(&argv(&input, "corrupt-report.json")).is_err());
        let report: Value =
            serde_json::from_slice(&std::fs::read(p.join("corrupt-report.json")).unwrap()).unwrap();
        assert_eq!(report["status"], "FAIL");
        assert_eq!(report["network_or_services_started"], false);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn manual_boundary_preserves_prior_prefix_and_new_hot_checkpoint() {
        let p = root();
        let first = Session::start(&p, "before_manual").unwrap();
        let mut cfg = conduit_core::Settings::default();
        cfg.ai_enabled = false;
        cfg.ea_enabled = false;
        let mut engine = Engine::new(cfg, 600.0);
        let mut broker = SimBroker::new(600.0, 0.0, 0.0);
        let q = Quote {
            ts: 1782400000000,
            bid: 4000.0,
            ask: 4000.2,
        };
        broker.on_quote(q);
        first.tick(&mut engine, &mut broker, &q, q.ts - 10_800_000);
        first.tape.append(
            "session_end",
            &json!({"reason":"manual_application_boundary","logical_capture_boundary":true}),
        );
        first.tape.finish_async();
        engine.resume_trading(q.ts + 1);
        let next = Session::checkpoint(
            &p,
            "after_manual",
            "before_manual",
            "manual_application_boundary_not_rederived",
        )
        .unwrap();
        let q = Quote {
            ts: q.ts + 250,
            ..q
        };
        broker.on_quote(q);
        next.tick(&mut engine, &mut broker, &q, q.ts - 10_800_000);
        finish_capture(&next, &p.join("after_manual"));
        let start = std::time::Instant::now();
        while !conduit_server::replay_capture::read_manifest(&p.join("before_manual"))
            .unwrap()
            .writer_closed
        {
            assert!(start.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            verify_directory(&p.join("before_manual"), &p.join("before.json")).unwrap()["frames"],
            1
        );
        assert_eq!(
            verify_directory(&p.join("after_manual"), &p.join("after.json")).unwrap()["frames"],
            1
        );
        let mut origin = Value::Null;
        conduit_server::replay_capture::read_events(&p.join("after_manual"), |e| {
            if e.seq == 1 {
                origin = e.payload;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(origin["predecessor_run_id"], "before_manual");
        assert_eq!(origin["application_interval_rederived"], false);
        assert_eq!(
            origin["origin"],
            "complete_engine_checkpoint_at_next_decision"
        );
        std::fs::remove_dir_all(p).unwrap();
    }
}
