//! Exercise the shipped CLI. Only the remote HTTP service is replaced here;
//! gateway tests own authorization, durable learning and asynchronous execution.
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Output},
};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_choruz"))
        .args(args)
        .env_remove("CHORUZ_SESSION_TOKEN")
        .env_remove("CHORUZ_OPERATOR_PASSWORD")
        .env("CHORUZ_API_BASE_URL", "http://127.0.0.1:1")
        .output()
        .unwrap()
}

fn result(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[cfg(unix)]
#[test]
fn local_start_passes_api_only_selection_and_does_not_request_pairing() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("server");
    let received = root.path().join("received");
    // The server's startup handshake is the replaced boundary; supervisor
    // tests separately exercise the selected service's real process lifetime.
    fs::write(&binary, "#!/bin/sh\nprintf '%s\\n' \"$*\" \"$CHORUZ_API_PORT\" > \"$CHORUZ_TEST_RECEIVED\"\nprintf 'CHORUZ_LISTENING=%s\\n' \"$CHORUZ_API_PORT\"\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let output = Command::new(env!("CARGO_BIN_EXE_choruz"))
        .args([
            "start",
            "local",
            "--api-url",
            &format!("http://127.0.0.1:{port}"),
        ])
        .env("HOME", root.path())
        .env("XDG_DATA_HOME", root.path())
        .env("CHORUZ_SERVER_BINARY", binary)
        .env("CHORUZ_TEST_RECEIVED", &received)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(received).unwrap(),
        format!("--api-only\n{port}\n")
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Pairing credential"));
}

#[cfg(unix)]
#[test]
fn tool_enable_finishes_installation_and_selection_survives_cli_exit() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    for name in ["bsk", "cua-driver"] {
        let path = bin.join(name);
        fs::write(&path, r#"#!/bin/sh
case "$1" in
--version) echo fixture;;
install-skill) mkdir -p "$HOME/.agents/skills/browser-skill"; echo installed > "$HOME/.agents/skills/browser-skill/SKILL.md";;
doctor) if [ "${0##*/}" = bsk ]; then echo '[{"name":"extension","ok":false}]'; else echo '{"ok":true}'; fi;;
permissions) echo '{"accessibility":false,"screen_recording":false}';;
*) exit 2;;
esac
"#).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = |args: &[&str]| {
        result(
            Command::new(env!("CARGO_BIN_EXE_choruz"))
                .args(args)
                .env("HOME", home.path())
                .output()
                .unwrap(),
        )
    };
    assert_eq!(
        run(&["tools", "disable", "browser"])["tools"][0]["enabled"],
        false
    );
    assert_eq!(run(&["tools", "status"])["tools"][0]["enabled"], false);
    let enabled = run(&["tools", "enable", "browser"]);
    assert_eq!(enabled["tools"][0]["enabled"], true);
    assert_eq!(enabled["tools"][0]["status"], "needs_attention");
    assert!(
        home.path()
            .join(".agents/skills/browser-skill/SKILL.md")
            .is_file()
    );
    assert_eq!(run(&["tools", "status"])["tools"][0]["enabled"], true);
}

