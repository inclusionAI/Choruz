//! The `choruz-pipeline rebootstrap` subcommand: force-rewrite a workspace's
//! managed instruction file after the runtime refused to refresh it.

use std::path::PathBuf;

use choruz_host_runtime::instructions::{BOOTSTRAP_INSTRUCTION_VERSION, force_rewrite_bootstrap};

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
