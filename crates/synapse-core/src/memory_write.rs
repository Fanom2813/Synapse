//! Memory writing + review proposals (docs/MEMORY.md §3–4).
//!
//! Two write paths, both pure (no I/O; the store crate executes the
//! resulting [`ApplyAction`]):
//!
//! 1. **Explicit** [`remember_explicit`]: `remember(scope, text, kind)`.
//!    Writes from the user apply immediately; writes from agents apply to
//!    the agent's own notes immediately and land in the [`ReviewQueue`] as
//!    proposals for `project` / `user` scopes.
//! 2. **Automatic extraction** [`extract_proposals`]: mem0-style comparison
//!    of candidate facts against the most similar existing memories, each
//!    decided as ADD / UPDATE (supersede: old row gets `invalid_at`) /
//!    DELETE / NOOP. Superseded facts stay in history; recalled views only
//!    use active rows.
//!
//! Both paths filter secrets through [`Redactor`](crate::redact::Redactor):
//! anything containing a known or pattern-matched secret, or flagged as
//! coming from a sensitive source, is rejected and never stored.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::redact::Redactor;

/// mem0-style decision for one candidate fact (MEMORY.md §3 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryOp {
    Add,
    Update,
    Delete,
    Noop,
}

/// A proposed or decided memory write. `target_id` is the existing memory
/// the candidate refers to for UPDATE / DELETE / NOOP; `None` for ADD.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub op: MemoryOp,
    pub scope: String,
    pub scope_id: String,
    pub kind: String,
    pub text: String,
    pub target_id: Option<String>,
    pub source_message_id: String,
    pub created_by: String,
    pub confidence: f64,
    pub reason: String,
}

impl Proposal {
    /// What the store should do to apply this proposal. UPDATE maps to a
    /// supersede (old row keeps `invalid_at` history); NOOP maps to nothing.
    pub fn apply_action(&self) -> ApplyAction {
        match self.op {
            MemoryOp::Add => ApplyAction::Insert,
            MemoryOp::Update => match &self.target_id {
                Some(id) => ApplyAction::Supersede {
                    target_id: id.clone(),
                },
                None => ApplyAction::Insert,
            },
            MemoryOp::Delete => match &self.target_id {
                Some(id) => ApplyAction::Delete {
                    target_id: id.clone(),
                },
                None => ApplyAction::None,
            },
            MemoryOp::Noop => ApplyAction::None,
        }
    }

    /// Proposals for `project` / `user` scopes need user review; agent-note
    /// writes apply immediately (MEMORY.md §3.1).
    pub fn needs_review(&self) -> bool {
        matches!(self.scope.as_str(), "project" | "user")
    }
}

/// Store-executable form of a [`Proposal`] (supersession keeps history).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyAction {
    Insert,
    Supersede { target_id: String },
    Delete { target_id: String },
    None,
}

/// One fact the cheap extraction model proposed from a run digest or new
/// messages. `forget` marks "the user retracted this" candidates, which
/// decide to DELETE the most similar existing memory instead of ADD.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateFact {
    pub text: String,
    pub kind: String,
    pub confidence: f64,
    pub source_message_id: String,
    pub forget: bool,
    /// True when the source page/message is marked sensitive: never stored.
    pub sensitive_source: bool,
}

impl CandidateFact {
    pub fn new(text: &str, kind: &str) -> Self {
        Self {
            text: text.to_string(),
            kind: kind.to_string(),
            confidence: 1.0,
            source_message_id: String::new(),
            forget: false,
            sensitive_source: false,
        }
    }
}

/// One existing active memory to compare against (id + text + kind only —
/// the store's `Memory` maps to this without a dependency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingMemory {
    pub id: String,
    pub text: String,
    pub kind: String,
}

impl ExistingMemory {
    pub fn new(id: &str, text: &str, kind: &str) -> Self {
        Self {
            id: id.to_string(),
            text: text.to_string(),
            kind: kind.to_string(),
        }
    }
}

