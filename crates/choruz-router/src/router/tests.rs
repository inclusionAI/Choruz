use super::*;
use crate::models::ChannelTaskStatus;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn make_event(sender_id: &str, content: &str, metadata: serde_json::Value) -> ConversationEventRow {
    make_event_for("conv-1", 1, "evt-1", sender_id, content, metadata)
}

fn make_event_for(
    conversation_id: &str,
    seq: i64,
    event_id: &str,
    sender_id: &str,
    content: &str,
    metadata: serde_json::Value,
) -> ConversationEventRow {
    ConversationEventRow {
        conversation_id: conversation_id.into(),
        seq,
        event_id: event_id.into(),
        event_type: "message".into(),
        sender_id: sender_id.into(),
        content: Some(content.into()),
        content_type: "text/plain".into(),
        metadata,
        client_msg_id: None,
        turn_id: None,
        reply_event_id: None,
        created_at: Utc::now(),
    }
}

fn make_member_provider(
    members: Vec<ConversationMember>,
    policies: Vec<AgentPolicy>,
) -> InMemoryMemberProvider {
    InMemoryMemberProvider {
        members,
        policies,
        ..Default::default()
    }
}

fn make_agent_member(principal_id: &str, display_name: &str) -> ConversationMember {
    make_agent_member_for("conv-1", principal_id, display_name)
}

fn make_human_member(principal_id: &str, display_name: &str) -> ConversationMember {
    ConversationMember {
        conversation_id: "conv-1".into(),
        principal_id: principal_id.into(),
        principal_type: "human".into(),
        display_name: Some(display_name.into()),
        joined_at: Utc::now(),
        left_at: None,
    }
}

fn make_agent_member_for(
    conversation_id: &str,
    principal_id: &str,
    display_name: &str,
) -> ConversationMember {
    ConversationMember {
        conversation_id: conversation_id.into(),
        principal_id: principal_id.into(),
        principal_type: "agent".into(),
        display_name: Some(display_name.into()),
        joined_at: Utc::now(),
        left_at: None,
    }
}

fn workflow_task(task_id: &str, task_key: &str) -> GroupWorkflowTask {
    workflow_task_with_assignee(task_id, task_key, "agent-owner")
}

fn workflow_task_with_assignee(
    task_id: &str,
    task_key: &str,
    assignee_principal_id: &str,
) -> GroupWorkflowTask {
    GroupWorkflowTask {
        id: task_id.into(),
        conversation_id: "conv-1".into(),
        task_key: task_key.into(),
        status: ChannelTaskStatus::Todo,
        assignee_principal_id: assignee_principal_id.into(),
        assignee_principal_type: Some("agent".into()),
    }
}

fn workflow_participant(
    task_id: &str,
    principal_id: &str,
    role_key: &str,
) -> GroupWorkflowTaskParticipant {
    GroupWorkflowTaskParticipant {
        task_id: task_id.into(),
        principal_id: principal_id.into(),
        role_key: role_key.into(),
        principal_type: Some("agent".into()),
    }
}

fn coordinator_policy(coordinator_id: &str) -> ConversationRoutingPolicy {
    ConversationRoutingPolicy {
        conversation_id: "conv-1".into(),
        default_coordinator_agent_id: Some(coordinator_id.into()),
        untagged_human_mode: UntaggedHumanMode::CoordinatorOnly,
    }
}

async fn command_agent_ids(sink: &InMemoryDecisionSink) -> Vec<String> {
    let mut ids = sink
        .commands
        .lock()
        .await
        .iter()
        .map(|command| command.agent_id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

struct NamedMemberProvider {
    inner: InMemoryMemberProvider,
    principal_names: HashMap<String, String>,
    conversation_names: HashMap<String, String>,
}

impl MemberProvider for NamedMemberProvider {
    async fn list_agent_members(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<ConversationMember>> {
        self.inner.list_agent_members(conversation_id).await
    }

    async fn list_assignee_roster(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<AssigneeRosterEntry>> {
        self.inner.list_assignee_roster(conversation_id).await
    }

    async fn get_agent_policy(
        &self,
        agent_id: &str,
        conversation_id: &str,
    ) -> RouterResult<AgentPolicy> {
        self.inner.get_agent_policy(agent_id, conversation_id).await
    }

    async fn resolve_principal_name(&self, id: &str) -> Option<String> {
        self.principal_names.get(id).cloned()
    }

    async fn resolve_conversation_name(&self, id: &str) -> Option<String> {
        self.conversation_names.get(id).cloned()
    }
}

struct CountingRosterProvider {
    inner: InMemoryMemberProvider,
    roster_calls: Arc<AtomicUsize>,
}

impl MemberProvider for CountingRosterProvider {
    async fn list_agent_members(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<ConversationMember>> {
        self.inner.list_agent_members(conversation_id).await
    }

    async fn list_assignee_roster(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<AssigneeRosterEntry>> {
        self.roster_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.list_assignee_roster(conversation_id).await
    }

    async fn get_agent_policy(
        &self,
        agent_id: &str,
        conversation_id: &str,
    ) -> RouterResult<AgentPolicy> {
        self.inner.get_agent_policy(agent_id, conversation_id).await
    }

    async fn resolve_principal_name(&self, id: &str) -> Option<String> {
        self.inner.resolve_principal_name(id).await
    }

    async fn resolve_conversation_name(&self, id: &str) -> Option<String> {
        self.inner.resolve_conversation_name(id).await
    }
}

struct FailingRosterProvider {
    inner: InMemoryMemberProvider,
}

impl MemberProvider for FailingRosterProvider {
    async fn list_agent_members(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<ConversationMember>> {
        self.inner.list_agent_members(conversation_id).await
    }

    async fn list_assignee_roster(
        &self,
        _conversation_id: &str,
    ) -> RouterResult<Vec<AssigneeRosterEntry>> {
        Err(RouterError::Internal("roster unavailable".into()))
    }

    async fn get_agent_policy(
        &self,
        agent_id: &str,
        conversation_id: &str,
    ) -> RouterResult<AgentPolicy> {
        self.inner.get_agent_policy(agent_id, conversation_id).await
    }

    async fn resolve_principal_name(&self, id: &str) -> Option<String> {
        self.inner.resolve_principal_name(id).await
    }

    async fn resolve_conversation_name(&self, id: &str) -> Option<String> {
        self.inner.resolve_conversation_name(id).await
    }
}

#[tokio::test]
async fn route_event_triggers_mentioned_agent() {
    let members = make_member_provider(vec![make_agent_member("agent-be", "backend-dev")], vec![]);
    let sink = InMemoryDecisionSink::default();

    let event = make_event("user-1", "@backend-dev review this", serde_json::json!({}));

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0], RouteOutcome::Triggered);

    // Verify outputs
    let vis = sink.visibilities.lock().await;
    assert_eq!(vis.len(), 1);
    assert_eq!(vis[0].agent_id, "agent-be");

    let decs = sink.decisions.lock().await;
    assert_eq!(decs.len(), 1);
    assert_eq!(decs[0].decision, "trigger");

    let cmds = sink.commands.lock().await;
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].agent_id, "agent-be");
}

#[tokio::test]
async fn route_event_skips_not_mentioned_agent() {
    let members = make_member_provider(vec![make_agent_member("agent-be", "backend-dev")], vec![]);
    let sink = InMemoryDecisionSink::default();

    let event = make_event(
        "user-1",
        "@frontend-dev please review",
        serde_json::json!({}),
    );

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0], RouteOutcome::Skipped);

    // Visibility should still be written
    let vis = sink.visibilities.lock().await;
    assert_eq!(vis.len(), 1);

    // Decision should be skip
    let decs = sink.decisions.lock().await;
    assert_eq!(decs.len(), 1);
    assert_eq!(decs[0].decision, "skip");

    // No command should be generated
    let cmds = sink.commands.lock().await;
    assert!(cmds.is_empty());
}

#[tokio::test]
async fn untagged_human_mentioned_only_skip_has_audit_source() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-be", "backend-dev"),
            make_agent_member("agent-fe", "frontend-dev"),
        ],
        principal_types: HashMap::from([("human-1".into(), "human".into())]),
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();

    let event = make_event(
        "human-1",
        "Can someone look at this?",
        serde_json::json!({}),
    );

    route_event(&event, &members, &sink).await.unwrap();

    assert!(command_agent_ids(&sink).await.is_empty());
    let decisions = sink.decisions.lock().await;
    assert_eq!(decisions.len(), 2);
    assert!(decisions.iter().all(|decision| {
        decision.decision == "skip"
            && decision.reason == "not mentioned (mentioned_only policy)"
            && decision.policy_snapshot["routing_source"]
                == serde_json::json!("untagged_human_mentioned_only")
            && decision.policy_snapshot["untagged_human_mode"]
                == serde_json::json!("mentioned_only")
    }));
}

