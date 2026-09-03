//! Read-only discovery of native coding-harness sessions for one workspace.
//!
//! The native stores remain the source of truth. This module returns metadata
//! that a human can review before importing a session into Choruz; discovery
//! never starts, resumes, edits, archives, or deletes a harness session.

use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::SystemTime,
};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_METADATA_LINE_BYTES: usize = 1024 * 1024;
const OPENCODE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessKind {
    Claude,
    Codex,
    Pi,
    Grok,
    OpenCode,
}

impl HarnessKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Pi => "Pi",
            Self::Grok => "Grok",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeSessionSummary {
    pub harness: HarnessKind,
    pub native_session_id: String,
    pub title: String,
    pub workspace_path: String,
    pub updated_at: DateTime<Utc>,
    pub model: Option<String>,
    pub branch: Option<String>,
    pub archived: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionScanResult {
    pub workspace_path: String,
    pub sessions: Vec<NativeSessionSummary>,
    /// A failed harness must not make successful harness results disappear.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SessionCatalogScanner {
    home: PathBuf,
    codex_home: PathBuf,
    pi_sessions_root: PathBuf,
    grok_sessions_root: PathBuf,
    opencode_binary: PathBuf,
}

impl SessionCatalogScanner {
    pub fn from_env() -> Result<Self, String> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not configured".to_owned())?;
        let codex_home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        let pi_sessions_root = std::env::var_os("PI_CODING_AGENT_SESSION_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".pi/agent/sessions"));
        let grok_home = std::env::var_os("GROK_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".grok"));
        let opencode_binary = std::env::var_os("CHORUZ_OPENCODE_BINARY")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("opencode"));
        Ok(Self {
            home,
            codex_home,
            pi_sessions_root,
            grok_sessions_root: grok_home.join("sessions"),
            opencode_binary,
        })
    }

    #[cfg(test)]
    fn for_test(home: PathBuf) -> Self {
        Self {
            codex_home: home.join(".codex"),
            pi_sessions_root: home.join(".pi/agent/sessions"),
            grok_sessions_root: home.join(".grok/sessions"),
            opencode_binary: home.join("missing-opencode"),
            home,
        }
    }

    pub async fn scan(
        &self,
        workspace_path: &Path,
        harnesses: &BTreeSet<HarnessKind>,
    ) -> Result<SessionScanResult, String> {
        let workspace = canonical_workspace(workspace_path)?;
        let mut sessions = Vec::new();
        let mut warnings = Vec::new();

        for harness in harnesses {
            let result = match *harness {
                HarnessKind::OpenCode => self.scan_opencode(&workspace).await,
                blocking_harness => {
                    let scanner = self.clone();
                    let workspace = workspace.clone();
                    tokio::task::spawn_blocking(move || match blocking_harness {
                        HarnessKind::Claude => scanner.scan_claude(&workspace),
                        HarnessKind::Codex => scanner.scan_codex(&workspace),
                        HarnessKind::Pi => scanner.scan_pi(&workspace),
                        HarnessKind::Grok => scanner.scan_grok(&workspace),
                        HarnessKind::OpenCode => unreachable!("handled above"),
                    })
                    .await
                    .map_err(|error| format!("session scanner stopped unexpectedly: {error}"))?
                }
            };
            match result {
                Ok(mut found) => sessions.append(&mut found),
                Err(error) => warnings.push(format!("{}: {error}", harness.label())),
            }
        }

        sessions.sort_by(|left, right| {
            left.harness
                .cmp(&right.harness)
                .then_with(|| left.native_session_id.cmp(&right.native_session_id))
                .then_with(|| left.workspace_path.cmp(&right.workspace_path))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        sessions.dedup_by(|left, right| {
            left.harness == right.harness
                && left.native_session_id == right.native_session_id
                && left.workspace_path == right.workspace_path
        });
        sort_sessions_newest_first(&mut sessions);

        Ok(SessionScanResult {
            workspace_path: workspace.to_string_lossy().into_owned(),
            sessions,
            warnings,
        })
    }

    fn scan_claude(&self, workspace: &Path) -> Result<Vec<NativeSessionSummary>, String> {
        let projects_root = self.home.join(".claude/projects");
        let Ok(projects) = fs::read_dir(&projects_root) else {
            return Ok(Vec::new());
        };
        let mut sessions = Vec::new();
        for project in projects.flatten().filter(|entry| entry.path().is_dir()) {
            let Ok(entries) = fs::read_dir(project.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                    continue;
                }
                let id = match path.file_stem().and_then(|value| value.to_str()) {
                    Some(value) if !value.is_empty() => value.to_owned(),
                    _ => continue,
                };
                let Some(metadata) = first_matching_json_line(&path, |value| {
                    value.get("cwd").and_then(Value::as_str).is_some()
                }) else {
                    continue;
                };
                let Some(cwd) = metadata.get("cwd").and_then(Value::as_str) else {
                    continue;
                };
                let Some(session_workspace) = workspace_descendant(Path::new(cwd), workspace)
                else {
                    continue;
                };
                sessions.push(NativeSessionSummary {
                    harness: HarnessKind::Claude,
                    native_session_id: id.clone(),
                    title: metadata
                        .get("customTitle")
                        .or_else(|| metadata.get("name"))
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| fallback_title(HarnessKind::Claude, &id)),
                    workspace_path: session_workspace.to_string_lossy().into_owned(),
                    updated_at: modified_at(&path),
                    model: metadata
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    branch: metadata
                        .get("gitBranch")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    archived: false,
                });
            }
        }
        Ok(sessions)
    }

    fn scan_codex(&self, workspace: &Path) -> Result<Vec<NativeSessionSummary>, String> {
        let candidates = [
            self.codex_home.join("sqlite/state_5.sqlite"),
            self.codex_home.join("state_5.sqlite"),
        ];
        let mut sessions = Vec::new();
        for database in candidates.iter().filter(|path| path.is_file()) {
            let connection = Connection::open_with_flags(
                database,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(|error| format!("cannot read Codex session index: {error}"))?;
            let mut statement = connection
                .prepare(
                    "SELECT id, title, cwd, updated_at, model_provider, git_branch, archived, model
                     FROM threads
                     ORDER BY updated_at DESC, id DESC",
                )
                .map_err(|error| format!("unsupported Codex session index: {error}"))?;
            let rows = statement
                .query_map([], |row| {
                    let updated_at: i64 = row.get(3)?;
                    let provider: String = row.get(4)?;
                    let model: Option<String> = row.get(7)?;
                    Ok(NativeSessionSummary {
                        harness: HarnessKind::Codex,
                        native_session_id: row.get(0)?,
                        title: row.get(1)?,
                        workspace_path: row.get(2)?,
                        updated_at: unix_time(updated_at),
                        model: model.or_else(|| (!provider.is_empty()).then_some(provider)),
                        branch: row.get(5)?,
                        archived: row.get::<_, i64>(6)? != 0,
                    })
                })
                .map_err(|error| format!("cannot query Codex sessions: {error}"))?;
            for row in rows {
                let mut session =
                    row.map_err(|error| format!("invalid Codex session row: {error}"))?;
                let Some(session_workspace) =
                    workspace_descendant(Path::new(&session.workspace_path), workspace)
                else {
                    continue;
                };
                session.workspace_path = session_workspace.to_string_lossy().into_owned();
                if session.title.trim().is_empty() {
                    session.title = fallback_title(HarnessKind::Codex, &session.native_session_id);
                }
                sessions.push(session);
            }
        }
        sessions.sort_by(|left, right| {
            left.native_session_id
                .cmp(&right.native_session_id)
                .then_with(|| left.workspace_path.cmp(&right.workspace_path))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        sessions.dedup_by(|left, right| {
            left.native_session_id == right.native_session_id
                && left.workspace_path == right.workspace_path
        });
        Ok(sessions)
    }

    fn scan_pi(&self, workspace: &Path) -> Result<Vec<NativeSessionSummary>, String> {
        let Ok(projects) = fs::read_dir(&self.pi_sessions_root) else {
            return Ok(Vec::new());
        };
        let mut sessions = Vec::new();
        for project in projects.flatten().filter(|entry| entry.path().is_dir()) {
            let Ok(entries) = fs::read_dir(project.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                    continue;
                }
                let Some(header) = first_json_line(&path) else {
                    continue;
                };
                if header.get("type").and_then(Value::as_str) != Some("session") {
                    continue;
                }
                let Some(cwd) = header.get("cwd").and_then(Value::as_str) else {
                    continue;
                };
                let Some(session_workspace) = workspace_descendant(Path::new(cwd), workspace)
                else {
                    continue;
                };
                let Some(id) = header.get("id").and_then(Value::as_str) else {
                    continue;
                };
                sessions.push(NativeSessionSummary {
                    harness: HarnessKind::Pi,
                    native_session_id: id.to_owned(),
                    title: header
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| fallback_title(HarnessKind::Pi, id)),
                    workspace_path: session_workspace.to_string_lossy().into_owned(),
                    updated_at: modified_at(&path),
                    model: header
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    branch: None,
                    archived: false,
                });
            }
        }
        Ok(sessions)
    }

    fn scan_grok(&self, workspace: &Path) -> Result<Vec<NativeSessionSummary>, String> {
        let Ok(cwd_entries) = fs::read_dir(&self.grok_sessions_root) else {
            return Ok(Vec::new());
        };
        let mut sessions = Vec::new();
        for cwd_entry in cwd_entries.flatten().filter(|entry| entry.path().is_dir()) {
            let Ok(entries) = fs::read_dir(cwd_entry.path()) else {
                continue;
            };
            for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
                let summary_path = entry.path().join("summary.json");
                let Ok(bytes) = fs::read(&summary_path) else {
                    continue;
                };
                let Ok(summary) = serde_json::from_slice::<Value>(&bytes) else {
                    continue;
                };
                let info = summary.get("info").unwrap_or(&summary);
                let Some(cwd) = info.get("cwd").and_then(Value::as_str) else {
                    continue;
                };
                let Some(session_workspace) = workspace_descendant(Path::new(cwd), workspace)
                else {
                    continue;
                };
                let Some(id) = info.get("id").and_then(Value::as_str) else {
                    continue;
                };
                sessions.push(NativeSessionSummary {
                    harness: HarnessKind::Grok,
                    native_session_id: id.to_owned(),
                    title: info
                        .get("title")
                        .or_else(|| info.get("name"))
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| fallback_title(HarnessKind::Grok, id)),
                    workspace_path: session_workspace.to_string_lossy().into_owned(),
                    updated_at: modified_at(&summary_path),
                    model: info.get("model").and_then(Value::as_str).map(str::to_owned),
                    branch: info
                        .get("git_branch")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    archived: false,
                });
            }
        }
        Ok(sessions)
    }

    async fn scan_opencode(&self, workspace: &Path) -> Result<Vec<NativeSessionSummary>, String> {
        let output = tokio::time::timeout(
            OPENCODE_TIMEOUT,
            tokio::process::Command::new(&self.opencode_binary)
                .args(["session", "list", "--format", "json"])
                .current_dir(workspace)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| "session listing timed out".to_owned())?
        .map_err(|error| format!("cannot start OpenCode: {error}"))?;
        if !output.status.success() {
            return Err("OpenCode session listing failed".to_owned());
        }
        let values = serde_json::from_slice::<Vec<Value>>(&output.stdout)
            .map_err(|error| format!("invalid OpenCode session list: {error}"))?;
        let mut sessions = Vec::new();
        for value in values {
            let Some(directory) = value.get("directory").and_then(Value::as_str) else {
                continue;
            };
            let Some(session_workspace) = workspace_descendant(Path::new(directory), workspace)
            else {
                continue;
            };
            let Some(id) = value.get("id").and_then(Value::as_str) else {
                continue;
            };
            let updated = value
                .get("updated")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            sessions.push(NativeSessionSummary {
                harness: HarnessKind::OpenCode,
                native_session_id: id.to_owned(),
                title: value
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|title| !title.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| fallback_title(HarnessKind::OpenCode, id)),
                workspace_path: session_workspace.to_string_lossy().into_owned(),
                updated_at: unix_time_millis_or_seconds(updated),
                model: value
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                branch: None,
                archived: false,
            });
        }
        Ok(sessions)
    }
}

