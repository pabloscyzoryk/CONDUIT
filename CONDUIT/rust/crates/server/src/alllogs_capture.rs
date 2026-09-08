//! Lossless export of the recorder's committed prefix. Never date-filter a tape:
//! its bootstrap and every intermediate call are prerequisites for replay.
use super::{naglowek, Pozycja, ANULUJ};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, io::Read, path::Path, sync::atomic::Ordering};

const MANIFEST_LIMIT: u64 = 2 * 1024 * 1024;
const SEGMENT_LIMIT: u64 = 64 * 1024 * 1024;
const RUN_LIMIT: u64 = 512 * 1024 * 1024;
const EXPORT_TEXT_LIMIT: usize = 1024 * 1024 * 1024;
const RUN_COUNT_LIMIT: usize = 4096;

fn no_reparse_ancestors(path: &Path) -> Result<()> {
    let mut current = std::path::PathBuf::new();
    for part in path.components() {
        current.push(part.as_os_str());
        // A verbatim Windows drive prefix (\\?\C:) is not an entry until
        // RootDir is appended. Check that actual root and every descendant.
        if matches!(part, std::path::Component::Prefix(_)) {
            continue;
        }
        let meta = std::fs::symlink_metadata(&current)?;
        if meta.file_type().is_symlink() {
            bail!("symlink in capture path");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                bail!("reparse point in capture path");
            }
        }
    }
    Ok(())
}

fn safe_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn segment_name(name: &str) -> bool {
    name.strip_prefix("segment-")
        .and_then(|n| n.strip_suffix(".jsonl"))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
}

fn read_prefix(path: &Path, size: u64, limit: u64) -> Result<Vec<u8>> {
    if size > limit {
        bail!("file exceeds capture export limit");
    }
    no_reparse_ancestors(path)?;
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        bail!("not a regular capture file");
    }
    let mut bytes = vec![0; size as usize];
    std::fs::File::open(path)?.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn emit(out: &mut String, run_id: &str, file: &str, bytes: &[u8]) -> Result<()> {
    let content = std::str::from_utf8(bytes).context("capture file is not UTF-8")?;
    #[derive(serde::Serialize)]
    struct FileRecord<'a> {
        run_id: &'a str,
        file: &'a str,
        bytes: usize,
        sha256: String,
        content: &'a str,
    }
    let record = serde_json::to_string(&FileRecord {
        run_id,
        file,
        bytes: bytes.len(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
        content,
    })?;
    if out.len().saturating_add(record.len()).saturating_add(32) > EXPORT_TEXT_LIMIT {
        bail!("global allLogs text budget exceeded; capture export incomplete");
    }
    // One prefixed JSON record per file; JSON escapes preserve LF, CR and text
    // exactly. Ordinary chronological merge must never parse/reorder this data.
    writeln!(out, "CONDUIT_REPLAY_FILE_V1 {record}")?;
    Ok(())
}

fn export_run(
    out: &mut String,
    dir: &Path,
    run_id: &str,
    position: &mut Pozycja,
    cancelable: bool,
) -> Result<Value> {
    let manifest_path = dir.join("manifest.json");
    let manifest_size = std::fs::metadata(&manifest_path)?.len();
    let bytes = read_prefix(&manifest_path, manifest_size, MANIFEST_LIMIT)?;
    let manifest: Value = serde_json::from_slice(&bytes)?;
    if manifest["schema"] != "conduit.live-capture.manifest.v1" || manifest["run_id"] != run_id {
        bail!("capture manifest schema or run identifier mismatch");
    }
    emit(out, run_id, "manifest.json", &bytes)?;
    position.rekordow += 1;
    position.bajtow += bytes.len() as u64;
    let segments = manifest["segments"]
        .as_array()
        .context("manifest segments missing")?;
    let mut total = 0u64;
    let mut names = std::collections::HashSet::new();
    let mut gaps = Vec::new();
    for segment in segments {
        if cancelable && ANULUJ.load(Ordering::Relaxed) {
            bail!("scalanie anulowane przez użytkownika");
        }
        let name = segment["file"]
            .as_str()
            .context("segment filename missing")?;
        if !segment_name(name) || !names.insert(name.to_owned()) {
            bail!("unsafe or duplicate segment filename");
        }
        let size = segment["bytes"]
            .as_u64()
            .context("segment byte length missing")?;
        total = total.checked_add(size).context("capture size overflow")?;
        if total > RUN_LIMIT {
            bail!("run exceeds capture export budget");
        }
        // Manifest is an atomic snapshot. The open segment may already contain
        // a later suffix; read only the published prefix, not the moving EOF.
        match read_prefix(&dir.join(name), size, SEGMENT_LIMIT) {
            Ok(data) => {
                let digest = format!("{:x}", Sha256::digest(&data));
                if segment["sha256"].as_str() != Some(digest.as_str()) {
                    gaps.push(format!("{name}: sha256 mismatch"));
                }
                emit(out, run_id, name, &data)?;
                position.rekordow += 1;
                position.bajtow += data.len() as u64;
            }
            Err(error) => gaps.push(format!("{name}: {error}")),
        }
    }
    Ok(json!({
        "run_id": run_id,
        "status": if gaps.is_empty() { "committed_prefix_exported" } else { "incomplete" },
        "capture_status": manifest["status"],
        "capture_qualified": manifest["exact_replay_qualified"],
        "capture_gaps": manifest["gaps"],
        "first_seq": manifest["first_seq"], "last_seq": manifest["last_seq"],
        "gaps": gaps,
        "scope": "manifest committed prefix only; export integrity is not a replay verdict"
    }))
}

