//! Title-bar model (UI.md §1b): 40 px strip with sidebar toggle, project
//! switcher, run pill, command palette button, approvals inbox, budget meter,
//! right-panel toggle. Native window controls stay native per OS.
//!
//! Toolkit-free: the GPUI Kit view renders from this state; every action the
//! run popover offers (pause / resume / stop / open activity) is a method
//! here so tests drive it without a display.

/// Width below which the bar collapses: budget meter + palette move into
/// the overflow menu and the run pill shrinks to icon + avatars.
pub const NARROW_WIDTH: f32 = 720.0;

/// Max live-agent avatars shown in the run pill before "+N".
pub const MAX_PILL_AVATARS: usize = 5;

/// Run-pill state shown centred in the title bar.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum RunPill {
    /// No run: "Start a run" with the mode picker.
    #[default]
    Idle,
    /// Live run: mode icon + phase + progress + working-agent avatar ids.
    Running {
        mode: String,
        phase: String,
        /// 0.0..=1.0 progress; drives the 2 px line under the title bar.
        progress: f32,
        live_agent_ids: Vec<String>,
    },
}

impl RunPill {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    pub fn progress(&self) -> f32 {
        match self {
            Self::Idle => 0.0,
            Self::Running { progress, .. } => progress.clamp(0.0, 1.0),
        }
    }

    /// (shown avatar ids, overflow count) honouring [`MAX_PILL_AVATARS`].
    pub fn visible_agents(&self) -> (Vec<String>, usize) {
        match self {
            Self::Idle => (Vec::new(), 0),
            Self::Running { live_agent_ids, .. } => {
                if live_agent_ids.len() <= MAX_PILL_AVATARS {
                    (live_agent_ids.clone(), 0)
                } else {
                    (
                        live_agent_ids[..MAX_PILL_AVATARS].to_vec(),
                        live_agent_ids.len() - MAX_PILL_AVATARS,
                    )
                }
            }
        }
    }

    /// Centre text: "Start a run" idle, else "mode · phase".
    pub fn label(&self, paused: bool) -> String {
        match self {
            Self::Idle => "Start a run".to_string(),
            Self::Running { mode, phase, .. } if paused => {
                format!("Paused · {mode} · {phase}")
            }
            Self::Running { mode, phase, .. } => format!("{mode} · {phase}"),
        }
    }
}

/// Budget-meter state: spent vs budget for the current run.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BudgetMeter {
    pub spent_usd: f64,
    pub budget_usd: f64,
}

impl BudgetMeter {
    /// Green < 80 %, amber 80-100 %, red over budget.
    pub fn level(self) -> BudgetLevel {
        if self.budget_usd <= 0.0 {
            return BudgetLevel::Ok;
        }
        let frac = self.spent_usd / self.budget_usd;
        if frac >= 1.0 {
            BudgetLevel::Over
        } else if frac >= 0.8 {
            BudgetLevel::Warning
        } else {
            BudgetLevel::Ok
        }
    }

    /// Tiny ring label: "$0.42 / $1.00", or "—" with no budget.
    pub fn text(self) -> String {
        if self.budget_usd <= 0.0 {
            return "—".to_string();
        }
        format!("${:.2} / ${:.2}", self.spent_usd, self.budget_usd)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetLevel {
    Ok,
    Warning,
    Over,
}

/// One pending approval in the inbox popover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalItem {
    pub id: String,
    pub title: String,
    pub agent_id: String,
}

/// Run popover actions (UI.md §1b-3): pause/resume/stop + open Activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAction {
    Pause,
    Resume,
    Stop,
    OpenActivity,
}

/// How the bar lays out at a given width. Narrow windows collapse the
/// budget meter and palette into the "⋯" overflow menu first; the run pill
/// shrinks to icon + avatars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleBarLayout {
    pub show_budget: bool,
    pub show_palette: bool,
    pub compact_run_pill: bool,
    pub show_overflow: bool,
}

/// Full title-bar state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TitleBar {
    pub project_name: Option<String>,
    pub run: RunPill,
    /// True while the live run is paused (pause/resume in the popover).
    pub run_paused: bool,
    pub pending_approvals: u32,
    /// Detail rows for the inbox popover; `pending_approvals` stays the
    /// canonical count so older callers keep working.
    pub approvals: Vec<ApprovalItem>,
    pub budget: BudgetMeter,
    /// Phase timeline for the run popover (oldest first).
    pub phase_history: Vec<String>,
    /// Narrow window: palette + budget collapse into the overflow menu.
    pub collapsed_overflow: bool,
}

impl TitleBar {
    /// Text for the approvals inbox bell badge; empty when zero.
    pub fn approvals_badge(&self) -> String {
        if self.pending_approvals == 0 {
            String::new()
        } else if self.pending_approvals > 99 {
            "99+".to_string()
        } else {
            self.pending_approvals.to_string()
        }
    }

