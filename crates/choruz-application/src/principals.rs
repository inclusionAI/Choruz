use choruz_auth::{hash_secret, issue_secret, local_user_principal_id};
use choruz_common::{AppError, AppResult, new_id, now};
use choruz_domain::{Principal, PrincipalType};
use serde_json::json;

use crate::{
    AgentSecretResponse, ChatApp, CreateAgentRequest, CreatePrincipalRequest,
    RotateAgentSecretRequest,
};

impl ChatApp {
    pub fn ensure_local_operator(
        &self,
        workspace_id: &str,
        display_name: &str,
    ) -> AppResult<Principal> {
        if workspace_id.trim().is_empty() {
            return Err(AppError::Validation("workspace_id is required".into()));
        }
        if display_name.trim().is_empty() {
            return Err(AppError::Validation("display_name is required".into()));
        }

        let principal_id = local_user_principal_id(workspace_id, display_name);
        let mut state = self.inner.write().expect("lock poisoned");
        if let Some(existing) = state.principals.get(&principal_id) {
            return Ok(existing.clone());
        }

        let timestamp = now();
        let principal = Principal {
            id: principal_id,
            workspace_id: workspace_id.trim().into(),
            principal_type: PrincipalType::Human,
            name: display_name.trim().into(),
            avatar_url: None,
            scopes: vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
                "groups:manage".into(),
                "agents:manage".into(),
            ],
            secret_hash: None,
            disabled: false,
            deleted_at: None,
            channel_visibility: choruz_domain::ChannelVisibility::Visible,
            created_at: timestamp,
            updated_at: timestamp,
            user_id: None,
        };
        state
            .principals
            .insert(principal.id.clone(), principal.clone());
        Ok(principal)
    }

    pub fn create_principal(&self, request: CreatePrincipalRequest) -> AppResult<Principal> {
        if request.workspace_id.trim().is_empty() {
            return Err(AppError::Validation("workspace_id is required".into()));
        }
        if request.name.trim().is_empty() {
            return Err(AppError::Validation("name is required".into()));
        }
        if matches!(request.principal_type, PrincipalType::Agent) {
            return Err(AppError::Validation(
                "use the agent lifecycle API to create agents".into(),
            ));
        }

        let timestamp = now();
        let principal = Principal {
            id: new_id(),
            workspace_id: request.workspace_id,
            principal_type: request.principal_type,
            name: request.name,
            avatar_url: request.avatar_url,
            scopes: Vec::new(),
            secret_hash: None,
            disabled: false,
            deleted_at: None,
            channel_visibility: choruz_domain::ChannelVisibility::Visible,
            created_at: timestamp,
            updated_at: timestamp,
            user_id: None,
        };

        let mut state = self.inner.write().expect("lock poisoned");
        state
            .principals
            .insert(principal.id.clone(), principal.clone());

        Ok(principal)
    }

    pub fn get_principal(&self, principal_id: &str) -> AppResult<Principal> {
        let state = self.inner.read().expect("lock poisoned");
        self.require_active_principal(&state, principal_id)
    }

    /// Inject a principal directly into memory, bypassing validation.
    /// Used by callers that need to mirror a database principal in memory.
    pub fn inject_principal(&self, principal: Principal) {
        let mut state = self.inner.write().expect("lock poisoned");
        state.principals.insert(principal.id.clone(), principal);
    }

    /// Set the secret_hash for a principal in memory.
    /// Used to refresh an agent hash after token-file recovery.
    pub fn set_principal_secret_hash(&self, principal_id: &str, hash: &str) {
        let mut state = self.inner.write().expect("lock poisoned");
        if let Some(p) = state.principals.get_mut(principal_id) {
            p.secret_hash = Some(hash.to_string());
        }
    }

    /// Check if a principal exists in memory.
    pub fn has_principal(&self, principal_id: &str) -> bool {
        let state = self.inner.read().expect("lock poisoned");
        state.principals.contains_key(principal_id)
    }

    /// Return the number of principals currently held in memory.
    pub fn principal_count(&self) -> usize {
        let state = self.inner.read().expect("lock poisoned");
        state.principals.len()
    }

    pub fn create_agent(&self, request: CreateAgentRequest) -> AppResult<AgentSecretResponse> {
        let mut state = self.inner.write().expect("lock poisoned");
        self.check_rate_limit(&mut state, &request.actor_id)?;

        let actor = self.require_active_principal(&state, &request.actor_id)?;
        if request.name.trim().is_empty() {
            return Err(AppError::Validation("name is required".into()));
        }

        let ws_id = request
            .workspace_id
            .filter(|w| !w.trim().is_empty())
            .unwrap_or_else(|| actor.workspace_id.clone());
        let channel_visibility = request
            .channel_visibility
            .unwrap_or(choruz_domain::ChannelVisibility::Visible);
        if !matches!(actor.principal_type, PrincipalType::Human)
            || !self.principal_can_access_workspace(&state, &actor, &ws_id)
        {
            return Err(AppError::Forbidden(
                "not authorized to create agents in this workspace".into(),
            ));
        }

        let secret = issue_secret();
        let timestamp = now();
        let principal = Principal {
            id: new_id(),
            workspace_id: ws_id,
            principal_type: PrincipalType::Agent,
            name: request.name,
            avatar_url: None,
            scopes: request.scopes,
            secret_hash: Some(hash_secret(&secret)),
            disabled: false,
            deleted_at: None,
            channel_visibility,
            created_at: timestamp,
            updated_at: timestamp,
            user_id: None,
        };

        state
            .principals
            .insert(principal.id.clone(), principal.clone());

        self.record_audit(
            &mut state,
            &actor,
            "agent.created",
            "principal",
            &principal.id,
            json!({"scopes": principal.scopes}),
        );
        self.push_event(
            &mut state,
            std::slice::from_ref(&actor.id),
            "agent.created",
            json!({"agent_id": principal.id}),
        );

        Ok(AgentSecretResponse { principal, secret })
    }

    pub fn rotate_agent_secret(
        &self,
        agent_id: &str,
        request: RotateAgentSecretRequest,
    ) -> AppResult<AgentSecretResponse> {
        let mut state = self.inner.write().expect("lock poisoned");
        self.check_rate_limit(&mut state, &request.actor_id)?;

        let actor = self.require_active_principal(&state, &request.actor_id)?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "only humans can rotate agent secrets".into(),
            ));
        }

        let agent_workspace = state
            .principals
            .get(agent_id)
            .ok_or_else(|| AppError::NotFound("agent not found".into()))?
            .workspace_id
            .clone();
        if !self.principal_can_access_workspace(&state, &actor, &agent_workspace) {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }
        let agent = state
            .principals
            .get_mut(agent_id)
            .ok_or_else(|| AppError::NotFound("agent not found".into()))?;
        if !matches!(agent.principal_type, PrincipalType::Agent) {
            return Err(AppError::Validation("target is not an agent".into()));
        }

        let secret = issue_secret();
        agent.secret_hash = Some(hash_secret(&secret));
        agent.updated_at = now();
        let principal = agent.clone();

        self.record_audit(
            &mut state,
            &actor,
            "agent.secret_rotated",
            "principal",
            agent_id,
            json!({}),
        );
        self.push_event(
            &mut state,
            &[actor.id.clone(), agent_id.to_owned()],
            "agent.secret_rotated",
            json!({"agent_id": agent_id}),
        );

        Ok(AgentSecretResponse { principal, secret })
    }

    pub fn disable_principal(&self, actor_id: &str, target_id: &str) -> AppResult<Principal> {
        self.disable_principal_inner(actor_id, target_id, true)
    }

    /// Disable without consuming rate limit for human control-plane operations.
    pub fn disable_principal_batch(&self, actor_id: &str, target_id: &str) -> AppResult<Principal> {
        self.disable_principal_inner(actor_id, target_id, false)
    }

    fn disable_principal_inner(
        &self,
        actor_id: &str,
        target_id: &str,
        check_rate: bool,
    ) -> AppResult<Principal> {
        let mut state = self.inner.write().expect("lock poisoned");
        if check_rate {
            self.check_rate_limit(&mut state, actor_id)?;
        }

        let actor = self.require_active_principal(&state, actor_id)?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "only humans can disable principals".into(),
            ));
        }

        let target_workspace = state
            .principals
            .get(target_id)
            .ok_or_else(|| AppError::NotFound("principal not found".into()))?
            .workspace_id
            .clone();
        if !self.principal_can_access_workspace(&state, &actor, &target_workspace) {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }
        let target = state
            .principals
            .get_mut(target_id)
            .ok_or_else(|| AppError::NotFound("principal not found".into()))?;
        target.disabled = true;
        target.updated_at = now();
        let target = target.clone();

        self.record_audit(
            &mut state,
            &actor,
            "principal.disabled",
            "principal",
            target_id,
            json!({}),
        );

        Ok(target)
    }

    /// Move a principal between workspaces the human actor can access.
    pub fn migrate_principal_workspace(
        &self,
        principal_id: &str,
        new_workspace_id: &str,
        actor_id: &str,
    ) -> AppResult<Principal> {
        let mut state = self.inner.write().expect("lock poisoned");
        let actor = self.require_active_principal(&state, actor_id)?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "only humans can migrate workspaces".into(),
            ));
        }
        if new_workspace_id.trim().is_empty() {
            return Err(AppError::Validation("workspace_id is required".into()));
        }
        let current_workspace = state
            .principals
            .get(principal_id)
            .ok_or_else(|| AppError::NotFound("principal not found".into()))?
            .workspace_id
            .clone();
        if !self.principal_can_access_workspace(&state, &actor, &current_workspace)
            || !self.principal_can_access_workspace(&state, &actor, new_workspace_id)
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let principal = state
            .principals
            .get_mut(principal_id)
            .ok_or_else(|| AppError::NotFound("principal not found".into()))?;

        let old_workspace_id = principal.workspace_id.clone();
        principal.workspace_id = new_workspace_id.to_string();
        let principal = principal.clone();

        self.record_audit(
            &mut state,
            &actor,
            "principal.workspace_migrated",
            "principal",
            principal_id,
            json!({
                "old_workspace_id": old_workspace_id,
                "new_workspace_id": new_workspace_id,
            }),
        );

        Ok(principal)
    }
}

