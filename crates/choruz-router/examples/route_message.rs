//! Route one message without a database or an agent process.

use choruz_router::{
    ConversationMember, InMemoryDecisionSink, InMemoryMemberProvider, route_event,
};
use choruz_store::ConversationEventRow;
use chrono::Utc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let members = InMemoryMemberProvider {
        members: ["researcher", "reviewer"]
            .into_iter()
            .map(|name| ConversationMember {
                conversation_id: "example".into(),
                principal_id: name.into(),
                principal_type: "agent".into(),
                display_name: Some(name.into()),
                joined_at: Utc::now(),
                left_at: None,
            })
            .collect(),
        ..Default::default()
    };
    let event = ConversationEventRow {
        conversation_id: "example".into(),
        seq: 1,
        event_id: "review-request".into(),
        event_type: "message".into(),
        sender_id: "user".into(),
        content: Some("@reviewer check the researcher's proof before publishing".into()),
        content_type: "text/plain".into(),
        metadata: serde_json::json!({}),
        client_msg_id: None,
        turn_id: None,
        reply_event_id: None,
        created_at: Utc::now(),
    };
    let sink = InMemoryDecisionSink::default();
    route_event(&event, &members, &sink).await?;
    let commands = sink.commands.lock().await;
    for command in commands.iter() {
        println!("{}: {}", command.agent_id, command.prompt);
    }
    Ok(())
}
