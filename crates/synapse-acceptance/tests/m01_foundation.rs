//! M1 foundation acceptance test (see docs/MILESTONES.md §M1).
//!
//! Drives the real workspace crates through the foundation flows with no
//! network and no window: store migrations + CRUD + FTS, settings + theme,
//! secret redactor, tokio bridge, and app lifecycle (hide-to-tray,
//! single instance). Fails if the workspace skeleton is not wired together.

use std::path::Path;
use std::time::Duration;

use synapse_app::lifecycle::{
    apply_tray_command, decide_launch, parse_tray_menu_item, tray_menu_items, LaunchOutcome,
    OpenEffect, QuitModeExplicit, TrayCommand, TrayEffect, WindowLifecycle, WindowVisibility,
};
use synapse_app::runtime;
use synapse_app::settings::AppSettings;
use synapse_core::{is_denied_path, Redactor};

// --- store: migrations run, CRUD works, FTS finds messages ---

#[test]
fn store_migrates_and_persists_projects_agents_messages() {
    let store = synapse_store::Store::open_in_memory().expect("open in-memory store");

    store.setting_set("theme", "dark").expect("write setting");
    assert_eq!(
        store.setting_get("theme").expect("read setting").as_deref(),
        Some("dark")
    );

    let project = store
        .create_project(&synapse_store::NewProject {
            id: "m01-p1".into(),
            name: "Market study".into(),
            goal: "Understand fintech".into(),
            folder: "/tmp/synapse-m01".into(),
            lead_agent_id: String::new(),
            template: "research".into(),
        })
        .expect("create project");
    assert_eq!(project.name, "Market study");

    let agent = store
        .create_agent(&synapse_store::NewAgent {
            id: "m01-a1".into(),
            name: "Ada".into(),
            role: "lead".into(),
            instructions: "Be helpful".into(),
            backend: r#"{"type":"api"}"#.into(),
            avatar: "{}".into(),
            tools: "[]".into(),
            permissions: "{}".into(),
            persona_source: String::new(),
        })
        .expect("create agent");
    assert_eq!(agent.name, "Ada");
    store
        .add_agent_to_project("m01-p1", "m01-a1", "{}")
        .expect("add agent to project");
    assert_eq!(store.list_project_agents("m01-p1").expect("list").len(), 1);

    let conversation = store
        .create_conversation("m01-c1", "m01-p1", "team_room", "Kickoff")
        .expect("create conversation");
    assert_eq!(conversation.id, "m01-c1");

    store
        .insert_message(&synapse_store::NewMessage {
            id: "m01-m1".into(),
            conversation_id: "m01-c1".into(),
            run_id: String::new(),
            from_id: "user".into(),
            to_json: "[]".into(),
            kind: "ask".into(),
            reply_to: String::new(),
            thread_id: String::new(),
            body: "What are mobile-money fees in Kampala?".into(),
            meta: "{}".into(),
        })
        .expect("insert message");

    let hits = store.search_messages("Kampala", 10).expect("FTS search");
    assert!(
        hits.iter().any(|m| m.id == "m01-m1"),
        "FTS5 must find the inserted message, got {hits:?}"
    );
}

// --- settings + theme round-trip through the store ---

#[test]
fn settings_theme_persists_and_cycles() {
    let store = synapse_store::Store::open_in_memory().expect("open store");
    let mut settings = AppSettings::load(&store).expect("load defaults");
    settings.theme = synapse_app::settings::Theme::Dark;
    settings.save(&store).expect("save settings");
    let reloaded = AppSettings::load(&store).expect("reload settings");
    assert_eq!(reloaded.theme, synapse_app::settings::Theme::Dark);
    let mut cycled = reloaded;
    cycled.cycle_theme();
    assert_ne!(cycled.theme, synapse_app::settings::Theme::Dark);
}

// --- secret redactor is on: known values, patterns, deny paths ---

