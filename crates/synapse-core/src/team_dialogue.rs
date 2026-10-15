//! How the people in a project talk to each other in the main chat:
//! the roster every agent is shown, the prompt that teaches an agent how to
//! pass the work to a teammate, and the parser for those handoffs
//! (docs/TEAM-PROTOCOL.md).
//!
//! Work moves like a baton: one agent works, then hands the conversation to
//! whoever should act next (a researcher to an analyst, a critic back to the
//! researcher, anyone back to the lead). The handoff is plain text, not a
//! provider tool call, so it works with every model and streams like any
//! other reply:
//!
//! ```text
//! <ask agent="Mira">
//! Task: ...
//! Context: ...
//! Expected output: ...
//! </ask>
//! ```
//!
//! Pure functions only: the run loop that calls the models lives in the app.

/// One teammate as every agent sees them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub role: String,
    /// What this person is for, in one line: the roster shows it so others
    /// know when to ask them.
    pub brief: String,
    /// The persona text for this agent's own prompt.
    pub instructions: String,
    /// Avatar JSON for showing them in the chat.
    pub avatar: String,
}

/// Limits on one user message's worth of teamwork (TEAM-PROTOCOL.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Pings in total before the meeting stops asking for more.
    pub hops: usize,
    /// Pings one reply may send.
    pub pings_per_reply: usize,
    /// Turns one member may take in one relay (stops two agents bouncing
    /// work back and forth forever).
    pub turns_each: usize,
    /// Model calls running at once (listeners answering together).
    pub parallel_calls: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { hops: 12, pings_per_reply: 3, turns_each: 4, parallel_calls: 3 }
    }
}

/// A teammate request found in a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub agent: String,
    pub body: String,
}

const OPEN: &str = "<ask";
const CLOSE: &str = "</ask>";

/// Every complete `<ask agent="...">...</ask>` block, in order. An unclosed
/// block is ignored (the model was cut off mid-ask).
pub fn parse_asks(text: &str) -> Vec<Ask> {
    let mut asks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(tag_end) = after.find('>') else { break };
        let Some(close) = after[tag_end..].find(CLOSE).map(|i| i + tag_end) else { break };
        if let Some(agent) = attribute(&after[..tag_end], "agent") {
            let body = after[tag_end + 1..close].trim().to_string();
            if !agent.is_empty() && !body.is_empty() {
                asks.push(Ask { agent, body });
            }
        }
        rest = &after[close + CLOSE.len()..];
    }
    asks
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}="))? + name.len() + 1;
    let value = tag[at..].trim_start();
    let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let inner = &value[1..];
    let end = inner.find(quote)?;
    Some(inner[..end].trim().trim_start_matches('@').to_string())
}

/// The reply as the user should read it: ask blocks removed, including one
/// still being written, so partial tags never flash on screen.
pub fn visible_text(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        match after.find(CLOSE) {
            Some(close) => rest = &after[close + CLOSE.len()..],
            None => return out.trim_end().to_string(),
        }
    }
    out.push_str(rest);
    // A lone "<as" at the very end may be the start of a tag.
    for cut in (1..OPEN.len()).rev() {
        if out.ends_with(&OPEN[..cut]) {
            out.truncate(out.len() - cut);
            break;
        }
    }
    let out = out.trim();
    if is_pass(out) {
        return String::new();
    }
    out.to_string()
}

const PASS: &str = "<pass/>";

/// True for a reply that means "I have nothing to add": `<pass/>`, in any
/// spacing, or the first characters of it still being written.
pub fn is_pass(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    !compact.is_empty() && (PASS.starts_with(&compact) || compact.starts_with("<pass"))
}

/// The member a name refers to (case-insensitive; id, name or `@name`).
pub fn find_member<'a>(members: &'a [Member], name: &str) -> Option<&'a Member> {
    let wanted = name.trim().trim_start_matches('@').to_lowercase();
    members
        .iter()
        .find(|m| m.name.to_lowercase() == wanted || m.id.to_lowercase() == wanted)
}

