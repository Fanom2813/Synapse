//! Sources panel state (see docs/UI.md §1 right panel "Sources",
//! docs/DEEP-RESEARCH.md §6, docs/ARCHITECTURE.md §3 `sources` table).
//!
//! Pure state: deduplicated sources with titles + credibility notes, which
//! agents used them, and citation previews. The GPUI view renders this;
//! tests drive the state without GPUI.

use std::collections::{BTreeMap, BTreeSet};

/// Minimal source input (mirrors `synapse-research::Source` / `sources` row
/// without pulling that crate in).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSource {
    pub id: String,
    pub url: String,
    pub title: String,
    pub credibility_note: String,
}

impl PanelSource {
    pub fn new(
        id: impl Into<String>,
        url: impl Into<String>,
        title: impl Into<String>,
        credibility_note: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            title: title.into(),
            credibility_note: credibility_note.into(),
        }
    }
}

/// One citation attached to a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelCitation {
    /// Source id (or raw URL when the message cites a URL directly).
    pub source_id: String,
    pub quote: String,
}

impl PanelCitation {
    pub fn new(source_id: impl Into<String>, quote: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            quote: quote.into(),
        }
    }
}

/// Minimal message input: who cited what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelMessage {
    pub id: String,
    pub agent: String,
    pub citations: Vec<PanelCitation>,
}

impl PanelMessage {
    pub fn new(id: impl Into<String>, agent: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            agent: agent.into(),
            citations: Vec::new(),
        }
    }

    pub fn with_citations(mut self, citations: Vec<PanelCitation>) -> Self {
        self.citations = citations;
        self
    }
}

/// One citation preview shown under a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationPreview {
    /// Message that cited the source (navigate here on click).
    pub message_id: String,
    /// Agent that wrote the citing message.
    pub agent: String,
    /// Quoted text (truncated for preview).
    pub quote: String,
}

/// One deduplicated source row for the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DedupedSource {
    /// Canonical id (first-seen source id in the group).
    pub id: String,
    pub url: String,
    pub title: String,
    pub credibility_note: String,
    /// All source ids merged into this row.
    pub merged_ids: Vec<String>,
    /// Sorted agent names that cited this source.
    pub used_by: Vec<String>,
    /// Citation previews in message order.
    pub citations: Vec<CitationPreview>,
}

impl DedupedSource {
    /// Short preview-friendly title (falls back to URL).
    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            &self.url
        } else {
            &self.title
        }
    }

    pub fn citation_count(&self) -> usize {
        self.citations.len()
    }
}

/// Normalize a URL for dedup: trim, drop fragment, drop trailing slash
/// (except root), strip common tracking params, lowercase host.
pub fn normalize_url(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // Drop fragment.
    if let Some(i) = s.find('#') {
        s.truncate(i);
    }
    // Strip common tracking query params (utm_*, fbclid, gclid).
    if let Some(q) = s.find('?') {
        let (base, query) = s.split_at(q);
        let kept: Vec<&str> = query[1..]
            .split('&')
            .filter(|p| {
                let k = p.split('=').next().unwrap_or("").to_ascii_lowercase();
                !(k.starts_with("utm_") || k == "fbclid" || k == "gclid")
            })
            .collect();
        s = if kept.is_empty() {
            base.to_string()
        } else {
            format!("{base}?{}", kept.join("&"))
        };
    }
    // Drop trailing slash (keep "https://host/").
    while s.ends_with('/') && !s.ends_with("://") {
        // Don't strip the "//" of the scheme or the single root slash.
        let without = s.trim_end_matches('/');
        let is_root = without.contains("://")
            && !without.split("://").nth(1).unwrap_or("").contains('/');
        if is_root {
            break;
        }
        s = without.to_string();
    }
    s
}

/// Owned sources-panel state.
#[derive(Debug, Default)]
pub struct SourcesPanel {
    sources: Vec<DedupedSource>,
    selected_id: Option<String>,
}

