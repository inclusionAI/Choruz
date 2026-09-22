//! Manual real-browser acceptance: only inference is deterministic. Run with a
//! connected BrowserSkill browser id; the fixture and Agent Window are owned.
use choruz_decision::{
    Answer, Provider, Question, Request, Response, Usage,
    programs::Operation,
    workflow::{Step, Workflow},
};
use choruz_host_runtime::browser_workflow::{self, Execution};
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct FixtureProvider {
    handoff: bool,
}
impl Provider for FixtureProvider {
    async fn decide(&self, request: Request) -> choruz_decision::Result<Response> {
        request.validate()?;
        let operation = if request.questions.contains_key("type_text") {
            "type_text"
        } else if request.questions.contains_key("select") {
            "select"
        } else {
            "click"
        };
        let mut answers = BTreeMap::new();
        for (name, question) in request.questions {
            let Question::Choice { criteria, .. } = question else {
                return Err(choruz_decision::Error::Transport);
            };
            let selected = if name == "operation" && self.handoff && operation != "select" {
                "blocked".to_owned()
            } else if name == "operation" {
                operation.to_owned()
            } else {
                criteria
                    .keys()
                    .find(|key| key.as_str() != "none")
                    .cloned()
                    .ok_or(choruz_decision::Error::Transport)?
            };
            answers.insert(
                name,
                Answer::Choice {
                    choice: selected.clone(),
                    confidence: 1.0,
                    probabilities: criteria
                        .keys()
                        .map(|key| (key.clone(), if *key == selected { 1.0 } else { 0.0 }))
                        .collect(),
                },
            );
        }
        Ok(Response {
            model: request.model,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
            },
            answers,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();
    let browser = std::env::args()
        .nth(1)
        .ok_or("Supply a connected BrowserSkill browser id")?;
    let assisted = std::env::args().nth(2).as_deref() == Some("--codex-handoff");
    let assistant = assisted.then(|| choruz_host_runtime::TerminalSpec {
        authentication: false,
        terminal_id: "browser-acceptance".into(),
        driver_type: "codex_terminal".into(),
        binary_path: None,
        workspace_path: String::new(),
        cols: 120,
        rows: 40,
        resume_session_id: None,
        codex_home: None,
        model: None,
        harness_account: serde_json::json!({}),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/", listener.local_addr()?);
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut bytes = [0; 4096];
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(5), socket.read(&mut bytes))
                    .await;
            let body = r#"<!doctype html><html lang="en"><title>Workflow acceptance fixture</title><label>Title<input id="title"></label><label>Category<select id="category"><option value="">Choose</option><option value="work">Work</option></select></label><button id="save">Save draft</button><p role="status" id="status">Editing draft</p><script>document.getElementById('save').onclick=()=>{document.getElementById('status').textContent='Draft saved: '+document.getElementById('title').value+' / '+document.getElementById('category').value;};</script></html>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    let result = browser_workflow::run(
        Execution {
            browser,
            url: url.clone(),
            model: "fixture".into(),
            minimum_confidence: 0.9,
            workflow: Workflow {
                name: "Save a draft".into(),
                allowed_urls: vec![url],
                steps: vec![
                    Step {
                        goal: "Fill the title".into(),
                        operation: Operation::TypeText,
                        labels: vec!["textbox \"Title\"".into()],
                        value_key: Some("title".into()),
                    },
                    Step {
                        goal: "Choose Work category".into(),
                        operation: Operation::Select,
                        labels: vec!["combobox \"Category [has-submenu]\"".into()],
                        value_key: Some("category".into()),
                    },
                    Step {
                        goal: "Save the draft".into(),
                        operation: Operation::Click,
                        labels: vec!["button \"Save draft\"".into()],
                        value_key: None,
                    },
                ],
            },
            values: if assisted {
                BTreeMap::from([("category".into(), "work".into())])
            } else {
                BTreeMap::from([
                    ("title".into(), "Quarterly report".into()),
                    ("category".into(), "work".into()),
                ])
            },
            expected_text: vec!["Draft saved: Quarterly report / work".into()],
            assist_with_agent: assisted,
            text_requests: if assisted {
                BTreeMap::from([(
                    "title".into(),
                    "Write the title Quarterly report exactly, without quotes or punctuation."
                        .into(),
                )])
            } else {
                BTreeMap::new()
            },
        },
        FixtureProvider { handoff: assisted },
        assistant,
    )
    .await;
    server.abort();
    let _ = server.await;
    let report = result?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.checks_matched || report.completed_steps != 3 {
        return Err("Browser workflow did not satisfy the fixture".into());
    }
    if assisted
        && (report.handoffs != 2
            || !report.actions.iter().all(|action| {
                (action.selected_by == "reasoning_agent" || action.operation == Operation::Select)
                    && action.confirmed
            })
            || report.generated_inputs != ["title"])
    {
        return Err("The native Harness did not resolve the browser handoffs".into());
    }
    Ok(())
}
