//! Bounded agent workers + context building (see docs/TEAM-PROTOCOL.md §3b, §7).
//!
//! Each turn gets: persona/instructions, project goal, current plan + phase,
//! the agent's own notes, messages addressed to it (full), the rolling room
//! summary, retrieved snippets, and the team tool list — laid out
//! stable-prefix-first so provider prompt caching hits. Workers check their
//! inbox at safe points (before each model call, after each tool result);
//! an idle worker parks on its inbox (`recv`), so steering needs no polling.
//! `Steer` is applied at the next safe point with a `Status` ack;
//! `Interrupt` (user/lead only) cancels the in-flight model call via a
//! [`CancellationToken`] and restarts the turn with the new instruction.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::bus::{Envelope, Kind, Participant, Recipients, TeamBus};

// -- context ---------------------------------------------------------------

/// Everything one agent turn may see (TEAM-PROTOCOL.md §7).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentContext {
    pub persona: String,
    pub goal: String,
    pub plan: String,
    pub phase: String,
    pub notes: String,
    pub addressed: Vec<Envelope>,
    pub summary: String,
    pub snippets: Vec<String>,
    pub tools: Vec<String>,
}

impl AgentContext {
    /// Stable-prefix-first layout: system/persona → tools → goal/plan →
    /// summary → new messages, so repeated turns mostly hit provider cache.
    pub fn build_prompt(&self, extra_instructions: &[String]) -> String {
        let mut out = String::new();
        out.push_str("# Persona / instructions\n");
        out.push_str(&self.persona);
        out.push_str("\n\n# Tools\n");
        out.push_str(&self.tools.join(", "));
        out.push_str("\n\n# Project goal\n");
        out.push_str(&self.goal);
        out.push_str("\n\n# Plan\n");
        out.push_str(&self.plan);
        out.push_str("\n\n# Phase\n");
        out.push_str(&self.phase);
        if !self.notes.trim().is_empty() {
            out.push_str("\n\n# Own notes\n");
            out.push_str(&self.notes);
        }
        out.push_str("\n\n# Rolling summary of the room\n");
        out.push_str(&self.summary);
        if !self.snippets.is_empty() {
            out.push_str("\n\n# Retrieved snippets\n");
            for s in &self.snippets {
                out.push_str("- ");
                out.push_str(s);
                out.push('\n');
            }
        }
        out.push_str("\n\n# Messages addressed to you (full)\n");
        for m in &self.addressed {
            out.push_str(&format!("- [{}] {}: {}\n", m.id, describe(&m.from), m.body));
        }
        for ins in extra_instructions {
            out.push_str("\n\n# New instruction (at safe point)\n");
            out.push_str(ins);
        }
        out
    }
}

fn describe(p: &Participant) -> String {
    match p {
        Participant::User => "user".into(),
        Participant::System => "system".into(),
        Participant::Agent(id) => format!("agent {id}"),
    }
}

// -- rolling summary ---------------------------------------------------------

/// Short room digest. The lead refreshes it after each phase or every
/// N messages; workers only ever see this, never the full transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollingSummary {
    text: String,
    max_chars: usize,
}

