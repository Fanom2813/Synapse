//! Headless CLI fallbacks for agents without ACP support.
//!
//! Commands (see `docs/STACK.md` §CLI agents):
//! `claude -p --output-format stream-json --verbose`,
//! `codex exec --json`, `opencode run --format json`,
//! `gemini -p --output-format stream-json`.
//!
//! Each CLI emits line-delimited JSON on stdout. This module parses those
//! streams into the common [`AgentEvent`] behavior: text deltas, tool calls,
//! usage, and a terminal `Done` (or a mapped [`LlmError`]). Cancellation
//! kills the child process and surfaces as a stream error.

use std::path::{Path, PathBuf};

use serde_json::Value;
use synapse_llm::{AgentEvent, LlmError, ModelBackend, TurnOutput, TurnRequest, Usage};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

/// Which CLI's JSON stream to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlessKind {
    Claude,
    Codex,
    Opencode,
    Gemini,
}

impl HeadlessKind {
    /// CLI program name for this fallback.
    pub fn program(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
            Self::Gemini => "gemini",
        }
    }
}

/// How to spawn a headless CLI turn.
#[derive(Debug, Clone)]
pub struct HeadlessConfig {
    pub kind: HeadlessKind,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

impl HeadlessConfig {
    pub fn new(kind: HeadlessKind, cwd: impl Into<PathBuf>) -> Self {
        let program = kind.program().to_string();
        let args = default_args(kind);
        Self {
            kind,
            program,
            args,
            cwd: cwd.into(),
            env: Vec::new(),
        }
    }

    pub fn with_program(mut self, program: impl Into<String>) -> Self {
        self.program = program.into();
        self
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Full argv for one turn: base args + rendered prompt.
    pub fn argv_for(&self, prompt: &str) -> (String, Vec<String>) {
        let mut args = self.args.clone();
        // All four CLIs take the prompt as a trailing positional arg.
        args.push(prompt.to_string());
        (self.program.clone(), args)
    }
}

/// Default argv (before the prompt) per CLI.
pub fn default_args(kind: HeadlessKind) -> Vec<String> {
    match kind {
        HeadlessKind::Claude => vec![
            "-p".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
        ],
        HeadlessKind::Codex => vec!["exec".into(), "--json".into()],
        HeadlessKind::Opencode => vec!["run".into(), "--format".into(), "json".into()],
        HeadlessKind::Gemini => vec!["-p".into(), "--output-format".into(), "stream-json".into()],
    }
}

/// Failures of a headless turn, mapped to [`LlmError`] at the boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeadlessError {
    #[error("could not spawn `{0}`: {1}")]
    Spawn(String, String),
    #[error("headless agent failed: {0}")]
    Agent(String),
    #[error("turn cancelled")]
    Cancelled,
}

impl From<HeadlessError> for LlmError {
    fn from(e: HeadlessError) -> Self {
        match e {
            HeadlessError::Spawn(_, _) => LlmError::Provider(short(&e.to_string())),
            HeadlessError::Agent(m) => LlmError::classify(&m),
            HeadlessError::Cancelled => LlmError::Stream("turn cancelled".into()),
        }
    }
}

fn short(s: &str) -> String {
    s.chars().take(300).collect()
}

// -- line parsing ------------------------------------------------------------

/// Outcome of parsing one JSONL line.
#[derive(Debug, Default)]
struct ParsedLine {
    deltas: Vec<String>,
    tool_calls: Vec<synapse_llm::ToolCallRequest>,
    usage: Option<Usage>,
    /// Final assistant text when this line completes the turn.
    done_text: Option<String>,
    /// Non-empty when this line reports a fatal error.
    error: Option<String>,
}

fn get_u64(v: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| {
        v.get(*k).and_then(|n| {
            n.as_u64()
                .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok()))
                .or_else(|| n.as_str().and_then(|s| s.parse().ok()))
        })
    })
}

