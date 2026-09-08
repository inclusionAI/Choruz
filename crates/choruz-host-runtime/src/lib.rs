//! Device-local runtime operations shared by the API gateway (for its own
//! device) and `choruz-connector` (for a paired remote device).
//!
//! [`HostRequest`] is the one vocabulary for the request/response work a
//! device does: browse directories, scan session catalogs, find the newest
//! native session, prepare a managed Codex home and attribute the session
//! file it produces. [`execute`] runs a request on the current device.
//! Interactive terminals are streams rather than requests and live in
//! [`terminal`]; [`link`] carries both to a remote device.

pub mod codex;
pub mod drivers;
pub mod filesystem;
pub mod inbox;
pub mod instructions;
pub mod link;
pub mod outbox;
pub mod process;
pub mod session;
mod session_history;
pub mod session_protocol;
pub mod terminal;

#[cfg(test)]
static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

use std::{collections::BTreeSet, path::PathBuf};

use choruz_agent_runtime::{HarnessKind, SessionAccount, SessionCatalogScanner};
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use codex::{CodexSessionFileMeta, ImportedCodexSession, ManagedCodexHome};
pub use filesystem::{FilesystemEntry, FilesystemHome, FilesystemListing};
pub use process::ProcessContainer;
pub use terminal::{
    EnsureOutcome, TerminalPool, TerminalSession, TerminalSpec, default_terminal_binary,
    ensure_terminal, evict_stale_terminals, is_terminal_driver, live_terminal_exists,
    new_terminal_pool, terminal_binary, terminal_cli_args,
};

/// The helper installed as `<workspace>/.choruz/send`; `"$CHORUZ_SEND"`
/// points at it.
pub const SEND_HELPER: &str = include_str!("../assets/choruz-send.sh");

/// One request/response operation on a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HostRequest {
    DriverCatalog {
        driver_type: Option<String>,
    },
    FilesystemHome,
    FilesystemList {
        path: String,
        #[serde(default)]
        show_hidden: bool,
        #[serde(default)]
        include_files: bool,
    },
    ScanSessions {
        workspace_path: String,
        harnesses: BTreeSet<HarnessKind>,
        accounts: Vec<SessionAccount>,
    },
    LatestSession {
        workspace_path: String,
        driver_type: String,
    },
    CodexPrepareHome {
        binding_id: String,
        workspace_path: String,
        harness_account: Value,
    },
    CodexNewSession {
        home_path: String,
        sessions_path: String,
        baseline_session_files: Vec<String>,
        workspace_path: String,
    },
    CodexAnchorMatches {
        sessions_path: String,
        native_session_path: String,
        session_id: String,
        workspace_path: String,
    },
    CodexImportSession {
        binding_id: String,
        workspace_path: String,
        native_session_id: String,
        harness_account: Value,
    },
    /// Install the outbox helper and its Maildir directories in a workspace.
    EnsureOutboxHelper {
        workspace_path: String,
    },
    /// Read the identity, models and exact quota of a Harness account the
    /// device already holds a login for.
    HarnessProbe {
        driver_type: String,
        account_id: String,
        profile_kind: String,
    },
    /// Create an agent workspace on the device: the directory (a generated
    /// one under `~/.choruz/workspaces` when `workspace_path` is absent),
    /// the files the controller rendered for it, and the outbox helper.
    ProvisionWorkspace {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_path: Option<String>,
        name: String,
        files: Vec<WorkspaceFile>,
    },
}

/// One file the controller wants in a provisioned workspace, relative to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionedWorkspace {
    pub workspace_path: String,
}

/// A verified account snapshot, as the device reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessProbeResult {
    pub account_fingerprint: String,
    pub subscription_type: Option<String>,
    pub models: Value,
    pub usage: Value,
}

/// A managed Codex home together with the session files that already existed
/// when the terminal was about to spawn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexHomeReady {
    pub home_path: PathBuf,
    pub sessions_path: PathBuf,
    pub baseline_session_files: Vec<String>,
}

