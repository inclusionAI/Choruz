//! Harness events projected into conversation items without interpreting terminal pixels.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionItem {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub position: usize,
    pub id: String,
    pub kind: String,
    pub text: String,
    pub detail: Value,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionSnapshot {
    #[serde(default)]
    pub history_truncated: bool,
    #[serde(default)]
    pub retained_from: usize,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub instance: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub cursor: u64,
    #[serde(default)]
    pub more: bool,
    pub session_id: Option<String>,
    #[serde(default)]
    pub fork_source: Option<String>,
    #[serde(default)]
    pub native_home_path: Option<String>,
    #[serde(default)]
    pub native_session_path: Option<String>,
    pub turn_id: Option<String>,
    pub status: String,
    pub items: Vec<SessionItem>,
    pub requests: Vec<Value>,
    pub error: Option<String>,
    #[serde(skip)]
    stream_message_id: String,
    #[serde(skip)]
    final_message_id: String,
    #[serde(skip)]
    final_block_offset: usize,
    #[serde(skip)]
    final_fragments: HashMap<String, usize>,
    #[serde(skip)]
    preview_sizes: HashMap<String, (u64, usize)>,
    #[serde(skip)]
    turn_start_position: usize,
}

impl SessionSnapshot {
    /// Reconstructed native item ids need not equal live notification ids.
    /// Replace the preview at resume, using full native items rather than
    /// appending a display-summary replay to the previously saved projection.
    pub fn restore_codex_history(&mut self, result: &Value) -> Result<(), choruz_common::AppError> {
        let page = &result["initialTurnsPage"];
        let turns = if let Some(turns) = page["data"].as_array() {
            turns.iter().rev().collect::<Vec<_>>()
        } else if self.items.is_empty()
            && result["thread"]["turns"]
                .as_array()
                .is_some_and(Vec::is_empty)
        {
            Vec::new()
        } else {
            return Err(choruz_common::AppError::Validation("Codex did not return full native history. Update Codex or use Terminal to continue this session.".into()));
        };
        self.items.clear();
        self.preview_sizes.clear();
        self.retained_from = 0;
        self.history_truncated = !page["nextCursor"].is_null();
        for turn in turns {
            let start = self.next_position();
            if let Some(items) = turn["items"].as_array() {
                for item in items {
                    self.codex_item(item);
                }
            }
            if turn["status"] == "interrupted" {
                self.interrupt_items_from(start);
            }
        }
        Ok(())
    }

    /// Keep a rolling preview; the Harness owns the complete native transcript.
    pub fn enforce_limits(&mut self) {
        let mut sizes = Vec::with_capacity(self.items.len());
        for item in &mut self.items {
            if let Some((revision, size)) = self.preview_sizes.get(&item.id)
                && *revision == item.revision
            {
                sizes.push(*size);
                continue;
            }
            let mut truncated = false;
            if item.text.len() > 16 * 1024 {
                clip(&mut item.text, 16 * 1024);
                truncated = true;
            }
            let detail = item.detail.to_string();
            if detail.len() > 24 * 1024 {
                let mut preview = detail;
                clip(&mut preview, 24 * 1024);
                item.detail = json!({"preview":preview,"truncated":true});
                truncated = true;
            }
            if truncated {
                self.history_truncated = true;
            }
            let size = serde_json::to_vec(&item).map_or(0, |value| value.len());
            self.preview_sizes
                .insert(item.id.clone(), (item.revision, size));
            sizes.push(size);
        }
        let mut bytes: usize = sizes.iter().sum();
        let mut remove = 0;
        while self.items.len() - remove > 512 || bytes > 4 * 1024 * 1024 {
            bytes -= sizes[remove];
            remove += 1;
        }
        if remove > 0 {
            for item in self.items.drain(..remove) {
                self.preview_sizes.remove(&item.id);
            }
            self.history_truncated = true;
        }
        self.retained_from = self
            .items
            .first()
            .map_or(self.retained_from, |item| item.position);
    }

