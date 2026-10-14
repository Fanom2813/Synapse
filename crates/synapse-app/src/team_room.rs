//! Team room view state (see docs/PRD.md §5.1, docs/UI.md §1–3, docs/TEAM-PROTOCOL.md).
//!
//! Pure state over [`Envelope`] history: typed message styles, threaded
//! replies, per-agent live state, coalesced streaming deltas, `@mentions`,
//! user interjection, run controls (pause/resume/stop/skip/budget/members)
//! and the three-layer detail toggle (Summary / Conversation / Raw).
//! The GPUI view renders this; tests drive the state without GPUI.

use std::collections::HashMap;

use synapse_core::{Envelope, Kind, Participant, Recipients, RunMode, RunState, RunStatus};

/// Live agent state for the team strip (docs/UI.md §4 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoomAgentState {
    #[default]
    Idle,
    Thinking,
    Speaking,
    Working,
    NeedsYou,
    Paused,
    Error,
}

impl RoomAgentState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Thinking => "thinking",
            Self::Speaking => "speaking",
            Self::Working => "working",
            Self::NeedsYou => "needs you",
            Self::Paused => "paused",
            Self::Error => "error",
        }
    }

    pub fn from_event_state(s: &str) -> Self {
        match s {
            "thinking" => Self::Thinking,
            "speaking" | "streaming" => Self::Speaking,
            "working" => Self::Working,
            "needs-you" | "needs_you" | "need_user" => Self::NeedsYou,
            "paused" => Self::Paused,
            "error" => Self::Error,
            _ => Self::Idle,
        }
    }
}

/// One row in the room.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomMessage {
    pub envelope: Envelope,
    pub streaming: bool,
}

impl RoomMessage {
    /// Display author: user name, agent id, or "System".
    pub fn author(&self) -> String {
        match &self.envelope.from {
            Participant::User => "You".to_string(),
            Participant::System => "System".to_string(),
            Participant::Agent(id) => id.clone(),
        }
    }

    /// Short style key for the typed message look (docs/UI.md §3).
    pub fn style(&self) -> &'static str {
        message_style(&self.envelope.kind)
    }

    /// Header line for kinds that address someone (e.g. Ask → "@agent").
    pub fn address_header(&self) -> Option<String> {
        match &self.envelope.to {
            Recipients::Agent(id) => Some(format!("@{id}")),
            Recipients::Agents(ids) => Some(
                ids.iter()
                    .map(|id| format!("@{id}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            Recipients::Lead => Some("@lead".to_string()),
            Recipients::User => Some("@you".to_string()),
            Recipients::Room => None,
        }
    }
}

/// Typed message style key (docs/UI.md §3 table).
pub fn message_style(kind: &Kind) -> &'static str {
    match kind {
        Kind::Say => "say",
        Kind::Ask { .. } => "ask",
        Kind::Answer => "answer",
        Kind::Challenge { .. } => "challenge",
        Kind::Agree { .. } => "agree",
        Kind::Refuse { .. } => "refuse",
        Kind::Delegate { .. } => "delegate",
        Kind::Finding { .. } => "finding",
        Kind::Propose { .. } => "propose",
        Kind::Vote { .. } => "vote",
        Kind::NeedUser { .. } => "need-user",
        Kind::Status { .. } => "status",
        Kind::Steer { .. } => "steer",
        Kind::Done { .. } => "done",
    }
}

/// Detail layers (docs/UI.md §3): Summary shows only findings/decisions,
/// Conversation is the default full room, Raw adds tool/usage metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailLevel {
    Summary,
    #[default]
    Conversation,
    Raw,
}

impl DetailLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Conversation => "Conversation",
            Self::Raw => "Raw",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Summary => Self::Conversation,
            Self::Conversation => Self::Raw,
            Self::Raw => Self::Summary,
        }
    }
}

fn summary_visible(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Finding { .. }
            | Kind::Propose { .. }
            | Kind::Vote { .. }
            | Kind::NeedUser { .. }
            | Kind::Done { .. }
    )
}

