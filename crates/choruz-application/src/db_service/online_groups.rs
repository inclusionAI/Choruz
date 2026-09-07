use super::{DbService, OnlineIdentity};
use choruz_common::{AppError, new_id};
use choruz_domain::{
    ConversationType, OnlineAuthorContext, OnlineSharedMessage, Principal, PrincipalType,
};
use serde_json::{Value, json};

/// Server material; the encryption key is never part of a group-list response.
#[derive(Clone)]
pub struct OnlineGroupLink {
    pub id: String,
    pub channel_id: String,
    pub principal_id: String,
    pub workspace_id: String,
    pub account_id: String,
    pub peer_account_id: Option<String>,
    pub role: String,
    pub conversation_id: String,
    pub peer_principal_id: Option<String>,
    pub encryption_key: String,
    pub name: String,
    pub status: String,
    pub last_seq: i64,
}

impl OnlineGroupLink {
    pub fn summary(&self) -> Value {
        json!({"id":self.id,"role":self.role,"name":self.name,"status":self.status,"conversation_id":self.conversation_id})
    }
}

fn row(r: tokio_postgres::Row) -> OnlineGroupLink {
    OnlineGroupLink {
        id: r.get("id"),
        channel_id: r.get("channel_id"),
        principal_id: r.get("principal_id"),
        workspace_id: r.get("workspace_id"),
        account_id: r.get("account_id"),
        peer_account_id: r.get("peer_account_id"),
        role: r.get("role"),
        conversation_id: r.get("conversation_id"),
        peer_principal_id: r.get("peer_principal_id"),
        encryption_key: r.get("encryption_key"),
        name: r.get("name"),
        status: r.get("status"),
        last_seq: r.get("last_seq"),
    }
}
fn storage(e: tokio_postgres::Error) -> AppError {
    if let Some(error) = e.as_db_error() {
        tracing::error!(
            sqlstate = error.code().code(),
            constraint = error.constraint(),
            "Online group database operation failed"
        );
    }
    AppError::Internal(format!("Online group storage: {e}"))
}

