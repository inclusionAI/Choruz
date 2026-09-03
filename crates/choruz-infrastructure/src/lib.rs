use std::io;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogFormat {
    Human,
    Json,
}

fn parse_log_format(value: Option<&str>) -> Result<LogFormat, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("human") => Ok(LogFormat::Human),
        Some("json") => Ok(LogFormat::Json),
        Some(value) => Err(format!(
            "invalid CHORUZ_LOG_FORMAT `{value}`; expected `human` or `json`"
        )),
    }
}

fn parse_log_filter(value: Option<&str>) -> Result<EnvFilter, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => EnvFilter::try_new(value)
            .map_err(|error| format!("invalid RUST_LOG `{value}`: {error}")),
        None => Ok(EnvFilter::new("info")),
    }
}

/// Initialize stderr tracing from `RUST_LOG` and `CHORUZ_LOG_FORMAT`.
///
/// The default is human-readable `info` output. Invalid values fail startup
/// instead of silently selecting another logging configuration.
pub fn init_tracing(service_name: &str) -> Result<(), String> {
    let format = parse_log_format(std::env::var("CHORUZ_LOG_FORMAT").ok().as_deref())?;
    let filter = parse_log_filter(std::env::var("RUST_LOG").ok().as_deref())?;

    let result = match format {
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_target(true)
            .with_thread_names(true)
            .with_file(false)
            .with_line_number(false)
            .with_writer(io::stderr)
            .try_init(),
        LogFormat::Human => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_thread_names(true)
            .with_file(false)
            .with_line_number(false)
            .with_writer(io::stderr)
            .try_init(),
    };
    result.map_err(|error| format!("initialize tracing: {error}"))?;
    tracing::info!(service = service_name, "tracing initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{LogFormat, parse_log_filter, parse_log_format};

    #[test]
    fn log_format_defaults_to_human() {
        assert_eq!(parse_log_format(None).unwrap(), LogFormat::Human);
        assert_eq!(parse_log_format(Some("")).unwrap(), LogFormat::Human);
        assert_eq!(parse_log_format(Some("human")).unwrap(), LogFormat::Human);
    }

    #[test]
    fn log_format_accepts_json_and_rejects_unknown_values() {
        assert_eq!(parse_log_format(Some("json")).unwrap(), LogFormat::Json);
        assert!(parse_log_format(Some("pretty")).is_err());
    }

    #[test]
    fn log_filter_defaults_to_info_and_rejects_invalid_values() {
        assert_eq!(parse_log_filter(None).unwrap().to_string(), "info");
        assert_eq!(
            parse_log_filter(Some("choruz_pipeline=debug,warn"))
                .unwrap()
                .to_string(),
            "choruz_pipeline=debug,warn"
        );
        assert!(parse_log_filter(Some("not a filter [")).is_err());
    }
}
