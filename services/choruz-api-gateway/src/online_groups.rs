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

async fn process(
    app: &ChatApp,
    db: &DbService,
    actor: &Principal,
    link: &OnlineGroupLink,
    id: &str,
    body: &Value,
) -> Result<(), choruz_common::AppError> {
    match (link.role.as_str(), body["kind"].as_str()) {
        ("host", Some("join")) => {
            db.activate_online_guest(actor, &link.id, body["name"].as_str().unwrap_or(""))
                .await?
        }
        ("host", Some("say")) if link.status == "active" => {
            let content = body["content"]
                .as_str()
                .filter(|s| !s.trim().is_empty() && s.len() <= 32_000)
                .ok_or_else(|| {
                    choruz_common::AppError::Validation("Invalid Online message".into())
                })?;
            let sender = link.peer_principal_id.clone().ok_or_else(|| {
                choruz_common::AppError::Internal("Online guest identity missing".into())
            })?;
            super::handlers_messages::publish_message(
                app,
                db,
                SendMessageRequest {
                    actor_id: sender,
                    conversation_id: link.conversation_id.clone(),
                    idempotency_key: format!("online:{}:{id}", link.id),
                    content: content.into(),
                    content_type: "text".into(),
                    metadata: json!({"online_link_id":link.id}),
                    trace_id: None,
                },
            )
            .await?;
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
    db.online_processed(actor, id).await
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
    loop {
        tokio::select! {
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
                            client.acknowledge(&delivery.id).await.map_err(choruz_common::AppError::Internal)?;
                        }
                        Event::Accepted(id)=>db.online_accepted(&actor,&id).await?,
                        Event::Revoked(id)=> { for link in db.online_groups(&actor).await?.into_iter().filter(|l|l.channel_id==id) { db.revoke_online_group(&actor,&link.id).await?; } },
                        Event::Rejected{id,reason}=>tracing::warn!(message_id=%id,reason=%reason,"Online shipment not accepted; retained for retry"),
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
                                Err(choruz_common::AppError::Validation(_)|choruz_common::AppError::Forbidden(_))=> { db.online_processed(&actor,&id).await?; tracing::warn!(message_id=%id,"Online message rejected by group permission or protocol"); },
                                Err(e)=>return Err(e),
                            }
                        }
                        db.queue_online_history(&actor,&link.id).await?;
                        let current=db.online_group(&actor,&link.id).await?;
                        if current.status=="revoked" { return Ok(()); }
                        if let Some(recipient)=current.peer_account_id.as_deref() {
                            for (id,body) in db.online_outgoing(&actor,&link.id).await? {
                                if budget==0 { break; }
                                budget-=1;
                                tokio::time::timeout(Duration::from_secs(2),client.send(&id,&link.channel_id,recipient,&identity.account_id,&link.encryption_key,body)).await.map_err(|_|choruz_common::AppError::Internal("Online send queue is busy".into()))?.map_err(choruz_common::AppError::Internal)?;
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
