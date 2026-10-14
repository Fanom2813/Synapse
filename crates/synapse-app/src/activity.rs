//! Activity view: runs, tool calls, and approvals per agent/project
//! (see docs/MILESTONES.md task 60, docs/TEAM-PROTOCOL.md §9).
//!
//! Toolkit-free state so tests run headless: [`ActivityView`] lists run rows
//! with pause/resume/cancel/retry controls (each returning the visible
//! resulting status), plus filtered tool-call and approval rows.

use synapse_core::{RunMode, RunPhase, RunState, RunStatus};

use crate::approvals::{ApprovalCard, ApprovalsCenter, AuditEntry};

/// One run row in the activity list.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub state: RunState,
    pub project_id: String,
}

impl RunRow {
    pub fn new(project_id: &str, state: RunState) -> Self {
        Self {
            state,
            project_id: project_id.into(),
        }
    }

    /// Visible status string shown in the row (e.g. "paused").
    pub fn status_label(&self) -> &'static str {
        match self.state.status {
            RunStatus::Idle => "idle",
            RunStatus::Running => "running",
            RunStatus::Paused => "paused",
            RunStatus::Stopped => "cancelled",
            RunStatus::Failed => "failed",
            RunStatus::Interrupted => "interrupted",
            RunStatus::Done => "done",
        }
    }

    pub fn phase_label(&self) -> String {
        format!("{:?}", self.state.phase()).to_lowercase()
    }
}

/// One tool-call row (mirrors the `tool_calls` audit table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRow {
    pub id: String,
    pub project_id: String,
    pub agent_id: String,
    pub tool: String,
    pub status: String,
    pub result_summary: String,
}

impl ToolCallRow {
    pub fn status_label(&self) -> &str {
        &self.status
    }
}

/// Scope filter: per project and/or per agent.
#[derive(Debug, Clone, Default)]
pub struct ActivityFilter {
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
}

impl ActivityFilter {
    pub fn matches_run(&self, row: &RunRow) -> bool {
        if let Some(p) = &self.project_id {
            if &row.project_id != p {
                return false;
            }
        }
        if let Some(a) = &self.agent_id {
            if !row.state.members.iter().any(|m| m == a) {
                return false;
            }
        }
        true
    }

    pub fn matches_tool(&self, row: &ToolCallRow) -> bool {
        if let Some(p) = &self.project_id {
            if &row.project_id != p {
                return false;
            }
        }
        if let Some(a) = &self.agent_id {
            if &row.agent_id != a {
                return false;
            }
        }
        true
    }

    pub fn matches_approval(&self, card: &ApprovalCard) -> bool {
        if let Some(a) = &self.agent_id {
            if &card.agent_id != a {
                return false;
            }
        }
        true
    }
}

/// Activity tab state: run list + tool-call list + approvals center.
#[derive(Debug, Default)]
pub struct ActivityView {
    runs: Vec<RunRow>,
    tool_calls: Vec<ToolCallRow>,
    approvals: ApprovalsCenter,
    filter: ActivityFilter,
}

