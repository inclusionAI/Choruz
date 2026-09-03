use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::{Path as FsPath, PathBuf};
use std::sync::LazyLock;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use choruz_common::AppError;
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
use tokio::sync::Mutex;

use crate::{ApiError, ApiState, require_human_operator};

const SSH_CONNECT_TIMEOUT_SECS: u64 = 15;
const TUNNEL_READY_TIMEOUT_SECS: u64 = SSH_CONNECT_TIMEOUT_SECS + 5;
const SSH_HOST_RESOLVE_TIMEOUT_SECS: u64 = 5;
const SSH_HOST_LIST_TIMEOUT_SECS: u64 = 10;
const MAX_RESOLVED_SSH_HOSTS: usize = 128;
const MAX_SSH_STDERR_BYTES: usize = 64 * 1024;
const _: () = assert!(TUNNEL_READY_TIMEOUT_SECS >= SSH_CONNECT_TIMEOUT_SECS);
const _: () = assert!(SSH_HOST_LIST_TIMEOUT_SECS >= SSH_HOST_RESOLVE_TIMEOUT_SECS);

// ── Types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SshHost {
    name: String,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity_file: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TunnelRequest {
    host: String,
    #[serde(default)]
    local_port: Option<u16>,
    #[serde(default)]
    remote_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TunnelInfo {
    id: String,
    host: String,
    local_port: u16,
    remote_port: u16,
    pid: Option<u32>,
    started_at: String,
    generation: Option<u64>,
    status: TunnelStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    disconnected_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TunnelStatus {
    Ready,
    Disconnected,
}

// ── In-memory tunnel store ────────────────────────────────────────────

struct TunnelEntry {
    info: TunnelInfo,
    /// The `ssh -L … -N` process that holds the port forward open. `kill()`
    /// this to tear down the tunnel.
    child: Child,
    /// For VS-Code-style tunnels: the `ssh host 'choruz-server'` process
    /// that's running on the remote. `None` for manual tunnels where the
    /// user just forwarded a port without us spawning a remote process.
    runner_child: Option<Child>,
}

impl TunnelEntry {
    fn refresh_status(&mut self) {
        if self.info.status == TunnelStatus::Disconnected {
            return;
        }
        let tunnel_failure = child_failure(&mut self.child, "SSH tunnel");
        let runner_failure = self
            .runner_child
            .as_mut()
            .and_then(|runner| child_failure(runner, "remote Choruz runner"));
        if let Some(error) = tunnel_failure.or(runner_failure) {
            let _ = self.child.start_kill();
            if let Some(runner) = self.runner_child.as_mut() {
                let _ = runner.start_kill();
            }
            self.info.status = TunnelStatus::Disconnected;
            self.info.disconnected_at = Some(chrono::Utc::now().to_rfc3339());
            self.info.last_error = Some(error);
        }
    }
}

fn child_failure(child: &mut Child, label: &str) -> Option<String> {
    match child.try_wait() {
        Ok(Some(status)) => Some(format!("{label} exited with {status}")),
        Ok(None) => None,
        Err(error) => Some(format!("could not inspect {label}: {error}")),
    }
}

#[derive(Default)]
struct TunnelRegistry {
    entries: HashMap<String, TunnelEntry>,
    latest_generation: HashMap<String, u64>,
}

impl TunnelRegistry {
    fn reserve_generation(&mut self, host: &str) -> Result<u64, ApiError> {
        let generation = self
            .latest_generation
            .get(host)
            .copied()
            .unwrap_or_default()
            .checked_add(1)
            .ok_or_else(|| {
                ApiError::from(AppError::Internal(format!(
                    "SSH session generation exhausted for {host}"
                )))
            })?;
        self.latest_generation.insert(host.to_string(), generation);
        Ok(generation)
    }

    fn is_current(&self, host: &str, generation: u64) -> bool {
        self.latest_generation.get(host).copied() == Some(generation)
    }
}

static TUNNELS: LazyLock<Mutex<TunnelRegistry>> =
    LazyLock::new(|| Mutex::new(TunnelRegistry::default()));

// ── SSH config parser ─────────────────────────────────────────────────

fn strip_inline_comment(line: &str) -> &str {
    let mut escaped = false;
    let mut quote = None;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if matches!(ch, '\'' | '"') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            }
            continue;
        }
        if ch == '#' && quote.is_none() {
            return &line[..index];
        }
    }
    line
}

fn split_directive(line: &str) -> Option<(&str, &str)> {
    let line = strip_inline_comment(line).trim();
    let split_at = line.find(|ch: char| ch.is_whitespace() || ch == '=')?;
    let (key, rest) = line.split_at(split_at);
    let rest = rest.trim_start();
    let value = rest.strip_prefix('=').unwrap_or(rest).trim();
    (!key.is_empty() && !value.is_empty()).then_some((key, value))
}

fn is_concrete_host_alias(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('!') && !value.contains('*') && !value.contains('?')
}

