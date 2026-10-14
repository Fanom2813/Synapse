//! The model each provider and each coding CLI runs by default. Set on the
//! Models page; agents that pick "provider default" or "CLI default" follow
//! it, and agents with a model of their own do not.

use synapse_store::Store;

fn provider_key(id: &str) -> String {
    format!("model.default.provider.{id}")
}

fn cli_key(id: &str) -> String {
    format!("model.default.cli.{id}")
}

fn get(store: &Store, key: &str) -> Option<String> {
    store.setting_get(key).ok().flatten().filter(|m| !m.is_empty())
}

/// The model chosen for a provider, else the catalog's suggestion for it.
pub fn provider_default(store: &Store, provider_id: &str) -> Option<String> {
    get(store, &provider_key(provider_id)).or_else(|| {
        let catalog = crate::custom_providers::catalog_with_custom(Some(store))?;
        let suggested = catalog.suggested_model(provider_id).map(|m| m.id.clone());
        suggested.or_else(|| {
            catalog
                .models(provider_id)
                .ok()
                .and_then(|m| m.first().map(|m| m.id.clone()))
        })
    })
}

/// The model chosen for a CLI; `None` lets the CLI use its own default.
pub fn cli_default(store: &Store, cli_id: &str) -> Option<String> {
    get(store, &cli_key(cli_id))
}

pub fn set_provider_default(store: &Store, provider_id: &str, model: &str) -> Result<(), String> {
    store
        .setting_set(&provider_key(provider_id), model)
        .map_err(|e| e.to_string())
}

/// `model` of `None` clears the choice (the CLI picks its own).
pub fn set_cli_default(store: &Store, cli_id: &str, model: Option<&str>) -> Result<(), String> {
    store
        .setting_set(&cli_key(cli_id), model.unwrap_or(""))
        .map_err(|e| e.to_string())
}