    /// Pages are ordered by mutation revision, not transcript position. A client
    /// merges by item id, then sorts by position; replay never duplicates a row.
    pub fn page(mut self, after: u64) -> Self {
        self.items.retain(|item| item.revision > after);
        self.items.sort_by_key(|item| item.revision);
        let mut bytes = 0;
        let mut count = 0;
        for item in &mut self.items {
            clip(&mut item.text, 16 * 1024);
            let detail = item.detail.to_string();
            if detail.len() > 24 * 1024 {
                let mut preview = detail;
                clip(&mut preview, 24 * 1024);
                item.detail = json!({"preview":preview,"truncated":true});
            }
            let size = serde_json::to_vec(item).map_or(0, |value| value.len());
            if count > 0 && bytes + size > 192 * 1024 {
                break;
            }
            bytes += size;
            count += 1;
        }
        self.more = count < self.items.len();
        self.items.truncate(count);
        self.cursor = if self.more {
            self.items.last().map_or(after, |item| item.revision)
        } else {
            self.revision
        };
        self
    }

    fn put(&mut self, mut item: SessionItem) {
        self.revision += 1;
        item.revision = self.revision;
        if let Some(existing) = self.items.iter_mut().find(|old| old.id == item.id) {
            item.position = existing.position;
            *existing = item;
        } else {
            item.position = self
                .items
                .last()
                .map_or(self.retained_from, |last| last.position + 1);
            self.items.push(item);
        }
        self.enforce_limits();
    }

