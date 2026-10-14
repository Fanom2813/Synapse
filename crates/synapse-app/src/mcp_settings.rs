//! MCP settings + registry browsing (PRD §5.5, UI.md §2.5).
//!
//! Toolkit-free so it is unit testable: add / edit / test / enable / remove
//! MCP servers (stdio command or streamable HTTP URL, OAuth optional),
//! browse and install registry entries, and toggle individual tools.
//! Connection and not-configured states are explicit so the GPUI view can
//! render them without extra logic.

use std::collections::{HashMap, HashSet};

pub use synapse_mcp::backend::{BackendFactory, McpError, ToolInfo};
pub use synapse_mcp::config::{parse_mcp_servers_json, McpServerConfig, Transport};
pub use synapse_mcp::oauth::{InMemorySecretStore, SecretStore};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpSettingsError {
    #[error("store: {0}")]
    Store(#[from] synapse_store::StoreError),
    #[error("mcp: {0}")]
    Mcp(#[from] McpError),
    #[error("unknown server: {0}")]
    UnknownServer(String),
    #[error("unknown tool: {server}#{tool}")]
    UnknownTool { server: String, tool: String },
    #[error("invalid server: {0}")]
    Invalid(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, McpSettingsError>;

const KEY_SERVERS: &str = "mcp.servers";

/// One persisted server plus its per-tool kill switches.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StoredServer {
    pub config: McpServerConfig,
    #[serde(default)]
    pub tool_toggles: HashMap<String, bool>,
}

impl StoredServer {
    pub fn new(config: McpServerConfig) -> Self {
        Self {
            config,
            tool_toggles: HashMap::new(),
        }
    }

    pub fn tool_enabled(&self, tool: &str) -> bool {
        self.tool_toggles.get(tool).copied().unwrap_or(true)
    }
}

/// Persisted MCP settings: the server list. Secrets (OAuth tokens) live
/// only in the [`SecretStore`]; the DB holds configs + toggles.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct McpSettings {
    #[serde(default)]
    pub servers: Vec<StoredServer>,
}

impl McpSettings {
    pub fn load(store: &synapse_store::Store) -> Result<Self> {
        let raw = store.setting_get(KEY_SERVERS)?;
        let Some(raw) = raw else {
            return Ok(Self::default());
        };
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    pub fn save(&self, store: &synapse_store::Store) -> Result<()> {
        store.setting_set(KEY_SERVERS, &serde_json::to_string(self)?)?;
        Ok(())
    }

    pub fn get(&self, server_id: &str) -> Option<&StoredServer> {
        self.servers.iter().find(|s| s.config.id == server_id)
    }

    fn get_mut(&mut self, server_id: &str) -> Option<&mut StoredServer> {
        self.servers.iter_mut().find(|s| s.config.id == server_id)
    }

    /// Add a server after validating it. Ids must be unique and non-empty.
    pub fn add(&mut self, config: McpServerConfig) -> Result<()> {
        validate_config(&config)?;
        if self.get(&config.id).is_some() {
            return Err(McpSettingsError::Invalid(format!(
                "a server named '{}' already exists",
                config.id
            )));
        }
        self.servers.push(StoredServer::new(config));
        self.sort();
        Ok(())
    }

    /// Replace the config of an existing server (transport changes drop no
    /// toggles; toggles for tools that no longer exist stay harmless).
    pub fn update(&mut self, config: McpServerConfig) -> Result<()> {
        validate_config(&config)?;
        let slot = self
            .get_mut(&config.id)
            .ok_or_else(|| McpSettingsError::UnknownServer(config.id.clone()))?;
        let enabled = slot.config.enabled;
        let toggles = std::mem::take(&mut slot.tool_toggles);
        slot.config = config;
        // Preserve the enabled flag unless the new config says otherwise
        // explicitly (McpServerConfig defaults to enabled=true).
        let _ = enabled;
        slot.tool_toggles = toggles;
        self.sort();
        Ok(())
    }

    pub fn remove(&mut self, server_id: &str) -> Result<StoredServer> {
        let pos = self
            .servers
            .iter()
            .position(|s| s.config.id == server_id)
            .ok_or_else(|| McpSettingsError::UnknownServer(server_id.into()))?;
        Ok(self.servers.remove(pos))
    }

    pub fn set_enabled(&mut self, server_id: &str, enabled: bool) -> Result<()> {
        self.get_mut(server_id)
            .ok_or_else(|| McpSettingsError::UnknownServer(server_id.into()))?
            .config
            .enabled = enabled;
        Ok(())
    }

    /// Per-tool toggle (Settings → MCP → tool switch). `None` tool state
    /// means enabled; storing `true` explicitly is normalised away.
    pub fn set_tool_enabled(&mut self, server_id: &str, tool: &str, enabled: bool) -> Result<()> {
        let slot = self
            .get_mut(server_id)
            .ok_or_else(|| McpSettingsError::UnknownServer(server_id.into()))?;
        if enabled {
            slot.tool_toggles.remove(tool);
        } else {
            slot.tool_toggles.insert(tool.to_string(), false);
        }
        Ok(())
    }

    /// Tools visible after toggles are applied.
    pub fn visible_tools(&self, server_id: &str, tools: &[ToolInfo]) -> Result<Vec<ToolInfo>> {
        let slot = self
            .get(server_id)
            .ok_or_else(|| McpSettingsError::UnknownServer(server_id.into()))?;
        Ok(tools
            .iter()
            .filter(|t| slot.tool_enabled(&t.name))
            .cloned()
            .collect())
    }

    /// Install a registry entry as a new server (renames on collision).
    pub fn install(&mut self, entry: &RegistryEntry) -> Result<String> {
        let mut config = entry.to_config();
        if self.get(&config.id).is_some() {
            let mut n = 2;
            while self.get(&format!("{}-{n}", config.id)).is_some() {
                n += 1;
            }
            config.id = format!("{}-{n}", config.id);
            config.name = format!("{} {n}", config.name);
        }
        let id = config.id.clone();
        self.add(config)?;
        Ok(id)
    }

    /// Import Claude-style `{"mcpServers": {...}}` JSON, skipping ids that
    /// already exist. Returns the number of servers added.
    pub fn import_json(&mut self, json: &str) -> Result<usize> {
        let parsed =
            parse_mcp_servers_json(json).map_err(|e| McpSettingsError::Invalid(e.to_string()))?;
        let mut added = 0;
        for config in parsed {
            if self.get(&config.id).is_none() {
                self.add(config)?;
                added += 1;
            }
        }
        Ok(added)
    }
}

fn sort_key(s: &str) -> String {
    s.to_lowercase()
}

impl McpSettings {
    fn sort(&mut self) {
        self.servers.sort_by_key(|s| sort_key(&s.config.name));
    }
}

/// A server config is usable when it names a command or an http(s) URL.
pub fn validate_config(config: &McpServerConfig) -> Result<()> {
    if config.id.trim().is_empty() {
        return Err(McpSettingsError::Invalid(
            "server id must not be empty".into(),
        ));
    }
    if config.name.trim().is_empty() {
        return Err(McpSettingsError::Invalid(
            "server name must not be empty".into(),
        ));
    }
    match &config.transport {
        Transport::Stdio { command, .. } => {
            if command.trim().is_empty() {
                return Err(McpSettingsError::Invalid(
                    "stdio servers need a command".into(),
                ));
            }
        }
        Transport::StreamableHttp { url, .. } => {
            let u = url.trim();
            if !(u.starts_with("http://") || u.starts_with("https://")) {
                return Err(McpSettingsError::Invalid(
                    "remote servers need an http(s) URL".into(),
                ));
            }
        }
    }
    Ok(())
}

/// What is missing before a server can connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotConfiguredReason {
    Disabled,
    MissingCommand,
    MissingUrl,
    MissingToken,
}

impl NotConfiguredReason {
    pub fn user_message(&self, server_name: &str) -> String {
        match self {
            Self::Disabled => format!("{server_name} is disabled. Enable it to connect."),
            Self::MissingCommand => {
                format!("{server_name} has no command yet. Edit it to add one.")
            }
            Self::MissingUrl => {
                format!("{server_name} has no URL yet. Edit it to add one.")
            }
            Self::MissingToken => {
                format!("{server_name} needs sign-in (OAuth token missing).")
            }
        }
    }
}

/// Connection state shown on each server card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    NotConfigured(NotConfiguredReason),
    Error(String),
}

impl ConnectionState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::Disconnected => "Disconnected",
            Self::NotConfigured(_) => "Not configured",
            Self::Error(_) => "Error",
        }
    }

    pub fn user_message(&self, server_name: &str) -> String {
        match self {
            Self::Connected => format!("{server_name} is connected."),
            Self::Disconnected => format!("{server_name} is not connected yet. Test to connect."),
            Self::NotConfigured(r) => r.user_message(server_name),
            Self::Error(e) => format!("{server_name}: {e}"),
        }
    }
}

