use std::collections::{BTreeMap, HashSet};

use choruz_common::{AppError, new_id, now};
use choruz_domain::{Conversation, ConversationMember, ConversationType, Message, PrincipalType};
use chrono::{DateTime, Utc};

use super::DbService;
use super::helpers::{row_to_conversation, row_to_member, rows_to_members};

impl DbService {
    /// Resolve the workspace's human for conversations initiated entirely by
    /// agents. Company ownership is preferred; a workspace-scoped human is the
    /// fallback for legacy/default workspaces.
    async fn human_for_conversation_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<choruz_domain::Principal, AppError> {
        let client = self.store.connect().await?;
        if let Some(row) = client
            .query_opt(
                "SELECT p.id, p.workspace_id, p.type, p.name, p.avatar_url, p.secret_hash,
                        p.channel_visibility, p.disabled, p.deleted_at, p.created_at, p.updated_at
                 FROM company c
                 JOIN principal p ON p.id = c.owner_id
                 WHERE c.id = $1 AND c.deleted_at IS NULL
                   AND p.type = 'human' AND p.disabled = FALSE AND p.deleted_at IS NULL
                   AND (p.workspace_id = c.id OR EXISTS (
                       SELECT 1 FROM company_member cm
                       WHERE cm.company_id = c.id AND cm.principal_id = p.id
                   ))",
                &[&workspace_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("conversation human owner lookup: {e}")))?
        {
            return Ok(super::helpers::row_to_principal(&row));
        }

        let rows = client
            .query(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash,
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal
                 WHERE type = 'human' AND disabled = FALSE AND deleted_at IS NULL
                   AND (workspace_id = $1 OR EXISTS (
                       SELECT 1 FROM company_member
                       WHERE company_id = $1 AND principal_id = principal.id
                   ))
                 ORDER BY created_at ASC
                 LIMIT 2",
                &[&workspace_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("conversation human lookup: {e}")))?;
        match rows.as_slice() {
            [row] => Ok(super::helpers::row_to_principal(row)),
            [] => Err(AppError::Conflict(
                "a human user must exist before agents can create conversations".into(),
            )),
            _ => Err(AppError::Conflict(
                "this workspace needs one active human owner for agent conversations".into(),
            )),
        }
    }

    // ── Conversation reads (Phase 1B) ───────────────────────────────────

    /// Get a conversation by ID with its active members from the database.
    ///
    /// Mirrors `ChatApp::get_conversation` but queries PostgreSQL directly.
    pub async fn get_conversation(&self, id: &str) -> Result<Conversation, AppError> {
        let client = self.store.connect().await?;

        let row = client
            .query_opt(
                "SELECT id, workspace_id, type, name, description, avatar_url, \
                        creator_id, created_at, updated_at
                 FROM conversation WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("get_conversation: {e}")))?;

        let row = row.ok_or_else(|| AppError::NotFound("conversation not found".into()))?;

        let member_rows = client
            .query(
                "SELECT principal_id, joined_at
                 FROM conversation_member
                 WHERE conv_id = $1 AND removed_at IS NULL",
                &[&id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("get_conversation members: {e}")))?;

