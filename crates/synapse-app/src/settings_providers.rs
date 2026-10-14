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

/// Toolkit-free view-model for the navigable Settings → Providers screen
/// (UI.md §2.5). The GPUI view renders from this; all behaviour here runs
/// without a display. Every mutation persists through the `settings` table
/// (`ProvidersSettings::save`) and the configured [`SecretStore`], and
/// reports a human-readable success/failure status for the screen.
#[derive(Debug, Clone, Default)]
pub struct ProvidersScreen {
    settings: ProvidersSettings,
    query: String,
    selected: Option<String>,
    key_input: String,
    base_url_input: String,
    status: Option<String>,
    status_ok: bool,
    last_test: Option<KeyTestOutcome>,
}

impl ProvidersScreen {
    pub fn new(settings: ProvidersSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    /// Open the screen, loading persisted settings. Never fails: corrupt
    /// rows fall back to defaults so the screen always opens.
    pub fn open(store: &synapse_store::Store) -> Self {
        Self::new(ProvidersSettings::load(store).unwrap_or_default())
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Select a catalog provider; pre-fills the endpoint draft.
    pub fn select(&mut self, catalog: &Catalog, provider_id: &str) -> Result<()> {
        catalog
            .provider(provider_id)
            .map_err(|_| ProvidersError::UnknownProvider(provider_id.into()))?;
        self.selected = Some(provider_id.to_string());
        self.base_url_input = self
            .settings
            .base_url_for(catalog, provider_id)
            .unwrap_or_default();
        self.key_input.clear();
        self.clear_status();
        Ok(())
    }

    pub fn key_input(&self) -> &str {
        &self.key_input
    }

    pub fn set_key_input(&mut self, v: impl Into<String>) {
        self.key_input = v.into();
    }

    pub fn base_url_input(&self) -> &str {
        &self.base_url_input
    }

    pub fn set_base_url_input(&mut self, v: impl Into<String>) {
        self.base_url_input = v.into();
    }

    /// Last success/failure line shown on the screen, if any.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn status_ok(&self) -> bool {
        self.status_ok
    }

    pub fn last_test(&self) -> Option<&KeyTestOutcome> {
        self.last_test.as_ref()
    }

    pub fn settings(&self) -> &ProvidersSettings {
        &self.settings
    }

    fn set_status(&mut self, ok: bool, msg: impl Into<String>) {
        self.status_ok = ok;
        self.status = Some(msg.into());
    }

    fn clear_status(&mut self) {
        self.status = None;
        self.status_ok = false;
    }

    /// Visible rows for the current search text.
    pub fn rows(&self, catalog: &Catalog, secrets: &dyn SecretStore) -> Vec<ProviderRow> {
        search_providers(catalog, &self.settings, secrets, &self.query)
    }

    /// Enable/disable a provider and persist.
    pub fn toggle_enabled(
        &mut self,
        catalog: &Catalog,
        store: &synapse_store::Store,
        provider_id: &str,
        enabled: bool,
    ) -> Result<()> {
        match self.settings.set_enabled(catalog, provider_id, enabled) {
            Ok(()) => {
                self.settings.save(store)?;
                self.set_status(
                    true,
                    if enabled {
                        format!("Enabled {provider_id}.")
                    } else {
                        format!("Disabled {provider_id}.")
                    },
                );
                Ok(())
            }
            Err(e) => {
                self.set_status(false, e.to_string());
                Err(e)
            }
        }
    }

    /// Save the key draft into the secret store (never the DB).
    pub fn submit_key(&mut self, secrets: &dyn SecretStore, provider_id: &str) -> Result<()> {
        match self
            .settings
            .set_key(secrets, provider_id, self.key_input.trim())
        {
            Ok(()) => {
                self.key_input.clear();
                self.set_status(true, format!("Saved key for {provider_id}."));
                Ok(())
            }
            Err(e) => {
                self.set_status(false, e.to_string());
                Err(e)
            }
        }
    }

    /// Delete the provider key from the secret store.
    pub fn remove_key(&mut self, secrets: &dyn SecretStore, provider_id: &str) -> Result<()> {
        match self.settings.remove_key(secrets, provider_id) {
            Ok(()) => {
                self.set_status(true, format!("Removed key for {provider_id}."));
                Ok(())
            }
            Err(e) => {
                self.set_status(false, e.to_string());
                Err(e)
            }
        }
    }

    /// Save the endpoint draft as the custom base URL override and persist.
    pub fn submit_base_url(
        &mut self,
        store: &synapse_store::Store,
        provider_id: &str,
    ) -> Result<()> {
        self.settings
            .set_custom_base_url(provider_id, self.base_url_input.trim());
        self.settings.save(store)?;
        self.set_status(true, format!("Saved endpoint for {provider_id}."));
        Ok(())
    }

    /// Pick the default model for a role and persist.
    pub fn set_default(
        &mut self,
        catalog: &Catalog,
        store: &synapse_store::Store,
        role: ModelRole,
        provider_id: &str,
        model_id: &str,
    ) -> Result<()> {
        match self
            .settings
            .set_default(catalog, role, provider_id, model_id)
        {
            Ok(()) => {
                self.settings.save(store)?;
                self.set_status(
                    true,
                    format!("Default {} is now {provider_id}/{model_id}.", role.as_str()),
                );
                Ok(())
            }
            Err(e) => {
                self.set_status(false, e.to_string());
                Err(e)
            }
        }
    }

    /// Test the stored key for the selected (or given) provider against its
    /// own effective base URL. Shows the success/failure line on the screen.
    pub async fn test_selected_key(
        &mut self,
        catalog: &Catalog,
        secrets: &dyn SecretStore,
        client: &reqwest::Client,
        provider_id: &str,
    ) -> KeyTestOutcome {
        use secrecy::ExposeSecret as _;
        let base = self
            .settings
            .base_url_for(catalog, provider_id)
            .unwrap_or_default();
        let key = secrets
            .get("provider", provider_id)
            .ok()
            .flatten()
            .map(|s| s.expose_secret().to_string())
            .unwrap_or_default();
        let outcome = test_key(client, &base, &key).await;
        self.last_test = Some(outcome.clone());
        self.set_status(outcome.ok(), outcome.user_message());
        outcome
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

    #[test]
    fn settings_providers_screen_search_enable_key_defaults_persist() {
        let cat = test_catalog();
        let store = mem_store();
        let secrets = MemorySecretStore::new();
        let mut screen = ProvidersScreen::open(&store);
        assert!(screen.status().is_none());

        // Search filters the catalog list.
        screen.set_query("open");
        let rows = screen.rows(&cat, &secrets);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "openai");
        screen.set_query("");
        assert_eq!(screen.rows(&cat, &secrets).len(), 2);

        // Select + enable persists through the settings table.
        screen.select(&cat, "openai").unwrap();
        assert_eq!(screen.selected(), Some("openai"));
        assert!(screen.select(&cat, "nope").is_err());
        screen.toggle_enabled(&cat, &store, "openai", true).unwrap();
        assert!(screen.settings().is_enabled("openai"));
        assert!(screen.status_ok());
        assert!(screen.status().unwrap().contains("Enabled"));
        assert!(screen.toggle_enabled(&cat, &store, "nope", true).is_err());
        assert!(!screen.status_ok());

        // Enter + remove the key through the secret store only.
        screen.set_key_input("sk-screen-key-abcdef1234567890");
        screen.submit_key(&secrets, "openai").unwrap();
        assert!(screen.settings().has_key(&secrets, "openai"));
        assert_eq!(screen.key_input(), "");
        assert!(screen.status_ok());
        let rows = screen.rows(&cat, &secrets);
        let row = rows.iter().find(|r| r.id == "openai").unwrap();
        assert!(row.has_key && row.key_masked.is_some());
        screen.set_key_input("   ");
        assert!(screen.submit_key(&secrets, "openai").is_err());
        assert!(!screen.status_ok());
        screen.remove_key(&secrets, "openai").unwrap();
        assert!(!screen.settings().has_key(&secrets, "openai"));

        // Endpoint override + per-role defaults persist.
        screen.set_base_url_input("http://127.0.0.1:1234/v1");
        screen.submit_base_url(&store, "openai").unwrap();
        screen
            .set_default(&cat, &store, ModelRole::Lead, "openai", "gpt-4o-mini")
            .unwrap();
        assert!(screen
            .set_default(&cat, &store, ModelRole::Lead, "openai", "nope")
            .is_err());
        assert!(!screen.status_ok());

        // Reload: flags/defaults/URLs survived, the DB never held the key.
        let reopened = ProvidersScreen::open(&store);
        assert!(reopened.settings().is_enabled("openai"));
        assert_eq!(
            reopened.settings().default_for(ModelRole::Lead),
            Some(("openai".to_string(), "gpt-4o-mini".to_string()))
        );
        assert_eq!(
            reopened.settings().base_url_for(&cat, "openai").as_deref(),
            Some("http://127.0.0.1:1234/v1")
        );
        let raw = store.setting_get("providers.enabled").unwrap().unwrap();
        assert!(!raw.contains("sk-screen-key"));
    }

    #[tokio::test]
    async fn settings_providers_screen_test_shows_success_and_failure() {
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
                "data": [{"id": "a"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad"))
            .mount(&server)
            .await;
        let cat = test_catalog();
        let store = mem_store();
        let secrets = MemorySecretStore::new();
        let client = reqwest::Client::new();
        let mut screen = ProvidersScreen::open(&store);
        screen.select(&cat, "openai").unwrap();
        screen.set_base_url_input(format!("{}/v1", server.uri()));
        screen.submit_base_url(&store, "openai").unwrap();

        secrets.set("provider", "openai", "good-key").unwrap();
        let ok = screen
            .test_selected_key(&cat, &secrets, &client, "openai")
            .await;
        assert!(ok.ok());
        assert!(screen.status_ok());
        assert!(screen.status().unwrap().contains("1 model(s)"));

        secrets.set("provider", "openai", "bad-key").unwrap();
        let bad = screen
            .test_selected_key(&cat, &secrets, &client, "openai")
            .await;
        assert!(!bad.ok());
        assert!(!screen.status_ok());
        assert!(screen.status().unwrap().contains("failed"));
    }
}
