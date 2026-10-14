//! Settings pages (UI.md §2.5), shown in the main area while the navigator
//! is on a settings section. Pages that are not built yet say so plainly.

mod appearance;
pub mod providers;
mod setting_row;

use crate::empty::EmptyState;
use crate::navigation::SettingsSection;
use crate::views::shell::ShellView;
use gpui_kit::component::{v_flex, ActiveTheme as _};
use gpui_kit::{
    div, px, AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};

use crate::theme::TextRole;
use crate::typography::Typography as _;

pub(crate) fn page(
    view: &ShellView,
    section: SettingsSection,
    cx: &mut Context<ShellView>,
) -> AnyElement {
    let t = cx.theme().clone();
    let body = match section {
        SettingsSection::Appearance => appearance::page(view, cx),
        SettingsSection::Providers => providers::page(view, cx),
        other => crate::empty_view::empty_card(
            cx,
            &EmptyState {
                icon: other.icon(),
                title: "Not available yet",
                body: other.blurb(),
                action: "",
            },
        )
        .into_any_element(),
    };
    div()
        .id("settings-scroll")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .child(
            v_flex()
                .max_w(px(720.0))
                .mx_auto()
                .gap_5()
                .px_6()
                .py_6()
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().typeset(TextRole::Title, cx).child(section.title()))
                        .child(
                            div()
                                .typeset(TextRole::Body, cx)
                                .text_color(t.colors.muted_foreground)
                                .child(section.blurb()),
                        ),
                )
                .child(body),
        )
        .into_any_element()
}
