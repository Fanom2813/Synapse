//! Persisted settings on top of the `settings` table (ARCHITECTURE.md §3).
//!
//! One source of truth for each key: [`SettingKey`] owns the key string,
//! [`AppSettings`] owns the typed values. Stored as JSON strings via
//! `synapse-store`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("store: {0}")]
    Store(#[from] synapse_store::StoreError),
    #[error("invalid {key}: {message}")]
    Invalid { key: &'static str, message: String },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SettingsError>;

/// Every persisted setting key, in one place (AGENTS.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    Theme,
    DestroyHiddenWindowAfterSecs,
    RedactionEnabled,
}

impl SettingKey {
    pub fn as_str(self) -> &'static str {
        match self {
            SettingKey::Theme => "theme",
            SettingKey::DestroyHiddenWindowAfterSecs => "destroy_hidden_window_after_secs",
            SettingKey::RedactionEnabled => "redaction_enabled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl std::str::FromStr for Theme {
    type Err = SettingsError;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "system" => Ok(Theme::System),
            "light" => Ok(Theme::Light),
            "dark" => Ok(Theme::Dark),
            other => Err(SettingsError::Invalid {
                key: "theme",
                message: format!("unknown theme '{other}', expected system|light|dark"),
            }),
        }
    }
}

/// Typed app settings. Defaults match docs/UI.md + ARCHITECTURE.md §5.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: Theme,
    /// Seconds a hidden window is kept before being destroyed (default 60).
    pub destroy_hidden_window_after_secs: u64,
    /// Secret redactor on by default; never off for logs.
    pub redaction_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            destroy_hidden_window_after_secs: 60,
            redaction_enabled: true,
        }
    }
}

impl AppSettings {
    /// Load from the store; missing keys fall back to defaults.
    pub fn load(store: &synapse_store::Store) -> Result<Self> {
        let mut out = Self::default();
        if let Some(raw) = store.setting_get(SettingKey::Theme.as_str())? {
            let v: serde_json::Value = serde_json::from_str(&raw)?;
            let s = v.as_str().unwrap_or(&raw);
            out.theme = s.parse()?;
        }
        if let Some(raw) = store.setting_get(SettingKey::DestroyHiddenWindowAfterSecs.as_str())? {
            let v: serde_json::Value = serde_json::from_str(&raw)?;
            let n = v
                .as_u64()
                .or_else(|| raw.trim().parse().ok())
                .ok_or_else(|| SettingsError::Invalid {
                    key: "destroy_hidden_window_after_secs",
                    message: format!("expected a non-negative integer, got '{raw}'"),
                })?;
            out.destroy_hidden_window_after_secs = n;
        }
        if let Some(raw) = store.setting_get(SettingKey::RedactionEnabled.as_str())? {
            let v: serde_json::Value = serde_json::from_str(&raw)?;
            let b = v
                .as_bool()
                .or_else(|| raw.trim().parse().ok())
                .ok_or_else(|| SettingsError::Invalid {
                    key: "redaction_enabled",
                    message: format!("expected true|false, got '{raw}'"),
                })?;
            out.redaction_enabled = b;
        }
        out.validate()?;
        Ok(out)
    }

    /// Validate + persist all fields.
    pub fn save(&self, store: &synapse_store::Store) -> Result<()> {
        self.validate()?;
        let theme = match self.theme {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        };
        store.setting_set(SettingKey::Theme.as_str(), &serde_json::to_string(theme)?)?;
        store.setting_set(
            SettingKey::DestroyHiddenWindowAfterSecs.as_str(),
            &serde_json::to_string(&self.destroy_hidden_window_after_secs)?,
        )?;
        store.setting_set(
            SettingKey::RedactionEnabled.as_str(),
            &serde_json::to_string(&self.redaction_enabled)?,
        )?;
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.destroy_hidden_window_after_secs > 3600 {
            return Err(SettingsError::Invalid {
                key: "destroy_hidden_window_after_secs",
                message: "must be 0..=3600 seconds".to_string(),
            });
        }
        Ok(())
    }

    /// Cycle system → light → dark (theme switch in the title bar).
    pub fn cycle_theme(&mut self) {
        self.theme = match self.theme {
            Theme::System => Theme::Light,
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::System,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().unwrap()
    }

    #[test]
    fn settings_defaults_match_docs() {
        let s = AppSettings::default();
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.destroy_hidden_window_after_secs, 60);
        assert!(s.redaction_enabled);
    }

    #[test]
    fn settings_roundtrip_persists() {
        let store = mem_store();
        let loaded = AppSettings::load(&store).unwrap();
        assert_eq!(loaded, AppSettings::default());
        let mut s = loaded;
        s.theme = Theme::Dark;
        s.destroy_hidden_window_after_secs = 120;
        s.save(&store).unwrap();
        let again = AppSettings::load(&store).unwrap();
        assert_eq!(again.theme, Theme::Dark);
        assert_eq!(again.destroy_hidden_window_after_secs, 120);
    }

    #[test]
    fn settings_theme_validation_rejects_unknown() {
        let store = mem_store();
        store.setting_set("theme", "\"neon\"").unwrap();
        let err = AppSettings::load(&store).unwrap_err();
        assert!(err.to_string().contains("theme"));
        let s = AppSettings {
            theme: "neon".parse().unwrap_or(Theme::System),
            ..AppSettings::default()
        };
        assert_eq!(s.theme, Theme::System);
    }

    #[test]
    fn settings_cycle_theme_persists() {
        let store = mem_store();
        let mut s = AppSettings::load(&store).unwrap();
        s.cycle_theme();
        assert_eq!(s.theme, Theme::Light);
        s.save(&store).unwrap();
        assert_eq!(AppSettings::load(&store).unwrap().theme, Theme::Light);
    }

    #[test]
    fn settings_invalid_window_timeout_rejected() {
        let store = mem_store();
        let s = AppSettings {
            destroy_hidden_window_after_secs: 99999,
            ..AppSettings::default()
        };
        assert!(s.save(&store).is_err());
    }
}
