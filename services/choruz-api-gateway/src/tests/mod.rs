#![allow(dead_code, unused_imports)]

use crate::attachments::{AttachmentRecord, UploadAttachmentRequest};
use crate::handlers_workspace_sessions::native_session_import_lock_key;
use crate::test_support::api_test_env_lock;
use crate::{
    ApiError, LocalAuthConfig, persist_principal_to_db, router, router_with_attachment_root,
    router_with_runtime,
};
use axum::{
    Json as AxumJson, Router,
    body::{Body, to_bytes},
    extract::{State, WebSocketUpgrade},
    http::{HeaderMap, Method, Request, StatusCode, header::CONTENT_TYPE},
    response::IntoResponse,
    routing::{get, post},
};
use choruz_agent_runtime::{
    AuditActor, BindingState, CodexTerminalCaptureInput, CreateBindingInput, DriverType,
    RuntimeStore,
};
use choruz_application::{
    ChannelTaskStatus, CreateAgentRequest, CreateChannelTaskRequest, CreateCompanyRequest,
    CreateDirectConversationRequest, CreateGroupRequest, CreatePrincipalRequest, ListEventsQuery,
    ListMessagesQuery, NullablePatch, PatchChannelTaskRequest, SendMessageRequest,
    SetEventWebhookRequest,
};
use choruz_auth::{SessionClaims, issue_session_token};
use choruz_common::AppError;
use choruz_domain::PrincipalType;
use choruz_session::{CommandStatus, CommandStatusUpdate, InsertCommand, PgSessionStore};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex as StdMutex, MutexGuard, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};
use tokio_postgres::NoTls;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tower::util::ServiceExt;
use uuid::Uuid;

mod agents;
mod channel_tasks;
mod contracts;
mod conversations;
mod harness_logins;
mod messages;
mod observability;
mod runtime;
mod sessions_and_routes;
mod sync;
mod threads;
mod workflow_tasks;

static CHANNEL_TASK_ENV_LOCK: StdMutex<()> = StdMutex::new(());

struct ChannelTaskEnvGuard {
    _guard: MutexGuard<'static, ()>,
    saved: Option<String>,
    gateway_secret: Option<String>,
    gateway_url: Option<String>,
}

impl ChannelTaskEnvGuard {
    fn enabled() -> Self {
        let guard = CHANNEL_TASK_ENV_LOCK.lock().expect("channel task env lock");
        let saved = env::var("CHORUZ_PLUGINS").ok();
        let gateway_secret = env::var("CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET").ok();
        let gateway_url = env::var("CHORUZ_REMOTE_CONTROL_GATEWAY_URL").ok();
        // SAFETY: this test guard serializes mutations of this env var inside
        // the choruz-api-gateway test process.
        unsafe {
            env::set_var("CHORUZ_PLUGINS", "kanban,pixel-world");
        }
        Self {
            _guard: guard,
            saved,
            gateway_secret,
            gateway_url,
        }
    }

    fn disabled() -> Self {
        let guard = CHANNEL_TASK_ENV_LOCK.lock().expect("channel task env lock");
        let saved = env::var("CHORUZ_PLUGINS").ok();
        let gateway_secret = env::var("CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET").ok();
        let gateway_url = env::var("CHORUZ_REMOTE_CONTROL_GATEWAY_URL").ok();
        // SAFETY: this test guard serializes mutations of this env var inside
        // the choruz-api-gateway test process.
        unsafe {
            env::set_var("CHORUZ_PLUGINS", "pixel-world");
        }
        Self {
            _guard: guard,
            saved,
            gateway_secret,
            gateway_url,
        }
    }

    fn remote_control() -> Self {
        Self::remote_control_at(None)
    }

    fn remote_control_with_gateway(gateway: &str) -> Self {
        Self::remote_control_at(Some(gateway))
    }

