//! Skill library: search, preview, import, enable, update, delete skills
//! at global / project / agent scope, plus opt-in local skill folders.
//! Exposes enabled skills for the composer slash menu (`/skill-name`).
//!
//! Toolkit-free so it is unit testable (see docs/PRD.md §5.2).

use std::path::{Path, PathBuf};

use synapse_personas::{parse_skill_md, SkillDraft, SourceMeta};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillLibraryError {
    #[error("store: {0}")]
    Store(#[from] synapse_store::StoreError),
    #[error("skill error: {0}")]
    Skill(String),
    #[error("io: {0}")]
    Io(String),
    #[error("fetch failed for {0}: {1}")]
    Fetch(String, String),
    #[error("invalid scope {0:?}: use global, project or agent")]
    Scope(String),
}

pub type Result<T> = std::result::Result<T, SkillLibraryError>;

/// A remote collection skills can be imported from.
#[derive(Debug, Clone)]
pub struct SkillSource {
    pub repo: &'static str,
    pub label: &'static str,
    pub license: Option<&'static str>,
    pub tarball: String,
}

/// Built-in skill sources (PRD §5.2). Directory indexes (skills.sh,
/// ClawHub) are opt-in URLs rather than vendored entries.
pub fn builtin_skill_sources() -> Vec<SkillSource> {
    for (repo, label, license) in [
        (
            "anthropics/skills",
            "Agent Skills reference set",
            Some("per-skill"),
        ),
        (
            "K-Dense-AI/scientific-agent-skills",
            "Scientific research skills",
            Some("MIT"),
        ),
        (
            "wshobson/agents",
            "Technical skills (plugin layout)",
            Some("MIT"),
        ),
    ] {
        let _ = (repo, label, license);
    }
    vec![
        SkillSource {
            repo: "anthropics/skills",
            label: "Agent Skills reference set",
            license: Some("per-skill"),
            tarball: "https://github.com/anthropics/skills/tarball/HEAD".into(),
        },
        SkillSource {
            repo: "K-Dense-AI/scientific-agent-skills",
            label: "Scientific research skills",
            license: Some("MIT"),
            tarball: "https://github.com/K-Dense-AI/scientific-agent-skills/tarball/HEAD".into(),
        },
        SkillSource {
            repo: "wshobson/agents",
            label: "Technical skills (plugin layout)",
            license: Some("MIT"),
            tarball: "https://github.com/wshobson/agents/tarball/HEAD".into(),
        },
    ]
}

/// Cache of tarball-fetched skill drafts, keyed by repo.
#[derive(Debug, Default)]
pub struct SkillCache {
    entries: std::collections::HashMap<String, Vec<SkillDraft>>,
    shas: std::collections::HashMap<String, String>,
}

impl SkillCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store one fetched tarball: every `SKILL.md` file becomes a draft.
    pub fn insert_tarball(
        &mut self,
        repo: &str,
        sha: &str,
        files: Vec<(String, String)>,
        license: &str,
    ) {
        let mut out = Vec::new();
        for (path, text) in files {
            if !path.ends_with("SKILL.md") {
                continue;
            }
            let source = SourceMeta::new(repo, sha, license, path.clone());
            if let Ok(d) = parse_skill_md(&text, source) {
                out.push(d);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        self.shas.insert(repo.to_string(), sha.to_string());
        self.entries.insert(repo.to_string(), out);
    }

    pub fn entries_for(&self, repo: &str) -> &[SkillDraft] {
        self.entries.get(repo).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn sha_for(&self, repo: &str) -> Option<&str> {
        self.shas.get(repo).map(String::as_str)
    }

    pub fn all_entries(&self) -> Vec<&SkillDraft> {
        let mut out: Vec<&SkillDraft> = self.entries.values().flatten().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn is_cached(&self, repo: &str) -> bool {
        self.entries.contains_key(repo)
    }

    /// Starter skills so the library works offline.
    pub fn with_starter() -> Self {
        let mut cache = Self::new();
        cache
            .entries
            .insert("synapse".to_string(), synapse_personas::starter_skills());
        cache
            .shas
            .insert("synapse".to_string(), "bundled".to_string());
        cache
    }
}

/// Search installed skills or drafts. Empty query matches all.
pub fn search_skills<'a>(
    skills: &'a [synapse_store::Skill],
    query: &str,
) -> Vec<&'a synapse_store::Skill> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<&synapse_store::Skill> = skills
        .iter()
        .filter(|s| {
            q.is_empty()
                || s.name.to_lowercase().contains(&q)
                || s.description.to_lowercase().contains(&q)
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Search drafts (remote cache / local scan results).
pub fn search_drafts<'a>(drafts: &'a [SkillDraft], query: &str) -> Vec<&'a SkillDraft> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<&SkillDraft> = drafts
        .iter()
        .filter(|d| {
            q.is_empty()
                || d.name.to_lowercase().contains(&q)
                || d.description.to_lowercase().contains(&q)
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Preview one installed skill by name.
pub fn preview<'a>(
    skills: &'a [synapse_store::Skill],
    name: &str,
) -> Option<&'a synapse_store::Skill> {
    skills.iter().find(|s| s.name == name)
}

fn check_scope(scope: &str, scope_id: &str) -> Result<String> {
    match scope {
        "global" => Ok(String::new()),
        "project" | "agent" => {
            if scope_id.trim().is_empty() {
                return Err(SkillLibraryError::Scope(format!("{scope}:empty-id")));
            }
            Ok(scope_id.to_string())
        }
        other => Err(SkillLibraryError::Scope(other.to_string())),
    }
}

fn draft_to_new(draft: &SkillDraft, local_path: &str) -> synapse_store::NewSkill {
    synapse_store::NewSkill {
        id: synapse_store::new_id("skill"),
        name: draft.name.clone(),
        description: draft.description.clone(),
        body: draft.body.clone(),
        license: draft
            .license
            .clone()
            .unwrap_or_else(|| draft.source.license.clone()),
        repo: draft.source.repo.clone(),
        sha: draft.source.sha.clone(),
        path: draft.source.path.clone(),
        local_path: local_path.to_string(),
    }
}

/// Import a draft into the library and enable it at `scope`/`scope_id`.
/// Re-importing the same name updates the installed body instead of
/// duplicating the row (idempotent update path).
pub fn import_draft(
    store: &synapse_store::Store,
    draft: &SkillDraft,
    scope: &str,
    scope_id: &str,
) -> Result<synapse_store::Skill> {
    let sid = check_scope(scope, scope_id)?;
    if draft.body.trim().is_empty() {
        return Err(SkillLibraryError::Skill(format!(
            "skill {} is empty",
            draft.name
        )));
    }
    let row = match store.get_skill_by_name(&draft.name) {
        Ok(existing) => {
            let updated = store.update_skill_body(
                &existing.id,
                &draft.description,
                &draft.body,
                &draft.source.sha,
            )?;
            // Keep origin metadata fresh when the source changed.
            if updated.repo != draft.source.repo || updated.path != draft.source.path {
                // Name is the stable key; origin columns refresh on next
                // update path — record via body update only for now.
            }
            updated
        }
        Err(_) => store.create_skill(&draft_to_new(draft, ""))?,
    };
    store.set_skill_enabled(&row.id, scope, &sid, true)?;
    Ok(store.get_skill(&row.id)?)
}

/// Enable / disable an installed skill (by name or id) at a scope.
pub fn set_enabled(
    store: &synapse_store::Store,
    skill: &str,
    scope: &str,
    scope_id: &str,
    enabled: bool,
) -> Result<()> {
    let sid = check_scope(scope, scope_id)?;
    let row = store
        .get_skill_by_name(skill)
        .or_else(|_| store.get_skill(skill))
        .map_err(|_| SkillLibraryError::Skill(format!("unknown skill {skill:?}")))?;
    store.set_skill_enabled(&row.id, scope, &sid, enabled)?;
    Ok(())
}

/// Delete an installed skill (by name or id), including its scope rows.
pub fn delete_skill(store: &synapse_store::Store, skill: &str) -> Result<()> {
    let row = store
        .get_skill_by_name(skill)
        .or_else(|_| store.get_skill(skill))
        .map_err(|_| SkillLibraryError::Skill(format!("unknown skill {skill:?}")))?;
    store.delete_skill(&row.id)?;
    Ok(())
}

/// Re-fetch a skill's `SKILL.md` and update the installed body.
/// `fetch` receives the installed row and returns `(new_text, new_sha)`.
pub fn update_from_source(
    store: &synapse_store::Store,
    skill: &str,
    fetch: impl Fn(&synapse_store::Skill) -> std::result::Result<(String, String), String>,
) -> Result<synapse_store::Skill> {
    let row = store
        .get_skill_by_name(skill)
        .or_else(|_| store.get_skill(skill))
        .map_err(|_| SkillLibraryError::Skill(format!("unknown skill {skill:?}")))?;
    let (text, sha) = fetch(&row).map_err(|e| SkillLibraryError::Fetch(row.name.clone(), e))?;
    let source = SourceMeta::new(
        row.repo.clone(),
        sha.clone(),
        row.license.clone(),
        row.path.clone(),
    );
    let draft = parse_skill_md(&text, source)
        .map_err(|e| SkillLibraryError::Skill(format!("{}: {e}", row.name)))?;
    Ok(store.update_skill_body(&row.id, &draft.description, &draft.body, &sha)?)
}

/// Install the bundled starter skills (idempotent) and enable globally.
pub fn ensure_starter(store: &synapse_store::Store) -> Result<Vec<synapse_store::Skill>> {
    let mut out = Vec::new();
    for draft in synapse_personas::starter_skills() {
        out.push(import_draft(store, &draft, "global", "")?);
    }
    Ok(out)
}

// -- Opt-in local skill folders --------------------------------------------

const LOCAL_DIRS_KEY: &str = "skills.local_dirs";

fn read_local_dirs(store: &synapse_store::Store) -> Vec<String> {
    let raw = store
        .setting_get(LOCAL_DIRS_KEY)
        .unwrap_or(None)
        .unwrap_or_default();
    if raw.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default()
}

/// Opt-in local folders whose `*/SKILL.md` files are offered as skills.
pub fn local_dirs(store: &synapse_store::Store) -> Vec<PathBuf> {
    read_local_dirs(store).iter().map(PathBuf::from).collect()
}

/// Opt in a local folder (must exist). Idempotent.
pub fn add_local_dir(store: &synapse_store::Store, dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Err(SkillLibraryError::Io(format!(
            "{}: not a directory",
            dir.display()
        )));
    }
    let mut dirs = read_local_dirs(store);
    let canon = dir
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| dir.to_string_lossy().into_owned());
    if !dirs.iter().any(|d| d == &canon) {
        dirs.push(canon);
        dirs.sort();
    }
    store
        .setting_set(
            LOCAL_DIRS_KEY,
            &serde_json::to_string(&dirs).unwrap_or_default(),
        )
        .map_err(SkillLibraryError::Store)?;
    Ok(())
}

/// Remove a folder from the opt-in list (installed skills stay).
pub fn remove_local_dir(store: &synapse_store::Store, dir: &str) -> Result<()> {
    let mut dirs = read_local_dirs(store);
    let before = dirs.len();
    dirs.retain(|d| {
        d != dir
            && Path::new(d)
                .file_name()
                .is_some_and(|_| Path::new(d) != Path::new(dir))
    });
    // Fall back to plain string compare for non-canonical entries.
    dirs.retain(|d| d != dir);
    if dirs.len() == before {
        return Ok(());
    }
    store
        .setting_set(
            LOCAL_DIRS_KEY,
            &serde_json::to_string(&dirs).unwrap_or_default(),
        )
        .map_err(SkillLibraryError::Store)?;
    Ok(())
}

/// Scan one folder: each immediate subdir with a `SKILL.md` is a draft.
/// Missing folder → empty list (opt-in folders may be unplugged).
pub fn scan_local_dir(dir: &Path) -> Vec<SkillDraft> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for sub in dirs {
        let skill_md = sub.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let name = sub
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("local-skill");
        let source = SourceMeta::new(
            "local",
            "local",
            "local",
            sub.to_string_lossy().into_owned(),
        );
        match synapse_personas::parse_skill_dir(
            &sub,
            SourceMeta::new("local", "local", "local", format!("{name}/SKILL.md")),
        ) {
            Ok(mut draft) => {
                draft.source = source;
                out.push(draft);
            }
            Err(_) => continue,
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Scan all opted-in local folders.
pub fn scan_local_dirs(store: &synapse_store::Store) -> Vec<SkillDraft> {
    let mut out = Vec::new();
    for dir in local_dirs(store) {
        out.extend(scan_local_dir(&dir));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Import a local skill folder (`dir/<name>/SKILL.md` layout or the folder
/// itself) and enable it at `scope`/`scope_id`.
pub fn import_local_skill(
    store: &synapse_store::Store,
    dir: &Path,
    scope: &str,
    scope_id: &str,
) -> Result<synapse_store::Skill> {
    let sid = check_scope(scope, scope_id)?;
    let skill_md_direct = dir.join("SKILL.md");
    let (skill_dir, fallback_name) = if skill_md_direct.is_file() {
        (
            dir.to_path_buf(),
            dir.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("local-skill")
                .to_string(),
        )
    } else {
        // Maybe `dir` is the parent holding one skill folder: pick the first.
        let mut subs: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| SkillLibraryError::Io(e.to_string()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("SKILL.md").is_file())
            .collect();
        subs.sort();
        let first = subs.into_iter().next().ok_or_else(|| {
            SkillLibraryError::Io(format!("{}: no SKILL.md found", dir.display()))
        })?;
        let name = first
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("local-skill")
            .to_string();
        (first, name)
    };
    let _ = fallback_name;
    let source = SourceMeta::new(
        "local",
        "local",
        "local",
        format!("{}/SKILL.md", skill_dir.to_string_lossy()),
    );
    let mut draft = synapse_personas::parse_skill_dir(&skill_dir, source)
        .map_err(|e| SkillLibraryError::Skill(e.to_string()))?;
    draft.source.repo = "local".to_string();
    draft.source.sha = "local".to_string();
    draft.source.path = skill_dir.to_string_lossy().into_owned();
    let local_path = skill_dir.to_string_lossy().into_owned();
    let row = match store.get_skill_by_name(&draft.name) {
        Ok(existing) => {
            store.update_skill_body(&existing.id, &draft.description, &draft.body, "local")?
        }
        Err(_) => {
            let mut new = draft_to_new(&draft, &local_path);
            new.repo = "local".to_string();
            new.sha = "local".to_string();
            new.path = local_path.clone();
            store.create_skill(&new)?
        }
    };
    store.set_skill_enabled(&row.id, scope, &sid, true)?;
    Ok(store.get_skill(&row.id)?)
}

// -- Composer slash menu ----------------------------------------------------

/// One slash-menu entry for an enabled skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashItem {
    pub command: String,
    pub description: String,
}

/// Enabled skills for `project_id`/`agent_id` as `/name` entries,
/// filtered by the typed query (without the leading slash).
/// `agent_id` may be empty for the team-room composer.
pub fn slash_menu(
    store: &synapse_store::Store,
    query: &str,
    project_id: &str,
    agent_id: &str,
) -> Result<Vec<SlashItem>> {
    let skills = store.enabled_skills(project_id, agent_id)?;
    let q = query.trim().trim_start_matches('/').to_lowercase();
    let mut out: Vec<SlashItem> = skills
        .into_iter()
        .filter(|s| {
            q.is_empty()
                || s.name.to_lowercase().contains(&q)
                || s.description.to_lowercase().contains(&q)
        })
        .map(|s| SlashItem {
            command: format!("/{}", s.name),
            description: s.description.clone(),
        })
        .collect();
    out.sort_by(|a, b| a.command.cmp(&b.command));
    Ok(out)
}

/// `name + description` lines for the agent system prompt (progressive
/// disclosure: full SKILL.md is loaded via `load_skill` only when used).
pub fn prompt_snippet(
    store: &synapse_store::Store,
    project_id: &str,
    agent_id: &str,
) -> Result<String> {
    let skills = store.enabled_skills(project_id, agent_id)?;
    Ok(skills
        .iter()
        .map(|s| format!("- {}: {}", s.name, s.description))
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod skill_library_tests {
    use super::*;

    fn mem_store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().expect("store")
    }

    fn draft(name: &str, desc: &str) -> SkillDraft {
        SkillDraft {
            name: name.into(),
            description: desc.into(),
            body: format!("# {name}\n\nDo {name} things well.\n"),
            license: Some("MIT".into()),
            allowed_tools: None,
            extra_files: vec![],
            source: SourceMeta::new(
                "anthropics/skills",
                "sha1",
                "MIT",
                format!("{name}/SKILL.md"),
            ),
        }
    }

    #[test]
    fn skill_library_search_preview_import_enable_scopes() {
        let store = mem_store();
        assert!(store.list_skills().unwrap().is_empty());

        let a = import_draft(&store, &draft("pdf-reading", "Reads PDFs"), "global", "").unwrap();
        assert_eq!(a.name, "pdf-reading");
        let b = import_draft(
            &store,
            &draft("web-extract", "Extracts web tables"),
            "project",
            "p1",
        )
        .unwrap();
        assert_eq!(b.repo, "anthropics/skills");

        // Re-import updates in place (no duplicate row).
        let a2 = import_draft(
            &store,
            &SkillDraft {
                description: "Reads PDFs fast".into(),
                body: "# pdf-reading\n\nFast path.\n".into(),
                source: SourceMeta::new("anthropics/skills", "sha2", "MIT", "pdf-reading/SKILL.md"),
                ..draft("pdf-reading", "Reads PDFs fast")
            },
            "global",
            "",
        )
        .unwrap();
        assert_eq!(a.id, a2.id);
        assert_eq!(a2.description, "Reads PDFs fast");
        assert_eq!(store.list_skills().unwrap().len(), 2);

        // Search + preview over installed skills.
        let all = store.list_skills().unwrap();
        assert_eq!(search_skills(&all, "").len(), 2);
        assert_eq!(search_skills(&all, "pdf").len(), 1);
        assert_eq!(search_skills(&all, "PDF").len(), 1);
        assert!(search_skills(&all, "nope").is_empty());
        assert_eq!(preview(&all, "pdf-reading").unwrap().id, a.id);
        assert!(preview(&all, "missing").is_none());

        // Scopes: global skill visible everywhere; project skill only in p1.
        let enabled = store.enabled_skills("p1", "agent1").unwrap();
        assert_eq!(enabled.len(), 2);
        let enabled = store.enabled_skills("p9", "agent1").unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "pdf-reading");

        // Per-agent enable + disable override.
        set_enabled(&store, "web-extract", "agent", "agent1", true).unwrap();
        let enabled = store.enabled_skills("p9", "agent1").unwrap();
        assert_eq!(enabled.len(), 2);
        set_enabled(&store, "pdf-reading", "project", "p1", false).unwrap();
        let names: Vec<_> = store
            .enabled_skills("p1", "nobody")
            .unwrap()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert!(!names.contains(&"pdf-reading".to_string()), "{names:?}");
        // Other projects unaffected.
        assert!(store
            .enabled_skills("p9", "nobody")
            .unwrap()
            .iter()
            .any(|s| s.name == "pdf-reading"));

        // Bad scope / unknown skill rejected.
        assert!(import_draft(&store, &draft("x", "y"), "team", "").is_err());
        assert!(set_enabled(&store, "nope", "global", "", true).is_err());
        assert!(set_enabled(&store, "pdf-reading", "project", "", true).is_err());
    }

    #[test]
    fn skill_library_update_from_source_and_delete() {
        let store = mem_store();
        import_draft(&store, &draft("pdf-reading", "Reads PDFs"), "global", "").unwrap();

        let updated = update_from_source(&store, "pdf-reading", |row| {
            assert_eq!(row.name, "pdf-reading");
            Ok((
                "---\nname: pdf-reading\ndescription: Reads PDFs v2.\n---\n\nNew body v2.\n"
                    .to_string(),
                "sha-v2".to_string(),
            ))
        })
        .unwrap();
        assert_eq!(updated.description, "Reads PDFs v2.");
        assert!(updated.body.contains("v2"));
        assert_eq!(updated.sha, "sha-v2");

        let err = update_from_source(&store, "pdf-reading", |_| Err("404".to_string()))
            .expect_err("fetch failure surfaces");
        assert!(err.to_string().contains("404"), "{err}");

        delete_skill(&store, "pdf-reading").unwrap();
        assert!(store.list_skills().unwrap().is_empty());
        assert!(store.enabled_skills("p1", "a1").unwrap().is_empty());
        assert!(delete_skill(&store, "pdf-reading").is_err());
    }

    #[test]
    fn skill_library_tarball_cache_search_and_starter() {
        let mut cache = SkillCache::new();
        assert!(!cache.is_cached("anthropics/skills"));
        cache.insert_tarball(
            "anthropics/skills",
            "sha1",
            vec![
                (
                    "pdf-reading/SKILL.md".into(),
                    "---\nname: pdf-reading\ndescription: Reads PDFs.\n---\n\nRead them.\n".into(),
                ),
                ("README.md".into(), "# not a skill\n".into()),
                (
                    "bad/SKILL.md".into(),
                    "---\nname: BAD!\ndescription: x\n---\n\nbody\n".into(),
                ),
            ],
            "MIT",
        );
        assert!(cache.is_cached("anthropics/skills"));
        assert_eq!(cache.sha_for("anthropics/skills"), Some("sha1"));
        assert_eq!(cache.entries_for("anthropics/skills").len(), 1);

        let all = cache.all_entries();
        assert_eq!(all.len(), 1);
        let drafts: Vec<SkillDraft> = cache.all_entries().into_iter().cloned().collect();
        assert_eq!(search_drafts(&drafts, "pdf").len(), 1);
        assert!(search_drafts(&drafts, "zzz").is_empty());

        // Starter cache + ensure_starter work offline.
        let starter = SkillCache::with_starter();
        assert!(starter.is_cached("synapse"));
        assert!(!starter.all_entries().is_empty());
        assert!(!builtin_skill_sources().is_empty());

        let store = mem_store();
        let rows = ensure_starter(&store).unwrap();
        assert!(!rows.is_empty());
        // Idempotent: second run keeps the same rows.
        let rows2 = ensure_starter(&store).unwrap();
        assert_eq!(rows.len(), rows2.len());
        assert_eq!(store.list_skills().unwrap().len(), rows.len());
        for s in store.list_skills().unwrap() {
            let states = store.skill_states(&s.id).unwrap();
            assert!(states
                .iter()
                .any(|(sc, id, en)| sc == "global" && id.is_empty() && *en));
        }
    }

    #[test]
    fn skill_library_local_dirs_opt_in_scan_import() {
        let store = mem_store();
        assert!(local_dirs(&store).is_empty());
        assert!(scan_local_dirs(&store).is_empty());

        let base = std::env::temp_dir().join(format!("synapse-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let skill_dir = base.join("my-local-skill");
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-local-skill\ndescription: Local helper.\n---\n\nHelp locally.\n",
        )
        .unwrap();
        std::fs::write(skill_dir.join("references/notes.md"), "notes\n").unwrap();

        // Not a directory → rejected.
        assert!(add_local_dir(&store, &skill_dir.join("SKILL.md")).is_err());
        add_local_dir(&store, &base).unwrap();
        // Idempotent second add.
        add_local_dir(&store, &base).unwrap();
        assert_eq!(local_dirs(&store).len(), 1);

        let found = scan_local_dirs(&store);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "my-local-skill");
        assert_eq!(found[0].source.repo, "local");

        // Import from the folder, enabled at project scope.
        let row = import_local_skill(&store, &skill_dir, "project", "p1").unwrap();
        assert_eq!(row.name, "my-local-skill");
        assert!(!row.local_path.is_empty());
        assert_eq!(store.enabled_skills("p1", "a").unwrap().len(), 1);
        assert!(store.enabled_skills("other", "a").unwrap().is_empty());

        // Missing folder scans to empty, never errors.
        assert!(scan_local_dir(Path::new("/nonexistent-synapse-skills-dir")).is_empty());

        let dir = local_dirs(&store)[0].to_string_lossy().into_owned();
        remove_local_dir(&store, &dir).unwrap();
        assert!(local_dirs(&store).is_empty());
        // Installed skill survives opt-out.
        assert_eq!(store.list_skills().unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn skill_library_slash_menu_lists_enabled_skills() {
        let store = mem_store();
        ensure_starter(&store).unwrap();
        import_draft(&store, &draft("pdf-reading", "Reads PDFs"), "global", "").unwrap();
        import_draft(
            &store,
            &draft("secret-skill", "Project secret"),
            "project",
            "p1",
        )
        .unwrap();

        // Team-room composer (no agent): global + project skills.
        let menu = slash_menu(&store, "", "p1", "").unwrap();
        assert!(menu.iter().any(|i| i.command == "/pdf-reading"), "{menu:?}");
        assert!(
            menu.iter().any(|i| i.command == "/secret-skill"),
            "{menu:?}"
        );
        // Sorted and prefixed.
        let cmds: Vec<_> = menu.iter().map(|i| i.command.clone()).collect();
        assert_eq!(cmds, {
            let mut c = cmds.clone();
            c.sort();
            c
        });
        for item in &menu {
            assert!(item.command.starts_with('/'));
            assert!(!item.description.is_empty());
        }

        // Query filters; leading slash tolerated.
        let menu = slash_menu(&store, "/pdf", "p1", "").unwrap();
        assert_eq!(menu.len(), 1);
        assert_eq!(menu[0].command, "/pdf-reading");

        // Other projects don't see project-scoped skills.
        let menu = slash_menu(&store, "", "other", "").unwrap();
        assert!(!menu.iter().any(|i| i.command == "/secret-skill"));

        // Disabled skills leave the menu.
        set_enabled(&store, "pdf-reading", "global", "", false).unwrap();
        let menu = slash_menu(&store, "", "p1", "").unwrap();
        assert!(!menu.iter().any(|i| i.command == "/pdf-reading"));

        // Prompt snippet lists name + description lines only.
        let snippet = prompt_snippet(&store, "p1", "").unwrap();
        assert!(snippet.contains("secret-skill"), "{snippet}");
        assert!(!snippet.contains("SKILL.md"));
    }
}
