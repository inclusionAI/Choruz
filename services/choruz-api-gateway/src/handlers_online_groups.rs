use crate::{ApiError, ApiState, online_groups::cloud, require_human_operator};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use choruz_application::db_service::{OnlineGroupLink, OnlineIdentity};
use choruz_common::{AppError, new_id};
use choruz_domain::{ConversationType, Principal};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

async fn identity(
    headers: &HeaderMap,
    state: &ApiState,
) -> Result<(Principal, OnlineIdentity), ApiError> {
    let actor = require_human_operator(headers, state).await?;
    let identity = state
        .db
        .online_identity(&actor)
        .await?
        .ok_or_else(|| AppError::Forbidden("Sign in to Online first".into()))?;
    Ok((actor, identity))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Invite {
    conversation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Join {
    invitation: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Invitation {
    version: u8,
    origin: String,
    link: String,
    grant: String,
    key: String,
    conversation: String,
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Text {
    id: String,
    content: String,
}
#[derive(Deserialize)]
pub(crate) struct Cursor {
    #[serde(default)]
    after_seq: i64,
}

pub(crate) async fn list(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Result<Json<Value>, ApiError> {
    let (actor, _) = identity(&headers, &state).await?;
    let links = state.db.online_groups(&actor).await?;
    Ok(Json(
        json!({"groups":links.iter().map(OnlineGroupLink::summary).collect::<Vec<_>>(),"connection":state.online.status(&actor.id)}),
    ))
}

pub(crate) async fn invite(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(input): Json<Invite>,
) -> Result<Json<Value>, ApiError> {
    let (actor, identity) = identity(&headers, &state).await?;
    let conv = state.db.get_conversation(&input.conversation_id).await?;
    if conv.creator_id != actor.id
        || conv.conversation_type != ConversationType::Group
        || !conv.members.contains_key(&actor.id)
    {
        return Err(AppError::Forbidden("Only the group owner can invite".into()).into());
    }
    let response = cloud(&identity, "links", reqwest::Method::POST, None).await?;
    if !response.status().is_success() {
        return Err(AppError::Internal(
            "Could not create Online invitation; check your Online session".into(),
        )
        .into());
    }
    let data: Value = response
        .json()
        .await
        .map_err(|_| AppError::Internal("Invalid Online invitation response".into()))?;
    let id = data["id"]
        .as_str()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .ok_or_else(|| AppError::Internal("Missing Online link id".into()))?
        .to_owned();
    let grant = data["invitation"]
        .as_str()
        .filter(|s| s.len() == 64)
        .ok_or_else(|| AppError::Internal("Missing Online invitation".into()))?
        .to_owned();
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let key = URL_SAFE_NO_PAD.encode(key);
    let link = OnlineGroupLink {
        id: new_id(),
        channel_id: id.clone(),
        principal_id: actor.id.clone(),
        workspace_id: actor.workspace_id.clone(),
        account_id: identity.account_id.clone(),
        peer_account_id: None,
        role: "host".into(),
        conversation_id: conv.id.clone(),
        peer_principal_id: Some(new_id()),
        encryption_key: key.clone(),
        name: conv.name.clone().unwrap_or_else(|| "Group".into()),
        status: "pending".into(),
        last_seq: 0,
    };
    if let Err(e) = state.db.save_online_group(&actor, &link).await {
        let _ = cloud(
            &identity,
            &format!("links/{id}"),
            reqwest::Method::DELETE,
            None,
        )
        .await;
        return Err(e.into());
    }
    let invitation = Invitation {
        version: 1,
        origin: identity.service_url,
        link: id,
        grant,
        key,
        conversation: conv.id,
        name: link.name,
    };
    let bytes = serde_json::to_vec(&invitation)
        .map_err(|_| AppError::Internal("Encode Online invitation".into()))?;
    Ok(Json(
        json!({"invitation":format!("choruz-online.{}",URL_SAFE_NO_PAD.encode(bytes)),"expires_at":data["expires_at"]}),
    ))
}

pub(crate) async fn join(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Json(input): Json<Join>,
) -> Result<Json<Value>, ApiError> {
    let (actor, identity) = identity(&headers, &state).await?;
    let invalid = || AppError::Validation("Invalid Online group invitation".into());
    if input.invitation.len() > 4096 {
        return Err(invalid().into());
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(
            input
                .invitation
                .trim()
                .strip_prefix("choruz-online.")
                .ok_or_else(invalid)?,
        )
        .map_err(|_| invalid())?;
    let invitation: Invitation = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if invitation.version != 1
        || invitation.origin != identity.service_url
        || uuid::Uuid::parse_str(&invitation.link).is_err()
        || invitation.grant.len() != 64
        || URL_SAFE_NO_PAD
            .decode(&invitation.key)
            .map_or(true, |k| k.len() != 32)
        || invitation.name.len() > 200
    {
        return Err(invalid().into());
    }
    if let Some(existing) = state
        .db
        .online_groups(&actor)
        .await?
        .into_iter()
        .find(|l| l.channel_id == invitation.link)
    {
        if existing.role == "guest" && existing.status != "revoked" {
            return Ok(Json(existing.summary()));
        }
        return Err(
            AppError::Conflict("This invitation is already linked or revoked".into()).into(),
        );
    }
    let response = cloud(
        &identity,
        &format!("links/{}/join", invitation.link),
        reqwest::Method::POST,
        Some(&invitation.grant),
    )
    .await?;
    if !response.status().is_success() {
        return Err(AppError::Validation(
            "Online invitation expired, was used, or belongs to this account".into(),
        )
        .into());
    }
    let data: Value = response
        .json()
        .await
        .map_err(|_| AppError::Internal("Invalid Online join response".into()))?;
    let owner = data["owner_id"].as_str().ok_or_else(invalid)?.to_owned();
    if data["peer_id"].as_str() != Some(&identity.account_id) {
        return Err(invalid().into());
    }
    let link = OnlineGroupLink {
        id: new_id(),
        channel_id: invitation.link,
        principal_id: actor.id.clone(),
        workspace_id: actor.workspace_id.clone(),
        account_id: identity.account_id,
        peer_account_id: Some(owner),
        role: "guest".into(),
        conversation_id: invitation.conversation,
        peer_principal_id: None,
        encryption_key: invitation.key,
        name: invitation.name,
        status: "pending".into(),
        last_seq: 0,
    };
    state.db.save_online_group(&actor, &link).await?;
    Ok(Json(link.summary()))
}

pub(crate) async fn leave(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (actor, identity) = identity(&headers, &state).await?;
    let link = state.db.online_group(&actor, &id).await?;
    state.db.revoke_online_group(&actor, &id).await?;
    let response = cloud(
        &identity,
        &format!("links/{}", link.channel_id),
        reqwest::Method::DELETE,
        None,
    )
    .await?;
    if !response.status().is_success() && response.status() != reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::Internal(
            "Group disabled locally; cloud revocation is pending retry".into(),
        )
        .into());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn messages(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(cursor): Query<Cursor>,
) -> Result<Json<Value>, ApiError> {
    let (actor, _) = identity(&headers, &state).await?;
    if cursor.after_seq < 0 {
        return Err(AppError::Validation("Invalid message cursor".into()).into());
    }
    Ok(Json(
        state
            .db
            .online_messages(&actor, &id, cursor.after_seq)
            .await?,
    ))
}
pub(crate) async fn send(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(input): Json<Text>,
) -> Result<StatusCode, ApiError> {
    let (actor, _) = identity(&headers, &state).await?;
    if uuid::Uuid::parse_str(&input.id).is_err() {
        return Err(AppError::Validation("Message id must be a UUID".into()).into());
    }
    state.db.check_rate_limit(&actor.id)?;
    state
        .db
        .queue_online_text(&actor, &id, &input.id, &input.content)
        .await?;
    Ok(StatusCode::ACCEPTED)
}
