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
}
