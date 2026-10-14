//! Typed team communication tools (see docs/TEAM-PROTOCOL.md §2).
//!
//! Agents emit room messages by calling these tools (function calling), not
//! by free-text parsing. Every call builds a correctly addressed [`Envelope`]
//! with the right [`Kind`](crate::bus::Kind), redacts it, persists it via
//! [`TeamBus::publish`], and returns the stored envelope.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::bus::{Envelope, Kind, Participant, Recipients, SourceRef, TeamBus};
use crate::redact::Redactor;

/// Tool names exposed to agents (function-calling + in-process MCP server).
pub const TEAM_TOOL_NAMES: &[&str] = &[
    "say",
    "ask_agent",
    "answer",
    "challenge",
    "agree",
    "refuse",
    "delegate",
    "report_finding",
    "propose",
    "vote",
    "ask_user",
    "done",
];

static MSG_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_id(caller: &str) -> String {
    let n = MSG_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("msg_{caller}_{n}")
}

/// JSON-schema description of one team tool for model function calling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

fn schema(props: &[(&str, &str, Value)], required: &[&str]) -> Value {
    let properties: serde_json::Map<String, Value> = props
        .iter()
        .map(|(name, kind, extra)| {
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), Value::String(kind.to_string()));
            if let Value::Object(map) = extra {
                for (k, v) in map {
                    obj.insert(k.clone(), v.clone());
                }
            }
            (name.to_string(), Value::Object(obj))
        })
        .collect();
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn str_prop(desc: &str) -> Value {
    Value::Object(
        [("description".to_string(), Value::String(desc.to_string()))]
            .into_iter()
            .collect(),
    )
}

/// Function-calling specs for every team tool.
pub fn team_tool_specs() -> Vec<TeamToolSpec> {
    vec![
        TeamToolSpec {
            name: "say".into(),
            description: "Say something to the room (ordinary contribution).".into(),
            parameters: schema(
                &[(
                    "text",
                    "string",
                    str_prop("What to say to the room (markdown)"),
                )],
                &["text"],
            ),
        },
        TeamToolSpec {
            name: "ask_agent".into(),
            description: "Ask another agent a question. They answer at a safe point.".into(),
            parameters: schema(
                &[
                    ("agent", "string", str_prop("Agent id to ask")),
                    ("question", "string", str_prop("The question to ask")),
                ],
                &["agent", "question"],
            ),
        },
        TeamToolSpec {
            name: "answer".into(),
            description: "Answer a message addressed to you (Ask or Delegate).".into(),
            parameters: schema(
                &[
                    (
                        "message_id",
                        "string",
                        str_prop("Id of the message being answered"),
                    ),
                    ("text", "string", str_prop("The answer text (markdown)")),
                ],
                &["message_id", "text"],
            ),
        },
        TeamToolSpec {
            name: "challenge".into(),
            description: "Dispute a specific message or claim with reasons or evidence.".into(),
            parameters: schema(
                &[
                    (
                        "message_id",
                        "string",
                        str_prop("Id of the message being challenged"),
                    ),
                    (
                        "reason",
                        "string",
                        str_prop("Reasons and evidence for the challenge"),
                    ),
                ],
                &["message_id", "reason"],
            ),
        },
        TeamToolSpec {
            name: "agree".into(),
            description: "Agree with a specific message or claim.".into(),
            parameters: schema(
                &[
                    (
                        "message_id",
                        "string",
                        str_prop("Id of the message being agreed with"),
                    ),
                    ("note", "string", str_prop("Optional supporting note")),
                ],
                &["message_id"],
            ),
        },
        TeamToolSpec {
            name: "refuse".into(),
            description: "Decline an Ask or Delegate addressed to you. A reason is required."
                .into(),
            parameters: schema(
                &[
                    (
                        "message_id",
                        "string",
                        str_prop("Id of the message being refused"),
                    ),
                    ("reason", "string", str_prop("Why the request is declined")),
                ],
                &["message_id", "reason"],
            ),
        },
        TeamToolSpec {
            name: "delegate".into(),
            description: "Ask another agent to do a sub-task with a clear deliverable.".into(),
            parameters: schema(
                &[
                    ("agent", "string", str_prop("Agent id to delegate to")),
                    ("task", "string", str_prop("What the agent should do")),
                    (
                        "deliverable",
                        "string",
                        str_prop("What the agent must return"),
                    ),
                ],
                &["agent", "task", "deliverable"],
            ),
        },
        TeamToolSpec {
            name: "report_finding".into(),
            description: "Report a research finding with citations to the room.".into(),
            parameters: schema(
                &[
                    (
                        "text",
                        "string",
                        str_prop("Finding text (markdown, max ~300 words)"),
                    ),
                    (
                        "confidence",
                        "number",
                        str_prop("Confidence between 0.0 and 1.0"),
                    ),
                ],
                &["text"],
            ),
        },
        TeamToolSpec {
            name: "propose".into(),
            description: "Propose a candidate conclusion for convergence.".into(),
            parameters: schema(
                &[("conclusion", "string", str_prop("The proposed conclusion"))],
                &["conclusion"],
            ),
        },
        TeamToolSpec {
            name: "vote".into(),
            description: "Vote on a proposal during convergence or ranking.".into(),
            parameters: schema(
                &[
                    ("on", "string", str_prop("Id of the proposal message")),
                    ("stance", "string", str_prop("agree, disagree or abstain")),
                ],
                &["on", "stance"],
            ),
        },
        TeamToolSpec {
            name: "ask_user".into(),
            description: "Escalate a question to the user.".into(),
            parameters: schema(
                &[("question", "string", str_prop("The question for the user"))],
                &["question"],
            ),
        },
        TeamToolSpec {
            name: "done".into(),
            description: "Finish your work and hand back a summary.".into(),
            parameters: schema(
                &[("summary", "string", str_prop("Summary of what was done"))],
                &["summary"],
            ),
        },
    ]
}

