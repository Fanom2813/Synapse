//! One user message, handled by the team (docs/TEAM-PROTOCOL.md).
//!
//! The team is a meeting in one shared chat. The lead (or the @mentioned
//! member) replies to the user. A reply may end with `<ask agent="Name">`
//! blocks that ping teammates: one, several, or none. Everyone pinged speaks
//! next, together, each seeing the whole chat; their replies may ping others
//! (a critic sends a gap back to the researcher, the researcher answers to
//! the analyst), and so on until a round pings nobody. That is the end of the
//! meeting. Every speaker streams into the chat under their own name; the
//! view shows the first speaker (the orchestrator, or the member the user
//! @mentioned) in full and everyone else as a collapsed working note.
//!
//! Bounded (`Limits`): pings per meeting and per reply, turns per member,
//! model calls at once. Dropping the task (the user pressed Stop) cancels
//! every model call in flight.

use futures::future::join_all;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use synapse_core::team_dialogue::{
    find_member, parse_asks, team_prompt, turn_note, visible_text, Ask, Limits,
    Member,
};
use synapse_llm::{AgentEvent, LlmError, ModelBackend, Role, TurnMessage, TurnRequest, Usage};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;

/// What the chat view shows, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum TeamEvent {
    /// A member starts writing; show a new message under their name.
    Start { stream: u32, speaker: String, lead: bool },
    /// More of that message. May contain `<ask>` blocks (hidden by the view).
    Delta { stream: u32, text: String },
    /// Tokens reported for that message.
    Usage { stream: u32, usage: Usage },
    /// That message is complete.
    Finish { stream: u32 },
    /// `from` asked `to` to do something (shown as an avatar under `from`'s
    /// message, with the request on hover).
    Ping { from: String, to: String, body: String },
}

/// The backend and model id that run a member, or `None` when they have no
/// model (nothing picked in Settings → Models yet).
pub type Resolver<B> = Arc<dyn Fn(&str) -> Option<(Arc<B>, String)> + Send + Sync>;

pub struct TeamEnv<B> {
    pub project: String,
    pub goal: String,
    pub members: Vec<Member>,
    pub lead_id: String,
    pub resolve: Resolver<B>,
    pub limits: Limits,
}

struct Run<B> {
    env: Arc<TeamEnv<B>>,
    /// The member the user is talking to; the view shows their messages in
    /// full and everyone else's as collapsed working notes.
    voice: String,
    tx: mpsc::Sender<TeamEvent>,
    next_stream: AtomicU32,
    calls: Semaphore,
}

/// A spawned task that is cancelled when dropped.
struct Guarded<T>(Option<JoinHandle<T>>);

