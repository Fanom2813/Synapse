//! Run orchestration: modes, phases, speaker selection, budgets,
//! convergence and pause/resume/stop (see docs/TEAM-PROTOCOL.md §4–6, §9).
//!
//! Pure logic over [`Envelope`] history so it stays testable without I/O.
//! Persistence is a JSON snapshot (`RunSnapshot`): phase, plan, ledger and
//! member list round-trip so an `interrupted` run can be resumed.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::bus::{Envelope, Kind, Participant, Recipients};

// -- modes + phases ----------------------------------------------------------

/// Mode presets (TEAM-PROTOCOL.md §4 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Brainstorm,
    DeepResearch,
    DeepAnalysis,
    Revise,
    FreeDiscussion,
}

/// Run phases. `Paused`/`Stopped`/`Failed` are statuses, not phases, but the
/// state machine surface keeps them distinct via [`RunStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    #[default]
    Idle,
    Plan,
    Brainstorm,
    Research,
    Analysis,
    Critique,
    Revise,
    Synthesize,
    Done,
}

/// Liveness of a run (TEAM-PROTOCOL.md §4 diagram).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Idle,
    Running,
    Paused,
    Stopped,
    Failed,
    /// Persisted mid-run by quit; resumable via [`RunState::resume`].
    Interrupted,
    Done,
}

/// Phases for each mode preset.
pub fn phases_for_mode(mode: RunMode) -> Vec<RunPhase> {
    match mode {
        RunMode::Brainstorm => vec![
            RunPhase::Plan,
            RunPhase::Brainstorm,
            RunPhase::Critique,
            RunPhase::Synthesize,
            RunPhase::Done,
        ],
        RunMode::DeepResearch => vec![
            RunPhase::Plan,
            RunPhase::Research,
            RunPhase::Analysis,
            RunPhase::Critique,
            RunPhase::Revise,
            RunPhase::Synthesize,
            RunPhase::Done,
        ],
        RunMode::DeepAnalysis => vec![
            RunPhase::Plan,
            RunPhase::Analysis,
            RunPhase::Critique,
            RunPhase::Revise,
            RunPhase::Synthesize,
            RunPhase::Done,
        ],
        RunMode::Revise => vec![
            RunPhase::Critique,
            RunPhase::Revise,
            RunPhase::Synthesize,
            RunPhase::Done,
        ],
        RunMode::FreeDiscussion => vec![RunPhase::Plan, RunPhase::Done],
    }
}

/// Parallel (delegated, semaphore-limited) vs discussion (selector) phases.
pub fn is_parallel_phase(phase: RunPhase) -> bool {
    matches!(phase, RunPhase::Research | RunPhase::Analysis)
}

// -- limits + budget ---------------------------------------------------------

/// All configurable limits (TEAM-PROTOCOL.md §6 table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunLimits {
    pub max_concurrent: usize,
    pub max_turns_per_phase: usize,
    pub critique_revise_rounds: usize,
    pub max_critique_revise_rounds: usize,
    pub delegation_depth: usize,
    pub per_agent_tool_calls: usize,
    pub budget_usd: f64,
    pub token_budget: u64,
    pub warn_fraction: f64,
    pub wall_clock_secs: u64,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            max_turns_per_phase: 12,
            critique_revise_rounds: 2,
            max_critique_revise_rounds: 3,
            delegation_depth: 2,
            per_agent_tool_calls: 25,
            budget_usd: 1.0,
            token_budget: 400_000,
            warn_fraction: 0.8,
            wall_clock_secs: 30 * 60,
        }
    }
}

/// Spend ledger: usage reported by providers × models.dev prices.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BudgetLedger {
    pub spent_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl BudgetLedger {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    pub fn fraction(&self, limits: &RunLimits) -> f64 {
        let usd = if limits.budget_usd > 0.0 {
            self.spent_usd / limits.budget_usd
        } else {
            0.0
        };
        let tok = if limits.token_budget > 0 {
            self.total_tokens() as f64 / limits.token_budget as f64
        } else {
            0.0
        };
        usd.max(tok)
    }
}

/// Outcome of recording usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetEvent {
    Ok,
    Warn,
    Exhausted,
}