fn usage_from_obj(v: &Value) -> Option<Usage> {
    // Accept usage blobs at any nesting the CLIs use.
    for key in ["usage", "tokens", "usageMetadata", "usage_metadata"] {
        if let Some(u) = v.get(key) {
            let input = get_u64(
                u,
                &[
                    "inputTokens",
                    "input_tokens",
                    "input",
                    "promptTokens",
                    "prompt_tokens",
                    "promptTokenCount",
                ],
            );
            let output = get_u64(
                u,
                &[
                    "outputTokens",
                    "output_tokens",
                    "output",
                    "completionTokens",
                    "completion_tokens",
                    "candidatesTokenCount",
                ],
            );
            if let (Some(i), Some(o)) = (input, output) {
                return Some(Usage::new(i, o));
            }
            // Single total only: split conservatively (all to output).
            if let Some(t) = get_u64(u, &["totalTokens", "total_tokens", "total"]) {
                return Some(Usage {
                    input_tokens: 0,
                    output_tokens: t,
                    total_tokens: t,
                });
            }
        }
    }
    // Flat usage fields on the object itself.
    let input = get_u64(
        v,
        &[
            "inputTokens",
            "input_tokens",
            "promptTokenCount",
            "prompt_tokens",
        ],
    );
    let output = get_u64(
        v,
        &[
            "outputTokens",
            "output_tokens",
            "candidatesTokenCount",
            "completion_tokens",
        ],
    );
    match (input, output) {
        (Some(i), Some(o)) => Some(Usage::new(i, o)),
        _ => None,
    }
}

fn str_field(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(|s| s.as_str()).map(|s| s.to_string()))
}

fn tool_call(id: String, name: String, args: Value) -> synapse_llm::ToolCallRequest {
    synapse_llm::ToolCallRequest { id, name, args }
}

fn parse_args(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
        other => other.clone(),
    }
}

/// Parse one line of a headless stream. Never fails: unknown shapes are
/// ignored so a CLI version skew degrades to fewer deltas, not a broken turn.
pub fn parse_headless_line(kind: HeadlessKind, line: &Value) -> Vec<AgentEvent> {
    parsed_to_events(parse_line(kind, line))
}

fn parsed_to_events(p: ParsedLine) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    for d in p.deltas {
        if !d.is_empty() {
            out.push(AgentEvent::TextDelta(d));
        }
    }
    for c in p.tool_calls {
        out.push(AgentEvent::ToolCall(c));
    }
    if let Some(u) = p.usage {
        out.push(AgentEvent::Usage(u));
    }
    out
}

fn parse_line(kind: HeadlessKind, v: &Value) -> ParsedLine {
    match kind {
        HeadlessKind::Claude => parse_claude(v),
        HeadlessKind::Codex => parse_codex(v),
        HeadlessKind::Opencode => parse_opencode(v),
        HeadlessKind::Gemini => parse_gemini(v),
    }
}

