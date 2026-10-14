//! Logo tile for a detected coding CLI (onboarding, Settings → CLI agents).
//!
//! Glyphs are SVGs in `assets/brand/clis/` (Lobe Icons, MIT), coloured or
//! one-colour per CLI. A CLI without a bundled glyph gets a letter tile, so a
//! new agent never needs an asset to appear.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use std::borrow::Cow;

/// How a glyph is drawn.
#[derive(Clone, Copy)]
enum Look {
    /// Brand colours baked into the SVG.
    Colour,
    /// One shape that follows the text colour (light and dark themes).
    Mono,
}

/// (CLI id, asset path, bytes, look).
const LOGOS: [(&str, &str, &[u8], Look); 7] = [
    (
        "claude",
        "brand/clis/claude.svg",
        include_bytes!("../../../assets/brand/clis/claude.svg"),
        Look::Colour,
    ),
    (
        "codex",
        "brand/clis/codex.svg",
        include_bytes!("../../../assets/brand/clis/codex.svg"),
        Look::Colour,
    ),
    (
        "gemini",
        "brand/clis/gemini.svg",
        include_bytes!("../../../assets/brand/clis/gemini.svg"),
        Look::Colour,
    ),
    (
        "copilot",
        "brand/clis/copilot.svg",
        include_bytes!("../../../assets/brand/clis/copilot.svg"),
        Look::Colour,
    ),
    (
        "cursor",
        "brand/clis/cursor.svg",
        include_bytes!("../../../assets/brand/clis/cursor.svg"),
        Look::Mono,
    ),
    (
        "goose",
        "brand/clis/goose.svg",
        include_bytes!("../../../assets/brand/clis/goose.svg"),
        Look::Mono,
    ),
    (
        "opencode",
        "brand/clis/opencode.svg",
        include_bytes!("../../../assets/brand/clis/opencode.svg"),
        Look::Mono,
    ),
];

/// Bytes for a bundled logo path, for `AppAssets`.
pub fn load(path: &str) -> Option<Cow<'static, [u8]>> {
    LOGOS
        .iter()
        .find(|l| l.1 == path)
        .map(|l| Cow::Borrowed(l.2))
}

/// Asset paths under `prefix`, for `AppAssets::list`.
pub fn list(prefix: &str) -> impl Iterator<Item = &'static str> + '_ {
    LOGOS
        .iter()
        .map(|l| l.1)
        .filter(move |p| p.starts_with(prefix))
}

/// A rounded square holding the CLI's logo, or its first letter.
pub fn tile(id: &str, label: &str, size: f32, cx: &App) -> Div {
    let theme = cx.theme();
    let base = div()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.28))
        .bg(theme.colors.secondary)
        .border_1()
        .border_color(theme.colors.border);
    // Detected ids can carry a suffix (`cursor-agent`); the glyph is keyed by the short one.
    let id = id.strip_suffix("-agent").unwrap_or(id);
    match LOGOS.iter().find(|l| l.0 == id) {
        Some((_, path, _, Look::Colour)) => base.child(img(*path).size(px(size * 0.58))),
        Some((_, path, _, Look::Mono)) => base.child(
            svg()
                .path(*path)
                .size(px(size * 0.56))
                .text_color(theme.colors.foreground),
        ),
        None => match crate::provider_logo::path_for(id) {
            Some(path) => base.child(
                svg()
                    .path(path)
                    .size(px(size * 0.56))
                    .text_color(theme.colors.foreground),
            ),
            None => base
                .text_color(theme.colors.muted_foreground)
                .text_size(px(size * 0.45))
                .font_weight(FontWeight::SEMIBOLD)
                .child(
                    label
                        .chars()
                        .next()
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string(),
                ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{list, load};

    #[test]
    fn bundled_logos_load_and_list_by_path() {
        assert!(load("brand/clis/claude.svg").is_some());
        assert!(load("brand/clis/unknown.svg").is_none());
        assert_eq!(list("brand/clis/").count(), 7);
    }
}