/// Explicit `remember(scope, text, kind)` request (MEMORY.md §3.1).
#[derive(Debug, Clone, PartialEq)]
pub struct RememberRequest {
    pub scope: String,
    pub scope_id: String,
    pub kind: String,
    pub text: String,
    pub created_by: String,
    pub source_message_id: String,
    pub confidence: f64,
    /// True for writes from the user ("remember that…", Remember button,
    /// edited mirror import). Agent tool calls pass false.
    pub from_user: bool,
    /// True when the text comes from a sensitive page: never stored.
    pub sensitive_source: bool,
}

impl RememberRequest {
    pub fn user(scope: &str, scope_id: &str, text: &str, kind: &str) -> Self {
        Self {
            scope: scope.to_string(),
            scope_id: scope_id.to_string(),
            kind: kind.to_string(),
            text: text.to_string(),
            created_by: "user".to_string(),
            source_message_id: String::new(),
            confidence: 1.0,
            from_user: true,
            sensitive_source: false,
        }
    }

    pub fn agent(scope: &str, scope_id: &str, text: &str, kind: &str, created_by: &str) -> Self {
        Self {
            scope: scope.to_string(),
            scope_id: scope_id.to_string(),
            kind: kind.to_string(),
            text: text.to_string(),
            created_by: created_by.to_string(),
            source_message_id: String::new(),
            confidence: 1.0,
            from_user: false,
            sensitive_source: false,
        }
    }
}

/// A memory that passed filtering and is ready to insert.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedMemory {
    pub scope: String,
    pub scope_id: String,
    pub kind: String,
    pub text: String,
    pub source_message_id: String,
    pub created_by: String,
    pub confidence: f64,
}

/// Outcome of an explicit remember call.
#[derive(Debug, Clone, PartialEq)]
pub enum RememberOutcome {
    /// Insert now (user writes; agent's own notes).
    Applied(AppliedMemory),
    /// Needs user approval in the review queue (agent → project/user).
    Proposed(Proposal),
    /// Dropped: empty text, secret content, or sensitive source.
    Rejected { reason: String },
}

fn new_proposal_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    format!("prop_{ms}_{n}")
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn valid_scope(scope: &str) -> bool {
    matches!(scope, "user" | "project" | "agent" | "digest")
}

fn scrubbed(redactor: &Redactor, text: &str) -> String {
    normalize(&redactor.redact(text).text)
}

/// Explicit remember (MEMORY.md §3.1). Secrets and sensitive sources are
/// rejected before anything is stored; project/user writes from agents
/// become review proposals, everything else applies immediately.
pub fn remember_explicit(redactor: &Redactor, req: &RememberRequest) -> RememberOutcome {
    if !valid_scope(&req.scope) {
        return RememberOutcome::Rejected {
            reason: format!("unknown scope '{}'", req.scope),
        };
    }
    if normalize(&req.text).is_empty() {
        return RememberOutcome::Rejected {
            reason: "empty text".to_string(),
        };
    }
    if req.sensitive_source {
        return RememberOutcome::Rejected {
            reason: "sensitive source is never stored".to_string(),
        };
    }
    if redactor.contains_secret(&req.text) {
        return RememberOutcome::Rejected {
            reason: "secret content is never stored".to_string(),
        };
    }
    let kind = if req.kind.trim().is_empty() {
        "fact"
    } else {
        req.kind.trim()
    };
    let text = scrubbed(redactor, &req.text);
    if text.is_empty() {
        return RememberOutcome::Rejected {
            reason: "empty text".to_string(),
        };
    }
    // Agent's own notes apply immediately; user writes apply immediately.
    if req.from_user || req.scope == "agent" {
        return RememberOutcome::Applied(AppliedMemory {
            scope: req.scope.clone(),
            scope_id: req.scope_id.clone(),
            kind: kind.to_string(),
            text,
            source_message_id: req.source_message_id.clone(),
            created_by: req.created_by.clone(),
            confidence: req.confidence,
        });
    }
    RememberOutcome::Proposed(Proposal {
        id: new_proposal_id(),
        op: MemoryOp::Add,
        scope: req.scope.clone(),
        scope_id: req.scope_id.clone(),
        kind: kind.to_string(),
        text,
        target_id: None,
        source_message_id: req.source_message_id.clone(),
        created_by: req.created_by.clone(),
        confidence: req.confidence,
        reason: "explicit agent remember awaits review".to_string(),
    })
}

