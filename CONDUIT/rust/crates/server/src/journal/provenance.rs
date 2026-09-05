
use conduit_core::journal::{
    iso8601_broker, session_day_str, EventCategory, EventKind, EventLevel, JournalEvent,
};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub const PROVENANCE_SOURCE: &str = "runtime_provenance";
pub const PROVENANCE_VERSION: u32 = 1;

/// Dane stałe dla jednego połączenia żywej pętli z rachunkiem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContext {
    pub run_id: String,
    pub instance_id: String,
    pub account_login: i64,
    pub account_server: String,
    pub symbol: String,
    pub server_offset_ms: i64,
    pub session_offset_ms: i64,
}

/// Migawka wejściowa. Hash obejmuje wyłącznie config; runtime służy do
/// powiązania rekordu z konkretnymi buforami dziennika i nie zmienia odcisku.
#[derive(Debug, Clone)]
pub struct ProvenanceSnapshot {
    pub config: Value,
    pub runtime: Value,
}

impl ProvenanceSnapshot {
    pub fn new(config: Value) -> Self {
        ProvenanceSnapshot {
            config,
            runtime: Value::Null,
        }
    }

    pub fn with_runtime(mut self, runtime: Value) -> Self {
        self.runtime = runtime;
        self
    }
}

/// Tożsamość uruchomionej binarki. Celowo bez pełnej ścieżki procesu:
/// nazwa, rozmiar i SHA-256 wystarczają do identyfikacji, a nie ujawniają
/// nazwy konta systemowego ani układu katalogów maszyny.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BinaryIdentity {
    pub file_name: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BinaryIdentity {
    pub fn current() -> Self {
        match std::env::current_exe() {
            Ok(path) => Self::from_path(&path),
            Err(e) => BinaryIdentity {
                file_name: String::new(),
                bytes: 0,
                sha256: None,
                error: Some(format!("current_exe: {}", e.kind())),
            },
        }
    }

    pub fn from_path(path: &Path) -> Self {
        let file_name = path
            .file_name()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        match sha256_file(path) {
            Ok(sha256) => BinaryIdentity {
                file_name,
                bytes,
                sha256: Some(sha256),
                error: None,
            },
            Err(e) => BinaryIdentity {
                file_name,
                bytes,
                sha256: None,
                error: Some(format!("read: {}", e.kind())),
            },
        }
    }
}

/// Jeden identyfikator na życie procesu, wspólny dla kolejnych reconnectów.
pub fn process_instance_id() -> &'static str {
    static INSTANCE: OnceLock<String> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("proc-{}-{ms}", std::process::id())
    })
}

/// Unikalny identyfikator jednego żywego połączenia z rachunkiem.
pub fn new_run_id(prefix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{}-{ms}-{seq}", std::process::id())
}

/// Stan deduplikacji i numeracji rekordów provenance jednej sesji.
pub struct RuntimeProvenance {
    context: RuntimeContext,
    binary: BinaryIdentity,
    seq: u64,
    last_config_sha256: Option<String>,
}

impl RuntimeProvenance {
    pub fn new(context: RuntimeContext) -> Self {
        RuntimeProvenance {
            context,
            binary: BinaryIdentity::current(),
            seq: 0,
            last_config_sha256: None,
        }
    }

    /// Konstruktor z jawną ścieżką wyłącznie do testów i narzędzi offline.
    pub fn with_binary_path(context: RuntimeContext, path: &Path) -> Self {
        RuntimeProvenance {
            context,
            binary: BinaryIdentity::from_path(path),
            seq: 0,
            last_config_sha256: None,
        }
    }

    pub fn config_sha256(&self) -> Option<&str> {
        self.last_config_sha256.as_deref()
    }

    /// Pierwszy rekord sesji. Jest zapisywany zawsze, nawet gdy poziom
    /// journal_min_level odfiltrowuje zwykłe wpisy info.
    pub fn session_start(
        &mut self,
        ts_broker_ms: i64,
        snapshot: ProvenanceSnapshot,
    ) -> JournalEvent {
        let prepared = prepare_snapshot(snapshot);
        self.make_event("session_start", ts_broker_ms, prepared, None)
    }

