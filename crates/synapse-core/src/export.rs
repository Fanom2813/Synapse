//! Export reports and transcripts (see docs/PRD.md §5.9).
//!
//! Artifacts and full transcripts export to Markdown or HTML; HTML exports
//! carry a `@media print` stylesheet so PDF goes through the OS print
//! dialog. Every export runs through [`Redactor`] so no secrets leak.
//! File writes are atomic (temp + rename, as in [`crate::artifacts`]);
//! opening the project folder shells out to the OS opener.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::artifacts::{strip_frontmatter, ArtifactStore};
use crate::bus::{Envelope, Participant};
use crate::redact::Redactor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Html,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Html => "html",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "html" | "htm" => Some(Self::Html),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExportError {
    #[error("artifact not found: {0}")]
    ArtifactNotFound(String),
    #[error("nothing to export: the transcript has no messages")]
    EmptyTranscript,
    #[error("file error: {0}")]
    Io(String),
}

/// Escape `&<>"'` for HTML text content.
pub fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Minimal markdown → HTML for exports: headings, paragraphs, unordered
/// lists, fenced code blocks and inline code. Input is escaped first so
/// message bodies can never inject markup.
pub fn markdown_to_html_body(md: &str) -> String {
    let mut html = String::new();
    let mut in_code = false;
    let mut in_list = false;
    for raw_line in md.lines() {
        let line = raw_line.trim_end();
        if line.trim_start().starts_with("```") {
            if in_code {
                html.push_str("</code></pre>\n");
            } else {
                if in_list {
                    html.push_str("</ul>\n");
                    in_list = false;
                }
                html.push_str("<pre><code>");
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            html.push_str(&html_escape(line));
            html.push('\n');
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            continue;
        }
        if let Some(rest) = heading_text(trimmed) {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!(
                "<h{level}>{text}</h{level}>\n",
                level = rest.0,
                text = inline_code_html(&html_escape(rest.1))
            ));
            continue;
        }
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            if !in_list {
                html.push_str("<ul>\n");
                in_list = true;
            }
            let item = trimmed[2..].trim();
            html.push_str(&format!(
                "<li>{}</li>\n",
                inline_code_html(&html_escape(item))
            ));
            continue;
        }
        if in_list {
            html.push_str("</ul>\n");
            in_list = false;
        }
        html.push_str(&format!(
            "<p>{}</p>\n",
            inline_code_html(&html_escape(trimmed))
        ));
    }
    if in_code {
        html.push_str("</code></pre>\n");
    }
    if in_list {
        html.push_str("</ul>\n");
    }
    html
}

fn heading_text(line: &str) -> Option<(u8, &str)> {
    let level = line.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
        Some((level as u8, line[level + 1..].trim()))
    } else {
        None
    }
}

/// Replace `` `code` `` spans inside already-escaped text with `<code>`.
fn inline_code_html(escaped: &str) -> String {
    let parts: Vec<&str> = escaped.split('`').collect();
    // Even count = unbalanced backtick: return the escaped text unchanged.
    if parts.len().is_multiple_of(2) {
        return escaped.to_string();
    }
    let mut out = String::with_capacity(escaped.len());
    for (i, part) in parts.iter().enumerate() {
        if i % 2 == 1 {
            out.push_str("<code>");
            out.push_str(part);
            out.push_str("</code>");
        } else {
            out.push_str(part);
        }
    }
    out
}

