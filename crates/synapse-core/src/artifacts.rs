//! Synthesis artifacts: versioned reports with citations, dissent,
//! open questions and changelogs (see docs/DEEP-RESEARCH.md §5–6,
//! docs/ARCHITECTURE.md §3, docs/TEAM-PROTOCOL.md Revise row).
//!
//! [`ArtifactStore`] is the in-memory repository used by the app and tests;
//! persistence lives in `synapse-store` (tables `artifacts` /
//! `artifact_versions` / `citations`). File mirrors under the project folder
//! (`reports/<slug>.md`, `.synapse/history/<slug>/v<N>.md`) are written
//! atomically here so tests cover the exact bytes without GPUI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One citation attached to an artifact version: which source, which quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCitation {
    pub source_id: String,
    pub quote: String,
}

/// One recorded disagreement and how it was resolved (or left open).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dissent {
    pub author: String,
    pub statement: String,
    pub resolution: String,
}

/// A numbered source for the report's source list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEntry {
    pub id: String,
    pub title: String,
    pub url: String,
}

/// Unvalidated input for [`render_synthesis_report`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReportDraft {
    pub summary: String,
    pub findings: Vec<(String, String)>,
    pub analysis: String,
    pub dissent: Vec<Dissent>,
    pub open_questions: Vec<String>,
    pub sources: Vec<SourceEntry>,
}

/// Render the canonical synthesis report markdown (DEEP-RESEARCH.md §6):
/// executive summary, findings by question, analysis, disagreements and how
/// they were resolved, open questions, numbered sources.
pub fn render_synthesis_report(title: &str, draft: &ReportDraft) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {title}\n\n"));
    out.push_str("## Executive summary\n\n");
    out.push_str(if draft.summary.trim().is_empty() {
        "_No summary yet._\n"
    } else {
        draft.summary.trim()
    });
    out.push('\n');
    out.push_str("\n## Findings\n\n");
    if draft.findings.is_empty() {
        out.push_str("_No findings yet._\n");
    } else {
        for (q, body) in &draft.findings {
            out.push_str(&format!("### {q}\n\n{}\n\n", body.trim()));
        }
    }
    out.push_str("## Analysis\n\n");
    out.push_str(if draft.analysis.trim().is_empty() {
        "_No analysis yet._\n"
    } else {
        draft.analysis.trim()
    });
    out.push('\n');
    out.push_str("\n## Disagreements\n\n");
    if draft.dissent.is_empty() {
        out.push_str("None recorded.\n");
    } else {
        for d in &draft.dissent {
            out.push_str(&format!(
                "- **{}**: {} — {}\n",
                d.author.trim(),
                d.statement.trim(),
                d.resolution.trim()
            ));
        }
    }
    out.push_str("\n## Open questions\n\n");
    if draft.open_questions.is_empty() {
        out.push_str("None.\n");
    } else {
        for q in &draft.open_questions {
            out.push_str(&format!("- {}\n", q.trim()));
        }
    }
    out.push_str("\n## Sources\n\n");
    if draft.sources.is_empty() {
        out.push_str("_No sources cited._\n");
    } else {
        for (i, s) in draft.sources.iter().enumerate() {
            out.push_str(&format!(
                "{}. {} — {} [{}]\n",
                i + 1,
                s.title.trim(),
                s.url.trim(),
                s.id.trim()
            ));
        }
    }
    out
}

/// One saved version of an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub version: u32,
    pub body_md: String,
    pub changelog: String,
    pub created_by_run: String,
    pub created_at: i64,
    pub citations: Vec<ArtifactCitation>,
}

/// An artifact with its current-version pointer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub kind: String,
    pub slug: String,
    pub current_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArtifactError {
    #[error("title is required: give the report a short title")]
    EmptyTitle,
    #[error("report body is required: the writer produced no text")]
    EmptyBody,
    #[error("changelog is required: say what changed in this version")]
    EmptyChangelog,
    #[error("artifact not found: {0}")]
    NotFound(String),
    #[error("version {1} not found for artifact {0}")]
    VersionNotFound(String, u32),
    #[error("no changes: the edited text matches version {0}")]
    NoChanges(u32),
    #[error("file error: {0}")]
    Io(String),
}

