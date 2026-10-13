//! Main-window dock layout: sidebar / main / right panel (UI.md §1).
//! Panel sizes and open tabs persist per project. Toolkit-free model; the
//! GPUI Kit dock renders from this state.

use std::collections::HashMap;

/// Tabs of the resizable right panel (UI.md §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PanelTab {
    /// Members + add agent.
    #[default]
    Team,
    Sources,
    Artifact,
    Activity,
    Map,
    Scheduled,
}

impl PanelTab {
    pub const ALL: [Self; 6] = [
        Self::Team,
        Self::Sources,
        Self::Artifact,
        Self::Activity,
        Self::Map,
        Self::Scheduled,
    ];

    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Team => "Team",
            Self::Sources => "Sources",
            Self::Artifact => "Artifact",
            Self::Activity => "Activity",
            Self::Map => "Map",
            Self::Scheduled => "Scheduled",
        }
    }
}

/// Sidebar sections (UI.md §1): pinned + all projects, agent roster, settings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SidebarState {
    pub visible: bool,
    pub pinned_project_ids: Vec<String>,
    pub selected_project_id: Option<String>,
}

impl SidebarState {
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

/// Dock layout for one project.
#[derive(Debug, Clone, PartialEq)]
pub struct DockLayout {
    pub sidebar: SidebarState,
    pub right_panel_visible: bool,
    pub right_panel_tab: PanelTab,
    /// Sidebar width px, clamped by the view to [180, 360].
    pub sidebar_width: f32,
    /// Right panel width px, clamped by the view to [280, 560].
    pub right_panel_width: f32,
}

impl Default for DockLayout {
    fn default() -> Self {
        Self {
            sidebar: SidebarState {
                visible: true,
                pinned_project_ids: Vec::new(),
                selected_project_id: None,
            },
            right_panel_visible: true,
            right_panel_tab: PanelTab::Team,
            sidebar_width: 248.0,
            right_panel_width: 344.0,
        }
    }
}

impl DockLayout {
    pub fn toggle_sidebar(&mut self) {
        self.sidebar.toggle();
    }

    pub fn toggle_right_panel(&mut self) {
        self.right_panel_visible = !self.right_panel_visible;
    }

    /// Cmd/Ctrl-1..5 switches panel tabs (UI.md §1).
    pub fn select_tab_by_index(&mut self, i: usize) {
        if let Some(tab) = PanelTab::from_index(i) {
            self.right_panel_tab = tab;
            self.right_panel_visible = true;
        }
    }

    pub fn set_sidebar_width(&mut self, w: f32) {
        self.sidebar_width = w.clamp(180.0, 360.0);
    }

    pub fn set_right_panel_width(&mut self, w: f32) {
        self.right_panel_width = w.clamp(280.0, 560.0);
    }

    pub fn to_json(&self) -> String {
        serde_json::json!({
            "sidebar_visible": self.sidebar.visible,
            "pinned": self.sidebar.pinned_project_ids,
            "selected": self.sidebar.selected_project_id,
            "panel_visible": self.right_panel_visible,
            "panel_tab": self.right_panel_tab.index(),
            "sidebar_width": self.sidebar_width,
            "panel_width": self.right_panel_width,
        })
        .to_string()
    }

    pub fn from_json(s: &str) -> Self {
        let mut out = Self::default();
        let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else {
            return out;
        };
        if let Some(x) = v.get("sidebar_visible").and_then(|x| x.as_bool()) {
            out.sidebar.visible = x;
        }
        if let Some(arr) = v.get("pinned").and_then(|x| x.as_array()) {
            out.sidebar.pinned_project_ids = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .take(64)
                .collect();
        }
        if let Some(sel) = v.get("selected").and_then(|x| x.as_str()) {
            out.sidebar.selected_project_id = Some(sel.to_string());
        }
        if let Some(x) = v.get("panel_visible").and_then(|x| x.as_bool()) {
            out.right_panel_visible = x;
        }
        if let Some(i) = v.get("panel_tab").and_then(|x| x.as_u64()) {
            if let Some(tab) = PanelTab::from_index(i as usize) {
                out.right_panel_tab = tab;
            }
        }
        if let Some(w) = v.get("sidebar_width").and_then(|x| x.as_f64()) {
            out.set_sidebar_width(w as f32);
        }
        if let Some(w) = v.get("panel_width").and_then(|x| x.as_f64()) {
            out.set_right_panel_width(w as f32);
        }
        out
    }
}

/// Per-project persisted layouts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutStore {
    per_project: HashMap<String, DockLayout>,
}

impl LayoutStore {
    pub fn get(&self, project_id: &str) -> DockLayout {
        self.per_project
            .get(project_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set(&mut self, project_id: &str, layout: DockLayout) {
        self.per_project.insert(project_id.to_string(), layout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_layout_toggles_and_tabs() {
        let mut l = DockLayout::default();
        assert!(l.sidebar.visible && l.right_panel_visible);
        l.toggle_sidebar();
        assert!(!l.sidebar.visible);
        l.toggle_right_panel();
        assert!(!l.right_panel_visible);
        l.select_tab_by_index(2);
        assert_eq!(l.right_panel_tab, PanelTab::Artifact);
        assert!(l.right_panel_visible);
        l.select_tab_by_index(99); // ignored
        assert_eq!(l.right_panel_tab, PanelTab::Artifact);
        l.set_sidebar_width(10.0);
        assert_eq!(l.sidebar_width, 180.0);
        l.set_right_panel_width(9999.0);
        assert_eq!(l.right_panel_width, 560.0);
        let back = DockLayout::from_json(&l.to_json());
        assert_eq!(l, back);
    }
}
