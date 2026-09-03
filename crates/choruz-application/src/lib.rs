mod audit;
mod companies;
mod conversations;
pub mod db_service;
mod events;
mod messages;
mod principals;
mod state;
mod types;

pub use db_service::{DbService, RateLimiter};
pub use types::*;

use std::sync::{Arc, RwLock};

use choruz_common::{AppError, AppResult, new_id, now};
use choruz_domain::{AuditLog, EventEnvelope, Principal, PrincipalType};
use chrono::{Duration, Utc};
use serde_json::Value;
use state::State;

use serde_json::json;

#[derive(Clone)]
pub struct ChatApp {
    inner: Arc<RwLock<State>>,
    rate_limit_per_minute: usize,
}

impl Default for ChatApp {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatApp {
    pub fn new() -> Self {
        Self::new_with_rate_limit(600)
    }

    pub fn new_with_rate_limit(rate_limit_per_minute: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(State::default())),
            rate_limit_per_minute,
        }
    }

    pub(crate) fn require_active_principal(
        &self,
        state: &State,
        principal_id: &str,
    ) -> AppResult<Principal> {
        let principal = state
            .principals
            .get(principal_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound("principal not found".into()))?;
        if principal.disabled || principal.deleted_at.is_some() {
            return Err(AppError::Forbidden("principal is disabled".into()));
        }
        Ok(principal)
    }

    pub(crate) fn ensure_scope(
        &self,
        principal: &Principal,
        required_scope: &str,
    ) -> AppResult<()> {
        if !matches!(principal.principal_type, PrincipalType::Agent) {
            return Ok(());
        }
        if principal.scopes.iter().any(|scope| scope == required_scope) {
            return Ok(());
        }
        Err(AppError::Forbidden(format!(
            "agent missing required scope: {required_scope}"
        )))
    }

    pub(crate) fn principal_can_access_workspace(
        &self,
        state: &State,
        principal: &Principal,
        workspace_id: &str,
    ) -> bool {
        let Some(company) = state.companies.get(workspace_id) else {
            return principal.workspace_id == workspace_id;
        };
        if company.deleted_at.is_some() {
            return false;
        }
        principal.workspace_id == workspace_id
            || state
                .company_members
                .get(workspace_id)
                .is_some_and(|members| members.contains_key(&principal.id))
    }

    pub(crate) fn check_rate_limit(&self, state: &mut State, principal_id: &str) -> AppResult<()> {
        let window_start = Utc::now() - Duration::minutes(1);
        let entries = state
            .rate_limit_windows
            .entry(principal_id.to_owned())
            .or_default();
        entries.retain(|timestamp| *timestamp > window_start);
        if entries.len() >= self.rate_limit_per_minute {
            return Err(AppError::RateLimited {
                retry_after_ms: 1000,
            });
        }
        entries.push(Utc::now());
        Ok(())
    }

    pub(crate) fn record_audit(
        &self,
        state: &mut State,
        actor: &Principal,
        action: &str,
        target_type: &str,
        target_id: &str,
        metadata: Value,
    ) {
        state.audit_logs.push(AuditLog {
            id: new_id(),
            workspace_id: actor.workspace_id.clone(),
            actor_id: actor.id.clone(),
            action: action.to_owned(),
            target_type: target_type.to_owned(),
            target_id: target_id.to_owned(),
            metadata,
            created_at: now(),
        });

        // GC: keep only the most recent 10 000 audit log entries
        const MAX_AUDIT_LOGS: usize = 10_000;
        if state.audit_logs.len() > MAX_AUDIT_LOGS {
            state
                .audit_logs
                .drain(0..state.audit_logs.len() - MAX_AUDIT_LOGS);
        }
    }

    pub(crate) fn push_event(
        &self,
        state: &mut State,
        principal_ids: &[String],
        event_type: &str,
        payload: Value,
    ) {
        for principal_id in principal_ids {
            let next_seq = state
                .next_event_seq
                .entry(principal_id.clone())
                .and_modify(|seq| *seq += 1)
                .or_insert(1);
            let events = state.events.entry(principal_id.clone()).or_default();
            events.push(EventEnvelope {
                delivery_seq: *next_seq,
                event_id: new_id(),
                principal_id: principal_id.clone(),
                event_type: event_type.to_owned(),
                payload: payload.clone(),
                created_at: now(),
            });
            // GC: only remove events the runner has already acked.
            // Take the min of polling ack cursor and webhook cursor so we
            // never discard events that a consumer has not yet processed.
            let ack_seq = state.ack_cursor.get(principal_id).copied().unwrap_or(0);
            let webhook_cursor = state
                .event_webhooks
                .get(principal_id)
                .map(|c| c.cursor)
                .unwrap_or(u64::MAX);
            let safe_cursor = ack_seq.min(webhook_cursor);
            events.retain(|e| e.delivery_seq > safe_cursor);

            // Hard safety net: if retain still leaves >50 000 entries,
            // truncate to prevent unbounded memory growth.
            const HARD_CAP: usize = 50_000;
            if events.len() > HARD_CAP {
                events.drain(0..events.len() - HARD_CAP);
            }
        }
    }

    /// Emit the `message.created` event that accompanies a membership change.
    ///
    /// The `content_type: "system"` body this used to build was written to the
    /// in-memory message map and nowhere else, and the event payload never
    /// carried it, so no consumer could ever read it — dropping the map made
    /// the body, and the caller-supplied text that fed it, dead weight.
    pub(crate) fn announce_system_message(
        &self,
        state: &mut State,
        conversation_id: &str,
    ) -> AppResult<()> {
        let conversation = state
            .conversations
            .get(conversation_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;
        let next_seq = *state
            .next_server_seq
            .entry(conversation_id.to_owned())
            .and_modify(|seq| *seq += 1)
            .or_insert(1);
        let recipients: Vec<String> = conversation.members.keys().cloned().collect();
        self.push_event(
            state,
            &recipients,
            "message.created",
            json!({
                "conversation_id": conversation_id,
                "message_id": new_id(),
                "server_seq": next_seq,
                "content_type": "system"
            }),
        );
        Ok(())
    }
}

