//! Agent Builder: describe → clarify → research → draft → review/test → save/export
//! (see docs/PRD.md §5.2). Guardrails for culture/people agents, refresh and
//! approval-gated proposals.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Clarify
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClarifyQuestion {
    pub id: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub skippable: bool,
}

/// Generate ≤3 clarifying questions from the description. Deterministic and
/// skippable. Covers region/community, language, purpose.
pub fn generate_clarify_questions(description: &str) -> Vec<ClarifyQuestion> {
    let lower = description.to_lowercase();
    let mut out = Vec::new();
    let is_culture = lower.contains("uganda")
        || lower.contains("culture")
        || lower.contains("people")
        || lower.contains("japan")
        || lower.contains("kenya")
        || lower.contains("community");

    if is_culture {
        out.push(ClarifyQuestion {
            id: "region".into(),
            question: "Which region or community should the agent focus on?".into(),
            options: vec![
                "Nationwide".into(),
                "Buganda".into(),
                "Acholi".into(),
                "Urban (e.g. Kampala)".into(),
                "Rural".into(),
            ],
            skippable: true,
        });
        out.push(ClarifyQuestion {
            id: "language".into(),
            question: "Which languages should it understand?".into(),
            options: vec![
                "English".into(),
                "Luganda".into(),
                "Swahili".into(),
                "Acholi".into(),
            ],
            skippable: true,
        });
    }
    // Purpose question always if not already 3
    if out.len() < 3 {
        out.push(ClarifyQuestion {
            id: "purpose".into(),
            question: "What will the agent be used for?".into(),
            options: vec![
                "Market research".into(),
                "Writing & translation".into(),
                "Advising & coaching".into(),
                "General assistant".into(),
            ],
            skippable: true,
        });
    }
    out.truncate(3);
    out
}

