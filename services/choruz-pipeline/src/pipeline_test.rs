//! In-memory integration tests for the pipeline chain.
//!
//! These tests verify the routing -> writing chain using the in-memory
//! test doubles provided by each crate, without requiring a database.

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use choruz_router::{
        AgentPolicy, AutoMode, ConversationMember, InMemoryDecisionSink, InMemoryMemberProvider,
        route_event,
    };
    use choruz_store::ConversationEventRow;
    use choruz_writer::{
        AgentResult, AgentResultStatus, InMemoryResultStore, WriteOutcome, commit_result,
    };

    /// Helper: create a conversation event from a user mentioning an agent.
    fn make_user_event(content: &str) -> ConversationEventRow {
        ConversationEventRow {
            conversation_id: "conv-test".into(),
            seq: 1,
            event_id: "evt-1".into(),
            event_type: "message".into(),
            sender_id: "user-1".into(),
            content: Some(content.into()),
            content_type: "text/plain".into(),
            metadata: serde_json::json!({}),
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            created_at: Utc::now(),
        }
    }

    fn make_agent_member(principal_id: &str, display_name: &str) -> ConversationMember {
        ConversationMember {
            conversation_id: "conv-test".into(),
            principal_id: principal_id.into(),
            principal_type: "agent".into(),
            display_name: Some(display_name.into()),
            joined_at: Utc::now(),
            left_at: None,
        }
    }

    /// End-to-end: route an event that mentions an agent, then commit the
    /// resulting agent result as a reply event.
    #[tokio::test]
    async fn route_then_write_end_to_end() {
        // 1. Setup router with one agent
        let members = InMemoryMemberProvider {
            members: vec![make_agent_member("agent-be", "backend-dev")],
            policies: vec![],
            ..Default::default()
        };
        let sink = InMemoryDecisionSink::default();

        // 2. Route: user mentions the agent
        let event = make_user_event("@backend-dev please review this PR");
        let outcomes = route_event(&event, &members, &sink).await.unwrap();
        assert_eq!(outcomes.len(), 1);

        // 3. Verify a command was generated
        let cmds = sink.commands.lock().await;
        assert_eq!(cmds.len(), 1);
        let cmd = &cmds[0];
        assert_eq!(cmd.agent_id, "agent-be");
        assert_eq!(cmd.conversation_id, "conv-test");
        assert!(!cmd.turn_id.is_empty());

        // 4. Simulate executor producing a result
        let result = AgentResult {
            turn_id: cmd.turn_id.clone(),
            attempt_id: "attempt-1".into(),
            command_id: cmd.command_id.clone(),
            session_key: cmd.session_key.clone(),
            conversation_id: cmd.conversation_id.clone(),
            agent_id: cmd.agent_id.clone(),
            status: AgentResultStatus::Succeeded,
            content: Some("LGTM, approved!".into()),
            content_type: Some("text/plain".into()),
            error: None,
            tool_calls_count: 0,
            execution_duration_ms: 1500,
            secondary_command_attempts: Vec::new(),
            command_results: Vec::new(),
            trace_id: None,
        };

        // 5. Write: commit the result
        let store = InMemoryResultStore::default();
        let outcome = commit_result(&result, &store).await.unwrap();
        assert!(matches!(outcome, WriteOutcome::Committed { seq: 1, .. }));

        // 6. Verify the reply event was stored
        let events = store.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "reply");
        assert_eq!(events[0].sender_id, "agent-be");
        assert_eq!(events[0].content, Some("LGTM, approved!".into()));
        assert_eq!(events[0].turn_id, Some(cmd.turn_id.clone()));
    }

    /// Verify that a second attempt with the same turn_id is deduplicated.
    #[tokio::test]
    async fn duplicate_turn_is_deduped() {
        let store = InMemoryResultStore::default();

        let result = AgentResult {
            turn_id: "turn-dedup-1".into(),
            attempt_id: "attempt-1".into(),
            command_id: "cmd-1".into(),
            session_key: "agent-1:conv-1".into(),
            conversation_id: "conv-1".into(),
            agent_id: "agent-1".into(),
            status: AgentResultStatus::Succeeded,
            content: Some("first reply".into()),
            content_type: Some("text/plain".into()),
            error: None,
            tool_calls_count: 0,
            execution_duration_ms: 100,
            secondary_command_attempts: Vec::new(),
            command_results: Vec::new(),
            trace_id: None,
        };

        let o1 = commit_result(&result, &store).await.unwrap();
        assert!(matches!(o1, WriteOutcome::Committed { .. }));

        // Same turn_id, different attempt
        let result2 = AgentResult {
            attempt_id: "attempt-2".into(),
            ..result.clone()
        };
        let o2 = commit_result(&result2, &store).await.unwrap();
        assert_eq!(o2, WriteOutcome::DuplicateTurn);

        let events = store.events.lock().await;
        assert_eq!(events.len(), 1);
    }

    /// A repeated group delivery must remain idempotent at the result writer.
    /// This uses the in-memory store so the assertion is independent of any
    /// shared database or running pipeline process.
    #[tokio::test]
    async fn duplicate_group_turn_is_deduped() {
        let store = InMemoryResultStore::default();
        let result = AgentResult {
            turn_id: "turn-group-dedup-1".into(),
            attempt_id: "attempt-group-1".into(),
            command_id: "cmd-group-1".into(),
            session_key: "agent-group-1:group-conversation-1".into(),
            conversation_id: "group-conversation-1".into(),
            agent_id: "agent-group-1".into(),
            status: AgentResultStatus::Succeeded,
            content: Some("single group reply".into()),
            content_type: Some("text/plain".into()),
            error: None,
            tool_calls_count: 0,
            execution_duration_ms: 100,
            secondary_command_attempts: Vec::new(),
            command_results: Vec::new(),
            trace_id: None,
        };

        assert!(matches!(
            commit_result(&result, &store).await.unwrap(),
            WriteOutcome::Committed { .. }
        ));
        assert_eq!(
            commit_result(
                &AgentResult {
                    attempt_id: "attempt-group-2".into(),
                    ..result.clone()
                },
                &store,
            )
            .await
            .unwrap(),
            WriteOutcome::DuplicateTurn
        );

        let events = store.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].conversation_id, "group-conversation-1");
        assert_eq!(events[0].turn_id.as_deref(), Some("turn-group-dedup-1"));
    }

    /// Route an @all message to multiple agents and verify all get commands.
    #[tokio::test]
    async fn route_at_all_generates_multiple_commands() {
        let members = InMemoryMemberProvider {
            members: vec![
                make_agent_member("agent-be", "backend-dev"),
                make_agent_member("agent-fe", "frontend-dev"),
                make_agent_member("agent-rev", "reviewer"),
            ],
            policies: vec![],
            ..Default::default()
        };
        let sink = InMemoryDecisionSink::default();

        let event = make_user_event("@all status check please");
        let outcomes = route_event(&event, &members, &sink).await.unwrap();
        assert_eq!(outcomes.len(), 3);

        let cmds = sink.commands.lock().await;
        assert_eq!(cmds.len(), 3);

        let agent_ids: Vec<&str> = cmds.iter().map(|c| c.agent_id.as_str()).collect();
        assert!(agent_ids.contains(&"agent-be"));
        assert!(agent_ids.contains(&"agent-fe"));
        assert!(agent_ids.contains(&"agent-rev"));
    }

    /// B-001 regression: mentioning an agent whose name extends another
    /// agent's name must not wake both agents.
    #[tokio::test]
    async fn selective_mention_does_not_trigger_prefix_named_agent() {
        let members = InMemoryMemberProvider {
            members: vec![
                make_agent_member("agent-dev", "dev"),
                make_agent_member("agent-dev2", "dev2"),
            ],
            policies: vec![],
            ..Default::default()
        };
        let sink = InMemoryDecisionSink::default();

        let event = make_user_event("@dev2 hi, only you should respond");
        let outcomes = route_event(&event, &members, &sink).await.unwrap();
        assert_eq!(outcomes.len(), 2);

        let cmds = sink.commands.lock().await;
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].agent_id, "agent-dev2");
    }

    /// B-001 regression: the shorter agent must not wake when a longer
    /// space-separated agent name is mentioned.
    #[tokio::test]
    async fn selective_mention_does_not_trigger_space_separated_prefix_named_agent() {
        let members = InMemoryMemberProvider {
            members: vec![
                make_agent_member("agent-claude", "Claude"),
                make_agent_member("agent-claude-code", "Claude Code 1"),
            ],
            policies: vec![],
            ..Default::default()
        };
        let sink = InMemoryDecisionSink::default();

        let event = make_user_event("@Claude Code 1 hi, only you should respond");
        let outcomes = route_event(&event, &members, &sink).await.unwrap();
        assert_eq!(outcomes.len(), 2);

        let cmds = sink.commands.lock().await;
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].agent_id, "agent-claude-code");
    }

    /// MENTION-002: mentioning an agent that is not in the group must not
    /// route or wake that agent, even when its alias appears in policy data.
    #[tokio::test]
    async fn mentioning_agent_outside_group_does_not_generate_command() {
        let members = InMemoryMemberProvider {
            members: vec![make_agent_member("agent-member", "Group Agent")],
            policies: vec![AgentPolicy {
                agent_id: "agent-outsider".into(),
                conversation_id: "conv-test".into(),
                auto_mode: AutoMode::MentionedOnly,
                mention_aliases: vec!["Outside Agent".into()],
            }],
            ..Default::default()
        };
        let sink = InMemoryDecisionSink::default();

        let event = make_user_event("@Outside Agent please take this");
        let outcomes = route_event(&event, &members, &sink).await.unwrap();
        assert_eq!(outcomes.len(), 1);

        let cmds = sink.commands.lock().await;
        assert!(cmds.is_empty());

        let decisions = sink.decisions.lock().await;
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].agent_id, "agent-member");
        assert_eq!(decisions[0].decision, "skip");
        assert_eq!(decisions[0].reason, "not mentioned (mentioned_only policy)");
    }

    /// AllMessages policy triggers even without mention.
    #[tokio::test]
    async fn all_messages_policy_triggers_without_mention() {
        let members = InMemoryMemberProvider {
            members: vec![make_agent_member("agent-be", "backend-dev")],
            policies: vec![AgentPolicy {
                agent_id: "agent-be".into(),
                conversation_id: "conv-test".into(),
                auto_mode: AutoMode::AllMessages,
                mention_aliases: vec![],
            }],
            ..Default::default()
        };
        let sink = InMemoryDecisionSink::default();

        let event = make_user_event("just chatting, no mentions");
        let outcomes = route_event(&event, &members, &sink).await.unwrap();
        assert_eq!(outcomes.len(), 1);

        let cmds = sink.commands.lock().await;
        assert_eq!(cmds.len(), 1);
    }

    /// Failed results are not committed.
    #[tokio::test]
    async fn failed_result_not_committed() {
        let store = InMemoryResultStore::default();

        let result = AgentResult {
            turn_id: "turn-fail".into(),
            attempt_id: "attempt-1".into(),
            command_id: "cmd-1".into(),
            session_key: "agent-1:conv-1".into(),
            conversation_id: "conv-1".into(),
            agent_id: "agent-1".into(),
            status: AgentResultStatus::Failed,
            content: None,
            content_type: None,
            error: Some("timeout".into()),
            tool_calls_count: 0,
            execution_duration_ms: 60000,
            secondary_command_attempts: Vec::new(),
            command_results: Vec::new(),
            trace_id: None,
        };

        let outcome = commit_result(&result, &store).await.unwrap();
        assert_eq!(outcome, WriteOutcome::SkippedNotSucceeded);

        let events = store.events.lock().await;
        assert!(events.is_empty());
    }
}
