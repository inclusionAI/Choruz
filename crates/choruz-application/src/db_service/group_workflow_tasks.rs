use choruz_common::metrics::{self, IntCounter};
use choruz_common::{AppError, new_id};
use choruz_domain::{ConversationType, PrincipalType};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, str::FromStr, sync::LazyLock};
use tokio_postgres::{Row, Transaction};

use super::DbService;
use crate::{
    AppendGroupWorkflowEventRequest, CreateGroupWorkflowTaskRequest, GroupWorkflowEvent,
    GroupWorkflowTask, GroupWorkflowTaskParticipant, UpdateGroupWorkflowTaskRequest,
    WorkflowTaskParticipantInput,
};
use crate::{
    ChannelTaskDetailResponse, ChannelTaskEventProjection, ChannelTaskEventVisibleValues,
    ChannelTaskExport, ChannelTaskSnapshot, ChannelTaskSourceKind, ChannelTaskStatus,
    CreateChannelTaskFromMessageRequest, CreateChannelTaskRequest, NullablePatch,
    PatchChannelTaskRequest,
};

const VALID_TASK_STATUSES: &[&str] = &["todo", "in_progress", "blocked", "in_review", "done"];
const CHANNEL_TASK_RECENT_EVENT_LIMIT: i64 = 50;
const CHANNEL_TASK_RAPID_UPDATE_SECONDS: i64 = 2;

static CHANNEL_TASK_CREATES_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    metrics::register_counter(
        "choruz_channel_task_creates_total",
        "Channel task cards created.",
    )
});
static CHANNEL_TASK_UPDATES_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    metrics::register_counter(
        "choruz_channel_task_updates_total",
        "Channel task cards updated.",
    )
});
static CHANNEL_TASK_MUTATION_ERRORS_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    metrics::register_counter(
        "choruz_channel_task_mutation_errors_total",
        "Channel task mutations rejected or failed.",
    )
});
static CHANNEL_TASK_LOAD_ERRORS_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    metrics::register_counter(
        "choruz_channel_task_load_errors_total",
        "Channel task rows that failed to load or project.",
    )
});

/// Forces the counters so a scrape reports each at `0` before its first event.
pub(super) fn register_channel_task_metrics() {
    for counter in [
        &CHANNEL_TASK_CREATES_TOTAL,
        &CHANNEL_TASK_UPDATES_TOTAL,
        &CHANNEL_TASK_MUTATION_ERRORS_TOTAL,
        &CHANNEL_TASK_LOAD_ERRORS_TOTAL,
    ] {
        LazyLock::force(counter);
    }
}

pub fn record_channel_task_load_error() {
    CHANNEL_TASK_LOAD_ERRORS_TOTAL.inc();
}

fn record_channel_task_load_error_result<T>(result: Result<T, AppError>) -> Result<T, AppError> {
    if result.is_err() {
        record_channel_task_load_error();
    }
    result
}

fn record_channel_task_create() {
    CHANNEL_TASK_CREATES_TOTAL.inc();
}

fn record_channel_task_update() {
    CHANNEL_TASK_UPDATES_TOTAL.inc();
}

fn record_channel_task_mutation_error() {
    CHANNEL_TASK_MUTATION_ERRORS_TOTAL.inc();
}

fn log_channel_task_mutation_failure(
    mutation: &str,
    actor_id: &str,
    conversation_id: Option<&str>,
    task_id: Option<&str>,
    assignee_principal_id: Option<&str>,
    error: &AppError,
) {
    tracing::warn!(
        event = "channel_task_mutation_failed",
        mutation,
        actor_supplied = !actor_id.trim().is_empty(),
        conversation_id,
        task_id_supplied = task_id.is_some(),
        assignee_supplied = assignee_principal_id.is_some(),
        error_kind = app_error_kind(error),
        error_detail_redacted = true,
        "channel task mutation failed"
    );
}

#[derive(Debug, Clone)]
struct RapidChannelTaskUpdateLog {
    actor_principal_id: Option<String>,
    actor_type: String,
    conversation_id: String,
    task_id: String,
    current_version: i64,
    resulting_version: i64,
    changed_fields: String,
    age_ms: i64,
}

fn rapid_channel_task_update_log(
    actor_principal_id: Option<&str>,
    actor_type: Option<&str>,
    existing: &GroupWorkflowTask,
    request: &PatchChannelTaskRequest,
) -> Option<RapidChannelTaskUpdateLog> {
    let age = Utc::now().signed_duration_since(existing.updated_at);
    if existing.version > 1
        && age >= Duration::zero()
        && age < Duration::seconds(CHANNEL_TASK_RAPID_UPDATE_SECONDS)
    {
        Some(RapidChannelTaskUpdateLog {
            actor_principal_id: actor_principal_id.map(str::to_string),
            actor_type: actor_type.unwrap_or("system").to_string(),
            conversation_id: existing.conversation_id.clone(),
            task_id: existing.id.clone(),
            current_version: existing.version,
            resulting_version: existing.version + 1,
            changed_fields: channel_task_patch_changed_fields(existing, request),
            age_ms: age.num_milliseconds(),
        })
    } else {
        None
    }
}

fn emit_rapid_channel_task_update_log(log: &RapidChannelTaskUpdateLog) {
    tracing::warn!(
        event = "channel_task_rapid_successive_update",
        actor_principal_supplied = log
            .actor_principal_id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty()),
        actor_type = %log.actor_type,
        system_actor = log.actor_principal_id.is_none(),
        conversation_id = %log.conversation_id,
        task_id = %log.task_id,
        current_version = log.current_version,
        resulting_version = log.resulting_version,
        changed_fields = %log.changed_fields,
        age_ms = log.age_ms,
        "rapid successive channel task update"
    );
}

fn channel_task_patch_changed_fields(
    existing: &GroupWorkflowTask,
    request: &PatchChannelTaskRequest,
) -> String {
    let mut fields = Vec::new();
    if let Some(status) = request.status
        && status.as_str() != existing.status
    {
        fields.push("status");
    }
    if let Some(assignee_principal_id) = &request.assignee_principal_id
        && assignee_principal_id != &existing.assignee_principal_id
    {
        fields.push("assignee");
    }
    if nullable_patch_changes(existing.blocked_reason.as_deref(), &request.blocked_reason) {
        fields.push("blocked_reason");
    }
    if nullable_patch_changes(existing.context_label.as_deref(), &request.context_label) {
        fields.push("context_label");
    }
    if fields.is_empty() {
        "none".to_string()
    } else {
        fields.join(",")
    }
}

fn nullable_patch_changes(current: Option<&str>, patch: &NullablePatch<String>) -> bool {
    match patch {
        NullablePatch::Unchanged => false,
        NullablePatch::Clear => current.is_some(),
        NullablePatch::Set(value) => current != Some(value.as_str()),
    }
}

fn log_unresolved_workflow_task_reference(
    conversation_id: &str,
    actor_id: Option<&str>,
    workflow_kind: &str,
    task_id: Option<&str>,
    task_key_supplied: bool,
) {
    tracing::warn!(
        event = "channel_task_workflow_reference_unresolved",
        conversation_id,
        actor_supplied = actor_id.is_some_and(|id| !id.trim().is_empty()),
        workflow_kind = safe_workflow_kind_label(workflow_kind),
        task_id_supplied = task_id.is_some(),
        task_key_supplied,
        "workflow metadata could not resolve channel task reference"
    );
}

fn safe_workflow_kind_label(workflow_kind: &str) -> &'static str {
    match workflow_kind {
        "task.created" => "task.created",
        "task.started" => "task.started",
        "task.ready_for_next_step" => "task.ready_for_next_step",
        "task.feedback" => "task.feedback",
        "task.cleared" => "task.cleared",
        "task.blocked" => "task.blocked",
        "human_input_needed" => "human_input_needed",
        "approval_required" => "approval_required",
        "task.completed" => "task.completed",
        "external_check.failed" => "external_check.failed",
        "external_check.passed" => "external_check.passed",
        _ => "unsupported",
    }
}

fn app_error_kind(error: &AppError) -> &'static str {
    match error {
        AppError::Unauthorized(_) => "unauthorized",
        AppError::NotFound(_) => "not_found",
        AppError::Conflict(_) => "conflict",
        AppError::Validation(_) => "validation",
        AppError::Forbidden(_) => "forbidden",
        AppError::RateLimited { .. } => "rate_limited",
        AppError::Internal(_) => "internal",
    }
}

