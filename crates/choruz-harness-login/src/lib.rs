//! Drive a Harness's own browser sign-in and hand the pieces a user needs to
//! a [`LoginSink`]: the authorization link, any callback the user must paste,
//! and the verified account snapshot.
//!
//! The same driver runs inside the API gateway for accounts on the gateway's
//! own device and inside `choruz-connector` for accounts on a remote runtime
//! host; only the sink differs. Credentials never pass through the sink: the
//! Harness process writes them into the account's profile directory.

use std::{fs, time::Duration};

use choruz_agent_runtime::headless::{HeadlessDriver, harness_account_env};
use sha2::Digest;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
};

/// How long a sign-in may wait for the browser before the driver gives up.
pub const DEFAULT_LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const POST_AUTH_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the browser opening a Codex authorization URL is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexLoginLocation {
    Local,
    /// The browser runs on the controlling device. Choruz forwards its pasted
    /// localhost callback to the Codex app-server on the remote device.
    Remote,
}

/// One queued sign-in for one harness account.
#[derive(Debug, Clone)]
pub struct LoginJob {
    pub login_id: String,
    pub account_id: String,
    pub driver: HeadlessDriver,
    /// `default` reuses the device's own profile; `isolated` selects the
    /// account's private profile directory under the harness account root.
    pub profile_kind: String,
    pub codex_login_location: CodexLoginLocation,
}

/// The verified identity of a signed-in account plus any available model and
/// quota snapshot. Authentication may complete before those catalogs are
/// available, in which case `models` and `usage.windows` are empty.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountProbe {
    pub fingerprint: String,
    pub subscription_type: Option<String>,
    pub models: serde_json::Value,
    pub usage: serde_json::Value,
}

/// Result of a completed browser login and its best-effort account refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOutcome {
    /// A non-fatal diagnostic from the post-authentication catalog refresh.
    pub snapshot_error: Option<String>,
}

