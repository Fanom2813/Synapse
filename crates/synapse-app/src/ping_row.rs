//! The small avatars under a message showing who it asked for something,
//! like the "seen by" row in Slack. Hovering one shows who asked whom, and
//! what was asked.

use crate::avatar_view::avatar_img;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{div, px, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _};
use gpui_kit::StatefulInteractiveElement as _;

/// One avatar per teammate asked: `(name, avatar json, what was asked)`.
pub fn ping_row(message: usize, from: &str, pinged: Vec<(String, String, String)>) -> impl IntoElement {
    let from = from.to_string();
    h_flex().gap_1().items_center().children(pinged.into_iter().enumerate().map(
        move |(n, (to, avatar, body))| {
            let from = from.clone();
            div()
                .id(("ping", message * 16 + n))
                .rounded(px(999.0))
                .overflow_hidden()
                .child(avatar_img(&avatar, &to, 16.0))
                .tooltip(move |window, cx| {
                    let (from, to, body) = (from.clone(), to.clone(), body.clone());
                    Tooltip::element(move |_, _| {
                        v_flex()
                            .max_w(px(320.0))
                            .gap_1()
                            .child(div().child(format!("{from} asked {to}")))
                            .child(div().child(body.clone()))
                    })
                    .build(window, cx)
                })
        },
    ))
}
