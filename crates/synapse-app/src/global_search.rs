//! Global archive search: command-palette search over messages, sources,
//! artifacts and memories, jumping to the match in its project/panel.
//!
//! Toolkit-free state over [`Store`]; the GPUI Kit palette renders from this.

use crate::layout::PanelTab;
use crate::shell::AppShell;
use synapse_store::Store;

/// What kind of archive row a hit points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalSearchKind {
    Message,
    Source,
    Artifact,
    Memory,
}

impl GlobalSearchKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Message => "Message",
            Self::Source => "Source",
            Self::Artifact => "Artifact",
            Self::Memory => "Memory",
        }
    }

    /// Right-panel tab a hit opens (messages stay in the room).
    pub fn panel_tab(self) -> Option<PanelTab> {
        match self {
            Self::Message => None,
            Self::Source => Some(PanelTab::Sources),
            Self::Artifact => Some(PanelTab::Artifact),
            Self::Memory => Some(PanelTab::Team),
        }
    }
}

/// One archive hit: where it lives and what to show in the palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalSearchHit {
    pub kind: GlobalSearchKind,
    pub project_id: String,
    pub title: String,
    pub snippet: String,
}

impl GlobalSearchHit {
    /// Open the hit's project and reveal its panel. Returns false when the
    /// project is unknown to the shell.
    pub fn navigate(&self, shell: &mut AppShell) -> bool {
        if !shell.open_project(&self.project_id) {
            return false;
        }
        shell.open_room();
        if let Some(tab) = self.kind.panel_tab() {
            shell.select_panel_tab(tab);
        }
        true
    }
}

fn snippet(text: &str, query: &str, len: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.trim().is_empty() {
        return flat.chars().take(len).collect();
    }
    let lower = flat.to_lowercase();
    let needle = query.trim().to_lowercase();
    if let Some(pos) = lower.find(&needle) {
        // Char-boundary safe window around the match.
        let chars: Vec<char> = flat.chars().collect();
        let char_pos = flat[..pos].chars().count();
        let start = char_pos.saturating_sub(len / 4);
        let end = (char_pos + needle.chars().count() + len / 2).min(chars.len());
        chars[start..end].iter().collect()
    } else {
        flat.chars().take(len).collect()
    }
}

/// Search all four archives. Empty/blank queries return no hits. Each kind
/// contributes up to `per_kind` hits (default 5 when `per_kind` is 0).
pub fn global_search(store: &Store, query: &str, per_kind: i64) -> Vec<GlobalSearchHit> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let per_kind = if per_kind > 0 { per_kind } else { 5 };
    let mut hits = Vec::new();

    if let Ok(msgs) = store.search_messages_with_project(q, per_kind) {
        for (m, project_id) in msgs {
            hits.push(GlobalSearchHit {
                kind: GlobalSearchKind::Message,
                project_id,
                title: format!("Message {}", m.id),
                snippet: snippet(&m.body, q, 120),
            });
        }
    }
    if let Ok(srcs) = store.search_sources(q, per_kind) {
        for s in srcs {
            let body = if s.title.is_empty() {
                s.content_md.clone()
            } else {
                format!("{} {}", s.title, s.content_md)
            };
            hits.push(GlobalSearchHit {
                kind: GlobalSearchKind::Source,
                project_id: s.project_id,
                title: if s.title.is_empty() {
                    format!("Source {}", s.id)
                } else {
                    s.title.clone()
                },
                snippet: snippet(&body, q, 120),
            });
        }
    }
    if let Ok(arts) = store.search_artifacts(q, per_kind) {
        for a in arts {
            hits.push(GlobalSearchHit {
                kind: GlobalSearchKind::Artifact,
                project_id: a.project_id,
                title: format!("{} (v{})", a.title, a.version),
                snippet: snippet(&a.body_md, q, 120),
            });
        }
    }
    if let Ok(mems) = store.search_memories(q, None, per_kind) {
        for m in mems {
            hits.push(GlobalSearchHit {
                kind: GlobalSearchKind::Memory,
                project_id: m.scope_id.clone(),
                title: format!("Memory {}", m.id),
                snippet: snippet(&m.text, q, 120),
            });
        }
    }
    hits
}

/// Command-palette state: query + hits + selection, toolkit-free.
#[derive(Debug, Default)]
pub struct CommandPalette {
    pub query: String,
    pub hits: Vec<GlobalSearchHit>,
    pub selected: usize,
}

impl CommandPalette {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-run the search for `query` and reset the selection.
    pub fn set_query(&mut self, store: &Store, query: &str) {
        self.query = query.to_string();
        self.hits = global_search(store, query, 5);
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.hits.is_empty() {
            self.selected = 0;
            return;
        }
        let n = self.hits.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
    }

    pub fn selected_hit(&self) -> Option<&GlobalSearchHit> {
        self.hits.get(self.selected)
    }

