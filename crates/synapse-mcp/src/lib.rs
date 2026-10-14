//! synapse-mcp: MCP client manager (see docs/ARCHITECTURE.md).
//!
//! Lazy connections to MCP servers over stdio and streamable HTTP,
//! OAuth tokens kept in a [`SecretStore`] (only a `key_ref` lives in config),
//! per-agent/per-tool toggles, read-only annotations, and idle shutdown.

pub mod backend;
pub mod config;
pub mod manager;
pub mod oauth;
pub mod server;

pub use backend::{CallResult, McpBackend, McpError, ToolAnnotations, ToolInfo};
pub use config::{McpServerConfig, OAuthServerConfig, Transport};
pub use manager::{ExposedTool, McpClientManager};
pub use oauth::{FakeOAuthFlow, InMemorySecretStore, OAuthFlow, SecretStore};
