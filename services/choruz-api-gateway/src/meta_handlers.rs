use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{LazyLock, Mutex},
    time::Instant,
};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use choruz_common::{
    AppError,
    metrics::{self, Histogram, IntCounter, IntGauge},
};
use choruz_domain::{
    AuditLog, ChannelVisibility, Company, Conversation, Message, Principal, PrincipalType,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ApiError, ApiState, authenticated_principal};

// ── Core handlers (health, metrics, console) ──────────────────────────

static HTTP_REQUESTS_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    metrics::register_counter(
        "choruz_http_requests_total",
        "HTTP requests received by the gateway.",
    )
});
static HTTP_REQUEST_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    metrics::register_histogram(
        "choruz_http_request_duration",
        "Gateway request latency in seconds.",
        vec![0.05, 0.2, 1.0],
    )
});
static PRINCIPALS_TOTAL: LazyLock<IntGauge> = LazyLock::new(|| {
    metrics::register_gauge(
        "choruz_principals_total",
        "Principals in the application state.",
    )
});
static CONVERSATIONS_TOTAL: LazyLock<IntGauge> = LazyLock::new(|| {
    metrics::register_gauge(
        "choruz_conversations_total",
        "Conversations in the application state.",
    )
});
static MESSAGES_TOTAL: LazyLock<IntGauge> = LazyLock::new(|| {
    metrics::register_gauge(
        "choruz_messages_total",
        "Messages injected since the gateway started.",
    )
});
static AUDIT_LOGS_TOTAL: LazyLock<IntGauge> = LazyLock::new(|| {
    metrics::register_gauge(
        "choruz_audit_logs_total",
        "Audit log entries in the application state.",
    )
});
static EVENT_BACKLOG_TOTAL: LazyLock<IntGauge> = LazyLock::new(|| {
    metrics::register_gauge(
        "choruz_event_backlog_total",
        "Events queued for delivery in the application state.",
    )
});

/// A scrape refreshes the gauges from its own `ChatApp` and encodes under this
/// lock, so a response never carries another router's refresh: the registry is
/// process-wide, and a test binary drives several routers at once.
static SCRAPE: Mutex<()> = Mutex::new(());

pub(crate) async fn liveness() -> impl IntoResponse {
    Json(choruz_common::HostServiceStatus::new(
        "choruz-api-gateway",
        "ok",
    ))
}

pub(crate) async fn readiness(State(state): State<ApiState>) -> impl IntoResponse {
    let db_ok = state.runtime.health_check().await.is_ok();
    let status = if db_ok { "ready" } else { "not_ready" };
    let code = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(choruz_common::HostServiceStatus {
            database: Some(db_ok),
            ..choruz_common::HostServiceStatus::new("choruz-api-gateway", status)
        }),
    )
}

pub(crate) async fn request_logging_middleware(
    request: axum::extract::Request,
    next: Next,
) -> Response {
    use tracing::Instrument;

    HTTP_REQUESTS_TOTAL.inc();

    let request_id = choruz_common::new_id();
    let method = request.method().clone();
    let uri = request.uri().path().to_string();
    let start = Instant::now();

    // Propagate front-end trace ID if present. Convention: the literal
    // "none" means the request was genuinely untraced (header absent);
    // different from "-" which is ambiguous with "dropped along the way".
    // Downstream layers (db_service, router, executor, writer) also use
    // "none", so grepping prod logs for either the real 8-char hex id or
    // the literal "none" is enough to answer "did this request carry a
    // trace" unambiguously.
    let trace_id = request
        .headers()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none")
        .to_string();

    let span = tracing::info_span!("request", request_id = %request_id, method = %method, path = %uri, trace_id = %trace_id);

    let mut response = async { next.run(request).await }
        .instrument(span.clone())
        .await;

    // Attach request_id to response headers for client-side correlation
    if let Ok(val) = request_id.parse() {
        response.headers_mut().insert("x-request-id", val);
    }

    let status = response.status();
    let elapsed = start.elapsed();

    HTTP_REQUEST_DURATION.observe(elapsed.as_secs_f64());

    let _guard = span.enter();
    tracing::info!(
        status = status.as_u16(),
        elapsed_ms = elapsed.as_millis() as u64,
        "completed"
    );

    response
}

