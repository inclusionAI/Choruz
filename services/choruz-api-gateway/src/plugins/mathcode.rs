use super::HostPluginManifest;

pub(super) fn manifest() -> HostPluginManifest {
    HostPluginManifest {
        id: choruz_common::plugins::MATHCODE_PLUGIN_ID,
        version: "1",
        host_capabilities: &["mathcode-agent-driver"],
        client_capabilities: &["agent-provisioning"],
    }
}
