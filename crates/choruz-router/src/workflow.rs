//! Workflow metadata parsing for hybrid group routing.

use crate::models::WorkflowRoutingEvent;

pub fn parse_workflow_routing_event(metadata: &serde_json::Value) -> Option<WorkflowRoutingEvent> {
    let workflow = metadata.get("workflow")?.as_object()?;
    let kind = optional_string(workflow.get("kind")?)?;

    Some(WorkflowRoutingEvent {
        kind,
        task_key: workflow.get("task_key").and_then(optional_string),
        task_id: workflow.get("task_id").and_then(optional_string),
        next_role: workflow.get("next_role").and_then(optional_string),
        target_role: workflow.get("target_role").and_then(optional_string),
        target_principal_ids: string_list(workflow.get("target_principal_ids"))
            .or_else(|| string_list(workflow.get("target_agent_ids")))
            .unwrap_or_default(),
    })
}

fn optional_string(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_list(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    match value? {
        serde_json::Value::String(raw) => {
            let raw = raw.trim();
            (!raw.is_empty()).then(|| vec![raw.to_string()])
        }
        serde_json::Value::Array(items) => {
            Some(items.iter().filter_map(optional_string).collect::<Vec<_>>())
        }
        _ => None,
    }
    .filter(|items| !items.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_workflow_metadata_fields() {
        let event = parse_workflow_routing_event(&serde_json::json!({
            "workflow": {
                "kind": "task.ready_for_next_step",
                "task_key": "DOC-P0-03",
                "task_id": "task-1",
                "next_role": "quality_check",
                "target_role": "owner",
                "target_principal_ids": ["agent-owner", "", 42],
            }
        }))
        .expect("workflow metadata should parse");

        assert_eq!(event.kind, "task.ready_for_next_step");
        assert_eq!(event.task_key.as_deref(), Some("DOC-P0-03"));
        assert_eq!(event.task_id.as_deref(), Some("task-1"));
        assert_eq!(event.next_role.as_deref(), Some("quality_check"));
        assert_eq!(event.target_role.as_deref(), Some("owner"));
        assert_eq!(event.target_principal_ids, vec!["agent-owner"]);
    }

    #[test]
    fn ignores_missing_or_blank_workflow_kind() {
        assert!(parse_workflow_routing_event(&serde_json::json!({})).is_none());
        assert!(
            parse_workflow_routing_event(&serde_json::json!({
                "workflow": { "kind": " " }
            }))
            .is_none()
        );
    }
}