fn split_ssh_words(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if matches!(ch, '\'' | '"') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            } else {
                current.push(ch);
            }
        } else if ch.is_whitespace() && quote.is_none() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn expand_home_path(value: &str, ssh_dir: &FsPath) -> PathBuf {
    if value == "~" {
        return ssh_dir.parent().unwrap_or(ssh_dir).to_path_buf();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return ssh_dir.parent().unwrap_or(ssh_dir).join(rest);
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        ssh_dir.join(path)
    }
}

fn collect_ssh_aliases_from_file(
    config_path: &FsPath,
    ssh_dir: &FsPath,
    visited: &mut HashSet<PathBuf>,
    aliases: &mut Vec<String>,
    seen_aliases: &mut HashSet<String>,
) -> Result<(), std::io::Error> {
    let canonical =
        std::fs::canonicalize(config_path).unwrap_or_else(|_| config_path.to_path_buf());
    if !visited.insert(canonical) {
        return Ok(());
    }

    let content = match std::fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    for line in content.lines() {
        let Some((key, value)) = split_directive(line) else {
            continue;
        };
        if key.eq_ignore_ascii_case("host") {
            for alias in split_ssh_words(value)
                .into_iter()
                .filter(|name| is_concrete_host_alias(name))
            {
                if seen_aliases.insert(alias.clone()) {
                    aliases.push(alias);
                }
            }
        } else if key.eq_ignore_ascii_case("include") {
            for pattern in split_ssh_words(value) {
                let expanded = expand_home_path(&pattern, ssh_dir);
                let pattern = expanded.to_string_lossy();
                let matches = match glob::glob(&pattern) {
                    Ok(matches) => matches,
                    Err(error) => {
                        tracing::warn!(%pattern, %error, "skipping invalid ssh Include pattern");
                        continue;
                    }
                };
                for included in matches.flatten() {
                    if let Err(error) = collect_ssh_aliases_from_file(
                        &included,
                        ssh_dir,
                        visited,
                        aliases,
                        seen_aliases,
                    ) {
                        tracing::warn!(path = %included.display(), %error, "skipping unreadable ssh Include file");
                    }
                }
            }
        }
    }

    Ok(())
}

fn collect_ssh_aliases(config_path: &FsPath) -> Result<Vec<String>, std::io::Error> {
    let ssh_dir = config_path.parent().unwrap_or_else(|| FsPath::new("."));
    let mut aliases = Vec::new();
    collect_ssh_aliases_from_file(
        config_path,
        ssh_dir,
        &mut HashSet::new(),
        &mut aliases,
        &mut HashSet::new(),
    )?;
    Ok(aliases)
}

fn parse_effective_ssh_config(name: String, content: &str) -> SshHost {
    let mut host = SshHost {
        name,
        hostname: None,
        user: None,
        port: None,
        identity_file: None,
    };
    for line in content.lines() {
        let Some((key, value)) = split_directive(line) else {
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            "hostname" if host.hostname.is_none() => host.hostname = Some(value.to_string()),
            "user" if host.user.is_none() => host.user = Some(value.to_string()),
            "port" if host.port.is_none() => host.port = value.parse().ok(),
            "identityfile" if host.identity_file.is_none() => {
                host.identity_file = Some(value.to_string())
            }
            _ => {}
        }
    }
    host
}

async fn resolve_ssh_host(alias: String) -> SshHost {
    let mut command = Command::new("ssh");
    command.arg("-G").arg("--").arg(&alias).kill_on_drop(true);
    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(SSH_HOST_RESOLVE_TIMEOUT_SECS),
        command.output(),
    )
    .await
    {
        Ok(output) => output,
        Err(_) => {
            tracing::warn!(host = %alias, "ssh -G timed out");
            return parse_effective_ssh_config(alias, "");
        }
    };
    match output {
        Ok(output) if output.status.success() => {
            parse_effective_ssh_config(alias, &String::from_utf8_lossy(&output.stdout))
        }
        Ok(output) => {
            tracing::warn!(
                host = %alias,
                error = %String::from_utf8_lossy(&output.stderr).trim(),
                "ssh -G failed; returning alias without computed metadata"
            );
            parse_effective_ssh_config(alias, "")
        }
        Err(error) => {
            tracing::warn!(host = %alias, %error, "could not execute ssh -G");
            parse_effective_ssh_config(alias, "")
        }
    }
}

