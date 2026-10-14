//! Connectors gallery (PRD §5.5, M6 task 7).
//!
//! Curated local / remote / aggregator connectors from
//! `assets/connectors.json`, all built on MCP so there is no per-app code.
//! Toolkit-free so it is unit testable: connector cards show read/write
//! capabilities, connection state and per-agent access; connect/disconnect
//! run through the mock MCP backend + fake OAuth flow in tests; connector
//! output is redacted before storage or model use.

use std::collections::{HashMap, HashSet};

use synapse_core::redact::{Redacted, Redactor};
use thiserror::Error;

pub use crate::mcp_settings::{ConnectionState, NotConfiguredReason};
use crate::mcp_settings::{McpSettings, ServerTestOutcome};
use synapse_mcp::backend::{BackendFactory, McpError, ToolInfo};
use synapse_mcp::config::{McpServerConfig, OAuthServerConfig, Transport};
use synapse_mcp::oauth::{OAuthFlow, SecretStore};

#[derive(Debug, Error)]
pub enum ConnectorsError {
    #[error("store: {0}")]
    Store(#[from] synapse_store::StoreError),
    #[error("mcp: {0}")]
    Mcp(#[from] McpError),
    #[error("unknown connector: {0}")]
    UnknownConnector(String),
    #[error("sign-in required for {0}: pass the OAuth code from authorize_url")]
    SignInRequired(String),
    #[error("invalid catalog: {0}")]
    Invalid(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ConnectorsError>;

const KEY_AGENT_ACCESS: &str = "connectors.agent_access";
const CATALOG_JSON: &str = include_str!("../../../assets/connectors.json");

/// Local (stdio, runs on demand), remote (hosted MCP + OAuth) or
/// aggregator (many apps behind one endpoint, user brings a key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorKind {
    Local,
    Remote,
    Aggregator,
}

impl ConnectorKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Remote => "Remote",
            Self::Aggregator => "Aggregator",
        }
    }
}

/// One curated connector entry. URLs were verified against the MCP
/// registry / vendor docs at build time; entries that no longer exist
/// were dropped (see PRD §5.5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConnectorEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub kind: ConnectorKind,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub needs_oauth: bool,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
}

impl ConnectorEntry {
    /// Config used when the connector is installed as an MCP server.
    pub fn to_server_config(&self) -> McpServerConfig {
        let transport = if let Some(url) = &self.url {
            Transport::StreamableHttp {
                url: url.clone(),
                headers: HashMap::new(),
            }
        } else {
            Transport::Stdio {
                command: self.command.clone().unwrap_or_else(|| "npx".into()),
                args: self.args.clone(),
                env: HashMap::new(),
            }
        };
        let oauth = if self.needs_oauth {
            Some(OAuthServerConfig {
                authorization_endpoint: format!("https://auth.example.test/authorize?server={}", self.id),
                token_endpoint: "https://auth.example.test/token".into(),
                client_id: format!("synapse-{}", self.id),
                scopes: Vec::new(),
                key_ref: String::new(),
            })
        } else {
            None
        };
        McpServerConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            transport,
            enabled: true,
            oauth,
        }
    }
}

/// Parse + validate a catalog JSON document.
pub fn parse_catalog(json: &str) -> Result<Vec<ConnectorEntry>> {
    let entries: Vec<ConnectorEntry> = serde_json::from_str(json)?;
    validate_catalog(&entries)?;
    Ok(entries)
}