#[tokio::test]
async fn workflow_text_without_metadata_skip_has_audit_marker() {
    let members = make_member_provider(vec![make_agent_member("agent-be", "backend-dev")], vec![]);
    let sink = InMemoryDecisionSink::default();

    let event = make_event(
        "agent-reporter",
        "[DONE] DOC-P0-03 implementation complete",
        serde_json::json!({}),
    );

    route_event(&event, &members, &sink).await.unwrap();

    assert!(command_agent_ids(&sink).await.is_empty());
    let decisions = sink.decisions.lock().await;
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].decision, "skip");
    assert_eq!(
        decisions[0].policy_snapshot["routing_source"],
        serde_json::json!("policy")
    );
    assert_eq!(
        decisions[0].policy_snapshot["workflow_text_marker"],
        serde_json::json!("done_marker")
    );
}

#[tokio::test]
async fn passive_group_launch_kickoff_does_not_create_agent_commands() {
    let members = make_member_provider(
        vec![
            make_agent_member("agent-operator", "Project Operator"),
            make_agent_member("agent-backend", "Backend Engineer"),
            make_agent_member("agent-reviewer", "Code Reviewer"),
        ],
        vec![],
    );
    let sink = InMemoryDecisionSink::default();

    let event = make_event(
        "user-1",
        "Mission: Ship onboarding MVP\n\nRoles: Project Operator, Backend Engineer, Code Reviewer.\n\nNext user action: send the first concrete work item or question when ready.",
        serde_json::json!({
            "source": "group_provisioning",
            "job_id": "job-1",
            "passive": true
        }),
    );

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert_eq!(outcomes, vec![RouteOutcome::Skipped; 3]);
    assert!(sink.commands.lock().await.is_empty());
}

#[tokio::test]
async fn route_event_skips_self_sent() {
    let members = make_member_provider(vec![make_agent_member("agent-be", "backend-dev")], vec![]);
    let sink = InMemoryDecisionSink::default();

    // Event is sent by the agent itself
    let event = make_event(
        "agent-be",
        "I'm done with the review",
        serde_json::json!({}),
    );

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0], RouteOutcome::Skipped);

    // No visibility, no decision, no command for self-sent
    let vis = sink.visibilities.lock().await;
    assert!(vis.is_empty());
    let decs = sink.decisions.lock().await;
    assert!(decs.is_empty());
    let cmds = sink.commands.lock().await;
    assert!(cmds.is_empty());
}

#[tokio::test]
async fn route_event_multiple_agents() {
    let members = make_member_provider(
        vec![
            make_agent_member("agent-be", "backend-dev"),
            make_agent_member("agent-fe", "frontend-dev"),
            make_agent_member("agent-rev", "reviewer"),
        ],
        vec![],
    );
    let sink = InMemoryDecisionSink::default();

    let event = make_event("user-1", "@all status update?", serde_json::json!({}));

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert_eq!(outcomes.len(), 3);
    assert!(outcomes.iter().all(|o| *o == RouteOutcome::Triggered));

    let vis = sink.visibilities.lock().await;
    assert_eq!(vis.len(), 3);
    let decs = sink.decisions.lock().await;
    assert_eq!(decs.len(), 3);
    let cmds = sink.commands.lock().await;
    assert_eq!(cmds.len(), 3);
}

#[tokio::test]
async fn route_event_selective_mention() {
    let members = make_member_provider(
        vec![
            make_agent_member("agent-be", "backend-dev"),
            make_agent_member("agent-fe", "frontend-dev"),
        ],
        vec![],
    );
    let sink = InMemoryDecisionSink::default();

    let event = make_event("user-1", "@backend-dev fix the API", serde_json::json!({}));

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert_eq!(outcomes.len(), 2);

    // backend-dev triggered, frontend-dev skipped
    let cmds = sink.commands.lock().await;
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].agent_id, "agent-be");
}

#[tokio::test]
async fn explicit_mention_wins_over_workflow_metadata() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-owner", "owner"),
            make_agent_member("agent-coordinator", "coordinator"),
            make_agent_member("agent-quality", "quality-checker"),
        ],
        principal_types: HashMap::from([("human-1".into(), "human".into())]),
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        workflow_tasks: vec![workflow_task("task-1", "DOC-P0-03")],
        workflow_participants: vec![
            workflow_participant("task-1", "agent-owner", "owner"),
            workflow_participant("task-1", "agent-coordinator", "coordinator"),
        ],
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event(
        "human-1",
        "@quality-checker please review this feedback",
        serde_json::json!({
            "workflow": {
                "kind": "task.feedback",
                "task_key": "DOC-P0-03",
            }
        }),
    );

    route_event(&event, &members, &sink).await.unwrap();

    assert_eq!(command_agent_ids(&sink).await, vec!["agent-quality"]);
    let decisions = sink.decisions.lock().await;
    let quality = decisions
        .iter()
        .find(|decision| decision.agent_id == "agent-quality")
        .expect("quality decision");
    assert_eq!(quality.decision, "trigger");
    assert_eq!(
        quality.policy_snapshot["routing_source"],
        serde_json::json!("explicit_target")
    );
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.agent_id == "agent-owner")
            .expect("owner decision")
            .reason,
        "explicit_target_not_selected"
    );
}

#[tokio::test]
async fn explicit_metadata_target_wins_over_workflow_metadata() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-owner", "owner"),
            make_agent_member("agent-coordinator", "coordinator"),
            make_agent_member("agent-reviewer", "reviewer"),
        ],
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        workflow_tasks: vec![workflow_task("task-1", "DOC-P0-03")],
        workflow_participants: vec![
            workflow_participant("task-1", "agent-owner", "owner"),
            workflow_participant("task-1", "agent-coordinator", "coordinator"),
        ],
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event(
        "agent-reporter",
        "workflow feedback",
        serde_json::json!({
            "turn_for": "agent-reviewer",
            "workflow": {
                "kind": "task.feedback",
                "task_key": "DOC-P0-03",
            }
        }),
    );

    route_event(&event, &members, &sink).await.unwrap();

    assert_eq!(command_agent_ids(&sink).await, vec!["agent-reviewer"]);
    let decisions = sink.decisions.lock().await;
    let reviewer = decisions
        .iter()
        .find(|decision| decision.agent_id == "agent-reviewer")
        .expect("reviewer decision");
    assert_eq!(reviewer.reason, "metadata turn_for matches agent");
    assert_eq!(
        reviewer.policy_snapshot["routing_source"],
        serde_json::json!("explicit_target")
    );
}

