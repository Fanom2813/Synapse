//! The "API key" popup for one provider: save a new key, replace the saved
//! one, or remove it. The key goes to the OS keychain on a worker thread and
//! is never shown again.

use super::ShellView;
use crate::settings_providers::ProvidersSettings;
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
use synapse_llm::keys::KeychainSecretStore;

impl ShellView {
    pub(crate) fn open_key_dialog(
        &mut self,
        provider_id: String,
        provider_name: String,
        has_key: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = cx.entity().downgrade();
        let title = format!("{provider_name} key");
        let form = cx.new(|cx| KeyForm::new(parent, provider_id, has_key, window, cx));
        window.open_dialog(cx, move |dialog, window, cx| {
            let top = ((window.viewport_size().height.as_f32() - 220.0) / 2.0).max(24.0);
            let t = cx.theme();
            dialog
                .title(div().typeset(TextRole::Title, cx).child(title.clone()))
                .w(px(420.0))
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

struct KeyForm {
    parent: WeakEntity<ShellView>,
    provider_id: String,
    has_key: bool,
    key: Entity<InputState>,
    busy: bool,
    note: Option<String>,
    _sub: Subscription,
}

impl KeyForm {
    fn new(
        parent: WeakEntity<ShellView>,
        provider_id: String,
        has_key: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(if has_key { "Paste a new key to replace it" } else { "Paste the key" })
                .masked(true)
        });
        let sub = cx.subscribe_in(&key, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save(window, cx);
            }
        });
        key.update(cx, |input, cx| input.focus(window, cx));
        Self { parent, provider_id, has_key, key, busy: false, note: None, _sub: sub }
    }

    /// Run the keychain work off the UI thread, then close and refresh.
    fn run(
        &mut self,
        work: impl FnOnce(&str) -> Result<(), String> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.note = None;
        cx.notify();
        let id = self.provider_id.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { work(&id) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        let _ = this.parent.update(cx, |view, cx| {
                            view.reload_models_page(cx);
                            view.refresh_chat_backend(cx);
                        });
                        window.close_dialog(cx);
                    }
                    Err(e) => {
                        this.note = Some(e);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key.read(cx).value().trim().to_string();
        if key.is_empty() {
            self.note = Some("Paste a key first.".into());
            cx.notify();
            return;
        }
        self.run(
            move |id| {
                ProvidersSettings::default()
                    .set_key(&KeychainSecretStore, id, &key)
                    .map_err(|e| e.to_string())?;
                if let Some(store) = crate::window::open_store() {
                    crate::provider_setup::mark_key_saved(&store, id, true);
                }
                // A new key may unlock a different model list; failing here
                // (offline) leaves the saved list as it was.
                let _ = crate::model_discovery::refresh(id);
                Ok(())
            },
            window,
            cx,
        );
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(
            |id| {
                ProvidersSettings::default()
                    .remove_key(&KeychainSecretStore, id)
                    .map_err(|e| e.to_string())?;
                if let Some(store) = crate::window::open_store() {
                    crate::provider_setup::mark_key_saved(&store, id, false);
                }
                Ok(())
            },
            window,
            cx,
        );
    }
}

impl Render for KeyForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut form = v_flex().gap_3().pt_3().child(Input::new(&self.key));
        if let Some(note) = &self.note {
            form = form.child(
                div().typeset(TextRole::Caption, cx).text_color(t.colors.danger).child(note.clone()),
            );
        }
        let mut buttons = h_flex().justify_end().gap_2();
        if self.has_key {
            buttons = buttons.child(
                Button::new("key-remove")
                    .label("Remove key")
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))),
            );
        }
        form.child(
            buttons
                .child(
                    Button::new("key-cancel")
                        .label("Cancel")
                        .ghost()
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("key-save")
                        .label(if self.busy { "Saving..." } else { "Save" })
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
    }
}
