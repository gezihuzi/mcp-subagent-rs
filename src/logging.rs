use std::{env, path::Path};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub struct LoggingGuard {
    _file_guard: WorkerGuard,
}

pub fn init_logging(
    state_dir: &Path,
    cli_log_level: Option<&str>,
    configured_log_level: &str,
) -> std::result::Result<LoggingGuard, String> {
    std::fs::create_dir_all(state_dir).map_err(|err| {
        format!(
            "failed to create state dir for logging {}: {err}",
            state_dir.display()
        )
    })?;

    let filter_directive = resolve_log_filter(cli_log_level, configured_log_level);
    let env_filter = EnvFilter::try_new(filter_directive.clone())
        .map_err(|err| format!("invalid log filter `{filter_directive}`: {err}"))?;

    let file_appender = tracing_appender::rolling::never(state_dir, "server.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true)
        .with_target(false);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(file_layer)
        .try_init()
        .map_err(|err| format!("failed to initialize tracing subscriber: {err}"))?;

    tracing::debug!(
        filter = filter_directive,
        server_log = %state_dir.join("server.log").display(),
        "tracing initialized"
    );

    Ok(LoggingGuard {
        _file_guard: file_guard,
    })
}

fn resolve_log_filter(cli_log_level: Option<&str>, configured_log_level: &str) -> String {
    let env_rust_log = env::var("RUST_LOG").ok();
    resolve_log_filter_with_env(cli_log_level, configured_log_level, env_rust_log.as_deref())
}

fn resolve_log_filter_with_env(
    cli_log_level: Option<&str>,
    configured_log_level: &str,
    env_rust_log: Option<&str>,
) -> String {
    if let Some(cli) = sanitize_level(cli_log_level) {
        return cli.to_string();
    }
    if let Some(env_log) = sanitize_level(env_rust_log) {
        return env_log.to_string();
    }
    sanitize_level(Some(configured_log_level))
        .unwrap_or("info")
        .to_string()
}

fn sanitize_level(level: Option<&str>) -> Option<&str> {
    let trimmed = level?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

#[cfg(test)]
mod tests {
    use super::resolve_log_filter_with_env;

    #[test]
    fn cli_level_overrides_env_and_config() {
        let chosen = resolve_log_filter_with_env(Some("trace"), "info", Some("debug"));
        assert_eq!(chosen, "trace");
    }

    #[test]
    fn env_overrides_config_when_cli_missing() {
        let chosen = resolve_log_filter_with_env(None, "info", Some("warn"));
        assert_eq!(chosen, "warn");
    }

    #[test]
    fn config_used_when_cli_and_env_missing() {
        let chosen = resolve_log_filter_with_env(None, "debug", None);
        assert_eq!(chosen, "debug");
    }
}
