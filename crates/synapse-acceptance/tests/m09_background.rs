//! M9 background acceptance test (see docs/PRD.md §5.7).
//!
//! Offline integration through the real store, core and app APIs: window
//! close/reopen (hide, destroy, rebuild), tray menu/status/actions,
//! actionable notifications (delivery + click targets), configurable
//! quick-ask hotkey/window behavior, confirmed schedules + missed-run
//! handling, background settings persistence/platform behavior, and activity
//! controls. In-memory storage only; no OS services, keys, or network.

use synapse_app::activity::{ActivityFilter, ActivityView, ToolCallRow};
use synapse_app::background_settings::{
    AutoLaunchBackend as _, BackgroundSettings, BackgroundSettingsSection, LaunchState,
    MemoryAutoLaunch, PlatformSupport,
};
use synapse_app::lifecycle::{
    apply_tray_command, decide_launch, handle_second_launch, parse_tray_menu_item, tray_icon_state,
    tray_menu_labels, OpenEffect, TrayCommand, TrayEffect, TrayIconState, TrayRun, TraySnapshot,
    WindowLifecycle, WindowVisibility, FOCUS_MESSAGE,
};
use synapse_app::notifications::{
    delivery_for, NotificationDelivery, NotificationDestination, NotificationKind,
    NotificationTarget, NotificationsCenter,
};
use synapse_app::quick_ask::{
    hotkey_backend_for, HotkeyBackend, QuickAskHotkey, QuickAskSettings, QuickAskState,
    QuickAskWindow, DEFAULT_QUICK_ASK_HOTKEY,
};
use synapse_core::redact::Redactor;
use synapse_core::schedules::{preview_schedule, ScheduleStore};
use synapse_core::{RunMode, RunState};
use synapse_store::Store;

fn mem_store() -> Store {
    Store::open_in_memory().unwrap()
}

fn run(id: &str, members: &[&str]) -> RunState {
    RunState::new(
        id,
        RunMode::DeepResearch,
        members.iter().map(|s| s.to_string()).collect(),
        0,
    )
}

// ---------------------------------------------------------------------------
// 1. window close/reopen: hide, destroy-after-timeout, rebuild
// ---------------------------------------------------------------------------

#[test]
fn m09_window_close_hides_and_rebuilds_after_destroy() {
    let mut w = WindowLifecycle::new_visible();
    assert!(w.is_visible());

    // Close hides instead of quitting.
    assert!(w.close_window());
    assert_eq!(w.visibility(), WindowVisibility::Hidden);

    // Reopen while hidden restores cheaply.
    assert_eq!(w.open_window(), OpenEffect::Restored);
    assert!(w.is_visible());

    // Close again, sit hidden past the threshold: destroyed to free memory.
    w.close_window();
    assert!(!w.advance(WindowLifecycle::destroy_threshold_secs() - 1));
    assert_eq!(w.visibility(), WindowVisibility::Hidden);
    assert!(w.advance(1));
    assert_eq!(w.visibility(), WindowVisibility::Destroyed);

    // Reopen after destroy rebuilds from the store.
    assert_eq!(w.open_window(), OpenEffect::Rebuilt);
    assert!(w.is_visible());
    assert_eq!(w.open_window(), OpenEffect::Focus);

    // Second launch (single instance) focuses / rebuilds the window.
    let mut hidden = WindowLifecycle::new_visible();
    hidden.close_window();
    let (_, open) = handle_second_launch(&mut hidden, FOCUS_MESSAGE);
    assert_eq!(open, OpenEffect::Restored);
    assert!(hidden.is_visible());
    assert_eq!(
        decide_launch(false),
        synapse_app::lifecycle::LaunchOutcome::Primary
    );
    assert_eq!(
        decide_launch(true),
        synapse_app::lifecycle::LaunchOutcome::FocusExisting
    );
}

// ---------------------------------------------------------------------------
// 2. tray menu, icon status, and actions
// ---------------------------------------------------------------------------

