//! Online's background owner. Its typed group messages cannot invoke runtime APIs.
use choruz_application::{
    ChatApp, DbService, SendMessageRequest,
    db_service::{OnlineGroupLink, OnlineIdentity},
};
use choruz_domain::Principal;
use choruz_relay_client::online::{Client, Credentials, Event, Status};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};

struct Session {
    device: String,
    task: JoinHandle<()>,
    status: watch::Receiver<Status>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Clone, Default)]
pub(crate) struct OnlineHub(Arc<Mutex<HashMap<String, Session>>>);
impl OnlineHub {
    pub fn spawn(app: ChatApp, db: DbService) -> Self {
        let hub = Self::default();
        let weak = Arc::downgrade(&hub.0);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                let people = match db.online_people().await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(error=%e,"Online identity scan failed");
                        continue;
                    }
                };
                let Ok(mut sessions) = inner.lock() else {
                    break;
                };
                sessions.retain(|id, _| people.iter().any(|(p, _)| p.id == *id));
                for (actor, identity) in people {
                    if sessions
                        .get(&actor.id)
                        .is_some_and(|s| s.device == identity.device_id)
                    {
                        continue;
                    }
                    let (client, events) = Client::start(Credentials {
                        origin: identity.service_url.clone(),
                        account: identity.account_id.clone(),
                        token: identity.session_token.clone(),
                    });
                    let status = client.status.clone();
                    let id = actor.id.clone();
                    let device = identity.device_id.clone();
                    let db = db.clone();
                    let app = app.clone();
                    let task = tokio::spawn(async move {
                        maintain(app, db, actor, identity, client, events).await;
                    });
                    sessions.insert(
                        id,
                        Session {
                            device,
                            task,
                            status,
                        },
                    );
                }
            }
        });
        hub
    }
    pub fn status(&self, id: &str) -> Status {
        self.0
            .lock()
            .ok()
            .and_then(|s| s.get(id).map(|s| *s.status.borrow()))
            .unwrap_or(Status::Connecting)
    }
    pub fn stop(&self, id: &str) {
        if let Ok(mut s) = self.0.lock() {
            s.remove(id);
        }
    }
}

pub(crate) async fn cloud(
    identity: &OnlineIdentity,
    path: &str,
    method: reqwest::Method,
    invitation: Option<&str>,
) -> Result<reqwest::Response, choruz_common::AppError> {
    let mut request = super::handlers_online::client()?
        .request(method, format!("{}/v1/online/{path}", identity.service_url))
        .bearer_auth(&identity.session_token);
    if let Some(grant) = invitation {
        request = request.header("x-online-invitation", grant);
    }
    request
        .send()
        .await
        .map_err(|_| choruz_common::AppError::Internal("Online gateway is unreachable".into()))
}

