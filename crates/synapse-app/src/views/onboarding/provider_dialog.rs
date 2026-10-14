//! The "Add a provider" popup: pick a provider, paste its key, then Add, Add
//! more (keeps the popup open for the next one) or Close. "Custom endpoint…"
//! at the top of the list reveals name, address and models for any
//! OpenAI-compatible server. The form is its own view so it redraws itself;
//! saving runs on a worker thread because it touches the keychain.

use crate::custom_providers::{embedded_catalog, parse_models};
use crate::provider_setup::{add_provider, AddedProvider, NewProvider};
use crate::theme::TextRole;
use crate::typography::Typography as _;
use crate::window::open_store;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, WindowExt as _};
use gpui_kit::*;
use std::rc::Rc;
use std::sync::OnceLock;

/// Called with each provider the popup saves.
pub(crate) type OnAdded = Rc<dyn Fn(AddedProvider, &mut App)>;

/// Catalog ids listed first; the rest follow A to Z.
const POPULAR: [&str; 3] = ["anthropic", "openai", "openrouter"];
/// Value of the entry that switches the form to a custom endpoint.
const CUSTOM: &str = "\u{0}custom";

/// One row of the provider list.
#[derive(Clone)]
struct ProviderChoice {
    id: String,
    name: SharedString,
    /// Lower-cased once so a keystroke compares without allocating per row.
    lower: String,
}

impl ProviderChoice {
    fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string().into(),
            lower: name.to_lowercase(),
        }
    }
}

impl SelectItem for ProviderChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &String {
        &self.id
    }

    /// The custom entry stays visible while searching.
    fn matches(&self, query: &str) -> bool {
        self.id == CUSTOM || self.lower.contains(&query.to_lowercase())
    }
}

type ProviderSelect = SelectState<SearchableVec<ProviderChoice>>;
type ProviderEvent = SelectEvent<SearchableVec<ProviderChoice>>;

/// Built once per process from the embedded catalog; opening the popup only
/// copies the rows.
fn provider_items() -> SearchableVec<ProviderChoice> {
    static ROWS: OnceLock<Vec<ProviderChoice>> = OnceLock::new();
    let rows = ROWS.get_or_init(|| {
        let mut rows = vec![ProviderChoice::new(CUSTOM, "Custom endpoint…")];
        if let Some(catalog) = embedded_catalog() {
            let mut rest = catalog.providers();
            rest.sort_by_key(|p| p.name.to_lowercase());
            for id in POPULAR {
                if let Some(p) = rest.iter().find(|p| p.id == id) {
                    rows.push(ProviderChoice::new(&p.id, &p.name));
                }
            }
            rows.extend(
                rest.iter()
                    .filter(|p| !POPULAR.contains(&p.id.as_str()))
                    .map(|p| ProviderChoice::new(&p.id, &p.name)),
            );
        }
        rows
    });
    SearchableVec::new(rows.clone())
}

/// A small label above a form control.
fn field(label: &str, control: impl IntoElement, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .typeset(TextRole::Caption, cx)
                .text_color(cx.theme().colors.muted_foreground)
                .child(label.to_string()),
        )
        .child(control)
}

pub(crate) struct AddProviderForm {
    select: Entity<ProviderSelect>,
    key: Entity<InputState>,
    name: Entity<InputState>,
    url: Entity<InputState>,
    models: Entity<InputState>,
    custom: bool,
    busy: bool,
    /// Last result line: text and whether it is good news.
    note: Option<(String, bool)>,
    on_added: OnAdded,
    _sub: Subscription,
}