    fn remote_control_at(gateway: Option<&str>) -> Self {
        let guard = CHANNEL_TASK_ENV_LOCK.lock().expect("plugin env lock");
        let saved = env::var("CHORUZ_PLUGINS").ok();
        let gateway_secret = env::var("CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET").ok();
        let gateway_url = env::var("CHORUZ_REMOTE_CONTROL_GATEWAY_URL").ok();
        // SAFETY: this test guard serializes mutations of this env var inside
        // the choruz-api-gateway test process.
        unsafe {
            env::set_var("CHORUZ_PLUGINS", "remote-control");
            // Exercise the local signed-ticket contract. Hosted opaque-ticket
            // issuance is covered by the gateway package without making this
            // API suite depend on the public Worker.
            env::set_var(
                "CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET",
                "test-remote-control-gateway-secret-32b",
            );
            if let Some(gateway) = gateway {
                env::set_var("CHORUZ_REMOTE_CONTROL_GATEWAY_URL", gateway);
            } else {
                env::remove_var("CHORUZ_REMOTE_CONTROL_GATEWAY_URL");
            }
        }
        Self {
            _guard: guard,
            saved,
            gateway_secret,
            gateway_url,
        }
    }
}

impl Drop for ChannelTaskEnvGuard {
    fn drop(&mut self) {
        // SAFETY: this guard still holds CHANNEL_TASK_ENV_LOCK while restoring.
        unsafe {
            match &self.saved {
                Some(value) => env::set_var("CHORUZ_PLUGINS", value),
                None => env::remove_var("CHORUZ_PLUGINS"),
            }
            match &self.gateway_secret {
                Some(value) => env::set_var("CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET", value),
                None => env::remove_var("CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET"),
            }
            match &self.gateway_url {
                Some(value) => env::set_var("CHORUZ_REMOTE_CONTROL_GATEWAY_URL", value),
                None => env::remove_var("CHORUZ_REMOTE_CONTROL_GATEWAY_URL"),
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PairingGatewayEvent {
    Connected,
    Closed,
}

async fn spawn_pairing_gateway() -> (String, mpsc::UnboundedReceiver<PairingGatewayEvent>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let app = Router::new().route(
        "/connect",
        get(move |upgrade: WebSocketUpgrade| {
            let events = events_tx.clone();
            async move {
                upgrade.on_upgrade(move |mut socket| async move {
                    let _ = events.send(PairingGatewayEvent::Connected);
                    while socket.recv().await.is_some() {}
                    let _ = events.send(PairingGatewayEvent::Closed);
                })
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), events_rx)
}

#[derive(Clone)]
struct WebhookReceiverState {
    attempts: Arc<AtomicUsize>,
    payloads: Arc<Mutex<Vec<Value>>>,
    headers: Arc<Mutex<Vec<HeaderMap>>>,
}

struct TestDatabase {
    database_url: String,
    admin_database_url: String,
    database_name: String,
}

impl TestDatabase {
    async fn create() -> Self {
        let admin_database_url = connection_string("postgres");
        let database_name = format!("choruz_api_gateway_{}", Uuid::now_v7().simple());
        let (admin_client, connection) = connect_admin_database(&admin_database_url).await;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        admin_client
            .execute(&format!("CREATE DATABASE {database_name}"), &[])
            .await
            .expect("create temp db");

        let database_url = connection_string(&database_name);
        let database = Self {
            database_url,
            admin_database_url,
            database_name,
        };
        database.apply_migrations().await;
        database
    }

    async fn create_without_migrations() -> Self {
        let admin_database_url = connection_string("postgres");
        let database_name = format!("choruz_api_gateway_{}", Uuid::now_v7().simple());
        let (admin_client, connection) = connect_admin_database(&admin_database_url).await;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        admin_client
            .execute(&format!("CREATE DATABASE {database_name}"), &[])
            .await
            .expect("create temp db");

        Self {
            database_url: connection_string(&database_name),
            admin_database_url,
            database_name,
        }
    }

    async fn apply_migrations(&self) {
        let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls)
            .await
            .expect("connect temp db");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        apply_migration_files(&client, |_| true).await;
    }

    async fn apply_migrations_through(&self, max_filename: &str) {
        let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls)
            .await
            .expect("connect temp db");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        apply_migration_files(&client, |file_name| file_name <= max_filename).await;
    }

    async fn apply_migration(&self, filename: &str) {
        let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls)
            .await
            .expect("connect temp db");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        apply_migration_files(&client, |file_name| file_name == filename).await;
    }

    async fn audit_actions(&self) -> Vec<String> {
        let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls)
            .await
            .expect("connect temp db");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        client
            .query("SELECT action FROM audit_log ORDER BY created_at ASC", &[])
            .await
            .expect("query audit log")
            .into_iter()
            .map(|row| row.get::<_, String>(0))
            .collect()
    }
}

async fn apply_migration_files(
    client: &tokio_postgres::Client,
    should_apply: impl Fn(&str) -> bool,
) {
    let mut files = fs::read_dir(migrations_dir())
        .expect("read migrations dir")
        .map(|entry| entry.expect("migration dir entry").path())
        .collect::<Vec<_>>();
    files.sort();

    for file in files {
        let file_name = migration_file_name(&file);
        if !should_apply(&file_name) {
            continue;
        }
        let sql = fs::read_to_string(&file).expect("read migration file");
        if sql.contains("CONCURRENTLY") {
            let executable_sql = sql
                .lines()
                .filter(|line| !line.trim_start().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n");
            for statement in executable_sql
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                client
                    .batch_execute(statement)
                    .await
                    .expect("apply non-transactional migration statement");
            }
        } else {
            client.batch_execute(&sql).await.expect("apply migration");
        }
    }
}

fn migration_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .expect("migration file name")
        .to_owned()
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let admin_database_url = self.admin_database_url.clone();
        let database_name = self.database_name.clone();
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("create cleanup runtime");
            runtime.block_on(async move {
                let (client, connection) = tokio_postgres::connect(&admin_database_url, NoTls)
                    .await
                    .expect("connect operator db for cleanup");
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                let _ = client
                    .execute(
                        "SELECT pg_terminate_backend(pid)
                         FROM pg_stat_activity
                         WHERE datname = $1
                           AND pid <> pg_backend_pid()",
                        &[&database_name],
                    )
                    .await;
                let _ = client
                    .execute(&format!("DROP DATABASE IF EXISTS {database_name}"), &[])
                    .await;
            });
        });
        let _ = handle.join();
    }
}

