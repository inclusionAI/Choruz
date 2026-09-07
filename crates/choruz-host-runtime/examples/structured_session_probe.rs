//! Opt-in live Harness probe in a disposable workspace; no login data is printed.
use choruz_host_runtime::{
    TerminalSpec, new_terminal_pool,
    session::{self, SessionCommand},
};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let driver = std::env::args()
        .nth(1)
        .ok_or("pass claude_terminal or codex_terminal")?;
    let directory = tempfile::tempdir()?;
    choruz_host_runtime::ensure_outbox_helper(directory.path())?;
    let id = choruz_common::new_id();
    let pool = new_terminal_pool();
    let spec = TerminalSpec {
        terminal_id: id.clone(),
        driver_type: driver.clone(),
        binary_path: None,
        workspace_path: directory.path().to_string_lossy().into(),
        cols: 80,
        rows: 24,
        resume_session_id: None,
        codex_home: None,
        model: Some(
            if driver == "claude_terminal" {
                "haiku"
            } else {
                "gpt-5.4-mini"
            }
            .into(),
        ),
        harness_account: serde_json::json!({}),
    };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        session::ensure(&pool, spec.clone())?;
        wait_ready(&id)?;
        session::command(
            &id,
            SessionCommand::Send {
                text: "Reply with exactly: Structured conversation connected. Do not use tools."
                    .into(),
                submission_id: choruz_common::new_id(),
            },
        )?;
        wait_ready(&id)?;
        let snapshot = session::snapshot(&id)?;
        if snapshot
            .items
            .iter()
            .filter(|item| {
                item.kind == "assistant" && item.text.contains("Structured conversation connected")
            })
            .count()
            != 1
        {
            eprintln!("Probe projection: {}", serde_json::to_string(&snapshot)?);
            return Err("expected exactly one assistant reply".into());
        }
        println!(
            "PASS {driver}: initialized, completed real turn, projected assistant text; items={}",
            snapshot.items.len()
        );
        let native_id = snapshot.session_id;
        session::command(&id, SessionCommand::Send {
            text: "Use the command execution tool to run exactly pwd (no other commands). Then reply with the current directory. Do not modify files.".into(),
            submission_id: choruz_common::new_id(),
        })?;
        wait_ready(&id)?;
        let tools = session::snapshot(&id)?;
        if !tools.items.iter().any(|item| {
            item.kind == "tool"
                && item.status == "completed"
                && item
                    .detail
                    .to_string()
                    .contains(directory.path().to_string_lossy().as_ref())
        }) {
            return Err("read-only tool output was not projected".into());
        }
        session::command(&id, SessionCommand::Close)?;
        session::ensure(&pool, spec.clone())?;
        wait_ready(&id)?;
        let restored = session::snapshot(&id)?;
        if restored.session_id != native_id
            || restored
                .items
                .iter()
                .filter(|item| {
                    item.kind == "assistant"
                        && item.text.contains("Structured conversation connected")
                })
                .count()
                != 1
            || !restored.items.iter().any(|item| {
                item.kind == "tool"
                    && item
                        .detail
                        .to_string()
                        .contains(directory.path().to_string_lossy().as_ref())
            })
        {
            return Err("reopen did not preserve the exact session and prior answer".into());
        }
        println!(
            "PASS {driver}: one assistant reply, read-only tool output, and exact native session/history recovery without duplicate messages"
        );
        if driver == "claude_terminal" {
            let marker = directory.path().join("permission-probe.txt");
            session::command(
                &id,
                SessionCommand::Send {
                    text: format!(
                        "Use the Write tool to create exactly {} containing exactly structured-permission-ok. Do not use other tools or modify any other files.",
                        marker.display()
                    ),
                    submission_id: choruz_common::new_id(),
                },
            )?;
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                let pending = session::snapshot(&id)?;
                if let Some(request) = pending.requests.first() {
                    let input = &request["request"]["input"];
                    if request["request"]["tool_name"] != "Write"
                        || input["file_path"] != marker.to_string_lossy().as_ref()
                        || input["content"] != "structured-permission-ok"
                    {
                        return Err("unexpected write permission payload".into());
                    }
                    if marker.exists() {
                        return Err("write occurred before approval".into());
                    }
                    session::command(
                        &id,
                        SessionCommand::Respond {
                            request_id: request["request_id"].clone(),
                            allow: true,
                            answers: Default::default(),
                        },
                    )?;
                    break;
                }
                if pending.status == "ready"
                    || pending.status == "failed"
                    || Instant::now() > deadline
                {
                    return Err("Claude did not request write approval".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            wait_ready(&id)?;
            if std::fs::read_to_string(&marker)? != "structured-permission-ok" {
                return Err("approved write result missing".into());
            }
            println!(
                "PASS {driver}: real permission request blocked a workspace write until explicit approval"
            );
            use choruz_agent_runtime::headless::{
                CLAUDE_PARENT_SESSION_ENV, HeadlessDriver, parse_output,
            };
            let mut process = std::process::Command::new("claude");
            process.current_dir(directory.path()).args(
                HeadlessDriver::Claude.args_with_session_fork(
                    native_id.as_deref(),
                    Some("haiku"),
                    "Reply exactly: Routed turn isolated. Do not use tools.",
                    true,
                ),
            );
            for key in CLAUDE_PARENT_SESSION_ENV {
                process.env_remove(key);
            }
            let output = process.output()?;
            if !output.status.success() {
                return Err("forked headless turn failed".into());
            }
            let parsed = parse_output(
                HeadlessDriver::Claude,
                &String::from_utf8_lossy(&output.stdout),
            );
            if parsed.session_id.is_none() || parsed.session_id == native_id {
                return Err("headless fork reused the direct session".into());
            }
            let forked = parsed.session_id;
            session::command(&id, SessionCommand::Close)?;
            session::ensure(&pool, spec.clone())?;
            wait_ready(&id)?;
            if session::snapshot(&id)?
                .items
                .iter()
                .any(|item| item.text.contains("Routed turn isolated"))
            {
                return Err("headless turn leaked into the direct transcript".into());
            }
            session::close(&id)?;
            let mut from_headless = spec.clone();
            from_headless.terminal_id = choruz_common::new_id();
            from_headless.resume_session_id = forked;
            from_headless.harness_account = serde_json::json!({"external_session_mode":"headless"});
            let other = from_headless.terminal_id.clone();
            let fork_result = (|| -> Result<(), Box<dyn std::error::Error>> {
                session::ensure(&pool, from_headless.clone())?;
                wait_ready(&other)?;
                session::command(
                    &other,
                    SessionCommand::Send {
                        text: "Reply exactly: Direct fork connected. Do not use tools.".into(),
                        submission_id: choruz_common::new_id(),
                    },
                )?;
                wait_ready(&other)?;
                let snapshot = session::snapshot(&other)?;
                if snapshot.session_id == from_headless.resume_session_id
                    || !snapshot
                        .items
                        .iter()
                        .any(|item| item.text.contains("Direct fork connected"))
                {
                    return Err("direct fork did not isolate the headless source".into());
                }
                Ok(())
            })();
            session::close(&other)?;
            fork_result?;
            println!(
                "PASS {driver}: simultaneous direct/headless identities and reverse fork are isolated"
            );
        }
        Ok(())
    })();
    session::close(&id)?;
    result
}

fn wait_ready(id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let state = session::snapshot(id)?;
        match state.status.as_str() {
            "ready" => return Ok(()),
            "failed" => return Err(state.error.unwrap_or("Harness failed".into()).into()),
            "waiting" => {
                for request in state.requests {
                    let command = request["request"]["input"]["command"]
                        .as_str()
                        .or_else(|| request["params"]["command"].as_str());
                    if command != Some("pwd") {
                        return Err("probe refused an unexpected permission request".into());
                    }
                    session::command(
                        id,
                        SessionCommand::Respond {
                            request_id: request
                                .get("request_id")
                                .or_else(|| request.get("id"))
                                .cloned()
                                .ok_or("missing request id")?,
                            allow: true,
                            answers: Default::default(),
                        },
                    )?;
                }
            }
            _ => {}
        }
        if Instant::now() > deadline {
            return Err(format!("Harness timed out in {}", state.status).into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