#[test]
fn local_commands_read_selected_native_account_and_preserve_judge_requirement() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("work");
    fs::create_dir(&workspace).unwrap();
    for harness in ["claude", "codex"] {
        for account in ["default", "selected"] {
            let home = root.path().join(format!("{harness}-{account}"));
            let directory = home.join(if harness == "claude" {
                "projects/work"
            } else {
                "sessions"
            });
            fs::create_dir_all(&directory).unwrap();
            let records = if harness == "claude" {
                vec![
                    json!({"type":"user","cwd":workspace,"message":{"role":"user","content":account}}),
                ]
            } else {
                vec![
                    json!({"type":"session_meta","payload":{"id":"owned","cwd":workspace}}),
                    json!({"type":"response_item","payload":{"type":"message","role":"user","content":account}}),
                ]
            };
            fs::write(
                directory.join("owned.jsonl"),
                records.iter().map(|r| format!("{r}\n")).collect::<String>(),
            )
            .unwrap();
            if account == "selected" {
                let source = root.path().join("source.json");
                fs::write(&source, json!({"harness":harness,"account_home":home,"workspace_path":workspace,"session_id":"owned"}).to_string()).unwrap();
                let window = result(cli(&["library", "trace", source.to_str().unwrap()]));
                assert_eq!(window["records"].as_array().unwrap().len(), 1);
                assert!(window["records"].to_string().contains("selected"));
                assert!(!window["records"].to_string().contains("default"));
                assert_eq!(window["cursor"]["session"], "owned");
                assert!(!home.join(".credentials.json").exists());
            }
        }
    }
    let check = root.path().join("check.json");
    let answer = root.path().join("answer.txt");
    fs::write(&answer, "correct").unwrap();
    for (value, expected) in [
        (
            json!({"type":"exact","expected":"correct"}),
            json!({"score":1.0,"requires_judge":false}),
        ),
        (
            json!({"type":"exact","expected":"different"}),
            json!({"score":0.0,"requires_judge":false}),
        ),
        (
            json!({"type":"judge","expected":"correct","rubric":"Check meaning"}),
            json!({"score":null,"requires_judge":true}),
        ),
    ] {
        fs::write(&check, value.to_string()).unwrap();
        assert_eq!(
            result(cli(&[
                "library",
                "score",
                check.to_str().unwrap(),
                answer.to_str().unwrap()
            ])),
            expected
        );
    }
    fs::write(&check, "[]").unwrap();
    assert_eq!(
        result(cli(&["library", "community", check.to_str().unwrap()]))["encountered"],
        0
    );
    fs::write(&check, "[{}]").unwrap();
    assert!(
        !cli(&["library", "community", check.to_str().unwrap()])
            .status
            .success()
    );
}

#[test]
fn learning_and_generic_api_preserve_method_body_token_and_server_errors() {
    let root = tempfile::tempdir().unwrap();
    let body = root.path().join("settings.json");
    let settings = json!({"enabled":false,"analyst_binding_id":"analyst"});
    fs::write(&body, settings.to_string()).unwrap();
    for (args, expected_method, expected_path, status, response) in [
        (
            vec!["learning", "configure", "binding", body.to_str().unwrap()],
            "PUT",
            "/v1/runtime/bindings/binding/experience",
            "200 OK",
            "{\"saved\":true}",
        ),
        (
            vec!["learning", "show", "binding"],
            "GET",
            "/v1/runtime/bindings/binding/experience",
            "403 Forbidden",
            "{\"error\":\"not authorized\"}",
        ),
        (
            vec!["learning", "prepare", "binding", body.to_str().unwrap()],
            "POST",
            "/v1/runtime/bindings/binding/experience/prepare",
            "200 OK",
            "{\"saved\":true}",
        ),
        (
            vec!["api", "DELETE", "/v1/tasks/owned"],
            "DELETE",
            "/v1/tasks/owned",
            "204 No Content",
            "",
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline, "CLI did not connect");
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept CLI request: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let (header, content) = loop {
                let mut chunk = [0; 1024];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .map(|n| n.parse().unwrap())
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break (header, bytes[end + 4..end + 4 + length].to_vec());
                    }
                }
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
            (header, content)
        });
        let mut command = args;
        command.extend(["--api-url", &address, "--token", "fixture-token"]);
        let output = cli(&command);
        let (header, content) = server.join().unwrap();
        assert!(
            header.starts_with(&format!("{expected_method} {expected_path} HTTP/1.1")),
            "{header}"
        );
        assert!(
            header
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-token")
        );
        if matches!(expected_method, "PUT" | "POST") {
            assert_eq!(serde_json::from_slice::<Value>(&content).unwrap(), settings);
            assert_eq!(result(output)["saved"], true);
        } else if expected_method == "DELETE" {
            assert_eq!(result(output), Value::Null);
        } else {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("403"));
        }
    }
    for target in ["https://other.example/v1/me", "//other.example/v1/me"] {
        let rejected = cli(&["api", "GET", target]);
        assert_eq!(rejected.status.code(), Some(2));
    }
}
