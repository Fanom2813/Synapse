//! Memory recall for agent turns (docs/MEMORY.md §5).
//!
//! Pure, store-agnostic context building: the store crate maps its `Memory`
//! rows to [`MemoryView`] and calls in here. Per turn, inside a fixed
//! **memory budget** (default 1,500 tokens ≈ 6,000 chars):
//!
//! 1. **Core block**, always included: pinned memories first, then the top
//!    user-profile facts, then this project's decisions/facts, then this
//!    agent's notes and this project's run digests. The block stops when the
//!    budget is spent (lead re-summarises when it grows past it).
//! 2. **Retrieved block**: scored keyword search over the same visible rows
//!    (memories, run digests, the agent's notes) using the current task as
//!    the query; top-k after de-duplication.
//!
//! Superseded facts (`invalid_at.is_some()`) are excluded everywhere —
//! history stays in the store, never in context. Every recalled item carries
//! its id, so the UI can show "used N memories" on a message (see
//! [`MemoryContext::attribution_line`] and `Envelope::memory_ids`).
//!
//! Agents can search further with the [`RECALL_TOOL`] (`recall`) and
//! [`SEARCH_ARCHIVE_TOOL`] (`search_archive`) tools (specs from
//! [`memory_tool_specs`], pure search in [`recall_search`] /
//! [`archive_search`]).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::team_tools::TeamToolSpec;

/// Default per-turn memory budget (MEMORY.md §5: 1,500 tokens).
pub const DEFAULT_MEMORY_BUDGET_TOKENS: usize = 1500;
/// Rough chars-per-token for budget accounting.
pub const CHARS_PER_TOKEN: usize = 4;
/// Default top-k for the retrieved block.
pub const DEFAULT_RETRIEVED_TOP_K: usize = 5;

/// `recall(query, scope?)`: search active memories (user / project / agent
/// notes / digests) beyond what is already in context.
pub const RECALL_TOOL: &str = "recall";
/// `search_archive(query)`: search older/archived content (run digests,
/// superseded history, artifacts index) that is not in the core block.
pub const SEARCH_ARCHIVE_TOOL: &str = "search_archive";
/// Memory tool names exposed to agents (function-calling + MCP).
pub const MEMORY_TOOL_NAMES: &[&str] = &[RECALL_TOOL, SEARCH_ARCHIVE_TOOL];

/// Store-agnostic view of one memory row. The store crate builds this from
/// its `Memory` (including `invalid_at` for supersession history).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryView {
    pub id: String,
    pub scope: String,
    pub scope_id: String,
    pub kind: String,
    pub text: String,
    pub source_message_id: String,
    pub created_by: String,
    pub confidence: f64,
    pub pinned: bool,
    /// `Some(ts)` once superseded/retracted (history only, never recalled).
    pub invalid_at: Option<i64>,
}

impl MemoryView {
    pub fn new(id: &str, scope: &str, scope_id: &str, kind: &str, text: &str) -> Self {
        Self {
            id: id.into(),
            scope: scope.into(),
            scope_id: scope_id.into(),
            kind: kind.into(),
            text: text.into(),
            source_message_id: String::new(),
            created_by: String::new(),
            confidence: 1.0,
            pinned: false,
            invalid_at: None,
        }
    }

    pub fn pinned(mut self) -> Self {
        self.pinned = true;
        self
    }

    pub fn superseded(mut self, at: i64) -> Self {
        self.invalid_at = Some(at);
        self
    }

    /// Active rows only: superseded facts stay in history, never in context.
    pub fn is_active(&self) -> bool {
        self.invalid_at.is_none()
    }
}

/// One memory placed in context, with its id for UI attribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RecalledMemory {
    pub id: String,
    pub scope: String,
    pub kind: String,
    pub text: String,
    pub pinned: bool,
    pub score: f64,
}

impl RecalledMemory {
    /// One-line rendering with the id the UI links back (`[mem:<id>]`).
    pub fn render(&self) -> String {
        format!("[mem:{}] {}", self.id, self.text)
    }
}

/// One archived item searchable via `search_archive` (old digests,
/// superseded versions, artifact index entries).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveItem {
    pub id: String,
    pub title: String,
    pub text: String,
}

impl ArchiveItem {
    pub fn new(id: &str, title: &str, text: &str) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            text: text.into(),
        }
    }
}

/// One archive hit, with its id for UI attribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveHit {
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

/// The memory part of one agent turn's context: core + retrieved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MemoryContext {
    pub core: Vec<RecalledMemory>,
    pub retrieved: Vec<RecalledMemory>,
}

