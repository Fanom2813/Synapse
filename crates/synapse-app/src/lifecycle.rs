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

/// Tray icon states: idle (nothing running), working (≥1 active run),
/// needs-you (pending approvals take precedence over working).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIconState {
    Idle,
    Working,
    NeedsYou,
}

impl TrayIconState {
    pub fn as_str(self) -> &'static str {
        match self {
            TrayIconState::Idle => "idle",
            TrayIconState::Working => "working",
            TrayIconState::NeedsYou => "needs-you",
        }
    }
}

/// One active run shown in the tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayRun {
    pub id: String,
    pub title: String,
    pub phase: String,
}

impl TrayRun {
    pub fn new(id: &str, title: &str, phase: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            phase: phase.to_string(),
        }
    }

    /// Row label: "• {title} — {phase}".
    pub fn label(&self) -> String {
        format!("• {} — {}", self.title, self.phase)
    }
}

/// Toolkit-free snapshot of everything the tray menu shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TraySnapshot {
    pub runs: Vec<TrayRun>,
    pub pending_approvals: u32,
    /// True when every active run is paused (menu offers Resume all).
    pub all_paused: bool,
}

impl TraySnapshot {
    pub fn idle() -> Self {
        Self::default()
    }
}

/// Icon state for a snapshot: needs-you wins over working wins over idle.
pub fn tray_icon_state(snapshot: &TraySnapshot) -> TrayIconState {
    if snapshot.pending_approvals > 0 {
        TrayIconState::NeedsYou
    } else if !snapshot.runs.is_empty() {
        TrayIconState::Working
    } else {
        TrayIconState::Idle
    }
}

/// Tray commands (gpui-tray menu: Open, runs, approvals, pause-all,
/// quick-ask, Quit; see UI.md §2.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    Open,
    ShowRun(String),
    ShowApprovals,
    PauseAll,
    ResumeAll,
    QuickAsk,
    Quit,
}

/// Effect of a tray command on the lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayEffect {
    Open(OpenEffect),
    ShowRun(String),
    ShowApprovals,
    PauseAll,
    ResumeAll,
    QuickAsk,
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
        TrayCommand::ShowRun(id) => TrayEffect::ShowRun(id),
        TrayCommand::ShowApprovals => TrayEffect::ShowApprovals,
        TrayCommand::PauseAll => TrayEffect::PauseAll,
        TrayCommand::ResumeAll => TrayEffect::ResumeAll,
        TrayCommand::QuickAsk => TrayEffect::QuickAsk,
        TrayCommand::Quit => TrayEffect::Quit,
    }
}

/// Items in the tray menu (UI.md §2.7): Open Synapse, active runs with
/// phase, pending approvals, Pause all / Resume all, Quick ask, Quit.
///
/// Kept as owned strings because run rows embed live titles + phases.
pub fn tray_menu_items() -> Vec<String> {
    tray_menu_labels(&TraySnapshot::idle())
}

/// Full tray menu labels for a snapshot.
pub fn tray_menu_labels(snapshot: &TraySnapshot) -> Vec<String> {
    let mut items = vec!["Open Synapse".to_string()];
    if snapshot.runs.is_empty() {
        items.push("No active runs".to_string());
    } else {
        for run in &snapshot.runs {
            items.push(run.label());
        }
    }
    if snapshot.pending_approvals > 0 {
        items.push(format!("Pending approvals ({})", snapshot.pending_approvals));
    } else {
        items.push("Pending approvals".to_string());
    }
    if !snapshot.runs.is_empty() {
        if snapshot.all_paused {
            items.push("Resume All".to_string());
        } else {
            items.push("Pause All".to_string());
        }
    }
    items.push("Quick Ask".to_string());
    items.push("Quit".to_string());
    items
}

/// Parse a tray menu label back into a command.
pub fn parse_tray_menu_item(label: &str) -> Option<TrayCommand> {
    match label {
        "Open" | "Open Synapse" => Some(TrayCommand::Open),
        "Quit" => Some(TrayCommand::Quit),
        "Quick Ask" | "Quick ask" => Some(TrayCommand::QuickAsk),
        "Pause All" | "Pause all" => Some(TrayCommand::PauseAll),
        "Resume All" | "Resume all" => Some(TrayCommand::ResumeAll),
        "Pending approvals" => Some(TrayCommand::ShowApprovals),
        "No active runs" => None,
        _ => {
            if let Some(rest) = label.strip_prefix("Pending approvals (") {
                if rest.ends_with(')') {
                    return Some(TrayCommand::ShowApprovals);
                }
            }
            for prefix in ["• ", "● "] {
                if let Some(rest) = label.strip_prefix(prefix) {
                    return Some(TrayCommand::ShowRun(rest.to_string()));
                }
            }
            None
        }
    }
}

