//! Shared DTO contract for experimental history preview. No order execution.
use serde::{Deserialize, Serialize};

fn default_messages() -> usize {
    10_000
}
fn default_pages() -> usize {
    200
}
fn default_parents() -> usize {
    1_000
}
fn default_bytes() -> usize {
    8 * 1024 * 1024
}
fn default_page_timeout() -> u64 {
    15_000
}
fn default_total_timeout() -> u64 {
    60_000
}

const DAY: i64 = 86_400_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryWindow {
    Today,
    TwoDays,
    ThreeDays,
    OneWeek,
    TwoWeeks,
    FourWeeks,
    All,
    Custom { from_ms: i64, to_ms: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryFetchRequest {
    #[serde(default)]
    pub experimental_enabled: bool,
    pub chat_id: i64,
    /// None selects the whole chat; a number selects exactly that forum topic.
    pub topic_id: Option<i64>,
    /// Immutable cutoff chosen before fetching; later publications are excluded.
    pub cutoff_ms: i64,
    /// Midnight in the user's explicitly selected timezone, not server midnight.
    pub today_start_ms: i64,
    pub window: HistoryWindow,
    /// Limits count ALL scanned messages, including other topics and duplicates.
    #[serde(default = "default_messages")]
    pub max_messages: usize,
    #[serde(default = "default_pages")]
    pub max_pages: usize,
    #[serde(default = "default_parents")]
    pub max_parent_messages: usize,
    #[serde(default = "default_bytes")]
    pub max_text_bytes: usize,
    #[serde(default = "default_page_timeout")]
    pub page_timeout_ms: u64,
    #[serde(default = "default_total_timeout")]
    pub total_timeout_ms: u64,
}

impl HistoryFetchRequest {
    /// Entry publication bounds. Management is ALWAYS fetched through cutoff.
    pub fn entry_bounds(&self) -> Result<(i64, i64), String> {
        let from = match self.window {
            HistoryWindow::Today => self.today_start_ms,
            HistoryWindow::TwoDays => self.cutoff_ms.saturating_sub(2 * DAY),
            HistoryWindow::ThreeDays => self.cutoff_ms.saturating_sub(3 * DAY),
            HistoryWindow::OneWeek => self.cutoff_ms.saturating_sub(7 * DAY),
            HistoryWindow::TwoWeeks => self.cutoff_ms.saturating_sub(14 * DAY),
            HistoryWindow::FourWeeks => self.cutoff_ms.saturating_sub(28 * DAY),
            HistoryWindow::All => 0,
            HistoryWindow::Custom { from_ms, to_ms } => {
                if from_ms < 0 || from_ms > to_ms || to_ms > self.cutoff_ms {
                    return Err("invalid custom history window".into());
                }
                return Ok((from_ms, to_ms));
            }
        };
        if from < 0 || from > self.cutoff_ms {
            return Err("invalid history window or timezone midnight".into());
        }
        Ok((from, self.cutoff_ms))
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.experimental_enabled {
            return Err("experimental history preview is disabled".into());
        }
        self.entry_bounds()?;
        if self.chat_id == 0
            || self.topic_id.is_some_and(|x| x <= 0 || x > i32::MAX as i64)
            || self.cutoff_ms <= 0
            || self.cutoff_ms > crate::now_ms()
            || !(1..=50_000).contains(&self.max_messages)
            || !(1..=1_000).contains(&self.max_pages)
            || self.max_parent_messages > 5_000
            || !(1..=32 * 1024 * 1024).contains(&self.max_text_bytes)
            || !(1..=30_000).contains(&self.page_timeout_ms)
            || !(1..=300_000).contains(&self.total_timeout_ms)
        {
            return Err("invalid or excessive history fetch bounds".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub chat_id: i64,
    pub topic_id: Option<i64>,
    pub msg_id: i64,
    pub reply_to: Option<i64>,
    /// Cross-peer replies cannot be resolved as local basket ancestry.
    pub reply_peer_id: Option<i64>,
    pub published_ms: i64,
    pub edited_ms: Option<i64>,
    pub fetched_ms: i64,
    pub text: String,
    pub outgoing: bool,
    pub service_message: bool,
    pub parent_context_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FetchIssue {
    Timeout,
    FloodWait { seconds: Option<u32> },
    Rpc { code: i32, name: String },
    Transport,
    Protocol { detail: String },
    DataLimit { resource: String },
    ConflictingDuplicate { msg_id: i64 },
    MissingParent { msg_id: i64 },
    CrossTopicParent { msg_id: i64 },
    CrossPeerReply { msg_id: i64 },
    EditedAfterCutoff { msg_id: i64 },
    ReplyCycle { msg_id: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySnapshot {
    pub request: HistoryFetchRequest,
    pub started_ms: i64,
    pub completed_ms: i64,
    pub records: Vec<HistoryRecord>,
    pub pages_fetched: usize,
    pub parent_batches_fetched: usize,
    pub raw_messages_scanned: usize,
    pub duplicate_records: usize,
    pub other_topic_records: usize,
    pub publications_after_cutoff: usize,
    pub scanned_text_bytes: usize,
    /// All currently visible pages reached selected lower bound or API exhaustion.
    pub visible_pages_complete: bool,
    pub reply_parents_complete: bool,
    /// ALWAYS false for this GetHistory-only producer.
    pub prior_edits_complete: bool,
    /// ALWAYS false for this GetHistory-only producer.
    pub deletions_complete: bool,
    /// Fetch is paginated, not an atomic server snapshot at cutoff.
    pub atomic_at_cutoff: bool,
    /// Grammers may retry internally; an outer timeout is not proof of FLOOD_WAIT.
    pub library_retry_may_be_hidden: bool,
    pub issues: Vec<FetchIssue>,
}

impl HistorySnapshot {
    pub fn can_submit_orders(&self) -> bool {
        false
    }
    pub fn data_limit_hit(&self) -> bool {
        self.issues
            .iter()
            .any(|x| matches!(x, FetchIssue::DataLimit { .. }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPreviewDecision {
    pub chat_id: i64,
    pub msg_id: i64,
    pub published_ms: i64,
    pub status: String,
    pub reasons: Vec<String>,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPreviewResponse {
    pub read_only: bool,
    pub can_submit_orders: bool,
    pub snapshot: HistorySnapshot,
    pub decisions: Vec<HistoryPreviewDecision>,
}

/// Reuse production source routing. Mixed forums must select a specific topic.
pub fn source_allowed(
    request: &HistoryFetchRequest,
    bindings: &std::collections::BTreeMap<String, crate::ui::ChannelBinding>,
) -> bool {
    bindings.values().any(|b| {
        b.channel_id == request.chat_id
            && b.obserwuje(request.topic_id)
            && (b.topics.is_empty() || request.topic_id.is_some())
            && b.format_dla(request.topic_id)
                .is_some_and(|f| f.trim().eq_ignore_ascii_case("Synergy"))
    })
}

pub async fn preview_endpoint(
    axum::extract::State(st): axum::extract::State<crate::state::StateHandle>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    axum::Json(request): axum::Json<HistoryFetchRequest>,
) -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse, Json};
    let error = |status: StatusCode, message: String| {
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    };
    if !peer.ip().is_loopback() {
        return error(
            StatusCode::FORBIDDEN,
            "history preview is localhost-only".into(),
        );
    }
    if !request.experimental_enabled {
        return error(
            StatusCode::FORBIDDEN,
            "experimental history preview is disabled".into(),
        );
    }
    if let Err(e) = request.validate() {
        return error(StatusCode::BAD_REQUEST, e);
    }
    if !st.read(|s| source_allowed(&request, &s.bindings)) {
        return error(
            StatusCode::FORBIDDEN,
            "select an observed Synergy source/topic".into(),
        );
    }
    if !st.auth.state().is_logged_in() {
        return error(
            StatusCode::CONFLICT,
            "existing Telegram session is not logged in".into(),
        );
    }
    let auth = st.auth.clone();
    let timeout = std::time::Duration::from_millis(request.total_timeout_ms + 3_000);
    match tokio::time::timeout(timeout, auth.history_import_preview(request)).await {
        Ok(Ok(result)) => Json(result).into_response(),
        Ok(Err(e)) => error(StatusCode::CONFLICT, e),
        Err(_) => error(
            StatusCode::GATEWAY_TIMEOUT,
            "bounded history preview timed out".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> HistoryFetchRequest {
        serde_json::from_value(serde_json::json!({
            "chat_id": -100123, "topic_id": null, "cutoff_ms": 1700000000000i64,
            "today_start_ms": 1699920000000i64, "window": {"kind":"today"}
        }))
        .unwrap()
    }

    #[test]
    fn preview_is_off_by_default_and_limits_are_bounded() {
        let mut r = request();
        assert!(!r.experimental_enabled);
        assert!(r.validate().is_err());
        r.experimental_enabled = true;
        assert!(r.validate().is_ok());
        assert_eq!(r.max_messages, 10_000);
        assert_eq!(r.total_timeout_ms, 60_000);
        r.max_messages = 50_001;
        assert!(r.validate().is_err());
    }

    #[test]
    fn custom_window_does_not_drop_management_after_entry_window() {
        let mut r = request();
        r.window = HistoryWindow::Custom {
            from_ms: 1000,
            to_ms: 2000,
        };
        assert_eq!(r.entry_bounds().unwrap(), (1000, 2000));
        assert!(r.cutoff_ms > r.entry_bounds().unwrap().1);
        r.window = HistoryWindow::Custom {
            from_ms: 1000,
            to_ms: r.cutoff_ms + 1,
        };
        assert!(r.entry_bounds().is_err());
    }

    #[test]
    fn routing_requires_observed_synergy_and_exact_mixed_forum_topic() {
        let mut r = request();
        let mut b: crate::ui::ChannelBinding = serde_json::from_value(serde_json::json!({
            "channelId":-100123, "monitored":true,"notify":false,"format":"Synergy","topics":{}
        }))
        .unwrap();
        let mut bindings = std::collections::BTreeMap::from([("s".into(), b.clone())]);
        assert!(source_allowed(&r, &bindings));
        b.monitored = false;
        bindings.insert("s".into(), b.clone());
        assert!(!source_allowed(&r, &bindings));
        b.monitored = true;
        b.topics.insert("3".into(), "Synergy".into());
        b.topics.insert("4".into(), "Other".into());
        bindings.insert("s".into(), b);
        assert!(!source_allowed(&r, &bindings));
        r.topic_id = Some(3);
        assert!(source_allowed(&r, &bindings));
        r.topic_id = Some(4);
        assert!(!source_allowed(&r, &bindings));
    }

    #[tokio::test]
    async fn endpoint_rejects_remote_and_disabled_without_auth_hook() {
        use axum::{
            extract::{ConnectInfo, State},
            http::StatusCode,
            Json,
        };
        let cfg = crate::ServerConfig {
            workspace: std::env::temp_dir().join(format!(
                "conduit-history-preview-{}-{}",
                std::process::id(),
                crate::now_ms()
            )),
            ..Default::default()
        };
        let st = crate::bootstrap(&cfg, crate::default_auth()).unwrap();
        let mut r = request();
        r.experimental_enabled = true;
        let response = preview_endpoint(
            State(st.clone()),
            ConnectInfo("192.168.1.1:42".parse().unwrap()),
            Json(r),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let response = preview_endpoint(
            State(st.clone()),
            ConnectInfo("127.0.0.1:42".parse().unwrap()),
            Json(request()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let mut r = request();
        r.experimental_enabled = true;
        let response = preview_endpoint(
            State(st),
            ConnectInfo("127.0.0.1:42".parse().unwrap()),
            Json(r),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "unobserved source must fail before auth hook"
        );
    }
}
