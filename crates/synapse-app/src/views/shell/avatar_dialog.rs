//! The avatar popup, opened from the Team tab and from Settings → Agents.
//! Three ways to change how an agent looks: design the blob (shape, colour,
//! eyes, mouth), pick a ready-made style (with a role badge), or surprise me.

use super::ShellView;
use crate::avatar::{AvatarSpec, Eyes, Mouth, PALETTES, Shape};
use crate::avatar_packs::{all_badges, all_packs, PackAvatar};
use crate::avatar_view::avatar_img;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Selectable as _, Sizable as _, WindowExt as _,
};
use gpui_kit::{
    div, px, rgb, AppContext as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window,
};

impl ShellView {
    /// Open the avatar popup for `agent_id`; saving refreshes the Team tab
    /// and the Agents settings page.
    pub(crate) fn open_avatar_dialog(
        &mut self,
        agent_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = crate::window::open_store().and_then(|s| s.get_agent(agent_id).ok())
        else {
            return;
        };
        let parent = cx.entity().downgrade();
        let form = cx.new(|_| AvatarForm::new(parent, &agent));
        window.open_dialog(cx, move |dialog, window, cx| {
            let top = ((window.viewport_size().height.as_f32() - 560.0) / 2.0).max(24.0);
            let t = cx.theme();
            dialog
                .title(div().typeset(TextRole::Title, cx).child("Avatar"))
                .w(px(460.0))
                .margin_top(px(top))
                .entrance(DialogEntrance::FadeSlide)
                .bg(t.colors.popover)
                .border_1()
                .border_color(t.colors.border)
                .shadow_lg()
                .child(form.clone())
        });
    }

    /// An avatar was saved: redraw everything that shows agents.
    pub(crate) fn avatar_saved(&mut self, cx: &mut Context<Self>) {
        self.refresh_team(cx);
        self.change_engines(cx, |_| Ok(()));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Design,
    Style,
}

struct AvatarForm {
    parent: WeakEntity<ShellView>,
    agent_id: String,
    name: String,
    mode: Mode,
    blob: AvatarSpec,
    pack: PackAvatar,
    note: Option<String>,
}

fn clock_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

impl AvatarForm {
    fn new(parent: WeakEntity<ShellView>, agent: &synapse_store::Agent) -> Self {
        let style = PackAvatar::from_json(&agent.avatar);
        Self {
            parent,
            agent_id: agent.id.clone(),
            name: agent.name.clone(),
            mode: if style.is_some() { Mode::Style } else { Mode::Design },
            blob: AvatarSpec::from_json(&agent.avatar),
            pack: style
                .unwrap_or_else(|| PackAvatar::new("notionists", &agent.name).expect("known pack")),
            note: None,
        }
    }

    fn current_json(&self) -> String {
        match self.mode {
            Mode::Design => self.blob.to_json(),
            Mode::Style => self.pack.to_json(),
        }
    }

