//! Shared layout for settings pages: a titled card of label + control rows.

use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Theme};
use gpui_kit::{div, AnyElement, App, Div, ParentElement as _, Styled as _};

/// A bordered card holding rows.
pub(super) fn card(theme: &Theme) -> Div {
    v_flex()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.colors.border)
        .bg(theme.colors.sidebar)
        .overflow_hidden()
}

/// One row: label and hint on the left, the control on the right.
pub(super) fn row(cx: &App, label: &'static str, hint: &'static str, control: AnyElement) -> Div {
    let theme = cx.theme();
    h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.colors.border)
        .child(
            v_flex()
                .gap_1()
                .child(div().typeset(TextRole::Label, cx).child(label))
                .child(
                    div()
                        .typeset(TextRole::Caption, cx)
                        .text_color(theme.colors.muted_foreground)
                        .child(hint),
                ),
        )
        .child(control)
}
