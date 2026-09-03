//! Process supervisor — spawns & reaps the sibling Rust binaries
//! `choruz-server` depends on.
//!
//! Responsibilities:
//!   1. Resolve the binary paths (same directory as the running host
//!      binary).
//!   2. Pick a working directory (repo root) so the gateway's relative paths
//!      (e.g. `.runtime/attachments`) resolve the same way as they do under
//!      `infra/host/dev.sh`.
//!   3. Reuse an existing listener only after its versioned readiness response
//!      proves it is the expected Choruz service.
//!   4. Wait for spawned services to become ready and prove the new child is
//!      still alive before publishing success.
//!   5. Stop spawned children gracefully, then force and reap after a deadline.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_INTERVAL: Duration = Duration::from_millis(100);
const PROBE_IO_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Everything spawned by this process gets stashed here. `Drop` kills them
/// all so the host exiting cleans up.
pub struct Supervisor {
    children: Mutex<Vec<ChildProc>>,
    shutting_down: AtomicBool,
    monitor_started: AtomicBool,
    backend_failed: AtomicBool,
}

struct ChildProc {
    name: &'static str,
    child: Child,
}

impl Supervisor {
    pub fn new() -> Self {
        Self {
            children: Mutex::new(Vec::new()),
            shutting_down: AtomicBool::new(false),
            monitor_started: AtomicBool::new(false),
            backend_failed: AtomicBool::new(false),
        }
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    /// Resolve `<bin>` next to the current executable — `target/{debug,release}/`
    /// under `cargo run`, or wherever the release bundle put the binaries.
    fn sibling_binary_path(bin: &str) -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        let candidate = dir.join(bin);
        if candidate.exists() {
            return Some(candidate);
        }
        None
    }

    fn host_working_dir() -> Option<PathBuf> {
        let executable = std::env::current_exe().ok()?;
        Self::host_working_dir_from(&executable)
    }

    /// Walk up from the current exe until we find a `Cargo.toml` + `apps/`
    /// directory combination — that's the workspace root the gateway and
    /// pipeline expect as their CWD so relative paths like
    /// `.runtime/attachments` resolve correctly. Public so `main.rs` can
    /// find it before the Supervisor exists, e.g. to locate the
    /// `migrations/` folder for embedded-Postgres setup.
    pub fn workspace_root_static() -> Option<PathBuf> {
        let executable = std::env::current_exe().ok()?;
        Self::workspace_root_from(&executable)
    }

    fn workspace_root_from(executable: &Path) -> Option<PathBuf> {
        let mut cur = executable.to_path_buf();
        for _ in 0..8 {
            cur.pop();
            let cargo_toml = cur.join("Cargo.toml");
            let apps_dir = cur.join("apps");
            if cargo_toml.exists() && apps_dir.exists() {
                return Some(cur);
            }
        }
        None
    }

    /// Resolve the directory from which backend children should run. A source
    /// checkout uses its workspace root; a complete bundle uses the binary
    /// directory, which contains every child binary and `migrations/`.
    fn host_working_dir_from(executable: &Path) -> Option<PathBuf> {
        Self::workspace_root_from(executable).or_else(|| {
            let binary_dir = executable.parent()?;
            let required = ["choruz-api-gateway", "choruz-pipeline"];
            if binary_dir.join("migrations").is_dir()
                && required.iter().all(|name| binary_dir.join(name).is_file())
            {
                Some(binary_dir.to_path_buf())
            } else {
                None
            }
        })
    }

    /// Returns true if something is already listening on 127.0.0.1:<port>.
    /// Callers must still verify the service identity before reusing it.
    fn port_in_use(port: u16) -> bool {
        TcpListener::bind(("127.0.0.1", port)).is_err()
    }

