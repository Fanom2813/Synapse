//! Agent data, validation and CRUD (see docs/PRD.md §5.2).
//!
//! [`AgentDraft`] is untrusted input from the agent editor. [`validate_draft`]
//! explains every problem before anything is saved: callers surface
//! [`ValidationError::issues`] in the form so the user can fix them.
//! [`AgentStore`] is the in-memory CRUD repository used by the app and tests;
//! persistence lives in `synapse-store` (backend/tools serialised as JSON).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Built-in tools an agent may select. `mcp:*` / names containing `:` are
/// MCP tools and pass through (the MCP manager owns their existence).
pub const KNOWN_TOOLS: &[&str] = &[
    "web_search",
    "read_url",
    "read_file",
    "write_file",
    "edit_file",
    "list_files",
    "shell",
    "ask_user",
    "notes",
    "create_artifact",
    "load_skill",
    "read_skill_file",
    // Team communication tools (see `team_tools::TEAM_TOOL_NAMES`).
    "ask_agent",
    "answer",
    "challenge",
    "agree",
    "refuse",
    "delegate",
    "report_finding",
    "propose",
    "vote",
    "ask_user",
    "done",
];

/// Reasoning effort for API backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiBackend {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub thinking: ThinkingLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliBackend {
    /// Registered CLI agent id, e.g. `claude`, `codex`, `opencode`.
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Model the CLI should run (its own id, e.g. `sonnet`); `None` = the CLI's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AgentBackend {
    Api(ApiBackend),
    Cli(CliBackend),
}

/// A saved, valid agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub backend: AgentBackend,
    pub tools: Vec<String>,
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub persona_source: String,
}

/// Unvalidated input from the agent editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AgentDraft {
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub backend: AgentBackend,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub persona_source: String,
}

impl Default for AgentBackend {
    fn default() -> Self {
        Self::Api(ApiBackend {
            provider: String::new(),
            model: String::new(),
            temperature: None,
            max_tokens: None,
            thinking: ThinkingLevel::Off,
        })
    }
}

/// Every reason an agent cannot be saved. Display joins them so the form can
/// show one bullet per issue.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid agent configuration: {issues:?}")]
pub struct ValidationError {
    pub issues: Vec<String>,
}

impl ValidationError {
    pub fn new(issues: Vec<String>) -> Self {
        Self { issues }
    }

    /// Single human-readable explanation (`"; "`-joined).
    pub fn explain(&self) -> String {
        self.issues.join("; ")
    }
}

/// Check a draft and return the normalised agent fields, or every problem.
pub fn validate_draft(draft: &AgentDraft) -> Result<ValidatedAgent, ValidationError> {
    let mut issues: Vec<String> = Vec::new();

    let name = draft.name.trim();
    if name.is_empty() {
        issues.push("name is required: give the agent a short name (e.g. \"Researcher\")".into());
    } else if name.chars().count() > 80 {
        issues.push("name is too long: keep it under 80 characters".into());
    }

    let role = draft.role.trim();
    if role.is_empty() {
        issues.push(
            "role is required: one line describing the agent's job (e.g. \"fact-checker\")".into(),
        );
    }

    if draft.instructions.trim().is_empty() {
        issues.push(
            "instructions are required: tell the agent how to behave before it can be saved".into(),
        );
    }

    match &draft.backend {
        AgentBackend::Api(api) => {
            if api.provider.trim().is_empty() {
                issues.push(
                    "API backend needs a provider: pick one from the catalog or add a custom endpoint"
                        .into(),
                );
            }
            if api.model.trim().is_empty() {
                issues.push(
                    "API backend needs a model: pick a model for the selected provider".into(),
                );
            }
            if let Some(t) = api.temperature {
                if !t.is_finite() || t < 0.0 || t > 2.0 {
                    issues.push(format!(
                        "temperature {t} is out of range: use a value between 0.0 and 2.0"
                    ));
                }
            }
            if let Some(m) = api.max_tokens {
                if m == 0 {
                    issues.push(
                        "max_tokens must be at least 1: clear it for the provider default".into(),
                    );
                }
            }
        }
        AgentBackend::Cli(cli) => {
            if cli.agent_id.trim().is_empty() {
                issues.push(
                    "CLI backend needs an agent: pick an installed CLI (e.g. \"claude\")".into(),
                );
            }
            if let Some(cmd) = &cli.command {
                if cmd.trim().is_empty() {
                    issues.push(
                        "CLI command is blank: clear it to use the default or enter a binary path"
                            .into(),
                    );
                }
            }
        }
    }

    // Tool selection: unknown names are rejected with the valid list, blanks
    // and duplicates are reported, MCP tools (`prefix:name`) pass through.
    let mut seen: Vec<String> = Vec::new();
    for tool in &draft.tools {
        let t = tool.trim();
        if t.is_empty() {
            issues.push("tool list contains a blank entry: remove it or pick a tool".into());
            continue;
        }
        if seen.iter().any(|s| s == t) {
            issues.push(format!(
                "tool \"{t}\" is listed twice: select each tool once"
            ));
            continue;
        }
        if !t.contains(':') && !KNOWN_TOOLS.contains(&t) {
            issues.push(format!(
                "unknown tool \"{t}\": pick from {} or an MCP tool (\"server:tool\")",
                KNOWN_TOOLS.join(", ")
            ));
            continue;
        }
        seen.push(t.to_string());
    }

    if !issues.is_empty() {
        return Err(ValidationError::new(issues));
    }

    Ok(ValidatedAgent {
        name: name.to_string(),
        role: role.to_string(),
        instructions: draft.instructions.trim().to_string(),
        backend: draft.backend.clone(),
        tools: seen,
        avatar: draft.avatar.clone(),
        persona_source: draft.persona_source.clone(),
    })
}

