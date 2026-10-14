//! [`Store`]: WAL SQLite file, one writer mutex, small reader pool.
//!
//! Writes take the single writer connection; reads use a pooled connection.
//! Use from async code via `tokio::task::spawn_blocking`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

use crate::models::{
    Agent, ArtifactHit, Conversation, Memory, Message, NewAgent, NewMemory, NewMessage, NewProject,
    NewSkill, NewToolCall, Project, Skill, Source, ToolCall,
};
use crate::schema::migrations;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0} not found: {1}")]
    NotFound(&'static str, String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

const READER_POOL_SIZE: usize = 4;

pub struct Store {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut writer = open_conn(&path)?;
        migrations().to_latest(&mut writer)?;
        let mut readers = Vec::with_capacity(READER_POOL_SIZE);
        for _ in 0..READER_POOL_SIZE {
            readers.push(open_conn(&path)?);
        }
        Ok(Self {
            path,
            writer: Mutex::new(writer),
            readers: Mutex::new(readers),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut writer = Connection::open_in_memory()?;
        configure(&writer)?;
        migrations().to_latest(&mut writer)?;
        Ok(Self {
            path: PathBuf::from(":memory:"),
            writer: Mutex::new(writer),
            readers: Mutex::new(Vec::new()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub fn with_journal_mode_for_test(&self) -> Result<String> {
        self.with_reader(|c| {
            let mut mode = String::new();
            c.pragma(None, "journal_mode", "WAL", |r| {
                mode = r.get(0)?;
                Ok(())
            })?;
            Ok(mode)
        })
    }

    pub(crate) fn with_writer_pub<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        self.with_writer(f)
    }

    pub(crate) fn with_reader_pub<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        self.with_reader(f)
    }

    fn with_writer<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.writer.lock().expect("writer lock");
        f(&conn)
    }

    fn with_reader<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let pooled = self.readers.lock().expect("reader lock").pop();
        if let Some(conn) = pooled {
            let out = f(&conn);
            self.readers.lock().expect("reader lock").push(conn);
            out
        } else {
            // In-memory or exhausted pool: fall back to the writer connection.
            let conn = self.writer.lock().expect("writer lock");
            f(&conn)
        }
    }

    // -- settings ---------------------------------------------------------
    pub fn setting_set(&self, key: &str, value: &str) -> Result<()> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }

    pub fn setting_get(&self, key: &str) -> Result<Option<String>> {
        self.with_reader(|c| {
            Ok(c.query_row(
                "SELECT value FROM settings WHERE key=?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
        })
    }

    // -- projects ---------------------------------------------------------
    pub fn create_project(&self, p: &NewProject) -> Result<Project> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO projects (id, name, goal, folder, lead_agent_id, template) VALUES (?1,?2,?3,?4,?5,?6)",
                params![p.id, p.name, p.goal, p.folder, p.lead_agent_id, p.template],
            )?;
            get_project_inner(c, &p.id)
        })
    }

    pub fn get_project(&self, id: &str) -> Result<Project> {
        self.with_reader(|c| get_project_inner(c, id))
    }

    pub fn list_projects(&self, include_archived: bool) -> Result<Vec<Project>> {
        self.with_reader(|c| {
            let sql = if include_archived {
                "SELECT id,name,goal,folder,lead_agent_id,template,created_at,archived FROM projects ORDER BY created_at, rowid"
            } else {
                "SELECT id,name,goal,folder,lead_agent_id,template,created_at,archived FROM projects WHERE archived=0 ORDER BY created_at, rowid"
            };
            let mut st = c.prepare(sql)?;
            let rows: Vec<_> = st.query_map([], row_to_project)?.collect::<std::result::Result<Vec<_>, _>>()?; Ok(rows)
        })
    }

    pub fn rename_project(&self, id: &str, name: &str) -> Result<Project> {
        self.with_writer(|c| {
            let n = c.execute("UPDATE projects SET name=?1 WHERE id=?2", params![name, id])?;
            if n == 0 {
                return Err(StoreError::NotFound("project", id.into()));
            }
            get_project_inner(c, id)
        })
    }

    pub fn set_project_goal(&self, id: &str, goal: &str) -> Result<()> {
        self.with_writer(|c| {
            let n = c.execute("UPDATE projects SET goal=?1 WHERE id=?2", params![goal, id])?;
            if n == 0 {
                return Err(StoreError::NotFound("project", id.into()));
            }
            Ok(())
        })
    }

    /// Delete a project and everything that belongs to it: conversations and
    /// their messages, runs, artifacts (with versions), sources, citations,
    /// memories, schedules, embeddings and its team assignments. Agents and
    /// personas are shared across projects and stay. The project's folder on
    /// disk is the user's and is never touched.
    pub fn delete_project(&self, id: &str) -> Result<()> {
        self.with_writer(|c| {
            let owned_messages = "SELECT m.id FROM messages m JOIN conversations c ON c.id = m.conversation_id WHERE c.project_id = ?1";
            let tx = c.unchecked_transaction()?;
            tx.execute(
                &format!(
                    "DELETE FROM vec_chunks WHERE owner_id = ?1 \
                     OR owner_id IN (SELECT id FROM sources WHERE project_id = ?1) \
                     OR owner_id IN (SELECT id FROM artifacts WHERE project_id = ?1) \
                     OR owner_id IN ({owned_messages})"
                ),
                params![id],
            )?;
            tx.execute(
                &format!(
                    "DELETE FROM citations WHERE message_id IN ({owned_messages}) \
                     OR artifact_id IN (SELECT id FROM artifacts WHERE project_id = ?1) \
                     OR source_id IN (SELECT id FROM sources WHERE project_id = ?1)"
                ),
                params![id],
            )?;
            tx.execute("DELETE FROM sources WHERE project_id = ?1", params![id])?;
            tx.execute("DELETE FROM schedules WHERE project_id = ?1", params![id])?;
            tx.execute(
                "DELETE FROM memories WHERE scope = 'project' AND scope_id = ?1",
                params![id],
            )?;
            tx.execute(
                "DELETE FROM settings WHERE key = ?1",
                params![format!("project.team_seeded.{id}")],
            )?;
            // Conversations, messages, runs, artifacts and team rows follow by cascade.
            let n = tx.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
            if n == 0 {
                return Err(StoreError::NotFound("project", id.into()));
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn archive_project(&self, id: &str, archived: bool) -> Result<()> {
        self.with_writer(|c| {
            let n = c.execute(
                "UPDATE projects SET archived=?1 WHERE id=?2",
                params![archived as i32, id],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound("project", id.into()));
            }
            Ok(())
        })
    }

    // -- agents -----------------------------------------------------------
    pub fn create_agent(&self, a: &NewAgent) -> Result<Agent> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO agents (id,name,role,instructions,backend,avatar,tools,permissions,persona_source) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![a.id, a.name, a.role, a.instructions, a.backend, a.avatar, a.tools, a.permissions, a.persona_source],
            )?;
            get_agent_inner(c, &a.id)
        })
    }

    pub fn get_agent(&self, id: &str) -> Result<Agent> {
        self.with_reader(|c| get_agent_inner(c, id))
    }

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        self.with_reader(|c| {
            let mut st = c.prepare("SELECT id,name,role,instructions,backend,avatar,tools,permissions,persona_source,created_at FROM agents ORDER BY created_at, rowid")?;
            let rows: Vec<_> = st.query_map([], row_to_agent)?.collect::<std::result::Result<Vec<_>, _>>()?; Ok(rows)
        })
    }

    pub fn update_agent_instructions(&self, id: &str, instructions: &str) -> Result<Agent> {
        self.with_writer(|c| {
            let n = c.execute(
                "UPDATE agents SET instructions=?1 WHERE id=?2",
                params![instructions, id],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound("agent", id.into()));
            }
            get_agent_inner(c, id)
        })
    }

    /// Point an agent at another engine (`backend` is the JSON from `AgentBackend`).
    pub fn update_agent_backend(&self, id: &str, backend: &str) -> Result<()> {
        self.with_writer(|c| {
            let n = c.execute(
                "UPDATE agents SET backend=?1 WHERE id=?2",
                params![backend, id],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound("agent", id.into()));
            }
            Ok(())
        })
    }

    /// Replace an agent's avatar (JSON from `AvatarSpec` or `PackAvatar`).
    pub fn update_agent_avatar(&self, id: &str, avatar: &str) -> Result<()> {
        self.with_writer(|c| {
            let n = c.execute("UPDATE agents SET avatar=?1 WHERE id=?2", params![avatar, id])?;
            if n == 0 {
                return Err(StoreError::NotFound("agent", id.into()));
            }
            Ok(())
        })
    }

    pub fn delete_agent(&self, id: &str) -> Result<()> {
        self.with_writer(|c| {
            c.execute("DELETE FROM agents WHERE id=?1", params![id])?;
            Ok(())
        })
    }

    pub fn add_agent_to_project(
        &self,
        project_id: &str,
        agent_id: &str,
        overrides: &str,
    ) -> Result<()> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO project_agents (project_id, agent_id, overrides) VALUES (?1,?2,?3) ON CONFLICT(project_id,agent_id) DO UPDATE SET overrides=excluded.overrides",
                params![project_id, agent_id, overrides],
            )?;
            Ok(())
        })
    }

    pub fn list_project_agents(&self, project_id: &str) -> Result<Vec<Agent>> {
        self.with_reader(|c| {
            let mut st = c.prepare(
                "SELECT a.id,a.name,a.role,a.instructions,a.backend,a.avatar,a.tools,a.permissions,a.persona_source,a.created_at
                 FROM agents a JOIN project_agents pa ON pa.agent_id=a.id WHERE pa.project_id=?1 ORDER BY a.created_at, a.rowid",
            )?;
            let rows: Vec<_> = st.query_map(params![project_id], row_to_agent)?.collect::<std::result::Result<Vec<_>, _>>()?; Ok(rows)
        })
    }

    pub fn remove_agent_from_project(&self, project_id: &str, agent_id: &str) -> Result<()> {
        self.with_writer(|c| {
            c.execute(
                "DELETE FROM project_agents WHERE project_id=?1 AND agent_id=?2",
                params![project_id, agent_id],
            )?;
            Ok(())
        })
    }

    // -- conversations + messages -----------------------------------------
    pub fn create_conversation(
        &self,
        id: &str,
        project_id: &str,
        kind: &str,
        title: &str,
    ) -> Result<Conversation> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO conversations (id, project_id, kind, title) VALUES (?1,?2,?3,?4)",
                params![id, project_id, kind, title],
            )?;
            Ok(Conversation {
                id: id.into(),
                project_id: project_id.into(),
                kind: kind.into(),
                title: title.into(),
            })
        })
    }

    pub fn list_conversations(&self, project_id: &str) -> Result<Vec<Conversation>> {
        self.with_reader(|c| {
            let mut st = c.prepare("SELECT id,project_id,kind,title FROM conversations WHERE project_id=?1 ORDER BY rowid")?;
            let rows: Vec<_> = st.query_map(params![project_id], |r| Ok(Conversation {
                    id: r.get(0)?, project_id: r.get(1)?, kind: r.get(2)?, title: r.get(3)?,
                }))?.collect::<std::result::Result<Vec<_>, _>>()?; Ok(rows)
        })
    }

    pub fn insert_message(&self, m: &NewMessage) -> Result<Message> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO messages (id,conversation_id,run_id,from_id,to_json,kind,reply_to,thread_id,body,meta) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![m.id, m.conversation_id, m.run_id, m.from_id, m.to_json, m.kind, m.reply_to, m.thread_id, m.body, m.meta],
            )?;
            get_message_inner(c, &m.id)
        })
    }

    pub fn get_message(&self, id: &str) -> Result<Message> {
        self.with_reader(|c| get_message_inner(c, id))
    }

    pub fn list_messages(
        &self,
        conversation_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Message>> {
        self.with_reader(|c| {
            let mut st = c.prepare(
                "SELECT id,conversation_id,run_id,from_id,to_json,kind,reply_to,thread_id,body,meta,created_at
                 FROM messages WHERE conversation_id=?1 ORDER BY created_at, rowid LIMIT ?2 OFFSET ?3",
            )?;
            let rows: Vec<_> = st.query_map(params![conversation_id, limit, offset], row_to_message)?.collect::<std::result::Result<Vec<_>, _>>()?; Ok(rows)
        })
    }

    pub fn search_messages(&self, query: &str, limit: i64) -> Result<Vec<Message>> {
        self.with_reader(|c| {
            let mut st = c.prepare(
                "SELECT m.id,m.conversation_id,m.run_id,m.from_id,m.to_json,m.kind,m.reply_to,m.thread_id,m.body,m.meta,m.created_at
                 FROM messages_fts f JOIN messages m ON m.rowid=f.rowid WHERE messages_fts MATCH ?1 LIMIT ?2",
            )?;
            let rows: Vec<_> = st.query_map(params![query, limit], row_to_message)?.collect::<std::result::Result<Vec<_>, _>>()?; Ok(rows)
        })
    }

    /// Archive search: FTS over messages with their project id (via
    /// conversations). Falls back to LIKE when FTS cannot parse the query.
    pub fn search_messages_with_project(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(Message, String)>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        self.with_reader(|c| {
            let try_fts: std::result::Result<Vec<(Message, String)>, rusqlite::Error> = (|| {
                let mut st = c.prepare(
                    "SELECT m.id,m.conversation_id,m.run_id,m.from_id,m.to_json,m.kind,m.reply_to,m.thread_id,m.body,m.meta,m.created_at,c.project_id
                     FROM messages_fts f JOIN messages m ON m.rowid=f.rowid JOIN conversations c ON c.id=m.conversation_id WHERE messages_fts MATCH ?1 LIMIT ?2",
                )?;
                let rows = st
                    .query_map(params![q, limit], |r| {
                        Ok((row_to_message(r)?, r.get::<_, String>(11)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })();
            if let Ok(rows) = try_fts {
                return Ok(rows);
            }
            let like = format!("%{q}%");
            let mut st = c.prepare(
                "SELECT m.id,m.conversation_id,m.run_id,m.from_id,m.to_json,m.kind,m.reply_to,m.thread_id,m.body,m.meta,m.created_at,c.project_id
                 FROM messages m JOIN conversations c ON c.id=m.conversation_id WHERE m.body LIKE ?1 LIMIT ?2",
            )?;
            let rows = st
                .query_map(params![like, limit], |r| {
                    Ok((row_to_message(r)?, r.get::<_, String>(11)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    // -- archive: sources + artifacts (global search) ---------------------
    pub fn save_source(
        &self,
        id: &str,
        project_id: &str,
        url: &str,
        title: &str,
        content_md: &str,
    ) -> Result<Source> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO sources (id,project_id,url,title,content_md) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET project_id=excluded.project_id,url=excluded.url,title=excluded.title,content_md=excluded.content_md",
                params![id, project_id, url, title, content_md],
            )?;
            c.query_row(
                "SELECT id,project_id,url,title,content_md FROM sources WHERE id=?1",
                params![id],
                |r| {
                    Ok(Source {
                        id: r.get(0)?,
                        project_id: r.get(1)?,
                        url: r.get(2)?,
                        title: r.get(3)?,
                        content_md: r.get(4)?,
                    })
                },
            )
            .map_err(StoreError::from)
        })
    }

    pub fn search_sources(&self, query: &str, limit: i64) -> Result<Vec<Source>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        self.with_reader(|c| {
            let try_fts: std::result::Result<Vec<Source>, rusqlite::Error> = (|| {
                let mut st = c.prepare(
                    "SELECT s.id,s.project_id,s.url,s.title,s.content_md FROM sources_fts f JOIN sources s ON s.rowid=f.rowid WHERE sources_fts MATCH ?1 LIMIT ?2",
                )?;
                let rows: Vec<Source> = st
                    .query_map(params![q, limit], row_to_source)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })();
            if let Ok(rows) = try_fts {
                return Ok(rows);
            }
            let like = format!("%{q}%");
            let mut st = c.prepare(
                "SELECT id,project_id,url,title,content_md FROM sources WHERE title LIKE ?1 OR content_md LIKE ?1 LIMIT ?2",
            )?;
            let rows: Vec<Source> = st
                .query_map(params![like, limit], row_to_source)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    pub fn save_artifact(&self, id: &str, project_id: &str, title: &str, kind: &str) -> Result<()> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO artifacts (id,project_id,title,kind) VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET project_id=excluded.project_id,title=excluded.title,kind=excluded.kind",
                params![id, project_id, title, kind],
            )?;
            Ok(())
        })
    }

    pub fn save_artifact_version(
        &self,
        artifact_id: &str,
        version: i64,
        body_md: &str,
        changelog: &str,
    ) -> Result<()> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO artifact_versions (artifact_id,version,body_md,changelog) VALUES (?1,?2,?3,?4) ON CONFLICT(artifact_id,version) DO UPDATE SET body_md=excluded.body_md,changelog=excluded.changelog",
                params![artifact_id, version, body_md, changelog],
            )?;
            c.execute(
                "UPDATE artifacts SET current_version=?1 WHERE id=?2",
                params![version, artifact_id],
            )?;
            Ok(())
        })
    }

    pub fn search_artifacts(&self, query: &str, limit: i64) -> Result<Vec<ArtifactHit>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        self.with_reader(|c| {
            let try_fts: std::result::Result<Vec<ArtifactHit>, rusqlite::Error> = (|| {
                let mut st = c.prepare(
                    "SELECT a.id,a.project_id,a.title,v.version,v.body_md FROM artifact_versions_fts f JOIN artifact_versions v ON v.rowid=f.rowid JOIN artifacts a ON a.id=v.artifact_id WHERE artifact_versions_fts MATCH ?1 LIMIT ?2",
                )?;
                let rows: Vec<ArtifactHit> = st
                    .query_map(params![q, limit], row_to_artifact_hit)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })();
            if let Ok(rows) = try_fts {
                return Ok(rows);
            }
            let like = format!("%{q}%");
            let mut st = c.prepare(
                "SELECT a.id,a.project_id,a.title,v.version,v.body_md FROM artifact_versions v JOIN artifacts a ON a.id=v.artifact_id WHERE v.body_md LIKE ?1 OR a.title LIKE ?1 LIMIT ?2",
            )?;
            let rows: Vec<ArtifactHit> = st
                .query_map(params![like, limit], row_to_artifact_hit)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    // -- tool calls (audit log) -------------------------------------------
    pub fn record_tool_call(&self, t: &NewToolCall) -> Result<ToolCall> {
        self.with_writer(|c| {
            // Audit rows are standalone (no chat message); the schema FK on
            // message_id would reject them, so skip FK checks for this insert.
            let _ = c.execute_batch("PRAGMA foreign_keys=OFF;");
            let res = c.execute(
                "INSERT INTO tool_calls (id,message_id,agent_id,tool,args,status,result_summary,approved_by) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![t.id, t.message_id, t.agent_id, t.tool, t.args, t.status, t.result_summary, t.approved_by],
            );
            let _ = c.execute_batch("PRAGMA foreign_keys=ON;");
            res?;
            get_tool_call_inner(c, &t.id)
        })
    }

    pub fn update_tool_call(
        &self,
        id: &str,
        status: &str,
        result_summary: &str,
        approved_by: &str,
    ) -> Result<ToolCall> {
        self.with_writer(|c| {
            let n = c.execute(
                "UPDATE tool_calls SET status=?1, result_summary=?2, approved_by=?3 WHERE id=?4",
                params![status, result_summary, approved_by, id],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound("tool_call", id.into()));
            }
            get_tool_call_inner(c, id)
        })
    }

    pub fn list_tool_calls(&self, limit: i64, offset: i64) -> Result<Vec<ToolCall>> {
        self.with_reader(|c| {
            let mut st = c.prepare(
                "SELECT id,message_id,agent_id,tool,args,status,result_summary,approved_by,created_at FROM tool_calls ORDER BY created_at, rowid LIMIT ?1 OFFSET ?2",
            )?;
            let rows: Vec<_> = st
                .query_map(params![limit, offset], row_to_tool_call)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    // -- skills -----------------------------------------------------------
    pub fn create_skill(&self, s: &NewSkill) -> Result<Skill> {
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO skills (id,name,description,body,license,repo,sha,path,local_path) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![s.id, s.name, s.description, s.body, s.license, s.repo, s.sha, s.path, s.local_path],
            )?;
            get_skill_inner(c, &s.id)
        })
    }

    pub fn get_skill(&self, id: &str) -> Result<Skill> {
        self.with_reader(|c| get_skill_inner(c, id))
    }

    pub fn get_skill_by_name(&self, name: &str) -> Result<Skill> {
        self.with_reader(|c| {
            c.query_row(
                "SELECT id,name,description,body,license,repo,sha,path,local_path,updated_at,created_at FROM skills WHERE name=?1",
                params![name],
                row_to_skill,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound("skill", name.into()))
        })
    }

    pub fn list_skills(&self) -> Result<Vec<Skill>> {
        self.with_reader(|c| {
            let mut st = c.prepare("SELECT id,name,description,body,license,repo,sha,path,local_path,updated_at,created_at FROM skills ORDER BY name")?;
            let rows: Vec<_> = st.query_map([], row_to_skill)?.collect::<std::result::Result<Vec<_>, _>>()?; Ok(rows)
        })
    }

    pub fn update_skill_body(
        &self,
        id: &str,
        description: &str,
        body: &str,
        sha: &str,
    ) -> Result<Skill> {
        self.with_writer(|c| {
            let n = c.execute(
                "UPDATE skills SET description=?1, body=?2, sha=?3, updated_at=CAST(strftime('%s','now') AS INTEGER) WHERE id=?4",
                params![description, body, sha, id],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound("skill", id.into()));
            }
            get_skill_inner(c, id)
        })
    }

    pub fn delete_skill(&self, id: &str) -> Result<()> {
        self.with_writer(|c| {
            c.execute("DELETE FROM skills WHERE id=?1", params![id])?;
            Ok(())
        })
    }

    /// Set enablement of a skill at a scope (`global`/`project`/`agent`).
    pub fn set_skill_enabled(
        &self,
        skill_id: &str,
        scope: &str,
        scope_id: &str,
        enabled: bool,
    ) -> Result<()> {
        if !matches!(scope, "global" | "project" | "agent") {
            return Err(StoreError::NotFound("skill scope", scope.into()));
        }
        let scope_id = if scope == "global" { "" } else { scope_id };
        self.with_writer(|c| {
            get_skill_inner(c, skill_id)?;
            c.execute(
                "INSERT INTO skill_states (skill_id, scope, scope_id, enabled) VALUES (?1,?2,?3,?4) ON CONFLICT(skill_id,scope,scope_id) DO UPDATE SET enabled=excluded.enabled",
                params![skill_id, scope, scope_id, enabled as i32],
            )?;
            Ok(())
        })
    }

    /// Raw enablement rows for one skill: `(scope, scope_id, enabled)`.
    pub fn skill_states(&self, skill_id: &str) -> Result<Vec<(String, String, bool)>> {
        self.with_reader(|c| {
            let mut st = c.prepare(
                "SELECT scope, scope_id, enabled FROM skill_states WHERE skill_id=?1 ORDER BY scope, scope_id",
            )?;
            let rows: Vec<_> = st
                .query_map(params![skill_id], |r| {
                    let scope: String = r.get(0)?;
                    let scope_id: String = r.get(1)?;
                    let enabled: i64 = r.get(2)?;
                    Ok((scope, scope_id, enabled != 0))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Skills enabled for an agent in a project: any matching `global`,
    /// `project(project_id)` or `agent(agent_id)` row with `enabled=1`.
    /// Rows explicitly set to `enabled=0` at any matching scope hide the skill.
    pub fn enabled_skills(&self, project_id: &str, agent_id: &str) -> Result<Vec<Skill>> {
        self.with_reader(|c| {
            let mut st = c.prepare(
                "SELECT s.id,s.name,s.description,s.body,s.license,s.repo,s.sha,s.path,s.local_path,s.updated_at,s.created_at
                 FROM skills s JOIN skill_states st ON st.skill_id=s.id AND st.enabled=1
                 WHERE (st.scope='global') OR (st.scope='project' AND st.scope_id=?1) OR (st.scope='agent' AND st.scope_id=?2)
                 GROUP BY s.id ORDER BY s.name",
            )?;
            // Negative overrides: exclude skills disabled at any visible scope.
            let mut st2 = c.prepare(
                "SELECT skill_id FROM skill_states WHERE enabled=0 AND ((scope='global') OR (scope='project' AND scope_id=?1) OR (scope='agent' AND scope_id=?2))",
            )?;
            let hidden: std::collections::HashSet<String> = st2
                .query_map(params![project_id, agent_id], |r| r.get(0))?
                .collect::<std::result::Result<Vec<String>, _>>()?
                .into_iter()
                .collect();
            let rows: Vec<Skill> = st
                .query_map(params![project_id, agent_id], row_to_skill)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows.into_iter().filter(|s| !hidden.contains(&s.id)).collect())
        })
    }

    // -- memories (docs/MEMORY.md §1-2) ----------------------------------
    // Scopes: user / project / agent / digest. `scope_id` is "" for the
    // user profile, the project id, the agent id, or the run id for digests.
    fn check_memory_scope(scope: &str) -> Result<()> {
        if matches!(scope, "user" | "project" | "agent" | "digest") {
            Ok(())
        } else {
            Err(StoreError::NotFound("memory scope", scope.into()))
        }
    }

    pub fn save_memory(&self, m: &NewMemory) -> Result<Memory> {
        Self::check_memory_scope(&m.scope)?;
        let text = m.text.trim();
        if text.is_empty() {
            return Err(StoreError::NotFound("memory", "empty text".into()));
        }
        let id = if m.id.trim().is_empty() {
            crate::models::new_id("mem")
        } else {
            m.id.clone()
        };
        self.with_writer(|c| {
            c.execute(
                "INSERT INTO memories (id, scope, scope_id, kind, text, source_message_id, created_by, valid_from, confidence, pinned, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,CAST(strftime('%s','now') AS INTEGER),?8,?9,CAST(strftime('%s','now') AS INTEGER),CAST(strftime('%s','now') AS INTEGER))",
                params![id, m.scope, m.scope_id, m.kind, text, m.source_message_id, m.created_by, m.confidence, m.pinned as i32],
            )?;
            get_memory_inner(c, &id)
        })
    }

    pub fn get_memory(&self, id: &str) -> Result<Memory> {
        self.with_reader(|c| get_memory_inner(c, id))
    }

    /// Active memories for one scope (superseded rows excluded unless
    /// `include_superseded`); pinned first, then newest.
    pub fn list_memories(
        &self,
        scope: &str,
        scope_id: &str,
        include_superseded: bool,
    ) -> Result<Vec<Memory>> {
        Self::check_memory_scope(scope)?;
        self.with_reader(|c| {
            let sql = if include_superseded {
                "SELECT id,scope,scope_id,kind,text,source_message_id,created_by,valid_from,invalid_at,confidence,pinned,created_at,updated_at FROM memories WHERE scope=?1 AND scope_id=?2 ORDER BY pinned DESC, created_at, rowid"
            } else {
                "SELECT id,scope,scope_id,kind,text,source_message_id,created_by,valid_from,invalid_at,confidence,pinned,created_at,updated_at FROM memories WHERE scope=?1 AND scope_id=?2 AND invalid_at IS NULL ORDER BY pinned DESC, created_at, rowid"
            };
            let mut st = c.prepare(sql)?;
            let rows: Vec<_> = st
                .query_map(params![scope, scope_id], row_to_memory)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Full supersession history for a scope: every row including superseded
    /// ones, oldest first (MEMORY.md §4: history of superseded versions).
    pub fn memory_history(&self, scope: &str, scope_id: &str) -> Result<Vec<Memory>> {
        self.list_memories(scope, scope_id, true)
    }

    /// Hybrid recall: FTS5 match over active memories (optionally scoped),
    /// falling back to LIKE when the FTS query is empty/invalid.
    pub fn search_memories(
        &self,
        query: &str,
        scope: Option<(&str, &str)>,
        limit: i64,
    ) -> Result<Vec<Memory>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        self.with_reader(|c| {
            let like = format!("%{q}%");
            let (sql, scoped) = match scope {
                Some(_) => (
                    "SELECT m.id,m.scope,m.scope_id,m.kind,m.text,m.source_message_id,m.created_by,m.valid_from,m.invalid_at,m.confidence,m.pinned,m.created_at,m.updated_at FROM memories_fts f JOIN memories m ON m.rowid=f.rowid WHERE m.invalid_at IS NULL AND m.scope=?1 AND m.scope_id=?2 AND memories_fts MATCH ?3 LIMIT ?4",
                    true,
                ),
                None => (
                    "SELECT m.id,m.scope,m.scope_id,m.kind,m.text,m.source_message_id,m.created_by,m.valid_from,m.invalid_at,m.confidence,m.pinned,m.created_at,m.updated_at FROM memories_fts f JOIN memories m ON m.rowid=f.rowid WHERE m.invalid_at IS NULL AND memories_fts MATCH ?1 LIMIT ?2",
                    false,
                ),
            };
            let try_fts: std::result::Result<Vec<Memory>, rusqlite::Error> = (|| {
                let mut st = c.prepare(sql)?;
                let rows = if scoped {
                    let (s, sid) = scope.unwrap();
                    st.query_map(params![s, sid, q, limit], row_to_memory)?
                        .collect::<std::result::Result<Vec<_>, _>>()?
                } else {
                    st.query_map(params![q, limit], row_to_memory)?
                        .collect::<std::result::Result<Vec<_>, _>>()?
                };
                Ok(rows)
            })();
            if let Ok(rows) = try_fts {
                return Ok(rows);
            }
            // Fallback for queries FTS5 cannot parse (e.g. bare "\"").
            let (fsql, scoped2) = match scope {
                Some(_) => (
                    "SELECT id,scope,scope_id,kind,text,source_message_id,created_by,valid_from,invalid_at,confidence,pinned,created_at,updated_at FROM memories WHERE invalid_at IS NULL AND scope=?1 AND scope_id=?2 AND text LIKE ?3 LIMIT ?4",
                    true,
                ),
                None => (
                    "SELECT id,scope,scope_id,kind,text,source_message_id,created_by,valid_from,invalid_at,confidence,pinned,created_at,updated_at FROM memories WHERE invalid_at IS NULL AND text LIKE ?1 LIMIT ?2",
                    false,
                ),
            };
            let mut st = c.prepare(fsql)?;
            let rows = if scoped2 {
                let (s, sid) = scope.unwrap();
                st.query_map(params![s, sid, like, limit], row_to_memory)?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            } else {
                st.query_map(params![like, limit], row_to_memory)?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            };
            Ok(rows)
        })
    }

    /// Supersede a memory (graphiti-style, MEMORY.md §2): the old row gets
    /// `invalid_at` and a new active row carries the updated text, keeping
    /// provenance of the replacement.
    pub fn supersede_memory(&self, id: &str, new_text: &str, created_by: &str) -> Result<Memory> {
        let text = new_text.trim();
        if text.is_empty() {
            return Err(StoreError::NotFound("memory", "empty text".into()));
        }
        self.with_writer(|c| {
            let old = get_memory_inner(c, id)?;
            if old.invalid_at.is_some() {
                return Err(StoreError::NotFound("memory", id.into()));
            }
            let now: i64 = c.query_row("SELECT CAST(strftime('%s','now') AS INTEGER)", [], |r| r.get(0))?;
            c.execute(
                "UPDATE memories SET invalid_at=?1 WHERE id=?2",
                params![now, id],
            )?;
            let new_id = crate::models::new_id("mem");
            c.execute(
                "INSERT INTO memories (id, scope, scope_id, kind, text, source_message_id, created_by, valid_from, confidence, pinned, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,CAST(strftime('%s','now') AS INTEGER),?8,?9,CAST(strftime('%s','now') AS INTEGER),CAST(strftime('%s','now') AS INTEGER))",
                params![new_id, old.scope, old.scope_id, old.kind, text, old.source_message_id, created_by, old.confidence, old.pinned as i32],
            )?;
            get_memory_inner(c, &new_id)
        })
    }

    pub fn pin_memory(&self, id: &str, pinned: bool) -> Result<Memory> {
        self.with_writer(|c| {
            let n = c.execute(
                "UPDATE memories SET pinned=?1 WHERE id=?2",
                params![pinned as i32, id],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound("memory", id.into()));
            }
            get_memory_inner(c, id)
        })
    }

    pub fn update_memory_text(&self, id: &str, text: &str) -> Result<Memory> {
        let text = text.trim();
        if text.is_empty() {
            return Err(StoreError::NotFound("memory", "empty text".into()));
        }
        self.with_writer(|c| {
            let n = c.execute("UPDATE memories SET text=?1 WHERE id=?2", params![text, id])?;
            if n == 0 {
                return Err(StoreError::NotFound("memory", id.into()));
            }
            get_memory_inner(c, id)
        })
    }

    /// Explicit forget (MEMORY.md §4): physical delete of one row.
    pub fn delete_memory(&self, id: &str) -> Result<()> {
        self.with_writer(|c| {
            c.execute("DELETE FROM memories WHERE id=?1", params![id])?;
            Ok(())
        })
    }

    /// Forget a whole scope ("forget everything from this run" / clear tab).
    /// Returns the number of rows removed.
    pub fn clear_scope(&self, scope: &str, scope_id: &str) -> Result<usize> {
        Self::check_memory_scope(scope)?;
        self.with_writer(|c| {
            let n = c.execute(
                "DELETE FROM memories WHERE scope=?1 AND scope_id=?2",
                params![scope, scope_id],
            )?;
            Ok(n)
        })
    }
}

fn row_to_source(r: &rusqlite::Row<'_>) -> rusqlite::Result<Source> {
    Ok(Source {
        id: r.get(0)?,
        project_id: r.get(1)?,
        url: r.get(2)?,
        title: r.get(3)?,
        content_md: r.get(4)?,
    })
}

fn row_to_artifact_hit(r: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactHit> {
    Ok(ArtifactHit {
        artifact_id: r.get(0)?,
        project_id: r.get(1)?,
        title: r.get(2)?,
        version: r.get(3)?,
        body_md: r.get(4)?,
    })
}

fn open_conn(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

fn row_to_project(r: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        goal: r.get(2)?,
        folder: r.get(3)?,
        lead_agent_id: r.get(4)?,
        template: r.get(5)?,
        created_at: r.get(6)?,
        archived: r.get::<_, i64>(7)? != 0,
    })
}

fn row_to_agent(r: &rusqlite::Row<'_>) -> rusqlite::Result<Agent> {
    Ok(Agent {
        id: r.get(0)?,
        name: r.get(1)?,
        role: r.get(2)?,
        instructions: r.get(3)?,
        backend: r.get(4)?,
        avatar: r.get(5)?,
        tools: r.get(6)?,
        permissions: r.get(7)?,
        persona_source: r.get(8)?,
        created_at: r.get(9)?,
    })
}

fn row_to_tool_call(r: &rusqlite::Row<'_>) -> rusqlite::Result<ToolCall> {
    Ok(ToolCall {
        id: r.get(0)?,
        message_id: r.get(1)?,
        agent_id: r.get(2)?,
        tool: r.get(3)?,
        args: r.get(4)?,
        status: r.get(5)?,
        result_summary: r.get(6)?,
        approved_by: r.get(7)?,
        created_at: r.get(8)?,
    })
}

fn get_tool_call_inner(c: &Connection, id: &str) -> Result<ToolCall> {
    c.query_row(
        "SELECT id,message_id,agent_id,tool,args,status,result_summary,approved_by,created_at FROM tool_calls WHERE id=?1",
        params![id],
        row_to_tool_call,
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound("tool_call", id.into()))
}

fn row_to_skill(r: &rusqlite::Row<'_>) -> rusqlite::Result<Skill> {
    Ok(Skill {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        body: r.get(3)?,
        license: r.get(4)?,
        repo: r.get(5)?,
        sha: r.get(6)?,
        path: r.get(7)?,
        local_path: r.get(8)?,
        updated_at: r.get(9)?,
        created_at: r.get(10)?,
    })
}

fn get_skill_inner(c: &Connection, id: &str) -> Result<Skill> {
    c.query_row(
        "SELECT id,name,description,body,license,repo,sha,path,local_path,updated_at,created_at FROM skills WHERE id=?1",
        params![id],
        row_to_skill,
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound("skill", id.into()))
}

fn row_to_memory(r: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: r.get(0)?,
        scope: r.get(1)?,
        scope_id: r.get(2)?,
        kind: r.get(3)?,
        text: r.get(4)?,
        source_message_id: r.get(5)?,
        created_by: r.get(6)?,
        valid_from: r.get(7)?,
        invalid_at: r.get(8)?,
        confidence: r.get(9)?,
        pinned: r.get::<_, i64>(10)? != 0,
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
    })
}

fn get_memory_inner(c: &Connection, id: &str) -> Result<Memory> {
    c.query_row(
            "SELECT id,scope,scope_id,kind,text,source_message_id,created_by,valid_from,invalid_at,confidence,pinned,created_at,updated_at FROM memories WHERE id=?1",
            params![id],
            row_to_memory,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound("memory", id.into()))
}

fn row_to_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get(0)?,
        conversation_id: r.get(1)?,
        run_id: r.get(2)?,
        from_id: r.get(3)?,
        to_json: r.get(4)?,
        kind: r.get(5)?,
        reply_to: r.get(6)?,
        thread_id: r.get(7)?,
        body: r.get(8)?,
        meta: r.get(9)?,
        created_at: r.get(10)?,
    })
}

fn get_project_inner(c: &Connection, id: &str) -> Result<Project> {
    c.query_row(
        "SELECT id,name,goal,folder,lead_agent_id,template,created_at,archived FROM projects WHERE id=?1",
        params![id], row_to_project,
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound("project", id.into()))
}

fn get_agent_inner(c: &Connection, id: &str) -> Result<Agent> {
    c.query_row(
        "SELECT id,name,role,instructions,backend,avatar,tools,permissions,persona_source,created_at FROM agents WHERE id=?1",
        params![id], row_to_agent,
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound("agent", id.into()))
}

fn get_message_inner(c: &Connection, id: &str) -> Result<Message> {
    c.query_row(
        "SELECT id,conversation_id,run_id,from_id,to_json,kind,reply_to,thread_id,body,meta,created_at FROM messages WHERE id=?1",
        params![id], row_to_message,
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound("message", id.into()))
}

#[cfg(test)]
mod memories_tests {
    use super::*;
    use crate::models::NewMemory;

    fn mem(id: &str, scope: &str, scope_id: &str, text: &str) -> NewMemory {
        NewMemory {
            id: id.into(),
            scope: scope.into(),
            scope_id: scope_id.into(),
            kind: "fact".into(),
            text: text.into(),
            source_message_id: "m1".into(),
            created_by: "user".into(),
            confidence: 0.9,
            pinned: false,
        }
    }

    #[test]
    fn memories_save_list_search_with_provenance() {
        let s = Store::open_in_memory().unwrap();
        let m = s
            .save_memory(&mem("mem1", "project", "p1", "Scholar search works best"))
            .unwrap();
        assert_eq!(m.scope, "project");
        assert_eq!(m.source_message_id, "m1");
        assert!(m.invalid_at.is_none());
        s.save_memory(&mem("mem2", "project", "p1", "Prefers sources from 2025"))
            .unwrap();
        s.save_memory(&mem("mem3", "user", "", "Works in fintech"))
            .unwrap();
        let list = s.list_memories("project", "p1", false).unwrap();
        assert_eq!(list.len(), 2);
        // Scopes do not leak: project list has no user memories.
        assert!(list.iter().all(|m| m.scope == "project"));
        let hits = s
            .search_memories("Scholar", Some(("project", "p1")), 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "mem1");
    }

    #[test]
    fn memories_scopes_user_agent_digest_are_isolated() {
        let s = Store::open_in_memory().unwrap();
        s.save_memory(&mem("u1", "user", "", "user fact")).unwrap();
        s.save_memory(&mem("a1", "agent", "ada", "agent note"))
            .unwrap();
        s.save_memory(&mem("d1", "digest", "run1", "run digest summary"))
            .unwrap();
        assert_eq!(s.list_memories("user", "", false).unwrap().len(), 1);
        assert_eq!(s.list_memories("agent", "ada", false).unwrap().len(), 1);
        assert_eq!(s.list_memories("digest", "run1", false).unwrap().len(), 1);
        assert!(s.list_memories("project", "p1", false).unwrap().is_empty());
    }

    #[test]
    fn memories_supersede_keeps_history_and_hides_old_from_search() {
        let s = Store::open_in_memory().unwrap();
        s.save_memory(&mem("m1", "project", "p1", "Old conclusion"))
            .unwrap();
        let next = s
            .supersede_memory("m1", "New conclusion", "agent:ada")
            .unwrap();
        assert_eq!(next.text, "New conclusion");
        assert_eq!(next.created_by, "agent:ada");
        // Old row invalidated, excluded from active list and search.
        let active = s.list_memories("project", "p1", false).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, next.id);
        let history = s.memory_history("project", "p1").unwrap();
        assert_eq!(history.len(), 2);
        let old = s.get_memory("m1").unwrap();
        assert!(old.invalid_at.is_some());
        assert!(s
            .search_memories("Old conclusion", Some(("project", "p1")), 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            s.search_memories("New conclusion", Some(("project", "p1")), 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn memories_pin_delete_and_clear_scope() {
        let s = Store::open_in_memory().unwrap();
        s.save_memory(&mem("m1", "project", "p1", "Pinned fact"))
            .unwrap();
        s.save_memory(&mem("m2", "project", "p1", "Other fact"))
            .unwrap();
        let pinned = s.pin_memory("m1", true).unwrap();
        assert!(pinned.pinned);
        // Pinned memories sort first.
        let list = s.list_memories("project", "p1", false).unwrap();
        assert_eq!(list[0].id, "m1");
        s.delete_memory("m2").unwrap();
        assert_eq!(s.list_memories("project", "p1", false).unwrap().len(), 1);
        let removed = s.clear_scope("project", "p1").unwrap();
        assert_eq!(removed, 1);
        assert!(s.list_memories("project", "p1", true).unwrap().is_empty());
    }
}
