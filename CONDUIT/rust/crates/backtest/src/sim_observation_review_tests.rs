//! Independent B15 physical-row review: account resets, ZZN and operations
//! between observations. These are broker-model tests, not strategy tuning.
use crate::{run, KonfOkien, RunConfig, SimBroker, TickData};
use crate::data::ReplayMessage;
use crate::runner::FormatCfg;
use conduit_core::{broker::*, settings::*, types::*};

const DAY: i64 = 20_000 * 86_400_000;

fn q(ts: i64, bid: f64, ask: f64) -> Quote { Quote { ts, bid, ask } }

struct Tape {
    ticks: Option<TickData>,
    path: std::path::PathBuf,
}
impl Tape {
    fn new(name: &str, quotes: &[Quote]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "conduit-observation-review-{name}-{}.cdtk", std::process::id()));
        let mut bytes = vec![0u8; 64];
        bytes[..4].copy_from_slice(&0x4B544443u32.to_le_bytes());
        bytes[8..16].copy_from_slice(&(quotes.len() as u64).to_le_bytes());
        for quote in quotes {
            bytes.extend(quote.ts.to_le_bytes());
            bytes.extend((quote.bid as f32).to_le_bytes());
            bytes.extend((quote.ask as f32).to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        let ticks = TickData::open(&path).unwrap();
        Self { ticks: Some(ticks), path }
    }
    fn ticks(&self) -> &TickData { self.ticks.as_ref().unwrap() }
}
impl Drop for Tape {
    fn drop(&mut self) {
        self.ticks.take(); // release the Windows mmap before removal
        let _ = std::fs::remove_file(&self.path);
    }
}

fn settings(v2: bool, strict: bool) -> Settings {
    Settings {
        auto_limit: true, entry_units: 1, lot_fixed: 0.01,
        pending_lifetime: PendingLifetime::Never,
        pending_drop_on_target: false,
        server_tz_offset_ms: 0, msg_clock_offset_ms: Some(0), exec_latency_ms: 0,
        live_tick_order_strict: strict, runner_ksiegowanie_v2: v2,
        session_filter: false, skip_if_sl_breached: false,
        rearm_grid_on_return: false, swap_enabled: false,
        max_dd_pct: 0.0, max_dd_usd: 0.0, equity_floor_pct: 0.0,
        ..Settings::default()
    }
}

fn formats(multi: bool, cfg: &Settings) -> Vec<FormatCfg> {
    if !multi { return vec![]; }
    vec![
        FormatCfg { format: "B15-review".into(), preset: "active".into(), settings: cfg.clone() },
        FormatCfg { format: "Other".into(), preset: "idle".into(), settings: cfg.clone() },
    ]
}

fn entry(ts: i64, msg_id: i64) -> ReplayMessage {
    ReplayMessage {
        telegram_published_ts: None,
        kanal: "B15-review".into(), ts, msg_id, reply_to: None, edit_of: None,
        text: "BUY LIMITS GOLD @ 4000/4000 AREA\nTP 4030\nTP 4060\nTP 4090\nSL 3995".into(),
    }
}

#[test]
fn physical_rows_preserve_forced_eod_and_new_day_entry_in_reset_and_flat_modes() {
    let tape = Tape::new("reset-flat", &[
        q(DAY-3000, 4010.0, 4010.2),
        q(DAY-2000, 3999.8, 4000.0), // old basket fills, SL untouched
        q(DAY-1000, 4001.0, 4001.2),
        q(DAY+1000, 4002.0, 4002.2), // explicit EOD close; then dispatch new message
        q(DAY+2000, 3999.8, 4000.0), // new day's pending fills
        q(DAY+3000, 3990.0, 3990.2), // its first subsequent SL quote
    ]);
    let messages = [entry(DAY-3000, 1), entry(DAY+500, 2)];
    for enabled in [false, true] { for v2 in [false, true] {
        for strict in [false, true] { for multi in [false, true] {
            for reset in [false, true] {
                let cfg = settings(v2, strict);
                let result = run(tape.ticks(), &messages, &RunConfig {
                    from: DAY-3000, to: DAY+4000, start_balance: 1000.0,
                    formaty: formats(multi, &cfg), settings: cfg,
                    sim_new_pending_sl_next_tick: enabled,
                    daily_reset: reset, flat_na_dobie: !reset,
                    ..Default::default()
                });
                let label = format!("B15={enabled} D4={v2} strict={strict} multi={multi} reset={reset}");
                assert!(result.sim_execution_reconciliation_required.is_none(), "{label}");
                assert_eq!(result.trades.len(), 2, "{label}");
                let old = &result.trades[0];
                assert_eq!(old.reason, CloseReason::EodFlat, "{label}");
                assert_eq!((old.open_ts, old.close_ts), (DAY-2000, DAY+1000), "{label}");
                assert_eq!(old.close_price, 4002.0, "{label}");
                let new = &result.trades[1];
                assert_eq!(new.reason, CloseReason::Sl, "{label}");
                assert_eq!((new.open_ts, new.close_ts), (DAY+2000, DAY+3000), "{label}");
                assert_eq!(new.close_price, 3990.0, "{label}");
                assert!((result.metrics.total_profit + 8.0).abs() < 1e-8, "{label}");
                assert_eq!(result.daily.len(), 2, "{label}");
                // The report uses the broker closing date for both records,
                // independently of D4/B15 execution and reset semantics.
                assert_eq!(result.daily[0].trades, 0, "{label}");
                assert_eq!(result.daily[1].trades, 2, "{label}");
            }
        }}
    }}
}

