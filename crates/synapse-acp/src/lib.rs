//! synapse-acp: CLI agent detection and ACP client (see docs/ARCHITECTURE.md).
//!
//! Task 1 (detection) only: resolve the user's login-shell `PATH`, inspect
//! the ACP registry, and list supported installed agents with available
//! adapter and login-state information.

pub mod client;
pub mod detect;
pub mod path;
pub mod permissions;
pub mod registry;

pub use client::{config_from_adapter, default_cwd, AcpBackend, AcpConfig, AcpError, AcpSession};
pub use detect::{detect_installed, AvailableAdapter, DetectedAgent, LoginState};
pub use path::{effective_path_dirs, login_shell_path, split_path_var};
pub use permissions::{acp_tool_request, handle_agent_request, AcpApproval, PermissionBridge};
pub use registry::{fetch_registry, parse_registry, RegistryAgent, RegistryError};
