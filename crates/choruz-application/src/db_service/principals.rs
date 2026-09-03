use choruz_auth::{hash_secret, issue_secret};
use choruz_common::{AppError, new_id, now};
use choruz_domain::{ChannelVisibility, Principal, PrincipalType};

use super::DbService;
use super::helpers::{row_to_principal, scopes_for_type};
use crate::{AgentSecretResponse, CreateAgentRequest, RotateAgentSecretRequest};

impl DbService {
    // ── Principal reads (Phase 1A) ───────────────────────────────────────

    /// Get a principal by ID from the database.
    ///
    /// Returns the principal if found and active (not disabled, not deleted).
    /// Mirrors the semantics of `ChatApp::get_principal` which calls
    /// `require_active_principal`.
    pub async fn get_principal(&self, id: &str) -> Result<Principal, AppError> {
        let client = self.store.connect().await?;
        let row = client
            .query_opt(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash, \
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("get_principal: {e}")))?;

        let row = row.ok_or_else(|| AppError::NotFound("principal not found".into()))?;
        let principal = row_to_principal(&row);

        // Mirror ChatApp::require_active_principal — reject disabled / deleted
        if principal.disabled || principal.deleted_at.is_some() {
            return Err(AppError::Forbidden("principal is disabled".into()));
        }