impl<T> Drop for Guarded<T> {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

/// What the lead is told when the meeting goes quiet before they answered.
const CLOSING_NOTE: &str = "The room has gone quiet. Check what has been done against what the user asked for. \
If something is still missing or short of what was asked, ping the people who can do it now. \
Otherwise give the user the final answer: the actual deliverable, complete, saying who contributed.";

const NO_MODEL: &str =
    "This teammate has no model yet. Pick one in Settings → Models, or set their engine in Settings → Agents.";


/// One line of the shared chat.
#[derive(Clone)]
struct Line {
    /// Member id, or "user" for the user's own lines.
    who: String,
    name: String,
    /// What the speaker wrote, as they wrote it (their memory of it).
    raw: String,
    /// What the others read: the reply without the block, plus the handoff.
    shown: String,
}

/// Run the team on `history` (ending with the user's message). `speaker_id`
/// is the lead, or the member the user @mentioned.
///
/// A meeting in waves: everyone pinged speaks together (each sees the chat as
/// it stood when the wave began), their replies join the chat, and whoever
/// they pinged speaks next. It ends when a wave pings nobody.
pub async fn run_team_turn<B: ModelBackend + 'static>(
    env: Arc<TeamEnv<B>>,
    speaker_id: &str,
    history: Vec<TurnMessage>,
    tx: mpsc::Sender<TeamEvent>,
) -> Result<(), LlmError> {
    let first = find_member(&env.members, speaker_id)
        .or_else(|| find_member(&env.members, &env.lead_id))
        .cloned()
        .ok_or_else(|| LlmError::BadRequest("The team has no members.".into()))?;
    let run = Run {
        voice: first.id.clone(),
        calls: Semaphore::new(env.limits.parallel_calls.max(1)),
        env,
        tx,
        next_stream: AtomicU32::new(0),
    };
    let lead_spoke_first = first.id == run.env.lead_id;
    let mut chat = chat_from(&run.env, &history);
    let mut turns: HashMap<String, usize> = HashMap::new();
    let mut hops = 0;
    let mut handed_off = false;
    // Who speaks in the next wave, each with what they were pinged about.
    let mut wave: Vec<(Member, Option<String>)> = vec![(first.clone(), None)];
    let mut first_wave = true;
    let mut closings = 0;
    while !wave.is_empty() {
        let can_hand_off = hops < run.env.limits.hops;
        let snapshot = chat.clone();
        for (me, _) in &wave {
            *turns.entry(me.id.clone()).or_default() += 1;
        }
        let results = join_all(wave.iter().map(|(me, note)| {
            let (run, snapshot) = (&run, &snapshot);
            async move { (me, turn_of(run, me, snapshot, note.as_deref(), can_hand_off).await) }
        }))
        .await;

        let mut next: Vec<(Member, Vec<String>)> = Vec::new();
        for (me, result) in results {
            let raw = match result {
                Ok(raw) => raw,
                // The very first speaker failing is the user's problem; a
                // teammate failing mid-meeting is just something the others see.
                Err(e) if first_wave => return Err(e),
                Err(e) => format!("(could not answer: {})", e.user_message()),
            };
            let visible = visible_text(&raw);
            let mut shown = visible;
            let asks = if can_hand_off { parse_asks(&raw) } else { Vec::new() };
            for ask in asks.into_iter().take(run.env.limits.pings_per_reply) {
                match pick(&run, me, &ask, &turns) {
                    Ok(target) => {
                        hops += 1;
                        handed_off = true;
                        let ping = TeamEvent::Ping { from: me.name.clone(), to: target.name.clone(), body: ask.body.clone() };
                        let _ = run.tx.send(ping).await;
                        shown.push_str(&format!("\n[asked {}: {}]", target.name, ask.body));
                        let note = turn_note(&me.name, &ask);
                        match next.iter_mut().find(|(m, _)| m.id == target.id) {
                            Some((_, notes)) => notes.push(note),
                            None => next.push((target, vec![note])),
                        }
                    }
                    Err(reason) => {
                        // Tell the sender why, as their next turn.
                        shown.push_str(&format!("\n[could not ask {}: {reason}]", ask.agent));
                        match next.iter_mut().find(|(m, _)| m.id == me.id) {
                            Some((_, notes)) => notes.push(format!("Handoff not made: {reason}")),
                            None => next.push((me.clone(), vec![format!("Handoff not made: {reason}")])),
                        }
                    }
                }
            }
            chat.push(Line { who: me.id.clone(), name: me.name.clone(), raw, shown });
        }
        first_wave = false;
        wave = next.into_iter().map(|(m, notes)| (m, Some(notes.join("\n\n")))).collect();
        let capped = hops >= run.env.limits.hops + run.env.members.len();
        // The room went quiet but the lead did not have the last word: they
        // check the work against the request, send people back or answer.
        let lead_last = chat.last().is_some_and(|l| l.who == run.env.lead_id);
        if (wave.is_empty() || capped) && lead_spoke_first && handed_off && !lead_last && closings < 2 {
            closings += 1;
            wave = vec![(first.clone(), Some(CLOSING_NOTE.to_string()))];
        } else if capped {
            break;
        }
    }
    Ok(())
}

