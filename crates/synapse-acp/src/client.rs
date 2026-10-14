//! ACP model backend: spawn a CLI agent over ACP, create sessions, stream
//! updates and map them onto the [`ModelBackend`] interface
//! (see `docs/STACK.md` §CLI agents, `docs/ARCHITECTURE.md` §2).
//!
//! The client speaks line-delimited JSON-RPC over the agent's stdio using the
//! [`agent_client_protocol`] types for params, reusing one long-lived session
//! per connection. Each turn sends `session/prompt`, collects
//! `session/update` notifications into [`AgentEvent`]s and returns the prompt
//! response's stop reason. Cancellation sends `session/cancel`.
//!
//! Token usage has no stable ACP field in the pinned `0.3.0` protocol, so
//! it is read opportunistically from `_meta.usage`
//! (`inputTokens`/`input_tokens` + `outputTokens`/`output_tokens`) on the
//! prompt response; when the agent reports nothing, usage stays zero.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use crate::permissions::{handle_agent_request, PermissionBridge};
use agent_client_protocol as acp;
use serde_json::Value;
use synapse_llm::{
    AgentEvent, LlmError, ModelBackend, Role, ToolCallRequest, TurnOutput, TurnRequest, Usage,
};
use synapse_tools::permissions::Engine;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

// -- config / errors ---------------------------------------------------------

/// How to spawn an ACP agent (see `detect::AvailableAdapter`).
#[derive(Debug, Clone)]
pub struct AcpConfig {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

impl AcpConfig {
    pub fn new(program: impl Into<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: Vec::new(),
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }
}

/// Failures of the ACP backend. Mapped to [`LlmError`] at the trait boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AcpError {
    #[error("could not spawn ACP agent `{0}`: {1}")]
    Spawn(String, String),
    #[error("ACP agent closed its stdio: {0}")]
    Closed(String),
    #[error("ACP protocol error: {0}")]
    Protocol(String),
    #[error("ACP agent reported an error: {0}")]
    Agent(String),
    #[error("ACP request timed out: {0}")]
    Timeout(String),
    #[error("turn cancelled")]
    Cancelled,
}

impl AcpError {
    fn short(s: impl Into<String>) -> String {
        let s = s.into();
        s.chars().take(300).collect()
    }
}

impl From<AcpError> for LlmError {
    fn from(e: AcpError) -> Self {
        match e {
            AcpError::Spawn(_, _) => LlmError::Provider(AcpError::short(e.to_string())),
            AcpError::Closed(_) => LlmError::Stream(AcpError::short(e.to_string())),
            AcpError::Protocol(_) => LlmError::Stream(AcpError::short(e.to_string())),
            AcpError::Agent(m) => LlmError::classify(&m),
            AcpError::Timeout(_) => LlmError::Network(AcpError::short(e.to_string())),
            AcpError::Cancelled => LlmError::Stream("turn cancelled".into()),
        }
    }
}

// -- JSON-RPC plumbing --------------------------------------------------------

type Pending = oneshot::Sender<Result<Value, AcpError>>;

struct Rpc {
    next_id: AtomicI64,
    writer: mpsc::UnboundedSender<String>,
    pending: Arc<std::sync::Mutex<HashMap<i64, Pending>>>,
    _child: Child,
    _io: tokio::task::JoinHandle<()>,
}

impl Rpc {
    fn send_request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<oneshot::Receiver<Result<Value, AcpError>>, AcpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("acp pending").insert(id, tx);
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.send_line(&line.to_string()).map_err(|_| {
            self.pending.lock().expect("acp pending").remove(&id);
            AcpError::Closed(format!("agent stdin closed while sending {method}"))
        })?;
        Ok(rx)
    }

    fn send_notification(&self, method: &str, params: Value) -> Result<(), AcpError> {
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send_line(&line.to_string())
            .map_err(|()| AcpError::Closed(format!("agent stdin closed ({method})")))
    }

    fn send_line(&self, line: &str) -> Result<(), ()> {
        self.writer.send(line.to_string()).map_err(|_| ())
    }
}

