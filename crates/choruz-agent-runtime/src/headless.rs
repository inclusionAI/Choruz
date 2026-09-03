use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadlessDriver {
    Claude,
    Codex,
    Pi,
    Grok,
    OpenCode,
    MathCode,
}

/// Apply the binding workspace to a headless CLI command.
///
/// OpenCode additionally resolves `--dir .` through `PWD`. Other Harnesses
/// must retain their inherited `PWD`; changing it globally alters their
/// observable path semantics on systems where logical and physical paths
/// differ (for example `/var` and `/private/var` on macOS).
pub fn configure_command_workspace(
    command: &mut tokio::process::Command,
    driver: HeadlessDriver,
    workspace: &Path,
) {
    command.current_dir(workspace);
    if driver == HeadlessDriver::OpenCode {
        // Keep OpenCode's logical directory aligned with the physical cwd
        // used by session discovery (notably `/private/var` on macOS).
        let pwd = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        command.env("PWD", pwd);
    }
}

/// Resolve the device-local profile selected for this binding. Only the
/// server-written account id and profile kind are accepted; bindings cannot
/// inject an arbitrary credential path or environment variable.
pub fn harness_account_env(
    driver: HeadlessDriver,
    config: &serde_json::Value,
) -> Result<Option<(&'static str, PathBuf)>, String> {
    if config
        .get("harness_account_profile_kind")
        .and_then(|value| value.as_str())
        != Some("isolated")
    {
        return harness_account_env_with_root(driver, config, Path::new("."));
    }
    let root = std::env::var_os("CHORUZ_HARNESS_ACCOUNT_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".choruz/accounts"))
        })
        .ok_or_else(|| "HOME is unavailable for harness account profiles".to_owned())?;
    harness_account_env_with_root(driver, config, &root)
}

fn harness_account_env_with_root(
    driver: HeadlessDriver,
    config: &serde_json::Value,
    root: &Path,
) -> Result<Option<(&'static str, PathBuf)>, String> {
    let Some(account_id) = config
        .get("harness_account_id")
        .and_then(|value| value.as_str())
    else {
        return Ok(None);
    };
    let profile_kind = config
        .get("harness_account_profile_kind")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "harness account profile kind is missing".to_owned())?;
    if profile_kind == "default" {
        return Ok(None);
    }
    if profile_kind != "isolated"
        || account_id.len() != 36
        || !account_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err("harness account profile metadata is invalid".to_owned());
    }
    match driver {
        HeadlessDriver::Claude => Ok(Some((
            "CLAUDE_CONFIG_DIR",
            root.join(account_id).join("claude"),
        ))),
        HeadlessDriver::Codex => Ok(Some(("CODEX_HOME", root.join(account_id).join("codex")))),
        _ => Err("selected harness accounts are not supported by this driver".to_owned()),
    }
}

#[cfg(test)]
mod account_tests {
    use super::{HeadlessDriver, harness_account_env_with_root};
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn isolated_profiles_map_only_to_the_selected_harness_environment() {
        let id = "12345678-1234-1234-1234-123456789abc";
        let config = json!({
            "harness_account_id": id,
            "harness_account_profile_kind": "isolated"
        });
        assert_eq!(
            harness_account_env_with_root(HeadlessDriver::Claude, &config, Path::new("/accounts")),
            Ok(Some((
                "CLAUDE_CONFIG_DIR",
                Path::new("/accounts").join(id).join("claude")
            )))
        );
        assert_eq!(
            harness_account_env_with_root(HeadlessDriver::Codex, &config, Path::new("/accounts")),
            Ok(Some((
                "CODEX_HOME",
                Path::new("/accounts").join(id).join("codex")
            )))
        );
        assert!(
            harness_account_env_with_root(HeadlessDriver::Pi, &config, Path::new("/accounts"))
                .is_err()
        );
    }

    #[test]
    fn default_profile_inherits_login_and_malformed_metadata_fails_closed() {
        let default = json!({
            "harness_account_id": "12345678-1234-1234-1234-123456789abc",
            "harness_account_profile_kind": "default"
        });
        assert_eq!(
            harness_account_env_with_root(HeadlessDriver::Claude, &default, Path::new("/accounts")),
            Ok(None)
        );
        let malicious = json!({
            "harness_account_id": "../../other-profile",
            "harness_account_profile_kind": "isolated"
        });
        assert!(
            harness_account_env_with_root(
                HeadlessDriver::Codex,
                &malicious,
                Path::new("/accounts")
            )
            .is_err()
        );
    }
}

