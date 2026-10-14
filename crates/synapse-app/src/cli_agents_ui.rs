//! CLI agent settings + picker (ARCHITECTURE.md §2, MILESTONES.md).
//!
//! Toolkit-free so it is unit testable. Shows the CLI agents detected by
//! `synapse-acp` with their login state, explains when an agent is
//! unavailable or needs configuration, persists the user's preferred CLI,
//! and lets the agent editor pick an installed CLI as an agent backend.

use thiserror::Error;

pub use synapse_acp::{DetectedAgent, LoginState};

#[derive(Debug, Error)]
pub enum CliAgentsError {
    #[error("store: {0}")]
    Store(#[from] synapse_store::StoreError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, CliAgentsError>;

const KEY_SELECTED: &str = "cli_agents.selected";

/// Whether the agent can be used right now (its ACP adapter is runnable).
pub fn agent_usable(agent: &DetectedAgent) -> bool {
    agent.adapter.available
}

/// Whether the agent is installed but needs the user to log in first.
pub fn agent_needs_login(agent: &DetectedAgent) -> bool {
    agent.login == LoginState::LoggedOut
}

/// One-line status for the settings list, including login state.
pub fn status_line(agent: &DetectedAgent) -> String {
    let version = agent
        .version
        .as_deref()
        .map(|v| format!(" {v}"))
        .unwrap_or_default();
    let base = format!("{}{}", agent.label, version);
    if !agent.adapter.available {
        return format!("{base} · unavailable — {}", adapter_hint(agent));
    }
    match agent.login {
        LoginState::LoggedIn => format!("{base} · logged in · ready"),
        LoginState::LoggedOut => {
            format!(
                "{base} · not logged in — run `{}` to log in",
                login_command(agent)
            )
        }
        LoginState::Unknown => format!("{base} · ready"),
    }
}

/// How to fix an unavailable agent, or log in where needed. `None` when the
/// agent is ready and no action is needed.
pub fn setup_hint(agent: &DetectedAgent) -> Option<String> {
    if !agent.adapter.available {
        return Some(adapter_hint(agent));
    }
    match agent.login {
        LoginState::LoggedOut => Some(format!(
            "Not logged in. Run `{}` in a terminal, then refresh this list.",
            login_command(agent)
        )),
        LoginState::Unknown => Some(
            "Login state is unknown for this agent; if runs fail, log in with its own CLI first."
                .to_string(),
        ),
        LoginState::LoggedIn => None,
    }
}

fn adapter_hint(agent: &DetectedAgent) -> String {
    if agent.adapter.command == "npx" {
        "the ACP adapter needs Node/npx on PATH; install Node 20+ and refresh".to_string()
    } else {
        format!(
            "the ACP adapter `{}` {} is not runnable; update the CLI and refresh",
            agent.adapter.command,
            agent.adapter.args.join(" ")
        )
    }
}

fn login_command(agent: &DetectedAgent) -> String {
    format!("{} login", agent.cli_path.to_string_lossy())
}

/// One row of the Settings → CLI agents list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliAgentRow {
    pub id: String,
    pub label: String,
    pub version: Option<String>,
    pub login: LoginState,
    pub login_label: &'static str,
    pub adapter_available: bool,
    pub usable: bool,
    pub status: String,
    pub hint: Option<String>,
}

impl CliAgentRow {
    pub fn new(agent: &DetectedAgent) -> Self {
        Self {
            id: agent.id.clone(),
            label: agent.label.clone(),
            version: agent.version.clone(),
            login: agent.login,
            login_label: agent.login.as_str(),
            adapter_available: agent.adapter.available,
            usable: agent_usable(agent),
            status: status_line(agent),
            hint: setup_hint(agent),
        }
    }
}

/// Persisted CLI-agent preference: the default CLI backend for new agents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliAgentsSettings {
    pub selected: Option<String>,
}

impl CliAgentsSettings {
    pub fn load(store: &synapse_store::Store) -> Result<Self> {
        let raw = store.setting_get(KEY_SELECTED)?;
        let selected = raw
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty());
        Ok(Self { selected })
    }

    pub fn save(&self, store: &synapse_store::Store) -> Result<()> {
        match &self.selected {
            Some(id) => store.setting_set(KEY_SELECTED, &serde_json::to_string(id)?)?,
            None => store.setting_set(KEY_SELECTED, "")?,
        }
        Ok(())
    }
}

