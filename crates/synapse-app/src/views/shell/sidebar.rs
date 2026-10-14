//! Sidebar: projects in the room, or the settings pages while in Settings.

use super::nav_row::nav_row;
use super::ShellView;
use crate::icons;
use crate::navigation::{AppScreen, SettingsSection};
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, TitleBar};
use gpui_kit::{
    div, px, AnyElement, Context, Div, IntoElement as _, ParentElement as _, Styled as _,
};

/// Sidebar width in px; the shell animates between this and a thin gap.
pub(super) const WIDTH: f32 = 248.0;

/// `strip_toggle` is the sidebar toggle when it lives in the sidebar's own
/// title strip (macOS, traffic lights over the sidebar).
pub(super) fn sidebar(
    view: &ShellView,
    strip_toggle: Option<Button>,
    cx: &mut Context<ShellView>,
) -> Div {
    let t = cx.theme().clone();
    let mut column = v_flex().w(px(WIDTH)).flex_none().h_full();
    if let Some(toggle) = strip_toggle {
        column = column.child(
            TitleBar::new()
                .h(px(crate::window_frame::BAR_HEIGHT))
                .bg(t.colors.sidebar)
                .border_b_0()
                .child(h_flex().w_full().justify_end().pr_3().child(toggle)),
        );
    }
    let body = match view.shell.screen() {
        AppScreen::Room => room_nav(view, cx),
        AppScreen::Settings(current) => settings_nav(current, cx),
    };
    column.child(v_flex().flex_1().gap_3().p_3().child(body))
}

fn heading(text: &'static str, cx: &Context<ShellView>) -> Div {
    div()
        .px_2()
        .pb_1()
        .typeset(TextRole::Overline, cx)
        .text_color(cx.theme().colors.muted_foreground)
        .child(text)
}

fn room_nav(view: &ShellView, cx: &mut Context<ShellView>) -> AnyElement {
    let mut projects = v_flex().gap_1();
    for (i, project) in view.shell.projects.iter().enumerate() {
        let id = project.id.clone();
        let selected = view.shell.open_project_id.as_deref() == Some(project.id.as_str());
        projects = projects.child(nav_row(
            ("project-row", i),
            icons::EMPTY_PROJECTS,
            project.name.clone(),
            selected,
            cx,
            cx.listener(move |this, _, _, cx| this.show_project(&id, None, cx)),
        ));
    }
    let new_project = cx.listener(|this, _, window, cx| this.open_new_project_dialog(window, cx));
    let list = match view.shell.sidebar_empty() {
        Some(e) => v_flex()
            .flex_1()
            .child(
                crate::empty_view::empty_card(cx, &e).child(
                    Button::new("new-project-empty")
                        .primary()
                        .small()
                        .icon(icons::PLUS)
                        .label("New project")
                        .on_click(new_project),
                ),
            )
            .into_any_element(),
        None => v_flex().flex_1().child(projects).into_any_element(),
    };
    v_flex()
        .size_full()
        .gap_2()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(heading("PROJECTS", cx))
                .child(
                    Button::new("new-project")
                        .ghost()
                        .small()
                        .icon(icons::PLUS)
                        .tooltip("New project")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_new_project_dialog(window, cx)
                        })),
                ),
        )
        .child(list)
        .child(nav_row(
            "open-settings",
            icons::GEAR,
            "Settings",
            false,
            cx,
            cx.listener(|this, _, window, cx| {
                this.navigate(AppScreen::Settings(SettingsSection::Models), window, cx);
            }),
        ))
        .into_any_element()
}

fn settings_nav(current: SettingsSection, cx: &mut Context<ShellView>) -> AnyElement {
    let mut list = v_flex().gap_1();
    for (i, section) in SettingsSection::ALL.into_iter().enumerate() {
        list = list.child(nav_row(
            ("settings-section", i),
            section.icon(),
            section.title(),
            section == current,
            cx,
            cx.listener(move |this, _, window, cx| {
                this.navigate(AppScreen::Settings(section), window, cx);
            }),
        ));
    }
    v_flex()
        .size_full()
        .gap_2()
        .child(heading("SETTINGS", cx))
        .child(list)
        .into_any_element()
}
