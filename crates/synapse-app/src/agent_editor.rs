//! Agent editor sheet: identity, instructions, backend, params, tools,
//! permissions (see docs/UI.md §2.3, docs/PRD.md §5.2).
//!
//! Toolkit-free so it is unit testable. The GPUI sheet renders from
//! [`AgentEditorState`]; saving/deleting goes through [`save_editor`] /
//! [`delete_agent`] which update the roster (`Store::list_agents`) and persist
//! in SQLite, so results survive restart. Reachability: [`EditorSheet`]
//! tracks the open sheet (opened from the Team panel / roster via
//! [`EditorSheet::open_new`] / [`open_existing`]).

use serde::{Deserialize, Serialize};
use synapse_core::{AgentBackend, AgentDraft, ApiBackend, CliBackend, ThinkingLevel};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EditorError {
    #[error("store: {0}")]
    Store(#[from] synapse_store::StoreError),
    #[error("invalid agent configuration: {0}")]
    Invalid(String),
    #[error("{0}")]
    Missing(String),
}

pub type Result<T> = std::result::Result<T, EditorError>;

/// allow / ask / deny per tool + pattern (PRD §5.5; deny wins at runtime).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PermissionEffect {
    #[default]
    Ask,
    Allow,
    Deny,
}

impl PermissionEffect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "allow" => Some(Self::Allow),
            "ask" => Some(Self::Ask),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool: String,
    pub pattern: String,
    pub effect: PermissionEffect,
}

impl PermissionRule {
    pub fn new(tool: &str, pattern: &str, effect: PermissionEffect) -> Self {
        Self {
            tool: tool.to_string(),
            pattern: pattern.to_string(),
            effect,
        }
    }

