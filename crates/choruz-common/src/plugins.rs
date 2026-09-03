pub const KANBAN_PLUGIN_ID: &str = "kanban";
pub const PIXEL_WORLD_PLUGIN_ID: &str = "pixel-world";
pub const WORKSPACE_GIT_PLUGIN_ID: &str = "workspace-git";
pub const REMOTE_SSH_PLUGIN_ID: &str = "remote-ssh";
pub const REMOTE_CONTROL_PLUGIN_ID: &str = "remote-control";
pub const AGENT_SKILLS_PLUGIN_ID: &str = "agent-skills";
pub const MATHCODE_PLUGIN_ID: &str = "mathcode";
pub const KANBAN_PLUGIN_DISABLED_DETAIL: &str =
    "plugin 'kanban' is disabled; include it in CHORUZ_PLUGINS";

pub const BUILTIN_PLUGIN_IDS: [&str; 7] = [
    KANBAN_PLUGIN_ID,
    PIXEL_WORLD_PLUGIN_ID,
    WORKSPACE_GIT_PLUGIN_ID,
    REMOTE_SSH_PLUGIN_ID,
    REMOTE_CONTROL_PLUGIN_ID,
    AGENT_SKILLS_PLUGIN_ID,
    MATHCODE_PLUGIN_ID,
];

/// Returns the built-in plugins enabled for this host.
///
/// All built-ins are enabled by default. Set `CHORUZ_PLUGINS` to a
/// comma-separated allowlist (or an empty string to disable every plugin).
pub fn enabled_plugin_ids() -> Vec<&'static str> {
    enabled_plugin_ids_from_env(std::env::var("CHORUZ_PLUGINS"))
}

fn enabled_plugin_ids_from_env(value: Result<String, std::env::VarError>) -> Vec<&'static str> {
    match value {
        Ok(value) => enabled_plugin_ids_from(Some(&value)),
        Err(std::env::VarError::NotPresent) => BUILTIN_PLUGIN_IDS.to_vec(),
        Err(std::env::VarError::NotUnicode(_)) => Vec::new(),
    }
}

pub fn plugin_enabled(plugin_id: &str) -> bool {
    enabled_plugin_ids().contains(&plugin_id)
}

fn enabled_plugin_ids_from(value: Option<&str>) -> Vec<&'static str> {
    let Some(value) = value else {
        return BUILTIN_PLUGIN_IDS.to_vec();
    };

    BUILTIN_PLUGIN_IDS
        .into_iter()
        .filter(|plugin_id| {
            value
                .split(',')
                .map(str::trim)
                .any(|configured| configured == *plugin_id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_SKILLS_PLUGIN_ID, KANBAN_PLUGIN_ID, MATHCODE_PLUGIN_ID, PIXEL_WORLD_PLUGIN_ID,
        REMOTE_CONTROL_PLUGIN_ID, REMOTE_SSH_PLUGIN_ID, WORKSPACE_GIT_PLUGIN_ID,
        enabled_plugin_ids_from, enabled_plugin_ids_from_env,
    };

    #[test]
    fn enables_all_builtins_when_configuration_is_absent() {
        assert_eq!(
            enabled_plugin_ids_from(None),
            vec![
                KANBAN_PLUGIN_ID,
                PIXEL_WORLD_PLUGIN_ID,
                WORKSPACE_GIT_PLUGIN_ID,
                REMOTE_SSH_PLUGIN_ID,
                REMOTE_CONTROL_PLUGIN_ID,
                AGENT_SKILLS_PLUGIN_ID,
                MATHCODE_PLUGIN_ID,
            ]
        );
    }

    #[test]
    fn configuration_is_an_allowlist() {
        assert_eq!(
            enabled_plugin_ids_from(Some(" pixel-world ")),
            vec![PIXEL_WORLD_PLUGIN_ID]
        );
        assert_eq!(
            enabled_plugin_ids_from(Some("kanban,unknown,kanban")),
            vec![KANBAN_PLUGIN_ID]
        );
        assert!(enabled_plugin_ids_from(Some("")).is_empty());
    }

    #[test]
    fn invalid_environment_encoding_disables_all_plugins() {
        assert!(
            enabled_plugin_ids_from_env(Err(std::env::VarError::NotUnicode("invalid".into())))
                .is_empty()
        );
    }
}
