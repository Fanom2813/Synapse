//! ACP permission bridge (see docs/TEAM-PROTOCOL.md, docs/PRD.md §5.5).
//!
//! CLI agents ask for permission over ACP (`session/request_permission`)
//! and touch files/terminals through `fs/*` and `terminal/*` client
//! methods. This module maps those requests onto the same
//! [`synapse_tools::permissions`] engine the built-in tools use, so one
//! rule set (deny > ask > allow) governs both, and confines every path to
//! the permitted project root.
//!
//! - [`acp_tool_request`] maps an ACP tool call (title + kind +
//!   raw input) onto a [`Request`] (tool + target).
//! - [`PermissionBridge`] evaluates requests, picks the ACP option that
//!   matches the [`Decision`], and gates `fs/*` / `terminal/*` on the
//!   project root. `Ask` means "surface an approval card": [`resolve`]
//!   returns `None` so the caller can pend instead of auto-answering.
//! - [`handle_agent_request`] is the JSON dispatcher the ACP client uses
//!   to answer agent→client methods safely.

use std::path::{Component, Path, PathBuf};

use agent_client_protocol as acp;
use synapse_tools::permissions::{Decision, Engine, Request};

/// What the UI should do with an ACP permission request.
#[derive(Debug, Clone)]
pub enum AcpApproval {
    /// Auto-answer with this option (engine allowed or denied outright).
    Decided(acp::RequestPermissionOutcome),
    /// Engine said `Ask`: show the same approval card as built-in tools.
    /// `tool`/`target` feed the card preview; `options` are the agent's.
    NeedsApproval {
        tool: String,
        target: String,
        options: Vec<acp::PermissionOption>,
    },
}

/// Map an ACP tool call onto a permission [`Request`].
///
/// `title` is free text ("Read notes.md", "cargo test"); `kind` carries
/// the structured hint; `raw_input` may hold `path`/`file`/`command`
/// keys; `locations` holds touched file paths.
pub fn acp_tool_request(
    title: &str,
    kind: acp::ToolKind,
    raw_input: Option<&serde_json::Value>,
    locations: &[acp::ToolCallLocation],
) -> Request {
    let (tool, fallback_target) = match kind {
        acp::ToolKind::Read => ("read_file", String::new()),
        acp::ToolKind::Edit => ("write_file", String::new()),
        acp::ToolKind::Delete => ("delete_file", String::new()),
        acp::ToolKind::Move => ("write_file", String::new()),
        acp::ToolKind::Execute => ("shell", String::new()),
        acp::ToolKind::Search | acp::ToolKind::Fetch => ("read_file", String::new()),
        acp::ToolKind::Think | acp::ToolKind::SwitchMode | acp::ToolKind::Other => {
            (infer_tool_from_title(title), String::new())
        }
    };
    let mut target = raw_target(raw_input).unwrap_or(fallback_target);
    if target.is_empty() {
        target = locations
            .first()
            .map(|l| l.path.to_string_lossy().into_owned())
            .unwrap_or_else(|| title.trim().to_string());
    }
    Request::new(tool, &target)
}

/// Guess the built-in tool name from a free-text title when `kind` is
/// `Other` (agents set titles like "Run cargo test").
fn infer_tool_from_title(title: &str) -> &'static str {
    let t = title.trim().to_lowercase();
    if t.starts_with("read") || t.contains("read file") {
        "read_file"
    } else if t.starts_with("list") || t.contains("list files") {
        "list_files"
    } else if t.starts_with("write") || t.starts_with("edit") || t.contains("write file") {
        "write_file"
    } else if t.starts_with("delete") || t.starts_with("remove") || t.starts_with("rm ") {
        "delete_file"
    } else if t.starts_with("run")
        || t.starts_with("execute")
        || t.starts_with("shell")
        || t.starts_with("cargo")
        || t.starts_with("git ")
        || t.starts_with("npm ")
        || t.starts_with("bun ")
    {
        "shell"
    } else {
        "write_file"
    }
}

