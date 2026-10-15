//! A project's team: the default agents every project starts with, adding
//! agents from the persona library, and which engine runs each agent.
//! Toolkit-free; the Team tab, the "Add agent" popup and Settings → Agents
//! call these.
//!
//! Engines: one default for the whole app (`agents.default_engine`), and an
//! optional override per agent (`agent.engine.<id>`). Both write through to
//! the agent's `backend` column, so readers only ever look at that column.

use crate::avatar::AvatarSpec;
use crate::settings_providers::{ModelRole, ProvidersSettings};
use synapse_core::{AgentBackend, ApiBackend, CliBackend, ThinkingLevel};
use synapse_personas::PersonaDraft;
use synapse_store::{Agent, NewAgent, Store};

const DEFAULT_ENGINE_KEY: &str = "agents.default_engine";

/// What runs an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Engine {
    /// The default model for its role from Settings → Providers.
    Model,
    /// A provider, running whichever model is its default on the Models page.
    Provider(String),
    /// A specific provider and model chosen in the engine picker.
    Api { provider: String, model: String },
    /// An installed coding CLI, by its detected id (`claude`, `codex`, ...).
    /// `model` is the CLI's own model id; `None` runs the CLI's default.
    Cli { id: String, model: Option<String> },
}

impl Engine {
    fn to_setting(&self) -> String {
        match self {
            Self::Model => "model".into(),
            Self::Provider(id) => format!("provider:{id}"),
            Self::Api { provider, model } => format!("api:{provider}/{model}"),
            Self::Cli { id, model: None } => format!("cli:{id}"),
            Self::Cli { id, model: Some(model) } => format!("cli:{id}/{model}"),
        }
    }

    fn from_setting(raw: &str) -> Option<Self> {
        match raw.trim() {
            "model" => Some(Self::Model),
            other => {
                if let Some(id) = other.strip_prefix("provider:").filter(|id| !id.is_empty()) {
                    return Some(Self::Provider(id.to_string()));
                }
                if let Some((provider, model)) =
                    other.strip_prefix("api:").and_then(|r| r.split_once('/'))
                {
                    return (!provider.is_empty() && !model.is_empty()).then(|| Self::Api {
                        provider: provider.to_string(),
                        model: model.to_string(),
                    });
                }
                let rest = other.strip_prefix("cli:")?;
                // The model may itself contain '/', so split at the first one.
                let (id, model) = match rest.split_once('/') {
                    Some((id, model)) => (id, Some(model.to_string()).filter(|m| !m.is_empty())),
                    None => (rest, None),
                };
                (!id.is_empty()).then(|| Self::Cli { id: id.to_string(), model })
            }
        }
    }
}

fn override_key(agent_id: &str) -> String {
    format!("agent.engine.{agent_id}")
}

/// The app-wide default engine (a model unless the user chose a CLI).
pub fn default_engine(store: &Store) -> Engine {
    store
        .setting_get(DEFAULT_ENGINE_KEY)
        .ok()
        .flatten()
        .and_then(|raw| Engine::from_setting(&raw))
        .unwrap_or(Engine::Model)
}

/// This agent's own engine, if the user set one.
pub fn agent_override(store: &Store, agent_id: &str) -> Option<Engine> {
    store
        .setting_get(&override_key(agent_id))
        .ok()
        .flatten()
        .and_then(|raw| Engine::from_setting(&raw))
}

/// Backend JSON for an engine. A model engine reads the default model for
/// the agent's role; with none chosen yet the fields stay empty and the
/// first turn says to pick one in Settings → Providers.
fn backend_json(store: &Store, engine: &Engine, role: ModelRole) -> String {
    let api = |provider: String, model: String| {
        AgentBackend::Api(ApiBackend {
            provider,
            model,
            temperature: None,
            max_tokens: None,
            thinking: ThinkingLevel::Off,
        })
    };
    let backend = match engine {
        Engine::Api { provider, model } => api(provider.clone(), model.clone()),
        Engine::Provider(id) => api(
            id.clone(),
            crate::model_defaults::provider_default(store, id).unwrap_or_default(),
        ),
        Engine::Model => {
            let (provider, model) = ProvidersSettings::load(store)
                .ok()
                .and_then(|s| s.default_for(role))
                .unwrap_or_default();
            api(provider, model)
        }
        Engine::Cli { id, model } => AgentBackend::Cli(CliBackend {
            agent_id: id.clone(),
            command: None,
            args: Vec::new(),
            model: model
                .clone()
                .or_else(|| crate::model_defaults::cli_default(store, id)),
        }),
    };
    serde_json::to_string(&backend).unwrap_or_default()
}

fn role_for(agent: &Agent) -> ModelRole {
    if agent.id == "agent-lead" {
        ModelRole::Lead
    } else {
        ModelRole::Member
    }
}

