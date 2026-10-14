//! Persona library: browse, search, preview and import personas
//! (see docs/PRD.md §5.2, docs/MILESTONES.md M3 task 6).
//!
//! Toolkit-free so it is unit testable. Sources from PRD §5.2 are fetched
//! as GitHub tarballs and cached ([`LibraryCache`]); the UI searches,
//! filters, previews and imports licensed personas. Unlicensed
//! (`LinkOnly`) sources are link-only: [`import_draft`] refuses them
//! instead of silently importing. URL / local-file import parses markdown
//! through `synapse-personas` and records repo/sha/license.

use std::collections::HashMap;
use std::path::Path;

use synapse_personas::{parse_persona_markdown, PersonaDraft, SourceMeta};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("store: {0}")]
    Store(#[from] synapse_store::StoreError),
    #[error("source is link-only (no license): {0} — open {1} instead")]
    LinkOnly(String, String),
    #[error("source needs explicit confirmation (license {0}): {1}")]
    NeedsConfirmation(String, String),
    #[error("persona is empty: {0}")]
    Empty(String),
    #[error("io: {0}")]
    Io(String),
    #[error("fetch failed for {0}: {1}")]
    Fetch(String, String),
}

pub type Result<T> = std::result::Result<T, LibraryError>;

/// How a source may be imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportPolicy {
    /// Clear license (e.g. MIT): one-click import.
    Licensed,
    /// License NOASSERTION: import only on explicit user request
    /// (`allow_request_only = true`).
    RequestOnly,
    /// No license: never imported; UI shows a link instead.
    LinkOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaSource {
    pub repo: &'static str,
    pub label: &'static str,
    pub url: String,
    pub license: Option<&'static str>,
    pub policy: ImportPolicy,
    pub tarball: String,
}

impl PersonaSource {
    pub const fn new(
        repo: &'static str,
        label: &'static str,
        license: Option<&'static str>,
        policy: ImportPolicy,
    ) -> Self {
        Self {
            repo,
            label,
            url: String::new(),
            license,
            policy,
            tarball: String::new(),
        }
    }

    pub fn with_urls(mut self) -> Self {
        self.url = format!("https://github.com/{}", self.repo);
        self.tarball = format!("https://github.com/{}/tarball/HEAD", self.repo);
        self
    }

    pub fn license_badge(&self) -> &str {
        self.license.unwrap_or("link-only")
    }

    pub fn importable(&self, allow_request_only: bool) -> bool {
        match self.policy {
            ImportPolicy::Licensed => true,
            ImportPolicy::RequestOnly => allow_request_only,
            ImportPolicy::LinkOnly => false,
        }
    }
}

/// Built-in sources from PRD.md §5.2 (all MIT unless noted).
pub fn builtin_sources() -> Vec<PersonaSource> {
    vec![
        PersonaSource::new(
            "msitarzewski/agency-agents",
            "Agency agents (team members)",
            Some("MIT"),
            ImportPolicy::Licensed,
        )
        .with_urls(),
        PersonaSource::new(
            "danielmiessler/Fabric",
            "Fabric patterns (analysis frameworks)",
            Some("MIT"),
            ImportPolicy::Licensed,
        )
        .with_urls(),
        PersonaSource::new(
            "K-Dense-AI/scientific-agent-skills",
            "Scientific agent skills",
            Some("MIT"),
            ImportPolicy::Licensed,
        )
        .with_urls(),
        PersonaSource::new(
            "wshobson/agents",
            "Technical specialists + skills",
            Some("MIT"),
            ImportPolicy::Licensed,
        )
        .with_urls(),
        PersonaSource::new(
            "VoltAgent/awesome-claude-code-subagents",
            "Technical specialists",
            Some("MIT"),
            ImportPolicy::Licensed,
        )
        .with_urls(),
        PersonaSource::new(
            "0xfurai/claude-code-subagents",
            "Extra specialists",
            Some("MIT"),
            ImportPolicy::Licensed,
        )
        .with_urls(),
        PersonaSource::new(
            "anthropics/skills",
            "Agent Skills reference set",
            Some("per-skill"),
            ImportPolicy::Licensed,
        )
        .with_urls(),
        PersonaSource::new(
            "f/prompts.chat",
            "Role prompts (license unconfirmed)",
            None,
            ImportPolicy::RequestOnly,
        )
        .with_urls(),
        PersonaSource::new(
            "contains-studio/agents",
            "Agents (no license)",
            None,
            ImportPolicy::LinkOnly,
        )
        .with_urls(),
    ]
}