#[test]
fn m09_tray_menu_status_and_actions() {
    // Idle: open + quit + quick-ask, empty-runs row, no pause-all.
    let idle = tray_menu_labels(&TraySnapshot::idle());
    for want in ["Open Synapse", "No active runs", "Quick Ask", "Quit"] {
        assert!(idle.contains(&want.to_string()), "missing {want}: {idle:?}");
    }
    assert_eq!(tray_icon_state(&TraySnapshot::idle()), TrayIconState::Idle);

    // Working snapshot: runs with phases, approvals count, pause-all.
    let snapshot = TraySnapshot {
        runs: vec![
            TrayRun::new("r1", "Market study", "Searching"),
            TrayRun::new("r2", "Uganda fintech", "Reading"),
        ],
        pending_approvals: 0,
        all_paused: false,
    };
    assert_eq!(tray_icon_state(&snapshot), TrayIconState::Working);
    let with_approvals = TraySnapshot {
        pending_approvals: 2,
        ..snapshot.clone()
    };
    let labels = tray_menu_labels(&with_approvals);
    assert!(labels
        .iter()
        .any(|l| l.contains("Market study") && l.contains("Searching")));
    assert!(labels
        .iter()
        .any(|l| l.contains("Pending approvals") && l.contains('2')));
    assert!(labels.contains(&"Pause All".to_string()));
    // Approvals take precedence over working.
    let needs_you = TraySnapshot {
        pending_approvals: 1,
        ..snapshot.clone()
    };
    assert_eq!(tray_icon_state(&needs_you), TrayIconState::NeedsYou);

    // Every actionable row parses back to a command.
    for label in &labels {
        assert!(
            parse_tray_menu_item(label).is_some(),
            "tray row has no command: {label}"
        );
    }
    assert_eq!(
        parse_tray_menu_item("Open Synapse"),
        Some(TrayCommand::Open)
    );
    assert_eq!(parse_tray_menu_item("Quit"), Some(TrayCommand::Quit));
    assert_eq!(
        parse_tray_menu_item("Quick Ask"),
        Some(TrayCommand::QuickAsk)
    );
    assert_eq!(
        parse_tray_menu_item("Pause All"),
        Some(TrayCommand::PauseAll)
    );
    assert_eq!(
        parse_tray_menu_item("Pending approvals (3)"),
        Some(TrayCommand::ShowApprovals)
    );

    // Commands drive the lifecycle: tray Open restores a hidden window,
    // Quit signals quit, pause-all/quick-ask pass through.
    let mut w = WindowLifecycle::new_visible();
    w.close_window();
    assert_eq!(
        apply_tray_command(&mut w, TrayCommand::Open),
        TrayEffect::Open(OpenEffect::Restored)
    );
    assert!(w.is_visible());
    assert_eq!(
        apply_tray_command(&mut w, TrayCommand::Quit),
        TrayEffect::Quit
    );
    assert!(TrayEffect::Quit.quits());
    assert_eq!(
        apply_tray_command(&mut w, TrayCommand::QuickAsk),
        TrayEffect::QuickAsk
    );
    assert_eq!(
        apply_tray_command(&mut w, TrayCommand::PauseAll),
        TrayEffect::PauseAll
    );
    assert_eq!(
        apply_tray_command(&mut w, TrayCommand::ShowApprovals),
        TrayEffect::ShowApprovals
    );
}

// ---------------------------------------------------------------------------
// 3. actionable notifications: delivery rules + click targets
// ---------------------------------------------------------------------------

