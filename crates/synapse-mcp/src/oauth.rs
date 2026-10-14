//! OAuth + secret storage.
//!
//! Tokens live only in a [`SecretStore`] (OS keychain in the app, in-memory
//! here); configs and logs hold just the `key_ref`. `Debug` never prints a
//! token value.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("secret not found: {0}")]
    NotFound(String),
    #[error("oauth flow failed: {0}")]
    Flow(String),
}

/// Minimal secret store. The app implements this on the OS keychain;
/// tests use [`InMemorySecretStore`].
pub trait SecretStore: Send + Sync {
    fn put(&self, key_ref: &str, value: SecretString);
    fn get(&self, key_ref: &str) -> Result<SecretString, SecretError>;
    fn delete(&self, key_ref: &str);
}

/// Non-persistent store for tests and for environments without a keychain.
#[derive(Debug, Default, Clone)]
pub struct InMemorySecretStore {
    inner: Arc<Mutex<HashMap<String, SecretString>>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper: read back the raw value. Production code must not use this.
    pub fn test_value(&self, key_ref: &str) -> Option<String> {
        self.inner
            .lock()
            .ok()?
            .get(key_ref)
            .map(|s| s.expose_secret().to_string())
    }
}

impl SecretStore for InMemorySecretStore {
    fn put(&self, key_ref: &str, value: SecretString) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(key_ref.to_string(), value);
        }
    }

    fn get(&self, key_ref: &str) -> Result<SecretString, SecretError> {
        self.inner
            .lock()
            .map_err(|_| SecretError::Flow("secret store poisoned".into()))?
            .get(key_ref)
            .cloned()
            .ok_or_else(|| SecretError::NotFound(key_ref.to_string()))
    }

    fn delete(&self, key_ref: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(key_ref);
        }
    }
}

/// OAuth authorization-code flow. The app implements the real HTTPS token
/// exchange; tests use [`FakeOAuthFlow`] (no network).
pub trait OAuthFlow: Send + Sync {
    /// URL to open in the browser for `server_id`.
    fn authorize_url(&self, server_id: &str) -> String;
    /// Exchange `code` for an access token (and optional refresh token).
    fn exchange_code(
        &self,
        server_id: &str,
        code: &str,
    ) -> Result<(SecretString, Option<SecretString>), SecretError>;
    /// Refresh with a refresh token.
    fn refresh(
        &self,
        server_id: &str,
        refresh_token: &SecretString,
    ) -> Result<SecretString, SecretError> {
        let _ = (server_id, refresh_token);
        Err(SecretError::Flow("refresh not supported".into()))
    }
}

/// Deterministic fake OAuth flow for tests: `code == "code-<server>"`
/// yields `token-for-<server>`.
#[derive(Debug, Default, Clone)]
pub struct FakeOAuthFlow;

impl FakeOAuthFlow {
    pub fn new() -> Self {
        Self
    }

    pub fn expected_code(server_id: &str) -> String {
        format!("code-{server_id}")
    }

    pub fn issued_token(server_id: &str) -> String {
        format!("token-for-{server_id}")
    }
}

impl OAuthFlow for FakeOAuthFlow {
    fn authorize_url(&self, server_id: &str) -> String {
        format!("https://auth.example.test/authorize?server={server_id}")
    }

    fn exchange_code(
        &self,
        server_id: &str,
        code: &str,
    ) -> Result<(SecretString, Option<SecretString>), SecretError> {
        if code == Self::expected_code(server_id) {
            Ok((
                SecretString::from(Self::issued_token(server_id)),
                Some(SecretString::from(format!("refresh-for-{server_id}"))),
            ))
        } else {
            Err(SecretError::Flow("invalid authorization code".into()))
        }
    }

    fn refresh(
        &self,
        server_id: &str,
        _refresh_token: &SecretString,
    ) -> Result<SecretString, SecretError> {
        Ok(SecretString::from(format!(
            "token-for-{server_id}-refreshed"
        )))
    }
}