pub(super) fn append(
    out: &mut String,
    logs_dir: &Path,
    enabled: bool,
    cancelable: bool,
) -> Result<Pozycja> {
    let mut position = Pozycja::nowa("replay_capture", "nagranie odtwarzania live", enabled);
    naglowek(
        out,
        "10A. NAGRANIE ODTWARZANIA LIVE — PEŁNY ZAPIS WEJŚĆ I ODPOWIEDZI",
    );
    writeln!(out, "  Nagranie zawiera bootstrap i uporządkowane wywołania silnika/brokera.\n  Eksport obejmuje cały zachowany początek sesji do wskazanego last_seq.\n  Starszych luk ani zdarzeń po last_seq nie da się uzupełnić eksportem ticków.\n  COMMITTED_PREFIX oznacza komplet bajtów, nie potwierdzenie poprawności strategii.\n  Wiadomości, transakcje i tożsamość rachunku są prywatne.\n  Hasła, klucze API i klucze sesji uwierzytelniającej nie należą do kontraktu nagrania.")?;
    if !enabled {
        position.uwaga = "odznaczone w panelu — odtworzenie live niedostępne".into();
        writeln!(out, "  ({})", position.uwaga)?;
        return Ok(position);
    }
    let root = logs_dir.join("replay_capture");
    if !root.exists() {
        position.uwaga = "brak nagrania; starszy zrzut nie dowodzi odtworzenia 1:1".into();
        writeln!(out, "  ({})", position.uwaga)?;
        return Ok(position);
    }
    no_reparse_ancestors(&root)?;
    let mut incomplete = 0;
    let retention_path = root.join("retention.json");
    if retention_path.exists() {
        let retention = (|| -> Result<()> {
            let size = std::fs::metadata(&retention_path)?.len();
            let bytes = read_prefix(&retention_path, size, MANIFEST_LIMIT)?;
            let record: Value = serde_json::from_slice(&bytes)?;
            if record["schema"] != "conduit.live-capture.retention.v1" {
                bail!("unknown retention journal schema");
            }
            emit(out, "_retention", "retention.json", &bytes)?;
            position.rekordow += 1;
            position.bajtow += size;
            Ok(())
        })();
        if let Err(error) = retention {
            incomplete += 1;
            writeln!(
                out,
                "CONDUIT_REPLAY_EXPORT_STATUS_V1 {}",
                json!({
                    "run_id":"_retention", "status":"incomplete", "gaps":[error.to_string()]
                })
            )?;
        }
    }
    let mut runs = std::fs::read_dir(root)?
        .take(RUN_COUNT_LIMIT + 1)
        .collect::<std::io::Result<Vec<_>>>()?;
    if runs.len() > RUN_COUNT_LIMIT {
        bail!("capture directory count exceeds export limit; no complete export available");
    }
    runs.sort_by_key(|e| e.file_name());
    for run in runs {
        if cancelable && ANULUJ.load(Ordering::Relaxed) {
            bail!("scalanie anulowane przez użytkownika");
        }
        let kind = run.file_type()?;
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let run_id = run.file_name().to_string_lossy().into_owned();
        if run_id == "_retention" {
            incomplete += 1;
            writeln!(
                out,
                "CONDUIT_REPLAY_EXPORT_STATUS_V1 {}",
                json!({
                    "run_id":run_id,"status":"incomplete","gaps":["reserved run identifier"]
                })
            )?;
            continue;
        }
        if !safe_component(&run_id) {
            continue;
        }
        let status = match export_run(out, &run.path(), &run_id, &mut position, cancelable) {
            Ok(status) => status,
            Err(error) => {
                if cancelable && ANULUJ.load(Ordering::Relaxed) {
                    return Err(error);
                }
                json!({"run_id": run_id, "status": "incomplete", "gaps": [error.to_string()]})
            }
        };
        if status["status"] == "incomplete" {
            incomplete += 1;
        }
        writeln!(out, "CONDUIT_REPLAY_EXPORT_STATUS_V1 {status}")?;
    }
    position.uwaga = if incomplete > 0 {
        format!("niekompletne eksporty sesji: {incomplete}; sprawdź statusy odtwarzania")
    } else if position.rekordow == 0 {
        "brak zachowanych sesji odtwarzania".into()
    } else {
        "pełne opublikowane prefiksy; kwalifikacja osobno w manifestach i replay".into()
    };
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(tag: &str, content: &[u8], declared: &[u8]) -> (PathBuf, String) {
        let root = std::env::temp_dir().join(format!(
            "conduit-capture-export-{tag}-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let id = "test-run".to_string();
        let dir = root.join("replay_capture").join(&id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("segment-000001.jsonl"), content).unwrap();
        std::fs::write(dir.join("manifest.json"), serde_json::to_vec(&json!({
            "schema":"conduit.live-capture.manifest.v1", "run_id":id,
            "status":"open", "exact_replay_qualified":true,"gaps":[],
            "first_seq":1,"last_seq":1,"segments":[{"file":"segment-000001.jsonl",
            "bytes":declared.len(),"sha256":format!("{:x}",Sha256::digest(declared)),"sealed":false}]
        })).unwrap()).unwrap();
        (root, id)
    }
    use std::path::PathBuf;
    #[test]
    fn active_export_preserves_bytes_and_committed_prefix_only() {
        let prefix = "{\"text\":\"edycja — złoto\\nCANCEL\",\"seq\":1}\n".as_bytes();
        let mut actual = prefix.to_vec();
        actual.extend_from_slice(b"uncommitted partial");
        let (root, _) = fixture("prefix", &actual, prefix);
        let mut out = String::new();
        append(&mut out, &root, true, false).unwrap();
        let files: Vec<Value> = out
            .lines()
            .filter_map(|l| l.strip_prefix("CONDUIT_REPLAY_FILE_V1 "))
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1]["content"].as_str().unwrap().as_bytes(), prefix);
        assert!(!out.contains("uncommitted partial"));
        assert!(out.contains("committed_prefix_exported"));
    }
    #[test]
    fn corrupt_or_missing_segment_is_never_reported_complete() {
        let (root, id) = fixture("corrupt", b"wrong\n", b"right\n");
        let mut out = String::new();
        append(&mut out, &root, true, false).unwrap();
        assert!(out.contains("sha256 mismatch"));
        assert!(!out.contains("committed_prefix_exported"));
        std::fs::remove_file(
            root.join("replay_capture")
                .join(id)
                .join("segment-000001.jsonl"),
        )
        .unwrap();
        out.clear();
        append(&mut out, &root, true, false).unwrap();
        assert!(out.contains("\"status\":\"incomplete\""));
    }
    #[test]
    fn old_install_and_opt_out_are_explicit() {
        let (root, _) = fixture("disabled", b"PRIVATE-CONTENT", b"PRIVATE-CONTENT");
        let mut out = String::new();
        append(&mut out, &root, false, false).unwrap();
        assert!(!out.contains("PRIVATE-CONTENT"));
        assert!(out.contains("odznaczone w panelu"));
        out.clear();
        append(&mut out, &root.join("absent"), true, false).unwrap();
        assert!(out.contains("brak nagrania"));
    }
    #[test]
    fn filenames_cannot_escape_recording_directory() {
        for name in ["../secret", "C:\\secret", "..", "a/b"] {
            assert!(!safe_component(name));
        }
        for name in [
            "../secrets.json",
            "segment-1/../../secrets.jsonl",
            "manifest.json",
        ] {
            assert!(!segment_name(name));
        }
        assert!(segment_name("segment-000001.jsonl"));
    }
}