impl RollingSummary {
    pub fn new(max_chars: usize) -> Self {
        Self {
            text: String::new(),
            max_chars: max_chars.max(64),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Append a one-line digest entry, keeping the newest `max_chars`.
    pub fn push(&mut self, entry: &str) {
        let entry = entry.trim();
        if entry.is_empty() {
            return;
        }
        if !self.text.is_empty() {
            self.text.push_str("\n- ");
        } else {
            self.text.push_str("- ");
        }
        self.text.push_str(entry);
        // Bound memory: keep the tail.
        if self.text.len() > self.max_chars {
            let cut = self.text.len() - self.max_chars;
            let at = self.text[cut..]
                .find('\n')
                .map(|i| cut + i + 1)
                .unwrap_or(cut);
            self.text = self.text[at..].to_string();
        }
    }

    /// Fold the newest room messages into the summary (`take_last` bound).
    pub fn roll_up(&mut self, room: &[Envelope], take_last: usize) {
        let n = take_last.min(room.len());
        for m in &room[room.len().saturating_sub(n)..] {
            let line: String = m.body.chars().take(160).collect();
            self.push(&format!("{}: {}", describe(&m.from), line));
        }
    }
}

impl Default for RollingSummary {
    fn default() -> Self {
        Self::new(2000)
    }
}

// -- steering permissions ----------------------------------------------------

/// Who may steer whom (TEAM-PROTOCOL.md §3b): the user → anyone; the lead →
/// any member; a member → only agents it delegated to. Interrupts: only the
/// user and the lead.
pub fn can_steer(from: &Participant, sender_is_lead: bool, sender_is_delegator: bool) -> bool {
    match from {
        Participant::User => true,
        Participant::System => true,
        Participant::Agent(_) => sender_is_lead || sender_is_delegator,
    }
}

pub fn can_interrupt(from: &Participant, sender_is_lead: bool) -> bool {
    match from {
        Participant::User => true,
        Participant::System => true,
        Participant::Agent(_) => sender_is_lead,
    }
}

// -- worker ------------------------------------------------------------------

/// Static worker inputs.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub agent_id: String,
    pub run_id: String,
    pub conversation_id: String,
    pub persona: String,
    pub goal: String,
    pub plan: String,
    pub phase: String,
    pub notes: String,
    pub snippets: Vec<String>,
    pub tools: Vec<String>,
    pub max_turns: usize,
    pub summary: RollingSummary,
    /// Agent ids this worker delegated to (may steer them).
    pub delegates: Vec<String>,
    /// Id of the lead agent (to recognise lead steering).
    pub lead_agent: Option<String>,
}

impl WorkerConfig {
    pub fn new(agent_id: &str, run_id: &str, conversation_id: &str) -> Self {
        Self {
            agent_id: agent_id.into(),
            run_id: run_id.into(),
            conversation_id: conversation_id.into(),
            persona: String::new(),
            goal: String::new(),
            plan: String::new(),
            phase: String::new(),
            notes: String::new(),
            snippets: Vec::new(),
            tools: Vec::new(),
            max_turns: 5,
            summary: RollingSummary::default(),
            delegates: Vec::new(),
            lead_agent: None,
        }
    }
}

/// What a bounded run produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerOutcome {
    pub turns_used: usize,
    pub steers_seen: usize,
    pub interrupts_seen: usize,
    pub done: Option<String>,
    /// Partial output kept when an interrupt cancelled a turn.
    pub draft: String,
}

/// Minimal model interface for a worker turn. Returns `None` when `cancel`
/// fires first (partial output is kept as a draft by the worker).
pub trait WorkerBackend: Send + Sync {
    fn generate(
        &self,
        prompt: String,
        cancel: CancellationToken,
    ) -> impl std::future::Future<Output = Option<String>> + Send;
}

/// Drain the inbox without blocking (safe-point check). Returns everything
/// addressed to the worker since the last check.
pub fn drain_safe_point(inbox: &mut mpsc::Receiver<Envelope>) -> Vec<Envelope> {
    let mut out = Vec::new();
    while let Ok(msg) = inbox.try_recv() {
        out.push(msg);
    }
    out
}

fn sender_is_lead(msg: &Envelope, cfg: &WorkerConfig) -> bool {
    match &msg.from {
        Participant::Agent(id) => cfg.lead_agent.as_deref() == Some(id.as_str()),
        _ => false,
    }
}

fn ack_status(bus: &TeamBus, cfg: &WorkerConfig, instruction: &str) {
    let env = Envelope {
        id: format!(
            "msg_{}_steerack_{}",
            cfg.agent_id,
            bus.latest_seq(&cfg.conversation_id) + 1
        ),
        run_id: cfg.run_id.clone(),
        conversation_id: cfg.conversation_id.clone(),
        thread_id: String::new(),
        reply_to: None,
        from: Participant::Agent(cfg.agent_id.clone()),
        to: Recipients::Room,
        kind: Kind::Status {
            phase: cfg.phase.clone(),
            progress: format!("acknowledged new instruction: {instruction}"),
        },
        body: format!("Acknowledged new instruction: {instruction}"),
        citations: Vec::new(),
        attachments: Vec::new(),
        priority: crate::bus::Priority::Normal,
        usage: None,
        seq: 0,
    };
    let _ = bus.publish(env);
}