impl SourcesPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild the panel from sources + citing messages. Dedupes by
    /// normalized URL; citations referencing an unknown id/URL synthesize a
    /// placeholder row titled by URL.
    pub fn rebuild(&mut self, sources: &[PanelSource], messages: &[PanelMessage]) {
        // Group sources by normalized URL, preserving first-seen order.
        let mut order: Vec<String> = Vec::new();
        let mut groups: BTreeMap<String, Vec<&PanelSource>> = BTreeMap::new();
        for s in sources {
            let key = normalize_url(&s.url);
            if key.is_empty() {
                continue;
            }
            groups.entry(key.clone()).or_default().push(s);
            if !order.contains(&key) {
                order.push(key);
            }
        }
        // Alias: source id -> normalized URL key; also raw URL -> key.
        let mut id_to_key: BTreeMap<String, String> = BTreeMap::new();
        for (key, group) in &groups {
            for s in group {
                id_to_key.insert(s.id.clone(), key.clone());
                id_to_key.insert(s.url.trim().to_string(), key.clone());
            }
        }

        #[derive(Debug)]
        struct Row {
            id: String,
            url: String,
            title: String,
            credibility_note: String,
            merged_ids: Vec<String>,
            used_by: BTreeSet<String>,
            citations: Vec<CitationPreview>,
        }
        let mut rows: BTreeMap<String, Row> = BTreeMap::new();
        for key in &order {
            let group = &groups[key];
            let first = group[0];
            let title = group
                .iter()
                .find_map(|s| {
                    if s.title.trim().is_empty() {
                        None
                    } else {
                        Some(s.title.clone())
                    }
                })
                .unwrap_or_default();
            let note = group
                .iter()
                .find_map(|s| {
                    if s.credibility_note.trim().is_empty() {
                        None
                    } else {
                        Some(s.credibility_note.clone())
                    }
                })
                .unwrap_or_default();
            rows.insert(
                key.clone(),
                Row {
                    id: first.id.clone(),
                    url: first.url.clone(),
                    title,
                    credibility_note: note,
                    merged_ids: group.iter().map(|s| s.id.clone()).collect(),
                    used_by: BTreeSet::new(),
                    citations: Vec::new(),
                },
            );
        }

        // Attach citations in message order; unknown refs become placeholders.
        let mut placeholder_order: Vec<String> = Vec::new();
        for m in messages {
            for c in &m.citations {
                let key = id_to_key.get(&c.source_id).cloned().or_else(|| {
                    let trimmed = c.source_id.trim();
                    if trimmed.contains("://") || trimmed.starts_with("www.") {
                        Some(normalize_url(trimmed))
                    } else {
                        None
                    }
                });
                let Some(key) = key else { continue };
                if !rows.contains_key(&key) && !placeholder_order.contains(&key) {
                    placeholder_order.push(key.clone());
                }
                let row = rows.entry(key.clone()).or_insert_with(|| Row {
                    id: c.source_id.clone(),
                    url: c.source_id.clone(),
                    title: String::new(),
                    credibility_note: String::new(),
                    merged_ids: vec![c.source_id.clone()],
                    used_by: BTreeSet::new(),
                    citations: Vec::new(),
                });
                if !m.agent.trim().is_empty() {
                    row.used_by.insert(m.agent.clone());
                }
                row.citations.push(CitationPreview {
                    message_id: m.id.clone(),
                    agent: m.agent.clone(),
                    quote: preview_quote(&c.quote),
                });
            }
        }

        let mut out: Vec<DedupedSource> = Vec::new();
        for key in order.into_iter().chain(placeholder_order) {
            if let Some(r) = rows.remove(&key) {
                out.push(DedupedSource {
                    id: r.id,
                    url: r.url,
                    title: r.title,
                    credibility_note: r.credibility_note,
                    merged_ids: r.merged_ids,
                    used_by: r.used_by.into_iter().collect(),
                    citations: r.citations,
                });
            }
        }
        // Keep selection only if it still exists.
        if let Some(sel) = &self.selected_id {
            if !out.iter().any(|s| &s.id == sel) {
                self.selected_id = None;
            }
        }
        self.sources = out;
    }

    pub fn list(&self) -> &[DedupedSource] {
        &self.sources
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn empty_state(&self) -> (&'static str, &'static str) {
        ("No sources yet", "Run research to collect sources")
    }

    pub fn get(&self, id: &str) -> Option<&DedupedSource> {
        self.sources
            .iter()
            .find(|s| s.id == id || s.merged_ids.iter().any(|m| m == id))
    }

    /// URL to open in the browser for a source id.
    pub fn open_url(&self, id: &str) -> Option<&str> {
        self.get(id).map(|s| s.url.as_str())
    }

    /// Message ids to navigate to for a source id, in cited order.
    pub fn cited_messages(&self, id: &str) -> Vec<&str> {
        self.get(id)
            .map(|s| s.citations.iter().map(|c| c.message_id.as_str()).collect())
            .unwrap_or_default()
    }

    /// Citation previews for a source id.
    pub fn citation_previews(&self, id: &str) -> &[CitationPreview] {
        self.get(id)
            .map(|s| s.citations.as_slice())
            .unwrap_or(&[])
    }

    pub fn select(&mut self, id: &str) -> bool {
        if self.get(id).is_some() {
            self.selected_id = Some(self.get(id).unwrap().id.clone());
            true
        } else {
            false
        }
    }

    pub fn selected(&self) -> Option<&DedupedSource> {
        self.selected_id.as_deref().and_then(|id| self.get(id))
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected_id.as_deref()
    }
}

