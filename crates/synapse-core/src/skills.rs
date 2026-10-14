//! Skills runtime: progressive disclosure at global/project/agent scope.
//!
//! Only each skill's name + description sit in the agent's prompt
//! ([`SkillStore::prompt_listing`]); the agent calls `load_skill(name)` to read
//! the full body, then `read_skill_file` for `references/` siblings. Skill
//! scripts are never executed here: they run only through the shell tool and
//! its permission rules (see [`Skill::is_script`] / [`SHELL_NOTE`]).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Tool names agents call to load skill content (see `KNOWN_TOOLS`).
pub const LOAD_SKILL_TOOL: &str = "load_skill";
pub const READ_SKILL_FILE_TOOL: &str = "read_skill_file";

/// Reminder attached to script listings: execution goes via the shell tool.
pub const SHELL_NOTE: &str =
    "Skill scripts run only through the shell tool and its permission rules (ask by default).";

/// Where a skill folder lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SkillScope {
    Global,
    Project(String),
    Agent(String),
}

impl SkillScope {
    fn key(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Project(p) => format!("project:{p}"),
            Self::Agent(a) => format!("agent:{a}"),
        }
    }
}

/// One installed skill: metadata + full body + bundled reference files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    pub scope: SkillScope,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source: String,
    /// Library-level toggle (Skill library screen: enable/disable).
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Skill {
    pub fn script_paths(&self) -> Vec<&str> {
        self.files
            .keys()
            .filter(|p| Self::is_script_path(p))
            .map(String::as_str)
            .collect()
    }

    pub fn is_script(&self, path: &str) -> bool {
        Self::is_script_path(path)
    }

    fn is_script_path(path: &str) -> bool {
        path == "scripts" || path.starts_with("scripts/") && !path.contains('\0')
    }

    /// Reference files an agent may load with `read_skill_file`.
    pub fn reference_paths(&self) -> Vec<&str> {
        self.files.keys().map(String::as_str).collect()
    }
}

/// Input for installing a skill (parsed `SKILL.md` + sibling file contents).
#[derive(Debug, Clone)]
pub struct SkillInput {
    pub name: String,
    pub description: String,
    pub body: String,
    pub files: BTreeMap<String, String>,
    pub scope: SkillScope,
    pub license: String,
    pub source: String,
}

impl SkillInput {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        body: impl Into<String>,
        scope: SkillScope,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            body: body.into(),
            files: BTreeMap::new(),
            scope,
            license: String::new(),
            source: String::new(),
        }
    }

    pub fn with_file(mut self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.files.insert(path.into(), content.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillsError {
    #[error("invalid skill {0:?}: {1}")]
    Invalid(String, String),
    #[error("skill not found: {0}")]
    NotFound(String),
    #[error("skill {0:?} is disabled")]
    Disabled(String),
    #[error("skill file not found: {0:?} in skill {1:?}")]
    FileNotFound(String, String),
    #[error("rejected skill file path {0:?}: must be a relative path inside the skill folder")]
    BadPath(String),
    #[error("skill {0:?} already exists in this scope")]
    Duplicate(String),
}

pub fn validate_skill_name(name: &str) -> Result<(), SkillsError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(SkillsError::Invalid(
            name.to_string(),
            "must be lowercase alphanumeric/hyphen, max 64 chars".into(),
        ));
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), SkillsError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return Err(SkillsError::BadPath(path.to_string()));
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(SkillsError::BadPath(path.to_string()));
        }
    }
    Ok(())
}

/// Installed skills at every scope, plus per-agent enablement toggles.
#[derive(Debug, Default)]
pub struct SkillStore {
    skills: HashMap<(String, String), Skill>,
    /// `(agent_id, skill_name)` explicitly disabled; everything else enabled.
    agent_disabled: HashSet<(String, String)>,
    /// `(agent_id, skill_name)` explicitly enabled when the skill is off.
    agent_enabled: HashSet<(String, String)>,
}