// -- speaker selection --------------------------------------------------------

/// Who the selector picked (TEAM-PROTOCOL.md §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeakerDecision {
    Agent(String),
    Converged,
    WaitUser,
}

fn agent_of(p: &Participant) -> Option<&str> {
    match p {
        Participant::Agent(id) => Some(id.as_str()),
        _ => None,
    }
}

/// Rule-based selector first, cheap-model selector second (§5):
///
/// - pending `Ask`/`Delegate` → that agent;
/// - unanswered `Challenge` → the challenged author;
/// - @mentioned agent → that agent;
/// - otherwise next speaker round-robin (never repeats unless asked).
///
/// Returns `Converged` when [`convergence_status`] says the latest proposal
/// has majority agreement.
pub fn select_speaker(
    history: &[Envelope],
    members: &[String],
    last_speaker: Option<&str>,
    mentioned: &[String],
) -> SpeakerDecision {
    if members.is_empty() {
        return SpeakerDecision::WaitUser;
    }
    if convergence_status(history, members) == Convergence::Converged {
        return SpeakerDecision::Converged;
    }
    // 1. Pending Ask / Delegate: last one whose target has not spoken since.
    for msg in history.iter().rev() {
        match &msg.kind {
            Kind::Ask { .. } | Kind::Delegate { .. } => {
                let targets: Vec<String> = match &msg.to {
                    Recipients::Agent(id) => vec![id.clone()],
                    Recipients::Agents(ids) => ids.clone(),
                    _ => Vec::new(),
                };
                for t in targets {
                    if !members.contains(&t) {
                        continue;
                    }
                    let answered = history
                        .iter()
                        .rev()
                        .take_while(|m| m.id != msg.id)
                        .any(|m| {
                            agent_of(&m.from) == Some(t.as_str())
                                && (matches!(
                                    m.kind,
                                    Kind::Answer | Kind::Done { .. } | Kind::Finding { .. }
                                ) || m.reply_to.as_deref() == Some(msg.id.as_str()))
                        });
                    // Simpler robust rule: target speaks if it sent anything after the ask.
                    let idx = history.iter().position(|m| m.id == msg.id).unwrap_or(0);
                    let spoke_after = history[idx..]
                        .iter()
                        .skip(1)
                        .any(|m| agent_of(&m.from) == Some(t.as_str()));
                    if !answered && !spoke_after {
                        return SpeakerDecision::Agent(t);
                    }
                }
            }
            _ => {}
        }
    }
    // 2. Unanswered Challenge → challenged author.
    for (i, msg) in history.iter().enumerate().rev() {
        if let Kind::Challenge { claim_ref } = &msg.kind {
            if let Some(target) = history
                .iter()
                .find(|m| m.id == *claim_ref)
                .and_then(|m| agent_of(&m.from).map(str::to_string))
            {
                let answered = history[i + 1..]
                    .iter()
                    .any(|m| agent_of(&m.from) == Some(target.as_str()));
                if members.contains(&target) && !answered {
                    return SpeakerDecision::Agent(target);
                }
            }
        }
    }
    // 3. @mentioned member.
    for m in mentioned {
        if members.contains(m) {
            return SpeakerDecision::Agent(m.clone());
        }
    }
    // 4. Round-robin avoiding repeats.
    let next = match last_speaker {
        Some(last) => {
            let pos = members.iter().position(|m| m == last).unwrap_or(0);
            members[(pos + 1) % members.len()].clone()
        }
        None => members[0].clone(),
    };
    SpeakerDecision::Agent(next)
}

// -- convergence --------------------------------------------------------------

/// Convergence state for discussion phases (§8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convergence {
    Pending,
    Converged,
    TurnLimit,
}

