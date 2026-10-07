//! Native and remote execution share one evaluation adapter. This module owns
//! transport mapping, while `choruz_learning::workflow` owns evaluation policy.
use crate::{HostRequest, TerminalSpec};
use choruz_common::AppError;
use choruz_evaluation::evaluation::{EvaluationCandidate, EvaluationCase, JudgeResult};
use choruz_learning::workflow::{EvaluationExecutor, ExecutionOutput, ProposalExecutor};
use serde_json::Value;

/// Dispatches to an already-authorized device. Implementations must not reroute
/// a disconnected remote device to the local machine or automatically retry calls.
pub trait LearningHost: Sync {
    fn dispatch(
        &self,
        request: HostRequest,
    ) -> impl Future<Output = Result<Value, AppError>> + Send;
}

pub struct LocalLearningHost;

impl LearningHost for LocalLearningHost {
    async fn dispatch(&self, request: HostRequest) -> Result<Value, AppError> {
        crate::execute(request).await
    }
}

/// A caller-selected account and device; no Company, database or API is required.
pub struct LearningExecutor<'a, H> {
    pub host: &'a H,
    pub spec: TerminalSpec,
}

impl<H: LearningHost> LearningExecutor<'_, H> {
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        request: HostRequest,
    ) -> Result<T, AppError> {
        serde_json::from_value(self.host.dispatch(request).await?)
            .map_err(|e| AppError::Internal(format!("decode learning response: {e}")))
    }
}

impl<H: LearningHost> ProposalExecutor for LearningExecutor<'_, H> {
    async fn propose(&self, prompt: String) -> Result<String, AppError> {
        self.call(HostRequest::ProposeExperience {
            spec: Box::new(self.spec.clone()),
            prompt,
        })
        .await
    }
}

impl<H: LearningHost> EvaluationExecutor for LearningExecutor<'_, H> {
    async fn prepare(
        &self,
        candidate: &EvaluationCandidate,
        input: &str,
    ) -> Result<String, AppError> {
        let team = candidate
            .team
            .clone()
            .ok_or_else(|| AppError::Validation("Execution team is missing".into()))?;
        self.call(HostRequest::PrepareExecutionTeam {
            spec: Box::new(self.spec.clone()),
            role: crate::harness::ExecutionTeam {
                revision_id: candidate
                    .revision_id
                    .clone()
                    .unwrap_or_else(|| "baseline".into()),
                team,
            },
            request: input.to_owned(),
        })
        .await
    }

    async fn execute(
        &self,
        candidate: &EvaluationCandidate,
        case: &EvaluationCase,
        preflight: &str,
    ) -> Result<ExecutionOutput, AppError> {
        if let Some(environment) = &case.environment {
            let result: crate::evaluation_replay::ReplayResult = self
                .call(HostRequest::ReplayExperience {
                    spec: Box::new(self.spec.clone()),
                    input: case.input.clone(),
                    instruction: candidate.instruction.clone(),
                    preflight: preflight.to_owned(),
                    environment: environment.clone(),
                })
                .await?;
            let checks_passed = result.checks.iter().all(|check| check["passed"] == true);
            let output = result.output.clone();
            let replay = serde_json::to_value(result)
                .map_err(|e| AppError::Internal(format!("encode evaluation replay: {e}")))?;
            Ok(ExecutionOutput {
                output,
                replay: Some(replay),
                checks_passed,
            })
        } else {
            let output = self
                .call(HostRequest::EvaluateExperience {
                    spec: Box::new(self.spec.clone()),
                    input: case.input.clone(),
                    instruction: candidate.instruction.clone(),
                    preflight: preflight.to_owned(),
                })
                .await?;
            Ok(ExecutionOutput {
                output,
                replay: None,
                checks_passed: true,
            })
        }
    }

    async fn judge(&self, case: &EvaluationCase, output: &str) -> Result<JudgeResult, AppError> {
        self.call(HostRequest::JudgeExperience {
            spec: Box::new(self.spec.clone()),
            input: case.input.clone(),
            check: case.check.clone(),
            output: output.to_owned(),
        })
        .await
    }
}
