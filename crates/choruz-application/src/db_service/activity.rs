use choruz_common::{AppError, new_id};
use choruz_domain::Principal;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::DbService;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySource {
    #[default]
    Telemetry,
    Audit,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ActivityCursor {
    pub time: DateTime<Utc>,
    pub id: String,
}

pub struct ActivityFilter {
    pub source: ActivitySource,
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub trace_id: Option<String>,
}

impl ActivityFilter {
    fn validate(&self) -> Result<(), AppError> {
        if self.since >= self.until || self.until - self.since > Duration::days(31) {
            return Err(AppError::Validation(
                "activity range must be positive and at most 31 days".into(),
            ));
        }
        if self
            .trace_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return Err(AppError::Validation(
                "invalid activity trace identifier".into(),
            ));
        }
        Ok(())
    }

    // Identifiers are static, never derived from request text. Audit payloads
    // are projected explicitly: old diagnostic metadata can contain content.
    fn query_parts(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self.source {
            ActivitySource::Telemetry => (
                "telemetry_event",
                "principal_id",
                "trace_id",
                "jsonb_build_object('event_id', event_id, 'schema_version', schema_version, 'trace_id', trace_id, 'span_id', span_id, 'session_id', session_id, 'name', name, 'occurred_at', occurred_at, 'duration_ms', duration_ms, 'data', data)",
            ),
            ActivitySource::Audit => (
                "audit_log",
                "actor_id",
                "metadata->>'trace_id'",
                "jsonb_build_object('name', action, 'target_type', target_type, 'target_id', target_id, 'trace_id', metadata->>'trace_id', 'data', jsonb_strip_nulls(jsonb_build_object('conversation_id', metadata->'conversation_id', 'agent_id', metadata->'agent_id', 'company_id', metadata->'company_id', 'runtime_host_id', metadata->'runtime_host_id', 'harness_account_id', metadata->'harness_account_id', 'attachment_id', metadata->'attachment_id', 'submission_id', metadata->'submission_id', 'outcome', metadata->'outcome', 'input_bytes', metadata->'input_bytes', 'output_bytes', metadata->'output_bytes', 'resize_count', metadata->'resize_count', 'duration_ms', metadata->'duration_ms')))",
            ),
        }
    }
}

impl DbService {
    async fn activity_workspaces(&self, principal: &Principal) -> Result<Vec<String>, AppError> {
        let mut ids = vec![principal.workspace_id.clone()];
        ids.extend(
            self.list_companies(&principal.id)
                .await?
                .into_iter()
                .map(|c| c.id),
        );
        Ok(ids)
    }
    /// Read the authenticated actor's records in currently accessible workspaces,
    /// ordered by receipt time and ID descending. The cursor is exclusive.
    /// Invalid ranges/pages return Validation; storage failures are propagated.
    /// The gateway owns human authorization, export sanitization and read auditing.
    pub async fn activity_page(
        &self,
        principal: &Principal,
        filter: &ActivityFilter,
        cursor: Option<&ActivityCursor>,
        limit: i64,
    ) -> Result<(Vec<Value>, Option<ActivityCursor>), AppError> {
        filter.validate()?;
        if !(1..=500).contains(&limit)
            || cursor.is_some_and(|c| c.id.is_empty() || c.id.len() > 128)
        {
            return Err(AppError::Validation(
                "invalid activity page size or cursor".into(),
            ));
        }
        let (table, actor, trace, projection) = filter.query_parts();
        let workspaces = self.activity_workspaces(principal).await?;
        let client = self.store.connect().await?;
        let rows = client
            .query(
                &format!(
                    "SELECT id::text AS record_id, created_at, {projection} AS record FROM {table}
            WHERE workspace_id=ANY($1) AND {actor}=$2 AND created_at >= $3 AND created_at < $4
              AND ($5::text IS NULL OR {trace}=$5)
              AND ($6::timestamptz IS NULL OR (created_at,id::text) < ($6,$7))
            ORDER BY created_at DESC,id::text DESC LIMIT $8"
                ),
                &[
                    &workspaces,
                    &principal.id,
                    &filter.since,
                    &filter.until,
                    &filter.trace_id,
                    &cursor.map(|c| c.time),
                    &cursor.map(|c| &c.id),
                    &(limit + 1),
                ],
            )
            .await
            .map_err(|e| AppError::Internal(format!("query activity: {e}")))?;
        let has_more = rows.len() > limit as usize;
        let mut next = None;
        let records = rows
            .into_iter()
            .take(limit as usize)
            .map(|row| {
                let id: String = row.get("record_id");
                let time: DateTime<Utc> = row.get("created_at");
                let mut record: Value = row.get("record");
                record["id"] = json!(id);
                record["received_at"] = json!(time);
                record["source"] = json!(filter.source);
                if has_more {
                    next = Some(ActivityCursor { time, id });
                }
                record
            })
            .collect();
        Ok((records, next))
    }

