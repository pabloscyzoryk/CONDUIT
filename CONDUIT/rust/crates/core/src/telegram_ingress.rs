//! Deterministic Telegram ingress rules shared by LIVE and live-backtest.
//!
//! This module deliberately contains no network, clock or file access.  The
//! caller supplies the message and the observed receive time.  Keeping these
//! rules in `conduit-core` prevents the desktop application and the Rust
//! replay runner from implementing subtly different duplicate/age gates.

use crate::engine::IncomingMessage;
use crate::parser::{self, Signal};
use crate::types::{SourceKey, Ts};
use std::collections::{HashMap, VecDeque};

/// Memory of the latest exact text observed for each Telegram message.
///
/// Telegram emits edit updates for reaction/view/link-preview changes even
/// when text is byte-for-byte unchanged.  LIVE drops those deliveries before
/// they enter the trading queue.  A live-backtest must use this exact type.
#[derive(Debug, Clone)]
pub struct ContentMemory {
    map: HashMap<(SourceKey, i64), ContentState>,
    order: VecDeque<(SourceKey, i64)>,
    capacity: usize,
}

#[derive(Debug, Clone)]
struct ContentState {
    text: String,
    reply_to: Option<i64>,
    /// An EDIT may be delivered before its NEW during reconnect/backlog.
    /// Seeing the orphan edit must not make the later canonical NEW vanish.
    saw_new: bool,
    saw_edit: bool,
}

impl Default for ContentMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentMemory {
    /// Production capacity: over three normal Synergy days while bounded.
    pub const LIVE_CAPACITY: usize = 512;

