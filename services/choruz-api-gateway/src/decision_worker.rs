//! Optional advisory decisions share the source claim and revision transaction.
use crate::{
    experience_diagnostics::{LearningCheck, failure},
    host_runtime::RuntimeHost,
};
use choruz_application::db_service::ExperienceClaim;
use choruz_common::AppError;
use choruz_decision::task::{DecisionTask, Job};
use choruz_host_runtime::HostRequest;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn training_examples(
    cases: &[choruz_evaluation::evaluation::EvaluationCase],
    records: &[Value],
) -> Value {
    use choruz_evaluation::evaluation::EvaluationSplit;
    let protected: std::collections::BTreeSet<_> = cases
        .iter()
        .filter(|case| case.split != EvaluationSplit::Train)
        .filter_map(|case| case.source.as_ref())
        .flat_map(|source| std::iter::once(&source.episode_ref).chain(&source.evidence))
        .collect();
    json!(
        cases
            .iter()
            .filter(|case| case.split == EvaluationSplit::Train)
            .map(|case| {
                let evidence: Vec<_> = case
                    .source
                    .as_ref()
                    .into_iter()
                    .flat_map(|source| &source.evidence)
                    .filter(|reference| !protected.contains(reference))
                    .filter_map(|reference| {
                        records.iter().find(|record| record["ref"] == *reference)
                    })
                    .collect();
                json!({"input":case.input,"check":case.check,"action_evidence":evidence})
            })
            .collect::<Vec<_>>()
    )
}

/// Generation sees training objectives only. Evaluation answers and feedback
/// never return to the builder; a corpus is tried once, not mined repeatedly.
pub(crate) async fn build(
    state: &crate::ApiState,
    claim: &ExperienceClaim,
    check: &LearningCheck<'_>,
    decision_host: &RuntimeHost,
    cases: &[choruz_evaluation::evaluation::TraceCase],
    changes: &[choruz_evaluation::evaluation::TraceCase],
    records: &[Value],
) -> Result<Value, AppError> {
    let Some(settings) = &claim.decision_settings else {
        return Ok(Value::Null);
    };
    let Some(builder_id) = &settings.builder_binding_id else {
        return Ok(Value::Null);
    };
    let current: std::collections::BTreeMap<_, _> = cases
        .iter()
        .chain(changes)
        .map(|case| (case.episode_ref.clone(), case.clone()))
        .collect();
    let current: Vec<_> = current.into_values().collect();
    let Some(suite) =
        choruz_application::db_service::measured_trace_suite(&current, 24, &Default::default())
    else {
        return Ok(json!({"status":"insufficient_independent_objectives"}));
    };
    let corpus = hex::encode(Sha256::digest(
        serde_json::to_vec(&suite).map_err(|e| AppError::Internal(e.to_string()))?,
    ));
    let actor = state.db.get_principal(&claim.owner_id).await?;
    let builder = crate::handlers_terminals::authorize_terminal_binding(state, &actor, builder_id)
        .await
        .map_err(|e| e.0)?;
    if builder.driver_type.as_str() != "codex_terminal" {
        return Err(AppError::Validation(
            "Select a Codex program builder".into(),
        ));
    }
    let host = RuntimeHost::for_binding(state, &builder)?;
    let target =
        crate::handlers_terminals::authorize_terminal_binding(state, &actor, &claim.binding_id)
            .await
            .map_err(|e| e.0)?;
    let judge_spec = crate::handlers_terminals::terminal_spec(&target, 120, 40, None, None);
    if !state
        .db
        .reserve_decision_program_trial(claim, &corpus)
        .await?
    {
        return Ok(json!({"status":"corpus_reserved_or_completed"}));
    }
    let spec = crate::handlers_terminals::terminal_spec(&builder, 120, 40, None, None);
    let training = training_examples(&suite.cases, records);
    let generated: Result<Option<choruz_learning::GeneratedProgram>, AppError> = check
        .call(
            &host,
            "program_generation",
            HostRequest::BuildDecisionProgram {
                spec: Box::new(spec.clone()),
                training,
            },
        )
        .await;
    let mut program = match generated {
        Ok(Some(choruz_learning::GeneratedProgram::Finite(program))) => program,
        Ok(Some(choruz_learning::GeneratedProgram::Browser(workflow))) => {
            return Ok(
                json!({"corpus":corpus,"status":"browser_ready_for_current_task","workflow":workflow,"model":settings.model}),
            );
        }
        Ok(None) => return Ok(json!({"corpus":corpus,"status":"not_suitable"})),
        Err(error) => {
            return Ok(
                json!({"corpus":corpus,"status":"generation_failed","failure":failure(&error)}),
            );
        }
    };
    program.minimum_confidence = program.minimum_confidence.max(settings.minimum_confidence);
    let mut results = Vec::new();
    for case in &suite.cases {
        if !state.db.decision_claim_current(claim).await? {
            return Err(AppError::Conflict(
                "Decision learning was disabled or its lease expired".into(),
            ));
        }
        let result: Result<choruz_decision::programs::ProgramResult, AppError> = check
            .call(
                decision_host,
                "program_evaluation",
                HostRequest::Decision {
                    request: DecisionTask {
                        model: settings.model.clone(),
                        minimum_confidence: settings.minimum_confidence,
                        job: Job::Program {
                            program: program.clone(),
                            state: json!(case.input),
                        },
                    },
                },
            )
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                results
                    .push(json!({"case_id":case.id,"split":case.split,"failure":failure(&error)}));
                break;
            }
        };
        let assessment = if let Some(output) = &result.output {
            if !state.db.decision_claim_current(claim).await? {
                return Err(AppError::Conflict(
                    "Decision learning changed during evaluation".into(),
                ));
            }
            crate::evaluation_worker::assess_output(decision_host, judge_spec.clone(), case, output)
                .await
        } else {
            Ok((None, None))
        };
        match assessment {
            Ok((score, judge)) => results.push(json!({"case_id":case.id,"split":case.split,"score":score,"judge":judge,"result":result})),
            Err(error) => {
                results.push(json!({"case_id":case.id,"split":case.split,"failure":failure(&error)}));
                break;
            }
        }
    }
    let resolved_model = results
        .first()
        .and_then(|r| r["result"]["decision"]["model"].as_str());
    let passed = results.len() == suite.cases.len()
        && results.iter().all(|r| {
            r["score"] == 1.0
                && resolved_model.is_some()
                && r["result"]["decision"]["model"].as_str() == resolved_model
        });
    Ok(
        json!({"corpus":corpus,"status":if passed {"validated"} else {"not_validated"},
        "program":program,"model":settings.model,"resolved_model":resolved_model,"suite":suite,"results":results}),
    )
}

