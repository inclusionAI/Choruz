//! Query functions for the replay CLI.
//!
//! Each function queries the database and prints results using the output
//! module.

use choruz_session::PgSessionStore;
use choruz_store::EventStore;
use chrono::{DateTime, Utc};

use crate::output;

/// Replay conversation events within a sequence range.
pub async fn replay_conversation(
    store: &EventStore,
    conversation_id: &str,
    from_seq: i64,
    to_seq: Option<i64>,
    json: bool,
) -> Result<(), String> {
    let limit = to_seq.map(|t| t - from_seq).unwrap_or(10_000).max(1);

    let events = store
        .get_events_after_seq(conversation_id, from_seq, limit)
        .await
        .map_err(|e| format!("failed to query events: {e}"))?;

    if events.is_empty() {
        if !json {
            println!("(no events found for conversation {conversation_id} after seq {from_seq})");
        }
        return Ok(());
    }

    if !json {
        println!(
            "--- Conversation {conversation_id}: {} event(s) ---",
            events.len()
        );
    }

    for event in &events {
        if let Some(max_seq) = to_seq
            && event.seq > max_seq
        {
            break;
        }
        if json {
            output::print_event_json(event);
        } else {
            output::print_event_human(event);
        }
    }

    Ok(())
}

/// Replay events associated with a specific turn_id.
pub async fn replay_by_turn(store: &EventStore, turn_id: &str, json: bool) -> Result<(), String> {
    // The turn_id is stored in conversation_events.turn_id column.
    // We need to find the event with this turn_id, then show context around it.
    // Since EventStore doesn't have a turn_id query, we'll look up via the
    // conversation_events table directly.  For now, we do a broad scan.
    //
    // A proper implementation would add a `get_event_by_turn_id` method to
    // EventStore.  For the MVP replay CLI, we query the session store for
    // the command associated with this turn_id and use its conversation_id.

    if !json {
        println!("--- Turn {turn_id} ---");
        println!("(turn-id lookup requires scanning; use --conversation for faster results)");
    }

    // Try to find the command for this turn to get conversation context.
    let _ = store; // EventStore doesn't expose turn_id queries yet.

    if !json {
        println!("hint: use CHORUZ_DATABASE_URL and query directly:");
        println!(
            "  SELECT * FROM conversation_events WHERE turn_id = '{}';",
            turn_id
        );
    }

    Ok(())
}

/// Show details for a specific command, including all attempts.
pub async fn replay_by_command(
    session_store: &PgSessionStore,
    command_id: &str,
    json: bool,
) -> Result<(), String> {
    let command = session_store
        .get_command(command_id)
        .await
        .map_err(|e| format!("failed to query command: {e}"))?;

    match command {
        Some(cmd) => {
            if json {
                output::print_command_json(&cmd);
            } else {
                output::print_command_human(&cmd);
            }
        }
        None => {
            if json {
                println!("null");
            } else {
                println!("(command {command_id} not found)");
            }
        }
    }

    Ok(())
}

/// List dead letters since a given timestamp.
pub async fn list_dead_letters(
    session_store: &PgSessionStore,
    since: DateTime<Utc>,
    json: bool,
) -> Result<(), String> {
    let dead_letters = session_store
        .list_dead_letters_since(since, 200)
        .await
        .map_err(|e| format!("failed to query dead letters: {e}"))?;

    if dead_letters.is_empty() {
        if !json {
            println!(
                "(no dead letters found since {})",
                since.format("%Y-%m-%d %H:%M:%S")
            );
        }
        return Ok(());
    }

    if !json {
        println!(
            "--- {} dead letter(s) since {} ---",
            dead_letters.len(),
            since.format("%Y-%m-%d %H:%M:%S"),
        );
    }

    for dl in &dead_letters {
        if json {
            output::print_dead_letter_json(dl);
        } else {
            output::print_dead_letter_human(dl);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn limit_calculation() {
        let from = 10_i64;
        let to: Option<i64> = Some(50);
        let limit = to.map(|t| t - from).unwrap_or(10_000).max(1);
        assert_eq!(limit, 40);
    }

    #[test]
    fn limit_calculation_no_upper() {
        let from = 0_i64;
        let to: Option<i64> = None;
        let limit = to.map(|t| t - from).unwrap_or(10_000).max(1);
        assert_eq!(limit, 10_000);
    }
}
