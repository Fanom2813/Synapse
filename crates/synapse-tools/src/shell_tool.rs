//! Sandboxed shell tool: off by default, permission-gated, with timeout,
//! output caps and OS sandboxing (nono: Landlock on Linux, Seatbelt on macOS).
//!
//! Rules (docs/ARCHITECTURE.md §6, docs/MILESTONES.md M6.3):
//! - the tool is **off by default** ([`ShellConfig::enabled`]); when off,
//!   every request is denied without running anything;
//! - when on, the permission [`Engine`] decides per command (allow/ask/deny
//!   + globset patterns); destructive commands deny by default;
//! - on Windows there is no OS sandbox in v1, so an `Allow` decision is
//!   escalated to `Ask` (every command needs approval);
//! - on macOS/Linux the child is sandboxed to workspace-write (the work dir
//!   plus tmp read-write, system paths read-only) with no network unless
//!   [`ShellConfig::allow_network`] is set;
//! - every run has a timeout ([`ShellConfig::timeout`]) and an output cap
//!   ([`ShellConfig::max_output_bytes`]).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::permissions::{Decision, Engine, Request};

/// Default timeout for one command.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default cap for captured output (stdout + stderr combined).
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// Sandbox details for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    /// OS sandbox applied (Landlock / Seatbelt).
    Enforced,
    /// No OS sandbox available on this platform (Windows v1).
    Unavailable,
}

/// Configuration for the shell tool. Off by default.
#[derive(Debug, Clone)]
pub struct ShellConfig {
    /// Master switch. `false` (default) denies every request.
    pub enabled: bool,
    /// Directory commands run in; gets read-write sandbox access.
    pub work_dir: PathBuf,
    /// When `false` (default), network access is blocked in the sandbox.
    pub allow_network: bool,
    /// Kill the command after this long.
    pub timeout: Duration,
    /// Cap for captured stdout + stderr combined.
    pub max_output_bytes: usize,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            work_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            allow_network: false,
            timeout: DEFAULT_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

impl ShellConfig {
    pub fn new(work_dir: PathBuf) -> Self {
        Self {
            work_dir,
            ..Self::default()
        }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn allow_network(mut self, allow: bool) -> Self {
        self.allow_network = allow;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn max_output_bytes(mut self, max: usize) -> Self {
        self.max_output_bytes = max;
        self
    }
}

/// Output of a finished command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// True when output hit the cap (tail was dropped).
    pub truncated: bool,
    /// True when the command was killed for exceeding the timeout.
    pub timed_out: bool,
    pub sandbox: SandboxMode,
}

/// Errors from the shell tool. Messages are user-facing (what to do next).
#[derive(Debug, Error)]
pub enum ShellError {
    #[error("shell tool is disabled (enable it in Settings first)")]
    Disabled,
    #[error("shell command denied by permission rules: {0}")]
    Denied(String),
    #[error("shell command needs approval: {0}")]
    NeedsApproval(String),
    #[error("shell command timed out after {0:?}")]
    Timeout(Duration),
    #[error("empty command")]
    EmptyCommand,
    #[error("io error running shell command: {0}")]
    Io(String),
}

fn io_err(e: std::io::Error) -> ShellError {
    ShellError::Io(e.to_string())
}

/// The shell tool: permission check + sandboxed, capped execution.
#[derive(Debug, Clone)]
pub struct ShellTool {
    config: ShellConfig,
}

impl ShellTool {
    pub fn new(config: ShellConfig) -> Self {
        Self { config }
    }

    pub fn with_work_dir(work_dir: PathBuf) -> Self {
        Self::new(ShellConfig::new(work_dir))
    }

