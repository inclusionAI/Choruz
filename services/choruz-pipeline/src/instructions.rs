//! Workspace instruction file (CLAUDE.md / AGENTS.md) bootstrap.
//!
//! Implements ADR-006: bootstrap files carry a version header so the pipeline
//! can refresh stale managed instructions written by an older code version,
//! while leaving operator-edited files untouched. When a refresh is
//! skipped because the file appears edited, a non-chat warning sidecar is
//! written to `<workspace>/.choruz-bootstrap-warning.json` so operators can
//! discover the situation without polluting the chat timeline; a force-rewrite
//! escape hatch is exposed via the `choruz-pipeline rebootstrap <agent>`
//! subcommand.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Current modular agent-instruction bootstrap version.
///
/// The runtime recognises the current managed layout plus the v6-v9
/// managed layouts so protocol refreshes preserve designed roles. Older
/// bootstrap bodies are preserved with a warning and require explicit offline
/// rebootstrap.
///
/// Version history:
/// - v1: initial channel-task-aware bootstrap. Body included the
///   "visible agent or human in the current roster" wording for `assignee`.
/// - v2: tightened `assignee` rule to "visible agent from the current roster"
///   across templates and this fallback. The injected `roster:` only contains
///   visible agents, so the v1 wording was confusing and produced avoidable
///   `invalid_assignee` failures.
/// - v3: document the `your_tasks:` envelope hint so agents `task_update`
///   against existing cards instead of fabricating keys or creating
///   duplicates.
/// - v4: document message threads (the `thread:` envelope field, replying
///   into a thread via the send command's `"thread"` param, broadcast
///   etiquette).
/// - v5: clarify the broadcast-default difference between the outbox
///   `"thread"` param (broadcast by default) and direct HTTP API threaded
///   replies (thread-only by default).
/// - v6: delimit role instructions so protocol refreshes preserve them.
/// - v7: compose a core transport protocol and standard capability extensions;
///   AI Manager-only workflow instructions remain in its designed role block.
/// - v8: teach chronological batch supersession, action-only mentions,
///   owner-scoped task closure, and final-acceptance discipline.
/// - v9: keep concise result formatting from suppressing task closure and
///   the routing mention needed to wake a coordinator.
/// - v10: rely on the assignee's authoritative `your_tasks:` key and keep its
///   task update plus routed completion report in one turn instead of waiting
///   on asynchronously emitted command results.
pub(crate) const BOOTSTRAP_INSTRUCTION_VERSION: u32 = 10;

const BOOTSTRAP_VERSION_HEADER_PREFIX: &str = "<!-- choruz-bootstrap-version: ";
const BOOTSTRAP_VERSION_HEADER_SUFFIX: &str = " -->";
const ROLE_PLACEHOLDER: &str = "{{AGENT_INSTRUCTIONS}}";
const ROLE_START: &str = "<!-- choruz-role:start -->";
const ROLE_END: &str = "<!-- choruz-role:end -->";
const CORE_PROTOCOL_PLACEHOLDER: &str = "{{CHORUZ_CORE_PROTOCOL}}";
const STANDARD_EXTENSIONS_PLACEHOLDER: &str = "{{CHORUZ_STANDARD_EXTENSIONS}}";
const DEFAULT_ROLE: &str = "You are an AI assistant on the Choruz platform.";
const CLAUDE_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../../agent-templates/agent-claude-md-template.md");
const CODEX_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("../../../agent-templates/agent-codex-md-template.md");
const V6_CLAUDE_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("instructions_fixtures/bootstrap-v6-claude.md");
const V6_CODEX_INSTRUCTIONS_TEMPLATE: &str =
    include_str!("instructions_fixtures/bootstrap-v6-codex.md");
const V7_MULTI_AGENT_COLLABORATION: &str =
    include_str!("instructions_fixtures/bootstrap-v7-multi-agent-collaboration.md");
const V8_MULTI_AGENT_COLLABORATION: &str =
    include_str!("instructions_fixtures/bootstrap-v8-multi-agent-collaboration.md");
const V9_MULTI_AGENT_COLLABORATION: &str =
    include_str!("instructions_fixtures/bootstrap-v9-multi-agent-collaboration.md");
const CORE_PROTOCOL: &str = include_str!("../../../agent-templates/core-protocol.md");
const STANDARD_EXTENSIONS: &[&str] = &[
    include_str!("../../../agent-templates/extensions/multi-agent-collaboration.md"),
    include_str!("../../../agent-templates/extensions/command-results.md"),
    include_str!("../../../agent-templates/extensions/file-sharing.md"),
    include_str!("../../../agent-templates/extensions/agent-management.md"),
    include_str!("../../../agent-templates/extensions/group-management.md"),
    include_str!("../../../agent-templates/extensions/scheduled-tasks.md"),
    include_str!("../../../agent-templates/extensions/collaboration-practices.md"),
];

/// Sidecar file written when a bootstrap refresh is skipped because the
/// on-disk content is not a recognised managed template. Operators discover
/// it by listing workspaces (`find <runtime>/workspaces -name
/// .choruz-bootstrap-warning.json`); the chat timeline is never used for this.
pub(crate) const BOOTSTRAP_WARNING_FILENAME: &str = ".choruz-bootstrap-warning.json";

const INSTRUCTION_FILENAMES: &[&str] = &["CLAUDE.md", "AGENTS.md"];

/// Pick the canonical bootstrap filename for a runtime binding's
/// `driver_type` column. When the workspace has no existing instruction file
/// and we know which CLI will read it, this avoids planting `CLAUDE.md` in a
/// workspace that only an AGENTS-compatible CLI will look at. Unknown / null
/// A missing hint selects `CLAUDE.md`; an unknown driver is rejected.
fn known_instruction_filename(driver_type: Option<&str>) -> Option<&'static str> {
    match driver_type? {
        "codex_exec" | "codex_terminal" | "codex_app_server" | "pi_terminal" | "grok_terminal"
        | "opencode_terminal" => Some("AGENTS.md"),
        "claude_print" | "claude_terminal" => Some("CLAUDE.md"),
        _ => None,
    }
}

fn bootstrap_filename_for_driver(driver_type: Option<&str>) -> Result<&'static str, String> {
    match driver_type {
        None => Ok("CLAUDE.md"),
        Some(driver_type) => known_instruction_filename(Some(driver_type))
            .ok_or_else(|| format!("unsupported instruction driver: {driver_type}")),
    }
}

fn template_for_path(path: &Path) -> &'static str {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("AGENTS.md") => CODEX_INSTRUCTIONS_TEMPLATE,
        _ => CLAUDE_INSTRUCTIONS_TEMPLATE,
    }
}

fn render_instructions(path: &Path, role: &str) -> String {
    render_instructions_with_extensions(path, role, STANDARD_EXTENSIONS)
}

fn render_instructions_with_extensions(path: &Path, role: &str, extensions: &[&str]) -> String {
    format!(
        "{}\n",
        template_for_path(path)
            .replace(CORE_PROTOCOL_PLACEHOLDER, CORE_PROTOCOL.trim())
            .replace(
                STANDARD_EXTENSIONS_PLACEHOLDER,
                &extensions
                    .iter()
                    .map(|extension| extension.trim())
                    .collect::<Vec<_>>()
                    .join("\n\n---\n\n"),
            )
            .replace(ROLE_PLACEHOLDER, role.trim())
            .trim()
    )
}

fn render_v7_instructions(path: &Path, role: &str) -> String {
    let mut extensions = STANDARD_EXTENSIONS.to_vec();
    extensions[0] = V7_MULTI_AGENT_COLLABORATION;
    render_instructions_with_extensions(path, role, &extensions).replacen(
        &format!("choruz-bootstrap-version: {BOOTSTRAP_INSTRUCTION_VERSION}"),
        "choruz-bootstrap-version: 7",
        1,
    )
}

fn render_v8_instructions(path: &Path, role: &str) -> String {
    let mut extensions = STANDARD_EXTENSIONS.to_vec();
    extensions[0] = V8_MULTI_AGENT_COLLABORATION;
    render_instructions_with_extensions(path, role, &extensions).replacen(
        &format!("choruz-bootstrap-version: {BOOTSTRAP_INSTRUCTION_VERSION}"),
        "choruz-bootstrap-version: 8",
        1,
    )
}

fn render_v9_instructions(path: &Path, role: &str) -> String {
    let mut extensions = STANDARD_EXTENSIONS.to_vec();
    extensions[0] = V9_MULTI_AGENT_COLLABORATION;
    render_instructions_with_extensions(path, role, &extensions).replacen(
        &format!("choruz-bootstrap-version: {BOOTSTRAP_INSTRUCTION_VERSION}"),
        "choruz-bootstrap-version: 9",
        1,
    )
}

fn extract_role_from_layout<'a>(content_body: &'a str, layout: &str) -> Option<&'a str> {
    let (content_prefix, after_start) = content_body.split_once(ROLE_START)?;
    let (role, content_suffix) = after_start.split_once(ROLE_END)?;
    let (_, layout_body) = parse_version_and_body(layout);
    let (layout_prefix, after_layout_start) = layout_body.split_once(ROLE_START)?;
    let (_, layout_suffix) = after_layout_start.split_once(ROLE_END)?;
    (content_prefix == layout_prefix && content_suffix == layout_suffix).then(|| role.trim())
}