async fn io_loop(
    stdout: ChildStdout,
    mut stdin: ChildStdin,
    mut lines_out: mpsc::UnboundedReceiver<String>,
    writer: mpsc::UnboundedSender<String>,
    pending: Arc<std::sync::Mutex<HashMap<i64, Pending>>>,
    updates: mpsc::UnboundedSender<acp::SessionNotification>,
    bridge: PermissionBridge,
) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => handle_line(&line, &pending, &updates, &writer, &bridge),
                    _ => break,
                }
            }
            line = lines_out.recv() => {
                match line {
                    Some(line) => {
                        if stdin.write_all(line.as_bytes()).await.is_err()
                            || stdin.write_all(b"\n").await.is_err()
                            || stdin.flush().await.is_err()
                        {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
    // Wake every waiter so turns fail instead of hanging on a dead agent.
    for (_, tx) in pending.lock().expect("acp pending").drain() {
        let _ = tx.send(Err(AcpError::Closed("agent exited".into())));
    }
}

fn handle_line(
    line: &str,
    pending: &Arc<std::sync::Mutex<HashMap<i64, Pending>>>,
    updates: &mpsc::UnboundedSender<acp::SessionNotification>,
    writer: &mpsc::UnboundedSender<String>,
    bridge: &PermissionBridge,
) {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return,
    };
    if let Some(id) = msg.get("id").and_then(Value::as_i64) {
        if msg.get("method").is_some() {
            // Agent -> client request (permission, fs, terminal...): answer
            // through the permission bridge so CLI agents share the same
            // approval flow + project-root confinement as built-in tools.
            // `Ask` fails closed headless (first reject option); the
            // bridge's `NeedsApproval` path is for UI-driven callers.
            let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let reply = match handle_agent_request(bridge, method, &params) {
                Ok(result) => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result,
                })
                .to_string(),
                Err(message) => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": message },
                })
                .to_string(),
            };
            let _ = writer.send(reply);
            return;
        }
        let tx = pending.lock().expect("acp pending").remove(&id);
        let Some(tx) = tx else { return };
        if let Some(err) = msg.get("error") {
            let text = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("agent error");
            let _ = tx.send(Err(AcpError::Agent(text.to_string())));
        } else {
            let result = msg.get("result").cloned().unwrap_or(Value::Null);
            let _ = tx.send(Ok(result));
        }
        return;
    }
    if msg.get("method").and_then(Value::as_str) == Some("session/update") {
        if let Some(params) = msg.get("params") {
            if let Ok(notif) = serde_json::from_value::<acp::SessionNotification>(params.clone()) {
                let _ = updates.send(notif);
            }
        }
    }
}

// -- session -----------------------------------------------------------------

/// One live ACP session (one spawned agent + one `session/new` id).
pub struct AcpSession {
    rpc: Rpc,
    session_id: acp::SessionId,
    updates: mpsc::UnboundedReceiver<acp::SessionNotification>,
}

impl AcpSession {
    /// Spawn the agent, run `initialize`, open one session.
    pub async fn connect(config: &AcpConfig) -> Result<Self, AcpError> {
        let mut cmd = Command::new(&config.program);
        cmd.args(&config.args)
            .current_dir(&config.cwd)
            .envs(config.env.iter().cloned())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| AcpError::Spawn(config.program.clone(), AcpError::short(e.to_string())))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::Spawn(config.program.clone(), "no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::Spawn(config.program.clone(), "no stdout".into()))?;
        let (wtx, wrx) = mpsc::unbounded_channel::<String>();
        let (utx, urx) = mpsc::unbounded_channel::<acp::SessionNotification>();
        let pending: Arc<std::sync::Mutex<HashMap<i64, Pending>>> = Arc::default();
        // Permission bridge confined to the session cwd: the same
        // allow/ask/deny engine as built-in tools, so `session/
        // request_permission` + `fs/*` + `terminal/*` stay inside the
        // project and ask through the same approval flow.
        let bridge = PermissionBridge::new(Engine::new(), config.cwd.clone());
        let capabilities = bridge.client_capabilities();
        let io = tokio::spawn(io_loop(
            stdout,
            stdin,
            wrx,
            wtx.clone(),
            pending.clone(),
            utx,
            bridge,
        ));
        let rpc = Rpc {
            next_id: AtomicI64::new(0),
            writer: wtx,
            pending,
            _child: child,
            _io: io,
        };

        let params = serde_json::to_value(acp::InitializeRequest {
            protocol_version: acp::V1,
            client_capabilities: capabilities,
            meta: None,
        })
        .expect("initialize params");
        let _init: acp::InitializeResponse = rpc_call(&rpc, "initialize", params).await?;

        let params = serde_json::to_value(acp::NewSessionRequest {
            cwd: config.cwd.clone(),
            mcp_servers: Vec::new(),
            meta: None,
        })
        .expect("new_session params");
        let session: acp::NewSessionResponse = rpc_call(&rpc, "session/new", params).await?;
        Ok(Self {
            rpc,
            session_id: session.session_id,
            updates: urx,
        })
    }

