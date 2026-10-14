//! Where the user is in the main window and how they got there (UI.md §2).
//! Toolkit-free so the routing rules are unit tested without a display.

use crate::icons::{self, Icon};

/// A settings page (UI.md §2.5), in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Providers,
    Models,
    Agents,
    CliAgents,
    Search,
    Mcp,
    Permissions,
    Appearance,
    Background,
    Data,
    About,
}

impl SettingsSection {
    /// Providers and CLIs are set up on the Models page; their own pages are no longer listed.
    pub const ALL: [Self; 9] = [
        Self::Models,
        Self::Agents,
        Self::Search,
        Self::Mcp,
        Self::Permissions,
        Self::Appearance,
        Self::Background,
        Self::Data,
        Self::About,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Providers => "Providers",
            Self::Models => "Models",
            Self::Agents => "Agents",
            Self::CliAgents => "CLI agents",
            Self::Search => "Search",
            Self::Mcp => "MCP servers",
            Self::Permissions => "Permissions",
            Self::Appearance => "Appearance",
            Self::Background => "Background",
            Self::Data => "Data",
            Self::About => "About",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Providers => Icon::Cloud,
            Self::Models => Icon::Cpu,
            Self::Agents => icons::TAB_TEAM,
            Self::CliAgents => Icon::SquareTerminal,
            Self::Search => icons::MAGNIFIER,
            Self::Mcp => Icon::Network,
            Self::Permissions => Icon::ShieldCheck,
            Self::Appearance => Icon::Palette,
            Self::Background => Icon::Power,
            Self::Data => Icon::HardDrive,
            Self::About => Icon::Info,
        }
    }

    /// One line shown under the page title.
    pub fn blurb(self) -> &'static str {
        match self {
            Self::Providers => "Model providers, API keys and which ones are on.",
            Self::Models => "Default models, your AI providers and the coding CLIs on this computer.",
            Self::Agents => "What runs each agent by default: a model, or a coding CLI.",
            Self::CliAgents => "Coding agents found on this computer and their login state.",
            Self::Search => "SearXNG address and search keys for deep research.",
            Self::Mcp => "Tool servers agents can use, with per-tool switches.",
            Self::Permissions => "What agents may do without asking.",
            Self::Appearance => "Theme, accent colour, text size and motion.",
            Self::Background => "Keep running when closed, launch at login, hotkey.",
            Self::Data => "Project folders, export and clearing data.",
            Self::About => "Version, updates and credits.",
        }
    }
}

/// A top-level destination in the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppScreen {
    #[default]
    Room,
    Settings(SettingsSection),
}

impl AppScreen {
    pub fn title(self) -> &'static str {
        match self {
            Self::Room => "Team room",
            Self::Settings(section) => section.title(),
        }
    }

    pub fn is_settings(self) -> bool {
        matches!(self, Self::Settings(_))
    }
}

/// Current screen plus a bounded back stack. Moving to the screen you are
/// already on does not grow the stack, so Back never "does nothing".
#[derive(Debug, Clone, Default)]
pub struct Navigator {
    current: AppScreen,
    back: Vec<AppScreen>,
}

const MAX_HISTORY: usize = 20;

impl Navigator {
    pub fn current(&self) -> AppScreen {
        self.current
    }

    pub fn go(&mut self, to: AppScreen) {
        if to == self.current {
            return;
        }
        self.back.push(self.current);
        if self.back.len() > MAX_HISTORY {
            self.back.remove(0);
        }
        self.current = to;
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Return to the previous screen; false when there is none.
    pub fn go_back(&mut self) -> bool {
        match self.back.pop() {
            Some(prev) => {
                self.current = prev;
                true
            }
            None => false,
        }
    }

    /// Jump to the room and forget history (new project opened, onboarding done).
    pub fn reset_to_room(&mut self) {
        self.current = AppScreen::Room;
        self.back.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_returns_through_settings_pages_to_the_room() {
        let mut nav = Navigator::default();
        nav.go(AppScreen::Settings(SettingsSection::Providers));
        nav.go(AppScreen::Settings(SettingsSection::Appearance));
        nav.go(AppScreen::Settings(SettingsSection::Appearance));
        assert!(nav.go_back());
        assert_eq!(
            nav.current(),
            AppScreen::Settings(SettingsSection::Providers)
        );
        assert!(nav.go_back());
        assert_eq!(nav.current(), AppScreen::Room);
        assert!(!nav.go_back());
    }

    #[test]
    fn history_is_bounded_and_reset_clears_it() {
        let mut nav = Navigator::default();
        for i in 0..100 {
            nav.go(if i % 2 == 0 {
                AppScreen::Settings(SettingsSection::About)
            } else {
                AppScreen::Room
            });
        }
        assert!(nav.back.len() <= MAX_HISTORY);
        nav.reset_to_room();
        assert!(!nav.can_go_back());
        assert_eq!(nav.current(), AppScreen::Room);
    }
}