async fn webhook_receiver(
    State(state): State<WebhookReceiverState>,
    headers: HeaderMap,
    AxumJson(payload): AxumJson<Value>,
) -> StatusCode {
    let attempt = state.attempts.fetch_add(1, Ordering::SeqCst) + 1;
    if attempt == 1 {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    state.payloads.lock().await.push(payload);
    state.headers.lock().await.push(headers);
    StatusCode::OK
}

fn session_token(principal: &choruz_domain::Principal) -> String {
    issue_session_token(
        &SessionClaims {
            principal_id: principal.id.clone(),
            workspace_id: principal.workspace_id.clone(),
            display_name: principal.name.clone(),
            expires_at_epoch_s: chrono::Utc::now().timestamp() + 3600,
        },
        &LocalAuthConfig::from_env().session_secret,
    )
    .unwrap()
}

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn host_start_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../infra/host/start.sh")
}

fn isolated_test_dir(name: &str) -> PathBuf {
    let dir = env::temp_dir().join(format!("choruz-api-{name}-{}", Uuid::now_v7().simple()));
    fs::create_dir_all(&dir).expect("create isolated test dir");
    dir
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let guard = Self {
            key,
            previous: env::var_os(key),
        };
        unsafe {
            env::set_var(key, value);
        }
        guard
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(previous) = &self.previous {
                env::set_var(self.key, previous);
            } else {
                env::remove_var(self.key);
            }
        }
    }
}

#[cfg(unix)]
fn write_executable_script(path: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, body).expect("write executable script");
    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("chmod executable script");
}

async fn conversation_event_count(database_url: &str, conversation_id: &str) -> i64 {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .expect("connect for conversation event count");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .query_one(
            "SELECT COUNT(*)::bigint FROM conversation_events WHERE conversation_id = $1",
            &[&conversation_id],
        )
        .await
        .expect("count conversation events")
        .get(0)
}

