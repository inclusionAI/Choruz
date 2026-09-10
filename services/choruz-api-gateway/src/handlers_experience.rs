use crate::{
    ApiError, ApiState, handlers_terminals::authorize_terminal_binding, require_human_operator,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use choruz_common::AppError;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationRequest {
    revision_id: String,
    suite: choruz_domain::evaluation::EvaluationSuite,
    optimization: Option<choruz_domain::optimization::OptimizationConfig>,
}

pub(crate) async fn evaluate(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<EvaluationRequest>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    let spec = crate::handlers_terminals::terminal_spec(&binding, 120, 40, None, None);
    let fingerprint = crate::evaluation_worker::fingerprint(&spec)?;
    let optimization = if let Some(config) = body.optimization {
        let policy = state
            .db
            .experience_policy(&conversation.workspace_id, &actor.id, &id)
            .await?
            .ok_or_else(|| AppError::NotFound("Learning policy not found".into()))?;
        let analyst =
            authorize_terminal_binding(&state, &actor, &policy.analyst_binding_id).await?;
        Some(choruz_application::db_service::OptimizationSetup {
            config,
            analyst_binding_id: policy.analyst_binding_id,
            analyst_fingerprint: crate::evaluation_worker::analyst_fingerprint(
                &crate::handlers_terminals::terminal_spec(&analyst, 120, 40, None, None),
            )?,
        })
    } else {
        None
    };
    let run_id = state
        .db
        .queue_experience_evaluation(
            &conversation.workspace_id,
            &actor.id,
            &id,
            &body.revision_id,
            &body.suite,
            &choruz_application::db_service::EvaluationContext {
                automatic_generation: None,
                fingerprint,
                optimization,
            },
        )
        .await?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({"id":run_id,"status":"queued"})),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationQuery {
    id: Option<String>,
}

pub(crate) async fn evaluations(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<EvaluationQuery>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    Ok(Json(
        json!({"evaluations":state.db.experience_evaluations(&conversation.workspace_id,&actor.id,&id,query.id.as_deref()).await?}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    enabled: bool,
    analyst_binding_id: String,
    optimization_settings: Option<choruz_domain::optimization::OptimizationSettings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    #[serde(deserialize_with = "Option::deserialize")]
    revision_id: Option<String>,
}

pub(crate) async fn select(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<Selection>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let target = authorize_terminal_binding(&state, &actor, &id).await?;
    let conversation = state.db.get_conversation(&target.conversation_id).await?;
    if !matches!(
        target.driver_type.as_str(),
        "claude_terminal" | "codex_terminal"
    ) {
        return Err(AppError::Validation(
            "Experience learning requires a Claude Code or Codex Agent".into(),
        )
        .into());
    }
    state
        .db
        .select_experience_revision(
            &conversation.workspace_id,
            &actor.id,
            &id,
            body.revision_id.as_deref(),
            None,
        )
        .await?;
    state
        .db
        .record_audit(
            &conversation.workspace_id,
            &actor.id,
            "experience.select_revision",
            "runtime_binding",
            &id,
            json!({"revision_id":body.revision_id}),
        )
        .await?;
    get(headers, State(state), Path(id)).await
}

pub(crate) async fn get(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    let policy = state
        .db
        .experience_policy(&conversation.workspace_id, &actor.id, &id)
        .await?;
    let revisions = state
        .db
        .experience_revisions(&conversation.workspace_id, &actor.id, &id)
        .await?;
    Ok(Json(json!({"policy": policy, "revisions": revisions})))
}

pub(crate) async fn configure(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<Settings>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let target = authorize_terminal_binding(&state, &actor, &id).await?;
    let analyst = authorize_terminal_binding(&state, &actor, &body.analyst_binding_id).await?;
    if !matches!(
        target.driver_type.as_str(),
        "claude_terminal" | "codex_terminal"
    ) {
        return Err(AppError::Validation(
            "Experience learning requires a Claude Code or Codex Agent".into(),
        )
        .into());
    }
    let conversation = state.db.get_conversation(&target.conversation_id).await?;
    if !matches!(
        analyst.driver_type.as_str(),
        "claude_terminal" | "codex_terminal"
    ) {
        return Err(
            AppError::Validation("Select a Claude Code or Codex analysis Agent".into()).into(),
        );
    }
    if let Some(settings) = &body.optimization_settings {
        settings.validate().map_err(AppError::Validation)?;
        crate::evaluation_worker::fingerprint(&crate::handlers_terminals::terminal_spec(
            &target, 120, 40, None, None,
        ))?;
        crate::evaluation_worker::analyst_fingerprint(&crate::handlers_terminals::terminal_spec(
            &analyst, 120, 40, None, None,
        ))?;
    }
    state
        .db
        .configure_experience(
            &conversation.workspace_id,
            &actor.id,
            &id,
            &analyst.id,
            body.enabled,
            body.optimization_settings.as_ref(),
        )
        .await?;
    state
        .db
        .record_audit(
            &conversation.workspace_id,
            &actor.id,
            "experience.configure",
            "runtime_binding",
            &id,
            json!({"enabled": body.enabled, "analyst_binding_id": analyst.id}),
        )
        .await?;
    get(headers, State(state), Path(id)).await
}
