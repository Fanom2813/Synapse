//! Persisted team event bus (see docs/TEAM-PROTOCOL.md §2–3).
//!
//! Every [`Envelope`] is appended to the log **before** it is published:
//! the log is the event source of truth (SQLite in the app, in-memory here)
//! and lagging subscribers resync from it. Room observers get [`Event`]s
//! over a bounded broadcast channel; addressed agents get [`Envelope`]s
//! over bounded per-agent inboxes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc};

/// UI/observer fan-out per project. Lagging receivers resync from the log.
pub const BROADCAST_CAPACITY: usize = 1024;
/// Per-agent inbox bound. Full inboxes keep the stored message and resync.
pub const INBOX_CAPACITY: usize = 64;

/// Who sent an envelope. The user is a participant like any agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum Participant {
    User,
    Agent(String),
    System,
}

/// Who should receive an envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum Recipients {
    Room,
    Agent(String),
    Agents(Vec<String>),
    Lead,
    User,
}

impl Recipients {
    /// Agent ids directly addressed (empty for [`Recipients::Room`]/`User`).
    pub fn addressed(&self) -> Vec<&str> {
        match self {
            Self::Agent(id) => vec![id.as_str()],
            Self::Agents(ids) => ids.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        }
    }
}

/// Delivery urgency (TEAM-PROTOCOL.md §3b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    #[default]
    Normal,
    Steer,
    Interrupt,
}

