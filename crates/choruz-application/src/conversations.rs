use std::collections::BTreeMap;

use choruz_common::{AppError, AppResult, new_id, now};
use choruz_domain::{Conversation, ConversationMember, ConversationType, PrincipalType};
use serde_json::json;

use crate::{
    AddGroupMembersRequest, ChatApp, CreateDirectConversationRequest, CreateGroupRequest,
    UpdateGroupRequest, direct_key,
};

impl ChatApp {
    /// Inject a conversation directly into memory, bypassing validation.
    /// If the conversation already exists, merge members (add missing ones).
    pub fn inject_conversation(&self, conversation: Conversation) {
        let mut state = self.inner.write().expect("lock poisoned");
        if let Some(existing) = state.conversations.get_mut(&conversation.id) {
            // Merge members — add any that are missing
            for (mid, member) in &conversation.members {
                if !existing.members.contains_key(mid) {
                    existing.members.insert(mid.clone(), member.clone());
                }
            }
        } else {
            state
                .conversations
                .insert(conversation.id.clone(), conversation);
        }
    }

    /// Check if a conversation exists in memory.
    pub fn has_conversation(&self, conversation_id: &str) -> bool {
        let state = self.inner.read().expect("lock poisoned");
        state.conversations.contains_key(conversation_id)
    }

    /// List all conversations without principal-based filtering.
    /// Used internally for DB backfill on startup.
    pub fn list_all_conversations_internal(&self) -> Vec<Conversation> {
        let state = self.inner.read().expect("lock poisoned");
        state.conversations.values().cloned().collect()
    }