/// Extract `@mentions` of known members from composer/body text.
pub fn extract_mentions(text: &str, members: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut token = String::new();
    let mut in_mention = false;
    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch == '@' {
            in_mention = true;
            token.clear();
            continue;
        }
        if in_mention {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                token.push(ch);
            } else {
                if !token.is_empty() {
                    let lower = token.to_lowercase();
                    if let Some(hit) = members.iter().find(|m| m.to_lowercase() == lower) {
                        if !out.contains(hit) {
                            out.push(hit.clone());
                        }
                    }
                    token.clear();
                }
                in_mention = false;
            }
        }
    }
    out
}

/// Owned team-room state.
#[derive(Debug)]
pub struct TeamRoomState {
    conversation_id: String,
    run: RunState,
    members: Vec<RoomMember>,
    messages: Vec<RoomMessage>,
    pending: HashMap<String, String>,
    detail: DetailLevel,
    draft: String,
    queued: Vec<String>,
    last_error: Option<String>,
    next_seq: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomMember {
    pub id: String,
    pub role: String,
    pub state: RoomAgentState,
}

impl TeamRoomState {
    pub fn new(conversation_id: &str, mode: RunMode, members: Vec<(String, String)>) -> Self {
        let ids: Vec<String> = members.iter().map(|(id, _)| id.clone()).collect();
        let run = RunState::new("run-1", mode, ids, 0);
        Self {
            conversation_id: conversation_id.to_string(),
            run,
            members: members
                .into_iter()
                .map(|(id, role)| RoomMember {
                    id,
                    role,
                    state: RoomAgentState::Idle,
                })
                .collect(),
            messages: Vec::new(),
            pending: HashMap::new(),
            detail: DetailLevel::Conversation,
            draft: String::new(),
            queued: Vec::new(),
            last_error: None,
            next_seq: 1,
        }
    }

    pub fn messages(&self) -> &[RoomMessage] {
        &self.messages
    }

    pub fn detail(&self) -> DetailLevel {
        self.detail
    }

    pub fn set_detail(&mut self, level: DetailLevel) {
        self.detail = level;
    }

    pub fn draft(&self) -> &str {
        &self.draft
    }

    pub fn set_draft(&mut self, draft: impl Into<String>) {
        self.draft = draft.into();
    }

    pub fn queued_followups(&self) -> &[String] {
        &self.queued
    }

    pub fn run_status(&self) -> RunStatus {
        self.run.status
    }

    pub fn run_phase(&self) -> synapse_core::RunPhase {
        self.run.phase()
    }

    pub fn run(&self) -> &RunState {
        &self.run
    }

    pub fn run_mut(&mut self) -> &mut RunState {
        &mut self.run
    }

    pub fn member_state(&self, id: &str) -> Option<RoomAgentState> {
        self.members.iter().find(|m| m.id == id).map(|m| m.state)
    }

    pub fn set_agent_state(&mut self, agent: &str, state: RoomAgentState) {
        if let Some(m) = self.members.iter_mut().find(|m| m.id == agent) {
            m.state = state;
        } else {
            self.members.push(RoomMember {
                id: agent.to_string(),
                role: String::new(),
                state,
            });
        }
    }

    pub fn apply_agent_state_event(&mut self, agent: &str, state: &str) {
        self.set_agent_state(agent, RoomAgentState::from_event_state(state));
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn set_error(&mut self, err: impl Into<String>) {
        self.last_error = Some(err.into());
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Empty-room headline + primary action (docs/UI.md §2.2).
    pub fn empty_state(&self) -> Option<(&'static str, &'static str)> {
        if self.is_empty() {
            Some(("Describe the goal to start", "Start a run"))
        } else {
            None
        }
    }

    pub fn item_count(&self) -> usize {
        self.visible_messages().len()
    }

    /// Messages after the detail filter.
    pub fn visible_messages(&self) -> Vec<&RoomMessage> {
        match self.detail {
            DetailLevel::Summary => self
                .messages
                .iter()
                .filter(|m| !m.streaming && summary_visible(&m.envelope.kind))
                .collect(),
            DetailLevel::Conversation | DetailLevel::Raw => self.messages.iter().collect(),
        }
    }

    /// Direct replies to a message id (threaded view).
    pub fn replies_to(&self, id: &str) -> Vec<&RoomMessage> {
        self.messages
            .iter()
            .filter(|m| m.envelope.reply_to.as_deref() == Some(id))
            .collect()
    }

    /// Find the message a reply quotes (challenge/agree jump target).
    pub fn parent_of(&self, id: &str) -> Option<&RoomMessage> {
        let reply_to = self
            .messages
            .iter()
            .find(|m| m.envelope.id == id)?
            .envelope
            .reply_to
            .clone()?;
        self.messages.iter().find(|m| m.envelope.id == reply_to)
    }

    /// Mentioned members across the visible history (selector input).
    pub fn mentioned_in(&self, body: &str) -> Vec<String> {
        let ids: Vec<String> = self.members.iter().map(|m| m.id.clone()).collect();
        extract_mentions(body, &ids)
    }

    fn alloc_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}-{}", self.next_seq);
        self.next_seq += 1;
        id
    }

