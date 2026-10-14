//! Synapse as an MCP server (see docs/PRD.md §5.5, docs/MILESTONES.md M6.8).
//!
//! Opt-in local server exposing the documented team tools over stdio (for
//! Claude Desktop / Cursor / Zed configs) and over localhost streamable HTTP
//! guarded by a local bearer token. The token lives only in the
//! [`SecretStore`](crate::oauth::SecretStore); configs and logs hold just
//! the `key_ref`.
//!
//! This module is transport-independent on purpose: [`SynapseServer`] is the
//! in-process implementation that the real stdio/HTTP transports drive, and
//! that tests verify calls against. [`client_snippets`] renders the
//! copy-paste configs Settings shows.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

/// Tools Synapse exposes to other apps. This list is the documented API;
/// clients must not rely on any other tool name.
pub const SERVER_TOOL_NAMES: &[&str] = &[
    "list_projects",
    "start_run",
    "run_status",
    "get_report",
    "ask_agent",
];

/// Default localhost HTTP port for the opt-in server.
pub const DEFAULT_HTTP_PORT: u16 = 4317;

/// `key_ref` under which the local HTTP token is kept in the secret store.
pub const LOCAL_TOKEN_KEY_REF: &str = "synapse-local-mcp-token";

/// Binary clients spawn for the stdio transport.
pub const STDIO_BINARY: &str = "synapse-mcp-server";

/// One documented tool with its JSON-schema parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerToolDoc {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// All server tools require an approval-free read or create agent-visible
    /// artefacts only; none touches secrets.
    pub read_only: bool,
}

/// JSON-schema descriptions of the documented tools.
pub fn server_tool_docs() -> Vec<ServerToolDoc> {
    fn schema(props: &[(&str, &str)], required: &[&str]) -> Value {
        let properties: serde_json::Map<String, Value> = props
            .iter()
            .map(|(name, desc)| {
                (
                    name.to_string(),
                    json!({"type": "string", "description": desc}),
                )
            })
            .collect();
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    }
    vec![
        ServerToolDoc {
            name: "list_projects".into(),
            description: "List Synapse projects (id, name, goal).".into(),
            input_schema: schema(&[], &[]),
            read_only: true,
        },
        ServerToolDoc {
            name: "start_run".into(),
            description: "Start a team run for a project (mode + prompt).".into(),
            input_schema: schema(
                &[
                    ("project_id", "Project id from list_projects"),
                    (
                        "mode",
                        "Run mode: brainstorm, research, analysis, revision, chat",
                    ),
                    ("prompt", "The goal/question for the team"),
                ],
                &["project_id", "mode", "prompt"],
            ),
            read_only: false,
        },
        ServerToolDoc {
            name: "run_status".into(),
            description: "Status of a run started with start_run.".into(),
            input_schema: schema(&[("run_id", "Run id from start_run")], &["run_id"]),
            read_only: true,
        },
        ServerToolDoc {
            name: "get_report".into(),
            description: "Current report (markdown) for a project.".into(),
            input_schema: schema(
                &[("project_id", "Project id from list_projects")],
                &["project_id"],
            ),
            read_only: true,
        },
        ServerToolDoc {
            name: "ask_agent".into(),
            description: "Ask one agent a question and get its reply.".into(),
            input_schema: schema(
                &[
                    ("agent", "Agent id"),
                    ("question", "The question for the agent"),
                ],
                &["agent", "question"],
            ),
            read_only: false,
        },
    ]
}

/// Opt-in settings for the local server. Off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynapseServerSettings {
    pub enabled: bool,
    pub port: u16,
    pub token_key_ref: String,
}

impl Default for SynapseServerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_HTTP_PORT,
            token_key_ref: LOCAL_TOKEN_KEY_REF.into(),
        }
    }
}

/// Generate a fresh 256-bit local token (`syn-` + 64 hex chars).
pub fn generate_local_token() -> SecretString {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("rng unavailable");
    SecretString::from(format!("syn-{}", hex::encode(bytes)))
}

/// Masked preview for UI (`syn-…a1b2`), never the full value.
pub fn token_preview(token: &SecretString) -> String {
    let v = token.expose_secret();
    if v.len() <= 8 {
        return "syn-…".into();
    }
    format!("{}…{}", &v[..4], &v[v.len() - 4..])
}

/// Extract the bearer token from an `Authorization` header value.
pub fn bearer_token(header: Option<&str>) -> Option<String> {
    let header = header?;
    let rest = header.strip_prefix("Bearer ")?;
    let token = rest.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// Constant-time bearer check for the localhost HTTP transport.
pub fn verify_bearer(header: Option<&str>, expected: &SecretString) -> bool {
    let Some(presented) = bearer_token(header) else {
        return false;
    };
    let a = presented.as_bytes();
    let b = expected.expose_secret().as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Which client a snippet is for (Settings shows all three).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    ClaudeDesktop,
    Cursor,
    Zed,
}

impl ClientKind {
    pub fn label(&self) -> &'static str {
        match self {
            ClientKind::ClaudeDesktop => "Claude Desktop",
            ClientKind::Cursor => "Cursor",
            ClientKind::Zed => "Zed",
        }
    }
}

