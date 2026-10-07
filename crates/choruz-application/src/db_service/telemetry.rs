use choruz_activity::TelemetryEvent;
use choruz_common::AppError;
use choruz_domain::Principal;

use super::DbService;

impl DbService {
    /// Commits a validated, sanitized batch atomically. Event identity is scoped
    /// to the authenticated principal; acknowledgement can safely be retried.
    pub async fn record_telemetry(
        &self,
        principal: &Principal,
        events: &[TelemetryEvent],
    ) -> Result<(), AppError> {
        choruz_activity::validate_batch(events)?;
        let mut client = self.store.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| AppError::Internal(format!("begin activity batch: {e}")))?;
        for event in events {
            tx.execute(
                "INSERT INTO telemetry_event (workspace_id, principal_id, event_id, schema_version, trace_id, span_id, session_id, name, occurred_at, duration_ms, data)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
                 ON CONFLICT (workspace_id, principal_id, event_id) DO NOTHING",
                &[&principal.workspace_id, &principal.id, &event.event_id, &event.schema_version, &event.trace_id, &event.span_id, &event.session_id, &event.name, &event.ts, &event.duration_ms, &event.data],
            ).await.map_err(|e| AppError::Internal(format!("persist activity batch: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| AppError::Internal(format!("commit activity batch: {e}")))?;
        for event in events {
            tracing::debug!(event_id = %event.event_id, trace_id = %event.trace_id, span_id = %event.span_id, name = %event.name, principal_id = %principal.id, "activity persisted or replayed");
        }
        tracing::info!(principal_id = %principal.id, count = events.len(), "activity batch committed");
        Ok(())
    }
}
