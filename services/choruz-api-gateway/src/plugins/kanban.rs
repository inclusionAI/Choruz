use axum::{
    Router,
    routing::{get, post},
};

use crate::{ApiState, handlers_channel_tasks};

use super::HostPluginManifest;

pub(super) fn manifest() -> HostPluginManifest {
    HostPluginManifest {
        id: choruz_common::plugins::KANBAN_PLUGIN_ID,
        version: "1",
        host_capabilities: &["channel-task-api", "channel-task-events"],
        client_capabilities: &["conversation-tab", "message-action"],
    }
}

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/v1/conversations/{conversation_id}/tasks",
            get(handlers_channel_tasks::list_channel_tasks)
                .post(handlers_channel_tasks::create_channel_task),
        )
        .route(
            "/v1/conversations/{conversation_id}/tasks/from-message",
            post(handlers_channel_tasks::create_channel_task_from_message),
        )
        .route(
            "/v1/tasks/{task_id}",
            get(handlers_channel_tasks::get_channel_task)
                .patch(handlers_channel_tasks::patch_channel_task),
        )
}
