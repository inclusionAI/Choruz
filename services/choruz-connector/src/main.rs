use std::{
    env, fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant},
};

use choruz_agent_runtime::headless::{
    HeadlessDriver, ParsedOutput, configure_command_workspace, parse_output,
    prepare_harness_account_env, validate_model,
};
use choruz_harness_login::{AccountProbe, LoginJob, LoginSink, run_login};
use choruz_host_runtime::inbox::{
    IncomingAttachment, incoming_attachments, stage_incoming_attachments,
};
use choruz_host_runtime::instructions::ensure_claude_md;
use choruz_host_runtime::link::{DeviceIdentity, LINK_PATH, run_device_link};
use choruz_host_runtime::outbox::{
    ShippedOutboxCommand, collect_outbox_commands, commands_from_frames, shipments,
    spool_shipped_commands, spooled_binding_ids, take_spooled_commands,
};
use choruz_host_runtime::{TerminalPool, new_terminal_pool};
use choruz_relay_client::{RelayHttpClient, RelayStream, RemoteCredentials, StreamMessage};
use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::{process::Command, sync::Semaphore, task::JoinSet};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{error, info, warn};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const HOST_HEARTBEAT: Duration = Duration::from_secs(15);
const COMMAND_HEARTBEAT: Duration = Duration::from_secs(10);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConnectorConfig {
    #[serde(default)]
    api_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay: Option<RemoteCredentials>,
    host_id: String,
    host_name: String,
    host_token: String,
    #[serde(default = "default_concurrency")]
    max_concurrency: usize,
}

#[derive(Debug, Deserialize)]
struct HostView {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct PairResponse {
    host: HostView,
    host_token: String,
}

#[derive(Debug, Deserialize)]
struct RelayPairRequest {
    gateway_url: String,
    credential: String,
    runtime_host_code: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaimedCommand {
    command_id: String,
    attempt_id: String,
    binding_id: String,
    agent_id: String,
    conversation_id: String,
    turn_id: String,
    prompt: String,
    driver_type: String,
    workspace_path: String,
    model: Option<String>,
    external_session_id: Option<String>,
    #[serde(default)]
    fork_session: bool,
    harness_account: Option<ClaimedHarnessAccount>,
    #[serde(default)]
    metadata: serde_json::Value,
    #[serde(default)]
    preflight: Option<choruz_host_runtime::harness::ExecutionTeam>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaimedHarnessAccount {
    id: String,
    profile_kind: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaimedHarnessAccountLogin {
    login_id: String,
    account_id: String,
    driver_type: String,
    profile_kind: String,
}

#[derive(Debug, Deserialize)]
struct LoginCallback {
    code: String,
}

#[derive(Debug, Serialize)]
struct PublishLogin<'a> {
    authorization_url: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_code: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct CompleteLogin<'a> {
    account_fingerprint: &'a str,
    subscription_type: Option<&'a str>,
    models: &'a serde_json::Value,
    usage: &'a serde_json::Value,
}

#[derive(Debug, Serialize)]
struct CompleteCommand<'a> {
    attempt_id: &'a str,
    succeeded: bool,
    contents: &'a [String],
    error: Option<&'a str>,
    tool_calls_count: i32,
    execution_duration_ms: i64,
    external_session_id: Option<&'a str>,
    clear_external_session: bool,
}

fn default_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 16)
}

fn default_config_path() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or("HOME is not set; pass --config <path>")?;
    Ok(PathBuf::from(home).join(".choruz").join("connector.json"))
}

fn value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

fn config_path(args: &[String]) -> Result<PathBuf, String> {
    value(args, "--config")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(default_config_path)
}

fn endpoint(config: &ConnectorConfig, path: &str) -> String {
    format!("{}{}", config.api_url.trim_end_matches('/'), path)
}

fn save_config(path: &Path, config: &ConnectorConfig) -> Result<(), String> {
    let parent = path.parent().ok_or("config path has no parent")?;
    fs::create_dir_all(parent).map_err(|error| format!("create config directory: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("write connector config: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("write connector config: {error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("install connector config: {error}"))
}

fn load_config(path: &Path) -> Result<ConnectorConfig, String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let config: ConnectorConfig = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse connector config: {error}"))?;
    if config.api_url.trim().is_empty() && config.relay.is_none() {
        return Err("connector config has no controller transport".into());
    }
    if config.host_id.trim().is_empty()
        || config.host_token.trim().is_empty()
        || config.max_concurrency == 0
        || config.max_concurrency > 64
    {
        return Err("connector config contains invalid values".into());
    }
    Ok(config)
}

async fn pair(args: &[String]) -> Result<(), String> {
    let api_url = value(args, "--api-url").ok_or("pair requires --api-url")?;
    let code = value(args, "--code").ok_or("pair requires --code")?;
    let name = value(args, "--name").ok_or("pair requires --name")?;
    if code.len() != 8 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("pairing code must contain exactly 8 digits".into());
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!(
            "{}/v1/runtime-host-pairings/redeem",
            api_url.trim_end_matches('/')
        ))
        .json(&serde_json::json!({ "code": code, "name": name }))
        .send()
        .await
        .map_err(|error| format!("pair with Choruz: {error}"))?;
    if response.status() != StatusCode::CREATED {
        return Err(format!("pairing rejected with HTTP {}", response.status()));
    }
    let paired: PairResponse = response
        .json()
        .await
        .map_err(|error| format!("read pairing response: {error}"))?;
    let path = config_path(args)?;
    save_config(
        &path,
        &ConnectorConfig {
            api_url,
            relay: None,
            host_id: paired.host.id,
            host_name: paired.host.name,
            host_token: paired.host_token,
            max_concurrency: value(args, "--max-concurrency")
                .map(|raw| raw.parse::<usize>())
                .transpose()
                .map_err(|_| "--max-concurrency must be an integer")?
                .unwrap_or_else(default_concurrency)
                .clamp(1, 64),
        },
    )?;
    println!(
        "Connected {}. Credentials saved to {}.",
        name,
        path.display()
    );
    Ok(())
}

/// A loopback HTTP proxy that carries the connector's API calls through the
/// relay; the returned client also opens the relay streams the host link
/// rides on.
async fn relay_proxy(
    credentials: RemoteCredentials,
) -> Result<(String, tokio::task::JoinHandle<()>, RelayHttpClient), String> {
    use axum::{
        Router,
        body::{Body, Bytes},
        extract::{OriginalUri, State},
        http::{HeaderMap, Method, Response, StatusCode as AxumStatus},
        routing::any,
    };

    async fn forward(
        State(client): State<RelayHttpClient>,
        method: Method,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response<Body> {
        let headers = headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_owned()))
            })
            .collect();
        let relay_path = relay_api_path(&uri.to_string());
        match client
            .request(method.as_str(), relay_path, headers, body.to_vec())
            .await
        {
            Ok(response) => {
                let mut builder = Response::builder().status(response.status);
                for (name, value) in response.headers {
                    builder = builder.header(name, value);
                }
                builder.body(Body::from(response.body)).unwrap_or_else(|_| {
                    Response::builder()
                        .status(AxumStatus::BAD_GATEWAY)
                        .body(Body::from("invalid relayed response"))
                        .expect("static response")
                })
            }
            Err(error) => Response::builder()
                .status(AxumStatus::BAD_GATEWAY)
                .body(Body::from(error))
                .expect("static response"),
        }
    }

    let client = RelayHttpClient::start(credentials);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("bind relay proxy: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("inspect relay proxy: {error}"))?;
    let proxy_client = client.clone();
    let task = tokio::spawn(async move {
        let app = Router::new()
            .fallback(any(forward))
            .with_state(proxy_client);
        if let Err(error) = axum::serve(listener, app).await {
            warn!(%error, "relay proxy stopped");
        }
    });
    Ok((format!("http://{address}"), task, client))
}