    pub fn session_id(&self) -> String {
        self.session_id.0.to_string()
    }

    /// Run one turn: `session/prompt` + streamed `session/update`s.
    /// `cancel` races the turn; on fire it sends `session/cancel` and the
    /// turn fails with [`AcpError::Cancelled`].
    pub async fn prompt(
        &mut self,
        req: &TurnRequest,
        cancel: &CancellationToken,
    ) -> Result<Vec<AgentEvent>, AcpError> {
        let params = serde_json::to_value(acp::PromptRequest {
            session_id: self.session_id.clone(),
            prompt: prompt_blocks(req),
            meta: None,
        })
        .map_err(|e| AcpError::Protocol(AcpError::short(e.to_string())))?;
        let rx = self.rpc.send_request("session/prompt", params)?;
        let mut events: Vec<AgentEvent> = Vec::new();
        let mut text = String::new();
        let mut tool_calls: Vec<ToolCallRequest> = Vec::new();
        let mut usage = Usage::default();
        // Poll the prompt response and the update channel together.
        let mut rx = rx;
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    let params = serde_json::to_value(acp::CancelNotification {
                        session_id: self.session_id.clone(),
                        meta: None,
                    }).unwrap_or(Value::Null);
                    let _ = self.rpc.send_notification("session/cancel", params);
                    // Give the agent a moment to acknowledge; then fail fast.
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), &mut rx).await;
                    return Err(AcpError::Cancelled);
                }
                res = &mut rx => {
                    let result = res.map_err(|_| AcpError::Closed("agent exited mid-turn".into()))??;
                    return self.finish_turn(result, events, text, tool_calls, usage).await;
                }
                note = self.updates.recv() => {
                    let Some(note) = note else {
                        return Err(AcpError::Closed("agent exited mid-turn".into()));
                    };
                    if note.session_id != self.session_id {
                        continue;
                    }
                    for ev in update_events(&note.update) {
                        match &ev {
                            AgentEvent::TextDelta(d) => text.push_str(d),
                            AgentEvent::ToolCall(c) => tool_calls.push(c.clone()),
                            AgentEvent::Usage(u) => usage = *u,
                            AgentEvent::Done { .. } => {}
                        }
                        events.push(ev);
                    }
                }
            }
        }
    }

    /// Drain straggler updates, read usage from the response `_meta` and
    /// close the turn with a `Done` event (or `Cancelled`).
    async fn finish_turn(
        &mut self,
        result: Value,
        mut events: Vec<AgentEvent>,
        mut text: String,
        mut tool_calls: Vec<ToolCallRequest>,
        mut usage: Usage,
    ) -> Result<Vec<AgentEvent>, AcpError> {
        while let Ok(note) = self.updates.try_recv() {
            if note.session_id != self.session_id {
                continue;
            }
            for ev in update_events(&note.update) {
                match &ev {
                    AgentEvent::TextDelta(d) => text.push_str(d),
                    AgentEvent::ToolCall(c) => tool_calls.push(c.clone()),
                    AgentEvent::Usage(u) => usage = *u,
                    AgentEvent::Done { .. } => {}
                }
                events.push(ev);
            }
        }
        let resp: acp::PromptResponse = serde_json::from_value(result)
            .map_err(|e| AcpError::Protocol(AcpError::short(e.to_string())))?;
        if resp.stop_reason == acp::StopReason::Cancelled {
            return Err(AcpError::Cancelled);
        }
        if let Some(u) = resp.meta.as_ref().and_then(usage_from_meta) {
            usage = u;
            events.push(AgentEvent::Usage(u));
        }
        events.push(AgentEvent::Done {
            text,
            tool_calls,
            usage,
        });
        Ok(events)
    }
}