#[test]
fn m09_actionable_notifications_delivery_and_targets() {
    let mut c = NotificationsCenter::new();
    c.set_now(1_000);
    c.set_visibility(WindowVisibility::Hidden);

    // One notification per event kind, each with a deep-link target.
    let approval = c.notify_approval("p1", "card1", "ada", "rm -rf /tmp/x");
    let done = c.notify_run_completed("p1", "run1", "Market study");
    let question = c.notify_agent_question("p1", "mira", "msg9", "Which source?");
    let err = c.notify_error("p2", Some("run2"), "provider timeout");
    assert_eq!(c.unread_count(), 4);

    // Focused window -> in-app toast; hidden/destroyed -> OS notification.
    let n = c.all().iter().find(|n| n.id == done).unwrap().clone();
    assert!(matches!(
        delivery_for(&n, WindowVisibility::Visible),
        NotificationDelivery::Toast(_)
    ));
    for vis in [WindowVisibility::Hidden, WindowVisibility::Destroyed] {
        assert!(
            matches!(delivery_for(&n, vis), NotificationDelivery::Os { .. }),
            "must be OS delivery when {vis:?}"
        );
    }
    let toast = c.surface(&done, WindowVisibility::Visible).expect("toast");
    assert!(matches!(toast, NotificationDelivery::Toast(_)));
    assert_eq!(c.toasts().len(), 1);

    // Clicks navigate to the right project/destination and mark read.
    assert_eq!(
        c.click(&approval).unwrap().destination,
        NotificationDestination::Approvals {
            card_id: Some("card1".into())
        }
    );
    assert_eq!(
        c.click(&done).unwrap(),
        NotificationTarget {
            project_id: "p1".into(),
            destination: NotificationDestination::Run {
                run_id: "run1".into()
            },
        }
    );
    assert_eq!(
        c.click(&question).unwrap().destination,
        NotificationDestination::Question {
            message_id: "msg9".into()
        }
    );
    assert_eq!(
        c.click(&err).unwrap().destination,
        NotificationDestination::ErrorDetail {
            run_id: Some("run2".into())
        }
    );
    assert_eq!(c.unread_count(), 0);
    assert!(c.click("missing").is_none());

    // Emit while focused queues a toast; toasts expire; badge clears.
    c.set_visibility(WindowVisibility::Visible);
    let (id, delivery) = c.emit(
        NotificationKind::AgentQuestion,
        "p1",
        "mira has a question",
        "Which angle?",
        NotificationTarget {
            project_id: "p1".into(),
            destination: NotificationDestination::Question {
                message_id: "m1".into(),
            },
        },
    );
    assert!(matches!(delivery, NotificationDelivery::Toast(_)));
    assert_eq!(c.badge(), "1");
    c.set_now(1_000 + synapse_app::notifications::TOAST_TTL_SECS + 1);
    assert_eq!(c.prune_toasts(), 1);
    assert!(c.mark_read(&id));
    c.mark_all_read();
    assert_eq!(c.badge(), "");
}

// ---------------------------------------------------------------------------
// 4. configurable quick-ask hotkey + compact window behavior
// ---------------------------------------------------------------------------

#[test]
fn m09_quick_ask_hotkey_and_window() {
    // Default combo parses and round-trips; bare keys are rejected.
    let default: QuickAskHotkey = DEFAULT_QUICK_ASK_HOTKEY.parse().expect("default parses");
    assert_eq!(default.display(), DEFAULT_QUICK_ASK_HOTKEY);
    assert!("Space".parse::<QuickAskHotkey>().is_err());
    assert!("Ctrl+Shift".parse::<QuickAskHotkey>().is_err());
    let custom: QuickAskHotkey = "Alt+Space".parse().expect("custom parses");
    assert_eq!(custom.display(), "Alt+Space");

    // Backend selection: native off-Wayland, portal on Wayland, else external.
    assert_eq!(hotkey_backend_for(false, false), HotkeyBackend::Native);
    assert_eq!(hotkey_backend_for(true, true), HotkeyBackend::Portal);
    assert_eq!(
        hotkey_backend_for(true, false),
        HotkeyBackend::ExternalBinding
    );

    // Window toggles on hotkey, destroys on blur.
    let mut w = QuickAskWindow::closed();
    assert!(w.on_hotkey_pressed());
    assert!(w.is_open());
    assert!(!w.on_hotkey_pressed());
    w.open();
    assert!(w.on_blur());
    assert!(!w.is_open());
    assert!(!w.on_blur());

    // Prefs persist (hotkey + last project); invalid hotkeys rejected.
    let store = mem_store();
    assert_eq!(
        QuickAskSettings::load(&store).hotkey,
        DEFAULT_QUICK_ASK_HOTKEY
    );
    let settings = QuickAskSettings {
        hotkey: "Alt+Space".to_string(),
        last_project_id: "p-last".to_string(),
    };
    settings.save(&store).unwrap();
    assert_eq!(QuickAskSettings::load(&store), settings);
    assert!(QuickAskSettings {
        hotkey: "Space".into(),
        last_project_id: String::new()
    }
    .save(&store)
    .is_err());

    // Compact ask: last-project fallback, per-frame streaming, open-in-app.
    let mut s = QuickAskState::new();
    assert!(!s.start("  ", "p1", "p2"));
    assert!(!s.start("hello", "", ""));
    assert!(s.start("hello lead", "", "p-last"));
    assert_eq!(s.project_id(), "p-last");
    s.push_delta("Hello world");
    assert!(s.flush_frame());
    assert_eq!(s.answer(), "Hello world");
    s.finish();
    assert!(!s.is_running());
    let effect = s.open_in_app().expect("open effect");
    assert_eq!(effect.project_id, "p-last");
    assert!(!effect.conversation_id.is_empty());
    assert!(QuickAskState::new().open_in_app().is_none());
}

