use choruz_common::{AppError, new_id, now};
use choruz_domain::AuditLog;
use serde_json::Value;

use super::DbService;
use super::helpers::row_to_audit_log;

impl DbService {
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