fn content_blocks_text(content: &Value) -> (Vec<String>, Vec<synapse_llm::ToolCallRequest>) {
    let mut deltas = Vec::new();
    let mut calls = Vec::new();
    let items = match content {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    for b in items {
        let t = b.get("type").and_then(|s| s.as_str()).unwrap_or("");
        match t {
            "text" => {
                if let Some(s) = b.get("text").and_then(|s| s.as_str()) {
                    deltas.push(s.to_string());
                }
            }
            "tool_use" => {
                let id = str_field(&b, &["id", "tool_use_id"])
                    .unwrap_or_else(|| format!("call_{}", calls.len()));
                let name = str_field(&b, &["name", "tool"]).unwrap_or_else(|| "tool".into());
                let args = b
                    .get("input")
                    .or_else(|| b.get("args"))
                    .cloned()
                    .unwrap_or(Value::Null);
                calls.push(tool_call(id, name, args));
            }
            _ => {
                // Delta-style chunks: {"type":"..._delta","delta":{"text":".."}} or {"delta":{...}}
                if let Some(delta) = b.get("delta") {
                    if let Some(s) = delta.get("text").and_then(|s| s.as_str()) {
                        deltas.push(s.to_string());
                    } else if let Some(s) = delta.as_str() {
                        deltas.push(s.to_string());
                    }
                } else if let Some(s) = b.get("text").and_then(|s| s.as_str()) {
                    deltas.push(s.to_string());
                }
            }
        }
    }
    (deltas, calls)
}

fn parse_claude(v: &Value) -> ParsedLine {
    let mut p = ParsedLine::default();
    if let Some(u) = usage_from_obj(v) {
        p.usage = Some(u);
    }
    let typ = v.get("type").and_then(|s| s.as_str()).unwrap_or("");
    match typ {
        "assistant" | "stream_event" => {
            let content = v
                .get("message")
                .and_then(|m| m.get("content"))
                .unwrap_or(&Value::Null);
            let target = if content.is_null() {
                v.get("event")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.get("content"))
                    .unwrap_or(&Value::Null)
            } else {
                content
            };
            // streaming deltas nested under event.delta
            if let Some(delta) = v.get("event").and_then(|e| e.get("delta")) {
                if let Some(s) = delta.get("text").and_then(|s| s.as_str()) {
                    p.deltas.push(s.to_string());
                    return p;
                }
            }
            let (d, c) = content_blocks_text(target);
            p.deltas = d;
            p.tool_calls = c;
        }
        "result" => {
            if v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false)
                || matches!(v.get("subtype").and_then(|s| s.as_str()), Some("error"))
            {
                p.error = Some(
                    str_field(v, &["error", "result"]).unwrap_or_else(|| "claude failed".into()),
                );
                return p;
            }
            if let Some(s) = v.get("result").and_then(|s| s.as_str()) {
                p.done_text = Some(s.to_string());
            } else if let Some(s) = str_field(v, &["text", "response"]) {
                p.done_text = Some(s);
            } else {
                p.done_text = Some(String::new());
            }
        }
        "system" | "user" => {}
        _ => {
            // Lenient: bare text or content shapes from newer CLI versions.
            if let Some(msg) = v.get("message").and_then(|m| m.get("content")) {
                let (d, c) = content_blocks_text(msg);
                p.deltas = d;
                p.tool_calls = c;
            } else if let Some(s) = v.get("text").and_then(|s| s.as_str()) {
                p.deltas.push(s.to_string());
            }
        }
    }
    p
}