fn relay_api_path(path: &str) -> String {
    path.strip_prefix("/v1/")
        .map_or_else(|| path.to_owned(), |rest| format!("/api/v1/{rest}"))
}

async fn pair_relay(args: &[String]) -> Result<(), String> {
    use std::io::Read;

    let mut input = Vec::new();
    std::io::stdin()
        .take(16 * 1024)
        .read_to_end(&mut input)
        .map_err(|error| format!("read relay pairing request: {error}"))?;
    let request: RelayPairRequest = serde_json::from_slice(&input)
        .map_err(|error| format!("decode relay pairing request: {error}"))?;
    if request.runtime_host_code.len() != 8
        || !request
            .runtime_host_code
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err("runtime host code must contain exactly 8 digits".into());
    }
    if request.name.trim().is_empty() || request.name.len() > 80 {
        return Err("runtime host name must contain between 1 and 80 characters".into());
    }
    let credentials =
        choruz_relay_client::pair(&request.gateway_url, &request.credential, &request.name).await?;
    let (api_url, proxy, _) = relay_proxy(credentials.clone()).await?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(35))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!("{api_url}/v1/runtime-host-pairings/redeem"))
        .json(&serde_json::json!({
            "code": request.runtime_host_code,
            "name": request.name.trim(),
        }))
        .send()
        .await
        .map_err(|error| format!("register runtime host: {error}"))?;
    if response.status() != StatusCode::CREATED {
        return Err(format!(
            "runtime host registration returned HTTP {}",
            response.status()
        ));
    }
    let paired: PairResponse = response
        .json()
        .await
        .map_err(|error| format!("read runtime host registration: {error}"))?;
    let path = config_path(args)?;
    save_config(
        &path,
        &ConnectorConfig {
            api_url: String::new(),
            relay: Some(credentials),
            host_id: paired.host.id.clone(),
            host_name: paired.host.name.clone(),
            host_token: paired.host_token,
            max_concurrency: value(args, "--max-concurrency")
                .map(|raw| raw.parse::<usize>())
                .transpose()
                .map_err(|_| "--max-concurrency must be an integer")?
                .unwrap_or_else(default_concurrency)
                .clamp(1, 64),
        },
    )?;
    proxy.abort();
    println!(
        "{}",
        serde_json::json!({
            "host_id": paired.host.id,
            "host_name": paired.host.name,
            "config_path": path,
        })
    );
    Ok(())
}

fn authenticated(
    request: reqwest::RequestBuilder,
    config: &ConnectorConfig,
) -> reqwest::RequestBuilder {
    request.header("x-choruz-host-token", &config.host_token)
}

async fn heartbeat(client: &Client, config: &ConnectorConfig) -> Result<(), reqwest::Error> {
    authenticated(
        client.post(endpoint(
            config,
            &format!("/v1/runtime-hosts/{}/heartbeat", config.host_id),
        )),
        config,
    )
    .send()
    .await?
    .error_for_status()?;
    Ok(())
}

async fn wait_for_controller(client: &Client, config: &ConnectorConfig) -> Result<(), String> {
    let mut delay = Duration::from_secs(1);
    loop {
        match heartbeat(client, config).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                let status = error.status();
                let reason = error.without_url().to_string();
                if status.is_some_and(|status| {
                    status.is_client_error()
                        && status != StatusCode::REQUEST_TIMEOUT
                        && status != StatusCode::TOO_MANY_REQUESTS
                }) {
                    return Err(reason);
                }
                warn!(host = %config.host_id, %reason, retry_seconds = delay.as_secs(), "waiting for controller heartbeat");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        }
    }
}

async fn claim(
    client: &Client,
    config: &ConnectorConfig,
) -> Result<Option<ClaimedCommand>, String> {
    let response = authenticated(
        client.post(endpoint(
            config,
            &format!(
                "/v1/runtime-hosts/{}/commands/claim?wait_ms=25000",
                config.host_id
            ),
        )),
        config,
    )
    .send()
    .await
    .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("command claim returned HTTP {}", response.status()));
    }
    response.json().await.map_err(|error| error.to_string())
}

async fn claim_harness_login(
    client: &Client,
    config: &ConnectorConfig,
) -> Result<Option<ClaimedHarnessAccountLogin>, String> {
    let response = authenticated(
        client.post(endpoint(
            config,
            &format!(
                "/v1/runtime-hosts/{}/harness-account-logins/claim",
                config.host_id
            ),
        )),
        config,
    )
    .send()
    .await
    .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "harness login claim returned HTTP {}",
            response.status()
        ));
    }
    response.json().await.map_err(|error| error.to_string())
}