/// Typed message kind (TEAM-PROTOCOL.md §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    Say,
    Ask {
        question: String,
    },
    Answer,
    Challenge {
        claim_ref: String,
    },
    Agree {
        claim_ref: String,
    },
    Refuse {
        reason: String,
    },
    Delegate {
        task: String,
        budget: String,
        deliverable: String,
    },
    Finding {
        confidence: f32,
    },
    Propose {
        conclusion: String,
    },
    Vote {
        on: String,
        stance: String,
    },
    NeedUser {
        question: String,
        options: Vec<String>,
    },
    Status {
        phase: String,
        progress: String,
    },
    Steer {
        instruction: String,
    },
    Done {
        summary: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourceRef {
    pub source_id: String,
    pub quote: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AttachmentRef {
    pub id: String,
    pub name: String,
    pub mime: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// One typed utterance. `seq` is the per-conversation monotonic order
/// assigned at persist time; it is 0 until published.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub id: String,
    pub run_id: String,
    pub conversation_id: String,
    pub thread_id: String,
    #[serde(default)]
    pub reply_to: Option<String>,
    pub from: Participant,
    pub to: Recipients,
    pub kind: Kind,
    pub body: String,
    #[serde(default)]
    pub citations: Vec<SourceRef>,
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub usage: Option<Usage>,
    /// Ids of memories that informed this message (MEMORY.md §5): the UI
    /// shows "used N memories" with links from these ids.
    #[serde(default)]
    pub memory_ids: Vec<String>,
    /// Assigned on persist; 0 = not yet published.
    #[serde(default)]
    pub seq: u64,
}

impl Envelope {
    pub fn say(
        id: &str,
        conversation_id: &str,
        from: Participant,
        to: Recipients,
        body: &str,
    ) -> Self {
        Self {
            id: id.into(),
            run_id: String::new(),
            conversation_id: conversation_id.into(),
            thread_id: String::new(),
            reply_to: None,
            from,
            to,
            kind: Kind::Say,
            body: body.into(),
            citations: Vec::new(),
            attachments: Vec::new(),
            priority: Priority::Normal,
            usage: None,
            memory_ids: Vec::new(),
            seq: 0,
        }
    }
}

/// Fanned-out event for room observers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Message(Box<Envelope>),
    TextDelta {
        message_id: String,
        delta: String,
    },
    ToolCall {
        message_id: String,
        tool: String,
        status: String,
    },
    ApprovalNeeded {
        tool_call_id: String,
        tool: String,
        preview: String,
    },
    AgentState {
        agent: String,
        state: String,
    },
    RunState {
        run_id: String,
        state: String,
    },
    Budget {
        run_id: String,
        spent_usd: f64,
        budget_usd: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BusError {
    #[error("envelope body is empty: add text before publishing")]
    EmptyBody,
    #[error("envelope id is empty: every message needs a unique id")]
    EmptyId,
    #[error("conversation id is empty: publish into a conversation")]
    EmptyConversation,
    #[error("refusal must carry a reason")]
    RefuseNeedsReason,
    #[error("challenge must reference a message")]
    ChallengeNeedsRef,
    #[error("agent inbox for {0} is full: stored, resync to catch up")]
    InboxFull(String),
}

fn validate(env: &Envelope) -> Result<(), BusError> {
    if env.id.trim().is_empty() {
        return Err(BusError::EmptyId);
    }
    if env.conversation_id.trim().is_empty() {
        return Err(BusError::EmptyConversation);
    }
    if env.body.trim().is_empty() {
        return Err(BusError::EmptyBody);
    }
    match &env.kind {
        Kind::Refuse { reason } if reason.trim().is_empty() => Err(BusError::RefuseNeedsReason),
        Kind::Challenge { claim_ref } if claim_ref.trim().is_empty() => {
            Err(BusError::ChallengeNeedsRef)
        }
        _ => Ok(()),
    }
}

/// Persist-then-publish bus. Clone it to share across tasks.
#[derive(Debug)]
struct Inner {
    sender: broadcast::Sender<Event>,
    inboxes: Mutex<HashMap<String, mpsc::Sender<Envelope>>>,
    lead_agent: Mutex<Option<String>>,
    log: Mutex<Vec<Envelope>>,
    seq: Mutex<HashMap<String, u64>>,
}

/// Persist-then-publish bus. Clone it to share across tasks.
#[derive(Clone)]
pub struct TeamBus {
    inner: Arc<Inner>,
}

impl TeamBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(Inner {
                sender,
                inboxes: Mutex::new(HashMap::new()),
                lead_agent: Mutex::new(None),
                log: Mutex::new(Vec::new()),
                seq: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn set_lead(&self, agent_id: &str) {
        *self.inner.lead_agent.lock().expect("lead lock") = Some(agent_id.into());
    }

    /// Room observer subscription.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.sender.subscribe()
    }

    /// Register (or replace) an agent's inbox and return its receiver.
    pub fn agent_inbox(&self, agent_id: &str) -> mpsc::Receiver<Envelope> {
        let (tx, rx) = mpsc::channel(INBOX_CAPACITY);
        self.inner
            .inboxes
            .lock()
            .expect("inbox lock")
            .insert(agent_id.into(), tx);
        rx
    }

    /// Persist `env` (assigning its per-conversation `seq`), then publish to
    /// room observers and to addressed agent inboxes. Returns the `seq`.
    pub fn publish(&self, mut env: Envelope) -> Result<u64, BusError> {
        validate(&env)?;
        let seq = {
            let mut seqs = self.inner.seq.lock().expect("seq lock");
            let next = seqs.get(&env.conversation_id).copied().unwrap_or(0) + 1;
            seqs.insert(env.conversation_id.clone(), next);
            next
        };
        env.seq = seq;
        // Persist first: the log is the source of truth for resync.
        self.inner.log.lock().expect("log lock").push(env.clone());
        // Room observers (lag is fine: they resync from history()).
        let _ = self
            .inner
            .sender
            .send(Event::Message(Box::new(env.clone())));
        // Addressed inboxes. A full inbox keeps the stored copy; the worker
        // resyncs via history() — never block the publisher.
        for target in self.route_targets(&env) {
            if let Some(tx) = self.inner.inboxes.lock().expect("inbox lock").get(&target) {
                if tx.try_send(env.clone()).is_err() {
                    // Stored already; surfaced so tests/metrics can count it.
                    let _ = self.inner.sender.send(Event::AgentState {
                        agent: target.clone(),
                        state: "inbox_full".into(),
                    });
                }
            }
        }
        Ok(seq)
    }

    /// Publish a non-message event (deltas, state, budget) without persisting.
    pub fn publish_event(&self, event: Event) {
        let _ = self.inner.sender.send(event);
    }

    /// Replay persisted envelopes for one conversation after `after_seq`.
    pub fn history(&self, conversation_id: &str, after_seq: u64, limit: usize) -> Vec<Envelope> {
        let log = self.inner.log.lock().expect("log lock");
        log.iter()
            .filter(|e| e.conversation_id == conversation_id && e.seq > after_seq)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Highest persisted `seq` for a conversation (0 when empty).
    pub fn latest_seq(&self, conversation_id: &str) -> u64 {
        self.inner
            .seq
            .lock()
            .expect("seq lock")
            .get(conversation_id)
            .copied()
            .unwrap_or(0)
    }

    /// Drain a broadcast receiver that hit `Lagged`: returns the events still
    /// buffered plus the missed persisted envelopes to replay in order.
    pub fn resync(
        &self,
        rx: &mut broadcast::Receiver<Event>,
        conversation_id: &str,
        last_seen_seq: u64,
    ) -> (Vec<Event>, Vec<Envelope>) {
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let missed = self.history(conversation_id, last_seen_seq, usize::MAX);
        (events, missed)
    }

    fn route_targets(&self, env: &Envelope) -> Vec<String> {
        match &env.to {
            Recipients::Room => self
                .inner
                .inboxes
                .lock()
                .expect("inbox lock")
                .keys()
                .cloned()
                .collect(),
            Recipients::Agent(id) => vec![id.clone()],
            Recipients::Agents(ids) => ids.clone(),
            Recipients::Lead => {
                let lead = self.inner.lead_agent.lock().expect("lead lock").clone();
                lead.into_iter().collect()
            }
            Recipients::User => Vec::new(),
        }
    }
}

impl Default for TeamBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod bus_tests {
    use super::*;
    use tokio::sync::broadcast::error::RecvError;

    fn env(id: &str, to: Recipients, body: &str) -> Envelope {
        Envelope::say(id, "c1", Participant::Agent("lead".into()), to, body)
    }

    #[test]
    fn bus_persists_before_publish_in_order() {
        let bus = TeamBus::new();
        let mut rx = bus.subscribe();
        let s1 = bus.publish(env("m1", Recipients::Room, "first")).unwrap();
        let s2 = bus.publish(env("m2", Recipients::Room, "second")).unwrap();
        assert_eq!((s1, s2), (1, 2));
        assert_eq!(bus.latest_seq("c1"), 2);
        // Persisted log first: history has both in seq order.
        let hist = bus.history("c1", 0, 10);
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].id, "m1");
        assert_eq!(hist[1].seq, 2);
        // Published after: broadcast carries the same envelopes.
        let got = [rx.try_recv().unwrap(), rx.try_recv().unwrap()];
        assert!(matches!(&got[0], Event::Message(e) if e.id == "m1" && e.seq == 1));
        assert!(matches!(&got[1], Event::Message(e) if e.id == "m2" && e.seq == 2));
    }

    #[test]
    fn bus_delivers_to_room_and_addressed_inboxes() {
        let bus = TeamBus::new();
        let mut room = bus.subscribe();
        let mut ada = bus.agent_inbox("ada");
        let mut max = bus.agent_inbox("max");
        bus.set_lead("ada");

        // Room reaches every inbox + observers.
        bus.publish(env("m1", Recipients::Room, "hello room"))
            .unwrap();
        assert_eq!(ada.try_recv().unwrap().id, "m1");
        assert_eq!(max.try_recv().unwrap().id, "m1");
        assert!(matches!(room.try_recv().unwrap(), Event::Message(_)));

        // Direct address reaches only that inbox (but still the room feed).
        bus.publish(env("m2", Recipients::Agent("max".into()), "for max"))
            .unwrap();
        assert_eq!(max.try_recv().unwrap().id, "m2");
        assert!(ada.try_recv().is_err());
        assert!(matches!(room.try_recv().unwrap(), Event::Message(_)));

        // Lead + multi-agent routing.
        bus.publish(env("m3", Recipients::Lead, "for lead"))
            .unwrap();
        assert_eq!(ada.try_recv().unwrap().id, "m3");
        bus.publish(env(
            "m4",
            Recipients::Agents(vec!["ada".into(), "max".into()]),
            "both",
        ))
        .unwrap();
        assert_eq!(ada.try_recv().unwrap().id, "m4");
        assert_eq!(max.try_recv().unwrap().id, "m4");

        // User-addressed messages go to observers only.
        bus.publish(env("m5", Recipients::User, "for user"))
            .unwrap();
        assert!(ada.try_recv().is_err());
        assert!(matches!(room.try_recv().unwrap(), Event::Message(_)));
    }

    #[test]
    fn bus_lagging_subscriber_resyncs_from_store() {
        let bus = TeamBus::new();
        let mut slow = bus.subscribe();
        for i in 0..5 {
            bus.publish(env(&format!("m{i}"), Recipients::Room, &format!("msg {i}")))
                .unwrap();
        }
        // Slow reader only takes the first; everything after `last_seen_seq`
        // replays from the persisted log in seq order.
        let first = slow.try_recv().unwrap();
        assert!(matches!(first, Event::Message(e) if e.seq == 1));
        let (_buffered, missed) = bus.resync(&mut slow, "c1", 1);
        assert_eq!(missed.len(), 4);
        assert_eq!(missed[0].seq, 2);
        assert_eq!(missed[3].seq, 5);
        // A Lagged broadcast receiver is the same recovery path.
        let (tx, mut rx) = broadcast::channel::<u32>(1);
        let _ = tx.send(1);
        let _ = tx.send(2);
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
        assert!(matches!(RecvError::Lagged(1), RecvError::Lagged(_)));
    }

    #[test]
    fn bus_full_inbox_keeps_stored_copy_for_resync() {
        let bus = TeamBus::new();
        let mut inbox = bus.agent_inbox("ada");
        // Fill the 64-slot inbox without draining.
        for i in 0..INBOX_CAPACITY {
            bus.publish(env(&format!("m{i}"), Recipients::Agent("ada".into()), "x"))
                .unwrap();
        }
        bus.publish(env("overflow", Recipients::Agent("ada".into()), "x"))
            .unwrap();
        // Stored anyway: resync returns it even though the inbox dropped it.
        let hist = bus.history("c1", INBOX_CAPACITY as u64, 10);
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].id, "overflow");
        // Drain to prove the first 64 arrived live.
        for _ in 0..INBOX_CAPACITY {
            inbox.try_recv().unwrap();
        }
    }

    #[test]
    fn bus_rejects_invalid_envelopes_before_storing() {
        let bus = TeamBus::new();
        let mut bad = env("m1", Recipients::Room, "  ");
        assert_eq!(bus.publish(bad.clone()), Err(BusError::EmptyBody));
        bad.body = "ok".into();
        bad.id = "".into();
        assert_eq!(bus.publish(bad.clone()), Err(BusError::EmptyId));
        bad.id = "m1".into();
        bad.kind = Kind::Refuse { reason: " ".into() };
        assert_eq!(bus.publish(bad.clone()), Err(BusError::RefuseNeedsReason));
        bad.kind = Kind::Challenge {
            claim_ref: "".into(),
        };
        assert_eq!(bus.publish(bad), Err(BusError::ChallengeNeedsRef));
        assert_eq!(bus.latest_seq("c1"), 0);
        assert!(bus.history("c1", 0, 10).is_empty());
    }

    #[test]
    fn bus_event_json_shape() {
        let ev = Event::Message(Box::new(env("m1", Recipients::Room, "hi")));
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"event\":\"message\""));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }
}
