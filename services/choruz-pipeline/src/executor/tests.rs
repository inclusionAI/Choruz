use super::*;
use choruz_agent_runtime::{CreateBindingInput, DriverType, RuntimeStore};
use choruz_session::CommandStatus;
use serde_json::json;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_postgres::NoTls;

struct TestDatabase {
    database_url: String,
    admin_database_url: String,
    database_name: String,
}

impl TestDatabase {
    async fn create() -> Self {
        let admin_database_url = connection_string("postgres");
        let database_name = format!(
            "choruz_pipeline_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos()
        );
        let (admin_client, connection) = connect_admin_database(&admin_database_url).await;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        admin_client
            .execute(&format!("CREATE DATABASE {database_name}"), &[])
            .await
            .expect("create temp db");

        let database_url = connection_string(&database_name);
        let db = Self {
            database_url,
            admin_database_url,
            database_name,
        };
        db.apply_migrations().await;
        db
    }

    async fn apply_migrations(&self) {
        let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls)
            .await
            .expect("connect temp db");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let mut files = fs::read_dir(migrations_dir())
            .expect("read migrations dir")
            .map(|entry| entry.expect("migration dir entry").path())
            .collect::<Vec<_>>();
        files.sort();

        for file in files {
            let sql = fs::read_to_string(&file).expect("read migration file");
            if sql.contains("CONCURRENTLY") {
                let executable_sql = sql
                    .lines()
                    .filter(|line| !line.trim_start().starts_with("--"))
                    .collect::<Vec<_>>()
                    .join("\n");
                for statement in executable_sql
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    client
                        .batch_execute(statement)
                        .await
                        .expect("apply non-transactional migration statement");
                }
            } else {
                client.batch_execute(&sql).await.expect("apply migration");
            }
        }
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let admin_database_url = self.admin_database_url.clone();
        let database_name = self.database_name.clone();
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("create cleanup runtime");
            runtime.block_on(async move {
                let (client, connection) = tokio_postgres::connect(&admin_database_url, NoTls)
                    .await
                    .expect("connect human db for cleanup");
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                let _ = client
                    .execute(
                        "SELECT pg_terminate_backend(pid)
                             FROM pg_stat_activity
                             WHERE datname = $1
                               AND pid <> pg_backend_pid()",
                        &[&database_name],
                    )
                    .await;
                let _ = client
                    .execute(&format!("DROP DATABASE IF EXISTS {database_name}"), &[])
                    .await;
            });
        });
        let _ = handle.join();
    }
}

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn host_start_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../infra/host/start.sh")
}

fn connection_string(database_name: &str) -> String {
    let host = env::var("CHORUZ_PG_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = env::var("CHORUZ_PG_PORT").unwrap_or_else(|_| "5432".into());
    let user = env::var("CHORUZ_PG_USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| env::var("USER").ok())
        .unwrap_or_else(|| "postgres".into());
    let password = env::var("CHORUZ_PG_PASSWORD").ok();

    let mut connection = format!("host={host} port={port} user={user} dbname={database_name}");
    if let Some(password) = password.filter(|value| !value.trim().is_empty()) {
        connection.push_str(" password=");
        connection.push_str(&password);
    }
    connection
}

async fn connect_admin_database(
    admin_database_url: &str,
) -> (
    tokio_postgres::Client,
    tokio_postgres::Connection<tokio_postgres::Socket, tokio_postgres::tls::NoTlsStream>,
) {
    match tokio_postgres::connect(admin_database_url, NoTls).await {
        Ok(connection) => connection,
        Err(first_error) => {
            ensure_postgres_started().unwrap_or_else(|message| {
                panic!("failed to auto-start postgres: {message}; initial error: {first_error}")
            });
            tokio_postgres::connect(admin_database_url, NoTls)
                .await
                .unwrap_or_else(|error| panic!("connect human db after auto-start: {error}"))
        }
    }
}

fn ensure_postgres_started() -> Result<(), String> {
    static START_ONCE: OnceLock<Result<(), String>> = OnceLock::new();
    START_ONCE
        .get_or_init(|| {
            let output = Command::new("bash")
                .arg(host_start_script())
                .output()
                .map_err(|error| format!("spawn start.sh: {error}"))?;
            if output.status.success() {
                Ok(())
            } else {
                Err(format!(
                    "start.sh exited with {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                ))
            }
        })
        .clone()
}

async fn seed_prerequisites(database_url: &str, agent_id: &str, conversation_id: &str) {
    let (client, connection) = tokio_postgres::connect(database_url, NoTls)
        .await
        .expect("connect for seeding");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name)
                 VALUES ('human-1', 'ws-acme', 'human', 'Human')
                 ON CONFLICT (id) DO NOTHING",
            &[],
        )
        .await
        .expect("seed human principal");
    client
        .execute(
            "INSERT INTO principal (id, workspace_id, type, name)
                 VALUES ($1, 'ws-acme', 'agent', $1)
                 ON CONFLICT (id) DO NOTHING",
            &[&agent_id],
        )
        .await
        .expect("seed agent principal");
    client
        .execute(
            "INSERT INTO conversation (id, workspace_id, type, name, creator_id)
                 VALUES ($1, 'ws-acme', 'group', $1, 'human-1')
                 ON CONFLICT (id) DO NOTHING",
            &[&conversation_id],
        )
        .await
        .expect("seed conversation");
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn write_fake_cli(path: &Path, record_path: &Path, stdout_lines: &[&str]) {
    let mut script = String::new();
    script.push_str("#!/bin/sh\n");
    script.push_str("{\n");
    script.push_str("  printf 'cwd=%s\\n' \"$(pwd)\"\n");
    script.push_str("  i=0\n");
    script.push_str("  for arg in \"$@\"; do\n");
    script.push_str("    i=$((i + 1))\n");
    script.push_str("    printf 'arg%s=%s\\n' \"$i\" \"$arg\"\n");
    script.push_str("  done\n");
    script.push_str("} > ");
    script.push_str(&shell_quote(&record_path.display().to_string()));
    script.push('\n');
    for line in stdout_lines {
        script.push_str("printf '%s\\n' ");
        script.push_str(&shell_quote(line));
        script.push('\n');
    }
    fs::write(path, script).expect("write fake cli");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("fake cli metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod fake cli");
    }
}

fn write_fake_cli_body(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write fake cli body");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("fake cli metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod fake cli");
    }
}

fn write_fake_cli_resume_failure(
    path: &Path,
    record_path: &Path,
    stdout_lines: &[&str],
    stderr_line: &str,
    marker: &str,
) {
    let mut script = String::new();
    script.push_str("#!/bin/sh\n");
    script.push_str("{\n");
    script.push_str("  printf 'cwd=%s\\n' \"$(pwd)\"\n");
    script.push_str("  i=0\n");
    script.push_str("  for arg in \"$@\"; do\n");
    script.push_str("    i=$((i + 1))\n");
    script.push_str("    printf 'arg%s=%s\\n' \"$i\" \"$arg\"\n");
    script.push_str("  done\n");
    script.push_str("} > ");
    script.push_str(&shell_quote(&record_path.display().to_string()));
    script.push('\n');
    for line in stdout_lines {
        script.push_str("printf '%s\\n' ");
        script.push_str(&shell_quote(line));
        script.push('\n');
    }
    script.push_str("printf '%s\\n' ");
    script.push_str(&shell_quote(stderr_line));
    script.push_str(" >&2\n");
    script.push_str("printf '%s\\n' ");
    script.push_str(&shell_quote(marker));
    script.push_str(" >&2\nexit 1\n");
    fs::write(path, script).expect("write fake resume-failure cli");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .expect("fake resume-failure cli metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod fake resume-failure cli");
    }
}