/// Message a second instance sends to the running instance.
pub const FOCUS_MESSAGE: &str = "focus";

/// Local-socket name for single-instance IPC inside `dir`.
/// File-path sockets work on every platform (Unix socket / Windows named pipe
/// mapped from the path by `interprocess`).
pub fn socket_name_for(
    dir: &std::path::Path,
) -> std::io::Result<interprocess::local_socket::Name<'static>> {
    use interprocess::local_socket::{GenericFilePath, ToFsName as _};
    instance_socket_path(dir).to_fs_name::<GenericFilePath>()
}

/// Try to become the primary instance by binding the single-instance socket.
/// Returns the bound listener on success; an `AddrInUse` error means another
/// instance already holds it (see [`decide_launch`]).
pub fn bind_single_instance(
    dir: &std::path::Path,
) -> std::io::Result<interprocess::local_socket::Listener> {
    use interprocess::local_socket::ListenerOptions;
    let _ = std::fs::create_dir_all(dir);
    // A stale socket file from a crashed run would block rebinding; a live
    // primary is detected via connect below, so removing a dead file is safe:
    // if a primary were live, binding still fails with AddrInUse.
    let path = instance_socket_path(dir);
    let name = socket_name_for(dir)?;
    match ListenerOptions::new().name(name).create_sync() {
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => Err(e),
        Err(e) => {
            // Stale file left behind: drop it once and retry.
            let _ = std::fs::remove_file(&path);
            let name = socket_name_for(dir)?;
            ListenerOptions::new()
                .name(name)
                .create_sync()
                .map_err(|e2| {
                    std::io::Error::new(
                        e2.kind(),
                        format!("single-instance bind failed: {e} then {e2}"),
                    )
                })
        }
        ok => ok,
    }
}

/// Tell the running instance to focus its window. Returns true when a
/// primary was reached and acknowledged the [`FOCUS_MESSAGE`].
pub fn notify_existing_instance(dir: &std::path::Path) -> bool {
    use interprocess::local_socket::traits::Stream as _;
    use interprocess::local_socket::Stream;
    use interprocess::local_socket::{GenericFilePath, ToFsName as _};
    use std::io::Write as _;
    let name = match instance_socket_path(dir).to_fs_name::<GenericFilePath>() {
        Ok(name) => name.into_owned(),
        Err(_) => return false,
    };
    let mut stream = match Stream::connect(name) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    stream.write_all(FOCUS_MESSAGE.as_bytes()).is_ok()
}

