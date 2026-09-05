//! Bounded, read-only current-history snapshots. No MessageSink or Engine access.
//! A complete pagination is NOT a complete edit/deletion history.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use conduit_server::history_import::{
    FetchIssue, HistoryFetchRequest, HistoryRecord, HistorySnapshot, HistoryWindow,
};
use grammers_client::{session::types::PeerRef, tl, Client, InvocationError};
use tokio::time::Instant;

use crate::incoming;

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|x| x.as_millis() as i64)
        .unwrap_or(0)
}

// Kept independent from grammers for deterministic fake-pagination tests.
pub(crate) struct HistoryPage {
    pub records: Vec<HistoryRecord>,
    pub exhausted: bool,
}

pub(crate) trait HistorySource {
    fn page(
        &mut self,
        offset_id: i32,
        limit: usize,
    ) -> impl Future<Output = Result<HistoryPage, FetchIssue>> + Send;
    fn parents(
        &mut self,
        ids: Vec<i32>,
    ) -> impl Future<Output = Result<Vec<Option<HistoryRecord>>, FetchIssue>> + Send;
}

pub(crate) struct TelegramHistorySource {
    pub client: Client,
    pub peer: PeerRef,
    pub chat_is_forum: bool,
}

fn rpc_issue(e: InvocationError) -> FetchIssue {
    match e {
        InvocationError::Rpc(r) if r.name.contains("FLOOD") => {
            FetchIssue::FloodWait { seconds: r.value }
        }
        InvocationError::Rpc(r) => FetchIssue::Rpc {
            code: r.code,
            name: r.name,
        },
        _ => FetchIssue::Transport,
    }
}

fn record_from_raw(raw: &tl::enums::Message, is_forum: bool) -> Option<HistoryRecord> {
    use grammers_client::session::types::PeerId;
    let (id, peer, date, reply, edited, text, out, service) = match raw {
        tl::enums::Message::Message(m) => (
            m.id,
            &m.peer_id,
            m.date,
            m.reply_to.as_ref(),
            m.edit_date,
            m.message.clone(),
            m.out,
            false,
        ),
        tl::enums::Message::Service(m) => (
            m.id,
            &m.peer_id,
            m.date,
            m.reply_to.as_ref(),
            None,
            String::new(),
            m.out,
            true,
        ),
        tl::enums::Message::Empty(_) => return None,
    };
    let (mut topic_id, reply_to, _) = incoming::split_reply(reply);
    if is_forum && topic_id.is_none() {
        topic_id = Some(incoming::GENERAL_TOPIC);
    }
    let reply_peer_id = match reply {
        Some(tl::enums::MessageReplyHeader::Header(r)) => r
            .reply_to_peer_id
            .as_ref()
            .map(|p| PeerId::from(p.clone()).bot_api_dialog_id_unchecked()),
        _ => None,
    };
    Some(HistoryRecord {
        chat_id: PeerId::from(peer.clone()).bot_api_dialog_id_unchecked(),
        topic_id,
        msg_id: id as i64,
        reply_to,
        reply_peer_id,
        published_ms: date as i64 * 1000,
        edited_ms: edited.map(|x| x as i64 * 1000),
        fetched_ms: now_ms(),
        text,
        outgoing: out,
        service_message: service,
        parent_context_only: false,
    })
}