impl ActivityView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_filter(&mut self, filter: ActivityFilter) {
        self.filter = filter;
    }

    pub fn filter(&self) -> &ActivityFilter {
        &self.filter
    }

    pub fn add_run(&mut self, project_id: &str, state: RunState) {
        if let Some(row) = self.runs.iter_mut().find(|r| r.state.id == state.id) {
            row.state = state;
            row.project_id = project_id.into();
        } else {
            self.runs.push(RunRow::new(project_id, state));
        }
    }

    pub fn add_tool_call(&mut self, row: ToolCallRow) {
        if let Some(existing) = self.tool_calls.iter_mut().find(|t| t.id == row.id) {
            *existing = row;
        } else {
            self.tool_calls.push(row);
        }
    }

    pub fn approvals_mut(&mut self) -> &mut ApprovalsCenter {
        &mut self.approvals
    }

    pub fn approvals(&self) -> &ApprovalsCenter {
        &self.approvals
    }

    /// Runs in scope, newest last. Empty when nothing matches (empty state).
    pub fn runs(&self) -> Vec<&RunRow> {
        self.runs
            .iter()
            .filter(|r| self.filter.matches_run(r))
            .collect()
    }

    pub fn tool_calls(&self) -> Vec<&ToolCallRow> {
        self.tool_calls
            .iter()
            .filter(|t| self.filter.matches_tool(t))
            .collect()
    }

    pub fn pending_approvals(&self) -> Vec<&ApprovalCard> {
        self.approvals
            .pending()
            .iter()
            .filter(|c| self.filter.matches_approval(c))
            .collect()
    }

    pub fn audit(&self) -> &[AuditEntry] {
        self.approvals.audit()
    }

    fn find_run_mut(&mut self, run_id: &str) -> Option<&mut RunRow> {
        self.runs.iter_mut().find(|r| r.state.id == run_id)
    }

    pub fn run_status(&self, run_id: &str) -> Option<&'static str> {
        self.runs
            .iter()
            .find(|r| r.state.id == run_id)
            .map(|r| r.status_label())
    }

    /// Pause a running run. Returns the visible resulting status.
    pub fn pause_run(&mut self, run_id: &str) -> Option<&'static str> {
        let row = self.find_run_mut(run_id)?;
        row.state.pause();
        Some(row.status_label())
    }

    /// Resume a paused/interrupted/idle run. Returns the visible status.
    pub fn resume_run(&mut self, run_id: &str) -> Option<&'static str> {
        let row = self.find_run_mut(run_id)?;
        row.state.resume();
        Some(row.status_label())
    }

    /// Cancel (stop) a run. Returns the visible resulting status.
    pub fn cancel_run(&mut self, run_id: &str) -> Option<&'static str> {
        let row = self.find_run_mut(run_id)?;
        row.state.stop();
        Some(row.status_label())
    }

    /// Retry a failed/cancelled/interrupted/finished run: back to running.
    /// Running/paused runs are left untouched. Returns visible status.
    pub fn retry_run(&mut self, run_id: &str) -> Option<&'static str> {
        let row = self.find_run_mut(run_id)?;
        match row.state.status {
            RunStatus::Failed | RunStatus::Stopped | RunStatus::Interrupted | RunStatus::Done => {
                row.state.status = RunStatus::Running;
            }
            _ => {}
        }
        Some(row.status_label())
    }

    /// One-line summary for the tab header, e.g. "2 runs · 1 pending approval".
    pub fn summary(&self) -> String {
        let runs = self.runs().len();
        let pending = self.pending_approvals().len();
        format!(
            "{} run{} · {} pending approval{}",
            runs,
            if runs == 1 { "" } else { "s" },
            pending,
            if pending == 1 { "" } else { "s" }
        )
    }

    /// Load tool-call rows from the store so the tab survives restarts.
    pub fn load_tool_calls_from_store(&mut self, project_id: &str, store: &synapse_store::Store) {
        if let Ok(rows) = store.list_tool_calls(i64::MAX, 0) {
            for r in rows {
                self.add_tool_call(ToolCallRow {
                    id: r.id,
                    project_id: project_id.into(),
                    agent_id: r.agent_id,
                    tool: r.tool,
                    status: r.status,
                    result_summary: r.result_summary,
                });
            }
        }
    }
}

/// Visible status label for a [`RunStatus`] (used by the row + tests).
pub fn status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Idle => "idle",
        RunStatus::Running => "running",
        RunStatus::Paused => "paused",
        RunStatus::Stopped => "cancelled",
        RunStatus::Failed => "failed",
        RunStatus::Interrupted => "interrupted",
        RunStatus::Done => "done",
    }
}

#[allow(dead_code)]
fn _phase_uses(phase: RunPhase, mode: RunMode) -> bool {
    let _ = (phase, mode);
    true
}

#[cfg(test)]
mod activity_tab_tests {
    use super::*;
    use synapse_core::redact::Redactor;

    fn run(id: &str, members: &[&str]) -> RunState {
        RunState::new(
            id,
            RunMode::DeepResearch,
            members.iter().map(|s| s.to_string()).collect(),
            0,
        )
    }

    #[test]
    fn activity_tab_pause_resume_cancel_retry() {
        let mut view = ActivityView::new();
        view.add_run("proj-a", run("run-1", &["ada", "mira"]));
        // Initial visible status.
        assert_eq!(view.run_status("run-1"), Some("running"));

        // Pause shows paused; pausing twice stays paused.
        assert_eq!(view.pause_run("run-1"), Some("paused"));
        assert_eq!(view.run_status("run-1"), Some("paused"));
        assert_eq!(view.pause_run("run-1"), Some("paused"));

        // Resume shows running again.
        assert_eq!(view.resume_run("run-1"), Some("running"));

        // Cancel shows cancelled; resume does not revive a cancelled run.
        assert_eq!(view.cancel_run("run-1"), Some("cancelled"));
        assert_eq!(view.resume_run("run-1"), Some("cancelled"));

        // Retry brings it back to running with visible status.
        assert_eq!(view.retry_run("run-1"), Some("running"));

        // Failed runs retry too.
        view.add_run("proj-a", run("run-2", &["ada"]));
        view.cancel_run("run-2");
        assert_eq!(view.run_status("run-2"), Some("cancelled"));
        assert_eq!(view.retry_run("run-2"), Some("running"));

        // Unknown ids change nothing.
        assert!(view.pause_run("missing").is_none());
        assert!(view.resume_run("missing").is_none());
        assert!(view.cancel_run("missing").is_none());
        assert!(view.retry_run("missing").is_none());
        assert!(view.run_status("missing").is_none());
    }