/// Run one request on this device. Filesystem work runs on a blocking thread.
pub async fn execute(request: HostRequest) -> Result<Value, AppError> {
    match request {
        HostRequest::DriverCatalog { driver_type } => {
            drivers::inspect(driver_type.as_deref()).await
        }
        HostRequest::FilesystemHome => Ok(json!(filesystem::home_directory())),
        HostRequest::FilesystemList {
            path,
            show_hidden,
            include_files,
        } => blocking(move || filesystem::list_directory(&path, show_hidden, include_files)).await,
        HostRequest::ScanSessions {
            workspace_path,
            harnesses,
            accounts,
        } => scan_sessions(&workspace_path, &harnesses, &accounts).await,
        HostRequest::LatestSession {
            workspace_path,
            driver_type,
        } => Ok(json!(
            choruz_agent_runtime::latest_native_session(&workspace_path, &driver_type).await
        )),
        HostRequest::CodexPrepareHome {
            binding_id,
            workspace_path,
            harness_account,
        } => {
            blocking(move || {
                let account_home = choruz_agent_runtime::headless::prepare_harness_account_env(
                    choruz_agent_runtime::headless::HeadlessDriver::Codex,
                    &harness_account,
                )
                .map_err(AppError::Validation)?
                .map(|(_, path)| path);
                let managed = codex::provision_managed_codex_home(
                    &binding_id,
                    &workspace_path,
                    account_home.as_deref(),
                )?;
                let baseline = codex::collect_codex_session_files(&managed.sessions_path)?
                    .into_iter()
                    .collect();
                Ok(CodexHomeReady {
                    home_path: managed.home_path,
                    sessions_path: managed.sessions_path,
                    baseline_session_files: baseline,
                })
            })
            .await
        }
        HostRequest::CodexNewSession {
            home_path,
            sessions_path,
            baseline_session_files,
            workspace_path,
        } => {
            blocking(move || {
                codex::unique_new_codex_session(
                    std::path::Path::new(&home_path),
                    std::path::Path::new(&sessions_path),
                    &baseline_session_files,
                    &workspace_path,
                )
            })
            .await
        }
        HostRequest::CodexAnchorMatches {
            sessions_path,
            native_session_path,
            session_id,
            workspace_path,
        } => {
            blocking(move || {
                Ok(codex::codex_anchor_file_matches(
                    std::path::Path::new(&sessions_path),
                    std::path::Path::new(&native_session_path),
                    &session_id,
                    &workspace_path,
                ))
            })
            .await
        }
        HostRequest::CodexImportSession {
            binding_id,
            workspace_path,
            native_session_id,
            harness_account,
        } => {
            blocking(move || {
                let account_home = choruz_agent_runtime::headless::prepare_harness_account_env(
                    choruz_agent_runtime::headless::HeadlessDriver::Codex,
                    &harness_account,
                )
                .map_err(AppError::Validation)?
                .map(|(_, path)| path);
                codex::import_codex_session(
                    &binding_id,
                    &workspace_path,
                    &native_session_id,
                    account_home.as_deref(),
                )
            })
            .await
        }
        HostRequest::EnsureOutboxHelper { workspace_path } => {
            blocking(move || ensure_outbox_helper(std::path::Path::new(&workspace_path))).await
        }
        HostRequest::ProvisionWorkspace {
            workspace_path,
            name,
            files,
        } => blocking(move || provision_workspace(workspace_path.as_deref(), &name, &files)).await,
        HostRequest::HarnessProbe {
            driver_type,
            account_id,
            profile_kind,
        } => {
            let driver =
                choruz_agent_runtime::headless::HeadlessDriver::from_driver_type(&driver_type)
                    .ok_or_else(|| {
                        AppError::Validation(format!("{driver_type} has no Harness account probe"))
                    })?;
            let probe =
                choruz_harness_login::probe_account(&choruz_harness_login::AccountProfile {
                    driver,
                    account_id,
                    profile_kind,
                })
                .await
                .map_err(AppError::Validation)?;
            Ok(json!(HarnessProbeResult {
                account_fingerprint: probe.fingerprint,
                subscription_type: probe.subscription_type,
                models: probe.models,
                usage: probe.usage,
            }))
        }
    }
}

/// Write `<workspace>/.choruz/send` and the `.choruz-outbox/{tmp,new}`
/// directories the helper needs.
pub fn ensure_outbox_helper(workspace: &std::path::Path) -> Result<(), AppError> {
    let helper_dir = workspace.join(".choruz");
    let outbox_dir = workspace.join(".choruz-outbox");
    for directory in [
        outbox_dir.join("tmp"),
        outbox_dir.join("new"),
        helper_dir.clone(),
    ] {
        std::fs::create_dir_all(directory)
            .map_err(|error| AppError::Internal(format!("prepare Agent outbox: {error}")))?;
    }
    let helper_path = helper_dir.join("send");
    std::fs::write(&helper_path, SEND_HELPER)
        .map_err(|error| AppError::Internal(format!("install Agent outbox helper: {error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&helper_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| AppError::Internal(format!("enable Agent outbox helper: {error}")))?;
    }
    Ok(())
}

async fn scan_sessions(
    workspace_path: &str,
    harnesses: &BTreeSet<HarnessKind>,
    accounts: &[SessionAccount],
) -> Result<Value, AppError> {
    if harnesses.is_empty() {
        return Err(AppError::Validation("select at least one Harness".into()));
    }
    let canonical = filesystem::allowed_canonical_path(workspace_path)?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|error| AppError::NotFound(format!("cannot inspect workspace: {error}")))?;
    if !metadata.is_dir() {
        return Err(AppError::Validation(
            "workspace path must be a directory".into(),
        ));
    }
    let result = SessionCatalogScanner::from_env()
        .map_err(AppError::Internal)?
        .scan_profiles(&canonical, harnesses, accounts)
        .await
        .map_err(AppError::Internal)?;
    serde_json::to_value(result).map_err(|error| AppError::Internal(error.to_string()))
}

async fn blocking<T: Serialize + Send + 'static>(
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<Value, AppError> {
    let value = tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| AppError::Internal(format!("device operation panicked: {error}")))??;
    serde_json::to_value(value).map_err(|error| AppError::Internal(error.to_string()))
}

