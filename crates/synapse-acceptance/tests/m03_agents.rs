//! M3 agents/personas/avatars acceptance test (see docs/MILESTONES.md §M3).
//!
//! Offline end-to-end through the real crates: agent editor create/edit with
//! persistence, avatar customize + render, licensed persona browse/import
//! (link-only rejected), and a project team assembled from a built-in
//! template. No network, no credentials.

use synapse_app::agent_editor::{save_editor, AgentEditorState, EditorSheet};
use synapse_app::avatar::{Accessory, AvatarSpec, Eyes, Mouth, Shape};
use synapse_app::persona_library::{
    builtin_sources, import_draft, import_from_file, import_from_url, link_url, search,
    LibraryCache, LibraryError,
};
use synapse_core::{api_draft, builtin_template, AgentStore, ProjectDraft, ProjectStore};
use synapse_personas::{parse_persona_markdown, starter_personas, SourceMeta};

fn mem_store() -> synapse_store::Store {
    synapse_store::Store::open_in_memory().expect("open in-memory store")
}

fn good_editor(name: &str) -> AgentEditorState {
    let mut e = AgentEditorState::open_new();
    e.name = name.into();
    e.role = "researcher".into();
    e.instructions = "Be rigorous and cite sources.".into();
    e.provider = "openai".into();
    e.model = "gpt-4o".into();
    e.tools = vec!["web_search".into(), "read_url".into()];
    e
}

// --- 1. create + edit an agent through the editor; it persists ---

#[test]
fn m03_agent_editor_creates_edits_and_persists() {
    let store = mem_store();

    // Create through the editor sheet (Team panel "add agent" flow).
    let mut sheet = EditorSheet::new();
    sheet.open_new();
    assert!(sheet.is_open());
    sheet.editor = Some(good_editor("Ada"));
    let created = sheet.save_open(&store).expect("create saves");
    assert_eq!(created.name, "Ada");
    assert_eq!(created.role, "researcher");
    assert!(!created.id.is_empty());
    assert!(!sheet.is_open());

    // Roster shows it; the row survives a fresh read from SQLite.
    let roster = synapse_app::agent_editor::roster(&store).expect("roster");
    assert!(roster.iter().any(|a| a.id == created.id));
    let reloaded = store.get_agent(&created.id).expect("reload row");
    assert_eq!(reloaded.instructions, "Be rigorous and cite sources.");

    // Edit through the sheet: reopen, change instructions, save.
    let mut edit = EditorSheet::new();
    edit.open_existing(&store, &created.id)
        .expect("open existing");
    assert_eq!(edit.open_agent_id(), Some(created.id.as_str()));
    let mut state = edit.editor.clone().expect("editor state");
    state.instructions = "Be even more rigorous.".into();
    state.toggle_tool("load_skill");
    assert!(state.is_valid());
    edit.editor = Some(state);
    let updated = edit.save_open(&store).expect("edit saves");
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.instructions, "Be even more rigorous.");
    let tools: Vec<String> = serde_json::from_str(&updated.tools).expect("tools json");
    assert!(tools.contains(&"load_skill".to_string()));

    // Direct save_editor path also persists edits.
    let row = store.get_agent(&created.id).expect("row");
    let mut direct = AgentEditorState::open_existing(&row);
    direct.role = "fact-checker".into();
    let saved = save_editor(&store, &direct).expect("direct save");
    assert_eq!(saved.role, "fact-checker");
    assert_eq!(
        store.get_agent(&saved.id).expect("reload").role,
        "fact-checker"
    );

    // Validation: a blank draft explains itself and saves nothing.
    let mut bad = AgentEditorState::open_new();
    bad.name = "  ".into();
    let issues = bad.issues();
    assert!(!issues.is_empty(), "blank draft must list issues");
    assert!(save_editor(&store, &bad).is_err());
}

// --- 2. customize + render an avatar ---

#[test]
fn m03_avatar_customize_render_and_roundtrip() {
    // Deterministic default from the agent name.
    let a = AvatarSpec::from_name("Ada");
    let b = AvatarSpec::from_name("Ada");
    assert_eq!(a, b, "same name must give the same look");
    assert_ne!(
        AvatarSpec::from_name("Ada"),
        AvatarSpec::from_name("Bo"),
        "different names should usually differ"
    );

    // Lead default is the green bean with diamond eyes (UI.md §4).
    let lead = AvatarSpec::default_lead();
    assert_eq!(lead.shape, Shape::Bean);
    assert_eq!(lead.eyes, Eyes::Diamond);

    // Customize every knob and render SVG at both detail levels.
    let mut spec = AvatarSpec::from_name("Ada");
    spec.shape = Shape::Cat;
    spec.palette = 3;
    spec.eyes = Eyes::Happy;
    spec.mouth = Mouth::Smile;
    spec.accessories = vec![Accessory::Glasses];
    assert!(spec.is_valid());
    let full = spec.to_svg(96);
    assert!(full.contains("<svg"), "full render must be SVG, got {full}");
    assert!(full.contains("Glasses") || full.contains("<g") || full.contains("path"));
    let small = spec.to_svg(16);
    assert!(small.contains("<svg"));
    assert!(
        AvatarSpec::detail(16) != AvatarSpec::detail(96),
        "small sizes render minimal detail"
    );

    // JSON round-trips through the agent row.
    let json = spec.to_json();
    let back = AvatarSpec::from_json(&json);
    assert_eq!(back, spec);
    assert!(AvatarSpec::from_json("not json").is_valid());

    // "Surprise me" is deterministic per seed and stays valid.
    assert_eq!(
        AvatarSpec::surprise_me_from(7),
        AvatarSpec::surprise_me_from(7)
    );
    assert!(AvatarSpec::surprise_me_from(7).is_valid());

    // Persist the look on the agent: avatar JSON survives restart.
    let store = mem_store();
    let mut editor = good_editor("Ada");
    editor.avatar = json.clone();
    let row = save_editor(&store, &editor).expect("save with avatar");
    assert_eq!(row.avatar, json);
    let reloaded = store.get_agent(&row.id).expect("reload");
    assert_eq!(AvatarSpec::from_json(&reloaded.avatar), spec);
}

