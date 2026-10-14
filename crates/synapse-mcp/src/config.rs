//! Server configuration: transports, OAuth refs, Claude-style import.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Transport for one MCP server (documented transports only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transport {
    /// Spawn a child process and speak MCP over stdio. Started lazily,
    /// killed after `idle_child_timeout` (see manager).
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
    /// Streamable HTTP endpoint (rmcp
    /// `transport-streamable-http-client-reqwest`).
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

/// OAuth settings for a remote server. Secrets are never stored here;
/// `key_ref` names the entry in the [`crate::oauth::SecretStore`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuthServerConfig {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub client_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Reference into the secret store, never the token itself.
    #[serde(default)]
    pub key_ref: String,
}

/// One configured MCP server (mirrors the `mcp_servers` table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub transport: Transport,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub oauth: Option<OAuthServerConfig>,
}

fn default_enabled() -> bool {
    true
}

impl McpServerConfig {
    /// Key reference used for this server's OAuth token.
    pub fn token_key_ref(&self) -> String {
        if let Some(oauth) = &self.oauth {
            if !oauth.key_ref.is_empty() {
                return oauth.key_ref.clone();
            }
        }
        format!("mcp-oauth:{}", self.id)
    }

    /// Remote servers that declare OAuth need a token before connecting.
    pub fn requires_oauth(&self) -> bool {
        self.oauth.is_some()
    }
}

/// Import Claude Desktop / Claude Code style `{"mcpServers": {...}}` JSON.
///
/// Each entry is either `{"command","args","env"}` (stdio) or `{"url",...}`
/// (streamable HTTP). Returns one [`McpServerConfig`] per entry.
pub fn parse_mcp_servers_json(json: &str) -> Result<Vec<McpServerConfig>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default, rename = "mcpServers")]
        servers: HashMap<String, RawEntry>,
    }
    #[derive(Deserialize)]
    struct RawEntry {
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        args: Option<Vec<String>>,
        #[serde(default)]
        env: Option<HashMap<String, String>>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        headers: Option<HashMap<String, String>>,
    }

    let doc: Doc = serde_json::from_str(json)?;
    Ok(doc
        .servers
        .into_iter()
        .filter_map(|(name, e)| {
            let transport = match (e.command, e.url) {
                (Some(command), _) => Transport::Stdio {
                    command,
                    args: e.args.unwrap_or_default(),
                    env: e.env.unwrap_or_default(),
                },
                (None, Some(url)) => Transport::StreamableHttp {
                    url,
                    headers: e.headers.unwrap_or_default(),
                },
                (None, None) => return None,
            };
            Some(McpServerConfig {
                id: slugify(&name),
                name,
                transport,
                enabled: true,
                oauth: None,
            })
        })
        .collect())
}

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