impl HeadlessDriver {
    pub fn from_driver_type(driver_type: &str) -> Option<Self> {
        match driver_type {
            "claude_print" | "claude_terminal" => Some(Self::Claude),
            "codex_exec" | "codex_terminal" | "codex_app_server" => Some(Self::Codex),
            "pi_terminal" => Some(Self::Pi),
            "grok_terminal" => Some(Self::Grok),
            "opencode_terminal" => Some(Self::OpenCode),
            "mathcode_terminal" => Some(Self::MathCode),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
            Self::MathCode => "mathcode",
        }
    }

    pub fn default_binary(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
            Self::MathCode => "mathcode",
        }
    }

    pub fn args(
        self,
        resume_session_id: Option<&str>,
        model: Option<&str>,
        prompt: &str,
    ) -> Vec<String> {
        let resume = resume_session_id.filter(|value| !value.is_empty());
        match self {
            Self::Claude => {
                let mut args = vec![
                    "--print".into(),
                    "--output-format".into(),
                    "stream-json".into(),
                    "--dangerously-skip-permissions".into(),
                    "--verbose".into(),
                ];
                if let Some(session_id) = resume {
                    args.extend(["--resume".into(), session_id.into()]);
                }
                if let Some(model) = model {
                    args.extend(["--model".into(), model.into()]);
                }
                args.extend(["--".into(), prompt.into()]);
                args
            }
            Self::Codex => {
                let mut args = vec!["exec".into()];
                if let Some(session_id) = resume {
                    args.extend(["resume".into(), session_id.into()]);
                }
                args.extend([
                    "--json".into(),
                    "--skip-git-repo-check".into(),
                    "--dangerously-bypass-approvals-and-sandbox".into(),
                    "--config".into(),
                    "check_for_update_on_startup=false".into(),
                ]);
                if let Some(model) = model {
                    args.extend(["--model".into(), model.into()]);
                }
                args.push(prompt.into());
                args
            }
            Self::Pi => {
                let mut args = vec!["--mode".into(), "json".into(), "--approve".into()];
                if let Some(session_id) = resume {
                    args.extend(["--session".into(), session_id.into()]);
                }
                if let Some(model) = model {
                    args.extend(["--model".into(), model.into()]);
                }
                args.push(prompt.into());
                args
            }
            Self::Grok => {
                let mut args = vec![
                    "--no-auto-update".into(),
                    "-p".into(),
                    prompt.into(),
                    "--output-format".into(),
                    "streaming-json".into(),
                    "--always-approve".into(),
                ];
                if let Some(session_id) = resume {
                    args.extend(["--resume".into(), session_id.into()]);
                }
                if let Some(model) = model {
                    args.extend(["--model".into(), model.into()]);
                }
                args
            }
            Self::OpenCode => {
                let mut args = vec![
                    "run".into(),
                    "--format".into(),
                    "json".into(),
                    "--auto".into(),
                    // OpenCode otherwise promotes a nested working directory
                    // to the enclosing Git root. Pin the CLI project to the
                    // binding workspace selected by Command::current_dir.
                    "--dir".into(),
                    ".".into(),
                ];
                if let Some(session_id) = resume {
                    args.extend(["--session".into(), session_id.into()]);
                }
                if let Some(model) = model {
                    args.extend(["--model".into(), model.into()]);
                }
                args.push(prompt.into());
                args
            }
            Self::MathCode => vec!["-p".into(), prompt.into()],
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedOutput {
    pub response_text: String,
    pub session_id: Option<String>,
    pub tool_calls_count: i32,
    pub structured_error: bool,
}

/// Pi emits an assistant `message_end` with a successful process exit even
/// when the turn itself failed. Both terminal failure reasons must therefore
/// be interpreted from the structured message rather than the exit status.
pub fn pi_message_is_failed(message: &serde_json::Value) -> bool {
    message.get("role").and_then(|value| value.as_str()) == Some("assistant")
        && matches!(
            message.get("stopReason").and_then(|value| value.as_str()),
            Some("error" | "aborted")
        )
}

pub fn parse_output(driver: HeadlessDriver, stdout: &str) -> ParsedOutput {
    if driver == HeadlessDriver::MathCode {
        return ParsedOutput {
            response_text: stdout.trim().to_owned(),
            ..ParsedOutput::default()
        };
    }
    let mut parsed = ParsedOutput::default();
    for line in stdout.lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let event_type = event
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let session_id = match driver {
            HeadlessDriver::Claude if event_type == "system" => {
                event.get("session_id").and_then(|value| value.as_str())
            }
            HeadlessDriver::Codex if event_type == "thread.started" => {
                event.get("thread_id").and_then(|value| value.as_str())
            }
            HeadlessDriver::Pi if event_type == "session" => {
                event.get("id").and_then(|value| value.as_str())
            }
            HeadlessDriver::Grok if event_type == "end" => {
                event.get("sessionId").and_then(|value| value.as_str())
            }
            HeadlessDriver::OpenCode => event.get("sessionID").and_then(|value| value.as_str()),
            _ => None,
        };
        if let Some(session_id) = session_id.filter(|value| !value.is_empty()) {
            parsed.session_id = Some(session_id.to_owned());
        }
        match driver {
            HeadlessDriver::Claude if event_type == "result" => {
                if let Some(text) = event.get("result").and_then(|value| value.as_str()) {
                    parsed.response_text = text.to_owned();
                }
            }
            HeadlessDriver::Claude if event_type == "assistant" => {
                if let Some(content) = event
                    .get("message")
                    .and_then(|value| value.get("content"))
                    .and_then(|value| value.as_array())
                {
                    for block in content {
                        if block.get("type").and_then(|value| value.as_str()) == Some("tool_use") {
                            parsed.tool_calls_count = parsed.tool_calls_count.saturating_add(1);
                        }
                        if parsed.response_text.is_empty()
                            && let Some(text) = block.get("text").and_then(|value| value.as_str())
                        {
                            parsed.response_text = text.to_owned();
                        }
                    }
                }
            }
            HeadlessDriver::Codex if event_type == "item.completed" => {
                if let Some(item) = event.get("item") {
                    if item.get("type").and_then(|value| value.as_str()) == Some("agent_message")
                        && let Some(text) = item.get("text").and_then(|value| value.as_str())
                    {
                        parsed.response_text = text.to_owned();
                    } else if item.get("type").and_then(|value| value.as_str())
                        == Some("command_execution")
                    {
                        parsed.tool_calls_count = parsed.tool_calls_count.saturating_add(1);
                    }
                }
            }
            HeadlessDriver::Pi if event_type == "message_end" => {
                let Some(message) = event.get("message") else {
                    continue;
                };
                if message.get("role").and_then(|value| value.as_str()) != Some("assistant") {
                    continue;
                }
                if pi_message_is_failed(message) {
                    parsed.structured_error = true;
                    continue;
                }
                if let Some(content) = message.get("content") {
                    if let Some(text) = content.as_str() {
                        parsed.response_text = text.to_owned();
                    } else if let Some(parts) = content.as_array() {
                        parsed.response_text = parts
                            .iter()
                            .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
                            .collect();
                    }
                }
            }
            HeadlessDriver::Grok if event_type == "text" => {
                if let Some(text) = event.get("data").and_then(|value| value.as_str()) {
                    parsed.response_text.push_str(text);
                }
            }
            HeadlessDriver::OpenCode if event_type == "text" => {
                if let Some(text) = event
                    .get("part")
                    .and_then(|value| value.get("text"))
                    .and_then(|value| value.as_str())
                {
                    parsed.response_text.push_str(text);
                }
            }
            _ => {}
        }
    }
    parsed.response_text = parsed.response_text.trim().to_owned();
    parsed
}

pub fn validate_model(value: &str) -> Result<&str, &'static str> {
    if value.len() > 256 {
        return Err("configured model is too long");
    }
    if value.starts_with('-') || value.chars().any(char::is_control) {
        return Err("configured model contains unsafe characters");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_opencode_overrides_pwd_for_relative_dir_resolution() {
        let workspace = std::env::temp_dir().join(format!(
            "choruz-binding-workspace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for driver in [
            HeadlessDriver::Claude,
            HeadlessDriver::Codex,
            HeadlessDriver::Pi,
            HeadlessDriver::Grok,
            HeadlessDriver::OpenCode,
            HeadlessDriver::MathCode,
        ] {
            let mut command = tokio::process::Command::new(driver.default_binary());
            configure_command_workspace(&mut command, driver, &workspace);
            let command = command.as_std();
            assert_eq!(command.get_current_dir(), Some(workspace.as_path()));
            let pwd = command
                .get_envs()
                .find(|(name, _)| *name == "PWD")
                .and_then(|(_, value)| value);
            if driver == HeadlessDriver::OpenCode {
                assert_eq!(pwd, Some(workspace.as_os_str()));
            } else {
                assert_eq!(pwd, None, "{} must inherit PWD", driver.label());
            }
        }
    }

    #[test]
    fn codex_args_resume_exact_session_and_model() {
        assert_eq!(
            HeadlessDriver::Codex.args(Some("thread-1"), Some("gpt-5.6-codex"), "hello"),
            vec![
                "exec",
                "resume",
                "thread-1",
                "--json",
                "--skip-git-repo-check",
                "--dangerously-bypass-approvals-and-sandbox",
                "--config",
                "check_for_update_on_startup=false",
                "--model",
                "gpt-5.6-codex",
                "hello",
            ]
        );
    }

    #[test]
    fn opencode_args_pin_the_binding_workspace_for_new_and_resumed_sessions() {
        for session_id in [None, Some("session-1")] {
            let args =
                HeadlessDriver::OpenCode.args(session_id, Some("opencode/mimo-v2.5-free"), "hello");
            assert_eq!(
                args.windows(2).find(|pair| pair[0] == "--dir"),
                Some(["--dir".to_owned(), ".".to_owned()].as_slice())
            );
            assert_eq!(args.last().map(String::as_str), Some("hello"));
        }
    }

    #[test]
    fn mathcode_uses_its_documented_prompt_mode_and_preserves_plain_output() {
        assert_eq!(
            HeadlessDriver::MathCode.args(
                Some("ignored-session"),
                Some("ignored-model"),
                "prove it"
            ),
            vec!["-p", "prove it"],
        );
        let parsed = parse_output(HeadlessDriver::MathCode, "proof output\n");
        assert_eq!(parsed.response_text, "proof output");
        assert_eq!(parsed.session_id, None);
    }

    #[test]
    fn parses_each_driver_session_and_response() {
        let cases = [
            (
                HeadlessDriver::Claude,
                "{\"type\":\"system\",\"session_id\":\"c1\"}\n{\"type\":\"result\",\"result\":\"claude reply\"}",
                "c1",
                "claude reply",
            ),
            (
                HeadlessDriver::Codex,
                "{\"type\":\"thread.started\",\"thread_id\":\"x1\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"codex reply\"}}",
                "x1",
                "codex reply",
            ),
            (
                HeadlessDriver::Pi,
                "{\"type\":\"session\",\"id\":\"p1\"}\n{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"content\":\"pi reply\"}}",
                "p1",
                "pi reply",
            ),
            (
                HeadlessDriver::Grok,
                "{\"type\":\"text\",\"data\":\"grok reply\"}\n{\"type\":\"end\",\"sessionId\":\"g1\"}",
                "g1",
                "grok reply",
            ),
            (
                HeadlessDriver::OpenCode,
                "{\"type\":\"text\",\"sessionID\":\"o1\",\"part\":{\"text\":\"open reply\"}}",
                "o1",
                "open reply",
            ),
            (HeadlessDriver::MathCode, "formal proof", "", "formal proof"),
        ];
        for (driver, stdout, session_id, response) in cases {
            let parsed = parse_output(driver, stdout);
            assert_eq!(parsed.session_id.as_deref().unwrap_or(""), session_id);
            assert_eq!(parsed.response_text, response);
        }
    }

    #[test]
    fn pi_uses_only_successful_assistant_message_end_events() {
        let stdout = concat!(
            "{\"type\":\"session\",\"id\":\"pi-real-session\"}\n",
            "{\"type\":\"message_end\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"[choruz-incoming] user prompt\"}]}}\n",
            "{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"thinking\":\"internal\"},{\"type\":\"text\",\"text\":\"PI_IMPORT_RESUME_PI-MEM-0831-GREEN-ORBIT\"}],\"stopReason\":\"stop\"}}"
        );

        let parsed = parse_output(HeadlessDriver::Pi, stdout);

        assert_eq!(
            parsed.response_text,
            "PI_IMPORT_RESUME_PI-MEM-0831-GREEN-ORBIT"
        );
        assert_eq!(parsed.session_id.as_deref(), Some("pi-real-session"));
        assert!(!parsed.structured_error);
    }

    #[test]
    fn pi_marks_structured_error_even_when_process_exit_would_be_successful() {
        let stdout = concat!(
            "{\"type\":\"session\",\"id\":\"pi-error-session\"}\n",
            "{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"content\":[],\"stopReason\":\"error\",\"errorMessage\":\"provider detail must stay private\"}}"
        );

        let parsed = parse_output(HeadlessDriver::Pi, stdout);

        assert!(parsed.response_text.is_empty());
        assert!(parsed.structured_error);
    }

    #[test]
    fn pi_marks_captured_aborted_message_with_partial_content_as_failure() {
        let stdout = concat!(
            "{\"type\":\"session\",\"id\":\"pi-aborted-session\"}\n",
            "{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"partial content must not escape\"}],\"stopReason\":\"aborted\",\"errorMessage\":\"Request aborted\"}}"
        );

        let parsed = parse_output(HeadlessDriver::Pi, stdout);

        assert_eq!(parsed.session_id.as_deref(), Some("pi-aborted-session"));
        assert!(parsed.response_text.is_empty());
        assert!(parsed.structured_error);
    }
}