    /// Append a finished envelope (persisted order). Replaces a streaming
    /// placeholder with the same id.
    pub fn push_envelope(&mut self, envelope: Envelope) {
        if let Some(slot) = self
            .messages
            .iter_mut()
            .find(|m| m.envelope.id == envelope.id)
        {
            slot.envelope = envelope;
            slot.streaming = false;
            self.pending.remove(&slot.envelope.id.clone());
            return;
        }
        // Agent starts speaking: hop state.
        if let Participant::Agent(id) = &envelope.from {
            self.set_agent_state(id, RoomAgentState::Speaking);
        }
        if matches!(envelope.kind, Kind::NeedUser { .. }) {
            if let Participant::Agent(id) = &envelope.from {
                self.set_agent_state(id, RoomAgentState::NeedsYou);
            }
        }
        self.messages.push(RoomMessage {
            envelope,
            streaming: false,
        });
    }

    /// Start a streaming placeholder for an agent turn.
    pub fn begin_stream(
        &mut self,
        id: impl Into<String>,
        from: Participant,
        to: Recipients,
    ) -> String {
        let id = id.into();
        self.set_streaming_state(&from);
        self.messages.push(RoomMessage {
            envelope: Envelope {
                id: id.clone(),
                run_id: self.run.id.clone(),
                conversation_id: self.conversation_id.clone(),
                thread_id: String::new(),
                reply_to: None,
                from,
                to,
                kind: Kind::Say,
                body: String::new(),
                citations: Vec::new(),
                attachments: Vec::new(),
                priority: synapse_core::Priority::Normal,
                usage: None,
                seq: 0,
            },
            streaming: true,
        });
        id
    }

    fn set_streaming_state(&mut self, from: &Participant) {
        if let Participant::Agent(id) = from {
            self.set_agent_state(id, RoomAgentState::Speaking);
        }
    }

    /// Buffer a text delta; coalesced until [`Self::flush_frame`].
    pub fn push_delta(&mut self, id: &str, delta: &str) {
        if self.messages.iter().any(|m| m.envelope.id == id) {
            self.pending
                .entry(id.to_string())
                .or_default()
                .push_str(delta);
        }
    }

    /// Apply buffered deltas: at most one body update per message per frame.
    pub fn flush_frame(&mut self) -> Vec<String> {
        let mut changed = Vec::new();
        let pending = std::mem::take(&mut self.pending);
        for (id, text) in pending {
            if let Some(m) = self.messages.iter_mut().find(|m| m.envelope.id == id) {
                m.body_mut().push_str(&text);
                changed.push(id);
            }
        }
        changed
    }

    /// Finish a stream with its final envelope (replaces the placeholder).
    pub fn finish_stream(&mut self, envelope: Envelope) {
        self.push_envelope(envelope);
        if let Some(last) = self.messages.last() {
            if let Participant::Agent(id) = &last.envelope.from {
                let id = id.clone();
                self.set_agent_state(&id, RoomAgentState::Idle);
            }
        }
    }

    /// User interjection at any moment: posts to the room immediately and
    /// routes `@mentions` to the selector. While paused, queues as follow-up.
    pub fn interject(&mut self, body: &str) -> (String, Vec<String>) {
        let body = body.trim().to_string();
        let id = self.alloc_id("user");
        let ids: Vec<String> = self.members.iter().map(|m| m.id.clone()).collect();
        let mentioned = extract_mentions(&body, &ids);
        if self.run.status == RunStatus::Paused {
            self.queued.push(body.clone());
        }
        let env = Envelope {
            id: id.clone(),
            run_id: self.run.id.clone(),
            conversation_id: self.conversation_id.clone(),
            thread_id: String::new(),
            reply_to: None,
            from: Participant::User,
            to: Recipients::Room,
            kind: Kind::Say,
            body,
            citations: Vec::new(),
            attachments: Vec::new(),
            priority: synapse_core::Priority::Normal,
            usage: None,
            seq: 0,
        };
        self.messages.push(RoomMessage {
            envelope: env,
            streaming: false,
        });
        (id, mentioned)
    }

