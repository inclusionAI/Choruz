//! Device-owned single-turn CLI invocation. Callers retain leases and delivery.

use std::{future::Future, io, path::Path, process::Output, time::Duration};

use choruz_agent_runtime::{
    DriverType,
    executable::terminal_binary,
    headless::{
        HeadlessDriver, configure_command_workspace, prepare_harness_account_env, validate_model,
    },
    process::ProcessContainer,
};
use serde_json::Value;
use tokio::process::Command;

pub struct TurnSpec<'a> {
    pub driver_type: &'a str,
    pub binary_path: Option<&'a str>,
    pub workspace: &'a Path,
    pub account: &'a Value,
    pub resume_session_id: Option<&'a str>,
    pub model: Option<&'a str>,
    pub prompt: &'a str,
    pub fork_session: bool,
}

/// Resolve this device's executable and isolated profile; never resolve another
/// device's profile on the controller. Callers supply their own outbox transport.
pub fn command(spec: TurnSpec<'_>) -> Result<Command, String> {
    let driver = HeadlessDriver::from_driver_type(spec.driver_type)
        .ok_or_else(|| "unsupported headless Harness".to_owned())?;
    let kind = match spec
        .driver_type
        .parse::<DriverType>()
        .map_err(|error| format!("{error}"))?
    {
        DriverType::ClaudePrint => DriverType::ClaudeTerminal,
        DriverType::CodexExec | DriverType::CodexAppServer => DriverType::CodexTerminal,
        kind => kind,
    };
    if let Some(model) = spec.model {
        validate_model(model)?;
    }
    let mut command = Command::new(terminal_binary(&kind, spec.binary_path));
    configure_command_workspace(&mut command, driver, spec.workspace);
    if let Some((key, profile)) = prepare_harness_account_env(driver, spec.account)? {
        std::fs::create_dir_all(&profile)
            .map_err(|error| format!("create isolated Harness profile: {error}"))?;
        command.env(key, profile);
    }
    command
        .args(driver.args_with_session_fork(
            spec.resume_session_id,
            spec.model,
            spec.prompt,
            spec.fork_session,
        ))
        .env("CHORUZ_WORKSPACE", spec.workspace)
        .env("DISABLE_AUTOUPDATER", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("CLAUDE_CODE_ENABLE_TASKS", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    Ok(command)
}

/// Cancellation and timeout kill the entire process container and await child
/// exit before returning. The caller owns durable command/attempt fencing.
pub async fn run(
    mut command: Command,
    timeout: Duration,
    cancelled: impl Future<Output = String>,
) -> io::Result<Output> {
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn()?;
    let container = child
        .id()
        .map(|pid| ProcessContainer::new(format!("headless-{pid}"), pid));
    let output = child.wait_with_output();
    tokio::pin!(output);
    let error = tokio::select! {
        result = &mut output => return result,
        reason = cancelled => io::Error::new(io::ErrorKind::Interrupted, reason),
        _ = tokio::time::sleep(timeout) => io::Error::new(io::ErrorKind::TimedOut, "Harness execution timed out"),
    };
    if let Some(container) = &container {
        container.kill_all();
    }
    let _ = output.await?;
    Err(error)
}

/// Native resume errors may arrive in structured stdout with a successful exit.
/// Assistant text discussing an error must never clear a valid session.
pub fn resume_failed(output: &Output, resume_session_id: Option<&str>) -> bool {
    resume_session_id.is_some_and(|id| !id.is_empty())
        && (resume_failure_phrase(&String::from_utf8_lossy(&output.stderr).to_lowercase())
            || structured_resume_failure(&String::from_utf8_lossy(&output.stdout)))
}

fn resume_failure_phrase(text: &str) -> bool {
    [
        "no saved session",
        "invalid session identifier",
        "error resuming session",
        "no rollout found",
        "thread/resume failed",
        "session not found",
        "unknown session",
        "could not find session",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

fn structured_resume_failure(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        event["type"] == "error"
            && event["message"]
                .as_str()
                .is_some_and(|message| resume_failure_phrase(&message.to_lowercase()))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn native_resume_error_and_normal_assistant_text_have_distinct_outcomes() {
        for (text, failed) in [
            (
                r#"{"type":"error","message":"No saved session found with ID"}"#,
                true,
            ),
            (
                r#"{"type":"error","message":"thread/resume failed: no rollout found"}"#,
                true,
            ),
            (
                r#"{"type":"item.completed","item":{"type":"agent_message","text":"The error resuming session was fixed."}}"#,
                false,
            ),
        ] {
            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", "printf '%s' \"$1\"", "fixture", text])
                .stdout(std::process::Stdio::piped());
            let output = run(command, Duration::from_secs(5), std::future::pending())
                .await
                .unwrap();
            assert!(output.status.success());
            assert_eq!(resume_failed(&output, Some("owned-session")), failed);
            assert!(!resume_failed(&output, None));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_kills_and_reaps_the_owned_native_process() {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("native-fixture");
        std::fs::write(
            &binary,
            "#!/bin/sh\necho $$ > \"$PID_FILE\"\nwhile :; do /bin/sleep 1; done\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let pid_file = directory.path().join("pid");
        let mut command = Command::new(binary);
        command
            .env("PID_FILE", &pid_file)
            .stdout(std::process::Stdio::piped());
        command.process_group(0);
        let running = tokio::spawn(run(
            command,
            Duration::from_secs(60),
            std::future::pending(),
        ));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let pid = loop {
            if let Some(pid) = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|value| value.trim().parse::<i32>().ok())
            {
                break pid;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native fixture never became ready"
            );
            tokio::task::yield_now().await;
        };
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        tokio::time::advance(Duration::from_secs(61)).await;
        let error = running.await.unwrap().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
}
