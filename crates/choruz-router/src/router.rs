//! Core router logic: consume conversation events, evaluate policies,
//! write visibility + decisions, and generate agent commands.
//!
//! The router uses an in-memory channel (from choruz-store's CDC poller)
//! instead of Kafka, following the pattern established in Phase B.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use choruz_store::{ConversationEventRow, EventStore, OutboxRow};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing;
use uuid::Uuid;

use crate::models::{
    AgentPolicy, AssignedTaskHint, AssigneeRosterEntry, AutoMode, ChannelTaskStatus,
    ConversationMember, ConversationRoutingPolicy, GroupWorkflowTask, GroupWorkflowTaskParticipant,
    MailboxVisibility, RouteDecision, RouteOutcome, UntaggedHumanMode, WorkflowRoutingEvent,
};
use crate::policy::evaluate_trigger_with_candidates;
use crate::workflow::parse_workflow_routing_event;

const OUTBOX_DEAD_LETTER_AFTER_ATTEMPTS: i32 = 5;

// ---------------------------------------------------------------------------
// Router error
// ---------------------------------------------------------------------------

/// Errors from the router.
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("store error: {0}")]
    Store(#[from] choruz_common::AppError),

    #[error("session error: {0}")]
    Session(#[from] choruz_session::SessionError),

    #[error("deserialization error: {0}")]
    Deserialization(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type RouterResult<T> = Result<T, RouterError>;

// ---------------------------------------------------------------------------
// MemberProvider trait — abstracts the lookup of conversation members and
// agent policies so we can test without a real database.
// ---------------------------------------------------------------------------

/// Provides conversation membership and policy data to the router.
#[allow(async_fn_in_trait)]
pub trait MemberProvider {
    /// List active agent members of a conversation.
    async fn list_agent_members(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<ConversationMember>>;

    /// List current valid visible agent task assignees for a conversation.
    async fn list_assignee_roster(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<AssigneeRosterEntry>>;

    /// Get the routing policy for an agent in a conversation.
    /// Returns a default MentionedOnly policy if none is configured.
    async fn get_agent_policy(
        &self,
        agent_id: &str,
        conversation_id: &str,
    ) -> RouterResult<AgentPolicy>;

    /// Resolve a principal's display name by ID.
    /// Returns None if not found.
    async fn resolve_principal_name(&self, principal_id: &str) -> Option<String>;

    /// Resolve a conversation's name by ID.
    /// Returns None if not found.
    async fn resolve_conversation_name(&self, conversation_id: &str) -> Option<String>;

    /// Resolve a principal's type by ID.
    async fn resolve_principal_type(
        &self,
        _principal_id: &str,
        _conversation_id: &str,
    ) -> RouterResult<Option<String>> {
        Ok(None)
    }

    /// Load the conversation-level routing policy.
    async fn get_conversation_routing_policy(
        &self,
        conversation_id: &str,
    ) -> RouterResult<ConversationRoutingPolicy> {
        Ok(ConversationRoutingPolicy::default_for(conversation_id))
    }

    /// Find workflow task state by canonical ID or conversation-local task key.
    async fn find_workflow_task(
        &self,
        _conversation_id: &str,
        _task_id: Option<&str>,
        _task_key: Option<&str>,
    ) -> RouterResult<Option<GroupWorkflowTask>> {
        Ok(None)
    }

    /// List participants for a workflow task.
    async fn list_workflow_task_participants(
        &self,
        _task_id: &str,
    ) -> RouterResult<Vec<GroupWorkflowTaskParticipant>> {
        Ok(Vec::new())
    }

    /// List open (non-`done`) tasks assigned to a specific principal in a
    /// conversation, used to populate the `your_tasks:` hint in the
    /// `[choruz-incoming]` envelope so the receiving agent knows which board
    /// cards it already owns.
    ///
    /// Implementations should return an empty list when the principal has no
    /// open assignments — that signal causes the prompt builder to omit the
    /// field entirely.
    async fn list_open_tasks_for_agent(
        &self,
        _conversation_id: &str,
        _principal_id: &str,
    ) -> RouterResult<Vec<AssignedTaskHint>> {
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// DecisionSink trait — abstracts where routing outputs go.
// ---------------------------------------------------------------------------

/// Sink for routing outputs (visibility, decisions, commands).
#[allow(async_fn_in_trait)]
pub trait DecisionSink {
    /// Write a mailbox visibility record.
    async fn write_visibility(&self, v: &MailboxVisibility) -> RouterResult<()>;

    /// Write a route decision record.
    async fn write_decision(&self, d: &RouteDecision) -> RouterResult<RouteDecision>;

    /// Write an agent command to the agent_commands table.
    async fn write_command(&self, cmd: &choruz_session::InsertCommand) -> RouterResult<()>;
}

// ---------------------------------------------------------------------------
// InMemoryMemberProvider — for testing
// ---------------------------------------------------------------------------

/// A test-friendly in-memory member provider.
#[derive(Default, Clone)]
pub struct InMemoryMemberProvider {
    pub members: Vec<ConversationMember>,
    pub policies: Vec<AgentPolicy>,
    pub principal_types: HashMap<String, String>,
    pub conversation_policies: Vec<ConversationRoutingPolicy>,
    pub workflow_tasks: Vec<GroupWorkflowTask>,
    pub workflow_participants: Vec<GroupWorkflowTaskParticipant>,
    /// Optional titles keyed by `GroupWorkflowTask::id` for use in
    /// `list_open_tasks_for_agent`; missing keys fall back to the task key.
    pub workflow_task_titles: HashMap<String, String>,
}

impl MemberProvider for InMemoryMemberProvider {
    async fn list_agent_members(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<ConversationMember>> {
        Ok(self
            .members
            .iter()
            .filter(|m| {
                m.conversation_id == conversation_id
                    && m.principal_type == "agent"
                    && m.left_at.is_none()
            })
            .cloned()
            .collect())
    }

    async fn list_assignee_roster(
        &self,
        conversation_id: &str,
    ) -> RouterResult<Vec<AssigneeRosterEntry>> {
        let mut roster = self
            .members
            .iter()
            .filter(|m| {
                m.conversation_id == conversation_id
                    && m.left_at.is_none()
                    && m.principal_type == "agent"
            })
            .map(|member| AssigneeRosterEntry {
                principal_id: member.principal_id.clone(),
                display_name: member
                    .display_name
                    .clone()
                    .unwrap_or_else(|| member.principal_id.clone()),
                principal_type: member.principal_type.clone(),
                runtime_host_name: None,
            })
            .collect::<Vec<_>>();
        roster.sort_by(|left, right| {
            left.display_name
                .to_lowercase()
                .cmp(&right.display_name.to_lowercase())
                .then_with(|| left.principal_id.cmp(&right.principal_id))
        });
        Ok(roster)
    }

    async fn get_agent_policy(
        &self,
        agent_id: &str,
        conversation_id: &str,
    ) -> RouterResult<AgentPolicy> {
        Ok(self
            .policies
            .iter()
            .find(|p| p.agent_id == agent_id && p.conversation_id == conversation_id)
            .cloned()
            .unwrap_or(AgentPolicy {
                agent_id: agent_id.into(),
                conversation_id: conversation_id.into(),
                auto_mode: AutoMode::MentionedOnly,
                mention_aliases: vec![],
            }))
    }

    async fn resolve_principal_name(&self, id: &str) -> Option<String> {
        Some(id[..8.min(id.len())].to_string())
    }

    async fn resolve_conversation_name(&self, id: &str) -> Option<String> {
        Some(id[..8.min(id.len())].to_string())
    }

    async fn resolve_principal_type(
        &self,
        principal_id: &str,
        _conversation_id: &str,
    ) -> RouterResult<Option<String>> {
        Ok(self.principal_types.get(principal_id).cloned())
    }

    async fn get_conversation_routing_policy(
        &self,
        conversation_id: &str,
    ) -> RouterResult<ConversationRoutingPolicy> {
        Ok(self
            .conversation_policies
            .iter()
            .find(|policy| policy.conversation_id == conversation_id)
            .cloned()
            .unwrap_or_else(|| ConversationRoutingPolicy::default_for(conversation_id)))
    }

    async fn find_workflow_task(
        &self,
        conversation_id: &str,
        task_id: Option<&str>,
        task_key: Option<&str>,
    ) -> RouterResult<Option<GroupWorkflowTask>> {
        if let Some(task_id) = task_id
            && let Some(task) = self
                .workflow_tasks
                .iter()
                .find(|task| task.conversation_id == conversation_id && task.id == task_id)
        {
            return Ok(task_key
                .is_none_or(|task_key| task.task_key == task_key)
                .then(|| task.clone()));
        }

        Ok(task_key.and_then(|task_key| {
            self.workflow_tasks
                .iter()
                .find(|task| task.conversation_id == conversation_id && task.task_key == task_key)
                .cloned()
        }))
    }

    async fn list_workflow_task_participants(
        &self,
        task_id: &str,
    ) -> RouterResult<Vec<GroupWorkflowTaskParticipant>> {
        Ok(self
            .workflow_participants
            .iter()
            .filter(|participant| participant.task_id == task_id)
            .cloned()
            .collect())
    }

    async fn list_open_tasks_for_agent(
        &self,
        conversation_id: &str,
        principal_id: &str,
    ) -> RouterResult<Vec<AssignedTaskHint>> {
        let mut hints: Vec<AssignedTaskHint> = self
            .workflow_tasks
            .iter()
            .filter(|task| {
                task.conversation_id == conversation_id
                    && task.assignee_principal_id == principal_id
                    && task.status != ChannelTaskStatus::Done
            })
            .map(|task| AssignedTaskHint {
                task_key: task.task_key.clone(),
                title: self
                    .workflow_task_titles
                    .get(&task.id)
                    .cloned()
                    .unwrap_or_else(|| task.task_key.clone()),
                status: task.status,
            })
            .collect();
        hints.sort_by(|left, right| left.task_key.cmp(&right.task_key));
        Ok(hints)
    }
}

// ---------------------------------------------------------------------------
// InMemoryDecisionSink — for testing
// ---------------------------------------------------------------------------

/// A test-friendly in-memory decision sink that collects outputs.
#[derive(Default, Clone)]
pub struct InMemoryDecisionSink {
    pub visibilities: Arc<tokio::sync::Mutex<Vec<MailboxVisibility>>>,
    pub decisions: Arc<tokio::sync::Mutex<Vec<RouteDecision>>>,
    pub commands: Arc<tokio::sync::Mutex<Vec<choruz_session::InsertCommand>>>,
}

impl DecisionSink for InMemoryDecisionSink {
    async fn write_visibility(&self, v: &MailboxVisibility) -> RouterResult<()> {
        self.visibilities.lock().await.push(v.clone());
        Ok(())
    }

    async fn write_decision(&self, d: &RouteDecision) -> RouterResult<RouteDecision> {
        self.decisions.lock().await.push(d.clone());
        Ok(d.clone())
    }

    async fn write_command(&self, cmd: &choruz_session::InsertCommand) -> RouterResult<()> {
        self.commands.lock().await.push(cmd.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// route_event — the core routing function (generic over provider/sink)
// ---------------------------------------------------------------------------

/// Route a single conversation event through the policy engine.
///
/// For each agent member of the conversation (excluding the sender):
/// 1. Write a mailbox_visibility record
/// 2. Evaluate the trigger policy
/// 3. Write a route_decision (always — skip or trigger)
/// 4. If triggered, generate and write an agent_command
pub async fn route_event<M, S>(
    event: &ConversationEventRow,
    members: &M,
    sink: &S,
) -> RouterResult<Vec<RouteOutcome>>
where
    M: MemberProvider,
    S: DecisionSink,
{
    let sender_name = members
        .resolve_principal_name(&event.sender_id)
        .await
        .unwrap_or_else(|| event.sender_id[..8.min(event.sender_id.len())].to_string());
    let conversation_name = members
        .resolve_conversation_name(&event.conversation_id)
        .await
        .unwrap_or_else(|| event.conversation_id[..8.min(event.conversation_id.len())].to_string());

    let agent_members = members.list_agent_members(&event.conversation_id).await?;
    let mut policies_by_agent = HashMap::new();
    for member in &agent_members {
        let policy = members
            .get_agent_policy(&member.principal_id, &event.conversation_id)
            .await?;
        policies_by_agent.insert(member.principal_id.clone(), policy);
    }
    let mention_candidates = mention_candidates(&agent_members, &policies_by_agent);
    let conversation_policy = members
        .get_conversation_routing_policy(&event.conversation_id)
        .await?;
    let route_plans = plan_routes(
        event,
        members,
        &agent_members,
        &policies_by_agent,
        &mention_candidates,
        &conversation_policy,
        conversation_name == "[DM]",
    )
    .await?;

    // Extract the FE-originated trace id from the event metadata (set by
    // `db_service::messages::send_message`). Used purely for log correlation
    // so a single user action (FE click → message → router → dispatch →
    // executor) can be stitched together in production logs.
    //
    // Convention: the string literal "none" means the request was genuinely
    // untraced (no `x-trace-id` header). That's intentionally different
    // from "-" (what we used before), because a `-` is a common default in
    // logs and could also look like "dropped along the way". Use "none"
    // everywhere so logs are self-describing.
    let trace_id: String = event
        .metadata
        .get("trace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("none")
        .to_string();

    log_route_plan_observability(&trace_id, event, &route_plans);

    let mut outcomes = Vec::new();
    let mut assignee_roster_cache: Option<Vec<AssigneeRosterEntry>> = None;

    for member in &agent_members {
        // Skip self-sent messages
        if member.principal_id == event.sender_id {
            // Self-sent skips used to be silent, which broke the "why
            // didn't Sally trigger?" debugging story when Sally was the
            // sender. Surface them at info! with the same structured
            // shape as other decisions so one grep by trace_id / agent_id
            // covers every case.
            tracing::info!(
                event = "agent_trigger_decision",
                decision = "skipped",
                trace_id = %trace_id,
                agent_id = %member.principal_id,
                conversation_id = %event.conversation_id,
                message_id = %event.event_id,
                sender_id = %event.sender_id,
                reason = "self_sent",
                "skipped agent (self-sent message)"
            );
            outcomes.push(RouteOutcome::Skipped);
            continue;
        }

        // C2: Write mailbox visibility
        let visibility = MailboxVisibility {
            agent_id: member.principal_id.clone(),
            conversation_id: event.conversation_id.clone(),
            message_id: event.event_id.clone(),
            event_seq: event.seq,
        };
        sink.write_visibility(&visibility).await?;

        // C3: Evaluate policy
        let policy = policies_by_agent
            .get(&member.principal_id)
            .expect("policy was loaded for every agent member");
        let route_plan = route_plans
            .get(&member.principal_id)
            .cloned()
            .unwrap_or_else(|| {
                let trigger =
                    evaluate_trigger_with_candidates(policy, event, member, &mention_candidates);
                PlannedRoute {
                    should_trigger: trigger.should_trigger,
                    reason: trigger.reason,
                    policy_snapshot: policy_snapshot(
                        policy,
                        serde_json::json!({
                            "routing_source": "policy"
                        }),
                    ),
                }
            });

        let route_id = stable_router_id("route", &event.event_id, &member.principal_id);
        let outcome = if route_plan.should_trigger {
            RouteOutcome::Triggered
        } else {
            RouteOutcome::Skipped
        };

        // C4: Write route decision (always — even for skips)
        let decision = RouteDecision {
            route_id: route_id.clone(),
            message_id: event.event_id.clone(),
            agent_id: member.principal_id.clone(),
            conversation_id: event.conversation_id.clone(),
            decision: outcome.as_str().into(),
            reason: route_plan.reason.clone(),
            policy_snapshot: route_plan.policy_snapshot.clone(),
        };
        let persisted_decision = sink.write_decision(&decision).await?;
        let persisted_trigger = persisted_decision.decision == RouteOutcome::Triggered.as_str();
        let persisted_outcome = if persisted_trigger {
            RouteOutcome::Triggered
        } else {
            RouteOutcome::Skipped
        };
        let route_id = persisted_decision.route_id;

        // C5: Generate agent command if triggered
        if persisted_trigger {
            let command_id = stable_router_id("command", &event.event_id, &member.principal_id);
            let turn_id = stable_router_id("turn", &event.event_id, &member.principal_id);
            let session_key = format!("{}:{}", member.principal_id, event.conversation_id);

            if assignee_roster_cache.is_none() {
                assignee_roster_cache = Some(
                    match members.list_assignee_roster(&event.conversation_id).await {
                        Ok(roster) => roster,
                        Err(error) => {
                            tracing::warn!(
                                trace_id = %trace_id,
                                conversation_id = %event.conversation_id,
                                error = %error,
                                "list_assignee_roster failed; envelope will use an empty roster"
                            );
                            Vec::new()
                        }
                    },
                );
            }
            // Per-recipient query — different agents in the same conversation
            // own different cards, so we can't share a cache across the loop.
            // Failures here are non-fatal: log and fall back to no hints
            // rather than blocking the route, since the existing behavior
            // (no hints) is what shipped before this change.
            let assigned_tasks = match members
                .list_open_tasks_for_agent(&event.conversation_id, &member.principal_id)
                .await
            {
                Ok(hints) => hints,
                Err(error) => {
                    tracing::warn!(
                        trace_id = %trace_id,
                        agent_id = %member.principal_id,
                        conversation_id = %event.conversation_id,
                        error = %error,
                        "list_open_tasks_for_agent failed; envelope will omit your_tasks hint"
                    );
                    Vec::new()
                }
            };
            let prompt = build_prompt(
                event,
                member,
                &sender_name,
                &conversation_name,
                assignee_roster_cache
                    .as_deref()
                    .expect("assignee roster loaded before prompt build"),
                &assigned_tasks,
            );

            // Forward any incoming attachment metadata to the executor so it
            // can stage the file into the agent's workspace before spawning
            // the CLI. We pass data through command.metadata rather than
            // mutating the prompt here, because the router has no access to
            // the agent's workspace_path.
            //
            // Also thread the FE trace_id forward so the executor, writer,
            // and reply-back flow can all cite the same correlator in their
            // structured logs. Without this, the trace goes dark after the
            // router — you lose the ability to answer "did my message's
            // agent actually finish?" from logs alone.
            let cmd_metadata = {
                let mut m = build_command_metadata(event);
                if trace_id != "none" {
                    if let Some(obj) = m.as_object_mut() {
                        obj.insert(
                            "trace_id".to_string(),
                            serde_json::Value::String(trace_id.clone()),
                        );
                    } else {
                        m = serde_json::json!({ "trace_id": trace_id });
                    }
                }
                m
            };

            let cmd = choruz_session::InsertCommand {
                command_id: command_id.clone(),
                route_id: route_id.clone(),
                session_key,
                agent_id: member.principal_id.clone(),
                conversation_id: event.conversation_id.clone(),
                message_id: event.event_id.clone(),
                turn_id: turn_id.clone(),
                prompt,
                max_attempts: choruz_session::DEFAULT_MAX_ATTEMPTS,
                metadata: cmd_metadata,
            };
            sink.write_command(&cmd).await?;

            tracing::info!(
                event = "agent_trigger_decision",
                decision = "triggered",
                trace_id = %trace_id,
                agent_id = %member.principal_id,
                conversation_id = %event.conversation_id,
                message_id = %event.event_id,
                command_id = %command_id,
                turn_id = %turn_id,
                sender_id = %event.sender_id,
                reason = %route_plan.reason,
                "triggered agent command"
            );
        } else {
            // Promoted to info! in round 3 so "why didn't Sally trigger?"
            // is answerable from production logs. Skipped decisions are
            // rare events (one per non-triggered agent per message) and
            // carry exactly the fields needed for debugging.
            tracing::info!(
                event = "agent_trigger_decision",
                decision = "skipped",
                trace_id = %trace_id,
                agent_id = %member.principal_id,
                conversation_id = %event.conversation_id,
                message_id = %event.event_id,
                sender_id = %event.sender_id,
                reason = %route_plan.reason,
                "skipped agent (not triggered)"
            );
        }

        outcomes.push(persisted_outcome);
    }

    Ok(outcomes)
}

#[derive(Clone)]
struct PlannedRoute {
    should_trigger: bool,
    reason: String,
    policy_snapshot: serde_json::Value,
}

async fn plan_routes<M>(
    event: &ConversationEventRow,
    members: &M,
    agent_members: &[ConversationMember],
    policies_by_agent: &HashMap<String, AgentPolicy>,
    mention_candidates: &[String],
    conversation_policy: &ConversationRoutingPolicy,
    is_direct_conversation: bool,
) -> RouterResult<HashMap<String, PlannedRoute>>
where
    M: MemberProvider,
{
    if let Some(plan) =
        explicit_target_plan(event, agent_members, policies_by_agent, mention_candidates)
    {
        return Ok(plan);
    }

    if !is_direct_conversation
        && let Some(workflow_event) = parse_workflow_routing_event(&event.metadata)
    {
        return workflow_target_plan(
            event,
            members,
            agent_members,
            policies_by_agent,
            conversation_policy,
            workflow_event,
        )
        .await;
    }

    if !is_direct_conversation {
        let sender_type = members
            .resolve_principal_type(&event.sender_id, &event.conversation_id)
            .await?;
        if sender_type.as_deref().is_some_and(|value| value == "human") {
            match conversation_policy.untagged_human_mode {
                UntaggedHumanMode::CoordinatorOnly => {
                    if let Some(coordinator_id) =
                        active_coordinator_agent_id(conversation_policy, agent_members, event)
                    {
                        return Ok(selected_targets_plan(
                            agent_members,
                            policies_by_agent,
                            HashSet::from([coordinator_id]),
                            "untagged_human_to_coordinator",
                            serde_json::json!({
                                "routing_source": "untagged_human_to_coordinator",
                                "untagged_human_mode": conversation_policy.untagged_human_mode.as_str(),
                                "default_coordinator_agent_id": conversation_policy.default_coordinator_agent_id.clone(),
                            }),
                        ));
                    }

                    return Ok(policy_fallback_plan(
                        event,
                        agent_members,
                        policies_by_agent,
                        mention_candidates,
                        serde_json::json!({
                            "routing_source": "coordinator_policy_fallback",
                            "untagged_human_mode": conversation_policy.untagged_human_mode.as_str(),
                            "coordinator_fallback_reason": coordinator_fallback_reason(
                                conversation_policy,
                                agent_members,
                                event,
                            ),
                            "default_coordinator_agent_id": conversation_policy.default_coordinator_agent_id.clone(),
                        }),
                    ));
                }
                UntaggedHumanMode::AllAgents => {
                    let targets = active_agent_ids(agent_members, &event.sender_id);
                    return Ok(selected_targets_plan(
                        agent_members,
                        policies_by_agent,
                        targets,
                        "untagged_human_to_all_agents",
                        serde_json::json!({
                            "routing_source": "untagged_human_to_all_agents",
                            "untagged_human_mode": conversation_policy.untagged_human_mode.as_str(),
                        }),
                    ));
                }
                UntaggedHumanMode::MentionedOnly => {
                    return Ok(policy_fallback_plan(
                        event,
                        agent_members,
                        policies_by_agent,
                        mention_candidates,
                        fallback_snapshot_extra(
                            event,
                            "untagged_human_mentioned_only",
                            serde_json::json!({
                                "untagged_human_mode": conversation_policy.untagged_human_mode.as_str(),
                            }),
                        ),
                    ));
                }
            }
        }
    }

    Ok(policy_fallback_plan(
        event,
        agent_members,
        policies_by_agent,
        mention_candidates,
        fallback_snapshot_extra(event, "policy", serde_json::json!({})),
    ))
}

fn explicit_target_plan(
    event: &ConversationEventRow,
    agent_members: &[ConversationMember],
    policies_by_agent: &HashMap<String, AgentPolicy>,
    mention_candidates: &[String],
) -> Option<HashMap<String, PlannedRoute>> {
    let explicit_results = agent_members
        .iter()
        .map(|member| {
            let policy = policies_by_agent
                .get(&member.principal_id)
                .expect("policy was loaded for every agent member");
            let explicit_policy = AgentPolicy {
                auto_mode: AutoMode::MentionedOnly,
                ..policy.clone()
            };
            (
                member.principal_id.clone(),
                evaluate_trigger_with_candidates(
                    &explicit_policy,
                    event,
                    member,
                    mention_candidates,
                ),
            )
        })
        .collect::<HashMap<_, _>>();

    if !explicit_results
        .values()
        .any(|trigger| trigger.should_trigger)
    {
        if let Some(explicit_metadata_target_id) = explicit_metadata_target_id(event) {
            return Some(selected_targets_plan(
                agent_members,
                policies_by_agent,
                HashSet::new(),
                "explicit_target_not_found",
                serde_json::json!({
                    "routing_source": "explicit_target",
                    "target_principal_id": explicit_metadata_target_id,
                }),
            ));
        }

        return None;
    }

    Some(
        agent_members
            .iter()
            .map(|member| {
                let trigger = explicit_results
                    .get(&member.principal_id)
                    .expect("explicit result exists for every agent member");
                let policy = policies_by_agent
                    .get(&member.principal_id)
                    .expect("policy was loaded for every agent member");
                let target_principal_id = if trigger.should_trigger {
                    member.principal_id.clone()
                } else {
                    String::new()
                };
                (
                    member.principal_id.clone(),
                    PlannedRoute {
                        should_trigger: trigger.should_trigger,
                        reason: if trigger.should_trigger {
                            trigger.reason.clone()
                        } else {
                            "explicit_target_not_selected".into()
                        },
                        policy_snapshot: policy_snapshot(
                            policy,
                            serde_json::json!({
                                "routing_source": "explicit_target",
                                "target_principal_id": target_principal_id,
                            }),
                        ),
                    },
                )
            })
            .collect(),
    )
}

fn explicit_metadata_target_id(event: &ConversationEventRow) -> Option<String> {
    event
        .metadata
        .get("turn_for")
        .or_else(|| event.metadata.get("request_review_by"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn workflow_target_plan<M>(
    event: &ConversationEventRow,
    members: &M,
    agent_members: &[ConversationMember],
    policies_by_agent: &HashMap<String, AgentPolicy>,
    conversation_policy: &ConversationRoutingPolicy,
    workflow_event: WorkflowRoutingEvent,
) -> RouterResult<HashMap<String, PlannedRoute>>
where
    M: MemberProvider,
{
    let task = members
        .find_workflow_task(
            &event.conversation_id,
            workflow_event.task_id.as_deref(),
            workflow_event.task_key.as_deref(),
        )
        .await?;
    let mut participant_roles = Vec::new();
    let mut target_roles = Vec::new();
    let mut target_ids = active_agent_ids_from(
        &workflow_event.target_principal_ids,
        agent_members,
        &event.sender_id,
    );
    let mut used_coordinator_fallback = false;
    let mut reason = if target_ids.is_empty() {
        "workflow_task_not_found".to_string()
    } else {
        "workflow_event_to_target_principal".to_string()
    };

    if target_ids.is_empty() {
        if let Some(task) = &task {
            let mut participants = members.list_workflow_task_participants(&task.id).await?;
            apply_canonical_owner_participant(&mut participants, task);
            participant_roles = participants
                .iter()
                .map(|participant| participant.role_key.clone())
                .collect();
            target_roles = workflow_target_roles(&workflow_event);
            target_ids = active_participant_ids_for_roles(
                &participants,
                &target_roles,
                agent_members,
                &event.sender_id,
            );

            if target_roles.iter().any(|role| role == "coordinator")
                && let Some(coordinator_id) =
                    active_coordinator_agent_id(conversation_policy, agent_members, event)
            {
                target_ids.insert(coordinator_id);
            }

            if target_ids.is_empty()
                && let Some(coordinator_id) =
                    active_coordinator_agent_id(conversation_policy, agent_members, event)
            {
                target_ids.insert(coordinator_id);
                used_coordinator_fallback = true;
            }

            reason = if used_coordinator_fallback {
                "workflow_event_to_coordinator".into()
            } else {
                workflow_reason(&workflow_event, !target_ids.is_empty())
            };
        } else if let Some(coordinator_id) =
            active_coordinator_agent_id(conversation_policy, agent_members, event)
        {
            target_ids.insert(coordinator_id);
            reason = "workflow_task_missing_coordinator_fallback".into();
        }
    }

    let task_id = task
        .as_ref()
        .map(|task| task.id.clone())
        .or_else(|| workflow_event.task_id.clone());
    let task_key = task
        .as_ref()
        .map(|task| task.task_key.clone())
        .or_else(|| workflow_event.task_key.clone());
    let target_role = workflow_event
        .target_role
        .clone()
        .or_else(|| workflow_event.next_role.clone())
        .or_else(|| target_roles.first().cloned());

    if target_ids.is_empty() {
        reason = "workflow_task_not_found".into();
    }

    Ok(selected_targets_plan(
        agent_members,
        policies_by_agent,
        target_ids,
        &reason,
        serde_json::json!({
            "routing_source": "workflow_event",
            "workflow_kind": workflow_event.kind,
            "task_key": task_key,
            "task_id": task_id,
            "task_status": task.as_ref().map(|task| task.status.as_str()),
            "assignee_principal_id": task.as_ref().map(|task| task.assignee_principal_id.as_str()),
            "target_role": target_role,
            "participant_roles": participant_roles,
        }),
    ))
}

fn apply_canonical_owner_participant(
    participants: &mut Vec<GroupWorkflowTaskParticipant>,
    task: &GroupWorkflowTask,
) {
    participants.retain(|participant| participant.role_key != "owner");
    participants.push(GroupWorkflowTaskParticipant {
        task_id: task.id.clone(),
        principal_id: task.assignee_principal_id.clone(),
        role_key: "owner".into(),
        principal_type: task.assignee_principal_type.clone(),
    });
}

fn workflow_target_roles(workflow_event: &WorkflowRoutingEvent) -> Vec<String> {
    if let Some(target_role) = &workflow_event.target_role {
        return vec![target_role.clone()];
    }

    match workflow_event.kind.as_str() {
        "task.started" => vec!["coordinator".into()],
        "task.ready_for_next_step" => workflow_event.next_role.iter().cloned().collect::<Vec<_>>(),
        "task.feedback" | "task.cleared" | "external_check.failed" => {
            vec!["owner".into(), "coordinator".into()]
        }
        "task.blocked" | "task.idle" | "external_check.passed" | "human_input_needed" => {
            vec!["coordinator".into()]
        }
        "approval_required" => vec!["approver".into(), "coordinator".into()],
        _ => Vec::new(),
    }
}

fn workflow_reason(workflow_event: &WorkflowRoutingEvent, has_targets: bool) -> String {
    if !has_targets {
        return "workflow_task_not_found".into();
    }

    match workflow_event.kind.as_str() {
        "task.ready_for_next_step" if workflow_event.next_role.is_some() => {
            "workflow_event_to_role".into()
        }
        "task.feedback" | "task.cleared" | "external_check.failed" => {
            "workflow_event_to_owner".into()
        }
        _ => "workflow_event_to_coordinator".into(),
    }
}

fn selected_targets_plan(
    agent_members: &[ConversationMember],
    policies_by_agent: &HashMap<String, AgentPolicy>,
    target_ids: HashSet<String>,
    reason: &str,
    snapshot_extra: serde_json::Value,
) -> HashMap<String, PlannedRoute> {
    agent_members
        .iter()
        .map(|member| {
            let policy = policies_by_agent
                .get(&member.principal_id)
                .expect("policy was loaded for every agent member");
            let should_trigger = target_ids.contains(&member.principal_id);
            let has_targets = !target_ids.is_empty();
            let mut extra = snapshot_extra.clone();
            if should_trigger && let Some(obj) = extra.as_object_mut() {
                obj.insert(
                    "target_principal_id".into(),
                    serde_json::Value::String(member.principal_id.clone()),
                );
            }
            (
                member.principal_id.clone(),
                PlannedRoute {
                    should_trigger,
                    reason: if should_trigger {
                        reason.to_string()
                    } else if has_targets {
                        format!("{reason}_not_targeted")
                    } else {
                        reason.to_string()
                    },
                    policy_snapshot: policy_snapshot(policy, extra),
                },
            )
        })
        .collect()
}

fn policy_fallback_plan(
    event: &ConversationEventRow,
    agent_members: &[ConversationMember],
    policies_by_agent: &HashMap<String, AgentPolicy>,
    mention_candidates: &[String],
    snapshot_extra: serde_json::Value,
) -> HashMap<String, PlannedRoute> {
    agent_members
        .iter()
        .map(|member| {
            let policy = policies_by_agent
                .get(&member.principal_id)
                .expect("policy was loaded for every agent member");
            let trigger =
                evaluate_trigger_with_candidates(policy, event, member, mention_candidates);
            let reason = snapshot_extra
                .get("coordinator_fallback_reason")
                .and_then(|value| value.as_str())
                .map(|fallback_reason| format!("{fallback_reason}: {}", trigger.reason))
                .unwrap_or(trigger.reason);
            (
                member.principal_id.clone(),
                PlannedRoute {
                    should_trigger: trigger.should_trigger,
                    reason,
                    policy_snapshot: policy_snapshot(policy, snapshot_extra.clone()),
                },
            )
        })
        .collect()
}

fn policy_snapshot(policy: &AgentPolicy, extra: serde_json::Value) -> serde_json::Value {
    let mut snapshot = serde_json::to_value(policy).unwrap_or(serde_json::json!({}));
    let Some(snapshot_obj) = snapshot.as_object_mut() else {
        return extra;
    };
    if let Some(extra_obj) = extra.as_object() {
        for (key, value) in extra_obj {
            if !(value.is_null() || value.is_string() && value.as_str() == Some("")) {
                snapshot_obj.insert(key.clone(), value.clone());
            }
        }
    }
    snapshot
}

fn fallback_snapshot_extra(
    event: &ConversationEventRow,
    routing_source: &str,
    mut extra: serde_json::Value,
) -> serde_json::Value {
    if !extra.is_object() {
        extra = serde_json::json!({});
    }
    let Some(extra_obj) = extra.as_object_mut() else {
        return serde_json::json!({ "routing_source": routing_source });
    };
    extra_obj.insert(
        "routing_source".into(),
        serde_json::Value::String(routing_source.into()),
    );
    if let Some(marker) = workflow_text_marker_without_metadata(event) {
        extra_obj.insert(
            "workflow_text_marker".into(),
            serde_json::Value::String(marker.into()),
        );
    }
    extra
}

fn workflow_text_marker_without_metadata(event: &ConversationEventRow) -> Option<&'static str> {
    if parse_workflow_routing_event(&event.metadata).is_some() {
        return None;
    }
    let content = event.content.as_deref()?.trim();
    if content.starts_with("[DONE]") {
        return Some("done_marker");
    }
    if content.starts_with("[BLOCKED]") {
        return Some("blocked_marker");
    }
    if content.to_ascii_lowercase().contains("feedback") && has_task_key_like_token(content) {
        return Some("feedback_with_task_key");
    }
    None
}

fn has_task_key_like_token(content: &str) -> bool {
    content
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
        .any(|token| {
            token.len() >= 5
                && token.contains('-')
                && token.chars().any(|ch| ch.is_ascii_alphabetic())
                && token.chars().any(|ch| ch.is_ascii_digit())
        })
}

fn log_route_plan_observability(
    trace_id: &str,
    event: &ConversationEventRow,
    route_plans: &HashMap<String, PlannedRoute>,
) {
    if route_plans.is_empty() {
        return;
    }

    let triggered_count = route_plans
        .values()
        .filter(|plan| plan.should_trigger)
        .count();
    let skipped_count = route_plans.len().saturating_sub(triggered_count);
    let all_skipped = triggered_count == 0;
    let summary_reason = route_plans
        .values()
        .find_map(|plan| route_plan_observability_reason(plan, all_skipped));
    let Some(summary_reason) = summary_reason else {
        return;
    };

    tracing::info!(
        event = "route_plan_summary",
        trace_id = %trace_id,
        conversation_id = %event.conversation_id,
        message_id = %event.event_id,
        sender_id = %event.sender_id,
        reason = %summary_reason,
        routing_source = %snapshot_string(route_plans, "routing_source"),
        workflow_kind = %snapshot_string(route_plans, "workflow_kind"),
        task_key = %snapshot_string(route_plans, "task_key"),
        workflow_text_marker = %snapshot_string(route_plans, "workflow_text_marker"),
        agent_count = route_plans.len(),
        triggered_count,
        skipped_count,
        "route plan observability summary"
    );
}

fn route_plan_observability_reason(plan: &PlannedRoute, all_skipped: bool) -> Option<&'static str> {
    if plan
        .policy_snapshot
        .get("workflow_text_marker")
        .and_then(|value| value.as_str())
        .is_some()
        && all_skipped
    {
        return Some("workflow_text_without_metadata_skipped");
    }

    let routing_source = plan
        .policy_snapshot
        .get("routing_source")
        .and_then(|value| value.as_str());
    if routing_source == Some("untagged_human_mentioned_only") && all_skipped {
        return Some("untagged_human_group_message_skipped_all_agents");
    }

    match plan.reason.as_str() {
        "workflow_task_missing_coordinator_fallback" => {
            Some("workflow_task_missing_coordinator_fallback")
        }
        "workflow_task_not_found" => Some("workflow_task_not_found"),
        _ => None,
    }
}

fn snapshot_string(route_plans: &HashMap<String, PlannedRoute>, key: &str) -> String {
    route_plans
        .values()
        .find_map(|plan| {
            plan.policy_snapshot
                .get(key)
                .and_then(|value| value.as_str())
        })
        .unwrap_or("")
        .to_string()
}

fn active_agent_ids(agent_members: &[ConversationMember], sender_id: &str) -> HashSet<String> {
    agent_members
        .iter()
        .filter(|member| member.principal_id != sender_id)
        .map(|member| member.principal_id.clone())
        .collect()
}

fn active_agent_ids_from(
    candidate_ids: &[String],
    agent_members: &[ConversationMember],
    sender_id: &str,
) -> HashSet<String> {
    let active = active_agent_ids(agent_members, sender_id);
    candidate_ids
        .iter()
        .filter(|candidate| active.contains(*candidate))
        .cloned()
        .collect()
}

fn active_participant_ids_for_roles(
    participants: &[GroupWorkflowTaskParticipant],
    role_keys: &[String],
    agent_members: &[ConversationMember],
    sender_id: &str,
) -> HashSet<String> {
    let active = active_agent_ids(agent_members, sender_id);
    participants
        .iter()
        .filter(|participant| {
            role_keys.iter().any(|role| role == &participant.role_key)
                && active.contains(&participant.principal_id)
        })
        .map(|participant| participant.principal_id.clone())
        .collect()
}

fn active_coordinator_agent_id(
    policy: &ConversationRoutingPolicy,
    agent_members: &[ConversationMember],
    event: &ConversationEventRow,
) -> Option<String> {
    let coordinator_id = policy.default_coordinator_agent_id.as_ref()?;
    if coordinator_id == &event.sender_id {
        return None;
    }
    agent_members
        .iter()
        .any(|member| &member.principal_id == coordinator_id)
        .then(|| coordinator_id.clone())
}

fn coordinator_fallback_reason(
    policy: &ConversationRoutingPolicy,
    agent_members: &[ConversationMember],
    event: &ConversationEventRow,
) -> &'static str {
    match policy.default_coordinator_agent_id.as_deref() {
        None => "coordinator_not_configured",
        Some(coordinator_id) if coordinator_id == event.sender_id => "coordinator_is_sender",
        Some(coordinator_id)
            if !agent_members
                .iter()
                .any(|member| member.principal_id == coordinator_id) =>
        {
            "coordinator_unavailable"
        }
        Some(_) => "coordinator_available",
    }
}

fn stable_router_id(kind: &str, message_id: &str, agent_id: &str) -> String {
    let mut hasher = Sha256::new();
    // This salt is a persisted-ID compatibility boundary, not a display name.
    // Changing it would generate different route, command, and turn IDs.
    hasher.update(b"echat-router-id-v1");
    hasher.update([0]);
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    hasher.update(message_id.as_bytes());
    hasher.update([0]);
    hasher.update(agent_id.as_bytes());
    let digest = hasher.finalize();

    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn mention_candidates(
    agent_members: &[ConversationMember],
    policies_by_agent: &HashMap<String, AgentPolicy>,
) -> Vec<String> {
    let mut candidates = vec!["all".to_string()];
    for member in agent_members {
        push_mention_candidate(&mut candidates, &member.principal_id);
        if let Some(display_name) = &member.display_name {
            push_mention_candidate(&mut candidates, display_name);
        }
        if let Some(policy) = policies_by_agent.get(&member.principal_id) {
            for alias in &policy.mention_aliases {
                push_mention_candidate(&mut candidates, alias);
            }
        }
    }
    candidates
}

fn push_mention_candidate(candidates: &mut Vec<String>, value: &str) {
    let candidate = value.trim().trim_start_matches('@').trim();
    if !candidate.is_empty() && !candidates.iter().any(|existing| existing == candidate) {
        candidates.push(candidate.to_string());
    }
}

/// Extract attachment metadata from the conversation event so the executor
/// can stage incoming files into the agent's workspace. Messages produced by
/// `share_file` (or human uploads) carry `attachment_id` + `filename` +
/// `mime_type` + `download_path` + `size_bytes` on the event — forward them
/// verbatim as an `attachments` array. Returns an empty object when there's
/// nothing to stage.
fn build_command_metadata(event: &ConversationEventRow) -> serde_json::Value {
    if event
        .metadata
        .get("attachment_id")
        .and_then(|v| v.as_str())
        .is_none()
    {
        return serde_json::json!({});
    }
    let pick = |key: &str| {
        event
            .metadata
            .get(key)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };
    serde_json::json!({
        "attachments": [{
            "attachment_id": pick("attachment_id"),
            "filename": pick("filename"),
            "mime_type": pick("mime_type"),
            "download_path": pick("download_path"),
            "size_bytes": pick("size_bytes"),
        }]
    })
}

/// Build a prompt string matching the `[choruz-incoming]` format from CLAUDE.md.
///
/// This format tells the agent WHO sent the message, WHICH group it's in,
/// and the conversation ID — so it knows where to reply via outbox. When the
/// receiving agent has open (non-`done`) tasks assigned to it in this
/// conversation, their keys/titles/statuses are appended as a
/// `your_tasks:[…]` field so the agent can `task_update` against existing
/// cards instead of fabricating keys or creating duplicates.
fn build_prompt(
    event: &ConversationEventRow,
    _member: &ConversationMember,
    sender_name: &str,
    conversation_name: &str,
    assignee_roster: &[AssigneeRosterEntry],
    assigned_tasks: &[AssignedTaskHint],
) -> String {
    let conv_short = if event.conversation_id.len() > 13 {
        &event.conversation_id[..13]
    } else {
        &event.conversation_id
    };
    let content = event.content.as_deref().unwrap_or("");
    let roster = format_assignee_roster(assignee_roster);
    let your_tasks_suffix = format_your_tasks_suffix(assigned_tasks);
    // Thread context:
    // when the routed event is a THREADED reply (reply_event_id set AND
    // metadata.thread == true — legacy quote-replies don't qualify), the
    // envelope carries `thread:<root_event_id>` so the agent can reply
    // into the same thread via the send command's `thread` param. The
    // write paths canonicalize reply_event_id to the root, so it passes
    // through verbatim.
    let thread_suffix = match &event.reply_event_id {
        Some(root) if choruz_store::ThreadFlags::from_metadata(&event.metadata).is_thread_reply => {
            format!(" thread:{root}")
        }
        _ => String::new(),
    };
    if conversation_name == "[DM]" {
        // Direct chat: no group field, agent should reply via terminal output
        format!(
            "[choruz-incoming] from:@{sender_name} direct-chat conv:{conv_short}{thread_suffix} roster:{roster}{your_tasks_suffix} | {content}"
        )
    } else {
        format!(
            "[choruz-incoming] from:@{sender_name} group:{conversation_name} conv:{conv_short}{thread_suffix} roster:{roster}{your_tasks_suffix} | {content}"
        )
    }
}

fn format_your_tasks_suffix(assigned_tasks: &[AssignedTaskHint]) -> String {
    // Empty assignments → omit the field entirely so DMs and non-board
    // channels keep the existing envelope shape verbatim.
    if assigned_tasks.is_empty() {
        return String::new();
    }
    // Cap the list defensively. The query already filters out `done`, but if
    // a single agent has dozens of open cards the envelope shouldn't balloon.
    const MAX_HINTS: usize = 20;
    let values = assigned_tasks
        .iter()
        .take(MAX_HINTS)
        .map(|hint| {
            serde_json::json!({
                "task_key": hint.task_key,
                "title": sanitize_assigned_task_title(&hint.title),
                "status": hint.status.as_str(),
            })
        })
        .collect::<Vec<_>>();
    match serde_json::to_string(&values) {
        Ok(payload) => format!(" your_tasks:{payload}"),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "assigned task hints failed to serialize; omitting your_tasks hint"
            );
            String::new()
        }
    }
}

fn sanitize_assigned_task_title(title: &str) -> String {
    const MAX_TITLE_CHARS: usize = 120;
    title
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '|')
        .take(MAX_TITLE_CHARS)
        .collect()
}

fn format_assignee_roster(roster: &[AssigneeRosterEntry]) -> String {
    let values = roster
        .iter()
        .map(|entry| {
            let mut value = serde_json::json!({
                "id": entry.principal_id,
                "name": sanitize_roster_display_name(&entry.display_name),
                "type": entry.principal_type,
            });
            if let Some(runtime_host_name) = &entry.runtime_host_name {
                value["host"] =
                    serde_json::Value::String(sanitize_roster_display_name(runtime_host_name));
            }
            value
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&values).unwrap_or_else(|error| {
        tracing::warn!(
            error = %error,
            "assignee roster failed to serialize; falling back to empty roster"
        );
        "[]".to_string()
    })
}

fn sanitize_roster_display_name(name: &str) -> String {
    const MAX_ROSTER_DISPLAY_NAME_CHARS: usize = 120;
    name.chars()
        .filter(|ch| !ch.is_control() && *ch != '|')
        .take(MAX_ROSTER_DISPLAY_NAME_CHARS)
        .collect()
}

// ---------------------------------------------------------------------------
// Router loop — consumes outbox events from the CDC channel
// ---------------------------------------------------------------------------

/// Configuration for the router loop.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Maximum consecutive errors before the loop pauses.
    pub max_consecutive_errors: u32,
    /// Pause duration after max consecutive errors.
    pub error_pause: std::time::Duration,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            max_consecutive_errors: 10,
            error_pause: std::time::Duration::from_secs(5),
        }
    }
}

/// Run the router loop, consuming from the CDC channel and routing events.
///
/// This function blocks until the receiver is closed or the cancellation
/// token is triggered.
pub async fn run_router_loop<M, S>(
    mut rx: mpsc::Receiver<OutboxRow>,
    store: EventStore,
    members: M,
    sink: S,
    _config: RouterConfig,
) where
    M: MemberProvider,
    S: DecisionSink,
{
    tracing::info!("Router loop started");

    while let Some(outbox_entry) = rx.recv().await {
        let outbox_id = outbox_entry.id;

        // C1: Parse the outbox payload into a ConversationEventRow
        let event = match parse_outbox_event(&store, &outbox_entry).await {
            Ok(Some(event)) => event,
            Ok(None) => {
                tracing::warn!(
                    outbox_id,
                    "outbox entry references non-existent event, skipping"
                );
                // Mark as published even if the event doesn't exist, but
                // preserve a diagnosable dead-letter record first. Re-polling
                // cannot fix an orphaned CDC row.
                if let Err(e) = dead_letter_outbox_entry(
                    &store,
                    &outbox_entry,
                    "outbox entry references non-existent conversation event",
                )
                .await
                {
                    tracing::warn!(outbox_id, error = %e, "dead-letter orphaned outbox entry failed");
                }
                continue;
            }
            Err(e) => {
                tracing::error!(
                    outbox_id,
                    error = %e,
                    "failed to parse outbox event (will retry on next poll)"
                );
                if should_dead_letter_outbox(outbox_entry.attempt_count) {
                    let error = format!("failed to parse outbox event: {e}");
                    if let Err(dead_letter_error) =
                        dead_letter_outbox_entry(&store, &outbox_entry, &error).await
                    {
                        tracing::error!(
                            outbox_id,
                            error = %dead_letter_error,
                            "failed to dead-letter malformed outbox entry"
                        );
                    }
                }
                // If not dead-lettered yet, leave unpublished so the entry
                // retries after its claim lease expires.
                continue;
            }
        };

        // Pull trace_id from event metadata for log correlation — set by
        // the FE through `x-trace-id` header + persisted by the gateway's
        // send_message handler.
        let loop_trace_id: String = event
            .metadata
            .get("trace_id")
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string();

        // Route the event (name resolution is handled by MemberProvider)
        match route_event(&event, &members, &sink).await {
            Ok(outcomes) => {
                let triggers = outcomes
                    .iter()
                    .filter(|o| **o == RouteOutcome::Triggered)
                    .count();
                tracing::info!(
                    event = "router_event_processed",
                    trace_id = %loop_trace_id,
                    event_id = %event.event_id,
                    message_id = %event.event_id,
                    conversation_id = %event.conversation_id,
                    sender_id = %event.sender_id,
                    total = outcomes.len(),
                    triggers,
                    "routed event"
                );

                // Mark the outbox entry as published ONLY after successful
                // routing.  This ensures events are never lost even if the
                // Router crashes mid-processing (P0-2 fix).
                let node_id = outbox_entry.claimed_by.as_deref().unwrap_or("unknown");
                if let Err(e) = store.mark_published(&[outbox_id], node_id).await {
                    tracing::error!(
                        event = "router_mark_published_failed",
                        trace_id = %loop_trace_id,
                        outbox_id,
                        event_id = %event.event_id,
                        message_id = %event.event_id,
                        conversation_id = %event.conversation_id,
                        sender_id = %event.sender_id,
                        error = %e,
                        "failed to mark outbox entry as published"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    event = "router_route_failed",
                    trace_id = %loop_trace_id,
                    event_id = %event.event_id,
                    message_id = %event.event_id,
                    conversation_id = %event.conversation_id,
                    sender_id = %event.sender_id,
                    error = %e,
                    "failed to route event (will retry on next poll)"
                );
                if should_dead_letter_outbox(outbox_entry.attempt_count) {
                    let error = format!("failed to route event: {e}");
                    if let Err(dead_letter_error) =
                        dead_letter_outbox_entry(&store, &outbox_entry, &error).await
                    {
                        tracing::error!(
                            event = "router_dead_letter_failed",
                            trace_id = %loop_trace_id,
                            outbox_id,
                            event_id = %event.event_id,
                            message_id = %event.event_id,
                            conversation_id = %event.conversation_id,
                            sender_id = %event.sender_id,
                            error = %dead_letter_error,
                            "failed to dead-letter repeatedly failing route"
                        );
                    }
                }
                // If not dead-lettered yet, leave unpublished so the entry
                // retries after its claim lease expires.
            }
        }
    }

    tracing::info!("Router loop stopped (channel closed)");
}

fn should_dead_letter_outbox(attempt_count: i32) -> bool {
    attempt_count >= OUTBOX_DEAD_LETTER_AFTER_ATTEMPTS
}

async fn dead_letter_outbox_entry(
    store: &EventStore,
    outbox: &OutboxRow,
    error: &str,
) -> RouterResult<()> {
    let Some(claimed_by) = outbox.claimed_by.as_deref() else {
        tracing::warn!(
            outbox_id = outbox.id,
            "refusing to dead-letter outbox row without a claim owner"
        );
        return Ok(());
    };

    let mut client = store.connect().await?;
    let tx = client
        .transaction()
        .await
        .map_err(|e| RouterError::Internal(format!("begin dead-letter tx: {e}")))?;
    let source_id = outbox.id.to_string();
    let payload = serde_json::json!({
        "aggregate_type": outbox.aggregate_type.clone(),
        "aggregate_id": outbox.aggregate_id.clone(),
        "event_type": outbox.event_type.clone(),
        "payload": outbox.payload.clone(),
        "claimed_by": outbox.claimed_by.clone(),
        "claimed_at": outbox.claimed_at.clone(),
        "claim_deadline": outbox.claim_deadline.clone(),
    });

    let still_owned = tx
        .query_opt(
            "UPDATE event_outbox
             SET published = TRUE
             WHERE id = $1
               AND published = FALSE
               AND claimed_by = $2
               AND claimed_at IS NOT DISTINCT FROM $3
             RETURNING id",
            &[&outbox.id, &claimed_by, &outbox.claimed_at],
        )
        .await
        .map_err(|e| RouterError::Internal(format!("publish dead-lettered outbox: {e}")))?;
    if still_owned.is_none() {
        tx.rollback()
            .await
            .map_err(|e| RouterError::Internal(format!("rollback stale dead-letter tx: {e}")))?;
        tracing::warn!(
            outbox_id = outbox.id,
            claimed_by,
            "skipped dead-letter because outbox claim is no longer owned by this worker"
        );
        return Ok(());
    }

    tx.execute(
        "INSERT INTO dead_letters (source_type, source_id, payload, error, attempt_count)
             VALUES ('event_outbox', $1, $2, $3, $4)",
        &[&source_id, &payload, &error, &outbox.attempt_count],
    )
    .await
    .map_err(|e| RouterError::Internal(format!("insert dead letter: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| RouterError::Internal(format!("commit dead-letter tx: {e}")))?;
    Ok(())
}

/// Parse an outbox row back into a ConversationEventRow by looking it up
/// in the event store.
async fn parse_outbox_event(
    store: &EventStore,
    outbox: &OutboxRow,
) -> RouterResult<Option<ConversationEventRow>> {
    // The outbox payload contains the event data, and the aggregate_id is the
    // conversation_id. We can look up the event by message_id from the payload.
    let message_id = outbox
        .payload
        .get("message_id")
        .and_then(|v| v.as_str())
        .or_else(|| outbox.payload.get("event_id").and_then(|v| v.as_str()));

    match message_id {
        Some(mid) => {
            let event = store.get_event_by_message_id(mid).await?;
            Ok(event)
        }
        None => Err(RouterError::Deserialization(
            "outbox payload missing message_id/event_id".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
