//! synapse-core: domain types, team bus, orchestrator (see docs/ARCHITECTURE.md).

pub mod agents;
pub mod redact;
pub mod skills;

pub use agents::{
    api_draft, validate_draft, Agent, AgentBackend, AgentDraft, AgentStore, ApiBackend, CliBackend,
    StoreError as AgentStoreError, ThinkingLevel, ValidatedAgent, ValidationError, KNOWN_TOOLS,
};
pub use redact::{is_denied_path, Redacted, Redactor};
pub use skills::{
    Skill, SkillInput, SkillScope, SkillStore, SkillsError, LOAD_SKILL_TOOL, READ_SKILL_FILE_TOOL,
    SHELL_NOTE,
};