/// `Propose` + majority `Vote` agree (author excluded), or turn limit.
pub fn convergence_status(history: &[Envelope], members: &[String]) -> Convergence {
    let Some(propose) = history
        .iter()
        .rev()
        .find(|m| matches!(m.kind, Kind::Propose { .. }))
    else {
        return Convergence::Pending;
    };
    let author = agent_of(&propose.from).unwrap_or("");
    let mut agree: HashSet<String> = HashSet::new();
    for m in history.iter() {
        if let Kind::Vote { on, stance } = &m.kind {
            if on == &propose.id && stance.eq_ignore_ascii_case("agree") {
                if let Some(voter) = agent_of(&m.from) {
                    if voter != author {
                        agree.insert(voter.to_string());
                    }
                }
            }
        }
    }
    let voters = members.iter().filter(|m| *m != author).count().max(1);
    if agree.len() * 2 > voters || (voters == 1 && !agree.is_empty()) {
        Convergence::Converged
    } else {
        Convergence::Pending
    }
}

// -- run state ----------------------------------------------------------------

/// Serializable snapshot: everything needed to resume an interrupted run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSnapshot {
    pub id: String,
    pub mode: RunMode,
    pub phase: RunPhase,
    pub status: RunStatus,
    pub phase_index: usize,
    pub members: Vec<String>,
    pub lead: Option<String>,
    pub plan: String,
    pub turn_count: usize,
    pub revise_round: usize,
    pub ledger: BudgetLedger,
    pub limits: RunLimits,
    pub last_speaker: Option<String>,
    pub started_at_secs: u64,
}

/// Live run with controls (§9): pause/resume/stop, skip phase, budget and
/// membership changes, interrupted-run resume.
#[derive(Debug, Clone)]
pub struct RunState {
    pub id: String,
    pub mode: RunMode,
    pub phases: Vec<RunPhase>,
    pub phase_index: usize,
    pub status: RunStatus,
    pub members: Vec<String>,
    pub lead: Option<String>,
    pub plan: String,
    pub turn_count: usize,
    pub revise_round: usize,
    pub ledger: BudgetLedger,
    pub limits: RunLimits,
    pub last_speaker: Option<String>,
    pub started_at_secs: u64,
    pub warned: bool,
}

impl RunState {
    pub fn new(id: &str, mode: RunMode, members: Vec<String>, now_secs: u64) -> Self {
        let phases = phases_for_mode(mode);
        let phase0 = phases.first().copied().unwrap_or(RunPhase::Done);
        let lead = members.first().cloned();
        Self {
            id: id.into(),
            mode,
            phases,
            phase_index: 0,
            status: RunStatus::Running,
            members,
            lead,
            plan: String::new(),
            turn_count: 0,
            revise_round: 0,
            ledger: BudgetLedger::default(),
            limits: RunLimits::default(),
            last_speaker: None,
            started_at_secs: now_secs,
            warned: phase0 == RunPhase::Done,
        }
    }

    pub fn phase(&self) -> RunPhase {
        self.phases
            .get(self.phase_index)
            .copied()
            .unwrap_or(RunPhase::Done)
    }

    pub fn is_parallel(&self) -> bool {
        is_parallel_phase(self.phase())
    }

    /// Semaphore bound for parallel phases (TEAM-PROTOCOL.md §3c/§6).
    pub fn concurrency(&self) -> usize {
        self.limits
            .max_concurrent
            .max(1)
            .min(self.members.len().max(1))
    }

    pub fn deadline_secs(&self) -> u64 {
        self.started_at_secs
            .saturating_add(self.limits.wall_clock_secs)
    }

    pub fn is_expired(&self, now_secs: u64) -> bool {
        now_secs >= self.deadline_secs()
    }

    /// Advance one phase; Critique↔Revise repeats up to the round cap, then
    /// continues to Synthesize (repeat Critique→Revise ≤ max_rounds).
    pub fn advance_phase(&mut self) {
        if self.phase() == RunPhase::Critique
            && self.revise_round < self.limits.critique_revise_rounds
        {
            // Critique → Revise directly (adjacent in mode lists).
        }
        if self.phase() == RunPhase::Revise {
            self.revise_round += 1;
            if self.revise_round < self.limits.critique_revise_rounds {
                // Loop back to Critique for another round.
                if let Some(i) = self.phases.iter().position(|p| *p == RunPhase::Critique) {
                    self.phase_index = i;
                    self.turn_count = 0;
                    return;
                }
            }
        }
        if self.phase_index + 1 < self.phases.len() {
            self.phase_index += 1;
            self.turn_count = 0;
        }
        if self.phase() == RunPhase::Done {
            self.status = RunStatus::Done;
        }
    }

