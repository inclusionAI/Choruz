//! CLI Adapter for Codex.
//!
//! Communication protocol:
//! 1. Write prompt to stdin of the Codex process
//! 2. Read structured output from the Codex session directory
//!
//! Codex uses a different session format than Claude Code but the overall
//! pattern is similar: inject → poll → extract.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing;

use crate::adapter::{CliAdapter, CliResponse, CliType, ToolCallRecord};
use crate::error::{ExecutorError, ExecutorResult};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum time to wait for a response before timing out.
/// Set to 24 hours — agents should run to completion without artificial timeouts.
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(86400);

/// Polling interval when watching the session output.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

// ---------------------------------------------------------------------------
// Codex Session structures
// ---------------------------------------------------------------------------

/// A record from the Codex session output.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type")]
enum CodexRecord {
    #[serde(rename = "message")]
    Message {
        role: String,
        #[serde(default)]
        content: String,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        #[serde(default)]
        call_id: String,
        name: String,
        #[serde(default)]
        arguments: serde_json::Value,
    },
    #[serde(rename = "function_call_output")]
    FunctionCallOutput {
        #[serde(default)]
        call_id: String,
        output: serde_json::Value,
    },
    #[serde(rename = "completed")]
    Completed,
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------------
// CodexAdapter
// ---------------------------------------------------------------------------

/// Adapter for Codex CLI.
pub struct CodexAdapter {
    /// Working directory (sandbox root).
    #[allow(dead_code)]
    work_dir: PathBuf,

    /// Path to the Codex session output directory.
    session_dir: PathBuf,

    /// Last read position in the session file (byte offset).
    read_offset: Arc<Mutex<u64>>,

    /// PTY stdin writer.
    pty_stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,

    /// The child process handle.
    child: Arc<Mutex<Option<tokio::process::Child>>>,

    /// Read timeout.
    read_timeout: Duration,
}

impl CodexAdapter {
    /// Create a new Codex adapter.
    pub fn new(work_dir: PathBuf, session_dir: PathBuf) -> Self {
        Self {
            work_dir,
            session_dir,
            read_offset: Arc::new(Mutex::new(0)),
            pty_stdin: Arc::new(Mutex::new(None)),
            child: Arc::new(Mutex::new(None)),
            read_timeout: DEFAULT_READ_TIMEOUT,
        }
    }

    /// Set a custom read timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    /// Set the PTY stdin writer.
    pub async fn set_pty_stdin(&self, stdin: tokio::process::ChildStdin) {
        let mut guard = self.pty_stdin.lock().await;
        *guard = Some(stdin);
    }

    /// Set the child process handle.
    pub async fn set_child(&self, child: tokio::process::Child) {
        let mut guard = self.child.lock().await;
        *guard = Some(child);
    }

    /// Find the most recent session file in the Codex session directory.
    async fn find_session_file(&self) -> ExecutorResult<Option<PathBuf>> {
        if !self.session_dir.exists() {
            return Ok(None);
        }

        let mut entries = tokio::fs::read_dir(&self.session_dir).await.map_err(|e| {
            ExecutorError::Adapter(format!(
                "failed to read codex session dir {}: {e}",
                self.session_dir.display()
            ))
        })?;

        let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ExecutorError::Adapter(format!("failed to read dir entry: {e}")))?
        {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == "jsonl" || ext == "json")
            {
                let metadata = entry
                    .metadata()
                    .await
                    .map_err(|e| ExecutorError::Adapter(format!("failed to read metadata: {e}")))?;
                let modified = metadata
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

                match &newest {
                    None => newest = Some((path, modified)),
                    Some((_, prev_time)) if modified > *prev_time => {
                        newest = Some((path, modified));
                    }
                    _ => {}
                }
            }
        }

        Ok(newest.map(|(path, _)| path))
    }

    /// Parse new records from the Codex session file starting at the current offset.
    async fn read_new_records(&self) -> ExecutorResult<Vec<CodexRecord>> {
        let session_file = match self.find_session_file().await? {
            Some(f) => f,
            None => return Ok(vec![]),
        };

        let content = tokio::fs::read_to_string(&session_file)
            .await
            .map_err(|e| {
                ExecutorError::Adapter(format!(
                    "failed to read codex session file {}: {e}",
                    session_file.display()
                ))
            })?;

        let mut offset = self.read_offset.lock().await;
        let bytes = content.as_bytes();

        if (*offset as usize) >= bytes.len() {
            return Ok(vec![]);
        }

        let new_content = &content[(*offset as usize)..];
        let mut records = Vec::new();

        for line in new_content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<CodexRecord>(trimmed) {
                Ok(record) => records.push(record),
                Err(e) => {
                    tracing::warn!(
                        line = trimmed,
                        error = %e,
                        "failed to parse codex session line, skipping"
                    );
                }
            }
        }

        *offset = bytes.len() as u64;
        Ok(records)
    }

    /// Extract text and tool calls from codex records.
    fn extract_response(records: &[CodexRecord]) -> (String, Vec<ToolCallRecord>, bool) {
        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();
        let mut is_complete = false;

        for record in records {
            match record {
                CodexRecord::Message { role, content }
                    if role == "assistant" && !content.is_empty() =>
                {
                    text_parts.push(content.clone());
                }
                CodexRecord::FunctionCall {
                    call_id,
                    name,
                    arguments,
                } => {
                    tool_calls.push(ToolCallRecord {
                        tool_call_id: call_id.clone(),
                        name: name.clone(),
                        input: arguments.clone(),
                        output: serde_json::Value::Null,
                    });
                }
                CodexRecord::FunctionCallOutput { call_id, output } => {
                    if let Some(tc) = tool_calls.iter_mut().find(|tc| tc.tool_call_id == *call_id) {
                        tc.output = output.clone();
                    }
                }
                CodexRecord::Completed => {
                    is_complete = true;
                }
                _ => {}
            }
        }

        (text_parts.join(""), tool_calls, is_complete)
    }
}

