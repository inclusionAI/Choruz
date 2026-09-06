use std::path::PathBuf;

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

use crate::{ApiError, ApiState, require_human_operator};

// ── Types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct FilesystemListQuery {
    path: String,
    show_hidden: Option<bool>,
    include_files: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FilesystemStatResponse {
    exists: bool,
    #[serde(rename = "type")]
    entry_type: Option<String>,
    path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct FilesystemHomeResponse {
    home: String,
    separator: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FilesystemStatQuery {
    path: String,
}

// ── Helpers ───────────────────────────────────────────────────────────

fn allowed_browse_roots() -> Vec<PathBuf> {
    choruz_host_runtime::filesystem::browse_roots()
}

pub(crate) fn validate_path_whitelist(path: &std::path::Path) -> Result<(), ApiError> {
    let roots = allowed_browse_roots();
    if path_is_within_roots(path, &roots) {
        Ok(())
    } else {
        Err(ApiError(AppError::Forbidden(format!(
            "path {} is outside allowed browse roots",
            path.display()
        ))))
    }
}

fn path_is_within_roots(path: &std::path::Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

// ── Handlers ──────────────────────────────────────────────────────────

pub(crate) async fn filesystem_list(
    headers: HeaderMap,
    Query(query): Query<FilesystemListQuery>,
    State(state): State<ApiState>,
) -> Result<Json<choruz_host_runtime::filesystem::FilesystemListing>, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    let listing = tokio::task::spawn_blocking(move || {
        choruz_host_runtime::filesystem::list_directory(
            &query.path,
            query.show_hidden.unwrap_or(false),
            query.include_files.unwrap_or(false),
        )
    })
    .await
    .map_err(|error| {
        ApiError(AppError::Internal(format!(
            "directory listing task: {error}"
        )))
    })?
    .map_err(ApiError)?;
    Ok(Json(listing))
}

pub(crate) async fn filesystem_stat(
    headers: HeaderMap,
    Query(query): Query<FilesystemStatQuery>,
    State(state): State<ApiState>,
) -> Result<Json<FilesystemStatResponse>, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    let raw_path = PathBuf::from(&query.path);

    // If the path doesn't exist, return exists=false without whitelist check
    let canonical = match tokio::fs::canonicalize(&raw_path).await {
        Ok(p) => p,
        Err(_) => {
            return Ok(Json(FilesystemStatResponse {
                exists: false,
                entry_type: None,
                path: query.path,
            }));
        }
    };

    validate_path_whitelist(&canonical)?;

    let metadata = match tokio::fs::metadata(&canonical).await {
        Ok(m) => m,
        Err(_) => {
            return Ok(Json(FilesystemStatResponse {
                exists: false,
                entry_type: None,
                path: canonical.to_string_lossy().to_string(),
            }));
        }
    };

    let entry_type = if metadata.is_dir() {
        "directory"
    } else if metadata.is_file() {
        "file"
    } else {
        "other"
    };

    Ok(Json(FilesystemStatResponse {
        exists: true,
        entry_type: Some(entry_type.into()),
        path: canonical.to_string_lossy().to_string(),
    }))
}

pub(crate) async fn filesystem_home(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<FilesystemHomeResponse>, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());

    Ok(Json(FilesystemHomeResponse {
        home,
        separator: std::path::MAIN_SEPARATOR.to_string(),
    }))
}

// ── File read / write ────────────────────────────────────────────────

const MAX_READ_SIZE: u64 = 1_048_576; // 1 MB

