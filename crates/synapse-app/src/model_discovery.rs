//! Live model lists per provider. A list fetched from the provider is kept in
//! the settings and laid over the bundled catalog (`catalog_with_custom`), so
//! every picker shows what the provider serves today. Models the catalog
//! knows keep their prices and limits; new ones get generic capabilities.
//! With no saved list, or no network, the catalog list is used unchanged.

use serde::{Deserialize, Serialize};
use synapse_llm::catalog::CatalogModel;
use synapse_llm::Catalog;

fn key(provider_id: &str) -> String {
    format!("provider.models.{provider_id}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovered {
    pub fetched_at: i64,
    pub models: Vec<String>,
}

pub fn load(store: &synapse_store::Store, provider_id: &str) -> Option<Discovered> {
    let raw = store.setting_get(&key(provider_id)).ok().flatten()?;
    serde_json::from_str(&raw).ok().filter(|d: &Discovered| !d.models.is_empty())
}

pub fn save(store: &synapse_store::Store, provider_id: &str, models: Vec<String>) {
    let found = Discovered { fetched_at: synapse_llm::catalog::now_unix(), models };
    if let Ok(raw) = serde_json::to_string(&found) {
        let _ = store.setting_set(&key(provider_id), &raw);
    }
}

/// Replace each provider's model list with its saved live list.
pub fn apply(store: &synapse_store::Store, catalog: &mut Catalog) {
    for provider in catalog.providers.values_mut() {
        let Some(found) = load(store, &provider.id) else {
            continue;
        };
        let mut known = std::mem::take(&mut provider.models);
        provider.models = found
            .models
            .into_iter()
            .map(|id| {
                let model = known.remove(&id).unwrap_or_else(|| CatalogModel {
                    id: id.clone(),
                    name: id.clone(),
                    tool_call: true,
                    temperature: true,
                    ..CatalogModel::default()
                });
                (id, model)
            })
            .collect();
    }
}

/// Ask the provider for its models (key read from the keychain, so call this
/// off the UI thread) and save the list. Returns how many models it serves.
pub fn refresh(provider_id: &str) -> Result<usize, String> {
    use secrecy::ExposeSecret as _;
    use synapse_llm::keys::SecretStore as _;
    let store = crate::window::open_store().ok_or("The settings database is unavailable.")?;
    let catalog = crate::custom_providers::catalog_with_custom(Some(&store))
        .ok_or("The provider catalog could not be loaded.")?;
    let settings = crate::settings_providers::ProvidersSettings::load(&store)
        .map_err(|e| e.to_string())?;
    let base = settings.base_url_for(&catalog, provider_id);
    let key = if crate::provider_setup::key_saved(&store, provider_id) {
        synapse_llm::keys::KeychainSecretStore
            .get("provider", provider_id)
            .ok()
            .flatten()
            .map(|s| s.expose_secret().to_string())
    } else {
        None
    };
    let client = reqwest::Client::new();
    let id = provider_id.to_string();
    let models = crate::runtime::block_on(async move {
        synapse_llm::discovery::discover(&client, &id, base.as_deref(), key.as_deref()).await
    })
    .map_err(|e| e.to_string())?;
    if models.is_empty() {
        return Err("The provider listed no chat models.".into());
    }
    let count = models.len();
    save(&store, provider_id, models);
    repair_defaults(&store, provider_id);
    Ok(count)
}

/// A default that the provider no longer serves would 404 on every message:
/// move it to the best model on the live list.
fn repair_defaults(store: &synapse_store::Store, provider_id: &str) {
    use crate::settings_providers::{ModelRole, ProvidersSettings};
    let Some(catalog) = crate::custom_providers::catalog_with_custom(Some(store)) else {
        return;
    };
    let served = |model: &str| catalog.model(provider_id, model).is_ok();
    let Some(better) = catalog.suggested_model(provider_id).map(|m| m.id.clone()) else {
        return;
    };
    if let Ok(mut settings) = ProvidersSettings::load(store) {
        let stale = settings
            .default_for(ModelRole::Lead)
            .is_some_and(|(p, m)| p == provider_id && !served(&m));
        if stale && settings.set_default(&catalog, ModelRole::Lead, provider_id, &better).is_ok() {
            let _ = settings.save(store);
        }
    }
    let own = crate::model_defaults::provider_default(store, provider_id);
    if own.is_some_and(|m| !served(&m)) {
        let _ = crate::model_defaults::set_provider_default(store, provider_id, &better);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_list_replaces_the_catalog_models_and_keeps_known_details() {
        let store = synapse_store::Store::open_in_memory().expect("store");
        let mut catalog = Catalog::default();
        let mut provider = synapse_llm::catalog::Provider { id: "p".into(), ..Default::default() };
        provider.models.insert(
            "old".into(),
            CatalogModel { id: "old".into(), name: "Old".into(), ..Default::default() },
        );
        provider.models.insert(
            "kept".into(),
            CatalogModel { id: "kept".into(), name: "Kept".into(), ..Default::default() },
        );
        catalog.providers.insert("p".into(), provider);
        save(&store, "p", vec!["kept".into(), "fresh".into()]);
        apply(&store, &mut catalog);
        let models = &catalog.providers["p"].models;
        assert_eq!(models.len(), 2);
        assert_eq!(models["kept"].name, "Kept");
        assert!(models["fresh"].tool_call);
        assert!(!models.contains_key("old"));
    }
}
