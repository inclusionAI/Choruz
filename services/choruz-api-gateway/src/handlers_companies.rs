use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use choruz_common::AppError;
use serde::Deserialize;

use crate::{ApiError, ApiState, authenticated_principal, require_actor};

pub(crate) async fn require_company_access(
    headers: &HeaderMap,
    state: &ApiState,
    company_id: &str,
) -> Result<choruz_domain::Principal, ApiError> {
    let principal = authenticated_principal(headers, state).await?;
    let company = state.db.get_company(company_id).await?;
    if company.deleted_at.is_some() {
        return Err(ApiError(AppError::NotFound(format!(
            "company {company_id}"
        ))));
    }
    let companies = state.db.list_companies(&principal.id).await?;
    if companies.iter().any(|company| company.id == company_id) {
        Ok(principal)
    } else {
        Err(ApiError(AppError::Forbidden(
            "cannot read company from another workspace".into(),
        )))
    }
}

async fn require_conversation_export_access(
    state: &ApiState,
    actor: &choruz_domain::Principal,
    conversation_id: &str,
) -> Result<(), ApiError> {
    let client = state.event_store.connect().await.map_err(ApiError::from)?;
    let row = client
        .query_opt(
            "SELECT 1
             FROM conversation c
             LEFT JOIN company co ON co.id = c.workspace_id
             LEFT JOIN company_member com
               ON com.company_id = co.id AND com.principal_id = $2
             JOIN conversation_member cm
               ON cm.conv_id = c.id AND cm.principal_id = $2 AND cm.removed_at IS NULL
             WHERE c.id = $1
               AND ((co.id IS NULL AND c.workspace_id = $3)
                    OR (co.deleted_at IS NULL
                        AND (c.workspace_id = $3 OR com.principal_id IS NOT NULL)))",
            &[&conversation_id, &actor.id, &actor.workspace_id],
        )
        .await
        .map_err(|e| {
            ApiError(AppError::Internal(format!(
                "export conversation access check: {e}"
            )))
        })?;
    if row.is_some() {
        Ok(())
    } else {
        Err(ApiError(AppError::Forbidden(
            "cannot export this conversation".into(),
        )))
    }
}

// ── List companies ────────────────────────────────────────────────────

pub(crate) async fn list_companies(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<Vec<choruz_domain::Company>>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    Ok(Json(state.db.list_companies(&principal.id).await?))
}

// ── Create company ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct CreateCompanyPayload {
    actor_id: String,
    name: String,
    slug: Option<String>,
    description: Option<String>,
    folder_path: Option<String>,
}

pub(crate) async fn create_company(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(payload): Json<CreateCompanyPayload>,
) -> Result<(StatusCode, Json<choruz_domain::Company>), ApiError> {
    // Enforce session principal == payload.actor_id so a logged-in user
    // cannot attribute a company creation to someone else.
    require_actor(&headers, &state, &payload.actor_id).await?;
    state.db.check_rate_limit(&payload.actor_id)?;
    let company = state
        .db
        .create_company(choruz_application::CreateCompanyRequest {
            actor_id: payload.actor_id.clone(),
            name: payload.name,
            slug: payload.slug,
            description: payload.description,
            folder_path: payload.folder_path,
        })
        .await?;

    Ok((StatusCode::CREATED, Json(company)))
}

// ── Get company ───────────────────────────────────────────────────────

pub(crate) async fn get_company(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
) -> Result<Json<choruz_domain::Company>, ApiError> {
    require_company_access(&headers, &state, &company_id).await?;
    Ok(Json(state.db.get_company(&company_id).await?))
}

// ── Update company ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateCompanyPayload {
    actor_id: String,
    name: Option<String>,
    description: Option<String>,
    avatar_url: Option<String>,
    agents_active: Option<bool>,
    folder_path: Option<String>,
    multi_harness_accounts: Option<bool>,
}

