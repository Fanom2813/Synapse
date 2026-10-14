//! Creating a project: the store rows plus the sidebar entry. Toolkit-free;
//! the "New project" popup and the sidebar call these.

use crate::shell::ShellProject;
use synapse_store::{new_id, NewProject, Store};

/// A project just created, with the conversation its room opens.
pub struct CreatedProject {
    pub project: ShellProject,
    pub conversation_id: String,
}

/// Create a project named `name` with an empty room conversation.
pub fn create_project(store: &Store, name: &str) -> Result<CreatedProject, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the project a name.".into());
    }
    let id = new_id("project");
    store
        .create_project(&NewProject {
            id: id.clone(),
            name: name.to_string(),
            goal: String::new(),
            folder: String::new(),
            lead_agent_id: String::new(),
            template: "custom".into(),
        })
        .map_err(|e| e.to_string())?;
    let conversation_id = new_id("conversation");
    store
        .create_conversation(&conversation_id, &id, "room", name)
        .map_err(|e| e.to_string())?;
    crate::team_setup::seed_default_team(store, &id);
    Ok(CreatedProject {
        project: ShellProject {
            id,
            name: name.to_string(),
            has_messages: false,
        },
        conversation_id,
    })
}

/// Projects saved in the store, for the sidebar. The hidden single-chat
/// project (`ensure_chat_conversation`) is not a user project.
pub fn load_projects(store: &Store) -> Vec<ShellProject> {
    store
        .list_projects(false)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.id != crate::window::CHAT_PROJECT_ID)
        .map(|p| ShellProject {
            id: p.id,
            name: p.name,
            has_messages: false,
        })
        .collect()
}

/// The conversation a project's room shows: its first one, made on demand.
pub fn room_conversation(store: &Store, project_id: &str, title: &str) -> Option<String> {
    if let Some(first) = store.list_conversations(project_id).ok()?.into_iter().next() {
        return Some(first.id);
    }
    let id = new_id("conversation");
    store
        .create_conversation(&id, project_id, "room", title)
        .ok()?;
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::{create_project, room_conversation};
    use synapse_store::Store;

    #[test]
    fn a_new_project_is_stored_with_an_empty_room() {
        let store = Store::open_in_memory().unwrap();
        assert!(create_project(&store, "  ").is_err());
        let made = create_project(&store, " Market study ").unwrap();
        assert_eq!(made.project.name, "Market study");
        assert_eq!(store.list_projects(false).unwrap().len(), 1);
        assert_eq!(
            room_conversation(&store, &made.project.id, "x"),
            Some(made.conversation_id)
        );
    }
}