fn preview_quote(quote: &str) -> String {
    const LIMIT: usize = 240;
    let q: String = quote.split_whitespace().collect::<Vec<_>>().join(" ");
    if q.len() <= LIMIT {
        return q;
    }
    let mut end = LIMIT;
    while !q.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", q[..end].trim_end())
}

#[cfg(test)]
mod sources_panel_tests {
    use super::*;

    fn sample() -> (Vec<PanelSource>, Vec<PanelMessage>) {
        let sources = vec![
            PanelSource::new("s1", "https://example.com/report", "Report", "Peer-reviewed"),
            // Duplicate URL (trailing slash + tracking params): must dedupe.
            PanelSource::new(
                "s2",
                "https://example.com/report/?utm_source=x#sec",
                "",
                "",
            ),
            PanelSource::new("s3", "https://other.org/a", "Other", ""),
        ];
        let messages = vec![
            PanelMessage::new("m1", "ana").with_citations(vec![PanelCitation::new(
                "s1",
                "Markets grew 12% year over year.",
            )]),
            PanelMessage::new("m2", "ivan").with_citations(vec![
                PanelCitation::new("s2", "Growth confirmed by second review."),
                PanelCitation::new("s3", "Contrary view from field notes."),
            ]),
        ];
        (sources, messages)
    }

    #[test]
    fn sources_panel_dedupes_by_url() {
        let (sources, messages) = sample();
        let mut panel = SourcesPanel::new();
        panel.rebuild(&sources, &messages);
        assert_eq!(panel.list().len(), 2);
        let first = &panel.list()[0];
        // Canonical row keeps first id/title/note, merges alias ids.
        assert_eq!(first.id, "s1");
        assert_eq!(first.title, "Report");
        assert_eq!(first.credibility_note, "Peer-reviewed");
        assert_eq!(first.merged_ids, vec!["s1".to_string(), "s2".to_string()]);
        // Both citing agents are listed, sorted.
        assert_eq!(first.used_by, vec!["ana".to_string(), "ivan".to_string()]);
        assert_eq!(first.citation_count(), 2);
        // Lookup by alias id resolves to the same row.
        assert_eq!(panel.get("s2").unwrap().id, "s1");
    }

    #[test]
    fn sources_panel_shows_titles_notes_agents_and_previews() {
        let (sources, messages) = sample();
        let mut panel = SourcesPanel::new();
        panel.rebuild(&sources, &messages);
        let previews = panel.citation_previews("s1");
        assert_eq!(previews.len(), 2);
        assert_eq!(previews[0].message_id, "m1");
        assert_eq!(previews[0].agent, "ana");
        assert!(previews[0].quote.contains("Markets grew"));
        // Cited-message navigation order.
        assert_eq!(panel.cited_messages("s1"), vec!["m1", "m2"]);
        assert_eq!(panel.cited_messages("s3"), vec!["m2"]);
        assert!(panel.cited_messages("missing").is_empty());
        // Second row keeps its own title.
        assert_eq!(panel.get("s3").unwrap().display_title(), "Other");
    }

    #[test]
    fn sources_panel_open_in_browser_and_navigate_to_message() {
        let (sources, messages) = sample();
        let mut panel = SourcesPanel::new();
        panel.rebuild(&sources, &messages);
        // Open in browser: canonical URL (alias resolves too).
        assert_eq!(
            panel.open_url("s2"),
            Some("https://example.com/report")
        );
        assert!(panel.open_url("missing").is_none());
        // Select + navigate.
        assert!(panel.select("s3"));
        assert_eq!(panel.selected_id(), Some("s3"));
        assert_eq!(panel.selected().unwrap().url, "https://other.org/a");
        assert!(!panel.select("missing"));
    }

    #[test]
    fn sources_panel_empty_and_placeholder_states() {
        let mut panel = SourcesPanel::new();
        panel.rebuild(&[], &[]);
        assert!(panel.is_empty());
        let (title, hint) = panel.empty_state();
        assert!(!title.is_empty() && !hint.is_empty());
        assert!(panel.selected().is_none());
        // Citation for an unknown source synthesizes a placeholder row.
        panel.rebuild(
            &[],
            &[PanelMessage::new("m9", "max")
                .with_citations(vec![PanelCitation::new("https://new.example/x", "New claim.")])],
        );
        assert_eq!(panel.list().len(), 1);
        assert_eq!(panel.list()[0].used_by, vec!["max".to_string()]);
        assert_eq!(panel.open_url(panel.list()[0].id.clone().as_str()), Some("https://new.example/x"));
    }

    #[test]
    fn sources_panel_normalizes_tracking_urls() {
        assert_eq!(
            normalize_url("https://example.com/a/?utm_source=x&fbclid=1"),
            "https://example.com/a"
        );
        assert_eq!(
            normalize_url("https://example.com/a/#sec"),
            "https://example.com/a"
        );
        assert_eq!(
            normalize_url("https://example.com/a/"),
            "https://example.com/a"
        );
    }
}
