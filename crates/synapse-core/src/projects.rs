//! Projects and team templates (see docs/PRD.md §5.2, docs/MILESTONES.md M3.7).
//!
//! [`ProjectDraft`] is untrusted input from the new-project form: name, goal,
//! folder, a team template, the lead agent and the member list.
//! [`validate_project`] explains every problem before anything is saved.
//! [`ProjectStore`] is the in-memory CRUD repository used by the app and
//! tests; persistence lives in `synapse-store` (rows serialised from
//! [`Project`]).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::agents::AgentStore;

/// One suggested seat inside a team template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedRole {
    pub role: String,
    pub instructions: String,
}

/// A team template: a named starting team. `custom` is the empty one the user
/// edits freely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub roles: Vec<SuggestedRole>,
}

fn role(role: &str, instructions: &str) -> SuggestedRole {
    SuggestedRole {
        role: role.into(),
        instructions: instructions.into(),
    }
}

/// Built-in templates that work offline (PRD §5.2).
pub fn builtin_templates() -> Vec<TeamTemplate> {
    vec![
        TeamTemplate {
            id: "research-squad".into(),
            name: "Research squad".into(),
            description: "Lead plus researchers, a fact-checker, a critic and a writer."
                .into(),
            roles: vec![
                role("Lead", "Plan the research, assign questions, converge on findings."),
                role("Researcher", "Search sources and report findings with citations."),
                role("Researcher", "Search sources and report findings with citations."),
                role("Researcher", "Search sources and report findings with citations."),
                role(
                    "Fact-checker",
                    "Verify every claim against its cited source.",
                ),
                role(
                    "Devil's advocate",
                    "Challenge weak claims with reasons and evidence.",
                ),
                role("Writer", "Synthesize findings into a cited report."),
            ],
        },
        TeamTemplate {
            id: "product-brainstorm".into(),
            name: "Product brainstorm".into(),
            description: "Lead plus diverse ideators and a critic to rank ideas.".into(),
            roles: vec![
                role("Lead", "Elicit distinct perspectives, then converge and rank."),
                role("Ideator", "Generate bold product ideas from your perspective."),
                role("Ideator", "Generate bold product ideas from your perspective."),
                role("Ideator", "Generate bold product ideas from your perspective."),
                role(
                    "Critic",
                    "Attack weak ideas with reasons; keep what survives.",
                ),
                role("Writer", "Summarize the ranked ideas and open questions."),
            ],
        },
        TeamTemplate {
            id: "investment-analysis".into(),
            name: "Investment analysis".into(),
            description: "Lead plus analysts covering risk, economics and first principles."
                .into(),
            roles: vec![
                role("Lead", "Plan the analysis and write the final recommendation."),
                role("Risk analyst", "Surface risks, downsides and falsifiability checks."),
                role("Economics analyst", "Model costs, incentives and market forces."),
                role(
                    "First-principles analyst",
                    "Break the thesis into fundamentals and test each claim.",
                ),
                role(
                    "Devil's advocate",
                    "Argue the short case with reasons and evidence.",
                ),
                role("Writer", "Synthesize the analysis, dissent and open questions."),
            ],
        },
        TeamTemplate {
            id: "custom".into(),
            name: "Custom".into(),
            description: "Blank team; add any members.".into(),
            roles: Vec::new(),
        },
    ]
}

/// Look up a built-in template by id (`"custom"` included). Unknown ids are
/// treated as custom by validation, so this is only for display/scaffolding.
pub fn builtin_template(id: &str) -> Option<TeamTemplate> {
    builtin_templates().into_iter().find(|t| t.id == id.trim())
}

/// A saved project with its team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub goal: String,
    pub folder: String,
    pub template: String,
    pub lead_agent_id: String,
    pub member_ids: Vec<String>,
}

/// Unvalidated input from the new-project / edit-team form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectDraft {
    pub name: String,
    pub goal: String,
    pub folder: String,
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub lead_agent_id: String,
    #[serde(default)]
    pub member_ids: Vec<String>,
}

