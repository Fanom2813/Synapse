//! Permission engine: global → project → agent allow/ask/deny rules.
//!
//! Semantics (docs/PRD.md §5.5, docs/STACK.md): rules are per tool + pattern
//! (path glob, command prefix, MCP tool name). Every rule whose scope, tool
//! and pattern match the request votes; **deny wins**, then ask, then allow.
//! When no rule matches, read-only tools allow and everything risky asks
//! (destructive shell commands deny).

use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};

/// allow / ask / deny (PRD §5.5; deny wins at runtime).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    #[default]
    Ask,
    Allow,
    Deny,
}

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "allow" => Some(Self::Allow),
            "ask" => Some(Self::Ask),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// What the engine decides for one tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

impl Decision {
    pub fn allows(self) -> bool {
        matches!(self, Self::Allow)
    }
    pub fn needs_approval(self) -> bool {
        matches!(self, Self::Ask)
    }
    pub fn denies(self) -> bool {
        matches!(self, Self::Deny)
    }
}

/// Rule scope: global applies everywhere, project/agent only on match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleScope {
    #[default]
    Global,
    Project(String),
    Agent(String),
}

/// One row of the `permission_rules` table
/// (ARCHITECTURE.md §3: scope, scope_id, effect, tool, pattern).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub scope: RuleScope,
    pub tool: String,
    pub pattern: String,
    pub effect: Effect,
}

impl Rule {
    pub fn new(scope: RuleScope, tool: &str, pattern: &str, effect: Effect) -> Self {
        Self {
            scope,
            tool: tool.to_string(),
            pattern: pattern.to_string(),
            effect,
        }
    }

    pub fn global(tool: &str, pattern: &str, effect: Effect) -> Self {
        Self::new(RuleScope::Global, tool, pattern, effect)
    }

    pub fn project(project_id: &str, tool: &str, pattern: &str, effect: Effect) -> Self {
        Self::new(
            RuleScope::Project(project_id.to_string()),
            tool,
            pattern,
            effect,
        )
    }

    pub fn agent(agent_id: &str, tool: &str, pattern: &str, effect: Effect) -> Self {
        Self::new(
            RuleScope::Agent(agent_id.to_string()),
            tool,
            pattern,
            effect,
        )
    }
}

/// One tool call asking for a decision.
///
/// `target` is what the pattern matches against:
/// - file tools (`read_file`, `write_file`, `edit_file`, `delete_file`, `list_files`): the path
/// - `shell`: the full command line (patterns are prefixes or globs)
/// - MCP / other tools: the sub-tool name (or args summary); `*` matches all
/// - tools without a target: empty string
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub tool: String,
    pub target: String,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
}