/// The first teammate the user @mentioned in `text`.
pub fn mentioned<'a>(members: &'a [Member], text: &str) -> Option<&'a Member> {
    let lower = text.to_lowercase();
    let mut best: Option<(usize, &Member)> = None;
    for m in members {
        let needle = format!("@{}", m.name.to_lowercase());
        let mut from = 0;
        while let Some(i) = lower[from..].find(&needle).map(|i| i + from) {
            let end = i + needle.len();
            let boundary = lower[end..].chars().next().map_or(true, |c| !c.is_alphanumeric());
            if boundary && best.map_or(true, |(b, _)| i < b) {
                best = Some((i, m));
            }
            from = end;
        }
    }
    best.map(|(_, m)| m)
}

/// One line saying what a member is for: their role plus the first sentence
/// of their instructions.
pub fn brief_of(role: &str, instructions: &str) -> String {
    let first = instructions
        .trim()
        .split(['.', '\n'])
        .map(str::trim)
        .find(|s| s.chars().count() > 12)
        .unwrap_or("");
    let first: String = first.chars().take(140).collect();
    match (role.trim().is_empty(), first.is_empty()) {
        (_, true) => role.trim().to_string(),
        (true, false) => first,
        (false, false) => format!("{}. {first}", role.trim()),
    }
}

/// The system prompt for `me`: who they are, what the project is, who else is
/// on the team, how to pass the work on, and what to do when it is theirs.
pub fn team_prompt(
    project: &str,
    goal: &str,
    members: &[Member],
    me: &Member,
    is_lead: bool,
    can_hand_off: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("You are {}", me.name));
    if !me.role.trim().is_empty() {
        out.push_str(&format!(", {}", me.role.trim()));
    }
    out.push_str(&format!(
        ", on the team for the project \"{project}\" in Synapse, a desktop app where a team of \
         AI agents works with the user in one shared group chat.\n"
    ));
    if !me.instructions.trim().is_empty() {
        out.push_str(&format!("\n# Who you are\n{}\n", me.instructions.trim()));
    }
    out.push_str("\n# What we are working on\n");
    if goal.trim().is_empty() {
        out.push_str(
            "The project has no written goal yet. Work out what the user wants from what they \
             say, and ask them when it is unclear.\n",
        );
    } else {
        out.push_str(&format!("{}\n", goal.trim()));
    }
    out.push_str("\n# The team\n");
    for m in members {
        let you = if m.id == me.id { " (you)" } else { "" };
        out.push_str(&format!("- {}{you}: {}\n", m.name, m.brief));
    }
    out.push_str("- The user: owns the project and talks to the team in the main chat.\n");
    out.push_str(
        "\n# How we work\n\
         You see the whole group chat: the user's messages and what each teammate said, in order. \
         It is a meeting, not a pipeline: you speak when it is your part, and you ping a \
         teammate when you need them. Whoever you ping answers next, seeing everything said so \
         far, and may ping you or others back. Two people can work on something together, or \
         three, or only you; use as many as the matter needs and no more. Nobody works in the \
         background: people speak only when pinged or when the chat is addressed to everyone.\n",
    );
    if can_hand_off {
        out.push_str(
            "\n# Passing the work on\n\
             When you need a teammate (to do work, check yours, answer a question, or take over \
             next), end your reply with a block like this for each one:\n\n\
             <ask agent=\"Name\">\nWhat you need from them and what you are leaving them, in a \
             few sentences.\n</ask>\n\n\
             - Use the teammate's name exactly as listed. Ping several only when they can work \
             independently at the same time; otherwise ping the one who should act next.\n\
             - Do your own part first and put the result in your reply; do not just forward the \
             request.\n\
             - Choose who is best placed. A critic who finds a gap sends it back to whoever can \
             fill it (for example the researcher); an analyst who lacks data asks the \
             researcher; whoever finishes hands to the lead to wrap up for the user.\n\
             - You can ping back whoever pinged you, for example to say what is still missing. \
             Keep going until the matter is settled, then give the result to the lead.\n\
             - If your reply needs no one else, write no block: nothing further is asked.\n\
             - Never hand to yourself and never pass back what you were just given unchanged.\n",
        );
    } else {
        out.push_str("\nYou cannot pass the work on in this task; finish your part yourself.\n");
    }
    out.push_str(
        "\n# Keep it proportionate\n\
         Match the size of your reply to the message. A greeting, thanks or small talk gets a \
         short, friendly reply of a sentence or two and nothing more: do not introduce your \
         role, list what you can do, start work, or pass the work on. Wait for the user's actual \
         request.\n",
    );
    if is_lead {
        out.push_str(
            "\n# Your job: the user's point of contact\n\
             Every message from the user comes to you, and you are the only one the user reads. \
             Answer simple things directly and keep chatting normally. For work that belongs to \
             teammates, frame it (what is wanted and why) and ping whoever it needs: the project \
             lead, a specialist, or several at once. Teammates' replies are working notes the user \
             does not read unless they open them; the discussion between them stays in the \
             background. When it comes back finished, give the user one clear answer in your own \
             words, saying who contributed. If a teammate needs something only the user can give, \
             or a decision is the user's, ask the user yourself, briefly, and stop. If the user \
             @mentions someone, that person answers instead of you.\n",
        );
    } else {
        out.push_str(
            "\n# Reporting\n\
             Your replies go to your teammates, not to the user: the user reads only what the \
             orchestrator tells them. Lead with your result, keep it under 300 words, and say what \
             you could not do. If you need an answer only the user can give, put the question to \
             the orchestrator in an ask block instead of guessing. Do not talk about this protocol.\n",
        );
    }
    out
}

