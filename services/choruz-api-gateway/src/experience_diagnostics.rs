//! Learning checks share one durable audit trace without copying native transcripts or account configuration.
use crate::{ApiState, host_runtime::RuntimeHost};
use choruz_application::db_service::ExperienceClaim;
use choruz_common::AppError;
use choruz_host_runtime::HostRequest;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) struct LearningCheck<'a> {
    pub state: &'a ApiState,
    pub claim: &'a ExperienceClaim,
    pub id: String,
    scope: std::sync::OnceLock<String>,
}

impl<'a> LearningCheck<'a> {
    pub fn new(state: &'a ApiState, claim: &'a ExperienceClaim) -> Self {
        Self {
            state,
            claim,
            id: choruz_common::new_id(),
            scope: std::sync::OnceLock::new(),
        }
    }

    pub fn scope_window(&self, window: &Value) -> Result<(), AppError> {
        let bytes = serde_json::to_vec(window).map_err(|e| AppError::Internal(e.to_string()))?;
        self.scope
            .set(digest(&bytes))
            .map_err(|_| AppError::Internal("Learning window scope was set twice".into()))
    }

    pub async fn record(&self, stage: &str, data: Value) -> Result<(), AppError> {
        let mut metadata = json!({
            "trace_id": self.id, "stage": stage,
            "analyst_binding_id": self.claim.analyst_binding_id,
            "policy_generation": self.claim.generation,
            "active_revision_id": self.claim.active_revision_id,
        });
        if let Some(data) = data.as_object() {
            metadata
                .as_object_mut()
                .expect("metadata object")
                .extend(data.clone());
        }
        self.state
            .db
            .record_audit(
                &self.claim.workspace_id,
                &self.claim.owner_id,
                "learning.check",
                "agent_binding",
                &self.claim.binding_id,
                metadata,
            )
            .await
    }

