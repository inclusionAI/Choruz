//! One structured Harness process per binding, owned by the execution device.

use crate::{ProcessContainer, TerminalPool, TerminalSpec, session_protocol::SessionSnapshot};
use choruz_agent_runtime::{
    DriverType,
    headless::{CLAUDE_PARENT_SESSION_ENV, HeadlessDriver, prepare_harness_account_env},
};
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum SessionRequest {
    Prepare {
        spec: Box<TerminalSpec>,
    },
    Ensure {
        spec: Box<TerminalSpec>,
    },
    Read {
        binding_id: String,
        owner: String,
        #[serde(default)]
        after: u64,
    },
    Command {
        binding_id: String,
        owner: String,
        instance: String,
        command: SessionCommand,
        #[serde(default)]
        experience: Option<(String, String)>,
        #[serde(default)]
        preflight: Option<Box<crate::harness::ExecutionTeam>>,
    },
}

pub async fn execute(
    pool: TerminalPool,
    request: SessionRequest,
) -> Result<SessionSnapshot, AppError> {
    if let SessionRequest::Command {
        binding_id,
        owner,
        instance,
        command: SessionCommand::Send {
            text,
            submission_id,
        },
        experience,
        preflight: Some(role),
    } = &request
    {
        let session = SESSIONS
            .lock()
            .expect("sessions")
            .get(binding_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound("Structured session is not open".into()))?;
        let cancellation = Arc::new(tokio::sync::Notify::new());
        let reservation_id = choruz_common::new_id();
        {
            let mut state = session.state.lock().expect("session state");
            verify_owner(&state, owner)?;
            if text.trim().is_empty()
                || text.len() > 128 * 1024
                || submission_id.is_empty()
                || submission_id.len() > 128
            {
                return Err(AppError::Validation(
                    "A message and a valid submission id are required".into(),
                ));
            }
            if state.instance != *instance {
                return Err(AppError::Conflict(
                    "Wait for the current turn or refresh the session before sending".into(),
                ));
            }
            if session
                .submissions
                .lock()
                .expect("session submissions")
                .contains_key(submission_id)
            {
                drop(state);
                return command_owned(
                    binding_id,
                    Some((owner, instance)),
                    SessionCommand::Send {
                        text: text.clone(),
                        submission_id: submission_id.clone(),
                    },
                    experience.clone(),
                )
                .map(|state| state.page(0));
            }
            if state.status != "ready" {
                return Err(AppError::Conflict(
                    "Wait for the current turn to finish before sending another message".into(),
                ));
            }
            *session.preflight.lock().expect("preflight reservation") =
                Some((reservation_id.clone(), cancellation.clone()));
            state.status = "running".into();
            state.revision += 1;
        }
        let reservation = PreflightReservation {
            session: session.clone(),
            id: reservation_id.clone(),
        };
        let plan = tokio::select! {
            result = crate::harness::prepare(session.spec.clone(), role, text) => result?,
            _ = cancellation.notified() => return Err(AppError::Conflict("Execution review was cancelled; the task was not sent".into())),
        };
        let result = command_prepared(
            binding_id,
            Some((owner, instance)),
            SessionCommand::Send {
                text: text.clone(),
                submission_id: submission_id.clone(),
            },
            experience.clone(),
            Some((reservation_id, plan)),
        );
        drop(reservation);
        return result.map(|state| state.page(0));
    }
    tokio::task::spawn_blocking(move || match request {
        SessionRequest::Prepare { spec } => {
            ensure_inner(&pool, *spec, true).map(|state| state.page(0))
        }
        SessionRequest::Ensure { spec } => ensure(&pool, *spec).map(|state| state.page(0)),
        SessionRequest::Read {
            binding_id,
            owner,
            after,
        } => {
            let state = snapshot(&binding_id)?;
            verify_owner(&state, &owner)?;
            Ok(state.page(after))
        }
        SessionRequest::Command {
            binding_id,
            owner,
            instance,
            command: action,
            experience,
            preflight: _,
        } => command_owned(&binding_id, Some((&owner, &instance)), action, experience)
            .map(|state| state.page(0)),
    })
    .await
    .map_err(|e| internal("session operation", e))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SessionCommand {
    Send {
        text: String,
        submission_id: String,
    },
    Respond {
        request_id: Value,
        allow: bool,
        #[serde(default)]
        answers: HashMap<String, Vec<String>>,
    },
    Interrupt,
    Close,
}

struct Session {
    state: Mutex<SessionSnapshot>,
    writer: Mutex<ChildStdin>,
    child: Mutex<Child>,
    container: Mutex<Option<ProcessContainer>>,
    codex: bool,
    spec: TerminalSpec,
    journal: PathBuf,
    submissions: Mutex<HashMap<String, Submission>>,
    interrupt_pending: AtomicBool,
    preflight: Mutex<Option<(String, Arc<tokio::sync::Notify>)>>,
}

struct PreflightReservation {
    session: Arc<Session>,
    id: String,
}

impl Drop for PreflightReservation {
    fn drop(&mut self) {
        let mut state = self.session.state.lock().expect("session state");
        let mut pending = self
            .session
            .preflight
            .lock()
            .expect("preflight reservation");
        if pending.as_ref().is_some_and(|(id, _)| id == &self.id) {
            pending.take();
            if state.status == "running" {
                state.status = "ready".into();
                state.revision += 1;
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Submission {
    fingerprint: String,
    instance: String,
}

#[derive(Serialize, Deserialize)]
struct SavedSession {
    version: u32,
    owner: Value,
    state: SessionSnapshot,
    #[serde(default)]
    submissions: HashMap<String, Submission>,
}

fn owner(spec: &TerminalSpec) -> Value {
    json!({"binding":spec.terminal_id,"driver":spec.driver_type,"workspace":spec.workspace_path,
        "account":spec.harness_account["harness_account_id"],
        "profile":spec.harness_account["harness_account_profile_kind"],
        "generation":spec.harness_account["terminal_generation"],
        "host":spec.harness_account["runtime_host_id"],
        "company":spec.harness_account["conversation_workspace_id"]})
}

pub fn owner_key(spec: &TerminalSpec) -> String {
    hex::encode(Sha256::digest(owner(spec).to_string().as_bytes()))
}

fn verify_owner(state: &SessionSnapshot, expected: &str) -> Result<(), AppError> {
    if state.owner != expected {
        return Err(AppError::Conflict(
            "The Agent configuration changed. Reopen its conversation.".into(),
        ));
    }
    Ok(())
}

fn internal(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("{context}: {error}"))
}

impl Session {
    fn stop_process(&self) -> Result<(), AppError> {
        // Dropping the container stops the complete process group exactly once.
        drop(self.container.lock().expect("session container").take());
        self.child
            .lock()
            .expect("session child")
            .wait()
            .map_err(|e| internal("wait for Harness exit", e))?;
        Ok(())
    }

    fn write(&self, value: Value) -> Result<(), AppError> {
        let mut data =
            serde_json::to_vec(&value).map_err(|e| internal("encode Harness request", e))?;
        data.push(b'\n');
        let mut writer = self.writer.lock().expect("session writer");
        writer
            .write_all(&data)
            .and_then(|()| writer.flush())
            .map_err(|e| internal("write Harness request", e))
    }

    fn save(&self, state: &SessionSnapshot) -> Result<(), AppError> {
        let bytes = serde_json::to_vec(&SavedSession {
            version: 1,
            owner: owner(&self.spec),
            state: state.clone(),
            submissions: self
                .submissions
                .lock()
                .expect("session submissions")
                .clone(),
        })
        .map_err(|e| internal("encode session history", e))?;
        write_history(&self.journal, &bytes)
    }
}

fn write_history(journal: &std::path::Path, bytes: &[u8]) -> Result<(), AppError> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(AppError::Validation(
            "The conversation preview exceeds its journal limit. Continue in Terminal.".into(),
        ));
    }
    let temporary = journal.with_extension(format!("{}.tmp", choruz_common::new_id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, journal)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| internal("save session history", e))
}

impl Session {
    fn receive(&self, event: Value) -> Result<(), AppError> {
        let mut state = self.state.lock().expect("session state");
        if self.codex {
            if event["id"] == "initialize" {
                if !event["error"].is_null() {
                    return Err(internal("initialize Codex", &event["error"]));
                }
                self.write(json!({"method":"initialized"}))?;
                let mut params = json!({"cwd":self.spec.workspace_path});
                if let Some(model) = &self.spec.model {
                    params["model"] = json!(model);
                }
                let method = if let Some(id) = &state.session_id {
                    params["threadId"] = json!(id);
                    params["excludeTurns"] = json!(true);
                    params["initialTurnsPage"] =
                        json!({"limit":20,"sortDirection":"desc","itemsView":"full"});
                    "thread/resume"
                } else {
                    params["historyMode"] = json!("paginated");
                    "thread/start"
                };
                self.write(json!({"id":"thread","method":method,"params":params}))?;
            } else if event["id"] == "thread" {
                if !event["error"].is_null() {
                    return Err(internal("open Codex thread", &event["error"]));
                }
                let thread = &event["result"]["thread"];
                state.session_id = thread["id"].as_str().map(str::to_owned);
                if state.session_id.is_none() {
                    return Err(internal("open Codex thread", "missing thread id"));
                }
                state.restore_codex_history(&event["result"])?;
                state.status = "ready".into();
                self.save(&state)?;
            } else {
                if !event["error"].is_null() {
                    state.status = "failed".into();
                    state.error = Some(event["error"].to_string());
                }
                state.codex(&event);
                if let Some(turn_id) = &state.turn_id
                    && self.interrupt_pending.swap(false, Ordering::SeqCst)
                {
                    self.write(json!({"id":"interrupt","method":"turn/interrupt","params":{"threadId":state.session_id,"turnId":turn_id}}))?;
                }
            }
        } else {
            if event["type"] == "control_response"
                && event["response"]["request_id"] == "initialize"
            {
                if event["response"]["subtype"] == "error" {
                    return Err(internal("initialize Claude", &event["response"]["error"]));
                }
                state.status = "ready".into();
            }
            state.claude(&event);
            // Claude reports a user interrupt as an error result. It ends the
            // requested turn, not the reusable SDK session.
            if event["type"] == "result" && self.interrupt_pending.swap(false, Ordering::SeqCst) {
                acknowledge_claude_interrupt(&mut state, &event);
            }
        }
        state.revision += 1;
        state.enforce_limits();
        if serde_json::to_vec(&state.requests)
            .map_err(|e| internal("encode pending requests", e))?
            .len()
            > 64 * 1024
        {
            state.requests.clear();
            return Err(AppError::Validation("The Harness requested an interaction too large for Conversation. Use Terminal for this interaction.".into()));
        }
        if (state.status == "ready" || state.status == "failed")
            && (self.codex || !state.items.is_empty())
        {
            self.save(&state)?;
        }
        Ok(())
    }
}

/// Checked while holding the terminal pool lock so the two launch paths cannot race.
pub fn active(id: &str) -> bool {
    SESSIONS
        .lock()
        .expect("sessions")
        .get(id)
        .is_some_and(|session| {
            matches!(
                session.child.lock().expect("session child").try_wait(),
                Ok(None)
            )
        })
}

/// Binding teardown also closes structured sessions, including pending approvals.
pub fn close(id: &str) -> Result<(), AppError> {
    let mut sessions = SESSIONS.lock().expect("sessions");
    if let Some(session) = sessions.get(id) {
        let mut state = session.state.lock().expect("session state");
        state.status = "closed".into();
        state.requests.clear();
        session.stop_process()?;
        session.save(&state)?;
    }
    sessions.remove(id);
    Ok(())
}

/// Stop device-owned structured processes before the host exits.
pub fn shutdown() {
    let ids: Vec<_> = SESSIONS.lock().expect("sessions").keys().cloned().collect();
    for id in ids {
        if let Err(error) = close(&id) {
            tracing::warn!(binding_id = %id, %error, "structured session shutdown failed");
        }
    }
}

/// Reattach without restarting an existing process. A live PTY is never replaced implicitly.
pub fn ensure(pool: &TerminalPool, spec: TerminalSpec) -> Result<SessionSnapshot, AppError> {
    ensure_inner(pool, spec, false)
}

fn ensure_inner(
    pool: &TerminalPool,
    spec: TerminalSpec,
    prepare_only: bool,
) -> Result<SessionSnapshot, AppError> {
    let terminals = pool.lock().expect("terminal pool");
    if terminals
        .get(&spec.terminal_id)
        .is_some_and(|session| session.is_child_alive())
    {
        return Err(AppError::Conflict("This session is open in Terminal. Close its terminal before switching to Conversation.".into()));
    }
    let mut sessions = SESSIONS.lock().expect("sessions");
    if let Some(session) = sessions.get(&spec.terminal_id) {
        let state = session.state.lock().expect("session state");
        if !matches!(state.status.as_str(), "failed" | "closed")
            && matches!(
                session.child.lock().expect("session child").try_wait(),
                Ok(None)
            )
        {
            if owner(&session.spec) != owner(&spec) {
                return Err(AppError::Conflict(
                    "Close the session before changing its binding configuration".into(),
                ));
            }
            return Ok(state.clone());
        }
        session.stop_process()?;
    }
    if spec.terminal_id.is_empty()
        || !spec
            .terminal_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::Validation("invalid session binding id".into()));
    }
    let driver = spec
        .driver_type
        .parse::<DriverType>()
        .map_err(|e| internal("session driver", e))?;
    let codex = driver == DriverType::CodexTerminal;
    if !codex && driver != DriverType::ClaudeTerminal {
        return Err(AppError::Validation(
            "Structured conversations support Claude Code and Codex".into(),
        ));
    }
    let directory = PathBuf::from(&spec.workspace_path).join(".choruz/sessions");
    std::fs::create_dir_all(&directory)
        .map_err(|e| internal("create session history directory", e))?;
    let identity = owner_key(&spec);
    let journal = directory.join(format!("{}-{identity}.json", spec.terminal_id));
    let mut submissions = HashMap::new();
    let had_saved = journal.exists();
    if std::fs::metadata(&journal).is_ok_and(|metadata| metadata.len() > 8 * 1024 * 1024) {
        return Err(AppError::Validation("Saved conversation preview exceeds its size limit. Use Terminal to inspect the native session.".into()));
    }
    let mut state: SessionSnapshot = match std::fs::read(&journal) {
        Ok(bytes) => {
            let saved: SavedSession =
                serde_json::from_slice(&bytes).map_err(|e| internal("read session history", e))?;
            if saved.version != 1 || saved.owner != owner(&spec) {
                return Err(AppError::Conflict(
                    "Saved session belongs to a different binding configuration".into(),
                ));
            }
            submissions = saved.submissions;
            saved.state
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => SessionSnapshot::default(),
        Err(e) => return Err(internal("read session history", e)),
    };
    state.status = "connecting".into();
    state.owner = identity;
    state.instance = choruz_common::new_id();
    state.revision += 1;
    state.requests.clear();
    state.turn_id = None;
    state.error = None;
    if state.session_id.is_none() {
        state.session_id = spec.resume_session_id.clone();
        if !codex
            && spec.harness_account["external_session_mode"] == "headless"
            && spec.harness_account["terminal_session"]["session_id"].as_str()
                != spec.resume_session_id.as_deref()
        {
            state.fork_source = state.session_id.take();
        }
    }
    if !codex {
        let id = state
            .session_id
            .get_or_insert_with(choruz_common::new_id)
            .clone();
        state.native_home_path = Some(
            crate::session_history::claude_account_root(&spec)?
                .to_string_lossy()
                .into(),
        );
        match crate::session_history::claude_history(&spec, &id) {
            Ok(native) => {
                state.items = native.items;
                state.native_session_path = native.native_session_path;
                state.fork_source = None;
                state.revision = state.revision.max(native.revision) + 1;
            }
            Err(AppError::NotFound(_))
                if had_saved
                    || spec.resume_session_id.is_none()
                    || state.fork_source.is_some()
                    || (spec.harness_account["terminal_session"]["provenance"]
                        == "direct_session_reserved"
                        && spec.harness_account["terminal_session"]["session_id"] == id) =>
            {
                state.native_session_path = None;
                if let Some(source) = &state.fork_source {
                    let native = crate::session_history::claude_history(&spec, source)?;
                    state.items = native.items;
                    state.revision = state.revision.max(native.revision) + 1;
                } else if !state.items.is_empty() {
                    return Err(AppError::NotFound("The native session was removed from this account. Its stored conversation preview is not a resumable session.".into()));
                }
            }
            Err(error) => return Err(error),
        }
    }
    state.enforce_limits();
    if prepare_only {
        let bytes = serde_json::to_vec(&SavedSession {
            version: 1,
            owner: owner(&spec),
            state: state.clone(),
            submissions,
        })
        .map_err(|e| internal("reserve session identity", e))?;
        write_history(&journal, &bytes)?;
        return Ok(state);
    }
    let mut command = Command::new(crate::terminal_binary(&driver, spec.binary_path.as_deref()));
    command
        .current_dir(&spec.workspace_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // The process container requires a dedicated session/process group.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }
    command.env(
        "CHORUZ_SEND",
        PathBuf::from(&spec.workspace_path).join(".choruz/send"),
    );
    command.env("DISABLE_AUTOUPDATER", "1");
    if let Some(path) = choruz_agent_runtime::computer_use::executable_path() {
        command.env("PATH", path);
    }
    if let Some((key, path)) = prepare_harness_account_env(
        if codex {
            HeadlessDriver::Codex
        } else {
            HeadlessDriver::Claude
        },
        &spec.harness_account,
    )
    .map_err(AppError::Validation)?
    {
        command.env(key, path);
    }
    if codex {
        if let Some(home) = &spec.codex_home {
            command.env("CODEX_HOME", home);
        }
        command.arg("app-server");
    } else {
        for key in CLAUDE_PARENT_SESSION_ENV {
            command.env_remove(key);
        }
        command.args([
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--replay-user-messages",
            "--permission-prompt-tool",
            "stdio",
        ]);
        let id = state
            .session_id
            .as_deref()
            .expect("reserved Claude identity");
        if let Some(source) = &state.fork_source {
            command.args(["--resume", source, "--fork-session", "--session-id", id]);
        } else if state.native_session_path.is_some() {
            command.args(["--resume", id]);
        } else {
            command.args(["--session-id", id]);
        }
        if let Some(model) = &spec.model {
            command.args(["--model", model]);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|e| internal("start structured Harness", e))?;
    let container = ProcessContainer::new(format!("session-{}", spec.terminal_id), child.id());
    let writer = child
        .stdin
        .take()
        .ok_or_else(|| internal("start Harness", "stdin unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| internal("start Harness", "stdout unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| internal("start Harness", "stderr unavailable"))?;
    let session = Arc::new(Session {
        state: Mutex::new(state),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
        container: Mutex::new(Some(container)),
        codex,
        spec: spec.clone(),
        journal,
        submissions: Mutex::new(submissions),
        interrupt_pending: AtomicBool::new(false),
        preflight: Mutex::new(None),
    });
    let reader_session = Arc::clone(&session);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let result = (&mut reader).take(8 * 1024 * 1024 + 1).read_until(b'\n', &mut line)
                .map_err(|e| internal("read Harness output", e))
                .and_then(|_| {
                    if line.len() > 8 * 1024 * 1024 { return Err(AppError::Validation("Harness output exceeded the conversation event limit. Use Terminal to inspect the native session.".into())); }
                    if line.is_empty() { return Ok(()); }
                    let event = serde_json::from_slice(&line).map_err(|e| internal("decode Harness output", e))?;
                    reader_session.receive(event)
                });
            if let Err(error) = result {
                let mut state = reader_session.state.lock().expect("session state");
                state.error = Some(error.to_string());
                state.status = "failed".into();
                state.revision += 1;
                break;
            }
            if line.is_empty() {
                break;
            }
        }
        let mut state = reader_session.state.lock().expect("session state");
        if state.status != "closed" {
            state.status = "failed".into();
            state.revision += 1;
            if state.error.is_none() {
                state.error = Some(
                    "The Harness process disconnected. Reopen the conversation to resume.".into(),
                );
            }
        }
        drop(state);
        let _ = reader_session.stop_process();
    });
    let starting_session = Arc::downgrade(&session);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(90));
        if let Some(session) = starting_session.upgrade() {
            let mut state = session.state.lock().expect("session state");
            if state.status == "connecting" {
                state.status = "failed".into();
                state.error = Some("The Harness did not initialize within 90 seconds. Check the account and CLI installation, then reconnect.".into());
                state.revision += 1;
                drop(state);
                let _ = session.stop_process();
            }
        }
    });
    // Drain diagnostics so a full stderr pipe cannot freeze the protocol. Raw diagnostics
    // can contain credentials or project content and are not copied to platform logs.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut BufReader::new(stderr), &mut std::io::sink());
    });
    session.write(if codex {
        json!({"id":"initialize","method":"initialize","params":{"clientInfo":{"name":"choruz","version":"1"},"capabilities":{"experimentalApi":true}}})
    } else {
        json!({"type":"control_request","request_id":"initialize","request":{"subtype":"initialize"}})
    })?;
    sessions.insert(spec.terminal_id, Arc::clone(&session));
    Ok(session.state.lock().expect("session state").clone())
}

