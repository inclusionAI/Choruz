use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
};

use choruz_agent_runtime::RuntimeStore;
use choruz_api_gateway::{Config, LocalAuthConfig, router_with_runtime};
use choruz_application::ChatApp;
use choruz_infrastructure::init_tracing;
use choruz_session::PgSessionStore;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    init_tracing("choruz-api-gateway").map_err(std::io::Error::other)?;

    let cfg = Config::from_env().map_err(std::io::Error::other)?;
    cfg.validate_production();

    let runtime_store = RuntimeStore::new(choruz_common::PgConfig::from_env().to_connect_string());
    runtime_store
        .list_active_bindings()
        .await
        .map_err(|error| std::io::Error::other(format!("cannot connect to database: {error}")))?;
    tracing::info!("database connectivity verified");

    let app = build_app_from_db(&runtime_store)
        .await
        .map_err(std::io::Error::other)?;
    tracing::info!(
        principals = app.principal_count(),
        "app state loaded from database"
    );

    let auth = LocalAuthConfig::new(
        cfg.session_secret.clone(),
        cfg.operator_password.clone(),
        cfg.operator_workspace.clone(),
        cfg.operator_user.clone(),
        cfg.session_ttl_hours,
    );
    // Use sync ensure_operator on ChatApp for in-memory state during transition.
    // The DB-backed ensure_operator runs later when DbService is available.
    auth.ensure_operator_sync(&app)
        .map_err(|error| std::io::Error::other(error.to_string()))?;

    // ── Seed provisioner agent ──
    {
        match seed_provisioner_if_missing(&app, &runtime_store).await {
            Ok(Some(id)) => tracing::info!(id, "auto-created provisioner agent principal"),
            Ok(None) => tracing::debug!("provisioner agent already exists"),
            Err(e) => tracing::warn!(error = %e, "failed to seed provisioner agent (non-fatal)"),
        }
    }

    // ── Repair agent secret_hash from agent_tokens.json ──
    {
        let token_file = cfg.agent_tokens_file.display().to_string();
        if let Ok(content) = std::fs::read_to_string(&token_file)
            && let Ok(tokens) = serde_json::from_str::<HashMap<String, String>>(&content)
        {
            let mut repaired = 0u64;
            for (principal_id, token) in &tokens {
                let hash = choruz_auth::hash_secret(token);
                app.set_principal_secret_hash(principal_id, &hash);
                if let Ok(client) = runtime_store.connect().await {
                    if let Err(e) = client.execute(
                        "UPDATE principal SET secret_hash = $1 WHERE id = $2 AND (secret_hash IS NULL OR secret_hash != $1)",
                        &[&hash, principal_id],
                    ).await {
                        tracing::warn!(principal_id = %principal_id, error = %e, "secret_hash repair update failed");
                    }
                }
                repaired += 1;
            }
            if repaired > 0 {
                tracing::info!(repaired, "repaired agent secret_hash from token file");
            }
        }
    }

    let port = cfg.api_port;
    let host = &cfg.api_host;
    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await?;
    let attachment_root = cfg.attachment_dir.clone();

    match runtime_store.backfill_session_ids().await {
        Ok(n) if n > 0 => tracing::info!(count = n, "backfilled session IDs from disk"),
        Ok(_) => tracing::debug!("no session IDs to backfill"),
        Err(e) => tracing::warn!(error = %e, "session ID backfill failed (non-fatal)"),
    }

    // Initialize the event store for the new message pipeline.
    let event_store =
        choruz_store::EventStore::new(choruz_common::PgConfig::from_env().to_connect_string());
    if let Err(e) = event_store.health_check().await {
        tracing::warn!(error = %e, "event store health check failed (non-fatal, pipeline tables may not exist yet)");
    } else {
        tracing::info!("event store connectivity verified");
    }

    tracing::info!(host, port, "choruz-api-gateway listening");
    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("register SIGTERM");
        tokio::select! {
            _ = ctrl_c => {},
            _ = sigterm.recv() => {},
        }
        tracing::info!("shutdown signal received, draining connections...");
    };
    let router = router_with_runtime(
        app,
        attachment_root,
        auth,
        runtime_store,
        PgSessionStore::new(&choruz_common::PgConfig::from_env().to_connect_string()),
        event_store,
    );
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await?;

    Ok(())
}