// ---------------------------------------------------------------------------
// 5. confirmed schedules + missed-run handling (app closed)
// ---------------------------------------------------------------------------

#[test]
fn m09_confirmed_schedules_and_missed_runs() {
    let now: i64 = 1_790_000_000; // ~Oct 2026
    let mut schedules = ScheduleStore::new();

    // Preview parses natural language; staged rows never fire until confirmed.
    let preview = preview_schedule(
        "p1",
        "Morning news brief",
        "team_room",
        "weekdays at 9am",
        now,
    )
    .unwrap();
    assert_eq!(preview.cron_expr, "00 09 * * MON-FRI");
    assert!(!preview.summary().is_empty());
    let staged = schedules.propose(preview);
    assert!(!staged.confirmed);
    assert!(schedules.due(i64::MAX).is_empty());
    assert!(schedules.missed(i64::MAX, 60).is_empty());

    // Confirm recomputes next run; double-confirm rejected.
    let confirmed = schedules.confirm(&staged.id, now).unwrap();
    assert!(confirmed.confirmed);
    let next = confirmed.next_run_at.unwrap();
    assert!(next > now);
    assert!(schedules.confirm(&staged.id, now).is_err());

    // Runner sleeps until the next firing (no polling).
    let sleep = schedules.sleep_until(now).expect("scheduled");
    assert!(sleep.as_secs() > 0);

    // Far future: the run is due, and (past the grace window) counts as
    // missed while the app was closed, with a run-now action.
    let later = next + 3_600;
    assert!(!schedules.due(later).is_empty());
    let missed = schedules.missed(later, 60);
    assert_eq!(missed.len(), 1);
    assert_eq!(missed[0].schedule_id, staged.id);
    assert!(!missed[0].description.is_empty());
    schedules.run_now(&staged.id, later).unwrap();
    assert!(schedules.missed(later, 60).is_empty());
    let fired = schedules.get(&staged.id).unwrap();
    assert_eq!(fired.last_run_at, Some(later));
    assert!(fired.next_run_at.unwrap() > later);

    // Disabled schedules neither fire nor count as missed.
    schedules.set_enabled(&staged.id, false).unwrap();
    assert!(schedules.due(later).is_empty());
    assert!(schedules.missed(later, 60).is_empty());
    assert_eq!(schedules.sleep_until(later), None);
}

// ---------------------------------------------------------------------------
// 6. background settings persistence + platform behavior
// ---------------------------------------------------------------------------

