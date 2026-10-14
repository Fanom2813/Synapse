//! M8 memory acceptance test (see docs/MEMORY.md).
//!
//! Offline integration through the real store, core and app APIs: scoped
//! persistence + supersession history, memory writing (secret rejection,
//! review proposals), recall scope boundaries (superseded excluded), vector
//! search with its unavailable-vector fallback, memory review controls, and
//! global search across memory and archive content. In-memory storage only.

use synapse_app::global_search::{global_search, GlobalSearchKind};
use synapse_app::memory_screens::{MemoryProposal, MemoryScopeTab, MemoryScreens};
use synapse_core::memory_recall::{
    active_memories, archive_search, build_memory_context, recall_search, ArchiveItem, MemoryView,
};
use synapse_core::memory_write::{
    extract_proposals, filter_secret_proposals, remember_explicit, CandidateFact, ExistingMemory,
    MemoryOp, RememberOutcome, RememberRequest, ReviewQueue,
};
use synapse_core::redact::{fixtures, Redactor};
use synapse_store::{NewMemory, NewMessage, NewProject, Store};

fn test_redactor() -> Redactor {
    let mut r = Redactor::new();
    r.add_secrets(fixtures::all());
    r
}

fn save(
    store: &Store,
    id: &str,
    scope: &str,
    scope_id: &str,
    kind: &str,
    text: &str,
    source: &str,
) {
    store
        .save_memory(&NewMemory {
            id: id.into(),
            scope: scope.into(),
            scope_id: scope_id.into(),
            kind: kind.into(),
            text: text.into(),
            source_message_id: source.into(),
            created_by: "user".into(),
            confidence: 0.9,
            pinned: false,
        })
        .unwrap();
}

fn views(store: &Store, scope: &str, scope_id: &str) -> Vec<MemoryView> {
    store
        .list_memories(scope, scope_id, true)
        .unwrap()
        .into_iter()
        .map(|m| MemoryView {
            id: m.id,
            scope: m.scope,
            scope_id: m.scope_id,
            kind: m.kind,
            text: m.text,
            source_message_id: m.source_message_id,
            created_by: m.created_by,
            confidence: m.confidence,
            pinned: m.pinned,
            invalid_at: m.invalid_at,
        })
        .collect()
}

fn all_views(store: &Store) -> Vec<MemoryView> {
    let mut out = Vec::new();
    for (scope, sid) in [
        ("user", ""),
        ("project", "p1"),
        ("project", "p2"),
        ("agent", "ada"),
        ("digest", "run1"),
    ] {
        out.extend(views(store, scope, sid));
    }
    out
}

// ---------------------------------------------------------------------------
// 1. scoped persistence and supersession history
// ---------------------------------------------------------------------------

#[test]
fn m08_scoped_persistence_and_supersession_history() {
    let store = Store::open_in_memory().unwrap();
    save(
        &store,
        "u1",
        "user",
        "",
        "preference",
        "prefers sources from 2025 onwards",
        "",
    );
    save(
        &store,
        "p1m1",
        "project",
        "p1",
        "decision",
        "Accepted finding: X causes Y",
        "msg1",
    );
    save(
        &store,
        "p2m1",
        "project",
        "p2",
        "fact",
        "Other project fact stays isolated",
        "",
    );
    save(
        &store,
        "a1",
        "agent",
        "ada",
        "lesson",
        "Scholar search works better for this topic",
        "",
    );

    // Each scope lists only its own rows: no project leaks into another.
    assert_eq!(
        store.list_memories("project", "p1", false).unwrap().len(),
        1
    );
    assert_eq!(
        store.list_memories("project", "p2", false).unwrap().len(),
        1
    );
    assert_eq!(store.list_memories("user", "", false).unwrap().len(), 1);
    assert_eq!(store.list_memories("agent", "ada", false).unwrap().len(), 1);

    // Supersession (graphiti-style): old row keeps invalid_at history, a new
    // active row carries the updated text.
    let updated = store
        .supersede_memory("p1m1", "Revised finding: X mostly causes Y", "agent:ada")
        .unwrap();
    assert_ne!(updated.id, "p1m1");
    assert_eq!(updated.text, "Revised finding: X mostly causes Y");
    let old = store.get_memory("p1m1").unwrap();
    assert!(old.invalid_at.is_some());
    // Active list hides the superseded row; history shows both.
    let active = store.list_memories("project", "p1", false).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, updated.id);
    let history = store.memory_history("project", "p1").unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().any(|m| m.invalid_at.is_some()));
}