#[derive(Debug, Deserialize)]
pub(crate) struct FilesystemReadQuery {
    path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct FilesystemReadResponse {
    content: String,
    path: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FilesystemWriteRequest {
    path: String,
    content: String,
    original_content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct FilesystemWriteResponse {
    ok: bool,
    path: String,
}

fn validate_path_under_home(path: &std::path::Path) -> Result<(), ApiError> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    if path.starts_with(&home) {
        Ok(())
    } else {
        Err(ApiError(AppError::Forbidden(format!(
            "path {} is outside HOME directory",
            path.display()
        ))))
    }
}

fn looks_binary(buf: &[u8]) -> bool {
    // If the first 8KB contain a NUL byte, treat as binary
    let check_len = buf.len().min(8192);
    buf[..check_len].contains(&0)
}

pub(crate) async fn filesystem_read(
    headers: HeaderMap,
    Query(query): Query<FilesystemReadQuery>,
    State(state): State<ApiState>,
) -> Result<Json<FilesystemReadResponse>, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    let canonical = tokio::fs::canonicalize(&query.path)
        .await
        .map_err(|e| ApiError(AppError::NotFound(format!("path not found: {e}"))))?;

    validate_path_under_home(&canonical)?;

    let content = read_editable_content(&canonical).await?;
    Ok(Json(FilesystemReadResponse {
        size: content.len() as u64,
        content,
        path: canonical.to_string_lossy().to_string(),
    }))
}

async fn read_editable_content(path: &std::path::Path) -> Result<String, ApiError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|e| ApiError(AppError::NotFound(format!("cannot stat file: {e}"))))?;

    if !metadata.is_file() {
        return Err(ApiError(AppError::Validation("path is not a file".into())));
    }

    let size = metadata.len();
    if size > MAX_READ_SIZE {
        return Err(ApiError(AppError::Validation(format!(
            "file too large ({} bytes, max {})",
            size, MAX_READ_SIZE
        ))));
    }

    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| ApiError(AppError::Internal(format!("read error: {e}"))))?;
    let mut bytes = Vec::new();
    file.take(MAX_READ_SIZE + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| ApiError(AppError::Internal(format!("read error: {e}"))))?;

    if bytes.len() as u64 > MAX_READ_SIZE {
        return Err(ApiError(AppError::Validation("file too large".into())));
    }
    if looks_binary(&bytes) {
        return Err(ApiError(AppError::Validation(
            "binary file, cannot display".into(),
        )));
    }

    String::from_utf8(bytes)
        .map_err(|_| ApiError(AppError::Validation("binary file, cannot display".into())))
}

