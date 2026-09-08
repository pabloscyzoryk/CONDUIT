//! Offline synthetic load. Uses the production capture module; never starts services.
#[path = "../src/replay_capture.rs"]
#[allow(dead_code)]
mod replay_capture;
use anyhow::{ensure, Context, Result};
use conduit_backtest::sim::SimBroker;
use conduit_core::{
    broker::Broker,
    engine::{Engine, IncomingMessage},
    types::{Quote, SourceKey},
    Settings,
};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn summary(mut values: Vec<f64>) -> serde_json::Value {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.sort_by(f64::total_cmp);
    json!({"mean_us":mean,"p95_us":values[values.len()*95/100]})
}
fn main() -> Result<()> {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("supply a new private output directory")?,
    );
    ensure!(!output.exists(), "output directory must not exist");
    std::fs::create_dir_all(&output)?;
    let session = replay_capture::Session::start(&output, "synthetic_busy")?;
    let mut cfg = Settings::default();
    cfg.ai_enabled = false;
    cfg.ea_enabled = false;
    cfg.lot_mode_percent = false;
    cfg.lot_fixed = 0.01;
    cfg.lot_max = 0.01;
    cfg.max_open_baskets = 0;
    let mut e = Engine::new(cfg.clone(), 10_000.0);
    let mut b = SimBroker::new(10_000.0, 0.0, 0.0);
    let mut base = Engine::new(cfg, 10_000.0);
    let mut bb = SimBroker::new(10_000.0, 0.0, 0.0);
    let start = 1782400000000i64;
    let q = Quote {
        ts: start,
        bid: 4000.0,
        ask: 4000.2,
    };
    b.on_quote(q);
    bb.on_quote(q);
    for id in 1..=40 {
        let message = IncomingMessage {
            ts: start,
            source: SourceKey::new(id, None),
            source_name: "Synthetic fixture".into(),
            msg_id: id,
            reply_to: None,
            edit_of: None,
            text: "GOLD BUY 3999-4001 SL 3990 TP1 4010 TP2 4020 TP3 4030".into(),
        };
        session.message(&mut e, &mut b, &message, start - 10_800_000);
        base.on_message_received(&mut bb, &message, start - 10_800_000);
        // Production live drains the decision journal at each outer loop.
        // Engine.logs and basket history remain intact, just as in live.rs.
        e.drain_journal();
        base.drain_journal();
    }
    ensure!(
        e.baskets.len() >= 30,
        "fixture did not create enough baskets"
    );
    replay_capture::benchmark::take();
    let mut captured = Vec::new();
    let mut plain = Vec::new();
    for n in 1..=1200 {
        let price = 3999.0 + (n % 31) as f64 * 0.08;
        let q = Quote {
            ts: start + n * 250,
            bid: price,
            ask: price + 0.2,
        };
        b.on_quote(q);
        bb.on_quote(q);
        let t = Instant::now();
        session.tick(&mut e, &mut b, &q, q.ts - 10_800_000);
        captured.push(t.elapsed().as_secs_f64() * 1e6);
        let t = Instant::now();
        base.on_tick_received(&mut bb, &q, q.ts - 10_800_000);
        plain.push(t.elapsed().as_secs_f64() * 1e6);
        e.drain_journal();
        base.drain_journal();
    }
    ensure!(
        b.account().balance.to_bits() == bb.account().balance.to_bits(),
        "cash changed"
    );
    ensure!(
        b.account().equity.to_bits() == bb.account().equity.to_bits(),
        "equity changed"
    );
    session.tape.finish();
    let deadline = Instant::now() + Duration::from_secs(30);
    let dir = output.join("synthetic_busy");
    while !conduit_server::replay_capture::read_manifest(&dir)?.writer_closed {
        ensure!(Instant::now() < deadline, "writer did not finalize");
        std::thread::sleep(Duration::from_millis(20));
    }
    ensure!(
        session.tape.take_warning().is_none(),
        "capture was incomplete"
    );
    let m = conduit_server::replay_capture::read_manifest(&dir)?;
    let bytes: u64 = m.segments.iter().map(|s| s.bytes).sum();
    let phases = replay_capture::benchmark::take();
    ensure!(
        phases.len() == 1200,
        "phase coverage differs from tick count"
    );
    let labels = [
        "pre_bootstrap_patch_begin",
        "recorded_engine_apply_trace",
        "post_bootstrap_and_hash",
        "pool_pack_append_and_memory",
    ];
    let mut phase_report = serde_json::Map::new();
    for (i, label) in labels.iter().enumerate() {
        phase_report.insert(
            (*label).into(),
            summary(phases.iter().map(|p| p[i]).collect()),
        );
    }
    let fields = e
        .export_replay_bootstrap()
        .map_err(anyhow::Error::msg)?
        .fields;
    let mut sizes = fields
        .into_iter()
        .map(|(k, v)| Ok((k, serde_json::to_vec(&v)?.len())))
        .collect::<Result<Vec<_>>>()?;
    sizes.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    let report = json!({"schema":"conduit.capture.synthetic-performance.v2","profile":if cfg!(debug_assertions){"debug"}else{"release"},"journal_drain":"each_outer_loop","engine_logs":"retained_production_bounded_policy","ticks":1200,"baskets":e.baskets.len(),"positions":b.positions().len(),"captured":summary(captured),"baseline":summary(plain),"phases":phase_report,"final_bootstrap_field_encoded_bytes":sizes,"bytes":bytes,"bytes_per_tick_including_bootstrap":bytes as f64/1200.0,"estimated_MiB_per_hour_at_10_ticks_s":bytes as f64/1200.0*36_000.0/1_048_576.0,"binary_sha256":replay_capture::binary_sha256()?,"source_sha256":env!("CONDUIT_REPLAY_SOURCE_SHA256"),"live_services_started":false,"capture_directory":dir});
    std::fs::write(
        output.join("performance.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    let verified = replay_capture::verify_directory(&dir, &output.join("replay.json"))?;
    std::fs::write(
        output.join("replay.json"),
        serde_json::to_vec_pretty(&verified)?,
    )?;
    ensure!(verified["status"] == "PASS", "offline replay failed");
    Ok(())
}
