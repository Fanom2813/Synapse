//! synapse-llm: provider catalog and ModelBackend (see docs/ARCHITECTURE.md).

pub mod backend;
pub mod catalog;

pub use backend::{
    AgentEvent, GenaiBackend, LlmError, MockLlm, ModelBackend, Role, ToolCallRequest,
    ToolResultMsg, ToolSpec, TurnMessage, TurnOutput, TurnRequest, Usage,
};
pub use catalog::Catalog;