    /// Project switcher label; cross-fades on switch in the view.
    pub fn project_label(&self) -> String {
        self.project_name
            .clone()
            .unwrap_or_else(|| "Select project".to_string())
    }

    /// Run pill centre text (paused state included).
    pub fn run_label(&self) -> String {
        self.run.label(self.run_paused)
    }

    /// Whether the bar is in the narrow/overflow arrangement. Either the
    /// explicit flag (set from the window width) or a width at/under
    /// [`NARROW_WIDTH`] counts.
    pub fn is_narrow(&self, width: f32) -> bool {
        self.collapsed_overflow || width <= NARROW_WIDTH
    }

    /// Layout decision for `width` (logical px).
    pub fn layout_for_width(&self, width: f32) -> TitleBarLayout {
        let narrow = self.is_narrow(width);
        TitleBarLayout {
            show_budget: !narrow,
            show_palette: !narrow,
            compact_run_pill: narrow,
            show_overflow: narrow,
        }
    }

    /// Actions the run popover offers right now. Idle offers nothing
    /// (the pill opens the mode picker instead); a live run offers
    /// pause/resume + stop + open Activity.
    pub fn run_actions(&self) -> Vec<RunAction> {
        match &self.run {
            RunPill::Idle => Vec::new(),
            RunPill::Running { .. } if self.run_paused => {
                vec![RunAction::Resume, RunAction::Stop, RunAction::OpenActivity]
            }
            RunPill::Running { .. } => {
                vec![RunAction::Pause, RunAction::Stop, RunAction::OpenActivity]
            }
        }
    }

    /// Apply a popover action. Pause/resume toggle [`Self::run_paused`];
    /// stop returns to idle; OpenActivity is a navigation no-op here and
    /// returns false so the view can select the Activity tab itself.
    /// Returns true when state changed.
    pub fn apply_run_action(&mut self, action: RunAction) -> bool {
        match action {
            RunAction::Pause => {
                if matches!(self.run, RunPill::Running { .. }) && !self.run_paused {
                    self.run_paused = true;
                    true
                } else {
                    false
                }
            }
            RunAction::Resume => {
                if self.run_paused {
                    self.run_paused = false;
                    true
                } else {
                    false
                }
            }
            RunAction::Stop => {
                if matches!(self.run, RunPill::Running { .. }) {
                    self.run = RunPill::Idle;
                    self.run_paused = false;
                    self.phase_history.clear();
                    true
                } else {
                    false
                }
            }
            RunAction::OpenActivity => false,
        }
    }

    /// Record a phase change: updates the pill and pushes the timeline.
    pub fn set_phase(&mut self, phase: &str, progress: f32) {
        if let RunPill::Running {
            phase: current,
            progress: p,
            ..
        } = &mut self.run
        {
            *current = phase.to_string();
            *p = progress.clamp(0.0, 1.0);
        }
        self.phase_history.push(phase.to_string());
    }

    /// Replace the live-agent avatar ids on a running pill.
    pub fn set_live_agents(&mut self, ids: &[&str]) {
        if let RunPill::Running { live_agent_ids, .. } = &mut self.run {
            *live_agent_ids = ids.iter().map(|s| s.to_string()).collect();
        }
    }

