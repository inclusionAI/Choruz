//! Headless Executor: spawns one-shot CLI processes per command.
//!
//! Each supported coding CLI runs headlessly for one turn, then exits. No
//! persistent processes, adapters, or interactive WAL interaction are used.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use choruz_agent_runtime::headless::{
    HeadlessDriver as LocalCliDriver, configure_command_workspace, harness_account_env,
    parse_output, pi_message_is_failed,
};
use choruz_executor::sandbox::{SandboxManager, WorkspaceConfig};
use choruz_executor::wal::AdapterWal;
use choruz_session::{AgentCommand, PgSessionStore};
use choruz_store::EventStore;
use choruz_tools::gateway::{ToolExecutor, ToolGateway};
use choruz_tools::registry::default_registry;
use choruz_writer::{AgentResult, AgentResultStatus};

use crate::config::PipelineConfig;
use crate::instructions::ensure_claude_md;

// ---------------------------------------------------------------------------
// Tool Gateway integration (audit #2)
// ---------------------------------------------------------------------------

/// No-op tool executor for Phase 1.
///
/// In the current architecture, the CLI process (claude) handles tool
/// execution internally.  The ToolGateway is wired in to record tool
/// calls in the effect journal for idempotency on replay.  When we
/// intercept tool calls before they reach the CLI (Phase 2), this
/// executor will be replaced with real HTTP / shell backends.
pub(crate) struct PassthroughToolExecutor;

#[async_trait::async_trait]
impl ToolExecutor for PassthroughToolExecutor {
    async fn execute(
        &self,
        tool_name: &str,
        _input: &serde_json::Value,
        _idempotency_key: &str,
    ) -> Result<serde_json::Value, String> {
        // Phase 1: the CLI already executed the tool.  Return a marker
        // indicating passthrough so the effect journal records the call.
        Ok(serde_json::json!({
            "passthrough": true,
            "tool_name": tool_name,
            "note": "CLI handled execution; recorded for idempotency audit"
        }))
    }
}

// ---------------------------------------------------------------------------
// Task event detection
// ---------------------------------------------------------------------------

/// A task event detected from stream-json output of any CLI agent.
#[derive(Debug, Clone)]
struct DetectedTask {
    id: String,
    subject: String,
    description: Option<String>,
    status: String,
}

#[derive(Debug, Clone)]
struct BindingSessionState {
    binding_id: String,
    workspace_path: String,
    external_session_id: Option<String>,
    driver_type: String,
    config_json: serde_json::Value,
}

fn session_provenance_matches(binding: &BindingSessionState, expected_mode: &str) -> bool {
    matches!(
        binding
            .config_json
            .get("external_session_provenance")
            .and_then(|v| v.as_str()),
        Some("process_captured" | "workspace_scan_verified")
    ) && binding
        .config_json
        .get("external_session_binding_id")
        .and_then(|v| v.as_str())
        == Some(binding.binding_id.as_str())
        && binding
            .config_json
            .get("external_session_driver_type")
            .and_then(|v| v.as_str())
            == Some(binding.driver_type.as_str())
        && binding
            .config_json
            .get("external_session_mode")
            .and_then(|v| v.as_str())
            == Some(expected_mode)
}

/// Extract task events from a Claude Code tool_use block (TaskCreate / TaskUpdate).
fn extract_claude_task(tool_name: &str, input: &serde_json::Value) -> Option<DetectedTask> {
    match tool_name {
        "TaskCreate" => {
            let subject = input.get("subject").and_then(|v| v.as_str())?.to_string();
            let description = input
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            // TaskCreate assigns an ID server-side; use a placeholder that will
            // be overwritten by TaskUpdate.  Use a content-hash so duplicates
            // are naturally deduped.
            let id = input
                .get("id")
                .and_then(|v| {
                    v.as_str()
                        .map(String::from)
                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                })
                .unwrap_or_else(|| format!("tc-{:x}", fxhash(&subject)));
            Some(DetectedTask {
                id,
                subject,
                description,
                status: "pending".into(),
            })
        }
        "TaskUpdate" => {
            let id = input
                .get("taskId")
                .or_else(|| input.get("id"))
                .and_then(|v| {
                    v.as_str()
                        .map(String::from)
                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                })?;
            let status = input
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending")
                .to_string();
            let subject = input
                .get("subject")
                .and_then(|v| v.as_str())
                .unwrap_or("(updated)")
                .to_string();
            let description = input
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(DetectedTask {
                id,
                subject,
                description,
                status,
            })
        }
        _ => None,
    }
}

/// Extract task list from a Codex `todo_list` item (full plan replacement).
fn extract_codex_tasks(item: &serde_json::Value) -> Vec<DetectedTask> {
    let items = match item.get("items").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return vec![],
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            let text = entry.get("text").and_then(|v| v.as_str())?;
            let completed = entry
                .get("completed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(DetectedTask {
                id: (i + 1).to_string(),
                subject: text.to_string(),
                description: None,
                status: if completed {
                    "completed".into()
                } else {
                    "pending".into()
                },
            })
        })
        .collect()
}

/// Simple non-cryptographic hash for generating deterministic task IDs.
fn fxhash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ---------------------------------------------------------------------------
// ExecutorContext
// ---------------------------------------------------------------------------

/// Context for the headless executor.
///
/// Each command spawns a one-shot coding CLI process that exits after producing
/// a response. No persistent processes are maintained.
pub struct ExecutorContext {
    /// Sandbox manager for creating/reusing workspace directories.
    pub sandbox_manager: SandboxManager,

    /// Maximum time to wait for a single response (retained for config compatibility).
    #[allow(dead_code)]
    pub executor_timeout: std::time::Duration,

    /// Path to the `claude` CLI binary.
    pub claude_cli_path: String,

    /// Path to the `codex` CLI binary.
    pub codex_cli_path: String,

    /// Path to the `pi` CLI binary.
    pub pi_cli_path: String,

    /// Path to the `grok` CLI binary.
    pub grok_cli_path: String,

    /// Path to the `opencode` CLI binary.
    pub opencode_cli_path: String,

    /// Path to the `mathcode` CLI binary.
    pub mathcode_cli_path: String,

    /// Base directory for WAL databases.
    pub wal_base_dir: PathBuf,

    /// Reference to the session store for heartbeat updates.
    pub session_store: Option<Arc<PgSessionStore>>,

    /// Tool Gateway for idempotent tool invocation tracking (retained for future use).
    #[allow(dead_code)]
    pub tool_gateway: Arc<ToolGateway<PassthroughToolExecutor>>,

    /// Event store for obtaining DB clients (used by Tool Gateway to write
    /// to the effect_journal table).
    pub event_store: Option<EventStore>,

    /// Gateway base URL for outbox command processing (provision_agent, share_file, etc.)
    pub gateway_base_url: String,
}

