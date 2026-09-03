use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::HeaderMap,
};
use choruz_common::AppError;
use serde::Serialize;

use crate::handlers_runtime::accessible_workspace_ids;
use crate::{ApiError, ApiState, require_human_operator};

// ── Response types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AgentTask {
    id: String,
    subject: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentTasksResponse {
    agent_id: String,
    driver_type: String,
    tasks: Vec<AgentTask>,
}

// ── Handler ──────────────────────────────────────────────────────────

pub(crate) async fn get_agent_tasks(
    headers: HeaderMap,
    AxumPath(agent_id): AxumPath<String>,
    State(state): State<ApiState>,
) -> Result<Json<AgentTasksResponse>, ApiError> {
    let operator = require_human_operator(&headers, &state).await?;
    let allowed_ws = accessible_workspace_ids(&state.db, &operator.id).await?;

    // Find the binding for this agent (for driver_type + access check)
    let bindings = state.runtime.list_bindings_by_agent(&agent_id).await?;
    let binding = match bindings.into_iter().next() {
        Some(b) => b,
        None => {
            return Ok(Json(AgentTasksResponse {
                agent_id,
                driver_type: "unknown".into(),
                tasks: vec![],
            }));
        }
    };

    // Verify workspace access
    let agent = state.db.get_principal(&agent_id).await?;
    if !allowed_ws.contains(&agent.workspace_id) {
        return Ok(Json(AgentTasksResponse {
            agent_id,
            driver_type: binding.driver_type.as_str().into(),
            tasks: vec![],
        }));
    }

    // Read tasks from the agent_task table
    let client = state
        .event_store
        .connect()
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("query tasks: {e}"))))?;
    let rows = client
        .query(
            "SELECT id, subject, description, status, owner FROM agent_task \
             WHERE agent_id = $1 ORDER BY id",
            &[&agent_id],
        )
        .await
        .map_err(|e| ApiError::from(AppError::Internal(format!("query tasks: {e}"))))?;

    let tasks: Vec<AgentTask> = rows
        .iter()
        .map(|r| AgentTask {
            id: r.get(0),
            subject: r.get(1),
            description: r.get(2),
            status: r.get(3),
            owner: r.get(4),
        })
        .collect();

    Ok(Json(AgentTasksResponse {
        agent_id,
        driver_type: binding.driver_type.as_str().into(),
        tasks,
    }))
}
