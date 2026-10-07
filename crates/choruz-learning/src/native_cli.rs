//! Official CLI adapter for fixed learning procedures in fresh scratch conversations.
use choruz_agent_runtime::headless::{
    HeadlessDriver, configure_command_workspace, harness_account_env, parse_output,
};
use choruz_common::AppError;
use std::{
    process::Stdio,
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

/// Runs each learning call in a fresh bounded CLI process, never a live Agent session.
/// Dropping the future terminates its process group; no account credentials are copied.
pub struct CliRunner(pub CliConfig);

/// Device-local CLI selection; learning never attaches to a foreground workspace.
#[derive(Debug, Clone)]
pub struct CliConfig {
    pub driver_type: String,
    pub binary_path: Option<String>,
    pub model: Option<String>,
    pub harness_account: serde_json::Value,
}

impl crate::Runner for CliRunner {
    async fn run(
        &self,
        prompt: String,
        research: bool,
        system_prompt: &'static str,
    ) -> Result<String, AppError> {
        run_with_role(self.0.clone(), prompt, research, system_prompt).await
    }
}

async fn run_with_role(
    spec: CliConfig,
    prompt: String,
    research: bool,
    system_prompt: &str,
) -> Result<String, AppError> {
    let job = choruz_agent_runtime::process_scope::Job::begin(
        spec.harness_account["harness_account_id"].as_str(),
    )
    .map_err(|message| AppError::Conflict(message.into()))?;
    if prompt.len() > 256 * 1024 {
        return Err(AppError::Validation(
            "Learning input exceeds the analysis window".into(),
        ));
    }
    let driver = HeadlessDriver::from_driver_type(&spec.driver_type)
        .filter(|driver| matches!(driver, HeadlessDriver::Claude | HeadlessDriver::Codex))
        .ok_or_else(|| AppError::Validation("Unsupported analysis Harness".into()))?;
    let scratch = tempfile::tempdir()
        .map_err(|e| AppError::Internal(format!("create analysis workspace: {e}")))?;
    let binary = choruz_agent_runtime::executable::terminal_binary(
        &spec
            .driver_type
            .parse()
            .map_err(|e| AppError::Validation(format!("analysis driver: {e}")))?,
        spec.binary_path.as_deref(),
    );
    let mut command = Command::new(binary);
    configure_command_workspace(&mut command, driver, scratch.path());
    if let Some((key, directory)) =
        harness_account_env(driver, &spec.harness_account).map_err(AppError::Validation)?
    {
        command.env(key, directory);
    }
    match driver {
        HeadlessDriver::Claude => {
            command.args([
                "--print",
                "--output-format",
                "stream-json",
                "--verbose",
                "--tools",
                if research { "WebSearch" } else { "" },
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--no-session-persistence",
                "--safe-mode",
                "--disable-slash-commands",
                "--no-chrome",
                "--system-prompt",
                system_prompt,
            ]);
            if research {
                command.args(["--allowedTools", "WebSearch"]);
            }
        }
        HeadlessDriver::Codex => {
            command.args([
                "exec",
                "--json",
                "--skip-git-repo-check",
                "--sandbox",
                "read-only",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--disable",
                "shell_tool",
                "-c",
                if research {
                    "web_search=\"live\""
                } else {
                    "web_search=\"disabled\""
                },
            ]);
        }
        _ => unreachable!(),
    }
    if let Some(model) = &spec.model {
        command.args(["--model", model]);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    if job.is_cancelled() {
        return Err(AppError::Conflict(
            "Harness account is being removed".into(),
        ));
    }
    let mut child = command
        .spawn()
        .map_err(|e| AppError::Internal(format!("start analysis Harness: {e}")))?;
    let container = child.id().map(|pid| {
        choruz_agent_runtime::process::ProcessContainer::new(
            format!("experience-analysis-{pid}"),
            pid,
        )
    });
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppError::Internal("analysis input unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Internal("analysis output unavailable".into()))?;
    let phase = AtomicU8::new(0);
    let run = async {
        let send = async {
            stdin.write_all(prompt.as_bytes()).await?;
            drop(stdin);
            Ok::<_, std::io::Error>(())
        };
        let read = async {
            let mut output = Vec::new();
            let mut reader = BufReader::new(stdout.take(1024 * 1024 + 1));
            loop {
                let start = output.len();
                if reader.read_until(b'\n', &mut output).await? == 0 {
                    break;
                }
                if let Ok(event) = serde_json::from_slice::<serde_json::Value>(&output[start..]) {
                    let observed = if event["type"] == "result"
                        || event["item"]["type"] == "agent_message"
                    {
                        3
                    } else if event["item"]["type"] == "web_search"
                        || event["message"]["content"]
                            .as_array()
                            .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_use"))
                    {
                        2
                    } else {
                        1
                    };
                    phase.fetch_max(observed, Ordering::Relaxed);
                }
            }
            if output.len() > 1024 * 1024 {
                return Err(std::io::Error::other("analysis output exceeds limit"));
            }
            Ok(output)
        };
        tokio::try_join!(send, read, child.wait())
    };
    let outcome = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(75), run) => Some(result),
        _ = job.cancelled() => None,
    };
    let completed = match outcome {
        Some(Ok(result)) => {
            result.map_err(|e| AppError::Internal(format!("wait for analysis: {e}")))
        }
        None => Err(AppError::Conflict("Harness account was removed".into())),
        Some(Err(_)) => Err(AppError::Internal(format!(
            "Background analysis timed out ({}); foreground work is unaffected",
            match phase.load(Ordering::Relaxed) {
                0 => "waiting for Harness output",
                1 => "waiting for model report",
                2 => "waiting for search completion",
                _ => "waiting for process exit",
            }
        ))),
    };
    if completed.is_err() {
        if let Some(container) = &container {
            container.kill_all();
        }
        child
            .wait()
            .await
            .map_err(|e| AppError::Internal(format!("reap analysis: {e}")))?;
    }
    let (_, output, status) = completed?;
    if String::from_utf8_lossy(&output).lines().any(|line| {
        serde_json::from_str::<serde_json::Value>(line).is_ok_and(|event| {
            event["subtype"] == "model_refusal_no_fallback"
                || event["stop_reason"] == "refusal"
                || event["message"]["stop_reason"] == "refusal"
        })
    }) {
        return Err(AppError::Validation(
            "The selected model declined background analysis. No guidance was changed; review the provider's policy before continuing.".into(),
        ));
    }
    if !status.success() {
        return Err(AppError::Internal(
            "Analysis Harness failed; check the selected account and model".into(),
        ));
    }
    let parsed = parse_output(driver, &String::from_utf8_lossy(&output));
    if parsed.structured_error || (!research && parsed.tool_calls_count != 0) {
        return Err(AppError::Validation(
            "Analysis must return a report without executing tools".into(),
        ));
    }
    if research {
        let searched = completed_search(&String::from_utf8_lossy(&output));
        if !searched || parsed.response_text.is_empty() || parsed.response_text.len() > 8000 {
            return Err(AppError::Validation(
                "Research did not complete an observable web search".into(),
            ));
        }
    }
    Ok(parsed.response_text)
}

fn completed_search(output: &str) -> bool {
    let mut searches = std::collections::HashSet::new();
    let mut completed = false;
    for event in output
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
    {
        if event["type"] == "item.completed" {
            match event["item"]["type"].as_str() {
                Some("web_search") => completed = true,
                Some(
                    "command_execution" | "file_change" | "mcp_tool_call" | "collab_tool_call",
                ) => return false,
                _ => {}
            }
        }
        if let Some(blocks) = event["message"]["content"].as_array() {
            for block in blocks {
                if block["type"] == "tool_use" {
                    if block["name"] != "WebSearch" {
                        return false;
                    }
                    if let Some(id) = block["id"].as_str() {
                        searches.insert(id.to_owned());
                    }
                }
                if block["type"] == "tool_result"
                    && block["is_error"] != true
                    && block["tool_use_id"]
                        .as_str()
                        .is_some_and(|id| searches.contains(id))
                {
                    completed = true;
                }
            }
        }
    }
    completed
}

#[cfg(test)]
mod tests {
    use super::completed_search;

    #[cfg(unix)]
    #[tokio::test]
    async fn native_drivers_execute_in_scratch_and_reject_unsafe_reports() {
        use super::{CliConfig, CliRunner};
        use crate::Runner;
        use std::os::unix::fs::PermissionsExt;

        // Replace only the external model binary. Exercise real process creation,
        // stdin, arguments, workspace selection and production output validation.
        let fixture = tempfile::tempdir().unwrap();
        let binary = fixture.path().join("model");
        let args = fixture.path().join("args");
        let input = fixture.path().join("input");
        let cwd = fixture.path().join("cwd");
        for (driver, event, required) in [
            (
                "claude_terminal",
                r#"{"type":"result","result":"fixture report"}"#,
                vec![
                    "--print",
                    "--strict-mcp-config",
                    "--safe-mode",
                    "--no-session-persistence",
                ],
            ),
            (
                "codex_terminal",
                r#"{"type":"item.completed","item":{"type":"agent_message","text":"fixture report"}}"#,
                vec![
                    "exec",
                    "--ephemeral",
                    "--ignore-user-config",
                    "read-only",
                    "shell_tool",
                ],
            ),
        ] {
            let script = format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\npwd > '{}'\nprintf '%s\\n' '{}'\n",
                args.display(),
                input.display(),
                cwd.display(),
                event
            );
            std::fs::write(&binary, script).unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
            let runner = CliRunner(CliConfig {
                driver_type: driver.into(),
                binary_path: Some(binary.to_string_lossy().into_owned()),
                model: Some("fixture-model".into()),
                harness_account: serde_json::json!({}),
            });
            assert_eq!(
                runner
                    .run("private input".into(), false, "fixed role")
                    .await
                    .unwrap(),
                "fixture report"
            );
            assert_eq!(std::fs::read_to_string(&input).unwrap(), "private input");
            let observed = std::fs::read_to_string(&args).unwrap();
            for flag in required {
                assert!(observed.lines().any(|arg| arg == flag), "missing {flag}");
            }
            assert!(observed.contains("--model\nfixture-model\n"));
            let scratch = std::fs::read_to_string(&cwd).unwrap();
            assert!(
                !std::path::Path::new(scratch.trim()).exists(),
                "scratch survives completion"
            );

            std::fs::write(&binary, "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"type\":\"result\",\"stop_reason\":\"refusal\"}'\n").unwrap();
            assert!(
                runner
                    .run("input".into(), false, "role")
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("declined")
            );
            std::fs::write(&binary, "#!/bin/sh\ncat >/dev/null\nexit 7\n").unwrap();
            assert!(
                runner
                    .run("input".into(), false, "role")
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("Harness failed")
            );
        }
    }

    #[test]
    fn research_requires_a_completed_search_not_a_claim_or_failed_attempt() {
        let started = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"WebSearch","id":"s1"}]}}"#;
        assert!(!completed_search(started));
        let failed = format!(
            "{started}\n{}",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"s1","is_error":true}]}}"#
        );
        assert!(!completed_search(&failed));
        let successful = failed.replace("\"is_error\":true", "\"is_error\":false");
        assert!(completed_search(&successful));
        assert!(completed_search(
            r#"{"type":"item.completed","item":{"type":"web_search"}}"#
        ));
        assert!(!completed_search(
            r#"{"type":"item.started","item":{"type":"web_search"}}"#
        ));
        assert!(!completed_search(&format!(
            "{successful}\n{}",
            r#"{"type":"item.completed","item":{"type":"command_execution"}}"#
        )));
    }
}