/// Every way a team tool call can fail before anything is stored.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TeamToolError {
    #[error("unknown team tool: {0}")]
    UnknownTool(String),
    #[error("team tool \"{tool}\" needs \"{field}\": {hint}")]
    MissingField {
        tool: String,
        field: String,
        hint: String,
    },
    #[error(transparent)]
    Bus(#[from] crate::bus::BusError),
}

/// Caller identity + run context for a batch of tool calls.
pub struct TeamTools {
    bus: TeamBus,
    from: Participant,
    caller_label: String,
    run_id: String,
    conversation_id: String,
    redactor: Redactor,
}

impl Clone for TeamTools {
    fn clone(&self) -> Self {
        Self {
            bus: self.bus.clone(),
            from: self.from.clone(),
            caller_label: self.caller_label.clone(),
            run_id: self.run_id.clone(),
            conversation_id: self.conversation_id.clone(),
            redactor: Redactor::new(),
        }
    }
}

impl TeamTools {
    /// Tools for an agent worker.
    pub fn for_agent(bus: &TeamBus, agent_id: &str, run_id: &str, conversation_id: &str) -> Self {
        Self {
            bus: bus.clone(),
            from: Participant::Agent(agent_id.into()),
            caller_label: agent_id.into(),
            run_id: run_id.into(),
            conversation_id: conversation_id.into(),
            redactor: Redactor::new(),
        }
    }

    /// Tools for the user (interjections, steering).
    pub fn for_user(bus: &TeamBus, run_id: &str, conversation_id: &str) -> Self {
        Self {
            bus: bus.clone(),
            from: Participant::User,
            caller_label: "user".into(),
            run_id: run_id.into(),
            conversation_id: conversation_id.into(),
            redactor: Redactor::new(),
        }
    }

