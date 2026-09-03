use choruz_common::{AppError, AppResult, now};
use choruz_domain::EventEnvelope;
use rand::RngCore;
use serde_json::json;

use crate::{
    AckEventsRequest, ChatApp, EventWebhookConfig, ListEventsQuery, SetEventWebhookRequest,
    WebhookDelivery,
};

/// Generate a 32-byte hex signing secret for outbound webhook HMAC.
/// Format: 64 lower-case hex chars (matches how apps typically echo it
/// back). Entropy source: `rand::rngs::OsRng`.
fn generate_webhook_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

impl ChatApp {
    /// Inject an event webhook config directly into memory.
    pub fn inject_event_webhook(&self, config: EventWebhookConfig) {
        let mut state = self.inner.write().expect("lock poisoned");
        state
            .event_webhooks
            .insert(config.principal_id.clone(), config);
    }

    /// Ensure `next_event_seq` for a principal is at least `min_seq`.
    /// Used on startup to align event sequence numbers with runner cursors
    /// so that new events are not skipped by clients with saved cursor positions.
    pub fn ensure_event_seq_at_least(&self, principal_id: &str, min_seq: u64) {
        let mut state = self.inner.write().expect("lock poisoned");
        let entry = state
            .next_event_seq
            .entry(principal_id.to_owned())
            .or_insert(0);
        if *entry < min_seq {
            *entry = min_seq;
        }
    }

    pub fn list_events(
        &self,
        principal_id: &str,
        query: ListEventsQuery,
    ) -> AppResult<Vec<EventEnvelope>> {
        let state = self.inner.read().expect("lock poisoned");
        let principal = self.require_active_principal(&state, principal_id)?;
        self.ensure_scope(&principal, "events:read")?;
        let cursor = query
            .cursor
            .unwrap_or_else(|| state.ack_cursor.get(principal_id).copied().unwrap_or(0));
        Ok(state
            .events
            .get(principal_id)
            .map(|events| {
                events
                    .iter()
                    .filter(|e| e.delivery_seq > cursor)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn ack_events(&self, principal_id: &str, request: AckEventsRequest) -> AppResult<u64> {
        let mut state = self.inner.write().expect("lock poisoned");
        self.require_active_principal(&state, principal_id)?;
        let ack = state.ack_cursor.entry(principal_id.to_owned()).or_insert(0);
        *ack = (*ack).max(request.upto_delivery_seq);
        let cursor = *ack;

        // GC: remove events that have been acknowledged by *all* consumers.
        // Take the min of polling cursor and webhook cursor so we never
        // discard events the webhook consumer has not yet delivered.
        let webhook_cursor = state
            .event_webhooks
            .get(principal_id)
            .map(|config| config.cursor)
            .unwrap_or(u64::MAX);
        let min_cursor = cursor.min(webhook_cursor);
        if let Some(events) = state.events.get_mut(principal_id) {
            events.retain(|e| e.delivery_seq > min_cursor);
        }

        Ok(cursor)
    }

    pub fn set_event_webhook(
        &self,
        principal_id: &str,
        request: SetEventWebhookRequest,
    ) -> AppResult<EventWebhookConfig> {
        let mut state = self.inner.write().expect("lock poisoned");
        self.check_rate_limit(&mut state, &request.actor_id)?;

        if request.url.trim().is_empty() {
            return Err(AppError::Validation("webhook url is required".into()));
        }
        if !request.url.starts_with("http://") && !request.url.starts_with("https://") {
            return Err(AppError::Validation(
                "webhook url must use http or https".into(),
            ));
        }

        let actor = self.require_active_principal(&state, &request.actor_id)?;
        let target = self.require_active_principal(&state, principal_id)?;
        if actor.id != target.id
            && (!matches!(actor.principal_type, choruz_domain::PrincipalType::Human)
                || !matches!(target.principal_type, choruz_domain::PrincipalType::Agent)
                || !self.principal_can_access_workspace(&state, &actor, &target.workspace_id))
        {
            return Err(AppError::Forbidden(
                "webhook target is outside the actor's workspace access".into(),
            ));
        }

        // Use caller-provided secret if present; otherwise generate a fresh
        // 32-byte hex string. The full secret is returned in the response
        // (once) so the app can store it for signature verification.
        let webhook_secret = request
            .secret
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(generate_webhook_secret);

        let config = EventWebhookConfig {
            principal_id: target.id.clone(),
            url: request.url,
            event_types: request.event_types,
            cursor: state.ack_cursor.get(&target.id).copied().unwrap_or(0),
            updated_at: now(),
            webhook_secret,
        };
        state
            .event_webhooks
            .insert(target.id.clone(), config.clone());

        self.record_audit(
            &mut state,
            &actor,
            "principal.webhook_configured",
            "principal",
            &target.id,
            json!({"event_types": config.event_types, "url": config.url}),
        );

        Ok(config)
    }

    pub fn collect_pending_webhook_deliveries(&self) -> Vec<WebhookDelivery> {
        let state = self.inner.read().expect("lock poisoned");
        let mut deliveries = Vec::new();

        let mut principals: Vec<_> = state.event_webhooks.keys().cloned().collect();
        principals.sort();
        for principal_id in principals {
            let Some(config) = state.event_webhooks.get(&principal_id) else {
                continue;
            };
            let Some(events) = state.events.get(&principal_id) else {
                continue;
            };
            for event in events
                .iter()
                .filter(|event| event.delivery_seq > config.cursor)
            {
                if !config.event_types.is_empty()
                    && !config
                        .event_types
                        .iter()
                        .any(|kind| kind == &event.event_type)
                {
                    continue;
                }
                deliveries.push(WebhookDelivery {
                    principal_id: principal_id.clone(),
                    url: config.url.clone(),
                    event: event.clone(),
                    secret: config.webhook_secret.clone(),
                });
            }
        }

        deliveries.sort_by(|left, right| {
            left.principal_id
                .cmp(&right.principal_id)
                .then(left.event.delivery_seq.cmp(&right.event.delivery_seq))
        });
        deliveries
    }

    pub fn mark_webhook_delivered(
        &self,
        principal_id: &str,
        upto_delivery_seq: u64,
    ) -> AppResult<u64> {
        let mut state = self.inner.write().expect("lock poisoned");
        let config = state
            .event_webhooks
            .get_mut(principal_id)
            .ok_or_else(|| AppError::NotFound("webhook not configured".into()))?;
        config.cursor = config.cursor.max(upto_delivery_seq);
        config.updated_at = now();
        Ok(config.cursor)
    }
}
