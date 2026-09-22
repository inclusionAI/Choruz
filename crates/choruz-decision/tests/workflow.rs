use choruz_decision::{
    Answer, Error, Provider, Question, Request, Response, Usage,
    programs::{Element, Observation, Operation},
    workflow::{self, Browser, Snapshot, Step, Workflow},
};
use std::collections::BTreeMap;

struct SelectFirst {
    confidence: f64,
    model: &'static str,
}
impl Provider for SelectFirst {
    async fn decide(&self, request: Request) -> choruz_decision::Result<Response> {
        request.validate()?;
        let operation = if request.questions.contains_key("type_text") {
            "type_text"
        } else {
            "click"
        };
        Ok(Response {
            model: self.model.into(),
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
            answers: request
                .questions
                .iter()
                .map(|(name, question)| {
                    let Question::Choice { criteria, .. } = question else {
                        panic!("choice")
                    };
                    let selected = if name == "operation" {
                        operation
                    } else {
                        criteria.keys().find(|key| key.as_str() != "none").unwrap()
                    };
                    (
                        name.clone(),
                        Answer::Choice {
                            choice: selected.into(),
                            confidence: self.confidence,
                            probabilities: criteria
                                .keys()
                                .map(|key| (key.clone(), if key == selected { 1.0 } else { 0.0 }))
                                .collect(),
                        },
                    )
                })
                .collect(),
        })
    }
}

#[derive(Default)]
struct Form {
    actions: Vec<(Operation, String, Option<String>)>,
    observed: usize,
    fail_at: Option<usize>,
    saved: bool,
}
impl Browser for Form {
    async fn observe(&mut self) -> choruz_decision::Result<Snapshot> {
        self.observed += 1;
        Ok(Snapshot {
            observation: Observation {
                id: self.observed.to_string(),
                url: "https://example.org/form".into(),
                elements: vec![
                    Element {
                        id: format!("field-{}", self.observed),
                        description: "Title".into(),
                        operations: vec![Operation::TypeText],
                    },
                    Element {
                        id: format!("save-{}", self.observed),
                        description: "Save draft".into(),
                        operations: vec![Operation::Click],
                    },
                    Element {
                        id: "a-dangerous".into(),
                        description: "Publish immediately".into(),
                        operations: vec![Operation::Click],
                    },
                ],
            },
            text: if self.saved {
                "Draft saved"
            } else {
                "Editing draft"
            }
            .into(),
        })
    }
    async fn apply(
        &mut self,
        snapshot: &Snapshot,
        operation: Operation,
        target: &str,
        value: Option<&str>,
    ) -> choruz_decision::Result<()> {
        assert_eq!(snapshot.observation.id, self.observed.to_string());
        assert!(
            target.ends_with(&self.observed.to_string()),
            "fresh target required"
        );
        self.actions
            .push((operation, target.into(), value.map(str::to_owned)));
        if self.fail_at == Some(self.actions.len()) {
            return Err(Error::Transport);
        }
        if operation == Operation::Click {
            self.saved = true;
        }
        Ok(())
    }
}

fn workflow() -> Workflow {
    Workflow {
        name: "Save draft".into(),
        allowed_urls: vec!["https://example.org/form".into()],
        steps: vec![
            Step {
                goal: "Fill draft title".into(),
                operation: Operation::TypeText,
                labels: vec!["Title".into()],
                value_key: Some("title".into()),
            },
            Step {
                goal: "Save without publishing".into(),
                operation: Operation::Click,
                labels: vec!["Save draft".into()],
                value_key: None,
            },
        ],
    }
}

#[tokio::test]
async fn fresh_targets_fixed_values_and_independent_postcondition() {
    let mut browser = Form::default();
    let provider = SelectFirst {
        confidence: 0.99,
        model: "test",
    };
    let values = BTreeMap::from([("title".into(), "Quarterly report".into())]);
    let result = workflow::run(
        &mut browser,
        &provider,
        &workflow(),
        "test",
        &values,
        &["Draft saved".into()],
        0.9,
    )
    .await
    .unwrap();
    assert!(result.checks_matched);
    assert_eq!(result.completed_steps, 2);
    assert_eq!(
        browser.actions,
        vec![
            (
                Operation::TypeText,
                "field-1".into(),
                Some("Quarterly report".into())
            ),
            (Operation::Click, "save-2".into(), None)
        ]
    );
    let result = workflow::run(
        &mut browser,
        &provider,
        &workflow(),
        "test",
        &values,
        &["Published".into()],
        0.9,
    )
    .await
    .unwrap();
    assert!(
        !result.checks_matched,
        "successful actions are not evidence of the expected outcome"
    );
}