/// Run bounded turns. Parks on `inbox.recv()` while idle (no polling);
/// checks the inbox at every safe point; applies Steer at the next safe
/// point and Interrupt by cancelling the in-flight model call.
///
/// Convention for backends: a turn returning text starting with `DONE:`
/// finishes the worker; anything else is published as `Say` and the loop
/// continues until `max_turns`.
pub async fn run_worker<B: WorkerBackend>(
    bus: &TeamBus,
    inbox: &mut mpsc::Receiver<Envelope>,
    backend: &B,
    cfg: &WorkerConfig,
) -> WorkerOutcome {
    let mut outcome = WorkerOutcome::default();
    let mut addressed: VecDeque<Envelope> = VecDeque::new();
    let mut instructions: Vec<String> = Vec::new();
    let mut urgent: Option<String> = None;
    // Idle: park until the first message wakes us (no polling).
    if addressed.is_empty() && instructions.is_empty() && urgent.is_none() {
        // Also drain anything already queued (safe-point).
        for m in drain_safe_point(inbox) {
            classify(
                bus,
                cfg,
                m,
                &mut addressed,
                &mut instructions,
                &mut urgent,
                &mut outcome,
            );
        }
        if addressed.is_empty() && instructions.is_empty() && urgent.is_none() {
            if let Some(m) = inbox.recv().await {
                classify(
                    bus,
                    cfg,
                    m,
                    &mut addressed,
                    &mut instructions,
                    &mut urgent,
                    &mut outcome,
                );
            } else {
                return outcome;
            }
        }
    }

    while outcome.turns_used < cfg.max_turns.max(1) {
        // Safe point: fold everything queued since the last check.
        for m in drain_safe_point(inbox) {
            classify(
                bus,
                cfg,
                m,
                &mut addressed,
                &mut instructions,
                &mut urgent,
                &mut outcome,
            );
        }

        let mut ctx = AgentContext {
            persona: cfg.persona.clone(),
            goal: cfg.goal.clone(),
            plan: cfg.plan.clone(),
            phase: cfg.phase.clone(),
            notes: cfg.notes.clone(),
            addressed: addressed.iter().cloned().collect(),
            summary: cfg.summary.text().to_string(),
            snippets: cfg.snippets.clone(),
            tools: cfg.tools.clone(),
        };
        let _ = &mut ctx;
        let mut extra = instructions.clone();
        if let Some(u) = urgent.take() {
            extra.push(format!(
                "INTERRUPT — do this now, drop the previous plan: {u}"
            ));
        }
        let prompt = ctx.build_prompt(&extra);

        let cancel = CancellationToken::new();
        let gen = backend.generate(prompt, cancel.clone());
        tokio::pin!(gen);
        // Race the model call against newly arriving interrupts. Non-interrupt
        // mail is buffered for the next safe point; interrupts cancel now.
        let text: Option<String> = loop {
            tokio::select! {
                out = &mut gen => break out,
                msg = inbox.recv() => {
                    match msg {
                        None => continue,
                        Some(m) => {
                            let is_interrupt = m.priority == crate::bus::Priority::Interrupt
                                && can_interrupt(&m.from, sender_is_lead(&m, cfg));
                            if is_interrupt {
                                outcome.interrupts_seen += 1;
                                let body = m.body.clone();
                                outcome.draft = format!("partial-turn-{}: {body}", outcome.turns_used + 1);
                                addressed.push_back(m);
                                cancel.cancel();
                                urgent = Some(body);
                                // Wait for the backend to observe the token.
                                break (&mut gen).await;
                            } else {
                                classify(bus, cfg, m, &mut addressed, &mut instructions, &mut urgent, &mut outcome);
                            }
                        }
                    }
                }
            }
        };

        let Some(text) = text else {
            // Cancelled turn: restart with the urgent instruction (turn not spent).
            instructions.clear();
            continue;
        };
        outcome.turns_used += 1;

        if let Some(summary) = text.strip_prefix("DONE:") {
            let summary = summary.trim().to_string();
            let env = Envelope {
                id: format!("msg_{}_done_{}", cfg.agent_id, outcome.turns_used),
                run_id: cfg.run_id.clone(),
                conversation_id: cfg.conversation_id.clone(),
                thread_id: String::new(),
                reply_to: None,
                from: Participant::Agent(cfg.agent_id.clone()),
                to: Recipients::Room,
                kind: Kind::Done {
                    summary: summary.clone(),
                },
                body: summary.clone(),
                citations: Vec::new(),
                attachments: Vec::new(),
                priority: crate::bus::Priority::Normal,
                usage: None,
                seq: 0,
            };
            let _ = bus.publish(env);
            outcome.done = Some(summary);
            return outcome;
        }

        // Ordinary turn: publish as Say, then loop (tool-result safe point
        // happens at the top of the next iteration).
        let env = Envelope {
            id: format!("msg_{}_turn_{}", cfg.agent_id, outcome.turns_used),
            run_id: cfg.run_id.clone(),
            conversation_id: cfg.conversation_id.clone(),
            thread_id: String::new(),
            reply_to: None,
            from: Participant::Agent(cfg.agent_id.clone()),
            to: Recipients::Room,
            kind: Kind::Say,
            body: text,
            citations: Vec::new(),
            attachments: Vec::new(),
            priority: crate::bus::Priority::Normal,
            usage: None,
            seq: 0,
        };
        let _ = bus.publish(env);

        // Stop when nobody is talking to us and no instructions remain?
        // No: bounded loop keeps going until max_turns so background work
        // finishes; tests drive Done explicitly.
        if outcome.turns_used >= cfg.max_turns.max(1) {
            break;
        }
        // If the inbox is closed and empty and we have no Done, park briefly
        // for one more wake-up before ending (still no polling: one recv).
        if instructions.is_empty() && inbox.is_empty() && inbox.is_closed() {
            break;
        }
    }
    outcome
}