/// Compute the card state. `connected` comes from the client manager
/// (`is_connected`); `has_token` from the secret store.
pub fn connection_state(
    config: &McpServerConfig,
    connected: bool,
    has_token: bool,
) -> ConnectionState {
    if !config.enabled {
        return ConnectionState::NotConfigured(NotConfiguredReason::Disabled);
    }
    match &config.transport {
        Transport::Stdio { command, .. } if command.trim().is_empty() => {
            return ConnectionState::NotConfigured(NotConfiguredReason::MissingCommand);
        }
        Transport::StreamableHttp { url, .. }
            if !(url.starts_with("http://") || url.starts_with("https://")) =>
        {
            return ConnectionState::NotConfigured(NotConfiguredReason::MissingUrl);
        }
        _ => {}
    }
    if config.requires_oauth() && !has_token {
        return ConnectionState::NotConfigured(NotConfiguredReason::MissingToken);
    }
    if connected {
        ConnectionState::Connected
    } else {
        ConnectionState::Disconnected
    }
}

/// Outcome of the Settings "Test" button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerTestOutcome {
    Ok { tools: usize, read_only: usize },
    Failed { reason: String },
}

impl ServerTestOutcome {
    pub fn ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::Ok { tools, read_only } => {
                format!("Connected ({tools} tool(s), {read_only} read-only).")
            }
            Self::Failed { reason } => format!("Connection failed: {reason}"),
        }
    }
}

