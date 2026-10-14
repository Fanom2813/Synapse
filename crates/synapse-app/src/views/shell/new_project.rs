//! The "New project" popup: a name, then Create. Enter creates too.

use super::ShellView;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, WindowExt as _};
use gpui_kit::{
    div, px, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, WeakEntity, Window,
};

impl ShellView {
    pub(super) fn open_new_project_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.entity().downgrade();
        let form = cx.new(|cx| NewProjectForm::new(parent, window, cx));
        window.open_dialog(cx, move |dialog, window, cx| {
            let top = ((window.viewport_size().height.as_f32() - 190.0) / 2.0).max(24.0);
            let t = cx.theme();
            dialog
                .title(div().typeset(TextRole::Title, cx).child("New project"))
                .w(px(400.0))
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

struct NewProjectForm {
    parent: WeakEntity<ShellView>,
    name: Entity<InputState>,
    note: Option<String>,
    _sub: Subscription,
}

impl NewProjectForm {
    fn new(parent: WeakEntity<ShellView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let sub = cx.subscribe_in(&name, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create(window, cx);
            }
        });
        name.update(cx, |input, cx| input.focus(window, cx));
        Self {
            parent,
            name,
            note: None,
            _sub: sub,
        }
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.name.read(cx).value().to_string();
        let made = crate::window::open_store()
            .ok_or_else(|| "The project database is unavailable.".to_string())
            .and_then(|store| crate::project_setup::create_project(&store, &text));
        match made {
            Ok(made) => {
                let _ = self
                    .parent
                    .update(cx, |view, cx| view.project_created(made, window, cx));
                window.close_dialog(cx);
            }
            Err(e) => {
                self.note = Some(e);
                cx.notify();
            }
        }
    }
}

impl Render for NewProjectForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut form = v_flex()
            .gap_3()
            .pt_3()
            .child(Input::new(&self.name));
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
                    Button::new("new-project-cancel")
                        .label("Cancel")
                        .ghost()
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("new-project-create")
                        .label("Create")
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
                ),
        )
    }
}
