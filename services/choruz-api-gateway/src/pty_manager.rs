//! Cross-platform process containment for PTY child processes.
//!
//! When a PTY spawns a CLI tool (e.g. `claude`, `codex`), that tool may itself
//! spawn subprocesses.  The default portable-pty `Child::kill()` only sends a
//! signal to the *direct* child PID — any grandchildren become orphans.
//!
//! This module wraps the child's entire process tree so that cleanup is
//! reliable:
//!
//! - **macOS / generic Unix**: portable-pty already calls `setsid()` before
//!   exec, making the child a *session leader*.  We record the child PID
//!   (which equals the session ID) and on teardown send `SIGKILL` to the
//!   entire process group via `killpg()`, then fall back to `kill(-sid, …)`.
//!
//! - **Linux** (when cgroup v2 is available): we additionally place the child
//!   in a dedicated cgroup scope (`/sys/fs/cgroup/choruz/pty-{id}/`).  On
//!   teardown we freeze the cgroup (to prevent fork-escapes), then write `1`
//!   to `cgroup.kill` (kernel ≥ 5.14) for an atomic wipe.  If the kernel is
//!   too old or cgroup setup fails, we fall back to the session-kill path.

use std::io;

// ── Public container type ───────────────────────────────────────────────

/// A process container that wraps a child process tree for reliable cleanup.
///
/// Created via [`ProcessContainer::new`] right after `portable_pty` spawns the
/// child.  Call [`ProcessContainer::kill_all`] to tear down every process in
/// the tree.  The `Drop` impl calls `kill_all` as a safety net.
pub struct ProcessContainer {
    /// Human-readable identifier, e.g. `"pty-{binding_id}"`.
    id: String,
    inner: PlatformContainer,
}

impl ProcessContainer {
    /// Wrap an already-spawned child.  `child_pid` **must** be the direct child
    /// returned by `portable_pty::SlavePty::spawn_command`, which is also the
    /// session leader (because portable-pty calls `setsid()`).
    pub fn new(id: impl Into<String>, child_pid: u32) -> Self {
        let id = id.into();
        let inner = PlatformContainer::new(&id, child_pid);
        Self { id, inner }
    }

    /// Kill every process in the container.  Safe to call more than once.
    pub fn kill_all(&self) {
        self.inner.kill_all(&self.id);
    }
}

impl Drop for ProcessContainer {
    fn drop(&mut self) {
        self.kill_all();
    }
}

// ── Platform dispatch ───────────────────────────────────────────────────

#[cfg(target_os = "linux")]
struct PlatformContainer {
    session_kill: SessionKillContainer,
    cgroup: Option<CgroupContainer>,
}

#[cfg(not(target_os = "linux"))]
struct PlatformContainer {
    session_kill: SessionKillContainer,
}

#[cfg(target_os = "linux")]
impl PlatformContainer {
    fn new(id: &str, child_pid: u32) -> Self {
        let session_kill = SessionKillContainer::new(child_pid);
        let cgroup = match CgroupContainer::create(id, child_pid) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(
                    container_id = %id,
                    error = %e,
                    "cgroup containment unavailable, falling back to session-kill"
                );
                None
            }
        };
        Self {
            session_kill,
            cgroup,
        }
    }

    fn kill_all(&self, id: &str) {
        // Prefer cgroup kill (atomic, no fork-escape) when available.
        if let Some(ref cg) = self.cgroup {
            if let Err(e) = cg.kill_all() {
                tracing::warn!(
                    container_id = %id,
                    error = %e,
                    "cgroup kill failed, falling back to session-kill"
                );
                self.session_kill.kill_all(id);
            }
        } else {
            self.session_kill.kill_all(id);
        }
    }
}

#[cfg(not(target_os = "linux"))]
impl PlatformContainer {
    fn new(_id: &str, child_pid: u32) -> Self {
        Self {
            session_kill: SessionKillContainer::new(child_pid),
        }
    }

    fn kill_all(&self, id: &str) {
        self.session_kill.kill_all(id);
    }
}

// ── macOS / generic Unix: session-kill ──────────────────────────────────
//
// portable-pty's `pre_exec` calls `setsid()`, so the child PID == session ID
// == initial process group ID.  We use `killpg(pgid, sig)` which targets the
// process group, catching any grandchildren that haven't called `setpgid`.

struct SessionKillContainer {
    /// The child PID, which is also the session leader and initial PGID.
    child_pid: u32,
}

impl SessionKillContainer {
    fn new(child_pid: u32) -> Self {
        Self { child_pid }
    }

    /// Send SIGTERM to the process group, wait briefly, then SIGKILL the group.
    fn kill_all(&self, id: &str) {
        let pgid = self.child_pid as i32;

        // First try graceful shutdown via SIGTERM to the whole process group.
        let term_result = unsafe { libc::killpg(pgid, libc::SIGTERM) };
        if term_result == 0 {
            tracing::debug!(container_id = %id, pgid, "sent SIGTERM to process group");
        } else {
            let err = io::Error::last_os_error();
            // ESRCH means no such process (group) — already dead, which is fine.
            if err.raw_os_error() != Some(libc::ESRCH) {
                tracing::warn!(
                    container_id = %id,
                    pgid,
                    error = %err,
                    "killpg(SIGTERM) failed"
                );
            }
            return; // Nothing left to kill.
        }

        // Give processes a brief window to handle SIGTERM (200 ms).
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Now force-kill anything remaining.
        let kill_result = unsafe { libc::killpg(pgid, libc::SIGKILL) };
        if kill_result == 0 {
            tracing::info!(container_id = %id, pgid, "sent SIGKILL to process group");
        } else {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::ESRCH) {
                tracing::warn!(
                    container_id = %id,
                    pgid,
                    error = %err,
                    "killpg(SIGKILL) failed"
                );
            }
        }
    }
}