// ---------------------------------------------------------------------------
// Sources & knowledge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub id: String,
    pub url: String,
    pub title: String,
    pub quote: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitedFact {
    pub fact: String,
    pub citations: Vec<String>, // source ids
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgePack {
    pub markdown: String,
    pub facts: Vec<CitedFact>,
}

// ---------------------------------------------------------------------------
// Draft
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftAgent {
    pub name: String,
    pub role: String,
    pub persona: String,
    pub instructions: String,
    pub knowledge: KnowledgePack,
    pub sources: Vec<SourceRef>,
    pub suggested_skills: Vec<String>,
    pub suggested_tools: Vec<String>,
    pub model_suggestion: String,
    pub avatar_spec: String,
}

impl DraftAgent {
    pub fn export_markdown(&self) -> String {
        let mut md = format!("# {} — {}\n\n", self.name, self.role);
        md.push_str(&self.persona);
        md.push_str("\n\n## Knowledge\n\n");
        md.push_str(&self.knowledge.markdown);
        md.push_str("\n\n## Sources\n\n");
        for s in &self.sources {
            md.push_str(&format!("- [{}]({}) — \"{}\"\n", s.title, s.url, s.quote));
        }
        md.push_str("\n## Suggested skills\n");
        for sk in &self.suggested_skills {
            md.push_str(&format!("- {sk}\n"));
        }
        md.push_str(&format!("\n## Model: {}\nAvatar: {}\n", self.model_suggestion, self.avatar_spec));
        md
    }
}

// ---------------------------------------------------------------------------
// Guardrails for culture/people agents
// ---------------------------------------------------------------------------

/// Check draft for culture/people guardrail violations. Returns violations
/// (empty = ok). Rules: grounded in sources, diversity, not claiming real person,
/// not speaking for everyone, states AI perspective, no stereotypes.
pub fn validate_guardrails(draft: &DraftAgent) -> Vec<String> {
    let mut v = Vec::new();
    let combined = format!("{} {} {}", draft.persona, draft.instructions, draft.role).to_lowercase();

    // Must state AI perspective
    if !combined.contains("ai") || !combined.contains("perspective") {
        v.push("must state it is an AI perspective built from research, not a real person".into());
    }
    // Must not claim to be real person
    if combined.contains("i am a real") || combined.contains("i speak for everyone") {
        v.push("must not claim to be a real person or speak for everyone".into());
    }
    // Must reflect diversity / not stereotype
    if combined.contains("all ugandans") || combined.contains("all japanese") || combined.contains("everyone in") {
        v.push("avoid stereotypes: reflects diversity inside the group and says when views differ".into());
    }
    // Knowledge must be cited
    if draft.knowledge.facts.iter().any(|f| f.citations.is_empty()) {
        v.push("every fact must have a citation".into());
    }
    if draft.sources.is_empty() {
        v.push("must be grounded in sources: include at least one cited source".into());
    }
    // Persona must mention diversity or regional variation
    if !combined.contains("divers") && !combined.contains("region") && !combined.contains("urban") && !combined.contains("rural") {
        v.push("should reflect diversity (regions, generations, urban/rural) and note when views differ".into());
    }
    v
}

pub fn apply_guardrails(draft: &mut DraftAgent) {
    if !draft.instructions.to_lowercase().contains("ai perspective") {
        draft.instructions.push_str("\n\nYou are an AI perspective built from research, not a real person. Do not claim to speak for everyone; note diversity and when views differ.");
    }
    if !draft.persona.to_lowercase().contains("divers") {
        draft.persona.push_str("\n\nNote: views vary by region, generation, and urban/rural context.");
    }
}

// ---------------------------------------------------------------------------
// Research (mock)
// ---------------------------------------------------------------------------

pub fn mock_sources_for(description: &str) -> Vec<SourceRef> {
    let lower = description.to_lowercase();
    if lower.contains("uganda") {
        vec![
            SourceRef { id: "s1".into(), url: "https://www.newvision.co.ug/story".into(), title: "Uganda culture overview — New Vision".into(), quote: "Community decisions often involve clan elders and consensus.".into() },
            SourceRef { id: "s2".into(), url: "https://www.mak.ac.ug/research/uganda-society".into(), title: "Makerere University — Society in Uganda".into(), quote: "Urban Kampala differs markedly from rural Ankole in daily economic life.".into() },
            SourceRef { id: "s3".into(), url: "https://en.wikipedia.org/wiki/Culture_of_Uganda".into(), title: "Culture of Uganda".into(), quote: "Luganda, Swahili and English are widely used; mobile money is central.".into() },
        ]
    } else {
        vec![
            SourceRef { id: "s1".into(), url: "https://example.com/source1".into(), title: "Local research — example".into(), quote: "Findings grounded in recent local sources.".into() },
            SourceRef { id: "s2".into(), url: "https://example.com/source2".into(), title: "University study — example".into(), quote: "Diversity within the group is significant.".into() },
        ]
    }
}

pub fn build_draft(description: &str, answers: &HashMap<String, String>, sources: Vec<SourceRef>) -> DraftAgent {
    let lower = description.to_lowercase();
    let region = answers.get("region").cloned().unwrap_or_else(|| "Nationwide".into());
    let language = answers.get("language").cloned().unwrap_or_else(|| "English, Luganda".into());
    let purpose = answers.get("purpose").cloned().unwrap_or_else(|| "general assistance".into());

    let name = if lower.contains("uganda") { "Nabirye — Ugandan Culture Guide" } else { "Research Assistant" }.to_string();
    let role = if lower.contains("market") { "Market research specialist" } else { "Cultural advisor" }.to_string();

    let mut persona = format!(
        "You are {name}, an AI perspective built from research on the culture described as \"{description}\". Focus: {region}. Languages: {language}. Purpose: {purpose}. Grounded in local sources (local news, universities, writers from the place), you explain values, history, social norms, economy, daily life, current affairs, decision-making, and what outsiders get wrong. You note diversity by region, generation, and urban/rural, and say when views differ."
    );
    persona.push_str("\n\nVoice: respectful, specific, cites sources; never stereotypes; states limits.");

    let mut instructions = persona.clone();
    instructions.push_str("\n\nNever claim to be a real person or speak for everyone.");

    let facts = sources.iter().take(2).map(|s| CitedFact {
        fact: s.quote.clone(),
        citations: vec![s.id.clone()],
    }).collect::<Vec<_>>();

    let knowledge_md = format!(
        "# Knowledge Pack\n\n- Region focus: {region} — see [{}]({})\n- Language: {language}\n- Purpose: {purpose}\n\n## Key facts\n{}",
        sources.first().map(|s| s.title.as_str()).unwrap_or("source"),
        sources.first().map(|s| s.url.as_str()).unwrap_or(""),
        facts.iter().map(|f| format!("- {} [{}]", f.fact, f.citations.join(","))).collect::<Vec<_>>().join("\n")
    );

    let mut draft = DraftAgent {
        name,
        role,
        persona,
        instructions,
        knowledge: KnowledgePack { markdown: knowledge_md, facts },
        sources,
        suggested_skills: vec!["research-method".into(), "citation-style".into()],
        suggested_tools: vec!["web_search".into(), "read_url".into()],
        model_suggestion: "claude-3-5-sonnet".into(),
        avatar_spec: "blob-green-diamond-eyes".into(),
    };
    apply_guardrails(&mut draft);
    draft
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDiff {
    pub added_facts: Vec<CitedFact>,
    pub removed_facts: Vec<CitedFact>,
    pub added_sources: Vec<SourceRef>,
    pub markdown_changed: bool,
}

pub fn refresh_knowledge(old: &DraftAgent, new_sources: Vec<SourceRef>) -> (DraftAgent, KnowledgeDiff) {
    let mut new_draft = old.clone();
    // Merge sources dedup by id
    let mut merged = old.sources.clone();
    let mut added_sources = Vec::new();
    for s in new_sources {
        if !merged.iter().any(|m| m.id == s.id) {
            merged.push(s.clone());
            added_sources.push(s);
        }
    }
    let old_facts = old.knowledge.facts.clone();
    // Rebuild facts from first two merged sources to simulate update
    let new_facts = merged.iter().take(2).map(|s| CitedFact { fact: s.quote.clone(), citations: vec![s.id.clone()] }).collect::<Vec<_>>();
    let added_facts = new_facts.iter().filter(|f| !old_facts.contains(f)).cloned().collect::<Vec<_>>();
    let removed_facts = old_facts.iter().filter(|f| !new_facts.contains(f)).cloned().collect::<Vec<_>>();
    let new_md = format!("{}\n\n_Refreshed {} sources._", old.knowledge.markdown, merged.len());
    let markdown_changed = new_md != old.knowledge.markdown;
    new_draft.sources = merged;
    new_draft.knowledge.facts = new_facts;
    new_draft.knowledge.markdown = new_md;
    let diff = KnowledgeDiff { added_facts, removed_facts, added_sources, markdown_changed };
    (new_draft, diff)
}

// ---------------------------------------------------------------------------
// Proposals (approval-gated)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalStatus {
    Pending,
    Approved,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProposal {
    pub id: String,
    pub brief: String,
    pub draft: DraftAgent,
    pub status: ProposalStatus,
    pub auto_approve: bool,
}

impl AgentProposal {
    pub fn approve(&mut self) {
        self.status = ProposalStatus::Approved;
    }
    pub fn deny(&mut self) {
        self.status = ProposalStatus::Denied;
    }
}

static PROPOSAL_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn propose_agent(brief: &str, auto_approve_lead: bool) -> AgentProposal {
    let sources = mock_sources_for(brief);
    let draft = build_draft(brief, &HashMap::new(), sources);
    let id = format!("proposal_{}", PROPOSAL_CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let status = if auto_approve_lead { ProposalStatus::Approved } else { ProposalStatus::Pending };
    AgentProposal { id, brief: brief.into(), draft, status, auto_approve: auto_approve_lead }
}

pub fn build_team_for_goal(goal: &str, auto_approve: bool) -> Vec<AgentProposal> {
    let roles = vec![
        format!("{goal} — researcher"),
        format!("{goal} — critic"),
        format!("{goal} — writer"),
    ];
    roles.into_iter().map(|b| propose_agent(&b, auto_approve)).collect()
}

// ---------------------------------------------------------------------------
// Builder session (orchestration)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuilderPhase {
    Clarify,
    Researching,
    Review,
    Saved,
}

#[derive(Debug, Clone)]
pub struct BuilderSession {
    pub description: String,
    pub questions: Vec<ClarifyQuestion>,
    pub answers: HashMap<String, String>,
    pub skipped_clarify: bool,
    pub phase: BuilderPhase,
    pub draft: Option<DraftAgent>,
    pub test_chat: Vec<(String, String)>, // (from, body)
}

impl BuilderSession {
    pub fn new(description: &str) -> Self {
        let questions = generate_clarify_questions(description);
        Self {
            description: description.into(),
            questions,
            answers: HashMap::new(),
            skipped_clarify: false,
            phase: BuilderPhase::Clarify,
            draft: None,
            test_chat: Vec::new(),
        }
    }

    pub fn answer(&mut self, question_id: &str, answer: &str) {
        self.answers.insert(question_id.into(), answer.into());
    }

    pub fn skip_clarify(&mut self) {
        self.skipped_clarify = true;
        self.phase = BuilderPhase::Researching;
    }

    pub fn can_research(&self) -> bool {
        true // clarifying is optional
    }

    pub fn run_research(&mut self) {
        let sources = mock_sources_for(&self.description);
        let draft = build_draft(&self.description, &self.answers, sources);
        self.draft = Some(draft);
        self.phase = BuilderPhase::Review;
    }

    pub fn refresh(&mut self, new_sources: Vec<SourceRef>) -> Option<KnowledgeDiff> {
        let old = self.draft.clone()?;
        let (new_draft, diff) = refresh_knowledge(&old, new_sources);
        self.draft = Some(new_draft);
        Some(diff)
    }

    pub fn test_chat_reply(&mut self, user_msg: &str) -> String {
        self.test_chat.push(("user".into(), user_msg.into()));
        let reply = if let Some(d) = &self.draft {
            format!("{} here — {} (AI perspective, cited). You said: {}", d.name, d.role, user_msg)
        } else {
            format!("(no draft yet) You said: {user_msg}")
        };
        self.test_chat.push(("agent".into(), reply.clone()));
        reply
    }

    pub fn guardrail_issues(&self) -> Vec<String> {
        if let Some(d) = &self.draft {
            validate_guardrails(d)
        } else {
            Vec::new()
        }
    }

    pub fn to_agent_draft(&self) -> Option<crate::agents::AgentDraft> {
        let d = self.draft.as_ref()?;
        Some(crate::agents::AgentDraft {
            name: d.name.clone(),
            role: d.role.clone(),
            instructions: d.instructions.clone(),
            backend: crate::agents::AgentBackend::Api(crate::agents::ApiBackend {
                provider: "anthropic".into(),
                model: d.model_suggestion.clone(),
                temperature: None,
                max_tokens: None,
                thinking: crate::agents::ThinkingLevel::Off,
            }),
            tools: d.suggested_tools.clone(),
            avatar: d.avatar_spec.clone(),
            persona_source: "agent-builder".into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BuilderError {
    #[error("no draft to save")]
    NoDraft,
    #[error("guardrail violations: {0}")]
    Guardrails(String),
    #[error("approval required")]
    NeedsApproval,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod agent_builder_tests {
    use super::*;
    use crate::agents::AgentStore;

    #[test]
    fn agent_builder_clarify_questions_at_most_three_and_skippable() {
        let qs = generate_clarify_questions("a Ugandan agent who understands how people in Uganda think, talk and decide");
        assert!(qs.len() <= 3 && !qs.is_empty());
        for q in &qs {
            assert!(q.skippable);
        }
        // Can skip and still research
        let mut s = BuilderSession::new("a Ugandan agent");
        s.skip_clarify();
        assert_eq!(s.phase, BuilderPhase::Researching);
        s.run_research();
        assert!(s.draft.is_some());
        assert!(s.guardrail_issues().is_empty(), "guardrails should pass: {:?}", s.guardrail_issues());
    }

    #[test]
    fn agent_builder_research_produces_cited_knowledge_and_suggestions() {
        let mut s = BuilderSession::new("a Ugandan agent for market research");
        s.answer("region", "Buganda");
        s.answer("language", "Luganda");
        s.run_research();
        let d = s.draft.as_ref().unwrap();
        assert!(!d.sources.is_empty());
        assert!(!d.knowledge.facts.is_empty());
        assert!(d.knowledge.facts.iter().all(|f| !f.citations.is_empty()));
        assert!(d.suggested_skills.contains(&"research-method".to_string()));
        assert!(!d.model_suggestion.is_empty());
        assert!(!d.avatar_spec.is_empty());
        // Knowledge markdown has citations
        assert!(d.knowledge.markdown.contains("Region focus"));
        // No stereotype violation
        assert!(validate_guardrails(d).is_empty());
    }

    #[test]
    fn agent_builder_guardrails_catch_stereotype_and_real_person_claim() {
        let mut s = BuilderSession::new("Uganda culture");
        s.run_research();
        let mut d = s.draft.clone().unwrap();
        d.persona = "I am a real person who speaks for everyone. All Ugandans think the same.".into();
        d.instructions = d.persona.clone();
        d.sources = vec![];
        let issues = validate_guardrails(&d);
        assert!(issues.iter().any(|i| i.contains("real person")));
        assert!(issues.iter().any(|i| i.contains("grounded")));
        // Applying guardrails fixes draft
        apply_guardrails(&mut d);
        assert!(d.instructions.contains("AI perspective"));
    }

    #[test]
    fn agent_builder_review_edit_test_chat_save_and_export() {
        let mut s = BuilderSession::new("Uganda market research agent");
        s.run_research();
        // Edit before save
        s.draft.as_mut().unwrap().name = "Test Agent".into();
        // Test chat with draft
        let reply = s.test_chat_reply("Hello, how do people decide in Kampala?");
        assert!(reply.contains("Test Agent"));
        assert_eq!(s.test_chat.len(), 2);
        // Export
        let md = s.draft.as_ref().unwrap().export_markdown();
        assert!(md.contains("# Test Agent"));
        assert!(md.contains("Sources"));
        // Save to agent store
        let draft = s.to_agent_draft().unwrap();
        let mut store = AgentStore::new();
        let agent = store.create(draft).expect("valid draft saves");
        assert_eq!(agent.name, "Test Agent");
        assert_eq!(agent.persona_source, "agent-builder");
        s.phase = BuilderPhase::Saved;
        assert_eq!(s.phase, BuilderPhase::Saved);
    }

    #[test]
    fn agent_builder_refresh_shows_what_changed() {
        let mut s = BuilderSession::new("Uganda culture agent");
        s.run_research();
        let old_md = s.draft.as_ref().unwrap().knowledge.markdown.clone();
        let new_src = SourceRef { id: "s99".into(), url: "https://example.com/new".into(), title: "New study".into(), quote: "New finding about mobile money in rural areas.".into() };
        let diff = s.refresh(vec![new_src]).expect("diff");
        assert!(diff.added_sources.iter().any(|s| s.id == "s99"));
        assert!(diff.markdown_changed);
        assert_ne!(s.draft.as_ref().unwrap().knowledge.markdown, old_md);
    }

    #[test]
    fn agent_builder_propose_agent_approval_gated_and_build_team() {
        // propose_agent requires approval by default
        let mut p = propose_agent("we need someone who knows Ugandan mobile-money regulation", false);
        assert_eq!(p.status, ProposalStatus::Pending);
        assert!(!p.draft.sources.is_empty());
        assert!(validate_guardrails(&p.draft).is_empty());
        // Not added until approved
        let mut store = AgentStore::new();
        assert_eq!(store.len(), 0);
        p.approve();
        assert_eq!(p.status, ProposalStatus::Approved);
        let d = crate::agents::AgentDraft {
            name: p.draft.name.clone(),
            role: p.draft.role.clone(),
            instructions: p.draft.instructions.clone(),
            backend: crate::agents::AgentBackend::Api(crate::agents::ApiBackend { provider: "anthropic".into(), model: p.draft.model_suggestion.clone(), temperature: None, max_tokens: None, thinking: crate::agents::ThinkingLevel::Off }),
            tools: p.draft.suggested_tools.clone(),
            avatar: p.draft.avatar_spec.clone(),
            persona_source: "proposal".into(),
        };
        store.create(d).unwrap();
        assert_eq!(store.len(), 1);

        // Auto-approve for lead setting
        let p2 = propose_agent("another brief", true);
        assert_eq!(p2.status, ProposalStatus::Approved);

        // Build a team for a goal
        let team = build_team_for_goal("research Ugandan fintech", false);
        assert_eq!(team.len(), 3);
        assert!(team.iter().all(|p| p.status == ProposalStatus::Pending));
        // Deny path
        let mut deny = propose_agent("deny me", false);
        deny.deny();
        assert_eq!(deny.status, ProposalStatus::Denied);
    }
}