impl DbService {
    pub async fn list_channel_tasks(
        &self,
        actor_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<ChannelTaskSnapshot>, AppError> {
        self.require_channel_task_conversation_access(actor_id, conversation_id)
            .await?;
        let client = self.store.connect().await.inspect_err(|_| {
            record_channel_task_load_error();
        })?;
        let rows = client
            .query(
                channel_task_snapshot_select_sql(
                    "
                    WHERE gwt.conversation_id = $1
                    ORDER BY gwt.created_at ASC, gwt.task_key ASC
                    ",
                )
                .as_str(),
                &[&conversation_id],
            )
            .await
            .map_err(|e| {
                record_channel_task_load_error();
                AppError::Internal(format!("list channel tasks: {e}"))
            })?;
        record_channel_task_load_error_result(
            rows.iter().map(channel_task_snapshot_from_row).collect(),
        )
    }

    pub async fn list_channel_task_exports(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ChannelTaskExport>, AppError> {
        let client = self.store.connect().await.inspect_err(|_| {
            record_channel_task_load_error();
        })?;
        let task_rows = client
            .query(
                channel_task_snapshot_select_sql(
                    "
                    WHERE gwt.conversation_id = $1
                    ORDER BY gwt.created_at ASC, gwt.task_key ASC
                    ",
                )
                .as_str(),
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list channel task export snapshots: {e}")))?;
        let tasks = task_rows
            .iter()
            .map(channel_task_snapshot_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        if tasks.is_empty() {
            return Ok(Vec::new());
        }

        let event_rows = client
            .query(
                "
                SELECT gwe.id,
                       gwe.task_id,
                       ap.id AS actor_principal_id,
                       ap.type AS actor_type,
                       prev_assignee.id AS previous_assignee_visible_id,
                       new_assignee.id AS new_assignee_visible_id,
                       prev_source.event_id AS previous_source_message_visible_id,
                       new_source.event_id AS new_source_message_visible_id,
                       gwe.kind,
                       gwe.payload,
                       gwe.resulting_version,
                       gwe.created_at
                FROM group_workflow_event gwe
                JOIN group_workflow_task gwt
                  ON gwt.id = gwe.task_id
                 AND gwt.conversation_id = gwe.conversation_id
                LEFT JOIN principal ap
                  ON ap.id = gwe.actor_principal_id
                 AND ap.workspace_id = (
                    SELECT c.workspace_id FROM conversation c WHERE c.id = gwe.conversation_id
                 )
                 AND ap.disabled = FALSE
                 AND ap.deleted_at IS NULL
                 AND NOT (ap.type = 'agent' AND ap.channel_visibility = 'internal')
                 AND EXISTS (
                    SELECT 1 FROM conversation_member cm
                    WHERE cm.conv_id = gwe.conversation_id
                      AND cm.principal_id = ap.id
                      AND cm.removed_at IS NULL
                 )
                LEFT JOIN principal prev_assignee
                  ON prev_assignee.id = gwe.payload #>> '{previous,assignee_principal_id}'
                 AND prev_assignee.workspace_id = (
                    SELECT c.workspace_id FROM conversation c WHERE c.id = gwe.conversation_id
                 )
                 AND prev_assignee.disabled = FALSE
                 AND prev_assignee.deleted_at IS NULL
                 AND NOT (prev_assignee.type = 'agent' AND prev_assignee.channel_visibility = 'internal')
                 AND EXISTS (
                    SELECT 1 FROM conversation_member cm
                    WHERE cm.conv_id = gwe.conversation_id
                      AND cm.principal_id = prev_assignee.id
                      AND cm.removed_at IS NULL
                 )
                LEFT JOIN principal new_assignee
                  ON new_assignee.id = gwe.payload #>> '{new,assignee_principal_id}'
                 AND new_assignee.workspace_id = (
                    SELECT c.workspace_id FROM conversation c WHERE c.id = gwe.conversation_id
                 )
                 AND new_assignee.disabled = FALSE
                 AND new_assignee.deleted_at IS NULL
                 AND NOT (new_assignee.type = 'agent' AND new_assignee.channel_visibility = 'internal')
                 AND EXISTS (
                    SELECT 1 FROM conversation_member cm
                    WHERE cm.conv_id = gwe.conversation_id
                      AND cm.principal_id = new_assignee.id
                      AND cm.removed_at IS NULL
                 )
                LEFT JOIN conversation_events prev_source
                  ON prev_source.conversation_id = gwe.conversation_id
                 AND prev_source.event_id = gwe.payload #>> '{previous,source_message_id}'
                 AND prev_source.event_type IN ('message', 'message.created', 'reply')
                LEFT JOIN conversation_events new_source
                  ON new_source.conversation_id = gwe.conversation_id
                 AND new_source.event_id = gwe.payload #>> '{new,source_message_id}'
                 AND new_source.event_type IN ('message', 'message.created', 'reply')
                WHERE gwe.conversation_id = $1
                  AND (
                      NOT (gwe.payload ? 'workflow_diagnostic')
                      OR gwe.payload #>> '{workflow_diagnostic,reason_code}' = 'workflow_status_noop'
                  )
                ORDER BY gwt.created_at ASC, gwt.task_key ASC, gwe.created_at ASC, gwe.id ASC
                ",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list channel task export events: {e}")))?;
        let mut events_by_task = BTreeMap::<String, Vec<ChannelTaskEventProjection>>::new();
        for row in &event_rows {
            let event =
                record_channel_task_load_error_result(channel_task_event_projection_from_row(row))?;
            events_by_task
                .entry(event.task_id.clone())
                .or_default()
                .push(event);
        }

        Ok(tasks
            .into_iter()
            .map(|task| ChannelTaskExport {
                events: events_by_task.remove(&task.task_id).unwrap_or_default(),
                task,
            })
            .collect())
    }

    pub async fn get_channel_task_detail(
        &self,
        actor_id: &str,
        task_id: &str,
    ) -> Result<ChannelTaskDetailResponse, AppError> {
        let client = self.store.connect().await.inspect_err(|_| {
            record_channel_task_load_error();
        })?;
        let row = client
            .query_opt(
                channel_task_snapshot_select_sql("WHERE gwt.id = $1").as_str(),
                &[&task_id],
            )
            .await
            .map_err(|e| {
                record_channel_task_load_error();
                AppError::Internal(format!("get channel task: {e}"))
            })?;
        let row = row.ok_or_else(|| AppError::NotFound("channel task not found".into()))?;
        let task = record_channel_task_load_error_result(channel_task_snapshot_from_row(&row))?;
        self.require_channel_task_conversation_access(actor_id, &task.conversation_id)
            .await?;
        let event_rows = client
            .query(
                "
                SELECT recent.id,
                       recent.task_id,
                       ap.id AS actor_principal_id,
                       ap.type AS actor_type,
                       prev_assignee.id AS previous_assignee_visible_id,
                       new_assignee.id AS new_assignee_visible_id,
                       prev_source.event_id AS previous_source_message_visible_id,
                       new_source.event_id AS new_source_message_visible_id,
                       recent.kind,
                       recent.payload,
                       recent.resulting_version,
                       recent.created_at
                FROM (
                    SELECT id, conversation_id, task_id, actor_principal_id, kind, payload,
                           resulting_version, created_at
                    FROM group_workflow_event
                    WHERE task_id = $1
                      AND conversation_id = $2
                      AND (
                          NOT (payload ? 'workflow_diagnostic')
                          OR payload #>> '{workflow_diagnostic,reason_code}' = 'workflow_status_noop'
                      )
                    ORDER BY created_at DESC, id DESC
                    LIMIT $3
                ) recent
                LEFT JOIN principal ap
                  ON ap.id = recent.actor_principal_id
                 AND ap.workspace_id = (
                    SELECT c.workspace_id FROM conversation c WHERE c.id = recent.conversation_id
                 )
                 AND ap.disabled = FALSE
                 AND ap.deleted_at IS NULL
                 AND NOT (ap.type = 'agent' AND ap.channel_visibility = 'internal')
                 AND EXISTS (
                    SELECT 1 FROM conversation_member cm
                    WHERE cm.conv_id = recent.conversation_id
                      AND cm.principal_id = ap.id
                      AND cm.removed_at IS NULL
                 )
                LEFT JOIN principal prev_assignee
                  ON prev_assignee.id = recent.payload #>> '{previous,assignee_principal_id}'
                 AND prev_assignee.workspace_id = (
                    SELECT c.workspace_id FROM conversation c WHERE c.id = recent.conversation_id
                 )
                 AND prev_assignee.disabled = FALSE
                 AND prev_assignee.deleted_at IS NULL
                 AND NOT (prev_assignee.type = 'agent' AND prev_assignee.channel_visibility = 'internal')
                 AND EXISTS (
                    SELECT 1 FROM conversation_member cm
                    WHERE cm.conv_id = recent.conversation_id
                      AND cm.principal_id = prev_assignee.id
                      AND cm.removed_at IS NULL
                 )
                LEFT JOIN principal new_assignee
                  ON new_assignee.id = recent.payload #>> '{new,assignee_principal_id}'
                 AND new_assignee.workspace_id = (
                    SELECT c.workspace_id FROM conversation c WHERE c.id = recent.conversation_id
                 )
                 AND new_assignee.disabled = FALSE
                 AND new_assignee.deleted_at IS NULL
                 AND NOT (new_assignee.type = 'agent' AND new_assignee.channel_visibility = 'internal')
                 AND EXISTS (
                    SELECT 1 FROM conversation_member cm
                    WHERE cm.conv_id = recent.conversation_id
                      AND cm.principal_id = new_assignee.id
                      AND cm.removed_at IS NULL
                 )
                LEFT JOIN conversation_events prev_source
                  ON prev_source.conversation_id = recent.conversation_id
                 AND prev_source.event_id = recent.payload #>> '{previous,source_message_id}'
                 AND prev_source.event_type IN ('message', 'message.created', 'reply')
                LEFT JOIN conversation_events new_source
                  ON new_source.conversation_id = recent.conversation_id
                 AND new_source.event_id = recent.payload #>> '{new,source_message_id}'
                 AND new_source.event_type IN ('message', 'message.created', 'reply')
                ORDER BY recent.created_at ASC, recent.id ASC
                ",
                &[
                    &task_id,
                    &task.conversation_id,
                    &CHANNEL_TASK_RECENT_EVENT_LIMIT,
                ],
            )
            .await
            .map_err(|e| {
                record_channel_task_load_error();
                AppError::Internal(format!("list channel task events: {e}"))
            })?;
        let events = record_channel_task_load_error_result(
            event_rows
                .iter()
                .map(channel_task_event_projection_from_row)
                .collect::<Result<Vec<_>, _>>(),
        )?;
        Ok(ChannelTaskDetailResponse { task, events })
    }

    async fn get_channel_task_snapshot(
        &self,
        actor_id: &str,
        task_id: &str,
    ) -> Result<ChannelTaskSnapshot, AppError> {
        let client = self.store.connect().await.inspect_err(|_| {
            record_channel_task_load_error();
        })?;
        let row = client
            .query_opt(
                channel_task_snapshot_select_sql("WHERE gwt.id = $1").as_str(),
                &[&task_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("get channel task snapshot: {e}")))?;
        let row = row.ok_or_else(|| AppError::NotFound("channel task not found".into()))?;
        let task = record_channel_task_load_error_result(channel_task_snapshot_from_row(&row))?;
        self.require_channel_task_conversation_access(actor_id, &task.conversation_id)
            .await?;
        Ok(task)
    }

    pub async fn create_channel_task_from_message(
        &self,
        actor_id: &str,
        conversation_id: &str,
        request: CreateChannelTaskFromMessageRequest,
    ) -> Result<(bool, ChannelTaskSnapshot), AppError> {
        let assignee_principal_id = request.assignee_principal_id.clone();
        let message_id = request.message_id.clone();
        let result = self
            .create_channel_task_from_message_impl(actor_id, conversation_id, request)
            .await;
        match &result {
            Ok((true, task)) => {
                record_channel_task_create();
                tracing::info!(
                    event = "channel_task_create_succeeded",
                    source_kind = "message",
                    actor_id,
                    conversation_id,
                    task_id = %task.task_id,
                    assignee_principal_id = task.assignee_principal_id.as_deref().unwrap_or("none"),
                    status = %task.status.as_str(),
                    "channel task created from message"
                );
            }
            Ok((false, task)) => {
                tracing::info!(
                    event = "channel_task_create_deduped",
                    source_kind = "message",
                    actor_id,
                    conversation_id,
                    task_id = %task.task_id,
                    message_id = %message_id,
                    "channel task create from message returned existing task"
                );
            }
            Err(error) => {
                record_channel_task_mutation_error();
                log_channel_task_mutation_failure(
                    "create_from_message",
                    actor_id,
                    Some(conversation_id),
                    None,
                    Some(assignee_principal_id.as_str()),
                    error,
                );
            }
        }
        result
    }

    async fn create_channel_task_from_message_impl(
        &self,
        actor_id: &str,
        conversation_id: &str,
        request: CreateChannelTaskFromMessageRequest,
    ) -> Result<(bool, ChannelTaskSnapshot), AppError> {
        validate_required_text("title", &request.title)?;
        validate_required_text("message_id", &request.message_id)?;
        validate_required_text("assignee_principal_id", &request.assignee_principal_id)?;
        if let Some(context_label) = &request.context_label {
            validate_required_text("context_label", context_label)?;
        }
        let actor = self.get_principal(actor_id).await?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "only humans can create channel tasks from messages".into(),
            ));
        }

        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("create channel task tx: {e}")))?;
        require_channel_task_member_access_tx(&tx, actor_id, conversation_id).await?;
        require_channel_task_conversation_eligible_tx(&tx, conversation_id).await?;
        validate_visible_channel_task_assignee_tx(
            &tx,
            conversation_id,
            &request.assignee_principal_id,
            true,
            true,
        )
        .await?;
        validate_visible_message_source_tx(&tx, conversation_id, &request.message_id).await?;

        if let Some(existing_id) =
            find_message_derived_task_tx(&tx, conversation_id, actor_id, &request.message_id)
                .await?
        {
            tx.commit()
                .await
                .map_err(|e| AppError::Internal(format!("commit channel task dedupe: {e}")))?;
            return Ok((
                false,
                self.get_channel_task_snapshot(actor_id, &existing_id)
                    .await?,
            ));
        }

        let mut inserted_task_id = None;
        for _ in 0..GENERATED_TASK_INSERT_MAX_ATTEMPTS {
            let task_key = allocate_unused_task_key_tx(&tx, conversation_id).await?;
            let task_id = new_id();
            let inserted = tx
                .query_opt(
                    "INSERT INTO group_workflow_task
                    (id, conversation_id, task_key, title, status, assignee_principal_id,
                     source_kind, source_message_id, context_label, idempotency_key,
                     created_by, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, 'todo', $5, 'message', $6, $7, $8, $9, NOW(), NOW())
                 ON CONFLICT DO NOTHING
                 RETURNING id",
                    &[
                        &task_id,
                        &conversation_id,
                        &task_key,
                        &request.title,
                        &request.assignee_principal_id,
                        &request.message_id,
                        &request.context_label,
                        &request.idempotency_key,
                        &actor_id,
                    ],
                )
                .await
                .map_err(|e| {
                    AppError::Internal(format!("create channel task from message: {e}"))
                })?;
            if inserted.is_some() {
                inserted_task_id = Some(task_id);
                break;
            }
            if let Some(existing_id) =
                find_message_derived_task_tx(&tx, conversation_id, actor_id, &request.message_id)
                    .await?
            {
                tx.commit().await.map_err(|e| {
                    AppError::Internal(format!("commit raced channel task dedupe: {e}"))
                })?;
                return Ok((
                    false,
                    self.get_channel_task_snapshot(actor_id, &existing_id)
                        .await?,
                ));
            }
            if let Some(idempotency_key) = request.idempotency_key.as_deref()
                && find_agent_idempotent_task_tx(&tx, conversation_id, actor_id, idempotency_key)
                    .await?
                    .is_some()
            {
                return Err(AppError::Conflict(
                    "channel task key or idempotency key already exists".into(),
                ));
            }
        }
        let task_id = inserted_task_id.ok_or_else(|| {
            AppError::Internal(
                "could not create channel task after retrying generated task keys".into(),
            )
        })?;
        sync_owner_participant_tx(
            &tx,
            conversation_id,
            &task_id,
            &request.assignee_principal_id,
        )
        .await?;
        insert_workflow_event_tx(
            &tx,
            conversation_id,
            Some(&task_id),
            Some(&request.message_id),
            Some(actor_id),
            "channel_task.created",
            &json!({
                "new": {
                    "status": "todo",
                    "assignee_principal_id": request.assignee_principal_id,
                    "context_label": request.context_label,
                    "source_kind": "message",
                    "source_message_id": request.message_id,
                }
            }),
            Some(1),
        )
        .await?;
        let task = insert_channel_task_fanout_event_tx(
            &tx,
            conversation_id,
            Some(actor_id),
            "channel_task.created",
            &task_id,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit channel task create: {e}")))?;

        Ok((true, task))
    }

    pub async fn create_channel_task(
        &self,
        actor_id: &str,
        conversation_id: &str,
        request: CreateChannelTaskRequest,
    ) -> Result<(bool, ChannelTaskSnapshot), AppError> {
        let requested_assignee_principal_id = request.assignee_principal_id.clone();
        let result = self
            .create_channel_task_impl(actor_id, conversation_id, request)
            .await;
        match &result {
            Ok((true, task)) => {
                record_channel_task_create();
                tracing::info!(
                    event = "channel_task_create_succeeded",
                    source_kind = "agent",
                    actor_id,
                    conversation_id,
                    task_id = %task.task_id,
                    assignee_principal_id = task.assignee_principal_id.as_deref().unwrap_or("none"),
                    status = %task.status.as_str(),
                    "channel task created"
                );
            }
            Ok((false, task)) => {
                tracing::info!(
                    event = "channel_task_create_idempotent",
                    source_kind = "agent",
                    actor_id,
                    conversation_id,
                    task_id = %task.task_id,
                    "channel task create returned existing task"
                );
            }
            Err(error) => {
                record_channel_task_mutation_error();
                log_channel_task_mutation_failure(
                    "create",
                    actor_id,
                    Some(conversation_id),
                    None,
                    requested_assignee_principal_id.as_deref(),
                    error,
                );
            }
        }
        result
    }

    async fn create_channel_task_impl(
        &self,
        actor_id: &str,
        conversation_id: &str,
        request: CreateChannelTaskRequest,
    ) -> Result<(bool, ChannelTaskSnapshot), AppError> {
        if let Some(key) = request.task_key.as_deref() {
            validate_task_key(key)?;
        }
        validate_meaningful_title(&request.title)?;
        validate_required_text("idempotency_key", &request.idempotency_key)?;
        if let Some(context_label) = &request.context_label {
            validate_required_text("context_label", context_label)?;
        }

        let status = request.status.unwrap_or(ChannelTaskStatus::Todo);
        let assignee_principal_id = request
            .assignee_principal_id
            .clone()
            .unwrap_or_else(|| actor_id.to_string());
        // Hash on the caller-supplied key (empty sentinel when omitted) so retries
        // without an explicit task_key produce a stable payload hash even though we
        // synthesize a fresh TASK-{N} on each insert path.
        let payload_hash = channel_task_create_payload_hash(
            request.task_key.as_deref().unwrap_or(""),
            &request.title,
            &assignee_principal_id,
            status,
            request.context_label.as_deref(),
        )?;

        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("create channel task tx: {e}")))?;
        require_visible_group_agent_actor_tx(&tx, actor_id, conversation_id).await?;
        if let Some((existing_id, existing_hash)) =
            find_agent_idempotent_task_tx(&tx, conversation_id, actor_id, &request.idempotency_key)
                .await?
        {
            if existing_hash.as_deref() == Some(payload_hash.as_str()) {
                tx.commit().await.map_err(|e| {
                    AppError::Internal(format!("commit channel task idempotent read: {e}"))
                })?;
                return Ok((
                    false,
                    self.get_channel_task_snapshot(actor_id, &existing_id)
                        .await?,
                ));
            }
            return Err(AppError::Conflict(
                "idempotency_key was already used for a different channel task payload".into(),
            ));
        }

        if let Err(error) = validate_visible_channel_task_assignee_tx(
            &tx,
            conversation_id,
            &assignee_principal_id,
            false,
            true,
        )
        .await
        {
            if let Some((existing_id, existing_hash)) = find_agent_idempotent_task_tx(
                &tx,
                conversation_id,
                actor_id,
                &request.idempotency_key,
            )
            .await?
            {
                if existing_hash.as_deref() == Some(payload_hash.as_str()) {
                    tx.commit().await.map_err(|e| {
                        AppError::Internal(format!(
                            "commit channel task idempotent validation fallback: {e}"
                        ))
                    })?;
                    return Ok((
                        false,
                        self.get_channel_task_snapshot(actor_id, &existing_id)
                            .await?,
                    ));
                }
                return Err(AppError::Conflict(
                    "idempotency_key was already used for a different channel task payload".into(),
                ));
            }
            return Err(error);
        }

        let explicit_task_key = request.task_key.as_deref();
        let max_insert_attempts = if explicit_task_key.is_some() {
            1
        } else {
            GENERATED_TASK_INSERT_MAX_ATTEMPTS
        };
        let mut inserted_task_id = None;
        for _ in 0..max_insert_attempts {
            let task_id = new_id();
            let resolved_task_key = match explicit_task_key {
                Some(key) => key.to_string(),
                None => allocate_unused_task_key_tx(&tx, conversation_id).await?,
            };
            let inserted = if explicit_task_key.is_some() {
                tx.query_opt(
                    "INSERT INTO group_workflow_task
                        (id, conversation_id, task_key, title, status, assignee_principal_id,
                         source_kind, context_label, idempotency_key, idempotency_payload_hash,
                         created_by, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, 'agent', $7, $8, $9, $10, NOW(), NOW())
                     ON CONFLICT (conversation_id, created_by, idempotency_key)
                       WHERE idempotency_key IS NOT NULL
                     DO NOTHING
                     RETURNING id",
                    &[
                        &task_id,
                        &conversation_id,
                        &resolved_task_key,
                        &request.title,
                        &status.as_str(),
                        &assignee_principal_id,
                        &request.context_label,
                        &request.idempotency_key,
                        &payload_hash,
                        &actor_id,
                    ],
                )
                .await
                .map_err(|e| {
                    if is_unique_violation(&e) {
                        AppError::Conflict("channel task key already exists".into())
                    } else {
                        AppError::Internal(format!("create channel task: {e}"))
                    }
                })?
            } else {
                tx.query_opt(
                    "INSERT INTO group_workflow_task
                        (id, conversation_id, task_key, title, status, assignee_principal_id,
                         source_kind, context_label, idempotency_key, idempotency_payload_hash,
                         created_by, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, 'agent', $7, $8, $9, $10, NOW(), NOW())
                     ON CONFLICT DO NOTHING
                     RETURNING id",
                    &[
                        &task_id,
                        &conversation_id,
                        &resolved_task_key,
                        &request.title,
                        &status.as_str(),
                        &assignee_principal_id,
                        &request.context_label,
                        &request.idempotency_key,
                        &payload_hash,
                        &actor_id,
                    ],
                )
                .await
                .map_err(|e| AppError::Internal(format!("create channel task: {e}")))?
            };
            if inserted.is_some() {
                inserted_task_id = Some(task_id);
                break;
            }
            if let Some((existing_id, existing_hash)) = find_agent_idempotent_task_tx(
                &tx,
                conversation_id,
                actor_id,
                &request.idempotency_key,
            )
            .await?
            {
                if existing_hash.as_deref() == Some(payload_hash.as_str()) {
                    tx.commit().await.map_err(|e| {
                        AppError::Internal(format!("commit channel task raced idempotency: {e}"))
                    })?;
                    return Ok((
                        false,
                        self.get_channel_task_snapshot(actor_id, &existing_id)
                            .await?,
                    ));
                }
                return Err(AppError::Conflict(
                    "idempotency_key was already used for a different channel task payload".into(),
                ));
            }
            if explicit_task_key.is_some() {
                return Err(AppError::Conflict(
                    "idempotency_key was already used for a different channel task payload".into(),
                ));
            }
        }
        let task_id = inserted_task_id.ok_or_else(|| {
            AppError::Internal(
                "could not create channel task after retrying generated task keys".into(),
            )
        })?;

        sync_owner_participant_tx(&tx, conversation_id, &task_id, &assignee_principal_id).await?;
        insert_workflow_event_tx(
            &tx,
            conversation_id,
            Some(&task_id),
            None,
            Some(actor_id),
            "channel_task.created",
            &json!({
                "new": {
                    "status": status.as_str(),
                    "assignee_principal_id": assignee_principal_id,
                    "context_label": request.context_label,
                    "source_kind": "agent",
                }
            }),
            Some(1),
        )
        .await?;
        let task = insert_channel_task_fanout_event_tx(
            &tx,
            conversation_id,
            Some(actor_id),
            "channel_task.created",
            &task_id,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit channel task create: {e}")))?;

        Ok((true, task))
    }

    pub async fn patch_channel_task(
        &self,
        actor_id: &str,
        task_id: &str,
        request: PatchChannelTaskRequest,
    ) -> Result<ChannelTaskSnapshot, AppError> {
        let requested_assignee_principal_id = request.assignee_principal_id.clone();
        let requested_status = request.status;
        let result = self
            .patch_channel_task_impl(actor_id, task_id, request)
            .await;
        match &result {
            Ok((task, rapid_update_log)) => {
                record_channel_task_update();
                if let Some(rapid_update_log) = rapid_update_log {
                    emit_rapid_channel_task_update_log(rapid_update_log);
                }
                tracing::info!(
                    event = "channel_task_update_succeeded",
                    actor_id,
                    conversation_id = %task.conversation_id,
                    task_id = %task.task_id,
                    status = %task.status.as_str(),
                    assignee_principal_id = task.assignee_principal_id.as_deref().unwrap_or("none"),
                    version = task.version,
                    "channel task updated"
                );
            }
            Err(error) => {
                record_channel_task_mutation_error();
                log_channel_task_mutation_failure(
                    "patch",
                    actor_id,
                    None,
                    Some(task_id),
                    requested_assignee_principal_id.as_deref(),
                    error,
                );
                if let Some(status) = requested_status {
                    tracing::debug!(
                        event = "channel_task_failed_status_update",
                        actor_supplied = !actor_id.trim().is_empty(),
                        task_id_supplied = !task_id.trim().is_empty(),
                        status = %status.as_str(),
                        "channel task status update failed"
                    );
                }
            }
        }
        result.map(|(task, _)| task)
    }

    async fn patch_channel_task_impl(
        &self,
        actor_id: &str,
        task_id: &str,
        request: PatchChannelTaskRequest,
    ) -> Result<(ChannelTaskSnapshot, Option<RapidChannelTaskUpdateLog>), AppError> {
        validate_patch_request(&request)?;
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("patch channel task tx: {e}")))?;
        let existing = lock_group_workflow_task_tx(&tx, task_id).await?;
        require_channel_task_member_access_tx(&tx, actor_id, &existing.conversation_id).await?;
        let (event, rapid_update_log) = apply_channel_task_patch_tx(
            &tx,
            actor_id,
            &existing,
            &request,
            None,
            "channel_task.updated",
            existing.source_message_id.as_deref(),
            &json!({}),
        )
        .await?;
        let task = if event.resulting_version.is_some() {
            insert_channel_task_fanout_event_tx(
                &tx,
                &existing.conversation_id,
                Some(actor_id),
                "channel_task.updated",
                task_id,
            )
            .await?
        } else {
            channel_task_snapshot_tx(&tx, task_id).await?
        };
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit channel task patch: {e}")))?;

        Ok((task, rapid_update_log))
    }

    pub async fn list_group_workflow_tasks(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<GroupWorkflowTask>, AppError> {
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT id, conversation_id, task_key, title, status,
                        assignee_principal_id, blocked_reason, source_kind, context_label,
                        idempotency_key, idempotency_payload_hash, version,
                        source_message_id, created_by, created_at, updated_at
                 FROM group_workflow_task
                 WHERE conversation_id = $1
                 ORDER BY created_at ASC, task_key ASC",
                &[&conversation_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list group workflow tasks: {e}")))?;

        let mut tasks = Vec::with_capacity(rows.len());
        for row in rows {
            let mut task = task_from_row(&row);
            task.participants = self.list_task_participants(&task.id).await?;
            tasks.push(task);
        }
        Ok(tasks)
    }

    pub async fn create_group_workflow_task(
        &self,
        conversation_id: &str,
        created_by: &str,
        request: CreateGroupWorkflowTaskRequest,
    ) -> Result<GroupWorkflowTask, AppError> {
        validate_task_key(&request.task_key)?;
        validate_required_text("title", &request.title)?;
        validate_required_text("assignee_principal_id", &request.assignee_principal_id)?;
        validate_non_owner_participants(&request.participants)?;
        self.require_group_conversation(conversation_id).await?;

        let task_id = new_id();
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("create workflow task tx: {e}")))?;
        let row = tx
            .query_one(
                "INSERT INTO group_workflow_task
                    (id, conversation_id, task_key, title, assignee_principal_id,
                     source_message_id, created_by, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW())
                 RETURNING id, conversation_id, task_key, title, status,
                           assignee_principal_id, blocked_reason, source_kind, context_label,
                           idempotency_key, idempotency_payload_hash, version,
                           source_message_id, created_by, created_at, updated_at",
                &[
                    &task_id,
                    &conversation_id,
                    &request.task_key,
                    &request.title,
                    &request.assignee_principal_id,
                    &request.source_message_id,
                    &created_by,
                ],
            )
            .await
            .map_err(|e| {
                if is_unique_violation(&e) {
                    AppError::Conflict("workflow task key already exists".into())
                } else {
                    AppError::Internal(format!("create workflow task: {e}"))
                }
            })?;

        replace_non_owner_task_participants_tx(
            &tx,
            conversation_id,
            &task_id,
            &request.participants,
        )
        .await?;
        sync_owner_participant_tx(
            &tx,
            conversation_id,
            &task_id,
            &request.assignee_principal_id,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit workflow task: {e}")))?;

        let mut task = task_from_row(&row);
        task.participants = self.list_task_participants(&task.id).await?;
        Ok(task)
    }

    pub async fn get_group_workflow_task(
        &self,
        task_id: &str,
    ) -> Result<GroupWorkflowTask, AppError> {
        let client = self.store.connect().await?;
        let row = client
            .query_opt(
                "SELECT id, conversation_id, task_key, title, status,
                        assignee_principal_id, blocked_reason, source_kind, context_label,
                        idempotency_key, idempotency_payload_hash, version,
                        source_message_id, created_by, created_at, updated_at
                 FROM group_workflow_task
                 WHERE id = $1",
                &[&task_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("get workflow task: {e}")))?;
        let row = row.ok_or_else(|| AppError::NotFound("workflow task not found".into()))?;
        let mut task = task_from_row(&row);
        task.participants = self.list_task_participants(&task.id).await?;
        task.events = self.list_task_events(&task.id).await?;
        Ok(task)
    }

    pub async fn update_group_workflow_task(
        &self,
        task_id: &str,
        request: UpdateGroupWorkflowTaskRequest,
    ) -> Result<GroupWorkflowTask, AppError> {
        if let Some(title) = &request.title {
            validate_required_text("title", title)?;
        }
        if let Some(status) = &request.status {
            validate_task_status(status)?;
        }
        if let Some(assignee_principal_id) = &request.assignee_principal_id {
            validate_required_text("assignee_principal_id", assignee_principal_id)?;
        }
        if let Some(participants) = &request.participants {
            validate_non_owner_participants(participants)?;
        }

        let existing = self.get_group_workflow_task(task_id).await?;
        let title = request.title.unwrap_or(existing.title);
        let status = request.status.unwrap_or(existing.status);
        let assignee_principal_id = request
            .assignee_principal_id
            .unwrap_or(existing.assignee_principal_id);
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("update workflow task tx: {e}")))?;
        let row = tx
            .query_one(
                "UPDATE group_workflow_task
                 SET title = $2,
                     status = $3,
                     assignee_principal_id = $4,
                     version = version + 1,
                     updated_at = NOW()
                 WHERE id = $1
                 RETURNING id, conversation_id, task_key, title, status,
                           assignee_principal_id, blocked_reason, source_kind, context_label,
                           idempotency_key, idempotency_payload_hash, version,
                           source_message_id, created_by, created_at, updated_at",
                &[&task_id, &title, &status, &assignee_principal_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("update workflow task: {e}")))?;

        if let Some(participants) = request.participants {
            replace_non_owner_task_participants_tx(
                &tx,
                &existing.conversation_id,
                task_id,
                &participants,
            )
            .await?;
        }
        sync_owner_participant_tx(
            &tx,
            &existing.conversation_id,
            task_id,
            &assignee_principal_id,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit workflow task update: {e}")))?;

        let mut task = task_from_row(&row);
        task.participants = self.list_task_participants(&task.id).await?;
        task.events = self.list_task_events(&task.id).await?;
        Ok(task)
    }

    pub async fn replace_group_workflow_task_participants(
        &self,
        task_id: &str,
        participants: &[WorkflowTaskParticipantInput],
    ) -> Result<Vec<GroupWorkflowTaskParticipant>, AppError> {
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("replace workflow participants tx: {e}")))?;
        let task = lock_group_workflow_task_tx(&tx, task_id).await?;
        validate_non_owner_participants(participants)?;
        replace_non_owner_task_participants_tx(&tx, &task.conversation_id, task_id, participants)
            .await?;
        sync_owner_participant_tx(
            &tx,
            &task.conversation_id,
            task_id,
            &task.assignee_principal_id,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit workflow participants: {e}")))?;
        self.list_task_participants(task_id).await
    }

    pub async fn add_group_workflow_task_participant(
        &self,
        task_id: &str,
        participant: WorkflowTaskParticipantInput,
    ) -> Result<Vec<GroupWorkflowTaskParticipant>, AppError> {
        let task = self.get_group_workflow_task(task_id).await?;
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("add workflow participant tx: {e}")))?;
        validate_non_owner_participant(&participant)?;
        insert_task_participant_tx(&tx, &task.conversation_id, task_id, &participant, false)
            .await?;
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit workflow participant add: {e}")))?;
        self.list_task_participants(task_id).await
    }

    pub async fn remove_group_workflow_task_participant(
        &self,
        task_id: &str,
        principal_id: &str,
        role_key: &str,
    ) -> Result<Vec<GroupWorkflowTaskParticipant>, AppError> {
        self.get_group_workflow_task(task_id).await?;
        validate_required_text("principal_id", principal_id)?;
        validate_required_text("role_key", role_key)?;
        if role_key == "owner" {
            return Err(AppError::Validation(
                "owner participant is synchronized from assignee_principal_id".into(),
            ));
        }
        let client = self.store.connect().await?;
        client
            .execute(
                "DELETE FROM group_workflow_task_participant
                 WHERE task_id = $1 AND principal_id = $2 AND role_key = $3",
                &[&task_id, &principal_id, &role_key],
            )
            .await
            .map_err(|e| AppError::Internal(format!("remove workflow participant: {e}")))?;
        self.list_task_participants(task_id).await
    }

    pub async fn append_group_workflow_event(
        &self,
        task_id: &str,
        default_actor_id: &str,
        request: AppendGroupWorkflowEventRequest,
    ) -> Result<GroupWorkflowEvent, AppError> {
        let result: Result<GroupWorkflowEvent, AppError> = async {
            validate_required_text("kind", &request.kind)?;
            validate_object_payload(&request.payload)?;
            let task = self.get_group_workflow_task(task_id).await?;
            if let Some(task_key) = request.task_key.as_deref()
                && task_key != task.task_key
            {
                return Err(AppError::Validation(
                    "task_key does not match workflow task".into(),
                ));
            }

            let actor_id = default_actor_id.to_string();
            let status_update = status_for_workflow_event(&request.kind, &request.payload);
            let mut client = self.store.connect().await?;
            let tx = client
                .transaction()
                .await
                .map_err(|e| AppError::Internal(format!("append workflow event tx: {e}")))?;
            validate_workflow_event_actor_tx(&tx, &task.conversation_id, &actor_id).await?;
            let (event, rapid_update_log) = apply_workflow_status_update_tx(
                &tx,
                &task,
                &actor_id,
                &request.kind,
                request.source_message_id.as_deref(),
                status_update,
                &request.payload,
            )
            .await?;
            if event.resulting_version.is_some() {
                insert_channel_task_fanout_event_tx(
                    &tx,
                    &task.conversation_id,
                    Some(actor_id.as_str()),
                    "channel_task.updated",
                    &task.id,
                )
                .await?;
            }
            tx.commit()
                .await
                .map_err(|e| AppError::Internal(format!("commit workflow event: {e}")))?;

            if event.resulting_version.is_some() {
                record_channel_task_update();
                if let Some(rapid_update_log) = &rapid_update_log {
                    emit_rapid_channel_task_update_log(rapid_update_log);
                }
            }
            Ok(event)
        }
        .await;
        if let Err(error) = &result {
            record_channel_task_mutation_error();
            log_channel_task_mutation_failure(
                "workflow_append",
                default_actor_id,
                None,
                Some(task_id),
                None,
                error,
            );
        }
        result
    }

    pub async fn append_group_workflow_event_for_conversation(
        &self,
        conversation_id: &str,
        default_actor_id: &str,
        request: AppendGroupWorkflowEventRequest,
    ) -> Result<GroupWorkflowEvent, AppError> {
        let result: Result<GroupWorkflowEvent, AppError> = async {
            validate_required_text("kind", &request.kind)?;
            validate_object_payload(&request.payload)?;
            self.require_group_conversation(conversation_id).await?;
            let resolved_task_id = self
                .find_task_id_by_workflow_reference(
                    conversation_id,
                    request
                        .payload
                        .get("task_id")
                        .and_then(|value| value.as_str()),
                    request.task_key.as_deref(),
                )
                .await?;
            let resolved_task = match resolved_task_id.as_deref() {
                Some(task_id) => Some(self.get_group_workflow_task(task_id).await?),
                None => None,
            };
            let actor_id = default_actor_id.to_string();
            let status_update = status_for_workflow_event(&request.kind, &request.payload);
            let mut client = self.store.connect().await?;
            let tx = client
                .transaction()
                .await
                .map_err(|e| AppError::Internal(format!("append workflow event tx: {e}")))?;
            validate_workflow_event_actor_tx(&tx, conversation_id, &actor_id).await?;
            let (event, rapid_update_log) = match resolved_task.as_ref() {
                Some(task) => {
                    apply_workflow_status_update_tx(
                        &tx,
                        task,
                        &actor_id,
                        &request.kind,
                        request.source_message_id.as_deref(),
                        status_update,
                        &request.payload,
                    )
                    .await?
                }
                None => {
                    log_unresolved_workflow_task_reference(
                        conversation_id,
                        Some(actor_id.as_str()),
                        &request.kind,
                        request
                            .payload
                            .get("task_id")
                            .and_then(|value| value.as_str()),
                        request.task_key.is_some(),
                    );
                    let payload = workflow_diagnostic_payload(
                        &request.payload,
                        "workflow_task_unresolved",
                        status_update.status().map(ChannelTaskStatus::as_str),
                        None,
                    );
                    let event = insert_workflow_event_tx(
                        &tx,
                        conversation_id,
                        None,
                        request.source_message_id.as_deref(),
                        Some(actor_id.as_str()),
                        &request.kind,
                        &payload,
                        None,
                    )
                    .await?;
                    (event, None)
                }
            };
            if let Some(task) = resolved_task.as_ref()
                && event.resulting_version.is_some()
            {
                insert_channel_task_fanout_event_tx(
                    &tx,
                    conversation_id,
                    Some(actor_id.as_str()),
                    "channel_task.updated",
                    &task.id,
                )
                .await?;
            }
            tx.commit()
                .await
                .map_err(|e| AppError::Internal(format!("commit workflow event: {e}")))?;

            if event.resulting_version.is_some() {
                record_channel_task_update();
                if let Some(rapid_update_log) = &rapid_update_log {
                    emit_rapid_channel_task_update_log(rapid_update_log);
                }
            }
            Ok(event)
        }
        .await;
        if let Err(error) = &result {
            record_channel_task_mutation_error();
            log_channel_task_mutation_failure(
                "workflow_append_for_conversation",
                default_actor_id,
                Some(conversation_id),
                None,
                None,
                error,
            );
        }
        result
    }

    pub async fn append_group_workflow_event_for_conversation_trusted_system(
        &self,
        conversation_id: &str,
        request: AppendGroupWorkflowEventRequest,
    ) -> Result<GroupWorkflowEvent, AppError> {
        let result: Result<GroupWorkflowEvent, AppError> = async {
            validate_required_text("kind", &request.kind)?;
            validate_object_payload(&request.payload)?;
            self.require_group_conversation(conversation_id).await?;
            let resolved_task_id = self
                .find_task_id_by_workflow_reference(
                    conversation_id,
                    request
                        .payload
                        .get("task_id")
                        .and_then(|value| value.as_str()),
                    request.task_key.as_deref(),
                )
                .await?;
            let resolved_task = match resolved_task_id.as_deref() {
                Some(task_id) => Some(self.get_group_workflow_task(task_id).await?),
                None => None,
            };
            let status_update = status_for_workflow_event(&request.kind, &request.payload);
            let mut client = self.store.connect().await?;
            let tx = client.transaction().await.map_err(|e| {
                AppError::Internal(format!("append trusted workflow event tx: {e}"))
            })?;
            let (event, rapid_update_log) = match resolved_task.as_ref() {
                Some(task) => {
                    apply_trusted_workflow_status_update_tx(
                        &tx,
                        task,
                        &request.kind,
                        request.source_message_id.as_deref(),
                        status_update,
                        &request.payload,
                    )
                    .await?
                }
                None => {
                    log_unresolved_workflow_task_reference(
                        conversation_id,
                        None,
                        &request.kind,
                        request
                            .payload
                            .get("task_id")
                            .and_then(|value| value.as_str()),
                        request.task_key.is_some(),
                    );
                    let payload = workflow_diagnostic_payload(
                        &trusted_system_workflow_payload(&request.payload),
                        "workflow_task_unresolved",
                        status_update.status().map(ChannelTaskStatus::as_str),
                        None,
                    );
                    let event = insert_workflow_event_tx(
                        &tx,
                        conversation_id,
                        None,
                        request.source_message_id.as_deref(),
                        None,
                        &request.kind,
                        &payload,
                        None,
                    )
                    .await?;
                    (event, None)
                }
            };
            if let Some(task) = resolved_task.as_ref()
                && event.resulting_version.is_some()
            {
                insert_channel_task_fanout_event_tx(
                    &tx,
                    conversation_id,
                    None,
                    "channel_task.updated",
                    &task.id,
                )
                .await?;
            }
            tx.commit()
                .await
                .map_err(|e| AppError::Internal(format!("commit trusted workflow event: {e}")))?;

            if event.resulting_version.is_some() {
                record_channel_task_update();
                if let Some(rapid_update_log) = &rapid_update_log {
                    emit_rapid_channel_task_update_log(rapid_update_log);
                }
            }
            Ok(event)
        }
        .await;
        if let Err(error) = &result {
            record_channel_task_mutation_error();
            log_channel_task_mutation_failure(
                "trusted_workflow_append_for_conversation",
                "",
                Some(conversation_id),
                None,
                None,
                error,
            );
        }
        result
    }

    async fn require_group_conversation(&self, conversation_id: &str) -> Result<(), AppError> {
        let conversation = self.get_conversation(conversation_id).await?;
        if !matches!(conversation.conversation_type, ConversationType::Group) {
            return Err(AppError::Validation(
                "workflow tasks require a group conversation".into(),
            ));
        }
        Ok(())
    }

    async fn require_channel_task_conversation_access(
        &self,
        actor_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        self.get_principal(actor_id).await?;
        let conversation = self.get_conversation(conversation_id).await?;
        if conversation.members.contains_key(actor_id) {
            return Ok(());
        }
        Err(AppError::Forbidden(
            "principal is not a member of this conversation".into(),
        ))
    }

    async fn list_task_participants(
        &self,
        task_id: &str,
    ) -> Result<Vec<GroupWorkflowTaskParticipant>, AppError> {
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT task_id, principal_id, role_key, responsibility, required
                 FROM group_workflow_task_participant
                 WHERE task_id = $1
                 ORDER BY role_key ASC, principal_id ASC",
                &[&task_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list workflow task participants: {e}")))?;
        Ok(rows.iter().map(participant_from_row).collect())
    }

    async fn list_task_events(&self, task_id: &str) -> Result<Vec<GroupWorkflowEvent>, AppError> {
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT id, conversation_id, task_id, source_message_id,
                        actor_principal_id, kind, payload, resulting_version, created_at
                 FROM group_workflow_event
                 WHERE task_id = $1
                 ORDER BY created_at ASC, id ASC",
                &[&task_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list workflow task events: {e}")))?;
        Ok(rows.iter().map(event_from_row).collect())
    }

    async fn find_task_id_by_workflow_reference(
        &self,
        conversation_id: &str,
        task_id: Option<&str>,
        task_key: Option<&str>,
    ) -> Result<Option<String>, AppError> {
        if let Some(task_key) = task_key {
            validate_task_key(task_key)?;
        }
        let task_id = task_id.map(str::trim).filter(|task_id| !task_id.is_empty());
        if task_id.is_none() && task_key.is_none() {
            return Ok(None);
        }
        let client = self.store.connect().await?;
        let row = match (task_id, task_key) {
            (Some(task_id), Some(task_key)) => {
                client
                    .query_opt(
                        "SELECT id
                         FROM group_workflow_task
                         WHERE conversation_id = $1
                           AND id = $2
                           AND task_key = $3",
                        &[&conversation_id, &task_id, &task_key],
                    )
                    .await
            }
            (Some(task_id), None) => {
                client
                    .query_opt(
                        "SELECT id
                         FROM group_workflow_task
                         WHERE conversation_id = $1
                           AND id = $2",
                        &[&conversation_id, &task_id],
                    )
                    .await
            }
            (None, Some(task_key)) => {
                client
                    .query_opt(
                        "SELECT id
                         FROM group_workflow_task
                         WHERE conversation_id = $1
                           AND task_key = $2",
                        &[&conversation_id, &task_key],
                    )
                    .await
            }
            (None, None) => unreachable!("checked above"),
        }
        .map_err(|e| {
            AppError::Internal(format!("find workflow task by metadata reference: {e}"))
        })?;
        Ok(row.map(|row| row.get("id")))
    }
}

async fn replace_non_owner_task_participants_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    task_id: &str,
    participants: &[WorkflowTaskParticipantInput],
) -> Result<(), AppError> {
    tx.execute(
        "DELETE FROM group_workflow_task_participant
         WHERE task_id = $1 AND role_key <> 'owner'",
        &[&task_id],
    )
    .await
    .map_err(|e| AppError::Internal(format!("delete workflow participants: {e}")))?;

    for participant in participants {
        insert_task_participant_tx(tx, conversation_id, task_id, participant, false).await?;
    }
    Ok(())
}

