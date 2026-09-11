//! Read native transcripts in resumable windows; never start a Harness to inspect history.
use crate::TerminalSpec;
use choruz_agent_runtime::headless::{HeadlessDriver, harness_account_env};
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_WINDOW_BYTES: usize = 160 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cursor {
    pub session: String,
    pub offset: u64,
}

/// A projected record recovered from the selected append-only native source.
/// Offsets are comparable only within that source's session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalRecord {
    pub reference: String,
    pub session: String,
    pub offset: u64,
    pub record: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Window {
    pub records: Vec<Value>,
    pub cursor: Cursor,
    pub more: bool,
    pub reset: bool,
    pub project_guidance: Vec<Value>,
}

pub fn read(spec: &TerminalSpec, previous: Cursor) -> Result<Window, AppError> {
    let live = crate::session::recorded_snapshot(spec);
    if let Ok(state) = &live
        && matches!(state.status.as_str(), "running" | "connecting" | "waiting")
    {
        return Err(AppError::Conflict(
            "Learning waits for the current turn to finish".into(),
        ));
    }
    let id = spec
        .resume_session_id
        .as_deref()
        .or_else(|| {
            live.as_ref()
                .ok()
                .and_then(|state| state.session_id.as_deref())
        })
        .ok_or_else(|| AppError::NotFound("The Agent has no recorded native session yet".into()))?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(AppError::Validation(
            "Invalid native session identifier".into(),
        ));
    }
    let path = source_path(spec, id)?;
    let mut window = read_window(&path, id, previous, spec.driver_type == "claude_terminal")?;
    let root = Path::new(&spec.workspace_path)
        .canonicalize()
        .map_err(io_error)?;
    for name in ["CLAUDE.md", "AGENTS.md"] {
        let path = root.join(name);
        if !path.exists() {
            continue;
        }
        let path = path.canonicalize().map_err(io_error)?;
        if !path.starts_with(&root) {
            return Err(AppError::Forbidden(
                "Project guidance points outside this workspace".into(),
            ));
        }
        let mut text = String::new();
        File::open(path)
            .map_err(io_error)?
            .take(32 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(io_error)?;
        if text.len() > 32 * 1024 {
            return Err(AppError::Validation(
                "Project guidance exceeds the analysis context limit".into(),
            ));
        }
        window
            .project_guidance
            .push(json!({"path":name,"content":text}));
    }
    Ok(window)
}

fn source_path(spec: &TerminalSpec, id: &str) -> Result<PathBuf, AppError> {
    let workspace = std::fs::canonicalize(&spec.workspace_path).map_err(io_error)?;
    if spec.driver_type == "claude_terminal" {
        let root = crate::session_history::claude_account_root(spec)?.join("projects");
        let root = root.canonicalize().map_err(io_error)?;
        let mut matches = Vec::new();
        for entry in std::fs::read_dir(&root).map_err(io_error)? {
            let path = entry.map_err(io_error)?.path().join(format!("{id}.jsonl"));
            if let Ok(path) = path.canonicalize()
                && path.starts_with(&root)
                && path.is_file()
            {
                matches.push(path);
            }
        }
        if matches.len() != 1 {
            return Err(AppError::NotFound(
                "Selected native session is missing or ambiguous".into(),
            ));
        }
        let path = matches.remove(0);
        let mut reader = BufReader::new(File::open(&path).map_err(io_error)?);
        while let Some((entry, _)) = entry(&mut reader)? {
            if let Some(cwd) = entry["cwd"].as_str() {
                if Path::new(cwd).canonicalize().ok().as_ref() != Some(&workspace) {
                    return Err(AppError::Forbidden(
                        "Native session belongs to another workspace".into(),
                    ));
                }
                return Ok(path);
            }
        }
        return Err(AppError::Validation(
            "Native session has no workspace provenance".into(),
        ));
    }
    if spec.driver_type != "codex_terminal" {
        return Err(AppError::Validation(
            "Native learning supports Claude Code and Codex".into(),
        ));
    }
    let home = spec
        .codex_home
        .as_ref()
        .map(PathBuf::from)
        .or(
            harness_account_env(HeadlessDriver::Codex, &spec.harness_account)
                .map_err(AppError::Validation)?
                .map(|(_, path)| path),
        )
        .or_else(crate::codex::normal_codex_home)
        .ok_or_else(|| AppError::NotFound("Selected Codex account home is unavailable".into()))?;
    let files = crate::codex::collect_codex_session_files(&home.join("sessions"))?;
    let mut matches = Vec::new();
    for path in files {
        // The metadata, not the filename, owns the identity and workspace.
        if let Some(meta) = crate::codex::read_codex_session_meta(Path::new(&path))?
            && meta.session_id == id
            && Path::new(&meta.cwd).canonicalize().ok().as_ref() == Some(&workspace)
        {
            matches.push(PathBuf::from(path));
        }
    }
    if matches.len() != 1 {
        return Err(AppError::NotFound(
            "Selected Codex session is missing or ambiguous in this account".into(),
        ));
    }
    Ok(matches.remove(0))
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::Internal(format!("read native experience source: {}", error.kind()))
}