impl ExecutorContext {
    pub fn from_config(config: &PipelineConfig) -> Self {
        let workspace_config = WorkspaceConfig {
            base_dir: PathBuf::from(&config.sandbox_base_dir),
            git_repo: None,
            git_branch: None,
        };

        let wal_base_dir = PathBuf::from(&config.sandbox_base_dir).join("_wal");

        // Instantiate Tool Gateway with the default tool registry.
        // Phase 1 uses a passthrough executor since the CLI handles tool
        // execution directly; the gateway records calls in the effect
        // journal for idempotent replay.
        let tool_registry = default_registry();
        let tool_gateway = Arc::new(ToolGateway::new(tool_registry, PassthroughToolExecutor));

        Self {
            sandbox_manager: SandboxManager::new(workspace_config),
            executor_timeout: config.executor_timeout(),
            claude_cli_path: config.claude_cli_path.clone(),
            codex_cli_path: config.codex_cli_path.clone(),
            pi_cli_path: config.pi_cli_path.clone(),
            grok_cli_path: config.grok_cli_path.clone(),
            opencode_cli_path: config.opencode_cli_path.clone(),
            mathcode_cli_path: config.mathcode_cli_path.clone(),
            wal_base_dir,
            session_store: None,
            tool_gateway,
            event_store: None,
            gateway_base_url: config.gateway_base_url.clone(),
        }
    }

    /// Set the session store reference (for heartbeats).
    pub fn with_session_store(mut self, store: Arc<PgSessionStore>) -> Self {
        self.session_store = Some(store);
        self
    }

    /// Set the event store reference (for Tool Gateway DB access).
    pub fn with_event_store(mut self, store: EventStore) -> Self {
        self.event_store = Some(store);
        self
    }

