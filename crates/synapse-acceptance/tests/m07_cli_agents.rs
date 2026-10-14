//! M7 CLI agents acceptance test (see docs/MILESTONES.md §M7).
//!
//! Offline end-to-end through the real `synapse-acp` crate: CLI detection
//! and selection into a fake ACP backend (spawn, session, streamed output
//! with usage), the permission bridge (allow/ask/deny + project-root
//! confinement), team tools for CLI agents over the in-process MCP server,
//! and headless CLI output parsing. No installed CLIs, no credentials,
//! no network.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use synapse_acp::{
    acp_tool_request, config_for_kind, config_from_adapter, default_headless_cwd,
    detect::detect_in_dirs, handle_agent_request, parse_headless_output, team_mcp_server,
    AcpBackend, AcpConfig, AcpSession, AvailableAdapter, HeadlessBackend, HeadlessConfig,
    HeadlessKind, LoginState, PermissionBridge, TeamMcpServer,
};
use synapse_core::TeamBus;
use synapse_llm::{AgentEvent, ModelBackend, TurnMessage, TurnRequest};
use synapse_tools::permissions::{Decision, Effect, Engine, Rule};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn tmp(prefix: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "synapse-m07-{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(unix)]
fn write_exe(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    let mut perms = std::fs::metadata(&p).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&p, perms).unwrap();
    p
}

/// Minimal fake ACP agent speaking line-delimited JSON-RPC over stdio:
/// answers `initialize` + `session/new`, then streams two text chunks and a
/// tool call for every `session/prompt`, reporting usage in `_meta`.
const FAKE_ACP: &str = r#"
import sys, json
def send(o):
    sys.stdout.write(json.dumps(o) + "\n")
    sys.stdout.flush()
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
              "result": {"sessionId": "sess_m07"}})
    elif m == "session/prompt":
        sid = msg["params"]["sessionId"]
        send({"jsonrpc": "2.0", "method": "session/update",
              "params": {"sessionId": sid, "update": {
                  "sessionUpdate": "agent_message_chunk",
                  "content": {"type": "text", "text": "drafting "}}}})
        send({"jsonrpc": "2.0", "method": "session/update",
              "params": {"sessionId": sid, "update": {
                  "sessionUpdate": "agent_message_chunk",
                  "content": {"type": "text", "text": "report"}}}})
        send({"jsonrpc": "2.0", "method": "session/update",
              "params": {"sessionId": sid, "update": {
                  "sessionUpdate": "tool_call", "toolCallId": "call_m07",
                  "title": "report_finding",
                  "rawInput": {"text": "X causes Y."}}}})
        send({"jsonrpc": "2.0", "id": mid,
              "result": {"stopReason": "end_turn",
                         "_meta": {"usage": {"inputTokens": 9,
                                             "outputTokens": 5}}}})
"#;

fn fake_acp_config() -> (AcpConfig, PathBuf) {
    let dir = tmp("acp");
    let script = dir.join("fake_acp.py");
    std::fs::write(&script, FAKE_ACP).unwrap();
    let mut cfg = AcpConfig::new("python3", dir.clone());
    cfg.args = vec![script.to_string_lossy().into_owned()];
    (cfg, dir)
}

fn user_req() -> TurnRequest {
    TurnRequest::new("cli-agent").with_message(TurnMessage::user("Draft the report."))
}

// ---------------------------------------------------------------------------
// 1. detection and selection into an ACP backend
// ---------------------------------------------------------------------------

#[tokio::test]
async fn m07_detection_and_selection_into_acp_backend() {
    let bin = tmp("bin");
    let home = tmp("home");
    write_exe(&bin, "claude", "#!/bin/sh\necho 2.1.0\n");
    write_exe(&bin, "npx", "#!/bin/sh\necho 10\n");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(home.join(".claude/.credentials.json"), "{}").unwrap();

    // Detection finds the installed CLI with its adapter + login state.
    let found = detect_in_dirs(std::slice::from_ref(&bin), &home).await;
    let ids: Vec<&str> = found.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"claude"), "found: {ids:?}");
    let claude = found.iter().find(|a| a.id == "claude").unwrap();
    assert_eq!(claude.cli_path, bin.join("claude"));
    assert!(claude.adapter.available, "npx adapter must be runnable");
    assert_eq!(claude.login, LoginState::LoggedIn);
    // Unsupported CLIs are absent even though detection ran.
    assert!(!ids.contains(&"codex"), "found: {ids:?}");

    // Selection: the picker row (adapter command + args) becomes an
    // AcpConfig rooted at the project folder, advertising the team-tools
    // MCP server so the CLI agent can ask/delegate/report.
    let cwd = tmp("proj");
    let adapter = AvailableAdapter {
        command: claude.adapter.command.clone(),
        args: claude.adapter.args.clone(),
        available: true,
    };
    let entry = team_mcp_server(None, vec![("SYNAPSE_RUN_ID".into(), "run1".into())]);
    let mut config = config_from_adapter(&adapter.command, &adapter.args, &cwd);
    config.mcp_servers = vec![entry];
    assert_eq!(config.program, "npx");
    assert_eq!(config.cwd, cwd);
    assert_eq!(config.mcp_servers.len(), 1);
    let backend = AcpBackend::new(config);
    assert_eq!(backend.config().program, "npx");

    // A CLI without a login marker reports Unknown rather than guessing.
    let home2 = tmp("home2");
    write_exe(&bin, "goose", "#!/bin/sh\necho goose 1.0\n");
    let found = detect_in_dirs(std::slice::from_ref(&bin), &home2).await;
    let goose = found.iter().find(|a| a.id == "goose").unwrap();
    assert_eq!(goose.login, LoginState::Unknown);
}