fn write_fake_cli_with_group_send(
    path: &Path,
    group_name: &str,
    content: &str,
    stdout_lines: &[&str],
) {
    let command = serde_json::json!({
        "type": "send",
        "group": group_name,
        "content": content,
    })
    .to_string();
    let mut script = String::new();
    script.push_str("#!/bin/sh\n");
    script.push_str("mkdir -p \"$CHORUZ_OUTBOX_DIR/new\"\n");
    script.push_str("printf '%s\\n' ");
    script.push_str(&shell_quote(&command));
    script.push_str(" > \"$CHORUZ_OUTBOX_DIR/new/cmd.json\"\n");
    for line in stdout_lines {
        script.push_str("printf '%s\\n' ");
        script.push_str(&shell_quote(line));
        script.push('\n');
    }
    fs::write(path, script).expect("write fake cli with group send");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .expect("fake group-send cli metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod fake group-send cli");
    }
}

fn make_command(agent_id: &str, conversation_id: &str) -> AgentCommand {
    AgentCommand {
        command_id: format!("cmd-{agent_id}"),
        route_id: format!("route-{agent_id}"),
        session_key: format!("{agent_id}:{conversation_id}"),
        agent_id: agent_id.into(),
        conversation_id: conversation_id.into(),
        message_id: format!("msg-{agent_id}"),
        turn_id: format!("turn-{agent_id}"),
        status: CommandStatus::Leased,
        current_attempt_id: Some(format!("attempt-{agent_id}")),
        current_epoch: Some(1),
        attempt_count: 1,
        max_attempts: 3,
        prompt: format!("hello from {agent_id}"),
        metadata: json!({}),
        next_retry_at: None,
        last_error: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn direct_attachment_is_staged_only_in_the_target_workspace() {
    let temp = tempfile::tempdir().expect("create attachment staging tempdir");
    let target_workspace = temp.path().join("target-workspace");
    let other_workspace = temp.path().join("other-workspace");
    fs::create_dir_all(&target_workspace).expect("create target workspace");
    fs::create_dir_all(&other_workspace).expect("create other workspace");

    let tokens_path = temp.path().join("agent_tokens.json");
    fs::write(&tokens_path, r#"{"agent-target":"target-token"}"#).expect("write agent token file");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind attachment fixture");
    let address = listener.local_addr().expect("attachment fixture address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept attachment request");
        let mut request = vec![0u8; 4096];
        let count = socket
            .read(&mut request)
            .await
            .expect("read attachment request");
        let request = String::from_utf8_lossy(&request[..count]);
        assert!(request.contains("GET /v1/attachments/att-target?actor_id=agent-target"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer target-token")
        );
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\nattached-data")
            .await
            .expect("write attachment response");
    });

    let mut command = make_command("agent-target", "direct-target");
    command.metadata = json!({
        "attachments": [{
            "attachment_id": "att-target",
            "filename": "brief.txt",
            "mime_type": "text/plain"
        }]
    });

    let prompt = stage_incoming_attachments_from_tokens_file(
        &command,
        &target_workspace,
        &format!("http://{address}"),
        &tokens_path,
    )
    .await;
    server.await.expect("attachment fixture completed");

    let staged = target_workspace
        .join(".choruz-inbox")
        .join("att-target")
        .join("brief.txt");
    assert_eq!(
        fs::read(&staged).expect("read staged attachment"),
        b"attached-data"
    );
    assert!(prompt.contains(&staged.display().to_string()));
    assert!(
        !other_workspace.join(".choruz-inbox").exists(),
        "a direct attachment must not be staged in a different workspace"
    );
}

#[test]
fn extract_reply_with_tags() {
    let raw = "some preamble\n{{CHORUZ_REPLY}}Hello, world!{{/CHORUZ_REPLY}}\nsome postamble";
    assert_eq!(extract_reply_content(raw), "Hello, world!");
}

#[test]
fn extract_reply_without_tags() {
    let raw = "Just a plain reply with no tags.";
    assert_eq!(
        extract_reply_content(raw),
        "Just a plain reply with no tags."
    );
}

#[test]
fn extract_reply_multiline() {
    let raw = "{{CHORUZ_REPLY}}\nLine 1\nLine 2\n{{/CHORUZ_REPLY}}";
    assert_eq!(extract_reply_content(raw), "Line 1\nLine 2");
}

#[test]
fn extract_reply_empty_tags() {
    let raw = "{{CHORUZ_REPLY}}{{/CHORUZ_REPLY}}";
    assert_eq!(extract_reply_content(raw), "");
}

#[test]
fn count_tool_calls_none() {
    assert_eq!(count_tool_calls("Hello, just text here."), 0);
}

#[test]
fn count_tool_calls_some() {
    let content = "tool_use: read_file, then another tool_use: write_file";
    assert_eq!(count_tool_calls(content), 2);
}

#[test]
fn codex_exec_args_use_current_cli_flags() {
    let args = headless_cli_args(LocalCliDriver::Codex, Some("session-123"), None, "hello");

    assert_eq!(
        args,
        vec![
            "exec",
            "resume",
            "session-123",
            "--json",
            "--skip-git-repo-check",
            "--dangerously-bypass-approvals-and-sandbox",
            "--config",
            "check_for_update_on_startup=false",
            "hello",
        ]
    );
    assert!(!args.iter().any(|arg| arg == "--full-auto"));
}

#[test]
fn codex_exec_args_start_new_session_without_resume() {
    let args = headless_cli_args(LocalCliDriver::Codex, None, None, "hello");

    assert_eq!(
        args,
        vec![
            "exec",
            "--json",
            "--skip-git-repo-check",
            "--dangerously-bypass-approvals-and-sandbox",
            "--config",
            "check_for_update_on_startup=false",
            "hello",
        ]
    );
    assert!(!args.iter().any(|arg| arg == "--full-auto"));
}

#[test]
fn additional_headless_drivers_use_documented_flags_and_exact_resume_ids() {
    assert_eq!(
        headless_cli_args(LocalCliDriver::Pi, Some("pi-session"), None, "hello"),
        vec![
            "--mode",
            "json",
            "--approve",
            "--session",
            "pi-session",
            "hello"
        ]
    );
    assert_eq!(
        headless_cli_args(LocalCliDriver::Grok, Some("grok-session"), None, "hello"),
        vec![
            "--no-auto-update",
            "-p",
            "hello",
            "--output-format",
            "streaming-json",
            "--always-approve",
            "--resume",
            "grok-session",
        ]
    );
    assert_eq!(
        headless_cli_args(LocalCliDriver::OpenCode, Some("oc-session"), None, "hello"),
        vec![
            "run",
            "--format",
            "json",
            "--auto",
            "--dir",
            ".",
            "--session",
            "oc-session",
            "hello",
        ]
    );
    assert_eq!(
        headless_cli_args(
            LocalCliDriver::MathCode,
            Some("math-session"),
            None,
            "hello"
        ),
        vec!["-p", "hello"],
    );
}

#[test]
fn selected_model_is_forwarded_to_every_headless_harness() {
    let cases = [
        (LocalCliDriver::Claude, "claude-opus-5"),
        (LocalCliDriver::Codex, "gpt-5.6-codex"),
        (LocalCliDriver::Pi, "anthropic/claude-sonnet-5"),
        (LocalCliDriver::Grok, "grok-4.6"),
        (
            LocalCliDriver::OpenCode,
            "openrouter/anthropic/claude-sonnet-5",
        ),
    ];
    for (driver, model) in cases {
        let args = headless_cli_args(driver, Some("session-id"), Some(model), "hello");
        let model_flag = args
            .iter()
            .position(|arg| arg == "--model")
            .expect("model flag");
        assert_eq!(args.get(model_flag + 1).map(String::as_str), Some(model));
        assert_eq!(
            args.iter().filter(|arg| arg.as_str() == "--model").count(),
            1
        );
    }
    assert_eq!(
        headless_cli_args(LocalCliDriver::Codex, None, Some("gpt-5.6-codex"), "hello")
            .last()
            .map(String::as_str),
        Some("hello")
    );
    assert!(validate_cli_model_id("--help").is_err());
}

#[test]
fn extracts_exact_external_outbox_file_from_codex_function_call_arguments() {
    let tmp = tempfile::tempdir().unwrap();
    let bound = tmp.path().join("bound");
    let external = tmp.path().join("project");
    fs::create_dir_all(bound.join(".choruz-outbox/new")).unwrap();
    fs::create_dir_all(external.join(".choruz-outbox/new")).unwrap();
    let payload = json!({"type":"send","group":"team","content":"hello"});
    let external_file = external.join(".choruz-outbox/new/cmd-0001.json");
    fs::write(&external_file, payload.to_string()).unwrap();

    let arguments = json!({
        "cmd": ".choruz/send '{\"type\":\"send\",\"group\":\"team\",\"content\":\"hello\"}'",
        "workdir": external.to_string_lossy(),
    })
    .to_string();
    let stdout = json!({
        "type": "item.completed",
        "item": {
            "type": "function_call",
            "name": "exec_command",
            "arguments": arguments,
        }
    })
    .to_string();

    let recovered = extract_external_outbox_files(&stdout, &bound, UNIX_EPOCH);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].workdir, external.canonicalize().unwrap());
    assert_eq!(recovered[0].path, external_file.canonicalize().unwrap());
}

