use choruz_common::AppError;

use super::DbService;
use crate::{SyncChange, SyncChangePage};

impl DbService {
    /// Register a browser/device and return its durable acknowledged cursor.
    /// A locally persisted cursor may advance a newly restored device, but it
    /// can never point beyond this principal's feed.
    pub async fn register_sync_device(
        &self,
        principal_id: &str,
        device_id: &str,
        local_cursor: u64,
    ) -> Result<u64, AppError> {
        if device_id.is_empty() || device_id.len() > 128 || device_id.chars().any(char::is_control)
        {
            return Err(AppError::Validation("invalid sync device id".into()));
        }
        let local_cursor = i64::try_from(local_cursor)
            .map_err(|_| AppError::Validation("sync cursor is too large".into()))?;
        let client = self.store.connect().await?;
        let head: i64 = client
            .query_one(
                "SELECT COALESCE(MAX(cursor), 0) FROM sync_change WHERE principal_id = $1",
                &[&principal_id],
            )
            .await
            .map_err(|error| AppError::Internal(format!("sync device head: {error}")))?
            .get(0);
        if local_cursor > head {
            return Err(AppError::Validation(
                "sync cursor is ahead of this principal's feed".into(),
            ));
        }
        let row = client
            .query_one(
                "INSERT INTO sync_device (principal_id, device_id, ack_cursor)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (principal_id, device_id) DO UPDATE
                 SET ack_cursor = GREATEST(sync_device.ack_cursor, EXCLUDED.ack_cursor)
                 RETURNING ack_cursor",
                &[&principal_id, &device_id, &local_cursor],
            )
            .await
            .map_err(|error| AppError::Internal(format!("register sync device: {error}")))?;
        let cursor: i64 = row.get(0);
        Ok(cursor as u64)
    }

    /// Persist a monotonic device ACK. The WebSocket handler additionally
    /// ensures the cursor was actually sent on this connection.
    pub async fn acknowledge_sync_device(
        &self,
        principal_id: &str,
        device_id: &str,
        cursor: u64,
    ) -> Result<u64, AppError> {
        let cursor = i64::try_from(cursor)
            .map_err(|_| AppError::Validation("sync cursor is too large".into()))?;
        let client = self.store.connect().await?;
        let row = client
            .query_opt(
                "UPDATE sync_device
                 SET ack_cursor = GREATEST(ack_cursor, $3)
                 WHERE principal_id = $1 AND device_id = $2
                   AND $3 <= (SELECT COALESCE(MAX(cursor), 0)
                              FROM sync_change WHERE principal_id = $1)
                 RETURNING ack_cursor",
                &[&principal_id, &device_id, &cursor],
            )
            .await
            .map_err(|error| AppError::Internal(format!("acknowledge sync device: {error}")))?
            .ok_or_else(|| AppError::Validation("invalid sync acknowledgement".into()))?;
        let cursor: i64 = row.get(0);
        Ok(cursor as u64)
    }

    /// Current high-water mark for one principal's durable dashboard feed.
    /// Capture this before loading a bootstrap snapshot; replaying after it
    /// may duplicate snapshot state but cannot miss a concurrent mutation.
    pub async fn current_sync_cursor(&self, principal_id: &str) -> Result<u64, AppError> {
        let client = self.store.connect().await?;
        let row = client
            .query_one(
                "SELECT COALESCE(MAX(cursor), 0) AS cursor
                 FROM sync_change
                 WHERE principal_id = $1",
                &[&principal_id],
            )
            .await
            .map_err(|error| AppError::Internal(format!("current sync cursor: {error}")))?;
        let cursor: i64 = row.get("cursor");
        Ok(cursor as u64)
    }

    /// Read the oldest unseen changes first. The extra row determines
    /// `has_more`, preventing a burst larger than one page from skipping its
    /// middle when the client persists `next_cursor`.
    pub async fn list_sync_changes(
        &self,
        principal_id: &str,
        cursor: u64,
        limit: u32,
    ) -> Result<SyncChangePage, AppError> {
        let cursor = i64::try_from(cursor)
            .map_err(|_| AppError::Validation("sync cursor is too large".into()))?;
        let limit = limit.clamp(1, 500);
        let fetch_limit = i64::from(limit) + 1;
        let client = self.store.connect().await?;
        let head_row = client
            .query_one(
                "SELECT COALESCE(MAX(cursor), 0) AS cursor
                 FROM sync_change
                 WHERE principal_id = $1",
                &[&principal_id],
            )
            .await
            .map_err(|error| AppError::Internal(format!("sync head cursor: {error}")))?;
        let head_cursor: i64 = head_row.get("cursor");
        if cursor > head_cursor {
            return Err(AppError::Validation(
                "sync cursor is ahead of this principal's feed".into(),
            ));
        }
        let mut rows = client
            .query(
                "SELECT cursor, event_type, entity_type, entity_id,
                        conversation_id, payload, created_at
                 FROM sync_change
                 WHERE principal_id = $1 AND cursor > $2 AND cursor <= $4
                 ORDER BY cursor ASC
                 LIMIT $3",
                &[&principal_id, &cursor, &fetch_limit, &head_cursor],
            )
            .await
            .map_err(|error| AppError::Internal(format!("list sync changes: {error}")))?;
        let has_more = rows.len() > limit as usize;
        if has_more {
            rows.truncate(limit as usize);
        }
        let changes: Vec<SyncChange> = rows
            .into_iter()
            .map(|row| SyncChange {
                cursor: row.get::<_, i64>("cursor") as u64,
                event_type: row.get("event_type"),
                entity_type: row.get("entity_type"),
                entity_id: row.get("entity_id"),
                conversation_id: row.get("conversation_id"),
                payload: row.get("payload"),
                created_at: row.get("created_at"),
            })
            .collect();
        let next_cursor = changes.last().map_or(cursor as u64, |change| change.cursor);

        Ok(SyncChangePage {
            changes,
            next_cursor,
            head_cursor: head_cursor as u64,
            has_more,
        })
    }
}
