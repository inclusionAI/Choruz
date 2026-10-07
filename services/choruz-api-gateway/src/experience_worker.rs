//! Durable, opt-in analysis outside request handlers and foreground execution.
use crate::{
    ApiState,
    experience_diagnostics::{LearningCheck, failure},
    handlers_terminals::{authorize_terminal_binding, terminal_spec},
    host_runtime::RuntimeHost,
};
use choruz_application::db_service::{ExperienceClaim, ExperienceReport};
use choruz_common::AppError;
use choruz_host_runtime::HostRequest;
use choruz_learning::analysis_workflow::finish_curation;
use choruz_learning::source::{Cursor, HistoricalRecord};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub(crate) fn spawn(mut state: ApiState) -> std::sync::Arc<crate::background::WorkerGuard> {
    state.experience_worker = None;
    crate::background::spawn(async move {
        tokio::join!(
            crate::evaluation_worker::run(&state),
            run_analysis(&state),
            crate::behavior_worker::run(&state),
            crate::browser_completion_worker::run(&state)
        );
    })
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
    let mut checkpoint = claim.source_cursor.as_object().cloned().unwrap_or_default();
    let feedback = state.db.experience_feedback(claim).await?;
    let mut windows = vec![("feedback".to_owned(), feedback)];
    let mut native_sources = Vec::new();
    {
        let mut sessions = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in state
            .db
            .learning_source_bindings(&claim.workspace_id, &claim.binding_id)
            .await?
        {
            let binding = if id == target.id {
                target.clone()
            } else {
                match authorize_terminal_binding(state, &actor, &id).await {
                    Ok(binding) => binding,
                    Err(crate::ApiError(AppError::Forbidden(_) | AppError::NotFound(_))) => {
                        continue;
                    }
                    Err(error) => return Err(error.0),
                }
            };
            if state
                .db
                .learning_binding_id(&claim.workspace_id, &binding.id, Some(&claim.owner_id))
                .await?
                != claim.binding_id
            {
                continue;
            }
            let mut retained = state.runtime.retained_native_sources(&binding).await?;
            if let Some(anchor) = binding.valid_terminal_session_anchor_for_context(
                None,
                None,
                Some(binding.terminal_generation()),
                None,
            ) && !retained
                .iter()
                .any(|source| source.session_id == anchor.session_id)
            {
                retained.push(anchor);
            }
            for source in retained {
                if seen.insert(source.session_id.clone()) {
                    sessions.push((
                        binding.clone(),
                        Some(source.session_id),
                        Some(source.native_home_path),
                    ));
                }
            }
            if let Some(external) = &binding.external_session_id
                && seen.insert(external.clone())
            {
                sessions.push((binding.clone(), Some(external.clone()), None));
            }
        }
        if sessions.is_empty() {
            sessions.push((target.clone(), None, None));
        }
        for (binding, resume, home) in sessions {
            let session_key = resume.as_deref().unwrap_or("live");
            let key = if binding.id == target.id {
                session_key.to_owned()
            } else {
                format!("task:{}:{session_key}", binding.id)
            };
            let cursor: choruz_learning::source::Cursor = checkpoint
                .get(&key)
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| AppError::Internal("Invalid stored source cursor".into()))?
                .unwrap_or_default();
            let mut spec = terminal_spec(
                &binding,
                120,
                40,
                resume.clone(),
                home.filter(|path| !path.is_empty()),
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
            windows.push((key, window));
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
    let services = PlatformAnalysis {
        state,
        claim,
        check,
        analyst: &analyst,
        source_host: &source_host,
        native_sources,
        source: &source,
    };
    let analyzed = choruz_learning::analysis_workflow::analyze_window(
        &services,
        &choruz_learning::analysis_workflow::AnalysisInput {
            instruction: claim.instruction.clone(),
            active_revision_id: claim.active_revision_id.clone(),
            source_summary: claim.source_summary.clone(),
            source_references: claim.source_references.clone(),
            trace_cases: claim.trace_cases,
            measured: claim.measured,
        },
        &source,
        &known_problems,
        &existing_cases,
        &mut checkpoint,
    )
    .await?;
    let records = source["records"]
        .as_array()
        .ok_or_else(|| AppError::Validation("Learning source records are missing".into()))?;
    let historical = &analyzed.historical;
    let evaluation_cases = &analyzed.evaluation_cases;
    let decisions = crate::decision_worker::annotate(&source_host, claim, check, &source).await?;
    let program_trial = if source["more"] == false {
        let mut program_records = records.clone();
        program_records.extend(
            historical
                .iter()
                .map(|record| json!({"ref":record.reference,"record":record.record})),
        );
        crate::decision_worker::build(
            state,
            claim,
            check,
            &source_host,
            &existing_cases,
            evaluation_cases,
            &program_records,
        )
        .await?
    } else {
        Value::Null
    };
    let mut validation = analyzed.validation;
    validation["decisions"] = decisions;
    validation["program_trial"] = program_trial;
    validation["trace_id"] = json!(check.id);
    let revision = state
        .db
        .save_experience_candidate(
            claim,
            ExperienceReport {
                digest: &digest,
                references: &json!(analyzed.analysis.evidence),
                analysis: &analyzed.analysis.summary,
                instruction: analyzed.instruction.as_deref(),
                validation: &validation,
                checkpoint: Some(&Value::Object(checkpoint)),
                activate: analyzed.review_passed && !claim.measured,
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

struct PlatformAnalysis<'a> {
    state: &'a ApiState,
    claim: &'a ExperienceClaim,
    check: &'a LearningCheck<'a>,
    analyst: &'a choruz_agent_runtime::RuntimeBinding,
    source_host: &'a RuntimeHost,
    native_sources: Vec<(choruz_host_runtime::TerminalSpec, Cursor)>,
    source: &'a Value,
}

impl choruz_learning::analysis_workflow::AnalysisServices for PlatformAnalysis<'_> {
    async fn analyze(
        &self,
        stage: &str,
        prompt: String,
    ) -> Result<choruz_learning::Analysis, AppError> {
        self.check
            .call(
                &RuntimeHost::for_binding(self.state, self.analyst)?,
                stage,
                HostRequest::AnalyzeExperience {
                    spec: Box::new(terminal_spec(self.analyst, 120, 40, None, None)),
                    prompt,
                },
            )
            .await
    }
    async fn review(
        &self,
        stage: &str,
        prompt: String,
    ) -> Result<choruz_learning::Review, AppError> {
        self.check
            .call(
                &RuntimeHost::for_binding(self.state, self.analyst)?,
                stage,
                HostRequest::ReviewExperience {
                    spec: Box::new(terminal_spec(self.analyst, 120, 40, None, None)),
                    prompt,
                },
            )
            .await
    }
    async fn research(&self, categories: Vec<String>) -> Result<String, AppError> {
        self.check
            .call(
                &RuntimeHost::for_binding(self.state, self.analyst)?,
                "research",
                HostRequest::ResearchExperience {
                    spec: Box::new(terminal_spec(self.analyst, 120, 40, None, None)),
                    categories,
                },
            )
            .await
    }
    async fn recover(
        &self,
        missing: &[String],
    ) -> Result<choruz_learning::analysis_workflow::RecoveredEvidence, AppError> {
        let mut references = self
            .state
            .db
            .experience_feedback_references(self.claim, missing)
            .await?;
        let start = self.native_sources.iter().find_map(|(_, cursor)| {
            (self.source["reset"] != true && self.source["cursor"]["session"] == cursor.session)
                .then(|| cursor.clone())
        });
        let mut historical = Vec::new();
        if !missing.is_empty() {
            for (spec, cursor) in &self.native_sources {
                let requested: Vec<_> = missing
                    .iter()
                    .filter(|reference| reference.starts_with(&format!("{}:", cursor.session)))
                    .cloned()
                    .collect();
                if requested.is_empty() {
                    continue;
                }
                let verified: Vec<HistoricalRecord> = self
                    .check
                    .call(
                        self.source_host,
                        "source_recovery",
                        HostRequest::ExperienceReferences {
                            spec: Box::new(spec.clone()),
                            cursor: cursor.clone(),
                            references: requested,
                        },
                    )
                    .await?;
                references.extend(verified.iter().map(|record| record.reference.clone()));
                historical.extend(verified);
            }
        }
        Ok(choruz_learning::analysis_workflow::RecoveredEvidence {
            references,
            historical,
            start,
        })
    }
    async fn behavior_candidates(&self, query: String) -> Result<Vec<Value>, AppError> {
        self.state
            .db
            .behavior_candidates(
                &self.claim.workspace_id,
                &self.claim.owner_id,
                &self.claim.binding_id,
                &query,
            )
            .await
    }
    async fn existing_team(&self) -> Result<Option<Value>, AppError> {
        Ok(self
            .state
            .db
            .experience_for_turn(&self.claim.workspace_id, &self.claim.binding_id)
            .await?
            .and_then(|turn| turn.team)
            .map(|team| json!({"config":team,"review":"passed"})))
    }
    async fn record(&self, stage: &str, details: Value) -> Result<(), AppError> {
        self.check.record(stage, details).await
    }
    fn sanitize_report(&self, report: Value) -> Value {
        choruz_activity::sanitize_value(report)
    }
}

impl choruz_learning::task_quality::TaskServices for PlatformAnalysis<'_> {
    async fn trial(&self, prompt: String) -> Result<String, AppError> {
        self.check
            .call(
                &RuntimeHost::for_binding(self.state, self.analyst)?,
                "task_trial",
                HostRequest::EvaluateExperience {
                    spec: Box::new(terminal_spec(self.analyst, 120, 40, None, None)),
                    input: prompt,
                    instruction: String::new(),
                    preflight: String::new(),
                },
            )
            .await
    }

    async fn review_tasks(
        &self,
        prompt: String,
    ) -> Result<Vec<choruz_learning::TaskDecision>, AppError> {
        self.check
            .call(
                &RuntimeHost::for_binding(self.state, self.analyst)?,
                "task_quality_review",
                HostRequest::ReviewTasks {
                    spec: Box::new(terminal_spec(self.analyst, 120, 40, None, None)),
                    prompt,
                },
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use choruz_learning::analysis_workflow::{
        applied_before_failure, escalation_diagnostics, historical_applied_before_failure,
        repeated_after_prompt,
    };
    fn review_diagnostics(
        addressed: &[String],
        report: &choruz_learning::Review,
        known: impl Fn(&String) -> bool,
    ) -> Value {
        choruz_learning::analysis_workflow::review_diagnostics(
            addressed,
            report,
            known,
            choruz_activity::sanitize_value,
        )
    }

    #[test]
    fn daily_curation_resumes_paging_and_withdraws_unverifiable_evidence() {
        let mut claim = ExperienceClaim {
            decision_settings: None,
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
        let old = choruz_evaluation::evaluation::TraceCase {
            variant: None,
            group_ref: None,
            classification: None,
            episode_ref: "missing".into(),
            evidence: vec!["answer".into()],
            input: "Task".into(),
            check: Some(choruz_evaluation::evaluation::OutputCheck::Exact {
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
        let report = choruz_learning::Review {
            accepted: false,
            reason: "Missing evidence. token=fixture-secret".into(),
            evidence: vec!["verified".into()],
            addressed_problems: vec![],
        };
        let rejected = review_diagnostics(&[], &report, |r| r == "verified");
        assert_eq!(rejected["passed"], false);
        assert_eq!(rejected["accepted"], false);
        assert_eq!(rejected["evidence_verified"], true);
        assert!(
            rejected["reviewer_report"]["reason"]
                .as_str()
                .unwrap()
                .contains("Missing evidence")
        );
        assert!(!rejected.to_string().contains("fixture-secret"));
        let mut report = report;
        report.accepted = true;
        assert_eq!(review_diagnostics(&[], &report, |_| true)["passed"], true);
        assert_eq!(
            review_diagnostics(&[], &report, |_| false)["evidence_verified"],
            false
        );
        assert_eq!(
            review_diagnostics(&["p".into()], &report, |_| true)["addressed_problems_match"],
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
