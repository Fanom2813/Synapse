//! Persisted appearance settings: theme mode, accent, density, font size.
//!
//! UI.md §8: System / Light / Dark + one accent colour. Avatar colours are
//! theme-independent. Applied to GPUI Kit's theme system by the view layer;
//! this module is deliberately UI-toolkit free so it is unit testable.

use std::fmt;

/// Theme mode chosen by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    /// Follow the OS setting.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    /// Parse from the persisted string form.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    /// Persisted string form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Resolve against the OS-reported dark state.
    pub fn resolve(self, os_dark: bool) -> ResolvedTheme {
        match self {
            Self::Light => ResolvedTheme::Light,
            Self::Dark => ResolvedTheme::Dark,
            Self::System if os_dark => ResolvedTheme::Dark,
            Self::System => ResolvedTheme::Light,
        }
    }
}

/// Concrete theme after resolving `System` against the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTheme {
    Light,
    Dark,
}

impl fmt::Display for ResolvedTheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Light => write!(f, "light"),
            Self::Dark => write!(f, "dark"),
        }
    }
}

/// Spring parameters (stiffness, damping) from UI.md §5.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
}

/// Motion tokens from UI.md §5. Single source of truth; views must not
/// inline animation durations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionTokens {
    /// 80 ms hover/press colour, focus ring.
    pub instant_ms: u32,
    /// 140 ms tooltips, chips, toggles.
    pub quick_ms: u32,
    /// 200 ms popovers, menus, message enter.
    pub standard_ms: u32,
    /// 280 ms approval cards, onboarding steps.
    pub emphasized_ms: u32,
    /// Stagger per list item, max 6 items.
    pub stagger_ms: u32,
    /// Max staggered items.
    pub stagger_max: u32,
    /// stiffness 170, damping 26: avatars, drag/resize settle, list reflow.
    pub spring_soft: Spring,
    /// stiffness 260, damping 18: send→stop morph, badge bounce, avatar hop.
    pub spring_bouncy: Spring,
    /// stiffness 700, damping 53 (critically damped, ~0.2 s, no overshoot):
    /// panels and sidebars opening and closing. Snappy and interruptible.
    pub spring_snappy: Spring,
}

impl MotionTokens {
    /// Exit is a faster fade: 60 % of the enter duration (UI.md §5).
    pub fn exit_ms(self, enter_ms: u32) -> u32 {
        enter_ms * 60 / 100
    }

    /// Stagger delay for item `index` (0-based); capped at [`Self::stagger_max`].
    pub fn stagger_delay_ms(self, index: u32) -> u32 {
        self.stagger_ms * index.min(self.stagger_max.saturating_sub(1))
    }
}

impl Default for MotionTokens {
    fn default() -> Self {
        Self {
            instant_ms: 80,
            quick_ms: 140,
            standard_ms: 200,
            emphasized_ms: 280,
            stagger_ms: 30,
            stagger_max: 6,
            spring_soft: Spring {
                stiffness: 170.0,
                damping: 26.0,
            },
            spring_bouncy: Spring {
                stiffness: 260.0,
                damping: 18.0,
            },
            spring_snappy: Spring {
                stiffness: 700.0,
                damping: 53.0,
            },
        }
    }
}

/// Typography roles from UI.md §6. Font files are loaded by the view layer
/// with GPUI's text system; this only names the roles so views stay consistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontRole {
    /// Bricolage Grotesque: brand, display, project titles, empty states.
    Display,
    /// Atkinson Hyperlegible Next: every message, label, menu, button.
    Ui,
    /// Newsreader: reports, artifacts, long findings.
    Reading,
    /// Atkinson Hyperlegible Mono: code, data, tool calls, numbers.
    Mono,
}

impl FontRole {
    /// Font family name per UI.md §6. Never hard-code families in views.
    pub fn family(self) -> &'static str {
        match self {
            Self::Display => "Bricolage Grotesque",
            Self::Ui => "Atkinson Hyperlegible Next",
            Self::Reading => "Newsreader",
            Self::Mono => "Atkinson Hyperlegible Mono",
        }
    }
}