    fn surprise(&mut self) {
        let seed = clock_seed();
        match self.mode {
            Mode::Design => self.blob = AvatarSpec::surprise_me_from(seed),
            Mode::Style => {
                let packs = all_packs();
                if let Some(pack) = packs.get(seed as usize % packs.len().max(1)) {
                    self.pack.pack = pack.id.clone();
                }
                self.pack.seed = format!("{}-{seed}", self.name);
            }
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let json = self.current_json();
        let saved = crate::window::open_store()
            .ok_or_else(|| "The agent database is unavailable.".to_string())
            .and_then(|store| crate::team_setup::set_avatar(&store, &self.agent_id, &json));
        match saved {
            Ok(()) => {
                let _ = self.parent.update(cx, |view, cx| view.avatar_saved(cx));
                window.close_dialog(cx);
            }
            Err(e) => {
                self.note = Some(e);
                cx.notify();
            }
        }
    }
}

/// A small choice button that shows which option is picked.
fn choice(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
) -> Button {
    Button::new(id.into())
        .small()
        .ghost()
        .label(label.into())
        .selected(selected)
}

fn caption(text: &'static str, cx: &Context<AvatarForm>) -> impl IntoElement {
    div()
        .typeset(TextRole::Overline, cx)
        .text_color(cx.theme().colors.muted_foreground)
        .child(text)
}

impl AvatarForm {
    fn design_controls(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let t = cx.theme().clone();
        let mut shapes = h_flex().flex_wrap().gap_1();
        for (i, shape) in Shape::all().iter().copied().enumerate() {
            shapes = shapes.child(
                choice(format!("shape-{i}"), shape.as_str(), self.blob.shape == shape).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.blob.shape = shape;
                        cx.notify();
                    }),
                ),
            );
        }
        let mut colours = h_flex().flex_wrap().gap_2();
        for (i, palette) in PALETTES.iter().enumerate() {
            let selected = self.blob.custom.is_none() && self.blob.palette == i;
            let base = crate::theme::parse_hex(palette.base).unwrap_or(0x3fa34d);
            colours = colours.child(
                div()
                    .id(("palette", i))
                    .size(px(22.0))
                    .rounded_full()
                    .bg(rgb(base))
                    .border_2()
                    .border_color(if selected { t.colors.foreground } else { t.transparent })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.blob.palette = i;
                        this.blob.custom = None;
                        cx.notify();
                    })),
            );
        }
        let mut eyes = h_flex().flex_wrap().gap_1();
        for (i, eye) in Eyes::all().iter().copied().enumerate() {
            eyes = eyes.child(
                choice(format!("eyes-{i}"), eye.as_str(), self.blob.eyes == eye).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.blob.eyes = eye;
                        cx.notify();
                    }),
                ),
            );
        }
        let mut mouths = h_flex().flex_wrap().gap_1();
        for (i, mouth) in Mouth::all().iter().copied().enumerate() {
            mouths = mouths.child(
                choice(format!("mouth-{i}"), mouth.as_str(), self.blob.mouth == mouth).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.blob.mouth = mouth;
                        cx.notify();
                    }),
                ),
            );
        }
        v_flex()
            .gap_2()
            .child(caption("SHAPE", cx))
            .child(shapes)
            .child(caption("COLOUR", cx))
            .child(colours)
            .child(caption("EYES", cx))
            .child(eyes)
            .child(caption("MOUTH", cx))
            .child(mouths)
    }

    fn style_controls(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let mut packs = h_flex().flex_wrap().gap_1();
        for (i, pack) in all_packs().into_iter().enumerate() {
            let id = pack.id.clone();
            packs = packs.child(
                choice(format!("pack-{i}"), pack.title, self.pack.pack == id).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.pack.pack = id.clone();
                        cx.notify();
                    }),
                ),
            );
        }
        let mut badges = h_flex().flex_wrap().gap_1().child(
            choice("badge-none", "None", self.pack.badge.is_none()).on_click(cx.listener(
                |this, _, _, cx| {
                    this.pack.badge = None;
                    cx.notify();
                },
            )),
        );
        for (i, badge) in all_badges().into_iter().enumerate() {
            let id = badge.id.clone();
            badges = badges.child(
                choice(
                    format!("badge-{i}"),
                    format!("{} {}", badge.emoji, badge.title),
                    self.pack.badge.as_deref() == Some(id.as_str()),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pack.badge = Some(id.clone());
                    cx.notify();
                })),
            );
        }
        v_flex()
            .gap_2()
            .child(caption("STYLE", cx))
            .child(
                div()
                    .id("pack-scroll")
                    .max_h(px(140.0))
                    .overflow_y_scroll()
                    .child(packs),
            )
            .child(caption("BADGE", cx))
            .child(badges)
    }
}

impl Render for AvatarForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let controls = match self.mode {
            Mode::Design => self.design_controls(cx),
            Mode::Style => self.style_controls(cx),
        };
        let mut form = v_flex()
            .gap_3()
            .pt_3()
            .child(
                h_flex()
                    .items_center()
                    .gap_4()
                    .child(avatar_img(&self.current_json(), &self.name, 96.0))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        choice("mode-design", "Design", self.mode == Mode::Design)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.mode = Mode::Design;
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        choice("mode-style", "Style", self.mode == Mode::Style)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.mode = Mode::Style;
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                Button::new("avatar-surprise")
                                    .small()
                                    .outline()
                                    .label("Surprise me")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.surprise();
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(controls);
        if let Some(note) = &self.note {
            form = form.child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(t.colors.danger)
                    .child(note.clone()),
            );
        }
        form.child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("avatar-cancel")
                        .label("Cancel")
                        .ghost()
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("avatar-save")
                        .label("Save")
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
    }
}
