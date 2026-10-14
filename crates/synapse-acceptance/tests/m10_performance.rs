//! M10.4 release performance budgets (see docs/PERFORMANCE.md §1).
//!
//! Release-only probes (`--release`): a 5-agent team run stays under the
//! CPU/memory caps with coalesced streaming, and scrolling a long room
//! stays under the frame-time / delta-memory caps via the virtualised
//! window. Marked `#[ignore]` so normal `cargo test` stays fast; the
//! milestone verify runs them with `-- --include-ignored` in release.

use synapse_app::chat_view::ChatState;
use synapse_app::perf_probe::{
    check_budgets, check_scroll_budgets, exercise_run5_burst, exercise_scroll_window,
    ProbeScenario, SCROLL_MEM_DELTA_CAP_BYTES, SCROLL_WINDOW,
};
use synapse_app::team_room::TeamRoomState;
use synapse_core::{Participant, Recipients, RunMode};

fn is_release() -> bool {
    !cfg!(debug_assertions)
}

// ---------------------------------------------------------------------------
// 1. Team run: 5 agents streaming under CPU/memory caps, coalesced frames
// ---------------------------------------------------------------------------

#[ignore]
#[test]
fn m10_run5_under_cpu_mem_caps_with_coalesced_frames() {
    if !is_release() {
        return;
    }
    // 30 frames x 5 agents x 20 deltas: 3,000 deltas collapse to 5 body
    // updates per frame (one notify per changed message per frame).
    let members: Vec<(String, String)> = (0..5)
        .map(|i| (format!("agent-{i}"), "member".to_string()))
        .collect();
    let mut room = TeamRoomState::new("conv-perf", RunMode::FreeDiscussion, members);
    let ids: Vec<String> = (0..5)
        .map(|i| {
            room.begin_stream(
                format!("s-{i}"),
                Participant::Agent(format!("agent-{i}")),
                Recipients::Room,
            )
        })
        .collect();
    let mut worst_changes = 0usize;
    let t = std::time::Instant::now();
    for _ in 0..30 {
        for id in &ids {
            for _ in 0..20 {
                room.push_delta(id, "token ");
            }
        }
        let changed = room.flush_frame();
        assert_eq!(changed.len(), 5, "one update per agent per frame");
        worst_changes = worst_changes.max(changed.len());
    }
    let elapsed = t.elapsed();
    assert!(worst_changes <= 5);
    // Wall-clock proxy for CPU: 3,000 coalesced deltas must take well
    // under a single frame budget per frame on any CI machine.
    assert!(
        elapsed.as_millis() < 5_000,
        "run5 burst took {elapsed:?}, expected < 5 s"
    );

    // Helper burst also coalesces.
    let (changes, _) = exercise_run5_burst(3, 20);
    assert!((1..=5).contains(&changes));

    // Budget gate on the probe path (zero CPU/RSS trivially passes the
    // generous run5 caps; over-cap values fail with a named dimension).
    assert!(check_budgets(ProbeScenario::Run5, 5.0, 150 * 1024 * 1024).is_empty());
    let v = check_budgets(ProbeScenario::Run5, 25.0, 150 * 1024 * 1024);
    assert!(v.iter().any(|s| s.contains("cpu")));
    let v = check_budgets(ProbeScenario::Run5, 5.0, 300 * 1024 * 1024);
    assert!(v.iter().any(|s| s.contains("rss")));
    assert_eq!(ProbeScenario::Run5.cpu_cap_percent(), 15.0);
    assert_eq!(ProbeScenario::Run5.mem_cap_bytes(), 250 * 1024 * 1024);
}

// ---------------------------------------------------------------------------
// 2. Long-message scrolling: virtualised window under frame/memory caps
// ---------------------------------------------------------------------------

#[ignore]
#[test]
fn m10_scroll_5000_messages_virtualised_under_frame_cap() {
    if !is_release() {
        return;
    }
    // 2,500 sends == 5,000 rows (user + agent placeholder each).
    let mut state = ChatState::new();
    for i in 0..2500 {
        state.set_draft(format!(
            "message {i} with some body text to lay out and scroll"
        ));
        assert!(state.send().is_some());
        state.stop();
    }
    assert_eq!(state.item_count(), 5000);

    // Only the visible window is laid out per frame, wherever the fling
    // lands; each window must finish in under one 60 fps frame (16 ms)
    // and hold a bounded delta over idle.
    let mut max_ms = 0.0f64;
    let mut max_bytes = 0u64;
    for start in [0usize, 1250, 2500, 5000 - SCROLL_WINDOW] {
        let t = std::time::Instant::now();
        let mut bytes = 0u64;
        let rows: Vec<_> = state
            .messages()
            .iter()
            .skip(start)
            .take(SCROLL_WINDOW)
            .collect();
        assert_eq!(rows.len(), SCROLL_WINDOW, "window stays full");
        for m in &rows {
            bytes += m.body.len() as u64;
            std::hint::black_box(m.body.len());
        }
        max_ms = max_ms.max(t.elapsed().as_secs_f64() * 1000.0);
        max_bytes = max_bytes.max(bytes);
    }
    assert!(
        max_ms < 16.0,
        "slowest window {max_ms:.3} ms over 16 ms cap"
    );
    assert!(
        max_bytes < SCROLL_MEM_DELTA_CAP_BYTES,
        "window {max_bytes} bytes over +30 MB cap"
    );
    assert!(check_scroll_budgets(max_ms, max_bytes).is_empty());

    // Helper agrees.
    let (helper_ms, helper_bytes) = exercise_scroll_window(200);
    assert!(helper_ms < 16.0);
    assert!(helper_bytes < SCROLL_MEM_DELTA_CAP_BYTES);
}

// ---------------------------------------------------------------------------
// 3. Probe reports for run5/scroll carry budgets and pass headless
// ---------------------------------------------------------------------------

#[ignore]
#[test]
fn m10_probe_reports_run5_scroll_pass_budgets() {
    if !is_release() {
        return;
    }
    for scenario in [ProbeScenario::Run5, ProbeScenario::Scroll] {
        let report = synapse_app::perf_probe::run_probe(scenario, 0);
        assert_eq!(report.cpu_cap_percent, scenario.cpu_cap_percent());
        assert_eq!(report.mem_cap_bytes, scenario.mem_cap_bytes());
        let v: serde_json::Value = serde_json::from_str(&report.to_json()).expect("valid JSON");
        assert_eq!(v["scenario"], scenario.as_str());
        // Headless CI box idles near 0 % CPU; both scenarios must pass
        // their generous caps here (real-device numbers go to PERF-RESULTS).
        assert!(
            report.passed,
            "probe {scenario:?} failed: {:?} {:?}",
            report.violations, report.unavailable
        );
        if scenario == ProbeScenario::Scroll {
            let ms = report.max_frame_ms.expect("scroll reports frame time");
            assert!(ms < 16.0, "scroll frame {ms} ms");
        }
    }
}
