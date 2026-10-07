//! Device-local executable resolution shared by interactive and background callers.
use crate::DriverType;

pub fn default_terminal_binary(driver_type: &DriverType) -> &'static str {
    match driver_type {
        DriverType::ClaudeTerminal => "claude",
        DriverType::CodexTerminal => "codex",
        DriverType::MuseTerminal => "muse",
        DriverType::PiTerminal => "pi",
        DriverType::GrokTerminal => "grok",
        DriverType::OpenCodeTerminal => "opencode",
        DriverType::MathCodeTerminal => "mathcode",
        _ => "claude",
    }
}

/// The executable for a terminal. `binary_path: "codex"` is the portable
/// default persisted by older bindings; an explicitly configured Harness
/// executable (`CHORUZ_<HARNESS>_BINARY`, then supported `*_CLI_PATH`, on this device) beats that bare
/// default because PATH can otherwise select a stale CLI. Absolute or custom
/// per-agent paths remain authoritative.
pub fn terminal_binary(driver_type: &DriverType, configured: Option<&str>) -> String {
    terminal_binary_with_env(driver_type, configured, |key| std::env::var(key).ok())
}

fn terminal_binary_with_env(
    driver_type: &DriverType,
    configured: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
) -> String {
    let configured = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_terminal_binary(driver_type));
    let environment_keys: &[&str] = match driver_type {
        DriverType::ClaudeTerminal => &["CHORUZ_CLAUDE_BINARY", "CHORUZ_CLAUDE_CLI_PATH"],
        DriverType::CodexTerminal => &["CHORUZ_CODEX_BINARY", "CHORUZ_CODEX_CLI_PATH"],
        DriverType::MuseTerminal => &["CHORUZ_MUSE_BINARY", "CHORUZ_MUSE_CLI_PATH"],
        DriverType::PiTerminal => &["CHORUZ_PI_BINARY", "CHORUZ_PI_CLI_PATH"],
        DriverType::GrokTerminal => &["CHORUZ_GROK_BINARY", "CHORUZ_GROK_CLI_PATH"],
        DriverType::OpenCodeTerminal => &["CHORUZ_OPENCODE_BINARY", "CHORUZ_OPENCODE_CLI_PATH"],
        DriverType::MathCodeTerminal => &["CHORUZ_MATHCODE_BINARY"],
        _ => &[],
    };
    if configured == default_terminal_binary(driver_type)
        && let Some(path) = environment_keys.iter().find_map(|key| {
            env(key)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
    {
        return path;
    }
    configured.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_driver_defaults_do_not_fall_back_to_claude() {
        assert_eq!(default_terminal_binary(&DriverType::CodexTerminal), "codex");
        assert_eq!(default_terminal_binary(&DriverType::PiTerminal), "pi");
        assert_eq!(default_terminal_binary(&DriverType::GrokTerminal), "grok");
        assert_eq!(
            default_terminal_binary(&DriverType::OpenCodeTerminal),
            "opencode"
        );
        assert_eq!(
            default_terminal_binary(&DriverType::MathCodeTerminal),
            "mathcode"
        );
        assert_eq!(
            terminal_binary(&DriverType::CodexTerminal, Some("/opt/codex/bin/codex")),
            "/opt/codex/bin/codex"
        );
    }

    #[test]
    fn target_executable_configuration_preserves_alias_precedence() {
        for driver in [
            DriverType::ClaudeTerminal,
            DriverType::CodexTerminal,
            DriverType::MuseTerminal,
            DriverType::PiTerminal,
            DriverType::GrokTerminal,
            DriverType::OpenCodeTerminal,
            DriverType::MathCodeTerminal,
        ] {
            assert_eq!(
                terminal_binary_with_env(&driver, None, |key| Some(
                    if key.ends_with("_BINARY") {
                        "  /target/primary  "
                    } else {
                        "/target/alias"
                    }
                    .into()
                )),
                "/target/primary"
            );
            let expected = if driver == DriverType::MathCodeTerminal {
                "mathcode"
            } else {
                "/target/alias"
            };
            assert_eq!(
                terminal_binary_with_env(&driver, None, |key| Some(
                    if key.ends_with("_BINARY") {
                        "  "
                    } else {
                        " /target/alias "
                    }
                    .into()
                )),
                expected
            );
            assert_eq!(
                terminal_binary_with_env(&driver, None, |_| Some("  ".into())),
                default_terminal_binary(&driver)
            );
            assert_eq!(
                terminal_binary_with_env(&driver, Some("/target/explicit"), |_| Some(
                    "/target/automatic".into()
                )),
                "/target/explicit"
            );
        }
    }
}