async fn rpc_call<T: serde::de::DeserializeOwned>(
    rpc: &Rpc,
    method: &str,
    params: Value,
) -> Result<T, AcpError> {
    let rx = rpc.send_request(method, params)?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
        .await
        .map_err(|_| AcpError::Timeout(method.into()))?
        .map_err(|_| AcpError::Closed(format!("agent exited during {method}")))?;
    let value = result?;
    serde_json::from_value(value).map_err(|e| AcpError::Protocol(AcpError::short(e.to_string())))
}

// -- prompt / update mapping -------------------------------------------------

fn prompt_blocks(req: &TurnRequest) -> Vec<acp::ContentBlock> {
    let mut blocks = Vec::new();
    if let Some(system) = req.system.as_ref().filter(|s| !s.trim().is_empty()) {
        blocks.push(acp::ContentBlock::Text(acp::TextContent {
            annotations: None,
            text: format!("<system>\n{system}\n</system>"),
            meta: None,
        }));
    }
    for m in &req.messages {
        let prefix = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool-result",
        };
        blocks.push(acp::ContentBlock::Text(acp::TextContent {
            annotations: None,
            text: format!("[{prefix}] {}", m.text),
            meta: None,
        }));
    }
    for r in &req.tool_results {
        blocks.push(acp::ContentBlock::Text(acp::TextContent {
            annotations: None,
            text: format!("[tool-result {}] {}", r.call_id, r.result),
            meta: None,
        }));
    }
    if blocks.is_empty() {
        blocks.push("".into());
    }
    blocks
}

fn block_text(block: &acp::ContentBlock) -> Option<String> {
    match block {
        acp::ContentBlock::Text(t) => Some(t.text.clone()),
        acp::ContentBlock::Image(_) => Some("[image]".into()),
        acp::ContentBlock::Audio(_) => Some("[audio]".into()),
        acp::ContentBlock::ResourceLink(l) => Some(format!("[resource: {}]", l.uri)),
        acp::ContentBlock::Resource(r) => match &r.resource {
            acp::EmbeddedResourceResource::TextResourceContents(t) => Some(t.text.clone()),
            acp::EmbeddedResourceResource::BlobResourceContents(b) => {
                Some(format!("[binary resource: {}]", b.uri))
            }
        },
    }
}

fn tool_content_text(c: &acp::ToolCallContent) -> Option<String> {
    match c {
        acp::ToolCallContent::Content { content } => block_text(content),
        acp::ToolCallContent::Diff { diff } => {
            serde_json::to_string(&serde_json::json!({ "diff": format!("{diff:?}") })).ok()
        }
        acp::ToolCallContent::Terminal { terminal_id } => {
            Some(format!("[terminal {}]", terminal_id.0))
        }
    }
}

/// Map one `session/update` onto zero or more [`AgentEvent`]s.
fn update_events(update: &acp::SessionUpdate) -> Vec<AgentEvent> {
    match update {
        acp::SessionUpdate::AgentMessageChunk { content } => block_text(content)
            .map(|t| vec![AgentEvent::TextDelta(t)])
            .unwrap_or_default(),
        acp::SessionUpdate::ToolCall(call) => vec![AgentEvent::ToolCall(ToolCallRequest {
            id: call.id.0.to_string(),
            name: call.title.clone(),
            args: call.raw_input.clone().unwrap_or(Value::Null),
        })],
        acp::SessionUpdate::ToolCallUpdate(u) => {
            let mut out = Vec::new();
            if let Some(fields) = Some(&u.fields) {
                if let Some(content) = fields.content.as_ref() {
                    for c in content {
                        if let Some(t) = tool_content_text(c) {
                            if !t.is_empty() {
                                out.push(AgentEvent::TextDelta(t));
                            }
                        }
                    }
                }
            }
            out
        }
        // Thoughts, plans, modes and user-message echoes carry no model
        // output for the turn transcript.
        _ => Vec::new(),
    }
}

