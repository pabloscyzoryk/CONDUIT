//! Bounded, private engine-input tapes. Trading never waits for disk writes.
//! A manifest commits an exact flushed prefix, not an inferred whole session.
use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

pub const RUN_BUDGET: u64 = 512 * 1024 * 1024;
const SEGMENT_BUDGET: u64 = 16 * 1024 * 1024;
const QUEUE_BUDGET: usize = 16 * 1024 * 1024;
const RECORD_BUDGET: usize = 8 * 1024 * 1024;
pub const MANIFEST_SCHEMA: &str = "conduit.live-capture.manifest.v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub kind: String,
    pub payload: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    pub first_seq: u64,
    pub last_seq: u64,
    pub sealed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub run_id: String,
    pub status: String,
    pub writer_closed: bool,
    pub qualification_scope: String,
    pub origin: bool,
    pub first_seq: u64,
    pub exact_replay_qualified: bool,
    pub last_seq: u64,
    pub segments: Vec<Segment>,
    pub gaps: Vec<String>,
    pub binary_sha256: String,
    pub created_utc: i64,
    pub limits: Value,
}
#[derive(Serialize, Deserialize)]
struct Batch {
    codec: String,
    first_seq: u64,
    last_seq: u64,
    uncompressed_bytes: usize,
    payload_b64: String,
}
enum Command {
    Event(Vec<u8>),
    Finish(mpsc::Sender<()>),
}
struct Signals {
    queued: AtomicUsize,
    written: AtomicUsize,
    failure: Mutex<Option<String>>,
    warned: AtomicBool,
    finished: AtomicBool,
}
impl Signals {
    fn fail(&self, reason: impl Into<String>) {
        let mut e = self.failure.lock().unwrap_or_else(|p| p.into_inner());
        if e.is_none() {
            let reason = reason.into();
            tracing::warn!(capture_status="incomplete",reason=%reason,"Replay capture incomplete");
            *e = Some(reason);
        }
    }
}
struct Sender {
    tx: mpsc::SyncSender<Command>,
    seq: u64,
}
#[derive(Clone)]
pub struct Capture {
    sender: Arc<Mutex<Sender>>,
    signals: Arc<Signals>,
    pub run_id: String,
}
impl Capture {
    pub fn start(root: &Path, run_id: &str, binary_sha256: &str) -> Result<Self> {
        if !safe_component(run_id) {
            bail!("invalid capture run identifier");
        }
        fs::create_dir_all(root)?;
        no_reparse(root)?;
        let mut writer = Writer::new(root, run_id, binary_sha256)?;
        let (tx, rx) = mpsc::sync_channel(256);
        let signals = Arc::new(Signals {
            queued: AtomicUsize::new(0),
            written: AtomicUsize::new(0),
            failure: Mutex::new(None),
            warned: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        });
        let shared = signals.clone();
        std::thread::Builder::new()
            .name("replay-capture".into())
            .spawn(move || {
                let mut closed = false;
                loop {
                    match rx.recv_timeout(Duration::from_millis(250)) {
                        Ok(Command::Event(bytes)) => {
                            shared.queued.fetch_sub(bytes.len(), Ordering::Relaxed);
                            if shared
                                .failure
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .is_none()
                            {
                                if let Err(e) = writer.push(&bytes) {
                                    shared.fail(format!("capture write failed: {e}"));
                                }
                            }
                        }
                        Ok(Command::Finish(done)) => {
                            closed = true;
                            writer.finish(&shared, closed);
                            shared.finished.store(true, Ordering::Release);
                            let _ = done.send(());
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if let Err(e) = writer.flush(false) {
                                shared.fail(format!("capture flush failed: {e}"));
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    shared
                        .written
                        .store(writer.total as usize, Ordering::Release);
                    if let Some(reason) = shared
                        .failure
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone()
                    {
                        writer.gap(&reason);
                        let _ = writer.commit();
                    }
                }
                if !closed {
                    writer.finish(&shared, false);
                    shared.finished.store(true, Ordering::Release);
                }
            })?;
        Ok(Self {
            sender: Arc::new(Mutex::new(Sender { tx, seq: 0 })),
            signals,
            run_id: run_id.into(),
        })
    }
    /// Best effort, bounded and ordered. A refused event makes qualification fail.
    pub fn append<T: Serialize>(&self, kind: &str, payload: &T) {
        if self.signals.finished.load(Ordering::Acquire) {
            self.signals.fail("event after capture closed");
            return;
        }
        if self
            .signals
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
        {
            return;
        }
        let mut sender = self.sender.lock().unwrap_or_else(|p| p.into_inner());
        sender.seq += 1;
        let bytes =
            match serde_json::to_vec(&json!({"seq":sender.seq,"kind":kind,"payload":payload})) {
                Ok(v) => v,
                Err(_) => {
                    self.signals.fail("capture serialization failed");
                    return;
                }
            };
        if bytes.len() > RECORD_BUDGET {
            self.signals
                .fail("capture event exceeds bounded record size");
            return;
        }
        let n = bytes.len();
        let old = self.signals.queued.fetch_add(n, Ordering::Relaxed);
        if old.saturating_add(n) > QUEUE_BUDGET {
            self.signals.queued.fetch_sub(n, Ordering::Relaxed);
            self.signals
                .fail("capture queue budget exceeded; input gap");
            return;
        }
        if sender.tx.try_send(Command::Event(bytes)).is_err() {
            self.signals.queued.fetch_sub(n, Ordering::Relaxed);
            self.signals.fail("capture queue unavailable; input gap");
        }
    }
    pub fn active(&self) -> bool {
        !self.signals.finished.load(Ordering::Acquire)
            && self
                .signals
                .failure
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_none()
    }
    pub fn invalidate(&self, reason: &str) {
        self.signals.fail(reason);
    }
    pub fn take_warning(&self) -> Option<String> {
        let e = self
            .signals
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if e.is_some() && !self.signals.warned.swap(true, Ordering::AcqRel) {
            e
        } else {
            None
        }
    }
    /// Rotate only between Engine decisions. The 64 MiB soft limit leaves room
    /// for an in-flight batch and a new checkpoint under the global 512 MiB cap.
    pub fn rotation_due(&self) -> bool {
        self.active() && self.signals.written.load(Ordering::Acquire) >= 64 * 1024 * 1024
    }
    pub fn finish_async(&self) {
        let (tx, _) = mpsc::channel();
        if self
            .sender
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tx
            .try_send(Command::Finish(tx))
            .is_err()
        {
            self.signals.fail("capture finish queue unavailable");
        }
    }
    /// Only called when leaving the trading session, never per market event.
    pub fn finish(&self) {
        let (tx, rx) = mpsc::channel();
        let mut command = Command::Finish(tx);
        let start = Instant::now();
        loop {
            let r = self
                .sender
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .tx
                .try_send(command);
            match r {
                Ok(()) => {
                    // A slow worker can finish after the bounded caller wait.
                    // Its manifest remains an open committed prefix until then.
                    let _ = rx.recv_timeout(Duration::from_secs(2));
                    break;
                }
                Err(mpsc::TrySendError::Full(c)) if start.elapsed() < Duration::from_secs(1) => {
                    command = c;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => {
                    self.signals.fail("capture finish queue unavailable");
                    break;
                }
            }
        }
    }
}

struct Writer {
    root: PathBuf,
    dir: PathBuf,
    manifest: Manifest,
    file: File,
    hash: Sha256,
    compressor: flate2::write::DeflateEncoder<Vec<u8>>,
    buffer: Vec<u8>,
    first: u64,
    last: u64,
    last_commit: Instant,
    total: u64,
    inflight: std::collections::BTreeSet<u64>,
}
impl Writer {
    fn new(root: &Path, id: &str, binary: &str) -> Result<Self> {
        let root = root.canonicalize()?;
        let _ = prune(&root, 0)?;
        let dir = root.join(id);
        fs::create_dir(&dir)?;
        no_reparse(&dir)?;
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(dir.join("segment-000001.jsonl"))?;
        let manifest = Manifest {
            schema: MANIFEST_SCHEMA.into(),
            run_id: id.into(),
            status: "open".into(),
            writer_closed: false,
            qualification_scope: "rules_engine_given_recorded_external_state_updates_v1".into(),
            origin: false,
            first_seq: 0,
            exact_replay_qualified: false,
            last_seq: 0,
            segments: vec![Segment {
                file: "segment-000001.jsonl".into(),
                bytes: 0,
                sha256: format!("{:x}", Sha256::digest([])),
                first_seq: 0,
                last_seq: 0,
                sealed: false,
            }],
            gaps: vec![],
            binary_sha256: binary.into(),
            created_utc: crate::now_ms(),
            limits: json!({"run_bytes":RUN_BUDGET,"total_recording_bytes":RUN_BUDGET,"segment_bytes":SEGMENT_BUDGET,"queue_bytes":QUEUE_BUDGET,"record_bytes":RECORD_BUDGET}),
        };
        let mut w = Self {
            root,
            dir,
            manifest,
            file,
            hash: Sha256::new(),
            compressor: flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast()),
            buffer: vec![],
            first: 0,
            last: 0,
            last_commit: Instant::now(),
            total: 0,
            inflight: Default::default(),
        };
        w.commit()?;
        Ok(w)
    }
    fn push(&mut self, bytes: &[u8]) -> Result<()> {
        let event: Event = serde_json::from_slice(bytes)?;
        if event.seq != self.last + 1 {
            bail!("event sequence discontinuity");
        }
        if event.seq == 1 {
            if event.kind != "session_origin" {
                bail!("session origin missing");
            }
            self.manifest.origin = true;
            self.manifest.first_seq = 1;
            self.manifest.exact_replay_qualified = true;
        }
        if event.kind == "engine_begin" {
            let id = event.payload["action_seq"]
                .as_u64()
                .context("engine begin sequence missing")?;
            if !self.inflight.insert(id) {
                bail!("duplicate inflight engine action");
            }
        }
        if event.kind == "engine_frame" {
            let id = event.payload["action_seq"]
                .as_u64()
                .context("engine frame sequence missing")?;
            if !self.inflight.remove(&id) {
                bail!("engine frame has no beginning");
            }
        }
        if self.buffer.is_empty() {
            self.first = event.seq;
        }
        self.last = event.seq;
        self.buffer.extend_from_slice(bytes);
        self.buffer.push(b'\n');
        if self.buffer.len() >= 256 * 1024 {
            self.flush(false)?;
        }
        Ok(())
    }
    fn flush(&mut self, force: bool) -> Result<()> {
        if !self.buffer.is_empty() {
            self.compressor.write_all(&self.buffer)?;
            self.compressor.flush()?;
            let zipped = std::mem::take(self.compressor.get_mut());
            let batch = Batch {
                codec: "deflate_stream_chunk_v1".into(),
                first_seq: self.first,
                last_seq: self.last,
                uncompressed_bytes: self.buffer.len(),
                payload_b64: base64::engine::general_purpose::STANDARD.encode(zipped),
            };
            let mut line = serde_json::to_vec(&batch)?;
            line.push(b'\n');
            if self.total + line.len() as u64 > RUN_BUDGET - 1024 * 1024 {
                bail!("run disk budget exceeded; recording stopped");
            }
            // Serialize reservations from all capture writers in this process;
            // count actual files, including data beyond committed manifests.
            static BUDGET_LOCK: Mutex<()> = Mutex::new(());
            let _budget = BUDGET_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let global = prune(&self.root, line.len() as u64)?;
            if global.saturating_add(line.len() as u64) > RUN_BUDGET - 1024 * 1024 {
                bail!("total recording disk budget exceeded; active origins preserved");
            }
            if self.manifest.segments.last().unwrap().bytes + line.len() as u64 > SEGMENT_BUDGET {
                self.manifest.segments.last_mut().unwrap().sealed = true;
                self.commit()?;
                let name = format!("segment-{:06}.jsonl", self.manifest.segments.len() + 1);
                self.file = File::options()
                    .create_new(true)
                    .write(true)
                    .open(self.dir.join(&name))?;
                self.hash = Sha256::new();
                self.manifest.segments.push(Segment {
                    file: name,
                    bytes: 0,
                    sha256: format!("{:x}", Sha256::digest([])),
                    first_seq: 0,
                    last_seq: 0,
                    sealed: false,
                });
            }
            self.file.write_all(&line)?;
            self.hash.update(&line);
            self.total += line.len() as u64;
            let s = self.manifest.segments.last_mut().unwrap();
            if s.first_seq == 0 {
                s.first_seq = self.first;
            }
            s.last_seq = self.last;
            s.bytes += line.len() as u64;
            s.sha256 = format!("{:x}", self.hash.clone().finalize());
            self.manifest.last_seq = self.last;
            self.buffer.clear();
        }
        if force || self.last_commit.elapsed() >= Duration::from_secs(5) {
            self.commit()?;
        }
        Ok(())
    }
    fn gap(&mut self, reason: &str) {
        if !self.manifest.gaps.iter().any(|r| r == reason) {
            self.manifest.gaps.push(reason.into());
        }
        self.manifest.status = "incomplete".into();
        self.manifest.exact_replay_qualified = false;
    }
    fn commit(&mut self) -> Result<()> {
        // An active allLogs export sees only a complete Engine action boundary.
        // Bytes already appended beyond that manifest prefix remain private
        // until their frame is complete, or a declared gap is committed.
        if !self.inflight.is_empty() && self.manifest.gaps.is_empty() {
            return Ok(());
        }
        self.file.flush()?;
        self.file.sync_data()?;
        atomic_json(&self.dir.join("manifest.json"), &self.manifest)?;
        self.last_commit = Instant::now();
        Ok(())
    }
    fn finish(&mut self, signals: &Signals, closed: bool) {
        if !self.inflight.is_empty() {
            self.gap("capture ended inside an engine action");
        }
        if let Err(e) = self.flush(true) {
            self.gap(&format!("final flush failed: {e}"));
        }
        if let Some(e) = signals
            .failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            self.gap(&e);
        }
        if !closed {
            self.gap("session writer disconnected without closed boundary");
        }
        if self.manifest.gaps.is_empty() {
            self.manifest.status = "closed".into();
        }
        self.manifest.writer_closed = true;
        self.manifest.segments.last_mut().unwrap().sealed = true;
        let _ = self.commit();
    }
}
fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = File::create(&tmp)?;
    f.write_all(&serde_json::to_vec(value)?)?;
    f.sync_all()?;
    drop(f);
    fs::rename(tmp, path)?;
    Ok(())
}
pub fn safe_component(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 200
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub fn safe_segment(s: &str) -> bool {
    s.strip_prefix("segment-")
        .and_then(|v| v.strip_suffix(".jsonl"))
        .is_some_and(|v| v.len() == 6 && v.bytes().all(|b| b.is_ascii_digit()))
}
pub fn no_reparse(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut p = PathBuf::new();
    for c in absolute.components() {
        p.push(c);
        // A Windows verbatim drive prefix (\\?\C:) is not a filesystem entry
        // until RootDir has been appended. Inspect the actual root next.
        if matches!(c, std::path::Component::Prefix(_)) {
            continue;
        }
        let m = fs::symlink_metadata(&p).context("capture path metadata")?;
        if m.file_type().is_symlink() {
            bail!("capture path is a symlink");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                bail!("capture path is a reparse point");
            }
        }
    }
    Ok(())
}
fn directory_bytes(path: &Path) -> Result<u64> {
    no_reparse(path)?;
    let mut n = 0;
    for e in fs::read_dir(path)? {
        let e = e?;
        no_reparse(&e.path())?;
        let m = e.metadata()?;
        if !m.is_file() {
            bail!("unexpected directory inside capture run");
        }
        n += m.len();
    }
    Ok(n)
}
fn prune(root: &Path, reserved: u64) -> Result<u64> {
    no_reparse(root)?;
    let mut total = 0u64;
    let mut closed = vec![];
    for e in fs::read_dir(root)? {
        let e = e?;
        no_reparse(&e.path())?;
        let m = e.metadata()?;
        if m.is_file() {
            total += m.len();
            continue;
        }
        if !m.is_dir() {
            bail!("unexpected capture root entry");
        }
        let n = directory_bytes(&e.path())?;
        total += n;
        if let Ok(m) = read_manifest(&e.path()) {
            if m.writer_closed
                && safe_component(&m.run_id)
                && e.file_name().to_str() == Some(&m.run_id)
            {
                closed.push((m.created_utc, e.path(), m.run_id, n));
            }
        }
    }
    closed.sort_by_key(|r| r.0);
    let retention = root.join("retention.json");
    let mut journal: Value = if retention.exists() {
        serde_json::from_slice(&fs::read(&retention)?)?
    } else {
        json!({"schema":"conduit.live-capture.retention.v1","budget_bytes":RUN_BUDGET,"total_deleted_count":0,"deleted_runs":[]})
    };
    let mut changed = false;
    for (_, path, id, n) in closed {
        if total.saturating_add(reserved) < RUN_BUDGET - 1024 * 1024 {
            break;
        }
        // Validate every target before deleting this one closed run. No recursive
        // traversal and no deletion of active origins, junctions or unknown data.
        let files: Vec<_> = fs::read_dir(&path)?.collect::<std::io::Result<Vec<_>>>()?;
        if files.iter().any(|e| {
            let s = e.file_name().to_string_lossy().into_owned();
            s != "manifest.json" && s != "manifest.tmp" && !safe_segment(&s)
        }) {
            continue;
        }
        for e in &files {
            no_reparse(&e.path())?;
            if !e.metadata()?.is_file() {
                bail!("unsafe retention target");
            }
        }
        let record = json!({"run_id":id,"bytes":n,"removed_utc":crate::now_ms(),"reason":"total_recording_budget"});
        let count = journal["total_deleted_count"].as_u64().unwrap_or(0) + 1;
        journal["total_deleted_count"] = json!(count);
        let list = journal["deleted_runs"]
            .as_array_mut()
            .context("invalid retention journal")?;
        list.push(record);
        if list.len() > 512 {
            list.remove(0);
        }
        atomic_json(&retention, &journal)?;
        for e in files {
            fs::remove_file(e.path())?;
        }
        fs::remove_dir(path)?;
        total = total.saturating_sub(n);
        changed = true;
    }
    if changed {
        atomic_json(&retention, &journal)?;
    }
    Ok(total)
}
pub fn read_manifest(dir: &Path) -> Result<Manifest> {
    no_reparse(dir)?;
    let p = dir.join("manifest.json");
    no_reparse(&p)?;
    if fs::metadata(&p)?.len() > 2 * 1024 * 1024 {
        bail!("manifest too large");
    }
    let m: Manifest = serde_json::from_slice(&fs::read(p)?)?;
    if m.schema != MANIFEST_SCHEMA || !safe_component(&m.run_id) {
        bail!("unsupported capture manifest");
    }
    Ok(m)
}
/// Streaming reader: validates every committed prefix before exposing events.
pub fn read_events(dir: &Path, mut consume: impl FnMut(Event) -> Result<()>) -> Result<Manifest> {
    let m = read_manifest(dir)?;
    if !m.origin
        || m.first_seq != 1
        || !m.exact_replay_qualified
        || !m.gaps.is_empty()
        || !matches!(m.status.as_str(), "open" | "closed")
    {
        bail!("capture has no qualified origin or contains gaps");
    }
    let mut seq = 0;
    let mut total = 0u64;
    let mut names = std::collections::BTreeSet::new();
    let mut decompressor = flate2::Decompress::new(false);
    for segment in &m.segments {
        if !safe_segment(&segment.file)
            || !names.insert(&segment.file)
            || segment.bytes > 64 * 1024 * 1024
        {
            bail!("invalid segment contract");
        }
        total = total
            .checked_add(segment.bytes)
            .context("capture size overflow")?;
        if total > RUN_BUDGET {
            bail!("capture run too large");
        }
        if segment.bytes > 0 && segment.first_seq != seq + 1 {
            bail!("segment first sequence mismatch");
        }
        let path = dir.join(&segment.file);
        no_reparse(&path)?;
        let mut bytes = vec![0; segment.bytes as usize];
        File::open(path)?.read_exact(&mut bytes)?;
        if format!("{:x}", Sha256::digest(&bytes)) != segment.sha256 {
            bail!("capture segment digest mismatch");
        }
        for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let batch: Batch = serde_json::from_slice(line)?;
            if batch.codec != "deflate_stream_chunk_v1"
                || batch.uncompressed_bytes > RECORD_BUDGET + 256 * 1024
                || batch.first_seq != seq + 1
            {
                bail!("invalid compressed batch contract");
            }
            let zip = base64::engine::general_purpose::STANDARD.decode(batch.payload_b64)?;
            let before_in = decompressor.total_in();
            let before_out = decompressor.total_out();
            let mut plain = vec![0u8; batch.uncompressed_bytes + 1];
            decompressor.decompress(&zip, &mut plain, flate2::FlushDecompress::Sync)?;
            let actual = (decompressor.total_out() - before_out) as usize;
            if actual != batch.uncompressed_bytes
                || decompressor.total_in() - before_in != zip.len() as u64
            {
                bail!("capture decompression length mismatch");
            }
            plain.truncate(actual);
            for row in plain.split(|b| *b == b'\n').filter(|r| !r.is_empty()) {
                let e: Event = serde_json::from_slice(row)?;
                if e.seq != seq + 1 {
                    bail!("capture event sequence gap");
                }
                seq = e.seq;
                consume(e)?;
            }
            if seq != batch.last_seq {
                bail!("compressed batch last sequence mismatch");
            }
        }
        if segment.bytes > 0 && seq != segment.last_seq {
            bail!("segment sequence mismatch");
        }
    }
    if seq != m.last_seq || seq == 0 {
        bail!("manifest prefix sequence mismatch");
    }
    Ok(m)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn dir() -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "conduit-capture-{}-{}-{}",
            std::process::id(),
            crate::now_ms(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn compressed_prefix_roundtrip_and_corruption_fail_closed() {
        let p = dir();
        let c = Capture::start(&p, "synthetic_run", "synthetic_sha").unwrap();
        c.append("session_origin", &json!({"fixture":true}));
        for n in 0..1000 {
            c.append("test", &json!({"n":n,"repeat":"same constant value"}));
            if n % 32 == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        c.finish();
        let mut count = 0;
        let m = read_events(&p.join("synthetic_run"), |_| {
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 1001);
        assert_eq!(m.status, "closed");
        let f = p.join("synthetic_run").join(&m.segments[0].file);
        let mut bytes = fs::read(&f).unwrap();
        bytes[10] ^= 1;
        fs::write(f, bytes).unwrap();
        assert!(read_events(&p.join("synthetic_run"), |_| Ok(())).is_err());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn missing_origin_and_declared_gap_never_qualify() {
        let p = dir();
        let c = Capture::start(&p, "gap", "sha").unwrap();
        c.append("not_origin", &json!({}));
        c.finish();
        assert!(read_events(&p.join("gap"), |_| Ok(())).is_err());
        assert!(c.take_warning().is_some());
        assert!(c.take_warning().is_none());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn unsafe_components_and_oversized_records_are_refused() {
        assert!(!safe_component("../escape"));
        assert!(!safe_segment("segment-../../x.jsonl"));
        let p = dir();
        let c = Capture::start(&p, "large", "sha").unwrap();
        c.append("session_origin", &json!({}));
        c.append("large", &"x".repeat(RECORD_BUDGET + 1));
        c.finish();
        assert!(read_events(&p.join("large"), |_| Ok(())).is_err());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn active_manifest_never_publishes_an_unfinished_engine_action() {
        let root = dir();
        let mut w = Writer::new(&root, "prefix", "fixture-sha").unwrap();
        let row = |seq, kind: &str, payload: Value| {
            serde_json::to_vec(&Event {
                seq,
                kind: kind.into(),
                payload,
            })
            .unwrap()
        };
        w.push(&row(1, "session_origin", json!({}))).unwrap();
        w.flush(true).unwrap();
        assert_eq!(read_manifest(&root.join("prefix")).unwrap().last_seq, 1);
        w.push(&row(2, "engine_begin", json!({"action_seq":1})))
            .unwrap();
        w.flush(true).unwrap();
        assert_eq!(
            read_manifest(&root.join("prefix")).unwrap().last_seq,
            1,
            "inflight bytes cannot enter a qualified prefix"
        );
        w.push(&row(3, "engine_frame", json!({"action_seq":1})))
            .unwrap();
        w.flush(true).unwrap();
        assert_eq!(read_manifest(&root.join("prefix")).unwrap().last_seq, 3);
        drop(w);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn soft_rotation_precedes_hard_limit_and_does_not_revive_failed_capture() {
        let root = dir();
        let c = Capture::start(&root, "rotation", "fixture").unwrap();
        c.append("session_origin", &json!({}));
        assert!(!c.rotation_due());
        c.signals
            .written
            .store(64 * 1024 * 1024 - 1, Ordering::Release);
        assert!(!c.rotation_due());
        c.signals.written.store(64 * 1024 * 1024, Ordering::Release);
        assert!(c.rotation_due());
        assert!((64 * 1024 * 1024) < RUN_BUDGET);
        c.invalidate("synthetic input gap");
        assert!(
            !c.rotation_due(),
            "a missing input is not a normal checkpoint boundary"
        );
        c.finish();
        assert!(read_events(&root.join("rotation"), |_| Ok(())).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
