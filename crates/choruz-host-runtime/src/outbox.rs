//! Outbox commands a remote device ships to the controller.
//!
//! On the gateway's device the pipeline drains `<workspace>/.choruz-outbox/new`
//! itself. A remote workspace is on another machine, so the connector
//! collects the command files there (with the bytes of any `share_file`
//! target) and the controller stores them in a per-binding mirror under the
//! runtime directory, laid out exactly like a workspace outbox so the same
//! pipeline handler drains it.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

static STORE_SHIPMENT_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Largest `share_file` target a device ships inline.
pub const MAX_SHIPPED_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// The base64 length of a file at the cap; a longer encoding is refused
/// before it is decoded.
pub const MAX_SHIPPED_ENCODED_BYTES: usize = (MAX_SHIPPED_FILE_BYTES as usize).div_ceil(3) * 4;
/// Commands one shipment carries at most.
pub const MAX_COMMANDS_PER_SHIPMENT: usize = 200;
/// Encoded command bytes one shipment carries at most, measured on the
/// serialized commands the request body holds; a device splits a larger
/// collection into several requests, and the route accepts this plus the
/// envelope around the commands.
pub const MAX_SHIPMENT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShippedFile {
    /// Path relative to the workspace, as the command names it.
    pub path: String,
    /// Base64 of the file bytes.
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShippedOutboxCommand {
    /// The Maildir file name, which carries the sequence the helper assigned.
    pub name: String,
    pub command: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ShippedFile>,
}

/// The controller-side mirror of a remote binding's workspace outbox, keyed
/// by the device that shipped into it. The route accepts a shipment only
/// from the binding's current device, and the pipeline drains every mirror
/// of the binding ([`remote_outbox_mirrors`]), so a shipment accepted in the
/// moment a binding moves to another device is still processed and no device
/// is told its commands were stored when nothing will read them.
pub fn remote_outbox_dir(binding_id: &str, host_id: &str) -> PathBuf {
    remote_outbox_root(binding_id).join(host_id)
}

fn remote_outbox_root(binding_id: &str) -> PathBuf {
    crate::codex::choruz_runtime_dir()
        .join("remote-outbox")
        .join(binding_id)
}

/// Every mirror a device has shipped into for a binding, in name order: the
/// current device's and, until they are drained, those of devices the
/// binding ran on before.
pub fn remote_outbox_mirrors(binding_id: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(remote_outbox_root(binding_id)) else {
        return Vec::new();
    };
    let mut mirrors: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    mirrors.sort();
    mirrors
}

/// Where a device keeps commands the controller has not accepted yet: the
/// commands of a headless turn outlive the turn's temporary outbox here
/// until a shipment succeeds.
pub fn spool_dir(binding_id: &str) -> PathBuf {
    crate::codex::choruz_runtime_dir()
        .join("outbox-spool")
        .join(binding_id)
}

/// Keep commands a shipment could not deliver, one file each, for the next
/// shipping pass.
pub fn spool_shipped_commands(
    binding_id: &str,
    commands: &[ShippedOutboxCommand],
) -> Result<(), String> {
    let dir = spool_dir(binding_id);
    std::fs::create_dir_all(&dir).map_err(|error| format!("create outbox spool: {error}"))?;
    for command in commands {
        let name = Path::new(&command.name)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "outbox command name is invalid".to_owned())?;
        let text = serde_json::to_vec(command).map_err(|error| format!("encode: {error}"))?;
        let tmp = dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, text).map_err(|error| format!("write spool: {error}"))?;
        std::fs::rename(&tmp, dir.join(name)).map_err(|error| format!("publish spool: {error}"))?;
    }
    Ok(())
}