#[test]
fn m09_background_settings_persist_and_reflect_platform() {
    let store = mem_store();

    // Defaults: tray-first app (keep running on close), no auto-launch.
    let loaded = BackgroundSettings::load(&store);
    assert_eq!(loaded, BackgroundSettings::default());
    assert!(loaded.keep_running_on_close);

    // Round-trip persists both toggles.
    let mut s = loaded;
    s.keep_running_on_close = false;
    s.launch_at_login = true;
    s.save(&store).unwrap();
    let again = BackgroundSettings::load(&store);
    assert!(!again.keep_running_on_close);
    assert!(again.launch_at_login);

    // Launch state surfaces OS/pref mismatch with a repair note.
    let state = LaunchState::probe(&again, &MemoryAutoLaunch::new(false));
    assert!(state.is_mismatched());
    assert!(state.mismatch_note().is_some());

    // Applying the pref drives the OS backend and re-probes into sync.
    let mut section =
        BackgroundSettingsSection::load(&store, &MemoryAutoLaunch::new(false), "linux");
    let mut backend = MemoryAutoLaunch::new(false);
    // Stored pref from above wants launch-on; applying enables the backend.
    synapse_app::background_settings::apply_launch_at_login(&section.settings, &mut backend)
        .unwrap();
    assert!(backend.is_enabled().unwrap());
    section
        .set_launch_at_login(&store, &mut backend, false)
        .unwrap();
    assert!(!backend.is_enabled().unwrap());
    assert!(!BackgroundSettings::load(&store).launch_at_login);

    // Menu-bar-only is macOS-only: disabled elsewhere with an explanation.
    let section = BackgroundSettingsSection::new(
        BackgroundSettings::default(),
        LaunchState {
            desired: false,
            effective: Some(false),
            error: None,
        },
        "linux",
    );
    let row = section
        .rows()
        .iter()
        .find(|r| r.title == "Menu bar only")
        .unwrap()
        .clone();
    assert!(!row.interactive);
    assert!(!row.toggled_on);
    assert!(row.footnote.unwrap_or_default().contains("macOS only"));
    assert!(!PlatformSupport::for_os("linux").menu_bar_only);
    assert!(PlatformSupport::for_os("macos").menu_bar_only);

    // Section rows always cover the three toggles.
    assert_eq!(section.rows().len(), 3);
}

// ---------------------------------------------------------------------------
// 7. activity controls: pause/resume/cancel/retry + scopes
// ---------------------------------------------------------------------------

#[test]
fn m09_activity_controls_and_scopes() {
    let mut view = ActivityView::new();
    view.add_run("p1", run("run-1", &["ada", "mira"]));
    assert_eq!(view.run_status("run-1"), Some("running"));

    // Each control returns the visible resulting status.
    assert_eq!(view.pause_run("run-1"), Some("paused"));
    assert_eq!(view.resume_run("run-1"), Some("running"));
    assert_eq!(view.cancel_run("run-1"), Some("cancelled"));
    assert_eq!(view.resume_run("run-1"), Some("cancelled"));
    assert_eq!(view.retry_run("run-1"), Some("running"));
    assert!(view.pause_run("missing").is_none());

    // Tool calls + approvals listed per project/agent scope.
    view.add_tool_call(ToolCallRow {
        id: "tc-1".into(),
        project_id: "p1".into(),
        agent_id: "ada".into(),
        tool: "shell".into(),
        status: "done".into(),
        result_summary: "exit 0".into(),
    });
    let redactor = Redactor::new();
    view.approvals_mut()
        .request("ada", "shell", r#"{"command": "cargo test"}"#, &redactor)
        .expect("card");
    assert_eq!(view.runs().len(), 1);
    assert_eq!(view.tool_calls().len(), 1);
    assert_eq!(view.pending_approvals().len(), 1);
    assert!(view.summary().contains("1 run"));

    view.set_filter(ActivityFilter {
        project_id: Some("p1".into()),
        agent_id: None,
    });
    assert_eq!(view.runs().len(), 1);
    view.set_filter(ActivityFilter {
        project_id: None,
        agent_id: Some("mira".into()),
    });
    assert_eq!(view.runs().len(), 1);
    assert!(view.pending_approvals().is_empty()); // card belongs to ada
    view.set_filter(ActivityFilter {
        project_id: Some("nope".into()),
        agent_id: None,
    });
    assert!(view.runs().is_empty()); // empty state, no break
}