    /// Recover incomplete turns from all WAL databases on startup.
    ///
    /// For each incomplete turn, we report failure to the session manager so it
    /// can schedule a retry.
    pub async fn recover_from_wal(&self) {
        // Ensure WAL dir exists
        if let Err(e) = tokio::fs::create_dir_all(&self.wal_base_dir).await {
            tracing::error!(error = %e, "failed to create WAL base dir");
            return;
        }

        let mut entries = match tokio::fs::read_dir(&self.wal_base_dir).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "WAL base dir not readable, skipping recovery");
                return;
            }
        };

        let mut recovered_count = 0u32;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "db") {
                match AdapterWal::open(&path) {
                    Ok(wal) => match wal.find_incomplete_turns().await {
                        Ok(incomplete) => {
                            for turn in &incomplete {
                                tracing::warn!(
                                    turn_id = %turn.turn_id,
                                    attempt_id = %turn.attempt_id,
                                    "WAL recovery: found incomplete turn, marking as failed"
                                );
                                if let Err(e) = wal
                                    .log_turn_failed(
                                        &turn.turn_id,
                                        &turn.attempt_id,
                                        "executor crash recovery: process was not running",
                                    )
                                    .await
                                {
                                    tracing::warn!(error = %e, "WAL: failed to record turn failure");
                                }
                                recovered_count += 1;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                path = %path.display(),
                                error = %e,
                                "failed to query WAL for incomplete turns"
                            );
                        }
                    },
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "failed to open WAL database"
                        );
                    }
                }
            }
        }

        if recovered_count > 0 {
            tracing::info!(recovered_count, "WAL crash recovery complete");
        } else {
            tracing::info!("WAL crash recovery: no incomplete turns found");
        }
    }

    /// Execute a command headlessly (one-shot process, returns response text).
    async fn execute_headless(
        &self,
        cmd: &AgentCommand,
    ) -> Result<(Option<String>, Vec<serde_json::Value>), String> {
        self.spawn_headless_session(cmd).await
    }

    /// Query `agent_runtime_bindings` for the agent's real workspace path,
    /// external session ID, driver_type, and provenance metadata.
    async fn lookup_agent_binding(
        &self,
        agent_principal_id: &str,
    ) -> Result<Option<BindingSessionState>, String> {
        let client = if let Some(ref es) = self.event_store {
            es.connect().await.map_err(|error| {
                format!(
                    "agent binding lookup failed for {agent_principal_id}: database connection: {error}"
                )
            })?
        } else {
            return Err(format!(
                "agent binding lookup failed for {agent_principal_id}: event store is not configured"
            ));
        };

        // UNIQUE partial index `agent_runtime_bindings_one_per_agent`
        // (migration 0018) guarantees at most one active binding per
        // agent_principal_id in steady state. We still pin the row with
        // `ORDER BY updated_at DESC LIMIT 1` because provision flows that
        // disable-then-insert can briefly expose two non-disabled rows to a
        // concurrent reader — without the LIMIT, query_opt would blow up and
        // the whole dispatch path fails. Deliberately no conversation_id
        // filter: runtime state is workspace-scoped and shared across every
        // group the agent participates in.
        match client
            .query_opt(
                "SELECT id, workspace_path, external_session_id, driver_type, config_json
                 FROM agent_runtime_bindings
                 WHERE agent_principal_id = $1
                   AND state NOT IN ('disabled')
                 ORDER BY updated_at DESC
                 LIMIT 1",
                &[&agent_principal_id],
            )
            .await
        {
            Ok(Some(row)) => {
                let binding_id: String = row.get("id");
                let workspace_path: String = row.get("workspace_path");
                let external_session_id: Option<String> = row.get("external_session_id");
                let driver_type: String = row.get("driver_type");
                let config_json: serde_json::Value = row.get("config_json");
                tracing::info!(
                    agent_principal_id,
                    binding_id = %binding_id,
                    has_external_session_id = external_session_id.is_some(),
                    driver_type = %driver_type,
                    "resolved agent binding from DB"
                );
                Ok(Some(BindingSessionState {
                    binding_id,
                    workspace_path,
                    external_session_id,
                    driver_type,
                    config_json,
                }))
            }
            Ok(None) => Ok(None),
            Err(error) => Err(format!(
                "agent binding lookup failed for {agent_principal_id}: query: {error}"
            )),
        }
    }

    /// Spawn a headless CLI process for a single command.
    /// Returns `Some(response_text)` with the process output.
    async fn spawn_headless_session(
        &self,
        cmd: &AgentCommand,
    ) -> Result<(Option<String>, Vec<serde_json::Value>), String> {
        let epoch = cmd.current_epoch.unwrap_or(0);
        let session_key = &cmd.session_key;

        // 0. Query agent_runtime_bindings for real workspace + session ID + driver_type
        let binding = self.lookup_agent_binding(&cmd.agent_id).await?;

        // 1. Resolve work_dir + driver_type from the binding.
        //
        let binding = binding.ok_or_else(|| {
            format!(
                "no active agent_runtime_binding for agent {} (provisioning likely incomplete)",
                cmd.agent_id
            )
        })?;
        let binding_id = binding.binding_id.clone();
        let selected_model = binding
            .config_json
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(validate_cli_model_id)
            .transpose()?;
        let ws_path = binding.workspace_path.clone();
        let drv = binding.driver_type.clone();
        let real_dir = PathBuf::from(&ws_path);
        if !real_dir.is_dir() {
            return Err(format!(
                "binding workspace_path does not exist on disk: {ws_path} \
                 (provision must mkdir before inserting the binding)"
            ));
        }
        // Still create a sandbox for bookkeeping (session state, epoch tracking).
        let _sandbox = self
            .sandbox_manager
            .get_or_create_sandbox(session_key, &cmd.agent_id, epoch)
            .await
            .map_err(|e| format!("sandbox creation failed: {e}"))?;
        tracing::info!(
            session_key,
            driver_type = %drv,
            "using real workspace from agent_runtime_bindings"
        );

        // Webhook agents are delivered outside this process. They may still
        // leave platform commands in the bound outbox, but require no local
        // CLI selection or process spawn.
        if drv == "webhook_agent" {
            let outbox_result = crate::outbox_handler::process_outbox_commands_with_stats(
                session_key,
                &cmd.agent_id,
                &real_dir,
                &self.gateway_base_url,
                self.event_store.as_ref(),
            )
            .await;
            let failure_count = outbox_result
                .command_results
                .iter()
                .filter(|result| result.get("ok").and_then(|value| value.as_bool()) == Some(false))
                .count();
            if failure_count > 0 {
                tracing::warn!(
                    session_key,
                    agent_id = %cmd.agent_id,
                    processed = outbox_result.processed_count,
                    failures = failure_count,
                    command_results = ?outbox_result.command_results,
                    "webhook_agent: outbox scan complete with command failures"
                );
            } else {
                tracing::info!(
                    session_key,
                    agent_id = %cmd.agent_id,
                    processed = outbox_result.processed_count,
                    "webhook_agent: outbox scan complete"
                );
            }
            return Ok((Some(outbox_result.reply), outbox_result.command_results));
        }

        let cli_driver = LocalCliDriver::from_driver_type(&drv)
            .ok_or_else(|| format!("unsupported local CLI driver: {drv}"))?;
        let resume_session_id = if cli_driver != LocalCliDriver::Claude {
            match binding.external_session_id.clone() {
                Some(sid)
                    if !sid.is_empty() && session_provenance_matches(&binding, "headless") =>
                {
                    Some(sid)
                }
                Some(_) => {
                    tracing::warn!(
                        binding_id = %binding_id,
                        agent_id = %cmd.agent_id,
                        driver_type = %drv,
                        "not resuming CLI session; external_session_id lacks matching headless provenance"
                    );
                    None
                }
                None => None,
            }
        } else {
            binding
                .external_session_id
                .clone()
                .filter(|sid| !sid.is_empty())
        };
        let (work_dir, driver_type) = (real_dir, drv);

        // 1b. Ensure the driver-appropriate bootstrap file exists in work_dir
        //     (Choruz protocol bootstrap). Passing `driver_type` lets the
        //     empty-workspace branch plants AGENTS.md for AGENTS-compatible
        //     drivers instead of silently writing a CLAUDE.md the running
        //     CLI will never read — the preserve-path warning sidecar does
        //     not fire on the fresh-write branch, so this is the only guard
        //     against that silent-failure case.
        ensure_claude_md(&work_dir, Some(driver_type.as_str())).await;

        // 1c. Stage any incoming attachments the router forwarded via
        //     cmd.metadata.attachments into `<workspace>/.choruz-inbox/<id>/<name>`
        //     so the agent can read them with plain fs tools. The effective
        //     prompt gets an `[attached: ...]` suffix listing the local paths.
        //     Telegram-style: files are pre-fetched to a well-known spot and
        //     the agent just reads them; no getFile dance required.
        let effective_prompt =
            stage_incoming_attachments(&cmd, &work_dir, &self.gateway_base_url).await;

        // 2. Build spawn args — headless mode so process exits after each command.
        //    This prevents 200+ zombie processes from accumulating.
        //    Session continuity is maintained via --resume SESSION_ID.
        //    Binary selection is driven by the binding's driver_type, not by a
        //    path-name heuristic, so one pipeline can host mixed CLI agents.
        let cli_path: &str = match cli_driver {
            LocalCliDriver::Claude => &self.claude_cli_path,
            LocalCliDriver::Codex => &self.codex_cli_path,
            LocalCliDriver::Pi => &self.pi_cli_path,
            LocalCliDriver::Grok => &self.grok_cli_path,
            LocalCliDriver::OpenCode => &self.opencode_cli_path,
            LocalCliDriver::MathCode => &self.mathcode_cli_path,
        };
        let spawn_args = headless_cli_args(
            cli_driver,
            resume_session_id.as_deref(),
            selected_model.as_deref(),
            &effective_prompt,
        );

        // 6. Headless mode: spawn process with prompt, read stdout directly.
        //    The process exits after one response — no need for persistent session / adapter.
        //    Every supported local CLI uses this path.
        {
            // Redact full prompt/args — they routinely contain user message
            // content (PII), conversation history, and credentials baked into
            // env. Log only counts + the shape of the invocation.
            tracing::info!(
                event = "cli_spawn",
                session_key,
                driver_type = %driver_type,
                arg_count = spawn_args.len(),
                prompt_len = effective_prompt.len(),
                "spawning CLI process"
            );
            // Hard kill timeout. tokio::time::timeout drops the inner future
            // on elapse; `kill_on_drop(true)` propagates SIGKILL to the
            // spawned CLI on drop, so a hung process (e.g. claude --print
            // stuck in its own internal 10x retry on Anthropic 429) can't
            // pin a slot forever. Covers both the "spawn returned slowly"
            // path AND the "spawn errored but didn't exit" path because the
            // single tokio::time::timeout wraps the whole .output() future.
            let external_outbox_recovery_started_at = SystemTime::now();
            let mut command = tokio::process::Command::new(cli_path);
            configure_command_workspace(&mut command, cli_driver, &work_dir);
            if let Some((key, value)) = harness_account_env(cli_driver, &binding.config_json)? {
                command.env(key, value);
            }
            let cli_future = command
                .args(&spawn_args)
                .env("CHORUZ_WORKSPACE", &work_dir)
                .env("CHORUZ_SEND", work_dir.join(".choruz").join("send"))
                .env("CHORUZ_OUTBOX_DIR", work_dir.join(".choruz-outbox"))
                // Auto-update suppression — per-CLI mechanisms (the old
                // CODEX_/CLAUDE_DISABLE_UPDATE_CHECK vars were recognized
                // by none of the CLIs): Claude reads DISABLE_AUTOUPDATER;
                // codex takes a --config flag (see codex_exec_args). Pi has a
                // documented environment switch; Grok uses --no-auto-update.
                .env("DISABLE_AUTOUPDATER", "1")
                .env("PI_SKIP_VERSION_CHECK", "1")
                .env("CLAUDE_CODE_ENABLE_TASKS", "1") // Enable file-backed TaskCreate in --print mode
                .kill_on_drop(true)
                .output();

            let output = match tokio::time::timeout(self.executor_timeout, cli_future).await {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    // Do not retain the OS error text: it can disclose a local
                    // path. A missing executable is deterministic and must not
                    // enter the retry loop; other spawn failures can be brief
                    // process/OS hiccups and are safe to retry.
                    let kind = classify_cli_start_error(e.kind());
                    return Err(format!("headless CLI could not start [kind={kind}]"));
                }
                Err(_elapsed) => {
                    tracing::error!(
                        event = "cli_hard_timeout",
                        session_key,
                        driver_type = %driver_type,
                        timeout_secs = self.executor_timeout.as_secs(),
                        "CLI process exceeded hard timeout; SIGKILL via kill_on_drop"
                    );
                    return Err(format!(
                        "CLI process exceeded hard timeout of {}s [kind=timeout]",
                        self.executor_timeout.as_secs()
                    ));
                }
            };

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            // Detect resume-failure signals so we can drop the stale
            // external_session_id from the binding.  Without this the
            // executor loops forever: every dispatch reads the same bad
            // session_id back from agent_runtime_bindings.
            //
            // Codex:  "No saved session found with ID <uuid>"
            // Other drivers expose equivalent missing-session failures.
            //
            // This check is intentionally INDEPENDENT of exit status and
            // stdout emptiness.  Codex (and friends) can emit a handful of
            // stdout bytes (meta / init / version events) before reporting
            // the resume failure on stderr, and exit status is not
            // guaranteed to be non-zero in every path.  Gating cleanup on
            // `!status.success() && stdout.is_empty()` as before meant those
            // edge cases silently skipped the clear, so the same bad
            // session_id was re-read on every retry — the exact loop this
            // fix is supposed to break.
            //
            // We still require `resume_session_id` to have been set — other
            // failure modes (network, quota, binary missing) must NOT wipe
            // the session UUID.
            let stderr_lower = stderr.to_lowercase();
            let looks_like_resume_failure =
                resume_session_id.as_ref().is_some_and(|s| !s.is_empty())
                    && (contains_resume_failure_phrase(&stderr_lower)
                        || structured_stdout_indicates_resume_failure(&stdout));

            if looks_like_resume_failure {
                // Safe to unwrap: looks_like_resume_failure implies Some(non-empty).
                let sid = resume_session_id.as_ref().unwrap();
                let driver_label = cli_driver.label();
                // Never log or return stderr content here — CLI stderr
                // routinely carries prompt fragments (user content) and
                // occasionally stray credentials baked into env. We only
                // need *shape* information to alert on resume-failure rates.
                tracing::warn!(
                    event = "resume_failure_detected",
                    session_key,
                    agent_id = %cmd.agent_id,
                    driver = driver_label,
                    exit_status = %output.status,
                    stdout_len = stdout.len(),
                    stderr_len = stderr.len(),
                    error_kind = "resume_failure",
                    "resume failed — clearing stale external_session_id so retry spawns a fresh session"
                );
                if let Some(ref es) = self.event_store {
                    match es.connect().await {
                        Ok(client) => {
                            let clear = client
                                .execute(
                                    "UPDATE agent_runtime_bindings
                                     SET external_session_id = NULL,
                                         config_json = config_json
                                           - 'external_session_provenance'
                                           - 'external_session_driver_type'
                                           - 'external_session_binding_id'
                                           - 'external_session_mode'
                                           - 'external_session_captured_at',
                                         updated_at = NOW()
                                     WHERE id = $1
                                       AND external_session_id = $2",
                                    &[&binding_id, sid],
                                )
                                .await;
                            match clear {
                                Ok(rows) => tracing::info!(
                                    binding_id = %binding_id,
                                    agent_id = %cmd.agent_id,
                                    rows,
                                    "binding session_id and provenance cleared"
                                ),
                                Err(e) => tracing::warn!(
                                    binding_id = %binding_id,
                                    agent_id = %cmd.agent_id,
                                    error = %e,
                                    "binding session_id clear failed"
                                ),
                            }
                        }
                        Err(e) => tracing::warn!(
                            agent_id = %cmd.agent_id,
                            error = %e,
                            "binding session_id clear: DB connect failed"
                        ),
                    }
                } else {
                    tracing::warn!(
                        agent_id = %cmd.agent_id,
                        "binding session_id clear: no event_store"
                    );
                }

                // A resume failure is terminal for this invocation even when
                // the CLI emitted metadata before the failure. Parsing that
                // metadata could capture and re-persist a stale session ID,
                // defeating the cleanup above and marking the command as a
                // false success. Keep the durable error summary safe.
                return Err(format!(
                    "headless CLI resume failure [kind=resume_failure; exit_status={}; stdout_len={}; stderr_len={}]",
                    output.status,
                    stdout.len(),
                    stderr.len(),
                ));
            }

            if cli_driver == LocalCliDriver::Pi
                && parse_output(cli_driver, &stdout).structured_error
            {
                tracing::error!(
                    event = "cli_structured_error",
                    session_key,
                    driver_type = %driver_type,
                    exit_status = %output.status,
                    stdout_len = stdout.len(),
                    stderr_len = stderr.len(),
                    "headless CLI reported a structured error"
                );
                return Err(format!(
                    "headless CLI failed [kind=structured_driver_error; exit_status={}; stdout_len={}; stderr_len={}]",
                    output.status,
                    stdout.len(),
                    stderr.len(),
                ));
            }

            // Generic "completely failed" fallback: only abort the dispatch
            // when the CLI exited non-zero AND produced no stdout at all.
            // If stdout is non-empty we fall through to JSONL parsing so a
            // partial reply (plus any cleanup above) still makes it to the
            // caller — the next retry will run with a fresh session.
            let hard_failure_error = if !output.status.success() && stdout.is_empty() {
                // Raw stderr commonly carries prompt material, credentials,
                // and driver-local paths. AgentResult.error is persisted by
                // dispatch, so retain only a stable classification and
                // non-sensitive diagnostic shape.
                let error_kind = classify_executor_error(&stderr);
                let error_msg = format!(
                    "headless CLI failed [kind={error_kind}; exit_status={}; stdout_len={}; stderr_len={}; resume_failure={}]",
                    output.status,
                    stdout.len(),
                    stderr.len(),
                    looks_like_resume_failure,
                );
                tracing::error!(
                    event = "cli_print_failed",
                    session_key,
                    exit_status = %output.status,
                    stderr_len = stderr.len(),
                    stdout_len = stdout.len(),
                    "headless --print failed"
                );
                Some(error_msg)
            } else {
                None
            };

            // Extract result from JSONL stdout.
            // Supports the structured event formats emitted by every local CLI.
            let mut response_text = String::new();
            let mut new_session_id: Option<String> = None;
            // Outbox commands are processed by Maildir watcher after execution.
            let mut detected_tasks: Vec<DetectedTask> = Vec::new();
            let mut is_codex_full_plan = false; // Codex sends full plan — requires replace-all
            for line in stdout.lines() {
                if let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) {
                    let msg_type = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    let discovered_session_id = match cli_driver {
                        LocalCliDriver::Pi if msg_type == "session" => {
                            obj.get("id").and_then(|value| value.as_str())
                        }
                        LocalCliDriver::Grok if msg_type == "end" => {
                            obj.get("sessionId").and_then(|value| value.as_str())
                        }
                        LocalCliDriver::OpenCode => {
                            obj.get("sessionID").and_then(|value| value.as_str())
                        }
                        _ => None,
                    };
                    if let Some(session_id) = discovered_session_id {
                        new_session_id = Some(session_id.to_owned());
                    }
                    if let Some(text) = structured_response_text(cli_driver, &obj) {
                        response_text.push_str(&text);
                    }
                    match msg_type {
                        // ── Claude Code: session init ──
                        "system" => {
                            if let Some(sid) = obj.get("session_id").and_then(|s| s.as_str()) {
                                tracing::info!(
                                    session_key,
                                    session_captured = true,
                                    "claude init: captured session_id"
                                );
                                new_session_id = Some(sid.to_string());
                            }
                        }
                        // ── Codex: thread init ──
                        "thread.started" => {
                            if let Some(tid) = obj.get("thread_id").and_then(|s| s.as_str()) {
                                new_session_id = Some(tid.to_string());
                            }
                        }
                        // ── Claude Code: final result ──
                        "result" => {
                            if let Some(result) = obj.get("result").and_then(|r| r.as_str()) {
                                response_text = result.to_string();
                            }
                        }
                        // ── Codex: item completed (agent_message = reply text, todo_list = tasks) ──
                        "item.completed" => {
                            if let Some(item) = obj.get("item") {
                                let item_type =
                                    item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                if item_type == "agent_message" {
                                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                        // Keep last agent_message as the response (final summary)
                                        response_text = text.to_string();
                                    }
                                }
                                // ── Codex: todo_list → task detection ──
                                if item_type == "todo_list" {
                                    let codex_tasks = extract_codex_tasks(item);
                                    if !codex_tasks.is_empty() {
                                        // Codex sends the FULL plan each time — replace all
                                        detected_tasks = codex_tasks;
                                        is_codex_full_plan = true;
                                    }
                                }
                            }
                        }
                        // ── Claude Code: assistant message with content blocks ──
                        "assistant" => {
                            if let Some(msg) = obj.get("message") {
                                if let Some(content) = msg.get("content").and_then(|c| c.as_array())
                                {
                                    for block in content {
                                        if let Some(text) =
                                            block.get("text").and_then(|t| t.as_str())
                                        {
                                            if response_text.is_empty() {
                                                response_text = text.to_string();
                                            }
                                        }
                                        // Detect tool_use blocks (outbox commands + task events)
                                        if block.get("type").and_then(|t| t.as_str())
                                            == Some("tool_use")
                                        {
                                            let tool_name = block
                                                .get("name")
                                                .and_then(|n| n.as_str())
                                                .unwrap_or("unknown");
                                            // ── Claude Code: TaskCreate / TaskUpdate ──
                                            if let Some(input) = block.get("input") {
                                                if let Some(task) =
                                                    extract_claude_task(tool_name, input)
                                                {
                                                    tracing::info!(
                                                        session_key,
                                                        tool_name,
                                                        task_id = %task.id,
                                                        task_status = %task.status,
                                                        "detected Claude Code task event"
                                                    );
                                                    detected_tasks.push(task);
                                                }
                                            }
                                            // Outbox commands are handled by the Maildir-style
                                            // outbox_watcher — no stream-json detection needed.
                                        }
                                    }
                                }
                            }
                        }
                        // ── Unknown type: skip ──
                        _ => {}
                    }
                }
            }

            // Write detected tasks to agent_task table
            if !detected_tasks.is_empty() {
                if let Some(ref es) = self.event_store {
                    if let Ok(client) = es.connect().await {
                        if is_codex_full_plan {
                            // Codex sends the FULL plan each time — delete existing and insert fresh
                            let _ = client
                                .execute(
                                    "DELETE FROM agent_task WHERE agent_id = $1",
                                    &[&cmd.agent_id],
                                )
                                .await;
                        }
                        for task in &detected_tasks {
                            let _ = client
                                .execute(
                                    "INSERT INTO agent_task (id, agent_id, conversation_id, subject, description, status, updated_at)
                                     VALUES ($1, $2, $3, $4, $5, $6, NOW())
                                     ON CONFLICT (agent_id, id) DO UPDATE SET
                                       subject = EXCLUDED.subject,
                                       description = EXCLUDED.description,
                                       status = EXCLUDED.status,
                                       updated_at = NOW()",
                                    &[
                                        &task.id,
                                        &cmd.agent_id,
                                        &cmd.conversation_id,
                                        &task.subject,
                                        &task.description,
                                        &task.status,
                                    ],
                                )
                                .await;
                        }
                        tracing::info!(
                            session_key,
                            count = detected_tasks.len(),
                            is_codex_full_plan,
                            "tasks written to DB"
                        );
                    }
                }
            }

            // Save new session_id back to binding if we got one.
            if let Some(ref sid) = new_session_id {
                tracing::info!(
                    session_key,
                    agent_id = %cmd.agent_id,
                    new_session_captured = true,
                    driver = cli_driver.label(),
                    "updating binding external_session_id"
                );
                if let Some(ref es) = self.event_store {
                    match es.connect().await {
                        Ok(client) => {
                            let provenance = serde_json::json!({
                                "external_session_provenance": "process_captured",
                                "external_session_driver_type": driver_type.as_str(),
                                "external_session_binding_id": binding_id.as_str(),
                                "external_session_mode": "headless",
                                "external_session_captured_at": chrono::Utc::now().to_rfc3339(),
                            });
                            match client
                                .execute(
                                    "UPDATE agent_runtime_bindings
                                     SET external_session_id = $1,
                                         config_json = config_json || $2::jsonb,
                                         updated_at = NOW()
                                     WHERE id = $3",
                                    &[sid, &provenance, &binding_id],
                                )
                                .await
                            {
                                Ok(rows) => tracing::info!(
                                    binding_id = %binding_id,
                                    agent_id = %cmd.agent_id,
                                    rows,
                                    "binding session_id and provenance updated"
                                ),
                                Err(e) => tracing::warn!(
                                    binding_id = %binding_id,
                                    agent_id = %cmd.agent_id,
                                    error = %e,
                                    "binding session_id update failed"
                                ),
                            }
                        }
                        Err(e) => {
                            tracing::warn!(agent_id = %cmd.agent_id, error = %e, "binding session_id update: DB connect failed")
                        }
                    }
                } else {
                    tracing::warn!(agent_id = %cmd.agent_id, "binding session_id update: no event_store");
                }
            } else {
                tracing::debug!(session_key, "no new_session_id extracted from JSONL output");
            }

            tracing::info!(
                session_key,
                response_len = response_text.len(),
                new_session_captured = new_session_id.is_some(),
                "headless --print completed"
            );

            // Drop the CLI's stdout — stdout is internal chatter, not a message.
            // Agents speak into conversations via `.choruz/send`; the outbox
            // scanner returns whatever the outbox produced (empty for a group
            // send that's already been INSERTed, the DM content, or empty).
            let mut outbox_result = crate::outbox_handler::process_outbox_commands_with_stats(
                session_key,
                &cmd.agent_id,
                &work_dir,
                &self.gateway_base_url,
                self.event_store.as_ref(),
            )
            .await;
            let mut saw_outbox_command = outbox_result.processed_count > 0;

            let external_outbox_files = extract_external_outbox_files(
                &stdout,
                &work_dir,
                external_outbox_recovery_started_at,
            );
            if !external_outbox_files.is_empty() {
                tracing::warn!(
                    session_key,
                    agent_id = %cmd.agent_id,
                    file_count = external_outbox_files.len(),
                    "recovering exact outbox commands written from non-bound workdir"
                );
                let mut external_replies = Vec::new();
                let mut external_command_results = Vec::new();
                for recovered in external_outbox_files {
                    let recovered_result = crate::outbox_handler::process_outbox_command_files(
                        session_key,
                        &cmd.agent_id,
                        &recovered.workdir,
                        &self.gateway_base_url,
                        self.event_store.as_ref(),
                        &[recovered.path],
                    )
                    .await;
                    saw_outbox_command |= recovered_result.processed_count > 0;
                    if !recovered_result.reply.trim().is_empty() {
                        external_replies.push(recovered_result.reply);
                    }
                    external_command_results.extend(recovered_result.command_results);
                }
                if !external_replies.is_empty() {
                    let external_reply = external_replies.join("\n\n");
                    if outbox_result.reply.trim().is_empty() {
                        outbox_result.reply = external_reply;
                    } else {
                        outbox_result.reply =
                            format!("{}\n\n{}", outbox_result.reply, external_reply);
                    }
                }
                outbox_result
                    .command_results
                    .extend(external_command_results);
            }

            if let Some(error_msg) = hard_failure_error
                && !saw_outbox_command
            {
                return Err(error_msg);
            }

            if !saw_outbox_command
                && outbox_result.reply.trim().is_empty()
                && !response_text.trim().is_empty()
            {
                tracing::info!(
                    session_key,
                    response_len = response_text.len(),
                    "using parsed assistant text because no outbox reply was produced"
                );
                outbox_result.reply = response_text;
            }

            return Ok((Some(outbox_result.reply), outbox_result.command_results));
        }
    }

    /// Shut down all sessions (no-op — all sessions are headless one-shot processes).
    pub async fn shutdown_all(&self) {
        tracing::debug!("shutdown_all: no-op (all sessions are headless)");
    }
}

