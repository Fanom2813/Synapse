//! Agent Builder UI state (see docs/PRD.md §5.2).
//! Pure state for the wizard: describe → clarify (≤3, skippable) → research →
//! review (sources, skills/backend, avatar) → test chat → save/export.
//! Approval cards for `propose_agent` and refresh diff are here too.

use std::collections::HashMap;

use synapse_core::agent_builder::{
    generate_clarify_questions, mock_sources_for, refresh_knowledge, validate_guardrails,
    AgentProposal, BuilderSession, ClarifyQuestion, DraftAgent, KnowledgeDiff, ProposalStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuilderStep {
    Describe,
    Clarify,
    Researching,
    Review,
    TestChat,
    Saved,
}

#[derive(Debug, Clone)]
pub struct BuilderUiState {
    pub description: String,
    pub step: BuilderStep,
    pub session: Option<BuilderSession>,
    pub questions: Vec<ClarifyQuestion>,
    pub answers: HashMap<String, String>,
    pub draft: Option<DraftAgent>,
    pub edit_name: String,
    pub edit_role: String,
    pub edit_instructions: String,
    pub test_input: String,
    pub test_log: Vec<(String, String)>,
    pub proposals: Vec<AgentProposal>,
    pub last_diff: Option<KnowledgeDiff>,
    pub error: Option<String>,
    pub saved_agent_id: Option<String>,
}

impl Default for BuilderUiState {
    fn default() -> Self {
        Self {
            description: String::new(),
            step: BuilderStep::Describe,
            session: None,
            questions: Vec::new(),
            answers: HashMap::new(),
            draft: None,
            edit_name: String::new(),
            edit_role: String::new(),
            edit_instructions: String::new(),
            test_input: String::new(),
            test_log: Vec::new(),
            proposals: Vec::new(),
            last_diff: None,
            error: None,
            saved_agent_id: None,
        }
    }
}

impl BuilderUiState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_description(&mut self, desc: &str) {
        self.description = desc.trim().to_string();
        self.error = None;
    }

    pub fn can_start(&self) -> bool {
        !self.description.is_empty()
    }

    pub fn start(&mut self) -> bool {
        if !self.can_start() {
            self.error = Some("Describe the agent first: what should it know and do?".into());
            return false;
        }
        let qs = generate_clarify_questions(&self.description);
        self.questions = qs;
        let mut session = BuilderSession::new(&self.description);
        // Keep session questions in sync
        session.questions = self.questions.clone();
        self.session = Some(session);
        self.step = BuilderStep::Clarify;
        self.error = None;
        true
    }

    pub fn answer_question(&mut self, id: &str, answer: &str) {
        self.answers.insert(id.into(), answer.into());
        if let Some(s) = &mut self.session {
            s.answer(id, answer);
        }
    }

    pub fn skip_clarify(&mut self) {
        if let Some(s) = &mut self.session {
            s.skip_clarify();
        }
        self.research();
    }

    pub fn continue_from_clarify(&mut self) {
        // Answers are optional; proceed to research
        self.research();
    }

    fn research(&mut self) {
        self.step = BuilderStep::Researching;
        if let Some(s) = &mut self.session {
            // Ensure answers from UI are in session
            for (k, v) in &self.answers {
                s.answer(k, v);
            }
            s.run_research();
            if let Some(d) = s.draft.clone() {
                self.edit_name = d.name.clone();
                self.edit_role = d.role.clone();
                self.edit_instructions = d.instructions.clone();
                self.draft = Some(d);
            }
        }
        self.step = BuilderStep::Review;
    }

    pub fn guardrail_issues(&self) -> Vec<String> {
        if let Some(d) = &self.draft {
            validate_guardrails(d)
        } else {
            Vec::new()
        }
    }

    pub fn apply_edits(&mut self) {
        if let Some(d) = &mut self.draft {
            if !self.edit_name.trim().is_empty() {
                d.name = self.edit_name.trim().to_string();
            }
            if !self.edit_role.trim().is_empty() {
                d.role = self.edit_role.trim().to_string();
            }
            if !self.edit_instructions.trim().is_empty() {
                d.instructions = self.edit_instructions.trim().to_string();
                d.persona = d.instructions.clone();
            }
            if let Some(s) = &mut self.session {
                s.draft = Some(d.clone());
            }
        }
    }

    pub fn open_test_chat(&mut self) {
        self.apply_edits();
        self.step = BuilderStep::TestChat;
    }

    pub fn send_test_message(&mut self, text: &str) {
        let t = text.trim();
        if t.is_empty() {
            return;
        }
        self.test_log.push(("user".into(), t.into()));
        let reply = if let Some(d) = &self.draft {
            format!("{} ({} — AI perspective, cited) replies: you said \"{t}\"", d.name, d.role)
        } else {
            format!("(no draft) you said \"{t}\"")
        };
        self.test_log.push(("agent".into(), reply));
        self.test_input.clear();
        if let Some(s) = &mut self.session {
            // also push into session for parity
            let _ = s.test_chat_reply(t);
        }
    }

    pub fn save(&mut self, store: &mut synapse_core::AgentStore) -> Result<String, String> {
        self.apply_edits();
        let draft = self.draft.clone().ok_or_else(|| "no draft to save".to_string())?;
        let issues = validate_guardrails(&draft);
        if !issues.is_empty() {
            return Err(issues.join("; "));
        }
        let agent_draft = synapse_core::AgentDraft {
            name: draft.name.clone(),
            role: draft.role.clone(),
            instructions: draft.instructions.clone(),
            backend: synapse_core::AgentBackend::Api(synapse_core::ApiBackend {
                provider: "anthropic".into(),
                model: draft.model_suggestion.clone(),
                temperature: None,
                max_tokens: None,
                thinking: synapse_core::ThinkingLevel::Off,
            }),
            tools: draft.suggested_tools.clone(),
            avatar: draft.avatar_spec.clone(),
            persona_source: "agent-builder".into(),
        };
        let agent = store.create(agent_draft).map_err(|e| e.explain())?;
        self.saved_agent_id = Some(agent.id.clone());
        self.step = BuilderStep::Saved;
        Ok(agent.id)
    }

    pub fn export_markdown(&self) -> Option<String> {
        self.draft.as_ref().map(|d| d.export_markdown())
    }

    pub fn refresh_knowledge(&mut self) {
        if let Some(s) = &mut self.session {
            let new_sources = mock_sources_for(&format!("{} refreshed", self.description));
            // add a new source to ensure diff
            let extra = synapse_core::agent_builder::SourceRef {
                id: "s-refresh".into(),
                url: "https://example.com/refreshed".into(),
                title: "Refreshed source".into(),
                quote: "Updated finding after refresh.".into(),
            };
            let diff = s.refresh(vec![extra].into_iter().chain(new_sources.into_iter().take(1)).collect());
            if let Some(d) = diff {
                self.last_diff = Some(d);
                self.draft = s.draft.clone();
                if let Some(d) = &self.draft {
                    self.edit_name = d.name.clone();
                }
            }
        } else if let Some(d) = &self.draft {
            let extra = synapse_core::agent_builder::SourceRef {
                id: "s-refresh".into(),
                url: "https://example.com/refreshed".into(),
                title: "Refreshed source".into(),
                quote: "Updated finding after refresh.".into(),
            };
            let (new_draft, diff) = refresh_knowledge(d, vec![extra]);
            self.last_diff = Some(diff);
            self.draft = Some(new_draft);
        }
    }

    // -- proposals (approval-gated) --

    pub fn add_proposal(&mut self, proposal: AgentProposal) {
        self.proposals.push(proposal);
    }

    pub fn approve_proposal(&mut self, id: &str, store: &mut synapse_core::AgentStore) -> bool {
        if let Some(p) = self.proposals.iter_mut().find(|p| p.id == id) {
            if p.status != ProposalStatus::Pending {
                return false;
            }
            p.approve();
            let d = synapse_core::AgentDraft {
                name: p.draft.name.clone(),
                role: p.draft.role.clone(),
                instructions: p.draft.instructions.clone(),
                backend: synapse_core::AgentBackend::Api(synapse_core::ApiBackend {
                    provider: "anthropic".into(),
                    model: p.draft.model_suggestion.clone(),
                    temperature: None,
                    max_tokens: None,
                    thinking: synapse_core::ThinkingLevel::Off,
                }),
                tools: p.draft.suggested_tools.clone(),
                avatar: p.draft.avatar_spec.clone(),
                persona_source: "proposal".into(),
            };
            let _ = store.create(d);
            return true;
        }
        false
    }

    pub fn deny_proposal(&mut self, id: &str) -> bool {
        if let Some(p) = self.proposals.iter_mut().find(|p| p.id == id) {
            if p.status != ProposalStatus::Pending {
                return false;
            }
            p.deny();
            return true;
        }
        false
    }

    pub fn pending_proposals(&self) -> Vec<&AgentProposal> {
        self.proposals.iter().filter(|p| p.status == ProposalStatus::Pending).collect()
    }
}