#[async_trait]
impl CliAdapter for CodexAdapter {
    fn cli_type(&self) -> CliType {
        CliType::Codex
    }

    async fn inject_prompt(&self, prompt: &str) -> ExecutorResult<()> {
        // Codex reads from stdin directly
        let mut stdin_guard = self.pty_stdin.lock().await;
        if let Some(ref mut stdin) = *stdin_guard {
            let formatted = format!("{prompt}\n");
            stdin.write_all(formatted.as_bytes()).await.map_err(|e| {
                ExecutorError::Adapter(format!("failed to write to Codex stdin: {e}"))
            })?;
            stdin
                .flush()
                .await
                .map_err(|e| ExecutorError::Adapter(format!("failed to flush Codex stdin: {e}")))?;

            tracing::debug!(prompt_len = prompt.len(), "injected prompt to Codex stdin");
        } else {
            return Err(ExecutorError::Adapter(
                "no PTY stdin available for Codex".into(),
            ));
        }

        Ok(())
    }

    async fn read_response(&self) -> ExecutorResult<CliResponse> {
        let deadline = tokio::time::Instant::now() + self.read_timeout;

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(ExecutorError::Timeout(
                    "timed out waiting for Codex response".into(),
                ));
            }

            let records = self.read_new_records().await?;
            if !records.is_empty() {
                let (content, tool_calls, is_complete) = Self::extract_response(&records);
                return Ok(CliResponse {
                    content,
                    is_complete,
                    tool_calls,
                });
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    async fn is_alive(&self) -> bool {
        let mut guard = self.child.lock().await;
        match guard.as_mut() {
            Some(child) => child.try_wait().ok().flatten().is_none(),
            None => false,
        }
    }

    async fn terminate(&self) -> ExecutorResult<()> {
        let mut guard = self.child.lock().await;
        if let Some(ref mut child) = *guard {
            child.kill().await.map_err(|e| {
                ExecutorError::Adapter(format!("failed to kill Codex process: {e}"))
            })?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codex_message_record() {
        let json = r#"{"type":"message","role":"assistant","content":"Hello!"}"#;
        let record: CodexRecord = serde_json::from_str(json).unwrap();
        match record {
            CodexRecord::Message { role, content } => {
                assert_eq!(role, "assistant");
                assert_eq!(content, "Hello!");
            }
            _ => panic!("expected message record"),
        }
    }

    #[test]
    fn parse_codex_function_call() {
        let json =
            r#"{"type":"function_call","call_id":"fc-1","name":"shell","arguments":{"cmd":"ls"}}"#;
        let record: CodexRecord = serde_json::from_str(json).unwrap();
        match record {
            CodexRecord::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(call_id, "fc-1");
                assert_eq!(name, "shell");
                assert_eq!(arguments["cmd"], "ls");
            }
            _ => panic!("expected function_call record"),
        }
    }

    #[test]
    fn parse_codex_completed() {
        let json = r#"{"type":"completed"}"#;
        let record: CodexRecord = serde_json::from_str(json).unwrap();
        assert!(matches!(record, CodexRecord::Completed));
    }

    #[test]
    fn extract_response_from_codex_records() {
        let records = vec![
            CodexRecord::Message {
                role: "assistant".into(),
                content: "I will list files.".into(),
            },
            CodexRecord::FunctionCall {
                call_id: "fc-1".into(),
                name: "shell".into(),
                arguments: serde_json::json!({"cmd": "ls"}),
            },
            CodexRecord::FunctionCallOutput {
                call_id: "fc-1".into(),
                output: serde_json::json!("file1.txt"),
            },
            CodexRecord::Completed,
        ];

        let (content, tool_calls, is_complete) = CodexAdapter::extract_response(&records);
        assert_eq!(content, "I will list files.");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "shell");
        assert_eq!(tool_calls[0].output, serde_json::json!("file1.txt"));
        assert!(is_complete);
    }

    #[test]
    fn unknown_codex_record_skipped() {
        let json = r#"{"type":"debug","data":"internal"}"#;
        let record: CodexRecord = serde_json::from_str(json).unwrap();
        assert!(matches!(record, CodexRecord::Unknown));
    }

    #[test]
    fn extract_response_ignores_user_messages() {
        let records = vec![
            CodexRecord::Message {
                role: "user".into(),
                content: "user input".into(),
            },
            CodexRecord::Message {
                role: "assistant".into(),
                content: "assistant output".into(),
            },
        ];

        let (content, tool_calls, is_complete) = CodexAdapter::extract_response(&records);
        assert_eq!(content, "assistant output");
        assert!(tool_calls.is_empty());
        assert!(!is_complete);
    }
}
