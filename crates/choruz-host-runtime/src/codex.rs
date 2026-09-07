//! Binding-owned Codex homes and the session file a Codex terminal writes.
//!
//! Codex stores sessions globally, keyed by nothing that identifies a Choruz
//! binding, so every Codex terminal runs with its own managed `CODEX_HOME`
//! under the runtime directory. The home shares the user's config, auth and
//! plugins through symlinks and keeps a private `sessions/` tree, which is
//! what makes the one new `.jsonl` file after spawn attributable.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};

use choruz_common::AppError;
use serde::{Deserialize, Serialize};

const MANAGED_CODEX_HOME_DIR: &str = "codex-homes";

#[derive(Debug, Clone)]
pub struct ManagedCodexHome {
    pub home_path: PathBuf,
    pub sessions_path: PathBuf,
}

/// One Codex session file and the identity written in its `session_meta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexSessionFileMeta {
    pub session_id: String,
    pub cwd: String,
    pub path: PathBuf,
}

pub fn choruz_runtime_dir() -> PathBuf {
    std::env::var("CHORUZ_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".choruz-runtime"))
}

pub fn normal_codex_home() -> Option<PathBuf> {
    std::env::var("CODEX_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|home| PathBuf::from(home).join(".codex"))
        })
}

fn redact_local_path_error(context: &str, error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("{context}: {error}"))
}

fn safe_binding_path_segment(binding_id: &str) -> Result<&str, AppError> {
    let valid = !binding_id.is_empty()
        && binding_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if !valid {
        return Err(AppError::Validation("invalid terminal binding id".into()));
    }
    Ok(binding_id)
}

#[cfg(unix)]
fn ensure_owner_only_dir(path: &Path) -> Result<(), AppError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(AppError::Internal(
            "managed Codex runtime path cannot be a symlink".into(),
        ));
    }
    fs::create_dir_all(path)
        .map_err(|error| redact_local_path_error("create runtime dir", error))?;
    let symlink_metadata = fs::symlink_metadata(path)
        .map_err(|error| redact_local_path_error("stat runtime dir", error))?;
    if symlink_metadata.file_type().is_symlink() || !symlink_metadata.is_dir() {
        return Err(AppError::Internal(
            "managed Codex runtime path must be a local directory".into(),
        ));
    }
    let mut permissions = fs::metadata(path)
        .map_err(|error| redact_local_path_error("stat runtime dir", error))?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
        .map_err(|error| redact_local_path_error("chmod runtime dir", error))
}

#[cfg(unix)]
fn ensure_local_dir_not_symlink(path: &Path) -> Result<(), AppError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(AppError::Internal(
            "managed Codex sessions path cannot be a symlink".into(),
        ));
    }
    ensure_owner_only_dir(path)
}

#[cfg(unix)]
fn ensure_shared_codex_link(
    source_home: &Path,
    managed_home: &Path,
    name: &str,
) -> Result<(), AppError> {
    let source = source_home.join(name);
    if !source.exists() {
        return Ok(());
    }

    let target = managed_home.join(name);
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if !metadata.file_type().is_symlink() {
            return Err(AppError::Internal(
                "managed Codex home contains an unexpected local entry".into(),
            ));
        }
        let target_canonical = fs::canonicalize(&target)
            .map_err(|error| redact_local_path_error("validate Codex shared link", error))?;
        let source_canonical = fs::canonicalize(&source)
            .map_err(|error| redact_local_path_error("validate Codex shared source", error))?;
        if target_canonical != source_canonical {
            return Err(AppError::Internal(
                "managed Codex shared link points outside the intended source".into(),
            ));
        }
        return Ok(());
    }

    symlink(&source, &target)
        .map_err(|error| redact_local_path_error("link Codex shared state", error))
}

