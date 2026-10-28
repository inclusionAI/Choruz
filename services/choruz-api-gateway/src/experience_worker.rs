//! Durable, opt-in analysis outside request handlers and foreground execution.
use crate::{
    ApiState,
    experience_diagnostics::{LearningCheck, failure},
    handlers_terminals::{authorize_terminal_binding, terminal_spec},
    host_runtime::RuntimeHost,
};
use choruz_application::db_service::{CheckOutcome, ExperienceClaim, ExperienceReport};
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
    let mut jobs = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = interval.tick() => {
                // A slow device must not hold up every other learning profile.
                // Database leases retain per-binding exclusion across processes.
                while jobs.len() < 4 {
                    match state.db.claim_experience().await {
                        Ok(Some(claim)) => {
                            let state = state.clone();
                            jobs.spawn(async move { process_claim(&state, claim).await });
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(%error,"learning queue unavailable");
                            break;
                        }
                    }
                }
            }
            result = jobs.join_next(), if !jobs.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::warn!(%error,"learning task exited unexpectedly");
                }
            }
        }
    }
}

pub(crate) async fn process_claim(state: &ApiState, mut claim: ExperienceClaim) {
    prepare_curation(&mut claim, chrono::Utc::now().timestamp());
    let check = LearningCheck::new(state, &claim);
    let outcome = async {
        check.record("check", json!({"outcome":"started"})).await?;
        let review = review(state, &claim, &check);
        tokio::pin!(review);
        let mut renew = tokio::time::interval(Duration::from_secs(30));
        renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                result = &mut review => break result,
                _ = renew.tick() => {
                    if !state.db.renew_experience(&claim).await? {
                        break Err(AppError::Conflict("Learning settings changed or reservation expired".into()));
                    }
                }
            }
        }
    }.await;
    finish_check(state, &claim, &check, outcome).await;
}

/// Record how a check ended and release its lease. Only a busy source defers;
/// every error, including a host or provider rate limit, is a visible failure.
pub(crate) async fn finish_check(
    state: &ApiState,
    claim: &ExperienceClaim,
    check: &LearningCheck<'_>,
    outcome: Result<Reviewed, AppError>,
) {
    let details = match &outcome {
        Ok(Reviewed::Done) => json!({"outcome":"completed"}),
        Ok(Reviewed::Deferred(delay)) => {
            json!({"outcome":"deferred","retry_after_ms":delay.as_millis() as u64})
        }
        Err(error) => failure(error),
    };
    if let Err(error) = check.record("check", details).await {
        tracing::warn!(trace_id=%check.id, binding_id=%claim.binding_id, %error, "learning diagnostic persistence failed");
    }
    let message = failure_message(&outcome);
    let released = match (&outcome, message.as_deref()) {
        (Ok(Reviewed::Deferred(delay)), _) => CheckOutcome::Deferred(*delay),
        (_, Some(message)) => CheckOutcome::Failed(message),
        (_, None) => CheckOutcome::Completed,
    };
    if let Err(error) = state.db.release_experience(claim, released).await {
        tracing::warn!(binding_id=%claim.binding_id, %error, "learning lease release failed");
    }
    tracing::info!(trace_id=%check.id, binding_id=%claim.binding_id, success=message.is_none(), "learning check finished");
}

/// How a learning review ended without an error.
#[derive(Debug, PartialEq)]
pub(crate) enum Reviewed {
    Done,
    /// The source turn was still running; check again after the delay.
    Deferred(Duration),
}

