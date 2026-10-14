//! Safe project file tools: read / list / write / edit inside approved roots.
//!
//! Rules (docs/ARCHITECTURE.md §6, research/rust-stack.md):
//! - every path is resolved against approved roots, canonicalised, and must
//!   stay inside a root; `..` and absolute-path escapes are rejected;
//! - symlink escapes are rejected (any symlink component on the resolved
//!   path, including a symlinked file itself or a symlinked ancestor);
//! - writes provide a unified diff preview (`similar`) before approval.

use std::fs;
use std::path::{Component, Path, PathBuf};

use similar::TextDiff;
use thiserror::Error;

const MAX_READ_BYTES: u64 = 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 2000;

/// Errors from the file tools. Messages are user-facing (what to do next).
#[derive(Debug, Error)]
pub enum FsError {
    #[error("path escapes the project folder: {0}")]
    OutsideRoot(String),
    #[error("symlink escape rejected: {0}")]
    SymlinkEscape(String),
    #[error("file not found: {0}")]
    NotFound(String),
    #[error("expected a file but found a directory: {0}")]
    IsDir(String),
    #[error("expected a directory but found a file: {0}")]
    IsFile(String),
    #[error("file too large ({0} bytes, limit {1} bytes)")]
    TooLarge(u64, u64),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("edit found {0} matches, expected exactly 1")]
    AmbiguousEdit(usize),
    #[error("edit target not found")]
    EditNotFound,
    #[error("io error at {0}: {1}")]
    Io(String, String),
}

fn io_err(path: &str, e: std::io::Error) -> FsError {
    FsError::Io(path.to_string(), e.to_string())
}

/// One directory entry from [`FsScope::list_files`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub len: u64,
}

/// A write/edit waiting for approval, with a diff preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWrite {
    /// Canonical absolute path that would be written.
    pub path: PathBuf,
    /// Previous content (`None` when creating a new file).
    pub old: Option<String>,
    /// New content that would be written.
    pub new: String,
    /// Unified diff preview (`similar`) of old → new.
    pub diff: String,
    /// Whether this write needs user approval (always true for now;
    /// the permission engine may auto-allow read-only-adjacent cases).
    pub needs_approval: bool,
}

impl PendingWrite {
    pub fn preview(&self) -> &str {
        &self.diff
    }
}

/// Approved roots for file access. Cloneable per evaluation context.
#[derive(Debug, Clone)]
pub struct FsScope {
    roots: Vec<PathBuf>,
}