pub(crate) fn direct_key(workspace_id: &str, left: &str, right: &str) -> (String, String, String) {
    if left <= right {
        (workspace_id.to_owned(), left.to_owned(), right.to_owned())
    } else {
        (workspace_id.to_owned(), right.to_owned(), left.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use choruz_domain::{Principal, PrincipalType};

    fn make_principal(id: &str, workspace: &str, scopes: Vec<&str>) -> Principal {
        Principal {
            id: id.to_string(),
            workspace_id: workspace.to_string(),
            name: format!("name-{id}"),
            avatar_url: None,
            principal_type: PrincipalType::Agent,
            scopes: scopes.into_iter().map(String::from).collect(),
            secret_hash: None,
            disabled: false,
            deleted_at: None,
            channel_visibility: choruz_domain::ChannelVisibility::Visible,
            created_at: now(),
            updated_at: now(),
            user_id: None,
        }
    }

    fn make_human(id: &str, workspace: &str) -> Principal {
        Principal {
            principal_type: PrincipalType::Human,
            ..make_principal(id, workspace, vec![])
        }
    }

    // direct_key ----------------------------------------------------------

    #[test]
    fn direct_key_sorts_lexicographically() {
        assert_eq!(
            direct_key("ws", "alice", "bob"),
            ("ws".into(), "alice".into(), "bob".into()),
        );
        assert_eq!(
            direct_key("ws", "bob", "alice"),
            ("ws".into(), "alice".into(), "bob".into()),
        );
    }

    #[test]
    fn direct_key_is_symmetric() {
        let a = direct_key("ws", "x", "y");
        let b = direct_key("ws", "y", "x");
        assert_eq!(a, b, "(x,y) and (y,x) must produce the same key");
    }

    #[test]
    fn direct_key_keeps_workspace_prefix() {
        let (ws, _, _) = direct_key("workspace-42", "a", "b");
        assert_eq!(ws, "workspace-42");
    }

    #[test]
    fn direct_key_handles_equal_ids() {
        // Edge case: someone trying to DM themselves. Function doesn't reject;
        // it just returns (ws, id, id).
        assert_eq!(
            direct_key("ws", "self", "self"),
            ("ws".into(), "self".into(), "self".into())
        );
    }

    // require_active_principal -------------------------------------------

    #[test]
    fn require_active_principal_returns_principal_when_active() {
        let app = ChatApp::new();
        let mut state = state::State::default();
        let p = make_principal("p1", "ws", vec![]);
        state.principals.insert(p.id.clone(), p.clone());
        let got = app.require_active_principal(&state, "p1").unwrap();
        assert_eq!(got.id, "p1");
    }

    #[test]
    fn require_active_principal_rejects_unknown_id() {
        let app = ChatApp::new();
        let state = state::State::default();
        let err = app.require_active_principal(&state, "missing").unwrap_err();
        assert!(matches!(err, choruz_common::AppError::NotFound(_)));
    }

    #[test]
    fn require_active_principal_rejects_disabled_principals() {
        let app = ChatApp::new();
        let mut state = state::State::default();
        let mut p = make_principal("p1", "ws", vec![]);
        p.disabled = true;
        state.principals.insert(p.id.clone(), p);
        let err = app.require_active_principal(&state, "p1").unwrap_err();
        assert!(matches!(err, choruz_common::AppError::Forbidden(_)));
    }

    #[test]
    fn require_active_principal_rejects_soft_deleted_principals() {
        let app = ChatApp::new();
        let mut state = state::State::default();
        let mut p = make_principal("p1", "ws", vec![]);
        p.deleted_at = Some(now());
        state.principals.insert(p.id.clone(), p);
        let err = app.require_active_principal(&state, "p1").unwrap_err();
        assert!(matches!(err, choruz_common::AppError::Forbidden(_)));
    }

    // ensure_scope --------------------------------------------------------

    #[test]
    fn ensure_scope_passes_for_human_regardless_of_scope() {
        let app = ChatApp::new();
        let human = make_human("human1", "ws");
        // Human lacks any scope; the check should still pass because
        // `ensure_scope` only enforces scopes on Agents.
        assert!(app.ensure_scope(&human, "messages.write").is_ok());
    }

    #[test]
    fn ensure_scope_passes_when_agent_has_required_scope() {
        let app = ChatApp::new();
        let agent = make_principal("a1", "ws", vec!["messages.write"]);
        assert!(app.ensure_scope(&agent, "messages.write").is_ok());
    }

    #[test]
    fn ensure_scope_rejects_agent_without_required_scope() {
        let app = ChatApp::new();
        let agent = make_principal("a1", "ws", vec!["messages.read"]);
        let err = app.ensure_scope(&agent, "messages.write").unwrap_err();
        assert!(matches!(err, choruz_common::AppError::Forbidden(_)));
    }

    // check_rate_limit ----------------------------------------------------

    #[test]
    fn check_rate_limit_admits_under_limit() {
        let app = ChatApp::new_with_rate_limit(3);
        let mut state = state::State::default();
        for _ in 0..3 {
            assert!(app.check_rate_limit(&mut state, "p1").is_ok());
        }
    }

    #[test]
    fn check_rate_limit_rejects_at_limit() {
        let app = ChatApp::new_with_rate_limit(2);
        let mut state = state::State::default();
        assert!(app.check_rate_limit(&mut state, "p1").is_ok());
        assert!(app.check_rate_limit(&mut state, "p1").is_ok());
        let err = app.check_rate_limit(&mut state, "p1").unwrap_err();
        assert!(matches!(err, choruz_common::AppError::RateLimited { .. }));
    }

    #[test]
    fn check_rate_limit_is_per_principal() {
        let app = ChatApp::new_with_rate_limit(1);
        let mut state = state::State::default();
        assert!(app.check_rate_limit(&mut state, "p1").is_ok());
        // Different principal — fresh window.
        assert!(app.check_rate_limit(&mut state, "p2").is_ok());
        // Re-using p1 — exceeds.
        assert!(app.check_rate_limit(&mut state, "p1").is_err());
    }

    // ChatApp::new_with_rate_limit configures correctly --------------------

    #[test]
    fn new_with_rate_limit_stores_the_limit() {
        let app = ChatApp::new_with_rate_limit(42);
        assert_eq!(app.rate_limit_per_minute, 42);
    }
}
