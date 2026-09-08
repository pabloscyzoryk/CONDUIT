//! Operational quote liveness, independent of signal time and strategy state.
use std::time::{Duration, Instant};
use conduit_core::types::Quote;

fn tick_key(q: Quote) -> (i64, u64, u64) { (q.ts, q.bid.to_bits(), q.ask.to_bits()) }

pub(crate) const ALERT_AFTER: Duration = Duration::from_secs(5 * 60);
pub(crate) const FIRST_RECOVERY_AFTER: Duration = Duration::from_secs(12 * 60);
pub(crate) const RECONNECT_PREFIX: &str = "Brak kwotowań ";

#[derive(Default)]
pub(crate) struct QuoteSilence {
    scope: String,
    last_tick: Option<(i64, u64, u64)>,
    last_progress: Option<Instant>,
    last_recovery: Option<Instant>,
    recoveries: u32,
    alerted: bool,
    last_health_check: Option<Instant>,
}

impl QuoteSilence {
    pub fn bind(&mut self, scope: String, tick: Quote, now: Instant) {
        if self.scope != scope || self.last_progress.is_none() {
            *self = Self { scope, last_tick: Some(tick_key(tick)), last_progress: Some(now), ..Self::default() };
        } else {
            self.observe_tick(tick, now);
        }
    }

    /// Reconnecting to exactly the same cached quote is not progress.
    pub fn observe_tick(&mut self, tick: Quote, now: Instant) -> bool {
        if tick.ts <= 0 || !tick.bid.is_finite() || !tick.ask.is_finite() || tick.bid <= 0.0 || tick.ask <= 0.0 { return false; }
        let key = tick_key(tick);
        if self.last_tick == Some(key) { return false; }
        let announce_return = self.alerted;
        self.last_tick = Some(key);
        self.last_progress = Some(now);
        self.last_recovery = None;
        self.recoveries = 0;
        self.alerted = false;
        self.last_health_check = None;
        announce_return
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.last_progress.map(|at| now.saturating_duration_since(at)).unwrap_or_default()
    }

    pub fn needs_health_check(&mut self, now: Instant) -> bool {
        if self.elapsed(now) < ALERT_AFTER { return false; }
        if self.last_health_check.is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(60)) { return false; }
        self.last_health_check = Some(now);
        true
    }

    pub fn alert_due(&mut self, now: Instant, expected_session: bool) -> bool {
        if !expected_session || self.alerted || self.elapsed(now) < ALERT_AFTER { return false; }
        self.alerted = true;
        true
    }

    pub fn recovery_due(&mut self, now: Instant, expected_session: bool) -> bool {
        if !expected_session { return false; }
        let interval = match self.recoveries { 0 => FIRST_RECOVERY_AFTER, 1 => Duration::from_secs(24 * 60), 2 => Duration::from_secs(48 * 60), _ => Duration::from_secs(60 * 60) };
        let since = self.last_recovery.or(self.last_progress).unwrap_or(now);
        if now.saturating_duration_since(since) < interval { return false; }
        self.last_recovery = Some(now);
        self.recoveries = self.recoveries.saturating_add(1);
        true
    }
}

pub(crate) fn is_quote_recovery(reason: &str) -> bool {
    reason.starts_with(RECONNECT_PREFIX) && reason.ends_with("— odbudowuję połączenie z terminalem.")
}

