//! Guest-owned Agents run in an internal local group through the normal message pipeline.
//! The host owns shared ordering; credentialless peer principals never execute locally.
use super::{DbService, OnlineGroupLink};
use choruz_common::{AppError, new_id};
use choruz_domain::{OnlineAuthorContext, OnlineSharedMessage, Principal, PrincipalType};
use serde_json::{Value, json};

fn storage(e: tokio_postgres::Error) -> AppError {
    if let Some(db) = e.as_db_error() {
        tracing::error!(
            sqlstate = db.code().code(),
            constraint = db.constraint(),
            "Online Agent storage failed"
        );
    }
    AppError::Internal("Online Agent storage failed; check the server log".into())
}

fn proxy(link: &OnlineGroupLink, id: &str) -> String {
    format!("online:{}:{id}", link.channel_id)
}

fn generation(body: &Value) -> Result<i64, AppError> {
    body["generation"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(|| AppError::Validation("Missing Online membership generation".into()))
}

impl DbService {
    async fn guest_agent_link(
        &self,
        actor: &Principal,
        id: &str,
    ) -> Result<OnlineGroupLink, AppError> {
        let link = self.online_group(actor, id).await?;
        if actor.principal_type != PrincipalType::Human
            || link.role != "guest"
            || link.status != "active"
        {
            return Err(AppError::Forbidden(
                "An active guest group is required to share your Agents".into(),
            ));
        }
        Ok(link)
    }

    /// Lists only this device's visible, bound Agents reachable by the link owner.
    pub async fn online_agents(&self, actor: &Principal, id: &str) -> Result<Value, AppError> {
        let link = self.guest_agent_link(actor, id).await?;
        let accessible: Vec<String> = self
            .list_accessible_agents(&actor.id)
            .await?
            .into_iter()
            .map(|p| p.id)
            .collect();
        let client = self.store.connect().await?;
        let rows = client.query(
            "SELECT p.id,p.name,a.status,a.error FROM principal p
             LEFT JOIN online_group_agent a ON a.link_id=$1 AND a.agent_id=p.id
             WHERE p.id=ANY($2) AND p.type='agent' AND NOT p.online_guest
               AND NOT p.disabled AND p.deleted_at IS NULL AND p.channel_visibility='visible'
               AND EXISTS(SELECT 1 FROM agent_runtime_bindings b WHERE b.agent_principal_id=p.id AND b.state<>'disabled')
             ORDER BY lower(p.name),p.id", &[&link.id,&accessible]).await.map_err(storage)?;
        Ok(json!(rows.into_iter().map(|r| json!({"id":r.get::<_,String>("id"),"name":r.get::<_,String>("name"),"status":r.get::<_,Option<String>>("status"),"error":r.get::<_,Option<String>>("error")})).collect::<Vec<_>>()))
    }

    /// Queue a registration; no Agent executes until the authenticated host accepts it.
    pub async fn add_online_agent(
        &self,
        actor: &Principal,
        id: &str,
        agent_id: &str,
    ) -> Result<(), AppError> {
        let link = self.guest_agent_link(actor, id).await?;
        let eligible = self.online_agents(actor, id).await?;
        if !eligible
            .as_array()
            .is_some_and(|a| a.iter().any(|p| p["id"] == agent_id))
        {
            return Err(AppError::Forbidden(
                "Choose a visible Agent with a runtime on this device".into(),
            ));
        }
        let agent = self.get_principal(agent_id).await?;
        let identity = self
            .online_identity(actor)
            .await?
            .ok_or_else(|| AppError::Forbidden("Sign in to Online first".into()))?;
        let mut client = self.store.connect().await?;
        let binding=client.query_one("SELECT driver_type,config_json->>'harness_account_name' AS account_name FROM agent_runtime_bindings WHERE agent_principal_id=$1 AND state<>'disabled' ORDER BY created_at LIMIT 1", &[&agent.id]).await.map_err(storage)?;
        let context = OnlineAuthorContext {
            owner_name: Some(identity.display_name),
            device_name: Some("Guest's device".into()),
            account_name: binding.get("account_name"),
            harness: Some(binding.get("driver_type")),
        };
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
            return Err(AppError::Forbidden(
                "Online group is no longer active".into(),
            ));
        }
        if tx.query_opt("SELECT 1 FROM online_group_agent WHERE link_id=$1 AND agent_id=$2 AND status IN ('pending','active')", &[&link.id,&agent.id]).await.map_err(storage)?.is_some() { return Ok(()); }
        // Recheck Company access inside the membership transaction, not only in the picker.
        let allowed=tx.query_opt("SELECT 1 FROM principal p LEFT JOIN company c ON c.id=p.workspace_id WHERE p.id=$1 AND NOT p.disabled AND p.deleted_at IS NULL AND (c.id IS NULL OR c.deleted_at IS NULL) AND (p.workspace_id=$2 OR EXISTS(SELECT 1 FROM company_member cm WHERE cm.company_id=c.id AND cm.principal_id=$3)) FOR SHARE OF p", &[&agent.id,&actor.workspace_id,&actor.id]).await.map_err(storage)?;
        if allowed.is_none() {
            return Err(AppError::Forbidden(
                "Agent workspace is no longer accessible".into(),
            ));
        }
        let conv = tx.query_opt("SELECT conversation_id FROM online_execution_workspace WHERE link_id=$1 AND workspace_id=$2", &[&link.id,&agent.workspace_id]).await.map_err(storage)?.map(|r|r.get::<_,String>(0)).unwrap_or_else(||format!("online-group:{}:{}",link.id,agent.workspace_id));
        // No local human membership: this execution projection must not create a second sidebar group.
        tx.execute("INSERT INTO conversation(id,workspace_id,type,name,creator_id) VALUES($1,$2,'group',$3,$4) ON CONFLICT(id) DO NOTHING", &[&conv,&agent.workspace_id,&link.name,&actor.id]).await.map_err(storage)?;
        tx.execute("INSERT INTO online_execution_workspace(link_id,workspace_id,conversation_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING", &[&link.id,&agent.workspace_id,&conv]).await.map_err(storage)?;
        let generation:i64=tx.query_one("INSERT INTO online_group_agent(link_id,workspace_id,agent_id,remote_id,context,status,start_seq)
                    VALUES($1,$2,$3,$4,$5,'pending',(SELECT COALESCE(MAX(server_seq),0) FROM online_group_message WHERE link_id=$1))
                    ON CONFLICT(link_id,agent_id) DO UPDATE SET status='pending',error=NULL,start_seq=EXCLUDED.start_seq,context=EXCLUDED.context,generation=online_group_agent.generation+1 RETURNING generation",
                    &[&link.id,&actor.workspace_id,&agent.id,&proxy(&link,&agent.id),&json!(context)]).await.map_err(storage)?.get(0);
        tx.execute("INSERT INTO online_group_outbox(id,link_id,workspace_id,body) VALUES($1,$2,$3,$4)", &[&new_id(),&link.id,&actor.workspace_id,&json!({"kind":"agent_join","generation":generation,"agent":{"id":agent.id,"name":agent.name,"context":context}})]).await.map_err(storage)?;
        tx.execute("INSERT INTO audit_log(id,workspace_id,actor_id,action,target_type,target_id,metadata) VALUES($1,$2,$3,'online.agent_requested','agent',$4,$5)", &[&new_id(),&actor.workspace_id,&actor.id,&agent.id,&json!({"link_id":link.id})]).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        tracing::info!(event="online.agent_requested",link_id=%link.id,agent_id=%agent.id,"Online Agent registration queued");
        Ok(())
    }

    pub async fn remove_online_agent(
        &self,
        actor: &Principal,
        id: &str,
        agent_id: &str,
    ) -> Result<(), AppError> {
        let link = self.guest_agent_link(actor, id).await?;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        tx.query_one(
            "SELECT id FROM online_group_link WHERE id=$1 FOR UPDATE",
            &[&link.id],
        )
        .await
        .map_err(storage)?;
        let generation:i64=tx.query_opt("UPDATE online_group_agent SET status='removed',error=NULL,generation=generation+1 WHERE link_id=$1 AND agent_id=$2 RETURNING generation", &[&link.id,&agent_id]).await.map_err(storage)?.ok_or_else(||AppError::NotFound("This Agent is not shared in this group".into()))?.get(0);
        tx.execute(
            "UPDATE conversation_member SET removed_at=NOW() WHERE conv_id IN (SELECT conversation_id FROM online_execution_workspace WHERE link_id=$1) AND principal_id=$2",
            &[&link.id, &agent_id],
        )
        .await
        .map_err(storage)?;
        tx.execute(
            "INSERT INTO online_group_outbox(id,link_id,workspace_id,body) VALUES($1,$2,$3,$4)",
            &[
                &new_id(),
                &link.id,
                &actor.workspace_id,
                &json!({"kind":"agent_leave","agent_id":agent_id,"generation":generation}),
            ],
        )
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        self.record_audit(
            &actor.workspace_id,
            &actor.id,
            "online.agent_removed",
            "agent",
            agent_id,
            json!({"link_id":link.id}),
        )
        .await?;
        tracing::info!(event="online.agent_removed",link_id=%link.id,agent_id,"Online Agent stopped sharing");
        Ok(())
    }

    /// An encrypted peer can register only namespaced, credentialless principals in its invited group.
    pub async fn accept_online_agent(
        &self,
        actor: &Principal,
        id: &str,
        delivery_id: &str,
        body: &Value,
    ) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        if link.role != "host" || link.status != "active" {
            return Err(AppError::Forbidden(
                "Online host group is not active".into(),
            ));
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Registration {
            id: String,
            name: String,
            context: OnlineAuthorContext,
        }
        let agent: Registration = serde_json::from_value(body["agent"].clone())
            .map_err(|_| AppError::Validation("Invalid Online Agent registration".into()))?;
        if agent.id.is_empty()
            || agent.id.len() > 64
            || !agent
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || agent.name.trim().is_empty()
            || agent.name.len() > 80
            || agent.name.chars().any(char::is_control)
            || [
                &agent.context.owner_name,
                &agent.context.device_name,
                &agent.context.account_name,
                &agent.context.harness,
            ]
            .into_iter()
            .flatten()
            .any(|s| s.len() > 200 || s.chars().any(char::is_control))
        {
            return Err(AppError::Validation("Invalid Online Agent fields".into()));
        }
        let conv = self.get_conversation(&link.conversation_id).await?;
        if conv.creator_id != actor.id
            || !link
                .peer_principal_id
                .as_ref()
                .is_some_and(|p| conv.members.contains_key(p))
        {
            return Err(AppError::Forbidden(
                "Online guest is not a group member".into(),
            ));
        }
        let proxy_id = proxy(&link, &agent.id);
        let generation = generation(body)?;
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
            return Err(AppError::Forbidden("Online group revoked".into()));
        }
        if tx.query_opt("SELECT 1 FROM online_group_agent WHERE link_id=$1 AND remote_id=$2 AND generation>$3", &[&link.id,&agent.id,&generation]).await.map_err(storage)?.is_some(){return Ok(());}
        let collision=tx.query_opt("SELECT 1 FROM conversation_member m JOIN principal p ON p.id=m.principal_id WHERE m.conv_id=$1 AND m.removed_at IS NULL AND lower(p.name)=lower($2) AND p.id<>$3", &[&conv.id,&agent.name,&proxy_id]).await.map_err(storage)?.is_some();
        let error = collision
            .then_some("An Agent with this name is already in the group; rename your Agent first");
        if !collision {
            tx.execute("INSERT INTO principal(id,workspace_id,type,name,disabled,online_guest) VALUES($1,$2,'agent',$3,FALSE,TRUE) ON CONFLICT(id) DO UPDATE SET name=EXCLUDED.name", &[&proxy_id,&conv.workspace_id,&agent.name]).await.map_err(storage)?;
            tx.execute("INSERT INTO conversation_member(conv_id,principal_id) VALUES($1,$2) ON CONFLICT(conv_id,principal_id) DO UPDATE SET removed_at=NULL", &[&conv.id,&proxy_id]).await.map_err(storage)?;
            tx.execute("INSERT INTO online_group_agent(link_id,workspace_id,agent_id,remote_id,context,status,generation) VALUES($1,$2,$3,$4,$5,'active',$6) ON CONFLICT(link_id,agent_id) DO UPDATE SET status='active',context=EXCLUDED.context,generation=EXCLUDED.generation", &[&link.id,&actor.workspace_id,&proxy_id,&agent.id,&json!(agent.context),&generation]).await.map_err(storage)?;
        }
        tx.execute("INSERT INTO online_group_outbox(id,link_id,workspace_id,body) VALUES($1,$2,$3,$4)", &[&new_id(),&link.id,&actor.workspace_id,&json!({"kind":"agent_joined","agent_id":agent.id,"generation":generation,"error":error})]).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        tracing::info!(event="online.agent_registration",link_id=%link.id,delivery_id,agent_id=%proxy_id,accepted=!collision,"Online Agent registration processed");
        Ok(())
    }

    pub async fn confirm_online_agent(
        &self,
        actor: &Principal,
        id: &str,
        body: &Value,
    ) -> Result<(), AppError> {
        let link = self.guest_agent_link(actor, id).await?;
        let agent = body["agent_id"]
            .as_str()
            .ok_or_else(|| AppError::Validation("Missing Online Agent id".into()))?;
        let rejected = body["error"].is_string();
        let generation = generation(body)?;
        let error =
            rejected.then_some("The host rejected this Agent name; rename it and try again");
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        tx.query_one(
            "SELECT id FROM online_group_link WHERE id=$1 FOR UPDATE",
            &[&link.id],
        )
        .await
        .map_err(storage)?;
        let count=tx.execute("UPDATE online_group_agent SET status=$3,error=$4 WHERE link_id=$1 AND agent_id=$2 AND status='pending' AND generation=$5", &[&link.id,&agent,&if rejected {"error"} else {"active"},&error,&generation]).await.map_err(storage)?;
        if count > 0 && !rejected {
            tx.execute("INSERT INTO conversation_member(conv_id,principal_id) SELECT x.conversation_id,p.id FROM online_execution_workspace x JOIN principal p ON p.workspace_id=x.workspace_id WHERE x.link_id=$1 AND p.id=$2 ON CONFLICT(conv_id,principal_id) DO UPDATE SET removed_at=NULL", &[&link.id,&agent]).await.map_err(storage)?;
        }
        tx.commit().await.map_err(storage)?;
        tracing::info!(event="online.agent_confirmed",link_id=%link.id,agent_id=agent,accepted=!rejected,"Online Agent membership confirmation received");
        Ok(())
    }

    pub async fn accept_online_agent_leave(
        &self,
        actor: &Principal,
        id: &str,
        agent: &str,
        generation: i64,
    ) -> Result<(), AppError> {
        let link = self.online_group(actor, id).await?;
        if link.role != "host" {
            return Err(AppError::Forbidden(
                "Only the host receives Agent removal".into(),
            ));
        }
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        tx.query_one(
            "SELECT id FROM online_group_link WHERE id=$1 FOR UPDATE",
            &[&link.id],
        )
        .await
        .map_err(storage)?;
        if generation <= 0
            || agent.len() > 64
            || agent.is_empty()
            || !agent
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(AppError::Validation("Invalid Online Agent removal".into()));
        }
        let proxy_id = proxy(&link, agent);
        tx.execute("INSERT INTO principal(id,workspace_id,type,name,disabled,online_guest) VALUES($1,$2,'agent','Removed Agent',FALSE,TRUE) ON CONFLICT(id) DO NOTHING", &[&proxy_id,&actor.workspace_id]).await.map_err(storage)?;
        let rows=tx.query("INSERT INTO online_group_agent(link_id,workspace_id,agent_id,remote_id,status,generation) VALUES($1,$2,$3,$4,'removed',$5) ON CONFLICT(link_id,agent_id) DO UPDATE SET status='removed',generation=EXCLUDED.generation WHERE online_group_agent.generation<=EXCLUDED.generation RETURNING agent_id", &[&link.id,&actor.workspace_id,&proxy_id,&agent,&generation]).await.map_err(storage)?;
        for row in rows {
            tx.execute("UPDATE conversation_member SET removed_at=NOW() WHERE conv_id=$1 AND principal_id=$2", &[&link.conversation_id,&row.get::<_,String>(0)]).await.map_err(storage)?;
        }
        tx.commit().await.map_err(storage)?;
        tracing::info!(event="online.agent_removal_received",link_id=%link.id,remote_agent_id=agent,agent_id=%proxy_id,generation,"Host processed guest Agent removal");
        Ok(())
    }

    pub async fn online_agent_sender(
        &self,
        actor: &Principal,
        id: &str,
        remote_id: &str,
    ) -> Result<(String, OnlineAuthorContext), AppError> {
        let link = self.online_group(actor, id).await?;
        if link.role != "host" || link.status != "active" {
            return Err(AppError::Forbidden("Online host is not active".into()));
        }
        let client = self.store.connect().await?;
        let row=client.query_opt("SELECT a.agent_id,a.context FROM online_group_agent a JOIN conversation_member m ON m.principal_id=a.agent_id AND m.conv_id=$3 AND m.removed_at IS NULL WHERE a.link_id=$1 AND a.remote_id=$2 AND a.status='active'", &[&link.id,&remote_id,&link.conversation_id]).await.map_err(storage)?
            .ok_or_else(|| AppError::Forbidden("Online Agent is not a member of this group".into()))?;
        let context = serde_json::from_value(row.get(1))
            .map_err(|_| AppError::Internal("Invalid stored Online Agent attribution".into()))?;
        Ok((row.get(0), context))
    }

    /// Return the next undispatched shared message and its local sender, or advance past history/echoes.
    pub async fn next_online_agent_input(
        &self,
        actor: &Principal,
        id: &str,
    ) -> Result<Option<crate::SendMessageRequest>, AppError> {
        let link = self.guest_agent_link(actor, id).await?;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        let row=tx.query_opt("SELECT m.body,x.conversation_id,x.workspace_id FROM online_group_message m JOIN online_execution_workspace x ON x.link_id=m.link_id
            WHERE m.link_id=$1 AND m.server_seq>x.input_seq
              AND EXISTS(SELECT 1 FROM online_group_agent a JOIN principal p ON p.id=a.agent_id WHERE a.link_id=$1 AND a.status='active' AND a.start_seq<m.server_seq AND p.workspace_id=x.workspace_id)
              AND NOT EXISTS(SELECT 1 FROM online_group_agent a WHERE a.link_id=$1 AND a.remote_id=m.body->>'sender_id')
            ORDER BY m.server_seq,x.conversation_id LIMIT 1", &[&link.id]).await.map_err(storage)?;
        let Some(row) = row else { return Ok(None) };
        let m: OnlineSharedMessage = serde_json::from_value(row.get(0))
            .map_err(|_| AppError::Validation("Stored Online message is invalid".into()))?;
        let conv: String = row.get(1);
        let workspace: String = row.get(2);
        let sender = format!("online-input:{}:{}:{}", link.id, workspace, m.sender_id);
        // Imported principals are attribution only. The member provider excludes online_guest.
        tx.execute("INSERT INTO principal(id,workspace_id,type,name,disabled,online_guest) VALUES($1,$2,$3,$4,FALSE,TRUE) ON CONFLICT(id) DO UPDATE SET name=EXCLUDED.name", &[&sender,&workspace,&if m.agent {"agent"} else {"human"},&m.sender_name]).await.map_err(storage)?;
        tx.execute("INSERT INTO conversation_member(conv_id,principal_id) VALUES($1,$2) ON CONFLICT(conv_id,principal_id) DO UPDATE SET removed_at=NULL", &[&conv,&sender]).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(Some(crate::SendMessageRequest {
            actor_id: sender,
            idempotency_key: format!("online-input:{conv}:{}", m.id),
            conversation_id: conv,
            content: m.content,
            content_type: "text".into(),
            metadata: json!({"online_inbound":true,"online_link_id":link.id,"online_seq":m.seq,"online_message_id":m.id}),
            trace_id: Some(m.id),
        }))
    }

    pub async fn finish_online_agent_input(
        &self,
        actor: &Principal,
        id: &str,
        conversation_id: &str,
        seq: i64,
    ) -> Result<(), AppError> {
        let link = self.guest_agent_link(actor, id).await?;
        self.store
            .connect()
            .await?
            .execute(
                "UPDATE online_execution_workspace SET input_seq=GREATEST(input_seq,$3) WHERE link_id=$1 AND conversation_id=$2",
                &[&link.id, &conversation_id, &seq],
            )
            .await
            .map_err(storage)?;
        Ok(())
    }

    /// Queue only locally executed replies. History echoes never become new shared messages.
    pub async fn queue_online_agent_replies(
        &self,
        actor: &Principal,
        id: &str,
    ) -> Result<(), AppError> {
        let link = self.guest_agent_link(actor, id).await?;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        tx.query_one(
            "SELECT id FROM online_group_link WHERE id=$1 FOR UPDATE",
            &[&link.id],
        )
        .await
        .map_err(storage)?;
        let rows=tx.query("SELECT e.event_id,e.seq,e.conversation_id,e.content,e.content_type,e.sender_id,a.status FROM online_execution_workspace x JOIN conversation_events e ON e.conversation_id=x.conversation_id LEFT JOIN online_group_agent a ON a.link_id=x.link_id AND a.agent_id=e.sender_id WHERE x.link_id=$1 AND e.seq>x.output_seq AND e.event_type IN ('message','message.created','reply') ORDER BY e.seq,e.conversation_id LIMIT 100", &[&link.id]).await.map_err(storage)?;
        for r in rows {
            let seq: i64 = r.get("seq");
            if r.get::<_, Option<String>>("status").as_deref() == Some("active") {
                let event: String = r.get("event_id");
                let sender: String = r.get("sender_id");
                let content: Option<String> = r.get("content");
                if let Some(content) = content.filter(|s| !s.trim().is_empty()) {
                    let content_type: Option<String> = r.get("content_type");
                    let content = super::online_groups::shared_content(
                        content_type.as_deref().unwrap_or("text"),
                        content,
                    );
                    let mut body = json!({"kind":"agent_say","agent_id":sender,"content":content});
                    if body.to_string().len() > 450_000 {
                        body["content"] = json!(
                            "[Message exceeds the Online transfer limit; read it on the owner's device.]"
                        );
                    }
                    let delivery_id = new_id();
                    tx.execute("INSERT INTO online_group_outbox(id,link_id,workspace_id,body) VALUES($1,$2,$3,$4)", &[&delivery_id,&link.id,&actor.workspace_id,&body]).await.map_err(storage)?;
                    tracing::info!(event="online.agent_reply_queued",link_id=%link.id,delivery_id=%delivery_id,message_id=%event,agent_id=%sender,seq,"Local Agent reply queued for host");
                }
            }
            tx.execute(
                "UPDATE online_execution_workspace SET output_seq=$3 WHERE link_id=$1 AND conversation_id=$2",
                &[&link.id, &r.get::<_,String>("conversation_id"), &seq],
            )
            .await
            .map_err(storage)?;
        }
        tx.commit().await.map_err(storage)
    }
}
