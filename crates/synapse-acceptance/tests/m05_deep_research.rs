//! M5 deep-research acceptance test (see docs/DEEP-RESEARCH.md).
//!
//! Offline end-to-end through the real `synapse-research` crate with its
//! mocks: research loop (search → read → follow-up lead, findings cite
//! sources), perspective/outline creation, report critique + citation
//! checking, and revision with a changelog. No network, no credentials.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use synapse_research::factcheck::{
    critique_report, fact_check_report, revise_report, ChallengeKind, Citation, Paragraph, Report,
    Resolution, SourceDoc,
};
use synapse_research::perspectives::{
    assemble_outline, conduct_qa, identify_perspectives, map_perspectives_to_members,
    MockPerspectiveLlm, TeamMember,
};
use synapse_research::research_loop::{
    ExtractOutput, Finding, Learning, MockLlm, MockReader, PageReader, ResearchConfig, ResearchLlm,
    ResearchLoop,
};
use synapse_research::{MockSearch, SearchHit, SearchProvider};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn hit(url: &str) -> SearchHit {
    SearchHit::new(url, format!("Title {url}"), "snippet")
}

fn finding_for(text: &str, url: &str) -> Finding {
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

// ---------------------------------------------------------------------------
// 1. research loop: search → read → follow-up lead, findings cite sources
// ---------------------------------------------------------------------------

#[tokio::test]
async fn m05_research_loop_search_read_follow_up_cites_sources() {
    let cfg = ResearchConfig {
        breadth: 2,
        depth: 2,
        max_searches: 10,
        max_time: Duration::from_secs(5),
        top_k_per_query: 1,
    };
    let hits = vec![
        hit("https://a.example/solar"),
        hit("https://b.example/solar"),
        hit("https://c.example/solar-followup"),
    ];
    let search = Arc::new(MockSearch::new("mock", hits)) as Arc<dyn SearchProvider>;
    let reader = Arc::new(MockReader::new().with_pages(vec![
        (
            "https://a.example/solar",
            "Solar A",
            "Solar output rose 12% in 2024.",
        ),
        (
            "https://b.example/solar",
            "Solar B",
            "Panel costs fell 8% year over year.",
        ),
        (
            "https://c.example/solar-followup",
            "Solar C",
            "Grid storage increased alongside solar growth.",
        ),
    ])) as Arc<dyn PageReader>;

    let llm = MockLlm::new();
    llm.push_queries(vec!["solar costs".into(), "solar output".into()]);
    llm.push_queries(vec!["solar storage follow-up".into()]);
    llm.push_extract(ExtractOutput {
        learnings: vec![Learning {
            text: "Solar output rose 12% in 2024.".into(),
            sources: vec!["https://a.example/solar".into()],
        }],
        follow_ups: vec!["solar storage follow-up".into()],
    });
    llm.push_extract(ExtractOutput {
        learnings: vec![Learning {
            text: "Panel costs fell 8% year over year.".into(),
            sources: vec!["https://b.example/solar".into()],
        }],
        follow_ups: vec![],
    });
    llm.push_extract(ExtractOutput {
        learnings: vec![Learning {
            text: "Grid storage increased alongside solar growth.".into(),
            sources: vec!["https://c.example/solar-followup".into()],
        }],
        follow_ups: vec![],
    });
    let llm = Arc::new(llm) as Arc<dyn ResearchLlm>;

    let lp = ResearchLoop::new(cfg, search, reader, llm);
    let finding = lp.run("solar energy trends").await;

    // loop ran searches, read pages, and followed the lead into depth 2
    assert!(finding.searches_used >= 3, "got {}", finding.searches_used);
    assert!(finding.learnings.len() >= 3, "got {:?}", finding.learnings);
    // every learning cites at least one source
    for l in &finding.learnings {
        assert!(!l.sources.is_empty(), "learning without source: {l:?}");
    }
    // findings cite the sources they visited
    assert!(finding
        .citations
        .contains(&"https://a.example/solar".to_string()));
    assert!(finding
        .citations
        .contains(&"https://c.example/solar-followup".to_string()));
    assert!(finding
        .visited
        .contains(&"https://c.example/solar-followup".to_string()));
    // follow-up learning made it into the findings
    assert!(finding
        .learnings
        .iter()
        .any(|l| l.text.contains("Grid storage")));
    assert!(finding.word_count() <= 320);
}

// ---------------------------------------------------------------------------
// 2. perspectives + sourced outline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn m05_perspectives_and_sourced_outline() {
    let llm = MockPerspectiveLlm::with_defaults();
    let perspectives = identify_perspectives(&llm, "solar energy trends", 5).await;
    assert!((4..=6).contains(&perspectives.len()));

    let members = vec![
        TeamMember::new("m1", "Alice", "Engineering"),
        TeamMember::new("m2", "Bob", "Economics"),
        TeamMember::new("m3", "Carol", "Policy"),
    ];
    let assignments = map_perspectives_to_members(&perspectives, &members);
    assert_eq!(assignments.len(), perspectives.len());

    // findings grouped per perspective, each with its own source
    let mut findings: HashMap<String, Vec<Finding>> = HashMap::new();
    for p in &perspectives {
        findings.insert(
            p.id.clone(),
            vec![finding_for(
                &format!("Study on {}: output rose 12%.", p.name),
                &format!("https://src.example/{}", p.id),
            )],
        );
    }
    let qa = conduct_qa(&assignments, &findings);
    assert!(!qa.is_empty());
    for ex in &qa {
        assert!(!ex.answer.is_empty());
        assert!(!ex.sources.is_empty(), "QA answer without sources: {ex:?}");
    }

    let outline = assemble_outline("solar energy trends", None, &assignments, &findings, &qa);
    assert_eq!(outline.sections.len(), perspectives.len());
    assert!(outline.title.contains("solar energy trends"));
    for sec in &outline.sections {
        assert!(!sec.perspective_ids.is_empty());
        assert!(
            !sec.key_questions.is_empty(),
            "section {} missing questions",
            sec.id
        );
        assert!(
            !sec.source_refs.is_empty(),
            "section {} missing sources",
            sec.id
        );
        assert!(!sec.key_points.is_empty());
        assert!(!sec.contributor_member_ids.is_empty());
    }
    let mut sorted = outline.all_sources.clone();
    sorted.sort();
    assert_eq!(sorted, outline.all_sources);
    assert!(outline.all_sources.len() >= perspectives.len());
}