impl FsScope {
    /// Build a scope from approved roots. Existing roots are canonicalised
    /// up front so later containment checks are exact.
    pub fn new(roots: Vec<PathBuf>) -> Result<Self, FsError> {
        if roots.is_empty() {
            return Err(FsError::InvalidPath("no approved roots".to_string()));
        }
        let mut canon = Vec::with_capacity(roots.len());
        for r in roots {
            let abs = to_absolute(&r);
            let c = fs::canonicalize(&abs)
                .map(normalize_canonical)
                .unwrap_or(abs);
            canon.push(c);
        }
        Ok(Self { roots: canon })
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Resolve a user-supplied path to an absolute path inside a root.
    ///
    /// - absolute inputs must already sit inside a root (lexically);
    /// - relative inputs join the first root;
    /// - `..` escapes that leave the root are rejected;
    /// - any symlink component on an existing prefix is rejected;
    /// - the final canonical parent must stay inside the same root.
    pub fn resolve(&self, raw: &str) -> Result<PathBuf, FsError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(FsError::InvalidPath("empty path".to_string()));
        }
        let p = Path::new(raw);
        // Join relative paths onto the first root; keep absolute as-is.
        let joined = if p.is_absolute() {
            normalize_lexical(p)
        } else {
            normalize_lexical(&self.roots[0].join(p))
        };
        // Must land inside some root lexically (catches `..` escapes).
        let root = self
            .roots
            .iter()
            .find(|r| path_inside(&joined, r))
            .ok_or_else(|| FsError::OutsideRoot(raw.to_string()))?;
        // Reject symlink components on the existing prefix.
        reject_symlink_components(&joined, raw)?;
        // Canonical containment: canonicalise the nearest existing
        // ancestor and re-attach the remainder, then require containment.
        let canonical = canonical_with_missing(&joined).map_err(|e| io_err(raw, e))?;
        if !path_inside(&canonical, root) {
            // The canonical root itself may differ in form (macOS /private
            // alias); compare against the canonicalised root too.
            let canon_root = fs::canonicalize(root)
                .map(normalize_canonical)
                .unwrap_or_else(|_| root.clone());
            if !path_inside(&canonical, &canon_root) {
                return Err(FsError::OutsideRoot(raw.to_string()));
            }
        }
        // A symlink as the final component that points outside is an
        // escape even when its parent is inside.
        if let Ok(meta) = fs::symlink_metadata(&joined) {
            if meta.file_type().is_symlink() {
                let target = fs::canonicalize(&joined)
                    .map(normalize_canonical)
                    .unwrap_or(canonical.clone());
                let inside = self.roots.iter().any(|r| path_inside(&target, r));
                if !inside {
                    return Err(FsError::SymlinkEscape(raw.to_string()));
                }
            }
        }
        Ok(canonical)
    }

    /// Read a UTF-8 text file inside the roots.
    pub fn read_file(&self, raw: &str) -> Result<String, FsError> {
        let path = self.resolve(raw)?;
        let meta = fs::symlink_metadata(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FsError::NotFound(raw.to_string())
            } else {
                io_err(raw, e)
            }
        })?;
        if meta.file_type().is_symlink() {
            return Err(FsError::SymlinkEscape(raw.to_string()));
        }
        if meta.is_dir() {
            return Err(FsError::IsDir(raw.to_string()));
        }
        if meta.len() > MAX_READ_BYTES {
            return Err(FsError::TooLarge(meta.len(), MAX_READ_BYTES));
        }
        let bytes = fs::read(&path).map_err(|e| io_err(raw, e))?;
        String::from_utf8(bytes).map_err(|_| FsError::InvalidPath(format!("not UTF-8: {raw}")))
    }

    /// List a directory inside the roots (non-recursive, sorted by name).
    pub fn list_files(&self, raw: &str) -> Result<Vec<FileEntry>, FsError> {
        let path = self.resolve(raw)?;
        let meta = fs::symlink_metadata(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FsError::NotFound(raw.to_string())
            } else {
                io_err(raw, e)
            }
        })?;
        if meta.file_type().is_symlink() {
            return Err(FsError::SymlinkEscape(raw.to_string()));
        }
        if !meta.is_dir() {
            return Err(FsError::IsFile(raw.to_string()));
        }
        let mut out = Vec::new();
        let rd = fs::read_dir(&path).map_err(|e| io_err(raw, e))?;
        for entry in rd {
            let entry = entry.map_err(|e| io_err(raw, e))?;
            let ft = entry.file_type().map_err(|e| io_err(raw, e))?;
            // Never follow symlinked entries: report the link itself and
            // skip descending; a symlinked dir is listed as a file-ish
            // entry so callers cannot walk out of the root.
            let is_dir = !ft.is_symlink() && ft.is_dir();
            let len = if ft.is_symlink() || ft.is_dir() {
                0
            } else {
                entry.metadata().map(|m| m.len()).unwrap_or(0)
            };
            out.push(FileEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
                len,
            });
            if out.len() > MAX_LIST_ENTRIES {
                break;
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Build a pending write with a diff preview. The caller shows
    /// [`PendingWrite::diff`] and asks the permission engine / user before
    /// calling [`FsScope::apply_write`].
    pub fn plan_write(&self, raw: &str, new_content: &str) -> Result<PendingWrite, FsError> {
        let path = self.resolve(raw)?;
        if path.is_dir() {
            return Err(FsError::IsDir(raw.to_string()));
        }
        let old = match fs::read(&path) {
            Ok(b) => Some(
                String::from_utf8(b)
                    .map_err(|_| FsError::InvalidPath(format!("not UTF-8: {raw}")))?,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_err(raw, e)),
        };
        let diff = unified_diff(
            &path_label(&path),
            old.as_deref().unwrap_or(""),
            new_content,
        );
        Ok(PendingWrite {
            path,
            old,
            new: new_content.to_string(),
            diff,
            needs_approval: true,
        })
    }

    /// Apply a previously planned write (after approval).
    pub fn apply_write(&self, pending: &PendingWrite) -> Result<(), FsError> {
        // Re-validate containment at apply time (root set may be shared).
        let label = pending.path.to_string_lossy().into_owned();
        let inside = self.roots.iter().any(|r| {
            path_inside(&pending.path, r)
                || fs::canonicalize(r)
                    .map(normalize_canonical)
                    .is_ok_and(|cr| path_inside(&pending.path, &cr))
        });
        if !inside {
            return Err(FsError::OutsideRoot(label.clone()));
        }
        reject_symlink_components(&pending.path, &label)?;
        if let Some(parent) = pending.path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(&label, e))?;
        }
        // Atomic write: temp file + rename, so readers never see a half
        // write (ARCHITECTURE.md §3).
        let tmp = pending.path.with_extension("tmp.synapse-write");
        fs::write(&tmp, pending.new.as_bytes()).map_err(|e| io_err(&label, e))?;
        fs::rename(&tmp, &pending.path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io_err(&label, e)
        })?;
        Ok(())
    }

    /// Convenience: plan + apply in one call (caller already approved).
    pub fn write_file(&self, raw: &str, new_content: &str) -> Result<PendingWrite, FsError> {
        let pending = self.plan_write(raw, new_content)?;
        self.apply_write(&pending)?;
        Ok(pending)
    }

    /// Build a pending exact-string edit with a diff preview. `old_str`
    /// must occur exactly once in the current file.
    pub fn plan_edit(
        &self,
        raw: &str,
        old_str: &str,
        new_str: &str,
    ) -> Result<PendingWrite, FsError> {
        let path = self.resolve(raw)?;
        let current = self.read_file(raw)?;
        let matches = current.matches(old_str).count();
        if old_str.is_empty() || matches == 0 {
            return Err(FsError::EditNotFound);
        }
        if matches != 1 {
            return Err(FsError::AmbiguousEdit(matches));
        }
        let new_content = current.replacen(old_str, new_str, 1);
        let diff = unified_diff(&path_label(&path), &current, &new_content);
        Ok(PendingWrite {
            path,
            old: Some(current),
            new: new_content,
            diff,
            needs_approval: true,
        })
    }

    /// Convenience: plan + apply an edit in one call (caller approved).
    pub fn edit_file(
        &self,
        raw: &str,
        old_str: &str,
        new_str: &str,
    ) -> Result<PendingWrite, FsError> {
        let pending = self.plan_edit(raw, old_str, new_str)?;
        self.apply_write(&pending)?;
        Ok(pending)
    }
}