    pub fn new() -> Self {
        Self::with_capacity(Self::LIVE_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// `true` means this delivery adds no executable revision and LIVE will not forward it
    /// to the engine.  Unknown ids always pass, including an orphan edit after
    /// process restart, because there is no evidence it is a duplicate.
    pub fn duplikat_tresci(&mut self, message: &IncomingMessage) -> bool {
        let key = (message.source.clone(), message.msg_id);
        match self.map.get_mut(&key) {
            Some(previous) => {
                let incoming_is_new = message.edit_of.is_none();
                // A replayed publication carries its original timestamp, not
                // evidence of a newer revision. Once NEW + EDIT were seen, a
                // late NEW must never restore the pre-edit geometry or stop.
                if incoming_is_new && previous.saw_new && previous.saw_edit {
                    return true;
                }
                let changed_text = previous.text != message.text;
                let changed_parent = previous.reply_to != message.reply_to;
                let missing_canonical_new = incoming_is_new && !previous.saw_new;
                if changed_text || changed_parent || missing_canonical_new {
                    previous.text = message.text.clone();
                    previous.reply_to = message.reply_to;
                    previous.saw_new |= incoming_is_new;
                    previous.saw_edit |= !incoming_is_new;
                    false
                } else {
                    true
                }
            }
            None => {
                if self.order.len() >= self.capacity {
                    if let Some(oldest) = self.order.pop_front() {
                        self.map.remove(&oldest);
                    }
                }
                self.order.push_back(key.clone());
                self.map.insert(
                    key,
                    ContentState {
                        text: message.text.clone(),
                        reply_to: message.reply_to,
                        saw_new: message.edit_of.is_none(),
                        saw_edit: message.edit_of.is_some(),
                    },
                );
                false
            }
        }
    }
}

/// Whether a Telegram delivery asks to open a new basket.
///
/// Edits never go through the live stale-entry gate: they can contain crucial
/// management for an already open position even when the original post is old.
pub fn opens_basket(text: &str, edit_of: Option<i64>) -> bool {
    if edit_of.is_some() {
        return false;
    }
    parser::parse(text)
        .iter()
        .any(|signal| matches!(signal, Signal::Entry(_) | Signal::MarketOpen { .. }))
}

/// Age in minutes if a fresh entry exceeds the configured live threshold.
///
/// A non-positive threshold disables the gate.  A missing/invalid Telegram
/// timestamp and a timestamp in the future both fail open, matching LIVE.
pub fn stale_entry_age_minutes(
    received_at_ms: Ts,
    telegram_published_ms: Ts,
    max_age_min: f64,
) -> Option<f64> {
    if max_age_min <= 0.0 || telegram_published_ms <= 0 {
        return None;
    }
    let age = (received_at_ms - telegram_published_ms) as f64 / 60_000.0;
    (age > max_age_min).then_some(age)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(source: SourceKey, id: i64, edit: Option<i64>, text: &str) -> IncomingMessage {
        IncomingMessage {
            ts: 1,
            source,
            source_name: "Synergy".into(),
            msg_id: id,
            reply_to: None,
            edit_of: edit,
            text: text.into(),
        }
    }

    #[test]
    fn exact_live_content_dedup_contract() {
        let source = SourceKey::new(-1001, None);
        let mut memory = ContentMemory::new();
        assert!(!memory.duplikat_tresci(&message(source.clone(), 1, None, "TP1 HIT")));
        assert!(memory.duplikat_tresci(&message(source.clone(), 1, Some(1), "TP1 HIT")));
        assert!(!memory.duplikat_tresci(&message(source.clone(), 1, Some(1), "TP2 HIT")));
        assert!(memory.duplikat_tresci(&message(source.clone(), 1, Some(1), "TP2 HIT")));
        assert!(!memory.duplikat_tresci(&message(source, 2, None, "TP2 HIT")));
    }

    #[test]
    fn eviction_matches_the_bounded_live_memory() {
        let source = SourceKey::new(-1001, None);
        let mut memory = ContentMemory::with_capacity(2);
        assert!(!memory.duplikat_tresci(&message(source.clone(), 1, None, "A")));
        assert!(!memory.duplikat_tresci(&message(source.clone(), 2, None, "B")));
        assert!(!memory.duplikat_tresci(&message(source, 3, None, "C")));
        // id=1 was evicted, therefore the old delivery must pass fail-open.
        assert!(!memory.duplikat_tresci(&message(SourceKey::new(-1001, None), 1, Some(1), "A")));
    }

    #[test]
    fn orphan_edit_does_not_swallow_later_canonical_new() {
        let source = SourceKey::new(-1001, None);
        let mut memory = ContentMemory::new();
        assert!(!memory.duplikat_tresci(&message(source.clone(), 7, Some(7), "BUY GOLD")));
        assert!(!memory.duplikat_tresci(&message(source.clone(), 7, None, "BUY GOLD")));
        assert!(memory.duplikat_tresci(&message(source, 7, None, "BUY GOLD")));
    }

    #[test]
    fn metadata_only_parent_correction_is_not_hidden() {
        let source = SourceKey::new(-1001, None);
        let mut memory = ContentMemory::new();
        let mut wrong = message(source.clone(), 8, None, "TP1 HIT");
        wrong.reply_to = Some(9000);
        assert!(!memory.duplikat_tresci(&wrong));
        let mut corrected = message(source, 8, Some(8), "TP1 HIT");
        corrected.reply_to = Some(42);
        assert!(!memory.duplikat_tresci(&corrected));
    }

    #[test]
    fn delayed_new_cannot_roll_back_a_text_edit_or_poison_next_dedup() {
        let source = SourceKey::new(-1001, None);
        let mut memory = ContentMemory::new();
        let original = message(source.clone(), 10, None, "MOVE SL TO 2090");
        let corrected = message(source.clone(), 10, Some(10), "MOVE SL TO 2094");
        assert!(!memory.duplikat_tresci(&original));
        assert!(!memory.duplikat_tresci(&corrected));
        assert!(memory.duplikat_tresci(&original), "late NEW rolled back latest EDIT");
        assert!(memory.duplikat_tresci(&corrected), "late NEW poisoned content memory");
        assert!(!memory.duplikat_tresci(&message(source, 10, Some(10), "MOVE SL TO 2096")));
    }

    #[test]
    fn age_gate_only_classifies_fresh_entries() {
        let entry = "BUY LIMITS GOLD @ 4500/4495\nTP 4510\nSL 4490";
        assert!(opens_basket(entry, None));
        assert!(!opens_basket(entry, Some(1)));
        assert_eq!(stale_entry_age_minutes(360_001, 1, 5.0), Some(6.0));
        assert_eq!(stale_entry_age_minutes(300_001, 1, 5.0), None);
        assert_eq!(stale_entry_age_minutes(1, 10, 5.0), None);
    }
}