fn parse_codex(v: &Value) -> ParsedLine {
    let mut p = ParsedLine::default();
    if let Some(u) = usage_from_obj(v) {
        p.usage = Some(u);
    }
    let typ = v.get("type").and_then(|s| s.as_str()).unwrap_or("");
    match typ {
        "item.completed" | "item.updated" | "item.started" => {
            let item = v.get("item").unwrap_or(&Value::Null);
            let item_type = item.get("type").and_then(|s| s.as_str()).unwrap_or("");
            match item_type {
                "agent_message" => {
                    if let Some(s) = item.get("text").and_then(|s| s.as_str()) {
                        // completed items carry full text; updates carry deltas — both stream as deltas.
                        p.deltas.push(s.to_string());
                    }
                }
                "reasoning" => {}
                "function_call" | "tool_call" => {
                    if typ == "item.completed" {
                        let id = str_field(item, &["call_id", "id", "callId"])
                            .unwrap_or_else(|| format!("call_{}", 0));
                        let name =
                            str_field(item, &["name", "tool"]).unwrap_or_else(|| "tool".into());
                        let args = item
                            .get("arguments")
                            .or_else(|| item.get("args"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        p.tool_calls.push(tool_call(id, name, parse_args(&args)));
                    }
                }
                _ => {
                    if let Some(s) = item.get("text").and_then(|s| s.as_str()) {
                        p.deltas.push(s.to_string());
                    }
                }
            }
        }
        "turn.completed" | "thread.completed" | "result" | "done" => {
            p.done_text = str_field(v, &["text", "result", "response"]);
            if p.done_text.is_none() {
                // Completion without inline text: caller closes with accumulated deltas.
                p.done_text = Some(String::new());
            }
        }
        "error" => {
            p.error =
                Some(str_field(v, &["message", "error"]).unwrap_or_else(|| "codex failed".into()));
        }
        _ => {
            if let Some(s) = v.get("text").and_then(|s| s.as_str()) {
                p.deltas.push(s.to_string());
            }
        }
    }
    p
}

fn parse_opencode(v: &Value) -> ParsedLine {
    let mut p = ParsedLine::default();
    if let Some(u) = usage_from_obj(v) {
        p.usage = Some(u);
    }
    // opencode `run --format json` emits step/part frames; be lenient.
    if let Some(part) = v.get("part").or_else(|| v.get("data")) {
        let ptype = part
            .get("type")
            .and_then(|s| s.as_str())
            .unwrap_or_else(|| v.get("type").and_then(|s| s.as_str()).unwrap_or(""));
        match ptype {
            "text" => {
                if let Some(s) = part.get("text").and_then(|s| s.as_str()) {
                    p.deltas.push(s.to_string());
                } else if let Some(s) = part.get("snapshot").and_then(|s| s.as_str()) {
                    p.deltas.push(s.to_string());
                }
                return p;
            }
            "tool" | "tool_use" | "tool-call" => {
                let state = part.get("state");
                // Only surface completed tool states to avoid duplicate calls per frame.
                let status = state
                    .and_then(|s| s.get("status"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("completed");
                if status == "completed"
                    || part.get("callID").is_some()
                    || part.get("callId").is_some()
                {
                    let id = str_field(part, &["callID", "callId", "id"])
                        .unwrap_or_else(|| "call_0".into());
                    let name = str_field(part, &["tool", "name"]).unwrap_or_else(|| "tool".into());
                    let args = state
                        .and_then(|s| s.get("input"))
                        .or_else(|| part.get("input"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    // Skip pure text parts mislabeled as tools.
                    if name != "tool" || args != Value::Null {
                        p.tool_calls.push(tool_call(id, name, args));
                        return p;
                    }
                }
                return p;
            }
            _ => {}
        }
    }
    let typ = v.get("type").and_then(|s| s.as_str()).unwrap_or("");
    match typ {
        "text" => {
            if let Some(s) = str_field(v, &["text", "content", "delta"]) {
                p.deltas.push(s);
            }
        }
        "result" | "done" | "final" => {
            p.done_text = str_field(v, &["text", "result", "response", "content"]);
            if p.done_text.is_none() {
                p.done_text = Some(String::new());
            }
        }
        "error" => {
            p.error = Some(
                str_field(v, &["message", "error"]).unwrap_or_else(|| "opencode failed".into()),
            );
        }
        _ => {
            if let Some(s) = v.get("text").and_then(|s| s.as_str()) {
                p.deltas.push(s.to_string());
            }
        }
    }
    p
}

fn parse_gemini(v: &Value) -> ParsedLine {
    let mut p = ParsedLine::default();
    if let Some(u) = usage_from_obj(v).or_else(|| {
        v.get("usageMetadata")
            .and_then(usage_from_obj)
            .or_else(|| v.get("usage_metadata").and_then(usage_from_obj))
    }) {
        p.usage = Some(u);
    }
    // usageMetadata may sit inside result objects.
    if p.usage.is_none() {
        for key in ["result", "response", "data"] {
            if let Some(inner) = v.get(key) {
                if let Some(u) = usage_from_obj(inner) {
                    p.usage = Some(u);
                    break;
                }
            }
        }
    }
    let typ = v.get("type").and_then(|s| s.as_str()).unwrap_or("");
    match typ {
        "message" | "content" | "delta" => {
            if let Some(s) = v.get("content").and_then(|s| s.as_str()) {
                p.deltas.push(s.to_string());
            } else if let Some(parts) = v.get("parts").and_then(|a| a.as_array()) {
                for part in parts {
                    if let Some(s) = part.get("text").and_then(|s| s.as_str()) {
                        p.deltas.push(s.to_string());
                    } else if part.get("functionCall").is_some() {
                        let fc = &part["functionCall"];
                        let name = str_field(fc, &["name"]).unwrap_or_else(|| "tool".into());
                        let args = fc.get("args").cloned().unwrap_or(Value::Null);
                        p.tool_calls.push(tool_call(
                            format!("call_{}", p.tool_calls.len()),
                            name,
                            args,
                        ));
                    }
                }
            } else if let Some(s) = str_field(v, &["text", "delta"]) {
                p.deltas.push(s);
            }
        }
        "result" | "done" | "final" => {
            p.done_text = str_field(v, &["response", "result", "text", "content"]);
            if p.done_text.is_none() {
                p.done_text = Some(String::new());
            }
        }
        "error" => {
            let msg = v.get("error").map(|e| match e {
                Value::String(s) => s.clone(),
                other => str_field(other, &["message"]).unwrap_or_else(|| other.to_string()),
            });
            p.error = Some(msg.unwrap_or_else(|| "gemini failed".into()));
        }
        _ => {
            // Role-based frames: {"role":"model","parts":[...]}
            if v.get("parts").and_then(|a| a.as_array()).is_some() {
                let mut sub = serde_json::json!({"type": "message"});
                sub["parts"] = v["parts"].clone();
                return parse_gemini(&sub);
            }
            if let Some(s) = v.get("content").and_then(|s| s.as_str()) {
                p.deltas.push(s.to_string());
            }
        }
    }
    p
}

// -- running -----------------------------------------------------------------

fn render_prompt(req: &TurnRequest) -> String {
    let mut out = String::new();
    if let Some(s) = req.system.as_ref().filter(|s| !s.trim().is_empty()) {
        out.push_str(s);
        out.push_str("\n\n");
    }
    for m in &req.messages {
        let role = match m.role {
            synapse_llm::Role::System => "system",
            synapse_llm::Role::User => "user",
            synapse_llm::Role::Assistant => "assistant",
            synapse_llm::Role::Tool => "tool-result",
        };
        out.push_str(&format!("[{role}] {}\n", m.text));
    }
    for r in &req.tool_results {
        out.push_str(&format!("[tool-result {}] {}\n", r.call_id, r.result));
    }
    if out.trim().is_empty() {
        out.push_str("(no prompt)");
    }
    out
}

/// Feed parsed JSONL lines into events; on completion push `Done`.
fn fold_lines(kind: HeadlessKind, lines: &[Value]) -> Result<Vec<AgentEvent>, HeadlessError> {
    let mut events = Vec::new();
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut usage = Usage::default();
    let mut done_text: Option<String> = None;
    for line in lines {
        let p = parse_line(kind, line);
        if let Some(err) = p.error {
            return Err(HeadlessError::Agent(err));
        }
        for d in p.deltas {
            text.push_str(&d);
            events.push(AgentEvent::TextDelta(d));
        }
        for c in p.tool_calls {
            tool_calls.push(c.clone());
            events.push(AgentEvent::ToolCall(c));
        }
        if let Some(u) = p.usage {
            usage = u;
            events.push(AgentEvent::Usage(u));
        }
        if let Some(t) = p.done_text {
            // Empty completion markers close with accumulated text.
            done_text = Some(if t.is_empty() { text.clone() } else { t });
        }
    }
    let final_text = done_text.unwrap_or_else(|| text.clone());
    events.push(AgentEvent::Done {
        text: final_text,
        tool_calls,
        usage,
    });
    Ok(events)
}

/// Parse one stdout blob (multiple JSONL lines; non-JSON lines ignored).
pub fn parse_headless_output(
    kind: HeadlessKind,
    stdout: &str,
) -> Result<Vec<AgentEvent>, HeadlessError> {
    let mut lines = Vec::new();
    for raw in stdout.lines() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        // Some CLIs pretty-print one JSON object across lines; try each line,
        // skipping fragments that do not parse alone.
        match serde_json::from_str::<Value>(raw) {
            Ok(v) => lines.push(v),
            Err(_) => continue,
        }
    }
    // A single pretty-printed object spanning lines: parse the whole blob.
    if lines.is_empty() && !stdout.trim().is_empty() {
        if let Ok(v) = serde_json::from_str::<Value>(stdout) {
            lines.push(v);
        }
    }
    fold_lines(kind, &lines)
}

/// Headless backend: one CLI process per turn, implementing [`ModelBackend`].
pub struct HeadlessBackend {
    config: HeadlessConfig,
}

impl HeadlessBackend {
    pub fn new(config: HeadlessConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &HeadlessConfig {
        &self.config
    }

    /// Run one turn over the CLI's stdout stream. `cancel` kills the child
    /// and fails the turn with [`HeadlessError::Cancelled`].
    pub async fn turn_stream_cancelable(
        &self,
        req: TurnRequest,
        cancel: &CancellationToken,
    ) -> Result<Vec<AgentEvent>, HeadlessError> {
        let prompt = render_prompt(&req);
        let (program, args) = self.config.argv_for(&prompt);
        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .current_dir(&self.config.cwd)
            .envs(self.config.env.iter().cloned())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| HeadlessError::Spawn(program.clone(), short(&e.to_string())))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HeadlessError::Spawn(program.clone(), "no stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| HeadlessError::Spawn(program, "no stderr".into()))?;

        let kind = self.config.kind;
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut values = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(t) {
                    values.push(v);
                }
            }
            values
        });
        let err_reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut buf = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&line);
            }
            buf
        });

        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(HeadlessError::Cancelled)
            }
            status = child.wait() => {
                let values = reader.await.unwrap_or_default();
                let stderr_text = err_reader.await.unwrap_or_default();
                let status = status.map_err(|e| HeadlessError::Agent(short(&e.to_string())))?;
                // Surface the first reported error line before exit-code handling.
                for v in &values {
                    if parse_line(kind, v).error.is_some() {
                        return fold_lines(kind, &values);
                    }
                }
                if !status.success() && values.is_empty() {
                    let msg = if stderr_text.trim().is_empty() {
                        format!("process exited with {status}")
                    } else {
                        short(stderr_text.trim())
                    };
                    Err(HeadlessError::Agent(msg))
                } else {
                    fold_lines(kind, &values)
                }
            }
        }
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