async fn publish_harness_login(
    client: &Client,
    config: &ConnectorConfig,
    login: &ClaimedHarnessAccountLogin,
    url: &str,
    user_code: Option<&str>,
) -> Result<(), String> {
    let response = authenticated(
        client.post(endpoint(
            config,
            &format!(
                "/v1/runtime-hosts/{}/harness-account-logins/{}/publish",
                config.host_id, login.login_id
            ),
        )),
        config,
    )
    .json(&PublishLogin {
        authorization_url: url,
        user_code,
    })
    .send()
    .await
    .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "publish harness login returned HTTP {}",
            response.status()
        ))
    }
}

async fn claim_harness_login_callback(
    client: &Client,
    config: &ConnectorConfig,
    login: &ClaimedHarnessAccountLogin,
) -> Result<Option<LoginCallback>, String> {
    let response = authenticated(
        client.post(endpoint(
            config,
            &format!(
                "/v1/runtime-hosts/{}/harness-account-logins/{}/callback/claim",
                config.host_id, login.login_id
            ),
        )),
        config,
    )
    .send()
    .await
    .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "authorization code claim returned HTTP {}",
            response.status()
        ));
    }
    response.json().await.map_err(|error| error.to_string())
}

async fn fail_harness_login(
    client: &Client,
    config: &ConnectorConfig,
    login: &ClaimedHarnessAccountLogin,
    message: &str,
) {
    let _ = authenticated(
        client.post(endpoint(
            config,
            &format!(
                "/v1/runtime-hosts/{}/harness-account-logins/{}/fail",
                config.host_id, login.login_id
            ),
        )),
        config,
    )
    .json(&serde_json::json!({"error": message}))
    .send()
    .await;
}

async fn complete_harness_login(
    client: &Client,
    config: &ConnectorConfig,
    login: &ClaimedHarnessAccountLogin,
    probe: &AccountProbe,
) -> Result<(), String> {
    let response = authenticated(
        client.post(endpoint(
            config,
            &format!(
                "/v1/runtime-hosts/{}/harness-account-logins/{}/complete",
                config.host_id, login.login_id
            ),
        )),
        config,
    )
    .json(&CompleteLogin {
        account_fingerprint: &probe.fingerprint,
        subscription_type: probe.subscription_type.as_deref(),
        models: &probe.models,
        usage: &probe.usage,
    })
    .send()
    .await
    .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "complete harness login returned HTTP {}",
            response.status()
        ))
    }
}

async fn publish_harness_account_snapshot(
    client: &Client,
    config: &ConnectorConfig,
    login: &ClaimedHarnessAccountLogin,
    probe: &AccountProbe,
) -> Result<(), String> {
    let response = authenticated(
        client.post(endpoint(
            config,
            &format!(
                "/v1/runtime-hosts/{}/harness-accounts/{}/verify",
                config.host_id, login.account_id
            ),
        )),
        config,
    )
    .json(&CompleteLogin {
        account_fingerprint: &probe.fingerprint,
        subscription_type: probe.subscription_type.as_deref(),
        models: &probe.models,
        usage: &probe.usage,
    })
    .send()
    .await
    .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "verify harness account returned HTTP {}",
            response.status()
        ))
    }
}

struct HttpLoginSink<'a> {
    client: &'a Client,
    config: &'a ConnectorConfig,
    login: &'a ClaimedHarnessAccountLogin,
}

impl LoginSink for HttpLoginSink<'_> {
    async fn publish(
        &self,
        authorization_url: &str,
        user_code: Option<&str>,
    ) -> Result<(), String> {
        publish_harness_login(
            self.client,
            self.config,
            self.login,
            authorization_url,
            user_code,
        )
        .await
    }

    async fn take_callback(&self) -> Result<Option<String>, String> {
        Ok(
            claim_harness_login_callback(self.client, self.config, self.login)
                .await?
                .map(|callback| callback.code),
        )
    }

    async fn complete_authentication(&self, probe: &AccountProbe) -> Result<(), String> {
        complete_harness_login(self.client, self.config, self.login, probe).await
    }

    async fn publish_snapshot(&self, probe: &AccountProbe) -> Result<(), String> {
        publish_harness_account_snapshot(self.client, self.config, self.login, probe).await
    }
}

async fn execute_harness_login(
    client: Client,
    config: Arc<ConnectorConfig>,
    login: ClaimedHarnessAccountLogin,
) {
    let result = match HeadlessDriver::from_driver_type(&login.driver_type) {
        Some(driver @ (HeadlessDriver::Claude | HeadlessDriver::Codex)) => {
            let job = LoginJob {
                login_id: login.login_id.clone(),
                account_id: login.account_id.clone(),
                driver,
                profile_kind: login.profile_kind.clone(),
            };
            let sink = HttpLoginSink {
                client: &client,
                config: &config,
                login: &login,
            };
            run_login(&job, &sink, LOGIN_TIMEOUT).await.map(|outcome| {
                if let Some(reason) = outcome.snapshot_error {
                    warn!(login_id = %login.login_id, %reason, "Harness account snapshot refresh failed after login");
                }
            })
        }
        _ => Err("Remote account login is unsupported for this Harness".into()),
    };
    if let Err(reason) = result {
        warn!(login_id = %login.login_id, %reason, "remote Harness login failed");
        fail_harness_login(&client, &config, &login, &reason).await;
    }
}

async fn command_heartbeat(
    client: &Client,
    config: &ConnectorConfig,
    command: &ClaimedCommand,
) -> Result<(), String> {
    let response = authenticated(
        client.post(endpoint(
            config,
            &format!(
                "/v1/runtime-hosts/{}/commands/{}/heartbeat",
                config.host_id, command.command_id
            ),
        )),
        config,
    )
    .json(&serde_json::json!({ "attempt_id": command.attempt_id }))
    .send()
    .await
    .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "command heartbeat returned HTTP {}",
            response.status()
        ))
    }
}

async fn complete(
    client: &Client,
    config: &ConnectorConfig,
    command: &ClaimedCommand,
    result: &CompleteCommand<'_>,
) -> Result<(), String> {
    let response = authenticated(
        client.post(endpoint(
            config,
            &format!(
                "/v1/runtime-hosts/{}/commands/{}/complete",
                config.host_id, command.command_id
            ),
        )),
        config,
    )
    .json(result)
    .send()
    .await
    .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "command completion returned HTTP {}",
            response.status()
        ))
    }
}