pub(crate) async fn metrics(State(state): State<ApiState>) -> impl IntoResponse {
    let snapshot = state.app.metrics_snapshot();
    let body = {
        let _scrape = SCRAPE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        PRINCIPALS_TOTAL.set(snapshot.principals_total as i64);
        CONVERSATIONS_TOTAL.set(snapshot.conversations_total as i64);
        MESSAGES_TOTAL.set(snapshot.messages_total as i64);
        AUDIT_LOGS_TOTAL.set(snapshot.audit_logs_total as i64);
        EVENT_BACKLOG_TOTAL.set(snapshot.event_backlog_total as i64);
        metrics::text()
    };

    ([(CONTENT_TYPE, metrics::TEXT_CONTENT_TYPE)], body)
}

pub(crate) async fn phase_status(
    State(state): State<ApiState>,
) -> Json<choruz_application::PhaseStatus> {
    Json(state.app.phase_status())
}

#[derive(Debug, Serialize)]
pub(crate) struct ConsoleSnapshotResponse {
    principal: Principal,
    principals: Vec<ConsolePrincipalResponse>,
    conversations: Vec<Conversation>,
    messages_by_conversation: BTreeMap<String, Vec<Message>>,
    agents: Vec<ConsolePrincipalResponse>,
    audit_logs: Vec<AuditLog>,
    plugins: Vec<crate::plugins::HostPluginManifest>,
    /// Unread + mention counts per conversation (Mattermost pattern).
    /// Populated from `get_unread_counts`; empty on failure so clients
    /// can fall back to `/v1/unreads`.
    unreads: Vec<choruz_application::ConversationUnread>,
    pinned_conversations: Vec<choruz_application::PinnedConversation>,
    archived_conversations: Vec<choruz_application::ArchivedConversation>,
    hidden_conversations: Vec<choruz_application::HiddenConversation>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ConsolePrincipalResponse {
    id: String,
    workspace_id: String,
    principal_type: PrincipalType,
    name: String,
    avatar_url: Option<String>,
    scopes: Vec<String>,
    disabled: bool,
    channel_visibility: ChannelVisibility,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const DEFAULT_BOOTSTRAP_LIMIT: u32 = 50;
const MAX_BOOTSTRAP_LIMIT: u32 = 100;

#[derive(Debug, Deserialize)]
pub(crate) struct BootstrapQuery {
    limit: Option<u32>,
    after: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BootstrapCursor {
    last_activity_at: DateTime<Utc>,
    conversation_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct BootstrapConversationResponse {
    conversation: Conversation,
    last_message: Option<Message>,
    last_activity_at: DateTime<Utc>,
    unread_count: i64,
    mention_count: i64,
    thread_unread_count: i64,
    pinned_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
    hidden_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct BootstrapConversationPage {
    items: Vec<BootstrapConversationResponse>,
    next_cursor: Option<String>,
    has_more: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct BootstrapResponse {
    principal: Principal,
    principals: Vec<ConsolePrincipalResponse>,
    companies: Vec<Company>,
    agents: Vec<ConsolePrincipalResponse>,
    conversations: BootstrapConversationPage,
    plugins: Vec<crate::plugins::HostPluginManifest>,
    hidden_conversations: Vec<choruz_application::HiddenConversation>,
    /// Every runtime binding the person may see, read under the same
    /// `sync_cursor` as the conversations so a `runtime_binding.*` change
    /// after this snapshot is never older than the snapshot.
    runtime_bindings: Vec<crate::handlers_runtime::RuntimeBindingView>,
    sync_cursor: u64,
    snapshot_at: DateTime<Utc>,
}

const DEFAULT_SYNC_LIMIT: u32 = 100;
const MAX_SYNC_LIMIT: u32 = 500;

#[derive(Debug, Deserialize)]
pub(crate) struct SyncQuery {
    cursor: Option<u64>,
    limit: Option<u32>,
}

/// Gap-fill endpoint for dashboard clients. It is deliberately read-only and
/// non-destructive: every browser/device owns its cursor locally.
pub(crate) async fn sync_changes(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<SyncQuery>,
) -> Result<Json<choruz_application::SyncChangePage>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let page = state
        .db
        .list_sync_changes(
            &principal.id,
            query.cursor.unwrap_or(0),
            query
                .limit
                .unwrap_or(DEFAULT_SYNC_LIMIT)
                .clamp(1, MAX_SYNC_LIMIT),
        )
        .await?;
    Ok(Json(page))
}

fn decode_bootstrap_cursor(value: &str) -> Result<(DateTime<Utc>, String), ApiError> {
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| {
        ApiError(choruz_common::AppError::Validation(
            "invalid bootstrap cursor".into(),
        ))
    })?;
    let cursor: BootstrapCursor = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError(choruz_common::AppError::Validation(
            "invalid bootstrap cursor".into(),
        ))
    })?;
    if cursor.conversation_id.is_empty() {
        return Err(ApiError(choruz_common::AppError::Validation(
            "invalid bootstrap cursor".into(),
        )));
    }
    Ok((cursor.last_activity_at, cursor.conversation_id))
}

fn encode_bootstrap_cursor(entry: &choruz_application::ConversationBootstrapEntry) -> String {
    let cursor = BootstrapCursor {
        last_activity_at: entry.last_activity_at,
        conversation_id: entry.conversation.id.clone(),
    };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).expect("bootstrap cursor serializes"))
}