/// Every reason a project cannot be saved.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid project configuration: {issues:?}")]
pub struct ProjectValidationError {
    pub issues: Vec<String>,
}

impl ProjectValidationError {
    pub fn new(issues: Vec<String>) -> Self {
        Self { issues }
    }

    pub fn explain(&self) -> String {
        self.issues.join("; ")
    }
}

/// A draft that passed [`validate_project`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedProject {
    pub name: String,
    pub goal: String,
    pub folder: String,
    pub template: String,
    pub lead_agent_id: String,
    pub member_ids: Vec<String>,
}

/// Check a draft against the form rules and the agent roster.
/// `agents` is only read for existence checks; it is not stored.
pub fn validate_project(
    draft: &ProjectDraft,
    agents: &AgentStore,
) -> Result<ValidatedProject, ProjectValidationError> {
    let mut issues: Vec<String> = Vec::new();

    let name = draft.name.trim();
    if name.is_empty() {
        issues.push("name is required: give the project a short name".into());
    } else if name.chars().count() > 80 {
        issues.push("name is too long: keep it under 80 characters".into());
    }

    let goal = draft.goal.trim();
    if goal.is_empty() {
        issues.push("goal is required: say what the team should achieve".into());
    }

    let folder = draft.folder.trim();
    if folder.is_empty() {
        issues.push("folder is required: pick a folder for project files".into());
    }

    let template = draft.template.trim();
    let template = if template.is_empty() {
        "custom".to_string()
    } else if builtin_template(template).is_some() {
        template.to_string()
    } else {
        // User-saved templates: accept any slug-like id, reject free text so
        // the row stays joinable.
        let ok = !template.chars().count().eq(&0)
            && template.chars().count() <= 64
            && template
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !ok {
            issues.push(format!(
                "unknown template \"{template}\": pick a built-in template or a saved one"
            ));
        }
        template.to_string()
    };

    let lead = draft.lead_agent_id.trim();
    if lead.is_empty() {
        issues.push("lead is required: choose the lead agent for the team".into());
    } else if agents.get(lead).is_none() {
        issues.push(format!(
            "lead agent \"{lead}\" was not found: pick a team member as lead"
        ));
    }

    let mut members: Vec<String> = Vec::new();
    for m in &draft.member_ids {
        let id = m.trim();
        if id.is_empty() {
            issues.push("member list contains a blank entry: remove it".into());
            continue;
        }
        if members.iter().any(|s| s == id) {
            issues.push(format!(
                "member \"{id}\" is listed twice: select each member once"
            ));
            continue;
        }
        if agents.get(id).is_none() {
            issues.push(format!(
                "member \"{id}\" was not found: add an existing agent to the team"
            ));
            continue;
        }
        members.push(id.to_string());
    }
    if members.is_empty() && issues.iter().all(|i| !i.starts_with("member")) {
        issues.push("team is empty: add at least one member".into());
    }
    // The lead must be on the team so @mentions and delegation reach them.
    if !lead.is_empty() && agents.get(lead).is_some() && !members.iter().any(|s| s == lead) {
        issues.push("lead must be a team member: add the lead to the members list".into());
    }

    if !issues.is_empty() {
        return Err(ProjectValidationError::new(issues));
    }
    Ok(ValidatedProject {
        name: name.to_string(),
        goal: goal.to_string(),
        folder: folder.to_string(),
        template,
        lead_agent_id: lead.to_string(),
        member_ids: members,
    })
}

/// In-memory project + team CRUD. IDs are `project_<n>`; names stay unique by
/// suffixing. Agent existence is checked against a caller-provided
/// [`AgentStore`] on every write.
#[derive(Debug, Default)]
pub struct ProjectStore {
    projects: HashMap<String, Project>,
    next_id: u64,
}

