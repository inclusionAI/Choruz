//! Fanout Gateway: manages client connections, polls for new events,
//! pushes events to subscribed clients via per-connection sender tasks.
//!
//! Architecture: each WS connection gets its own sender task (tokio::spawn).
//! The fanout loop fetches new events and broadcasts them to all sender tasks
//! via mpsc channels. No lock is held during I/O or sending.
//!
//! # User-scoped subscriptions
//!
//! Connections are keyed by **user_id**, not conversation_id.  A single
//! WebSocket receives events for every conversation the user is a member of
//! (Slack / Matrix / Discord pattern).  Per-conversation cursors live inside
//! `ClientConnection::cursors` so the gateway can still de-dup and replay.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use choruz_store::ConversationEventRow;
use chrono::Utc;
use tokio::sync::{RwLock, mpsc};
use tracing;

use crate::cursor::CursorStore;
use crate::models::FanoutEvent;

const CHANNEL_TASK_STALE_FETCH_WARN_SECONDS: i64 = 5;

// ---------------------------------------------------------------------------
// Fanout error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum FanoutError {
    #[error("store error: {0}")]
    Store(#[from] choruz_common::AppError),

    #[error("client disconnected: {0}")]
    ClientDisconnected(String),

    #[error("event source error: {0}")]
    EventSource(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type FanoutResult<T> = Result<T, FanoutError>;

// ---------------------------------------------------------------------------
// EventSource trait
// ---------------------------------------------------------------------------

pub trait EventSource: Send + Sync + 'static {
    fn fetch_events_after(
        &self,
        conversation_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> impl std::future::Future<Output = FanoutResult<Vec<ConversationEventRow>>> + Send;

    /// Return the list of active (non-removed) member principal_ids for a
    /// conversation.  Used by the fanout loop to route events to the
    /// currently-connected members.
    fn get_conversation_members(
        &self,
        conversation_id: &str,
    ) -> impl std::future::Future<Output = FanoutResult<Vec<String>>> + Send;

    /// Return the set of conversation_ids a user is currently an active
    /// member of.  Used at subscribe-time to seed the user's per-conversation
    /// cursor map so the fanout loop knows which conversations to poll.
    fn get_user_conversations(
        &self,
        user_id: &str,
    ) -> impl std::future::Future<Output = FanoutResult<Vec<String>>> + Send;
}

// ---------------------------------------------------------------------------
// InMemoryEventSource — for testing
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct InMemoryEventSource {
    pub events: Arc<RwLock<Vec<ConversationEventRow>>>,
    /// conversation_id -> list of member user_ids (for testing fanout routing)
    pub members: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl EventSource for InMemoryEventSource {
    async fn fetch_events_after(
        &self,
        conversation_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> FanoutResult<Vec<ConversationEventRow>> {
        let events = self.events.read().await;
        let result: Vec<_> = events
            .iter()
            .filter(|e| e.conversation_id == conversation_id && e.seq > after_seq)
            .take(limit as usize)
            .cloned()
            .collect();
        Ok(result)
    }

    async fn get_conversation_members(&self, conversation_id: &str) -> FanoutResult<Vec<String>> {
        let members = self.members.read().await;
        Ok(members.get(conversation_id).cloned().unwrap_or_default())
    }

    async fn get_user_conversations(&self, user_id: &str) -> FanoutResult<Vec<String>> {
        let members = self.members.read().await;
        let mut out = Vec::new();
        for (conv_id, member_ids) in members.iter() {
            if member_ids.iter().any(|m| m == user_id) {
                out.push(conv_id.clone());
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// ClientConnection — lightweight handle, actual sending is in a spawned task
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ClientConnection {
    pub user_id: String,
    pub client_id: String,
    /// Per-conversation last-pushed seq for this connection (for de-dup).
    pub cursors: HashMap<String, i64>,
    tx: mpsc::Sender<FanoutEvent>,
}

impl ClientConnection {
    pub fn new(
        client_id: impl Into<String>,
        user_id: impl Into<String>,
        buffer_size: usize,
    ) -> (Self, mpsc::Receiver<FanoutEvent>) {
        let (tx, rx) = mpsc::channel(buffer_size);
        let conn = Self {
            user_id: user_id.into(),
            client_id: client_id.into(),
            cursors: HashMap::new(),
            tx,
        };
        (conn, rx)
    }

    /// Non-blocking push. Uses try_send so we never await inside a lock.
    /// Returns Ok even if buffer is full (event is dropped for this client).
    /// Returns Err only if the client disconnected.
    pub fn push(&mut self, event: FanoutEvent) -> FanoutResult<()> {
        match self.tx.try_send(event) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(event)) => {
                // Buffer full — client is slow. Drop this event for this client.
                // The client will catch up on reconnect via cursor replay.
                tracing::debug!(
                    client_id = %self.client_id,
                    seq = event.seq,
                    "client buffer full, event dropped (will replay on reconnect)"
                );
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(FanoutError::ClientDisconnected(self.client_id.clone()))
            }
        }
    }

    /// Return the last-pushed seq for a conversation (defaults to 0).
    pub fn get_cursor(&self, conversation_id: &str) -> i64 {
        self.cursors.get(conversation_id).copied().unwrap_or(0)
    }

    /// Update last-pushed seq for a conversation.
    pub fn advance_cursor(&mut self, conversation_id: &str, seq: i64) {
        let entry = self.cursors.entry(conversation_id.to_string()).or_insert(0);
        if seq > *entry {
            *entry = seq;
        }
    }

    pub fn is_connected(&self) -> bool {
        !self.tx.is_closed()
    }
}

// ---------------------------------------------------------------------------
// FanoutGateway
// ---------------------------------------------------------------------------

/// User-scoped fanout gateway.
///
/// `connections` maps **user_id** to the list of `ClientConnection`s for that
/// user (one user may have multiple tabs/devices open simultaneously).
pub struct FanoutGateway<E: EventSource, C: CursorStore> {
    event_source: E,
    cursor_store: C,
    connections: Arc<RwLock<HashMap<String, Vec<ClientConnection>>>>,
}

impl<E: EventSource, C: CursorStore> FanoutGateway<E, C> {
    pub fn new(event_source: E, cursor_store: C) -> Self {
        Self {
            event_source,
            cursor_store,
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // -----------------------------------------------------------------------
    // Connection management
    // -----------------------------------------------------------------------

    pub async fn subscribe(
        &self,
        client_id: &str,
        user_id: &str,
        buffer_size: usize,
    ) -> mpsc::Receiver<FanoutEvent> {
        let (conn, rx) = ClientConnection::new(client_id, user_id, buffer_size);

        let mut connections = self.connections.write().await;
        connections
            .entry(user_id.to_string())
            .or_default()
            .push(conn);

        tracing::info!(client_id, user_id, "client subscribed");

        rx
    }

    /// Subscribe a user and seed their per-conversation cursors from the
    /// event source's membership table + persisted cursor store.  This is
    /// what the WS handler calls on connect: after this returns, the fanout
    /// loop will poll every conversation the user belongs to.
    pub async fn subscribe_user(
        &self,
        client_id: &str,
        user_id: &str,
        buffer_size: usize,
    ) -> FanoutResult<mpsc::Receiver<FanoutEvent>> {
        let rx = self.subscribe(client_id, user_id, buffer_size).await;

        // Look up all conversations this user belongs to.
        let convs = self
            .event_source
            .get_user_conversations(user_id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(user_id, error = %e, "failed to load user conversations; starting empty");
                Vec::new()
            });

        // Pre-fetch persisted cursors for each (client, conv) pair BEFORE
        // taking the connections write lock (cursor_store is async).
        let mut seeded: Vec<(String, i64)> = Vec::with_capacity(convs.len());
        for conv_id in &convs {
            let seq = self
                .cursor_store
                .get_cursor(client_id, conv_id)
                .await
                .map(|c| c.last_seen_seq)
                .unwrap_or(0);
            seeded.push((conv_id.clone(), seq));
        }

        // Now write the seeded cursors into the in-memory connection.
        {
            let mut connections = self.connections.write().await;
            if let Some(conns) = connections.get_mut(user_id) {
                for conn in conns.iter_mut().filter(|c| c.client_id == client_id) {
                    for (conv_id, seq) in &seeded {
                        conn.cursors.entry(conv_id.clone()).or_insert(*seq);
                    }
                }
            }
        }

        tracing::info!(
            client_id,
            user_id,
            conversations = convs.len(),
            "user subscribed with seeded cursors"
        );

        Ok(rx)
    }

    pub async fn unsubscribe(&self, client_id: &str, user_id: &str) {
        let mut connections = self.connections.write().await;
        if let Some(conns) = connections.get_mut(user_id) {
            conns.retain(|c| c.client_id != client_id);
            if conns.is_empty() {
                connections.remove(user_id);
            }
        }

        tracing::info!(client_id, user_id, "client unsubscribed");
    }

    /// Number of connections currently open for a given user (multiple tabs/devices).
    pub async fn connection_count(&self, user_id: &str) -> usize {
        let connections = self.connections.read().await;
        connections.get(user_id).map_or(0, Vec::len)
    }

    pub async fn total_connections(&self) -> usize {
        let connections = self.connections.read().await;
        connections.values().map(Vec::len).sum()
    }

    // -----------------------------------------------------------------------
    // Fanout: fetch events, then push WITHOUT holding lock across I/O
    // -----------------------------------------------------------------------

    pub async fn fanout_once(&self) -> FanoutResult<usize> {
        // Phase 1: GC dead connections (short write lock, no await).
        {
            let mut connections = self.connections.write().await;
            for conns in connections.values_mut() {
                conns.retain(|c| c.is_connected());
            }
            connections.retain(|_, conns| !conns.is_empty());
        }

        // Phase 1b: admit any conversations this user joined (or was added
        // to) after `subscribe_user` ran. The WS is user-scoped and seeds
        // cursors only once at connect-time, so without this step a client
        // that opens a new group after connecting never sees events from
        // that group until the page reloads. Re-query membership on every
        // tick and or_insert cursor=0 for any conv that isn't already
        // tracked. The per-user SELECT is indexed and runs every 2s, which
        // stays cheap even with a few hundred online users; if that ever
        // becomes a problem, flip this to a PG LISTEN-driven updater.
        let online_users: Vec<String> = {
            let connections = self.connections.read().await;
            connections.keys().cloned().collect()
        };
        let mut membership_refresh: Vec<(String, Vec<String>)> =
            Vec::with_capacity(online_users.len());
        for user_id in &online_users {
            match self.event_source.get_user_conversations(user_id).await {
                Ok(convs) => membership_refresh.push((user_id.clone(), convs)),
                Err(e) => {
                    tracing::warn!(user_id, error = %e, "fanout: membership refresh failed");
                }
            }
        }
        if !membership_refresh.is_empty() {
            let mut connections = self.connections.write().await;
            for (user_id, convs) in &membership_refresh {
                if let Some(conns) = connections.get_mut(user_id) {
                    for conn in conns.iter_mut() {
                        for conv_id in convs {
                            conn.cursors.entry(conv_id.clone()).or_insert(0);
                        }
                    }
                }
            }
        }

        // Phase 2: build the fetch plan in one read-lock pass — collect the
        // union of conversations across every connected user + the per-conv
        // global min cursor (so we fetch once for all connected members).
        //
        // Each user's `cursors` HashMap is the authoritative set of
        // "conversations this connection cares about right now" (seeded at
        // subscribe-time from the member table + persisted cursors,
        // refreshed in Phase 1b above).
        let fetch_plan: Vec<(String, i64)> = {
            let connections = self.connections.read().await;
            let mut per_conv: HashMap<String, i64> = HashMap::new();
            for conns in connections.values() {
                for conn in conns {
                    for (conv_id, &seq) in conn.cursors.iter() {
                        per_conv
                            .entry(conv_id.clone())
                            .and_modify(|m| {
                                if seq < *m {
                                    *m = seq;
                                }
                            })
                            .or_insert(seq);
                    }
                }
            }
            per_conv.into_iter().collect()
        };

        if fetch_plan.is_empty() {
            return Ok(0);
        }

        // Phase 4: fetch events per conversation (NO lock held)
        let mut fetched: Vec<(String, Vec<String>, Vec<ConversationEventRow>)> = Vec::new();
        for (conv_id, min_seq) in &fetch_plan {
            let events = self
                .event_source
                .fetch_events_after(conv_id, *min_seq, 50)
                .await?;
            if events.is_empty() {
                continue;
            }
            let members = self.event_source.get_conversation_members(conv_id).await?;
            fetched.push((conv_id.clone(), members, events));
        }

        if fetched.is_empty() {
            return Ok(0);
        }
        log_channel_task_stale_fetch_candidates(&fetched);

        // Phase 5: push events (short write lock, NO await inside)
        let mut total_pushed = 0;
        let mut dead: Vec<(String /* user_id */, String /* client_id */)> = Vec::new();
        {
            let mut connections = self.connections.write().await;

            for (conv_id, members, events) in &fetched {
                for member_id in members {
                    let conns = match connections.get_mut(member_id) {
                        Some(c) => c,
                        None => continue, // member not connected
                    };

                    for conn in conns.iter_mut() {
                        let current = conn.get_cursor(conv_id);
                        for event in events {
                            if event.seq <= current {
                                continue;
                            }
                            // Pull the FE trace_id out of the event metadata
                            // so fanout failures can be stitched back to the
                            // originating user action — otherwise a broken
                            // reply-delivery stage looks like silent loss
                            // in `trace_id`-grepped logs.
                            let ev_trace_id: String = event
                                .metadata
                                .get("trace_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("none")
                                .to_string();
                            let fanout_event = event_row_to_fanout(event);
                            let seq = fanout_event.seq;
                            match conn.push(fanout_event) {
                                Ok(()) => {
                                    conn.advance_cursor(conv_id, seq);
                                    total_pushed += 1;
                                }
                                Err(FanoutError::ClientDisconnected(ref cid)) => {
                                    tracing::warn!(
                                        event = "fanout_client_disconnected",
                                        trace_id = %ev_trace_id,
                                        client_id = %cid,
                                        conversation_id = %conv_id,
                                        member_id = %member_id,
                                        event_id = %event.event_id,
                                        seq,
                                        "client disconnected during push"
                                    );
                                    dead.push((member_id.clone(), cid.clone()));
                                    break;
                                }
                                Err(e) => {
                                    tracing::error!(
                                        event = "fanout_push_failed",
                                        trace_id = %ev_trace_id,
                                        conversation_id = %conv_id,
                                        member_id = %member_id,
                                        event_id = %event.event_id,
                                        seq,
                                        error = %e,
                                        "fanout push error"
                                    );
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // Remove dead connections (still inside write lock)
            for (user_id, client_id) in &dead {
                if let Some(conns) = connections.get_mut(user_id) {
                    conns.retain(|c| c.client_id != *client_id);
                    if conns.is_empty() {
                        connections.remove(user_id);
                    }
                }
                tracing::info!(client_id, user_id, "removed dead connection");
            }
        } // lock dropped

        // Phase 6: persist cursors (NO lock held across await; snapshot then upsert)
        let cursor_snapshot: Vec<(String, String, i64)> = {
            let connections = self.connections.read().await;
            let mut out = Vec::new();
            for (conv_id, _, _) in &fetched {
                for conns in connections.values() {
                    for conn in conns {
                        if let Some(&seq) = conn.cursors.get(conv_id) {
                            out.push((conn.client_id.clone(), conv_id.clone(), seq));
                        }
                    }
                }
            }
            out
        };
        for (client_id, conv_id, seq) in cursor_snapshot {
            self.cursor_store
                .upsert_cursor(&client_id, &conv_id, seq)
                .await;
        }

        Ok(total_pushed)
    }

    pub async fn run_fanout_loop(&self, interval: Duration) {
        tracing::info!("Fanout loop started");
        let mut tick = tokio::time::interval(interval);

        loop {
            tick.tick().await;

            match self.fanout_once().await {
                Ok(0) => {}
                Ok(n) => {
                    tracing::debug!(events_pushed = n, "fanout cycle completed");
                }
                Err(e) => {
                    tracing::error!(error = %e, "fanout cycle error");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Cursor replay
    // -----------------------------------------------------------------------

    /// Persist a single (client, conversation) cursor.
    pub async fn set_client_cursor(
        &self,
        client_id: &str,
        conversation_id: &str,
        last_seen_seq: i64,
    ) {
        self.cursor_store
            .upsert_cursor(client_id, conversation_id, last_seen_seq)
            .await;
    }

    /// Seed a connection's in-memory cursor (so the fanout loop knows which
    /// conversations this user cares about).  Called during WS upgrade after
    /// we've looked up the user's conversation membership.
    pub async fn seed_cursor(
        &self,
        client_id: &str,
        user_id: &str,
        conversation_id: &str,
        last_seen_seq: i64,
    ) {
        let mut connections = self.connections.write().await;
        if let Some(conns) = connections.get_mut(user_id) {
            for conn in conns.iter_mut() {
                if conn.client_id == client_id {
                    conn.advance_cursor(conversation_id, last_seen_seq);
                }
            }
        }
    }

    /// Replay events for a single (client, conversation) pair.  Used on
    /// reconnect when the client supplies a cursor map.
    pub async fn replay_for_client(
        &self,
        client_id: &str,
        user_id: &str,
        conversation_id: &str,
        max_replay: i64,
    ) -> FanoutResult<usize> {
        let members = self
            .event_source
            .get_conversation_members(conversation_id)
            .await?;
        if !members.iter().any(|member_id| member_id == user_id) {
            tracing::warn!(
                client_id,
                user_id,
                conversation_id,
                "replay denied for non-member"
            );
            return Ok(0);
        }

        let cursor_seq = self
            .cursor_store
            .get_cursor(client_id, conversation_id)
            .await
            .map(|c| c.last_seen_seq)
            .unwrap_or(0);

        let missed = self
            .event_source
            .fetch_events_after(conversation_id, cursor_seq, max_replay)
            .await?;

        if missed.is_empty() {
            return Ok(0);
        }

        // Push replay events (short lock, no await inside)
        let mut replayed = 0;
        let mut final_seq = cursor_seq;
        {
            let mut connections = self.connections.write().await;
            let conns = match connections.get_mut(user_id) {
                Some(c) => c,
                None => return Ok(0),
            };

            for conn in conns.iter_mut().filter(|c| c.client_id == client_id) {
                let current = conn.get_cursor(conversation_id);
                for event in &missed {
                    if event.seq <= current {
                        continue;
                    }
                    let fanout_event = event_row_to_fanout(event);
                    let seq = fanout_event.seq;
                    match conn.push(fanout_event) {
                        Ok(()) => {
                            conn.advance_cursor(conversation_id, seq);
                            replayed += 1;
                            final_seq = final_seq.max(seq);
                        }
                        Err(_) => break,
                    }
                }
            }
        } // lock dropped

        // Update cursor after replay (no lock)
        if replayed > 0 {
            self.cursor_store
                .upsert_cursor(client_id, conversation_id, final_seq)
                .await;
        }

        tracing::info!(
            client_id,
            user_id,
            conversation_id,
            replayed,
            "replay completed"
        );
        Ok(replayed)
    }
}

// ---------------------------------------------------------------------------
// Conversion helper
// ---------------------------------------------------------------------------

fn event_row_to_fanout(row: &ConversationEventRow) -> FanoutEvent {
    FanoutEvent {
        conversation_id: row.conversation_id.clone(),
        seq: row.seq,
        event_id: row.event_id.clone(),
        event_type: row.event_type.clone(),
        sender_id: row.sender_id.clone(),
        content: row.content.clone(),
        content_type: row.content_type.clone(),
        metadata: row.metadata.clone(),
        client_msg_id: row.client_msg_id.clone(),
        created_at: row.created_at,
    }
}

fn log_channel_task_stale_fetch_candidates(
    fetched: &[(String, Vec<String>, Vec<ConversationEventRow>)],
) {
    let now = Utc::now();
    for (conversation_id, _, events) in fetched {
        if let Some(summary) = channel_task_stale_fetch_summary(events, now) {
            tracing::warn!(
                event = "channel_task_stale_fetch_candidate",
                conversation_id,
                channel_task_events = summary.count,
                oldest_age_seconds = summary.oldest_age_seconds,
                min_seq = summary.min_seq,
                max_seq = summary.max_seq,
                "fetched channel task events are older than expected; may indicate replay, backfill, or live delivery lag"
            );
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ChannelTaskStaleFetchSummary {
    count: usize,
    oldest_age_seconds: i64,
    min_seq: i64,
    max_seq: i64,
}

fn channel_task_stale_fetch_summary(
    events: &[ConversationEventRow],
    now: chrono::DateTime<Utc>,
) -> Option<ChannelTaskStaleFetchSummary> {
    let mut count = 0usize;
    let mut oldest_age_seconds = 0i64;
    let mut min_seq = i64::MAX;
    let mut max_seq = 0i64;
    for event in events {
        if !matches!(
            event.event_type.as_str(),
            "channel_task.created" | "channel_task.updated"
        ) {
            continue;
        }
        count += 1;
        oldest_age_seconds =
            oldest_age_seconds.max(now.signed_duration_since(event.created_at).num_seconds());
        min_seq = min_seq.min(event.seq);
        max_seq = max_seq.max(event.seq);
    }
    (count > 0 && oldest_age_seconds >= CHANNEL_TASK_STALE_FETCH_WARN_SECONDS).then_some(
        ChannelTaskStaleFetchSummary {
            count,
            oldest_age_seconds,
            min_seq,
            max_seq,
        },
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::InMemoryCursorStore;
    use chrono::{Duration, Utc};

    fn make_event_row(conv_id: &str, seq: i64, content: &str) -> ConversationEventRow {
        ConversationEventRow {
            conversation_id: conv_id.into(),
            seq,
            event_id: format!("evt-{seq}"),
            event_type: "message".into(),
            sender_id: "user-1".into(),
            content: Some(content.into()),
            content_type: "text/plain".into(),
            metadata: serde_json::json!({}),
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            created_at: Utc::now(),
        }
    }

    fn make_typed_event_row(
        conv_id: &str,
        seq: i64,
        event_type: &str,
        now: chrono::DateTime<Utc>,
        age_seconds: i64,
    ) -> ConversationEventRow {
        ConversationEventRow {
            event_type: event_type.into(),
            created_at: now - Duration::seconds(age_seconds),
            ..make_event_row(conv_id, seq, "{}")
        }
    }

    async fn make_gateway(
        events: Vec<ConversationEventRow>,
        members: HashMap<String, Vec<String>>,
    ) -> FanoutGateway<InMemoryEventSource, InMemoryCursorStore> {
        let source = InMemoryEventSource {
            events: Arc::new(RwLock::new(events)),
            members: Arc::new(RwLock::new(members)),
        };
        let cursors = InMemoryCursorStore::new();
        FanoutGateway::new(source, cursors)
    }

    #[tokio::test]
    async fn subscribe_and_unsubscribe() {
        let gw = make_gateway(vec![], HashMap::new()).await;
        let _rx = gw.subscribe("c1", "user-1", 10).await;
        assert_eq!(gw.connection_count("user-1").await, 1);
        gw.unsubscribe("c1", "user-1").await;
        assert_eq!(gw.connection_count("user-1").await, 0);
    }

    #[test]
    fn channel_task_stale_fetch_summary_filters_and_thresholds() {
        let now = Utc::now();
        let events = vec![
            make_typed_event_row("conv-1", 1, "message", now, 30),
            make_typed_event_row("conv-1", 2, "channel_task.created", now, 3),
        ];
        assert_eq!(channel_task_stale_fetch_summary(&events, now), None);

        let events = vec![
            make_typed_event_row("conv-1", 1, "message", now, 30),
            make_typed_event_row("conv-1", 2, "channel_task.created", now, 6),
            make_typed_event_row("conv-1", 4, "channel_task.updated", now, 8),
        ];
        assert_eq!(
            channel_task_stale_fetch_summary(&events, now),
            Some(ChannelTaskStaleFetchSummary {
                count: 2,
                oldest_age_seconds: 8,
                min_seq: 2,
                max_seq: 4,
            })
        );
    }

    #[tokio::test]
    async fn fanout_pushes_new_events_to_member() {
        let events = vec![
            make_event_row("conv-1", 1, "hello"),
            make_event_row("conv-1", 2, "world"),
        ];
        let mut members = HashMap::new();
        members.insert("conv-1".to_string(), vec!["user-1".to_string()]);
        let gw = make_gateway(events, members).await;
        let mut rx = gw.subscribe("c1", "user-1", 10).await;

        // Seed the connection so the gateway knows this user cares about conv-1.
        gw.seed_cursor("c1", "user-1", "conv-1", 0).await;

        let pushed = gw.fanout_once().await.unwrap();
        assert_eq!(pushed, 2);

        let e1 = rx.recv().await.unwrap();
        assert_eq!(e1.seq, 1);
        let e2 = rx.recv().await.unwrap();
        assert_eq!(e2.seq, 2);
    }

    #[tokio::test]
    async fn fanout_routes_to_all_members() {
        let events = vec![make_event_row("conv-1", 1, "broadcast")];
        let mut members = HashMap::new();
        members.insert(
            "conv-1".to_string(),
            vec!["user-a".to_string(), "user-b".to_string()],
        );
        let gw = make_gateway(events, members).await;

        let mut rx1 = gw.subscribe("c1", "user-a", 10).await;
        let mut rx2 = gw.subscribe("c2", "user-b", 10).await;
        gw.seed_cursor("c1", "user-a", "conv-1", 0).await;
        gw.seed_cursor("c2", "user-b", "conv-1", 0).await;

        let pushed = gw.fanout_once().await.unwrap();
        assert_eq!(pushed, 2);

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert_eq!(e1.seq, 1);
        assert_eq!(e2.seq, 1);
    }

    #[tokio::test]
    async fn fanout_skips_non_members() {
        let events = vec![make_event_row("conv-1", 1, "secret")];
        let mut members = HashMap::new();
        members.insert("conv-1".to_string(), vec!["user-a".to_string()]);
        let gw = make_gateway(events, members).await;

        let mut rx_in = gw.subscribe("c1", "user-a", 10).await;
        let mut rx_out = gw.subscribe("c2", "user-b", 10).await;
        gw.seed_cursor("c1", "user-a", "conv-1", 0).await;
        gw.seed_cursor("c2", "user-b", "conv-1", 0).await; // user-b pretends to care

        let pushed = gw.fanout_once().await.unwrap();
        assert_eq!(pushed, 1);

        let e = rx_in.recv().await.unwrap();
        assert_eq!(e.seq, 1);

        // user-b is NOT in conv-1 members, must not receive
        assert!(rx_out.try_recv().is_err());
    }

    #[tokio::test]
    async fn fanout_stops_sending_to_removed_member() {
        let events = vec![make_event_row("conv-1", 1, "before removal")];
        let mut members = HashMap::new();
        members.insert(
            "conv-1".to_string(),
            vec!["user-a".to_string(), "user-b".to_string()],
        );
        let gw = make_gateway(events, members).await;

        let mut rx_a = gw.subscribe("c-a", "user-a", 10).await;
        let mut rx_b = gw.subscribe("c-b", "user-b", 10).await;
        gw.seed_cursor("c-a", "user-a", "conv-1", 0).await;
        gw.seed_cursor("c-b", "user-b", "conv-1", 0).await;

        assert_eq!(gw.fanout_once().await.unwrap(), 2);
        assert_eq!(rx_a.recv().await.unwrap().seq, 1);
        assert_eq!(rx_b.recv().await.unwrap().seq, 1);

        {
            let source = &gw.event_source;
            source
                .members
                .write()
                .await
                .insert("conv-1".to_string(), vec!["user-a".to_string()]);
            source
                .events
                .write()
                .await
                .push(make_event_row("conv-1", 2, "after removal"));
        }

        assert_eq!(gw.fanout_once().await.unwrap(), 1);
        assert_eq!(rx_a.recv().await.unwrap().seq, 2);
        assert!(rx_b.try_recv().is_err());
    }

    #[tokio::test]
    async fn replay_denies_removed_member_with_stale_cursor() {
        let events = vec![
            make_event_row("conv-1", 1, "before removal"),
            make_event_row("conv-1", 2, "after removal"),
        ];
        let mut members = HashMap::new();
        members.insert("conv-1".to_string(), vec!["user-a".to_string()]);
        let gw = make_gateway(events, members).await;
        gw.cursor_store.upsert_cursor("c-b", "conv-1", 1).await;

        let mut rx_b = gw.subscribe("c-b", "user-b", 10).await;
        gw.seed_cursor("c-b", "user-b", "conv-1", 1).await;

        let replayed = gw
            .replay_for_client("c-b", "user-b", "conv-1", 100)
            .await
            .unwrap();
        assert_eq!(replayed, 0);
        assert!(rx_b.try_recv().is_err());
    }

    #[tokio::test]
    async fn user_in_multiple_conversations_receives_all() {
        let events = vec![
            make_event_row("conv-1", 1, "msg in conv 1"),
            make_event_row("conv-2", 7, "msg in conv 2"),
        ];
        let mut members = HashMap::new();
        members.insert("conv-1".to_string(), vec!["user-a".to_string()]);
        members.insert("conv-2".to_string(), vec!["user-a".to_string()]);
        let gw = make_gateway(events, members).await;

        let mut rx = gw.subscribe("c1", "user-a", 10).await;
        gw.seed_cursor("c1", "user-a", "conv-1", 0).await;
        gw.seed_cursor("c1", "user-a", "conv-2", 0).await;

        let pushed = gw.fanout_once().await.unwrap();
        assert_eq!(pushed, 2);

        let mut got = Vec::new();
        for _ in 0..2 {
            got.push(rx.recv().await.unwrap());
        }
        got.sort_by_key(|e| e.seq);
        assert_eq!(got[0].conversation_id, "conv-1");
        assert_eq!(got[1].conversation_id, "conv-2");
    }

    #[tokio::test]
    async fn fanout_persists_direct_and_group_cursors() {
        let events = vec![
            make_event_row("direct-conversation-1", 1, "direct message"),
            make_event_row("group-conversation-1", 1, "group message"),
        ];
        let mut members = HashMap::new();
        members.insert(
            "direct-conversation-1".to_string(),
            vec!["user-a".to_string()],
        );
        members.insert(
            "group-conversation-1".to_string(),
            vec!["user-a".to_string(), "user-b".to_string()],
        );
        let gw = make_gateway(events, members).await;
        let mut user_a = gw.subscribe("client-a", "user-a", 10).await;
        let mut user_b = gw.subscribe("client-b", "user-b", 10).await;
        for conversation_id in ["direct-conversation-1", "group-conversation-1"] {
            gw.seed_cursor("client-a", "user-a", conversation_id, 0)
                .await;
        }
        gw.seed_cursor("client-b", "user-b", "group-conversation-1", 0)
            .await;

        assert_eq!(gw.fanout_once().await.unwrap(), 3);
        let mut user_a_conversations = vec![
            user_a.recv().await.unwrap().conversation_id,
            user_a.recv().await.unwrap().conversation_id,
        ];
        user_a_conversations.sort();
        assert_eq!(
            user_a_conversations,
            ["direct-conversation-1", "group-conversation-1"]
        );
        assert_eq!(
            user_b.recv().await.unwrap().conversation_id,
            "group-conversation-1"
        );

        assert_eq!(
            gw.cursor_store
                .get_cursor("client-a", "direct-conversation-1")
                .await
                .expect("persisted direct cursor")
                .last_seen_seq,
            1
        );
        assert_eq!(
            gw.cursor_store
                .get_cursor("client-a", "group-conversation-1")
                .await
                .expect("persisted group cursor for direct member")
                .last_seen_seq,
            1
        );
        assert_eq!(
            gw.cursor_store
                .get_cursor("client-b", "group-conversation-1")
                .await
                .expect("persisted group cursor for group member")
                .last_seen_seq,
            1
        );
    }

    #[tokio::test]
    async fn slow_client_does_not_block_others() {
        let events = vec![make_event_row("conv-1", 1, "test")];
        let mut members = HashMap::new();
        members.insert(
            "conv-1".to_string(),
            vec!["slow".to_string(), "fast".to_string()],
        );
        let gw = make_gateway(events, members).await;

        let _rx_slow = gw.subscribe("c-slow", "slow", 1).await;
        let mut rx_fast = gw.subscribe("c-fast", "fast", 10).await;
        gw.seed_cursor("c-slow", "slow", "conv-1", 0).await;
        gw.seed_cursor("c-fast", "fast", "conv-1", 0).await;

        gw.fanout_once().await.unwrap();

        {
            let source = &gw.event_source;
            let mut events = source.events.write().await;
            events.push(make_event_row("conv-1", 2, "second"));
        }

        let pushed = gw.fanout_once().await.unwrap();
        assert!(pushed >= 1);

        let e = rx_fast.recv().await.unwrap();
        assert_eq!(e.seq, 1);
    }

    #[tokio::test]
    async fn cursor_replay_on_reconnect() {
        let events = vec![
            make_event_row("conv-1", 1, "first"),
            make_event_row("conv-1", 2, "second"),
            make_event_row("conv-1", 3, "third"),
        ];

        let mut members = HashMap::new();
        members.insert("conv-1".to_string(), vec!["user-a".to_string()]);
        let source = InMemoryEventSource {
            events: Arc::new(RwLock::new(events)),
            members: Arc::new(RwLock::new(members)),
        };
        let cursors = InMemoryCursorStore::new();
        cursors.upsert_cursor("c1", "conv-1", 1).await;

        let gw = FanoutGateway::new(source, cursors);
        let mut rx = gw.subscribe("c1", "user-a", 10).await;

        let replayed = gw
            .replay_for_client("c1", "user-a", "conv-1", 100)
            .await
            .unwrap();
        assert_eq!(replayed, 2);

        let e1 = rx.recv().await.unwrap();
        assert_eq!(e1.seq, 2);
        let e2 = rx.recv().await.unwrap();
        assert_eq!(e2.seq, 3);
    }

    #[tokio::test]
    async fn cursor_replay_preserves_channel_task_event_payload_in_seq_order() {
        // Channel task fanout events share the same transport as message events
        // but carry a typed event_type, a channel-task content_type, and a
        // structured metadata payload. This test pins down that replay preserves
        // those fields verbatim and delivers them in monotonic seq order.
        let now = Utc::now();
        let make_task_row =
            |seq: i64, event_type: &str, task_id: &str, version: i64| ConversationEventRow {
                conversation_id: "conv-task".into(),
                seq,
                event_id: format!("task-evt-{seq}"),
                event_type: event_type.into(),
                sender_id: "user-author".into(),
                content: None,
                content_type: "application/vnd.choruz.channel-task+json".into(),
                metadata: serde_json::json!({
                    "event_type": event_type,
                    "conversation_id": "conv-task",
                    "task_id": task_id,
                    "version": version,
                    "updated_at": now.to_rfc3339(),
                    "task": {
                        "task_id": task_id,
                        "conversation_id": "conv-task",
                        "task_key": "TASK-1",
                        "title": "Replay-shape task",
                        "status": if version == 1 { "todo" } else { "in_progress" },
                        "source_kind": "agent",
                        "version": version,
                        "created_at": now.to_rfc3339(),
                        "updated_at": now.to_rfc3339(),
                    }
                }),
                client_msg_id: None,
                turn_id: None,
                reply_event_id: None,
                created_at: now,
            };

        let events = vec![
            make_event_row("conv-task", 1, "ignored-message"),
            make_task_row(2, "channel_task.created", "task-1", 1),
            make_task_row(3, "channel_task.updated", "task-1", 2),
        ];

        let mut members = HashMap::new();
        members.insert("conv-task".to_string(), vec!["user-a".to_string()]);
        let source = InMemoryEventSource {
            events: Arc::new(RwLock::new(events)),
            members: Arc::new(RwLock::new(members)),
        };
        let cursors = InMemoryCursorStore::new();
        // Cursor at seq=1 means the client has already seen the leading message
        // and must replay both channel task envelopes in order.
        cursors.upsert_cursor("c-replay", "conv-task", 1).await;

        let gw = FanoutGateway::new(source, cursors);
        let mut rx = gw.subscribe("c-replay", "user-a", 10).await;

        let replayed = gw
            .replay_for_client("c-replay", "user-a", "conv-task", 100)
            .await
            .unwrap();
        assert_eq!(replayed, 2);

        let created = rx.recv().await.unwrap();
        assert_eq!(created.seq, 2);
        assert_eq!(created.event_type, "channel_task.created");
        assert_eq!(
            created.content_type,
            "application/vnd.choruz.channel-task+json"
        );
        assert_eq!(created.content, None);
        assert_eq!(created.metadata["task_id"], "task-1");
        assert_eq!(created.metadata["version"], 1);
        assert_eq!(created.metadata["task"]["status"], "todo");

        let updated = rx.recv().await.unwrap();
        assert_eq!(updated.seq, 3);
        assert_eq!(updated.event_type, "channel_task.updated");
        assert_eq!(
            updated.content_type,
            "application/vnd.choruz.channel-task+json"
        );
        assert_eq!(updated.metadata["task_id"], "task-1");
        assert_eq!(updated.metadata["version"], 2);
        assert_eq!(updated.metadata["task"]["status"], "in_progress");

        // Cursor must have advanced past the replayed events so a second replay
        // is a no-op.
        let second_replay = gw
            .replay_for_client("c-replay", "user-a", "conv-task", 100)
            .await
            .unwrap();
        assert_eq!(second_replay, 0);
        assert!(rx.try_recv().is_err());
    }
}