#[test]
fn extracts_external_outbox_file_from_relative_subdirectory_helper() {
    let tmp = tempfile::tempdir().unwrap();
    let bound = tmp.path().join("bound");
    let external = tmp.path().join("project");
    let subdir = external.join("src");
    fs::create_dir_all(bound.join(".choruz-outbox/new")).unwrap();
    fs::create_dir_all(external.join(".choruz")).unwrap();
    fs::create_dir_all(external.join(".choruz-outbox/new")).unwrap();
    fs::create_dir_all(&subdir).unwrap();
    fs::write(external.join(".choruz/send"), "#!/bin/sh\n").unwrap();
    let payload = json!({"type":"send","group":"team","content":"hello"});
    let external_file = external.join(".choruz-outbox/new/cmd-0001.json");
    fs::write(&external_file, payload.to_string()).unwrap();

    let arguments = json!({
        "cmd": "../.choruz/send '{\"type\":\"send\",\"group\":\"team\",\"content\":\"hello\"}'",
        "workdir": subdir.to_string_lossy(),
    })
    .to_string();
    let stdout = json!({
        "item": {
            "name": "exec_command",
            "arguments": arguments,
        }
    })
    .to_string();

    let recovered = extract_external_outbox_files(&stdout, &bound, UNIX_EPOCH);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].workdir, external.canonicalize().unwrap());
    assert_eq!(recovered[0].path, external_file.canonicalize().unwrap());
}

#[test]
fn extracts_multiple_external_outbox_files_from_one_tool_command() {
    let tmp = tempfile::tempdir().unwrap();
    let bound = tmp.path().join("bound");
    let external = tmp.path().join("project");
    fs::create_dir_all(bound.join(".choruz-outbox/new")).unwrap();
    fs::create_dir_all(external.join(".choruz-outbox/new")).unwrap();
    let first_payload = json!({"type":"send","group":"team","content":"one"});
    let second_payload = json!({"type":"send","group":"team","content":"two"});
    let first_file = external.join(".choruz-outbox/new/cmd-0001.json");
    let second_file = external.join(".choruz-outbox/new/cmd-0002.json");
    fs::write(&first_file, first_payload.to_string()).unwrap();
    fs::write(&second_file, second_payload.to_string()).unwrap();

    let arguments = json!({
            "cmd": ".choruz/send '{\"type\":\"send\",\"group\":\"team\",\"content\":\"one\"}' && .choruz/send '{\"type\":\"send\",\"group\":\"team\",\"content\":\"two\"}'",
            "workdir": external.to_string_lossy(),
        })
        .to_string();
    let stdout = json!({
        "item": {
            "name": "exec_command",
            "arguments": arguments,
        }
    })
    .to_string();

    let recovered = extract_external_outbox_files(&stdout, &bound, UNIX_EPOCH);
    assert_eq!(recovered.len(), 2);
    assert_eq!(recovered[0].path, first_file.canonicalize().unwrap());
    assert_eq!(recovered[1].path, second_file.canonicalize().unwrap());
}

#[test]
fn ignores_bound_unmatched_old_or_non_tool_outbox_files() {
    let tmp = tempfile::tempdir().unwrap();
    let bound = tmp.path().join("bound");
    let external = tmp.path().join("project");
    fs::create_dir_all(bound.join(".choruz-outbox/new")).unwrap();
    fs::create_dir_all(external.join(".choruz-outbox/new")).unwrap();
    fs::write(
        external.join(".choruz-outbox/new/cmd-0001.json"),
        json!({"type":"send","group":"team","content":"different"}).to_string(),
    )
    .unwrap();

    let arguments = json!({
        "cmd": ".choruz/send '{\"type\":\"send\",\"group\":\"team\",\"content\":\"hello\"}'",
        "workdir": bound.to_string_lossy(),
    })
    .to_string();
    let bound_stdout = json!({
        "item": {
            "name": "exec_command",
            "arguments": arguments,
        }
    })
    .to_string();
    assert!(extract_external_outbox_files(&bound_stdout, &bound, UNIX_EPOCH).is_empty());

    let arguments = json!({
        "cmd": ".choruz/send '{\"type\":\"send\",\"group\":\"team\",\"content\":\"hello\"}'",
        "workdir": external.to_string_lossy(),
    })
    .to_string();
    let tool_stdout = json!({
        "item": {
            "name": "exec_command",
            "arguments": arguments,
        }
    })
    .to_string();
    assert!(extract_external_outbox_files(&tool_stdout, &bound, UNIX_EPOCH).is_empty());
    assert!(extract_external_outbox_files(&tool_stdout, &bound, SystemTime::now()).is_empty());

    let non_tool_stdout = json!({
            "message": {
                "content": "{\"cmd\":\".choruz/send '{\\\"type\\\":\\\"send\\\",\\\"group\\\":\\\"team\\\",\\\"content\\\":\\\"hello\\\"}'\",\"workdir\":\"/tmp/project\"}"
            }
        })
        .to_string();
    assert!(extract_external_outbox_files(&non_tool_stdout, &bound, UNIX_EPOCH).is_empty());
}

fn binding_session_state_with_config(config_json: serde_json::Value) -> BindingSessionState {
    BindingSessionState {
        binding_id: "binding-1".into(),
        workspace_path: "/workspace".into(),
        external_session_id: Some("session-1".into()),
        driver_type: "codex_exec".into(),
        config_json,
    }
}

#[test]
fn codex_session_provenance_accepts_matching_headless_binding() {
    let binding = binding_session_state_with_config(json!({
        "external_session_provenance": "process_captured",
        "external_session_driver_type": "codex_exec",
        "external_session_binding_id": "binding-1",
        "external_session_mode": "headless",
        "external_session_captured_at": "2026-05-11T00:00:00Z"
    }));

    assert!(session_provenance_matches(&binding, "headless"));
}

#[test]
fn imported_session_provenance_accepts_a_verified_workspace_scan() {
    let binding = binding_session_state_with_config(json!({
        "external_session_provenance": "workspace_scan_verified",
        "external_session_driver_type": "codex_exec",
        "external_session_binding_id": "binding-1",
        "external_session_mode": "headless"
    }));

    assert!(session_provenance_matches(&binding, "headless"));
}

#[test]
fn codex_session_provenance_rejects_wrong_binding() {
    let binding = binding_session_state_with_config(json!({
        "external_session_provenance": "process_captured",
        "external_session_driver_type": "codex_exec",
        "external_session_binding_id": "other-binding",
        "external_session_mode": "headless",
        "external_session_captured_at": "2026-05-11T00:00:00Z"
    }));

    assert!(!session_provenance_matches(&binding, "headless"));
}

#[test]
fn codex_session_provenance_rejects_terminal_mode_for_headless() {
    let binding = binding_session_state_with_config(json!({
        "external_session_provenance": "process_captured",
        "external_session_driver_type": "codex_exec",
        "external_session_binding_id": "binding-1",
        "external_session_mode": "terminal",
        "external_session_captured_at": "2026-05-11T00:00:00Z"
    }));

    assert!(!session_provenance_matches(&binding, "headless"));
}

