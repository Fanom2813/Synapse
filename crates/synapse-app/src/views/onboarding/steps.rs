//! One onboarding step, centred on the plain window: progress dots, title, the step's
//! controls, then Back / Skip / Continue.

use super::OnboardingView;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use crate::window::open_store;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _};
use gpui_kit::*;

impl OnboardingView {
    /// The logo's home is the window frame (a small centred mark in the bar).
    /// On the first step it flies down to hero size above the card, and flies
    /// back up when you leave. One spring drives size, position and the room
    /// it takes in the column, so nothing can drift apart.
    /// `top` is where the slot starts below the bar, `width` the window width, in px.
    pub(super) fn render_logo_slot(&self, hello: bool, top: f32, width: f32) -> impl IntoElement {
        const HERO: f32 = 112.0;
        const SMALL: f32 = 22.0;
        const GAP: f32 = 24.0;
        let bar_x = crate::platform::LEFT_CONTROLS_END + 8.0 + SMALL / 2.0;
        let bar_centre = -(top + crate::window_frame::BAR_HEIGHT / 2.0);
        let (cfg, playback) = self.reveal.spring_soft();
        div().relative().w_full().with_spring(
            "ob-logo",
            SpringAnimation::new(cfg)
                .to(hello)
                .from(false)
                .playback(playback),
            move |slot, phase| {
                let size = phase.interpolate(SMALL, HERO);
                let centre = phase.interpolate(bar_centre, HERO / 2.0);
                let centre_x = phase.interpolate(bar_x, width / 2.0);
                slot.h(px(phase.interpolate(0.0, HERO + GAP))).child(
                    div()
                        .absolute()
                        .top(px(centre - size / 2.0))
                        .left(px(centre_x - size / 2.0))
                        .child(crate::brand_mark::mark(size)),
                )
            },
        )
    }