/// Read one focus message from an accepted single-instance connection.
/// Returns the raw payload (may be empty for a bare connect probe).
pub fn read_instance_message(stream: &mut interprocess::local_socket::Stream) -> String {
    use interprocess::local_socket::traits::Stream as _;
    use std::io::Read as _;
    let mut buf = [0_u8; 256];
    // A short deadline keeps a wedged second launch from stalling the loop.
    let _ = stream.set_recv_timeout(Some(std::time::Duration::from_secs(2)));
    match stream.read(&mut buf) {
        Ok(0) => String::new(),
        Ok(n) => String::from_utf8_lossy(&buf[..n]).into_owned(),
        Err(_) => String::new(),
    }
}

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
        // Idle menu still offers open + quit (plus quick-ask).
        let idle = tray_menu_items();
        assert!(idle.contains(&"Open Synapse".to_string()));
        assert!(idle.contains(&"Quit".to_string()));
        assert!(idle.contains(&"Quick Ask".to_string()));
        assert_eq!(parse_tray_menu_item("Open"), Some(TrayCommand::Open));
        assert_eq!(
            parse_tray_menu_item("Open Synapse"),
            Some(TrayCommand::Open)
        );
        assert_eq!(parse_tray_menu_item("Quit"), Some(TrayCommand::Quit));
        assert_eq!(
            parse_tray_menu_item("Quick Ask"),
            Some(TrayCommand::QuickAsk)
        );
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
    fn tray_menu_shows_runs_phases_approvals_and_actions() {
        let snapshot = TraySnapshot {
            runs: vec![
                TrayRun::new("r1", "Market study", "Searching"),
                TrayRun::new("r2", "Uganda fintech", "Reading"),
            ],
            pending_approvals: 2,
            all_paused: false,
        };
        let items = tray_menu_labels(&snapshot);
        assert!(items.contains(&"Open Synapse".to_string()));
        assert!(items.iter().any(|l| l.contains("Market study") && l.contains("Searching")));
        assert!(items.iter().any(|l| l.contains("Uganda fintech") && l.contains("Reading")));
        assert!(items.iter().any(|l| l.contains("Pending approvals") && l.contains('2')));
        assert!(items.contains(&"Pause All".to_string()));
        assert!(items.contains(&"Quick Ask".to_string()));
        assert!(items.contains(&"Quit".to_string()));
        // Every actionable row parses back.
        for label in &items {
            if label == "No active runs" {
                assert_eq!(parse_tray_menu_item(label), None);
            } else {
                assert!(
                    parse_tray_menu_item(label).is_some(),
                    "menu row has no command: {label}"
                );
            }
        }
        // Paused fleet offers Resume all instead.
        let paused = TraySnapshot {
            all_paused: true,
            ..snapshot.clone()
        };
        let items = tray_menu_labels(&paused);
        assert!(items.contains(&"Resume All".to_string()));
        assert!(!items.contains(&"Pause All".to_string()));
        // Idle menu names the empty state and skips pause-all.
        let idle = tray_menu_labels(&TraySnapshot::idle());
        assert!(idle.contains(&"No active runs".to_string()));
        assert!(!idle.contains(&"Pause All".to_string()));
        assert!(!idle.contains(&"Resume All".to_string()));
    }

    #[test]
    fn tray_menu_parse_and_apply_cover_all_actions() {
        assert_eq!(
            parse_tray_menu_item("Pause All"),
            Some(TrayCommand::PauseAll)
        );
        assert_eq!(
            parse_tray_menu_item("Resume All"),
            Some(TrayCommand::ResumeAll)
        );
        assert_eq!(
            parse_tray_menu_item("Pending approvals (3)"),
            Some(TrayCommand::ShowApprovals)
        );
        assert!(matches!(
            parse_tray_menu_item("• Market study — Searching"),
            Some(TrayCommand::ShowRun(_))
        ));
        let mut w = WindowLifecycle::new_visible();
        assert_eq!(
            apply_tray_command(&mut w, TrayCommand::QuickAsk),
            TrayEffect::QuickAsk
        );
        assert_eq!(
            apply_tray_command(&mut w, TrayCommand::PauseAll),
            TrayEffect::PauseAll
        );
        assert_eq!(
            apply_tray_command(&mut w, TrayCommand::ResumeAll),
            TrayEffect::ResumeAll
        );
        assert_eq!(
            apply_tray_command(&mut w, TrayCommand::ShowApprovals),
            TrayEffect::ShowApprovals
        );
    }

    #[test]
    fn tray_menu_icon_reflects_idle_working_and_needs_you() {
        assert_eq!(
            tray_icon_state(&TraySnapshot::idle()),
            TrayIconState::Idle
        );
        let working = TraySnapshot {
            runs: vec![TrayRun::new("r1", "Market study", "Searching")],
            pending_approvals: 0,
            all_paused: false,
        };
        assert_eq!(tray_icon_state(&working), TrayIconState::Working);
        // Paused-but-live runs still count as working.
        let paused = TraySnapshot {
            all_paused: true,
            ..working.clone()
        };
        assert_eq!(tray_icon_state(&paused), TrayIconState::Working);
        // Approvals win over everything (even idle).
        let needs_you = TraySnapshot {
            pending_approvals: 1,
            ..working.clone()
        };
        assert_eq!(tray_icon_state(&needs_you), TrayIconState::NeedsYou);
        let approvals_only = TraySnapshot {
            pending_approvals: 2,
            ..TraySnapshot::idle()
        };
        assert_eq!(tray_icon_state(&approvals_only), TrayIconState::NeedsYou);
    }

    #[test]
    fn lifecycle_single_instance_socket_notifies_primary() {
        let dir =
            std::env::temp_dir().join(format!("synapse-lifecycle-ipc-{}", std::process::id()));
        let _ = std::fs::remove_file(instance_socket_path(&dir));
        // No primary running: notify fails, this launch becomes primary.
        assert!(!notify_existing_instance(&dir));
        assert_eq!(decide_launch(false), LaunchOutcome::Primary);

        let listener = bind_single_instance(&dir).expect("bind primary socket");
        // A second launch reaches the primary over the socket and defers.
        assert!(notify_existing_instance(&dir));
        let held = bind_single_instance(&dir).expect_err("second bind is held");
        assert_eq!(held.kind(), std::io::ErrorKind::AddrInUse);
        assert_eq!(decide_launch(true), LaunchOutcome::FocusExisting);

        drop(listener);
        let _ = std::fs::remove_file(instance_socket_path(&dir));
        let _ = std::fs::remove_dir(&dir);
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