    fn probe_service(port: u16, expected_service: &str) -> Result<(), String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(PROBE_IO_TIMEOUT)
            .build()
            .map_err(|error| format!("build readiness client: {error}"))?;
        let status = client
            .get(format!("http://127.0.0.1:{port}/readyz"))
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .and_then(reqwest::blocking::Response::json::<choruz_common::HostServiceStatus>)
            .map_err(|error| format!("read readiness: {error}"))?;
        if status.service != expected_service {
            return Err(format!(
                "expected service {expected_service}, found {}",
                status.service
            ));
        }
        if status.protocol_version != choruz_common::HOST_SERVICE_PROTOCOL_VERSION {
            return Err(format!(
                "unsupported host protocol {}, expected {}",
                status.protocol_version,
                choruz_common::HOST_SERVICE_PROTOCOL_VERSION
            ));
        }
        if status.status != "ready" {
            return Err(format!("service reported status {}", status.status));
        }
        Ok(())
    }

    fn child_is_running(&self, name: &str) -> Result<bool, String> {
        let mut children = self.children.lock().unwrap();
        let child = children
            .iter_mut()
            .find(|child| child.name == name)
            .ok_or_else(|| format!("spawned child {name} is not registered"))?;
        child
            .child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(|error| format!("inspect {name}: {error}"))
    }

    /// Start one lightweight monitor for children that survived startup.
    /// Unexpected exits are reported and the remaining owned children are
    /// stopped, preventing a partially alive backend stack.
    pub fn start_child_monitor(self: &std::sync::Arc<Self>) {
        if self
            .monitor_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let supervisor = std::sync::Arc::downgrade(self);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let Some(supervisor) = supervisor.upgrade() else {
                    return;
                };
                if supervisor.shutting_down.load(Ordering::SeqCst) {
                    return;
                }
                if let Err(error) = supervisor.check_children() {
                    supervisor.backend_failed.store(true, Ordering::SeqCst);
                    tracing::error!(error, "backend child exited after startup");
                    supervisor.shutdown();
                    return;
                }
            }
        });
    }

    /// Whether the post-readiness monitor observed an unexpected child exit.
    pub fn backend_failed(&self) -> bool {
        self.backend_failed.load(Ordering::SeqCst)
    }

    fn check_children(&self) -> Result<(), String> {
        let mut children = self.children.lock().unwrap();
        for child in children.iter_mut() {
            match child.child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    return Err(format!("{} exited with {status}", child.name));
                }
                Err(error) => return Err(format!("inspect {}: {error}", child.name)),
            }
        }
        Ok(())
    }

    fn wait_until_ready(&self, name: &str, port: u16, service: &str) -> Result<(), String> {
        let deadline = Instant::now() + READY_TIMEOUT;
        let mut last_error = "service did not accept connections".to_string();
        while Instant::now() < deadline {
            if !self.child_is_running(name)? {
                return Err(format!("{name} exited before becoming ready: {last_error}"));
            }
            match Self::probe_service(port, service) {
                Ok(()) if self.child_is_running(name)? => return Ok(()),
                Ok(()) => return Err(format!("{name} exited during its readiness check")),
                Err(error) => last_error = error,
            }
            std::thread::sleep(PROBE_INTERVAL);
        }
        Err(format!(
            "{name} did not become ready within {}s: {last_error}",
            READY_TIMEOUT.as_secs()
        ))
    }

    /// Spawn a sibling binary and wait for readiness, or reuse a listener only
    /// when it serves the expected versioned Choruz readiness contract.
    fn spawn_backend(
        &self,
        name: &'static str,
        bin: &str,
        port: u16,
        service: &str,
        env: &[(&str, String)],
        cwd: &Path,
    ) -> Result<(), String> {
        if Self::port_in_use(port) {
            Self::probe_service(port, service).map_err(|error| {
                format!("port {port} is occupied by an incompatible or unready service: {error}")
            })?;
            tracing::info!(name, port, "reusing compatible ready service");
            return Ok(());
        }
        let path = Self::sibling_binary_path(bin)
            .ok_or_else(|| format!("binary '{bin}' not found next to the host binary"))?;

        let mut cmd = Command::new(&path);
        cmd.current_dir(cwd);
        // Route children's stdout to our stderr so `choruz-server`'s own
        // stdout (used for the `CHORUZ_LISTENING=<port>` handshake line
        // read by SSH clients) doesn't get polluted by choruz-api-gateway /
        // pipeline tracing output.
        let child_stdout = dup_stderr_to_stdio();
        cmd.stdin(Stdio::null())
            .stdout(child_stdout)
            .stderr(Stdio::inherit());
        for (k, v) in env {
            cmd.env(k, v);
        }

        let child = cmd.spawn().map_err(|e| format!("spawn {bin}: {e}"))?;

        tracing::info!(name, pid = child.id(), "spawned backend process");
        self.children
            .lock()
            .unwrap()
            .push(ChildProc { name, child });
        self.wait_until_ready(name, port, service)
    }

    /// Spawn the Choruz backend stack against the supplied DATABASE_URL.
    /// The URL is whatever `pg::EmbeddedPg` allocated for our private
    /// cluster (port 5433, user `postgres`, db `choruz`) — the gateway +
    /// pipeline just see "a postgres" the same way as in dev.
    pub fn start_backend(&self, database_url: &str) -> Result<(), String> {
        let cwd = Self::host_working_dir().ok_or_else(|| {
            "could not resolve a Choruz source workspace or complete bundled host directory"
                .to_string()
        })?;
        tracing::info!(cwd = %cwd.display(), "workspace root for spawned processes");

        let db_url = database_url.to_string();

        let gateway_env = vec![
            ("CHORUZ_DATABASE_URL", db_url.clone()),
            (
                "CHORUZ_ATTACHMENT_DIR",
                ".choruz-runtime/attachments".to_string(),
            ),
            ("CHORUZ_API_HOST", "127.0.0.1".to_string()),
            ("CHORUZ_API_PORT", "3000".to_string()),
            (
                "RUST_LOG",
                std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
            ),
        ];
        let start = (|| {
            self.spawn_backend(
                "choruz-api-gateway",
                "choruz-api-gateway",
                3000,
                "choruz-api-gateway",
                &gateway_env,
                &cwd,
            )?;

            let pipeline_env = vec![
                ("CHORUZ_DATABASE_URL", db_url),
                ("CHORUZ_PIPELINE_METRICS_PORT", "3020".to_string()),
                ("CHORUZ_PIPELINE_METRICS_HOST", "127.0.0.1".to_string()),
                (
                    "RUST_LOG",
                    std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
                ),
            ];
            self.spawn_backend(
                "choruz-pipeline",
                "choruz-pipeline",
                3020,
                "choruz-pipeline",
                &pipeline_env,
                &cwd,
            )
        })();

        if start.is_err() {
            self.shutdown();
        }
        start
    }
}