fn connection_string(database_name: &str) -> String {
    let host = env::var("CHORUZ_PG_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = env::var("CHORUZ_PG_PORT").unwrap_or_else(|_| "5432".into());
    let user = env::var("CHORUZ_PG_USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| env::var("USER").ok())
        .unwrap_or_else(|| "postgres".into());
    let password = env::var("CHORUZ_PG_PASSWORD").ok();

    let mut connection = format!("host={host} port={port} user={user} dbname={database_name}");
    if let Some(password) = password.filter(|value| !value.trim().is_empty()) {
        connection.push_str(" password=");
        connection.push_str(&password);
    }
    connection
}

async fn connect_admin_database(
    admin_database_url: &str,
) -> (
    tokio_postgres::Client,
    tokio_postgres::Connection<tokio_postgres::Socket, tokio_postgres::tls::NoTlsStream>,
) {
    match tokio_postgres::connect(admin_database_url, NoTls).await {
        Ok(connection) => connection,
        Err(first_error) => {
            ensure_postgres_started().unwrap_or_else(|message| {
                panic!("failed to auto-start postgres: {message}; initial error: {first_error}")
            });
            tokio_postgres::connect(admin_database_url, NoTls)
                .await
                .unwrap_or_else(|error| panic!("connect operator db after auto-start: {error}"))
        }
    }
}

fn ensure_postgres_started() -> Result<(), String> {
    static START_ONCE: OnceLock<Result<(), String>> = OnceLock::new();
    START_ONCE
        .get_or_init(|| {
            let output = Command::new("bash")
                .arg(host_start_script())
                .output()
                .map_err(|error| format!("spawn start.sh: {error}"))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(format!(
                    "start.sh exited with {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                ))
            }
        })
        .clone()
}

fn audit_actor(principal: &choruz_domain::Principal) -> AuditActor {
    AuditActor {
        actor_id: principal.id.clone(),
        workspace_id: principal.workspace_id.clone(),
    }
}

/// Seed a principal row into the test database so DbService can find it.
async fn seed_principal_to_db(database_url: &str, p: &choruz_domain::Principal) {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .expect("connect for principal seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let type_str = match p.principal_type {
        PrincipalType::Human => "human",
        PrincipalType::Agent => "agent",
    };
    let channel_visibility = match p.channel_visibility {
        choruz_domain::ChannelVisibility::Visible => "visible",
        choruz_domain::ChannelVisibility::Internal => "internal",
    };
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name, secret_hash, disabled, channel_visibility, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (id) DO NOTHING",
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
        .expect("seed principal to DB");
}

/// Seed a conversation + members into the test database so DbService can find it.
async fn seed_conversation_to_db(database_url: &str, conv: &choruz_domain::Conversation) {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .expect("connect for conversation seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let type_str = match conv.conversation_type {
        choruz_domain::ConversationType::Direct => "direct",
        choruz_domain::ConversationType::Group => "group",
    };
    client
        .execute(
            "INSERT INTO conversation (id, workspace_id, type, name, creator_id, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO NOTHING",
            &[
                &conv.id,
                &conv.workspace_id,
                &type_str,
                &conv.name,
                &conv.creator_id,
                &conv.created_at,
                &conv.updated_at,
            ],
        )
        .await
        .expect("seed conversation to DB");
    for member in conv.members.values() {
        client
            .execute(
                "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (conv_id, principal_id) DO NOTHING",
                &[&conv.id, &member.principal_id, &member.joined_at],
            )
            .await
            .expect("seed conversation member to DB");
    }
}

async fn api_send_text_message(
    router: Router,
    actor: &choruz_domain::Principal,
    conversation_id: &str,
    idempotency_key: &str,
    content: &str,
) -> StatusCode {
    api_send_text_message_with_actor_id(
        router,
        actor,
        &actor.id,
        conversation_id,
        idempotency_key,
        content,
    )
    .await
}

async fn api_send_text_message_with_actor_id(
    router: Router,
    session_principal: &choruz_domain::Principal,
    actor_id: &str,
    conversation_id: &str,
    idempotency_key: &str,
    content: &str,
) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/messages")
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(session_principal)),
                )
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&SendMessageRequest {
                        actor_id: actor_id.to_string(),
                        conversation_id: conversation_id.to_string(),
                        idempotency_key: idempotency_key.to_string(),
                        content: content.to_string(),
                        content_type: "text".into(),
                        metadata: json!({}),
                        trace_id: None,
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn api_json_request(
    router: Router,
    principal: &choruz_domain::Principal,
    method: Method,
    uri: String,
) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(principal)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };
    (status, json)
}