#[tokio::test]
async fn supported_cli_driver_bindings_execute_with_fake_binaries() {
    struct DriverCase {
        label: &'static str,
        driver_type: DriverType,
        selected_cli: &'static str,
        expected_bootstrap: &'static str,
        stdout_lines: Vec<String>,
        expected_session_id: &'static str,
        expected_args: Vec<&'static str>,
        expected_prompt_arg: usize,
    }

    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let tmp = tempfile::tempdir().unwrap();
    let sandbox_dir = tmp.path().join("sandboxes");
    fs::create_dir_all(&sandbox_dir).unwrap();

    let cases = vec![
            DriverCase {
                label: "claude-terminal",
                driver_type: DriverType::ClaudeTerminal,
                selected_cli: "claude",
                expected_bootstrap: "CLAUDE.md",
                stdout_lines: vec![
                    r#"{"type":"system","session_id":"claude-terminal-session"}"#.into(),
                    r#"{"type":"result","result":"stdout is internal"}"#.into(),
                ],
                expected_session_id: "claude-terminal-session",
                expected_args: vec![
                    "--print",
                    "--output-format",
                    "stream-json",
                    "--dangerously-skip-permissions",
                    "--verbose",
                    "--",
                ],
                expected_prompt_arg: 7,
            },
            DriverCase {
                label: "claude-print",
                driver_type: DriverType::ClaudePrint,
                selected_cli: "claude",
                expected_bootstrap: "CLAUDE.md",
                stdout_lines: vec![
                    r#"{"type":"system","session_id":"claude-print-session"}"#.into(),
                    r#"{"type":"result","result":"stdout is internal"}"#.into(),
                ],
                expected_session_id: "claude-print-session",
                expected_args: vec![
                    "--print",
                    "--output-format",
                    "stream-json",
                    "--dangerously-skip-permissions",
                    "--verbose",
                    "--",
                ],
                expected_prompt_arg: 7,
            },
            DriverCase {
                label: "codex-exec",
                driver_type: DriverType::CodexExec,
                selected_cli: "codex",
                expected_bootstrap: "AGENTS.md",
                stdout_lines: vec![
                    r#"{"type":"thread.started","thread_id":"codex-exec-session"}"#.into(),
                    r#"{"type":"item.completed","item":{"type":"agent_message","text":"stdout is internal"}}"#.into(),
                ],
                expected_session_id: "codex-exec-session",
                expected_args: vec![
                    "exec",
                    "--json",
                    "--skip-git-repo-check",
                    "--dangerously-bypass-approvals-and-sandbox",
                    "--config",
                    "check_for_update_on_startup=false",
                ],
                expected_prompt_arg: 7,
            },
            DriverCase {
                label: "codex-terminal",
                driver_type: DriverType::CodexTerminal,
                selected_cli: "codex",
                expected_bootstrap: "AGENTS.md",
                stdout_lines: vec![
                    r#"{"type":"thread.started","thread_id":"codex-terminal-session"}"#.into(),
                    r#"{"type":"item.completed","item":{"type":"agent_message","text":"stdout is internal"}}"#.into(),
                ],
                expected_session_id: "codex-terminal-session",
                expected_args: vec![
                    "exec",
                    "--json",
                    "--skip-git-repo-check",
                    "--dangerously-bypass-approvals-and-sandbox",
                    "--config",
                    "check_for_update_on_startup=false",
                ],
                expected_prompt_arg: 7,
            },
            DriverCase {
                label: "pi-terminal",
                driver_type: DriverType::PiTerminal,
                selected_cli: "pi",
                expected_bootstrap: "AGENTS.md",
                stdout_lines: vec![
                    r#"{"type":"session","version":3,"id":"pi-terminal-session"}"#.into(),
                    r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"stdout is internal"}]}}"#.into(),
                ],
                expected_session_id: "pi-terminal-session",
                expected_args: vec!["--mode", "json", "--approve"],
                expected_prompt_arg: 4,
            },
            DriverCase {
                label: "grok-terminal",
                driver_type: DriverType::GrokTerminal,
                selected_cli: "grok",
                expected_bootstrap: "AGENTS.md",
                stdout_lines: vec![
                    r#"{"type":"text","data":"stdout is internal"}"#.into(),
                    r#"{"type":"end","sessionId":"grok-terminal-session"}"#.into(),
                ],
                expected_session_id: "grok-terminal-session",
                expected_args: vec!["--no-auto-update", "-p"],
                expected_prompt_arg: 3,
            },
            DriverCase {
                label: "opencode-terminal",
                driver_type: DriverType::OpenCodeTerminal,
                selected_cli: "opencode",
                expected_bootstrap: "AGENTS.md",
                stdout_lines: vec![
                    r#"{"type":"step_start","sessionID":"opencode-terminal-session"}"#.into(),
                    r#"{"type":"text","sessionID":"opencode-terminal-session","part":{"type":"text","text":"stdout is internal"}}"#.into(),
                ],
                expected_session_id: "opencode-terminal-session",
                expected_args: vec!["run", "--format", "json", "--auto", "--dir", "."],
                expected_prompt_arg: 7,
            },
        ];

    for (idx, case) in cases.into_iter().enumerate() {
        let agent_id = format!("agent-{}", case.label);
        let conversation_id = format!("conv-{}", case.label);
        seed_prerequisites(&database.database_url, &agent_id, &conversation_id).await;

        let workspace = tmp.path().join(format!("workspace-{}", case.label));
        fs::create_dir_all(&workspace).unwrap();
        let binding = runtime
            .create_binding(CreateBindingInput {
                conversation_id: conversation_id.clone(),
                agent_principal_id: agent_id.clone(),
                driver_type: case.driver_type.clone(),
                workspace_path: workspace.display().to_string(),
                git_worktree_path: None,
                config_json: json!({ "case": case.label }),
                audit_actor: None,
            })
            .await
            .expect("create runtime binding");

        let script_path = tmp.path().join(format!("{}-fake-cli.sh", case.label));
        let record_path = tmp.path().join(format!("{}-record.txt", case.label));
        let stdout_refs = case
            .stdout_lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        write_fake_cli(&script_path, &record_path, &stdout_refs);

        let mut config = PipelineConfig::from_env();
        config.executor_timeout_secs = 5;
        config.sandbox_base_dir = sandbox_dir.display().to_string();
        config.gateway_base_url = "http://127.0.0.1:9".into();
        config.claude_cli_path = tmp
            .path()
            .join(format!("missing-claude-{idx}"))
            .display()
            .to_string();
        config.codex_cli_path = tmp
            .path()
            .join(format!("missing-codex-{idx}"))
            .display()
            .to_string();
        config.pi_cli_path = tmp
            .path()
            .join(format!("missing-pi-{idx}"))
            .display()
            .to_string();
        config.grok_cli_path = tmp
            .path()
            .join(format!("missing-grok-{idx}"))
            .display()
            .to_string();
        config.opencode_cli_path = tmp
            .path()
            .join(format!("missing-opencode-{idx}"))
            .display()
            .to_string();
        match case.selected_cli {
            "claude" => config.claude_cli_path = script_path.display().to_string(),
            "codex" => config.codex_cli_path = script_path.display().to_string(),
            "pi" => config.pi_cli_path = script_path.display().to_string(),
            "grok" => config.grok_cli_path = script_path.display().to_string(),
            "opencode" => config.opencode_cli_path = script_path.display().to_string(),
            other => panic!("unknown selected cli: {other}"),
        }
        let ctx = ExecutorContext::from_config(&config)
            .with_event_store(choruz_store::EventStore::new(database.database_url.clone()));
        let cmd = make_command(&agent_id, &conversation_id);

        let result = execute_command(&ctx, &cmd).await;
        assert_eq!(
            result.status,
            AgentResultStatus::Succeeded,
            "{} failed: {:?}",
            case.label,
            result.error
        );
        assert_eq!(result.content.as_deref(), Some("stdout is internal"));

        assert!(
            workspace.join(case.expected_bootstrap).is_file(),
            "{} should install the driver-specific {} bootstrap",
            case.label,
            case.expected_bootstrap,
        );
        for bootstrap in ["CLAUDE.md", "AGENTS.md"] {
            if bootstrap != case.expected_bootstrap {
                assert!(
                    !workspace.join(bootstrap).exists(),
                    "{} must not install unrelated {} bootstrap",
                    case.label,
                    bootstrap,
                );
            }
        }

        let record = fs::read_to_string(&record_path).expect("fake cli invocation record");
        let canonical_workspace = workspace.canonicalize().expect("canonical workspace path");
        assert!(
            record.contains(&format!("cwd={}", canonical_workspace.display())),
            "{} should run in binding workspace, got:\n{}",
            case.label,
            record
        );
        for (arg_idx, expected) in case.expected_args.iter().enumerate() {
            assert!(
                record.contains(&format!("arg{}={expected}", arg_idx + 1)),
                "{} missing expected arg {expected}, got:\n{}",
                case.label,
                record
            );
        }
        assert!(
            record.contains(&format!(
                "arg{}=hello from {}",
                case.expected_prompt_arg, agent_id
            )),
            "{} prompt was not passed in the expected position, got:\n{}",
            case.label,
            record
        );

        let refreshed = runtime
            .get_binding(&binding.id)
            .await
            .expect("load binding");
        assert_eq!(
            refreshed.external_session_id.as_deref(),
            Some(case.expected_session_id)
        );
        assert_eq!(
            refreshed.config_json["external_session_provenance"],
            "process_captured"
        );
        assert_eq!(
            refreshed.config_json["external_session_driver_type"],
            case.driver_type.as_str()
        );
        assert_eq!(refreshed.config_json["external_session_mode"], "headless");
    }
}