/// Full HTML document with a print stylesheet: the OS print dialog
/// ("print to PDF") uses the `@media print` rules.
pub fn printable_html(title: &str, body_html: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n<style>\nbody {{ font-family: system-ui, sans-serif; max-width: 44rem; margin: 2rem auto; padding: 0 1rem; line-height: 1.5; }}\npre {{ background: #f4f4f4; padding: 0.75rem; overflow-x: auto; }}\n@media print {{ body {{ max-width: none; margin: 0; }} }}\n</style>\n</head>\n<body>\n{body_html}</body>\n</html>\n",
        title = html_escape(title),
        body_html = body_html
    )
}

fn sender_label(from: &Participant) -> String {
    match from {
        Participant::User => "You".to_string(),
        Participant::Agent(id) => id.clone(),
        Participant::System => "System".to_string(),
    }
}

/// Export one artifact version body. Redacted; HTML is a full printable doc.
pub fn export_artifact(
    store: &ArtifactStore,
    artifact_id: &str,
    format: ExportFormat,
    redactor: &Redactor,
) -> Result<String, ExportError> {
    let body = store
        .current_body(artifact_id)
        .ok_or_else(|| ExportError::ArtifactNotFound(artifact_id.into()))?;
    let title = store
        .get(artifact_id)
        .map(|a| a.title.clone())
        .unwrap_or_else(|| "Report".to_string());
    let clean = strip_frontmatter(body);
    let redacted = redactor.redact(clean).text;
    match format {
        ExportFormat::Markdown => Ok(format!("# {title}\n\n{redacted}\n")),
        ExportFormat::Html => Ok(printable_html(&title, &markdown_to_html_body(&redacted))),
    }
}

/// Export a transcript (ordered envelopes) to Markdown or printable HTML.
/// Redacted with `redactor` before rendering.
pub fn export_transcript(
    title: &str,
    messages: &[Envelope],
    format: ExportFormat,
    redactor: &Redactor,
) -> Result<String, ExportError> {
    if messages.is_empty() {
        return Err(ExportError::EmptyTranscript);
    }
    match format {
        ExportFormat::Markdown => {
            let mut out = format!("# {title}\n\n");
            for m in messages {
                let who = sender_label(&m.from);
                let body = redactor.redact(&m.body).text;
                out.push_str(&format!("## {who}\n\n{body}\n\n"));
            }
            Ok(out)
        }
        ExportFormat::Html => {
            let mut body = String::new();
            body.push_str(&format!("<h1>{}</h1>\n", html_escape(title)));
            for m in messages {
                let who = html_escape(&sender_label(&m.from));
                let text = redactor.redact(&m.body).text;
                body.push_str(&format!(
                    "<section><h2>{who}</h2>\n{}</section>\n",
                    markdown_to_html_body(&text)
                ));
            }
            Ok(printable_html(title, &body))
        }
    }
}

/// File-safe base name for an export file (no extension).
pub fn export_filename(title: &str) -> String {
    let slug = crate::artifacts::slugify(title);
    if slug.is_empty() {
        "export".to_string()
    } else {
        slug
    }
}

/// Atomically write an export string into `folder/exports/<filename>`.
/// Returns the full path written.
pub fn write_export_file(
    folder: &Path,
    filename: &str,
    content: &str,
) -> Result<PathBuf, ExportError> {
    let dir = folder.join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| ExportError::Io(e.to_string()))?;
    let safe: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = if safe.trim().is_empty() {
        "export.md".to_string()
    } else {
        safe
    };
    let path = dir.join(name);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content).map_err(|e| ExportError::Io(e.to_string()))?;
    std::fs::rename(&tmp, &path).map_err(|e| ExportError::Io(e.to_string()))?;
    Ok(path)
}

/// Canonical project folder path for "open folder": resolved, no IO needed.
pub fn project_folder_path(folder: &Path) -> PathBuf {
    folder.to_path_buf()
}

