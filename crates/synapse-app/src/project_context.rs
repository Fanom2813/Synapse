//! What the main chat's model is told about the project it works in: its
//! name, goal and team. Without this the model answers "I don't see a
//! project". Pure text, rebuilt whenever the project, its goal or its team
//! changes.

/// System prompt for the lead's chat turns. `team` is `(name, role)` pairs.
pub fn system_prompt(project: &str, goal: &str, team: &[(String, String)]) -> String {
    let mut out = format!(
        "You are the lead of the project \"{project}\" in Synapse, a desktop app where a team \
         of AI agents works with the user. Speak to the user directly, plainly and briefly.\n"
    );
    if goal.trim().is_empty() {
        out.push_str(
            "\nThe project has no written goal yet. Work out what the user wants from what \
             they say, and ask when it is unclear.\n",
        );
    } else {
        out.push_str(&format!("\nProject goal: {}\n", goal.trim()));
    }
    if team.is_empty() {
        out.push_str("\nThe team has no other members yet.\n");
    } else {
        out.push_str("\nTeam members:\n");
        for (name, role) in team {
            out.push_str(&format!("- {name}: {role}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::system_prompt;

    #[test]
    fn names_the_project_goal_and_team() {
        let team = vec![("Mira".to_string(), "Researcher".to_string())];
        let text = system_prompt("Uganda fintech", "Find the best niche.", &team);
        assert!(text.contains("\"Uganda fintech\""));
        assert!(text.contains("Project goal: Find the best niche."));
        assert!(text.contains("- Mira: Researcher"));
        assert!(system_prompt("P", "", &[]).contains("no written goal"));
    }
}