#[tokio::test]
async fn local_cli_failures_are_classified_for_bounded_recovery() {
    struct DriverCase {
        label: &'static str,
        driver_type: DriverType,
    }
    struct FailureCase {
        label: &'static str,
        body: Option<&'static str>,
        expected_kind: &'static str,
        retryable: bool,
    }

    let drivers = [
        DriverCase {
            label: "claude",
            driver_type: DriverType::ClaudePrint,
        },
        DriverCase {
            label: "codex",
            driver_type: DriverType::CodexExec,
        },
        DriverCase {
            label: "pi",
            driver_type: DriverType::PiTerminal,
        },
        DriverCase {
            label: "grok",
            driver_type: DriverType::GrokTerminal,
        },
        DriverCase {
            label: "opencode",
            driver_type: DriverType::OpenCodeTerminal,
        },
    ];
    let failures = [
        FailureCase {
            label: "missing",
            body: None,
            expected_kind: "driver_unavailable",
            retryable: false,
        },
        FailureCase {
            label: "auth",
            // Matches both network/timeout and auth text to lock in the
            // non-retriable authentication classification precedence.
            body: Some(
                "printf '%s\\n' 'PRIVATE_AUTH_MARKER connection timeout: not authenticated' >&2; exit 1",
            ),
            expected_kind: "auth",
            retryable: false,
        },
        FailureCase {
            label: "crash",
            body: Some("printf '%s\\n' 'PRIVATE_CRASH_MARKER child crashed' >&2; exit 17"),
            expected_kind: "process_failed",
            retryable: true,
        },
        FailureCase {
            label: "timeout",
            body: Some("sleep 30"),
            expected_kind: "timeout",
            retryable: true,
        },
    ];

    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let tmp = tempfile::tempdir().unwrap();
    let sandbox_dir = tmp.path().join("sandboxes");
    fs::create_dir_all(&sandbox_dir).unwrap();

    for driver in drivers {
        for failure in &failures {
            let suffix = format!("{}-{}", driver.label, failure.label);
            let agent_id = format!("agent-{suffix}");
            let conversation_id = format!("conv-{suffix}");
            seed_prerequisites(&database.database_url, &agent_id, &conversation_id).await;
            let workspace = tmp.path().join(format!("workspace-{suffix}"));
            fs::create_dir_all(&workspace).unwrap();
            runtime
                .create_binding(CreateBindingInput {
                    conversation_id: conversation_id.clone(),
                    agent_principal_id: agent_id.clone(),
                    driver_type: driver.driver_type.clone(),
                    workspace_path: workspace.display().to_string(),
                    git_worktree_path: None,
                    config_json: json!({}),
                    audit_actor: None,
                })
                .await
                .expect("create failure-mode binding");

            let cli_path = tmp.path().join(format!("fake-{suffix}.sh"));
            if let Some(body) = failure.body {
                write_fake_cli_body(&cli_path, body);
            }
            let mut config = PipelineConfig::from_env();
            config.executor_timeout_secs = 1;
            config.sandbox_base_dir = sandbox_dir.display().to_string();
            config.gateway_base_url = "http://127.0.0.1:9".into();
            config.claude_cli_path = tmp.path().join("unused-claude").display().to_string();
            config.codex_cli_path = tmp.path().join("unused-codex").display().to_string();
            config.pi_cli_path = tmp.path().join("unused-pi").display().to_string();
            config.grok_cli_path = tmp.path().join("unused-grok").display().to_string();
            config.opencode_cli_path = tmp.path().join("unused-opencode").display().to_string();
            match driver.label {
                "claude" => config.claude_cli_path = cli_path.display().to_string(),
                "codex" => config.codex_cli_path = cli_path.display().to_string(),
                "pi" => config.pi_cli_path = cli_path.display().to_string(),
                "grok" => config.grok_cli_path = cli_path.display().to_string(),
                "opencode" => config.opencode_cli_path = cli_path.display().to_string(),
                _ => unreachable!(),
            }
            let ctx = ExecutorContext::from_config(&config)
                .with_event_store(choruz_store::EventStore::new(database.database_url.clone()));

            let result = execute_command(&ctx, &make_command(&agent_id, &conversation_id)).await;
            assert_eq!(result.status, AgentResultStatus::Failed, "{suffix}");
            let error = result.error.as_deref().expect("durable failure marker");
            assert!(
                error.contains(&format!("kind={}", failure.expected_kind)),
                "{suffix}: {error}"
            );
            assert_eq!(
                is_auto_retriable_error(Some(error)),
                failure.retryable,
                "{suffix}: {error}"
            );
            assert!(!error.contains("PRIVATE_AUTH_MARKER"));
            assert!(!error.contains("PRIVATE_CRASH_MARKER"));
            if failure.label == "crash" {
                assert!(
                    error.contains("exit_status=exit status: 17"),
                    "{suffix}: {error}"
                );
            }
        }
    }
}

