//! Shared evaluation execution. Callers own authorization, durable action reservations
//! and committing the updated search; an error never authorizes retry or activation.
use choruz_common::AppError;
use choruz_evaluation::{
    evaluation::{EvaluationCandidate, EvaluationCase, EvaluationSuite, JudgeResult},
    optimization::{Optimization, SearchAction},
};
use serde_json::{Value, json};

/// Execution boundaries, not alternative evaluation policies. Each judge must run
/// independently of candidate instructions. Implementations enforce isolation,
/// permissions, budgets and cancellation on the selected execution device.
pub trait EvaluationExecutor: Sync {
    fn prepare(
        &self,
        candidate: &EvaluationCandidate,
        input: &str,
    ) -> impl Future<Output = Result<String, AppError>> + Send;
    fn execute(
        &self,
        candidate: &EvaluationCandidate,
        case: &EvaluationCase,
        preflight: &str,
    ) -> impl Future<Output = Result<ExecutionOutput, AppError>> + Send;
    fn judge(
        &self,
        case: &EvaluationCase,
        output: &str,
    ) -> impl Future<Output = Result<JudgeResult, AppError>> + Send;
}

/// Replay metadata is retained in the report. `checks_passed` must come from
/// independent environment checks, never the candidate's self-assessment.
pub struct ExecutionOutput {
    pub output: String,
    pub replay: Option<Value>,
    pub checks_passed: bool,
}

/// Calls the fixed proposal procedure on the selected analyst. The result must
/// satisfy `crate::propose`; transport adapters return its decoded text.
pub trait ProposalExecutor: Sync {
    fn propose(&self, prompt: String) -> impl Future<Output = Result<String, AppError>> + Send;
}

/// Score through the fixed task contract. An inconclusive judge stays unscored.
pub async fn assess_output(
    executor: &impl EvaluationExecutor,
    case: &EvaluationCase,
    output: &str,
) -> Result<(Option<f64>, Option<JudgeResult>), AppError> {
    let judge = if case.check.model_calls() > 0 {
        Some(executor.judge(case, output).await?)
    } else {
        None
    };
    let score = match &judge {
        Some(result) => {
            result.validate().map_err(AppError::Validation)?;
            result.score()
        }
        None => case.check.score(output),
    };
    Ok((score, judge))
}

/// Execute one candidate and independently assess it. No checkpoint or guidance
/// is persisted here; the caller commits the report under its reservation.
pub async fn evaluate_task(
    executor: &impl EvaluationExecutor,
    candidate: &EvaluationCandidate,
    case: &EvaluationCase,
) -> Result<Value, AppError> {
    let preflight = if candidate.team.is_some() {
        executor.prepare(candidate, &case.input).await?
    } else {
        String::new()
    };
    let execution = executor.execute(candidate, case, &preflight).await?;
    if execution.output.len() > 16_000 {
        return Err(AppError::Validation(
            "Evaluation output exceeds its limit".into(),
        ));
    }
    let (mut score, judge) = assess_output(executor, case, &execution.output).await?;
    if !execution.checks_passed {
        score = Some(0.0);
    }
    Ok(
        json!({"status":if score.is_some() {"completed"} else {"inconclusive"},
        "case_id":case.id,"split":case.split,"revision_id":candidate.revision_id,
        "score":score,"output":execution.output,"preflight":preflight,
        "judge":judge,"replay":execution.replay}),
    )
}