// ---------------------------------------------------------------------------
// 2. fake ACP backend: streamed output, tool call, usage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn m07_acp_backend_streams_text_tool_call_and_usage() {
    let (cfg, _dir) = fake_acp_config();
    let mut session = AcpSession::connect(&cfg).await.expect("connect");
    assert_eq!(session.session_id(), "sess_m07");

    let events = session
        .prompt(&user_req(), &CancellationToken::new())
        .await
        .expect("prompt");
    // Streamed text arrives as deltas before the terminal Done.
    let streamed: String = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta(d) => Some(d.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(streamed, "drafting report");
    // The agent's tool call surfaces for the team-tools path below.
    let call = events.iter().find_map(|e| match e {
        AgentEvent::ToolCall(c) => Some(c.clone()),
        _ => None,
    });
    let call = call.expect("tool call event");
    assert_eq!(call.id, "call_m07");
    assert_eq!(call.name, "report_finding");
    assert_eq!(call.args, json!({"text": "X causes Y."}));
    // Terminal Done carries the full text + reported usage.
    let done = events.iter().find_map(|e| match e {
        AgentEvent::Done {
            text,
            tool_calls,
            usage,
        } => Some((text.clone(), tool_calls.len(), *usage)),
        _ => None,
    });
    let (text, n_calls, usage) = done.expect("Done event");
    assert_eq!(text, "drafting report");
    assert_eq!(n_calls, 1);
    assert_eq!(usage.input_tokens, 9);
    assert_eq!(usage.output_tokens, 5);
    assert_eq!(usage.total_tokens, 14);

    // Same flow through the ModelBackend trait the orchestrator drives.
    let (cfg, _dir) = fake_acp_config();
    let backend = AcpBackend::new(cfg);
    let out = backend.turn(user_req()).await.expect("turn");
    assert_eq!(out.text, "drafting report");
    assert_eq!(out.tool_calls.len(), 1);
    assert_eq!(out.usage.total_tokens, 14);
}

// ---------------------------------------------------------------------------
// 3. permission bridge + team tools for CLI agents
// ---------------------------------------------------------------------------

#[test]
fn m07_cli_agent_permission_bridge_and_team_tools() {
    let root = tmp("permroot");
    // CLI agents share the built-in rule engine: deny wins, writes ask.
    let engine = Engine::with_rules(vec![
        Rule::global("shell", "echo *", Effect::Allow),
        Rule::global("shell", "cargo test", Effect::Allow),
        Rule::global("shell", "cargo publish *", Effect::Deny),
        Rule::global("read_file", "*", Effect::Allow),
    ]);
    let bridge = PermissionBridge::new(engine, root.clone());

    // Reads inside the project allow; destructive shell denies.
    assert_eq!(
        bridge.check_fs_read(&root.join("notes.md")),
        Decision::Allow
    );
    assert_eq!(bridge.check_terminal("rm -rf /", &[], None), Decision::Deny);
    // Writes ask by default → the approval card (NeedsApproval), never
    // auto-answered; out-of-root paths deny even with a broad allow.
    assert_eq!(bridge.check_fs_write(&root.join("notes.md")), Decision::Ask);
    assert_eq!(
        bridge.check_fs_read(Path::new("/etc/passwd")),
        Decision::Deny
    );
    assert_eq!(
        bridge.check_fs_write(Path::new("/tmp/m07-evil.md")),
        Decision::Deny
    );
    assert!(!bridge.path_in_root(Path::new("/proj/../etc/passwd")));

    // The JSON dispatcher answers agent→client methods through the bridge:
    // out-of-root reads/terminals deny instead of leaking or running.
    let err = handle_agent_request(
        &bridge,
        "fs/read_text_file",
        &json!({"sessionId": "s1", "path": "/etc/passwd"}),
    )
    .expect_err("out-of-root read must deny");
    assert!(err.contains("denied"), "{err}");
    let err = handle_agent_request(
        &bridge,
        "terminal/create",
        &json!({"sessionId": "s1", "command": "ls", "args": [], "cwd": "/tmp"}),
    )
    .expect_err("out-of-root cwd must deny");
    assert!(err.contains("denied"), "{err}");

    // ACP tool-call mapping feeds the same engine (read → allow rule).
    let raw: Value = json!({"path": "reports/q.md"});
    let req = acp_tool_request(
        "Read notes",
        agent_client_protocol::ToolKind::Read,
        Some(&raw),
        &[],
    );
    assert_eq!(req.tool, "read_file");
    assert_eq!(bridge.evaluate(&req), Decision::Allow);

    // Team tools: the CLI agent's reported finding lands on the bus as a
    // typed room message; ask/answer flows work like built-in members.
    let bus = TeamBus::new();
    let mira = TeamMcpServer::new(&bus, "mira-cli", "run1", "c1");
    let lead = TeamMcpServer::new(&bus, "lead", "run1", "c1");
    let names = mira.tool_names();
    for want in [
        "say",
        "ask_agent",
        "answer",
        "delegate",
        "report_finding",
        "done",
    ] {
        assert!(
            names.contains(&want.to_string()),
            "missing team tool {want}"
        );
    }
    let out = mira
        .call_tool(
            "report_finding",
            json!({"text": "X causes Y.", "confidence": 0.8}),
        )
        .expect("finding");
    assert_eq!(out["body"], json!("X causes Y."));
    let ask = lead
        .call_tool(
            "ask_agent",
            json!({"agent": "mira-cli", "question": "Summarise?"}),
        )
        .expect("ask");
    let ask_id = ask["message_id"].as_str().expect("ask id").to_string();
    mira.call_tool(
        "answer",
        json!({"message_id": ask_id, "text": "It shows X."}),
    )
    .expect("answer");
    assert_eq!(bus.history("c1", 0, usize::MAX).len(), 3);
    // Unknown tools and blank fields store nothing.
    assert!(mira.call_tool("teleport", json!({})).is_err());
    let before = bus.history("c1", 0, usize::MAX).len();
    assert!(mira.call_tool("say", json!({"text": "  "})).is_err());
    assert_eq!(bus.history("c1", 0, usize::MAX).len(), before);
}

