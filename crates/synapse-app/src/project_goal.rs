//! A project's goal is written by a model from the first prompt, so creating
//! a project only needs a title. The prompt is scrubbed by `redact` first.

use synapse_core::redact::redact_text;
use synapse_llm::{AgentEvent, Role, TurnMessage, TurnRequest};

const SYSTEM: &str = "You name the goal of a research project. Read the user's first message \
and reply with ONE plain sentence (at most 25 words) saying what the project wants to find out \
or achieve. No quotes, no preamble, no markdown.";

/// Longest goal we keep, in characters.
const MAX_GOAL: usize = 200;
/// Longest part of the first prompt we send, in characters.
const MAX_PROMPT: usize = 4000;

/// The one-shot request that summarises `first_prompt` into a goal.
pub fn request(model: &str, first_prompt: &str) -> TurnRequest {
    let text: String = redact_text(first_prompt).text.chars().take(MAX_PROMPT).collect();
    let mut req = TurnRequest::new(model).with_system(SYSTEM);
    req.messages.push(TurnMessage { role: Role::User, text });
    req.max_tokens = Some(80);
    req.temperature = Some(0.2);
    req
}

/// The goal sentence from a finished stream, or `None` when it said nothing.
pub fn from_events(events: Vec<AgentEvent>) -> Option<String> {
    let mut text = String::new();
    for event in events {
        if let AgentEvent::TextDelta(delta) = event {
            text.push_str(&delta);
        }
    }
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches(|c| c == '"' || c == '\'' || c == '*');
    let goal: String = line.chars().take(MAX_GOAL).collect();
    (!goal.is_empty()).then_some(goal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_first_line_without_quotes() {
        let events = vec![
            AgentEvent::TextDelta("\"Find the best ".into()),
            AgentEvent::TextDelta("fintech niche.\"\nExtra".into()),
        ];
        assert_eq!(from_events(events).as_deref(), Some("Find the best fintech niche."));
        assert_eq!(from_events(vec![]), None);
    }
}