/// Copyable client configuration snippets.
///
/// Returns `(client label, stdio JSON, http JSON)`. The stdio snippet spawns
/// the local binary (no token needed: the OS user already owns the process);
/// the HTTP snippet targets the localhost server and carries the token in an
/// `Authorization` header — keep the placeholder replaced with the real token
/// shown once in Settings.
pub fn client_snippets(binary: &str, port: u16) -> Vec<(String, String, String)> {
    let stdio = |extra: &str| {
        format!(
            concat!(
                "{{\n  \"mcpServers\": {{\n",
                "    \"synapse\": {{\n",
                "      \"command\": \"{bin}\",\n",
                "      \"args\": [\"{args}\"]\n",
                "    }}\n  }}\n}}"
            ),
            bin = binary,
            args = extra
        )
    };
    let http_url = format!("http://127.0.0.1:{port}/mcp");
    let http = |client: &str| {
        match client {
        "zed" => format!(
            "{{\n  \"context_servers\": {{\n    \"synapse\": {{\n      \"source\": \"custom\",\n      \"url\": \"{url}\",\n      \"headers\": [{{\"name\": \"Authorization\", \"value\": \"Bearer <TOKEN>\"}}]\n    }}\n  }}\n}}",
            url = http_url
        ),
        _ => format!(
            "{{\n  \"mcpServers\": {{\n    \"synapse\": {{\n      \"url\": \"{url}\",\n      \"headers\": {{\"Authorization\": \"Bearer <TOKEN>\"}}\n    }}\n  }}\n}}",
            url = http_url
        ),
    }
    };
    vec![
        (
            ClientKind::ClaudeDesktop.label().into(),
            stdio("serve --stdio"),
            http("claude"),
        ),
        (
            ClientKind::Cursor.label().into(),
            stdio("serve --stdio"),
            http("cursor"),
        ),
        (
            ClientKind::Zed.label().into(),
            stdio("serve --stdio"),
            http("zed"),
        ),
    ]
}

/// Every way a server-side tool call can fail.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ServerError {
    #[error("local MCP server is disabled (opt in via Settings → MCP server)")]
    Disabled,
    #[error("missing or invalid local token")]
    Unauthorized,
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("tool \"{tool}\" needs \"{field}\": {hint}")]
    MissingField {
        tool: String,
        field: String,
        hint: String,
    },
    #[error("unknown project: {0}")]
    UnknownProject(String),
    #[error("unknown run: {0}")]
    UnknownRun(String),
}

#[derive(Debug, Clone)]
struct Project {
    id: String,
    name: String,
    goal: String,
    report_md: String,
}

#[derive(Debug, Clone)]
struct Run {
    id: String,
    project_id: String,
    mode: String,
    prompt: String,
    status: String,
}

#[derive(Debug, Default)]
struct State {
    projects: Vec<Project>,
    runs: HashMap<String, Run>,
    run_counter: u64,
}

/// In-process Synapse MCP server: the documented tools backed by a small
/// in-memory model. The app wires this to real stores/transports; tests
/// verify calls against it directly.
#[derive(Debug, Clone)]
pub struct SynapseServer {
    inner: Arc<Mutex<State>>,
    settings: Arc<Mutex<SynapseServerSettings>>,
    token: Arc<Mutex<Option<SecretString>>>,
}

impl Default for SynapseServer {
    fn default() -> Self {
        Self::new(SynapseServerSettings::default())
    }
}

impl SynapseServer {
    pub fn new(settings: SynapseServerSettings) -> Self {
        let server = Self {
            inner: Arc::new(Mutex::new(State::default())),
            settings: Arc::new(Mutex::new(settings)),
            token: Arc::new(Mutex::new(None)),
        };
        server.add_project("demo", "Demo project", "Try the team.");
        server
    }

    /// Test helper: seed data lives here, not in the app.
    pub fn add_project(&self, id: &str, name: &str, goal: &str) {
        let mut state = self.inner.lock().expect("poisoned");
        if state.projects.iter().any(|p| p.id == id) {
            return;
        }
        state.projects.push(Project {
            id: id.into(),
            name: name.into(),
            goal: goal.into(),
            report_md: format!("# {name}\n\nNo runs yet.\n"),
        });
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.settings.lock().expect("poisoned").enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.settings.lock().expect("poisoned").enabled
    }

    pub fn set_token(&self, token: SecretString) {
        *self.token.lock().expect("poisoned") = Some(token);
    }

    /// Names + schemas of the documented tools. Fails closed while opted out.
    pub fn list_tools(&self) -> Result<Vec<ServerToolDoc>, ServerError> {
        if !self.is_enabled() {
            return Err(ServerError::Disabled);
        }
        Ok(server_tool_docs())
    }

    fn require_enabled(&self) -> Result<(), ServerError> {
        if self.is_enabled() {
            Ok(())
        } else {
            Err(ServerError::Disabled)
        }
    }

