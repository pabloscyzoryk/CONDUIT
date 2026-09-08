//! Includes the actual exporter unchanged. Only its display/inventory host is
//! stubbed here; all filesystem, manifest, hash and output-limit logic is production.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::{Path, PathBuf}, sync::atomic::{AtomicBool, AtomicU64, Ordering}};

static ANULUJ: AtomicBool = AtomicBool::new(false);
fn naglowek(out: &mut String, title: &str) { out.push_str(title); out.push('\n'); }
fn now_ms() -> i64 { conduit_server::now_ms() }
#[derive(Default)]
struct Pozycja { rekordow:u64, bajtow:u64, uwaga:String }
impl Pozycja { fn nowa(_: &'static str, _: &str, _:bool) -> Self { Self::default() } }
#[path = "../src/alllogs_capture.rs"]
mod actual_exporter;

fn fixture() -> (PathBuf, PathBuf) {
    static NEXT:AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!("conduit-export-guard-{}-{}-{}",
        std::process::id(),now_ms(),NEXT.fetch_add(1,Ordering::Relaxed)));
    let run = root.join("replay_capture").join("synthetic");
    std::fs::create_dir_all(&run).unwrap();
    let content = b"SYNTHETIC-DATA\n";
    std::fs::write(run.join("segment-000001.jsonl"), content).unwrap();
    std::fs::write(run.join("manifest.json"),serde_json::to_vec(&json!({
        "schema":"conduit.live-capture.manifest.v1","run_id":"synthetic","status":"open",
        "exact_replay_qualified":true,"gaps":[],"first_seq":1,"last_seq":1,
        "segments":[{"file":"segment-000001.jsonl","bytes":content.len(),
            "sha256":format!("{:x}",Sha256::digest(content)),"sealed":false}]
    })).unwrap()).unwrap();
    (root,run)
}
fn export(root:&Path) -> anyhow::Result<String> {
    let mut out=String::new(); actual_exporter::append(&mut out,root,true,false)?; Ok(out)
}

#[test]
fn canonical_windows_path_is_supported_without_weakening_reparse_guard() {
    let (root,_)=fixture();
    let canonical=root.canonicalize().unwrap();
    let out=export(&canonical).expect("ordinary canonical path must export without prefix metadata error");
    assert!(out.contains("committed_prefix_exported"));
}

#[test]
fn retention_frame_is_preserved_and_invalid_retention_marks_inventory_incomplete() {
    let (root,_)=fixture(); let path=root.join("replay_capture/retention.json");
    let content=serde_json::to_vec(&json!({"schema":"conduit.live-capture.retention.v1",
        "total_deleted_count":1,"deleted_runs":[{"run_id":"previous-synthetic"}]})).unwrap();
    std::fs::write(&path,&content).unwrap(); let out=export(&root).unwrap();
    let frame:Value=out.lines().filter_map(|l|l.strip_prefix("CONDUIT_REPLAY_FILE_V1 "))
        .map(|v|serde_json::from_str::<Value>(v).unwrap()).find(|v|v["run_id"]=="_retention").unwrap();
    assert_eq!(frame["content"].as_str().unwrap().as_bytes(),content);
    std::fs::write(&path,b"not valid json").unwrap(); let out=export(&root).unwrap();
    assert!(out.contains("\"run_id\":\"_retention\""));
    assert!(out.contains("\"status\":\"incomplete\""));
}

#[test]
fn reserved_retention_directory_cannot_impersonate_a_run() {
    let (root,_)=fixture(); std::fs::create_dir(root.join("replay_capture/_retention")).unwrap();
    let out=export(&root).unwrap();
    assert!(out.contains("reserved run identifier"));
    assert!(out.contains("\"status\":\"incomplete\""));
}

#[test]
fn oversized_manifest_and_declared_segment_are_rejected_before_payload_allocation() {
    let (root,run)=fixture(); let path=run.join("manifest.json");
    let mut manifest:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["segments"][0]["bytes"]=json!(64*1024*1024+1);
    std::fs::write(&path,serde_json::to_vec(&manifest).unwrap()).unwrap();
    let out=export(&root).unwrap(); assert!(out.contains("file exceeds capture export limit"));
    assert!(!out.contains("SYNTHETIC-DATA"));
    let file=std::fs::File::options().write(true).open(&path).unwrap();
    file.set_len(2*1024*1024+1).unwrap(); drop(file);
    let out=export(&root).unwrap(); assert!(out.contains("file exceeds capture export limit"));
    assert!(!out.contains("committed_prefix_exported"));
}

#[cfg(windows)]
#[test]
fn root_junction_is_rejected_before_reading_outside_content() {
    use std::os::windows::process::CommandExt;
    let (target,_)=fixture(); let link_parent=target.with_extension("link-host");
    std::fs::create_dir(&link_parent).unwrap(); let link=link_parent.join("replay_capture");
    let status=std::process::Command::new("powershell.exe")
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW: headless fixture only.
        .args(["-NoProfile","-NonInteractive","-Command",
            "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:CONDUIT_TEST_JUNCTION -Target $env:CONDUIT_TEST_TARGET | Out-Null"])
        .env("CONDUIT_TEST_JUNCTION",&link).env("CONDUIT_TEST_TARGET",target.join("replay_capture"))
        .status().unwrap(); assert!(status.success(),"synthetic junction fixture creation");
    let result=export(&link_parent);
    // Remove only this link, never recursively traverse the synthetic target.
    std::fs::remove_dir(&link).unwrap();
    assert!(result.is_err(),"a junction must not export its target's content");
    assert!(target.join("replay_capture/synthetic/manifest.json").is_file());
}
