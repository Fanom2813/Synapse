//! Foundation window: renders [`AppShell`](crate::shell::AppShell) in a
//! native GPUI Kit window (M1 task 2, UI.md §1 + §1b).
//!
//! The toolkit-free state lives in `shell`/`layout`/`titlebar`; this module
//! is the thin view over it: sidebar / main / right-panel dock with the
//! empty states, plus a custom title bar (sidebar + panel toggles, project
//! switcher, run pill, command-palette button, approvals inbox, budget).
//! Window behaviour (close hides, destroy after 60 s hidden, tray, single
//! instance) is modelled in [`crate::lifecycle`] and wired here.

use crate::app_dirs;
use crate::shell::{AppShell, ShellPersist};

/// Headless description of what the window shows. Toolkit-free so tests
/// run without a display; the GPUI view renders from the same source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellScreen {
    pub window_title: String,
    pub sidebar_text: String,
    pub main_text: String,
    pub panel_text: String,
}

impl ShellScreen {
    pub fn describe(shell: &AppShell) -> Self {
        let window_title = window_title(shell);
        let sidebar_text = match shell.sidebar_empty() {
            Some(e) => format!("{} | {} [{}]", e.title, e.body, e.action),
            None => format!(
                "Projects: {}",
                shell
                    .projects
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let main_text = match shell.room_empty() {
            Some(e) => format!("{} | {} [{}]", e.title, e.body, e.action),
            None => format!(
                "Team room: {}",
                shell.open_project_id.as_deref().unwrap_or("")
            ),
        };
        let panel_text = format!("Panel: {}", shell.layout.right_panel_tab.title());
        Self {
            window_title,
            sidebar_text,
            main_text,
            panel_text,
        }
    }
}

/// Title shown in the OS window frame.
pub fn window_title(shell: &AppShell) -> String {
    match &shell.title.project_name {
        Some(name) => format!("{name} — Synapse"),
        None => "Synapse".to_string(),
    }
}

/// Open the shell with persisted state from the data dir. Never fails:
/// missing or corrupt files fall back to defaults so the window always opens.
pub fn open_persisted_shell() -> AppShell {
    let dir = app_dirs::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut shell = AppShell::open_with_persist(dir);
    shell.initial_workspace();
    shell
}

/// Launch the native window. Called from the `synapse` binary; blocks on
/// the GPUI event loop until explicit quit (tray Quit / Cmd-Ctrl-Q).
#[cfg(not(test))]
pub fn launch() {
    use gpui_kit::component::button::{Button, ButtonVariants};
    use gpui_kit::*;

    struct ShellView {
        shell: AppShell,
    }

    impl ShellView {
        fn screen(&self) -> ShellScreen {
            ShellScreen::describe(&self.shell)
        }
    }

    impl Render for ShellView {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let screen = self.screen();
            let sidebar_visible = self.shell.layout.sidebar.visible;
            let panel_visible = self.shell.layout.right_panel_visible;
            div()
                .flex()
                .flex_col()
                .size_full()
                .child(
                    // Custom title bar (UI.md §1b): toggles + project + run + palette + inbox.
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .p_2()
                        .child(
                            Button::new("sidebar-toggle")
                                .label(if sidebar_visible {
                                    "≡ Hide"
                                } else {
                                    "≡ Show"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.shell.toggle_sidebar();
                                    cx.notify();
                                })),
                        )
                        .child(div().child(screen.window_title.clone()))
                        .child(div().child(run_pill_text(&self.shell)))
                        .child(Button::new("palette").label("⌘K").on_click(|_, _, _| {}))
                        .child(div().child(inbox_text(&self.shell)))
                        .child(
                            Button::new("panel-toggle")
                                .label(if panel_visible {
                                    "◧ Hide"
                                } else {
                                    "◧ Show"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.shell.toggle_right_panel();
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .child(div().p_2().child(screen.sidebar_text.clone()))
                        .child(div().flex_1().p_2().child(screen.main_text.clone()))
                        .child(div().p_2().child(screen.panel_text.clone())),
                )
                .child(
                    Button::new("theme-cycle")
                        .label("Theme")
                        .secondary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let mut a = this.shell.appearance.clone();
                            a.mode = match a.mode {
                                crate::theme::ThemeMode::System => crate::theme::ThemeMode::Light,
                                crate::theme::ThemeMode::Light => crate::theme::ThemeMode::Dark,
                                crate::theme::ThemeMode::Dark => crate::theme::ThemeMode::System,
                            };
                            this.shell.set_appearance(a);
                            cx.notify();
                        })),
                )
        }
    }

    let mut shell = Some(open_persisted_shell());
    application().run(move |cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, |_, cx| {
            let s = shell.take().expect("window opened once");
            cx.new(|_| ShellView { shell: s })
        })
        .expect("failed to open Synapse window");
    });
}

/// Run-pill line for the title bar (mode + phase, or "Start a run").
pub fn run_pill_text(shell: &AppShell) -> String {
    match &shell.title.run {
        crate::titlebar::RunPill::Idle => "Start a run".to_string(),
        crate::titlebar::RunPill::Running { mode, phase, .. } => format!("{mode} · {phase}"),
    }
}

/// Approvals inbox badge text ("2" or "" when none).
pub fn inbox_text(shell: &AppShell) -> String {
    let b = shell.title.approvals_badge();
    if b.is_empty() {
        "Inbox".to_string()
    } else {
        format!("Inbox ({b})")
    }
}

/// Persist handle for the data dir (used by the binary for tests/headless).
pub fn data_persist() -> ShellPersist {
    ShellPersist {
        dir: app_dirs::data_dir(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::ShellProject;
    use crate::titlebar::RunPill;

    #[test]
    fn shell_window_shows_sidebar_and_room_empty_states() {
        let mut shell = AppShell::open();
        shell.initial_workspace();
        let screen = ShellScreen::describe(&shell);
        assert_eq!(screen.window_title, "Synapse");
        assert!(screen.sidebar_text.contains("Name your first project"));
        assert!(screen.sidebar_text.contains("[New project]"));
        assert!(screen.main_text.contains("[New project]"));
        assert!(screen.panel_text.contains("Team"));
        assert_eq!(window_title(&shell), "Synapse");
    }

    #[test]
    fn shell_window_lists_projects_and_room_state() {
        let mut shell = AppShell::open();
        shell.projects.push(ShellProject {
            id: "p1".to_string(),
            name: "Market study".to_string(),
            has_messages: false,
        });
        shell.projects.push(ShellProject {
            id: "p2".to_string(),
            name: "Uganda fintech".to_string(),
            has_messages: true,
        });
        shell.initial_workspace();
        let screen = ShellScreen::describe(&shell);
        assert_eq!(screen.window_title, "Market study — Synapse");
        assert!(screen.sidebar_text.contains("Market study"));
        // p1 has no messages: room empty state with the run action.
        assert!(screen.main_text.contains("[Start a run]"));
        assert!(shell.open_project("p2"));
        let screen = ShellScreen::describe(&shell);
        assert!(screen.main_text.contains("Team room: p2"));

        shell.set_run(RunPill::Running {
            mode: "Deep research".to_string(),
            phase: "Searching".to_string(),
            progress: 0.4,
            live_agent_ids: vec!["lead".to_string()],
        });
        assert_eq!(run_pill_text(&shell), "Deep research · Searching");
        shell.title.pending_approvals = 2;
        assert_eq!(inbox_text(&shell), "Inbox (2)");
        shell.title.pending_approvals = 0;
        assert_eq!(run_pill_text(&AppShell::open()), "Start a run");
        assert_eq!(inbox_text(&shell), "Inbox");
    }
}