/// Read token usage from a response `_meta` blob. Accepts
/// `{usage: {inputTokens|input_tokens, outputTokens|output_tokens}}` and
/// `{tokens: {...}}`; returns `None` when the agent reports nothing.
fn usage_from_meta(meta: &Value) -> Option<Usage> {
    let u = meta.get("usage").or_else(|| meta.get("tokens"))?;
    let num = |keys: &[&str]| -> Option<u64> {
        keys.iter().find_map(|k| {
            u.get(*k).and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
            })
        })
    };
    let input = num(&["inputTokens", "input_tokens", "input"])?;
    let output = num(&["outputTokens", "output_tokens", "output"])?;
    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input + output,
    })
}

// -- ModelBackend ------------------------------------------------------------

/// Long-lived ACP backend: spawns the agent per turn and implements
/// [`ModelBackend`] so the orchestrator can drive CLI agents like API models.
pub struct AcpBackend {
    config: AcpConfig,
}

impl AcpBackend {
    pub fn new(config: AcpConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &AcpConfig {
        &self.config
    }

    /// One turn over a fresh agent process + session (agents are stateless
    /// across turns; conversation context travels in the prompt).
    pub async fn turn_stream_cancelable(
        &self,
        req: TurnRequest,
        cancel: &CancellationToken,
    ) -> Result<Vec<AgentEvent>, AcpError> {
        let mut session = AcpSession::connect(&self.config).await?;
        session.prompt(&req, cancel).await
    }
}

fn events_to_output(events: &[AgentEvent]) -> TurnOutput {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut usage = Usage::default();
    for e in events {
        match e {
            AgentEvent::TextDelta(d) => text.push_str(d),
            AgentEvent::ToolCall(c) => tool_calls.push(c.clone()),
            AgentEvent::Usage(u) => usage = *u,
            AgentEvent::Done {
                text: t,
                tool_calls: c,
                usage: u,
            } => {
                text = t.clone();
                tool_calls = c.clone();
                usage = *u;
            }
        }
    }
    TurnOutput {
        text,
        tool_calls,
        usage,
    }
}

impl ModelBackend for AcpBackend {
    async fn turn(&self, req: TurnRequest) -> Result<TurnOutput, LlmError> {
        let events = self.turn_stream(req).await?;
        Ok(events_to_output(&events))
    }

