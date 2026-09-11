//! Read-only harness discovery on the device that owns the executables and login.

use std::{process::Stdio, time::Duration};

use choruz_agent_runtime::headless::HeadlessDriver;
use choruz_common::AppError;
use choruz_harness_login::{AccountProfile, login_binary, probe_account};
use serde_json::{Value, json};
use tokio::process::Command;

const DRIVERS: [&str; 7] = [
    "claude_terminal",
    "codex_terminal",
    "codex_exec",
    "pi_terminal",
    "grok_terminal",
    "opencode_terminal",
    "mathcode_terminal",
];

/// Version checks never start a session. Model inspection uses the device's
/// default profile; explicit accounts keep their account-probe owner.
pub async fn inspect(driver_type: Option<&str>) -> Result<Value, AppError> {
    if let Some(driver_type) = driver_type {
        let driver = HeadlessDriver::from_driver_type(driver_type)
            .ok_or_else(|| AppError::Validation("unsupported driver catalog".into()))?;
        return match driver {
            HeadlessDriver::Claude | HeadlessDriver::Codex => {
                let profile = AccountProfile {
                    driver,
                    account_id: String::new(),
                    profile_kind: "default".into(),
                };
                let models = if driver == HeadlessDriver::Claude {
                    choruz_harness_login::claude_model_catalog(&profile).await
                } else {
                    probe_account(&profile).await.map(|probe| probe.models)
                }
                .map_err(AppError::Internal)?;
                Ok(json!({"models": models}))
            }
            HeadlessDriver::Pi => output(driver, &["--list-models"]).await,
            HeadlessDriver::Grok | HeadlessDriver::OpenCode => output(driver, &["models"]).await,
            HeadlessDriver::MathCode => Ok(json!({"models": []})),
        };
    }
    let mut tasks = tokio::task::JoinSet::new();
    for driver_type in DRIVERS {
        tasks.spawn(async move {
            let Some(driver) = HeadlessDriver::from_driver_type(driver_type) else {
                return json!({"driverId": driver_type, "status": "unavailable", "reason": "Unsupported harness"});
            };
            let result = output(driver, &["--version"]).await;
            json!({
                "driverId": driver_type, "label": driver.label(),
                "status": if result.is_ok() { "available" } else { "unavailable" },
                "reason": result.err().map(|error| error.to_string()).unwrap_or_default(),
                "setupHint": format!("Install {} on the selected device.", driver.label()),
            })
        });
    }
    let mut drivers = vec![
        json!({"driverId": "webhook_agent", "label": "Webhook", "status": "available", "reason": "", "setupHint": ""}),
    ];
    while let Some(result) = tasks.join_next().await {
        drivers.push(result.map_err(|error| AppError::Internal(error.to_string()))?);
    }
    Ok(json!({"drivers": drivers}))
}

async fn output(driver: HeadlessDriver, args: &[&str]) -> Result<Value, AppError> {
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        Command::new(login_binary(driver))
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| AppError::Internal(format!("{} inspection timed out", driver.label())))?
    .map_err(|error| AppError::Internal(format!("{} inspection: {error}", driver.label())))?;
    if !result.status.success() {
        return Err(AppError::Internal(format!(
            "{} inspection failed: {}",
            driver.label(),
            String::from_utf8_lossy(&result.stderr)
        )));
    }
    Ok(
        json!({"stdout": String::from_utf8_lossy(&result.stdout), "stderr": String::from_utf8_lossy(&result.stderr)}),
    )
}