async fn api_json_payload_request(
    router: Router,
    principal: &choruz_domain::Principal,
    method: Method,
    uri: String,
    payload: Value,
) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(principal)),
                )
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    };
    (status, json)
}

fn validation_detail(body: &Value) -> String {
    body["error"]["detail"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn assert_safe_detail(body: &Value, principal_id: &str, case: &str) {
    assert_safe_detail_excludes(body, principal_id, &[], case);
}

fn assert_safe_detail_excludes(
    body: &Value,
    principal_id: &str,
    extra_forbidden: &[&str],
    case: &str,
) {
    let detail = validation_detail(body);
    assert!(
        !detail.is_empty(),
        "{case}: validation response must carry a non-empty error.detail, got {body:?}"
    );
    assert!(
        detail.contains("assignee"),
        "{case}: validation detail must name the rejected concept (\"assignee\"), got {detail:?}"
    );
    assert!(
        !detail.contains(principal_id),
        "{case}: validation detail must not echo assignee principal id, got {detail:?}"
    );
    for forbidden in extra_forbidden {
        assert!(
            !forbidden.is_empty() && !detail.contains(forbidden),
            "{case}: validation detail must not echo unsafe assignee identifier {forbidden:?}, got {detail:?}"
        );
    }
}

async fn api_list_messages(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
) -> (StatusCode, Value) {
    api_list_messages_with_principal_id(router, principal, &principal.id, conversation_id).await
}

async fn api_message_page(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
    query: &str,
) -> (StatusCode, Value) {
    let separator = if query.is_empty() { "" } else { "&" };
    api_json_request(
        router,
        principal,
        Method::GET,
        format!(
            "/v1/conversations/{conversation_id}/message-page?principal_id={}{}{}",
            principal.id, separator, query
        ),
    )
    .await
}

async fn api_list_messages_with_principal_id(
    router: Router,
    session_principal: &choruz_domain::Principal,
    principal_id: &str,
    conversation_id: &str,
) -> (StatusCode, Value) {
    api_json_request(
        router,
        session_principal,
        Method::GET,
        format!("/v1/conversations/{conversation_id}/messages?principal_id={principal_id}"),
    )
    .await
}

async fn api_search_messages(
    router: Router,
    principal: &choruz_domain::Principal,
    query: &str,
    conversation_id: Option<&str>,
) -> (StatusCode, Value) {
    api_search_messages_with_principal_id(router, principal, &principal.id, query, conversation_id)
        .await
}

async fn api_search_messages_with_principal_id(
    router: Router,
    session_principal: &choruz_domain::Principal,
    principal_id: &str,
    query: &str,
    conversation_id: Option<&str>,
) -> (StatusCode, Value) {
    let mut uri = format!("/v1/messages/search?principal_id={principal_id}&q={query}");
    if let Some(conversation_id) = conversation_id {
        uri.push_str("&conversation_id=");
        uri.push_str(conversation_id);
    }
    api_json_request(router, session_principal, Method::GET, uri).await
}

async fn api_list_conversations(
    router: Router,
    principal: &choruz_domain::Principal,
) -> (StatusCode, Value) {
    api_list_conversations_with_principal_id(router, principal, &principal.id).await
}

async fn api_list_conversations_with_principal_id(
    router: Router,
    session_principal: &choruz_domain::Principal,
    principal_id: &str,
) -> (StatusCode, Value) {
    api_json_request(
        router,
        session_principal,
        Method::GET,
        format!("/v1/conversations?principal_id={principal_id}"),
    )
    .await
}

async fn api_console_snapshot(
    router: Router,
    principal: &choruz_domain::Principal,
) -> (StatusCode, Value) {
    api_json_request(router, principal, Method::GET, "/v1/console".into()).await
}

async fn api_bootstrap(
    router: Router,
    principal: &choruz_domain::Principal,
    query: &str,
) -> (StatusCode, Value) {
    let suffix = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    api_json_request(
        router,
        principal,
        Method::GET,
        format!("/v1/bootstrap{suffix}"),
    )
    .await
}

async fn api_sync(
    router: Router,
    principal: &choruz_domain::Principal,
    query: &str,
) -> (StatusCode, Value) {
    let suffix = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    api_json_request(router, principal, Method::GET, format!("/v1/sync{suffix}")).await
}

async fn api_pin_conversation(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/v1/conversations/{conversation_id}/pin"))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(principal)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn api_unpin_conversation(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/v1/conversations/{conversation_id}/pin"))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(principal)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn api_archive_conversation(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/v1/conversations/{conversation_id}/archive"))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(principal)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn api_unarchive_conversation(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/v1/conversations/{conversation_id}/archive"))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(principal)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn api_hide_conversation(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/v1/conversations/{conversation_id}/hide"))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(principal)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn api_restore_hidden_conversation(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/v1/conversations/{conversation_id}/hide"))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(principal)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn db_pinned_at(
    database_url: &str,
    principal_id: &str,
    conversation_id: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .expect("connect for pin lookup");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .query_opt(
            "SELECT pinned_at FROM conversation_pin WHERE principal_id = $1 AND conv_id = $2",
            &[&principal_id, &conversation_id],
        )
        .await
        .expect("query pin")
        .map(|row| row.get("pinned_at"))
}