impl ProjectStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.projects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Project> {
        self.projects.get(id)
    }

    /// Workspace order: creation order (stable for the sidebar).
    pub fn list(&self) -> Vec<&Project> {
        let mut out: Vec<&Project> = self.projects.values().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn create(
        &mut self,
        draft: ProjectDraft,
        agents: &AgentStore,
    ) -> Result<Project, ProjectValidationError> {
        let v = validate_project(&draft, agents)?;
        let id = self.alloc_id();
        let project = Project {
            id,
            name: self.unique_name(&v.name, None),
            goal: v.goal,
            folder: v.folder,
            template: v.template,
            lead_agent_id: v.lead_agent_id,
            member_ids: v.member_ids,
        };
        self.projects
            .insert(project.id.clone(), project.clone());
        Ok(project)
    }

    pub fn update(
        &mut self,
        id: &str,
        draft: ProjectDraft,
        agents: &AgentStore,
    ) -> Result<Project, ProjectStoreError> {
        if !self.projects.contains_key(id) {
            return Err(ProjectStoreError::NotFound(id.into()));
        }
        let v = validate_project(&draft, agents).map_err(ProjectStoreError::Invalid)?;
        let cur = self.projects.get(id).expect("checked");
        let name = if v.name == cur.name {
            cur.name.clone()
        } else {
            self.unique_name(&v.name, Some(id))
        };
        let project = Project {
            id: id.to_string(),
            name,
            goal: v.goal,
            folder: v.folder,
            template: v.template,
            lead_agent_id: v.lead_agent_id,
            member_ids: v.member_ids,
        };
        self.projects.insert(id.to_string(), project.clone());
        Ok(project)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), ProjectStoreError> {
        self.projects
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| ProjectStoreError::NotFound(id.into()))
    }

    /// Add an agent to the team. Unknown agents and duplicates are errors so
    /// the UI can say so.
    pub fn add_member(
        &mut self,
        project_id: &str,
        agent_id: &str,
        agents: &AgentStore,
    ) -> Result<Project, ProjectStoreError> {
        let id = agent_id.trim();
        if agents.get(id).is_none() {
            return Err(ProjectStoreError::UnknownAgent(id.into()));
        }
        let p = self
            .projects
            .get_mut(project_id)
            .ok_or_else(|| ProjectStoreError::NotFound(project_id.into()))?;
        if p.member_ids.iter().any(|m| m == id) {
            return Err(ProjectStoreError::AlreadyMember(id.into()));
        }
        p.member_ids.push(id.to_string());
        Ok(p.clone())
    }

    /// Remove an agent from the team. The lead cannot be removed while they
    /// lead; change the lead first. The last member cannot be removed.
    pub fn remove_member(
        &mut self,
        project_id: &str,
        agent_id: &str,
    ) -> Result<Project, ProjectStoreError> {
        let p = self
            .projects
            .get_mut(project_id)
            .ok_or_else(|| ProjectStoreError::NotFound(project_id.into()))?;
        if !p.member_ids.iter().any(|m| m == agent_id) {
            return Err(ProjectStoreError::NotMember(agent_id.into()));
        }
        if p.lead_agent_id == agent_id {
            return Err(ProjectStoreError::LeadRemoval(
                "change the lead before removing them from the team".into(),
            ));
        }
        if p.member_ids.len() <= 1 {
            return Err(ProjectStoreError::EmptyTeam(
                "the team needs at least one member".into(),
            ));
        }
        p.member_ids.retain(|m| m != agent_id);
        Ok(p.clone())
    }

    /// Choose a new lead; they must already be on the team.
    pub fn set_lead(
        &mut self,
        project_id: &str,
        agent_id: &str,
    ) -> Result<Project, ProjectStoreError> {
        let p = self
            .projects
            .get_mut(project_id)
            .ok_or_else(|| ProjectStoreError::NotFound(project_id.into()))?;
        if !p.member_ids.iter().any(|m| m == agent_id) {
            return Err(ProjectStoreError::NotMember(agent_id.into()));
        }
        p.lead_agent_id = agent_id.to_string();
        Ok(p.clone())
    }

    fn alloc_id(&mut self) -> String {
        self.next_id += 1;
        format!("project_{}", self.next_id)
    }

    fn unique_name(&self, want: &str, exclude_id: Option<&str>) -> String {
        let taken = |n: &str| {
            self.projects
                .values()
                .any(|p| p.name == n && Some(p.id.as_str()) != exclude_id)
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
pub enum ProjectStoreError {
    #[error("invalid project configuration: {0}")]
    Invalid(ProjectValidationError),
    #[error("project not found: {0}")]
    NotFound(String),
    #[error("agent not found: {0}")]
    UnknownAgent(String),
    #[error("agent is already on the team: {0}")]
    AlreadyMember(String),
    #[error("agent is not on the team: {0}")]
    NotMember(String),
    #[error("{0}")]
    LeadRemoval(String),
    #[error("{0}")]
    EmptyTeam(String),
}

#[cfg(test)]
mod projects_tests {
    use super::*;
    use crate::agents::api_draft;

    fn agents_with(names: &[&str]) -> (AgentStore, Vec<String>) {
        let mut store = AgentStore::new();
        let mut ids = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let d = api_draft(n, "researcher", "Be rigorous.", "openai", "gpt-4o");
            let a = store.create(d).expect("agent saves");
            let _ = i;
            ids.push(a.id.clone());
        }
        (store, ids)
    }

    fn draft_for(ids: &[String]) -> ProjectDraft {
        ProjectDraft {
            name: "Market study".into(),
            goal: "Research the market.".into(),
            folder: "/tmp/market-study".into(),
            template: "research-squad".into(),
            lead_agent_id: ids[0].clone(),
            member_ids: ids.to_vec(),
        }
    }

    #[test]
    fn projects_create_with_template_lead_and_members() {
        let (agents, ids) = agents_with(&["Ada", "Bo", "Cy"]);
        let mut store = ProjectStore::new();
        let p = store.create(draft_for(&ids), &agents).expect("valid saves");
        assert!(p.id.starts_with("project_"));
        assert_eq!(p.template, "research-squad");
        assert_eq!(p.lead_agent_id, ids[0]);
        assert_eq!(p.member_ids, ids);
        assert_eq!(store.list().len(), 1);
        // Workspace shows it.
        assert_eq!(store.get(&p.id).unwrap().name, "Market study");
    }

    #[test]
    fn projects_create_rejects_incomplete_form_with_explanation() {
        let (agents, _) = agents_with(&["Ada"]);
        let mut store = ProjectStore::new();
        let draft = ProjectDraft::default();
        let err = store.create(draft, &agents).expect_err("must not save");
        let text = err.explain();
        for want in ["name", "goal", "folder", "lead", "team"] {
            assert!(text.contains(want), "explanation must mention {want}: {text}");
        }
        assert!(store.is_empty());
    }

    #[test]
    fn projects_create_rejects_unknown_lead_members_and_template() {
        let (agents, ids) = agents_with(&["Ada"]);
        let mut store = ProjectStore::new();
        let draft = ProjectDraft {
            name: "X".into(),
            goal: "G".into(),
            folder: "/tmp/x".into(),
            template: "not a template!!".into(),
            lead_agent_id: "agent_999".into(),
            member_ids: vec!["agent_999".into(), "  ".into()],
        };
        let err = store.create(draft, &agents).expect_err("bad team must not save");
        let text = err.explain();
        assert!(text.contains("template"), "got: {text}");
        assert!(text.contains("agent_999"), "got: {text}");

        // Lead must be on the team.
        let draft = ProjectDraft {
            name: "X".into(),
            goal: "G".into(),
            folder: "/tmp/x".into(),
            template: String::new(),
            lead_agent_id: ids[0].clone(),
            member_ids: Vec::new(),
        };
        let err = store.create(draft, &agents).expect_err("empty team must not save");
        assert!(err.explain().contains("team"), "got: {}", err.explain());
    }

    #[test]
    fn projects_builtin_templates_cover_expected_teams() {
        let templates = builtin_templates();
        let ids: Vec<&str> = templates.iter().map(|t| t.id.as_str()).collect();
        for want in ["research-squad", "product-brainstorm", "investment-analysis", "custom"] {
            assert!(ids.contains(&want), "missing template {want}: {ids:?}");
        }
        let squad = builtin_template("research-squad").expect("squad exists");
        assert!(squad.roles.len() >= 5);
        assert!(builtin_template("custom").unwrap().roles.is_empty());
        // Blank template id falls back to custom.
        let (agents, ids) = agents_with(&["Ada"]);
        let mut draft = draft_for(&ids);
        draft.template = String::new();
        let v = validate_project(&draft, &agents).expect("blank template is custom");
        assert_eq!(v.template, "custom");
    }

    #[test]
    fn projects_team_membership_add_remove_and_lead() {
        let (agents, ids) = agents_with(&["Ada", "Bo", "Cy"]);
        let mut store = ProjectStore::new();
        let p = store
            .create(draft_for(&ids[..2]), &agents)
            .expect("valid saves");
        // Add a member; duplicates and unknown agents fail.
        let p = store.add_member(&p.id, &ids[2], &agents).expect("add");
        assert_eq!(p.member_ids.len(), 3);
        assert!(matches!(
            store.add_member(&p.id, &ids[2], &agents),
            Err(ProjectStoreError::AlreadyMember(_))
        ));
        assert!(matches!(
            store.add_member(&p.id, "agent_999", &agents),
            Err(ProjectStoreError::UnknownAgent(_))
        ));
        // Change lead to another member; outsiders fail.
        let p = store.set_lead(&p.id, &ids[1]).expect("set lead");
        assert_eq!(p.lead_agent_id, ids[1]);
        assert!(matches!(
            store.set_lead(&p.id, "agent_999"),
            Err(ProjectStoreError::NotMember(_))
        ));
        // Lead cannot be removed while they lead; others can.
        assert!(matches!(
            store.remove_member(&p.id, &ids[1]),
            Err(ProjectStoreError::LeadRemoval(_))
        ));
        let p = store.remove_member(&p.id, &ids[2]).expect("remove");
        assert_eq!(p.member_ids.len(), 2);
    }

    #[test]
    fn projects_update_rename_delete_persist_in_store() {
        let (agents, ids) = agents_with(&["Ada", "Bo"]);
        let mut store = ProjectStore::new();
        let a = store.create(draft_for(&ids), &agents).unwrap();
        // Same name on a second project is suffixed so the workspace stays clear.
        let b = store.create(draft_for(&ids), &agents).unwrap();
        assert_ne!(a.name, b.name);

        let mut draft = draft_for(&ids);
        draft.goal = "Updated goal.".into();
        let u = store.update(&a.id, draft, &agents).expect("update");
        assert_eq!(u.goal, "Updated goal.");

        let bad = ProjectDraft::default();
        assert!(store.update(&a.id, bad, &agents).is_err());
        assert_eq!(store.get(&a.id).unwrap().goal, "Updated goal.");

        store.delete(&b.id).expect("delete");
        assert!(store.get(&b.id).is_none());
        assert!(matches!(
            store.delete(&b.id),
            Err(ProjectStoreError::NotFound(_))
        ));
    }

    #[test]
    fn projects_serialize_for_persistence() {
        let (agents, ids) = agents_with(&["Ada"]);
        let mut store = ProjectStore::new();
        let p = store.create(draft_for(&ids), &agents).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        // Drafts also round-trip (form state survives reloads).
        let draft = draft_for(&ids);
        let json = serde_json::to_string(&draft).unwrap();
        let back: ProjectDraft = serde_json::from_str(&json).unwrap();
        assert_eq!(back, draft);
    }
}