/// A draft that passed [`validate_draft`].
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedAgent {
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub backend: AgentBackend,
    pub tools: Vec<String>,
    pub avatar: String,
    pub persona_source: String,
}

impl ValidatedAgent {
    pub fn into_draft(self) -> AgentDraft {
        AgentDraft {
            name: self.name,
            role: self.role,
            instructions: self.instructions,
            backend: self.backend,
            tools: self.tools,
            avatar: self.avatar,
            persona_source: self.persona_source,
        }
    }
}

/// In-memory agent CRUD. IDs are `agent_<n>`; names stay unique by suffixing.
#[derive(Debug, Default)]
pub struct AgentStore {
    agents: HashMap<String, Agent>,
    next_id: u64,
}

impl AgentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.agents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Agent> {
        self.agents.get(id)
    }

    pub fn list(&self) -> Vec<&Agent> {
        let mut out: Vec<&Agent> = self.agents.values().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        out
    }

    /// Create and store a new agent. The draft must validate first; the
    /// returned error explains every problem.
    pub fn create(&mut self, draft: AgentDraft) -> Result<Agent, ValidationError> {
        let v = validate_draft(&draft)?;
        let id = self.alloc_id();
        let agent = Agent {
            id,
            name: self.unique_name(&v.name, None),
            role: v.role,
            instructions: v.instructions,
            backend: v.backend,
            tools: v.tools,
            avatar: v.avatar,
            persona_source: v.persona_source,
        };
        self.agents.insert(agent.id.clone(), agent.clone());
        Ok(agent)
    }

    /// Replace every field of an existing agent.
    pub fn update(&mut self, id: &str, draft: AgentDraft) -> Result<Agent, StoreError> {
        let v = validate_draft(&draft).map_err(StoreError::Invalid)?;
        let cur = self
            .agents
            .get(id)
            .ok_or_else(|| StoreError::NotFound(id.into()))?;
        let name = if v.name == cur.name {
            cur.name.clone()
        } else {
            self.unique_name(&v.name, Some(id))
        };
        let agent = Agent {
            id: id.to_string(),
            name,
            role: v.role,
            instructions: v.instructions,
            backend: v.backend,
            tools: v.tools,
            avatar: v.avatar,
            persona_source: v.persona_source,
        };
        self.agents.insert(id.to_string(), agent.clone());
        Ok(agent)
    }

    /// Copy an agent with a fresh id. An optional new name may be given;
    /// otherwise `"Name (copy)"` (suffixed until unique).
    pub fn duplicate(&mut self, id: &str, new_name: Option<&str>) -> Result<Agent, StoreError> {
        let src = self
            .agents
            .get(id)
            .ok_or_else(|| StoreError::NotFound(id.into()))?
            .clone();
        let base = match new_name {
            Some(n) if !n.trim().is_empty() => n.trim().to_string(),
            _ => format!("{} (copy)", src.name),
        };
        let draft = AgentDraft {
            name: base,
            role: src.role,
            instructions: src.instructions,
            backend: src.backend,
            tools: src.tools,
            avatar: src.avatar,
            persona_source: src.persona_source,
        };
        self.create(draft).map_err(StoreError::Invalid)
    }

    /// Delete an agent. Missing ids are an error so the UI can say so.
    pub fn delete(&mut self, id: &str) -> Result<(), StoreError> {
        self.agents
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| StoreError::NotFound(id.into()))
    }

    fn alloc_id(&mut self) -> String {
        self.next_id += 1;
        format!("agent_{}", self.next_id)
    }

    fn unique_name(&self, want: &str, exclude_id: Option<&str>) -> String {
        let taken = |n: &str| {
            self.agents
                .values()
                .any(|a| a.name == n && Some(a.id.as_str()) != exclude_id)
        };
        if !taken(want) {
            return want.to_string();
        }
        let mut n = 2u32;
        loop {
            let candidate = format!("{want} ({n})");
            if !taken(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StoreError {
    #[error("invalid agent configuration: {0}")]
    Invalid(ValidationError),
    #[error("agent not found: {0}")]
    NotFound(String),
}

pub fn api_draft(
    name: &str,
    role: &str,
    instructions: &str,
    provider: &str,
    model: &str,
) -> AgentDraft {
    AgentDraft {
        name: name.into(),
        role: role.into(),
        instructions: instructions.into(),
        backend: AgentBackend::Api(ApiBackend {
            provider: provider.into(),
            model: model.into(),
            temperature: None,
            max_tokens: None,
            thinking: ThinkingLevel::Off,
        }),
        tools: Vec::new(),
        avatar: String::new(),
        persona_source: String::new(),
    }
}

#[cfg(test)]
mod agents_tests {
    use super::*;

    fn good_draft() -> AgentDraft {
        let mut d = api_draft("Ada", "researcher", "Be rigorous.", "openai", "gpt-4o");
        d.tools = vec!["web_search".into(), "read_url".into()];
        d
    }

    #[test]
    fn agents_create_validates_and_assigns_id() {
        let mut store = AgentStore::new();
        let a = store.create(good_draft()).expect("valid draft saves");
        assert!(a.id.starts_with("agent_"));
        assert_eq!(a.tools, vec!["web_search", "read_url"]);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn agents_create_rejects_incomplete_config_with_explanation() {
        let mut store = AgentStore::new();
        let draft = AgentDraft {
            name: "  ".into(),
            role: "".into(),
            instructions: "".into(),
            backend: AgentBackend::Api(ApiBackend {
                provider: "".into(),
                model: "".into(),
                temperature: Some(9.0),
                max_tokens: Some(0),
                thinking: ThinkingLevel::Off,
            }),
            tools: vec!["nope".into()],
            ..Default::default()
        };
        let err = store.create(draft).expect_err("must not save");
        let text = err.explain();
        for want in [
            "name",
            "role",
            "instructions",
            "provider",
            "model",
            "temperature",
            "max_tokens",
            "nope",
        ] {
            assert!(
                text.contains(want),
                "explanation must mention {want}: {text}"
            );
        }
        assert!(store.is_empty());
    }

    #[test]
    fn agents_cli_backend_requires_agent_selection() {
        let mut store = AgentStore::new();
        let mut d = good_draft();
        d.backend = AgentBackend::Cli(CliBackend {
            agent_id: "".into(),
            command: None,
            args: vec![],
            model: None,
        });
        let err = store
            .create(d)
            .expect_err("cli without agent must not save");
        assert!(err.explain().contains("CLI backend needs an agent"));
    }

    #[test]
    fn agents_update_edits_fields_and_revalidates() {
        let mut store = AgentStore::new();
        let a = store.create(good_draft()).unwrap();
        let mut d = good_draft();
        d.instructions = "Be even more rigorous.".into();
        d.backend = AgentBackend::Api(ApiBackend {
            provider: "anthropic".into(),
            model: "claude".into(),
            temperature: Some(0.2),
            max_tokens: None,
            thinking: ThinkingLevel::Medium,
        });
        let u = store.update(&a.id, d).expect("update");
        assert_eq!(u.instructions, "Be even more rigorous.");
        assert!(matches!(u.backend, AgentBackend::Api(_)));

        let bad = AgentDraft {
            name: "".into(),
            ..good_draft()
        };
        assert!(store.update(&a.id, bad).is_err());
        // Failed update keeps the old row.
        assert_eq!(store.get(&a.id).unwrap().name, "Ada");
    }

    #[test]
    fn agents_duplicate_copies_with_fresh_id_and_unique_name() {
        let mut store = AgentStore::new();
        let a = store.create(good_draft()).unwrap();
        let b = store.duplicate(&a.id, None).expect("duplicate");
        assert_ne!(a.id, b.id);
        assert_eq!(b.role, a.role);
        assert_eq!(b.backend, a.backend);
        assert!(b.name.contains("(copy)"));
        let c = store
            .duplicate(&a.id, Some("Ada"))
            .expect("duplicate with name");
        assert_ne!(c.name, "Ada");
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn agents_delete_removes_and_errors_on_missing() {
        let mut store = AgentStore::new();
        let a = store.create(good_draft()).unwrap();
        store.delete(&a.id).expect("delete");
        assert!(store.is_empty());
        assert!(store.delete(&a.id).is_err());
        assert!(store.duplicate(&a.id, None).is_err());
    }

    #[test]
    fn agents_tool_selection_rejects_blanks_dupes_unknown() {
        for (tools, want) in [
            (vec!["  "], "blank"),
            (vec!["web_search", "web_search"], "twice"),
            (vec![" Bogus "], "unknown"),
        ] {
            let mut d = good_draft();
            d.tools = tools.into_iter().map(String::from).collect();
            let err = validate_draft(&d).expect_err("bad tools must not validate");
            assert!(err.explain().contains(want), "got: {}", err.explain());
        }
        // MCP tools pass through.
        let mut d = good_draft();
        d.tools = vec!["github:list_issues".into()];
        assert!(validate_draft(&d).is_ok());
    }

    #[test]
    fn agents_backend_json_roundtrip() {
        let b = AgentBackend::Api(ApiBackend {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            temperature: Some(0.7),
            max_tokens: None,
            thinking: ThinkingLevel::Low,
        });
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.contains("\"type\":\"api\""));
        let back: AgentBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
    }
}
