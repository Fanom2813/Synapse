//! The "Add agent" popup: search the persona library (the starter team plus
//! the open collections) and add one to the open project's team. What runs the
//! agent is set in Settings → Agents, not here.

use super::persona_list::{PersonaList, ROW_HEIGHT};
use super::ShellView;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, WindowExt as _};
use gpui_kit::{
    div, px, AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window,
};
use std::rc::Rc;
use synapse_personas::PersonaDraft;

impl ShellView {
    pub(super) fn open_add_agent_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.shell.open_project_id.clone() else {
            return;
        };
        let parent = cx.entity().downgrade();
        let form = cx.new(|cx| AddAgentForm::new(parent, project, window, cx));
        window.open_dialog(cx, move |dialog, window, cx| {
            let top = ((window.viewport_size().height.as_f32() - 520.0) / 2.0).max(24.0);
            let t = cx.theme();
            dialog
                .title(div().typeset(TextRole::Title, cx).child("Add agent"))
                .w(px(480.0))
                .margin_top(px(top))
                .entrance(DialogEntrance::FadeSlide)
                .bg(t.colors.popover)
                .border_1()
                .border_color(t.colors.border)
                .shadow_lg()
                .child(form.clone())
        });
    }
}

struct AddAgentForm {
    parent: WeakEntity<ShellView>,
    project: String,
    list: Entity<PersonaList>,
    note: Option<String>,
}

impl AddAgentForm {
    fn new(
        parent: WeakEntity<ShellView>,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let form = cx.weak_entity();
        let row = Rc::new(move |index: usize, persona: &PersonaDraft, app: &App| {
            persona_row(index, persona, form.clone(), app)
        });
        let list = cx.new(|cx| PersonaList::new(360.0, row, window, cx));
        list.update(cx, |list, cx| list.focus_search(window, cx));
        Self { parent, project, list, note: None }
    }

    fn add(&mut self, persona: &PersonaDraft, window: &mut Window, cx: &mut Context<Self>) {
        let result = crate::window::open_store()
            .ok_or_else(|| "The project database is unavailable.".to_string())
            .and_then(|store| crate::team_setup::add_persona(&store, &self.project, persona));
        match result {
            Ok(_) => {
                let _ = self.parent.update(cx, |view, cx| view.refresh_team(cx));
                window.close_dialog(cx);
            }
            Err(e) => {
                self.note = Some(e);
                cx.notify();
            }
        }
    }
}

impl Render for AddAgentForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut form = v_flex().gap_3().pt_3().child(self.list.clone());
        if let Some(note) = &self.note {
            form = form.child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(t.colors.danger)
                    .child(note.clone()),
            );
        }
        form.child(
            h_flex().justify_end().child(
                Button::new("add-agent-cancel")
                    .label("Cancel")
                    .ghost()
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            ),
        )
    }
}

fn persona_row(
    index: usize,
    persona: &PersonaDraft,
    form: WeakEntity<AddAgentForm>,
    app: &App,
) -> AnyElement {
    let t = app.theme().clone();
    let hover = t.colors.list_hover;
    let blurb = persona.vibe.clone().unwrap_or_else(|| persona.description.clone());
    let picked = persona.clone();
    h_flex()
        .id(("persona-row", index))
        .w_full()
        .min_w_0()
        .h(px(ROW_HEIGHT))
        .items_center()
        .gap_3()
        .px_3()
        .rounded(t.radius)
        .cursor_pointer()
        .hover(move |row| row.bg(hover))
        .on_click(move |_, window, app| {
            let _ = form.update(app, |this, cx| this.add(&picked, window, cx));
        })
        .child(
            div()
                .w(px(24.0))
                .flex_none()
                .child(persona.emoji.clone().unwrap_or_else(|| "•".into())),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().typeset(TextRole::Label, app).child(persona.name.clone()))
                        .child(
                            div()
                                .typeset(TextRole::Caption, app)
                                .text_color(t.colors.muted_foreground)
                                .child(persona.category.clone().unwrap_or_default()),
                        ),
                )
                .child(
                    div()
                        .typeset(TextRole::Caption, app)
                        .text_color(t.colors.muted_foreground)
                        .truncate()
                        .child(blurb),
                ),
        )
        .into_any_element()
}