fn tokens(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.to_string())
        .collect()
}

fn jaccard(a: &str, b: &str) -> f64 {
    let (ta, tb) = (tokens(a), tokens(b));
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f64;
    let union = ta.union(&tb).count() as f64;
    inter / union
}

fn best_match<'a>(
    candidate: &str,
    existing: &'a [ExistingMemory],
) -> Option<(&'a ExistingMemory, f64)> {
    let norm = candidate.to_lowercase();
    let mut best: Option<(&ExistingMemory, f64)> = None;
    for m in existing {
        let other = m.text.to_lowercase();
        let score = if norm == other {
            1.0
        } else if norm.contains(other.as_str()) || other.contains(norm.as_str()) {
            0.9
        } else {
            jaccard(candidate, &m.text)
        };
        if best.map(|(_, s)| score > s).unwrap_or(true) {
            best = Some((m, score));
        }
    }
    best
}

/// Decide one candidate against existing memories: exact duplicates → NOOP,
/// near-duplicates / retractions → UPDATE / DELETE, otherwise ADD.
fn decide_candidate(
    scope: &str,
    scope_id: &str,
    created_by: &str,
    c: &CandidateFact,
    existing: &[ExistingMemory],
) -> Proposal {
    let base = Proposal {
        id: new_proposal_id(),
        op: MemoryOp::Add,
        scope: scope.to_string(),
        scope_id: scope_id.to_string(),
        kind: if c.kind.trim().is_empty() {
            "fact".to_string()
        } else {
            c.kind.trim().to_string()
        },
        text: normalize(&c.text),
        target_id: None,
        source_message_id: c.source_message_id.clone(),
        created_by: created_by.to_string(),
        confidence: c.confidence,
        reason: String::new(),
    };
    let norm = normalize(&c.text);
    if norm.is_empty() {
        return Proposal {
            op: MemoryOp::Noop,
            reason: "empty candidate".to_string(),
            ..base
        };
    }
    let matched = best_match(&norm, existing);
    if c.forget {
        return match matched {
            Some((m, s)) if s >= 0.3 => Proposal {
                op: MemoryOp::Delete,
                target_id: Some(m.id.clone()),
                reason: format!("retraction matches {} (score {s:.2})", m.id),
                ..base
            },
            _ => Proposal {
                op: MemoryOp::Noop,
                reason: "retraction with no matching memory".to_string(),
                ..base
            },
        };
    }
    match matched {
        Some((m, s)) if s >= 0.99 => Proposal {
            op: MemoryOp::Noop,
            target_id: Some(m.id.clone()),
            reason: format!("duplicate of {} — already remembered", m.id),
            ..base
        },
        Some((m, s)) if s >= 0.45 => Proposal {
            op: MemoryOp::Update,
            target_id: Some(m.id.clone()),
            reason: format!(
                "supersedes {} (score {s:.2}) — old row kept as history",
                m.id
            ),
            ..base
        },
        _ => Proposal {
            reason: "new fact".to_string(),
            ..base
        },
    }
}

/// Post-run extraction (MEMORY.md §3.2): filter secrets/sensitive sources,
/// then decide every candidate as ADD / UPDATE / DELETE / NOOP. Returns
/// `(decisions, dropped_count)`; dropped candidates never reach the store.
pub fn extract_proposals(
    redactor: &Redactor,
    scope: &str,
    scope_id: &str,
    created_by: &str,
    candidates: &[CandidateFact],
    existing: &[ExistingMemory],
) -> (Vec<Proposal>, usize) {
    let mut out = Vec::new();
    let mut dropped = 0usize;
    for c in candidates {
        if c.sensitive_source || redactor.contains_secret(&c.text) {
            dropped += 1;
            continue;
        }
        let mut p = decide_candidate(scope, scope_id, created_by, c, existing);
        p.text = scrubbed(redactor, &p.text);
        out.push(p);
    }
    (out, dropped)
}

