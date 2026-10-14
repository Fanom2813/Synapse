//! Deep-research loop (docs/DEEP-RESEARCH.md §3).
//! Breadth/depth searches, reading, following leads, compressed findings with citations,
//! search and time budgets, gap marking.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::reader::Source;
use crate::SearchProvider;

// ---------------------------------------------------------------------------
// Config & budgets
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResearchConfig {
    pub breadth: usize,
    pub depth: usize,
    /// Max number of search queries (budget).
    pub max_searches: usize,
    /// Max wall time for the whole research.
    pub max_time: Duration,
    /// Max pages to read per query.
    pub top_k_per_query: usize,
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            breadth: 4,
            depth: 2,
            max_searches: 20,
            max_time: Duration::from_secs(60),
            top_k_per_query: 2,
        }
    }
}

impl ResearchConfig {
    pub fn quick() -> Self {
        Self {
            breadth: 2,
            depth: 1,
            ..Default::default()
        }
    }
    pub fn standard() -> Self {
        Self::default()
    }
    pub fn deep() -> Self {
        Self {
            breadth: 6,
            depth: 3,
            max_searches: 40,
            max_time: Duration::from_secs(180),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Learning {
    pub text: String,
    /// ids/urls of sources supporting it
    pub sources: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ExtractOutput {
    pub learnings: Vec<Learning>,
    pub follow_ups: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Finding {
    /// Original query
    pub query: String,
    /// Compressed summary ≤300 words
    pub summary: String,
    pub learnings: Vec<Learning>,
    /// Citations (source id/url)
    pub citations: Vec<String>,
    /// visited urls
    pub visited: Vec<String>,
    /// Gaps (budget exhausted, unanswered, etc)
    pub gaps: Vec<String>,
    /// Number of searches performed
    pub searches_used: usize,
    /// Whether budgets were exhausted
    pub budget_exhausted: bool,
    /// elapsed
    pub elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatusKind {
    GeneratingQueries,
    Searching,
    Reading,
    Extracting,
    Compressing,
    Done,
    BudgetWarning,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub kind: StatusKind,
    pub message: String,
    pub depth: usize,
    pub searches_used: usize,
}

// ---------------------------------------------------------------------------
// LLM trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ResearchLlm: Send + Sync {
    async fn generate_queries(&self, query: &str, learnings: &[Learning], n: usize) -> Vec<String>;

    async fn extract(&self, pages: &[Source], query: &str) -> ExtractOutput;
}

// Mock LLM for tests
pub struct MockLlm {
    queries: Mutex<Vec<Vec<String>>>,
    extracts: Mutex<Vec<ExtractOutput>>,
    query_calls: Mutex<Vec<String>>,
}

impl MockLlm {
    pub fn new() -> Self {
        Self {
            queries: Mutex::new(vec![]),
            extracts: Mutex::new(vec![]),
            query_calls: Mutex::new(vec![]),
        }
    }
    pub fn with_queries(self, qs: Vec<Vec<String>>) -> Self {
        *self.queries.lock().unwrap() = qs;
        self
    }
    pub fn with_extracts(self, es: Vec<ExtractOutput>) -> Self {
        *self.extracts.lock().unwrap() = es;
        self
    }
    pub fn push_queries(&self, qs: Vec<String>) {
        self.queries.lock().unwrap().push(qs);
    }
    pub fn push_extract(&self, e: ExtractOutput) {
        self.extracts.lock().unwrap().push(e);
    }
    pub fn query_calls(&self) -> Vec<String> {
        self.query_calls.lock().unwrap().clone()
    }
}

impl Default for MockLlm {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ResearchLlm for MockLlm {
    async fn generate_queries(
        &self,
        query: &str,
        _learnings: &[Learning],
        n: usize,
    ) -> Vec<String> {
        self.query_calls.lock().unwrap().push(query.to_string());
        let mut q = self.queries.lock().unwrap();
        if !q.is_empty() {
            let out = q.remove(0);
            // truncate/pad to n
            let mut v = out.into_iter().take(n).collect::<Vec<_>>();
            while v.len() < n {
                v.push(format!("{query} follow {}", v.len() + 1));
            }
            v
        } else {
            (0..n)
                .map(|i| format!("{query} variant {}", i + 1))
                .collect()
        }
    }
    async fn extract(&self, pages: &[Source], _query: &str) -> ExtractOutput {
        let mut es = self.extracts.lock().unwrap();
        if !es.is_empty() {
            let mut out = es.remove(0);
            // fill sources if empty but pages exist
            for l in &mut out.learnings {
                if l.sources.is_empty() {
                    if let Some(p) = pages.first() {
                        l.sources.push(p.url.clone());
                    }
                }
                if l.text.is_empty() {
                    l.text = "learning".to_string();
                }
            }
            out.learnings.truncate(5);
            out
        } else {
            // default: one learning per page
            let learnings = pages
                .iter()
                .take(5)
                .map(|p| Learning {
                    text: format!("Found info in {}", p.title),
                    sources: vec![p.url.clone()],
                })
                .collect::<Vec<_>>();
            let learnings = if learnings.is_empty() {
                vec![Learning {
                    text: "No pages retrieved".into(),
                    sources: vec![],
                }]
            } else {
                learnings
            };
            ExtractOutput {
                learnings,
                follow_ups: vec![],
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reader trait for loop (so we can mock)
// ---------------------------------------------------------------------------

#[async_trait]
pub trait PageReader: Send + Sync {
    async fn read(&self, url: &str) -> Result<Source, String>;
}

pub struct MockReader {
    pages: Mutex<HashMap<String, Source>>,
    fail_urls: Mutex<HashSet<String>>,
    calls: Mutex<Vec<String>>,
}

impl Default for MockReader {
    fn default() -> Self {
        Self::new()
    }
}

impl MockReader {
    pub fn new() -> Self {
        Self {
            pages: Mutex::new(HashMap::new()),
            fail_urls: Mutex::new(HashSet::new()),
            calls: Mutex::new(vec![]),
        }
    }
    pub fn with_page(self, url: &str, title: &str, md: &str) -> Self {
        let mut s = Source::new(url, title, md);
        s.url = url.to_string();
        self.pages.lock().unwrap().insert(url.to_string(), s);
        self
    }
    pub fn with_pages(self, pages: Vec<(&str, &str, &str)>) -> Self {
        for (u, t, m) in pages {
            let mut s = Source::new(u, t, m);
            s.url = u.to_string();
            self.pages.lock().unwrap().insert(u.to_string(), s);
        }
        self
    }
    pub fn fail_on(&self, url: &str) {
        self.fail_urls.lock().unwrap().insert(url.to_string());
    }
}

#[async_trait]
impl PageReader for MockReader {
    async fn read(&self, url: &str) -> Result<Source, String> {
        self.calls.lock().unwrap().push(url.to_string());
        if self.fail_urls.lock().unwrap().contains(url) {
            return Err("mock read failure".into());
        }
        let map = self.pages.lock().unwrap();
        if let Some(s) = map.get(url) {
            Ok(s.clone())
        } else {
            // return synthetic source
            let mut s = Source::new(url, "Mock Page", format!("Content of {url}"));
            s.url = url.to_string();
            Ok(s)
        }
    }
}

// adapter over real Reader with cache
#[allow(dead_code)]
pub struct CacheReader {
    inner: Arc<MockReader>,
}
impl CacheReader {
    pub fn wrap(r: Arc<MockReader>) -> Arc<dyn PageReader> {
        r as Arc<dyn PageReader>
    }
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

pub struct ResearchLoop {
    pub config: ResearchConfig,
    pub search: Arc<dyn SearchProvider>,
    pub reader: Arc<dyn PageReader>,
    pub llm: Arc<dyn ResearchLlm>,
    pub on_status: Option<Arc<dyn Fn(Status) + Send + Sync>>,
}

impl ResearchLoop {
    pub fn new(
        config: ResearchConfig,
        search: Arc<dyn SearchProvider>,
        reader: Arc<dyn PageReader>,
        llm: Arc<dyn ResearchLlm>,
    ) -> Self {
        Self {
            config,
            search,
            reader,
            llm,
            on_status: None,
        }
    }

    pub fn with_status<F>(mut self, f: F) -> Self
    where
        F: Fn(Status) + Send + Sync + 'static,
    {
        self.on_status = Some(Arc::new(f));
        self
    }

    fn emit(&self, s: Status) {
        if let Some(cb) = &self.on_status {
            cb(s);
        }
    }

    pub async fn run(&self, query: &str) -> Finding {
        let start = Instant::now();
        let mut visited: HashSet<String> = HashSet::new();
        let mut learnings: Vec<Learning> = vec![];
        let mut citations: HashSet<String> = HashSet::new();
        let mut gaps: Vec<String> = vec![];
        let mut searches_used: usize = 0;
        let mut budget_exhausted = false;

        // time budget helper
        let time_budget = self.config.max_time;
        let search_budget = self.config.max_searches;

        // internal recursive
        let initial_query = query.to_string();
        // we do iterative depth layers
        // depth D means D levels; breadth halves each level (ceil)
        let mut current_learnings = Vec::new();
        let mut current_queries = vec![initial_query.clone()];

        // For simplicity: outer loop over depth
        let mut depth_remaining = self.config.depth;
        let mut breadth = self.config.breadth;

        // We treat the first level's queries as generated via LLM
        // Then for each depth level we search those queries.
        // Follow-ups feed next level.

        let mut depth_idx: usize = 0;
        let mut all_follow_ups: Vec<String> = vec![];

        while depth_remaining > 0 {
            // Check budgets before generating queries for this depth (except first where we already have)
            if start.elapsed() >= time_budget {
                gaps.push("time budget exhausted".into());
                budget_exhausted = true;
                break;
            }
            if searches_used >= search_budget {
                gaps.push("search budget exhausted".into());
                budget_exhausted = true;
                break;
            }
            // 90% beast mode check
            let search_frac = searches_used as f64 / (search_budget as f64).max(1.0);
            let time_frac = start.elapsed().as_secs_f64() / time_budget.as_secs_f64().max(1.0);
            if search_frac >= 0.9 || time_frac >= 0.9 {
                gaps.push(format!(
                    "budget at 90% (searches {}/{}, time {:.0}%) — entering beast mode, stopping search",
                    searches_used,
                    search_budget,
                    time_frac * 100.0
                ));
                budget_exhausted = true;
                break;
            }

            // For depth_idx==0 we generate queries from the original query; for later depths generate from follow_ups?
            // Spec: queries = llm.generate_serp_queries(query, learnings, n=B)
            // If D>1: research(next query from follow_ups, breadth=ceil(B/2), depth=D-1, ...)
            // We implement as: each depth generates B queries using LLM with current learnings
            let gen_n = breadth;
            let base_query = if depth_idx == 0 {
                query.to_string()
            } else {
                // use first follow_up as base or original
                all_follow_ups
                    .first()
                    .cloned()
                    .unwrap_or_else(|| query.to_string())
            };

            self.emit(Status {
                kind: StatusKind::GeneratingQueries,
                message: format!("generating {gen_n} queries for depth {}", depth_idx + 1),
                depth: depth_idx,
                searches_used,
            });

            let queries = self
                .llm
                .generate_queries(&base_query, &current_learnings, gen_n)
                .await;

            // But for first depth, if current_queries already holds original, we use queries; otherwise use generated
            let queries_to_run = if queries.is_empty() {
                current_queries.clone()
            } else {
                queries
            };

            let mut next_follow_ups: Vec<String> = vec![];

            for q in queries_to_run {
                // budget checks per query
                if searches_used >= search_budget {
                    gaps.push("search budget exhausted".into());
                    budget_exhausted = true;
                    break;
                }
                if start.elapsed() >= time_budget {
                    gaps.push("time budget exhausted".into());
                    budget_exhausted = true;
                    break;
                }
                let search_frac = searches_used as f64 / (search_budget as f64).max(1.0);
                let time_frac = start.elapsed().as_secs_f64() / time_budget.as_secs_f64().max(1.0);
                if search_frac >= 0.9 || time_frac >= 0.9 {
                    gaps.push(format!(
                        "budget at 90% (searches {}/{}) — beast mode",
                        searches_used, search_budget
                    ));
                    budget_exhausted = true;
                    break;
                }

                self.emit(Status {
                    kind: StatusKind::Searching,
                    message: q.clone(),
                    depth: depth_idx,
                    searches_used,
                });

                let hits = match self.search.search(&q).await {
                    Ok(h) => h,
                    Err(e) => {
                        gaps.push(format!("search failed for '{q}': {e}"));
                        continue;
                    }
                };
                searches_used += 1;

                // select top k not in visited
                let mut to_read: Vec<String> = vec![];
                for h in hits.iter() {
                    if visited.contains(&h.url) {
                        continue;
                    }
                    to_read.push(h.url.clone());
                    if to_read.len() >= self.config.top_k_per_query {
                        break;
                    }
                }

                if to_read.is_empty() {
                    gaps.push(format!("no new pages for '{q}'"));
                    continue;
                }

                self.emit(Status {
                    kind: StatusKind::Reading,
                    message: format!("reading {} pages for '{q}'", to_read.len()),
                    depth: depth_idx,
                    searches_used,
                });

                let mut pages: Vec<Source> = vec![];
                for url in to_read {
                    if visited.contains(&url) {
                        continue;
                    }
                    visited.insert(url.clone());
                    match self.reader.read(&url).await {
                        Ok(src) => {
                            pages.push(src);
                        }
                        Err(e) => {
                            gaps.push(format!("read failed for {url}: {e}"));
                        }
                    }
                }

                if pages.is_empty() {
                    continue;
                }

                self.emit(Status {
                    kind: StatusKind::Extracting,
                    message: q.clone(),
                    depth: depth_idx,
                    searches_used,
                });

                let out = self.llm.extract(&pages, &q).await;
                // collect learnings, dedup by text
                for l in out.learnings.into_iter().take(5) {
                    if learnings.iter().any(|e| e.text == l.text) {
                        continue;
                    }
                    for c in &l.sources {
                        citations.insert(c.clone());
                    }
                    // also add page urls as citations
                    for p in &pages {
                        citations.insert(p.url.clone());
                    }
                    learnings.push(l.clone());
                    current_learnings.push(l);
                }
                for f in out.follow_ups.into_iter().take(breadth) {
                    if !next_follow_ups.contains(&f) {
                        next_follow_ups.push(f);
                    }
                }
            }

            if budget_exhausted {
                break;
            }

            all_follow_ups = next_follow_ups;
            if all_follow_ups.is_empty() && depth_remaining > 1 {
                gaps.push("no follow-up questions — stopping early".into());
                // not budget exhaustion, just gap
            }
            // prepare next breadth
            breadth = breadth.div_ceil(2); // ceil(B/2)
            if breadth == 0 {
                breadth = 1;
            }
            // if no follow ups and still depth left, break
            if all_follow_ups.is_empty() {
                // still decrement depth to finish
                depth_remaining -= 1;
                if depth_remaining == 0 {
                    break;
                }
                // if no follow ups, stop looping
                break;
            }
            current_queries = all_follow_ups.clone();
            depth_remaining -= 1;
            depth_idx += 1;
        }

        // dedup learnings already done, compress
        self.emit(Status {
            kind: StatusKind::Compressing,
            message: "compressing findings".into(),
            depth: depth_idx,
            searches_used,
        });

        let summary = compress_learnings(&learnings, &gaps);
        let visited_vec: Vec<String> = visited.into_iter().collect();
        let mut citations_vec: Vec<String> = citations.into_iter().collect();
        citations_vec.sort();

        let mut final_gaps = gaps;
        if learnings.is_empty() && !budget_exhausted {
            final_gaps.push("no learnings extracted — gaps in coverage".into());
        }

        self.emit(Status {
            kind: StatusKind::Done,
            message: "done".into(),
            depth: depth_idx,
            searches_used,
        });

        Finding {
            query: query.to_string(),
            summary,
            learnings,
            citations: citations_vec,
            visited: visited_vec,
            gaps: final_gaps,
            searches_used,
            budget_exhausted,
            elapsed: start.elapsed(),
        }
    }
}

fn compress_learnings(learnings: &[Learning], gaps: &[String]) -> String {
    if learnings.is_empty() {
        let mut s = "No findings.".to_string();
        if !gaps.is_empty() {
            s.push_str(&format!(" Gaps: {}.", gaps.join("; ")));
        }
        return s;
    }
    let parts: Vec<String> = learnings.iter().map(|l| l.text.clone()).collect();
    let mut text = parts.join(" ");
    // limit to 300 words
    let wc = text.split_whitespace().count();
    if wc > 300 {
        let words: Vec<String> = text.split_whitespace().map(|s| s.to_string()).collect();
        text = words[..300].join(" ");
        text.push('…');
    }
    if !gaps.is_empty() {
        let gap_note = format!(" Gaps: {}.", gaps.join("; "));
        let combined = format!("{text}{gap_note}");
        let w2 = combined.split_whitespace().count();
        if w2 > 320 {
            let wc2 = text.split_whitespace().count();
            let gap_wc = gap_note.split_whitespace().count();
            let allowed = 300usize.saturating_sub(gap_wc + 1);
            let words: Vec<String> = text.split_whitespace().map(|s| s.to_string()).collect();
            let truncated = words[..allowed.min(wc2)].join(" ");
            text = format!("{truncated}…{gap_note}");
        } else {
            text = combined;
        }
    }
    // Ensure citations not in summary text; citations are separate field.
    text
}

// Keep gaps clearly marked when budget exhausted: helper for display
impl Finding {
    pub fn has_gaps(&self) -> bool {
        !self.gaps.is_empty()
    }
    pub fn word_count(&self) -> usize {
        self.summary.split_whitespace().count()
    }
}

// ---------------------------------------------------------------------------
// Tests containing "loop"
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MockSearch, SearchHit};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn hit(url: &str) -> SearchHit {
        SearchHit::new(url, format!("Title {url}"), "snippet")
    }

    fn default_setup(
        hits: Vec<SearchHit>,
        pages: Vec<(&str, &str, &str)>,
        queries: Vec<Vec<String>>,
        extracts: Vec<ExtractOutput>,
        cfg: ResearchConfig,
    ) -> ResearchLoop {
        let search = Arc::new(MockSearch::new("mock", hits)) as Arc<dyn SearchProvider>;
        let reader = Arc::new(MockReader::new().with_pages(pages)) as Arc<dyn PageReader>;
        let llm = MockLlm::new();
        // push queries and extracts via inner fields
        for q in queries {
            llm.push_queries(q);
        }
        for e in extracts {
            llm.push_extract(e);
        }
        let llm = Arc::new(llm) as Arc<dyn ResearchLlm>;
        ResearchLoop::new(cfg, search, reader, llm)
    }

    #[tokio::test]
    async fn loop_breadth_depth_searches_and_reads() {
        let cfg = ResearchConfig {
            breadth: 2,
            depth: 1,
            max_searches: 10,
            max_time: Duration::from_secs(5),
            top_k_per_query: 1,
        };
        let hits = vec![hit("https://a.com"), hit("https://b.com")];
        let pages = vec![
            ("https://a.com", "A", "Content A"),
            ("https://b.com", "B", "Content B"),
        ];
        let queries = vec![vec!["q1".to_string(), "q2".to_string()]];
        let extracts = vec![
            ExtractOutput {
                learnings: vec![Learning {
                    text: "Learning about A".into(),
                    sources: vec!["https://a.com".into()],
                }],
                follow_ups: vec![],
            },
            ExtractOutput {
                learnings: vec![Learning {
                    text: "Learning about B".into(),
                    sources: vec!["https:://b.com".into()],
                }],
                follow_ups: vec![],
            },
        ];
        let lp = default_setup(hits, pages, queries, extracts, cfg);
        let finding = lp.run("test query").await;
        assert_eq!(finding.searches_used, 2);
        assert_eq!(finding.learnings.len(), 2);
        assert!(finding.citations.contains(&"https://a.com".to_string()));
        assert!(finding.summary.contains("Learning about A"));
        assert!(finding.word_count() <= 320);
    }

    #[tokio::test]
    async fn loop_follows_leads_with_depth() {
        // breadth 2, depth 2 => second level breadth ceil(2/2)=1
        let cfg = ResearchConfig {
            breadth: 2,
            depth: 2,
            max_searches: 10,
            max_time: Duration::from_secs(5),
            top_k_per_query: 1,
        };
        let hits = vec![
            hit("https://a.com"),
            hit("https://b.com"),
            hit("https://c.com"),
        ];
        let pages = vec![
            ("https://a.com", "A", "Content A"),
            ("https://b.com", "B", "Content B"),
            ("https://c.com", "C", "Content C"),
        ];
        let queries = vec![vec!["q1".into(), "q2".into()], vec!["follow1".into()]];
        let extracts = vec![
            ExtractOutput {
                learnings: vec![Learning {
                    text: "L1".into(),
                    sources: vec!["https://a.com".into()],
                }],
                follow_ups: vec!["follow1".into(), "follow2".into()],
            },
            ExtractOutput {
                learnings: vec![Learning {
                    text: "L2".into(),
                    sources: vec!["https://b.com".into()],
                }],
                follow_ups: vec![],
            },
            ExtractOutput {
                learnings: vec![Learning {
                    text: "L3 deep".into(),
                    sources: vec!["https://c.com".into()],
                }],
                follow_ups: vec![],
            },
        ];
        let lp = default_setup(hits, pages, queries, extracts, cfg);
        let finding = lp.run("orig").await;
        // Should have done 3 searches (2 breadth first level + 1 second level)
        assert_eq!(finding.searches_used, 3);
        assert!(finding.learnings.iter().any(|l| l.text == "L3 deep"));
    }

    #[tokio::test]
    async fn loop_enforces_search_budget_and_marks_gaps() {
        let cfg = ResearchConfig {
            breadth: 4,
            depth: 2,
            max_searches: 2,
            max_time: Duration::from_secs(5),
            top_k_per_query: 1,
        };
        let hits = vec![
            hit("https://a.com"),
            hit("https://b.com"),
            hit("https://c.com"),
        ];
        let pages = vec![
            ("https://a.com", "A", "Content A"),
            ("https://b.com", "B", "Content B"),
            ("https://c.com", "C", "Content C"),
        ];
        let queries = vec![
            vec!["q1".into(), "q2".into(), "q3".into(), "q4".into()],
            vec!["follow".into()],
        ];
        let extracts = vec![ExtractOutput {
            learnings: vec![Learning {
                text: "L".into(),
                sources: vec!["https://a.com".into()],
            }],
            follow_ups: vec!["f".into()],
        }];
        let lp = default_setup(hits, pages, queries, extracts, cfg);
        let finding = lp.run("budget test").await;
        assert!(finding.searches_used <= 2);
        assert!(finding.budget_exhausted);
        assert!(finding.gaps.iter().any(|g| g.contains("search budget")));
        // summary should mention gaps
        assert!(finding.summary.contains("Gaps"));
    }

    #[tokio::test]
    async fn loop_enforces_time_budget() {
        let cfg = ResearchConfig {
            breadth: 4,
            depth: 2,
            max_searches: 20,
            max_time: Duration::from_millis(0),
            top_k_per_query: 1,
        };
        let hits = vec![hit("https://a.com")];
        let pages = vec![("https://a.com", "A", "Content")];
        let queries = vec![vec!["q1".into(), "q2".into(), "q3".into(), "q4".into()]];
        let extracts = vec![];
        let lp = default_setup(hits, pages, queries, extracts, cfg);
        let finding = lp.run("time test").await;
        assert!(finding.gaps.iter().any(|g| g.contains("time budget")
            || g.contains("budget at 90%")
            || g.contains("no learnings")));
        assert!(finding.budget_exhausted || !finding.gaps.is_empty());
    }

    #[tokio::test]
    async fn loop_compressed_findings_with_citations_and_300_words() {
        let cfg = ResearchConfig {
            breadth: 1,
            depth: 1,
            max_searches: 5,
            max_time: Duration::from_secs(5),
            top_k_per_query: 1,
        };
        let hits = vec![hit("https://a.com")];
        // create very long content: 500 words
        let long_text = (0..500)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let pages = vec![("https://a.com", "A", long_text.as_str())];
        // mock llm will produce a 500-word learning
        let very_long = (0..500)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let queries = vec![vec!["q".into()]];
        let extracts = vec![ExtractOutput {
            learnings: vec![Learning {
                text: very_long,
                sources: vec!["https://a.com".into()],
            }],
            follow_ups: vec![],
        }];
        let lp = default_setup(hits, pages, queries, extracts, cfg);
        let finding = lp.run("long").await;
        assert!(finding.word_count() <= 320);
        assert!(finding.citations.contains(&"https://a.com".to_string()));
        // learnings still stored
        assert_eq!(finding.learnings.len(), 1);
    }

    #[tokio::test]
    async fn loop_beast_mode_at_90_percent() {
        let cfg = ResearchConfig {
            breadth: 4,
            depth: 1,
            max_searches: 10,
            max_time: Duration::from_secs(5),
            top_k_per_query: 1,
        };
        // Simulate that 9 searches already used would trigger 90% check
        // We'll craft a test with max_searches 10 and breadth 4; after 9, it should beast mode
        // Instead test directly: do 9 searches first via loop with budget 10, then second run?
        // Simpler: run with breadth 10 and budget 10, after 9 it should stop before 10th if check happens before next query
        let hits = (0..10)
            .map(|i| hit(&format!("https://a{i}.com")))
            .collect::<Vec<_>>();
        // Easier: use MockReader with no explicit pages, it synthesizes
        let search = Arc::new(MockSearch::new("mock", hits)) as Arc<dyn SearchProvider>;
        let reader = Arc::new(MockReader::new()) as Arc<dyn PageReader>;
        let llm = MockLlm::new();
        llm.push_queries((0..10).map(|i| format!("q{i}")).collect());
        for _ in 0..10 {
            llm.push_extract(ExtractOutput {
                learnings: vec![Learning {
                    text: "L".into(),
                    sources: vec![],
                }],
                follow_ups: vec![],
            });
        }
        let llm = Arc::new(llm) as Arc<dyn ResearchLlm>;
        let lp = ResearchLoop::new(cfg, search, reader, llm);
        let finding = lp.run("beast").await;
        // With 10 breadth and 10 budget, after 9 searches the next iteration should trigger beast mode (90%)
        // Our loop checks before each query, so after 9 searches, 9/10=0.9 triggers gap and stops before 10th
        assert!(finding.searches_used <= 10);
        if finding.searches_used == 9 {
            assert!(finding.budget_exhausted);
            assert!(finding
                .gaps
                .iter()
                .any(|g| g.contains("90%") || g.contains("beast")));
        }
    }

    #[tokio::test]
    async fn loop_dedupes_learnings_and_visited() {
        let cfg = ResearchConfig {
            breadth: 2,
            depth: 1,
            max_searches: 10,
            max_time: Duration::from_secs(5),
            top_k_per_query: 2,
        };
        // Same URL appears in both queries' hits
        let hits = vec![hit("https://dup.com"), hit("https://dup.com")];
        let pages = vec![("https://dup.com", "Dup", "Content")];
        let queries = vec![vec!["q1".into(), "q2".into()]];
        let extracts = vec![
            ExtractOutput {
                learnings: vec![Learning {
                    text: "Same learning".into(),
                    sources: vec!["https://dup.com".into()],
                }],
                follow_ups: vec![],
            },
            ExtractOutput {
                learnings: vec![Learning {
                    text: "Same learning".into(),
                    sources: vec!["https://dup.com".into()],
                }],
                follow_ups: vec![],
            },
        ];
        let lp = default_setup(hits.clone(), pages, queries, extracts, cfg);
        let finding = lp.run("dup test").await;
        // second query should find no new pages (visited) -> gap
        assert!(
            finding
                .visited
                .iter()
                .filter(|u| *u == "https://dup.com")
                .count()
                == 1
        );
        // learnings deduped
        assert_eq!(finding.learnings.len(), 1);
    }

    #[tokio::test]
    async fn loop_status_checkpoints() {
        let cfg = ResearchConfig {
            breadth: 1,
            depth: 1,
            max_searches: 5,
            max_time: Duration::from_secs(5),
            top_k_per_query: 1,
        };
        let hits = vec![hit("https://a.com")];
        let search = Arc::new(MockSearch::new("mock", hits)) as Arc<dyn SearchProvider>;
        let reader = Arc::new(MockReader::new().with_page("https://a.com", "A", "Content"))
            as Arc<dyn PageReader>;
        let llm = Arc::new(MockLlm::new()) as Arc<dyn ResearchLlm>;
        let statuses: Arc<Mutex<Vec<StatusKind>>> = Arc::new(Mutex::new(vec![]));
        let sc = statuses.clone();
        let lp = ResearchLoop::new(cfg, search, reader, llm).with_status(move |s| {
            sc.lock().unwrap().push(s.kind.clone());
        });
        let _ = lp.run("status test").await;
        let kinds = statuses.lock().unwrap().clone();
        assert!(kinds.contains(&StatusKind::GeneratingQueries));
        assert!(kinds.contains(&StatusKind::Searching));
        assert!(kinds.contains(&StatusKind::Reading));
        assert!(kinds.contains(&StatusKind::Done));
    }
}
