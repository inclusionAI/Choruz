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
    HeadlessDriver, configure_command_workspace, harness_account_env, parse_output, validate_model,
};
use choruz_harness_login::{AccountProbe, CodexLoginLocation, LoginJob, LoginSink, run_login};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::{process::Command, sync::Semaphore, task::JoinSet};
use tracing::{error, info, warn};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const HOST_HEARTBEAT: Duration = Duration::from_secs(15);
const COMMAND_HEARTBEAT: Duration = Duration::from_secs(10);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConnectorConfig {
    api_url: String,
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

#[derive(Debug, Clone, Deserialize)]
struct ClaimedCommand {
    command_id: String,
    attempt_id: String,
    agent_id: String,
    conversation_id: String,
    turn_id: String,
    prompt: String,
    driver_type: String,
    workspace_path: String,
    model: Option<String>,
    external_session_id: Option<String>,
    harness_account: Option<ClaimedHarnessAccount>,
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
    if config.api_url.trim().is_empty()
        || config.host_id.trim().is_empty()
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

fn authenticated(
    request: reqwest::RequestBuilder,
    config: &ConnectorConfig,
) -> reqwest::RequestBuilder {
    request.header("x-choruz-host-token", &config.host_token)
}

async fn heartbeat(client: &Client, config: &ConnectorConfig) -> Result<(), String> {
    let response = authenticated(
        client.post(endpoint(
            config,
            &format!("/v1/runtime-hosts/{}/heartbeat", config.host_id),
        )),
        config,
    )
    .send()
    .await
    .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!(
            "host heartbeat returned HTTP {}",
            response.status()
        ))
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
                codex_login_location: CodexLoginLocation::Remote,
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
    let args = driver.args(
        command.external_session_id.as_deref(),
        command.model.as_deref(),
        &command.prompt,
    );
    let outbox_dir = tempfile::Builder::new()
        .prefix("choruz-connector-outbox-")
        .tempdir()
        .map_err(|error| format!("create turn outbox: {error}"))?;
    let outbox_path = outbox_dir.path().join("commands");
    let connector_executable =
        env::current_exe().map_err(|error| format!("locate connector helper: {error}"))?;
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
    let mut process_command = Command::new(driver.default_binary());
    configure_command_workspace(&mut process_command, driver, &workspace);
    let account_env = if let Some(account) = command.harness_account.as_ref()
        && account.profile_kind == "isolated"
    {
        let account_config = serde_json::json!({
            "harness_account_id": account.id,
            "harness_account_profile_kind": account.profile_kind,
        });
        match harness_account_env(driver, &account_config) {
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
    let process = process_command
        .args(args)
        .env("CHORUZ_WORKSPACE", &workspace)
        .env("CHORUZ_SEND", &connector_executable)
        .env("CHORUZ_CONNECTOR_OUTBOX", &outbox_path)
        .env("DISABLE_AUTOUPDATER", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("CLAUDE_CODE_ENABLE_TASKS", "1")
        .kill_on_drop(true)
        .output();
    let outcome = tokio::time::timeout(DEFAULT_TIMEOUT, process).await;
    heartbeat_task.abort();
    let elapsed = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
    let group_reply = is_group_prompt(&command.prompt);
    let routed_messages = read_routed_messages(&outbox_path);
    let (succeeded, contents, error_message, tool_calls, session_id, clear_session) = match outcome
    {
        Ok(Ok(output)) if output.status.success() => {
            let parsed = parse_output(driver, &String::from_utf8_lossy(&output.stdout));
            if parsed.structured_error {
                (
                    false,
                    Vec::new(),
                    Some("Harness reported a structured error response".to_owned()),
                    parsed.tool_calls_count,
                    parsed.session_id,
                    false,
                )
            } else {
                let response_text = if group_reply {
                    routed_messages.join("\n\n")
                } else {
                    parsed.response_text
                };
                if response_text.is_empty() {
                    (
                        false,
                        Vec::new(),
                        Some(if group_reply {
                            "Harness did not route its group reply through CHORUZ_SEND".to_owned()
                        } else {
                            "Harness returned no Agent response".to_owned()
                        }),
                        0,
                        parsed.session_id,
                        false,
                    )
                } else {
                    (
                        true,
                        if group_reply {
                            routed_messages
                        } else {
                            vec![response_text]
                        },
                        None,
                        parsed.tool_calls_count,
                        parsed.session_id,
                        false,
                    )
                }
            }
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
    info!(command_id = %command.command_id, succeeded, elapsed_ms = elapsed, "remote Agent turn completed");
    Ok(())
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
    let config = Arc::new(load_config(&path)?);
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(35))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| error.to_string())?;
    heartbeat(&client, &config).await?;
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
        "Usage:\n  choruz-connector pair --api-url <url> --code <8 digits> --name <machine> [--config <path>] [--max-concurrency <n>]\n  choruz-connector run [--config <path>]"
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
        Some("run") => run(&args[1..]).await,
        _ => {
            usage();
            return ExitCode::from(2);
        }
    };
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
    fn config_round_trip_is_private() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("connector.json");
        let config = ConnectorConfig {
            api_url: "https://choruz.example".into(),
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
            host_id: "host-1".into(),
            host_name: "Builder".into(),
            host_token: "secret".into(),
            max_concurrency: 1,
        };
        heartbeat(&Client::new(), &config).await.unwrap();
        server.await.unwrap();
    }
}
