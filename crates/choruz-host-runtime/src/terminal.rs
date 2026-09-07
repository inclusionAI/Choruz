//! Interactive Harness terminals: one PTY per terminal id, pooled for the
//! life of the child process, with startup bytes for the first attachment
//! and a bounded terminal screen snapshot for subsequent attachments.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::Instant,
};

use choruz_agent_runtime::{
    DriverType,
    headless::{CLAUDE_PARENT_SESSION_ENV, HeadlessDriver, harness_account_env},
};
use choruz_common::AppError;
use serde::{Deserialize, Serialize};

use crate::process::ProcessContainer;

/// Everything a device needs to spawn one interactive Harness. The gateway
/// builds it from the binding; the connector receives it over the host link.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSpec {
    /// Pool key; the gateway uses the binding id.
    pub terminal_id: String,
    pub driver_type: String,
    /// Explicit per-agent executable on the target device; None uses its configuration.
    pub binary_path: Option<String>,
    pub workspace_path: String,
    pub cols: u16,
    pub rows: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_session_id: Option<String>,
    /// Managed `CODEX_HOME` for a Codex terminal, prepared on the same device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The binding's `harness_account_id` / `harness_account_profile_kind`
    /// keys; the profile directory resolves on the device that runs the CLI.
    #[serde(default)]
    pub harness_account: serde_json::Value,
}

pub struct TerminalSession {
    /// The workspace the Harness runs in; a remote device ships this
    /// workspace's outbox while the terminal lives.
    pub workspace_path: PathBuf,
    pub writer: Arc<StdMutex<Box<dyn std::io::Write + Send>>>,
    pub output_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
    replay: Arc<StdMutex<TerminalReplay>>,
    pub child: StdMutex<Box<dyn portable_pty::Child + Send + Sync>>,
    pub master: Arc<StdMutex<Box<dyn portable_pty::MasterPty + Send>>>,
    pub last_accessed: StdMutex<Instant>,
    /// Kills the whole child process tree, not just the direct child, when
    /// the last `Arc<TerminalSession>` drops.
    _container: ProcessContainer,
}

impl TerminalSession {
    pub fn touch(&self) {
        *self.last_accessed.lock().expect("last_accessed lock") = Instant::now();
    }

    pub fn is_child_alive(&self) -> bool {
        let mut child = self.child.lock().expect("child lock");
        matches!(child.try_wait(), Ok(None))
    }

    pub fn write_all(&self, data: &[u8]) -> Result<(), AppError> {
        let mut writer = self.writer.lock().expect("writer lock");
        writer
            .write_all(data)
            .and_then(|()| writer.flush())
            .map_err(|error| AppError::Internal(format!("pty write: {error}")))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), AppError> {
        validate_terminal_size(cols, rows)?;
        self.master
            .lock()
            .expect("master lock")
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| AppError::Internal(format!("pty resize: {error}")))?;
        self.replay
            .lock()
            .expect("terminal replay lock")
            .screen
            .screen_mut()
            .set_size(rows, cols);
        Ok(())
    }

    /// Capture the screen and subscribe under the reader's lock: bytes belong
    /// either to the replay or the live stream, never both and never neither.
    pub fn subscribe_with_replay(
        &self,
    ) -> (Vec<Vec<u8>>, tokio::sync::broadcast::Receiver<Vec<u8>>) {
        let mut replay = self.replay.lock().expect("terminal replay lock");
        let output = self.output_tx.subscribe();
        (replay.snapshot(), output)
    }
}

struct TerminalReplay {
    startup: Option<Vec<Vec<u8>>>,
    startup_bytes: usize,
    screen: vt100::Parser,
}

fn validate_terminal_size(cols: u16, rows: u16) -> Result<(), AppError> {
    if !(1..=512).contains(&cols) || !(1..=256).contains(&rows) {
        return Err(AppError::Validation(
            "terminal size must be 1–512 columns and 1–256 rows".into(),
        ));
    }
    Ok(())
}