fn validate_catalog(entries: &[ConnectorEntry]) -> Result<()> {
    let mut seen = HashSet::new();
    for e in entries {
        if e.id.trim().is_empty() {
            return Err(ConnectorsError::Invalid("connector id must not be empty".into()));
        }
        if e.name.trim().is_empty() {
            return Err(ConnectorsError::Invalid(format!("connector {} has no name", e.id)));
        }
        if !seen.insert(e.id.clone()) {
            return Err(ConnectorsError::Invalid(format!("duplicate connector '{}'", e.id)));
        }
        match &e.url {
            Some(u) if !(u.starts_with("https://") || u.starts_with("http://localhost")) => {
                return Err(ConnectorsError::Invalid(format!(
                    "connector {} has an unverifiable URL ({u}); drop it",
                    e.id
                )));
            }
            _ => {}
        }
        if e.url.is_none() && e.command.as_deref().unwrap_or("").trim().is_empty() {
            return Err(ConnectorsError::Invalid(format!(
                "connector {} needs a command or a URL",
                e.id
            )));
        }
    }
    Ok(())
}

/// Curated catalog embedded at build time.
pub fn builtin_catalog() -> Vec<ConnectorEntry> {
    parse_catalog(CATALOG_JSON).expect("assets/connectors.json must be valid")
}

/// One connector card: capabilities, live tools split by read/write,
/// connection state and per-agent access summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorCard {
    pub id: String,
    pub name: String,
    pub kind: ConnectorKind,
    pub category: String,
    pub description: String,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub state: ConnectionState,
    pub read_tools: Vec<String>,
    pub write_tools: Vec<String>,
    pub agents_allowed: usize,
    pub agents_total: usize,
    pub redacted_count: usize,
}

impl ConnectorCard {
    /// Short capability line, e.g. "Reads issues · Writes comments".
    pub fn capabilities_line(&self) -> String {
        let r = if self.reads.is_empty() {
            "—".to_string()
        } else {
            self.reads.join(", ")
        };
        let w = if self.writes.is_empty() {
            "read-only".to_string()
        } else {
            self.writes.join(", ")
        };
        format!("Reads {r} · Writes {w}")
    }
}

/// Write tools default to **ask** (PRD §5.5); read-only tools may auto-run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPermission {
    AutoRun,
    Ask,
}

pub fn tool_permission(tool: &ToolInfo) -> ToolPermission {
    if tool.read_only() {
        ToolPermission::AutoRun
    } else {
        ToolPermission::Ask
    }
}

/// Connectors gallery: catalog + installed servers + per-agent access +
/// cached tool lists. Secrets (OAuth tokens) live only in the
/// [`SecretStore`]; the DB holds install state + access rules.
pub struct ConnectorsGallery {
    catalog: Vec<ConnectorEntry>,
    settings: McpSettings,
    /// connector_id -> (agent_id -> allowed). Missing entry means allowed.
    agent_access: HashMap<String, HashMap<String, bool>>,
    tool_cache: HashMap<String, Vec<ToolInfo>>,
    redact_counts: HashMap<String, usize>,
    redactor: Redactor,
}

impl ConnectorsGallery {
    pub fn new(catalog: Vec<ConnectorEntry>) -> Self {
        Self {
            catalog,
            settings: McpSettings::default(),
            agent_access: HashMap::new(),
            tool_cache: HashMap::new(),
            redact_counts: HashMap::new(),
            redactor: Redactor::new(),
        }
    }

    /// Open the gallery, loading install state + access rules. Never fails:
    /// corrupt rows fall back to empty so the screen always opens.
    pub fn open(store: &synapse_store::Store) -> Self {
        let catalog = builtin_catalog();
        let settings = McpSettings::load(store).unwrap_or_default();
        let mut g = Self::new(catalog);
        g.settings = settings;
        if let Ok(Some(raw)) = store.setting_get(KEY_AGENT_ACCESS) {
            if let Ok(map) = serde_json::from_str::<HashMap<String, HashMap<String, bool>>>(&raw) {
                g.agent_access = map;
            }
        }
        g
    }

    pub fn catalog(&self) -> &[ConnectorEntry] {
        &self.catalog
    }

    pub fn entry(&self, connector_id: &str) -> Option<&ConnectorEntry> {
        self.catalog.iter().find(|e| e.id == connector_id)
    }

    fn require_entry(&self, connector_id: &str) -> Result<&ConnectorEntry> {
        self.entry(connector_id)
            .ok_or_else(|| ConnectorsError::UnknownConnector(connector_id.into()))
    }