// ---------------------------------------------------------------------------
// Public API: execute_command (persistent session mode)
// ---------------------------------------------------------------------------

/// Execute an agent command using a persistent session.
///
/// This is the main entry point called by the dispatch loop in pipeline.rs.
/// It:
/// 1. Validates the epoch (fencing)
/// 2. Gets or creates a persistent claude process for the session
/// 3. Logs the turn start in the local WAL
/// 4. Injects the prompt via the CLI adapter
/// 5. Reads the response from the session JSONL file
/// 6. Records tool calls via Tool Gateway + local WAL
/// 7. Logs the turn result in the WAL
/// 8. Returns an AgentResult
///
/// Note: heartbeat and command status transitions (started / heartbeating)
/// are managed by the dispatch loop in pipeline.rs, not here.
pub async fn execute_command(ctx: &ExecutorContext, cmd: &AgentCommand) -> AgentResult {
    let start = Instant::now();
    let attempt_id = cmd.current_attempt_id.clone().unwrap_or_default();

    // Pull the trace_id the router stashed for us so we can stamp the
    // executor's own logs and pass it through on the AgentResult.
    let trace_id: Option<String> = cmd
        .metadata
        .get("trace_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match execute_command_inner(ctx, cmd).await {
        Ok((content, tool_calls_count, command_results)) => {
            let duration_ms = start.elapsed().as_millis() as i64;
            tracing::info!(
                event = "executor_command_succeeded",
                trace_id = trace_id.as_deref().unwrap_or("none"),
                command_id = %cmd.command_id,
                turn_id = %cmd.turn_id,
                attempt_id = %attempt_id,
                agent_id = %cmd.agent_id,
                conversation_id = %cmd.conversation_id,
                session_key = %cmd.session_key,
                duration_ms,
                content_len = content.len(),
                tool_calls_count,
                "executor: command succeeded"
            );
            AgentResult {
                turn_id: cmd.turn_id.clone(),
                attempt_id,
                command_id: cmd.command_id.clone(),
                session_key: cmd.session_key.clone(),
                conversation_id: cmd.conversation_id.clone(),
                agent_id: cmd.agent_id.clone(),
                status: AgentResultStatus::Succeeded,
                content: Some(content),
                content_type: Some("text/plain".into()),
                error: None,
                tool_calls_count,
                execution_duration_ms: duration_ms,
                secondary_command_attempts: Vec::new(),
                command_results,
                trace_id: trace_id.clone(),
            }
        }
        Err(error_msg) => {
            let duration_ms = start.elapsed().as_millis() as i64;
            // Redact raw CLI stderr before logging — it commonly carries
            // prompt material (PII / user-provided content) and occasionally
            // stray credentials baked into env. Keep `error_kind` + `len`
            // for triage; the full string still rides on AgentResult.error
            // where the writer can persist it into structured DB state (not
            // a log stream) if needed.
            let error_kind = classify_executor_error(&error_msg);
            tracing::error!(
                event = "executor_command_failed",
                trace_id = trace_id.as_deref().unwrap_or("none"),
                command_id = %cmd.command_id,
                turn_id = %cmd.turn_id,
                attempt_id = %attempt_id,
                agent_id = %cmd.agent_id,
                conversation_id = %cmd.conversation_id,
                session_key = %cmd.session_key,
                duration_ms,
                error_kind = %error_kind,
                error_len = error_msg.len(),
                "executor: command failed"
            );
            AgentResult {
                turn_id: cmd.turn_id.clone(),
                attempt_id,
                command_id: cmd.command_id.clone(),
                session_key: cmd.session_key.clone(),
                conversation_id: cmd.conversation_id.clone(),
                agent_id: cmd.agent_id.clone(),
                status: AgentResultStatus::Failed,
                content: None,
                content_type: None,
                error: Some(error_msg),
                tool_calls_count: 0,
                execution_duration_ms: duration_ms,
                secondary_command_attempts: Vec::new(),
                command_results: Vec::new(),
                trace_id,
            }
        }
    }
}

