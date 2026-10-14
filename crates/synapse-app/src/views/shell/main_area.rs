//! The main area: whatever screen the navigator points at.

use super::ShellView;
use crate::navigation::AppScreen;
use gpui_kit::component::v_flex;
use gpui_kit::{div, AnyElement, Context, IntoElement as _, ParentElement as _, Styled as _};

pub(super) fn main_area(view: &ShellView, cx: &mut Context<ShellView>) -> AnyElement {
    match view.shell.screen() {
        AppScreen::Room => v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .py_4()
                    .child(view.chat.clone()),
            )
            .into_any_element(),
        AppScreen::Settings(section) => crate::views::settings::page(view, section, cx),
    }
}