#[tokio::test]
async fn missing_explicit_metadata_target_does_not_fall_through_to_workflow() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-owner", "owner"),
            make_agent_member("agent-coordinator", "coordinator"),
        ],
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        workflow_tasks: vec![workflow_task("task-1", "DOC-P0-03")],
        workflow_participants: vec![
            workflow_participant("task-1", "agent-owner", "owner"),
            workflow_participant("task-1", "agent-coordinator", "coordinator"),
        ],
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event(
        "agent-reporter",
        "workflow feedback with stale target",
        serde_json::json!({
            "turn_for": "agent-missing",
            "workflow": {
                "kind": "task.feedback",
                "task_key": "DOC-P0-03",
            }
        }),
    );

    route_event(&event, &members, &sink).await.unwrap();

    assert!(command_agent_ids(&sink).await.is_empty());
    let decisions = sink.decisions.lock().await;
    assert!(decisions.iter().all(|decision| {
        decision.decision == "skip" && decision.reason == "explicit_target_not_found"
    }));
}

#[tokio::test]
async fn untagged_human_message_routes_only_to_configured_coordinator() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-coordinator", "coordinator"),
            make_agent_member("agent-worker", "worker"),
        ],
        principal_types: HashMap::from([("human-1".into(), "human".into())]),
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event(
        "human-1",
        "Can someone pick this up?",
        serde_json::json!({}),
    );

    route_event(&event, &members, &sink).await.unwrap();

    assert_eq!(command_agent_ids(&sink).await, vec!["agent-coordinator"]);
    let decisions = sink.decisions.lock().await;
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.agent_id == "agent-coordinator")
            .expect("coordinator decision")
            .reason,
        "untagged_human_to_coordinator"
    );
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.agent_id == "agent-worker")
            .expect("worker decision")
            .reason,
        "untagged_human_to_coordinator_not_targeted"
    );
}

#[tokio::test]
async fn untagged_agent_message_does_not_use_human_coordinator_policy() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-coordinator", "coordinator"),
            make_agent_member("agent-worker", "worker"),
        ],
        principal_types: HashMap::from([("agent-worker".into(), "agent".into())]),
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event(
        "agent-worker",
        "Work update with no mention",
        serde_json::json!({}),
    );

    route_event(&event, &members, &sink).await.unwrap();

    assert!(command_agent_ids(&sink).await.is_empty());
    let decisions = sink.decisions.lock().await;
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].agent_id, "agent-coordinator");
    assert_eq!(decisions[0].reason, "not mentioned (mentioned_only policy)");
}

#[tokio::test]
async fn unavailable_coordinator_policy_falls_back_to_mention_only_with_audit_reason() {
    let members = InMemoryMemberProvider {
        members: vec![make_agent_member("agent-worker", "worker")],
        principal_types: HashMap::from([("human-1".into(), "human".into())]),
        conversation_policies: vec![coordinator_policy("agent-missing")],
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event("human-1", "No explicit target here", serde_json::json!({}));

    route_event(&event, &members, &sink).await.unwrap();

    assert!(command_agent_ids(&sink).await.is_empty());
    let decisions = sink.decisions.lock().await;
    assert_eq!(
        decisions[0].reason,
        "coordinator_unavailable: not mentioned (mentioned_only policy)"
    );
    assert_eq!(
        decisions[0].policy_snapshot["coordinator_fallback_reason"],
        serde_json::json!("coordinator_unavailable")
    );
}

#[tokio::test]
async fn workflow_event_kinds_route_to_expected_agent_roles() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-owner", "owner"),
            make_agent_member("agent-coordinator", "coordinator"),
            make_agent_member("agent-quality", "quality"),
            make_agent_member("agent-approver", "approver"),
        ],
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        workflow_tasks: vec![workflow_task("task-1", "DOC-P0-03")],
        workflow_participants: vec![
            workflow_participant("task-1", "agent-owner", "owner"),
            workflow_participant("task-1", "agent-coordinator", "coordinator"),
            workflow_participant("task-1", "agent-quality", "quality_check"),
            workflow_participant("task-1", "agent-approver", "approver"),
        ],
        ..Default::default()
    };

    let cases = [
        ("task.started", None, vec!["agent-coordinator"]),
        (
            "task.ready_for_next_step",
            Some("quality_check"),
            vec!["agent-quality"],
        ),
        (
            "task.feedback",
            None,
            vec!["agent-coordinator", "agent-owner"],
        ),
        (
            "task.cleared",
            None,
            vec!["agent-coordinator", "agent-owner"],
        ),
        ("task.blocked", None, vec!["agent-coordinator"]),
        ("task.idle", None, vec!["agent-coordinator"]),
        (
            "external_check.failed",
            None,
            vec!["agent-coordinator", "agent-owner"],
        ),
        ("external_check.passed", None, vec!["agent-coordinator"]),
        ("human_input_needed", None, vec!["agent-coordinator"]),
        (
            "approval_required",
            None,
            vec!["agent-approver", "agent-coordinator"],
        ),
    ];

    for (idx, (kind, next_role, expected)) in cases.into_iter().enumerate() {
        let sink = InMemoryDecisionSink::default();
        let mut workflow = serde_json::json!({
            "kind": kind,
            "task_key": "DOC-P0-03",
        });
        if let Some(next_role) = next_role {
            workflow["next_role"] = serde_json::Value::String(next_role.into());
        }
        let event = make_event_for(
            "conv-1",
            idx as i64 + 1,
            &format!("evt-workflow-{idx}"),
            "agent-reporter",
            "workflow update",
            serde_json::json!({ "workflow": workflow }),
        );

        route_event(&event, &members, &sink).await.unwrap();

        let mut expected = expected.into_iter().map(str::to_string).collect::<Vec<_>>();
        expected.sort();
        assert_eq!(command_agent_ids(&sink).await, expected, "kind {kind}");
    }
}

#[tokio::test]
async fn owner_workflow_event_adds_configured_coordinator_when_participant_missing() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-owner", "owner"),
            make_agent_member("agent-coordinator", "coordinator"),
        ],
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        workflow_tasks: vec![workflow_task("task-1", "DOC-P0-03")],
        workflow_participants: vec![workflow_participant("task-1", "agent-owner", "owner")],
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event(
        "agent-reporter",
        "feedback",
        serde_json::json!({
            "workflow": {
                "kind": "task.feedback",
                "task_key": "DOC-P0-03",
            }
        }),
    );

    route_event(&event, &members, &sink).await.unwrap();

    assert_eq!(
        command_agent_ids(&sink).await,
        vec!["agent-coordinator", "agent-owner"]
    );
}

#[tokio::test]
async fn owner_workflow_event_uses_canonical_assignee_over_stale_owner_participant() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-canonical", "canonical-owner"),
            make_agent_member("agent-stale", "stale-owner"),
            make_agent_member("agent-coordinator", "coordinator"),
        ],
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        workflow_tasks: vec![workflow_task_with_assignee(
            "task-1",
            "DOC-P0-03",
            "agent-canonical",
        )],
        workflow_participants: vec![
            workflow_participant("task-1", "agent-stale", "owner"),
            workflow_participant("task-1", "agent-coordinator", "coordinator"),
        ],
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event(
        "agent-reporter",
        "feedback",
        serde_json::json!({
            "workflow": {
                "kind": "task.feedback",
                "task_key": "DOC-P0-03",
            }
        }),
    );

    route_event(&event, &members, &sink).await.unwrap();

    assert_eq!(
        command_agent_ids(&sink).await,
        vec!["agent-canonical", "agent-coordinator"]
    );
    let decisions = sink.decisions.lock().await;
    let canonical = decisions
        .iter()
        .find(|decision| decision.agent_id == "agent-canonical")
        .expect("canonical owner decision");
    assert_eq!(canonical.decision, "trigger");
    assert_eq!(
        canonical.policy_snapshot["assignee_principal_id"],
        serde_json::json!("agent-canonical")
    );
    assert_eq!(
        canonical.policy_snapshot["task_status"],
        serde_json::json!("todo")
    );
    let stale = decisions
        .iter()
        .find(|decision| decision.agent_id == "agent-stale")
        .expect("stale owner decision");
    assert_eq!(stale.decision, "skip");
    assert_eq!(stale.reason, "workflow_event_to_owner_not_targeted");
}

