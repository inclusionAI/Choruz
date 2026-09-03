use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use choruz_domain::{
    AuditLog, Company, CompanyMember, Conversation, ConversationType, EventEnvelope, Principal,
    PrincipalType,
};
use chrono::DateTime;

use crate::{ChatApp, EventWebhookConfig, direct_key};

#[derive(Debug, Clone, Default)]
pub(crate) struct State {
    pub(crate) principals: HashMap<String, Principal>,
    pub(crate) conversations: HashMap<String, Conversation>,
    pub(crate) direct_index: HashMap<(String, String, String), String>,
    pub(crate) audit_logs: Vec<AuditLog>,
    pub(crate) events: HashMap<String, Vec<EventEnvelope>>,
    pub(crate) ack_cursor: HashMap<String, u64>,
    pub(crate) event_webhooks: HashMap<String, EventWebhookConfig>,
    pub(crate) next_event_seq: HashMap<String, u64>,
    /// Sequence counter for the system `message.created` announcements emitted on
    /// membership changes. Bounded by the conversation count and cleared with the
    /// conversation. It is no longer derived from stored messages at boot —
    /// nothing has loaded messages into memory since the DB became the source of
    /// truth, so the derivation was already producing nothing.
    pub(crate) next_server_seq: HashMap<String, u64>,
    pub(crate) rate_limit_windows: HashMap<String, Vec<DateTime<chrono::Utc>>>,
    /// A fresh process has no in-flight retries to suppress.
    pub(crate) recent_message_ids: RecentMessageIds,
    /// Number of messages mirrored into this process since boot. Replaces
    /// `messages.values().map(Vec::len).sum()` now that message bodies are no
    /// longer held in memory; it feeds the `choruz_messages_total` gauge.
    pub(crate) messages_injected: usize,
    // ── Company (organization) layer ──
    pub(crate) companies: HashMap<String, Company>,
    pub(crate) company_members: HashMap<String, BTreeMap<String, CompanyMember>>,
}

/// Bounded set of recently-mirrored message ids, in insertion order.
///
/// `inject_message_with_event` must not push a second `message.created` event
/// for a message the gateway already mirrored — that happens when a client
/// retries a send and `DbService::send_message` returns the existing row. Those
/// duplicates arrive within seconds of each other, so a fixed-size window is
/// sufficient and, unlike the message map it replaces, cannot grow without
/// bound.
#[derive(Debug, Clone, Default)]
pub(crate) struct RecentMessageIds {
    seen: HashSet<String>,
    order: VecDeque<String>,
}

impl RecentMessageIds {
    const CAPACITY: usize = 10_000;

    /// Records `id` and returns `true` if it had not been seen recently.
    pub(crate) fn insert(&mut self, id: &str) -> bool {
        if !self.seen.insert(id.to_owned()) {
            return false;
        }
        self.order.push_back(id.to_owned());
        if self.order.len() > Self::CAPACITY
            && let Some(evicted) = self.order.pop_front()
        {
            self.seen.remove(&evicted);
        }
        true
    }
}

impl ChatApp {
    /// Build a ChatApp from raw data loaded externally (e.g., from DB tables).
    /// Computes all derived indices (`direct_index`, `next_event_seq`)
    /// automatically. Ephemeral fields (rate_limit, dedupe window) start empty.
    pub fn from_parts(
        principals: HashMap<String, Principal>,
        conversations: HashMap<String, Conversation>,
        companies: HashMap<String, Company>,
        company_members: HashMap<String, BTreeMap<String, CompanyMember>>,
        audit_logs: Vec<AuditLog>,
        rate_limit_per_minute: usize,
    ) -> Self {
        let mut state = State {
            principals,
            conversations,
            companies,
            company_members,
            audit_logs,
            ..State::default()
        };
        Self::rebuild_indices(&mut state);
        Self {
            inner: std::sync::Arc::new(std::sync::RwLock::new(state)),
            rate_limit_per_minute,
        }
    }

    /// Rebuild computed indices from core data.
    pub(crate) fn rebuild_indices(state: &mut State) {
        // ── direct_index: (workspace, left, right) → conversation_id
        state.direct_index.clear();
        for conv in state.conversations.values() {
            if conv.conversation_type == ConversationType::Direct {
                let all_member_ids: Vec<&String> = conv.members.keys().collect();
                let member_ids = if all_member_ids.len() == 2 {
                    all_member_ids
                } else {
                    all_member_ids
                        .into_iter()
                        .filter(|principal_id| {
                            state
                                .principals
                                .get(*principal_id)
                                .is_some_and(|principal| {
                                    matches!(principal.principal_type, PrincipalType::Agent)
                                })
                        })
                        .collect()
                };
                if member_ids.len() == 2 {
                    let key = direct_key(&conv.workspace_id, member_ids[0], member_ids[1]);
                    state.direct_index.insert(key, conv.id.clone());
                }
            }
        }
        // ── next_event_seq: principal_id → max(delivery_seq) + 1
        state.next_event_seq.clear();
        for (principal_id, envelopes) in &state.events {
            if let Some(max_seq) = envelopes.iter().map(|e| e.delivery_seq).max() {
                state
                    .next_event_seq
                    .insert(principal_id.clone(), max_seq + 1);
            }
        }
    }
}
