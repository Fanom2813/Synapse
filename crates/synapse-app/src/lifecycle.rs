//! Tray and window lifecycle (ARCHITECTURE.md §5, MILESTONES.md M1.6).
//!
//! Toolkit-free state machine so it is testable headless. The GPUI shell
//! wires it up: `QuitMode::Explicit`, close hides the main window, a
//! background task destroys the hidden window after [`DESTROY_HIDDEN_AFTER_SECS`],
//! the tray offers Open/Quit, and a second launch focuses the first instance
//! over the `interprocess` local socket.

use std::path::PathBuf;

/// Seconds a hidden main window is kept before it is dropped entirely
/// (frees GPU surfaces and text caches; see PERFORMANCE.md).
pub const DESTROY_HIDDEN_AFTER_SECS: u64 = 60;

/// Quit behaviour: the app only quits from an explicit action
/// (tray Quit / Cmd-Ctrl-Q), never from closing the last window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuitModeExplicit;

impl QuitModeExplicit {
    /// Closing the last window never quits.
    pub fn should_quit_on_last_window_closed() -> bool {
        false
    }
}

/// Visibility of the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowVisibility {
    Visible,
    Hidden,
    Destroyed,
}

/// What the (hidden or destroyed) window needs when asked to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenEffect {
    /// Window was already visible; bring it to front.
    Focus,
    /// Hidden window shown again (cheap restore).
    Restored,
    /// Window was destroyed; caller must rebuild it from the store.
    Rebuilt,
}

/// Main-window lifecycle: close hides, reopen restores/rebuilds, and a
/// hidden window is destroyed after [`DESTROY_HIDDEN_AFTER_SECS`].
#[derive(Debug, Clone)]
pub struct WindowLifecycle {
    visibility: WindowVisibility,
    hidden_elapsed_secs: u64,
}

impl WindowLifecycle {
    pub fn new_visible() -> Self {
        Self {
            visibility: WindowVisibility::Visible,
            hidden_elapsed_secs: 0,
        }
    }

    pub fn visibility(&self) -> WindowVisibility {
        self.visibility
    }

    pub fn is_visible(&self) -> bool {
        self.visibility == WindowVisibility::Visible
    }

    pub fn hidden_elapsed_secs(&self) -> u64 {
        self.hidden_elapsed_secs
    }

    /// Close button: hide the window instead of quitting. Returns false
    /// only when there is no window to hide (already destroyed).
    pub fn close_window(&mut self) -> bool {
        match self.visibility {
            WindowVisibility::Visible | WindowVisibility::Hidden => {
                self.visibility = WindowVisibility::Hidden;
                true
            }
            WindowVisibility::Destroyed => false,
        }
    }

    /// Reopen the window. Reports whether the caller must rebuild it.
    pub fn open_window(&mut self) -> OpenEffect {
        match self.visibility {
            WindowVisibility::Visible => OpenEffect::Focus,
            WindowVisibility::Hidden => {
                self.visibility = WindowVisibility::Visible;
                self.hidden_elapsed_secs = 0;
                OpenEffect::Restored
            }
            WindowVisibility::Destroyed => {
                self.visibility = WindowVisibility::Visible;
                self.hidden_elapsed_secs = 0;
                OpenEffect::Rebuilt
            }
        }
    }

    /// Advance the hidden clock by `elapsed_secs`. Destroys the window
    /// once it has been hidden for [`DESTROY_HIDDEN_AFTER_SECS`].
    /// Returns true when the window was destroyed by this call.
    pub fn advance(&mut self, elapsed_secs: u64) -> bool {
        if self.visibility != WindowVisibility::Hidden {
            return false;
        }
        self.hidden_elapsed_secs = self.hidden_elapsed_secs.saturating_add(elapsed_secs);
        if self.hidden_elapsed_secs >= DESTROY_HIDDEN_AFTER_SECS {
            self.visibility = WindowVisibility::Destroyed;
            self.hidden_elapsed_secs = 0;
            return true;
        }
        false
    }

    pub fn destroy_threshold_secs() -> u64 {
        DESTROY_HIDDEN_AFTER_SECS
    }
}

/// Tray commands (gpui-tray menu: Open, Quit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Open,
    Quit,
}

/// Effect of a tray command on the lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEffect {
    Open(OpenEffect),
    Quit,
}

impl TrayEffect {
    pub fn quits(&self) -> bool {
        matches!(self, TrayEffect::Quit)
    }
}

/// Apply a tray menu command to the window lifecycle.
pub fn apply_tray_command(lifecycle: &mut WindowLifecycle, cmd: TrayCommand) -> TrayEffect {
    match cmd {
        TrayCommand::Open => TrayEffect::Open(lifecycle.open_window()),
        TrayCommand::Quit => TrayEffect::Quit,
    }
}

/// Items in the tray menu (UI.md §7 subset for M1: Open, Quit).
pub fn tray_menu_items() -> Vec<&'static str> {
    vec!["Open", "Quit"]
}

/// Parse a tray menu label back into a command.
pub fn parse_tray_menu_item(label: &str) -> Option<TrayCommand> {
    match label {
        "Open" => Some(TrayCommand::Open),
        "Quit" => Some(TrayCommand::Quit),
        _ => None,
    }
}