#[tokio::test]
async fn workflow_missing_task_falls_back_to_coordinator_or_skips() {
    let with_coordinator = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-coordinator", "coordinator"),
            make_agent_member("agent-worker", "worker"),
        ],
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        ..Default::default()
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event(
        "agent-reporter",
        "missing task feedback",
        serde_json::json!({
            "workflow": {
                "kind": "task.feedback",
                "task_key": "MISSING",
            }
        }),
    );

    route_event(&event, &with_coordinator, &sink).await.unwrap();

    assert_eq!(command_agent_ids(&sink).await, vec!["agent-coordinator"]);
    let decisions = sink.decisions.lock().await;
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.agent_id == "agent-coordinator")
            .expect("coordinator decision")
            .reason,
        "workflow_task_missing_coordinator_fallback"
    );
    assert_eq!(
        decisions
            .iter()
            .find(|decision| decision.agent_id == "agent-coordinator")
            .expect("coordinator decision")
            .policy_snapshot["routing_source"],
        serde_json::json!("workflow_event")
    );

    let without_coordinator = InMemoryMemberProvider {
        members: vec![make_agent_member("agent-worker", "worker")],
        ..Default::default()
    };
    let no_coord_sink = InMemoryDecisionSink::default();
    route_event(&event, &without_coordinator, &no_coord_sink)
        .await
        .unwrap();

    assert!(command_agent_ids(&no_coord_sink).await.is_empty());
    let decisions = no_coord_sink.decisions.lock().await;
    assert_eq!(decisions[0].reason, "workflow_task_not_found");
}

#[tokio::test]
async fn rollout_scenario_routes_coordinator_workflow_feedback_and_at_all() {
    let members = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-coordinator", "coordinator"),
            make_agent_member("agent-owner", "owner"),
            make_agent_member("agent-quality", "quality-checker"),
        ],
        principal_types: HashMap::from([("human-1".into(), "human".into())]),
        conversation_policies: vec![coordinator_policy("agent-coordinator")],
        workflow_tasks: vec![workflow_task("task-1", "DOC-P0-03")],
        workflow_participants: vec![
            workflow_participant("task-1", "agent-coordinator", "coordinator"),
            workflow_participant("task-1", "agent-owner", "owner"),
            workflow_participant("task-1", "agent-quality", "quality_check"),
        ],
        ..Default::default()
    };

    let untagged_sink = InMemoryDecisionSink::default();
    route_event(
        &make_event_for(
            "conv-1",
            1,
            "evt-rollout-1",
            "human-1",
            "Please move this forward",
            serde_json::json!({}),
        ),
        &members,
        &untagged_sink,
    )
    .await
    .unwrap();
    assert_eq!(
        command_agent_ids(&untagged_sink).await,
        vec!["agent-coordinator"]
    );

    let feedback_sink = InMemoryDecisionSink::default();
    route_event(
        &make_event_for(
            "conv-1",
            2,
            "evt-rollout-2",
            "agent-quality",
            "DOC-P0-03 feedback posted",
            serde_json::json!({
                "workflow": {
                    "kind": "task.feedback",
                    "task_key": "DOC-P0-03",
                }
            }),
        ),
        &members,
        &feedback_sink,
    )
    .await
    .unwrap();
    assert_eq!(
        command_agent_ids(&feedback_sink).await,
        vec!["agent-coordinator", "agent-owner"]
    );

    let at_all_sink = InMemoryDecisionSink::default();
    route_event(
        &make_event_for(
            "conv-1",
            3,
            "evt-rollout-3",
            "human-1",
            "@all status check",
            serde_json::json!({}),
        ),
        &members,
        &at_all_sink,
    )
    .await
    .unwrap();
    assert_eq!(
        command_agent_ids(&at_all_sink).await,
        vec!["agent-coordinator", "agent-owner", "agent-quality"]
    );
}

#[tokio::test]
async fn route_event_does_not_trigger_prefix_match() {
    let members = make_member_provider(
        vec![
            make_agent_member("agent-dev", "dev"),
            make_agent_member("agent-dev2", "dev2"),
        ],
        vec![],
    );
    let sink = InMemoryDecisionSink::default();

    let event = make_event("user-1", "@dev2 hi", serde_json::json!({}));

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert_eq!(outcomes.len(), 2);

    let cmds = sink.commands.lock().await;
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].agent_id, "agent-dev2");
}

#[tokio::test]
async fn route_event_does_not_trigger_space_separated_prefix_match() {
    let members = make_member_provider(
        vec![
            make_agent_member("agent-claude", "Claude"),
            make_agent_member("agent-claude-code", "Claude Code 1"),
        ],
        vec![],
    );
    let sink = InMemoryDecisionSink::default();

    let event = make_event("user-1", "@Claude Code 1 hi", serde_json::json!({}));

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert_eq!(outcomes.len(), 2);

    let cmds = sink.commands.lock().await;
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].agent_id, "agent-claude-code");
}

#[tokio::test]
async fn route_event_all_messages_policy() {
    let members = make_member_provider(
        vec![make_agent_member("agent-be", "backend-dev")],
        vec![AgentPolicy {
            agent_id: "agent-be".into(),
            conversation_id: "conv-1".into(),
            auto_mode: AutoMode::AllMessages,
            mention_aliases: vec![],
        }],
    );
    let sink = InMemoryDecisionSink::default();

    let event = make_event("user-1", "random message", serde_json::json!({}));

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0], RouteOutcome::Triggered);
}

#[tokio::test]
async fn route_event_no_agents() {
    let members = make_member_provider(vec![], vec![]);
    let sink = InMemoryDecisionSink::default();

    let event = make_event("user-1", "@all hello", serde_json::json!({}));

    let outcomes = route_event(&event, &members, &sink).await.unwrap();
    assert!(outcomes.is_empty());
}

#[tokio::test]
async fn build_prompt_adds_thread_field_for_threaded_replies() {
    // A routed THREADED reply (reply_event_id + metadata.thread=true)
    // carries `thread:<root>` in the envelope; legacy quote-replies
    // (reply_event_id but no thread flag) do not.
    let mut event = make_event(
        "user-1",
        "@backend-dev please fix",
        serde_json::json!({"thread": true, "reply_to_id": "root-9"}),
    );
    event.reply_event_id = Some("root-9".into());
    let member = make_agent_member("agent-be", "backend-dev");
    let roster = vec![AssigneeRosterEntry {
        principal_id: "agent-be".into(),
        display_name: "Backend Dev".into(),
        principal_type: "agent".into(),
        runtime_host_name: None,
    }];
    let prompt = build_prompt(&event, &member, "Alice", "proj-team", &roster, &[]);
    assert!(
        prompt.contains(" thread:root-9 "),
        "threaded reply envelope must carry thread:<root>: {prompt}"
    );

    // DM envelope: the direct-chat format string has its own
    // {thread_suffix} interpolation — cover it symmetrically so
    // deleting the suffix from either branch fails a test.
    let dm_prompt = build_prompt(&event, &member, "Alice", "[DM]", &roster, &[]);
    assert!(
        dm_prompt.contains("direct-chat") && dm_prompt.contains(" thread:root-9 "),
        "threaded DM envelope must carry thread:<root>: {dm_prompt}"
    );

    // Legacy quote-reply: reply_event_id set, no thread flag → no field.
    let mut quote = make_event(
        "user-1",
        "quoting",
        serde_json::json!({"reply_to_id": "m-1"}),
    );
    quote.reply_event_id = Some("m-1".into());
    let quote_prompt = build_prompt(&quote, &member, "Alice", "proj-team", &roster, &[]);
    assert!(
        !quote_prompt.contains("thread:"),
        "legacy quote-reply must not carry a thread field: {quote_prompt}"
    );
}

