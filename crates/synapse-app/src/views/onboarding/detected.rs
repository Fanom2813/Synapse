//! "Found on this computer" list for the Brains step: one row per CLI agent
//! and local model the background probe has seen, with logo and status.

use super::OnboardingView;
use crate::cli_logo;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _};
use gpui_kit::*;

/// One list row: logo tile, name, then whatever sits on the right.
pub(super) fn entry_row(
    tile: Div,
    name: String,
    right: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .gap_3()
        .items_center()
        .child(tile)
        .child(div().flex_1().typeset(TextRole::Label, cx).child(name))
        .child(right)
}

fn status_text(text: &str, colour: Hsla, cx: &App) -> impl IntoElement {
    div()
        .typeset(TextRole::Caption, cx)
        .text_color(colour)
        .child(text.to_string())
}

impl OnboardingView {
    pub(super) fn render_detected(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let det = &self.flow.flow().snapshot().detection;
        let mut list = v_flex().gap_2();
        for c in &det.cli_agents {
            let (status, colour) = if c.logged_in {
                ("Ready", theme.colors.success)
            } else {
                ("Log in needed", theme.colors.warning)
            };
            list = list.child(entry_row(
                cli_logo::tile(&c.id, &c.label, 32.0, cx),
                c.label.clone(),
                status_text(status, colour, cx),
                cx,
            ));
        }
        for m in &det.local_models {
            list = list.child(entry_row(
                cli_logo::tile(&m.server, &m.server, 32.0, cx),
                format!("{} · {}", m.server, m.model),
                status_text("Local", theme.colors.muted_foreground, cx),
                cx,
            ));
        }
        let empty = det.cli_agents.is_empty() && det.local_models.is_empty();
        if empty {
            let note = if det.done {
                "No coding CLIs or local models found. Add a key below, or try it free."
            } else {
                "Looking for installed agents…"
            };
            list = list.child(
                div()
                    .typeset(TextRole::Body, cx)
                    .text_color(theme.colors.muted_foreground)
                    .child(note),
            );
        } else if !det.done {
            list = list.child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(theme.colors.muted_foreground)
                    .child("Still looking…"),
            );
        }
        list
    }
}
