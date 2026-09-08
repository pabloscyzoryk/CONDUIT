//! Private, bounded allLogs export of an immutable terminal-history job.
//! This is retrospective broker evidence, never an input to the trading engine.
use super::{Pozycja, ANULUJ};
use crate::market::MarketSource;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const SCHEMA: &str = "conduit.broker_history.v1";
const HISTORY_LIMIT: usize = 128 * 1024 * 1024;
// Same aggregate ceiling as the replay section, which is appended later and
// counts this section too. Leave room for final diagnostics/source inventory.
const ALLLOGS_LIMIT: usize = 1024 * 1024 * 1024;
const METADATA_RESERVE: usize = 2 * 1024 * 1024;
const PAGE_LIMIT: usize = 256 * 1024;
const MAX_PAGES: u64 = 100_000;
const TIME_LIMIT: Duration = Duration::from_secs(120);

fn frame(out: &mut String, prefix: &str, value: &Value) {
    let _ = writeln!(out, "{prefix} {value}");
}

fn job_id(value: &Value) -> Option<&str> {
    value["job_id"].as_str().filter(|id| {
        !id.is_empty()
            && id.len() <= 96
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}

fn metadata_valid(value: &Value) -> bool {
    value["schema"] == SCHEMA
        && value.to_string().len() <= METADATA_RESERVE / 2
        && job_id(value).is_some()
        && value["source"].is_object()
        && value["requested_bounds"].is_object()
        && value["effective_bounds"]["from_ms"].as_i64() == Some(0)
        && value["effective_bounds"]["to_ms"]
            .as_i64()
            .is_some_and(|n| n > 0)
        && value["time_semantics"].is_object()
        && value["errors"].is_array()
}

fn coverage_complete(status: &Value) -> bool {
    let Some(end) = status["effective_bounds"]["to_ms"]
        .as_u64()
        .and_then(|n| n.checked_add(999))
        .map(|n| n / 1000 * 1000)
    else {
        return false;
    };
    let Some(ranges) = status["completed_ranges"].as_array() else {
        return false;
    };
    for kind in ["order", "deal"] {
        let mut next = 0;
        for range in ranges.iter().filter(|r| r["kind"] == kind) {
            let (Some(from), Some(to)) = (range["from_ms"].as_u64(), range["to_ms"].as_u64())
            else {
                return false;
            };
            if from != next || to <= from || to > end {
                return false;
            }
            next = to;
        }
        if next != end {
            return false;
        }
    }
    status["account_verified"] == true
        && status["source_after"] == status["source"]
        && ["position", "pending"].iter().all(|kind| {
            let snapshot = &status[format!("{kind}_snapshot")];
            snapshot["count"].as_u64().is_some()
                && snapshot["count"] == status["counts"][*kind]
                && snapshot["observed_start_utc_ms"].as_i64().is_some()
                && snapshot["observed_end_utc_ms"].as_i64().is_some()
                && snapshot["observed_end_utc_ms"].as_i64()
                    >= snapshot["observed_start_utc_ms"].as_i64()
        })
}

struct Limits {
    bytes: usize,
    duration: Duration,
    poll: Duration,
}

/// The injected RPC/clock wait keeps tests offline while exercising the real
/// exporter, including exact page bytes, limits, and release on every exit.
fn collect(
    out: &mut String,
    mut rpc: impl FnMut(Value) -> anyhow::Result<Value>,
    cancelled: impl Fn() -> bool,
    mut pause: impl FnMut(Duration),
    limits: Limits,
) -> Value {
    let started = Instant::now();
    let initial_len = out.len();
    let mut pages = 0u64;
    let mut counts: BTreeMap<String, u64> = ["order", "deal", "position", "pending"]
        .into_iter()
        .map(|s| (s.to_owned(), 0))
        .collect();
    let mut last = Value::Null;
    let mut initial = Value::Null;
    let mut id = None::<String>;
    let mut error = None::<&'static str>;
    let mut complete = false;
    let result = (|| -> Result<(), &'static str> {
        if cancelled() {
            return Err("cancelled");
        }
        if limits.duration.is_zero() {
            return Err("export_deadline");
        }
        initial = rpc(json!({"op":"start"})).map_err(|_| "history_start_unavailable")?;
        // Preserve an addressable job for cleanup even if its metadata is bad.
        id = job_id(&initial).map(str::to_owned);
        if !metadata_valid(&initial) {
            return Err("invalid_job_metadata");
        }
        frame(out, "CONDUIT_BROKER_HISTORY_BEGIN_V1", &initial);
        last = initial.clone();
        let mut prior_page_count = 0;
        loop {
            if cancelled() {
                return Err("cancelled");
            }
            if started.elapsed() >= limits.duration {
                return Err("export_deadline");
            }
            if !metadata_valid(&last)
                || last["job_id"] != initial["job_id"]
                || last["source"] != initial["source"]
                || last["requested_bounds"] != initial["requested_bounds"]
                || last["effective_bounds"] != initial["effective_bounds"]
                || last["time_semantics"] != initial["time_semantics"]
            {
                return Err("job_identity_or_bounds_changed");
            }
            let state = last["state"].as_str().ok_or("missing_job_state")?;
            if !matches!(state, "running" | "complete" | "partial" | "failed") {
                return Err("unknown_job_state");
            }
            let page_count = last["page_count"].as_u64().ok_or("missing_page_count")?;
            if page_count < prior_page_count || page_count > MAX_PAGES {
                return Err("invalid_page_count");
            }
            prior_page_count = page_count;
            while pages < page_count {
                if cancelled() {
                    return Err("cancelled");
                }
                if started.elapsed() >= limits.duration {
                    return Err("export_deadline");
                }
                let page = rpc(json!({"op":"page","job_id":id,"index":pages}))
                    .map_err(|_| "history_page_unavailable")?;
                if page["job_id"] != initial["job_id"] || page["index"].as_u64() != Some(pages) {
                    return Err("page_identity_or_order_mismatch");
                }
                let raw = page["records_json"].as_str().ok_or("missing_page_bytes")?;
                if raw.len() > PAGE_LIMIT {
                    return Err("page_byte_limit");
                }
                let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
                if page["sha256"].as_str() != Some(hash.as_str()) {
                    return Err("page_hash_mismatch");
                }
                let records: Vec<Value> =
                    serde_json::from_str(raw).map_err(|_| "page_decode_error")?;
                if records.len() > 256 || page["count"].as_u64() != Some(records.len() as u64) {
                    return Err("page_record_count_mismatch");
                }
                let mut page_counts = counts.clone();
                for record in &records {
                    let kind = record["kind"].as_str().ok_or("record_kind_missing")?;
                    if !record["raw"].is_object() || !record["observed_utc_ms"].is_i64() {
                        return Err("record_shape_invalid");
                    }
                    let count = page_counts.get_mut(kind).ok_or("record_kind_unknown")?;
                    *count = count.checked_add(1).ok_or("record_count_overflow")?;
                }
                // Preserve records_json byte-for-byte. Never route raw broker
                // tickets through browser JSON/Number or normalized UI DTOs.
                let line = format!("CONDUIT_BROKER_HISTORY_PAGE_V1 {page}\n");
                if out
                    .len()
                    .saturating_sub(initial_len)
                    .saturating_add(line.len())
                    .saturating_add(METADATA_RESERVE)
                    > limits.bytes
                {
                    return Err("export_byte_limit");
                }
                out.push_str(&line);
                counts = page_counts;
                pages += 1;
            }
            if state != "running" {
                let totals_match = counts
                    .iter()
                    .all(|(kind, n)| last["counts"][kind].as_u64() == Some(*n));
                complete = state == "complete"
                    && last["complete"].as_bool() == Some(true)
                    && last["partial"].as_bool() == Some(false)
                    && last["errors"].as_array().is_some_and(Vec::is_empty)
                    && totals_match
                    && coverage_complete(&last);
                if !complete {
                    return Err(if !totals_match {
                        "final_record_count_mismatch"
                    } else {
                        "terminal_history_incomplete"
                    });
                }
                return Ok(());
            }
            pause(limits.poll);
            last = rpc(json!({"op":"status","job_id":id}))
                .map_err(|_| "history_status_unavailable")?;
        }
    })();
    if let Err(reason) = result {
        error = Some(reason);
    }
    let released = id.as_ref().map(|id| {
        rpc(json!({"op":"release","job_id":id}))
            .is_ok_and(|v| v["released"] == true && v["job_id"] == *id)
    });
    json!({
        "schema":"conduit.broker_history.export.v1", "job_id":id,
        "status":if complete {"complete"} else if pages > 0 {"partial"} else {"unavailable"},
        "complete":complete, "partial":!complete, "export_error":error,
        "scope":"history_made_available_by_terminal_not_broker_archive_guarantee",
        "pages_exported":pages, "records_exported":counts,
        "bytes_before_status":out.len().saturating_sub(initial_len),
        "limit_bytes":limits.bytes,"limit_ms":limits.duration.as_millis(),
        "elapsed_ms":started.elapsed().as_millis(),"job_released":released,
        // Complete terminal metadata can qualify only this page set. Its own
        // complete=true must not mask an interrupted allLogs export.
        "terminal_status":if last.to_string().len() <= METADATA_RESERVE / 2 {last} else {Value::Null}
    })
}

pub(super) fn append(
    out: &mut String,
    source: Option<&dyn MarketSource>,
    enabled: bool,
    cancelable: bool,
) -> Pozycja {
    let mut position = Pozycja::nowa("broker_history", "historia rachunku z terminala", enabled);
    super::naglowek(
        out,
        "9B. HISTORIA RACHUNKU Z TERMINALA / TERMINAL ACCOUNT HISTORY",
    );
    if !enabled {
        position.uwaga = "odznaczone w panelu / disabled in panel".into();
        frame(
            out,
            "CONDUIT_BROKER_HISTORY_STATUS_V1",
            &json!({"status":"disabled","complete":false}),
        );
        return position;
    }
    let Some(source) = source else {
        position.uwaga = "brak źródła MT5 / MT5 source unavailable".into();
        frame(
            out,
            "CONDUIT_BROKER_HISTORY_STATUS_V1",
            &json!({"status":"unavailable","complete":false,"reason":"no_market_source"}),
        );
        return position;
    };
    let available = ALLLOGS_LIMIT
        .saturating_sub(out.len())
        .saturating_sub(2 * METADATA_RESERVE);
    let status = if available < METADATA_RESERVE {
        json!({"status":"unavailable","complete":false,"reason":"global_alllogs_byte_limit"})
    } else {
        collect(
            out,
            |request| source.broker_history(request),
            || cancelable && ANULUJ.load(Ordering::Relaxed),
            std::thread::sleep,
            Limits {
                bytes: HISTORY_LIMIT.min(available),
                duration: TIME_LIMIT,
                poll: Duration::from_millis(100),
            },
        )
    };
    position.rekordow = status["records_exported"]
        .as_object()
        .map(|c| c.values().filter_map(Value::as_u64).sum())
        .unwrap_or(0);
    position.bajtow = status["bytes_before_status"].as_u64().unwrap_or(0);
    position.uwaga = if status["complete"] == true {
        "pełna historia udostępniona przez terminal / complete history made available by the terminal".into()
    } else {
        format!(
            "NIEPEŁNE / INCOMPLETE: {}",
            status["export_error"]
                .as_str()
                .or(status["reason"].as_str())
                .unwrap_or("history_unavailable")
        )
    };
    // Raw broker timestamps are deliberately NOT passed to Pozycja::zakres:
    // that inventory formatter uses the host zone. Bounds/domain are in JSON.
    frame(out, "CONDUIT_BROKER_HISTORY_STATUS_V1", &status);
    position
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{CandlesDoc, CostsDoc, DealsDoc, SymbolDoc};
    fn status(state: &str, pages: u64, orders: u64, deals: u64) -> Value {
        json!({"schema":SCHEMA,"job_id":"synthetic_job","state":state,
            "complete":state=="complete","partial":state=="partial",
            "source":{"account":{"login":"42","server":"fixture"}},
            "requested_bounds":{"from_ms":0,"to_ms":1900000000000i64},
            "effective_bounds":{"from_ms":0,"to_ms":1900000000000i64},
            "time_semantics":{"basis":"mt5_raw_epoch_fields"},
            "errors":[],"counts":{"order":orders,"deal":deals,"position":0,"pending":0},"page_count":pages,
            "account_verified":true,"source_after":{"account":{"login":"42","server":"fixture"}},
            "position_snapshot":{"count":0,"observed_start_utc_ms":1,"observed_end_utc_ms":2},
            "pending_snapshot":{"count":0,"observed_start_utc_ms":1,"observed_end_utc_ms":2},
            "completed_ranges":[{"kind":"order","from_ms":0,"to_ms":1900000000000i64},
                {"kind":"deal","from_ms":0,"to_ms":1900000000000i64}]})
    }
    fn page(index: u64, kind: &str) -> Value {
        let raw = format!(
            r#"[{{"kind":"{kind}","observed_utc_ms":1900000000000,"raw":{{"ticket":"18446744073709551615","commission":-1.25,"swap":0.5,"fee":-0.2,"type":2,"entry":1,"comment":"synthetic"}}}}]"#
        );
        json!({"job_id":"synthetic_job","index":index,"count":1,
            "sha256":format!("{:x}",Sha256::digest(raw.as_bytes())),"records_json":raw})
    }
    fn run(responses: Vec<Value>, max_bytes: usize) -> (String, Value, Vec<Value>) {
        let mut iter = responses.into_iter();
        let mut out = String::new();
        let mut calls = Vec::new();
        let result = collect(
            &mut out,
            |q| {
                calls.push(q.clone());
                if q["op"] == "release" {
                    return Ok(json!({"released":true,"job_id":"synthetic_job"}));
                }
                iter.next()
                    .ok_or_else(|| anyhow::anyhow!("synthetic exhausted"))
            },
            || false,
            |_| {},
            Limits {
                bytes: max_bytes,
                duration: Duration::from_secs(2),
                poll: Duration::ZERO,
            },
        );
        (out, result, calls)
    }
    #[test]
    fn asynchronous_pages_preserve_raw_ids_costs_and_complete_counts() {
        let p = page(0, "order");
        let raw = p["records_json"].clone();
        let (out, result, calls) = run(
            vec![
                status("running", 0, 0, 0),
                status("running", 1, 1, 0),
                p,
                status("complete", 2, 1, 1),
                page(1, "deal"),
            ],
            HISTORY_LIMIT,
        );
        assert_eq!(result["complete"], true);
        assert_eq!(result["pages_exported"], 2);
        let saved: Value = serde_json::from_str(
            out.lines()
                .find_map(|l| l.strip_prefix("CONDUIT_BROKER_HISTORY_PAGE_V1 "))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["records_json"], raw);
        assert_eq!(calls.last().unwrap()["op"], "release");
        assert_eq!(result["records_exported"]["deal"], 1);
    }
    #[test]
    fn genuine_empty_history_is_complete_but_unknown_and_partial_are_not() {
        assert_eq!(
            run(vec![status("complete", 0, 0, 0)], HISTORY_LIMIT).1["complete"],
            true
        );
        for state in ["partial", "failed", "other"] {
            let (_, result, calls) = run(vec![status(state, 0, 0, 0)], HISTORY_LIMIT);
            assert_eq!(result["complete"], false);
            assert_eq!(calls.last().unwrap()["op"], "release");
        }
    }
    #[test]
    fn corrupt_duplicate_and_wrong_page_are_rejected_and_released() {
        for field in ["sha256", "index", "job_id", "count"] {
            let mut p = page(0, "deal");
            p[field] = json!("wrong");
            let (_, result, calls) = run(vec![status("complete", 1, 0, 1), p], HISTORY_LIMIT);
            assert_eq!(result["complete"], false, "{field}");
            assert_eq!(result["pages_exported"], 0);
            assert_eq!(calls.last().unwrap()["op"], "release");
        }
        let (_, r, _) = run(
            vec![
                status("complete", 2, 0, 2),
                page(0, "deal"),
                page(0, "deal"),
            ],
            HISTORY_LIMIT,
        );
        assert_eq!(r["status"], "partial");
        assert_eq!(r["pages_exported"], 1);
    }
    #[test]
    fn account_bounds_and_missing_pages_fail_closed() {
        for field in [
            "source",
            "requested_bounds",
            "time_semantics",
            "effective_bounds",
        ] {
            let mut changed = status("complete", 0, 0, 0);
            changed[field] = json!({"changed":true});
            assert_eq!(
                run(vec![status("running", 0, 0, 0), changed], HISTORY_LIMIT).1["export_error"],
                "job_identity_or_bounds_changed"
            );
        }
        assert_eq!(
            run(vec![status("complete", 1, 1, 0)], HISTORY_LIMIT).1["complete"],
            false
        );
        assert_eq!(
            run(vec![status("complete", 0, 1, 0)], HISTORY_LIMIT).1["export_error"],
            "final_record_count_mismatch"
        );
    }
    #[test]
    fn complete_flag_cannot_hide_unverified_account_or_interval_gap() {
        for field in [
            "account_verified",
            "source_after",
            "completed_ranges",
            "position_snapshot",
            "pending_snapshot",
        ] {
            let mut s = status("complete", 0, 0, 0);
            s[field] = Value::Null;
            assert_eq!(run(vec![s], HISTORY_LIMIT).1["complete"], false, "{field}");
        }
        let mut s = status("complete", 0, 0, 0);
        s["completed_ranges"][1]["from_ms"] = json!(1000);
        assert_eq!(run(vec![s], HISTORY_LIMIT).1["complete"], false);
    }
    #[test]
    fn byte_limit_cannot_publish_truncated_page_as_complete() {
        let (out, result, calls) = run(
            vec![status("complete", 1, 0, 1), page(0, "deal")],
            METADATA_RESERVE,
        );
        assert_eq!(result["export_error"], "export_byte_limit");
        assert!(!out.contains("HISTORY_PAGE_V1"));
        assert_eq!(calls.last().unwrap()["op"], "release");
    }
    #[test]
    fn cancellation_and_deadline_do_not_start_an_unbounded_job() {
        for cancel in [true, false] {
            let mut out = String::new();
            let mut called = false;
            let r = collect(
                &mut out,
                |_| {
                    called = true;
                    Ok(Value::Null)
                },
                || cancel,
                |_| {},
                Limits {
                    bytes: HISTORY_LIMIT,
                    duration: Duration::ZERO,
                    poll: Duration::ZERO,
                },
            );
            assert!(!called);
            assert_eq!(r["complete"], false);
        }
    }
    #[test]
    fn cancellation_after_start_releases_job() {
        let cancelled = std::cell::Cell::new(false);
        let mut calls = Vec::new();
        let mut out = String::new();
        let r = collect(
            &mut out,
            |q| {
                calls.push(q.clone());
                cancelled.set(true);
                if q["op"] == "release" {
                    Ok(json!({"released":true,"job_id":"synthetic_job"}))
                } else {
                    Ok(status("running", 0, 0, 0))
                }
            },
            || cancelled.get(),
            |_| {},
            Limits {
                bytes: HISTORY_LIMIT,
                duration: Duration::from_secs(1),
                poll: Duration::ZERO,
            },
        );
        assert_eq!(r["export_error"], "cancelled");
        assert_eq!(calls.last().unwrap()["op"], "release");
    }

    #[test]
    fn actual_alllogs_uses_dedicated_terminal_job_not_capped_ui_history() {
        struct Source;
        impl MarketSource for Source {
            fn candles(
                &self,
                _: &str,
                _: &str,
                _: usize,
                _: Option<i64>,
            ) -> anyhow::Result<CandlesDoc> {
                panic!("unrelated market call")
            }
            fn symbol(&self, _: &str) -> anyhow::Result<SymbolDoc> {
                panic!("unrelated market call")
            }
            fn deals(
                &self,
                _: Option<i64>,
                _: Option<i64>,
                _: Option<&str>,
                _: Option<i64>,
                _: bool,
                _: usize,
                _: usize,
            ) -> anyhow::Result<DealsDoc> {
                panic!("UI history API must not replace raw history")
            }
            fn costs(&self, _: &str, _: f64) -> anyhow::Result<CostsDoc> {
                panic!("unrelated market call")
            }
            fn default_symbol(&self) -> String {
                "SYNTHETIC".into()
            }
            fn broker_history(&self, q: Value) -> anyhow::Result<Value> {
                match q["op"].as_str().unwrap() {
                    "start" => Ok(status("complete", 1, 0, 40)),
                    "page" => {
                        let records:Vec<Value>=(0..40).map(|n|json!({"kind":"deal","observed_utc_ms":1900000000000i64,
                            "raw":{"ticket":(u64::MAX-n).to_string(),"type":2,"symbol":"","profit":-1.25,"commission":-0.05,"fee":-0.02,"swap":0.1}})).collect();
                        let raw = serde_json::to_string(&records).unwrap();
                        Ok(json!({"job_id":"synthetic_job","index":0,"count":40,
                            "sha256":format!("{:x}",Sha256::digest(raw.as_bytes())),"records_json":raw}))
                    }
                    "release" => Ok(json!({"released":true,"job_id":"synthetic_job"})),
                    _ => panic!("unexpected polling for completed immutable job"),
                }
            }
        }
        let root = std::env::temp_dir().join(format!(
            "conduit-alllogs-history-{}-{}",
            std::process::id(),
            crate::now_ms()
        ));
        let st = crate::bootstrap(
            &crate::ServerConfig {
                workspace: root.clone(),
                ..Default::default()
            },
            crate::default_auth(),
        )
        .unwrap();
        st.set_market(std::sync::Arc::new(Source));
        let text = crate::alllogs::zbuduj(&st);
        let final_status: Value = serde_json::from_str(
            text.lines()
                .find_map(|line| line.strip_prefix("CONDUIT_BROKER_HISTORY_STATUS_V1 "))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(final_status["complete"], true);
        assert_eq!(final_status["records_exported"]["deal"], 40);
        let page: Value = serde_json::from_str(
            text.lines()
                .find_map(|line| line.strip_prefix("CONDUIT_BROKER_HISTORY_PAGE_V1 "))
                .unwrap(),
        )
        .unwrap();
        let records: Vec<Value> =
            serde_json::from_str(page["records_json"].as_str().unwrap()).unwrap();
        assert_eq!(records.len(), 40);
        assert_eq!(records[0]["raw"]["ticket"], u64::MAX.to_string());
        assert_eq!(records[0]["raw"]["symbol"], ""); // Account balance/credit events are retained.
        assert_eq!(records[0]["raw"]["fee"], -0.02);
        std::fs::remove_dir_all(root).unwrap();
    }
}