/// Bounded initial dashboard state. Unlike `/v1/console`, this endpoint never
/// walks every conversation or loads message history per conversation.
/// Bindings for the dashboard snapshot: what `GET /v1/runtime/bindings`
/// returns for a person, nothing for an agent token.
async fn bootstrap_runtime_bindings(
    state: &ApiState,
    principal: &Principal,
) -> Result<Vec<crate::handlers_runtime::RuntimeBindingView>, AppError> {
    if !matches!(principal.principal_type, PrincipalType::Human) {
        return Ok(Vec::new());
    }
    let allowed = crate::handlers_runtime::accessible_workspace_ids(&state.db, &principal.id)
        .await
        .map_err(|error| error.0)?;
    crate::handlers_runtime::list_binding_views(state, &allowed, None)
        .await
        .map_err(|error| error.0)
}

pub(crate) async fn bootstrap(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<BootstrapQuery>,
) -> Result<Json<BootstrapResponse>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    // Capture the feed high-water mark before reading snapshot rows. Changes
    // racing with the snapshot may be replayed twice, but cannot be missed.
    let sync_cursor = state.db.current_sync_cursor(&principal.id).await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_BOOTSTRAP_LIMIT)
        .clamp(1, MAX_BOOTSTRAP_LIMIT);
    let after = query
        .after
        .as_deref()
        .map(decode_bootstrap_cursor)
        .transpose()?;

    let mut entries = state
        .db
        .list_conversation_bootstrap_page(&principal.id, limit + 1, after)
        .await?;
    let has_more = entries.len() > limit as usize;
    if has_more {
        entries.truncate(limit as usize);
    }

    let conversation_ids: Vec<String> = entries
        .iter()
        .map(|entry| entry.conversation.id.clone())
        .collect();
    let member_principal_ids: Vec<String> = entries
        .iter()
        .flat_map(|entry| entry.conversation.members.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let (known_principals, agents, companies, unreads, hidden_conversations, runtime_bindings) = tokio::try_join!(
        state.db.list_principals_by_ids(&member_principal_ids),
        state.db.list_accessible_agents(&principal.id),
        state.db.list_companies(&principal.id),
        state
            .db
            .get_unread_counts_for_conversations(&principal.id, &conversation_ids),
        state.db.list_visible_hidden_conversations(&principal.id),
        bootstrap_runtime_bindings(&state, &principal),
    )?;
    let principals = console_task_assignee_principals(
        &entries
            .iter()
            .map(|entry| entry.conversation.clone())
            .collect::<Vec<_>>(),
        known_principals,
    );
    let mut agents: Vec<_> = agents.into_iter().map(redact_console_principal).collect();
    agents.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    let unread_by_conversation: BTreeMap<_, _> = unreads
        .into_iter()
        .map(|unread| (unread.conversation_id.clone(), unread))
        .collect();
    let encoded_next_cursor = has_more
        .then(|| encode_bootstrap_cursor(entries.last().expect("has_more requires a page item")));

    let items = entries
        .into_iter()
        .map(|entry| {
            let unread = unread_by_conversation.get(&entry.conversation.id);
            BootstrapConversationResponse {
                conversation: entry.conversation,
                last_message: entry.last_message,
                last_activity_at: entry.last_activity_at,
                unread_count: unread.map_or(0, |value| value.unread_count),
                mention_count: unread.map_or(0, |value| value.mention_count),
                thread_unread_count: unread.map_or(0, |value| value.thread_unread_count),
                pinned_at: entry.pinned_at,
                archived_at: entry.archived_at,
                hidden_at: entry.hidden_at,
            }
        })
        .collect();

    Ok(Json(BootstrapResponse {
        principal,
        principals,
        companies,
        agents,
        conversations: BootstrapConversationPage {
            items,
            next_cursor: encoded_next_cursor,
            has_more,
        },
        plugins: crate::plugins::enabled_manifests(),
        hidden_conversations,
        runtime_bindings,
        sync_cursor,
        snapshot_at: Utc::now(),
    }))
}

