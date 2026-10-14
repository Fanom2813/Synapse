//! Providers settings screen model (PRD §5.3, UI.md §2.5).
//!
//! UI-toolkit free so it is unit testable: searchable catalog list, enable a
//! provider, enter/remove a key (stored only in the [`SecretStore`]), test the
//! key against the provider's own host, and default models per role
//! (lead / member / selector). Persisted in the `settings` table; the DB
//! never holds a secret value.

use std::collections::HashMap;

use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use synapse_llm::catalog::{Catalog, Provider};
pub use synapse_llm::keys::{mask_key, MemorySecretStore, SecretStore};

#[derive(Debug, Error)]
pub enum ProvidersError {
    #[error("store: {0}")]
    Store(#[from] synapse_store::StoreError),
    #[error("secrets: {0}")]
    Secrets(String),
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("unknown model: {0}/{1}")]
    UnknownModel(String, String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("network: {0}")]
    Network(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, ProvidersError>;

const KEY_ENABLED: &str = "providers.enabled";
const KEY_DEFAULTS: &str = "models.defaults";
const KEY_CUSTOM_URLS: &str = "providers.custom_base_urls";

/// Roles with a default model (UI.md §2.5: lead, member, selector).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Lead,
    Member,
    Selector,
}

impl ModelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Member => "member",
            Self::Selector => "selector",
        }
    }

    pub fn all() -> [Self; 3] {
        [Self::Lead, Self::Member, Self::Selector]
    }
}

impl std::str::FromStr for ModelRole {
    type Err = ProvidersError;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "lead" => Ok(Self::Lead),
            "member" => Ok(Self::Member),
            "selector" => Ok(Self::Selector),
            other => Err(ProvidersError::Invalid(format!(
                "unknown role '{other}', expected lead|member|selector"
            ))),
        }
    }
}

/// One row of the searchable catalog list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub has_key: bool,
    pub key_masked: Option<String>,
    pub model_count: usize,
    pub base_url: Option<String>,
}

/// Persisted provider settings (enabled flags, per-role defaults, custom URLs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvidersSettings {
    #[serde(default)]
    pub enabled: HashMap<String, bool>,
    /// role -> "provider/model".
    #[serde(default)]
    pub defaults: HashMap<String, String>,
    #[serde(default)]
    pub custom_base_urls: HashMap<String, String>,
}

impl ProvidersSettings {
    pub fn load(store: &synapse_store::Store) -> Result<Self> {
        let mut out = Self::default();
        if let Some(raw) = store.setting_get(KEY_ENABLED)? {
            out.enabled = serde_json::from_str(&raw).unwrap_or_default();
        }
        if let Some(raw) = store.setting_get(KEY_DEFAULTS)? {
            out.defaults = serde_json::from_str(&raw).unwrap_or_default();
        }
        if let Some(raw) = store.setting_get(KEY_CUSTOM_URLS)? {
            out.custom_base_urls = serde_json::from_str(&raw).unwrap_or_default();
        }
        Ok(out)
    }

    pub fn save(&self, store: &synapse_store::Store) -> Result<()> {
        store.setting_set(KEY_ENABLED, &serde_json::to_string(&self.enabled)?)?;
        store.setting_set(KEY_DEFAULTS, &serde_json::to_string(&self.defaults)?)?;
        store.setting_set(
            KEY_CUSTOM_URLS,
            &serde_json::to_string(&self.custom_base_urls)?,
        )?;
        Ok(())
    }

    pub fn is_enabled(&self, provider_id: &str) -> bool {
        self.enabled.get(provider_id).copied().unwrap_or(false)
    }

    /// Enable/disable a catalog provider (validates the id exists).
    pub fn set_enabled(
        &mut self,
        catalog: &Catalog,
        provider_id: &str,
        enabled: bool,
    ) -> Result<()> {
        catalog
            .provider(provider_id)
            .map_err(|_| ProvidersError::UnknownProvider(provider_id.into()))?;
        self.enabled.insert(provider_id.to_string(), enabled);
        Ok(())
    }

    /// Custom OpenAI-compatible base URL for a provider (empty clears it).
    pub fn set_custom_base_url(&mut self, provider_id: &str, url: &str) {
        let url = url.trim();
        if url.is_empty() {
            self.custom_base_urls.remove(provider_id);
        } else {
            self.custom_base_urls
                .insert(provider_id.to_string(), url.to_string());
        }
    }

    /// Effective base URL: custom override wins, else the catalog value.
    pub fn base_url_for(&self, catalog: &Catalog, provider_id: &str) -> Option<String> {
        if let Some(custom) = self.custom_base_urls.get(provider_id) {
            return Some(custom.clone());
        }
        catalog
            .provider(provider_id)
            .ok()
            .and_then(|p| p.api.clone())
    }

    /// Default model for a role as `(provider, model)`.
    pub fn default_for(&self, role: ModelRole) -> Option<(String, String)> {
        self.defaults
            .get(role.as_str())
            .and_then(|v| v.split_once('/'))
            .map(|(p, m)| (p.to_string(), m.to_string()))
    }

