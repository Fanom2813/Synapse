//! Pushes the saved appearance into GPUI Kit's global theme. The one place
//! our palette tokens meet the toolkit.

use crate::theme::FontRole;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{px, App, Hsla};

/// Push the persisted [`Appearance`](crate::theme::Appearance) into GPUI
/// Kit's global theme: mode, palette surfaces, accent and radii. The one
/// place palette tokens meet the toolkit (UI.md §8).
pub fn apply_theme(cx: &mut App, appearance: &crate::theme::Appearance) {
    use crate::theme::{parse_hex, Palette, ThemeMode};
    use gpui_kit::component::scroll::ScrollbarMode;
    use gpui_kit::component::{Theme, ThemeMode as KitMode};
    let os_dark = cx.theme().is_dark();
    let resolved = appearance.mode.resolve(os_dark);
    let kit_mode = match appearance.mode {
        ThemeMode::Light => KitMode::Light,
        ThemeMode::Dark => KitMode::Dark,
        ThemeMode::System => {
            if os_dark {
                KitMode::Dark
            } else {
                KitMode::Light
            }
        }
    };
    Theme::change(kit_mode, None, cx);
    let p = Palette::for_theme(resolved);
    let accent = gpui_kit::rgb(parse_hex(&appearance.accent).unwrap_or(0x3d9df2));
    let c = |hex: u32| -> Hsla { gpui_kit::rgb(hex).into() };
    Theme::update(cx, |t| {
        if !appearance.use_system_font {
            t.font_family = FontRole::Ui.family().into();
        }
        t.mono_font_family = FontRole::Mono.family().into();
        t.font_size = px(14.0 * appearance.font_scale);
        t.radius = px(8.0);
        t.radius_lg = px(14.0);
        t.colors.background = c(p.canvas);
        t.colors.foreground = c(p.text);
        t.colors.border = c(p.border);
        t.colors.muted = c(p.surface);
        t.colors.muted_foreground = c(p.text_muted);
        t.colors.sidebar = c(p.sidebar);
        t.colors.sidebar_foreground = c(p.text);
        t.colors.sidebar_border = c(p.border);
        t.colors.title_bar = c(p.canvas);
        t.colors.title_bar_border = c(p.border);
        t.colors.secondary = c(p.surface);
        t.colors.secondary_hover = c(p.hover);
        t.colors.secondary_foreground = c(p.text);
        t.colors.list_hover = c(p.hover);
        t.colors.popover = c(p.surface);
        // Scrim behind dialogs: the page's own colour, see-through, so what is
        // underneath reads as softly frosted. GPUI cannot blur behind an element.
        t.colors.overlay = c(p.canvas).opacity(0.62);
        t.colors.primary = accent.into();
        t.colors.primary_hover = accent.into();
        t.colors.primary_active = accent.into();
        t.colors.primary_foreground = gpui_kit::rgb(0xffffff).into();
        t.colors.ring = accent.into();
        // One scrollbar look on every OS: no track, a quiet thumb that shows
        // while scrolling (the toolkit would otherwise follow the OS setting).
        t.scrollbar_mode = ScrollbarMode::Scrolling;
        t.colors.scrollbar = gpui_kit::transparent_black();
        t.colors.scrollbar_thumb = c(p.text_muted).opacity(0.35);
        t.colors.scrollbar_thumb_hover = c(p.text_muted).opacity(0.6);
    });
}