impl MemoryContext {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.core.is_empty() && self.retrieved.is_empty()
    }

    pub fn len(&self) -> usize {
        self.core.len() + self.retrieved.len()
    }

    /// Every memory id that informed the turn (core first, de-duplicated).
    /// Stamped onto the published `Envelope::memory_ids` so the UI can show
    /// "used N memories" with links.
    pub fn used_ids(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for m in self.core.iter().chain(self.retrieved.iter()) {
            if seen.insert(m.id.clone()) {
                out.push(m.id.clone());
            }
        }
        out
    }

    /// Prompt rendering: stable core block first, then retrieved.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if !self.core.is_empty() {
            out.push_str("# Core memory (always in context)\n");
            for m in &self.core {
                out.push_str("- ");
                out.push_str(&m.render());
                out.push('\n');
            }
        }
        if !self.retrieved.is_empty() {
            out.push_str("# Recalled memories (relevant to this task)\n");
            for m in &self.retrieved {
                out.push_str("- ");
                out.push_str(&m.render());
                out.push('\n');
            }
        }
        out
    }

    /// "used 3 memories" line for the UI, with linkable ids.
    pub fn attribution_line(ids: &[String]) -> String {
        match ids.len() {
            0 => String::new(),
            1 => format!("used 1 memory: {}", ids[0]),
            n => format!("used {n} memories: {}", ids.join(", ")),
        }
    }
}

/// Rough token estimate for budget accounting.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

/// Budget in chars for a token budget.
pub fn budget_chars(budget_tokens: usize) -> usize {
    budget_tokens * CHARS_PER_TOKEN
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tokens(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.to_string())
        .collect()
}

/// Relevance of `text` to `query`: exact/substring boost, else token overlap.
fn relevance(query: &str, text: &str) -> f64 {
    let (q, t) = (query.to_lowercase(), text.to_lowercase());
    if q.is_empty() || t.is_empty() {
        return 0.0;
    }
    if q == t {
        return 1.0;
    }
    if t.contains(q.as_str()) || q.contains(t.as_str()) {
        return 0.9;
    }
    let (tq, tt) = (tokens(&q), tokens(&t));
    if tq.is_empty() || tt.is_empty() {
        return 0.0;
    }
    let inter = tq.intersection(&tt).count() as f64;
    let union = tq.union(&tt).count() as f64;
    inter / union
}

/// Rows this agent may see: user profile (all), this project (incl. its
/// digests), and its own agent notes. Nothing from other projects.
fn is_visible(m: &MemoryView, project_id: &str, agent_id: &str) -> bool {
    match m.scope.as_str() {
        "user" => true,
        "project" | "digest" => m.scope_id == project_id,
        "agent" => m.scope_id == agent_id,
        _ => false,
    }
}

/// Active, visible rows (superseded facts excluded).
pub fn active_memories<'a>(
    all: &'a [MemoryView],
    project_id: &str,
    agent_id: &str,
) -> Vec<&'a MemoryView> {
    all.iter()
        .filter(|m| m.is_active() && is_visible(m, project_id, agent_id))
        .collect()
}

/// Rank for the core block: pinned first, then user facts, then project
/// decisions/facts, then agent notes, then digests.
fn core_rank(m: &MemoryView) -> u8 {
    if m.pinned {
        return 0;
    }
    match m.scope.as_str() {
        "user" => 1,
        "project" if m.kind == "decision" => 2,
        "project" => 3,
        "agent" => 4,
        _ => 5,
    }
}

/// Core block (MEMORY.md §5.1): pinned + top user facts + project goal,
/// decisions and open questions, bounded by `budget_tokens`. Lowest-priority
/// rows are dropped first when the budget is spent.
pub fn build_core_block(
    all: &[MemoryView],
    project_id: &str,
    agent_id: &str,
    budget_tokens: usize,
) -> Vec<RecalledMemory> {
    let mut visible = active_memories(all, project_id, agent_id);
    visible.sort_by_key(|m| (core_rank(m), m.id.clone()));
    let mut budget = budget_chars(budget_tokens.max(16));
    let mut out = Vec::new();
    for m in visible {
        let text = normalize(&m.text);
        if text.is_empty() {
            continue;
        }
        let cost = text.len() + 16;
        if cost > budget {
            continue;
        }
        budget -= cost;
        out.push(RecalledMemory {
            id: m.id.clone(),
            scope: m.scope.clone(),
            kind: m.kind.clone(),
            text,
            pinned: m.pinned,
            score: 1.0 - f64::from(core_rank(m)) * 0.1,
        });
    }
    out
}

