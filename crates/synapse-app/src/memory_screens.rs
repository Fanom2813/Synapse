//! Memory management screens (see docs/MEMORY.md §4).
//!
//! Pure state over the `memories` table in `synapse-store`: list by scope,
//! search, edit, pin, delete, provenance (link to the source message),
//! supersession history, a review queue for proposed memories, forget a run
//! or a whole scope, and pause memory per project. The GPUI view renders
//! this; tests drive the state without GPUI.

use synapse_store::{Memory, NewMemory, Store, StoreError};

/// Which scope tab the Memory screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScopeTab {
    User,
    Project(String),
    Agent(String),
    Digest(String),
}

impl MemoryScopeTab {
    pub fn scope(&self) -> &'static str {
        match self {
            MemoryScopeTab::User => "user",
            MemoryScopeTab::Project(_) => "project",
            MemoryScopeTab::Agent(_) => "agent",
            MemoryScopeTab::Digest(_) => "digest",
        }
    }

    pub fn scope_id(&self) -> &str {
        match self {
            MemoryScopeTab::User => "",
            MemoryScopeTab::Project(id)
            | MemoryScopeTab::Agent(id)
            | MemoryScopeTab::Digest(id) => id.as_str(),
        }
    }

    pub fn title(&self) -> String {
        match self {
            MemoryScopeTab::User => "User profile".to_string(),
            MemoryScopeTab::Project(id) => format!("Project {id}"),
            MemoryScopeTab::Agent(id) => format!("Agent {id}"),
            MemoryScopeTab::Digest(id) => format!("Run {id}"),
        }
    }
}

/// A proposed memory awaiting user review (MEMORY.md §4: agent writes to
/// project/user memory land here; agent's own notes apply immediately).
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryProposal {
    pub id: String,
    pub scope: String,
    pub scope_id: String,
    pub kind: String,
    pub text: String,
    pub source_message_id: String,
    pub created_by: String,
    pub confidence: f64,
}