// ---------------------------------------------------------------------------
// 2. memory writing: secret rejection + review proposals
// ---------------------------------------------------------------------------

#[test]
fn m08_memory_writing_rejects_secrets_and_proposes_reviews() {
    let redactor = test_redactor();

    // Explicit user write applies immediately.
    let out = remember_explicit(
        &redactor,
        &RememberRequest::user("project", "p1", "Prefers morning runs", "preference"),
    );
    assert!(matches!(out, RememberOutcome::Applied(_)));

    // Agent's own notes apply immediately; agent writes to project/user
    // memory become review proposals.
    let out = remember_explicit(
        &redactor,
        &RememberRequest::agent("agent", "ada", "Lesson learned", "lesson", "agent:ada"),
    );
    assert!(matches!(out, RememberOutcome::Applied(_)));
    let out = remember_explicit(
        &redactor,
        &RememberRequest::agent("project", "p1", "Proposed fact", "fact", "agent:ada"),
    );
    match out {
        RememberOutcome::Proposed(p) => assert!(p.needs_review()),
        other => panic!("expected proposal, got {other:?}"),
    }

    // Secrets are rejected and never stored — known fake key, pattern
    // match, sensitive source, and empty text alike.
    let secret_text = format!("my key is {}", fixtures::OPENAI.1);
    for req in [
        RememberRequest::user("project", "p1", &secret_text, "fact"),
        RememberRequest::agent("project", "p1", &secret_text, "fact", "agent:ada"),
        RememberRequest {
            sensitive_source: true,
            ..RememberRequest::user("project", "p1", "sensitive page fact", "fact")
        },
        RememberRequest::user("project", "p1", "   ", "fact"),
    ] {
        assert!(
            matches!(req, _)
                && matches!(
                    remember_explicit(&redactor, &req),
                    RememberOutcome::Rejected { .. }
                ),
            "must reject {req:?}"
        );
    }

    // Automatic extraction: duplicate → NOOP, near-duplicate → UPDATE
    // (supersede), retraction → DELETE, new → ADD; secrets dropped.
    let existing = vec![ExistingMemory::new("e1", "X causes Y", "fact")];
    let candidates = vec![
        CandidateFact::new("X causes Y", "fact"),
        CandidateFact::new("X causes Y in most cases", "fact"),
        CandidateFact {
            forget: true,
            ..CandidateFact::new("X causes Y", "fact")
        },
        CandidateFact::new("Brand new unrelated finding here", "fact"),
        CandidateFact::new(&secret_text, "fact"),
        CandidateFact {
            sensitive_source: true,
            ..CandidateFact::new("from a sensitive page", "fact")
        },
    ];
    let (decisions, dropped) = extract_proposals(
        &redactor,
        "project",
        "p1",
        "agent:ada",
        &candidates,
        &existing,
    );
    assert_eq!(dropped, 2);
    let ops: Vec<MemoryOp> = decisions.iter().map(|d| d.op).collect();
    assert!(
        ops.contains(&MemoryOp::Noop),
        "duplicate must NOOP: {ops:?}"
    );
    assert!(
        ops.contains(&MemoryOp::Update),
        "near-duplicate must UPDATE: {ops:?}"
    );
    assert!(
        ops.contains(&MemoryOp::Delete),
        "retraction must DELETE: {ops:?}"
    );
    assert!(ops.contains(&MemoryOp::Add), "new fact must ADD: {ops:?}");
    let update = decisions.iter().find(|d| d.op == MemoryOp::Update).unwrap();
    assert_eq!(
        update.apply_action(),
        synapse_core::memory_write::ApplyAction::Supersede {
            target_id: "e1".into()
        }
    );

    // Defence in depth: model-produced proposals carrying secrets are split
    // out before they can reach the store.
    let proposals = vec![
        synapse_core::memory_write::Proposal {
            id: "ok1".into(),
            op: MemoryOp::Add,
            scope: "project".into(),
            scope_id: "p1".into(),
            kind: "fact".into(),
            text: "Clean fact".into(),
            target_id: None,
            source_message_id: String::new(),
            created_by: "agent:ada".into(),
            confidence: 0.8,
            reason: "new fact".into(),
        },
        synapse_core::memory_write::Proposal {
            id: "bad1".into(),
            op: MemoryOp::Add,
            scope: "project".into(),
            scope_id: "p1".into(),
            kind: "fact".into(),
            text: secret_text.clone(),
            target_id: None,
            source_message_id: String::new(),
            created_by: "agent:ada".into(),
            confidence: 0.8,
            reason: "new fact".into(),
        },
    ];
    let (kept, dropped) = filter_secret_proposals(&redactor, proposals);
    assert_eq!(kept.len(), 1);
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].id, "bad1");

    // Review queue: project/user proposals wait for approval; NOOPs and
    // agent-note writes never enter the queue.
    let mut queue = ReviewQueue::new();
    let mut reviewable = synapse_core::memory_write::Proposal {
        id: "q1".into(),
        op: MemoryOp::Add,
        scope: "project".into(),
        scope_id: "p1".into(),
        kind: "fact".into(),
        text: "Awaiting review".into(),
        target_id: None,
        source_message_id: String::new(),
        created_by: "agent:ada".into(),
        confidence: 0.7,
        reason: String::new(),
    };
    assert!(queue.propose(reviewable.clone()));
    assert!(!queue.propose(reviewable.clone())); // duplicate id ignored
    reviewable.op = MemoryOp::Noop;
    assert!(!queue.propose(reviewable));
    assert_eq!(queue.pending().len(), 1);
    assert_eq!(queue.approve("q1").unwrap().text, "Awaiting review");
    assert!(queue.pending().is_empty());
}

