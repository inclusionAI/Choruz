use super::HostPluginManifest;

pub(super) fn manifest() -> HostPluginManifest {
    HostPluginManifest {
        id: choruz_common::plugins::AGENT_SKILLS_PLUGIN_ID,
        version: "1",
        host_capabilities: &["agent-workspace-access"],
        client_capabilities: &["agent-detail-tab", "agent-provisioning"],
    }
}