async fn db_pin_count(database_url: &str, principal_id: &str, conversation_id: &str) -> i64 {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .expect("connect for pin count");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM conversation_pin WHERE principal_id = $1 AND conv_id = $2",
            &[&principal_id, &conversation_id],
        )
        .await
        .expect("count pins")
        .get(0)
}

async fn api_view_conversation(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/conversations/{conversation_id}/view"))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(principal)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn api_remove_group_member(
    router: Router,
    actor: &choruz_domain::Principal,
    conversation_id: &str,
    target_id: &str,
) -> StatusCode {
    api_remove_group_member_with_actor_id(router, actor, &actor.id, conversation_id, target_id)
        .await
}

async fn api_remove_group_member_with_actor_id(
    router: Router,
    session_principal: &choruz_domain::Principal,
    actor_id: &str,
    conversation_id: &str,
    target_id: &str,
) -> StatusCode {
    router
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!(
                    "/v1/groups/{conversation_id}/members/{target_id}?actor_id={actor_id}"
                ))
                .header(
                    "authorization",
                    format!("Bearer {}", session_token(session_principal)),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// Build a router backed by a specific test database (no RuntimeStore).
fn router_with_db(app: choruz_application::ChatApp, database_url: &str) -> Router {
    let attachment_root =
        std::env::temp_dir().join(format!("choruz-attachments-{}", choruz_common::new_id()));
    router_with_runtime(
        app,
        attachment_root,
        LocalAuthConfig::from_env(),
        RuntimeStore::new(database_url),
        PgSessionStore::new(database_url),
        choruz_store::EventStore::new(database_url),
    )
}

fn runtime_router(app: choruz_application::ChatApp, runtime: RuntimeStore) -> Router {
    runtime_router_with_db(
        app,
        runtime,
        &choruz_common::PgConfig::from_env().to_connect_string(),
    )
}

fn runtime_router_with_db(
    app: choruz_application::ChatApp,
    runtime: RuntimeStore,
    database_url: &str,
) -> Router {
    let attachment_root =
        std::env::temp_dir().join(format!("choruz-attachments-{}", choruz_common::new_id()));
    router_with_runtime(
        app,
        attachment_root,
        LocalAuthConfig::from_env(),
        runtime,
        PgSessionStore::new(database_url),
        choruz_store::EventStore::new(database_url),
    )
}

fn prometheus_metric_value(text: &str, metric_name: &str) -> u64 {
    text.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(' ')?;
            (name == metric_name).then(|| value.parse::<u64>().expect("metric value should be u64"))
        })
        .unwrap_or_else(|| panic!("missing metric line: {metric_name}"))
}

