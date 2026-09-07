//! Read the selected Claude session's active message chain on its account's device.
use crate::{TerminalSpec, session_protocol::SessionSnapshot};
use choruz_agent_runtime::headless::{HeadlessDriver, harness_account_env};
use choruz_common::AppError;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

pub fn claude_history(spec: &TerminalSpec, id: &str) -> Result<SessionSnapshot, AppError> {
    if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(AppError::Validation(
            "Claude session id is not a UUID".into(),
        ));
    }
    let root = claude_account_root(spec)?;
    read_claude_history(&root, Path::new(&spec.workspace_path), id)
}

pub fn claude_account_root(spec: &TerminalSpec) -> Result<PathBuf, AppError> {
    harness_account_env(HeadlessDriver::Claude, &spec.harness_account)
        .map_err(AppError::Validation)?
        .map(|(_, path)| path)
        .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
        .ok_or_else(|| AppError::Internal("Claude account directory is unavailable".into()))
}

fn read_claude_history(
    root: &Path,
    workspace: &Path,
    id: &str,
) -> Result<SessionSnapshot, AppError> {
    let projects = std::fs::read_dir(root.join("projects")).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::NotFound("Claude account has no sessions".into())
        } else {
            AppError::Internal(format!("read Claude projects: {e}"))
        }
    })?;
    let expected = workspace
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("resolve session workspace: {e}")))?;
    let mut matches = Vec::new();
    for project in projects {
        let project =
            project.map_err(|e| AppError::Internal(format!("read Claude project: {e}")))?;
        let file = project.path().join(format!("{id}.jsonl"));
        if file.is_file() {
            matches.push(file);
        }
    }
    if matches.len() != 1 {
        return Err(AppError::NotFound(
            "The selected Claude session is missing or ambiguous in this account".into(),
        ));
    }
    let file = std::fs::File::open(&matches[0])
        .map_err(|e| AppError::Internal(format!("read Claude session: {e}")))?;
    let mut entries = VecDeque::new();
    let mut retained_bytes = 0;
    let mut truncated = false;
    let mut valid_workspace = false;
    let mut reader = BufReader::new(file);
    loop {
        let mut line = Vec::new();
        (&mut reader)
            .take(8 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)
            .map_err(|e| AppError::Internal(format!("read Claude history: {e}")))?;
        if line.is_empty() {
            break;
        }
        if line.len() > 8 * 1024 * 1024 {
            return Err(AppError::Validation("A native history entry exceeds the conversation preview limit. Open Terminal for the complete session.".into()));
        }
        let entry: Value = serde_json::from_slice(&line)
            .map_err(|e| AppError::Internal(format!("decode Claude history: {e}")))?;
        if let Some(cwd) = entry["cwd"].as_str() {
            valid_workspace |= Path::new(cwd).canonicalize().ok().as_ref() == Some(&expected);
        }
        retained_bytes += line.len();
        entries.push_back((entry, line.len()));
        while retained_bytes > 32 * 1024 * 1024 || entries.len() > 2048 {
            if let Some((_, size)) = entries.pop_front() {
                retained_bytes -= size;
            }
            truncated = true;
        }
    }
    if !valid_workspace {
        return Err(AppError::Forbidden(
            "Claude session does not belong to this workspace".into(),
        ));
    }
    let mut state = project_claude_chain(entries.into_iter().map(|(entry, _)| entry).collect());
    state.history_truncated |= truncated;
    state.native_home_path = Some(root.to_string_lossy().into());
    state.native_session_path = Some(matches[0].to_string_lossy().into());
    Ok(state)
}

fn project_claude_chain(entries: Vec<Value>) -> SessionSnapshot {
    let mut state = SessionSnapshot::default();
    let indexed = entries
        .iter()
        .filter_map(|entry| entry["uuid"].as_str().map(|id| (id, entry)))
        .collect::<HashMap<_, _>>();
    let mut leaf = entries.iter().rev().find(|entry| {
        matches!(entry["type"].as_str(), Some("assistant" | "user")) && entry["isSidechain"] != true
    });
    let mut visited = HashSet::new();
    let mut chain = Vec::new();
    while let Some(entry) = leaf {
        let Some(id) = entry["uuid"].as_str() else {
            break;
        };
        if !visited.insert(id) {
            break;
        }
        chain.push(entry);
        leaf = entry["parentUuid"]
            .as_str()
            .and_then(|parent| indexed.get(parent).copied());
    }
    for entry in chain.into_iter().rev() {
        state.claude(entry);
        state.enforce_limits();
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn follows_selected_leaf_without_replaying_abandoned_branch() {
        let message = |id: &str, parent: Value, text: &str| json!({"type":"assistant","uuid":id,"parentUuid":parent,"message":{"id":id,"content":[{"type":"text","text":text}]}});
        let state = project_claude_chain(vec![
            message("a", Value::Null, "first"),
            message("old", json!("a"), "discarded"),
            message("new", json!("a"), "replacement"),
        ]);
        assert_eq!(
            state
                .items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "replacement"]
        );
    }
}
