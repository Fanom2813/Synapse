//! Conversation map: team members as graph nodes, Ask/Delegate/Challenge
//! messages as directed edges (see docs/MILESTONES.md #24).
//!
//! Pure state over [`Envelope`] history; the GPUI view renders nodes/edges
//! and selecting an edge jumps to the corresponding message id.

use std::collections::BTreeMap;

use synapse_core::{Envelope, Kind, Participant, Recipients};

/// Node id for a participant.
pub fn node_id_of(p: &Participant) -> String {
    match p {
        Participant::User => "user".to_string(),
        Participant::System => "system".to_string(),
        Participant::Agent(id) => id.clone(),
    }
}

/// Display label for a node id.
pub fn node_label_of(id: &str) -> String {
    if id == "user" {
        "You".to_string()
    } else {
        id.to_string()
    }
}

/// One team member node in the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapNode {
    pub id: String,
    pub label: String,
    pub role: String,
    pub message_count: usize,
}

/// Edge kind shown in the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapEdgeKind {
    Ask,
    Delegate,
    Challenge,
}

impl MapEdgeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Delegate => "delegate",
            Self::Challenge => "challenge",
        }
    }

    fn of(kind: &Kind) -> Option<Self> {
        match kind {
            Kind::Ask { .. } => Some(Self::Ask),
            Kind::Delegate { .. } => Some(Self::Delegate),
            Kind::Challenge { .. } => Some(Self::Challenge),
            _ => None,
        }
    }
}

/// One directed edge: `from` asked/delegated/challenged `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEdge {
    /// Index into [`ConversationMap::edges`].
    pub index: usize,
    /// Message this edge was built from; selecting the edge jumps here.
    pub message_id: String,
    pub from: String,
    pub to: String,
    pub kind: MapEdgeKind,
}

/// Graph of members (nodes) and Ask/Delegate/Challenge links (edges).
#[derive(Debug, Clone, Default)]
pub struct ConversationMap {
    nodes: Vec<MapNode>,
    edges: Vec<MapEdge>,
    selected: Option<usize>,
}

impl ConversationMap {
    /// Build the map from message history plus the known member list
    /// (`(id, role)` pairs; members with no messages still get a node).
    pub fn build(envelopes: &[Envelope], members: &[(String, String)]) -> Self {
        let mut roles: BTreeMap<String, String> = BTreeMap::new();
        for (id, role) in members {
            roles.entry(id.clone()).or_insert_with(|| role.clone());
        }
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for env in envelopes {
            let from = node_id_of(&env.from);
            *counts.entry(from.clone()).or_insert(0) += 1;
            roles.entry(from).or_default();
        }
        // Authors indexed so room-addressed challenges can target the
        // challenged author instead of a synthetic "room" node.
        let authors: BTreeMap<&str, String> = envelopes
            .iter()
            .map(|e| (e.id.as_str(), node_id_of(&e.from)))
            .collect();

        let mut edges = Vec::new();
        for env in envelopes {
            let Some(kind) = MapEdgeKind::of(&env.kind) else {
                continue;
            };
            let from = node_id_of(&env.from);
            for target in edge_targets(env, &authors) {
                roles.entry(target.clone()).or_default();
                edges.push(MapEdge {
                    index: edges.len(),
                    message_id: env.id.clone(),
                    from: from.clone(),
                    to: target,
                    kind,
                });
            }
        }

        let nodes = roles
            .into_iter()
            .map(|(id, role)| MapNode {
                label: node_label_of(&id),
                message_count: counts.get(&id).copied().unwrap_or(0),
                id,
                role,
            })
            .collect();
        Self {
            nodes,
            edges,
            selected: None,
        }
    }

    pub fn nodes(&self) -> &[MapNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[MapEdge] {
        &self.edges
    }

    pub fn node(&self, id: &str) -> Option<&MapNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Edges leaving a node.
    pub fn edges_from(&self, id: &str) -> Vec<&MapEdge> {
        self.edges.iter().filter(|e| e.from == id).collect()
    }

    /// Edges arriving at a node.
    pub fn edges_to(&self, id: &str) -> Vec<&MapEdge> {
        self.edges.iter().filter(|e| e.to == id).collect()
    }

    /// Select an edge by its index; returns the message id to jump to.
    pub fn select_edge(&mut self, index: usize) -> Option<String> {
        let edge = self.edges.iter().find(|e| e.index == index)?;
        self.selected = Some(index);
        Some(edge.message_id.clone())
    }

    /// Select the edge built from `message_id`; returns its index.
    pub fn select_message(&mut self, message_id: &str) -> Option<usize> {
        let edge = self.edges.iter().find(|e| e.message_id == message_id)?;
        self.selected = Some(edge.index);
        Some(edge.index)
    }

    pub fn selected(&self) -> Option<&MapEdge> {
        self.selected
            .and_then(|ix| self.edges.iter().find(|e| e.index == ix))
    }

    /// Message id the map currently jumps to, if an edge is selected.
    pub fn selected_message_id(&self) -> Option<&str> {
        self.selected().map(|e| e.message_id.as_str())
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }
}

/// Resolve the target node(s) of an Ask/Delegate/Challenge envelope.
fn edge_targets(env: &Envelope, authors: &BTreeMap<&str, String>) -> Vec<String> {
    match &env.to {
        Recipients::Agent(id) => vec![id.clone()],
        Recipients::Agents(ids) => ids.clone(),
        Recipients::Lead => vec!["lead".to_string()],
        Recipients::User => vec!["user".to_string()],
        Recipients::Room => {
            if let Kind::Challenge { claim_ref } = &env.kind {
                if let Some(author) = authors.get(claim_ref.as_str()) {
                    return vec![author.clone()];
                }
                if let Some(reply_to) = env.reply_to.as_deref() {
                    if let Some(author) = authors.get(reply_to) {
                        return vec![author.clone()];
                    }
                }
            }
            vec![]
        }
    }
}

#[cfg(test)]
mod conversation_map_tests {
    use super::*;
    use synapse_core::{Participant, Recipients};

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

