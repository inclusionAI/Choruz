use choruz_common::{AppError, new_id, now};
use choruz_domain::AuditLog;
use serde_json::Value;

use super::DbService;
use super::helpers::row_to_audit_log;

impl DbService {
    /// Read process-wide gauges from their durable owners, not an incomplete
    /// in-memory identity projection. A storage failure fails the scrape.
    pub async fn metrics_snapshot(&self) -> Result<crate::MetricsSnapshot, AppError> {
        let row = self.store.connect().await?.query_one(
            "SELECT (SELECT COUNT(*) FROM principal WHERE deleted_at IS NULL) AS principals,
                    (SELECT COUNT(*) FROM conversation) AS conversations,
                    (SELECT COUNT(*) FROM conversation_events WHERE event_type IN ('message','message.created','reply')) AS messages,
                    (SELECT COUNT(*) FROM audit_log) AS audits,
                    (SELECT COUNT(*) FROM outbox_event WHERE acknowledged_at IS NULL) AS backlog", &[],
        ).await.map_err(|e|AppError::Internal(format!("metrics snapshot: {e}")))?;
        Ok(crate::MetricsSnapshot {
            principals_total: row.get::<_, i64>("principals") as usize,
            conversations_total: row.get::<_, i64>("conversations") as usize,
            messages_total: row.get::<_, i64>("messages") as usize,
            audit_logs_total: row.get::<_, i64>("audits") as usize,
            event_backlog_total: row.get::<_, i64>("backlog") as usize,
        })
    }

    // ── Audit log reads (Phase 1D) ──────────────────────────────────────

    /// List audit logs for a workspace from the database.
    pub async fn list_audit_logs(&self, workspace_id: &str) -> Result<Vec<AuditLog>, AppError> {
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT id, workspace_id, actor_id, action, target_type, target_id,
                        metadata, created_at
                 FROM audit_log
                 WHERE workspace_id = $1
                 ORDER BY created_at DESC
                 LIMIT 10000",
                &[&workspace_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_audit_logs: {e}")))?;

        Ok(rows.iter().map(row_to_audit_log).collect())
    }

    // ── Audit log writes (Phase 2A) ────────────────────────────────────

    /// Record an audit log entry directly into the database.
    pub async fn record_audit(
        &self,
        workspace_id: &str,
        actor_id: &str,
        action: &str,
        target_type: &str,
        target_id: &str,
        metadata: Value,
    ) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        let id = new_id();
        let ts = now();
        client
            .execute(
                "INSERT INTO audit_log (id, workspace_id, actor_id, action, target_type, target_id, metadata, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[&id, &workspace_id, &actor_id, &action, &target_type, &target_id, &metadata, &ts],
            )
            .await
            .map_err(|e| AppError::Internal(format!("record_audit: {e}")))?;
        Ok(())
    }
}
