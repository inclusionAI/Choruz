use choruz_common::{AppError, new_id, now};
use choruz_domain::{Message, PrincipalType};
use choruz_store::conversation_events::THREAD_FLAG_SQL;

use super::DbService;
use crate::ConversationUnread;

impl DbService {
    // ── Message reads (Phase 1D) ────────────────────────────────────────

    /// List messages for a conversation from the database.
    ///
    /// Queries `conversation_events` via `EventStore::list_messages_by_conversation`
    /// and converts rows to `choruz_domain::Message`.
    pub async fn list_messages(
        &self,
        conversation_id: &str,
        limit: Option<u64>,
        since_seq: Option<u64>,
    ) -> Result<Vec<Message>, AppError> {
        let db_limit = limit.unwrap_or(200) as i64;
        let db_since_seq = since_seq.map(|s| s as i64);

        // Resolve workspace_id for the message construction.
        // Try DB first; fall back to empty string if conversation not found
        // (e.g. deleted conversation whose messages still exist).
        let workspace_id = self
            .get_conversation(conversation_id)
            .await
            .map(|c| c.workspace_id)
            .unwrap_or_default();

        let mut events = self
            .store
            .list_messages_by_conversation(conversation_id, db_limit, db_since_seq)
            .await?;

        // An initial page is newest-first; incremental pages already arrive
        // oldest-unseen-first to guarantee gapless cursor advancement.
        if since_seq.is_none() {
            events.reverse();
        }

        let msgs = events
            .into_iter()
            .map(|e| Message {
                id: e.event_id,
                workspace_id: workspace_id.clone(),
                conversation_id: e.conversation_id,
                sender_id: e.sender_id,
                content: e.content.unwrap_or_default(),
                content_type: e.content_type,
                metadata: e.metadata,
                edited_at: None,
                edited_by: None,
                server_seq: e.seq as u64,
                idempotency_key: e.client_msg_id.or(e.turn_id).unwrap_or_default(),
                created_at: e.created_at,
            })
            .collect();

        Ok(msgs)
    }

    /// Read a stable page in either direction without offset pagination.
    /// `after_seq` is deliberately ordered ASC so a burst larger than the
    /// page size cannot skip the oldest unseen events.
    pub async fn list_message_page(
        &self,
        conversation_id: &str,
        limit: u64,
        before_seq: Option<u64>,
        after_seq: Option<u64>,
    ) -> Result<crate::MessagePage, AppError> {
        if before_seq.is_some() && after_seq.is_some() {
            return Err(AppError::Validation(
                "before_seq and after_seq are mutually exclusive".into(),
            ));
        }

        let limit = limit.clamp(1, 100);
        let workspace_id = self.get_conversation(conversation_id).await?.workspace_id;
        let client = self.store.connect().await?;
        let fetch_limit = i64::try_from(limit.saturating_add(1))
            .map_err(|_| AppError::Validation("message page limit is too large".into()))?;
        let before = before_seq
            .map(i64::try_from)
            .transpose()
            .map_err(|_| AppError::Validation("before_seq is too large".into()))?;
        let after = after_seq
            .map(i64::try_from)
            .transpose()
            .map_err(|_| AppError::Validation("after_seq is too large".into()))?;

        let select = "SELECT conversation_id, seq, event_id, sender_id, content,
                             content_type, metadata, client_msg_id, turn_id, created_at
                      FROM conversation_events
                      WHERE conversation_id = $1
                        AND event_type IN ('message', 'message.created', 'reply')";
        let (rows, direction) = if let Some(cursor) = before {
            let sql = format!("{select} AND seq < $3 ORDER BY seq DESC LIMIT $2");
            (
                client
                    .query(&sql, &[&conversation_id, &fetch_limit, &cursor])
                    .await,
                "before",
            )
        } else if let Some(cursor) = after {
            let sql = format!("{select} AND seq > $3 ORDER BY seq ASC LIMIT $2");
            (
                client
                    .query(&sql, &[&conversation_id, &fetch_limit, &cursor])
                    .await,
                "after",
            )
        } else {
            let sql = format!("{select} ORDER BY seq DESC LIMIT $2");
            (
                client.query(&sql, &[&conversation_id, &fetch_limit]).await,
                "latest",
            )
        };
        let mut rows =
            rows.map_err(|error| AppError::Internal(format!("list message page: {error}")))?;
        let has_more = rows.len() > limit as usize;
        if has_more {
            rows.truncate(limit as usize);
        }
        if direction != "after" {
            rows.reverse();
        }

        let messages: Vec<Message> = rows
            .into_iter()
            .map(|row| Message {
                id: row.get("event_id"),
                workspace_id: workspace_id.clone(),
                conversation_id: row.get("conversation_id"),
                sender_id: row.get("sender_id"),
                content: row.get::<_, Option<String>>("content").unwrap_or_default(),
                content_type: row.get("content_type"),
                metadata: row.get("metadata"),
                edited_at: None,
                edited_by: None,
                server_seq: row.get::<_, i64>("seq") as u64,
                idempotency_key: row
                    .get::<_, Option<String>>("client_msg_id")
                    .or_else(|| row.get("turn_id"))
                    .unwrap_or_default(),
                created_at: row.get("created_at"),
            })
            .collect();
        let next_cursor = has_more.then(|| {
            if direction == "after" {
                messages.last()
            } else {
                messages.first()
            }
            .expect("has_more requires a page item")
            .server_seq
        });

        Ok(crate::MessagePage {
            messages,
            direction: direction.into(),
            has_more,
            next_cursor,
        })
    }

