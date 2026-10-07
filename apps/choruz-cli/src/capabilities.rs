//! Thin command adapters; library policy and authenticated host mutations keep their owners.
use super::{Args, authenticate, read_json_response};
use choruz_learning::{native_source, source::Cursor};
use reqwest::{Client, Method};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs::File, io::Read, path::PathBuf};

const MAX_INPUT: u64 = 4 * 1024 * 1024;

pub(super) fn is_local(input: &[String]) -> bool {
    matches!(input.first().map(String::as_str), Some("library" | "tools"))
}

pub(super) fn validate(input: &[String]) -> Result<(), String> {
    let words: Vec<_> = input.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["library", "trace" | "community", _]
        | ["library", "score", _, _]
        | ["tools", "status"]
        | ["tools", "enable" | "disable", "browser" | "desktop"] => Ok(()),
        ["learning", "show" | "evaluations" | "community", binding]
        | [
            "learning",
            "configure" | "select" | "evaluate" | "community-configure" | "prepare",
            binding,
            _,
        ] if !binding.is_empty() => Ok(()),
        ["api", method, path, rest @ ..]
            if rest.len() <= 1
                && matches!(*method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE")
                && path.starts_with("/v1/")
                && !path.contains('#')
                && !path.chars().any(char::is_control)
                && (*method != "GET" || rest.is_empty()) =>
        {
            Ok(())
        }
        _ => Err("invalid capability command; see choruz --help".into()),
    }
}

fn text(path: &str) -> Result<String, String> {
    let mut content = String::new();
    File::open(path)
        .map_err(|e| format!("open input: {e}"))?
        .take(MAX_INPUT + 1)
        .read_to_string(&mut content)
        .map_err(|e| format!("read input: {e}"))?;
    if content.len() as u64 > MAX_INPUT {
        return Err("input exceeds 4 MiB".into());
    }
    Ok(content)
}

fn read<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    serde_json::from_str(&text(path)?).map_err(|e| format!("decode input JSON: {e}"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceSource {
    harness: String,
    account_home: PathBuf,
    workspace_path: PathBuf,
    session_id: String,
    #[serde(default)]
    cursor: Cursor,
}

pub(super) async fn local(input: &[String]) -> Result<Value, String> {
    validate(input)?;
    match (input[0].as_str(), input[1].as_str()) {
        ("library", "trace") => {
            let source: TraceSource = read(&input[2])?;
            let harness = match source.harness.as_str() {
                "claude" => native_source::Harness::Claude,
                "codex" => native_source::Harness::Codex,
                _ => return Err("trace harness must be claude or codex".into()),
            };
            let result = native_source::read(
                &native_source::Source {
                    harness,
                    account_home: source.account_home,
                    workspace_path: source.workspace_path,
                    session_id: source.session_id,
                },
                source.cursor,
            )
            .map_err(|e| e.to_string())?;
            serde_json::to_value(result).map_err(|e| e.to_string())
        }
        ("library", "score") => {
            let check: choruz_evaluation::evaluation::OutputCheck = read(&input[2])?;
            check.validate()?;
            let score = check.score(&text(&input[3])?);
            Ok(json!({"score":score,"requires_judge":score.is_none()}))
        }
        ("library", "community") => {
            let records: Vec<choruz_community::behavior::BehaviorRecord> = read(&input[2])?;
            for record in &records {
                record.validate()?;
            }
            serde_json::to_value(choruz_community::behavior::counts(&records))
                .map_err(|e| e.to_string())
        }
        ("tools", action) => {
            let tool = input.get(2).map(|name| {
                if name == "browser" {
                    choruz_computer_use::Tool::Browser
                } else {
                    choruz_computer_use::Tool::Desktop
                }
            });
            let mut result = choruz_computer_use::manage(tool, tool.map(|_| action == "enable"))
                .await
                .map_err(|e| e.to_string())?;
            // Keep the runtime alive until its owned asynchronous installer finishes.
            while result["tools"]
                .as_array()
                .is_some_and(|tools| tools.iter().any(|t| t["status"] == "installing"))
            {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                result = choruz_computer_use::manage(None, None)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            if action == "enable"
                && result["tools"]
                    .as_array()
                    .is_some_and(|tools| tools.iter().any(|t| t["status"] == "error"))
            {
                return Err(format!("tool installation failed: {result}"));
            }
            Ok(result)
        }
        _ => unreachable!("validated local command"),
    }
}

pub(super) async fn remote(
    client: &Client,
    args: &Args,
    input: &[String],
) -> Result<Value, String> {
    validate(input)?;
    let mut url =
        reqwest::Url::parse(&args.api_url).map_err(|e| format!("invalid API URL: {e}"))?;
    url.set_query(None);
    url.set_fragment(None);
    let (method, body) = if input[0] == "api" {
        // A relative API path cannot replace the configured host or receive its token elsewhere.
        url = url.join(&input[2]).map_err(|e| e.to_string())?;
        (
            Method::from_bytes(input[1].as_bytes()).map_err(|e| e.to_string())?,
            input.get(3),
        )
    } else {
        let (method, suffix) = match input[1].as_str() {
            "show" => (Method::GET, None),
            "configure" => (Method::PUT, None),
            "select" => (Method::PATCH, None),
            "evaluations" => (Method::GET, Some("evaluations")),
            "evaluate" => (Method::POST, Some("evaluations")),
            "community" => (Method::GET, Some("community")),
            "community-configure" => (Method::PUT, Some("community")),
            "prepare" => (Method::POST, Some("prepare")),
            _ => unreachable!("validated learning command"),
        };
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "API URL is not hierarchical")?;
        segments
            .clear()
            .extend(["v1", "runtime", "bindings", &input[2], "experience"]);
        if let Some(suffix) = suffix {
            segments.push(suffix);
        }
        drop(segments);
        (method, input.get(3))
    };
    let body = body.map(|path| read::<Value>(path)).transpose()?;
    let token = authenticate(client, args).await?;
    let mut request = client.request(method, url).bearer_auth(token);
    if input[0] == "learning" && input[1] == "prepare" {
        request = request.timeout(std::time::Duration::from_secs(90));
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("capability request: {e}"))?;
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    read_json_response(response, "capability request").await
}