/// In-memory artifact + version CRUD. IDs are `artifact_<n>`.
#[derive(Debug, Default)]
pub struct ArtifactStore {
    artifacts: HashMap<String, Artifact>,
    versions: HashMap<(String, u32), ArtifactVersion>,
    next_id: u64,
}

impl ArtifactStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.artifacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Artifact> {
        self.artifacts.get(id)
    }

    /// Newest-first? No: creation order (stable for the panel list).
    pub fn list_for_project(&self, project_id: &str) -> Vec<&Artifact> {
        let mut out: Vec<&Artifact> = self
            .artifacts
            .values()
            .filter(|a| a.project_id == project_id)
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn get_version(&self, artifact_id: &str, version: u32) -> Option<&ArtifactVersion> {
        self.versions.get(&(artifact_id.to_string(), version))
    }

    pub fn versions_of(&self, artifact_id: &str) -> Vec<&ArtifactVersion> {
        let mut out: Vec<&ArtifactVersion> = self
            .versions
            .iter()
            .filter(|((id, _), _)| id == artifact_id)
            .map(|(_, v)| v)
            .collect();
        out.sort_by_key(|v| v.version);
        out
    }

    pub fn current_body(&self, artifact_id: &str) -> Option<&str> {
        let a = self.artifacts.get(artifact_id)?;
        self.get_version(artifact_id, a.current_version)
            .map(|v| v.body_md.as_str())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &mut self,
        project_id: &str,
        title: &str,
        kind: &str,
        body_md: &str,
        changelog: &str,
        citations: Vec<ArtifactCitation>,
        created_by_run: &str,
        created_at: i64,
    ) -> Result<Artifact, ArtifactError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(ArtifactError::EmptyTitle);
        }
        if body_md.trim().is_empty() {
            return Err(ArtifactError::EmptyBody);
        }
        let changelog = changelog.trim();
        if changelog.is_empty() {
            return Err(ArtifactError::EmptyChangelog);
        }
        self.next_id += 1;
        let id = format!("artifact_{}", self.next_id);
        let slug = slugify(title);
        let version = ArtifactVersion {
            version: 1,
            body_md: body_md.to_string(),
            changelog: changelog.to_string(),
            created_by_run: created_by_run.to_string(),
            created_at,
            citations,
        };
        self.versions.insert((id.clone(), 1), version);
        let artifact = Artifact {
            id: id.clone(),
            project_id: project_id.to_string(),
            title: title.to_string(),
            kind: if kind.trim().is_empty() {
                "report".to_string()
            } else {
                kind.trim().to_string()
            },
            slug,
            current_version: 1,
        };
        self.artifacts.insert(id.clone(), artifact.clone());
        Ok(artifact)
    }

    pub fn new_version(
        &mut self,
        artifact_id: &str,
        body_md: &str,
        changelog: &str,
        citations: Vec<ArtifactCitation>,
        created_by_run: &str,
        created_at: i64,
    ) -> Result<ArtifactVersion, ArtifactError> {
        if body_md.trim().is_empty() {
            return Err(ArtifactError::EmptyBody);
        }
        if changelog.trim().is_empty() {
            return Err(ArtifactError::EmptyChangelog);
        }
        let artifact = self
            .artifacts
            .get_mut(artifact_id)
            .ok_or_else(|| ArtifactError::NotFound(artifact_id.into()))?;
        let next = artifact.current_version + 1;
        let version = ArtifactVersion {
            version: next,
            body_md: body_md.to_string(),
            changelog: changelog.to_string(),
            created_by_run: created_by_run.to_string(),
            created_at,
            citations,
        };
        self.versions
            .insert((artifact_id.to_string(), next), version.clone());
        artifact.current_version = next;
        Ok(version)
    }

    /// Unified line diff between two versions of one artifact (STACK.md:
    /// `similar` crate). Empty when the bodies match.
    pub fn diff(&self, artifact_id: &str, from: u32, to: u32) -> Result<String, ArtifactError> {
        let a = self
            .get_version(artifact_id, from)
            .ok_or_else(|| ArtifactError::VersionNotFound(artifact_id.into(), from))?;
        let b = self
            .get_version(artifact_id, to)
            .ok_or_else(|| ArtifactError::VersionNotFound(artifact_id.into(), to))?;
        Ok(unified_diff(&a.body_md, &b.body_md))
    }
}