    /// Rekord zmiany. Identyczna konfiguracja jest deduplikowana po SHA-256.
    pub fn config_change(
        &mut self,
        ts_broker_ms: i64,
        snapshot: ProvenanceSnapshot,
    ) -> Option<JournalEvent> {
        let prepared = prepare_snapshot(snapshot);
        if self.last_config_sha256.as_deref() == Some(prepared.sha256.as_str()) {
            return None;
        }
        let previous = self.last_config_sha256.clone();
        Some(self.make_event("config_change", ts_broker_ms, prepared, previous))
    }

    fn make_event(
        &mut self,
        change: &'static str,
        ts_broker_ms: i64,
        prepared: PreparedSnapshot,
        previous_config_sha256: Option<String>,
    ) -> JournalEvent {
        self.seq += 1;
        self.last_config_sha256 = Some(prepared.sha256.clone());

        let mut data = Map::new();
        data.insert(
            "record_type".into(),
            Value::String("runtime_provenance".into()),
        );
        data.insert("provenance_version".into(), Value::from(PROVENANCE_VERSION));
        data.insert("change".into(), Value::String(change.into()));
        data.insert("run_id".into(), Value::String(self.context.run_id.clone()));
        data.insert(
            "instance_id".into(),
            Value::String(self.context.instance_id.clone()),
        );
        data.insert(
            "account".into(),
            serde_json::json!({
                "login": self.context.account_login,
                "server": self.context.account_server,
            }),
        );
        data.insert("symbol".into(), Value::String(self.context.symbol.clone()));
        data.insert(
            "config_sha256".into(),
            Value::String(prepared.sha256.clone()),
        );
        if let Some(previous) = previous_config_sha256 {
            data.insert("previous_config_sha256".into(), Value::String(previous));
        }
        data.insert(
            "binary".into(),
            serde_json::to_value(&self.binary).unwrap_or(Value::Null),
        );
        data.insert("config".into(), prepared.config);
        if !prepared.runtime.is_null() {
            data.insert("runtime".into(), prepared.runtime);
        }

        let chain = data
            .get("config")
            .and_then(|v| v.get("active_chain"))
            .and_then(Value::as_str)
            .unwrap_or("—");
        let binary_hash = self.binary.sha256.as_deref().unwrap_or("brak");
        let text = format!(
            "Provenance {change}: run {} · instancja {} · konto {}@{} · {} · \
             łańcuch {} · config sha256 {} · binary sha256 {}",
            self.context.run_id,
            self.context.instance_id,
            self.context.account_login,
            self.context.account_server,
            self.context.symbol,
            chain,
            prepared.sha256,
            binary_hash,
        );

        JournalEvent {
            event_id: format!("{}#prov-{:08}", self.context.run_id, self.seq),
            ts_broker: iso8601_broker(ts_broker_ms, self.context.server_offset_ms),
            ts_broker_ms,
            level: EventLevel::Info,
            category: EventCategory::System,
            kind: EventKind::Note,
            session_day: session_day_str(ts_broker_ms, self.context.session_offset_ms),
            source: Some(PROVENANCE_SOURCE.into()),
            text,
            data,
            ..JournalEvent::default()
        }
    }
}

struct PreparedSnapshot {
    config: Value,
    runtime: Value,
    sha256: String,
}