/// Build the ChatApp shell from PostgreSQL tables (source of truth).
/// Loads principals, conversations, companies, and webhook configs.
/// Messages and audit logs are NOT loaded — they are read on-demand via DbService.
/// The in-memory ChatApp is kept only for backward-compatible inject calls
/// and webhook event delivery during the ongoing stateless migration.
async fn build_app_from_db(store: &RuntimeStore) -> Result<ChatApp, String> {
    use choruz_domain::PrincipalType;

    let client = store
        .connect()
        .await
        .map_err(|e| format!("db connect: {e}"))?;

    // ── Principals ──
    let rows = client
        .query(
            "SELECT id, workspace_id, type, name, disabled, secret_hash, channel_visibility, created_at, updated_at
             FROM principal",
            &[],
        )
        .await
        .map_err(|e| format!("query principals: {e}"))?;

    let mut principals = HashMap::new();
    for row in rows {
        let ptype: String = row.get("type");
        let principal_type = match ptype.as_str() {
            "agent" => PrincipalType::Agent,
            "human" => PrincipalType::Human,
            _ => continue,
        };
        let scopes = match principal_type {
            PrincipalType::Agent => vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
            ],
            PrincipalType::Human => vec![
                "messages:read".into(),
                "messages:write".into(),
                "events:read".into(),
                "groups:manage".into(),
                "agents:manage".into(),
            ],
        };
        let id: String = row.get("id");
        let channel_visibility = match row
            .get::<_, Option<String>>("channel_visibility")
            .as_deref()
        {
            Some("internal") => choruz_domain::ChannelVisibility::Internal,
            Some("visible") | None => choruz_domain::ChannelVisibility::Visible,
            Some(other) => {
                return Err(format!(
                    "unknown channel_visibility '{other}' for principal {id}"
                ));
            }
        };
        principals.insert(
            id.clone(),
            choruz_domain::Principal {
                id,
                workspace_id: row.get("workspace_id"),
                principal_type,
                name: row.get::<_, Option<String>>("name").unwrap_or_default(),
                avatar_url: None,
                scopes,
                secret_hash: row.get("secret_hash"),
                disabled: row.get("disabled"),
                deleted_at: None,
                channel_visibility,
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                user_id: None,
            },
        );
    }
    tracing::info!(count = principals.len(), "loaded principals from DB");

    // ── Conversations + members ──
    let conv_rows = client
        .query(
            "SELECT id, workspace_id, type, name, creator_id, created_at, updated_at
             FROM conversation",
            &[],
        )
        .await
        .map_err(|e| format!("query conversations: {e}"))?;

    let member_rows = client
        .query(
            "SELECT conv_id, principal_id, joined_at
             FROM conversation_member WHERE removed_at IS NULL",
            &[],
        )
        .await
        .map_err(|e| format!("query conversation_member: {e}"))?;

    // Group members by conv_id
    let mut members_by_conv: HashMap<
        String,
        std::collections::BTreeMap<String, choruz_domain::ConversationMember>,
    > = HashMap::new();
    for mr in member_rows {
        let conv_id: String = mr.get("conv_id");
        let pid: String = mr.get("principal_id");
        members_by_conv.entry(conv_id).or_default().insert(
            pid.clone(),
            choruz_domain::ConversationMember {
                principal_id: pid,
                joined_at: mr.get("joined_at"),
            },
        );
    }

    let mut conversations = HashMap::new();
    for row in conv_rows {
        let id: String = row.get("id");
        let ctype: String = row.get("type");
        let conversation_type = match ctype.as_str() {
            "group" => choruz_domain::ConversationType::Group,
            "direct" => choruz_domain::ConversationType::Direct,
            _ => continue,
        };
        let members = members_by_conv.remove(&id).unwrap_or_default();
        conversations.insert(
            id.clone(),
            choruz_domain::Conversation {
                id,
                workspace_id: row.get("workspace_id"),
                conversation_type,
                name: row.get("name"),
                description: None,
                avatar_url: None,
                creator_id: row
                    .get::<_, Option<String>>("creator_id")
                    .unwrap_or_default(),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                members,
            },
        );
    }
    tracing::info!(count = conversations.len(), "loaded conversations from DB");

    // ── Messages: never loaded, and no longer held in memory at all ──
    // All message reads go through DbService, which queries PostgreSQL on
    // demand. `ChatApp` keeps only a bounded window of recently-seen message
    // ids, enough to suppress duplicate `message.created` events on retries.

    // ── Companies + company_members ──
    let company_rows = client
        .query(
            "SELECT id, name, slug, description, avatar_url, owner_id, agents_active, folder_path, multi_harness_accounts, archived_at, deleted_at, created_at, updated_at
             FROM company",
            &[],
        )
        .await
        .map_err(|e| format!("query companies: {e}"))?;

    let cm_rows = client
        .query(
            "SELECT company_id, principal_id, joined_at FROM company_member",
            &[],
        )
        .await
        .map_err(|e| format!("query company_member: {e}"))?;

    let mut company_members_map: HashMap<String, BTreeMap<String, choruz_domain::CompanyMember>> =
        HashMap::new();
    for mr in cm_rows {
        let cid: String = mr.get("company_id");
        let pid: String = mr.get("principal_id");
        company_members_map.entry(cid).or_default().insert(
            pid.clone(),
            choruz_domain::CompanyMember {
                principal_id: pid,
                joined_at: mr.get("joined_at"),
            },
        );
    }

    let mut companies = HashMap::new();
    for row in company_rows {
        let id: String = row.get("id");
        companies.insert(
            id.clone(),
            choruz_domain::Company {
                id,
                name: row.get("name"),
                slug: row.get("slug"),
                description: row.get("description"),
                avatar_url: row.get("avatar_url"),
                owner_id: row.get("owner_id"),
                agents_active: row.get("agents_active"),
                folder_path: row
                    .get::<_, Option<String>>("folder_path")
                    .and_then(|s| if s.is_empty() { None } else { Some(s) }),
                multi_harness_accounts: row.get("multi_harness_accounts"),
                archived_at: row.get("archived_at"),
                deleted_at: row.get("deleted_at"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
            },
        );
    }
    tracing::info!(count = companies.len(), "loaded companies from DB");

    // ── Audit logs: NO LONGER LOADED AT STARTUP ──
    // Audit log reads now go through DbService. The /v1/audit-logs endpoint
    // already queries PostgreSQL directly. Keeping the in-memory list empty
    // reduces startup time and memory usage.
    let audit_logs: Vec<choruz_domain::AuditLog> = Vec::new();

    // ── Assemble via from_parts (rebuilds all indices) ──
    let app = ChatApp::from_parts(
        principals,
        conversations,
        companies,
        company_members_map,
        audit_logs,
        120,
    );

    // ── Load event webhook configs ──
    let wh_rows = client
        .query(
            "SELECT principal_id, url, event_types, cursor, webhook_secret, updated_at
             FROM event_webhook",
            &[],
        )
        .await
        .unwrap_or_default();
    let wh_count = wh_rows.len();
    for row in wh_rows {
        app.inject_event_webhook(choruz_application::EventWebhookConfig {
            principal_id: row.get("principal_id"),
            url: row.get("url"),
            event_types: row.get::<_, Vec<String>>("event_types"),
            cursor: row.get::<_, i64>("cursor") as u64,
            updated_at: row.get("updated_at"),
            webhook_secret: row.get("webhook_secret"),
        });
    }
    if wh_count > 0 {
        tracing::info!(count = wh_count, "loaded event webhooks from DB");
    }

    // ── Align event sequence numbers with runner cursors ──
    // The runner saves cursor positions per binding. If next_event_seq < cursor,
    // the runner will skip all new events. Ensure seq >= max cursor for each agent.
    let cursor_rows = client
        .query(
            "SELECT agent_principal_id, MAX(last_event_cursor) as max_cursor
             FROM agent_runtime_bindings
             GROUP BY agent_principal_id",
            &[],
        )
        .await
        .unwrap_or_default();
    for row in cursor_rows {
        let pid: String = row.get("agent_principal_id");
        let cursor: i64 = row.get("max_cursor");
        if cursor > 0 {
            app.ensure_event_seq_at_least(&pid, cursor as u64 + 1);
        }
    }

    Ok(app)
}