/// The owner-visible reason a check failed, or `None` when it did not.
/// Diagnostics never include the trace, model output or credentials.
fn failure_message(outcome: &Result<Reviewed, AppError>) -> Option<String> {
    match outcome {
        Ok(_) => None,
        Err(AppError::NotFound(message) | AppError::Conflict(message) | AppError::Validation(message)) => Some(message.clone()),
        Err(AppError::Forbidden(_) | AppError::Unauthorized(_)) => Some("Learning no longer has access to the selected Agent or workspace.".into()),
        Err(_) => Some("Background analysis failed. Check the selected account and device; learning will retry.".into()),
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
) -> Result<Reviewed, AppError> {
    let actor = state.db.get_principal(&claim.owner_id).await?;
    // Recheck access at execution time, not only when the user enabled learning.
    let target = authorize_terminal_binding(state, &actor, &claim.binding_id)
        .await
        .map_err(|e| e.0)?;
    let analyst = authorize_terminal_binding(state, &actor, &claim.analyst_binding_id)
        .await
        .map_err(|e| e.0)?;
    let source_host = RuntimeHost::for_binding(state, &target)?;
    let mut existing_cases = state
        .db
        .experience_trace_cases(&claim.workspace_id, &claim.binding_id)
        .await?;
    let mut checkpoint = claim.source_cursor.as_object().cloned().unwrap_or_default();
    let feedback = state.db.experience_feedback(claim).await?;
    let mut windows = vec![("feedback".to_owned(), feedback)];
    let mut native_sources = Vec::new();
    {
        for source in
            crate::learning_sources::native_sources(state, &actor, &claim.workspace_id, &target)
                .await?
        {
            let key = source.key;
            let cursor: choruz_learning::source::Cursor = checkpoint
                .get(&key)
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| AppError::Internal("Invalid stored source cursor".into()))?
                .unwrap_or_default();
            let mut spec = source.spec;
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
            if window["busy"] == true {
                let delay = window["retry_after_ms"].as_u64().ok_or_else(|| {
                    AppError::Internal("Busy source window has no retry delay".into())
                })?;
                return Ok(Reviewed::Deferred(Duration::from_millis(delay)));
            }
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
    let mut known_problems = state.db.experience_problems(claim).await?;
    let execution_context = json!({
        "target":crate::experience_diagnostics::execution_context(&terminal_spec(&target,120,40,None,None)),
        "analyst":crate::experience_diagnostics::execution_context(&terminal_spec(&analyst,120,40,None,None)),
    });
    let retained_window = state.db.experience_window(claim, None).await?;
    if let Some(window) = &retained_window {
        let mut feedback_references = Vec::new();
        continuation_feedback_references(window, &mut feedback_references);
        if !state
            .db
            .experience_window_feedback_current(claim, &feedback_references)
            .await?
        {
            return Err(AppError::Forbidden("Learning continuation includes feedback that is no longer shared. Save learning settings to cancel it.".into()));
        }
    }
    if retained_window
        .as_ref()
        .is_some_and(|window| window["context"] != execution_context)
    {
        return Err(AppError::Conflict("Learning account, model or device changed. Save learning settings to cancel the old continuation and start a fresh attempt.".into()));
    }
    let next = retained_window
        .as_ref()
        .map(|window| {
            (
                window["key"].as_str().unwrap_or("_continuation").to_owned(),
                window["source"].clone(),
            )
        })
        .or_else(|| pending.next())
        .or_else(|| {
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
        return Ok(Reviewed::Done);
    };
    // Do not activate guidance from feedback while older native work is unread.
    if retained_window.is_none() {
        source["more"] = json!(unread || pending.next().is_some());
    }
    if claim.trace_cases && retained_window.is_none() {
        source["curation_cycle"] = claim.source_cursor["_curation_started"].clone();
    }
    if let Some(guidance) = guidance.filter(|_| retained_window.is_none()) {
        source["project_guidance"] = guidance;
    }
    source = choruz_activity::sanitize_value(source);
    checkpoint.insert(key.clone(), source["cursor"].clone());
    checkpoint.insert("_more".into(), source["more"].clone());
    let digest = hex::encode(Sha256::digest(source.to_string().as_bytes()));
    if state.db.experience_source_seen(claim, &digest).await? {
        state.db.finish_experience_window(claim).await?;
        check
            .record(
                "source_selection",
                json!({"outcome":"already_analyzed","source_digest":digest}),
            )
            .await?;
        return Ok(Reviewed::Done);
    }
    let initial_input = choruz_learning::analysis_workflow::AnalysisInput {
        instruction: claim.instruction.clone(),
        active_revision_id: claim.active_revision_id.clone(),
        source_summary: claim.source_summary.clone(),
        source_references: claim.source_references.clone(),
        trace_cases: claim.trace_cases,
        measured: claim.measured,
    };
    let window = match retained_window {
        Some(window) => window,
        None => state
            .db
            .experience_window(
                claim,
                Some(&json!({"key":key,"source":source,
            "context":execution_context,"input":initial_input,"known_problems":known_problems,
            "existing_cases":existing_cases,"checkpoint":checkpoint})),
            )
            .await?
            .ok_or_else(|| AppError::Internal("Learning continuation was not reserved".into()))?,
    };
    check.scope_window(&window)?;
    let input = serde_json::from_value(window["input"].clone())
        .map_err(|e| AppError::Internal(format!("restore learning input: {e}")))?;
    known_problems = window["known_problems"].clone();
    existing_cases = serde_json::from_value(window["existing_cases"].clone())
        .map_err(|e| AppError::Internal(format!("restore learning objectives: {e}")))?;
    checkpoint = serde_json::from_value(window["checkpoint"].clone())
        .map_err(|e| AppError::Internal(format!("restore learning cursor: {e}")))?;
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
        &input,
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
        match crate::decision_worker::build(
            state,
            claim,
            check,
            &source_host,
            &existing_cases,
            evaluation_cases,
            &program_records,
        )
        .await
        {
            Ok(report) => report,
            Err(error) => {
                check.record("program_pending", failure(&error)).await?;
                json!({"status":"pending","failure":failure(&error),
                    "recovery":"Check the selected program builder account and device. Core learning is saved independently."})
            }
        }
    } else {
        Value::Null
    };
    let mut validation = analyzed.validation;
    let current_target = authorize_terminal_binding(state, &actor, &claim.binding_id)
        .await
        .map_err(|e| e.0)?;
    let current_analyst = authorize_terminal_binding(state, &actor, &claim.analyst_binding_id)
        .await
        .map_err(|e| e.0)?;
    let current_context = json!({
        "target":crate::experience_diagnostics::execution_context(&terminal_spec(&current_target,120,40,None,None)),
        "analyst":crate::experience_diagnostics::execution_context(&terminal_spec(&current_analyst,120,40,None,None)),
    });
    if current_context != window["context"] {
        return Err(AppError::Conflict("Learning execution context changed before publication. Save learning settings to restart.".into()));
    }
    let research_pending = validation["research_status"] == "pending";
    let program_pending = program_trial["status"] == "pending";
    let commit_digest = if research_pending {
        format!("{digest}:research_pending")
    } else if program_pending {
        format!("{digest}:program_pending")
    } else {
        digest.clone()
    };
    validation["decisions"] = decisions;
    validation["program_trial"] = program_trial;
    validation["trace_id"] = json!(check.id);
    let revision = state
        .db
        .save_experience_candidate(
            claim,
            ExperienceReport {
                digest: &commit_digest,
                references: &json!(analyzed.analysis.evidence),
                analysis: &analyzed.analysis.summary,
                instruction: analyzed.instruction.as_deref(),
                validation: &validation,
                checkpoint: (!(research_pending || program_pending))
                    .then_some(&Value::Object(checkpoint)),
                activate: analyzed.review_passed && !claim.measured,
            },
        )
        .await?;
    if revision.is_none() && !(research_pending || program_pending) {
        check
            .record(
                "revision_commit",
                json!({"outcome":"stale_or_duplicate_claim"}),
            )
            .await?;
        return Err(AppError::Conflict(
            "Learning result was not committed because its reservation changed".into(),
        ));
    }
    if research_pending {
        return Err(AppError::Validation("Source analysis was saved, but research is pending. Check search access; save learning settings to authorize a fresh attempt after an interrupted model call.".into()));
    }
    if program_pending {
        return Err(AppError::Validation("Core learning was saved, but program construction is pending. Check or reselect the program builder account and device in learning settings.".into()));
    }
    Ok(Reviewed::Done)
}

fn continuation_feedback_references(value: &Value, references: &mut Vec<String>) {
    match value {
        Value::Array(values) => values
            .iter()
            .for_each(|value| continuation_feedback_references(value, references)),
        Value::Object(values) => {
            for (key, value) in values {
                if matches!(
                    key.as_str(),
                    "ref"
                        | "episode_ref"
                        | "evidence"
                        | "source_references"
                        | "prior_references"
                        | "references"
                ) {
                    if let Some(reference) = value
                        .as_str()
                        .filter(|reference| reference.starts_with("message:"))
                    {
                        references.push(reference.to_owned());
                    }
                    if let Some(values) = value.as_array() {
                        references.extend(
                            values
                                .iter()
                                .filter_map(Value::as_str)
                                .filter(|reference| reference.starts_with("message:"))
                                .map(str::to_owned),
                        );
                    }
                }
                continuation_feedback_references(value, references);
            }
        }
        _ => {}
    }
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

    #[test]
    fn only_a_busy_source_defers_and_every_error_stays_visible() {
        assert_eq!(
            failure_message(&Ok(Reviewed::Deferred(Duration::from_secs(5)))),
            None
        );
        // A provider or host rate limit is a failed check, not a busy turn.
        assert!(
            failure_message(&Err(AppError::RateLimited {
                retry_after_ms: 5_000
            }))
            .is_some_and(|message| message.contains("learning will retry"))
        );
        assert_eq!(
            failure_message(&Err(AppError::Conflict("context changed".into()))).as_deref(),
            Some("context changed")
        );
    }
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