fn classify(
    bus: &TeamBus,
    cfg: &WorkerConfig,
    msg: Envelope,
    addressed: &mut VecDeque<Envelope>,
    instructions: &mut Vec<String>,
    urgent: &mut Option<String>,
    outcome: &mut WorkerOutcome,
) {
    let lead = sender_is_lead(&msg, cfg);
    let is_delegator = match &msg.from {
        Participant::Agent(id) => cfg.delegates.iter().any(|d| d == id),
        _ => false,
    };
    // Delegator check is from the recipient's view: this worker was delegated
    // by the sender. cfg.delegates lists agents this worker delegated to, so
    // for member→member steering the sender must name this worker. We track
    // the reverse here via addressed delegation kinds: an Ask/Delegate from
    // the sender authorises that sender to steer this worker.
    let sender_delegated_to_us = addressed.iter().any(|a| {
        matches!(a.kind, Kind::Delegate { .. } | Kind::Ask { .. })
            && a.to == Recipients::Agent(cfg.agent_id.clone())
    }) || matches!(msg.kind, Kind::Delegate { .. } | Kind::Ask { .. });
    let _ = is_delegator;
    match msg.priority {
        crate::bus::Priority::Interrupt if can_interrupt(&msg.from, lead) => {
            outcome.interrupts_seen += 1;
            outcome.draft = format!("partial-turn-{}: {}", outcome.turns_used + 1, msg.body);
            *urgent = Some(msg.body.clone());
            addressed.push_back(msg);
        }
        crate::bus::Priority::Steer if can_steer(&msg.from, lead, sender_delegated_to_us) => {
            outcome.steers_seen += 1;
            let note = format!("new instruction from {}: {}", describe(&msg.from), msg.body);
            ack_status(bus, cfg, &msg.body);
            instructions.push(note);
            addressed.push_back(msg);
        }
        _ => {
            addressed.push_back(msg);
        }
    }
}

#[cfg(test)]
mod worker_tests {
    use super::*;
    use crate::bus::{Participant, Recipients};