#[cfg(test)]
mod agent_builder_ui_tests {
    use super::*;
    use synapse_core::agent_builder::{propose_agent, ProposalStatus};

    #[test]
    fn agent_builder_ui_describe_clarify_skip_research_flow() {
        let mut ui = BuilderUiState::new();
        assert!(!ui.can_start());
        assert!(!ui.start());
        assert!(ui.error.is_some());
        ui.set_description("a Ugandan agent who understands how people in Uganda think, talk and decide");
        assert!(ui.can_start());
        assert!(ui.start());
        assert_eq!(ui.step, BuilderStep::Clarify);
        assert!(ui.questions.len() <= 3 && !ui.questions.is_empty());
        // Answer some, then skip remaining (skippable)
        ui.answer_question("region", "Buganda");
        ui.skip_clarify();
        assert_eq!(ui.step, BuilderStep::Review);
        assert!(ui.draft.is_some());
        let d = ui.draft.as_ref().unwrap();
        assert!(!d.sources.is_empty());
        assert!(d.knowledge.facts.iter().all(|f| !f.citations.is_empty()));
        assert!(ui.guardrail_issues().is_empty());
    }

    #[test]
    fn agent_builder_ui_review_edit_test_chat_save_and_export() {
        let mut ui = BuilderUiState::new();
        ui.set_description("Uganda market research agent");
        ui.start();
        ui.continue_from_clarify();
        assert_eq!(ui.step, BuilderStep::Review);
        // Edit
        ui.edit_name = "Kampala Insight".into();
        ui.edit_role = "Market analyst".into();
        ui.apply_edits();
        assert_eq!(ui.draft.as_ref().unwrap().name, "Kampala Insight");
        // Export contains sources and skills
        let md = ui.export_markdown().unwrap();
        assert!(md.contains("Kampala Insight"));
        assert!(md.contains("Sources"));
        // Test chat before save
        ui.open_test_chat();
        assert_eq!(ui.step, BuilderStep::TestChat);
        ui.send_test_message("What drives decisions in rural Ankole?");
        assert_eq!(ui.test_log.len(), 2);
        assert!(ui.test_log[1].1.contains("Kampala Insight"));
        // Save
        let mut store = synapse_core::AgentStore::new();
        let id = ui.save(&mut store).expect("save ok");
        assert_eq!(ui.step, BuilderStep::Saved);
        assert_eq!(ui.saved_agent_id.as_deref(), Some(id.as_str()));
        assert_eq!(store.len(), 1);
        assert!(store.get(&id).unwrap().instructions.contains("AI perspective"));
    }

