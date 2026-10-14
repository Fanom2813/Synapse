//! App shell: the main window state (UI.md §1 + §1b).
//!
//! Owns the sidebar / main / right-panel [`DockLayout`](crate::layout::DockLayout),
//! the [`TitleBar`](crate::titlebar::TitleBar), the persisted
//! [`Appearance`](crate::theme::Appearance), and the project that is open.
//! The GPUI Kit window renders from this; all behaviour here is toolkit-free
//! and covered by the `shell` tests.

use crate::empty::{self, EmptyState};
use crate::layout::{DockLayout, LayoutStore, PanelTab};
use crate::theme::Appearance;
use crate::titlebar::{RunPill, TitleBar};
use std::io;

/// Where shell settings persist. The store crate owns the SQL schema; the
/// shell keeps two small JSON blobs (appearance + per-project layouts) that
/// the app writes through the settings table. For tests and headless use the
/// blobs can live in a plain directory.
#[derive(Debug, Clone)]
pub struct ShellPersist {
    pub dir: std::path::PathBuf,
}

impl ShellPersist {
    pub fn appearance_path(&self) -> std::path::PathBuf {
        self.dir.join("appearance.json")
    }

    pub fn layout_path(&self) -> std::path::PathBuf {
        self.dir.join("layout.json")
    }

    pub fn load_appearance(&self) -> Appearance {
        match std::fs::read_to_string(self.appearance_path()) {
            Ok(s) => Appearance::from_json(&s),
            Err(_) => Appearance::default(),
        }
    }

    pub fn save_appearance(&self, a: &Appearance) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.appearance_path(), a.to_json())
    }

    pub fn load_layout(&self) -> DockLayout {
        match std::fs::read_to_string(self.layout_path()) {
            Ok(s) => DockLayout::from_json(&s),
            Err(_) => DockLayout::default(),
        }
    }

    pub fn save_layout(&self, l: &DockLayout) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.layout_path(), l.to_json())
    }
}

/// Reachable top-level screens in the main window (navigable via the
/// sidebar / command palette). The Settings → Providers screen is a real
/// destination, not a placeholder: the window renders it when selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppScreen {
    #[default]
    Room,
    ProvidersSettings,
}

impl AppScreen {
    pub fn title(self) -> &'static str {
        match self {
            Self::Room => "Team room",
            Self::ProvidersSettings => "Settings · Providers",
        }
    }
}

/// A project as far as the shell cares: id + display name + goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellProject {
    pub id: String,
    pub name: String,
    pub has_messages: bool,
}

/// The main window shell.
#[derive(Debug)]
pub struct AppShell {
    pub appearance: Appearance,
    pub layout: DockLayout,
    layouts: LayoutStore,
    pub title: TitleBar,
    pub projects: Vec<ShellProject>,
    pub open_project_id: Option<String>,
    pub screen: AppScreen,
    persist: Option<ShellPersist>,
}

impl AppShell {
    /// Open a usable main window with no prior state: default layout,
    /// system theme, no project. Never fails.
    pub fn open() -> Self {
        Self {
            appearance: Appearance::default(),
            layout: DockLayout::default(),
            layouts: LayoutStore::default(),
            title: TitleBar::default(),
            projects: Vec::new(),
            open_project_id: None,
            screen: AppScreen::Room,
            persist: None,
        }
    }

    /// Open with persisted settings in `dir` (data dir). Missing or corrupt
    /// files fall back to defaults so the window always opens.
    pub fn open_with_persist(dir: std::path::PathBuf) -> Self {
        let persist = ShellPersist { dir };
        let mut shell = Self::open();
        shell.appearance = persist.load_appearance();
        shell.layout = persist.load_layout();
        shell.persist = Some(persist);
        shell
    }

    /// Reach the initial workspace: with no projects, the sidebar empty
    /// state; with projects, the first (or selected) project opens.
    pub fn initial_workspace(&mut self) {
        if self.projects.is_empty() {
            self.open_project_id = None;
            self.title.project_name = None;
            return;
        }
        let id = self
            .layout
            .sidebar
            .selected_project_id
            .clone()
            .filter(|id| self.projects.iter().any(|p| &p.id == id))
            .or_else(|| self.projects.first().map(|p| p.id.clone()));
        if let Some(id) = id {
            self.open_project(&id);
        }
    }

    /// Open a project: restore its persisted layout, update the title bar.
    /// Returns false when the id is unknown.
    pub fn open_project(&mut self, id: &str) -> bool {
        let Some(p) = self.projects.iter().find(|p| p.id == id) else {
            return false;
        };
        if let Some(open) = self.open_project_id.clone() {
            self.layouts.set(&open, self.layout.clone());
        }
        self.layout = self.layouts.get(id);
        // Keep the global sidebar selection in sync.
        self.layout.sidebar.selected_project_id = Some(id.to_string());
        self.open_project_id = Some(id.to_string());
        self.title.project_name = Some(p.name.clone());
        self.persist_current();
        true
    }

    pub fn toggle_sidebar(&mut self) {
        self.layout.toggle_sidebar();
        self.persist_current();
    }

    pub fn toggle_right_panel(&mut self) {
        self.layout.toggle_right_panel();
        self.persist_current();
    }

    pub fn select_panel_tab(&mut self, tab: PanelTab) {
        self.layout.right_panel_tab = tab;
        self.layout.right_panel_visible = true;
        self.persist_current();
    }