    /// Open the selected hit in its project/panel.
    pub fn open_selected(&self, shell: &mut AppShell) -> bool {
        match self.selected_hit() {
            Some(hit) => hit.navigate(shell),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synapse_store::{NewMemory, NewMessage, NewProject};

    fn test_store() -> Store {
        let s = Store::open_in_memory().unwrap();
        for (id, name) in [("p1", "Market study"), ("p2", "Uganda fintech")] {
            s.create_project(&NewProject {
                id: id.into(),
                name: name.into(),
                goal: "g".into(),
                folder: "/tmp".into(),
                lead_agent_id: String::new(),
                template: "research".into(),
            })
            .unwrap();
        }
        s.create_conversation("c1", "p1", "team_room", "Room")
            .unwrap();
        s.create_conversation("c2", "p2", "team_room", "Room")
            .unwrap();
        s.insert_message(&NewMessage {
            id: "m1".into(),
            conversation_id: "c1".into(),
            run_id: String::new(),
            from_id: "user".into(),
            to_json: "[]".into(),
            kind: "chat".into(),
            reply_to: String::new(),
            thread_id: String::new(),
            body: "fintech ledger reaches the villages".into(),
            meta: "{}".into(),
        })
        .unwrap();
        s.save_source(
            "s1",
            "p2",
            "https://example.com/fintech",
            "Fintech report",
            "mobile money fintech ledger analysis",
        )
        .unwrap();
        s.save_artifact("a1", "p1", "Market report", "report")
            .unwrap();
        s.save_artifact_version("a1", 1, "fintech adoption ledger summary", "first draft")
            .unwrap();
        s.save_memory(&NewMemory {
            id: "mem1".into(),
            scope: "project".into(),
            scope_id: "p2".into(),
            kind: "fact".into(),
            text: "fintech agent prefers morning runs".into(),
            source_message_id: "m1".into(),
            created_by: "user".into(),
            confidence: 0.9,
            pinned: false,
        })
        .unwrap();
        s
    }

    fn shell_with_projects() -> AppShell {
        let mut shell = AppShell::open();
        shell.projects.push(crate::shell::ShellProject {
            id: "p1".into(),
            name: "Market study".into(),
            has_messages: true,
        });
        shell.projects.push(crate::shell::ShellProject {
            id: "p2".into(),
            name: "Uganda fintech".into(),
            has_messages: true,
        });
        shell
    }

    #[test]
    fn global_search_finds_all_four_kinds() {
        let s = test_store();
        let hits = global_search(&s, "fintech", 5);
        let kinds: Vec<_> = hits.iter().map(|h| h.kind).collect();
        assert!(kinds.contains(&GlobalSearchKind::Message));
        assert!(kinds.contains(&GlobalSearchKind::Source));
        assert!(kinds.contains(&GlobalSearchKind::Artifact));
        assert!(kinds.contains(&GlobalSearchKind::Memory));
        // Message hit resolves to its project; artifact to its project.
        let msg = hits
            .iter()
            .find(|h| h.kind == GlobalSearchKind::Message)
            .unwrap();
        assert_eq!(msg.project_id, "p1");
        assert!(msg.snippet.to_lowercase().contains("fintech"));
        assert!(global_search(&s, "   ", 5).is_empty());
    }

    #[test]
    fn global_search_navigates_to_project_and_panel() {
        let s = test_store();
        let mut shell = shell_with_projects();
        let mut palette = CommandPalette::new();
        palette.set_query(&s, "fintech");
        assert!(!palette.hits.is_empty());
        // Select the source hit: opens p2 + Sources panel.
        palette.selected = palette
            .hits
            .iter()
            .position(|h| h.kind == GlobalSearchKind::Source)
            .unwrap();
        assert!(palette.open_selected(&mut shell));
        assert_eq!(shell.open_project_id.as_deref(), Some("p2"));
        assert_eq!(
            shell.layout.right_panel_tab,
            crate::layout::PanelTab::Sources
        );
        // Artifact hit: opens p1 + Artifact panel.
        palette.selected = palette
            .hits
            .iter()
            .position(|h| h.kind == GlobalSearchKind::Artifact)
            .unwrap();
        assert!(palette.open_selected(&mut shell));
        assert_eq!(shell.open_project_id.as_deref(), Some("p1"));
        assert_eq!(
            shell.layout.right_panel_tab,
            crate::layout::PanelTab::Artifact
        );
        // Unknown project never breaks the shell.
        let bad = GlobalSearchHit {
            kind: GlobalSearchKind::Message,
            project_id: "missing".into(),
            title: "Message m9".into(),
            snippet: "x".into(),
        };
        assert!(!bad.navigate(&mut shell));
        assert_eq!(shell.open_project_id.as_deref(), Some("p1"));
    }

    #[test]
    fn global_search_palette_selection_wraps() {
        let s = test_store();
        let mut palette = CommandPalette::new();
        palette.set_query(&s, "fintech");
        assert!(!palette.hits.is_empty());
        assert!(palette.selected_hit().is_some());
        palette.move_selection(1);
        assert_eq!(palette.selected, 1 % palette.hits.len());
        palette.move_selection(-1);
        assert_eq!(palette.selected, 0);
        palette.move_selection(-1);
        assert_eq!(palette.selected, palette.hits.len() - 1);
        // Empty query: no hits, open is a no-op.
        palette.set_query(&s, "   ");
        assert!(palette.hits.is_empty());
        assert!(!palette.open_selected(&mut shell_with_projects()));
    }
}