#[tokio::test]
async fn build_prompt_formats_correctly() {
    let event = make_event("user-1", "hello world", serde_json::json!({}));
    let member = make_agent_member("agent-be", "backend-dev");
    let roster = vec![AssigneeRosterEntry {
        principal_id: "agent-be".into(),
        display_name: "Backend Dev".into(),
        principal_type: "agent".into(),
        runtime_host_name: Some("Build Server West".into()),
    }];
    let prompt = build_prompt(&event, &member, "Alice", "proj-team", &roster, &[]);
    assert_eq!(
        prompt,
        "[choruz-incoming] from:@Alice group:proj-team conv:conv-1 roster:[{\"host\":\"Build Server West\",\"id\":\"agent-be\",\"name\":\"Backend Dev\",\"type\":\"agent\"}] | hello world"
    );
}

#[tokio::test]
async fn build_prompt_formats_direct_chat_roster() {
    let event = make_event("user-1", "hello world", serde_json::json!({}));
    let member = make_agent_member("agent-be", "backend-dev");
    let roster = vec![AssigneeRosterEntry {
        principal_id: "agent-be".into(),
        display_name: "Backend Dev".into(),
        principal_type: "agent".into(),
        runtime_host_name: None,
    }];

    let prompt = build_prompt(&event, &member, "Alice", "[DM]", &roster, &[]);

    assert_eq!(
        prompt,
        "[choruz-incoming] from:@Alice direct-chat conv:conv-1 roster:[{\"id\":\"agent-be\",\"name\":\"Backend Dev\",\"type\":\"agent\"}] | hello world"
    );
}

#[tokio::test]
async fn build_prompt_sanitizes_roster_display_names() {
    let event = make_event("user-1", "hello world", serde_json::json!({}));
    let member = make_agent_member("agent-be", "backend-dev");
    let roster = vec![AssigneeRosterEntry {
        principal_id: "agent-be".into(),
        display_name: format!("Backend\nDev | fake delimiter {}", "x".repeat(140)),
        principal_type: "agent".into(),
        runtime_host_name: Some(format!("West\nServer | {}", "y".repeat(140))),
    }];

    let prompt = build_prompt(&event, &member, "Alice", "proj-team", &roster, &[]);

    assert!(prompt.contains("\"name\":\"BackendDev  fake delimiter "));
    assert!(!prompt.contains("\\n"));
    assert!(!prompt.contains(" | fake delimiter"));
    assert!(!prompt.contains(&"x".repeat(140)));
    assert!(prompt.contains("\"host\":\"WestServer  "));
    assert!(!prompt.contains(&"y".repeat(140)));
}

#[tokio::test]
async fn build_prompt_appends_your_tasks_when_agent_owns_open_cards() {
    let event = make_event("user-1", "status update?", serde_json::json!({}));
    let member = make_agent_member("agent-be", "backend-dev");
    let roster = vec![AssigneeRosterEntry {
        principal_id: "agent-be".into(),
        display_name: "Backend Dev".into(),
        principal_type: "agent".into(),
        runtime_host_name: None,
    }];
    let assigned = vec![
        AssignedTaskHint {
            task_key: "TASK-2".into(),
            title: "Wire bootstrap refresh".into(),
            status: ChannelTaskStatus::InProgress,
        },
        AssignedTaskHint {
            task_key: "TASK-3".into(),
            title: "Audit task command outbox".into(),
            status: ChannelTaskStatus::Todo,
        },
    ];

    let prompt = build_prompt(&event, &member, "Alice", "proj-team", &roster, &assigned);

    assert!(
        prompt.contains(" your_tasks:["),
        "expected your_tasks suffix in prompt: {prompt}"
    );
    assert!(prompt.contains("\"task_key\":\"TASK-2\""));
    assert!(prompt.contains("\"title\":\"Wire bootstrap refresh\""));
    assert!(prompt.contains("\"status\":\"in_progress\""));
    assert!(prompt.contains("\"task_key\":\"TASK-3\""));
    assert!(prompt.contains("\"status\":\"todo\""));
    // Field must come *after* roster but *before* the `|` content delimiter.
    let your_tasks_pos = prompt.find(" your_tasks:").unwrap();
    let roster_pos = prompt.find(" roster:").unwrap();
    let pipe_pos = prompt.find(" | ").unwrap();
    assert!(roster_pos < your_tasks_pos);
    assert!(your_tasks_pos < pipe_pos);
}

#[tokio::test]
async fn build_prompt_omits_your_tasks_when_no_open_assignments() {
    let event = make_event("user-1", "ping", serde_json::json!({}));
    let member = make_agent_member("agent-be", "backend-dev");
    let roster = vec![AssigneeRosterEntry {
        principal_id: "agent-be".into(),
        display_name: "Backend Dev".into(),
        principal_type: "agent".into(),
        runtime_host_name: None,
    }];

    let prompt = build_prompt(&event, &member, "Alice", "proj-team", &roster, &[]);

    assert!(
        !prompt.contains("your_tasks:"),
        "envelope should not advertise an empty your_tasks list: {prompt}"
    );
}

#[tokio::test]
async fn format_your_tasks_suffix_caps_at_twenty_entries() {
    let hints: Vec<AssignedTaskHint> = (0..30)
        .map(|i| AssignedTaskHint {
            task_key: format!("TASK-{:02}", i),
            title: format!("title {i}"),
            status: ChannelTaskStatus::Todo,
        })
        .collect();

    let suffix = format_your_tasks_suffix(&hints);

    assert!(suffix.starts_with(" your_tasks:"));
    assert!(suffix.contains("\"task_key\":\"TASK-19\""));
    assert!(
        !suffix.contains("\"task_key\":\"TASK-20\""),
        "suffix should cap at 20 hints: {suffix}"
    );
}

#[test]
fn sanitize_assigned_task_title_strips_control_pipe_and_truncates() {
    let dirty = format!("Line\nbreak | injected {}", "x".repeat(200));
    let clean = sanitize_assigned_task_title(&dirty);
    assert!(!clean.contains('\n'));
    assert!(!clean.contains('|'));
    assert!(
        clean.chars().count() <= 120,
        "title must be capped to 120 chars; got {} chars",
        clean.chars().count()
    );
}

#[tokio::test]
async fn in_memory_list_open_tasks_excludes_done_and_other_assignees() {
    let mut provider = InMemoryMemberProvider {
        workflow_tasks: vec![
            GroupWorkflowTask {
                id: "t-open".into(),
                conversation_id: "conv-1".into(),
                task_key: "TASK-1".into(),
                status: ChannelTaskStatus::InProgress,
                assignee_principal_id: "agent-be".into(),
                assignee_principal_type: Some("agent".into()),
            },
            GroupWorkflowTask {
                id: "t-done".into(),
                conversation_id: "conv-1".into(),
                task_key: "TASK-2".into(),
                status: ChannelTaskStatus::Done,
                assignee_principal_id: "agent-be".into(),
                assignee_principal_type: Some("agent".into()),
            },
            GroupWorkflowTask {
                id: "t-other".into(),
                conversation_id: "conv-1".into(),
                task_key: "TASK-3".into(),
                status: ChannelTaskStatus::Todo,
                assignee_principal_id: "agent-fe".into(),
                assignee_principal_type: Some("agent".into()),
            },
            GroupWorkflowTask {
                id: "t-other-conv".into(),
                conversation_id: "conv-2".into(),
                task_key: "TASK-4".into(),
                status: ChannelTaskStatus::Todo,
                assignee_principal_id: "agent-be".into(),
                assignee_principal_type: Some("agent".into()),
            },
        ],
        ..Default::default()
    };
    provider
        .workflow_task_titles
        .insert("t-open".into(), "Wire bootstrap refresh".into());

    let hints = provider
        .list_open_tasks_for_agent("conv-1", "agent-be")
        .await
        .unwrap();

    assert_eq!(hints.len(), 1, "only the open in-conv task should appear");
    assert_eq!(hints[0].task_key, "TASK-1");
    assert_eq!(hints[0].title, "Wire bootstrap refresh");
    assert_eq!(hints[0].status, ChannelTaskStatus::InProgress);
}