impl SkillStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Install a skill folder. Names are unique per scope.
    pub fn install(&mut self, input: SkillInput) -> Result<Skill, SkillsError> {
        let name = input.name.trim().to_string();
        validate_skill_name(&name)?;
        let description = input.description.trim().to_string();
        if description.is_empty() {
            return Err(SkillsError::Invalid(name, "description is required".into()));
        }
        if description.len() > 1024 {
            return Err(SkillsError::Invalid(
                name,
                format!(
                    "description too long ({} chars, max 1024)",
                    description.len()
                ),
            ));
        }
        if input.body.trim().is_empty() {
            return Err(SkillsError::Invalid(name, "SKILL.md body is empty".into()));
        }
        for path in input.files.keys() {
            validate_path(path)?;
        }
        let key = (input.scope.key(), name.clone());
        if self.skills.contains_key(&key) {
            return Err(SkillsError::Duplicate(name));
        }
        let skill = Skill {
            name,
            description,
            body: input.body.trim().to_string(),
            files: input.files,
            scope: input.scope,
            license: input.license,
            source: input.source,
            enabled: true,
        };
        self.skills.insert(key, skill.clone());
        Ok(skill)
    }

    /// Replace body/files/description of an installed skill (update from source).
    pub fn update(
        &mut self,
        scope: &SkillScope,
        name: &str,
        description: Option<&str>,
        body: Option<&str>,
        files: Option<BTreeMap<String, String>>,
    ) -> Result<Skill, SkillsError> {
        let key = (scope.key(), name.to_string());
        let skill = self
            .skills
            .get_mut(&key)
            .ok_or_else(|| SkillsError::NotFound(name.into()))?;
        if let Some(d) = description {
            let d = d.trim().to_string();
            if d.is_empty() {
                return Err(SkillsError::Invalid(
                    name.into(),
                    "description is required".into(),
                ));
            }
            if d.len() > 1024 {
                return Err(SkillsError::Invalid(
                    name.into(),
                    "description too long".into(),
                ));
            }
            skill.description = d;
        }
        if let Some(b) = body {
            if b.trim().is_empty() {
                return Err(SkillsError::Invalid(
                    name.into(),
                    "SKILL.md body is empty".into(),
                ));
            }
            skill.body = b.trim().to_string();
        }
        if let Some(f) = files {
            for path in f.keys() {
                validate_path(path)?;
            }
            skill.files = f;
        }
        Ok(skill.clone())
    }

    pub fn delete(&mut self, scope: &SkillScope, name: &str) -> Result<(), SkillsError> {
        self.skills
            .remove(&(scope.key(), name.to_string()))
            .map(|_| ())
            .ok_or_else(|| SkillsError::NotFound(name.into()))
    }

    /// Library-level toggle.
    pub fn set_enabled(
        &mut self,
        scope: &SkillScope,
        name: &str,
        enabled: bool,
    ) -> Result<(), SkillsError> {
        let key = (scope.key(), name.to_string());
        self.skills
            .get_mut(&key)
            .map(|s| s.enabled = enabled)
            .ok_or_else(|| SkillsError::NotFound(name.into()))
    }

    /// Per-agent toggle (Skill library screen: enable per agent).
    pub fn set_agent_enabled(&mut self, agent_id: &str, skill_name: &str, enabled: bool) {
        let key = (agent_id.to_string(), skill_name.to_string());
        if enabled {
            self.agent_disabled.remove(&key);
            // Remember the opt-in so a library-disabled skill still resolves.
            self.agent_enabled.insert(key);
        } else {
            self.agent_enabled.remove(&key);
            self.agent_disabled.insert(key);
        }
    }

    pub fn agent_skill_enabled(&self, agent_id: &str, skill: &Skill) -> bool {
        let key = (agent_id.to_string(), skill.name.clone());
        if self.agent_disabled.contains(&key) {
            return false;
        }
        if skill.enabled {
            return true;
        }
        self.agent_enabled.contains(&key)
    }

    fn visible_skills(&self, project_id: Option<&str>, agent_id: &str) -> Vec<&Skill> {
        let mut out: Vec<&Skill> = self
            .skills
            .values()
            .filter(|s| {
                if !self.agent_skill_enabled(agent_id, s) {
                    return false;
                }
                match &s.scope {
                    SkillScope::Global => true,
                    SkillScope::Project(p) => Some(p.as_str()) == project_id,
                    SkillScope::Agent(a) => a == agent_id,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Skills effective for one agent in one project (global + project + agent).
    pub fn effective_skills(&self, project_id: Option<&str>, agent_id: &str) -> Vec<Skill> {
        self.visible_skills(project_id, agent_id)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Progressive-disclosure prompt fragment: names + descriptions only.
    /// Full bodies are never injected; agents call `load_skill(name)`.
    pub fn prompt_listing(&self, project_id: Option<&str>, agent_id: &str) -> String {
        let skills = self.visible_skills(project_id, agent_id);
        if skills.is_empty() {
            return String::new();
        }
        let mut out = String::from("Available skills (call load_skill(name) to read one):");
        for s in skills {
            out.push_str(&format!("\n- {}: {}", s.name, s.description));
        }
        out
    }

    /// `load_skill(name)`: full SKILL.md body for a visible, enabled skill.
    pub fn load_skill(
        &self,
        project_id: Option<&str>,
        agent_id: &str,
        name: &str,
    ) -> Result<Skill, SkillsError> {
        let key = name.trim();
        let found = self
            .visible_skills(project_id, agent_id)
            .into_iter()
            .find(|s| s.name == key)
            .cloned();
        match found {
            Some(s) => Ok(s),
            None => {
                // Distinguish "exists but not visible/disabled" from unknown.
                if self.skills.values().any(|s| s.name == key) {
                    Err(SkillsError::Disabled(key.into()))
                } else {
                    Err(SkillsError::NotFound(key.into()))
                }
            }
        }
    }

    /// `read_skill_file(skill, path)`: one sibling file (references/assets).
    /// Script files are returned as text with [`SHELL_NOTE`]; running them
    /// must go through the shell tool and its permission rules.
    pub fn read_skill_file(
        &self,
        project_id: Option<&str>,
        agent_id: &str,
        skill_name: &str,
        path: &str,
    ) -> Result<String, SkillsError> {
        validate_path(path)?;
        let skill = self.load_skill(project_id, agent_id, skill_name)?;
        skill
            .files
            .get(path)
            .cloned()
            .ok_or_else(|| SkillsError::FileNotFound(path.into(), skill_name.into()))
    }

    /// Preview without scope filtering (library screen).
    pub fn preview(&self, scope: &SkillScope, name: &str) -> Result<Skill, SkillsError> {
        self.skills
            .get(&(scope.key(), name.to_string()))
            .cloned()
            .ok_or_else(|| SkillsError::NotFound(name.into()))
    }

    /// Case-insensitive substring search over name + description.
    pub fn search(&self, query: &str) -> Vec<Skill> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<Skill> = self
            .skills
            .values()
            .filter(|s| {
                q.is_empty()
                    || s.name.to_lowercase().contains(&q)
                    || s.description.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// `/skill-name` slash menu: enabled skills matching the prefix.
    pub fn slash_menu(&self, project_id: Option<&str>, agent_id: &str, prefix: &str) -> Vec<Skill> {
        let p = prefix.trim_start_matches('/').to_lowercase();
        self.visible_skills(project_id, agent_id)
            .into_iter()
            .filter(|s| s.name.to_lowercase().starts_with(&p))
            .cloned()
            .collect()
    }

    /// JSON schemas for the two agent tools.
    pub fn tool_specs() -> Vec<(String, String, serde_json::Value)> {
        vec![
            (
                LOAD_SKILL_TOOL.to_string(),
                "Read the full SKILL.md body of one enabled skill.".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {"name": {"type": "string"}},
                    "required": ["name"]
                }),
            ),
            (
                READ_SKILL_FILE_TOOL.to_string(),
                "Read one reference file bundled with an enabled skill.".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "skill": {"type": "string"},
                        "path": {"type": "string"}
                    },
                    "required": ["skill", "path"]
                }),
            ),
        ]
    }
}

#[cfg(test)]
mod skills_tests {
    use super::*;

    fn input(name: &str, scope: SkillScope) -> SkillInput {
        SkillInput {
            name: name.into(),
            description: format!("Use {name} for testing."),
            body: format!("# {name}\n\nDo the thing well.\n"),
            files: BTreeMap::from([
                ("references/notes.md".to_string(), "notes body".to_string()),
                ("scripts/run.sh".to_string(), "echo hi\n".to_string()),
            ]),
            scope,
            license: "MIT".into(),
            source: "synapse".into(),
        }
    }

    fn seeded_skills_store() -> SkillStore {
        let mut store = SkillStore::new();
        store
            .install(input("research-method", SkillScope::Global))
            .unwrap();
        store
            .install(input("report-writing", SkillScope::Project("p1".into())))
            .unwrap();
        store
            .install(input("agent-trick", SkillScope::Agent("a1".into())))
            .unwrap();
        store
    }

    #[test]
    fn skills_prompt_lists_names_not_bodies() {
        let store = seeded_skills_store();
        let listing = store.prompt_listing(Some("p1"), "a1");
        assert!(listing.contains("research-method"));
        assert!(listing.contains("report-writing"));
        assert!(listing.contains("agent-trick"));
        // Progressive disclosure: full bodies stay out of the prompt.
        assert!(!listing.contains("Do the thing well"));
        assert!(!listing.contains("echo hi"));
    }

    #[test]
    fn skills_scopes_resolve_per_agent_and_project() {
        let store = seeded_skills_store();
        let names = |p: Option<&str>, a: &str| {
            store
                .effective_skills(p, a)
                .into_iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(Some("p1"), "a1"),
            vec!["agent-trick", "report-writing", "research-method"]
        );
        // Other project: no project skill, other agent: no agent skill.
        assert_eq!(names(Some("p2"), "a9"), vec!["research-method"]);
        assert_eq!(names(None, "a1"), vec!["agent-trick", "research-method"]);
    }

    #[test]
    fn skills_load_skill_and_read_skill_file() {
        let store = seeded_skills_store();
        let s = store
            .load_skill(Some("p1"), "a1", "research-method")
            .unwrap();
        assert!(s.body.contains("Do the thing well"));
        let notes = store
            .read_skill_file(Some("p1"), "a1", "research-method", "references/notes.md")
            .unwrap();
        assert_eq!(notes, "notes body");
        // Scripts are readable as text; they only *run* via the shell tool.
        let script = store
            .read_skill_file(Some("p1"), "a1", "research-method", "scripts/run.sh")
            .unwrap();
        assert!(script.contains("echo hi"));
        assert!(store
            .preview(&SkillScope::Global, "research-method")
            .unwrap()
            .is_script("scripts/run.sh"));
        // Unknown skill / file / traversal are errors, never filesystem access.
        assert!(matches!(
            store.load_skill(Some("p1"), "a1", "nope"),
            Err(SkillsError::NotFound(_))
        ));
        assert!(matches!(
            store.read_skill_file(Some("p1"), "a1", "research-method", "references/missing.md"),
            Err(SkillsError::FileNotFound(_, _))
        ));
        assert!(matches!(
            store.read_skill_file(Some("p1"), "a1", "research-method", "../evil.md"),
            Err(SkillsError::BadPath(_))
        ));
        assert!(matches!(
            store.read_skill_file(Some("p1"), "a1", "research-method", "/abs.md"),
            Err(SkillsError::BadPath(_))
        ));
    }

    #[test]
    fn skills_disabled_and_per_agent_toggles() {
        let mut store = seeded_skills_store();
        store
            .set_enabled(&SkillScope::Global, "research-method", false)
            .unwrap();
        assert!(store
            .prompt_listing(Some("p1"), "a1")
            .contains("report-writing"));
        assert!(!store
            .prompt_listing(Some("p1"), "a1")
            .contains("research-method"));
        assert!(matches!(
            store.load_skill(Some("p1"), "a1", "research-method"),
            Err(SkillsError::Disabled(_))
        ));
        // Per-agent opt-in re-enables a library-disabled skill for one agent.
        store.set_agent_enabled("a1", "research-method", true);
        assert!(store
            .load_skill(Some("p1"), "a1", "research-method")
            .is_ok());
        assert!(matches!(
            store.load_skill(Some("p1"), "a2", "research-method"),
            Err(SkillsError::Disabled(_))
        ));
        // Per-agent opt-out hides a library-enabled skill for one agent.
        store.set_agent_enabled("a1", "report-writing", false);
        assert!(matches!(
            store.load_skill(Some("p1"), "a1", "report-writing"),
            Err(SkillsError::Disabled(_))
        ));
        assert!(store.load_skill(Some("p1"), "a2", "report-writing").is_ok());
    }

    #[test]
    fn skills_search_preview_slash_menu_update_delete() {
        let mut store = seeded_skills_store();
        let hits = store.search("REPORT");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "report-writing");
        assert_eq!(store.search("").len(), 3);
        // Slash menu lists enabled skills by prefix.
        let menu = store.slash_menu(Some("p1"), "a1", "/rep");
        assert_eq!(menu.len(), 1);
        assert_eq!(menu[0].name, "report-writing");
        // Update from source + delete.
        store
            .update(
                &SkillScope::Global,
                "research-method",
                None,
                Some("# v2 body"),
                None,
            )
            .unwrap();
        assert!(store
            .load_skill(Some("p1"), "a1", "research-method")
            .unwrap()
            .body
            .contains("v2"));
        store
            .delete(&SkillScope::Global, "research-method")
            .unwrap();
        assert!(matches!(
            store.preview(&SkillScope::Global, "research-method"),
            Err(SkillsError::NotFound(_))
        ));
        // Validation: bad names, blank description, duplicates rejected.
        assert!(store
            .install(input("Bad_Name", SkillScope::Global))
            .is_err());
        let mut bad = input("ok-name", SkillScope::Global);
        bad.description = "  ".into();
        assert!(matches!(
            store.install(bad),
            Err(SkillsError::Invalid(_, _))
        ));
        assert!(matches!(
            store.install(input("report-writing", SkillScope::Project("p1".into()))),
            Err(SkillsError::Duplicate(_))
        ));
        // Same name in a different scope is fine.
        assert!(store
            .install(input("report-writing", SkillScope::Global))
            .is_ok());
    }

    #[test]
    fn skills_tool_specs_cover_both_agent_tools() {
        let specs = SkillStore::tool_specs();
        let names: Vec<_> = specs.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, vec!["load_skill", "read_skill_file"]);
        for (_, _, schema) in &specs {
            assert_eq!(schema["type"], "object");
        }
    }
}
