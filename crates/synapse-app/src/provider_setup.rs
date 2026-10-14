//! Connect and disconnect providers: save the key, turn the provider on and
//! make sure the lead has a model to talk to. Toolkit-free so it is testable;
//! the onboarding dialog and (later) Settings both call it.

use crate::custom_providers::{catalog_with_custom, CustomProviders};
use crate::settings_providers::{ModelRole, ProvidersSettings};
use anyhow::{anyhow, Context as _, Result};
use synapse_llm::keys::SecretStore;

fn key_marker(id: &str) -> String {
    format!("provider.key_saved.{id}")
}

/// Whether a key is saved for the provider, answered from a marker in the
/// settings instead of reading the keychain (every keychain read can raise
/// the OS access prompt). Providers from before the marker count as saved.
pub fn key_saved(store: &synapse_store::Store, id: &str) -> bool {
    store.setting_get(&key_marker(id)).ok().flatten().map_or(true, |v| v == "1")
}

/// Record that a key was saved or removed (call next to the keychain write).
pub fn mark_key_saved(store: &synapse_store::Store, id: &str, saved: bool) {
    let _ = store.setting_set(&key_marker(id), if saved { "1" } else { "0" });
}

/// A provider that is switched on, as the onboarding list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedProvider {
    pub id: String,
    pub name: String,
}

/// What the user typed in the dialog.
pub enum NewProvider {
    /// A catalog provider: needs a key.
    Catalog { id: String, key: String },
    /// Any OpenAI-compatible endpoint; the key is optional (local servers).
    Custom {
        name: String,
        url: String,
        models: Vec<String>,
        key: String,
    },
}

/// Save a provider and its key, switch it on, and give the lead a default
/// model if it has none yet (an existing choice is never overridden).
pub fn add_provider(
    store: &synapse_store::Store,
    secrets: &dyn SecretStore,
    new: NewProvider,
) -> Result<AddedProvider> {
    let mut custom = CustomProviders::load(store);
    let (id, key, model, is_custom) = match &new {
        NewProvider::Catalog { id, key } => {
            if id.is_empty() {
                return Err(anyhow!("Choose a provider first."));
            }
            if key.trim().is_empty() {
                return Err(anyhow!("Paste the API key."));
            }
            let catalog = catalog_with_custom(Some(store))
                .ok_or_else(|| anyhow!("The provider catalog could not be loaded."))?;
            let model = catalog.suggested_model(id).map(|m| m.id.clone()).ok_or_else(|| {
                anyhow!("That provider lists no models that can use tools. Add it as a custom endpoint instead.")
            })?;
            (id.clone(), key.trim().to_string(), model, false)
        }
        NewProvider::Custom {
            name,
            url,
            models,
            key,
        } => {
            let id = custom.add(name, url, models)?;
            (id, key.trim().to_string(), models[0].clone(), true)
        }
    };
    if !key.is_empty() {
        secrets
            .set("provider", &id, &key)
            .map_err(|_| anyhow!("Could not save that key (keychain unavailable)."))?;
    }
    mark_key_saved(store, &id, !key.is_empty());
    if is_custom {
        custom.save(store)?;
    }
    let catalog = catalog_with_custom(Some(store))
        .ok_or_else(|| anyhow!("The provider catalog could not be loaded."))?;
    let mut settings = ProvidersSettings::load(store).context("loading provider settings")?;
    settings.set_enabled(&catalog, &id, true)?;
    if settings.default_for(ModelRole::Lead).is_none() {
        settings.set_default(&catalog, ModelRole::Lead, &id, &model)?;
    }
    settings.save(store)?;
    let name = catalog
        .provider(&id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|_| id.clone());
    Ok(AddedProvider { id, name })
}

/// Switch a provider off, delete its key, and forget it entirely when it was
/// a custom endpoint. A lead default pointing at it is cleared.
pub fn remove_provider(
    store: &synapse_store::Store,
    secrets: &dyn SecretStore,
    id: &str,
) -> Result<()> {
    let mut settings = ProvidersSettings::load(store).context("loading provider settings")?;
    if let Some(catalog) = catalog_with_custom(Some(store)) {
        let _ = settings.set_enabled(&catalog, id, false);
    }
    if settings
        .default_for(ModelRole::Lead)
        .is_some_and(|(provider, _)| provider == id)
    {
        settings.defaults.remove(ModelRole::Lead.as_str());
    }
    settings.save(store)?;
    let _ = secrets.delete("provider", id);
    mark_key_saved(store, id, false);
    if id.starts_with("custom-") {
        let mut custom = CustomProviders::load(store);
        if custom.remove(id) {
            custom.save(store)?;
        }
    }
    Ok(())
}

/// Providers that are switched on, A to Z.
pub fn list_added(store: &synapse_store::Store) -> Vec<AddedProvider> {
    let Some(catalog) = catalog_with_custom(Some(store)) else {
        return Vec::new();
    };
    let settings = ProvidersSettings::load(store).unwrap_or_default();
    let mut out: Vec<AddedProvider> = catalog
        .providers()
        .into_iter()
        .filter(|p| settings.is_enabled(&p.id))
        .map(|p| AddedProvider {
            id: p.id.clone(),
            name: p.name.clone(),
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use synapse_llm::keys::MemorySecretStore;

    fn store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().unwrap()
    }

    #[test]
    fn provider_setup_catalog_provider_needs_a_key_and_gets_a_default_model() {
        let (store, secrets) = (store(), MemorySecretStore::new());
        let blank = NewProvider::Catalog {
            id: "anthropic".into(),
            key: " ".into(),
        };
        assert!(add_provider(&store, &secrets, blank).is_err());

        let new = NewProvider::Catalog {
            id: "anthropic".into(),
            key: " sk-test ".into(),
        };
        let added = add_provider(&store, &secrets, new).unwrap();
        assert_eq!(added.id, "anthropic");
        assert_eq!(list_added(&store), vec![added]);
        assert!(secrets.get("provider", "anthropic").unwrap().is_some());
        let settings = ProvidersSettings::load(&store).unwrap();
        assert_eq!(
            settings.default_for(ModelRole::Lead).unwrap().0,
            "anthropic"
        );
    }

    #[test]
    fn provider_setup_keeps_an_existing_lead_default_and_removal_cleans_up() {
        let (store, secrets) = (store(), MemorySecretStore::new());
        let local = NewProvider::Custom {
            name: "Local".into(),
            url: "http://localhost:1234/v1".into(),
            models: vec!["llama3".to_string()],
            key: String::new(),
        };
        let local = add_provider(&store, &secrets, local).unwrap();
        assert_eq!(local.id, "custom-local");

        let key = NewProvider::Catalog {
            id: "openai".into(),
            key: "sk-x".into(),
        };
        add_provider(&store, &secrets, key).unwrap();
        let settings = ProvidersSettings::load(&store).unwrap();
        assert_eq!(
            settings.default_for(ModelRole::Lead).unwrap().0,
            "custom-local"
        );

        remove_provider(&store, &secrets, "custom-local").unwrap();
        remove_provider(&store, &secrets, "openai").unwrap();
        assert!(list_added(&store).is_empty());
        assert!(CustomProviders::load(&store).list().is_empty());
        assert!(secrets.get("provider", "openai").unwrap().is_none());
        let settings = ProvidersSettings::load(&store).unwrap();
        assert!(settings.default_for(ModelRole::Lead).is_none());
    }
}