/// The note that tells a member the baton is theirs.
pub fn turn_note(from: &str, ask: &Ask) -> String {
    format!("{from} handed the work to you:\n\n{}\n\nIt is your turn.", ask.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, name: &str, role: &str) -> Member {
        Member {
            id: id.into(),
            name: name.into(),
            role: role.into(),
            brief: brief_of(role, "Finds sources and reads them carefully."),
            instructions: "Finds sources and reads them carefully.".into(),
            avatar: String::new(),
        }
    }

    #[test]
    fn parses_complete_asks_and_ignores_cut_off_ones() {
        let text = "Let me check.\n<ask agent=\"Mira\">\nTask: find data\n</ask>\n\
                    and <ask agent='@Leo'>Review it</ask> then <ask agent=\"Zed\">half";
        let asks = parse_asks(text);
        assert_eq!(asks.len(), 2);
        assert_eq!(asks[0].agent, "Mira");
        assert_eq!(asks[0].body, "Task: find data");
        assert_eq!(asks[1].agent, "Leo");
    }

    #[test]
    fn visible_text_hides_blocks_even_while_one_is_being_written() {
        let done = "Hello <ask agent=\"Mira\">x</ask> there";
        assert_eq!(visible_text(done), "Hello  there");
        assert_eq!(visible_text("Checking.\n<ask agent=\"Mi"), "Checking.");
        assert_eq!(visible_text("Checking <as"), "Checking");
        assert_eq!(visible_text("plain"), "plain");
    }

    #[test]
    fn a_pass_hides_the_reply_even_while_it_is_being_written() {
        assert!(is_pass("<pass/>") && is_pass(" <pass />\n") && is_pass("<pa"));
        assert!(!is_pass("") && !is_pass("Hi!"));
        assert_eq!(visible_text("<pass/>"), "");
    }

    #[test]
    fn mentions_need_a_word_boundary_and_the_first_one_wins() {
        let team = [member("a1", "Mira", "Researcher"), member("a2", "Mirand", "Critic")];
        assert_eq!(mentioned(&team, "ask @Mirand about it").unwrap().id, "a2");
        assert_eq!(mentioned(&team, "hi @mira, and @Mirand").unwrap().id, "a1");
        assert!(mentioned(&team, "email me@miranda.com").is_none());
        assert_eq!(find_member(&team, "@MIRA").unwrap().id, "a1");
    }

    #[test]
    fn the_prompt_tells_each_agent_the_project_the_team_and_how_to_ask() {
        let team = [member("lead", "Lead", "Chief of staff"), member("a1", "Mira", "Researcher")];
        let lead = team_prompt("Uganda fintech", "Find the best niche.", &team, &team[0], true, true);
        assert!(lead.contains("\"Uganda fintech\""));
        assert!(lead.contains("Find the best niche."));
        assert!(lead.contains("- Mira: Researcher. Finds sources"));
        assert!(lead.contains("- Lead (you)"));
        assert!(lead.contains("<ask agent=\"Name\">"));
        assert!(lead.contains("point of contact"));
        let worker = team_prompt("P", "", &team, &team[1], false, false);
        assert!(worker.contains("no written goal"));
        assert!(worker.contains("cannot pass the work on"));
        assert!(worker.contains("Reporting"));
    }
}
