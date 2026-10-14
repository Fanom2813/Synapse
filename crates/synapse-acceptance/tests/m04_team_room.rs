//! M4 team-room acceptance test (see docs/MILESTONES.md §M4).
//!
//! Offline end-to-end through the real crates: a team exchange over the
//! bus and typed tools (say, addressed ask/answer, challenge), a worker
//! driven by a scripted backend, and the room reflecting streaming,
//! interjection and pause/resume/stop controls. No network, no credentials.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use synapse_app::team_room::{RoomAgentState, TeamRoomState};
use synapse_core::{
    can_interrupt, can_steer, drain_safe_point, run_worker, Envelope, Kind, Participant, Priority,
    Recipients, RunMode, RunStatus, TeamBus, TeamTools, WorkerBackend, WorkerConfig,
};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// scripted backend for worker
// ---------------------------------------------------------------------------

struct ScriptBackend {
    outputs: Mutex<VecDeque<Option<String>>>,
    seen: Mutex<Vec<String>>,
}

impl ScriptBackend {
    fn with(outputs: Vec<Option<String>>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
            seen: Mutex::new(Vec::new()),
        }
    }
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl WorkerBackend for ScriptBackend {
    async fn generate(&self, prompt: String, cancel: CancellationToken) -> Option<String> {
        self.seen.lock().unwrap().push(prompt);
        let next = self
            .outputs
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Some("DONE: ok".into()));
        tokio::select! {
            _ = cancel.cancelled() => None,
            _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => next,
        }
    }
}

// ---------------------------------------------------------------------------
// 1. team exchange through the bus and typed tools
// ---------------------------------------------------------------------------

#[test]
fn m04_team_exchange_say_ask_answer_challenge() {
    let bus = TeamBus::new();
    bus.set_lead("lead");
    let mut room_rx = bus.subscribe();
    let mut lead_inbox = bus.agent_inbox("lead");
    let mut mira_inbox = bus.agent_inbox("mira");

    let lead_tools = TeamTools::for_agent(&bus, "lead", "run-m04", "c-m04");
    let mira_tools = TeamTools::for_agent(&bus, "mira", "run-m04", "c-m04");
    let user_tools = TeamTools::for_user(&bus, "run-m04", "c-m04");

    // say → room
    let hello = lead_tools.say("Hello team, let's research X.").unwrap();
    assert!(matches!(hello.kind, Kind::Say));
    assert_eq!(hello.to, Recipients::Room);
    assert_eq!(hello.seq, 1);
    // History is persist-then-publish: available before broadcast is drained.
    let hist = bus.history("c-m04", 0, 10);
    assert_eq!(hist.len(), 1);
    assert_eq!(hist[0].id, hello.id);

    // Addressed request: lead asks mira
    let ask = lead_tools
        .ask_agent("mira", "Can you summarise the paper?")
        .unwrap();
    assert!(matches!(ask.kind, Kind::Ask { .. }));
    assert_eq!(ask.to, Recipients::Agent("mira".into()));
    // mira's inbox got it, lead's did not
    assert_eq!(mira_inbox.try_recv().unwrap().id, hello.id);
    assert_eq!(mira_inbox.try_recv().unwrap().id, ask.id);
    // lead inbox only got the room say
    assert_eq!(lead_inbox.try_recv().unwrap().id, hello.id);
    assert!(lead_inbox.try_recv().is_err()); // Ask only to mira, not lead

    // Mira replies via answer (routes back to asker)
    let ans = mira_tools.answer(&ask.id, "It shows X causes Y.").unwrap();
    assert!(matches!(ans.kind, Kind::Answer));
    assert_eq!(ans.reply_to.as_deref(), Some(ask.id.as_str()));
    // answer routes to the original sender (lead)
    assert_eq!(ans.to, Recipients::Agent("lead".into()));

    // Challenge the answer (room-visible)
    let ch = lead_tools
        .challenge(&ans.id, "Figure 2 contradicts that claim.")
        .unwrap();
    assert!(matches!(ch.kind, Kind::Challenge { .. }));
    assert_eq!(ch.to, Recipients::Room);
    assert_eq!(ch.reply_to.as_deref(), Some(ans.id.as_str()));

    // User interjection via typed tool dispatch as well
    let via_call = user_tools
        .call(
            "say",
            serde_json::json!({"text": "Please stay focused on peer-reviewed sources."}),
        )
        .unwrap();
    assert!(matches!(via_call.kind, Kind::Say));
    assert_eq!(via_call.from, Participant::User);

    // All envelopes persisted in order
    let all = bus.history("c-m04", 0, 100);
    assert_eq!(all.len(), 5);
    assert!(all.windows(2).all(|w| w[0].seq < w[1].seq));

    // Broadcast observers saw them
    let mut msgs = Vec::new();
    while let Ok(ev) = room_rx.try_recv() {
        if let synapse_core::Event::Message(e) = ev {
            msgs.push(e.id.clone());
        }
    }
    assert_eq!(msgs.len(), 5);

    // Invalid tool calls store nothing
    let before = bus.latest_seq("c-m04");
    assert!(lead_tools
        .call("ask_agent", serde_json::json!({"agent":"mira"}))
        .is_err());
    assert_eq!(bus.latest_seq("c-m04"), before);

    // Lag resync: a slow observer replays missed envelopes from the store
    let missed_direct = bus.history("c-m04", 1, 100);
    assert_eq!(missed_direct.len(), 4);
    assert_eq!(missed_direct[0].seq, 2);
    let mut slow = bus.subscribe();
    let (_buffered, missed) = bus.resync(&mut slow, "c-m04", 1);
    assert_eq!(missed.len(), 4);
    assert_eq!(missed[0].seq, 2);

    // Permissions helpers: user and lead may steer/interrupt as per protocol
    assert!(can_steer(&Participant::User, false, false));
    assert!(can_steer(&Participant::Agent("lead".into()), true, false));
    assert!(!can_interrupt(&Participant::Agent("mira".into()), false));
    assert!(can_interrupt(&Participant::User, false));

    // Typed specs cover say
    let specs = synapse_core::team_tools::team_tool_specs();
    assert!(specs.iter().any(|s| s.name == "say"));
}