/// Revalidate cited records before a committed cursor in the selected native
/// session. Returns projected records; excluded reasoning and unread records are
/// never evidence. Requires the reader's append-only source assumption; missing
/// or truncated source fails the review. An envelope or complete serialized
/// response exceeding 160 KiB fails validation without returning partial evidence.
pub fn references(
    spec: &TerminalSpec,
    cursor: &Cursor,
    references: &[String],
) -> Result<Vec<HistoricalRecord>, AppError> {
    if spec.resume_session_id.as_deref() != Some(&cursor.session)
        || cursor.session.is_empty()
        || !cursor
            .session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::Validation(
            "Historical evidence requires the selected native session".into(),
        ));
    }
    references_before(
        &source_path(spec, &cursor.session)?,
        cursor,
        references,
        spec.driver_type == "claude_terminal",
    )
}

fn references_before(
    path: &Path,
    cursor: &Cursor,
    references: &[String],
    claude: bool,
) -> Result<Vec<HistoricalRecord>, AppError> {
    let file = File::open(path).map_err(io_error)?;
    if cursor.offset > file.metadata().map_err(io_error)?.len() {
        return Err(AppError::Validation(
            "Historical learning source was truncated; evidence cannot be recovered".into(),
        ));
    }
    let mut reader = BufReader::new(file);
    let mut verified = Vec::new();
    let mut response_bytes = 2; // Serialized array brackets, including the empty response.
    for reference in references {
        let Some(offset) = reference
            .strip_prefix(&format!("{}:", cursor.session))
            .and_then(|offset| offset.parse::<u64>().ok())
            .filter(|offset| {
                *reference == format!("{}:{offset}", cursor.session) && *offset < cursor.offset
            })
        else {
            continue;
        };
        // A byte inside a JSON string or nested value is not a source record.
        if offset > 0 {
            reader.seek(SeekFrom::Start(offset - 1)).map_err(io_error)?;
            let mut preceding = [0];
            reader.read_exact(&mut preceding).map_err(io_error)?;
            if preceding != [b'\n'] {
                continue;
            }
        } else {
            reader.seek(SeekFrom::Start(0)).map_err(io_error)?;
        }
        if let Some((raw, size)) = entry(&mut reader)?
            && size <= cursor.offset - offset
            && let Some(record) = project(raw, claude)
        {
            let envelope = HistoricalRecord {
                reference: reference.clone(),
                session: cursor.session.clone(),
                offset,
                record,
            };
            let count = serde_json::to_vec(&envelope)
                .map_err(|_| AppError::Internal("Encode historical learning record".into()))?
                .len();
            if count > MAX_WINDOW_BYTES {
                return Err(AppError::Validation(
                    "Historical record exceeds the recovery response limit".into(),
                ));
            }
            response_bytes += count + usize::from(!verified.is_empty());
            if response_bytes > MAX_WINDOW_BYTES {
                return Err(AppError::Validation(
                    "Historical records exceed the recovery response limit".into(),
                ));
            }
            verified.push(envelope);
        }
    }
    Ok(verified)
}

fn entry(reader: &mut BufReader<File>) -> Result<Option<(Value, u64)>, AppError> {
    let mut bytes = Vec::new();
    reader
        .take(8 * 1024 * 1024 + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(io_error)?;
    if bytes.is_empty() || bytes.last() != Some(&b'\n') {
        // A writer may still be completing this record. Do not commit its cursor.
        return Ok(None);
    }
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(AppError::Validation(
            "Native record exceeds the analysis reader limit; no source cursor was advanced".into(),
        ));
    }
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Validation("Native transcript contains an invalid record".into()))?;
    Ok(Some((value, bytes.len() as u64)))
}

fn read_window(path: &Path, id: &str, previous: Cursor, claude: bool) -> Result<Window, AppError> {
    let file = File::open(path).map_err(io_error)?;
    let length = file.metadata().map_err(io_error)?.len();
    let reset = previous.session != id || previous.offset > length;
    let mut offset = if reset { 0 } else { previous.offset };
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(offset)).map_err(io_error)?;
    let mut records = Vec::new();
    let mut bytes = 0;
    let mut more = false;
    while let Some((raw, size)) = entry(&mut reader)? {
        if let Some(record) = project(raw, claude) {
            let record = json!({"ref":format!("{id}:{offset}"),"record":record});
            let count = record.to_string().len();
            if count > MAX_WINDOW_BYTES {
                return Err(AppError::Validation(
                    "A native record exceeds the model window; no partial record was analyzed"
                        .into(),
                ));
            }
            if bytes + count > MAX_WINDOW_BYTES {
                more = true;
                break;
            }
            bytes += count;
            records.push(record);
        }
        offset += size;
    }
    Ok(Window {
        records,
        cursor: Cursor {
            session: id.into(),
            offset,
        },
        more,
        reset,
        project_guidance: Vec::new(),
    })
}