async fn read_ssh_stderr(stderr: Option<ChildStderr>) -> String {
    let Some(mut pipe) = stderr else {
        return String::new();
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    // ProxyJump/ProxyCommand descendants may inherit stderr after the main ssh
    // process exits. Never let diagnostics keep an HTTP request open forever.
    while buffer.len() < MAX_SSH_STDERR_BYTES {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let read_len = chunk.len().min(MAX_SSH_STDERR_BYTES - buffer.len());
        match tokio::time::timeout(remaining, pipe.read(&mut chunk[..read_len])).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(count)) => buffer.extend_from_slice(&chunk[..count]),
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

fn ssh_config_path(home: &str) -> Option<PathBuf> {
    if home.trim().is_empty() {
        return None;
    }
    Some(FsPath::new(home).join(".ssh").join("config"))
}

async fn read_ssh_aliases(home: &str) -> Result<Vec<String>, ApiError> {
    let Some(config_path) = ssh_config_path(home) else {
        return Ok(Vec::new());
    };
    tokio::task::spawn_blocking(move || collect_ssh_aliases(&config_path))
        .await
        .map_err(|error| ApiError::from(AppError::Internal(format!("scan ssh config: {error}"))))?
        .map_err(|error| ApiError::from(AppError::Internal(format!("read ssh config: {error}"))))
}

// ── Handlers ──────────────────────────────────────────────────────────

/// GET /v1/ssh/hosts — parse ~/.ssh/config and return available hosts.
pub(crate) async fn list_ssh_hosts(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<Vec<SshHost>>, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    let home = std::env::var("HOME").unwrap_or_default();
    let aliases = read_ssh_aliases(&home).await?;
    // `ssh -G` is local-only but may evaluate slow `Match exec` rules. Resolve
    // several aliases at once without spawning an unbounded process burst.
    if aliases.len() > MAX_RESOLVED_SSH_HOSTS {
        tracing::warn!(
            aliases = aliases.len(),
            resolved = MAX_RESOLVED_SSH_HOSTS,
            "ssh host list exceeds metadata resolution limit; remaining aliases will be returned without computed metadata"
        );
    }
    let fallback = aliases.clone();
    let mut resolution = stream::iter(aliases.into_iter().enumerate().map(
        |(index, alias)| async move {
            let host = if index < MAX_RESOLVED_SSH_HOSTS {
                resolve_ssh_host(alias).await
            } else {
                parse_effective_ssh_config(alias, "")
            };
            (index, host)
        },
    ))
    .buffer_unordered(8);
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(SSH_HOST_LIST_TIMEOUT_SECS);
    let mut resolved = vec![None; fallback.len()];
    loop {
        match tokio::time::timeout_at(deadline, resolution.next()).await {
            Ok(Some((index, host))) => resolved[index] = Some(host),
            Ok(None) => break,
            Err(_) => {
                tracing::warn!(
                    timeout_seconds = SSH_HOST_LIST_TIMEOUT_SECS,
                    "ssh host metadata resolution timed out; preserving completed metadata and returning remaining aliases unresolved"
                );
                break;
            }
        }
    }
    let hosts = fallback
        .into_iter()
        .zip(resolved)
        .map(|(alias, host)| host.unwrap_or_else(|| parse_effective_ssh_config(alias, "")))
        .collect();
    Ok(Json(hosts))
}

/// POST /v1/ssh/tunnel — spawn an `ssh -L <local>:localhost:<remote> -N <host>`
/// child process and track it in-memory. Returns the tunnel id so the caller
/// can open the forwarded URL and later close the tunnel.
pub(crate) async fn create_tunnel(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(req): Json<TunnelRequest>,
) -> Result<Json<TunnelInfo>, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    if req.host.trim().is_empty() {
        return Err(ApiError::from(AppError::Validation(
            "host must not be empty".to_string(),
        )));
    }

    let local_port = req.local_port.unwrap_or(3100);
    let remote_port = req.remote_port.unwrap_or(3100);
    let forward = format!("{local_port}:localhost:{remote_port}");

    let mut cmd = Command::new("ssh");
    cmd.arg("-N")
        .arg("-T")
        .arg("-o")
        .arg("RemoteCommand=none")
        .arg("-o")
        .arg(format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"))
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-L")
        .arg(&forward)
        .arg("--")
        .arg(&req.host)
        // Close stdin so the child doesn't block waiting for input, and
        // drop stdout/stderr so buffers don't fill.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);

    let child = cmd
        .spawn()
        .map_err(|e| ApiError::from(AppError::Internal(format!("spawn ssh: {e}"))))?;

    let pid = child.id();
    let id = choruz_common::new_id();
    let info = TunnelInfo {
        id: id.clone(),
        host: req.host.clone(),
        local_port,
        remote_port,
        pid,
        started_at: chrono::Utc::now().to_rfc3339(),
        generation: None,
        status: TunnelStatus::Ready,
        disconnected_at: None,
        last_error: None,
    };

    let mut tunnels = TUNNELS.lock().await;
    tunnels.entries.insert(
        id.clone(),
        TunnelEntry {
            info: info.clone(),
            child,
            runner_child: None,
        },
    );

    Ok(Json(info))
}

// ── VS-Code-style single-click connect ────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ConnectChoruzRequest {
    host: String,
    /// Optional override: the choruz-server binary name to run on the
    /// remote. Defaults to `"choruz-server"` (expected on $PATH). Useful
    /// if the user installed it somewhere non-standard.
    #[serde(default)]
    remote_binary: Option<String>,
}

/// POST /v1/ssh/connect-choruz — the VS-Code-Remote-SSH analog:
///
/// 1. `ssh {host} '<remote_binary>'` — runs `choruz-server` on the remote,
///    which boots its own embedded Postgres + gateway + pipeline, then
///    prints `CHORUZ_LISTENING=<port>` on stdout.
/// 2. Read that first line (with a timeout) to learn the remote port.
/// 3. Pick a free local high port (random, not 3100, to avoid colliding
///    with a local Choruz dev server).
/// 4. `ssh -L {local}:localhost:{remote} -N {host}` — the actual tunnel.
/// 5. Track BOTH ssh children as one TunnelEntry so Disconnect tears
///    down the remote runner AND the tunnel in one shot.
///
/// The user never sees a port number. One click, they're in.
pub(crate) async fn connect_choruz_tunnel(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(req): Json<ConnectChoruzRequest>,
) -> Result<Json<TunnelInfo>, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    if req.host.trim().is_empty() {
        return Err(ApiError::from(AppError::Validation(
            "host must not be empty".to_string(),
        )));
    }

    // The remote is expected to already have `choruz-server` on its
    // $PATH (or the caller passes the absolute path via `remote_binary`).
    // We tried auto-deploying it over scp at one point, but:
    //   - same-arch deploy worked, cross-arch required a local
    //     cross-compile toolchain the user probably didn't have
    //   - a "real" VS-Code-style solution needs prebuilt CI artifacts +
    //     release channel, out of scope for Choruz
    // so we just require the user to install it themselves. The UI's
    // Remote Servers modal surfaces this expectation in its intro copy.
    let remote_binary = req
        .remote_binary
        .as_deref()
        .unwrap_or("choruz-server")
        .to_string();
    if !is_safe_remote_binary(&remote_binary) {
        return Err(ApiError::from(AppError::Validation(
            "remote_binary must be choruz-server or an absolute executable path".to_string(),
        )));
    }

    // Allocate before any remote work begins. A newer attempt for the same
    // host immediately fences this one, even if this request later completes
    // an older SSH handshake out of order.
    let generation = TUNNELS.lock().await.reserve_generation(&req.host)?;

    // Step 1: spawn the remote runner. ssh inherits our stdin (but we
    // give it /dev/null) and pipes stdout so we can parse the handshake
    // line.
    let mut runner_cmd = Command::new("ssh");
    runner_cmd
        .arg("-T")
        .arg("-o")
        .arg("RemoteCommand=none")
        .arg("-o")
        .arg(format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"))
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("--")
        .arg(&req.host)
        // Run the remote binary with unbuffered stdout so we see the
        // handshake line immediately. `stdbuf -oL` forces line-buffering
        // on most Linux hosts; falls back gracefully if stdbuf isn't
        // installed (the binary uses explicit flush() anyway).
        .arg(format!(
            "stdbuf -oL {remote_binary} 2>&1 || {remote_binary}"
        ))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut runner = runner_cmd
        .spawn()
        .map_err(|e| ApiError::from(AppError::Internal(format!("spawn ssh runner: {e}"))))?;

    let runner_stdout = runner
        .stdout
        .take()
        .ok_or_else(|| ApiError::from(AppError::Internal("ssh runner has no stdout".into())))?;
    let mut reader = BufReader::new(runner_stdout).lines();

    // Step 2: read lines until we see `CHORUZ_LISTENING=` or time out.
    // 30s is comfortable: a cold-start remote that downloads Postgres
    // for the first time takes ~30s. Subsequent connects are ~2s.
    let handshake = tokio::time::timeout(std::time::Duration::from_secs(45), async {
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::debug!(host = %req.host, line, "ssh runner stdout");
            if let Some(port_str) = line.trim().strip_prefix("CHORUZ_LISTENING=") {
                return port_str.parse::<u16>().ok();
            }
        }
        None
    })
    .await;

    let remote_port = match handshake {
        Ok(Some(p)) => p,
        Ok(None) => {
            // The runner finished without emitting the line — could be
            // any of: `choruz-server` missing, ssh auth failed, network
            // hiccup. Grab stderr to figure out which.
            let stderr = runner.stderr.take();
            let _ = runner.start_kill();
            // Read the FULL stderr; we want to detect known patterns
            // (auth failures) wherever they appear. OpenSSH on Princeton
            // hosts prepends 3 lines of post-quantum warnings before the
            // real "Permission denied" line, so a naive .take(3) misses it.
            let full_stderr = read_ssh_stderr(stderr).await;
            // Display snippet stays short (first 3 informative lines,
            // skipping leading blanks).
            let snippet = full_stderr
                .lines()
                .filter(|l| !l.trim().is_empty())
                .take(3)
                .collect::<Vec<_>>()
                .join(" | ");

            // Special-case keyboard-interactive auth failures. macOS
            // OpenSSH backend can't prompt for Duo / 2FA / passwords
            // because we spawn ssh with `Stdio::null()` and no PTY —
            // there's no channel to surface the prompt over. Tell the
            // user to pre-authenticate via terminal so OpenSSH's
            // ControlMaster persists the auth, then we silently reuse.
            if full_stderr.contains("Permission denied (keyboard-interactive")
                || full_stderr.contains("Permission denied, please try again")
                || full_stderr.contains("Too many authentication failures")
            {
                return Err(ApiError::from(AppError::Internal(format!(
                    "{} requires interactive auth (Duo / password / 2FA) which we can't prompt for over HTTP. \
                     Open a terminal and run `ssh {0} exit` first; complete the prompt; then click Connect again — \
                     OpenSSH's ControlMaster will reuse the authenticated session. \
                     Underlying: {snippet}",
                    req.host
                ))));
            }
            if is_known_ssh_failure(&full_stderr) {
                return Err(ApiError::from(AppError::Internal(describe_ssh_failure(
                    &req.host,
                    "starting choruz-server",
                    &full_stderr,
                ))));
            }
            return Err(ApiError::from(AppError::Internal(format!(
                "remote choruz-server never emitted CHORUZ_LISTENING line (is '{remote_binary}' installed on {}?): {snippet}",
                req.host
            ))));
        }
        Err(_) => {
            let stderr = runner.stderr.take();
            let _ = runner.start_kill();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), runner.wait()).await;
            let full_stderr = read_ssh_stderr(stderr).await;
            if !full_stderr.trim().is_empty() {
                return Err(ApiError::from(AppError::Internal(describe_ssh_failure(
                    &req.host,
                    "starting choruz-server",
                    &full_stderr,
                ))));
            }
            return Err(ApiError::from(AppError::Internal(format!(
                "timeout waiting for remote choruz-server on {} to announce its port",
                req.host
            ))));
        }
    };
    if !TUNNELS.lock().await.is_current(&req.host, generation) {
        let _ = runner.start_kill();
        let _ = runner.wait().await;
        return Err(superseded_session_error(&req.host, generation));
    }
    tracing::info!(host = %req.host, remote_port, "remote choruz-server ready");

    // Step 3: pick a free local port in the high range. `bind(0)` lets the
    // OS pick; we read back the assigned port, then drop the listener so
    // the actual ssh tunnel can claim it. There's a narrow TOCTOU window
    // but on a desktop/dev machine it's a non-issue.
    let local_port = pick_free_port().ok_or_else(|| {
        ApiError::from(AppError::Internal(
            "could not find a free local port".into(),
        ))
    })?;

    // Step 4: spawn the port-forward tunnel as a SECOND ssh process.
    let forward = format!("{local_port}:localhost:{remote_port}");
    let mut tunnel_cmd = Command::new("ssh");
    tunnel_cmd
        .arg("-N")
        .arg("-T")
        .arg("-o")
        .arg("RemoteCommand=none")
        .arg("-o")
        .arg(format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"))
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-L")
        .arg(&forward)
        .arg("--")
        .arg(&req.host)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut tunnel_child = tunnel_cmd.spawn().map_err(|e| {
        // If the tunnel fails, kill the runner too so we don't leak.
        let _ = runner.start_kill();
        ApiError::from(AppError::Internal(format!("spawn ssh tunnel: {e}")))
    })?;

    // Wait until the forwarded port accepts connections. A fixed sleep raced
    // slow ProxyJump/auth handshakes and also reported success when ssh had
    // already exited. Keep the wait bounded and surface OpenSSH's real error.
    let tunnel_ready = tokio::time::timeout(
        std::time::Duration::from_secs(TUNNEL_READY_TIMEOUT_SECS),
        async {
            loop {
                if let Some(status) = tunnel_child.try_wait().map_err(|error| error.to_string())? {
                    let stderr = read_ssh_stderr(tunnel_child.stderr.take()).await;
                    return Err(if stderr.trim().is_empty() {
                        format!("ssh tunnel exited with {status}")
                    } else {
                        stderr
                    });
                }
                if tokio::net::TcpStream::connect(("127.0.0.1", local_port))
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        },
    )
    .await;

    match tunnel_ready {
        Ok(Ok(())) => {
            // The remote server is long-lived too. Continue consuming both of
            // its pipes after the handshake so application logs cannot stall
            // the server or its SSH session once their pipe buffers fill.
            let runner_host = req.host.clone();
            tokio::spawn(async move {
                while let Ok(Some(line)) = reader.next_line().await {
                    tracing::debug!(host = %runner_host, line, "ssh runner stdout");
                }
            });
            if let Some(stderr) = runner.stderr.take() {
                let runner_host = req.host.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        tracing::debug!(host = %runner_host, line, "ssh runner stderr");
                    }
                });
            }
            // The connection is long-lived. Keep draining diagnostics so a
            // chatty ssh/ProxyCommand process cannot fill the pipe and stall
            // the tunnel after it has been registered.
            if let Some(stderr) = tunnel_child.stderr.take() {
                let host = req.host.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        tracing::debug!(host = %host, line, "ssh tunnel stderr");
                    }
                });
            }
        }
        Ok(Err(stderr)) => {
            let _ = tunnel_child.start_kill();
            let _ = runner.start_kill();
            return Err(ApiError::from(AppError::Internal(describe_ssh_failure(
                &req.host,
                "opening the port forward",
                &stderr,
            ))));
        }
        Err(_) => {
            let _ = tunnel_child.start_kill();
            let _ = runner.start_kill();
            return Err(ApiError::from(AppError::Internal(format!(
                "timed out waiting for the SSH port forward to {}",
                req.host
            ))));
        }
    }

    if !TUNNELS.lock().await.is_current(&req.host, generation) {
        let _ = tunnel_child.start_kill();
        let _ = runner.start_kill();
        let _ = tunnel_child.wait().await;
        let _ = runner.wait().await;
        return Err(superseded_session_error(&req.host, generation));
    }

    let pid = tunnel_child.id();
    let id = choruz_common::new_id();
    let info = TunnelInfo {
        id: id.clone(),
        host: req.host.clone(),
        local_port,
        remote_port,
        pid,
        started_at: chrono::Utc::now().to_rfc3339(),
        generation: Some(generation),
        status: TunnelStatus::Ready,
        disconnected_at: None,
        last_error: None,
    };

    let mut tunnels = TUNNELS.lock().await;
    if !tunnels.is_current(&req.host, generation) {
        drop(tunnels);
        let _ = tunnel_child.start_kill();
        let _ = runner.start_kill();
        let _ = tunnel_child.wait().await;
        let _ = runner.wait().await;
        return Err(superseded_session_error(&req.host, generation));
    }
    let replaced_ids = tunnels
        .entries
        .iter()
        .filter_map(|(id, entry)| {
            managed_session_matches(&entry.info, &req.host).then_some(id.clone())
        })
        .collect::<Vec<_>>();
    let replaced = replaced_ids
        .into_iter()
        .filter_map(|id| tunnels.entries.remove(&id))
        .collect::<Vec<_>>();
    tunnels.entries.insert(
        id.clone(),
        TunnelEntry {
            info: info.clone(),
            child: tunnel_child,
            runner_child: Some(runner),
        },
    );
    drop(tunnels);
    for mut entry in replaced {
        stop_tunnel_entry(&mut entry).await;
    }

    Ok(Json(info))
}

