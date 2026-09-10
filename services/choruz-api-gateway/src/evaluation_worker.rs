use crate::{
    ApiState,
    handlers_terminals::{authorize_terminal_binding, terminal_spec},
    host_runtime::RuntimeHost,
};
use choruz_application::db_service::EvaluationClaim;
use choruz_common::AppError;
use choruz_domain::{
    evaluation::{EvaluationCandidate, EvaluationCase},
    optimization::SearchAction,
};
use choruz_host_runtime::{HostRequest, TerminalSpec};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const PROPOSAL: &str = include_str!("../../../agent-templates/experience-proposal.md");
const APPLICATION_REVIEW: &str =
    include_str!("../../../agent-templates/experience-application-review.md");
const ANALYSIS: &str = include_str!("../../../agent-templates/experience-analysis.md");

pub(crate) fn analyst_fingerprint(spec: &TerminalSpec) -> Result<String, AppError> {
    Ok(hex::encode(Sha256::digest(
        format!(
            "{}:{PROPOSAL}:{APPLICATION_REVIEW}:{ANALYSIS}",
            fingerprint(spec)?
        )
        .as_bytes(),
    )))
}

pub(crate) fn fingerprint(spec: &TerminalSpec) -> Result<String, AppError> {
    if spec
        .model
        .as_ref()
        .is_none_or(|model| model.trim().is_empty())
    {
        return Err(AppError::Validation(
            "Select an explicit model before evaluating guidance".into(),
        ));
    }
    let context = json!({"driver":spec.driver_type,"model":spec.model,"binary":spec.binary_path,
        "workspace":spec.workspace_path,"host":spec.harness_account["runtime_host_id"],
        "account":spec.harness_account["harness_account_id"],
        "profile":spec.harness_account["harness_account_profile_kind"]});
    Ok(hex::encode(Sha256::digest(context.to_string().as_bytes())))
}

