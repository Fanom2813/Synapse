//! Title bar contents (UI.md §1b): sidebar toggle, back, breadcrumb, run pill,
//! inbox, palette, theme and right-panel toggles.

use super::ShellView;
use crate::icons;
use crate::navigation::AppScreen;
use crate::theme::{TextRole, ThemeMode};
use crate::typography::Typography as _;
use crate::window::inbox_text;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::{div, Context, Div, ParentElement as _, Styled as _};

/// `toggle` is the sidebar toggle, unless it lives in the sidebar's own strip.
pub(super) fn bar_row(
    view: &ShellView,
    toggle: Option<Button>,
    cx: &mut Context<ShellView>,
) -> Div {
    let t = cx.theme().clone();
    let muted = t.colors.muted_foreground;
    let screen = view.shell.screen();
    let mut row = h_flex().w_full().items_center().gap_2().px_3();
    if let Some(toggle) = toggle {
        row = row.child(toggle);
    }
    if view.shell.nav.can_go_back() {
        row = row.child(
            Button::new("nav-back")
                .ghost()
                .small()
                .icon(IconName::ArrowLeft)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.shell.go_back();
                    cx.notify();
                })),
        );
    }
    let project = view
        .shell
        .title
        .project_name
        .clone()
        .unwrap_or_else(|| "Synapse".into());
    let crumb = match screen {
        AppScreen::Room => {
            let mut row = h_flex().gap_2().min_w_0().child(crumb(project, false, cx));
            if !view.goal.is_empty() {
                row = row.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .typeset(TextRole::Caption, cx)
                        .text_color(muted)
                        .child(view.goal.clone()),
                );
            }
            row
        }
        AppScreen::Settings(section) => h_flex()
            .gap_2()
            .child(crumb("Settings".to_string(), true, cx))
            .child(div().text_color(muted).child("/"))
            .child(crumb(section.title().to_string(), false, cx)),
    };
    row = row.child(crumb);
    row = row
        .child(div().flex_1())
        .child(
            Button::new("inbox")
                .ghost()
                .small()
                .icon(icons::BELL)
                .label(inbox_text(&view.shell))
                .on_click(|_, _, _| {}),
        )
        .child(
            Button::new("palette")
                .ghost()
                .small()
                .icon(icons::COMMAND)
                .label("K")
                .on_click(|_, _, _| {}),
        )
        .child(
            Button::new("theme-cycle")
                .ghost()
                .small()
                .icon(icons::THEME)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.update_appearance(cx, |a| {
                        a.mode = match a.mode {
                            ThemeMode::System => ThemeMode::Light,
                            ThemeMode::Light => ThemeMode::Dark,
                            ThemeMode::Dark => ThemeMode::System,
                        }
                    });
                })),
        );
    if screen == AppScreen::Room {
        row = row.child(
            Button::new("panel-toggle")
                .ghost()
                .small()
                .icon(icons::PANEL_RIGHT)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.shell.toggle_right_panel();
                    cx.notify();
                })),
        );
    }
    row
}

/// One breadcrumb piece: parents are muted, the current page is full strength.
fn crumb(text: String, parent: bool, cx: &Context<ShellView>) -> Div {
    let t = cx.theme();
    div()
        .typeset(TextRole::Heading, cx)
        .text_color(if parent {
            t.colors.muted_foreground
        } else {
            t.colors.foreground
        })
        .child(text)
}
