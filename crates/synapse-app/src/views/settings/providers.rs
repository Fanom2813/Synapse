//! Settings → Providers: search the catalog and switch providers on or off.
//!
//! The catalog and key checks load once when the page first opens, never per
//! frame: listing uses an empty secret store, and only the few enabled
//! providers are asked the keychain whether they have a key.

use super::setting_row::card;
use crate::settings_providers::{search_providers, ProviderRow, ProvidersScreen};
use crate::theme::TextRole;
use crate::typography::Typography as _;
use crate::views::shell::ShellView;
use crate::window::open_store;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _};
use gpui_kit::{
    div, AnyElement, AppContext as _, Context, Entity, IntoElement as _, ParentElement as _,
    Styled as _, Subscription, Window,
};
use std::collections::HashSet;
use synapse_llm::keys::{KeychainSecretStore, MemorySecretStore, SecretStore as _};
use synapse_llm::Catalog;

/// Rows drawn at once; the search box narrows the rest.
const MAX_ROWS: usize = 40;

pub(crate) struct ProvidersPage {
    screen: ProvidersScreen,
    catalog: Option<Catalog>,
    search: Entity<InputState>,
    rows: Vec<ProviderRow>,
    with_key: HashSet<String>,
    _sub: Subscription,
}

impl ProvidersPage {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<ShellView>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search providers"));
        let sub = cx.subscribe_in(
            &search,
            window,
            |this: &mut ShellView, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    if let Some(page) = this.providers.as_mut() {
                        page.screen.set_query(query);
                        page.refresh_rows();
                    }
                    cx.notify();
                }
            },
        );
        let screen = match open_store() {
            Some(store) => ProvidersScreen::open(&store),
            None => ProvidersScreen::default(),
        };
        let mut page = Self {
            screen,
            catalog: crate::custom_providers::catalog_with_custom(open_store().as_ref()),
            search,
            rows: Vec::new(),
            with_key: HashSet::new(),
            _sub: sub,
        };
        page.refresh_keys();
        page.refresh_rows();
        page
    }

    fn refresh_rows(&mut self) {
        let Some(catalog) = &self.catalog else { return };
        self.rows = search_providers(
            catalog,
            self.screen.settings(),
            &MemorySecretStore::new(),
            self.screen.query(),
        );
    }

    /// Ask the keychain about enabled providers only.
    fn refresh_keys(&mut self) {
        let Some(catalog) = &self.catalog else { return };
        self.with_key = catalog
            .providers()
            .into_iter()
            .filter(|p| self.screen.settings().is_enabled(&p.id))
            .filter(|p| matches!(KeychainSecretStore.get("provider", &p.id), Ok(Some(_))))
            .map(|p| p.id.clone())
            .collect();
    }

    fn toggle(&mut self, id: &str, enabled: bool) {
        if let (Some(catalog), Some(store)) = (&self.catalog, open_store()) {
            let _ = self.screen.toggle_enabled(catalog, &store, id, enabled);
        }
        self.refresh_keys();
        self.refresh_rows();
    }
}

pub(super) fn page(view: &ShellView, cx: &mut Context<ShellView>) -> AnyElement {
    let t = cx.theme().clone();
    let Some(page) = view.providers.as_ref() else {
        return div().into_any_element();
    };
    if page.catalog.is_none() {
        return div()
            .text_color(t.colors.muted_foreground)
            .child("The provider catalog could not be loaded.")
            .into_any_element();
    }
    let mut list = card(&t);
    for (i, row) in page.rows.iter().take(MAX_ROWS).enumerate() {
        let id = row.id.clone();
        let key_note = if !row.enabled {
            String::new()
        } else if page.with_key.contains(&row.id) {
            "Key saved".to_string()
        } else {
            "No key yet".to_string()
        };
        list = list.child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_4()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(t.colors.border)
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().typeset(TextRole::Label, cx).child(row.name.clone()))
                        .child(
                            div()
                                .typeset(TextRole::Caption, cx)
                                .figures()
                                .text_color(t.colors.muted_foreground)
                                .child(format!("{} models {}", row.model_count, key_note)),
                        ),
                )
                .child(
                    Switch::new(("provider-enabled", i))
                        .checked(row.enabled)
                        .on_click(cx.listener(move |this, on: &bool, _, cx| {
                            if let Some(p) = this.providers.as_mut() {
                                p.toggle(&id, *on);
                            }
                            cx.notify();
                        })),
                ),
        );
    }
    let hidden = page.rows.len().saturating_sub(MAX_ROWS);
    v_flex()
        .gap_3()
        .child(Input::new(&page.search))
        .child(list)
        .child(
            div()
                .typeset(TextRole::Caption, cx)
                .text_color(t.colors.muted_foreground)
                .child(if hidden > 0 {
                    format!("{hidden} more: refine the search.")
                } else if let Some(status) = page.screen.status() {
                    status.to_string()
                } else {
                    String::new()
                }),
        )
        .into_any_element()
}
