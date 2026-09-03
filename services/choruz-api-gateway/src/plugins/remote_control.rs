use axum::{
    Router,
    routing::{delete, get, post, put},
};

use crate::{
    ApiState, handlers_harness_logins, handlers_remote_control, handlers_runtime_hosts,
    handlers_workspace_sessions,
};

use super::HostPluginManifest;

pub(super) fn manifest() -> HostPluginManifest {
    HostPluginManifest {
        id: choruz_common::plugins::REMOTE_CONTROL_PLUGIN_ID,
        version: "1",
        host_capabilities: &[
            "opaque-pairing-credential",
            "remote-device-api",
            "cloud-gateway",
            "end-to-end-encryption",
            "workspace-session-import",
            "multi-runtime-hosts",
            "host-aware-agent-routing",
        ],
        client_capabilities: &["sidebar-action", "modal", "web-dashboard"],
    }
}

pub(super) fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/v1/remote-control/settings",
            get(handlers_remote_control::get_settings),
        )
        .route(
            "/v1/remote-control/pairings",
            post(handlers_remote_control::create_pairing),
        )
        .route(
            "/v1/remote-control/pairings/redeem",
            post(handlers_remote_control::redeem_pairing),
        )
        .route(
            "/v1/remote-control/devices",
            get(handlers_remote_control::list_devices),
        )
        .route(
            "/v1/remote-control/bridge-config",
            get(handlers_remote_control::get_bridge_config),
        )
        .route(
            "/v1/remote-control/devices/{device_id}",
            delete(handlers_remote_control::revoke_device),
        )
        .route(
            "/v1/remote-control/devices/{device_id}/seen",
            put(handlers_remote_control::mark_device_seen),
        )
        .route(
            "/v1/companies/{company_id}/runtime-host-pairings",
            post(handlers_runtime_hosts::create_pairing),
        )
        .route(
            "/v1/runtime-host-pairings/redeem",
            post(handlers_runtime_hosts::redeem_pairing),
        )
        .route(
            "/v1/companies/{company_id}/runtime-hosts",
            get(handlers_runtime_hosts::list_hosts),
        )
        .route(
            "/v1/runtime-hosts/{host_id}",
            put(handlers_runtime_hosts::rename_host).delete(handlers_runtime_hosts::revoke_host),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/heartbeat",
            post(handlers_runtime_hosts::heartbeat),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/harness-accounts",
            post(handlers_runtime_hosts::register_harness_account),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/harness-accounts/{account_id}/verify",
            post(handlers_runtime_hosts::verify_harness_account),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/harness-account-logins/claim",
            post(handlers_harness_logins::claim_harness_account_login),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/harness-account-logins/{login_id}/publish",
            post(handlers_harness_logins::publish_harness_account_login),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/harness-account-logins/{login_id}/callback/claim",
            post(handlers_harness_logins::claim_harness_account_login_callback),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/harness-account-logins/{login_id}/complete",
            post(handlers_harness_logins::complete_harness_account_login),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/harness-account-logins/{login_id}/fail",
            post(handlers_harness_logins::fail_harness_account_login),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/commands/claim",
            post(handlers_runtime_hosts::claim_command),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/commands/{command_id}/complete",
            post(handlers_runtime_hosts::complete_command),
        )
        .route(
            "/v1/runtime-hosts/{host_id}/commands/{command_id}/heartbeat",
            post(handlers_runtime_hosts::heartbeat_command),
        )
        .route(
            "/v1/runtime/bindings/{binding_id}/host",
            put(handlers_runtime_hosts::assign_binding_host),
        )
        .route(
            "/v1/workspace-sessions/scan",
            post(handlers_workspace_sessions::scan_workspace_sessions),
        )
        .route(
            "/v1/workspace-sessions/import",
            post(handlers_workspace_sessions::import_workspace_sessions),
        )
}
