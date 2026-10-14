//! Which models a coding CLI can run, so the engine picker can offer them.
//!
//! Claude Code, Codex and Gemini CLI run their vendor's models, so their list
//! is that vendor's models from the catalog. OpenCode lists its own with
//! `opencode models`. CLIs that choose their model themselves (Copilot,
//! Cursor, Goose) have none to pick: they run on the CLI's own default.

use synapse_llm::Catalog;

/// A model a CLI can be told to use: `(id passed to the CLI, display name, one-line detail)`.
pub type CliModel = (String, String, String);

/// Catalog provider whose models a CLI runs, if it runs one vendor's.
fn vendor(cli_id: &str) -> Option<&'static str> {
    match cli_id {
        "claude" => Some("anthropic"),
        "codex" => Some("openai"),
        "gemini" => Some("google"),
        _ => None,
    }
}

/// Short names Claude Code accepts, newest of each family.
const CLAUDE_ALIASES: [(&str, &str); 3] = [
    ("opus", "Opus (latest)"),
    ("sonnet", "Sonnet (latest)"),
    ("haiku", "Haiku (latest)"),
];

/// Models for CLIs that run one vendor's models; empty when the CLI picks
/// its own or lists them by running a command ([`opencode_models`]).
pub fn catalog_models(cli_id: &str, catalog: &Catalog) -> Vec<CliModel> {
    let mut out: Vec<CliModel> = Vec::new();
    if cli_id == "claude" {
        out.extend(CLAUDE_ALIASES.iter().map(|(a, b)| {
            (a.to_string(), b.to_string(), "Always the newest in this family".to_string())
        }));
    }
    if let Some(models) = vendor(cli_id).and_then(|v| catalog.models(v).ok()) {
        out.extend(models.into_iter().map(|m| {
            let name = if m.name.is_empty() { m.id.clone() } else { m.name.clone() };
            (m.id.clone(), name, model_detail(m))
        }));
    }
    out
}

/// `id · reasoning · vision · tools`: what a model row says under its name.
pub fn model_detail(m: &synapse_llm::catalog::CatalogModel) -> String {
    let mut parts = vec![m.id.clone()];
    for (on, tag) in [(m.reasoning, "reasoning"), (m.attachment, "files"), (m.tool_call, "tools")] {
        if on {
            parts.push(tag.to_string());
        }
    }
    parts.join(" · ")
}

/// True for CLIs whose model list comes from running the CLI.
pub fn lists_by_command(cli_id: &str) -> bool {
    cli_id == "opencode"
}

/// Where the `opencode` binary is. GUI apps do not inherit the shell `PATH`
/// (OpenCode installs to `~/.opencode/bin`), so ask detection, which resolves it.
fn opencode_path() -> std::path::PathBuf {
    let found = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
        .and_then(|rt| {
            rt.block_on(synapse_acp::detect_installed())
                .into_iter()
                .find(|a| a.id == "opencode")
                .map(|a| a.cli_path)
        });
    found.unwrap_or_else(|| "opencode".into())
}

/// Ask OpenCode for its models (`provider/model` per line). Waits up to
/// twenty seconds; call it off the UI thread.
pub fn opencode_models() -> Vec<CliModel> {
    use std::process::{Command, Stdio};
    let Ok(child) = Command::new(opencode_path())
        .arg("models")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    // Read on another thread so a long list cannot fill the pipe and stall the CLI.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(20)) {
        Ok(Ok(out)) => parse_model_lines(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

fn parse_model_lines(text: &str) -> Vec<CliModel> {
    let mut out: Vec<CliModel> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(' '))
        .map(|l| (l.to_string(), l.to_string(), "OpenCode model".to_string()))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_lines_keep_only_ids() {
        let got = parse_model_lines("openai/gpt-4o\nanthropic/claude-sonnet\n\nnote: ignore me\n");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "anthropic/claude-sonnet");
    }

    #[test]
    fn claude_offers_aliases_first() {
        let catalog = synapse_llm::Catalog::embedded().expect("catalog");
        let models = catalog_models("claude", &catalog);
        assert_eq!(models[0].0, "opus");
        assert!(models.len() > 3);
        assert!(catalog_models("goose", &catalog).is_empty());
    }

    /// Real OpenCode; run with `cargo test -p synapse-app live_opencode -- --ignored`.
    #[test]
    #[ignore = "needs opencode installed"]
    fn live_opencode_lists_models() {
        let models = opencode_models();
        println!("{} models", models.len());
        assert!(!models.is_empty());
    }
}