    /// Approve (`approve=true`) or deny one inbox item. Returns false when
    /// the id is unknown. Keeps `pending_approvals` in sync with the list.
    pub fn decide_approval(&mut self, id: &str, _approve: bool) -> bool {
        if let Some(pos) = self.approvals.iter().position(|a| a.id == id) {
            self.approvals.remove(pos);
            self.pending_approvals = self.pending_approvals.saturating_sub(1);
            // Keep the two in sync when the list is the source of truth.
            let n = self.approvals.len() as u32;
            if n < self.pending_approvals && !self.approvals.is_empty() {
                self.pending_approvals = n;
            } else if self.approvals.is_empty() {
                self.pending_approvals = 0;
            }
            true
        } else if self.pending_approvals > 0 && self.approvals.is_empty() {
            // Count-only state (no detail rows): one decision clears one.
            self.pending_approvals -= 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_titlebar_badges_and_budget() {
        let mut t = TitleBar::default();
        assert_eq!(t.approvals_badge(), "");
        assert!(!t.run.is_running());
        t.pending_approvals = 3;
        assert_eq!(t.approvals_badge(), "3");
        t.pending_approvals = 150;
        assert_eq!(t.approvals_badge(), "99+");
        assert_eq!(BudgetMeter::default().level(), BudgetLevel::Ok);
        let m = BudgetMeter {
            spent_usd: 0.85,
            budget_usd: 1.0,
        };
        assert_eq!(m.level(), BudgetLevel::Warning);
        let m = BudgetMeter {
            spent_usd: 1.5,
            budget_usd: 1.0,
        };
        assert_eq!(m.level(), BudgetLevel::Over);
    }

    #[test]
    fn title_bar_live_status_shows_project_phase_agents_budget_approvals() {
        let mut t = TitleBar {
            project_name: Some("Market study".to_string()),
            run: RunPill::Running {
                mode: "Research".to_string(),
                phase: "Searching".to_string(),
                progress: 0.4,
                live_agent_ids: vec!["a1".into(), "a2".into(), "a3".into()],
            },
            pending_approvals: 2,
            budget: BudgetMeter {
                spent_usd: 0.42,
                budget_usd: 1.0,
            },
            ..Default::default()
        };
        assert_eq!(t.project_label(), "Market study");
        assert_eq!(t.run_label(), "Research · Searching");
        assert!((t.run.progress() - 0.4).abs() < f32::EPSILON);
        let (shown, overflow) = t.run.visible_agents();
        assert_eq!(shown.len(), 3);
        assert_eq!(overflow, 0);
        assert_eq!(t.approvals_badge(), "2");
        assert_eq!(t.budget.level(), BudgetLevel::Ok);
        assert_eq!(t.budget.text(), "$0.42 / $1.00");
        assert_eq!(TitleBar::default().project_label(), "Select project");

        // Avatar stack caps at 5 with +N.
        t.set_live_agents(&["a1", "a2", "a3", "a4", "a5", "a6", "a7"]);
        let (shown, overflow) = t.run.visible_agents();
        assert_eq!(shown.len(), MAX_PILL_AVATARS);
        assert_eq!(overflow, 2);

        // Phase changes update the pill and the popover timeline.
        t.set_phase("Reading", 0.6);
        assert_eq!(t.run_label(), "Research · Reading");
        assert_eq!(t.phase_history, vec!["Reading".to_string()]);
    }

    #[test]
    fn title_bar_run_popover_pause_resume_stop_and_activity() {
        let mut t = TitleBar {
            run: RunPill::Running {
                mode: "Research".to_string(),
                phase: "Searching".to_string(),
                progress: 0.2,
                live_agent_ids: vec!["lead".into()],
            },
            ..Default::default()
        };
        assert_eq!(
            t.run_actions(),
            vec![RunAction::Pause, RunAction::Stop, RunAction::OpenActivity]
        );
        assert!(t.apply_run_action(RunAction::Pause));
        assert!(t.run_paused);
        assert_eq!(t.run_label(), "Paused · Research · Searching");
        assert!(!t.apply_run_action(RunAction::Pause));
        assert_eq!(
            t.run_actions(),
            vec![RunAction::Resume, RunAction::Stop, RunAction::OpenActivity]
        );
        assert!(t.apply_run_action(RunAction::Resume));
        assert!(!t.run_paused);
        // OpenActivity is navigation: no state change here.
        assert!(!t.apply_run_action(RunAction::OpenActivity));
        assert!(t.run.is_running());
        assert!(t.apply_run_action(RunAction::Stop));
        assert_eq!(t.run, RunPill::Idle);
        assert!(t.run_actions().is_empty());
        assert!(!t.apply_run_action(RunAction::Stop));

        // Idle pill offers nothing (it opens the mode picker instead).
        assert!(TitleBar::default().run_actions().is_empty());
    }

    #[test]
    fn title_bar_approvals_popover_approve_deny_inline() {
        let mut t = TitleBar {
            pending_approvals: 2,
            approvals: vec![
                ApprovalItem {
                    id: "ap1".into(),
                    title: "Run `cargo test`".into(),
                    agent_id: "lead".into(),
                },
                ApprovalItem {
                    id: "ap2".into(),
                    title: "Read secrets.env".into(),
                    agent_id: "scout".into(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(t.approvals_badge(), "2");
        assert!(t.decide_approval("ap1", true));
        assert_eq!(t.approvals.len(), 1);
        assert!(t.decide_approval("ap2", false));
        assert_eq!(t.approvals_badge(), "");
        assert!(!t.decide_approval("missing", true));

        // Count-only state still clears one per decision.
        let mut c = TitleBar {
            pending_approvals: 1,
            ..Default::default()
        };
        assert!(c.decide_approval("anything", true));
        assert_eq!(c.pending_approvals, 0);
    }

    #[test]
    fn title_bar_overflow_collapses_at_narrow_widths() {
        let t = TitleBar::default();
        let wide = t.layout_for_width(1280.0);
        assert!(wide.show_budget);
        assert!(wide.show_palette);
        assert!(!wide.compact_run_pill);
        assert!(!wide.show_overflow);

        let narrow = t.layout_for_width(600.0);
        assert!(!narrow.show_budget);
        assert!(!narrow.show_palette);
        assert!(narrow.compact_run_pill);
        assert!(narrow.show_overflow);

        // Explicit flag forces overflow even at a wide width.
        let forced = TitleBar {
            collapsed_overflow: true,
            ..Default::default()
        };
        let layout = forced.layout_for_width(1280.0);
        assert!(layout.show_overflow);
        assert!(layout.compact_run_pill);
    }
}
