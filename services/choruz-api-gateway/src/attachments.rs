use std::{path::PathBuf, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use choruz_common::{AppError, AppResult, new_id, now};
use choruz_domain::Principal;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Max attachment size (25 MB). Matches common chat platforms (Slack/Discord
/// non-nitro tier). Enforced both at the HTTP body layer and here in put().
pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

/// Allowlist of MIME types we accept for upload. Anything else is rejected
/// at put() time. Keeps `text/html` / `application/javascript` / etc. out of
/// storage so a same-origin download of a rogue attachment can't XSS.
const ALLOWED_MIME_PREFIXES: &[&str] = &["image/", "video/", "audio/"];
const ALLOWED_MIME_EXACT: &[&str] = &[
    "application/pdf",
    "application/zip",
    "application/x-tar",
    "application/gzip",
    "application/octet-stream",
    "text/plain",
    "text/markdown",
    "text/csv",
];

/// Browser MIME detection for source files is inconsistent across platforms:
/// Python alone is commonly reported as `text/x-python-script`.  Store known
/// source extensions as `text/plain` instead of widening the MIME allowlist to
/// arbitrary executable text types (notably HTML and JavaScript).
fn is_source_file(filename: &str) -> bool {
    matches!(
        filename
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "c" | "cc"
            | "cpp"
            | "cs"
            | "css"
            | "go"
            | "h"
            | "hpp"
            | "java"
            | "js"
            | "jsx"
            | "kt"
            | "kts"
            | "lua"
            | "m"
            | "md"
            | "php"
            | "pl"
            | "pm"
            | "py"
            | "r"
            | "rb"
            | "rs"
            | "sh"
            | "sql"
            | "swift"
            | "toml"
            | "ts"
            | "tsx"
            | "vue"
            | "yaml"
            | "yml"
            | "zsh"
    )
}

fn mime_allowed(mime: &str) -> bool {
    let m = mime
        .split(';')
        .next()
        .unwrap_or(mime)
        .trim()
        .to_ascii_lowercase();
    ALLOWED_MIME_PREFIXES.iter().any(|p| m.starts_with(p))
        || ALLOWED_MIME_EXACT.iter().any(|e| *e == m)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentRecord {
    pub id: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: usize,
    pub download_path: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadAttachmentRequest {
    pub actor_id: String,
    pub filename: String,
    pub content_type: String,
    pub data_base64: String,
}

#[derive(Clone)]
pub struct AttachmentStore {
    root: Arc<PathBuf>,
    db: choruz_store::EventStore,
}

impl AttachmentStore {
    pub fn new(root: impl Into<PathBuf>, db: choruz_store::EventStore) -> Self {
        Self {
            root: Arc::new(root.into()),
            db,
        }
    }

    pub async fn put(
        &self,
        principal: &Principal,
        upload: UploadAttachmentRequest,
    ) -> AppResult<AttachmentRecord> {
        if upload.filename.trim().is_empty() {
            return Err(AppError::Validation("filename is required".into()));
        }
        if upload.content_type.trim().is_empty() {
            return Err(AppError::Validation("content_type is required".into()));
        }
        if upload.filename.contains('/') || upload.filename.contains('\\') {
            return Err(AppError::Validation(
                "filename must not contain path separators".into(),
            ));
        }

        let bytes = STANDARD
            .decode(upload.data_base64.as_bytes())
            .map_err(|error| {
                AppError::Validation(format!("invalid attachment payload: {error}"))
            })?;

        // 1) Size cap. Enforced here as a defence in depth — the HTTP body
        //    limit layer already blocks at the request level, but a caller
        //    could construct an oversized payload via other paths in future.
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(AppError::Validation(format!(
                "attachment exceeds {} bytes (got {})",
                MAX_ATTACHMENT_BYTES,
                bytes.len()
            )));
        }
        if bytes.is_empty() {
            return Err(AppError::Validation("attachment is empty".into()));
        }

        // 2) Magic-byte MIME re-detection. Never trust the client's content_type
        //    verbatim: a JS-typed file served same-origin with the right MIME
        //    is an XSS vector. `infer` returns None for types it doesn't know
        //    (most plain text); in that case we fall back to the claimed type
        //    but still require it to pass the allowlist.
        let detected = infer::get(&bytes).map(|t| t.mime_type().to_string());
        let mut effective_mime = detected.clone().unwrap_or_else(|| {
            upload
                .content_type
                .split(';')
                .next()
                .unwrap_or(&upload.content_type)
                .trim()
                .to_string()
        });
        if detected.is_none() && is_source_file(&upload.filename) {
            effective_mime = "text/plain".into();
        }
        if !mime_allowed(&effective_mime) {
            return Err(AppError::Validation(format!(
                "content_type {effective_mime} is not allowed"
            )));
        }
        // If the client claimed a specific MIME but magic bytes disagree at the
        // TYPE level (image vs. video vs. application), trust the bytes —
        // otherwise a PNG renamed to .html served as text/html owns the page.
        if let Some(ref actual) = detected {
            let claimed = upload.content_type.split('/').next().unwrap_or("");
            let actual_prefix = actual.split('/').next().unwrap_or("");
            if !claimed.is_empty() && !actual_prefix.is_empty() && claimed != actual_prefix {
                tracing::warn!(
                    claimed = %upload.content_type,
                    detected = %actual,
                    filename = %upload.filename,
                    "attachment MIME mismatch — storing with detected type"
                );
            }
        }

        let id = new_id();
        let storage_path = self.object_path(&id);
        let created_at = now();

        let attachment = AttachmentRecord {
            id: id.clone(),
            workspace_id: principal.workspace_id.clone(),
            owner_id: principal.id.clone(),
            filename: upload.filename,
            content_type: effective_mime,
            size_bytes: bytes.len(),
            download_path: format!("/v1/attachments/{}", id),
            created_at,
        };

        // Write file bytes to disk
        tokio::fs::create_dir_all(self.root.as_ref())
            .await
            .map_err(|error| {
                AppError::Internal(format!("failed to prepare attachment dir: {error}"))
            })?;
        tokio::fs::write(&storage_path, &bytes)
            .await
            .map_err(|error| AppError::Internal(format!("failed to write attachment: {error}")))?;

        // Insert metadata into DB
        let client = self.db.connect().await?;
        if let Err(error) = client
            .execute(
                "INSERT INTO attachment (id, workspace_id, owner_id, filename, content_type, size_bytes, storage_path, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &attachment.id,
                    &attachment.workspace_id,
                    &attachment.owner_id,
                    &attachment.filename,
                    &attachment.content_type,
                    &(attachment.size_bytes as i64),
                    &storage_path.to_string_lossy().as_ref(),
                    &attachment.created_at,
                ],
            )
            .await
        {
            match tokio::fs::remove_file(&storage_path).await {
                Ok(()) => {}
                Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => {}
                Err(cleanup_error) => {
                    return Err(AppError::Internal(format!(
                        "failed to insert attachment metadata: {error}; attachment bytes cleanup failed: {cleanup_error}"
                    )));
                }
            }
            return Err(AppError::Internal(format!("failed to insert attachment metadata: {error}")));
        }

        Ok(attachment)
    }

    pub async fn get(
        &self,
        principal: &Principal,
        attachment_id: &str,
    ) -> AppResult<(AttachmentRecord, Vec<u8>)> {
        let client = self.db.connect().await?;
        let row = client
            .query_opt(
                "SELECT id, workspace_id, owner_id, filename, content_type, size_bytes, storage_path, created_at
                 FROM attachment WHERE id = $1",
                &[&attachment_id],
            )
            .await
            .map_err(|error| AppError::Internal(format!("failed to query attachment: {error}")))?
            .ok_or_else(|| AppError::NotFound("attachment not found".into()))?;

        let storage_path: String = row.get("storage_path");
        let size_bytes: i64 = row.get("size_bytes");
        let workspace_id: String = row.get("workspace_id");
        let owner_id: String = row.get("owner_id");

        // Access rules, in priority order:
        //   1. Uploader/owner — always OK.
        //   2. Otherwise — must be a member of a conversation that
        //      references this attachment (so agents in a group can read
        //      images a human uploaded, even though the agent's workspace
        //      is the company and the uploader's is the human workspace).
        let is_owner = owner_id == principal.id;
        if !is_owner {
            let can_access = client
                .query_opt(
                    "SELECT 1 FROM conversation_events ce
                     JOIN conversation_member cm ON cm.conv_id = ce.conversation_id
                     WHERE ce.metadata->>'attachment_id' = $1
                       AND cm.principal_id = $2
                       AND cm.removed_at IS NULL
                     LIMIT 1",
                    &[&attachment_id, &principal.id],
                )
                .await
                .map_err(|error| {
                    AppError::Internal(format!("attachment membership check failed: {error}"))
                })?
                .is_some();
            if !can_access {
                return Err(AppError::Forbidden("attachment access denied".into()));
            }
        }

        let record = AttachmentRecord {
            id: row.get("id"),
            workspace_id,
            owner_id,
            filename: row.get("filename"),
            content_type: row.get("content_type"),
            size_bytes: size_bytes as usize,
            download_path: format!("/v1/attachments/{}", attachment_id),
            created_at: row.get("created_at"),
        };

        let bytes = tokio::fs::read(&storage_path)
            .await
            .map_err(|error| AppError::Internal(format!("failed to read attachment: {error}")))?;
        Ok((record, bytes))
    }

    pub async fn delete(&self, principal: &Principal, attachment_id: &str) -> AppResult<()> {
        let mut client = self.db.connect().await?;
        let tx = client.transaction().await.map_err(|error| {
            AppError::Internal(format!("failed to start attachment delete tx: {error}"))
        })?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&attachment_id],
        )
        .await
        .map_err(|error| {
            AppError::Internal(format!("failed to lock attachment for delete: {error}"))
        })?;

        let row = tx
            .query_opt(
                "SELECT owner_id, storage_path
                 FROM attachment
                 WHERE id = $1",
                &[&attachment_id],
            )
            .await
            .map_err(|error| {
                AppError::Internal(format!("failed to query attachment for delete: {error}"))
            })?
            .ok_or_else(|| AppError::NotFound("attachment not found".into()))?;

        let owner_id: String = row.get("owner_id");
        let storage_path: String = row.get("storage_path");
        let is_owner = owner_id == principal.id;
        if !is_owner {
            return Err(AppError::Forbidden("attachment delete denied".into()));
        }

        let referenced = tx
            .query_opt(
                "SELECT 1
                 FROM conversation_events
                 WHERE metadata->>'attachment_id' = $1
                 LIMIT 1",
                &[&attachment_id],
            )
            .await
            .map_err(|error| {
                AppError::Internal(format!("attachment reference check failed: {error}"))
            })?;
        if referenced.is_some() {
            return Err(AppError::Conflict(
                "attachment is already referenced by a message".into(),
            ));
        }
        match tokio::fs::remove_file(&storage_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "failed to delete attachment bytes: {error}"
                )));
            }
        }

        tx.execute("DELETE FROM attachment WHERE id = $1", &[&attachment_id])
            .await
            .map_err(|error| {
                AppError::Internal(format!("failed to delete attachment metadata: {error}"))
            })?;
        tx.commit().await.map_err(|error| {
            AppError::Internal(format!("failed to commit attachment delete: {error}"))
        })?;

        Ok(())
    }

    fn object_path(&self, attachment_id: &str) -> PathBuf {
        self.root.join(attachment_id)
    }
}
