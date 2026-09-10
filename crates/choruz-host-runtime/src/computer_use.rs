//! Device-owned setup. Requests accept tool identities, never commands or URLs.
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::LazyLock, time::Duration};
use tokio::{process::Command, sync::Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Browser,
    Desktop,
}

impl Tool {
    fn binary(self) -> &'static str {
        match self {
            Self::Browser => "bsk",
            Self::Desktop => "cua-driver",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Browser => "browser-skill",
            Self::Desktop => "cua-driver",
        }
    }
}

// A job survives the requesting tab; one job per tool prevents duplicate installers.
static JOBS: LazyLock<Mutex<BTreeMap<Tool, Option<String>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn home() -> Result<PathBuf, AppError> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| AppError::Validation("Device HOME is not configured".into()))
}

fn binary(tool: Tool) -> PathBuf {
    if let Ok(home) = home() {
        let local = home.join(".local/bin").join(tool.binary());
        if local.is_file() {
            return local;
        }
    }
    PathBuf::from(tool.binary())
}

/// Read health or persist automatic provisioning and start an installation.
/// Disable is not a sandbox: user-installed tools and OS grants are untouched.
pub async fn manage(tool: Option<Tool>, enabled: Option<bool>) -> Result<Value, AppError> {
    if tool.is_some() != enabled.is_some() {
        return Err(AppError::Validation(
            "tool and enabled must be supplied together".into(),
        ));
    }
    if let (Some(tool), Some(enabled)) = (tool, enabled) {
        let mut jobs = JOBS.lock().await;
        if jobs.get(&tool) == Some(&None) {
            return Err(AppError::Validation(
                "Installation is running; wait for it to finish".into(),
            ));
        }
        let marker = home()?
            .join(".choruz/computer-use")
            .join(format!("{}.disabled", tool.key()));
        if enabled {
            match tokio::fs::remove_file(&marker).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(AppError::Internal(format!("enable tool: {error}"))),
            }
            jobs.insert(tool, None);
            tokio::spawn(async move {
                let result = install(tool).await;
                tracing::info!(
                    ?tool,
                    success = result.is_ok(),
                    "computer-use installation finished"
                );
                let mut jobs = JOBS.lock().await;
                match result {
                    Ok(()) => {
                        jobs.remove(&tool);
                    }
                    Err(error) => {
                        jobs.insert(tool, Some(error));
                    }
                }
            });
        } else {
            tokio::fs::create_dir_all(
                marker
                    .parent()
                    .ok_or_else(|| AppError::Internal("invalid tool state directory".into()))?,
            )
            .await
            .map_err(|e| AppError::Internal(format!("prepare tool state: {e}")))?;
            tokio::fs::write(marker, b"")
                .await
                .map_err(|e| AppError::Internal(format!("disable tool: {e}")))?;
            jobs.remove(&tool);
        }
    }
    let mut tools = Vec::new();
    for tool in [Tool::Browser, Tool::Desktop] {
        let enabled = !home()?
            .join(".choruz/computer-use")
            .join(format!("{}.disabled", tool.key()))
            .exists();
        let job = JOBS.lock().await.get(&tool).cloned();
        let (status, checks) = match job {
            Some(None) => ("installing", json!([])),
            Some(Some(error)) => (
                "error",
                json!([{ "name": "Installation", "ok": false, "hint": error }]),
            ),
            None => match health(tool).await {
                Ok(checks) => (
                    if checks.iter().all(|check| check["ok"] == true) {
                        "ready"
                    } else {
                        "needs_attention"
                    },
                    json!(checks),
                ),
                Err(error) => (
                    "needs_attention",
                    json!([{ "name": "Tool", "ok": false, "hint": error }]),
                ),
            },
        };
        tools.push(json!({ "tool": tool, "enabled": enabled, "status": status, "checks": checks }));
    }
    Ok(json!({ "tools": tools }))
}

async fn run(program: PathBuf, args: &[&str], seconds: u64) -> Result<Vec<u8>, String> {
    run_command(program, args, seconds, false).await
}

