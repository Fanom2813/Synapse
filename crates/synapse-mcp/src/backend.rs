//! Backend abstraction over rmcp transports plus a mock for tests.
//!
//! Production wiring uses rmcp's `transport-child-process` (stdio) and
//! `transport-streamable-http-client-reqwest` (HTTP) clients behind this
//! trait; the manager only drives the trait, so lazy connect, toggles and
//! idle shutdown are transport-independent and fully testable offline.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum McpError {
    #[error("server not registered: {0}")]
    UnknownServer(String),
    #[error("server disabled: {0}")]
    Disabled(String),
    #[error("server not connected: {0}")]
    NotConnected(String),
    #[error("missing OAuth token for server: {0}")]
    MissingToken(String),
    #[error("tool not found: {server}#{tool}")]
    UnknownTool { server: String, tool: String },
    #[error("tool disabled: {server}#{tool}")]
    ToolDisabled { server: String, tool: String },
    #[error("transport error: {0}")]
    Transport(String),
}

/// Read/write annotations for one tool. `None` means the server did not
/// annotate the tool, in which case [`ToolInfo::read_only`] falls back to
/// the name heuristic in [`infer_read_only`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolAnnotations {
    pub read_only: Option<bool>,
}

/// One tool advertised by an MCP server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub annotations: ToolAnnotations,
}

impl ToolInfo {
    pub fn read_only(&self) -> bool {
        match self.annotations.read_only {
            Some(v) => v,
            None => infer_read_only(&self.name),
        }
    }
}

/// Heuristic for servers that omit `annotations.readOnly`: read/search/
/// list/get/query/describe verbs are read-only (auto-runnable).
pub fn infer_read_only(name: &str) -> bool {
    const READ_PREFIXES: &[&str] = &[
        "read", "get", "list", "search", "query", "fetch", "describe", "show", "lookup", "find",
    ];
    let lower = name.to_lowercase();
    let stem = lower.split(['_', '-', '/', ':']).next().unwrap_or(&lower);
    READ_PREFIXES.iter().any(|p| stem.starts_with(p))
}

/// Successful tool-call result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallResult {
    pub content: String,
    #[serde(default)]
    pub is_error: bool,
}

/// Transport-level connection. The manager holds one per connected server.
#[async_trait]
pub trait McpBackend: Send + Sync {
    fn server_id(&self) -> &str;
    async fn list_tools(&self) -> Result<Vec<ToolInfo>, McpError>;
    async fn call_tool(&self, tool: &str, args: Value) -> Result<CallResult, McpError>;
}

/// Factory the manager uses to (lazily) open a connection.
pub trait BackendFactory: Send + Sync {
    fn connect(&self, server_id: &str) -> Result<Arc<dyn McpBackend>, McpError>;
    fn disconnect(&self, server_id: &str);
    fn is_connected(&self, server_id: &str) -> bool;
}

/// In-memory mock server for tests: scripted tools, call log, and
/// connect/disconnect counters to prove lazy + idle behaviour.
#[derive(Debug, Default)]
pub struct MockMcpServer {
    server_id: String,
    tools: Mutex<Vec<ToolInfo>>,
    calls: Mutex<Vec<(String, Value)>>,
    fail_next: Mutex<Option<String>>,
}

impl MockMcpServer {
    pub fn new(server_id: &str, tools: Vec<ToolInfo>) -> Arc<Self> {
        Arc::new(Self {
            server_id: server_id.to_string(),
            tools: Mutex::new(tools),
            calls: Mutex::default(),
            fail_next: Mutex::default(),
        })
    }

    pub fn calls(server: &Arc<Self>) -> Vec<(String, Value)> {
        server.calls.lock().expect("poisoned").clone()
    }

    pub fn fail_next(server: &Arc<Self>, message: &str) {
        *server.fail_next.lock().expect("poisoned") = Some(message.to_string());
    }
}

#[async_trait]
impl McpBackend for MockMcpServer {
    fn server_id(&self) -> &str {
        &self.server_id
    }

    async fn list_tools(&self) -> Result<Vec<ToolInfo>, McpError> {
        Ok(self.tools.lock().expect("poisoned").clone())
    }

    async fn call_tool(&self, tool: &str, args: Value) -> Result<CallResult, McpError> {
        if let Some(msg) = self.fail_next.lock().expect("poisoned").take() {
            return Err(McpError::Transport(msg));
        }
        let known = self
            .tools
            .lock()
            .expect("poisoned")
            .iter()
            .any(|t| t.name == tool);
        if !known {
            return Err(McpError::UnknownTool {
                server: self.server_id.clone(),
                tool: tool.to_string(),
            });
        }
        self.calls
            .lock()
            .expect("poisoned")
            .push((tool.to_string(), args.clone()));
        Ok(CallResult {
            content: format!("{tool} ok: {args}"),
            is_error: false,
        })
    }
}

/// Mock factory backing [`MockMcpServer`]s by server id.
#[derive(Default)]
pub struct MockBackendFactory {
    servers: Mutex<HashMap<String, Arc<MockMcpServer>>>,
    connected: Mutex<HashMap<String, Arc<dyn McpBackend>>>,
    connects: Mutex<HashMap<String, usize>>,
    disconnects: Mutex<HashMap<String, usize>>,
}

impl MockBackendFactory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_server(&self, server: Arc<MockMcpServer>) {
        self.servers
            .lock()
            .expect("poisoned")
            .insert(server.server_id.clone(), server);
    }

    pub fn connect_count(&self, server_id: &str) -> usize {
        *self
            .connects
            .lock()
            .expect("poisoned")
            .get(server_id)
            .unwrap_or(&0)
    }

    pub fn disconnect_count(&self, server_id: &str) -> usize {
        *self
            .disconnects
            .lock()
            .expect("poisoned")
            .get(server_id)
            .unwrap_or(&0)
    }

    pub fn server(&self, server_id: &str) -> Option<Arc<MockMcpServer>> {
        self.servers
            .lock()
            .expect("poisoned")
            .get(server_id)
            .cloned()
    }
}

impl BackendFactory for MockBackendFactory {
    fn connect(&self, server_id: &str) -> Result<Arc<dyn McpBackend>, McpError> {
        if let Some(existing) = self.connected.lock().expect("poisoned").get(server_id) {
            return Ok(existing.clone());
        }
        let server = self
            .servers
            .lock()
            .expect("poisoned")
            .get(server_id)
            .cloned()
            .ok_or_else(|| McpError::UnknownServer(server_id.to_string()))?;
        let backend: Arc<dyn McpBackend> = server;
        self.connected
            .lock()
            .expect("poisoned")
            .insert(server_id.to_string(), backend.clone());
        *self
            .connects
            .lock()
            .expect("poisoned")
            .entry(server_id.to_string())
            .or_insert(0) += 1;
        Ok(backend)
    }

    fn disconnect(&self, server_id: &str) {
        self.connected.lock().expect("poisoned").remove(server_id);
        *self
            .disconnects
            .lock()
            .expect("poisoned")
            .entry(server_id.to_string())
            .or_insert(0) += 1;
    }

    fn is_connected(&self, server_id: &str) -> bool {
        self.connected
            .lock()
            .expect("poisoned")
            .contains_key(server_id)
    }
}

use async_trait::async_trait;
