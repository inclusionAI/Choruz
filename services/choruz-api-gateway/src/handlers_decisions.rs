use crate::{
    ApiError, ApiState, handlers_terminals::authorize_terminal_binding, host_runtime::RuntimeHost,
    require_human_operator,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use choruz_common::AppError;
use choruz_decision::task::DecisionTask;
use choruz_host_runtime::HostRequest;
use serde::Deserialize;
use serde_json::{Value, json};

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
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    state
        .db
        .select_decision_program(
            &conversation.workspace_id,
            &actor.id,
            &id,
            body.revision_id.as_deref(),
        )
        .await?;
    state
        .db
        .record_audit(
            &conversation.workspace_id,
            &actor.id,
            "decision.select",
            "runtime_binding",
            &id,
            json!({"revision_id":body.revision_id}),
        )
        .await?;
    crate::handlers_experience::get(headers, State(state), Path(id)).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Execution {
    input: String,
}

pub(crate) async fn execute(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<Execution>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    if body.input.trim().is_empty() || body.input.len() > 16000 {
        return Err(
            AppError::Validation("Program input must contain 1 to 16000 bytes".into()).into(),
        );
    }
    let workspace = state
        .db
        .get_conversation(&binding.conversation_id)
        .await?
        .workspace_id;
    let (revision, trial) = state
        .db
        .selected_decision_program(&workspace, &actor.id, &id)
        .await?;
    let program: choruz_decision::programs::Program =
        serde_json::from_value(trial["program"].clone())
            .map_err(|_| AppError::Internal("Stored program is invalid".into()))?;
    let model = trial["resolved_model"]
        .as_str()
        .ok_or_else(|| AppError::Conflict("Program has no tested model identity".into()))?;
    state
        .db
        .record_audit(
            &workspace,
            &actor.id,
            "decision.execution_requested",
            "runtime_binding",
            &id,
            json!({"revision_id":revision,"model":model}),
        )
        .await?;
    let started = std::time::Instant::now();
    let result: Result<choruz_decision::programs::ProgramResult, AppError> = async {
        let result: choruz_decision::programs::ProgramResult =
            RuntimeHost::for_binding(&state, &binding)?
                .call(HostRequest::Decision {
                    request: DecisionTask {
                        model: model.into(),
                        minimum_confidence: program.minimum_confidence,
                        job: choruz_decision::task::Job::Program {
                            program,
                            state: json!(body.input),
                        },
                    },
                })
                .await?;
        if result.decision.model != model {
            return Err(AppError::Conflict(
                "Provider model changed; revalidate the program before use".into(),
            ));
        }
        // A concurrent disable or selection change must not publish the old result.
        let (current, _) = state
            .db
            .selected_decision_program(&workspace, &actor.id, &id)
            .await?;
        if current != revision {
            return Err(AppError::Conflict(
                "Selected program changed during execution".into(),
            ));
        }
        Ok(result)
    }
    .await;
    let outcome = match &result {
        Ok(result) => {
            json!({"outcome":"completed","abstained":result.output.is_none(),"model":result.decision.model,"usage":result.decision.usage})
        }
        Err(error) => crate::experience_diagnostics::failure(error),
    };
    state.db.record_audit(&workspace, &actor.id, "decision.execution_finished", "runtime_binding", &id, json!({"revision_id":revision,"duration_ms":started.elapsed().as_millis(),"result":outcome})).await?;
    let result = result?;
    Ok(Json(json!({"revision_id":revision,"result":result})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    #[serde(deserialize_with = "Option::deserialize")]
    settings: Option<choruz_decision::settings::LearningSettings>,
}

pub(crate) async fn configure(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<Settings>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let target = authorize_terminal_binding(&state, &actor, &id).await?;
    if let Some(builder) = body
        .settings
        .as_ref()
        .and_then(|s| s.builder_binding_id.as_deref())
    {
        let binding = authorize_terminal_binding(&state, &actor, builder).await?;
        if binding.driver_type.as_str() != "codex_terminal" {
            return Err(AppError::Validation("Select a Codex program builder".into()).into());
        }
    }
    let conversation = state.db.get_conversation(&target.conversation_id).await?;
    state
        .db
        .configure_decisions(
            &conversation.workspace_id,
            &actor.id,
            &id,
            body.settings.as_ref(),
        )
        .await?;
    state
        .db
        .record_audit(
            &conversation.workspace_id,
            &actor.id,
            "decision.configure",
            "runtime_binding",
            &id,
            json!({"settings":body.settings}),
        )
        .await?;
    crate::handlers_experience::get(headers, State(state), Path(id)).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    transmit_to_typesafe: bool,
    request: DecisionTask,
}

pub(crate) async fn run(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<Request>,
) -> Result<Json<Value>, ApiError> {
    let actor = require_human_operator(&headers, &state).await?;
    let binding = authorize_terminal_binding(&state, &actor, &id).await?;
    if !body.transmit_to_typesafe {
        return Err(AppError::Forbidden(
            "Decision inference requires permission to send the supplied state to TypeSafe".into(),
        )
        .into());
    }
    let conversation = state.db.get_conversation(&binding.conversation_id).await?;
    let kind = body.request.kind();
    let model = body.request.model.clone();
    state
        .db
        .record_audit(
            &conversation.workspace_id,
            &actor.id,
            "decision.requested",
            "runtime_binding",
            &id,
            json!({"kind":kind,"model":model}),
        )
        .await?;
    let started = std::time::Instant::now();
    let result = RuntimeHost::for_binding(&state, &binding)?
        .call::<Value>(HostRequest::Decision {
            request: body.request,
        })
        .await;
    state.db.record_audit(&conversation.workspace_id, &actor.id, "decision.finished", "runtime_binding", &id, json!({"kind":kind,"success":result.is_ok(),"duration_ms":started.elapsed().as_millis() as u64})).await?;
    tracing::info!(binding_id=%id, kind, success=result.is_ok(), "structured decision finished");
    result.map(Json).map_err(ApiError)
}
