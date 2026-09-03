//! choruz-replay: replay CLI for debugging and auditing conversation events.
//!
//! Reads from the `conversation_events` table and outputs events to stdout
//! in a human-readable or JSON format.
//!
//! # Usage
//!
//! ```bash
//! # Replay all events in a conversation
//! choruz-replay --conversation abc123
//!
//! # Replay a range of sequences
//! choruz-replay --conversation abc123 --from-seq 10 --to-seq 50
//!
//! # Replay events for a specific turn
//! choruz-replay --turn-id xyz789
//!
//! # Replay events for a specific command
//! choruz-replay --command-id cmd456
//!
//! # List dead letters
//! choruz-replay --dead-letters --since 24h
//!
//! # Output as JSON (machine-readable)
//! choruz-replay --conversation abc123 --json
//! ```
//!
//! # Environment
//!
//! - `CHORUZ_DATABASE_URL` — libpq connection string
//! - `RUST_LOG` — tracing filter (default: `info`)

use std::process;

use choruz_session::PgSessionStore;
use choruz_store::EventStore;
use chrono::{Duration, Utc};

mod cli;
mod output;
mod query;

/// Configuration for the choruz-replay process.
///
/// # Environment variables
///
/// - `CHORUZ_DATABASE_URL` — libpq connection string (falls back to `PgConfig` component vars)
struct Config {
    database_url: String,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let database_url = std::env::var("CHORUZ_DATABASE_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| choruz_common::PgConfig::from_env().to_connect_string());
        Ok(Self { database_url })
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = choruz_infrastructure::init_tracing("choruz-replay") {
        eprintln!("error: invalid logging configuration: {error}");
        process::exit(2);
    }

    let args = cli::parse_args();

    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    let event_store = EventStore::new(&cfg.database_url);
    let session_store = PgSessionStore::new(&cfg.database_url);

    // Verify connectivity.
    if let Err(e) = event_store.health_check().await {
        eprintln!("error: cannot connect to database: {e}");
        process::exit(1);
    }

    let result = match args.mode {
        cli::ReplayMode::Conversation {
            conversation_id,
            from_seq,
            to_seq,
        } => {
            query::replay_conversation(&event_store, &conversation_id, from_seq, to_seq, args.json)
                .await
        }
        cli::ReplayMode::TurnId { turn_id } => {
            query::replay_by_turn(&event_store, &turn_id, args.json).await
        }
        cli::ReplayMode::CommandId { command_id } => {
            query::replay_by_command(&session_store, &command_id, args.json).await
        }
        cli::ReplayMode::DeadLetters { since_hours } => {
            let since = Utc::now() - Duration::hours(since_hours);
            query::list_dead_letters(&session_store, since, args.json).await
        }
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

#[cfg(test)]
/// Build a libpq connection string from component env vars (used by tests).
fn database_url_from_defaults() -> String {
    choruz_common::PgConfig::from_env().to_connect_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_url_from_defaults_is_valid() {
        let url = database_url_from_defaults();
        assert!(url.contains("host="));
        assert!(url.contains("dbname="));
    }
}
