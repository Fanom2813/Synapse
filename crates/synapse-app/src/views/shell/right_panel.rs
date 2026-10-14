//! Right panel: tab strip and the active tab's content (UI.md §1).

use super::{tab_icon, ShellView};
use crate::layout::PanelTab;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Selectable as _, Sizable as _};
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::{
    div, px, Context, Div, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};

/// Panel width in px, without the gap around it.
pub(super) const WIDTH: f32 = 300.0;

/// The panel is its own rounded card, like the main one.
pub(super) fn right_panel(view: &ShellView, cx: &mut Context<ShellView>) -> Div {
    let t = cx.theme().clone();
    let active = view.shell.layout.right_panel_tab;
    let mut tabs = h_flex().w_full().justify_between();
    for tab in PanelTab::ALL {
        tabs = tabs.child(
            Button::new(("panel-tab", tab.index()))
                .small()
                .icon(tab_icon(tab))
                .tooltip(tab.title())
                .ghost()
                .selected(tab == active)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.shell.select_panel_tab(tab);
                    cx.notify();
                })),
        );
    }
    let heading = h_flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .px_1()
                .typeset(TextRole::Heading, cx)
                .child(active.title()),
        );
    let can_add = active == PanelTab::Team && view.shell.open_project_id.is_some();
    let heading = if can_add {
        heading.child(
            Button::new("add-agent")
                .ghost()
                .small()
                .icon(crate::icons::PLUS)
                .tooltip("Add agent")
                .on_click(cx.listener(|this, _, window, cx| this.open_add_agent_dialog(window, cx))),
        )
    } else {
        heading
    };
    let body = if active == PanelTab::Team && !view.team.is_empty() {
        let mut list = v_flex().flex_1().gap_1();
        for (n, agent) in view.team.iter().enumerate() {
            list = list.child(agent_row(n, agent, cx));
        }
        list.into_any_element()
    } else {
        let empty = match active {
        PanelTab::Sources => crate::empty::no_sources(),
        PanelTab::Artifact => crate::empty::no_artifact(),
        PanelTab::Team => crate::empty::no_team(),
        _ => crate::empty::empty_room(),
        };
        let mut card = crate::empty_view::empty_card(cx, &empty);
        if can_add {
            card = card.child(
                Button::new("add-agent-empty")
                    .primary()
                    .small()
                    .icon(crate::icons::PLUS)
                    .label("Add agent")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_add_agent_dialog(window, cx)),
                    ),
            );
        }
        card.into_any_element()
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
        .child(heading)
        .child(body)
}

/// One teammate: avatar (click to change it), name, role and a remove button.
fn agent_row(n: usize, agent: &synapse_store::Agent, cx: &mut Context<ShellView>) -> Div {
    let t = cx.theme().clone();
    let (avatar_id, remove_id) = (agent.id.clone(), agent.id.clone());
    h_flex()
        .items_center()
        .gap_2()
        .px_1()
        .py_1()
        .child(
            div()
                .id(("agent-avatar", n))
                .cursor_pointer()
                .child(crate::avatar_view::avatar_img(&agent.avatar, &agent.name, 32.0))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_avatar_dialog(&avatar_id, window, cx)
                })),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().typeset(TextRole::Label, cx).truncate().child(agent.name.clone()))
                .child(
                    div()
                        .typeset(TextRole::Caption, cx)
                        .text_color(t.colors.muted_foreground)
                        .truncate()
                        .child(agent.role.clone()),
                ),
        )
        .child(
            Button::new(("agent-remove", n))
                .ghost()
                .small()
                .icon(crate::icons::REMOVE)
                .tooltip("Remove from team")
                .on_click(cx.listener(move |this, _, _, cx| this.remove_agent(&remove_id, cx))),
        )
}