/// Message a second instance sends to the running instance.
pub const FOCUS_MESSAGE: &str = "focus";

/// Local-socket file for single-instance IPC inside `dir`.
pub fn instance_socket_path(dir: &std::path::Path) -> PathBuf {
    dir.join("synapse.sock")
}

/// Outcome of a launch attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOutcome {
    /// No instance is running; this process becomes the primary.
    Primary,
    /// Another instance holds the socket; this process must exit
    /// after asking it to focus.
    FocusExisting,
}

/// Decide the launch outcome from whether the single-instance socket
/// is already held. (`socket_held == true` means another instance runs.)
pub fn decide_launch(socket_held: bool) -> LaunchOutcome {
    if socket_held {
        LaunchOutcome::FocusExisting
    } else {
        LaunchOutcome::Primary
    }
}

/// Effect on the running instance when a second launch arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondLaunchEffect {
    Focus,
}

/// Handle an incoming IPC message on the primary. Unknown payloads are
/// ignored but still focus the window (a second launch always means focus).
pub fn handle_second_launch(
    lifecycle: &mut WindowLifecycle,
    message: &str,
) -> (SecondLaunchEffect, OpenEffect) {
    debug_assert!(!message.is_empty() || true);
    let _ = message;
    (SecondLaunchEffect::Focus, lifecycle.open_window())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_close_hides_instead_of_quitting() {
        assert!(!QuitModeExplicit::should_quit_on_last_window_closed());
        let mut w = WindowLifecycle::new_visible();
        assert!(w.close_window());
        assert_eq!(w.visibility(), WindowVisibility::Hidden);
        assert!(!w.is_visible());
        // Closing again keeps it hidden (still handled, no quit).
        assert!(w.close_window());
        assert_eq!(w.visibility(), WindowVisibility::Hidden);
    }

    #[test]
    fn lifecycle_reopen_restores_hidden_window() {
        let mut w = WindowLifecycle::new_visible();
        w.close_window();
        assert_eq!(w.open_window(), OpenEffect::Restored);
        assert_eq!(w.visibility(), WindowVisibility::Visible);
        // Opening a visible window just focuses it.
        assert_eq!(w.open_window(), OpenEffect::Focus);
    }

    #[test]
    fn lifecycle_hidden_window_eventually_destroyed_and_rebuilt() {
        assert_eq!(WindowLifecycle::destroy_threshold_secs(), 60);
        let mut w = WindowLifecycle::new_visible();
        w.close_window();
        // Just under the threshold: still hidden.
        assert!(!w.advance(59));
        assert_eq!(w.visibility(), WindowVisibility::Hidden);
        // Past the threshold: destroyed.
        assert!(w.advance(1));
        assert_eq!(w.visibility(), WindowVisibility::Destroyed);
        // Reopening rebuilds from the store.
        assert_eq!(w.open_window(), OpenEffect::Rebuilt);
        assert_eq!(w.visibility(), WindowVisibility::Visible);
        // Advancing while visible never destroys.
        assert!(!w.advance(3600));
        assert_eq!(w.visibility(), WindowVisibility::Visible);
    }

    #[test]
    fn lifecycle_tray_can_open_or_quit() {
        assert_eq!(tray_menu_items(), vec!["Open", "Quit"]);
        assert_eq!(parse_tray_menu_item("Open"), Some(TrayCommand::Open));
        assert_eq!(parse_tray_menu_item("Quit"), Some(TrayCommand::Quit));
        assert_eq!(parse_tray_menu_item("Nope"), None);

        let mut w = WindowLifecycle::new_visible();
        w.close_window();
        assert_eq!(
            apply_tray_command(&mut w, TrayCommand::Open),
            TrayEffect::Open(OpenEffect::Restored)
        );
        assert!(w.is_visible());
        assert_eq!(
            apply_tray_command(&mut w, TrayCommand::Quit),
            TrayEffect::Quit
        );
        assert!(TrayEffect::Quit.quits());
    }

    #[test]
    fn lifecycle_second_launch_focuses_running_instance() {
        // First launch becomes primary; second launch defers to it.
        assert_eq!(decide_launch(false), LaunchOutcome::Primary);
        assert_eq!(decide_launch(true), LaunchOutcome::FocusExisting);

        // Running instance focuses/restores its window on second launch.
        let mut w = WindowLifecycle::new_visible();
        w.close_window();
        let (effect, open) = handle_second_launch(&mut w, FOCUS_MESSAGE);
        assert_eq!(effect, SecondLaunchEffect::Focus);
        assert_eq!(open, OpenEffect::Restored);
        assert!(w.is_visible());

        // Even when destroyed, a second launch rebuilds the window.
        w.close_window();
        w.advance(DESTROY_HIDDEN_AFTER_SECS);
        assert_eq!(w.visibility(), WindowVisibility::Destroyed);
        let (_, open) = handle_second_launch(&mut w, FOCUS_MESSAGE);
        assert_eq!(open, OpenEffect::Rebuilt);
        assert!(w.is_visible());

        // Socket path is stable inside the data dir.
        let dir = std::path::Path::new("/tmp/synapse-test");
        assert_eq!(
            instance_socket_path(dir),
            std::path::PathBuf::from("/tmp/synapse-test/synapse.sock")
        );
    }
}