/// Return a `Stdio` that redirects the child's stdout to the CURRENT
/// process's stderr. Used so children's tracing output doesn't pollute
/// a stdout-based handshake protocol (see `choruz-server`).
///
/// Unix: `dup(2)` the stderr fd and wrap in `File` → `Stdio::from(file)`.
/// If the dup fails for any reason we fall back to `Stdio::inherit()` so
/// we at least preserve log visibility.
#[cfg(unix)]
fn dup_stderr_to_stdio() -> Stdio {
    use std::os::fd::FromRawFd;
    // SAFETY: fd 2 (stderr) is guaranteed open in a well-formed Rust
    // program's start conditions. `libc::dup` either returns a fresh fd
    // we own or -1; we handle -1 by falling back.
    unsafe {
        let fd = libc::dup(2);
        if fd < 0 {
            return Stdio::inherit();
        }
        Stdio::from(std::fs::File::from_raw_fd(fd))
    }
}

#[cfg(not(unix))]
fn dup_stderr_to_stdio() -> Stdio {
    // Windows dup semantics differ; for MVP just fall back to inherit.
    // choruz-server is intended for *nix hosts (Linux dev boxes, macOS).
    Stdio::inherit()
}

impl Supervisor {
    /// Explicit cleanup — call from the signal handlers (SIGINT/SIGTERM).
    /// Drop also calls this as a last resort when the main thread unwinds
    /// normally. Idempotent: exited children are reaped without signalling
    /// them again.
    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let mut children = self.children.lock().unwrap();
        for c in children.iter_mut() {
            match stop_child(&mut c.child) {
                Ok(_) => tracing::info!(name = c.name, "stopped backend process"),
                Err(e) => {
                    tracing::warn!(name = c.name, error = %e, "kill failed (possibly already exited)")
                }
            }
        }
        // Best-effort reap so we don't leave zombies.
        for c in children.iter_mut() {
            let _ = c.child.wait();
        }
        children.clear();
    }
}