// ---------------------------------------------------------------------------
// 2. worker driven by scripted backend (safe-point inbox, steering)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn m04_worker_scripted_backend_with_steering_and_interrupt() {
    let bus = TeamBus::new();
    bus.set_lead("lead");
    let mut inbox = bus.agent_inbox("mira");
    let mut cfg = WorkerConfig::new("mira", "run-m04", "c-m04");
    cfg.persona = "You are Mira, a researcher.".into();
    cfg.goal = "Summarise X.".into();
    cfg.plan = "Read then write.".into();
    cfg.phase = "Research".into();
    cfg.lead_agent = Some("lead".into());
    cfg.max_turns = 3;

    // Worker parks until woken; wake it with an addressed message.
    let backend = Arc::new(ScriptBackend::with(vec![
        Some("working on X...".into()),
        Some("DONE: summarised X".into()),
    ]));
    let wake = Envelope::say(
        "wake-1",
        "c-m04",
        Participant::User,
        Recipients::Agent("mira".into()),
        "please research X",
    );
    let mut wake = wake;
    wake.run_id = "run-m04".into();
    bus.publish(wake).unwrap();

    // Steer arrives while first turn runs: lead steer should be picked up at safe point.
    let bus2 = bus.clone();
    let backend2 = backend.clone();
    // Need to spawn worker then send steer shortly after start.
    let handle = tokio::spawn({
        let b = bus.clone();
        async move { run_worker(&b, &mut inbox, &*backend2, &cfg).await }
    });
    // Give worker a moment to enter first generate, then steer.
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let mut steer = Envelope::say(
        "steer-1",
        "c-m04",
        Participant::Agent("lead".into()),
        Recipients::Agent("mira".into()),
        "only peer-reviewed sources",
    );
    steer.run_id = "run-m04".into();
    steer.priority = Priority::Steer;
    bus2.publish(steer).unwrap();

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
        .await
        .expect("worker must not hang")
        .unwrap();
    assert_eq!(outcome.done.as_deref(), Some("summarised X"));
    assert_eq!(outcome.steers_seen, 1);
    // Second prompt contained the steered instruction.
    let seen = backend.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen[1].contains("peer-reviewed"), "got: {}", seen[1]);
    // Steer ack published as Status.
    let hist = bus.history("c-m04", 0, 100);
    assert!(hist.iter().any(|e| matches!(e.kind, Kind::Status { .. })));

    // Interrupt path: pre-queued interrupt cancels in-flight call, draft kept.
    let bus3 = TeamBus::new();
    let mut inbox3 = bus3.agent_inbox("mira");
    let mut cfg3 = WorkerConfig::new("mira", "run-m04", "c-m04b");
    cfg3.max_turns = 2;
    let backend3 = ScriptBackend::with(vec![Some("DONE: after interrupt".into())]);
    let mut first = Envelope::say(
        "wake-b",
        "c-m04b",
        Participant::User,
        Recipients::Agent("mira".into()),
        "long task",
    );
    first.run_id = "run-m04".into();
    let mut intr = Envelope::say(
        "intr-1",
        "c-m04b",
        Participant::User,
        Recipients::Agent("mira".into()),
        "stop, do Y instead",
    );
    intr.run_id = "run-m04".into();
    intr.priority = Priority::Interrupt;
    bus3.publish(first).unwrap();
    bus3.publish(intr).unwrap();
    let out2 = run_worker(&bus3, &mut inbox3, &backend3, &cfg3).await;
    assert!(out2.interrupts_seen >= 1);
    assert!(!out2.draft.is_empty());
    assert_eq!(out2.done.as_deref(), Some("after interrupt"));

    // drain_safe_point helper is non-blocking and returns queued items.
    let bus4 = TeamBus::new();
    let mut inbox4 = bus4.agent_inbox("mira");
    let mut e1 = Envelope::say(
        "d1",
        "c-drain",
        Participant::User,
        Recipients::Agent("mira".into()),
        "hello",
    );
    e1.run_id = "r".into();
    e1.conversation_id = "c-drain".into();
    bus4.publish(e1).unwrap();
    let drained = drain_safe_point(&mut inbox4);
    assert_eq!(drained.len(), 1);
    assert!(drain_safe_point(&mut inbox4).is_empty());
}

