//! Approval cards + audit history (PRD §5.5).
//!
//! Toolkit-free state so tests run headless: [`ApprovalsCenter`] holds
//! pending [`ApprovalCard`]s (approve-once / always-allow / deny with an
//! action preview) and a reachable [`AuditEntry`] log with redacted
//! arguments and outcomes. [`approval_notification`] fires an OS
//! notification when an approval arrives while the window is hidden.

use synapse_core::redact::Redactor;

use crate::lifecycle::WindowVisibility;

/// What the user picked on an approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalChoice {
    /// Run this one call only.
    ApproveOnce,
    /// Approve and remember allow(tool, pattern) for the future.
    AlwaysAllow,
    /// Refuse the call.
    Deny,
}

/// How an audited call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditOutcome {
    Pending,
    ApprovedOnce,
    AlwaysAllowed,
    Denied,
    AutoAllowed,
    Done,
    Error(String),
}

impl AuditOutcome {
    pub fn status_str(&self) -> String {
        match self {
            Self::Pending => "pending".into(),
            Self::ApprovedOnce => "approved".into(),
            Self::AlwaysAllowed => "always_allowed".into(),
            Self::Denied => "denied".into(),
            Self::AutoAllowed => "auto_allowed".into(),
            Self::Done => "done".into(),
            Self::Error(e) => format!("error: {e}"),
        }
    }
}

/// One pending approval card with a useful action preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCard {
    pub id: String,
    pub agent_id: String,
    pub tool: String,
    /// Redacted JSON args (never contains secrets).
    pub args_redacted: String,
    /// Short human preview: diff for writes, full command for shell.
    pub preview: String,
    pub redacted_count: usize,
    pub created_at: u64,
}

impl ApprovalCard {
    /// One-line title, e.g. "Ada wants to run `cargo test`".
    pub fn title(&self) -> String {
        match self.tool.as_str() {
            "shell" => format!(
                "{} wants to run `{}`",
                self.agent_id,
                preview_head(&self.preview, 80)
            ),
            "write_file" | "edit_file" => {
                format!(
                    "{} wants to write {}",
                    self.agent_id,
                    preview_head(&self.preview, 80)
                )
            }
            _ => format!("{} wants to call {}", self.agent_id, self.tool),
        }
    }
}

/// One audit row: redacted args + outcome, reachable from the Activity view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub id: String,
    pub agent_id: String,
    pub tool: String,
    pub args_redacted: String,
    pub outcome: AuditOutcome,
    pub approved_by: String,
    pub result_summary: String,
    pub redacted_count: usize,
}

/// OS notification payload for a pending approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalNotification {
    pub title: String,
    pub body: String,
}

/// Pending approvals + audit history + remembered always-allow rules.
#[derive(Debug, Default)]
pub struct ApprovalsCenter {
    pending: Vec<ApprovalCard>,
    audit: Vec<AuditEntry>,
    allow_rules: Vec<(String, String)>,
    next_id: u64,
}

impl ApprovalsCenter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending(&self) -> &[ApprovalCard] {
        &self.pending
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Reachable audit view: newest last; filter by tool/agent substring.
    pub fn audit(&self) -> &[AuditEntry] {
        &self.audit
    }

