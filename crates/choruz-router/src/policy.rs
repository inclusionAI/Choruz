//! Policy evaluation logic for the router.
//!
//! Implements the trigger-plane logic: given an event, a conversation member,
//! and their policy, determine whether the agent should be triggered.

use crate::models::{AgentPolicy, AutoMode, ConversationMember, TriggerResult};
use choruz_store::ConversationEventRow;

/// Evaluate whether an agent should be triggered for a given event.
///
/// This checks:
/// - `AllMessages` policy: always trigger
/// - `MentionedOnly` policy: trigger if the agent's display name, principal_id,
///   or any alias appears as `@mention` in the content, or if `@all` is present,
///   or if metadata contains `turn_for` / `request_review_by` matching the agent.
/// - `Manual` policy: never trigger
pub fn evaluate_trigger(
    policy: &AgentPolicy,
    event: &ConversationEventRow,
    member: &ConversationMember,
) -> TriggerResult {
    evaluate_trigger_with_candidates(policy, event, member, &[])
}

pub fn evaluate_trigger_with_candidates(
    policy: &AgentPolicy,
    event: &ConversationEventRow,
    member: &ConversationMember,
    mention_candidates: &[String],
) -> TriggerResult {
    match policy.auto_mode {
        AutoMode::AllMessages => TriggerResult {
            should_trigger: true,
            reason: "all_messages policy".into(),
        },
        AutoMode::Manual => TriggerResult {
            should_trigger: false,
            reason: "manual mode, no auto trigger".into(),
        },
        AutoMode::MentionedOnly => evaluate_mention(policy, event, member, mention_candidates),
    }
}

/// Check if the agent is mentioned in the event content or metadata.
fn evaluate_mention(
    policy: &AgentPolicy,
    event: &ConversationEventRow,
    member: &ConversationMember,
    mention_candidates: &[String],
) -> TriggerResult {
    let content = event.content.as_deref().unwrap_or("");

    // Check @all
    if contains_mention(content, "all", mention_candidates) {
        return TriggerResult {
            should_trigger: true,
            reason: "mentioned via @all".into(),
        };
    }

    // Check @display_name
    if let Some(ref name) = member.display_name
        && contains_mention(content, name, mention_candidates)
    {
        return TriggerResult {
            should_trigger: true,
            reason: format!("mentioned via @{name}"),
        };
    }

    // Check @principal_id
    if contains_mention(content, &member.principal_id, mention_candidates) {
        return TriggerResult {
            should_trigger: true,
            reason: format!("mentioned via @{}", member.principal_id),
        };
    }

    // Check aliases
    for alias in &policy.mention_aliases {
        if contains_mention(content, alias, mention_candidates) {
            return TriggerResult {
                should_trigger: true,
                reason: format!("mentioned via alias @{alias}"),
            };
        }
    }

    // Check metadata: turn_for
    if let Some(turn_for) = event.metadata.get("turn_for").and_then(|v| v.as_str())
        && turn_for == member.principal_id
    {
        return TriggerResult {
            should_trigger: true,
            reason: "metadata turn_for matches agent".into(),
        };
    }

    // Check metadata: request_review_by
    if let Some(reviewer) = event
        .metadata
        .get("request_review_by")
        .and_then(|v| v.as_str())
        && reviewer == member.principal_id
    {
        return TriggerResult {
            should_trigger: true,
            reason: "metadata request_review_by matches agent".into(),
        };
    }

    TriggerResult {
        should_trigger: false,
        reason: "not mentioned (mentioned_only policy)".into(),
    }
}

fn contains_mention(content: &str, mention: &str, mention_candidates: &[String]) -> bool {
    if mention.is_empty() {
        return false;
    }

    let pattern = format!("@{mention}");
    content.match_indices(&pattern).any(|(start, _)| {
        mention_matches_at(content, mention, start)
            && !mention_candidates
                .iter()
                .filter(|candidate| candidate.len() > mention.len())
                .any(|candidate| mention_matches_at(content, candidate, start))
    })
}

fn mention_matches_at(content: &str, mention: &str, start: usize) -> bool {
    if mention.is_empty() {
        return false;
    }

    let pattern = format!("@{mention}");
    if !content[start..].starts_with(&pattern) {
        return false;
    }

    let before = content[..start].chars().next_back();
    let after = content[start + pattern.len()..].chars().next();

    before.is_none_or(|ch| !is_mention_char(ch)) && after.is_none_or(|ch| !is_mention_char(ch))
}