    /// Fetch ONE message by id, scoped to a conversation. Backs the
    /// quote-reply preview: when a client renders a reply whose target is
    /// outside its locally loaded history window, it fetches the original
    /// here instead of showing a "not loaded" placeholder (WeChat/Feishu
    /// behavior, but resolved live from the DB so edits stay current
    /// rather than snapshotting content into the reply).
    ///
    /// Uniform NotFound for nonexistent ids AND ids that live in another
    /// conversation — the conversation_id scope keeps this from acting as
    /// a cross-conversation message-existence oracle (same rule as the
    /// thread read paths).
    pub async fn get_message(
        &self,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<Message, AppError> {
        let workspace_id = self
            .get_conversation(conversation_id)
            .await
            .map(|c| c.workspace_id)
            .unwrap_or_default();

        let client = self.store.connect().await?;
        let row = client
            .query_opt(
                "SELECT conversation_id, seq, event_id, event_type, sender_id, \
                        content, content_type, metadata, client_msg_id, turn_id, \
                        reply_event_id, created_at \
                 FROM conversation_events \
                 WHERE event_id = $1 AND conversation_id = $2 \
                   AND event_type IN ('message', 'message.created', 'reply')",
                &[&message_id, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("get_message: {e}")))?
            .ok_or_else(|| AppError::NotFound("message not found".into()))?;
        Ok(event_row_to_message(&row, &workspace_id))
    }

    /// List all message events for a conversation, for full-history exports.
    pub async fn list_all_messages(&self, conversation_id: &str) -> Result<Vec<Message>, AppError> {
        let workspace_id = self
            .get_conversation(conversation_id)
            .await
            .map(|c| c.workspace_id)
            .unwrap_or_default();

        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT conversation_id, seq, event_id, sender_id, content, content_type,
                        metadata, client_msg_id, turn_id, created_at
                 FROM conversation_events
                 WHERE conversation_id = $1
                   AND event_type IN ('message', 'message.created', 'reply')
                 ORDER BY seq ASC",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_all_messages: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|row| Message {
                id: row.get("event_id"),
                workspace_id: workspace_id.clone(),
                conversation_id: row.get("conversation_id"),
                sender_id: row.get("sender_id"),
                content: row.get::<_, Option<String>>("content").unwrap_or_default(),
                content_type: row.get("content_type"),
                metadata: row.get("metadata"),
                edited_at: None,
                edited_by: None,
                server_seq: row.get::<_, i64>("seq") as u64,
                idempotency_key: row
                    .get::<_, Option<String>>("client_msg_id")
                    .or_else(|| row.get::<_, Option<String>>("turn_id"))
                    .unwrap_or_default(),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    // ── Message writes (Phase 2D) ──────────────────────────────────────

    /// Send a message to a conversation, writing directly to the database.
    ///
    /// This is the DB-first replacement for `ChatApp::send_message` +
    /// `dual_write_shadow`. It:
    /// 1. Validates actor and conversation membership
    /// 2. Auto-joins agents as members if not already joined
    /// 3. Checks idempotency via DB (client_msg_id UNIQUE)
    /// 4. INSERTs into conversation_events + event_outbox in one transaction
    /// 5. Returns Message with DB-assigned seq
    pub async fn send_message(
        &self,
        request: crate::SendMessageRequest,
    ) -> Result<Message, AppError> {
        if request.content.trim().is_empty() {
            return Err(AppError::Validation("message content is required".into()));
        }
        if request.idempotency_key.trim().is_empty() {
            return Err(AppError::Validation("idempotency_key is required".into()));
        }

        // 1. Validate actor and conversation
        let actor = self.get_principal(&request.actor_id).await?;
        let conv = self.get_conversation(&request.conversation_id).await?;

        // Check workspace access
        if !self
            .principal_can_access_workspace(&actor, &conv.workspace_id)
            .await?
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        // 2. Check membership; auto-join agents
        let mut client = self.store.connect().await?;

        if !conv.members.contains_key(&actor.id) {
            if matches!(actor.principal_type, PrincipalType::Agent) {
                // Auto-join agent as member
                let timestamp = now();
                client
                    .execute(
                        "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                         VALUES ($1, $2, $3)
                         ON CONFLICT (conv_id, principal_id) DO UPDATE
                         SET removed_at = NULL",
                        &[&conv.id, &actor.id, &timestamp],
                    )
                    .await
                    .map_err(|e| AppError::Internal(format!("send_message auto-join: {e}")))?;
                tracing::info!(
                    actor_id = %actor.id,
                    conversation_id = %conv.id,
                    "DB auto-joined agent as conversation member"
                );
            } else {
                return Err(AppError::Forbidden(
                    "sender is not a conversation member".into(),
                ));
            }
        }

        // 3. Idempotency check via DB
        let client_msg_id = request.idempotency_key.clone();
        if let Some(existing) = self
            .store
            .find_event_by_client_msg_id(&client_msg_id)
            .await?
        {
            // Return the existing message — dedup
            return Ok(Message {
                id: existing.event_id,
                workspace_id: conv.workspace_id,
                conversation_id: existing.conversation_id,
                sender_id: existing.sender_id,
                content: existing.content.unwrap_or_default(),
                content_type: existing.content_type,
                metadata: existing.metadata,
                edited_at: None,
                edited_by: None,
                server_seq: existing.seq as u64,
                idempotency_key: existing.client_msg_id.unwrap_or_default(),
                created_at: existing.created_at,
            });
        }

        // 4. INSERT conversation_events + event_outbox in one transaction
        let message_id = new_id();
        let event_type = "message".to_string();
        let content: Option<String> = Some(request.content.clone());
        let turn_id: Option<String> = None;
        let reply_event_id: Option<String> = request
            .metadata
            .get("reply_to_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // Thread discriminators, parsed
        // by the shared choruz-store helper so every write path and the
        // router probe agree on the semantics. Legacy quote-replies have
        // reply_to_id but no thread flag and keep today's behavior.
        let thread_flags = choruz_store::ThreadFlags::from_metadata(&request.metadata);
        let is_thread_reply = thread_flags.is_thread_reply;
        let is_broadcast = thread_flags.is_broadcast;
        if is_thread_reply && reply_event_id.is_none() {
            return Err(AppError::Validation(
                "threaded reply requires metadata.reply_to_id".into(),
            ));
        }
        let client_msg_id_opt: Option<String> = Some(client_msg_id.clone());
        // Persist the FE trace id into row metadata so the rest of the
        // system — outbox payloads, pipeline, writer, webhooks — sees the
        // same correlator without needing signature changes. `None` when
        // the originating request didn't carry `x-trace-id`.
        let trace_id_opt: Option<String> = request.trace_id.clone();
        let row_metadata = {
            let mut meta = request.metadata.clone();
            if let Some(ref tid) = trace_id_opt {
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert(
                        "trace_id".to_string(),
                        serde_json::Value::String(tid.clone()),
                    );
                } else {
                    meta = serde_json::json!({ "trace_id": tid });
                }
            }
            meta
        };

        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("send_message begin tx: {e}")))?;

        // Serialize concurrent writers targeting the same conversation so the
        // COALESCE(MAX(seq), 0) + 1 allocation below cannot race to the same
        // value and collide on the (conversation_id, seq) unique constraint
        // (see Bug M). Different conversations remain parallel.
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&conv.id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("send_message advisory lock: {e}")))?;

        // ── Thread root canonicalization (flat threads, Slack semantics).
        // Only the threaded path pays the lookup; legacy quote-replies
        // skip it entirely. See `canonicalize_thread_root`.
        let reply_event_id: Option<String> = if is_thread_reply {
            let target_id = reply_event_id.as_deref().unwrap_or_default().to_string();
            Some(Self::canonicalize_thread_root(&tx, &conv.id, &target_id).await?)
        } else {
            reply_event_id
        };

        if let Some(attachment_id) = row_metadata
            .get("attachment_id")
            .and_then(|value| value.as_str())
        {
            tx.execute(
                "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
                &[&attachment_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("send_message attachment lock: {e}")))?;

            let attachment_exists = tx
                .query_opt(
                    "SELECT owner_id FROM attachment WHERE id = $1",
                    &[&attachment_id],
                )
                .await
                .map_err(|e| AppError::Internal(format!("send_message attachment lookup: {e}")))?;
            let Some(attachment_row) = attachment_exists else {
                return Err(AppError::NotFound("attachment not found".into()));
            };

            let attachment_owner_id: String = attachment_row.get("owner_id");
            let can_reuse_attachment = attachment_owner_id == actor.id
                || matches!(actor.principal_type, PrincipalType::Human);
            if !can_reuse_attachment {
                let prior_access = tx
                    .query_opt(
                        "SELECT 1
                         FROM conversation_events ce
                         JOIN conversation_member cm ON cm.conv_id = ce.conversation_id
                         WHERE ce.metadata->>'attachment_id' = $1
                           AND cm.principal_id = $2
                           AND cm.removed_at IS NULL
                         LIMIT 1",
                        &[&attachment_id, &actor.id],
                    )
                    .await
                    .map_err(|e| {
                        AppError::Internal(format!("send_message attachment auth: {e}"))
                    })?;
                if prior_access.is_none() {
                    return Err(AppError::Forbidden("attachment access denied".into()));
                }
            }
        }

        let row = tx
            .query_one(
                "INSERT INTO conversation_events
                    (conversation_id, seq, event_id, event_type, sender_id,
                     content, content_type, metadata, client_msg_id, turn_id,
                     reply_event_id, created_at)
                 VALUES (
                    $1,
                    COALESCE((SELECT MAX(seq) FROM conversation_events WHERE conversation_id = $1), 0) + 1,
                    $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW()
                 )
                 ON CONFLICT DO NOTHING
                 RETURNING event_id, seq, created_at",
                &[
                    &conv.id,
                    &message_id,
                    &event_type,
                    &actor.id,
                    &content,
                    &request.content_type,
                    &row_metadata,
                    &client_msg_id_opt,
                    &turn_id,
                    &reply_event_id,
                ],
            )
            .await
            .map_err(|e| {
                // Check for unique constraint violations (dedup)
                if let Some(db_err) = e.as_db_error()
                    && db_err.code() == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION
                {
                    return AppError::Conflict(format!(
                        "duplicate message: {}",
                        db_err.detail().unwrap_or("unique constraint violated")
                    ));
                }
                AppError::Internal(format!("send_message insert: {e}"))
            })?;

        let db_event_id: String = row.get(0);
        let seq: i64 = row.get(1);
        let created_at: chrono::DateTime<chrono::Utc> = row.get(2);

        // Insert into event_outbox
        let outbox_payload = serde_json::json!({
            "message_id": db_event_id,
            "conversation_id": conv.id,
            "sender_id": actor.id,
            "content": request.content,
            "content_type": request.content_type,
            "seq": seq,
            "metadata": row_metadata,
            "trace_id": trace_id_opt,
        });
        let aggregate_type = "conversation_event".to_string();
        let outbox_event_type = "message".to_string();

        tx.execute(
            "INSERT INTO event_outbox
                (aggregate_type, aggregate_id, event_type, payload, created_at, published)
             VALUES ($1, $2, $3, $4, NOW(), FALSE)",
            &[
                &aggregate_type,
                &conv.id,
                &outbox_event_type,
                &outbox_payload,
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("send_message outbox insert: {e}")))?;

        // Phase 4: push message.created events to outbox_event for each
        // conversation member so that list_events/ack_events consumers
        // (SSE, webhooks) see the new message.
        //
        // Payload schema (stable contract for external webhook apps):
        //   event_type    = "message.created" (carried at envelope level)
        //   workspace_id  = conv.workspace_id
        //   conversation_id, conversation_type
        //   message_id, content, content_type, server_seq
        //   sender: { id, name, type }
        //   metadata (preserved for integrations such as the platform bridge)
        //   client_msg_id (for dedup on receiver side, may be null)
        let event_payload = serde_json::json!({
            "workspace_id": conv.workspace_id,
            "conversation_id": conv.id,
            "conversation_type": conv.conversation_type,
            "message_id": db_event_id,
            "sender": {
                "id": actor.id,
                "name": actor.name,
                "type": actor.principal_type,
            },
            // Keep sender_id at top level for backwards compatibility with
            // existing consumers before this envelope standardisation.
            "sender_id": actor.id,
            "content": request.content,
            "content_type": request.content_type,
            "server_seq": seq,
            "client_msg_id": request.idempotency_key,
            "metadata": row_metadata,
            // Carry the originating FE trace id through the outbox so every
            // downstream consumer (webhooks, router, writer) can cite the
            // same correlator in their structured logs.
            "trace_id": trace_id_opt,
        });
        for member_id in conv.members.keys() {
            let evt_id = new_id();
            let evt_type = "message.created";
            tx.execute(
                "INSERT INTO outbox_event (id, workspace_id, principal_id, event_type, payload, created_at)
                 VALUES ($1, $2, $3, $4, $5, NOW())",
                &[&evt_id, &conv.workspace_id, member_id, &evt_type, &event_payload],
            )
            .await
            .map_err(|e| AppError::Internal(format!("send_message push_event: {e}")))?;
        }

        // Additionally emit `app_mention` events for each *agent* member
        // whose name (case-insensitive) appears as `@<name>` in the
        // content. Slack-style: external webhook agents default to
        // subscribing only to this event type so they stop responding
        // to every unrelated message in a chatty group.
        let member_ids: Vec<String> = conv.members.keys().cloned().collect();
        let agent_rows = tx
            .query(
                "SELECT id, name FROM principal
                 WHERE id = ANY($1) AND type = 'agent' AND disabled = FALSE",
                &[&member_ids],
            )
            .await
            .map_err(|e| AppError::Internal(format!("send_message agent lookup: {e}")))?;

        // Defense-in-depth: V013 added a partial-unique index on
        // (workspace_id, lower(name)) so same-name agents can't coexist
        // going forward. But migrations run once; pre-migration rows can
        // still be members of a group (soft-deleted is fine; disabled is
        // already filtered above). If we nevertheless see two active agents
        // with the same lowercase name inside this conversation, every one
        // of them would fan out on a single @mention — the exact
        // "openclaw-bridge both spin" bug. Log it loudly so operators can
        // clean up.
        {
            use std::collections::HashMap;
            let mut name_counts: HashMap<String, Vec<String>> = HashMap::new();
            for row in &agent_rows {
                let id: String = row.get("id");
                let name: String = row.get("name");
                name_counts.entry(name.to_lowercase()).or_default().push(id);
            }
            for (name_lc, ids) in &name_counts {
                if ids.len() > 1 {
                    tracing::warn!(
                        event = "duplicate_agent_name_in_conversation",
                        conversation_id = %conv.id,
                        workspace_id = %conv.workspace_id,
                        name_lc = %name_lc,
                        agent_ids = ?ids,
                        agent_count = ids.len(),
                        "two+ active agents share a display name in this conversation; @mention will fan out to all of them"
                    );
                }
            }
        }

        let content_lc = request.content.to_lowercase();
        let mention_candidates: Vec<String> = agent_rows
            .iter()
            .map(|row| row.get::<_, String>("name").to_lowercase())
            .collect();
        for row in &agent_rows {
            let agent_id: String = row.get("id");
            let agent_name: String = row.get("name");
            // Sender can't @mention themselves into a self-loop.
            if agent_id == actor.id {
                continue;
            }
            if !contains_mention_with_candidates(
                &content_lc,
                &agent_name.to_lowercase(),
                &mention_candidates,
            ) {
                continue;
            }
            let evt_id = new_id();
            tx.execute(
                "INSERT INTO outbox_event (id, workspace_id, principal_id, event_type, payload, created_at)
                 VALUES ($1, $2, $3, $4, $5, NOW())",
                &[
                    &evt_id,
                    &conv.workspace_id,
                    &agent_id,
                    &"app_mention",
                    &event_payload,
                ],
            )
            .await
            .map_err(|e| {
                AppError::Internal(format!("send_message app_mention push: {e}"))
            })?;

            // Structured "mention detected" log. This is the gateway-side
            // proof that a mention was DETECTED and ROUTED — the FE
            // `pixel_agent_mention` telemetry on its own is just display-name
            // substring matching and doesn't prove the backend fanned out to
            // the agent. Intentionally does NOT log raw content (PII).
            tracing::info!(
                event = "agent_mention_detected",
                trace_id = trace_id_opt.as_deref().unwrap_or("none"),
                sender_id = %actor.id,
                conversation_id = %conv.id,
                message_id = %db_event_id,
                target_agent_id = %agent_id,
                matched_alias = %agent_name,
                content_len = request.content.len(),
                "app_mention outbox row inserted"
            );
        }

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("send_message commit: {e}")))?;

        // Increment unread counters (Mattermost pattern).
        // This runs outside the transaction — eventual consistency is fine.
        //
        // Threaded replies WITHOUT broadcast do not bump the conversation-
        // level counter: a busy thread should not light up the conversation
        // badge for members who already triaged the root. Broadcast replies
        // appear on the main timeline, so they count like normal messages.
        // Thread-level unread is computed from thread_read_receipt instead
        //.
        //
        // PINNED SEMANTICS for future mention wiring: per the RFC's
        // counter table, MENTIONS inside threads must still bump
        // mention_count unconditionally (a mention is a direct ask
        // regardless of where it happens). Today increment_msg_count is
        // called with an empty mentioned_user_ids list — when mentions
        // are wired through this path, the mention bump must NOT sit
        // behind this gate; only the total_msg_count bump is gated.
        let bumps_conversation_unread = thread_flags.bumps_conversation_unread();
        if bumps_conversation_unread && let Err(e) = self.increment_msg_count(&conv.id, &[]).await {
            tracing::warn!(
                conversation_id = %conv.id,
                error = %e,
                "increment_msg_count failed (non-fatal)"
            );
        }

        // Auto-mark sender as read (sender shouldn't see their own message
        // as unread). Gated on the same condition as the counter bump: for
        // a quiet (non-broadcast) threaded reply, total_msg_count was not
        // bumped, so marking viewed here would have only one effect —
        // wiping the sender's PRE-EXISTING main-timeline unread and
        // mention_count, which they haven't actually read. Skip it.
        if bumps_conversation_unread
            && let Err(e) = self.mark_conversation_viewed(&conv.id, &actor.id).await
        {
            tracing::warn!(
                conversation_id = %conv.id,
                error = %e,
                "auto mark_viewed for sender failed (non-fatal)"
            );
        }

        tracing::info!(
            event = "send_message_succeeded",
            trace_id = trace_id_opt.as_deref().unwrap_or("none"),
            message_id = %db_event_id,
            conversation_id = %conv.id,
            sender_id = %actor.id,
            seq,
            content_len = request.content.len(),
            is_thread_reply,
            is_broadcast,
            thread_root = reply_event_id.as_deref().unwrap_or(""),
            "DB-first send_message succeeded"
        );

        // 5. Return the Message
        Ok(Message {
            id: db_event_id,
            workspace_id: conv.workspace_id,
            conversation_id: conv.id,
            sender_id: actor.id,
            content: request.content,
            content_type: request.content_type,
            metadata: row_metadata,
            edited_at: None,
            edited_by: None,
            server_seq: seq as u64,
            idempotency_key: client_msg_id,
            created_at,
        })
    }

    /// Resolve the canonical thread root for a threaded reply. Thin
    /// wrapper over the shared
    /// [`EventStore::canonicalize_thread_root_in_tx`] (scoping rules and
    /// discriminator semantics documented there — they are shared with
    /// the agent outbox write path) that maps the missing-target case to
    /// the API-facing uniform `NotFound`.
    async fn canonicalize_thread_root(
        tx: &deadpool_postgres::Transaction<'_>,
        conversation_id: &str,
        target_id: &str,
    ) -> Result<String, AppError> {
        let root = choruz_store::EventStore::canonicalize_thread_root_in_tx(
            &**tx,
            conversation_id,
            target_id,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("thread target message not found".into()))?;
        if root != target_id {
            tracing::debug!(
                conversation_id = %conversation_id,
                requested_target = %target_id,
                canonical_root = %root,
                "thread reply canonicalized to root"
            );
        }
        Ok(root)
    }

    // ── Unread counts (Mattermost pattern) ─────────────────────────────

    /// Increment `conversation.total_msg_count` by 1 and optionally
    /// increment `mention_count` for the specified user IDs.
    ///
    /// Called inside `send_message` after the message row is committed.
    pub async fn increment_msg_count(
        &self,
        conversation_id: &str,
        mentioned_user_ids: &[String],
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;

        // Increment total_msg_count on the conversation row.
        client
            .execute(
                "UPDATE conversation SET total_msg_count = total_msg_count + 1 WHERE id = $1",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("increment_msg_count: {e}")))?;

        // Increment mention_count for explicitly mentioned users (best-effort).
        for uid in mentioned_user_ids {
            client
                .execute(
                    "UPDATE conversation_member SET mention_count = mention_count + 1 \
                     WHERE conv_id = $1 AND principal_id = $2 AND removed_at IS NULL",
                    &[&conversation_id, uid],
                )
                .await
                .ok(); // best-effort — don't fail the message send
        }

        Ok(())
    }

    /// Mark a conversation as viewed by a user: set `msg_count` to the
    /// conversation's current `total_msg_count` and reset `mention_count`.
    ///
    /// Called when the user opens/selects a conversation.
    pub async fn mark_conversation_viewed(
        &self,
        conversation_id: &str,
        user_id: &str,
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        client
            .execute(
                "UPDATE conversation_member SET \
                   msg_count = (SELECT total_msg_count FROM conversation WHERE id = $1), \
                   mention_count = 0, \
                   last_viewed_at = NOW() \
                 WHERE conv_id = $1 AND principal_id = $2",
                &[&conversation_id, &user_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("mark_conversation_viewed: {e}")))?;
        Ok(())
    }

    /// Return unread counts for every conversation the user belongs to.
    ///
    /// `unread_count = GREATEST(c.total_msg_count - cm.msg_count, 0)`
    pub async fn get_unread_counts(
        &self,
        user_id: &str,
    ) -> Result<Vec<ConversationUnread>, AppError> {
        self.get_unread_counts_scoped(user_id, None).await
    }

    /// The bootstrap variant limits the thread-unread calculation to the
    /// bounded conversation page instead of scanning every visible channel.
    pub async fn get_unread_counts_for_conversations(
        &self,
        user_id: &str,
        conversation_ids: &[String],
    ) -> Result<Vec<ConversationUnread>, AppError> {
        if conversation_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.get_unread_counts_scoped(user_id, Some(conversation_ids.to_vec()))
            .await
    }

    async fn get_unread_counts_scoped(
        &self,
        user_id: &str,
        conversation_ids: Option<Vec<String>>,
    ) -> Result<Vec<ConversationUnread>, AppError> {
        let client = self.store.connect().await?;
        // thread_unreads: per conversation, count distinct thread roots
        // that have at least one threaded reply newer than the
        // principal's receipt (no receipt row ⇒ last_read_seq 0, so any
        // reply counts). Scoped to the principal's member conversations
        // by the outer JOIN below.
        // Sender exclusion (`ce.sender_id <> $1`): a thread where only YOUR
        // OWN replies are newer than your receipt is not unread — without
        // this, sending a quiet reply lights your own badge until you
        // re-view the thread.
        //
        // Cost note (accepted tradeoff): the LATERAL walks EVERY threaded
        // reply in the conversation per poll — the `seq > last_read_seq`
        // bound comes from the nullable side of the LEFT JOIN, so it
        // filters after the scan and can never narrow the index range.
        // That is O(threaded replies in the conversation), NOT O(unread
        // replies). The V018 partial index covers exactly this scan
        // (predicate includes the thread flag, so legacy quote-replies
        // and non-reply rows are never touched), which keeps it
        // proportional to genuine thread traffic. A per-thread counter
        // table is the escape hatch if reply volume ever makes this
        // measurable.
        let unreads_sql = format!(
            "SELECT c.id, \
                    GREATEST(c.total_msg_count - cm.msg_count, 0) AS unread_count, \
                    cm.mention_count, \
                    COALESCE(tu.thread_unread_count, 0) AS thread_unread_count \
             FROM conversation c \
             JOIN conversation_member cm ON cm.conv_id = c.id \
             JOIN principal p ON p.id = $1 \
             LEFT JOIN company co ON co.id = c.workspace_id \
             LEFT JOIN company_member com \
               ON com.company_id = co.id AND com.principal_id = $1 \
             LEFT JOIN LATERAL ( \
               SELECT COUNT(DISTINCT ce.reply_event_id)::BIGINT AS thread_unread_count \
               FROM conversation_events ce \
               LEFT JOIN thread_read_receipt trr \
                 ON trr.conversation_id = ce.conversation_id \
                AND trr.thread_root_id = ce.reply_event_id \
                AND trr.principal_id = $1 \
               WHERE ce.conversation_id = c.id \
                 AND ce.reply_event_id IS NOT NULL \
                 AND ce.sender_id <> $1 \
                 AND {THREAD_FLAG_SQL_CE} \
                 AND ce.seq > COALESCE(trr.last_read_seq, 0) \
             ) tu ON TRUE \
             WHERE cm.principal_id = $1 AND cm.removed_at IS NULL \
               AND ($2::TEXT[] IS NULL OR c.id = ANY($2)) \
               AND ((co.id IS NULL AND c.workspace_id = p.workspace_id) OR (co.deleted_at IS NULL \
                    AND (c.workspace_id = p.workspace_id OR com.principal_id IS NOT NULL)))",
            THREAD_FLAG_SQL_CE = choruz_store::conversation_events::thread_flag_sql_for("ce"),
        );
        let rows = client
            .query(&unreads_sql, &[&user_id, &conversation_ids])
            .await
            .map_err(|e| AppError::Internal(format!("get_unread_counts: {e}")))?;

        Ok(rows
            .iter()
            .map(|r| ConversationUnread {
                conversation_id: r.get(0),
                unread_count: r.get::<_, i64>(1),
                mention_count: r.get::<_, i64>(2),
                thread_unread_count: r.get::<_, i64>(3),
            })
            .collect())
    }

    // ── Threads read path ───────────

    /// Timeline view of a conversation: like `list_messages`, but
    /// threaded replies WITHOUT broadcast are filtered out, and a batched
    /// rollup (`ThreadSummary`) is computed for every thread root visible
    /// on the page. Legacy quote-replies (no thread flag) stay inline.
    pub async fn list_timeline_messages(
        &self,
        conversation_id: &str,
        limit: Option<u64>,
        since_seq: Option<u64>,
    ) -> Result<crate::TimelineMessages, AppError> {
        // Clamp BEFORE the i64 cast: an absurd u64 would wrap negative and
        // Postgres rejects a negative LIMIT with a 500.
        let db_limit = limit.unwrap_or(200).clamp(1, 200) as i64;
        // Clamp BEFORE the i64 cast: a u64 above i64::MAX would wrap
        // negative and turn the high-watermark bound into "return
        // everything".
        let db_since_seq = since_seq
            .map(|s| s.min(i64::MAX as u64) as i64)
            .unwrap_or(0);

        let workspace_id = self
            .get_conversation(conversation_id)
            .await
            .map(|c| c.workspace_id)
            .unwrap_or_default();

        let client = self.store.connect().await?;
        // Timeline filter: keep a row unless it is a threaded (flagged)
        // reply that is not broadcast. Both jsonb predicates are
        // COALESCE-wrapped, so they are safe to evaluate on any row —
        // Postgres guarantees no WHERE-clause evaluation order, and none
        // is needed here.
        //
        // Cost advisory (perf, accepted): the thread/broadcast predicate
        // cannot be served by the (conversation_id, seq) PK, so a page
        // fill walks (and heap-fetches) quiet threaded replies between
        // visible rows. Quiet-reply-heavy conversations pay per page
        // request; if this measures hot, add a partial index excluding
        // quiet replies.
        let order = if since_seq.is_some() { "ASC" } else { "DESC" };
        let timeline_sql = format!(
            "SELECT conversation_id, seq, event_id, event_type, sender_id, \
                    content, content_type, metadata, client_msg_id, turn_id, \
                    reply_event_id, created_at \
             FROM conversation_events \
             WHERE conversation_id = $1 \
               AND event_type IN ('message', 'message.created', 'reply') \
               AND seq > $3 \
               AND ( \
                 reply_event_id IS NULL \
                 OR NOT {THREAD_FLAG_SQL} \
                 OR COALESCE(metadata->'broadcast' = 'true'::jsonb, false) \
               ) \
             ORDER BY seq {order} \
             LIMIT $2"
        );
        let rows = client
            .query(&timeline_sql, &[&conversation_id, &db_limit, &db_since_seq])
            .await
            .map_err(|e| AppError::Internal(format!("list_timeline_messages: {e}")))?;

        let mut messages: Vec<Message> = rows
            .iter()
            .map(|row| event_row_to_message(row, &workspace_id))
            .collect();
        if since_seq.is_none() {
            messages.reverse(); // initial newest-first → chronological
        }

        // Batched rollups over the page's message ids. Served by the
        // V018 partial index; bounded by the page size (≤200 roots).
        let page_ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
        let thread_summaries = if page_ids.is_empty() {
            Vec::new()
        } else {
            let id_refs: Vec<&str> = page_ids.iter().map(String::as_str).collect();
            let rollup_sql = format!(
                "SELECT reply_event_id AS root, \
                        COUNT(*)::BIGINT AS reply_count, \
                        MAX(created_at) AS last_reply_at, \
                        (ARRAY_AGG(DISTINCT sender_id))[1:5] AS participant_sample \
                 FROM conversation_events \
                 WHERE conversation_id = $1 \
                   AND reply_event_id = ANY($2) \
                   AND {THREAD_FLAG_SQL} \
                 GROUP BY reply_event_id"
            );
            let rollup_rows = client
                .query(&rollup_sql, &[&conversation_id, &id_refs])
                .await
                .map_err(|e| AppError::Internal(format!("thread rollups: {e}")))?;
            rollup_rows
                .iter()
                .map(|r| crate::ThreadSummary {
                    root_event_id: r.get("root"),
                    reply_count: r.get("reply_count"),
                    last_reply_at: r.get("last_reply_at"),
                    participant_sample: r.get("participant_sample"),
                })
                .collect()
        };

        Ok(crate::TimelineMessages {
            messages,
            thread_summaries,
        })
    }

    /// Read one thread: the root message plus its threaded replies in seq
    /// order. Returns NotFound when the root does not exist in this
    /// conversation (uniform with the write-path canonicalization — no
    /// cross-conversation existence oracle).
    pub async fn list_thread_replies(
        &self,
        conversation_id: &str,
        thread_root_id: &str,
        limit: Option<u64>,
        since_seq: Option<u64>,
    ) -> Result<crate::ThreadDetail, AppError> {
        // Clamp BEFORE the i64 cast (same rationale as list_timeline_messages).
        let db_limit = limit.unwrap_or(200).clamp(1, 200) as i64;
        // Clamp BEFORE the i64 cast: a u64 above i64::MAX would wrap
        // negative and turn the high-watermark bound into "return
        // everything".
        let db_since_seq = since_seq
            .map(|s| s.min(i64::MAX as u64) as i64)
            .unwrap_or(0);

        let workspace_id = self
            .get_conversation(conversation_id)
            .await
            .map(|c| c.workspace_id)
            .unwrap_or_default();

        let client = self.store.connect().await?;

        // Canonicalize-on-read: a client may pass a REPLY's id (e.g. from a
        // deep link to a specific threaded message). Resolve it to the real
        // root via the shared helper so the endpoint always returns the
        // full thread instead of a degenerate "reply as root with zero
        // replies" shape. Uniform NotFound for unknown/cross-conversation
        // ids comes from the same helper.
        let canonical_root = choruz_store::EventStore::canonicalize_thread_root_in_tx(
            &**client,
            conversation_id,
            thread_root_id,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("thread root not found".into()))?;

        let root_row = client
            .query_opt(
                "SELECT conversation_id, seq, event_id, event_type, sender_id, \
                        content, content_type, metadata, client_msg_id, turn_id, \
                        reply_event_id, created_at \
                 FROM conversation_events \
                 WHERE event_id = $1 AND conversation_id = $2 \
                   AND event_type IN ('message', 'message.created', 'reply')",
                &[&canonical_root, &conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("thread root lookup: {e}")))?
            .ok_or_else(|| AppError::NotFound("thread root not found".into()))?;
        let root = event_row_to_message(&root_row, &workspace_id);

        let replies_sql = format!(
            "SELECT conversation_id, seq, event_id, event_type, sender_id, \
                    content, content_type, metadata, client_msg_id, turn_id, \
                    reply_event_id, created_at \
             FROM conversation_events \
             WHERE conversation_id = $1 \
               AND reply_event_id = $2 \
               AND {THREAD_FLAG_SQL} \
               AND seq > $4 \
             ORDER BY seq ASC \
             LIMIT $3"
        );
        let reply_rows = client
            .query(
                &replies_sql,
                &[&conversation_id, &canonical_root, &db_limit, &db_since_seq],
            )
            .await
            .map_err(|e| AppError::Internal(format!("thread replies: {e}")))?;
        let replies = reply_rows
            .iter()
            .map(|r| event_row_to_message(r, &workspace_id))
            .collect();

        Ok(crate::ThreadDetail { root, replies })
    }

    /// Upsert the caller's read receipt for one thread: records the max
    /// reply seq currently in the thread. Lazy — the row appears on first
    /// view. Returns NotFound for a root that doesn't exist in this
    /// conversation (FK would reject anyway; checking first gives a clean
    /// 404 instead of a 500).
    ///
    /// Canonicalize-on-write: a client may POST a REPLY's id (mirroring
    /// the GET endpoint's deep-link case). The receipt and its MAX(seq)
    /// MUST be keyed on the canonical root — the unreads LATERAL joins
    /// receipts on `reply_event_id` (always canonical), so a receipt
    /// keyed to a reply id would be a dead row and the thread's unread
    /// dot would never clear.
    pub async fn mark_thread_viewed(
        &self,
        conversation_id: &str,
        thread_root_id: &str,
        principal_id: &str,
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        let canonical_root = choruz_store::EventStore::canonicalize_thread_root_in_tx(
            &**client,
            conversation_id,
            thread_root_id,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("thread root not found".into()))?;

        // The MAX(seq) subquery interpolates THREAD_FLAG_SQL like every
        // other thread read path — without it the predicate doesn't match
        // the V018 partial index (whose WHERE includes the flag) and the
        // per-view receipt write degrades to a full-conversation scan.
        let upsert_sql = format!(
            "INSERT INTO thread_read_receipt \
                (conversation_id, thread_root_id, principal_id, last_read_seq, last_read_at) \
             SELECT $1, $2, $3, \
                    COALESCE((SELECT MAX(seq) FROM conversation_events \
                              WHERE conversation_id = $1 AND reply_event_id = $2 \
                                AND {THREAD_FLAG_SQL}), 0), \
                    NOW() \
             ON CONFLICT (conversation_id, thread_root_id, principal_id) DO UPDATE \
             SET last_read_seq = EXCLUDED.last_read_seq, \
                 last_read_at = EXCLUDED.last_read_at"
        );
        client
            .execute(
                &upsert_sql,
                &[&conversation_id, &canonical_root, &principal_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("mark_thread_viewed upsert: {e}")))?;
        Ok(())
    }
}

