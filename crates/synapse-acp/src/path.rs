//! Login-shell `PATH` resolution.
//!
//! GUI apps do not inherit the user's shell `PATH` (see docs/STACK.md), so
//! detection resolves it once via `$SHELL -l -c 'printf %s "$PATH"'`
//! (the same approach Zed and VS Code use) and merges it with the
//! process `PATH`.

use std::path::PathBuf;
use std::time::Duration;

/// Split a `PATH`-style variable into directories.
pub fn split_path_var(var: &str) -> Vec<PathBuf> {
    var.split(':')
        .filter(|s| !s.trim().is_empty())
        .map(|s| PathBuf::from(s.trim()))
        .collect()
}

/// Directories from the current process `PATH`.
pub fn process_path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|v| split_path_var(&v.to_string_lossy()))
        .unwrap_or_default()
}

/// Query the user's login shell for its `PATH`.
///
/// Runs `$SHELL -l -c 'printf %s "$PATH"'` with a short timeout. Returns an
/// empty vec when the shell cannot be run (tests stay offline; callers fall
/// back to the process `PATH`).
pub async fn login_shell_path() -> Vec<PathBuf> {
    login_shell_path_with(shell_program(), Duration::from_secs(5)).await
}

fn shell_program() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

async fn login_shell_path_with(shell: String, timeout: Duration) -> Vec<PathBuf> {
    let output = async {
        tokio::process::Command::new(&shell)
            .args(["-l", "-c", "printf %s \"$PATH\""])
            .output()
            .await
    };
    let out = match tokio::time::timeout(timeout, output).await {
        Ok(Ok(out)) if out.status.success() => out,
        _ => return Vec::new(),
    };
    split_path_var(&String::from_utf8_lossy(&out.stdout))
}

/// Effective search path: process `PATH` plus login-shell `PATH`, de-duplicated.
pub async fn effective_path_dirs() -> Vec<PathBuf> {
    let mut extra = login_shell_path().await;
    let mut out = process_path_dirs();
    out.append(&mut extra);
    dedup_dirs(out)
}

fn dedup_dirs(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for d in dirs {
        if seen.insert(d.clone()) {
            out.push(d);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_var_skips_empty() {
        let dirs = split_path_var("/usr/bin::/opt/homebrew/bin:");
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/opt/homebrew/bin")
            ]
        );
    }
}
