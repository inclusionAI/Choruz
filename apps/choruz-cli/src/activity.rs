use super::{Args, authenticate, authenticated_json_post, read_json_response};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, Write},
};

pub(super) fn parse(input: &[String]) -> Result<(&str, BTreeMap<&str, &str>), String> {
    let action = input
        .first()
        .map(String::as_str)
        .ok_or("activity requires an action")?;
    let allowed: &[&str] = match action {
        "list" | "export" | "summary" => &["--since", "--until", "--source", "--trace-id"],
        "messages" => &["--conversation", "--include-content"],
        "prune" => &["--before", "--apply"],
        _ => return Err("unknown activity action".into()),
    };
    let mut options = BTreeMap::new();
    let mut i = 1;
    while i < input.len() {
        let key = input[i].as_str();
        if !allowed.contains(&key) || options.contains_key(key) {
            return Err(format!("invalid or duplicate activity option: {key}"));
        }
        let value = if matches!(key, "--apply" | "--include-content") {
            "true"
        } else {
            i += 1;
            input
                .get(i)
                .filter(|v| !v.is_empty() && !v.starts_with("--"))
                .map(String::as_str)
                .ok_or_else(|| format!("{key} requires a value"))?
        };
        options.insert(key, value);
        i += 1;
    }
    let required: &[&str] = match action {
        "messages" => &["--conversation"],
        "prune" => &["--before"],
        _ => &["--since", "--until"],
    };
    for key in required {
        if !options.contains_key(key) {
            return Err(format!("activity {action} requires {key}"));
        }
    }
    Ok((action, options))
}

pub(super) async fn run(client: &Client, args: &Args, input: &[String]) -> Result<(), String> {
    let (action, options) = parse(input)?;
    if action == "prune" {
        let result = authenticated_json_post(
            client,
            args,
            "/v1/activity/prune",
            json!({"before":options["--before"],"apply":options.contains_key("--apply")}),
        )
        .await?;
        println!("{result}");
        return Ok(());
    }
    let mut url =
        reqwest::Url::parse(&args.api_url).map_err(|e| format!("invalid API URL: {e}"))?;
    url.set_query(None);
    url.set_fragment(None);
    if action == "messages" {
        url.path_segments_mut()
            .map_err(|_| "API URL cannot contain path segments")?
            .clear()
            .extend([
                "v1",
                "conversations",
                options["--conversation"],
                "interactions",
            ]);
        url.query_pairs_mut().append_pair(
            "include_content",
            if options.contains_key("--include-content") {
                "true"
            } else {
                "false"
            },
        );
    } else {
        url.set_path(if action == "summary" {
            "/v1/activity/summary"
        } else {
            "/v1/activity"
        });
        for (key, value) in &options {
            url.query_pairs_mut().append_pair(
                key.trim_start_matches("--").replace('-', "_").as_str(),
                value,
            );
        }
    }
    let token = authenticate(client, args).await?;
    let base = url.clone();
    loop {
        let response = client
            .get(url.clone())
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("read activity: {e}"))?;
        let result = read_json_response(response, "read activity").await?;
        if matches!(action, "list" | "summary") {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
            );
            return Ok(());
        }
        let records = result
            .get("records")
            .and_then(Value::as_array)
            .ok_or("activity response lacks records")?;
        let mut stdout = io::stdout().lock();
        for record in records {
            writeln!(stdout, "{record}").map_err(|e| format!("write activity export: {e}"))?;
        }
        drop(stdout);
        let cursor_key = if action == "messages" {
            "next_before_seq"
        } else {
            "next_cursor"
        };
        let Some(cursor) = result.get(cursor_key).filter(|v| !v.is_null()) else {
            return Ok(());
        };
        let value = cursor
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| cursor.to_string());
        url = base.clone();
        url.query_pairs_mut().append_pair(
            if action == "messages" {
                "before_seq"
            } else {
                "cursor"
            },
            &value,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn retention_requires_explicit_apply_and_filters_are_action_specific() {
        let args = input(&["prune", "--before", "2026-01-01T00:00:00Z"]);
        assert!(!parse(&args).unwrap().1.contains_key("--apply"));
        assert!(parse(&input(&["export", "--since", "x"])).is_err());
        assert!(parse(&input(&["messages", "--conversation", "a", "--apply"])).is_err());
        assert!(parse(&input(&["prune", "--before", "a", "--before", "b"])).is_err());
        assert!(
            parse(&input(&[
                "messages",
                "--conversation",
                "a",
                "--include-content"
            ]))
            .is_ok()
        );
    }
}
