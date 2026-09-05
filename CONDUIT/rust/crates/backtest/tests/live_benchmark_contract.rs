//! Deterministic live-ingress benchmark, with synthetic data only.
//! These are execution equivalence checks, not historical profit evidence.
use conduit_backtest::data::{ReplayMessage, TickData};
use conduit_backtest::runner::{run, RunConfig, RunResult};
use conduit_core::settings::{PendingLifetime, Settings};
use conduit_core::types::Ts;
use std::sync::atomic::{AtomicU64, Ordering};

const T0: Ts = 1_800_000_000_000;
const ENTRY: &str = "BUY LIMITS GOLD @ 4000/3995\nTP 4010\nTP 4020\nTP 4030\nSL 3990";
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct Tape {
    data: Option<TickData>,
    path: std::path::PathBuf,
}
impl Tape {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "conduit_live_benchmark_{}_{}.bin",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        // Message phases happen while price is fixed. Only later physical
        // observations touch the entries and protective stops/targets.
        let prices = [
            4008., 4008., 4008., 4008., 4008., 4008., 4008., 4008., 4008., 4008., 3999., 3997.,
            3994., 4003., 4011., 4021., 4031., 3990.,
        ];
        let mut bytes = vec![0u8; 64];
        bytes[..4].copy_from_slice(&0x4B54_4443u32.to_le_bytes());
        bytes[8..16].copy_from_slice(&(prices.len() as u64).to_le_bytes());
        for (index, price) in prices.iter().enumerate() {
            bytes.extend_from_slice(&(T0 + index as i64 * 1000).to_le_bytes());
            bytes.extend_from_slice(&(*price as f32).to_le_bytes());
            bytes.extend_from_slice(&((*price + 0.2) as f32).to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        let mut data = TickData::open(&path).unwrap();
        data.set_price_digits(Some(2)).unwrap();
        Self {
            data: Some(data),
            path,
        }
    }
}
impl Drop for Tape {
    fn drop(&mut self) {
        self.data.take(); // Release the Windows mmap before deleting this file.
        let _ = std::fs::remove_file(&self.path);
    }
}

fn message(second: i64, id: i64, text: &str, edit: bool, parent: Option<i64>) -> ReplayMessage {
    ReplayMessage {
        ts: T0 + second * 1000,
        telegram_published_ts: Some(T0),
        msg_id: id,
        reply_to: parent,
        edit_of: edit.then_some(id),
        text: text.into(),
        kanal: "Synergy".into(),
    }
}
fn settings(cap: f64) -> Settings {
    let mut c = Settings::default();
    c.msg_clock_offset_ms = Some(0);
    c.server_tz_offset_ms = 0;
    c.exec_latency_ms = 0;
    c.session_filter = false;
    c.ignore_old_after_min = 0.;
    c.entry_units = 3;
    c.auto_limit = false;
    c.lot_mode_percent = false;
    c.lot_fixed = 0.1;
    c.lot_min = 0.01;
    c.lot_max = cap;
    c.risk_per_basket_pct = 0.;
    c.max_portfolio_risk_pct = 0.;
    c.max_open_positions = 0;
    c.max_open_baskets = 0;
    c.pending_lifetime = PendingLifetime::Never;
    c.pending_drop_on_target = false;
    c.explicit_pending_until_cancel = false;
    c.edycja_sieroty_nie_otwiera = false;
    c.entry_idempotencja = false;
    c.dedup_edited_signals = true;
    c.dedup_management_po_restarcie = true;
    c.reply_veto = true;
    c.reply_graph_transitive = true;
    c.honor_cancel = true;
    c.journal_enabled = false;
    c.swap_enabled = false;
    c
}
fn replay(tape: &Tape, messages: &[ReplayMessage], cap: f64, ingress: bool) -> RunResult {
    replay_with_settings(tape, messages, settings(cap), ingress)
}
fn replay_with_settings(
    tape: &Tape,
    messages: &[ReplayMessage],
    configuration: Settings,
    ingress: bool,
) -> RunResult {
    run(
        tape.data.as_ref().unwrap(),
        messages,
        &RunConfig {
            from: T0,
            to: T0 + 18_000,
            start_balance: 600.,
            settings: configuration,
            source_name: "Synergy".into(),
            live_telegram_ingress: ingress,
            sim_new_pending_sl_next_tick: true,
            sim_limit_price_improvement: true,
            curve_interval_ms: 1000,
            ..Default::default()
        },
    )
}

#[test]
fn first_observed_edit_of_a_two_week_old_source_uses_current_observation_time() {
    let tape = Tape::new();
    for cap in [0.01, 0.1, 1., 10.] {
        let mut configuration = settings(cap);
        configuration.ignore_old_after_min = 5.;
        let fresh = message(1, 100, ENTRY, true, None);
        let mut old_publication = fresh.clone();
        old_publication.telegram_published_ts = Some(T0 - 14 * 86_400_000);
        let expected = replay_with_settings(&tape, &[fresh], configuration.clone(), true);
        let actual = replay_with_settings(
            &tape,
            &[old_publication.clone()],
            configuration.clone(),
            true,
        );
        assert!(!expected.trades.is_empty());
        assert_economics_equal(
            &expected,
            &actual,
            "old publication, current EDIT observation",
        );
        assert!(actual.trades.iter().all(|trade| trade.open_ts >= T0 + 1000));
        // Prove that freshness checking is active, rather than obtaining this
        // equivalence by turning off the ordinary stale-NEW gate.
        old_publication.edit_of = None;
        let stale_new = replay_with_settings(&tape, &[old_publication], configuration, true);
        assert!(stale_new.trades.is_empty());
    }
}
fn assert_economics_equal(a: &RunResult, b: &RunResult, context: &str) {
    assert_eq!(
        serde_json::to_value(&a.trades).unwrap(),
        serde_json::to_value(&b.trades).unwrap(),
        "transaction decisions differ: {context}"
    );
    assert_eq!(a.equity_curve, b.equity_curve, "equity path: {context}");
    assert_eq!(a.balance_curve, b.balance_curve, "balance path: {context}");
    assert_eq!(
        a.metrics.end_equity, b.metrics.end_equity,
        "final equity: {context}"
    );
    assert_eq!(
        a.baskets_dump.len(),
        b.baskets_dump.len(),
        "basket count: {context}"
    );
}

#[test]
fn first_complete_edit_matches_new_and_survives_listener_restart_redelivery() {
    let tape = Tape::new();
    for cap in [0.01, 0.1, 1., 10.] {
        let clean = vec![message(1, 100, ENTRY, false, None)];
        let mut restart = message(
            4,
            -1,
            "__CONDUIT_LIVEBACKTEST_INGRESS_RESTART__",
            false,
            None,
        );
        restart.kanal = "__CONDUIT_CONTROL__".into();
        let perturbed = vec![
            message(1, 100, ENTRY, true, None),
            message(2, 100, ENTRY, true, None),
            message(3, 100, ENTRY, false, None),
            restart,
            message(5, 100, ENTRY, true, None),
            message(6, 100, ENTRY, false, None),
        ];
        let expected = replay(&tape, &clean, cap, true);
        assert!(
            !expected.trades.is_empty(),
            "fixture must execute real trades at cap {cap}"
        );
        let actual = replay(&tape, &perturbed, cap, true);
        assert_economics_equal(&expected, &actual, &format!("first EDIT / cap {cap}"));
        assert_eq!(actual.metrics.known_full_entry_sources, 1);
        assert_eq!(actual.metrics.entry_sources_first_seen_as_edit, 1);
    }
}

#[test]
fn complete_edit_policy_is_identical_with_and_without_live_content_dedup() {
    let tape = Tape::new();
    let messages = vec![
        message(1, 100, ENTRY, true, None),
        message(2, 100, ENTRY, true, None),
        message(3, 100, ENTRY, false, None),
        message(4, 100, ENTRY, true, None),
    ];
    for cap in [0.01, 0.1, 1., 10.] {
        let ordinary = replay(&tape, &messages, cap, false);
        let live = replay(&tape, &messages, cap, true);
        assert!(!ordinary.trades.is_empty());
        assert_economics_equal(&ordinary, &live, &format!("ingress ON/OFF / cap {cap}"));
    }
}

#[test]
fn source_bound_cancel_before_first_entry_prevents_late_adoption() {
    let tape = Tape::new();
    let messages = vec![
        message(1, 101, "CANCEL", false, Some(100)),
        message(2, 100, ENTRY, true, None),
        message(3, 100, ENTRY, false, None),
        message(4, 100, &ENTRY.replace("SL 3990", "SL 3991"), true, None),
    ];
    for ingress in [false, true] {
        let result = replay(&tape, &messages, 0.01, ingress);
        assert!(
            result.trades.is_empty(),
            "cancelled source must never open on later EDIT/NEW"
        );
        assert_eq!(result.metrics.end_equity, 600.);
    }
}

#[test]
fn late_new_cannot_restore_geometry_older_than_an_accepted_edit() {
    let tape = Tape::new();
    let changed = ENTRY.replace("SL 3990", "SL 3992");
    let clean = vec![
        message(1, 100, ENTRY, true, None),
        message(2, 100, &changed, true, None),
    ];
    let mut replayed = clean.clone();
    replayed.push(message(3, 100, ENTRY, false, None));
    replayed.push(message(4, 100, &changed, true, None));
    for ingress in [false, true] {
        let expected = replay(&tape, &clean, 0.01, ingress);
        let actual = replay(&tape, &replayed, 0.01, ingress);
        assert!(!expected.trades.is_empty());
        assert_economics_equal(&expected, &actual, "late NEW after accepted updated EDIT");
    }
}
