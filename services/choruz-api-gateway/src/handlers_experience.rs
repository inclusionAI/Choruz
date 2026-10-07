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
pub(crate) struct PrepareRequest {
    input: String,
}

pub(crate) async fn prepare(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<PrepareRequest>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    if body.input.trim().is_empty() || body.input.len() > 256 * 1024 {
        return Err(
            AppError::Validation("Turn input must be nonempty and at most 256 KiB".into()).into(),
        );
    }
    if !matches!(
        binding.driver_type,
        choruz_agent_runtime::DriverType::ClaudeTerminal
            | choruz_agent_runtime::DriverType::CodexTerminal
    ) {
        return Err(
            AppError::Validation("Native learning supports Claude Code and Codex".into()).into(),
        );
    }
    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    let policy = state
        .db
        .experience_policy(&conversation.workspace_id, &actor.id, &id)
        .await?
        .ok_or_else(|| {
            AppError::Forbidden("Configure an owned learning policy before preparing a turn".into())
        })?;
    let context = state
        .db
        .experience_for_turn(&conversation.workspace_id, &id)
        .await?;
    let mut prompt = body.input;
    if let Some(context) = &context
        && let Some(team) = &context.team
    {
        let host = crate::host_runtime::RuntimeHost::for_binding(&state, &binding)?;
        let findings: String = host
            .call(choruz_host_runtime::HostRequest::PrepareExecutionTeam {
                spec: Box::new(crate::handlers_terminals::terminal_spec(
                    &binding, 120, 40, None, None,
                )),
                role: choruz_host_runtime::harness::ExecutionTeam {
                    revision_id: context.revision_id.clone(),
                    team: team.clone(),
                },
                request: prompt.clone(),
            })
            .await?;
        prompt.push_str("\n\n");
        prompt.push_str(&findings);
    }
    // A paid collaborator call can overlap revocation or account/device changes.
    // Preparation must not export guidance under a superseded selection.
    let current = authorize_terminal_binding(&state, &actor, &id).await?;
    let current_policy = state
        .db
        .experience_policy(&conversation.workspace_id, &actor.id, &id)
        .await?;
    if current.updated_at != binding.updated_at
        || current_policy.as_ref().is_none_or(|current| {
            current.generation != policy.generation
                || (current
                    .enabled
                    .then_some(current.active_revision_id.as_deref())
                    .flatten()
                    != context.as_ref().map(|context| context.revision_id.as_str()))
        })
    {
        return Err(AppError::Conflict("Learning or binding changed during preparation; prepare the next turn with the current selection".into()).into());
    }
    let experience = context.map(|context| (context.revision_id, context.instruction));
    let revision = experience.as_ref().map(|(revision, _)| revision);
    state
        .db
        .record_audit(
            &conversation.workspace_id,
            &actor.id,
            "learning.prepare",
            "agent_binding",
            &id,
            json!({"revision_id":revision,"submitted":false}),
        )
        .await?;
    Ok(Json(
        json!({"prompt":choruz_agent_runtime::headless::with_experience(prompt, experience.as_ref()),"revision_id":revision}),
    ))
}

pub(crate) async fn community(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    let id = state
        .db
        .learning_binding_id(&conversation.workspace_id, &id, Some(&actor.id))
        .await?;
    let mut result = state
        .db
        .behavior_community(&conversation.workspace_id, &actor.id, &id)
        .await?;
    result["publisher_configured"] = json!(
        std::env::var("CHORUZ_COMMUNITY_HF_TOKEN").is_ok_and(|token| !token.trim().is_empty())
    );
    Ok(Json(result))
}

pub(crate) async fn configure_community(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(settings): Json<choruz_community::behavior::CommunitySettings>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    let id = state
        .db
        .learning_binding_id(&conversation.workspace_id, &id, Some(&actor.id))
        .await?;
    state
        .db
        .configure_behavior_community(&conversation.workspace_id, &actor.id, &id, &settings)
        .await?;
    state
        .db
        .record_audit(
            &conversation.workspace_id,
            &actor.id,
            "learning.community.configure",
            "agent_binding",
            &id,
            json!(settings),
        )
        .await?;
    Ok(Json(json!(settings)))
}