async fn peer(
    db: &DbService,
    actor: &Principal,
    identity: &OnlineIdentity,
    link: &OnlineGroupLink,
) -> Result<bool, choruz_common::AppError> {
    let response = cloud(
        identity,
        &format!("links/{}", link.channel_id),
        reqwest::Method::GET,
        None,
    )
    .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        db.revoke_online_group(actor, &link.id).await?;
        return Ok(false);
    }
    if !response.status().is_success() {
        return Err(choruz_common::AppError::Internal(
            "Online link verification failed".into(),
        ));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|_| choruz_common::AppError::Internal("Invalid Online link response".into()))?;
    let expected = if link.role == "host" {
        &data["owner_id"]
    } else {
        &data["peer_id"]
    };
    if expected.as_str() != Some(&identity.account_id) {
        return Err(choruz_common::AppError::Forbidden(
            "Online link identity changed".into(),
        ));
    }
    let other = if link.role == "host" {
        &data["peer_id"]
    } else {
        &data["owner_id"]
    };
    if let Some(id) = other.as_str() {
        db.online_set_peer(actor, &link.id, id).await?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) async fn process(
    app: &ChatApp,
    db: &DbService,
    actor: &Principal,
    link: &OnlineGroupLink,
    id: &str,
    body: &Value,
) -> Result<(), choruz_common::AppError> {
    match (link.role.as_str(), body["kind"].as_str()) {
        ("host", Some("agent_join")) => db.accept_online_agent(actor, &link.id, id, body).await?,
        ("host", Some("agent_leave")) => {
            db.accept_online_agent_leave(
                actor,
                &link.id,
                body["agent_id"].as_str().ok_or_else(|| {
                    choruz_common::AppError::Validation("Missing Online Agent id".into())
                })?,
                body["generation"].as_i64().unwrap_or(0),
            )
            .await?
        }
        ("guest", Some("agent_joined")) => db.confirm_online_agent(actor, &link.id, body).await?,
        ("host", Some("join")) => {
            db.activate_online_guest(actor, &link.id, body["name"].as_str().unwrap_or(""))
                .await?
        }
        ("host", Some("say" | "agent_say")) if link.status == "active" => {
            let is_agent = body["kind"] == "agent_say";
            let content = body["content"]
                .as_str()
                .filter(|s| {
                    !s.trim().is_empty() && s.len() <= if is_agent { 400_000 } else { 32_000 }
                })
                .ok_or_else(|| {
                    choruz_common::AppError::Validation("Invalid Online message".into())
                })?;
            let mut metadata = json!({"online_link_id":link.id});
            let sender = if is_agent {
                let (sender, context) = db
                    .online_agent_sender(
                        actor,
                        &link.id,
                        body["agent_id"].as_str().ok_or_else(|| {
                            choruz_common::AppError::Validation("Missing Online Agent id".into())
                        })?,
                    )
                    .await?;
                metadata["online_author_context"] = json!(context);
                metadata["runtime_host_name"] = json!(context.device_name);
                sender
            } else {
                link.peer_principal_id.clone().ok_or_else(|| {
                    choruz_common::AppError::Internal("Online guest identity missing".into())
                })?
            };
            let message = super::handlers_messages::publish_message(
                app,
                db,
                SendMessageRequest {
                    actor_id: sender,
                    conversation_id: link.conversation_id.clone(),
                    idempotency_key: format!("online:{}:{id}", link.id),
                    content: content.into(),
                    content_type: "text".into(),
                    metadata,
                    trace_id: Some(id.into()),
                },
            )
            .await?;
            tracing::info!(event="online.message_published",link_id=%link.id,delivery_id=id,message_id=%message.id,conversation_id=%message.conversation_id,agent=is_agent,"Online message entered the group pipeline");
        }
        ("host", Some("sync")) => {
            db.rewind_online_history(actor, &link.id, body["after"].as_i64().unwrap_or(-1))
                .await?
        }
        ("guest", Some("welcome" | "batch")) => db.save_online_batch(actor, &link.id, body).await?,
        _ => {
            return Err(choruz_common::AppError::Validation(
                "Unsupported Online group message".into(),
            ));
        }
    }
    db.online_processed(actor, id).await?;
    tracing::info!(event="online.delivery_processed",link_id=%link.id,delivery_id=id,kind=body["kind"].as_str().unwrap_or("invalid"),"Online delivery processed");
    Ok(())
}