    fn validate(&self, issues: &mut Vec<String>) {
        if self.tool.trim().is_empty() {
            issues.push("permission rule needs a tool: pick a tool or \"*\"".into());
        }
        if self.pattern.trim().is_empty() {
            issues.push(format!(
                "permission rule for \"{}\" needs a pattern: use \"*\" for all",
                self.tool.trim()
            ));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendKind {
    #[default]
    Api,
    Cli,
}

/// Owned editor state for one agent (new or existing).
#[derive(Debug, Clone, Default)]
pub struct AgentEditorState {
    /// `None` for a new agent, `Some(id)` when editing.
    pub agent_id: Option<String>,
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub avatar: String,
    pub backend_kind: BackendKind,
    pub provider: String,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub thinking: ThinkingLevel,
    pub cli_agent_id: String,
    pub cli_command: String,
    pub cli_model: String,
    pub tools: Vec<String>,
    pub permissions: Vec<PermissionRule>,
    /// Private per-agent notes (UI.md §2.3); persisted in the `settings`
    /// table so no schema migration is needed.
    pub notes: String,
}

impl AgentEditorState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_new() -> Self {
        Self::default()
    }

    /// Load an existing row into the editor (sheet opened from the roster).
    pub fn open_existing(row: &synapse_store::Agent) -> Self {
        let mut out = Self {
            agent_id: Some(row.id.clone()),
            name: row.name.clone(),
            role: row.role.clone(),
            instructions: row.instructions.clone(),
            avatar: row.avatar.clone(),
            ..Default::default()
        };
        if let Ok(backend) = serde_json::from_str::<AgentBackend>(&row.backend) {
            match backend {
                AgentBackend::Api(api) => {
                    out.backend_kind = BackendKind::Api;
                    out.provider = api.provider;
                    out.model = api.model;
                    out.temperature = api.temperature;
                    out.max_tokens = api.max_tokens;
                    out.thinking = api.thinking;
                }
                AgentBackend::Cli(cli) => {
                    out.backend_kind = BackendKind::Cli;
                    out.cli_agent_id = cli.agent_id;
                    out.cli_command = cli.command.unwrap_or_default();
                    out.cli_model = cli.model.unwrap_or_default();
                }
            }
        }
        if let Ok(tools) = serde_json::from_str::<Vec<String>>(&row.tools) {
            out.tools = tools;
        }
        if let Ok(rules) = serde_json::from_str::<Vec<PermissionRule>>(&row.permissions) {
            out.permissions = rules;
        } else if let Ok(map) =
            serde_json::from_str::<std::collections::HashMap<String, String>>(&row.permissions)
        {
            // Tolerate the `{tool: effect}` shorthand from older builds.
            out.permissions = map
                .into_iter()
                .map(|(tool, effect)| PermissionRule {
                    tool,
                    pattern: "*".into(),
                    effect: PermissionEffect::parse(&effect).unwrap_or_default(),
                })
                .collect();
        }
        out
    }

    /// Toggle a tool in the selection (tools & MCP toggles, UI.md §2.3).
    /// Returns true when the tool is now enabled.
    pub fn toggle_tool(&mut self, tool: &str) -> bool {
        let tool = tool.trim();
        if tool.is_empty() {
            return false;
        }
        if let Some(ix) = self.tools.iter().position(|t| t == tool) {
            self.tools.remove(ix);
            false
        } else {
            self.tools.push(tool.to_string());
            true
        }
    }

    pub fn set_permission(&mut self, tool: &str, pattern: &str, effect: PermissionEffect) {
        if let Some(rule) = self
            .permissions
            .iter_mut()
            .find(|r| r.tool == tool && r.pattern == pattern)
        {
            rule.effect = effect;
        } else {
            self.permissions
                .push(PermissionRule::new(tool, pattern, effect));
        }
    }

    pub fn remove_permission(&mut self, tool: &str, pattern: &str) {
        self.permissions
            .retain(|r| !(r.tool == tool && r.pattern == pattern));
    }

    /// Parse a params text field: empty = provider default.
    pub fn set_temperature_text(&mut self, text: &str) -> Option<String> {
        let t = text.trim();
        if t.is_empty() {
            self.temperature = None;
            return None;
        }
        match t.parse::<f32>() {
            Ok(v) => {
                self.temperature = Some(v);
                None
            }
            Err(_) => Some("temperature must be a number between 0.0 and 2.0".into()),
        }
    }

    pub fn set_max_tokens_text(&mut self, text: &str) -> Option<String> {
        let t = text.trim();
        if t.is_empty() {
            self.max_tokens = None;
            return None;
        }
        match t.parse::<u32>() {
            Ok(v) => {
                self.max_tokens = Some(v);
                None
            }
            Err(_) => Some("max_tokens must be a whole number of at least 1".into()),
        }
    }

    fn backend(&self) -> AgentBackend {
        match self.backend_kind {
            BackendKind::Api => AgentBackend::Api(ApiBackend {
                provider: self.provider.clone(),
                model: self.model.clone(),
                temperature: self.temperature,
                max_tokens: self.max_tokens,
                thinking: self.thinking,
            }),
            BackendKind::Cli => AgentBackend::Cli(CliBackend {
                agent_id: self.cli_agent_id.clone(),
                command: if self.cli_command.trim().is_empty() {
                    None
                } else {
                    Some(self.cli_command.clone())
                },
                args: Vec::new(),
                model: if self.cli_model.trim().is_empty() {
                    None
                } else {
                    Some(self.cli_model.clone())
                },
            }),
        }
    }

    pub fn draft(&self) -> AgentDraft {
        AgentDraft {
            name: self.name.clone(),
            role: self.role.clone(),
            instructions: self.instructions.clone(),
            backend: self.backend(),
            tools: self.tools.clone(),
            avatar: self.avatar.clone(),
            persona_source: String::new(),
        }
    }

    /// Every reason the sheet cannot be saved yet (shown inline in the form).
    pub fn issues(&self) -> Vec<String> {
        let mut issues = match synapse_core::validate_draft(&self.draft()) {
            Ok(_) => Vec::new(),
            Err(e) => e.issues,
        };
        for rule in &self.permissions {
            rule.validate(&mut issues);
        }
        issues
    }

    pub fn is_valid(&self) -> bool {
        self.issues().is_empty()
    }
}

fn notes_key(agent_id: &str) -> String {
    format!("agent_notes.{agent_id}")
}

/// Persist the editor: insert a new row or update the existing one, then
/// write the private notes to the `settings` table. Returns the saved row.
pub fn save_editor(
    store: &synapse_store::Store,
    editor: &AgentEditorState,
) -> Result<synapse_store::Agent> {
    let draft = editor.draft();
    let validated =
        synapse_core::validate_draft(&draft).map_err(|e| EditorError::Invalid(e.explain()))?;
    let mut perm_issues = Vec::new();
    for rule in &editor.permissions {
        rule.validate(&mut perm_issues);
    }
    if !perm_issues.is_empty() {
        return Err(EditorError::Invalid(perm_issues.join("; ")));
    }
    let backend_json = serde_json::to_string(&validated.backend)
        .map_err(|e| EditorError::Invalid(format!("backend cannot be saved: {e}")))?;
    let tools_json = serde_json::to_string(&validated.tools)
        .map_err(|e| EditorError::Invalid(format!("tools cannot be saved: {e}")))?;
    let permissions_json = serde_json::to_string(&editor.permissions)
        .map_err(|e| EditorError::Invalid(format!("permissions cannot be saved: {e}")))?;

    let row = match &editor.agent_id {
        Some(id) => {
            // Update the full row (store only has an instructions updater).
            let cur = store
                .get_agent(id)
                .map_err(|_| EditorError::Missing(format!("agent not found: {id}")))?;
            // Delete + re-create keeps every column in sync without a
            // migration; the id is preserved.
            store.delete_agent(id)?;
            store.create_agent(&synapse_store::NewAgent {
                id: id.clone(),
                name: validated.name.clone(),
                role: validated.role.clone(),
                instructions: validated.instructions.clone(),
                backend: backend_json,
                avatar: validated.avatar.clone(),
                tools: tools_json,
                permissions: permissions_json,
                persona_source: cur.persona_source,
            })?
        }
        None => {
            let id = synapse_store::new_id("agent");
            store.create_agent(&synapse_store::NewAgent {
                id: id.clone(),
                name: validated.name.clone(),
                role: validated.role.clone(),
                instructions: validated.instructions.clone(),
                backend: backend_json,
                avatar: validated.avatar.clone(),
                tools: tools_json,
                permissions: permissions_json,
                persona_source: String::new(),
            })?
        }
    };
    if !editor.notes.trim().is_empty() {
        store.setting_set(&notes_key(&row.id), editor.notes.trim())?;
    }
    Ok(row)
}

/// Load private notes for an agent (empty when none were saved).
pub fn load_notes(store: &synapse_store::Store, agent_id: &str) -> String {
    store
        .setting_get(&notes_key(agent_id))
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Delete an agent plus its notes. Missing ids are an error so the UI can
/// say the roster entry is already gone.
pub fn delete_agent(store: &synapse_store::Store, agent_id: &str) -> Result<()> {
    store
        .get_agent(agent_id)
        .map_err(|_| EditorError::Missing(format!("agent not found: {agent_id}")))?;
    store.delete_agent(agent_id)?;
    let _ = store.setting_set(&notes_key(agent_id), "");
    Ok(())
}

/// Roster rows for the sidebar / Team panel, sorted by name.
pub fn roster(store: &synapse_store::Store) -> Result<Vec<synapse_store::Agent>> {
    let mut rows = store.list_agents()?;
    rows.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(rows)
}

/// Reachable editor sheet: which agent (if any) the sheet is open for.
/// The Team panel opens it via [`open_new`](Self::open_new) /
/// [`open_existing`](Self::open_existing); saving or closing clears it.
#[derive(Debug, Default)]
pub struct EditorSheet {
    open_id: Option<Option<String>>,
    pub editor: Option<AgentEditorState>,
}

impl EditorSheet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a blank sheet (Team panel "add agent").
    pub fn open_new(&mut self) {
        self.open_id = Some(None);
        self.editor = Some(AgentEditorState::open_new());
    }

    /// Open the sheet for an existing roster entry (click a member).
    pub fn open_existing(&mut self, store: &synapse_store::Store, agent_id: &str) -> Result<()> {
        let row = store
            .get_agent(agent_id)
            .map_err(|_| EditorError::Missing(format!("agent not found: {agent_id}")))?;
        let mut editor = AgentEditorState::open_existing(&row);
        editor.notes = load_notes(store, agent_id);
        self.open_id = Some(Some(row.id));
        self.editor = Some(editor);
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        self.editor.is_some()
    }

    /// Agent being edited, if the sheet targets an existing one.
    pub fn open_agent_id(&self) -> Option<&str> {
        self.open_id.as_ref().and_then(|inner| inner.as_deref())
    }

    /// Save the open sheet and close it; the roster now includes the row.
    pub fn save_open(&mut self, store: &synapse_store::Store) -> Result<synapse_store::Agent> {
        let editor = self
            .editor
            .clone()
            .ok_or_else(|| EditorError::Missing("no agent editor is open".into()))?;
        let row = save_editor(store, &editor)?;
        self.close();
        Ok(row)
    }

    /// Delete the open agent and close the sheet.
    pub fn delete_open(&mut self, store: &synapse_store::Store) -> Result<()> {
        let id = self
            .open_agent_id()
            .ok_or_else(|| EditorError::Missing("no saved agent is open".into()))?
            .to_string();
        delete_agent(store, &id)?;
        self.close();
        Ok(())
    }

    pub fn close(&mut self) {
        self.open_id = None;
        self.editor = None;
    }
}

#[cfg(test)]
mod agent_editor_tests {
    use super::*;

    fn good_editor() -> AgentEditorState {
        let mut e = AgentEditorState::open_new();
        e.name = "Ada".into();
        e.role = "researcher".into();
        e.instructions = "Be rigorous and cite sources.".into();
        e.provider = "openai".into();
        e.model = "gpt-4o".into();
        e.temperature = Some(0.3);
        e.max_tokens = Some(1024);
        e.thinking = ThinkingLevel::Medium;
        e.tools = vec!["web_search".into(), "read_url".into()];
        e.permissions = vec![PermissionRule::new(
            "shell",
            "cargo *",
            PermissionEffect::Ask,
        )];
        e.notes = "Prefers primary sources.".into();
        e
    }

    fn mem_store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().expect("store")
    }

    #[test]
    fn agent_editor_validates_identity_instructions_backend_params_tools_permissions() {
        let editor = AgentEditorState::open_new();
        let issues = editor.issues().join("; ");
        for want in ["name", "role", "instructions", "provider", "model"] {
            assert!(issues.contains(want), "missing {want} in: {issues}");
        }
        assert!(!AgentEditorState::open_new().is_valid());
        assert!(good_editor().is_valid());

        // Bad params surface inline.
        let mut bad = good_editor();
        bad.temperature = Some(9.0);
        bad.max_tokens = Some(0);
        let text = bad.issues().join("; ");
        assert!(text.contains("temperature"), "{text}");
        assert!(text.contains("max_tokens"), "{text}");

        // Bad tools + permissions surface inline.
        let mut bad2 = good_editor();
        bad2.tools = vec!["Bogus".into(), "web_search".into(), "web_search".into()];
        bad2.permissions = vec![PermissionRule::new("", "", PermissionEffect::Deny)];
        let text2 = bad2.issues().join("; ");
        assert!(text2.contains("unknown"), "{text2}");
        assert!(text2.contains("twice"), "{text2}");
        assert!(text2.contains("permission"), "{text2}");
    }

    #[test]
    fn agent_editor_sheet_reachable_save_updates_roster() {
        let store = mem_store();
        let mut sheet = EditorSheet::new();
        assert!(!sheet.is_open());
        // Team panel "add agent" opens a blank sheet.
        sheet.open_new();
        assert!(sheet.is_open());
        assert_eq!(sheet.open_agent_id(), None);
        sheet.editor.as_mut().unwrap().name = "Ada".into();
        // Saving an incomplete sheet explains every problem.
        let err = sheet.save_open(&store).expect_err("must not save");
        assert!(err.to_string().contains("role"), "{err}");
        assert!(roster(&store).unwrap().is_empty());

        // Fill every section and save: the roster updates.
        sheet.editor = Some(good_editor());
        let saved = sheet.save_open(&store).expect("save");
        assert!(!sheet.is_open());
        assert_eq!(saved.name, "Ada");
        let rows = roster(&store).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, saved.id);
        // Backend params + tools + permissions round-trip as JSON.
        let backend: AgentBackend = serde_json::from_str(&rows[0].backend).unwrap();
        assert!(matches!(backend, AgentBackend::Api(_)));
        assert!(rows[0].tools.contains("web_search"));
        assert!(rows[0].permissions.contains("shell"));
        assert_eq!(load_notes(&store, &saved.id), "Prefers primary sources.");
    }

