//! Sandbox lifecycle management.
//!
//! Each agent session gets an isolated workspace directory. The sandbox
//! provides directory-based isolation (or git worktree in the future).
//!
//! # Lifecycle
//!
//! 1. `create_sandbox(agent_id, config)` — creates a workspace directory
//! 2. The executor uses the sandbox's `work_dir` as the CWD for CLI processes
//! 3. `destroy_sandbox(handle)` — cleans up the workspace directory

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing;

use crate::error::{ExecutorError, ExecutorResult};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for workspace provisioning.
#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    /// Base directory under which all sandboxes are created.
    pub base_dir: PathBuf,

    /// Optional git repo to clone/worktree from.
    pub git_repo: Option<String>,

    /// Optional branch to check out.
    pub git_branch: Option<String>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            base_dir: PathBuf::from("/tmp/choruz-sandboxes"),
            git_repo: None,
            git_branch: None,
        }
    }
}

// ---------------------------------------------------------------------------
// SandboxHandle
// ---------------------------------------------------------------------------

/// An opaque handle to a running sandbox.
#[derive(Debug, Clone)]
pub struct SandboxHandle {
    /// Unique identifier for this sandbox (typically session_key).
    pub id: String,

    /// The agent that owns this sandbox.
    pub agent_id: String,

    /// The working directory for this sandbox.
    pub work_dir: PathBuf,

    /// Current epoch at the time of sandbox creation (used for fencing).
    pub epoch: i32,
}

// ---------------------------------------------------------------------------
// SandboxManager
// ---------------------------------------------------------------------------

/// Manages the lifecycle of sandbox directories.
#[derive(Clone)]
pub struct SandboxManager {
    config: WorkspaceConfig,
    sandboxes: Arc<RwLock<HashMap<String, SandboxHandle>>>,
}