// ---------------------------------------------------------------------------
// 3. critique + citation check + revision with changelog
// ---------------------------------------------------------------------------

#[test]
fn m05_report_critique_citation_check_and_revision() {
    let sources = vec![
        SourceDoc::new(
            "s1",
            "Solar report",
            "https://src.example/s1",
            "Solar output rose 12% in 2024 according to grid data.",
        ),
        SourceDoc::new(
            "s2",
            "Cost survey",
            "https://src.example/s2",
            "Panel costs fell 8% year over year in the survey.",
        ),
    ];
    let report = Report::new(
        "Solar trends",
        vec![
            Paragraph::new(
                "p1",
                "Solar output grew last year.",
                vec![Citation::new("s1", "rose 12%")],
            ),
            Paragraph::new(
                "p2",
                "The market increased by 40% last year and will triple next year according to analysts.",
                vec![],
            ),
            Paragraph::new(
                "p3",
                "Costs moved too.",
                vec![Citation::new("s2", "invented quote not in source")],
            ),
        ],
    )
    .with_perspectives(
        vec!["Technical".into(), "Economic".into()],
        vec!["Technical".into()],
    );

    // citation check: p1 supported, p3 quote flagged
    let results = fact_check_report(&report, &sources);
    assert_eq!(results.len(), 2);
    let p1 = results.iter().find(|r| r.paragraph_id == "p1").unwrap();
    assert!(p1.supported, "got {p1:?}");
    let p3 = results.iter().find(|r| r.paragraph_id == "p3").unwrap();
    assert!(!p3.supported);

    // critique: unsupported claim (p2), quote not found (p3), missing perspective
    let challenges = critique_report(&report, &sources);
    assert!(
        challenges
            .iter()
            .any(|c| c.kind == ChallengeKind::UnsupportedClaim && c.paragraph_id == "p2"),
        "got {challenges:?}"
    );
    assert!(challenges
        .iter()
        .any(|c| c.kind == ChallengeKind::QuoteNotFound));
    assert!(challenges
        .iter()
        .any(|c| c.kind == ChallengeKind::MissingPerspective));

    // revision: fix p2, rebut the quote challenge with evidence note, keep one open question
    let unsupported = challenges
        .iter()
        .find(|c| c.kind == ChallengeKind::UnsupportedClaim)
        .unwrap()
        .clone();
    let quote = challenges
        .iter()
        .find(|c| c.kind == ChallengeKind::QuoteNotFound)
        .unwrap()
        .clone();
    let missing = challenges
        .iter()
        .find(|c| c.kind == ChallengeKind::MissingPerspective)
        .unwrap()
        .clone();
    let resolutions = vec![
        Resolution::fix(
            unsupported.id.clone(),
            "Scoped the claim to the sourced figure.",
            "Solar output rose 12% in 2024 according to grid data.",
        ),
        Resolution::rebut(
            quote.id.clone(),
            "Quote wording matches source s2 line 1 after normalization review",
        ),
        Resolution::open_question(
            missing.id.clone(),
            "Need an economist to cover the Economic angle",
        ),
    ];
    let rev = revise_report(&report, &challenges, &resolutions).expect("revise");
    assert!(rev
        .revised
        .paragraphs
        .iter()
        .any(|p| p.text.contains("rose 12%")));
    assert!(rev.changelog.contains(&unsupported.id));
    assert!(rev.changelog.contains("Fixed"));
    assert!(rev.changelog.contains("Rebutted"));
    assert!(rev.changelog.contains("Open question"));
    assert!(!rev.diff.is_empty());
    assert!(rev.diff.contains('+'));
}
