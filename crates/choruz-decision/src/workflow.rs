//! Reusable browser steps use live semantic targets, never recorded refs.
//! Callers own the browser session, authorization and cleanup. Outcome checks
//! come from the caller, not from the decision model's completion claim.
use crate::{Error, Provider, Result, bounded, programs};
use programs::{Observation, Operation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub name: String,
    pub allowed_urls: Vec<String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub goal: String,
    pub operation: Operation,
    pub labels: Vec<String>,
    pub value_key: Option<String>,
}

impl Workflow {
    pub fn validate(&self) -> Result<()> {
        if !bounded(&self.name, 128)
            || self.allowed_urls.is_empty()
            || self.allowed_urls.len() > 16
            || self.allowed_urls.iter().any(|url| {
                !bounded(url, 4000) || !(url.starts_with("https://") || url.starts_with("http://"))
            })
            || self.steps.is_empty()
            || self.steps.len() > 16
            || self.steps.iter().any(|step| {
                !bounded(&step.goal, 2000)
                    || step.labels.is_empty()
                    || step.labels.len() > 16
                    || step.labels.iter().any(|label| !bounded(label, 256))
                    || match step.operation {
                        Operation::Click => step.value_key.is_some(),
                        Operation::TypeText | Operation::Select => {
                            step.value_key.as_ref().is_none_or(|key| !bounded(key, 128))
                        }
                    }
            })
        {
            return Err(Error::Invalid(
                "browser workflow requires bounded semantic steps and explicit value keys".into(),
            ));
        }
        Ok(())
    }