    pub fn redactor_mut(&mut self) -> &mut Redactor {
        &mut self.redactor
    }

    pub fn add_known_secret(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.redactor.add_secret(name, value);
    }

    fn save_access(&self, store: &synapse_store::Store) -> Result<()> {
        store.setting_set(KEY_AGENT_ACCESS, &serde_json::to_string(&self.agent_access)?)?;
        Ok(())
    }

    pub fn is_installed(&self, connector_id: &str) -> bool {
        self.settings.get(connector_id).is_some()
    }

    /// Install the connector as an MCP server (idempotent) and persist.
    pub fn ensure_installed(
        &mut self,
        store: &synapse_store::Store,
        connector_id: &str,
    ) -> Result<()> {
        let entry = self.require_entry(connector_id)?.clone();
        if self.settings.get(&entry.id).is_none() {
            let config = entry.to_server_config();
            self.settings
                .add(config)
                .map_err(|e| ConnectorsError::Invalid(e.to_string()))?;
            self.settings.save(store).map_err(|e| ConnectorsError::Invalid(e.to_string()))?;
        }
        Ok(())
    }

    /// Browser URL that starts the (fake in tests) OAuth flow.
    pub fn authorize_url(&self, flow: &dyn OAuthFlow, connector_id: &str) -> Result<String> {
        self.require_entry(connector_id)?;
        Ok(flow.authorize_url(connector_id))
    }

    /// One-click connect: installs the server, runs the OAuth exchange when
    /// a token is missing (needs `auth_code`), connects the mock/real
    /// backend and caches the tool list. Write tools stay ask-by-default
    /// (see [`tool_permission`]).
    pub async fn connect(
        &mut self,
        store: &synapse_store::Store,
        factory: &dyn BackendFactory,
        secrets: &dyn SecretStore,
        flow: &dyn OAuthFlow,
        connector_id: &str,
        auth_code: Option<&str>,
    ) -> Result<ConnectionState> {
        let entry = self.require_entry(connector_id)?.clone();
        self.ensure_installed(store, &entry.id)?;
        let config = self.settings.get(&entry.id).expect("installed").config.clone();
        if config.requires_oauth() && secrets.get(&config.token_key_ref()).is_err() {
            let code = auth_code.ok_or_else(|| ConnectorsError::SignInRequired(entry.id.clone()))?;
            let (token, refresh) = flow
                .exchange_code(&entry.id, code)
                .map_err(|e| ConnectorsError::Invalid(e.to_string()))?;
            secrets.put(&config.token_key_ref(), token);
            if let Some(r) = refresh {
                secrets.put(&format!("{}:refresh", config.token_key_ref()), r);
            }
        }
        let backend = factory.connect(&config.id)?;
        let tools = backend.list_tools().await?;
        self.tool_cache.insert(entry.id.clone(), tools);
        Ok(self.connection_state(factory, secrets, &entry.id))
    }

    /// Disconnect: kills the backend connection. The OAuth token stays in
    /// the secret store unless `forget_token` is set.
    pub fn disconnect(
        &mut self,
        factory: &dyn BackendFactory,
        secrets: &dyn SecretStore,
        connector_id: &str,
        forget_token: bool,
    ) -> Result<ConnectionState> {
        let entry = self.require_entry(connector_id)?.clone();
        factory.disconnect(&entry.id);
        self.tool_cache.remove(&entry.id);
        if forget_token {
            if let Some(cfg) = self.settings.get(&entry.id) {
                let key = cfg.config.token_key_ref();
                secrets.delete(&key);
                secrets.delete(&format!("{key}:refresh"));
            }
        }
        Ok(self.connection_state(factory, secrets, &entry.id))
    }