async fn execute(
    client: Client,
    config: Arc<ConnectorConfig>,
    command: ClaimedCommand,
) -> Result<(), String> {
    let started = Instant::now();
    let Some(driver) = HeadlessDriver::from_driver_type(&command.driver_type) else {
        let message = format!("unsupported Harness {}", command.driver_type);
        return complete(
            &client,
            &config,
            &command,
            &CompleteCommand {
                attempt_id: &command.attempt_id,
                succeeded: false,
                contents: &[],
                error: Some(&message),
                tool_calls_count: 0,
                execution_duration_ms: 0,
                external_session_id: None,
                clear_external_session: false,
            },
        )
        .await;
    };
    let workspace = PathBuf::from(&command.workspace_path);
    if !workspace.is_absolute() || !workspace.is_dir() {
        let error = "configured workspace is not an accessible absolute directory";
        return complete(
            &client,
            &config,
            &command,
            &CompleteCommand {
                attempt_id: &command.attempt_id,
                succeeded: false,
                contents: &[],
                error: Some(error),
                tool_calls_count: 0,
                execution_duration_ms: 0,
                external_session_id: None,
                clear_external_session: false,
            },
        )
        .await;
    }
    if let Some(model) = command.model.as_deref()
        && let Err(message) = validate_model(model)
    {
        return complete(
            &client,
            &config,
            &command,
            &CompleteCommand {
                attempt_id: &command.attempt_id,
                succeeded: false,
                contents: &[],
                error: Some(message),
                tool_calls_count: 0,
                execution_duration_ms: 0,
                external_session_id: None,
                clear_external_session: false,
            },
        )
        .await;
    }
    info!(
        command_id = %command.command_id,
        agent_id = %command.agent_id,
        conversation_id = %command.conversation_id,
        turn_id = %command.turn_id,
        driver = driver.label(),
        "starting remote Agent turn"
    );
    let prompt = stage_command_attachments(&client, &config, &command, &workspace).await;
    let outbox_dir = tempfile::Builder::new()
        .prefix("choruz-connector-outbox-")
        .tempdir()
        .map_err(|error| format!("create turn outbox: {error}"))?;
    let outbox_path = outbox_dir.path().join("commands");
    let connector_executable =
        env::current_exe().map_err(|error| format!("locate connector helper: {error}"))?;
    // The device that runs the turn owns the workspace's instruction file,
    // exactly as the pipeline does for the gateway's device.
    let bootstrap = ensure_claude_md(&workspace, Some(&command.driver_type)).await;
    for (path, error) in &bootstrap.io_errors {
        warn!(path = %path.display(), %error, "instruction bootstrap failed");
    }
    let mut process_command = Command::new(driver.default_binary());
    configure_command_workspace(&mut process_command, driver, &workspace);
    let account_env = if let Some(account) = command.harness_account.as_ref()
        && account.profile_kind == "isolated"
    {
        let account_config = serde_json::json!({
            "harness_account_id": account.id,
            "harness_account_profile_kind": account.profile_kind,
        });
        match prepare_harness_account_env(driver, &account_config) {
            Ok(Some((key, value))) => {
                if let Err(error) = fs::create_dir_all(&value) {
                    let message = format!("create isolated Harness profile: {error}");
                    return complete(
                        &client,
                        &config,
                        &command,
                        &CompleteCommand {
                            attempt_id: &command.attempt_id,
                            succeeded: false,
                            contents: &[],
                            error: Some(&message),
                            tool_calls_count: 0,
                            execution_duration_ms: started
                                .elapsed()
                                .as_millis()
                                .min(i64::MAX as u128)
                                as i64,
                            external_session_id: None,
                            clear_external_session: false,
                        },
                    )
                    .await;
                }
                Some((key, value))
            }
            Ok(None) => None,
            Err(error) => {
                return complete(
                    &client,
                    &config,
                    &command,
                    &CompleteCommand {
                        attempt_id: &command.attempt_id,
                        succeeded: false,
                        contents: &[],
                        error: Some(&error),
                        tool_calls_count: 0,
                        execution_duration_ms: started.elapsed().as_millis().min(i64::MAX as u128)
                            as i64,
                        external_session_id: None,
                        clear_external_session: false,
                    },
                )
                .await;
            }
        }
    } else {
        None
    };
    if let Some((key, value)) = account_env {
        process_command.env(key, value);
    }
    let heartbeat_client = client.clone();
    let heartbeat_config = Arc::clone(&config);
    let heartbeat_command = command.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(COMMAND_HEARTBEAT);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(reason) =
                command_heartbeat(&heartbeat_client, &heartbeat_config, &heartbeat_command).await
            {
                warn!(command_id = %heartbeat_command.command_id, %reason, "command heartbeat failed");
            }
        }
    });
    let process = async {
        let mut prompt = prompt;
        if let Some(role) = &command.preflight {
            let account = command
                .harness_account
                .as_ref()
                .map(|account| {
                    serde_json::json!({
                        "harness_account_id": account.id,
                        "harness_account_profile_kind": account.profile_kind,
                    })
                })
                .unwrap_or_else(|| serde_json::json!({}));
            let plan = choruz_host_runtime::harness::prepare(
                choruz_host_runtime::TerminalSpec {
                    authentication: false,
                    terminal_id: command.binding_id.clone(),
                    driver_type: command.driver_type.clone(),
                    binary_path: None,
                    workspace_path: command.workspace_path.clone(),
                    cols: 120,
                    rows: 40,
                    resume_session_id: None,
                    codex_home: None,
                    model: command.model.clone(),
                    harness_account: account,
                },
                role,
                &prompt,
            )
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            prompt.push_str("\n\n");
            prompt.push_str(&plan);
        }
        let args = driver.args_with_session_fork(
            command.external_session_id.as_deref(),
            command.model.as_deref(),
            &prompt,
            command.fork_session,
        );
        process_command
            .args(args)
            .env("CHORUZ_WORKSPACE", &workspace)
            .env("CHORUZ_SEND", &connector_executable)
            .env("CHORUZ_CONNECTOR_OUTBOX", &outbox_path)
            .env("DISABLE_AUTOUPDATER", "1")
            .env("PI_SKIP_VERSION_CHECK", "1")
            .env("CLAUDE_CODE_ENABLE_TASKS", "1")
            .kill_on_drop(true)
            .output()
            .await
    };
    let outcome = tokio::time::timeout(DEFAULT_TIMEOUT, process).await;
    heartbeat_task.abort();
    let _ = heartbeat_task.await;
    let elapsed = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
    let group_reply = is_group_prompt(&command.prompt);
    let routed_messages = read_routed_messages(&outbox_path);
    // Every command other than `send` (which the completion carries) goes to
    // the binding's mirror on the controller, where the pipeline runs it as
    // it would for a local workspace. The turn's outbox is temporary, so a
    // shipment that fails is spooled under the runtime directory and
    // retried by the shipping loop.
    let shipped = commands_from_frames(&workspace, &fs::read(&outbox_path).unwrap_or_default());
    let shipment = match ship_outbox(&client, &config, &command.binding_id, shipped.clone()).await {
        Ok(()) => Ok(()),
        Err(ShipError::Refused(reason)) => {
            warn!(command_id = %command.command_id, %reason, "outbox commands were refused and dropped");
            Err(reason)
        }
        Err(ShipError::Transient(reason)) => {
            warn!(command_id = %command.command_id, %reason, "outbox commands spooled for retry");
            spool_shipped_commands(&command.binding_id, &shipped)
                .map_err(|error| format!("outbox commands could not be spooled: {error}"))
        }
    };
    let (succeeded, contents, error_message, tool_calls, session_id, clear_session) = match outcome
    {
        Ok(Ok(output)) if output.status.success() => {
            let parsed = parse_output(driver, &String::from_utf8_lossy(&output.stdout));
            let response = shipment.and_then(|()| {
                if command.fork_session
                    && (parsed.session_id.is_none()
                        || parsed.session_id == command.external_session_id)
                {
                    return Err(
                        "Claude did not isolate the routed turn from the direct session".into(),
                    );
                }
                turn_response(&parsed, group_reply, routed_messages, !shipped.is_empty())
            });
            let (succeeded, contents, error) = match response {
                Ok(contents) => (true, contents, None),
                Err(error) => (false, Vec::new(), Some(error)),
            };
            (
                succeeded,
                contents,
                error,
                parsed.tool_calls_count,
                parsed.session_id,
                false,
            )
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
            let clear_session = command.external_session_id.is_some()
                && [
                    "no saved session found",
                    "session not found",
                    "unknown session",
                    "could not find session",
                ]
                .iter()
                .any(|phrase| stderr.contains(phrase));
            (
                false,
                Vec::new(),
                Some(format!("Harness exited with status {}", output.status)),
                0,
                None,
                clear_session,
            )
        }
        Ok(Err(error)) => (
            false,
            Vec::new(),
            Some(format!("Harness could not start ({:?})", error.kind())),
            0,
            None,
            false,
        ),
        Err(_) => (
            false,
            Vec::new(),
            Some("Harness exceeded the 30 minute timeout".into()),
            0,
            None,
            false,
        ),
    };
    complete(
        &client,
        &config,
        &command,
        &CompleteCommand {
            attempt_id: &command.attempt_id,
            succeeded,
            contents: &contents,
            error: error_message.as_deref(),
            tool_calls_count: tool_calls,
            execution_duration_ms: elapsed,
            external_session_id: session_id.as_deref(),
            clear_external_session: clear_session,
        },
    )
    .await?;
    info!(command_id = %command.command_id, succeeded, error = error_message.as_deref(), elapsed_ms = elapsed, "remote Agent turn completed");
    Ok(())
}