#[cfg(test)]
mod principals_tests {
    use super::*;
    use crate::{ChatApp, CreateAgentRequest, CreatePrincipalRequest};

    fn human(app: &ChatApp, ws: &str) -> Principal {
        app.ensure_local_operator(ws, "operator").unwrap()
    }

    // ensure_local_operator -------------------------------------------------

    #[test]
    fn ensure_local_operator_creates_then_returns_existing_idempotently() {
        let app = ChatApp::new();
        let a = app.ensure_local_operator("ws", "operator").unwrap();
        let b = app.ensure_local_operator("ws", "operator").unwrap();
        assert_eq!(a.id, b.id, "idempotent");
        assert_eq!(app.principal_count(), 1);
        assert_eq!(a.principal_type, PrincipalType::Human);
        assert!(a.scopes.contains(&"agents:manage".to_string()));
    }

    #[test]
    fn ensure_local_operator_supports_multiple_workspaces() {
        let app = ChatApp::new();
        let first = app.ensure_local_operator("ws-one", "operator").unwrap();
        let second = app.ensure_local_operator("ws-two", "operator").unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(first.workspace_id, "ws-one");
        assert_eq!(second.workspace_id, "ws-two");
        assert_eq!(app.principal_count(), 2);
    }

    #[test]
    fn ensure_local_operator_rejects_blank_workspace_id() {
        let app = ChatApp::new();
        let err = app.ensure_local_operator("   ", "operator").unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn ensure_local_operator_rejects_blank_display_name() {
        let app = ChatApp::new();
        let err = app.ensure_local_operator("ws", "  ").unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    // create_principal ---------------------------------------------------

    #[test]
    fn create_principal_creates_a_human_principal() {
        let app = ChatApp::new();
        let p = app
            .create_principal(CreatePrincipalRequest {
                workspace_id: "ws".into(),
                name: "alice".into(),
                principal_type: PrincipalType::Human,
                avatar_url: None,
            })
            .unwrap();
        assert_eq!(p.principal_type, PrincipalType::Human);
        assert!(
            p.scopes.is_empty(),
            "create_principal does not set default scopes"
        );
        assert!(!p.disabled);
        assert_eq!(
            p.channel_visibility,
            choruz_domain::ChannelVisibility::Visible
        );
        assert!(app.has_principal(&p.id));
    }

    #[test]
    fn create_agent_defaults_visible_and_allows_internal_visibility() {
        let app = ChatApp::new();
        let human = human(&app, "ws");

        let visible_agent = app
            .create_agent(CreateAgentRequest {
                actor_id: human.id.clone(),
                name: "visible-agent".into(),
                scopes: vec!["messages:read".into()],
                workspace_id: None,
                channel_visibility: None,
            })
            .unwrap()
            .principal;
        assert_eq!(
            visible_agent.channel_visibility,
            choruz_domain::ChannelVisibility::Visible
        );
        assert!(app.has_principal(&visible_agent.id));

        let internal_agent = app
            .create_agent(CreateAgentRequest {
                actor_id: human.id.clone(),
                name: "internal-agent".into(),
                scopes: vec!["messages:read".into()],
                workspace_id: None,
                channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
            })
            .unwrap()
            .principal;
        assert_eq!(
            internal_agent.channel_visibility,
            choruz_domain::ChannelVisibility::Internal
        );
        assert!(app.has_principal(&internal_agent.id));
    }

    #[test]
    fn create_agent_allows_human_in_own_workspace_but_not_another_workspace() {
        let app = ChatApp::new();
        let human = app
            .create_principal(CreatePrincipalRequest {
                workspace_id: "human-workspace".into(),
                principal_type: PrincipalType::Human,
                name: "Alice".into(),
                avatar_url: None,
            })
            .unwrap();

        let agent = app
            .create_agent(CreateAgentRequest {
                actor_id: human.id.clone(),
                name: "Alice's agent".into(),
                scopes: vec![],
                workspace_id: None,
                channel_visibility: None,
            })
            .unwrap()
            .principal;
        assert_eq!(agent.workspace_id, human.workspace_id);

        let internal_agent = app
            .create_agent(CreateAgentRequest {
                actor_id: human.id.clone(),
                name: "Internal agent".into(),
                scopes: vec![],
                workspace_id: None,
                channel_visibility: Some(choruz_domain::ChannelVisibility::Internal),
            })
            .unwrap()
            .principal;
        assert_eq!(
            internal_agent.channel_visibility,
            choruz_domain::ChannelVisibility::Internal
        );

        let err = app
            .create_agent(CreateAgentRequest {
                actor_id: human.id,
                name: "Unauthorized agent".into(),
                scopes: vec![],
                workspace_id: Some("another-workspace".into()),
                channel_visibility: None,
            })
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn create_principal_rejects_agent_type() {
        let app = ChatApp::new();
        let err = app
            .create_principal(CreatePrincipalRequest {
                workspace_id: "ws".into(),
                name: "agent".into(),
                principal_type: PrincipalType::Agent,
                avatar_url: None,
            })
            .unwrap_err();
        // Should redirect to the agent lifecycle API
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn create_principal_rejects_blank_workspace_or_name() {
        let app = ChatApp::new();
        for (ws, name) in [("", "alice"), ("ws", ""), ("   ", "alice"), ("ws", "  ")] {
            assert!(
                app.create_principal(CreatePrincipalRequest {
                    workspace_id: ws.into(),
                    name: name.into(),
                    principal_type: PrincipalType::Human,
                    avatar_url: None,
                })
                .is_err()
            );
        }
    }

    // get_principal / has_principal / principal_count -------------------

    #[test]
    fn principal_lookup_helpers() {
        let app = ChatApp::new();
        let human = human(&app, "ws");
        assert!(app.has_principal(&human.id));
        assert!(!app.has_principal("missing"));
        assert_eq!(app.principal_count(), 1);
        assert_eq!(app.get_principal(&human.id).unwrap().id, human.id);
        assert!(app.get_principal("missing").is_err());
    }

    // inject_principal / set_principal_secret_hash ---------------------

    #[test]
    fn inject_and_secret_hash_round_trip() {
        let app = ChatApp::new();
        let p = Principal {
            id: "p-injected".into(),
            workspace_id: "ws".into(),
            principal_type: PrincipalType::Agent,
            name: "agent-x".into(),
            avatar_url: None,
            scopes: vec!["messages:write".into()],
            secret_hash: None,
            disabled: false,
            deleted_at: None,
            channel_visibility: choruz_domain::ChannelVisibility::Visible,
            created_at: now(),
            updated_at: now(),
            user_id: None,
        };
        app.inject_principal(p.clone());
        app.set_principal_secret_hash("p-injected", "deadbeef");
        let got = app.get_principal("p-injected").unwrap();
        assert_eq!(got.secret_hash.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn set_secret_hash_is_a_no_op_for_unknown_principal() {
        let app = ChatApp::new();
        // Should not panic.
        app.set_principal_secret_hash("missing", "abcd");
        assert!(!app.has_principal("missing"));
    }
}