pub fn snapshot(id: &str) -> Result<SessionSnapshot, AppError> {
    let sessions = SESSIONS.lock().expect("sessions");
    let session = sessions
        .get(id)
        .ok_or_else(|| AppError::NotFound("Structured session is not open".into()))?;
    let mut state = session.state.lock().expect("session state");
    if !session.codex
        && state.native_session_path.is_none()
        && state.items.iter().any(|item| item.kind == "assistant")
        && let Some(id) = &state.session_id
        && let Ok(native) = crate::session_history::claude_history(&session.spec, id)
    {
        state.native_session_path = native.native_session_path;
        state.fork_source = None;
        state.revision += 1;
        session.save(&state)?;
    }
    Ok(state.clone())
}

/// Read an existing preview without launching or resuming a Harness.
/// The same owner check applies to live and persisted conversations.
pub fn recorded_snapshot(spec: &TerminalSpec) -> Result<SessionSnapshot, AppError> {
    let expected = owner_key(spec);
    match snapshot(&spec.terminal_id) {
        Ok(state) => {
            verify_owner(&state, &expected)?;
            return Ok(state);
        }
        Err(AppError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    if spec.terminal_id.is_empty()
        || !spec
            .terminal_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::Validation("invalid learning binding id".into()));
    }
    let path = PathBuf::from(&spec.workspace_path)
        .join(".choruz/sessions")
        .join(format!("{}-{expected}.json", spec.terminal_id));
    let file = std::fs::File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::NotFound("No recorded conversation is available yet".into())
        } else {
            internal("read learning source", e)
        }
    })?;
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| internal("read learning source", e))?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(AppError::Validation(
            "Recorded conversation exceeds the journal limit".into(),
        ));
    }
    let saved: SavedSession =
        serde_json::from_slice(&bytes).map_err(|e| internal("decode learning source", e))?;
    if saved.version != 1 || saved.owner != owner(spec) {
        return Err(AppError::Conflict(
            "Learning source belongs to a different Agent configuration".into(),
        ));
    }
    verify_owner(&saved.state, &expected)?;
    Ok(saved.state)
}