    /// Pick the default model for a role (validates against the catalog).
    pub fn set_default(
        &mut self,
        catalog: &Catalog,
        role: ModelRole,
        provider_id: &str,
        model_id: &str,
    ) -> Result<()> {
        catalog
            .model(provider_id, model_id)
            .map_err(|_| ProvidersError::UnknownModel(provider_id.into(), model_id.into()))?;
        self.defaults.insert(
            role.as_str().to_string(),
            format!("{provider_id}/{model_id}"),
        );
        Ok(())
    }

    /// Store a key in the secret store (never in the DB). Empty keys rejected.
    pub fn set_key(&self, secrets: &dyn SecretStore, provider_id: &str, key: &str) -> Result<()> {
        if key.trim().is_empty() {
            return Err(ProvidersError::Invalid("key must not be empty".into()));
        }
        secrets
            .set("provider", provider_id, key.trim())
            .map_err(|e| ProvidersError::Secrets(e.to_string()))
    }

    /// Delete the key (called when a provider is removed / key replaced).
    pub fn remove_key(&self, secrets: &dyn SecretStore, provider_id: &str) -> Result<()> {
        secrets
            .delete("provider", provider_id)
            .map_err(|e| ProvidersError::Secrets(e.to_string()))
    }

    pub fn has_key(&self, secrets: &dyn SecretStore, provider_id: &str) -> bool {
        secrets
            .get("provider", provider_id)
            .ok()
            .flatten()
            .is_some()
    }
}

/// Searchable catalog list, sorted by name. Case-insensitive substring over
/// id + name; empty query returns everything. `has_key`/masked values come
/// from the secret store (values never leave it).
pub fn search_providers(
    catalog: &Catalog,
    settings: &ProvidersSettings,
    secrets: &dyn SecretStore,
    query: &str,
) -> Vec<ProviderRow> {
    let q = query.trim().to_lowercase();
    let mut rows: Vec<ProviderRow> = catalog
        .providers()
        .into_iter()
        .filter(|p| {
            q.is_empty() || p.id.to_lowercase().contains(&q) || p.name.to_lowercase().contains(&q)
        })
        .map(|p| {
            let masked = secrets
                .get("provider", &p.id)
                .ok()
                .flatten()
                .map(|s| mask_key(s.expose_secret()));
            ProviderRow {
                id: p.id.clone(),
                name: p.name.clone(),
                enabled: settings.is_enabled(&p.id),
                has_key: masked.is_some(),
                key_masked: masked,
                model_count: p.models.len(),
                base_url: settings
                    .custom_base_urls
                    .get(&p.id)
                    .cloned()
                    .or_else(|| p.api.clone()),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    rows
}

/// Outcome of the Settings "Test" button for an API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyTestOutcome {
    Ok { models: usize },
    Failed { reason: String },
}

impl KeyTestOutcome {
    pub fn ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::Ok { models } => format!("Key works ({models} model(s) listed)."),
            Self::Failed { reason } => format!("Key test failed: {reason}"),
        }
    }
}

fn trim_msg(msg: &str) -> String {
    const MAX: usize = 160;
    let mut s: String = msg.chars().take(MAX).collect();
    if msg.len() > MAX {
        s.push('…');
    }
    s
}

