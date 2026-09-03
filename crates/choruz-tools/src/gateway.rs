//! Tool Gateway: idempotent tool invocation with Effect Journal integration.
//!
//! # Lifecycle
//!
//! 1. Receive a [`ToolCallRequest`].
//! 2. If the tool is **read-only** -> execute directly, skip journal.
//! 3. If **mutating** -> check `effect_journal`:
//!    - `succeeded` -> return cached result (CacheAndSuppress).
//!    - `failed` -> re-execute.
//!    - `executing` + stale -> treat as failed, re-execute.
//!    - `executing` + fresh -> return `StillExecuting` conflict.
//!    - absent -> insert `pending`, claim -> `executing`, run, record result.
//! 4. Attach the stable, opaque `external_idempotency_key` namespace for APIs
//!    that accept idempotency keys. This value is persisted and must not be
//!    rebranded, or a retried pre-cutover effect could be duplicated.

use std::time::Duration;

use choruz_ids::{AttemptId, ToolCallId, TurnId};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::effect::{self, EffectStatus, NewEffect};
use crate::registry::ToolRegistry;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Inbound tool call request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub tool_call_id: ToolCallId,
    pub attempt_id: AttemptId,
    pub turn_id: TurnId,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
}

/// Outbound tool call response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResponse {
    /// The output produced by the tool (or cached from a prior invocation).
    pub output: serde_json::Value,
    /// Whether this result was served from the effect journal cache.
    pub cached: bool,
    /// The idempotency key that was (or would be) attached to the external call.
    pub idempotency_key: String,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during tool gateway execution.
#[derive(Debug, thiserror::Error)]
pub enum ToolGatewayError {
    /// The tool call is already executing (not stale). Caller should wait.
    #[error("tool call {0} is still executing")]
    StillExecuting(ToolCallId),

    /// The tool executor returned an error.
    #[error("tool execution failed: {0}")]
    ExecutionFailed(String),