    pub fn audit_filtered(&self, query: &str) -> Vec<&AuditEntry> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.audit.iter().collect();
        }
        self.audit
            .iter()
            .filter(|e| {
                e.tool.to_lowercase().contains(&q) || e.agent_id.to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn allow_rules(&self) -> &[(String, String)] {
        &self.allow_rules
    }

    fn alloc_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}_{}", self.next_id)
    }

    fn is_always_allowed(&self, tool: &str, args: &str) -> bool {
        self.allow_rules
            .iter()
            .any(|(t, p)| (t == "*" || t == tool) && (p == "*" || args.contains(p.as_str())))
    }

    /// Request approval for a tool call. Args are redacted before storage.
    /// Always-allow matches auto-approve (audited as `AutoAllowed`) and
    /// return `None`; otherwise returns the new card id.
    pub fn request(
        &mut self,
        agent_id: &str,
        tool: &str,
        args_json: &str,
        redactor: &Redactor,
    ) -> Option<String> {
        let redacted = redactor.redact(args_json);
        let preview = preview_for(tool, &redacted.text);
        let entry_id = self.alloc_id("tc");
        if self.is_always_allowed(tool, args_json) {
            self.audit.push(AuditEntry {
                id: entry_id,
                agent_id: agent_id.into(),
                tool: tool.into(),
                args_redacted: redacted.text,
                outcome: AuditOutcome::AutoAllowed,
                approved_by: "rule".into(),
                result_summary: String::new(),
                redacted_count: redacted.count,
            });
            return None;
        }
        let card_id = entry_id.clone();
        self.pending.push(ApprovalCard {
            id: card_id.clone(),
            agent_id: agent_id.into(),
            tool: tool.into(),
            args_redacted: redacted.text.clone(),
            preview,
            redacted_count: redacted.count,
            created_at: now_secs(),
        });
        self.audit.push(AuditEntry {
            id: entry_id,
            agent_id: agent_id.into(),
            tool: tool.into(),
            args_redacted: redacted.text,
            outcome: AuditOutcome::Pending,
            approved_by: String::new(),
            result_summary: String::new(),
            redacted_count: redacted.count,
        });
        Some(card_id)
    }

    /// Decide a pending card. Records the redacted outcome in the audit log;
    /// `AlwaysAllow` remembers an allow rule. Returns the choice, or `None`
    /// when the id is unknown.
    pub fn decide(&mut self, id: &str, choice: ApprovalChoice, by: &str) -> Option<AuditOutcome> {
        let pos = self.pending.iter().position(|c| c.id == id)?;
        let card = self.pending.remove(pos);
        let outcome = match choice {
            ApprovalChoice::ApproveOnce => AuditOutcome::ApprovedOnce,
            ApprovalChoice::AlwaysAllow => {
                // Remember a narrow rule: tool + path/command hint from preview.
                let pattern = allow_pattern_for(&card.tool, &card.args_redacted);
                if !self
                    .allow_rules
                    .iter()
                    .any(|(t, p)| t == &card.tool && p == &pattern)
                {
                    self.allow_rules.push((card.tool.clone(), pattern));
                }
                AuditOutcome::AlwaysAllowed
            }
            ApprovalChoice::Deny => AuditOutcome::Denied,
        };
        if let Some(entry) = self.audit.iter_mut().find(|e| e.id == id) {
            entry.outcome = outcome.clone();
            entry.approved_by = by.into();
        }
        Some(outcome)
    }

    /// Record the final result of a decided call (done/error) by audit id.
    pub fn record_result(&mut self, id: &str, summary: &str, is_error: bool) -> bool {
        if let Some(entry) = self.audit.iter_mut().find(|e| e.id == id) {
            entry.result_summary = summary.into();
            if is_error {
                entry.outcome = AuditOutcome::Error(summary.into());
            } else if matches!(
                entry.outcome,
                AuditOutcome::ApprovedOnce
                    | AuditOutcome::AlwaysAllowed
                    | AuditOutcome::AutoAllowed
            ) {
                entry.outcome = AuditOutcome::Done;
            }
            return true;
        }
        false
    }

    /// Persist pending + audit rows to the store's `tool_calls` table
    /// (args already redacted). Returns rows written.
    pub fn save_to_store(&self, store: &synapse_store::Store) -> usize {
        let mut n = 0;
        for e in &self.audit {
            let row = synapse_store::NewToolCall {
                id: e.id.clone(),
                message_id: String::new(),
                agent_id: e.agent_id.clone(),
                tool: e.tool.clone(),
                args: e.args_redacted.clone(),
                status: e.outcome.status_str(),
                result_summary: e.result_summary.clone(),
                approved_by: e.approved_by.clone(),
            };
            // Insert-or-refresh: update when the id already exists.
            let already = store
                .list_tool_calls(i64::MAX, 0)
                .map(|rows| rows.iter().any(|r| r.id == e.id))
                .unwrap_or(false);
            let ok = if already {
                store
                    .update_tool_call(&e.id, &row.status, &row.result_summary, &row.approved_by)
                    .is_ok()
            } else {
                store.record_tool_call(&row).is_ok()
            };
            if ok {
                n += 1;
            }
        }
        n
    }

    /// Rebuild the audit view from the store (reachable after restart).
    pub fn load_audit_from_store(&mut self, store: &synapse_store::Store) {
        if let Ok(rows) = store.list_tool_calls(i64::MAX, 0) {
            self.audit = rows
                .into_iter()
                .map(|r| AuditEntry {
                    id: r.id,
                    agent_id: r.agent_id,
                    tool: r.tool,
                    args_redacted: r.args,
                    outcome: match r.status.as_str() {
                        "approved" => AuditOutcome::ApprovedOnce,
                        "always_allowed" => AuditOutcome::AlwaysAllowed,
                        "denied" => AuditOutcome::Denied,
                        "auto_allowed" => AuditOutcome::AutoAllowed,
                        "done" => AuditOutcome::Done,
                        s if s.starts_with("error") => {
                            AuditOutcome::Error(r.result_summary.clone())
                        }
                        _ => AuditOutcome::Pending,
                    },
                    approved_by: r.approved_by,
                    result_summary: r.result_summary,
                    redacted_count: 0,
                })
                .collect();
        }
    }
}

