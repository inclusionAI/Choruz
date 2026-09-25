use super::HostPluginManifest;

pub(super) fn manifest() -> HostPluginManifest {
    HostPluginManifest {
        id: choruz_common::plugins::PI_PLUGIN_ID,
        version: "1",
        host_capabilities: &["pi-agent-driver"],
        client_capabilities: &["agent-provisioning", "session-import"],
    }
}
