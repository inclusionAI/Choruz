use axum::Router;
use serde::Serialize;

use crate::ApiState;

mod agent_skills;
mod kanban;
mod mathcode;
mod pixel_world;
mod remote_control;
mod remote_ssh;
mod workspace_git;

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct HostPluginManifest {
    pub id: &'static str,
    pub version: &'static str,
    pub host_capabilities: &'static [&'static str],
    pub client_capabilities: &'static [&'static str],
}

struct HostPluginRegistration {
    manifest: fn() -> HostPluginManifest,
    router: Option<fn() -> Router<ApiState>>,
}

fn registrations() -> [HostPluginRegistration; 7] {
    [
        HostPluginRegistration {
            manifest: kanban::manifest,
            router: Some(kanban::router),
        },
        HostPluginRegistration {
            manifest: pixel_world::manifest,
            router: None,
        },
        HostPluginRegistration {
            manifest: workspace_git::manifest,
            router: None,
        },
        HostPluginRegistration {
            manifest: remote_ssh::manifest,
            router: Some(remote_ssh::router),
        },
        HostPluginRegistration {
            manifest: remote_control::manifest,
            router: Some(remote_control::router),
        },
        HostPluginRegistration {
            manifest: agent_skills::manifest,
            router: None,
        },
        HostPluginRegistration {
            manifest: mathcode::manifest,
            router: None,
        },
    ]
}

pub(crate) fn router() -> Router<ApiState> {
    let mut router = Router::new();
    for registration in registrations() {
        let manifest = (registration.manifest)();
        if choruz_common::plugins::plugin_enabled(manifest.id) {
            if let Some(plugin_router) = registration.router {
                router = router.merge(plugin_router());
            }
        }
    }
    router
}

pub(crate) fn enabled_manifests() -> Vec<HostPluginManifest> {
    let enabled = choruz_common::plugins::enabled_plugin_ids();
    registrations()
        .into_iter()
        .map(|registration| (registration.manifest)())
        .filter(|manifest| enabled.contains(&manifest.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        agent_skills, kanban, mathcode, pixel_world, registrations, remote_control, remote_ssh,
        workspace_git,
    };

    #[test]
    fn builtin_manifests_have_matching_host_and_client_contracts() {
        let registered_ids: Vec<_> = registrations()
            .into_iter()
            .map(|registration| (registration.manifest)().id)
            .collect();
        assert_eq!(registered_ids, choruz_common::plugins::BUILTIN_PLUGIN_IDS);

        let kanban = kanban::manifest();
        assert_eq!(kanban.id, "kanban");
        assert!(kanban.host_capabilities.contains(&"channel-task-api"));
        assert!(kanban.client_capabilities.contains(&"conversation-tab"));

        let pixel_world = pixel_world::manifest();
        assert_eq!(pixel_world.id, "pixel-world");
        assert!(pixel_world.host_capabilities.contains(&"workspace-roster"));
        assert!(
            pixel_world
                .client_capabilities
                .contains(&"workspace-overlay")
        );

        assert_eq!(workspace_git::manifest().id, "workspace-git");
        assert!(
            workspace_git::manifest()
                .client_capabilities
                .contains(&"detail-tab")
        );
        assert_eq!(remote_ssh::manifest().id, "remote-ssh");
        assert!(
            remote_ssh::manifest()
                .host_capabilities
                .contains(&"ssh-tunnel-api")
        );
        assert_eq!(remote_control::manifest().id, "remote-control");
        assert!(
            remote_control::manifest()
                .host_capabilities
                .contains(&"opaque-pairing-credential")
        );
        assert_eq!(agent_skills::manifest().id, "agent-skills");
        assert!(
            agent_skills::manifest()
                .client_capabilities
                .contains(&"agent-detail-tab")
        );
        assert_eq!(mathcode::manifest().id, "mathcode");
        assert!(
            mathcode::manifest()
                .host_capabilities
                .contains(&"mathcode-agent-driver")
        );
    }
}