    #[test]
    fn agent_builder_ui_refresh_knowledge_shows_diff() {
        let mut ui = BuilderUiState::new();
        ui.set_description("Uganda culture agent");
        ui.start();
        ui.continue_from_clarify();
        let before = ui.draft.as_ref().unwrap().sources.len();
        ui.refresh_knowledge();
        assert!(ui.last_diff.is_some());
        let diff = ui.last_diff.as_ref().unwrap();
        assert!(diff.markdown_changed);
        assert!(ui.draft.as_ref().unwrap().sources.len() >= before);
    }

    #[test]
    fn agent_builder_ui_proposal_approval_gated_never_auto_adds() {
        let mut ui = BuilderUiState::new();
        let mut store = synapse_core::AgentStore::new();
        let p = propose_agent("we need someone who knows Ugandan mobile-money regulation", false);
        assert_eq!(p.status, ProposalStatus::Pending);
        let pid = p.id.clone();
        ui.add_proposal(p);
        assert_eq!(ui.pending_proposals().len(), 1);
        // Not in store until approved
        assert_eq!(store.len(), 0);
        assert!(ui.approve_proposal(&pid, &mut store));
        assert_eq!(store.len(), 1);
        assert!(ui.pending_proposals().is_empty());
        // Deny path
        let p2 = propose_agent("second proposal", false);
        let pid2 = p2.id.clone();
        ui.add_proposal(p2);
        assert!(ui.deny_proposal(&pid2));
        assert_eq!(ui.proposals.iter().find(|p| p.id == pid2).unwrap().status, ProposalStatus::Denied);
        assert_eq!(store.len(), 1); // not added
    }

    #[test]
    fn agent_builder_ui_build_team_proposals_and_empty_states() {
        let mut ui = BuilderUiState::new();
        // Empty draft shows no guardrail noise before research
        assert!(ui.guardrail_issues().is_empty());
        assert!(ui.export_markdown().is_none());
        ui.set_description("research Ugandan fintech");
        ui.start();
        ui.continue_from_clarify();
        // Build team for goal (3 proposals)
        for p in synapse_core::agent_builder::build_team_for_goal("research Ugandan fintech", false) {
            ui.add_proposal(p);
        }
        assert_eq!(ui.proposals.len(), 3);
        // Empty test message does nothing
        let before = ui.test_log.len();
        ui.send_test_message("   ");
        assert_eq!(ui.test_log.len(), before);
    }
}
