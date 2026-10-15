//! A teammate's turn in the main chat, folded into one quiet line: who
//! worked and a glimpse of what they said. Clicking opens the full note.
//! Only the orchestrator (or the member the user addressed) speaks in full;
//! the rest of the discussion stays here, out of the way.

use crate::avatar_view::avatar_img;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::text::TextView;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _};
use gpui_kit::{div, px, App, IntoElement, ParentElement as _, Styled as _};

/// How much of a note shows on the folded line.
const GLIMPSE_CHARS: usize = 90;

/// The first line of `body`, shortened.
pub fn glimpse(body: &str) -> String {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line = line.trim_start_matches(['#', '*', '-', '>', ' ']);
    if line.chars().count() <= GLIMPSE_CHARS {
        return line.to_string();
    }
    let cut: String = line.chars().take(GLIMPSE_CHARS).collect();
    format!("{}…", cut.trim_end())
}

pub struct WorkNote {
    pub index: usize,
    pub author: String,
    pub avatar: String,
    pub body: String,
    pub working: bool,
    pub open: bool,
}

impl WorkNote {
    /// The line, with the full note under it when `open`. `toggle` runs on click.
    pub fn render(self, app: &App, toggle: impl Fn(&mut App) + 'static) -> impl IntoElement {
        let muted = app.theme().colors.muted_foreground;
        let status = if self.working { "working…".to_string() } else { glimpse(&self.body) };
        let head = h_flex()
            .gap_2()
            .items_center()
            .child(avatar_img(&self.avatar, &self.author, 18.0))
            .child(
                Button::new(("work-note", self.index))
                    .ghost()
                    .small()
                    .icon(if self.open { crate::icons::CHEVRON_DOWN } else { crate::icons::CHEVRON_RIGHT })
                    .label(format!("{} · {status}", self.author))
                    .on_click(move |_, _, app: &mut App| toggle(app)),
            );
        let mut note = v_flex().gap_1().max_w(px(680.0)).child(head);
        if self.open && !self.body.trim().is_empty() {
            note = note.child(
                div()
                    .pl_8()
                    .typeset(TextRole::Caption, app)
                    .text_color(muted)
                    .child(TextView::markdown(format!("work-body-{}", self.index), self.body)),
            );
        }
        note
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glimpse_is_the_first_line_without_markdown_marks_and_is_short() {
        assert_eq!(glimpse("\n## Findings\nmore"), "Findings");
        assert_eq!(glimpse(""), "");
        let long = "word ".repeat(40);
        assert!(glimpse(&long).chars().count() <= GLIMPSE_CHARS + 1);
        assert!(glimpse(&long).ends_with('…'));
    }
}