pub(crate) fn health_recovery_reason(reply: Result<&serde_json::Value, &str>) -> Option<String> {
    match reply {
        Ok(value) if value.get("terminal_connected").and_then(serde_json::Value::as_bool) == Some(false) =>
            Some("Terminal potwierdził brak połączenia z brokerem — odbudowuję połączenie.".into()),
        Err(error) => Some(format!("Kontrola połączenia sidecara nie powiodła się: {error}")),
        _ => None, // Healthy or legacy unknown is not evidence of transport loss or market hours.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn q(ts: i64) -> Quote { Quote::new(ts, 4000.0, 4000.2).unwrap() }
    fn at(start: Instant, minutes: u64) -> Instant { start + Duration::from_secs(minutes * 60) }

    #[test]
    fn same_quote_reconnect_preserves_one_alert_and_12_24_48_60_backoff() {
        let t = Instant::now(); let mut s = QuoteSilence::default();
        s.bind("A/gold".into(), q(100), t);
        assert!(!s.alert_due(at(t, 4), true));
        assert!(s.alert_due(at(t, 5), true));
        assert!(!s.recovery_due(at(t, 11), true));
        for minute in [12, 36, 84, 144, 204] {
            assert!(s.recovery_due(at(t, minute), true));
            s.bind("A/gold".into(), q(100), at(t, minute));
            assert!(!s.observe_tick(q(100), at(t, minute)));
            assert!(!s.alert_due(at(t, minute), true));
            assert!(!s.recovery_due(at(t, minute + 1), true));
        }
        assert_eq!(s.elapsed(at(t, 205)), Duration::from_secs(205 * 60));
    }

    #[test]
    fn fresh_quote_and_account_or_symbol_change_reset_the_episode() {
        let t = Instant::now(); let mut s = QuoteSilence::default();
        s.bind("A/gold".into(), q(100), t);
        assert!(s.alert_due(at(t, 5), true));
        assert!(s.recovery_due(at(t, 12), true));
        assert!(s.observe_tick(q(101), at(t, 13)));
        assert_eq!(s.elapsed(at(t, 13)), Duration::ZERO);
        assert!(!s.recovery_due(at(t, 24), true));
        assert!(s.recovery_due(at(t, 25), true));
        s.bind("B/gold.s".into(), q(90), at(t, 26));
        assert!(!s.recovery_due(at(t, 37), true));
        assert!(s.recovery_due(at(t, 38), true));
    }

    #[test]
    fn scheduled_break_suppresses_quote_restarts_but_not_health_checks() {
        let t = Instant::now(); let mut s = QuoteSilence::default();
        s.bind("A".into(), q(100), t);
        assert!(!s.alert_due(at(t, 60), false));
        assert!(!s.recovery_due(at(t, 60), false));
        assert!(s.needs_health_check(at(t, 60)));
        assert!(!s.needs_health_check(at(t, 60) + Duration::from_secs(59)));
        assert!(s.needs_health_check(at(t, 61)));
        assert!(s.recovery_due(at(t, 62), true));
    }

    #[test]
    fn quote_silence_notification_is_distinct_from_real_transport_failure() {
        assert!(is_quote_recovery("Brak kwotowań XAUUSD od 12 min — odbudowuję połączenie z terminalem."));
        assert!(!is_quote_recovery("Połączenie z sidecarem zerwane."));
        assert!(!is_quote_recovery("Follow terminal: konto wymaga wznowienia"));
    }

    #[test]
    fn health_disconnect_and_transport_error_recover_but_healthy_and_legacy_unknown_do_not() {
        use serde_json::json;
        assert!(health_recovery_reason(Ok(&json!({"terminal_connected": false}))).is_some());
        assert!(health_recovery_reason(Err("synthetic transport disconnected")).is_some());
        for value in [json!({"terminal_connected":true}), json!({"pong":true}),
            json!({"terminal_connected":null}), json!({"terminal_connected":"false"})] {
            assert!(health_recovery_reason(Ok(&value)).is_none());
        }
    }

    #[test]
    fn changed_quote_is_progress_even_when_broker_clock_moves_back_or_price_changes_at_same_ms() {
        let t = Instant::now(); let mut s = QuoteSilence::default();
        s.bind("A".into(), q(3_600_100), t);
        assert!(s.alert_due(at(t, 5), true));
        assert!(s.observe_tick(q(100), at(t, 6)), "broker clock rollback is not quote silence");
        assert_eq!(s.elapsed(at(t, 6)), Duration::ZERO);
        assert!(!s.observe_tick(q(100), at(t, 7)), "same cached quote is not fresh");
        let changed_price = Quote::new(100, 4000.1, 4000.3).unwrap();
        s.observe_tick(changed_price, at(t, 8));
        assert_eq!(s.elapsed(at(t, 8)), Duration::ZERO);
        s.bind("A".into(), changed_price, at(t, 9));
        assert_eq!(s.elapsed(at(t, 9)), Duration::from_secs(60));
        s.bind("A".into(), q(101), at(t, 10));
        assert_eq!(s.elapsed(at(t, 10)), Duration::ZERO, "new initial quote on reconnect is progress");
    }
}
