//! Skill creator + learned skills (see docs/PRD.md §5.2).
//!
//! - The built-in `skill-creator` skill helps the user (or an agent, with
//!   approval) write a new skill in chat.
//! - [`SkillCreatorDraft`] is the preview-editor model: validated live and
//!   saved as a normal skill folder ([`skill_folder_files`]) only explicitly.
//! - After a run the lead may propose "save this as a skill"; the proposal
//!   sits in [`LearnedSkillQueue`] for user review and is installed into the
//!   [`SkillStore`] only via [`LearnedSkillQueue::approve`]. Never saved silently.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::skills::{validate_skill_name, Skill, SkillInput, SkillScope, SkillStore, SkillsError};

/// Name of the built-in skill-creator skill.
pub const SKILL_CREATOR_NAME: &str = "skill-creator";

/// Built-in `skill-creator` skill input (starter skill, offline).
pub fn skill_creator_input() -> SkillInput {
    let mut input = SkillInput::new(
        SKILL_CREATOR_NAME,
        "Helps you write a new skill: name, description, SKILL.md body and supporting files.",
        "# skill-creator\n\nGuide the user (or proposing agent) through writing a new skill:\n\n1. Pick a lowercase hyphenated `name` and one-line `description` (what the skill does and when to use it).\n2. Draft the SKILL.md `body`: concise instructions an agent can follow, with steps and examples.\n3. Add supporting `files` under `references/`, `scripts/` or `assets/` when needed.\n4. Preview with the editor, fix validation issues, then ask for approval to save.\n\nRules: names are lowercase alphanumeric/hyphen (max 64 chars); description is required (max 1024 chars); body must be non-empty; file paths must be relative inside the skill folder. Scripts run only through the shell tool and its permission rules (ask by default).\n",
        SkillScope::Global,
    );
    input.license = "MIT".into();
    input.source = "synapse-builtin".into();
    input
}

// ---------------------------------------------------------------------------
// Creator draft (preview editor model)
// ---------------------------------------------------------------------------

/// Editable draft for a new skill, as held by the preview editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCreatorDraft {
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
}

impl SkillCreatorDraft {
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
            source: "skill-creator".into(),
        }
    }

    pub fn with_file(mut self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.files.insert(path.into(), content.into());
        self
    }

    pub fn set_body(&mut self, body: impl Into<String>) {
        self.body = body.into();
    }

    /// Live validation for the preview editor; empty = ready to save.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        let name = self.name.trim();
        if let Err(e) = validate_skill_name(name) {
            issues.push(format!("name: {e}"));
        }
        let desc = self.description.trim();
        if desc.is_empty() {
            issues.push("description is required".into());
        } else if desc.len() > 1024 {
            issues.push(format!(
                "description too long ({} chars, max 1024)",
                desc.len()
            ));
        }
        if self.body.trim().is_empty() {
            issues.push("SKILL.md body is empty".into());
        }
        for path in self.files.keys() {
            if let Err(e) = validate_draft_path(path) {
                issues.push(format!("file: {e}"));
            }
        }
        issues
    }

    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }

    /// Rendered preview of the skill folder's `SKILL.md` (frontmatter + body).
    pub fn preview_markdown(&self) -> String {
        render_skill_markdown(&self.name, &self.description, &self.body)
    }

    /// Full folder contents: `SKILL.md` plus bundled files.
    pub fn folder_files(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        out.insert(
            "SKILL.md".to_string(),
            render_skill_markdown(&self.name, &self.description, &self.body),
        );
        for (k, v) in &self.files {
            out.insert(k.clone(), v.clone());
        }
        out
    }

    pub fn to_skill_input(&self) -> SkillInput {
        SkillInput {
            name: self.name.trim().to_string(),
            description: self.description.trim().to_string(),
            body: self.body.trim().to_string(),
            files: self.files.clone(),
            scope: self.scope.clone(),
            license: self.license.clone(),
            source: if self.source.is_empty() {
                "skill-creator".into()
            } else {
                self.source.clone()
            },
        }
    }
}

fn validate_draft_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path == "SKILL.md"
    {
        return Err(format!(
            "rejected skill file path {path:?}: must be a relative path inside the skill folder"
        ));
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(format!(
                "rejected skill file path {path:?}: must be a relative path inside the skill folder"
            ));
        }
    }
    Ok(())
}

/// Render `SKILL.md` with YAML frontmatter (name + description) + body.
pub fn render_skill_markdown(name: &str, description: &str, body: &str) -> String {
    format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}\n",
        name.trim(),
        description.trim().replace('\n', " "),
        body.trim()
    )
}

/// Normal skill folder contents for an installed [`Skill`].
pub fn skill_folder_files(skill: &Skill) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    out.insert(
        "SKILL.md".to_string(),
        render_skill_markdown(&skill.name, &skill.description, &skill.body),
    );
    for (k, v) in &skill.files {
        out.insert(k.clone(), v.clone());
    }
    out
}