    fn delta(&mut self, id: String, kind: &str, text: &str) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            self.revision += 1;
            item.revision = self.revision;
            item.text.push_str(text);
            self.enforce_limits();
        } else {
            self.put(SessionItem {
                revision: 0,
                position: 0,
                id,
                kind: kind.into(),
                text: text.into(),
                detail: Value::Null,
                status: "running".into(),
            });
        }
    }

    pub fn codex_item(&mut self, item: &Value) {
        let Some(id) = item["id"].as_str() else {
            return;
        };
        let kind = match item["type"].as_str() {
            Some("userMessage") => "user",
            Some("agentMessage") => "assistant",
            Some("reasoning") => "reasoning",
            _ => "tool",
        };
        let text = if kind == "user" {
            item["content"]
                .as_array()
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|block| block["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default()
        } else {
            item["text"]
                .as_str()
                .or_else(|| item["command"].as_str())
                .or_else(|| item["type"].as_str())
                .unwrap_or_default()
                .into()
        };
        self.put(SessionItem {
            revision: 0,
            position: 0,
            id: id.into(),
            kind: kind.into(),
            text,
            detail: item.clone(),
            status: item["status"].as_str().unwrap_or("completed").into(),
        });
    }

    pub fn codex(&mut self, event: &Value) {
        self.revision += 1;
        let params = &event["params"];
        if event.get("id").is_some() && event.get("method").is_some() {
            self.requests.retain(|request| request["id"] != event["id"]);
            self.requests.push(event.clone());
            self.status = "waiting".into();
            return;
        }
        match event["method"].as_str().unwrap_or_default() {
            "thread/started" => {
                self.session_id = params["thread"]["id"].as_str().map(str::to_owned)
            }
            "turn/started" => {
                self.turn_start_position = self.next_position();
                self.turn_id = params["turn"]["id"].as_str().map(str::to_owned);
                self.status = "running".into();
            }
            "turn/completed" => {
                if params["turn"]["status"] == "interrupted" {
                    self.interrupt_items_from(self.turn_start_position);
                }
                self.status = "ready".into();
                self.requests.clear();
                self.turn_id = None;
                if !params["turn"]["error"].is_null() {
                    self.error = Some(params["turn"]["error"].to_string());
                    self.status = "failed".into();
                }
            }
            "item/started" | "item/completed" => self.codex_item(&params["item"]),
            "item/agentMessage/delta" => {
                if let (Some(id), Some(delta)) =
                    (params["itemId"].as_str(), params["delta"].as_str())
                {
                    self.delta(id.into(), "assistant", delta);
                }
            }
            "item/commandExecution/outputDelta" => {
                if let Some(item) = self
                    .items
                    .iter_mut()
                    .find(|item| Some(item.id.as_str()) == params["itemId"].as_str())
                {
                    item.revision = self.revision;
                    let prior = item.detail["aggregatedOutput"].as_str().unwrap_or_default();
                    item.detail["aggregatedOutput"] = json!(format!(
                        "{prior}{}",
                        params["delta"].as_str().unwrap_or_default()
                    ));
                }
            }
            "serverRequest/resolved" => {
                self.requests
                    .retain(|request| request["id"] != params["requestId"]);
                if self.requests.is_empty() && self.turn_id.is_some() {
                    self.status = "running".into();
                }
            }
            "error" => {
                self.error = Some(
                    params["message"]
                        .as_str()
                        .unwrap_or("Codex reported an error")
                        .into(),
                );
            }
            _ => {}
        }
        self.enforce_limits();
    }

    pub fn claude(&mut self, event: &Value) {
        self.revision += 1;
        if let Some(id) = event["session_id"].as_str() {
            self.session_id = Some(id.into());
        }
        match event["type"].as_str().unwrap_or_default() {
            "stream_event" => {
                let stream = &event["event"];
                let index = stream["index"].as_u64().unwrap_or_default();
                match stream["type"].as_str().unwrap_or_default() {
                    "message_start" => {
                        self.stream_message_id =
                            stream["message"]["id"].as_str().unwrap_or_default().into();
                    }
                    "content_block_delta" if !self.stream_message_id.is_empty() => {
                        if let Some(text) = stream["delta"]["text"].as_str() {
                            self.delta(
                                format!("{}:{index}", self.stream_message_id),
                                "assistant",
                                text,
                            );
                        }
                    }
                    _ => {}
                }
            }
            "control_request" => {
                self.requests
                    .retain(|request| request["request_id"] != event["request_id"]);
                self.requests.push(event.clone());
                self.status = "waiting".into();
            }
            "control_cancel_request" => {
                self.requests
                    .retain(|request| request["request_id"] != event["request_id"]);
                if self.requests.is_empty() {
                    self.status = "running".into();
                }
            }
            "assistant" | "user" => {
                let role = event["type"].as_str().unwrap_or_default();
                let message = &event["message"];
                let message_id = message["id"].as_str().or_else(|| event["uuid"].as_str());
                if let (Some(text), Some(id)) = (message["content"].as_str(), message_id) {
                    self.put(SessionItem {
                        revision: 0,
                        position: 0,
                        id: format!("{id}:0"),
                        kind: role.into(),
                        text: text.into(),
                        detail: Value::Null,
                        status: "completed".into(),
                    });
                }
                if let Some(blocks) = message["content"].as_array() {
                    // Claude emits one assistant envelope per completed block,
                    // while stream indexes span the whole message (including
                    // thinking). Native JSONL uses the same fragmented shape.
                    let offset = if role == "assistant" {
                        let id = message_id.unwrap_or_default();
                        if self.final_message_id != id {
                            self.final_message_id = id.into();
                            self.final_block_offset = 0;
                            self.final_fragments.clear();
                        }
                        let fragment = event["uuid"].as_str().unwrap_or_default();
                        if let Some(offset) = self.final_fragments.get(fragment) {
                            *offset
                        } else {
                            let offset = self.final_block_offset;
                            self.final_block_offset += blocks.len();
                            if !fragment.is_empty() && self.final_fragments.len() < 512 {
                                self.final_fragments.insert(fragment.into(), offset);
                            }
                            offset
                        }
                    } else {
                        0
                    };
                    for (index, block) in blocks.iter().enumerate() {
                        let kind = block["type"].as_str().unwrap_or_default();
                        if kind == "tool_result" {
                            if let Some(item) = self.items.iter_mut().find(|item| {
                                Some(item.id.as_str()) == block["tool_use_id"].as_str()
                            }) {
                                item.revision = self.revision;
                                item.detail["output"] = block["content"].clone();
                                item.status = if block["is_error"] == true {
                                    "failed"
                                } else {
                                    "completed"
                                }
                                .into();
                            }
                            continue;
                        }
                        let Some(id) = block["id"]
                            .as_str()
                            .map(str::to_owned)
                            .or_else(|| message_id.map(|id| format!("{id}:{}", offset + index)))
                        else {
                            continue;
                        };
                        self.put(SessionItem {
                            revision: 0,
                            position: 0,
                            id,
                            kind: match kind {
                                "text" => role,
                                "thinking" => "reasoning",
                                _ => "tool",
                            }
                            .into(),
                            text: block["text"]
                                .as_str()
                                .or_else(|| block["thinking"].as_str())
                                .or_else(|| block["name"].as_str())
                                .unwrap_or_default()
                                .into(),
                            detail: block.clone(),
                            status: if kind == "tool_use" {
                                "running"
                            } else {
                                "completed"
                            }
                            .into(),
                        });
                    }
                }
            }
            "result" => {
                self.requests.clear();
                self.turn_id = None;
                self.status = if event["is_error"] == true {
                    "failed"
                } else {
                    "ready"
                }
                .into();
                if event["is_error"] == true {
                    self.error = Some(event["errors"].to_string());
                }
            }
            _ => {}
        }
    }

    fn next_position(&self) -> usize {
        self.items
            .last()
            .map_or(self.retained_from, |item| item.position + 1)
    }

    fn interrupt_items_from(&mut self, position: usize) {
        for item in &mut self.items {
            if item.position >= position && matches!(item.status.as_str(), "running" | "inProgress")
            {
                self.revision += 1;
                item.revision = self.revision;
                item.status = "interrupted".into();
            }
        }
    }
}