impl Request {
    pub fn new(tool: &str, target: &str) -> Self {
        Self {
            tool: tool.to_string(),
            target: target.to_string(),
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
}

/// The rule set. Small enough to clone per evaluation context.
#[derive(Debug, Clone, Default)]
pub struct Engine {
    rules: Vec<Rule>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_rules(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    pub fn add(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Evaluate a request. Matching rules vote; deny > ask > allow.
    /// With no matching rule, [`default_for`] decides.
    pub fn evaluate(&self, req: &Request) -> Decision {
        let mut saw_allow = false;
        let mut saw_ask = false;
        for rule in &self.rules {
            if !scope_matches(&rule.scope, req) {
                continue;
            }
            if !tool_matches(&rule.tool, &req.tool) {
                continue;
            }
            if !pattern_matches(&rule.pattern, &req.target) {
                continue;
            }
            match rule.effect {
                Effect::Deny => return Decision::Deny,
                Effect::Ask => saw_ask = true,
                Effect::Allow => saw_allow = true,
            }
        }
        if saw_ask {
            return Decision::Ask;
        }
        if saw_allow {
            return Decision::Allow;
        }
        default_for(&req.tool, &req.target)
    }
}

fn scope_matches(scope: &RuleScope, req: &Request) -> bool {
    match scope {
        RuleScope::Global => true,
        RuleScope::Project(id) => req.project_id.as_deref() == Some(id.as_str()),
        RuleScope::Agent(id) => req.agent_id.as_deref() == Some(id.as_str()),
    }
}

fn tool_matches(rule_tool: &str, req_tool: &str) -> bool {
    let t = rule_tool.trim();
    t == "*" || t.is_empty() || t.eq_ignore_ascii_case(req_tool.trim())
}

fn has_glob_meta(pattern: &str) -> bool {
    pattern.chars().any(|c| matches!(c, '*' | '?' | '[' | '{'))
}

/// Pattern semantics (documented):
/// - `*` / empty: matches everything
/// - contains glob meta: glob match against the target (also tries the
///   file name for path targets so `*.pem` hits `keys/x.pem`)
/// - otherwise: exact match or prefix match (`cargo test` matches
///   `cargo test -p foo`; `src/` matches `src/main.rs`)
pub fn pattern_matches(pattern: &str, target: &str) -> bool {
    let p = pattern.trim();
    let t = target.trim();
    if p == "*" || p.is_empty() {
        return true;
    }
    if has_glob_meta(p) {
        // Try full target plus, for paths, the file name.
        let mut builder = GlobSetBuilder::new();
        if let Ok(g) = Glob::new(p) {
            builder.add(g);
        } else {
            return t == p;
        }
        // `**/*.pem` should also match top-level `x.pem`.
        if let Some(rest) = p.strip_prefix("**/") {
            if let Ok(g) = Glob::new(rest) {
                builder.add(g);
            }
        }
        let Ok(set) = builder.build() else {
            return false;
        };
        if set.is_match(t) {
            return true;
        }
        if let Some(name) = t.rsplit(['/', '\\']).next() {
            if name != t && set.is_match(name) {
                return true;
            }
        }
        return false;
    }
    t == p || t.starts_with(p)
}

/// Default when no rule matches: read-only tools allow, risky tools ask,
/// destructive shell commands deny.
pub fn default_for(tool: &str, target: &str) -> Decision {
    let t = tool.trim().to_ascii_lowercase();
    match t.as_str() {
        // Read-only built-ins: safe to auto-run.
        "read_file" | "list_files" | "read_url" | "web_search" | "recall" | "search_archive"
        | "read_attachment" | "load_skill" | "read_skill_file" | "run_status" | "get_report"
        | "list_projects" => Decision::Allow,
        // Writes, shell, browser actions, agent creation: need approval.
        "shell" => {
            if is_destructive_command(target) {
                Decision::Deny
            } else {
                Decision::Ask
            }
        }
        _ => {
            if t.starts_with("mcp__") || t.contains("mcp") {
                Decision::Ask
            } else if is_read_only_name(&t) {
                Decision::Allow
            } else {
                Decision::Ask
            }
        }
    }
}

fn is_read_only_name(tool: &str) -> bool {
    tool.starts_with("read")
        || tool.starts_with("list")
        || tool.starts_with("search")
        || tool.starts_with("get")
        || tool == "say"
        || tool == "answer"
}

/// Small denylist for obviously destructive shell commands.
fn is_destructive_command(cmd: &str) -> bool {
    let c = cmd.trim().to_ascii_lowercase();
    // Collapse whitespace for robust prefix checks.
    let flat: String = c.split_whitespace().collect::<Vec<_>>().join(" ");
    let prefixes = [
        "rm -rf /",
        "rm -rf ~",
        "rm -rf $home",
        "mkfs",
        "dd of=/dev",
        "dd if=",
        ":(){",
        "fork bomb",
        "shutdown",
        "reboot",
        "halt",
        "poweroff",
        "format c:",
        "del /f /s /q c:",
    ];
    if prefixes.iter().any(|p| flat.starts_with(p)) {
        return true;
    }
    // `rm -rf` with no safe path-ish target is destructive.
    if let Some(rest) = flat.strip_prefix("rm -rf") {
        // `rm -rf <project-relative path>` is a normal write (asks);
        // bare `rm -rf`, `/`, `~`, `*`, `.` deny.
        let rest = rest.trim();
        let dangerous = ["", "/", "/*", "~", "~/*", "*", ".", "./", "../", ".."];
        if dangerous.contains(&rest) {
            return true;
        }
        // Flags between rm and path, e.g. `rm -rf --no-preserve-root /`.
        if rest.contains('/')
            && !rest.contains("./")
            && !rest.starts_with("reports")
            && !rest.starts_with("files")
        {
            // Absolute or home-relative paths deny; project-relative asks.
            if rest.starts_with('/') || rest.starts_with('~') || rest.starts_with('$') {
                return true;
            }
        }
        return false;
    }
    if flat.contains(">/dev/sda") || flat.contains(">/dev/nvme") {
        return true;
    }
    false
}

#[cfg(test)]
mod permissions_tests {
    use super::*;

    fn req(tool: &str, target: &str) -> Request {
        Request::new(tool, target)
    }

    #[test]
    fn permissions_deny_wins_over_allow_and_ask() {
        let engine = Engine::with_rules(vec![
            Rule::global("shell", "*", Effect::Allow),
            Rule::global("shell", "cargo *", Effect::Ask),
            Rule::global("shell", "cargo publish *", Effect::Deny),
        ]);
        assert_eq!(engine.evaluate(&req("shell", "cargo test")), Decision::Ask);
        // Deny wins even though broader allow + ask rules also match.
        assert_eq!(
            engine.evaluate(&req("shell", "cargo publish --dry-run")),
            Decision::Deny
        );
        assert_eq!(engine.evaluate(&req("shell", "ls -la")), Decision::Allow);
    }

    #[test]
    fn permissions_scopes_global_project_agent() {
        let engine = Engine::with_rules(vec![
            Rule::global("write_file", "*", Effect::Ask),
            Rule::project("p1", "write_file", "reports/*", Effect::Allow),
            Rule::agent("a9", "write_file", "*", Effect::Deny),
        ]);
        // Project allow narrows the global ask for that project.
        let in_project = req("write_file", "reports/q.md").with_project("p1");
        assert_eq!(engine.evaluate(&in_project), Decision::Ask);
        // Same request in another project: only the global ask matches.
        let elsewhere = req("write_file", "reports/q.md").with_project("p2");
        assert_eq!(engine.evaluate(&elsewhere), Decision::Ask);
        // Agent deny wins over the project allow.
        let blocked = req("write_file", "reports/q.md")
            .with_project("p1")
            .with_agent("a9");
        assert_eq!(engine.evaluate(&blocked), Decision::Deny);
        // Project rule does not leak into the file it doesn't cover.
        let other_file = req("write_file", "notes/scratch.md").with_project("p1");
        assert_eq!(engine.evaluate(&other_file), Decision::Ask);
        // Rules for other scopes never apply.
        let engine2 = Engine::with_rules(vec![Rule::project("p1", "shell", "*", Effect::Deny)]);
        assert_eq!(
            engine2.evaluate(&req("shell", "ls").with_project("p2")),
            Decision::Ask
        );
    }

    #[test]
    fn permissions_path_glob_patterns() {
        let engine = Engine::with_rules(vec![
            Rule::global("read_file", "*", Effect::Allow),
            Rule::global("read_file", ".env*", Effect::Deny),
            Rule::global("read_file", "**/*.pem", Effect::Deny),
            Rule::global("read_file", "~/.ssh/*", Effect::Deny),
        ]);
        assert_eq!(
            engine.evaluate(&req("read_file", "reports/q.md")),
            Decision::Allow
        );
        assert_eq!(engine.evaluate(&req("read_file", ".env")), Decision::Deny);
        assert_eq!(
            engine.evaluate(&req("read_file", ".env.local")),
            Decision::Deny
        );
        assert_eq!(
            engine.evaluate(&req("read_file", "keys/deploy.pem")),
            Decision::Deny
        );
        assert_eq!(
            engine.evaluate(&req("read_file", "~/.ssh/id_rsa")),
            Decision::Deny
        );
    }

    #[test]
    fn permissions_command_prefix_and_tool_patterns() {
        let engine = Engine::with_rules(vec![
            Rule::global("shell", "cargo *", Effect::Allow),
            Rule::global("shell", "git push *", Effect::Ask),
            Rule::global("shell", "curl *", Effect::Deny),
        ]);
        assert_eq!(
            engine.evaluate(&req("shell", "cargo test -p synapse-tools")),
            Decision::Allow
        );
        // Plain (non-glob) patterns act as command prefixes.
        let engine2 = Engine::with_rules(vec![Rule::global("shell", "cargo test", Effect::Allow)]);
        assert_eq!(
            engine2.evaluate(&req("shell", "cargo test -p foo")),
            Decision::Allow
        );
        assert_eq!(
            engine2.evaluate(&req("shell", "cargo publish")),
            Decision::Ask
        );
        assert_eq!(
            engine.evaluate(&req("shell", "git push origin main")),
            Decision::Ask
        );
        assert_eq!(
            engine.evaluate(&req("shell", "curl https://example.com")),
            Decision::Deny
        );
        // Tool-name matching: `*` covers every tool, MCP names match exactly.
        let mcp = Engine::with_rules(vec![Rule::global(
            "mcp__github__create_pr",
            "*",
            Effect::Ask,
        )]);
        assert_eq!(
            mcp.evaluate(&req("mcp__github__create_pr", "mcp__github__create_pr")),
            Decision::Ask
        );
    }

    #[test]
    fn permissions_risky_actions_get_expected_decision() {
        let engine = Engine::new();
        // Read-only tools auto-run with no rules at all.
        assert_eq!(
            engine.evaluate(&req("read_file", "reports/q.md")),
            Decision::Allow
        );
        assert_eq!(
            engine.evaluate(&req("web_search", "uganda mobile money")),
            Decision::Allow
        );
        // Writes and shell ask by default.
        assert_eq!(
            engine.evaluate(&req("write_file", "reports/q.md")),
            Decision::Ask
        );
        assert_eq!(engine.evaluate(&req("shell", "cargo test")), Decision::Ask);
        assert_eq!(
            engine.evaluate(&req("edit_file", "src/main.rs")),
            Decision::Ask
        );
        // Destructive commands deny without any rule.
        for cmd in [
            "rm -rf /",
            "rm -rf ~",
            "rm -rf / --no-preserve-root",
            "mkfs.ext4 /dev/sda1",
            "dd of=/dev/sda",
        ] {
            assert_eq!(
                engine.evaluate(&req("shell", cmd)),
                Decision::Deny,
                "expected deny for {cmd}"
            );
        }
        // Project-relative `rm -rf` is a normal risky write: asks, not denies.
        assert_eq!(
            engine.evaluate(&req("shell", "rm -rf reports/draft")),
            Decision::Ask
        );
        // An explicit allow can still permit a risky default (defaults are
        // not rules, so deny-wins does not block the override).
        let permissive = Engine::with_rules(vec![Rule::global("shell", "*", Effect::Allow)]);
        assert_eq!(
            permissive.evaluate(&req("shell", "rm -rf /tmp/scratch")),
            Decision::Allow
        );
    }

    #[test]
    fn permissions_effect_and_scope_round_trip() {
        assert_eq!(Effect::parse("ALLOW"), Some(Effect::Allow));
        assert_eq!(Effect::parse("deny"), Some(Effect::Deny));
        assert_eq!(Effect::Deny.as_str(), "deny");
        let rule = Rule::project("p1", "shell", "cargo *", Effect::Ask);
        let json = serde_json::to_string(&rule).unwrap();
        let back: Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, back);
    }
}