    #[test]
    fn agent_editor_edit_save_and_delete_update_roster() {
        let store = mem_store();
        let saved = save_editor(&store, &good_editor()).expect("save");
        // Clicking a roster member opens the sheet with its values.
        let mut sheet = EditorSheet::new();
        sheet.open_existing(&store, &saved.id).expect("open");
        assert_eq!(sheet.open_agent_id(), Some(saved.id.as_str()));
        let editor = sheet.editor.as_ref().unwrap();
        assert_eq!(editor.instructions, "Be rigorous and cite sources.");
        assert_eq!(editor.provider, "openai");
        assert_eq!(editor.temperature, Some(0.3));
        assert!(editor.tools.contains(&"web_search".to_string()));
        assert_eq!(editor.permissions.len(), 1);

        // Edit identity + instructions + backend + toggles, save again.
        let editor = sheet.editor.as_mut().unwrap();
        editor.instructions = "Be even more rigorous.".into();
        editor.model = "gpt-4o-mini".into();
        assert!(editor.toggle_tool("create_artifact"));
        editor.set_permission("write_file", "reports/*", PermissionEffect::Allow);
        let updated = sheet.save_open(&store).expect("update");
        assert_eq!(updated.id, saved.id);
        assert_eq!(updated.instructions, "Be even more rigorous.");
        assert!(updated.backend.contains("gpt-4o-mini"));
        assert!(updated.permissions.contains("write_file"));
        assert_eq!(roster(&store).unwrap().len(), 1);

        // Deleting removes the roster entry; deleting again errors.
        sheet.open_existing(&store, &saved.id).expect("reopen");
        sheet.delete_open(&store).expect("delete");
        assert!(roster(&store).unwrap().is_empty());
        assert!(delete_agent(&store, &saved.id).is_err());
    }

