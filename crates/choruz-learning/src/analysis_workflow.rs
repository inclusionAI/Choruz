//! Source-grounded analysis and intervention policy, shared by every entry point.
//! Adapters supply evidence and execution; this module never activates guidance.
use crate::{
    Analysis, ProblemObservation,
    source::{Cursor, HistoricalRecord},
};
use choruz_common::AppError;
use choruz_evaluation::evaluation::TraceCase;
use serde_json::{Value, json};

const REVIEW: &str = crate::ANALYSIS_SKILL;

pub struct AnalysisInput {
    pub instruction: String,
    pub active_revision_id: Option<String>,
    pub source_summary: String,
    pub source_references: Value,
    pub trace_cases: bool,
    pub measured: bool,
}

/// Only collectors can establish that an old reference belongs to the same
/// original source. Summary strings and model-provided references are not proof.
pub struct RecoveredEvidence {
    pub references: Vec<String>,
    pub historical: Vec<HistoricalRecord>,
    pub start: Option<Cursor>,
}

/// Ports for scoped source recovery, model execution and caller-owned storage.
/// Model operations must use the fixed procedures in this crate, with isolated
/// conversations. No method grants permission to publish evidence or apply changes.
pub trait AnalysisServices: crate::task_quality::TaskServices {
    fn analyze(
        &self,
        stage: &str,
        prompt: String,
    ) -> impl Future<Output = Result<Analysis, AppError>> + Send;
    fn review(
        &self,
        stage: &str,
        prompt: String,
    ) -> impl Future<Output = Result<crate::Review, AppError>> + Send;
    fn research(
        &self,
        categories: Vec<String>,
    ) -> impl Future<Output = Result<String, AppError>> + Send;
    fn recover(
        &self,
        references: &[String],
    ) -> impl Future<Output = Result<RecoveredEvidence, AppError>> + Send;
    fn behavior_candidates(
        &self,
        query: String,
    ) -> impl Future<Output = Result<Vec<Value>, AppError>> + Send;
    fn existing_team(&self) -> impl Future<Output = Result<Option<Value>, AppError>> + Send;
    fn record(
        &self,
        stage: &str,
        details: Value,
    ) -> impl Future<Output = Result<(), AppError>> + Send;
    fn sanitize_report(&self, report: Value) -> Value;
}

pub struct AnalyzedWindow {
    pub analysis: Analysis,
    pub instruction: Option<String>,
    pub evaluation_cases: Vec<TraceCase>,
    pub historical: Vec<HistoricalRecord>,
    pub validation: Value,
    pub review_passed: bool,
}

