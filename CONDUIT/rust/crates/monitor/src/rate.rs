//! Estimate throughput from completed work, not from UI polling frequency.
use std::time::Instant;

pub(crate) struct ProgressRate {
    advanced_at: Instant,
    completed: f64,
    speed: f64,
    interval_s: f64,
}

impl ProgressRate {
    pub(crate) fn new(now: Instant) -> Self {
        Self { advanced_at: now, completed: 0.0, speed: 0.0, interval_s: 0.0 }
    }

    pub(crate) fn update(&mut self, now: Instant, completed: f64, total: f64) -> (f64, f64) {
        if !completed.is_finite() || completed < 0.0 || !total.is_finite() || total <= 0.0 {
            return (0.0, -1.0);
        }
        if completed < self.completed {
            // A restarted counter is a new measurement, not negative throughput.
            *self = Self::new(now);
            self.completed = completed;
            return (0.0, -1.0);
        }
        let elapsed = now.saturating_duration_since(self.advanced_at).as_secs_f64();
        if completed > self.completed && elapsed > 0.01 {
            let measured = (completed - self.completed) / elapsed;
            // Sparse atomic counters can advance once every several seconds.
            // Repeated unchanged UI samples must not decay the rate to zero.
            let alpha = (elapsed / (elapsed + 15.0)).clamp(0.05, 0.8);
            self.speed = if self.speed <= 0.0 { measured }
                else { alpha * measured + (1.0 - alpha) * self.speed };
            self.completed = completed;
            self.advanced_at = now;
            self.interval_s = elapsed;
        }
        if completed >= total {
            return (self.speed, 0.0);
        }
        let idle = now.saturating_duration_since(self.advanced_at).as_secs_f64();
        let stale_after = (3.0 * self.interval_s).clamp(30.0, 120.0);
        if self.speed <= 0.0 || idle > stale_after {
            // Work may be blocked or its next chunk may be expensive. An
            // unknown ETA is more useful than a fictitious multi-day finish.
            return (0.0, -1.0);
        }
        (self.speed, ((total - completed) / self.speed).min(30.0 * 86_400.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sparse_chunks_do_not_turn_into_multi_day_estimates() {
        let start = Instant::now();
        let mut rate = ProgressRate::new(start);
        let first = rate.update(start + Duration::from_secs(10), 1000.0, 10_000.0);
        assert_eq!(first, (100.0, 90.0));
        for quarter in 41..80 {
            assert_eq!(rate.update(start + Duration::from_millis(quarter * 250), 1000.0, 10_000.0), first);
        }
        assert_eq!(rate.update(start + Duration::from_secs(20), 2000.0, 10_000.0), (100.0, 80.0));
    }

    #[test]
    fn actual_stall_is_unknown_and_a_new_chunk_recovers() {
        let start = Instant::now();
        let mut rate = ProgressRate::new(start);
        rate.update(start + Duration::from_secs(10), 1000.0, 10_000.0);
        assert_eq!(rate.update(start + Duration::from_secs(41), 1000.0, 10_000.0), (0.0, -1.0));
        let (speed, eta) = rate.update(start + Duration::from_secs(50), 2000.0, 10_000.0);
        assert!(speed > 25.0 && speed < 100.0);
        assert!(eta > 80.0 && eta < 320.0);
    }

    #[test]
    fn polling_frequency_does_not_change_the_estimate() {
        let start = Instant::now();
        let mut sparse = ProgressRate::new(start);
        let mut frequent = ProgressRate::new(start);
        for second in 1..=30 {
            let done = f64::from(second / 10) * 1000.0;
            let got = frequent.update(start + Duration::from_secs(second as u64), done, 10_000.0);
            if second % 10 == 0 {
                assert_eq!(got, sparse.update(start + Duration::from_secs(second as u64), done, 10_000.0));
            }
        }
    }

    #[test]
    fn completion_reset_and_invalid_input_are_explicit() {
        let start = Instant::now();
        let mut rate = ProgressRate::new(start);
        assert_eq!(rate.update(start, 0.0, 1000.0), (0.0, -1.0));
        assert_eq!(rate.update(start + Duration::from_secs(10), 1000.0, 1000.0).1, 0.0);
        assert_eq!(rate.update(start + Duration::from_secs(11), 10.0, 1000.0), (0.0, -1.0));
        assert_eq!(rate.update(start + Duration::from_secs(12), f64::NAN, 1000.0), (0.0, -1.0));
    }
}