    /// Remove a conversation and all associated data (messages, events, members).
    /// Only a human member can delete conversations they belong to.
    pub fn delete_conversation(&self, actor_id: &str, conversation_id: &str) -> AppResult<()> {
        let mut state = self.inner.write().expect("lock poisoned");
        let actor = self.require_active_principal(&state, actor_id)?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "only the signed-in person can delete conversations".into(),
            ));
        }
        let conv = state
            .conversations
            .get(conversation_id)
            .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;
        if !conv.members.contains_key(actor_id) {
            return Err(AppError::Forbidden("not a conversation member".into()));
        }

        // Remove conversation, events, policies
        state.conversations.remove(conversation_id);
        state.next_server_seq.remove(conversation_id);
        // Remove events for members of this conversation

        self.record_audit(
            &mut state,
            &actor,
            "conversation.deleted",
            "conversation",
            conversation_id,
            json!({}),
        );
        Ok(())
    }

    /// Batch delete conversations without rate limit for human control-plane operations.
    pub fn delete_conversations_batch(
        &self,
        actor_id: &str,
        conversation_ids: &[String],
    ) -> (u64, u64) {
        let mut deleted = 0u64;
        let mut failed = 0u64;
        for cid in conversation_ids {
            match self.delete_conversation(actor_id, cid) {
                Ok(()) => deleted += 1,
                Err(_) => failed += 1,
            }
        }
        (deleted, failed)
    }

    pub fn create_direct_conversation(
        &self,
        request: CreateDirectConversationRequest,
    ) -> AppResult<Conversation> {
        let mut state = self.inner.write().expect("lock poisoned");
        self.check_rate_limit(&mut state, &request.actor_id)?;

        if request.actor_id == request.peer_principal_id {
            return Err(AppError::Validation(
                "cannot create a direct conversation with yourself".into(),
            ));
        }

        let actor = self.require_active_principal(&state, &request.actor_id)?;
        let peer = self.require_active_principal(&state, &request.peer_principal_id)?;
        // When an explicit workspace_id is provided, skip the same-workspace check
        // (the human may be in "ws-local" while the agent belongs to a company workspace).
        let ws_id = request
            .workspace_id
            .filter(|w| !w.trim().is_empty())
            .unwrap_or_else(|| {
                // No override — enforce same workspace
                // (error is returned inline below if they differ)
                actor.workspace_id.clone()
            });
        if !self.principal_can_access_workspace(&state, &actor, &ws_id)
            || !self.principal_can_access_workspace(&state, &peer, &ws_id)
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let key = direct_key(&ws_id, &actor.id, &peer.id);
        if let Some(conversation_id) = state.direct_index.get(&key) {
            return state
                .conversations
                .get(conversation_id)
                .cloned()
                .ok_or_else(|| AppError::Internal("direct index is inconsistent".into()));
        }

        let timestamp = now();
        let mut members = BTreeMap::from([
            (
                actor.id.clone(),
                ConversationMember {
                    principal_id: actor.id.clone(),
                    joined_at: timestamp,
                },
            ),
            (
                peer.id.clone(),
                ConversationMember {
                    principal_id: peer.id.clone(),
                    joined_at: timestamp,
                },
            ),
        ]);
        if !matches!(actor.principal_type, PrincipalType::Human)
            && !matches!(peer.principal_type, PrincipalType::Human)
        {
            let human = state
                .companies
                .get(&ws_id)
                .and_then(|company| state.principals.get(&company.owner_id))
                .filter(|principal| {
                    matches!(principal.principal_type, PrincipalType::Human)
                        && !principal.disabled
                        && principal.deleted_at.is_none()
                        && self.principal_can_access_workspace(&state, principal, &ws_id)
                })
                .or_else(|| {
                    let mut humans = state.principals.values().filter(|principal| {
                        matches!(principal.principal_type, PrincipalType::Human)
                            && !principal.disabled
                            && principal.deleted_at.is_none()
                            && self.principal_can_access_workspace(&state, principal, &ws_id)
                    });
                    let human = humans.next();
                    if humans.next().is_none() { human } else { None }
                })
                .ok_or_else(|| {
                    AppError::Conflict(
                        "exactly one active human owner is required for agent conversations".into(),
                    )
                })?;
            members.insert(
                human.id.clone(),
                ConversationMember {
                    principal_id: human.id.clone(),
                    joined_at: timestamp,
                },
            );
        }
        let conversation = Conversation {
            id: new_id(),
            workspace_id: ws_id,
            conversation_type: ConversationType::Direct,
            name: None,
            description: None,
            avatar_url: None,
            creator_id: actor.id.clone(),
            created_at: timestamp,
            updated_at: timestamp,
            members,
        };

        state.direct_index.insert(key, conversation.id.clone());
        state
            .conversations
            .insert(conversation.id.clone(), conversation.clone());

        self.record_audit(
            &mut state,
            &actor,
            "conversation.direct_created",
            "conversation",
            &conversation.id,
            json!({"peer_principal_id": peer.id}),
        );
        self.push_event(
            &mut state,
            &[actor.id.clone(), peer.id.clone()],
            "conversation.created",
            json!({"conversation_id": conversation.id}),
        );

        Ok(conversation)
    }

    pub fn create_group(&self, request: CreateGroupRequest) -> AppResult<Conversation> {
        if request.name.trim().is_empty() {
            return Err(AppError::Validation("group name is required".into()));
        }

        let mut state = self.inner.write().expect("lock poisoned");
        self.check_rate_limit(&mut state, &request.actor_id)?;

        let actor = self.require_active_principal(&state, &request.actor_id)?;
        let ws_id = request
            .workspace_id
            .filter(|w| !w.trim().is_empty())
            .unwrap_or_else(|| actor.workspace_id.clone());
        if !self.principal_can_access_workspace(&state, &actor, &ws_id) {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }
        let timestamp = now();
        let conversation_id = new_id();
        let mut members = BTreeMap::new();
        members.insert(
            actor.id.clone(),
            ConversationMember {
                principal_id: actor.id.clone(),
                joined_at: timestamp,
            },
        );

        let mut has_human = matches!(actor.principal_type, PrincipalType::Human);
        for member_id in request.member_ids {
            if member_id == actor.id || members.contains_key(&member_id) {
                continue;
            }
            let member = self.require_active_principal(&state, &member_id)?;
            if !self.principal_can_access_workspace(&state, &member, &ws_id) {
                return Err(AppError::Forbidden("cross-workspace access denied".into()));
            }
            has_human |= matches!(member.principal_type, PrincipalType::Human);
            members.insert(
                member.id.clone(),
                ConversationMember {
                    principal_id: member.id.clone(),
                    joined_at: timestamp,
                },
            );
        }

        if !has_human {
            let human = state
                .companies
                .get(&ws_id)
                .and_then(|company| state.principals.get(&company.owner_id))
                .filter(|principal| {
                    matches!(principal.principal_type, PrincipalType::Human)
                        && !principal.disabled
                        && principal.deleted_at.is_none()
                        && self.principal_can_access_workspace(&state, principal, &ws_id)
                })
                .or_else(|| {
                    let mut humans = state.principals.values().filter(|principal| {
                        matches!(principal.principal_type, PrincipalType::Human)
                            && !principal.disabled
                            && principal.deleted_at.is_none()
                            && self.principal_can_access_workspace(&state, principal, &ws_id)
                    });
                    let human = humans.next();
                    if humans.next().is_none() { human } else { None }
                })
                .ok_or_else(|| {
                    AppError::Conflict(
                        "exactly one active human owner is required for agent conversations".into(),
                    )
                })?;
            members.insert(
                human.id.clone(),
                ConversationMember {
                    principal_id: human.id.clone(),
                    joined_at: timestamp,
                },
            );
        }

        let conversation = Conversation {
            id: conversation_id.clone(),
            workspace_id: ws_id,
            conversation_type: ConversationType::Group,
            name: Some(request.name.clone()),
            description: request.description,
            avatar_url: request.avatar_url,
            creator_id: actor.id.clone(),
            created_at: timestamp,
            updated_at: timestamp,
            members,
        };

        state
            .conversations
            .insert(conversation.id.clone(), conversation.clone());

        self.record_audit(
            &mut state,
            &actor,
            "group.created",
            "conversation",
            &conversation.id,
            json!({"name": request.name}),
        );

        let recipients: Vec<String> = conversation.members.keys().cloned().collect();
        self.push_event(
            &mut state,
            &recipients,
            "group.created",
            json!({"conversation_id": conversation.id, "name": conversation.name}),
        );
        self.announce_system_message(&mut state, &conversation.id)?;

        Ok(conversation)
    }

    pub fn add_group_members(
        &self,
        conversation_id: &str,
        request: AddGroupMembersRequest,
    ) -> AppResult<Conversation> {
        let mut state = self.inner.write().expect("lock poisoned");
        self.check_rate_limit(&mut state, &request.actor_id)?;

        let actor = self.require_active_principal(&state, &request.actor_id)?;
        let actor_id = actor.id.clone();
        let (conversation_type, conversation_workspace, actor_is_member, existing_ids) = {
            let conversation = state
                .conversations
                .get(conversation_id)
                .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;
            (
                conversation.conversation_type.clone(),
                conversation.workspace_id.clone(),
                conversation.members.contains_key(&actor_id),
                conversation.members.keys().cloned().collect::<Vec<_>>(),
            )
        };

        if !matches!(conversation_type, ConversationType::Group) {
            return Err(AppError::Validation("conversation is not a group".into()));
        }

        // Any active group member can invite new members (Slack/Discord semantics).
        if !actor_is_member {
            return Err(AppError::Forbidden("not a group member".into()));
        }
        if !self.principal_can_access_workspace(&state, &actor, &conversation_workspace) {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let timestamp = now();
        let mut added_ids = Vec::new();
        for member_id in request.member_ids {
            if existing_ids.contains(&member_id) {
                continue;
            }
            let member = self.require_active_principal(&state, &member_id)?;
            if !self.principal_can_access_workspace(&state, &member, &conversation_workspace) {
                return Err(AppError::Forbidden("cross-workspace access denied".into()));
            }
            let conversation = state
                .conversations
                .get_mut(conversation_id)
                .expect("conversation must exist");
            conversation.members.insert(
                member.id.clone(),
                ConversationMember {
                    principal_id: member.id.clone(),
                    joined_at: timestamp,
                },
            );
            added_ids.push(member.id.clone());
        }

        if added_ids.is_empty() {
            return state
                .conversations
                .get(conversation_id)
                .cloned()
                .ok_or_else(|| AppError::Internal("conversation disappeared".into()));
        }

        let conversation = state
            .conversations
            .get(conversation_id)
            .cloned()
            .ok_or_else(|| AppError::Internal("conversation disappeared".into()))?;

        self.record_audit(
            &mut state,
            &actor,
            "group.members_added",
            "conversation",
            conversation_id,
            json!({"member_ids": added_ids }),
        );

        for member_id in &added_ids {
            self.push_event(
                &mut state,
                std::slice::from_ref(member_id),
                "group.member_added",
                json!({"conversation_id": conversation_id, "member_id": member_id}),
            );
            self.announce_system_message(&mut state, conversation_id)?;
        }

        Ok(conversation)
    }

    pub fn remove_group_member(
        &self,
        conversation_id: &str,
        actor_id: &str,
        target_id: &str,
    ) -> AppResult<Conversation> {
        let mut state = self.inner.write().expect("lock poisoned");
        self.check_rate_limit(&mut state, actor_id)?;

        let actor = self.require_active_principal(&state, actor_id)?;
        // Called for its validation: removing a principal that is gone or
        // deactivated is rejected here, before any membership is touched.
        let target = self.require_active_principal(&state, target_id)?;
        if matches!(target.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "human members cannot be removed from conversations".into(),
            ));
        }

        let (
            conversation_type,
            conversation_workspace,
            actor_is_member,
            target_is_member,
            creator_id,
        ) = {
            let conversation = state
                .conversations
                .get(conversation_id)
                .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;
            (
                conversation.conversation_type.clone(),
                conversation.workspace_id.clone(),
                conversation.members.contains_key(actor_id),
                conversation.members.contains_key(target_id),
                conversation.creator_id.clone(),
            )
        };

        if !matches!(conversation_type, ConversationType::Group) {
            return Err(AppError::Validation("conversation is not a group".into()));
        }
        if !self.principal_can_access_workspace(&state, &actor, &conversation_workspace) {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        if !actor_is_member {
            return Err(AppError::Forbidden("not a group member".into()));
        }
        if !target_is_member {
            return Err(AppError::NotFound("target is not a group member".into()));
        }
        if target_id == creator_id {
            return Err(AppError::Forbidden(
                "cannot remove conversation creator".into(),
            ));
        }

        let conversation = state
            .conversations
            .get_mut(conversation_id)
            .expect("conversation must exist");
        conversation.members.remove(target_id);
        conversation.updated_at = now();
        let conversation = conversation.clone();

        self.record_audit(
            &mut state,
            &actor,
            "group.member_removed",
            "conversation",
            conversation_id,
            json!({"member_id": target_id}),
        );
        self.push_event(
            &mut state,
            &[target_id.to_owned()],
            "group.member_removed",
            json!({"conversation_id": conversation_id, "member_id": target_id}),
        );
        self.announce_system_message(&mut state, conversation_id)?;

        Ok(conversation)
    }

    pub fn update_group(
        &self,
        conversation_id: &str,
        request: UpdateGroupRequest,
    ) -> AppResult<Conversation> {
        let mut state = self.inner.write().expect("lock poisoned");
        self.check_rate_limit(&mut state, &request.actor_id)?;

        let actor = self.require_active_principal(&state, &request.actor_id)?;
        let (conversation_type, conversation_workspace, actor_is_member) = {
            let conversation = state
                .conversations
                .get(conversation_id)
                .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;
            (
                conversation.conversation_type.clone(),
                conversation.workspace_id.clone(),
                conversation.members.contains_key(&actor.id),
            )
        };
        if !matches!(conversation_type, ConversationType::Group) {
            return Err(AppError::Validation("conversation is not a group".into()));
        }
        if !actor_is_member {
            return Err(AppError::Forbidden("not a group member".into()));
        }
        if !self.principal_can_access_workspace(&state, &actor, &conversation_workspace) {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let conversation = state
            .conversations
            .get_mut(conversation_id)
            .expect("conversation must exist");
        if let Some(name) = request.name {
            conversation.name = Some(name);
        }
        if let Some(description) = request.description {
            conversation.description = Some(description);
        }
        if let Some(avatar_url) = request.avatar_url {
            conversation.avatar_url = Some(avatar_url);
        }
        conversation.updated_at = now();
        let conversation = conversation.clone();

        self.record_audit(
            &mut state,
            &actor,
            "group.updated",
            "conversation",
            conversation_id,
            json!({}),
        );

        Ok(conversation)
    }

    /// Move a conversation between workspaces the human actor can access.
    ///
    /// Only the conversation row moves. Its messages keep their original
    /// `workspace_id` — the caller's `UPDATE conversation SET workspace_id`
    /// never touched the message table, so the in-memory rewrite this used to do
    /// only made the mirror disagree with Postgres.
    pub fn migrate_conversation_workspace(
        &self,
        conversation_id: &str,
        new_workspace_id: &str,
        actor_id: &str,
    ) -> AppResult<Conversation> {
        let mut state = self.inner.write().expect("lock poisoned");
        let actor = self.require_active_principal(&state, actor_id)?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "only humans can migrate workspaces".into(),
            ));
        }
        if new_workspace_id.trim().is_empty() {
            return Err(AppError::Validation("workspace_id is required".into()));
        }
        let existing = state
            .conversations
            .get(conversation_id)
            .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;
        if !self.principal_can_access_workspace(&state, &actor, &existing.workspace_id)
            || !self.principal_can_access_workspace(&state, &actor, new_workspace_id)
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let old_direct_key = state
            .direct_index
            .iter()
            .find_map(|(key, indexed_id)| (indexed_id == conversation_id).then(|| key.clone()));

        let conversation = state
            .conversations
            .get_mut(conversation_id)
            .ok_or_else(|| AppError::NotFound("conversation not found".into()))?;

        let old_workspace_id = conversation.workspace_id.clone();
        conversation.workspace_id = new_workspace_id.to_string();
        conversation.updated_at = now();
        let conversation = conversation.clone();

        // Update direct_index if this is a direct conversation
        if matches!(conversation.conversation_type, ConversationType::Direct)
            && let Some(old_key) = old_direct_key
            && state.direct_index.remove(&old_key).is_some()
        {
            let new_key = direct_key(new_workspace_id, &old_key.1, &old_key.2);
            state.direct_index.insert(new_key, conversation.id.clone());
        }

        self.record_audit(
            &mut state,
            &actor,
            "conversation.workspace_migrated",
            "conversation",
            conversation_id,
            json!({
                "old_workspace_id": old_workspace_id,
                "new_workspace_id": new_workspace_id,
            }),
        );

        Ok(conversation)
    }

    pub fn list_conversations(&self, principal_id: &str) -> AppResult<Vec<Conversation>> {
        let state = self.inner.read().expect("lock poisoned");
        let actor = self.require_active_principal(&state, principal_id)?;

        let conversations = state
            .conversations
            .values()
            .filter(|conversation| {
                self.principal_can_access_workspace(&state, &actor, &conversation.workspace_id)
                    && conversation.members.contains_key(principal_id)
            })
            .cloned()
            .collect();

        Ok(conversations)
    }

    pub fn get_conversation(&self, conversation_id: &str) -> AppResult<Conversation> {
        let state = self.inner.read().expect("lock poisoned");
        state
            .conversations
            .get(conversation_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound("conversation not found".into()))
    }
}