impl ModelBackend for HeadlessBackend {
    async fn turn(&self, req: TurnRequest) -> Result<TurnOutput, LlmError> {
        Ok(events_to_output(&self.turn_stream(req).await?))
    }

    async fn turn_stream(&self, req: TurnRequest) -> Result<Vec<AgentEvent>, LlmError> {
        self.turn_stream_cancelable(req, &CancellationToken::new())
            .await
            .map_err(LlmError::from)
    }
}

/// Default working directory: the process cwd.
pub fn default_headless_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Config rooted at `cwd` for `kind`'s default CLI.
pub fn config_for_kind(kind: HeadlessKind, cwd: &Path) -> HeadlessConfig {
    HeadlessConfig::new(kind, cwd)
}

#[cfg(test)]
mod headless_tests {
    use super::*;
    use synapse_llm::{ToolCallRequest, TurnMessage};

    fn v(json: serde_json::Value) -> Value {
        json
    }

    #[test]
    fn headless_claude_parses_text_tool_and_result() {
        let kind = HeadlessKind::Claude;
        let msg = v(
            serde_json::json!({"type": "assistant", "message": {"content": [
                {"type": "text", "text": "hi "},
                {"type": "tool_use", "id": "tu1", "name": "read", "input": {"path": "a"}}
            ]}}),
        );
        let p = parse_line(kind, &msg);
        assert_eq!(p.deltas, vec!["hi ".to_string()]);
        assert_eq!(p.tool_calls.len(), 1);
        assert_eq!(p.tool_calls[0].name, "read");
        assert!(p.done_text.is_none());

        let delta = v(serde_json::json!({"type": "stream_event",
            "event": {"delta": {"text": "there"}}}));
        assert_eq!(parse_line(kind, &delta).deltas, vec!["there".to_string()]);

        let done = v(serde_json::json!({"type": "result", "subtype": "success",
            "result": "hi there", "usage": {"inputTokens": 5, "outputTokens": 6}}));
        let p = parse_line(kind, &done);
        assert_eq!(p.done_text.as_deref(), Some("hi there"));
        assert_eq!(p.usage, Some(Usage::new(5, 6)));

        let err = v(serde_json::json!({"type": "result", "subtype": "error", "result": "boom"}));
        assert!(parse_line(kind, &err).error.is_some());

        // Unknown shapes are ignored, never fatal.
        assert!(parse_headless_line(kind, &v(serde_json::json!({"type": "system"}))).is_empty());
    }