/// Unified diff preview of `old` → `new` for display on an approval card.
pub fn unified_diff(label: &str, old: &str, new: &str) -> String {
    let header_old = format!("a/{label}");
    let header_new = format!("b/{label}");
    format!(
        "{}",
        TextDiff::from_lines(old, new)
            .unified_diff()
            .header(&header_old, &header_new)
    )
}

fn path_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn path_inside(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn to_absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        normalize_lexical(p)
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        normalize_lexical(&cwd.join(p))
    }
}

/// Lexically normalise a path (resolve `.` / `..` without touching the fs).
fn normalize_lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}

#[cfg(windows)]
fn normalize_canonical(p: PathBuf) -> PathBuf {
    // Strip the `\\?\` verbatim prefix so `starts_with` works.
    let s = p.to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    PathBuf::from(stripped)
}

#[cfg(not(windows))]
fn normalize_canonical(p: PathBuf) -> PathBuf {
    p
}

/// Canonicalise the nearest existing ancestor and re-attach missing tail.
fn canonical_with_missing(path: &Path) -> std::io::Result<PathBuf> {
    if let Ok(c) = fs::canonicalize(path) {
        return Ok(normalize_canonical(c));
    }
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(c) = fs::canonicalize(&cur) {
            let mut out = normalize_canonical(c);
            for tail in missing.iter().rev() {
                out.push(tail);
            }
            return Ok(normalize_lexical(&out));
        }
        match cur.file_name() {
            Some(name) => {
                missing.push(name.to_os_string());
                cur.pop();
            }
            None => {
                // Reached filesystem root without an existing ancestor.
                let mut out = cur;
                for tail in missing.iter().rev() {
                    out.push(tail);
                }
                return Ok(out);
            }
        }
    }
}