fn turn_response(
    parsed: &ParsedOutput,
    group_reply: bool,
    routed_messages: Vec<String>,
    has_platform_commands: bool,
) -> Result<Vec<String>, String> {
    if parsed.structured_error {
        return Err("Harness reported a structured error response".into());
    }
    if group_reply {
        if routed_messages.is_empty() && !has_platform_commands {
            return Err("Harness did not route its group reply through CHORUZ_SEND".into());
        }
        // File shares and silent board mutations are valid platform replies;
        // requiring an additional chat message would replay their side effects.
        return Ok(routed_messages);
    }
    if parsed.response_text.is_empty() {
        return Err("Harness returned no Agent response".into());
    }
    Ok(vec![parsed.response_text.clone()])
}

/// Stage the files attached to the turn into the workspace inbox, fetching
/// each through the host-facing command attachment route, and return the
/// prompt with their local paths appended, as the pipeline does locally.
async fn stage_command_attachments(
    client: &Client,
    config: &ConnectorConfig,
    command: &ClaimedCommand,
    workspace: &Path,
) -> String {
    let attachments = incoming_attachments(&command.metadata);
    if attachments.is_empty() {
        return command.prompt.clone();
    }
    let fetch = |attachment: &IncomingAttachment| {
        let request = authenticated(
            client.get(endpoint(
                config,
                &format!(
                    "/v1/runtime-hosts/{}/commands/{}/attachments/{}",
                    config.host_id, command.command_id, attachment.attachment_id
                ),
            )),
            config,
        );
        async move {
            let response = request
                .send()
                .await
                .map_err(|error| format!("download attachment: {error}"))?;
            if !response.status().is_success() {
                return Err(format!("download attachment: HTTP {}", response.status()));
            }
            response
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|error| format!("read attachment: {error}"))
        }
    };
    stage_incoming_attachments(&command.prompt, workspace, &attachments, fetch).await
}

/// Why a shipment did not land in the mirror: the controller refused these
/// commands as they are (the binding moved to another device, is gone, or
/// the batch is malformed), so another attempt would fail the same way; or
/// the request did not get through, so the commands wait for the next pass.
#[derive(Debug)]
enum ShipError {
    Refused(String),
    Transient(String),
}

impl std::fmt::Display for ShipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShipError::Refused(reason) | ShipError::Transient(reason) => f.write_str(reason),
        }
    }
}

/// A client error other than an expired credential, a timeout or a rate
/// limit is the controller's verdict on the batch itself.
fn shipment_refused(status: StatusCode) -> bool {
    status.is_client_error()
        && !matches!(
            status,
            StatusCode::UNAUTHORIZED | StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
        )
}

