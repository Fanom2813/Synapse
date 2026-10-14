//! Builds the team the main chat talks to: the open project's members, and
//! for each one the model that runs them. Models are looked up when a member
//! is asked to speak, so a changed key, default or engine applies to the very
//! next message without rebuilding anything.

use crate::settings_providers::{ModelRole, ProvidersSettings};
use crate::team_run::{Resolver, TeamEnv};
use secrecy::ExposeSecret as _;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use synapse_core::agents::AgentBackend;
use synapse_core::team_dialogue::{brief_of, Limits, Member};
use synapse_llm::keys::SecretStore as _;
use synapse_llm::GenaiBackend;

const KEY_CACHE_TTL: Duration = Duration::from_secs(60);

/// The lead's agent id (the starter team's chief of staff).
const LEAD_ID: &str = "agent-lead";

/// The team for `project_id`, or `None` when it has no members yet.
pub fn team_env(
    store: &synapse_store::Store,
    project_id: &str,
    project_name: &str,
    goal: &str,
) -> Option<Arc<TeamEnv<GenaiBackend>>> {
    let members: Vec<Member> = crate::team_setup::project_team(store, project_id)
        .into_iter()
        .map(|a| Member {
            brief: brief_of(&a.role, &a.instructions),
            id: a.id,
            name: a.name,
            role: a.role,
            instructions: a.instructions,
            avatar: a.avatar,
        })
        .collect();
    let lead_id = members.iter().find(|m| m.id == LEAD_ID).or(members.first())?.id.clone();
    let resolve: Resolver<GenaiBackend> = Arc::new(resolve_member);
    Some(Arc::new(TeamEnv {
        project: project_name.to_string(),
        goal: goal.to_string(),
        members,
        lead_id,
        resolve,
        limits: Limits::default(),
    }))
}

/// Forget cached keys (call after a key is saved or removed).
pub fn forget_keys() {
    if let Ok(mut cache) = key_cache().lock() {
        cache.clear();
    }
}

fn key_cache() -> &'static Mutex<HashMap<String, (Instant, Option<String>)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, Option<String>)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The key for a provider, read from the keychain at most once a minute.
fn provider_key(store: &synapse_store::Store, provider: &str) -> Option<String> {
    if let Ok(cache) = key_cache().lock() {
        if let Some((at, key)) = cache.get(provider) {
            if at.elapsed() < KEY_CACHE_TTL {
                return key.clone();
            }
        }
    }
    let key = crate::provider_setup::key_saved(store, provider)
        .then(|| {
            synapse_llm::keys::KeychainSecretStore
                .get("provider", provider)
                .ok()
                .flatten()
                .map(|s| s.expose_secret().to_string())
        })
        .flatten();
    if let Ok(mut cache) = key_cache().lock() {
        cache.insert(provider.to_string(), (Instant::now(), key.clone()));
    }
    key
}

/// The provider and model that run an agent: its own API engine when it has
/// one, else the lead's default model (coding CLIs are not run from the chat
/// yet, so a CLI agent speaks through the default model).
fn model_for(
    store: &synapse_store::Store,
    settings: &ProvidersSettings,
    agent_id: &str,
) -> Option<(String, String)> {
    let own = store
        .get_agent(agent_id)
        .ok()
        .and_then(|a| serde_json::from_str::<AgentBackend>(&a.backend).ok())
        .and_then(|backend| match backend {
            AgentBackend::Api(api) if !api.provider.is_empty() && !api.model.is_empty() => {
                Some((api.provider, api.model))
            }
            _ => None,
        });
    own.or_else(|| settings.default_for(ModelRole::Lead))
}

fn resolve_member(agent_id: &str) -> Option<(Arc<GenaiBackend>, String)> {
    let store = crate::window::open_store()?;
    let catalog = crate::custom_providers::catalog_with_custom(Some(&store))?;
    let settings = ProvidersSettings::load(&store).ok()?;
    let (provider, model) = model_for(&store, &settings, agent_id)?;
    let base_url = settings.base_url_for(&catalog, &provider);
    let key = provider_key(&store, &provider);
    Some((Arc::new(GenaiBackend::new(model.clone(), key, base_url)), model))
}