    /// Skip the current phase (user control §9).
    pub fn skip_phase(&mut self) {
        self.advance_phase();
    }

    pub fn pause(&mut self) {
        if self.status == RunStatus::Running {
            self.status = RunStatus::Paused;
        }
    }

    pub fn resume(&mut self) {
        match self.status {
            RunStatus::Paused | RunStatus::Interrupted | RunStatus::Idle => {
                self.status = RunStatus::Running;
            }
            _ => {}
        }
    }

    pub fn stop(&mut self) {
        self.status = RunStatus::Stopped;
    }

    pub fn fail(&mut self) {
        self.status = RunStatus::Failed;
    }

    pub fn mark_interrupted(&mut self) {
        if self.status == RunStatus::Running || self.status == RunStatus::Paused {
            self.status = RunStatus::Interrupted;
        }
    }

    pub fn set_budget_usd(&mut self, usd: f64) {
        if usd > 0.0 {
            self.limits.budget_usd = usd;
        }
    }

    pub fn add_member(&mut self, agent: &str) {
        if !self.members.iter().any(|m| m == agent) {
            self.members.push(agent.into());
        }
    }

    pub fn remove_member(&mut self, agent: &str) {
        self.members.retain(|m| m != agent);
        if self.lead.as_deref() == Some(agent) {
            self.lead = self.members.first().cloned();
        }
    }

    /// Record provider-reported usage. Warn at 80 %, exhaust at 100 %
    /// (whichever of USD / tokens hits first) — exhausted pauses the run.
    pub fn record_usage(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
    ) -> BudgetEvent {
        self.ledger.input_tokens = self.ledger.input_tokens.saturating_add(input_tokens);
        self.ledger.output_tokens = self.ledger.output_tokens.saturating_add(output_tokens);
        self.ledger.spent_usd += cost_usd;
        let frac = self.ledger.fraction(&self.limits);
        if frac >= 1.0 {
            self.pause();
            BudgetEvent::Exhausted
        } else if frac >= self.limits.warn_fraction {
            self.warned = true;
            BudgetEvent::Warn
        } else {
            BudgetEvent::Ok
        }
    }

    /// One discussion turn: bump the counter and report turn-limit convergence.
    pub fn record_turn(&mut self, speaker: &str) -> Convergence {
        self.turn_count += 1;
        self.last_speaker = Some(speaker.into());
        if self.turn_count >= self.limits.max_turns_per_phase {
            Convergence::TurnLimit
        } else {
            Convergence::Pending
        }
    }

    pub fn snapshot(&self) -> RunSnapshot {
        RunSnapshot {
            id: self.id.clone(),
            mode: self.mode,
            phase: self.phase(),
            status: if self.status == RunStatus::Running {
                RunStatus::Interrupted
            } else {
                self.status
            },
            phase_index: self.phase_index,
            members: self.members.clone(),
            lead: self.lead.clone(),
            plan: self.plan.clone(),
            turn_count: self.turn_count,
            revise_round: self.revise_round,
            ledger: self.ledger.clone(),
            limits: self.limits.clone(),
            last_speaker: self.last_speaker.clone(),
            started_at_secs: self.started_at_secs,
        }
    }

    /// Restore from a snapshot (quit-resume path: `interrupted` → `resume`).
    pub fn restore(snap: RunSnapshot) -> Self {
        let phases = phases_for_mode(snap.mode);
        let phase_index = snap.phase_index.min(phases.len().saturating_sub(1));
        Self {
            id: snap.id,
            mode: snap.mode,
            phases,
            phase_index,
            status: snap.status,
            members: snap.members,
            lead: snap.lead,
            plan: snap.plan,
            turn_count: snap.turn_count,
            revise_round: snap.revise_round,
            ledger: snap.ledger,
            limits: snap.limits,
            last_speaker: snap.last_speaker,
            started_at_secs: snap.started_at_secs,
            warned: false,
        }
    }

    /// Per-agent tool-call budget check (§6: 25 per sub-task).
    pub fn tool_calls_allowed(
        calls_used: &HashMap<String, usize>,
        agent: &str,
        limits: &RunLimits,
    ) -> bool {
        calls_used.get(agent).copied().unwrap_or(0) < limits.per_agent_tool_calls
    }
}

