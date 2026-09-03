use std::collections::BTreeMap;

use choruz_domain::{
    AuditLog, ChannelVisibility, Company, CompanyMember, Conversation, ConversationMember,
    ConversationType, EventEnvelope, Principal, PrincipalType,
};

/// Convert a `tokio_postgres::Row` to a `Principal`.
///
/// Scopes are not stored in the DB — they are inferred from `principal_type`,
/// matching the logic in `build_app_from_db`.
pub(crate) fn row_to_principal(row: &tokio_postgres::Row) -> Principal {
    let ptype: String = row.get("type");
    let principal_type = match ptype.as_str() {
        "agent" => PrincipalType::Agent,
        "human" => PrincipalType::Human,
        _ => PrincipalType::Human,
    };

    let scopes = scopes_for_type(&principal_type);
    let channel_visibility = match row
        .get::<_, Option<String>>("channel_visibility")
        .as_deref()
    {
        Some("internal") => ChannelVisibility::Internal,
        Some("visible") | None => ChannelVisibility::Visible,
        Some(_) => ChannelVisibility::Internal,
    };

    Principal {
        id: row.get("id"),
        workspace_id: row.get("workspace_id"),
        principal_type,
        name: row.get::<_, Option<String>>("name").unwrap_or_default(),
        avatar_url: row.get("avatar_url"),
        scopes,
        secret_hash: row.get("secret_hash"),
        disabled: row.get("disabled"),
        deleted_at: row.get("deleted_at"),
        channel_visibility,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        user_id: None,
    }
}

/// Return the default scopes for a given principal type.
///
/// This must stay in sync with `build_app_from_db` in
/// `choruz-api-gateway/src/main.rs`.
pub(crate) fn scopes_for_type(pt: &PrincipalType) -> Vec<String> {
    match pt {
        PrincipalType::Agent => vec![
            "messages:read".into(),
            "messages:write".into(),
            "events:read".into(),
        ],
        PrincipalType::Human => vec![
            "messages:read".into(),
            "messages:write".into(),
            "events:read".into(),
            "groups:manage".into(),
            "agents:manage".into(),
        ],
    }
}

/// Convert a `tokio_postgres::Row` to a `Conversation`.
///
/// Expects columns: id, workspace_id, type, name, description, avatar_url,
/// creator_id, created_at, updated_at.
pub(crate) fn row_to_conversation(
    row: &tokio_postgres::Row,
    members: BTreeMap<String, ConversationMember>,
) -> Conversation {
    let ctype: String = row.get("type");
    let conversation_type = match ctype.as_str() {
        "group" => ConversationType::Group,
        _ => ConversationType::Direct,
    };

    Conversation {
        id: row.get("id"),
        workspace_id: row.get("workspace_id"),
        conversation_type,
        name: row.get("name"),
        description: row.get("description"),
        avatar_url: row.get("avatar_url"),
        creator_id: row
            .get::<_, Option<String>>("creator_id")
            .unwrap_or_default(),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        members,
    }
}

/// Convert a `tokio_postgres::Row` to a `ConversationMember`.
///
/// Expects columns: principal_id, role, joined_at.
pub(crate) fn row_to_member(row: &tokio_postgres::Row) -> ConversationMember {
    ConversationMember {
        principal_id: row.get("principal_id"),
        joined_at: row.get("joined_at"),
    }
}

/// Convert multiple member rows into a BTreeMap keyed by principal_id.
pub(crate) fn rows_to_members(
    rows: &[tokio_postgres::Row],
) -> BTreeMap<String, ConversationMember> {
    let mut members = BTreeMap::new();
    for row in rows {
        let member = row_to_member(row);
        members.insert(member.principal_id.clone(), member);
    }
    members
}

/// Convert a `tokio_postgres::Row` to a `Company`.
///
/// Expects columns: id, name, slug, description, avatar_url, owner_id,
/// agents_active, folder_path, multi_harness_accounts, archived_at, deleted_at,
/// created_at, updated_at.
pub(crate) fn row_to_company(row: &tokio_postgres::Row) -> Company {
    Company {
        id: row.get("id"),
        name: row.get("name"),
        slug: row.get("slug"),
        description: row.get("description"),
        avatar_url: row.get("avatar_url"),
        owner_id: row.get("owner_id"),
        agents_active: row.get("agents_active"),
        folder_path: row.get("folder_path"),
        multi_harness_accounts: row.get("multi_harness_accounts"),
        archived_at: row.get("archived_at"),
        deleted_at: row.get("deleted_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Convert a `tokio_postgres::Row` to a `CompanyMember`.
///
/// Expects columns: principal_id, role, joined_at.
pub(crate) fn row_to_company_member(row: &tokio_postgres::Row) -> CompanyMember {
    CompanyMember {
        principal_id: row.get("principal_id"),
        joined_at: row.get("joined_at"),
    }
}

/// Convert a `tokio_postgres::Row` to an `AuditLog`.
///
/// Expects columns: id, workspace_id, actor_id, action, target_type,
/// target_id, metadata, created_at.
pub(crate) fn row_to_audit_log(row: &tokio_postgres::Row) -> AuditLog {
    AuditLog {
        id: row.get("id"),
        workspace_id: row.get("workspace_id"),
        actor_id: row.get("actor_id"),
        action: row.get("action"),
        target_type: row.get("target_type"),
        target_id: row.get("target_id"),
        metadata: row.get("metadata"),
        created_at: row.get("created_at"),
    }
}

/// Convert a `tokio_postgres::Row` to an `EventEnvelope`.
///
/// Expects columns: id, principal_id, event_type, payload, created_at,
/// delivery_seq.
pub(crate) fn row_to_event_envelope(row: &tokio_postgres::Row) -> EventEnvelope {
    let seq: i64 = row.get("delivery_seq");
    EventEnvelope {
        delivery_seq: seq as u64,
        event_id: row.get("id"),
        principal_id: row.get("principal_id"),
        event_type: row.get("event_type"),
        payload: row.get("payload"),
        created_at: row.get("created_at"),
    }
}