/// Where the driver reports progress. Implementations update the
/// `harness_account_login` row directly (gateway) or through the runtime-host
/// API (connector).
pub trait LoginSink: Send + Sync {
    /// The Harness produced its authorization link. `user_code` remains in
    /// the sink contract so an already-running legacy login can finish.
    fn publish(
        &self,
        authorization_url: &str,
        user_code: Option<&str>,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;
    /// The code or callback URL the user pasted, once available. Polled by a
    /// flow that requires manual callback handoff until the login times out.
    fn take_callback(
        &self,
    ) -> impl std::future::Future<Output = Result<Option<String>, String>> + Send;
    /// Mark the OAuth flow verified as soon as the Harness reports the signed-in
    /// identity. Model and quota catalogs are deliberately not prerequisites.
    fn complete_authentication(
        &self,
        probe: &AccountProbe,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;
    /// Store a complete model and quota snapshot after authentication. A
    /// failure here must not reverse the completed login.
    fn publish_snapshot(
        &self,
        probe: &AccountProbe,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;
}

/// The executable that runs the sign-in: the `CHORUZ_<HARNESS>_BINARY`
/// override, else the Harness's default command name.
pub fn login_binary(driver: HeadlessDriver) -> String {
    let variable = match driver {
        HeadlessDriver::Claude => "CHORUZ_CLAUDE_BINARY",
        HeadlessDriver::Codex => "CHORUZ_CODEX_BINARY",
        HeadlessDriver::Pi => "CHORUZ_PI_BINARY",
        HeadlessDriver::Grok => "CHORUZ_GROK_BINARY",
        HeadlessDriver::OpenCode => "CHORUZ_OPENCODE_BINARY",
        HeadlessDriver::MathCode => "CHORUZ_MATHCODE_BINARY",
    };
    std::env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| driver.default_binary().to_owned())
}

/// Run the sign-in for `job` to completion. Authentication succeeds once the
/// Harness reports a signed-in identity; catalog refresh failures are returned
/// separately and never reverse that success.
pub async fn run_login<S: LoginSink>(
    job: &LoginJob,
    sink: &S,
    timeout: Duration,
) -> Result<LoginOutcome, String> {
    match job.driver {
        HeadlessDriver::Codex => codex_login(job, sink, timeout).await,
        HeadlessDriver::Claude => claude_login(job, sink, timeout).await,
        _ => Err("Browser sign-in is unsupported for this Harness".into()),
    }
}

fn apply_account_profile(command: &mut Command, job: &LoginJob) -> Result<(), String> {
    if job.profile_kind == "isolated" {
        let profile = harness_account_env(
            job.driver,
            &serde_json::json!({
                "harness_account_id": job.account_id,
                "harness_account_profile_kind": job.profile_kind,
            }),
        )?
        .ok_or("isolated account did not resolve a profile directory")?;
        fs::create_dir_all(&profile.1)
            .map_err(|error| format!("create isolated Harness profile: {error}"))?;
        command.env(profile.0, profile.1);
    }
    Ok(())
}

async fn write_json_line(stdin: &mut ChildStdin, value: &serde_json::Value) -> Result<(), String> {
    let mut encoded = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    encoded.push(b'\n');
    stdin
        .write_all(&encoded)
        .await
        .map_err(|error| format!("write Harness login request: {error}"))
}

type Lines = tokio::io::Lines<BufReader<ChildStdout>>;

async fn next_json_line(reader: &mut Lines) -> Result<serde_json::Value, String> {
    loop {
        let line = reader
            .next_line()
            .await
            .map_err(|error| format!("read Harness login response: {error}"))?
            .ok_or("Harness login process closed before completing authentication")?;
        if let Ok(value) = serde_json::from_str(&line) {
            return Ok(value);
        }
    }
}

async fn wait_for_rpc(reader: &mut Lines, id: i64) -> Result<serde_json::Value, String> {
    loop {
        let response = next_json_line(reader).await?;
        if response.get("id").and_then(serde_json::Value::as_i64) == Some(id) {
            if let Some(error) = response.get("error") {
                return Err(format!("Harness login request failed: {error}"));
            }
            return response
                .get("result")
                .cloned()
                .ok_or("Harness login response was missing result".into());
        }
    }
}

async fn codex_login<S: LoginSink>(
    job: &LoginJob,
    sink: &S,
    timeout: Duration,
) -> Result<LoginOutcome, String> {
    let mut command = Command::new(login_binary(HeadlessDriver::Codex));
    command
        .arg("app-server")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    apply_account_profile(&mut command, job)?;
    let mut child = command
        .spawn()
        .map_err(|error| format!("start Codex app server: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("Codex app server stdin unavailable")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Codex app server stdout unavailable")?;
    let mut reader = BufReader::new(stdout).lines();
    write_json_line(&mut stdin, &serde_json::json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"clientInfo":{"name":"choruz","title":"Choruz","version":"1"},"capabilities":{"experimentalApi":true}}})).await?;
    wait_for_rpc(&mut reader, 0).await.map_err(|_| {
        "Codex app-server is unavailable; update Codex or set CHORUZ_CODEX_BINARY to a current Codex CLI"
            .to_owned()
    })?;
    write_json_line(
        &mut stdin,
        &serde_json::json!({"jsonrpc":"2.0","method":"initialized"}),
    )
    .await?;
    write_json_line(&mut stdin, &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"account/login/start","params":{"type":"chatgpt","useHostedLoginSuccessPage":true,"appBrand":"codex"}})).await?;
    let start = wait_for_rpc(&mut reader, 1).await?;
    let url = start
        .get("authUrl")
        .and_then(serde_json::Value::as_str)
        .ok_or("Codex did not return an authorization URL")?;
    let login_id = start
        .get("loginId")
        .and_then(serde_json::Value::as_str)
        .ok_or("Codex did not return a login id")?;
    sink.publish(url, None).await?;

    tokio::time::timeout(timeout, async {
        if job.codex_login_location == CodexLoginLocation::Remote {
            wait_for_remote_codex_login(&mut reader, sink, url, login_id).await?;
        } else {
            wait_for_codex_login(&mut reader, login_id).await?;
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "Codex login timed out".to_owned())??;

    write_json_line(&mut stdin, &serde_json::json!({"jsonrpc":"2.0","id":10,"method":"account/read","params":{"refreshToken":true}})).await?;
    let account = tokio::time::timeout(POST_AUTH_SNAPSHOT_TIMEOUT, wait_for_rpc(&mut reader, 10))
        .await
        .map_err(|_| "Codex account identity was not ready after login".to_owned())??;
    let identity = codex_identity_probe(&account)?;
    sink.complete_authentication(&identity).await?;

    let snapshot = tokio::time::timeout(POST_AUTH_SNAPSHOT_TIMEOUT, async {
        write_json_line(&mut stdin, &serde_json::json!({"jsonrpc":"2.0","id":11,"method":"account/rateLimits/read"})).await?;
        let limits = wait_for_rpc(&mut reader, 11).await?;
        write_json_line(&mut stdin, &serde_json::json!({"jsonrpc":"2.0","id":12,"method":"model/list","params":{"limit":100,"includeHidden":false}})).await?;
        let models = wait_for_rpc(&mut reader, 12).await?;
        let probe = codex_account_probe(&account, &limits, &models)?;
        sink.publish_snapshot(&probe).await
    })
    .await;
    let snapshot_error = match snapshot {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(_) => Some("Codex account model and usage snapshot timed out".to_owned()),
    };
    Ok(LoginOutcome { snapshot_error })
}

async fn wait_for_remote_codex_login<S: LoginSink>(
    reader: &mut Lines,
    sink: &S,
    authorization_url: &str,
    login_id: &str,
) -> Result<(), String> {
    let mut callback_poll = tokio::time::interval(Duration::from_secs(1));
    let mut callback_forwarded = false;
    loop {
        tokio::select! {
            message = next_json_line(reader) => {
                if let Some(result) = codex_login_completion(&message?, login_id) {
                    return result;
                }
            }
            _ = callback_poll.tick(), if !callback_forwarded => {
                if let Some(callback) = sink.take_callback().await? {
                    forward_codex_callback(authorization_url, &callback).await?;
                    callback_forwarded = true;
                }
            }
        }
    }
}

async fn wait_for_codex_login(reader: &mut Lines, login_id: &str) -> Result<(), String> {
    loop {
        let message = next_json_line(reader).await?;
        if let Some(result) = codex_login_completion(&message, login_id) {
            return result;
        }
    }
}

fn codex_login_completion(
    message: &serde_json::Value,
    login_id: &str,
) -> Option<Result<(), String>> {
    if message.get("method").and_then(serde_json::Value::as_str) != Some("account/login/completed")
    {
        return None;
    }
    let Some(params) = message.get("params") else {
        return Some(Err("Codex login completion was missing parameters".into()));
    };
    if params.get("loginId").and_then(serde_json::Value::as_str) != Some(login_id) {
        return None;
    }
    Some(
        match params.get("success").and_then(serde_json::Value::as_bool) {
            Some(true) => Ok(()),
            Some(false) => Err("Codex rejected the login request".into()),
            None => Err("Codex login completion was invalid".into()),
        },
    )
}

async fn forward_codex_callback(authorization_url: &str, callback: &str) -> Result<(), String> {
    let authorization = url::Url::parse(authorization_url)
        .map_err(|_| "Codex returned an invalid authorization URL")?;
    let redirect = authorization
        .query_pairs()
        .find(|(key, _)| key == "redirect_uri")
        .map(|(_, value)| value.into_owned())
        .ok_or("Codex authorization callback is unavailable")?;
    let mut redirect =
        url::Url::parse(&redirect).map_err(|_| "Codex returned an invalid callback URL")?;
    let local_host = matches!(redirect.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if redirect.scheme() != "http" || !local_host {
        return Err("Codex returned an unsafe callback URL".into());
    }
    let callback = url::Url::parse(callback.trim())
        .map_err(|_| "Paste the complete Codex localhost callback URL".to_owned())?;
    let values = callback
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    let code = values
        .get("code")
        .filter(|value| !value.is_empty())
        .ok_or("The Codex callback is missing its authorization code")?;
    let state = values
        .get("state")
        .filter(|value| !value.is_empty())
        .ok_or("The Codex callback is missing its state")?;
    let expected_state = authorization
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or("Codex authorization state is unavailable")?;
    if state.as_ref() != expected_state {
        return Err("The callback does not belong to this Codex login".into());
    }
    redirect.set_query(None);
    redirect
        .query_pairs_mut()
        .append_pair("code", code)
        .append_pair("state", state);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "Prepare Codex callback forwarding failed")?;
    let response = client
        .get(redirect)
        .send()
        .await
        .map_err(|_| "Forward the Codex callback to the remote device failed")?;
    if response.status().is_success() || response.status().is_redirection() {
        Ok(())
    } else {
        Err("The remote Codex login rejected its browser callback".into())
    }
}

async fn wait_for_control(
    reader: &mut Lines,
    request_id: &str,
) -> Result<serde_json::Value, String> {
    loop {
        let message = next_json_line(reader).await?;
        let response = message.get("response").unwrap_or(&message);
        if response
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            == Some(request_id)
        {
            if response.get("subtype").and_then(serde_json::Value::as_str) == Some("error") {
                return Err(claude_control_error(response));
            }
            return Ok(response
                .get("response")
                .cloned()
                .unwrap_or_else(|| response.clone()));
        }
    }
}

fn claude_control_error(response: &serde_json::Value) -> String {
    let detail = response
        .get("error")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|detail| {
            !detail.is_empty() && detail.len() <= 512 && !detail.chars().any(char::is_control)
        });
    match detail {
        Some(detail) => format!("Claude login request failed: {detail}"),
        None => "Claude login request failed".into(),
    }
}