pub(crate) async fn annotate(
    host: &RuntimeHost,
    claim: &ExperienceClaim,
    check: &LearningCheck<'_>,
    source: &Value,
) -> Result<Value, AppError> {
    let Some(settings) = &claim.decision_settings else {
        return Ok(Value::Null);
    };
    settings
        .validate()
        .map_err(|e| AppError::Validation(e.to_string()))?;
    let mut results = serde_json::Map::new();
    for (enabled, job) in [
        (
            settings.classify,
            Job::Classify {
                state: source.clone(),
            },
        ),
        (
            settings.supervise,
            Job::Supervise {
                state: source.clone(),
            },
        ),
    ] {
        if !enabled {
            continue;
        }
        let request = DecisionTask {
            model: settings.model.clone(),
            minimum_confidence: settings.minimum_confidence,
            job,
        };
        let kind = request.kind();
        let decision: Result<Value, AppError> = check
            .call(
                host,
                &format!("decision_{kind}"),
                HostRequest::Decision { request },
            )
            .await;
        // Optional provider outages must not discard the independent analysis.
        // Persist the failure alongside this revision, never as a valid answer.
        results.insert(
            kind.into(),
            match decision {
                Ok(answer) => json!({"status":"completed","advisory":true,"answer":answer}),
                Err(error) => failure(&error),
            },
        );
    }
    Ok(Value::Object(results))
}

#[cfg(test)]
mod tests {
    use super::*;
    use choruz_evaluation::evaluation::{EvaluationCase, EvaluationSplit, OutputCheck, TraceCase};

    #[test]
    fn builder_receives_only_referenced_training_actions_not_heldout_or_shared_records() {
        let cases: Vec<_> = [EvaluationSplit::Train, EvaluationSplit::Test]
            .into_iter()
            .enumerate()
            .map(|(index, split)| EvaluationCase {
                environment: None,
                source: Some(TraceCase {
                    variant: None,
                    group_ref: None,
                    classification: None,
                    episode_ref: format!("task-{index}"),
                    evidence: vec![format!("action-{index}"), "shared".into()],
                    input: format!("task {index}"),
                    check: None,
                    reason: "verified".into(),
                }),
                id: index.to_string(),
                split,
                input: format!("task {index}"),
                check: OutputCheck::Exact {
                    expected: format!("answer {index}"),
                },
            })
            .collect();
        let examples = training_examples(
            &cases,
            &[
                json!({"ref":"action-0","record":{"tool":"click Save"}}),
                json!({"ref":"action-1","record":"heldout-secret"}),
                json!({"ref":"shared","record":"cross-split-secret"}),
                json!({"ref":"unrelated","record":"unrelated-secret"}),
            ],
        );
        assert_eq!(
            examples,
            json!([{"input":"task 0","check":{"type":"exact","expected":"answer 0"},
            "action_evidence":[{"ref":"action-0","record":{"tool":"click Save"}}]}])
        );
    }
}