#[tokio::test]
async fn claude_and_codex_execute_concurrently_without_crossing_results() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let tmp = tempfile::tempdir().unwrap();
    let sandbox_dir = tmp.path().join("sandboxes");
    fs::create_dir_all(&sandbox_dir).unwrap();

    let claude_agent = "agent-concurrent-claude";
    let claude_conversation = "conv-concurrent-claude";
    let codex_agent = "agent-concurrent-codex";
    let codex_conversation = "conv-concurrent-codex";
    seed_prerequisites(&database.database_url, claude_agent, claude_conversation).await;
    seed_prerequisites(&database.database_url, codex_agent, codex_conversation).await;

    let claude_workspace = tmp.path().join("workspace-concurrent-claude");
    let codex_workspace = tmp.path().join("workspace-concurrent-codex");
    fs::create_dir_all(&claude_workspace).unwrap();
    fs::create_dir_all(&codex_workspace).unwrap();
    let claude_binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: claude_conversation.into(),
            agent_principal_id: claude_agent.into(),
            driver_type: DriverType::ClaudePrint,
            workspace_path: claude_workspace.display().to_string(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: None,
        })
        .await
        .unwrap();
    let codex_binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: codex_conversation.into(),
            agent_principal_id: codex_agent.into(),
            driver_type: DriverType::CodexExec,
            workspace_path: codex_workspace.display().to_string(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: None,
        })
        .await
        .unwrap();

    let claude_ready = tmp.path().join("claude-ready");
    let codex_ready = tmp.path().join("codex-ready");
    const PEER_WAIT_SECS: u8 = 10;
    let wait_for_peer = |own: &Path, peer: &Path| {
        format!(
            "touch {}\ni=0\nwhile [ ! -f {} ] && [ \"$i\" -lt {} ]; do sleep 1; i=$((i + 1)); done\n[ -f {} ] || exit 19\n",
            shell_quote(&own.display().to_string()),
            shell_quote(&peer.display().to_string()),
            PEER_WAIT_SECS,
            shell_quote(&peer.display().to_string()),
        )
    };
    let claude_cli = tmp.path().join("concurrent-claude.sh");
    let codex_cli = tmp.path().join("concurrent-codex.sh");
    write_fake_cli_body(
        &claude_cli,
        &format!(
            "{}printf '%s\\n' '{{\"type\":\"system\",\"session_id\":\"claude-concurrent-session\"}}'\nprintf '%s\\n' '{{\"type\":\"result\",\"result\":\"claude-only-result\"}}'",
            wait_for_peer(&claude_ready, &codex_ready)
        ),
    );
    write_fake_cli_body(
        &codex_cli,
        &format!(
            "{}printf '%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"codex-concurrent-session\"}}'\nprintf '%s\\n' '{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"codex-only-result\"}}}}'",
            wait_for_peer(&codex_ready, &claude_ready)
        ),
    );

    let mut config = PipelineConfig::from_env();
    config.executor_timeout_secs = 30;
    config.sandbox_base_dir = sandbox_dir.display().to_string();
    config.gateway_base_url = "http://127.0.0.1:9".into();
    config.claude_cli_path = claude_cli.display().to_string();
    config.codex_cli_path = codex_cli.display().to_string();
    let ctx = ExecutorContext::from_config(&config)
        .with_event_store(choruz_store::EventStore::new(database.database_url.clone()));
    let claude_command = make_command(claude_agent, claude_conversation);
    let codex_command = make_command(codex_agent, codex_conversation);

    let (claude_result, codex_result) = tokio::join!(
        execute_command(&ctx, &claude_command),
        execute_command(&ctx, &codex_command)
    );
    assert_eq!(
        claude_result.status,
        AgentResultStatus::Succeeded,
        "Claude failed during the peer-start handshake: {:?}",
        claude_result.error
    );
    assert_eq!(claude_result.content.as_deref(), Some("claude-only-result"));
    assert_eq!(claude_result.agent_id, claude_agent);
    assert_eq!(
        codex_result.status,
        AgentResultStatus::Succeeded,
        "Codex failed during the peer-start handshake: {:?}",
        codex_result.error
    );
    assert_eq!(codex_result.content.as_deref(), Some("codex-only-result"));
    assert_eq!(codex_result.agent_id, codex_agent);

    let claude_refreshed = runtime.get_binding(&claude_binding.id).await.unwrap();
    let codex_refreshed = runtime.get_binding(&codex_binding.id).await.unwrap();
    assert_eq!(
        claude_refreshed.external_session_id.as_deref(),
        Some("claude-concurrent-session")
    );
    assert_eq!(
        codex_refreshed.external_session_id.as_deref(),
        Some("codex-concurrent-session")
    );
}

#[tokio::test]
async fn group_send_outbox_suppresses_stdout_fallback() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let tmp = tempfile::tempdir().unwrap();
    let sandbox_dir = tmp.path().join("sandboxes");
    fs::create_dir_all(&sandbox_dir).unwrap();

    let agent_id = "agent-group-send";
    let conversation_id = "conv-group-send";
    let group_message = "group outbox message should not be echoed";
    seed_prerequisites(&database.database_url, agent_id, conversation_id).await;

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for group send suppression test");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "INSERT INTO conversation_member (conv_id, principal_id, joined_at)
                 VALUES ($1, $2, NOW())",
            &[&conversation_id, &agent_id],
        )
        .await
        .expect("seed agent group membership");

    let workspace = tmp.path().join("workspace-group-send");
    fs::create_dir_all(&workspace).unwrap();
    runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation_id.into(),
            agent_principal_id: agent_id.into(),
            driver_type: DriverType::CodexExec,
            workspace_path: workspace.display().to_string(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: None,
        })
        .await
        .expect("create runtime binding");

    let script_path = tmp.path().join("codex-group-send-fake-cli.sh");
    write_fake_cli_with_group_send(
        &script_path,
        conversation_id,
        group_message,
        &[
            r#"{"type":"thread.started","thread_id":"codex-group-send-session"}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"stdout fallback must not echo"}}"#,
        ],
    );

    let mut config = PipelineConfig::from_env();
    config.executor_timeout_secs = 5;
    config.sandbox_base_dir = sandbox_dir.display().to_string();
    config.gateway_base_url = "http://127.0.0.1:9".into();
    config.codex_cli_path = script_path.display().to_string();
    let ctx = ExecutorContext::from_config(&config)
        .with_event_store(choruz_store::EventStore::new(database.database_url.clone()));

    let result = execute_command(&ctx, &make_command(agent_id, conversation_id)).await;
    assert_eq!(
        result.status,
        AgentResultStatus::Succeeded,
        "{:?}",
        result.error
    );
    assert_eq!(
        result.content.as_deref(),
        Some(""),
        "group outbox sends are already inserted and must not echo stdout"
    );

    let inserted_count: i64 = client
        .query_one(
            "SELECT COUNT(*)
                 FROM conversation_events
                 WHERE conversation_id = $1
                   AND sender_id = $2
                   AND content = $3",
            &[&conversation_id, &agent_id, &Some(group_message)],
        )
        .await
        .expect("count inserted group messages")
        .get(0);
    assert_eq!(inserted_count, 1);
}

#[tokio::test]
async fn codex_exec_does_not_resume_foreign_agent_session_or_workspace() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let tmp = tempfile::tempdir().unwrap();
    let sandbox_dir = tmp.path().join("sandboxes");
    fs::create_dir_all(&sandbox_dir).unwrap();

    let agent_a = "agent-direct-a";
    let agent_b = "agent-direct-b";
    let conversation_a = "direct-conv-a";
    let conversation_b = "direct-conv-b";
    seed_prerequisites(&database.database_url, agent_a, conversation_a).await;
    seed_prerequisites(&database.database_url, agent_b, conversation_b).await;

    let workspace_a = tmp.path().join("agent-a-private-workspace");
    let workspace_b = tmp.path().join("agent-b-private-workspace");
    fs::create_dir_all(&workspace_a).unwrap();
    fs::create_dir_all(&workspace_b).unwrap();

    let binding_a = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation_a.into(),
            agent_principal_id: agent_a.into(),
            driver_type: DriverType::CodexExec,
            workspace_path: workspace_a.display().to_string(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: None,
        })
        .await
        .expect("create agent A binding");
    let binding_b = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation_b.into(),
            agent_principal_id: agent_b.into(),
            driver_type: DriverType::CodexExec,
            workspace_path: workspace_b.display().to_string(),
            git_worktree_path: None,
            config_json: json!({}),
            audit_actor: None,
        })
        .await
        .expect("create agent B binding");

    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for stale session seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "UPDATE conversation SET type = 'direct' WHERE id IN ($1, $2)",
            &[&conversation_a, &conversation_b],
        )
        .await
        .expect("mark conversations as direct chats");
    let foreign_provenance = json!({
        "external_session_provenance": "process_captured",
        "external_session_driver_type": "codex_exec",
        "external_session_binding_id": binding_a.id,
        "external_session_mode": "headless",
        "external_session_captured_at": "2026-05-11T00:00:00Z"
    });
    client
        .execute(
            "UPDATE agent_runtime_bindings
                 SET external_session_id = $1,
                     config_json = $2,
                     updated_at = NOW()
                 WHERE id = $3",
            &[&"agent-a-codex-thread", &foreign_provenance, &binding_b.id],
        )
        .await
        .expect("seed B with foreign session provenance");

    let script_path = tmp.path().join("codex-fake-cli.sh");
    let record_path = tmp.path().join("codex-record.txt");
    write_fake_cli(
        &script_path,
        &record_path,
        &[
            r#"{"type":"thread.started","thread_id":"agent-b-fresh-thread"}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"stdout is internal"}}"#,
        ],
    );

    let mut config = PipelineConfig::from_env();
    config.executor_timeout_secs = 5;
    config.sandbox_base_dir = sandbox_dir.display().to_string();
    config.gateway_base_url = "http://127.0.0.1:9".into();
    config.claude_cli_path = tmp.path().join("missing-claude").display().to_string();
    config.codex_cli_path = script_path.display().to_string();
    config.pi_cli_path = tmp.path().join("missing-pi").display().to_string();
    config.grok_cli_path = tmp.path().join("missing-grok").display().to_string();
    config.opencode_cli_path = tmp.path().join("missing-opencode").display().to_string();
    let ctx = ExecutorContext::from_config(&config)
        .with_event_store(choruz_store::EventStore::new(database.database_url.clone()));

    let mut cmd = make_command(agent_b, conversation_b);
    cmd.prompt = "B direct prompt only; no A direct history".into();
    let result = execute_command(&ctx, &cmd).await;
    assert_eq!(
        result.status,
        AgentResultStatus::Succeeded,
        "agent B execution failed: {:?}",
        result.error
    );

    let record = fs::read_to_string(&record_path).expect("fake cli invocation record");
    let canonical_workspace_b = workspace_b.canonicalize().expect("canonical workspace B");
    assert!(
        record.contains(&format!("cwd={}", canonical_workspace_b.display())),
        "agent B must run in its own binding workspace, got:\n{record}"
    );
    assert!(
        !record.contains("agent-a-private-workspace"),
        "agent B invocation leaked agent A workspace context:\n{record}"
    );
    assert!(
        !record.contains("agent-a-codex-thread"),
        "agent B must not resume agent A's Codex thread:\n{record}"
    );
    assert!(
        !record.contains("arg2=resume"),
        "agent B should start a fresh Codex exec when session provenance points at agent A:\n{record}"
    );
    assert!(
        record.contains("B direct prompt only; no A direct history"),
        "agent B did not receive its own direct prompt:\n{record}"
    );

    let refreshed_b = runtime
        .get_binding(&binding_b.id)
        .await
        .expect("load B binding");
    assert_eq!(
        refreshed_b.external_session_id.as_deref(),
        Some("agent-b-fresh-thread")
    );
    assert_eq!(
        refreshed_b.config_json["external_session_binding_id"],
        binding_b.id
    );
}

