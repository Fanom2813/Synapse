//! Team tools for CLI agents via the in-process MCP server
//! (see docs/TEAM-PROTOCOL.md §2, docs/ARCHITECTURE.md §2).
//!
//! CLI agents run their own tool loop, so Synapse exposes the same typed
//! team tools (`say`, `ask_agent`, `delegate`, `report_finding`, …) to them
//! through MCP: [`TeamMcpServer`] is the transport-independent in-process
//! implementation (backed by [`TeamTools`](synapse_core::TeamTools)) that
//! stdio/HTTP transports drive, and [`team_mcp_server`] builds the
//! `session/new` `mcp_servers` entry pointing at the bundled stdio bridge.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use synapse_core::{team_tools::team_tool_specs, TeamBus, TeamTools, TEAM_TOOL_NAMES};

/// Name under which the team tools appear to CLI agents.
pub const TEAM_MCP_SERVER_NAME: &str = "synapse-team";

/// Binary clients spawn for the team-tools stdio bridge.
pub const TEAM_MCP_STDIO_BINARY: &str = "synapse-team-mcp";

/// One team tool as an MCP tool (`tools/list` entry).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamMcpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

impl From<synapse_core::TeamToolSpec> for TeamMcpTool {
    fn from(spec: synapse_core::TeamToolSpec) -> Self {
        Self {
            name: spec.name,
            description: spec.description,
            input_schema: spec.parameters,
        }
    }
}

/// Every way a team MCP tool call can fail.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TeamMcpError {
    #[error(transparent)]
    Team(#[from] synapse_core::TeamToolError),
    #[error("unknown team tool: {0}")]
    UnknownTool(String),
}

impl TeamMcpError {
    pub fn message(&self) -> String {
        self.to_string()
    }
}

/// In-process MCP server over the documented team tools.
///
/// Wraps [`TeamTools`] for one CLI agent session: `list_tools` serves the
/// function-calling specs as MCP `tools/list` entries, `call_tool` dispatches
/// by name and publishes the typed envelope as a room message, and
/// `handle_jsonrpc` speaks the minimal `tools/list` + `tools/call`
/// JSON-RPC surface transports drive.
#[derive(Clone)]
pub struct TeamMcpServer {
    tools: TeamTools,
}

impl TeamMcpServer {
    /// Bind team tools to one CLI agent's identity + run context.
    pub fn new(bus: &TeamBus, agent_id: &str, run_id: &str, conversation_id: &str) -> Self {
        Self {
            tools: TeamTools::for_agent(bus, agent_id, run_id, conversation_id),
        }
    }

    /// Tool names exposed through this server (== [`TEAM_TOOL_NAMES`]).
    pub fn tool_names(&self) -> Vec<String> {
        TEAM_TOOL_NAMES.iter().map(|s| s.to_string()).collect()
    }

    /// MCP `tools/list` entries for every team tool.
    pub fn list_tools(&self) -> Vec<TeamMcpTool> {
        team_tool_specs()
            .into_iter()
            .map(TeamMcpTool::from)
            .collect()
    }

    /// Dispatch one MCP `tools/call` by team tool name with JSON args.
    ///
    /// Returns the stored envelope as JSON (`message_id`, `to`, `kind`,
    /// `body`) so CLI agents can quote it (ask/answer/challenge flows).
    pub fn call_tool(&self, tool: &str, args: Value) -> Result<Value, TeamMcpError> {
        if !TEAM_TOOL_NAMES.contains(&tool) {
            return Err(TeamMcpError::UnknownTool(tool.into()));
        }
        let args = if args.is_null() { json!({}) } else { args };
        let env = self.tools.call(tool, args)?;
        Ok(json!({
            "message_id": env.id,
            "to": env.to,
            "kind": env.kind,
            "body": env.body,
        }))
    }

    /// Minimal JSON-RPC dispatcher (`tools/list`, `tools/call`) so tests and
    /// transports verify calls without a socket.
    pub fn handle_jsonrpc(&self, request: Value) -> Value {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "tools/list" => {
                json!({"jsonrpc": "2.0", "id": id, "result": {"tools": self.list_tools()}})
            }
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call_tool(name, args) {
                    Ok(v) => json!({"jsonrpc": "2.0", "id": id,
                        "result": {"content": [{"type": "text", "text": v.to_string()}], "result": v}}),
                    Err(e) => json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32602, "message": e.message()}}),
                }
            }
            other => json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("unknown method: {other}")}}),
        }
    }
}