    fn require_auth(&self, auth_header: Option<&str>) -> Result<(), ServerError> {
        let guard = self.token.lock().expect("poisoned");
        match guard.as_ref() {
            Some(expected) => {
                if verify_bearer(auth_header, expected) {
                    Ok(())
                } else {
                    Err(ServerError::Unauthorized)
                }
            }
            None => Err(ServerError::Unauthorized),
        }
    }

    /// Dispatch one tool call (stdio path: process ownership is the auth).
    pub fn call_tool(&self, tool: &str, args: Value) -> Result<Value, ServerError> {
        self.require_enabled()?;
        match tool {
            "list_projects" => Ok(self.list_projects()),
            "start_run" => {
                let project_id = req_str(tool, &args, "project_id", "a project id")?;
                let mode = req_str(tool, &args, "mode", "e.g. research")?;
                let prompt = req_str(tool, &args, "prompt", "the goal for the team")?;
                self.start_run(&project_id, &mode, &prompt)
            }
            "run_status" => {
                let run_id = req_str(tool, &args, "run_id", "a run id")?;
                self.run_status(&run_id)
            }
            "get_report" => {
                let project_id = req_str(tool, &args, "project_id", "a project id")?;
                self.get_report(&project_id)
            }
            "ask_agent" => {
                let agent = req_str(tool, &args, "agent", "an agent id")?;
                let question = req_str(tool, &args, "question", "the question")?;
                Ok(json!({
                    "agent": agent,
                    "question": question,
                    "answer": format!("{agent} says: noted — working on it."),
                }))
            }
            other => Err(ServerError::UnknownTool(other.into())),
        }
    }

    /// Dispatch one tool call for the localhost HTTP transport (token first).
    pub fn call_tool_http(
        &self,
        auth_header: Option<&str>,
        tool: &str,
        args: Value,
    ) -> Result<Value, ServerError> {
        self.require_enabled()?;
        self.require_auth(auth_header)?;
        // Re-check after auth so the disabled path stays identical.
        self.call_tool(tool, args)
    }

    /// Minimal JSON-RPC dispatcher (`tools/list`, `tools/call`) so tests and
    /// smoke checks can verify calls against a test server without a socket.
    pub fn handle_jsonrpc(&self, auth_header: Option<&str>, request: Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "tools/list" => {
                if self.token.lock().expect("poisoned").is_some() {
                    match self.require_auth(auth_header) {
                        Ok(()) => self.list_tools().map(|tools| json!({"tools": tools})),
                        Err(e) => Err(e),
                    }
                } else {
                    self.list_tools().map(|tools| json!({"tools": tools}))
                }
            }
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                if self.token.lock().expect("poisoned").is_some() {
                    self.call_tool_http(auth_header, name, args)
                        .map(|v| json!({"content": [{"type": "text", "text": v.to_string()}], "result": v}))
                } else {
                    self.call_tool(name, args)
                        .map(|v| json!({"content": [{"type": "text", "text": v.to_string()}], "result": v}))
                }
            }
            _ => Err(ServerError::UnknownTool(format!("method {method}"))),
        };
        match result {
            Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
            Err(e) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": e.to_string()}})
            }
        }
    }

    fn list_projects(&self) -> Value {
        let state = self.inner.lock().expect("poisoned");
        let projects: Vec<Value> = state
            .projects
            .iter()
            .map(|p| json!({"id": p.id, "name": p.name, "goal": p.goal}))
            .collect();
        json!({"projects": projects})
    }

    fn start_run(&self, project_id: &str, mode: &str, prompt: &str) -> Result<Value, ServerError> {
        let mut state = self.inner.lock().expect("poisoned");
        if !state.projects.iter().any(|p| p.id == project_id) {
            return Err(ServerError::UnknownProject(project_id.into()));
        }
        state.run_counter += 1;
        let id = format!("run-{}", state.run_counter);
        state.runs.insert(
            id.clone(),
            Run {
                id: id.clone(),
                project_id: project_id.into(),
                mode: mode.into(),
                prompt: prompt.into(),
                status: "running".into(),
            },
        );
        Ok(json!({"run_id": id, "status": "running"}))
    }

    fn run_status(&self, run_id: &str) -> Result<Value, ServerError> {
        let state = self.inner.lock().expect("poisoned");
        match state.runs.get(run_id) {
            Some(run) => Ok(json!({
                "run_id": run.id,
                "project_id": run.project_id,
                "mode": run.mode,
                "prompt": run.prompt,
                "status": run.status,
            })),
            None => Err(ServerError::UnknownRun(run_id.into())),
        }
    }

    fn get_report(&self, project_id: &str) -> Result<Value, ServerError> {
        let state = self.inner.lock().expect("poisoned");
        match state.projects.iter().find(|p| p.id == project_id) {
            Some(p) => Ok(json!({"project_id": p.id, "markdown": p.report_md})),
            None => Err(ServerError::UnknownProject(project_id.into())),
        }
    }
}

fn req_str(tool: &str, args: &Value, field: &str, hint: &str) -> Result<String, ServerError> {
    match args.get(field).and_then(Value::as_str) {
        Some(v) if !v.trim().is_empty() => Ok(v.to_string()),
        _ => Err(ServerError::MissingField {
            tool: tool.into(),
            field: field.into(),
            hint: hint.into(),
        }),
    }
}
