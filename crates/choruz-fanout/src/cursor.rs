//! Client cursor CRUD — tracks per-client, per-conversation read positions.
//!
//! The cursor store enables:
//! - H3: client_cursors CRUD
//! - H4: cursor replay on reconnect (read cursor → fetch events after cursor → push)

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::RwLock;

use crate::models::ClientCursor;

// ---------------------------------------------------------------------------
// CursorStore trait
// ---------------------------------------------------------------------------

/// Abstracts cursor persistence so tests can use in-memory storage.
pub trait CursorStore: Send + Sync + 'static {
    /// Get the cursor for a client + conversation.
    /// Returns `None` if the client has never connected to this conversation.
    fn get_cursor(
        &self,
        client_id: &str,
        conversation_id: &str,
    ) -> impl std::future::Future<Output = Option<ClientCursor>> + Send;

    /// Upsert the cursor — create or update the last_seen_seq.
    fn upsert_cursor(
        &self,
        client_id: &str,
        conversation_id: &str,
        last_seen_seq: i64,
    ) -> impl std::future::Future<Output = ()> + Send;

    /// Delete a cursor (e.g. when a client unsubscribes).
    fn delete_cursor(
        &self,
        client_id: &str,
        conversation_id: &str,
    ) -> impl std::future::Future<Output = ()> + Send;

    /// List all cursors for a given client (across conversations).
    fn list_cursors_for_client(
        &self,
        client_id: &str,
    ) -> impl std::future::Future<Output = Vec<ClientCursor>> + Send;
}

// ---------------------------------------------------------------------------
// InMemoryCursorStore
// ---------------------------------------------------------------------------

/// In-memory cursor store using a `HashMap` behind a `RwLock`.
///
/// Suitable for single-node deployments and testing. Production would use
/// PostgreSQL or Redis.
#[derive(Clone, Default)]
pub struct InMemoryCursorStore {
    /// Key: (client_id, conversation_id)
    cursors: Arc<RwLock<HashMap<(String, String), ClientCursor>>>,
}

impl InMemoryCursorStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the total number of stored cursors (for testing).
    pub async fn len(&self) -> usize {
        self.cursors.read().await.len()
    }

    /// Check if the store is empty (for testing).
    pub async fn is_empty(&self) -> bool {
        self.cursors.read().await.is_empty()
    }
}

impl CursorStore for InMemoryCursorStore {
    async fn get_cursor(&self, client_id: &str, conversation_id: &str) -> Option<ClientCursor> {
        let cursors = self.cursors.read().await;
        cursors
            .get(&(client_id.to_string(), conversation_id.to_string()))
            .cloned()
    }

    async fn upsert_cursor(&self, client_id: &str, conversation_id: &str, last_seen_seq: i64) {
        let mut cursors = self.cursors.write().await;
        let key = (client_id.to_string(), conversation_id.to_string());
        let cursor = cursors
            .entry(key)
            .or_insert_with(|| ClientCursor::new(client_id, conversation_id));
        cursor.last_seen_seq = last_seen_seq;
        cursor.updated_at = Utc::now();
    }

    async fn delete_cursor(&self, client_id: &str, conversation_id: &str) {
        let mut cursors = self.cursors.write().await;
        cursors.remove(&(client_id.to_string(), conversation_id.to_string()));
    }

    async fn list_cursors_for_client(&self, client_id: &str) -> Vec<ClientCursor> {
        let cursors = self.cursors.read().await;
        cursors
            .values()
            .filter(|c| c.client_id == client_id)
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_nonexistent_cursor_returns_none() {
        let store = InMemoryCursorStore::new();
        assert!(store.get_cursor("c1", "conv-1").await.is_none());
    }

    #[tokio::test]
    async fn upsert_and_get_cursor() {
        let store = InMemoryCursorStore::new();

        store.upsert_cursor("c1", "conv-1", 42).await;

        let cursor = store.get_cursor("c1", "conv-1").await.unwrap();
        assert_eq!(cursor.client_id, "c1");
        assert_eq!(cursor.conversation_id, "conv-1");
        assert_eq!(cursor.last_seen_seq, 42);
    }

    #[tokio::test]
    async fn upsert_updates_existing_cursor() {
        let store = InMemoryCursorStore::new();

        store.upsert_cursor("c1", "conv-1", 10).await;
        store.upsert_cursor("c1", "conv-1", 50).await;

        let cursor = store.get_cursor("c1", "conv-1").await.unwrap();
        assert_eq!(cursor.last_seen_seq, 50);
        assert_eq!(store.len().await, 1);
    }

    #[tokio::test]
    async fn delete_cursor() {
        let store = InMemoryCursorStore::new();

        store.upsert_cursor("c1", "conv-1", 10).await;
        assert_eq!(store.len().await, 1);

        store.delete_cursor("c1", "conv-1").await;
        assert!(store.is_empty().await);
        assert!(store.get_cursor("c1", "conv-1").await.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_is_noop() {
        let store = InMemoryCursorStore::new();
        store.delete_cursor("c1", "conv-1").await;
        assert!(store.is_empty().await);
    }

    #[tokio::test]
    async fn list_cursors_for_client() {
        let store = InMemoryCursorStore::new();

        store.upsert_cursor("c1", "conv-1", 10).await;
        store.upsert_cursor("c1", "conv-2", 20).await;
        store.upsert_cursor("c2", "conv-1", 30).await;

        let c1_cursors = store.list_cursors_for_client("c1").await;
        assert_eq!(c1_cursors.len(), 2);

        let c2_cursors = store.list_cursors_for_client("c2").await;
        assert_eq!(c2_cursors.len(), 1);
        assert_eq!(c2_cursors[0].last_seen_seq, 30);
    }

    #[tokio::test]
    async fn list_cursors_empty_for_unknown_client() {
        let store = InMemoryCursorStore::new();
        store.upsert_cursor("c1", "conv-1", 10).await;

        let cursors = store.list_cursors_for_client("unknown").await;
        assert!(cursors.is_empty());
    }

    #[tokio::test]
    async fn multiple_clients_same_conversation() {
        let store = InMemoryCursorStore::new();

        store.upsert_cursor("c1", "conv-1", 10).await;
        store.upsert_cursor("c2", "conv-1", 20).await;

        let c1 = store.get_cursor("c1", "conv-1").await.unwrap();
        let c2 = store.get_cursor("c2", "conv-1").await.unwrap();

        assert_eq!(c1.last_seen_seq, 10);
        assert_eq!(c2.last_seen_seq, 20);
        assert_eq!(store.len().await, 2);
    }
}