/// Execute exactly the already-reserved optimizer action. The proposer receives
/// training evidence only; the unchanged suite remains outside its conversation.
/// Persist the returned search and report atomically before dispatching another action.
pub async fn optimization_step(
    executor: &impl EvaluationExecutor,
    proposer: &impl ProposalExecutor,
    search: &mut Optimization,
    suite: &EvaluationSuite,
    community_seed: Option<&Value>,
) -> Result<Value, AppError> {
    let action = search
        .pending
        .clone()
        .ok_or_else(|| AppError::Validation("Optimization action is missing".into()))?;
    match action {
        SearchAction::Evaluate { candidate, case } => {
            let guidance = search
                .candidates
                .get(candidate)
                .ok_or_else(|| AppError::Validation("Optimization candidate is missing".into()))?;
            let task = suite
                .cases
                .get(case)
                .ok_or_else(|| AppError::Validation("Optimization case is missing".into()))?;
            let mut report = evaluate_task(executor, &guidance.guidance, task).await?;
            if report["status"] == "inconclusive" {
                return Ok(report);
            }
            search
                .complete_scored_rollout(
                    report["output"]
                        .as_str()
                        .ok_or_else(|| AppError::Validation("Evaluation output is missing".into()))?
                        .to_owned(),
                    report["preflight"].as_str().unwrap_or_default().to_owned(),
                    report["score"].as_f64().ok_or_else(|| {
                        AppError::Validation("Evaluation judge was inconclusive".into())
                    })?,
                    serde_json::from_value(report["judge"].clone())
                        .map_err(|e| AppError::Validation(format!("Invalid assessment: {e}")))?,
                )
                .map_err(AppError::Validation)?;
            report["action"] = json!("evaluate");
            report["candidate"] = json!(candidate);
            report["case"] = json!(case);
            Ok(report)
        }
        SearchAction::Propose {
            parents, component, ..
        } => {
            let mut input = search.proposal_input(suite).map_err(AppError::Validation)?;
            if let Some(seed) = community_seed
                && seed["community_trials_enabled"] == true
            {
                input["behavior_sources"] = seed["validation"]["behavior_sources"].clone();
            }
            let text = proposer
                .propose(format!("{}\n\n{input}", crate::PROPOSAL_SKILL))
                .await?;
            search
                .complete_proposal(text)
                .map_err(AppError::Validation)?;
            Ok(
                json!({"status":"completed","action":"propose","parents":parents,"component":component}),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use choruz_evaluation::{
        evaluation::{EvaluationSplit, JudgeVerdict, OutputCheck},
        optimization::OptimizationConfig,
    };

    struct Fixture(JudgeVerdict);
    impl EvaluationExecutor for Fixture {
        async fn prepare(&self, _: &EvaluationCandidate, _: &str) -> Result<String, AppError> {
            panic!("no team in this fixture")
        }
        async fn execute(
            &self,
            _: &EvaluationCandidate,
            case: &EvaluationCase,
            _: &str,
        ) -> Result<ExecutionOutput, AppError> {
            assert!(case.input.starts_with("Add 7 and 8."));
            Ok(ExecutionOutput {
                output: "15".into(),
                replay: None,
                checks_passed: true,
            })
        }
        async fn judge(&self, _: &EvaluationCase, output: &str) -> Result<JudgeResult, AppError> {
            assert_eq!(output, "15");
            Ok(JudgeResult {
                verdict: self.0,
                reason: "Independent fixture judgment".into(),
            })
        }
    }
    impl ProposalExecutor for Fixture {
        async fn propose(&self, _: String) -> Result<String, AppError> {
            panic!("no proposal reserved in this fixture")
        }
    }

    fn search() -> (EvaluationSuite, Optimization) {
        let suite = EvaluationSuite {
            name: "Arithmetic".into(),
            cases: [
                EvaluationSplit::Train,
                EvaluationSplit::Validation,
                EvaluationSplit::Test,
            ]
            .into_iter()
            .enumerate()
            .map(|(i, split)| EvaluationCase {
                id: i.to_string(),
                split,
                input: format!("Add 7 and 8. Case {i}"),
                source: None,
                environment: None,
                check: OutputCheck::Judge {
                    expected: "15".into(),
                    rubric: "Correct sum".into(),
                },
            })
            .collect(),
        };
        let search = Optimization::new(
            OptimizationConfig {
                max_metric_calls: 24,
                max_proposals: 1,
                minibatch_size: 1,
                seed: 7,
                merge: false,
                cache_evaluations: false,
                evolve_team: false,
                max_agents: 4,
            },
            &suite,
            vec![
                EvaluationCandidate {
                    revision_id: None,
                    instruction: "Check arithmetic".into(),
                    team: None,
                },
                EvaluationCandidate {
                    revision_id: None,
                    instruction: "Check the sum independently".into(),
                    team: None,
                },
            ],
        )
        .unwrap();
        (suite, search)
    }

    #[tokio::test]
    async fn reserved_action_records_conclusive_scores_but_not_uncertain_judgments() {
        let (suite, mut search) = search();
        search.next_action(&suite).unwrap().unwrap();
        let before = json!(search);
        let inconclusive = Fixture(JudgeVerdict::Inconclusive);
        let report = optimization_step(&inconclusive, &inconclusive, &mut search, &suite, None)
            .await
            .unwrap();
        assert_eq!(report["status"], "inconclusive");
        assert_eq!(
            json!(search),
            before,
            "caller must reconcile the uncertain reservation"
        );

        // A separate search models a separately authorized run, not an automatic retry.
        let (_, mut conclusive) = self::search();
        conclusive.next_action(&suite).unwrap().unwrap();
        let fixture = Fixture(JudgeVerdict::Pass);
        let report = optimization_step(&fixture, &fixture, &mut conclusive, &suite, None)
            .await
            .unwrap();
        assert_eq!(report["score"], 1.0);
        assert_eq!(conclusive.observations.len(), 1);
        assert_eq!(conclusive.observations[0].output, "15");
        assert_eq!(conclusive.observations[0].score, 1.0);
        assert!(conclusive.pending.is_none());
    }

    #[tokio::test]
    async fn malformed_reservations_return_errors_without_changing_the_checkpoint() {
        let (suite, mut search) = search();
        let fixture = Fixture(JudgeVerdict::Pass);
        for action in [
            None,
            Some(SearchAction::Evaluate {
                candidate: 99,
                case: 0,
            }),
            Some(SearchAction::Evaluate {
                candidate: 0,
                case: 99,
            }),
        ] {
            search.pending = action;
            let before = json!(search);
            assert!(
                optimization_step(&fixture, &fixture, &mut search, &suite, None)
                    .await
                    .is_err()
            );
            assert_eq!(json!(search), before);
        }
    }
}
