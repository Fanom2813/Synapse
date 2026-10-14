//! synapse-core: domain types, team bus, orchestrator (see docs/ARCHITECTURE.md).

pub mod redact;

pub use redact::{is_denied_path, Redacted, Redactor};