async fn sync_owner_participant_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    task_id: &str,
    assignee_principal_id: &str,
) -> Result<(), AppError> {
    tx.execute(
        "DELETE FROM group_workflow_task_participant
         WHERE task_id = $1 AND role_key = 'owner'",
        &[&task_id],
    )
    .await
    .map_err(|e| AppError::Internal(format!("delete workflow owner participant: {e}")))?;
    insert_task_participant_tx(
        tx,
        conversation_id,
        task_id,
        &WorkflowTaskParticipantInput {
            principal_id: assignee_principal_id.to_string(),
            role_key: "owner".to_string(),
            responsibility: None,
            required: true,
        },
        true,
    )
    .await
}

async fn insert_task_participant_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    task_id: &str,
    participant: &WorkflowTaskParticipantInput,
    allow_owner: bool,
) -> Result<(), AppError> {
    validate_required_text("principal_id", &participant.principal_id)?;
    validate_required_text("role_key", &participant.role_key)?;
    if participant.role_key == "owner" && !allow_owner {
        return Err(AppError::Validation(
            "owner participant is synchronized from assignee_principal_id".into(),
        ));
    }
    let participant_id = new_id();
    let inserted = tx
        .query_opt(
            "INSERT INTO group_workflow_task_participant
                (id, task_id, principal_id, role_key, responsibility, required,
                 created_at, updated_at)
             SELECT $1, $2, p.id, $4, $5, $6, NOW(), NOW()
             FROM principal p
             JOIN conversation c ON c.id = $3
             JOIN conversation_member cm
               ON cm.conv_id = c.id
              AND cm.principal_id = p.id
              AND cm.removed_at IS NULL
             WHERE p.id = $7
               AND p.workspace_id = c.workspace_id
               AND p.disabled = FALSE
               AND p.deleted_at IS NULL
             ON CONFLICT (task_id, principal_id, role_key) DO UPDATE
             SET responsibility = EXCLUDED.responsibility,
                 required = EXCLUDED.required,
                 updated_at = NOW()
             RETURNING id",
            &[
                &participant_id,
                &task_id,
                &conversation_id,
                &participant.role_key,
                &participant.responsibility,
                &participant.required,
                &participant.principal_id,
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("insert workflow participant: {e}")))?;
    if inserted.is_none() {
        return Err(AppError::Validation(format!(
            "participant {} must be an active conversation member in the same workspace",
            participant.principal_id
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_workflow_event_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    task_id: Option<&str>,
    source_message_id: Option<&str>,
    actor_principal_id: Option<&str>,
    kind: &str,
    payload: &serde_json::Value,
    resulting_version: Option<i64>,
) -> Result<GroupWorkflowEvent, AppError> {
    let event_id = new_id();
    let row = tx
        .query_one(
            "INSERT INTO group_workflow_event
                (id, conversation_id, task_id, source_message_id,
                 actor_principal_id, kind, payload, resulting_version, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
             RETURNING id, conversation_id, task_id, source_message_id,
                       actor_principal_id, kind, payload, resulting_version, created_at",
            &[
                &event_id,
                &conversation_id,
                &task_id,
                &source_message_id,
                &actor_principal_id,
                &kind,
                &payload,
                &resulting_version,
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("insert workflow event: {e}")))?;
    Ok(event_from_row(&row))
}

async fn insert_channel_task_fanout_event_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    actor_principal_id: Option<&str>,
    event_type: &str,
    task_id: &str,
) -> Result<ChannelTaskSnapshot, AppError> {
    let task = channel_task_snapshot_tx(tx, task_id).await?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
        &[&conversation_id],
    )
    .await
    .map_err(|e| AppError::Internal(format!("channel task fanout advisory lock: {e}")))?;

    let event_id = new_id();
    let sender_id = actor_principal_id.unwrap_or("system");
    let content: Option<String> = None;
    let content_type = "application/vnd.choruz.channel-task+json";
    let client_msg_id: Option<String> = None;
    let turn_id: Option<String> = None;
    let reply_event_id: Option<String> = None;
    let metadata = serde_json::to_value(&task)
        .map(|snapshot| {
            json!({
                "event_type": event_type,
                "conversation_id": task.conversation_id,
                "task_id": task.task_id,
                "version": task.version,
                "updated_at": task.updated_at,
                "task": snapshot,
            })
        })
        .map_err(|e| AppError::Internal(format!("serialize channel task fanout payload: {e}")))?;

    // The WebSocket fanout gateway polls conversation_events directly. Do not
    // enqueue these task-only rows into event_outbox, which is consumed by the
    // router and would turn silent board changes into agent input.
    tx.execute(
        "INSERT INTO conversation_events
                (conversation_id, seq, event_id, event_type, sender_id,
                 content, content_type, metadata, client_msg_id, turn_id,
                 reply_event_id, created_at)
             VALUES (
                $1,
                COALESCE((SELECT MAX(seq) FROM conversation_events WHERE conversation_id = $1), 0) + 1,
                $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW()
             )",
        &[
            &conversation_id,
            &event_id,
            &event_type,
            &sender_id,
            &content,
            &content_type,
            &metadata,
            &client_msg_id,
            &turn_id,
            &reply_event_id,
        ],
    )
    .await
    .map_err(|e| AppError::Internal(format!("insert channel task fanout event: {e}")))?;

    Ok(task)
}

#[allow(clippy::too_many_arguments)]
async fn apply_channel_task_patch_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    existing: &GroupWorkflowTask,
    request: &PatchChannelTaskRequest,
    workflow_kind: Option<&str>,
    event_kind: &str,
    source_message_id: Option<&str>,
    base_payload: &serde_json::Value,
) -> Result<(GroupWorkflowEvent, Option<RapidChannelTaskUpdateLog>), AppError> {
    require_channel_task_member_access_tx(tx, actor_id, &existing.conversation_id).await?;
    let actor_type = channel_task_actor_type_tx(tx, actor_id, &existing.conversation_id).await?;
    let agent_coordinator_authorized = if let Some(workflow_kind) = workflow_kind {
        authorize_workflow_metadata_patch_tx(
            tx,
            actor_id,
            actor_type.as_str(),
            existing,
            workflow_kind,
        )
        .await?;
        false
    } else {
        authorize_channel_task_patch_tx(tx, actor_id, actor_type.as_str(), existing, request)
            .await?
    };

    if let Some(assignee_principal_id) = &request.assignee_principal_id {
        let allow_human_assignee = actor_type == "human" && !agent_coordinator_authorized;
        validate_visible_channel_task_assignee_tx(
            tx,
            &existing.conversation_id,
            assignee_principal_id,
            allow_human_assignee,
            true,
        )
        .await?;
    }

    apply_authorized_channel_task_patch_tx(
        tx,
        Some(actor_id),
        Some(actor_type.as_str()),
        existing,
        request,
        event_kind,
        source_message_id,
        base_payload,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn apply_authorized_channel_task_patch_tx(
    tx: &Transaction<'_>,
    actor_principal_id: Option<&str>,
    actor_type: Option<&str>,
    existing: &GroupWorkflowTask,
    request: &PatchChannelTaskRequest,
    event_kind: &str,
    source_message_id: Option<&str>,
    base_payload: &serde_json::Value,
) -> Result<(GroupWorkflowEvent, Option<RapidChannelTaskUpdateLog>), AppError> {
    let status = request
        .status
        .map(|status| status.as_str().to_string())
        .unwrap_or_else(|| existing.status.clone());
    let assignee_principal_id = request
        .assignee_principal_id
        .clone()
        .unwrap_or_else(|| existing.assignee_principal_id.clone());
    let blocked_reason =
        apply_nullable_patch(existing.blocked_reason.clone(), &request.blocked_reason);
    let context_label =
        apply_nullable_patch(existing.context_label.clone(), &request.context_label);

    let rapid_update_log =
        rapid_channel_task_update_log(actor_principal_id, actor_type, existing, request);
    let row = tx
        .query_one(
            "UPDATE group_workflow_task
             SET status = $2,
                 assignee_principal_id = $3,
                 blocked_reason = $4,
                 context_label = $5,
                 version = version + 1,
                 updated_at = NOW()
             WHERE id = $1
             RETURNING version",
            &[
                &existing.id,
                &status,
                &assignee_principal_id,
                &blocked_reason,
                &context_label,
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("patch channel task: {e}")))?;
    let resulting_version = row.get::<_, i64>("version");
    sync_owner_participant_tx(
        tx,
        &existing.conversation_id,
        &existing.id,
        &assignee_principal_id,
    )
    .await?;

    let mut payload = sanitize_workflow_event_payload(base_payload);
    if let Some(object) = payload.as_object_mut() {
        if actor_principal_id.is_none() {
            object.insert(
                "workflow_authority".to_string(),
                json!({
                    "type": "trusted_system"
                }),
            );
        }
        object.insert(
            "previous".to_string(),
            json!({
                "status": existing.status,
                "assignee_principal_id": existing.assignee_principal_id,
                "blocked_reason": existing.blocked_reason,
                "context_label": existing.context_label,
            }),
        );
        object.insert(
            "new".to_string(),
            json!({
                "status": status,
                "assignee_principal_id": assignee_principal_id,
                "blocked_reason": blocked_reason,
                "context_label": context_label,
            }),
        );
    }

    let event = insert_workflow_event_tx(
        tx,
        &existing.conversation_id,
        Some(&existing.id),
        source_message_id,
        actor_principal_id,
        event_kind,
        &payload,
        Some(resulting_version),
    )
    .await?;
    Ok((event, rapid_update_log))
}

async fn apply_workflow_status_update_tx(
    tx: &Transaction<'_>,
    task: &GroupWorkflowTask,
    actor_id: &str,
    kind: &str,
    source_message_id: Option<&str>,
    status_update: WorkflowStatusEffect,
    payload: &serde_json::Value,
) -> Result<(GroupWorkflowEvent, Option<RapidChannelTaskUpdateLog>), AppError> {
    let status = match status_update {
        WorkflowStatusEffect::Update(status) => status,
        WorkflowStatusEffect::KnownNoop => {
            let locked_task = lock_group_workflow_task_tx(tx, &task.id).await?;
            let actor_type =
                channel_task_actor_type_tx(tx, actor_id, &locked_task.conversation_id).await?;
            match authorize_workflow_metadata_patch_tx(
                tx,
                actor_id,
                actor_type.as_str(),
                &locked_task,
                kind,
            )
            .await
            {
                Ok(()) => {}
                Err(AppError::Forbidden(detail)) => {
                    let payload = workflow_diagnostic_payload(
                        payload,
                        "workflow_status_unauthorized",
                        None,
                        Some(detail.as_str()),
                    );
                    let event = insert_workflow_event_tx(
                        tx,
                        &locked_task.conversation_id,
                        Some(&locked_task.id),
                        source_message_id,
                        Some(actor_id),
                        kind,
                        &payload,
                        None,
                    )
                    .await?;
                    return Ok((event, None));
                }
                Err(error) => return Err(error),
            }
            let payload = workflow_diagnostic_payload(payload, "workflow_status_noop", None, None);
            let event = insert_workflow_event_tx(
                tx,
                &locked_task.conversation_id,
                Some(&locked_task.id),
                source_message_id,
                Some(actor_id),
                kind,
                &payload,
                None,
            )
            .await?;
            return Ok((event, None));
        }
        WorkflowStatusEffect::Unsupported => {
            let payload =
                workflow_diagnostic_payload(payload, "workflow_status_unsupported", None, None);
            let event = insert_workflow_event_tx(
                tx,
                &task.conversation_id,
                Some(&task.id),
                source_message_id,
                Some(actor_id),
                kind,
                &payload,
                None,
            )
            .await?;
            return Ok((event, None));
        }
    };
    let locked_task = lock_group_workflow_task_tx(tx, &task.id).await?;
    let request = PatchChannelTaskRequest {
        status: Some(status),
        assignee_principal_id: None,
        blocked_reason: NullablePatch::Unchanged,
        context_label: NullablePatch::Unchanged,
    };
    match apply_channel_task_patch_tx(
        tx,
        actor_id,
        &locked_task,
        &request,
        Some(kind),
        kind,
        source_message_id,
        payload,
    )
    .await
    {
        Ok((event, rapid_update_log)) => Ok((event, rapid_update_log)),
        Err(AppError::Forbidden(detail)) => {
            let payload = workflow_diagnostic_payload(
                payload,
                "workflow_status_unauthorized",
                Some(status.as_str()),
                Some(detail.as_str()),
            );
            let event = insert_workflow_event_tx(
                tx,
                &locked_task.conversation_id,
                Some(&locked_task.id),
                source_message_id,
                Some(actor_id),
                kind,
                &payload,
                None,
            )
            .await?;
            Ok((event, None))
        }
        Err(error) => Err(error),
    }
}

async fn apply_trusted_workflow_status_update_tx(
    tx: &Transaction<'_>,
    task: &GroupWorkflowTask,
    kind: &str,
    source_message_id: Option<&str>,
    status_update: WorkflowStatusEffect,
    payload: &serde_json::Value,
) -> Result<(GroupWorkflowEvent, Option<RapidChannelTaskUpdateLog>), AppError> {
    match status_update {
        WorkflowStatusEffect::Update(status)
            if workflow_kind_requires_agent_owner_or_coordinator(kind) =>
        {
            let locked_task = lock_group_workflow_task_tx(tx, &task.id).await?;
            let request = PatchChannelTaskRequest {
                status: Some(status),
                assignee_principal_id: None,
                blocked_reason: NullablePatch::Unchanged,
                context_label: NullablePatch::Unchanged,
            };
            apply_authorized_channel_task_patch_tx(
                tx,
                None,
                Some("system"),
                &locked_task,
                &request,
                kind,
                source_message_id,
                &trusted_system_workflow_payload(payload),
            )
            .await
        }
        WorkflowStatusEffect::KnownNoop
            if workflow_kind_requires_agent_owner_or_coordinator(kind) =>
        {
            let locked_task = lock_group_workflow_task_tx(tx, &task.id).await?;
            let payload = workflow_diagnostic_payload(
                &trusted_system_workflow_payload(payload),
                "workflow_status_noop",
                None,
                None,
            );
            let event = insert_workflow_event_tx(
                tx,
                &locked_task.conversation_id,
                Some(&locked_task.id),
                source_message_id,
                None,
                kind,
                &payload,
                None,
            )
            .await?;
            Ok((event, None))
        }
        WorkflowStatusEffect::Update(status) => {
            let payload = workflow_diagnostic_payload(
                &trusted_system_workflow_payload(payload),
                "workflow_status_unauthorized",
                Some(status.as_str()),
                Some("trusted system workflow metadata is not authorized for this kind"),
            );
            let event = insert_workflow_event_tx(
                tx,
                &task.conversation_id,
                Some(&task.id),
                source_message_id,
                None,
                kind,
                &payload,
                None,
            )
            .await?;
            Ok((event, None))
        }
        WorkflowStatusEffect::KnownNoop => {
            let payload = workflow_diagnostic_payload(
                &trusted_system_workflow_payload(payload),
                "workflow_status_unauthorized",
                None,
                Some("trusted system workflow metadata is not authorized for this kind"),
            );
            let event = insert_workflow_event_tx(
                tx,
                &task.conversation_id,
                Some(&task.id),
                source_message_id,
                None,
                kind,
                &payload,
                None,
            )
            .await?;
            Ok((event, None))
        }
        WorkflowStatusEffect::Unsupported => {
            let payload = workflow_diagnostic_payload(
                &trusted_system_workflow_payload(payload),
                "workflow_status_unsupported",
                None,
                None,
            );
            let event = insert_workflow_event_tx(
                tx,
                &task.conversation_id,
                Some(&task.id),
                source_message_id,
                None,
                kind,
                &payload,
                None,
            )
            .await?;
            Ok((event, None))
        }
    }
}

async fn lock_group_workflow_task_tx(
    tx: &Transaction<'_>,
    task_id: &str,
) -> Result<GroupWorkflowTask, AppError> {
    let row = tx
        .query_opt(
            "SELECT id, conversation_id, task_key, title, status,
                    assignee_principal_id, blocked_reason, source_kind, context_label,
                    idempotency_key, idempotency_payload_hash, version,
                    source_message_id, created_by, created_at, updated_at
             FROM group_workflow_task
             WHERE id = $1
             FOR UPDATE",
            &[&task_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("lock workflow task: {e}")))?;
    let row = row.ok_or_else(|| AppError::NotFound("channel task not found".into()))?;
    Ok(task_from_row(&row))
}

fn sanitize_workflow_event_payload(payload: &serde_json::Value) -> serde_json::Value {
    let mut sanitized = payload.clone();
    redact_workflow_event_payload(&mut sanitized);
    sanitized
}

fn redact_workflow_event_payload(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.retain(|key, _| !is_sensitive_workflow_payload_key(key));
            for value in object.values_mut() {
                redact_workflow_event_payload(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_workflow_event_payload(value);
            }
        }
        _ => {}
    }
}

fn is_sensitive_workflow_payload_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "previous"
            | "new"
            | "private"
            | "reasoncode"
            | "workflowauthority"
            | "workflowdiagnostic"
            | "taskid"
            | "sourcemessageid"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "apikey"
            | "authorization"
            | "password"
            | "secret"
            | "privatenote"
            | "note"
            | "path"
            | "workspacepath"
            | "filepath"
            | "directory"
    )
}

fn trusted_system_workflow_payload(payload: &serde_json::Value) -> serde_json::Value {
    let mut sanitized = sanitize_workflow_event_payload(payload);
    if let Some(object) = sanitized.as_object_mut() {
        object.insert(
            "workflow_authority".to_string(),
            json!({
                "type": "trusted_system"
            }),
        );
    }
    sanitized
}

fn workflow_diagnostic_payload(
    payload: &serde_json::Value,
    reason_code: &str,
    status_effect: Option<&str>,
    detail: Option<&str>,
) -> serde_json::Value {
    let mut sanitized = sanitize_workflow_event_payload(payload);
    if let Some(object) = sanitized.as_object_mut() {
        object.insert(
            "workflow_diagnostic".to_string(),
            json!({
                "reason_code": reason_code,
                "status_effect": status_effect,
                "detail": detail,
            }),
        );
    }
    sanitized
}

async fn validate_workflow_event_actor_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    actor_principal_id: &str,
) -> Result<(), AppError> {
    validate_required_text("actor_principal_id", actor_principal_id)?;
    let row = tx
        .query_opt(
            "SELECT p.id
             FROM principal p
             JOIN conversation c ON c.id = $1
             JOIN conversation_member cm
               ON cm.conv_id = c.id
              AND cm.principal_id = p.id
              AND cm.removed_at IS NULL
             WHERE p.id = $2
               AND p.workspace_id = c.workspace_id
               AND p.disabled = FALSE
               AND p.deleted_at IS NULL
               AND NOT (p.type = 'agent' AND p.channel_visibility = 'internal')",
            &[&conversation_id, &actor_principal_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("validate workflow event actor: {e}")))?;
    if row.is_none() {
        return Err(AppError::Validation(
            "actor_principal_id must be an active visible conversation member".into(),
        ));
    }
    Ok(())
}

