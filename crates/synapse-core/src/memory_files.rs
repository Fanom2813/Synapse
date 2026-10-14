//! Scoped memory markdown mirrors (docs/MEMORY.md §2, ARCHITECTURE.md §3).
//!
//! Each scope is mirrored as a readable file the user can edit:
//! `<project folder>/.synapse/MEMORY.md`, `<data dir>/memory/USER.md`,
//! `<data dir>/memory/agents/<name>.md`. The DB is the source of truth;
//! these helpers render active memories to markdown (atomic write) and parse
//! user edits back into ADD / UPDATE / DELETE changes for the store to apply.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryFileError {
    #[error("file error: {0}")]
    Io(String),
}

/// One memory as seen by the mirror layer. Mirrors [`synapse_store::Memory`]
/// without depending on the store crate (`synapse-core` owns no I/O specifics
/// beyond these pure helpers + atomic file writes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileMemory {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub pinned: bool,
}

impl FileMemory {
    pub fn new(id: &str, kind: &str, text: &str, pinned: bool) -> Self {
        Self {
            id: id.to_string(),
            kind: kind.to_string(),
            text: text.to_string(),
            pinned,
        }
    }
}

/// One line parsed back from an edited mirror file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntry {
    pub id: Option<String>,
    pub kind: String,
    pub text: String,
    pub pinned: bool,
}

/// Diff of an edited mirror file against the active rows (MEMORY.md §2:
/// diff → ADD / UPDATE / DELETE).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryChange {
    Add {
        kind: String,
        text: String,
        pinned: bool,
    },
    Update {
        id: String,
        kind: String,
        text: String,
        pinned: bool,
    },
    Delete {
        id: String,
    },
}

/// Canonical mirror path for a project folder.
pub fn project_memory_path(folder: &Path) -> PathBuf {
    folder.join(".synapse").join("MEMORY.md")
}

/// Canonical mirror path for the user profile.
pub fn user_memory_path(data_dir: &Path) -> PathBuf {
    data_dir.join("memory").join("USER.md")
}

/// Canonical mirror path for one agent's notes.
pub fn agent_memory_path(data_dir: &Path, agent_name: &str) -> PathBuf {
    data_dir
        .join("memory")
        .join("agents")
        .join(format!("{agent_name}.md"))
}

/// Canonical mirror path for a run digest.
pub fn digest_memory_path(folder: &Path, run_id: &str) -> PathBuf {
    folder
        .join(".synapse")
        .join("history")
        .join(format!("run-{run_id}.md"))
}

fn escape_single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Render one scope's active memories as user-editable markdown. Every bullet
/// carries an `<!-- id: … -->` trailer so edits round-trip; removing the
/// trailer makes the line a new memory on import. Users may freely reorder
/// lines, edit text, or delete lines.
pub fn render_memory_file(title: &str, memories: &[FileMemory]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {title}\n\n"));
    out.push_str(
        "<!-- Synapse memory mirror: edit bullets, add lines, or delete them, then Synapse imports the diff. -->\n\n",
    );
    if memories.is_empty() {
        out.push_str("_No memories yet._\n");
        return out;
    }
    // Group by kind for readability, stable order.
    let mut kinds: Vec<String> = Vec::new();
    for m in memories {
        let k = if m.kind.trim().is_empty() {
            "fact".to_string()
        } else {
            m.kind.trim().to_string()
        };
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    kinds.sort();
    for kind in &kinds {
        let plural = format!("{kind}s");
        out.push_str(&format!("## {plural}\n\n"));
        for m in memories.iter().filter(|m| {
            let k = if m.kind.trim().is_empty() {
                "fact"
            } else {
                m.kind.trim()
            };
            k == *kind
        }) {
            let pin = if m.pinned { "📌 " } else { "" };
            out.push_str(&format!(
                "- {}{} <!-- id: {} kind: {} pinned: {} -->\n",
                pin,
                escape_single_line(&m.text),
                m.id,
                if m.kind.trim().is_empty() {
                    "fact"
                } else {
                    m.kind.trim()
                },
                m.pinned
            ));
        }
        out.push('\n');
    }
    out
}

/// Atomically write (temp file + rename) the mirror for one scope.
pub fn mirror_memory_file(
    path: &Path,
    title: &str,
    memories: &[FileMemory],
) -> Result<PathBuf, MemoryFileError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| MemoryFileError::Io(e.to_string()))?;
    }
    let content = render_memory_file(title, memories);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content).map_err(|e| MemoryFileError::Io(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| MemoryFileError::Io(e.to_string()))?;
    Ok(path.to_path_buf())
}