/// The bindings with spooled commands waiting for a shipment.
pub fn spooled_binding_ids() -> Vec<String> {
    let root = crate::codex::choruz_runtime_dir().join("outbox-spool");
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut ids = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

/// Read a binding's spooled commands in name order; the files stay until
/// [`CollectedOutbox::delivered`] removes them.
pub fn take_spooled_commands(binding_id: &str) -> CollectedOutbox {
    let Ok(entries) = std::fs::read_dir(spool_dir(binding_id)) else {
        return CollectedOutbox::default();
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut collected = CollectedOutbox::default();
    for path in paths {
        match std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ShippedOutboxCommand>(&bytes).ok())
        {
            Some(command) => {
                collected.commands.push(command);
                collected.files.push(path);
            }
            None => {
                tracing::warn!(file = %path.display(), "dropped an undecodable spooled command");
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    collected
}

/// Split commands into shipments of at most [`MAX_COMMANDS_PER_SHIPMENT`]
/// commands and [`MAX_SHIPMENT_BYTES`] of serialized commands, in order. A
/// command that alone exceeds the byte cap can never be accepted and is
/// dropped with a warning rather than retried forever.
pub fn shipments(commands: Vec<ShippedOutboxCommand>) -> Vec<Vec<ShippedOutboxCommand>> {
    let mut batches: Vec<Vec<ShippedOutboxCommand>> = Vec::new();
    let mut current: Vec<ShippedOutboxCommand> = Vec::new();
    let mut current_bytes = 0usize;
    for command in commands {
        // One byte per command for the separator in the request's array.
        let bytes = serde_json::to_vec(&command).map_or(0, |encoded| encoded.len()) + 1;
        if bytes > MAX_SHIPMENT_BYTES {
            tracing::warn!(
                name = %command.name,
                bytes,
                "dropped an outbox command larger than one shipment"
            );
            continue;
        }
        let full = current.len() >= MAX_COMMANDS_PER_SHIPMENT
            || (!current.is_empty() && current_bytes + bytes > MAX_SHIPMENT_BYTES);
        if full {
            batches.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current_bytes += bytes;
        current.push(command);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// Command files read from a workspace outbox and not yet delivered. The
/// files stay on the device until [`CollectedOutbox::delivered`] removes
/// them, so a ship that fails is retried on the next pass instead of losing
/// the commands.
#[derive(Debug, Default)]
pub struct CollectedOutbox {
    pub commands: Vec<ShippedOutboxCommand>,
    files: Vec<PathBuf>,
}

impl CollectedOutbox {
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Remove the shipped files from the device once the controller has
    /// stored them.
    pub fn delivered(self) {
        for path in self.files {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Read every command file under `<workspace>/.choruz-outbox/new`, in helper
/// order, attaching the bytes of a `share_file` target. A file that cannot be
/// parsed is removed with a warning rather than shipped.
pub fn collect_outbox_commands(workspace: &Path) -> CollectedOutbox {
    let maildir_new = workspace.join(".choruz-outbox").join("new");
    let Ok(entries) = std::fs::read_dir(&maildir_new) else {
        return CollectedOutbox::default();
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut collected = CollectedOutbox::default();
    for path in paths {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        match serde_json::from_str::<Value>(&raw) {
            Ok(command) if command.is_object() => {
                let files = share_file_payload(workspace, &command);
                collected.commands.push(ShippedOutboxCommand {
                    name,
                    command,
                    files,
                });
                collected.files.push(path);
            }
            _ => {
                tracing::warn!(file = %path.display(), "dropped an undecodable outbox command");
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    collected
}

/// Commands the connector's own `CHORUZ_SEND` wrote during a headless turn,
/// as NUL-separated JSON frames. `send` frames are excluded: the turn's
/// completion already carries them.
pub fn commands_from_frames(workspace: &Path, frames: &[u8]) -> Vec<ShippedOutboxCommand> {
    frames
        .split(|byte| *byte == 0)
        .filter_map(|frame| serde_json::from_slice::<Value>(frame).ok())
        .filter(|command| {
            command.is_object() && command.get("type").and_then(Value::as_str) != Some("send")
        })
        .enumerate()
        .map(|(index, command)| ShippedOutboxCommand {
            name: format!("cmd-{:020}-{}.json", index + 1, choruz_common::new_id()),
            files: share_file_payload(workspace, &command),
            command,
        })
        .collect()
}

/// The bytes of a `share_file` target, when the command names a regular
/// file that really lives inside the workspace (symlinks resolved on both
/// sides) and is under the size cap.
fn share_file_payload(workspace: &Path, command: &Value) -> Vec<ShippedFile> {
    if command.get("type").and_then(Value::as_str) != Some("share_file") {
        return Vec::new();
    }
    let Some(relative) = command.get("path").and_then(Value::as_str) else {
        return Vec::new();
    };
    if !relative_path_is_safe(relative) {
        return Vec::new();
    }
    // A link inside the workspace that points elsewhere would otherwise
    // ship bytes from outside it.
    let (Ok(root), Ok(target)) = (
        workspace.canonicalize(),
        workspace.join(relative).canonicalize(),
    ) else {
        return Vec::new();
    };
    if !target.starts_with(&root) {
        tracing::warn!(
            path = relative,
            "share_file target resolves outside the workspace"
        );
        return Vec::new();
    }
    let Ok(metadata) = std::fs::metadata(&target) else {
        return Vec::new();
    };
    if !metadata.is_file() || metadata.len() > MAX_SHIPPED_FILE_BYTES {
        tracing::warn!(
            path = relative,
            "share_file target was not shipped (missing, not a file, or over the size cap)"
        );
        return Vec::new();
    }
    match std::fs::read(&target) {
        Ok(bytes) => vec![ShippedFile {
            path: relative.to_owned(),
            content: STANDARD.encode(bytes),
        }],
        Err(_) => Vec::new(),
    }
}

fn relative_path_is_safe(relative: &str) -> bool {
    let path = Path::new(relative);
    !relative.is_empty()
        && !relative.contains('\0')
        && !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// Whether the pipeline may claim an outbox file. Remote shipments publish a
/// pending receipt before the command and atomically accept it afterwards, so
/// the separate pipeline process cannot run a command from an uncommitted
/// shipment. Local and legacy command files have no pending receipt and remain
/// immediately eligible.
pub fn outbox_command_is_ready(work_dir: &Path, command_path: &Path) -> bool {
    if command_path
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("json")
    {
        return true;
    }
    let Some(name) = command_path.file_name() else {
        return false;
    };
    !work_dir
        .join(".choruz-outbox/receipts/pending")
        .join(name)
        .exists()
}

struct PreparedCommand {
    name: String,
    command_text: String,
    digest: String,
    files: Vec<(PathBuf, Vec<u8>)>,
}

fn prepare_shipped_command(shipped: &ShippedOutboxCommand) -> Result<PreparedCommand, AppError> {
    let name = Path::new(&shipped.name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.ends_with(".json"))
        .ok_or_else(|| AppError::Validation("outbox command name is invalid".into()))?
        .to_owned();
    let mut files = Vec::with_capacity(shipped.files.len());
    for file in &shipped.files {
        if !relative_path_is_safe(&file.path) {
            return Err(AppError::Validation(format!(
                "shipped file path {} must stay inside the workspace",
                file.path
            )));
        }
        if file.content.len() > MAX_SHIPPED_ENCODED_BYTES {
            return Err(AppError::Validation(format!(
                "shipped file {} exceeds {MAX_SHIPPED_FILE_BYTES} bytes",
                file.path
            )));
        }
        let bytes = STANDARD.decode(&file.content).map_err(|error| {
            AppError::Validation(format!("shipped file is not base64: {error}"))
        })?;
        if bytes.len() as u64 > MAX_SHIPPED_FILE_BYTES {
            return Err(AppError::Validation(format!(
                "shipped file {} exceeds {MAX_SHIPPED_FILE_BYTES} bytes",
                file.path
            )));
        }
        files.push((PathBuf::from(&file.path), bytes));
    }
    let command_text = serde_json::to_string(&shipped.command)
        .map_err(|error| AppError::Internal(format!("encode outbox command: {error}")))?;
    let shipment = serde_json::to_vec(shipped)
        .map_err(|error| AppError::Internal(format!("encode shipped command: {error}")))?;
    let digest = hex::encode(Sha256::digest(shipment));
    Ok(PreparedCommand {
        name,
        command_text,
        digest,
        files,
    })
}

fn read_receipt(path: &Path) -> Result<Option<String>, AppError> {
    match std::fs::read_to_string(path) {
        Ok(digest) => Ok(Some(digest)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::Internal(format!(
            "read outbox shipment receipt: {error}"
        ))),
    }
}

fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>, operation: &str) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Internal(format!("{operation}: path has no parent")))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| AppError::Internal(format!("{operation}: {error}")))?;
    let tmp = parent.join(format!(".tmp-{}", choruz_common::new_id()));
    std::fs::write(&tmp, bytes)
        .map_err(|error| AppError::Internal(format!("{operation}: {error}")))?;
    if let Err(error) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::Internal(format!("{operation}: {error}")));
    }
    Ok(())
}

fn shipment_lock(mirror: &Path) -> Arc<Mutex<()>> {
    let mut locks = STORE_SHIPMENT_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(lock) = locks.get(mirror).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(mirror.to_path_buf(), Arc::downgrade(&lock));
    lock
}

/// Write shipped commands into the binding's mirror for `host_id` so the
/// pipeline's outbox watcher drains them like a local workspace: command
/// files under `.choruz-outbox/new`, `share_file` targets at the path the
/// command names. A file over [`MAX_SHIPPED_FILE_BYTES`] is refused before
/// it is decoded.
pub fn store_shipped_commands(
    binding_id: &str,
    host_id: &str,
    commands: &[ShippedOutboxCommand],
) -> Result<PathBuf, AppError> {
    if commands.len() > MAX_COMMANDS_PER_SHIPMENT {
        return Err(AppError::Validation(format!(
            "ship at most {MAX_COMMANDS_PER_SHIPMENT} outbox commands per request"
        )));
    }
    let prepared = commands
        .iter()
        .map(prepare_shipped_command)
        .collect::<Result<Vec<_>, _>>()?;
    let mut names = HashMap::new();
    let mut unique = Vec::with_capacity(prepared.len());
    for command in prepared {
        match names.get(&command.name) {
            Some(digest) if digest != &command.digest => {
                return Err(AppError::Conflict(format!(
                    "outbox command {} was reused with different content",
                    command.name
                )));
            }
            Some(_) => continue,
            None => {
                names.insert(command.name.clone(), command.digest.clone());
                unique.push(command);
            }
        }
    }
    let prepared = unique;

    let mirror = remote_outbox_dir(binding_id, host_id);
    let shipment_lock = shipment_lock(&mirror);
    let _lock = shipment_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let outbox = mirror.join(".choruz-outbox");
    let maildir_new = outbox.join("new");
    let pending_dir = outbox.join("receipts/pending");
    let accepted_dir = outbox.join("receipts/accepted");
    std::fs::create_dir_all(&maildir_new)
        .map_err(|error| AppError::Internal(format!("create remote outbox mirror: {error}")))?;
    std::fs::create_dir_all(&pending_dir)
        .map_err(|error| AppError::Internal(format!("create pending receipt dir: {error}")))?;
    std::fs::create_dir_all(&accepted_dir)
        .map_err(|error| AppError::Internal(format!("create accepted receipt dir: {error}")))?;

    let mut accepted = Vec::with_capacity(prepared.len());
    for command in &prepared {
        let accepted_receipt = accepted_dir.join(&command.name);
        let pending_receipt = pending_dir.join(&command.name);
        if let Some(digest) = read_receipt(&accepted_receipt)? {
            if digest != command.digest {
                return Err(AppError::Conflict(format!(
                    "outbox command {} was reused with different content",
                    command.name
                )));
            }
            if let Some(pending_digest) = read_receipt(&pending_receipt)?
                && pending_digest != command.digest
            {
                return Err(AppError::Conflict(format!(
                    "outbox command {} has conflicting shipment receipts",
                    command.name
                )));
            }
            accepted.push(true);
            continue;
        }
        if let Some(digest) = read_receipt(&pending_receipt)?
            && digest != command.digest
        {
            return Err(AppError::Conflict(format!(
                "outbox command {} was reused with different content",
                command.name
            )));
        }
        accepted.push(false);
    }

    for (command, already_accepted) in prepared.iter().zip(accepted) {
        let pending_receipt = pending_dir.join(&command.name);
        if already_accepted {
            let _ = std::fs::remove_file(pending_receipt);
            continue;
        }
        if !pending_receipt.exists() {
            write_atomic(
                &pending_receipt,
                command.digest.as_bytes(),
                "write pending outbox shipment receipt",
            )?;
        }
        for (path, bytes) in &command.files {
            let target = mirror.join(path);
            write_atomic(&target, bytes, "write shipped file")?;
        }
        write_atomic(
            &maildir_new.join(&command.name),
            command.command_text.as_bytes(),
            "publish outbox command",
        )?;
        std::fs::rename(&pending_receipt, accepted_dir.join(&command.name)).map_err(|error| {
            AppError::Internal(format!("accept outbox shipment receipt: {error}"))
        })?;
    }
    Ok(mirror)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Point the runtime directory at a temporary root for one test body.
    /// Bodies run one at a time, and the previous value comes back even
    /// when a body panics, so parallel tests never see each other's root.
    fn with_runtime_dir<T>(runtime: &Path, body: impl FnOnce() -> T) -> T {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    match self.0.take() {
                        Some(value) => std::env::set_var("CHORUZ_RUNTIME_DIR", value),
                        None => std::env::remove_var("CHORUZ_RUNTIME_DIR"),
                    }
                }
            }
        }
        let _serialized = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _restore = Restore(std::env::var_os("CHORUZ_RUNTIME_DIR"));
        unsafe { std::env::set_var("CHORUZ_RUNTIME_DIR", runtime) };
        body()
    }

    #[test]
    fn workspace_outbox_commands_ship_in_order_with_share_file_bytes_and_leave_no_files() {
        let workspace = tempfile::tempdir().unwrap();
        let new = workspace.path().join(".choruz-outbox").join("new");
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(workspace.path().join("report.md"), b"# done\n").unwrap();
        std::fs::write(
            new.join("cmd-00000000000000000002-b.json"),
            json!({"type": "share_file", "group": "team", "path": "report.md"}).to_string(),
        )
        .unwrap();
        std::fs::write(
            new.join("cmd-00000000000000000001-a.json"),
            json!({"type": "send", "group": "team", "content": "hi"}).to_string(),
        )
        .unwrap();
        std::fs::write(new.join("garbage.json"), "not json").unwrap();

        let collected = collect_outbox_commands(workspace.path());
        let shipped = &collected.commands;
        assert_eq!(shipped.len(), 2);
        assert_eq!(shipped[0].command["type"], "send");
        assert_eq!(shipped[1].command["type"], "share_file");
        assert_eq!(shipped[1].files.len(), 1);
        assert_eq!(shipped[1].files[0].path, "report.md");
        assert_eq!(
            STANDARD.decode(&shipped[1].files[0].content).unwrap(),
            b"# done\n"
        );
        assert_eq!(
            std::fs::read_dir(&new).unwrap().count(),
            2,
            "command files stay until the controller has them; garbage is gone"
        );
        collected.delivered();
        assert!(std::fs::read_dir(&new).unwrap().next().is_none());
    }

    #[test]
    fn headless_frames_keep_every_command_except_send() {
        let workspace = tempfile::tempdir().unwrap();
        let frames = format!(
            "{}\0{}\0{}\0",
            json!({"type": "send", "group": "team", "content": "hi"}),
            json!({"type": "create_group", "name": "review", "members": ["a"]}),
            json!({"type": "share_file", "group": "team", "path": "../etc/passwd"}),
        );
        let shipped = commands_from_frames(workspace.path(), frames.as_bytes());
        assert_eq!(shipped.len(), 2);
        assert_eq!(shipped[0].command["type"], "create_group");
        assert!(shipped[0].name.ends_with(".json"));
        assert!(
            shipped[1].files.is_empty(),
            "a path outside the workspace is never read"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_share_file_target_behind_a_symlink_outside_the_workspace_is_not_shipped() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"top secret").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret"),
            workspace.path().join("linked"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), workspace.path().join("linked-dir")).unwrap();
        std::fs::write(workspace.path().join("mine.txt"), b"mine").unwrap();
        let shipped = |path: &str| {
            share_file_payload(
                workspace.path(),
                &json!({"type": "share_file", "group": "team", "path": path}),
            )
        };
        assert!(shipped("linked").is_empty());
        assert!(shipped("linked-dir/secret").is_empty());
        assert_eq!(shipped("mine.txt").len(), 1);
    }

    #[test]
    fn stored_commands_land_in_the_mirror_like_a_workspace_outbox() {
        let runtime = tempfile::tempdir().unwrap();
        let (stored, escape) = with_runtime_dir(runtime.path(), || {
            let stored = store_shipped_commands(
                "binding-9",
                "host-a",
                &[ShippedOutboxCommand {
                    name: "cmd-00000000000000000007-x.json".into(),
                    command: json!({"type": "share_file", "group": "team", "path": "out/report.md"}),
                    files: vec![ShippedFile {
                        path: "out/report.md".into(),
                        content: STANDARD.encode(b"hello"),
                    }],
                }],
            );
            let escape = store_shipped_commands(
                "binding-9",
                "host-a",
                &[ShippedOutboxCommand {
                    name: "cmd-1.json".into(),
                    command: json!({"type": "share_file", "path": "../x"}),
                    files: vec![ShippedFile {
                        path: "../x".into(),
                        content: STANDARD.encode(b"no"),
                    }],
                }],
            );
            let from_former_device = store_shipped_commands(
                "binding-9",
                "host-0",
                &[ShippedOutboxCommand {
                    name: "cmd-00000000000000000001-f.json".into(),
                    command: json!({"type": "task_update", "task_key": "T-1"}),
                    files: Vec::new(),
                }],
            );
            assert_eq!(
                remote_outbox_mirrors("binding-9"),
                vec![from_former_device.unwrap(), stored.clone().unwrap()],
                "every device's mirror of the binding is listed, in name order"
            );
            assert!(remote_outbox_mirrors("binding-none").is_empty());
            (stored, escape)
        });
        let mirror = stored.unwrap();
        assert_eq!(
            mirror,
            runtime
                .path()
                .join("remote-outbox")
                .join("binding-9")
                .join("host-a"),
            "the mirror is keyed by binding and by the device that shipped"
        );
        assert_eq!(
            std::fs::read(mirror.join("out/report.md")).unwrap(),
            b"hello"
        );
        let command = std::fs::read_to_string(
            mirror
                .join(".choruz-outbox/new")
                .join("cmd-00000000000000000007-x.json"),
        )
        .unwrap();
        assert!(command.contains("share_file"));
        assert!(escape.is_err());
    }

    #[test]
    fn an_accepted_shipment_is_not_republished_after_its_command_was_drained() {
        let runtime = tempfile::tempdir().unwrap();
        with_runtime_dir(runtime.path(), || {
            let command = ShippedOutboxCommand {
                name: "cmd-00000000000000000007-x.json".into(),
                command: json!({"type": "send", "group": "team", "content": "once"}),
                files: Vec::new(),
            };
            let mirror =
                store_shipped_commands("binding-9", "host-a", std::slice::from_ref(&command))
                    .expect("store first shipment");
            let published = mirror.join(".choruz-outbox/new").join(&command.name);
            std::fs::remove_file(&published).expect("simulate the pipeline draining the command");

            store_shipped_commands("binding-9", "host-a", &[command])
                .expect("an identical retry is accepted");

            assert!(
                !published.exists(),
                "an ACK-loss retry must not recreate a command the pipeline already drained"
            );
        });
    }

    #[test]
    fn a_shipment_name_cannot_be_reused_for_different_content() {
        let runtime = tempfile::tempdir().unwrap();
        let result = with_runtime_dir(runtime.path(), || {
            store_shipped_commands(
                "binding-9",
                "host-a",
                &[ShippedOutboxCommand {
                    name: "cmd-00000000000000000007-x.json".into(),
                    command: json!({"type": "send", "content": "first"}),
                    files: Vec::new(),
                }],
            )
            .expect("store first shipment");

            store_shipped_commands(
                "binding-9",
                "host-a",
                &[ShippedOutboxCommand {
                    name: "cmd-00000000000000000007-x.json".into(),
                    command: json!({"type": "send", "content": "different"}),
                    files: Vec::new(),
                }],
            )
        });

        assert!(matches!(result, Err(AppError::Conflict(_))));
    }

    #[test]
    fn oversized_shipped_files_are_refused_before_they_are_decoded() {
        let runtime = tempfile::tempdir().unwrap();
        let (too_long, over_cap) = with_runtime_dir(runtime.path(), || {
            let too_long = store_shipped_commands(
                "binding-big",
                "host-a",
                &[ShippedOutboxCommand {
                    name: "cmd-1.json".into(),
                    command: json!({"type": "share_file", "path": "big.bin"}),
                    files: vec![ShippedFile {
                        path: "big.bin".into(),
                        content: "A".repeat(MAX_SHIPPED_ENCODED_BYTES + 4),
                    }],
                }],
            );
            let over_cap = store_shipped_commands(
                "binding-big",
                "host-a",
                &[ShippedOutboxCommand {
                    name: "cmd-2.json".into(),
                    command: json!({"type": "share_file", "path": "big.bin"}),
                    files: vec![ShippedFile {
                        path: "big.bin".into(),
                        content: STANDARD.encode(vec![0u8; MAX_SHIPPED_FILE_BYTES as usize + 1]),
                    }],
                }],
            );
            (too_long, over_cap)
        });
        assert!(matches!(too_long, Err(AppError::Validation(_))));
        assert!(matches!(over_cap, Err(AppError::Validation(_))));
        assert!(
            !runtime
                .path()
                .join("remote-outbox/binding-big/host-a/big.bin")
                .exists(),
            "a refused file is never written"
        );
    }

    #[test]
    fn spooled_commands_wait_for_the_next_shipment() {
        let runtime = tempfile::tempdir().unwrap();
        let commands = vec![
            ShippedOutboxCommand {
                name: "cmd-00000000000000000001-a.json".into(),
                command: json!({"type": "create_group", "name": "review"}),
                files: Vec::new(),
            },
            ShippedOutboxCommand {
                name: "cmd-00000000000000000002-b.json".into(),
                command: json!({"type": "share_file", "path": "r.md"}),
                files: vec![ShippedFile {
                    path: "r.md".into(),
                    content: STANDARD.encode(b"r"),
                }],
            },
        ];
        let (ids, taken_commands, after_empty) = with_runtime_dir(runtime.path(), || {
            spool_shipped_commands("binding-s", &commands).unwrap();
            let ids = spooled_binding_ids();
            let taken = take_spooled_commands("binding-s");
            let taken_commands = taken.commands.clone();
            taken.delivered();
            (
                ids,
                taken_commands,
                take_spooled_commands("binding-s").is_empty(),
            )
        });
        assert_eq!(ids, ["binding-s"]);
        assert_eq!(taken_commands, commands, "the spool round-trips files too");
        assert!(after_empty, "delivered commands leave the spool");
    }

    #[test]
    fn shipments_are_split_by_count_and_by_encoded_bytes() {
        let small = |i: usize| ShippedOutboxCommand {
            name: format!("cmd-{i:020}-s.json"),
            command: json!({"type": "task_update"}),
            files: Vec::new(),
        };
        let big = |i: usize| ShippedOutboxCommand {
            name: format!("cmd-{i:020}-b.json"),
            command: json!({"type": "share_file", "path": "b"}),
            files: vec![ShippedFile {
                path: "b".into(),
                content: "x".repeat(MAX_SHIPMENT_BYTES / 2 + 1),
            }],
        };
        let by_count = shipments((0..MAX_COMMANDS_PER_SHIPMENT + 1).map(small).collect());
        assert_eq!(by_count.len(), 2);
        assert_eq!(by_count[0].len(), MAX_COMMANDS_PER_SHIPMENT);
        let by_bytes = shipments(vec![big(1), big(2), small(3)]);
        assert_eq!(
            by_bytes.len(),
            2,
            "two half-cap files do not share a shipment"
        );
        assert_eq!(by_bytes[1].len(), 2);
        let wordy = |i: usize| ShippedOutboxCommand {
            name: format!("cmd-{i:020}-w.json"),
            command: json!({"type": "send", "content": "y".repeat(MAX_SHIPMENT_BYTES / 2 + 1)}),
            files: Vec::new(),
        };
        assert_eq!(
            shipments(vec![wordy(1), wordy(2)]).len(),
            2,
            "the command text counts, not only attached files"
        );
        let oversized = ShippedOutboxCommand {
            name: "cmd-00000000000000000009-o.json".into(),
            command: json!({"type": "send", "content": "z".repeat(MAX_SHIPMENT_BYTES)}),
            files: Vec::new(),
        };
        let without_oversized = shipments(vec![small(1), oversized, small(2)]);
        assert_eq!(without_oversized.len(), 1);
        assert_eq!(
            without_oversized[0].len(),
            2,
            "a command no shipment can carry is dropped"
        );
        assert!(shipments(Vec::new()).is_empty());
    }
}
