use super::HostPluginManifest;

pub(super) fn manifest() -> HostPluginManifest {
    HostPluginManifest {
        id: choruz_common::plugins::PIXEL_WORLD_PLUGIN_ID,
        version: "1",
        host_capabilities: &["workspace-roster", "conversation-activity"],
        client_capabilities: &["sidebar-action", "workspace-overlay"],
    }
}