    /// Composer send: interjects the draft, or queues it while a run streams.
    pub fn send_draft(&mut self, streaming: bool) -> Option<(String, Vec<String>)> {
        let body = self.draft.trim().to_string();
        if body.is_empty() {
            return None;
        }
        if streaming {
            self.queued.push(body);
            self.draft.clear();
            return None;
        }
        self.draft.clear();
        Some(self.interject(&body))
    }

    // -- run controls (TEAM-PROTOCOL.md §9) --

    pub fn pause_run(&mut self) {
        self.run.pause();
        for m in &mut self.members {
            if m.state == RoomAgentState::Speaking
                || m.state == RoomAgentState::Thinking
                || m.state == RoomAgentState::Working
            {
                m.state = RoomAgentState::Paused;
            }
        }
    }

    pub fn resume_run(&mut self) {
        self.run.resume();
        for m in &mut self.members {
            if m.state == RoomAgentState::Paused {
                m.state = RoomAgentState::Idle;
            }
        }
    }

    pub fn stop_run(&mut self) {
        self.run.stop();
        self.pending.clear();
        for m in &mut self.messages {
            m.streaming = false;
        }
        for m in &mut self.members {
            m.state = RoomAgentState::Idle;
        }
    }

    pub fn skip_phase(&mut self) {
        self.run.skip_phase();
    }

    pub fn set_budget(&mut self, usd: f64) {
        self.run.set_budget_usd(usd);
    }

    pub fn add_member(&mut self, id: &str, role: &str) {
        self.run.add_member(id);
        if !self.members.iter().any(|m| m.id == id) {
            self.members.push(RoomMember {
                id: id.to_string(),
                role: role.to_string(),
                state: RoomAgentState::Idle,
            });
        }
    }

    pub fn remove_member(&mut self, id: &str) {
        self.run.remove_member(id);
        self.members.retain(|m| m.id != id);
    }

    /// Graph of members and Ask/Delegate/Challenge edges for the map panel.
    pub fn conversation_map(&self) -> crate::conversation_map::ConversationMap {
        let envelopes: Vec<Envelope> = self.messages.iter().map(|m| m.envelope.clone()).collect();
        let members: Vec<(String, String)> = self
            .members
            .iter()
            .map(|m| (m.id.clone(), m.role.clone()))
            .collect();
        crate::conversation_map::ConversationMap::build(&envelopes, &members)
    }

    pub fn send_label(&self) -> &'static str {
        if self.run.status == RunStatus::Running {
            "Stop"
        } else {
            "Send"
        }
    }
}

trait BodyMut {
    fn body_mut(&mut self) -> &mut String;
}

impl BodyMut for RoomMessage {
    fn body_mut(&mut self) -> &mut String {
        &mut self.envelope.body
    }
}

#[cfg(test)]
mod team_room_tests {
    use super::*;
    use synapse_core::{Kind, Participant, Recipients};

    fn room() -> TeamRoomState {
        TeamRoomState::new(
            "c1",
            RunMode::Brainstorm,
            vec![
                ("lead".into(), "Lead".into()),
                ("mira".into(), "Researcher".into()),
                ("ivan".into(), "Critic".into()),
            ],
        )
    }

    fn say(id: &str, from: &str, body: &str) -> Envelope {
        let mut e = Envelope::say(
            id,
            "c1",
            Participant::Agent(from.into()),
            Recipients::Room,
            body,
        );
        e.run_id = "run-1".into();
        e
    }