        Ok(principal)
    }

    /// List active principals by ID, preserving only non-disabled/non-deleted rows.
    pub async fn list_principals_by_ids(&self, ids: &[String]) -> Result<Vec<Principal>, AppError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash, \
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal
                 WHERE id = ANY($1)
                   AND disabled = FALSE
                   AND deleted_at IS NULL
                 ORDER BY name",
                &[&ids],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_principals_by_ids: {e}")))?;

        Ok(rows.iter().map(row_to_principal).collect())
    }

    /// Authenticate an agent by its secret token.
    ///
    /// Loads all agent principals with a non-null secret_hash from the
    /// database and checks each one against the provided token.
    /// This mirrors `ChatApp::authenticate_agent_secret` but queries DB.
    pub async fn authenticate_agent_secret(&self, secret: &str) -> Result<Principal, AppError> {
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash, \
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal
                 WHERE type = 'agent'
                   AND secret_hash IS NOT NULL
                   AND disabled = FALSE
                   AND deleted_at IS NULL",
                &[],
            )
            .await
            .map_err(|e| AppError::Internal(format!("authenticate_agent_secret: {e}")))?;

        for row in &rows {
            let principal = row_to_principal(row);
            if let Some(ref hash) = principal.secret_hash
                && choruz_auth::verify_secret(secret, hash)
            {
                return Ok(principal);
            }
        }

        Err(AppError::Unauthorized("invalid agent secret".into()))
    }

    /// Ensure the local operator's human principal exists in the database.
    ///
    /// If the operator principal already exists, returns it. Otherwise creates
    /// it. Mirrors `ChatApp::ensure_local_operator` but persists to DB.
    pub async fn ensure_local_operator(
        &self,
        workspace_id: &str,
        display_name: &str,
    ) -> Result<Principal, AppError> {
        if workspace_id.trim().is_empty() {
            return Err(AppError::Validation("workspace_id is required".into()));
        }
        if display_name.trim().is_empty() {
            return Err(AppError::Validation("display_name is required".into()));
        }

        let principal_id = choruz_auth::local_user_principal_id(workspace_id, display_name);
        let mut client = self.store.connect().await?;

        // Reuse the configured operator if it already exists.
        let existing = client
            .query_opt(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash, \
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal WHERE id = $1",
                &[&principal_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("ensure_local_operator lookup: {e}")))?;

        if let Some(row) = existing {
            let principal = row_to_principal(&row);
            drop(client);
            self.ensure_default_company_for_operator(&principal).await?;
            return Ok(principal);
        }

        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("ensure_local_operator transaction: {e}")))?;
        // Create this configured operator without reusing an unrelated account.
        let now = choruz_common::now();
        let name = display_name.trim();
        tx.execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'human', $3, FALSE, $4, $4)
                 ON CONFLICT DO NOTHING",
            &[&principal_id, &workspace_id.trim(), &name, &now],
        )
        .await
        .map_err(|e| AppError::Internal(format!("ensure_local_operator insert: {e}")))?;

        // Re-fetch to get the canonical row (handles race conditions)
        let row = tx
            .query_one(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash, \
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal WHERE id = $1",
                &[&principal_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("ensure_local_operator refetch: {e}")))?;

        let principal = row_to_principal(&row);
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("ensure_local_operator commit: {e}")))?;
        drop(client);
        self.ensure_default_company_for_operator(&principal).await?;
        Ok(principal)
    }

    async fn ensure_default_company_for_operator(
        &self,
        principal: &Principal,
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        let timestamp = choruz_common::now();
        client
            .execute(
                "INSERT INTO company
                    (id, name, slug, description, avatar_url, owner_id, agents_active, folder_path, created_at, updated_at)
                 VALUES ($1, 'Default', 'default', 'Default local workspace', '', $2, TRUE, '', $3, $3)
                 ON CONFLICT DO NOTHING",
                &[&principal.workspace_id, &principal.id, &timestamp],
            )
            .await
            .map_err(|e| AppError::Internal(format!("ensure_default_company insert: {e}")))?;

        client
            .execute(
                "UPDATE company
                    SET owner_id = $2,
                        agents_active = TRUE,
                        updated_at = updated_at
                  WHERE id = $1",
                &[&principal.workspace_id, &principal.id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("ensure_default_company update: {e}")))?;

        client
            .execute(
                "INSERT INTO company_member (company_id, principal_id, joined_at)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (company_id, principal_id) DO NOTHING",
                &[&principal.workspace_id, &principal.id, &timestamp],
            )
            .await
            .map_err(|e| AppError::Internal(format!("ensure_default_company member: {e}")))?;

        Ok(())
    }

    /// List agents in a workspace from the database.
    ///
    /// Returns only active (non-disabled) agents, sorted by name.
    /// Mirrors `ChatApp::list_workspace_agents`.
    pub async fn list_workspace_agents(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<Principal>, AppError> {
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash, \
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal
                 WHERE workspace_id = $1 AND type = 'agent'
                   AND disabled = FALSE AND deleted_at IS NULL
                 ORDER BY name",
                &[&workspace_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_workspace_agents: {e}")))?;

        Ok(rows.iter().map(row_to_principal).collect())
    }

    /// List active agents from every company visible to a human in one query.
    /// This is the bounded-bootstrap counterpart to calling
    /// `list_agents_for_company` once per company.
    pub async fn list_accessible_agents(
        &self,
        principal_id: &str,
    ) -> Result<Vec<Principal>, AppError> {
        let principal = self.get_principal(principal_id).await?;
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT DISTINCT p.id, p.workspace_id, p.type, p.name, p.avatar_url,
                        p.secret_hash, p.channel_visibility, p.disabled, p.deleted_at,
                        p.created_at, p.updated_at
                 FROM principal p
                 LEFT JOIN company c ON c.id = p.workspace_id
                 LEFT JOIN company_member cm
                   ON cm.company_id = c.id AND cm.principal_id = $1
                 WHERE p.type = 'agent'
                   AND p.disabled = FALSE
                   AND p.deleted_at IS NULL
                   AND ((c.id IS NULL AND p.workspace_id = $2)
                     OR (c.deleted_at IS NULL
                         AND (p.workspace_id = $2 OR cm.principal_id IS NOT NULL)))
                 ORDER BY p.name, p.id",
                &[&principal_id, &principal.workspace_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_accessible_agents: {e}")))?;

        Ok(rows.iter().map(row_to_principal).collect())
    }

    // ── Principal writes (Phase 2A) ─────────────────────────────────────

    // ── Human signup / login (Phase 6) ─────────────────────────────────

    /// Look up a human user by case-insensitive username.
    ///
    /// Used by the login form: humans authenticate with username + password
    /// rather than a bearer token, so we resolve the principal first, then
    /// verify the password against `secret_hash` (sha256 — same hashing
    /// used for agent secrets, see `choruz_auth::hash_secret`). The
    /// `principal_human_username_unique_idx` partial index guarantees at
    /// most one active human shares any username.
    pub async fn find_human_by_username(
        &self,
        username: &str,
    ) -> Result<Option<Principal>, AppError> {
        let client = self.store.connect().await?;
        let row = client
            .query_opt(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash, \
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal
                 WHERE type = 'human'
                   AND lower(name) = lower($1)
                   AND deleted_at IS NULL
                 LIMIT 1",
                &[&username],
            )
            .await
            .map_err(|e| AppError::Internal(format!("find_human_by_username: {e}")))?;
        Ok(row.as_ref().map(row_to_principal))
    }

    /// Create a new human user with a hashed password.
    ///
    /// Each signup gets a fresh per-user workspace `ws-{principal_id}` so
    /// different signups don't collide on the (workspace_id, name) agent
    /// uniqueness rule when they later create agents named the same thing
    /// (e.g. two users both spinning up a `frontend-dev`).
    pub async fn create_human_user(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Principal, AppError> {
        let trimmed = username.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation("username is required".into()));
        }
        if trimmed.len() < 3 || trimmed.len() > 32 {
            return Err(AppError::Validation(
                "username must be 3–32 characters".into(),
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(AppError::Validation(
                "username may only contain letters, digits, '-', '_', '.'".into(),
            ));
        }
        if password.len() < 8 {
            return Err(AppError::Validation(
                "password must be at least 8 characters".into(),
            ));
        }

        let principal_id = new_id();
        let workspace_id = format!("ws-{principal_id}");
        let secret_hash = hash_secret(password);
        let timestamp = now();

        let client = self.store.connect().await?;
        client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, avatar_url, secret_hash, disabled, created_at, updated_at)
                 VALUES ($1, $2, 'human', $3, NULL, $4, false, $5, $5)",
                &[
                    &principal_id,
                    &workspace_id,
                    &trimmed,
                    &secret_hash,
                    &timestamp,
                ],
            )
            .await
            .map_err(|e| {
                // tokio_postgres::Error::Display reduces to "db error". To
                // distinguish unique-violation we drop down to as_db_error()
                // and check SqlState 23505.
                if e.as_db_error()
                    .map(|db| db.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
                    .unwrap_or(false)
                {
                    AppError::Conflict(format!("username '{trimmed}' is already taken"))
                } else {
                    AppError::Internal(format!("create_human_user: {e}"))
                }
            })?;
        Ok(Principal {
            id: principal_id,
            workspace_id,
            principal_type: PrincipalType::Human,
            name: trimmed.to_string(),
            avatar_url: None,
            scopes: scopes_for_type(&PrincipalType::Human),
            secret_hash: Some(secret_hash),
            disabled: false,
            deleted_at: None,
            channel_visibility: ChannelVisibility::Visible,
            created_at: timestamp,
            updated_at: timestamp,
            user_id: None,
        })
    }

    /// Create an agent principal in the database with a generated secret.
    ///
    /// Mirrors `ChatApp::create_agent` validation and logic: checks that
    /// the actor is a human who can access the target workspace, generates a
    /// secret, hashes it, persists to DB, and returns the plaintext secret
    /// alongside the principal.
    pub async fn create_agent(
        &self,
        request: CreateAgentRequest,
    ) -> Result<AgentSecretResponse, AppError> {
        // Validate actor is active and can create in the target workspace.
        let actor = self.get_principal(&request.actor_id).await?;
        if request.name.trim().is_empty() {
            return Err(AppError::Validation("name is required".into()));
        }

        let ws_id = request
            .workspace_id
            .filter(|w| !w.trim().is_empty())
            .unwrap_or_else(|| actor.workspace_id.clone());
        let channel_visibility = request
            .channel_visibility
            .unwrap_or(ChannelVisibility::Visible);
        if !matches!(actor.principal_type, PrincipalType::Human)
            || !self.principal_can_access_workspace(&actor, &ws_id).await?
        {
            return Err(AppError::Forbidden(
                "not authorized to create agents in this workspace".into(),
            ));
        }

        let secret = issue_secret();
        let secret_hash = hash_secret(&secret);
        let timestamp = now();
        let id = new_id();
        let channel_visibility_str = channel_visibility_as_str(&channel_visibility);

        let client = self.store.connect().await?;
        client
            .execute(
                "INSERT INTO principal (id, workspace_id, type, name, avatar_url, secret_hash, disabled, channel_visibility, created_at, updated_at)
                 VALUES ($1, $2, 'agent', $3, NULL, $4, false, $5, $6, $7)",
                &[
                    &id,
                    &ws_id,
                    &request.name,
                    &secret_hash,
                    &channel_visibility_str,
                    &timestamp,
                    &timestamp,
                ],
            )
            .await
            .map_err(|e| {
                // V013 added a partial-unique index on (workspace_id, lower(name))
                // for non-deleted principals. Surface the collision as a clear
                // 409 Conflict rather than letting the DB error bubble up as
                // an opaque 500 — and more importantly, stop the caller from
                // assuming the INSERT silently succeeded.
                if let Some(db_err) = e.as_db_error()
                    && db_err.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
                {
                    return AppError::Conflict(format!(
                        "an agent named '{}' already exists in this workspace",
                        request.name
                    ));
                }
                AppError::Internal(format!("create_agent: {e}"))
            })?;

        let scopes = if request.scopes.is_empty() {
            scopes_for_type(&PrincipalType::Agent)
        } else {
            request.scopes
        };

        let principal = Principal {
            id,
            workspace_id: ws_id,
            principal_type: PrincipalType::Agent,
            name: request.name,
            avatar_url: None,
            scopes,
            secret_hash: Some(secret_hash),
            disabled: false,
            deleted_at: None,
            channel_visibility,
            created_at: timestamp,
            updated_at: timestamp,
            user_id: None,
        };

        // Audit log
        self.record_audit(
            &principal.workspace_id,
            &actor.id,
            "agent.created",
            "principal",
            &principal.id,
            serde_json::json!({"scopes": principal.scopes}),
        )
        .await?;

        Ok(AgentSecretResponse { principal, secret })
    }

    /// Rotate the secret for an agent principal in the database.
    ///
    /// Mirrors `ChatApp::rotate_agent_secret` validation: checks that
    /// the actor is a human with access to the target agent's workspace.
    pub async fn rotate_agent_secret(
        &self,
        agent_id: &str,
        request: RotateAgentSecretRequest,
    ) -> Result<AgentSecretResponse, AppError> {
        let actor = self.get_principal(&request.actor_id).await?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "only humans can rotate agent secrets".into(),
            ));
        }

        // Fetch the agent (must be active)
        let agent = self.get_principal(agent_id).await?;
        if !matches!(agent.principal_type, PrincipalType::Agent) {
            return Err(AppError::Validation("target is not an agent".into()));
        }
        if !self
            .principal_can_access_workspace(&actor, &agent.workspace_id)
            .await?
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let secret = issue_secret();
        let secret_hash = hash_secret(&secret);
        let timestamp = now();

        let client = self.store.connect().await?;
        client
            .execute(
                "UPDATE principal SET secret_hash = $1, updated_at = $2 WHERE id = $3",
                &[&secret_hash, &timestamp, &agent_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("rotate_agent_secret: {e}")))?;

        let principal = Principal {
            secret_hash: Some(secret_hash),
            updated_at: timestamp,
            ..agent
        };

        // Audit log
        self.record_audit(
            &principal.workspace_id,
            &actor.id,
            "agent.secret_rotated",
            "principal",
            agent_id,
            serde_json::json!({}),
        )
        .await?;

        Ok(AgentSecretResponse { principal, secret })
    }

    /// Disable a principal in the database.
    ///
    /// Mirrors `ChatApp::disable_principal` validation: checks that the
    /// actor is a human with access to the target workspace.
    pub async fn disable_principal(
        &self,
        actor_id: &str,
        target_id: &str,
    ) -> Result<Principal, AppError> {
        self.deactivate_principal(actor_id, target_id, false).await
    }

    /// Soft-delete a principal while also marking it disabled.
    ///
    /// Generated-agent cleanup must set `deleted_at`: the workspace/name
    /// uniqueness index excludes soft-deleted principals, but not merely
    /// disabled ones.
    pub async fn soft_delete_principal(
        &self,
        actor_id: &str,
        target_id: &str,
    ) -> Result<Principal, AppError> {
        self.deactivate_principal(actor_id, target_id, true).await
    }

    async fn deactivate_principal(
        &self,
        actor_id: &str,
        target_id: &str,
        soft_delete: bool,
    ) -> Result<Principal, AppError> {
        let actor = self.get_principal(actor_id).await?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(if soft_delete {
                "only humans can soft-delete principals".into()
            } else {
                "only humans can disable principals".into()
            }));
        }

        // Fetch the target — we use a raw query here instead of get_principal
        // because get_principal rejects already-disabled principals, and we
        // want to allow idempotent disable.
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("deactivate_principal transaction: {e}")))?;
        let row = tx
            .query_opt(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash, \
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal WHERE id = $1
                 FOR UPDATE",
                &[&target_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("deactivate_principal: {e}")))?;

        let row = row.ok_or_else(|| AppError::NotFound("principal not found".into()))?;
        let target = row_to_principal(&row);

        if !self
            .principal_can_access_workspace(&actor, &target.workspace_id)
            .await?
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let timestamp = now();
        let deleted_at = soft_delete.then(|| target.deleted_at.unwrap_or(timestamp));
        tx.execute(
            "UPDATE agent_runtime_bindings
                 SET state = 'disabled',
                     in_flight_turn_id = NULL,
                     updated_at = $1
                 WHERE agent_principal_id = $2
                   AND state <> 'disabled'",
            &[&timestamp, &target_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("disable principal bindings: {e}")))?;
        tx.execute(
            "UPDATE principal
                 SET disabled = true,
                     deleted_at = COALESCE($1, deleted_at),
                     updated_at = $2
                 WHERE id = $3",
            &[&deleted_at, &timestamp, &target_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("deactivate_principal update: {e}")))?;

        let action = if soft_delete {
            "principal.soft_deleted"
        } else {
            "principal.disabled"
        };
        tx.execute(
            "INSERT INTO audit_log
             (id, workspace_id, actor_id, action, target_type, target_id, metadata, created_at)
             VALUES ($1, $2, $3, $4, 'principal', $5, $6, $7)",
            &[
                &new_id(),
                &target.workspace_id,
                &actor_id,
                &action,
                &target_id,
                &serde_json::json!({}),
                &timestamp,
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("deactivate_principal audit: {e}")))?;

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("deactivate_principal commit: {e}")))?;

        let result = Principal {
            disabled: true,
            deleted_at: deleted_at.or(target.deleted_at),
            updated_at: timestamp,
            ..target
        };

        Ok(result)
    }
}

fn channel_visibility_as_str(value: &ChannelVisibility) -> &'static str {
    match value {
        ChannelVisibility::Visible => "visible",
        ChannelVisibility::Internal => "internal",
    }
}