#[cfg(test)]
mod orchestrator_tests {
    use super::*;
    use crate::bus::{Participant, Recipients};

    fn members() -> Vec<String> {
        vec!["lead".into(), "mira".into(), "ivan".into()]
    }

    fn say(id: &str, from: &str, to: Recipients, body: &str) -> Envelope {
        let mut e = Envelope::say(id, "c1", Participant::Agent(from.into()), to, body);
        e.run_id = "run1".into();
        e
    }

    fn ask(id: &str, from: &str, to: &str, q: &str) -> Envelope {
        Envelope {
            id: id.into(),
            run_id: "run1".into(),
            conversation_id: "c1".into(),
            thread_id: String::new(),
            reply_to: None,
            from: Participant::Agent(from.into()),
            to: Recipients::Agent(to.into()),
            kind: Kind::Ask { question: q.into() },
            body: q.into(),
            citations: vec![],
            attachments: vec![],
            priority: crate::bus::Priority::Normal,
            usage: None,
            seq: 0,
        }
    }

    #[test]
    fn orchestrator_modes_map_to_documented_phases() {
        assert_eq!(
            phases_for_mode(RunMode::Brainstorm),
            vec![
                RunPhase::Plan,
                RunPhase::Brainstorm,
                RunPhase::Critique,
                RunPhase::Synthesize,
                RunPhase::Done
            ]
        );
        assert_eq!(
            phases_for_mode(RunMode::DeepResearch),
            vec![
                RunPhase::Plan,
                RunPhase::Research,
                RunPhase::Analysis,
                RunPhase::Critique,
                RunPhase::Revise,
                RunPhase::Synthesize,
                RunPhase::Done
            ]
        );
        assert_eq!(
            phases_for_mode(RunMode::DeepAnalysis),
            vec![
                RunPhase::Plan,
                RunPhase::Analysis,
                RunPhase::Critique,
                RunPhase::Revise,
                RunPhase::Synthesize,
                RunPhase::Done
            ]
        );
        assert_eq!(
            phases_for_mode(RunMode::Revise).first(),
            Some(&RunPhase::Critique)
        );
        assert!(is_parallel_phase(RunPhase::Research));
        assert!(is_parallel_phase(RunPhase::Analysis));
        assert!(!is_parallel_phase(RunPhase::Brainstorm));
        assert!(!is_parallel_phase(RunPhase::Critique));
    }

    #[test]
    fn orchestrator_state_machine_advances_and_repeats_critique_revise() {
        let mut run = RunState::new("r1", RunMode::DeepResearch, members(), 0);
        assert_eq!(run.phase(), RunPhase::Plan);
        assert_eq!(run.status, RunStatus::Running);
        run.advance_phase();
        assert_eq!(run.phase(), RunPhase::Research);
        // Walk to Critique → Revise → (round 2) Critique → Revise → Synthesize.
        run.advance_phase(); // Analysis
        run.advance_phase(); // Critique
        assert_eq!(run.phase(), RunPhase::Critique);
        run.advance_phase(); // Revise
        assert_eq!(run.phase(), RunPhase::Revise);
        run.advance_phase(); // loops back to Critique (round 1 < 2)
        assert_eq!(run.phase(), RunPhase::Critique);
        assert_eq!(run.revise_round, 1);
        run.advance_phase(); // Revise again
        assert_eq!(run.phase(), RunPhase::Revise);
        run.advance_phase(); // round cap reached → Synthesize
        assert_eq!(run.phase(), RunPhase::Synthesize);
        run.advance_phase();
        assert_eq!(run.phase(), RunPhase::Done);
        assert_eq!(run.status, RunStatus::Done);
    }