fn extract_managed_role<'a>(path: &Path, content: &'a str) -> Option<&'a str> {
    let (version, content_body) = parse_version_and_body(content);
    let rendered_template = render_instructions(path, ROLE_PLACEHOLDER);
    if let Some(role) = extract_role_from_layout(content_body, &rendered_template) {
        return Some(role);
    }
    if version == Some(7) {
        return extract_role_from_layout(
            content_body,
            &render_v7_instructions(path, ROLE_PLACEHOLDER),
        );
    }
    if version == Some(8) {
        return extract_role_from_layout(
            content_body,
            &render_v8_instructions(path, ROLE_PLACEHOLDER),
        );
    }
    if version == Some(9) {
        return extract_role_from_layout(
            content_body,
            &render_v9_instructions(path, ROLE_PLACEHOLDER),
        );
    }
    let v6_template = match path.file_name().and_then(|name| name.to_str()) {
        Some("AGENTS.md") => V6_CODEX_INSTRUCTIONS_TEMPLATE,
        _ => V6_CLAUDE_INSTRUCTIONS_TEMPLATE,
    };
    if version == Some(6) {
        extract_role_from_layout(content_body, v6_template)
    } else {
        None
    }
}

/// Per-file detail captured when an instruction file is preserved because its
/// content is not a recognised managed template. The aggregate warning
/// sidecar materialises one entry of this shape per preserved file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedEditedEntry {
    pub path: PathBuf,
    pub on_disk_version: Option<u32>,
    pub body_sha256: String,
}

/// Top-level outcome record returned by [`ensure_claude_md`] so tests (and
/// callers that want to surface what happened) can inspect what the refresh
/// did to each instruction file in the workspace. The executor call site
/// ignores the return value — only logs and the sidecar are operator-visible
/// in production.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BootstrapOutcome {
    /// Set when the workspace had no instruction file and a fresh CLAUDE.md
    /// was written.
    pub wrote_fresh: Option<PathBuf>,
    /// Files that were rewritten from a recognised managed template to the
    /// current canonical version while preserving the role block.
    pub refreshed: Vec<PathBuf>,
    /// Files that already carried the current version header and were left
    /// untouched.
    pub already_current: Vec<PathBuf>,
    /// Files that were preserved because the body did not match a known prior
    /// generated hash (operator-edited or pre-version legacy). After
    /// [`ensure_claude_md`] finishes, a single aggregate warning sidecar at
    /// `<workspace>/.choruz-bootstrap-warning.json` lists every entry here so
    /// operators see all preserved files at once instead of only the
    /// last-processed one.
    pub preserved_edited: Vec<PreservedEditedEntry>,
    /// (path, error message) pairs for read/write failures during the refresh.
    /// I/O errors do not abort bootstrapping of other files.
    pub io_errors: Vec<(PathBuf, String)>,
}

/// Ensure a canonical driver-specific instructions file exists in the
/// workspace so the agent knows both its designed role and the Choruz
/// protocol (inbox, outbox, team coordination, channel tasks).
///
/// Behaviour (per ADR-006):
/// - If no instruction file exists in the workspace, write a fresh
///   driver-appropriate file (CLAUDE.md / AGENTS.md selected by
///   `driver_hint`) stamped with the current bootstrap version. `None` selects
///   CLAUDE.md; an unknown driver is rejected without writing a file.
/// - If an instruction file exists with the current version header, leave it
///   alone.
/// - If it is a recognised managed template with an older version header or
///   no header, atomically rewrite it from the canonical driver template while
///   preserving the role block.
/// - Otherwise (operator-edited or unrelated legacy content), preserve it and
///   emit a non-chat warning: a `tracing::warn` log line plus a sidecar at
///   `<workspace>/.choruz-bootstrap-warning.json` so operators can decide
///   whether to run `choruz-pipeline rebootstrap <agent>` to force a rewrite.
///
/// `driver_hint` should be the `agent_runtime_bindings.driver_type` for the
/// agent owning this workspace. Without it, an AGENTS-compatible
/// workspace whose instruction file was deleted between turns (operator
/// cleanup, fresh volume mount, etc.) would silently get a CLAUDE.md that
/// the actual CLI never reads — there is no preserve-path warning sidecar to
/// surface the mismatch, so this hint is the only guard against that
/// silent-failure footgun.
///
/// Recognised names: CLAUDE.md (Claude Code) and AGENTS.md (Codex, Pi, Grok
/// Build, and OpenCode).
pub async fn ensure_claude_md(work_dir: &Path, driver_hint: Option<&str>) -> BootstrapOutcome {
    let mut outcome = BootstrapOutcome::default();
    let primary = match bootstrap_filename_for_driver(driver_hint) {
        Ok(filename) => work_dir.join(filename),
        Err(error) => {
            tracing::error!(driver_hint = driver_hint.unwrap_or("<none>"), %error, "refusing to guess an instruction filename");
            outcome.io_errors.push((work_dir.to_path_buf(), error));
            return outcome;
        }
    };

    let mut existing: Vec<PathBuf> = INSTRUCTION_FILENAMES
        .iter()
        .map(|name| work_dir.join(name))
        .filter(|path| path.exists())
        .collect();

    if existing.is_empty() {
        let fresh = primary;
        match write_bootstrap_atomically(&fresh, DEFAULT_ROLE).await {
            Ok(()) => {
                tracing::info!(
                    path = %fresh.display(),
                    version = BOOTSTRAP_INSTRUCTION_VERSION,
                    driver_hint = driver_hint.unwrap_or("<none>"),
                    "wrote driver-appropriate bootstrap file for choruz protocol"
                );
                clear_bootstrap_warning(work_dir).await;
                outcome.wrote_fresh = Some(fresh);
            }
            Err(e) => {
                tracing::warn!(
                    path = %fresh.display(),
                    error = %e,
                    "failed to write bootstrap instruction file (non-fatal)"
                );
                outcome.io_errors.push((fresh, e.to_string()));
            }
        }
        return outcome;
    }

    // A driver change can also change the filename the CLI reads. When only
    // the alternate instruction file exists, move it to the selected name
    // before refreshing so configuration never remains stranded in a file
    // the active CLI ignores. The rename happens first to preserve even an
    // operator-edited file if rendering the canonical template later fails.
    if known_instruction_filename(driver_hint).is_some() && !primary.exists() && existing.len() == 1
    {
        let alternate = existing.remove(0);
        let managed_role = tokio::fs::read_to_string(&alternate)
            .await
            .ok()
            .and_then(|content| extract_managed_role(&alternate, &content).map(str::to_owned));
        match tokio::fs::rename(&alternate, &primary).await {
            Ok(()) => {
                tracing::info!(
                    from = %alternate.display(),
                    to = %primary.display(),
                    driver_hint = driver_hint.unwrap_or("<none>"),
                    "migrated instruction filename for selected driver"
                );
                if let Some(role) = managed_role {
                    match write_bootstrap_atomically(&primary, &role).await {
                        Ok(()) => {
                            tracing::info!(
                                path = %primary.display(),
                                version = BOOTSTRAP_INSTRUCTION_VERSION,
                                "refreshed canonical instructions after filename migration"
                            );
                            outcome.refreshed.push(primary.clone());
                        }
                        Err(error) => outcome.io_errors.push((primary.clone(), error.to_string())),
                    }
                } else {
                    existing.push(primary.clone());
                }
            }
            Err(error) => {
                outcome
                    .io_errors
                    .push((alternate.clone(), error.to_string()));
                existing.push(alternate);
            }
        }
    }

    for path in existing {
        refresh_one(&path, &mut outcome).await;
    }

    // Drop any stale sidecar from a previous run before deciding whether to
    // re-emit. If every existing file is up to date or got refreshed, the
    // workspace is healthy now and the operator should not chase a problem
    // that no longer applies. If we have preserved entries this turn, the
    // aggregate warning below replaces it with the current set.
    clear_bootstrap_warning(work_dir).await;
    if !outcome.preserved_edited.is_empty() {
        write_bootstrap_warning_aggregate(work_dir, &outcome.preserved_edited).await;
    }

    outcome
}

