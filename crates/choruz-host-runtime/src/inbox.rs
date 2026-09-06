//! Incoming attachments staged into an agent workspace before a turn.
//!
//! A message that carries files reaches the agent as a prompt plus
//! `metadata.attachments`. Whichever device runs the turn fetches each file
//! into `<workspace>/.choruz-inbox/<attachment_id>/<filename>` and appends
//! the local paths to the prompt, so the agent reads them with plain
//! filesystem tools and never authenticates against the platform itself.
//! The pipeline fetches with the agent's own token; the connector fetches
//! through the host-facing command attachment route.

use std::future::Future;
use std::path::Path;

use serde_json::Value;

/// One file the router attached to a turn's metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingAttachment {
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
}

/// The attachments a command's metadata names, in message order. Entries
/// without an `attachment_id` are ignored.
pub fn incoming_attachments(metadata: &Value) -> Vec<IncomingAttachment> {
    metadata
        .get("attachments")
        .and_then(Value::as_array)
        .map(|attachments| {
            attachments
                .iter()
                .filter_map(|attachment| {
                    let attachment_id = attachment.get("attachment_id")?.as_str()?;
                    Some(IncomingAttachment {
                        attachment_id: attachment_id.to_owned(),
                        filename: attachment
                            .get("filename")
                            .and_then(Value::as_str)
                            .unwrap_or("attachment.bin")
                            .to_owned(),
                        mime_type: attachment
                            .get("mime_type")
                            .and_then(Value::as_str)
                            .unwrap_or("application/octet-stream")
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A path component of a staged attachment (its id as the directory, the
/// name the sender gave as the file) minus anything that could leave the
/// inbox.
pub fn staged_file_name(filename: &str) -> String {
    let safe: String = filename
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | '\0'))
        .collect();
    if safe.is_empty() {
        "attachment.bin".into()
    } else {
        safe
    }
}

/// Fetch every attachment not yet staged with `fetch`, write it under the
/// workspace inbox, and return the prompt with the staged paths appended.
/// Fetching is best effort: a file that cannot be fetched is left out of
/// the list and the agent still sees the caption in the prompt.
pub async fn stage_incoming_attachments<F, Fut>(
    prompt: &str,
    work_dir: &Path,
    attachments: &[IncomingAttachment],
    fetch: F,
) -> String
where
    F: Fn(&IncomingAttachment) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let inbox_root = work_dir.join(".choruz-inbox");
    let mut staged_lines = Vec::new();
    for attachment in attachments {
        let file_name = staged_file_name(&attachment.filename);
        let dest_dir = inbox_root.join(staged_file_name(&attachment.attachment_id));
        let dest_path = dest_dir.join(&file_name);
        // A retried or re-triggered turn stages the same files again; keep
        // the copy that is already there.
        if tokio::fs::metadata(&dest_path).await.is_err() {
            let bytes = match fetch(attachment).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!(
                        attachment_id = %attachment.attachment_id,
                        %error,
                        "incoming attachment was not staged"
                    );
                    continue;
                }
            };
            if let Err(error) = tokio::fs::create_dir_all(&dest_dir).await {
                tracing::warn!(attachment_id = %attachment.attachment_id, %error, "inbox directory was not created");
                continue;
            }
            if let Err(error) = tokio::fs::write(&dest_path, &bytes).await {
                tracing::warn!(attachment_id = %attachment.attachment_id, %error, "incoming attachment was not written");
                continue;
            }
            tracing::info!(
                attachment_id = %attachment.attachment_id,
                path = %dest_path.display(),
                size = bytes.len(),
                mime = %attachment.mime_type,
                "staged incoming attachment"
            );
        }
        staged_lines.push(format!(
            "- {} ({}): {}",
            file_name,
            attachment.mime_type,
            dest_path.display()
        ));
    }
    if staged_lines.is_empty() {
        return prompt.to_owned();
    }
    format!(
        "{prompt}\n\n[attached files available locally — read them as needed]\n{}",
        staged_lines.join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    #[test]
    fn metadata_attachments_keep_message_order_and_defaults() {
        let attachments = incoming_attachments(&json!({
            "attachments": [
                {"attachment_id": "a1", "filename": "brief.txt", "mime_type": "text/plain"},
                {"filename": "no-id.txt"},
                {"attachment_id": "a2"}
            ]
        }));
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].filename, "brief.txt");
        assert_eq!(attachments[1].filename, "attachment.bin");
        assert_eq!(attachments[1].mime_type, "application/octet-stream");
        assert!(incoming_attachments(&json!({})).is_empty());
    }

    #[test]
    fn staged_names_cannot_leave_their_directory() {
        assert_eq!(staged_file_name("../../etc/passwd"), "....etcpasswd");
        assert_eq!(staged_file_name("a\\b\0c"), "abc");
        assert_eq!(staged_file_name("///"), "attachment.bin");
    }

    #[tokio::test]
    async fn staging_fetches_missing_files_once_and_lists_them_in_the_prompt() {
        let workspace = tempfile::tempdir().unwrap();
        let attachments = vec![
            IncomingAttachment {
                attachment_id: "a1".into(),
                filename: "brief.txt".into(),
                mime_type: "text/plain".into(),
            },
            IncomingAttachment {
                attachment_id: "a2".into(),
                filename: "missing.bin".into(),
                mime_type: "application/octet-stream".into(),
            },
        ];
        let fetched = Mutex::new(Vec::new());
        let fetch = |attachment: &IncomingAttachment| {
            fetched
                .lock()
                .unwrap()
                .push(attachment.attachment_id.clone());
            let id = attachment.attachment_id.clone();
            async move {
                if id == "a1" {
                    Ok(b"attached-data".to_vec())
                } else {
                    Err("gone".to_owned())
                }
            }
        };
        let prompt =
            stage_incoming_attachments("Read this", workspace.path(), &attachments, fetch).await;
        let staged = workspace
            .path()
            .join(".choruz-inbox")
            .join("a1")
            .join("brief.txt");
        assert_eq!(std::fs::read(&staged).unwrap(), b"attached-data");
        assert!(prompt.starts_with("Read this\n\n[attached files"));
        assert!(prompt.contains(&staged.display().to_string()));
        assert!(
            !prompt.contains("missing.bin"),
            "a file that could not be fetched is not promised to the agent"
        );

        let again =
            stage_incoming_attachments("Read this", workspace.path(), &attachments[..1], fetch)
                .await;
        assert_eq!(again, prompt);
        assert_eq!(
            fetched.lock().unwrap().as_slice(),
            ["a1", "a2"],
            "an attachment already in the inbox is not fetched again"
        );
    }

    #[tokio::test]
    async fn no_attachments_leave_the_prompt_untouched() {
        let workspace = tempfile::tempdir().unwrap();
        let prompt = stage_incoming_attachments(
            "plain",
            workspace.path(),
            &[],
            |_: &IncomingAttachment| async { Ok(Vec::new()) },
        )
        .await;
        assert_eq!(prompt, "plain");
        assert!(!workspace.path().join(".choruz-inbox").exists());
    }
}
