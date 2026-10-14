//! synapse-core: domain types, team bus, orchestrator (see docs/ARCHITECTURE.md).

pub mod agents;
pub mod redact;

pub use agents::{
    api_draft, validate_draft, Agent, AgentBackend, AgentDraft, AgentStore, ApiBackend, CliBackend,
    StoreError as AgentStoreError, ThinkingLevel, ValidatedAgent, ValidationError, KNOWN_TOOLS,
};
pub use redact::{is_denied_path, Redacted, Redactor};
