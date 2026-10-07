#![cfg(unix)]

use std::{
    fs,
    io::{BufRead, BufReader},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_some() {
            return;
        }
        // Failed assertions must not leave a fixture behind when deliberately
        // testing a host that lacks its shutdown handler.
        if std::thread::panicking()
            && let Ok(children) = Command::new("pgrep")
                .args(["-P", &self.0.id().to_string()])
                .output()
        {
            for child in String::from_utf8_lossy(&children.stdout).lines() {
                let _ = Command::new("kill").args(["-TERM", child]).status();
            }
        }
        let _ = Command::new("kill")
            .args(["-TERM", &self.0.id().to_string()])
            .status();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while matches!(self.0.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[test]
fn api_only_host_uses_external_database_and_stops_its_gateway() {
    exercise_host(false);
}

#[test]
fn signal_registration_failure_reaps_children_without_advertising_readiness() {
    exercise_host(true);
}

fn exercise_host(fail_registration: bool) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("migrations")).unwrap();
    let server = root.path().join("choruz-server");
    fs::copy(env!("CARGO_BIN_EXE_choruz-server"), &server).unwrap();
    let gateway = root.path().join("choruz-api-gateway");
    // Only the gateway is replaced: the compiled host must select its database,
    // discover the bundle, start a child, wait for readiness and reap on SIGTERM.
    fs::write(
        &gateway,
        r#"#!/usr/bin/env python3
import http.server, os
assert os.environ['CHORUZ_DATABASE_URL'] == 'host=fixture dbname=owned'
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = b'{"status":"ready","service":"choruz-api-gateway","protocol_version":1}'
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
http.server.HTTPServer(('127.0.0.1', int(os.environ['CHORUZ_API_PORT'])), Handler).serve_forever()
"#,
    )
    .unwrap();
    fs::set_permissions(&gateway, fs::Permissions::from_mode(0o755)).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let mut command = Command::new(server);
    if fail_registration {
        // Interpose only SIGINT registration in this child. This exercises the
        // real OS error path without a production-only fault-injection flag.
        let source = root.path().join("signal.c");
        let library = root.path().join("signal.so");
        fs::write(&source, include_str!("signal_failure.c")).unwrap();
        let mut compiler = Command::new("cc");
        if cfg!(target_os = "macos") {
            compiler.arg("-dynamiclib");
            command.env("DYLD_INSERT_LIBRARIES", &library);
        } else {
            compiler.args(["-shared", "-fPIC"]);
            command.env("LD_PRELOAD", &library);
        }
        assert!(
            compiler
                .arg(source)
                .arg("-o")
                .arg(library)
                .status()
                .unwrap()
                .success()
        );
    }
    let mut host = Host(
        command
            .arg("--api-only")
            .env("HOME", root.path())
            .env("XDG_DATA_HOME", root.path())
            .env("CHORUZ_DATABASE_URL", "host=fixture dbname=owned")
            .env("CHORUZ_API_PORT", port.to_string())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = host.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.unwrap();
            if line.starts_with("CHORUZ_LISTENING=") {
                let _ = tx.send(line);
                break;
            }
        }
    });
    if fail_registration {
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(30)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        assert_eq!(host.0.wait().unwrap().code(), Some(1));
    } else {
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(30)).unwrap(),
            format!("CHORUZ_LISTENING={port}")
        );
    }
    drop(host);
    reader.join().unwrap();
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    assert!(!root.path().join("choruz/pgdata").exists());
}