    pub fn connection_state(
        &self,
        factory: &dyn BackendFactory,
        secrets: &dyn SecretStore,
        connector_id: &str,
    ) -> ConnectionState {
        let Some(slot) = self.settings.get(connector_id) else {
            let entry = match self.require_entry(connector_id) {
                Ok(e) => e,
                Err(_) => {
                    return ConnectionState::Error(format!("unknown connector: {connector_id}"));
                }
            };
            return crate::mcp_settings::connection_state(
                &entry.to_server_config(),
                false,
                false,
            );
        };
        let has_token = secrets.get(&slot.config.token_key_ref()).is_ok();
        crate::mcp_settings::connection_state(
            &slot.config,
            factory.is_connected(connector_id),
            has_token,
        )
    }

    /// Cached tools (populated by [`connect`](Self::connect)).
    pub fn tools(&self, connector_id: &str) -> Vec<ToolInfo> {
        self.tool_cache.get(connector_id).cloned().unwrap_or_default()
    }

    pub fn read_tools(&self, connector_id: &str) -> Vec<ToolInfo> {
        self.tools(connector_id).into_iter().filter(|t| t.read_only()).collect()
    }

    pub fn write_tools(&self, connector_id: &str) -> Vec<ToolInfo> {
        self.tools(connector_id).into_iter().filter(|t| !t.read_only()).collect()
    }

    /// Per-agent access: missing rules default to allowed.
    pub fn is_agent_allowed(&self, connector_id: &str, agent_id: &str) -> bool {
        self.agent_access
            .get(connector_id)
            .and_then(|m| m.get(agent_id))
            .copied()
            .unwrap_or(true)
    }

    pub fn set_agent_allowed(
        &mut self,
        store: &synapse_store::Store,
        connector_id: &str,
        agent_id: &str,
        allowed: bool,
    ) -> Result<()> {
        self.require_entry(connector_id)?;
        if allowed {
            if let Some(map) = self.agent_access.get_mut(connector_id) {
                map.remove(agent_id);
                if map.is_empty() {
                    self.agent_access.remove(connector_id);
                }
            }
        } else {
            self.agent_access
                .entry(connector_id.to_string())
                .or_default()
                .insert(agent_id.to_string(), false);
        }
        self.save_access(store)?;
        Ok(())
    }

    /// Can `agent_id` run `tool` on this connector right now? Disabled
    /// agents and globally-disabled tools are rejected.
    pub fn can_agent_call(
        &self,
        connector_id: &str,
        agent_id: &str,
        tool: &str,
    ) -> bool {
        if !self.is_agent_allowed(connector_id, agent_id) {
            return false;
        }
        if let Some(slot) = self.settings.get(connector_id) {
            if !slot.tool_enabled(tool) {
                return false;
            }
        }
        true
    }