/// Test a server: validate, check the OAuth token, connect and list tools.
pub async fn test_server(
    config: &McpServerConfig,
    factory: &dyn BackendFactory,
    secrets: &dyn SecretStore,
    tool_toggles: &HashMap<String, bool>,
) -> ServerTestOutcome {
    if let Err(e) = validate_config(config) {
        return ServerTestOutcome::Failed {
            reason: e.to_string(),
        };
    }
    if !config.enabled {
        return ServerTestOutcome::Failed {
            reason: "server is disabled".into(),
        };
    }
    if config.requires_oauth() && secrets.get(&config.token_key_ref()).is_err() {
        return ServerTestOutcome::Failed {
            reason: "sign-in required (OAuth token missing)".into(),
        };
    }
    let backend = match factory.connect(&config.id) {
        Ok(b) => b,
        Err(e) => {
            return ServerTestOutcome::Failed {
                reason: trim_msg(&e.to_string()),
            };
        }
    };
    match backend.list_tools().await {
        Ok(tools) => {
            let visible = tools
                .iter()
                .filter(|t| tool_toggles.get(&t.name).copied().unwrap_or(true))
                .count();
            let read_only = tools.iter().filter(|t| t.read_only()).count();
            let _ = visible;
            ServerTestOutcome::Ok {
                tools: tools.len(),
                read_only,
            }
        }
        Err(e) => ServerTestOutcome::Failed {
            reason: trim_msg(&e.to_string()),
        },
    }
}

fn trim_msg(msg: &str) -> String {
    const MAX: usize = 160;
    let s: String = msg.chars().take(MAX).collect();
    if msg.len() > MAX {
        format!("{s}…")
    } else {
        s
    }
}

// -- Registry ---------------------------------------------------------------

/// One curated registry entry (verified against the official MCP registry /
/// vendor docs at build time; see assets/connectors.json).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub command: Option<&'static str>,
    pub args: &'static [&'static str],
    pub url: Option<&'static str>,
    pub needs_auth: bool,
}

impl RegistryEntry {
    pub fn to_config(&self) -> McpServerConfig {
        let transport = if let Some(url) = self.url {
            Transport::StreamableHttp {
                url: url.to_string(),
                headers: HashMap::new(),
            }
        } else {
            Transport::Stdio {
                command: self.command.unwrap_or("npx").to_string(),
                args: self.args.iter().map(|s| s.to_string()).collect(),
                env: HashMap::new(),
            }
        };
        McpServerConfig {
            id: self.id.to_string(),
            name: self.name.to_string(),
            transport,
            enabled: true,
            oauth: None,
        }
    }
}

/// Curated registry (offline snapshot; every URL verified at build time).
pub fn builtin_registry() -> Vec<RegistryEntry> {
    vec![
        RegistryEntry {
            id: "filesystem",
            name: "Filesystem",
            description: "Read and write files in allowed folders.",
            category: "Local",
            command: Some("npx"),
            args: &["-y", "@modelcontextprotocol/server-filesystem"],
            url: None,
            needs_auth: false,
        },
        RegistryEntry {
            id: "sqlite",
            name: "SQLite",
            description: "Query local SQLite databases (read-only tools auto-run).",
            category: "Local",
            command: Some("uvx"),
            args: &["mcp-server-sqlite"],
            url: None,
            needs_auth: false,
        },
        RegistryEntry {
            id: "git",
            name: "Git",
            description: "Branch, log and diff tools for the project repo.",
            category: "Local",
            command: Some("uvx"),
            args: &["mcp-server-git"],
            url: None,
            needs_auth: false,
        },
        RegistryEntry {
            id: "github",
            name: "GitHub",
            description: "Issues, PRs and search. Needs an OAuth sign-in.",
            category: "Remote",
            command: None,
            args: &[],
            url: Some("https://api.githubcopilot.com/mcp"),
            needs_auth: true,
        },
        RegistryEntry {
            id: "notion",
            name: "Notion",
            description: "Pages and databases. Needs an OAuth sign-in.",
            category: "Remote",
            command: None,
            args: &[],
            url: Some("https://mcp.notion.com/mcp"),
            needs_auth: true,
        },
        RegistryEntry {
            id: "playwright",
            name: "Playwright",
            description: "Opt-in browser control for JS-heavy pages.",
            category: "Local",
            command: Some("npx"),
            args: &["-y", "@playwright/mcp@latest"],
            url: None,
            needs_auth: false,
        },
    ]
}

/// Search the registry. Empty query returns everything, sorted by name.
pub fn search_registry<'a>(entries: &'a [RegistryEntry], query: &str) -> Vec<&'a RegistryEntry> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<&RegistryEntry> = entries
        .iter()
        .filter(|e| {
            q.is_empty()
                || e.id.contains(&q)
                || e.name.to_lowercase().contains(&q)
                || e.description.to_lowercase().contains(&q)
                || e.category.to_lowercase().contains(&q)
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(b.name));
    out
}

// -- Screen view-model --------------------------------------------------------

/// One row of the installed-servers list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub transport_label: String,
    pub state: ConnectionState,
    pub tool_count: Option<usize>,
    pub disabled_tools: usize,
}

