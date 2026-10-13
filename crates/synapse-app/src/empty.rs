//! Empty states (UI.md §5): duotone Phosphor illustration + one clear
//! primary action. One per area that can be empty.

use crate::icons::{self, Icon};

/// An empty state: illustration, headline, body, primary action label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyState {
    pub icon: Icon,
    pub title: &'static str,
    pub body: &'static str,
    pub action: &'static str,
}

/// No projects yet / none selected.
pub fn no_projects() -> EmptyState {
    EmptyState {
        icon: icons::EMPTY_PROJECTS,
        title: "Name your first project",
        body: "Give the team a goal to work on. You can add providers and members next.",
        action: "New project",
    }
}

/// Project exists but the team room has no messages yet (UI.md §2.2).
pub fn empty_room() -> EmptyState {
    EmptyState {
        icon: icons::EMPTY_ROOM,
        title: "Describe the goal to start",
        body: "Pick a run mode — Brainstorm, Deep research, Deep analysis — and the lead introduces the team.",
        action: "Start a run",
    }
}

/// No sources fetched yet for this project.
pub fn no_sources() -> EmptyState {
    EmptyState {
        icon: icons::EMPTY_SOURCES,
        title: "No sources yet",
        body: "Sources the team reads will appear here with credibility notes.",
        action: "Start research",
    }
}

/// No artifact produced yet.
pub fn no_artifact() -> EmptyState {
    EmptyState {
        icon: icons::EMPTY_ARTIFACT,
        title: "No report yet",
        body: "The team's synthesis will appear here when the run finishes.",
        action: "View activity",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_empty_states_have_actions() {
        for s in [no_projects(), empty_room(), no_sources(), no_artifact()] {
            assert!(!s.title.is_empty());
            assert!(!s.body.is_empty());
            assert!(!s.action.is_empty());
        }
    }
}