impl MemoryProposal {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: impl Into<String>,
        scope_id: impl Into<String>,
        kind: impl Into<String>,
        text: impl Into<String>,
        source_message_id: impl Into<String>,
        created_by: impl Into<String>,
        confidence: f64,
    ) -> Self {
        Self {
            id: synapse_store::models::new_id("prop"),
            scope: scope.into(),
            scope_id: scope_id.into(),
            kind: kind.into(),
            text: text.into(),
            source_message_id: source_message_id.into(),
            created_by: created_by.into(),
            confidence,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryScreensError {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, MemoryScreensError>;

fn pause_key(scope: &str, scope_id: &str) -> String {
    format!("memory_paused:{scope}:{scope_id}")
}

const AUTO_ACCEPT_PROJECT_KEY: &str = "memory_auto_accept_project";

fn read_bool_setting(store: &Store, key: &str) -> bool {
    let raw = store.setting_get(key).unwrap_or(None).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::String(raw));
    v.as_bool().unwrap_or(false)
}

/// Owned Memory screen state.
#[derive(Debug)]
pub struct MemoryScreens {
    tab: MemoryScopeTab,
    query: String,
    rows: Vec<Memory>,
    selected_id: Option<String>,
    proposals: Vec<MemoryProposal>,
    paused: bool,
    auto_accept_project: bool,
    last_error: Option<String>,
    show_history: bool,
}

impl MemoryScreens {
    pub fn new(tab: MemoryScopeTab) -> Self {
        Self {
            tab,
            query: String::new(),
            rows: Vec::new(),
            selected_id: None,
            proposals: Vec::new(),
            paused: false,
            auto_accept_project: false,
            last_error: None,
            show_history: false,
        }
    }

    pub fn tab(&self) -> &MemoryScopeTab {
        &self.tab
    }

    pub fn rows(&self) -> &[Memory] {
        &self.rows
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn empty_state(&self) -> (&'static str, &'static str) {
        if self.query.trim().is_empty() {
            (
                "No memories yet",
                "Say “remember that…” or use Remember on a message",
            )
        } else {
            ("No matches", "Try a different search")
        }
    }

    pub fn selected(&self) -> Option<&Memory> {
        self.selected_id
            .as_deref()
            .and_then(|id| self.rows.iter().find(|m| m.id == id))
    }

    pub fn select(&mut self, id: &str) -> bool {
        if self.rows.iter().any(|m| m.id == id) {
            self.selected_id = Some(id.to_string());
            true
        } else {
            false
        }
    }

    pub fn switch_tab(&mut self, tab: MemoryScopeTab, store: &Store) {
        self.tab = tab;
        self.query.clear();
        self.selected_id = None;
        self.show_history = false;
        self.refresh(store);
    }

    /// Reload rows for the current tab + pause/auto-accept flags.
    pub fn refresh(&mut self, store: &Store) {
        self.last_error = None;
        self.paused = read_bool_setting(store, &pause_key(self.tab.scope(), self.tab.scope_id()));
        self.auto_accept_project = read_bool_setting(store, AUTO_ACCEPT_PROJECT_KEY);
        match store.list_memories(self.tab.scope(), self.tab.scope_id(), self.show_history) {
            Ok(rows) => {
                self.rows = rows;
                if let Some(sel) = &self.selected_id {
                    if !self.rows.iter().any(|m| &m.id == sel) {
                        self.selected_id = None;
                    }
                }
            }
            Err(e) => self.last_error = Some(e.to_string()),
        }
    }

    /// Search active memories; empty query restores the full list.
    pub fn search(&mut self, store: &Store, query: &str) {
        self.query = query.to_string();
        self.last_error = None;
        let q = query.trim();
        if q.is_empty() {
            self.refresh(store);
            return;
        }
        match store.search_memories(q, Some((self.tab.scope(), self.tab.scope_id())), 50) {
            Ok(rows) => self.rows = rows,
            Err(e) => self.last_error = Some(e.to_string()),
        }
    }

    /// Edit a memory's text (empty text is rejected with a visible error).
    pub fn edit_text(&mut self, store: &Store, id: &str, text: &str) -> bool {
        if text.trim().is_empty() {
            self.last_error = Some("Memory text cannot be empty".to_string());
            return false;
        }
        match store.update_memory_text(id, text) {
            Ok(_) => {
                self.refresh(store);
                true
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                false
            }
        }
    }

    /// Pin / unpin (pinned memories sort first, always in the core block).
    pub fn set_pinned(&mut self, store: &Store, id: &str, pinned: bool) -> bool {
        match store.pin_memory(id, pinned) {
            Ok(_) => {
                self.refresh(store);
                true
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                false
            }
        }
    }

    /// Forget one memory (physical delete).
    pub fn delete(&mut self, store: &Store, id: &str) -> bool {
        match store.delete_memory(id) {
            Ok(()) => {
                self.refresh(store);
                true
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                false
            }
        }
    }

    /// Provenance: the message this memory came from, if any.
    pub fn provenance(&self, store: &Store, memory_id: &str) -> Option<synapse_store::Message> {
        let source_id = self
            .rows
            .iter()
            .find(|m| m.id == memory_id)
            .map(|m| m.source_message_id.clone())
            .or_else(|| {
                store
                    .get_memory(memory_id)
                    .ok()
                    .map(|m| m.source_message_id)
            })?;
        if source_id.trim().is_empty() {
            return None;
        }
        store.get_message(&source_id).ok()
    }

    /// Supersession history for the current tab (oldest first, includes
    /// superseded rows).
    pub fn history(&self, store: &Store) -> Vec<Memory> {
        store
            .memory_history(self.tab.scope(), self.tab.scope_id())
            .unwrap_or_default()
    }

    pub fn showing_history(&self) -> bool {
        self.show_history
    }

    pub fn toggle_history(&mut self, store: &Store) {
        self.show_history = !self.show_history;
        self.refresh(store);
    }

    // -- review queue (MEMORY.md §4) ------------------------------------

    /// Queue a proposal. With auto-accept on, project-scope proposals apply
    /// immediately; everything else waits for review.
    pub fn propose(&mut self, store: &Store, proposal: MemoryProposal) -> Result<Option<Memory>> {
        if proposal.text.trim().is_empty() {
            return Err(MemoryScreensError::Invalid(
                "Proposal text cannot be empty".to_string(),
            ));
        }
        if proposal.scope == "project"
            && read_bool_setting(store, AUTO_ACCEPT_PROJECT_KEY)
            && !self.is_paused_scope(store, &proposal.scope, &proposal.scope_id)
        {
            let saved = store.save_memory(&NewMemory {
                id: String::new(),
                scope: proposal.scope,
                scope_id: proposal.scope_id,
                kind: proposal.kind,
                text: proposal.text,
                source_message_id: proposal.source_message_id,
                created_by: proposal.created_by,
                confidence: proposal.confidence,
                pinned: false,
            })?;
            self.refresh(store);
            return Ok(Some(saved));
        }
        self.proposals.push(proposal);
        Ok(None)
    }

    pub fn proposals(&self) -> &[MemoryProposal] {
        &self.proposals
    }

    /// Badge count for the review queue.
    pub fn pending_count(&self) -> usize {
        self.proposals.len()
    }

    pub fn approve(&mut self, store: &Store, proposal_id: &str) -> bool {
        let Some(i) = self.proposals.iter().position(|p| p.id == proposal_id) else {
            self.last_error = Some(format!("No proposal {proposal_id}"));
            return false;
        };
        let p = self.proposals.remove(i);
        match store.save_memory(&NewMemory {
            id: String::new(),
            scope: p.scope,
            scope_id: p.scope_id,
            kind: p.kind,
            text: p.text,
            source_message_id: p.source_message_id,
            created_by: p.created_by,
            confidence: p.confidence,
            pinned: false,
        }) {
            Ok(_) => {
                self.refresh(store);
                true
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                false
            }
        }
    }

    pub fn approve_all(&mut self, store: &Store) -> usize {
        let ids: Vec<String> = self.proposals.iter().map(|p| p.id.clone()).collect();
        let mut n = 0;
        for id in ids {
            if self.approve(store, &id) {
                n += 1;
            }
        }
        n
    }

    pub fn reject(&mut self, proposal_id: &str) -> bool {
        let Some(i) = self.proposals.iter().position(|p| p.id == proposal_id) else {
            return false;
        };
        self.proposals.remove(i);
        true
    }

    pub fn set_auto_accept_project(&mut self, store: &Store, on: bool) -> bool {
        match store.setting_set(AUTO_ACCEPT_PROJECT_KEY, if on { "true" } else { "false" }) {
            Ok(()) => {
                self.auto_accept_project = on;
                true
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                false
            }
        }
    }

    pub fn auto_accept_project(&self) -> bool {
        self.auto_accept_project
    }

    // -- forget + pause (MEMORY.md §4) -----------------------------------

    /// Forget a whole scope ("clear tab"). Returns rows removed.
    pub fn forget_scope(&mut self, store: &Store) -> Result<usize> {
        let n = store.clear_scope(self.tab.scope(), self.tab.scope_id())?;
        self.refresh(store);
        Ok(n)
    }

    /// "Forget everything from this run": drop the run's digest rows plus
    /// any memories whose source message is in `message_ids`.
    pub fn forget_run(
        &mut self,
        store: &Store,
        run_id: &str,
        message_ids: &[String],
    ) -> Result<usize> {
        // Delete rows whose source message is in this run, across the
        // current tab and the user scope (agent notes stay with the agent).
        let mut removed = 0;
        for id in message_ids {
            for (scope, sid) in [("user", ""), (self.tab.scope(), self.tab.scope_id())] {
                let rows = store.list_memories(scope, sid, true).unwrap_or_default();
                for m in rows {
                    if &m.source_message_id == id {
                        let _ = store.delete_memory(&m.id);
                        removed += 1;
                    }
                }
            }
        }
        removed += store.clear_scope("digest", run_id).unwrap_or(0);
        self.refresh(store);
        Ok(removed)
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    fn is_paused_scope(&self, store: &Store, scope: &str, scope_id: &str) -> bool {
        read_bool_setting(store, &pause_key(scope, scope_id))
    }

    /// Pause / resume project memory for the current tab.
    pub fn set_paused(&mut self, store: &Store, paused: bool) -> bool {
        match store.setting_set(
            &pause_key(self.tab.scope(), self.tab.scope_id()),
            if paused { "true" } else { "false" },
        ) {
            Ok(()) => {
                self.paused = paused;
                true
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                false
            }
        }
    }

    /// Per-scope memory size (active rows) for the quota display.
    pub fn scope_size(&self, store: &Store) -> usize {
        store
            .list_memories(self.tab.scope(), self.tab.scope_id(), false)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn save(
        store: &Store,
        id: &str,
        scope: &str,
        scope_id: &str,
        text: &str,
        source: &str,
    ) -> Memory {
        store
            .save_memory(&NewMemory {
                id: id.into(),
                scope: scope.into(),
                scope_id: scope_id.into(),
                kind: "fact".into(),
                text: text.into(),
                source_message_id: source.into(),
                created_by: "user".into(),
                confidence: 0.9,
                pinned: false,
            })
            .unwrap()
    }

    fn project_screen(store: &Store) -> MemoryScreens {
        let mut s = MemoryScreens::new(MemoryScopeTab::Project("p1".into()));
        s.refresh(store);
        s
    }

    #[test]
    fn memory_screens_list_by_scope_without_leaks() {
        let store = mem_store();
        save(&store, "m1", "project", "p1", "Project fact", "msg1");
        save(&store, "m2", "user", "", "User fact", "msg1");
        let s = project_screen(&store);
        assert_eq!(s.rows().len(), 1);
        assert_eq!(s.rows()[0].id, "m1");
        let mut u = MemoryScreens::new(MemoryScopeTab::User);
        u.refresh(&store);
        assert_eq!(u.rows().len(), 1);
        assert!(s.rows().iter().all(|m| m.scope == "project"));
    }

    #[test]
    fn memory_screens_search_filters_and_restores() {
        let store = mem_store();
        save(
            &store,
            "m1",
            "project",
            "p1",
            "Scholar search works best",
            "msg1",
        );
        save(
            &store,
            "m2",
            "project",
            "p1",
            "Prefers tea over coffee",
            "msg2",
        );
        let mut s = project_screen(&store);
        s.search(&store, "Scholar");
        assert_eq!(s.rows().len(), 1);
        assert_eq!(s.rows()[0].id, "m1");
        assert_eq!(s.query(), "Scholar");
        // Empty query restores the full list.
        s.search(&store, "  ");
        assert_eq!(s.rows().len(), 2);
        // No matches keeps the list usable with an empty state.
        s.search(&store, "zzz-no-such-memory");
        assert!(s.rows().is_empty());
        assert!(s.is_empty());
        let (title, hint) = s.empty_state();
        assert!(!title.is_empty() && !hint.is_empty());
    }

    #[test]
    fn memory_screens_edit_pin_and_delete() {
        let store = mem_store();
        save(&store, "m1", "project", "p1", "Old text", "msg1");
        save(&store, "m2", "project", "p1", "Other", "msg2");
        let mut s = project_screen(&store);
        // Edit applies and refreshes.
        assert!(s.edit_text(&store, "m1", "New text"));
        assert_eq!(store.get_memory("m1").unwrap().text, "New text");
        // Empty edit is rejected with a visible error.
        assert!(!s.edit_text(&store, "m1", "   "));
        assert!(s.last_error().is_some());
        assert_eq!(store.get_memory("m1").unwrap().text, "New text");
        // Pin sorts first.
        assert!(s.set_pinned(&store, "m2", true));
        assert_eq!(s.rows()[0].id, "m2");
        assert!(s.select("m2"));
        assert_eq!(s.selected().unwrap().id, "m2");
        assert!(!s.select("missing"));
        // Delete removes the row.
        assert!(s.delete(&store, "m2"));
        assert_eq!(s.rows().len(), 1);
    }

    #[test]
    fn memory_screens_provenance_and_history() {
        let store = mem_store();
        store
            .create_project(&synapse_store::NewProject {
                id: "p1".into(),
                name: "P1".into(),
                goal: "g".into(),
                folder: "/tmp/synapse-mem".into(),
                lead_agent_id: String::new(),
                template: "custom".into(),
            })
            .unwrap();
        store.create_conversation("c1", "p1", "chat", "C1").unwrap();
        store
            .insert_message(&synapse_store::NewMessage {
                id: "msg1".into(),
                conversation_id: "c1".into(),
                run_id: "r1".into(),
                from_id: "user".into(),
                to_json: "[]".into(),
                kind: "chat".into(),
                reply_to: String::new(),
                thread_id: String::new(),
                body: "Original claim".into(),
                meta: "{}".into(),
            })
            .unwrap();
        save(&store, "m1", "project", "p1", "Old conclusion", "msg1");
        store
            .supersede_memory("m1", "New conclusion", "agent:ada")
            .unwrap();
        let mut s = project_screen(&store);
        s.refresh(&store);
        // Active list hides the superseded row; history shows both.
        assert_eq!(s.rows().len(), 1);
        assert_eq!(s.rows()[0].text, "New conclusion");
        let history = s.history(&store);
        assert_eq!(history.len(), 2);
        assert!(history.iter().any(|m| m.invalid_at.is_some()));
        // Provenance links the active row back to its message.
        let active_id = s.rows()[0].id.clone();
        let msg = s.provenance(&store, &active_id).unwrap();
        assert_eq!(msg.id, "msg1");
        assert!(s.provenance(&store, "missing").is_none());
        // History toggle includes superseded rows in the list.
        s.toggle_history(&store);
        assert!(s.showing_history());
        assert_eq!(s.rows().len(), 2);
        s.toggle_history(&store);
        assert!(!s.showing_history());
    }

    #[test]
    fn memory_screens_review_queue_approve_and_reject() {
        let store = mem_store();
        let mut s = project_screen(&store);
        assert_eq!(s.pending_count(), 0);
        s.propose(
            &store,
            MemoryProposal::new(
                "project",
                "p1",
                "fact",
                "Proposed fact one",
                "m9",
                "agent:ada",
                0.7,
            ),
        )
        .unwrap();
        s.propose(
            &store,
            MemoryProposal::new(
                "project",
                "p1",
                "fact",
                "Proposed fact two",
                "m9",
                "agent:ada",
                0.7,
            ),
        )
        .unwrap();
        assert_eq!(s.pending_count(), 2);
        // Nothing applied until approved.
        assert!(s.rows().is_empty());
        let first = s.proposals()[0].id.clone();
        assert!(s.approve(&store, &first));
        assert_eq!(s.pending_count(), 1);
        assert_eq!(s.rows().len(), 1);
        // Approve-all applies the rest.
        assert_eq!(s.approve_all(&store), 1);
        assert_eq!(s.pending_count(), 0);
        assert_eq!(s.rows().len(), 2);
        // Reject drops without applying.
        s.propose(
            &store,
            MemoryProposal::new("project", "p1", "fact", "Bad idea", "m9", "agent:ada", 0.2),
        )
        .unwrap();
        let bad = s.proposals()[0].id.clone();
        assert!(s.reject(&bad));
        assert_eq!(s.pending_count(), 0);
        assert_eq!(s.rows().len(), 2);
        assert!(!s.approve(&store, "missing"));
        // Empty proposals are rejected.
        assert!(s
            .propose(
                &store,
                MemoryProposal::new("project", "p1", "fact", "  ", "m9", "agent:ada", 0.5),
            )
            .is_err());
    }

    #[test]
    fn memory_screens_auto_accept_and_pause_project_memory() {
        let store = mem_store();
        let mut s = project_screen(&store);
        assert!(!s.auto_accept_project());
        assert!(s.set_auto_accept_project(&store, true));
        assert!(s.auto_accept_project());
        // With auto-accept on, project proposals apply immediately.
        let applied = s
            .propose(
                &store,
                MemoryProposal::new("project", "p1", "fact", "Auto fact", "m1", "agent:ada", 0.8),
            )
            .unwrap();
        assert!(applied.is_some());
        assert_eq!(s.pending_count(), 0);
        assert_eq!(s.rows().len(), 1);
        // Pause stops auto-accept: proposals queue again.
        assert!(!s.is_paused());
        assert!(s.set_paused(&store, true));
        assert!(s.is_paused());
        let queued = s
            .propose(
                &store,
                MemoryProposal::new(
                    "project",
                    "p1",
                    "fact",
                    "Queued while paused",
                    "m1",
                    "agent:ada",
                    0.8,
                ),
            )
            .unwrap();
        assert!(queued.is_none());
        assert_eq!(s.pending_count(), 1);
        // Pause persists across screen rebuilds.
        let mut s2 = project_screen(&store);
        assert!(s2.is_paused());
        assert!(s2.set_paused(&store, false));
        assert!(!s2.is_paused());
    }

    #[test]
    fn memory_screens_forget_run_and_scope() {
        let store = mem_store();
        save(&store, "d1", "digest", "run1", "Run digest summary", "msg1");
        save(&store, "m1", "project", "p1", "Fact from run", "msg1");
        save(&store, "m2", "project", "p1", "Unrelated fact", "msg2");
        let mut s = project_screen(&store);
        assert_eq!(s.scope_size(&store), 2);
        let removed = s.forget_run(&store, "run1", &["msg1".to_string()]).unwrap();
        assert!(removed >= 2);
        assert!(store
            .list_memories("digest", "run1", true)
            .unwrap()
            .is_empty());
        // Only the run's memory is gone.
        let remaining = store.list_memories("project", "p1", false).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "m2");
        // Forget scope clears the tab.
        let cleared = s.forget_scope(&store).unwrap();
        assert_eq!(cleared, 1);
        assert!(s.rows().is_empty());
    }
}
