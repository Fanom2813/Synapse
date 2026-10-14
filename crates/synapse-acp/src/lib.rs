//! synapse-acp: CLI agent detection and ACP client (see docs/ARCHITECTURE.md).
//!
//! Task 1 (detection) only: resolve the user's login-shell `PATH`, inspect
//! the ACP registry, and list supported installed agents with available
//! adapter and login-state information.

pub mod client;
pub mod detect;
pub mod headless;
pub mod path;
pub mod permissions;
pub mod registry;
pub mod team_mcp;

pub use client::{config_from_adapter, default_cwd, AcpBackend, AcpConfig, AcpError, AcpSession};
pub use detect::{detect_installed, AvailableAdapter, DetectedAgent, LoginState};
pub use headless::{
    config_for_kind, default_args, default_headless_cwd, parse_headless_line,
    parse_headless_output, HeadlessBackend, HeadlessConfig, HeadlessError, HeadlessKind,
};
pub use path::{effective_path_dirs, login_shell_path, split_path_var};
pub use permissions::{acp_tool_request, handle_agent_request, AcpApproval, PermissionBridge};
pub use registry::{fetch_registry, parse_registry, RegistryAgent, RegistryError};
pub use team_mcp::{
    team_mcp_server, TeamMcpError, TeamMcpServer, TeamMcpTool, TEAM_MCP_SERVER_NAME,
    TEAM_MCP_STDIO_BINARY,
};
