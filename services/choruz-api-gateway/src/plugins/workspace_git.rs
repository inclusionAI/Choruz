use super::HostPluginManifest;

pub(super) fn manifest() -> HostPluginManifest {
    HostPluginManifest {
        id: choruz_common::plugins::WORKSPACE_GIT_PLUGIN_ID,
        version: "1",
        host_capabilities: &["workspace-repository-access"],
        client_capabilities: &["detail-tab", "git-graph"],
    }
}