async fn require_channel_task_member_access_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    conversation_id: &str,
) -> Result<(), AppError> {
    validate_required_text("actor_id", actor_id)?;
    let row = tx
        .query_opt(
            "SELECT 1
             FROM principal p
             JOIN conversation c ON c.id = $2
             JOIN conversation_member cm
               ON cm.conv_id = c.id
              AND cm.principal_id = p.id
              AND cm.removed_at IS NULL
             WHERE p.id = $1
               AND p.workspace_id = c.workspace_id
               AND p.disabled = FALSE
               AND p.deleted_at IS NULL",
            &[&actor_id, &conversation_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("check channel task membership: {e}")))?;
    if row.is_some() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "principal is not a member of this conversation".into(),
        ))
    }
}

async fn require_channel_task_conversation_eligible_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
) -> Result<(), AppError> {
    let row = tx
        .query_one(
            "SELECT c.type,
                    EXISTS (
                        SELECT 1
                        FROM conversation_member cm
                        JOIN principal p ON p.id = cm.principal_id
                        WHERE cm.conv_id = c.id
                          AND cm.removed_at IS NULL
                          AND p.type = 'agent'
                          AND p.channel_visibility <> 'internal'
                          AND p.disabled = FALSE
                          AND p.deleted_at IS NULL
                    ) AS has_agent
             FROM conversation c
             WHERE c.id = $1",
            &[&conversation_id],
        )
        .await
        .map_err(|e| {
            AppError::Internal(format!("check channel task conversation eligibility: {e}"))
        })?;
    let conversation_type: String = row.get("type");
    let has_agent: bool = row.get("has_agent");
    if conversation_type == "group" || (conversation_type == "direct" && has_agent) {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "channel tasks require a group or direct conversation involving an agent".into(),
        ))
    }
}