    #[test]
    fn headless_codex_parses_messages_tools_and_completion() {
        let kind = HeadlessKind::Codex;
        let msg = v(serde_json::json!({"type": "item.completed",
            "item": {"type": "agent_message", "text": "hello "}}));
        assert_eq!(parse_line(kind, &msg).deltas, vec!["hello ".to_string()]);
        // Reasoning carries no transcript text.
        let reasoning = v(serde_json::json!({"type": "item.completed",
            "item": {"type": "reasoning", "text": "thinking"}}));
        assert!(parse_line(kind, &reasoning).deltas.is_empty());

        let call = v(serde_json::json!({"type": "item.completed",
            "item": {"type": "function_call", "call_id": "c1", "name": "shell",
                     "arguments": "{\"cmd\":\"ls\"}"}}));
        let p = parse_line(kind, &call);
        assert_eq!(
            p.tool_calls,
            vec![ToolCallRequest {
                id: "c1".into(),
                name: "shell".into(),
                args: serde_json::json!({"cmd": "ls"})
            }]
        );

        let done = v(serde_json::json!({"type": "turn.completed",
            "usage": {"input_tokens": 2, "output_tokens": 3}}));
        let p = parse_line(kind, &done);
        assert!(p.done_text.is_some());
        assert_eq!(p.usage, Some(Usage::new(2, 3)));

        let err = v(serde_json::json!({"type": "error", "message": "nope"}));
        assert_eq!(parse_line(kind, &err).error.as_deref(), Some("nope"));
    }

