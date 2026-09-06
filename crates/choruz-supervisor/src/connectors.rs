//! Lifecycle supervision for persistent runtime-host connectors.

use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::supervisor::stop_child;

const SCAN_INTERVAL: Duration = Duration::from_millis(500);
const STABLE_RUN: Duration = Duration::from_secs(60);
const RESTART_DELAYS: &[Duration] = &[
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

/// Resolve the directory shared by onboarding and connector supervision.
/// An explicit instance directory must be absolute; invalid configuration does
/// not fall back to the user's default connector store.
pub fn connector_config_directory() -> Result<PathBuf, String> {
    resolve_config_directory(
        std::env::var_os("CHORUZ_CONNECTOR_CONFIG_DIR").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn resolve_config_directory(
    configured: Option<PathBuf>,
    user_directory: Option<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(directory) = configured {
        if !directory.is_absolute() {
            return Err("CHORUZ_CONNECTOR_CONFIG_DIR must be an absolute path".into());
        }
        return Ok(directory);
    }
    user_directory
        .map(|directory| directory.join(".choruz").join("connectors"))
        .ok_or_else(|| "HOME is not set; configure CHORUZ_CONNECTOR_CONFIG_DIR".into())
}

/// Owns every connector config in one directory until this guard is dropped.
pub struct ConnectorSupervisor {
    stop: mpsc::Sender<()>,
    monitor: Option<thread::JoinHandle<()>>,
}

struct ManagedConnector {
    child: Option<Child>,
    failures: usize,
    retry_at: Instant,
    started_at: Option<Instant>,
}

impl ManagedConnector {
    fn pending(now: Instant) -> Self {
        Self {
            child: None,
            failures: 0,
            retry_at: now,
            started_at: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Timing {
    scan_interval: Duration,
    stable_run: Duration,
    restart_delays: &'static [Duration],
}

impl ConnectorSupervisor {
    /// Spawn the monitor for the configured connector directory.
    ///
    /// Dropping the returned guard stops and reaps every connector it owns. The
    /// function fails when the shared directory configuration is invalid.
    pub fn start_default() -> Result<Self, String> {
        let directory = connector_config_directory()?;
        let binary = std::env::var_os("CHORUZ_CONNECTOR_BINARY")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::current_exe().ok().and_then(|executable| {
                    executable
                        .parent()
                        .map(|parent| parent.join("choruz-connector"))
                        .filter(|candidate| candidate.is_file())
                })
            })
            .unwrap_or_else(|| PathBuf::from("choruz-connector"));
        Ok(Self::start(directory, binary, default_timing()))
    }

    fn start(directory: PathBuf, binary: PathBuf, timing: Timing) -> Self {
        let (stop, stopped) = mpsc::channel();
        let monitor = thread::spawn(move || {
            supervise(directory, binary, timing, stopped);
        });
        Self {
            stop,
            monitor: Some(monitor),
        }
    }
}

impl Drop for ConnectorSupervisor {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(monitor) = self.monitor.take()
            && monitor.join().is_err()
        {
            tracing::error!("connector supervisor thread panicked during shutdown");
        }
    }
}

fn default_timing() -> Timing {
    Timing {
        scan_interval: SCAN_INTERVAL,
        stable_run: STABLE_RUN,
        restart_delays: RESTART_DELAYS,
    }
}

fn supervise(directory: PathBuf, binary: PathBuf, timing: Timing, stopped: mpsc::Receiver<()>) {
    let mut connectors = HashMap::new();
    loop {
        reconcile(&directory, &binary, timing, &mut connectors);
        match stopped.recv_timeout(timing.scan_interval) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    for (config, mut connector) in connectors {
        if let Some(child) = connector.child.as_mut()
            && let Err(error) = stop_child(child)
        {
            tracing::warn!(%error, config = %config.display(), "connector did not stop cleanly");
        }
    }
}

fn reconcile(
    directory: &Path,
    binary: &Path,
    timing: Timing,
    connectors: &mut HashMap<PathBuf, ManagedConnector>,
) {
    let configs = connector_configs(directory);
    let desired = configs.iter().cloned().collect::<HashSet<_>>();
    connectors.retain(|config, connector| {
        if desired.contains(config) {
            return true;
        }
        if let Some(child) = connector.child.as_mut()
            && let Err(error) = stop_child(child)
        {
            tracing::warn!(%error, config = %config.display(), "removed connector did not stop cleanly");
        }
        tracing::info!(config = %config.display(), "stopped removed connector config");
        false
    });

    let now = Instant::now();
    for config in configs {
        let connector = connectors
            .entry(config.clone())
            .or_insert_with(|| ManagedConnector::pending(now));
        inspect_child(&config, connector, now, timing);
        if connector.child.is_none()
            && now >= connector.retry_at
            && !another_connector_holds_lock(&config)
        {
            match spawn_connector(binary, &config) {
                Ok(child) => {
                    tracing::info!(pid = child.id(), config = %config.display(), "started managed connector");
                    connector.child = Some(child);
                    connector.started_at = Some(now);
                }
                Err(error) => {
                    schedule_restart(&config, connector, now, timing, &error);
                }
            }
        }
    }
}

fn inspect_child(config: &Path, connector: &mut ManagedConnector, now: Instant, timing: Timing) {
    let Some(child) = connector.child.as_mut() else {
        return;
    };
    match child.try_wait() {
        Ok(None) => {
            if connector
                .started_at
                .is_some_and(|started| now.duration_since(started) >= timing.stable_run)
            {
                connector.failures = 0;
            }
        }
        Ok(Some(status)) => {
            connector.child = None;
            schedule_restart(
                config,
                connector,
                now,
                timing,
                &format!("connector exited with {status}"),
            );
        }
        Err(error) => {
            if let Some(mut child) = connector.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            schedule_restart(
                config,
                connector,
                now,
                timing,
                &format!("inspect connector: {error}"),
            );
        }
    }
}

fn schedule_restart(
    config: &Path,
    connector: &mut ManagedConnector,
    now: Instant,
    timing: Timing,
    reason: &str,
) {
    let index = connector.failures.min(timing.restart_delays.len() - 1);
    let delay = timing.restart_delays[index];
    connector.failures = connector.failures.saturating_add(1);
    connector.retry_at = now + delay;
    connector.started_at = None;
    tracing::warn!(
        %reason,
        config = %config.display(),
        retry_ms = delay.as_millis(),
        "managed connector stopped; scheduling restart"
    );
}

fn another_connector_holds_lock(config: &Path) -> bool {
    let Ok(lock) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(config.with_extension("lock"))
    else {
        return false;
    };
    match fs2::FileExt::try_lock_exclusive(&lock) {
        Ok(()) => {
            let _ = fs2::FileExt::unlock(&lock);
            false
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => true,
        Err(_) => false,
    }
}

fn connector_configs(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut configs = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    configs.sort();
    configs
}

fn spawn_connector(binary: &Path, config: &Path) -> Result<Child, String> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(config.with_extension("log"))
        .map_err(|error| format!("open connector log: {error}"))?;
    let stderr = log
        .try_clone()
        .map_err(|error| format!("clone connector log: {error}"))?;
    Command::new(binary)
        .arg("run")
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| format!("start connector: {error}"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use fs2::FileExt;
    use std::{fs, os::unix::fs::PermissionsExt};

    const TEST_DELAYS: &[Duration] = &[Duration::from_millis(20), Duration::from_millis(40)];

    #[test]
    fn instance_directory_is_explicit_and_invalid_values_do_not_fall_back() {
        let temp = tempfile::tempdir().unwrap();
        let user = temp.path().join("user");
        let instance = temp.path().join("instance");
        assert_eq!(
            resolve_config_directory(None, Some(user.clone())).unwrap(),
            user.join(".choruz/connectors")
        );
        assert_eq!(
            resolve_config_directory(Some(instance.clone()), Some(user.clone())).unwrap(),
            instance
        );
        assert_eq!(
            resolve_config_directory(Some(instance.clone()), None).unwrap(),
            instance
        );
        assert!(resolve_config_directory(Some(PathBuf::new()), Some(user.clone())).is_err());
        assert!(resolve_config_directory(Some("relative".into()), Some(user)).is_err());
        assert!(resolve_config_directory(None, None).is_err());
    }

    fn wait_until(description: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if condition() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for {description}");
    }

    #[test]
    fn discovers_new_configs_restarts_crashes_and_stops_removed_connectors() {
        let temp = tempfile::tempdir().expect("temporary connector directory");
        let counter = temp.path().join("starts");
        let stopped = temp.path().join("stopped");
        let script = temp.path().join("fake-connector");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf x >> '{}'\nif [ \"$(wc -c < '{}')\" -lt 2 ]; then exit 17; fi\ntrap \"touch '{}'; exit 0\" TERM INT\nwhile :; do sleep 0.1; done\n",
                counter.display(),
                counter.display(),
                stopped.display()
            ),
        )
        .expect("write fake connector");
        let mut permissions = fs::metadata(&script)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&script, permissions).expect("make script executable");

        let supervisor = ConnectorSupervisor::start(
            temp.path().to_path_buf(),
            script,
            Timing {
                scan_interval: Duration::from_millis(10),
                stable_run: Duration::from_millis(100),
                restart_delays: TEST_DELAYS,
            },
        );
        let config = temp.path().join("device.json");
        fs::write(&config, "{}").expect("add connector config after supervisor starts");

        wait_until("crashed connector to restart", || {
            fs::read(&counter).is_ok_and(|starts| starts.len() >= 2)
        });
        fs::remove_file(config).expect("remove connector config");
        wait_until("removed connector to stop", || stopped.exists());
        drop(supervisor);
    }

    #[test]
    fn restart_delay_caps_at_the_longest_backoff() {
        let timing = Timing {
            scan_interval: Duration::ZERO,
            stable_run: Duration::from_secs(1),
            restart_delays: TEST_DELAYS,
        };
        let now = Instant::now();
        let mut connector = ManagedConnector::pending(now);
        connector.failures = 20;
        schedule_restart(
            Path::new("device.json"),
            &mut connector,
            now,
            timing,
            "failed",
        );
        assert_eq!(connector.retry_at.duration_since(now), TEST_DELAYS[1]);
    }

    #[test]
    fn an_existing_connector_lock_prevents_a_duplicate_process() {
        let temp = tempfile::tempdir().expect("temporary connector directory");
        let config = temp.path().join("device.json");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(config.with_extension("lock"))
            .expect("open connector lock");
        lock.try_lock_exclusive().expect("hold connector lock");
        assert!(another_connector_holds_lock(&config));
        FileExt::unlock(&lock).expect("release connector lock");
        assert!(!another_connector_holds_lock(&config));
    }
}
