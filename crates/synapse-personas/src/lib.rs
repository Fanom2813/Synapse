//! synapse-personas: persona and skill parsing (see docs/ARCHITECTURE.md).
//!
//! Parses the documented persona markdown/frontmatter formats
//! (agency-agents, wshobson/agents, VoltAgent, 0xfurai, lst97),
//! Fabric `system.md` patterns, and Agent Skills (`SKILL.md`) folders
//! into importable drafts. Every draft preserves source, commit and
//! license metadata.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Where an importable draft came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMeta {
    /// e.g. `msitarzewski/agency-agents`.
    pub repo: String,
    /// Commit sha of the collection (or `bundled` for the offline set).
    pub sha: String,
    /// SPDX id or short license text, e.g. `MIT`.
    pub license: String,
    /// Path within the source repo (or asset path for bundled).
    pub path: String,
}

impl SourceMeta {
    pub fn new(
        repo: impl Into<String>,
        sha: impl Into<String>,
        license: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            sha: sha.into(),
            license: license.into(),
            path: path.into(),
        }
    }

    pub fn bundled(path: impl Into<String>) -> Self {
        Self {
            repo: "synapse".to_string(),
            sha: "bundled".to_string(),
            license: "MIT".to_string(),
            path: path.into(),
        }
    }
}

/// An importable agent draft parsed from a persona markdown file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaDraft {
    pub name: String,
    pub description: String,
    /// Full persona instructions (markdown body).
    pub instructions: String,
    pub role: Option<String>,
    pub category: Option<String>,
    pub color: Option<String>,
    pub emoji: Option<String>,
    pub vibe: Option<String>,
    pub model: Option<String>,
    pub tools: Vec<String>,
    pub source: SourceMeta,
}

/// An importable skill draft parsed from a `SKILL.md` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDraft {
    /// Lowercase-hyphen identifier (max 64 chars).
    pub name: String,
    /// When to use the skill (max 1024 chars).
    pub description: String,
    /// Full SKILL.md body (instructions).
    pub body: String,
    pub license: Option<String>,
    pub allowed_tools: Option<String>,
    #[serde(default)]
    pub extra_files: Vec<String>,
    pub source: SourceMeta,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("empty persona: no instructions found")]
    Empty,
    #[error("missing required frontmatter field: {0}")]
    MissingField(&'static str),
    #[error("invalid skill name {0:?}: must be lowercase alphanumeric/hyphen, max 64 chars")]
    BadSkillName(String),
    #[error("skill description too long ({0} chars, max 1024)")]
    DescriptionTooLong(usize),
    #[error("frontmatter error: {0}")]
    Frontmatter(String),
    #[error("io error: {0}")]
    Io(String),
}

// ---------------------------------------------------------------------------
// Frontmatter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct PersonaFrontmatter {
    name: Option<String>,
    title: Option<String>,
    description: Option<String>,
    role: Option<String>,
    category: Option<String>,
    color: Option<String>,
    colour: Option<String>,
    emoji: Option<String>,
    vibe: Option<String>,
    model: Option<String>,
    tools: Option<ToolsField>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ToolsField {
    List(Vec<String>),
    Single(String),
}

