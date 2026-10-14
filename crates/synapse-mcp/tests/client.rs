//! MCP client manager tests: mock server + fake OAuth flow.

use std::time::Duration;

use serde_json::json;
use synapse_mcp::{
    backend::{MockBackendFactory, MockMcpServer, ToolAnnotations, ToolInfo},
    config::{McpServerConfig, OAuthServerConfig, Transport},
    manager::McpClientManager,
    oauth::{FakeOAuthFlow, InMemorySecretStore},
    McpError,
};

fn tool(name: &str, read_only: Option<bool>) -> ToolInfo {
    ToolInfo {
        name: name.into(),
        description: format!("{name} tool"),
        input_schema: json!({"type": "object"}),
        annotations: ToolAnnotations { read_only },
    }
}

fn stdio_config(id: &str) -> McpServerConfig {
    McpServerConfig {
        id: id.into(),
        name: id.into(),
        transport: Transport::Stdio {
            command: "mock-server".into(),
            args: vec![],
            env: Default::default(),
        },
        enabled: true,
        oauth: None,
    }
}

fn oauth_config(id: &str) -> McpServerConfig {
    McpServerConfig {
        id: id.into(),
        name: id.into(),
        transport: Transport::StreamableHttp {
            url: "https://mcp.example.test/mcp".into(),
            headers: Default::default(),
        },
        enabled: true,
        oauth: Some(OAuthServerConfig {
            authorization_endpoint: "https://auth.example.test/authorize".into(),
            token_endpoint: "https://auth.example.test/token".into(),
            client_id: "synapse-test".into(),
            scopes: vec!["tools".into()],
            key_ref: String::new(),
        }),
    }
}

#[tokio::test]
async fn client_connects_lazily_and_lists_tools() {
    let factory = MockBackendFactory::new();
    factory.add_server(MockMcpServer::new(
        "docs",
        vec![
            tool("search_docs", Some(true)),
            tool("delete_page", Some(false)),
        ],
    ));
    let manager = McpClientManager::new(
        InMemorySecretStore::new(),
        FakeOAuthFlow::new(),
        factory,
        Duration::from_secs(300),
    );
    manager.register(stdio_config("docs"));

    // Lazy: registering must not connect.
    assert_eq!(manager.connected_servers(), Vec::<String>::new());

    let tools = manager.tools_for_agent("analyst").await.expect("list");
    assert_eq!(tools.len(), 2);
    assert_eq!(manager.connected_servers(), vec!["docs".to_string()]);

    let read_only: Vec<&str> = tools
        .iter()
        .filter(|t| t.read_only)
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(read_only, vec!["search_docs"]);
}

#[tokio::test]
async fn client_enforces_toggles_per_agent() {
    let factory = MockBackendFactory::new();
    factory.add_server(MockMcpServer::new(
        "docs",
        vec![
            tool("search_docs", Some(true)),
            tool("delete_page", Some(false)),
        ],
    ));
    let manager = McpClientManager::new(
        InMemorySecretStore::new(),
        FakeOAuthFlow::new(),
        factory,
        Duration::from_secs(300),
    );
    manager.register(stdio_config("docs"));

    manager.set_tool_enabled("docs", "delete_page", false);
    let tools = manager.tools_for_agent("analyst").await.expect("list");
    assert!(tools.iter().all(|t| t.name != "delete_page"));

    // Re-enable globally but deny for one agent.
    manager.set_tool_enabled("docs", "delete_page", true);
    manager.set_agent_tool_enabled("intern", "docs", "delete_page", false);
    let intern = manager.tools_for_agent("intern").await.expect("list");
    assert!(intern.iter().all(|t| t.name != "delete_page"));
    let analyst = manager.tools_for_agent("analyst").await.expect("list");
    assert!(analyst.iter().any(|t| t.name == "delete_page"));

    // Disabled tools cannot be called even directly.
    let err = manager
        .call_tool("intern", "docs", "delete_page", json!({}))
        .await
        .expect_err("denied");
    assert!(matches!(err, McpError::ToolDisabled { .. }));

    // Disabled servers refuse to connect.
    manager.set_server_enabled("docs", false);
    let err = manager.list_tools("docs").await.expect_err("disabled");
    assert!(matches!(err, McpError::Disabled(_)));
}