/// Deliver outbox commands to the binding's mirror on the controller, in
/// shipments the controller's per-request limits accept. A transient error
/// leaves the caller's files in place for the next pass; a refusal tells
/// the caller to drop them.
async fn ship_outbox(
    client: &Client,
    config: &ConnectorConfig,
    binding_id: &str,
    commands: Vec<ShippedOutboxCommand>,
) -> Result<(), ShipError> {
    for shipment in shipments(commands) {
        let response = authenticated(
            client.post(endpoint(
                config,
                &format!(
                    "/v1/runtime-hosts/{}/bindings/{binding_id}/outbox",
                    config.host_id
                ),
            )),
            config,
        )
        .json(&serde_json::json!({ "commands": shipment }))
        .send()
        .await
        .map_err(|error| ShipError::Transient(format!("ship outbox: {error}")))?;
        let status = response.status();
        if shipment_refused(status) {
            return Err(ShipError::Refused(format!(
                "ship outbox returned HTTP {status}"
            )));
        }
        if !status.is_success() {
            return Err(ShipError::Transient(format!(
                "ship outbox returned HTTP {status}"
            )));
        }
    }
    Ok(())
}

/// Every two seconds, the cadence the pipeline's watcher uses for local
/// terminals: ship the workspace outbox of every live terminal on this
/// device (the agent's only voice for platform commands there) and retry
/// whatever an earlier shipment left in the spool.
async fn ship_pending_outboxes(
    client: Client,
    config: Arc<ConnectorConfig>,
    terminals: TerminalPool,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        interval.tick().await;
        let workspaces = terminals
            .lock()
            .expect("terminal pool lock")
            .iter()
            .filter(|(_, session)| !session.authentication && session.is_child_alive())
            .map(|(id, session)| (id.clone(), session.workspace_path.clone()))
            .collect::<Vec<_>>();
        for (binding_id, workspace) in workspaces {
            let collected = collect_outbox_commands(&workspace);
            if collected.is_empty() {
                continue;
            }
            match ship_outbox(&client, &config, &binding_id, collected.commands.clone()).await {
                Ok(()) => collected.delivered(),
                Err(ShipError::Refused(reason)) => {
                    warn!(%binding_id, %reason, "terminal outbox commands were refused and dropped");
                    collected.delivered();
                }
                Err(ShipError::Transient(reason)) => {
                    warn!(%binding_id, %reason, "terminal outbox commands were not delivered")
                }
            }
        }
        for binding_id in spooled_binding_ids() {
            let spooled = take_spooled_commands(&binding_id);
            if spooled.is_empty() {
                continue;
            }
            match ship_outbox(&client, &config, &binding_id, spooled.commands.clone()).await {
                Ok(()) => spooled.delivered(),
                Err(ShipError::Refused(reason)) => {
                    warn!(%binding_id, %reason, "spooled outbox commands were refused and dropped");
                    spooled.delivered();
                }
                Err(ShipError::Transient(reason)) => {
                    warn!(%binding_id, %reason, "spooled outbox commands were not delivered")
                }
            }
        }
    }
}

