//! synapse-core: domain types, team bus, orchestrator (see docs/ARCHITECTURE.md).

pub mod agents;
pub mod bus;
pub mod projects;
pub mod redact;
pub mod skills;

pub use agents::{
    api_draft, validate_draft, Agent, AgentBackend, AgentDraft, AgentStore, ApiBackend, CliBackend,
    StoreError as AgentStoreError, ThinkingLevel, ValidatedAgent, ValidationError, KNOWN_TOOLS,
};
pub use bus::{
    AttachmentRef, BusError, Envelope, Event, Kind, Participant, Priority, Recipients, SourceRef,
    TeamBus, Usage, BROADCAST_CAPACITY, INBOX_CAPACITY,
};
pub use projects::{
    builtin_template, builtin_templates, validate_project, Project, ProjectDraft, ProjectStore,
    ProjectStoreError, ProjectValidationError, SuggestedRole, TeamTemplate, ValidatedProject,
};
pub use redact::{is_denied_path, Redacted, Redactor};
pub use skills::{
    Skill, SkillInput, SkillScope, SkillStore, SkillsError, LOAD_SKILL_TOOL, READ_SKILL_FILE_TOOL,
    SHELL_NOTE,
};