// --- 3. browse/import a licensed persona; link-only is rejected ---

#[test]
fn m03_persona_browse_import_and_link_only_rejected() {
    // Starter set works offline: browse, search, category filter, preview.
    let cache = LibraryCache::with_starter();
    let entries: Vec<synapse_personas::PersonaDraft> =
        cache.all_entries().into_iter().cloned().collect();
    assert!(
        entries.len() >= 8,
        "starter set must ship, got {}",
        entries.len()
    );
    let hits = search(&entries, "lead", None, None);
    assert!(!hits.is_empty(), "search must find the lead persona");
    let cats = synapse_app::persona_library::categories(&entries);
    assert!(!cats.is_empty());
    let preview = synapse_app::persona_library::preview(&entries, &hits[0].name);
    assert!(preview.is_some());
    assert!(!preview.expect("preview").instructions.is_empty());
    assert_eq!(cache.sha_for("synapse"), Some("bundled"));

    // Sources carry tarball URLs + license badges.
    let sources = builtin_sources();
    assert!(sources.len() >= 9);
    let agency = sources
        .iter()
        .find(|s| s.repo == "msitarzewski/agency-agents")
        .expect("agency source");
    assert_eq!(agency.license_badge(), "MIT");
    assert!(agency.importable(false));
    let link_only = sources
        .iter()
        .find(|s| s.repo == "contains-studio/agents")
        .expect("link-only source");
    assert!(!link_only.importable(false));
    assert!(!link_only.importable(true));

    // Import one licensed starter persona: repo/sha/license recorded.
    let store = mem_store();
    let licensed = entries[0].clone();
    assert_eq!(
        synapse_app::persona_library::license_badge(&licensed),
        "MIT"
    );
    assert!(synapse_app::persona_library::importable(&licensed, false));
    let row = import_draft(&store, &licensed, false).expect("licensed import");
    assert_eq!(row.name, licensed.name);
    assert!(
        row.persona_source.contains("synapse"),
        "source ref recorded: {}",
        row.persona_source
    );
    assert!(row.persona_source.contains("MIT"));

    // A link-only persona is rejected with a link to open instead.
    let link_draft = synapse_personas::PersonaDraft {
        name: "Linky".into(),
        description: "Link-only persona".into(),
        instructions: "# Linky\n\nYou help.".into(),
        role: None,
        category: None,
        color: None,
        emoji: None,
        vibe: None,
        model: None,
        tools: vec![],
        source: SourceMeta::new("contains-studio/agents", "HEAD", "", "linky.md"),
    };
    assert!(!synapse_app::persona_library::importable(&link_draft, true));
    match import_draft(&store, &link_draft, true).expect_err("link-only must fail") {
        LibraryError::LinkOnly(repo, url) => {
            assert_eq!(repo, "contains-studio/agents");
            assert!(!url.is_empty());
            assert_eq!(link_url(&link_draft), url);
        }
        other => panic!("expected LinkOnly, got {other}"),
    }

    // Request-only source (prompts.chat) needs explicit user confirmation.
    let request_draft = synapse_personas::PersonaDraft {
        name: "ActAs".into(),
        description: "Role prompt".into(),
        instructions: "# ActAs\n\nAct as a coach.".into(),
        role: None,
        category: None,
        color: None,
        emoji: None,
        vibe: None,
        model: None,
        tools: vec![],
        source: SourceMeta::new("f/prompts.chat", "HEAD", "", "act-as-coach.md"),
    };
    assert!(!synapse_app::persona_library::importable(
        &request_draft,
        false
    ));
    assert!(import_draft(&store, &request_draft, false).is_err());

    // URL + local-file imports stay offline via injected fetch/read.
    let url_row = import_from_url(
        &store,
        "https://example.com/coach.md",
        |_| Ok("---\nname: Coach\ndescription: Coaches.\n---\n\nYou coach kindly.".to_string()),
        false,
    )
    .expect("url import");
    assert_eq!(url_row.name, "Coach");
    let file_row = import_from_file(&store, std::path::Path::new("/tmp/coach.md"), |_| {
        Ok("# FileCoach\n\nYou coach from a file.".to_string())
    })
    .expect("file import");
    assert_eq!(file_row.name, "coach");

    // Markdown parsing records license + category from the path.
    let parsed = parse_persona_markdown(
        "---\nname: Growth\ndescription: Grows things.\n---\n\nYou grow startups.\n",
        SourceMeta::new(
            "msitarzewski/agency-agents",
            "sha1",
            "MIT",
            "strategy/growth.md",
        ),
    )
    .expect("parse");
    assert_eq!(parsed.category.as_deref(), Some("strategy"));
    assert_eq!(parsed.source.license, "MIT");
    let _ = starter_personas();
}