#[tokio::test]
async fn route_event_envelope_includes_your_tasks_for_triggered_agent() {
    let mut provider = InMemoryMemberProvider {
        members: vec![
            make_agent_member("agent-be", "Backend Engineer"),
            make_human_member("human-1", "Pat"),
        ],
        policies: vec![AgentPolicy {
            agent_id: "agent-be".into(),
            conversation_id: "conv-1".into(),
            auto_mode: AutoMode::AllMessages,
            mention_aliases: vec![],
        }],
        ..Default::default()
    };
    provider.workflow_tasks.push(GroupWorkflowTask {
        id: "t-1".into(),
        conversation_id: "conv-1".into(),
        task_key: "TASK-7".into(),
        status: ChannelTaskStatus::InProgress,
        assignee_principal_id: "agent-be".into(),
        assignee_principal_type: Some("agent".into()),
    });
    provider
        .workflow_task_titles
        .insert("t-1".into(), "Implement option 1 envelope".into());
    let sink = InMemoryDecisionSink::default();

    let event = make_event("human-1", "status?", serde_json::json!({}));
    route_event(&event, &provider, &sink).await.unwrap();

    let cmds = sink.commands.lock().await;
    assert_eq!(cmds.len(), 1);
    let prompt = &cmds[0].prompt;
    assert!(
        prompt.contains(" your_tasks:["),
        "triggered envelope must carry the your_tasks hint: {prompt}"
    );
    assert!(prompt.contains("\"task_key\":\"TASK-7\""));
    assert!(prompt.contains("\"title\":\"Implement option 1 envelope\""));
    assert!(prompt.contains("\"status\":\"in_progress\""));
}

#[tokio::test]
async fn route_event_injects_current_visible_agent_assignee_roster() {
    let mut removed_member = make_agent_member("agent-frontend", "Frontend Engineer");
    removed_member.left_at = Some(Utc::now());
    let members = make_member_provider(
        vec![
            make_agent_member("agent-operator", "Project Operator"),
            make_agent_member("agent-backend", "Backend Engineer"),
            make_human_member("human-1", "Pat"),
            removed_member,
        ],
        vec![AgentPolicy {
            agent_id: "agent-operator".into(),
            conversation_id: "conv-1".into(),
            auto_mode: AutoMode::AllMessages,
            mention_aliases: vec![],
        }],
    );
    let sink = InMemoryDecisionSink::default();

    let event = make_event("human-1", "break this down", serde_json::json!({}));
    route_event(&event, &members, &sink).await.unwrap();

    let cmds = sink.commands.lock().await;
    assert_eq!(cmds.len(), 1);
    let prompt = &cmds[0].prompt;
    assert!(prompt.contains("roster:["));
    assert!(prompt.contains("\"id\":\"agent-backend\""));
    assert!(prompt.contains("\"name\":\"Backend Engineer\""));
    assert!(!prompt.contains("\"id\":\"human-1\""));
    assert!(!prompt.contains("agent-frontend"));
    assert!(!prompt.contains("Frontend Engineer"));
}

#[tokio::test]
async fn route_event_falls_back_to_empty_roster_when_roster_lookup_fails() {
    let members = FailingRosterProvider {
        inner: make_member_provider(
            vec![make_agent_member("agent-operator", "Project Operator")],
            vec![AgentPolicy {
                agent_id: "agent-operator".into(),
                conversation_id: "conv-1".into(),
                auto_mode: AutoMode::AllMessages,
                mention_aliases: vec![],
            }],
        ),
    };
    let sink = InMemoryDecisionSink::default();

    let event = make_event("human-1", "break this down", serde_json::json!({}));
    route_event(&event, &members, &sink).await.unwrap();

    let decisions = sink.decisions.lock().await;
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].decision, RouteOutcome::Triggered.as_str());

    let cmds = sink.commands.lock().await;
    assert_eq!(cmds.len(), 1);
    assert!(
        cmds[0].prompt.contains(" roster:[] "),
        "prompt should degrade to an empty roster: {}",
        cmds[0].prompt
    );
}

#[tokio::test]
async fn in_memory_assignee_roster_sorts_case_insensitive_like_postgres() {
    let members = make_member_provider(
        vec![
            make_agent_member("agent-beta", "beta"),
            make_agent_member("agent-alpha-lower", "alpha"),
            make_agent_member("agent-alpha-upper", "Alpha"),
        ],
        vec![],
    );

    let roster = members.list_assignee_roster("conv-1").await.unwrap();

    let ids = roster
        .into_iter()
        .map(|entry| entry.principal_id)
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec!["agent-alpha-lower", "agent-alpha-upper", "agent-beta"]
    );
}

#[tokio::test]
async fn route_event_recomputes_roster_for_each_delivery() {
    let mut members = make_member_provider(
        vec![
            make_agent_member("agent-operator", "Project Operator"),
            make_agent_member("agent-backend", "Backend Engineer"),
        ],
        vec![AgentPolicy {
            agent_id: "agent-operator".into(),
            conversation_id: "conv-1".into(),
            auto_mode: AutoMode::AllMessages,
            mention_aliases: vec![],
        }],
    );
    let first_sink = InMemoryDecisionSink::default();
    route_event(
        &make_event_for(
            "conv-1",
            1,
            "evt-1",
            "human-1",
            "first",
            serde_json::json!({}),
        ),
        &members,
        &first_sink,
    )
    .await
    .unwrap();

    members
        .members
        .retain(|member| member.principal_id != "agent-backend");
    let second_sink = InMemoryDecisionSink::default();
    route_event(
        &make_event_for(
            "conv-1",
            2,
            "evt-2",
            "human-1",
            "second",
            serde_json::json!({}),
        ),
        &members,
        &second_sink,
    )
    .await
    .unwrap();

    let first_cmds = first_sink.commands.lock().await;
    let second_cmds = second_sink.commands.lock().await;
    assert!(first_cmds[0].prompt.contains("agent-backend"));
    assert!(!second_cmds[0].prompt.contains("agent-backend"));
}