// ---------------------------------------------------------------------------
// 3. recall scope boundaries; superseded memories excluded
// ---------------------------------------------------------------------------

#[test]
fn m08_recall_scope_boundaries_exclude_superseded() {
    let store = Store::open_in_memory().unwrap();
    save(
        &store,
        "u1",
        "user",
        "",
        "preference",
        "prefers fintech ledgers",
        "",
    );
    save(
        &store,
        "p1m1",
        "project",
        "p1",
        "fact",
        "fintech ledger pilot results",
        "m1",
    );
    save(
        &store,
        "p2m1",
        "project",
        "p2",
        "fact",
        "fintech ledger other project",
        "m2",
    );
    save(
        &store,
        "a1",
        "agent",
        "ada",
        "lesson",
        "fintech ledger search tips",
        "",
    );
    save(
        &store,
        "old1",
        "project",
        "p1",
        "fact",
        "fintech outdated claim",
        "m3",
    );
    store
        .supersede_memory("old1", "fintech corrected claim", "agent:ada")
        .unwrap();

    let all = all_views(&store);
    // Superseded rows never enter context.
    assert!(all.iter().any(|m| m.invalid_at.is_some()));
    let active = active_memories(&all, "p1", "ada");
    assert!(!active.iter().any(|m| m.id == "old1"));
    // Scope boundaries: this project + user + own notes visible, other
    // projects invisible.
    let ids: Vec<&str> = active.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"u1") && ids.contains(&"p1m1") && ids.contains(&"a1"));
    assert!(!ids.contains(&"p2m1"));

    // recall(query, scope?) respects scope; superseded text stays out.
    let hits = recall_search("fintech ledger", "", &all, "p1", "ada", 10);
    assert!(!hits.is_empty());
    assert!(!hits.iter().any(|h| h.id == "old1" || h.id == "p2m1"));
    let scoped = recall_search("fintech ledger", "project", &all, "p1", "ada", 10);
    assert!(!scoped.is_empty());
    assert!(scoped.iter().all(|h| h.scope == "project"));

    // Full turn context carries linkable ids for the "used N memories" line.
    let ctx = build_memory_context("fintech ledger", &all, "p1", "ada", 1500, 5);
    assert!(!ctx.is_empty());
    let used = ctx.used_ids();
    assert!(!used.is_empty());
    let line = synapse_core::memory_recall::MemoryContext::attribution_line(&used);
    assert!(line.starts_with("used "), "{line}");
    assert!(!used.contains(&"old1".to_string()));
    assert!(!used.contains(&"p2m1".to_string()));

    // Archive search covers superseded versions and old digests.
    let archive = vec![
        ArchiveItem::new("old1", "superseded", "fintech outdated claim"),
        ArchiveItem::new("run1-digest", "run digest", "fintech ledger pilot summary"),
    ];
    let hits = archive_search("fintech ledger", &archive, 5);
    assert!(!hits.is_empty());
}

