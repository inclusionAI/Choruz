//! Durable, opt-in analysis outside request handlers and foreground execution.
use crate::{
    ApiState,
    experience_diagnostics::{LearningCheck, failure},
    handlers_terminals::{authorize_terminal_binding, terminal_spec},
    host_runtime::RuntimeHost,
};
use choruz_application::db_service::{ExperienceClaim, ExperienceReport};
use choruz_common::AppError;
use choruz_host_runtime::{
    HostRequest,
    experience::{Analysis, ProblemObservation},
    experience_source::{Cursor, HistoricalRecord},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

const REVIEW: &str = include_str!("../../../agent-templates/experience-analysis.md");

pub(crate) struct WorkerGuard(tokio::task::AbortHandle);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) fn spawn(mut state: ApiState) -> std::sync::Arc<WorkerGuard> {
    state.experience_worker = None;
    let task = tokio::spawn(async move {
        tokio::join!(crate::evaluation_worker::run(&state), run_analysis(&state));
    });
    std::sync::Arc::new(WorkerGuard(task.abort_handle()))
}

async fn run_analysis(state: &ApiState) {
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        match state.db.claim_experience().await {
            Ok(Some(mut claim)) => {
                prepare_curation(&mut claim, chrono::Utc::now().timestamp());
                let check = LearningCheck::new(state, &claim);
                let outcome = async {
                    check.record("check", json!({"outcome":"started"})).await?;
                    review(state, &claim, &check).await
                }
                .await;
                let details = match &outcome {
                    Ok(()) => json!({"outcome":"completed"}),
                    Err(error) => failure(error),
                };
                if let Err(error) = check.record("check", details).await {
                    tracing::warn!(trace_id=%check.id, binding_id=%claim.binding_id, %error, "learning diagnostic persistence failed");
                }
                // Diagnostics never include the trace, model output or credentials.
                let error = outcome.err().map(|error| match error {
                        AppError::NotFound(message) | AppError::Conflict(message) | AppError::Validation(message) => message,
                        AppError::Forbidden(_) | AppError::Unauthorized(_) => "Learning no longer has access to the selected Agent or workspace.".into(),
                        _ => "Background analysis failed. Check the selected account and device; learning will retry.".into(),
                    });
                if let Err(error) = state.db.release_experience(&claim, error.as_deref()).await {
                    tracing::warn!(binding_id=%claim.binding_id, %error, "learning lease release failed");
                }
                tracing::info!(trace_id=%check.id, binding_id=%claim.binding_id, success=error.is_none(), "learning check finished");
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "learning queue unavailable"),
        }
    }
}

/// Revisit retained original evidence daily, after the preceding paged sweep
/// completes. The durable cursor makes restarts resume rather than restart it.
fn prepare_curation(claim: &mut ExperienceClaim, now: i64) {
    if !claim.trace_cases {
        return;
    }
    let started = claim.source_cursor["_curation_started"].as_i64();
    if started.is_none()
        || (claim.source_cursor["_more"] != true
            && started.is_some_and(|t| now.saturating_sub(t) >= 86400))
    {
        claim.source_cursor = json!({"_curation_started":now});
    }
}

fn finish_curation(
    checkpoint: &mut serde_json::Map<String, Value>,
    previous: &[choruz_domain::evaluation::TraceCase],
    changes: &mut Vec<choruz_domain::evaluation::TraceCase>,
    complete: bool,
) {
    let mut seen: std::collections::BTreeSet<String> = checkpoint
        .get("_curation_seen")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    // Only working-corpus entries need remembered validation; unrelated newly
    // discovered objectives must not evict one during a long paged sweep.
    seen.retain(|reference| previous.iter().any(|c| &c.episode_ref == reference));
    seen.extend(changes.iter().map(|c| c.episode_ref.clone()));
    if complete {
        for old in previous
            .iter()
            .filter(|c| c.check.is_some() && !seen.contains(&c.episode_ref))
        {
            let mut withdrawn = old.clone();
            withdrawn.check = None;
            withdrawn.reason =
                "Original evidence was not revalidated in the retained-source sweep".into();
            changes.push(withdrawn);
        }
    }
    checkpoint.insert(
        "_curation_seen".into(),
        json!(seen.into_iter().collect::<Vec<_>>()),
    );
}