#[cfg(test)]
mod conversations_tests {
    use super::*;
    use crate::{
        AddGroupMembersRequest, ChatApp, CreateDirectConversationRequest, CreateGroupRequest,
        UpdateGroupRequest,
    };
    use choruz_domain::{Company, CompanyMember, Principal, PrincipalType};
    use chrono::Utc;

    fn mk_human(id: &str, ws: &str) -> Principal {
        Principal {
            id: id.into(),
            workspace_id: ws.into(),
            principal_type: PrincipalType::Human,
            name: id.into(),
            avatar_url: None,
            scopes: vec!["messages:write".into()],
            secret_hash: None,
            disabled: false,
            deleted_at: None,
            channel_visibility: choruz_domain::ChannelVisibility::Visible,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            user_id: None,
        }
    }

    fn mk_operator(id: &str, ws: &str) -> Principal {
        Principal {
            principal_type: PrincipalType::Human,
            ..mk_human(id, ws)
        }
    }

    fn mk_agent(id: &str, ws: &str) -> Principal {
        Principal {
            principal_type: PrincipalType::Agent,
            ..mk_human(id, ws)
        }
    }

    fn mk_company(id: &str, owner_id: &str) -> Company {
        Company {
            id: id.into(),
            name: format!("company-{id}"),
            slug: id.into(),
            description: None,
            avatar_url: None,
            owner_id: owner_id.into(),
            agents_active: true,
            folder_path: None,
            multi_harness_accounts: false,
            archived_at: None,
            deleted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn mk_deleted_company(id: &str, owner_id: &str) -> Company {
        Company {
            deleted_at: Some(Utc::now()),
            ..mk_company(id, owner_id)
        }
    }

    fn mk_company_member(id: &str) -> CompanyMember {
        CompanyMember {
            principal_id: id.into(),
            joined_at: Utc::now(),
        }
    }

    fn mk_conv(id: &str, ws: &str, ctype: ConversationType, members: &[&str]) -> Conversation {
        let mut m = BTreeMap::new();
        for mid in members {
            m.insert(
                (*mid).into(),
                ConversationMember {
                    principal_id: (*mid).into(),
                    joined_at: Utc::now(),
                },
            );
        }
        Conversation {
            id: id.into(),
            workspace_id: ws.into(),
            conversation_type: ctype,
            name: Some(format!("name-{id}")),
            description: None,
            avatar_url: None,
            creator_id: members.first().map(|id| (**id).into()).unwrap_or_default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            members: m,
        }
    }

    // inject_conversation / has_conversation / list_all ------------------

    #[test]
    fn inject_conversation_adds_then_merges_members_on_re_inject() {
        let app = ChatApp::new();
        app.inject_conversation(mk_conv("c1", "ws", ConversationType::Group, &["alice"]));
        assert!(app.has_conversation("c1"));
        // Re-inject with an additional member.
        app.inject_conversation(mk_conv(
            "c1",
            "ws",
            ConversationType::Group,
            &["alice", "bob"],
        ));
        let convs = app.list_all_conversations_internal();
        assert_eq!(convs.len(), 1);
        assert!(convs[0].members.contains_key("alice"));
        assert!(convs[0].members.contains_key("bob"));
    }

    #[test]
    fn has_conversation_returns_false_for_unknown_id() {
        let app = ChatApp::new();
        assert!(!app.has_conversation("missing"));
    }

    #[test]
    fn list_all_conversations_internal_returns_every_conv_unfiltered() {
        let app = ChatApp::new();
        for id in ["a", "b", "c"] {
            app.inject_conversation(mk_conv(id, "ws", ConversationType::Group, &["alice"]));
        }
        assert_eq!(app.list_all_conversations_internal().len(), 3);
    }

    // delete_conversation ------------------------------------------------

    #[test]
    fn delete_conversation_allows_human_member() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("alice", "ws"));
        app.inject_conversation(mk_conv("c1", "ws", ConversationType::Group, &["alice"]));
        app.delete_conversation("alice", "c1").unwrap();
        assert!(!app.has_conversation("c1"));
    }

