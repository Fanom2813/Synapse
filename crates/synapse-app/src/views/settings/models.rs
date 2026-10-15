//! Settings → Models: your AI providers (keys, default model) and the coding
//! CLIs on this computer (default model). Agents that pick a provider's or a
//! CLI's default follow what is set here; which engine each agent uses is set
//! under Settings → Agents.

use super::setting_row::card;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use crate::views::shell::engine_dialog::EngineTarget;
use crate::views::shell::ShellView;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _};
use gpui_kit::{
    div, AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};

/// A provider that is switched on.
struct ProviderEntry {
    id: String,
    name: String,
    has_key: bool,
    default_model: Option<String>,
}

/// A coding CLI found on this computer.
struct CliRow {
    id: String,
    label: String,
    logged_in: bool,
    default_model: Option<String>,
}

/// What the page shows. Providers are read on a worker thread (the keychain
/// answers the key question); CLIs are found by detection, once.
pub(crate) struct ModelsPage {
    providers: Vec<ProviderEntry>,
    /// `None` while detection is still running.
    clis: Option<Vec<CliRow>>,
}

fn load_providers() -> Option<Vec<ProviderEntry>> {
    let store = crate::window::open_store()?;
    Some(
        crate::provider_setup::list_added(&store)
            .into_iter()
            .map(|p| ProviderEntry {
                has_key: crate::provider_setup::key_saved(&store, &p.id),
                default_model: crate::model_defaults::provider_default(&store, &p.id),
                id: p.id,
                name: p.name,
            })
            .collect(),
    )
}

fn cli_rows(found: Vec<crate::onboarding::CliEntry>) -> Vec<CliRow> {
    let store = crate::window::open_store();
    found
        .into_iter()
        .map(|c| CliRow {
            default_model: store.as_ref().and_then(|s| crate::model_defaults::cli_default(s, &c.id)),
            id: c.id,
            label: c.label,
            logged_in: c.logged_in,
        })
        .collect()
}