    #[test]
    fn headless_opencode_parses_parts_and_completion() {
        let kind = HeadlessKind::Opencode;
        let text = v(serde_json::json!({"type": "step", "part": {"type": "text", "text": "yo"}}));
        assert_eq!(parse_line(kind, &text).deltas, vec!["yo".to_string()]);
        let tool = v(serde_json::json!({"type": "step",
            "part": {"type": "tool", "tool": "read", "callID": "t1",
                     "state": {"status": "completed", "input": {"path": "b"}}}}));
        let p = parse_line(kind, &tool);
        assert_eq!(p.tool_calls.len(), 1);
        assert_eq!(p.tool_calls[0].name, "read");
        let done = v(serde_json::json!({"type": "result", "text": "yo"}));
        assert_eq!(parse_line(kind, &done).done_text.as_deref(), Some("yo"));
        // Bare completion marker closes with accumulated text.
        let bare = v(serde_json::json!({"type": "done"}));
        assert_eq!(parse_line(kind, &bare).done_text.as_deref(), Some(""));
    }

    #[test]
    fn headless_gemini_parses_content_and_usage_metadata() {
        let kind = HeadlessKind::Gemini;
        let msg = v(serde_json::json!({"type": "message", "content": "hello"}));
        assert_eq!(parse_line(kind, &msg).deltas, vec!["hello".to_string()]);
        let parts = v(serde_json::json!({"role": "model",
            "parts": [{"text": "hi"}, {"functionCall": {"name": "search", "args": {"q": "x"}}}]}));
        let p = parse_line(kind, &parts);
        assert_eq!(p.deltas, vec!["hi".to_string()]);
        assert_eq!(p.tool_calls.len(), 1);
        assert_eq!(p.tool_calls[0].name, "search");
        let done = v(serde_json::json!({"type": "result", "response": "hi",
            "usageMetadata": {"promptTokenCount": 4, "candidatesTokenCount": 5}}));
        let p = parse_line(kind, &done);
        assert_eq!(p.done_text.as_deref(), Some("hi"));
        assert_eq!(p.usage, Some(Usage::new(4, 5)));
    }