/// Map an executor error string to a coarse, log-safe classification.
/// Categories are chosen so we can alert / group by them without ever
/// printing the raw stderr (which may carry PII).
fn classify_executor_error(msg: &str) -> &'static str {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("unauthorized")
        || lower.contains("401")
        || lower.contains("forbidden")
        || lower.contains("403")
        || lower.contains("authentication")
        || lower.contains("credential")
        || lower.contains("api key")
        || lower.contains("login")
        || lower.contains("not logged in")
        || lower.contains("not authenticated")
        || lower.contains("ineligible")
    {
        "auth"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else if lower.contains("killed") || lower.contains("oom") {
        "killed"
    } else if lower.contains("connection") || lower.contains("network") {
        "network"
    } else if lower.contains("rate limit") || lower.contains("429") {
        "rate_limited"
    } else if lower.contains("invalid config") || lower.contains("configuration") {
        "configuration"
    } else if lower.contains("not found") || lower.contains("404") {
        "not_found"
    } else if lower.contains("resume") {
        "resume_failure"
    } else if lower.contains("spawn") || lower.contains("exec") {
        "spawn"
    } else {
        // A CLI that exits non-zero without a recognized diagnostic most
        // often represents a crashed child process. Retry it within the
        // bounded budget rather than making the user resend the message.
        "process_failed"
    }
}

