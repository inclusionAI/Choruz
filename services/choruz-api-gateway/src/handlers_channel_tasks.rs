use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use choruz_common::AppError;
use serde::de::DeserializeOwned;

use crate::{ApiError, ApiState, authenticated_principal};

pub(crate) async fn list_channel_tasks(
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Response, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    Ok(Json(
        state
            .db
            .list_channel_tasks(&principal.id, &conversation_id)
            .await?,
    )
    .into_response())
}

pub(crate) async fn create_channel_task(
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    State(state): State<ApiState>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let payload = parse_json_body::<choruz_application::CreateChannelTaskRequest>(&body)?;
    let (created, task) = state
        .db
        .create_channel_task(&principal.id, &conversation_id, payload)
        .await?;
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(task),
    )
        .into_response())
}

pub(crate) async fn create_channel_task_from_message(
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
    State(state): State<ApiState>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let payload =
        parse_json_body::<choruz_application::CreateChannelTaskFromMessageRequest>(&body)?;
    let (created, task) = state
        .db
        .create_channel_task_from_message(&principal.id, &conversation_id, payload)
        .await?;
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(task),
    )
        .into_response())
}

pub(crate) async fn get_channel_task(
    headers: HeaderMap,
    Path(task_id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Response, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    Ok(Json(
        state
            .db
            .get_channel_task_detail(&principal.id, &task_id)
            .await?,
    )
    .into_response())
}

pub(crate) async fn patch_channel_task(
    headers: HeaderMap,
    Path(task_id): Path<String>,
    State(state): State<ApiState>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    let payload = parse_json_body::<choruz_application::PatchChannelTaskRequest>(&body)?;
    Ok(Json(
        state
            .db
            .patch_channel_task(&principal.id, &task_id, payload)
            .await?,
    )
    .into_response())
}

fn parse_json_body<T>(body: &[u8]) -> Result<T, ApiError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(body)
        .map_err(|e| ApiError(AppError::Validation(format!("invalid JSON body: {e}"))))
}