async fn require_visible_group_agent_actor_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    conversation_id: &str,
) -> Result<(), AppError> {
    validate_required_text("actor_id", actor_id)?;
    let row = tx
        .query_opt(
            "SELECT 1
             FROM principal p
             JOIN conversation c ON c.id = $2
             JOIN conversation_member cm
               ON cm.conv_id = c.id
              AND cm.principal_id = p.id
              AND cm.removed_at IS NULL
             WHERE p.id = $1
               AND c.type = 'group'
               AND p.workspace_id = c.workspace_id
               AND p.type = 'agent'
               AND p.channel_visibility <> 'internal'
               AND p.disabled = FALSE
               AND p.deleted_at IS NULL",
            &[&actor_id, &conversation_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("validate channel task agent actor: {e}")))?;
    if row.is_some() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "generic channel task creation is only available to visible group agents".into(),
        ))
    }
}

async fn channel_task_actor_type_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    conversation_id: &str,
) -> Result<String, AppError> {
    let row = tx
        .query_opt(
            "SELECT p.type
             FROM principal p
             JOIN conversation c ON c.id = $2
             WHERE p.id = $1
               AND p.workspace_id = c.workspace_id
               AND p.disabled = FALSE
               AND p.deleted_at IS NULL",
            &[&actor_id, &conversation_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("load channel task actor type: {e}")))?;
    row.map(|row| row.get("type"))
        .ok_or_else(|| AppError::Forbidden("actor cannot access this channel task".into()))
}

