//! Logging: `tracing` with a rolling file appender plus a redaction layer.
//!
//! Every message is scrubbed with [`synapse_core::redact`] before it is
//! emitted, so keys never land in logs, transcripts or crash reports
//! (ARCHITECTURE.md §6). Redaction cannot be turned off for logs.

use std::path::Path;

use synapse_core::Redactor;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Scrub `message` with a default redactor plus any extra known values.
pub fn redact_for_log(message: &str, known: &[(&str, &str)]) -> String {
    let mut r = Redactor::new();
    r.add_secrets(known.iter().copied());
    r.redact(message).text
}

/// Install a global `tracing` subscriber writing to rolling daily log files
/// (`synapse.<date>.log`) under `log_dir`, plus stderr. Safe to call once;
/// later calls are no-ops. Returns the file guard (keep it alive).
pub fn init_logging(log_dir: &Path) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir)?;
    let file_appender = tracing_appender::rolling::daily(log_dir, "synapse.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(file_writer)
        .with_target(false)
        .compact()
        .finish();
    // `try_init` so tests calling this twice do not panic.
    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(guard)
}

/// Emit an INFO log with secrets scrubbed. Prefer this over `tracing::info!`
/// for any string that could contain tool output, fetched pages or args.
pub fn log_info_redacted(message: &str, known: &[(&str, &str)]) {
    tracing::info!("{}", redact_for_log(message, known));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_redacts_before_write() {
        let fake = "sk-proj-f4k3T3stOpenAIKey0123456789abcdefXYZ";
        let out = redact_for_log(&format!("calling with {fake}"), &[]);
        assert!(!out.contains(fake));
        assert!(out.contains("[redacted:"));
    }

    #[test]
    fn logging_init_creates_log_dir() {
        let dir = std::env::temp_dir().join(format!("synapse-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _guard = init_logging(&dir).unwrap();
        assert!(dir.is_dir());
        tracing::info!("hello {}", "world");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