impl ToolsField {
    fn into_vec(self) -> Vec<String> {
        match self {
            ToolsField::List(v) => v,
            ToolsField::Single(s) => s
                .split([',', ' ', '\n', '\t'])
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.trim().trim_matches(',').to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    license: Option<String>,
    #[serde(rename = "allowed-tools")]
    allowed_tools: Option<String>,
}

fn matter_parse<T>(text: &str) -> Result<(Option<T>, String), ParseError>
where
    T: Default + for<'de> Deserialize<'de>,
{
    use gray_matter::engine::YAML;
    use gray_matter::Matter;
    let matter = Matter::<YAML>::new();
    // gray_matter 0.3: parse::<T> returns ParsedEntity<T>.
    let parsed = matter
        .parse::<T>(text)
        .map_err(|e| ParseError::Frontmatter(e.to_string()))?;
    Ok((parsed.data, parsed.content))
}

fn has_frontmatter(text: &str) -> bool {
    text.trim_start().starts_with("---")
}

// ---------------------------------------------------------------------------
// Persona parsing
// ---------------------------------------------------------------------------

/// Parse a persona markdown file (YAML frontmatter + markdown body).
///
/// Handles agency-agents (`name, description, color, emoji, vibe`),
/// wshobson/VoltAgent/0xfurai/lst97 (`name, description, model, tools`)
/// and plain markdown with no frontmatter (name derived from `path`).
pub fn parse_persona_markdown(text: &str, source: SourceMeta) -> Result<PersonaDraft, ParseError> {
    let (front, body) = if has_frontmatter(text) {
        let (data, content): (Option<PersonaFrontmatter>, String) = matter_parse(text)?;
        (data.unwrap_or_default(), content)
    } else {
        (PersonaFrontmatter::default(), text.to_string())
    };
    let instructions = body.trim().to_string();
    if instructions.is_empty() {
        return Err(ParseError::Empty);
    }
    let fallback_name = Path::new(&source.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("persona")
        .replace(['_', '+'], "-");
    let name = front
        .name
        .or(front.title)
        .unwrap_or(fallback_name)
        .trim()
        .to_string();
    if name.is_empty() {
        return Err(ParseError::MissingField("name"));
    }
    let description = front.description.unwrap_or_else(|| {
        // First non-empty, non-heading line of the body, truncated.
        instructions
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| l.trim_start_matches('#').trim().to_string())
            .next()
            .unwrap_or_default()
    });
    let description = truncate(&description, 500);
    Ok(PersonaDraft {
        name,
        description,
        instructions,
        role: front.role,
        category: front.category.or_else(|| category_from_path(&source.path)),
        color: front.color.or(front.colour),
        emoji: front.emoji,
        vibe: front.vibe,
        model: front.model,
        tools: front.tools.map(ToolsField::into_vec).unwrap_or_default(),
        source,
    })
}

fn category_from_path(path: &str) -> Option<String> {
    // agency-agents layout: `<category>/<name>.md`; wshobson:
    // `plugins/<plugin>/agents/<name>.md`; VoltAgent: `<category>/<name>.md`.
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 {
        if parts[0] == "plugins" && parts.len() >= 4 {
            return Some(parts[1].to_string());
        }
        if parts[0] == "agents" || parts[0] == "data" || parts[0] == "patterns" {
            return None;
        }
        return Some(parts[parts.len() - 2].to_string());
    }
    None
}

/// Parse a Fabric pattern (`data/patterns/<name>/system.md`, no frontmatter).
pub fn parse_fabric_pattern(
    pattern_name: &str,
    text: &str,
    source: SourceMeta,
) -> Result<PersonaDraft, ParseError> {
    let instructions = text.trim().to_string();
    if instructions.is_empty() {
        return Err(ParseError::Empty);
    }
    let description = instructions
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.trim_start_matches('#').trim().to_string())
        .next()
        .unwrap_or_else(|| format!("Fabric analysis pattern: {pattern_name}"));
    Ok(PersonaDraft {
        name: pattern_name.trim().to_string(),
        description: truncate(&description, 500),
        instructions,
        role: Some("analysis framework".to_string()),
        category: Some("fabric".to_string()),
        color: None,
        emoji: None,
        vibe: None,
        model: None,
        tools: vec![],
        source,
    })
}

/// The same Fabric pattern importable as a skill draft.
pub fn parse_fabric_pattern_as_skill(
    pattern_name: &str,
    text: &str,
    source: SourceMeta,
) -> Result<SkillDraft, ParseError> {
    let body = text.trim().to_string();
    if body.is_empty() {
        return Err(ParseError::Empty);
    }
    let slug = slugify(pattern_name);
    validate_skill_name(&slug)?;
    let first_line = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.trim_start_matches('#').trim().to_string())
        .next()
        .unwrap_or_else(|| format!("Fabric analysis pattern: {pattern_name}"));
    Ok(SkillDraft {
        name: slug,
        description: truncate(&format!("{pattern_name}: {first_line}"), 1024),
        body,
        license: Some(source.license.clone()),
        allowed_tools: None,
        extra_files: vec![],
        source,
    })
}

// ---------------------------------------------------------------------------
// Skill parsing
// ---------------------------------------------------------------------------

