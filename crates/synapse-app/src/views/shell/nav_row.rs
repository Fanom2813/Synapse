//! One clickable sidebar row (icon + label). Used for projects and settings
//! pages so every list in the sidebar looks and behaves the same.

use crate::icons::Icon;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::{ActiveTheme as _, Icon as KitIcon, Sizable as _};
use gpui_kit::{
    div, App, ClickEvent, Div, ElementId, InteractiveElement as _, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window,
};

pub fn nav_row(
    id: impl Into<ElementId>,
    icon: Icon,
    label: impl Into<gpui_kit::SharedString>,
    selected: bool,
    cx: &App,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let t = cx.theme();
    let tint = if selected {
        t.colors.foreground
    } else {
        t.colors.muted_foreground
    };
    let hover = t.colors.list_hover;
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded(t.radius)
        .typeset(TextRole::Label, cx)
        .text_color(tint)
        .cursor_pointer()
        .bg(if selected {
            t.colors.list_active
        } else {
            t.transparent
        })
        .hover(move |row| row.bg(hover))
        .child(KitIcon::new(icon).small())
        .child(div().flex_1().truncate().child(label.into()))
        .on_click(on_click)
}