impl DbService {
    pub async fn queue_online_resync(&self, actor: &Principal, id: &str) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        if link.role != "guest" || link.status != "active" {
            return Ok(());
        }
        let client = self.store.connect().await?;
        let after: i64 = client
            .query_one(
                "SELECT COALESCE(MAX(server_seq),0) FROM online_group_message WHERE link_id=$1",
                &[&link.id],
            )
            .await
            .map_err(storage)?
            .get(0);
        client.execute("INSERT INTO online_group_outbox(id,link_id,workspace_id,body) SELECT $1,$2,$3,$4 WHERE NOT EXISTS(SELECT 1 FROM online_group_outbox WHERE link_id=$2 AND body->>'kind'='sync')", &[&new_id(),&link.id,&actor.workspace_id,&json!({"kind":"sync","after":after})]).await.map_err(storage)?;
        Ok(())
    }

    pub async fn rewind_online_history(
        &self,
        actor: &Principal,
        id: &str,
        after: i64,
    ) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        if link.role != "host" || link.status != "active" || after < 0 {
            return Err(AppError::Validation(
                "Invalid Online history request".into(),
            ));
        }
        let client = self.store.connect().await?;
        client.execute("UPDATE online_group_link SET last_seq=LEAST(last_seq,$2) WHERE id=$1 AND status='active'", &[&link.id,&after]).await.map_err(storage)?;
        Ok(())
    }

    pub async fn online_people(&self) -> Result<Vec<(Principal, OnlineIdentity)>, AppError> {
        let client = self.store.connect().await?;
        let ids = client
            .query("SELECT principal_id FROM online_identity", &[])
            .await
            .map_err(storage)?;
        let mut people = Vec::new();
        for id in ids {
            let actor = self.get_principal(&id.get::<_, String>(0)).await?;
            if actor.principal_type == PrincipalType::Human
                && !actor.disabled
                && actor.deleted_at.is_none()
                && let Some(identity) = self.online_identity(&actor).await?
            {
                people.push((actor, identity));
            }
        }
        Ok(people)
    }

    pub async fn online_groups(&self, actor: &Principal) -> Result<Vec<OnlineGroupLink>, AppError> {
        let client = self.store.connect().await?;
        Ok(client.query("SELECT l.* FROM online_group_link l JOIN online_identity i ON i.principal_id=l.principal_id AND i.account_id=l.account_id WHERE l.principal_id=$1 AND l.workspace_id=$2 ORDER BY l.created_at", &[&actor.id,&actor.workspace_id]).await.map_err(storage)?.into_iter().map(row).collect())
    }

    pub async fn online_group_summaries(&self, actor: &Principal) -> Result<Vec<Value>, AppError> {
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT l.*,
                (SELECT COUNT(*) FROM online_group_message m
                 WHERE m.link_id=l.id AND m.workspace_id=l.workspace_id) AS message_count,
                (SELECT COALESCE(MAX(server_seq),0) FROM online_group_message m
                 WHERE m.link_id=l.id AND m.workspace_id=l.workspace_id) AS latest_seq,
                (SELECT jsonb_build_object('content', left(m.body->>'content', 200),
                                          'created_at', m.body->>'created_at')
                 FROM online_group_message m
                 WHERE m.link_id=l.id AND m.workspace_id=l.workspace_id
                 ORDER BY m.server_seq DESC LIMIT 1) AS last_message
             FROM online_group_link l
             JOIN online_identity i ON i.principal_id=l.principal_id AND i.account_id=l.account_id
             WHERE l.principal_id=$1 AND l.workspace_id=$2 ORDER BY l.created_at",
                &[&actor.id, &actor.workspace_id],
            )
            .await
            .map_err(storage)?;
        Ok(rows
            .into_iter()
            .map(|record| {
                let count: i64 = record.get("message_count");
                let latest: i64 = record.get("latest_seq");
                let preview: Option<Value> = record.get("last_message");
                let mut summary = row(record).summary();
                summary["message_count"] = json!(count);
                summary["latest_seq"] = json!(latest);
                summary["last_message"] = json!(preview);
                summary
            })
            .collect())
    }

    pub async fn online_group(
        &self,
        actor: &Principal,
        id: &str,
    ) -> Result<OnlineGroupLink, AppError> {
        self.online_groups(actor)
            .await?
            .into_iter()
            .find(|l| l.id == id)
            .ok_or_else(|| AppError::NotFound("Online group unavailable".into()))
    }

    pub async fn save_online_group(
        &self,
        actor: &Principal,
        link: &OnlineGroupLink,
    ) -> Result<(), AppError> {
        let identity = self
            .online_identity(actor)
            .await?
            .ok_or_else(|| AppError::Forbidden("Sign in to Online first".into()))?;
        if link.principal_id != actor.id
            || link.workspace_id != actor.workspace_id
            || link.account_id != identity.account_id
        {
            return Err(AppError::Forbidden("Online group identity mismatch".into()));
        }
        if link.role == "host" {
            let conv = self.get_conversation(&link.conversation_id).await?;
            if conv.creator_id != actor.id
                || conv.conversation_type != ConversationType::Group
                || !conv.members.contains_key(&actor.id)
            {
                return Err(AppError::Forbidden(
                    "Only the group owner can invite".into(),
                ));
            }
        }
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        tx.execute("INSERT INTO online_group_link(id,principal_id,workspace_id,account_id,peer_account_id,role,conversation_id,peer_principal_id,encryption_key,name,channel_id,status) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'pending')", &[&link.id,&actor.id,&actor.workspace_id,&link.account_id,&link.peer_account_id,&link.role,&link.conversation_id,&link.peer_principal_id,&link.encryption_key,&link.name,&link.channel_id]).await.map_err(storage)?;
        if link.role == "guest" {
            tx.execute(
                "INSERT INTO online_group_outbox(id,link_id,workspace_id,body) VALUES($1,$2,$3,$4)",
                &[
                    &new_id(),
                    &link.id,
                    &actor.workspace_id,
                    &json!({"kind":"join","name":identity.display_name}),
                ],
            )
            .await
            .map_err(storage)?;
        }
        tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata) VALUES($1,$2,$3,'online.group_linked','online_group',$4,$5)", &[&new_id(),&actor.workspace_id,&actor.id,&link.id,&json!({"role":link.role})]).await.map_err(storage)?;
        tx.commit().await.map_err(storage)
    }

    pub async fn online_set_peer(
        &self,
        actor: &Principal,
        id: &str,
        peer: &str,
    ) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        if link
            .peer_account_id
            .as_deref()
            .is_some_and(|old| old != peer)
        {
            return Err(AppError::Forbidden("Online peer changed".into()));
        }
        let client = self.store.connect().await?;
        client
            .execute(
                "UPDATE online_group_link SET peer_account_id=$2 WHERE id=$1 AND status<>'revoked'",
                &[&link.id, &peer],
            )
            .await
            .map_err(storage)?;
        Ok(())
    }

    pub async fn revoke_online_group(&self, actor: &Principal, id: &str) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        tx.execute(
            "UPDATE online_group_link SET status='revoked' WHERE id=$1",
            &[&link.id],
        )
        .await
        .map_err(storage)?;
        tx.execute(
            "DELETE FROM online_group_outbox WHERE link_id=$1",
            &[&link.id],
        )
        .await
        .map_err(storage)?;
        tx.execute(
            "UPDATE online_group_inbox SET processed=TRUE WHERE link_id=$1",
            &[&link.id],
        )
        .await
        .map_err(storage)?;
        if link.role == "host" {
            tx.execute("UPDATE conversation_member SET removed_at=NOW() WHERE conv_id=$1 AND principal_id=$2", &[&link.conversation_id,&link.peer_principal_id]).await.map_err(storage)?;
        }
        tx.execute("UPDATE conversation_member SET removed_at=NOW() WHERE (conv_id=$1 OR conv_id IN (SELECT conversation_id FROM online_execution_workspace WHERE link_id=$2)) AND principal_id IN (SELECT agent_id FROM online_group_agent WHERE link_id=$2)", &[&link.conversation_id,&link.id]).await.map_err(storage)?;
        tx.execute(
            "UPDATE online_group_agent SET status='removed' WHERE link_id=$1",
            &[&link.id],
        )
        .await
        .map_err(storage)?;
        tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata) VALUES($1,$2,$3,'online.group_revoked','online_group',$4,'{}')", &[&new_id(),&actor.workspace_id,&actor.id,&link.id]).await.map_err(storage)?;
        tx.commit().await.map_err(storage)
    }

    pub async fn queue_online_text(
        &self,
        actor: &Principal,
        id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        if link.role != "guest" || link.status != "active" {
            return Err(AppError::Conflict("This Online group is not active".into()));
        }
        if content.trim().is_empty() || content.len() > 32_000 {
            return Err(AppError::Validation(
                "Message must contain 1–32000 bytes".into(),
            ));
        }
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        let status: String = tx
            .query_one(
                "SELECT status FROM online_group_link WHERE id=$1 FOR UPDATE",
                &[&link.id],
            )
            .await
            .map_err(storage)?
            .get(0);
        if status != "active" {
            return Err(AppError::Conflict("This Online group is not active".into()));
        }
        let body = json!({"kind":"say","content":content});
        let existing = tx
            .query_opt(
                "SELECT link_id,body FROM online_group_outbox WHERE id=$1",
                &[&message_id],
            )
            .await
            .map_err(storage)?;
        if let Some(r) = existing {
            if r.get::<_, String>(0) == link.id && r.get::<_, Value>(1) == body {
                return Ok(());
            }
            return Err(AppError::Conflict("Message ID already used".into()));
        }
        tx.execute(
            "INSERT INTO online_group_outbox(id,link_id,workspace_id,body) VALUES($1,$2,$3,$4)",
            &[&message_id, &link.id, &actor.workspace_id, &body],
        )
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)
    }

    pub async fn online_outgoing(
        &self,
        actor: &Principal,
        id: &str,
    ) -> Result<Vec<(String, Value)>, AppError> {
        let link = self.online_group(actor, id).await?;
        let client = self.store.connect().await?;
        Ok(client.query("SELECT id,body FROM online_group_outbox WHERE link_id=$1 ORDER BY created_at LIMIT 32", &[&link.id]).await.map_err(storage)?.into_iter().map(|r|(r.get(0),r.get(1))).collect())
    }

    pub async fn online_accepted(&self, actor: &Principal, id: &str) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        client.execute("DELETE FROM online_group_outbox o USING online_group_link l WHERE o.id=$1 AND o.link_id=l.id AND l.principal_id=$2 AND l.workspace_id=$3", &[&id,&actor.id,&actor.workspace_id]).await.map_err(storage)?;
        Ok(())
    }

    /// ACK is safe after this commit; processing can resume after a crash.
    pub async fn receive_online(
        &self,
        actor: &Principal,
        link_id: &str,
        id: &str,
        body: &Value,
    ) -> Result<(), AppError> {
        let link = self.online_group(actor, link_id).await?;
        if link.status == "revoked" {
            return Err(AppError::Forbidden("Online group revoked".into()));
        }
        let client = self.store.connect().await?;
        client.execute("INSERT INTO online_group_inbox(id,link_id,workspace_id,body) VALUES($1,$2,$3,$4) ON CONFLICT(id) DO NOTHING", &[&id,&link.id,&actor.workspace_id,&body]).await.map_err(storage)?;
        let stored = client
            .query_one(
                "SELECT link_id,body FROM online_group_inbox WHERE id=$1",
                &[&id],
            )
            .await
            .map_err(storage)?;
        if stored.get::<_, String>(0) != link.id || stored.get::<_, Value>(1) != *body {
            return Err(AppError::Conflict(
                "Online delivery identity conflict".into(),
            ));
        }
        Ok(())
    }

    pub async fn online_incoming(
        &self,
        actor: &Principal,
        id: &str,
    ) -> Result<Vec<(String, Value)>, AppError> {
        let link = self.online_group(actor, id).await?;
        let client = self.store.connect().await?;
        Ok(client.query("SELECT id,body FROM online_group_inbox WHERE link_id=$1 AND NOT processed ORDER BY created_at LIMIT 32", &[&link.id]).await.map_err(storage)?.into_iter().map(|r|(r.get(0),r.get(1))).collect())
    }

    pub async fn online_processed(&self, actor: &Principal, id: &str) -> Result<(), AppError> {
        let client = self.store.connect().await?;
        client.execute("UPDATE online_group_inbox i SET processed=TRUE FROM online_group_link l WHERE i.id=$1 AND i.link_id=l.id AND l.principal_id=$2 AND l.workspace_id=$3", &[&id,&actor.id,&actor.workspace_id]).await.map_err(storage)?;
        Ok(())
    }

    pub async fn activate_online_guest(
        &self,
        actor: &Principal,
        id: &str,
        name: &str,
    ) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        if link.role != "host"
            || link.status == "revoked"
            || link.peer_account_id.is_none()
            || name.trim().is_empty()
            || name.len() > 80
        {
            return Err(AppError::Validation("Invalid Online join".into()));
        }
        let conv = self.get_conversation(&link.conversation_id).await?;
        if conv.creator_id != actor.id || !conv.members.contains_key(&actor.id) {
            return Err(AppError::Forbidden("Group owner unavailable".into()));
        }
        let peer = link
            .peer_principal_id
            .ok_or_else(|| AppError::Internal("Online guest identity missing".into()))?;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        let status: String = tx
            .query_one(
                "SELECT status FROM online_group_link WHERE id=$1 FOR UPDATE",
                &[&link.id],
            )
            .await
            .map_err(storage)?
            .get(0);
        if status == "revoked" {
            return Err(AppError::Forbidden("Online group revoked".into()));
        }
        if status == "active" {
            return Ok(());
        }
        tx.execute("INSERT INTO principal(id,workspace_id,type,name,disabled,online_guest) VALUES($1,$2,'human',$3,FALSE,TRUE) ON CONFLICT(id) DO NOTHING", &[&peer,&conv.workspace_id,&name]).await.map_err(storage)?;
        tx.execute("INSERT INTO conversation_member(conv_id,principal_id) VALUES($1,$2) ON CONFLICT(conv_id,principal_id) DO NOTHING", &[&conv.id,&peer]).await.map_err(storage)?;
        tx.execute(
            "UPDATE online_group_link SET status='active' WHERE id=$1",
            &[&link.id],
        )
        .await
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO online_group_outbox(id,link_id,workspace_id,body) VALUES($1,$2,$3,$4)",
            &[
                &new_id(),
                &link.id,
                &actor.workspace_id,
                &json!({"kind":"welcome","name":conv.name}),
            ],
        )
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)
    }

    pub async fn save_online_batch(
        &self,
        actor: &Principal,
        id: &str,
        body: &Value,
    ) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        if link.role != "guest" || link.status == "revoked" {
            return Err(AppError::Forbidden("Online group unavailable".into()));
        }
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        let status: String = tx
            .query_one(
                "SELECT status FROM online_group_link WHERE id=$1 FOR UPDATE",
                &[&link.id],
            )
            .await
            .map_err(storage)?
            .get(0);
        if status == "revoked" {
            return Err(AppError::Forbidden("Online group revoked".into()));
        }
        if body["kind"] == "welcome" {
            let name = body["name"]
                .as_str()
                .filter(|n| !n.is_empty() && n.len() <= 200)
                .ok_or_else(|| AppError::Validation("Invalid group name".into()))?;
            tx.execute("UPDATE online_group_link SET status='active',name=$2 WHERE id=$1 AND status<>'revoked'", &[&link.id,&name]).await.map_err(storage)?;
        } else {
            let messages = body["messages"]
                .as_array()
                .filter(|m| m.len() <= 20)
                .ok_or_else(|| AppError::Validation("Invalid Online message batch".into()))?;
            for message in messages {
                let parsed: OnlineSharedMessage = serde_json::from_value(message.clone())
                    .map_err(|_| AppError::Validation("Invalid shared message".into()))?;
                if parsed.id.is_empty()
                    || parsed.id.len() > 100
                    || parsed.seq <= 0
                    || parsed.sender_id.is_empty()
                    || parsed.sender_id.len() > 100
                    || parsed.sender_name.len() > 200
                    || parsed.content.len() > 400_000
                    || [
                        &parsed.author_context.owner_name,
                        &parsed.author_context.device_name,
                        &parsed.author_context.account_name,
                        &parsed.author_context.harness,
                    ]
                    .into_iter()
                    .flatten()
                    .any(|value| value.len() > 200 || value.chars().any(char::is_control))
                {
                    return Err(AppError::Validation("Invalid shared message fields".into()));
                }
                tx.execute("INSERT INTO online_group_message(link_id,workspace_id,event_id,server_seq,body) VALUES($1,$2,$3,$4,$5) ON CONFLICT(link_id,event_id) DO NOTHING", &[&link.id,&actor.workspace_id,&parsed.id,&parsed.seq,&message]).await.map_err(storage)?;
            }
        }
        tx.commit().await.map_err(storage)
    }

    pub async fn online_messages(
        &self,
        actor: &Principal,
        id: &str,
        after: i64,
    ) -> Result<Value, AppError> {
        let link = self.online_group(actor, id).await?;
        let client = self.store.connect().await?;
        let messages:Vec<Value>=client.query("SELECT body FROM online_group_message WHERE link_id=$1 AND server_seq>$2 ORDER BY server_seq LIMIT 100", &[&link.id,&after]).await.map_err(storage)?.into_iter().map(|r|r.get(0)).collect();
        let pending: Vec<Value> = self
            .online_outgoing(actor, id)
            .await?
            .into_iter()
            .filter(|(_, b)| b["kind"] == "say")
            .map(|(id, b)| json!({"id":id,"content":b["content"]}))
            .collect();
        Ok(json!({"messages":messages,"pending":pending,"status":link.status}))
    }

    /// Advance the history cursor in the same transaction as its durable shipment.
    pub async fn queue_online_history(&self, actor: &Principal, id: &str) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        if link.role != "host" || link.status != "active" {
            return Ok(());
        }
        if !self.online_outgoing(actor, id).await?.is_empty() {
            return Ok(());
        }
        let conv = self.get_conversation(&link.conversation_id).await?;
        if !link
            .peer_principal_id
            .as_ref()
            .is_some_and(|p| conv.members.contains_key(p))
            || !conv.members.contains_key(&actor.id)
        {
            self.revoke_online_group(actor, id).await?;
            return Ok(());
        }
        let page = self
            .list_message_page(&conv.id, 20, None, Some(link.last_seq as u64))
            .await?;
        if page.messages.is_empty() {
            return Ok(());
        }
        let mut last_seq = link.last_seq;
        let mut bytes = 0;
        let mut messages = Vec::new();
        let identity = self
            .online_identity(actor)
            .await?
            .ok_or_else(|| AppError::Forbidden("Online signed out".into()))?;
        let client = self.store.connect().await?;
        for m in page.messages {
            let sender = self.get_principal(&m.sender_id).await?;
            let mut author_context = OnlineAuthorContext::default();
            if sender.principal_type == PrincipalType::Agent {
                author_context.owner_name = Some(identity.display_name.clone());
                let binding = client.query_opt(
                    "SELECT b.driver_type, b.config_json->>'harness_account_name' AS account_name,
                            h.name AS device_name
                     FROM agent_runtime_bindings b
                     JOIN principal p ON p.id=b.agent_principal_id
                     LEFT JOIN runtime_host h ON h.id=b.config_json->>'runtime_host_id'
                                              AND h.company_id=p.workspace_id
                     WHERE b.agent_principal_id=$1 AND p.workspace_id=$2
                     ORDER BY (b.conversation_id=$3) DESC,
                              (b.config_json->>'is_primary'='true') DESC NULLS LAST,
                              b.created_at LIMIT 1",
                    &[&sender.id, &conv.workspace_id, &conv.id],
                ).await.map_err(storage)?;
                if let Some(binding) = binding {
                    author_context.device_name = Some(
                        binding
                            .get::<_, Option<String>>("device_name")
                            .unwrap_or_else(|| "Group owner's device".into()),
                    );
                    author_context.account_name = binding.get("account_name");
                    author_context.harness = Some(binding.get("driver_type"));
                }
            }
            if let Some(peer_agent)=client.query_opt("SELECT context FROM online_group_agent WHERE agent_id=$1 AND link_id IN (SELECT id FROM online_group_link WHERE conversation_id=$2 AND role='host') LIMIT 1", &[&sender.id,&conv.id]).await.map_err(storage)? {
                author_context=serde_json::from_value(peer_agent.get(0)).map_err(|_|AppError::Internal("Invalid stored Online Agent attribution".into()))?;
            }
            let sender_name = if sender.id == actor.id {
                &identity.display_name
            } else {
                &sender.name
            };
            let content = shared_content(&m.content_type, m.content);
            let mut message = json!({"id":m.id,"seq":m.server_seq,"sender_id":m.sender_id,"sender_name":sender_name,"agent":sender.principal_type==PrincipalType::Agent,"own":link.peer_principal_id.as_deref()==Some(&m.sender_id),"content":content,"created_at":m.created_at,"author_context":author_context});
            if message.to_string().len() > 450_000 {
                message["content"] = json!(
                    "[Message exceeds the Online transfer limit; read it on the owner's device.]"
                );
            }
            let size = message.to_string().len();
            if bytes + size > 500_000 && !messages.is_empty() {
                break;
            }
            bytes += size;
            last_seq = m.server_seq as i64;
            messages.push(message);
        }
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        let count=tx.execute("UPDATE online_group_link SET last_seq=$2 WHERE id=$1 AND last_seq=$3 AND status='active'", &[&link.id,&last_seq,&link.last_seq]).await.map_err(storage)?;
        if count == 1 {
            tx.execute(
                "INSERT INTO online_group_outbox(id,link_id,workspace_id,body) VALUES($1,$2,$3,$4)",
                &[
                    &new_id(),
                    &link.id,
                    &actor.workspace_id,
                    &json!({"kind":"batch","messages":messages}),
                ],
            )
            .await
            .map_err(storage)?;
        }
        tx.commit().await.map_err(storage)
    }
}

pub(super) fn shared_content(content_type: &str, content: String) -> String {
    if content_type == "attachment" {
        "[Attachment remains on the owner's device; Online shares text and Agent replies.]".into()
    } else if content.len() > 400_000 {
        "[Message exceeds the Online transfer limit; read it on the owner's device.]".into()
    } else {
        content
    }
}