async fn authorize_channel_task_patch_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    actor_type: &str,
    existing: &GroupWorkflowTask,
    request: &PatchChannelTaskRequest,
) -> Result<bool, AppError> {
    match actor_type {
        "human" => Ok(false),
        "agent" => {
            require_visible_group_agent_actor_tx(tx, actor_id, &existing.conversation_id).await?;
            let is_coordinator =
                is_configured_channel_task_coordinator_tx(tx, actor_id, &existing.conversation_id)
                    .await?;
            let is_owner = existing.assignee_principal_id == actor_id;
            if !is_owner && !is_coordinator {
                return Err(AppError::Forbidden(
                    "agent can update only owned group tasks or tasks in a coordinated group"
                        .into(),
                ));
            }
            if request.assignee_principal_id.is_some() && !is_owner && !is_coordinator {
                return Err(AppError::Forbidden(
                    "agent can transfer only owned group tasks or tasks in a coordinated group"
                        .into(),
                ));
            }
            Ok(is_coordinator)
        }
        _ => Err(AppError::Forbidden(
            "unsupported actor type for channel task mutation".into(),
        )),
    }
}

async fn authorize_workflow_metadata_patch_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    actor_type: &str,
    existing: &GroupWorkflowTask,
    workflow_kind: &str,
) -> Result<(), AppError> {
    let is_owner = existing.assignee_principal_id == actor_id
        || actor_has_workflow_role_tx(tx, &existing.id, actor_id, &["owner"]).await?;
    let is_configured_coordinator =
        is_configured_channel_task_coordinator_tx(tx, actor_id, &existing.conversation_id).await?;
    let is_task_role_coordinator =
        actor_has_workflow_role_tx(tx, &existing.id, actor_id, &["coordinator"]).await?;
    let is_coordinator =
        is_configured_coordinator || (actor_type != "agent" && is_task_role_coordinator);
    let is_owner_or_coordinator = is_owner || is_coordinator;

    if workflow_kind_requires_agent_owner_or_coordinator(workflow_kind) {
        if actor_type == "agent" && is_owner_or_coordinator {
            return Ok(());
        }
        return Err(AppError::Forbidden(
            "workflow metadata external check requires an agent owner or coordinator".into(),
        ));
    }

    if is_owner_or_coordinator && workflow_kind_allows_owner_or_coordinator(workflow_kind) {
        return Ok(());
    }
    if workflow_kind == "task.feedback"
        && actor_has_workflow_role_tx(
            tx,
            &existing.id,
            actor_id,
            &["reviewer", "quality_check", "approver"],
        )
        .await?
    {
        return Ok(());
    }
    let detail = match actor_type {
        "agent" => "workflow metadata actor is not authorized for this task role",
        "human" => "workflow metadata human actor is not an owner or coordinator",
        _ => "unsupported workflow metadata actor type",
    };
    Err(AppError::Forbidden(detail.into()))
}