    #[test]
    fn delete_conversation_rejects_human_who_is_not_a_member() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("alice", "ws"));
        app.inject_conversation(mk_conv("c1", "ws", ConversationType::Group, &["bob"]));

        let error = app.delete_conversation("alice", "c1").unwrap_err();

        assert!(matches!(error, AppError::Forbidden(_)));
        assert!(app.has_conversation("c1"));
    }

    #[test]
    fn delete_conversation_returns_not_found_for_unknown_conv() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws"));
        let err = app.delete_conversation("human", "missing").unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn delete_conversation_removes_conv_and_seq() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws"));
        app.inject_conversation(mk_conv("c1", "ws", ConversationType::Group, &["human"]));
        // seed next_seq
        {
            let mut state = app.inner.write().unwrap();
            state.next_server_seq.insert("c1".into(), 5);
        }
        app.delete_conversation("human", "c1").unwrap();
        assert!(!app.has_conversation("c1"));
        let state = app.inner.read().unwrap();
        assert!(!state.next_server_seq.contains_key("c1"));
    }

    #[test]
    fn delete_conversations_batch_returns_ok_and_failed_counts() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws"));
        app.inject_conversation(mk_conv("c1", "ws", ConversationType::Group, &["human"]));
        app.inject_conversation(mk_conv("c2", "ws", ConversationType::Group, &["human"]));
        let (ok, failed) =
            app.delete_conversations_batch("human", &["c1".into(), "c2".into(), "missing".into()]);
        assert_eq!((ok, failed), (2, 1));
    }

    // create_direct_conversation -----------------------------------------

    #[test]
    fn create_direct_conversation_rejects_self_dm() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("alice", "ws"));
        let err = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: "alice".into(),
                peer_principal_id: "alice".into(),
                workspace_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn create_direct_conversation_returns_existing_when_pair_already_exists() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("alice", "ws"));
        app.inject_principal(mk_human("bob", "ws"));
        let first = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: "alice".into(),
                peer_principal_id: "bob".into(),
                workspace_id: None,
            })
            .unwrap();
        let second = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: "bob".into(),
                peer_principal_id: "alice".into(),
                workspace_id: None,
            })
            .unwrap();
        assert_eq!(first.id, second.id, "direct conv pair-key dedupe");
    }

    #[test]
    fn create_direct_conversation_rejects_cross_workspace_pair() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("alice", "ws-a"));
        app.inject_principal(mk_human("bob", "ws-b"));
        let err = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: "alice".into(),
                peer_principal_id: "bob".into(),
                workspace_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn create_agent_direct_rejects_human_without_target_workspace_access() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("human", "ws-other"));
        app.inject_principal(mk_agent("agent-a", "company-1"));
        app.inject_principal(mk_agent("agent-b", "company-1"));
        app.inject_company(mk_company("company-1", "human"), vec![]);

        let error = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: "agent-a".into(),
                peer_principal_id: "agent-b".into(),
                workspace_id: Some("company-1".into()),
            })
            .unwrap_err();

        assert!(matches!(error, AppError::Conflict(_)));
    }

    // create_group --------------------------------------------------------

    #[test]
    fn create_group_rejects_blank_name() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws"));
        let err = app
            .create_group(CreateGroupRequest {
                actor_id: "human".into(),
                name: "  ".into(),
                description: None,
                avatar_url: None,
                member_ids: vec![],
                workspace_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn create_group_makes_actor_owner_and_adds_distinct_members() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws"));
        app.inject_principal(mk_human("alice", "ws"));
        app.inject_principal(mk_human("bob", "ws"));
        let g = app
            .create_group(CreateGroupRequest {
                actor_id: "human".into(),
                name: "team".into(),
                description: Some("desc".into()),
                avatar_url: None,
                // include duplicates and the actor — they should be deduped
                member_ids: vec!["alice".into(), "bob".into(), "alice".into(), "human".into()],
                workspace_id: None,
            })
            .unwrap();
        assert_eq!(g.members.len(), 3); // human + alice + bob
        assert_eq!(g.name.as_deref(), Some("team"));
    }

    #[test]
    fn create_group_rejects_member_in_different_workspace() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws-a"));
        app.inject_principal(mk_human("outsider", "ws-b"));
        let err = app
            .create_group(CreateGroupRequest {
                actor_id: "human".into(),
                name: "team".into(),
                description: None,
                avatar_url: None,
                member_ids: vec!["outsider".into()],
                workspace_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn create_group_accepts_company_workspace_members() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("owner", "ws-owner"));
        app.inject_principal(mk_agent("member", "ws-member"));
        app.inject_company(
            mk_company("company-1", "owner"),
            vec![mk_company_member("owner"), mk_company_member("member")],
        );

        let group = app
            .create_group(CreateGroupRequest {
                actor_id: "owner".into(),
                name: "company team".into(),
                description: None,
                avatar_url: None,
                member_ids: vec!["member".into()],
                workspace_id: Some("company-1".into()),
            })
            .unwrap();

        assert_eq!(group.workspace_id, "company-1");
        assert!(group.members.contains_key("owner"));
        assert!(group.members.contains_key("member"));
    }

    #[test]
    fn agent_created_group_falls_back_from_inaccessible_owner_to_eligible_human() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("stale-owner", "ws-stale"));
        app.inject_principal(mk_human("eligible-human", "ws-human"));
        app.inject_principal(mk_agent("agent", "company-1"));
        app.inject_company(
            mk_company("company-1", "stale-owner"),
            vec![
                mk_company_member("eligible-human"),
                mk_company_member("agent"),
            ],
        );

        let group = app
            .create_group(CreateGroupRequest {
                actor_id: "agent".into(),
                name: "agent team".into(),
                description: None,
                avatar_url: None,
                member_ids: vec![],
                workspace_id: Some("company-1".into()),
            })
            .unwrap();

        assert!(group.members.contains_key("eligible-human"));
        assert!(!group.members.contains_key("stale-owner"));
    }

    #[test]
    fn create_direct_conversation_rejects_unrelated_explicit_company_workspace() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("alice", "ws-alice"));
        app.inject_principal(mk_human("bob", "ws-bob"));
        app.inject_company(mk_company("company-1", "bob"), vec![]);

        let err = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: "alice".into(),
                peer_principal_id: "bob".into(),
                workspace_id: Some("company-1".into()),
            })
            .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn create_direct_conversation_accepts_company_workspace_members() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("owner", "ws-owner"));
        app.inject_principal(mk_human("member", "ws-member"));
        app.inject_company(
            mk_company("company-1", "owner"),
            vec![mk_company_member("owner"), mk_company_member("member")],
        );

        let conversation = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: "owner".into(),
                peer_principal_id: "member".into(),
                workspace_id: Some("company-1".into()),
            })
            .unwrap();

        assert_eq!(conversation.workspace_id, "company-1");
    }

    // update_group --------------------------------------------------------

    #[test]
    fn update_group_changes_name_and_description() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws"));
        let group = app
            .create_group(CreateGroupRequest {
                actor_id: "human".into(),
                name: "old".into(),
                description: None,
                avatar_url: None,
                member_ids: vec![],
                workspace_id: None,
            })
            .unwrap();
        let updated = app
            .update_group(
                &group.id,
                UpdateGroupRequest {
                    actor_id: "human".into(),
                    name: Some("new".into()),
                    description: Some("desc".into()),
                    avatar_url: None,
                },
            )
            .unwrap();
        assert_eq!(updated.name.as_deref(), Some("new"));
        assert_eq!(updated.description.as_deref(), Some("desc"));
    }

    // add_group_members / remove_group_member ----------------------------

    #[test]
    fn add_group_members_adds_new_members() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws"));
        app.inject_principal(mk_human("alice", "ws"));
        app.inject_principal(mk_human("bob", "ws"));
        let group = app
            .create_group(CreateGroupRequest {
                actor_id: "human".into(),
                name: "team".into(),
                description: None,
                avatar_url: None,
                member_ids: vec![],
                workspace_id: None,
            })
            .unwrap();
        let updated = app
            .add_group_members(
                &group.id,
                AddGroupMembersRequest {
                    actor_id: "human".into(),
                    member_ids: vec!["alice".into(), "bob".into()],
                },
            )
            .unwrap();
        assert!(updated.members.contains_key("alice"));
        assert!(updated.members.contains_key("bob"));
        assert_eq!(updated.members.len(), 3);
    }

    #[test]
    fn add_group_members_accepts_company_workspace_members() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("owner", "ws-owner"));
        app.inject_principal(mk_human("member", "ws-member"));
        app.inject_company(
            mk_company("company-1", "owner"),
            vec![mk_company_member("owner"), mk_company_member("member")],
        );
        let group = app
            .create_group(CreateGroupRequest {
                actor_id: "owner".into(),
                name: "company team".into(),
                description: None,
                avatar_url: None,
                member_ids: vec![],
                workspace_id: Some("company-1".into()),
            })
            .unwrap();

        let updated = app
            .add_group_members(
                &group.id,
                AddGroupMembersRequest {
                    actor_id: "owner".into(),
                    member_ids: vec!["member".into()],
                },
            )
            .unwrap();

        assert!(updated.members.contains_key("member"));
    }

    #[test]
    fn stale_company_member_cannot_add_group_members() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("stale-owner", "ws-stale"));
        app.inject_principal(mk_human("member", "ws-member"));
        app.inject_company(
            mk_company("company-1", "stale-owner"),
            vec![mk_company_member("member")],
        );
        app.inject_conversation(mk_conv(
            "stale-company-conv",
            "company-1",
            ConversationType::Group,
            &["stale-owner"],
        ));

        let err = app
            .add_group_members(
                "stale-company-conv",
                AddGroupMembersRequest {
                    actor_id: "stale-owner".into(),
                    member_ids: vec!["member".into()],
                },
            )
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn remove_group_member_removes_existing() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws"));
        app.inject_principal(mk_agent("alice", "ws"));
        let group = app
            .create_group(CreateGroupRequest {
                actor_id: "human".into(),
                name: "team".into(),
                description: None,
                avatar_url: None,
                member_ids: vec!["alice".into()],
                workspace_id: None,
            })
            .unwrap();
        app.remove_group_member(&group.id, "human", "alice")
            .unwrap();
        let after = app.get_conversation(&group.id).unwrap();
        assert!(!after.members.contains_key("alice"));
    }

    #[test]
    fn remove_group_member_accepts_company_workspace_members() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("owner", "ws-owner"));
        app.inject_principal(mk_agent("member", "ws-member"));
        app.inject_company(
            mk_company("company-1", "owner"),
            vec![mk_company_member("owner"), mk_company_member("member")],
        );
        let group = app
            .create_group(CreateGroupRequest {
                actor_id: "owner".into(),
                name: "company team".into(),
                description: None,
                avatar_url: None,
                member_ids: vec!["member".into()],
                workspace_id: Some("company-1".into()),
            })
            .unwrap();

        let updated = app
            .remove_group_member(&group.id, "owner", "member")
            .unwrap();
        assert!(!updated.members.contains_key("member"));
    }

    #[test]
    fn stale_company_member_cannot_update_group() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("stale-owner", "ws-stale"));
        app.inject_company(mk_company("company-1", "stale-owner"), vec![]);
        app.inject_conversation(mk_conv(
            "stale-company-conv",
            "company-1",
            ConversationType::Group,
            &["stale-owner"],
        ));

        let err = app
            .update_group(
                "stale-company-conv",
                UpdateGroupRequest {
                    actor_id: "stale-owner".into(),
                    name: Some("renamed".into()),
                    description: None,
                    avatar_url: None,
                },
            )
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn migrated_agent_direct_keeps_original_pair_index() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("human", "ws-old"));
        app.inject_principal(mk_agent("agent-a", "ws-old"));
        app.inject_principal(mk_agent("agent-b", "ws-old"));
        app.inject_company(
            mk_company("ws-old", "human"),
            vec![mk_company_member("human")],
        );
        app.inject_company(
            mk_company("ws-new", "human"),
            vec![
                mk_company_member("human"),
                mk_company_member("agent-a"),
                mk_company_member("agent-b"),
            ],
        );

        let original = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: "agent-a".into(),
                peer_principal_id: "agent-b".into(),
                workspace_id: Some("ws-old".into()),
            })
            .unwrap();
        assert_eq!(original.members.len(), 3);

        app.migrate_conversation_workspace(&original.id, "ws-new", "human")
            .unwrap();
        let repeated = app
            .create_direct_conversation(CreateDirectConversationRequest {
                actor_id: "agent-b".into(),
                peer_principal_id: "agent-a".into(),
                workspace_id: Some("ws-new".into()),
            })
            .unwrap();

        assert_eq!(repeated.id, original.id);
    }

    // list_conversations / get_conversation ------------------------------

    #[test]
    fn list_conversations_filters_by_membership() {
        let app = ChatApp::new();
        app.inject_principal(mk_operator("human", "ws"));
        app.inject_principal(mk_human("alice", "ws"));
        app.create_group(CreateGroupRequest {
            actor_id: "human".into(),
            name: "team".into(),
            description: None,
            avatar_url: None,
            member_ids: vec!["alice".into()],
            workspace_id: None,
        })
        .unwrap();
        app.create_group(CreateGroupRequest {
            actor_id: "human".into(),
            name: "private".into(),
            description: None,
            avatar_url: None,
            member_ids: vec![],
            workspace_id: None,
        })
        .unwrap();

        let alice_view = app.list_conversations("alice").unwrap();
        // alice is in 'team' but not 'private'
        let names: Vec<String> = alice_view.iter().filter_map(|c| c.name.clone()).collect();
        assert!(names.iter().any(|n| n == "team"));
    }

    #[test]
    fn list_conversations_hides_deleted_company_workspace() {
        let app = ChatApp::new();
        app.inject_principal(mk_human("owner", "owner-ws"));
        app.inject_company(
            mk_deleted_company("company-1", "owner"),
            vec![mk_company_member("owner")],
        );
        app.inject_conversation(mk_conv(
            "deleted-company-conv",
            "company-1",
            ConversationType::Group,
            &["owner"],
        ));

        let owner_view = app.list_conversations("owner").unwrap();
        assert!(owner_view.is_empty());
    }

    #[test]
    fn get_conversation_returns_not_found_for_unknown() {
        let app = ChatApp::new();
        let err = app.get_conversation("missing").unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }
}