/// Retrieved block (MEMORY.md §5.2): top-`top_k` visible rows scored against
/// `query`, de-duplicated by text; superseded facts excluded. Ids already in
/// `exclude_ids` (the core block) are skipped.
pub fn retrieve(
    query: &str,
    all: &[MemoryView],
    project_id: &str,
    agent_id: &str,
    exclude_ids: &HashSet<String>,
    top_k: usize,
) -> Vec<RecalledMemory> {
    if query.trim().is_empty() || top_k == 0 {
        return Vec::new();
    }
    let mut scored: Vec<(&MemoryView, f64)> = active_memories(all, project_id, agent_id)
        .into_iter()
        .filter(|m| !exclude_ids.contains(&m.id))
        .filter_map(|m| {
            // Base relevance must be non-zero: the confidence/pinned bonuses
            // below never promote an unrelated row into context.
            let rel = relevance(query, &m.text);
            if rel <= 0.0 {
                return None;
            }
            let score = rel + if m.pinned { 0.05 } else { 0.0 } + m.confidence * 0.01;
            Some((m, score))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    let mut seen_texts = HashSet::new();
    let mut out = Vec::new();
    for (m, score) in scored {
        if out.len() >= top_k {
            break;
        }
        if !seen_texts.insert(normalize(&m.text).to_lowercase()) {
            continue;
        }
        out.push(RecalledMemory {
            id: m.id.clone(),
            scope: m.scope.clone(),
            kind: m.kind.clone(),
            text: normalize(&m.text),
            pinned: m.pinned,
            score,
        });
    }
    out
}

/// Full per-turn memory context: bounded core block + top-k retrieved.
pub fn build_memory_context(
    query: &str,
    all: &[MemoryView],
    project_id: &str,
    agent_id: &str,
    budget_tokens: usize,
    top_k: usize,
) -> MemoryContext {
    let core = build_core_block(all, project_id, agent_id, budget_tokens);
    let exclude: HashSet<String> = core.iter().map(|m| m.id.clone()).collect();
    let retrieved = retrieve(query, all, project_id, agent_id, &exclude, top_k);
    MemoryContext { core, retrieved }
}

/// `recall(query, scope?)` tool implementation (pure): scored search over
/// active memories. `scope` optionally narrows to `user` / `project` /
/// `agent` / `digest` (empty = all visible). Superseded facts excluded.
pub fn recall_search(
    query: &str,
    scope: &str,
    all: &[MemoryView],
    project_id: &str,
    agent_id: &str,
    top_k: usize,
) -> Vec<RecalledMemory> {
    let exclude = HashSet::new();
    let mut hits = retrieve(query, all, project_id, agent_id, &exclude, top_k.max(1));
    if !scope.trim().is_empty() {
        hits.retain(|m| m.scope == scope);
    }
    hits.truncate(top_k.max(1));
    hits
}

/// `search_archive(query)` tool implementation (pure): scored search over
/// archived items (old digests, superseded history, artifact index).
pub fn archive_search(query: &str, archive: &[ArchiveItem], top_k: usize) -> Vec<ArchiveHit> {
    if query.trim().is_empty() || top_k == 0 {
        return Vec::new();
    }
    let mut scored: Vec<(&ArchiveItem, f64)> = archive
        .iter()
        .map(|a| {
            let score = relevance(query, &format!("{} {}", a.title, a.text));
            (a, score)
        })
        .filter(|(_, s)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    scored
        .into_iter()
        .take(top_k)
        .map(|(a, score)| {
            let snippet: String = a.text.chars().take(280).collect();
            ArchiveHit {
                id: a.id.clone(),
                title: a.title.clone(),
                snippet,
                score,
            }
        })
        .collect()
}

fn str_prop(desc: &str) -> serde_json::Value {
    serde_json::Value::Object(
        [(
            "description".to_string(),
            serde_json::Value::String(desc.to_string()),
        )]
        .into_iter()
        .collect(),
    )
}

fn schema(props: &[(&str, &str, serde_json::Value)], required: &[&str]) -> serde_json::Value {
    let properties: serde_json::Map<String, serde_json::Value> = props
        .iter()
        .map(|(name, kind, extra)| {
            let mut obj = serde_json::Map::new();
            obj.insert(
                "type".to_string(),
                serde_json::Value::String(kind.to_string()),
            );
            if let serde_json::Value::Object(map) = extra {
                for (k, v) in map {
                    obj.insert(k.clone(), v.clone());
                }
            }
            (name.to_string(), serde_json::Value::Object(obj))
        })
        .collect();
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// Function-calling specs for the memory tools agents can call mid-turn.
pub fn memory_tool_specs() -> Vec<TeamToolSpec> {
    vec![
        TeamToolSpec {
            name: RECALL_TOOL.into(),
            description: "Search remembered facts, preferences, decisions and lessons relevant to a query. Returns ids to cite.".into(),
            parameters: schema(
                &[
                    ("query", "string", str_prop("What to look up in memory")),
                    (
                        "scope",
                        "string",
                        str_prop("Narrow to one scope: user, project, agent, digest (empty = all)"),
                    ),
                ],
                &["query"],
            ),
        },
        TeamToolSpec {
            name: SEARCH_ARCHIVE_TOOL.into(),
            description: "Search the archive: old run digests and superseded memory versions not in context.".into(),
            parameters: schema(
                &[(
                    "query",
                    "string",
                    str_prop("What to look up in the archive"),
                )],
                &["query"],
            ),
        },
    ]
}

/// Validate a `recall` call's args (`query` required, `scope` optional).
pub fn parse_recall_args(args: &serde_json::Value) -> Result<(String, String), String> {
    match args.get("query").and_then(serde_json::Value::as_str) {
        Some(q) if !q.trim().is_empty() => Ok((
            q.to_string(),
            args.get("scope")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )),
        _ => Err("recall needs \"query\": what to look up in memory".to_string()),
    }
}

/// Validate a `search_archive` call's args (`query` required).
pub fn parse_archive_args(args: &serde_json::Value) -> Result<String, String> {
    match args.get("query").and_then(serde_json::Value::as_str) {
        Some(q) if !q.trim().is_empty() => Ok(q.to_string()),
        _ => Err("search_archive needs \"query\": what to look up".to_string()),
    }
}

#[cfg(test)]
mod memory_recall_tests {
    use super::*;

    fn sample() -> Vec<MemoryView> {
        vec![
            MemoryView::new(
                "u1",
                "user",
                "",
                "preference",
                "prefers sources from 2025 onwards",
            ),
            MemoryView::new("u2", "user", "", "fact", "works in fintech").pinned(),
            MemoryView::new("p1", "project", "p1", "decision", "team chose sqlite-vec"),
            MemoryView::new("p2", "project", "p1", "fact", "deadline is in June"),
            MemoryView::new(
                "old",
                "project",
                "p1",
                "fact",
                "deadline was in March, moved to June",
            )
            .superseded(1700000001),
            MemoryView::new(
                "n1",
                "agent",
                "ada",
                "lesson",
                "Scholar search works better here",
            ),
            MemoryView::new(
                "d1",
                "digest",
                "p1",
                "digest",
                "run 7 found three papers on vec search",
            ),
            // Another project's rows must never leak in.
            MemoryView::new(
                "x1",
                "project",
                "p2",
                "decision",
                "other project secret plan",
            ),
            MemoryView::new("x2", "agent", "bob", "lesson", "bob private note"),
        ]
    }

    #[test]
    fn memory_recall_core_block_is_bounded_and_pinned_first() {
        let all = sample();
        let core = build_core_block(&all, "p1", "ada", DEFAULT_MEMORY_BUDGET_TOKENS);
        let ids: Vec<&str> = core.iter().map(|m| m.id.as_str()).collect();
        // Pinned first, superseded excluded, other projects excluded.
        assert_eq!(ids[0], "u2");
        assert!(ids.contains(&"u1"));
        assert!(ids.contains(&"p1"));
        assert!(!ids.contains(&"old"), "superseded facts stay in history");
        assert!(!ids.contains(&"x1"), "project memory must not leak");
        assert!(!ids.contains(&"x2"), "other agents' notes are private");
        // Bounded: tiny budget keeps only the head of the block.
        let tiny = build_core_block(&all, "p1", "ada", 8);
        assert!(tiny.len() < core.len());
        assert_eq!(tiny[0].id, "u2");
        let chars: usize = tiny.iter().map(|m| m.text.len() + 16).sum();
        assert!(chars <= budget_chars(8) + 64, "chars={chars}");
        // Every item carries its id for UI attribution.
        assert!(core
            .iter()
            .all(|m| !m.id.is_empty() && !m.render().is_empty()));
    }

    #[test]
    fn memory_recall_retrieved_excludes_superseded_and_dedups() {
        let mut all = sample();
        all.push(MemoryView::new(
            "p3",
            "project",
            "p1",
            "fact",
            "team chose sqlite-vec",
        ));
        // Small core budget so sqlite rows fall out of the core block and
        // must be found by retrieval instead.
        let core = build_core_block(&all, "p1", "ada", 10);
        let exclude: HashSet<String> = core.iter().map(|m| m.id.clone()).collect();
        let hits = retrieve("sqlite-vec embeddings", &all, "p1", "ada", &exclude, 5);
        assert!(!hits.is_empty());
        // Core ids are not repeated in the retrieved block.
        assert!(hits.iter().all(|m| !exclude.contains(&m.id)));
        // Superseded rows and other projects never retrieved.
        assert!(hits.iter().all(|m| m.id != "old" && m.id != "x1"));
        // Near-duplicate texts de-duplicated to one hit.
        let dupes = hits
            .iter()
            .filter(|m| m.text.to_lowercase().contains("sqlite-vec"))
            .count();
        assert_eq!(dupes, 1, "got: {hits:?}");
        // Empty query / zero k retrieve nothing.
        assert!(retrieve("", &all, "p1", "ada", &HashSet::new(), 5).is_empty());
        assert!(retrieve("sqlite", &all, "p1", "ada", &HashSet::new(), 0).is_empty());
    }

    #[test]
    fn memory_recall_context_combines_core_and_retrieved_with_ids() {
        let all = sample();
        let ctx = build_memory_context(
            "which vector library did we choose?",
            &all,
            "p1",
            "ada",
            DEFAULT_MEMORY_BUDGET_TOKENS,
            DEFAULT_RETRIEVED_TOP_K,
        );
        assert!(!ctx.core.is_empty());
        let ids = ctx.used_ids();
        assert!(ids.contains(&"p1".to_string()), "got: {ids:?}");
        assert_eq!(ids.len(), ctx.len());
        // Render keeps the [mem:id] links the UI uses.
        let rendered = ctx.render();
        assert!(rendered.contains("# Core memory"));
        assert!(rendered.contains("[mem:p1]"));
        // Attribution line the UI shows on the message.
        assert_eq!(MemoryContext::attribution_line(&[]), "");
        assert_eq!(
            MemoryContext::attribution_line(&["p1".to_string()]),
            "used 1 memory: p1"
        );
        assert_eq!(
            MemoryContext::attribution_line(&["a".to_string(), "b".to_string()]),
            "used 2 memories: a, b"
        );
    }

    #[test]
    fn memory_recall_tools_search_scopes_and_archive() {
        let all = sample();
        // recall tool: scoped search over active memories.
        let hits = recall_search("fintech", "", &all, "p1", "ada", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "u2");
        let scoped = recall_search("team chose", "project", &all, "p1", "ada", 5);
        assert!(scoped.iter().all(|m| m.scope == "project"));
        assert!(scoped.iter().any(|m| m.id == "p1"));
        // Superseded facts are not recallable, but are archive-searchable.
        let gone = recall_search("March", "", &all, "p1", "ada", 5);
        assert!(gone.iter().all(|m| m.id != "old"), "got: {gone:?}");
        let archive = vec![
            ArchiveItem::new("a1", "run 3 digest", "deadline was in March, moved to June"),
            ArchiveItem::new("a2", "run 2 digest", "unrelated brainstorming notes"),
        ];
        let found = archive_search("March deadline", &archive, 5);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "a1");
        assert!(!found[0].snippet.is_empty());
        assert!(archive_search("", &archive, 5).is_empty());
        assert!(archive_search("March", &archive, 0).is_empty());
        // Specs: both tools documented with schemas.
        let specs = memory_tool_specs();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, MEMORY_TOOL_NAMES);
        for spec in &specs {
            assert!(!spec.description.is_empty());
            assert!(spec.parameters.get("properties").is_some());
        }
        // Arg validation.
        assert!(parse_recall_args(&serde_json::json!({})).is_err());
        assert_eq!(
            parse_recall_args(&serde_json::json!({"query": "x", "scope": "user"})),
            Ok(("x".to_string(), "user".to_string()))
        );
        assert!(parse_archive_args(&serde_json::json!({})).is_err());
        assert_eq!(
            parse_archive_args(&serde_json::json!({"query": "y"})),
            Ok("y".to_string())
        );
    }

    #[test]
    fn memory_recall_active_memories_respect_scope_visibility() {
        let all = sample();
        let ids: Vec<&str> = active_memories(&all, "p1", "ada")
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        for want in ["u1", "u2", "p1", "p2", "n1", "d1"] {
            assert!(ids.contains(&want), "missing {want} in {ids:?}");
        }
        for banned in ["old", "x1", "x2"] {
            assert!(!ids.contains(&banned), "leaked {banned} in {ids:?}");
        }
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(budget_chars(1500), 6000);
    }
}