fn sort_sessions_newest_first(sessions: &mut [NativeSessionSummary]) {
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.harness.cmp(&right.harness))
            .then_with(|| left.native_session_id.cmp(&right.native_session_id))
    });
}

fn canonical_workspace(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("workspace path must be absolute".to_owned());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("workspace is not accessible: {error}"))?;
    if !canonical.is_dir() {
        return Err("workspace path must be a directory".to_owned());
    }
    Ok(canonical)
}

fn workspace_descendant(candidate: &Path, workspace_root: &Path) -> Option<PathBuf> {
    let candidate = candidate.canonicalize().ok()?;
    candidate.starts_with(workspace_root).then_some(candidate)
}

fn first_json_line(path: &Path) -> Option<Value> {
    first_matching_json_line(path, |_| true)
}

fn first_matching_json_line(path: &Path, predicate: impl Fn(&Value) -> bool) -> Option<Value> {
    let file = fs::File::open(path).ok()?;
    for line in BufReader::new(file).lines().take(64).flatten() {
        if line.len() > MAX_METADATA_LINE_BYTES {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if predicate(&value) {
            return Some(value);
        }
    }
    None
}

fn modified_at(path: &Path) -> DateTime<Utc> {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    DateTime::<Utc>::from(modified)
}

fn unix_time(value: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(value, 0)
        .single()
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

fn unix_time_millis_or_seconds(value: i64) -> DateTime<Utc> {
    if value.abs() >= 10_000_000_000 {
        Utc.timestamp_millis_opt(value)
            .single()
            .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
    } else {
        unix_time(value)
    }
}

fn fallback_title(harness: HarnessKind, id: &str) -> String {
    let short = id.chars().take(8).collect::<String>();
    format!("{} session {short}", harness.label())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs};

    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "choruz-session-catalog-{label}-{}",
            uuid::Uuid::now_v7()
        ));
        fs::create_dir_all(&path).expect("create temp directory");
        path
    }

    #[test]
    fn sessions_are_globally_sorted_newest_first_across_harnesses() {
        let session = |harness, id: &str, timestamp| NativeSessionSummary {
            harness,
            native_session_id: id.to_owned(),
            title: id.to_owned(),
            workspace_path: format!("/projects/{id}"),
            updated_at: Utc.timestamp_opt(timestamp, 0).single().unwrap(),
            model: None,
            branch: None,
            archived: false,
        };
        let mut sessions = vec![
            session(HarnessKind::Claude, "old-claude", 100),
            session(HarnessKind::OpenCode, "new-opencode", 300),
            session(HarnessKind::Codex, "middle-codex", 200),
        ];

        sort_sessions_newest_first(&mut sessions);

        assert_eq!(
            sessions
                .iter()
                .map(|session| session.native_session_id.as_str())
                .collect::<Vec<_>>(),
            ["new-opencode", "middle-codex", "old-claude"]
        );
    }

    #[tokio::test]
    async fn claude_scan_returns_sessions_from_nested_workspaces() {
        let home = temp_dir("claude");
        let workspace_root = home.join("projects");
        let workspace = workspace_root.join("repo/nested");
        let sibling = home.join("projects-other");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let sibling = sibling.canonicalize().unwrap();
        let project_root = |workspace: &Path| {
            let mangled: String = workspace
                .to_string_lossy()
                .chars()
                .map(|character| {
                    if matches!(character, '/' | '.' | '_') {
                        '-'
                    } else {
                        character
                    }
                })
                .collect();
            home.join(".claude/projects").join(mangled)
        };
        let root = project_root(&workspace);
        fs::create_dir_all(&root).unwrap();
        for id in ["session-a", "session-b"] {
            fs::write(
                root.join(format!("{id}.jsonl")),
                format!(
                    "{{\"type\":\"system\",\"cwd\":{},\"customTitle\":\"{id}\"}}\n",
                    serde_json::to_string(&workspace.to_string_lossy()).unwrap()
                ),
            )
            .unwrap();
        }
        let sibling_root = project_root(&sibling);
        fs::create_dir_all(&sibling_root).unwrap();
        fs::write(
            sibling_root.join("sibling.jsonl"),
            format!(
                "{{\"type\":\"system\",\"cwd\":{},\"customTitle\":\"sibling\"}}\n",
                serde_json::to_string(&sibling.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();

        let result = SessionCatalogScanner::for_test(home.clone())
            .scan(&workspace_root, &BTreeSet::from([HarnessKind::Claude]))
            .await
            .unwrap();

        assert_eq!(result.sessions.len(), 2);
        assert!(
            result
                .sessions
                .iter()
                .all(|session| session.workspace_path == workspace.to_string_lossy())
        );
        assert_eq!(result.sessions[0].harness, HarnessKind::Claude);
        assert!(result.warnings.is_empty());
        fs::remove_dir_all(home).ok();
    }

    #[tokio::test]
    async fn pi_scan_includes_descendants_and_rejects_prefix_siblings() {
        let home = temp_dir("pi");
        let workspace_root = home.join("projects");
        let workspace = workspace_root.join("repo-a");
        let second_workspace = workspace_root.join("repo-b");
        let other = home.join("projects-other");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&second_workspace).unwrap();
        fs::create_dir_all(&other).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let second_workspace = second_workspace.canonicalize().unwrap();
        let root = home.join(".pi/agent/sessions/project");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("matching.jsonl"),
            format!(
                "{{\"type\":\"session\",\"id\":\"matching\",\"cwd\":{}}}\n",
                serde_json::to_string(&workspace.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        fs::write(
            root.join("same-id-different-workspace.jsonl"),
            format!(
                "{{\"type\":\"session\",\"id\":\"matching\",\"cwd\":{}}}\n",
                serde_json::to_string(&second_workspace.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        fs::write(
            root.join("foreign.jsonl"),
            format!(
                "{{\"type\":\"session\",\"id\":\"foreign\",\"cwd\":{}}}\n",
                serde_json::to_string(&other.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();

        let result = SessionCatalogScanner::for_test(home.clone())
            .scan(&workspace_root, &BTreeSet::from([HarnessKind::Pi]))
            .await
            .unwrap();

        assert_eq!(result.sessions.len(), 2);
        assert!(
            result
                .sessions
                .iter()
                .all(|session| session.native_session_id == "matching")
        );
        assert_eq!(
            result
                .sessions
                .iter()
                .map(|session| session.workspace_path.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                workspace.to_string_lossy().into_owned(),
                second_workspace.to_string_lossy().into_owned(),
            ])
        );
        fs::remove_dir_all(home).ok();
    }

    #[tokio::test]
    async fn codex_scan_reads_threads_from_nested_workspaces() {
        let home = temp_dir("codex");
        let workspace_root = home.join("projects");
        let workspace = workspace_root.join("repo");
        let sibling = home.join("projects-other");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        let database = home.join(".codex/state_5.sqlite");
        let connection = Connection::open(database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (
               id TEXT PRIMARY KEY, title TEXT NOT NULL, cwd TEXT NOT NULL,
               updated_at INTEGER NOT NULL, model_provider TEXT NOT NULL,
               git_branch TEXT, archived INTEGER NOT NULL, model TEXT
             );",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    "thread-1",
                    "Investigate cache",
                    workspace.to_string_lossy(),
                    1_700_000_000_i64,
                    "openai",
                    "main",
                    0_i64,
                    "gpt-5"
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, NULL)",
                rusqlite::params![
                    "thread-2",
                    "Review patch",
                    workspace.to_string_lossy(),
                    1_700_000_100_i64,
                    "openai",
                    1_i64
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES ('thread-sibling', 'Sibling', ?1, 1700000300, 'openai', NULL, 0, NULL)",
                [sibling.to_string_lossy().as_ref()],
            )
            .unwrap();
        fs::create_dir_all(home.join(".codex/sqlite")).unwrap();
        let current = Connection::open(home.join(".codex/sqlite/state_5.sqlite")).unwrap();
        current
            .execute_batch(
                "CREATE TABLE threads (
                   id TEXT PRIMARY KEY, title TEXT NOT NULL, cwd TEXT NOT NULL,
                   updated_at INTEGER NOT NULL, model_provider TEXT NOT NULL,
                   git_branch TEXT, archived INTEGER NOT NULL, model TEXT
                 );",
            )
            .unwrap();
        current
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3, ?4, ?5, NULL, 0, NULL)",
                rusqlite::params![
                    "thread-current",
                    "Current index",
                    workspace.to_string_lossy(),
                    1_700_000_200_i64,
                    "openai"
                ],
            )
            .unwrap();

        let result = SessionCatalogScanner::for_test(home.clone())
            .scan(&workspace_root, &BTreeSet::from([HarnessKind::Codex]))
            .await
            .unwrap();

        assert_eq!(result.sessions.len(), 3);
        assert!(
            result
                .sessions
                .iter()
                .all(|session| session.workspace_path == workspace.to_string_lossy())
        );
        assert_eq!(result.sessions[0].native_session_id, "thread-current");
        assert!(result.sessions[1].archived);
        assert_eq!(result.sessions[2].model.as_deref(), Some("gpt-5"));
        fs::remove_dir_all(home).ok();
    }

    #[tokio::test]
    async fn grok_scan_reads_nested_workspace_metadata() {
        let home = temp_dir("grok");
        let workspace_root = home.join("projects");
        let workspace = workspace_root.join("repo");
        let sibling = home.join("projects-other");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let session_dir = home.join(".grok/sessions/project/session-1");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("summary.json"),
            serde_json::to_vec(&serde_json::json!({
                "info": {
                    "id": "grok-session-1",
                    "cwd": workspace,
                    "title": "Trace the gateway",
                    "model": "grok-code-fast",
                    "git_branch": "feature/remote"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let sibling_dir = home.join(".grok/sessions/project/session-sibling");
        fs::create_dir_all(&sibling_dir).unwrap();
        fs::write(
            sibling_dir.join("summary.json"),
            serde_json::to_vec(&serde_json::json!({
                "info": { "id": "grok-sibling", "cwd": sibling, "title": "Sibling" }
            }))
            .unwrap(),
        )
        .unwrap();

        let result = SessionCatalogScanner::for_test(home.clone())
            .scan(&workspace_root, &BTreeSet::from([HarnessKind::Grok]))
            .await
            .unwrap();

        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].title, "Trace the gateway");
        assert_eq!(
            result.sessions[0].workspace_path,
            workspace.to_string_lossy()
        );
        assert_eq!(result.sessions[0].branch.as_deref(), Some("feature/remote"));
        fs::remove_dir_all(home).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn opencode_scan_includes_descendants_and_rejects_prefix_siblings() {
        use std::os::unix::fs::PermissionsExt;

        let home = temp_dir("opencode");
        let workspace_root = home.join("projects");
        let workspace = workspace_root.join("repo/nested");
        let other = home.join("projects-other");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&other).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let binary = home.join("opencode-fixture");
        let output = serde_json::to_string(&serde_json::json!([
            {"id":"session-match","title":"Matching","updated":1700000000000_i64,"directory":workspace},
            {"id":"session-other","title":"Other","updated":1700000000001_i64,"directory":other}
        ])).unwrap();
        fs::write(
            &binary,
            format!("#!/bin/sh\nprintf '%s' {}\n", shell_quote(&output)),
        )
        .unwrap();
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();
        let mut scanner = SessionCatalogScanner::for_test(home.clone());
        scanner.opencode_binary = binary;

        let result = scanner
            .scan(&workspace_root, &BTreeSet::from([HarnessKind::OpenCode]))
            .await
            .unwrap();

        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].native_session_id, "session-match");
        assert_eq!(
            result.sessions[0].workspace_path,
            workspace.to_string_lossy()
        );
        fs::remove_dir_all(home).ok();
    }

    #[test]
    fn descendant_matching_is_component_aware() {
        let home = temp_dir("components");
        let root = home.join("projects");
        let nested = root.join("repo/nested");
        let prefix_sibling = home.join("projects-other");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&prefix_sibling).unwrap();
        let root = root.canonicalize().unwrap();

        assert_eq!(workspace_descendant(&root, &root), Some(root.clone()));
        assert_eq!(
            workspace_descendant(&nested, &root),
            Some(nested.canonicalize().unwrap())
        );
        assert_eq!(workspace_descendant(&prefix_sibling, &root), None);
        fs::remove_dir_all(home).ok();
    }

    #[cfg(unix)]
    fn shell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    #[tokio::test]
    async fn inaccessible_harness_is_a_warning_not_a_total_failure() {
        let home = temp_dir("warning");
        let workspace = home.join("repo");
        fs::create_dir_all(&workspace).unwrap();

        let result = SessionCatalogScanner::for_test(home.clone())
            .scan(
                &workspace,
                &BTreeSet::from([HarnessKind::Claude, HarnessKind::OpenCode]),
            )
            .await
            .unwrap();

        assert!(result.sessions.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].starts_with("OpenCode:"));
        fs::remove_dir_all(home).ok();
    }
}
