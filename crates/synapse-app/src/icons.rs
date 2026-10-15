//! Icon names (UI.md §7). GPUI Kit's Lucide set is the one icon set; this
//! module is the single source of truth for which icon means what, so views
//! never pick icons ad hoc. Icons beyond GPUI Kit's built-in 101 are embedded
//! by `app_assets::ExtraIcons`; add a new name there too.

pub use gpui_kit::assets::IconName as Icon;

// Chrome.
pub const SIDEBAR: Icon = Icon::PanelLeft;
pub const PANEL_RIGHT: Icon = Icon::PanelRight;
pub const MAGNIFIER: Icon = Icon::Search;
pub const BELL: Icon = Icon::Bell;
pub const COMMAND: Icon = Icon::Command;
pub const DOTS: Icon = Icon::Ellipsis;
pub const PLUS: Icon = Icon::Plus;
pub const REMOVE: Icon = Icon::Minus;
pub const DELETE: Icon = Icon::Delete;
pub const COPY: Icon = Icon::Copy;
pub const COPIED: Icon = Icon::Check;
pub const BACK: Icon = Icon::ArrowLeft;
pub const CHEVRON_DOWN: Icon = Icon::ChevronDown;
pub const CHEVRON_RIGHT: Icon = Icon::ChevronRight;
pub const GEAR: Icon = Icon::Settings;
pub const THEME: Icon = Icon::Contrast;
pub const SEND: Icon = Icon::Send;
pub const STOP: Icon = Icon::Square;

// Right-panel tabs.
pub const TAB_TEAM: Icon = Icon::Users;
pub const TAB_SOURCES: Icon = Icon::Library;
pub const TAB_ARTIFACT: Icon = Icon::FileText;
pub const TAB_ACTIVITY: Icon = Icon::Activity;
pub const TAB_MAP: Icon = Icon::Waypoints;
pub const TAB_SCHEDULED: Icon = Icon::Clock;

// Message-kind semantic icons (UI.md §7, fixed mapping).
pub const ASK: Icon = Icon::MessageCircleMore;
pub const CHALLENGE: Icon = Icon::Zap;
pub const AGREE: Icon = Icon::CircleCheck;
pub const REFUSE: Icon = Icon::Ban;
pub const DELEGATE: Icon = Icon::CornerUpRight;
pub const FINDING: Icon = Icon::Search;
pub const PROPOSE: Icon = Icon::Lightbulb;
pub const STEER: Icon = Icon::Waypoints;
pub const NEED_USER: Icon = Icon::Hand;
pub const DONE: Icon = Icon::Flag;

// Empty-state illustrations.
pub const EMPTY_PROJECTS: Icon = Icon::FolderOpen;
pub const EMPTY_ROOM: Icon = Icon::MessagesSquare;
pub const EMPTY_SOURCES: Icon = Icon::Library;
pub const EMPTY_ARTIFACT: Icon = Icon::FileText;