    /// Call a connector tool and redact the output before it is stored or
    /// handed to a model. Returns the scrubbed text + redaction count.
    pub async fn call_tool_redacted(
        &mut self,
        factory: &dyn BackendFactory,
        connector_id: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<Redacted> {
        if self.settings.get(connector_id).is_none() {
            return Err(ConnectorsError::UnknownConnector(connector_id.into()));
        }
        let backend = factory.connect(connector_id)?;
        let out = backend.call_tool(tool, args).await?;
        let redacted = self.redactor.redact(&out.content);
        *self.redact_counts.entry(connector_id.to_string()).or_insert(0) += redacted.count;
        Ok(redacted)
    }

    /// Full card for the gallery grid / onboarding "connect apps" step.
    pub fn card(
        &self,
        factory: &dyn BackendFactory,
        secrets: &dyn SecretStore,
        connector_id: &str,
        agent_ids: &[String],
    ) -> Result<ConnectorCard> {
        let entry = self.require_entry(connector_id)?.clone();
        let state = self.connection_state(factory, secrets, connector_id);
        let tools = self.tools(connector_id);
        let mut read_tools: Vec<String> = tools.iter().filter(|t| t.read_only()).map(|t| t.name.clone()).collect();
        let mut write_tools: Vec<String> = tools.iter().filter(|t| !t.read_only()).map(|t| t.name.clone()).collect();
        read_tools.sort();
        write_tools.sort();
        let allowed = agent_ids.iter().filter(|a| self.is_agent_allowed(connector_id, a)).count();
        Ok(ConnectorCard {
            id: entry.id,
            name: entry.name,
            kind: entry.kind,
            category: entry.category,
            description: entry.description,
            reads: entry.reads,
            writes: entry.writes,
            state,
            read_tools,
            write_tools,
            agents_allowed: allowed,
            agents_total: agent_ids.len(),
            redacted_count: self.redact_counts.get(connector_id).copied().unwrap_or(0),
        })
    }

    pub fn cards(
        &self,
        factory: &dyn BackendFactory,
        secrets: &dyn SecretStore,
        agent_ids: &[String],
    ) -> Vec<ConnectorCard> {
        let mut out: Vec<ConnectorCard> = self
            .catalog
            .iter()
            .filter_map(|e| self.card(factory, secrets, &e.id, agent_ids).ok())
            .collect();
        out.sort_by_key(|a| a.name.to_lowercase());
        out
    }

    /// Connectors for the onboarding "connect apps" step: a small,
    /// friendly subset (local-first + the most popular remotes).
    pub fn onboarding_picks(&self) -> Vec<&ConnectorEntry> {
        const PICKS: &[&str] = &["filesystem", "github", "notion", "slack", "sqlite", "composio"];
        PICKS.iter().filter_map(|id| self.entry(id)).collect()
    }

    pub fn test_summary(&self, connector_id: &str) -> Option<ServerTestOutcome> {
        self.tool_cache.get(connector_id).map(|tools| ServerTestOutcome::Ok {
            tools: tools.len(),
            read_only: tools.iter().filter(|t| t.read_only()).count(),
        })
    }
}

#[cfg(test)]
mod connectors_tests {
    use super::*;
    use synapse_core::redact::fixtures;
    use synapse_mcp::backend::{MockBackendFactory, MockMcpServer, ToolAnnotations};
    use synapse_mcp::oauth::{FakeOAuthFlow, InMemorySecretStore};

    fn mem_store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().expect("store")
    }

