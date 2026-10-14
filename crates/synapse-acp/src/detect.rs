//! Detect supported installed CLI agents.
//!
//! Supported set (see docs/STACK.md): Claude Code (`claude`), Codex
//! (`codex`), OpenCode (`opencode`), Gemini CLI (`gemini`), Copilot CLI
//! (`copilot`), Cursor (`cursor-agent`) and Goose (`goose`).
//!
//! For each installed CLI we report the binary path, `--version` output when
//! obtainable, the ACP adapter command and whether that adapter binary is
//! present, plus login state where knowable (credential-marker files under
//! the home directory).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::path::effective_path_dirs;

/// How the agent is driven over ACP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableAdapter {
    /// Command to spawn, e.g. `["npx", "@agentclientprotocol/claude-agent-acp"]`
    /// or `["opencode", "acp"]`.
    pub command: String,
    pub args: Vec<String>,
    /// Whether the adapter entrypoint looks runnable (its binary is on `PATH`).
    pub available: bool,
}

/// Login state where knowable. `Unknown` for agents with no stable marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoginState {
    LoggedIn,
    LoggedOut,
    Unknown,
}

impl LoginState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LoggedIn => "logged-in",
            Self::LoggedOut => "logged-out",
            Self::Unknown => "unknown",
        }
    }
}

/// A supported agent found on `PATH`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedAgent {
    /// Stable id: `claude`, `codex`, `opencode`, `gemini`, `copilot`,
    /// `cursor-agent`, `goose`.
    pub id: String,
    pub label: String,
    pub cli_path: PathBuf,
    #[serde(default)]
    pub version: Option<String>,
    pub adapter: AvailableAdapter,
    pub login: LoginState,
}

struct SupportedAgent {
    id: &'static str,
    label: &'static str,
    bins: &'static [&'static str],
    adapter_command: &'static str,
    adapter_args: &'static [&'static str],
    /// Credential marker files relative to home; empty = login state unknown.
    login_markers: &'static [&'static str],
}

const SUPPORTED: &[SupportedAgent] = &[
    SupportedAgent {
        id: "claude",
        label: "Claude Code",
        bins: &["claude"],
        adapter_command: "npx",
        adapter_args: &["@agentclientprotocol/claude-agent-acp"],
        login_markers: &[".claude/.credentials.json"],
    },
    SupportedAgent {
        id: "codex",
        label: "Codex",
        bins: &["codex"],
        adapter_command: "npx",
        adapter_args: &["@agentclientprotocol/codex-acp"],
        login_markers: &[".codex/auth.json"],
    },
    SupportedAgent {
        id: "opencode",
        label: "OpenCode",
        bins: &["opencode"],
        adapter_command: "opencode",
        adapter_args: &["acp"],
        login_markers: &[
            ".local/share/opencode/auth.json",
            ".config/opencode/auth.json",
        ],
    },
    SupportedAgent {
        id: "gemini",
        label: "Gemini CLI",
        bins: &["gemini"],
        adapter_command: "gemini",
        adapter_args: &["--acp"],
        login_markers: &[".gemini/oauth_creds.json"],
    },
    SupportedAgent {
        id: "copilot",
        label: "Copilot CLI",
        bins: &["copilot"],
        adapter_command: "copilot",
        adapter_args: &["--acp"],
        login_markers: &[".config/github-copilot/apps.json"],
    },
    SupportedAgent {
        id: "cursor-agent",
        label: "Cursor",
        bins: &["cursor-agent"],
        adapter_command: "cursor-agent",
        adapter_args: &["acp"],
        login_markers: &[],
    },
    SupportedAgent {
        id: "goose",
        label: "Goose",
        bins: &["goose"],
        adapter_command: "goose",
        adapter_args: &["acp"],
        login_markers: &[],
    },
];

fn find_on_path(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let exe_names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            name.to_string(),
        ]
    } else {
        vec![name.to_string()]
    };
    for dir in dirs {
        for exe in &exe_names {
            let p = dir.join(exe);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn login_state(spec: &SupportedAgent, home: &Path) -> LoginState {
    if spec.login_markers.is_empty() {
        return LoginState::Unknown;
    }
    let logged_in = spec.login_markers.iter().any(|m| home.join(m).is_file());
    if logged_in {
        LoginState::LoggedIn
    } else {
        LoginState::LoggedOut
    }
}

async fn cli_version(path: &Path) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(path).arg("--version").output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(120).collect())
}