pub(crate) async fn filesystem_write(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(body): Json<FilesystemWriteRequest>,
) -> Result<Response, ApiError> {
    let _ = require_human_operator(&headers, &state).await?;

    if body.content.len() as u64 > MAX_READ_SIZE
        || body.original_content.len() as u64 > MAX_READ_SIZE
    {
        return Err(ApiError(AppError::Validation("file too large".into())));
    }
    let target = tokio::fs::canonicalize(&body.path)
        .await
        .map_err(|e| ApiError(AppError::NotFound(format!("path not found: {e}"))))?;
    validate_path_under_home(&target)?;
    let current_content = read_editable_content(&target).await?;
    if current_content != body.original_content {
        return Ok((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "File changed on disk. Reload or explicitly overwrite with your draft.",
                "current_content": current_content,
            })),
        )
            .into_response());
    }

    // Detect prior edits; arbitrary external writers do not participate in this comparison.

    tokio::fs::write(&target, body.content.as_bytes())
        .await
        .map_err(|e| ApiError(AppError::Internal(format!("write error: {e}"))))?;

    Ok(Json(FilesystemWriteResponse {
        ok: true,
        path: target.to_string_lossy().to_string(),
    })
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialise tests that mutate CHORUZ_FS_BROWSE_ROOTS / HOME, since
    /// `cargo test` runs tests in parallel and env vars are process-global.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<F: FnOnce()>(roots: Option<&str>, home: Option<&str>, body: F) {
        let _g = ENV_LOCK.lock().unwrap();
        let prev_roots = std::env::var("CHORUZ_FS_BROWSE_ROOTS").ok();
        let prev_home = std::env::var("HOME").ok();
        match roots {
            Some(v) => unsafe {
                std::env::set_var("CHORUZ_FS_BROWSE_ROOTS", v);
            },
            None => unsafe {
                std::env::remove_var("CHORUZ_FS_BROWSE_ROOTS");
            },
        }
        if let Some(v) = home {
            unsafe {
                std::env::set_var("HOME", v);
            }
        }
        body();
        match prev_roots {
            Some(v) => unsafe {
                std::env::set_var("CHORUZ_FS_BROWSE_ROOTS", v);
            },
            None => unsafe {
                std::env::remove_var("CHORUZ_FS_BROWSE_ROOTS");
            },
        }
        match prev_home {
            Some(v) => unsafe {
                std::env::set_var("HOME", v);
            },
            None => unsafe {
                std::env::remove_var("HOME");
            },
        }
    }

    // looks_binary -----------------------------------------------------------

    #[test]
    fn looks_binary_returns_false_for_pure_text() {
        assert!(!looks_binary(b"hello world\n"));
        assert!(!looks_binary("café résumé\n".as_bytes()));
    }

    #[test]
    fn looks_binary_returns_true_when_first_bytes_contain_nul() {
        assert!(looks_binary(b"hello\0world"));
    }

    #[test]
    fn looks_binary_only_inspects_first_8kb() {
        // NUL after 8KB → not detected (intentional, optimization).
        let mut buf = vec![b'a'; 8192];
        buf.push(0);
        assert!(!looks_binary(&buf));
    }

    #[test]
    fn looks_binary_handles_empty_input() {
        assert!(!looks_binary(&[]));
    }

    // allowed_browse_roots --------------------------------------------------

    #[test]
    fn allowed_browse_roots_parses_comma_list() {
        with_env(Some("/tmp/a,/tmp/b , /tmp/c"), None, || {
            let roots = allowed_browse_roots();
            assert_eq!(roots.len(), 3);
            assert_eq!(roots[0], PathBuf::from("/tmp/a"));
            assert_eq!(roots[1], PathBuf::from("/tmp/b"));
            assert_eq!(roots[2], PathBuf::from("/tmp/c"));
        });
    }

    #[test]
    fn allowed_browse_roots_skips_empty_segments() {
        with_env(Some(",/tmp/a,,"), None, || {
            let roots = allowed_browse_roots();
            assert_eq!(roots, vec![PathBuf::from("/tmp/a")]);
        });
    }

    #[test]
    fn allowed_browse_roots_falls_back_to_home_when_var_unset() {
        with_env(None, Some("/tmp/test-home"), || {
            let roots = allowed_browse_roots();
            assert_eq!(roots, vec![PathBuf::from("/tmp/test-home")]);
        });
    }

    // validate_path_whitelist -----------------------------------------------

    #[test]
    fn validate_path_whitelist_accepts_paths_under_a_root() {
        with_env(Some("/tmp/allowed"), None, || {
            assert!(validate_path_whitelist(&PathBuf::from("/tmp/allowed/inner")).is_ok());
            assert!(validate_path_whitelist(&PathBuf::from("/tmp/allowed")).is_ok());
        });
    }

    #[test]
    fn validate_path_whitelist_rejects_paths_outside_all_roots() {
        with_env(Some("/tmp/allowed"), None, || {
            let err = validate_path_whitelist(&PathBuf::from("/etc/passwd")).unwrap_err();
            // ApiError wraps AppError::Forbidden
            assert!(matches!(err.0, choruz_common::AppError::Forbidden(_)));
        });
    }

    // validate_path_under_home ---------------------------------------------

    #[test]
    fn validate_path_under_home_accepts_paths_inside_home() {
        with_env(None, Some("/tmp/myhome"), || {
            assert!(validate_path_under_home(&PathBuf::from("/tmp/myhome/work/file.txt")).is_ok());
        });
    }

    #[test]
    fn validate_path_under_home_rejects_paths_outside_home() {
        with_env(None, Some("/tmp/myhome"), || {
            let err = validate_path_under_home(&PathBuf::from("/etc/secret")).unwrap_err();
            assert!(matches!(err.0, choruz_common::AppError::Forbidden(_)));
        });
    }
}