#[tokio::test]
async fn client_oauth_flow_stores_token_in_secret_store() {
    let secrets = InMemorySecretStore::new();
    let factory = MockBackendFactory::new();
    factory.add_server(MockMcpServer::new(
        "remote",
        vec![tool("search_docs", Some(true))],
    ));
    let manager = McpClientManager::new(
        secrets.clone(),
        FakeOAuthFlow::new(),
        factory,
        Duration::from_secs(300),
    );
    manager.register(oauth_config("remote"));

    assert!(manager
        .begin_oauth("remote")
        .expect("url")
        .contains("remote"));

    // No token yet: lazy connect must fail closed, not start OAuth by itself.
    let err = manager.list_tools("remote").await.expect_err("no token");
    assert!(matches!(err, McpError::MissingToken(_)));

    // Fake browser flow: wrong code rejected, right code stored under key_ref only.
    assert!(manager.complete_oauth("remote", "wrong").is_err());
    let key_ref = manager
        .complete_oauth("remote", &FakeOAuthFlow::expected_code("remote"))
        .expect("exchange");
    assert_eq!(key_ref, "mcp-oauth:remote");
    assert_eq!(
        secrets.test_value(&key_ref).expect("stored"),
        FakeOAuthFlow::issued_token("remote")
    );

    let tools = manager.list_tools("remote").await.expect("authed list");
    assert_eq!(tools.len(), 1);

    // Forgetting clears the secret.
    manager.forget_oauth("remote");
    assert!(secrets.test_value(&key_ref).is_none());
}

#[tokio::test]
async fn client_shuts_down_idle_servers() {
    let factory = MockBackendFactory::new();
    factory.add_server(MockMcpServer::new(
        "docs",
        vec![tool("search_docs", Some(true))],
    ));
    let manager = McpClientManager::new(
        InMemorySecretStore::new(),
        FakeOAuthFlow::new(),
        factory,
        Duration::from_millis(50),
    );
    manager.register(stdio_config("docs"));
    manager.list_tools("docs").await.expect("connect");
    assert!(manager.is_connected("docs"));

    tokio::time::sleep(Duration::from_millis(80)).await;
    let shut = manager.shutdown_idle();
    assert_eq!(shut, vec!["docs".to_string()]);
    assert!(!manager.is_connected("docs"));

    // Next use reconnects lazily.
    manager.list_tools("docs").await.expect("reconnect");
    assert!(manager.is_connected("docs"));
    manager.disconnect("docs");
    assert!(!manager.is_connected("docs"));
}

#[tokio::test]
async fn client_imports_claude_style_config() {
    let json = r#"{"mcpServers": {
        "local": {"command": "npx", "args": ["-y", "x-mcp"]},
        "remote": {"url": "https://mcp.example.test/mcp"}
    }}"#;
    let parsed: Vec<McpServerConfig> =
        synapse_mcp::config::parse_mcp_servers_json(json).expect("parse");
    assert_eq!(parsed.len(), 2);
    assert!(parsed
        .iter()
        .any(|c| matches!(c.transport, Transport::Stdio { .. })));
    assert!(parsed
        .iter()
        .any(|c| matches!(c.transport, Transport::StreamableHttp { .. })));
}

#[tokio::test]
async fn client_read_only_heuristic_for_unannotated_tools() {
    let factory = MockBackendFactory::new();
    factory.add_server(MockMcpServer::new(
        "wiki",
        vec![tool("list_pages", None), tool("write_page", None)],
    ));
    let manager = McpClientManager::new(
        InMemorySecretStore::new(),
        FakeOAuthFlow::new(),
        factory,
        Duration::from_secs(300),
    );
    manager.register(stdio_config("wiki"));
    let auto = manager
        .read_only_tools_for_agent("analyst")
        .await
        .expect("auto");
    assert_eq!(auto.len(), 1);
    assert_eq!(auto[0].name, "list_pages");
}
