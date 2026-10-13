//! Title-bar model (UI.md §1b): 40 px strip with sidebar toggle, project
//! switcher, run pill, command palette button, approvals inbox, budget meter,
//! right-panel toggle. Native window controls stay native per OS.

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetLevel {
    Ok,
    Warning,
    Over,
}

/// Full title-bar state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TitleBar {
    pub project_name: Option<String>,
    pub run: RunPill,
    pub pending_approvals: u32,
    pub budget: BudgetMeter,
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
}