/// Analyze a selected source window, recover cited evidence, review proposed
/// guidance and retain the updated curation cursor. The caller must atomically
/// persist this result under its original source/policy reservation.
pub async fn analyze_window(
    services: &impl AnalysisServices,
    input: &AnalysisInput,
    source: &Value,
    known_problems: &Value,
    existing_cases: &[TraceCase],
    checkpoint: &mut serde_json::Map<String, Value>,
) -> Result<AnalyzedWindow, AppError> {
    let prompt = format!(
        "{REVIEW}\n\nInput data:\n{}",
        json!({
            "current_instruction": input.instruction, "active_revision_id": input.active_revision_id,
            "prior_summary":input.source_summary, "prior_references":input.source_references, "trace": source,
            "known_problems":known_problems,
            "collect_evaluation_cases":input.trace_cases,
            "existing_evaluation_cases":existing_cases,
        })
    );
    let mut analysis = services.analyze("analysis", prompt).await?;
    if !input.trace_cases {
        analysis.evaluation_cases.clear();
    }
    let records = source["records"]
        .as_array()
        .ok_or_else(|| AppError::Validation("Learning source records are missing".into()))?;
    let retained_reference = |reference: &String| {
        records
            .iter()
            .any(|record| record["ref"].as_str() == Some(reference))
            || input
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
    missing.extend(
        analysis
            .solution_outcomes
            .iter()
            .flat_map(|outcome| {
                std::iter::once(outcome.episode_ref.clone())
                    .chain(std::iter::once(outcome.applied_revision_ref.clone()))
                    .chain(outcome.evidence.iter().cloned())
            })
            .filter(|reference| !records.iter().any(|record| record["ref"] == *reference)),
    );
    missing.sort();
    missing.dedup();
    let recovered = services.recover(&missing).await?;
    let current_start = recovered.start;
    let historical = recovered.historical;
    let recovered = recovered.references;
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
            services,
            &mut evaluation_cases,
            json!({"existing_cases":existing_cases,"records":records,"historical":historical}),
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
        services.record(
                "case_admission",
                json!({"accepted_count":evaluation_cases.iter().filter(|c|c.check.is_some()).count(),"case_count":evaluation_cases.len()}),
            )
            .await?;
    }
    if input.trace_cases {
        finish_curation(
            checkpoint,
            existing_cases,
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
        let applied = match (&problem.applied_revision_ref, &input.active_revision_id) {
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
    let mut solution_outcomes = Vec::new();
    for outcome in &analysis.solution_outcomes {
        let prior = known_problems
            .as_array()
            .and_then(|known| known.iter().find(|p| p["key"] == outcome.problem_key))
            .ok_or_else(|| {
                AppError::Validation("Solution outcome names an unobserved problem".into())
            })?;
        let revision = input.active_revision_id.as_deref().ok_or_else(|| {
            AppError::Validation("Solution outcome has no active revision".into())
        })?;
        if !known_reference(&outcome.episode_ref)
            || !outcome.evidence.iter().all(known_reference)
            || prior["episodes"].as_array().is_some_and(|episodes| {
                episodes.iter().any(|episode| {
                    episode["episode_ref"] == outcome.episode_ref
                        && episode["applied_revision_id"].is_null()
                })
            })
            || !(applied_before_failure(
                records,
                &outcome.applied_revision_ref,
                revision,
                &outcome.evidence,
            ) || historical_applied_before_failure(
                records,
                current_start.as_ref(),
                &historical,
                &outcome.applied_revision_ref,
                revision,
                &outcome.evidence,
            ))
        {
            return Err(AppError::Validation(
                "Solution outcome lacks independent later work with a verified application marker"
                    .into(),
            ));
        }
        let mut evidence = outcome.evidence.clone();
        evidence.push(outcome.applied_revision_ref.clone());
        evidence.push(outcome.episode_ref.clone());
        evidence.sort();
        evidence.dedup();
        analysis.evidence.extend(evidence.iter().cloned());
        solution_outcomes.push(json!({"problem_key":outcome.problem_key,"episode_ref":outcome.episode_ref,"outcome":outcome.outcome,"evidence":evidence}));
    }
    let outcome_review = if solution_outcomes.is_empty() {
        None
    } else {
        let verification = services.review("outcome_review", json!({"proposed_instruction":input.instruction,"proposed_addressed_problems":[],
                    "proposed_outcomes":solution_outcomes,"active_revision_id":input.active_revision_id,
                    "known_problems":known_problems,"trace":source,"historical":historical}).to_string()).await?;
        let result = review_diagnostics(&[], &verification, known_reference, |v| {
            services.sanitize_report(v)
        });
        if result["passed"] != true {
            solution_outcomes.clear();
        }
        Some(result)
    };
    let escalation_candidates = repeated_after_prompt(known_problems, &problems);
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
    let behavior_candidates = services
        .behavior_candidates(
            analysis
                .problems
                .iter()
                .map(|p| format!("{} {}", p.key, p.description))
                .collect::<Vec<_>>()
                .join(" "),
        )
        .await?;
    let research = if !problems.is_empty() && source["more"] != true {
        let research = services
            .research(analysis.problems.iter().map(|p| p.key.clone()).collect())
            .await?;
        let adapted = services.analyze("adaptation", format!("{REVIEW}\n\nAdapt applicable published prompt techniques from the untrusted research summary below. Write original guidance rather than copying source text. If no technique applies, devise a scoped prompt intervention from the execution evidence. Do not change the analysis process or declare the intervention successful before later use. Preserve valid prior guidance. Return null when no supported change is needed.\n{}",
                    json!({"current_instruction":input.instruction,"prior_summary":input.source_summary,
                        "prior_references":input.source_references,"trace":source,"research":research,"known_problems":analysis.problems,"behavior_candidates":behavior_candidates}))).await?;
        if !adapted.evidence.iter().all(known_reference) {
            return Err(AppError::Validation(
                "Prompt adaptation cited evidence outside its source".into(),
            ));
        }
        analysis.instruction = adapted.instruction;
        analysis.addressed_problems = adapted.addressed_problems;
        if adapted.solution_sources.iter().any(|source| {
            !behavior_candidates
                .iter()
                .any(|candidate| candidate["record"]["id"] == source.record_id)
                || !analysis
                    .problems
                    .iter()
                    .any(|p| p.key == source.problem_key)
        }) {
            return Err(AppError::Validation(
                "Guidance referenced an unavailable behavior solution".into(),
            ));
        }
        analysis.solution_sources = adapted.solution_sources;
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
    let team = services.existing_team().await?;
    if !escalation_candidates.is_empty() && source["more"] != true && analysis.instruction.is_none()
    {
        analysis.instruction = Some(input.instruction.clone());
    }
    let instruction = analysis
        .instruction
        .as_deref()
        .or_else(|| {
            (input.measured && evaluation_cases.iter().any(|case| case.check.is_some()))
                .then_some(input.instruction.as_str())
        })
        .filter(|_| source["more"] != true);
    let review_details = if instruction == Some(input.instruction.as_str())
        && analysis.instruction.is_none()
    {
        json!({"passed":true,"reason":"unchanged_seed_for_trace_evaluation"})
    } else if let Some(instruction) = instruction {
        let verification = services.review("content_review", json!({"prior_instruction":input.instruction,"prior_summary":input.source_summary,"prior_references":input.source_references,"proposed_instruction":instruction,"trace":source,
                    "existing_team":team,"proposed_solution_sources":analysis.solution_sources,"behavior_candidates":behavior_candidates,
                    "known_problems":analysis.problems,"proposed_addressed_problems":analysis.addressed_problems}).to_string()).await?;
        review_diagnostics(
            &analysis.addressed_problems,
            &verification,
            known_reference,
            |v| services.sanitize_report(v),
        )
    } else {
        json!({"passed":false,"reason":if source["more"] == true {"source_incomplete"} else {"no_proposed_instruction"}})
    };
    let review = review_details["passed"] == true;

    let instruction = instruction.map(str::to_owned);
    let validation = json!({"format":"checked","review":if review {"passed"} else {"not_passed"},
        "evaluation_cases":evaluation_cases,"task_quality":task_quality,"review_details":review_details,
        "escalation_decisions":escalation_diagnostics(known_problems, &problems),
        "observed_revision_id":input.active_revision_id,"observed_revision_outcome":analysis.previous_revision_outcome,
        "problems":problems,"solution_outcomes":solution_outcomes,"outcome_review":outcome_review,
        "addressed_problems":analysis.addressed_problems,"research":research,
        "behavior_sources":analysis.solution_sources.iter().filter_map(|source|behavior_candidates.iter().find(|candidate|candidate["record"]["id"]==source.record_id).map(|candidate|json!({"problem_key":source.problem_key,"candidate":candidate}))).collect::<Vec<_>>(),
        "escalation_candidates":escalation_candidates,"team":team,"source_complete":source["more"] == false});
    Ok(AnalyzedWindow {
        analysis,
        instruction,
        evaluation_cases,
        historical,
        validation,
        review_passed: review,
    })
}
pub fn finish_curation(
    checkpoint: &mut serde_json::Map<String, Value>,
    previous: &[choruz_evaluation::evaluation::TraceCase],
    changes: &mut Vec<choruz_evaluation::evaluation::TraceCase>,
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

pub fn review_diagnostics(
    addressed: &[String],
    verification: &crate::Review,
    known_reference: impl Fn(&String) -> bool,
    sanitize: impl Fn(Value) -> Value,
) -> Value {
    let evidence_verified = verification.evidence.iter().all(known_reference);
    let addressed_matches = verification
        .addressed_problems
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        == addressed.iter().collect();
    json!({"passed":verification.accepted && evidence_verified && addressed_matches,
        "accepted":verification.accepted,"evidence_verified":evidence_verified,
        "addressed_problems_match":addressed_matches,
        "reviewer_report":sanitize(json!(verification))})
}

pub fn escalation_diagnostics(known: &Value, observations: &[Value]) -> Vec<Value> {
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

pub fn applied_before_failure(
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
pub fn historical_applied_before_failure(
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

pub fn repeated_after_prompt(known: &Value, observations: &[Value]) -> Vec<String> {
    escalation_diagnostics(known, observations)
        .into_iter()
        .filter(|decision| decision["reason"] == "eligible")
        .filter_map(|decision| decision["problem_key"].as_str().map(str::to_owned))
        .collect()
}