fn raw_target(raw: Option<&serde_json::Value>) -> Option<String> {
    let v = raw?;
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    let obj = v.as_object()?;
    for key in [
        "path",
        "file",
        "filePath",
        "file_path",
        "command",
        "cmd",
        "input",
        "target",
    ] {
        if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

/// Permission bridge: one rule engine scoped to one project root / agent.
#[derive(Debug, Clone)]
pub struct PermissionBridge {
    engine: Engine,
    root: PathBuf,
    project_id: Option<String>,
    agent_id: Option<String>,
}

impl PermissionBridge {
    pub fn new(engine: Engine, root: impl Into<PathBuf>) -> Self {
        Self {
            engine,
            root: root.into(),
            project_id: None,
            agent_id: None,
        }
    }

    pub fn with_project(mut self, project_id: &str) -> Self {
        self.project_id = Some(project_id.to_string());
        self
    }

    pub fn with_agent(mut self, agent_id: &str) -> Self {
        self.agent_id = Some(agent_id.to_string());
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn scoped(&self, mut req: Request) -> Request {
        if let Some(p) = &self.project_id {
            req.project_id = Some(p.clone());
        }
        if let Some(a) = &self.agent_id {
            req.agent_id = Some(a.clone());
        }
        req
    }

    /// Evaluate a built-in-style request through the engine.
    pub fn evaluate(&self, req: &Request) -> Decision {
        self.engine.evaluate(&self.scoped(req.clone()))
    }

    /// Is `path` (absolute or project-relative) inside the project root?
    pub fn path_in_root(&self, path: &Path) -> bool {
        let abs = if path.is_absolute() {
            normalize(path)
        } else {
            normalize(&self.root.join(path))
        };
        let root = normalize(&self.root);
        abs == root || abs.starts_with(&root)
    }

    /// Gate an `fs/read_text_file` path: outside the root always denies,
    /// otherwise the engine decides (reads allow by default).
    pub fn check_fs_read(&self, path: &Path) -> Decision {
        if !self.path_in_root(path) {
            return Decision::Deny;
        }
        let rel = display_target(&self.root, path);
        self.evaluate(&Request::new("read_file", &rel))
    }

    /// Gate an `fs/write_text_file` path: outside the root always denies,
    /// otherwise the engine decides (writes ask by default → approval).
    pub fn check_fs_write(&self, path: &Path) -> Decision {
        if !self.path_in_root(path) {
            return Decision::Deny;
        }
        let rel = display_target(&self.root, path);
        self.evaluate(&Request::new("write_file", &rel))
    }

    /// Gate a `terminal/create` command: a `cwd` outside the root denies,
    /// otherwise the engine decides on the full command line
    /// (destructive commands deny, the rest ask by default → approval).
    pub fn check_terminal(&self, command: &str, args: &[String], cwd: Option<&Path>) -> Decision {
        if let Some(dir) = cwd {
            if !self.path_in_root(dir) {
                return Decision::Deny;
            }
        }
        let mut full = command.to_string();
        for a in args {
            full.push(' ');
            full.push_str(a);
        }
        self.evaluate(&Request::new("shell", &full))
    }

    /// Declared client capabilities, confined to what the rules permit:
    /// each capability is advertised only when the matching tool is not
    /// blanket-denied. Paths/commands stay gated per call by the
    /// `check_*` functions above, so the agent can never escape the
    /// project root even when a capability is on.
    pub fn client_capabilities(&self) -> acp::ClientCapabilities {
        let denied = |tool: &str| self.evaluate(&Request::new(tool, "*")) == Decision::Deny;
        acp::ClientCapabilities {
            fs: acp::FileSystemCapability {
                read_text_file: !denied("read_file"),
                write_text_file: !denied("write_file"),
                meta: None,
            },
            terminal: !denied("shell"),
            meta: None,
        }
    }

    /// Route a `session/request_permission` call: `Allow` picks the first
    /// allow option, `Deny` the first reject option, `Ask` returns
    /// [`AcpApproval::NeedsApproval`] so the caller shows the same
    /// approval card as built-in tools instead of auto-answering.
    pub fn resolve(&self, req: &acp::RequestPermissionRequest) -> AcpApproval {
        let title = req
            .tool_call
            .fields
            .title
            .clone()
            .unwrap_or_else(|| "agent action".to_string());
        let kind = req.tool_call.fields.kind.unwrap_or(acp::ToolKind::Other);
        let permission_req = self.scoped(acp_tool_request(
            &title,
            kind,
            req.tool_call.fields.raw_input.as_ref(),
            req.tool_call.fields.locations.as_deref().unwrap_or(&[]),
        ));
        // Root confinement applies before the engine: an out-of-root path
        // denies even when a broad allow rule matches.
        let target_path = PathBuf::from(permission_req.target.clone());
        let confined_deny = is_path_like(&permission_req.tool, &permission_req.target)
            && !target_path.as_os_str().is_empty()
            && !self.path_in_root(&target_path);
        let decision = if confined_deny {
            Decision::Deny
        } else {
            self.engine.evaluate(&permission_req)
        };
        match decision {
            Decision::Allow => AcpApproval::Decided(pick_option(&req.options, true)),
            Decision::Deny => AcpApproval::Decided(pick_option(&req.options, false)),
            Decision::Ask => AcpApproval::NeedsApproval {
                tool: permission_req.tool,
                target: permission_req.target,
                options: req.options.clone(),
            },
        }
    }
}

/// Pick the agent's allow (or reject) option for an auto-decision.
/// Prefers the `*Once` variant; falls back to the first option of that
/// polarity, then to any option, else `Cancelled`.
fn pick_option(options: &[acp::PermissionOption], allow: bool) -> acp::RequestPermissionOutcome {
    use acp::PermissionOptionKind as Kind;
    let want = if allow {
        [Kind::AllowOnce, Kind::AllowAlways]
    } else {
        [Kind::RejectOnce, Kind::RejectAlways]
    };
    for kind in want {
        if let Some(o) = options.iter().find(|o| o.kind == kind) {
            return acp::RequestPermissionOutcome::Selected {
                option_id: o.id.clone(),
            };
        }
    }
    // Any same-polarity option, then anything, else cancelled.
    let same_polarity = options
        .iter()
        .find(|o| matches!(o.kind, Kind::AllowOnce | Kind::AllowAlways) == allow);
    if let Some(o) = same_polarity.or_else(|| options.first()) {
        return acp::RequestPermissionOutcome::Selected {
            option_id: o.id.clone(),
        };
    }
    acp::RequestPermissionOutcome::Cancelled
}

fn is_path_like(tool: &str, target: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    let t = tool.to_ascii_lowercase();
    t.contains("read") || t.contains("write") || t.contains("edit") || t.contains("delete")
}

/// JSON dispatcher answering agent→client methods against the bridge.
/// Returns the `result` value, or an error string for denials / unknown
/// methods (the caller turns it into a JSON-RPC error).
pub fn handle_agent_request(
    bridge: &PermissionBridge,
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    match method {
        "session/request_permission" => {
            let req: acp::RequestPermissionRequest = serde_json::from_value(params.clone())
                .map_err(|e| format!("bad request_permission params: {e}"))?;
            match bridge.resolve(&req) {
                AcpApproval::Decided(outcome) => {
                    let resp = acp::RequestPermissionResponse {
                        outcome,
                        meta: None,
                    };
                    serde_json::to_value(resp).map_err(|e| e.to_string())
                }
                // Headless default: no UI to approve, so fail closed.
                AcpApproval::NeedsApproval { .. } => {
                    let first_reject = req
                        .options
                        .iter()
                        .find(|o| {
                            matches!(
                                o.kind,
                                acp::PermissionOptionKind::RejectOnce
                                    | acp::PermissionOptionKind::RejectAlways
                            )
                        })
                        .or_else(|| req.options.first());
                    let outcome = match first_reject {
                        Some(o) => acp::RequestPermissionOutcome::Selected {
                            option_id: o.id.clone(),
                        },
                        None => acp::RequestPermissionOutcome::Cancelled,
                    };
                    let resp = acp::RequestPermissionResponse {
                        outcome,
                        meta: None,
                    };
                    serde_json::to_value(resp).map_err(|e| e.to_string())
                }
            }
        }
        "fs/read_text_file" => {
            let req: acp::ReadTextFileRequest = serde_json::from_value(params.clone())
                .map_err(|e| format!("bad read_text_file params: {e}"))?;
            match bridge.check_fs_read(&req.path) {
                Decision::Allow => Err("fs/read_text_file needs a file store".into()),
                Decision::Ask => Err("needs approval".into()),
                Decision::Deny => {
                    Err("denied: path outside the project or blocked by rules".into())
                }
            }
        }
        "fs/write_text_file" => {
            let req: acp::WriteTextFileRequest = serde_json::from_value(params.clone())
                .map_err(|e| format!("bad write_text_file params: {e}"))?;
            match bridge.check_fs_write(&req.path) {
                Decision::Allow => Err("fs/write_text_file needs a file store".into()),
                Decision::Ask => Err("needs approval".into()),
                Decision::Deny => {
                    Err("denied: path outside the project or blocked by rules".into())
                }
            }
        }
        "terminal/create" => {
            let req: acp::CreateTerminalRequest = serde_json::from_value(params.clone())
                .map_err(|e| format!("bad terminal/create params: {e}"))?;
            let full = std::iter::once(req.command.clone())
                .chain(req.args.clone())
                .collect::<Vec<_>>()
                .join(" ");
            match bridge.check_terminal(&req.command, &req.args, req.cwd.as_deref()) {
                Decision::Allow => Err(format!(
                    "terminal/create needs a terminal runner (would run: {full})"
                )),
                Decision::Ask => Err("needs approval".into()),
                Decision::Deny => Err("denied: command blocked or cwd outside the project".into()),
            }
        }
        _ => Err("method not found".into()),
    }
}

/// Lexically normalize a path (resolve `.`/`..` without touching disk).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

fn display_target(root: &Path, path: &Path) -> String {
    let abs = if path.is_absolute() {
        normalize(path)
    } else {
        normalize(&root.join(path))
    };
    let root_n = normalize(root);
    abs.strip_prefix(&root_n)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_else(|_| abs.to_string_lossy().into_owned())
}

#[cfg(test)]
mod permissions_tests {
    use super::*;
    use std::sync::Arc;
    use synapse_tools::permissions::{Effect, Rule};

    fn bridge() -> PermissionBridge {
        PermissionBridge::new(Engine::new(), PathBuf::from("/proj"))
    }

    fn perm_req(
        title: &str,
        kind: acp::ToolKind,
        options: Vec<acp::PermissionOption>,
    ) -> acp::RequestPermissionRequest {
        acp::RequestPermissionRequest {
            session_id: acp::SessionId(Arc::from("s1")),
            tool_call: acp::ToolCallUpdate {
                id: acp::ToolCallId(Arc::from("c1")),
                fields: acp::ToolCallUpdateFields {
                    title: Some(title.into()),
                    kind: Some(kind),
                    ..Default::default()
                },
                meta: None,
            },
            options,
            meta: None,
        }
    }

    fn opt(id: &str, kind: acp::PermissionOptionKind) -> acp::PermissionOption {
        acp::PermissionOption {
            id: acp::PermissionOptionId(Arc::from(id)),
            name: id.into(),
            kind,
            meta: None,
        }
    }

    fn allow_reject_options() -> Vec<acp::PermissionOption> {
        vec![
            opt("allow", acp::PermissionOptionKind::AllowOnce),
            opt("allow-always", acp::PermissionOptionKind::AllowAlways),
            opt("reject", acp::PermissionOptionKind::RejectOnce),
            opt("reject-always", acp::PermissionOptionKind::RejectAlways),
        ]
    }

    #[test]
    fn permissions_read_allows_and_write_needs_approval() {
        let b = bridge();
        let req = perm_req("Read notes.md", acp::ToolKind::Read, allow_reject_options());
        match b.resolve(&req) {
            AcpApproval::Decided(acp::RequestPermissionOutcome::Selected { option_id }) => {
                assert_eq!(option_id.to_string(), "allow");
            }
            other => panic!("read must auto-allow, got {other:?}"),
        }
        // Writes use the same ask-by-default rule as built-in tools.
        let req = perm_req("Edit notes.md", acp::ToolKind::Edit, allow_reject_options());
        match b.resolve(&req) {
            AcpApproval::NeedsApproval { tool, .. } => assert_eq!(tool, "write_file"),
            other => panic!("write must need approval, got {other:?}"),
        }
    }

    #[test]
    fn permissions_deny_wins_and_picks_reject_option() {
        let engine = Engine::with_rules(vec![Rule::global("shell", "*", Effect::Deny)]);
        let b = PermissionBridge::new(engine, PathBuf::from("/proj"));
        let req = perm_req(
            "cargo publish",
            acp::ToolKind::Execute,
            allow_reject_options(),
        );
        match b.resolve(&req) {
            AcpApproval::Decided(acp::RequestPermissionOutcome::Selected { option_id }) => {
                assert_eq!(option_id.to_string(), "reject");
            }
            other => panic!("deny must pick reject, got {other:?}"),
        }
    }

    #[test]
    fn permissions_out_of_root_denies_even_with_allow_rule() {
        let engine = Engine::with_rules(vec![Rule::global("read_file", "*", Effect::Allow)]);
        let b = PermissionBridge::new(engine, PathBuf::from("/proj"));
        assert_eq!(b.check_fs_read(Path::new("/etc/passwd")), Decision::Deny);
        assert_eq!(b.check_fs_write(Path::new("/proj/notes.md")), Decision::Ask);
        assert_eq!(b.check_fs_write(Path::new("/tmp/evil.md")), Decision::Deny);
        // `..` escapes are confined too.
        assert!(!b.path_in_root(Path::new("/proj/../etc/passwd")));
        assert!(b.path_in_root(Path::new("/proj/sub/../notes.md")));
    }

    #[test]
    fn permissions_terminal_confined_to_root_and_rules() {
        let b = bridge();
        // Safe command inside the root asks (approval card), like built-ins.
        assert_eq!(
            b.check_terminal("cargo test", &[], Some(Path::new("/proj"))),
            Decision::Ask
        );
        // Cwd outside the root denies.
        assert_eq!(
            b.check_terminal("cargo test", &[], Some(Path::new("/tmp"))),
            Decision::Deny
        );
        // Destructive commands deny via the shared engine default.
        assert_eq!(b.check_terminal("rm -rf /", &[], None), Decision::Deny);
    }

    #[test]
    fn permissions_capabilities_confined_by_rules() {
        let open = bridge().client_capabilities();
        assert!(open.fs.read_text_file);
        assert!(open.fs.write_text_file);
        assert!(open.terminal);
        let locked = PermissionBridge::new(
            Engine::with_rules(vec![
                Rule::global("read_file", "*", Effect::Deny),
                Rule::global("write_file", "*", Effect::Deny),
                Rule::global("shell", "*", Effect::Deny),
            ]),
            PathBuf::from("/proj"),
        )
        .client_capabilities();
        assert!(!locked.fs.read_text_file);
        assert!(!locked.fs.write_text_file);
        assert!(!locked.terminal);
    }

    #[test]
    fn permissions_dispatcher_answers_requests_safely() {
        let b = bridge();
        // Out-of-root fs read denies instead of leaking contents.
        let params = serde_json::json!({
            "sessionId": "s1",
            "path": "/etc/passwd",
        });
        let err = handle_agent_request(&b, "fs/read_text_file", &params).expect_err("must deny");
        assert!(err.contains("denied"), "{err}");
        // Out-of-root terminal cwd denies.
        let params = serde_json::json!({
            "sessionId": "s1",
            "command": "ls",
            "args": [],
            "cwd": "/tmp",
        });
        let err = handle_agent_request(&b, "terminal/create", &params).expect_err("must deny");
        assert!(err.contains("denied"), "{err}");
        // Unknown methods stay "method not found" for the JSON-RPC layer.
        let err =
            handle_agent_request(&b, "bogus/method", &serde_json::json!({})).expect_err("unknown");
        assert_eq!(err, "method not found");
    }

    #[test]
    fn permissions_request_mapping_uses_kind_and_raw_input() {
        let raw = serde_json::json!({"path": "reports/q.md"});
        let r = acp_tool_request("whatever", acp::ToolKind::Read, Some(&raw), &[]);
        assert_eq!(r.tool, "read_file");
        assert_eq!(r.target, "reports/q.md");
        let raw = serde_json::json!({"command": "cargo test"});
        let r = acp_tool_request("Run tests", acp::ToolKind::Execute, Some(&raw), &[]);
        assert_eq!(r.tool, "shell");
        assert_eq!(r.target, "cargo test");
    }
}