    #[test]
    fn team_room_typed_messages_have_distinct_styles() {
        let mut r = room();
        let mk = |id: &str, kind: Kind| Envelope {
            id: id.into(),
            run_id: "run-1".into(),
            conversation_id: "c1".into(),
            thread_id: String::new(),
            reply_to: None,
            from: Participant::Agent("mira".into()),
            to: Recipients::Room,
            kind,
            body: "body".into(),
            citations: Vec::new(),
            attachments: Vec::new(),
            priority: synapse_core::Priority::Normal,
            usage: None,
            seq: 0,
        };
        r.push_envelope(mk("k1", Kind::Say));
        r.push_envelope(mk(
            "k2",
            Kind::Challenge {
                claim_ref: "k1".into(),
            },
        ));
        r.push_envelope(mk("k3", Kind::Finding { confidence: 0.8 }));
        r.push_envelope(mk("k4", Kind::Answer));
        assert_eq!(r.messages()[0].style(), "say");
        assert_eq!(r.messages()[1].style(), "challenge");
        assert_eq!(r.messages()[2].style(), "finding");
        assert_eq!(r.messages()[3].style(), "answer");
        // Ask shows who it addresses.
        let mut ask = mk(
            "k5",
            Kind::Ask {
                question: "q".into(),
            },
        );
        ask.to = Recipients::Agent("ivan".into());
        r.push_envelope(ask);
        assert_eq!(r.messages()[4].address_header().as_deref(), Some("@ivan"));
    }

    #[test]
    fn team_room_replies_thread_and_jump_to_parent() {
        let mut r = room();
        r.push_envelope(say("m1", "mira", "X causes Y"));
        let mut ch = say("m2", "ivan", "no evidence");
        ch.reply_to = Some("m1".into());
        ch.kind = Kind::Challenge {
            claim_ref: "m1".into(),
        };
        r.push_envelope(ch);
        assert_eq!(r.replies_to("m1").len(), 1);
        assert_eq!(r.replies_to("m1")[0].envelope.id, "m2");
        assert_eq!(r.parent_of("m2").unwrap().envelope.id, "m1");
        assert!(r.replies_to("m2").is_empty());
    }

    #[test]
    fn team_room_agent_state_tracks_speaking_and_events() {
        let mut r = room();
        assert_eq!(r.member_state("mira"), Some(RoomAgentState::Idle));
        let id = r.begin_stream("s1", Participant::Agent("mira".into()), Recipients::Room);
        assert!(r
            .messages()
            .iter()
            .any(|m| m.envelope.id == id && m.streaming));
        assert_eq!(r.member_state("mira"), Some(RoomAgentState::Speaking));
        r.apply_agent_state_event("mira", "working");
        assert_eq!(r.member_state("mira"), Some(RoomAgentState::Working));
        r.apply_agent_state_event("mira", "paused");
        assert_eq!(r.member_state("mira"), Some(RoomAgentState::Paused));
        r.push_envelope(say("m9", "mira", "done"));
        assert_eq!(RoomAgentState::Error.label(), "error");
    }

    #[test]
    fn team_room_streaming_coalesces_deltas_per_frame() {
        let mut r = room();
        let id = r.begin_stream("s1", Participant::Agent("mira".into()), Recipients::Room);
        for w in ["Hello", " ", "team", "!"] {
            r.push_delta(&id, w);
        }
        // Buffered until flush: one update per frame.
        assert_eq!(
            r.messages()
                .iter()
                .find(|m| m.envelope.id == id)
                .unwrap()
                .envelope
                .body,
            ""
        );
        let changed = r.flush_frame();
        assert_eq!(changed, vec![id.clone()]);
        assert!(r.flush_frame().is_empty());
        // Final envelope replaces the placeholder.
        r.finish_stream(say("s1", "mira", "Hello team!"));
        assert!(r.messages().iter().all(|m| !m.streaming));
        assert_eq!(r.member_state("mira"), Some(RoomAgentState::Idle));
    }

    #[test]
    fn team_room_mentions_route_to_selector_and_interject_posts() {
        let mut r = room();
        let mentioned = r.mentioned_in("@mira please check this, and @ivan too");
        assert_eq!(mentioned, vec!["mira".to_string(), "ivan".to_string()]);
        assert!(r.mentioned_in("no mentions here").is_empty());
        // Unknown names are ignored; matching is case-insensitive, deduped.
        assert_eq!(
            r.mentioned_in("@MIRA @mira @ghost"),
            vec!["mira".to_string()]
        );

        let (id, mentioned) = r.interject("@mira hold on, new direction");
        assert_eq!(mentioned, vec!["mira".to_string()]);
        let msg = r.messages().iter().find(|m| m.envelope.id == id).unwrap();
        assert_eq!(msg.author(), "You");
        assert!(matches!(msg.envelope.from, Participant::User));
    }

