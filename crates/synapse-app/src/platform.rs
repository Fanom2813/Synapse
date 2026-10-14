//! The only place for `target_os` decisions in the app (AGENTS.md §10).

/// macOS: closing the window hides the app and keeps the native window instead
/// of destroying it, because destroying an NSWindow crashes AppKit on Touch Bar
/// Macs (`_NSTouchBarFinderObservation` KVO exception). See docs/PLATFORM-NOTES.md.
pub const CLOSE_HIDES_APP: bool = cfg!(target_os = "macos");

/// macOS: the sidebar runs full height up to the window edge with the traffic
/// lights over it, and the title bar belongs to the main card only. Windows and
/// Linux keep one full-width title bar because their window buttons sit top-right.
pub const SIDEBAR_UNDER_TITLEBAR: bool = cfg!(target_os = "macos");

/// Where the native window buttons end on the left edge of the title bar, in px.
/// macOS: the traffic lights (inset 16 px, three buttons). Windows and Linux put
/// their buttons top-right, so the left edge is free.
pub const LEFT_CONTROLS_END: f32 = if cfg!(target_os = "macos") {
    72.0
} else {
    12.0
};

