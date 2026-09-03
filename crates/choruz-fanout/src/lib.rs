//! Fanout Gateway for the Choruz message pipeline.
//!
//! This crate provides:
//!
//! - **Connection management** (H1) — subscribe/unsubscribe clients to
//!   conversations, using tokio channels to model WS/SSE connections
//! - **Event polling + push** (H2) — poll conversation_events and push new
//!   events to all subscribed clients
//! - **Client cursors** (H3) — CRUD for per-client, per-conversation read
//!   positions
//! - **Cursor replay** (H4) — on reconnect, replay all events after the
//!   client's last-seen cursor
//!
//! # Architecture
//!
//! The gateway maintains an in-memory registry of connected clients
//! (`FanoutGateway::connections`).  The fanout loop periodically polls
//! the event source for new events and pushes them to each client's
//! channel.  On disconnect, the client's cursor is persisted; on reconnect
//! it is read and used to replay missed events.

pub mod cursor;
pub mod gateway;
pub mod models;
pub mod ws;

// Re-export commonly used types at crate root.
pub use cursor::{CursorStore, InMemoryCursorStore};
pub use gateway::{
    ClientConnection, EventSource, FanoutError, FanoutGateway, FanoutResult, InMemoryEventSource,
};
pub use models::{ClientCursor, FanoutEvent, Subscription};
pub use ws::{WsFanoutState, ws_fanout_routes};