pub fn source_for(repo: &str) -> Option<PersonaSource> {
    builtin_sources().into_iter().find(|s| s.repo == repo)
}

/// Cache of tarball-fetched personas, keyed by repo.
/// Insert with [`LibraryCache::insert_tarball`]; reads are offline.
#[derive(Debug, Default)]
pub struct LibraryCache {
    entries: HashMap<String, Vec<PersonaDraft>>,
    shas: HashMap<String, String>,
}

impl LibraryCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store one fetched GitHub tarball: parse each `(path, text)` file
    /// into a draft. Fabric `system.md` files become fabric drafts.
    pub fn insert_tarball(
        &mut self,
        repo: &str,
        sha: &str,
        files: Vec<(String, String)>,
        license: &str,
    ) {
        let mut out = Vec::new();
        for (path, text) in files {
            let source = SourceMeta::new(repo, sha, license, path.clone());
            let parsed = if path.ends_with("system.md") {
                let name = Path::new(&path)
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or("pattern");
                synapse_personas::parse_fabric_pattern(name, &text, source).ok()
            } else {
                parse_persona_markdown(&text, source).ok()
            };
            if let Some(d) = parsed {
                out.push(d);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        self.shas.insert(repo.to_string(), sha.to_string());
        self.entries.insert(repo.to_string(), out);
    }

    pub fn entries_for(&self, repo: &str) -> &[PersonaDraft] {
        self.entries.get(repo).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn sha_for(&self, repo: &str) -> Option<&str> {
        self.shas.get(repo).map(String::as_str)
    }

    pub fn all_entries(&self) -> Vec<&PersonaDraft> {
        let mut out: Vec<&PersonaDraft> = self.entries.values().flatten().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn is_cached(&self, repo: &str) -> bool {
        self.entries.contains_key(repo)
    }

    /// Starter set so the library works offline with no tarball yet.
    pub fn with_starter() -> Self {
        let mut cache = Self::new();
        cache
            .entries
            .insert("synapse".to_string(), synapse_personas::starter_personas());
        cache
            .shas
            .insert("synapse".to_string(), "bundled".to_string());
        cache
    }
}

/// Search + category/source filter over library entries.
/// Empty query matches all; category/source filters are exact (case-insensitive).
pub fn search<'a>(
    entries: &'a [PersonaDraft],
    query: &str,
    category: Option<&str>,
    source_repo: Option<&str>,
) -> Vec<&'a PersonaDraft> {
    let q = query.trim().to_lowercase();
    entries
        .iter()
        .filter(|d| {
            if let Some(cat) = category {
                let cat = cat.trim().to_lowercase();
                if !cat.is_empty() {
                    let mine = d.category.clone().unwrap_or_default().to_lowercase();
                    if mine != cat {
                        return false;
                    }
                }
            }
            if let Some(repo) = source_repo {
                let repo = repo.trim();
                if !repo.is_empty() && d.source.repo != repo {
                    return false;
                }
            }
            if q.is_empty() {
                return true;
            }
            d.name.to_lowercase().contains(&q)
                || d.description.to_lowercase().contains(&q)
                || d.instructions.to_lowercase().contains(&q)
                || d.category
                    .clone()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&q)
        })
        .collect()
}

/// Distinct categories present in the entries (sorted, for the filter menu).
pub fn categories(entries: &[PersonaDraft]) -> Vec<String> {
    let mut out: Vec<String> = entries
        .iter()
        .filter_map(|d| d.category.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    out.sort();
    out
}

/// Preview one entry by name (exact match).
pub fn preview<'a>(entries: &'a [PersonaDraft], name: &str) -> Option<&'a PersonaDraft> {
    entries.iter().find(|d| d.name == name)
}

/// License badge for an entry (`MIT`, `per-skill`, or `link-only`).
pub fn license_badge(draft: &PersonaDraft) -> &str {
    if draft.source.license.trim().is_empty() {
        "link-only"
    } else {
        draft.source.license.as_str()
    }
}

/// Whether an entry may be imported (link-only entries never are).
pub fn importable(draft: &PersonaDraft, allow_request_only: bool) -> bool {
    match source_for(&draft.source.repo) {
        Some(src) => src.importable(allow_request_only),
        None => {
            // Unknown repos: import only when they carry a license string.
            !draft.source.license.trim().is_empty()
        }
    }
}

