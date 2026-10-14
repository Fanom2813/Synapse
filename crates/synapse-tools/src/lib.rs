//! synapse-tools: built-in tools and permission engine (see docs/ARCHITECTURE.md).

pub mod fs_tools;
pub mod permissions;
pub mod shell_tool;

pub use fs_tools::{unified_diff, FileEntry, FsError, FsScope, PendingWrite};
pub use permissions::{Decision, Effect, Engine, Request, Rule, RuleScope};
pub use shell_tool::{SandboxMode, ShellConfig, ShellError, ShellOutput, ShellTool};