fn project(mut raw: Value, claude: bool) -> Option<Value> {
    if claude {
        if !matches!(raw["type"].as_str(), Some("user" | "assistant" | "result")) {
            return None;
        }
        if let Some(content) = raw["message"]["content"].as_array_mut() {
            content.retain(|block| {
                !matches!(
                    block["type"].as_str(),
                    Some("thinking" | "redacted_thinking")
                )
            });
        }
        Some(
            json!({"type":raw["type"],"uuid":raw["uuid"],"parentUuid":raw["parentUuid"],
            "isSidechain":raw["isSidechain"],"timestamp":raw["timestamp"],"message":raw["message"],"result":raw["result"]}),
        )
    } else {
        if raw["type"] != "response_item" || raw["payload"]["type"] == "reasoning" {
            return None;
        }
        Some(json!({"timestamp":raw["timestamp"],"payload":raw["payload"]}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn historical_response_rejects_oversized_serialized_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let content = "x".repeat(160 * 1024 - 1);
        let raw = format!(
            "{}\n",
            json!({"type":"response_item","payload":{
                "type":"message","role":"user","content":content
            }})
        );
        std::fs::write(&path, &raw).unwrap();
        let cursor = Cursor {
            session: "owned".into(),
            offset: raw.len() as u64,
        };
        let result = references_before(&path, &cursor, &["owned:0".into()], false);
        assert!(matches!(result, Err(AppError::Validation(message))
            if message == "Historical record exceeds the recovery response limit"));
    }

    #[test]
    fn historical_response_bounds_serialized_array_including_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let raw = |content: String| {
            format!(
                "{}\n",
                json!({"type":"response_item","payload":{
                    "type":"message","role":"user","content":content
                }})
            )
        };
        let baseline = raw(String::new());
        let projected = project(serde_json::from_str(&baseline).unwrap(), false).unwrap();
        let envelope = HistoricalRecord {
            reference: "owned:0".into(),
            session: "owned".into(),
            offset: 0,
            record: projected,
        };
        let overhead = serde_json::to_vec(&vec![envelope]).unwrap().len();
        let boundary = raw("x".repeat(160 * 1024 - overhead));
        std::fs::write(&path, &boundary).unwrap();
        let cursor = Cursor {
            session: "owned".into(),
            offset: boundary.len() as u64,
        };
        let recovered = references_before(&path, &cursor, &["owned:0".into()], false).unwrap();
        assert_eq!(serde_json::to_vec(&recovered).unwrap().len(), 160 * 1024);
        let oversized = raw("x".repeat(160 * 1024 - overhead + 1));
        std::fs::write(&path, &oversized).unwrap();
        let cursor = Cursor {
            offset: oversized.len() as u64,
            ..cursor
        };
        assert!(
            matches!(references_before(&path, &cursor, &["owned:0".into()], false),
            Err(AppError::Validation(message)) if message == "Historical records exceed the recovery response limit")
        );
    }

    #[test]
    fn historical_response_rejects_aggregate_of_individually_small_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let small = format!(
            "{}\n",
            json!({"type":"response_item","payload":{
                "type":"message","role":"user","content":"x".repeat(80 * 1024)
            }})
        );
        std::fs::write(&path, format!("{small}{small}")).unwrap();
        let cursor = Cursor {
            session: "owned".into(),
            offset: (small.len() * 2) as u64,
        };
        let refs = vec!["owned:0".into(), format!("owned:{}", small.len())];
        for reference in &refs {
            let one =
                references_before(&path, &cursor, std::slice::from_ref(reference), false).unwrap();
            assert!(serde_json::to_vec(&one).unwrap().len() < 160 * 1024);
        }
        assert!(matches!(references_before(&path, &cursor, &refs, false),
            Err(AppError::Validation(message)) if message == "Historical records exceed the recovery response limit"));
    }

    #[test]
    fn historical_records_return_only_projected_visible_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let raw = format!(
            "{}\n",
            json!({"type":"user","message":{"content":[
                {"type":"text","text":"[choruz-experience revision=p1]"},
                {"type":"thinking","thinking":"excluded fixture"},
                {"type":"redacted_thinking","data":"excluded fixture"}
            ]}})
        );
        std::fs::write(&path, &raw).unwrap();
        let cursor = Cursor {
            session: "owned".into(),
            offset: raw.len() as u64,
        };
        let records = references_before(&path, &cursor, &["owned:0".into()], true).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].record["message"]["content"],
            json!([
                {"type":"text","text":"[choruz-experience revision=p1]"}
            ])
        );
    }

    #[test]
    fn historical_references_require_complete_projected_records_before_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let opening = format!(
            "{}\n",
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":"Check the result"}})
        );
        let reasoning = format!(
            "{}\n",
            json!({"type":"response_item","payload":{"type":"reasoning"}})
        );
        let unread = format!(
            "{}\n",
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":"Later result"}})
        );
        std::fs::write(&path, format!("{opening}{reasoning}{unread}")).unwrap();
        let cursor = Cursor {
            session: "owned".into(),
            offset: (opening.len() + reasoning.len()) as u64,
        };
        let refs = vec![
            "owned:0".into(),
            "other:0".into(),
            "owned:1".into(),
            "owned:00".into(),
            format!("owned:{}", opening.len()),
            format!("owned:{}", cursor.offset),
        ];
        let recovered = references_before(&path, &cursor, &refs, false).unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].reference, "owned:0");
        assert_eq!(recovered[0].session, "owned");
        assert_eq!(recovered[0].offset, 0);
        assert_eq!(recovered[0].record["payload"]["role"], "user");
        assert_eq!(
            recovered[0].record["payload"]["content"],
            "Check the result"
        );
        let partial = Cursor {
            offset: opening.len() as u64 - 1,
            ..cursor.clone()
        };
        assert!(
            references_before(&path, &partial, &refs, false)
                .unwrap()
                .is_empty()
        );
        std::fs::write(&path, &opening).unwrap();
        assert!(references_before(&path, &cursor, &refs, false).is_err());
    }

    #[test]
    fn windows_resume_without_dropping_attempts_or_partial_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut file = File::create(&path).unwrap();
        let first = json!({"type":"user","message":{"content":"Please explain the result"}});
        writeln!(file, "{first}").unwrap();
        for index in 0..4 {
            writeln!(file,"{}",json!({"type":"assistant","uuid":format!("attempt-{index}"),"message":{"content":[{"type":"text","text":"x".repeat(50000)},{"type":"thinking","thinking":"private reasoning"}]}})).unwrap();
        }
        let correction = json!({"type":"user","message":{"content":"The result is wrong; you skipped the failed check"}}).to_string();
        write!(file, "{}", &correction[..20]).unwrap();
        file.flush().unwrap();
        let first = read_window(&path, "owned-session", Cursor::default(), true).unwrap();
        assert!(first.more);
        assert_eq!(first.records.len(), 4);
        assert!(
            first.records[0]["record"]["message"]["content"]
                .as_str()
                .unwrap()
                .contains("explain")
        );
        assert!(
            !serde_json::to_string(&first)
                .unwrap()
                .contains("private reasoning")
        );
        let second = read_window(&path, "owned-session", first.cursor.clone(), true).unwrap();
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.records[0]["record"]["uuid"], "attempt-3");
        assert!(!second.more);
        writeln!(file, "{}", &correction[20..]).unwrap();
        file.flush().unwrap();
        let third = read_window(&path, "owned-session", second.cursor, true).unwrap();
        assert_eq!(third.records.len(), 1);
        assert!(
            third.records[0]["record"]["message"]["content"]
                .as_str()
                .unwrap()
                .contains("wrong")
        );
        assert!(
            read_window(&path, "owned-session", third.cursor, true)
                .unwrap()
                .records
                .is_empty()
        );
    }

    #[test]
    fn codex_source_uses_selected_device_home_and_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("account-b");
        let workspace = dir.path().join("workspace-b");
        std::fs::create_dir_all(home.join("sessions/2026")).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let path = home.join("sessions/2026/rollout.jsonl");
        std::fs::write(&path,format!("{}\n{}\n",json!({"type":"session_meta","payload":{"id":"session-b","cwd":workspace}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Only device B"}]}}))).unwrap();
        let mut spec = TerminalSpec {
            authentication: false,
            terminal_id: "binding-b".into(),
            driver_type: "codex_terminal".into(),
            binary_path: None,
            workspace_path: workspace.to_string_lossy().into(),
            cols: 120,
            rows: 40,
            resume_session_id: Some("session-b".into()),
            codex_home: Some(home.to_string_lossy().into()),
            model: None,
            harness_account: json!({}),
        };
        let window = read(&spec, Cursor::default()).unwrap();
        assert_eq!(window.records.len(), 1);
        assert!(window.records[0].to_string().contains("Only device B"));
        spec.workspace_path = dir.path().to_string_lossy().into();
        assert!(
            read(&spec, Cursor::default()).is_err(),
            "another workspace cannot consume this session"
        );
    }
}