/// Surface and text colours (0xRRGGBB) for one resolved theme. Calm neutrals
/// with a single accent, in the spirit of the Grok Bot / dots reference apps:
/// the sidebar sits one step off the canvas, cards one step above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub canvas: u32,
    pub sidebar: u32,
    pub surface: u32,
    pub hover: u32,
    pub border: u32,
    pub text: u32,
    pub text_muted: u32,
}

impl Palette {
    pub const fn for_theme(theme: ResolvedTheme) -> Self {
        match theme {
            ResolvedTheme::Dark => Self {
                canvas: 0x0f1012,
                sidebar: 0x15161a,
                surface: 0x1b1c21,
                hover: 0x24262c,
                border: 0x2a2c33,
                text: 0xececf0,
                text_muted: 0x8b8d98,
            },
            ResolvedTheme::Light => Self {
                canvas: 0xffffff,
                sidebar: 0xf6f6f8,
                surface: 0xf0f0f3,
                hover: 0xe8e8ec,
                border: 0xe3e3e8,
                text: 0x17181c,
                text_muted: 0x6b6d78,
            },
        }
    }
}

/// Parse `#rrggbb` / `#rgb` into 0xRRGGBB; `None` if malformed.
pub fn parse_hex(s: &str) -> Option<u32> {
    if !is_hex_colour(s) {
        return None;
    }
    let h = &s[1..];
    let full: String = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect()
    } else {
        h.to_string()
    };
    u32::from_str_radix(&full, 16).ok()
}

/// Brand colours, taken from the logo (`assets/brand/synapse-mark.svg`).
/// Use these where the brand should show (status dots, active agent, tray);
/// they are also the first presets below.
pub mod brand {
    pub const GREEN: &str = "#34d27b";
    pub const BLUE: &str = "#3d9df2";
    pub const AMBER: &str = "#f5a524";
    /// The logo tile background; a dark green-black.
    pub const INK: &str = "#0f1a14";
}

/// Accent colours offered in Settings → Appearance (name, hex).
pub const ACCENT_PRESETS: [(&str, &str); 6] = [
    ("Blue", brand::BLUE),
    ("Green", brand::GREEN),
    ("Violet", "#8b7cf6"),
    ("Amber", brand::AMBER),
    ("Pink", "#ec6aa6"),
    ("Teal", "#2ec4b6"),
];

/// Where text sits in the hierarchy. Few sizes with real contrast between
/// them (UI.md §6 scale); views pick a role, never a raw size or weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    /// Onboarding and hero moments.
    Display,
    /// Page titles.
    Title,
    /// Card and section titles, selected-item names.
    Heading,
    /// Messages and reading text.
    Body,
    /// Controls, nav rows, setting names.
    Label,
    /// Hints, metadata, counts.
    Caption,
    /// Small section labels such as PROJECTS.
    Overline,
    /// Code, tool calls, ids.
    Mono,
}

/// Size (px at 100 %), weight, line-height ratio and family for a [`TextRole`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeSpec {
    pub size: f32,
    pub weight: u16,
    pub line_height: f32,
    pub font: FontRole,
}

impl TextRole {
    /// Weights match the bundled files: UI 400/500/700, display 600/800.
    pub const fn spec(self) -> TypeSpec {
        let (size, weight, line_height, font) = match self {
            Self::Display => (32.0, 800, 1.15, FontRole::Display),
            Self::Title => (20.0, 600, 1.25, FontRole::Display),
            Self::Heading => (16.0, 500, 1.3, FontRole::Ui),
            Self::Body => (14.0, 400, 1.5, FontRole::Ui),
            Self::Label => (13.0, 500, 1.3, FontRole::Ui),
            Self::Caption => (12.0, 400, 1.4, FontRole::Ui),
            Self::Overline => (12.0, 700, 1.3, FontRole::Ui),
            Self::Mono => (13.0, 400, 1.5, FontRole::Mono),
        };
        TypeSpec {
            size,
            weight,
            line_height,
            font,
        }
    }
}