// ---------------------------------------------------------------------------
// 4. vector search with unavailable-vector fallback
// ---------------------------------------------------------------------------

#[test]
fn m08_vector_search_falls_back_when_unavailable() {
    let store = Store::open_in_memory().unwrap();
    save(
        &store,
        "v1",
        "project",
        "p1",
        "fact",
        "mobile money ledger reconciliation",
        "",
    );
    save(
        &store,
        "v2",
        "project",
        "p1",
        "fact",
        "unrelated tea preferences",
        "",
    );
    for id in ["v1", "v2"] {
        let n = store.index_memory_for_vector(id).unwrap();
        assert!(n > 0, "memory {id} must index chunks");
    }

    // Hybrid recall fuses FTS + vectors while the model is present.
    synapse_store::vector::set_vectors_available(true);
    let _ = synapse_store::vector::embeddings(); // lazy init, then present
    synapse_store::vector::set_vectors_available(true);
    let hybrid = store
        .hybrid_search_memories("ledger", Some(("project", "p1")), 5)
        .unwrap();
    assert!(!hybrid.is_empty());
    assert!(hybrid.iter().any(|h| h.memory.id == "v1"));

    // With the model missing/unloaded, search degrades to FTS instead of
    // failing — the same query still finds the ledger memory.
    synapse_store::vector::set_vectors_available(false);
    let pure = store
        .vector_search_chunks("ledger", Some("memory"), 5)
        .unwrap();
    assert!(pure.is_empty());
    let fallback = store
        .hybrid_search_memories("ledger", Some(("project", "p1")), 5)
        .unwrap();
    assert!(!fallback.is_empty(), "FTS fallback must still hit");
    assert!(fallback.iter().any(|h| h.memory.id == "v1"));
    synapse_store::vector::set_vectors_available(true);
}

// ---------------------------------------------------------------------------
// 5. memory review controls (app state over the store)
// ---------------------------------------------------------------------------

