//! Minimal ACP registry parsing.
//!
//! Registry source: `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`
//! (repo `agentclientprotocol/registry`, Apache-2.0). Only the fields
//! Synapse needs are modelled; unknown fields are ignored so hourly
//! registry updates do not break parsing.

use serde::Deserialize;

pub const REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("bad response ({0})")]
    BadResponse(String),
}

/// One registry entry (subset of fields).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RegistryAgent {
    /// Registry id, e.g. `claude-acp`, `codex-acp`, `opencode`, `gemini`.
    #[serde(default, alias = "name")]
    pub id: String,
    /// Human label when present.
    #[serde(default, alias = "title", alias = "label")]
    pub label: String,
    #[serde(default)]
    pub description: String,
}

impl RegistryAgent {
    pub fn display_label(&self) -> &str {
        if self.label.is_empty() {
            &self.id
        } else {
            &self.label
        }
    }
}

/// Parse registry JSON. Accepts either a bare array or an object with an
/// `agents` key (both shapes have appeared in the wild).
pub fn parse_registry(body: &str) -> Result<Vec<RegistryAgent>, String> {
    if let Ok(list) = serde_json::from_str::<Vec<RegistryAgent>>(body) {
        return Ok(list);
    }
    #[derive(Deserialize)]
    struct Wrapped {
        #[serde(default)]
        agents: Vec<RegistryAgent>,
    }
    if let Ok(w) = serde_json::from_str::<Wrapped>(body) {
        return Ok(w.agents);
    }
    Err("unrecognized registry shape".to_string())
}

/// Fetch the registry (override URL for tests). `Ok` with an empty vec on
/// non-success is not an error for callers that treat the registry as a
/// hint; this function returns `BadResponse` so callers can decide.
pub async fn fetch_registry(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<RegistryAgent>, RegistryError> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(RegistryError::BadResponse(format!(
            "status {}",
            resp.status()
        )));
    }
    let text = resp.text().await?;
    parse_registry(&text).map_err(RegistryError::BadResponse)
}
