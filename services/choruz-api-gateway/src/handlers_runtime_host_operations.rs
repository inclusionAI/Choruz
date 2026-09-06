//! `POST /v1/runtime-hosts/{host_id}/operations`: one request/response
//! operation on a paired device, answered over its host link.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use choruz_common::AppError;
use choruz_host_runtime::HostRequest;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    ApiError, ApiState, handlers_companies::require_company_access, host_runtime::RuntimeHost,
    require_human_operator,
};

#[derive(Debug, Deserialize)]
pub(crate) struct CreateOperationRequest {
    kind: String,
    #[serde(default)]
    request: Value,
}

pub(crate) async fn create_operation(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(host_id): Path<String>,
    Json(payload): Json<CreateOperationRequest>,
) -> Result<Json<Value>, ApiError> {
    require_human_operator(&headers, &state).await?;
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let company_id = client
        .query_opt(
            "SELECT company_id FROM runtime_host WHERE id = $1 AND revoked_at IS NULL",
            &[&host_id],
        )
        .await
        .map_err(|error| ApiError(AppError::Internal(format!("find runtime host: {error}"))))?
        .map(|row| row.get::<_, String>(0))
        .ok_or_else(|| ApiError(AppError::NotFound(format!("runtime host {host_id}"))))?;
    drop(client);
    require_company_access(&headers, &state, &company_id).await?;
    let mut request = host_request(&payload.kind, payload.request)?;
    if let HostRequest::ScanSessions { accounts, .. } = &mut request {
        *accounts = crate::handlers_workspace_sessions::session_accounts(
            &state,
            Some(&company_id),
            Some(&host_id),
        )
        .await?;
    }
    RuntimeHost::for_host(&state, &host_id)?
        .call(request)
        .await
        .map(Json)
        .map_err(ApiError)
}

/// The operations a human may run on a device from the dashboard:
/// browsing, session scans and workspace provisioning.
fn host_request(kind: &str, request: Value) -> Result<HostRequest, ApiError> {
    let text = |key: &str| {
        request[key]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| ApiError(AppError::Validation(format!("{kind} requires {key}"))))
    };
    match kind {
        "drivers.inspect" => Ok(HostRequest::DriverCatalog {
            driver_type: request["driver_type"].as_str().map(str::to_owned),
        }),
        "filesystem.home" => Ok(HostRequest::FilesystemHome),
        "filesystem.list" => Ok(HostRequest::FilesystemList {
            path: text("path")?,
            show_hidden: request["show_hidden"].as_bool().unwrap_or(false),
            include_files: request["include_files"].as_bool().unwrap_or(false),
        }),
        "workspace_sessions.scan" => Ok(HostRequest::ScanSessions {
            accounts: Vec::new(),
            workspace_path: text("workspace_path")?,
            harnesses: serde_json::from_value(request["harnesses"].clone()).map_err(|error| {
                ApiError(AppError::Validation(format!(
                    "invalid Harness selection: {error}"
                )))
            })?,
        }),
        "workspace.provision" => Ok(HostRequest::ProvisionWorkspace {
            workspace_path: request["workspace_path"].as_str().map(str::to_owned),
            name: text("name")?,
            files: match request.get("files") {
                Some(Value::Null) | None => Vec::new(),
                Some(files) => serde_json::from_value(files.clone()).map_err(|error| {
                    ApiError(AppError::Validation(format!(
                        "invalid workspace files: {error}"
                    )))
                })?,
            },
        }),
        _ => Err(ApiError(AppError::Validation(
            "unsupported runtime host operation".into(),
        ))),
    }
}