fn workflow_kind_allows_owner_or_coordinator(workflow_kind: &str) -> bool {
    matches!(
        workflow_kind,
        "task.created"
            | "task.started"
            | "task.ready_for_next_step"
            | "task.feedback"
            | "task.cleared"
            | "task.blocked"
            | "human_input_needed"
            | "approval_required"
            | "task.completed"
    )
}

fn workflow_kind_requires_agent_owner_or_coordinator(workflow_kind: &str) -> bool {
    matches!(
        workflow_kind,
        "external_check.failed" | "external_check.passed"
    )
}

async fn actor_has_workflow_role_tx(
    tx: &Transaction<'_>,
    task_id: &str,
    actor_id: &str,
    allowed_roles: &[&str],
) -> Result<bool, AppError> {
    let rows = tx
        .query(
            "SELECT role_key
             FROM group_workflow_task_participant
             WHERE task_id = $1
               AND principal_id = $2",
            &[&task_id, &actor_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("check workflow actor role: {e}")))?;
    Ok(rows.iter().any(|row| {
        let role_key = row.get::<_, String>("role_key");
        allowed_roles.contains(&role_key.as_str())
    }))
}

async fn is_configured_channel_task_coordinator_tx(
    tx: &Transaction<'_>,
    actor_id: &str,
    conversation_id: &str,
) -> Result<bool, AppError> {
    let row = tx
        .query_opt(
            "SELECT 1
             FROM conversation_runtime_policies
             WHERE conversation_id = $1
               AND default_coordinator_agent_id = $2",
            &[&conversation_id, &actor_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("check channel task coordinator: {e}")))?;
    Ok(row.is_some())
}

async fn validate_visible_channel_task_assignee_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    principal_id: &str,
    allow_human: bool,
    allow_agent: bool,
) -> Result<(), AppError> {
    validate_required_text("assignee_principal_id", principal_id)?;
    let row = tx
        .query_opt(
            "SELECT p.type
             FROM principal p
             JOIN conversation c ON c.id = $1
             JOIN conversation_member cm
               ON cm.conv_id = c.id
              AND cm.principal_id = p.id
              AND cm.removed_at IS NULL
             WHERE p.id = $2
               AND p.workspace_id = c.workspace_id
               AND p.disabled = FALSE
               AND p.deleted_at IS NULL
               AND NOT (p.type = 'agent' AND p.channel_visibility = 'internal')",
            &[&conversation_id, &principal_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("validate channel task assignee: {e}")))?;
    let principal_type = row.map(|row| row.get::<_, String>("type")).ok_or_else(|| {
        tracing::warn!(
            event = "channel_task_invalid_assignee_attempt",
            conversation_id,
            assignee_supplied = true,
            reason = "not_visible_active_member",
            allow_human,
            allow_agent,
            "channel task assignee validation failed"
        );
        AppError::Validation("assignee must be a visible active conversation member".into())
    })?;
    if (principal_type == "human" && allow_human) || (principal_type == "agent" && allow_agent) {
        Ok(())
    } else {
        tracing::warn!(
            event = "channel_task_invalid_assignee_attempt",
            conversation_id,
            principal_type = %principal_type,
            reason = "type_not_allowed",
            allow_human,
            allow_agent,
            "channel task assignee type rejected"
        );
        Err(AppError::Validation(
            "assignee type is not allowed for this channel task mutation".into(),
        ))
    }
}

async fn validate_visible_message_source_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    message_id: &str,
) -> Result<(), AppError> {
    let row = tx
        .query_opt(
            "SELECT 1
             FROM conversation_events
             WHERE conversation_id = $1
               AND event_id = $2
               AND event_type IN ('message', 'message.created', 'reply')",
            &[&conversation_id, &message_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("validate channel task source message: {e}")))?;
    if row.is_some() {
        Ok(())
    } else {
        Err(AppError::Validation(
            "source message must be a visible message in the same conversation".into(),
        ))
    }
}

async fn find_message_derived_task_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    actor_id: &str,
    message_id: &str,
) -> Result<Option<String>, AppError> {
    tx.query_opt(
        "SELECT id
         FROM group_workflow_task
         WHERE conversation_id = $1
           AND source_kind = 'message'
           AND source_message_id = $2
           AND created_by = $3",
        &[&conversation_id, &message_id, &actor_id],
    )
    .await
    .map(|row| row.map(|row| row.get("id")))
    .map_err(|e| AppError::Internal(format!("find message-derived channel task: {e}")))
}

async fn find_agent_idempotent_task_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
    actor_id: &str,
    idempotency_key: &str,
) -> Result<Option<(String, Option<String>)>, AppError> {
    tx.query_opt(
        "SELECT id, idempotency_payload_hash
         FROM group_workflow_task
         WHERE conversation_id = $1
           AND created_by = $2
           AND idempotency_key = $3",
        &[&conversation_id, &actor_id, &idempotency_key],
    )
    .await
    .map(|row| {
        row.map(|row| {
            (
                row.get::<_, String>("id"),
                row.get::<_, Option<String>>("idempotency_payload_hash"),
            )
        })
    })
    .map_err(|e| AppError::Internal(format!("find idempotent channel task: {e}")))
}

async fn next_channel_task_sequence_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
) -> Result<i64, AppError> {
    let row = tx
        .query_one(
            "INSERT INTO channel_task_sequence (conversation_id, next_value)
             VALUES ($1, 2)
             ON CONFLICT (conversation_id) DO UPDATE
             SET next_value = channel_task_sequence.next_value + 1
             RETURNING next_value - 1 AS sequence",
            &[&conversation_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("next channel task sequence: {e}")))?;
    Ok(row.get("sequence"))
}

const ALLOCATE_TASK_KEY_MAX_ATTEMPTS: u32 = 64;
const GENERATED_TASK_INSERT_MAX_ATTEMPTS: u32 = 8;

// Allocate a fresh `TASK-{N}` key, skipping sequence values that collide with
// explicitly-keyed cards already in the conversation. A unique violation on
// the outer INSERT rolls the sequence increment back together with the row,
// so without this skip an explicit `TASK-N` card would permanently wedge the
// auto-allocation path on that sequence value.
async fn allocate_unused_task_key_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
) -> Result<String, AppError> {
    for _ in 0..ALLOCATE_TASK_KEY_MAX_ATTEMPTS {
        let sequence = next_channel_task_sequence_tx(tx, conversation_id).await?;
        let candidate = format!("TASK-{sequence}");
        let existing = tx
            .query_opt(
                "SELECT 1 FROM group_workflow_task
                 WHERE conversation_id = $1 AND task_key = $2",
                &[&conversation_id, &candidate],
            )
            .await
            .map_err(|e| AppError::Internal(format!("probe channel task key: {e}")))?;
        if existing.is_none() {
            return Ok(candidate);
        }
    }
    allocate_task_key_after_existing_max_tx(tx, conversation_id).await
}

async fn allocate_task_key_after_existing_max_tx(
    tx: &Transaction<'_>,
    conversation_id: &str,
) -> Result<String, AppError> {
    let row = tx
        .query_one(
            "WITH numeric_task_keys AS (
                SELECT (regexp_match(task_key, '^TASK-([0-9]+)$'))[1]::bigint AS sequence
                FROM group_workflow_task
                WHERE conversation_id = $1
                  AND task_key ~ '^TASK-[0-9]+$'
             ),
             next_candidate AS (
                SELECT COALESCE(MAX(sequence), 0) + 1 AS sequence
                FROM numeric_task_keys
             )
             INSERT INTO channel_task_sequence (conversation_id, next_value)
             SELECT $1, sequence + 1
             FROM next_candidate
             ON CONFLICT (conversation_id) DO UPDATE
             SET next_value = GREATEST(channel_task_sequence.next_value + 1, EXCLUDED.next_value)
             RETURNING next_value - 1 AS sequence",
            &[&conversation_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("advance channel task sequence: {e}")))?;
    let sequence: i64 = row.get("sequence");
    Ok(format!("TASK-{sequence}"))
}

fn validate_non_owner_participants(
    participants: &[WorkflowTaskParticipantInput],
) -> Result<(), AppError> {
    for participant in participants {
        validate_non_owner_participant(participant)?;
    }
    Ok(())
}

fn validate_non_owner_participant(
    participant: &WorkflowTaskParticipantInput,
) -> Result<(), AppError> {
    if participant.role_key == "owner" {
        return Err(AppError::Validation(
            "owner participant is synchronized from assignee_principal_id".into(),
        ));
    }
    Ok(())
}

fn validate_patch_request(request: &PatchChannelTaskRequest) -> Result<(), AppError> {
    if let Some(assignee_principal_id) = &request.assignee_principal_id {
        validate_required_text("assignee_principal_id", assignee_principal_id)?;
    }
    if let NullablePatch::Set(blocked_reason) = &request.blocked_reason {
        validate_required_text("blocked_reason", blocked_reason)?;
    }
    if let NullablePatch::Set(context_label) = &request.context_label {
        validate_required_text("context_label", context_label)?;
    }
    if request.status.is_none()
        && request.assignee_principal_id.is_none()
        && request.blocked_reason.is_unchanged()
        && request.context_label.is_unchanged()
    {
        return Err(AppError::Validation(
            "patch must include at least one mutable channel task field".into(),
        ));
    }
    Ok(())
}

fn apply_nullable_patch(current: Option<String>, patch: &NullablePatch<String>) -> Option<String> {
    match patch {
        NullablePatch::Unchanged => current,
        NullablePatch::Clear => None,
        NullablePatch::Set(value) => Some(value.clone()),
    }
}

