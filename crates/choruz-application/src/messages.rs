use choruz_domain::Message;
use serde_json::json;

use crate::ChatApp;

impl ChatApp {
    /// Announce a message that `DbService` has already persisted: push a
    /// `message.created` event to conversation members so the in-process
    /// consumers (webhooks, SSE) see it.
    ///
    /// The message body is deliberately not retained. Postgres is the source of
    /// truth and every read path queries it, so keeping a second copy in memory
    /// only cost memory that grew with uptime. What is retained is the id, in a
    /// bounded window, so that a client retry — which makes
    /// `DbService::send_message` return the existing row and land here a second
    /// time — does not produce a duplicate event.
    pub fn inject_message_with_event(&self, message: Message) {
        let mut state = self.inner.write().expect("lock poisoned");
        let conv_id = message.conversation_id.clone();

        if !state.recent_message_ids.insert(&message.id) {
            return; // Already announced
        }
        state.messages_injected += 1;

        // Push event to conversation members
        let recipients: Vec<String> = state
            .conversations
            .get(&conv_id)
            .map(|c| c.members.keys().cloned().collect())
            .unwrap_or_default();
        self.push_event(
            &mut state,
            &recipients,
            "message.created",
            json!({
                "conversation_id": conv_id,
                "message_id": message.id,
                "server_seq": message.server_seq
            }),
        );
    }
}

#[cfg(test)]
mod messages_tests {
    use super::*;
    use choruz_common::now;
    use choruz_domain::{Conversation, ConversationMember, ConversationType};
    use std::collections::BTreeMap;

    fn mk_msg(id: &str, conv: &str, seq: u64) -> Message {
        Message {
            id: id.into(),
            workspace_id: "ws".into(),
            conversation_id: conv.into(),
            sender_id: "alice".into(),
            content: format!("body-{id}"),
            content_type: "text".into(),
            metadata: json!({}),
            edited_at: None,
            edited_by: None,
            server_seq: seq,
            idempotency_key: format!("k-{id}"),
            created_at: now(),
        }
    }

    fn mk_conv(id: &str, members: &[&str]) -> Conversation {
        let mut m: BTreeMap<String, ConversationMember> = BTreeMap::new();
        for principal_id in members {
            m.insert(
                (*principal_id).into(),
                ConversationMember {
                    principal_id: (*principal_id).into(),
                    joined_at: now(),
                },
            );
        }
        Conversation {
            id: id.into(),
            workspace_id: "ws".into(),
            conversation_type: ConversationType::Group,
            name: Some(format!("conv-{id}")),
            description: None,
            avatar_url: None,
            creator_id: members.first().copied().unwrap_or("alice").into(),
            created_at: now(),
            updated_at: now(),
            members: m,
        }
    }

    fn app_with_conversation(members: &[&str]) -> ChatApp {
        let app = ChatApp::new();
        let mut state = app.inner.write().unwrap();
        state
            .conversations
            .insert("c1".into(), mk_conv("c1", members));
        drop(state);
        app
    }

    #[test]
    fn inject_message_with_event_pushes_event_to_conversation_members() {
        let app = app_with_conversation(&["alice", "bob"]);
        app.inject_message_with_event(mk_msg("m1", "c1", 1));

        let state = app.inner.read().unwrap();
        assert_eq!(state.events.get("alice").map(|v| v.len()), Some(1));
        assert_eq!(state.events.get("bob").map(|v| v.len()), Some(1));
        let evt = &state.events["alice"][0];
        assert_eq!(evt.event_type, "message.created");
        assert_eq!(evt.payload["message_id"], "m1");
    }

    #[test]
    fn inject_message_with_event_suppresses_a_repeat_of_the_same_id() {
        let app = app_with_conversation(&["alice"]);
        app.inject_message_with_event(mk_msg("m1", "c1", 1));
        app.inject_message_with_event(mk_msg("m1", "c1", 1));

        let state = app.inner.read().unwrap();
        assert_eq!(state.events.get("alice").unwrap().len(), 1);
        assert_eq!(state.messages_injected, 1);
    }

    #[test]
    fn inject_message_with_event_counts_each_message_for_metrics() {
        let app = app_with_conversation(&["alice"]);
        for seq in 1..=3 {
            app.inject_message_with_event(mk_msg(&format!("m{seq}"), "c1", seq));
        }

        // This counter is what `choruz_messages_total` reports now that the
        // message bodies it used to be derived from are gone.
        assert_eq!(app.inner.read().unwrap().messages_injected, 3);
        assert_eq!(app.metrics_snapshot().messages_total, 3);
    }

    #[test]
    fn dedupe_window_is_bounded_and_forgets_the_oldest_id_first() {
        // No conversation, so `push_event` has no recipients and this stays
        // cheap: the point is the dedupe window, not the fanout.
        let app = ChatApp::new();
        for seq in 0..=10_000u64 {
            app.inject_message_with_event(mk_msg(&format!("m{seq}"), "c-absent", seq));
        }
        assert_eq!(app.inner.read().unwrap().messages_injected, 10_001);

        // 10 001 distinct ids against a 10 000-entry window: the first one has
        // been evicted, so it is no longer recognised as a duplicate …
        app.inject_message_with_event(mk_msg("m0", "c-absent", 0));
        assert_eq!(app.inner.read().unwrap().messages_injected, 10_002);
        // … while the most recent one still is.
        app.inject_message_with_event(mk_msg("m10000", "c-absent", 10_000));
        assert_eq!(app.inner.read().unwrap().messages_injected, 10_002);
    }
}