impl AddProviderForm {
    pub(crate) fn new(
        on_added: OnAdded,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let select =
            cx.new(|cx| ProviderSelect::new(provider_items(), None, window, cx).searchable(true));
        let sub = cx.subscribe_in(&select, window, Self::on_select);
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Paste the key")
                .masked(true)
        });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("My gateway"));
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("http://localhost:1234/v1"));
        let models = cx.new(|cx| InputState::new(window, cx).placeholder("gpt-4o-mini, llama3"));
        Self {
            select,
            key,
            name,
            url,
            models,
            custom: false,
            busy: false,
            note: None,
            on_added,
            _sub: sub,
        }
    }

    fn on_select(
        &mut self,
        _: &Entity<ProviderSelect>,
        event: &ProviderEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SelectEvent::Confirm(Some(id)) = event {
            self.custom = id == CUSTOM;
            self.note = None;
            cx.notify();
        }
    }

    fn request(&self, cx: &App) -> NewProvider {
        let key = self.key.read(cx).value().trim().to_string();
        if self.custom {
            NewProvider::Custom {
                name: self.name.read(cx).value().to_string(),
                url: self.url.read(cx).value().to_string(),
                models: parse_models(self.models.read(cx).value().as_ref()),
                key,
            }
        } else {
            NewProvider::Catalog {
                id: self
                    .select
                    .read(cx)
                    .selected_value()
                    .cloned()
                    .unwrap_or_default(),
                key,
            }
        }
    }

    /// Save on a worker thread (keychain and database), then report back.
    fn submit(&mut self, keep_open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let request = self.request(cx);
        self.busy = true;
        self.note = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let store = open_store()
                        .ok_or_else(|| anyhow::anyhow!("The settings database is unavailable."))?;
                    add_provider(&store, &synapse_llm::keys::KeychainSecretStore, request)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finished(result, keep_open, window, cx)
            });
        })
        .detach();
    }

    fn finished(
        &mut self,
        result: anyhow::Result<AddedProvider>,
        keep_open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        match result {
            Ok(added) => {
                let name = added.name.clone();
                (self.on_added)(added, cx);
                if keep_open {
                    for input in [&self.key, &self.name, &self.url, &self.models] {
                        input.update(cx, |i, cx| i.set_value("", window, cx));
                    }
                    self.select
                        .update(cx, |s, cx| s.set_selected_index(None, window, cx));
                    self.custom = false;
                    self.note = Some((format!("{name} added ✓"), true));
                } else {
                    window.close_dialog(cx);
                }
            }
            Err(e) => self.note = Some((e.to_string(), false)),
        }
        cx.notify();
    }
}

impl Render for AddProviderForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut form = v_flex().gap_4().pt_3().child(field(
            "Provider",
            Select::new(&self.select)
                .placeholder("Choose a provider")
                .search_placeholder("Search providers")
                .menu_max_h(px(240.0)),
            cx,
        ));
        if self.custom {
            form = form
                .child(field("Name", Input::new(&self.name), cx))
                .child(field("Address", Input::new(&self.url), cx))
                .child(field("Models", Input::new(&self.models), cx));
        }
        let key_label = if self.custom {
            "API key (optional for local servers)"
        } else {
            "API key"
        };
        form = form.child(field(key_label, Input::new(&self.key), cx));
        if let Some((text, ok)) = &self.note {
            form = form.child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(if *ok {
                        t.colors.success
                    } else {
                        t.colors.danger
                    })
                    .child(text.clone()),
            );
        }
        let add_label = if self.busy { "Adding…" } else { "Add" };
        form.child(
            h_flex()
                .justify_end()
                .gap_2()
                .pt_1()
                .child(
                    Button::new("provider-close")
                        .label("Close")
                        .ghost()
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("provider-add-more")
                        .label("Add more")
                        .secondary()
                        .on_click(cx.listener(|this, _, window, cx| this.submit(true, window, cx))),
                )
                .child(
                    Button::new("provider-add")
                        .label(add_label)
                        .primary()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.submit(false, window, cx)),
                        ),
                ),
        )
    }
}

/// Open the "Add a provider" popup; `on_added` hears about each provider saved.
pub(crate) fn open_add_provider_dialog(window: &mut Window, cx: &mut App, on_added: OnAdded) {
    let form = cx.new(|cx| AddProviderForm::new(on_added, window, cx));
    window.open_dialog(cx, move |dialog, window, cx| {
        // Centred vertically for the short form (about 300px tall).
        let top = ((window.viewport_size().height.as_f32() - 300.0) / 2.0).max(24.0);
        let t = cx.theme();
        dialog
            .title(div().typeset(TextRole::Title, cx).child("Add a provider"))
            .w(px(440.0))
            .margin_top(px(top))
            .entrance(gpui_kit::component::dialog::DialogEntrance::FadeSlide)
            .bg(t.colors.popover)
            .border_1()
            .border_color(t.colors.border)
            .shadow_lg()
            .child(form.clone())
    });
}