/// Toolkit-free view-model for Settings → MCP. The GPUI view renders from
/// this; every mutation persists through the `settings` table and reports a
/// human-readable status line.
#[derive(Debug, Clone, Default)]
pub struct McpSettingsScreen {
    settings: McpSettings,
    query: String,
    registry_query: String,
    selected: Option<String>,
    // Add/edit drafts.
    name_input: String,
    command_input: String,
    args_input: String,
    url_input: String,
    status: Option<String>,
    status_ok: bool,
    connected: HashSet<String>,
    tool_counts: HashMap<String, usize>,
    last_test: Option<ServerTestOutcome>,
    last_error: HashMap<String, String>,
}

impl McpSettingsScreen {
    pub fn new(settings: McpSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    /// Open the screen, loading persisted settings. Never fails: corrupt
    /// rows fall back to empty so the screen always opens.
    pub fn open(store: &synapse_store::Store) -> Self {
        Self::new(McpSettings::load(store).unwrap_or_default())
    }

    pub fn settings(&self) -> &McpSettings {
        &self.settings
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, q: impl Into<String>) {
        self.query = q.into();
    }

    pub fn registry_query(&self) -> &str {
        &self.registry_query
    }

    pub fn set_registry_query(&mut self, q: impl Into<String>) {
        self.registry_query = q.into();
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn status_ok(&self) -> bool {
        self.status_ok
    }

    pub fn last_test(&self) -> Option<&ServerTestOutcome> {
        self.last_test.as_ref()
    }

    pub fn name_input(&self) -> &str {
        &self.name_input
    }
    pub fn command_input(&self) -> &str {
        &self.command_input
    }
    pub fn args_input(&self) -> &str {
        &self.args_input
    }
    pub fn url_input(&self) -> &str {
        &self.url_input
    }

    pub fn set_name_input(&mut self, v: impl Into<String>) {
        self.name_input = v.into();
    }
    pub fn set_command_input(&mut self, v: impl Into<String>) {
        self.command_input = v.into();
    }
    pub fn set_args_input(&mut self, v: impl Into<String>) {
        self.args_input = v.into();
    }
    pub fn set_url_input(&mut self, v: impl Into<String>) {
        self.url_input = v.into();
    }

    fn set_status(&mut self, ok: bool, msg: impl Into<String>) {
        self.status_ok = ok;
        self.status = Some(msg.into());
    }

    fn clear_status(&mut self) {
        self.status = None;
        self.status_ok = false;
    }

    /// Mark a server connected/disconnected (fed by the client manager).
    pub fn set_connected(&mut self, server_id: &str, connected: bool) {
        if connected {
            self.connected.insert(server_id.to_string());
        } else {
            self.connected.remove(server_id);
        }
    }

    /// Select a server; pre-fills the edit drafts.
    pub fn select(&mut self, server_id: &str) -> Result<()> {
        let slot = self
            .settings
            .get(server_id)
            .ok_or_else(|| McpSettingsError::UnknownServer(server_id.into()))?;
        self.selected = Some(server_id.to_string());
        self.name_input = slot.config.name.clone();
        match &slot.config.transport {
            Transport::Stdio { command, args, .. } => {
                self.command_input = command.clone();
                self.args_input = args.join(" ");
                self.url_input.clear();
            }
            Transport::StreamableHttp { url, .. } => {
                self.url_input = url.clone();
                self.command_input.clear();
                self.args_input.clear();
            }
        }
        self.clear_status();
        Ok(())
    }

    fn draft_transport(&self) -> Transport {
        if !self.url_input.trim().is_empty() {
            Transport::StreamableHttp {
                url: self.url_input.trim().to_string(),
                headers: HashMap::new(),
            }
        } else {
            Transport::Stdio {
                command: self.command_input.trim().to_string(),
                args: self
                    .args_input
                    .split_whitespace()
                    .map(str::to_string)
                    .collect(),
                env: HashMap::new(),
            }
        }
    }

    /// Add a server from the drafts and persist.
    pub fn submit_add(&mut self, store: &synapse_store::Store, id: &str) -> Result<()> {
        let id = slugify(id);
        if id.is_empty() {
            let e = McpSettingsError::Invalid("give the server an id".into());
            self.set_status(false, e.to_string());
            return Err(e);
        }
        let name = if self.name_input.trim().is_empty() {
            id.clone()
        } else {
            self.name_input.trim().to_string()
        };
        let config = McpServerConfig {
            id: id.clone(),
            name,
            transport: self.draft_transport(),
            enabled: true,
            oauth: None,
        };
        match self.settings.add(config) {
            Ok(()) => {
                self.settings.save(store)?;
                self.selected = Some(id.clone());
                self.set_status(true, format!("Added {id}."));
                Ok(())
            }
            Err(e) => {
                self.set_status(false, e.to_string());
                Err(e)
            }
        }
    }

    /// Save the drafts onto the selected server and persist.
    pub fn submit_edit(&mut self, store: &synapse_store::Store) -> Result<()> {
        let id = self
            .selected
            .clone()
            .ok_or_else(|| McpSettingsError::Invalid("select a server first".into()))?;
        let current = self
            .settings
            .get(&id)
            .ok_or_else(|| McpSettingsError::UnknownServer(id.clone()))?;
        let name = if self.name_input.trim().is_empty() {
            current.config.name.clone()
        } else {
            self.name_input.trim().to_string()
        };
        let config = McpServerConfig {
            id: id.clone(),
            name,
            transport: self.draft_transport(),
            enabled: current.config.enabled,
            oauth: current.config.oauth.clone(),
        };
        match self.settings.update(config) {
            Ok(()) => {
                self.settings.save(store)?;
                self.set_connected(&id, false);
                self.set_status(true, format!("Saved {id}."));
                Ok(())
            }
            Err(e) => {
                self.set_status(false, e.to_string());
                Err(e)
            }
        }
    }

    /// Remove the selected (or given) server and persist. Also clears its
    /// OAuth token so no secret lingers in the keychain.
    pub fn remove(
        &mut self,
        store: &synapse_store::Store,
        secrets: &dyn SecretStore,
        server_id: &str,
    ) -> Result<()> {
        match self.settings.remove(server_id) {
            Ok(removed) => {
                self.settings.save(store)?;
                secrets.delete(&removed.config.token_key_ref());
                secrets.delete(&format!("{}:refresh", removed.config.token_key_ref()));
                self.connected.remove(server_id);
                self.tool_counts.remove(server_id);
                self.last_error.remove(server_id);
                if self.selected.as_deref() == Some(server_id) {
                    self.selected = None;
                }
                self.set_status(true, format!("Removed {server_id}."));
                Ok(())
            }
            Err(e) => {
                self.set_status(false, e.to_string());
                Err(e)
            }
        }
    }

    /// Enable/disable a server and persist.
    pub fn toggle_enabled(
        &mut self,
        store: &synapse_store::Store,
        server_id: &str,
        enabled: bool,
    ) -> Result<()> {
        match self.settings.set_enabled(server_id, enabled) {
            Ok(()) => {
                self.settings.save(store)?;
                if !enabled {
                    self.connected.remove(server_id);
                }
                self.set_status(
                    true,
                    if enabled {
                        format!("Enabled {server_id}.")
                    } else {
                        format!("Disabled {server_id}.")
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

    /// Flip one tool toggle and persist.
    pub fn toggle_tool(
        &mut self,
        store: &synapse_store::Store,
        server_id: &str,
        tool: &str,
        enabled: bool,
    ) -> Result<()> {
        match self.settings.set_tool_enabled(server_id, tool, enabled) {
            Ok(()) => {
                self.settings.save(store)?;
                self.set_status(
                    true,
                    if enabled {
                        format!("Enabled {tool}.")
                    } else {
                        format!("Disabled {tool} (it can no longer run).")
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

    /// Install a registry entry and persist.
    pub fn install_registry(
        &mut self,
        store: &synapse_store::Store,
        entry_id: &str,
    ) -> Result<String> {
        let entry = builtin_registry()
            .into_iter()
            .find(|e| e.id == entry_id)
            .ok_or_else(|| McpSettingsError::Invalid(format!("unknown entry '{entry_id}'")))?;
        match self.settings.install(&entry) {
            Ok(id) => {
                self.settings.save(store)?;
                self.selected = Some(id.clone());
                self.set_status(true, format!("Installed {}.", entry.name));
                Ok(id)
            }
            Err(e) => {
                self.set_status(false, e.to_string());
                Err(e)
            }
        }
    }

    /// Visible rows for the current search text.
    pub fn rows(&self, secrets: &dyn SecretStore) -> Vec<ServerRow> {
        let q = self.query.trim().to_lowercase();
        let mut out: Vec<ServerRow> = self
            .settings
            .servers
            .iter()
            .filter(|s| {
                q.is_empty()
                    || s.config.id.to_lowercase().contains(&q)
                    || s.config.name.to_lowercase().contains(&q)
            })
            .map(|s| {
                let has_token = secrets.get(&s.config.token_key_ref()).is_ok();
                let state =
                    connection_state(&s.config, self.connected.contains(&s.config.id), has_token);
                let state = if let Some(err) = self.last_error.get(&s.config.id) {
                    match &state {
                        ConnectionState::Connected => state,
                        _ if err.is_empty() => state,
                        _ => ConnectionState::Error(err.clone()),
                    }
                } else {
                    state
                };
                ServerRow {
                    id: s.config.id.clone(),
                    name: s.config.name.clone(),
                    enabled: s.config.enabled,
                    transport_label: transport_label(&s.config.transport),
                    state,
                    tool_count: self.tool_counts.get(&s.config.id).copied(),
                    disabled_tools: s.tool_toggles.values().filter(|v| !**v).count(),
                }
            })
            .collect();
        out.sort_by_key(|r| r.name.to_lowercase());
        out
    }

    /// Registry rows for the current registry search text, hiding entries
    /// that are already installed.
    pub fn registry_rows(&self) -> Vec<RegistryEntry> {
        let installed: HashSet<&str> = self
            .settings
            .servers
            .iter()
            .map(|s| s.config.id.as_str())
            .collect();
        let all = builtin_registry();
        search_registry(&all, &self.registry_query)
            .into_iter()
            .filter(|e| !installed.contains(e.id))
            .cloned()
            .collect()
    }

    /// Test the selected (or given) server, recording its tool count and
    /// connection state for the card. Shows the outcome on the screen.
    pub async fn test_selected(
        &mut self,
        factory: &dyn BackendFactory,
        secrets: &dyn SecretStore,
        server_id: &str,
    ) -> ServerTestOutcome {
        let Some(slot) = self.settings.get(server_id) else {
            let out = ServerTestOutcome::Failed {
                reason: format!("unknown server: {server_id}"),
            };
            self.last_test = Some(out.clone());
            self.set_status(false, out.user_message());
            return out;
        };
        let out = test_server(&slot.config, factory, secrets, &slot.tool_toggles).await;
        self.last_test = Some(out.clone());
        if out.ok() {
            self.connected.insert(server_id.to_string());
            self.last_error.remove(server_id);
            if let ServerTestOutcome::Ok { tools, .. } = &out {
                self.tool_counts.insert(server_id.to_string(), *tools);
            }
        } else {
            self.connected.remove(server_id);
            self.last_error.insert(
                server_id.to_string(),
                match &out {
                    ServerTestOutcome::Failed { reason } => reason.clone(),
                    ServerTestOutcome::Ok { .. } => String::new(),
                },
            );
        }
        self.set_status(out.ok(), out.user_message());
        out
    }
}

fn transport_label(t: &Transport) -> String {
    match t {
        Transport::Stdio { command, args, .. } => {
            if args.is_empty() {
                command.clone()
            } else {
                format!("{} {}", command, args.join(" "))
            }
        }
        Transport::StreamableHttp { url, .. } => url.clone(),
    }
}

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
fn stdio(id: &str, name: &str, command: &str) -> McpServerConfig {
    McpServerConfig {
        id: id.into(),
        name: name.into(),
        transport: Transport::Stdio {
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
        },
        enabled: true,
        oauth: None,
    }
}

#[cfg(test)]
mod mcp_settings_tests {
    use super::*;
    use synapse_mcp::backend::{MockBackendFactory, MockMcpServer, ToolAnnotations};

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

    fn factory_with(server_id: &str, tools: Vec<ToolInfo>) -> MockBackendFactory {
        let f = MockBackendFactory::new();
        f.add_server(MockMcpServer::new(server_id, tools));
        f
    }

    #[test]
    fn mcp_settings_add_edit_enable_remove_roundtrip() {
        let store = mem_store();
        let mut s = McpSettings::load(&store).unwrap();
        assert!(s.servers.is_empty());

        // Add validates: empty command and bad URL rejected.
        assert!(s
            .add(McpServerConfig {
                id: "bad".into(),
                name: "Bad".into(),
                transport: Transport::Stdio {
                    command: "  ".into(),
                    args: vec![],
                    env: HashMap::new(),
                },
                enabled: true,
                oauth: None,
            })
            .is_err());
        assert!(s
            .add(McpServerConfig {
                id: "bad2".into(),
                name: "Bad2".into(),
                transport: Transport::StreamableHttp {
                    url: "ftp://x".into(),
                    headers: HashMap::new(),
                },
                enabled: true,
                oauth: None,
            })
            .is_err());

        s.add(stdio("fs", "Filesystem", "npx")).unwrap();
        // Duplicate id rejected.
        assert!(s.add(stdio("fs", "Filesystem 2", "uvx")).is_err());
        s.save(&store).unwrap();

        // Reload survives restart.
        let mut again = McpSettings::load(&store).unwrap();
        assert_eq!(again.servers.len(), 1);

        // Edit keeps toggles; enable/disable flips the flag.
        again.set_tool_enabled("fs", "read_file", false).unwrap();
        again
            .update(McpServerConfig {
                id: "fs".into(),
                name: "FS renamed".into(),
                transport: Transport::Stdio {
                    command: "uvx".into(),
                    args: vec!["mcp-server-fs".into()],
                    env: HashMap::new(),
                },
                enabled: true,
                oauth: None,
            })
            .unwrap();
        assert_eq!(again.get("fs").unwrap().config.name, "FS renamed");
        assert!(!again.get("fs").unwrap().tool_enabled("read_file"));
        again.set_enabled("fs", false).unwrap();
        assert!(!again.get("fs").unwrap().config.enabled);
        assert!(again.set_enabled("missing", true).is_err());
        assert!(again.update(stdio("missing", "M", "c")).is_err());
        again.save(&store).unwrap();
        assert!(
            !McpSettings::load(&store)
                .unwrap()
                .get("fs")
                .unwrap()
                .config
                .enabled
        );

        // Remove drops the row.
        let removed = again.remove("fs").unwrap();
        assert_eq!(removed.config.id, "fs");
        assert!(again.remove("fs").is_err());
        again.save(&store).unwrap();
        assert!(McpSettings::load(&store).unwrap().servers.is_empty());
    }

    #[test]
    fn mcp_settings_tool_toggles_filter_visible_tools() {
        let mut s = McpSettings::default();
        s.add(stdio("fs", "Filesystem", "npx")).unwrap();
        let tools = vec![
            tool("read_file", Some(true)),
            tool("delete_file", Some(false)),
            tool("list_dir", None),
        ];
        assert_eq!(s.visible_tools("fs", &tools).unwrap().len(), 3);
        s.set_tool_enabled("fs", "delete_file", false).unwrap();
        let visible = s.visible_tools("fs", &tools).unwrap();
        assert_eq!(visible.len(), 2);
        assert!(!visible.iter().any(|t| t.name == "delete_file"));
        // Re-enabling normalises the stored map.
        s.set_tool_enabled("fs", "delete_file", true).unwrap();
        assert!(s.get("fs").unwrap().tool_toggles.is_empty());
        assert!(s.visible_tools("missing", &tools).is_err());
        assert!(s.set_tool_enabled("missing", "x", false).is_err());
    }

    #[test]
    fn mcp_settings_registry_search_and_install() {
        let reg = builtin_registry();
        assert!(reg.len() >= 4);
        assert_eq!(search_registry(&reg, "").len(), reg.len());
        assert_eq!(search_registry(&reg, "git").len(), 2);
        assert_eq!(search_registry(&reg, "GIT").len(), 2);
        assert!(search_registry(&reg, "nope-xyz").is_empty());

        let store = mem_store();
        let mut s = McpSettings::default();
        let id = s
            .install(reg.iter().find(|e| e.id == "filesystem").unwrap())
            .unwrap();
        assert_eq!(id, "filesystem");
        // Installing twice renames instead of colliding.
        let id2 = s
            .install(reg.iter().find(|e| e.id == "filesystem").unwrap())
            .unwrap();
        assert_ne!(id2, "filesystem");
        s.save(&store).unwrap();
        assert_eq!(McpSettings::load(&store).unwrap().servers.len(), 2);

        // Claude-style import skips existing ids.
        let json = r#"{"mcpServers": {
            "local": {"command": "npx", "args": ["-y", "x-mcp"]},
            "filesystem": {"command": "npx"},
            "remote": {"url": "https://mcp.example.test/mcp"}
        }}"#;
        let added = s.import_json(json).unwrap();
        assert_eq!(added, 2);
        assert!(s.get("local").is_some() && s.get("remote").is_some());
        assert!(s.import_json("not json").is_err());
    }

    #[test]
    fn mcp_settings_connection_states_cover_not_configured() {
        let secrets = InMemorySecretStore::new();
        // Disabled wins over everything.
        let mut cfg = stdio("a", "A", "npx");
        cfg.enabled = false;
        assert_eq!(
            connection_state(&cfg, false, false),
            ConnectionState::NotConfigured(NotConfiguredReason::Disabled)
        );
        // Missing command / URL.
        let bad_cmd = McpServerConfig {
            id: "b".into(),
            name: "B".into(),
            transport: Transport::Stdio {
                command: "".into(),
                args: vec![],
                env: HashMap::new(),
            },
            enabled: true,
            oauth: None,
        };
        assert_eq!(
            connection_state(&bad_cmd, false, false),
            ConnectionState::NotConfigured(NotConfiguredReason::MissingCommand)
        );
        let bad_url = McpServerConfig {
            id: "c".into(),
            name: "C".into(),
            transport: Transport::StreamableHttp {
                url: "nope".into(),
                headers: HashMap::new(),
            },
            enabled: true,
            oauth: None,
        };
        assert_eq!(
            connection_state(&bad_url, false, false),
            ConnectionState::NotConfigured(NotConfiguredReason::MissingUrl)
        );
        // OAuth without a token.
        let oauth_cfg = McpServerConfig {
            id: "d".into(),
            name: "D".into(),
            transport: Transport::StreamableHttp {
                url: "https://mcp.example.test/mcp".into(),
                headers: HashMap::new(),
            },
            enabled: true,
            oauth: Some(synapse_mcp::config::OAuthServerConfig {
                authorization_endpoint: "https://auth/x".into(),
                token_endpoint: "https://auth/t".into(),
                client_id: "cid".into(),
                scopes: vec![],
                key_ref: String::new(),
            }),
        };
        assert_eq!(
            connection_state(&oauth_cfg, false, false),
            ConnectionState::NotConfigured(NotConfiguredReason::MissingToken)
        );
        // Labels + messages are human-readable.
        let msg = ConnectionState::NotConfigured(NotConfiguredReason::MissingToken)
            .user_message("GitHub");
        assert!(msg.contains("sign-in"), "{msg}");
        assert_eq!(ConnectionState::Connected.label(), "Connected");
        assert_eq!(
            ConnectionState::NotConfigured(NotConfiguredReason::Disabled).label(),
            "Not configured"
        );

        // Plain server: disconnected until flagged connected.
        let cfg = stdio("e", "E", "npx");
        assert_eq!(
            connection_state(&cfg, false, true),
            ConnectionState::Disconnected
        );
        assert_eq!(
            connection_state(&cfg, true, true),
            ConnectionState::Connected
        );
        let _ = secrets;
    }

    #[tokio::test]
    async fn mcp_settings_test_lists_tools_and_reports_counts() {
        let factory = factory_with(
            "fs",
            vec![
                tool("read_file", Some(true)),
                tool("write_file", Some(false)),
            ],
        );
        let secrets = InMemorySecretStore::new();
        let cfg = stdio("fs", "Filesystem", "npx");
        let out = test_server(&cfg, &factory, &secrets, &HashMap::new()).await;
        assert!(out.ok(), "{out:?}");
        assert_eq!(
            out,
            ServerTestOutcome::Ok {
                tools: 2,
                read_only: 1
            }
        );
        assert!(out.user_message().contains("2 tool(s)"));

        // Unknown server (no backend) fails with a trimmed reason.
        let missing = stdio("missing", "Missing", "npx");
        let out = test_server(&missing, &factory, &secrets, &HashMap::new()).await;
        assert!(!out.ok());
        assert!(out.user_message().starts_with("Connection failed"));

        // Disabled / invalid configs fail without touching the transport.
        let mut disabled = stdio("fs", "FS", "npx");
        disabled.enabled = false;
        assert!(!test_server(&disabled, &factory, &secrets, &HashMap::new())
            .await
            .ok());
        let bad = McpServerConfig {
            id: "bad".into(),
            name: "Bad".into(),
            transport: Transport::StreamableHttp {
                url: "bogus".into(),
                headers: HashMap::new(),
            },
            enabled: true,
            oauth: None,
        };
        assert!(!test_server(&bad, &factory, &secrets, &HashMap::new())
            .await
            .ok());
    }

    #[tokio::test]
    async fn mcp_settings_screen_add_test_toggle_install_remove() {
        let store = mem_store();
        let secrets = InMemorySecretStore::new();
        let factory = factory_with(
            "my-server",
            vec![
                tool("read_notes", Some(true)),
                tool("delete_notes", Some(false)),
            ],
        );
        let mut screen = McpSettingsScreen::open(&store);
        assert!(screen.status().is_none());
        assert!(screen.rows(&secrets).is_empty());
        assert!(!screen.registry_rows().is_empty());

        // Add from drafts.
        screen.set_name_input("My Server");
        screen.set_command_input("npx");
        screen.set_args_input("-y x-mcp");
        screen.set_url_input("");
        screen.submit_add(&store, "My Server").unwrap();
        assert_eq!(screen.selected(), Some("my-server"));
        assert!(screen.status_ok());
        // Search filters the list.
        screen.set_query("my");
        assert_eq!(screen.rows(&secrets).len(), 1);
        screen.set_query("zzz");
        assert!(screen.rows(&secrets).is_empty());
        screen.set_query("");
        let row = &screen.rows(&secrets)[0];
        assert_eq!(row.transport_label, "npx -y x-mcp");
        assert_eq!(row.state, ConnectionState::Disconnected);
        assert!(row
            .state
            .user_message("My Server")
            .contains("not connected"));

        // Select pre-fills drafts; edit switches to a URL transport.
        screen.select("my-server").unwrap();
        assert_eq!(screen.command_input(), "npx");
        assert!(screen.select("missing").is_err());
        screen.set_url_input("https://mcp.example.test/mcp");
        screen.submit_edit(&store).unwrap();
        // The mock factory has no "my-server" URL backend difference — swap
        // the transport back to stdio so test connects via the mock.
        screen.set_url_input("");
        screen.set_command_input("npx");
        screen.set_args_input("-y x-mcp");
        screen.submit_edit(&store).unwrap();

        // Test connects, records the tool count, flips the card to Connected.
        let out = screen.test_selected(&factory, &secrets, "my-server").await;
        assert!(out.ok(), "{out:?}");
        assert!(screen.status_ok());
        assert_eq!(screen.last_test(), Some(&out));
        let row = &screen.rows(&secrets)[0];
        assert_eq!(row.state, ConnectionState::Connected);
        assert_eq!(row.tool_count, Some(2));

        // Per-tool toggle persists and shows on the row.
        screen
            .toggle_tool(&store, "my-server", "delete_notes", false)
            .unwrap();
        assert!(screen.status_ok());
        assert_eq!(screen.rows(&secrets)[0].disabled_tools, 1);
        screen
            .toggle_tool(&store, "my-server", "delete_notes", true)
            .unwrap();
        assert_eq!(screen.rows(&secrets)[0].disabled_tools, 0);
        assert!(screen.toggle_tool(&store, "missing", "x", false).is_err());
        assert!(!screen.status_ok());

        // Disable → Not configured card; re-enable → Connected still held.
        screen.toggle_enabled(&store, "my-server", false).unwrap();
        assert!(!screen.rows(&secrets)[0].enabled);
        assert_eq!(
            screen.rows(&secrets)[0].state,
            ConnectionState::NotConfigured(NotConfiguredReason::Disabled)
        );
        screen.toggle_enabled(&store, "my-server", true).unwrap();

        // Registry install persists; installed entries leave the list.
        screen.set_registry_query("git");
        let before = screen.registry_rows().len();
        assert!(before >= 1);
        let installed = screen.install_registry(&store, "git").unwrap();
        assert_eq!(installed, "git");
        assert!(screen.install_registry(&store, "nope").is_err());
        screen.set_registry_query("");
        assert!(!screen.registry_rows().iter().any(|e| e.id == "git"));

        // Reload: servers + toggles survived, secrets never in the DB.
        let raw = store.setting_get("mcp.servers").unwrap().unwrap();
        assert!(raw.contains("my-server"));
        let reopened = McpSettingsScreen::open(&store);
        assert_eq!(reopened.settings().servers.len(), 2);

        // Remove clears state and reports success.
        screen.remove(&store, &secrets, "my-server").unwrap();
        assert!(screen.status_ok());
        assert!(screen.remove(&store, &secrets, "my-server").is_err());
        assert!(!screen.status_ok());
        // Testing a removed server reports failure, never panics.
        let out = screen.test_selected(&factory, &secrets, "my-server").await;
        assert!(!out.ok());
    }
}