        let members = rows_to_members(&member_rows);
        Ok(row_to_conversation(&row, members))
    }

    /// List conversations that a principal is a member of.
    ///
    pub async fn list_conversations(
        &self,
        principal_id: &str,
    ) -> Result<Vec<Conversation>, AppError> {
        let principal = self.get_principal(principal_id).await?;

        let client = self.store.connect().await?;

        let conv_rows = client
            .query(
                "SELECT c.id, c.workspace_id, c.type, c.name, c.description, \
                        c.avatar_url, c.creator_id, c.created_at, c.updated_at
                 FROM conversation c
                 LEFT JOIN company co ON co.id = c.workspace_id
                 LEFT JOIN company_member com
                   ON com.company_id = co.id AND com.principal_id = $1
                 INNER JOIN conversation_member cm
                   ON cm.conv_id = c.id
                 WHERE cm.principal_id = $1 AND cm.removed_at IS NULL
                   AND ((co.id IS NULL AND c.workspace_id = $2) OR (co.deleted_at IS NULL
                        AND (c.workspace_id = $2 OR com.principal_id IS NOT NULL)))",
                &[&principal_id, &principal.workspace_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_conversations: {e}")))?;

        if conv_rows.is_empty() {
            return Ok(Vec::new());
        }

        // Collect conversation IDs to batch-load members
        let conv_ids: Vec<String> = conv_rows.iter().map(|r| r.get::<_, String>("id")).collect();

        // Load all active members for these conversations in one query
        // Build a ($1, $2, ...) parameter list for the IN clause
        let placeholders: Vec<String> = (1..=conv_ids.len()).map(|i| format!("${i}")).collect();
        let in_clause = placeholders.join(", ");

        let member_query = format!(
            "SELECT conv_id, principal_id, joined_at
             FROM conversation_member
             WHERE conv_id IN ({in_clause}) AND removed_at IS NULL"
        );
        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = conv_ids
            .iter()
            .map(|id| id as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        let member_rows = client
            .query(&member_query, &params)
            .await
            .map_err(|e| AppError::Internal(format!("list_conversations members: {e}")))?;

        // Group members by conv_id
        let mut members_by_conv: std::collections::HashMap<
            String,
            BTreeMap<String, ConversationMember>,
        > = std::collections::HashMap::new();
        for mr in &member_rows {
            let conv_id: String = mr.get("conv_id");
            let member = row_to_member(mr);
            members_by_conv
                .entry(conv_id)
                .or_default()
                .insert(member.principal_id.clone(), member);
        }

        let conversations = conv_rows
            .iter()
            .map(|row| {
                let id: String = row.get("id");
                let members = members_by_conv.remove(&id).unwrap_or_default();
                row_to_conversation(row, members)
            })
            .collect();

        Ok(conversations)
    }

    /// Return one stable, activity-ordered page for dashboard bootstrap.
    ///
    /// Unlike `list_conversations` + `list_messages`, this performs one page
    /// query and one batched member query regardless of the user's total
    /// conversation count.
    pub async fn list_conversation_bootstrap_page(
        &self,
        principal_id: &str,
        limit: u32,
        after: Option<(DateTime<Utc>, String)>,
    ) -> Result<Vec<crate::ConversationBootstrapEntry>, AppError> {
        let principal = self.get_principal(principal_id).await?;
        let client = self.store.connect().await?;
        let db_limit = i64::from(limit);

        let base = "SELECT c.id, c.workspace_id, c.type, c.name, c.description,
                           c.avatar_url, c.creator_id, c.created_at, c.updated_at,
                           COALESCE(ca.last_activity_at, c.updated_at) AS sort_activity,
                           cp.pinned_at, ar.archived_at, ch.hidden_at,
                           ce.seq AS last_seq, ce.event_id AS last_event_id,
                           ce.sender_id AS last_sender_id, ce.content AS last_content,
                           ce.content_type AS last_content_type,
                           ce.metadata AS last_metadata,
                           ce.client_msg_id AS last_client_msg_id,
                           ce.turn_id AS last_turn_id,
                           ce.created_at AS last_created_at
                    FROM conversation c
                    JOIN conversation_member own_cm
                      ON own_cm.conv_id = c.id
                     AND own_cm.principal_id = $1
                     AND own_cm.removed_at IS NULL
                    LEFT JOIN company co ON co.id = c.workspace_id
                    LEFT JOIN company_member com
                      ON com.company_id = co.id AND com.principal_id = $1
                    LEFT JOIN conversation_activity ca ON ca.conversation_id = c.id
                    LEFT JOIN conversation_events ce
                      ON ce.conversation_id = ca.conversation_id
                     AND ce.seq = ca.last_event_seq
                    LEFT JOIN conversation_pin cp
                      ON cp.conv_id = c.id AND cp.principal_id = $1
                    LEFT JOIN conversation_archive ar
                      ON ar.conv_id = c.id AND ar.principal_id = $1
                    LEFT JOIN conversation_hidden ch
                      ON ch.conv_id = c.id AND ch.principal_id = $1
                    WHERE ((co.id IS NULL AND c.workspace_id = $2)
                       OR (co.deleted_at IS NULL
                           AND (c.workspace_id = $2 OR com.principal_id IS NOT NULL)))
                      AND ch.conv_id IS NULL";

        let rows = if let Some((after_activity, after_id)) = after.as_ref() {
            let sql = format!(
                "{base}
                 AND (COALESCE(ca.last_activity_at, c.updated_at), c.id) < ($3, $4)
                 ORDER BY sort_activity DESC, c.id DESC
                 LIMIT $5"
            );
            client
                .query(
                    &sql,
                    &[
                        &principal_id,
                        &principal.workspace_id,
                        after_activity,
                        after_id,
                        &db_limit,
                    ],
                )
                .await
        } else {
            let sql = format!(
                "{base}
                 ORDER BY sort_activity DESC, c.id DESC
                 LIMIT $3"
            );
            client
                .query(&sql, &[&principal_id, &principal.workspace_id, &db_limit])
                .await
        }
        .map_err(|e| AppError::Internal(format!("list bootstrap conversations: {e}")))?;

        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let conversation_ids: Vec<String> = rows.iter().map(|row| row.get("id")).collect();
        let member_rows = client
            .query(
                "SELECT conv_id, principal_id, joined_at
                 FROM conversation_member
                 WHERE conv_id = ANY($1) AND removed_at IS NULL",
                &[&conversation_ids],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list bootstrap members: {e}")))?;
        let mut members_by_conversation: std::collections::HashMap<
            String,
            BTreeMap<String, ConversationMember>,
        > = std::collections::HashMap::new();
        for row in &member_rows {
            let conversation_id: String = row.get("conv_id");
            let member = row_to_member(row);
            members_by_conversation
                .entry(conversation_id)
                .or_default()
                .insert(member.principal_id.clone(), member);
        }

        Ok(rows
            .iter()
            .map(|row| {
                let id: String = row.get("id");
                let workspace_id: String = row.get("workspace_id");
                let last_seq: Option<i64> = row.get("last_seq");
                let last_message = last_seq.map(|server_seq| Message {
                    id: row.get("last_event_id"),
                    workspace_id: workspace_id.clone(),
                    conversation_id: id.clone(),
                    sender_id: row.get("last_sender_id"),
                    content: row
                        .get::<_, Option<String>>("last_content")
                        .unwrap_or_default(),
                    content_type: row.get("last_content_type"),
                    metadata: row.get("last_metadata"),
                    edited_at: None,
                    edited_by: None,
                    server_seq: server_seq as u64,
                    idempotency_key: row
                        .get::<_, Option<String>>("last_client_msg_id")
                        .or_else(|| row.get("last_turn_id"))
                        .unwrap_or_default(),
                    created_at: row.get("last_created_at"),
                });
                let members = members_by_conversation.remove(&id).unwrap_or_default();
                crate::ConversationBootstrapEntry {
                    conversation: row_to_conversation(row, members),
                    last_message,
                    last_activity_at: row.get("sort_activity"),
                    pinned_at: row.get("pinned_at"),
                    archived_at: row.get("archived_at"),
                    hidden_at: row.get("hidden_at"),
                }
            })
            .collect())
    }

    /// List conversation pins that are still visible in the principal's console snapshot.
    pub async fn list_visible_conversation_pins(
        &self,
        principal_id: &str,
    ) -> Result<Vec<crate::PinnedConversation>, AppError> {
        let visible_conversation_ids: HashSet<String> = self
            .list_conversations(principal_id)
            .await?
            .into_iter()
            .map(|conversation| conversation.id)
            .collect();

        if visible_conversation_ids.is_empty() {
            return Ok(Vec::new());
        }

        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT conv_id, pinned_at
                 FROM conversation_pin
                 WHERE principal_id = $1
                 ORDER BY pinned_at DESC",
                &[&principal_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_visible_conversation_pins: {e}")))?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let conversation_id: String = row.get("conv_id");
                if visible_conversation_ids.contains(&conversation_id) {
                    Some(crate::PinnedConversation {
                        conversation_id,
                        pinned_at: row.get("pinned_at"),
                    })
                } else {
                    None
                }
            })
            .collect())
    }

    /// Pin a conversation for a principal if it is visible in their console snapshot.
    pub async fn pin_conversation(
        &self,
        principal_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        let is_visible = self
            .list_conversations(principal_id)
            .await?
            .into_iter()
            .any(|conversation| conversation.id == conversation_id);
        if !is_visible {
            return Err(AppError::Forbidden(
                "conversation is not visible to principal".into(),
            ));
        }

        let client = self.store.connect().await?;
        client
            .execute(
                "INSERT INTO conversation_pin (principal_id, conv_id)
                 VALUES ($1, $2)
                 ON CONFLICT (principal_id, conv_id) DO NOTHING",
                &[&principal_id, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("pin_conversation: {e}")))?;

        Ok(())
    }

    /// Remove only this principal's pin row, without requiring current conversation access.
    pub async fn unpin_conversation(
        &self,
        principal_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        client
            .execute(
                "DELETE FROM conversation_pin
                 WHERE principal_id = $1 AND conv_id = $2",
                &[&principal_id, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("unpin_conversation: {e}")))?;

        Ok(())
    }

    /// List user-scoped archives that are still visible in the console snapshot.
    pub async fn list_visible_conversation_archives(
        &self,
        principal_id: &str,
    ) -> Result<Vec<crate::ArchivedConversation>, AppError> {
        let visible_conversation_ids: HashSet<String> = self
            .list_conversations(principal_id)
            .await?
            .into_iter()
            .map(|conversation| conversation.id)
            .collect();

        if visible_conversation_ids.is_empty() {
            return Ok(Vec::new());
        }

        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT conv_id, archived_at
                 FROM conversation_archive
                 WHERE principal_id = $1
                 ORDER BY archived_at DESC",
                &[&principal_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_visible_conversation_archives: {e}")))?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let conversation_id: String = row.get("conv_id");
                if visible_conversation_ids.contains(&conversation_id) {
                    Some(crate::ArchivedConversation {
                        conversation_id,
                        archived_at: row.get("archived_at"),
                    })
                } else {
                    None
                }
            })
            .collect())
    }

    /// List hidden sessions that are still visible to this user. Hidden is a
    /// personal view preference, not a conversation or runtime lifecycle.
    pub async fn list_visible_hidden_conversations(
        &self,
        principal_id: &str,
    ) -> Result<Vec<crate::HiddenConversation>, AppError> {
        let principal = self.get_principal(principal_id).await?;
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT ch.conv_id, ch.hidden_at
                 FROM conversation_hidden ch
                 JOIN conversation c ON c.id = ch.conv_id
                 JOIN conversation_member own_cm
                   ON own_cm.conv_id = c.id
                  AND own_cm.principal_id = $1
                  AND own_cm.removed_at IS NULL
                 LEFT JOIN company co ON co.id = c.workspace_id
                 LEFT JOIN company_member com
                   ON com.company_id = co.id AND com.principal_id = $1
                 WHERE ch.principal_id = $1
                   AND ((co.id IS NULL AND c.workspace_id = $2)
                     OR (co.deleted_at IS NULL
                       AND (c.workspace_id = $2 OR com.principal_id IS NOT NULL)))
                 ORDER BY ch.hidden_at DESC",
                &[&principal_id, &principal.workspace_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_visible_hidden_conversations: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|row| crate::HiddenConversation {
                conversation_id: row.get("conv_id"),
                hidden_at: row.get("hidden_at"),
            })
            .collect())
    }

    /// Archive a visible conversation for one user. Archiving also removes
    /// that user's pin so the conversation has one unambiguous sidebar home.
    pub async fn archive_conversation(
        &self,
        principal_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        let is_visible = self
            .list_conversations(principal_id)
            .await?
            .into_iter()
            .any(|conversation| conversation.id == conversation_id);
        if !is_visible {
            return Err(AppError::Forbidden(
                "conversation is not visible to principal".into(),
            ));
        }

        let client = self.store.connect().await?;
        client
            .execute(
                "WITH removed_pin AS (
                   DELETE FROM conversation_pin
                   WHERE principal_id = $1 AND conv_id = $2
                 ), removed_hidden AS (
                   DELETE FROM conversation_hidden
                   WHERE principal_id = $1 AND conv_id = $2
                 )
                 INSERT INTO conversation_archive (principal_id, conv_id)
                 VALUES ($1, $2)
                 ON CONFLICT (principal_id, conv_id) DO NOTHING",
                &[&principal_id, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("archive_conversation: {e}")))?;

        Ok(())
    }

    /// Remove only this user's archive marker; the shared conversation and
    /// its runtime are left untouched.
    pub async fn unarchive_conversation(
        &self,
        principal_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        client
            .execute(
                "DELETE FROM conversation_archive
                 WHERE principal_id = $1 AND conv_id = $2",
                &[&principal_id, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("unarchive_conversation: {e}")))?;

        Ok(())
    }

    /// Hide one direct Agent session for this user. This leaves its Agent and
    /// messages untouched, but removes pin/archive markers so Restore returns
    /// it to the ordinary direct-message list.
    pub async fn hide_agent_session(
        &self,
        principal_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        let conversations = self.list_conversations(principal_id).await?;
        let conversation = conversations
            .into_iter()
            .find(|conversation| conversation.id == conversation_id)
            .ok_or_else(|| {
                AppError::Forbidden("conversation is not visible to principal".into())
            })?;
        if conversation.conversation_type != ConversationType::Direct {
            return Err(AppError::Validation(
                "only direct Agent sessions can be hidden".into(),
            ));
        }
        let peer_id = conversation
            .members
            .keys()
            .find(|member_id| member_id.as_str() != principal_id)
            .ok_or_else(|| AppError::Validation("direct session has no peer".into()))?;
        if self.get_principal(peer_id).await?.principal_type != PrincipalType::Agent {
            return Err(AppError::Validation(
                "only direct Agent sessions can be hidden".into(),
            ));
        }

        let client = self.store.connect().await?;
        client
            .execute(
                "WITH removed_pin AS (
                   DELETE FROM conversation_pin
                   WHERE principal_id = $1 AND conv_id = $2
                 ), removed_archive AS (
                   DELETE FROM conversation_archive
                   WHERE principal_id = $1 AND conv_id = $2
                 )
                 INSERT INTO conversation_hidden (principal_id, conv_id)
                 VALUES ($1, $2)
                 ON CONFLICT (principal_id, conv_id) DO NOTHING",
                &[&principal_id, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("hide_agent_session: {e}")))?;
        Ok(())
    }

    pub async fn restore_hidden_agent_session(
        &self,
        principal_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        client
            .execute(
                "DELETE FROM conversation_hidden
                 WHERE principal_id = $1 AND conv_id = $2",
                &[&principal_id, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("restore_hidden_agent_session: {e}")))?;
        Ok(())
    }

    // ── Conversation writes (Phase 2B) ─────────────────────────────────

    /// Create a direct (1-on-1) conversation in the database.
    ///
    /// Validates both actor and peer exist and are in the same workspace
    /// (unless an explicit workspace_id override is provided).
    /// Uses a SQL query to check for existing direct conversations,
    /// replacing the in-memory `direct_index`.
    pub async fn create_direct_conversation(
        &self,
        request: crate::CreateDirectConversationRequest,
    ) -> Result<Conversation, AppError> {
        if request.actor_id == request.peer_principal_id {
            return Err(AppError::Validation(
                "cannot create a direct conversation with yourself".into(),
            ));
        }

        let actor = self.get_principal(&request.actor_id).await?;
        let peer = self.get_principal(&request.peer_principal_id).await?;

        let ws_id = request
            .workspace_id
            .filter(|w| !w.trim().is_empty())
            .unwrap_or_else(|| actor.workspace_id.clone());

        if !self.principal_can_access_workspace(&actor, &ws_id).await?
            || !self.principal_can_access_workspace(&peer, &ws_id).await?
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let client = self.store.connect().await?;

        // Check if a direct conversation already exists between these two principals
        let existing = client
            .query_opt(
                "SELECT c.id, c.workspace_id, c.type, c.name, c.description, c.avatar_url, \
                        c.creator_id, c.created_at, c.updated_at
                 FROM conversation c
                 WHERE c.type = 'direct' AND c.workspace_id = $1
                 AND EXISTS (SELECT 1 FROM conversation_member WHERE conv_id = c.id AND principal_id = $2 AND removed_at IS NULL)
                 AND EXISTS (SELECT 1 FROM conversation_member WHERE conv_id = c.id AND principal_id = $3 AND removed_at IS NULL)",
                &[&ws_id, &actor.id, &peer.id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("create_direct_conversation lookup: {e}")))?;

        if let Some(row) = existing {
            // Load members for the existing conversation
            let conv_id: String = row.get("id");
            let member_rows = client
                .query(
                    "SELECT principal_id, joined_at
                     FROM conversation_member
                     WHERE conv_id = $1 AND removed_at IS NULL",
                    &[&conv_id],
                )
                .await
                .map_err(|e| {
                    AppError::Internal(format!("create_direct_conversation members: {e}"))
                })?;
            let members = rows_to_members(&member_rows);
            return Ok(row_to_conversation(&row, members));
        }

        // Build the full member set before registration. Even agent-to-agent
        // conversations include the workspace's human as a real member.
        let timestamp = now();
        let conversation_id = new_id();
        let type_str = "direct";
        let mut members = BTreeMap::from([
            (
                actor.id.clone(),
                ConversationMember {
                    principal_id: actor.id.clone(),
                    joined_at: timestamp,
                },
            ),
            (
                peer.id.clone(),
                ConversationMember {
                    principal_id: peer.id.clone(),
                    joined_at: timestamp,
                },
            ),
        ]);
        if !matches!(actor.principal_type, PrincipalType::Human)
            && !matches!(peer.principal_type, PrincipalType::Human)
        {
            let human = self.human_for_conversation_workspace(&ws_id).await?;
            if !self.principal_can_access_workspace(&human, &ws_id).await? {
                return Err(AppError::Forbidden("cross-workspace access denied".into()));
            }
            members.insert(
                human.id.clone(),
                ConversationMember {
                    principal_id: human.id,
                    joined_at: timestamp,
                },
            );
        }

        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
                 VALUES ($1, $2, $3, NULL, $4, $5, $6)",
                &[&conversation_id, &ws_id, &type_str, &actor.id, &timestamp, &timestamp],
            )
            .await
            .map_err(|e| AppError::Internal(format!("create_direct_conversation insert: {e}")))?;

        for member in members.values() {
            client
                .execute(
                    "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                     VALUES ($1, $2, $3)",
                    &[&conversation_id, &member.principal_id, &timestamp],
                )
                .await
                .map_err(|e| {
                    AppError::Internal(format!("create_direct_conversation member insert: {e}"))
                })?;
        }

        // One post-membership touch fans the completed conversation out to
        // all members through the durable sync trigger. Keeping this outside
        // the member loop avoids O(n²) change-log growth for large groups.
        client
            .execute(
                "UPDATE conversation SET updated_at = updated_at WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("create_direct_conversation sync: {e}")))?;

        let actor_id = actor.id.clone();
        let peer_id = peer.id.clone();

        // Audit log
        self.record_audit(
            &ws_id,
            &actor_id,
            "conversation.direct_created",
            "conversation",
            &conversation_id,
            serde_json::json!({"peer_principal_id": &peer_id}),
        )
        .await?;

        Ok(Conversation {
            id: conversation_id,
            workspace_id: ws_id,
            conversation_type: ConversationType::Direct,
            name: None,
            description: None,
            avatar_url: None,
            creator_id: actor_id,
            created_at: timestamp,
            updated_at: timestamp,
            members,
        })
    }

    /// Create a group conversation in the database.
    ///
    /// Validates name is non-empty, actor exists, and all members are
    /// in the same workspace (unless an explicit workspace_id override is provided).
    pub async fn create_group(
        &self,
        request: crate::CreateGroupRequest,
    ) -> Result<Conversation, AppError> {
        if request.name.trim().is_empty() {
            return Err(AppError::Validation("group name is required".into()));
        }

        let actor = self.get_principal(&request.actor_id).await?;
        let ws_id = request
            .workspace_id
            .filter(|w| !w.trim().is_empty())
            .unwrap_or_else(|| actor.workspace_id.clone());
        if !self.principal_can_access_workspace(&actor, &ws_id).await? {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let timestamp = now();
        let conversation_id = new_id();
        let type_str = "group";

        // Build the membership set. Ownership is represented by creator_id.
        let mut members = BTreeMap::new();
        members.insert(
            actor.id.clone(),
            ConversationMember {
                principal_id: actor.id.clone(),
                joined_at: timestamp,
            },
        );

        let mut has_human = matches!(actor.principal_type, PrincipalType::Human);
        for member_id in &request.member_ids {
            if *member_id == actor.id || members.contains_key(member_id) {
                continue;
            }
            let member = self.get_principal(member_id).await?;
            if !self.principal_can_access_workspace(&member, &ws_id).await? {
                return Err(AppError::Forbidden("cross-workspace access denied".into()));
            }
            has_human |= matches!(member.principal_type, PrincipalType::Human);
            members.insert(
                member.id.clone(),
                ConversationMember {
                    principal_id: member.id.clone(),
                    joined_at: timestamp,
                },
            );
        }

        if !has_human {
            let human = self.human_for_conversation_workspace(&ws_id).await?;
            if !self.principal_can_access_workspace(&human, &ws_id).await? {
                return Err(AppError::Forbidden("cross-workspace access denied".into()));
            }
            members.insert(
                human.id.clone(),
                ConversationMember {
                    principal_id: human.id,
                    joined_at: timestamp,
                },
            );
        }

        let client = self.store.connect().await?;

        client
            .execute(
                "INSERT INTO conversation (id, workspace_id, type, name, description, avatar_url, creator_id, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                &[
                    &conversation_id,
                    &ws_id,
                    &type_str,
                    &Some(&request.name),
                    &request.description,
                    &request.avatar_url,
                    &actor.id,
                    &timestamp,
                    &timestamp,
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("create_group insert: {e}")))?;

        // Insert all members
        for member in members.values() {
            client
                .execute(
                    "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                     VALUES ($1, $2, $3)",
                    &[&conversation_id, &member.principal_id, &timestamp],
                )
                .await
                .map_err(|e| AppError::Internal(format!("create_group member insert: {e}")))?;
        }

        client
            .execute(
                "UPDATE conversation SET updated_at = updated_at WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("create_group sync: {e}")))?;

        // Audit log
        self.record_audit(
            &ws_id,
            &actor.id,
            "group.created",
            "conversation",
            &conversation_id,
            serde_json::json!({"name": request.name}),
        )
        .await?;

        Ok(Conversation {
            id: conversation_id,
            workspace_id: ws_id,
            conversation_type: ConversationType::Group,
            name: Some(request.name),
            description: request.description,
            avatar_url: request.avatar_url,
            creator_id: actor.id,
            created_at: timestamp,
            updated_at: timestamp,
            members,
        })
    }

    /// Update a group conversation's metadata (name, description, avatar_url).
    ///
    /// Validates that the actor is a member with manage permissions.
    pub async fn update_group(
        &self,
        conversation_id: &str,
        request: crate::UpdateGroupRequest,
    ) -> Result<Conversation, AppError> {
        let actor = self.get_principal(&request.actor_id).await?;

        let client = self.store.connect().await?;

        // Fetch conversation
        let row = client
            .query_opt(
                "SELECT id, workspace_id, type, name, description, avatar_url, \
                        creator_id, created_at, updated_at
                 FROM conversation WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("update_group fetch: {e}")))?
            .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;

        let ctype: String = row.get("type");
        let conv_ws_id: String = row.get("workspace_id");
        if ctype != "group" {
            return Err(AppError::Validation("conversation is not a group".into()));
        }
        if !self
            .principal_can_access_workspace(&actor, &conv_ws_id)
            .await?
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        // Every active group member may update group metadata.
        client
            .query_opt(
                "SELECT 1 FROM conversation_member
                 WHERE conv_id = $1 AND principal_id = $2 AND removed_at IS NULL",
                &[&conversation_id, &actor.id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("update_group check member: {e}")))?
            .ok_or_else(|| AppError::Forbidden("not a group member".into()))?;

        // Update once so both the row and its durable sync notification have
        // one canonical version even when several fields change together.
        let timestamp = now();
        client
            .execute(
                "UPDATE conversation
                 SET name = COALESCE($1, name),
                     description = COALESCE($2, description),
                     avatar_url = COALESCE($3, avatar_url),
                     updated_at = $4
                 WHERE id = $5",
                &[
                    &request.name,
                    &request.description,
                    &request.avatar_url,
                    &timestamp,
                    &conversation_id,
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("update_group: {e}")))?;

        // Audit log
        self.record_audit(
            &conv_ws_id,
            &actor.id,
            "group.updated",
            "conversation",
            conversation_id,
            serde_json::json!({}),
        )
        .await?;

        // Fetch and return the updated conversation
        self.get_conversation(conversation_id).await
    }

    /// Add members to a group conversation.
    ///
    /// Validates that the actor has manage permissions and all new members
    /// are in the same workspace.
    pub async fn add_group_members(
        &self,
        conversation_id: &str,
        request: crate::AddGroupMembersRequest,
    ) -> Result<Conversation, AppError> {
        let actor = self.get_principal(&request.actor_id).await?;

        let client = self.store.connect().await?;

        // Fetch conversation type
        let conv_row = client
            .query_opt(
                "SELECT type, workspace_id FROM conversation WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("add_group_members fetch: {e}")))?
            .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;

        let ctype: String = conv_row.get("type");
        let conv_ws_id: String = conv_row.get("workspace_id");
        if ctype != "group" {
            return Err(AppError::Validation("conversation is not a group".into()));
        }
        if !self
            .principal_can_access_workspace(&actor, &conv_ws_id)
            .await?
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        // Any active group member can invite new members (Slack/Discord semantics).
        // Verify the actor is an active member of this group.
        let _member_row = client
            .query_opt(
                "SELECT 1 FROM conversation_member
                 WHERE conv_id = $1 AND principal_id = $2 AND removed_at IS NULL",
                &[&conversation_id, &actor.id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("add_group_members check member: {e}")))?
            .ok_or_else(|| AppError::Forbidden("not a group member".into()))?;

        // Get existing member IDs
        let existing_rows = client
            .query(
                "SELECT principal_id FROM conversation_member
                 WHERE conv_id = $1 AND removed_at IS NULL",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("add_group_members existing: {e}")))?;
        let existing_ids: Vec<String> = existing_rows
            .iter()
            .map(|r| r.get("principal_id"))
            .collect();

        let timestamp = now();
        let mut added_ids = Vec::new();
        for member_id in &request.member_ids {
            if existing_ids.contains(member_id) {
                continue;
            }
            let member = self.get_principal(member_id).await?;
            if !self
                .principal_can_access_workspace(&member, &conv_ws_id)
                .await?
            {
                return Err(AppError::Forbidden("cross-workspace access denied".into()));
            }
            client
                .execute(
                    "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                     VALUES ($1, $2, $3)
                     ON CONFLICT (conv_id, principal_id) DO UPDATE
                     SET removed_at = NULL",
                    &[&conversation_id, &member.id, &timestamp],
                )
                .await
                .map_err(|e| AppError::Internal(format!("add_group_members insert: {e}")))?;
            added_ids.push(member.id);
        }

        // Audit log (only if members were actually added)
        if !added_ids.is_empty() {
            client
                .execute(
                    "UPDATE conversation SET updated_at = $1 WHERE id = $2",
                    &[&timestamp, &conversation_id],
                )
                .await
                .map_err(|e| {
                    AppError::Internal(format!("add_group_members update conversation: {e}"))
                })?;
            self.record_audit(
                &conv_ws_id,
                &actor.id,
                "group.members_added",
                "conversation",
                conversation_id,
                serde_json::json!({"member_ids": added_ids}),
            )
            .await?;
        }

        // Return updated conversation
        self.get_conversation(conversation_id).await
    }

    /// Remove a member from a group conversation.
    ///
    /// Validates that the actor has permission to remove the target
    /// (owners can't be removed, etc.).
    pub async fn remove_group_member(
        &self,
        conversation_id: &str,
        actor_id: &str,
        target_id: &str,
    ) -> Result<Conversation, AppError> {
        let actor = self.get_principal(actor_id).await?;
        let target = self.get_principal(target_id).await?;
        if matches!(target.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "human members cannot be removed from conversations".into(),
            ));
        }

        let client = self.store.connect().await?;

        // Fetch conversation type
        let conv_row = client
            .query_opt(
                "SELECT type, workspace_id, creator_id FROM conversation WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("remove_group_member fetch: {e}")))?
            .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;

        let ctype: String = conv_row.get("type");
        let conv_ws_id: String = conv_row.get("workspace_id");
        let creator_id: String = conv_row.get("creator_id");
        if ctype != "group" {
            return Err(AppError::Validation("conversation is not a group".into()));
        }
        if !self
            .principal_can_access_workspace(&actor, &conv_ws_id)
            .await?
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        // Every active group member may remove another member.
        client
            .query_opt(
                "SELECT 1 FROM conversation_member
                 WHERE conv_id = $1 AND principal_id = $2 AND removed_at IS NULL",
                &[&conversation_id, &actor.id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("remove_group_member actor check: {e}")))?
            .ok_or_else(|| AppError::Forbidden("not a group member".into()))?;

        client
            .query_opt(
                "SELECT 1 FROM conversation_member
                 WHERE conv_id = $1 AND principal_id = $2 AND removed_at IS NULL",
                &[&conversation_id, &target_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("remove_group_member target check: {e}")))?
            .ok_or_else(|| AppError::NotFound("target is not a group member".into()))?;
        if target_id == creator_id {
            return Err(AppError::Forbidden(
                "cannot remove conversation creator".into(),
            ));
        }

        // Soft-delete: set removed_at
        let now_ts = now();
        client
            .execute(
                "UPDATE conversation_member SET removed_at = $1
                 WHERE conv_id = $2 AND principal_id = $3 AND removed_at IS NULL",
                &[&now_ts, &conversation_id, &target_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("remove_group_member update: {e}")))?;

        // Update conversation updated_at
        client
            .execute(
                "UPDATE conversation SET updated_at = $1 WHERE id = $2",
                &[&now_ts, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("remove_group_member update conv: {e}")))?;

        // Audit log
        self.record_audit(
            &conv_ws_id,
            &actor.id,
            "group.member_removed",
            "conversation",
            conversation_id,
            serde_json::json!({"member_id": target_id}),
        )
        .await?;

        // Return updated conversation
        self.get_conversation(conversation_id).await
    }

    /// Delete a conversation and its events from the database.
    ///
    /// Only human conversation members can delete conversations.
    pub async fn delete_conversation(
        &self,
        actor_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        let actor = self.get_principal(actor_id).await?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "only humans can delete conversations".into(),
            ));
        }

        // Human control-plane authority does not bypass conversation membership.
        let conv = self.get_conversation(conversation_id).await?;
        if !conv.members.contains_key(actor_id) {
            return Err(AppError::Forbidden("not a conversation member".into()));
        }

        let client = self.store.connect().await?;

        // Delete conversation events first (no CASCADE)
        client
            .execute(
                "DELETE FROM conversation_events WHERE conversation_id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("delete_conversation events: {e}")))?;

        // Delete conversation (CASCADE handles conversation_member)
        client
            .execute(
                "DELETE FROM conversation WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("delete_conversation: {e}")))?;

        // Audit log
        self.record_audit(
            &conv.workspace_id,
            &actor.id,
            "conversation.deleted",
            "conversation",
            conversation_id,
            serde_json::json!({}),
        )
        .await?;

        Ok(())
    }
}
