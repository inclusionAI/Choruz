//! Shared CLI adapter trait and types.
//!
//! All CLI adapters (Claude Code, Codex, etc.) implement the [`CliAdapter`]
//! trait, providing a uniform interface for prompt injection and response
//! reading.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ExecutorResult;

// ---------------------------------------------------------------------------
// CLI type enum
// ---------------------------------------------------------------------------

/// The type of CLI being adapted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliType {
    /// Claude Code CLI (`claude`).
    ClaudeCode,
    /// OpenAI Codex CLI.
    Codex,
}

// ---------------------------------------------------------------------------
// Tool call record
// ---------------------------------------------------------------------------

/// A tool call observed in the CLI response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// The unique ID assigned to this tool call by the CLI.
    pub tool_call_id: String,
    /// The tool name (e.g. `"bash"`, `"file_write"`).
    pub name: String,
    /// The input arguments passed to the tool.
    pub input: serde_json::Value,
    /// The output returned by the tool (populated after execution).
    pub output: serde_json::Value,
}

// ---------------------------------------------------------------------------
// CLI response
// ---------------------------------------------------------------------------

/// A response read from the CLI adapter.
#[derive(Debug, Clone)]
pub struct CliResponse {
    /// The text content of the response.
    pub content: String,
    /// Whether the turn is complete (the CLI has finished responding).
    pub is_complete: bool,
    /// Tool calls observed during the response.
    pub tool_calls: Vec<ToolCallRecord>,
}

// ---------------------------------------------------------------------------
// CLI adapter trait
// ---------------------------------------------------------------------------

/// Trait for CLI adapters that manage prompt injection and response reading.
#[async_trait]
pub trait CliAdapter: Send + Sync {
    /// Return the type of CLI this adapter manages.
    fn cli_type(&self) -> CliType;

    /// Inject a prompt into the CLI process.
    async fn inject_prompt(&self, prompt: &str) -> ExecutorResult<()>;

    /// Read the next response from the CLI process.
    ///
    /// This may block until new output is available or a timeout is reached.
    async fn read_response(&self) -> ExecutorResult<CliResponse>;

    /// Check if the underlying CLI process is still alive.
    async fn is_alive(&self) -> bool;

    /// Terminate the underlying CLI process.
    async fn terminate(&self) -> ExecutorResult<()>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_type_serde() {
        let json = serde_json::to_string(&CliType::ClaudeCode).unwrap();
        assert_eq!(json, "\"claude_code\"");
        let parsed: CliType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, CliType::ClaudeCode);
    }

    #[test]
    fn tool_call_record_serde() {
        let record = ToolCallRecord {
            tool_call_id: "tc-1".into(),
            name: "bash".into(),
            input: serde_json::json!({"cmd": "ls"}),
            output: serde_json::json!("file.txt"),
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: ToolCallRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "bash");
    }

    #[test]
    fn cli_response_construction() {
        let resp = CliResponse {
            content: "Hello".into(),
            is_complete: true,
            tool_calls: vec![],
        };
        assert!(resp.is_complete);
        assert!(resp.tool_calls.is_empty());
    }
}
