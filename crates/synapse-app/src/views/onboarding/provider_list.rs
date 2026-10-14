//! Providers on the Brains step: a small "Add custom" button and the list of
//! providers you have switched on, scrollable when there are many, each with
//! a remove control. Adding and removing touch the keychain, so they run on a
//! worker thread.

use super::detected::entry_row;
use super::provider_dialog::open_add_provider_dialog;
use super::OnboardingView;
use crate::cli_logo;
use crate::icons::Icon;
use crate::provider_setup::{remove_provider, AddedProvider};
use crate::theme::TextRole;
use crate::typography::Typography as _;
use crate::window::open_store;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _};
use gpui_kit::*;

/// Rows shown before the list scrolls.
const LIST_MAX_HEIGHT: f32 = 188.0;

impl OnboardingView {
    pub(super) fn render_providers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let header = h_flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(t.colors.muted_foreground)
                    .child("Providers"),
            )
            .child(
                Button::new("ob-add-provider")
                    .label("Add custom")
                    .icon(Icon::Plus)
                    .outline()
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.open_add_dialog(window, cx))),
            );
        let body = if self.added.is_empty() {
            div()
                .typeset(TextRole::Body, cx)
                .text_color(t.colors.muted_foreground)
                .child("None yet. Add one with an API key.")
                .into_any_element()
        } else {
            let mut list = v_flex().gap_2();
            for p in &self.added {
                let id = p.id.clone();
                let remove = Button::new(format!("ob-rm-{}", p.id))
                    .icon(Icon::X)
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_provider(&id, cx)));
                list = list.child(entry_row(
                    cli_logo::tile(&p.id, &p.name, 32.0, cx),
                    p.name.clone(),
                    remove,
                    cx,
                ));
            }
            div()
                .id("ob-providers")
                .max_h(px(LIST_MAX_HEIGHT))
                .overflow_y_scroll()
                .child(list)
                .into_any_element()
        };
        v_flex().gap_2().child(header).child(body)
    }

    pub(super) fn open_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.entity().downgrade();
        open_add_provider_dialog(
            window,
            cx,
            std::rc::Rc::new(move |added, app| {
                let _ = parent.update(app, |view, cx| view.provider_added(added, cx));
            }),
        );
    }

    /// Called by the popup after a provider is saved.
    pub(super) fn provider_added(&mut self, added: AddedProvider, cx: &mut Context<Self>) {
        if let Some(store) = open_store() {
            self.flow.set_api_result(&store, &added.id, true);
        }
        if !self.added.iter().any(|p| p.id == added.id) {
            self.added.push(added);
            self.added.sort_by_key(|p| p.name.to_lowercase());
        }
        cx.notify();
    }

    /// The row disappears at once; the keychain and settings follow on a worker.
    fn remove_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        self.added.retain(|p| p.id != id);
        if self.added.is_empty() {
            if let Some(store) = open_store() {
                self.flow.set_api_result(&store, "", false);
            }
        }
        cx.notify();
        let id = id.to_string();
        cx.background_executor()
            .spawn(async move {
                if let Some(store) = open_store() {
                    let _ = remove_provider(&store, &synapse_llm::keys::KeychainSecretStore, &id);
                }
            })
            .detach();
    }
}