/// Map a conversation_events row (full SELECT list: conversation_id, seq,
/// event_id, event_type, sender_id, content, content_type, metadata,
/// client_msg_id, turn_id, reply_event_id, created_at) to a domain
/// Message. Shared by the thread read paths so SELECT-list/mapping
/// drift surfaces in one place instead of three.
fn event_row_to_message(row: &tokio_postgres::Row, workspace_id: &str) -> Message {
    Message {
        id: row.get("event_id"),
        workspace_id: workspace_id.to_string(),
        conversation_id: row.get("conversation_id"),
        sender_id: row.get("sender_id"),
        content: row.get::<_, Option<String>>("content").unwrap_or_default(),
        content_type: row.get("content_type"),
        metadata: row.get("metadata"),
        edited_at: None,
        edited_by: None,
        server_seq: row.get::<_, i64>("seq") as u64,
        idempotency_key: row
            .get::<_, Option<String>>("client_msg_id")
            .or_else(|| row.get::<_, Option<String>>("turn_id"))
            .unwrap_or_default(),
        created_at: row.get("created_at"),
    }
}

fn contains_mention_with_candidates(
    content: &str,
    mention: &str,
    mention_candidates: &[String],
) -> bool {
    if mention.is_empty() {
        return false;
    }

    let pattern = format!("@{mention}");
    content.match_indices(&pattern).any(|(start, _)| {
        mention_matches_at(content, mention, start)
            && !mention_candidates
                .iter()
                .filter(|candidate| candidate.len() > mention.len())
                .any(|candidate| mention_matches_at(content, candidate, start))
    })
}

