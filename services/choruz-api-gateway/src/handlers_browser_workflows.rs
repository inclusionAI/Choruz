//! Human-authorized bounded runs reuse device dispatch and durable admissions.
//! Request values stay transient; the audit holds only identity and outcome.
use crate::{ApiError, ApiState, authenticated_principal, host_runtime::RuntimeHost};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use choruz_common::AppError;
use choruz_host_runtime::{HostRequest, browser_workflow::Execution};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) fn device_id(workspace: &str, binding: &str, id: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{workspace}\0{binding}\0{id}"))
    )
}

pub(crate) fn binding_fingerprint(binding: &choruz_agent_runtime::RuntimeBinding) -> String {
    let identity = json!({"agent":binding.agent_principal_id,"driver":binding.driver_type,
        "workspace":binding.workspace_path,"host":binding.config_json["runtime_host_id"],
        "account":binding.config_json["harness_account_id"],"profile":binding.config_json["harness_account_profile_kind"],
        "binary":binding.config_json["binary_path"],"model":binding.config_json["model"]});
    format!("{:x}", Sha256::digest(identity.to_string()))
}

pub(crate) fn request_hash(
    secret: &str,
    execution: &impl serde::Serialize,
) -> Result<String, AppError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| AppError::Internal("Cannot authenticate browser admission".into()))?;
    mac.update(b"choruz/browser-workflow/admission/v1\0");
    mac.update(
        &serde_json::to_vec(execution).map_err(|error| AppError::Validation(error.to_string()))?,
    );
    Ok(hex::encode(mac.finalize().into_bytes()))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn launch(
    worker_state: ApiState,
    host: RuntimeHost,
    worker_workspace: String,
    worker_binding: String,
    worker_id: String,
    actor_id: String,
    execution: Execution,
    assistant: Option<Box<choruz_host_runtime::TerminalSpec>>,
) -> Result<(), AppError> {
    let Some(expires_at) = worker_state
        .db
        .claim_browser_dispatch(&worker_workspace, &worker_binding, &worker_id)
        .await?
    else {
        return Ok(());
    };
    tokio::spawn(async move {
        let result = host
            .call::<Value>(HostRequest::BrowserWorkflow {
                id: device_id(&worker_workspace, &worker_binding, &worker_id),
                expires_at,
                request: execution,
                assistant,
            })
            .await;
        let result = match result {
            Ok(report) => json!({"report":report}),
            Err(_) => {
                json!({"error":"Execution unavailable or interrupted; inspect any browser effects before starting a new run"})
            }
        };
        if let Err(error) = worker_state
            .db
            .finish_browser_workflow(&worker_workspace, &worker_binding, &worker_id, &result)
            .await
        {
            tracing::warn!(run_id=%worker_id,%error,"browser workflow result persistence failed");
        }
        let _ = worker_state
            .db
            .record_audit(
                &worker_workspace,
                &actor_id,
                "browser_workflow.finished",
                "runtime_binding",
                &worker_binding,
                json!({"run_id":worker_id,"result":result}),
            )
            .await;
    });
    Ok(())
}

pub(crate) async fn get(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((binding_id, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let actor = authenticated_principal(&headers, &state).await?;
    let binding = crate::handlers_browser_automation::access(&state, &actor, &binding_id).await?;
    let workspace = state
        .db
        .get_conversation(&binding.conversation_id)
        .await?
        .workspace_id;
    Ok(Json(
        state
            .db
            .browser_workflow_run(&workspace, &binding_id, &id)
            .await?,
    ))
}

pub(crate) async fn cancel(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((binding_id, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let actor = authenticated_principal(&headers, &state).await?;
    let binding = crate::handlers_browser_automation::access(&state, &actor, &binding_id).await?;
    let workspace = state
        .db
        .get_conversation(&binding.conversation_id)
        .await?
        .workspace_id;
    let existing = state
        .db
        .browser_workflow_run(&workspace, &binding_id, &id)
        .await?;
    if existing["status"] == "finished" {
        return Ok(Json(existing));
    }
    state
        .db
        .cancel_browser_workflow(&workspace, &binding_id, &id)
        .await?;
    let acknowledged = match RuntimeHost::for_binding(&state, &binding) {
        Ok(host) => host
            .call::<Value>(HostRequest::CancelBrowserWorkflow {
                id: device_id(&workspace, &binding_id, &id),
            })
            .await
            .is_ok(),
        Err(_) => false,
    };
    state
        .db
        .record_audit(
            &workspace,
            &actor.id,
            "browser_workflow.cancel_requested",
            "runtime_binding",
            &binding_id,
            json!({"run_id":id,"device_acknowledged":acknowledged}),
        )
        .await?;
    Ok(Json(
        json!({"run":state.db.browser_workflow_run(&workspace,&binding_id,&id).await?,"device_acknowledged":acknowledged}),
    ))
}
