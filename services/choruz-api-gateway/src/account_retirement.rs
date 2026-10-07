use crate::{
    ApiError, ApiState, handlers_companies::require_company_access, host_runtime::RuntimeHost,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use choruz_application::db_service::AccountRetirement;
use choruz_common::AppError;
use serde_json::{Value, json};

pub(crate) async fn remove(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company, account)): Path<(String, String)>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let actor = crate::auth::require_human_operator(&headers, &state).await?;
    require_company_access(&headers, &state, &company).await?;
    let request = state
        .db
        .request_account_retirement(&company, &account, &actor.id)
        .await?;
    let count = retire(&state, &request).await?;
    Ok((
        if count.is_some() {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        },
        Json(json!({"disabled_bindings":count,"removal_pending":count.is_none()})),
    ))
}

async fn retire(state: &ApiState, request: &AccountRetirement) -> Result<Option<i64>, AppError> {
    let stopped = async {
        let host = match request.runtime_host_id.as_deref() {
            Some(id) => RuntimeHost::for_host(state, id)?,
            None => RuntimeHost::local(state),
        };
        host.close_account(&request.account_id, &request.binding_ids)
            .await
    }
    .await;
    if let Err(error) = stopped {
        tracing::warn!(account_id=%request.account_id,company_id=%request.workspace_id,%error,"account removal awaiting device cleanup");
        state.db.account_retirement_error(request,"Waiting for the device to stop account processes. Reconnect or update the device to continue.").await?;
        return Ok(None);
    }
    state.db.finish_account_retirement(request).await
}

pub(crate) async fn run(state: &ApiState) {
    let mut timer = tokio::time::interval(std::time::Duration::from_secs(2));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        match state.db.pending_account_retirements().await {
            Ok(requests) => {
                for request in requests {
                    if let Err(error) = retire(state, &request).await {
                        tracing::warn!(account_id=%request.account_id,%error,"account removal could not complete");
                    }
                }
            }
            Err(error) => tracing::warn!(%error,"account removal queue unavailable"),
        }
    }
}
