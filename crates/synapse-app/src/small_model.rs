//! The small model: a cheap, fast model for quick jobs (naming a project,
//! writing its goal). Chosen on the Models page; the Lead model does these
//! jobs while none is set.

use crate::settings_providers::{ModelRole, ProvidersSettings};
use crate::team_setup::Engine;
use synapse_store::Store;

/// The saved `(provider, model)`, if any.
pub fn get(store: &Store) -> Option<(String, String)> {
    ProvidersSettings::load(store).ok()?.default_for(ModelRole::Small)
}

/// Save an API model as the small model; anything else clears it.
pub fn set(store: &Store, engine: Option<&Engine>) -> Result<(), String> {
    let mut settings = ProvidersSettings::load(store).map_err(|e| e.to_string())?;
    match engine {
        Some(Engine::Api { provider, model }) => {
            let catalog = crate::custom_providers::catalog_with_custom(Some(store))
                .ok_or("The model catalog is unavailable.")?;
            settings
                .set_default(&catalog, ModelRole::Small, provider, model)
                .map_err(|e| e.to_string())?;
        }
        _ => {
            settings.defaults.remove(ModelRole::Small.as_str());
        }
    }
    settings.save(store).map_err(|e| e.to_string())
}