/// Force-rewrite the bootstrap CLAUDE.md (and, when present, AGENTS.md) in
/// `work_dir` with the current versioned content, ignoring
/// existing edit state. It makes a best-effort backup of each preserved file
/// as `<name>.bak.choruz-rebootstrap` before overwriting so an operator can
/// recover hand-edited content if they ran the command by accident. A backup
/// failure is logged but does not cancel the explicitly requested rewrite.
/// Returns the list of files that were rewritten.
///
/// `driver_hint` selects the default instruction filename when the workspace
/// is empty (no existing CLAUDE.md / AGENTS.md). When the
/// rebootstrap is dispatched via `--principal`, the resolved
/// `agent_runtime_bindings.driver_type` flows in here so a Codex-driven
/// workspace gets `AGENTS.md` instead of being silently bootstrapped as
/// Claude. `None` selects `CLAUDE.md`; an unknown driver is rejected. When the workspace already has
/// at least one instruction file, the hint is ignored — every existing file
/// is rewritten regardless of driver.
///
/// This is the implementation behind `choruz-pipeline rebootstrap <agent>`.
pub async fn force_rewrite_bootstrap(
    work_dir: &Path,
    driver_hint: Option<&str>,
) -> std::io::Result<Vec<PathBuf>> {
    if !work_dir.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("workspace path does not exist: {}", work_dir.display()),
        ));
    }
    if !work_dir.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("workspace path is not a directory: {}", work_dir.display()),
        ));
    }

    // Force-rewrite every instruction file the workspace currently has; if
    // none exists, plant a fresh file using the driver-appropriate name so a
    // AGENTS-compatible agent does not silently end up with a Claude-shaped
    // bootstrap file it will never read.
    let existing: Vec<PathBuf> = INSTRUCTION_FILENAMES
        .iter()
        .map(|name| work_dir.join(name))
        .filter(|path| path.exists())
        .collect();

    let targets: Vec<PathBuf> =
        if existing.is_empty() {
            vec![work_dir.join(
                bootstrap_filename_for_driver(driver_hint).map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, error)
                })?,
            )]
        } else {
            existing
        };

    let mut rewritten = Vec::with_capacity(targets.len());
    for path in targets {
        // Best-effort backup of pre-existing content. We do not abort if the
        // backup fails — operators explicitly opted into a destructive rewrite
        // by running rebootstrap, but a backup gives them a recovery path on
        // the common case.
        if path.exists() {
            if let Ok(existing_bytes) = tokio::fs::read(&path).await {
                let backup_path = path.with_extension(format!(
                    "{}.bak.choruz-rebootstrap",
                    path.extension().and_then(|s| s.to_str()).unwrap_or("md")
                ));
                if let Err(error) = tokio::fs::write(&backup_path, &existing_bytes).await {
                    tracing::warn!(
                        path = %path.display(),
                        backup = %backup_path.display(),
                        %error,
                        "rebootstrap: failed to write backup before overwrite (continuing)"
                    );
                } else {
                    tracing::info!(
                        path = %path.display(),
                        backup = %backup_path.display(),
                        "rebootstrap: wrote backup of prior instruction file"
                    );
                }
            }
        }

        let role = if path.exists() {
            tokio::fs::read_to_string(&path)
                .await
                .ok()
                .and_then(|content| extract_managed_role(&path, &content).map(str::to_owned))
                .unwrap_or_else(|| DEFAULT_ROLE.to_owned())
        } else {
            DEFAULT_ROLE.to_owned()
        };
        write_bootstrap_atomically(&path, &role).await?;
        rewritten.push(path);
    }

    clear_bootstrap_warning(work_dir).await;
    Ok(rewritten)
}

/// Inspect one instruction file and either refresh, preserve, or skip it,
/// recording the outcome. Aggregated warning sidecar writes happen once at
/// the end of [`ensure_claude_md`], not per file.
async fn refresh_one(path: &Path, outcome: &mut BootstrapOutcome) {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "bootstrap: could not read existing instruction file; leaving alone"
            );
            outcome.io_errors.push((path.to_path_buf(), e.to_string()));
            return;
        }
    };

    let (on_disk_version, body) = parse_version_and_body(&content);

    if on_disk_version == Some(BOOTSTRAP_INSTRUCTION_VERSION) {
        outcome.already_current.push(path.to_path_buf());
        return;
    }

    let body_hash = body_sha256_hex(body);
    if let Some(role) = extract_managed_role(path, &content).map(str::to_owned) {
        match write_bootstrap_atomically(path, &role).await {
            Ok(()) => {
                tracing::info!(
                    path = %path.display(),
                    from_version = ?on_disk_version,
                    to_version = BOOTSTRAP_INSTRUCTION_VERSION,
                    body_sha256 = %body_hash,
                    "refreshed canonical choruz instructions while preserving the agent role"
                );
                outcome.refreshed.push(path.to_path_buf());
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "bootstrap: failed to refresh instruction file"
                );
                outcome.io_errors.push((path.to_path_buf(), e.to_string()));
            }
        }
        return;
    }

    // Edited or pre-version legacy — preserve and record for the aggregate
    // sidecar emitted at the end of ensure_claude_md. The log line per file
    // stays per-file because it is what makes the operator chase the sidecar.
    tracing::warn!(
        path = %path.display(),
        on_disk_version = ?on_disk_version,
        current_version = BOOTSTRAP_INSTRUCTION_VERSION,
        body_sha256 = %body_hash,
        "skipping choruz bootstrap refresh: file is not the current Choruz body; run \
         `choruz-pipeline rebootstrap --workspace <path>` or \
         `choruz-pipeline rebootstrap --principal <agent-principal-id>` \
         to force a rewrite (see .choruz-bootstrap-warning.json)"
    );
    outcome.preserved_edited.push(PreservedEditedEntry {
        path: path.to_path_buf(),
        on_disk_version,
        body_sha256: body_hash,
    });
}

/// Atomically replace `path` with the current versioned bootstrap content via
/// a same-directory temp file + rename. Same-dir + rename is POSIX-atomic and
/// avoids leaving a half-written instruction file mid-write.
async fn write_bootstrap_atomically(path: &Path, role: &str) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let tmp_name = format!(
        ".{}.partial-{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("bootstrap"),
        choruz_ids::MessageId::new()
    );
    let tmp_path = parent.join(tmp_name);
    if let Err(e) = tokio::fs::write(&tmp_path, render_instructions(path, role)).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(e);
    }
    if let Err(e) = tokio::fs::rename(&tmp_path, path).await {
        // Best-effort cleanup so a long-running workspace does not accumulate
        // hidden `.<name>.partial-*` files across repeated rename failures.
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(e);
    }
    Ok(())
}

async fn write_bootstrap_warning_aggregate(work_dir: &Path, skipped: &[PreservedEditedEntry]) {
    let warning_path = work_dir.join(BOOTSTRAP_WARNING_FILENAME);
    let skipped_payload: Vec<_> = skipped
        .iter()
        .map(|entry| {
            serde_json::json!({
                "path": entry.path.display().to_string(),
                "on_disk_version": entry.on_disk_version,
                "body_sha256": entry.body_sha256,
            })
        })
        .collect();
    let envelope = serde_json::json!({
        "kind": "bootstrap_refresh_skipped",
        "current_version": BOOTSTRAP_INSTRUCTION_VERSION,
        "skipped": skipped_payload,
        "remediation": format!(
            "Run `choruz-pipeline rebootstrap --workspace {}` (or use `--principal <agent>` to \
             look up the path) to force-overwrite every preserved file listed above. Existing \
             files are left untouched; rebootstrap attempts a best-effort backup before overwrite.",
            work_dir.display()
        ),
        "emitted_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    });
    if let Err(error) = tokio::fs::write(&warning_path, envelope.to_string()).await {
        tracing::warn!(
            warning_path = %warning_path.display(),
            %error,
            "bootstrap: failed to write warning sidecar"
        );
    }
}

async fn clear_bootstrap_warning(work_dir: &Path) {
    let warning_path = work_dir.join(BOOTSTRAP_WARNING_FILENAME);
    if warning_path.exists() {
        if let Err(error) = tokio::fs::remove_file(&warning_path).await {
            tracing::debug!(
                path = %warning_path.display(),
                %error,
                "bootstrap: could not remove stale warning sidecar (non-fatal)"
            );
        }
    }
}

/// Parse the version header off the start of the file and return
/// (version_if_present, body_without_header). The body never includes the
/// header line or the single blank line that follows it; callers can hash it
/// directly.
fn parse_version_and_body(content: &str) -> (Option<u32>, &str) {
    let first_line_end = match content.find('\n') {
        Some(idx) => idx,
        None => return (None, content),
    };
    let first_line = &content[..first_line_end];

    let after_prefix = match first_line.strip_prefix(BOOTSTRAP_VERSION_HEADER_PREFIX) {
        Some(s) => s,
        None => return (None, content),
    };
    let version_str = match after_prefix.strip_suffix(BOOTSTRAP_VERSION_HEADER_SUFFIX) {
        Some(s) => s.trim(),
        None => return (None, content),
    };
    let version = match version_str.parse::<u32>() {
        Ok(v) => v,
        Err(_) => return (None, content),
    };

    // Body starts after the header line; skip exactly one optional blank line
    // (so writing "<header>\n\nbody" and "<header>\nbody" both round-trip).
    let mut body_start = first_line_end + 1;
    if content.as_bytes().get(body_start) == Some(&b'\n') {
        body_start += 1;
    }
    (Some(version), &content[body_start..])
}

fn body_sha256_hex(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    hex::encode(hasher.finalize())
}

/// Run the `choruz-pipeline rebootstrap` subcommand. Returns the process exit
/// code; main.rs calls this and propagates the code to `std::process::exit`.
///
/// Usage:
///   choruz-pipeline rebootstrap --workspace <path>
///   choruz-pipeline rebootstrap --principal <agent-principal-id>
///   choruz-pipeline rebootstrap <agent-principal-id>
pub async fn run_rebootstrap_command(args: Vec<String>) -> i32 {
    let mut workspace: Option<PathBuf> = None;
    let mut principal: Option<String> = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--workspace" | "-w" => match iter.next() {
                Some(v) => workspace = Some(PathBuf::from(v)),
                None => {
                    eprintln!("rebootstrap: --workspace requires a value");
                    return 2;
                }
            },
            "--principal" | "-p" | "--agent" => match iter.next() {
                Some(v) => principal = Some(v),
                None => {
                    eprintln!("rebootstrap: --principal requires a value");
                    return 2;
                }
            },
            "-h" | "--help" => {
                print_rebootstrap_help();
                return 0;
            }
            other if !other.starts_with('-') && principal.is_none() && workspace.is_none() => {
                // Positional: treat as agent principal id (matches the
                // ADR-006 `choruz-pipeline rebootstrap <agent>` phrasing).
                principal = Some(other.to_string());
            }
            other => {
                eprintln!("rebootstrap: unrecognised argument: {other}");
                print_rebootstrap_help();
                return 2;
            }
        }
    }

    let (work_dir, driver_hint) = match (workspace, principal) {
        (Some(p), _) => (p, None),
        (None, Some(principal_id)) => match resolve_workspace_path(&principal_id).await {
            Ok((path, driver)) => (path, driver),
            Err(error) => {
                eprintln!("rebootstrap: {error}");
                return 1;
            }
        },
        (None, None) => {
            eprintln!("rebootstrap: must specify --workspace <path> or <agent-principal-id>");
            print_rebootstrap_help();
            return 2;
        }
    };

    match force_rewrite_bootstrap(&work_dir, driver_hint.as_deref()).await {
        Ok(rewritten) => {
            let report = serde_json::json!({
                "ok": true,
                "workspace": work_dir.display().to_string(),
                "version": BOOTSTRAP_INSTRUCTION_VERSION,
                "rewritten": rewritten.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            });
            println!("{report}");
            0
        }
        Err(error) => {
            eprintln!("rebootstrap: failed to rewrite bootstrap: {error}");
            1
        }
    }
}