    /// One onboarding step, centred on the plain window (UI.md §9): progress dots,
    /// display-size title, the step's controls, then Back / Skip / Continue.
    pub(super) fn render_step_card(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::onboarding::Step;
        use gpui_kit::component::button::{Button, ButtonVariants};
        use gpui_kit::component::input::Input;
        let theme = cx.theme().clone();
        let step = self.flow.flow().step();
        let (n, total) = self.flow.flow().progress();
        let snap = self.flow.flow().snapshot().clone();
        let can_continue = self.flow.can_continue();
        let continue_label = if step == Step::AllSet {
            "Start"
        } else {
            "Continue"
        };
        let (cfg, playback) = self.reveal.spring_soft();
        let dots = (1..=total).fold(h_flex().gap_2(), |row, i| {
            row.child(
                div()
                    .h(px(6.0))
                    .rounded_full()
                    .bg(if i <= n {
                        theme.colors.primary
                    } else {
                        theme.colors.border
                    })
                    .with_spring(
                        ("ob-dot", i),
                        SpringAnimation::new(cfg)
                            .to(i == n)
                            .from(i == n)
                            .playback(playback),
                        |dot, phase| dot.w(phase.interpolate(px(6.0), px(22.0))),
                    ),
            )
        });
        let mut col = v_flex()
            .w(px(560.0))
            .max_w_full()
            .gap_4()
            .p_4()
            .on_key_down(
                cx.listener(|this: &mut Self, event: &KeyDownEvent, window, cx| {
                    let key: &str = &event.keystroke.key;
                    match key {
                        "enter" => this.do_continue(window, cx),
                        "escape" => this.do_back(cx),
                        _ => {}
                    }
                }),
            )
            .child(dots)
            .child(
                self.reveal.enter(
                    div()
                        .typeset(TextRole::Display, cx)
                        .child(step.title().to_string()),
                    format!("ob-title-{n}"),
                    0,
                    (0.0, 10.0),
                ),
            );
        let mut body = v_flex().gap_4();
        match step {
            Step::Hello => {
                body = body
                    .child(
                        div()
                            .text_color(theme.colors.muted_foreground)
                            .child("Your research team lives here. One question at a time."),
                    )
                    .child(
                        Button::new("ob-import")
                            .label("I've used Synapse before → skip")
                            .secondary()
                            .on_click(cx.listener(|this, _, _, cx| this.do_skip(cx))),
                    );
            }
            Step::LeadName => {
                col =
                    col.child(Input::new(&self.name_input)).child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(Button::new("ob-name-dottie").label("Dottie").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.set_name_chip(window, cx, "Dottie");
                                }),
                            ))
                            .child(
                                Button::new("ob-name-mira")
                                    .label("Mira")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.set_name_chip(window, cx, "Mira");
                                    })),
                            )
                            .child(Button::new("ob-name-zara").label("Zara").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.set_name_chip(window, cx, "Zara");
                                }),
                            )),
                    );
            }
            Step::LeadLook => {
                let look = format!(
                    "{} · {} · {} {}",
                    snap.shape, snap.palette, snap.eyes, snap.accessory
                );
                body = body
                    .child(div().text_color(theme.colors.muted_foreground).child(look))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(Button::new("ob-shape-bean").label("bean").on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(store) = open_store() {
                                        let s = this.flow.flow().snapshot().clone();
                                        this.flow.set_look(
                                            &store,
                                            "bean",
                                            &s.palette,
                                            &s.eyes,
                                            &s.accessory,
                                        );
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(Button::new("ob-shape-round").label("round").on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(store) = open_store() {
                                        let s = this.flow.flow().snapshot().clone();
                                        this.flow.set_look(
                                            &store,
                                            "round",
                                            &s.palette,
                                            &s.eyes,
                                            &s.accessory,
                                        );
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(
                                Button::new("ob-surprise")
                                    .label("Surprise me")
                                    .secondary()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(store) = open_store() {
                                            // Deterministic shuffle from the name so it is
                                            // stable across restarts, bounced by the view.
                                            let n: usize = this
                                                .flow
                                                .flow()
                                                .snapshot()
                                                .lead_name
                                                .bytes()
                                                .map(|b| b as usize)
                                                .sum::<usize>()
                                                + this.flow.flow().snapshot().skipped.len();
                                            let shapes = ["bean", "round", "pear", "cloud", "drop"];
                                            let palettes = ["green", "amber", "violet", "blue"];
                                            this.flow.set_look(
                                                &store,
                                                shapes[n % shapes.len()],
                                                palettes[(n / shapes.len()) % palettes.len()],
                                                "diamond",
                                                "",
                                            );
                                        }
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            Step::Brains => {
                body = body
                    .child(self.render_detected(cx))
                    .child(self.render_providers(cx));
                if !can_continue {
                    body = body.child(
                        div()
                            .typeset(TextRole::Caption, cx)
                            .text_color(theme.colors.warning)
                            .child(
                                self.flow
                                    .flow()
                                    .blocked_hint()
                                    .unwrap_or("Still looking…")
                                    .to_string(),
                            ),
                    );
                }
            }
            Step::AllSet => {
                let team = self.flow.flow().template_name();
                let connected: Vec<String> = snap
                    .detection
                    .cli_agents
                    .iter()
                    .filter(|c| c.logged_in)
                    .map(|c| c.label.clone())
                    .chain(self.added.iter().map(|p| p.name.clone()))
                    .collect();
                let connected = if connected.is_empty() {
                    "Nothing connected yet. You can add providers later in Settings.".to_string()
                } else {
                    format!("Connected: {}", connected.join(", "))
                };
                body = body
                    .child(div().typeset(TextRole::Body, cx).child(format!(
                        "{} will lead your {team} team. Tell it what to look into once you are in.",
                        snap.lead_name
                    )))
                    .child(
                        div()
                            .typeset(TextRole::Caption, cx)
                            .text_color(theme.colors.muted_foreground)
                            .child(connected),
                    );
            }
        }
        let slide = if self.forward { 24.0 } else { -24.0 };
        col = col.child(
            self.reveal
                .enter(body, format!("ob-body-{n}"), 1, (slide, 0.0)),
        );
        if !self.status.is_empty() {
            col = col.child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(theme.colors.muted_foreground)
                    .child(self.status.clone()),
            );
        }
        col.child(
            h_flex()
                .gap_2()
                .pt_2()
                .child(
                    Button::new("ob-back")
                        .label("Back (Esc)")
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| this.do_back(cx))),
                )
                .child(
                    Button::new("ob-skip")
                        .label("Skip")
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| this.do_skip(cx))),
                )
                .child(div().flex_1())
                .child(if can_continue {
                    Button::new("ob-continue")
                        .primary()
                        .label(continue_label)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.do_continue(window, cx);
                        }))
                } else {
                    Button::new("ob-continue-disabled")
                        .outline()
                        .label(continue_label)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.status = this
                                .flow
                                .flow()
                                .blocked_hint()
                                .unwrap_or("Not yet — see above.")
                                .to_string();
                            cx.notify();
                        }))
                }),
        )
    }
}
