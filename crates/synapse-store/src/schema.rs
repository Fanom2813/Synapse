//! SQL schema and migrations (ARCHITECTURE.md §3).

use rusqlite_migration::{Migrations, M};

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(SCHEMA_V1)])
}

/// Initial schema: full table set from ARCHITECTURE.md §3.
/// `vec_chunks` is a plain table until the sqlite-vec extension is wired in.
const SCHEMA_V1: &str = r#"
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE providers (
    id TEXT PRIMARY KEY,
    catalog_id TEXT NOT NULL,
    base_url TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1,
    key_ref TEXT NOT NULL DEFAULT ''
);
CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT '',
    instructions TEXT NOT NULL DEFAULT '',
    backend TEXT NOT NULL DEFAULT '{}',
    avatar TEXT NOT NULL DEFAULT '{}',
    tools TEXT NOT NULL DEFAULT '[]',
    permissions TEXT NOT NULL DEFAULT '{}',
    persona_source TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
);
CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    goal TEXT NOT NULL DEFAULT '',
    folder TEXT NOT NULL DEFAULT '',
    lead_agent_id TEXT NOT NULL DEFAULT '',
    template TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    archived INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE project_agents (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    overrides TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (project_id, agent_id)
);
CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind TEXT NOT NULL DEFAULT 'team_room',
    title TEXT NOT NULL DEFAULT ''
);
CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    mode TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'running',
    phase TEXT NOT NULL DEFAULT '',
    budget TEXT NOT NULL DEFAULT '{}',
    spent TEXT NOT NULL DEFAULT '{}',
    started_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    ended_at INTEGER
);
CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL DEFAULT '',
    from_id TEXT NOT NULL DEFAULT '',
    to_json TEXT NOT NULL DEFAULT '[]',
    kind TEXT NOT NULL DEFAULT 'chat',
    reply_to TEXT NOT NULL DEFAULT '',
    thread_id TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',
    meta TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
);
CREATE TABLE tool_calls (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL DEFAULT '',
    tool TEXT NOT NULL DEFAULT '',
    args TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending',
    result_summary TEXT NOT NULL DEFAULT '',
    approved_by TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
);
CREATE TABLE permission_rules (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL DEFAULT 'global',
    scope_id TEXT NOT NULL DEFAULT '',
    effect TEXT NOT NULL DEFAULT 'ask',
    tool TEXT NOT NULL DEFAULT '',
    pattern TEXT NOT NULL DEFAULT ''
);
CREATE TABLE sources (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    fetched_at INTEGER NOT NULL DEFAULT 0,
    content_md TEXT NOT NULL DEFAULT '',
    content_hash TEXT NOT NULL DEFAULT '',
    etag TEXT NOT NULL DEFAULT '',
    credibility_note TEXT NOT NULL DEFAULT ''
);
CREATE TABLE citations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id TEXT NOT NULL DEFAULT '',
    artifact_id TEXT NOT NULL DEFAULT '',
    source_id TEXT NOT NULL DEFAULT '',
    quote TEXT NOT NULL DEFAULT ''
);
CREATE TABLE artifacts (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL DEFAULT '',
    current_version INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE artifact_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    body_md TEXT NOT NULL DEFAULT '',
    changelog TEXT NOT NULL DEFAULT '',
    created_by_run TEXT NOT NULL DEFAULT '',
    UNIQUE (artifact_id, version)
);
CREATE TABLE memories (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL DEFAULT 'project',
    scope_id TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL DEFAULT '',
    text TEXT NOT NULL DEFAULT '',
    source_message_id TEXT NOT NULL DEFAULT '',
    created_by TEXT NOT NULL DEFAULT '',
    valid_from INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    invalid_at INTEGER,
    confidence REAL NOT NULL DEFAULT 0,
    pinned INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE mcp_servers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    transport TEXT NOT NULL DEFAULT '{}',
    enabled INTEGER NOT NULL DEFAULT 1,
    scope TEXT NOT NULL DEFAULT 'global',
    tool_toggles TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE schedules (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL DEFAULT '',
    spec TEXT NOT NULL DEFAULT '{}',
    mode TEXT NOT NULL DEFAULT '',
    prompt TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1,
    last_run_at INTEGER,
    next_run_at INTEGER
);
CREATE TABLE catalog_cache (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    etag TEXT NOT NULL DEFAULT '',
    fetched_at INTEGER NOT NULL DEFAULT 0,
    blob BLOB NOT NULL DEFAULT x''
);
CREATE TABLE vec_chunks (
    id TEXT PRIMARY KEY,
    owner_kind TEXT NOT NULL DEFAULT '',
    owner_id TEXT NOT NULL DEFAULT '',
    chunk_index INTEGER NOT NULL DEFAULT 0,
    text TEXT NOT NULL DEFAULT '',
    embedding BLOB
);
CREATE VIRTUAL TABLE messages_fts USING fts5(body, content='messages', content_rowid='rowid');
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, body) VALUES (new.rowid, new.body);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, body) VALUES ('delete', old.rowid, old.body);
END;
CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, body) VALUES ('delete', old.rowid, old.body);
    INSERT INTO messages_fts(rowid, body) VALUES (new.rowid, new.body);
END;
"#;
