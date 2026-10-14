//! Lazy MCP client manager: per-agent tools, read-only flags, idle shutdown.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::{BackendFactory, CallResult, McpBackend, McpError, ToolInfo};
use crate::config::McpServerConfig;
use crate::oauth::{OAuthFlow, SecretError, SecretStore};

/// One tool as exposed to a specific agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExposedTool {
    pub server_id: String,
    pub server_name: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// True when the tool is safe to auto-run without an approval card.
    pub read_only: bool,
}

struct ServerState {
    config: McpServerConfig,
    backend: Option<Arc<dyn McpBackend>>,
    tools_cache: Option<Vec<ToolInfo>>,
    last_used: Option<Instant>,
    /// Global per-tool kill switches: tool name -> enabled.
    tool_toggles: HashMap<String, bool>,
    /// Per-agent overrides: (agent, tool) -> enabled.
    agent_toggles: HashMap<(String, String), bool>,
}

/// Lazily connects to MCP servers, enforces toggles, and shuts down idle
/// connections. All OAuth tokens stay in the [`SecretStore`].
pub struct McpClientManager<S, F, B> {
    states: Mutex<HashMap<String, ServerState>>,
    secrets: S,
    oauth: F,
    factory: B,
    idle_timeout: Duration,
}

impl<S, F, B> McpClientManager<S, F, B>
where
    S: SecretStore,
    F: OAuthFlow,
    B: BackendFactory,
{
    pub fn new(secrets: S, oauth: F, factory: B, idle_timeout: Duration) -> Self {
        Self {
            states: Mutex::default(),
            secrets,
            oauth,
            factory,
            idle_timeout,
        }
    }

    pub fn register(&self, config: McpServerConfig) {
        let mut states = self.states.lock().expect("poisoned");
        states
            .entry(config.id.clone())
            .or_insert_with(|| ServerState {
                config,
                backend: None,
                tools_cache: None,
                last_used: None,
                tool_toggles: HashMap::new(),
                agent_toggles: HashMap::new(),
            });
    }

    pub fn update_config(&self, config: McpServerConfig) {
        let mut states = self.states.lock().expect("poisoned");
        if let Some(state) = states.get_mut(&config.id) {
            state.config = config;
            // Transport changes invalidate the cached connection/tools.
            state.tools_cache = None;
        } else {
            states.insert(
                config.id.clone(),
                ServerState {
                    config,
                    backend: None,
                    tools_cache: None,
                    last_used: None,
                    tool_toggles: HashMap::new(),
                    agent_toggles: HashMap::new(),
                },
            );
        }
    }

    pub fn set_server_enabled(&self, server_id: &str, enabled: bool) {
        if let Some(state) = self.states.lock().expect("poisoned").get_mut(server_id) {
            state.config.enabled = enabled;
        }
    }

    /// Global per-tool toggle (Settings → MCP → tool switch).
    pub fn set_tool_enabled(&self, server_id: &str, tool: &str, enabled: bool) {
        if let Some(state) = self.states.lock().expect("poisoned").get_mut(server_id) {
            state.tool_toggles.insert(tool.to_string(), enabled);
        }
    }

    /// Per-agent override (agent editor → MCP toggles).
    pub fn set_agent_tool_enabled(&self, agent: &str, server_id: &str, tool: &str, enabled: bool) {
        if let Some(state) = self.states.lock().expect("poisoned").get_mut(server_id) {
            state
                .agent_toggles
                .insert((agent.to_string(), tool.to_string()), enabled);
        }
    }

    pub fn is_connected(&self, server_id: &str) -> bool {
        self.states
            .lock()
            .expect("poisoned")
            .get(server_id)
            .and_then(|s| s.backend.as_ref())
            .is_some()
            && self.factory.is_connected(server_id)
    }

    pub fn connected_servers(&self) -> Vec<String> {
        self.states
            .lock()
            .expect("poisoned")
            .keys()
            .filter(|id| self.factory.is_connected(id))
            .cloned()
            .collect()
    }

    fn tool_allowed_locked(state: &ServerState, agent: Option<&str>, tool: &str) -> bool {
        if let Some(agent) = agent {
            if let Some(v) = state
                .agent_toggles
                .get(&(agent.to_string(), tool.to_string()))
            {
                return *v;
            }
        }
        *state.tool_toggles.get(tool).unwrap_or(&true)
    }

    /// Connect on first use. Returns an error (instead of connecting) when
    /// the server is disabled or its OAuth token is missing.
    pub fn ensure_connected(&self, server_id: &str) -> Result<Arc<dyn McpBackend>, McpError> {
        let requires_oauth = {
            let states = self.states.lock().expect("poisoned");
            let state = states
                .get(server_id)
                .ok_or_else(|| McpError::UnknownServer(server_id.to_string()))?;
            if !state.config.enabled {
                return Err(McpError::Disabled(server_id.to_string()));
            }
            if let Some(existing) = state.backend.clone() {
                drop(states);
                self.touch(server_id);
                return Ok(existing);
            }
            state.config.requires_oauth()
        };
        if requires_oauth {
            let key_ref = self
                .states
                .lock()
                .expect("poisoned")
                .get(server_id)
                .map(|s| s.config.token_key_ref())
                .unwrap_or_default();
            if self.secrets.get(&key_ref).is_err() {
                return Err(McpError::MissingToken(server_id.to_string()));
            }
        }
        let backend = self.factory.connect(server_id)?;
        let mut states = self.states.lock().expect("poisoned");
        if let Some(state) = states.get_mut(server_id) {
            state.backend = Some(backend.clone());
            state.last_used = Some(Instant::now());
        }
        Ok(backend)
    }

    fn touch(&self, server_id: &str) {
        if let Some(state) = self.states.lock().expect("poisoned").get_mut(server_id) {
            state.last_used = Some(Instant::now());
        }
    }

    /// List tools, caching the result per connection.
    pub async fn list_tools(&self, server_id: &str) -> Result<Vec<ToolInfo>, McpError> {
        let backend = self.ensure_connected(server_id)?;
        let cached = self
            .states
            .lock()
            .expect("poisoned")
            .get(server_id)
            .and_then(|s| s.tools_cache.clone());
        if let Some(tools) = cached {
            self.touch(server_id);
            return Ok(tools);
        }
        let tools = backend.list_tools().await?;
        if let Some(state) = self.states.lock().expect("poisoned").get_mut(server_id) {
            state.tools_cache = Some(tools.clone());
            state.last_used = Some(Instant::now());
        }
        Ok(tools)
    }

    /// Tools visible to one agent: enabled servers only, toggles applied,
    /// each annotated with its read-only flag.
    pub async fn tools_for_agent(&self, agent: &str) -> Result<Vec<ExposedTool>, McpError> {
        let ids: Vec<(String, String, bool)> = self
            .states
            .lock()
            .expect("poisoned")
            .values()
            .map(|s| (s.config.id.clone(), s.config.name.clone(), s.config.enabled))
            .collect();
        let mut out = Vec::new();
        for (id, name, enabled) in ids {
            if !enabled {
                continue;
            }
            let tools = self.list_tools(&id).await?;
            let states = self.states.lock().expect("poisoned");
            let state = states
                .get(&id)
                .ok_or_else(|| McpError::UnknownServer(id.clone()))?;
            for tool in &tools {
                if !Self::tool_allowed_locked(state, Some(agent), &tool.name) {
                    continue;
                }
                out.push(ExposedTool {
                    server_id: id.clone(),
                    server_name: name.clone(),
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                    read_only: tool.read_only(),
                });
            }
        }
        out.sort_by(|a, b| (&a.server_id, &a.name).cmp(&(&b.server_id, &b.name)));
        Ok(out)
    }

    /// Names of read-only tools for `agent` (auto-run without approval).
    pub async fn read_only_tools_for_agent(
        &self,
        agent: &str,
    ) -> Result<Vec<ExposedTool>, McpError> {
        Ok(self
            .tools_for_agent(agent)
            .await?
            .into_iter()
            .filter(|t| t.read_only)
            .collect())
    }

    /// Call a tool after enforcing server + toggle state.
    pub async fn call_tool(
        &self,
        agent: &str,
        server_id: &str,
        tool: &str,
        args: Value,
    ) -> Result<CallResult, McpError> {
        {
            let states = self.states.lock().expect("poisoned");
            let state = states
                .get(server_id)
                .ok_or_else(|| McpError::UnknownServer(server_id.to_string()))?;
            if !state.config.enabled {
                return Err(McpError::Disabled(server_id.to_string()));
            }
            if !Self::tool_allowed_locked(state, Some(agent), tool) {
                return Err(McpError::ToolDisabled {
                    server: server_id.to_string(),
                    tool: tool.to_string(),
                });
            }
        }
        let backend = self.ensure_connected(server_id)?;
        let result = backend.call_tool(tool, args).await?;
        self.touch(server_id);
        Ok(result)
    }

    /// Disconnect servers idle longer than `idle_timeout`. Returns the ids
    /// that were shut down.
    pub fn shutdown_idle(&self) -> Vec<String> {
        self.shutdown_idle_as_of(Instant::now())
    }

    fn shutdown_idle_as_of(&self, now: Instant) -> Vec<String> {
        let idle: Vec<String> = self
            .states
            .lock()
            .expect("poisoned")
            .iter()
            .filter(|(_, s)| s.backend.is_some())
            .filter(|(_, s)| {
                s.last_used
                    .map(|t| now.duration_since(t) >= self.idle_timeout)
                    .unwrap_or(true)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &idle {
            self.factory.disconnect(id);
            if let Some(state) = self.states.lock().expect("poisoned").get_mut(id) {
                state.backend = None;
                state.tools_cache = None;
            }
        }
        idle
    }

    pub fn disconnect(&self, server_id: &str) {
        self.factory.disconnect(server_id);
        if let Some(state) = self.states.lock().expect("poisoned").get_mut(server_id) {
            state.backend = None;
            state.tools_cache = None;
        }
    }

    // -- OAuth -----------------------------------------------------------

    /// URL to open so the user can authorize `server_id`.
    pub fn begin_oauth(&self, server_id: &str) -> Result<String, McpError> {
        let states = self.states.lock().expect("poisoned");
        if !states.contains_key(server_id) {
            return Err(McpError::UnknownServer(server_id.to_string()));
        }
        Ok(self.oauth.authorize_url(server_id))
    }

    /// Finish the fake/real OAuth flow and store the token under the
    /// server's `key_ref`. Only the reference is ever kept in config.
    pub fn complete_oauth(&self, server_id: &str, code: &str) -> Result<String, SecretError> {
        let key_ref = self
            .states
            .lock()
            .expect("poisoned")
            .get(server_id)
            .map(|s| s.config.token_key_ref())
            .unwrap_or_else(|| format!("mcp-oauth:{server_id}"));
        let (access, refresh) = self.oauth.exchange_code(server_id, code)?;
        self.secrets.put(&key_ref, access);
        if let Some(refresh) = refresh {
            self.secrets.put(&format!("{key_ref}:refresh"), refresh);
        }
        Ok(key_ref)
    }

    /// Forget a server's OAuth credentials (e.g. provider removed).
    pub fn forget_oauth(&self, server_id: &str) {
        let key_ref = self
            .states
            .lock()
            .expect("poisoned")
            .get(server_id)
            .map(|s| s.config.token_key_ref())
            .unwrap_or_else(|| format!("mcp-oauth:{server_id}"));
        self.secrets.delete(&key_ref);
        self.secrets.delete(&format!("{key_ref}:refresh"));
    }

    /// Direct token access for building authenticated transports. Callers
    /// must use it immediately for the request and never log or persist it.
    pub fn oauth_token(&self, server_id: &str) -> Result<SecretString, SecretError> {
        let key_ref = self
            .states
            .lock()
            .expect("poisoned")
            .get(server_id)
            .map(|s| s.config.token_key_ref())
            .unwrap_or_else(|| format!("mcp-oauth:{server_id}"));
        self.secrets.get(&key_ref)
    }

    /// Agent-visible tool names across all enabled servers (`mcp__server__tool`).
    pub async fn agent_tool_names(&self, agent: &str) -> Result<HashSet<String>, McpError> {
        Ok(self
            .tools_for_agent(agent)
            .await?
            .into_iter()
            .map(|t| format!("mcp__{}__{}", t.server_id, t.name))
            .collect())
    }
}