/// Toolkit-free view-model for the Settings → CLI agents screen. The GPUI
/// view renders [`CliAgentRow`]s from [`CliAgentsScreen::rows`]; detection
/// results are pushed in via [`CliAgentsScreen::refresh`].
#[derive(Debug, Clone, Default)]
pub struct CliAgentsScreen {
    settings: CliAgentsSettings,
    detected: Vec<DetectedAgent>,
    status: Option<String>,
    status_ok: bool,
}

impl CliAgentsScreen {
    /// Open the screen; corrupt rows fall back to defaults so it always opens.
    pub fn open(store: &synapse_store::Store) -> Self {
        Self {
            settings: CliAgentsSettings::load(store).unwrap_or_default(),
            ..Self::default()
        }
    }

    /// Push fresh detection results (sorted by label). Keeps the selection
    /// only when the agent is still detected.
    pub fn refresh(&mut self, mut detected: Vec<DetectedAgent>) {
        detected.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
        self.detected = detected;
        let drop_selection = self
            .settings
            .selected
            .clone()
            .is_some_and(|sel| !self.detected.iter().any(|a| a.id == sel));
        if drop_selection {
            self.settings.selected = None;
        }
        self.clear_status();
    }

    pub fn rows(&self) -> Vec<CliAgentRow> {
        self.detected.iter().map(CliAgentRow::new).collect()
    }

    /// Summary line for the empty/header state, e.g. "2 of 3 ready".
    pub fn summary(&self) -> String {
        summarize(&self.detected)
    }

    pub fn selected(&self) -> Option<&str> {
        self.settings.selected.as_deref()
    }

    pub fn selected_agent(&self) -> Option<&DetectedAgent> {
        self.settings
            .selected
            .as_deref()
            .and_then(|id| self.detected.iter().find(|a| a.id == id))
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn status_ok(&self) -> bool {
        self.status_ok
    }

    fn set_status(&mut self, ok: bool, msg: impl Into<String>) {
        self.status_ok = ok;
        self.status = Some(msg.into());
    }

    fn clear_status(&mut self) {
        self.status = None;
        self.status_ok = false;
    }

    /// Select an installed, usable CLI as the default backend and persist.
    /// Explains when the agent is missing, unavailable, or needs login.
    pub fn select(&mut self, store: &synapse_store::Store, agent_id: &str) -> Result<()> {
        let agent = self
            .detected
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| {
                CliAgentsError::Invalid(format!(
                    "agent '{agent_id}' is not installed; install its CLI and refresh"
                ))
            })?;
        if !agent.adapter.available {
            let msg = format!("{} is unavailable: {}", agent.label, adapter_hint(agent));
            self.set_status(false, msg.clone());
            return Err(CliAgentsError::Invalid(msg));
        }
        self.settings.selected = Some(agent.id.clone());
        self.settings.save(store)?;
        if agent_needs_login(agent) {
            self.set_status(
                true,
                format!(
                    "Selected {}. Note: not logged in — run `{}` first.",
                    agent.label,
                    login_command(agent)
                ),
            );
        } else {
            self.set_status(
                true,
                format!("Selected {} as the default CLI agent.", agent.label),
            );
        }
        Ok(())
    }

    /// Clear the default CLI selection.
    pub fn clear_selection(&mut self, store: &synapse_store::Store) -> Result<()> {
        self.settings.selected = None;
        self.settings.save(store)?;
        self.set_status(true, "Cleared the default CLI agent.");
        Ok(())
    }
}

/// "2 of 3 ready" style summary; empty detection explains what to do next.
pub fn summarize(detected: &[DetectedAgent]) -> String {
    if detected.is_empty() {
        return "No CLI agents found on PATH. Install one (e.g. Claude Code, Codex, Gemini CLI) and refresh."
            .to_string();
    }
    let ready = detected.iter().filter(|a| agent_usable(a)).count();
    if ready == detected.len() {
        format!("{ready} of {} ready", detected.len())
    } else {
        format!(
            "{ready} of {} ready — {} need configuration",
            detected.len(),
            detected.len() - ready
        )
    }
}