// ── Linux: cgroup v2 containment ────────────────────────────────────────

#[cfg(target_os = "linux")]
struct CgroupContainer {
    cgroup_path: std::path::PathBuf,
}

#[cfg(target_os = "linux")]
impl CgroupContainer {
    /// Base path under which we create per-PTY cgroups.
    const CHORUZ_CGROUP_BASE: &'static str = "/sys/fs/cgroup/choruz";

    /// Create a cgroup scope for the given container `id` and move `child_pid`
    /// into it.
    fn create(id: &str, child_pid: u32) -> io::Result<Self> {
        use std::fs;
        use std::path::PathBuf;

        let base = PathBuf::from(Self::CHORUZ_CGROUP_BASE);

        // Ensure parent cgroup exists.
        if !base.exists() {
            fs::create_dir_all(&base).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("cannot create cgroup base {}: {e}", base.display()),
                )
            })?;

            // Enable controllers we may need in the subtree.
            // Writing to cgroup.subtree_control is best-effort; if it fails
            // (e.g. the controller isn't delegated to us) we continue anyway
            // because we only truly need cgroup.procs + cgroup.kill.
            let subtree_ctrl = base.join("cgroup.subtree_control");
            if subtree_ctrl.exists() {
                if let Err(e) = fs::write(&subtree_ctrl, "+pids") {
                    tracing::debug!(path = %subtree_ctrl.display(), error = %e, "cleanup: cgroup subtree_control write failed");
                }
            }
        }

        let cgroup_path = base.join(format!("pty-{id}"));
        fs::create_dir_all(&cgroup_path).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("cannot create cgroup {}: {e}", cgroup_path.display()),
            )
        })?;

        // Move the child into this cgroup.
        let procs_file = cgroup_path.join("cgroup.procs");
        fs::write(&procs_file, child_pid.to_string()).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "cannot add pid {child_pid} to {}: {e}",
                    procs_file.display()
                ),
            )
        })?;

        tracing::info!(
            container_id = %id,
            pid = child_pid,
            cgroup = %cgroup_path.display(),
            "child placed in cgroup v2 scope"
        );

        Ok(Self { cgroup_path })
    }

    /// Freeze the cgroup, then atomically kill all processes inside it.
    fn kill_all(&self) -> io::Result<()> {
        use std::fs;

        // Step 1: freeze to prevent fork-escapes.
        let freeze_file = self.cgroup_path.join("cgroup.freeze");
        if freeze_file.exists() {
            if let Err(e) = fs::write(&freeze_file, "1") {
                tracing::warn!(
                    cgroup = %self.cgroup_path.display(),
                    error = %e,
                    "cgroup.freeze write failed (continuing)"
                );
            }
        }

        // Step 2: try atomic cgroup.kill (Linux 5.14+).
        let kill_file = self.cgroup_path.join("cgroup.kill");
        if kill_file.exists() {
            fs::write(&kill_file, "1").map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!(
                        "cgroup.kill write failed for {}: {e}",
                        self.cgroup_path.display()
                    ),
                )
            })?;
        } else {
            // Fallback for older kernels: read cgroup.procs and kill each PID.
            self.kill_all_procs_fallback()?;
        }

        // Step 3: remove the cgroup directory.  It might not be immediately
        // removable if zombie processes still reference it, so we retry briefly.
        for attempt in 0..5 {
            match fs::remove_dir(&self.cgroup_path) {
                Ok(()) => break,
                Err(e) if attempt < 4 => {
                    tracing::trace!(
                        cgroup = %self.cgroup_path.display(),
                        attempt,
                        error = %e,
                        "cgroup rmdir retry"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => {
                    tracing::warn!(
                        cgroup = %self.cgroup_path.display(),
                        error = %e,
                        "could not remove cgroup directory (will be cleaned up later)"
                    );
                }
            }
        }

        Ok(())
    }

    /// Fallback for kernels without `cgroup.kill`: read `cgroup.procs` in a
    /// loop and SIGKILL each PID until the cgroup is empty.
    fn kill_all_procs_fallback(&self) -> io::Result<()> {
        use std::fs;

        let procs_file = self.cgroup_path.join("cgroup.procs");

        for round in 0..10 {
            let content = fs::read_to_string(&procs_file)?;
            let pids: Vec<i32> = content
                .split_whitespace()
                .filter_map(|s| s.parse::<i32>().ok())
                .collect();

            if pids.is_empty() {
                return Ok(());
            }

            for pid in &pids {
                unsafe {
                    libc::kill(*pid, libc::SIGKILL);
                }
            }

            if round < 9 {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }

        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for CgroupContainer {
    fn drop(&mut self) {
        if let Err(e) = self.kill_all() {
            tracing::warn!(
                cgroup = %self.cgroup_path.display(),
                error = %e,
                "CgroupContainer::drop kill_all failed"
            );
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_kill_handles_nonexistent_pid() {
        // Killing a process group that doesn't exist should not panic.
        let container = ProcessContainer::new("test-nonexistent", 999_999_999);
        container.kill_all(); // Should log ESRCH and return gracefully.
    }

    #[test]
    fn container_can_be_created_and_dropped() {
        // Verify basic construction and drop don't panic.
        let _container = ProcessContainer::new("test-drop", 999_999_999);
        // Drop runs here — should handle ESRCH silently.
    }
}