fn clip(text: &mut String, limit: usize) {
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[Preview truncated. The complete output remains in the native session.]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_interrupt_finishes_only_current_turn_activity_live_and_after_resume() {
        let mut state = SessionSnapshot::default();
        state.codex_item(&json!({"id":"old","type":"commandExecution","status":"inProgress"}));
        state.codex(&json!({"method":"turn/started","params":{"turn":{"id":"turn"}}}));
        state.codex(&json!({"method":"item/started","params":{"item":{"id":"sleep","type":"commandExecution","command":"sleep 30","status":"inProgress"}}}));
        state.codex(&json!({"method":"turn/completed","params":{"turn":{"id":"turn","status":"interrupted"}}}));
        assert_eq!(state.status, "ready");
        assert_eq!(state.items[0].status, "inProgress");
        assert_eq!(state.items[1].status, "interrupted");
        state.restore_codex_history(&json!({"initialTurnsPage":{"data":[{
            "status":"interrupted", "items":[{"id":"native-sleep","type":"commandExecution","command":"sleep 30","status":"inProgress"}]
        }], "nextCursor":null}})).unwrap();
        assert_eq!(state.items[0].status, "interrupted");
    }

    #[test]
    fn codex_native_full_history_replaces_live_ids_without_duplicating_messages() {
        let mut state = SessionSnapshot::default();
        state.codex_item(&json!({"id":"live-message","type":"agentMessage","text":"Done"}));
        state.restore_codex_history(&json!({"initialTurnsPage":{"data":[{"items":[
            {"id":"item-1","type":"commandExecution","command":"pwd","aggregatedOutput":"/workspace","status":"completed"},
            {"id":"item-2","type":"agentMessage","text":"Done"}
        ]}],"nextCursor":"older"}})).unwrap();
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].detail["aggregatedOutput"], "/workspace");
        assert_eq!(state.items[1].text, "Done");
        assert_eq!(state.items[1].position, 1);
        assert!(state.history_truncated);
    }

    #[test]
    fn claude_thinking_and_text_fragments_share_stream_indexes_and_replay_once() {
        let mut state = SessionSnapshot::default();
        state.claude(
            &json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m"}}}),
        );
        state.claude(&json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"text":"Review"}}}));
        let thinking = json!({"type":"assistant","uuid":"thinking","message":{"id":"m","content":[{"type":"thinking","thinking":"Plan"}]}});
        let answer = json!({"type":"assistant","uuid":"answer","message":{"id":"m","content":[{"type":"text","text":"Review the changes"}]}});
        state.claude(&thinking);
        state.claude(&answer);
        state.claude(&answer);
        assert_eq!(
            state
                .items
                .iter()
                .filter(|item| item.kind == "assistant")
                .count(),
            1
        );
        let reply = state
            .items
            .iter()
            .find(|item| item.kind == "assistant")
            .unwrap();
        assert_eq!(reply.id, "m:1");
        assert_eq!(reply.text, "Review the changes");
        assert_eq!(reply.status, "completed");
        let mut history = SessionSnapshot::default();
        history.claude(&thinking);
        history.claude(&answer);
        assert_eq!(history.items[1].id, reply.id);
        assert_eq!(history.items[0].kind, "reasoning");
    }

    #[test]
    fn retained_transcript_bounds_repeated_outputs_and_keeps_monotonic_positions() {
        let mut state = SessionSnapshot::default();
        for n in 0..650 {
            state.codex_item(&json!({"id":format!("tool{n}"), "type":"commandExecution", "aggregatedOutput":"x".repeat(32000)}));
        }
        assert!(state.history_truncated);
        assert!(state.retained_from > 0);
        assert!(state.items.len() <= 512);
        assert!(serde_json::to_vec(&state.items).unwrap().len() < 4 * 1024 * 1024 + 1024);
        let last = state.items.last().unwrap().position;
        state.codex_item(&json!({"id":"tail", "type":"agentMessage", "text":"Done"}));
        assert_eq!(state.items.last().unwrap().position, last + 1);
        for _ in 0..100 {
            state.codex(&json!({"method":"item/commandExecution/outputDelta", "params":{"itemId":"tail", "delta":"y".repeat(32000)}}));
        }
        assert!(state.items.last().unwrap().detail.to_string().len() < 64 * 1024);
    }

    #[test]
    fn cursor_pages_bound_relay_payload_and_replay_changed_items() {
        let mut state = SessionSnapshot::default();
        for n in 0..40 {
            state.codex_item(
                &json!({"id":format!("m{n}"),"type":"agentMessage","text":"界".repeat(40000)}),
            );
        }
        let mut cursor = 0;
        let mut ids = Vec::new();
        loop {
            let page = state.clone().page(cursor);
            assert!(serde_json::to_vec(&page).unwrap().len() < 512 * 1024);
            for item in &page.items {
                assert!(item.text.contains("Preview truncated"));
                ids.push(item.id.clone());
            }
            assert!(page.cursor > cursor);
            cursor = page.cursor;
            if !page.more {
                break;
            }
        }
        assert_eq!(ids.len(), 40);
        assert!(state.clone().page(cursor).items.is_empty());
        state.codex_item(&json!({"id":"m0","type":"agentMessage","text":"Completed"}));
        let delta = state.page(cursor);
        assert_eq!(delta.items.len(), 1);
        assert_eq!(delta.items[0].id, "m0");
        assert_eq!(delta.items[0].position, 0);
        assert_eq!(delta.items[0].text, "Completed");
    }

    #[test]
    fn claude_replayed_string_input_is_visible_and_pending_cancellation_is_not_stuck() {
        let mut state = SessionSnapshot::default();
        state.claude(
            &json!({"type":"user","uuid":"u","message":{"content":"Summarize this project"}}),
        );
        assert_eq!(state.items[0].text, "Summarize this project");
        assert_eq!(state.items[0].kind, "user");
        state.claude(&json!({"type":"control_request","request_id":"q"}));
        state.claude(&json!({"type":"control_cancel_request","request_id":"q"}));
        assert!(state.requests.is_empty());
        assert_eq!(state.status, "running");
    }

    #[test]
    fn codex_stream_replaces_final_item_instead_of_duplicating_reply() {
        let mut state = SessionSnapshot::default();
        state.codex(
            &json!({"method":"item/agentMessage/delta","params":{"itemId":"a","delta":"Hello"}}),
        );
        assert_eq!(state.items[0].text, "Hello");
        state.codex(&json!({"method":"item/completed","params":{"item":{"id":"a","type":"agentMessage","text":"Hello world"}}}));
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].text, "Hello world");
    }

    #[test]
    fn claude_tool_result_updates_the_original_call_and_completion_clears_prompts() {
        let mut state = SessionSnapshot::default();
        state.claude(&json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"t","name":"Bash","input":{"command":"pwd"}}]}}));
        state.claude(&json!({"type":"control_request","request_id":"p","request":{"subtype":"can_use_tool"}}));
        assert_eq!(state.status, "waiting");
        state.claude(&json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"/workspace"}]}}));
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].detail["output"], "/workspace");
        assert_eq!(state.items[0].status, "completed");
        state.claude(&json!({"type":"result","is_error":false}));
        assert_eq!(state.status, "ready");
        assert!(state.requests.is_empty());
    }
}