fn classify_cli_start_error(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::NotFound => "driver_unavailable",
        std::io::ErrorKind::PermissionDenied => "configuration",
        _ => "spawn_failure",
    }
}

/// Return whether a durable, sanitized executor error is safe to retry.
///
/// The decision deliberately works from the persisted `[kind=...]` marker,
/// never from raw CLI stderr. Authentication, missing binaries, and unknown
/// non-zero exits require a human environment fix; retrying them only blocks
/// the agent's one-at-a-time queue.
pub(crate) fn is_auto_retriable_error(error: Option<&str>) -> bool {
    let Some(error) = error else { return false };
    [
        "kind=timeout",
        "kind=killed",
        "kind=network",
        "kind=rate_limited",
        "kind=resume_failure",
        "kind=spawn_failure",
        "kind=process_failed",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

fn contains_resume_failure_phrase(text: &str) -> bool {
    text.contains("no saved session")
        || text.contains("invalid session identifier")
        || text.contains("error resuming session")
        || text.contains("no rollout found")
        || text.contains("thread/resume failed")
}

/// Return true only for a structured CLI error event, never for assistant
/// content carried in the same JSONL stream. This keeps a normal reply which
/// merely discusses resume errors from clearing a valid binding session.
fn structured_stdout_indicates_resume_failure(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        if event.get("type").and_then(|kind| kind.as_str()) != Some("error") {
            return false;
        }
        event
            .get("message")
            .and_then(|message| message.as_str())
            .is_some_and(|message| contains_resume_failure_phrase(&message.to_lowercase()))
    })
}