    #[test]
    fn orchestrator_selector_prefers_pending_ask_then_challenge_then_mention() {
        let m = members();
        // Pending Ask → that agent.
        let hist = vec![ask("a1", "lead", "mira", "read this")];
        assert_eq!(
            select_speaker(&hist, &m, Some("lead"), &[]),
            SpeakerDecision::Agent("mira".into())
        );
        // Answered Ask → falls through to round-robin.
        let mut hist2 = hist.clone();
        let mut ans = say("a2", "mira", Recipients::Room, "done");
        ans.reply_to = Some("a1".into());
        hist2.push(ans);
        assert_eq!(
            select_speaker(&hist2, &m, Some("mira"), &[]),
            SpeakerDecision::Agent("ivan".into())
        );
        // Unanswered Challenge → challenged author.
        let claim = say("c0", "ivan", Recipients::Room, "X causes Y");
        let ch = Envelope {
            id: "c1".into(),
            run_id: "run1".into(),
            conversation_id: "c1".into(),
            thread_id: String::new(),
            reply_to: Some("c0".into()),
            from: Participant::Agent("mira".into()),
            to: Recipients::Room,
            kind: Kind::Challenge {
                claim_ref: "c0".into(),
            },
            body: "no evidence".into(),
            citations: vec![],
            attachments: vec![],
            priority: crate::bus::Priority::Normal,
            usage: None,
            seq: 0,
        };
        let hist3 = vec![claim, ch];
        assert_eq!(
            select_speaker(&hist3, &m, Some("mira"), &[]),
            SpeakerDecision::Agent("ivan".into())
        );
        // @mention wins over round-robin.
        let hist4: Vec<Envelope> = vec![say("m1", "lead", Recipients::Room, "hi")];
        assert_eq!(
            select_speaker(&hist4, &m, Some("lead"), &["ivan".into()]),
            SpeakerDecision::Agent("ivan".into())
        );
        // No repeat: round-robin steps past last speaker.
        assert_eq!(
            select_speaker(&hist4, &m, Some("ivan"), &[]),
            SpeakerDecision::Agent("lead".into())
        );
    }

    #[test]
    fn orchestrator_convergence_needs_majority_agree_votes() {
        let m = members();
        let prop = Envelope {
            id: "p1".into(),
            run_id: "run1".into(),
            conversation_id: "c1".into(),
            thread_id: String::new(),
            reply_to: None,
            from: Participant::Agent("lead".into()),
            to: Recipients::Room,
            kind: Kind::Propose {
                conclusion: "ship it".into(),
            },
            body: "ship it".into(),
            citations: vec![],
            attachments: vec![],
            priority: crate::bus::Priority::Normal,
            usage: None,
            seq: 0,
        };
        let vote = |id: &str, from: &str, stance: &str| Envelope {
            id: id.into(),
            run_id: "run1".into(),
            conversation_id: "c1".into(),
            thread_id: String::new(),
            reply_to: Some("p1".into()),
            from: Participant::Agent(from.into()),
            to: Recipients::Room,
            kind: Kind::Vote {
                on: "p1".into(),
                stance: stance.into(),
            },
            body: stance.into(),
            citations: vec![],
            attachments: vec![],
            priority: crate::bus::Priority::Normal,
            usage: None,
            seq: 0,
        };
        assert_eq!(convergence_status(&[], &m), Convergence::Pending);
        assert_eq!(
            convergence_status(std::slice::from_ref(&prop), &m),
            Convergence::Pending
        );
        // One agree of two non-author members is not a majority.
        let one = vec![prop.clone(), vote("v1", "mira", "agree")];
        assert_eq!(convergence_status(&one, &m), Convergence::Pending);
        // Both agree → majority → converged, and selector reports it.
        let both = vec![
            prop.clone(),
            vote("v1", "mira", "agree"),
            vote("v2", "ivan", "agree"),
        ];
        assert_eq!(convergence_status(&both, &m), Convergence::Converged);
        assert_eq!(
            select_speaker(&both, &m, Some("lead"), &[]),
            SpeakerDecision::Converged
        );
        // Author's own vote does not count.
        let self_vote = vec![prop.clone(), vote("v1", "lead", "agree")];
        assert_eq!(convergence_status(&self_vote, &m), Convergence::Pending);
    }