impl TerminalReplay {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            startup: Some(Vec::new()),
            startup_bytes: 0,
            screen: vt100::Parser::new(rows, cols, 0),
        }
    }

    fn process(&mut self, bytes: &[u8]) {
        self.screen.process(bytes);
        if let Some(frames) = self.startup.as_mut() {
            self.startup_bytes += bytes.len();
            if self.startup_bytes <= 64 * 1024 {
                frames.push(bytes.to_vec());
            } else {
                self.startup = None;
            }
        }
    }

    fn snapshot(&mut self) -> Vec<Vec<u8>> {
        self.startup.take().unwrap_or_else(|| {
            let screen = self.screen.screen();
            let mut snapshot = if screen.alternate_screen() {
                b"\x1b[?1049h".to_vec()
            } else {
                b"\x1b[?1049l".to_vec()
            };
            snapshot.extend(screen.state_formatted());
            vec![snapshot]
        })
    }
}

pub type TerminalPool = Arc<StdMutex<HashMap<String, Arc<TerminalSession>>>>;

pub fn new_terminal_pool() -> TerminalPool {
    Arc::new(StdMutex::new(HashMap::new()))
}

pub struct EnsureOutcome {
    pub session: Arc<TerminalSession>,
    pub newly_created: bool,
}

/// Drop the sessions whose child has exited. Sessions are never evicted for
/// being idle: a terminal lives until its process ends or a caller removes it.
pub fn evict_stale_terminals(pool: &TerminalPool) {
    let mut sessions = pool.lock().expect("terminal pool lock");
    let before = sessions.len();
    sessions.retain(|id, session| {
        if !session.is_child_alive() {
            tracing::info!(terminal_id = %id, "evicting terminal with dead child process");
            false
        } else {
            true
        }
    });
    let evicted = before - sessions.len();
    if evicted > 0 {
        tracing::info!(
            evicted,
            remaining = sessions.len(),
            "terminal eviction complete"
        );
    }
}

pub fn live_terminal_exists(pool: &TerminalPool, terminal_id: &str) -> bool {
    let sessions = pool.lock().expect("terminal pool lock");
    sessions
        .get(terminal_id)
        .is_some_and(|session| session.is_child_alive())
}

/// Stop the child before releasing its pool claim, including when an attached
/// stream still holds another reference to the session.
pub fn close_terminal(pool: &TerminalPool, terminal_id: &str) -> Result<(), AppError> {
    let mut sessions = pool.lock().expect("terminal pool lock");
    crate::session::close(terminal_id)?;
    if let Some(session) = sessions.get(terminal_id) {
        session._container.kill_all();
        match session.child.lock().expect("child lock").try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(AppError::Conflict(
                    "terminal process has not stopped".into(),
                ));
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "confirm terminal exit: {error}"
                )));
            }
        }
    }
    sessions.remove(terminal_id);
    Ok(())
}

pub fn is_terminal_driver(driver_type: &DriverType) -> bool {
    matches!(
        driver_type,
        DriverType::ClaudeTerminal
            | DriverType::CodexTerminal
            | DriverType::PiTerminal
            | DriverType::GrokTerminal
            | DriverType::OpenCodeTerminal
            | DriverType::MathCodeTerminal
    )
}

pub fn default_terminal_binary(driver_type: &DriverType) -> &'static str {
    match driver_type {
        DriverType::ClaudeTerminal => "claude",
        DriverType::CodexTerminal => "codex",
        DriverType::PiTerminal => "pi",
        DriverType::GrokTerminal => "grok",
        DriverType::OpenCodeTerminal => "opencode",
        DriverType::MathCodeTerminal => "mathcode",
        _ => "claude",
    }
}

/// The executable for a terminal. `binary_path: "codex"` is the portable
/// default persisted by older bindings; an explicitly configured Harness
/// executable (`CHORUZ_<HARNESS>_BINARY`, then supported `*_CLI_PATH`, on this device) beats that bare
/// default because PATH can otherwise select a stale CLI. Absolute or custom
/// per-agent paths remain authoritative.
pub fn terminal_binary(driver_type: &DriverType, configured: Option<&str>) -> String {
    terminal_binary_with_env(driver_type, configured, |key| std::env::var(key).ok())
}