/// Create or reuse the managed home for one binding. `source_home` is the
/// isolated Harness account profile when the binding has one; otherwise the
/// user's normal Codex home is shared.
#[cfg(unix)]
pub fn provision_managed_codex_home(
    binding_id: &str,
    workspace_path: &str,
    source_home: Option<&Path>,
) -> Result<ManagedCodexHome, AppError> {
    let binding_segment = safe_binding_path_segment(binding_id)?;
    let runtime_dir = choruz_runtime_dir();
    ensure_owner_only_dir(&runtime_dir)?;
    let homes_root = runtime_dir.join(MANAGED_CODEX_HOME_DIR);
    ensure_owner_only_dir(&homes_root)?;

    let home_path = homes_root.join(binding_segment);
    ensure_owner_only_dir(&home_path)?;
    let sessions_path = home_path.join("sessions");
    ensure_local_dir_not_symlink(&sessions_path)?;

    if let Some(source_home) = source_home
        .map(Path::to_path_buf)
        .or_else(normal_codex_home)
        && source_home != home_path
    {
        for entry in ["config.toml", "auth.json", "plugins", "skills", "cache"] {
            ensure_shared_codex_link(&source_home, &home_path, entry)?;
        }
    }

    let canonical_home = fs::canonicalize(&home_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex home", error))?;
    let canonical_sessions = fs::canonicalize(&sessions_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex sessions", error))?;

    if !canonical_sessions.starts_with(&canonical_home) {
        return Err(AppError::Internal(
            "managed Codex sessions path escaped its home".into(),
        ));
    }

    if let Ok(workspace_canonical) = fs::canonicalize(workspace_path)
        && canonical_home.starts_with(workspace_canonical)
    {
        return Err(AppError::Internal(
            "managed Codex home must be outside the agent workspace".into(),
        ));
    }

    Ok(ManagedCodexHome {
        home_path: canonical_home,
        sessions_path: canonical_sessions,
    })
}

#[cfg(not(unix))]
pub fn provision_managed_codex_home(
    _binding_id: &str,
    _workspace_path: &str,
    _source_home: Option<&Path>,
) -> Result<ManagedCodexHome, AppError> {
    Err(AppError::Internal(
        "managed Codex homes require a Unix filesystem".into(),
    ))
}

/// Every regular `.jsonl` file under the sessions tree, canonical, symlinks
/// skipped; the spawn baseline that new session files are compared against.
pub fn collect_codex_session_files(sessions_path: &Path) -> Result<HashSet<String>, AppError> {
    let canonical_sessions = fs::canonicalize(sessions_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex sessions", error))?;
    let mut stack = vec![canonical_sessions.clone()];
    let mut files = HashSet::new();

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(redact_local_path_error("read Codex sessions", error)),
        };
        for entry in entries {
            let entry = entry
                .map_err(|error| redact_local_path_error("read Codex session entry", error))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| redact_local_path_error("stat Codex session entry", error))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if !metadata.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            {
                continue;
            }
            let canonical = fs::canonicalize(&path).map_err(|error| {
                redact_local_path_error("canonicalize Codex session file", error)
            })?;
            if canonical.starts_with(&canonical_sessions) {
                files.insert(canonical.to_string_lossy().to_string());
            }
        }
    }

    Ok(files)
}

