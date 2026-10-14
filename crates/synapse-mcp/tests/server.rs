//! Synapse as an MCP server: documented tools, local-token HTTP guard,
//! copyable client snippets, and calls verified against a test server.

use secrecy::ExposeSecret;
use serde_json::json;
use synapse_mcp::server::{
    bearer_token, client_snippets, generate_local_token, server_tool_docs, token_preview,
    verify_bearer, ServerError, SynapseServer, SynapseServerSettings, SERVER_TOOL_NAMES,
};

fn enabled_server() -> SynapseServer {
    SynapseServer::new(SynapseServerSettings {
        enabled: true,
        ..Default::default()
    })
}

#[test]
fn server_is_opt_in_disabled_by_default() {
    let server = SynapseServer::default();
    assert!(!server.is_enabled());
    assert!(matches!(
        server.list_tools().unwrap_err(),
        ServerError::Disabled
    ));
    assert!(matches!(
        server.call_tool("list_projects", json!({})).unwrap_err(),
        ServerError::Disabled
    ));
}

#[test]
fn server_exposes_only_documented_tools() {
    let server = enabled_server();
    let tools = server.list_tools().expect("list");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, SERVER_TOOL_NAMES);
    for tool in &tools {
        assert!(
            !tool.description.is_empty(),
            "{} needs a description",
            tool.name
        );
        assert!(
            tool.input_schema.get("type").is_some(),
            "{} needs a schema",
            tool.name
        );
    }
    assert_eq!(server_tool_docs().len(), 5);
}

#[test]
fn server_tool_calls_round_trip() {
    let server = enabled_server();
    let projects = server.call_tool("list_projects", json!({})).expect("list");
    let id = projects["projects"][0]["id"]
        .as_str()
        .expect("id")
        .to_string();

    let run = server
        .call_tool(
            "start_run",
            json!({"project_id": id, "mode": "research", "prompt": "Q?"}),
        )
        .expect("start");
    let run_id = run["run_id"].as_str().expect("run_id").to_string();

    let status = server
        .call_tool("run_status", json!({"run_id": run_id}))
        .expect("status");
    assert_eq!(status["status"], "running");

    let report = server
        .call_tool("get_report", json!({"project_id": id}))
        .expect("report");
    assert!(report["markdown"].as_str().unwrap().contains('#'));

    let reply = server
        .call_tool("ask_agent", json!({"agent": "lead", "question": "Hi?"}))
        .expect("ask");
    assert_eq!(reply["agent"], "lead");

    assert!(matches!(
        server.call_tool("teleport", json!({})).unwrap_err(),
        ServerError::UnknownTool(_)
    ));
    assert!(matches!(
        server
            .call_tool("start_run", json!({"project_id": id}))
            .unwrap_err(),
        ServerError::MissingField { .. }
    ));
}

#[test]
fn server_http_calls_require_local_token() {
    let server = enabled_server();
    let token = generate_local_token();
    let header = format!("Bearer {}", token.expose_secret());
    server.set_token(token.clone());

    // No / wrong token: rejected before any tool runs.
    assert!(matches!(
        server
            .call_tool_http(None, "list_projects", json!({}))
            .unwrap_err(),
        ServerError::Unauthorized
    ));
    assert!(matches!(
        server
            .call_tool_http(Some("Bearer wrong"), "list_projects", json!({}))
            .unwrap_err(),
        ServerError::Unauthorized
    ));

    // Right token: verified against the test server.
    let out = server
        .call_tool_http(Some(&header), "list_projects", json!({}))
        .expect("authed");
    assert!(out.get("projects").is_some());

    assert_eq!(
        bearer_token(Some(&header)).as_deref(),
        Some(token.expose_secret())
    );
    assert!(verify_bearer(Some(&header), &token));
    assert!(!verify_bearer(Some("Bearer wrong"), &token));
    assert!(!token_preview(&token).contains(token.expose_secret()));
}

#[test]
fn server_client_snippets_are_copyable_json() {
    let snippets = client_snippets("synapse-mcp-server", 4317);
    assert_eq!(snippets.len(), 3);
    for (label, stdio, http) in &snippets {
        assert!(!label.is_empty());
        let stdio_parsed: serde_json::Value = serde_json::from_str(stdio).expect("stdio json");
        assert!(stdio_parsed.get("mcpServers").is_some(), "{label} stdio");
        assert!(stdio.contains("synapse-mcp-server"));
        let http_parsed: serde_json::Value = serde_json::from_str(http).expect("http json");
        let raw = http_parsed.to_string();
        assert!(raw.contains("127.0.0.1:4317"), "{label} http port");
        assert!(
            http.contains("Bearer <TOKEN>"),
            "{label} http token placeholder"
        );
    }
    assert!(snippets.iter().any(|(l, _, _)| l.contains("Claude")));
    assert!(snippets.iter().any(|(l, _, _)| l.contains("Cursor")));
    assert!(snippets.iter().any(|(l, _, _)| l.contains("Zed")));
}

#[test]
fn server_jsonrpc_dispatch_verified_against_test_server() {
    let server = enabled_server();
    let list = server.handle_jsonrpc(
        None,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );
    assert!(list.get("result").is_some(), "got: {list}");

    let id = list["result"]["tools"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(id, "list_projects");
    let call = server.handle_jsonrpc(
        None,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": "list_projects", "arguments": {}}}),
    );
    assert!(call.get("result").is_some(), "got: {call}");

    let bad = server.handle_jsonrpc(
        None,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "nope", "arguments": {}}}),
    );
    assert!(bad.get("error").is_some());
}