/// Line diff in a small unified format (` ` context, `-` removed, `+` added).
pub fn unified_diff(before: &str, after: &str) -> String {
    let diff = similar::TextDiff::from_lines(before, after);
    let mut out = String::new();
    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let sign = match change.tag() {
                similar::ChangeTag::Delete => '-',
                similar::ChangeTag::Insert => '+',
                similar::ChangeTag::Equal => ' ',
            };
            out.push(sign);
            out.push_str(change.value());
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

/// URL/file-safe slug for `reports/<slug>.md`.
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for ch in title.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
        } else if (ch == ' ' || ch == '-' || ch == '_') && !slug.ends_with('-') && !slug.is_empty()
        {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "report".to_string()
    } else {
        slug
    }
}

fn atomic_write(path: &Path, content: &str) -> Result<(), ArtifactError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ArtifactError::Io(e.to_string()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content).map_err(|e| ArtifactError::Io(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| ArtifactError::Io(e.to_string()))?;
    Ok(())
}

/// Frontmatter for the current-version mirror (ARCHITECTURE.md §3).
pub fn report_file_content(
    artifact: &Artifact,
    version: &ArtifactVersion,
    source_ids: &[String],
) -> String {
    format!(
        "---\ntitle: {}\nversion: {}\nrun: {}\ndate: {}\nsources: {}\n---\n\n{}",
        artifact.title,
        version.version,
        version.created_by_run,
        version.created_at,
        source_ids.join(", "),
        version.body_md
    )
}

/// What [`mirror_artifact`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorResult {
    pub report_path: PathBuf,
    pub history_path: PathBuf,
}

/// Mirror the current version atomically into the project folder:
/// `reports/<slug>.md` (with frontmatter) plus
/// `.synapse/history/<slug>/v<N>.md` (changelog header + body).
/// Temp file + rename, never a partial report.
pub fn mirror_artifact(
    folder: &Path,
    store: &ArtifactStore,
    artifact_id: &str,
) -> Result<MirrorResult, ArtifactError> {
    let artifact = store
        .get(artifact_id)
        .ok_or_else(|| ArtifactError::NotFound(artifact_id.into()))?;
    let version = store
        .get_version(artifact_id, artifact.current_version)
        .ok_or_else(|| {
            ArtifactError::VersionNotFound(artifact_id.into(), artifact.current_version)
        })?;
    let source_ids: Vec<String> = version
        .citations
        .iter()
        .map(|c| c.source_id.clone())
        .collect();
    let report_path = folder.join("reports").join(format!("{}.md", artifact.slug));
    atomic_write(
        &report_path,
        &report_file_content(artifact, version, &source_ids),
    )?;
    let history_path = folder
        .join(".synapse")
        .join("history")
        .join(artifact.slug.clone())
        .join(format!("v{}.md", version.version));
    atomic_write(
        &history_path,
        &format!(
            "# v{} — {}\n\n{}",
            version.version, version.changelog, version.body_md
        ),
    )?;
    // Shared numbered source list (ARCHITECTURE.md §3).
    let sources_path = folder.join("reports").join("sources.md");
    let mut sources_md = String::from("# Sources\n\n");
    if source_ids.is_empty() {
        sources_md.push_str("_No sources cited._\n");
    } else {
        for (i, id) in source_ids.iter().enumerate() {
            sources_md.push_str(&format!("{}. {}\n", i + 1, id));
        }
    }
    atomic_write(&sources_path, &sources_md)?;
    Ok(MirrorResult {
        report_path,
        history_path,
    })
}

/// Strip the `---` frontmatter mirror files carry; returns the user-editable body.
pub fn strip_frontmatter(file_content: &str) -> &str {
    match file_content.strip_prefix("---") {
        None => file_content,
        Some(trimmed) => match trimmed.find("\n---") {
            None => file_content,
            Some(end) => {
                let rest = &trimmed[end + 4..];
                rest.strip_prefix('\n').unwrap_or(rest)
            }
        },
    }
}

/// True when the mirrored report file differs from the current version body
/// (after stripping frontmatter and trailing whitespace): the panel should
/// offer to import the edit as a new version.
pub fn needs_import(current_body: &str, file_content: &str) -> bool {
    strip_frontmatter(file_content).trim() != current_body.trim()
}

