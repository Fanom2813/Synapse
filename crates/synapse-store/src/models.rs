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