// ---------------------------------------------------------------------------
// Learned-skill proposals (approval-gated)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSkillStatus {
    Pending,
    Approved,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSkillProposal {
    pub id: String,
    pub run_id: String,
    pub proposed_by: String,
    pub reason: String,
    pub draft: SkillCreatorDraft,
    pub status: LearnedSkillStatus,
}

impl LearnedSkillProposal {
    pub fn approve(&mut self) {
        self.status = LearnedSkillStatus::Approved;
    }
    pub fn deny(&mut self) {
        self.status = LearnedSkillStatus::Denied;
    }
    pub fn is_pending(&self) -> bool {
        self.status == LearnedSkillStatus::Pending
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LearnedSkillError {
    #[error("learned skill proposal not found: {0}")]
    NotFound(String),
    #[error("learned skill proposal {0} is not pending")]
    NotPending(String),
    #[error("invalid draft: {0}")]
    InvalidDraft(String),
    #[error(transparent)]
    Skills(#[from] SkillsError),
}

static LEARNED_SKILL_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Propose "save this as a skill" after a run. Always starts `Pending`;
/// nothing is installed until [`LearnedSkillQueue::approve`] runs.
pub fn propose_learned_skill(
    run_id: &str,
    proposed_by: &str,
    draft: SkillCreatorDraft,
    reason: &str,
) -> LearnedSkillProposal {
    let id = format!(
        "learned_{}",
        LEARNED_SKILL_CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    LearnedSkillProposal {
        id,
        run_id: run_id.into(),
        proposed_by: proposed_by.into(),
        reason: reason.into(),
        draft,
        status: LearnedSkillStatus::Pending,
    }
}

/// Review queue for proposed learned skills. Proposals are shown for user
/// review; approving installs them as normal skill folders.
#[derive(Debug, Default)]
pub struct LearnedSkillQueue {
    proposals: BTreeMap<String, LearnedSkillProposal>,
}

impl LearnedSkillQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.proposals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.proposals.is_empty()
    }

    /// Submit a proposal for review. Draft validity is checked at approval
    /// time so the user can still edit an incomplete draft after proposing.
    pub fn submit(&mut self, proposal: LearnedSkillProposal) {
        self.proposals.insert(proposal.id.clone(), proposal);
    }

    pub fn get(&self, id: &str) -> Option<&LearnedSkillProposal> {
        self.proposals.get(id)
    }

    pub fn pending(&self) -> Vec<LearnedSkillProposal> {
        self.proposals
            .values()
            .filter(|p| p.is_pending())
            .cloned()
            .collect()
    }

    pub fn pending_for_run(&self, run_id: &str) -> Vec<LearnedSkillProposal> {
        self.proposals
            .values()
            .filter(|p| p.is_pending() && p.run_id == run_id)
            .cloned()
            .collect()
    }

    /// Replace the draft of a pending proposal (edit in the preview editor).
    pub fn edit_draft(
        &mut self,
        id: &str,
        draft: SkillCreatorDraft,
    ) -> Result<(), LearnedSkillError> {
        let p = self
            .proposals
            .get_mut(id)
            .ok_or_else(|| LearnedSkillError::NotFound(id.into()))?;
        if !p.is_pending() {
            return Err(LearnedSkillError::NotPending(id.into()));
        }
        p.draft = draft;
        Ok(())
    }

    /// Approve: validate the draft, then save it as a normal skill folder
    /// (installed into the [`SkillStore`]). Only this path installs.
    pub fn approve(
        &mut self,
        id: &str,
        store: &mut SkillStore,
    ) -> Result<Skill, LearnedSkillError> {
        let proposal = self
            .proposals
            .get_mut(id)
            .ok_or_else(|| LearnedSkillError::NotFound(id.into()))?;
        if !proposal.is_pending() {
            return Err(LearnedSkillError::NotPending(id.into()));
        }
        let issues = proposal.draft.validate();
        if !issues.is_empty() {
            return Err(LearnedSkillError::InvalidDraft(issues.join("; ")));
        }
        let skill = store.install(proposal.draft.to_skill_input())?;
        proposal.approve();
        Ok(skill)
    }

    /// Deny: the draft is never saved.
    pub fn deny(&mut self, id: &str) -> Result<(), LearnedSkillError> {
        let proposal = self
            .proposals
            .get_mut(id)
            .ok_or_else(|| LearnedSkillError::NotFound(id.into()))?;
        if !proposal.is_pending() {
            return Err(LearnedSkillError::NotPending(id.into()));
        }
        proposal.deny();
        Ok(())
    }
}

#[cfg(test)]
mod learned_skills_tests {
    use super::*;

    fn creator_draft(name: &str) -> SkillCreatorDraft {
        SkillCreatorDraft::new(
            name,
            format!("Use {name} after a good run."),
            format!("# {name}\n\nFollow these steps.\n"),
            SkillScope::Global,
        )
        .with_file("references/notes.md", "helper notes")
    }

    #[test]
    fn learned_skills_creator_draft_preview_and_validation() {
        let draft = creator_draft("postmortem-notes");
        assert!(draft.is_valid(), "{:?}", draft.validate());
        let md = draft.preview_markdown();
        assert!(md.contains("name: postmortem-notes"));
        assert!(md.contains("Follow these steps"));
        let files = draft.folder_files();
        assert!(files.contains_key("SKILL.md"));
        assert_eq!(files["references/notes.md"], "helper notes");

        let mut bad = SkillCreatorDraft::new("Bad_Name", "  ", "  ", SkillScope::Global);
        bad.files.insert("../evil.md".to_string(), "x".to_string());
        let issues = bad.validate();
        assert!(issues.iter().any(|i| i.contains("name")));
        assert!(issues.iter().any(|i| i.contains("description")));
        assert!(issues.iter().any(|i| i.contains("body")));
        assert!(issues.iter().any(|i| i.contains("rejected")));
        assert!(!bad.is_valid());
    }

    #[test]
    fn learned_skills_builtin_creator_installs_as_normal_skill() {
        let mut store = SkillStore::new();
        let skill = store.install(skill_creator_input()).unwrap();
        assert_eq!(skill.name, SKILL_CREATOR_NAME);
        assert!(store
            .load_skill(None, "any-agent", SKILL_CREATOR_NAME)
            .is_ok());
        // Saved as a normal skill folder.
        let files = skill_folder_files(&skill);
        assert!(files["SKILL.md"].contains("skill-creator"));
    }

    #[test]
    fn learned_skills_proposal_needs_approval_before_save() {
        let mut store = SkillStore::new();
        let mut queue = LearnedSkillQueue::new();
        let proposal = propose_learned_skill(
            "run-1",
            "lead",
            creator_draft("research-playbook"),
            "This research procedure worked well; save it.",
        );
        assert_eq!(proposal.status, LearnedSkillStatus::Pending);
        let id = proposal.id.clone();
        queue.submit(proposal);
        // Proposing alone never installs the skill.
        assert!(store.load_skill(None, "a1", "research-playbook").is_err());
        assert_eq!(queue.pending().len(), 1);
        assert_eq!(queue.pending_for_run("run-1").len(), 1);
        assert!(queue.pending_for_run("other-run").is_empty());
        // Approval saves it as a normal skill folder.
        let skill = queue.approve(&id, &mut store).unwrap();
        assert_eq!(skill.name, "research-playbook");
        assert_eq!(queue.get(&id).unwrap().status, LearnedSkillStatus::Approved);
        assert!(store.load_skill(None, "a1", "research-playbook").is_ok());
        let files = skill_folder_files(&skill);
        assert!(files.contains_key("SKILL.md"));
        // Second approve is rejected.
        assert!(matches!(
            queue.approve(&id, &mut store),
            Err(LearnedSkillError::NotPending(_))
        ));
    }

    #[test]
    fn learned_skills_denied_or_invalid_proposals_never_saved() {
        let mut store = SkillStore::new();
        let mut queue = LearnedSkillQueue::new();
        let denied = propose_learned_skill("run-2", "lead", creator_draft(" denied-x"), "x");
        let denied_id = denied.id.clone();
        queue.submit(denied);
        queue.deny(&denied_id).unwrap();
        assert_eq!(
            queue.get(&denied_id).unwrap().status,
            LearnedSkillStatus::Denied
        );
        assert!(queue.pending().is_empty());
        assert!(store.load_skill(None, "a1", "denied-x").is_err());

        // Invalid drafts cannot be approved; editing fixes them.
        let bad = SkillCreatorDraft::new("ok-name", "", "", SkillScope::Global);
        let bad_p = propose_learned_skill("run-3", "lead", bad, "incomplete draft");
        let bad_id = bad_p.id.clone();
        queue.submit(bad_p);
        assert!(matches!(
            queue.approve(&bad_id, &mut store),
            Err(LearnedSkillError::InvalidDraft(_))
        ));
        assert!(store.load_skill(None, "a1", "ok-name").is_err());
        queue.edit_draft(&bad_id, creator_draft("ok-name")).unwrap();
        queue.approve(&bad_id, &mut store).unwrap();
        assert!(store.load_skill(None, "a1", "ok-name").is_ok());
        // Unknown ids are errors.
        assert!(matches!(
            queue.approve("missing", &mut store),
            Err(LearnedSkillError::NotFound(_))
        ));
    }
}