/// Read the mirrored `reports/<slug>.md` body for an artifact, if it exists.
pub fn read_mirrored_body(folder: &Path, artifact: &Artifact) -> Option<String> {
    let path = folder.join("reports").join(format!("{}.md", artifact.slug));
    std::fs::read_to_string(&path)
        .ok()
        .map(|c| strip_frontmatter(&c).to_string())
}

/// Import the user's edited mirror file as a new version. Errors with
/// [`ArtifactError::NoChanges`] when nothing changed.
pub fn import_user_edit(
    store: &mut ArtifactStore,
    artifact_id: &str,
    edited_body: &str,
    created_by_run: &str,
    created_at: i64,
) -> Result<ArtifactVersion, ArtifactError> {
    let current = store
        .current_body(artifact_id)
        .ok_or_else(|| ArtifactError::NotFound(artifact_id.into()))?
        .to_string();
    let artifact = store.get(artifact_id).expect("checked").clone();
    let _ = artifact;
    if edited_body.trim() == current.trim() {
        let v = store
            .get(artifact_id)
            .map(|a| a.current_version)
            .unwrap_or(1);
        return Err(ArtifactError::NoChanges(v));
    }
    // Carry citations forward; the user edit keeps the same sources.
    let citations = store
        .get_version(
            artifact_id,
            store
                .get(artifact_id)
                .map(|a| a.current_version)
                .unwrap_or(1),
        )
        .map(|v| v.citations.clone())
        .unwrap_or_default();
    store.new_version(
        artifact_id,
        edited_body,
        "Imported user edit from the project folder.",
        citations,
        created_by_run,
        created_at,
    )
}

#[cfg(test)]
mod artifacts_tests {
    use super::*;

    fn draft() -> ReportDraft {
        ReportDraft {
            summary: "Markets look strong.".into(),
            findings: vec![("Demand?".into(), "Demand rose 12%. [s1]".into())],
            analysis: "Growth is broad-based.".into(),
            dissent: vec![Dissent {
                author: "ivan".into(),
                statement: "Sample is too small.".into(),
                resolution: "Noted; kept as caveat.".into(),
            }],
            open_questions: vec!["Will growth hold in Q3?".into()],
            sources: vec![SourceEntry {
                id: "s1".into(),
                title: "Market data".into(),
                url: "https://example.com/m".into(),
            }],
        }
    }

    fn store_with_report() -> (ArtifactStore, Artifact) {
        let mut store = ArtifactStore::new();
        let body = render_synthesis_report("Market study", &draft());
        let a = store
            .create(
                "project_1",
                "Market study",
                "report",
                &body,
                "Initial synthesis.",
                vec![ArtifactCitation {
                    source_id: "s1".into(),
                    quote: "Demand rose 12%.".into(),
                }],
                "run-1",
                1700000000,
            )
            .expect("create");
        (store, a)
    }

    #[test]
    fn artifacts_render_synthesis_report_has_citations_dissent_and_open_questions() {
        let body = render_synthesis_report("Market study", &draft());
        for section in [
            "## Executive summary",
            "## Findings",
            "## Analysis",
            "## Disagreements",
            "## Open questions",
            "## Sources",
        ] {
            assert!(body.contains(section), "missing {section}:\n{body}");
        }
        assert!(body.contains("ivan"), "dissent author missing");
        assert!(body.contains("Q3"), "open question missing");
        assert!(body.contains("https://example.com/m"), "source missing");
    }

    #[test]
    fn artifacts_create_versions_and_changelogs() {
        let (mut store, a) = store_with_report();
        assert_eq!(a.current_version, 1);
        assert_eq!(store.versions_of(&a.id).len(), 1);
        let v2 = store
            .new_version(
                &a.id,
                "# Market study\n\nRevised.\n",
                "Addressed the sample-size challenge.",
                Vec::new(),
                "run-1",
                1700000001,
            )
            .expect("v2");
        assert_eq!(v2.version, 2);
        assert_eq!(v2.changelog, "Addressed the sample-size challenge.");
        assert_eq!(store.get(&a.id).unwrap().current_version, 2);
        assert_eq!(
            store.current_body(&a.id).unwrap(),
            "# Market study\n\nRevised.\n"
        );
        // Validation explains itself.
        assert!(matches!(
            store.new_version(&a.id, "  ", "x", Vec::new(), "r", 0),
            Err(ArtifactError::EmptyBody)
        ));
        assert!(matches!(
            store.new_version(&a.id, "body", "  ", Vec::new(), "r", 0),
            Err(ArtifactError::EmptyChangelog)
        ));
        assert!(matches!(
            store.new_version("artifact_999", "body", "log", Vec::new(), "r", 0),
            Err(ArtifactError::NotFound(_))
        ));
    }