fn print_rebootstrap_help() {
    eprintln!(
        "Usage: choruz-pipeline rebootstrap [OPTIONS] [<agent-principal-id>]\n\
         \n\
         Force-rewrite the channel-task bootstrap instructions (CLAUDE.md /\n\
         AGENTS.md) in an agent workspace. Pre-existing files are\n\
         given a best-effort backup at <name>.<ext>.bak.choruz-rebootstrap\n\
         before overwrite; backup failure does not cancel the rewrite.\n\
         \n\
         Options:\n\
           -w, --workspace <PATH>         Target workspace directory directly.\n\
           -p, --principal <PRINCIPAL>    Agent principal id; resolves the\n\
                                          workspace_path via agent_runtime_bindings.\n\
                                          (Also accepts `--agent`.)\n\
           -h, --help                     Show this help.\n\
         \n\
         A positional argument is interpreted as <agent-principal-id> when no\n\
         --workspace / --principal is given.\n\
         \n\
         Environment:\n\
           CHORUZ_DATABASE_URL  Used by --principal to look up the workspace path."
    );
}

/// Look up the workspace path and (when set) driver_type for an agent
/// principal id via the event store. Picks the most-recently-updated active
/// binding so multi-conversation agents resolve to the canonical workspace.
/// The driver hint lets [`force_rewrite_bootstrap`] pick the right default
/// instruction filename when the workspace has no existing one.
async fn resolve_workspace_path(principal_id: &str) -> Result<(PathBuf, Option<String>), String> {
    let database_url = std::env::var("CHORUZ_DATABASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| choruz_common::PgConfig::from_env().to_connect_string());
    let store = choruz_store::EventStore::new(&database_url);
    let client = store
        .connect()
        .await
        .map_err(|e| format!("could not connect to event store: {e}"))?;
    let row = client
        .query_opt(
            "SELECT workspace_path, driver_type \
             FROM agent_runtime_bindings \
             WHERE agent_principal_id = $1 \
               AND workspace_path IS NOT NULL \
               AND workspace_path <> '' \
               AND state NOT IN ('disabled', 'paused', 'error') \
             ORDER BY updated_at DESC NULLS LAST, id DESC \
             LIMIT 1",
            &[&principal_id],
        )
        .await
        .map_err(|e| format!("workspace lookup failed: {e}"))?
        .ok_or_else(|| {
            format!(
                "no active agent_runtime_bindings row (state not in disabled/paused/error) \
                 with non-empty workspace_path for principal {principal_id}"
            )
        })?;
    let workspace_path: String = row.get(0);
    let driver_type: Option<String> = row.get(1);
    Ok((PathBuf::from(workspace_path), driver_type))
}

#[cfg(test)]
mod tests {
    use super::{
        BOOTSTRAP_INSTRUCTION_VERSION, BOOTSTRAP_VERSION_HEADER_PREFIX,
        BOOTSTRAP_VERSION_HEADER_SUFFIX, BOOTSTRAP_WARNING_FILENAME, DEFAULT_ROLE,
        V6_CLAUDE_INSTRUCTIONS_TEMPLATE, V6_CODEX_INSTRUCTIONS_TEMPLATE, body_sha256_hex,
        ensure_claude_md, force_rewrite_bootstrap, parse_version_and_body, render_instructions,
        render_v7_instructions, render_v8_instructions, render_v9_instructions,
    };

    /// Versioned Choruz bootstrap fixtures used to verify explicit
    /// rebootstrap behavior across supported historical versions.
    const V1_BOOTSTRAP_FROZEN: &str = include_str!("instructions_fixtures/bootstrap-v1.md");
    const V2_BOOTSTRAP_FROZEN: &str = include_str!("instructions_fixtures/bootstrap-v2.md");
    const V3_BOOTSTRAP_FROZEN: &str = include_str!("instructions_fixtures/bootstrap-v3.md");
    const V4_BOOTSTRAP_FROZEN: &str = include_str!("instructions_fixtures/bootstrap-v4.md");
    use std::path::Path;
    use std::sync::OnceLock;
    use tempfile::TempDir;

    fn canonical_claude_md() -> &'static str {
        static CONTENT: OnceLock<String> = OnceLock::new();
        CONTENT
            .get_or_init(|| render_instructions(Path::new("CLAUDE.md"), DEFAULT_ROLE))
            .as_str()
    }

    fn canonical_claude_md_body() -> &'static str {
        parse_version_and_body(canonical_claude_md()).1
    }

    fn canonical_instructions(filename: &str, role: &str) -> String {
        render_instructions(Path::new(filename), role)
    }

    fn canonical_body(filename: &str, role: &str) -> String {
        parse_version_and_body(&canonical_instructions(filename, role))
            .1
            .to_owned()
    }

    fn v6_instructions(filename: &str, role: &str) -> String {
        let template = if filename == "AGENTS.md" {
            V6_CODEX_INSTRUCTIONS_TEMPLATE
        } else {
            V6_CLAUDE_INSTRUCTIONS_TEMPLATE
        };
        template.replace("{{AGENT_INSTRUCTIONS}}", role)
    }

    fn v7_instructions(filename: &str, role: &str) -> String {
        render_v7_instructions(Path::new(filename), role)
    }

    fn v8_instructions(filename: &str, role: &str) -> String {
        render_v8_instructions(Path::new(filename), role)
    }

    fn v9_instructions(filename: &str, role: &str) -> String {
        render_v9_instructions(Path::new(filename), role)
    }

    fn current_header() -> String {
        format!(
            "{}{}{}",
            BOOTSTRAP_VERSION_HEADER_PREFIX,
            BOOTSTRAP_INSTRUCTION_VERSION,
            BOOTSTRAP_VERSION_HEADER_SUFFIX
        )
    }

    fn markdown_section(md: &str, heading: &str) -> String {
        let marker = format!("## {heading}");
        let lines: Vec<&str> = md.lines().collect();
        let start = lines
            .iter()
            .position(|line| *line == marker)
            .unwrap_or_else(|| panic!("missing markdown section `{heading}`"));
        let end = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find_map(|(index, line)| line.starts_with("## ").then_some(index))
            .unwrap_or(lines.len());
        lines[start..end].join("\n")
    }

    async fn read_string(p: &Path) -> String {
        tokio::fs::read_to_string(p).await.unwrap()
    }

    #[tokio::test]
    async fn bootstrap_writes_fresh_when_no_instruction_file_exists() {
        let dir = TempDir::new().unwrap();
        let outcome = ensure_claude_md(dir.path(), None).await;

        let claude = dir.path().join("CLAUDE.md");
        assert!(claude.exists(), "fresh CLAUDE.md must be written");
        assert_eq!(outcome.wrote_fresh.as_deref(), Some(claude.as_path()));
        assert!(outcome.refreshed.is_empty());
        assert!(outcome.preserved_edited.is_empty());
        assert!(outcome.io_errors.is_empty());

        let written = read_string(&claude).await;
        assert!(
            written.starts_with(&current_header()),
            "freshly bootstrapped file must be stamped with current version header: {written:?}"
        );
        assert!(
            !dir.path().join(BOOTSTRAP_WARNING_FILENAME).exists(),
            "fresh write must not leave a warning sidecar"
        );
    }

    #[tokio::test]
    async fn bootstrap_uses_driver_hint_in_empty_workspace_per_turn() {
        // Per-turn refresh path: an empty Codex workspace (e.g. operator
        // cleanup between turns, fresh volume mount) must NOT silently get a
        // CLAUDE.md the running CLI never reads. The executor knows the
        // driver_type and passes it in here; without that hint the fresh-write
        // branch would default to CLAUDE.md and no warning sidecar would
        // surface the mismatch — the warning machinery only fires on the
        // preserve path, not on fresh writes.
        for (driver, expected, other) in [
            ("codex_terminal", "AGENTS.md", &["CLAUDE.md"][..]),
            ("codex_exec", "AGENTS.md", &["CLAUDE.md"][..]),
            ("codex_app_server", "AGENTS.md", &["CLAUDE.md"][..]),
            ("pi_terminal", "AGENTS.md", &["CLAUDE.md"][..]),
            ("grok_terminal", "AGENTS.md", &["CLAUDE.md"][..]),
            ("opencode_terminal", "AGENTS.md", &["CLAUDE.md"][..]),
            ("claude_terminal", "CLAUDE.md", &["AGENTS.md"][..]),
            ("claude_print", "CLAUDE.md", &["AGENTS.md"][..]),
        ] {
            let dir = TempDir::new().unwrap();
            let outcome = ensure_claude_md(dir.path(), Some(driver)).await;

            let expected_path = dir.path().join(expected);
            assert_eq!(
                outcome.wrote_fresh.as_deref(),
                Some(expected_path.as_path()),
                "{driver}: per-turn empty-workspace bootstrap must plant {expected}"
            );
            assert!(
                expected_path.exists(),
                "{driver}: expected {expected} on disk"
            );
            for sibling in other {
                assert!(
                    !dir.path().join(sibling).exists(),
                    "{driver}: must not plant {sibling} alongside {expected}"
                );
            }
        }
    }

    #[tokio::test]
    async fn bootstrap_migrates_managed_instructions_when_driver_filename_changes() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        let agents = dir.path().join("AGENTS.md");
        tokio::fs::write(
            &claude,
            canonical_instructions("CLAUDE.md", "Own the release checklist."),
        )
        .await
        .unwrap();

        let outcome = ensure_claude_md(dir.path(), Some("pi_terminal")).await;

        assert!(!claude.exists(), "the inactive filename must be removed");
        assert!(agents.exists(), "the selected driver must get AGENTS.md");
        assert_eq!(outcome.refreshed, vec![agents.clone()]);
        let content = read_string(&agents).await;
        assert!(content.contains("Own the release checklist."));
        assert_eq!(
            content,
            canonical_instructions("AGENTS.md", "Own the release checklist.")
        );
    }

    #[tokio::test]
    async fn bootstrap_moves_edited_instructions_without_overwriting_them() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        let agents = dir.path().join("AGENTS.md");
        let edited = "Operator-owned instructions\nDo not rewrite this file.\n";
        tokio::fs::write(&claude, edited).await.unwrap();

        let outcome = ensure_claude_md(dir.path(), Some("opencode_terminal")).await;

        assert!(!claude.exists());
        assert_eq!(read_string(&agents).await, edited);
        assert_eq!(outcome.preserved_edited.len(), 1);
        assert_eq!(outcome.preserved_edited[0].path, agents);
    }

    #[tokio::test]
    async fn bootstrap_unknown_driver_hint_is_rejected_without_writing_a_file() {
        let dir = TempDir::new().unwrap();
        let outcome = ensure_claude_md(dir.path(), Some("mystery_driver_v9")).await;
        assert!(outcome.wrote_fresh.is_none());
        assert_eq!(outcome.io_errors.len(), 1);
        assert!(!dir.path().join("CLAUDE.md").exists());
        assert!(!dir.path().join("AGENTS.md").exists());
    }

    #[tokio::test]
    async fn bootstrap_leaves_already_current_file_alone() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        tokio::fs::write(&claude, canonical_claude_md())
            .await
            .unwrap();
        let mtime_before = tokio::fs::metadata(&claude).await.unwrap().modified().ok();

        let outcome = ensure_claude_md(dir.path(), None).await;

        assert!(outcome.wrote_fresh.is_none());
        assert_eq!(outcome.already_current, vec![claude.clone()]);
        assert!(outcome.refreshed.is_empty());
        assert!(outcome.preserved_edited.is_empty());
        let mtime_after = tokio::fs::metadata(&claude).await.unwrap().modified().ok();
        // Best-effort: rewriting would bump mtime; assert content unchanged either way.
        let on_disk = read_string(&claude).await;
        assert_eq!(on_disk, canonical_claude_md());
        if let (Some(before), Some(after)) = (mtime_before, mtime_after) {
            assert_eq!(before, after, "already-current file must not be rewritten");
        }
    }

    #[tokio::test]
    async fn bootstrap_refreshes_prior_generated_content() {
        // Cover the case where a workspace carries the *current* body without
        // its version header — e.g. a hand re-init from a copy/paste of the
        // template body. Managed role markers let the pipeline restore the
        // version header without maintaining a second prompt body.
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        let prior_unstamped = canonical_claude_md_body();
        tokio::fs::write(&claude, prior_unstamped).await.unwrap();

        let outcome = ensure_claude_md(dir.path(), None).await;

        assert!(outcome.wrote_fresh.is_none());
        assert_eq!(outcome.refreshed, vec![claude.clone()]);
        assert!(outcome.preserved_edited.is_empty());
        let after = read_string(&claude).await;
        assert!(
            after.starts_with(&current_header()),
            "refreshed file must carry the current version header: {after:?}"
        );
        assert_eq!(after, canonical_claude_md());
        assert!(
            !dir.path().join(BOOTSTRAP_WARNING_FILENAME).exists(),
            "successful refresh must not leave a warning sidecar"
        );
    }

    #[tokio::test]
    async fn bootstrap_preserves_prior_version_for_explicit_rebootstrap() {
        // Real rollout coverage: a workspace that was bootstrapped by the v1
        // pipeline carries the frozen v1 content (header included). The body
        // clean-breaking runtime must preserve it for explicit rebootstrap.
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        tokio::fs::write(&claude, V1_BOOTSTRAP_FROZEN)
            .await
            .unwrap();

        let outcome = ensure_claude_md(dir.path(), None).await;

        assert!(outcome.wrote_fresh.is_none());
        assert!(outcome.refreshed.is_empty());
        assert_eq!(outcome.preserved_edited.len(), 1);
        assert_eq!(outcome.preserved_edited[0].path, claude);
        assert_eq!(outcome.preserved_edited[0].on_disk_version, Some(1));
        let after = read_string(&claude).await;
        assert_eq!(after, V1_BOOTSTRAP_FROZEN);
        assert!(dir.path().join(BOOTSTRAP_WARNING_FILENAME).exists());
    }

    #[test]
    fn bootstrap_v1_fixture_uses_choruz_protocol() {
        // Pin: the frozen v1 fixture's header-stripped body must hash to the
        // first entry of KNOWN_PRIOR_BODY_HASHES (the v1 slot). If either side
        // drifts, the v1→v2 auto-refresh test will silently stop covering real
        // workspaces. Fail loud here instead of silently leaving the rollout
        // gap open.
        let (version, body) = parse_version_and_body(V1_BOOTSTRAP_FROZEN);
        assert_eq!(
            version,
            Some(1),
            "fixture must carry the v1 header so it represents real on-disk v1 content"
        );
        assert!(body.contains("[choruz-incoming]"));
    }

    #[test]
    fn bootstrap_v2_fixture_uses_choruz_protocol() {
        // Pin: same guard as the v1 variant, but for the v2 slot. Bumping to
        // BOOTSTRAP_INSTRUCTION_VERSION=3 added v2 to the append-only ledger;
        // if the fixture and the hash ever drift apart, v2→v3 auto-refresh
        // would silently miss every workspace still stamped at v2.
        let (version, body) = parse_version_and_body(V2_BOOTSTRAP_FROZEN);
        assert_eq!(
            version,
            Some(2),
            "fixture must carry the v2 header so it represents real on-disk v2 content"
        );
        assert!(body.contains("[choruz-incoming]"));
    }

    #[test]
    fn bootstrap_v3_fixture_uses_choruz_protocol() {
        // Pin: same guard as the v1/v2 variants, for the v3 slot. Bumping to
        // BOOTSTRAP_INSTRUCTION_VERSION=4 (message-threads docs) added v3 to
        // the append-only ledger; if the fixture and the hash drift apart,
        // v3→v4 auto-refresh would silently miss every workspace still
        // stamped at v3.
        let (version, body) = parse_version_and_body(V3_BOOTSTRAP_FROZEN);
        assert_eq!(
            version,
            Some(3),
            "fixture must carry the v3 header so it represents real on-disk v3 content"
        );
        assert!(body.contains("[choruz-incoming]"));
    }

    #[test]
    fn bootstrap_v4_fixture_uses_choruz_protocol() {
        // Pin: same guard as the v1-v3 variants, for the v4 slot. Bumping to
        // BOOTSTRAP_INSTRUCTION_VERSION=5 (broadcast-default clarification)
        // added v4 to the append-only ledger.
        let (version, body) = parse_version_and_body(V4_BOOTSTRAP_FROZEN);
        assert_eq!(
            version,
            Some(4),
            "fixture must carry the v4 header so it represents real on-disk v4 content"
        );
        assert!(body.contains("[choruz-incoming]"));
    }

    #[tokio::test]
    async fn bootstrap_preserves_edited_file_and_writes_warning() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        let edited = "# Operator hand-edited instructions\n\nDo not overwrite me.\n";
        tokio::fs::write(&claude, edited).await.unwrap();

        let outcome = ensure_claude_md(dir.path(), None).await;

        assert!(outcome.refreshed.is_empty());
        assert_eq!(outcome.preserved_edited.len(), 1);
        assert_eq!(outcome.preserved_edited[0].path, claude);
        assert_eq!(outcome.preserved_edited[0].on_disk_version, None);
        assert_eq!(read_string(&claude).await, edited);

        let warning_path = dir.path().join(BOOTSTRAP_WARNING_FILENAME);
        assert!(
            warning_path.exists(),
            "edited-file preservation must surface a warning sidecar"
        );
        let warning_body = read_string(&warning_path).await;
        let envelope: serde_json::Value = serde_json::from_str(&warning_body).unwrap();
        assert_eq!(envelope["kind"], "bootstrap_refresh_skipped");
        assert_eq!(envelope["current_version"], BOOTSTRAP_INSTRUCTION_VERSION);
        assert!(
            envelope["remediation"]
                .as_str()
                .unwrap_or("")
                .contains("rebootstrap")
        );
        let skipped = envelope["skipped"]
            .as_array()
            .expect("sidecar must list preserved files under `skipped`");
        assert_eq!(
            skipped.len(),
            1,
            "single-edited-file workspace must surface exactly one entry"
        );
        assert_eq!(skipped[0]["path"], claude.display().to_string());
        assert!(skipped[0]["on_disk_version"].is_null());
        assert!(skipped[0]["body_sha256"].is_string());
        assert!(envelope["emitted_at"].is_string());
    }

    #[tokio::test]
    async fn bootstrap_clears_stale_warning_when_files_are_clean() {
        let dir = TempDir::new().unwrap();
        // Pre-existing warning sidecar from an earlier run; everything is now
        // current, so the bootstrapper should sweep it away.
        tokio::fs::write(dir.path().join(BOOTSTRAP_WARNING_FILENAME), "{}")
            .await
            .unwrap();
        let claude = dir.path().join("CLAUDE.md");
        tokio::fs::write(&claude, canonical_claude_md())
            .await
            .unwrap();

        let _ = ensure_claude_md(dir.path(), None).await;

        assert!(
            !dir.path().join(BOOTSTRAP_WARNING_FILENAME).exists(),
            "stale warning must be cleared once the workspace is healthy again"
        );
    }

    #[tokio::test]
    async fn bootstrap_refreshes_each_driver_filename() {
        // ADR-006 must heal an unstamped canonical body using the template
        // for the CLI that owns each filename.
        for filename in ["AGENTS.md"] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(filename);
            let expected = canonical_instructions(filename, DEFAULT_ROLE);
            tokio::fs::write(&path, canonical_body(filename, DEFAULT_ROLE))
                .await
                .unwrap();

            let outcome = ensure_claude_md(dir.path(), None).await;

            assert!(
                outcome.wrote_fresh.is_none(),
                "{filename}: existing file must not trigger fresh-write path"
            );
            assert_eq!(
                outcome.refreshed,
                vec![path.clone()],
                "{filename}: existing managed body must be refreshed in place"
            );
            assert!(
                outcome.preserved_edited.is_empty(),
                "{filename}: managed body must not be flagged as edited"
            );
            assert_eq!(
                read_string(&path).await,
                expected,
                "{filename}: post-refresh contents must be the current stamped body"
            );
            assert!(
                !dir.path().join("CLAUDE.md").exists(),
                "{filename}: refresh must rewrite in place, not plant CLAUDE.md alongside"
            );
            assert!(
                !dir.path().join(BOOTSTRAP_WARNING_FILENAME).exists(),
                "{filename}: successful refresh must not leave a warning sidecar"
            );
        }
    }

    #[tokio::test]
    async fn bootstrap_refreshes_only_when_managed_layout_outside_role_is_unchanged() {
        let role = "You are the release reviewer.";

        let clean_dir = TempDir::new().unwrap();
        let clean_path = clean_dir.path().join("CLAUDE.md");
        let prior_header = "<!-- choruz-bootstrap-version: 5 -->";
        let prior_managed =
            canonical_instructions("CLAUDE.md", role).replacen(&current_header(), prior_header, 1);
        tokio::fs::write(&clean_path, prior_managed).await.unwrap();

        let clean_outcome = ensure_claude_md(clean_dir.path(), None).await;
        assert_eq!(clean_outcome.refreshed, vec![clean_path.clone()]);
        assert_eq!(
            read_string(&clean_path).await,
            canonical_instructions("CLAUDE.md", role)
        );

        let edited_dir = TempDir::new().unwrap();
        let edited_path = edited_dir.path().join("CLAUDE.md");
        let edited = canonical_instructions("CLAUDE.md", role)
            .replacen(&current_header(), prior_header, 1)
            .replacen(
                "Incoming messages use `[choruz-incoming] METADATA | BODY`.",
                "Operator-owned platform edit.",
                1,
            );
        tokio::fs::write(&edited_path, &edited).await.unwrap();

        let edited_outcome = ensure_claude_md(edited_dir.path(), None).await;
        assert!(edited_outcome.refreshed.is_empty());
        assert_eq!(edited_outcome.preserved_edited.len(), 1);
        assert_eq!(read_string(&edited_path).await, edited);
        assert!(edited_dir.path().join(BOOTSTRAP_WARNING_FILENAME).exists());
    }

    #[tokio::test]
    async fn bootstrap_refreshes_v6_managed_files_without_losing_roles() {
        for filename in ["CLAUDE.md", "AGENTS.md"] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(filename);
            let role = format!("You are the carefully designed {filename} reviewer.");
            tokio::fs::write(&path, v6_instructions(filename, &role))
                .await
                .unwrap();

            let outcome = ensure_claude_md(dir.path(), None).await;
            assert_eq!(outcome.refreshed, vec![path.clone()]);
            assert_eq!(
                read_string(&path).await,
                canonical_instructions(filename, &role)
            );
        }
    }

    #[tokio::test]
    async fn bootstrap_refreshes_v7_managed_files_without_losing_roles() {
        for filename in ["CLAUDE.md", "AGENTS.md"] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(filename);
            let role = format!("You are the carefully designed {filename} reviewer.");
            tokio::fs::write(&path, v7_instructions(filename, &role))
                .await
                .unwrap();

            let outcome = ensure_claude_md(dir.path(), None).await;
            assert_eq!(outcome.refreshed, vec![path.clone()]);
            assert!(outcome.preserved_edited.is_empty());
            assert_eq!(
                read_string(&path).await,
                canonical_instructions(filename, &role)
            );
        }
    }

    #[tokio::test]
    async fn bootstrap_refreshes_v8_managed_files_without_losing_roles() {
        for filename in ["CLAUDE.md", "AGENTS.md"] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(filename);
            let role = format!("You are the carefully designed {filename} reviewer.");
            tokio::fs::write(&path, v8_instructions(filename, &role))
                .await
                .unwrap();

            let outcome = ensure_claude_md(dir.path(), None).await;
            assert_eq!(outcome.refreshed, vec![path.clone()]);
            assert!(outcome.preserved_edited.is_empty());
            assert_eq!(
                read_string(&path).await,
                canonical_instructions(filename, &role)
            );
        }
    }

    #[tokio::test]
    async fn bootstrap_refreshes_v9_managed_files_without_losing_roles() {
        for filename in ["CLAUDE.md", "AGENTS.md"] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(filename);
            let role = format!("You are the carefully designed {filename} reviewer.");
            tokio::fs::write(&path, v9_instructions(filename, &role))
                .await
                .unwrap();

            let outcome = ensure_claude_md(dir.path(), None).await;
            assert_eq!(outcome.refreshed, vec![path.clone()]);
            assert!(outcome.preserved_edited.is_empty());
            assert_eq!(
                read_string(&path).await,
                canonical_instructions(filename, &role)
            );
        }
    }

    #[tokio::test]
    async fn bootstrap_preserves_edited_v7_platform_instructions() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        let edited = v7_instructions("AGENTS.md", "You are the release reviewer.").replacen(
            "Use `@agent-name` for one visible agent",
            "Operator-owned collaboration behavior. Use `@agent-name` for one visible agent",
            1,
        );
        tokio::fs::write(&path, &edited).await.unwrap();

        let outcome = ensure_claude_md(dir.path(), None).await;
        assert!(outcome.refreshed.is_empty());
        assert_eq!(outcome.preserved_edited.len(), 1);
        assert_eq!(read_string(&path).await, edited);
    }

    #[tokio::test]
    async fn force_rewrite_preserves_v6_managed_role() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CLAUDE.md");
        let role = "You are the release verifier. Never merge without evidence.";
        tokio::fs::write(&path, v6_instructions("CLAUDE.md", role))
            .await
            .unwrap();

        force_rewrite_bootstrap(dir.path(), None).await.unwrap();
        assert_eq!(
            read_string(&path).await,
            canonical_instructions("CLAUDE.md", role)
        );
    }

    #[tokio::test]
    async fn bootstrap_preserves_multi_driver_workspace_with_one_edited() {
        // Dual-driver workspace: CLAUDE.md carries the known v1 body
        // (refreshable) while AGENTS.md is operator-edited (must be
        // preserved). The aggregate sidecar must surface BOTH the refreshed
        // and the preserved facts via the new `skipped` array — the pre-fix
        // bug overwrote the sidecar once per loop iteration, so a workspace
        // with two preserved files only ever surfaced the last one.
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        let agents = dir.path().join("AGENTS.md");
        let edited_agents = "# Codex hand-edit\n\nDo not overwrite me.\n";
        tokio::fs::write(&claude, canonical_claude_md_body())
            .await
            .unwrap();
        tokio::fs::write(&agents, edited_agents).await.unwrap();

        let outcome = ensure_claude_md(dir.path(), None).await;

        assert_eq!(outcome.refreshed, vec![claude.clone()]);
        assert_eq!(
            outcome.preserved_edited.len(),
            1,
            "only AGENTS.md is edited; CLAUDE.md matched a known prior body"
        );
        assert_eq!(outcome.preserved_edited[0].path, agents);
        assert_eq!(read_string(&agents).await, edited_agents);

        let warning_path = dir.path().join(BOOTSTRAP_WARNING_FILENAME);
        assert!(
            warning_path.exists(),
            "preserved file must surface a sidecar"
        );
        let envelope: serde_json::Value =
            serde_json::from_str(&read_string(&warning_path).await).unwrap();
        let skipped = envelope["skipped"]
            .as_array()
            .expect("aggregate sidecar must list a `skipped` array");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0]["path"], agents.display().to_string());
    }

    #[tokio::test]
    async fn bootstrap_aggregates_sidecar_across_both_edited_files() {
        // Both CLAUDE.md and AGENTS.md are operator-edited. The aggregate
        // sidecar must list BOTH paths in `skipped`. This is the specific
        // regression the per-file write_bootstrap_warning bug masked: the
        // last-processed filename was the only one surfaced.
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        let agents = dir.path().join("AGENTS.md");
        let edited_claude = "# Hand-edited Claude\n\nleave me\n";
        let edited_agents = "# Hand-edited Codex\n\nleave me too\n";
        tokio::fs::write(&claude, edited_claude).await.unwrap();
        tokio::fs::write(&agents, edited_agents).await.unwrap();

        let outcome = ensure_claude_md(dir.path(), None).await;

        assert!(outcome.refreshed.is_empty());
        assert_eq!(outcome.preserved_edited.len(), 2);
        assert_eq!(read_string(&claude).await, edited_claude);
        assert_eq!(read_string(&agents).await, edited_agents);

        let envelope: serde_json::Value =
            serde_json::from_str(&read_string(&dir.path().join(BOOTSTRAP_WARNING_FILENAME)).await)
                .unwrap();
        let skipped = envelope["skipped"].as_array().expect("`skipped` array");
        assert_eq!(
            skipped.len(),
            2,
            "aggregate sidecar must surface every preserved file, not just the last one"
        );
        let paths: std::collections::HashSet<String> = skipped
            .iter()
            .filter_map(|e| e["path"].as_str().map(|s| s.to_string()))
            .collect();
        assert!(paths.contains(&claude.display().to_string()));
        assert!(paths.contains(&agents.display().to_string()));
    }

    #[tokio::test]
    async fn force_rewrite_backs_up_each_driver_filename_with_correct_extension() {
        // Pins the backup name for non-CLAUDE drivers. A bug that hardcoded
        // "CLAUDE" in the backup name would slip past the CLAUDE.md test.
        for filename in ["AGENTS.md"] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(filename);
            let edited = format!("# Operator-edited {filename}\nkeep me\n");
            tokio::fs::write(&path, &edited).await.unwrap();

            let rewritten = force_rewrite_bootstrap(dir.path(), None).await.unwrap();
            assert_eq!(rewritten, vec![path.clone()]);
            assert_eq!(
                read_string(&path).await,
                canonical_instructions(filename, DEFAULT_ROLE)
            );

            let backup = dir
                .path()
                .join(format!("{filename}.bak.choruz-rebootstrap"));
            assert!(
                backup.exists(),
                "{filename}: backup must use the original filename, not CLAUDE.md"
            );
            assert_eq!(read_string(&backup).await, edited);
        }
    }

    #[tokio::test]
    async fn force_rewrite_rewrites_edited_file_with_backup() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        let edited = "# Operator override\nKeep this around for me.\n";
        tokio::fs::write(&claude, edited).await.unwrap();

        // Stale warning from a prior auto-refresh skip.
        tokio::fs::write(dir.path().join(BOOTSTRAP_WARNING_FILENAME), "{}")
            .await
            .unwrap();

        let rewritten = force_rewrite_bootstrap(dir.path(), None).await.unwrap();
        assert_eq!(rewritten, vec![claude.clone()]);
        assert_eq!(read_string(&claude).await, canonical_claude_md());

        // Backup of the prior content must exist so an operator can recover.
        let backup = dir.path().join("CLAUDE.md.bak.choruz-rebootstrap");
        assert!(
            backup.exists(),
            "force-rewrite must preserve the prior file under <name>.bak.choruz-rebootstrap"
        );
        assert_eq!(read_string(&backup).await, edited);

        // Warning sidecar from the previous skipped refresh is no longer relevant.
        assert!(
            !dir.path().join(BOOTSTRAP_WARNING_FILENAME).exists(),
            "force-rewrite must clear any stale warning sidecar"
        );
    }

    #[tokio::test]
    async fn force_rewrite_defaults_to_claude_md_when_workspace_has_none_and_no_driver_hint() {
        // ADR-006 keeps CLAUDE.md as the historical default when neither the
        // workspace nor the caller tells us which CLI will read it. When the
        // caller does know (executor knows from agent_runtime_bindings.driver_type;
        // rebootstrap knows from --principal), the driver hint should flow in —
        // see `bootstrap_uses_driver_hint_in_empty_workspace_per_turn` for the
        // per-turn coverage and `force_rewrite_uses_driver_hint_when_workspace_empty`
        // for the rebootstrap path.
        let dir = TempDir::new().unwrap();
        let rewritten = force_rewrite_bootstrap(dir.path(), None).await.unwrap();
        let claude = dir.path().join("CLAUDE.md");
        assert_eq!(rewritten, vec![claude.clone()]);
        assert!(claude.exists());
        assert_eq!(read_string(&claude).await, canonical_claude_md());
    }

    #[tokio::test]
    async fn force_rewrite_uses_driver_hint_when_workspace_empty() {
        // Codex driver -> AGENTS.md.
        let dir = TempDir::new().unwrap();
        let rewritten = force_rewrite_bootstrap(dir.path(), Some("codex_terminal"))
            .await
            .unwrap();
        let agents = dir.path().join("AGENTS.md");
        assert_eq!(rewritten, vec![agents.clone()]);
        assert!(agents.exists());
        assert!(
            !dir.path().join("CLAUDE.md").exists(),
            "codex driver hint must not plant CLAUDE.md"
        );
        assert_eq!(
            read_string(&agents).await,
            canonical_instructions("AGENTS.md", DEFAULT_ROLE)
        );

        for driver in ["pi_terminal", "grok_terminal", "opencode_terminal"] {
            let dir = TempDir::new().unwrap();
            let rewritten = force_rewrite_bootstrap(dir.path(), Some(driver))
                .await
                .unwrap();
            let agents = dir.path().join("AGENTS.md");
            assert_eq!(rewritten, vec![agents.clone()]);
            assert!(agents.exists());
            assert!(
                !dir.path().join("CLAUDE.md").exists(),
                "{driver} must use AGENTS.md"
            );
        }

        // Unknown drivers are configuration errors, not Claude aliases.
        let dir = TempDir::new().unwrap();
        let error = force_rewrite_bootstrap(dir.path(), Some("mystery_driver"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!dir.path().join("CLAUDE.md").exists());
    }

    #[tokio::test]
    async fn force_rewrite_ignores_driver_hint_when_workspace_already_has_files() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("CLAUDE.md");
        tokio::fs::write(&claude, "stale\n").await.unwrap();

        let rewritten = force_rewrite_bootstrap(dir.path(), Some("codex_terminal"))
            .await
            .unwrap();

        assert_eq!(
            rewritten,
            vec![claude.clone()],
            "driver hint must not add new files when existing instruction files are present"
        );
        assert!(
            !dir.path().join("AGENTS.md").exists(),
            "driver hint must not plant AGENTS.md alongside an existing CLAUDE.md"
        );
    }

    #[tokio::test]
    async fn force_rewrite_errors_on_missing_workspace() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        let err = force_rewrite_bootstrap(&missing, None).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn version_header_round_trips_through_parser() {
        let stamped = format!("{}\n\nbody bytes\n", current_header());
        let (v, body) = parse_version_and_body(&stamped);
        assert_eq!(v, Some(BOOTSTRAP_INSTRUCTION_VERSION));
        assert_eq!(body, "body bytes\n");
    }

    #[test]
    fn parser_handles_header_followed_by_body_without_blank_line() {
        let stamped = format!("{}\nbody\n", current_header());
        let (v, body) = parse_version_and_body(&stamped);
        assert_eq!(v, Some(BOOTSTRAP_INSTRUCTION_VERSION));
        assert_eq!(body, "body\n");
    }

    #[test]
    fn parser_returns_none_for_unstamped_legacy_content() {
        let (v, body) = parse_version_and_body("# Legacy\n\nstuff\n");
        assert_eq!(v, None);
        assert_eq!(body, "# Legacy\n\nstuff\n");
    }

    #[test]
    fn body_hash_is_stable_for_current_canonical_claude_md() {
        let body = canonical_claude_md_body();
        let h = body_sha256_hex(body);
        assert_eq!(h.len(), 64);
    }

    #[test]
    fn canonical_claude_md_carries_current_version_header() {
        let header = current_header();
        assert!(
            canonical_claude_md().starts_with(&header),
            "canonical_claude_md() must start with `{header}` so on-disk files round-trip the version"
        );
    }

    #[test]
    fn canonical_claude_md_teaches_your_tasks_envelope_field() {
        // Surface guard: the v3 bootstrap must teach agents that the
        // `your_tasks:` envelope hint is the authoritative list of cards
        // they own, so they `task_update` against it instead of fabricating
        // task keys or creating duplicates. This is the whole point of the
        // v3 bump — if the wording disappears, the guidance is gone.
        let lower = canonical_claude_md().to_lowercase();
        assert!(
            lower.contains("your_tasks:"),
            "canonical_claude_md() must document the `your_tasks:` envelope hint"
        );
        assert!(
            lower.contains("task_update") && lower.contains("duplicate"),
            "canonical_claude_md() must steer agents toward task_update against your_tasks instead of creating duplicates"
        );
        assert!(
            lower.contains("task_not_found")
                && lower.contains("your_tasks")
                && lower.contains("authoritative"),
            "task_not_found recovery must point agents at the authoritative your_tasks: hint, not a non-existent task_list command"
        );
    }

    #[test]
    fn canonical_claude_md_uses_existing_cards_instead_of_duplicate_creates() {
        let receiving = canonical_claude_md().to_lowercase();
        assert!(
            receiving.contains("your_tasks:"),
            "incoming envelope section must document the `your_tasks:` field"
        );
        assert!(
            receiving.contains("authoritative open"),
            "`your_tasks:` must be presented as authoritative existing board state"
        );
        assert!(
            receiving.contains("prefer `task_update` or `task_transfer`")
                && receiving.contains("instead of creating duplicates"),
            "agents must update/transfer existing cards instead of creating duplicate task_create cards"
        );
    }

    #[test]
    fn canonical_claude_md_documents_channel_task_commands() {
        for cmd in ["task_create", "task_update", "task_transfer"] {
            assert!(
                canonical_claude_md().contains(cmd),
                "canonical_claude_md() must document `{cmd}` as a channel task command"
            );
        }
    }

    #[test]
    fn canonical_claude_md_composes_every_standard_extension() {
        for capability in [
            "roster:",
            "Mutable Command Result Envelopes",
            "\"type\":\"share_file\"",
            "\"type\":\"provision_agent\"",
            "\"type\":\"create_group\"",
            "\"type\":\"set_cron\"",
            "absolute file paths",
            "parallel editing",
        ] {
            assert!(
                canonical_claude_md().contains(capability),
                "canonical Rust bootstrap must compose standard extension capability `{capability}`"
            );
        }
    }

    #[test]
    fn canonical_claude_md_lists_all_board_statuses() {
        for status in ["todo", "in_progress", "blocked", "in_review", "done"] {
            assert!(
                canonical_claude_md().contains(status),
                "canonical_claude_md() must list board status `{status}`"
            );
        }
    }

    #[test]
    fn canonical_claude_md_requires_idempotency_key_for_task_create() {
        assert!(
            canonical_claude_md().contains("idempotency_key"),
            "canonical_claude_md() must require an idempotency_key on task_create"
        );
    }

    #[test]
    fn canonical_claude_md_includes_kanban_worthy_heuristic() {
        let lower = canonical_claude_md().to_lowercase();
        assert!(
            lower.contains("kanban-worthy"),
            "canonical_claude_md() must teach the Kanban-worthy heuristic"
        );
        assert!(
            lower.contains("did not explicitly ask")
                || lower.contains("did **not** explicitly ask"),
            "canonical_claude_md() must say to create tasks even when the user did not explicitly ask"
        );
        assert!(
            lower.contains("one-turn"),
            "canonical_claude_md() must specifically warn against board tasks for one-turn work"
        );
    }

    #[test]
    fn canonical_claude_md_enables_plain_work_requests_to_use_silent_board_commands() {
        let channel_tasks =
            markdown_section(canonical_claude_md(), "Channel Tasks (Kanban Board)").to_lowercase();
        assert!(
            channel_tasks.contains("use silent `task_create`, `task_update`, and `task_transfer`"),
            "Channel Tasks section must make the silent task command surface the board mutation path"
        );
        assert!(
            channel_tasks.contains("create a board task")
                && channel_tasks.contains("did **not** explicitly ask for a task list"),
            "Kanban-worthy ordinary work must not depend on an explicit task-list request"
        );
        for trigger in [
            "multi-step",
            "delegated",
            "blocked",
            "long-running",
            "growing in scope",
        ] {
            assert!(
                channel_tasks.contains(trigger),
                "Kanban-worthy heuristic must cover ordinary work trigger `{trigger}`"
            );
        }
        assert!(
            channel_tasks.contains("routine board changes")
                && channel_tasks.contains("not in")
                && channel_tasks.contains("chat messages"),
            "routine status changes must be steered to silent board commands, not chat noise"
        );
    }

    #[test]
    fn canonical_claude_md_teaches_handoff_through_task_transfer() {
        let channel_tasks =
            markdown_section(canonical_claude_md(), "Channel Tasks (Kanban Board)").to_lowercase();
        assert!(
            channel_tasks.contains("transfer a task you own only to another visible agent"),
            "Channel Tasks section must tell agents to hand off owned work with task_transfer"
        );
        assert!(
            channel_tasks.contains("\"type\":\"task_transfer\"")
                && channel_tasks.contains("\"assignee\":\"qa-engineer\""),
            "task_transfer command shape must include the target visible-agent assignee"
        );
        assert!(
            channel_tasks.contains("visible agent in the current `roster:`"),
            "task_transfer guidance must bind the target assignee to the current roster"
        );
    }

    #[test]
    fn canonical_claude_md_forbids_human_assignment_by_agents() {
        let lower = canonical_claude_md().to_lowercase();
        assert!(
            lower.contains("must not assign") && lower.contains("human"),
            "canonical_claude_md() must tell agents not to assign tasks to humans"
        );
    }

    #[test]
    fn canonical_claude_md_forbids_chat_status_updates_for_routine_changes() {
        let lower = canonical_claude_md().to_lowercase();
        for pattern in ["[done]", "[blocked]", "[in progress]"] {
            assert!(
                lower.contains(pattern),
                "canonical_claude_md() must reference the legacy `{pattern}` chat-status pattern"
            );
        }
        assert!(
            lower.contains("task_update") && lower.contains("silently"),
            "canonical_claude_md() must steer routine status changes to silent task_update"
        );
    }

    #[test]
    fn canonical_claude_md_separates_visible_from_internal_work() {
        let lower = canonical_claude_md().to_lowercase();
        assert!(
            lower.contains("visible board task"),
            "canonical_claude_md() must define what a visible board task is"
        );
        assert!(
            lower.contains("internal") && lower.contains("subagent"),
            "canonical_claude_md() must explicitly say internal/subagent work stays private"
        );
    }

    #[test]
    fn canonical_claude_md_disambiguates_cli_local_planning_tools() {
        for tool in ["TaskCreate", "update_plan"] {
            assert!(
                canonical_claude_md().contains(tool),
                "canonical_claude_md() must mention CLI-local planner `{tool}` to disambiguate from channel tasks"
            );
        }
        assert!(
            canonical_claude_md()
                .to_lowercase()
                .contains("keep that work private"),
            "canonical_claude_md() must say CLI-local planners are private and not on the board"
        );
    }

    #[test]
    fn canonical_claude_md_demotes_tasks_md_as_deprecated() {
        let lower = canonical_claude_md().to_lowercase();
        assert!(
            !(lower.contains("edit tasks.md") || lower.contains("update tasks.md")),
            "canonical_claude_md() must not instruct agents to edit/update TASKS.md"
        );
        if lower.contains("tasks.md") {
            assert!(
                lower.contains("deprecated") || lower.contains("do not edit"),
                "Any TASKS.md mention must mark it as deprecated / do-not-edit"
            );
        }
    }

    #[test]
    fn canonical_claude_md_excludes_ai_manager_workflow_extension() {
        let lower = canonical_claude_md().to_lowercase();
        assert!(
            !lower.contains("metadata.workflow")
                && !lower.contains("human_input_needed")
                && !lower.contains("approval_required"),
            "generic bootstraps must not include the AI Manager-only workflow extension"
        );
    }

    #[test]
    fn canonical_claude_md_documents_command_result_discovery_path() {
        assert!(
            canonical_claude_md().contains(".choruz-outbox/results/"),
            "canonical_claude_md() must point agents at <workspace>/.choruz-outbox/results/ for command result discovery"
        );
        let lower = canonical_claude_md().to_lowercase();
        assert!(
            lower.contains("headless") && lower.contains("pty"),
            "canonical_claude_md() must say the same result path applies in headless and PTY/watcher modes"
        );
        for field in [
            "command_type",
            "\"ok\"",
            "error_code",
            "message",
            "idempotency_key",
            "emitted_at",
        ] {
            assert!(
                canonical_claude_md().contains(field),
                "canonical_claude_md() must document envelope field `{field}` so agents can parse results"
            );
        }
        // Every error_code the pipeline can emit must appear in the playbook so
        // agents have an authoritative recovery strategy for each one — missing
        // codes would silently fall through to "I don't know what happened".
        for code in [
            "validation_failed",
            "missing_target",
            "missing_assignee",
            "missing_task",
            "missing_title",
            "invalid_assignee",
            "idempotency_conflict",
            "not_found",
            "task_not_found",
            "group_not_found",
            "forbidden",
            "unauthorized",
            "gateway_error",
            "gateway_unavailable",
            "event_store_unavailable",
            "agent_token_unavailable",
            "channel_tasks_disabled",
            "unsupported_command",
        ] {
            assert!(
                canonical_claude_md().contains(code),
                "canonical_claude_md() must list error_code `{code}` so agents know how to recover"
            );
        }
    }

    #[test]
    fn canonical_claude_md_keeps_result_envelope_free_of_sensitive_surfaces() {
        let lower = canonical_claude_md().to_lowercase();
        let results_section_start = lower
            .find("mutable command result envelopes")
            .expect("canonical_claude_md() must contain the command result extension");
        // Search the rest of the document; the receipt rules / privacy disclaimer should
        // explicitly say no tokens / no prompts / no hidden ids / no raw diagnostics.
        let tail = &lower[results_section_start..];
        for forbidden in ["token", "prompt", "hidden principal", "raw gateway"] {
            assert!(
                tail.contains(forbidden),
                "canonical_claude_md() results-discovery section must call out that `{forbidden}` is not in the envelope"
            );
        }
    }
}