/// Test a key against the provider's own base URL only (`GET {base}/models`
/// with `Bearer`). Never called with a URL on another host: the caller
/// passes the effective base URL from [`ProvidersSettings::base_url_for`].
pub async fn test_key(client: &reqwest::Client, base_url: &str, key: &str) -> KeyTestOutcome {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return KeyTestOutcome::Failed {
            reason: "no base URL for this provider; set a custom endpoint first".into(),
        };
    }
    if key.trim().is_empty() {
        return KeyTestOutcome::Failed {
            reason: "enter a key first".into(),
        };
    }
    let url = format!("{base}/models");
    let resp = match client
        .get(&url)
        .header("authorization", format!("Bearer {}", key.trim()))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return KeyTestOutcome::Failed {
                reason: trim_msg(&e.to_string()),
            };
        }
    };
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED
        || resp.status() == reqwest::StatusCode::FORBIDDEN
    {
        return KeyTestOutcome::Failed {
            reason: "the provider rejected the key (unauthorized)".into(),
        };
    }
    if !resp.status().is_success() {
        return KeyTestOutcome::Failed {
            reason: format!("status {}", resp.status()),
        };
    }
    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return KeyTestOutcome::Failed {
                reason: trim_msg(&e.to_string()),
            };
        }
    };
    let models = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    KeyTestOutcome::Ok { models }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().unwrap()
    }

    fn test_catalog() -> Catalog {
        let raw = serde_json::json!({
            "openai": {
                "id": "openai", "name": "OpenAI",
                "api": "https://api.openai.com/v1",
                "env": ["OPENAI_API_KEY"],
                "models": {
                    "gpt-4o-mini": {"id": "gpt-4o-mini", "name": "GPT-4o mini", "tool_call": true},
                    "gpt-4o": {"id": "gpt-4o", "name": "GPT-4o", "tool_call": true}
                }
            },
            "anthropic": {
                "id": "anthropic", "name": "Anthropic",
                "api": "https://api.anthropic.com/v1",
                "env": ["ANTHROPIC_API_KEY"],
                "models": {
                    "claude-sonnet": {"id": "claude-sonnet", "name": "Claude Sonnet", "tool_call": true}
                }
            }
        });
        Catalog::from_json(&serde_json::to_vec(&raw).unwrap()).unwrap()
    }

    #[test]
    fn settings_providers_search_filters_catalog() {
        let cat = test_catalog();
        let settings = ProvidersSettings::default();
        let secrets = MemorySecretStore::new();
        let all = search_providers(&cat, &settings, &secrets, "");
        assert_eq!(all.len(), 2);
        // sorted by name: Anthropic first
        assert_eq!(all[0].id, "anthropic");
        let hits = search_providers(&cat, &settings, &secrets, "open");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "openai");
        assert_eq!(hits[0].model_count, 2);
        assert!(!hits[0].has_key);
        assert!(hits[0].key_masked.is_none());
        // case-insensitive
        assert_eq!(search_providers(&cat, &settings, &secrets, "ANTH").len(), 1);
        assert!(search_providers(&cat, &settings, &secrets, "nope").is_empty());
    }

    #[test]
    fn settings_providers_enable_and_defaults_roundtrip() {
        let cat = test_catalog();
        let store = mem_store();
        let mut s = ProvidersSettings::load(&store).unwrap();
        assert!(!s.is_enabled("openai"));
        assert!(s.default_for(ModelRole::Lead).is_none());

        s.set_enabled(&cat, "openai", true).unwrap();
        assert!(s.is_enabled("openai"));
        assert!(s.set_enabled(&cat, "nope", true).is_err());

        s.set_default(&cat, ModelRole::Lead, "openai", "gpt-4o-mini")
            .unwrap();
        s.set_default(&cat, ModelRole::Member, "anthropic", "claude-sonnet")
            .unwrap();
        s.set_default(&cat, ModelRole::Selector, "openai", "gpt-4o")
            .unwrap();
        assert_eq!(
            s.default_for(ModelRole::Lead),
            Some(("openai".to_string(), "gpt-4o-mini".to_string()))
        );
        assert!(s
            .set_default(&cat, ModelRole::Lead, "openai", "nope")
            .is_err());
        assert!(s
            .set_default(&cat, ModelRole::Lead, "nope", "gpt-4o")
            .is_err());

        // custom endpoint override wins over the catalog URL
        assert_eq!(
            s.base_url_for(&cat, "openai"),
            Some("https://api.openai.com/v1".to_string())
        );
        s.set_custom_base_url("openai", "http://localhost:1234/v1");
        assert_eq!(
            s.base_url_for(&cat, "openai"),
            Some("http://localhost:1234/v1".to_string())
        );

        s.save(&store).unwrap();
        let again = ProvidersSettings::load(&store).unwrap();
        assert_eq!(again, s);
        assert!(again.is_enabled("openai"));
        assert!(!again.is_enabled("anthropic"));
    }

    #[test]
    fn settings_providers_key_set_masked_and_remove() {
        let s = ProvidersSettings::default();
        let secrets = MemorySecretStore::new();
        assert!(!s.has_key(&secrets, "openai"));
        assert!(s.set_key(&secrets, "openai", "   ").is_err());

        let key = "sk-test-secret-openai-1234567890abcdef";
        s.set_key(&secrets, "openai", key).unwrap();
        assert!(s.has_key(&secrets, "openai"));

        // list shows masked only; the value never appears
        let cat = test_catalog();
        let rows = search_providers(&cat, &s, &secrets, "openai");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].has_key);
        let masked = rows[0].key_masked.clone().unwrap();
        assert!(!masked.contains(key));
        assert_eq!(masked, mask_key(key));

        // remove deletes the secret
        s.remove_key(&secrets, "openai").unwrap();
        assert!(!s.has_key(&secrets, "openai"));
    }

    #[tokio::test]
    async fn settings_providers_test_key_against_stub() {
        use wiremock::{
            matchers::{header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer good-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [{"id": "a"}, {"id": "b"}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let base = format!("{}/v1", server.uri());

        let ok = test_key(&client, &base, "good-key").await;
        assert_eq!(ok, KeyTestOutcome::Ok { models: 2 });
        assert!(ok.ok());
        assert!(ok.user_message().contains("2 model(s)"));

        let bad = test_key(&client, &base, "bad-key").await;
        assert!(!bad.ok());
        assert!(bad.user_message().contains("unauthorized"));

        let empty = test_key(&client, "", "good-key").await;
        assert!(!empty.ok());
        let no_key = test_key(&client, &base, "  ").await;
        assert!(!no_key.ok());
    }

    #[tokio::test]
    async fn settings_providers_test_key_unreachable_is_failed() {
        let client = reqwest::Client::new();
        let out = test_key(&client, "http://127.0.0.1:9/v1", "k").await;
        assert!(!out.ok());
        assert!(matches!(out, KeyTestOutcome::Failed { .. }));
    }
}
