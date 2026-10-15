//! A project's name and goal are written by a model from the first prompt, so
//! creating a project asks for nothing. The prompt is scrubbed by `redact` first.

use synapse_core::redact::redact_text;
use synapse_llm::{AgentEvent, Role, TurnMessage, TurnRequest};

const SYSTEM: &str = "You set up a research project. Read the user's first message and reply \
with exactly two lines. Line 1: a short project name (at most 5 words). Line 2: ONE plain \
sentence (at most 25 words) saying what the project wants to find out or achieve. No quotes, \
no labels, no preamble, no markdown.";

/// Longest name we keep, in characters.
const MAX_NAME: usize = 60;

/// What a model made of the first prompt.
pub struct Drafted {
    pub name: String,
    pub goal: String,
}

/// Longest goal we keep, in characters.
const MAX_GOAL: usize = 200;
/// Longest part of the first prompt we send, in characters.
const MAX_PROMPT: usize = 4000;

/// The one-shot request that summarises `first_prompt` into a goal.
pub fn request(model: &str, first_prompt: &str) -> TurnRequest {
    let text: String = redact_text(first_prompt).text.chars().take(MAX_PROMPT).collect();
    let mut req = TurnRequest::new(model).with_system(SYSTEM);
    req.messages.push(TurnMessage { role: Role::User, text });
    req.max_tokens = Some(400);
    req.temperature = Some(0.2);
    req
}

/// The name and goal from a finished stream, or `None` when it said too little.
pub fn from_events(events: Vec<AgentEvent>) -> Option<Drafted> {
    let mut text = String::new();
    for event in events {
        match event {
            AgentEvent::TextDelta(delta) => text.push_str(&delta),
            AgentEvent::Done { text: full, .. } if text.trim().is_empty() => text = full,
            _ => {}
        }
    }
    let mut lines = text.lines().map(clean).filter(|l| !l.is_empty());
    let first = lines.next()?;
    let (name, goal) = match lines.next() {
        Some(goal) => (first.to_string(), goal),
        // One line only: it is the goal; its first words name the project.
        None => (first.split_whitespace().take(5).collect::<Vec<_>>().join(" "), first),
    };
    Some(Drafted {
        name: name.chars().take(MAX_NAME).collect(),
        goal: goal.chars().take(MAX_GOAL).collect(),
    })
}

fn clean(line: &str) -> &str {
    let line = line.trim();
    let line = ["Name:", "Goal:", "Project name:", "Title:"]
        .iter()
        .find_map(|label| line.strip_prefix(label))
        .unwrap_or(line);
    line.trim().trim_matches(|c| c == '"' || c == '\'' || c == '*' || c == '#').trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_name_then_a_goal_without_quotes() {
        let events = vec![
            AgentEvent::TextDelta("\"Fintech niche\"\nFind the best ".into()),
            AgentEvent::TextDelta("fintech niche.\nExtra".into()),
        ];
        let d = from_events(events).unwrap();
        assert_eq!((d.name.as_str(), d.goal.as_str()), ("Fintech niche", "Find the best fintech niche."));
        assert!(from_events(vec![]).is_none());
    }
}