impl SandboxManager {
    /// Create a new sandbox manager with the given configuration.
    pub fn new(config: WorkspaceConfig) -> Self {
        Self {
            config,
            sandboxes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a sandbox for the given agent and session key.
    ///
    /// The sandbox is a directory under `base_dir/{session_key}/`. If it
    /// already exists, it is reused.
    pub async fn create_sandbox(
        &self,
        session_key: &str,
        agent_id: &str,
        epoch: i32,
    ) -> ExecutorResult<SandboxHandle> {
        // Check if we already have a sandbox for this session
        {
            let sandboxes = self.sandboxes.read().await;
            if let Some(existing) = sandboxes.get(session_key) {
                tracing::debug!(
                    session_key,
                    work_dir = %existing.work_dir.display(),
                    "reusing existing sandbox"
                );
                return Ok(SandboxHandle {
                    epoch,
                    ..existing.clone()
                });
            }
        }

        // Build the workspace directory path
        // Sanitize session_key to use as directory name (replace : with _)
        let dir_name = session_key.replace(':', "_");
        let work_dir = self.config.base_dir.join(&dir_name);

        // Create the directory
        tokio::fs::create_dir_all(&work_dir).await.map_err(|e| {
            ExecutorError::Sandbox(format!(
                "failed to create sandbox dir {}: {e}",
                work_dir.display()
            ))
        })?;

        // If a git repo is configured, set up a worktree
        if let Some(ref repo) = self.config.git_repo {
            self.setup_git_worktree(&work_dir, repo, self.config.git_branch.as_deref())
                .await?;
        }

        let handle = SandboxHandle {
            id: session_key.to_string(),
            agent_id: agent_id.to_string(),
            work_dir: work_dir.clone(),
            epoch,
        };

        let mut sandboxes = self.sandboxes.write().await;
        sandboxes.insert(session_key.to_string(), handle.clone());

        tracing::info!(
            session_key,
            agent_id,
            work_dir = %work_dir.display(),
            "sandbox created"
        );

        Ok(handle)
    }

    /// Destroy a sandbox and clean up its workspace directory.
    pub async fn destroy_sandbox(&self, session_key: &str) -> ExecutorResult<()> {
        let handle = {
            let mut sandboxes = self.sandboxes.write().await;
            sandboxes.remove(session_key)
        };

        if let Some(handle) = handle {
            if handle.work_dir.exists() {
                tokio::fs::remove_dir_all(&handle.work_dir)
                    .await
                    .map_err(|e| {
                        ExecutorError::Sandbox(format!(
                            "failed to remove sandbox dir {}: {e}",
                            handle.work_dir.display()
                        ))
                    })?;
            }

            tracing::info!(
                session_key,
                work_dir = %handle.work_dir.display(),
                "sandbox destroyed"
            );
        } else {
            tracing::warn!(session_key, "sandbox not found for destruction");
        }

        Ok(())
    }

    /// Get the sandbox handle for a session, if it exists.
    pub async fn get_sandbox(&self, session_key: &str) -> Option<SandboxHandle> {
        let sandboxes = self.sandboxes.read().await;
        sandboxes.get(session_key).cloned()
    }

    /// Get or create a sandbox for the given session.
    pub async fn get_or_create_sandbox(
        &self,
        session_key: &str,
        agent_id: &str,
        epoch: i32,
    ) -> ExecutorResult<SandboxHandle> {
        if let Some(existing) = self.get_sandbox(session_key).await {
            return Ok(SandboxHandle { epoch, ..existing });
        }
        self.create_sandbox(session_key, agent_id, epoch).await
    }

    /// List all active sandbox session keys.
    pub async fn list_sandboxes(&self) -> Vec<String> {
        let sandboxes = self.sandboxes.read().await;
        sandboxes.keys().cloned().collect()
    }

    /// Set up a git worktree in the sandbox directory.
    async fn setup_git_worktree(
        &self,
        work_dir: &Path,
        repo_path: &str,
        branch: Option<&str>,
    ) -> ExecutorResult<()> {
        // Check if this is already a git directory
        let git_dir = work_dir.join(".git");
        if git_dir.exists() {
            tracing::debug!(
                work_dir = %work_dir.display(),
                "git worktree already exists, skipping setup"
            );
            return Ok(());
        }

        let mut cmd = tokio::process::Command::new("git");
        cmd.arg("worktree")
            .arg("add")
            .arg("--detach")
            .arg(work_dir)
            .current_dir(repo_path);

        if let Some(branch) = branch {
            cmd.arg(branch);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| ExecutorError::Sandbox(format!("failed to run git worktree: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ExecutorError::Sandbox(format!(
                "git worktree failed: {stderr}"
            )));
        }

        tracing::info!(
            work_dir = %work_dir.display(),
            repo_path,
            "git worktree created"
        );

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_destroy_sandbox() {
        let tmp = tempfile::tempdir().unwrap();
        let config = WorkspaceConfig {
            base_dir: tmp.path().to_path_buf(),
            git_repo: None,
            git_branch: None,
        };

        let mgr = SandboxManager::new(config);

        // Create
        let handle = mgr
            .create_sandbox("agent-1:conv-1", "agent-1", 1)
            .await
            .unwrap();
        assert_eq!(handle.id, "agent-1:conv-1");
        assert_eq!(handle.agent_id, "agent-1");
        assert_eq!(handle.epoch, 1);
        assert!(handle.work_dir.exists());

        // List
        let keys = mgr.list_sandboxes().await;
        assert_eq!(keys.len(), 1);
        assert!(keys.contains(&"agent-1:conv-1".to_string()));

        // Destroy
        mgr.destroy_sandbox("agent-1:conv-1").await.unwrap();
        assert!(!handle.work_dir.exists());

        let keys = mgr.list_sandboxes().await;
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn get_or_create_reuses_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let config = WorkspaceConfig {
            base_dir: tmp.path().to_path_buf(),
            git_repo: None,
            git_branch: None,
        };

        let mgr = SandboxManager::new(config);

        let h1 = mgr
            .get_or_create_sandbox("agent-1:conv-1", "agent-1", 1)
            .await
            .unwrap();
        let h2 = mgr
            .get_or_create_sandbox("agent-1:conv-1", "agent-1", 2)
            .await
            .unwrap();

        // Same work_dir, but epoch updated
        assert_eq!(h1.work_dir, h2.work_dir);
        assert_eq!(h2.epoch, 2);
    }

    #[tokio::test]
    async fn destroy_nonexistent_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let config = WorkspaceConfig {
            base_dir: tmp.path().to_path_buf(),
            git_repo: None,
            git_branch: None,
        };

        let mgr = SandboxManager::new(config);
        // Should not error
        mgr.destroy_sandbox("does-not-exist").await.unwrap();
    }

    #[tokio::test]
    async fn multiple_sandboxes_are_isolated() {
        let tmp = tempfile::tempdir().unwrap();
        let config = WorkspaceConfig {
            base_dir: tmp.path().to_path_buf(),
            git_repo: None,
            git_branch: None,
        };

        let mgr = SandboxManager::new(config);

        let h1 = mgr
            .create_sandbox("agent-1:conv-1", "agent-1", 1)
            .await
            .unwrap();
        let h2 = mgr
            .create_sandbox("agent-2:conv-2", "agent-2", 1)
            .await
            .unwrap();

        // Different work dirs
        assert_ne!(h1.work_dir, h2.work_dir);
        assert!(h1.work_dir.exists());
        assert!(h2.work_dir.exists());

        let keys = mgr.list_sandboxes().await;
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn sandbox_dir_uses_sanitized_session_key() {
        let tmp = tempfile::tempdir().unwrap();
        let config = WorkspaceConfig {
            base_dir: tmp.path().to_path_buf(),
            git_repo: None,
            git_branch: None,
        };

        let mgr = SandboxManager::new(config);
        let handle = mgr
            .create_sandbox("agent-1:conv-1", "agent-1", 1)
            .await
            .unwrap();

        // The : should be replaced with _
        assert!(
            handle
                .work_dir
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("agent-1_conv-1")
        );
    }
}