// ---------------------------------------------------------------------------
// 3. room reflects streaming/interjection and pause/resume/stop
// ---------------------------------------------------------------------------

#[test]
fn m04_room_streaming_interjection_and_controls() {
    let mut room = TeamRoomState::new(
        "c-m04",
        RunMode::Brainstorm,
        vec![
            ("lead".into(), "Lead".into()),
            ("mira".into(), "Researcher".into()),
            ("ivan".into(), "Critic".into()),
        ],
    );

    // Empty state until messages arrive.
    assert!(room.is_empty());
    assert!(room.empty_state().is_some());
    assert_eq!(room.run_status(), RunStatus::Running);
    assert_eq!(room.send_label(), "Stop");

    // Streaming: begin, coalesce deltas per frame, finish.
    let stream_id = room.begin_stream("s1", Participant::Agent("mira".into()), Recipients::Room);
    assert_eq!(room.member_state("mira"), Some(RoomAgentState::Speaking));
    for d in ["Hello", " ", "team", "!"] {
        room.push_delta(&stream_id, d);
    }
    // Buffered until flush.
    assert_eq!(
        room.messages()
            .iter()
            .find(|m| m.envelope.id == stream_id)
            .unwrap()
            .envelope
            .body,
        ""
    );
    let changed = room.flush_frame();
    assert_eq!(changed, vec![stream_id.clone()]);
    assert!(room.flush_frame().is_empty());
    // Finish replaces placeholder and idles the agent.
    let mut final_env = Envelope::say(
        &stream_id,
        "c-m04",
        Participant::Agent("mira".into()),
        Recipients::Room,
        "Hello team!",
    );
    final_env.run_id = "run-1".into();
    room.finish_stream(final_env);
    assert!(room.messages().iter().all(|m| !m.streaming));
    assert_eq!(room.member_state("mira"), Some(RoomAgentState::Idle));
    assert!(room.empty_state().is_none());

    // Interjection posts immediately and routes @mentions to selector.
    let (uid, mentioned) = room.interject("@mira hold on, new direction");
    assert_eq!(mentioned, vec!["mira".to_string()]);
    assert!(room.messages().iter().any(|m| m.envelope.id == uid));
    // Direct push of envelopes from the bus also updates room (typed styles).
    let bus = TeamBus::new();
    let bus_env = {
        let tools = TeamTools::for_agent(&bus, "ivan", "run-1", "c-m04");
        tools.say("A quick note from Ivan.").unwrap()
    };
    room.push_envelope(bus_env.clone());
    assert_eq!(room.messages().last().unwrap().style(), "say");
    // Challenge edge.
    let mut ch = Envelope::say(
        "ch1",
        "c-m04",
        Participant::Agent("ivan".into()),
        Recipients::Room,
        "I challenge the claim.",
    );
    ch.run_id = "run-1".into();
    ch.kind = Kind::Challenge {
        claim_ref: stream_id.clone(),
    };
    ch.reply_to = Some(stream_id.clone());
    room.push_envelope(ch);
    assert_eq!(room.replies_to(&stream_id).len(), 1);
    assert!(room.parent_of("ch1").is_some());

    // Pause: running speakers become paused, interjection queues follow-up.
    room.set_agent_state("mira", RoomAgentState::Working);
    room.pause_run();
    assert_eq!(room.run_status(), RunStatus::Paused);
    assert_eq!(room.member_state("mira"), Some(RoomAgentState::Paused));
    let before = room.messages().len();
    room.interject("wait, one more thing while paused");
    assert_eq!(room.messages().len(), before + 1);
    assert_eq!(room.queued_followups().len(), 1);
    assert_eq!(room.send_label(), "Send");

    // Resume idles paused agents.
    room.resume_run();
    assert_eq!(room.run_status(), RunStatus::Running);
    assert_eq!(room.member_state("mira"), Some(RoomAgentState::Idle));
    assert_eq!(room.send_label(), "Stop");

    // Stop clears streaming placeholders and idles agents.
    let sid = room.begin_stream("s2", Participant::Agent("ivan".into()), Recipients::Room);
    room.push_delta(&sid, "partial");
    room.stop_run();
    assert_eq!(room.run_status(), RunStatus::Stopped);
    assert!(room.messages().iter().all(|m| !m.streaming));
    for id in ["lead", "mira", "ivan"] {
        assert_eq!(room.member_state(id), Some(RoomAgentState::Idle));
    }

    // Detail levels filter to summary, budget and membership controls.
    room.set_budget(5.0);
    assert_eq!(room.run().limits.budget_usd, 5.0);
    room.add_member("zoe", "Writer");
    assert!(room.run().members.contains(&"zoe".to_string()));
    room.remove_member("zoe");
    assert!(!room.run().members.contains(&"zoe".to_string()));
    // Conversation map reflects members and edges.
    let map = room.conversation_map();
    assert!(!map.nodes().is_empty());
}