pub fn read_codex_session_meta(path: &Path) -> Result<Option<CodexSessionFileMeta>, AppError> {
    let file = fs::File::open(path)
        .map_err(|error| redact_local_path_error("read Codex session", error))?;
    let reader = std::io::BufReader::new(file);
    for line in std::io::BufRead::lines(reader).take(20) {
        let line = line.map_err(|error| redact_local_path_error("read Codex session", error))?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
            continue;
        }
        let payload = value.get("payload").unwrap_or(&value);
        let Some(session_id) = payload
            .get("id")
            .or_else(|| payload.get("session_id"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
        else {
            return Ok(None);
        };
        let Some(cwd) = payload
            .get("cwd")
            .or_else(|| payload.get("workspace_path"))
            .and_then(|v| v.as_str())
            .filter(|v| !v.trim().is_empty())
        else {
            return Ok(None);
        };
        return Ok(Some(CodexSessionFileMeta {
            session_id: session_id.to_string(),
            cwd: cwd.to_string(),
            path: path.to_path_buf(),
        }));
    }
    Ok(None)
}

/// The one session file that appeared under `sessions_path` since the
/// baseline and was written for `workspace_path`. Two or more candidates fail
/// closed: a wrong attribution would resume another conversation's history.
pub fn unique_new_codex_session(
    home_path: &Path,
    sessions_path: &Path,
    baseline_session_files: &[String],
    workspace_path: &str,
) -> Result<Option<CodexSessionFileMeta>, AppError> {
    let home = fs::canonicalize(home_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex home", error))?;
    let sessions = fs::canonicalize(sessions_path)
        .map_err(|error| redact_local_path_error("canonicalize Codex sessions", error))?;
    if !sessions.starts_with(&home) {
        return Ok(None);
    }

    let baseline: HashSet<&str> = baseline_session_files.iter().map(String::as_str).collect();
    let mut candidates = Vec::new();
    for path in collect_codex_session_files(&sessions)? {
        if baseline.contains(path.as_str()) {
            continue;
        }
        let canonical = fs::canonicalize(PathBuf::from(path))
            .map_err(|error| redact_local_path_error("canonicalize Codex session", error))?;
        if !canonical.starts_with(&sessions) {
            continue;
        }
        let Some(meta) = read_codex_session_meta(&canonical)? else {
            continue;
        };
        if meta.cwd == workspace_path {
            candidates.push(CodexSessionFileMeta {
                path: canonical,
                ..meta
            });
        }
    }

    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        _ => {
            tracing::warn!(
                candidate_count = candidates.len(),
                "Codex terminal session attribution failed closed with multiple candidates"
            );
            Ok(None)
        }
    }
}

/// Whether the anchored session file still lives in the managed sessions
/// tree and still names the anchored session and workspace.
pub fn codex_anchor_file_matches(
    sessions_path: &Path,
    native_session_path: &Path,
    session_id: &str,
    workspace_path: &str,
) -> bool {
    let Ok(session_path) = fs::canonicalize(native_session_path) else {
        return false;
    };
    if !session_path.starts_with(sessions_path) {
        return false;
    }
    match read_codex_session_meta(&session_path) {
        Ok(Some(meta)) => meta.session_id == session_id && meta.cwd == workspace_path,
        _ => false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedCodexSession {
    pub home_path: PathBuf,
    pub native_session_path: PathBuf,
}

/// Copy a user-selected native Codex session into the binding-owned session
/// store. Imports already identify one exact native session, unlike the
/// unsafe "newest session for this cwd" fallback used nowhere for Codex.
#[cfg(unix)]
pub fn import_codex_session(
    binding_id: &str,
    workspace_path: &str,
    native_session_id: &str,
    account_home: Option<&Path>,
) -> Result<ImportedCodexSession, AppError> {
    let source_home = account_home
        .map(Path::to_path_buf)
        .or_else(normal_codex_home)
        .ok_or_else(|| {
            AppError::NotFound("Codex home is unavailable for the selected session".into())
        })?;
    let source_sessions = fs::canonicalize(source_home.join("sessions"))
        .map_err(|error| redact_local_path_error("canonicalize imported Codex sessions", error))?;
    let candidates = collect_codex_session_files(&source_sessions)?
        .into_iter()
        .filter_map(|path| {
            let path = PathBuf::from(path);
            match read_codex_session_meta(&path) {
                Ok(Some(meta))
                    if meta.session_id == native_session_id && meta.cwd == workspace_path =>
                {
                    Some(Ok(meta))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let [source] = candidates.as_slice() else {
        return Err(AppError::NotFound(
            "the selected Codex session has no unique matching native history file".into(),
        ));
    };

    let relative_path = source.path.strip_prefix(&source_sessions).map_err(|_| {
        AppError::Internal("selected Codex session escaped its native sessions directory".into())
    })?;
    let managed = provision_managed_codex_home(binding_id, workspace_path, Some(&source_home))?;
    let destination = managed.sessions_path.join(relative_path);
    let parent = destination.parent().ok_or_else(|| {
        AppError::Internal("selected Codex session has no parent directory".into())
    })?;
    ensure_local_dir_not_symlink(parent)?;
    fs::copy(&source.path, &destination)
        .map_err(|error| redact_local_path_error("copy imported Codex session", error))?;
    let destination = fs::canonicalize(&destination)
        .map_err(|error| redact_local_path_error("canonicalize imported Codex copy", error))?;
    if !destination.starts_with(&managed.sessions_path) {
        return Err(AppError::Internal(
            "imported Codex session escaped its binding-owned store".into(),
        ));
    }
    let copied = read_codex_session_meta(&destination)?
        .ok_or_else(|| AppError::Internal("copied Codex session lost its metadata".into()))?;
    if copied.session_id != native_session_id || copied.cwd != workspace_path {
        return Err(AppError::Internal(
            "copied Codex session no longer matches the selected session".into(),
        ));
    }

    Ok(ImportedCodexSession {
        home_path: managed.home_path,
        native_session_path: destination,
    })
}

#[cfg(not(unix))]
pub fn import_codex_session(
    _binding_id: &str,
    _workspace_path: &str,
    _native_session_id: &str,
    _account_home: Option<&Path>,
) -> Result<ImportedCodexSession, AppError> {
    Err(AppError::Internal(
        "managed Codex homes require a Unix filesystem".into(),
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn isolated_test_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("choruz-{name}-{unique}"));
        fs::create_dir_all(&dir).expect("create isolated test dir");
        dir
    }

    /// Tests that set CHORUZ_RUNTIME_DIR or CODEX_HOME run one at a time.
    fn with_env_lock<T>(f: impl FnOnce() -> T) -> T {
        let _guard = crate::TEST_ENV_LOCK.blocking_lock();
        f()
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &Path) -> Self {
            let guard = Self {
                key,
                previous: std::env::var_os(key),
            };
            unsafe {
                std::env::set_var(key, value);
            }
            guard
        }

        fn remove(key: &'static str) -> Self {
            let guard = Self {
                key,
                previous: std::env::var_os(key),
            };
            unsafe {
                std::env::remove_var(key);
            }
            guard
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(previous) = &self.previous {
                    std::env::set_var(self.key, previous);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    pub(crate) fn write_codex_session_meta(path: &Path, session_id: &str, cwd: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create session parent");
        }
        fs::write(
            path,
            format!(r#"{{"type":"session_meta","payload":{{"id":"{session_id}","cwd":"{cwd}"}}}}"#)
                + "\n",
        )
        .expect("write session meta");
    }

    #[test]
    fn managed_codex_home_uses_local_sessions_and_shared_safe_links() {
        with_env_lock(|| {
            let root = isolated_test_dir("codex-managed-home");
            let runtime_dir = root.join("runtime");
            let normal_home = root.join("normal-codex");
            fs::create_dir_all(normal_home.join("plugins")).expect("create plugins");
            fs::create_dir_all(normal_home.join("skills")).expect("create skills");
            fs::create_dir_all(normal_home.join("cache")).expect("create cache");
            fs::write(normal_home.join("config.toml"), "model = \"gpt-5\"\n")
                .expect("write config");
            fs::write(normal_home.join("auth.json"), "{}\n").expect("write auth");
            let workspace = root.join("workspace");
            fs::create_dir_all(&workspace).expect("create workspace");

            let _runtime_env = EnvVarGuard::set_path("CHORUZ_RUNTIME_DIR", &runtime_dir);
            let _codex_home_env = EnvVarGuard::set_path("CODEX_HOME", &normal_home);

            let managed =
                provision_managed_codex_home("binding-1", workspace.to_str().unwrap(), None)
                    .expect("provision managed home");

            assert!(
                managed
                    .home_path
                    .starts_with(runtime_dir.canonicalize().unwrap())
            );
            assert!(managed.sessions_path.starts_with(&managed.home_path));
            assert!(managed.sessions_path.is_dir());
            assert!(
                !managed
                    .home_path
                    .starts_with(workspace.canonicalize().unwrap())
            );

            for entry in ["config.toml", "auth.json", "plugins", "skills", "cache"] {
                let target = managed.home_path.join(entry);
                let metadata = fs::symlink_metadata(&target).expect("shared entry exists");
                assert!(
                    metadata.file_type().is_symlink(),
                    "{entry} should be a symlink"
                );
                assert_eq!(
                    fs::canonicalize(&target).unwrap(),
                    fs::canonicalize(normal_home.join(entry)).unwrap()
                );
            }

            assert_eq!(
                fs::metadata(&managed.home_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&managed.sessions_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        });
    }

    #[test]
    fn imported_codex_session_is_copied_into_the_binding_owned_store() {
        with_env_lock(|| {
            let root = isolated_test_dir("imported-codex-terminal-session");
            let runtime_dir = root.join("runtime");
            let source_home = root.join("source-codex");
            let workspace = root.join("workspace");
            fs::create_dir_all(&workspace).expect("create workspace");
            let source_session = source_home.join("sessions/2026/09/03/imported.jsonl");
            write_codex_session_meta(
                &source_session,
                "imported-session",
                workspace.to_str().unwrap(),
            );

            let _runtime_env = EnvVarGuard::set_path("CHORUZ_RUNTIME_DIR", &runtime_dir);
            let _codex_home_env = EnvVarGuard::set_path("CODEX_HOME", &source_home);
            let imported = import_codex_session(
                "binding-1",
                workspace.to_str().unwrap(),
                "imported-session",
                None,
            )
            .expect("register imported Codex session");

            assert!(
                imported
                    .native_session_path
                    .starts_with(&imported.home_path)
            );
            assert_eq!(
                fs::read_to_string(&imported.native_session_path).unwrap(),
                fs::read_to_string(source_session).unwrap()
            );
            assert!(codex_anchor_file_matches(
                &imported.home_path.join("sessions"),
                &imported.native_session_path,
                "imported-session",
                workspace.to_str().unwrap(),
            ));
            assert!(!codex_anchor_file_matches(
                &imported.home_path.join("sessions"),
                &imported.native_session_path,
                "imported-session",
                "/elsewhere",
            ));
        });
    }

    #[test]
    fn codex_session_baseline_ignores_symlinks_and_non_jsonl_files() {
        let root = isolated_test_dir("codex-session-baseline");
        let sessions = root.join("sessions");
        let day = sessions.join("2026").join("05").join("29");
        fs::create_dir_all(&day).expect("create sessions day");
        let real = day.join("rollout-1.jsonl");
        fs::write(&real, "{}\n").expect("write jsonl");
        fs::write(day.join("notes.txt"), "ignore").expect("write ignored");
        std::os::unix::fs::symlink(&real, day.join("linked.jsonl")).expect("link jsonl");

        let files = collect_codex_session_files(&sessions).expect("collect files");

        assert_eq!(files.len(), 1);
        assert!(
            files.contains(
                &fs::canonicalize(real)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            )
        );
    }

    #[test]
    fn managed_codex_home_rejects_symlinked_binding_home() {
        with_env_lock(|| {
            let root = isolated_test_dir("codex-managed-home-symlink");
            let runtime_dir = root.join("runtime");
            let homes = runtime_dir.join("codex-homes");
            let target = root.join("redirected");
            fs::create_dir_all(&homes).expect("create homes");
            fs::create_dir_all(&target).expect("create target");
            std::os::unix::fs::symlink(&target, homes.join("binding-1"))
                .expect("create binding symlink");
            let workspace = root.join("workspace");
            fs::create_dir_all(&workspace).expect("create workspace");

            let _runtime_env = EnvVarGuard::set_path("CHORUZ_RUNTIME_DIR", &runtime_dir);
            let _codex_home_env = EnvVarGuard::remove("CODEX_HOME");

            let result =
                provision_managed_codex_home("binding-1", workspace.to_str().unwrap(), None);
            assert!(
                result.is_err(),
                "managed home must reject pre-existing binding symlinks"
            );
        });
    }

    #[test]
    fn new_codex_session_requires_unique_matching_workspace() {
        let root = isolated_test_dir("codex-session-candidate");
        let home = root.join("home");
        let sessions = home.join("sessions");
        fs::create_dir_all(&sessions).expect("create sessions");
        let old_file = sessions.join("2026/05/28/old.jsonl");
        write_codex_session_meta(&old_file, "old-session", "/workspace");
        let baseline = vec![
            fs::canonicalize(&old_file)
                .unwrap()
                .to_string_lossy()
                .to_string(),
        ];

        let new_file = sessions.join("2026/05/29/new.jsonl");
        write_codex_session_meta(&new_file, "new-session", "/workspace");

        let candidate = unique_new_codex_session(&home, &sessions, &baseline, "/workspace")
            .expect("candidate scan")
            .expect("unique candidate");
        assert_eq!(candidate.session_id, "new-session");
        assert_eq!(candidate.path, fs::canonicalize(&new_file).unwrap());

        let wrong_workspace = sessions.join("2026/05/29/wrong.jsonl");
        write_codex_session_meta(&wrong_workspace, "wrong-session", "/other");
        let candidate = unique_new_codex_session(&home, &sessions, &baseline, "/workspace")
            .expect("candidate scan")
            .expect("wrong workspace is ignored");
        assert_eq!(candidate.session_id, "new-session");

        let second = sessions.join("2026/05/29/second.jsonl");
        write_codex_session_meta(&second, "second-session", "/workspace");
        assert!(
            unique_new_codex_session(&home, &sessions, &baseline, "/workspace")
                .expect("candidate scan")
                .is_none(),
            "multiple matching Codex session files must fail closed"
        );
    }
}