pub(crate) async fn retry_community(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((id, event)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    state
        .db
        .retry_behavior(&conversation.workspace_id, &actor.id, &id, &event)
        .await?;
    state
        .db
        .record_audit(
            &conversation.workspace_id,
            &actor.id,
            "learning.community.retry",
            "behavior_event",
            &event,
            json!({}),
        )
        .await?;
    Ok(Json(json!({"queued":true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationRequest {
    revision_id: String,
    suite: choruz_evaluation::evaluation::EvaluationSuite,
    optimization: Option<choruz_evaluation::optimization::OptimizationConfig>,
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
    optimization_settings: Option<choruz_evaluation::optimization::OptimizationSettings>,
    target_model: Option<String>,
    analyst_model: Option<String>,
    #[serde(default)]
    reuse_for_new_tasks: bool,
}

async fn idle_model_host(
    state: &ApiState,
    binding: &choruz_agent_runtime::RuntimeBinding,
    selected: Option<&str>,
) -> Result<Option<crate::host_runtime::RuntimeHost>, AppError> {
    let Some(_) =
        selected.filter(|model| binding.config_json["model"].as_str() != Some(model.trim()))
    else {
        return Ok(None);
    };
    if binding.in_flight_turn_id.is_some()
        || binding.state == choruz_agent_runtime::BindingState::Running
    {
        return Err(AppError::Conflict(
            "Stop the Agent before changing its model".into(),
        ));
    }
    let host = crate::host_runtime::RuntimeHost::for_binding(state, binding)?;
    match host
        .session(choruz_host_runtime::session::SessionRequest::Read {
            binding_id: binding.id.clone(),
            owner: crate::handlers_sessions::binding_owner(binding),
            after: 0,
        })
        .await
    {
        Ok(snapshot)
            if !matches!(
                snapshot.status.as_str(),
                "ready" | "idle" | "closed" | "failed"
            ) || !snapshot.requests.is_empty() =>
        {
            return Err(AppError::Conflict(
                "Stop the Agent before changing its model".into(),
            ));
        }
        Ok(_) | Err(AppError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    Ok(Some(host))
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
    let id = state
        .db
        .learning_binding_id(&conversation.workspace_id, &id, Some(&actor.id))
        .await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    let policy = state
        .db
        .experience_policy(&conversation.workspace_id, &actor.id, &id)
        .await?;
    let revisions = state
        .db
        .experience_revisions(&conversation.workspace_id, &actor.id, &id)
        .await?;
    let cases = state
        .db
        .experience_trace_cases(&conversation.workspace_id, &id)
        .await?;
    let spec = crate::handlers_terminals::terminal_spec(&binding, 120, 40, None, None);
    let task_performance = if spec.model.as_ref().is_some_and(|m| !m.trim().is_empty()) {
        let context = crate::evaluation_worker::fingerprint(&spec)?;
        let performance = state
            .db
            .task_difficulty(&conversation.workspace_id, &id, &context, &cases)
            .await?;
        let tasks:Vec<_>=performance.into_iter().map(|(episode_ref,counts)|json!({"episode_ref":episode_ref,"samples":counts.samples,"successes":counts.successes,"band":counts.band()})).collect();
        json!({"context":context,"tasks":tasks})
    } else {
        Value::Null
    };
    Ok(Json(
        json!({"policy": policy, "revisions": revisions,"task_performance":task_performance}),
    ))
}

pub(crate) async fn configure(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<Settings>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let target = authorize_terminal_binding(&state, &actor, &id).await?;
    let workspace = state
        .db
        .get_conversation(&target.conversation_id)
        .await?
        .workspace_id;
    let id = state
        .db
        .learning_binding_id(&workspace, &id, Some(&actor.id))
        .await?;
    let target = authorize_terminal_binding(&state, &actor, &id).await?;
    let analyst = authorize_terminal_binding(&state, &actor, &body.analyst_binding_id).await?;
    if target.id == analyst.id {
        return Err(AppError::Validation(
            "Choose a different Agent for background analysis".into(),
        )
        .into());
    }
    for model in [&body.target_model, &body.analyst_model]
        .into_iter()
        .flatten()
    {
        choruz_agent_runtime::headless::validate_model(model)
            .map_err(|error| AppError::Validation(error.into()))?;
        if model.trim().is_empty() {
            return Err(AppError::Validation("Select an explicit model".into()).into());
        }
    }
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
        let mut target = target.clone();
        let mut analyst = analyst.clone();
        if let Some(model) = &body.target_model {
            target.config_json["model"] = json!(model.trim());
        }
        if let Some(model) = &body.analyst_model {
            analyst.config_json["model"] = json!(model.trim());
        }
        crate::evaluation_worker::fingerprint(&crate::handlers_terminals::terminal_spec(
            &target, 120, 40, None, None,
        ))?;
        crate::evaluation_worker::analyst_fingerprint(&crate::handlers_terminals::terminal_spec(
            &analyst, 120, 40, None, None,
        ))?;
    }
    let target_host = idle_model_host(&state, &target, body.target_model.as_deref()).await?;
    let analyst_host = idle_model_host(&state, &analyst, body.analyst_model.as_deref()).await?;
    let mut selections = Vec::new();
    for (host, binding, model) in [
        (target_host, &target, body.target_model.as_deref()),
        (analyst_host, &analyst, body.analyst_model.as_deref()),
    ] {
        if let (Some(host), Some(model)) = (host, model) {
            host.close_terminal(&binding.id).await?;
            selections.push((binding, model));
        }
    }
    if !selections.is_empty() {
        state.runtime.select_binding_models(&selections).await?;
    }
    state
        .db
        .configure_experience(
            &conversation.workspace_id,
            &actor.id,
            &target.id,
            &analyst.id,
            body.enabled,
            body.optimization_settings.as_ref(),
        )
        .await?;
    state
        .db
        .configure_task_profile(
            &conversation.workspace_id,
            &actor.id,
            &target.id,
            body.reuse_for_new_tasks,
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
            json!({"enabled": body.enabled, "analyst_binding_id": analyst.id, "reuse_for_new_tasks":body.reuse_for_new_tasks}),
        )
        .await?;
    get(headers, State(state), Path(id)).await
}