    pub fn set_run(&mut self, run: RunPill) {
        self.title.run = run;
        self.title.run_paused = false;
    }

    /// Run popover controls (UI.md §1b-3): pause / resume / stop.
    pub fn pause_run(&mut self) -> bool {
        use crate::titlebar::RunAction;
        self.title.apply_run_action(RunAction::Pause)
    }

    pub fn resume_run(&mut self) -> bool {
        use crate::titlebar::RunAction;
        self.title.apply_run_action(RunAction::Resume)
    }

    pub fn stop_run(&mut self) -> bool {
        use crate::titlebar::RunAction;
        self.title.apply_run_action(RunAction::Stop)
    }

    /// Navigate to the Settings → Providers screen (sidebar / palette).
    pub fn open_providers_settings(&mut self) {
        self.screen = AppScreen::ProvidersSettings;
    }

    /// Navigate back to the team room.
    pub fn open_room(&mut self) {
        self.screen = AppScreen::Room;
    }

    pub fn is_providers_settings_open(&self) -> bool {
        self.screen == AppScreen::ProvidersSettings
    }

    /// Empty state for the sidebar (no projects at all).
    pub fn sidebar_empty(&self) -> Option<EmptyState> {
        if self.projects.is_empty() {
            Some(empty::no_projects())
        } else {
            None
        }
    }

    /// Empty state for the main team-room area.
    pub fn room_empty(&self) -> Option<EmptyState> {
        match self
            .open_project_id
            .as_ref()
            .and_then(|id| self.projects.iter().find(|p| &p.id == id))
        {
            None => Some(empty::no_projects()),
            Some(p) if !p.has_messages => Some(empty::empty_room()),
            Some(_) => None,
        }
    }

    fn persist_current(&self) {
        if let Some(p) = &self.persist {
            let _ = p.save_layout(&self.layout);
            let _ = p.save_appearance(&self.appearance);
        }
    }

    /// Persist a theme change (Settings → Appearance, UI.md §8).
    pub fn set_appearance(&mut self, a: Appearance) {
        self.appearance = a;
        self.persist_current();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeMode;

    fn project(id: &str, name: &str, has_messages: bool) -> ShellProject {
        ShellProject {
            id: id.to_string(),
            name: name.to_string(),
            has_messages,
        }
    }

    #[test]
    fn shell_opens_to_initial_workspace_without_errors() {
        // No projects: sidebar + room empty states, window usable.
        let mut shell = AppShell::open();
        shell.initial_workspace();
        assert_eq!(shell.open_project_id, None);
        assert!(shell.sidebar_empty().is_some());
        assert!(shell.room_empty().is_some());
        assert!(shell.layout.sidebar.visible);
        assert!(shell.layout.right_panel_visible);

        // With projects: first one opens, title bar follows.
        shell.projects.push(project("p1", "Market study", false));
        shell.projects.push(project("p2", "Uganda fintech", true));
        shell.initial_workspace();
        assert_eq!(shell.open_project_id.as_deref(), Some("p1"));
        assert_eq!(shell.title.project_name.as_deref(), Some("Market study"));
        assert!(shell.sidebar_empty().is_none());
        // p1 has no messages yet -> room empty state with primary action.
        let room = shell.room_empty().expect("empty room state");
        assert_eq!(room.action, "Start a run");

        // p2 has messages -> no empty state.
        assert!(shell.open_project("p2"));
        assert_eq!(shell.title.project_name.as_deref(), Some("Uganda fintech"));
        assert!(shell.room_empty().is_none());

        // Unknown id never breaks the open window.
        assert!(!shell.open_project("missing"));
        assert_eq!(shell.open_project_id.as_deref(), Some("p2"));
    }

    #[test]
    fn shell_titlebar_toggles_and_theme_persist() {
        let dir = std::env::temp_dir().join(format!("synapse-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut shell = AppShell::open_with_persist(dir.clone());
        shell.toggle_sidebar();
        assert!(!shell.layout.sidebar.visible);
        shell.toggle_right_panel();
        assert!(!shell.layout.right_panel_visible);
        shell.select_panel_tab(PanelTab::Sources);
        assert!(shell.layout.right_panel_visible);

        let mut a = shell.appearance.clone();
        a.mode = ThemeMode::Dark;
        shell.set_appearance(a);

        // Reopen: theme + layout survived.
        let reopened = AppShell::open_with_persist(dir.clone());
        assert_eq!(reopened.appearance.mode, ThemeMode::Dark);
        assert!(!reopened.layout.sidebar.visible);
        assert_eq!(reopened.layout.right_panel_tab, PanelTab::Sources);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_run_pill_progress_line() {
        let mut shell = AppShell::open();
        assert!(!shell.title.run.is_running());
        shell.set_run(RunPill::Running {
            mode: "Deep research".to_string(),
            phase: "Searching".to_string(),
            progress: 0.4,
            live_agent_ids: vec!["lead".to_string()],
        });
        assert!(shell.title.run.is_running());
        assert!((shell.title.run.progress() - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn shell_navigates_to_providers_settings_and_back() {
        let mut shell = AppShell::open();
        assert_eq!(shell.screen, AppScreen::Room);
        assert!(!shell.is_providers_settings_open());
        shell.open_providers_settings();
        assert!(shell.is_providers_settings_open());
        assert_eq!(shell.screen.title(), "Settings · Providers");
        shell.open_room();
        assert!(!shell.is_providers_settings_open());
    }
}