fn superseded_session_error(host: &str, generation: u64) -> ApiError {
    ApiError::from(AppError::Conflict(format!(
        "SSH session {host} generation {generation} was superseded by a newer connection attempt"
    )))
}

fn managed_session_matches(info: &TunnelInfo, host: &str) -> bool {
    info.host == host && info.generation.is_some()
}

fn is_safe_remote_binary(value: &str) -> bool {
    if value == "choruz-server" {
        return true;
    }

    value.starts_with('/')
        && value.split('/').skip(1).all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
}

fn pick_free_port() -> Option<u16> {
    // Bind to 0 → OS picks a free port. Drop the listener before we
    // return; ssh will race to claim it but on localhost this is fine.
    TcpListener::bind(("127.0.0.1", 0))
        .ok()?
        .local_addr()
        .ok()
        .map(|a| a.port())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SshFailureKind {
    HostKeyChanged,
    RemoteCommandConflict,
    Authentication,
    Resolution,
    Connectivity,
    Unknown,
}

fn classify_ssh_failure(stderr: &str) -> SshFailureKind {
    if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        SshFailureKind::HostKeyChanged
    } else if stderr.contains("Cannot execute command-line and remote command") {
        SshFailureKind::RemoteCommandConflict
    } else if stderr.contains("Permission denied")
        || stderr.contains("Too many authentication failures")
    {
        SshFailureKind::Authentication
    } else if stderr.contains("Could not resolve hostname") {
        SshFailureKind::Resolution
    } else if stderr.contains("Connection timed out")
        || stderr.contains("Connection refused")
        || stderr.contains("No route to host")
    {
        SshFailureKind::Connectivity
    } else {
        SshFailureKind::Unknown
    }
}