/// Full appearance record, persisted as JSON in settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub mode: ThemeMode,
    /// Hex accent colour, e.g. `#3d9df2`.
    pub accent: String,
    pub reduce_motion: bool,
    /// When true the UI role uses the system font instead of Atkinson.
    pub use_system_font: bool,
    /// UI font-size scale multiplier (1.0 = default).
    pub font_scale: f32,
    pub motion: MotionTokens,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            mode: ThemeMode::System,
            accent: "#3d9df2".to_string(),
            reduce_motion: false,
            use_system_font: false,
            font_scale: 1.0,
            motion: MotionTokens::default(),
        }
    }
}

impl Appearance {
    /// Serialize to the JSON string stored in the settings table.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "mode": self.mode.as_str(),
            "accent": self.accent,
            "reduce_motion": self.reduce_motion,
            "use_system_font": self.use_system_font,
            "font_scale": self.font_scale,
        })
        .to_string()
    }

    /// Parse back; unknown/missing fields fall back to defaults so a
    /// corrupt row never breaks window open.
    pub fn from_json(s: &str) -> Self {
        let mut out = Self::default();
        let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else {
            return out;
        };
        if let Some(m) = v.get("mode").and_then(|m| m.as_str()) {
            out.mode = ThemeMode::from_str(m);
        }
        if let Some(a) = v.get("accent").and_then(|a| a.as_str()) {
            if is_hex_colour(a) {
                out.accent = a.to_string();
            }
        }
        if let Some(r) = v.get("reduce_motion").and_then(|r| r.as_bool()) {
            out.reduce_motion = r;
        }
        if let Some(u) = v.get("use_system_font").and_then(|u| u.as_bool()) {
            out.use_system_font = u;
        }
        if let Some(f) = v.get("font_scale").and_then(|f| f.as_f64()) {
            let f = f as f32;
            if (0.75..=2.0).contains(&f) {
                out.font_scale = f;
            }
        }
        out
    }
}

fn is_hex_colour(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() == 7 || b.len() == 4) && b[0] == b'#' && b[1..].iter().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_shell_roundtrip_and_resolve() {
        let a = Appearance::default();
        let b = Appearance::from_json(&a.to_json());
        assert_eq!(a, b);
        assert_eq!(ThemeMode::System.resolve(true), ResolvedTheme::Dark);
        assert_eq!(ThemeMode::System.resolve(false), ResolvedTheme::Light);
        assert_eq!(ThemeMode::Dark.resolve(false), ResolvedTheme::Dark);
        // Corrupt input falls back, never panics.
        assert_eq!(Appearance::from_json("not json"), Appearance::default());
        let mut bad = Appearance::from_json(r#"{"mode":"neon","accent":"red","font_scale":99}"#);
        assert_eq!(bad.mode, ThemeMode::System);
        assert_eq!(bad.accent, "#3d9df2");
        assert_eq!(bad.font_scale, 1.0);
        bad = Appearance::from_json(r#"{"mode":"dark"}"#);
        assert_eq!(bad.mode, ThemeMode::Dark);
    }

    #[test]
    fn type_roles_stay_on_the_ui_md_scale_and_bundled_weights() {
        const SCALE: [f32; 8] = [12.0, 13.0, 14.0, 16.0, 20.0, 24.0, 32.0, 48.0];
        for role in [
            TextRole::Display,
            TextRole::Title,
            TextRole::Heading,
            TextRole::Body,
            TextRole::Label,
            TextRole::Caption,
            TextRole::Overline,
            TextRole::Mono,
        ] {
            let spec = role.spec();
            assert!(SCALE.contains(&spec.size), "{role:?} size off the scale");
            assert!(spec.line_height >= 1.15, "{role:?} leading too tight");
            let bundled: &[u16] = match spec.font {
                FontRole::Display => &[600, 800],
                _ => &[400, 500, 700],
            };
            assert!(
                bundled.contains(&spec.weight),
                "{role:?} weight not bundled"
            );
        }
        // Reading text gets the most generous leading.
        assert!(TextRole::Body.spec().line_height > TextRole::Label.spec().line_height);
    }

    #[test]
    fn hex_parses_short_and_long_forms() {
        assert_eq!(parse_hex("#3d9df2"), Some(0x3d9df2));
        assert_eq!(parse_hex("#fa0"), Some(0xffaa00));
        assert_eq!(parse_hex("blue"), None);
    }
}