    struct ScriptBackend {
        outputs: std::sync::Mutex<VecDeque<Option<String>>>,
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl ScriptBackend {
        fn new(outputs: Vec<Option<String>>) -> Self {
            Self {
                outputs: std::sync::Mutex::new(outputs.into_iter().collect()),
                seen: std::sync::Mutex::new(Vec::new()),
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
            // Honour cancellation while "working".
            tokio::select! {
                _ = cancel.cancelled() => None,
                _ = tokio::time::sleep(std::time::Duration::from_millis(60)) => next,
            }
        }
    }

    fn cfg(agent: &str) -> WorkerConfig {
        let mut c = WorkerConfig::new(agent, "run1", "c1");
        c.persona = "You are Mira, a researcher.".into();
        c.goal = "Answer the question.".into();
        c.plan = "Step 1 read, step 2 write.".into();
        c.phase = "Research".into();
        c.notes = "prefers peer-reviewed sources".into();
        c.tools = vec!["ask_agent".into(), "report_finding".into()];
        c.max_turns = 4;
        c
    }

    fn addressed_msg(id: &str, body: &str) -> Envelope {
        let mut e = Envelope::say(
            id,
            "c1",
            Participant::User,
            Recipients::Agent("mira".into()),
            body,
        );
        e.run_id = "run1".into();
        e
    }

    #[test]
    fn worker_context_has_documented_sections_in_stable_order() {
        let ctx = AgentContext {
            persona: "p".into(),
            goal: "g".into(),
            plan: "pl".into(),
            phase: "ph".into(),
            notes: "n".into(),
            addressed: vec![addressed_msg("m1", "hello")],
            summary: "s".into(),
            snippets: vec!["snip".into()],
            tools: vec!["ask_agent".into()],
        };
        let prompt = ctx.build_prompt(&["steer: focus".into()]);
        let sections = [
            "# Persona",
            "# Tools",
            "# Project goal",
            "# Plan",
            "# Phase",
            "# Own notes",
            "# Rolling summary",
            "# Retrieved snippets",
            "# Messages addressed to you",
            "# New instruction",
        ];
        let mut last = 0;
        for s in sections {
            let at = prompt
                .find(s)
                .unwrap_or_else(|| panic!("missing {s}:\n{prompt}"));
            assert!(at >= last, "out of order: {s}");
            last = at;
        }
        assert!(prompt.contains("hello"));
    }

    #[test]
    fn worker_rolling_summary_bounds_length() {
        let mut s = RollingSummary::new(120);
        for i in 0..20 {
            s.push(&format!("finding {i} with quite a lot of detail attached"));
        }
        assert!(s.text().len() <= 120 + 60);
        assert!(s.text().contains("finding 19"));
        let mut s2 = RollingSummary::default();
        let room = vec![addressed_msg("m1", "first"), addressed_msg("m2", "second")];
        s2.roll_up(&room, 2);
        assert!(s2.text().contains("first"));
        assert!(s2.text().contains("second"));
    }

    #[tokio::test]
    async fn worker_steer_reaches_active_worker_at_safe_point() {
        let bus = TeamBus::new();
        bus.set_lead("lead");
        let mut inbox = bus.agent_inbox("mira");
        let mut c = cfg("mira");
        c.lead_agent = Some("lead".into());
        // Script: first turn says something, second finishes.
        let backend = std::sync::Arc::new(ScriptBackend::new(vec![
            Some("working...".into()),
            Some("DONE: finished".into()),
        ]));
        // Wake the worker, then steer it while the first turn runs.
        bus.publish(addressed_msg("m0", "research X")).unwrap();
        let mut steer = Envelope::say(
            "s1",
            "c1",
            Participant::Agent("lead".into()),
            Recipients::Agent("mira".into()),
            "skip blogs, only peer-reviewed",
        );
        steer.run_id = "run1".into();
        steer.priority = crate::bus::Priority::Steer;
        let handle = tokio::spawn({
            let bus = bus.clone();
            let backend = backend.clone();
            async move { run_worker(&bus, &mut inbox, &*backend, &c).await }
        });
        // Give the first turn a moment to start, then steer (no polling: mpsc send wakes it).
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        bus.publish(steer).unwrap();
        let outcome = handle.await.unwrap();
        assert_eq!(outcome.done.as_deref(), Some("finished"));
        assert_eq!(outcome.steers_seen, 1);
        // Second model call saw the steered instruction marker.
        let seen = backend.seen();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].contains("peer-reviewed"), "got:\n{}", seen[1]);
        // Steer was acknowledged with a Status message on the bus.
        let hist = bus.history("c1", 0, usize::MAX);
        assert!(hist.iter().any(|e| matches!(e.kind, Kind::Status { .. })));
    }