#[test]
fn physical_rows_cover_live_zzn_tail_and_forced_last_row_settlement() {
    for cut_tail in [false, true] {
        let mut quotes = vec![q(DAY-1000, 4010.0, 4010.2), q(DAY+1000, 3990.0, 3990.2)];
        if !cut_tail { quotes.push(q(DAY+2000, 3980.0, 3980.2)); }
        let tape = Tape::new(if cut_tail { "zzn-cut" } else { "zzn-sl" }, &quotes);
        let messages = [entry(DAY-1000, 10)];
        for enabled in [false, true] { for v2 in [false, true] {
            for strict in [false, true] { for multi in [false, true] {
                let cfg = settings(v2, strict);
                let result = crate::okna::uruchom(tape.ticks(), &messages, &KonfOkien {
                    from: DAY-1000, to: DAY, start_balance: 1000.0, n_dni: 1,
                    zzn: true, zzn_max_dni: 1,
                    formaty: formats(multi, &cfg), settings: cfg,
                    sim_new_pending_sl_next_tick: enabled, ..Default::default()
                });
                let label = format!("B15={enabled} D4={v2} strict={strict} multi={multi} cut={cut_tail}");
                assert!(result.sim_execution_reconciliation_required.is_none(), "{label}");
                assert_eq!(result.okna.len(), 1, "{label}");
                let window = &result.okna[0];
                assert_eq!(window.trejdy, 1, "new SL or forced EOD must settle once: {label}");
                let expected = if enabled && !cut_tail { -20.0 } else { -10.0 };
                assert!((window.zysk - expected).abs() < 1e-8,
                    "SL cannot activate on the repeated boundary row, but EOD is explicit: {label}; got {}", window.zysk);
                assert_eq!(result.tickow, quotes.len() as u64, "{label}");
                if enabled && cut_tail {
                    assert!(window.ogon_uciety, "the fresh position was explicitly truncated: {label}");
                }
            }}
        }}
    }
}

#[test]
fn physical_dedup_counts_margin_once_preserves_swap_and_does_not_block_explicit_exit() {
    for enabled in [false, true] {
        let mut broker = SimBroker::new(1000.0, 0.0, 0.0);
        broker.defer_new_pending_sl = enabled;
        broker.leverage = 5; // .01 lot @4000 uses800: positive margin level <150%
        broker.swap_enabled = true;
        broker.swap_long_points = -100.0;
        broker.swap_point_value = 1.0;
        broker.swap_rollover_mult = 1.0;
        broker.swap_pomijaj_weekend = false;
        let prior = q(DAY-1000, 4000.0, 4000.0);
        let current = q(DAY+1000, 4000.0, 4000.0);
        broker.on_tape_quote(prior, 10);
        let ticket = broker.open_market(OrderReq {
            side: Side::Buy, volume: 0.01, sl: None, tp: None,
            basket: Some(1), level: 0, is_toucher: false, comment: "review-owned".into(),
        }).unwrap();
        broker.mark(current); // D4 accounting path pays the same night first
        assert_eq!(broker.swap_total, -1.0);
        assert_eq!(broker.on_tape_quote(current, 11), (0, 0));
        for _ in 0..2 {
            broker.mark(current);
            assert_eq!(broker.on_tape_quote(current, 11), (0, 0));
        }
        let expected_samples = if enabled { 1 } else { 3 };
        assert_eq!(broker.ml_pod_200, expected_samples);
        assert_eq!(broker.ml_pod_150, expected_samples);
        assert_eq!(broker.ml_pod_100, 0);
        assert_eq!(broker.swap_total, -1.0, "same row cannot charge the night again");
        broker.close_position(ticket, CloseReason::EodFlat).unwrap();
        assert_eq!(broker.history.len(), 1);
        assert_eq!(broker.history[0].reason, CloseReason::EodFlat);
        assert!(broker.positions().is_empty());
        broker.place_pending(PendingReq {
            kind: PendingKind::BuyStop, volume: 0.01, price: 4000.0,
            sl: Some(3990.0), tp: None, basket: Some(2), level: 1,
            is_toucher: false, is_topup: false,no_market_fallback:false, comment: "after-EOD".into(),
        }).unwrap();
        assert_eq!(broker.market_instead_of_limit, 0, "zero stops makes this a real pending");
        assert_eq!(broker.on_tape_quote(current, 11), (if enabled { 0 } else { 1 }, 0));
        assert_eq!(broker.on_tape_quote(current, 12), (if enabled { 1 } else { 0 }, 0));
        assert_eq!(broker.positions().len(), 1);
        assert_eq!(broker.history.len(), 1, "an explicit close remains settled, never replayed");
        assert_eq!(broker.swap_total, -1.0);
    }
}