#[test]
fn m08_memory_review_controls() {
    let store = Store::open_in_memory().unwrap();
    let mut screen = MemoryScreens::new(MemoryScopeTab::Project("p1".into()));
    screen.refresh(&store);
    assert!(screen.is_empty());
    assert!(!screen.empty_state().0.is_empty());

    // Agent proposals queue for review; nothing applies until approved.
    screen
        .propose(
            &store,
            MemoryProposal::new(
                "project",
                "p1",
                "fact",
                "Proposed one",
                "m1",
                "agent:ada",
                0.7,
            ),
        )
        .unwrap();
    screen
        .propose(
            &store,
            MemoryProposal::new(
                "project",
                "p1",
                "fact",
                "Proposed two",
                "m1",
                "agent:ada",
                0.7,
            ),
        )
        .unwrap();
    assert_eq!(screen.pending_count(), 2);
    assert!(screen.rows().is_empty());
    let first = screen.proposals()[0].id.clone();
    assert!(screen.approve(&store, &first));
    assert_eq!(screen.rows().len(), 1);
    assert_eq!(screen.approve_all(&store), 1);
    assert_eq!(screen.rows().len(), 2);
    assert!(!screen.approve(&store, "missing"));

    // Reject drops without applying; empty proposals are invalid.
    screen
        .propose(
            &store,
            MemoryProposal::new("project", "p1", "fact", "Bad idea", "m1", "agent:ada", 0.2),
        )
        .unwrap();
    let bad = screen.proposals()[0].id.clone();
    assert!(screen.reject(&bad));
    assert_eq!(screen.pending_count(), 0);
    assert!(screen
        .propose(
            &store,
            MemoryProposal::new("project", "p1", "fact", "  ", "m1", "agent:ada", 0.5),
        )
        .is_err());

    // Edit / pin / delete apply with validation + visible errors.
    let id = screen.rows()[0].id.clone();
    assert!(screen.edit_text(&store, &id, "Edited text"));
    assert!(!screen.edit_text(&store, &id, "   "));
    assert!(screen.last_error().is_some());
    assert!(screen.set_pinned(&store, &id, true));
    assert_eq!(screen.rows()[0].id, id);
    assert!(screen.delete(&store, &id));
    assert_eq!(screen.rows().len(), 1);

    // Auto-accept + pause shape the queue (MEMORY.md §4).
    assert!(screen.set_auto_accept_project(&store, true));
    let applied = screen
        .propose(
            &store,
            MemoryProposal::new("project", "p1", "fact", "Auto fact", "m1", "agent:ada", 0.8),
        )
        .unwrap();
    assert!(applied.is_some());
    assert!(screen.set_paused(&store, true));
    assert!(screen.is_paused());
    let queued = screen
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
    assert_eq!(screen.pending_count(), 1);
    // Forget a scope clears the tab.
    assert!(screen.forget_scope(&store).unwrap() >= 1);
    assert!(screen.rows().is_empty());
    let _ = screen.set_paused(&store, false);
}

// ---------------------------------------------------------------------------
// 6. global search across memory and archive content
// ---------------------------------------------------------------------------

#[test]
fn m08_global_search_covers_memory_and_archive() {
    let store = Store::open_in_memory().unwrap();
    for (id, name) in [("p1", "Market study"), ("p2", "Uganda fintech")] {
        store
            .create_project(&NewProject {
                id: id.into(),
                name: name.into(),
                goal: "g".into(),
                folder: "/tmp".into(),
                lead_agent_id: String::new(),
                template: "research".into(),
            })
            .unwrap();
    }
    store
        .create_conversation("c1", "p1", "team_room", "Room")
        .unwrap();
    store
        .insert_message(&NewMessage {
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
    store
        .save_source(
            "s1",
            "p2",
            "https://example.com/fintech",
            "Fintech report",
            "mobile money fintech ledger analysis",
        )
        .unwrap();
    store
        .save_artifact("a1", "p1", "Market report", "report")
        .unwrap();
    store
        .save_artifact_version("a1", 1, "fintech adoption ledger summary", "first draft")
        .unwrap();
    save(
        &store,
        "mem1",
        "project",
        "p2",
        "fact",
        "fintech agent prefers morning runs",
        "m1",
    );

    let hits = global_search(&store, "fintech", 5);
    let kinds: Vec<GlobalSearchKind> = hits.iter().map(|h| h.kind).collect();
    for want in [
        GlobalSearchKind::Message,
        GlobalSearchKind::Source,
        GlobalSearchKind::Artifact,
        GlobalSearchKind::Memory,
    ] {
        assert!(kinds.contains(&want), "missing {want:?} in {kinds:?}");
    }
    let mem = hits
        .iter()
        .find(|h| h.kind == GlobalSearchKind::Memory)
        .unwrap();
    assert!(mem.snippet.to_lowercase().contains("fintech"));
    assert!(global_search(&store, "   ", 5).is_empty());
}