#[tokio::test]
async fn uncertain_or_different_model_never_acts_and_ambiguous_mutation_never_retries() {
    let values = BTreeMap::from([("title".into(), "Draft".into())]);
    for (confidence, model) in [(0.5, "test"), (0.99, "different-model")] {
        let mut browser = Form::default();
        let result = workflow::run(
            &mut browser,
            &SelectFirst { confidence, model },
            &workflow(),
            "test",
            &values,
            &["Draft saved".into()],
            0.9,
        )
        .await
        .unwrap();
        assert_eq!(result.reason, "decision_handback");
        assert!(browser.actions.is_empty());
    }
    let mut browser = Form {
        fail_at: Some(2),
        ..Default::default()
    };
    let result = workflow::run(
        &mut browser,
        &SelectFirst {
            confidence: 0.99,
            model: "test",
        },
        &workflow(),
        "test",
        &values,
        &["Draft saved".into()],
        0.9,
    )
    .await
    .unwrap();
    assert_eq!(result.reason, "action_outcome_unconfirmed");
    assert_eq!(result.completed_steps, 1);
    assert_eq!(browser.actions.len(), 2);
    assert!(!result.checks_matched);
    assert_eq!(result.actions.len(), 2);
    assert!(result.actions[0].confirmed);
    assert!(!result.actions[1].confirmed);
    assert_eq!(result.handoffs, 0);
}

struct ReasoningAgent {
    escape_scope: bool,
    calls: std::cell::Cell<usize>,
}

impl workflow::Assistant for ReasoningAgent {
    async fn select(
        &self,
        _: &str,
        observation: &Observation,
    ) -> choruz_decision::Result<Option<String>> {
        self.calls.set(self.calls.get() + 1);
        assert!(
            observation
                .elements
                .iter()
                .all(|target| target.description != "Publish immediately")
        );
        Ok(Some(if self.escape_scope {
            "a-dangerous".into()
        } else {
            observation.elements[0].id.clone()
        }))
    }
}

#[tokio::test]
async fn reasoning_handoff_continues_only_with_current_authorized_targets() {
    let values = BTreeMap::from([("title".into(), "Reviewed draft".into())]);
    for escape_scope in [false, true] {
        let assistant = ReasoningAgent {
            escape_scope,
            calls: std::cell::Cell::new(0),
        };
        let mut browser = Form::default();
        let result = workflow::run_with_assistant(
            &mut browser,
            &SelectFirst {
                confidence: 0.5,
                model: "test",
            },
            &assistant,
            &workflow(),
            "test",
            &values,
            &["Draft saved".into()],
            0.9,
        )
        .await
        .unwrap();
        if escape_scope {
            assert!(!result.checks_matched);
            assert!(
                browser.actions.is_empty(),
                "assistant proposals cannot expand the reviewed scope"
            );
            assert_eq!(result.reason, "decision_handback");
        } else {
            assert!(result.checks_matched);
            assert_eq!(result.completed_steps, 2);
            assert_eq!(result.handoffs, 2);
            assert!(
                result
                    .actions
                    .iter()
                    .all(|action| action.confirmed && action.selected_by == "reasoning_agent")
            );
            assert_eq!(browser.actions[0].2.as_deref(), Some("Reviewed draft"));
        }
        assert_eq!(assistant.calls.get(), result.handoffs);
    }
    let assistant = ReasoningAgent {
        escape_scope: false,
        calls: std::cell::Cell::new(0),
    };
    let mut browser = Form {
        fail_at: Some(1),
        ..Default::default()
    };
    let result = workflow::run_with_assistant(
        &mut browser,
        &SelectFirst {
            confidence: 0.5,
            model: "test",
        },
        &assistant,
        &workflow(),
        "test",
        &values,
        &["Draft saved".into()],
        0.9,
    )
    .await
    .unwrap();
    assert_eq!(result.reason, "action_outcome_unconfirmed");
    assert_eq!(browser.actions.len(), 1);
    assert_eq!(
        assistant.calls.get(),
        1,
        "uncertain mutation must not trigger a repair retry"
    );
}

#[tokio::test]
async fn unrelated_page_and_preexisting_success_are_not_execution_evidence() {
    let values = BTreeMap::from([("title".into(), "Draft".into())]);
    let provider = SelectFirst {
        confidence: 0.99,
        model: "test",
    };
    let mut browser = Form::default();
    let mut manifest = workflow();
    manifest.allowed_urls = vec!["https://another-site.example/form".into()];
    let result = workflow::run(
        &mut browser,
        &provider,
        &manifest,
        "test",
        &values,
        &["Draft saved".into()],
        0.9,
    )
    .await
    .unwrap();
    assert_eq!(result.reason, "page_out_of_scope");
    assert!(browser.actions.is_empty());
    browser.saved = true;
    let result = workflow::run(
        &mut browser,
        &provider,
        &workflow(),
        "test",
        &values,
        &["Draft saved".into()],
        0.9,
    )
    .await
    .unwrap();
    assert_eq!(result.reason, "postcondition_already_present");
    assert!(!result.checks_matched);
    assert!(browser.actions.is_empty());
    let mut selection = workflow();
    selection.steps[0].operation = Operation::Select;
    selection.validate().unwrap();
    selection.steps[0].value_key = None;
    assert!(
        selection.validate().is_err(),
        "selection requires an explicit fixed input key"
    );
}