/// Drop proposals whose text contains secrets (defence in depth for
/// model-produced candidates). Returns `(kept, dropped)`.
pub fn filter_secret_proposals(
    redactor: &Redactor,
    proposals: Vec<Proposal>,
) -> (Vec<Proposal>, Vec<Proposal>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for p in proposals {
        if redactor.contains_secret(&p.text) {
            dropped.push(p);
        } else {
            kept.push(p);
        }
    }
    (kept, dropped)
}

/// The user's review queue (MEMORY.md §4): proposed project/user memories
/// waiting for approve-all / one-by-one. NOOPs never enter the queue.
#[derive(Debug, Default)]
pub struct ReviewQueue {
    pending: Vec<Proposal>,
}

impl ReviewQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a proposal; NOOPs and non-review scopes are ignored.
    /// Returns true when the proposal entered the queue.
    pub fn propose(&mut self, p: Proposal) -> bool {
        if p.op == MemoryOp::Noop || !p.needs_review() {
            return false;
        }
        if self.pending.iter().any(|q| q.id == p.id) {
            return false;
        }
        self.pending.push(p);
        true
    }

    pub fn extend(&mut self, proposals: Vec<Proposal>) -> usize {
        let mut n = 0;
        for p in proposals {
            if self.propose(p) {
                n += 1;
            }
        }
        n
    }

    pub fn pending(&self) -> &[Proposal] {
        &self.pending
    }

    pub fn pending_for(&self, scope: &str, scope_id: &str) -> Vec<&Proposal> {
        self.pending
            .iter()
            .filter(|p| p.scope == scope && p.scope_id == scope_id)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Approve one proposal: removes and returns it for the store to apply.
    pub fn approve(&mut self, id: &str) -> Option<Proposal> {
        let i = self.pending.iter().position(|p| p.id == id)?;
        Some(self.pending.remove(i))
    }

    /// Reject one proposal: removes and discards it.
    pub fn reject(&mut self, id: &str) -> bool {
        if let Some(i) = self.pending.iter().position(|p| p.id == id) {
            self.pending.remove(i);
            true
        } else {
            false
        }
    }

    /// Approve everything (badge "approve all"); returns them for the store.
    pub fn approve_all(&mut self) -> Vec<Proposal> {
        std::mem::take(&mut self.pending)
    }
}

/// True when a proposal applies without review under the auto-accept
/// settings (MEMORY.md §4: auto-accept for project memory is allowed,
/// default off for the user profile).
pub fn should_auto_apply(p: &Proposal, auto_accept_project: bool) -> bool {
    p.scope == "project" && auto_accept_project && p.op != MemoryOp::Noop
}

#[cfg(test)]
mod memory_write_tests {
    use super::*;
    use crate::redact::fixtures;

    fn redactor() -> Redactor {
        let mut r = Redactor::new();
        r.add_secrets(fixtures::all());
        r
    }