    pub async fn call<T: DeserializeOwned + Serialize>(
        &self,
        host: &RuntimeHost,
        stage: &str,
        request: HostRequest,
    ) -> Result<T, AppError> {
        let checkpointed = matches!(
            &request,
            HostRequest::AnalyzeExperience { .. }
                | HostRequest::ReviewExperience { .. }
                | HostRequest::ResearchExperience { .. }
                | HostRequest::ReviewTasks { .. }
                | HostRequest::EvaluateExperience { .. }
        );
        let checkpoint = if checkpointed {
            // Hash the complete private execution context without storing it.
            // Procedure changes cannot reuse an old model answer.
            let mut request_key =
                serde_json::to_value(&request).map_err(|e| AppError::Internal(e.to_string()))?;
            if let Some(spec) = request_key.get_mut("spec") {
                spec["harness_account"] = execution_context(match &request {
                    HostRequest::AnalyzeExperience { spec, .. }
                    | HostRequest::ReviewExperience { spec, .. }
                    | HostRequest::ResearchExperience { spec, .. }
                    | HostRequest::ReviewTasks { spec, .. }
                    | HostRequest::EvaluateExperience { spec, .. } => spec,
                    _ => unreachable!(),
                });
            }
            let encoded = serde_json::to_vec(&(
                request_key,
                self.scope.get(),
                choruz_learning::ANALYSIS_SKILL,
                choruz_learning::REVIEW_SKILL,
                choruz_learning::TASK_REVIEW_SKILL,
            ))
            .map_err(|e| AppError::Internal(format!("encode learning stage key: {e}")))?;
            let key = digest(&encoded);
            if let Some(value) = self
                .state
                .db
                .begin_experience_stage(self.claim, stage, &key)
                .await?
            {
                self.record(stage, json!({"outcome":"checkpoint_reused"}))
                    .await?;
                return serde_json::from_value(value)
                    .map_err(|e| AppError::Internal(format!("decode learning checkpoint: {e}")));
            }
            Some(key)
        } else {
            None
        };
        // Never serialize TerminalSpec: it contains device-local account configuration.
        let input = match &request {
            HostRequest::AnalyzeExperience { prompt, .. }
            | HostRequest::ReviewExperience { prompt, .. }
            | HostRequest::ReviewTasks { prompt, .. } => {
                json!({"input_digest":digest(prompt.as_bytes()),"input_bytes":prompt.len()})
            }
            HostRequest::EvaluateExperience { input, .. } => {
                json!({"input_digest":digest(input.as_bytes()),"input_bytes":input.len()})
            }
            HostRequest::BuildDecisionProgram { training, .. } => {
                let encoded =
                    serde_json::to_vec(training).map_err(|e| AppError::Internal(e.to_string()))?;
                json!({"input_digest":digest(&encoded),"input_bytes":encoded.len()})
            }
            HostRequest::Decision { request } => {
                let encoded =
                    serde_json::to_vec(request).map_err(|e| AppError::Internal(e.to_string()))?;
                json!({"input_digest":digest(&encoded),"input_bytes":encoded.len(),"model":request.model,"kind":request.kind()})
            }
            _ => json!({}),
        };
        let call_id = choruz_common::new_id();
        self.record(
            stage,
            json!({"outcome":"started","call_id":call_id,"input":input}),
        )
        .await?;
        let started = std::time::Instant::now();
        let requires_decision_consent = matches!(
            &request,
            HostRequest::Decision { .. } | HostRequest::BuildDecisionProgram { .. }
        );
        let result: Result<T, AppError> = if requires_decision_consent
            && !self.state.db.decision_claim_current(self.claim).await?
        {
            Err(AppError::Conflict(
                "Decision learning was disabled or its claim expired before dispatch".into(),
            ))
        } else {
            host.call(request).await
        };
        let result: Result<T, AppError> = result.and_then(|value| {
            serde_json::to_value(value)
                .map(choruz_activity::sanitize_value)
                .and_then(serde_json::from_value)
                .map_err(|e| AppError::Internal(format!("redact learning stage response: {e}")))
        });
        if let Some(key) = checkpoint {
            let encoded = result
                .as_ref()
                .ok()
                .map(serde_json::to_value)
                .transpose()
                .map_err(|e| AppError::Internal(format!("encode learning checkpoint: {e}")))?;
            // A validation/auth failure is an observed response. Transport and
            // interruption errors may have incurred a charge and need consent.
            let retryable = matches!(
                &result,
                Err(AppError::Validation(_)
                    | AppError::Unauthorized(_)
                    | AppError::Forbidden(_)
                    | AppError::NotFound(_))
            );
            self.state
                .db
                .finish_experience_stage(self.claim, stage, &key, encoded.as_ref(), retryable)
                .await?;
        }
        let details = match &result {
            Ok(value) => {
                let encoded = serde_json::to_vec(value)
                    .map_err(|e| AppError::Internal(format!("encode learning diagnostic: {e}")))?;
                json!({"outcome":"completed","output_digest":digest(&encoded),"output_bytes":encoded.len()})
            }
            Err(error) => failure(error),
        };
        self.record(
            stage,
            json!({"call_id":call_id,"duration_ms":started.elapsed().as_millis(),"result":details}),
        )
        .await?;
        result
    }
}

pub(crate) fn execution_context(spec: &choruz_host_runtime::TerminalSpec) -> Value {
    json!(choruz_host_runtime::execution_identity::ExecutionIdentity::from(spec))
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn failure(error: &AppError) -> Value {
    let category = match error {
        AppError::Validation(_) => "validation",
        AppError::Conflict(_) => "conflict",
        AppError::Unauthorized(_) => "unauthorized",
        AppError::Forbidden(_) => "forbidden",
        AppError::NotFound(_) => "not_found",
        _ => "internal",
    };
    json!({"outcome":"failed","error_category":category})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_diagnostics_do_not_copy_unstructured_device_errors() {
        for error in [
            AppError::Internal("private source sentence without a secret marker".into()),
            AppError::Validation("private source sentence without a secret marker".into()),
        ] {
            let details = failure(&error);
            assert_eq!(details["outcome"], "failed");
            assert!(details["error_category"].is_string());
            assert!(details.get("error").is_none());
            assert!(!details.to_string().contains("private source"));
        }
    }
}