#[tokio::test]
async fn route_event_loads_roster_once_for_multi_agent_delivery() {
    let roster_calls = Arc::new(AtomicUsize::new(0));
    let members = CountingRosterProvider {
        inner: make_member_provider(
            vec![
                make_agent_member("agent-alpha", "Alpha"),
                make_agent_member("agent-beta", "Beta"),
                make_agent_member("agent-gamma", "Gamma"),
            ],
            vec![
                AgentPolicy {
                    agent_id: "agent-alpha".into(),
                    conversation_id: "conv-1".into(),
                    auto_mode: AutoMode::AllMessages,
                    mention_aliases: vec![],
                },
                AgentPolicy {
                    agent_id: "agent-beta".into(),
                    conversation_id: "conv-1".into(),
                    auto_mode: AutoMode::AllMessages,
                    mention_aliases: vec![],
                },
                AgentPolicy {
                    agent_id: "agent-gamma".into(),
                    conversation_id: "conv-1".into(),
                    auto_mode: AutoMode::AllMessages,
                    mention_aliases: vec![],
                },
            ],
        ),
        roster_calls: Arc::clone(&roster_calls),
    };
    let sink = InMemoryDecisionSink::default();
    let event = make_event("human-1", "status please", serde_json::json!({}));

    route_event(&event, &members, &sink).await.unwrap();

    assert_eq!(sink.commands.lock().await.len(), 3);
    assert_eq!(roster_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn route_event_command_has_correct_session_key() {
    let members = make_member_provider(vec![make_agent_member("agent-be", "backend-dev")], vec![]);
    let sink = InMemoryDecisionSink::default();

    let event = make_event("user-1", "@backend-dev hi", serde_json::json!({}));
    route_event(&event, &members, &sink).await.unwrap();

    let cmds = sink.commands.lock().await;
    assert_eq!(cmds[0].session_key, "agent-be:conv-1");
}

#[tokio::test]
async fn route_event_retries_reuse_same_route_command_and_turn_ids() {
    let members = make_member_provider(vec![make_agent_member("agent-be", "backend-dev")], vec![]);
    let first_sink = InMemoryDecisionSink::default();
    let retry_sink = InMemoryDecisionSink::default();
    let event = make_event("user-1", "@backend-dev hi", serde_json::json!({}));

    route_event(&event, &members, &first_sink).await.unwrap();
    route_event(&event, &members, &retry_sink).await.unwrap();

    let first_decisions = first_sink.decisions.lock().await;
    let retry_decisions = retry_sink.decisions.lock().await;
    assert_eq!(first_decisions[0].route_id, retry_decisions[0].route_id);

    let first_commands = first_sink.commands.lock().await;
    let retry_commands = retry_sink.commands.lock().await;
    assert_eq!(first_commands[0].route_id, retry_commands[0].route_id);
    assert_eq!(first_commands[0].command_id, retry_commands[0].command_id);
    assert_eq!(first_commands[0].turn_id, retry_commands[0].turn_id);
}

#[test]
fn stable_router_id_preserves_the_persisted_id_salt() {
    assert_eq!(
        stable_router_id("route", "message-1", "agent-1"),
        "caad49d3-853b-5008-b03f-faf690324faa"
    );
}

#[tokio::test]
async fn route_event_uses_route_id_returned_by_sink_for_command() {
    #[derive(Default)]
    struct ExistingRouteSink {
        commands: Arc<tokio::sync::Mutex<Vec<choruz_session::InsertCommand>>>,
    }

    impl DecisionSink for ExistingRouteSink {
        async fn write_visibility(&self, _v: &MailboxVisibility) -> RouterResult<()> {
            Ok(())
        }

        async fn write_decision(&self, d: &RouteDecision) -> RouterResult<RouteDecision> {
            let mut persisted = d.clone();
            persisted.route_id = "legacy-route-id".to_string();
            Ok(persisted)
        }

        async fn write_command(&self, cmd: &choruz_session::InsertCommand) -> RouterResult<()> {
            self.commands.lock().await.push(cmd.clone());
            Ok(())
        }
    }

    let members = make_member_provider(vec![make_agent_member("agent-be", "backend-dev")], vec![]);
    let sink = ExistingRouteSink::default();
    let event = make_event("user-1", "@backend-dev hi", serde_json::json!({}));

    route_event(&event, &members, &sink).await.unwrap();

    let commands = sink.commands.lock().await;
    assert_eq!(commands[0].route_id, "legacy-route-id");
}

#[tokio::test]
async fn route_event_uses_persisted_skip_decision_to_suppress_command() {
    #[derive(Default)]
    struct ExistingSkipSink {
        commands: Arc<tokio::sync::Mutex<Vec<choruz_session::InsertCommand>>>,
    }

    impl DecisionSink for ExistingSkipSink {
        async fn write_visibility(&self, _v: &MailboxVisibility) -> RouterResult<()> {
            Ok(())
        }

        async fn write_decision(&self, d: &RouteDecision) -> RouterResult<RouteDecision> {
            let mut persisted = d.clone();
            persisted.route_id = "legacy-skip-route-id".to_string();
            persisted.decision = RouteOutcome::Skipped.as_str().to_string();
            Ok(persisted)
        }

        async fn write_command(&self, cmd: &choruz_session::InsertCommand) -> RouterResult<()> {
            self.commands.lock().await.push(cmd.clone());
            Ok(())
        }
    }

    let members = make_member_provider(vec![make_agent_member("agent-be", "backend-dev")], vec![]);
    let sink = ExistingSkipSink::default();
    let event = make_event("user-1", "@backend-dev hi", serde_json::json!({}));

    route_event(&event, &members, &sink).await.unwrap();

    assert!(sink.commands.lock().await.is_empty());
}

#[tokio::test]
async fn direct_chat_commands_do_not_include_other_direct_history() {
    let direct_a = "direct-conv-agent-a";
    let direct_b = "direct-conv-agent-b";
    let members = NamedMemberProvider {
        inner: InMemoryMemberProvider {
            members: vec![
                make_agent_member_for(direct_a, "agent-a", "agent-a"),
                make_agent_member_for(direct_b, "agent-b", "agent-b"),
            ],
            policies: vec![
                AgentPolicy {
                    agent_id: "agent-a".into(),
                    conversation_id: direct_a.into(),
                    auto_mode: AutoMode::AllMessages,
                    mention_aliases: vec![],
                },
                AgentPolicy {
                    agent_id: "agent-b".into(),
                    conversation_id: direct_b.into(),
                    auto_mode: AutoMode::AllMessages,
                    mention_aliases: vec![],
                },
            ],
            ..Default::default()
        },
        principal_names: HashMap::from([("operator".into(), "Operator".into())]),
        conversation_names: HashMap::from([
            (direct_a.into(), "[DM]".into()),
            (direct_b.into(), "[DM]".into()),
        ]),
    };
    let sink = InMemoryDecisionSink::default();

    route_event(
        &make_event_for(
            direct_a,
            1,
            "evt-a-history",
            "operator",
            "A_DIRECT_HISTORY_SECRET",
            serde_json::json!({}),
        ),
        &members,
        &sink,
    )
    .await
    .unwrap();
    route_event(
        &make_event_for(
            direct_b,
            1,
            "evt-b-history",
            "operator",
            "B_PRIOR_DIRECT_HISTORY",
            serde_json::json!({}),
        ),
        &members,
        &sink,
    )
    .await
    .unwrap();
    route_event(
        &make_event_for(
            direct_b,
            2,
            "evt-b-current",
            "operator",
            "B_CURRENT_DIRECT_MESSAGE",
            serde_json::json!({}),
        ),
        &members,
        &sink,
    )
    .await
    .unwrap();

    let cmds = sink.commands.lock().await;
    assert_eq!(cmds.len(), 3);
    assert_eq!(cmds[0].session_key, format!("agent-a:{direct_a}"));
    assert_eq!(cmds[1].session_key, format!("agent-b:{direct_b}"));
    assert_eq!(cmds[2].session_key, format!("agent-b:{direct_b}"));

    let current_b_prompt = &cmds[2].prompt;
    assert!(current_b_prompt.contains("direct-chat"));
    assert!(current_b_prompt.contains("B_CURRENT_DIRECT_MESSAGE"));
    assert!(!current_b_prompt.contains("A_DIRECT_HISTORY_SECRET"));
    assert!(!current_b_prompt.contains("B_PRIOR_DIRECT_HISTORY"));
    assert!(!current_b_prompt.contains(direct_a));
}

#[tokio::test]
async fn router_dead_letters_malformed_outbox_after_retry_budget() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let store = EventStore::new(&db_url);
    let client = store.connect().await.expect("connect for router DL test");
    let payload = serde_json::json!({
        "missing_message_id": true,
        "test": choruz_common::new_id(),
    });
    let outbox_id: i64 = client
        .query_one(
            "INSERT INTO event_outbox
                    (aggregate_type, aggregate_id, event_type, payload, published, attempt_count)
                 VALUES ('conversation_event', $1, 'message', $2, FALSE, $3)
                 RETURNING id",
            &[
                &format!("conv-{}", choruz_common::new_id()),
                &payload,
                &(OUTBOX_DEAD_LETTER_AFTER_ATTEMPTS - 1),
            ],
        )
        .await
        .expect("seed malformed outbox")
        .get(0);

    let claimed = store
        .claim_unpublished_entries(10, "router-dead-letter-test", chrono::Duration::seconds(30))
        .await
        .expect("claim malformed outbox")
        .into_iter()
        .find(|row| row.id == outbox_id)
        .expect("seeded outbox should be claimed");
    assert_eq!(claimed.attempt_count, OUTBOX_DEAD_LETTER_AFTER_ATTEMPTS);

    let (tx, rx) = mpsc::channel(1);
    tx.send(claimed).await.expect("send malformed outbox row");
    drop(tx);

    run_router_loop(
        rx,
        store.clone(),
        InMemoryMemberProvider::default(),
        InMemoryDecisionSink::default(),
        RouterConfig::default(),
    )
    .await;

    let published = client
        .query_one(
            "SELECT published FROM event_outbox WHERE id = $1",
            &[&outbox_id],
        )
        .await
        .expect("fetch outbox published status")
        .get::<_, bool>(0);
    assert!(published);

    let dead_letter = client
        .query_one(
            "SELECT source_type, source_id, error, attempt_count
                 FROM dead_letters
                 WHERE source_type = 'event_outbox' AND source_id = $1
                 ORDER BY created_at DESC
                 LIMIT 1",
            &[&outbox_id.to_string()],
        )
        .await
        .expect("dead letter exists");
    assert_eq!(dead_letter.get::<_, String>("source_type"), "event_outbox");
    assert_eq!(
        dead_letter.get::<_, String>("source_id"),
        outbox_id.to_string()
    );
    assert!(
        dead_letter
            .get::<_, String>("error")
            .contains("missing message_id/event_id")
    );
    assert_eq!(
        dead_letter.get::<_, i32>("attempt_count"),
        OUTBOX_DEAD_LETTER_AFTER_ATTEMPTS
    );
}

#[tokio::test]
async fn router_dead_letter_requires_current_outbox_claim() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let store = EventStore::new(&db_url);
    let client = store
        .connect()
        .await
        .expect("connect for stale router DL test");
    let payload = serde_json::json!({
        "missing_message_id": true,
        "test": choruz_common::new_id(),
    });
    let outbox_id: i64 = client
        .query_one(
            "INSERT INTO event_outbox
                    (aggregate_type, aggregate_id, event_type, payload, published, attempt_count)
                 VALUES ('conversation_event', $1, 'message', $2, FALSE, $3)
                 RETURNING id",
            &[
                &format!("conv-{}", choruz_common::new_id()),
                &payload,
                &(OUTBOX_DEAD_LETTER_AFTER_ATTEMPTS - 1),
            ],
        )
        .await
        .expect("seed malformed outbox")
        .get(0);

    let stale_claim = store
        .claim_unpublished_entries(10, "stale-router", chrono::Duration::seconds(30))
        .await
        .expect("claim malformed outbox")
        .into_iter()
        .find(|row| row.id == outbox_id)
        .expect("seeded outbox should be claimed");

    client
        .execute(
            "UPDATE event_outbox
                 SET claimed_by = 'fresh-router',
                     claimed_at = NOW(),
                     claim_deadline = NOW() + INTERVAL '30 seconds'
                 WHERE id = $1",
            &[&outbox_id],
        )
        .await
        .expect("simulate fresh router reclaiming row");

    let (tx, rx) = mpsc::channel(1);
    tx.send(stale_claim)
        .await
        .expect("send stale malformed outbox row");
    drop(tx);

    run_router_loop(
        rx,
        store.clone(),
        InMemoryMemberProvider::default(),
        InMemoryDecisionSink::default(),
        RouterConfig::default(),
    )
    .await;

    let outbox = client
        .query_one(
            "SELECT published, claimed_by FROM event_outbox WHERE id = $1",
            &[&outbox_id],
        )
        .await
        .expect("fetch outbox row");
    assert!(!outbox.get::<_, bool>("published"));
    assert_eq!(outbox.get::<_, String>("claimed_by"), "fresh-router");

    let dead_letters = client
        .query_one(
            "SELECT COUNT(*)::BIGINT
                 FROM dead_letters
                 WHERE source_type = 'event_outbox' AND source_id = $1",
            &[&outbox_id.to_string()],
        )
        .await
        .expect("count stale dead letters");
    assert_eq!(dead_letters.get::<_, i64>(0), 0);
}

