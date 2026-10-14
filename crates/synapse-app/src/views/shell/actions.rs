//! Keyboard shortcuts for navigating the main window (UI.md §1).
//! `secondary` is Cmd on macOS and Ctrl elsewhere.

use gpui_kit::{actions, App, KeyBinding};

actions!(
    synapse_nav,
    [
        ToggleSidebar,
        TogglePanel,
        OpenSettings,
        GoBack,
        PanelTab1,
        PanelTab2,
        PanelTab3,
        PanelTab4,
        PanelTab5,
        PanelTab6,
    ]
);

/// Key context set on the shell root; bindings only apply inside it.
pub const CONTEXT: &str = "SynapseShell";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-b", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-b", TogglePanel, Some(CONTEXT)),
        KeyBinding::new("secondary-,", OpenSettings, Some(CONTEXT)),
        KeyBinding::new("escape", GoBack, Some(CONTEXT)),
        KeyBinding::new("secondary-1", PanelTab1, Some(CONTEXT)),
        KeyBinding::new("secondary-2", PanelTab2, Some(CONTEXT)),
        KeyBinding::new("secondary-3", PanelTab3, Some(CONTEXT)),
        KeyBinding::new("secondary-4", PanelTab4, Some(CONTEXT)),
        KeyBinding::new("secondary-5", PanelTab5, Some(CONTEXT)),
        KeyBinding::new("secondary-6", PanelTab6, Some(CONTEXT)),
    ]);
}