async fn run_command(
    program: PathBuf,
    args: &[&str],
    seconds: u64,
    diagnostics: bool,
) -> Result<Vec<u8>, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    if let Some(path) = choruz_agent_runtime::computer_use::executable_path() {
        command.env("PATH", path);
    }
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("Unable to run tool: {error}"))?;
    let _container = child
        .id()
        .map(|id| crate::ProcessContainer::new(format!("computer-use-{id}"), id));
    let output = tokio::time::timeout(Duration::from_secs(seconds), child.wait_with_output())
        .await
        .map_err(|_| "Operation timed out; check the device network and retry".to_string())?
        .map_err(|error| format!("Unable to run tool: {error}"))?;
    if !output.status.success() && !diagnostics {
        return Err(format!(
            "Tool command failed ({}). Run {} on the device for details.",
            output.status,
            args.first().unwrap_or(&"doctor")
        ));
    }
    Ok(output.stdout)
}

fn browser_checks(value: Value) -> Result<Vec<Value>, String> {
    let rows = value
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or("Browser diagnostics returned no checks")?;
    Ok(rows
        .iter()
        .map(|row| json!({ "name": row["name"], "ok": row["ok"] == true, "hint": row["hint"] }))
        .collect())
}

async fn health(tool: Tool) -> Result<Vec<Value>, String> {
    // Doctors can exit nonzero precisely when their structured checks need attention.
    let bytes = run_command(binary(tool), &["doctor", "--json"], 15, true).await?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Tool returned invalid diagnostics; update the installed tool")?;
    if tool == Tool::Browser {
        return browser_checks(value);
    }
    let mut checks = vec![json!({ "name": "Driver installation", "ok": value["ok"] == true })];
    let home = home().map_err(|error| error.to_string())?;
    let skill_installed = [".agents/skills", ".claude/skills", ".codex/skills"]
        .iter()
        .any(|root| home.join(root).join(tool.key()).join("SKILL.md").is_file());
    checks.push(json!({ "name": "Agent skill", "ok": skill_installed, "hint": "Use Install / repair to install the driver's skill." }));
    if cfg!(target_os = "macos") {
        let permissions =
            run_command(binary(tool), &["permissions", "status", "--json"], 15, true).await?;
        let value: Value = serde_json::from_slice(&permissions)
            .map_err(|_| "Driver returned invalid permission status")?;
        for (key, name) in [
            ("accessibility", "Accessibility permission"),
            ("screen_recording", "Screen Recording permission"),
        ] {
            checks.push(json!({ "name": name, "ok": value[key] == true, "hint": "Grant permission to CuaDriver in this device's System Settings, then check again." }));
        }
    } else if cfg!(target_os = "linux") {
        checks.push(json!({ "name": "Desktop display", "ok": std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some(), "hint": "A graphical desktop is required on this device." }));
    }
    Ok(checks)
}