async fn review(
    state: &ApiState,
    claim: &ExperienceClaim,
    check: &LearningCheck<'_>,
) -> Result<(), AppError> {
    let actor = state.db.get_principal(&claim.owner_id).await?;
    // Recheck access at execution time, not only when the user enabled learning.
    let target = authorize_terminal_binding(state, &actor, &claim.binding_id)
        .await
        .map_err(|e| e.0)?;
    let analyst = authorize_terminal_binding(state, &actor, &claim.analyst_binding_id)
        .await
        .map_err(|e| e.0)?;
    let source_host = RuntimeHost::for_binding(state, &target)?;
    let existing_cases = state
        .db
        .experience_trace_cases(&claim.workspace_id, &claim.binding_id)
        .await?;
    let anchor = target.valid_terminal_session_anchor_for_context(
        None,
        None,
        Some(target.terminal_generation()),
        None,
    );
    let home = anchor
        .as_ref()
        .map(|a| a.native_home_path.clone())
        .filter(|p| !p.is_empty());
    let mut checkpoint = claim.source_cursor.as_object().cloned().unwrap_or_default();
    let feedback = state.db.experience_feedback(claim).await?;
    let mut windows = vec![("feedback".to_owned(), feedback)];
    let mut native_sources = Vec::new();
    {
        let mut sessions = vec![
            anchor
                .as_ref()
                .map(|a| a.session_id.clone())
                .or_else(|| target.external_session_id.clone()),
        ];
        if target.external_session_id.is_some() && !sessions.contains(&target.external_session_id) {
            sessions.push(target.external_session_id.clone());
        }
        for resume in sessions {
            let key = resume.as_deref().unwrap_or("live");
            let cursor: choruz_host_runtime::experience_source::Cursor = checkpoint
                .get(key)
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| AppError::Internal("Invalid stored source cursor".into()))?
                .unwrap_or_default();
            let mut spec = terminal_spec(
                &target,
                120,
                40,
                resume.clone(),
                if anchor
                    .as_ref()
                    .is_some_and(|a| Some(&a.session_id) == resume.as_ref())
                {
                    home.clone()
                } else {
                    None
                },
            );
            let window: Value = check
                .call(
                    &source_host,
                    "source_read",
                    HostRequest::ExperienceTrace {
                        spec: Box::new(spec.clone()),
                        cursor: cursor.clone(),
                    },
                )
                .await?;
            if window["reset"] != true && cursor.offset > 0 {
                spec.resume_session_id = Some(cursor.session.clone());
                native_sources.push((spec, cursor));
            }
            windows.push((key.to_owned(), window));
        }
    }
    let guidance = windows
        .iter()
        .find_map(|(_, window)| window.get("project_guidance").cloned());
    let unread = windows.iter().any(|(_, window)| window["more"] == true);
    for (key, window) in &windows {
        if window["records"].as_array().is_some_and(Vec::is_empty) {
            checkpoint.insert(key.clone(), window["cursor"].clone());
        }
    }
    let mut pending = windows.into_iter().filter(|(_, window)| {
        window["records"]
            .as_array()
            .is_some_and(|records| !records.is_empty())
    });
    let known_problems = state.db.experience_problems(claim).await?;
    let next = pending.next().or_else(|| {
        let unfinished: Vec<_> = known_problems
            .as_array()?
            .iter()
            .filter(|problem| problem["prompt_revision_id"].is_null())
            .map(|problem| problem["key"].clone())
            .collect();
        (!unread && !unfinished.is_empty()).then(|| {
            (
                "_pending_problems".to_owned(),
                json!({"records":[],"cursor":unfinished,"more":false}),
            )
        })
    });
    let Some((key, mut source)) = next else {
        check
            .record(
                "source_selection",
                json!({"outcome":"no_new_records","unread":unread}),
            )
            .await?;
        checkpoint.insert("_more".into(), json!(unread));
        let mut withdrawals = Vec::new();
        if claim.trace_cases {
            finish_curation(&mut checkpoint, &existing_cases, &mut withdrawals, !unread);
        }
        let checkpoint = Value::Object(checkpoint);
        if checkpoint != claim.source_cursor || !withdrawals.is_empty() {
            let digest = hex::encode(Sha256::digest(checkpoint.to_string().as_bytes()));
            state
                .db
                .save_experience_candidate(
                    claim,
                    ExperienceReport {
                        digest: &digest,
                        references: &claim.source_references,
                        analysis: &claim.source_summary,
                        instruction: None,
                        validation: &json!({"review":"not_passed","source_only":true,"trace_id":check.id,"evaluation_cases":withdrawals}),
                        checkpoint: Some(&checkpoint),
                        activate: false,
                    },
                )
                .await?;
        }
        return Ok(());
    };
    // Do not activate guidance from feedback while older native work is unread.
    source["more"] = json!(unread || pending.next().is_some());
    if claim.trace_cases {
        source["curation_cycle"] = claim.source_cursor["_curation_started"].clone();
    }
    if let Some(guidance) = guidance {
        source["project_guidance"] = guidance;
    }
    checkpoint.insert(key, source["cursor"].clone());
    checkpoint.insert("_more".into(), source["more"].clone());
    let digest = hex::encode(Sha256::digest(source.to_string().as_bytes()));
    if state.db.experience_source_seen(claim, &digest).await? {
        check
            .record(
                "source_selection",
                json!({"outcome":"already_analyzed","source_digest":digest}),
            )
            .await?;
        return Ok(());
    }
    check.record("source_selection", json!({"outcome":"selected","source_digest":digest,"source_cursor":source["cursor"],"source_complete":source["more"] == false})).await?;
    let prompt = format!(
        "{REVIEW}\n\nInput data:\n{}",
        json!({
            "current_instruction": claim.instruction, "active_revision_id": claim.active_revision_id,
            "prior_summary":claim.source_summary, "prior_references":claim.source_references, "trace": source,
            "known_problems":known_problems,
            "collect_evaluation_cases":claim.trace_cases,
            "existing_evaluation_cases":existing_cases,
        })
    );
    let mut analysis: Analysis = check
        .call(
            &RuntimeHost::for_binding(state, &analyst)?,
            "analysis",
            HostRequest::AnalyzeExperience {
                spec: Box::new(terminal_spec(&analyst, 120, 40, None, None)),
                prompt,
            },
        )
        .await?;
    if !claim.trace_cases {
        analysis.evaluation_cases.clear();
    }
    let records = source["records"]
        .as_array()
        .ok_or_else(|| AppError::Validation("Learning source records are missing".into()))?;
    let retained_reference = |reference: &String| {
        records
            .iter()
            .any(|record| record["ref"].as_str() == Some(reference))
            || claim
                .source_references
                .as_array()
                .is_some_and(|refs| refs.iter().any(|r| r.as_str() == Some(reference)))
            || known_problems.as_array().is_some_and(|problems| {
                problems.iter().any(|problem| {
                    problem["episodes"].as_array().is_some_and(|episodes| {
                        episodes.iter().any(|episode| {
                            episode["episode_ref"] == *reference
                                || episode["evidence"]
                                    .as_array()
                                    .is_some_and(|refs| refs.iter().any(|r| r == reference))
                        })
                    })
                })
            })
    };
    // The cursor owns source continuity even when a no-change report cites
    // nothing. Recover only requested records from the original scoped
    // source; model-authored summary strings never establish provenance.
    let mut missing: Vec<_> = analysis
        .evidence
        .iter()
        .chain(analysis.problems.iter().flat_map(|problem| {
            std::iter::once(&problem.episode_ref).chain(problem.evidence.iter())
        }))
        .filter(|reference| !retained_reference(reference))
        .cloned()
        .collect();
    // A retained identifier establishes neither marker contents nor ordering.
    missing.extend(analysis.problems.iter().filter_map(|problem| {
        problem
            .applied_revision_ref
            .as_ref()
            .filter(|reference| !records.iter().any(|record| record["ref"] == **reference))
            .cloned()
    }));
    missing.extend(
        analysis
            .evaluation_cases
            .iter()
            .flat_map(|case| std::iter::once(&case.episode_ref).chain(case.evidence.iter()))
            .filter(|reference| !records.iter().any(|record| record["ref"] == **reference))
            .cloned(),
    );
    missing.sort();
    missing.dedup();
    let mut recovered = state
        .db
        .experience_feedback_references(claim, &missing)
        .await?;
    let current_start = native_sources.iter().find_map(|(_, cursor)| {
        (source["reset"] != true && source["cursor"]["session"] == cursor.session)
            .then(|| cursor.clone())
    });
    let mut historical = Vec::new();
    if !missing.is_empty() {
        for (spec, cursor) in native_sources {
            let requested: Vec<_> = missing
                .iter()
                .filter(|reference| reference.starts_with(&format!("{}:", cursor.session)))
                .cloned()
                .collect();
            if requested.is_empty() {
                continue;
            }
            let verified: Vec<HistoricalRecord> = check
                .call(
                    &source_host,
                    "source_recovery",
                    HostRequest::ExperienceReferences {
                        spec: Box::new(spec),
                        cursor,
                        references: requested,
                    },
                )
                .await?;
            recovered.extend(verified.iter().map(|record| record.reference.clone()));
            historical.extend(verified);
        }
    }
    let known_reference =
        |reference: &String| retained_reference(reference) || recovered.contains(reference);
    let mut evaluation_cases = analysis.evaluation_cases.clone();
    let mut task_quality = Value::Null;
    if !evaluation_cases.is_empty() {
        let known_cases: std::collections::BTreeSet<_> = existing_cases
            .iter()
            .chain(evaluation_cases.iter())
            .map(|c| c.episode_ref.clone())
            .collect();
        for case in &mut evaluation_cases {
            let classification_ok = case
                .classification
                .as_ref()
                .is_some_and(|c| c.related_refs.iter().all(|r| known_cases.contains(r)));
            let grounded = classification_ok
                && std::iter::once(&case.episode_ref)
                    .chain(case.evidence.iter())
                    .all(|reference| {
                        records.iter().any(|record| record["ref"] == *reference)
                            || historical
                                .iter()
                                .any(|record| record.reference == *reference)
                    });
            if !grounded {
                case.classification = existing_cases
                    .iter()
                    .find(|old| old.episode_ref == case.episode_ref)
                    .and_then(|old| old.classification.clone());
                case.check = None;
                case.reason =
                    "Dataset review could not establish a grounded, self-contained answer".into();
            }
        }
        task_quality = crate::task_quality::inspect(
            &RuntimeHost::for_binding(state, &analyst)?,
            terminal_spec(&analyst, 120, 40, None, None),
            &mut evaluation_cases,
            json!({"existing_cases":existing_cases,"records":records,"historical":historical}),
            check,
        )
        .await?;
        // Preserve previously reviewed equivalence links when an incremental
        // report omits them; reclassification cannot move exposed siblings apart.
        for case in &mut evaluation_cases {
            if let Some(classification) = &mut case.classification {
                if let Some(old) = existing_cases
                    .iter()
                    .find(|old| old.episode_ref == case.episode_ref)
                    .and_then(|old| old.classification.as_ref())
                {
                    classification
                        .related_refs
                        .extend(old.related_refs.iter().cloned());
                    classification.related_refs.sort();
                    classification.related_refs.dedup();
                    classification.related_refs.truncate(64);
                }
            }
        }
        check
            .record(
                "case_admission",
                json!({"accepted_count":evaluation_cases.iter().filter(|c|c.check.is_some()).count(),"case_count":evaluation_cases.len()}),
            )
            .await?;
    }
    if claim.trace_cases {
        finish_curation(
            &mut checkpoint,
            &existing_cases,
            &mut evaluation_cases,
            source["more"] != true,
        );
    }
    if !analysis.evidence.iter().all(known_reference) {
        return Err(AppError::Validation(
            "Analysis cited a record outside its source window".into(),
        ));
    }
    let mut problems = Vec::new();
    for problem in &analysis.problems {
        if !known_reference(&problem.episode_ref) {
            return Err(AppError::Validation("Learning objective opener is absent from its verified source; restore the original source to recover it".into()));
        }
        if !problem.evidence.iter().all(known_reference) {
            return Err(AppError::Validation(
                "Learning failure cites evidence absent from its verified source".into(),
            ));
        }
        if !problem
            .evidence
            .iter()
            .any(|reference| records.iter().any(|record| record["ref"] == *reference))
        {
            return Err(AppError::Validation(
                "Learning problem lacks new source evidence".into(),
            ));
        }
        let applied = match (&problem.applied_revision_ref, &claim.active_revision_id) {
            (Some(reference), Some(revision))
                if applied_before_failure(records, reference, revision, &problem.evidence)
                    || historical_applied_before_failure(
                        records,
                        current_start.as_ref(),
                        &historical,
                        reference,
                        revision,
                        &problem.evidence,
                    ) =>
            {
                Some(revision)
            }
            (None, _) => None,
            _ => {
                return Err(AppError::Validation(
                    "Learning observation lacks its claimed revision marker".into(),
                ));
            }
        };
        problems.push(json!({"key":problem.key,"description":problem.description,
            "episode_ref":problem.episode_ref,"evidence":problem.evidence,"applied_revision_id":applied}));
        analysis.evidence.push(problem.episode_ref.clone());
        analysis.evidence.extend(problem.evidence.iter().cloned());
        if let Some(reference) = &problem.applied_revision_ref {
            analysis.evidence.push(reference.clone());
        }
    }
    let escalation_candidates = repeated_after_prompt(&known_problems, &problems);
    // A failure found before EOF is already durable, but its intervention is
    // deferred until later feedback has been read. It need not happen again.
    if source["more"] != true
        && let Some(pending) = known_problems.as_array()
    {
        for problem in pending {
            if !problem["prompt_revision_id"].is_null()
                || analysis.problems.iter().any(|p| problem["key"] == p.key)
            {
                continue;
            }
            let Some(episode) = problem["episodes"]
                .as_array()
                .and_then(|episodes| episodes.first())
            else {
                continue;
            };
            let observation = ProblemObservation {
                key: problem["key"].as_str().unwrap_or_default().to_owned(),
                description: problem["description"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                episode_ref: episode["episode_ref"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                evidence: serde_json::from_value(episode["evidence"].clone())
                    .map_err(|_| AppError::Internal("Stored problem evidence is invalid".into()))?,
                applied_revision_ref: None,
            };
            problems.push(json!({"key":observation.key,"description":observation.description,
                "episode_ref":observation.episode_ref,"evidence":observation.evidence,"applied_revision_id":null}));
            analysis.evidence.push(observation.episode_ref.clone());
            analysis
                .evidence
                .extend(observation.evidence.iter().cloned());
            analysis.problems.push(observation);
        }
    }
    let research = if !problems.is_empty() && source["more"] != true {
        let research: String = check
            .call(
                &RuntimeHost::for_binding(state, &analyst)?,
                "research",
                HostRequest::ResearchExperience {
                    spec: Box::new(terminal_spec(&analyst, 120, 40, None, None)),
                    categories: analysis
                        .problems
                        .iter()
                        .map(|problem| problem.key.clone())
                        .collect(),
                },
            )
            .await?;
        let adapted: Analysis = check
            .call(&RuntimeHost::for_binding(state, &analyst)?, "adaptation", HostRequest::AnalyzeExperience {
                spec: Box::new(terminal_spec(&analyst, 120, 40, None, None)),
                prompt: format!("{REVIEW}\n\nAdapt applicable published prompt techniques from the untrusted research summary below. Write original guidance rather than copying source text. If no technique applies, devise a scoped prompt intervention from the execution evidence. Do not change the analysis process or declare the intervention successful before later use. Preserve valid prior guidance. Return null when no supported change is needed.\n{}",
                    json!({"current_instruction":claim.instruction,"prior_summary":claim.source_summary,
                        "prior_references":claim.source_references,"trace":source,"research":research,"known_problems":analysis.problems})),
            }).await?;
        if !adapted.evidence.iter().all(known_reference) {
            return Err(AppError::Validation(
                "Prompt adaptation cited evidence outside its source".into(),
            ));
        }
        analysis.instruction = adapted.instruction;
        analysis.addressed_problems = adapted.addressed_problems;
        analysis.evidence.extend(adapted.evidence);
        analysis.evidence.sort();
        analysis.evidence.dedup();
        Some(research)
    } else {
        None
    };
    if !analysis
        .addressed_problems
        .iter()
        .all(|key| analysis.problems.iter().any(|problem| &problem.key == key))
    {
        return Err(AppError::Validation(
            "Prompt intervention names an unobserved problem".into(),
        ));
    }
    let team = state
        .db
        .experience_for_turn(&claim.workspace_id, &claim.binding_id)
        .await?
        .and_then(|turn| turn.team)
        .map(|team| json!({"config":team,"review":"passed"}));
    if !escalation_candidates.is_empty() && source["more"] != true && analysis.instruction.is_none()
    {
        analysis.instruction = Some(claim.instruction.clone());
    }
    let instruction = analysis
        .instruction
        .as_deref()
        .or_else(|| {
            (claim.measured && evaluation_cases.iter().any(|case| case.check.is_some()))
                .then_some(claim.instruction.as_str())
        })
        .filter(|_| source["more"] != true);
    let review_details = if instruction == Some(claim.instruction.as_str())
        && analysis.instruction.is_none()
    {
        json!({"passed":true,"reason":"unchanged_seed_for_trace_evaluation"})
    } else if let Some(instruction) = instruction {
        let verification: Analysis = check.call(&RuntimeHost::for_binding(state, &analyst)?, "content_review", HostRequest::AnalyzeExperience {
            spec: Box::new(terminal_spec(&analyst, 120, 40, None, None)),
            prompt: format!("{REVIEW}\n\nYou are checking a proposed instruction change, not generating another one. Compare it with the immutable source and prior instruction. The existing_team is unchanged context, not a proposed structural change. Reject unsupported generalization, lost still-valid guidance, conflicts with that team, personal-data leakage and permission changes. Return the proposed instruction byte-for-byte only if justified; otherwise instruction must be null. This is a content review, not proof of future execution success.\n{}",
                json!({"prior_instruction":claim.instruction,"prior_summary":claim.source_summary,"prior_references":claim.source_references,"proposed_instruction":instruction,"trace":source,
                    "existing_team":team,
                    "known_problems":analysis.problems,"proposed_addressed_problems":analysis.addressed_problems})),
        }).await?;
        review_diagnostics(
            instruction,
            &analysis.addressed_problems,
            &verification,
            known_reference,
        )
    } else {
        json!({"passed":false,"reason":if source["more"] == true {"source_incomplete"} else {"no_proposed_instruction"}})
    };
    let review = review_details["passed"] == true;
    let revision = state
        .db
        .save_experience_candidate(
            claim,
            ExperienceReport {
                digest:&digest, references:&json!(analysis.evidence), analysis:&analysis.summary,
                instruction,
                validation:&json!({"format":"checked", "review":if review {"passed"} else {"not_passed"},
                    "evaluation_cases":evaluation_cases,
                    "task_quality":task_quality,
                    "trace_id":check.id,"review_details":review_details,
                    "escalation_decisions":escalation_diagnostics(&known_problems, &problems),
                    "observed_revision_id":claim.active_revision_id, "observed_revision_outcome":analysis.previous_revision_outcome,
                    "problems":problems,
                    "addressed_problems":analysis.addressed_problems,
                    "research":research,
                    "escalation_candidates":escalation_candidates,
                    "team":team,
                    "source_complete":source["more"] == false}),
                checkpoint:Some(&Value::Object(checkpoint)), activate:review && !claim.measured,
            },
        )
        .await?;
    if revision.is_none() {
        check
            .record(
                "revision_commit",
                json!({"outcome":"stale_or_duplicate_claim"}),
            )
            .await?;
    }
    Ok(())
}

fn review_diagnostics(
    instruction: &str,
    addressed: &[String],
    verification: &Analysis,
    known_reference: impl Fn(&String) -> bool,
) -> Value {
    let instruction_matches = verification.instruction.as_deref() == Some(instruction);
    let evidence_verified = verification.evidence.iter().all(known_reference);
    let addressed_matches = verification
        .addressed_problems
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        == addressed.iter().collect();
    json!({"passed":instruction_matches && evidence_verified && addressed_matches,
        "instruction_matches":instruction_matches,"evidence_verified":evidence_verified,
        "addressed_problems_match":addressed_matches,
        "reviewer_report":crate::handlers_events::sanitize_telemetry_value(json!(verification))})
}

fn escalation_diagnostics(known: &Value, observations: &[Value]) -> Vec<Value> {
    observations.iter().map(|observation| {
        let prior = known.as_array().and_then(|items| items.iter().find(|p| p["key"] == observation["key"]));
        let reason = match prior {
            _ if observation["applied_revision_id"].as_str().is_none() => "no_verified_applied_revision",
            None => "no_prior_problem",
            Some(prior) if prior["prompt_revision_id"] != observation["applied_revision_id"] => "intervention_revision_mismatch",
            Some(prior) if prior["episodes"].as_array().is_none_or(|episodes| episodes.is_empty()) => "no_prior_episode",
            Some(prior) if prior["episodes"].as_array().is_some_and(|episodes| episodes.iter().any(|p| p["episode_ref"] == observation["episode_ref"])) => "episode_already_observed",
            _ => "eligible",
        };
        json!({"problem_key":observation["key"],"episode_ref":observation["episode_ref"],"applied_revision_id":observation["applied_revision_id"],"intervention_revision_id":prior.map(|p| &p["prompt_revision_id"]),"reason":reason})
    }).collect()
}

fn applied_before_failure(
    records: &[Value],
    reference: &str,
    revision: &str,
    evidence: &[String],
) -> bool {
    let Some(index) = records.iter().position(|record| {
        record["ref"] == reference
            && (record["record"]["type"] == "user" || record["record"]["payload"]["role"] == "user")
            && record
                .to_string()
                .contains(&format!("[choruz-experience revision={revision}]"))
    }) else {
        return false;
    };
    records
        .iter()
        .skip(index + 1)
        .any(|record| evidence.iter().any(|r| record["ref"] == *r))
}

// Historical records remain separate: they must never satisfy the new-evidence guard.
fn historical_applied_before_failure(
    records: &[Value],
    start: Option<&Cursor>,
    historical: &[HistoricalRecord],
    reference: &str,
    revision: &str,
    evidence: &[String],
) -> bool {
    let Some(start) = start else {
        return false;
    };
    historical.iter().any(|marker| {
        marker.reference == reference
            && marker.session == start.session
            && marker.reference == format!("{}:{}", marker.session, marker.offset)
            && marker.offset < start.offset
            && (marker.record["type"] == "user" || marker.record["payload"]["role"] == "user")
            && marker
                .record
                .to_string()
                .contains(&format!("[choruz-experience revision={revision}]"))
            && records.iter().any(|record| {
                let Some(reference) = record["ref"].as_str() else {
                    return false;
                };
                evidence.iter().any(|r| r == reference)
                    && reference
                        .strip_prefix(&format!("{}:", start.session))
                        .and_then(|offset| offset.parse::<u64>().ok())
                        .is_some_and(|offset| {
                            reference == format!("{}:{offset}", start.session)
                                && offset >= start.offset
                        })
            })
    })
}

fn repeated_after_prompt(known: &Value, observations: &[Value]) -> Vec<String> {
    escalation_diagnostics(known, observations)
        .into_iter()
        .filter(|decision| decision["reason"] == "eligible")
        .filter_map(|decision| decision["problem_key"].as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_curation_resumes_paging_and_withdraws_unverifiable_evidence() {
        let mut claim = ExperienceClaim {
            trace_cases: true,
            measured: true,
            binding_id: "b".into(),
            workspace_id: "w".into(),
            owner_id: "o".into(),
            analyst_binding_id: "a".into(),
            generation: 1,
            token: "lease".into(),
            active_revision_id: None,
            instruction: String::new(),
            source_cursor: json!({"_curation_started":100,"session":{"offset":12},"_more":true}),
            source_summary: String::new(),
            source_references: json!([]),
        };
        prepare_curation(&mut claim, 90000);
        assert_eq!(claim.source_cursor["session"]["offset"], 12);
        claim.source_cursor["_more"] = json!(false);
        prepare_curation(&mut claim, 90000);
        assert!(claim.source_cursor["session"].is_null());
        claim.source_cursor["session"] = json!({"offset":3});
        prepare_curation(&mut claim, 90001);
        assert_eq!(claim.source_cursor["session"]["offset"], 3);
        let old = choruz_domain::evaluation::TraceCase {
            variant: None,
            group_ref: None,
            classification: None,
            episode_ref: "missing".into(),
            evidence: vec!["answer".into()],
            input: "Task".into(),
            check: Some(choruz_domain::evaluation::OutputCheck::Exact {
                expected: "ok".into(),
            }),
            reason: "Checked".into(),
        };
        let mut changes = vec![];
        let mut cursor = serde_json::Map::new();
        finish_curation(&mut cursor, std::slice::from_ref(&old), &mut changes, false);
        assert!(changes.is_empty());
        finish_curation(&mut cursor, std::slice::from_ref(&old), &mut changes, true);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].check.is_none());
        assert!(old.check.is_some());
        cursor.clear();
        changes = vec![old.clone()];
        finish_curation(&mut cursor, std::slice::from_ref(&old), &mut changes, false);
        for page in 0..10 {
            changes = (0..20)
                .map(|i| {
                    let mut case = old.clone();
                    case.episode_ref = format!("z-new:{page}:{i}");
                    case
                })
                .collect();
            finish_curation(&mut cursor, std::slice::from_ref(&old), &mut changes, false);
        }
        changes.clear();
        finish_curation(&mut cursor, std::slice::from_ref(&old), &mut changes, true);
        assert!(
            changes.is_empty(),
            "revalidated evidence must survive a long sweep"
        );
    }

    #[test]
    fn review_diagnostics_distinguish_rejection_from_protocol_mismatch() {
        let report = Analysis {
            evaluation_cases: vec![],
            summary: "Missing evidence. token=fixture-secret".into(),
            instruction: None,
            evidence: vec!["verified".into()],
            previous_revision_outcome: "not_observed".into(),
            problems: vec![],
            addressed_problems: vec![],
        };
        let rejected = review_diagnostics("Check", &[], &report, |r| r == "verified");
        assert_eq!(rejected["passed"], false);
        assert_eq!(rejected["instruction_matches"], false);
        assert_eq!(rejected["evidence_verified"], true);
        assert!(
            rejected["reviewer_report"]["summary"]
                .as_str()
                .unwrap()
                .contains("Missing evidence")
        );
        assert!(!rejected.to_string().contains("fixture-secret"));
        let mut report = report;
        report.instruction = Some("Check".into());
        assert_eq!(
            review_diagnostics("Check", &[], &report, |_| true)["passed"],
            true
        );
        assert_eq!(
            review_diagnostics("Check", &[], &report, |_| false)["evidence_verified"],
            false
        );
        assert_eq!(
            review_diagnostics("Check", &["p".into()], &report, |_| true)["addressed_problems_match"],
            false
        );
    }

    #[test]
    fn historical_marker_requires_same_source_and_new_ordered_evidence() {
        let start = Cursor {
            session: "owned".into(),
            offset: 10,
        };
        let marker = HistoricalRecord {
            reference: "owned:9".into(),
            session: "owned".into(),
            offset: 9,
            record: json!({"payload":{"role":"user","content":"[choruz-experience revision=p1]"}}),
        };
        let records = vec![json!({"ref":"owned:10","record":{"type":"assistant"}})];
        let check = |marker: &HistoricalRecord, start: Option<&Cursor>, evidence: &[String]| {
            historical_applied_before_failure(
                &records,
                start,
                std::slice::from_ref(marker),
                "owned:9",
                "p1",
                evidence,
            )
        };
        let evidence = vec!["owned:10".into()];
        assert!(!applied_before_failure(
            &records, "owned:9", "p1", &evidence
        ));
        assert!(check(&marker, Some(&start), &evidence));
        assert!(!check(&marker, None, &evidence));
        assert!(!historical_applied_before_failure(
            &records,
            Some(&start),
            &[],
            "owned:9",
            "p1",
            &evidence
        ));
        for reference in ["other:10", "message:feedback", "owned:010", "owned:8"] {
            assert!(!historical_applied_before_failure(
                &[json!({"ref":reference})],
                Some(&start),
                std::slice::from_ref(&marker),
                "owned:9",
                "p1",
                &[reference.into()]
            ));
        }
        assert!(!check(&marker, Some(&start), &["owned:8".into()]));
        assert!(!check(&marker, Some(&start), &["other:10".into()]));
        for changed in [
            HistoricalRecord {
                session: "other".into(),
                ..marker.clone()
            },
            HistoricalRecord {
                reference: "owned:09".into(),
                ..marker.clone()
            },
            HistoricalRecord {
                offset: 10,
                ..marker.clone()
            },
            HistoricalRecord {
                record: json!({"payload":{"role":"assistant","content":"[choruz-experience revision=p1]"}}),
                ..marker.clone()
            },
            HistoricalRecord {
                record: json!({"type":"user","message":{"content":"[choruz-experience revision=p2]"}}),
                ..marker.clone()
            },
        ] {
            assert!(!check(&changed, Some(&start), &evidence));
        }
        let claude = HistoricalRecord {
            record: json!({"type":"user","message":{"content":"[choruz-experience revision=p1]"}}),
            ..marker.clone()
        };
        assert!(check(&claude, Some(&start), &evidence));
        assert!(!check(
            &marker,
            Some(&Cursor {
                offset: 9,
                ..start.clone()
            }),
            &evidence
        ));
        assert!(!check(
            &marker,
            Some(&Cursor {
                offset: 11,
                ..start
            }),
            &evidence
        ));
    }

    #[test]
    fn escalation_requires_a_distinct_episode_after_the_prompt_intervention() {
        let history = json!([{"key":"skipped-check","prompt_revision_id":"p1","episodes":[{"episode_ref":"task1"}]}]);
        let observation =
            json!({"key":"skipped-check","episode_ref":"task2","applied_revision_id":"p1"});
        assert_eq!(
            repeated_after_prompt(&history, std::slice::from_ref(&observation)),
            ["skipped-check"]
        );
        let mismatch =
            json!({"key":"skipped-check","episode_ref":"task2","applied_revision_id":"p2"});
        let diagnostics = escalation_diagnostics(&history, &[mismatch]);
        assert_eq!(diagnostics[0]["reason"], "intervention_revision_mismatch");
        assert_eq!(diagnostics[0]["intervention_revision_id"], "p1");
        assert_eq!(diagnostics[0]["applied_revision_id"], "p2");
        for (field, value) in [
            ("episode_ref", json!("task1")),
            ("applied_revision_id", Value::Null),
            ("applied_revision_id", json!("unrelated")),
            ("key", json!("other-problem")),
        ] {
            let mut changed = observation.clone();
            changed[field] = value;
            assert!(repeated_after_prompt(&history, &[changed]).is_empty());
        }
        let records = vec![
            json!({"ref":"use","record":{"type":"user","message":{"content":"[choruz-experience revision=p1]"}}}),
            json!({"ref":"failure","record":{"type":"assistant"}}),
        ];
        assert!(applied_before_failure(
            &records,
            "use",
            "p1",
            &["failure".into()]
        ));
        assert!(!applied_before_failure(
            &records,
            "use",
            "p2",
            &["failure".into()]
        ));
        let reversed = vec![records[1].clone(), records[0].clone()];
        assert!(!applied_before_failure(
            &reversed,
            "use",
            "p1",
            &["failure".into()]
        ));
        let mut quoted = records;
        quoted[0]["record"]["type"] = json!("assistant");
        assert!(!applied_before_failure(
            &quoted,
            "use",
            "p1",
            &["failure".into()]
        ));
    }
}
