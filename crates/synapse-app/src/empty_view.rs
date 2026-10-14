//! Shared empty-state card (UI.md §5): one component for every empty area.
//! The copy comes from [`crate::empty`]; this only draws it.

use crate::empty::EmptyState;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::{v_flex, ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::{div, px, App, Div, ParentElement as _, Styled as _};

/// Centered card: soft badge, title, body. The primary action is a button
/// the caller adds, because only the caller knows what it does.
pub fn empty_card(cx: &App, state: &EmptyState) -> Div {
    let theme = cx.theme();
    v_flex()
        .items_center()
        .gap_2()
        .p_6()
        .text_center()
        .child(
            div()
                .size(px(44.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.colors.muted)
                .text_color(theme.colors.primary)
                .child(Icon::new(state.icon).large()),
        )
        .child(div().typeset(TextRole::Heading, cx).child(state.title))
        .child(
            div()
                .typeset(TextRole::Body, cx)
                .max_w(px(280.0))
                .text_color(theme.colors.muted_foreground)
                .child(state.body),
        )
}