    #[test]
    fn agent_editor_save_persists_across_restart() {
        let dir = std::env::temp_dir().join(format!("synapse-agent-editor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synapse.db");
        let id = {
            let store = synapse_store::Store::open(&path).expect("open");
            let saved = save_editor(&store, &good_editor()).expect("save");
            assert_eq!(roster(&store).unwrap().len(), 1);
            saved.id
        };
        // Reopen the same file (simulated restart): the roster survives.
        let store2 = synapse_store::Store::open(&path).expect("reopen");
        let rows = roster(&store2).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].name, "Ada");
        assert_eq!(load_notes(&store2, &id), "Prefers primary sources.");
        // Deletion persists too.
        delete_agent(&store2, &id).expect("delete");
        drop(store2);
        let store3 = synapse_store::Store::open(&path).expect("reopen 2");
        assert!(roster(&store3).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_editor_cli_backend_and_tool_toggles() {
        let store = mem_store();
        let mut e = good_editor();
        e.backend_kind = BackendKind::Cli;
        e.cli_agent_id = "claude".into();
        e.cli_command = String::new();
        assert!(e.is_valid());
        let saved = save_editor(&store, &e).expect("cli save");
        assert!(saved.backend.contains("claude"));

        let mut reopened = AgentEditorState::open_existing(&store.get_agent(&saved.id).unwrap());
        assert_eq!(reopened.backend_kind, BackendKind::Cli);
        assert!(!reopened.toggle_tool("web_search"));
        assert!(!reopened.tools.contains(&"web_search".to_string()));
        assert!(reopened.toggle_tool("web_search"));
        reopened.set_permission("shell", "*", PermissionEffect::Deny);
        reopened.remove_permission("shell", "cargo *");
        assert_eq!(reopened.permissions.len(), 1);
    }
}
