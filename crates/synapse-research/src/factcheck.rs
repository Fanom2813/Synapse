//! Critique, revision, and citation checks (DEEP-RESEARCH.md §5).
//!
//! - Critique: find unsupported claims, contradictions, missing perspectives, weak reasoning.
//! - Revision: answer each challenge (fix / rebut / open question) → new report version + changelog + diff.
//! - Fact-check: verify every cited quotation exists verbatim in stored source text; flag unsupported claims.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Report model (lightweight, independent of synapse-core artifacts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Citation {
    pub source_id: String,
    pub quote: String,
}

impl Citation {
    pub fn new(source_id: impl Into<String>, quote: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            quote: quote.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Paragraph {
    pub id: String,
    pub text: String,
    pub citations: Vec<Citation>,
}

impl Paragraph {
    pub fn new(id: impl Into<String>, text: impl Into<String>, citations: Vec<Citation>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            citations,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Report {
    pub title: String,
    pub paragraphs: Vec<Paragraph>,
    /// Perspectives the outline promised (to detect missing ones).
    pub expected_perspectives: Vec<String>,
    /// Which perspectives are actually covered (by name).
    pub covered_perspectives: Vec<String>,
}

impl Report {
    pub fn new(title: impl Into<String>, paragraphs: Vec<Paragraph>) -> Self {
        Self {
            title: title.into(),
            paragraphs,
            expected_perspectives: vec![],
            covered_perspectives: vec![],
        }
    }
    pub fn with_perspectives(
        mut self,
        expected: Vec<String>,
        covered: Vec<String>,
    ) -> Self {
        self.expected_perspectives = expected;
        self.covered_perspectives = covered;
        self
    }
    pub fn body_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# {}\n\n", self.title));
        for p in &self.paragraphs {
            out.push_str(&p.text);
            out.push_str("\n\n");
        }
        out
    }
}

/// Stored source text (from SourceCache / sources table).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceDoc {
    pub id: String,
    pub title: String,
    pub url: String,
    pub content_md: String,
}

impl SourceDoc {
    pub fn new(id: impl Into<String>, title: impl Into<String>, url: impl Into<String>, content_md: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            url: url.into(),
            content_md: content_md.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Challenges
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChallengeKind {
    UnsupportedClaim,
    QuoteNotFound,
    Contradiction,
    MissingPerspective,
    WeakReasoning,
}

impl ChallengeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnsupportedClaim => "unsupported_claim",
            Self::QuoteNotFound => "quote_not_found",
            Self::Contradiction => "contradiction",
            Self::MissingPerspective => "missing_perspective",
            Self::WeakReasoning => "weak_reasoning",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Challenge {
    /// Unique id for this challenge.
    pub id: String,
    /// Paragraph this challenge is linked to (empty for report-level like missing perspective).
    pub paragraph_id: String,
    pub kind: ChallengeKind,
    pub description: String,
    pub severity: String, // "error" | "warning"
}

impl Challenge {
    pub fn new(
        id: impl Into<String>,
        paragraph_id: impl Into<String>,
        kind: ChallengeKind,
        description: impl Into<String>,
    ) -> Self {
        let severity = match kind {
            ChallengeKind::UnsupportedClaim | ChallengeKind::QuoteNotFound | ChallengeKind::Contradiction => "error",
            _ => "warning",
        };
        Self {
            id: id.into(),
            paragraph_id: paragraph_id.into(),
            kind,
            description: description.into(),
            severity: severity.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Normalization helpers for quote matching
// ---------------------------------------------------------------------------

fn normalize(s: &str) -> String {
    // Collapse whitespace, trim, lowercase for robust matching.
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn quote_exists_in_source(quote: &str, source_content: &str) -> bool {
    let q = normalize(quote);
    if q.is_empty() {
        return false;
    }
    let c = normalize(source_content);
    c.contains(&q)
}

// ---------------------------------------------------------------------------
// Fact-check
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactCheckResult {
    pub paragraph_id: String,
    pub citation: Citation,
    pub supported: bool,
    pub reason: String,
}

/// Verify every citation's quote exists in the stored source text.
/// Returns one result per citation; unsupported if source missing or quote not found.
pub fn fact_check_report(report: &Report, sources: &[SourceDoc]) -> Vec<FactCheckResult> {
    let by_id: HashMap<&str, &SourceDoc> = sources.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut out = Vec::new();
    for p in &report.paragraphs {
        for c in &p.citations {
            let q_trim = c.quote.trim();
            if q_trim.is_empty() {
                out.push(FactCheckResult {
                    paragraph_id: p.id.clone(),
                    citation: c.clone(),
                    supported: false,
                    reason: "empty quote".into(),
                });
                continue;
            }
            match by_id.get(c.source_id.as_str()) {
                None => out.push(FactCheckResult {
                    paragraph_id: p.id.clone(),
                    citation: c.clone(),
                    supported: false,
                    reason: format!("source '{}' not found in store", c.source_id),
                }),
                Some(src) => {
                    if quote_exists_in_source(q_trim, &src.content_md) {
                        out.push(FactCheckResult {
                            paragraph_id: p.id.clone(),
                            citation: c.clone(),
                            supported: true,
                            reason: "quote found in source".into(),
                        });
                    } else {
                        out.push(FactCheckResult {
                            paragraph_id: p.id.clone(),
                            citation: c.clone(),
                            supported: false,
                            reason: format!("quote not found in source '{}'", c.source_id),
                        });
                    }
                }
            }
        }
    }
    out
}

/// Convenience: only failures.
pub fn unsupported_claims(report: &Report, sources: &[SourceDoc]) -> Vec<FactCheckResult> {
    fact_check_report(report, sources)
        .into_iter()
        .filter(|r| !r.supported)
        .collect()
}

// ---------------------------------------------------------------------------
// Critique
// ---------------------------------------------------------------------------

/// Run full critique: unsupported claims (missing citations), quote checks,
/// missing perspectives, weak reasoning heuristics, and contradictions.
///
/// Returns challenges linked to paragraphs (or report-level for missing perspectives).
pub fn critique_report(report: &Report, sources: &[SourceDoc]) -> Vec<Challenge> {
    let mut challenges = Vec::new();
    let mut cid = 0usize;
    let mut next_id = || {
        cid += 1;
        format!("ch{cid}")
    };

    // Map for quote checks
    let by_id: HashMap<&str, &SourceDoc> = sources.iter().map(|s| (s.id.as_str(), s)).collect();

    // 1. Paragraph-level: unsupported claim if paragraph has factual signal but no citation.
    // Heuristic: paragraphs with >15 words and no citations, or containing numbers/percent.
    for p in &report.paragraphs {
        if p.citations.is_empty() {
            let wc = p.text.split_whitespace().count();
            let looks_factual = wc > 15
                || p.text.contains('%')
                || p.text.contains("according to")
                || p.text.contains("study")
                || p.text.chars().any(|c| c.is_ascii_digit());
            if looks_factual {
                challenges.push(Challenge::new(
                    next_id(),
                    p.id.clone(),
                    ChallengeKind::UnsupportedClaim,
                    format!("Paragraph '{}' makes factual claims but has no citations.", p.id),
                ));
            }
        }
        // 2. Quote not found (fact-check inline)
        for c in &p.citations {
            let q = c.quote.trim();
            if q.is_empty() {
                challenges.push(Challenge::new(
                    next_id(),
                    p.id.clone(),
                    ChallengeKind::QuoteNotFound,
                    format!("Citation for '{}' has empty quote.", c.source_id),
                ));
                continue;
            }
            match by_id.get(c.source_id.as_str()) {
                None => challenges.push(Challenge::new(
                    next_id(),
                    p.id.clone(),
                    ChallengeKind::QuoteNotFound,
                    format!("Source '{}' not found for quote: \"{}\"", c.source_id, q),
                )),
                Some(src) => {
                    if !quote_exists_in_source(q, &src.content_md) {
                        challenges.push(Challenge::new(
                            next_id(),
                            p.id.clone(),
                            ChallengeKind::QuoteNotFound,
                            format!(
                                "Quoted text not found in source '{}': \"{}\"",
                                c.source_id, q
                            ),
                        ));
                    }
                }
            }
        }
        // 3. Weak reasoning heuristic: very short paragraph with vague quantifiers
        let wc = p.text.split_whitespace().count();
        if wc > 5 && wc < 20 {
            let lower = p.text.to_lowercase();
            let vague = ["some say", "many believe", "it is thought", "probably", "might be"];
            if vague.iter().any(|v| lower.contains(v)) {
                challenges.push(Challenge::new(
                    next_id(),
                    p.id.clone(),
                    ChallengeKind::WeakReasoning,
                    format!("Paragraph '{}' uses vague language without evidence.", p.id),
                ));
            }
        }
    }

    // 4. Missing perspectives
    if !report.expected_perspectives.is_empty() {
        let expected: HashSet<String> = report
            .expected_perspectives
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        let covered: HashSet<String> = report
            .covered_perspectives
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        for exp in &expected {
            if !covered.contains(exp) {
                challenges.push(Challenge::new(
                    next_id(),
                    "",
                    ChallengeKind::MissingPerspective,
                    format!("Missing perspective: '{}' — no paragraph covers it.", exp),
                ));
            }
        }
    }

    // 5. Contradictions between sources referenced in different paragraphs
    // Simple heuristic: if two citations from different sources have quotes that are contradictory
    // numeric claims (e.g., "rose 12%" vs "fell 5%") we flag. For now, flag when same paragraph
    // cites two sources with very different content? We do a simpler check:
    // If report has two paragraphs citing different sources but with identical normalized quotes
    // that diverge? To keep deterministic, we check for explicit contradiction markers in paragraph text.
    // We'll look for paragraphs containing "however" / "contradict" alongside citations from different sources.
    // More generally, if sources_by_id have conflicting numbers and report doesn't note disagreement, flag.
    // Simple implementation: if two paragraphs cite different sources and their texts contain opposing verbs
    // about same subject without a dissent note, emit one contradiction challenge.

    // Collect source ids per paragraph
    let mut source_sets: HashMap<String, HashSet<String>> = HashMap::new();
    for p in &report.paragraphs {
        let mut set = HashSet::new();
        for c in &p.citations {
            set.insert(c.source_id.clone());
        }
        if !set.is_empty() {
            source_sets.insert(p.id.clone(), set);
        }
    }
    // If multiple paragraphs and they cite distinct sources but claim opposing directions without
    // a paragraph mentioning disagreement, flag briefing.
    if report.paragraphs.len() >= 2 {
        let all_cited_sources: HashSet<String> = report
            .paragraphs
            .iter()
            .flat_map(|p| p.citations.iter().map(|c| c.source_id.clone()))
            .collect();
        if all_cited_sources.len() >= 2 {
            // Check if any paragraph text hints at contradiction not acknowledged
            let has_contradiction_text = report.paragraphs.iter().any(|p| {
                let l = p.text.to_lowercase();
                l.contains("contradict") || l.contains("however") || l.contains("on the other hand")
            });
            // If sources disagree numerically (simple: source contents contain opposite trends)
            // We detect by source docs containing opposing keywords.
            let mut has_opposing = false;
            let src_texts: Vec<String> = sources.iter().map(|s| s.content_md.to_lowercase()).collect();
            for t in &src_texts {
                if t.contains("increased") || t.contains("rose") || t.contains("growth") {
                    if src_texts.iter().any(|u| u.contains("decreased") || u.contains("fell") || u.contains("decline")) {
                        has_opposing = true;
                        break;
                    }
                }
            }
            if has_opposing && !has_contradiction_text {
                // Only emit if not already covered by dissent-like text
                let mentions_disagreement = report.title.to_lowercase().contains("disagreement")
                    || report.paragraphs.iter().any(|p| {
                        let l = p.text.to_lowercase();
                        l.contains("disagreement") || l.contains("conflict") || l.contains("mixed evidence")
                    });
                if !mentions_disagreement {
                    challenges.push(Challenge::new(
                        next_id(),
                        report.paragraphs.first().map(|p| p.id.clone()).unwrap_or_default(),
                        ChallengeKind::Contradiction,
                        "Sources contain contradictory claims (e.g., increase vs decrease) but the report does not discuss the disagreement.",
                    ));
                }
            }
        }
    }

    challenges
}

// ---------------------------------------------------------------------------
// Revision
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResolutionKind {
    Fixed,
    Rebutted,
    OpenQuestion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Resolution {
    pub challenge_id: String,
    pub kind: ResolutionKind,
    pub note: String,
    /// Optional replacement text for the paragraph (when Fixed).
    pub replacement_text: Option<String>,
    /// Optional extra citations added with the fix.
    pub added_citations: Vec<Citation>,
}

impl Resolution {
    pub fn fix(challenge_id: impl Into<String>, note: impl Into<String>, replacement_text: impl Into<String>) -> Self {
        Self {
            challenge_id: challenge_id.into(),
            kind: ResolutionKind::Fixed,
            note: note.into(),
            replacement_text: Some(replacement_text.into()),
            added_citations: vec![],
        }
    }
    pub fn rebut(challenge_id: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            challenge_id: challenge_id.into(),
            kind: ResolutionKind::Rebutted,
            note: note.into(),
            replacement_text: None,
            added_citations: vec![],
        }
    }
    pub fn open_question(challenge_id: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            challenge_id: challenge_id.into(),
            kind: ResolutionKind::OpenQuestion,
            note: note.into(),
            replacement_text: None,
            added_citations: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Revision {
    pub revised: Report,
    pub changelog: String,
    pub diff: String,
}

/// Apply resolutions to challenges, producing a revised report, a changelog and a unified diff.
///
/// - Fixed: replace paragraph text (and optionally add citations)
/// - Rebutted: keep text, record rebuttal in changelog
/// - OpenQuestion: keep text, record as open question
///
/// Returns error if changelog would be empty or report has no paragraphs.
#[derive(Debug, thiserror::Error)]
pub enum RevisionError {
    #[error("no resolutions: nothing to revise")]
    NoResolutions,
    #[error("challenge '{0}' not found")]
    ChallengeNotFound(String),
}

pub fn revise_report(
    report: &Report,
    challenges: &[Challenge],
    resolutions: &[Resolution],
) -> Result<Revision, RevisionError> {
    if resolutions.is_empty() {
        return Err(RevisionError::NoResolutions);
    }
    let challenge_by_id: HashMap<&str, &Challenge> =
        challenges.iter().map(|c| (c.id.as_str(), c)).collect();

    for r in resolutions {
        if !challenge_by_id.contains_key(r.challenge_id.as_str()) {
            return Err(RevisionError::ChallengeNotFound(r.challenge_id.clone()));
        }
    }

    let before_md = report.body_markdown();
    let mut revised = report.clone();

    // Index paragraphs by id for quick mutation
    let mut para_idx: HashMap<String, usize> = HashMap::new();
    for (i, p) in revised.paragraphs.iter().enumerate() {
        para_idx.insert(p.id.clone(), i);
    }

    let mut changelog_lines: Vec<String> = Vec::new();

    for res in resolutions {
        let ch = challenge_by_id[res.challenge_id.as_str()];
        match res.kind {
            ResolutionKind::Fixed => {
                if let Some(pid) = (!ch.paragraph_id.is_empty()).then_some(ch.paragraph_id.clone()) {
                    if let Some(&idx) = para_idx.get(&pid) {
                        if let Some(new_text) = &res.replacement_text {
                            revised.paragraphs[idx].text = new_text.clone();
                            // append added citations
                            for c in &res.added_citations {
                                if !revised.paragraphs[idx].citations.contains(c) {
                                    revised.paragraphs[idx].citations.push(c.clone());
                                }
                            }
                            changelog_lines.push(format!(
                                "Fixed {} ({}): {} — updated paragraph '{}'.",
                                ch.id, ch.kind.as_str(), ch.description, pid
                            ));
                            if !res.note.trim().is_empty() {
                                changelog_lines.push(format!("  Detail: {}", res.note.trim()));
                            }
                        }
                    }
                } else {
                    // report-level fix (e.g., missing perspective): add a paragraph
                    if let Some(new_text) = &res.replacement_text {
                        let new_id = format!("p{}", revised.paragraphs.len() + 1);
                        revised.paragraphs.push(Paragraph::new(
                            new_id.clone(),
                            new_text.clone(),
                            res.added_citations.clone(),
                        ));
                        revised.covered_perspectives.push(ch.description.clone());
                        changelog_lines.push(format!(
                            "Fixed {} ({}): {} — added paragraph '{}'.",
                            ch.id, ch.kind.as_str(), ch.description, new_id
                        ));
                    }
                }
            }
            ResolutionKind::Rebutted => {
                changelog_lines.push(format!(
                    "Rebutted {} ({}): {} — kept as is. Evidence: {}",
                    ch.id, ch.kind.as_str(), ch.description, res.note.trim()
                ));
            }
            ResolutionKind::OpenQuestion => {
                changelog_lines.push(format!(
                    "Open question from {} ({}): {} — {}",
                    ch.id, ch.kind.as_str(), ch.description, res.note.trim()
                ));
            }
        }
    }

    let after_md = revised.body_markdown();
    let diff = unified_diff(&before_md, &after_md);
    let changelog = changelog_lines.join("\n");

    Ok(Revision {
        revised,
        changelog,
        diff,
    })
}

fn unified_diff(before: &str, after: &str) -> String {
    let diff = similar::TextDiff::from_lines(before, after);
    let mut out = String::new();
    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let sign = match change.tag() {
                similar::ChangeTag::Delete => '-',
                similar::ChangeTag::Insert => '+',
                similar::ChangeTag::Equal => ' ',
            };
            out.push(sign);
            out.push_str(change.value());
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests — filter is "factcheck"
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn src(id: &str, content: &str) -> SourceDoc {
        SourceDoc::new(id, format!("Title {id}"), format!("https://example.com/{id}"), content)
    }

    fn para(id: &str, text: &str, quotes: Vec<(&str, &str)>) -> Paragraph {
        let citations = quotes
            .into_iter()
            .map(|(sid, q)| Citation::new(sid, q))
            .collect();
        Paragraph::new(id, text, citations)
    }

    // --- factcheck tests (verify filter "factcheck") ---

    #[test]
    fn factcheck_quote_found_is_supported() {
        let sources = vec![src("s1", "The market rose 12% in Q2 according to data.")];
        let report = Report::new(
            "Test",
            vec![para("p1", "Markets rose.", vec![("s1", "rose 12%")])],
        );
        let results = fact_check_report(&report, &sources);
        assert_eq!(results.len(), 1);
        assert!(results[0].supported, "got {:?}", results[0]);
    }

    #[test]
    fn factcheck_quote_not_found_is_flagged() {
        let sources = vec![src("s1", "The sky is blue.")];
        let report = Report::new(
            "Test",
            vec![para("p1", "Claim.", vec![("s1", "rose 12%")])],
        );
        let results = fact_check_report(&report, &sources);
        assert_eq!(results.len(), 1);
        assert!(!results[0].supported);
        assert!(results[0].reason.contains("not found"));
        let unsupported = unsupported_claims(&report, &sources);
        assert_eq!(unsupported.len(), 1);
    }

    #[test]
    fn factcheck_missing_source_is_flagged() {
        let sources = vec![src("s1", "hello")];
        let report = Report::new(
            "Test",
            vec![para("p1", "Claim.", vec![("s2", "hello")])],
        );
        let results = fact_check_report(&report, &sources);
        assert!(!results[0].supported);
        assert!(results[0].reason.contains("not found in store"));
    }

    #[test]
    fn factcheck_whitespace_and_case_normalized() {
        let sources = vec![src("s1", "  Demand \n\n rose   12%  ")];
        let report = Report::new(
            "Test",
            vec![para("p1", "x", vec![("s1", "demand rose 12%")])],
        );
        let results = fact_check_report(&report, &sources);
        assert!(results[0].supported);
    }

    #[test]
    fn factcheck_empty_quote_flagged() {
        let sources = vec![src("s1", "content")];
        let report = Report::new("Test", vec![para("p1", "x", vec![("s1", "   ")])]);
        let results = fact_check_report(&report, &sources);
        assert!(!results[0].supported);
        assert!(results[0].reason.contains("empty quote"));
    }

    #[test]
    fn factcheck_multiple_citations_per_paragraph() {
        let sources = vec![
            src("s1", "alpha beta gamma"),
            src("s2", "delta epsilon"),
        ];
        let report = Report::new(
            "Test",
            vec![para(
                "p1",
                "Both claims.",
                vec![("s1", "alpha beta"), ("s2", "delta epsilon"), ("s2", "missing quote")],
            )],
        );
        let results = fact_check_report(&report, &sources);
        assert_eq!(results.len(), 3);
        assert!(results[0].supported);
        assert!(results[1].supported);
        assert!(!results[2].supported);
    }

    // --- critique tests (also match "factcheck" filter if name contains it) ---

    #[test]
    fn factcheck_critique_finds_unsupported_claim_without_citation() {
        let sources = vec![src("s1", "some content")];
        let report = Report::new(
            "Test",
            vec![para(
                "p1",
                "The market increased by 25% last year and is expected to double next year according to studies.",
                vec![],
            )],
        );
        let challenges = critique_report(&report, &sources);
        assert!(
            challenges.iter().any(|c| c.kind == ChallengeKind::UnsupportedClaim && c.paragraph_id == "p1"),
            "got {challenges:?}"
        );
    }

    #[test]
    fn factcheck_critique_finds_quote_not_found() {
        let sources = vec![src("s1", "unrelated content")];
        let report = Report::new(
            "Test",
            vec![para("p1", "Claim with bad quote.", vec![("s1", "not in source")])],
        );
        let challenges = critique_report(&report, &sources);
        assert!(challenges.iter().any(|c| c.kind == ChallengeKind::QuoteNotFound));
    }

    #[test]
    fn factcheck_critique_finds_missing_perspective() {
        let sources = vec![src("s1", "content")];
        let report = Report::new("Test", vec![para("p1", "Short text.", vec![])])
            .with_perspectives(
                vec!["Technical".into(), "Economic".into(), "Regulatory".into()],
                vec!["Technical".into()],
            );
        let challenges = critique_report(&report, &sources);
        let missing: Vec<_> = challenges
            .iter()
            .filter(|c| c.kind == ChallengeKind::MissingPerspective)
            .collect();
        assert_eq!(missing.len(), 2);
        assert!(missing.iter().any(|c| c.description.contains("economic")));
    }

    #[test]
    fn factcheck_critique_flags_weak_reasoning() {
        let sources = vec![];
        let report = Report::new(
            "Test",
            vec![para("p1", "Many believe it might be good, some say it is useful.", vec![])],
        );
        let challenges = critique_report(&report, &sources);
        assert!(challenges.iter().any(|c| c.kind == ChallengeKind::WeakReasoning));
    }

    #[test]
    fn factcheck_critique_flags_contradiction_between_sources() {
        let sources = vec![
            src("s1", "Revenue increased steadily and growth was strong."),
            src("s2", "Revenue decreased sharply and decline continued."),
        ];
        let report = Report::new(
            "Test",
            vec![
                para("p1", "Revenue trends show changes.", vec![("s1", "increased")]),
                para("p2", "More on revenue.", vec![("s2", "decreased")]),
            ],
        );
        let challenges = critique_report(&report, &sources);
        assert!(
            challenges.iter().any(|c| c.kind == ChallengeKind::Contradiction),
            "expected contradiction, got {challenges:?}"
        );
    }

    #[test]
    fn factcheck_critique_challenges_link_to_paragraph() {
        let sources = vec![src("s1", "hello world")];
        let report = Report::new(
            "Test",
            vec![
                para("p1", "Good paragraph with citation.", vec![("s1", "hello world")]),
                para("p2", "The market rose 99% with no source.", vec![]),
            ],
        );
        let challenges = critique_report(&report, &sources);
        for c in &challenges {
            if c.kind == ChallengeKind::UnsupportedClaim {
                assert_eq!(c.paragraph_id, "p2");
            }
        }
    }

    // --- revision tests ---

    #[test]
    fn factcheck_revision_fix_updates_paragraph_and_changelog() {
        let sources = vec![src("s1", "market rose 12%")];
        let report = Report::new(
            "Test",
            vec![para("p1", "The market grew by 40% last year and will double next year according to estimates.", vec![])],
        );
        let challenges = critique_report(&report, &sources);
        // may have unsupported claim challenge for p1? ensure at least one
        assert!(!challenges.is_empty());
        let ch_id = challenges[0].id.clone();
        let res = Resolution::fix(ch_id.clone(), "Added citation", "Market rose 12% [s1].");
        let rev = revise_report(&report, &challenges, &[res]).expect("revise");
        assert!(rev.revised.paragraphs[0].text.contains("Market rose 12%"));
        assert!(rev.changelog.contains(&ch_id));
        assert!(rev.changelog.contains("Fixed"));
        assert!(rev.diff.contains("Market rose 12%") || rev.diff.contains("+"));
    }

    #[test]
    fn factcheck_revision_rebut_and_open_question_keep_text() {
        let report = Report::new("Test", vec![para("p1", "Some claim.", vec![])]);
        let challenges = vec![Challenge::new("ch1", "p1", ChallengeKind::WeakReasoning, "vague")];
        let rebut = Resolution::rebut("ch1", "Evidence in source s1 line 5");
        let rev = revise_report(&report, &challenges, &[rebut]).unwrap();
        assert_eq!(rev.revised.paragraphs[0].text, "Some claim.");
        assert!(rev.changelog.contains("Rebutted"));
        assert!(rev.changelog.contains("s1 line 5"));

        let open = Resolution::open_question("ch1", "Need more data on X");
        let rev2 = revise_report(&report, &challenges, &[open]).unwrap();
        assert!(rev2.changelog.contains("Open question"));
    }

    #[test]
    fn factcheck_revision_diff_shows_changes() {
        let report = Report::new("Orig", vec![para("p1", "Line one.", vec![])]);
        let challenges = vec![Challenge::new("ch1", "p1", ChallengeKind::UnsupportedClaim, "needs source")];
        let fix = Resolution::fix("ch1", "fix", "Line one revised with source.");
        let rev = revise_report(&report, &challenges, &[fix]).unwrap();
        assert!(rev.diff.contains("-Line one.") || rev.diff.contains("- Line one."));
        assert!(rev.diff.contains("+Line one revised"));
    }

    #[test]
    fn factcheck_revision_error_on_unknown_challenge() {
        let report = Report::new("T", vec![para("p1", "text", vec![])]);
        let challenges = vec![Challenge::new("ch1", "p1", ChallengeKind::UnsupportedClaim, "x")];
        let bad = Resolution::fix("ch999", "note", "new text");
        let err = revise_report(&report, &challenges, &[bad]).unwrap_err();
        assert!(err.to_string().contains("ch999"));
    }

    #[test]
    fn factcheck_revision_changelog_lists_all_resolutions() {
        let report = Report::new(
            "T",
            vec![
                para("p1", "claim one with 42% growth", vec![]),
                para("p2", "claim two", vec![("s1", "missing quote")]),
            ],
        );
        let sources = vec![src("s1", "unrelated")];
        let challenges = critique_report(&report, &sources);
        assert!(challenges.len() >= 2);
        let resolutions: Vec<Resolution> = challenges
            .iter()
            .map(|c| match c.kind {
                ChallengeKind::UnsupportedClaim => Resolution::fix(c.id.clone(), "added evidence", "revised text"),
                _ => Resolution::open_question(c.id.clone(), "needs follow-up"),
            })
            .collect();
        let rev = revise_report(&report, &challenges, &resolutions).unwrap();
        assert!(rev.changelog.lines().count() >= challenges.len());
    }
}