/// Commands address only a live request in this binding. Approval never changes account policy.
pub fn command(id: &str, command: SessionCommand) -> Result<SessionSnapshot, AppError> {
    command_owned(id, None, command, None)
}

fn command_owned(
    id: &str,
    expected_owner: Option<(&str, &str)>,
    command: SessionCommand,
    experience: Option<(String, String)>,
) -> Result<SessionSnapshot, AppError> {
    command_prepared(id, expected_owner, command, experience, None)
}

fn command_prepared(
    id: &str,
    expected_owner: Option<(&str, &str)>,
    command: SessionCommand,
    experience: Option<(String, String)>,
    prepared: Option<(String, String)>,
) -> Result<SessionSnapshot, AppError> {
    let session = SESSIONS
        .lock()
        .expect("sessions")
        .get(id)
        .cloned()
        .ok_or_else(|| AppError::NotFound("Structured session is not open".into()))?;
    let mut state = session.state.lock().expect("session state");
    if let Some((owner, instance)) = expected_owner {
        verify_owner(&state, owner)?;
        if state.instance != instance {
            return Err(AppError::Conflict(
                "The Harness restarted. Refresh before sending or approving an action.".into(),
            ));
        }
    }
    state.revision += 1;
    match command {
        SessionCommand::Send {
            text,
            submission_id,
        } => {
            let check_plan = if let Some((reservation, plan)) = prepared {
                let mut pending = session.preflight.lock().expect("preflight reservation");
                if pending.as_ref().is_none_or(|(id, _)| id != &reservation)
                    || state.status != "running"
                {
                    return Err(AppError::Conflict(
                        "Execution review no longer owns this turn".into(),
                    ));
                }
                pending.take();
                state.status = "ready".into();
                Some(plan)
            } else {
                if session
                    .preflight
                    .lock()
                    .expect("preflight reservation")
                    .is_some()
                {
                    return Err(AppError::Conflict(
                        "Wait for the execution review before sending another message".into(),
                    ));
                }
                None
            };
            if text.trim().is_empty()
                || text.len() > 128 * 1024
                || submission_id.is_empty()
                || submission_id.len() > 128
            {
                return Err(AppError::Validation(
                    "A message and a valid submission id are required".into(),
                ));
            }
            let mut submissions = session.submissions.lock().expect("session submissions");
            let fingerprint = hex::encode(Sha256::digest(text.as_bytes()));
            if let Some(prior) = submissions.get(&submission_id) {
                if prior.fingerprint != fingerprint {
                    return Err(AppError::Conflict(
                        "Submission id was used for another message".into(),
                    ));
                }
                if prior.instance != state.instance || state.status == "failed" {
                    return Err(AppError::Conflict(
                        "The previous delivery cannot be confirmed. Inspect the native conversation before sending again.".into(),
                    ));
                }
                return Ok(state.clone());
            }
            if submissions.len() >= 4096 {
                return Err(AppError::Validation("This conversation reached its submission limit. Continue in Terminal or create a new Agent session.".into()));
            }
            if state.status != "ready" {
                return Err(AppError::Conflict(
                    "Wait for the current turn to finish before sending another message".into(),
                ));
            }
            let mut input =
                choruz_agent_runtime::headless::with_experience(text, experience.as_ref());
            if let Some(plan) = check_plan {
                input.push_str("\n\n");
                input.push_str(&plan);
            }
            let request = if session.codex {
                json!({"id":submission_id,"method":"turn/start","params":{"threadId":state.session_id,"input":[{"type":"text","text":input}]}})
            } else {
                json!({"type":"user","uuid":submission_id,"message":{"role":"user","content":[{"type":"text","text":input}]}})
            };
            // Persist the claim before writing to the process. An ambiguous
            // crash is never automatically replayed as another paid turn.
            submissions.insert(
                submission_id,
                Submission {
                    fingerprint,
                    instance: state.instance.clone(),
                },
            );
            drop(submissions);
            state.turn_id = None;
            session.interrupt_pending.store(false, Ordering::SeqCst);
            state.status = "running".into();
            state.error = None;
            session.save(&state)?;
            if let Err(error) = session.write(request) {
                state.status = "failed".into();
                state.error = Some("Message delivery could not be confirmed. Reconnect and inspect the conversation before sending it again.".into());
                let _ = session.save(&state);
                return Err(error);
            }
        }
        SessionCommand::Respond {
            request_id,
            allow,
            answers,
        } => {
            if serde_json::to_vec(&answers)
                .map_err(|e| internal("encode answers", e))?
                .len()
                > 32 * 1024
            {
                return Err(AppError::Validation(
                    "Answers must fit within 32 KiB".into(),
                ));
            }
            let index = state
                .requests
                .iter()
                .position(|request| {
                    request[if session.codex { "id" } else { "request_id" }] == request_id
                })
                .ok_or_else(|| {
                    AppError::Conflict("This question or approval is no longer pending".into())
                })?;
            let pending = &state.requests[index];
            let questions = pending["params"]["questions"]
                .as_array()
                .or_else(|| pending["request"]["input"]["questions"].as_array());
            if allow && let Some(questions) = questions {
                for question in questions {
                    let key = question["id"]
                        .as_str()
                        .or_else(|| question["question"].as_str())
                        .ok_or_else(|| {
                            AppError::Validation("Harness question has no identifier".into())
                        })?;
                    if !answers.get(key).is_some_and(|values| {
                        !values.is_empty() && values.iter().all(|value| !value.trim().is_empty())
                    }) {
                        return Err(AppError::Validation(
                            "Answer each question before submitting".into(),
                        ));
                    }
                }
            }
            let response = if session.codex {
                let result = match pending["method"].as_str().unwrap_or_default() {
                    "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                        json!({"decision": if allow {"accept"} else {"decline"}})
                    }
                    "item/tool/requestUserInput" => {
                        json!({"answers":answers.iter().map(|(key, values)| (key.clone(), json!({"answers":values}))).collect::<serde_json::Map<_, _>>()})
                    }
                    "item/permissions/requestApproval" => {
                        json!({"permissions":if allow { pending["params"]["permissions"].clone() } else { json!({}) }, "scope":"turn"})
                    }
                    _ => {
                        return Err(AppError::Validation(
                            "This Harness interaction requires Terminal".into(),
                        ));
                    }
                };
                json!({"id":request_id,"result":result})
            } else {
                if pending["request"]["subtype"] != "can_use_tool" {
                    return Err(AppError::Validation(
                        "This Harness interaction requires Terminal".into(),
                    ));
                }
                let mut input = pending["request"]["input"].clone();
                if pending["request"]["tool_name"] == "AskUserQuestion" {
                    input["answers"] = json!(
                        answers
                            .iter()
                            .map(|(key, values)| (key.clone(), json!(values.join(", "))))
                            .collect::<serde_json::Map<_, _>>()
                    );
                }
                json!({"type":"control_response","response":{"subtype":"success","request_id":request_id,
                    "response":if allow { json!({"behavior":"allow","updatedInput":input}) } else { json!({"behavior":"deny","message":"User declined"}) }}})
            };
            session.write(response)?;
            state.requests.remove(index);
            if state.requests.is_empty() {
                state.status = "running".into();
            }
        }
        SessionCommand::Interrupt => {
            if let Some((_, cancellation)) = session
                .preflight
                .lock()
                .expect("preflight reservation")
                .take()
            {
                cancellation.notify_one();
                state.status = "ready".into();
                return Ok(state.clone());
            }
            if !matches!(state.status.as_str(), "running" | "waiting") {
                return Err(AppError::Conflict("No active turn to stop".into()));
            }
            if session.codex {
                if let Some(turn_id) = &state.turn_id {
                    session.write(json!({"id":"interrupt","method":"turn/interrupt","params":{"threadId":state.session_id,"turnId":turn_id}}))?;
                } else {
                    session.interrupt_pending.store(true, Ordering::SeqCst);
                }
            } else {
                session.write(json!({"type":"control_request","request_id":"interrupt","request":{"subtype":"interrupt"}}))?;
                session.interrupt_pending.store(true, Ordering::SeqCst);
            }
        }
        SessionCommand::Close => {
            if state.status == "running" || state.status == "waiting" {
                return Err(AppError::Conflict(
                    "Stop the active turn before switching to Terminal".into(),
                ));
            }
            session.save(&state)?;
            state.status = "closed".into();
            session.stop_process()?;
        }
    }
    Ok(state.clone())
}