/// Reject any symlink on the existing prefix of `joined`.
fn reject_symlink_components(joined: &Path, raw: &str) -> Result<(), FsError> {
    // Walk each ancestor that exists; a symlink anywhere on the path
    // could redirect the final target outside the roots.
    let mut cur = joined.to_path_buf();
    loop {
        match fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() => {
                // A symlink whose canonical target stays inside a root is
                // still rejected: links can be swapped between check and
                // use (TOCTOU). Deny them all for agent writes/reads.
                return Err(FsError::SymlinkEscape(raw.to_string()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(raw, e)),
        }
        match cur.parent() {
            Some(parent) if parent != cur => cur = parent.to_path_buf(),
            _ => break,
        }
        if cur.as_os_str().is_empty() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod fs_tools_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn scope_at(dir: &Path) -> FsScope {
        FsScope::new(vec![dir.to_path_buf()]).unwrap()
    }

    fn tmp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("synapse-fs-tools-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::canonicalize(&dir).unwrap_or(dir)
    }

    #[test]
    fn fs_tools_read_list_write_edit_round_trip() {
        let root = tmp_root("roundtrip");
        let scope = scope_at(&root);
        fs::create_dir_all(root.join("files")).unwrap();
        fs::write(root.join("files/a.md"), "hello\n").unwrap();

        // Read inside the root.
        assert_eq!(scope.read_file("files/a.md").unwrap(), "hello\n");
        // List inside the root.
        let entries = scope.list_files("files").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a.md");
        assert!(!entries[0].is_dir);

        // Write with a diff preview, then apply.
        let pending = scope.plan_write("files/b.md", "one\ntwo\n").unwrap();
        assert!(pending.needs_approval);
        assert!(pending.old.is_none());
        assert!(pending.diff.contains("+one"));
        scope.apply_write(&pending).unwrap();
        assert_eq!(scope.read_file("files/b.md").unwrap(), "one\ntwo\n");

        // Rewrite shows old → new in the preview.
        let pending2 = scope.plan_write("files/b.md", "one\nTWO\n").unwrap();
        assert!(pending2.diff.contains("-two"));
        assert!(pending2.diff.contains("+TWO"));
        scope.apply_write(&pending2).unwrap();

        // Exact-string edit with preview.
        let edited = scope.edit_file("files/b.md", "TWO", "two!").unwrap();
        assert!(edited.diff.contains("two!"));
        assert_eq!(scope.read_file("files/b.md").unwrap(), "one\ntwo!\n");
    }

    #[test]
    fn fs_tools_rejects_dotdot_and_absolute_escapes() {
        let root = tmp_root("escapes");
        let scope = scope_at(&root);
        fs::create_dir_all(root.join("files")).unwrap();
        let outside = tmp_root("escapes-outside");
        fs::write(outside.join("secret.md"), "s3cret").unwrap();

        assert!(matches!(
            scope.read_file("../escapes-outside/secret.md"),
            Err(FsError::OutsideRoot(_))
        ));
        assert!(matches!(
            scope.read_file("../../etc/hostname"),
            Err(FsError::OutsideRoot(_))
        ));
        let abs = outside.join("secret.md").to_string_lossy().into_owned();
        assert!(matches!(
            scope.read_file(&abs),
            Err(FsError::OutsideRoot(_))
        ));
        assert!(matches!(
            scope.plan_write(&abs, "x"),
            Err(FsError::OutsideRoot(_))
        ));
        assert!(matches!(
            scope.list_files(".."),
            Err(FsError::OutsideRoot(_))
        ));
    }

    #[test]
    fn fs_tools_rejects_symlink_escapes() {
        let root = tmp_root("symlink");
        let scope = scope_at(&root);
        fs::create_dir_all(root.join("files")).unwrap();
        let outside = tmp_root("symlink-outside");
        fs::write(outside.join("secret.md"), "s3cret").unwrap();

        // Symlinked file inside the root pointing outside.
        symlink(outside.join("secret.md"), root.join("files/link.md")).unwrap();
        assert!(matches!(
            scope.read_file("files/link.md"),
            Err(FsError::SymlinkEscape(_))
        ));
        assert!(matches!(
            scope.plan_write("files/link.md", "x"),
            Err(FsError::SymlinkEscape(_))
        ));

        // Symlinked directory inside the root pointing outside.
        symlink(&outside, root.join("files/dirlink")).unwrap();
        assert!(matches!(
            scope.read_file("files/dirlink/secret.md"),
            Err(FsError::SymlinkEscape(_))
        ));
        assert!(matches!(
            scope.list_files("files/dirlink"),
            Err(FsError::SymlinkEscape(_))
        ));
    }

    #[test]
    fn fs_tools_canonical_path_escape_rejected() {
        // `/tmp/a/../outside` style: the lexical path looks inside but the
        // canonical target is outside (via a symlinked parent).
        let root = tmp_root("canonical");
        let scope = scope_at(&root);
        let outside = tmp_root("canonical-outside");
        fs::write(outside.join("secret.md"), "s3cret").unwrap();
        symlink(&outside, root.join("bridge")).unwrap();
        assert!(matches!(
            scope.read_file("bridge/../bridge/secret.md"),
            Err(FsError::SymlinkEscape(_) | FsError::OutsideRoot(_))
        ));
    }

    #[test]
    fn fs_tools_edit_requires_exact_match() {
        let root = tmp_root("edit");
        let scope = scope_at(&root);
        fs::write(root.join("f.md"), "aa bb aa\n").unwrap();
        assert!(matches!(
            scope.plan_edit("f.md", "aa", "zz"),
            Err(FsError::AmbiguousEdit(2))
        ));
        assert!(matches!(
            scope.plan_edit("f.md", "missing", "zz"),
            Err(FsError::EditNotFound)
        ));
        // Failed edits leave the file untouched.
        assert_eq!(scope.read_file("f.md").unwrap(), "aa bb aa\n");
    }

    #[test]
    fn fs_tools_diff_preview_shows_old_and_new() {
        let root = tmp_root("diff");
        let scope = scope_at(&root);
        fs::write(root.join("f.md"), "line1\nline2\n").unwrap();
        let pending = scope.plan_write("f.md", "line1\nLINE2\n").unwrap();
        assert!(pending.needs_approval);
        assert!(pending.diff.contains("-line2"));
        assert!(pending.diff.contains("+LINE2"));
        assert!(pending.preview().contains("f.md"));
    }
}
