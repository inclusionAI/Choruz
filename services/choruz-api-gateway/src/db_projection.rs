use choruz_domain::{Principal, PrincipalType};

// ── DB persistence helpers ────────────────────────────────────────────

/// Execute a DB statement with up to 3 retries (exponential backoff).
/// Acquires its own connection from the pool on each attempt.
/// Used for fire-and-forget persistence where the in-memory mutation has
/// already been applied.  Logs a warning on each retry and an error if
/// all attempts fail.
pub(crate) async fn db_persist(
    store: &choruz_store::EventStore,
    stmt: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    context: &str,
) {
    for attempt in 0u32..3 {
        let client = match store.connect().await {
            Ok(c) => c,
            Err(e) if attempt < 2 => {
                tracing::warn!(attempt = attempt + 1, error = %e, context, "DB persist: connect failed, retrying");
                tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt as u64 + 1)))
                    .await;
                continue;
            }
            Err(e) => {
                tracing::error!(error = %e, context, "DB persist: connect failed after 3 attempts");
                return;
            }
        };
        match client.execute(stmt, params).await {
            Ok(_) => return,
            Err(e) if attempt < 2 => {
                tracing::warn!(attempt = attempt + 1, error = %e, context, "DB persist failed, retrying");
                tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt as u64 + 1)))
                    .await;
            }
            Err(e) => {
                tracing::error!(error = %e, context, "DB persist failed after 3 attempts");
            }
        }
    }
}

/// Persist a principal row to PostgreSQL (upsert).
pub(crate) async fn persist_principal_to_db(store: &choruz_store::EventStore, p: &Principal) {
    let client = match store.connect().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, principal_id = %p.id, "persist_principal_to_db: connect failed");
            return;
        }
    };
    let type_str = match p.principal_type {
        PrincipalType::Human => "human",
        PrincipalType::Agent => "agent",
    };
    let channel_visibility = match p.channel_visibility {
        choruz_domain::ChannelVisibility::Visible => "visible",
        choruz_domain::ChannelVisibility::Internal => "internal",
    };
    if let Err(e) = client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, secret_hash, disabled, channel_visibility, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO UPDATE
             SET workspace_id = EXCLUDED.workspace_id,
                 name = EXCLUDED.name,
                 secret_hash = COALESCE(EXCLUDED.secret_hash, principal.secret_hash),
                 disabled = EXCLUDED.disabled,
                 channel_visibility = EXCLUDED.channel_visibility,
                 updated_at = EXCLUDED.updated_at",
            &[
                &p.id,
                &p.workspace_id,
                &type_str,
                &p.name,
                &p.secret_hash,
                &p.disabled,
                &channel_visibility,
                &p.created_at,
                &p.updated_at,
            ],
        )
        .await
    {
        tracing::error!(error = %e, principal_id = %p.id, "persist_principal_to_db: upsert failed");
    }
}

// NOTE: persist_conversation_to_db, persist_members_to_db, and
// dual_write_shadow have been REMOVED (Phase 6 cleanup).
// All conversation/member/message writes now go through DbService.