    /// Validate the complete run before opening a browser or contacting a provider.
    pub fn validate_run(
        &self,
        model: &str,
        values: &BTreeMap<String, String>,
        expected_text: &[String],
        minimum_confidence: f64,
    ) -> Result<()> {
        self.validate()?;
        if !bounded(model, 128)
            || !crate::probability(minimum_confidence)
            || expected_text.is_empty()
            || expected_text.len() > 16
            || expected_text.iter().any(|text| !bounded(text, 1000))
            || values.len() > 16
            || values
                .iter()
                .any(|(key, value)| !bounded(key, 128) || value.len() > 4000)
            || self.steps.iter().any(|step| {
                step.value_key
                    .as_ref()
                    .is_some_and(|key| !values.contains_key(key))
            })
        {
            return Err(Error::Invalid(
                "browser run requires bounded inputs and independent expected text".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub observation: Observation,
    pub text: String,
}

/// `apply` must reject stale observations before acting. An ambiguous action
/// failure is never retried here; the native Agent must inspect its outcome.
pub trait Browser {
    fn observe(&mut self) -> impl std::future::Future<Output = Result<Snapshot>>;
    fn apply(
        &mut self,
        snapshot: &Snapshot,
        operation: Operation,
        element_id: &str,
        value: Option<&str>,
    ) -> impl std::future::Future<Output = Result<()>>;
}

/// A reasoning Agent can resolve an abstention only within the current approved
/// targets. It receives no authority to change steps, values or outcome checks.
pub trait Assistant {
    fn enabled(&self) -> bool {
        true
    }
    fn select(
        &self,
        goal: &str,
        observation: &Observation,
    ) -> impl std::future::Future<Output = Result<Option<String>>>;
}

pub struct NoAssistant;

impl Assistant for NoAssistant {
    fn enabled(&self) -> bool {
        false
    }
    async fn select(&self, _: &str, _: &Observation) -> Result<Option<String>> {
        Ok(None)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub observation_id: String,
    pub element_id: String,
    pub operation: Operation,
    pub selected_by: String,
    pub confirmed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub checks_matched: bool,
    pub completed_steps: usize,
    pub reason: String,
    pub decisions: Vec<programs::BrowserSelection>,
    pub baseline_observation: Option<String>,
    pub verification_observation: Option<String>,
    pub actions: Vec<ActionReceipt>,
    pub handoffs: usize,
    pub generated_inputs: Vec<String>,
    pub elapsed_ms: u64,
}

/// Execute an already-authorized workflow. Exact visible-text checks are an
/// observable postcondition, not a general assertion of business correctness.
/// The caller must impose its own deadline and close the browser on cancellation.
pub async fn run(
    browser: &mut impl Browser,
    provider: &impl Provider,
    workflow: &Workflow,
    model: &str,
    values: &BTreeMap<String, String>,
    expected_text: &[String],
    minimum_confidence: f64,
) -> Result<Report> {
    run_with_assistant(
        browser,
        provider,
        &NoAssistant,
        workflow,
        model,
        values,
        expected_text,
        minimum_confidence,
    )
    .await
}

/// An explicitly enabled assistant may resolve an abstention before an action.
/// It cannot retry an uncertain mutation or choose a target outside the manifest.
#[allow(clippy::too_many_arguments)]
pub async fn run_with_assistant(
    browser: &mut impl Browser,
    provider: &impl Provider,
    assistant: &impl Assistant,
    workflow: &Workflow,
    model: &str,
    values: &BTreeMap<String, String>,
    expected_text: &[String],
    minimum_confidence: f64,
) -> Result<Report> {
    let started = std::time::Instant::now();
    let mut report = run_steps(
        browser,
        provider,
        assistant,
        workflow,
        model,
        values,
        expected_text,
        minimum_confidence,
    )
    .await?;
    report.elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
async fn run_steps(
    browser: &mut impl Browser,
    provider: &impl Provider,
    assistant: &impl Assistant,
    workflow: &Workflow,
    model: &str,
    values: &BTreeMap<String, String>,
    expected_text: &[String],
    minimum_confidence: f64,
) -> Result<Report> {
    workflow.validate_run(model, values, expected_text, minimum_confidence)?;
    let mut report = Report {
        checks_matched: false,
        completed_steps: 0,
        reason: "checks_not_matched".into(),
        decisions: Vec::new(),
        baseline_observation: None,
        verification_observation: None,
        actions: Vec::new(),
        handoffs: 0,
        generated_inputs: Vec::new(),
        elapsed_ms: 0,
    };
    for step in &workflow.steps {
        let snapshot = match browser.observe().await {
            Ok(snapshot) => snapshot,
            Err(_) => {
                report.reason = "observation_failed".into();
                return Ok(report);
            }
        };
        if !workflow.allowed_urls.contains(&snapshot.observation.url) {
            report.reason = "page_out_of_scope".into();
            return Ok(report);
        }
        if report.baseline_observation.is_none() {
            report.baseline_observation = Some(snapshot.observation.id.clone());
            if expected_text
                .iter()
                .any(|text| snapshot.text.contains(text))
            {
                report.reason = "postcondition_already_present".into();
                return Ok(report);
            }
        }
        let mut observation = snapshot.observation.clone();
        observation.elements.retain(|element| {
            step.labels.contains(&element.description)
                && element.operations.contains(&step.operation)
        });
        for element in &mut observation.elements {
            element.operations = vec![step.operation];
        }
        let unique_labels: std::collections::BTreeSet<_> = observation
            .elements
            .iter()
            .map(|element| &element.description)
            .collect();
        if unique_labels.len() != observation.elements.len() {
            report.reason = "ambiguous_target".into();
            return Ok(report);
        }
        if observation.elements.is_empty() {
            report.reason = "target_unavailable".into();
            return Ok(report);
        }
        let selection = match programs::browser_action(
            provider,
            model,
            &step.goal,
            observation.clone(),
            minimum_confidence,
        )
        .await
        {
            Ok(selection) => selection,
            Err(_) => {
                report.reason = "decision_failed".into();
                return Ok(report);
            }
        };
        let expected_operation = match step.operation {
            Operation::Click => "click",
            Operation::TypeText => "type_text",
            Operation::Select => "select",
        };
        let target = selection.element_id.clone();
        let accepted =
            selection.operation == expected_operation && selection.decision.model == model;
        report.decisions.push(selection);
        let (target, selected_by) = match target.filter(|_| accepted) {
            Some(target) => (target, "decision"),
            None => {
                // Never reinterpret a different provider model as an abstention.
                if !assistant.enabled()
                    || report.handoffs >= 2
                    || report
                        .decisions
                        .last()
                        .is_some_and(|selection| selection.decision.model != model)
                {
                    report.reason = "decision_handback".into();
                    return Ok(report);
                }
                report.handoffs += 1;
                match assistant.select(&step.goal, &observation).await {
                    Ok(Some(target))
                        if observation
                            .elements
                            .iter()
                            .any(|element| element.id == target) =>
                    {
                        (target, "reasoning_agent")
                    }
                    _ => {
                        report.reason = "decision_handback".into();
                        return Ok(report);
                    }
                }
            }
        };
        let value = step
            .value_key
            .as_ref()
            .and_then(|key| values.get(key))
            .map(String::as_str);
        // No retry: a transport error may occur after the browser mutated.
        let confirmed = browser
            .apply(&snapshot, step.operation, &target, value)
            .await
            .is_ok();
        report.actions.push(ActionReceipt {
            observation_id: snapshot.observation.id.clone(),
            element_id: target,
            operation: step.operation,
            selected_by: selected_by.into(),
            confirmed,
        });
        if !confirmed {
            report.reason = "action_outcome_unconfirmed".into();
            return Ok(report);
        }
        report.completed_steps += 1;
    }
    let final_state = match browser.observe().await {
        Ok(snapshot) => snapshot,
        Err(_) => {
            report.reason = "verification_unavailable".into();
            return Ok(report);
        }
    };
    report.verification_observation = Some(final_state.observation.id.clone());
    if !workflow.allowed_urls.contains(&final_state.observation.url) {
        report.reason = "page_out_of_scope".into();
        return Ok(report);
    }
    report.checks_matched = expected_text
        .iter()
        .all(|text| final_state.text.contains(text));
    if report.checks_matched {
        report.reason = "checks_matched".into();
    }
    Ok(report)
}