fn describe_ssh_failure(host: &str, action: &str, stderr: &str) -> String {
    let detail = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("SSH exited without an error message");
    match classify_ssh_failure(stderr) {
        SshFailureKind::HostKeyChanged => format!(
            "SSH host key for {host} changed. Verify the server identity and repair known_hosts before retrying; Choruz will not bypass this security check."
        ),
        SshFailureKind::RemoteCommandConflict => format!(
            "SSH config for {host} defines RemoteCommand and it conflicted with the Choruz command ({action})."
        ),
        SshFailureKind::Authentication => {
            format!("SSH authentication failed for {host} while {action}: {detail}")
        }
        SshFailureKind::Resolution => {
            format!("SSH could not resolve {host} while {action}: {detail}")
        }
        SshFailureKind::Connectivity => {
            format!("SSH could not reach {host} while {action}: {detail}")
        }
        SshFailureKind::Unknown => format!("SSH failed for {host} while {action}: {detail}"),
    }
}

fn is_known_ssh_failure(stderr: &str) -> bool {
    classify_ssh_failure(stderr) != SshFailureKind::Unknown
}

/// GET /v1/ssh/tunnels — list tracked sessions and refresh their status from
/// the owned child handles before returning them.
pub(crate) async fn list_tunnels(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<Vec<TunnelInfo>>, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    let mut tunnels = TUNNELS.lock().await;
    for entry in tunnels.entries.values_mut() {
        entry.refresh_status();
    }
    let list = tunnels
        .entries
        .values()
        .map(|entry| entry.info.clone())
        .collect();
    Ok(Json(list))
}