    /// Dispatch one function call by name with JSON args. Returns the
    /// persisted envelope (with its assigned `seq`).
    pub fn call(&self, tool: &str, args: Value) -> Result<Envelope, TeamToolError> {
        match tool {
            "say" => {
                let text = req_str(tool, &args, "text", "what to say to the room")?;
                self.say(&text)
            }
            "ask_agent" => {
                let agent = req_str(tool, &args, "agent", "the agent id to ask")?;
                let question = req_str(tool, &args, "question", "the question to ask")?;
                self.ask_agent(&agent, &question)
            }
            "answer" => {
                let id = req_str(tool, &args, "message_id", "the message id to answer")?;
                let text = req_str(tool, &args, "text", "the answer text")?;
                self.answer(&id, &text)
            }
            "challenge" => {
                let id = req_str(tool, &args, "message_id", "the message id to challenge")?;
                let reason = req_str(tool, &args, "reason", "reasons or evidence")?;
                self.challenge(&id, &reason)
            }
            "agree" => {
                let id = req_str(tool, &args, "message_id", "the message id to agree with")?;
                let note = opt_str(&args, "note").unwrap_or_default();
                self.agree(&id, &note)
            }
            "refuse" => {
                let id = req_str(tool, &args, "message_id", "the message id to refuse")?;
                let reason = req_str(tool, &args, "reason", "why the request is declined")?;
                self.refuse(&id, &reason)
            }
            "delegate" => {
                let agent = req_str(tool, &args, "agent", "the agent id to delegate to")?;
                let task = req_str(tool, &args, "task", "what the agent should do")?;
                let deliverable =
                    req_str(tool, &args, "deliverable", "what the agent must return")?;
                self.delegate(&agent, &task, &deliverable)
            }
            "report_finding" => {
                let text = req_str(tool, &args, "text", "the finding text")?;
                let confidence = args
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5) as f32;
                let citations = args
                    .get("citations")
                    .and_then(|v| serde_json::from_value::<Vec<SourceRef>>(v.clone()).ok())
                    .unwrap_or_default();
                self.report_finding(&text, citations, confidence)
            }
            "propose" => {
                let conclusion = req_str(tool, &args, "conclusion", "the proposed conclusion")?;
                self.propose(&conclusion)
            }
            "vote" => {
                let on = req_str(tool, &args, "on", "the proposal message id")?;
                let stance = req_str(tool, &args, "stance", "agree, disagree or abstain")?;
                self.vote(&on, &stance)
            }
            "ask_user" => {
                let question = req_str(tool, &args, "question", "the question for the user")?;
                let options = args
                    .get("options")
                    .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
                    .unwrap_or_default();
                self.ask_user(&question, options)
            }
            "done" => {
                let summary = req_str(tool, &args, "summary", "a summary of what was done")?;
                self.finish(&summary)
            }
            other => Err(TeamToolError::UnknownTool(other.into())),
        }
    }

    /// Say something to the room (`Say`, addressed to the room).
    pub fn say(&self, text: &str) -> Result<Envelope, TeamToolError> {
        let text = non_empty("say", "text", text, "what to say to the room")?;
        self.store(
            Recipients::Room,
            Kind::Say,
            self.redact(&text),
            None,
            vec![],
        )
    }

    /// Ask another agent a question (`Ask`, addressed to that agent).
    pub fn ask_agent(&self, agent: &str, question: &str) -> Result<Envelope, TeamToolError> {
        let agent = non_empty("ask_agent", "agent", agent, "the agent id to ask")?;
        let question = non_empty("ask_agent", "question", question, "the question to ask")?;
        self.store(
            Recipients::Agent(agent),
            Kind::Ask {
                question: self.redact(&question),
            },
            self.redact(&question),
            None,
            vec![],
        )
    }

    /// Answer a message addressed to the caller. The answer goes back to the
    /// original sender when it is a single participant, else to the room.
    pub fn answer(&self, message_id: &str, text: &str) -> Result<Envelope, TeamToolError> {
        let message_id = non_empty("answer", "message_id", message_id, "the message id")?;
        let text = non_empty("answer", "text", text, "the answer text")?;
        let to = self.reply_route(&message_id);
        self.store(
            to,
            Kind::Answer,
            self.redact(&text),
            Some(message_id),
            vec![],
        )
    }

    /// Challenge a message or claim with reasons (`Challenge`, to the room).
    pub fn challenge(&self, message_id: &str, reason: &str) -> Result<Envelope, TeamToolError> {
        let message_id = non_empty("challenge", "message_id", message_id, "the message id")?;
        let reason = non_empty("challenge", "reason", reason, "reasons or evidence")?;
        self.store(
            Recipients::Room,
            Kind::Challenge {
                claim_ref: message_id.clone(),
            },
            self.redact(&reason),
            Some(message_id),
            vec![],
        )
    }

    /// Agree with a message or claim (`Agree`, to the room).
    pub fn agree(&self, message_id: &str, note: &str) -> Result<Envelope, TeamToolError> {
        let message_id = non_empty("agree", "message_id", message_id, "the message id")?;
        let body = if note.trim().is_empty() {
            format!("Agreed with {message_id}.")
        } else {
            note.to_string()
        };
        self.store(
            Recipients::Room,
            Kind::Agree {
                claim_ref: message_id.clone(),
            },
            self.redact(&body),
            Some(message_id),
            vec![],
        )
    }

    /// Refuse an Ask or Delegate. A reason is required (first-class refusal).
    pub fn refuse(&self, message_id: &str, reason: &str) -> Result<Envelope, TeamToolError> {
        let message_id = non_empty("refuse", "message_id", message_id, "the message id")?;
        let reason = non_empty("refuse", "reason", reason, "why the request is declined")?;
        let to = self.reply_route(&message_id);
        self.store(
            to,
            Kind::Refuse {
                reason: self.redact(&reason),
            },
            self.redact(&reason),
            Some(message_id),
            vec![],
        )
    }

    /// Delegate a sub-task to another agent (`Delegate`, addressed to it).
    pub fn delegate(
        &self,
        agent: &str,
        task: &str,
        deliverable: &str,
    ) -> Result<Envelope, TeamToolError> {
        let agent = non_empty("delegate", "agent", agent, "the agent id")?;
        let task = non_empty("delegate", "task", task, "what the agent should do")?;
        let deliverable = non_empty(
            "delegate",
            "deliverable",
            deliverable,
            "what the agent must return",
        )?;
        let body = format!("Task: {task}\nDeliverable: {deliverable}");
        self.store(
            Recipients::Agent(agent),
            Kind::Delegate {
                task: self.redact(&task),
                budget: String::new(),
                deliverable: self.redact(&deliverable),
            },
            self.redact(&body),
            None,
            vec![],
        )
    }

    /// Report a research finding with citations (`Finding`, to the room).
    pub fn report_finding(
        &self,
        text: &str,
        citations: Vec<SourceRef>,
        confidence: f32,
    ) -> Result<Envelope, TeamToolError> {
        let text = non_empty("report_finding", "text", text, "the finding text")?;
        let confidence = confidence.clamp(0.0, 1.0);
        self.store(
            Recipients::Room,
            Kind::Finding { confidence },
            self.redact(&text),
            None,
            citations,
        )
    }

    /// Propose a candidate conclusion (`Propose`, to the room).
    pub fn propose(&self, conclusion: &str) -> Result<Envelope, TeamToolError> {
        let conclusion = non_empty("propose", "conclusion", conclusion, "the conclusion")?;
        self.store(
            Recipients::Room,
            Kind::Propose {
                conclusion: self.redact(&conclusion),
            },
            self.redact(&conclusion),
            None,
            vec![],
        )
    }

    /// Vote on a proposal (`Vote`, to the room).
    pub fn vote(&self, on: &str, stance: &str) -> Result<Envelope, TeamToolError> {
        let on = non_empty("vote", "on", on, "the proposal message id")?;
        let stance = non_empty("vote", "stance", stance, "agree, disagree or abstain")?;
        let body = format!("Vote on {on}: {stance}");
        self.store(
            Recipients::Room,
            Kind::Vote {
                on: on.clone(),
                stance: stance.clone(),
            },
            self.redact(&body),
            Some(on),
            vec![],
        )
    }

    /// Escalate a question to the user (`NeedUser`, addressed to the user).
    pub fn ask_user(
        &self,
        question: &str,
        options: Vec<String>,
    ) -> Result<Envelope, TeamToolError> {
        let question = non_empty("ask_user", "question", question, "the question")?;
        self.store(
            Recipients::User,
            Kind::NeedUser {
                question: self.redact(&question),
                options,
            },
            self.redact(&question),
            None,
            vec![],
        )
    }

    /// Finish the caller's work (`Done`, to the room).
    pub fn finish(&self, summary: &str) -> Result<Envelope, TeamToolError> {
        let summary = non_empty("done", "summary", summary, "a summary of what was done")?;
        self.store(
            Recipients::Room,
            Kind::Done {
                summary: self.redact(&summary),
            },
            self.redact(&summary),
            None,
            vec![],
        )
    }

    /// Where a reply to `message_id` should go: back to a single-participant
    /// sender, otherwise the room so everyone sees it.
    fn reply_route(&self, message_id: &str) -> Recipients {
        let log = self.bus.history(&self.conversation_id, 0, usize::MAX);
        match log.iter().find(|e| e.id == message_id) {
            Some(prev) => match &prev.from {
                Participant::Agent(id) => Recipients::Agent(id.clone()),
                Participant::User => Recipients::User,
                Participant::System => Recipients::Room,
            },
            None => Recipients::Room,
        }
    }

    fn redact(&self, text: &str) -> String {
        self.redactor.redact(text).text
    }

    fn store(
        &self,
        to: Recipients,
        kind: Kind,
        body: String,
        reply_to: Option<String>,
        citations: Vec<SourceRef>,
    ) -> Result<Envelope, TeamToolError> {
        let env = Envelope {
            id: next_id(&self.caller_label),
            run_id: self.run_id.clone(),
            conversation_id: self.conversation_id.clone(),
            thread_id: String::new(),
            reply_to,
            from: self.from.clone(),
            to,
            kind,
            body,
            citations,
            attachments: Vec::new(),
            priority: crate::bus::Priority::Normal,
            usage: None,
            memory_ids: Vec::new(),
            seq: 0,
        };
        let seq = self.bus.publish(env.clone()).map_err(TeamToolError::Bus)?;
        let mut stored = env;
        stored.seq = seq;
        Ok(stored)
    }
}

