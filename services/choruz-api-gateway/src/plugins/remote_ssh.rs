use axum::{
    Router,
    routing::{delete, get, post},
};

use crate::{ApiState, handlers_ssh};

use super::HostPluginManifest;

pub(super) fn manifest() -> HostPluginManifest {
    HostPluginManifest {
        id: choruz_common::plugins::REMOTE_SSH_PLUGIN_ID,
        version: "1",
        host_capabilities: &[
            "ssh-host-discovery",
            "ssh-tunnel-api",
            "remote-choruz-connect",
        ],
        client_capabilities: &["sidebar-action", "modal"],
    }
}

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route("/v1/ssh/hosts", get(handlers_ssh::list_ssh_hosts))
        .route("/v1/ssh/tunnel", post(handlers_ssh::create_tunnel))
        .route("/v1/ssh/tunnels", get(handlers_ssh::list_tunnels))
        .route("/v1/ssh/tunnel/{id}", delete(handlers_ssh::delete_tunnel))
        .route(
            "/v1/ssh/connect-choruz",
            post(handlers_ssh::connect_choruz_tunnel),
        )
}