/// Ensure a "provisioner" agent principal exists in the DB and in-memory state.
async fn seed_provisioner_if_missing(
    app: &ChatApp,
    store: &RuntimeStore,
) -> Result<Option<String>, String> {
    let client = store
        .connect()
        .await
        .map_err(|e| format!("db connect: {e}"))?;

    let existing = client
        .query_opt(
            "SELECT id FROM principal WHERE name = 'provisioner' AND type = 'agent' AND disabled = FALSE",
            &[],
        )
        .await
        .map_err(|e| format!("query provisioner: {e}"))?;

    if existing.is_some() {
        return Ok(None);
    }

    let id = choruz_common::new_id();
    let workspace_id = format!("provisioner-{}", &id[..8]);

    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, disabled, created_at, updated_at)
             VALUES ($1, $2, 'agent', 'provisioner', FALSE, NOW(), NOW())
             ON CONFLICT DO NOTHING",
            &[&id, &workspace_id],
        )
        .await
        .map_err(|e| format!("insert provisioner: {e}"))?;

    let principal = choruz_domain::Principal {
        id: id.clone(),
        workspace_id,
        principal_type: choruz_domain::PrincipalType::Agent,
        name: "provisioner".to_string(),
        avatar_url: None,
        scopes: vec![
            "messages:read".into(),
            "messages:write".into(),
            "events:read".into(),
        ],
        secret_hash: None,
        disabled: false,
        deleted_at: None,
        channel_visibility: choruz_domain::ChannelVisibility::Visible,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        user_id: None,
    };
    app.inject_principal(principal);

    Ok(Some(id))
}