/// Build the `session/new` MCP server entry that gives a CLI agent the team
/// tools over stdio. `bridge` is the executable that serves [`TeamMcpServer`]
/// over stdio (defaults to [`TEAM_MCP_STDIO_BINARY`] on `PATH`); `env` carries
/// the run context (run id, conversation id, agent id) to the bridge.
pub fn team_mcp_server(
    bridge: Option<&Path>,
    env: Vec<(String, String)>,
) -> agent_client_protocol::McpServer {
    let (command, args) = match bridge {
        Some(path) => (path.to_path_buf(), Vec::new()),
        None => (PathBuf::from(TEAM_MCP_STDIO_BINARY), Vec::new()),
    };
    agent_client_protocol::McpServer::Stdio {
        name: TEAM_MCP_SERVER_NAME.into(),
        command,
        args,
        env: env
            .into_iter()
            .map(|(name, value)| agent_client_protocol::EnvVariable {
                name,
                value,
                meta: None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod team_mcp_tests {
    use super::*;
    use serde_json::json;

    fn setup(agent: &str) -> (TeamBus, TeamMcpServer) {
        let bus = TeamBus::new();
        let server = TeamMcpServer::new(&bus, agent, "run1", "c1");
        (bus, server)
    }

    #[test]
    fn team_mcp_lists_every_documented_tool_with_schemas() {
        let (_bus, server) = setup("mira");
        let names = server.tool_names();
        for want in TEAM_TOOL_NAMES {
            assert!(names.contains(&want.to_string()), "missing {want}");
        }
        let tools = server.list_tools();
        assert_eq!(tools.len(), TEAM_TOOL_NAMES.len());
        for tool in &tools {
            assert!(
                !tool.description.is_empty(),
                "{} needs a description",
                tool.name
            );
            assert!(
                tool.input_schema.get("properties").is_some(),
                "{} needs an inputSchema",
                tool.name
            );
        }
        // MCP names track the function-calling specs one to one.
        let mut listed: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        listed.sort_unstable();
        let mut want: Vec<&str> = TEAM_TOOL_NAMES.to_vec();
        want.sort_unstable();
        assert_eq!(listed, want);
    }

    #[test]
    fn team_mcp_call_publishes_typed_room_messages() {
        let (bus, server) = setup("mira");
        let out = server
            .call_tool("say", json!({"text": "hello room"}))
            .expect("say");
        assert_eq!(out["body"], json!("hello room"));
        let id = out["message_id"].as_str().expect("message_id");

        // The envelope is persisted on the bus so the room sees it.
        let hist = bus.history("c1", 0, usize::MAX);
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].id, id);

        // Delegate + finding round trip through the same dispatch.
        let out = server
            .call_tool(
                "delegate",
                json!({"agent": "lead", "task": "review", "deliverable": "notes"}),
            )
            .expect("delegate");
        assert!(out["message_id"].as_str().is_some());
        let out = server
            .call_tool(
                "report_finding",
                json!({"text": "X causes Y.", "confidence": 0.8}),
            )
            .expect("finding");
        assert_eq!(out["body"], json!("X causes Y."));
        assert_eq!(bus.history("c1", 0, usize::MAX).len(), 3);
    }

    #[test]
    fn team_mcp_ask_answer_flows_across_two_servers() {
        let bus = TeamBus::new();
        let lead = TeamMcpServer::new(&bus, "lead", "run1", "c1");
        let mira = TeamMcpServer::new(&bus, "mira", "run1", "c1");
        let ask = lead
            .call_tool(
                "ask_agent",
                json!({"agent": "mira", "question": "Summarise?"}),
            )
            .expect("ask");
        let ask_id = ask["message_id"].as_str().expect("id").to_string();
        let ans = mira
            .call_tool(
                "answer",
                json!({"message_id": ask_id, "text": "It shows X."}),
            )
            .expect("answer");
        assert!(ans["message_id"].as_str().is_some());
        let done = mira
            .call_tool("done", json!({"summary": "answered"}))
            .expect("done");
        assert!(done["body"].as_str().unwrap().contains("answered"));
        assert_eq!(bus.history("c1", 0, usize::MAX).len(), 3);
    }

    #[test]
    fn team_mcp_rejects_unknown_tools_and_blank_fields() {
        let (bus, server) = setup("mira");
        let err = server.call_tool("teleport", json!({})).unwrap_err();
        assert!(matches!(err, TeamMcpError::UnknownTool(_)));
        assert!(server.call_tool("say", json!({"text": "  "})).is_err());
        assert!(server
            .call_tool("ask_agent", json!({"agent": "lead"}))
            .is_err());
        // Failed calls store nothing.
        assert!(bus.history("c1", 0, usize::MAX).is_empty());
    }

    #[test]
    fn team_mcp_jsonrpc_tools_list_and_call() {
        let (bus, server) = setup("mira");
        let list = server.handle_jsonrpc(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {},
        }));
        let tools = list["result"]["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), TEAM_TOOL_NAMES.len());

        let call = server.handle_jsonrpc(json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "say", "arguments": {"text": "via rpc"}},
        }));
        assert_eq!(call["result"]["result"]["body"], json!("via rpc"));
        assert_eq!(bus.history("c1", 0, usize::MAX).len(), 1);

        let bad = server.handle_jsonrpc(json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "nope", "arguments": {}},
        }));
        assert!(bad.get("error").is_some(), "got: {bad}");
        let unknown = server.handle_jsonrpc(json!({
            "jsonrpc": "2.0", "id": 4, "method": "initialize", "params": {},
        }));
        assert!(unknown.get("error").is_some(), "got: {unknown}");
    }

    #[test]
    fn team_mcp_session_entry_advertises_stdio_bridge() {
        let entry = team_mcp_server(None, vec![("SYNAPSE_RUN_ID".into(), "run1".into())]);
        match entry {
            agent_client_protocol::McpServer::Stdio {
                ref name,
                ref command,
                ref env,
                ..
            } => {
                assert_eq!(name, TEAM_MCP_SERVER_NAME);
                assert_eq!(*command, PathBuf::from(TEAM_MCP_STDIO_BINARY));
                assert_eq!(env.len(), 1);
                assert_eq!(env[0].name, "SYNAPSE_RUN_ID");
            }
            other => panic!("team tools must use the stdio transport: {other:?}"),
        }
        // Serialises for `session/new` params.
        let params = serde_json::to_value(agent_client_protocol::NewSessionRequest {
            cwd: PathBuf::from("/tmp"),
            mcp_servers: vec![entry],
            meta: None,
        })
        .expect("params serialise");
        assert!(params["mcpServers"][0].get("name").is_some(), "{params}");
    }
}