fn non_empty(tool: &str, field: &str, value: &str, hint: &str) -> Result<String, TeamToolError> {
    if value.trim().is_empty() {
        return Err(TeamToolError::MissingField {
            tool: tool.into(),
            field: field.into(),
            hint: hint.into(),
        });
    }
    Ok(value.to_string())
}

fn req_str(tool: &str, args: &Value, field: &str, hint: &str) -> Result<String, TeamToolError> {
    match args.get(field).and_then(Value::as_str) {
        Some(v) if !v.trim().is_empty() => Ok(v.to_string()),
        _ => Err(TeamToolError::MissingField {
            tool: tool.into(),
            field: field.into(),
            hint: hint.into(),
        }),
    }
}

fn opt_str(args: &Value, field: &str) -> Option<String> {
    args.get(field).and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod team_tools_tests {
    use super::*;
    use crate::bus::Participant;
    use serde_json::json;

    fn setup(agent: &str) -> (TeamBus, TeamTools) {
        let bus = TeamBus::new();
        let tools = TeamTools::for_agent(&bus, agent, "run1", "c1");
        (bus, tools)
    }

    fn last(bus: &TeamBus) -> Envelope {
        bus.history("c1", 0, usize::MAX).pop().expect("stored")
    }

    #[test]
    fn team_tools_specs_cover_every_tool_with_schemas() {
        let specs = team_tool_specs();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        for want in TEAM_TOOL_NAMES {
            assert!(names.contains(want), "missing spec for {want}");
        }
        for spec in &specs {
            assert!(
                spec.parameters.get("properties").is_some(),
                "{} needs a parameters schema",
                spec.name
            );
            assert!(
                !spec.description.is_empty(),
                "{} needs a description",
                spec.name
            );
        }
    }

    #[test]
    fn team_tools_ask_answer_challenge_agree_refuse_round_trip() {
        let (bus, lead) = setup("lead");
        let member = TeamTools::for_agent(&bus, "mira", "run1", "c1");

        let ask = lead.ask_agent("mira", "Summarise the paper.").unwrap();
        assert_eq!(ask.to, Recipients::Agent("mira".into()));
        assert!(matches!(ask.kind, Kind::Ask { .. }));
        assert_eq!(ask.from, Participant::Agent("lead".into()));
        assert!(ask.seq >= 1);

        // Answer routes back to the asker with a reply link.
        let ans = member.answer(&ask.id, "It shows X.").unwrap();
        assert_eq!(ans.to, Recipients::Agent("lead".into()));
        assert_eq!(ans.reply_to.as_deref(), Some(ask.id.as_str()));
        assert!(matches!(ans.kind, Kind::Answer));

        let ch = lead
            .challenge(&ans.id, "Figure 2 contradicts that.")
            .unwrap();
        assert_eq!(ch.to, Recipients::Room);
        assert!(matches!(ch.kind, Kind::Challenge { .. }));
        assert_eq!(ch.reply_to.as_deref(), Some(ans.id.as_str()));

        let ag = member
            .agree(ans.id.as_str(), "Fair point, revising.")
            .unwrap();
        assert!(matches!(ag.kind, Kind::Agree { .. }));

        let refus = member.refuse(&ask.id, "No access to the paper.").unwrap();
        assert!(matches!(refus.kind, Kind::Refuse { .. }));
        assert_eq!(refus.to, Recipients::Agent("lead".into()));

        let hist = bus.history("c1", 0, usize::MAX);
        assert_eq!(hist.len(), 5);
        assert!(hist.windows(2).all(|w| w[0].seq < w[1].seq));
    }

    #[test]
    fn team_tools_delegate_finding_propose_vote_ask_user_done() {
        let (bus, lead) = setup("lead");

        let d = lead
            .delegate("mira", "Read the paper", "A 200-word summary")
            .unwrap();
        assert_eq!(d.to, Recipients::Agent("mira".into()));
        assert!(matches!(d.kind, Kind::Delegate { .. }));

        let f = lead
            .report_finding(
                "X causes Y.",
                vec![SourceRef {
                    source_id: "s1".into(),
                    quote: "X causes Y".into(),
                }],
                0.8,
            )
            .unwrap();
        assert_eq!(f.to, Recipients::Room);
        assert!(matches!(f.kind, Kind::Finding { .. }));
        assert_eq!(f.citations.len(), 1);

        let p = lead.propose("We conclude Y.").unwrap();
        assert!(matches!(p.kind, Kind::Propose { .. }));

        let v = lead.vote(&p.id, "agree").unwrap();
        assert!(matches!(v.kind, Kind::Vote { .. }));
        assert_eq!(v.reply_to.as_deref(), Some(p.id.as_str()));

        let u = lead
            .ask_user("Ship it?", vec!["yes".into(), "no".into()])
            .unwrap();
        assert_eq!(u.to, Recipients::User);
        assert!(matches!(u.kind, Kind::NeedUser { .. }));

        let done = lead.finish("Shipped.").unwrap();
        assert!(matches!(done.kind, Kind::Done { .. }));
        assert_eq!(done.to, Recipients::Room);

        assert_eq!(bus.history("c1", 0, usize::MAX).len(), 6);
        assert_eq!(last(&bus).id, done.id);
    }

    #[test]
    fn team_tools_dispatch_by_name_persists_typed_messages() {
        let (bus, tools) = setup("lead");
        let ask = tools
            .call("ask_agent", json!({"agent": "mira", "question": "Go?"}))
            .unwrap();
        assert!(matches!(ask.kind, Kind::Ask { .. }));
        let done = tools
            .call("done", json!({"summary": "All finished."}))
            .unwrap();
        assert!(matches!(done.kind, Kind::Done { .. }));
        assert_eq!(bus.history("c1", 0, usize::MAX).len(), 2);

        let err = tools.call("teleport", json!({})).unwrap_err();
        assert!(matches!(err, TeamToolError::UnknownTool(_)));

        let err = tools
            .call("ask_agent", json!({"agent": "mira"}))
            .unwrap_err();
        assert!(matches!(err, TeamToolError::MissingField { .. }));
        // Failed calls store nothing.
        assert_eq!(bus.history("c1", 0, usize::MAX).len(), 2);
    }

    #[test]
    fn team_tools_blank_fields_and_reasons_rejected() {
        let (_bus, tools) = setup("lead");
        assert!(tools.ask_agent(" ", "q").is_err());
        assert!(tools.answer("m1", "  ").is_err());
        assert!(tools.challenge("m1", "").is_err());
        assert!(tools.refuse("m1", "  ").is_err());
        assert!(tools.delegate("mira", "", "d").is_err());
        assert!(tools.propose("").is_err());
        assert!(tools.finish("").is_err());
    }

    #[test]
    fn team_tools_say_is_listed_dispatched_and_published_to_room() {
        // Listed in exported names and schemas.
        assert!(TEAM_TOOL_NAMES.contains(&"say"));
        let specs = team_tool_specs();
        let say_spec = specs.iter().find(|s| s.name == "say").expect("say spec");
        assert!(say_spec
            .parameters
            .get("properties")
            .unwrap()
            .get("text")
            .is_some());

        let (bus, tools) = setup("lead");
        // Dispatched via typed method: room-addressed Say, redacted.
        let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let env = tools.say(&format!("Hello room {secret}")).unwrap();
        assert_eq!(env.to, Recipients::Room);
        assert!(matches!(env.kind, Kind::Say));
        assert!(!env.body.contains(secret));
        assert!(env.body.contains("[redacted:"));
        assert_eq!(bus.history("c1", 0, usize::MAX).len(), 1);
        assert_eq!(last(&bus).id, env.id);

        // Dispatched via generic call() by name.
        let env2 = tools.call("say", json!({"text": "via dispatch"})).unwrap();
        assert!(matches!(env2.kind, Kind::Say));
        assert_eq!(env2.to, Recipients::Room);
        assert_eq!(env2.body, "via dispatch");
        assert_eq!(bus.history("c1", 0, usize::MAX).len(), 2);
    }

    #[test]
    fn team_tools_output_is_redacted_before_store() {
        let (bus, tools) = setup("lead");
        // GitHub-token-shaped secrets must not reach the persisted log.
        let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let f = tools
            .report_finding(&format!("Found {secret} in the log"), vec![], 0.9)
            .unwrap();
        assert!(!f.body.contains(secret), "secret leaked into stored body");
        assert!(f.body.contains("[redacted:"), "got: {}", f.body);
        assert!(!last(&bus).body.contains(secret));
    }
}
