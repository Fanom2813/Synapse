//! Domain row types for projects, agents, conversations and messages.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub goal: String,
    pub folder: String,
    pub lead_agent_id: String,
    pub template: String,
    pub created_at: i64,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProject {
    pub id: String,
    pub name: String,
    pub goal: String,
    pub folder: String,
    pub lead_agent_id: String,
    pub template: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub backend: String,
    pub avatar: String,
    pub tools: String,
    pub permissions: String,
    pub persona_source: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAgent {
    pub id: String,
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub backend: String,
    pub avatar: String,
    pub tools: String,
    pub permissions: String,
    pub persona_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub from_id: String,
    pub to_json: String,
    pub kind: String,
    pub reply_to: String,
    pub thread_id: String,
    pub body: String,
    pub meta: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewMessage {
    pub id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub from_id: String,
    pub to_json: String,
    pub kind: String,
    pub reply_to: String,
    pub thread_id: String,
    pub body: String,
    pub meta: String,
}

/// A skill installed in the library (Agent Skills `SKILL.md` format).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub license: String,
    pub repo: String,
    pub sha: String,
    pub path: String,
    pub local_path: String,
    pub updated_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub body: String,
    pub license: String,
    pub repo: String,
    pub sha: String,
    pub path: String,
    pub local_path: String,
}

/// One row of the `tool_calls` table (ARCHITECTURE.md §3): every tool call
/// with redacted args, status and outcome summary for the audit view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub message_id: String,
    pub agent_id: String,
    pub tool: String,
    pub args: String,
    pub status: String,
    pub result_summary: String,
    pub approved_by: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewToolCall {
    pub id: String,
    pub message_id: String,
    pub agent_id: String,
    pub tool: String,
    pub args: String,
    pub status: String,
    pub result_summary: String,
    pub approved_by: String,
}

/// Unique id helper (`prefix` + millis + counter) without extra dependencies.
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{ms}_{n}")
}