// --- 4. assemble a project team from a built-in template ---

#[test]
fn m03_project_team_assembled_from_builtin_template() {
    // Templates ship offline (research squad, brainstorm, investment, custom).
    let template = builtin_template("research-squad").expect("research squad exists");
    assert!(template.roles.len() >= 5);

    // Build one agent per template seat in core, then validate the team.
    let mut agents = AgentStore::new();
    let mut member_ids = Vec::new();
    for (i, seat) in template.roles.iter().enumerate() {
        let agent = agents
            .create(api_draft(
                &format!("{} {i}", seat.role),
                &seat.role,
                &seat.instructions,
                "openai",
                "gpt-4o",
            ))
            .expect("seat agent saves");
        member_ids.push(agent.id.clone());
    }
    let lead_id = member_ids[0].clone();
    let draft = ProjectDraft {
        name: "Market study".into(),
        goal: "Understand mobile money.".into(),
        folder: "/tmp/synapse-m03".into(),
        template: "research-squad".into(),
        lead_agent_id: lead_id.clone(),
        member_ids: member_ids.clone(),
    };
    let validated = synapse_core::validate_project(&draft, &agents).expect("team validates");
    assert_eq!(validated.template, "research-squad");
    assert_eq!(validated.member_ids.len(), template.roles.len());

    // Persist the project + memberships in the store; all seats attached.
    let store = mem_store();
    let project_id = synapse_store::new_id("project");
    let project = store
        .create_project(&synapse_store::NewProject {
            id: project_id.clone(),
            name: validated.name.clone(),
            goal: validated.goal.clone(),
            folder: validated.folder.clone(),
            lead_agent_id: lead_id.clone(),
            template: validated.template.clone(),
        })
        .expect("persist project");
    assert_eq!(project.template, "research-squad");

    // Persist each seat as an agent row and attach it to the project.
    for id in &member_ids {
        let src = agents.get(id).expect("seat exists");
        let backend = serde_json::to_string(&src.backend).expect("backend json");
        let row = store
            .create_agent(&synapse_store::NewAgent {
                id: synapse_store::new_id("agent"),
                name: src.name.clone(),
                role: src.role.clone(),
                instructions: src.instructions.clone(),
                backend,
                avatar: String::new(),
                tools: "[]".into(),
                permissions: "{}".into(),
                persona_source: String::new(),
            })
            .expect("persist seat");
        store
            .add_agent_to_project(&project_id, &row.id, "{}")
            .expect("attach seat");
    }
    let attached = store.list_project_agents(&project_id).expect("list team");
    assert_eq!(attached.len(), member_ids.len());
    assert!(attached
        .iter()
        .any(|a| a.role == "Lead" || a.name.contains("Lead")));

    // Core team edits: add/remove members around the persisted team.
    let mut projects = ProjectStore::new();
    // Mirror the persisted roster into fresh core stores for the edit flow.
    let mut core_agents = AgentStore::new();
    let mut core_ids = Vec::new();
    for a in &attached {
        let saved = core_agents
            .create(api_draft(
                &a.name,
                &a.role,
                &a.instructions,
                "openai",
                "gpt-4o",
            ))
            .expect("mirror agent");
        core_ids.push(saved.id.clone());
    }
    let team = projects
        .create(
            ProjectDraft {
                name: "Market study".into(),
                goal: "Understand mobile money.".into(),
                folder: "/tmp/synapse-m03".into(),
                template: "research-squad".into(),
                lead_agent_id: core_ids[0].clone(),
                member_ids: core_ids[..2].to_vec(),
            },
            &core_agents,
        )
        .expect("team created");
    assert_eq!(team.lead_agent_id, core_ids[0]);
    let grown = projects
        .add_member(&team.id, &core_ids[2], &core_agents)
        .expect("add member");
    assert_eq!(grown.member_ids.len(), 3);
    let switched = projects
        .set_lead(&team.id, &core_ids[1])
        .expect("switch lead");
    assert_eq!(switched.lead_agent_id, core_ids[1]);
    let shrunk = projects
        .remove_member(&team.id, &core_ids[2])
        .expect("remove member");
    assert!(!shrunk.member_ids.contains(&core_ids[2]));
}