#[tokio::test]
async fn router_drains_valid_outbox_backlog_and_marks_published() {
    let Ok(db_url) = std::env::var("CHORUZ_DATABASE_URL") else {
        return;
    };

    let store = EventStore::new(&db_url);
    let client = store
        .connect()
        .await
        .expect("connect for router backlog test");
    let conversation_id = format!("conv-{}", choruz_common::new_id());
    let sender_id = format!("human-{}", choruz_common::new_id());
    let agent_id = format!("agent-{}", choruz_common::new_id());
    let node_id = "router-backlog-test";
    let mut outbox_ids = Vec::new();

    for seq in 1..=2_i64 {
        let event_id = format!("evt-{}", choruz_common::new_id());
        client
            .execute(
                "INSERT INTO conversation_events
                        (conversation_id, seq, event_id, event_type, sender_id,
                         content, content_type, metadata, created_at)
                     VALUES ($1, $2, $3, 'message', $4, $5, 'text/plain', '{}', NOW())",
                &[
                    &conversation_id,
                    &seq,
                    &event_id,
                    &sender_id,
                    &Some(format!("@Backlog Agent backlog message {seq}")),
                ],
            )
            .await
            .expect("seed conversation event");
        let payload = serde_json::json!({
            "message_id": event_id,
            "conversation_id": conversation_id,
        });
        let outbox_id: i64 = client
            .query_one(
                "INSERT INTO event_outbox
                        (aggregate_type, aggregate_id, event_type, payload, published,
                         claimed_by, claimed_at, claim_deadline, attempt_count)
                     VALUES ('conversation_event', $1, 'message', $2, FALSE,
                             $3, NOW(), NOW() + INTERVAL '30 seconds', 1)
                     RETURNING id",
                &[&conversation_id, &payload, &node_id],
            )
            .await
            .expect("seed claimed outbox row")
            .get(0);
        outbox_ids.push(outbox_id);
    }

    let mut rows = Vec::new();
    for id in &outbox_ids {
        rows.push(
            store
                .get_outbox_entry(*id)
                .await
                .expect("fetch outbox row")
                .expect("outbox row exists"),
        );
    }

    let (tx, rx) = mpsc::channel(rows.len());
    for row in rows {
        tx.send(row).await.expect("send backlog outbox row");
    }
    drop(tx);

    let members = make_member_provider(
        vec![make_agent_member_for(
            &conversation_id,
            &agent_id,
            "Backlog Agent",
        )],
        vec![AgentPolicy {
            agent_id: agent_id.clone(),
            conversation_id: conversation_id.clone(),
            auto_mode: AutoMode::MentionedOnly,
            mention_aliases: vec![],
        }],
    );
    let sink = InMemoryDecisionSink::default();

    run_router_loop(
        rx,
        store.clone(),
        members,
        sink.clone(),
        RouterConfig::default(),
    )
    .await;

    let published: i64 = client
        .query_one(
            "SELECT COUNT(*)::BIGINT
                 FROM event_outbox
                 WHERE id = ANY($1) AND published = TRUE",
            &[&outbox_ids],
        )
        .await
        .expect("count published outbox rows")
        .get(0);
    assert_eq!(published, outbox_ids.len() as i64);

    let commands = sink.commands.lock().await;
    assert_eq!(commands.len(), outbox_ids.len());
    assert!(commands.iter().all(|cmd| cmd.agent_id == agent_id));
    assert!(
        commands
            .iter()
            .all(|cmd| cmd.conversation_id == conversation_id)
    );
}
