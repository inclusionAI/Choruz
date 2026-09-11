//! Blind trials diagnose tasks; independent source-grounded review controls admission.
use crate::{experience_diagnostics::LearningCheck, host_runtime::RuntimeHost};
use choruz_common::AppError;
use choruz_domain::evaluation::{TraceCase, case_review_covers};
use choruz_host_runtime::{HostRequest, TerminalSpec, experience::TaskDecision};
use serde_json::{Value, json};
use sha2::Digest;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) async fn inspect(
    host: &RuntimeHost,
    spec: TerminalSpec,
    cases: &mut [TraceCase],
    context: Value,
    check: &LearningCheck<'_>,
) -> Result<Value, AppError> {
    let variants: BTreeMap<_, _> = cases
        .iter_mut()
        .filter_map(|c| c.variant.take().map(|v| (c.episode_ref.clone(), v)))
        .collect();
    let mut history = inspect_rounds(host, spec.clone(), cases, &context, check, true).await?;
    let previous: Vec<TraceCase> = serde_json::from_value(context["existing_cases"].clone())
        .map_err(|e| AppError::Internal(format!("Invalid task review corpus: {e}")))?;
    let training = choruz_application::db_service::training_objectives(&previous, cases);
    let mut proposed: Vec<_> = cases
        .iter()
        .filter(|c| c.check.is_some() && training.contains(&c.episode_ref))
        .filter_map(|c| {
            variants
                .get(&c.episode_ref)
                .filter(|v| *v != &c.input)
                .map(|input| {
                    let mut variant = c.clone();
                    variant.input = input.clone();
                    variant
                })
        })
        .collect();
    if !proposed.is_empty() {
        let mut variant_context = context.clone();
        variant_context["original_cases"] = json!(cases);
        let reviewed =
            inspect_rounds(host, spec, &mut proposed, &variant_context, check, false).await?;
        history["variants"] = reviewed;
        for variant in proposed.into_iter().filter(|v| v.check.is_some()) {
            if let Some(original) = cases
                .iter_mut()
                .find(|c| c.episode_ref == variant.episode_ref)
            {
                original.variant = Some(variant.input);
            }
        }
    }
    Ok(history)
}

async fn inspect_rounds(
    host: &RuntimeHost,
    spec: TerminalSpec,
    cases: &mut [TraceCase],
    context: &Value,
    check: &LearningCheck<'_>,
    allow_repair: bool,
) -> Result<Value, AppError> {
    let known: BTreeSet<String> = context["records"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["ref"].as_str())
        .chain(
            context["historical"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r["reference"].as_str()),
        )
        .map(str::to_owned)
        .collect();
    let mut pending: Vec<usize> = cases
        .iter()
        .enumerate()
        .filter(|(_, c)| c.check.is_some())
        .map(|(i, _)| i)
        .collect();
    let mut history = Vec::new();
    // One repair attempt per source sweep. Unresolved tasks remain withdrawn,
    // rather than extending this claim indefinitely or admitting unreviewed edits.
    for round in 0..if allow_repair { 2 } else { 1 } {
        if pending.is_empty() {
            break;
        }
        let inputs: Vec<_> = pending
            .iter()
            .map(|&i| json!({"episode_ref":cases[i].episode_ref,"input":cases[i].input}))
            .collect();
        let answer: String = check.call(host, "task_trial", HostRequest::EvaluateExperience {
            spec: Box::new(spec.clone()),
            input: format!("Independently attempt each task using only its input. If information is missing, explain what is missing rather than inventing it. Return only a JSON object mapping every episode_ref to a concise answer (at most 2000 characters each).\n{}", json!({"task_trials":inputs})),
            instruction:String::new(), preflight:String::new(),
        }).await?;
        let trials: BTreeMap<String, String> =
            serde_json::from_str(answer.trim()).map_err(|_| {
                AppError::Validation("Blind task trial returned invalid answers".into())
            })?;
        if trials.len() != pending.len()
            || pending
                .iter()
                .any(|&i| !trials.contains_key(&cases[i].episode_ref))
            || trials
                .values()
                .any(|a| a.trim().is_empty() || a.len() > 2000)
        {
            return Err(AppError::Validation(
                "Blind task trial must answer each supplied task within its bounds".into(),
            ));
        }
        let proposed: Vec<_> = pending.iter().map(|&i| &cases[i]).collect();
        let decisions: Vec<TaskDecision> = check
            .call(
                host,
                "task_quality_review",
                HostRequest::ReviewTasks {
                    spec: Box::new(spec.clone()),
                    prompt: json!({"cases":proposed,"context":context,"trials":trials}).to_string(),
                },
            )
            .await?;
        let by_ref: BTreeMap<_, _> = decisions.iter().map(|d| (&d.episode_ref, d)).collect();
        if decisions.len() != pending.len()
            || by_ref.len() != decisions.len()
            || pending
                .iter()
                .any(|&i| !by_ref.contains_key(&cases[i].episode_ref))
        {
            return Err(AppError::Validation(
                "Task review must decide every supplied task exactly once".into(),
            ));
        }
        let mut retry = Vec::new();
        for &index in &pending {
            let case = &mut cases[index];
            let decision = by_ref[&case.episode_ref];
            if decision.sensitive {
                history.retain(|entry: &Value| entry["episode_ref"] != case.episode_ref);
                history
                    .push(json!({"episode_ref":case.episode_ref,"round":round,"sensitive":true}));
                case.input = "[Sensitive task excluded]".into();
                case.check = None;
                case.reason = "Task excluded because it contains sensitive data".into();
                continue;
            }
            let grounded = decision.evidence.iter().all(|r| known.contains(r))
                && case_review_covers(std::slice::from_ref(case), &decision.evidence);
            history.push(json!({"episode_ref":case.episode_ref,"round":round,"input":case.input,"check":case.check,"trial":trials[&case.episode_ref],"decision":decision}));
            if grounded && decision.accepted {
                case.reason = decision.reason.clone();
                continue;
            }
            if grounded && allow_repair && round == 0 {
                if let Some(repair) = &decision.repair {
                    let mut repaired = case.clone();
                    repaired.input = repair.input.clone();
                    repaired.check = Some(repair.check.clone());
                    repaired.reason = repair.reason.clone();
                    if repaired.validate().is_ok() {
                        *case = repaired;
                        retry.push(index);
                        continue;
                    }
                }
            }
            case.check = None;
            case.reason = if grounded {
                decision.reason.clone()
            } else {
                "Task quarantined: review did not establish source-grounded evidence".into()
            };
        }
        pending = retry;
    }
    Ok(
        json!({"procedure_digest":hex::encode(sha2::Sha256::digest(choruz_host_runtime::experience::TASK_REVIEW_SKILL.as_bytes())),"attempts":history}),
    )
}