    #[test]
    fn activity_tab_lists_runs_tools_and_approvals_per_agent_project() {
        let mut view = ActivityView::new();
        view.add_run("proj-a", run("run-1", &["ada"]));
        view.add_run("proj-b", run("run-2", &["mira"]));
        view.add_tool_call(ToolCallRow {
            id: "tc-1".into(),
            project_id: "proj-a".into(),
            agent_id: "ada".into(),
            tool: "shell".into(),
            status: "done".into(),
            result_summary: "exit 0".into(),
        });
        view.add_tool_call(ToolCallRow {
            id: "tc-2".into(),
            project_id: "proj-b".into(),
            agent_id: "mira".into(),
            tool: "write_file".into(),
            status: "pending".into(),
            result_summary: String::new(),
        });
        let redactor = Redactor::new();
        view.approvals_mut()
            .request("ada", "shell", r#"{"command": "cargo test"}"#, &redactor);

        // Unfiltered: everything visible.
        assert_eq!(view.runs().len(), 2);
        assert_eq!(view.tool_calls().len(), 2);
        assert_eq!(view.pending_approvals().len(), 1);
        assert!(view.summary().contains("2 runs"));

        // Per-project scope.
        view.set_filter(ActivityFilter {
            project_id: Some("proj-a".into()),
            agent_id: None,
        });
        assert_eq!(view.runs().len(), 1);
        assert_eq!(view.runs()[0].state.id, "run-1");
        assert_eq!(view.tool_calls().len(), 1);
        assert_eq!(view.tool_calls()[0].id, "tc-1");
        // Approvals carry no project; still visible unless agent-filtered.
        assert_eq!(view.pending_approvals().len(), 1);

        // Per-agent scope.
        view.set_filter(ActivityFilter {
            project_id: None,
            agent_id: Some("mira".into()),
        });
        assert_eq!(view.runs().len(), 1);
        assert_eq!(view.runs()[0].state.id, "run-2");
        assert_eq!(view.tool_calls().len(), 1);
        assert_eq!(view.tool_calls()[0].agent_id, "mira");
        assert_eq!(view.pending_approvals().len(), 0);

        // Empty state: unknown scope shows nothing but does not break.
        view.set_filter(ActivityFilter {
            project_id: Some("nope".into()),
            agent_id: None,
        });
        assert!(view.runs().is_empty());
        assert!(view.tool_calls().is_empty());
        assert_eq!(view.summary(), "0 runs · 1 pending approval");
    }

    #[test]
    fn activity_tab_failed_retry_then_pause_and_audit_visible() {
        let mut view = ActivityView::new();
        view.add_run("proj-a", run("run-9", &["ada"]));
        // Fail the run, retry, then pause: each step has a visible status.
        view.find_run_mut("run-9").unwrap().state.fail();
        assert_eq!(view.run_status("run-9"), Some("failed"));
        assert_eq!(status_label(RunStatus::Failed), "failed");
        assert_eq!(view.retry_run("run-9"), Some("running"));
        assert_eq!(view.pause_run("run-9"), Some("paused"));
        assert_eq!(view.run_status("run-9"), Some("paused"));

        // Decided approvals appear in the reachable audit trail.
        let redactor = Redactor::new();
        let id = view
            .approvals_mut()
            .request("ada", "shell", r#"{"command": "ls"}"#, &redactor)
            .expect("card");
        view.approvals_mut()
            .decide(&id, crate::approvals::ApprovalChoice::ApproveOnce, "user");
        assert_eq!(view.audit().len(), 1);
        // Tool calls reload from the store after restart.
        let store = synapse_store::Store::open_in_memory().expect("store");
        store
            .record_tool_call(&synapse_store::NewToolCall {
                id: "tc-9".into(),
                message_id: String::new(),
                agent_id: "ada".into(),
                tool: "shell".into(),
                args: "{}".into(),
                status: "done".into(),
                result_summary: "ok".into(),
                approved_by: "user".into(),
            })
            .expect("record");
        view.load_tool_calls_from_store("proj-a", &store);
        assert!(view
            .tool_calls()
            .iter()
            .any(|t| t.id == "tc-9" && t.status_label() == "done"));
    }
}