/// Create the workspace directory and the controller's files in it, then the
/// outbox helper. File paths stay inside the workspace.
pub fn provision_workspace(
    workspace_path: Option<&str>,
    name: &str,
    files: &[WorkspaceFile],
) -> Result<ProvisionedWorkspace, AppError> {
    let workspace = match workspace_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        Some(path) => PathBuf::from(path),
        None => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| AppError::Internal("HOME is not set on this device".into()))?;
            let slug = name
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() {
                        ch.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect::<String>()
                .trim_matches('-')
                .to_owned();
            let slug = if slug.is_empty() {
                "agent".to_owned()
            } else {
                slug
            };
            home.join(".choruz")
                .join("workspaces")
                .join(format!("{slug}-{}", &choruz_common::new_id()[..8]))
                .join("workspace")
        }
    };
    if !workspace.is_absolute() {
        return Err(AppError::Validation(
            "workspace path must be absolute".into(),
        ));
    }
    std::fs::create_dir_all(&workspace)
        .map_err(|error| AppError::Internal(format!("create workspace: {error}")))?;
    for file in files {
        let relative = std::path::Path::new(&file.path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(AppError::Validation(format!(
                "workspace file path {} must stay inside the workspace",
                file.path
            )));
        }
        let target = workspace.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| AppError::Internal(format!("create workspace dir: {error}")))?;
        }
        std::fs::write(&target, &file.content)
            .map_err(|error| AppError::Internal(format!("write workspace file: {error}")))?;
    }
    ensure_outbox_helper(&workspace)?;
    let canonical = std::fs::canonicalize(&workspace)
        .map_err(|error| AppError::Internal(format!("canonicalize workspace: {error}")))?;
    Ok(ProvisionedWorkspace {
        workspace_path: canonical.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisioned_workspace_holds_the_rendered_files_and_stays_inside_itself() {
        let root = tempfile::tempdir().expect("temp root");
        let target = root.path().join("agent").join("workspace");
        let provisioned = provision_workspace(
            Some(target.to_str().unwrap()),
            "Remote Builder",
            &[
                WorkspaceFile {
                    path: "CLAUDE.md".into(),
                    content: "# role\n".into(),
                },
                WorkspaceFile {
                    path: ".claude/skills/review/SKILL.md".into(),
                    content: "review\n".into(),
                },
            ],
        )
        .expect("provision");
        assert_eq!(
            provisioned.workspace_path,
            std::fs::canonicalize(&target).unwrap().to_string_lossy()
        );
        assert_eq!(
            std::fs::read_to_string(target.join("CLAUDE.md")).unwrap(),
            "# role\n"
        );
        assert!(target.join(".claude/skills/review/SKILL.md").is_file());
        assert!(target.join(".choruz/send").is_file());

        let escape = provision_workspace(
            Some(target.to_str().unwrap()),
            "Remote Builder",
            &[WorkspaceFile {
                path: "../outside.md".into(),
                content: String::new(),
            }],
        );
        assert!(escape.is_err(), "a path outside the workspace is refused");
    }

    #[test]
    fn workspace_gets_the_shared_outbox_helper() {
        let directory = tempfile::tempdir().expect("temp workspace");
        ensure_outbox_helper(directory.path()).expect("helper");
        let helper = directory.path().join(".choruz/send");
        assert_eq!(std::fs::read_to_string(&helper).unwrap(), SEND_HELPER);
        assert!(directory.path().join(".choruz-outbox/new").is_dir());
        assert!(directory.path().join(".choruz-outbox/tmp").is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(helper).unwrap().permissions().mode() & 0o111,
                0
            );
        }
    }
}
