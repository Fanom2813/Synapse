//! Phosphor icon names (UI.md §7). MIT phosphor-icons set, regular weight
//! for chrome, fill for active, duotone for empty states, bold at 12-14px.
//! Only the icons we use are bundled via assets/icons/icons.toml; this module
//! is the single source of truth for names so views never invent new ones.

/// Icon weight per UI.md §7 state language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconWeight {
    Regular,
    Fill,
    Duotone,
    Bold,
}

/// A named icon + its weight. Rendered by the view layer with GPUI `svg()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icon {
    pub name: &'static str,
    pub weight: IconWeight,
}

impl Icon {
    pub const fn new(name: &'static str, weight: IconWeight) -> Self {
        Self { name, weight }
    }
}

// Chrome.
pub const SIDEBAR: Icon = Icon::new("sidebar", IconWeight::Regular);
pub const PANEL_RIGHT: Icon = Icon::new("panel-right", IconWeight::Regular);
pub const MAGNIFIER: Icon = Icon::new("magnifying-glass", IconWeight::Regular);
pub const BELL: Icon = Icon::new("bell", IconWeight::Regular);
pub const COMMAND: Icon = Icon::new("command", IconWeight::Regular);
pub const DOTS: Icon = Icon::new("dots-three", IconWeight::Regular);
pub const PLUS: Icon = Icon::new("plus", IconWeight::Regular);
pub const GEAR: Icon = Icon::new("gear", IconWeight::Regular);

// Message-kind semantic icons (UI.md §7, fixed mapping).
pub const ASK: Icon = Icon::new("chat-circle-dots", IconWeight::Regular);
pub const CHALLENGE: Icon = Icon::new("lightning", IconWeight::Regular);
pub const AGREE: Icon = Icon::new("check-circle", IconWeight::Regular);
pub const REFUSE: Icon = Icon::new("prohibit", IconWeight::Regular);
pub const DELEGATE: Icon = Icon::new("arrow-bend-up-right", IconWeight::Regular);
pub const FINDING: Icon = Icon::new("magnifying-glass-plus", IconWeight::Regular);
pub const PROPOSE: Icon = Icon::new("lightbulb", IconWeight::Regular);
pub const STEER: Icon = Icon::new("steering-wheel", IconWeight::Regular);
pub const NEED_USER: Icon = Icon::new("hand-waving", IconWeight::Regular);
pub const DONE: Icon = Icon::new("flag-checkered", IconWeight::Regular);

/// Empty-state illustration icons (duotone, UI.md §5).
pub const EMPTY_PROJECTS: Icon = Icon::new("folder-open", IconWeight::Duotone);
pub const EMPTY_ROOM: Icon = Icon::new("chats-circle", IconWeight::Duotone);
pub const EMPTY_SOURCES: Icon = Icon::new("books", IconWeight::Duotone);
pub const EMPTY_ARTIFACT: Icon = Icon::new("file-text", IconWeight::Duotone);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_icons_have_names() {
        for icon in [SIDEBAR, ASK, CHALLENGE, DONE, EMPTY_ROOM] {
            assert!(!icon.name.is_empty());
        }
        assert_eq!(EMPTY_ROOM.weight, IconWeight::Duotone);
    }
}