/// Parse a `SKILL.md` file (agentskills.io format).
pub fn parse_skill_md(text: &str, source: SourceMeta) -> Result<SkillDraft, ParseError> {
    let (front, body) = if has_frontmatter(text) {
        let (data, content): (Option<SkillFrontmatter>, String) = matter_parse(text)?;
        (data.unwrap_or_default(), content)
    } else {
        (SkillFrontmatter::default(), text.to_string())
    };
    let body = body.trim().to_string();
    if body.is_empty() {
        return Err(ParseError::Empty);
    }
    let name = front
        .name
        .or_else(|| {
            Path::new(&source.path)
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .ok_or(ParseError::MissingField("name"))?;
    validate_skill_name(&name)?;
    let description = front
        .description
        .ok_or(ParseError::MissingField("description"))?;
    let description = description.trim().to_string();
    if description.is_empty() {
        return Err(ParseError::MissingField("description"));
    }
    if description.len() > 1024 {
        return Err(ParseError::DescriptionTooLong(description.len()));
    }
    Ok(SkillDraft {
        name,
        description,
        body,
        license: front.license.or_else(|| Some(source.license.clone())),
        allowed_tools: front.allowed_tools,
        extra_files: vec![],
        source,
    })
}

/// Parse a skill folder on disk (`<skill>/SKILL.md` + optional
/// `scripts/`, `references/`, `assets/` siblings).
pub fn parse_skill_dir(dir: &Path, source: SourceMeta) -> Result<SkillDraft, ParseError> {
    let skill_md = dir.join("SKILL.md");
    let text = std::fs::read_to_string(&skill_md).map_err(|e| ParseError::Io(e.to_string()))?;
    let mut draft = parse_skill_md(&text, source)?;
    let mut extra = Vec::new();
    for sub in ["scripts", "references", "assets"] {
        let d = dir.join(sub);
        if d.is_dir() {
            collect_files(&d, dir, &mut extra);
        }
    }
    extra.sort();
    draft.extra_files = extra;
    Ok(draft)
}

fn collect_files(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_files(&p, root, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

pub fn validate_skill_name(name: &str) -> Result<(), ParseError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ParseError::BadSkillName(name.to_string()));
    }
    Ok(())
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if matches!(c, '-' | '_' | ' ' | '.') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "skill".to_string()
    } else {
        truncate(&trimmed, 64)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

// ---------------------------------------------------------------------------
// Offline starter set (embedded, works with no network)
// ---------------------------------------------------------------------------

/// Curated offline starter personas embedded at compile time from
/// `assets/personas/*.md`.
pub fn starter_personas() -> Vec<PersonaDraft> {
    STARTER_PERSONAS
        .iter()
        .map(|(path, text)| {
            parse_persona_markdown(
                text,
                SourceMeta::new("synapse", "bundled", "MIT", (*path).to_string()),
            )
            .expect("bundled starter persona must parse")
        })
        .collect()
}

/// Built-in starter skills embedded at compile time.
pub fn starter_skills() -> Vec<SkillDraft> {
    STARTER_SKILLS
        .iter()
        .map(|(path, text)| {
            parse_skill_md(
                text,
                SourceMeta::new("synapse", "bundled", "MIT", (*path).to_string()),
            )
            .expect("bundled starter skill must parse")
        })
        .collect()
}

const STARTER_PERSONAS: &[(&str, &str)] = &[
    (
        "assets/personas/lead.md",
        include_str!("../../../assets/personas/lead.md"),
    ),
    (
        "assets/personas/critic.md",
        include_str!("../../../assets/personas/critic.md"),
    ),
    (
        "assets/personas/fact-checker.md",
        include_str!("../../../assets/personas/fact-checker.md"),
    ),
    (
        "assets/personas/writer.md",
        include_str!("../../../assets/personas/writer.md"),
    ),
    (
        "assets/personas/researcher.md",
        include_str!("../../../assets/personas/researcher.md"),
    ),
    (
        "assets/personas/strategist.md",
        include_str!("../../../assets/personas/strategist.md"),
    ),
    (
        "assets/personas/analyst.md",
        include_str!("../../../assets/personas/analyst.md"),
    ),
    (
        "assets/personas/ideator.md",
        include_str!("../../../assets/personas/ideator.md"),
    ),
];

const STARTER_SKILLS: &[(&str, &str)] = &[
    (
        "assets/skills/research-method/SKILL.md",
        include_str!("../../../assets/skills/research-method/SKILL.md"),
    ),
    (
        "assets/skills/report-writing/SKILL.md",
        include_str!("../../../assets/skills/report-writing/SKILL.md"),
    ),
    (
        "assets/skills/citation-style/SKILL.md",
        include_str!("../../../assets/skills/citation-style/SKILL.md"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn src(path: &str) -> SourceMeta {
        SourceMeta::new("example/repo", "abc123", "MIT", path)
    }

    #[test]
    fn parse_agency_agents_persona() {
        let md = "---\nname: Growth Strategist\ndescription: Scales startups with experiments.\ncolor: green\nemoji: 🌱\nvibe: pragmatic\n---\n\n# Growth Strategist\n\nYou help startups grow. Rules: test everything.\n";
        let d = parse_persona_markdown(md, src("strategy/growth-strategist.md")).unwrap();
        assert_eq!(d.name, "Growth Strategist");
        assert_eq!(d.description, "Scales startups with experiments.");
        assert_eq!(d.color.as_deref(), Some("green"));
        assert_eq!(d.emoji.as_deref(), Some("🌱"));
        assert_eq!(d.vibe.as_deref(), Some("pragmatic"));
        assert_eq!(d.category.as_deref(), Some("strategy"));
        assert!(d.instructions.contains("You help startups grow"));
        assert_eq!(d.source.sha, "abc123");
        assert_eq!(d.source.license, "MIT");
    }

    #[test]
    fn parse_wshobson_style_persona() {
        let md = "---\nname: rust-pro\ndescription: Writes idiomatic Rust.\nmodel: opus\ntools: Read, Edit, Bash\n---\n\nYou are a Rust expert.\n";
        let d = parse_persona_markdown(md, src("plugins/rust/agents/rust-pro.md")).unwrap();
        assert_eq!(d.name, "rust-pro");
        assert_eq!(d.model.as_deref(), Some("opus"));
        assert_eq!(d.tools, vec!["Read", "Edit", "Bash"]);
        assert_eq!(d.category.as_deref(), Some("rust"));
    }

    #[test]
    fn parse_plain_markdown_without_frontmatter() {
        let md = "# Helpful Analyst\n\nYou analyse data carefully.\n";
        let d = parse_persona_markdown(md, src("analyst.md")).unwrap();
        assert_eq!(d.name, "analyst");
        assert!(d.instructions.contains("analyse data"));
        assert!(!d.description.is_empty());
    }

    #[test]
    fn parse_plain_markdown_empty_is_error() {
        let err = parse_persona_markdown("   \n", src("empty.md")).unwrap_err();
        assert_eq!(err, ParseError::Empty);
    }

    #[test]
    fn parse_fabric_pattern_body() {
        let body = "# Analyze claims\n\nBreak the claim into parts and rate each.\n";
        let d = parse_fabric_pattern(
            "analyze_claims",
            body,
            src("data/patterns/analyze_claims/system.md"),
        )
        .unwrap();
        assert_eq!(d.name, "analyze_claims");
        assert_eq!(d.category.as_deref(), Some("fabric"));
        assert!(d.instructions.contains("Break the claim"));
        // Same pattern is also importable as a skill.
        let s = parse_fabric_pattern_as_skill(
            "analyze_claims",
            body,
            src("data/patterns/analyze_claims/system.md"),
        )
        .unwrap();
        assert_eq!(s.name, "analyze-claims");
        assert!(s.body.contains("Break the claim"));
    }

    #[test]
    fn parse_skill_md_frontmatter() {
        let md = "---\nname: pdf-reading\ndescription: Reads PDFs and extracts tables.\nlicense: Apache-2.0\nallowed-tools: Read Grep\n---\n\n# PDF reading\n\nSteps to read a PDF.\n";
        let s = parse_skill_md(md, src("pdf-reading/SKILL.md")).unwrap();
        assert_eq!(s.name, "pdf-reading");
        assert_eq!(s.description, "Reads PDFs and extracts tables.");
        assert_eq!(s.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(s.allowed_tools.as_deref(), Some("Read Grep"));
        assert!(s.body.contains("Steps to read"));
    }

    #[test]
    fn parse_skill_md_rejects_bad_name_and_missing_description() {
        let bad = "---\nname: Bad_Name!\ndescription: x\n---\n\nbody\n";
        assert!(matches!(
            parse_skill_md(bad, src("x/SKILL.md")).unwrap_err(),
            ParseError::BadSkillName(_)
        ));
        let missing = "---\nname: ok-name\n---\n\nbody\n";
        assert_eq!(
            parse_skill_md(missing, src("x/SKILL.md")).unwrap_err(),
            ParseError::MissingField("description")
        );
        let long = format!(
            "---\nname: ok-name\ndescription: {}\n---\n\nbody\n",
            "d".repeat(1025)
        );
        assert!(matches!(
            parse_skill_md(&long, src("x/SKILL.md")).unwrap_err(),
            ParseError::DescriptionTooLong(_)
        ));
    }

    #[test]
    fn parse_skill_dir_lists_extra_files() {
        let dir = std::env::temp_dir().join(format!("synapse-skill-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::create_dir_all(dir.join("references")).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: Does things.\n---\n\nDo things well.\n",
        )
        .unwrap();
        std::fs::write(dir.join("scripts/run.sh"), "echo hi\n").unwrap();
        std::fs::write(dir.join("references/notes.md"), "notes\n").unwrap();
        let s = parse_skill_dir(&dir, src("my-skill/SKILL.md")).unwrap();
        assert_eq!(s.name, "my-skill");
        assert!(s.extra_files.contains(&"scripts/run.sh".to_string()));
        assert!(s.extra_files.contains(&"references/notes.md".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_starter_set_offline() {
        let personas = starter_personas();
        assert!(
            personas.len() >= 8,
            "expected starter personas, got {}",
            personas.len()
        );
        let names: Vec<_> = personas.iter().map(|p| p.name.as_str()).collect();
        for want in ["Lead", "Critic", "Fact-checker", "Writer"] {
            assert!(
                names.iter().any(|n| n.contains(want)),
                "missing {want} in {names:?}"
            );
        }
        for p in &personas {
            assert!(!p.instructions.is_empty());
            assert_eq!(p.source.license, "MIT");
        }
        let skills = starter_skills();
        assert!(skills.len() >= 3);
        for s in &skills {
            assert!(!s.description.is_empty());
        }
    }
}