pub(crate) async fn console_snapshot(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<ConsoleSnapshotResponse>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let mut conversations = state.db.list_conversations(&principal.id).await?;
    conversations.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let member_principal_ids: Vec<String> = conversations
        .iter()
        .flat_map(|conversation| conversation.members.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let principals = state
        .db
        .list_principals_by_ids(&member_principal_ids)
        .await
        .unwrap_or_default();
    let principals = console_task_assignee_principals(&conversations, principals);

    // Only include the last message per conversation for sidebar preview.
    // Full message history is fetched on-demand via /v1/conversations/{id}/messages.
    let mut messages_by_conversation = BTreeMap::new();
    for conversation in &conversations {
        // DB is the source of truth — no in-memory fallback needed.
        let messages = state
            .db
            .list_messages(&conversation.id, None, None)
            .await
            .unwrap_or_default();
        // Keep only the last message for sidebar preview
        let preview: Vec<_> = if let Some(last) = messages.last() {
            vec![last.clone()]
        } else {
            Vec::new()
        };
        messages_by_conversation.insert(conversation.id.clone(), preview);
    }

    // Collect agents from all companies the human belongs to, not just their
    // own workspace.  This ensures agents provisioned under non-default
    // companies (e.g. choruz_cli) appear in the snapshot.
    let mut agents = state
        .db
        .list_workspace_agents(&principal.workspace_id)
        .await
        .unwrap_or_default();
    if let Ok(companies) = state.db.list_companies(&principal.id).await {
        for company in &companies {
            if company.id != principal.workspace_id {
                if let Ok(company_agents) = state.db.list_agents_for_company(&company.id).await {
                    agents.extend(company_agents);
                }
            }
        }
    }
    let mut agents: Vec<_> = agents.into_iter().map(redact_console_principal).collect();
    agents.sort_by(|a, b| a.name.cmp(&b.name));

    // Fetch unread counts so the client can refresh badges every snapshot
    // poll without a second round-trip to /v1/unreads.
    let unreads = state
        .db
        .get_unread_counts(&principal.id)
        .await
        .unwrap_or_default();
    let pinned_conversations = state
        .db
        .list_visible_conversation_pins(&principal.id)
        .await
        .unwrap_or_default();
    let archived_conversations = state
        .db
        .list_visible_conversation_archives(&principal.id)
        .await
        .unwrap_or_default();
    let hidden_conversations = state
        .db
        .list_visible_hidden_conversations(&principal.id)
        .await
        .unwrap_or_default();

    Ok(Json(ConsoleSnapshotResponse {
        principal,
        principals,
        conversations,
        messages_by_conversation,
        agents,
        audit_logs: Vec::new(),
        plugins: crate::plugins::enabled_manifests(),
        unreads,
        pinned_conversations,
        archived_conversations,
        hidden_conversations,
    }))
}

fn console_task_assignee_principals(
    conversations: &[Conversation],
    principals: Vec<Principal>,
) -> Vec<ConsolePrincipalResponse> {
    let mut visible: Vec<_> = principals
        .into_iter()
        .filter(|principal| {
            let is_visible_member = conversations.iter().any(|conversation| {
                conversation.workspace_id == principal.workspace_id
                    && conversation.members.contains_key(&principal.id)
            });
            let is_valid_type = matches!(
                principal.principal_type,
                PrincipalType::Human | PrincipalType::Agent
            );
            let is_visible_agent = principal.principal_type != PrincipalType::Agent
                || principal.channel_visibility != ChannelVisibility::Internal;
            is_visible_member && is_valid_type && is_visible_agent
        })
        .map(redact_console_principal)
        .collect();
    visible.sort_by(|a, b| a.name.cmp(&b.name));
    visible
}

fn redact_console_principal(principal: Principal) -> ConsolePrincipalResponse {
    ConsolePrincipalResponse {
        id: principal.id,
        workspace_id: principal.workspace_id,
        principal_type: principal.principal_type,
        name: principal.name,
        avatar_url: principal.avatar_url,
        scopes: principal.scopes,
        disabled: principal.disabled,
        channel_visibility: principal.channel_visibility,
        created_at: principal.created_at,
        updated_at: principal.updated_at,
    }
}
