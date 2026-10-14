//! Providers the user adds themselves: any OpenAI-compatible endpoint (a
//! gateway, a local server, a company proxy). Unlimited, stored as data and
//! merged into the catalog at load time, so pickers, key checks and
//! `chat_config` treat them like any catalog provider.
//!
//! Same shape the common apps use (Zed `openai_compatible`, Cherry Studio,
//! LobeChat): name + base URL + key + model ids. Keys go to the keychain.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use synapse_llm::catalog::{CatalogModel, Provider};
use synapse_llm::Catalog;
use thiserror::Error;

const KEY_CUSTOM: &str = "providers.custom";
const ID_PREFIX: &str = "custom-";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CustomProviderError {
    #[error("Give the provider a name.")]
    NameRequired,
    #[error("The address must start with http:// or https://.")]
    BadUrl,
    #[error("Add at least one model id (for example gpt-4o-mini).")]
    NoModels,
    #[error("store: {0}")]
    Store(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomProvider {
    /// `custom-<slug>`: never collides with a catalog id.
    pub id: String,
    pub name: String,
    /// OpenAI-compatible base, usually ending in `/v1`.
    pub base_url: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomProviders {
    list: Vec<CustomProvider>,
}

impl CustomProviders {
    pub fn load(store: &synapse_store::Store) -> Self {
        let list = store
            .setting_get(KEY_CUSTOM)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Self { list }
    }

    pub fn save(&self, store: &synapse_store::Store) -> Result<(), CustomProviderError> {
        let raw = serde_json::to_string(&self.list)
            .map_err(|e| CustomProviderError::Store(e.to_string()))?;
        store
            .setting_set(KEY_CUSTOM, &raw)
            .map_err(|e| CustomProviderError::Store(e.to_string()))
    }

    pub fn list(&self) -> &[CustomProvider] {
        &self.list
    }

    /// Add a provider and return its id. `models` may come straight from a
    /// text box: see [`parse_models`].
    pub fn add(
        &mut self,
        name: &str,
        base_url: &str,
        models: &[String],
    ) -> Result<String, CustomProviderError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CustomProviderError::NameRequired);
        }
        let base_url = normalize_base_url(base_url)?;
        if models.is_empty() {
            return Err(CustomProviderError::NoModels);
        }
        let id = self.unique_id(name);
        self.list.push(CustomProvider {
            id: id.clone(),
            name: name.to_string(),
            base_url,
            models: models.to_vec(),
        });
        Ok(id)
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.list.len();
        self.list.retain(|p| p.id != id);
        self.list.len() != before
    }

    /// Merge into a catalog. Capabilities are not known for a custom model, so
    /// it is assumed to support tools and temperature like current
    /// OpenAI-compatible chat models; limits and cost stay empty.
    pub fn apply(&self, catalog: &mut Catalog) {
        for p in &self.list {
            let models = p
                .models
                .iter()
                .map(|m| {
                    let model = CatalogModel {
                        id: m.clone(),
                        name: m.clone(),
                        tool_call: true,
                        temperature: true,
                        ..CatalogModel::default()
                    };
                    (m.clone(), model)
                })
                .collect();
            catalog.providers.insert(
                p.id.clone(),
                Provider {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    api: Some(p.base_url.clone()),
                    models,
                    ..Provider::default()
                },
            );
        }
    }

    fn unique_id(&self, name: &str) -> String {
        let slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let base = format!(
            "{ID_PREFIX}{}",
            if slug.is_empty() { "provider" } else { &slug }
        );
        let mut id = base.clone();
        let mut n = 2;
        while self.list.iter().any(|p| p.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }
}

/// The embedded catalog, parsed once per process. Parsing it takes tens of
/// milliseconds in a release build (hundreds in debug), so it must never be
/// repeated on a click or a keystroke.
pub fn embedded_catalog() -> Option<&'static Catalog> {
    static EMBEDDED: OnceLock<Option<Catalog>> = OnceLock::new();
    EMBEDDED.get_or_init(|| Catalog::embedded().ok()).as_ref()
}

/// Embedded catalog plus the user's custom providers.
pub fn catalog_with_custom(store: Option<&synapse_store::Store>) -> Option<Catalog> {
    let mut catalog = embedded_catalog()?.clone();
    if let Some(store) = store {
        CustomProviders::load(store).apply(&mut catalog);
    }
    Some(catalog)
}

/// Model ids typed as a list: separated by commas, spaces or new lines.
pub fn parse_models(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|m| !m.is_empty())
    {
        if !out.iter().any(|x| x == m) {
            out.push(m.to_string());
        }
    }
    out
}

fn normalize_base_url(url: &str) -> Result<String, CustomProviderError> {
    let url = url.trim().trim_end_matches('/');
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or("");
    if host.is_empty() {
        return Err(CustomProviderError::BadUrl);
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models() -> Vec<String> {
        parse_models("gpt-4o-mini, llama3")
    }

    #[test]
    fn custom_provider_add_validates_and_ids_stay_unique() {
        let mut c = CustomProviders::default();
        assert_eq!(
            c.add(" ", "https://x.test/v1", &models()),
            Err(CustomProviderError::NameRequired)
        );
        assert_eq!(
            c.add("A", "x.test/v1", &models()),
            Err(CustomProviderError::BadUrl)
        );
        assert_eq!(
            c.add("A", "https://x.test/v1", &[]),
            Err(CustomProviderError::NoModels)
        );
        let a = c
            .add("My Gateway!", "https://x.test/v1/", &models())
            .unwrap();
        let b = c
            .add("My Gateway!", "http://localhost:1234/v1", &models())
            .unwrap();
        assert_eq!(a, "custom-my-gateway");
        assert_eq!(b, "custom-my-gateway-2");
        assert_eq!(c.list()[0].base_url, "https://x.test/v1");
        assert!(c.remove(&a) && !c.remove(&a));
    }

    #[test]
    fn custom_provider_persists_and_merges_into_catalog() {
        let store = synapse_store::Store::open_in_memory().unwrap();
        let mut c = CustomProviders::default();
        let id = c
            .add("Local", "http://localhost:1234/v1", &models())
            .unwrap();
        c.save(&store).unwrap();
        let catalog = catalog_with_custom(Some(&store)).unwrap();
        assert_eq!(
            catalog.base_url(&id).unwrap(),
            Some("http://localhost:1234/v1")
        );
        assert!(catalog.model(&id, "llama3").unwrap().tool_call);
        // The built-in providers are untouched.
        assert!(catalog.provider("anthropic").is_ok());
    }

    #[test]
    fn parse_models_splits_and_dedups() {
        assert_eq!(parse_models("a,b  a\nc"), vec!["a", "b", "c"]);
        assert!(parse_models("  ").is_empty());
    }
}