    #[test]
    fn memory_write_explicit_user_applies_immediately() {
        let out = remember_explicit(
            &redactor(),
            &RememberRequest::user("project", "p1", "Prefers sources from 2025+", "preference"),
        );
        match out {
            RememberOutcome::Applied(m) => {
                assert_eq!(m.scope, "project");
                assert_eq!(m.text, "Prefers sources from 2025+");
            }
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn memory_write_agent_notes_apply_immediately_but_project_needs_review() {
        let r = redactor();
        let notes = remember_explicit(
            &r,
            &RememberRequest::agent(
                "agent",
                "ada",
                "Scholar works better here",
                "lesson",
                "agent:ada",
            ),
        );
        assert!(matches!(notes, RememberOutcome::Applied(_)));
        let prop = remember_explicit(
            &r,
            &RememberRequest::agent(
                "project",
                "p1",
                "Use Scholar first",
                "decision",
                "agent:ada",
            ),
        );
        match prop {
            RememberOutcome::Proposed(p) => {
                assert_eq!(p.op, MemoryOp::Add);
                assert!(p.needs_review());
                assert_eq!(p.apply_action(), ApplyAction::Insert);
            }
            other => panic!("expected Proposed, got {other:?}"),
        }
        let user_prop = remember_explicit(
            &r,
            &RememberRequest::agent("user", "", "Works in fintech", "fact", "agent:ada"),
        );
        match user_prop {
            RememberOutcome::Proposed(p) => assert_eq!(p.scope, "user"),
            other => panic!("expected Proposed, got {other:?}"),
        }
    }

    #[test]
    fn memory_write_secrets_and_sensitive_sources_never_stored() {
        let r = redactor();
        // Every documented fake secret must be rejected on the memory path.
        for (name, value) in fixtures::all() {
            let text = format!("remember that my key is {value}");
            let out = remember_explicit(&r, &RememberRequest::user("project", "p1", &text, "fact"));
            assert!(
                matches!(out, RememberOutcome::Rejected { .. }),
                "fixture {name} was not rejected: {out:?}"
            );
        }
        // Unregistered provider-shaped keys are caught by patterns too.
        let out = remember_explicit(
            &r,
            &RememberRequest::user(
                "project",
                "p1",
                "key sk-ant-fresh-Unregistered-Key-9876543210",
                "fact",
            ),
        );
        assert!(matches!(out, RememberOutcome::Rejected { .. }));
        // Sensitive sources are never stored even without secrets.
        let mut req = RememberRequest::user("project", "p1", "perfectly innocent fact", "fact");
        req.sensitive_source = true;
        assert!(matches!(
            remember_explicit(&r, &req),
            RememberOutcome::Rejected { .. }
        ));
        // Extraction drops secret candidates instead of proposing them.
        let cands = vec![
            CandidateFact::new(&format!("token {}", fixtures::OPENAI.1), "fact"),
            CandidateFact::new("Prefers concise answers", "preference"),
        ];
        let (decisions, dropped) = extract_proposals(&r, "project", "p1", "agent:ada", &cands, &[]);
        assert_eq!(dropped, 1);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].op, MemoryOp::Add);
        // Defence in depth: direct proposal filtering.
        let (kept, dropped) = filter_secret_proposals(
            &r,
            vec![Proposal {
                id: "p1".into(),
                op: MemoryOp::Add,
                scope: "project".into(),
                scope_id: "p1".into(),
                kind: "fact".into(),
                text: fixtures::GITHUB.1.to_string(),
                target_id: None,
                source_message_id: String::new(),
                created_by: "agent:ada".into(),
                confidence: 1.0,
                reason: String::new(),
            }],
        );
        assert!(kept.is_empty() || !kept.iter().any(|p| p.text.contains(fixtures::GITHUB.1)));
        assert_eq!(kept.len() + dropped.len(), 1);
    }