fn mention_matches_at(content: &str, mention: &str, start: usize) -> bool {
    if mention.is_empty() {
        return false;
    }

    let pattern = format!("@{mention}");
    if !content[start..].starts_with(&pattern) {
        return false;
    }

    let before = content[..start].chars().next_back();
    let after = content[start + pattern.len()..].chars().next();

    before.is_none_or(|ch| !is_mention_char(ch)) && after.is_none_or(|ch| !is_mention_char(ch))
}

fn is_mention_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

#[cfg(test)]
mod tests {
    use super::contains_mention_with_candidates;

    #[test]
    fn app_mention_matcher_rejects_prefix_match() {
        assert!(!contains_mention_with_candidates("@dev2 hi", "dev", &[]));
        assert!(!contains_mention_with_candidates(
            "@alliance status",
            "all",
            &[]
        ));
        let candidates = vec!["claude".to_string(), "claude code 1".to_string()];
        assert!(!contains_mention_with_candidates(
            "@claude code 1 hi",
            "claude",
            &candidates
        ));
        assert!(contains_mention_with_candidates(
            "@claude code 1 hi",
            "claude code 1",
            &candidates
        ));
    }

    #[test]
    fn app_mention_matcher_accepts_exact_token_with_punctuation() {
        assert!(contains_mention_with_candidates("@dev2, hi", "dev2", &[]));
        assert!(contains_mention_with_candidates(
            "please review @backend-dev",
            "backend-dev",
            &[]
        ));
    }

    #[test]
    fn app_mention_matcher_rejects_passive_group_launch_kickoff() {
        let content = "mission: ship onboarding mvp\n\nroles: project operator, backend engineer, code reviewer.\n\nnext user action: send the first concrete work item or question when ready.";
        let candidates = vec![
            "project operator".to_string(),
            "backend engineer".to_string(),
            "code reviewer".to_string(),
        ];

        assert!(!contains_mention_with_candidates(
            content,
            "project operator",
            &candidates
        ));
        assert!(!contains_mention_with_candidates(
            content,
            "backend engineer",
            &candidates
        ));
        assert!(!contains_mention_with_candidates(
            content,
            "code reviewer",
            &candidates
        ));
    }
}
