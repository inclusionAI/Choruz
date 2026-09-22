use choruz_decision::{
    Answer, Provider, Question, Request, Response, Usage,
    programs::*,
    task::{DecisionTask, Job},
};
use serde_json::json;
use std::collections::BTreeMap;

struct Selected {
    choices: BTreeMap<String, String>,
    confidence: f64,
}
impl Provider for Selected {
    async fn decide(&self, request: Request) -> choruz_decision::Result<Response> {
        request.validate()?;
        Ok(Response {
            model: "test-version".into(),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 2,
            },
            answers: request
                .questions
                .iter()
                .map(|(id, q)| {
                    let Question::Choice { criteria, .. } = q else {
                        panic!("choice expected")
                    };
                    let selected = self.choices.get(id).expect("test supplies every head");
                    (
                        id.clone(),
                        Answer::Choice {
                            choice: selected.clone(),
                            confidence: self.confidence,
                            probabilities: criteria
                                .keys()
                                .map(|k| (k.clone(), if k == selected { 1.0 } else { 0.0 }))
                                .collect(),
                        },
                    )
                })
                .collect(),
        })
    }
}

fn program() -> Program {
    Program {
        name: "Route ticket".into(),
        applicability: "Support tickets".into(),
        questions: BTreeMap::from([(
            "result".into(),
            choice(
                "Select queue",
                [("billing", "Billing issue"), ("abstain", "Unknown")],
            ),
        )]),
        result_question: "result".into(),
        outputs: BTreeMap::from([("billing".into(), "billing_queue".into())]),
        minimum_confidence: 0.5,
    }
}

#[test]
fn structured_questions_preserve_provider_semantics_and_reject_invalid_rubrics() {
    let wire = json!({"model":"test", "state":{"ticket":"Refund"}, "questions":{
        "queue":{"type":"choice", "instructions":{"question":"Select queue", "policy":["Use invoice evidence"]}, "criteria":{"billing":{"includes":["refund"]},"other":null}},
        "severity":{"type":"score", "instructions":["Rate urgency"], "criteria":[{"level":"Routine"},["Immediate outage"]]},
        "urgent":{"type":"noul", "instructions":"Is the request urgent?", "criteria":{"true":{"when":"Explicit deadline"},"false":null}}
    }});
    let request: Request = serde_json::from_value(wire.clone()).unwrap();
    request.validate().unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    let response: Response = serde_json::from_value(json!({"model":"test", "usage":{"input_tokens":20,"output_tokens":10}, "answers":{
        "queue":{"type":"choice","choice":"billing","probabilities":{"billing":0.9,"other":0.1},"confidence":0.8},
        "severity":{"type":"score","score":0.25,"probabilities":{"0":0.75,"1":0.25},"confidence":0.5},
        "urgent":{"type":"noul","noul":0.1}
    }})).unwrap();
    response.validate(&request).unwrap();
    for (pointer, invalid) in [
        ("/questions/queue/instructions", json!(true)),
        ("/questions/queue/instructions", json!({})),
        ("/questions/queue/criteria/billing", json!(42)),
        ("/questions/severity/criteria/0", json!(null)),
        ("/questions/urgent/criteria", json!({"maybe":"Unclear"})),
        (
            "/questions/urgent/criteria/true",
            json!({"text":"x".repeat(4001)}),
        ),
    ] {
        let mut changed = wire.clone();
        *changed.pointer_mut(pointer).unwrap() = invalid;
        let request: Request = serde_json::from_value(changed).unwrap();
        assert!(request.validate().is_err(), "accepted invalid {pointer}");
    }
}

#[tokio::test]
async fn generated_program_cannot_lower_operator_threshold_or_answer_out_of_scope() {
    let mut provider = Selected {
        choices: BTreeMap::from([
            ("result".into(), "billing".into()),
            ("_applicable".into(), "yes".into()),
        ]),
        confidence: 0.7,
    };
    let task = DecisionTask {
        model: "test".into(),
        minimum_confidence: 0.8,
        job: Job::Program {
            program: program(),
            state: json!("refund request"),
        },
    };
    assert!(task.clone().run(&provider).await.unwrap()["output"].is_null());
    provider.confidence = 0.9;
    assert_eq!(
        task.clone().run(&provider).await.unwrap()["output"],
        "billing_queue"
    );
    provider.choices.insert("_applicable".into(), "no".into());
    assert!(task.run(&provider).await.unwrap()["output"].is_null());
    let mut invalid = program();
    invalid
        .outputs
        .insert("abstain".into(), "silently answer".into());
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn browser_targets_are_bound_to_observation_and_supported_operation() {
    let observation = Observation {
        id: "snapshot-7".into(),
        url: "https://example.org".into(),
        elements: vec![
            Element {
                id: "button-1".into(),
                description: "Next page".into(),
                operations: vec![Operation::Click],
            },
            Element {
                id: "input-1".into(),
                description: "Search query".into(),
                operations: vec![Operation::TypeText],
            },
        ],
    };
    let mut provider = Selected {
        choices: BTreeMap::from([
            ("operation".into(), "click".into()),
            ("click".into(), "button-1".into()),
            ("type_text".into(), "input-1".into()),
        ]),
        confidence: 0.95,
    };
    let selected = browser_action(&provider, "test", "Next page", observation.clone(), 0.8)
        .await
        .unwrap();
    assert_eq!(selected.observation_id, "snapshot-7");
    assert_eq!(selected.element_id.as_deref(), Some("button-1"));
    provider.choices.insert("click".into(), "input-1".into());
    assert!(
        browser_action(&provider, "test", "Next page", observation.clone(), 0.8)
            .await
            .is_err()
    );
    provider.choices.insert("click".into(), "none".into());
    let blocked = browser_action(&provider, "test", "Next page", observation, 0.8)
        .await
        .unwrap();
    assert_eq!(blocked.operation, "blocked");
    assert!(blocked.element_id.is_none());
}