/// Run `agent` on its own engine, else the default one.
fn apply_engine(store: &Store, agent: &Agent) -> Result<(), String> {
    let engine = agent_override(store, &agent.id).unwrap_or_else(|| default_engine(store));
    store
        .update_agent_backend(&agent.id, &backend_json(store, &engine, role_for(agent)))
        .map_err(|e| e.to_string())
}

/// Run every agent's engine through the settings again, so agents that follow
/// a provider or CLI default pick up a changed default.
pub fn reapply_all(store: &Store) -> Result<(), String> {
    for agent in store.list_agents().map_err(|e| e.to_string())? {
        apply_engine(store, &agent)?;
    }
    Ok(())
}

/// Set a provider's default model; agents on "provider default" follow it.
pub fn set_provider_default(store: &Store, provider: &str, model: &str) -> Result<(), String> {
    crate::model_defaults::set_provider_default(store, provider, model)?;
    follow_in_main_chat(store, provider, model);
    reapply_all(store)
}

/// The main chat talks to the Lead model: when it is on this provider, it
/// follows the provider's new default instead of staying on an older pick.
fn follow_in_main_chat(store: &Store, provider: &str, model: &str) {
    use crate::settings_providers::{ModelRole, ProvidersSettings};
    let Some(catalog) = crate::custom_providers::catalog_with_custom(Some(store)) else {
        return;
    };
    if let Ok(mut settings) = ProvidersSettings::load(store) {
        let on_provider = settings.default_for(ModelRole::Lead).is_some_and(|(p, _)| p == provider);
        if on_provider && settings.set_default(&catalog, ModelRole::Lead, provider, model).is_ok() {
            let _ = settings.save(store);
        }
    }
}

/// Set a CLI's default model (`None` = the CLI's own); agents on "CLI default" follow it.
pub fn set_cli_default(store: &Store, cli: &str, model: Option<&str>) -> Result<(), String> {
    crate::model_defaults::set_cli_default(store, cli, model)?;
    reapply_all(store)
}

/// Change the default engine; agents without their own engine follow it.
pub fn set_default_engine(store: &Store, engine: &Engine) -> Result<(), String> {
    store
        .setting_set(DEFAULT_ENGINE_KEY, &engine.to_setting())
        .map_err(|e| e.to_string())?;
    for agent in store.list_agents().map_err(|e| e.to_string())? {
        if agent_override(store, &agent.id).is_none() {
            apply_engine(store, &agent)?;
        }
    }
    Ok(())
}

/// Give one agent its own engine, or `None` to follow the default again.
pub fn set_agent_engine(
    store: &Store,
    agent_id: &str,
    engine: Option<&Engine>,
) -> Result<(), String> {
    let value = engine.map_or_else(|| "default".to_string(), Engine::to_setting);
    store
        .setting_set(&override_key(agent_id), &value)
        .map_err(|e| e.to_string())?;
    let agent = store.get_agent(agent_id).map_err(|e| e.to_string())?;
    apply_engine(store, &agent)
}

/// Take an agent off a project's team. The agent itself stays in the library
/// (other projects keep it) and can be added again.
pub fn remove_from_team(store: &Store, project_id: &str, agent_id: &str) -> Result<(), String> {
    store
        .remove_agent_from_project(project_id, agent_id)
        .map_err(|e| e.to_string())
}

/// Save a new avatar (JSON from the avatar dialog) for an agent.
pub fn set_avatar(store: &Store, agent_id: &str, avatar_json: &str) -> Result<(), String> {
    store
        .update_agent_avatar(agent_id, avatar_json)
        .map_err(|e| e.to_string())
}

