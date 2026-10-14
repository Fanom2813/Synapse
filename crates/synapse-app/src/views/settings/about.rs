//! Settings → About: version, where your data lives, project links and credits.

use super::setting_row::{card, row};
use crate::theme::TextRole;
use crate::typography::Typography as _;
use crate::views::shell::ShellView;
use gpui_kit::component::button::Button;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _};
use gpui_kit::{
    div, AnyElement, Context, IntoElement as _, ParentElement as _, Styled as _,
};

const HOMEPAGE: &str = "https://github.com/synapse-ai/synapse";
const ISSUES: &str = "https://github.com/synapse-ai/synapse/issues";

/// Open-source pieces worth naming, with their licenses.
const CREDITS: [(&str, &str); 6] = [
    ("GPUI and GPUI Kit", "Apache-2.0"),
    ("genai (model providers)", "MIT / Apache-2.0"),
    ("Agent collections: agency-agents, wshobson/agents, VoltAgent, lst97", "MIT"),
    ("DiceBear avatar styles", "CC0 / CC BY 4.0"),
    ("Fluent Emoji role badges", "MIT"),
    ("Lucide icons", "ISC"),
];

pub(super) fn page(_view: &ShellView, cx: &mut Context<ShellView>) -> AnyElement {
    let t = cx.theme().clone();
    let data = crate::app_dirs::data_dir();

    let header = h_flex()
        .items_center()
        .gap_4()
        .child(crate::brand_mark::mark(56.0))
        .child(
            v_flex()
                .gap_1()
                .child(div().typeset(TextRole::Heading, cx).child("Synapse"))
                .child(
                    div()
                        .typeset(TextRole::Caption, cx)
                        .text_color(t.colors.muted_foreground)
                        .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                )
                .child(
                    div()
                        .typeset(TextRole::Body, cx)
                        .text_color(t.colors.muted_foreground)
                        .child("A team of AI agents for chat and deep research. MIT licensed."),
                ),
        );

    let open_data = Button::new("about-open-data")
        .small()
        .outline()
        .label("Show in folder")
        .on_click(move |_, _, cx| cx.reveal_path(&data))
        .into_any_element();
    let open_source = Button::new("about-source")
        .small()
        .outline()
        .label("Open")
        .on_click(|_, _, cx| cx.open_url(HOMEPAGE))
        .into_any_element();
    let open_issues = Button::new("about-issues")
        .small()
        .outline()
        .label("Open")
        .on_click(|_, _, cx| cx.open_url(ISSUES))
        .into_any_element();

    let places = card(&t)
        .child(row(
            cx,
            "Source code",
            "Read it, fork it, send a fix.",
            open_source,
        ))
        .child(row(
            cx,
            "Report a problem",
            "Tell us what broke or what you wish it did.",
            open_issues,
        ))
        .child(
            row(
                cx,
                "Your data",
                "Chats, projects and agents stay on this computer.",
                open_data,
            )
            .border_b_0(),
        );

    let mut credits = card(&t);
    for (n, (name, license)) in CREDITS.iter().enumerate() {
        let last = n + 1 == CREDITS.len();
        let line = h_flex()
            .items_center()
            .justify_between()
            .gap_4()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(t.colors.border)
            .child(div().typeset(TextRole::Body, cx).child(*name))
            .child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(t.colors.muted_foreground)
                    .child(*license),
            );
        credits = credits.child(if last { line.border_b_0() } else { line });
    }

    v_flex()
        .gap_5()
        .child(header)
        .child(places)
        .child(div().typeset(TextRole::Heading, cx).child("Credits"))
        .child(credits)
        .into_any_element()
}
