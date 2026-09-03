//! Local SQLite WAL (Write-Ahead Log) for crash recovery.
//!
//! Every prompt injection and response is logged to a local SQLite database
//! before being acknowledged. On startup, the executor checks for incomplete
//! turns and reports them as failures to the session manager, triggering retries.
//!
//! Schema:
//! ```sql
//! CREATE TABLE adapter_wal (
//!     turn_id TEXT NOT NULL,
//!     attempt_id TEXT NOT NULL,
//!     event_type TEXT NOT NULL,  -- 'start', 'chunk', 'finished', 'failed'
//!     payload TEXT,
//!     created_at TEXT NOT NULL DEFAULT (datetime('now')),
//!     PRIMARY KEY (turn_id, attempt_id, event_type)
//! );
//! ```

use std::path::Path;
use std::sync::Arc;

use rusqlite::params;
use tokio::sync::Mutex;

use crate::error::{ExecutorError, ExecutorResult};

// ---------------------------------------------------------------------------
// WAL event types
// ---------------------------------------------------------------------------

/// Event types stored in the WAL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalEventType {
    /// Turn execution started.
    Start,
    /// A chunk of content was received (optional, for streaming).
    Chunk,
    /// Turn finished successfully.
    Finished,
    /// Turn failed with an error.
    Failed,
}

impl WalEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Chunk => "chunk",
            Self::Finished => "finished",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Self::Start),
            "chunk" => Some(Self::Chunk),
            "finished" => Some(Self::Finished),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// WAL entry
// ---------------------------------------------------------------------------

/// A single entry in the WAL.
#[derive(Debug, Clone)]
pub struct WalEntry {
    pub turn_id: String,
    pub attempt_id: String,
    pub event_type: WalEventType,
    pub payload: Option<String>,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Incomplete turn (for crash recovery)
// ---------------------------------------------------------------------------

/// An incomplete turn found during crash recovery.
#[derive(Debug, Clone)]
pub struct IncompleteTurn {
    pub turn_id: String,
    pub attempt_id: String,
    pub prompt: Option<String>,
}

// ---------------------------------------------------------------------------
// AdapterWal
// ---------------------------------------------------------------------------

/// SQLite-backed write-ahead log for adapter crash recovery.
///
/// Thread-safety: the inner `Connection` is behind a `Mutex` because
/// `rusqlite::Connection` is `!Send`. All operations acquire the lock.
pub struct AdapterWal {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl AdapterWal {
    /// Open or create a WAL database at the given path.
    pub fn open(path: &Path) -> ExecutorResult<Self> {
        let conn = rusqlite::Connection::open(path)?;

        // Enable WAL mode for better concurrent performance
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        // Create the table if it doesn't exist
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS adapter_wal (
                turn_id TEXT NOT NULL,
                attempt_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (turn_id, attempt_id, event_type)
            );",
        )?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory WAL (useful for tests).
    pub fn open_in_memory() -> ExecutorResult<Self> {
        let conn = rusqlite::Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE adapter_wal (
                turn_id TEXT NOT NULL,
                attempt_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (turn_id, attempt_id, event_type)
            );",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Log a turn start event.
    pub async fn log_turn_start(
        &self,
        turn_id: &str,
        attempt_id: &str,
        prompt: &str,
    ) -> ExecutorResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO adapter_wal (turn_id, attempt_id, event_type, payload)
             VALUES (?1, ?2, 'start', ?3)",
            params![turn_id, attempt_id, prompt],
        )
        .map_err(|e| ExecutorError::Wal(format!("failed to log turn start: {e}")))?;
        Ok(())
    }

    /// Log a content chunk event.
    pub async fn log_chunk(
        &self,
        turn_id: &str,
        attempt_id: &str,
        chunk: &str,
    ) -> ExecutorResult<()> {
        let conn = self.conn.lock().await;
        // For chunks, we append to the payload using REPLACE to accumulate
        conn.execute(
            "INSERT OR REPLACE INTO adapter_wal (turn_id, attempt_id, event_type, payload)
             VALUES (?1, ?2, 'chunk', ?3)",
            params![turn_id, attempt_id, chunk],
        )
        .map_err(|e| ExecutorError::Wal(format!("failed to log chunk: {e}")))?;
        Ok(())
    }