async fn install(tool: Tool) -> Result<(), String> {
    if !cfg!(unix) {
        return Err("Automatic installation is supported on macOS and Linux. Install the upstream tool on this device, then check again.".into());
    }
    if run(binary(tool), &["--version"], 10).await.is_err() {
        let url = match tool {
            Tool::Browser => {
                "https://raw.githubusercontent.com/Tencent/BrowserSkill/main/install.sh"
            }
            Tool::Desktop => "https://cua.ai/driver/install.sh",
        };
        // Fetch completes before execution; curl cannot pipe a partial script into a shell.
        let script = run(
            PathBuf::from("curl"),
            &[
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "60",
                url,
            ],
            65,
        )
        .await?;
        let directory = home()
            .map_err(|e| e.to_string())?
            .join(".choruz/computer-use");
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, &script).map_err(|e| e.to_string())?;
        let path_arg = file.path().to_str().ok_or("Invalid install path")?;
        let args = if tool == Tool::Desktop {
            vec![path_arg, "--no-modify-path"]
        } else {
            vec![path_arg]
        };
        let result = run(PathBuf::from("bash"), &args, 300).await;
        result?;
    }
    let args: &[&str] = match tool {
        Tool::Browser => &["install-skill", "--all", "--yes"],
        Tool::Desktop => &["skills", "install"],
    };
    run(binary(tool), args, 120).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_health_requires_explicit_success_and_nonempty_checks() {
        assert!(browser_checks(json!([])).is_err());
        let checks = browser_checks(
            json!([{ "name": "extension", "status": "ok" }, { "name": "daemon", "ok": true }]),
        )
        .unwrap();
        assert_eq!(checks[0]["ok"], false);
        assert_eq!(checks[1]["ok"], true);
    }
    #[tokio::test]
    async fn incomplete_mutation_is_rejected_without_installing() {
        assert!(manage(Some(Tool::Browser), None).await.is_err());
        assert!(manage(None, Some(true)).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn device_setup_persists_selection_and_installs_skills_without_touching_another_home() {
        use std::{fs, os::unix::fs::PermissionsExt};
        if std::env::var_os("CHORUZ_TEST_TOOL_HOME").is_none() {
            let root = tempfile::tempdir().unwrap();
            let local = root.path().join(".local/bin");
            fs::create_dir_all(&local).unwrap();
            for name in ["bsk", "cua-driver"] {
                let path = local.join(name);
                fs::write(&path, r#"#!/bin/sh
case "$1" in
--version) echo test;;
install-skill) mkdir -p "$HOME/.agents/skills/browser-skill"; echo browser > "$HOME/.agents/skills/browser-skill/SKILL.md";;
skills) [ "$2" = install ] || exit 2; mkdir -p "$HOME/.agents/skills/cua-driver"; echo desktop > "$HOME/.agents/skills/cua-driver/SKILL.md";;
permissions) echo '{"accessibility":false,"screen_recording":true}';;
doctor) if [ "${0##*/}" = bsk ]; then echo '[{"name":"extension","ok":false,"hint":"Connect the browser extension"}]'; exit 1; else echo '{"ok":true}'; fi;;
esac
"#).unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
            }
            let result = std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", "computer_use::tests::device_setup_persists_selection_and_installs_skills_without_touching_another_home", "--nocapture"]).env("HOME", root.path()).env("CHORUZ_TEST_TOOL_HOME", "1").output().unwrap();
            assert!(
                result.status.success(),
                "{} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                root.path()
                    .join(".agents/skills/browser-skill/SKILL.md")
                    .exists()
            );
            return;
        }
        let result = crate::execute(crate::HostRequest::ComputerUse {
            tool: Some(Tool::Browser),
            enabled: Some(false),
        })
        .await
        .unwrap();
        assert_eq!(result["tools"][0]["enabled"], false);
        let result = crate::execute(crate::HostRequest::ComputerUse {
            tool: None,
            enabled: None,
        })
        .await
        .unwrap();
        assert_eq!(result["tools"][0]["enabled"], false);
        assert_eq!(result["tools"][0]["status"], "needs_attention");
        crate::execute(crate::HostRequest::ComputerUse {
            tool: Some(Tool::Browser),
            enabled: Some(true),
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if JOBS.lock().await.get(&Tool::Browser) != Some(&None) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let result = crate::execute(crate::HostRequest::ComputerUse {
            tool: None,
            enabled: None,
        })
        .await
        .unwrap();
        assert_eq!(result["tools"][0]["enabled"], true);
        assert_eq!(
            result["tools"][0]["checks"][0]["hint"],
            "Connect the browser extension"
        );
        crate::execute(crate::HostRequest::ComputerUse {
            tool: Some(Tool::Desktop),
            enabled: Some(true),
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while JOBS.lock().await.get(&Tool::Desktop) == Some(&None) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            home()
                .unwrap()
                .join(".agents/skills/cua-driver/SKILL.md")
                .is_file()
        );
        let result = manage(None, None).await.unwrap();
        assert_eq!(result["tools"][1]["checks"][1]["ok"], true);
        if cfg!(target_os = "macos") {
            assert_eq!(result["tools"][1]["status"], "needs_attention");
            assert_eq!(
                result["tools"][1]["checks"][2]["name"],
                "Accessibility permission"
            );
            assert_eq!(result["tools"][1]["checks"][2]["ok"], false);
        }
    }
}