pub(crate) async fn update_company(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
    Json(payload): Json<UpdateCompanyPayload>,
) -> Result<Json<choruz_domain::Company>, ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    let company = state
        .db
        .update_company(
            &company_id,
            choruz_application::UpdateCompanyRequest {
                actor_id: payload.actor_id,
                name: payload.name,
                description: payload.description,
                avatar_url: payload.avatar_url,
                agents_active: payload.agents_active,
                folder_path: payload.folder_path,
                multi_harness_accounts: payload.multi_harness_accounts,
            },
        )
        .await?;

    Ok(Json(company))
}

// ── Delete company (soft delete) ──────────────────────────────────────

pub(crate) async fn delete_company(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    state.db.delete_company(&company_id, &principal.id).await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Archive company ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ArchiveCompanyPayload {
    actor_id: String,
}

pub(crate) async fn archive_company(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
    Json(payload): Json<ArchiveCompanyPayload>,
) -> Result<Json<choruz_domain::Company>, ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    let company = state
        .db
        .archive_company(&company_id, &payload.actor_id)
        .await?;
    Ok(Json(company))
}

// ── Unarchive company ────────────────────────────────────────────────

pub(crate) async fn unarchive_company(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
    Json(payload): Json<ArchiveCompanyPayload>,
) -> Result<Json<choruz_domain::Company>, ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    let company = state
        .db
        .unarchive_company(&company_id, &payload.actor_id)
        .await?;
    Ok(Json(company))
}

// ── Company members ───────────────────────────────────────────────────

pub(crate) async fn list_company_members(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
) -> Result<Json<Vec<choruz_domain::CompanyMember>>, ApiError> {
    require_company_access(&headers, &state, &company_id).await?;
    Ok(Json(state.db.list_company_members(&company_id).await?))
}

#[derive(Debug, Deserialize)]
pub(crate) struct AddCompanyMemberPayload {
    actor_id: String,
    principal_id: String,
}

pub(crate) async fn add_company_member(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(company_id): Path<String>,
    Json(payload): Json<AddCompanyMemberPayload>,
) -> Result<(StatusCode, Json<choruz_domain::CompanyMember>), ApiError> {
    require_actor(&headers, &state, &payload.actor_id).await?;
    let member = state
        .db
        .add_company_member(
            &company_id,
            choruz_application::AddCompanyMemberRequest {
                actor_id: payload.actor_id,
                principal_id: payload.principal_id,
            },
        )
        .await?;

    Ok((StatusCode::CREATED, Json(member)))
}

pub(crate) async fn remove_company_member(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path((company_id, member_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    state
        .db
        .remove_company_member(&company_id, &principal.id, &member_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Audit logs ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct AuditLogsQuery {
    workspace_id: String,
}

pub(crate) async fn list_audit_logs(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<AuditLogsQuery>,
) -> Result<Json<Vec<choruz_domain::AuditLog>>, ApiError> {
    let principal = authenticated_principal(&headers, &state).await?;
    if principal.workspace_id != query.workspace_id {
        return Err(ApiError(AppError::Forbidden(
            "cannot read audit logs from another workspace".into(),
        )));
    }
    Ok(Json(state.db.list_audit_logs(&query.workspace_id).await?))
}

// ── Export conversation ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ExportConversationQuery {
    actor_id: String,
}

pub(crate) async fn export_conversation(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Query(query): Query<ExportConversationQuery>,
) -> Result<Json<choruz_application::ExportConversationResponse>, ApiError> {
    let actor = require_actor(&headers, &state, &query.actor_id).await?;
    require_conversation_export_access(&state, &actor, &conversation_id).await?;
    let conversation = state.db.get_conversation(&conversation_id).await?;
    let messages = state.db.list_all_messages(&conversation_id).await?;
    let channel_tasks = state.db.list_channel_task_exports(&conversation_id).await?;
    let audit_logs = state
        .db
        .list_audit_logs(&conversation.workspace_id)
        .await?
        .into_iter()
        .filter(|entry| entry.target_id == conversation.id)
        .collect();
    Ok(Json(choruz_application::ExportConversationResponse {
        conversation,
        messages,
        audit_logs,
        channel_tasks,
    }))
}