fn prepare_snapshot(snapshot: ProvenanceSnapshot) -> PreparedSnapshot {
    let config = canonical_safe(snapshot.config);
    let runtime = canonical_safe(snapshot.runtime);
    let bytes = serde_json::to_vec(&config).unwrap_or_else(|_| b"null".to_vec());
    let sha256 = sha256_bytes(&bytes);
    PreparedSnapshot {
        config,
        runtime,
        sha256,
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hash.update(&buf[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

/// Sortuje klucze rekurencyjnie i usuwa pola poświadczeń przed hashowaniem.
/// Usuwamy je, zamiast zastępować ich hashem: hash krótkiego hasła też może
/// ułatwić zgadywanie. Nazwy takie jak session_hours nie są tajne i zostają.
fn canonical_safe(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .into_iter()
                .filter(|(key, _)| !sensitive_key(key))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = Map::new();
            for (key, value) in entries {
                out.insert(key, canonical_safe(value));
            }
            Value::Object(out)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_safe).collect()),
        other => other,
    }
}

fn sensitive_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "password"
            | "passwd"
            | "mt5password"
            | "smtppassword"
            | "apihash"
            | "secret"
            | "secrets"
            | "sessionstring"
            | "sessionblob"
            | "telegramsession"
            | "sessiontoken"
            | "authtoken"
            | "accesstoken"
            | "refreshtoken"
            | "bottoken"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RuntimeContext {
        RuntimeContext {
            run_id: "live-test".into(),
            instance_id: "proc-test".into(),
            account_login: 123456,
            account_server: "Broker-Demo".into(),
            symbol: "XAUUSD".into(),
            server_offset_ms: 3 * 3_600_000,
            session_offset_ms: 0,
        }
    }

    fn binary_file(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "conduit-provenance-{tag}-{}-{}.bin",
            std::process::id(),
            new_run_id("test")
        ));
        std::fs::write(&path, b"abc").unwrap();
        path
    }

    #[test]
    fn start_niesie_tozsamosc_hash_i_pelna_bezpieczna_konfiguracje() {
        let path = binary_file("start");
        let mut recorder = RuntimeProvenance::with_binary_path(context(), &path);
        let config = serde_json::json!({
            "active_chain": "GOD-X4",
            "engines": [{
                "format": "ZEN",
                "preset": "FRESHQUEEN-5",
                "settings": {
                    "lot_fixed": 0.01,
                    "session_hours": "9-15",
                    "password": "NIE_WOLNO",
                    "mt5_password": "MT5_TEZ_NIE_WOLNO",
                    "apiHash": "TEZ_NIE_WOLNO",
                    "sessionString": "ANI_TO"
                }
            }]
        });
        let mut event = recorder.session_start(1_784_800_800_000, ProvenanceSnapshot::new(config));
        event.stamp_wall(1_784_800_800_000, 0);
        let json = serde_json::to_string(&event).unwrap();

        assert_eq!(event.kind, EventKind::Note);
        assert_eq!(event.source.as_deref(), Some(PROVENANCE_SOURCE));
        assert_eq!(event.event_id, "live-test#prov-00000001");
        assert_eq!(event.data["change"], "session_start");
        assert_eq!(event.data["run_id"], "live-test");
        assert_eq!(event.data["instance_id"], "proc-test");
        assert_eq!(event.data["account"]["login"], 123456);
        assert_eq!(event.data["account"]["server"], "Broker-Demo");
        assert_eq!(event.data["symbol"], "XAUUSD");
        assert_eq!(event.data["config"]["active_chain"], "GOD-X4");
        assert_eq!(
            event.data["binary"]["sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(event.data["config_sha256"].as_str().unwrap().len(), 64);
        assert!(json.contains("session_hours"), "zwykłe pole sesji zniknęło");
        for secret in ["NIE_WOLNO", "MT5_TEZ_NIE_WOLNO", "TEZ_NIE_WOLNO", "ANI_TO"] {
            assert!(
                !json.contains(secret),
                "sekret wyciekł do provenance: {secret}"
            );
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn zmiana_jest_deduplikowana_i_wskazuje_poprzedni_hash() {
        let path = binary_file("change");
        let mut recorder = RuntimeProvenance::with_binary_path(context(), &path);
        let a: Value =
            serde_json::from_str(r#"{"active_chain":"A","settings":{"b":2,"a":1}}"#).unwrap();
        let a_inny_porządek: Value =
            serde_json::from_str(r#"{"settings":{"a":1,"b":2},"active_chain":"A"}"#).unwrap();
        let b = serde_json::json!({
            "active_chain": "B",
            "settings": {"a": 1, "b": 2}
        });

        let start = recorder.session_start(1, ProvenanceSnapshot::new(a));
        let first_hash = start.data["config_sha256"].as_str().unwrap().to_string();
        assert!(
            recorder
                .config_change(2, ProvenanceSnapshot::new(a_inny_porządek))
                .is_none(),
            "kolejność kluczy nie jest zmianą konfiguracji"
        );
        let change = recorder
            .config_change(3, ProvenanceSnapshot::new(b))
            .expect("inna konfiguracja");
        assert_eq!(change.data["change"], "config_change");
        assert_eq!(change.data["previous_config_sha256"], first_hash);
        assert_ne!(
            change.data["config_sha256"],
            change.data["previous_config_sha256"]
        );
        assert_eq!(change.event_id, "live-test#prov-00000002");
        let _ = std::fs::remove_file(path);
    }
}