/// Build the card preview: unified-diff style for file writes, full command
/// for shell, truncated args otherwise.
pub fn preview_for(tool: &str, args_redacted: &str) -> String {
    let parsed: serde_json::Value =
        serde_json::from_str(args_redacted).unwrap_or(serde_json::Value::Null);
    match tool {
        "shell" => {
            let cmd = parsed
                .get("command")
                .or_else(|| parsed.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or(args_redacted);
            // Full command, capped so the card stays readable.
            truncate(cmd, 800)
        }
        "write_file" | "edit_file" => {
            let path = parsed
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown path)");
            let old = parsed
                .get("old")
                .or_else(|| parsed.get("before"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let new = parsed
                .get("content")
                .or_else(|| parsed.get("new"))
                .or_else(|| parsed.get("after"))
                .and_then(|v| v.as_str())
                .unwrap_or(args_redacted);
            if old.is_empty() {
                truncate(&format!("{path}\n+ {new}"), 800)
            } else {
                truncate(&format!("{path}\n- {old}\n+ {new}"), 800)
            }
        }
        _ => truncate(args_redacted, 500),
    }
}

fn allow_pattern_for(tool: &str, args_redacted: &str) -> String {
    let parsed: serde_json::Value =
        serde_json::from_str(args_redacted).unwrap_or(serde_json::Value::Null);
    match tool {
        "shell" => parsed
            .get("command")
            .and_then(|v| v.as_str())
            .map(|c| c.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "*".into()),
        "write_file" | "edit_file" => parsed
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("*")
            .into(),
        _ => "*".into(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}…", &s[..max])
}

fn preview_head(s: &str, max: usize) -> String {
    let first = s.lines().next().unwrap_or(s);
    truncate(first, max)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Notify when an approval is needed while the window is hidden/destroyed.
/// Returns `Some` only when `visibility != Visible` so foreground approvals
/// stay inline cards; the shell shows this as an OS notification.
pub fn approval_notification(
    card: &ApprovalCard,
    visibility: WindowVisibility,
) -> Option<ApprovalNotification> {
    if visibility == WindowVisibility::Visible {
        return None;
    }
    Some(ApprovalNotification {
        title: "Approval needed".into(),
        body: format!("{} — {}", card.agent_id, card.title()),
    })
}

#[cfg(test)]
mod approvals_tests {
    use super::*;
    use synapse_core::redact::{fixtures, Redactor};

    fn redactor() -> Redactor {
        let mut r = Redactor::new();
        r.add_secrets(fixtures::all());
        r
    }

    #[test]
    fn approvals_card_preview_and_three_choices() {
        let mut c = ApprovalsCenter::new();
        let r = redactor();
        // Shell: full command preview.
        let id = c
            .request(
                "ada",
                "shell",
                r##"{"command": "cargo test -p synapse-app"}"##,
                &r,
            )
            .expect("card");
        let card = c.pending().iter().find(|x| x.id == id).unwrap();
        assert!(
            card.preview.contains("cargo test -p synapse-app"),
            "{}",
            card.preview
        );
        assert_eq!(c.pending_count(), 1);
        // Approve once resolves and audits the outcome.
        let out = c.decide(&id, ApprovalChoice::ApproveOnce, "user").unwrap();
        assert_eq!(out, AuditOutcome::ApprovedOnce);
        assert_eq!(c.pending_count(), 0);
        assert_eq!(c.audit().len(), 1);
        assert_eq!(c.audit()[0].outcome, AuditOutcome::ApprovedOnce);

        // File write: diff-style preview.
        let id2 = c
            .request(
                "mira",
                "write_file",
                r##"{"path": "reports/q.md", "content": "# Hello"}"##,
                &r,
            )
            .expect("card");
        let card2 = c.pending().iter().find(|x| x.id == id2).unwrap();
        assert!(card2.preview.contains("reports/q.md"), "{}", card2.preview);
        assert!(card2.preview.contains("# Hello"), "{}", card2.preview);
        // Always-allow remembers a rule and audits it.
        let out2 = c.decide(&id2, ApprovalChoice::AlwaysAllow, "user").unwrap();
        assert_eq!(out2, AuditOutcome::AlwaysAllowed);
        assert_eq!(c.allow_rules().len(), 1);

        // Deny path.
        let id3 = c
            .request("ada", "shell", r##"{"command": "rm -rf /tmp/x"}"##, &r)
            .expect("card");
        assert_eq!(
            c.decide(&id3, ApprovalChoice::Deny, "user").unwrap(),
            AuditOutcome::Denied
        );
        assert_eq!(
            c.audit_filtered("shell").len(),
            2,
            "audit view is reachable/filterable"
        );
    }

    #[test]
    fn approvals_args_redacted_and_outcomes_recorded() {
        let mut c = ApprovalsCenter::new();
        let r = redactor();
        let secret = fixtures::OPENAI.1;
        let args = format!(r##"{{"command": "deploy", "key": "{secret}"}}"##);
        let id = c.request("ada", "shell", &args, &r).expect("card");
        let card = c.pending().iter().find(|x| x.id == id).unwrap();
        assert!(
            !card.args_redacted.contains(secret),
            "{}",
            card.args_redacted
        );
        assert!(
            card.args_redacted.contains("[redacted:"),
            "{}",
            card.args_redacted
        );
        assert!(card.redacted_count >= 1);
        c.decide(&id, ApprovalChoice::ApproveOnce, "user");
        assert!(c.record_result(&id, "exit 0", false));
        let entry = c.audit().iter().find(|e| e.id == id).unwrap();
        assert_eq!(entry.outcome, AuditOutcome::Done);
        assert_eq!(entry.result_summary, "exit 0");
        assert!(!entry.args_redacted.contains(secret));
        // Error outcome keeps the message.
        let id2 = c
            .request("ada", "shell", r##"{"command": "x"}"##, &r)
            .expect("card");
        c.decide(&id2, ApprovalChoice::ApproveOnce, "user");
        assert!(c.record_result(&id2, "boom", true));
        assert!(matches!(
            c.audit().iter().find(|e| e.id == id2).unwrap().outcome,
            AuditOutcome::Error(_)
        ));
    }

    #[test]
    fn approvals_always_allow_auto_runs_and_notifies_when_hidden() {
        let mut c = ApprovalsCenter::new();
        let r = redactor();
        let id = c
            .request(
                "ada",
                "write_file",
                r##"{"path": "reports/a.md", "content": "hi"}"##,
                &r,
            )
            .expect("card");
        c.decide(&id, ApprovalChoice::AlwaysAllow, "user");
        // Same tool+path auto-approves next time (no new card) and is audited.
        let auto = c.request(
            "ada",
            "write_file",
            r##"{"path": "reports/a.md", "content": "v2"}"##,
            &r,
        );
        assert!(auto.is_none(), "always-allow should auto-run");
        assert_eq!(c.audit().last().unwrap().outcome, AuditOutcome::AutoAllowed);

        // Notification only while the window is hidden/destroyed.
        let card = ApprovalCard {
            id: "tc_9".into(),
            agent_id: "ada".into(),
            tool: "shell".into(),
            args_redacted: "{}".into(),
            preview: "cargo test".into(),
            redacted_count: 0,
            created_at: 0,
        };
        assert!(approval_notification(&card, WindowVisibility::Visible).is_none());
        let n = approval_notification(&card, WindowVisibility::Hidden).expect("notify hidden");
        assert_eq!(n.title, "Approval needed");
        assert!(n.body.contains("ada"));
        let n2 =
            approval_notification(&card, WindowVisibility::Destroyed).expect("notify destroyed");
        assert!(n2.body.contains("ada"));
    }

    #[test]
    fn approvals_audit_persists_to_store_redacted() {
        let mut c = ApprovalsCenter::new();
        let r = redactor();
        let secret = fixtures::OPENAI.1;
        let args = format!(r##"{{"command": "run", "key": "{secret}"}}"##);
        let id = c.request("ada", "shell", &args, &r).expect("card");
        c.decide(&id, ApprovalChoice::ApproveOnce, "user");
        let store = synapse_store::Store::open_in_memory().expect("store");
        assert_eq!(c.save_to_store(&store), 1);
        let rows = store.list_tool_calls(10, 0).expect("list");
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].args.contains(secret), "{}", rows[0].args);
        assert!(rows[0].args.contains("[redacted:"), "{}", rows[0].args);
        // Reachable after restart: reload the audit view from the store.
        let mut reopened = ApprovalsCenter::new();
        reopened.load_audit_from_store(&store);
        assert_eq!(reopened.audit().len(), 1);
        assert_eq!(reopened.audit()[0].id, id);
        // Unknown ids change nothing.
        assert!(c.decide("missing", ApprovalChoice::Deny, "user").is_none());
        assert!(!c.record_result("missing", "x", false));
    }
}