    fn ask(id: &str, from: &str, to: &str) -> Envelope {
        let mut e = say(id, from, "question");
        e.to = Recipients::Agent(to.into());
        e.kind = Kind::Ask {
            question: "q".into(),
        };
        e
    }

    fn delegate(id: &str, from: &str, to: &str) -> Envelope {
        let mut e = say(id, from, "task");
        e.to = Recipients::Agent(to.into());
        e.kind = Kind::Delegate {
            task: "t".into(),
            budget: "b".into(),
            deliverable: "d".into(),
        };
        e
    }

    fn challenge(id: &str, from: &str, claim_ref: &str) -> Envelope {
        let mut e = say(id, from, "dispute");
        e.kind = Kind::Challenge {
            claim_ref: claim_ref.into(),
        };
        e.reply_to = Some(claim_ref.into());
        e
    }

    fn members() -> Vec<(String, String)> {
        vec![
            ("lead".into(), "Lead".into()),
            ("mira".into(), "Researcher".into()),
            ("ivan".into(), "Critic".into()),
        ]
    }

    #[test]
    fn conversation_map_members_become_nodes() {
        let history = vec![say("m1", "mira", "hello")];
        let map = ConversationMap::build(&history, &members());
        // All known members get a node even with no messages yet.
        for id in ["lead", "mira", "ivan"] {
            let node = map.node(id).unwrap_or_else(|| panic!("node {id}"));
            assert_eq!(node.label, id);
        }
        assert_eq!(map.node("mira").unwrap().message_count, 1);
        assert_eq!(map.node("ivan").unwrap().message_count, 0);
        assert!(map.edges().is_empty());
    }

    #[test]
    fn conversation_map_ask_delegate_challenge_become_edges() {
        let history = vec![
            say("m1", "mira", "X causes Y"),
            ask("m2", "lead", "mira"),
            delegate("m3", "lead", "ivan"),
            challenge("m4", "ivan", "m1"),
            say("m5", "mira", "just chatting"),
        ];
        let map = ConversationMap::build(&history, &members());
        let kinds: Vec<(String, String, &str)> = map
            .edges()
            .iter()
            .map(|e| (e.from.clone(), e.to.clone(), e.kind.label()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("lead".to_string(), "mira".to_string(), "ask"),
                ("lead".to_string(), "ivan".to_string(), "delegate"),
                // Room-addressed challenge targets the challenged author.
                ("ivan".to_string(), "mira".to_string(), "challenge"),
            ]
        );
        // Plain Say messages never become edges.
        assert!(!map.edges().iter().any(|e| e.message_id == "m5"));
    }

    #[test]
    fn conversation_map_selecting_edge_jumps_to_message() {
        let history = vec![say("m1", "mira", "claim"), challenge("m2", "ivan", "m1")];
        let mut map = ConversationMap::build(&history, &members());
        assert!(map.selected_message_id().is_none());
        let target = map.select_edge(0).expect("edge 0");
        assert_eq!(target, "m2");
        assert_eq!(map.selected_message_id(), Some("m2"));
        assert_eq!(map.selected().unwrap().message_id, "m2");
        // Selecting from the message side highlights the same edge.
        assert_eq!(map.select_message("m2"), Some(0));
        map.clear_selection();
        assert!(map.selected().is_none());
        assert!(map.select_edge(99).is_none());
        assert!(map.select_message("nope").is_none());
    }

    #[test]
    fn conversation_map_edges_from_and_to_filter_by_member() {
        let history = vec![ask("m1", "lead", "mira"), delegate("m2", "mira", "ivan")];
        let map = ConversationMap::build(&history, &members());
        assert_eq!(map.edges_from("lead").len(), 1);
        assert_eq!(map.edges_to("ivan").len(), 1);
        assert_eq!(map.edges_to("ivan")[0].kind, MapEdgeKind::Delegate);
        assert!(map.edges_from("ivan").is_empty());
    }
}