async fn metrics_text(router: Router) -> String {
    let response = router
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[test]
fn openapi_documents_channel_task_contract() {
    let openapi = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openapi/choruz.yaml"),
    )
    .expect("read openapi contract");

    for expected in [
        "/v1/conversations/{conversationId}/tasks:",
        "operationId: createChannelTask",
        "operationId: listChannelTasks",
        "/v1/conversations/{conversationId}/tasks/from-message:",
        "operationId: createChannelTaskFromMessage",
        "/v1/tasks/{taskId}:",
        "operationId: getChannelTask",
        "operationId: patchChannelTask",
        "ChannelTask:",
        "CreateChannelTaskFromMessageRequest:",
        "PatchChannelTaskRequest:",
        "Task key or idempotency conflict not covered by source-message dedupe",
    ] {
        assert!(
            openapi.contains(expected),
            "missing channel task contract fragment: {expected}"
        );
    }

    let from_message_section = openapi
        .split("  /v1/conversations/{conversationId}/tasks/from-message:")
        .nth(1)
        .and_then(|section| section.split("  /v1/tasks/{taskId}:").next())
        .expect("extract from-message OpenAPI section");
    for expected_status in ["\"201\":", "\"200\":", "\"400\":", "\"403\":", "\"409\":"] {
        assert!(
            from_message_section.contains(expected_status),
            "from-message contract missing response status {expected_status}"
        );
    }
    let from_message_schema = openapi
        .split("    CreateChannelTaskFromMessageRequest:")
        .nth(1)
        .and_then(|section| section.split("    PatchChannelTaskRequest:").next())
        .expect("extract from-message request schema");
    assert!(
        from_message_schema
            .contains("        context_label:\n          oneOf:\n            - type: string\n            - type: \"null\""),
        "from-message context_label must be nullable"
    );

    let generic_create_section = openapi
        .split("  /v1/conversations/{conversationId}/tasks:")
        .nth(1)
        .and_then(|section| {
            section
                .split("  /v1/conversations/{conversationId}/tasks/from-message:")
                .next()
        })
        .expect("extract generic create OpenAPI section");
    for expected_status in ["\"201\":", "\"200\":", "\"400\":", "\"403\":", "\"409\":"] {
        assert!(
            generic_create_section.contains(expected_status),
            "generic channel task create contract missing response status {expected_status}"
        );
    }
    assert!(
        !generic_create_section.contains("\"501\":"),
        "generic channel task create must not advertise an unimplemented response"
    );

    for forbidden in [
        "        description:\n          type: string",
        "        description:\n          oneOf:",
    ] {
        assert!(
            !openapi.contains(forbidden),
            "forbidden channel task contract fragment is present: {forbidden}"
        );
    }
}

async fn next_ws_json<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .expect("sync WebSocket frame timeout")
            .expect("sync WebSocket closed")
            .expect("sync WebSocket error");
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).expect("valid sync JSON frame");
        }
    }
}

async fn connect_sync_socket(
    address: std::net::SocketAddr,
    principal: &choruz_domain::Principal,
    device_id: &str,
    cursor: u64,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!("ws://{address}/v1/ws/sync?device_id={device_id}&cursor={cursor}")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", session_token(principal))
            .parse()
            .unwrap(),
    );
    connect_async(request).await.expect("connect sync socket").0
}

/// Helper: send a message via POST /v1/messages with arbitrary metadata.
async fn api_send_with_metadata(
    router: Router,
    principal: &choruz_domain::Principal,
    conversation_id: &str,
    content: &str,
    idempotency_key: &str,
    metadata: Value,
) -> (StatusCode, Value) {
    api_json_payload_request(
        router,
        principal,
        Method::POST,
        "/v1/messages".into(),
        json!({
            "actor_id": principal.id.clone(),
            "conversation_id": conversation_id,
            "idempotency_key": idempotency_key,
            "content": content,
            "content_type": "text/plain",
            "metadata": metadata,
        }),
    )
    .await
}

/// Helper: read reply_event_id straight from conversation_events.
async fn db_reply_event_id(database_url: &str, event_id: &str) -> Option<String> {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .expect("connect for thread assert");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .query_one(
            "SELECT reply_event_id FROM conversation_events WHERE event_id = $1",
            &[&event_id],
        )
        .await
        .expect("event row")
        .get(0)
}

/// Helper: read conversation.total_msg_count.
async fn db_total_msg_count(database_url: &str, conversation_id: &str) -> i64 {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .expect("connect for counter assert");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .query_one(
            "SELECT total_msg_count FROM conversation WHERE id = $1",
            &[&conversation_id],
        )
        .await
        .expect("conversation row")
        .get(0)
}
