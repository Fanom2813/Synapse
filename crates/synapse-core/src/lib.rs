//! synapse-core: domain types, team bus, orchestrator (see docs/ARCHITECTURE.md).

pub mod agent_builder;
pub mod agents;
pub mod artifacts;
pub mod attachments;
pub mod bus;
pub mod orchestrator;
pub mod projects;
pub mod redact;
pub mod skills;
pub mod team_tools;
pub mod worker;

pub use agents::{
    api_draft, validate_draft, Agent, AgentBackend, AgentDraft, AgentStore, ApiBackend, CliBackend,
    StoreError as AgentStoreError, ThinkingLevel, ValidatedAgent, ValidationError, KNOWN_TOOLS,
};
pub use artifacts::{
    import_user_edit, mirror_artifact, needs_import, read_mirrored_body, render_synthesis_report,
    report_file_content, slugify, strip_frontmatter, unified_diff, Artifact, ArtifactCitation,
    ArtifactError, ArtifactStore, ArtifactVersion, Dissent, MirrorResult, ReportDraft, SourceEntry,
};
pub use attachments::{
    AttachmentError, AttachmentKind, AttachmentStore, DeliveredAttachment, RecipientCapabilities,
    StoredAttachment, INLINE_TEXT_LIMIT, MAX_ATTACHMENTS_PER_MESSAGE, MAX_ATTACHMENT_BYTES,
};
pub use bus::{
    AttachmentRef, BusError, Envelope, Event, Kind, Participant, Priority, Recipients, SourceRef,
    TeamBus, Usage, BROADCAST_CAPACITY, INBOX_CAPACITY,
};
pub use orchestrator::{
    convergence_status, is_parallel_phase, phases_for_mode, select_speaker, BudgetEvent,
    BudgetLedger, Convergence, RunLimits, RunMode, RunPhase, RunSnapshot, RunState, RunStatus,
    SpeakerDecision,
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
pub use team_tools::{TeamToolError, TeamToolSpec, TeamTools, TEAM_TOOL_NAMES};
pub use worker::{
    can_interrupt, can_steer, drain_safe_point, run_worker, AgentContext, RollingSummary,
    WorkerBackend, WorkerConfig, WorkerOutcome,
};
