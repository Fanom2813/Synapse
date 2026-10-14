//! synapse-store: SQLite persistence (see docs/ARCHITECTURE.md).

pub mod models;
pub mod schema;
pub mod store;

pub use models::{new_id, Agent, Conversation, Message, NewAgent, NewMessage, NewProject, Project};
pub use store::{Result, Store, StoreError};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn test_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn sample_project(id: &str) -> NewProject {
        NewProject {
            id: id.into(),
            name: "Market study".into(),
            goal: "Understand fintech".into(),
            folder: "/tmp/synapse-test".into(),
            lead_agent_id: String::new(),
            template: "research".into(),
        }
    }

    fn sample_agent(id: &str) -> NewAgent {
        NewAgent {
            id: id.into(),
            name: "Ada".into(),
            role: "lead".into(),
            instructions: "Be helpful".into(),
            backend: r#"{"type":"api"}"#.into(),
            avatar: "{}".into(),
            tools: "[]".into(),
            permissions: "{}".into(),
            persona_source: String::new(),
        }
    }

    #[test]
    fn projects_crud_and_archive_filter() {
        let s = test_store();
        let p = s.create_project(&sample_project("p1")).unwrap();
        assert_eq!(p.name, "Market study");
        assert_eq!(s.get_project("p1").unwrap().id, "p1");
        assert_eq!(s.list_projects(false).unwrap().len(), 1);
        s.archive_project("p1", true).unwrap();
        assert!(s.list_projects(false).unwrap().is_empty());
        assert_eq!(s.list_projects(true).unwrap().len(), 1);
        let renamed = s.rename_project("p1", "New name").unwrap();
        assert_eq!(renamed.name, "New name");
        s.delete_project("p1").unwrap();
        assert!(s.list_projects(true).unwrap().is_empty());
    }

    #[test]
    fn agents_crud_and_project_membership() {
        let s = test_store();
        s.create_project(&sample_project("p1")).unwrap();
        s.create_agent(&sample_agent("a1")).unwrap();
        s.create_agent(&sample_agent("a2")).unwrap();
        assert_eq!(s.list_agents().unwrap().len(), 2);
        s.add_agent_to_project("p1", "a1", "{}").unwrap();
        s.add_agent_to_project("p1", "a2", "{}").unwrap();
        assert_eq!(s.list_project_agents("p1").unwrap().len(), 2);
        s.remove_agent_from_project("p1", "a1").unwrap();
        assert_eq!(s.list_project_agents("p1").unwrap().len(), 1);
        let updated = s
            .update_agent_instructions("a2", "New instructions")
            .unwrap();
        assert_eq!(updated.instructions, "New instructions");
        s.delete_agent("a2").unwrap();
        assert!(s.list_project_agents("p1").unwrap().is_empty());
    }

    #[test]
    fn messages_insert_list_and_search() {
        let s = test_store();
        s.create_project(&sample_project("p1")).unwrap();
        s.create_conversation("c1", "p1", "team_room", "Room")
            .unwrap();
        for i in 0..3 {
            s.insert_message(&NewMessage {
                id: format!("m{i}"),
                conversation_id: "c1".into(),
                run_id: "r1".into(),
                from_id: "user".into(),
                to_json: "[]".into(),
                kind: "chat".into(),
                reply_to: String::new(),
                thread_id: String::new(),
                body: format!("hello world message {i} fintech"),
                meta: "{}".into(),
            })
            .unwrap();
        }
        let list = s.list_messages("c1", 10, 0).unwrap();
        assert_eq!(list.len(), 3);
        let page = s.list_messages("c1", 2, 1).unwrap();
        assert_eq!(page.len(), 2);
        let hits = s.search_messages("fintech", 10).unwrap();
        assert_eq!(hits.len(), 3);
        assert!(s.get_message("m0").unwrap().body.contains("hello"));
    }

    #[test]
    fn settings_roundtrip() {
        let s = test_store();
        assert_eq!(s.setting_get("theme").unwrap(), None);
        s.setting_set("theme", "dark").unwrap();
        assert_eq!(s.setting_get("theme").unwrap().as_deref(), Some("dark"));
    }

    #[test]
    fn persists_across_reopen() {
        let dir = std::env::temp_dir().join(format!("synapse-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synapse.db");
        let _ = std::fs::remove_file(&path);
        {
            let s = Store::open(&path).unwrap();
            s.create_project(&sample_project("p1")).unwrap();
            s.create_agent(&sample_agent("a1")).unwrap();
            s.create_conversation("c1", "p1", "team_room", "Room")
                .unwrap();
            s.insert_message(&NewMessage {
                id: "m1".into(),
                conversation_id: "c1".into(),
                run_id: String::new(),
                from_id: "user".into(),
                to_json: "[]".into(),
                kind: "chat".into(),
                reply_to: String::new(),
                thread_id: String::new(),
                body: "persist me".into(),
                meta: "{}".into(),
            })
            .unwrap();
        }
        let s2 = Store::open(&path).unwrap();
        assert_eq!(s2.get_project("p1").unwrap().name, "Market study");
        assert_eq!(s2.get_agent("a1").unwrap().name, "Ada");
        assert_eq!(s2.get_message("m1").unwrap().body, "persist me");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(dir.join("synapse.db-wal"));
        let _ = std::fs::remove_file(dir.join("synapse.db-shm"));
    }

    #[test]
    fn wal_mode_and_concurrent_reads() {
        let dir = std::env::temp_dir().join(format!("synapse-store-wal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wal.db");
        let _ = std::fs::remove_file(&path);
        let s = Arc::new(Store::open(&path).unwrap());
        let mode: String = s.with_journal_mode_for_test().unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        s.create_project(&sample_project("p1")).unwrap();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let s = Arc::clone(&s);
                thread::spawn(move || s.list_projects(false).unwrap().len())
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), 1);
        }
        let _ = std::fs::remove_file(&path);
    }
}