/// Choices the agent-editor picker can offer (usable agents only, sorted).
pub fn available_choices(detected: &[DetectedAgent]) -> Vec<&DetectedAgent> {
    let mut out: Vec<&DetectedAgent> = detected.iter().filter(|a| agent_usable(a)).collect();
    out.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    out
}

/// Apply a detected CLI to the agent editor as its backend. Fails with an
/// explanation when the agent is missing, unavailable, or (as a warning
/// carried in the error) not logged in — the caller shows the message.
pub fn pick_cli_for_editor(
    editor: &mut crate::agent_editor::AgentEditorState,
    detected: &[DetectedAgent],
    agent_id: &str,
) -> Result<()> {
    use crate::agent_editor::BackendKind;
    let agent = detected.iter().find(|a| a.id == agent_id).ok_or_else(|| {
        CliAgentsError::Invalid(format!(
            "agent '{agent_id}' is not installed; install its CLI and refresh"
        ))
    })?;
    if !agent.adapter.available {
        return Err(CliAgentsError::Invalid(format!(
            "{} is unavailable: {}",
            agent.label,
            adapter_hint(agent)
        )));
    }
    editor.backend_kind = BackendKind::Cli;
    editor.cli_agent_id = agent.id.clone();
    editor.cli_command = String::new();
    if agent_needs_login(agent) {
        return Err(CliAgentsError::Invalid(format!(
            "Selected {}, but it is not logged in — run `{}` first, then save.",
            agent.label,
            login_command(agent)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod cli_agents_ui_tests {
    use super::*;
    use std::path::PathBuf;

    fn mem_store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().unwrap()
    }

    fn agent(id: &str, label: &str, login: LoginState, adapter_available: bool) -> DetectedAgent {
        DetectedAgent {
            id: id.into(),
            label: label.into(),
            cli_path: PathBuf::from(format!("/usr/local/bin/{id}")),
            version: Some("1.0.0".into()),
            adapter: synapse_acp::AvailableAdapter {
                command: if id == "claude" {
                    "npx".into()
                } else {
                    id.into()
                },
                args: vec!["acp".into()],
                available: adapter_available,
            },
            login,
        }
    }

    fn sample() -> Vec<DetectedAgent> {
        vec![
            agent("opencode", "OpenCode", LoginState::LoggedOut, true),
            agent("claude", "Claude Code", LoginState::LoggedIn, true),
            agent("goose", "Goose", LoginState::Unknown, true),
        ]
    }

    #[test]
    fn cli_agents_ui_status_explains_login_and_availability() {
        let ready = agent("claude", "Claude Code", LoginState::LoggedIn, true);
        assert!(agent_usable(&ready));
        assert!(!agent_needs_login(&ready));
        assert!(status_line(&ready).contains("logged in"));
        assert!(setup_hint(&ready).is_none());

        let logged_out = agent("opencode", "OpenCode", LoginState::LoggedOut, true);
        assert!(agent_usable(&logged_out));
        assert!(agent_needs_login(&logged_out));
        assert!(status_line(&logged_out).contains("not logged in"));
        let hint = setup_hint(&logged_out).unwrap();
        assert!(hint.contains("Not logged in"), "{hint}");

        let unknown = agent("goose", "Goose", LoginState::Unknown, true);
        assert!(agent_usable(&unknown));
        assert!(status_line(&unknown).contains("ready"));
        assert!(setup_hint(&unknown).is_some());

        let missing_npx = agent("claude", "Claude Code", LoginState::LoggedOut, false);
        assert!(!agent_usable(&missing_npx));
        let msg = status_line(&missing_npx);
        assert!(msg.contains("unavailable"), "{msg}");
        assert!(msg.contains("npx"), "{msg}");
        let hint = setup_hint(&missing_npx).unwrap();
        assert!(hint.contains("npx"), "{hint}");

        let native_missing = agent("goose", "Goose", LoginState::Unknown, false);
        assert!(status_line(&native_missing).contains("unavailable"));
    }

    #[test]
    fn cli_agents_ui_rows_sorted_and_summary() {
        let store = mem_store();
        let mut screen = CliAgentsScreen::open(&store);
        assert!(screen.rows().is_empty());
        assert!(
            screen.summary().contains("No CLI agents"),
            "{}",
            screen.summary()
        );

        screen.refresh(sample());
        let rows = screen.rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "Claude Code");
        assert_eq!(rows[0].login_label, "logged-in");
        assert!(rows[0].usable && rows[0].hint.is_none());
        assert!(rows.iter().any(|r| r.hint.is_some()));
        assert_eq!(screen.summary(), "3 of 3 ready");

        // One unavailable agent shows in the summary.
        let mut mixed = sample();
        mixed.push(agent("codex", "Codex", LoginState::LoggedOut, false));
        screen.refresh(mixed);
        assert_eq!(screen.summary(), "3 of 4 ready — 1 need configuration");
    }

    #[test]
    fn cli_agents_ui_select_persists_and_explains() {
        let store = mem_store();
        let mut screen = CliAgentsScreen::open(&store);
        screen.refresh(sample());

        screen.select(&store, "claude").unwrap();
        assert_eq!(screen.selected(), Some("claude"));
        assert!(screen.status_ok());
        assert!(
            screen.status().unwrap().contains("Selected"),
            "{:?}",
            screen.status()
        );
        assert_eq!(screen.selected_agent().unwrap().id, "claude");

        // Selecting an agent that needs login still saves, with a warning.
        screen.select(&store, "opencode").unwrap();
        assert_eq!(screen.selected(), Some("opencode"));
        assert!(
            screen.status().unwrap().contains("not logged in"),
            "{:?}",
            screen.status()
        );

        // Unknown ids say the CLI is not installed.
        let err = screen.select(&store, "nope").unwrap_err();
        assert!(err.to_string().contains("not installed"), "{err}");
        assert_eq!(screen.selected(), Some("opencode"));

        // Unavailable adapters explain the missing piece.
        let mut screen2 = CliAgentsScreen::open(&store);
        screen2.refresh(vec![agent("codex", "Codex", LoginState::LoggedOut, false)]);
        let err = screen2.select(&store, "codex").unwrap_err();
        assert!(err.to_string().contains("unavailable"), "{err}");
        assert!(screen2.selected().is_none());
        assert!(!screen2.status_ok());

        // Selection survives reopen; refresh drops it when the CLI is gone.
        let reopened = CliAgentsScreen::open(&store);
        assert_eq!(reopened.selected(), Some("opencode"));
        let mut reopened = reopened;
        reopened.refresh(vec![]);
        assert_eq!(reopened.selected(), None);

        // Clearing works and persists.
        let mut screen3 = CliAgentsScreen::open(&store);
        screen3.refresh(sample());
        screen3.clear_selection(&store).unwrap();
        assert_eq!(screen3.selected(), None);
        assert_eq!(CliAgentsScreen::open(&store).selected(), None);
    }

    #[test]
    fn cli_agents_ui_picker_applies_installed_cli_to_editor() {
        use crate::agent_editor::BackendKind;
        let detected = sample();
        let mut editor = crate::agent_editor::AgentEditorState::open_new();
        assert_eq!(editor.backend_kind, BackendKind::Api);

        // Ready agent applies cleanly as a CLI backend.
        pick_cli_for_editor(&mut editor, &detected, "claude").unwrap();
        assert_eq!(editor.backend_kind, BackendKind::Cli);
        assert_eq!(editor.cli_agent_id, "claude");

        // Only usable agents are offered, sorted by label.
        let choices = available_choices(&detected);
        assert_eq!(choices.len(), 3);
        assert_eq!(choices[0].id, "claude");

        // Missing CLI explains how to fix it.
        let mut editor2 = crate::agent_editor::AgentEditorState::open_new();
        let err = pick_cli_for_editor(&mut editor2, &detected, "codex").unwrap_err();
        assert!(err.to_string().contains("not installed"), "{err}");

        // Unavailable adapter explains the missing piece.
        let with_missing = vec![agent("codex", "Codex", LoginState::LoggedOut, false)];
        let err = pick_cli_for_editor(&mut editor2, &with_missing, "codex").unwrap_err();
        assert!(err.to_string().contains("unavailable"), "{err}");
        assert!(available_choices(&with_missing).is_empty());

        // Logged-out agent still applies but warns to log in first.
        let err = pick_cli_for_editor(&mut editor2, &detected, "opencode").unwrap_err();
        assert!(err.to_string().contains("not logged in"), "{err}");
        assert_eq!(editor2.cli_agent_id, "opencode");
    }
}