    #[test]
    fn team_room_run_controls_pause_resume_stop_skip_budget_members() {
        let mut r = room();
        assert_eq!(r.send_label(), "Stop"); // runs on creation
        r.set_agent_state("mira", RoomAgentState::Working);
        r.pause_run();
        assert_eq!(r.run_status(), RunStatus::Paused);
        assert_eq!(r.member_state("mira"), Some(RoomAgentState::Paused));
        // Interjection while paused queues a follow-up but still posts.
        let before = r.messages().len();
        r.interject("wait, one more thing");
        assert_eq!(r.messages().len(), before + 1);
        assert_eq!(r.queued_followups().len(), 1);
        r.resume_run();
        assert_eq!(r.run_status(), RunStatus::Running);
        assert_eq!(r.member_state("mira"), Some(RoomAgentState::Idle));
        let phase = r.run_phase();
        r.skip_phase();
        assert_ne!(r.run_phase(), phase);
        r.set_budget(5.0);
        assert_eq!(r.run().limits.budget_usd, 5.0);
        r.add_member("zoe", "Writer");
        assert!(r.run().members.contains(&"zoe".to_string()));
        r.remove_member("zoe");
        assert!(!r.run().members.contains(&"zoe".to_string()));
        // Stop clears streaming placeholders and idles agents.
        let id = r.begin_stream("sx", Participant::Agent("ivan".into()), Recipients::Room);
        r.push_delta(&id, "partial");
        r.stop_run();
        assert_eq!(r.run_status(), RunStatus::Stopped);
        assert!(r.messages().iter().all(|m| !m.streaming));
        assert_eq!(r.send_label(), "Send");
    }

    #[test]
    fn team_room_detail_levels_filter_to_summary() {
        let mut r = room();
        r.push_envelope(say("m1", "mira", "chatter"));
        let mut f = say("m2", "mira", "found it");
        f.kind = Kind::Finding { confidence: 0.9 };
        r.push_envelope(f);
        let mut p = say("m3", "lead", "we agree");
        p.kind = Kind::Propose {
            conclusion: "ship".into(),
        };
        r.push_envelope(p);
        assert_eq!(r.detail(), DetailLevel::Conversation);
        assert_eq!(r.item_count(), 3);
        r.set_detail(DetailLevel::Summary);
        assert_eq!(r.detail().label(), "Summary");
        let visible: Vec<String> = r
            .visible_messages()
            .iter()
            .map(|m| m.envelope.id.clone())
            .collect();
        assert_eq!(visible, vec!["m2".to_string(), "m3".to_string()]);
        r.set_detail(DetailLevel::Raw);
        assert_eq!(r.item_count(), 3);
        assert_eq!(r.detail().cycle(), DetailLevel::Summary);
    }

    #[test]
    fn team_room_empty_and_error_states() {
        let mut r = room();
        assert!(r.is_empty());
        let (title, action) = r.empty_state().expect("empty state");
        assert!(!title.is_empty());
        assert_eq!(action, "Start a run");
        assert!(r.last_error().is_none());
        r.set_error("Model key missing: add it in Settings → Providers.");
        assert!(r.last_error().unwrap().contains("Settings"));
        r.clear_error();
        assert!(r.last_error().is_none());
        r.push_envelope(say("m1", "mira", "hi"));
        assert!(!r.is_empty());
        assert!(r.empty_state().is_none());
    }

    #[test]
    fn team_room_composer_send_queues_while_streaming() {
        let mut r = room();
        r.set_draft("hello team");
        let (id, _) = r.send_draft(false).expect("sent");
        assert_eq!(r.draft(), "");
        assert!(r.messages().iter().any(|m| m.envelope.id == id));
        r.set_draft("follow-up while streaming");
        assert!(r.send_draft(true).is_none());
        assert_eq!(r.draft(), "");
        assert_eq!(r.queued_followups().len(), 1);
        r.set_draft("   ");
        assert!(r.send_draft(false).is_none());
    }
}
