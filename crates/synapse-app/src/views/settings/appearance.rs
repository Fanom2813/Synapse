//! Settings → Appearance: theme, accent, text size, motion (UI.md §8).

use super::setting_row::{card, row};
use crate::theme::{parse_hex, ThemeMode, ACCENT_PRESETS};
use crate::typography::Typography as _;
use crate::views::shell::ShellView;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, ActiveTheme as _, Selectable as _, Sizable as _};
use gpui_kit::{
    div, px, rgb, AnyElement, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _,
};

pub(super) fn page(view: &ShellView, cx: &mut Context<ShellView>) -> AnyElement {
    let t = cx.theme().clone();
    let a = view.shell.appearance.clone();

    let mut modes = h_flex().gap_1();
    for (mode, label) in [
        (ThemeMode::System, "System"),
        (ThemeMode::Light, "Light"),
        (ThemeMode::Dark, "Dark"),
    ] {
        modes = modes.child(
            Button::new(("theme-mode", mode as usize))
                .small()
                .ghost()
                .label(label)
                .selected(a.mode == mode)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.update_appearance(cx, |a| a.mode = mode);
                })),
        );
    }

    let mut swatches = h_flex().gap_2();
    for (i, (_name, hex)) in ACCENT_PRESETS.into_iter().enumerate() {
        let selected = a.accent == hex;
        let colour = rgb(parse_hex(hex).unwrap_or(0x3d9df2));
        swatches = swatches.child(
            div()
                .id(("accent", i))
                .size(px(22.0))
                .rounded_full()
                .bg(colour)
                .border_2()
                .border_color(if selected {
                    t.colors.foreground
                } else {
                    t.transparent
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.update_appearance(cx, |a| a.accent = hex.to_string());
                })),
        );
    }

    let size = h_flex()
        .gap_2()
        .items_center()
        .child(
            Button::new("font-smaller")
                .small()
                .ghost()
                .label("A−")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.update_appearance(cx, |a| {
                        a.font_scale = (a.font_scale - 0.1).clamp(0.75, 2.0)
                    });
                })),
        )
        .child(
            div()
                .w(px(44.0))
                .text_center()
                .figures()
                .child(format!("{:.0}%", a.font_scale * 100.0)),
        )
        .child(
            Button::new("font-larger")
                .small()
                .ghost()
                .label("A+")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.update_appearance(cx, |a| {
                        a.font_scale = (a.font_scale + 0.1).clamp(0.75, 2.0)
                    });
                })),
        );

    let reduce = Switch::new("reduce-motion")
        .checked(a.reduce_motion)
        .on_click(cx.listener(|this, checked: &bool, _, cx| {
            let on = *checked;
            this.update_appearance(cx, |a| a.reduce_motion = on);
        }));
    let system_font = Switch::new("system-font")
        .checked(a.use_system_font)
        .on_click(cx.listener(|this, checked: &bool, _, cx| {
            let on = *checked;
            this.update_appearance(cx, |a| a.use_system_font = on);
        }));

    card(&t)
        .child(row(
            cx,
            "Theme",
            "Follow the system or pick one.",
            modes.into_any_element(),
        ))
        .child(row(
            cx,
            "Accent colour",
            "Buttons, selection and focus.",
            swatches.into_any_element(),
        ))
        .child(row(
            cx,
            "Text size",
            "Scales the interface text.",
            size.into_any_element(),
        ))
        .child(row(
            cx,
            "Use system font",
            "Instead of Atkinson Hyperlegible.",
            system_font.into_any_element(),
        ))
        .child(row(
            cx,
            "Reduce motion",
            "Replace movement with short fades.",
            reduce.into_any_element(),
        ))
        .into_any_element()
}