impl HistorySource for TelegramHistorySource {
    async fn page(&mut self, offset_id: i32, limit: usize) -> Result<HistoryPage, FetchIssue> {
        let reply = self
            .client
            .invoke(&tl::functions::messages::GetHistory {
                peer: self.peer.into(),
                offset_id,
                offset_date: 0,
                add_offset: 0,
                limit: limit as i32,
                max_id: 0,
                min_id: 0,
                hash: 0,
            })
            .await
            .map_err(rpc_issue)?;
        let (raw, exhausted) = match reply {
            tl::enums::messages::Messages::Messages(m) => (m.messages, true),
            tl::enums::messages::Messages::Slice(m) => (m.messages, false),
            tl::enums::messages::Messages::ChannelMessages(m) => (m.messages, false),
            tl::enums::messages::Messages::NotModified(_) => {
                return Err(FetchIssue::Protocol {
                    detail: "NotModified with hash=0".into(),
                })
            }
        };
        // A short nonempty page is NOT exhaustion (Telegram may have ID holes).
        let exhausted = exhausted || raw.is_empty();
        let raw_len = raw.len();
        let records: Vec<_> = raw
            .iter()
            .filter_map(|m| record_from_raw(m, self.chat_is_forum))
            .collect();
        if records.len() != raw_len {
            return Err(FetchIssue::Protocol {
                detail: "history page contains an inaccessible/empty message".into(),
            });
        }
        Ok(HistoryPage { records, exhausted })
    }

    async fn parents(&mut self, ids: Vec<i32>) -> Result<Vec<Option<HistoryRecord>>, FetchIssue> {
        let messages = self
            .client
            .get_messages_by_id(self.peer, &ids)
            .await
            .map_err(rpc_issue)?;
        Ok(messages
            .iter()
            .map(|m| {
                m.as_ref()
                    .and_then(|m| record_from_raw(&m.raw, self.chat_is_forum))
            })
            .collect())
    }
}

async fn bounded<T>(
    deadline: Instant,
    page_ms: u64,
    future: impl Future<Output = Result<T, FetchIssue>>,
) -> Result<T, FetchIssue> {
    if Instant::now() >= deadline {
        return Err(FetchIssue::Timeout);
    }
    let end = deadline.min(Instant::now() + Duration::from_millis(page_ms));
    let result = tokio::time::timeout_at(end, future)
        .await
        .unwrap_or(Err(FetchIssue::Timeout));
    // Tokio may poll a ready inner future before the timer after scheduler lag.
    if Instant::now() > end {
        Err(FetchIssue::Timeout)
    } else {
        result
    }
}

fn same_record(a: &HistoryRecord, b: &HistoryRecord) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.fetched_ms = 0;
    b.fetched_ms = 0;
    a.parent_context_only = false;
    b.parent_context_only = false;
    a == b
}

fn insert_record(
    map: &mut BTreeMap<i64, HistoryRecord>,
    record: HistoryRecord,
    out: &mut HistorySnapshot,
) {
    if let Some(old) = map.get(&record.msg_id) {
        out.duplicate_records += 1;
        if !same_record(old, &record) {
            out.issues.push(FetchIssue::ConflictingDuplicate {
                msg_id: record.msg_id,
            });
        }
    } else {
        map.insert(record.msg_id, record);
    }
}

fn inspect_record(record: &HistoryRecord, out: &mut HistorySnapshot) -> bool {
    if record.chat_id != out.request.chat_id
        || record.msg_id <= 0
        || record.msg_id > i32::MAX as i64
        || record
            .reply_to
            .is_some_and(|id| id <= 0 || id > i32::MAX as i64)
        || record.published_ms <= 0
        || record.published_ms > record.fetched_ms
        || record
            .edited_ms
            .is_some_and(|e| e < record.published_ms || e > record.fetched_ms)
    {
        out.issues.push(FetchIssue::Protocol {
            detail: "invalid message identity/date metadata".into(),
        });
        return false;
    }
    out.raw_messages_scanned += 1;
    out.scanned_text_bytes = out.scanned_text_bytes.saturating_add(record.text.len());
    if out.raw_messages_scanned > out.request.max_messages
        || out.scanned_text_bytes > out.request.max_text_bytes
    {
        out.issues.push(FetchIssue::DataLimit {
            resource: "messages_or_text_bytes".into(),
        });
        return false;
    }
    if record.reply_peer_id.is_some_and(|id| id != record.chat_id) {
        out.issues.push(FetchIssue::CrossPeerReply {
            msg_id: record.msg_id,
        });
    }
    if record
        .edited_ms
        .is_some_and(|ts| ts > out.request.cutoff_ms)
    {
        out.issues.push(FetchIssue::EditedAfterCutoff {
            msg_id: record.msg_id,
        });
    }
    true
}