/// One member's turn in a wave. Returns what they wrote, raw.
async fn turn_of<B: ModelBackend + 'static>(
    run: &Run<B>,
    me: &Member,
    chat: &[Line],
    note: Option<&str>,
    can_hand_off: bool,
) -> Result<String, LlmError> {
    let env = &run.env;
    let (backend, model) = (env.resolve)(&me.id).ok_or_else(|| LlmError::BadRequest(NO_MODEL.into()))?;
    let is_lead = me.id == env.lead_id;
    let system = team_prompt(&env.project, &env.goal, &env.members, me, is_lead, can_hand_off);
    speak(run, &backend, &model, &system, &view_for(me, chat, note), me, is_lead || me.id == run.voice).await
}

/// The member an ask names, if they may be pinged.
fn pick<B>(run: &Run<B>, from: &Member, ask: &Ask, turns: &HashMap<String, usize>) -> Result<Member, String> {
    let env = &run.env;
    match find_member(&env.members, &ask.agent) {
        None => {
            let names: Vec<&str> = env.members.iter().map(|m| m.name.as_str()).collect();
            Err(format!("There is no teammate named \"{}\". The team: {}.", ask.agent, names.join(", ")))
        }
        Some(t) if t.id == from.id => Err("You cannot ask yourself.".to_string()),
        Some(t) if turns.get(&t.id).copied().unwrap_or(0) >= env.limits.turns_each => Err(format!(
            "{} has already spoken {} times on this; the discussion is going in circles. Settle it yourselves or wrap up for the user.",
            t.name, env.limits.turns_each
        )),
        Some(t) => Ok(t.clone()),
    }
}

/// The chat so far as lines: the user's, the lead's earlier replies and what
/// teammates reported earlier (already quoted in `history`).
fn chat_from<B>(env: &TeamEnv<B>, history: &[TurnMessage]) -> Vec<Line> {
    let lead = find_member(&env.members, &env.lead_id).map(|m| m.name.clone()).unwrap_or_default();
    history
        .iter()
        .map(|m| match m.role {
            Role::Assistant => Line {
                who: env.lead_id.clone(),
                name: lead.clone(),
                raw: m.text.clone(),
                shown: m.text.clone(),
            },
            _ => Line { who: "user".into(), name: String::new(), raw: m.text.clone(), shown: m.text.clone() },
        })
        .collect()
}

/// The chat as `me` reads it: their own lines are theirs, everyone else's are
/// quoted with a name. `note` (the handoff, or why one failed) comes last.
fn view_for(me: &Member, chat: &[Line], note: Option<&str>) -> Vec<TurnMessage> {
    let mut out: Vec<TurnMessage> = chat
        .iter()
        .map(|l| {
            if l.who == me.id {
                TurnMessage::assistant(l.raw.clone())
            } else if l.name.is_empty() {
                TurnMessage::user(l.shown.clone())
            } else {
                TurnMessage::user(format!("[{}]\n{}", l.name, l.shown))
            }
        })
        .collect();
    if let Some(note) = note {
        out.push(TurnMessage::user(note.to_string()));
    }
    merge_user_turns(out)
}

/// Consecutive user turns joined into one, for providers that want turns to
/// alternate.
fn merge_user_turns(messages: Vec<TurnMessage>) -> Vec<TurnMessage> {
    let mut out: Vec<TurnMessage> = Vec::new();
    for m in messages {
        match out.last_mut() {
            Some(last) if matches!(last.role, Role::User) && matches!(m.role, Role::User) => {
                last.text = format!("{}\n\n{}", last.text, m.text);
            }
            _ => out.push(m),
        }
    }
    out
}