/// Open the project folder in the OS file manager (Finder / Explorer /
/// xdg-open). Returns an error with next steps when the opener fails.
pub fn open_project_folder(folder: &Path) -> Result<(), ExportError> {
    let path = folder.to_string_lossy().to_string();
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&path).status()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(&path).status()
    } else {
        std::process::Command::new("xdg-open").arg(&path).status()
    };
    match result {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(ExportError::Io(format!(
            "could not open {path}: opener exited with {status}; open it manually"
        ))),
        Err(e) => Err(ExportError::Io(format!(
            "could not open {path}: {e}; open it manually"
        ))),
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use crate::artifacts::{render_synthesis_report, ArtifactStore, ReportDraft};
    use crate::bus::{Envelope, Participant, Recipients};
    use crate::redact::{fixtures, Redactor};

    fn redactor_with_fixtures() -> Redactor {
        let mut r = Redactor::new();
        r.add_secrets(fixtures::all());
        r
    }

    fn store_with_report() -> (ArtifactStore, String) {
        let mut store = ArtifactStore::new();
        let body = render_synthesis_report("Market study", &ReportDraft::default());
        let a = store
            .create(
                "project_1",
                "Market study",
                "report",
                &body,
                "Initial synthesis.",
                Vec::new(),
                "run-1",
                1700000000,
            )
            .unwrap();
        (store, a.id.clone())
    }

    fn transcript() -> Vec<Envelope> {
        vec![
            Envelope::say(
                "m1",
                "conv",
                Participant::User,
                Recipients::Room,
                "What did you find?",
            ),
            Envelope::say(
                "m2",
                "conv",
                Participant::Agent("writer".into()),
                Recipients::Room,
                "Demand rose. See `growth`.",
            ),
        ]
    }

    #[test]
    fn export_artifact_markdown_contains_title_and_body() {
        let (store, id) = store_with_report();
        let out = export_artifact(&store, &id, ExportFormat::Markdown, &Redactor::new()).unwrap();
        assert!(out.contains("# Market study"), "got:\n{out}");
        assert!(out.ends_with('\n'));
        assert!(matches!(
            export_artifact(
                &store,
                "artifact_999",
                ExportFormat::Markdown,
                &Redactor::new()
            ),
            Err(ExportError::ArtifactNotFound(_))
        ));
    }

    #[test]
    fn export_artifact_html_is_printable_and_escaped() {
        let (store, id) = store_with_report();
        let out = export_artifact(&store, &id, ExportFormat::Html, &Redactor::new()).unwrap();
        assert!(out.contains("@media print"), "got:\n{out}");
        assert!(out.contains("<title>Market study</title>"));
        // Bodies can never inject markup.
        let mut store2 = ArtifactStore::new();
        let a = store2
            .create(
                "p",
                "XSS",
                "report",
                "<script>alert(1)</script>",
                "First.",
                Vec::new(),
                "run",
                0,
            )
            .unwrap();
        let evil = export_artifact(&store2, &a.id, ExportFormat::Html, &Redactor::new()).unwrap();
        assert!(!evil.contains("<script>"), "got:\n{evil}");
        assert!(evil.contains("&lt;script&gt;"));
    }

    #[test]
    fn export_transcript_markdown_and_html_round_trip() {
        let messages = transcript();
        let md = export_transcript(
            "Team talk",
            &messages,
            ExportFormat::Markdown,
            &Redactor::new(),
        )
        .unwrap();
        assert!(md.contains("# Team talk"), "got:\n{md}");
        assert!(md.contains("## writer"), "got:\n{md}");
        let html = export_transcript("Team talk", &messages, ExportFormat::Html, &Redactor::new())
            .unwrap();
        assert!(html.contains("@media print"), "got:\n{html}");
        assert!(html.contains("<code>growth</code>"), "got:\n{html}");
        assert!(matches!(
            export_transcript("Empty", &[], ExportFormat::Markdown, &Redactor::new()),
            Err(ExportError::EmptyTranscript)
        ));
    }

    #[test]
    fn export_outputs_contain_no_secrets() {
        let r = redactor_with_fixtures();
        let secret_body = format!("Key is {} do not leak", fixtures::OPENAI.1);
        let mut store = ArtifactStore::new();
        let a = store
            .create(
                "p",
                "Secret",
                "report",
                &secret_body,
                "First.",
                Vec::new(),
                "run",
                0,
            )
            .unwrap();
        for format in [ExportFormat::Markdown, ExportFormat::Html] {
            let out = export_artifact(&store, &a.id, format, &r).unwrap();
            assert!(!out.contains(fixtures::OPENAI.1), "leaked in {format:?}");
            assert!(out.contains("[redacted:"), "missing marker in {format:?}");
        }
        let secret_msg = vec![Envelope::say(
            "m9",
            "conv",
            Participant::Agent("researcher".into()),
            Recipients::Room,
            &format!("token {} here", fixtures::GITHUB.1),
        )];
        for format in [ExportFormat::Markdown, ExportFormat::Html] {
            let out = export_transcript("Talk", &secret_msg, format, &r).unwrap();
            assert!(!out.contains(fixtures::GITHUB.1), "leaked in {format:?}");
        }
    }

    #[test]
    fn export_files_write_atomically_under_exports_dir() {
        let dir = std::env::temp_dir().join(format!("synapse-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = write_export_file(&dir, "report.md", "# Hello\n").unwrap();
        assert_eq!(path, dir.join("exports").join("report.md"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Hello\n");
        assert!(!path.with_extension("tmp").exists());
        // Unsafe names are sanitized, never escape the exports dir.
        let path2 = write_export_file(&dir, "../evil/../../x.md", "x").unwrap();
        assert!(path2.starts_with(dir.join("exports")));
        assert_eq!(export_filename("Market Study!"), "market-study");
        assert_eq!(project_folder_path(&dir), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_format_extensions_parse() {
        assert_eq!(ExportFormat::Markdown.extension(), "md");
        assert_eq!(ExportFormat::Html.extension(), "html");
        assert_eq!(
            ExportFormat::from_extension("md"),
            Some(ExportFormat::Markdown)
        );
        assert_eq!(
            ExportFormat::from_extension("HTML"),
            Some(ExportFormat::Html)
        );
        assert_eq!(ExportFormat::from_extension("pdf"), None);
    }
}