/// Keep one host link open to the controller for as long as the connector
/// runs: direct WebSocket when the API is reachable, a relay stream when the
/// controller is only reachable through the Cloud Gateway.
async fn maintain_host_link(
    config: Arc<ConnectorConfig>,
    relay: Option<RelayHttpClient>,
    terminals: TerminalPool,
) {
    let mut backoff = Duration::from_secs(2);
    loop {
        let identity = DeviceIdentity {
            host_id: config.host_id.clone(),
            host_token: config.host_token.clone(),
        };
        let outcome = match relay.as_ref() {
            Some(relay) => match relay.open_stream(LINK_PATH).await {
                Ok(stream) => serve_relayed_link(stream, identity, terminals.clone()).await,
                Err(error) => Err(error),
            },
            None => serve_direct_link(&config.api_url, identity, terminals.clone()).await,
        };
        match outcome {
            Ok(()) => {
                backoff = Duration::from_secs(2);
                info!("host link closed; reconnecting");
            }
            Err(reason) => {
                warn!(%reason, retry_seconds = backoff.as_secs(), "host link failed");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

async fn serve_relayed_link(
    stream: RelayStream,
    identity: DeviceIdentity,
    terminals: TerminalPool,
) -> Result<(), String> {
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<String>(256);
    let (inbound_tx, inbound_rx) = mpsc::channel::<String>(256);
    let RelayStream {
        outbound,
        mut inbound,
    } = stream;
    let pump_out = tokio::spawn(async move {
        while let Some(text) = outbound_rx.recv().await {
            if outbound.send(StreamMessage::Text(text)).await.is_err() {
                break;
            }
        }
    });
    let pump_in = tokio::spawn(async move {
        while let Some(message) = inbound.recv().await {
            if let StreamMessage::Text(text) = message
                && inbound_tx.send(text).await.is_err()
            {
                break;
            }
        }
    });
    let result = run_device_link(outbound_tx, inbound_rx, identity, terminals).await;
    pump_out.abort();
    pump_in.abort();
    result
}

async fn serve_direct_link(
    api_url: &str,
    identity: DeviceIdentity,
    terminals: TerminalPool,
) -> Result<(), String> {
    let url = format!(
        "{}{LINK_PATH}",
        api_url
            .trim_end_matches('/')
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
    );
    let (socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|error| format!("connect host link: {error}"))?;
    let (mut sink, mut source) = socket.split();
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<String>(256);
    let (inbound_tx, inbound_rx) = mpsc::channel::<String>(256);
    let pump_out = tokio::spawn(async move {
        while let Some(text) = outbound_rx.recv().await {
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    let pump_in = tokio::spawn(async move {
        while let Some(Ok(message)) = source.next().await {
            match message {
                WsMessage::Text(text) => {
                    if inbound_tx.send(text.to_string()).await.is_err() {
                        break;
                    }
                }
                WsMessage::Close(_) => break,
                _ => {}
            }
        }
    });
    let result = run_device_link(outbound_tx, inbound_rx, identity, terminals).await;
    pump_out.abort();
    pump_in.abort();
    result
}

fn append_outbox_command(args: &[String]) -> Option<Result<(), String>> {
    let path = env::var_os("CHORUZ_CONNECTOR_OUTBOX")?;
    Some((|| {
        if args.len() != 1 {
            return Err("CHORUZ_SEND expects one JSON command".into());
        }
        let value: serde_json::Value =
            serde_json::from_str(&args[0]).map_err(|_| "CHORUZ_SEND received invalid JSON")?;
        if !value.is_object() {
            return Err("CHORUZ_SEND command must be an object".into());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(PathBuf::from(path))
            .map_err(|error| format!("open connector outbox: {error}"))?;
        let mut frame = Vec::with_capacity(args[0].len() + 1);
        frame.extend_from_slice(args[0].as_bytes());
        frame.push(0);
        file.write_all(&frame)
            .map_err(|error| format!("write connector outbox: {error}"))
    })())
}

fn read_routed_messages(path: &Path) -> Vec<String> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    bytes
        .split(|byte| *byte == 0)
        .filter_map(|frame| serde_json::from_slice::<serde_json::Value>(frame).ok())
        .filter(|command| command.get("type").and_then(serde_json::Value::as_str) == Some("send"))
        .filter_map(|command| {
            command
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|content| !content.is_empty())
                .map(str::to_owned)
        })
        .collect()
}

fn is_group_prompt(prompt: &str) -> bool {
    prompt
        .split_once('|')
        .map(|(metadata, _)| {
            metadata.trim_start().starts_with("[choruz-incoming]")
                && metadata
                    .split_ascii_whitespace()
                    .any(|field| field.starts_with("group:"))
        })
        .unwrap_or(false)
}

async fn run(args: &[String]) -> Result<(), String> {
    let path = config_path(args)?;
    let lock_path = path.with_extension("lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("open connector lock: {error}"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| format!("connector for {} is already running", path.display()))?;
    let mut config = load_config(&path)?;
    if env::var_os("CHORUZ_RUNTIME_DIR").is_none()
        && let Some(home) = env::var_os("HOME")
    {
        // Managed Codex homes live under the user's Choruz directory rather
        // than whichever directory the connector happened to start in.
        unsafe {
            env::set_var(
                "CHORUZ_RUNTIME_DIR",
                PathBuf::from(home).join(".choruz").join("runtime"),
            );
        }
    }
    let mut relay_client = None;
    let _relay_proxy = if let Some(credentials) = config.relay.clone() {
        let (api_url, task, client) = relay_proxy(credentials).await?;
        config.api_url = api_url;
        relay_client = Some(client);
        Some(task)
    } else {
        None
    };
    let config = Arc::new(config);
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(35))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| error.to_string())?;
    wait_for_controller(&client, &config).await?;
    info!(host = %config.host_name, concurrency = config.max_concurrency, "Choruz Connector online");

    let heartbeat_client = client.clone();
    let heartbeat_config = Arc::clone(&config);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(HOST_HEARTBEAT);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(reason) = heartbeat(&heartbeat_client, &heartbeat_config).await {
                warn!(%reason, "host heartbeat failed");
            }
        }
    });

    let semaphore = Arc::new(Semaphore::new(config.max_concurrency));
    let mut jobs = JoinSet::new();
    let login_client = client.clone();
    let login_config = Arc::clone(&config);
    tokio::spawn(async move {
        let mut login_jobs = JoinSet::new();
        loop {
            while let Some(result) = login_jobs.try_join_next() {
                if let Err(error) = result {
                    warn!(%error, "remote Harness login task crashed");
                }
            }
            match claim_harness_login(&login_client, &login_config).await {
                Ok(Some(login)) => {
                    let job_client = login_client.clone();
                    let job_config = Arc::clone(&login_config);
                    login_jobs.spawn(async move {
                        execute_harness_login(job_client, job_config, login).await;
                    });
                }
                Ok(None) => {}
                Err(reason) => warn!(%reason, "remote Harness login claim failed"),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    let terminals = new_terminal_pool();
    let link_config = Arc::clone(&config);
    let link_terminals = terminals.clone();
    tokio::spawn(async move {
        maintain_host_link(link_config, relay_client, link_terminals).await;
    });
    let shipper_client = client.clone();
    let shipper_config = Arc::clone(&config);
    tokio::spawn(async move {
        ship_pending_outboxes(shipper_client, shipper_config, terminals).await;
    });
    let mut failures = 0u32;
    loop {
        while let Some(result) = jobs.try_join_next() {
            if let Err(reason) = result.unwrap_or_else(|error| Err(error.to_string())) {
                warn!(%reason, "remote Agent turn failed");
            }
        }
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .map_err(|_| "connector is shutting down")?;
        match claim(&client, &config).await {
            Ok(Some(command)) => {
                failures = 0;
                let job_client = client.clone();
                let job_config = Arc::clone(&config);
                jobs.spawn(async move {
                    let _permit = permit;
                    execute(job_client, job_config, command).await
                });
            }
            Ok(None) => {
                failures = 0;
                drop(permit);
            }
            Err(reason) => {
                drop(permit);
                if reason.contains("401 Unauthorized") || reason.contains("403 Forbidden") {
                    return Err("runtime host authorization was revoked".into());
                }
                failures = failures.saturating_add(1);
                let delay = Duration::from_secs(2u64.saturating_pow(failures.min(5)));
                warn!(%reason, retry_seconds = delay.as_secs(), "command stream interrupted");
                tokio::time::sleep(delay).await;
            }
        }
    }
}

fn usage() {
    eprintln!(
        "Usage:\n  choruz-connector pair --api-url <url> --code <8 digits> --name <machine> [--config <path>] [--max-concurrency <n>]\n  choruz-connector pair-relay --config <path> [--max-concurrency <n>] < request.json\n  choruz-connector run [--config <path>]"
    );
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Some(result) = append_outbox_command(&args) {
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(reason) => {
                eprintln!("choruz-connector send: {reason}");
                ExitCode::FAILURE
            }
        };
    }
    if let Err(error) = choruz_infrastructure::init_tracing("choruz-connector") {
        eprintln!("invalid logging configuration: {error}");
        return ExitCode::from(2);
    }
    let result = match args.first().map(String::as_str) {
        Some("pair") => pair(&args[1..]).await,
        Some("pair-relay") => pair_relay(&args[1..]).await,
        Some("run") => {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("register SIGTERM");
            tokio::select! {
                result = run(&args[1..]) => result,
                _ = tokio::signal::ctrl_c() => Ok(()),
                _ = sigterm.recv() => Ok(()),
            }
        }
        _ => {
            usage();
            return ExitCode::from(2);
        }
    };
    let _ = tokio::task::spawn_blocking(choruz_host_runtime::session::shutdown).await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            error!(%reason, "connector failed");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn file_only_group_turn_completes_without_replaying_the_harness() {
        let parsed = ParsedOutput::default();
        assert_eq!(
            turn_response(&parsed, true, Vec::new(), true),
            Ok(Vec::new())
        );
        assert!(turn_response(&parsed, true, Vec::new(), false).is_err());
        assert!(turn_response(&parsed, false, Vec::new(), true).is_err());
        let failed = ParsedOutput {
            structured_error: true,
            ..parsed
        };
        assert!(turn_response(&failed, true, vec!["sent".into()], true).is_err());
    }

    #[test]
    fn a_refused_shipment_is_dropped_and_a_failed_one_retried() {
        assert!(shipment_refused(StatusCode::FORBIDDEN), "binding moved");
        assert!(shipment_refused(StatusCode::NOT_FOUND), "binding gone");
        assert!(shipment_refused(StatusCode::BAD_REQUEST), "batch malformed");
        assert!(shipment_refused(StatusCode::PAYLOAD_TOO_LARGE));
        assert!(!shipment_refused(StatusCode::UNAUTHORIZED));
        assert!(!shipment_refused(StatusCode::TOO_MANY_REQUESTS));
        assert!(!shipment_refused(StatusCode::REQUEST_TIMEOUT));
        assert!(!shipment_refused(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!shipment_refused(StatusCode::BAD_GATEWAY));
    }

    #[test]
    fn config_round_trip_is_private() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("connector.json");
        let config = ConnectorConfig {
            api_url: "https://choruz.example".into(),
            relay: None,
            host_id: "host-1".into(),
            host_name: "Builder".into(),
            host_token: "secret".into(),
            max_concurrency: 4,
        };
        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.host_id, "host-1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn routed_messages_preserve_each_helper_call() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("commands");
        fs::write(
            &path,
            b"{\"type\":\"send\",\"group\":\"team\",\"content\":\"first\"}\0{\"type\":\"send\",\"group\":\"team\",\"content\":\"second\"}\0",
        )
        .unwrap();
        assert_eq!(read_routed_messages(&path), ["first", "second"]);
    }

    #[test]
    fn group_prompt_detection_parses_protocol_metadata_only() {
        assert!(is_group_prompt(
            "[choruz-incoming] from:@operator group:proj-team conv:123 roster:[] | hello"
        ));
        assert!(!is_group_prompt(
            "[choruz-incoming] from:@operator direct-chat conv:123 | hello group:later"
        ));
        assert!(!is_group_prompt("please post to group:proj-team | hello"));
    }

    #[test]
    fn relay_proxy_maps_connector_api_routes_to_remote_dashboard_routes() {
        assert_eq!(
            relay_api_path("/v1/runtime-hosts/host-1/heartbeat"),
            "/api/v1/runtime-hosts/host-1/heartbeat"
        );
        assert_eq!(
            relay_api_path("/api/filesystem?action=home"),
            "/api/filesystem?action=home"
        );
    }

    #[tokio::test]
    async fn remote_filesystem_listing_stays_inside_the_configured_root() {
        let home = env::var_os("HOME").expect("test HOME");
        let root = tempfile::tempdir_in(home).unwrap();
        tokio::fs::create_dir(root.path().join("project"))
            .await
            .unwrap();
        tokio::fs::write(root.path().join("ignored.txt"), b"file")
            .await
            .unwrap();

        let listing =
            choruz_host_runtime::execute(choruz_host_runtime::HostRequest::FilesystemList {
                path: root.path().to_string_lossy().into_owned(),
                show_hidden: false,
                include_files: false,
            })
            .await
            .unwrap();
        assert_eq!(listing["entries"].as_array().unwrap().len(), 1);
        assert_eq!(listing["entries"][0]["name"], "project");

        let outside =
            choruz_host_runtime::execute(choruz_host_runtime::HostRequest::FilesystemList {
                path: "/".into(),
                show_hidden: false,
                include_files: false,
            })
            .await;
        assert!(outside.is_err(), "the filesystem root is outside HOME");
    }

    #[test]
    fn connector_opencode_args_pin_the_claimed_binding_workspace() {
        let args = HeadlessDriver::OpenCode.args(
            Some("session-1"),
            Some("opencode/mimo-v2.5-free"),
            "hello",
        );
        assert_eq!(
            args.windows(2).find(|pair| pair[0] == "--dir"),
            Some(["--dir".to_owned(), ".".to_owned()].as_slice())
        );
        assert_eq!(args.last().map(String::as_str), Some("hello"));
    }

    #[tokio::test]
    async fn startup_survives_a_transient_heartbeat_failure() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for status in ["502 Bad Gateway", "204 No Content"] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let length = stream.read(&mut request).await.unwrap();
                assert!(
                    String::from_utf8_lossy(&request[..length])
                        .starts_with("POST /v1/runtime-hosts/host-1/heartbeat")
                );
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let config = ConnectorConfig {
            api_url: format!("http://{address}"),
            relay: None,
            host_id: "host-1".into(),
            host_name: "Test".into(),
            host_token: "test".into(),
            max_concurrency: 1,
        };
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_controller(&Client::new(), &config),
        )
        .await;
        server.abort();
        let joined = server.await;
        assert!(outcome.unwrap().is_ok());
        assert!(joined.is_ok());
    }

    #[tokio::test]
    async fn heartbeat_uses_the_runtime_host_post_contract() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let size = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.starts_with("POST /v1/runtime-hosts/host-1/heartbeat HTTP/1.1"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let config = ConnectorConfig {
            api_url: format!("http://{address}"),
            relay: None,
            host_id: "host-1".into(),
            host_name: "Builder".into(),
            host_token: "secret".into(),
            max_concurrency: 1,
        };
        heartbeat(&Client::new(), &config).await.unwrap();
        server.await.unwrap();
    }
}