    #[test]
    fn orchestrator_budgets_warn_at_80_and_pause_at_100() {
        let mut run = RunState::new("r1", RunMode::Brainstorm, members(), 0);
        assert_eq!(run.record_usage(100, 0, 0.5), BudgetEvent::Ok);
        assert_eq!(run.status, RunStatus::Running);
        // 0.5 + 0.35 = 0.85 ≥ 80 % → warn, still running.
        assert_eq!(run.record_usage(0, 0, 0.35), BudgetEvent::Warn);
        assert!(run.warned);
        assert_eq!(run.status, RunStatus::Running);
        // Over budget → pause.
        assert_eq!(run.record_usage(0, 0, 0.5), BudgetEvent::Exhausted);
        assert_eq!(run.status, RunStatus::Paused);
        // Token budget is the other rail: fresh run, 400k tokens.
        let mut run2 = RunState::new("r2", RunMode::Brainstorm, members(), 0);
        assert_eq!(run2.record_usage(320_000, 0, 0.0), BudgetEvent::Warn);
        assert_eq!(run2.record_usage(80_000, 0, 0.0), BudgetEvent::Exhausted);
    }

    #[test]
    fn orchestrator_pause_resume_stop_and_skip_phase() {
        let mut run = RunState::new("r1", RunMode::Brainstorm, members(), 0);
        run.pause();
        assert_eq!(run.status, RunStatus::Paused);
        run.resume();
        assert_eq!(run.status, RunStatus::Running);
        assert_eq!(run.phase(), RunPhase::Plan);
        run.skip_phase();
        assert_eq!(run.phase(), RunPhase::Brainstorm);
        run.stop();
        assert_eq!(run.status, RunStatus::Stopped);
        // Stopped runs do not resume by accident.
        run.resume();
        assert_eq!(run.status, RunStatus::Stopped);
        // Membership + budget controls.
        run.set_budget_usd(5.0);
        assert_eq!(run.limits.budget_usd, 5.0);
        run.add_member("zoe");
        assert!(run.members.contains(&"zoe".to_string()));
        run.remove_member("zoe");
        assert!(!run.members.contains(&"zoe".to_string()));
    }

    #[test]
    fn orchestrator_interrupted_run_resumes_from_snapshot() {
        let mut run = RunState::new("r1", RunMode::DeepResearch, members(), 1000);
        run.plan = "read then write".into();
        run.record_usage(1000, 500, 0.2);
        run.advance_phase(); // Research
        let snap = run.snapshot();
        assert_eq!(snap.status, RunStatus::Interrupted);
        assert_eq!(snap.phase, RunPhase::Research);
        // JSON round-trip (SQLite persistence shape).
        let json = serde_json::to_string(&snap).unwrap();
        let back: RunSnapshot = serde_json::from_str(&json).unwrap();
        let mut restored = RunState::restore(back);
        assert_eq!(restored.phase(), RunPhase::Research);
        assert_eq!(restored.plan, "read then write");
        assert_eq!(restored.ledger.spent_usd, 0.2);
        assert_eq!(restored.status, RunStatus::Interrupted);
        restored.resume();
        assert_eq!(restored.status, RunStatus::Running);
        restored.advance_phase();
        assert_eq!(restored.phase(), RunPhase::Analysis);
    }

    #[test]
    fn orchestrator_concurrency_turns_expiry_and_tool_budget() {
        let run = RunState::new("r1", RunMode::DeepResearch, members(), 0);
        assert!(!run.is_parallel()); // starts at Plan
        assert_eq!(run.concurrency(), 3);
        let solo = RunState::new("r2", RunMode::Brainstorm, vec!["lead".into()], 0);
        assert_eq!(solo.concurrency(), 1);
        // Wall clock: 30 min default.
        assert!(!run.is_expired(1799));
        assert!(run.is_expired(1800));
        // Turn limit ends a discussion phase.
        let mut run3 = RunState::new("r3", RunMode::Brainstorm, members(), 0);
        for _ in 0..11 {
            assert_eq!(run3.record_turn("mira"), Convergence::Pending);
        }
        assert_eq!(run3.record_turn("mira"), Convergence::TurnLimit);
        // Per-agent tool calls: 25 per sub-task.
        let limits = RunLimits::default();
        let mut used = HashMap::new();
        used.insert("mira".to_string(), 24);
        assert!(RunState::tool_calls_allowed(&used, "mira", &limits));
        used.insert("mira".to_string(), 25);
        assert!(!RunState::tool_calls_allowed(&used, "mira", &limits));
    }
}