#[test]
fn redactor_scrubs_known_values_patterns_and_deny_paths() {
    let mut redactor = Redactor::new();
    redactor.add_secret("OPENAI_KEY", "sk-test-value-123");
    let out = redactor.redact("key is sk-test-value-123 ok");
    assert!(
        !out.text.contains("sk-test-value-123"),
        "known value leaked"
    );
    assert_eq!(out.hits, vec!["OPENAI_KEY".to_string()]);
    assert_eq!(out.chip(), "1 secret hidden");

    // gitleaks-style pattern: a GitHub token must not pass through.
    let pat = Redactor::new().redact("token ghp_0123456789abcdefghij0123456789ab");
    assert!(
        !pat.text.contains("ghp_0123456789abcdefghij0123456789ab"),
        "pattern value leaked: {}",
        pat.text
    );
    assert!(pat.count > 0);

    assert!(is_denied_path(Path::new("/home/u/proj/.env")));
    assert!(is_denied_path(Path::new("/home/u/.ssh/id_rsa")));
    assert!(!is_denied_path(Path::new("/home/u/proj/notes.md")));
}

// --- tokio bridge: work runs off the calling thread, cancel works ---

#[test]
fn tokio_bridge_runs_work_and_cancels() {
    let answer = runtime::spawn(async { 6 * 7 }).block_on().expect("join");
    assert_eq!(answer, 42);

    // Cancelling aborts the work: awaiting it reports cancellation.
    let mut task = runtime::spawn_cancellable(|token| async move {
        token.cancelled().await;
        "cancelled"
    });
    task.cancel();
    let err = task.block_on().expect_err("cancelled task must not join");
    assert_eq!(err, runtime::BridgeError::Cancelled);

    // Firing the token (without aborting) lets the task finish normally.
    let task = runtime::spawn_cancellable(|token| async move {
        token.cancelled().await;
        "cancelled"
    });
    task.cancellation_token().cancel();
    let out = task.block_on().expect("token-fired task joins");
    assert_eq!(out, "cancelled");

    // spawn_blocking keeps blocking disk work off the async workers.
    let sum = runtime::spawn_blocking(|| (1..=100).sum::<u64>())
        .block_on()
        .expect("blocking join");
    assert_eq!(sum, 5050);
    assert_eq!(runtime::worker_threads(), 2);
}

// --- lifecycle: explicit quit, hide-to-tray, destroy+rebuild, tray, single instance ---

#[test]
fn lifecycle_hides_destroys_and_rebuilds_without_quitting() {
    assert!(!QuitModeExplicit::should_quit_on_last_window_closed());

    let mut life = WindowLifecycle::new_visible();
    assert!(life.is_visible());
    // Close hides the window; the app keeps running.
    assert!(life.close_window());
    assert!(!life.is_visible());

    // After the 60 s hidden threshold the window is destroyed (frees GPU).
    assert!(!life.advance(59));
    assert_eq!(life.visibility(), WindowVisibility::Hidden);
    assert!(life.advance(1));
    assert_eq!(life.visibility(), WindowVisibility::Destroyed);

    // Reopening rebuilds from the store.
    let effect = life.open_window();
    assert!(matches!(effect, OpenEffect::Rebuilt));
    assert!(life.is_visible());
}

#[test]
fn tray_menu_and_single_instance_focus_first() {
    let items = tray_menu_items();
    assert!(items.iter().any(|i| i == "Open Synapse") && items.iter().any(|i| i == "Quit"));

    let mut life = WindowLifecycle::new_visible();
    life.close_window();
    let effect = apply_tray_command(&mut life, TrayCommand::Open);
    assert!(matches!(effect, TrayEffect::Open(OpenEffect::Restored)));
    assert!(life.is_visible());
    assert_eq!(parse_tray_menu_item("Quit"), Some(TrayCommand::Quit));

    // Second launch focuses the first instead of opening a second copy.
    assert!(matches!(decide_launch(true), LaunchOutcome::FocusExisting));
    assert!(matches!(decide_launch(false), LaunchOutcome::Primary));
}

// --- app dirs + logging bootstrap without touching the real home ---

#[test]
fn app_dirs_resolve_under_tmp_and_logging_starts() {
    let dir = synapse_app::app_dirs::data_dir();
    assert!(dir.to_string_lossy().contains("synapse"));

    let tmp = std::env::temp_dir().join("synapse-m01-dirs");
    std::fs::create_dir_all(&tmp).expect("tmp dir");
    let _guard = synapse_app::logging::init_logging(&tmp).expect("logging starts in a temp dir");
    synapse_app::logging::log_info_redacted(
        "hello sk-test-value-123",
        &[("K", "sk-test-value-123")],
    );
    // Give the non-blocking writer a beat to flush without sleeping long.
    std::thread::sleep(Duration::from_millis(50));
}
