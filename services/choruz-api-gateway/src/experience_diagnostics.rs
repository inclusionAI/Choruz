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
}

impl<'a> LearningCheck<'a> {
    pub fn new(state: &'a ApiState, claim: &'a ExperienceClaim) -> Self {
        Self {
            state,
            claim,
            id: choruz_common::new_id(),
        }
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
        // Never serialize TerminalSpec: it contains device-local account configuration.
        let input = match &request {
            HostRequest::AnalyzeExperience { prompt, .. }
            | HostRequest::ReviewTasks { prompt, .. } => {
                json!({"input_digest":digest(prompt.as_bytes()),"input_bytes":prompt.len()})
            }
            HostRequest::EvaluateExperience { input, .. } => {
                json!({"input_digest":digest(input.as_bytes()),"input_bytes":input.len()})
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
        let result: Result<T, AppError> = host.call(request).await;
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