pub(crate) async fn run(state: &ApiState) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ticks = 0u8;
    loop {
        interval.tick().await;
        if ticks == 0 {
            match state.db.pending_experience_optimizations().await {
                Ok(jobs) => {
                    for job in jobs {
                        let result = queue_automatic(state, &job).await;
                        if let Err(error) = state
                            .db
                            .optimization_queue_error(&job, result.is_err())
                            .await
                        {
                            tracing::warn!(%error, "optimization queue diagnostic unavailable");
                        }
                        if let Err(error) = result {
                            tracing::warn!(binding_id=?job["binding"], %error, "automatic optimization could not queue");
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "automatic optimization queue unavailable"),
            }
        }
        ticks = (ticks + 1) % 15;
        if let Err(error) = step(state).await {
            tracing::warn!(%error, "evaluation queue unavailable");
        }
    }
}

async fn queue_automatic(state: &ApiState, job: &Value) -> Result<(), AppError> {
    let field = |name: &str| {
        job[name]
            .as_str()
            .ok_or_else(|| AppError::Internal(format!("Missing optimization {name}")))
    };
    let actor = state.db.get_principal(field("owner")?).await?;
    let target = authorize_terminal_binding(state, &actor, field("binding")?)
        .await
        .map_err(|e| e.0)?;
    let analyst = authorize_terminal_binding(state, &actor, field("analyst")?)
        .await
        .map_err(|e| e.0)?;
    let settings: choruz_domain::optimization::OptimizationSettings =
        serde_json::from_value(job["settings"].clone())
            .map_err(|e| AppError::Internal(e.to_string()))?;
    state
        .db
        .queue_experience_evaluation(
            field("workspace")?,
            &actor.id,
            &target.id,
            field("revision")?,
            &settings.suite,
            &choruz_application::db_service::EvaluationContext {
                fingerprint: fingerprint(&terminal_spec(&target, 120, 40, None, None))?,
                automatic_generation: Some(
                    job["generation"].as_i64().ok_or_else(|| {
                        AppError::Internal("Missing optimization generation".into())
                    })?,
                ),
                optimization: Some(choruz_application::db_service::OptimizationSetup {
                    config: settings.config,
                    analyst_binding_id: analyst.id.clone(),
                    analyst_fingerprint: analyst_fingerprint(&terminal_spec(
                        &analyst, 120, 40, None, None,
                    ))?,
                }),
            },
        )
        .await?;
    Ok(())
}

async fn step(state: &ApiState) -> Result<(), AppError> {
    let Some(mut claim) = state.db.claim_experience_evaluation().await? else {
        return Ok(());
    };
    let started = std::time::Instant::now();
    let result = execute(state, &mut claim).await;
    let (mut report, error_code) = match result {
        Ok(report) => (report, None),
        Err(error) => {
            let code = match error {
                AppError::Forbidden(_) | AppError::Unauthorized(_) | AppError::NotFound(_) => {
                    "access_revoked"
                }
                AppError::Conflict(_) => "context_changed",
                _ => "execution_failed",
            };
            (json!({"status":"failed"}), Some(code))
        }
    };
    report["ordinal"] = json!(claim.next_case);
    report["duration_ms"] = json!(started.elapsed().as_millis() as u64);
    if let Some(code) = error_code {
        report["error_code"] = json!(code);
    }
    let committed = state
        .db
        .finish_experience_evaluation_case(&claim, &report, error_code)
        .await?;
    tracing::info!(evaluation_id=%claim.id, binding_id=%claim.binding_id, ordinal=claim.next_case, committed, error_code, "evaluation case finished");
    Ok(())
}

async fn execute(state: &ApiState, claim: &mut EvaluationClaim) -> Result<Value, AppError> {
    let actor = state.db.get_principal(&claim.owner_id).await?;
    let binding = authorize_terminal_binding(state, &actor, &claim.binding_id)
        .await
        .map_err(|e| e.0)?;
    let spec = terminal_spec(&binding, 120, 40, None, None);
    if fingerprint(&spec)? != claim.context_fingerprint {
        return Err(AppError::Conflict("Evaluation context changed".into()));
    }
    let host = RuntimeHost::for_binding(state, &binding)?;
    if let Some(search) = &mut claim.optimization {
        let analyst_id = claim
            .analyst_binding_id
            .as_deref()
            .ok_or_else(|| AppError::Validation("Optimization analyst is missing".into()))?;
        let analyst = authorize_terminal_binding(state, &actor, analyst_id)
            .await
            .map_err(|e| e.0)?;
        let analyst_spec = terminal_spec(&analyst, 120, 40, None, None);
        if Some(analyst_fingerprint(&analyst_spec)?) != claim.analyst_fingerprint {
            return Err(AppError::Conflict(
                "Optimization analyst context changed".into(),
            ));
        }
        if claim.final_review {
            let proposed = search.candidates[search
                .winner
                .ok_or_else(|| AppError::Validation("Missing winner".into()))?]
            .guidance
            .clone();
            let seeds = claim.candidates.clone();
            let evidence = state.db.evaluation_seed_evidence(claim).await?;
            let seed_reference = format!(
                "reviewed-seed:{}",
                seeds[1]
                    .revision_id
                    .as_deref()
                    .ok_or_else(|| AppError::Validation("Missing seed revision".into()))?
            );
            let review: choruz_host_runtime::experience::Review = RuntimeHost::for_binding(state,&analyst)?.call(HostRequest::ReviewExperience {
                spec: Box::new(analyst_spec),
                prompt: format!("{APPLICATION_REVIEW}\n{}", json!({"proposed_instruction":proposed.instruction,"proposed_team":proposed.team,"seeds":seeds,"seed_reference":seed_reference,"seed_evidence":evidence})),
            }).await?;
            let target = authorize_terminal_binding(state, &actor, &claim.binding_id)
                .await
                .map_err(|e| e.0)?;
            let analyst = authorize_terminal_binding(state, &actor, analyst_id)
                .await
                .map_err(|e| e.0)?;
            if fingerprint(&terminal_spec(&target, 120, 40, None, None))?
                != claim.context_fingerprint
                || Some(analyst_fingerprint(&terminal_spec(
                    &analyst, 120, 40, None, None,
                ))?) != claim.analyst_fingerprint
            {
                return Err(AppError::Conflict(
                    "Execution context changed during review".into(),
                ));
            }
            let cited_seed_evidence = review.evidence.iter().all(|reference| {
                reference == &seed_reference
                    || evidence["references"]
                        .as_array()
                        .is_some_and(|refs| refs.iter().any(|r| r == reference))
            });
            return Ok(json!({
                "action": "application_review",
                "review_passed": review.accepted && cited_seed_evidence,
                "review": review,
            }));
        }
        let action = search
            .pending
            .clone()
            .ok_or_else(|| AppError::Validation("Optimization action is missing".into()))?;
        return match action {
            SearchAction::Evaluate { candidate, case } => {
                let report = evaluate_task(
                    &host,
                    spec,
                    &search.candidates[candidate].guidance,
                    &claim.suite.cases[case],
                )
                .await?;
                let output = report["output"]
                    .as_str()
                    .ok_or_else(|| AppError::Validation("Evaluation output is missing".into()))?
                    .to_owned();
                search
                    .complete_rollout(
                        &claim.suite,
                        output,
                        report["preflight"].as_str().unwrap_or_default().to_owned(),
                    )
                    .map_err(AppError::Validation)?;
                Ok(
                    json!({"status":"completed","action":"evaluate","candidate":candidate,"case":case,"score":report["score"]}),
                )
            }
            SearchAction::Propose {
                parents, component, ..
            } => {
                let input = search
                    .proposal_input(&claim.suite)
                    .map_err(AppError::Validation)?;
                let text: String = RuntimeHost::for_binding(state, &analyst)?
                    .call(HostRequest::ProposeExperience {
                        spec: Box::new(analyst_spec),
                        prompt: format!("{PROPOSAL}\n\n{}", input),
                    })
                    .await?;
                search
                    .complete_proposal(text)
                    .map_err(AppError::Validation)?;
                Ok(
                    json!({"status":"completed","action":"propose","parents":parents,"component":component}),
                )
            }
        };
    }
    // Alternate old/new on the same case rather than comparing two distant batches.
    let candidate = &claim.candidates[claim.next_case % claim.candidates.len()];
    let case = &claim.suite.cases[claim.next_case / claim.candidates.len()];
    evaluate_task(&host, spec, candidate, case).await
}

async fn evaluate_task(
    host: &RuntimeHost,
    spec: TerminalSpec,
    candidate: &EvaluationCandidate,
    case: &EvaluationCase,
) -> Result<Value, AppError> {
    let preflight = match &candidate.team {
        Some(focus) => {
            host.call::<String>(HostRequest::PrepareExecutionTeam {
                spec: Box::new(spec.clone()),
                role: choruz_host_runtime::harness::ExecutionTeam {
                    revision_id: candidate
                        .revision_id
                        .clone()
                        .unwrap_or_else(|| "baseline".into()),
                    team: focus.clone(),
                },
                request: case.input.clone(),
            })
            .await?
        }
        None => String::new(),
    };
    let output: Value = host
        .call(HostRequest::EvaluateExperience {
            spec: Box::new(spec),
            input: case.input.clone(),
            instruction: candidate.instruction.clone(),
            preflight: preflight.clone(),
        })
        .await?;
    let output = output
        .as_str()
        .ok_or_else(|| AppError::Validation("Invalid evaluation output".into()))?;
    if output.len() > 16_000 {
        return Err(AppError::Validation(
            "Evaluation output exceeds its limit".into(),
        ));
    }
    Ok(
        json!({"status":"completed","case_id":case.id,"split":case.split,"revision_id":candidate.revision_id,"score":case.check.score(output),"output":output,"preflight":preflight}),
    )
}