    #[test]
    fn memory_write_extract_decides_add_update_delete_noop() {
        let r = redactor();
        let existing = vec![
            ExistingMemory::new(
                "m1",
                "The user prefers sources from 2025 onwards",
                "preference",
            ),
            ExistingMemory::new("m2", "The project deadline is in June", "fact"),
        ];
        let cands = vec![
            // Exact duplicate → NOOP.
            CandidateFact::new("The user prefers sources from 2025 onwards", "preference"),
            // Near-duplicate → UPDATE (supersede, history kept).
            CandidateFact::new(
                "The user prefers sources from 2025 onwards, ideally peer-reviewed",
                "preference",
            ),
            // Brand new → ADD.
            CandidateFact::new("The team chose sqlite-vec for embeddings", "decision"),
        ];
        let (decisions, dropped) =
            extract_proposals(&r, "project", "p1", "agent:ada", &cands, &existing);
        assert_eq!(dropped, 0);
        assert_eq!(decisions[0].op, MemoryOp::Noop);
        assert_eq!(decisions[0].target_id.as_deref(), Some("m1"));
        assert_eq!(decisions[0].apply_action(), ApplyAction::None);
        assert_eq!(decisions[1].op, MemoryOp::Update);
        assert_eq!(decisions[1].target_id.as_deref(), Some("m1"));
        assert_eq!(
            decisions[1].apply_action(),
            ApplyAction::Supersede {
                target_id: "m1".to_string()
            }
        );
        assert_eq!(decisions[2].op, MemoryOp::Add);
        assert_eq!(decisions[2].apply_action(), ApplyAction::Insert);

        // Retraction → DELETE the matched memory.
        let mut forget = CandidateFact::new("The project deadline is in June", "fact");
        forget.forget = true;
        let (decisions, _) =
            extract_proposals(&r, "project", "p1", "agent:ada", &[forget], &existing);
        assert_eq!(decisions[0].op, MemoryOp::Delete);
        assert_eq!(decisions[0].target_id.as_deref(), Some("m2"));
        assert_eq!(
            decisions[0].apply_action(),
            ApplyAction::Delete {
                target_id: "m2".to_string()
            }
        );
    }

    #[test]
    fn memory_write_review_queue_approve_reject_all() {
        let r = redactor();
        let mut q = ReviewQueue::new();
        // NOOPs never enter the queue.
        assert!(!q.propose(Proposal {
            id: "n".into(),
            op: MemoryOp::Noop,
            scope: "project".into(),
            scope_id: "p1".into(),
            kind: "fact".into(),
            text: "dup".into(),
            target_id: Some("m1".into()),
            source_message_id: String::new(),
            created_by: "agent:ada".into(),
            confidence: 1.0,
            reason: String::new(),
        }));
        assert!(q.is_empty());
        let (decisions, _) = extract_proposals(
            &r,
            "project",
            "p1",
            "agent:ada",
            &[CandidateFact::new(
                "New finding: sqlite-vec wins",
                "decision",
            )],
            &[],
        );
        assert_eq!(q.extend(decisions), 1);
        assert_eq!(q.pending_for("project", "p1").len(), 1);
        assert_eq!(q.pending_for("project", "other").len(), 0);
        // Auto-accept defaults off for user profile.
        let user_p = Proposal {
            id: "u1".into(),
            op: MemoryOp::Add,
            scope: "user".into(),
            scope_id: String::new(),
            kind: "fact".into(),
            text: "Works in fintech".into(),
            target_id: None,
            source_message_id: String::new(),
            created_by: "agent:ada".into(),
            confidence: 1.0,
            reason: String::new(),
        };
        assert!(!should_auto_apply(&user_p, true));
        let proj_p = q.pending()[0].clone();
        assert!(!should_auto_apply(&proj_p, false));
        assert!(should_auto_apply(&proj_p, true));
        // Approve one, reject a re-added one, approve-all the rest.
        let id = proj_p.id.clone();
        let approved = q.approve(&id).expect("approve");
        assert_eq!(approved.id, id);
        assert!(q.is_empty());
        assert!(q.approve("missing").is_none());
        assert!(!q.reject("missing"));
        q.propose(proj_p.clone());
        assert!(q.reject(&proj_p.id));
        assert!(q.is_empty());
        q.propose(proj_p);
        q.propose(user_p);
        assert_eq!(q.len(), 2);
        let all = q.approve_all();
        assert_eq!(all.len(), 2);
        assert!(q.is_empty());
    }

    #[test]
    fn memory_write_rejects_empty_and_unknown_scope() {
        let r = redactor();
        let out = remember_explicit(&r, &RememberRequest::user("project", "p1", "   ", "fact"));
        assert!(matches!(out, RememberOutcome::Rejected { .. }));
        let out = remember_explicit(&r, &RememberRequest::user("org", "o1", "some fact", "fact"));
        assert!(matches!(out, RememberOutcome::Rejected { .. }));
    }
}
