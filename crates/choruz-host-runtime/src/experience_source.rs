//! Resolve a platform binding's native source; parsing belongs to choruz-learning.
use crate::TerminalSpec;
use choruz_agent_runtime::headless::{HeadlessDriver, harness_account_env};
use choruz_common::AppError;
use choruz_learning::native_source::{Harness, Source};
use std::path::PathBuf;

/// Only a finished platform turn may advance the analysis cursor. Historical
/// reference recovery uses the explicitly selected session instead.
pub(crate) fn for_read(spec: &TerminalSpec) -> Result<Source, AppError> {
    let live = crate::session::recorded_snapshot(spec);
    if let Ok(state) = &live
        && matches!(state.status.as_str(), "running" | "connecting" | "waiting")
    {
        return Err(AppError::Conflict(
            "Learning waits for the current turn to finish".into(),
        ));
    }
    let id = spec.resume_session_id.as_deref().or_else(|| {
        live.as_ref()
            .ok()
            .and_then(|state| state.session_id.as_deref())
    });
    resolve(spec, id)
}

pub(crate) fn selected(
    spec: &TerminalSpec,
    cursor: &choruz_learning::source::Cursor,
) -> Result<Source, AppError> {
    if spec.resume_session_id.as_deref() != Some(&cursor.session)
        || cursor.session.is_empty()
        || !cursor
            .session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::Validation(
            "Historical evidence requires the selected native session".into(),
        ));
    }
    resolve(spec, spec.resume_session_id.as_deref())
}

fn resolve(spec: &TerminalSpec, id: Option<&str>) -> Result<Source, AppError> {
    let session_id = id
        .ok_or_else(|| AppError::NotFound("The Agent has no recorded native session yet".into()))?;
    if session_id.is_empty()
        || !session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::Validation(
            "Invalid native session identifier".into(),
        ));
    }
    let (harness, account_home) = match spec.driver_type.as_str() {
        "claude_terminal" => (
            Harness::Claude,
            crate::session_history::claude_account_root(spec)?,
        ),
        "codex_terminal" => (
            Harness::Codex,
            spec.codex_home
                .as_ref()
                .map(PathBuf::from)
                .or(
                    harness_account_env(HeadlessDriver::Codex, &spec.harness_account)
                        .map_err(AppError::Validation)?
                        .map(|(_, path)| path),
                )
                .or_else(crate::codex::normal_codex_home)
                .ok_or_else(|| {
                    AppError::NotFound("Selected Codex account home is unavailable".into())
                })?,
        ),
        _ => {
            return Err(AppError::Validation(
                "Native learning supports Claude Code and Codex".into(),
            ));
        }
    };
    Ok(Source {
        harness,
        account_home,
        workspace_path: PathBuf::from(&spec.workspace_path),
        session_id: session_id.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn host_trace_waits_for_turn_completion_and_uses_recorded_session() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("account");
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        let spec = TerminalSpec {
            authentication: false,
            terminal_id: "source-gate".into(),
            driver_type: "codex_terminal".into(),
            binary_path: None,
            workspace_path: root.path().to_string_lossy().into(),
            cols: 80,
            rows: 24,
            resume_session_id: None,
            codex_home: Some(home.to_string_lossy().into()),
            model: None,
            harness_account: json!({}),
        };
        std::fs::write(home.join("sessions/source.jsonl"), format!("{}\n{}\n",
            json!({"type":"session_meta","payload":{"id":"recorded","cwd":root.path()}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":"source"}}))).unwrap();
        let directory = root.path().join(".choruz/sessions");
        std::fs::create_dir_all(&directory).unwrap();
        let owner_key = crate::session::owner_key(&spec);
        for status in ["running", "connecting", "waiting", "idle"] {
            // Persist through the existing journal wire contract, without a live CLI.
            std::fs::write(directory.join(format!("source-gate-{owner_key}.json")), json!({
                "version":1,
                "owner":{"binding":spec.terminal_id,"driver":spec.driver_type,"workspace":spec.workspace_path,"account":null,"profile":null,"generation":null,"host":null,"company":null},
                "state":{"owner":owner_key,"session_id":"recorded","turn_id":null,"status":status,"items":[],"requests":[],"error":null}
            }).to_string()).unwrap();
            let result = crate::execute(crate::HostRequest::ExperienceTrace {
                spec: Box::new(spec.clone()),
                cursor: Default::default(),
            })
            .await;
            if status == "idle" {
                let window = result.unwrap();
                assert_eq!(window["cursor"]["session"], "recorded");
                assert_eq!(window["records"].as_array().unwrap().len(), 1);
            } else {
                assert!(
                    matches!(result, Err(AppError::Conflict(message)) if message.contains("turn to finish"))
                );
            }
        }
    }
}