    /// Log a turn finished event.
    pub async fn log_turn_finished(
        &self,
        turn_id: &str,
        attempt_id: &str,
        content: &str,
    ) -> ExecutorResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO adapter_wal (turn_id, attempt_id, event_type, payload)
             VALUES (?1, ?2, 'finished', ?3)",
            params![turn_id, attempt_id, content],
        )
        .map_err(|e| ExecutorError::Wal(format!("failed to log turn finished: {e}")))?;
        Ok(())
    }

    /// Log a turn failed event.
    pub async fn log_turn_failed(
        &self,
        turn_id: &str,
        attempt_id: &str,
        error: &str,
    ) -> ExecutorResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO adapter_wal (turn_id, attempt_id, event_type, payload)
             VALUES (?1, ?2, 'failed', ?3)",
            params![turn_id, attempt_id, error],
        )
        .map_err(|e| ExecutorError::Wal(format!("failed to log turn failed: {e}")))?;
        Ok(())
    }

    /// Find all incomplete turns (have 'start' but no 'finished' or 'failed').
    ///
    /// This is used during crash recovery to identify turns that were in
    /// progress when the executor crashed.
    pub async fn find_incomplete_turns(&self) -> ExecutorResult<Vec<IncompleteTurn>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT w.turn_id, w.attempt_id, w.payload
                 FROM adapter_wal w
                 WHERE w.event_type = 'start'
                   AND NOT EXISTS (
                       SELECT 1 FROM adapter_wal w2
                       WHERE w2.turn_id = w.turn_id
                         AND w2.attempt_id = w.attempt_id
                         AND w2.event_type IN ('finished', 'failed')
                   )",
            )
            .map_err(|e| ExecutorError::Wal(format!("failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(IncompleteTurn {
                    turn_id: row.get(0)?,
                    attempt_id: row.get(1)?,
                    prompt: row.get(2)?,
                })
            })
            .map_err(|e| ExecutorError::Wal(format!("failed to query incomplete turns: {e}")))?;

        let mut result = Vec::new();
        for row in rows {
            result
                .push(row.map_err(|e| ExecutorError::Wal(format!("failed to read WAL row: {e}")))?);
        }
        Ok(result)
    }

    /// Clean up completed turns older than the given age (in seconds).
    pub async fn cleanup(&self, max_age_secs: i64) -> ExecutorResult<usize> {
        let conn = self.conn.lock().await;
        let affected = conn
            .execute(
                "DELETE FROM adapter_wal
                 WHERE turn_id IN (
                     SELECT DISTINCT turn_id FROM adapter_wal
                     WHERE event_type IN ('finished', 'failed')
                       AND created_at < datetime('now', ?1)
                 )",
                params![format!("-{max_age_secs} seconds")],
            )
            .map_err(|e| ExecutorError::Wal(format!("failed to cleanup WAL: {e}")))?;
        Ok(affected)
    }

    /// Get all WAL entries for a specific turn.
    pub async fn get_turn_entries(
        &self,
        turn_id: &str,
        attempt_id: &str,
    ) -> ExecutorResult<Vec<WalEntry>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT turn_id, attempt_id, event_type, payload, created_at
                 FROM adapter_wal
                 WHERE turn_id = ?1 AND attempt_id = ?2
                 ORDER BY created_at ASC, rowid ASC",
            )
            .map_err(|e| ExecutorError::Wal(format!("failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(params![turn_id, attempt_id], |row| {
                let event_type_str: String = row.get(2)?;
                Ok(WalEntry {
                    turn_id: row.get(0)?,
                    attempt_id: row.get(1)?,
                    event_type: WalEventType::parse(&event_type_str).unwrap_or(WalEventType::Start),
                    payload: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| ExecutorError::Wal(format!("failed to query WAL entries: {e}")))?;

        let mut result = Vec::new();
        for row in rows {
            result
                .push(row.map_err(|e| ExecutorError::Wal(format!("failed to read WAL row: {e}")))?);
        }
        Ok(result)
    }

    /// Count total entries in the WAL.
    pub async fn count(&self) -> ExecutorResult<usize> {
        let conn = self.conn.lock().await;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM adapter_wal", [], |row| row.get(0))
            .map_err(|e| ExecutorError::Wal(format!("failed to count WAL: {e}")))?;
        Ok(count as usize)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wal_lifecycle_happy_path() {
        let wal = AdapterWal::open_in_memory().unwrap();

        // Log start
        wal.log_turn_start("turn-1", "attempt-1", "What is 2+2?")
            .await
            .unwrap();

        // Should be incomplete
        let incomplete = wal.find_incomplete_turns().await.unwrap();
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].turn_id, "turn-1");
        assert_eq!(incomplete[0].prompt.as_deref(), Some("What is 2+2?"));

        // Log finished
        wal.log_turn_finished("turn-1", "attempt-1", "4")
            .await
            .unwrap();

        // Should no longer be incomplete
        let incomplete = wal.find_incomplete_turns().await.unwrap();
        assert!(incomplete.is_empty());
    }

    #[tokio::test]
    async fn wal_lifecycle_failed_path() {
        let wal = AdapterWal::open_in_memory().unwrap();

        wal.log_turn_start("turn-1", "attempt-1", "bad prompt")
            .await
            .unwrap();

        // Should be incomplete
        let incomplete = wal.find_incomplete_turns().await.unwrap();
        assert_eq!(incomplete.len(), 1);

        // Log failure
        wal.log_turn_failed("turn-1", "attempt-1", "process crashed")
            .await
            .unwrap();

        // Should no longer be incomplete
        let incomplete = wal.find_incomplete_turns().await.unwrap();
        assert!(incomplete.is_empty());
    }

    #[tokio::test]
    async fn wal_multiple_turns() {
        let wal = AdapterWal::open_in_memory().unwrap();

        // Start two turns
        wal.log_turn_start("turn-1", "attempt-1", "prompt 1")
            .await
            .unwrap();
        wal.log_turn_start("turn-2", "attempt-1", "prompt 2")
            .await
            .unwrap();

        // Both incomplete
        let incomplete = wal.find_incomplete_turns().await.unwrap();
        assert_eq!(incomplete.len(), 2);

        // Finish one
        wal.log_turn_finished("turn-1", "attempt-1", "result 1")
            .await
            .unwrap();

        // Only one incomplete
        let incomplete = wal.find_incomplete_turns().await.unwrap();
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].turn_id, "turn-2");
    }

    #[tokio::test]
    async fn wal_get_turn_entries() {
        let wal = AdapterWal::open_in_memory().unwrap();

        wal.log_turn_start("turn-1", "attempt-1", "hello")
            .await
            .unwrap();
        wal.log_chunk("turn-1", "attempt-1", "partial output")
            .await
            .unwrap();
        wal.log_turn_finished("turn-1", "attempt-1", "full output")
            .await
            .unwrap();

        let entries = wal.get_turn_entries("turn-1", "attempt-1").await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].event_type, WalEventType::Start);
        assert_eq!(entries[1].event_type, WalEventType::Chunk);
        assert_eq!(entries[2].event_type, WalEventType::Finished);
    }

    #[tokio::test]
    async fn wal_count() {
        let wal = AdapterWal::open_in_memory().unwrap();

        assert_eq!(wal.count().await.unwrap(), 0);

        wal.log_turn_start("turn-1", "attempt-1", "hello")
            .await
            .unwrap();
        assert_eq!(wal.count().await.unwrap(), 1);

        wal.log_turn_finished("turn-1", "attempt-1", "done")
            .await
            .unwrap();
        assert_eq!(wal.count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn wal_file_backed() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");

        {
            let wal = AdapterWal::open(&db_path).unwrap();
            wal.log_turn_start("turn-1", "attempt-1", "hello")
                .await
                .unwrap();
        }

        // Re-open and verify data persists
        {
            let wal = AdapterWal::open(&db_path).unwrap();
            let incomplete = wal.find_incomplete_turns().await.unwrap();
            assert_eq!(incomplete.len(), 1);
            assert_eq!(incomplete[0].turn_id, "turn-1");
        }
    }

    #[test]
    fn wal_event_type_roundtrip() {
        for evt in [
            WalEventType::Start,
            WalEventType::Chunk,
            WalEventType::Finished,
            WalEventType::Failed,
        ] {
            let s = evt.as_str();
            let parsed = WalEventType::parse(s).unwrap();
            assert_eq!(parsed, evt);
        }
    }

    #[test]
    fn wal_event_type_unknown() {
        assert!(WalEventType::parse("unknown").is_none());
    }

    #[tokio::test]
    async fn wal_idempotent_insert() {
        let wal = AdapterWal::open_in_memory().unwrap();

        // Insert same start twice (INSERT OR REPLACE)
        wal.log_turn_start("turn-1", "attempt-1", "first")
            .await
            .unwrap();
        wal.log_turn_start("turn-1", "attempt-1", "second")
            .await
            .unwrap();

        // Should only have one entry (the second one replaced the first)
        let entries = wal.get_turn_entries("turn-1", "attempt-1").await.unwrap();
        let starts: Vec<_> = entries
            .iter()
            .filter(|e| e.event_type == WalEventType::Start)
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].payload.as_deref(), Some("second"));
    }

    #[tokio::test]
    async fn wal_different_attempts_tracked_separately() {
        let wal = AdapterWal::open_in_memory().unwrap();

        // Same turn, different attempts
        wal.log_turn_start("turn-1", "attempt-1", "first try")
            .await
            .unwrap();
        wal.log_turn_failed("turn-1", "attempt-1", "crashed")
            .await
            .unwrap();
        wal.log_turn_start("turn-1", "attempt-2", "second try")
            .await
            .unwrap();

        // Only attempt-2 is incomplete
        let incomplete = wal.find_incomplete_turns().await.unwrap();
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].attempt_id, "attempt-2");
    }
}