    #[test]
    fn headless_output_folds_to_done_with_usage() {
        let stdout = concat!(
            "{\"type\": \"assistant\", \"message\": {\"content\": [{\"type\": \"text\", \"text\": \"hello \"}]}}\n",
            "not json, ignored\n",
            "{\"type\": \"assistant\", \"message\": {\"content\": [{\"type\": \"text\", \"text\": \"world\"}]}}\n",
            "{\"type\": \"result\", \"subtype\": \"success\", \"result\": \"hello world\"}\n",
        );
        let events = parse_headless_output(HeadlessKind::Claude, stdout).unwrap();
        let done = events.iter().find_map(|e| match e {
            AgentEvent::Done { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(done.as_deref(), Some("hello world"));
        // Completion without inline text falls back to accumulated deltas.
        let events = parse_headless_output(HeadlessKind::Codex,
            "{\"type\": \"item.completed\", \"item\": {\"type\": \"agent_message\", \"text\": \"abc\"}}\n{\"type\": \"turn.completed\"}\n").unwrap();
        let done = events.iter().find_map(|e| match e {
            AgentEvent::Done { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(done.as_deref(), Some("abc"));
        // Error lines surface as agent errors.
        let err = parse_headless_output(
            HeadlessKind::Codex,
            "{\"type\": \"error\", \"message\": \"bad key\"}\n",
        );
        assert!(matches!(err, Err(HeadlessError::Agent(_))));
    }

    fn fake_script(text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "synapse-headless-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake.py");
        std::fs::write(&script, text).unwrap();
        script
    }

    #[tokio::test]
    async fn headless_backend_runs_fake_claude_stream_to_done() {
        let script = fake_script(concat!(
            "import json\n",
            "print(json.dumps({'type':'assistant','message':{'content':[{'type':'text','text':'hello '}]} }), flush=True)\n",
            "print(json.dumps({'type':'result','subtype':'success','result':'hello ','usage':{'inputTokens':1,'outputTokens':2}}), flush=True)\n",
        ));
        let mut cfg = HeadlessConfig::new(HeadlessKind::Claude, std::env::temp_dir());
        cfg.program = "python3".into();
        cfg.args = vec![script.to_string_lossy().into_owned()];
        let backend = HeadlessBackend::new(cfg);
        let req = TurnRequest::new("m").with_message(TurnMessage::user("hi"));
        let events = backend.turn_stream(req).await.expect("headless turn");
        let done = events.iter().find_map(|e| match e {
            AgentEvent::Done { text, usage, .. } => Some((text.clone(), *usage)),
            _ => None,
        });
        let (text, usage) = done.expect("Done");
        assert_eq!(text, "hello ");
        assert_eq!(usage, Usage::new(1, 2));
        let out = backend.turn(TurnRequest::new("m")).await.unwrap();
        assert_eq!(out.text, "hello ");
    }

    #[tokio::test]
    async fn headless_backend_reports_spawn_failure() {
        let mut cfg = HeadlessConfig::new(HeadlessKind::Codex, std::env::temp_dir());
        cfg.program = "synapse-no-such-headless-binary-xyz".into();
        cfg.args = Vec::new();
        let backend = HeadlessBackend::new(cfg);
        let err = backend
            .turn(TurnRequest::new("m"))
            .await
            .expect_err("must fail");
        assert!(matches!(err, LlmError::Provider(_)), "{err:?}");
    }

    #[tokio::test]
    async fn headless_cancellation_kills_slow_child() {
        let script = fake_script("import time\ntime.sleep(30)\n");
        let mut cfg = HeadlessConfig::new(HeadlessKind::Gemini, std::env::temp_dir());
        cfg.program = "python3".into();
        cfg.args = vec![script.to_string_lossy().into_owned()];
        let backend = HeadlessBackend::new(cfg);
        let cancel = CancellationToken::new();
        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            canceller.cancel();
        });
        let err = backend
            .turn_stream_cancelable(TurnRequest::new("m"), &cancel)
            .await
            .expect_err("must cancel");
        assert_eq!(err, HeadlessError::Cancelled);
        let mapped = LlmError::from(err);
        assert!(matches!(mapped, LlmError::Stream(_)));
    }

    #[test]
    fn headless_argv_appends_prompt_per_cli() {
        for kind in [
            HeadlessKind::Claude,
            HeadlessKind::Codex,
            HeadlessKind::Opencode,
            HeadlessKind::Gemini,
        ] {
            let cfg = HeadlessConfig::new(kind, PathBuf::from("/tmp"));
            let (program, args) = cfg.argv_for("hello");
            assert_eq!(program, kind.program());
            assert_eq!(args.last().map(String::as_str), Some("hello"));
        }
        // Prompt rendering carries system + tool results into the CLI arg.
        let mut req = TurnRequest::new("m")
            .with_system("sys")
            .with_message(TurnMessage::user("q"));
        req.tool_results.push(TurnMessage::tool_result("c1", "42"));
        let prompt = render_prompt(&req);
        assert!(
            prompt.contains("sys") && prompt.contains("c1"),
            "{prompt:?}"
        );
    }
}