// ---------------------------------------------------------------------------
// 4. headless fallback parses CLI JSON streams offline
// ---------------------------------------------------------------------------

#[test]
fn m07_headless_fallback_parses_cli_streams() {
    // Claude stream-json: text deltas close with the result text.
    let stdout = concat!(
        "{\"type\": \"assistant\", \"message\": {\"content\": [{\"type\": \"text\", \"text\": \"hello \"}]}}\n",
        "{\"type\": \"assistant\", \"message\": {\"content\": [{\"type\": \"text\", \"text\": \"world\"}]}}\n",
        "{\"type\": \"result\", \"subtype\": \"success\", \"result\": \"hello world\"}\n",
    );
    let events = parse_headless_output(HeadlessKind::Claude, stdout).expect("parse");
    let done = events.iter().find_map(|e| match e {
        AgentEvent::Done { text, .. } => Some(text.clone()),
        _ => None,
    });
    assert_eq!(done.as_deref(), Some("hello world"));

    // Codex exec --json: agent message + completion marker without inline
    // text falls back to the accumulated deltas.
    let events = parse_headless_output(
        HeadlessKind::Codex,
        "{\"type\": \"item.completed\", \"item\": {\"type\": \"agent_message\", \"text\": \"abc\"}}\n{\"type\": \"turn.completed\"}\n",
    )
    .expect("parse");
    let done = events.iter().find_map(|e| match e {
        AgentEvent::Done { text, .. } => Some(text.clone()),
        _ => None,
    });
    assert_eq!(done.as_deref(), Some("abc"));

    // The headless backend runs a fake CLI binary and reports usage;
    // default argv carries the documented flags with the prompt appended.
    let cwd = default_headless_cwd();
    for kind in [
        HeadlessKind::Claude,
        HeadlessKind::Codex,
        HeadlessKind::Opencode,
        HeadlessKind::Gemini,
    ] {
        let cfg = config_for_kind(kind, &cwd);
        let (program, args) = cfg.argv_for("hello");
        assert_eq!(program, kind.program());
        assert_eq!(args.last().map(String::as_str), Some("hello"));
    }
}

#[tokio::test]
async fn m07_headless_backend_runs_fake_cli_to_done() {
    let dir = tmp("headless");
    let script = dir.join("fake_claude.py");
    std::fs::write(
        &script,
        concat!(
            "import json\n",
            "print(json.dumps({'type':'assistant','message':{'content':[{'type':'text','text':'hello '}]} }), flush=True)\n",
            "print(json.dumps({'type':'result','subtype':'success','result':'hello '}), flush=True)\n",
        ),
    )
    .unwrap();
    let mut cfg = HeadlessConfig::new(HeadlessKind::Claude, dir.clone());
    cfg.program = "python3".into();
    cfg.args = vec![script.to_string_lossy().into_owned()];
    let backend = HeadlessBackend::new(cfg);
    let out: synapse_llm::TurnOutput = ModelBackend::turn(
        &backend,
        TurnRequest::new("m").with_message(TurnMessage::user("hi")),
    )
    .await
    .expect("headless turn");
    assert_eq!(out.text, "hello ");
    let _ = Arc::new(());
}