/// Inner implementation for persistent-session execution.
async fn execute_command_inner(
    ctx: &ExecutorContext,
    cmd: &AgentCommand,
) -> Result<(String, i32, Vec<serde_json::Value>), String> {
    let epoch = cmd.current_epoch.unwrap_or(0);
    let session_key = &cmd.session_key;

    // 1. Epoch validation (fencing)
    //    If the session store is available, validate against the DB.
    //    Otherwise, validate against our local session state.
    if let Some(ref store) = ctx.session_store {
        let epoch_valid = store
            .validate_epoch(session_key, epoch)
            .await
            .map_err(|e| format!("epoch validation failed: {e}"))?;
        if !epoch_valid {
            return Err(format!(
                "epoch mismatch for session {session_key}: command epoch={epoch} is stale (fencing)"
            ));
        }
    }

    // 2. Get or create the persistent session.
    //    For --print mode, the response is captured directly from stdout.
    let (print_mode_response, command_results) = ctx.execute_headless(cmd).await?;

    // All agents now run headlessly — print_mode_response is always Some.
    let content = print_mode_response
        .ok_or_else(|| "headless mode did not produce a response".to_string())?;
    tracing::info!(
        command_id = %cmd.command_id,
        session_key,
        content_len = content.len(),
        "headless mode: returning direct response"
    );
    Ok((content, 0, command_results))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Stage any incoming attachments the router forwarded in `cmd.metadata.attachments`
/// into `<work_dir>/.choruz-inbox/<attachment_id>/<filename>` and return a
/// prompt suffixed with a list of local paths the agent can read.
///
/// Telegram-style behavior: files are pre-fetched to a predictable location so
/// the agent never has to do its own auth/download dance — it just opens the
/// path. Failures are best-effort; the agent still sees the caption in the
/// original prompt even if the download doesn't land.
async fn stage_incoming_attachments(
    cmd: &choruz_session::AgentCommand,
    work_dir: &std::path::Path,
    gateway_base_url: &str,
) -> String {
    let tokens_path = std::env::var("CHORUZ_AGENT_TOKENS_FILE")
        .unwrap_or_else(|_| ".choruz-runtime/agent_tokens.json".into());
    stage_incoming_attachments_from_tokens_file(
        cmd,
        work_dir,
        gateway_base_url,
        std::path::Path::new(&tokens_path),
    )
    .await
}

async fn stage_incoming_attachments_from_tokens_file(
    cmd: &choruz_session::AgentCommand,
    work_dir: &std::path::Path,
    gateway_base_url: &str,
    tokens_path: &std::path::Path,
) -> String {
    let Some(attachments) = cmd.metadata.get("attachments").and_then(|v| v.as_array()) else {
        return cmd.prompt.clone();
    };
    if attachments.is_empty() {
        return cmd.prompt.clone();
    }

    // Read agent's own bearer token (same path the outbox upload uses).
    // require_actor on /v1/attachments enforces caller == actor_id.
    let agent_token = match tokio::fs::read_to_string(&tokens_path).await {
        Ok(raw) => serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v.get(&cmd.agent_id)?.as_str().map(String::from)),
        Err(e) => {
            tracing::warn!(tokens_path = %tokens_path.display(), error = %e, "stage_incoming_attachments: read tokens failed");
            None
        }
    };

    let inbox_root = work_dir.join(".choruz-inbox");
    let mut staged_lines: Vec<String> = Vec::new();

    for att in attachments {
        let att_id = match att.get("attachment_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        let filename = att
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("attachment.bin");
        let mime = att
            .get("mime_type")
            .and_then(|v| v.as_str())
            .unwrap_or("application/octet-stream");
        // Defence in depth: the filename comes from untrusted metadata, so
        // strip anything that could escape the per-attachment directory.
        let safe_name: String = filename
            .chars()
            .filter(|c| !matches!(c, '/' | '\\' | '\0'))
            .collect();
        let safe_name = if safe_name.is_empty() {
            "attachment.bin".into()
        } else {
            safe_name
        };

        let dest_dir = inbox_root.join(att_id);
        let dest_path = dest_dir.join(&safe_name);

        // Skip re-download if already staged (message retries, multiple triggers).
        let already = tokio::fs::metadata(&dest_path).await.is_ok();
        if !already {
            let Some(ref token) = agent_token else {
                tracing::warn!(
                    agent_id = %cmd.agent_id,
                    attachment_id = att_id,
                    "stage_incoming_attachments: no agent token available, skipping download"
                );
                continue;
            };
            let url = format!(
                "{}/v1/attachments/{}?actor_id={}",
                gateway_base_url.trim_end_matches('/'),
                att_id,
                cmd.agent_id,
            );
            let resp = reqwest::Client::new()
                .get(&url)
                .bearer_auth(token)
                .send()
                .await;
            let bytes = match resp {
                Ok(r) if r.status().is_success() => match r.bytes().await {
                    Ok(b) => b.to_vec(),
                    Err(e) => {
                        tracing::warn!(attachment_id = att_id, error = %e, "stage_incoming_attachments: body read failed");
                        continue;
                    }
                },
                Ok(r) => {
                    tracing::warn!(attachment_id = att_id, status = %r.status(), "stage_incoming_attachments: download non-success");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(attachment_id = att_id, error = %e, "stage_incoming_attachments: download HTTP error");
                    continue;
                }
            };
            if let Err(e) = tokio::fs::create_dir_all(&dest_dir).await {
                tracing::warn!(attachment_id = att_id, error = %e, "stage_incoming_attachments: mkdir failed");
                continue;
            }
            if let Err(e) = tokio::fs::write(&dest_path, &bytes).await {
                tracing::warn!(attachment_id = att_id, error = %e, "stage_incoming_attachments: write failed");
                continue;
            }
            tracing::info!(
                attachment_id = att_id,
                path = %dest_path.display(),
                size = bytes.len(),
                mime,
                "staged incoming attachment"
            );
        }

        staged_lines.push(format!(
            "- {} ({}): {}",
            safe_name,
            mime,
            dest_path.display()
        ));
    }

    if staged_lines.is_empty() {
        return cmd.prompt.clone();
    }
    format!(
        "{}\n\n[attached files available locally — read them as needed]\n{}",
        cmd.prompt,
        staged_lines.join("\n"),
    )
}

/// Extract content from `{{CHORUZ_REPLY}}...{{/CHORUZ_REPLY}}` tags.
/// If no tags are present, return the full content.
#[cfg(test)]
fn extract_reply_content(raw: &str) -> String {
    let re = regex::Regex::new(r"\{\{CHORUZ_REPLY\}\}([\s\S]*?)\{\{/CHORUZ_REPLY\}\}")
        .expect("invalid regex");

    if let Some(captures) = re.captures(raw) {
        captures
            .get(1)
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_else(|| raw.to_string())
    } else {
        // No tags -- return the raw content (already clean)
        raw.trim().to_string()
    }
}

/// Count tool call markers in the response text (heuristic).
#[cfg(test)]
fn count_tool_calls(content: &str) -> i32 {
    let tool_re = regex::Regex::new(r"(?i)(tool_use|function_call|<tool>)").ok();
    match tool_re {
        Some(re) => re.find_iter(content).count() as i32,
        None => 0,
    }
}

fn headless_cli_args(
    driver: LocalCliDriver,
    resume_session_id: Option<&str>,
    model: Option<&str>,
    prompt: &str,
) -> Vec<String> {
    driver.args(resume_session_id, model, prompt)
}

fn validate_cli_model_id(value: &str) -> Result<String, String> {
    choruz_agent_runtime::headless::validate_model(value)
        .map(str::to_owned)
        .map_err(|message| format!("{message} [kind=invalid_model]"))
}

fn structured_response_text(driver: LocalCliDriver, event: &serde_json::Value) -> Option<String> {
    let event_type = event.get("type").and_then(|value| value.as_str())?;
    match driver {
        LocalCliDriver::Pi if event_type == "message_end" => {
            let message = event.get("message")?;
            if message.get("role").and_then(|value| value.as_str()) != Some("assistant")
                || pi_message_is_failed(message)
            {
                return None;
            }
            let content = message.get("content")?;
            if let Some(text) = content.as_str() {
                return Some(text.to_owned());
            }
            let text = content
                .as_array()?
                .iter()
                .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
                .collect::<String>();
            (!text.is_empty()).then_some(text)
        }
        LocalCliDriver::Grok if event_type == "text" => {
            event.get("data")?.as_str().map(ToOwned::to_owned)
        }
        LocalCliDriver::OpenCode if event_type == "text" => event
            .get("part")?
            .get("text")?
            .as_str()
            .map(ToOwned::to_owned),
        _ => None,
    }
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecoveredOutboxFile {
    workdir: PathBuf,
    path: PathBuf,
}

fn extract_external_outbox_files(
    stdout: &str,
    bound_work_dir: &Path,
    not_before: SystemTime,
) -> Vec<RecoveredOutboxFile> {
    let bound = bound_work_dir
        .canonicalize()
        .unwrap_or_else(|_| bound_work_dir.to_path_buf());
    let mut files = Vec::new();

    for line in stdout.lines() {
        let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        collect_external_outbox_files_from_codex_event(&obj, &bound, not_before, &mut files);
    }

    files.sort();
    files.dedup();
    files
}

fn collect_external_outbox_files_from_codex_event(
    value: &serde_json::Value,
    bound_work_dir: &Path,
    not_before: SystemTime,
    files: &mut Vec<RecoveredOutboxFile>,
) {
    let Some(obj) = value.as_object() else {
        return;
    };
    collect_external_outbox_files_from_object(obj, bound_work_dir, not_before, files);
    if let Some(item) = obj.get("item").and_then(|item| item.as_object()) {
        collect_external_outbox_files_from_object(item, bound_work_dir, not_before, files);
    }
}

fn collect_external_outbox_files_from_object(
    obj: &serde_json::Map<String, serde_json::Value>,
    bound_work_dir: &Path,
    not_before: SystemTime,
    files: &mut Vec<RecoveredOutboxFile>,
) {
    let Some(name) = obj.get("name").and_then(|value| value.as_str()) else {
        return;
    };
    if !matches!(name, "exec_command" | "shell" | "bash") {
        return;
    }
    let Some(arguments) = obj.get("arguments").and_then(|value| value.as_str()) else {
        return;
    };
    let Ok(arguments) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return;
    };
    let Some(args) = arguments.as_object() else {
        return;
    };

    let Some(command) = args
        .get("cmd")
        .or_else(|| args.get("command"))
        .and_then(|value| value.as_str())
    else {
        return;
    };
    let Some(workdir) = args
        .get("workdir")
        .or_else(|| args.get("cwd"))
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
    else {
        return;
    };

    for (external_workdir, payload) in external_choruz_send_targets(command, &workdir) {
        if external_workdir == bound_work_dir {
            continue;
        }

        files.extend(matching_external_outbox_files(
            &external_workdir,
            &payload,
            not_before,
        ));
    }
}

fn external_choruz_send_targets(
    command: &str,
    workdir: &Path,
) -> Vec<(PathBuf, serde_json::Value)> {
    let mut targets = Vec::new();
    let mut search_from = 0;
    while let Some(relative_idx) = command[search_from..].find(".choruz/send") {
        let helper_marker = search_from + relative_idx;
        let token_start = command[..helper_marker]
            .rfind(char::is_whitespace)
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let token_end = command[helper_marker..]
            .find(char::is_whitespace)
            .map(|idx| helper_marker + idx)
            .unwrap_or(command.len());
        let helper_token =
            command[token_start..token_end].trim_matches(|ch| matches!(ch, '"' | '\'' | '`'));
        if let Some(payload) = json_payload_after(command, token_end) {
            if let Some(external_workdir) = helper_workspace(helper_token, workdir) {
                targets.push((external_workdir, payload));
            }
        }
        search_from = token_end;
    }
    targets
}

fn helper_workspace(helper_token: &str, workdir: &Path) -> Option<PathBuf> {
    let helper_path = PathBuf::from(helper_token);
    let helper_path = if helper_path.is_absolute() {
        helper_path
    } else {
        workdir.join(helper_path)
    };
    let helper_path = helper_path.canonicalize().unwrap_or(helper_path);
    let external_workdir = helper_path.parent()?.parent()?.to_path_buf();
    Some(external_workdir.canonicalize().unwrap_or(external_workdir))
}

fn json_payload_after(command: &str, offset: usize) -> Option<serde_json::Value> {
    let start = offset + command[offset..].find('{')?;
    let mut in_string = false;
    let mut escaped = false;
    let mut depth = 0usize;
    for (relative_idx, ch) in command[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    let end = start + relative_idx;
                    return serde_json::from_str::<serde_json::Value>(&command[start..=end]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

fn matching_external_outbox_files(
    external_workdir: &Path,
    payload: &serde_json::Value,
    not_before: SystemTime,
) -> Vec<RecoveredOutboxFile> {
    let maildir_new = external_workdir.join(".choruz-outbox").join("new");
    let Ok(entries) = std::fs::read_dir(maildir_new) else {
        return Vec::new();
    };

    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata
            .modified()
            .ok()
            .is_some_and(|modified| modified < not_before)
        {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let parsed = serde_json::from_str::<serde_json::Value>(&raw)
            .or_else(|_| serde_json::from_str::<serde_json::Value>(&raw.replace("\\\"", "\"")));
        if parsed.as_ref().ok() == Some(payload) {
            matches.push(RecoveredOutboxFile {
                workdir: external_workdir.to_path_buf(),
                path,
            });
        }
    }
    matches
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
