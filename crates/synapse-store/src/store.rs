//! [`Store`]: WAL SQLite file, one writer mutex, small reader pool.
//!
//! Writes take the single writer connection; reads use a pooled connection.
//! Use from async code via `tokio::task::spawn_blocking`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

use crate::models::{Agent, Conversation, Message, NewAgent, NewMessage, NewProject, Project};
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

    pub fn delete_project(&self, id: &str) -> Result<()> {
        self.with_writer(|c| {
            c.execute("DELETE FROM projects WHERE id=?1", params![id])?;
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