fn channel_task_create_payload_hash(
    task_key: &str,
    title: &str,
    assignee_principal_id: &str,
    status: ChannelTaskStatus,
    context_label: Option<&str>,
) -> Result<String, AppError> {
    let canonical = json!({
        "assignee_principal_id": assignee_principal_id,
        "context_label": context_label,
        "source_kind": "agent",
        "status": status.as_str(),
        "task_key": task_key,
        "title": title,
    });
    let bytes = serde_json::to_vec(&canonical).map_err(|e| {
        AppError::Internal(format!("serialize channel task idempotency payload: {e}"))
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn channel_task_snapshot_select_sql(where_clause: &str) -> String {
    format!(
        "
        SELECT gwt.id AS task_id,
               gwt.conversation_id,
               gwt.task_key,
               gwt.title,
               gwt.status,
               assignee.id AS assignee_principal_id,
               assignee.type AS assignee_type,
               assignee.name AS assignee_name,
               gwt.blocked_reason,
               gwt.context_label,
               gwt.source_kind,
               source_message.event_id AS source_message_id,
               creator.id AS created_by,
               creator.type AS created_by_type,
               NULL::TEXT AS updated_by,
               NULL::TEXT AS updated_by_type,
               gwt.version,
               gwt.created_at,
               gwt.updated_at
        FROM group_workflow_task gwt
        JOIN conversation c ON c.id = gwt.conversation_id
        LEFT JOIN conversation_events source_message
          ON source_message.conversation_id = gwt.conversation_id
         AND source_message.event_id = gwt.source_message_id
         AND source_message.event_type IN ('message', 'message.created', 'reply')
        LEFT JOIN principal assignee
          ON assignee.id = gwt.assignee_principal_id
         AND assignee.workspace_id = c.workspace_id
         AND assignee.disabled = FALSE
         AND assignee.deleted_at IS NULL
         AND NOT (assignee.type = 'agent' AND assignee.channel_visibility = 'internal')
         AND EXISTS (
            SELECT 1 FROM conversation_member cm
            WHERE cm.conv_id = gwt.conversation_id
              AND cm.principal_id = assignee.id
              AND cm.removed_at IS NULL
         )
        LEFT JOIN principal creator
          ON creator.id = gwt.created_by
         AND creator.workspace_id = c.workspace_id
         AND creator.disabled = FALSE
         AND creator.deleted_at IS NULL
         AND NOT (creator.type = 'agent' AND creator.channel_visibility = 'internal')
         AND EXISTS (
            SELECT 1 FROM conversation_member cm
            WHERE cm.conv_id = gwt.conversation_id
              AND cm.principal_id = creator.id
              AND cm.removed_at IS NULL
         )
        {where_clause}
        "
    )
}

async fn channel_task_snapshot_tx(
    tx: &Transaction<'_>,
    task_id: &str,
) -> Result<ChannelTaskSnapshot, AppError> {
    let row = tx
        .query_opt(
            channel_task_snapshot_select_sql("WHERE gwt.id = $1").as_str(),
            &[&task_id],
        )
        .await
        .map_err(|e| AppError::Internal(format!("load channel task fanout snapshot: {e}")))?;
    let row = row.ok_or_else(|| AppError::NotFound("channel task not found".into()))?;
    channel_task_snapshot_from_row(&row)
}

fn channel_task_snapshot_from_row(row: &Row) -> Result<ChannelTaskSnapshot, AppError> {
    let status = ChannelTaskStatus::from_str(row.get::<_, String>("status").as_str())
        .map_err(AppError::Internal)?;
    let source_kind = ChannelTaskSourceKind::from_str(row.get::<_, String>("source_kind").as_str())
        .map_err(AppError::Internal)?;
    Ok(ChannelTaskSnapshot {
        task_id: row.get("task_id"),
        conversation_id: row.get("conversation_id"),
        task_key: row.get("task_key"),
        title: row.get("title"),
        status,
        assignee_principal_id: row.get("assignee_principal_id"),
        assignee_type: parse_principal_type_opt(row.get("assignee_type"))?,
        assignee_name: row.get("assignee_name"),
        blocked_reason: row.get("blocked_reason"),
        context_label: row.get("context_label"),
        source_kind,
        source_message_id: row.get("source_message_id"),
        created_by: row.get("created_by"),
        created_by_type: parse_principal_type_opt(row.get("created_by_type"))?,
        updated_by: row.get("updated_by"),
        updated_by_type: parse_principal_type_opt(row.get("updated_by_type"))?,
        version: row.get("version"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn channel_task_event_projection_from_row(
    row: &Row,
) -> Result<ChannelTaskEventProjection, AppError> {
    let payload: Value = row.get("payload");
    let previous = parse_event_visible_values(
        payload.get("previous"),
        row.get("previous_assignee_visible_id"),
        row.get("previous_source_message_visible_id"),
    )?;
    let new = parse_event_visible_values(
        payload.get("new"),
        row.get("new_assignee_visible_id"),
        row.get("new_source_message_visible_id"),
    )?;
    let status_effect = new.as_ref().and_then(|values| values.status);
    Ok(ChannelTaskEventProjection {
        event_id: row.get("id"),
        task_id: row.get("task_id"),
        kind: "channel_task.workflow_event".to_string(),
        actor_principal_id: row.get("actor_principal_id"),
        actor_type: parse_principal_type_opt(row.get("actor_type"))?,
        created_at: row.get("created_at"),
        resulting_version: row.get("resulting_version"),
        previous,
        new,
        workflow_kind: Some(row.get("kind")),
        status_effect,
        reason_code: None,
    })
}

fn parse_event_visible_values(
    value: Option<&Value>,
    visible_assignee_id: Option<String>,
    visible_source_message_id: Option<String>,
) -> Result<Option<ChannelTaskEventVisibleValues>, AppError> {
    let mut values: Option<ChannelTaskEventVisibleValues> = value
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| AppError::Internal(format!("parse channel task event visible values: {e}")))?;
    if let Some(values) = values.as_mut() {
        if let crate::NullablePatch::Set(assignee_id) = &values.assignee_principal_id
            && visible_assignee_id.as_deref() != Some(assignee_id.as_str())
        {
            values.assignee_principal_id = crate::NullablePatch::Unchanged;
        }
        if let crate::NullablePatch::Set(source_message_id) = &values.source_message_id
            && visible_source_message_id.as_deref() != Some(source_message_id.as_str())
        {
            values.source_message_id = crate::NullablePatch::Unchanged;
        }
    }
    Ok(values)
}

fn parse_principal_type_opt(value: Option<String>) -> Result<Option<PrincipalType>, AppError> {
    value
        .as_deref()
        .map(|principal_type| match principal_type {
            "human" => Ok(PrincipalType::Human),
            "agent" => Ok(PrincipalType::Agent),
            other => Err(AppError::Internal(format!(
                "invalid principal type in channel task projection: {other}"
            ))),
        })
        .transpose()
}

fn task_from_row(row: &Row) -> GroupWorkflowTask {
    GroupWorkflowTask {
        id: row.get("id"),
        conversation_id: row.get("conversation_id"),
        task_key: row.get("task_key"),
        title: row.get("title"),
        status: row.get("status"),
        assignee_principal_id: row.get("assignee_principal_id"),
        blocked_reason: row.get("blocked_reason"),
        source_kind: row.get("source_kind"),
        context_label: row.get("context_label"),
        idempotency_key: row.get("idempotency_key"),
        idempotency_payload_hash: row.get("idempotency_payload_hash"),
        version: row.get("version"),
        source_message_id: row.get("source_message_id"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        participants: Vec::new(),
        events: Vec::new(),
    }
}

fn participant_from_row(row: &Row) -> GroupWorkflowTaskParticipant {
    GroupWorkflowTaskParticipant {
        task_id: row.get("task_id"),
        principal_id: row.get("principal_id"),
        role_key: row.get("role_key"),
        responsibility: row.get("responsibility"),
        required: row.get("required"),
    }
}

fn event_from_row(row: &Row) -> GroupWorkflowEvent {
    GroupWorkflowEvent {
        id: row.get("id"),
        conversation_id: row.get("conversation_id"),
        task_id: row.get("task_id"),
        source_message_id: row.get("source_message_id"),
        actor_principal_id: row.get("actor_principal_id"),
        kind: row.get("kind"),
        payload: row.get("payload"),
        resulting_version: row.get("resulting_version"),
        created_at: row.get("created_at"),
    }
}

fn validate_task_key(task_key: &str) -> Result<(), AppError> {
    validate_required_text("task_key", task_key)?;
    if task_key.len() > 128 {
        return Err(AppError::Validation("task_key is too long".into()));
    }
    Ok(())
}

fn validate_required_text(field: &str, value: &str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        return Err(AppError::Validation(format!("{field} is required")));
    }
    Ok(())
}

fn validate_meaningful_title(title: &str) -> Result<(), AppError> {
    validate_required_text("title", title)?;
    if title.chars().any(char::is_alphanumeric) {
        Ok(())
    } else {
        Err(AppError::Validation(
            "title must include at least one letter or number".into(),
        ))
    }
}

fn validate_task_status(status: &str) -> Result<(), AppError> {
    if VALID_TASK_STATUSES.contains(&status) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "invalid workflow task status: {status}"
        )))
    }
}

fn validate_object_payload(payload: &serde_json::Value) -> Result<(), AppError> {
    if payload.is_object() {
        Ok(())
    } else {
        Err(AppError::Validation(
            "workflow event payload must be a JSON object".into(),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowStatusEffect {
    Update(ChannelTaskStatus),
    KnownNoop,
    Unsupported,
}

impl WorkflowStatusEffect {
    fn status(self) -> Option<ChannelTaskStatus> {
        match self {
            Self::Update(status) => Some(status),
            Self::KnownNoop | Self::Unsupported => None,
        }
    }
}

fn status_for_workflow_event(kind: &str, payload: &serde_json::Value) -> WorkflowStatusEffect {
    match kind {
        "task.created" => match explicit_workflow_status(payload) {
            ExplicitWorkflowStatus::Absent => WorkflowStatusEffect::Update(ChannelTaskStatus::Todo),
            ExplicitWorkflowStatus::Supported(status) => WorkflowStatusEffect::Update(status),
            ExplicitWorkflowStatus::Unsupported => WorkflowStatusEffect::Unsupported,
        },
        "task.started" | "task.cleared" | "task.feedback" => {
            WorkflowStatusEffect::Update(ChannelTaskStatus::InProgress)
        }
        "task.ready_for_next_step" => {
            if workflow_next_role_is_review_like(payload) {
                WorkflowStatusEffect::Update(ChannelTaskStatus::InReview)
            } else {
                WorkflowStatusEffect::Update(ChannelTaskStatus::InProgress)
            }
        }
        "task.blocked" | "external_check.failed" | "human_input_needed" => {
            WorkflowStatusEffect::Update(ChannelTaskStatus::Blocked)
        }
        "approval_required" => WorkflowStatusEffect::Update(ChannelTaskStatus::InReview),
        "task.completed" => WorkflowStatusEffect::Update(ChannelTaskStatus::Done),
        "external_check.passed" => WorkflowStatusEffect::KnownNoop,
        _ => WorkflowStatusEffect::Unsupported,
    }
}

enum ExplicitWorkflowStatus {
    Absent,
    Supported(ChannelTaskStatus),
    Unsupported,
}

fn explicit_workflow_status(payload: &serde_json::Value) -> ExplicitWorkflowStatus {
    let Some(status) = payload
        .get("status")
        .or_else(|| payload.pointer("/workflow/status"))
        .or_else(|| payload.pointer("/new/status"))
        .or_else(|| payload.pointer("/workflow/new/status"))
        .and_then(|value| value.as_str())
    else {
        return ExplicitWorkflowStatus::Absent;
    };
    match workflow_status_value(status) {
        Some(status) => ExplicitWorkflowStatus::Supported(status),
        None => ExplicitWorkflowStatus::Unsupported,
    }
}

fn workflow_status_value(status: &str) -> Option<ChannelTaskStatus> {
    match status {
        "pending" | "todo" => Some(ChannelTaskStatus::Todo),
        "in_progress" => Some(ChannelTaskStatus::InProgress),
        "blocked" | "needs_human" => Some(ChannelTaskStatus::Blocked),
        "needs_approval" | "in_review" => Some(ChannelTaskStatus::InReview),
        "completed" | "done" => Some(ChannelTaskStatus::Done),
        _ => None,
    }
}

fn workflow_next_role_is_review_like(payload: &serde_json::Value) -> bool {
    payload
        .get("next_role")
        .or_else(|| payload.pointer("/workflow/next_role"))
        .and_then(|value| value.as_str())
        .map(|role| {
            matches!(
                role,
                "reviewer" | "review" | "quality" | "quality_check" | "approver"
            )
        })
        .unwrap_or(false)
}

fn is_unique_violation(error: &tokio_postgres::Error) -> bool {
    error
        .code()
        .is_some_and(|code| *code == tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
}

#[cfg(test)]
mod observability_tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn workflow_kind_log_label_is_bounded() {
        assert_eq!(safe_workflow_kind_label("task.started"), "task.started");
        assert_eq!(
            safe_workflow_kind_label("secret-token-in-unsupported-kind"),
            "unsupported"
        );
    }

    #[test]
    fn rapid_update_changed_fields_are_bounded() {
        let task = GroupWorkflowTask {
            id: "task-1".into(),
            conversation_id: "conversation-1".into(),
            task_key: "raw-key-not-logged".into(),
            title: "raw title".into(),
            status: "todo".into(),
            assignee_principal_id: "agent-1".into(),
            blocked_reason: Some("raw blocked reason".into()),
            source_kind: "agent".into(),
            context_label: None,
            idempotency_key: None,
            idempotency_payload_hash: None,
            version: 2,
            source_message_id: None,
            created_by: Some("agent-1".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            participants: Vec::new(),
            events: Vec::new(),
        };
        let request = PatchChannelTaskRequest {
            status: Some(ChannelTaskStatus::InProgress),
            assignee_principal_id: Some("agent-2".into()),
            blocked_reason: NullablePatch::Clear,
            context_label: NullablePatch::Set("raw context".into()),
        };

        assert_eq!(
            channel_task_patch_changed_fields(&task, &request),
            "status,assignee,blocked_reason,context_label"
        );
    }

    #[test]
    fn rapid_update_log_ignores_future_db_timestamp() {
        let task = GroupWorkflowTask {
            id: "task-1".into(),
            conversation_id: "conversation-1".into(),
            task_key: "raw-key-not-logged".into(),
            title: "raw title".into(),
            status: "todo".into(),
            assignee_principal_id: "agent-1".into(),
            blocked_reason: None,
            source_kind: "agent".into(),
            context_label: None,
            idempotency_key: None,
            idempotency_payload_hash: None,
            version: 2,
            source_message_id: None,
            created_by: Some("agent-1".into()),
            created_at: Utc::now(),
            updated_at: Utc::now() + Duration::seconds(1),
            participants: Vec::new(),
            events: Vec::new(),
        };
        let request = PatchChannelTaskRequest {
            status: Some(ChannelTaskStatus::InProgress),
            assignee_principal_id: None,
            blocked_reason: NullablePatch::Unchanged,
            context_label: NullablePatch::Unchanged,
        };

        assert!(
            rapid_channel_task_update_log(Some("agent-1"), Some("agent"), &task, &request)
                .is_none()
        );
    }

    #[test]
    fn workflow_diagnostic_payload_redacts_sensitive_keys() {
        let payload = json!({
            "status": "done",
            "task_id": "raw-task-id",
            "note": "private diagnostic text",
            "api_key": "secret-api-key",
            "accessToken": "secret-access-token",
            "refresh-token": "secret-refresh-token",
            "authorization": "Bearer secret",
            "workspace_path": "/private/workspace",
            "workspacePath": "/private/workspace-camel",
            "filePath": "/private/file",
            "private": "private field",
            "nested": {
                "token": "secret-token",
                "privateNote": "private nested note",
                "next_role": "reviewer"
            },
            "previous": { "status": "todo" },
            "new": { "status": "done" }
        });

        let sanitized = workflow_diagnostic_payload(
            &payload,
            "workflow_status_unauthorized",
            Some("done"),
            None,
        );

        assert_eq!(sanitized["status"], "done");
        assert_eq!(sanitized["nested"]["next_role"], "reviewer");
        assert_eq!(
            sanitized["workflow_diagnostic"]["reason_code"],
            "workflow_status_unauthorized"
        );
        for key in [
            "task_id",
            "note",
            "api_key",
            "accessToken",
            "refresh-token",
            "authorization",
            "workspace_path",
            "workspacePath",
            "filePath",
            "private",
            "previous",
            "new",
        ] {
            assert_eq!(sanitized.get(key), None, "{key} should be redacted");
        }
        assert_eq!(sanitized["nested"].get("token"), None);
        assert_eq!(sanitized["nested"].get("privateNote"), None);
    }
}