    /// Count observed events, not inferred user outcomes. At most 200 named
    /// groups are returned with explicit truncation. Uses the same actor scope
    /// and range validation as activity_page; the gateway audits the read.
    pub async fn activity_summary(
        &self,
        principal: &Principal,
        filter: &ActivityFilter,
    ) -> Result<Value, AppError> {
        filter.validate()?;
        let (table, actor, trace, _) = filter.query_parts();
        let workspaces = self.activity_workspaces(principal).await?;
        let (name, outcome, duration) = match filter.source {
            ActivitySource::Telemetry => ("name", "data->>'outcome'", "duration_ms"),
            ActivitySource::Audit => ("action", "metadata->>'outcome'", "NULL::bigint"),
        };
        let client = self.store.connect().await?;
        let rows = client.query(&format!("SELECT {name} AS name,count(*) AS count,
            count(*) FILTER (WHERE {outcome}='failed') AS failed, avg({duration})::double precision AS mean_duration_ms
            FROM {table} WHERE workspace_id=ANY($1) AND {actor}=$2 AND created_at >= $3 AND created_at < $4
              AND ($5::text IS NULL OR {trace}=$5)
            GROUP BY {name} ORDER BY count DESC,{name} LIMIT 201"),
            &[&workspaces, &principal.id, &filter.since, &filter.until, &filter.trace_id]).await
            .map_err(|e| AppError::Internal(format!("summarize activity: {e}")))?;
        let truncated = rows.len() > 200;
        let groups: Vec<Value> = rows.into_iter().take(200).map(|r| json!({"name":r.get::<_,String>("name"), "count":r.get::<_,i64>("count"), "failed":r.get::<_,i64>("failed"), "mean_duration_ms":r.get::<_,Option<f64>>("mean_duration_ms")})).collect();
        Ok(
            json!({"source":filter.source,"since":filter.since,"until":filter.until,"groups":groups,"truncated":truncated}),
        )
    }

    /// Telemetry retention never removes canonical conversation or audit data.
    /// Applied batches and their audit marker commit or roll back together.
    pub async fn prune_activity(
        &self,
        principal: &Principal,
        before: DateTime<Utc>,
        apply: bool,
    ) -> Result<Value, AppError> {
        if before > Utc::now() - Duration::hours(24) {
            return Err(AppError::Validation(
                "activity cutoff must be at least 24 hours old".into(),
            ));
        }
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("begin activity retention: {e}")))?;
        let rows = tx.query("SELECT id FROM telemetry_event WHERE workspace_id=$1 AND principal_id=$2 AND created_at < $3 ORDER BY created_at,id LIMIT 1001 FOR UPDATE", &[&principal.workspace_id,&principal.id,&before]).await
            .map_err(|e| AppError::Internal(format!("select activity retention batch: {e}")))?;
        let has_more = rows.len() > 1000;
        let ids: Vec<i64> = rows.into_iter().take(1000).map(|r| r.get(0)).collect();
        if apply {
            tx.execute("DELETE FROM telemetry_event WHERE workspace_id=$1 AND principal_id=$2 AND id=ANY($3)", &[&principal.workspace_id,&principal.id,&ids]).await
                .map_err(|e| AppError::Internal(format!("delete activity batch: {e}")))?;
            tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata) VALUES($1,$2,$3,'activity.pruned','principal',$3,$4)",
                &[&new_id(),&principal.workspace_id,&principal.id,&json!({"before":before,"count":ids.len()})]).await
                .map_err(|e| AppError::Internal(format!("audit activity retention: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit activity retention: {e}")))?;
        Ok(json!({"applied":apply,"count":ids.len(),"has_more":has_more,"before":before}))
    }
}