/// Agents on a project's team, oldest first.
pub fn project_team(store: &Store, project_id: &str) -> Vec<Agent> {
    store.list_project_agents(project_id).unwrap_or_default()
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Add a persona to a project. The agent row is shared by every project that
/// uses the same persona, so editing it once applies everywhere.
/// The library agent for a persona, created (with its engine applied) the
/// first time it is needed. Agents are shared by every project.
pub fn ensure_agent(store: &Store, persona: &PersonaDraft) -> Result<Agent, String> {
    let id = agent_id(&persona.name);
    if let Ok(existing) = store.get_agent(&id) {
        return Ok(existing);
    }
    let role = persona.role.clone().unwrap_or_else(|| persona.name.clone());
    let created = store
        .create_agent(&NewAgent {
            id: id.clone(),
            name: persona.name.clone(),
            role,
            instructions: persona.instructions.clone(),
            backend: String::new(),
            avatar: AvatarSpec::from_name(&persona.name).to_json(),
            tools: serde_json::to_string(&persona.tools).unwrap_or_else(|_| "[]".into()),
            permissions: "[]".into(),
            persona_source: format!(
                "{}@{}:{} ({})",
                persona.source.repo, persona.source.sha, persona.source.path, persona.source.license
            ),
        })
        .map_err(|e| e.to_string())?;
    apply_engine(store, &created)?;
    store.get_agent(&id).map_err(|e| e.to_string())
}

/// The id a persona's agent has in the store.
pub fn agent_id(name: &str) -> String {
    format!("agent-{}", slug(name))
}

pub fn add_persona(store: &Store, project_id: &str, persona: &PersonaDraft) -> Result<Agent, String> {
    let agent = ensure_agent(store, persona)?;
    store
        .add_agent_to_project(project_id, &agent.id, "{}")
        .map_err(|e| e.to_string())?;
    Ok(agent)
}

fn seeded_key(project_id: &str) -> String {
    format!("project.team_seeded.{project_id}")
}

/// Every project starts with the starter team (lead, researcher, critic ...)
/// exactly once; a team the user later empties stays empty.
pub fn seed_default_team(store: &Store, project_id: &str) {
    for persona in synapse_personas::starter_personas() {
        let _ = add_persona(store, project_id, &persona);
    }
    let _ = store.setting_set(&seeded_key(project_id), "1");
}

/// Projects made before teams existed get the starter team the first time
/// they are opened, unless they already have agents or were seeded.
pub fn ensure_default_team(store: &Store, project_id: &str) {
    let seeded = store.setting_get(&seeded_key(project_id)).ok().flatten().is_some();
    if seeded {
        return;
    }
    if project_team(store, project_id).is_empty() {
        seed_default_team(store, project_id);
    } else {
        let _ = store.setting_set(&seeded_key(project_id), "1");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_setup::create_project;

    #[test]
    fn new_projects_start_with_the_default_team_and_follow_engine_settings() {
        let store = Store::open_in_memory().expect("store");
        let project = create_project(&store).expect("project").project;
        let team = project_team(&store, &project.id);
        assert!(team.iter().any(|a| a.name == "Lead"));
        assert!(team.len() >= 8);

        set_default_engine(&store, &Engine::Cli { id: "claude".into(), model: Some("sonnet".into()) }).unwrap();
        let lead = store.get_agent("agent-lead").unwrap();
        assert!(lead.backend.contains("claude") && lead.backend.contains("sonnet"));

        remove_from_team(&store, &project.id, "agent-lead").unwrap();
        assert!(!project_team(&store, &project.id).iter().any(|a| a.id == "agent-lead"));
        for agent in project_team(&store, &project.id) {
            remove_from_team(&store, &project.id, &agent.id).unwrap();
        }
        ensure_default_team(&store, &project.id);
        assert!(project_team(&store, &project.id).is_empty(), "emptied team stays empty");
        add_persona(&store, &project.id, &synapse_personas::starter_personas()[0]).unwrap();
        set_avatar(&store, "agent-critic", r#"{"pack":"notionists","seed":"x"}"#).unwrap();
        assert!(store.get_agent("agent-critic").unwrap().avatar.contains("notionists"));

        // An agent's own engine wins over the default, and clearing it follows the default.
        set_agent_engine(&store, "agent-lead", Some(&Engine::Model)).unwrap();
        assert!(store.get_agent("agent-lead").unwrap().backend.contains("api"));
        let picked = Engine::Api { provider: "openai".into(), model: "gpt-4o".into() };
        set_agent_engine(&store, "agent-lead", Some(&picked)).unwrap();
        assert!(store.get_agent("agent-lead").unwrap().backend.contains("gpt-4o"));
        assert_eq!(agent_override(&store, "agent-lead"), Some(picked));
        set_agent_engine(&store, "agent-lead", None).unwrap();
        assert!(store.get_agent("agent-lead").unwrap().backend.contains("claude"));

        // Provider and CLI defaults flow to agents that follow them, not to agents with a model of their own.
        set_default_engine(&store, &Engine::Provider("openai".into())).unwrap();
        set_agent_engine(&store, "agent-critic", Some(&Engine::Cli { id: "codex".into(), model: Some("gpt-5".into()) })).unwrap();
        set_agent_engine(&store, "agent-writer", Some(&Engine::Cli { id: "codex".into(), model: None })).unwrap();
        set_provider_default(&store, "openai", "gpt-4o").unwrap();
        assert!(store.get_agent("agent-analyst").unwrap().backend.contains("gpt-4o"));
        set_cli_default(&store, "codex", Some("gpt-5-codex")).unwrap();
        assert!(store.get_agent("agent-writer").unwrap().backend.contains("gpt-5-codex"));
        assert!(store.get_agent("agent-critic").unwrap().backend.contains("\"gpt-5\""));
    }
}
