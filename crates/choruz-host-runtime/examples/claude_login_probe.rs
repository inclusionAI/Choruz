//! Explicit live smoke using the installed Claude CLI and its default profile.
//! Does not print terminal contents, account identifiers or credentials.
use choruz_agent_runtime::headless::HeadlessDriver;
use choruz_harness_login::{AccountProfile, claude_model_catalog, claude_signed_in};
use choruz_host_runtime::{TerminalSpec, ensure_terminal, new_terminal_pool};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = new_terminal_pool();
    let id = choruz_common::new_id();
    let spec = TerminalSpec {
        authentication: true,
        terminal_id: id.clone(),
        driver_type: "claude_terminal".into(),
        binary_path: None,
        workspace_path: String::new(),
        cols: 100,
        rows: 30,
        resume_session_id: None,
        codex_home: None,
        model: None,
        harness_account: serde_json::json!({"harness_account_profile_kind": "default"}),
    };
    let terminal = ensure_terminal(&pool, &spec)?;
    let (replay, mut output) = terminal.session.subscribe_with_replay();
    if replay.is_empty() {
        tokio::time::timeout(std::time::Duration::from_secs(15), output.recv()).await??;
    }
    println!(
        "Official CLI produced PTY output; authentication mode: {}",
        terminal.session.authentication
    );
    for _ in 0..100 {
        if choruz_host_runtime::terminal::close_terminal(&pool, &id).is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    if terminal.session.is_child_alive() {
        return Err("authentication CLI did not stop".into());
    }
    let profile = AccountProfile {
        driver: HeadlessDriver::Claude,
        account_id: String::new(),
        profile_kind: "default".into(),
    };
    let identity = claude_signed_in(&profile)
        .await
        .map_err(|_| "default profile is not signed in")?;
    assert!(identity.models.as_array().is_some_and(Vec::is_empty));
    let models = claude_model_catalog(&profile)
        .await
        .map_err(|_| "model catalog unavailable")?;
    println!(
        "Default profile identity verified independently; {} selectable models",
        models.as_array().map_or(0, Vec::len)
    );
    Ok(())
}