fn is_mention_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_event(content: &str, metadata: serde_json::Value) -> ConversationEventRow {
        ConversationEventRow {
            conversation_id: "conv-1".into(),
            seq: 1,
            event_id: "evt-1".into(),
            event_type: "message".into(),
            sender_id: "user-1".into(),
            content: Some(content.into()),
            content_type: "text/plain".into(),
            metadata,
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            created_at: Utc::now(),
        }
    }

    fn make_member(display_name: &str) -> ConversationMember {
        ConversationMember {
            conversation_id: "conv-1".into(),
            principal_id: "agent-backend".into(),
            principal_type: "agent".into(),
            display_name: Some(display_name.into()),
            joined_at: Utc::now(),
            left_at: None,
        }
    }

    fn make_policy(mode: AutoMode, aliases: Vec<String>) -> AgentPolicy {
        AgentPolicy {
            agent_id: "agent-backend".into(),
            conversation_id: "conv-1".into(),
            auto_mode: mode,
            mention_aliases: aliases,
        }
    }

    #[test]
    fn all_messages_always_triggers() {
        let policy = make_policy(AutoMode::AllMessages, vec![]);
        let event = make_event("hello", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(result.should_trigger);
        assert_eq!(result.reason, "all_messages policy");
    }

    #[test]
    fn manual_never_triggers() {
        let policy = make_policy(AutoMode::Manual, vec![]);
        let event = make_event("@backend-dev hello", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(!result.should_trigger);
        assert_eq!(result.reason, "manual mode, no auto trigger");
    }

    #[test]
    fn mentioned_only_at_display_name() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event("@backend-dev please review", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(result.should_trigger);
        assert!(result.reason.contains("@backend-dev"));
    }

    #[test]
    fn mentioned_only_at_all() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event("@all status update?", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(result.should_trigger);
        assert!(result.reason.contains("@all"));
    }

    #[test]
    fn mentioned_only_at_principal_id() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event("@agent-backend help", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(result.should_trigger);
        assert!(result.reason.contains("@agent-backend"));
    }

    #[test]
    fn mentioned_only_via_alias() {
        let policy = make_policy(AutoMode::MentionedOnly, vec!["be".into(), "dev".into()]);
        let event = make_event("@be fix this bug", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(result.should_trigger);
        assert!(result.reason.contains("alias @be"));
    }

    #[test]
    fn mentioned_only_not_mentioned() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event("@frontend-dev please review", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(!result.should_trigger);
        assert!(result.reason.contains("not mentioned"));
    }

    #[test]
    fn mentioned_only_does_not_match_display_name_prefix() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event("@dev2 hi", serde_json::json!({}));
        let member = make_member("dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(!result.should_trigger);
    }

    #[test]
    fn mentioned_only_does_not_match_alias_prefix() {
        let policy = make_policy(AutoMode::MentionedOnly, vec!["dev".into()]);
        let event = make_event("@dev2 hi", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(!result.should_trigger);
    }

    #[test]
    fn mentioned_only_all_requires_exact_token() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event("@alliance status", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(!result.should_trigger);
    }

    #[test]
    fn mentioned_only_accepts_punctuation_after_mention() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event("@backend-dev, please review", serde_json::json!({}));
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(result.should_trigger);
    }

    #[test]
    fn mentioned_only_via_metadata_turn_for() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event(
            "here's the code",
            serde_json::json!({"turn_for": "agent-backend"}),
        );
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(result.should_trigger);
        assert!(result.reason.contains("turn_for"));
    }

    #[test]
    fn mentioned_only_via_metadata_request_review_by() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event(
            "code ready",
            serde_json::json!({"request_review_by": "agent-backend"}),
        );
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(result.should_trigger);
        assert!(result.reason.contains("request_review_by"));
    }

    #[test]
    fn mentioned_only_none_content() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = ConversationEventRow {
            conversation_id: "conv-1".into(),
            seq: 1,
            event_id: "evt-1".into(),
            event_type: "system".into(),
            sender_id: "system".into(),
            content: None,
            content_type: "text/plain".into(),
            metadata: serde_json::json!({}),
            client_msg_id: None,
            turn_id: None,
            reply_event_id: None,
            created_at: Utc::now(),
        };
        let member = make_member("backend-dev");

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(!result.should_trigger);
    }

    #[test]
    fn mentioned_only_no_display_name() {
        let policy = make_policy(AutoMode::MentionedOnly, vec![]);
        let event = make_event("@agent-backend hello", serde_json::json!({}));
        let member = ConversationMember {
            conversation_id: "conv-1".into(),
            principal_id: "agent-backend".into(),
            principal_type: "agent".into(),
            display_name: None,
            joined_at: Utc::now(),
            left_at: None,
        };

        let result = evaluate_trigger(&policy, &event, &member);
        assert!(result.should_trigger);
        assert!(result.reason.contains("@agent-backend"));
    }
}