/// True when the file on disk differs from the rendered mirror: the UI should
/// offer to import the user's edits.
pub fn memory_mirror_needs_import(path: &Path, title: &str, memories: &[FileMemory]) -> bool {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    current != render_memory_file(title, memories)
}

/// Parse an edited mirror file back into entries. Accepts bullets (`-`/`*`)
/// with or without the `<!-- id: … -->` trailer; lines without a trailer are
/// new memories. `📌` prefix or `pinned: true` marks pinning; `kind: <k>`
/// inside the trailer overrides the section kind.
pub fn parse_memory_file(content: &str) -> Vec<ParsedEntry> {
    let mut entries = Vec::new();
    let mut section_kind = "fact".to_string();
    for line in content.lines() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("##") {
            let name = h.trim().trim_end_matches('s').trim().to_lowercase();
            if !name.is_empty() && !name.starts_with('#') {
                section_kind = name.split_whitespace().next().unwrap_or("fact").to_string();
            }
            continue;
        }
        let body = if let Some(b) = t.strip_prefix("- ") {
            b
        } else if let Some(b) = t.strip_prefix("* ") {
            b
        } else {
            continue;
        };
        if body.trim().is_empty() || body.trim().starts_with("<!--") {
            continue;
        }
        // Split trailing `<!-- … -->` metadata.
        let (text_part, meta) = match body.find("<!--") {
            Some(i) => (body[..i].trim(), Some(body[i..].trim())),
            None => (body.trim(), None),
        };
        if text_part.trim().is_empty() || text_part.starts_with('_') && text_part.ends_with('_') {
            continue;
        }
        let mut id: Option<String> = None;
        let mut kind = section_kind.clone();
        let mut pinned = false;
        if let Some(meta) = meta {
            let inner = meta
                .trim_start_matches("<!--")
                .trim_end_matches("-->")
                .trim();
            let toks: Vec<&str> = inner.split_whitespace().collect();
            let mut i = 0;
            while i < toks.len() {
                let tok = toks[i];
                let (k, v) = if let Some((k, v)) = tok.split_once(':') {
                    if v.is_empty() {
                        let val = toks.get(i + 1).copied().unwrap_or("");
                        i += 1;
                        (k, val)
                    } else {
                        (k, v)
                    }
                } else {
                    i += 1;
                    continue;
                };
                match k {
                    "id" => {
                        if !v.trim().is_empty() {
                            id = Some(v.trim().to_string());
                        }
                    }
                    "kind" => {
                        if !v.trim().is_empty() {
                            kind = v.trim().to_string();
                        }
                    }
                    "pinned" => {
                        pinned = v.trim() == "true";
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        let mut text = text_part.trim().to_string();
        if let Some(rest) = text.strip_prefix("📌") {
            pinned = true;
            text = rest.trim().to_string();
        }
        if text.is_empty() {
            continue;
        }
        entries.push(ParsedEntry {
            id,
            kind,
            text,
            pinned,
        });
    }
    entries
}

/// Diff parsed edits against the active rows: matching ids with changed
/// text/kind/pin → UPDATE, new lines → ADD, missing ids → DELETE.
pub fn diff_memory_files(old: &[FileMemory], parsed: &[ParsedEntry]) -> Vec<MemoryChange> {
    let by_id: HashMap<&str, &FileMemory> = old.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut changes = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for p in parsed {
        match &p.id {
            Some(id) if by_id.contains_key(id.as_str()) => {
                seen.insert(id.as_str());
                let prev = by_id[id.as_str()];
                let prev_kind = if prev.kind.trim().is_empty() {
                    "fact"
                } else {
                    prev.kind.trim()
                };
                if prev.text.trim() != p.text.trim()
                    || prev_kind != p.kind.trim()
                    || prev.pinned != p.pinned
                {
                    changes.push(MemoryChange::Update {
                        id: id.clone(),
                        kind: p.kind.clone(),
                        text: p.text.clone(),
                        pinned: p.pinned,
                    });
                }
            }
            _ => {
                if p.id.is_some() {
                    // Unknown id (user pasted another scope's line): treat as new.
                    changes.push(MemoryChange::Add {
                        kind: p.kind.clone(),
                        text: p.text.clone(),
                        pinned: p.pinned,
                    });
                } else {
                    changes.push(MemoryChange::Add {
                        kind: p.kind.clone(),
                        text: p.text.clone(),
                        pinned: p.pinned,
                    });
                }
            }
        }
    }
    for m in old {
        if !seen.contains(m.id.as_str()) {
            // Entries the user deleted produce DELETE; but unknown-id pastes
            // above must not delete everything — only ids absent from parsed.
            let still_there = parsed
                .iter()
                .any(|p| p.id.as_deref() == Some(m.id.as_str()));
            if !still_there {
                changes.push(MemoryChange::Delete { id: m.id.clone() });
            }
        }
    }
    changes
}

#[cfg(test)]
mod memory_files_tests {
    use super::*;

    fn sample() -> Vec<FileMemory> {
        vec![
            FileMemory::new("mem_1", "fact", "Prefers sources from 2025+", false),
            FileMemory::new("mem_2", "decision", "Use Scholar search first", true),
        ]
    }

    #[test]
    fn memory_files_render_and_parse_roundtrip() {
        let md = render_memory_file("Project memory", &sample());
        assert!(md.contains("# Project memory"), "got:\n{md}");
        assert!(md.contains("mem_1"), "id trailer missing:\n{md}");
        assert!(md.contains("📌"), "pinned marker missing:\n{md}");
        let parsed = parse_memory_file(&md);
        assert_eq!(parsed.len(), 2);
        let first = parsed
            .iter()
            .find(|p| p.id.as_deref() == Some("mem_1"))
            .expect("mem_1");
        let second = parsed
            .iter()
            .find(|p| p.id.as_deref() == Some("mem_2"))
            .expect("mem_2");
        assert_eq!(first.text, "Prefers sources from 2025+");
        assert!(second.pinned);
        assert!(diff_memory_files(&sample(), &parsed).is_empty());
    }

    #[test]
    fn memory_files_edits_diff_to_add_update_delete() {
        let old = sample();
        let edited = "- Prefers sources from 2025 and later <!-- id: mem_1 kind: fact pinned: false -->\n- Brand new lesson\n";
        let parsed = parse_memory_file(edited);
        assert_eq!(parsed.len(), 2);
        let changes = diff_memory_files(&old, &parsed);
        assert!(
            changes.iter().any(|c| matches!(
                c, MemoryChange::Update { id, .. } if id == "mem_1"
            )),
            "expected UPDATE mem_1, got: {changes:?}"
        );
        assert!(
            changes
                .iter()
                .any(|c| matches!(c, MemoryChange::Add { .. })),
            "expected ADD, got: {changes:?}"
        );
        assert!(
            changes.iter().any(|c| matches!(
                c, MemoryChange::Delete { id } if id == "mem_2"
            )),
            "expected DELETE mem_2, got: {changes:?}"
        );
    }

    #[test]
    fn memory_files_mirror_atomically_and_detects_edits() {
        let dir = std::env::temp_dir().join(format!("synapse-memory-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = project_memory_path(&dir);
        mirror_memory_file(&path, "Project memory", &sample()).expect("mirror");
        assert!(!path.with_extension("tmp").exists());
        assert!(!memory_mirror_needs_import(
            &path,
            "Project memory",
            &sample()
        ));
        assert!(memory_mirror_needs_import(&path, "Project memory", &[]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_files_scope_paths() {
        let base = Path::new("/tmp/syn");
        assert_eq!(
            project_memory_path(base),
            Path::new("/tmp/syn/.synapse/MEMORY.md")
        );
        assert_eq!(user_memory_path(base), Path::new("/tmp/syn/memory/USER.md"));
        assert_eq!(
            agent_memory_path(base, "ada"),
            Path::new("/tmp/syn/memory/agents/ada.md")
        );
    }
}