    #[test]
    fn artifacts_diff_between_versions_shows_changes() {
        let (mut store, a) = store_with_report();
        let v1 = store.current_body(&a.id).unwrap().to_string();
        store
            .new_version(
                &a.id,
                &format!("{v1}\n\nNew paragraph.\n"),
                "Added a paragraph.",
                Vec::new(),
                "run-1",
                2,
            )
            .unwrap();
        let diff = store.diff(&a.id, 1, 2).expect("diff");
        assert!(diff.contains("+New paragraph."), "got:\n{diff}");
        let same = store.diff(&a.id, 2, 2).expect("same");
        assert!(
            same.lines().all(|l| l.starts_with(' ')),
            "identical versions must diff to context only, got:\n{same}"
        );
        assert!(matches!(
            store.diff(&a.id, 1, 9),
            Err(ArtifactError::VersionNotFound(_, 9))
        ));
    }

    #[test]
    fn artifacts_mirror_reports_atomically_into_project_folder() {
        let (store, a) = store_with_report();
        let dir = std::env::temp_dir().join(format!("synapse-artifacts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let res = mirror_artifact(&dir, &store, &a.id).expect("mirror");
        assert_eq!(res.report_path, dir.join("reports").join("market-study.md"));
        let report = std::fs::read_to_string(&res.report_path).unwrap();
        assert!(
            report.starts_with("---\n"),
            "frontmatter missing:\n{report}"
        );
        assert!(report.contains("version: 1"), "got:\n{report}");
        assert!(report.contains("Markets look strong."), "got:\n{report}");
        // No temp files left behind: the write was atomic.
        assert!(!res.report_path.with_extension("tmp").exists());
        assert!(res.history_path.exists());
        let history = std::fs::read_to_string(&res.history_path).unwrap();
        assert!(history.contains("Initial synthesis."), "got:\n{history}");
        assert!(dir.join("reports").join("sources.md").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifacts_import_user_edits_as_new_versions() {
        let (mut store, a) = store_with_report();
        let dir =
            std::env::temp_dir().join(format!("synapse-artifacts-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        mirror_artifact(&dir, &store, &a.id).expect("mirror");
        // Untouched file: no import offered.
        let artifact = store.get(&a.id).unwrap().clone();
        let mirrored = read_mirrored_body(&dir, &artifact).expect("read back");
        assert!(!needs_import(store.current_body(&a.id).unwrap(), &mirrored));
        assert!(matches!(
            import_user_edit(&mut store, &a.id, &mirrored, "run-1", 3),
            Err(ArtifactError::NoChanges(1))
        ));
        // User edits the report file: import creates v2 with a changelog.
        let edited = format!("{mirrored}\n\nUser note: double-check Q3.\n");
        assert!(needs_import(store.current_body(&a.id).unwrap(), &edited));
        let v2 = import_user_edit(&mut store, &a.id, &edited, "run-1", 3).expect("import");
        assert_eq!(v2.version, 2);
        assert!(v2.changelog.contains("user edit"), "got: {}", v2.changelog);
        assert!(v2.body_md.contains("double-check Q3"));
        // Citations carried forward.
        assert_eq!(v2.citations.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifacts_list_filters_by_project_and_serializes() {
        let (mut store, a) = store_with_report();
        let body = render_synthesis_report("Other", &ReportDraft::default());
        store
            .create(
                "project_2",
                "Other",
                "report",
                &body,
                "First.",
                Vec::new(),
                "run-2",
                0,
            )
            .unwrap();
        assert_eq!(store.list_for_project("project_1").len(), 1);
        assert_eq!(store.list_for_project("project_2").len(), 1);
        let json = serde_json::to_string(&a).unwrap();
        let back: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }
}
