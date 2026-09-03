//! Output formatting for replay results.

use choruz_store::ConversationEventRow;

/// Print a conversation event in human-readable format.
pub fn print_event_human(event: &ConversationEventRow) {
    let content_preview = event
        .content
        .as_deref()
        .map(|c| {
            if c.len() > 120 {
                format!("{}...", &c[..120])
            } else {
                c.to_string()
            }
        })
        .unwrap_or_else(|| "(no content)".into());

    println!(
        "[seq={:>4}] {} | {} | {}: {}",
        event.seq,
        event.created_at.format("%Y-%m-%d %H:%M:%S"),
        event.event_type,
        event.sender_id,
        content_preview,
    );
}

/// Print a conversation event as JSON.
pub fn print_event_json(event: &ConversationEventRow) {
    match serde_json::to_string(event) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("error: failed to serialize event: {e}"),
    }
}

/// Print a dead letter in human-readable format.
pub fn print_dead_letter_human(dl: &choruz_session::DeadLetter) {
    println!(
        "[{}] {} | source={}:{} | attempts={} | {}",
        dl.created_at.format("%Y-%m-%d %H:%M:%S"),
        if dl.resolved_at.is_some() {
            "RESOLVED"
        } else {
            "UNRESOLVED"
        },
        dl.source_type,
        dl.source_id,
        dl.attempt_count,
        dl.error,
    );
}

/// Print a dead letter as JSON.
pub fn print_dead_letter_json(dl: &choruz_session::DeadLetter) {
    match serde_json::to_string(dl) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("error: failed to serialize dead letter: {e}"),
    }
}

/// Print command details in human-readable format.
pub fn print_command_human(cmd: &choruz_session::AgentCommand) {
    println!("Command: {}", cmd.command_id);
    println!("  Agent:         {}", cmd.agent_id);
    println!("  Session:       {}", cmd.session_key);
    println!("  Conversation:  {}", cmd.conversation_id);
    println!("  Turn ID:       {}", cmd.turn_id);
    println!("  Status:        {}", cmd.status.as_str());
    println!(
        "  Attempts:      {}/{}",
        cmd.attempt_count, cmd.max_attempts
    );
    if let Some(ref err) = cmd.last_error {
        println!("  Last Error:    {err}");
    }
    println!(
        "  Created:       {}",
        cmd.created_at.format("%Y-%m-%d %H:%M:%S")
    );
    println!(
        "  Updated:       {}",
        cmd.updated_at.format("%Y-%m-%d %H:%M:%S")
    );
    println!(
        "  Prompt:        {}",
        if cmd.prompt.len() > 80 {
            format!("{}...", &cmd.prompt[..80])
        } else {
            cmd.prompt.clone()
        }
    );
}

/// Print command details as JSON.
pub fn print_command_json(cmd: &choruz_session::AgentCommand) {
    match serde_json::to_string(cmd) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("error: failed to serialize command: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn print_event_human_does_not_panic() {
        let event = ConversationEventRow {
            conversation_id: "conv-1".into(),
            seq: 1,
            event_id: "evt-1".into(),
            event_type: "message".into(),
            sender_id: "user-1".into(),
            content: Some("hello".into()),
            content_type: "text/plain".into(),
            metadata: serde_json::json!({}),
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            created_at: Utc::now(),
        };
        // Should not panic.
        print_event_human(&event);
    }

    #[test]
    fn print_event_json_does_not_panic() {
        let event = ConversationEventRow {
            conversation_id: "conv-1".into(),
            seq: 1,
            event_id: "evt-1".into(),
            event_type: "message".into(),
            sender_id: "user-1".into(),
            content: None,
            content_type: "text/plain".into(),
            metadata: serde_json::json!({}),
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            created_at: Utc::now(),
        };
        print_event_json(&event);
    }
}