impl ShellView {
    /// Rebuild the chat's backend from the saved key and Lead model, so a
    /// changed key or model applies to the next message without a restart.
    /// The keychain read runs off the UI thread.
    pub(crate) fn refresh_chat_backend(&mut self, cx: &mut Context<Self>) {
        crate::team_backends::forget_keys();
        cx.spawn(async move |this, cx| {
            let ((backend, model, persist), small) = cx
                .background_executor()
                .spawn(async { (crate::window::chat_config(), crate::window::small_config()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.chat.update(cx, |chat, cx| {
                    chat.reconfigure(backend, model, persist);
                    chat.set_small(small);
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// Ask a provider for its live model list, then redraw. Runs off the UI
    /// thread (keychain read plus a network call).
    pub(crate) fn refresh_provider_models(&mut self, id: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let name = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::model_discovery::refresh(&id) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.models_note = Some(match result {
                    Ok(n) => format!("{name} serves {n} models. The pickers now show them."),
                    Err(e) => format!("Could not update {name}'s models: {e}"),
                });
                this.reload_models_page(cx);
                this.refresh_chat_backend(cx);
            });
        })
        .detach();
    }

    /// Re-read the page in the background; detection of CLIs runs once.
    pub(crate) fn reload_models_page(&mut self, cx: &mut Context<Self>) {
        let detect = self.models_page.as_ref().map_or(true, |p| p.clis.is_none());
        cx.spawn(async move |this, cx| {
            let providers = cx.background_executor().spawn(async { load_providers() }).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(providers) = providers {
                    let old = this.models_page.take().and_then(|p| p.clis);
                    // Defaults of known CLIs may have changed: re-read them.
                    let clis = old.map(|rows| {
                        let store = crate::window::open_store();
                        rows.into_iter()
                            .map(|mut r| {
                                r.default_model = store
                                    .as_ref()
                                    .and_then(|s| crate::model_defaults::cli_default(s, &r.id));
                                r
                            })
                            .collect()
                    });
                    this.models_page = Some(ModelsPage { providers, clis });
                    cx.notify();
                }
            });
        })
        .detach();
        if detect {
            cx.spawn(async move |this, cx| {
                let found = cx
                    .background_executor()
                    .spawn(async { crate::window::onboarding_detection_probe().cli_agents })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(page) = this.models_page.as_mut() {
                        page.clis = Some(cli_rows(found));
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    fn remove_model_provider(&mut self, id: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .spawn(async move {
                    if let Some(store) = crate::window::open_store() {
                        let _ = crate::provider_setup::remove_provider(
                            &store,
                            &synapse_llm::keys::KeychainSecretStore,
                            &id,
                        );
                        let _ = crate::team_setup::reapply_all(&store);
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.reload_models_page(cx);
                this.refresh_chat_backend(cx);
            });
        })
        .detach();
    }
}

pub(super) fn page(view: &ShellView, cx: &mut Context<ShellView>) -> AnyElement {
    let t = cx.theme().clone();
    let Some(page) = &view.models_page else {
        return div()
            .typeset(TextRole::Body, cx)
            .text_color(t.colors.muted_foreground)
            .child("The settings database is unavailable.")
            .into_any_element();
    };
    let caption = |text: &'static str, cx: &Context<ShellView>| {
        div()
            .typeset(TextRole::Caption, cx)
            .text_color(cx.theme().colors.muted_foreground)
            .child(text)
    };
    let heading = |text: &'static str, cx: &Context<ShellView>| {
        div()
            .typeset(TextRole::Overline, cx)
            .text_color(cx.theme().colors.muted_foreground)
            .child(text)
    };

    let mut providers = card(&t);
    let hover = t.colors.list_hover;
    for (n, p) in page.providers.iter().enumerate() {
        let (pick_id, key_id, remove_id, sync_id) =
            (p.id.clone(), p.id.clone(), p.id.clone(), p.id.clone());
        let (key_name, has_key) = (p.name.clone(), p.has_key);
        let model = p.default_model.clone().unwrap_or_else(|| "no model".into());
        providers = providers.child(
            h_flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(if n + 1 < page.providers.len() { t.colors.border } else { t.transparent })
                .child(
                    h_flex()
                        .id(("provider-row", n))
                        .flex_1()
                        .items_center()
                        .gap_3()
                        .px_2()
                        .py_2()
                        .rounded(t.radius)
                        .cursor_pointer()
                        .hover(move |r| r.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_engine_dialog(
                                EngineTarget::ProviderDefault(pick_id.clone()),
                                Vec::new(),
                                window,
                                cx,
                            )
                        }))
                        .child(crate::cli_logo::tile(&p.id, &p.name, 32.0, cx))
                        .child(
                            v_flex()
                                .flex_1()
                                .child(div().typeset(TextRole::Label, cx).child(p.name.clone()))
                                .child(
                                    div()
                                        .typeset(TextRole::Caption, cx)
                                        .text_color(if p.has_key { t.colors.success } else { t.colors.warning })
                                        .child(format!(
                                            "{} · default: {model}",
                                            if p.has_key { "Key saved" } else { "No key saved" }
                                        )),
                                ),
                        ),
                )
                .child(
                    Button::new(("provider-sync", n))
                        .ghost()
                        .small()
                        .label("Refresh")
                        .tooltip("Fetch the models this provider offers now")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.refresh_provider_models(sync_id.clone(), cx)
                        })),
                )
                .child(
                    Button::new(("provider-key", n))
                        .ghost()
                        .small()
                        .label(if p.has_key { "Change key" } else { "Add key" })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_key_dialog(key_id.clone(), key_name.clone(), has_key, window, cx)
                        })),
                )
                .child(
                    Button::new(("provider-remove", n))
                        .ghost()
                        .small()
                        .icon(crate::icons::REMOVE)
                        .tooltip("Remove provider")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_model_provider(remove_id.clone(), cx)
                        })),
                ),
        );
    }
    if page.providers.is_empty() {
        providers = providers.child(
            div()
                .px_4()
                .py_3()
                .typeset(TextRole::Body, cx)
                .text_color(t.colors.muted_foreground)
                .child("No providers yet. Add one to use its models."),
        );
    }

    let small = crate::window::open_store()
        .and_then(|s| crate::small_model::get(&s))
        .map_or_else(|| "Not set · the Lead model does these jobs".to_string(), |(p, m)| format!("{m} · {p}"));
    let small_row = h_flex()
        .id("small-model-row")
        .items_center()
        .gap_3()
        .px_4()
        .py_3()
        .cursor_pointer()
        .hover(move |r| r.bg(hover))
        .on_click(cx.listener(|this, _, window, cx| {
            this.open_engine_dialog(EngineTarget::Small, Vec::new(), window, cx)
        }))
        .child(
            v_flex()
                .flex_1()
                .child(div().typeset(TextRole::Label, cx).child("Small model"))
                .child(
                    div()
                        .typeset(TextRole::Caption, cx)
                        .text_color(t.colors.muted_foreground)
                        .child(small),
                ),
        );

    let mut clis = card(&t);
    match &page.clis {
        None => {
            clis = clis.child(
                div().px_4().py_3().typeset(TextRole::Body, cx).text_color(t.colors.muted_foreground).child("Looking for coding CLIs..."),
            );
        }
        Some(found) if found.is_empty() => {
            clis = clis.child(
                div().px_4().py_3().typeset(TextRole::Body, cx).text_color(t.colors.muted_foreground).child("No coding CLIs found on this computer."),
            );
        }
        Some(found) => {
            let list: Vec<(String, String)> = found.iter().map(|c| (c.id.clone(), c.label.clone())).collect();
            for (n, c) in found.iter().enumerate() {
                let (status, colour) = if c.logged_in {
                    ("Ready", t.colors.success)
                } else {
                    ("Log in needed", t.colors.warning)
                };
                let (id, list) = (c.id.clone(), list.clone());
                let model = c.default_model.clone().unwrap_or_else(|| "the CLI's own model".into());
                clis = clis.child(
                    h_flex()
                        .id(("cli-row", n))
                        .items_center()
                        .gap_3()
                        .px_4()
                        .py_3()
                        .border_b_1()
                        .border_color(if n + 1 < found.len() { t.colors.border } else { t.transparent })
                        .cursor_pointer()
                        .hover(move |r| r.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_engine_dialog(
                                EngineTarget::CliDefault(id.clone()),
                                list.clone(),
                                window,
                                cx,
                            )
                        }))
                        .child(crate::cli_logo::tile(&c.id, &c.label, 32.0, cx))
                        .child(
                            v_flex()
                                .flex_1()
                                .child(div().typeset(TextRole::Label, cx).child(c.label.clone()))
                                .child(
                                    div()
                                        .typeset(TextRole::Caption, cx)
                                        .text_color(colour)
                                        .child(format!("{status} · default: {model}")),
                                ),
                        ),
                );
            }
        }
    }

    v_flex()
        .gap_4()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(heading("PROVIDERS", cx))
                .child(
                    Button::new("models-add-provider")
                        .small()
                        .outline()
                        .icon(crate::icons::PLUS)
                        .label("Add provider")
                        .on_click(cx.listener(|_, _, window, cx| {
                            let parent = cx.entity().downgrade();
                            crate::views::onboarding::provider_dialog::open_add_provider_dialog(
                                window,
                                cx,
                                std::rc::Rc::new(move |_, app| {
                                    let _ = parent.update(app, |view, cx| {
                                        view.reload_models_page(cx);
                                        view.refresh_chat_backend(cx);
                                    });
                                }),
                            );
                        })),
                ),
        )
        .children(view.models_note.clone().map(|note| {
            div()
                .typeset(TextRole::Caption, cx)
                .text_color(cx.theme().colors.muted_foreground)
                .child(note)
        }))
        .child(providers)
        .child(heading("SMALL MODEL", cx))
        .child(card(&t).child(small_row))
        .child(caption("A fast, cheap model for quick jobs like naming a new project from its first message.", cx))
        .child(heading("CODING CLIS", cx))
        .child(clis)
        .child(caption("Click a provider or CLI to choose its default model. Agents follow it unless they have a model of their own under Settings → Agents.", cx))
        .into_any_element()
}