async fn maintain(
    app: ChatApp,
    db: DbService,
    actor: Principal,
    identity: OnlineIdentity,
    client: Client,
    mut events: tokio::sync::mpsc::Receiver<Event>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(3));
    let mut cleaned = HashSet::new();
    let mut tick = 0u64;
    let mut connection = client.status.clone();
    loop {
        tokio::select! {
            changed=connection.changed()=> {
                if changed.is_err() { break; }
                tracing::info!(event="online.connection_changed",principal_id=%actor.id,device_id=%identity.device_id,status=?*connection.borrow_and_update(),"Online transport state changed");
            }
            event=events.recv()=> {
                let Some(event)=event else { break };
                let result=async {
                    match event {
                        Event::Delivery(delivery)=> {
                            let link=db.online_groups(&actor).await?.into_iter().find(|l|l.channel_id==delivery.channel).ok_or_else(||choruz_common::AppError::NotFound("Online channel unavailable".into()))?;
                            if !peer(&db,&actor,&identity,&link).await? { return Ok(()); }
                            let link=db.online_group(&actor,&link.id).await?;
                            if link.peer_account_id.as_deref()!=Some(&delivery.sender) { return Err(choruz_common::AppError::Forbidden("Online sender mismatch".into())); }
                            let body=delivery.open(&link.encryption_key,&identity.account_id).map_err(|_|choruz_common::AppError::Forbidden("Invalid encrypted group delivery".into()))?;
                            db.receive_online(&actor,&link.id,&delivery.id,&body).await?;
                            tracing::info!(event="online.delivery_stored",link_id=%link.id,delivery_id=%delivery.id,kind=body["kind"].as_str().unwrap_or("invalid"),"Encrypted Online delivery persisted before acknowledgement");
                            client.acknowledge(&delivery.id).await.map_err(choruz_common::AppError::Internal)?;
                        }
                        Event::Accepted(id)=> {
                            db.online_accepted(&actor,&id).await?;
                            tracing::info!(event="online.shipment_accepted",principal_id=%actor.id,delivery_id=%id,"Gateway accepted encrypted shipment");
                        },
                        Event::Revoked(id)=> { for link in db.online_groups(&actor).await?.into_iter().filter(|l|l.channel_id==id) { db.revoke_online_group(&actor,&link.id).await?; } },
                        Event::Rejected{id,reason}=>tracing::warn!(message_id=%id,reason=%reason,"Online shipment not accepted; retained for retry"),
                        Event::ProtocolRejected=>tracing::warn!(event="online.protocol_rejected",principal_id=%actor.id,"Gateway rejected the Online wire protocol; shipments retained for retry"),
                    }
                    Ok::<_,choruz_common::AppError>(())
                }.await;
                if let Err(e)=result { tracing::warn!(principal_id=%actor.id,error=%e,"Online delivery failed"); }
            }
            _=interval.tick()=> {
                if *client.status.borrow()!=Status::Connected { continue; }
                tick=tick.wrapping_add(1);
                let links=match db.online_groups(&actor).await { Ok(l)=>l,Err(e)=>{tracing::warn!(error=%e,"Online groups unavailable");continue;} };
                let mut budget=32usize;
                for link in links {
                    let result=async {
                        if link.status=="revoked" {
                            if !cleaned.contains(&link.id) {
                                let response=cloud(&identity,&format!("links/{}",link.channel_id),reqwest::Method::DELETE,None).await?;
                                if response.status().is_success()||response.status()==reqwest::StatusCode::NOT_FOUND { cleaned.insert(link.id.clone()); }
                            }
                            return Ok(());
                        }
                        if !peer(&db,&actor,&identity,&link).await? { return Ok(()); }
                        if tick%10==1 { db.queue_online_resync(&actor,&link.id).await?; }
                        for (id,body) in db.online_incoming(&actor,&link.id).await? {
                            let current=db.online_group(&actor,&link.id).await?;
                            match process(&app,&db,&actor,&current,&id,&body).await {
                                Ok(())=>{},
                                Err(e @ (choruz_common::AppError::Validation(_)|choruz_common::AppError::Forbidden(_)))=> { db.online_processed(&actor,&id).await?; tracing::warn!(event="online.delivery_rejected",link_id=%link.id,delivery_id=%id,kind=body["kind"].as_str().unwrap_or("invalid"),reason=%e,"Online message rejected by group permission or protocol"); },
                                Err(e)=>return Err(e),
                            }
                        }
                        if link.role=="guest"&&link.status=="active" {
                            for _ in 0..20 {
                                let Some(input)=db.next_online_agent_input(&actor,&link.id).await? else {break};
                                let seq=input.metadata["online_seq"].as_i64().unwrap_or(0);
                                let shared_id=input.metadata["online_message_id"].as_str().unwrap_or("").to_owned();
                                let message=super::handlers_messages::publish_message(&app,&db,input).await?;
                                db.finish_online_agent_input(&actor,&link.id,&message.conversation_id,seq).await?;
                                tracing::info!(event="online.agent_input_published",link_id=%link.id,shared_message_id=%shared_id,message_id=%message.id,conversation_id=%message.conversation_id,seq,"Shared message entered the guest's normal Agent pipeline");
                            }
                            db.queue_online_agent_replies(&actor,&link.id).await?;
                        }
                        db.queue_online_history(&actor,&link.id).await?;
                        let current=db.online_group(&actor,&link.id).await?;
                        if current.status=="revoked" { return Ok(()); }
                        if let Some(recipient)=current.peer_account_id.as_deref() {
                            for (id,body) in db.online_outgoing(&actor,&link.id).await? {
                                if budget==0 { break; }
                                budget-=1;
                                tokio::time::timeout(Duration::from_secs(2),client.send(&id,&link.channel_id,recipient,&identity.account_id,&link.encryption_key,body)).await.map_err(|_|choruz_common::AppError::Internal("Online send queue is busy".into()))?.map_err(choruz_common::AppError::Internal)?;
                                tracing::info!(event="online.shipment_sent",link_id=%link.id,delivery_id=%id,"Online shipment queued on encrypted transport");
                            }
                        }
                        Ok::<_,choruz_common::AppError>(())
                    }.await;
                    if let Err(e)=result { tracing::warn!(link_id=%link.id,error=%e,"Online group synchronization failed"); }
                }
            }
        }
    }
}