/// One model call, streamed to the chat. Returns the raw text.
async fn speak<B: ModelBackend + 'static>(
    run: &Run<B>,
    backend: &Arc<B>,
    model: &str,
    system: &str,
    messages: &[TurnMessage],
    speaker: &Member,
    lead: bool,
) -> Result<String, LlmError> {
    let stopped = || LlmError::Stream("stopped".into());
    let stream = run.next_stream.fetch_add(1, Ordering::Relaxed);
    let start = TeamEvent::Start { stream, speaker: speaker.name.clone(), lead };
    run.tx.send(start).await.map_err(|_| stopped())?;
    let _slot = run.calls.acquire().await.map_err(|_| stopped())?;

    let mut req = TurnRequest::new(model);
    req.system = Some(system.to_string());
    req.messages = messages.to_vec();
    let (sink, mut events) = mpsc::channel(64);
    let backend = backend.clone();
    let mut call = Guarded(Some(tokio::spawn(async move { backend.turn_live(req, sink).await })));

    let mut raw = String::new();
    while let Some(event) = events.recv().await {
        let out = match event {
            AgentEvent::TextDelta(text) => {
                raw.push_str(&text);
                Some(TeamEvent::Delta { stream, text })
            }
            AgentEvent::Usage(usage) | AgentEvent::Done { usage, .. } if usage != Usage::default() => {
                Some(TeamEvent::Usage { stream, usage })
            }
            _ => None,
        };
        if let Some(out) = out {
            run.tx.send(out).await.map_err(|_| stopped())?;
        }
    }
    let result = match call.0.as_mut() {
        Some(handle) => handle.await,
        None => return Err(stopped()),
    };
    let _ = run.tx.send(TeamEvent::Finish { stream }).await;
    result.map_err(|e| LlmError::Stream(e.to_string()))??;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use synapse_llm::{MockLlm, TurnOutput};

    fn member(id: &str, name: &str, role: &str) -> Member {
        Member {
            id: id.into(),
            name: name.into(),
            role: role.into(),
            brief: role.into(),
            instructions: String::new(),
            avatar: String::new(),
        }
    }

    fn output(text: &str) -> TurnOutput {
        TurnOutput { text: text.into(), tool_calls: Vec::new(), usage: Usage::default() }
    }

    #[tokio::test]
    async fn the_lead_asks_a_teammate_gets_the_answer_and_replies_to_the_user() {
        let mock = Arc::new(MockLlm::with_outputs(vec![
            output("Checking with Mira.\n<ask agent=\"Mira\">Task: size the market</ask>"),
            output("Market is about 5M users.\n<ask agent=\"Lead\">Please wrap up for the user.</ask>"),
            output("Mira found the market is about 5M users."),
        ]));
        let backend = mock.clone();
        let env = Arc::new(TeamEnv {
            project: "Fintech".into(),
            goal: "Find a niche".into(),
            members: vec![member("lead", "Lead", "Chief of staff"), member("a1", "Mira", "Researcher")],
            lead_id: "lead".into(),
            resolve: Arc::new(move |_| Some((backend.clone(), "m".into()))),
            limits: Limits::default(),
        });
        let (tx, mut rx) = mpsc::channel(256);
        let history = vec![TurnMessage::user("how big is the market?")];
        run_team_turn(env, "lead", history, tx).await.expect("turn");

        let mut speakers = Vec::new();
        let mut text = std::collections::HashMap::<u32, String>::new();
        let mut notes = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                TeamEvent::Start { stream, speaker, .. } => speakers.push((stream, speaker)),
                TeamEvent::Delta { stream, text: t } => text.entry(stream).or_default().push_str(&t),
                TeamEvent::Ping { from, to, .. } => notes.push(format!("{from} → {to}")),
                _ => {}
            }
        }
        let names: Vec<&str> = speakers.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["Lead", "Mira", "Lead"]);
        assert_eq!(notes[0], "Lead → Mira");
        assert!(text[&1].starts_with("Market is about 5M users."));
        assert!(text[&2].contains("Mira found"));

        // Mira saw the project, the team and the whole chat, plus the handoff.
        let seen = mock.seen_requests();
        let mira_system = seen[1].system.clone().unwrap();
        assert!(mira_system.contains("Find a niche") && mira_system.contains("- Lead:"));
        let mira_view = &seen[1].messages;
        assert!(mira_view[0].text.contains("how big is the market?"));
        assert!(mira_view.last().unwrap().text.contains("size the market"));
        // The lead's second call carries Mira's answer.
        assert!(seen[2].messages.iter().any(|m| m.text.contains("[Mira]")));
    }

    #[tokio::test]
    async fn the_lead_gets_the_last_word_when_a_worker_does_not_ping_back() {
        let mock = Arc::new(MockLlm::with_outputs(vec![
            output("<ask agent=\"Mira\">find data</ask>"),
            output("Here is the data."),
            output("Final answer from the lead."),
        ]));
        let backend = mock.clone();
        let env = Arc::new(TeamEnv {
            project: "P".into(),
            goal: "G".into(),
            members: vec![member("lead", "Lead", "Chief"), member("a1", "Mira", "Researcher")],
            lead_id: "lead".into(),
            resolve: Arc::new(move |_| Some((backend.clone(), "m".into()))),
            limits: Limits::default(),
        });
        let (tx, mut rx) = mpsc::channel(256);
        run_team_turn(env, "lead", vec![TurnMessage::user("do it")], tx).await.expect("turn");
        let mut names = Vec::new();
        while let Some(event) = rx.recv().await {
            if let TeamEvent::Start { speaker, .. } = event {
                names.push(speaker);
            }
        }
        assert_eq!(names, ["Lead", "Mira", "Lead"]);
        assert!(mock.seen_requests()[2].messages.last().unwrap().text.contains("gone quiet"));
    }

    #[tokio::test]
    async fn two_pinged_members_speak_together_and_can_ping_each_other_after() {
        let mock = Arc::new(MockLlm::with_outputs(vec![
            output("<ask agent=\"Mira\">find data</ask><ask agent=\"Leo\">find risks</ask>"),
            output("Data found."),
            output("Risks found."),
            output("Summary for the user."),
        ]));
        let backend = mock.clone();
        let env = Arc::new(TeamEnv {
            project: "P".into(),
            goal: String::new(),
            members: vec![member("lead", "Lead", ""), member("a1", "Mira", ""), member("a2", "Leo", "")],
            lead_id: "lead".into(),
            resolve: Arc::new(move |_| Some((backend.clone(), "m".into()))),
            limits: Limits::default(),
        });
        let (tx, mut rx) = mpsc::channel(256);
        run_team_turn(env, "lead", vec![TurnMessage::user("research it")], tx).await.expect("turn");
        let mut names = Vec::new();
        while let Some(event) = rx.recv().await {
            if let TeamEvent::Start { speaker, .. } = event {
                names.push(speaker);
            }
        }
        assert_eq!(names[0], "Lead");
        names[1..3].sort();
        assert_eq!(names[1..3], ["Leo", "Mira"]);
        assert_eq!(names[3], "Lead");
        // Both saw the same chat when they began.
        let seen = mock.seen_requests();
        assert_eq!(seen[1].messages.len(), seen[2].messages.len());
    }

    #[tokio::test]
    async fn unknown_names_and_repeats_are_answered_not_run() {
        let mock = Arc::new(MockLlm::with_outputs(vec![
            output("<ask agent=\"Nobody\">x</ask>"),
            output("Done without them."),
        ]));
        let backend = mock.clone();
        let env = Arc::new(TeamEnv {
            project: "P".into(),
            goal: String::new(),
            members: vec![member("lead", "Lead", "")],
            lead_id: "lead".into(),
            resolve: Arc::new(move |_| Some((backend.clone(), "m".into()))),
            limits: Limits::default(),
        });
        let (tx, mut rx) = mpsc::channel(64);
        run_team_turn(env, "lead", vec![TurnMessage::user("hi")], tx).await.expect("turn");
        while rx.recv().await.is_some() {}
        let seen = mock.seen_requests();
        assert!(seen[1].messages.last().unwrap().text.contains("no teammate named \"Nobody\""));
    }
}
