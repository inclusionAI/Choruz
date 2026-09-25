use super::HostPluginManifest;

pub(super) fn manifest() -> HostPluginManifest {
    HostPluginManifest {
        id: choruz_common::plugins::OPENCODE_PLUGIN_ID,
        version: "1",
        host_capabilities: &["opencode-agent-driver"],
        client_capabilities: &["agent-provisioning", "session-import"],
    }
}