fn stop_child(child: &mut Child) -> std::io::Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }

    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    child.kill()?;

    let deadline = Instant::now() + SHUTDOWN_GRACE;
    while Instant::now() < deadline {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill()?;
    let _ = child.wait()?;
    Ok(())
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};

    fn serve_once(body: String) -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test service");
        let port = listener.local_addr().expect("test address").port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept readiness probe");
            let mut request = [0_u8; 512];
            let _ = stream.read(&mut request).expect("read readiness request");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write readiness response");
        });
        (port, handle)
    }

    #[test]
    fn readiness_probe_requires_service_identity_and_protocol() {
        let compatible = serde_json::json!({
            "status": "ready",
            "service": "choruz-api-gateway",
            "protocol_version": choruz_common::HOST_SERVICE_PROTOCOL_VERSION,
        })
        .to_string();
        let (port, server) = serve_once(compatible);
        assert!(Supervisor::probe_service(port, "choruz-api-gateway").is_ok());
        server.join().expect("join compatible test service");

        let incompatible = serde_json::json!({
            "status": "ready",
            "service": "not-choruz",
            "protocol_version": choruz_common::HOST_SERVICE_PROTOCOL_VERSION,
        })
        .to_string();
        let (port, server) = serve_once(incompatible);
        let error = Supervisor::probe_service(port, "choruz-api-gateway").unwrap_err();
        assert!(error.contains("expected service choruz-api-gateway"));
        server.join().expect("join incompatible test service");
    }

    #[test]
    fn bundled_host_uses_its_binary_directory_as_the_working_directory() {
        let root =
            std::env::temp_dir().join(format!("choruz-host-bundle-test-{}", std::process::id()));
        let bin = root.join("bin");
        fs::create_dir_all(bin.join("migrations")).expect("create bundle migrations");
        for name in ["choruz-server", "choruz-api-gateway", "choruz-pipeline"] {
            fs::write(bin.join(name), []).expect("create bundle binary");
        }

        assert_eq!(
            Supervisor::host_working_dir_from(&bin.join("choruz-server")),
            Some(bin.clone())
        );

        fs::remove_dir_all(root).expect("remove test bundle");
    }

    #[test]
    fn incomplete_bundle_is_not_a_valid_host_working_directory() {
        let root = std::env::temp_dir().join(format!(
            "choruz-host-incomplete-bundle-test-{}",
            std::process::id()
        ));
        let bin = root.join("bin");
        fs::create_dir_all(bin.join("migrations")).expect("create bundle migrations");
        fs::write(bin.join("choruz-server"), []).expect("create server binary");

        assert_eq!(
            Supervisor::host_working_dir_from(&bin.join("choruz-server")),
            None
        );

        fs::remove_dir_all(root).expect("remove incomplete test bundle");
    }

    #[cfg(unix)]
    #[test]
    fn stop_child_reaps_a_running_process() {
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        stop_child(&mut child).expect("stop child");
        assert!(child.try_wait().expect("inspect child").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn child_check_reports_exit_after_readiness() {
        let supervisor = Supervisor::new();
        let child = Command::new("sh")
            .arg("-c")
            .arg("exit 7")
            .spawn()
            .expect("spawn short-lived child");
        supervisor.children.lock().unwrap().push(ChildProc {
            name: "test-backend",
            child,
        });
        std::thread::sleep(Duration::from_millis(50));

        let error = supervisor
            .check_children()
            .expect_err("exit must be reported");
        assert!(error.contains("test-backend exited with exit status: 7"));
    }
}