    /// A database or infrastructure error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<choruz_common::AppError> for ToolGatewayError {
    fn from(e: choruz_common::AppError) -> Self {
        Self::Internal(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tool executor trait
// ---------------------------------------------------------------------------

/// Trait for the actual tool execution backend.
///
/// Implementations perform the real side-effect (HTTP call, file write, etc.).
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute the tool with the given input.
    ///
    /// `idempotency_key` should be forwarded to external APIs that support it.
    async fn execute(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        idempotency_key: &str,
    ) -> Result<serde_json::Value, String>;
}

// ---------------------------------------------------------------------------
// Tool Gateway
// ---------------------------------------------------------------------------

/// Timeout after which an `executing` effect is considered stale and can be
/// reclaimed.
const STALE_EXECUTING_TIMEOUT: Duration = Duration::from_secs(120);

/// The core gateway that mediates all tool invocations.
pub struct ToolGateway<E: ToolExecutor> {
    registry: ToolRegistry,
    executor: E,
}

impl<E: ToolExecutor> ToolGateway<E> {
    /// Create a new gateway.
    pub fn new(registry: ToolRegistry, executor: E) -> Self {
        Self { registry, executor }
    }

    /// Build the idempotency key for a tool call.
    #[must_use]
    pub fn idempotency_key(tool_call_id: &ToolCallId) -> String {
        format!("echat:{tool_call_id}")
    }

    /// Execute a tool call with full idempotency handling.
    pub async fn execute(
        &self,
        client: &tokio_postgres::Client,
        req: &ToolCallRequest,
    ) -> Result<ToolCallResponse, ToolGatewayError> {
        let idem_key = Self::idempotency_key(&req.tool_call_id);

        // Fast path: read-only tools skip the journal entirely.
        if !self.registry.is_mutating(&req.tool_name) {
            tracing::debug!(
                tool_call_id = %req.tool_call_id,
                tool = %req.tool_name,
                "read-only tool, skipping idempotency check",
            );
            let output = self
                .executor
                .execute(&req.tool_name, &req.tool_input, &idem_key)
                .await
                .map_err(ToolGatewayError::ExecutionFailed)?;

            return Ok(ToolCallResponse {
                output,
                cached: false,
                idempotency_key: idem_key,
            });
        }

        // Mutating tool: check effect journal.
        if let Some(existing) = effect::get_effect(client, &req.tool_call_id).await? {
            match existing.status {
                EffectStatus::Succeeded => {
                    tracing::info!(
                        tool_call_id = %req.tool_call_id,
                        tool = %req.tool_name,
                        "returning cached tool result (CacheAndSuppress)",
                    );
                    return Ok(ToolCallResponse {
                        output: existing.tool_output.unwrap_or(serde_json::Value::Null),
                        cached: true,
                        idempotency_key: idem_key,
                    });
                }
                EffectStatus::Executing => {
                    let age = Utc::now()
                        .signed_duration_since(existing.created_at)
                        .to_std()
                        .unwrap_or(Duration::ZERO);

                    if age < STALE_EXECUTING_TIMEOUT {
                        return Err(ToolGatewayError::StillExecuting(req.tool_call_id));
                    }
                    tracing::warn!(
                        tool_call_id = %req.tool_call_id,
                        age_secs = age.as_secs(),
                        "stale executing effect, will re-execute",
                    );
                    // Fall through to re-execute.
                }
                EffectStatus::Failed => {
                    tracing::info!(
                        tool_call_id = %req.tool_call_id,
                        "re-executing previously failed tool call",
                    );
                    // Fall through to re-execute.
                }
                EffectStatus::Pending => {
                    // Someone inserted but never claimed. Try to claim.
                    tracing::debug!(
                        tool_call_id = %req.tool_call_id,
                        "found pending effect, attempting to claim",
                    );
                }
            }
        } else {
            // No existing record: insert a new pending effect.
            let new_effect = NewEffect {
                tool_call_id: req.tool_call_id,
                attempt_id: req.attempt_id,
                turn_id: req.turn_id,
                tool_name: req.tool_name.clone(),
                tool_input: req.tool_input.clone(),
                is_mutating: true,
                external_idempotency_key: Some(idem_key.clone()),
            };

            if let Err(e) = effect::insert_effect(client, &new_effect).await {
                // If a conflict occurs, another worker already inserted.
                // Re-read and decide based on their status.
                if matches!(e, choruz_common::AppError::Conflict(_)) {
                    return self.handle_race_on_insert(client, req, &idem_key).await;
                }
                return Err(e.into());
            }
        }

        // Claim: pending -> executing.
        let claimed = effect::claim_effect(client, &req.tool_call_id).await?;
        if !claimed {
            // Another worker claimed it. Check what they did.
            return self.handle_race_on_insert(client, req, &idem_key).await;
        }

        // Execute the actual tool call.
        let exec_result = self
            .executor
            .execute(&req.tool_name, &req.tool_input, &idem_key)
            .await;

        match exec_result {
            Ok(output) => {
                effect::update_effect_result(
                    client,
                    &req.tool_call_id,
                    Some(&output),
                    EffectStatus::Succeeded,
                )
                .await?;

                Ok(ToolCallResponse {
                    output,
                    cached: false,
                    idempotency_key: idem_key,
                })
            }
            Err(err_msg) => {
                if let Err(e) = effect::update_effect_result(
                    client,
                    &req.tool_call_id,
                    None,
                    EffectStatus::Failed,
                )
                .await
                {
                    tracing::warn!(error = %e, "effect journal: failed to record result");
                }

                Err(ToolGatewayError::ExecutionFailed(err_msg))
            }
        }
    }

    /// Handle the case where we lost a race on INSERT (another worker already
    /// owns this tool_call_id).
    async fn handle_race_on_insert(
        &self,
        client: &tokio_postgres::Client,
        req: &ToolCallRequest,
        idem_key: &str,
    ) -> Result<ToolCallResponse, ToolGatewayError> {
        // Re-read the effect.
        let existing = effect::get_effect(client, &req.tool_call_id)
            .await?
            .ok_or_else(|| {
                ToolGatewayError::Internal("effect disappeared after conflict".into())
            })?;

        match existing.status {
            EffectStatus::Succeeded => Ok(ToolCallResponse {
                output: existing.tool_output.unwrap_or(serde_json::Value::Null),
                cached: true,
                idempotency_key: idem_key.to_owned(),
            }),
            EffectStatus::Executing => Err(ToolGatewayError::StillExecuting(req.tool_call_id)),
            _ => {
                // pending or failed from another worker -- back off
                Err(ToolGatewayError::StillExecuting(req.tool_call_id))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotency_key_format() {
        let id = ToolCallId::new();
        let key = ToolGateway::<NoopExecutor>::idempotency_key(&id);
        assert!(key.starts_with("echat:"));
        assert!(key.contains(&id.to_string()));
    }

    #[test]
    fn tool_call_request_serde() {
        let req = ToolCallRequest {
            tool_call_id: ToolCallId::new(),
            attempt_id: AttemptId::new(),
            turn_id: TurnId::new(),
            tool_name: "git_push".into(),
            tool_input: serde_json::json!({"branch": "main"}),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: ToolCallRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool_name, "git_push");
        assert_eq!(parsed.tool_call_id, req.tool_call_id);
    }

    #[test]
    fn tool_call_response_serde() {
        let resp = ToolCallResponse {
            output: serde_json::json!({"ok": true}),
            cached: true,
            idempotency_key: "echat:abc".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: ToolCallResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.cached);
        assert_eq!(parsed.idempotency_key, "echat:abc");
    }

    /// A no-op executor for unit tests that don't need DB.
    struct NoopExecutor;

    #[async_trait::async_trait]
    impl ToolExecutor for NoopExecutor {
        async fn execute(
            &self,
            _tool_name: &str,
            _input: &serde_json::Value,
            _idempotency_key: &str,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"noop": true}))
        }
    }

    #[test]
    fn gateway_construction() {
        let registry = crate::registry::default_registry();
        let _gw = ToolGateway::new(registry, NoopExecutor);
    }
}