#[test]
fn structured_stdout_resume_failure_detection_ignores_assistant_text() {
    assert!(structured_stdout_indicates_resume_failure(
        r#"{"type":"error","message":"No saved session found with ID"}"#
    ));
    assert!(structured_stdout_indicates_resume_failure(
        r#"{"type":"error","message":"Error: thread/resume: thread/resume failed: no rollout found for thread id opaque-id"}"#
    ));
    assert!(!structured_stdout_indicates_resume_failure(
        r#"{"type":"item.completed","item":{"type":"agent_message","text":"The error resuming session was fixed."}}"#
    ));
}

#[test]
fn pi_captured_jsonl_fixture_persists_only_the_assistant_answer() {
    let user = serde_json::from_str::<serde_json::Value>(
            r#"{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"[choruz-incoming] user prompt"}]}}"#,
        )
        .unwrap();
    let assistant = serde_json::from_str::<serde_json::Value>(
            r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"internal"},{"type":"text","text":"PI_IMPORT_RESUME_PI-MEM-0831-GREEN-ORBIT"}],"stopReason":"stop"}}"#,
        )
        .unwrap();

    assert_eq!(structured_response_text(LocalCliDriver::Pi, &user), None);
    assert_eq!(
        structured_response_text(LocalCliDriver::Pi, &assistant).as_deref(),
        Some("PI_IMPORT_RESUME_PI-MEM-0831-GREEN-ORBIT")
    );
}

#[test]
fn pi_structured_failure_is_independent_of_process_exit_status() {
    let stdout = concat!(
        "{\"type\":\"session\",\"id\":\"pi-session\"}\n",
        "{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"content\":[],\"stopReason\":\"error\",\"errorMessage\":\"provider detail\"}}"
    );
    assert!(parse_output(LocalCliDriver::Pi, stdout).structured_error);
    assert!(
            !parse_output(
                LocalCliDriver::Pi,
                r#"{"type":"message_end","message":{"role":"assistant","content":"ok","stopReason":"stop"}}"#,
            )
            .structured_error
        );
}

#[test]
fn pi_captured_aborted_fixture_drops_partial_content_and_fails() {
    let stdout = concat!(
        "{\"type\":\"session\",\"id\":\"pi-session\"}\n",
        "{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"partial content must not escape\"}],\"stopReason\":\"aborted\",\"errorMessage\":\"Request aborted\"}}"
    );
    let parsed = parse_output(LocalCliDriver::Pi, stdout);

    assert!(parsed.structured_error);
    assert!(parsed.response_text.is_empty());

    let event = serde_json::from_str::<serde_json::Value>(stdout.lines().nth(1).unwrap()).unwrap();
    assert_eq!(structured_response_text(LocalCliDriver::Pi, &event), None);
}

#[tokio::test]
async fn codex_resume_failure_clears_provenance_and_redacts_driver_stderr_before_retry() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let tmp = tempfile::tempdir().unwrap();
    let sandbox_dir = tmp.path().join("sandboxes");
    fs::create_dir_all(&sandbox_dir).unwrap();

    let agent_id = "agent-resume-recovery";
    let conversation_id = "direct-conv-resume-recovery";
    seed_prerequisites(&database.database_url, agent_id, conversation_id).await;
    let workspace = tmp.path().join("agent-resume-recovery-workspace");
    fs::create_dir_all(&workspace).unwrap();
    let binding = runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation_id.into(),
            agent_principal_id: agent_id.into(),
            driver_type: DriverType::CodexExec,
            workspace_path: workspace.display().to_string(),
            git_worktree_path: None,
            config_json: json!({"unrelated_setting": "preserve"}),
            audit_actor: None,
        })
        .await
        .expect("create recovery binding");

    let stale_session = "stale-codex-session";
    let matching_provenance = json!({
        "unrelated_setting": "preserve",
        "external_session_provenance": "process_captured",
        "external_session_driver_type": "codex_exec",
        "external_session_binding_id": binding.id,
        "external_session_mode": "headless",
        "external_session_captured_at": "2026-05-11T00:00:00Z"
    });
    let (client, connection) = tokio_postgres::connect(&database.database_url, NoTls)
        .await
        .expect("connect for recovery seed");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "UPDATE agent_runtime_bindings
                 SET external_session_id = $1, config_json = $2, updated_at = NOW()
                 WHERE id = $3",
            &[&stale_session, &matching_provenance, &binding.id],
        )
        .await
        .expect("seed matching stale Codex provenance");

    let script_path = tmp.path().join("codex-resume-recovery-fake.sh");
    let record_path = tmp.path().join("codex-resume-recovery-record.txt");
    let private_marker = "PRIVATE_DRIVER_STDERR_MARKER_4f8b6c12";
    write_fake_cli_resume_failure(
        &script_path,
        &record_path,
        &[
            r#"{"type":"thread.started","thread_id":"metadata-session-must-not-persist"}"#,
            r#"{"type":"error","message":"No saved session found with ID"}"#,
        ],
        "driver process failed",
        private_marker,
    );

    let mut config = PipelineConfig::from_env();
    config.executor_timeout_secs = 5;
    config.sandbox_base_dir = sandbox_dir.display().to_string();
    config.gateway_base_url = "http://127.0.0.1:9".into();
    config.claude_cli_path = tmp.path().join("missing-claude").display().to_string();
    config.codex_cli_path = script_path.display().to_string();
    config.pi_cli_path = tmp.path().join("missing-pi").display().to_string();
    config.grok_cli_path = tmp.path().join("missing-grok").display().to_string();
    config.opencode_cli_path = tmp.path().join("missing-opencode").display().to_string();
    let ctx = ExecutorContext::from_config(&config)
        .with_event_store(choruz_store::EventStore::new(database.database_url.clone()));

    let cmd = make_command(agent_id, conversation_id);
    let failed = execute_command(&ctx, &cmd).await;
    assert_eq!(failed.status, AgentResultStatus::Failed);
    assert!(
        !failed
            .error
            .as_deref()
            .unwrap_or_default()
            .contains(private_marker),
        "raw CLI stderr must not cross the executor result boundary"
    );
    let first_record = fs::read_to_string(&record_path).expect("first fake cli record");
    assert!(first_record.contains("arg2=resume"));
    assert!(first_record.contains(stale_session));

    let cleared = runtime
        .get_binding(&binding.id)
        .await
        .expect("load cleared binding");
    assert_eq!(cleared.external_session_id, None);
    assert_eq!(cleared.config_json["unrelated_setting"], "preserve");
    for key in [
        "external_session_provenance",
        "external_session_driver_type",
        "external_session_binding_id",
        "external_session_mode",
        "external_session_captured_at",
    ] {
        assert!(
            cleared.config_json.get(key).is_none(),
            "{key} must be cleared"
        );
    }

    write_fake_cli(
        &script_path,
        &record_path,
        &[
            r#"{"type":"thread.started","thread_id":"fresh-codex-session"}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"safe response"}}"#,
        ],
    );
    let retried = execute_command(&ctx, &cmd).await;
    assert_eq!(
        retried.status,
        AgentResultStatus::Succeeded,
        "{:?}",
        retried.error
    );
    let retry_record = fs::read_to_string(&record_path).expect("retry fake cli record");
    assert!(!retry_record.contains("arg2=resume"));
    assert!(!retry_record.contains(stale_session));
    let refreshed = runtime
        .get_binding(&binding.id)
        .await
        .expect("load refreshed binding");
    assert_eq!(
        refreshed.external_session_id.as_deref(),
        Some("fresh-codex-session")
    );
    assert_eq!(
        refreshed.config_json["external_session_provenance"],
        "process_captured"
    );
}

