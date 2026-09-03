use choruz_common::{AppError, new_id};
use choruz_domain::EventEnvelope;

use super::DbService;
use super::helpers::row_to_event_envelope;
use crate::WebhookDelivery;

impl DbService {
    // ── Events (Phase 4) ──────────────────────────────────────────────────

    /// Push an event to one or more principals (DB-backed).
    ///
    /// Inserts one `outbox_event` row per recipient.  The `delivery_seq`
    /// column is a BIGSERIAL auto-assigned by PostgreSQL.
    pub async fn push_event(
        &self,
        principal_ids: &[String],
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), AppError> {
        if principal_ids.is_empty() {
            return Ok(());
        }
        let client = self.store.connect().await?;
        for pid in principal_ids {
            let id = new_id();
            client
                .execute(
                    "INSERT INTO outbox_event (id, workspace_id, principal_id, event_type, payload, created_at)
                     VALUES ($1, '', $2, $3, $4, NOW())",
                    &[&id, pid, &event_type, &payload],
                )
                .await
                .map_err(|e| AppError::Internal(format!("push_event insert: {e}")))?;
        }
        Ok(())
    }

    /// List unacknowledged events for a principal, ordered by delivery_seq.
    ///
    /// If `cursor` is provided, only events with `delivery_seq > cursor`
    /// are returned.  Otherwise all unacknowledged events are returned.
    pub async fn list_events(
        &self,
        principal_id: &str,
        cursor: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<EventEnvelope>, AppError> {
        let client = self.store.connect().await?;
        let cursor_val = cursor.unwrap_or(0) as i64;
        let limit_val = limit.unwrap_or(100) as i64;
        let rows = client
            .query(
                "SELECT id, principal_id, event_type, payload, created_at, delivery_seq
                 FROM outbox_event
                 WHERE principal_id = $1
                   AND acknowledged_at IS NULL
                   AND delivery_seq > $2
                 ORDER BY delivery_seq ASC
                 LIMIT $3",
                &[&principal_id, &cursor_val, &limit_val],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_events: {e}")))?;

        Ok(rows.iter().map(row_to_event_envelope).collect())
    }

    /// Acknowledge events up to (and including) a delivery_seq.
    ///
    /// Sets `acknowledged_at = NOW()` on all matching rows for the
    /// given principal with `delivery_seq <= upto_seq`.
    pub async fn ack_events(&self, principal_id: &str, upto_seq: u64) -> Result<u64, AppError> {
        let client = self.store.connect().await?;
        let seq_val = upto_seq as i64;
        client
            .execute(
                "UPDATE outbox_event
                 SET acknowledged_at = NOW()
                 WHERE principal_id = $1
                   AND delivery_seq <= $2
                   AND acknowledged_at IS NULL",
                &[&principal_id, &seq_val],
            )
            .await
            .map_err(|e| AppError::Internal(format!("ack_events: {e}")))?;

        Ok(upto_seq)
    }

    /// Collect pending webhook deliveries from the DB.
    ///
    /// Reads `event_webhook` configs and finds unacknowledged outbox_event
    /// rows whose `delivery_seq > webhook.cursor` for each configured
    /// principal.
    pub async fn collect_pending_webhook_deliveries(
        &self,
    ) -> Result<Vec<WebhookDelivery>, AppError> {
        let client = self.store.connect().await?;

        // Load all webhook configs
        let config_rows = client
            .query(
                "SELECT principal_id, url, event_types, cursor, webhook_secret
                 FROM event_webhook
                 ORDER BY principal_id",
                &[],
            )
            .await
            .map_err(|e| AppError::Internal(format!("collect_webhooks config: {e}")))?;

        let mut deliveries = Vec::new();
        for cfg_row in &config_rows {
            let principal_id: String = cfg_row.get("principal_id");
            let url: String = cfg_row.get("url");
            let event_types: Vec<String> = cfg_row.get("event_types");
            let cursor: i64 = cfg_row.get("cursor");
            let webhook_secret: String = cfg_row.get("webhook_secret");

            let event_rows = client
                .query(
                    "SELECT id, principal_id, event_type, payload, created_at, delivery_seq
                     FROM outbox_event
                     WHERE principal_id = $1
                       AND acknowledged_at IS NULL
                       AND delivery_seq > $2
                     ORDER BY delivery_seq ASC",
                    &[&principal_id, &cursor],
                )
                .await
                .map_err(|e| AppError::Internal(format!("collect_webhooks events: {e}")))?;

            for row in &event_rows {
                let event = row_to_event_envelope(row);
                if !event_types.is_empty()
                    && !event_types.iter().any(|kind| kind == &event.event_type)
                {
                    continue;
                }
                deliveries.push(WebhookDelivery {
                    principal_id: principal_id.clone(),
                    url: url.clone(),
                    event,
                    secret: webhook_secret.clone(),
                });
            }
        }
        Ok(deliveries)
    }

    /// Mark webhook events as delivered up to a delivery_seq.
    ///
    /// Updates the webhook cursor in the `event_webhook` table.
    pub async fn mark_webhook_delivered(
        &self,
        principal_id: &str,
        upto_delivery_seq: u64,
    ) -> Result<u64, AppError> {
        let client = self.store.connect().await?;
        let seq_val = upto_delivery_seq as i64;
        let result = client
            .execute(
                "UPDATE event_webhook
                 SET cursor = GREATEST(cursor, $2), updated_at = NOW()
                 WHERE principal_id = $1",
                &[&principal_id, &seq_val],
            )
            .await
            .map_err(|e| AppError::Internal(format!("mark_webhook_delivered: {e}")))?;

        if result == 0 {
            return Err(AppError::NotFound("webhook not configured".into()));
        }
        Ok(upto_delivery_seq)
    }
}
