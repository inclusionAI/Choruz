//! Turns a Remote Control pairing into a persistent Company runtime host.
//!
//! The controller creates a reverse Remote Control credential and a runtime
//! host code, then sends both to this endpoint over its already encrypted
//! session to the target. The target starts `choruz-connector`; that connector
//! reaches the controller through the reverse encrypted relay, so neither
//! computer needs a public API port.

use std::{path::PathBuf, process::Stdio};

use axum::{Json, extract::State, http::HeaderMap};
use choruz_common::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{io::AsyncWriteExt, process::Command};

use crate::{ApiError, ApiState, require_human_operator};

const PAIR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OnboardRequest {
    controller_gateway_url: String,
    controller_credential: String,
    runtime_host_code: String,
    name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OnboardResponse {
    host_id: String,
    host_name: String,
}

pub(crate) async fn onboard(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<OnboardRequest>,
) -> Result<Json<OnboardResponse>, ApiError> {
    require_human_operator(&headers, &state).await?;
    validate(&payload)?;
    let binary = connector_binary()?;
    let config = connector_config_path(
        &payload.controller_gateway_url,
        &payload.controller_credential,
        &payload.name,
    )?;
    if let Some(parent) = config.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| internal(format!("create connector directory: {error}")))?;
    }
    let mut child = Command::new(&binary)
        .arg("pair-relay")
        .arg("--config")
        .arg(&config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| internal(format!("start connector pairing: {error}")))?;
    let request = serde_json::json!({
        "gateway_url": payload.controller_gateway_url,
        "credential": payload.controller_credential,
        "runtime_host_code": payload.runtime_host_code,
        "name": payload.name,
    });
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| internal("connector pairing stdin is unavailable".into()))?;
    stdin
        .write_all(request.to_string().as_bytes())
        .await
        .map_err(|error| internal(format!("send connector pairing request: {error}")))?;
    drop(stdin);
    let output = tokio::time::timeout(PAIR_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| internal("connector pairing timed out".into()))?
        .map_err(|error| internal(format!("wait for connector pairing: {error}")))?;
    if !output.status.success() {
        let _ = tokio::fs::remove_file(&config).await;
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        return Err(internal(format!(
            "connector pairing failed: {}",
            diagnostic.lines().last().unwrap_or("unknown error")
        )));
    }
    let paired: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| internal(format!("decode connector pairing result: {error}")))?;
    let response = OnboardResponse {
        host_id: paired["host_id"]
            .as_str()
            .ok_or_else(|| internal("connector did not return a host id".into()))?
            .to_owned(),
        host_name: paired["host_name"]
            .as_str()
            .ok_or_else(|| internal("connector did not return a host name".into()))?
            .to_owned(),
    };
    Ok(Json(response))
}

fn validate(payload: &OnboardRequest) -> Result<(), ApiError> {
    let valid_token = |value: &str| {
        value.len() == 22
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    };
    let mut parts = payload.controller_credential.trim().split('.');
    if !matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some("v1"), Some(id), Some(secret), None) if valid_token(id) && valid_token(secret)
    ) {
        return Err(ApiError(AppError::Validation(
            "controller_credential is malformed".into(),
        )));
    }
    if payload.runtime_host_code.len() != 8
        || !payload
            .runtime_host_code
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(ApiError(AppError::Validation(
            "runtime_host_code must contain exactly 8 digits".into(),
        )));
    }
    if payload.name.trim().is_empty()
        || payload.name.len() > 80
        || payload.name.chars().any(char::is_control)
    {
        return Err(ApiError(AppError::Validation(
            "name must contain between 1 and 80 printable characters".into(),
        )));
    }
    let url = reqwest::Url::parse(payload.controller_gateway_url.trim()).map_err(|_| {
        ApiError(AppError::Validation(
            "controller_gateway_url is invalid".into(),
        ))
    })?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err(ApiError(AppError::Validation(
            "controller_gateway_url must be an HTTP or HTTPS URL".into(),
        )));
    }
    Ok(())
}

fn connector_binary() -> Result<PathBuf, ApiError> {
    if let Some(path) = std::env::var_os("CHORUZ_CONNECTOR_BINARY").map(PathBuf::from) {
        return executable(path);
    }
    if let Ok(current) = std::env::current_exe()
        && let Some(parent) = current.parent()
    {
        let sibling = parent.join("choruz-connector");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    Ok(PathBuf::from("choruz-connector"))
}

fn executable(path: PathBuf) -> Result<PathBuf, ApiError> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(internal(format!(
            "CHORUZ_CONNECTOR_BINARY does not point to a file: {}",
            path.display()
        )))
    }
}

fn connector_config_path(
    gateway_url: &str,
    credential: &str,
    name: &str,
) -> Result<PathBuf, ApiError> {
    let directory =
        choruz_supervisor::connectors::connector_config_directory().map_err(internal)?;
    let mut hash = Sha256::new();
    hash.update(gateway_url.as_bytes());
    hash.update(b"\0");
    hash.update(credential.as_bytes());
    hash.update(b"\0");
    hash.update(name.as_bytes());
    let key = hex::encode(hash.finalize());
    Ok(directory.join(format!("{}.json", &key[..24])))
}

fn internal(message: String) -> ApiError {
    ApiError(AppError::Internal(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> OnboardRequest {
        OnboardRequest {
            controller_gateway_url: "https://gateway.example".into(),
            controller_credential: "v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB".into(),
            runtime_host_code: "12345678".into(),
            name: "Build server".into(),
        }
    }

    #[test]
    fn onboarding_requires_both_independent_credentials() {
        assert!(validate(&request()).is_ok());
        let mut invalid = request();
        invalid.runtime_host_code = "1234".into();
        assert!(validate(&invalid).is_err());
        let mut invalid = request();
        invalid.controller_credential = "12345678".into();
        assert!(validate(&invalid).is_err());
    }

    #[test]
    fn config_name_is_stable_without_exposing_credentials() {
        let credential = "v1.AAAAAAAAAAAAAAAAAAAAAA.BBBBBBBBBBBBBBBBBBBBBB";
        let first =
            connector_config_path("https://gateway.example", credential, "Builder").unwrap();
        let second =
            connector_config_path("https://gateway.example", credential, "Builder").unwrap();
        assert_eq!(first, second);
        assert!(!first.to_string_lossy().contains("gateway.example"));
        assert!(!first.to_string_lossy().contains("AAAAAAAA"));
        assert_ne!(
            first,
            connector_config_path(
                "https://gateway.example",
                "v1.CCCCCCCCCCCCCCCCCCCCCC.DDDDDDDDDDDDDDDDDDDDDD",
                "Builder",
            )
            .unwrap()
        );
    }
}