/// DELETE /v1/ssh/tunnel/{id} — kill the child ssh process for a tunnel.
pub(crate) async fn delete_tunnel(
    headers: HeaderMap,
    Path(id): Path<String>,
    State(state): State<ApiState>,
) -> Result<StatusCode, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    let mut tunnels = TUNNELS.lock().await;
    let mut entry = tunnels
        .entries
        .remove(&id)
        .ok_or_else(|| ApiError::from(AppError::NotFound(format!("tunnel {id} not found"))))?;
    drop(tunnels);

    stop_tunnel_entry(&mut entry).await;

    Ok(StatusCode::NO_CONTENT)
}

async fn stop_tunnel_entry(entry: &mut TunnelEntry) {
    // Stop the forward first so the local port closes immediately. Then stop
    // the runner so its remote choruz-server receives the SSH session teardown.
    let _ = entry.child.start_kill();
    let _ = entry.child.wait().await;
    if let Some(mut runner) = entry.runner_child.take() {
        let _ = runner.start_kill();
        let _ = runner.wait().await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tunnel_info(host: &str, generation: Option<u64>) -> TunnelInfo {
        TunnelInfo {
            id: "tunnel-1".into(),
            host: host.into(),
            local_port: 41001,
            remote_port: 3000,
            pid: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            generation,
            status: TunnelStatus::Ready,
            disconnected_at: None,
            last_error: None,
        }
    }

    #[test]
    fn newer_generation_fences_older_connection_attempts() {
        let mut registry = TunnelRegistry::default();
        let first = registry
            .reserve_generation("alpha")
            .expect("reserve first generation");
        let second = registry
            .reserve_generation("alpha")
            .expect("reserve second generation");

        assert_eq!((first, second), (1, 2));
        assert!(!registry.is_current("alpha", first));
        assert!(registry.is_current("alpha", second));
        assert_eq!(
            registry
                .reserve_generation("beta")
                .expect("generations are per host"),
            1
        );
    }

    #[test]
    fn managed_replacement_preserves_manual_tunnels() {
        assert!(managed_session_matches(
            &tunnel_info("alpha", Some(1)),
            "alpha"
        ));
        assert!(!managed_session_matches(
            &tunnel_info("alpha", None),
            "alpha"
        ));
        assert!(!managed_session_matches(
            &tunnel_info("beta", Some(1)),
            "alpha"
        ));
    }

    #[tokio::test]
    async fn exited_child_is_reported_as_disconnected() {
        let child = Command::new("sh")
            .arg("-c")
            .arg("exit 7")
            .spawn()
            .expect("spawn short-lived child");
        let mut entry = TunnelEntry {
            info: TunnelInfo {
                pid: child.id(),
                ..tunnel_info("alpha", Some(1))
            },
            child,
            runner_child: None,
        };

        for _ in 0..20 {
            entry.refresh_status();
            if entry.info.status == TunnelStatus::Disconnected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert_eq!(entry.info.status, TunnelStatus::Disconnected);
        assert!(entry.info.disconnected_at.is_some());
        assert!(
            entry
                .info
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("exit status: 7"))
        );
    }

    #[test]
    fn parse_effective_config_uses_first_identity() {
        let host = parse_effective_ssh_config(
            "myserver".to_string(),
            "hostname 192.168.1.100\nuser alice\nport 2222\nidentityfile ~/.ssh/id_ed25519\nidentityfile ~/.ssh/id_rsa\n",
        );
        assert_eq!(host.name, "myserver");
        assert_eq!(host.hostname.as_deref(), Some("192.168.1.100"));
        assert_eq!(host.user.as_deref(), Some("alice"));
        assert_eq!(host.port, Some(2222));
        assert_eq!(host.identity_file.as_deref(), Some("~/.ssh/id_ed25519"));
    }

    #[test]
    fn inline_comments_preserve_quoted_hashes() {
        assert_eq!(
            strip_inline_comment("HostName host # comment").trim(),
            "HostName host"
        );
        assert_eq!(
            strip_inline_comment("ProxyCommand sh -c \"echo #value\"").trim(),
            "ProxyCommand sh -c \"echo #value\""
        );
    }

    #[test]
    fn directives_accept_openssh_equals_syntax() {
        assert_eq!(
            split_directive("Host=alpha beta"),
            Some(("Host", "alpha beta"))
        );
        assert_eq!(
            split_directive("Include = conf.d/*.conf"),
            Some(("Include", "conf.d/*.conf"))
        );
        assert_eq!(
            split_ssh_words("alpha \"my host\" 'path with spaces' escaped\\ value"),
            vec!["alpha", "my host", "path with spaces", "escaped value"]
        );
    }

    #[test]
    fn collect_aliases_expands_multi_host_nested_includes_and_deduplicates() {
        let home = std::env::temp_dir().join(format!("choruz-ssh-{}", choruz_common::new_id()));
        let ssh_dir = home.join(".ssh");
        let nested_dir = ssh_dir.join("conf.d/nested");
        std::fs::create_dir_all(&nested_dir).expect("create nested ssh config dir");
        std::fs::write(
            ssh_dir.join("config"),
            "Include [\nInclude conf.d/*.conf\nHost alpha beta *.wild !blocked # aliases\n",
        )
        .expect("write root config");
        std::fs::write(
            ssh_dir.join("conf.d/one.conf"),
            "Include conf.d/nested/*.conf\nHost gamma alpha\n",
        )
        .expect("write included config");
        std::fs::write(
            nested_dir.join("two.conf"),
            "Include config\nHost delta? epsilon\n",
        )
        .expect("write nested config");

        let aliases = collect_ssh_aliases(&ssh_dir.join("config")).expect("collect aliases");
        assert_eq!(aliases, vec!["epsilon", "gamma", "alpha", "beta"]);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn remote_binary_rejects_shell_metacharacters() {
        assert!(is_safe_remote_binary("choruz-server"));
        assert!(is_safe_remote_binary("/opt/choruz/bin/choruz-server"));
        assert!(!is_safe_remote_binary("choruz-server; id"));
        assert!(!is_safe_remote_binary("$(id)"));
        assert!(!is_safe_remote_binary("../choruz-server"));
    }

    #[test]
    fn ssh_failures_preserve_host_key_security_and_classify_auth() {
        let changed = describe_ssh_failure(
            "prod",
            "connecting",
            "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!",
        );
        assert!(changed.contains("will not bypass this security check"));

        let auth =
            describe_ssh_failure("prod", "connecting", "prod: Permission denied (publickey).");
        assert!(auth.contains("authentication failed"));
        assert!(is_known_ssh_failure("Permission denied (publickey)."));
        assert_eq!(
            classify_ssh_failure("Could not resolve hostname prod"),
            SshFailureKind::Resolution
        );
        assert!(!is_known_ssh_failure(
            "warning: server uses an unrecognized post-quantum key exchange"
        ));
    }

    #[test]
    fn unreadable_or_invalid_includes_do_not_hide_other_aliases() {
        let home = std::env::temp_dir().join(format!("choruz-ssh-{}", choruz_common::new_id()));
        let ssh_dir = home.join(".ssh");
        std::fs::create_dir_all(&ssh_dir).expect("create ssh dir");
        std::fs::write(
            ssh_dir.join("config"),
            "Include invalid-utf8.conf\nHost still-visible\n",
        )
        .expect("write root config");
        std::fs::write(ssh_dir.join("invalid-utf8.conf"), [0xff, 0xfe])
            .expect("write invalid include");

        let aliases = collect_ssh_aliases(&ssh_dir.join("config")).expect("collect aliases");
        assert_eq!(aliases, vec!["still-visible"]);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn read_ssh_aliases_treats_missing_home_as_empty() {
        let aliases = read_ssh_aliases("").await.expect("missing HOME is allowed");
        assert!(aliases.is_empty());
    }

    #[tokio::test]
    async fn read_ssh_aliases_treats_missing_file_as_empty() {
        let home =
            std::env::temp_dir().join(format!("choruz-missing-ssh-{}", choruz_common::new_id()));
        let aliases = read_ssh_aliases(&home.display().to_string())
            .await
            .expect("missing ssh config is allowed");
        assert!(aliases.is_empty());
    }

    #[tokio::test]
    async fn read_ssh_aliases_reads_existing_file() {
        let home = std::env::temp_dir().join(format!("choruz-ssh-{}", choruz_common::new_id()));
        let ssh_dir = home.join(".ssh");
        tokio::fs::create_dir_all(&ssh_dir)
            .await
            .expect("create ssh dir");
        let config_path = ssh_dir.join("config");
        tokio::fs::write(&config_path, "Host ci-box\n  HostName 127.0.0.1\n")
            .await
            .expect("write ssh config");

        let aliases = read_ssh_aliases(&home.display().to_string())
            .await
            .expect("read ssh config");
        assert_eq!(aliases, vec!["ci-box"]);

        let _ = tokio::fs::remove_dir_all(&home).await;
    }
}
