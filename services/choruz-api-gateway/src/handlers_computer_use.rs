use crate::{
    ApiError, ApiState, handlers_companies::require_company_access, host_runtime::RuntimeHost,
    require_human_operator,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use choruz_common::AppError;
use choruz_host_runtime::{HostRequest, computer_use::Tool};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    runtime_host_id: Option<String>,
    tool: Option<Tool>,
    enabled: Option<bool>,
}

pub(crate) async fn manage(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
    Json(body): Json<Request>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    require_company_access(&headers, &state, &company_id).await?;
    let host = if let Some(id) = body.runtime_host_id.as_deref() {
        let client = state.event_store.connect().await.map_err(ApiError::from)?;
        let exists = client.query_opt("SELECT id FROM runtime_host WHERE id = $1 AND company_id = $2 AND revoked_at IS NULL", &[&id, &company_id]).await.map_err(|error| ApiError(AppError::Internal(format!("find device: {error}"))))?;
        if exists.is_none() {
            return Err(ApiError(AppError::NotFound(
                "device in this company".into(),
            )));
        }
        RuntimeHost::for_host(&state, id)?
    } else {
        RuntimeHost::local(&state)
    };
    tracing::info!(company_id, runtime_host_id = ?body.runtime_host_id, tool = ?body.tool, enabled = ?body.enabled, "computer-use management");
    if body.tool.is_some() && body.enabled.is_some() {
        state.db.record_audit(&actor.workspace_id, &actor.id, "computer_use.configure_requested", "runtime_host", body.runtime_host_id.as_deref().unwrap_or("local"), serde_json::json!({"company_id": company_id, "tool": body.tool, "enabled": body.enabled})).await?;
    }
    host.call(HostRequest::ComputerUse {
        tool: body.tool,
        enabled: body.enabled,
    })
    .await
    .map(Json)
    .map_err(ApiError)
}
