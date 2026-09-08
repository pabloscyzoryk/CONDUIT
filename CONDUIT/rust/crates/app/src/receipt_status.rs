//! Presentation of the existing receipt barrier; never changes trading authority.
use conduit_core::broker::ReceiptBarrier;
use std::time::Duration;

pub(crate) const PENDING_WARNING_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq)]
pub(crate) struct Notice {
    pub level: &'static str,
    pub title: &'static str,
    pub body: String,
}

#[derive(Default)]
pub(crate) struct ReceiptStatus {
    pending_since: Option<Duration>,
    announced: bool,
    review_issue: Option<String>,
}

impl ReceiptStatus {
    /// `elapsed` is monotonic time since this live session started, never broker time.
    pub fn observe(&mut self, barrier: ReceiptBarrier, issue: Option<&str>, elapsed: Duration) -> Option<Notice> {
        match barrier {
            ReceiptBarrier::Clear => {
                let announced = self.announced;
                *self = Self::default();
                announced.then(|| Notice {
                    level: "info",
                    title: "Potwierdzenia zamknięć uzgodnione",
                    body: "Odczyt stanu brokera zakończony; tymczasowa bramka wejść zdjęta.".into(),
                })
            }
            ReceiptBarrier::RequiresReview => {
                let issue = issue.unwrap_or("Potwierdzenia wymagają sprawdzenia; nowe wejścia pozostają zablokowane.");
                self.pending_since = None;
                if self.review_issue.as_deref() == Some(issue) { return None; }
                self.review_issue = Some(issue.into());
                self.announced = true;
                Some(Notice {
                    level: "error",
                    title: "Niepełne potwierdzenie zamknięcia — blokada nowych wejść",
                    body: issue.into(),
                })
            }
            ReceiptBarrier::Temporary => {
                let since = *self.pending_since.get_or_insert(elapsed);
                // A changed intermediate reason is still the same pending episode.
                // A previously published review remains visible until fully Clear.
                if self.announced || elapsed.saturating_sub(since) < PENDING_WARNING_AFTER { return None; }
                self.announced = true;
                Some(Notice {
                    level: "warn",
                    title: "Oczekiwanie na potwierdzenie wykonania",
                    body: issue.unwrap_or("Potwierdzenia są w trakcie uzgadniania; nowe wejścia czekają.").into(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sec(n: u64) -> Duration { Duration::from_secs(n) }

    #[test]
    fn normal_receipt_then_engine_drain_is_quiet() {
        let mut status = ReceiptStatus::default();
        assert!(status.observe(ReceiptBarrier::Temporary, Some("snapshot pending"), sec(0)).is_none());
        assert!(status.observe(ReceiptBarrier::Temporary, Some("closed buffer ready"), sec(1)).is_none());
        assert!(status.observe(ReceiptBarrier::Clear, None, sec(1)).is_none());
        assert!(status.observe(ReceiptBarrier::Temporary, Some("next receipt"), sec(20)).is_none());
    }

    #[test]
    fn long_wait_warns_once_without_resetting_on_intermediate_reason_changes() {
        let mut status = ReceiptStatus::default();
        assert!(status.observe(ReceiptBarrier::Temporary, Some("first"), sec(10)).is_none());
        assert!(status.observe(ReceiptBarrier::Temporary, Some("second"), sec(14)).is_none());
        let notice = status.observe(ReceiptBarrier::Temporary, Some("third"), sec(15)).unwrap();
        assert_eq!(notice.level, "warn");
        assert_eq!(notice.body, "third");
        assert!(status.observe(ReceiptBarrier::Temporary, Some("fourth"), sec(60)).is_none());
        assert_eq!(status.observe(ReceiptBarrier::Clear, None, sec(61)).unwrap().level, "info");
        assert!(status.observe(ReceiptBarrier::Clear, None, sec(62)).is_none());
    }

    #[test]
    fn genuine_review_is_immediate_and_never_downgraded_to_pending() {
        let mut status = ReceiptStatus::default();
        assert!(status.observe(ReceiptBarrier::Temporary, Some("pending"), sec(0)).is_none());
        assert_eq!(status.observe(ReceiptBarrier::RequiresReview, Some("conflicting receipt"), sec(0)).unwrap().level, "error");
        assert!(status.observe(ReceiptBarrier::RequiresReview, Some("conflicting receipt"), sec(1)).is_none());
        assert!(status.observe(ReceiptBarrier::Temporary, Some("pending"), sec(2)).is_none());
        assert_eq!(status.observe(ReceiptBarrier::RequiresReview, Some("scope changed"), sec(3)).unwrap().level, "error");
        assert_eq!(status.observe(ReceiptBarrier::Clear, None, sec(4)).unwrap().level, "info");
    }
}