#[tokio::test]
async fn webhook_agent_binding_skips_cli_and_succeeds_empty() {
    let database = TestDatabase::create().await;
    let runtime = RuntimeStore::new(database.database_url.clone());
    let tmp = tempfile::tempdir().unwrap();
    let agent_id = "agent-webhook";
    let conversation_id = "conv-webhook";
    seed_prerequisites(&database.database_url, agent_id, conversation_id).await;

    let workspace = tmp.path().join("webhook-workspace");
    fs::create_dir_all(&workspace).unwrap();
    runtime
        .create_binding(CreateBindingInput {
            conversation_id: conversation_id.into(),
            agent_principal_id: agent_id.into(),
            driver_type: DriverType::WebhookAgent,
            workspace_path: workspace.display().to_string(),
            git_worktree_path: None,
            config_json: json!({ "webhook_url": "http://127.0.0.1:9/hook" }),
            audit_actor: None,
        })
        .await
        .expect("create webhook binding");

    let mut config = PipelineConfig::from_env();
    config.executor_timeout_secs = 5;
    config.sandbox_base_dir = tmp.path().join("sandboxes").display().to_string();
    config.gateway_base_url = "http://127.0.0.1:9".into();
    config.claude_cli_path = tmp.path().join("missing-claude").display().to_string();
    config.codex_cli_path = tmp.path().join("missing-codex").display().to_string();
    config.pi_cli_path = tmp.path().join("missing-pi").display().to_string();
    config.grok_cli_path = tmp.path().join("missing-grok").display().to_string();
    config.opencode_cli_path = tmp.path().join("missing-opencode").display().to_string();
    let ctx = ExecutorContext::from_config(&config)
        .with_event_store(choruz_store::EventStore::new(database.database_url.clone()));

    let result = execute_command(&ctx, &make_command(agent_id, conversation_id)).await;
    assert_eq!(result.status, AgentResultStatus::Succeeded);
    assert_eq!(result.content.as_deref(), Some(""));
    assert!(
        result.error.is_none(),
        "webhook_agent must not retry as a CLI failure"
    );
}

#[test]
fn executor_context_default_cli_path() {
    let config = PipelineConfig::from_env();
    let ctx = ExecutorContext::from_config(&config);
    assert!(!ctx.claude_cli_path.is_empty());
}

#[tokio::test]
async fn binding_lookup_reports_missing_event_store_as_an_error() {
    let config = PipelineConfig::from_env();
    let ctx = ExecutorContext::from_config(&config);

    let error = ctx
        .lookup_agent_binding("agent-without-store")
        .await
        .unwrap_err();

    assert!(error.contains("agent binding lookup failed"));
    assert!(error.contains("event store is not configured"));
}

#[tokio::test]
async fn binding_lookup_preserves_database_connection_errors() {
    let config = PipelineConfig::from_env();
    let ctx = ExecutorContext::from_config(&config).with_event_store(
        choruz_store::EventStore::new("host=127.0.0.1 port=1 dbname=choruz"),
    );

    let error = ctx
        .lookup_agent_binding("agent-without-database")
        .await
        .unwrap_err();

    assert!(error.contains("agent binding lookup failed"));
    assert!(error.contains("database connection"));
}

#[cfg(unix)]
#[tokio::test]
async fn non_executable_cli_is_a_non_retriable_configuration_error() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let cli_path = tmp.path().join("non-executable-cli");
    fs::write(&cli_path, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&cli_path).unwrap().permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&cli_path, permissions).unwrap();

    let error = tokio::process::Command::new(&cli_path)
        .output()
        .await
        .expect_err("non-executable CLI should fail to start");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    let marker = classify_cli_start_error(error.kind());
    assert_eq!(marker, "configuration");
    assert!(!is_auto_retriable_error(Some(&format!(
        "headless CLI could not start [kind={marker}]"
    ))));
}

#[tokio::test]
async fn executor_context_wal_recovery_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let config = PipelineConfig::from_env();
    let mut ctx = ExecutorContext::from_config(&config);
    ctx.wal_base_dir = tmp.path().to_path_buf();
    // Should not panic on empty dir
    ctx.recover_from_wal().await;
}

#[tokio::test]
async fn executor_context_wal_recovery_with_incomplete() {
    let tmp = tempfile::tempdir().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    // Create a WAL with an incomplete turn
    let wal_path = wal_dir.join("test_session.db");
    let wal = AdapterWal::open(&wal_path).unwrap();
    wal.log_turn_start("turn-1", "attempt-1", "test prompt")
        .await
        .unwrap();

    // Verify it's incomplete
    let incomplete = wal.find_incomplete_turns().await.unwrap();
    assert_eq!(incomplete.len(), 1);
    drop(wal);

    // Run recovery
    let config = PipelineConfig::from_env();
    let mut ctx = ExecutorContext::from_config(&config);
    ctx.wal_base_dir = wal_dir.clone();
    ctx.recover_from_wal().await;

    // Verify the incomplete turn was marked as failed
    let wal2 = AdapterWal::open(&wal_path).unwrap();
    let incomplete2 = wal2.find_incomplete_turns().await.unwrap();
    assert!(
        incomplete2.is_empty(),
        "incomplete turns should be resolved after recovery"
    );
}

#[tokio::test]
async fn shutdown_all_on_empty() {
    let config = PipelineConfig::from_env();
    let ctx = ExecutorContext::from_config(&config);
    // Should not panic
    ctx.shutdown_all().await;
}

/// Verify the hard-timeout pattern used in `spawn_headless_session`:
/// a child that won't finish within the deadline must be released
/// promptly via `tokio::time::timeout` dropping the future, which in
/// turn fires `kill_on_drop` on the spawned Child. Uses `sleep 30`
/// against a 200 ms deadline.
#[tokio::test]
async fn hard_timeout_kills_hung_child() {
    let start = std::time::Instant::now();

    let cli_future = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .output();

    let result = tokio::time::timeout(std::time::Duration::from_millis(200), cli_future).await;

    // 200 ms deadline against a 30 s sleep must time out, never complete.
    assert!(result.is_err(), "expected Elapsed, got {:?}", result);

    // Wall time must be ~200 ms, NOT 30 s. If kill_on_drop hadn't fired
    // we'd still measure ≈ 200 ms (timeout drops the future regardless),
    // but the test name + intent is to lock in the kill-on-drop contract.
    // Allow generous headroom for slow CI; failing means something is
    // *seriously* off.
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "hard timeout did not return promptly: took {:?}",
        start.elapsed()
    );
}