    pub fn config(&self) -> &ShellConfig {
        &self.config
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Decide a command against the permission engine on the current platform.
    pub fn decide(&self, engine: &Engine, req: &Request) -> Decision {
        self.decide_on_platform(engine, req, cfg!(windows))
    }

    /// Decide with an explicit platform flag (tests use this to cover the
    /// Windows "always ask" rule on any host).
    pub fn decide_on_platform(&self, engine: &Engine, req: &Request, is_windows: bool) -> Decision {
        if !self.config.enabled {
            return Decision::Deny;
        }
        let decision = engine.evaluate(req);
        // Windows has no OS sandbox in v1, so every command needs approval.
        if is_windows && decision == Decision::Allow {
            return Decision::Ask;
        }
        decision
    }

    /// Check permissions, then run. `Ask` surfaces as [`ShellError::NeedsApproval`]
    /// (the caller shows an approval card and retries after approval);
    /// `Deny` (or disabled) never runs.
    pub fn run_checked(
        &self,
        engine: &Engine,
        req: &Request,
        command: &str,
    ) -> Result<ShellOutput, ShellError> {
        match self.decide(engine, req) {
            Decision::Allow => self.run(command),
            Decision::Ask => Err(ShellError::NeedsApproval(command.to_string())),
            Decision::Deny => {
                if !self.config.enabled {
                    Err(ShellError::Disabled)
                } else {
                    Err(ShellError::Denied(command.to_string()))
                }
            }
        }
    }

    /// Run a command after the caller already obtained approval.
    pub fn run(&self, command: &str) -> Result<ShellOutput, ShellError> {
        if !self.config.enabled {
            return Err(ShellError::Disabled);
        }
        let command = command.trim();
        if command.is_empty() {
            return Err(ShellError::EmptyCommand);
        }
        run_sandboxed(command, &self.config)
    }
}

/// Whether an OS sandbox can be applied on this platform.
pub fn sandbox_available() -> bool {
    #[cfg(unix)]
    {
        nono::Sandbox::is_supported()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Build the workspace-write capability set: work dir + tmp read-write,
/// system paths read-only, network blocked unless allowed.
#[cfg(unix)]
fn build_capabilities(work_dir: &Path, allow_network: bool) -> nono::CapabilitySet {
    use nono::{AccessMode, CapabilitySet};
    let mut caps = CapabilitySet::new();
    let tmp = std::env::temp_dir();
    for dir in [&work_dir.to_path_buf(), &tmp] {
        if dir.is_dir() {
            if let Ok(next) = caps.clone().allow_path(dir, AccessMode::ReadWrite) {
                caps = next;
            }
        }
    }
    // System paths so toolchains/shells resolve and run.
    #[cfg(unix)]
    for dir in [
        "/usr", "/bin", "/lib", "/lib64", "/etc", "/dev", "/proc", "/tmp",
    ] {
        let p = Path::new(dir);
        if p.exists() {
            if let Ok(next) = caps.clone().allow_path(p, AccessMode::Read) {
                caps = next;
            }
        }
    }
    #[cfg(target_os = "macos")]
    for dir in ["/System", "/Library", "/private/tmp"] {
        let p = Path::new(dir);
        if p.exists() {
            if let Ok(next) = caps.clone().allow_path(p, AccessMode::Read) {
                caps = next;
            }
        }
    }
    if !allow_network {
        caps = caps.block_network();
    }
    caps
}

fn shell_command(command: &str, config: &ShellConfig) -> Command {
    let mut cmd;
    #[cfg(windows)]
    {
        cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
    }
    #[cfg(not(windows))]
    {
        cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(command);
        // New process group per command so a timeout kills the whole tree
        // (the shell plus anything it spawned), not just the shell.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.current_dir(&config.work_dir);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // Clear nothing: the command inherits the real environment (like sg);
    // redaction happens on the captured output, not the env.
    cmd
}

/// Apply the nono sandbox inside the forked child (Unix `pre_exec`).
/// Runs after fork, before exec: irreversible, but scoped to the child.
#[cfg(unix)]
fn apply_child_sandbox(config: &ShellConfig) -> std::io::Result<()> {
    if !nono::Sandbox::is_supported() {
        return Ok(());
    }
    let caps = build_capabilities(&config.work_dir, config.allow_network);
    nono::Sandbox::apply_auto(&caps)
        .map(|_| ())
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Read at most `limit + 1` bytes (the extra byte detects truncation).
fn read_capped(mut pipe: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > limit {
                    let room = limit.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..room]);
                    truncated = true;
                    // Drain the rest so the child does not block on a full pipe.
                    while pipe.read(&mut chunk).map(|n| n > 0).unwrap_or(false) {}
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(_) => break,
        }
    }
    (buf, truncated)
}

/// Kill a command's whole process tree (the child leads its own group).
/// No-op where groups are unsupported (Windows kills via `Child::kill`).
fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        // SIGKILL the group; grandchildren holding our pipes die too so
        // the reader threads see EOF instead of blocking to their exit.
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

fn run_sandboxed(command: &str, config: &ShellConfig) -> Result<ShellOutput, ShellError> {
    let mut cmd = shell_command(command, config);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let work_dir = config.work_dir.clone();
        let allow_network = config.allow_network;
        unsafe {
            cmd.pre_exec(move || {
                apply_child_sandbox(&ShellConfig {
                    enabled: true,
                    work_dir: work_dir.clone(),
                    allow_network,
                    timeout: DEFAULT_TIMEOUT,
                    max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
                })
            });
        }
    }
    let mut child = cmd.spawn().map_err(io_err)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let cap = config.max_output_bytes;
    // Reader threads so a chatty child never blocks on a full pipe; the
    // main thread below only waits (bounded by the timeout).
    let out_handle = stdout.map(|p| std::thread::spawn(move || read_capped(p, cap)));
    let err_handle = stderr.map(|p| std::thread::spawn(move || read_capped(p, cap / 2)));
    let start = Instant::now();
    let status = loop {
        match child.try_wait().map_err(io_err)? {
            Some(status) => break Some(status),
            None if start.elapsed() >= config.timeout => {
                kill_tree(child.id());
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let (out_bytes, out_trunc) = out_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    let (err_bytes, err_trunc) = err_handle
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    let timed_out = status.is_none();
    if timed_out {
        return Err(ShellError::Timeout(config.timeout));
    }
    let code = status.and_then(|s| s.code()).unwrap_or(-1);
    let sandbox = if sandbox_available() {
        SandboxMode::Enforced
    } else {
        SandboxMode::Unavailable
    };
    Ok(ShellOutput {
        stdout: String::from_utf8_lossy(&out_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&err_bytes).into_owned(),
        exit_code: code,
        truncated: out_trunc || err_trunc,
        timed_out: false,
        sandbox,
    })
}

#[cfg(test)]
mod shell_tool_tests {
    use super::*;
    use crate::permissions::{Effect, Rule};

    fn enabled_tool() -> ShellTool {
        ShellTool::new(ShellConfig::new(std::env::temp_dir()).enabled(true))
    }

    fn allow_engine() -> Engine {
        Engine::with_rules(vec![Rule::global("shell", "*", Effect::Allow)])
    }

    #[test]
    fn shell_tool_disabled_by_default() {
        let tool = ShellTool::new(ShellConfig::default());
        assert!(!tool.is_enabled());
        let engine = allow_engine();
        let req = Request::new("shell", "echo hi");
        // Even an explicit allow rule cannot run while disabled.
        assert_eq!(tool.decide(&engine, &req), Decision::Deny);
        assert!(matches!(tool.run("echo hi"), Err(ShellError::Disabled)));
        assert!(matches!(
            tool.run_checked(&engine, &req, "echo hi"),
            Err(ShellError::Disabled)
        ));
    }

    #[test]
    fn shell_tool_runs_echo_when_allowed() {
        let tool = enabled_tool();
        let engine = allow_engine();
        let req = Request::new("shell", "echo hello-synapse");
        assert_eq!(tool.decide(&engine, &req), Decision::Allow);
        let out = tool
            .run_checked(&engine, &req, "echo hello-synapse")
            .unwrap();
        assert_eq!(out.stdout.trim(), "hello-synapse");
        assert_eq!(out.exit_code, 0);
        assert!(!out.timed_out);
        assert!(!out.truncated);
        assert_eq!(
            out.sandbox,
            if sandbox_available() {
                SandboxMode::Enforced
            } else {
                SandboxMode::Unavailable
            }
        );
    }

    #[test]
    fn shell_tool_ask_becomes_needs_approval() {
        let tool = enabled_tool();
        // No rules: shell asks by default.
        let engine = Engine::new();
        let req = Request::new("shell", "echo hi");
        assert_eq!(tool.decide(&engine, &req), Decision::Ask);
        assert!(matches!(
            tool.run_checked(&engine, &req, "echo hi"),
            Err(ShellError::NeedsApproval(_))
        ));
    }

    #[test]
    fn shell_tool_destructive_command_denied() {
        let tool = enabled_tool();
        let engine = Engine::new();
        for cmd in ["rm -rf /", "mkfs.ext4 /dev/sda1", "dd of=/dev/sda"] {
            let req = Request::new("shell", cmd);
            assert_eq!(tool.decide(&engine, &req), Decision::Deny, "{cmd}");
            assert!(
                matches!(
                    tool.run_checked(&engine, &req, cmd),
                    Err(ShellError::Denied(_))
                ),
                "{cmd}"
            );
        }
    }

    #[test]
    fn shell_tool_deny_rule_wins() {
        let tool = enabled_tool();
        let engine = Engine::with_rules(vec![
            Rule::global("shell", "*", Effect::Allow),
            Rule::global("shell", "curl *", Effect::Deny),
        ]);
        let req = Request::new("shell", "curl https://example.com");
        assert_eq!(tool.decide(&engine, &req), Decision::Deny);
        assert!(matches!(
            tool.run_checked(&engine, &req, "curl https://example.com"),
            Err(ShellError::Denied(_))
        ));
    }

    #[test]
    fn shell_tool_windows_always_asks() {
        let tool = enabled_tool();
        let engine = allow_engine();
        let req = Request::new("shell", "echo hi");
        // Allow on unix/mac stays Allow...
        assert_eq!(
            tool.decide_on_platform(&engine, &req, false),
            Decision::Allow
        );
        // ...but on Windows every command needs approval (no OS sandbox).
        assert_eq!(tool.decide_on_platform(&engine, &req, true), Decision::Ask);
        // Deny still denies on Windows.
        let deny = Engine::with_rules(vec![Rule::global("shell", "*", Effect::Deny)]);
        assert_eq!(tool.decide_on_platform(&deny, &req, true), Decision::Deny);
    }

    #[test]
    fn shell_tool_timeout_kills_slow_command() {
        let tool = ShellTool::new(
            ShellConfig::new(std::env::temp_dir())
                .enabled(true)
                .timeout(Duration::from_millis(200)),
        );
        #[cfg(windows)]
        let cmd = "powershell -NoProfile -Command Start-Sleep -Seconds 10";
        #[cfg(not(windows))]
        let cmd = "sleep 10";
        let start = Instant::now();
        let err = tool.run(cmd).unwrap_err();
        assert!(matches!(err, ShellError::Timeout(_)), "got {err:?}");
        assert!(start.elapsed() < Duration::from_secs(8));
    }

    #[test]
    fn shell_tool_output_cap_truncates() {
        let tool = ShellTool::new(
            ShellConfig::new(std::env::temp_dir())
                .enabled(true)
                .max_output_bytes(1024),
        );
        #[cfg(windows)]
        let cmd = "powershell -NoProfile -Command 1..5000 | ForEach-Object { '0123456789' }";
        #[cfg(not(windows))]
        let cmd = "seq 1 5000";
        let out = tool.run(cmd).unwrap();
        assert!(out.truncated);
        assert!(out.stdout.len() <= 1024);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn shell_tool_exit_code_and_stderr() {
        let tool = enabled_tool();
        #[cfg(windows)]
        let out = tool.run("cmd /c exit 3").unwrap();
        #[cfg(not(windows))]
        let out = tool.run("exit 3").unwrap();
        assert_eq!(out.exit_code, 3);
        let out = tool.run("echo out-msg; echo err-msg 1>&2").unwrap();
        assert!(out.stdout.contains("out-msg"));
        assert!(out.stderr.contains("err-msg"));
    }

    #[test]
    fn shell_tool_rejects_empty_command() {
        let tool = enabled_tool();
        assert!(matches!(tool.run("   "), Err(ShellError::EmptyCommand)));
    }

    #[test]
    fn shell_tool_workspace_write_capabilities() {
        let dir = std::env::temp_dir();
        let caps = build_capabilities(&dir, false);
        assert!(caps.is_network_blocked());
        let open = build_capabilities(&dir, true);
        assert!(!open.is_network_blocked());
    }
}