fn terminal_binary_with_env(
    driver_type: &DriverType,
    configured: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> String {
    let configured = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_terminal_binary(driver_type));
    let environment_keys: &[&str] = match driver_type {
        DriverType::ClaudeTerminal => &["CHORUZ_CLAUDE_BINARY", "CHORUZ_CLAUDE_CLI_PATH"],
        DriverType::CodexTerminal => &["CHORUZ_CODEX_BINARY", "CHORUZ_CODEX_CLI_PATH"],
        DriverType::PiTerminal => &["CHORUZ_PI_BINARY", "CHORUZ_PI_CLI_PATH"],
        DriverType::GrokTerminal => &["CHORUZ_GROK_BINARY", "CHORUZ_GROK_CLI_PATH"],
        DriverType::OpenCodeTerminal => &["CHORUZ_OPENCODE_BINARY", "CHORUZ_OPENCODE_CLI_PATH"],
        DriverType::MathCodeTerminal => &["CHORUZ_MATHCODE_BINARY"],
        _ => &[],
    };
    if configured == default_terminal_binary(driver_type)
        && let Some(path) = environment_keys.iter().find_map(|key| {
            env(key)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
    {
        return path;
    }
    configured.to_string()
}

pub fn codex_terminal_args(resume_session_id: Option<&str>, model: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(sid) = resume_session_id
        && !sid.is_empty()
    {
        args.push("resume".into());
        args.push(sid.into());
        args.push("--all".into());
    }
    args.extend([
        "--sandbox".into(),
        "workspace-write".into(),
        "--ask-for-approval".into(),
        "on-request".into(),
        // Suppress the blocking "Update now / Skip" startup prompt — a PTY
        // agent can never answer it and the terminal hangs until a human
        // presses enter. (There is no env var for this; verified against
        // codex 0.138.)
        "--config".into(),
        "check_for_update_on_startup=false".into(),
    ]);
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    args
}

pub fn terminal_cli_args(
    driver_type: &DriverType,
    resume_session_id: Option<&str>,
    model: Option<&str>,
) -> Vec<String> {
    let resume_session_id = resume_session_id.filter(|session_id| !session_id.is_empty());
    match driver_type {
        DriverType::ClaudeTerminal => {
            let mut args = vec!["--dangerously-skip-permissions".into()];
            if let Some(session_id) = resume_session_id {
                args.extend(["--resume".into(), session_id.into()]);
            }
            if let Some(model) = model {
                args.extend(["--model".into(), model.into()]);
            }
            args
        }
        DriverType::CodexTerminal => codex_terminal_args(resume_session_id, model),
        DriverType::PiTerminal => {
            let mut args = vec!["--approve".into()];
            if let Some(session_id) = resume_session_id {
                args.extend(["--session".into(), session_id.into()]);
            }
            if let Some(model) = model {
                args.extend(["--model".into(), model.into()]);
            }
            args
        }
        DriverType::GrokTerminal => {
            let mut args = vec!["--no-auto-update".into(), "--always-approve".into()];
            if let Some(session_id) = resume_session_id {
                args.extend(["--resume".into(), session_id.into()]);
            }
            if let Some(model) = model {
                args.extend(["--model".into(), model.into()]);
            }
            args
        }
        DriverType::OpenCodeTerminal => {
            let mut args = vec!["--auto".into()];
            if let Some(session_id) = resume_session_id {
                args.extend(["--session".into(), session_id.into()]);
            }
            if let Some(model) = model {
                args.extend(["--model".into(), model.into()]);
            }
            args
        }
        DriverType::MathCodeTerminal => Vec::new(),
        _ => Vec::new(),
    }
}

/// Reuse the live terminal for `spec.terminal_id` or spawn the Harness in a
/// fresh PTY. Interactive PTYs use the same absolute workspace-bound command
/// surface as headless executions.
pub fn ensure_terminal(
    pool: &TerminalPool,
    spec: &TerminalSpec,
) -> Result<EnsureOutcome, AppError> {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    validate_terminal_size(spec.cols, spec.rows)?;
    let driver_type = spec
        .driver_type
        .parse::<DriverType>()
        .map_err(|_| AppError::Validation(format!("unknown driver type {}", spec.driver_type)))?;
    let terminal_id = spec.terminal_id.as_str();

    evict_stale_terminals(pool);

    let mut sessions = pool.lock().expect("terminal pool lock");
    if crate::session::active(&spec.terminal_id) {
        return Err(AppError::Conflict(
            "Close the structured conversation before opening Terminal".into(),
        ));
    }
    if let Some(session) = sessions.get(terminal_id) {
        if session.is_child_alive() {
            session.touch();
            return Ok(EnsureOutcome {
                session: Arc::clone(session),
                newly_created: false,
            });
        }
        tracing::warn!(terminal_id, "PTY child process exited, recreating session");
        sessions.remove(terminal_id);
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: spec.rows,
            cols: spec.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| AppError::Internal(format!("pty open: {e}")))?;

    let is_codex = driver_type == DriverType::CodexTerminal;
    let binary = terminal_binary(&driver_type, spec.binary_path.as_deref());
    let mut cmd = CommandBuilder::new(binary);
    cmd.cwd(&spec.workspace_path);
    if driver_type == DriverType::ClaudeTerminal {
        for key in CLAUDE_PARENT_SESSION_ENV {
            cmd.env_remove(key);
        }
    }
    // A capable terminal type prevents modern harnesses from pausing on a
    // TERM=dumb confirmation prompt.
    cmd.env("TERM", "xterm-256color");
    cmd.env(
        "CHORUZ_SEND",
        PathBuf::from(&spec.workspace_path)
            .join(".choruz")
            .join("send"),
    );
    // Each CLI has its own auto-update switch: Claude Code reads
    // DISABLE_AUTOUPDATER, Codex takes the `--config` flag appended in
    // `codex_terminal_args`, Pi reads PI_SKIP_VERSION_CHECK.
    cmd.env("DISABLE_AUTOUPDATER", "1");
    cmd.env("PI_SKIP_VERSION_CHECK", "1");
    if let Some(codex_home) = spec.codex_home.as_deref()
        && is_codex
    {
        cmd.env("CODEX_HOME", codex_home);
    }
    if !is_codex
        && let Some(driver) = HeadlessDriver::from_driver_type(driver_type.as_str())
        && let Some((key, value)) =
            harness_account_env(driver, &spec.harness_account).map_err(AppError::Validation)?
    {
        cmd.env(key, value);
    }

    let resume_session_id = spec
        .resume_session_id
        .as_deref()
        .filter(|session_id| !session_id.is_empty());
    if let Some(session_id) = resume_session_id {
        tracing::info!(
            terminal_id,
            session_id,
            driver_type = driver_type.as_str(),
            "resuming CLI session in PTY"
        );
    }
    let model = spec
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut args = terminal_cli_args(&driver_type, resume_session_id, model);
    if driver_type == DriverType::ClaudeTerminal
        && let Some(id) = resume_session_id
    {
        let anchor = &spec.harness_account["terminal_session"];
        if anchor["session_id"] == id {
            let root = crate::session_history::claude_account_root(spec)?;
            if anchor["native_home_path"].as_str() != Some(root.to_string_lossy().as_ref()) {
                return Err(AppError::Conflict(
                    "The direct session belongs to a different Claude account".into(),
                ));
            }
            match crate::session_history::claude_history(spec, id) {
                Ok(_) => {}
                Err(AppError::NotFound(_)) if anchor["provenance"] == "direct_session_reserved" => {
                    args = terminal_cli_args(&driver_type, None, model);
                    args.extend(["--session-id".into(), id.into()]);
                }
                Err(error) => return Err(error),
            }
        } else if spec.harness_account["external_session_mode"] == "headless" {
            args.push("--fork-session".into());
        }
    }
    for arg in args {
        cmd.arg(arg);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| AppError::Internal(format!("pty spawn: {e}")))?;
    drop(pair.slave);

    // portable-pty calls setsid() before exec, so child_pid == session leader
    // == initial PGID, which is what the container kills on cleanup.
    let child_pid = child.process_id().ok_or_else(|| {
        AppError::Internal("spawned child has no PID — cannot create process container".into())
    })?;
    let container = ProcessContainer::new(format!("pty-{terminal_id}"), child_pid);
    tracing::info!(
        terminal_id,
        child_pid,
        "PTY child spawned and placed in process container"
    );

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::Internal(format!("pty reader: {e}")))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::Internal(format!("pty writer: {e}")))?;

    let (output_tx, _) = tokio::sync::broadcast::channel(4096);
    let replay = Arc::new(StdMutex::new(TerminalReplay::new(spec.rows, spec.cols)));
    let tx_clone = output_tx.clone();
    let replay_clone = Arc::clone(&replay);

    // Keep draining the PTY even without subscribers so the child never
    // blocks on a full buffer. A screen snapshot stays available after detach.
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut reader = reader;
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut replay = replay_clone.lock().expect("terminal replay lock");
                    replay.process(&buf[..n]);
                    let _ = tx_clone.send(buf[..n].to_vec());
                }
                Err(_) => break,
            }
        }
    });

    let session = Arc::new(TerminalSession {
        workspace_path: PathBuf::from(&spec.workspace_path),
        writer: Arc::new(StdMutex::new(writer)),
        output_tx,
        replay,
        child: StdMutex::new(child),
        master: Arc::new(StdMutex::new(pair.master)),
        last_accessed: StdMutex::new(Instant::now()),
        _container: container,
    });

    sessions.insert(terminal_id.to_string(), Arc::clone(&session));
    Ok(EnsureOutcome {
        session,
        newly_created: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn terminal_size_rejects_zero_and_unbounded_screen_allocations() {
        for (cols, rows) in [(0, 24), (80, 0), (513, 24), (80, 257), (u16::MAX, u16::MAX)] {
            assert!(matches!(
                validate_terminal_size(cols, rows),
                Err(AppError::Validation(_))
            ));
        }
        assert!(validate_terminal_size(80, 24).is_ok());
        assert!(validate_terminal_size(512, 256).is_ok());
    }

    #[test]
    fn reconnect_replays_current_screen_after_startup_is_consumed() {
        let mut replay = TerminalReplay::new(24, 80);
        replay.process(b"old output");
        assert_eq!(replay.snapshot(), vec![b"old output".to_vec()]);
        replay.process(b"\x1b[2J\x1b[Hcurrent output\r\nready");
        for _ in 0..2 {
            let mut restored = vt100::Parser::new(24, 80, 0);
            for frame in replay.snapshot() {
                restored.process(&frame);
            }
            assert_eq!(restored.screen().contents(), "current output\nready");
            assert_eq!(restored.screen().cursor_position(), (1, 5));
        }
    }

    #[test]
    fn startup_overflow_keeps_a_bounded_current_screen() {
        let mut replay = TerminalReplay::new(24, 80);
        replay.process(&vec![b'x'; 128 * 1024]);
        replay.process(b"\x1b[2J\x1b[Hstill available");
        assert!(replay.startup.is_none());
        let snapshot = replay.snapshot().concat();
        assert!(snapshot.len() < 64 * 1024);
        let mut restored = vt100::Parser::new(24, 80, 0);
        restored.process(&snapshot);
        assert_eq!(restored.screen().contents(), "still available");
    }

    #[test]
    fn reconnect_preserves_alternate_screen_and_input_modes() {
        let mut replay = TerminalReplay::new(24, 80);
        replay.snapshot();
        replay.process(b"\x1b[?1049h\x1b[?2004h\x1b[?1h\x1b[?25lready");
        let mut restored = vt100::Parser::new(24, 80, 0);
        restored.process(&replay.snapshot().concat());
        assert!(restored.screen().alternate_screen());
        assert!(restored.screen().bracketed_paste());
        assert!(restored.screen().application_cursor());
        assert!(restored.screen().hide_cursor());
        assert_eq!(restored.screen().contents(), "ready");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn close_stops_a_terminal_held_by_an_attached_stream() {
        let temp = tempfile::tempdir().unwrap();
        let pool = new_terminal_pool();
        let attached = ensure_terminal(
            &pool,
            &TerminalSpec {
                terminal_id: "owned-close-test".into(),
                driver_type: "mathcode_terminal".into(),
                binary_path: Some("/bin/cat".into()),
                workspace_path: temp.path().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                resume_session_id: None,
                codex_home: None,
                model: None,
                harness_account: serde_json::json!({}),
            },
        )
        .unwrap()
        .session;
        assert!(attached.is_child_alive());
        close_terminal(&pool, "owned-close-test").unwrap();
        assert!(
            !attached.is_child_alive(),
            "removing the pool entry alone does not stop an attached child"
        );
        assert!(pool.lock().unwrap().is_empty());
        close_terminal(&pool, "owned-close-test").unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_cli_pty_round_trips_input_output_and_exit_code() {
        use std::os::unix::fs::PermissionsExt;

        let _environment = crate::TEST_ENV_LOCK.lock().await;

        let temp = tempfile::tempdir().expect("create fake PTY dir");
        let dir = temp.path();
        let codex_home = dir.join("managed-codex-home");
        fs::create_dir_all(&codex_home).expect("create managed Codex home");
        let cli = dir.join("fake-cli.sh");
        fs::write(
            &cli,
            "#!/bin/sh\nprintf 'READY\\r\\n'\nprintf 'ARGS:%s\\r\\n' \"$*\"\nprintf 'CODEX_HOME:%s\\r\\n' \"${CODEX_HOME:-}\"\nprintf 'TERM:%s\\r\\n' \"${TERM:-}\"\nprintf 'CHORUZ_SEND:%s\\r\\n' \"${CHORUZ_SEND:-}\"\nIFS= read -r line\nprintf 'ECHO:%s\\r\\n' \"$line\"\nexit 23\n",
        )
        .expect("write fake PTY cli");
        let mut permissions = fs::metadata(&cli).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&cli, permissions).unwrap();

        let original_codex_home = std::env::var_os("CODEX_HOME");
        let cases = [
            (
                "claude",
                DriverType::ClaudeTerminal,
                "claude-session",
                None,
                "ARGS:--dangerously-skip-permissions --resume claude-session",
            ),
            (
                "codex",
                DriverType::CodexTerminal,
                "codex-session",
                Some(codex_home.to_str().unwrap()),
                "ARGS:resume codex-session --all --sandbox workspace-write --ask-for-approval on-request --config check_for_update_on_startup=false",
            ),
            (
                "pi",
                DriverType::PiTerminal,
                "pi-session",
                None,
                "ARGS:--approve --session pi-session",
            ),
            (
                "grok",
                DriverType::GrokTerminal,
                "grok-session",
                None,
                "ARGS:--no-auto-update --always-approve --resume grok-session",
            ),
            (
                "opencode",
                DriverType::OpenCodeTerminal,
                "opencode-session",
                None,
                "ARGS:--auto --session opencode-session",
            ),
            (
                "mathcode",
                DriverType::MathCodeTerminal,
                "mathcode-session",
                None,
                "ARGS:",
            ),
        ];

        for (label, driver, resume_session_id, codex_home_env, expected_args) in cases {
            let pool = new_terminal_pool();
            let ensured = ensure_terminal(
                &pool,
                &TerminalSpec {
                    terminal_id: format!("binding-fake-pty-{label}"),
                    driver_type: driver.as_str().to_string(),
                    binary_path: Some(cli.to_str().unwrap().to_string()),
                    workspace_path: dir.to_str().unwrap().to_string(),
                    cols: 100,
                    rows: 30,
                    resume_session_id: Some(resume_session_id.to_string()),
                    codex_home: codex_home_env.map(str::to_string),
                    model: None,
                    harness_account: serde_json::json!({}),
                },
            )
            .expect("spawn fake CLI in PTY");
            assert!(ensured.newly_created);

            let (replay, mut output) = ensured.session.subscribe_with_replay();
            ensured
                .session
                .write_all(b"hello from test\n")
                .expect("write to PTY");

            let mut transcript = replay.into_iter().flatten().collect::<Vec<_>>();
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while !String::from_utf8_lossy(&transcript).contains("ECHO:hello from test") {
                    transcript.extend(output.recv().await.expect("PTY output frame"));
                }
            })
            .await
            .expect("fake CLI should echo PTY input");
            let (replay, _) = ensured.session.subscribe_with_replay();
            let mut restored = vt100::Parser::new(30, 100, 0);
            restored.process(&replay.concat());
            assert!(
                restored
                    .screen()
                    .contents()
                    .contains("ECHO:hello from test")
            );
            let transcript = String::from_utf8_lossy(&transcript);
            assert!(
                transcript.contains("READY"),
                "{label} startup output must reach the PTY client"
            );
            assert!(
                transcript.contains(expected_args),
                "{label} must receive its driver-specific session arguments: {transcript}"
            );
            assert!(
                transcript.contains("TERM:xterm-256color"),
                "{label} must receive a capable terminal type: {transcript}"
            );
            assert!(
                transcript.contains(&format!("CHORUZ_SEND:{}/.choruz/send", dir.display())),
                "{label} must receive its absolute Choruz helper binding: {transcript}"
            );
            if driver == DriverType::CodexTerminal {
                assert!(
                    transcript.contains(&format!("CODEX_HOME:{}", codex_home.display())),
                    "Codex child must receive its managed home: {transcript}"
                );
            }

            let exit_code = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    let status = ensured
                        .session
                        .child
                        .lock()
                        .expect("PTY child lock")
                        .try_wait()
                        .expect("query fake CLI status");
                    if let Some(status) = status {
                        break status.exit_code();
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("fake CLI should exit");
            assert_eq!(exit_code, 23);
            assert_eq!(std::env::var_os("CODEX_HOME"), original_codex_home);
            assert!(!live_terminal_exists(
                &pool,
                &format!("binding-fake-pty-{label}")
            ));

            pool.lock().unwrap().clear();
        }
    }

    #[test]
    fn codex_terminal_args_use_current_cli_flags() {
        let args = codex_terminal_args(Some("session-123"), None);
        assert_eq!(
            args,
            vec![
                "resume",
                "session-123",
                "--all",
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "--config",
                "check_for_update_on_startup=false",
            ]
        );
    }

    #[test]
    fn codex_terminal_args_start_new_session_without_resume() {
        assert_eq!(
            codex_terminal_args(None, None),
            vec![
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "--config",
                "check_for_update_on_startup=false",
            ]
        );
    }

    #[test]
    fn additional_terminal_drivers_use_documented_flags_and_exact_resume_ids() {
        assert_eq!(
            terminal_cli_args(&DriverType::PiTerminal, Some("pi-1"), None),
            vec!["--approve", "--session", "pi-1"]
        );
        assert_eq!(
            terminal_cli_args(&DriverType::GrokTerminal, Some("grok-1"), None),
            vec!["--no-auto-update", "--always-approve", "--resume", "grok-1"]
        );
        assert_eq!(
            terminal_cli_args(&DriverType::OpenCodeTerminal, Some("oc-1"), None),
            vec!["--auto", "--session", "oc-1"]
        );
        assert!(terminal_cli_args(&DriverType::MathCodeTerminal, Some("m-1"), None).is_empty());
        assert_eq!(
            terminal_cli_args(&DriverType::ClaudeTerminal, Some(""), None),
            vec!["--dangerously-skip-permissions"]
        );
    }

    #[test]
    fn selected_model_is_forwarded_to_every_interactive_harness() {
        for driver in [
            DriverType::ClaudeTerminal,
            DriverType::CodexTerminal,
            DriverType::PiTerminal,
            DriverType::GrokTerminal,
            DriverType::OpenCodeTerminal,
        ] {
            let args = terminal_cli_args(&driver, None, Some("model-x"));
            let position = args
                .iter()
                .position(|arg| arg == "--model")
                .unwrap_or_else(|| panic!("{} must accept --model", driver.as_str()));
            assert_eq!(args[position + 1], "model-x");
        }
    }

    #[test]
    fn terminal_driver_defaults_do_not_fall_back_to_claude() {
        assert_eq!(default_terminal_binary(&DriverType::CodexTerminal), "codex");
        assert_eq!(default_terminal_binary(&DriverType::PiTerminal), "pi");
        assert_eq!(default_terminal_binary(&DriverType::GrokTerminal), "grok");
        assert_eq!(
            default_terminal_binary(&DriverType::OpenCodeTerminal),
            "opencode"
        );
        assert_eq!(
            default_terminal_binary(&DriverType::MathCodeTerminal),
            "mathcode"
        );
        assert_eq!(
            terminal_binary(&DriverType::CodexTerminal, Some("/opt/codex/bin/codex")),
            "/opt/codex/bin/codex"
        );
    }

    #[test]
    fn target_executable_configuration_preserves_alias_precedence() {
        for driver in [
            DriverType::ClaudeTerminal,
            DriverType::CodexTerminal,
            DriverType::PiTerminal,
            DriverType::GrokTerminal,
            DriverType::OpenCodeTerminal,
            DriverType::MathCodeTerminal,
        ] {
            assert_eq!(
                terminal_binary_with_env(&driver, None, |key| Some(
                    if key.ends_with("_BINARY") {
                        "  /target/primary  "
                    } else {
                        "/target/alias"
                    }
                    .into()
                )),
                "/target/primary"
            );
            let expected = if driver == DriverType::MathCodeTerminal {
                "mathcode"
            } else {
                "/target/alias"
            };
            assert_eq!(
                terminal_binary_with_env(&driver, None, |key| Some(
                    if key.ends_with("_BINARY") {
                        "  "
                    } else {
                        " /target/alias "
                    }
                    .into()
                )),
                expected
            );
            assert_eq!(
                terminal_binary_with_env(&driver, None, |_| Some("  ".into())),
                default_terminal_binary(&driver)
            );
            assert_eq!(
                terminal_binary_with_env(&driver, Some("/target/explicit"), |_| Some(
                    "/target/automatic".into()
                )),
                "/target/explicit"
            );
        }
    }
}