/// Public link for a link-only (or any) entry.
pub fn link_url(draft: &PersonaDraft) -> String {
    if draft.source.repo == "synapse" {
        return draft.source.path.clone();
    }
    format!(
        "https://github.com/{}/blob/{}/{}",
        draft.source.repo, draft.source.sha, draft.source.path
    )
}

fn persona_source_ref(draft: &PersonaDraft) -> String {
    format!(
        "{}@{}:{} ({})",
        draft.source.repo, draft.source.sha, draft.source.path, draft.source.license
    )
}

/// Import one draft as an agent row. Records repo/sha/license in
/// `persona_source`. Link-only sources are rejected; request-only sources
/// need `allow_request_only = true`.
pub fn import_draft(
    store: &synapse_store::Store,
    draft: &PersonaDraft,
    allow_request_only: bool,
) -> Result<synapse_store::Agent> {
    if !importable(draft, allow_request_only) {
        if let Some(src) = source_for(&draft.source.repo) {
            match src.policy {
                ImportPolicy::LinkOnly => {
                    return Err(LibraryError::LinkOnly(
                        draft.source.repo.clone(),
                        link_url(draft),
                    ));
                }
                ImportPolicy::RequestOnly => {
                    return Err(LibraryError::NeedsConfirmation(
                        draft.source.license.clone(),
                        draft.name.clone(),
                    ));
                }
                ImportPolicy::Licensed => {}
            }
        }
        return Err(LibraryError::LinkOnly(
            draft.source.repo.clone(),
            link_url(draft),
        ));
    }
    if draft.instructions.trim().is_empty() {
        return Err(LibraryError::Empty(draft.name.clone()));
    }
    let role = draft
        .role
        .clone()
        .or_else(|| draft.category.clone())
        .unwrap_or_else(|| "team member".to_string());
    let row = store.create_agent(&synapse_store::NewAgent {
        id: synapse_store::new_id("agent"),
        name: draft.name.clone(),
        role,
        instructions: draft.instructions.clone(),
        backend: String::from(r#"{"type":"api","provider":"","model":""}"#),
        avatar: draft.emoji.clone().unwrap_or_default(),
        tools: String::from("[]"),
        permissions: String::from("[]"),
        persona_source: persona_source_ref(draft),
    })?;
    Ok(row)
}

/// Import several drafts; returns one row per draft in order.
pub fn import_many(
    store: &synapse_store::Store,
    drafts: &[PersonaDraft],
    allow_request_only: bool,
) -> Result<Vec<synapse_store::Agent>> {
    let mut out = Vec::with_capacity(drafts.len());
    for d in drafts {
        out.push(import_draft(store, d, allow_request_only)?);
    }
    Ok(out)
}

/// Parse markdown fetched from a URL into a draft. The source records the
/// URL as its path so the import stays traceable.
pub fn parse_url_import(url: &str, body: &str, license: &str) -> Result<PersonaDraft> {
    if url.trim().is_empty() {
        return Err(LibraryError::Fetch(
            String::new(),
            "empty URL: paste an https:// link to a .md file".into(),
        ));
    }
    let filename = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("remote.md");
    let source = SourceMeta::new("url", "url", license, format!("{url} ({filename})"));
    parse_persona_markdown(body, source)
        .map_err(|e| LibraryError::Empty(format!("{filename}: {e}")))
}

/// Fetch + parse a remote markdown file. `fetch` is injected so tests stay
/// offline (the UI passes a reqwest fetch).
pub fn import_from_url(
    store: &synapse_store::Store,
    url: &str,
    fetch: impl Fn(&str) -> std::result::Result<String, String>,
    allow_request_only: bool,
) -> Result<synapse_store::Agent> {
    let body = fetch(url).map_err(|e| LibraryError::Fetch(url.into(), e))?;
    // URL imports are user-requested, so request-only licenses are fine.
    let draft = parse_url_import(url, &body, "url")?;
    let _ = allow_request_only;
    if draft.instructions.trim().is_empty() {
        return Err(LibraryError::Empty(url.into()));
    }
    let row = store.create_agent(&synapse_store::NewAgent {
        id: synapse_store::new_id("agent"),
        name: draft.name.clone(),
        role: draft
            .role
            .clone()
            .or_else(|| draft.category.clone())
            .unwrap_or_else(|| "team member".to_string()),
        instructions: draft.instructions.clone(),
        backend: String::from(r#"{"type":"api","provider":"","model":""}"#),
        avatar: draft.emoji.clone().unwrap_or_default(),
        tools: String::from("[]"),
        permissions: String::from("[]"),
        persona_source: format!("url:{} (url)", url),
    })?;
    Ok(row)
}

/// Parse + import a local `.md` file.
pub fn import_from_file(
    store: &synapse_store::Store,
    path: &Path,
    read: impl Fn(&Path) -> std::result::Result<String, String>,
) -> Result<synapse_store::Agent> {
    let body = read(path).map_err(LibraryError::Io)?;
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("local.md");
    let source = SourceMeta::new(
        "local",
        "local",
        "local",
        path.to_string_lossy().into_owned(),
    );
    let draft =
        parse_persona_markdown(&body, source).map_err(|e| LibraryError::Empty(format!("{e}")))?;
    if draft.instructions.trim().is_empty() {
        return Err(LibraryError::Empty(filename.into()));
    }
    let row = store.create_agent(&synapse_store::NewAgent {
        id: synapse_store::new_id("agent"),
        name: draft.name.clone(),
        role: draft
            .role
            .clone()
            .or_else(|| draft.category.clone())
            .unwrap_or_else(|| "team member".to_string()),
        instructions: draft.instructions.clone(),
        backend: String::from(r#"{"type":"api","provider":"","model":""}"#),
        avatar: draft.emoji.clone().unwrap_or_default(),
        tools: String::from("[]"),
        permissions: String::from("[]"),
        persona_source: format!("local:{} (local)", path.to_string_lossy()),
    })?;
    Ok(row)
}

#[cfg(test)]
mod persona_library_tests {
    use super::*;

    fn licensed_draft(name: &str, category: &str) -> PersonaDraft {
        PersonaDraft {
            name: name.into(),
            description: format!("{name} helps teams think"),
            instructions: format!("# {name}\n\nYou are {name}, a careful specialist."),
            role: None,
            category: Some(category.into()),
            color: None,
            emoji: None,
            vibe: None,
            model: None,
            tools: vec![],
            source: SourceMeta::new(
                "msitarzewski/agency-agents",
                "sha123",
                "MIT",
                format!("{category}/{name}.md"),
            ),
        }
    }

    fn mem_store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().expect("store")
    }

    #[test]
    fn persona_library_sources_have_tarballs_licenses_and_link_only() {
        let sources = builtin_sources();
        assert!(sources.len() >= 9);
        for s in &sources {
            assert!(s.tarball.contains("tarball"), "{s:?}");
            assert!(s.url.contains(s.repo), "{s:?}");
        }
        let licensed = source_for("msitarzewski/agency-agents").unwrap();
        assert_eq!(licensed.license_badge(), "MIT");
        assert!(licensed.importable(false));
        let request = source_for("f/prompts.chat").unwrap();
        assert!(!request.importable(false));
        assert!(request.importable(true));
        let link_only = source_for("contains-studio/agents").unwrap();
        assert_eq!(link_only.policy, ImportPolicy::LinkOnly);
        assert_eq!(link_only.license_badge(), "link-only");
        assert!(!link_only.importable(false));
        assert!(!link_only.importable(true));
    }

    #[test]
    fn persona_library_tarball_cache_search_filter_preview() {
        let mut cache = LibraryCache::new();
        assert!(!cache.is_cached("msitarzewski/agency-agents"));
        cache.insert_tarball(
            "msitarzewski/agency-agents",
            "sha123",
            vec![
                (
                    "strategy/growth.md".into(),
                    "---\nname: Growth Strategist\ndescription: Scales startups.\n---\n\nYou scale startups.\n".into(),
                ),
                (
                    "data/patterns/analyze_claims/system.md".into(),
                    "Break claims into parts.\n".into(),
                ),
            ],
            "MIT",
        );
        assert!(cache.is_cached("msitarzewski/agency-agents"));
        assert_eq!(cache.sha_for("msitarzewski/agency-agents"), Some("sha123"));
        // Second insert replaces the cache (refresh), keeping one sha.
        cache.insert_tarball("msitarzewski/agency-agents", "sha999", vec![], "MIT");
        assert_eq!(cache.sha_for("msitarzewski/agency-agents"), Some("sha999"));
        assert!(cache.entries_for("msitarzewski/agency-agents").is_empty());

        let entries = vec![
            licensed_draft("Growth Strategist", "strategy"),
            licensed_draft("Risk Analyst", "finance"),
        ];
        let hits = search(&entries, "growth", None, None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Growth Strategist");
        let hits = search(&entries, "", Some("finance"), None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Risk Analyst");
        let hits = search(&entries, "", None, Some("msitarzewski/agency-agents"));
        assert_eq!(hits.len(), 2);
        let hits = search(&entries, "nope", None, None);
        assert!(hits.is_empty());
        assert_eq!(categories(&entries), vec!["finance", "strategy"]);
        let got = preview(&entries, "Risk Analyst").expect("preview");
        assert!(got.instructions.contains("Risk Analyst"));
        assert!(preview(&entries, "Missing").is_none());
    }

    #[test]
    fn persona_library_import_one_and_many_records_license() {
        let store = mem_store();
        let draft = licensed_draft("Growth Strategist", "strategy");
        assert_eq!(license_badge(&draft), "MIT");
        assert!(importable(&draft, false));
        let row = import_draft(&store, &draft, false).expect("import");
        assert_eq!(row.name, "Growth Strategist");
        assert!(row.instructions.contains("careful specialist"));
        assert!(row.persona_source.contains("msitarzewski/agency-agents"));
        assert!(row.persona_source.contains("MIT"));

        let rows = import_many(
            &store,
            &[
                licensed_draft("Risk Analyst", "finance"),
                licensed_draft("Brand Voice", "marketing"),
            ],
            false,
        )
        .expect("import many");
        assert_eq!(rows.len(), 2);
        assert_eq!(store.list_agents().unwrap().len(), 3);
    }

    #[test]
    fn persona_library_link_only_is_never_imported_silently() {
        let store = mem_store();
        let mut draft = licensed_draft("Linked One", "misc");
        draft.source = SourceMeta::new("contains-studio/agents", "sha1", "", "agents/one.md");
        assert_eq!(license_badge(&draft), "link-only");
        assert!(!importable(&draft, true));
        let err = import_draft(&store, &draft, true).expect_err("link-only must not import");
        assert!(err.to_string().contains("link-only"), "{err}");
        assert!(link_url(&draft).contains("contains-studio/agents"));
        assert!(store.list_agents().unwrap().is_empty());

        // Request-only needs explicit confirmation, never silent.
        let mut req = licensed_draft("Act As Coach", "roles");
        req.source = SourceMeta::new("f/prompts.chat", "sha2", "", "prompts/coach.md");
        req.source.license = "NOASSERTION".into();
        let err = import_draft(&store, &req, false).expect_err("needs confirmation");
        assert!(err.to_string().contains("confirmation"), "{err}");
        assert!(store.list_agents().unwrap().is_empty());
        let row = import_draft(&store, &req, true).expect("explicit request imports");
        assert_eq!(row.name, "Act As Coach");
    }

    #[test]
    fn persona_library_import_from_url_and_local_file() {
        let store = mem_store();
        let row = import_from_url(
            &store,
            "https://example.com/personas/coach.md",
            |_| {
                Ok("---\nname: Coach\ndescription: Coaches teams.\n---\n\nYou coach teams daily.\n"
                    .to_string())
            },
            false,
        )
        .expect("url import");
        assert_eq!(row.name, "Coach");
        assert!(
            row.persona_source.contains("https://example.com"),
            "{}",
            row.persona_source
        );

        let err = import_from_url(
            &store,
            "https://example.com/missing.md",
            |_| Err("404".into()),
            false,
        )
        .expect_err("fetch failure surfaces");
        assert!(err.to_string().contains("404"), "{err}");

        let path = Path::new("/tmp/coach-local.md");
        let row = import_from_file(&store, path, |_| {
            Ok(
                "---\nname: Local Coach\ndescription: Local file.\n---\n\nYou coach locally.\n"
                    .to_string(),
            )
        })
        .expect("file import");
        assert_eq!(row.name, "Local Coach");
        assert!(
            row.persona_source.contains("coach-local.md"),
            "{}",
            row.persona_source
        );

        let err = import_from_file(&store, Path::new("/tmp/empty.md"), |_| Ok("   \n".into()))
            .expect_err("empty file rejected");
        assert!(err.to_string().contains("empty"), "{err}");
        assert_eq!(store.list_agents().unwrap().len(), 2);
    }

    #[test]
    fn persona_library_starter_set_works_offline() {
        let cache = LibraryCache::with_starter();
        assert!(cache.is_cached("synapse"));
        let all = cache.all_entries();
        assert!(!all.is_empty());
        let names: Vec<_> = all.iter().map(|d| d.name.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("Lead")), "{names:?}");
        for d in all {
            assert_eq!(license_badge(d), "MIT");
        }
    }
}