fn control_request(id: &str, request: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"type":"control_request","request_id":id,"request":request})
}

async fn claude_login<S: LoginSink>(
    job: &LoginJob,
    sink: &S,
    timeout: Duration,
) -> Result<LoginOutcome, String> {
    let mut command = Command::new(login_binary(HeadlessDriver::Claude));
    command
        .args([
            "--print",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--verbose",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    apply_account_profile(&mut command, job)?;
    let mut child = command
        .spawn()
        .map_err(|error| format!("start Claude Code control session: {error}"))?;
    let mut stdin = child.stdin.take().ok_or("Claude Code stdin unavailable")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Claude Code stdout unavailable")?;
    let mut reader = BufReader::new(stdout).lines();
    write_json_line(
        &mut stdin,
        &control_request("initialize-1", serde_json::json!({"subtype":"initialize"})),
    )
    .await?;
    let _ = wait_for_control(&mut reader, "initialize-1").await?;
    write_json_line(
        &mut stdin,
        &control_request(
            "authenticate-1",
            serde_json::json!({"subtype":"claude_authenticate","loginWithClaudeAi":true}),
        ),
    )
    .await?;
    let auth = wait_for_control(&mut reader, "authenticate-1").await?;
    let url = auth
        .get("manualUrl")
        .and_then(serde_json::Value::as_str)
        .ok_or("Claude did not return a manual authorization URL")?;
    let state = url::Url::parse(url)
        .map_err(|_| "Claude returned an invalid authorization URL")?
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.to_string())
        .ok_or("Claude authorization state is unavailable")?;
    sink.publish(url, None).await?;
    let callback = tokio::time::timeout(timeout, async {
        loop {
            if let Some(value) = sink.take_callback().await? {
                return Ok::<_, String>(value);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    .map_err(|_| "Claude authorization timed out".to_owned())??;
    let code = callback_code_and_state(&callback, &state)?;
    write_json_line(&mut stdin, &control_request("callback-1", serde_json::json!({"subtype":"claude_oauth_callback","authorizationCode":code,"state":state}))).await?;
    let _ = wait_for_control(&mut reader, "callback-1").await?;
    write_json_line(
        &mut stdin,
        &control_request(
            "initialize-after-login",
            serde_json::json!({"subtype":"initialize"}),
        ),
    )
    .await?;
    let initialization = tokio::time::timeout(
        POST_AUTH_SNAPSHOT_TIMEOUT,
        wait_for_control(&mut reader, "initialize-after-login"),
    )
    .await
    .map_err(|_| "Claude account identity was not ready after login".to_owned())??;
    let identity = claude_identity_probe(&initialization)?;
    sink.complete_authentication(&identity).await?;

    let snapshot = tokio::time::timeout(POST_AUTH_SNAPSHOT_TIMEOUT, async {
        write_json_line(
            &mut stdin,
            &control_request(
                "usage-after-login",
                serde_json::json!({"subtype":"get_usage"}),
            ),
        )
        .await?;
        let usage = wait_for_control(&mut reader, "usage-after-login").await?;
        let probe = claude_account_probe(&initialization, &usage)?;
        sink.publish_snapshot(&probe).await
    })
    .await;
    let snapshot_error = match snapshot {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(_) => Some("Claude account model and usage snapshot timed out".to_owned()),
    };
    Ok(LoginOutcome { snapshot_error })
}

/// Extract the authorization code from a bare code, Claude's `code#state`
/// value, or a full callback URL. Supplied state must match this login.
pub fn callback_code_and_state(raw: &str, expected_state: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.len() > 4_000 || raw.is_empty() {
        return Err("The authorization callback is invalid".into());
    }
    if !raw.starts_with("http://") && !raw.starts_with("https://") {
        if let Some((code, state)) = raw.rsplit_once('#') {
            if code.is_empty() || state.is_empty() {
                return Err("The authorization callback is invalid".into());
            }
            if state != expected_state {
                return Err("The callback does not belong to this Claude login".into());
            }
            return Ok(code.to_owned());
        }
        return Ok(raw.to_owned());
    }
    let url = url::Url::parse(raw).map_err(|_| "The authorization callback is invalid")?;
    let values = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    if values.get("state").map(|value| value.as_ref()) != Some(expected_state) {
        return Err("The callback does not belong to this Claude login".into());
    }
    values
        .get("code")
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .ok_or("The callback is missing its authorization code".into())
}

fn fingerprint(identifier: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(identifier.trim().to_lowercase().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Extract Claude's authenticated identity without depending on its model or
/// quota response shapes.
pub fn claude_identity_probe(initialization: &serde_json::Value) -> Result<AccountProbe, String> {
    let account = initialization.get("account").unwrap_or(initialization);
    let identifier = account
        .get("email")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            account
                .get("organization")
                .and_then(serde_json::Value::as_str)
        })
        .ok_or("Claude account identity is unavailable after login")?;
    Ok(AccountProbe {
        fingerprint: fingerprint(identifier),
        subscription_type: account
            .get("subscriptionType")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        models: serde_json::json!([]),
        usage: serde_json::json!({"windows": []}),
    })
}

fn claude_models(initialization: &serde_json::Value) -> Vec<serde_json::Value> {
    initialization
        .get("models")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let id = model.get("value").or_else(|| model.get("id"))?.as_str()?;
            Some(serde_json::json!({
                "id": id,
                "label": model
                    .get("displayName")
                    .or_else(|| model.get("display_name"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(id)
            }))
        })
        .collect()
}

fn claude_usage_label(id: &str) -> String {
    match id {
        "five_hour" => "5-hour",
        "seven_day" => "Weekly",
        "seven_day_oauth_apps" => "Weekly OAuth apps",
        "seven_day_opus" => "Weekly Opus",
        "seven_day_sonnet" => "Weekly Sonnet",
        "extra_usage" => "Monthly extra usage",
        _ => return id.replace('_', " "),
    }
    .to_owned()
}

/// Build a complete Claude model and exact-quota snapshot after identity has
/// already established authentication.
pub fn claude_account_probe(
    initialization: &serde_json::Value,
    usage: &serde_json::Value,
) -> Result<AccountProbe, String> {
    let mut probe = claude_identity_probe(initialization)?;
    let models = claude_models(initialization);
    if models.is_empty() {
        return Err("Claude returned no selectable models".into());
    }
    let limits = usage
        .get("rate_limits")
        .and_then(serde_json::Value::as_object)
        .ok_or("Claude did not return exact plan rate limits")?;
    let mut windows = Vec::new();
    for (id, limit) in limits {
        let Some(value) = limit.as_object() else {
            continue;
        };
        let Some(used) = value.get("utilization").and_then(serde_json::Value::as_f64) else {
            continue;
        };
        if !(0.0..=100.0).contains(&used) {
            return Err("Claude returned invalid exact usage".into());
        }
        windows.push(serde_json::json!({
            "id": id,
            "label": claude_usage_label(id),
            "usedPercent": used,
            "remainingPercent": 100.0 - used,
            "resetsAt": value.get("resets_at").and_then(serde_json::Value::as_str),
            "windowDurationMinutes": serde_json::Value::Null,
        }));
    }
    if windows.is_empty() {
        return Err("Claude did not return exact plan rate limits".into());
    }
    probe.subscription_type = usage
        .get("subscription_type")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or(probe.subscription_type);
    probe.models = serde_json::Value::Array(models);
    probe.usage = serde_json::json!({"windows":windows});
    Ok(probe)
}

/// Turn the Codex app-server's `account/read`, `account/rateLimits/read` and
/// `model/list` results into an account snapshot.
pub fn codex_account_probe(
    account: &serde_json::Value,
    rate_limits: &serde_json::Value,
    models: &serde_json::Value,
) -> Result<AccountProbe, String> {
    let mut probe = codex_identity_probe(account)?;
    let models = models.get("data").and_then(serde_json::Value::as_array)
        .ok_or("Harness returned no selectable models")?
        .iter()
        .filter_map(|model| {
            let id = model.get("id")?.as_str()?;
            Some(serde_json::json!({"id": id, "label": model.get("displayName").and_then(serde_json::Value::as_str).unwrap_or(id)}))
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        return Err("Harness returned no selectable models".into());
    }
    let snapshots = rate_limits
        .get("rateLimitsByLimitId")
        .and_then(serde_json::Value::as_object)
        .map(|values| {
            values
                .iter()
                .map(|(limit_id, snapshot)| (limit_id.as_str(), snapshot))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![(
                "default",
                rate_limits.get("rateLimits").unwrap_or(rate_limits),
            )]
        });
    let mut windows = Vec::new();
    let mut seen_windows = std::collections::HashSet::new();
    for (limit_id, snapshot) in snapshots {
        let limit_name = snapshot
            .get("limitName")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                (!matches!(limit_id, "codex" | "default")).then(|| limit_id.replace('_', " "))
            });
        for kind in ["primary", "secondary"] {
            let Some(window) = snapshot.get(kind).and_then(serde_json::Value::as_object) else {
                continue;
            };
            let Some(used) = window
                .get("usedPercent")
                .and_then(serde_json::Value::as_f64)
            else {
                continue;
            };
            if !(0.0..=100.0).contains(&used) {
                return Err("Harness returned invalid exact usage".into());
            }
            let duration = window
                .get("windowDurationMins")
                .and_then(serde_json::Value::as_i64);
            let resets_at = window.get("resetsAt").and_then(serde_json::Value::as_i64);
            let signature = format!("{duration:?}:{resets_at:?}:{used}");
            if !seen_windows.insert(signature) {
                continue;
            }
            let period = match duration {
                Some(10_080) => "Weekly",
                Some(300) => "5-hour",
                _ => kind,
            };
            let label = limit_name
                .as_ref()
                .map(|name| format!("{name} {period}"))
                .unwrap_or_else(|| period.to_owned());
            windows.push(serde_json::json!({
                "id": format!("{limit_id}:{kind}"),
                "label": label,
                "usedPercent": used,
                "remainingPercent": 100.0 - used,
                "resetsAt": resets_at.and_then(|seconds| chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0).map(|value| value.to_rfc3339())),
                "windowDurationMinutes": duration,
            }));
        }
    }
    if windows.is_empty() {
        return Err("Harness returned no exact quota windows".into());
    }
    probe.models = serde_json::Value::Array(models);
    probe.usage = serde_json::json!({"windows": windows});
    Ok(probe)
}

/// Extract Codex's authenticated identity without depending on its model or
/// quota response shapes.
pub fn codex_identity_probe(account: &serde_json::Value) -> Result<AccountProbe, String> {
    let account = account.get("account").unwrap_or(account);
    let identifier = account
        .get("email")
        .and_then(serde_json::Value::as_str)
        .or_else(|| account.get("id").and_then(serde_json::Value::as_str))
        .ok_or("Harness account identity is unavailable after login")?;
    Ok(AccountProbe {
        fingerprint: fingerprint(identifier),
        subscription_type: account
            .get("planType")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        models: serde_json::json!([]),
        usage: serde_json::json!({"windows": []}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn callback_url_must_match_the_claude_authorization_state() {
        assert_eq!(
            callback_code_and_state(
                "https://platform.claude.com/oauth/code/callback?code=one&state=expected",
                "expected"
            ),
            Ok("one".into())
        );
        assert!(
            callback_code_and_state(
                "https://platform.claude.com/oauth/code/callback?code=one&state=other",
                "expected"
            )
            .is_err()
        );
        assert_eq!(
            callback_code_and_state("  bare-code  ", "x"),
            Ok("bare-code".into())
        );
        assert_eq!(
            callback_code_and_state("authorization-code#expected", "expected"),
            Ok("authorization-code".into())
        );
        assert!(callback_code_and_state("authorization-code#other", "expected").is_err());
        assert!(callback_code_and_state("#expected", "expected").is_err());
        assert!(callback_code_and_state("authorization-code#", "expected").is_err());
    }

    #[test]
    fn claude_control_errors_include_only_bounded_plain_text() {
        assert_eq!(
            claude_control_error(&serde_json::json!({
                "error": "No active claude_authenticate flow"
            })),
            "Claude login request failed: No active claude_authenticate flow"
        );
        assert_eq!(
            claude_control_error(&serde_json::json!({"error": "bad\nsecret"})),
            "Claude login request failed"
        );
        assert_eq!(
            claude_control_error(&serde_json::json!({"error": "x".repeat(513)})),
            "Claude login request failed"
        );
    }

    #[tokio::test]
    async fn remote_codex_callback_is_forwarded_only_to_its_loopback_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 2048];
            let size = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.starts_with("GET /auth/callback?code=code-1&state=state-1 "));
            stream
                .write_all(b"HTTP/1.1 302 Found\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let redirect = format!("http://localhost:{}/auth/callback", address.port());
        let mut authorization = url::Url::parse("https://auth.openai.com/oauth/authorize").unwrap();
        authorization
            .query_pairs_mut()
            .append_pair("state", "state-1")
            .append_pair("redirect_uri", &redirect);

        forward_codex_callback(
            authorization.as_str(),
            "http://localhost:9999/auth/callback?code=code-1&state=state-1",
        )
        .await
        .unwrap();
        server.await.unwrap();

        let wrong_state = forward_codex_callback(
            authorization.as_str(),
            "http://localhost:9999/auth/callback?code=code-1&state=other",
        )
        .await
        .unwrap_err();
        assert_eq!(
            wrong_state,
            "The callback does not belong to this Codex login"
        );

        let mut unsafe_authorization = authorization.clone();
        unsafe_authorization
            .query_pairs_mut()
            .clear()
            .append_pair("state", "state-1")
            .append_pair("redirect_uri", "http://example.com/auth/callback");
        assert_eq!(
            forward_codex_callback(
                unsafe_authorization.as_str(),
                "http://localhost/auth/callback?code=code-1&state=state-1",
            )
            .await
            .unwrap_err(),
            "Codex returned an unsafe callback URL"
        );
    }

    #[test]
    fn codex_probe_requires_exact_models_and_rate_limits() {
        let probe = codex_account_probe(
            &serde_json::json!({"account":{"email":"builder@example.test","planType":"team"}}),
            &serde_json::json!({"rateLimits":{"primary":{"usedPercent":38.0,"windowDurationMins":10080,"resetsAt":1_800_000_000}}}),
            &serde_json::json!({"data":[{"id":"gpt-test","displayName":"GPT Test"}]}),
        )
        .unwrap();
        assert_eq!(probe.models[0]["id"], "gpt-test");
        assert_eq!(probe.usage["windows"][0]["remainingPercent"], 62.0);
        assert_eq!(probe.subscription_type.as_deref(), Some("team"));
    }

    #[test]
    fn codex_probe_names_distinct_buckets_and_deduplicates_mirrors() {
        let probe = codex_account_probe(
            &serde_json::json!({"account":{"email":"builder@example.test","planType":"pro"}}),
            &serde_json::json!({"rateLimitsByLimitId":{
                "codex":{"limitId":"codex","primary":{"usedPercent":80.0,"windowDurationMins":10080,"resetsAt":1_800_000_000}},
                "codex_bengalfox":{"limitId":"codex_bengalfox","limitName":"GPT-5.3-Codex-Spark","primary":{"usedPercent":0.0,"windowDurationMins":300,"resetsAt":1_800_000_300},"secondary":{"usedPercent":0.0,"windowDurationMins":10080,"resetsAt":1_800_000_600}},
                "mirrored":{"limitName":"Mirror","primary":{"usedPercent":80.0,"windowDurationMins":10080,"resetsAt":1_800_000_000}}
            }}),
            &serde_json::json!({"data":[{"id":"gpt-test","displayName":"GPT Test"}]}),
        )
        .unwrap();
        assert_eq!(
            probe.usage["windows"],
            serde_json::json!([
                {"id":"codex:primary","label":"Weekly","usedPercent":80.0,"remainingPercent":20.0,"resetsAt":"2027-01-15T08:00:00+00:00","windowDurationMinutes":10080},
                {"id":"codex_bengalfox:primary","label":"GPT-5.3-Codex-Spark 5-hour","usedPercent":0.0,"remainingPercent":100.0,"resetsAt":"2027-01-15T08:05:00+00:00","windowDurationMinutes":300},
                {"id":"codex_bengalfox:secondary","label":"GPT-5.3-Codex-Spark Weekly","usedPercent":0.0,"remainingPercent":100.0,"resetsAt":"2027-01-15T08:10:00+00:00","windowDurationMinutes":10080}
            ])
        );
    }

    #[test]
    fn claude_probe_reads_identity_models_and_rate_limit_windows() {
        let probe = claude_account_probe(
            &serde_json::json!({"account":{"email":"builder@example.test"},"models":[{"value":"claude-sonnet","displayName":"Sonnet"}]}),
            &serde_json::json!({"subscription_type":"max","rate_limits":{"five_hour":{"utilization":25.0,"resets_at":"2026-09-03T10:00:00Z"}}}),
        )
        .unwrap();
        assert_eq!(probe.models[0]["label"], "Sonnet");
        assert_eq!(probe.usage["windows"][0]["id"], "five_hour");
        assert_eq!(probe.usage["windows"][0]["label"], "5-hour");
        assert_eq!(probe.usage["windows"][0]["remainingPercent"], 75.0);
        assert_eq!(probe.subscription_type.as_deref(), Some("max"));
    }

    #[test]
    fn claude_identity_ignores_catalog_and_snapshot_accepts_legacy_model_ids() {
        let identity = claude_identity_probe(&serde_json::json!({
            "account": {"email": "builder@example.test", "subscriptionType": "max"},
            "models": []
        }))
        .unwrap();
        assert_eq!(identity.models, serde_json::json!([]));
        assert_eq!(identity.usage, serde_json::json!({"windows": []}));

        let legacy = claude_account_probe(
            &serde_json::json!({
                "account": {"email": "builder@example.test"},
                "models": [{"id": "claude-legacy", "display_name": "Legacy"}]
            }),
            &serde_json::json!({
                "rate_limits": {"five_hour": {"utilization": 0.0}}
            }),
        )
        .unwrap();
        assert_eq!(legacy.models[0]["id"], "claude-legacy");
        assert_eq!(legacy.models[0]["label"], "Legacy");
    }

    #[test]
    fn claude_probe_uses_the_same_stable_quota_labels_as_the_dashboard() {
        let probe = claude_account_probe(
            &serde_json::json!({"account":{"email":"builder@example.test"},"models":[{"value":"claude-sonnet"}]}),
            &serde_json::json!({
                "rate_limits": {
                    "five_hour": {"utilization": 20.0},
                    "seven_day": {"utilization": 39.0},
                    "nimbus_quill": {"utilization": 0.0}
                }
            }),
        )
        .unwrap();
        assert_eq!(
            probe.usage["windows"],
            serde_json::json!([
                {"id":"five_hour","label":"5-hour","usedPercent":20.0,"remainingPercent":80.0,"resetsAt":null,"windowDurationMinutes":null},
                {"id":"nimbus_quill","label":"nimbus quill","usedPercent":0.0,"remainingPercent":100.0,"resetsAt":null,"windowDurationMinutes":null},
                {"id":"seven_day","label":"Weekly","usedPercent":39.0,"remainingPercent":61.0,"resetsAt":null,"windowDurationMinutes":null}
            ])
        );
    }
}