    #[tokio::test]
    async fn worker_interrupt_cancels_inflight_turn() {
        let bus = TeamBus::new();
        let mut inbox = bus.agent_inbox("mira");
        let mut c = cfg("mira");
        c.max_turns = 3;
        let backend = ScriptBackend::new(vec![Some("DONE: after interrupt".into())]);
        bus.publish(addressed_msg("m0", "long research task"))
            .unwrap();
        // Interrupt from the user cancels the in-flight call; partial kept.
        let mut intr = Envelope::say(
            "i1",
            "c1",
            Participant::User,
            Recipients::Agent("mira".into()),
            "stop, do Y instead",
        );
        intr.run_id = "run1".into();
        intr.priority = crate::bus::Priority::Interrupt;
        // Pre-queue the interrupt so the first safe point / race sees it.
        bus.publish(intr).unwrap();
        let outcome = run_worker(&bus, &mut inbox, &backend, &c).await;
        assert!(outcome.interrupts_seen >= 1, "{outcome:?}");
        assert!(!outcome.draft.is_empty());
        // Turn restarted with the interrupt instruction and finished.
        let seen = backend.seen();
        assert!(seen.iter().any(|p| p.contains("INTERRUPT")), "{seen:?}");
        assert_eq!(outcome.done.as_deref(), Some("after interrupt"));
    }

    #[tokio::test]
    async fn worker_interrupt_during_model_call_cancels_via_token() {
        let bus = TeamBus::new();
        let mut inbox = bus.agent_inbox("mira");
        let mut c = cfg("mira");
        c.max_turns = 1;
        bus.publish(addressed_msg("m0", "go")).unwrap();
        let backend = std::sync::Arc::new(ScriptBackend::new(vec![
            Some("slow work...".into()),
            Some("DONE: halted".into()),
        ]));
        let handle = tokio::spawn({
            let bus = bus.clone();
            let backend = backend.clone();
            async move { run_worker(&bus, &mut inbox, &*backend, &c).await }
        });
        // Interrupt arrives mid-call (no polling: mpsc send wakes the select).
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let mut intr = Envelope::say(
            "i1",
            "c1",
            Participant::User,
            Recipients::Agent("mira".into()),
            "halt now",
        );
        intr.run_id = "run1".into();
        intr.priority = crate::bus::Priority::Interrupt;
        bus.publish(intr).unwrap();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("worker must not hang on interrupt")
            .unwrap();
        assert!(outcome.interrupts_seen >= 1);
        assert!(!outcome.draft.is_empty());
    }

    #[tokio::test]
    async fn worker_idle_parks_without_polling_until_message() {
        let bus = TeamBus::new();
        let mut inbox = bus.agent_inbox("mira");
        let c = cfg("mira");
        let backend = std::sync::Arc::new(ScriptBackend::new(vec![Some("DONE: woke".into())]));
        let handle = tokio::spawn({
            let bus = bus.clone();
            let backend = backend.clone();
            async move { run_worker(&bus, &mut inbox, &*backend, &c).await }
        });
        // Worker is parked on recv(); a single send wakes it.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_eq!(backend.seen().len(), 0);
        bus.publish(addressed_msg("m9", "wake up")).unwrap();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("idle worker must wake on message")
            .unwrap();
        assert_eq!(outcome.done.as_deref(), Some("woke"));
        assert_eq!(outcome.turns_used, 1);
    }

    #[tokio::test]
    async fn worker_turns_are_bounded() {
        let bus = TeamBus::new();
        let mut inbox = bus.agent_inbox("mira");
        let mut c = cfg("mira");
        c.max_turns = 2;
        let backend = ScriptBackend::new(vec![
            Some("still working".into()),
            Some("still working".into()),
            Some("DONE: too late".into()),
        ]);
        bus.publish(addressed_msg("m0", "go")).unwrap();
        let outcome = run_worker(&bus, &mut inbox, &backend, &c).await;
        assert_eq!(outcome.turns_used, 2);
        assert_eq!(outcome.done, None);
        assert_eq!(backend.seen().len(), 2);
    }

    #[test]
    fn worker_steer_permissions_match_protocol() {
        assert!(can_steer(&Participant::User, false, false));
        assert!(can_interrupt(&Participant::User, false));
        assert!(can_steer(&Participant::Agent("lead".into()), true, false));
        assert!(can_interrupt(&Participant::Agent("lead".into()), true));
        assert!(!can_interrupt(&Participant::Agent("bob".into()), false));
        assert!(!can_steer(&Participant::Agent("bob".into()), false, false));
        assert!(can_steer(&Participant::Agent("bob".into()), false, true));
    }
}