    async fn turn_stream(&self, req: TurnRequest) -> Result<Vec<AgentEvent>, LlmError> {
        self.turn_stream_cancelable(req, &CancellationToken::new())
            .await
            .map_err(LlmError::from)
    }
}

/// Build the on-disk working directory default: the process cwd.
pub fn default_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Adapter command from detection (`detect::AvailableAdapter`) as an
/// [`AcpConfig`] rooted at `cwd`.
pub fn config_from_adapter(command: &str, args: &[String], cwd: &Path) -> AcpConfig {
    AcpConfig {
        program: command.to_string(),
        args: args.to_vec(),
        cwd: cwd.to_path_buf(),
        env: Vec::new(),
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;
    use std::sync::Arc;
    use synapse_llm::{ToolCallRequest, TurnMessage};

    const FAKE_AGENT: &str = r#"
import sys, json, os
def send(o):
    sys.stdout.write(json.dumps(o) + "\n")
    sys.stdout.flush()
mode = os.environ.get("FAKE_MODE", "happy")
log = os.environ.get("FAKE_LOG", "")
def note_cancel():
    if log:
        with open(log, "a") as f:
            f.write("cancel\n")
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue
    m = msg.get("method")
    mid = msg.get("id")
    if m == "initialize":
        send({"jsonrpc": "2.0", "id": mid,
              "result": {"protocolVersion": 1, "agentCapabilities": {}}})
    elif m == "session/new":
        send({"jsonrpc": "2.0", "id": mid,
              "result": {"sessionId": "sess_fake"}})
    elif m == "session/prompt":
        sid = msg["params"]["sessionId"]
        if mode == "slow":
            for line2 in sys.stdin:
                line2 = line2.strip()
                if not line2:
                    continue
                try:
                    msg2 = json.loads(line2)
                except Exception:
                    continue
                if msg2.get("method") == "session/cancel":
                    note_cancel()
                    send({"jsonrpc": "2.0", "id": mid,
                          "result": {"stopReason": "cancelled"}})
                    break
        else:
            send({"jsonrpc": "2.0", "method": "session/update",
                  "params": {"sessionId": sid, "update": {
                      "sessionUpdate": "agent_message_chunk",
                      "content": {"type": "text", "text": "hello "}}}})
            send({"jsonrpc": "2.0", "method": "session/update",
                  "params": {"sessionId": sid, "update": {
                      "sessionUpdate": "agent_message_chunk",
                      "content": {"type": "text", "text": "world"}}}})
            send({"jsonrpc": "2.0", "method": "session/update",
                  "params": {"sessionId": sid, "update": {
                      "sessionUpdate": "tool_call", "toolCallId": "call_1",
                      "title": "get_weather",
                      "rawInput": {"city": "Oslo"}}}})
            send({"jsonrpc": "2.0", "id": mid,
                  "result": {"stopReason": "end_turn",
                             "_meta": {"usage": {"inputTokens": 7,
                                                 "outputTokens": 11}}}})
    elif m == "session/cancel":
        note_cancel()
"#;

    fn tmp_dir(prefix: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "synapse-acp-client-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_config(mode: &str, log: Option<&Path>) -> (AcpConfig, PathBuf) {
        let dir = tmp_dir(mode);
        let script = dir.join("fake_acp.py");
        std::fs::write(&script, FAKE_AGENT).unwrap();
        let mut cfg = AcpConfig::new("python3", dir.clone());
        cfg.args = vec![script.to_string_lossy().into_owned()];
        cfg.env = vec![("FAKE_MODE".into(), mode.into())];
        if let Some(log) = log {
            cfg.env
                .push(("FAKE_LOG".into(), log.to_string_lossy().into_owned()));
        }
        (cfg, dir)
    }

    fn user_req() -> TurnRequest {
        TurnRequest::new("fake-model")
            .with_system("be brief")
            .with_message(TurnMessage::user("weather in Oslo?"))
    }

    #[tokio::test]
    async fn client_spawns_initializes_and_streams_turn() {
        let (cfg, _dir) = fake_config("happy", None);
        let mut session = AcpSession::connect(&cfg).await.expect("connect");
        assert_eq!(session.session_id(), "sess_fake");
        let events = session
            .prompt(&user_req(), &CancellationToken::new())
            .await
            .expect("prompt");
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::TextDelta(d) => Some(d.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "hello world");
        let call = events.iter().find_map(|e| match e {
            AgentEvent::ToolCall(c) => Some(c.clone()),
            _ => None,
        });
        assert_eq!(
            call,
            Some(ToolCallRequest {
                id: "call_1".into(),
                name: "get_weather".into(),
                args: serde_json::json!({"city": "Oslo"}),
            })
        );
        let done = events.iter().find_map(|e| match e {
            AgentEvent::Done {
                text,
                tool_calls,
                usage,
            } => Some((text.clone(), tool_calls.len(), *usage)),
            _ => None,
        });
        let (done_text, n_calls, usage) = done.expect("Done event");
        assert_eq!(done_text, "hello world");
        assert_eq!(n_calls, 1);
        assert_eq!(usage.input_tokens, 7);
        assert_eq!(usage.output_tokens, 11);
        assert_eq!(usage.total_tokens, 18);
    }

    #[tokio::test]
    async fn client_backend_implements_model_backend() {
        let (cfg, _dir) = fake_config("happy", None);
        let backend = AcpBackend::new(cfg);
        let out = backend.turn(user_req()).await.expect("turn");
        assert_eq!(out.text, "hello world");
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.usage.total_tokens, 18);
        // Stream form carries the same content plus usage + done events.
        let events = backend.turn_stream(user_req()).await.expect("stream");
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Usage(_))));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Done { .. })));
    }

    #[tokio::test]
    async fn client_missing_binary_maps_to_provider_error() {
        let cfg = AcpConfig::new("synapse-no-such-acp-binary-xyz", default_cwd());
        let err = match AcpSession::connect(&cfg).await {
            Err(e) => e,
            Ok(_) => panic!("connect must fail"),
        };
        assert!(matches!(err, AcpError::Spawn(_, _)), "{err:?}");
        let backend = AcpBackend::new(cfg);
        let err = backend.turn(user_req()).await.expect_err("must fail");
        assert!(matches!(err, LlmError::Provider(_)), "{err:?}");
    }

    #[tokio::test]
    async fn client_cancellation_sends_session_cancel() {
        let log = tmp_dir("cancellog").join("cancel.log");
        let (cfg, _dir) = fake_config("slow", Some(&log));
        let mut session = AcpSession::connect(&cfg).await.expect("connect");
        let cancel = CancellationToken::new();
        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            canceller.cancel();
        });
        let err = session
            .prompt(&user_req(), &cancel)
            .await
            .expect_err("must cancel");
        assert_eq!(err, AcpError::Cancelled);
        // The fake agent observed `session/cancel`.
        let seen = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !log.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(seen.is_ok(), "fake agent never saw session/cancel");
    }

    #[test]
    fn client_usage_parses_meta_shapes() {
        let camel = serde_json::json!({"usage": {"inputTokens": 3, "outputTokens": 4}});
        assert_eq!(
            usage_from_meta(&camel),
            Some(Usage {
                input_tokens: 3,
                output_tokens: 4,
                total_tokens: 7
            })
        );
        let snake = serde_json::json!({"tokens": {"input_tokens": 1, "output_tokens": 2}});
        assert_eq!(
            usage_from_meta(&snake),
            Some(Usage {
                input_tokens: 1,
                output_tokens: 2,
                total_tokens: 3
            })
        );
        // No usage reported -> None (caller keeps zero usage).
        assert_eq!(usage_from_meta(&serde_json::json!({})), None);
        assert_eq!(usage_from_meta(&serde_json::json!({"usage": {}})), None);
    }

    #[test]
    fn client_prompt_blocks_carry_system_and_tool_results() {
        let req = TurnRequest::new("m")
            .with_system("sys")
            .with_message(TurnMessage::user("hi"))
            .with_message(TurnMessage::assistant("yo"));
        let mut req = req;
        req.tool_results.push(TurnMessage::tool_result("c1", "42"));
        let blocks = prompt_blocks(&req);
        assert_eq!(blocks.len(), 4);
        let texts: Vec<String> = blocks.iter().filter_map(block_text).collect();
        assert!(texts[0].contains("sys"), "{texts:?}");
        assert!(texts[3].contains("c1"), "{texts:?}");
    }

    #[test]
    fn client_update_events_ignore_thoughts_and_map_tool_calls() {
        let thought = acp::SessionUpdate::AgentThoughtChunk {
            content: "hmm".into(),
        };
        assert!(update_events(&thought).is_empty());
        let call = acp::SessionUpdate::ToolCall(acp::ToolCall {
            id: acp::ToolCallId(Arc::from("c9")),
            title: "read".into(),
            kind: acp::ToolKind::Read,
            status: acp::ToolCallStatus::InProgress,
            content: Vec::new(),
            locations: Vec::new(),
            raw_input: Some(serde_json::json!({"path": "a"})),
            raw_output: None,
            meta: None,
        });
        assert_eq!(
            update_events(&call),
            vec![AgentEvent::ToolCall(ToolCallRequest {
                id: "c9".into(),
                name: "read".into(),
                args: serde_json::json!({"path": "a"}),
            })]
        );
    }
}