fn acknowledge_claude_interrupt(state: &mut SessionSnapshot, event: &Value) {
    let user_result = event["result_type"] == "user"
        || event["errors"].as_array().is_some_and(|errors| {
            !errors.is_empty()
                && errors.iter().all(|error| {
                    error
                        .as_str()
                        .is_some_and(|text| text.starts_with("[ede_diagnostic] result_type=user "))
                })
        });
    if user_result {
        state.status = "ready".into();
        state.error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_stop_does_not_hide_a_queued_execution_error() {
        for (errors, expected) in [
            (
                json!([
                    "[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"
                ]),
                "ready",
            ),
            (json!(["authentication expired"]), "failed"),
            (
                json!([
                    "[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use",
                    "disk full"
                ]),
                "failed",
            ),
        ] {
            let result = json!({"type":"result","is_error":true,"errors":errors});
            let mut state = SessionSnapshot::default();
            state.claude(&result);
            acknowledge_claude_interrupt(&mut state, &result);
            assert_eq!(state.status, expected);
            assert_eq!(state.error.is_some(), expected == "failed");
        }
    }

    fn wait(id: &str, status: &str) -> SessionSnapshot {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let state = snapshot(id).unwrap();
            if state.status == status {
                return state;
            }
            assert_ne!(state.status, "failed", "{:?}", state.error);
            assert!(
                std::time::Instant::now() < deadline,
                "waiting for {status}, got {}",
                state.status
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn structured_process_fences_owner_instance_approval_and_duplicate_delivery() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("fixture-cli");
        std::fs::write(
            &binary,
            include_str!("../../../apps/web/tests/fixtures/structured-cli.py"),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let id = choruz_common::new_id();
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = close(&self.0);
            }
        }
        let _cleanup = Cleanup(id.clone());
        let spec = TerminalSpec {
            authentication: false,
            terminal_id: id.clone(),
            driver_type: "codex_terminal".into(),
            binary_path: Some(binary.to_string_lossy().into()),
            workspace_path: directory.path().to_string_lossy().into(),
            cols: 80,
            rows: 24,
            resume_session_id: None,
            codex_home: None,
            model: None,
            harness_account: json!({}),
        };
        let pool = crate::new_terminal_pool();
        ensure(&pool, spec.clone()).unwrap();
        let ready = wait(&id, "ready");
        assert!(
            command_owned(
                &id,
                Some(("another-owner", &ready.instance)),
                SessionCommand::Interrupt,
                None
            )
            .is_err()
        );
        assert!(crate::ensure_terminal(&pool, &spec).is_err());
        let submission = choruz_common::new_id();
        let send = SessionCommand::Send {
            text: "Inspect workspace".into(),
            submission_id: submission.clone(),
        };
        let async_runtime = tokio::runtime::Runtime::new().unwrap();
        async_runtime.block_on(async {
            let request = SessionRequest::Command {
                binding_id: id.clone(),
                owner: ready.owner.clone(),
                instance: ready.instance.clone(),
                command: SessionCommand::Send {
                    text: "wait".into(),
                    submission_id: "cancelled-review".into(),
                },
                experience: None,
                preflight: Some(Box::new(crate::harness::ExecutionTeam {
                    revision_id: "review-revision".into(),
                    team: choruz_domain::team::Team::reviewer(
                        "Verify workspace changes before reporting completion.".into(),
                    ),
                })),
            };
            let reviewing = tokio::spawn(execute(pool.clone(), request));
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while snapshot(&id).unwrap().status != "running" {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            command(&id, SessionCommand::Interrupt).unwrap();
            assert!(
                tokio::time::timeout(std::time::Duration::from_secs(5), reviewing)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
            assert_eq!(snapshot(&id).unwrap().status, "ready");
            assert!(
                snapshot(&id).unwrap().items.is_empty(),
                "cancelled review must not start a native task"
            );
        });
        async_runtime
            .block_on(execute(
                pool.clone(),
                SessionRequest::Command {
                    binding_id: id.clone(),
                    owner: ready.owner.clone(),
                    instance: ready.instance.clone(),
                    command: send.clone(),
                    experience: None,
                    preflight: Some(Box::new(crate::harness::ExecutionTeam {
                        revision_id: "review-revision".into(),
                        team: choruz_domain::team::Team::reviewer(
                            "Verify workspace changes before reporting completion.".into(),
                        ),
                    })),
                },
            ))
            .unwrap();
        let pending = wait(&id, "waiting");
        let native = &pending
            .items
            .iter()
            .find(|item| item.kind == "user")
            .unwrap()
            .text;
        assert!(native.contains("[choruz-team revision=review-revision]"));
        assert!(native.contains("Inspect the changed files before reporting completion."));
        command_owned(&id, Some((&ready.owner, &ready.instance)), send, None).unwrap();
        assert_eq!(
            snapshot(&id)
                .unwrap()
                .items
                .iter()
                .filter(|item| item.kind == "user")
                .count(),
            1
        );
        assert!(!directory.path().join("approved-on-device").exists());
        assert!(
            command(
                &id,
                SessionCommand::Send {
                    text: "different".into(),
                    submission_id: submission
                }
            )
            .is_err()
        );
        let respond = SessionCommand::Respond {
            request_id: pending.requests[0]["id"].clone(),
            allow: true,
            answers: HashMap::new(),
        };
        assert!(
            command_owned(
                &id,
                Some((&ready.owner, "old-instance")),
                respond.clone(),
                None
            )
            .is_err()
        );
        assert!(!directory.path().join("approved-on-device").exists());
        command_owned(
            &id,
            Some((&ready.owner, &ready.instance)),
            respond.clone(),
            None,
        )
        .unwrap();
        wait(&id, "ready");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("approved-on-device")).unwrap(),
            std::fs::canonicalize(&spec.workspace_path)
                .unwrap()
                .to_string_lossy()
        );
        assert!(command(&id, respond.clone()).is_err());
        close(&id).unwrap();
        assert!(!active(&id));
        ensure(&pool, spec).unwrap();
        let reopened = wait(&id, "ready");
        assert_ne!(reopened.instance, ready.instance);
        assert_eq!(reopened.session_id, ready.session_id);
        assert!(command_owned(&id, Some((&ready.owner, &ready.instance)), respond, None).is_err());
        let session = SESSIONS.lock().unwrap().get(&id).unwrap().clone();
        for _ in 0..4096 {
            session.submissions.lock().unwrap().insert(
                choruz_common::new_id(),
                Submission {
                    fingerprint: "0".repeat(64),
                    instance: reopened.instance.clone(),
                },
            );
        }
        let overflow = command(
            &id,
            SessionCommand::Send {
                text: "must not be delivered".into(),
                submission_id: choruz_common::new_id(),
            },
        );
        assert!(
            overflow
                .unwrap_err()
                .to_string()
                .contains("submission limit")
        );
        assert_eq!(snapshot(&id).unwrap().status, "ready");
        let oversized = session.receive(json!({"id":"oversized", "method":"item/tool/requestUserInput", "params":{"questions":"x".repeat(65536)}}));
        assert!(oversized.unwrap_err().to_string().contains("too large"));
        assert!(snapshot(&id).unwrap().requests.is_empty());
    }

    #[test]
    fn oversized_journal_preserves_previous_recoverable_state() {
        let directory = tempfile::tempdir().unwrap();
        let journal = directory.path().join("session.json");
        write_history(&journal, b"previous").unwrap();
        assert!(write_history(&journal, &vec![b'x'; 8 * 1024 * 1024 + 1]).is_err());
        assert_eq!(std::fs::read(&journal).unwrap(), b"previous");
    }
}
