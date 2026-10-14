//! Right panel: tab strip and the active tab's content (UI.md §1).

use super::{tab_icon, ShellView};
use crate::layout::PanelTab;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Selectable as _, Sizable as _};
use gpui_kit::{px, Context, Div, ParentElement as _, Styled as _};

/// Panel width in px, without the gap around it.
pub(super) const WIDTH: f32 = 300.0;

/// The panel is its own rounded card, like the main one.
pub(super) fn right_panel(view: &ShellView, cx: &mut Context<ShellView>) -> Div {
    let t = cx.theme().clone();
    let active = view.shell.layout.right_panel_tab;
    let mut tabs = h_flex().flex_wrap().gap_1();
    for tab in PanelTab::ALL {
        tabs = tabs.child(
            Button::new(("panel-tab", tab.index()))
                .small()
                .icon(tab_icon(tab))
                .label(tab.title())
                .ghost()
                .selected(tab == active)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.shell.select_panel_tab(tab);
                    cx.notify();
                })),
        );
    }
    let empty = match active {
        PanelTab::Sources => crate::empty::no_sources(),
        PanelTab::Artifact => crate::empty::no_artifact(),
        _ => crate::empty::empty_room(),
    };
    v_flex()
        .w(px(WIDTH))
        .flex_none()
        .h_full()
        .gap_3()
        .p_3()
        .rounded(t.radius_lg)
        .border_1()
        .border_color(t.colors.border)
        .bg(t.colors.background)
        .overflow_hidden()
        .child(tabs)
        .child(crate::empty_view::empty_card(cx, &empty))
}
