//! synapse-core: domain types, team bus, orchestrator (see docs/ARCHITECTURE.md).

pub mod agent_builder;
pub mod agents;
pub mod artifacts;
pub mod attachments;
pub mod bus;
pub mod memory_files;
pub mod memory_recall;
pub mod memory_write;
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
pub use memory_files::{
    agent_memory_path, diff_memory_files, digest_memory_path, memory_mirror_needs_import,
    mirror_memory_file, parse_memory_file, project_memory_path, render_memory_file,
    user_memory_path, FileMemory, MemoryChange, MemoryFileError, ParsedEntry,
};
pub use memory_recall::{
    active_memories, archive_search, build_core_block, build_memory_context, estimate_tokens,
    memory_tool_specs, parse_archive_args, parse_recall_args, recall_search, ArchiveHit,
    ArchiveItem, MemoryContext, MemoryView, RecalledMemory, DEFAULT_MEMORY_BUDGET_TOKENS,
    DEFAULT_RETRIEVED_TOP_K, MEMORY_TOOL_NAMES, RECALL_TOOL, SEARCH_ARCHIVE_TOOL,
};
pub use memory_write::{
    extract_proposals, filter_secret_proposals, remember_explicit, should_auto_apply,
    AppliedMemory, ApplyAction, CandidateFact, ExistingMemory, MemoryOp, Proposal, RememberOutcome,
    RememberRequest, ReviewQueue,
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