/// Detect supported agents on `dirs`, using `home` for login markers.
pub async fn detect_in_dirs(dirs: &[PathBuf], home: &Path) -> Vec<DetectedAgent> {
    let mut out = Vec::new();
    for spec in SUPPORTED {
        let cli_path = spec.bins.iter().find_map(|b| find_on_path(b, dirs));
        let Some(cli_path) = cli_path else { continue };
        let adapter_bin = if spec.adapter_command == "npx" {
            find_on_path("npx", dirs).or_else(|| find_on_path("node", dirs))
        } else {
            find_on_path(spec.adapter_command, dirs)
        };
        // Native adapters (`opencode acp`, `gemini --acp`, ...) reuse the CLI
        // binary itself, so they are available whenever the CLI is installed.
        let available = adapter_bin.is_some() || spec.adapter_command != "npx";
        out.push(DetectedAgent {
            id: spec.id.to_string(),
            label: spec.label.to_string(),
            cli_path,
            version: None,
            adapter: AvailableAdapter {
                command: spec.adapter_command.to_string(),
                args: spec.adapter_args.iter().map(|s| s.to_string()).collect(),
                available,
            },
            login: login_state(spec, home),
        });
    }
    // Fill in versions (best effort, bounded).
    for agent in &mut out {
        agent.version = cli_version(&agent.cli_path).await;
    }
    out
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"))
}

/// Full detection: resolve the login-shell `PATH` and list supported
/// installed agents.
pub async fn detect_installed() -> Vec<DetectedAgent> {
    let dirs = effective_path_dirs().await;
    detect_in_dirs(&dirs, &home_dir()).await
}

#[cfg(test)]
mod detect_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_exe(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    fn tmp(prefix: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "synapse-acp-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn detect_finds_clis_adapters_and_login_state() {
        let bin = tmp("bin");
        let home = tmp("home");
        write_exe(&bin, "claude", "#!/bin/sh\necho 2.1.0\n");
        write_exe(&bin, "opencode", "#!/bin/sh\necho opencode 1.2.3\n");
        write_exe(&bin, "npx", "#!/bin/sh\necho 10\n");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(home.join(".claude/.credentials.json"), "{}").unwrap();

        let dirs = vec![bin.clone()];
        let found = detect_in_dirs(&dirs, &home).await;
        let ids: Vec<&str> = found.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"claude"), "found: {ids:?}");
        assert!(ids.contains(&"opencode"), "found: {ids:?}");
        assert!(!ids.contains(&"codex"), "found: {ids:?}");

        let claude = found.iter().find(|a| a.id == "claude").unwrap();
        assert_eq!(claude.cli_path, bin.join("claude"));
        assert_eq!(claude.version.as_deref(), Some("2.1.0"));
        assert_eq!(claude.adapter.command, "npx");
        assert!(claude.adapter.available);
        assert_eq!(claude.login, LoginState::LoggedIn);

        let opencode = found.iter().find(|a| a.id == "opencode").unwrap();
        assert_eq!(opencode.adapter.args, vec!["acp".to_string()]);
        // No marker file -> logged out (markers are known for opencode).
        assert_eq!(opencode.login, LoginState::LoggedOut);
    }

    #[tokio::test]
    async fn detect_unknown_login_and_missing_npx() {
        let bin = tmp("bin2");
        let home = tmp("home2");
        write_exe(&bin, "goose", "#!/bin/sh\nexit 1\n");
        // No npx/node on PATH: npx adapters unavailable, native ones available.
        let found = detect_in_dirs(std::slice::from_ref(&bin), &home).await;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "goose");
        assert_eq!(found[0].adapter.command, "goose");
        assert!(found[0].adapter.available);
        assert_eq!(found[0].login, LoginState::Unknown);
        // Failing --version yields None, not an error.
        assert_eq!(found[0].version, None);
    }

    #[tokio::test]
    async fn detect_empty_path_finds_nothing() {
        let home = tmp("home3");
        let found = detect_in_dirs(&[], &home).await;
        assert!(found.is_empty());
    }

    #[test]
    fn detect_login_state_strings() {
        assert_eq!(LoginState::LoggedIn.as_str(), "logged-in");
        assert_eq!(LoginState::LoggedOut.as_str(), "logged-out");
        assert_eq!(LoginState::Unknown.as_str(), "unknown");
    }

    #[tokio::test]
    async fn detect_registry_fetch_and_parse() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let body = serde_json::json!([
            {"id": "claude-acp", "label": "Claude"},
            {"id": "opencode", "description": "native ACP"},
        ]);
        Mock::given(method("GET"))
            .and(path("/registry.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let agents =
            crate::registry::fetch_registry(&client, &format!("{}/registry.json", server.uri()))
                .await
                .unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].id, "claude-acp");
        assert_eq!(agents[0].display_label(), "Claude");
        assert_eq!(agents[1].display_label(), "opencode");
    }

    #[test]
    fn detect_registry_parse_wrapped_shape() {
        let agents = crate::registry::parse_registry(
            r#"{"agents": [{"id": "gemini", "label": "Gemini CLI"}]}"#,
        )
        .unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "gemini");
    }
}
