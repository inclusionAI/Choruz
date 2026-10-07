//! Read native Codex session files without creating or modifying an account.
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

/// One Codex session file and the identity written in its `session_meta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexSessionFileMeta {
    pub session_id: String,
    pub cwd: String,
    pub path: PathBuf,
}

/// Every regular `.jsonl` file under the sessions tree, canonical, symlinks
/// skipped; the spawn baseline that new session files are compared against.
pub fn collect_codex_session_files(sessions_path: &Path) -> Result<HashSet<String>, AppError> {
    let canonical_sessions = fs::canonicalize(sessions_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex sessions", error))?;
    let mut stack = vec![canonical_sessions.clone()];
    let mut files = HashSet::new();

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(redact_local_path_error("read Codex sessions", error)),
        };
        for entry in entries {
            let entry = entry
                .map_err(|error| redact_local_path_error("read Codex session entry", error))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| redact_local_path_error("stat Codex session entry", error))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if !metadata.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            {
                continue;
            }
            let canonical = fs::canonicalize(&path).map_err(|error| {
                redact_local_path_error("canonicalize Codex session file", error)
            })?;
            if canonical.starts_with(&canonical_sessions) {
                files.insert(canonical.to_string_lossy().to_string());
            }
        }
    }

    Ok(files)
}

pub fn read_codex_session_meta(path: &Path) -> Result<Option<CodexSessionFileMeta>, AppError> {
    let file = fs::File::open(path)
        .map_err(|error| redact_local_path_error("read Codex session", error))?;
    let reader = std::io::BufReader::new(file);
    for line in std::io::BufRead::lines(reader).take(20) {
        let line = line.map_err(|error| redact_local_path_error("read Codex session", error))?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
            continue;
        }
        let payload = value.get("payload").unwrap_or(&value);
        let Some(session_id) = payload
            .get("id")
            .or_else(|| payload.get("session_id"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
        else {
            return Ok(None);
        };
        let Some(cwd) = payload
            .get("cwd")
            .or_else(|| payload.get("workspace_path"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
        else {
            return Ok(None);
        };
        return Ok(Some(CodexSessionFileMeta {
            session_id: session_id.to_string(),
            cwd: cwd.to_string(),
            path: path.to_path_buf(),
        }));
    }
    Ok(None)
}

fn redact_local_path_error(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("{context}: {error}"))
}