/// Collects only current-history evidence. Always returns partial data on RPC/
/// timeout/cap failure. No retries here, no credentials, no live update stream.
pub(crate) async fn fetch_with_source<S: HistorySource>(
    source: &mut S,
    request: HistoryFetchRequest,
    deadline: Instant,
) -> HistorySnapshot {
    let from_ms = request.entry_bounds().map(|x| x.0).unwrap_or(0);
    let mut out = HistorySnapshot {
        request,
        started_ms: now_ms(),
        completed_ms: 0,
        records: vec![],
        pages_fetched: 0,
        parent_batches_fetched: 0,
        raw_messages_scanned: 0,
        duplicate_records: 0,
        other_topic_records: 0,
        publications_after_cutoff: 0,
        scanned_text_bytes: 0,
        visible_pages_complete: false,
        reply_parents_complete: false,
        prior_edits_complete: false,
        deletions_complete: false,
        atomic_at_cutoff: false,
        library_retry_may_be_hidden: true,
        issues: vec![],
    };
    if let Err(detail) = out.request.validate() {
        out.issues.push(FetchIssue::Protocol { detail });
        out.completed_ms = now_ms();
        return out;
    }
    let mut map = BTreeMap::new();
    let mut offset = 0;
    let mut last_date = i64::MAX;
    'pages: loop {
        if out.pages_fetched >= out.request.max_pages
            || out.raw_messages_scanned >= out.request.max_messages
        {
            out.issues.push(FetchIssue::DataLimit {
                resource: "pages_or_messages".into(),
            });
            break;
        }
        let limit = 100.min(out.request.max_messages - out.raw_messages_scanned);
        let page = match bounded(
            deadline,
            out.request.page_timeout_ms,
            source.page(offset, limit),
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                out.issues.push(e);
                break;
            }
        };
        out.pages_fetched += 1;
        if page.records.len() > limit {
            out.issues.push(FetchIssue::Protocol {
                detail: "page exceeds requested size".into(),
            });
            break;
        }
        let mut next_offset = offset;
        let mut crossed_cutoff = false;
        for row in page.records {
            if !inspect_record(&row, &mut out) {
                break 'pages;
            }
            if row.published_ms > last_date || (next_offset > 0 && row.msg_id > next_offset as i64)
            {
                out.issues.push(FetchIssue::Protocol {
                    detail: "history is not descending by date/id".into(),
                });
                break 'pages;
            }
            last_date = row.published_ms;
            next_offset = if next_offset == 0 {
                row.msg_id as i32
            } else {
                next_offset.min(row.msg_id as i32)
            };
            if row.published_ms < from_ms {
                crossed_cutoff = true;
                continue;
            }
            if row.published_ms > out.request.cutoff_ms {
                out.publications_after_cutoff += 1;
                continue;
            }
            if out
                .request
                .topic_id
                .is_some_and(|t| row.topic_id != Some(t))
            {
                out.other_topic_records += 1;
                continue;
            }
            insert_record(&mut map, row, &mut out);
        }
        if crossed_cutoff || page.exhausted {
            out.visible_pages_complete = true;
            break;
        }
        if next_offset <= 0 || (offset > 0 && next_offset >= offset) {
            out.issues.push(FetchIssue::Protocol {
                detail: "pagination cursor did not advance".into(),
            });
            break;
        }
        offset = next_offset;
    }
    // Resolve ancestors transitively, including roots older than selected window.
    // They are context-only and never become eligible entry candidates.
    let mut requested = BTreeSet::new();
    loop {
        let parents: BTreeSet<_> = map
            .values()
            .filter(|r| r.reply_peer_id.is_none_or(|id| id == r.chat_id))
            .filter_map(|r| r.reply_to)
            .filter(|id| !map.contains_key(id) && !requested.contains(id))
            .collect();
        if parents.is_empty() {
            break;
        }
        if requested.len() >= out.request.max_parent_messages {
            out.issues.push(FetchIssue::DataLimit {
                resource: "reply_parents".into(),
            });
            break;
        }
        let count = 100
            .min(out.request.max_parent_messages - requested.len())
            .min(
                out.request
                    .max_messages
                    .saturating_sub(out.raw_messages_scanned),
            );
        if count == 0 {
            out.issues.push(FetchIssue::DataLimit {
                resource: "parent_message_budget".into(),
            });
            break;
        }
        let ids: Vec<i32> = parents
            .into_iter()
            .take(count)
            .filter_map(|id| i32::try_from(id).ok())
            .collect();
        if ids.is_empty() {
            out.issues.push(FetchIssue::Protocol {
                detail: "invalid parent id".into(),
            });
            break;
        }
        requested.extend(ids.iter().map(|id| *id as i64));
        let rows = match bounded(
            deadline,
            out.request.page_timeout_ms,
            source.parents(ids.clone()),
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                out.issues.push(e);
                break;
            }
        };
        out.parent_batches_fetched += 1;
        if rows.len() != ids.len() {
            out.issues.push(FetchIssue::Protocol {
                detail: "parent response size mismatch".into(),
            });
            break;
        }
        for (id, row) in ids.into_iter().zip(rows) {
            let Some(mut row) = row else {
                out.issues
                    .push(FetchIssue::MissingParent { msg_id: id as i64 });
                continue;
            };
            if row.msg_id != id as i64
                || row.published_ms > out.request.cutoff_ms
                || !inspect_record(&row, &mut out)
            {
                out.issues.push(FetchIssue::Protocol {
                    detail: "invalid parent result".into(),
                });
                continue;
            }
            if out
                .request
                .topic_id
                .is_some_and(|t| row.topic_id != Some(t))
            {
                out.issues
                    .push(FetchIssue::CrossTopicParent { msg_id: row.msg_id });
            }
            row.parent_context_only = true;
            insert_record(&mut map, row, &mut out);
        }
        if out.data_limit_hit() {
            break;
        }
    }
    for start in map.keys() {
        let mut seen = BTreeSet::new();
        let mut id = *start;
        while let Some(row) = map.get(&id) {
            if !seen.insert(id) {
                out.issues.push(FetchIssue::ReplyCycle { msg_id: id });
                break;
            }
            let Some(parent) = row.reply_to else {
                break;
            };
            id = parent;
        }
    }
    out.reply_parents_complete = map
        .values()
        .all(|r| r.reply_to.is_none_or(|id| map.contains_key(&id)))
        && !out.issues.iter().any(|x| {
            matches!(
                x,
                FetchIssue::CrossTopicParent { .. }
                    | FetchIssue::CrossPeerReply { .. }
                    | FetchIssue::ReplyCycle { .. }
            )
        });
    out.records = map.into_values().collect();
    out.records.sort_by_key(|r| (r.published_ms, r.msg_id));
    out.completed_ms = now_ms();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Fake {
        pages: VecDeque<Result<HistoryPage, FetchIssue>>,
        parents: BTreeMap<i64, HistoryRecord>,
        offsets: Vec<i32>,
        parent_calls: Vec<Vec<i32>>,
        delay: bool,
    }
    impl Fake {
        fn new(pages: Vec<HistoryPage>) -> Self {
            Self {
                pages: pages.into_iter().map(Ok).collect(),
                parents: BTreeMap::new(),
                offsets: vec![],
                parent_calls: vec![],
                delay: false,
            }
        }
    }
    impl HistorySource for Fake {
        async fn page(&mut self, offset: i32, _limit: usize) -> Result<HistoryPage, FetchIssue> {
            self.offsets.push(offset);
            if self.delay {
                std::future::pending::<()>().await;
            }
            self.pages
                .pop_front()
                .unwrap_or_else(|| Ok(page(vec![], true)))
        }
        async fn parents(
            &mut self,
            ids: Vec<i32>,
        ) -> Result<Vec<Option<HistoryRecord>>, FetchIssue> {
            self.parent_calls.push(ids.clone());
            Ok(ids
                .iter()
                .map(|id| self.parents.get(&(*id as i64)).cloned())
                .collect())
        }
    }
    fn row(id: i64) -> HistoryRecord {
        HistoryRecord {
            chat_id: -100123,
            topic_id: None,
            msg_id: id,
            reply_to: None,
            reply_peer_id: None,
            published_ms: id * 1000,
            edited_ms: None,
            fetched_ms: 9000,
            text: format!("message {id}"),
            outgoing: false,
            service_message: false,
            parent_context_only: false,
        }
    }
    fn page(records: Vec<HistoryRecord>, exhausted: bool) -> HistoryPage {
        HistoryPage { records, exhausted }
    }
    fn request() -> HistoryFetchRequest {
        HistoryFetchRequest {
            experimental_enabled: true,
            chat_id: -100123,
            topic_id: None,
            cutoff_ms: 8000,
            today_start_ms: 4000,
            window: HistoryWindow::Today,
            max_messages: 100,
            max_pages: 10,
            max_parent_messages: 10,
            max_text_bytes: 10000,
            page_timeout_ms: 100,
            total_timeout_ms: 1000,
        }
    }
    async fn run(fake: &mut Fake, r: HistoryFetchRequest) -> HistorySnapshot {
        fetch_with_source(fake, r, Instant::now() + Duration::from_secs(1)).await
    }

    #[tokio::test]
    async fn short_pages_continue_dedup_and_cutoff_is_inclusive() {
        let mut f = Fake::new(vec![
            page(vec![row(8), row(7)], false),
            page(vec![row(7), row(6), row(4), row(3)], false),
        ]);
        let s = run(&mut f, request()).await;
        assert_eq!(f.offsets, vec![0, 7]);
        assert_eq!(s.duplicate_records, 1);
        assert_eq!(
            s.records.iter().map(|r| r.msg_id).collect::<Vec<_>>(),
            vec![4, 6, 7, 8]
        );
        assert!(s.visible_pages_complete);
        assert!(s.reply_parents_complete);
        assert!(!s.prior_edits_complete);
        assert!(!s.deletions_complete);
        assert!(!s.atomic_at_cutoff);
        assert!(!s.can_submit_orders());
    }
    #[tokio::test]
    async fn all_window_requires_actual_exhaustion_not_short_page() {
        let mut r = request();
        r.window = HistoryWindow::All;
        let mut f = Fake::new(vec![
            page(vec![row(8)], false),
            page(vec![row(2)], false),
            page(vec![], true),
        ]);
        let s = run(&mut f, r).await;
        assert_eq!(s.pages_fetched, 3);
        assert!(s.visible_pages_complete);
    }
    #[tokio::test]
    async fn custom_entry_end_does_not_drop_later_management() {
        let mut r = request();
        r.window = HistoryWindow::Custom {
            from_ms: 4000,
            to_ms: 5000,
        };
        let mut f = Fake::new(vec![page(vec![row(8), row(5), row(4)], true)]);
        let s = run(&mut f, r).await;
        assert_eq!(s.records.len(), 3);
        assert_eq!(s.request.entry_bounds().unwrap(), (4000, 5000));
    }
    #[tokio::test]
    async fn selected_topic_is_exact_but_scanned_limit_counts_all_topics() {
        let mut r = request();
        r.topic_id = Some(2);
        let mut a = row(8);
        a.topic_id = Some(2);
        let mut b = row(7);
        b.topic_id = Some(3);
        let mut f = Fake::new(vec![page(vec![a, b, row(4)], true)]);
        let s = run(&mut f, r).await;
        assert_eq!(s.records.len(), 1);
        assert_eq!(s.other_topic_records, 2);
        assert_eq!(s.raw_messages_scanned, 3);
    }
    #[tokio::test]
    async fn resolves_transitive_parent_outside_window_without_importing_it() {
        let mut child = row(8);
        child.reply_to = Some(3);
        let mut parent = row(3);
        parent.reply_to = Some(1);
        let mut f = Fake::new(vec![page(vec![child], true)]);
        f.parents.insert(3, parent);
        f.parents.insert(1, row(1));
        let s = run(&mut f, request()).await;
        assert_eq!(f.parent_calls, vec![vec![3], vec![1]]);
        assert!(s.reply_parents_complete);
        assert!(s.records[0].parent_context_only);
        assert_eq!(s.records.len(), 3);
    }
    #[tokio::test]
    async fn missing_parent_is_unknown_not_deleted_or_complete() {
        let mut child = row(8);
        child.reply_to = Some(3);
        let mut f = Fake::new(vec![page(vec![child], true)]);
        let s = run(&mut f, request()).await;
        assert!(!s.reply_parents_complete);
        assert!(s.issues.contains(&FetchIssue::MissingParent { msg_id: 3 }));
        assert!(!s.deletions_complete);
    }
    #[tokio::test]
    async fn cross_topic_parent_is_flagged() {
        let mut r = request();
        r.topic_id = Some(2);
        let mut child = row(8);
        child.topic_id = Some(2);
        child.reply_to = Some(3);
        let mut parent = row(3);
        parent.topic_id = Some(3);
        let mut f = Fake::new(vec![page(vec![child], true)]);
        f.parents.insert(3, parent);
        let s = run(&mut f, r).await;
        assert!(s
            .issues
            .contains(&FetchIssue::CrossTopicParent { msg_id: 3 }));
        assert!(!s.reply_parents_complete);
    }
    #[tokio::test]
    async fn cross_peer_reply_never_fetches_local_id() {
        let mut child = row(8);
        child.reply_to = Some(3);
        child.reply_peer_id = Some(-100999);
        let mut f = Fake::new(vec![page(vec![child], true)]);
        let s = run(&mut f, request()).await;
        assert!(f.parent_calls.is_empty());
        assert!(!s.reply_parents_complete);
        assert!(s.issues.contains(&FetchIssue::CrossPeerReply { msg_id: 8 }));
    }
    #[tokio::test]
    async fn parent_cycle_is_flagged() {
        let mut a = row(8);
        a.reply_to = Some(7);
        let mut b = row(7);
        b.reply_to = Some(8);
        let mut f = Fake::new(vec![page(vec![a, b], true)]);
        let s = run(&mut f, request()).await;
        assert!(!s.reply_parents_complete);
        assert!(s
            .issues
            .iter()
            .any(|i| matches!(i, FetchIssue::ReplyCycle { .. })));
    }
    #[tokio::test]
    async fn no_progress_does_not_loop_forever() {
        let mut f = Fake::new(vec![page(vec![row(8)], false), page(vec![row(8)], false)]);
        let s = run(&mut f, request()).await;
        assert_eq!(s.pages_fetched, 2);
        assert!(!s.visible_pages_complete);
        assert!(!s.issues.is_empty());
    }
    #[tokio::test]
    async fn conflicting_duplicate_preserves_first_and_flags_uncertainty() {
        let mut changed = row(8);
        changed.text = "edited".into();
        changed.edited_ms = Some(8500);
        let mut f = Fake::new(vec![
            page(vec![row(8)], false),
            page(vec![changed, row(4)], true),
        ]);
        let s = run(&mut f, request()).await;
        assert!(s
            .issues
            .contains(&FetchIssue::ConflictingDuplicate { msg_id: 8 }));
        assert_eq!(s.records[1].text, "message 8");
    }
    #[tokio::test]
    async fn publication_after_cutoff_excluded_edit_after_cutoff_explicit() {
        let mut edited = row(8);
        edited.edited_ms = Some(8500);
        let mut f = Fake::new(vec![page(vec![row(9), edited], true)]);
        let s = run(&mut f, request()).await;
        assert_eq!(s.publications_after_cutoff, 1);
        assert_eq!(s.records.len(), 1);
        assert!(s
            .issues
            .contains(&FetchIssue::EditedAfterCutoff { msg_id: 8 }));
        assert_eq!(s.records[0].edited_ms, Some(8500));
    }
    #[tokio::test]
    async fn invalid_date_does_not_fake_lower_bound_completion() {
        let mut bad = row(8);
        bad.edited_ms = Some(7000);
        let mut f = Fake::new(vec![page(vec![bad], true)]);
        let s = run(&mut f, request()).await;
        assert!(!s.visible_pages_complete);
        assert!(s.records.is_empty());
    }
    #[tokio::test]
    async fn nonmonotonic_history_is_incomplete() {
        let mut f = Fake::new(vec![page(vec![row(7), row(8)], true)]);
        let s = run(&mut f, request()).await;
        assert!(!s.visible_pages_complete);
        assert!(s
            .issues
            .iter()
            .any(|x| matches!(x, FetchIssue::Protocol { .. })));
    }
    #[tokio::test]
    async fn timeout_is_partial_and_not_mislabelled_flood() {
        let mut r = request();
        r.page_timeout_ms = 1;
        let mut f = Fake::new(vec![]);
        f.delay = true;
        let s = run(&mut f, r).await;
        assert_eq!(s.issues, vec![FetchIssue::Timeout]);
        assert!(!s.visible_pages_complete);
    }
    #[tokio::test]
    async fn surfaced_flood_returns_without_retry_and_keeps_prior_page() {
        let mut f = Fake::new(vec![page(vec![row(8)], false)]);
        f.pages
            .push_back(Err(FetchIssue::FloodWait { seconds: Some(60) }));
        let s = run(&mut f, request()).await;
        assert_eq!(f.offsets.len(), 2);
        assert_eq!(s.records.len(), 1);
        assert!(s
            .issues
            .contains(&FetchIssue::FloodWait { seconds: Some(60) }));
        assert!(!s.visible_pages_complete);
    }
    #[tokio::test]
    async fn page_and_byte_caps_are_not_completion() {
        let mut r = request();
        r.max_pages = 1;
        let mut f = Fake::new(vec![page(vec![row(8)], false)]);
        let s = run(&mut f, r).await;
        assert!(s.data_limit_hit());
        assert!(!s.visible_pages_complete);
        let mut r = request();
        r.max_text_bytes = 1;
        let mut f = Fake::new(vec![page(vec![row(8)], true)]);
        let s = run(&mut f, r).await;
        assert!(s.data_limit_hit());
        assert!(!s.visible_pages_complete);
        assert!(s.records.is_empty());
    }
    #[tokio::test]
    async fn zero_parent_budget_is_honest_incomplete() {
        let mut r = request();
        r.max_parent_messages = 0;
        let mut child = row(8);
        child.reply_to = Some(3);
        let mut f = Fake::new(vec![page(vec![child], true)]);
        let s = run(&mut f, r).await;
        assert!(f.parent_calls.is_empty());
        assert!(!s.reply_parents_complete);
        assert!(s.data_limit_hit());
    }
    #[tokio::test]
    async fn disabled_fetch_does_not_touch_source() {
        let mut r = request();
        r.experimental_enabled = false;
        let mut f = Fake::new(vec![]);
        let s = run(&mut f, r).await;
        assert!(f.offsets.is_empty());
        assert!(!s.visible_pages_complete);
    }
}
