//! Perspectives and sourced outline (DEEP-RESEARCH.md §4).
//! STORM-style: lead identifies 4-6 perspectives, maps to members, Q&A with grounded expert, outline.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::research_loop::{Finding, Learning};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Perspective {
    pub id: String,
    pub name: String,
    /// Discipline / stakeholder angle
    pub angle: String,
    pub description: String,
    pub research_questions: Vec<String>,
}

impl Perspective {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        angle: impl Into<String>,
        description: impl Into<String>,
        research_questions: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            angle: angle.into(),
            description: description.into(),
            research_questions,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TeamMember {
    pub id: String,
    pub name: String,
    pub expertise: String,
}

impl TeamMember {
    pub fn new(id: impl Into<String>, name: impl Into<String>, expertise: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            expertise: expertise.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerspectiveAssignment {
    pub perspective: Perspective,
    /// Assigned member id (existing) or proposed persona id
    pub member_id: String,
    pub member_name: String,
    /// true if this proposes a new persona the user can add
    pub is_new_proposal: bool,
}

/// A Q&A exchange between a perspective member and the grounded expert.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QaExchange {
    pub perspective_id: String,
    pub question: String,
    pub answer: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutlineSection {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Which perspectives contribute to this section
    pub perspective_ids: Vec<String>,
    /// Which members contributed (mapped)
    pub contributor_member_ids: Vec<String>,
    pub key_questions: Vec<String>,
    /// Sourced citations backing this section
    pub source_refs: Vec<String>,
    pub key_points: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourcedOutline {
    /// Original topic
    pub topic: String,
    pub title: String,
    pub executive_summary: String,
    pub sections: Vec<OutlineSection>,
    /// All source urls used across sections (deduplicated, sorted)
    pub all_sources: Vec<String>,
    /// Contributions per member id
    pub contributions: HashMap<String, Vec<String>>, // member_id -> section ids
    /// Gaps / open questions
    pub gaps: Vec<String>,
}

// ---------------------------------------------------------------------------
// LLM trait for perspective generation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait PerspectiveLlm: Send + Sync {
    async fn generate_perspectives(&self, topic: &str, n: usize) -> Vec<Perspective>;
    async fn generate_outline_title(&self, topic: &str) -> String {
        format!("Research report: {topic}")
    }
}

pub struct MockPerspectiveLlm {
    perspectives: Mutex<Vec<Perspective>>,
}

impl MockPerspectiveLlm {
    pub fn new(perspectives: Vec<Perspective>) -> Self {
        Self { perspectives: Mutex::new(perspectives) }
    }
    pub fn with_defaults() -> Self {
        Self::new(default_perspectives())
    }
}

fn default_perspectives() -> Vec<Perspective> {
    vec![
        Perspective::new("p1", "Technical", "Engineering", "Technical feasibility and architecture", vec!["What are the technical challenges?".into(), "What architectures scale best?".into()]),
        Perspective::new("p2", "Economic", "Economics", "Cost, market and business model", vec!["What is the cost structure?".into()]),
        Perspective::new("p3", "User", "UX / Human factors", "User needs and adoption", vec!["Who are the users and what do they need?".into()]),
        Perspective::new("p4", "Regulatory", "Policy / Legal", "Regulation, compliance, risks", vec!["What regulations apply?".into()]),
        Perspective::new("p5", "Environmental", "Sustainability", "Environmental and societal impact", vec!["What is the environmental impact?".into()]),
    ]
}

#[async_trait::async_trait]
impl PerspectiveLlm for MockPerspectiveLlm {
    async fn generate_perspectives(&self, topic: &str, n: usize) -> Vec<Perspective> {
        let mut v = self.perspectives.lock().unwrap().clone();
        if v.is_empty() {
            v = default_perspectives();
        }
        // Ensure topic is reflected in descriptions if not already
        for p in &mut v {
            if !p.description.contains(topic) {
                // keep as is; do not mutate
            }
        }
        v.into_iter().take(n).collect()
    }
}

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// Identify 4-6 perspectives for a topic via LLM. Clamps n to 4-6.
pub async fn identify_perspectives(
    llm: &dyn PerspectiveLlm,
    topic: &str,
    n: usize,
) -> Vec<Perspective> {
    let n = n.clamp(4, 6);
    let mut perspectives = llm.generate_perspectives(topic, n).await;
    // Ensure distinct ids and names, clamp to requested range
    if perspectives.len() < 4 {
        let mut extra = default_perspectives();
        for p in extra.drain(..) {
            if perspectives.len() >= 4 {
                break;
            }
            if !perspectives.iter().any(|e| e.id == p.id) {
                perspectives.push(p);
            }
        }
    }
    perspectives.truncate(n);
    // dedupe ids
    let mut seen = HashSet::new();
    perspectives.retain(|p| seen.insert(p.id.clone()));
    // fill up if dedupe reduced below 4
    while perspectives.len() < 4 {
        let id = format!("p{}", perspectives.len() + 1);
        perspectives.push(Perspective::new(
            id.clone(),
            format!("Perspective {}", perspectives.len() + 1),
            "General",
            format!("Additional angle for {topic}"),
            vec![format!("What does {topic} mean from this angle?")],
        ));
    }
    perspectives
}

/// Map perspectives to team members. Prefer members whose expertise matches angle/name
/// (case-insensitive substring). Proposes new personas for unmatched perspectives.
pub fn map_perspectives_to_members(
    perspectives: &[Perspective],
    members: &[TeamMember],
) -> Vec<PerspectiveAssignment> {
    let mut used_members: HashSet<String> = HashSet::new();
    let mut assignments = Vec::new();

    for p in perspectives {
        let mut best: Option<&TeamMember> = None;
        // score: expertise contains angle or name contains expertise
        let angle_lower = p.angle.to_lowercase();
        let name_lower = p.name.to_lowercase();
        for m in members {
            if used_members.contains(&m.id) {
                continue;
            }
            let exp_lower = m.expertise.to_lowercase();
            let member_name_lower = m.name.to_lowercase();
            if exp_lower.contains(&angle_lower)
                || angle_lower.contains(&exp_lower)
                || name_lower.contains(&exp_lower)
                || exp_lower.contains(&name_lower)
                || member_name_lower.contains(&name_lower)
            {
                best = Some(m);
                break;
            }
        }
        // fallback: first unused member
        if best.is_none() {
            for m in members {
                if !used_members.contains(&m.id) {
                    best = Some(m);
                    break;
                }
            }
        }
        if let Some(m) = best {
            used_members.insert(m.id.clone());
            assignments.push(PerspectiveAssignment {
                perspective: p.clone(),
                member_id: m.id.clone(),
                member_name: m.name.clone(),
                is_new_proposal: false,
            });
        } else {
            // propose new persona
            assignments.push(PerspectiveAssignment {
                perspective: p.clone(),
                member_id: format!("proposed-{}", p.id),
                member_name: format!("{} Specialist", p.name),
                is_new_proposal: true,
            });
        }
    }
    assignments
}

/// Simulate Q&A: each perspective asks its research questions to a grounded expert.
/// Uses findings (learnings + citations) as the expert's knowledge base.
/// Returns QaExchanges with sourced answers.
pub fn conduct_qa(
    assignments: &[PerspectiveAssignment],
    findings_by_perspective: &HashMap<String, Vec<Finding>>,
) -> Vec<QaExchange> {
    let mut exchanges = Vec::new();
    for a in assignments {
        let findings = findings_by_perspective.get(&a.perspective.id);
        for q in &a.perspective.research_questions {
            let (answer, sources) = if let Some(findings) = findings {
                // build answer from learnings that exist
                let mut texts = Vec::new();
                let mut srcs = Vec::new();
                for f in findings.iter() {
                    for l in &f.learnings {
                        texts.push(l.text.clone());
                        srcs.extend(l.sources.clone());
                    }
                    srcs.extend(f.citations.clone());
                }
                srcs.sort();
                srcs.dedup();
                let ans = if texts.is_empty() {
                    format!("Based on available sources, no direct finding for: {q}")
                } else {
                    // join first two learnings as answer
                    texts.into_iter().take(2).collect::<Vec<_>>().join(" ")
                };
                (ans, srcs)
            } else {
                (
                    format!("No findings yet for perspective '{}': {q}", a.perspective.name),
                    Vec::new(),
                )
            };
            exchanges.push(QaExchange {
                perspective_id: a.perspective.id.clone(),
                question: q.clone(),
                answer,
                sources,
            });
        }
    }
    exchanges
}

/// Assemble a sourced outline from findings grouped by perspective.
/// Each section maps to one perspective (and its assigned member), includes key questions and citations.
pub fn assemble_outline(
    topic: &str,
    title: Option<String>,
    assignments: &[PerspectiveAssignment],
    findings_by_perspective: &HashMap<String, Vec<Finding>>,
    qa_exchanges: &[QaExchange],
) -> SourcedOutline {
    let outline_title = title.unwrap_or_else(|| format!("Research report: {topic}"));
    let mut sections = Vec::new();
    let mut all_sources_set: HashSet<String> = HashSet::new();
    let mut contributions: HashMap<String, Vec<String>> = HashMap::new();
    let mut gaps = Vec::new();

    // Map perspective id -> assignment for contributor lookup
    let _assign_by_pers: HashMap<String, &PerspectiveAssignment> =
        assignments.iter().map(|a| (a.perspective.id.clone(), a)).collect();

    for a in assignments {
        let findings = findings_by_perspective.get(&a.perspective.id);
        let mut source_refs: Vec<String> = Vec::new();
        let mut key_points: Vec<String> = Vec::new();
        if let Some(fs) = findings {
            for f in fs.iter() {
                for l in &f.learnings {
                    key_points.push(l.text.clone());
                    for s in &l.sources {
                        source_refs.push(s.clone());
                        all_sources_set.insert(s.clone());
                    }
                }
                for c in &f.citations {
                    source_refs.push(c.clone());
                    all_sources_set.insert(c.clone());
                }
                for g in &f.gaps {
                    gaps.push(format!("{}: {g}", a.perspective.name));
                }
            }
        }
        // also pull sources from Q&A for this perspective
        for qa in qa_exchanges.iter().filter(|q| q.perspective_id == a.perspective.id) {
            for s in &qa.sources {
                if !source_refs.contains(s) {
                    source_refs.push(s.clone());
                }
                all_sources_set.insert(s.clone());
            }
        }
        source_refs.sort();
        source_refs.dedup();
        key_points.sort();
        key_points.dedup();
        // truncate key_points to keep outline readable
        key_points.truncate(5);

        if key_points.is_empty() {
            gaps.push(format!("{}: no findings yet", a.perspective.name));
            key_points.push(format!("Open questions: {}", a.perspective.research_questions.join("; ")));
        }

        let section_id = format!("sec-{}", a.perspective.id);
        contributions.entry(a.member_id.clone()).or_default().push(section_id.clone());

        sections.push(OutlineSection {
            id: section_id,
            title: a.perspective.name.clone(),
            description: a.perspective.description.clone(),
            perspective_ids: vec![a.perspective.id.clone()],
            contributor_member_ids: vec![a.member_id.clone()],
            key_questions: a.perspective.research_questions.clone(),
            source_refs,
            key_points,
        });
    }

    // Ensure every perspective appears at least once (covered)
    let mut all_sources: Vec<String> = all_sources_set.into_iter().collect();
    all_sources.sort();

    // Executive summary from first points
    let executive_summary = if sections.is_empty() {
        format!("No perspectives identified for {topic}.")
    } else {
        let mut sum = format!("Report on {topic} covering {} perspectives: ", sections.len());
        sum.push_str(
            &sections
                .iter()
                .map(|s| s.title.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );
        sum.push_str(". ");
        for s in sections.iter().take(2) {
            if let Some(kp) = s.key_points.first() {
                sum.push_str(kp);
                sum.push(' ');
            }
        }
        let wc = sum.split_whitespace().count();
        if wc > 120 {
            sum = sum.split_whitespace().take(120).collect::<Vec<_>>().join(" ") + "…";
        }
        sum
    };

    // Deduplicate gaps
    gaps.sort();
    gaps.dedup();

    SourcedOutline {
        topic: topic.to_string(),
        title: outline_title,
        executive_summary,
        sections,
        all_sources,
        contributions,
        gaps,
    }
}

/// Convenience: full STORM flow — identify, map, QA, outline.
pub async fn research_perspectives_flow(
    llm: &dyn PerspectiveLlm,
    topic: &str,
    members: &[TeamMember],
    findings_by_perspective: &HashMap<String, Vec<Finding>>,
) -> (Vec<Perspective>, Vec<PerspectiveAssignment>, Vec<QaExchange>, SourcedOutline) {
    let perspectives = identify_perspectives(llm, topic, 5).await;
    let assignments = map_perspectives_to_members(&perspectives, members);
    let qa = conduct_qa(&assignments, findings_by_perspective);
    let outline = assemble_outline(topic, None, &assignments, findings_by_perspective, &qa);
    (perspectives, assignments, qa, outline)
}

// ---------------------------------------------------------------------------
// Tests — must contain "perspectives" in name
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research_loop::{Finding, Learning};
    use std::collections::HashMap;
    use std::time::Duration;

    fn members_fixture() -> Vec<TeamMember> {
        vec![
            TeamMember::new("m1", "Alice", "Engineering"),
            TeamMember::new("m2", "Bob", "Economics"),
            TeamMember::new("m3", "Carol", "Policy"),
        ]
    }

    fn finding_with(text: &str, url: &str) -> Finding {
        Finding {
            query: "q".into(),
            summary: text.into(),
            learnings: vec![Learning {
                text: text.into(),
                sources: vec![url.into()],
            }],
            citations: vec![url.into()],
            visited: vec![url.into()],
            gaps: vec![],
            searches_used: 1,
            budget_exhausted: false,
            elapsed: Duration::from_millis(10),
        }
    }

    #[tokio::test]
    async fn perspectives_identify_returns_four_to_six() {
        let llm = MockPerspectiveLlm::with_defaults();
        let ps = identify_perspectives(&llm, "quantum computing", 5).await;
        assert!(ps.len() >= 4 && ps.len() <= 6, "got {}", ps.len());
        // distinct ids
        let mut ids = std::collections::HashSet::new();
        for p in &ps {
            assert!(ids.insert(p.id.clone()), "duplicate id {}", p.id);
            assert!(!p.research_questions.is_empty());
        }
    }

    #[tokio::test]
    async fn perspectives_identify_clamps_to_range() {
        let llm = MockPerspectiveLlm::with_defaults();
        let ps_small = identify_perspectives(&llm, "topic", 2).await;
        assert!(ps_small.len() >= 4);
        let ps_large = identify_perspectives(&llm, "topic", 20).await;
        assert!(ps_large.len() <= 6);
    }

    #[tokio::test]
    async fn perspectives_map_to_members_covers_all() {
        let llm = MockPerspectiveLlm::with_defaults();
        let perspectives = identify_perspectives(&llm, "AI safety", 5).await;
        let members = members_fixture();
        let assignments = map_perspectives_to_members(&perspectives, &members);
        assert_eq!(assignments.len(), perspectives.len());
        // each assignment has a perspective and member
        for a in &assignments {
            assert!(!a.perspective.id.is_empty());
            assert!(!a.member_id.is_empty());
        }
        // at least one proposed persona when more perspectives than members
        let proposed = assignments.iter().filter(|a| a.is_new_proposal).count();
        assert!(proposed >= 2, "expected proposals when 5 perspectives and 3 members");
    }

    #[tokio::test]
    async fn perspectives_map_prefers_expertise_match() {
        let perspectives = vec![
            Perspective::new("p1", "Technical", "Engineering", "desc", vec!["q1".into()]),
            Perspective::new("p2", "Economic", "Economics", "desc", vec!["q2".into()]),
        ];
        let members = vec![
            TeamMember::new("m1", "Alice", "Engineering"),
            TeamMember::new("m2", "Bob", "Economics"),
        ];
        let assignments = map_perspectives_to_members(&perspectives, &members);
        assert_eq!(assignments[0].member_id, "m1");
        assert_eq!(assignments[1].member_id, "m2");
        assert!(!assignments[0].is_new_proposal);
    }

    #[tokio::test]
    async fn perspectives_qa_grounded_in_findings() {
        let perspectives = vec![Perspective::new(
            "p1",
            "Technical",
            "Engineering",
            "desc",
            vec!["What scales?".into()],
        )];
        let members = vec![TeamMember::new("m1", "Alice", "Engineering")];
        let assignments = map_perspectives_to_members(&perspectives, &members);
        let mut map = HashMap::new();
        map.insert("p1".to_string(), vec![finding_with("Edge computing scales via X", "https://example.com/a")]);
        let qa = conduct_qa(&assignments, &map);
        assert_eq!(qa.len(), 1);
        assert!(qa[0].answer.contains("Edge computing"));
        assert!(qa[0].sources.contains(&"https://example.com/a".to_string()));
    }

    #[tokio::test]
    async fn perspectives_assemble_sourced_outline() {
        let llm = MockPerspectiveLlm::with_defaults();
        let perspectives = identify_perspectives(&llm, "fusion energy", 4).await;
        let members = members_fixture();
        let assignments = map_perspectives_to_members(&perspectives, &members);
        // Build findings per perspective with distinct sources
        let mut findings: HashMap<String, Vec<Finding>> = HashMap::new();
        for p in &perspectives {
            findings.insert(
                p.id.clone(),
                vec![finding_with(
                    &format!("Finding for {}", p.name),
                    &format!("https://example.com/{}", p.id),
                )],
            );
        }
        let qa = conduct_qa(&assignments, &findings);
        let outline = assemble_outline("fusion energy", None, &assignments, &findings, &qa);
        assert_eq!(outline.topic, "fusion energy");
        assert!(outline.title.contains("fusion energy"));
        assert_eq!(outline.sections.len(), perspectives.len());
        assert!(!outline.executive_summary.is_empty());
        // every section has perspective and source and questions visible for user to follow
        for sec in &outline.sections {
            assert!(!sec.perspective_ids.is_empty());
            assert!(!sec.key_questions.is_empty(), "section {} missing questions", sec.id);
            assert!(!sec.source_refs.is_empty(), "section {} missing sources", sec.id);
            assert!(!sec.key_points.is_empty());
        }
        // all_sources deduped and sorted
        let mut sorted = outline.all_sources.clone();
        sorted.sort();
        assert_eq!(sorted, outline.all_sources);
        assert!(outline.all_sources.len() >= perspectives.len());
        // contributions map members to sections
        for a in &assignments {
            let contrib = outline.contributions.get(&a.member_id).expect("member missing");
            assert!(contrib.iter().any(|id| outline.sections.iter().any(|s| &s.id == id)));
        }
    }

    #[tokio::test]
    async fn perspectives_outline_maps_contributions_and_gaps() {
        let perspectives = vec![
            Perspective::new("p1", "Technical", "Engineering", "desc", vec!["q1".into()]),
            Perspective::new("p2", "Economic", "Economics", "desc", vec!["q2".into()]),
        ];
        let members = vec![TeamMember::new("m1", "Alice", "Engineering")];
        let assignments = map_perspectives_to_members(&perspectives, &members);
        // only p1 has findings, p2 will be a gap
        let mut findings = HashMap::new();
        findings.insert("p1".to_string(), vec![finding_with("Tech finding", "https://a.com")]);
        let qa = conduct_qa(&assignments, &findings);
        let outline = assemble_outline("topic", Some("My Report".into()), &assignments, &findings, &qa);
        assert_eq!(outline.title, "My Report");
        // p2 should contribute a gap
        assert!(outline.gaps.iter().any(|g| g.contains("Economic") || g.contains("no findings")));
        // section for p2 still present with open questions
        let sec2 = outline.sections.iter().find(|s| s.perspective_ids.contains(&"p2".to_string())).unwrap();
        assert!(sec2.key_points.iter().any(|k| k.contains("Open questions") || k.contains("q2")));
    }

    #[tokio::test]
    async fn perspectives_full_flow_integration() {
        let llm = MockPerspectiveLlm::with_defaults();
        let members = members_fixture();
        let mut findings: HashMap<String, Vec<Finding>> = HashMap::new();
        // We don't know perspective ids ahead of flow, so pass empty and check flow creates them
        let (perspectives, assignments, qa, outline) =
            research_perspectives_flow(&llm, "CRISPR regulation", &members, &findings).await;
        assert!(perspectives.len() >= 4 && perspectives.len() <= 6);
        assert_eq!(assignments.len(), perspectives.len());
        assert_eq!(outline.sections.len(), perspectives.len());
        // Now add findings for each perspective and re-run outline portion
        for p in &perspectives {
            findings.insert(
                p.id.clone(),
                vec![finding_with(&format!("Study on {}", p.name), &format!("https://src/{}.com", p.id))],
            );
        }
        let qa2 = conduct_qa(&assignments, &findings);
        let outline2 = assemble_outline("CRISPR regulation", None, &assignments, &findings, &qa2);
        assert!(outline2.all_sources.len() >= perspectives.len());
        // QA count = sum of research questions
        let expected_qa: usize = perspectives.iter().map(|p| p.research_questions.len()).sum();
        assert_eq!(qa2.len(), expected_qa);
        // perspectives and outline together let user follow questions and contributions
        for sec in &outline2.sections {
            assert!(!sec.key_questions.is_empty());
            assert!(!sec.contributor_member_ids.is_empty());
        }
        let _ = qa; // keep
    }
}