    fn tool(name: &str, read_only: Option<bool>) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            description: format!("{name} tool"),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: ToolAnnotations { read_only },
        }
    }

    fn agents() -> Vec<String> {
        vec!["ada".to_string(), "mira".to_string()]
    }

    #[test]
    fn connectors_catalog_is_curated_and_verified() {
        let catalog = builtin_catalog();
        // Local + remote + aggregator coverage (PRD §5.5).
        let kinds: HashSet<String> = catalog.iter().map(|e| format!("{:?}", e.kind)).collect();
        assert!(kinds.contains("Local"), "{kinds:?}");
        assert!(kinds.contains("Remote"), "{kinds:?}");
        assert!(kinds.contains("Aggregator"), "{kinds:?}");
        assert!(catalog.len() >= 10, "expected a full gallery, got {}", catalog.len());
        // Every URL is verifiable (https); unverifiable entries were dropped.
        for e in &catalog {
            if let Some(u) = &e.url {
                assert!(u.starts_with("https://") || u.starts_with("http://localhost"), "{}: {u}", e.id);
            } else {
                assert!(!e.command.as_deref().unwrap_or("").trim().is_empty(), "{} needs a command", e.id);
            }
            assert!(!e.reads.is_empty() || !e.writes.is_empty(), "{} needs capabilities", e.id);
        }
        // Spot-check the headline connectors.
        for id in ["filesystem", "github", "notion", "composio"] {
            assert!(catalog.iter().any(|e| e.id == id), "missing {id}");
        }
        // Invalid catalogs are rejected, never silently installed.
        assert!(parse_catalog(r#"[{"id":"","name":"x","kind":"local"}]"#).is_err());
        assert!(parse_catalog(r#"[{"id":"a","name":"A","kind":"local","url":"ftp://x"}]"#).is_err());
    }

    #[tokio::test]
    async fn connectors_local_connect_card_and_disconnect() {
        let store = mem_store();
        let secrets = InMemorySecretStore::new();
        let factory = MockBackendFactory::new();
        factory.add_server(MockMcpServer::new(
            "filesystem",
            vec![tool("read_file", Some(true)), tool("write_file", Some(false))],
        ));
        let mut g = ConnectorsGallery::open(&store);
        let flow = FakeOAuthFlow::new();

        // Before install the card is Disconnected (never an error screen).
        let card = g.card(&factory, &secrets, "filesystem", &agents()).unwrap();
        assert_eq!(card.state, ConnectionState::Disconnected);
        assert!(card.capabilities_line().contains("read"), "{}", card.capabilities_line());

        // One-click connect lists tools and splits read/write.
        let state = g.connect(&store, &factory, &secrets, &flow, "filesystem", None).await.unwrap();
        assert_eq!(state, ConnectionState::Connected);
        assert_eq!(g.read_tools("filesystem").len(), 1);
        assert_eq!(g.write_tools("filesystem").len(), 1);
        // Write tools default to ask.
        assert_eq!(tool_permission(&tool("write_file", Some(false))), ToolPermission::Ask);
        assert_eq!(tool_permission(&tool("read_file", Some(true))), ToolPermission::AutoRun);
        let card = g.card(&factory, &secrets, "filesystem", &agents()).unwrap();
        assert_eq!(card.state, ConnectionState::Connected);
        assert_eq!(card.read_tools, vec!["read_file".to_string()]);
        assert_eq!(card.write_tools, vec!["write_file".to_string()]);
        assert_eq!(card.agents_allowed, 2);

        // Disconnect drops the connection but keeps the install.
        let state = g.disconnect(&factory, &secrets, "filesystem", false).unwrap();
        assert_eq!(state, ConnectionState::Disconnected);
        assert!(g.is_installed("filesystem"));
        assert!(g.connect(&store, &factory, &secrets, &flow, "nope", None).await.is_err());
    }

    #[tokio::test]
    async fn connectors_remote_oauth_fake_flow_and_disconnect() {
        let store = mem_store();
        let secrets = InMemorySecretStore::new();
        let factory = MockBackendFactory::new();
        factory.add_server(MockMcpServer::new("github", vec![tool("search_issues", Some(true))]));
        let mut g = ConnectorsGallery::open(&store);
        let flow = FakeOAuthFlow::new();

        // Authorize URL opens the (fake) browser flow.
        let url = g.authorize_url(&flow, "github").unwrap();
        assert!(url.contains("github"), "{url}");

        // Without a code the gallery asks for sign-in instead of failing silently.
        let err = g.connect(&store, &factory, &secrets, &flow, "github", None).await.unwrap_err();
        assert!(err.to_string().contains("sign-in"), "{err}");

        // Wrong code is rejected; the right fake code connects and stores the token.
        assert!(g.connect(&store, &factory, &secrets, &flow, "github", Some("bogus")).await.is_err());
        let state = g.connect(&store, &factory, &secrets, &flow, "github", Some("code-github")).await.unwrap();
        assert_eq!(state, ConnectionState::Connected);
        // Token lives only in the secret store, never in the DB.
        assert_eq!(secrets.test_value("mcp-oauth:github").as_deref(), Some("token-for-github"));
        let raw = store.setting_get("mcp.servers").unwrap().unwrap_or_default();
        assert!(!raw.contains("token-for-github"), "{raw}");
        assert!(g.card(&factory, &secrets, "github", &agents()).unwrap().state == ConnectionState::Connected);

        // Disconnect keeps the token (reconnect needs no new code); forget drops it.
        g.disconnect(&factory, &secrets, "github", false).unwrap();
        let state = g.connect(&store, &factory, &secrets, &flow, "github", None).await.unwrap();
        assert_eq!(state, ConnectionState::Connected);
        g.disconnect(&factory, &secrets, "github", true).unwrap();
        assert!(secrets.test_value("mcp-oauth:github").is_none());
    }

    #[tokio::test]
    async fn connectors_per_agent_access_and_tool_gates() {
        let store = mem_store();
        let secrets = InMemorySecretStore::new();
        let factory = MockBackendFactory::new();
        factory.add_server(MockMcpServer::new(
            "notion",
            vec![tool("read_page", Some(true)), tool("update_page", Some(false))],
        ));
        let mut g = ConnectorsGallery::open(&store);
        let flow = FakeOAuthFlow::new();
        g.connect(&store, &factory, &secrets, &flow, "notion", Some("code-notion")).await.unwrap();

        // Everyone allowed by default.
        assert!(g.is_agent_allowed("notion", "ada"));
        assert!(g.can_agent_call("notion", "ada", "read_page"));

        // Deny one agent: card counts it, calls are gated, others unaffected.
        g.set_agent_allowed(&store, "notion", "mira", false).unwrap();
        assert!(!g.is_agent_allowed("notion", "mira"));
        assert!(!g.can_agent_call("notion", "mira", "read_page"));
        assert!(g.can_agent_call("notion", "ada", "read_page"));
        let card = g.card(&factory, &secrets, "notion", &agents()).unwrap();
        assert_eq!((card.agents_allowed, card.agents_total), (1, 2));

        // Rules survive reload.
        let reopened = ConnectorsGallery::open(&store);
        assert!(!reopened.is_agent_allowed("notion", "mira"));
        assert!(reopened.is_agent_allowed("notion", "ada"));

        // Re-allow restores full access.
        g.set_agent_allowed(&store, "notion", "mira", true).unwrap();
        assert!(g.is_agent_allowed("notion", "mira"));
        assert!(g.set_agent_allowed(&store, "nope", "ada", false).is_err());
    }

    #[tokio::test]
    async fn connectors_output_redacted_before_storage_or_model_use() {
        let store = mem_store();
        let secrets = InMemorySecretStore::new();
        let factory = MockBackendFactory::new();
        // The mock echoes args back, so the fake key would leak without redaction.
        factory.add_server(MockMcpServer::new("sqlite", vec![tool("query", Some(true))]));
        let mut g = ConnectorsGallery::open(&store);
        for (name, value) in fixtures::all() {
            g.add_known_secret(name, value);
        }
        let flow = FakeOAuthFlow::new();
        g.connect(&store, &factory, &secrets, &flow, "sqlite", None).await.unwrap();

        for (_, secret) in fixtures::all() {
            let args = serde_json::json!({"sql": "select 1", "note": secret});
            let out: Redacted = g.call_tool_redacted(&factory, "sqlite", "query", args).await.unwrap();
            assert!(!out.text.contains(secret), "leaked {secret:?} in {:?}", out.text);
            assert!(out.count >= 1, "nothing redacted for {secret:?}");
            assert!(out.text.contains("[redacted:"), "{}", out.text);
        }
        // The card reports redactions; unknown tools error cleanly.
        let card = g.card(&factory, &secrets, "sqlite", &agents()).unwrap();
        assert!(card.redacted_count >= fixtures::all().len(), "{card:?}");
        assert!(g.call_tool_redacted(&factory, "sqlite", "missing_tool", serde_json::json!({})).await.is_err());
        assert!(g.call_tool_redacted(&factory, "nope", "query", serde_json::json!({})).await.is_err());
    }

    #[test]
    fn connectors_onboarding_picks_use_gallery() {
        let store = mem_store();
        let g = ConnectorsGallery::open(&store);
        let picks = g.onboarding_picks();
        assert!(!picks.is_empty());
        assert!(picks.iter().any(|e| e.id == "filesystem"));
        // Gallery grid is sorted and covers the whole catalog.
        let secrets = InMemorySecretStore::new();
        let factory = MockBackendFactory::new();
        let cards = g.cards(&factory, &secrets, &agents());
        assert_eq!(cards.len(), g.catalog().len());
        let names: Vec<_> = cards.iter().map(|c| c.name.to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }
}
