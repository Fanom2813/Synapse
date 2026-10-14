//! Secret store per docs/ARCHITECTURE.md §6.
//!
//! - One [`SecretStore`] trait; nothing else touches secrets.
//! - OS keychain via [`KeychainSecretStore`] (`keyring`), service
//!   `ai.synapse.<kind>`, account = provider/connector id.
//! - Encrypted-file fallback ([`EncryptedFileStore`]) for Linux without a
//!   Secret Service: `chacha20poly1305`, key derived with `argon2` from a
//!   passphrase the user sets once. Never a plaintext fallback.
//! - In memory: [`secrecy::SecretString`], read just before the request.
//! - Each key is sent only to its provider's own host ([`KeyClient`]).
//! - The DB, settings, exports and logs hold only [`ProviderRecord::key_ref`].
//! - UI masking via [`mask_key`]; remove deletes the keychain entry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("keychain: {0}")]
    Keychain(String),
    #[error("locked: wrong passphrase or corrupted file")]
    Locked,
    #[error("io: {0}")]
    Io(String),
    #[error("encode: {0}")]
    Encode(String),
}

pub type SecretResult<T> = std::result::Result<T, SecretError>;

/// Service name for one secret kind, e.g. `ai.synapse.provider`.
pub fn service_name(kind: &str) -> String {
    format!("ai.synapse.{kind}")
}

/// Reference stored in the DB instead of the value:
/// `ai.synapse.<kind>/<account>`.
pub fn key_ref_for(kind: &str, account: &str) -> String {
    format!("{}/{}", service_name(kind), account)
}

/// Split a `key_ref` back into `(service, account)`.
pub fn split_key_ref(key_ref: &str) -> Option<(&str, &str)> {
    key_ref.rsplit_once('/')
}

/// Masked UI value, e.g. `sk-…a1b2`. Never returns the full key.
pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "…".to_string();
    }
    let head: String = chars.iter().take(3).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

// -- store trait ------------------------------------------------------------

/// The only way to touch secrets (ARCHITECTURE.md §6).
pub trait SecretStore: Send + Sync {
    /// Store `value` under `account` (service = [`service_name`] of `kind`).
    fn set(&self, kind: &str, account: &str, value: &str) -> SecretResult<()>;
    /// Read a secret just before its request. Never cached in long-lived structs.
    fn get(&self, kind: &str, account: &str) -> SecretResult<Option<SecretString>>;
    /// Delete the entry (called when a provider is removed / app is reset).
    fn delete(&self, kind: &str, account: &str) -> SecretResult<()>;
    /// Accounts that have a secret for `kind` (ids only, never values).
    fn accounts(&self, kind: &str) -> Vec<String>;
}

// -- in-memory store (tests, and non-persisted session keys) ---------------

#[derive(Debug, Default)]
pub struct MemorySecretStore {
    inner: Mutex<HashMap<(String, String), String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn set(&self, kind: &str, account: &str, value: &str) -> SecretResult<()> {
        self.inner
            .lock()
            .expect("memory secrets")
            .insert((kind.to_string(), account.to_string()), value.to_string());
        Ok(())
    }

    fn get(&self, kind: &str, account: &str) -> SecretResult<Option<SecretString>> {
        Ok(self
            .inner
            .lock()
            .expect("memory secrets")
            .get(&(kind.to_string(), account.to_string()))
            .map(|v| SecretString::new(v.clone().into_boxed_str())))
    }

    fn delete(&self, kind: &str, account: &str) -> SecretResult<()> {
        self.inner
            .lock()
            .expect("memory secrets")
            .remove(&(kind.to_string(), account.to_string()));
        Ok(())
    }

    fn accounts(&self, kind: &str) -> Vec<String> {
        self.inner
            .lock()
            .expect("memory secrets")
            .keys()
            .filter(|(k, _)| k == kind)
            .map(|(_, a)| a.clone())
            .collect()
    }
}

// -- OS keychain store -------------------------------------------------------

/// OS credential store via `keyring` (Keychain / Credential Manager /
/// Secret Service). One entry per secret.
pub struct KeychainSecretStore;

impl KeychainSecretStore {
    pub fn new() -> Self {
        Self
    }

    fn entry(kind: &str, account: &str) -> SecretResult<keyring::Entry> {
        keyring::Entry::new(&service_name(kind), account)
            .map_err(|e| SecretError::Keychain(e.to_string()))
    }

    /// `true` when a Secret Service / keychain answers (probe entry).
    pub fn is_available() -> bool {
        match keyring::Entry::new("ai.synapse.probe", "probe") {
            Ok(e) => e.get_password().is_ok() || e.set_password("").is_ok(),
            Err(_) => false,
        }
    }
}

impl Default for KeychainSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for KeychainSecretStore {
    fn set(&self, kind: &str, account: &str, value: &str) -> SecretResult<()> {
        Self::entry(kind, account).and_then(|e| {
            e.set_password(value)
                .map_err(|e| SecretError::Keychain(e.to_string()))
        })
    }

    fn get(&self, kind: &str, account: &str) -> SecretResult<Option<SecretString>> {
        match Self::entry(kind, account)?.get_password() {
            Ok(v) => Ok(Some(SecretString::new(v.into_boxed_str()))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Keychain(e.to_string())),
        }
    }

    fn delete(&self, kind: &str, account: &str) -> SecretResult<()> {
        match Self::entry(kind, account)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Keychain(e.to_string())),
        }
    }

    fn accounts(&self, _kind: &str) -> Vec<String> {
        // The OS store cannot be enumerated; the DB's key_refs are the index.
        Vec::new()
    }
}

// -- encrypted-file fallback (Linux without Secret Service) -----------------

#[derive(Debug, Serialize, Deserialize)]
struct VaultFile {
    salt: Vec<u8>,
    nonce: Vec<u8>,
    data: Vec<u8>,
}

fn derive_key(passphrase: &str, salt: &[u8]) -> SecretResult<[u8; 32]> {
    let mut key = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| SecretError::Encode(e.to_string()))?;
    Ok(key)
}

fn random_bytes(n: usize) -> SecretResult<Vec<u8>> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|e| SecretError::Encode(e.to_string()))?;
    Ok(buf)
}

/// Encrypted-file fallback: the whole map is one AEAD blob. The file bytes
/// never contain a plaintext secret. The user sets `passphrase` once;
/// Settings shows a warning while this fallback is active.
pub struct EncryptedFileStore {
    path: PathBuf,
    passphrase: SecretString,
}

impl EncryptedFileStore {
    pub fn open(path: &Path, passphrase: SecretString) -> SecretResult<Self> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(|e| SecretError::Io(e.to_string()))?;
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            passphrase,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load_map(&self) -> SecretResult<HashMap<String, HashMap<String, String>>> {
        let raw = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(e) => return Err(SecretError::Io(e.to_string())),
        };
        let vault: VaultFile = serde_json::from_slice(&raw).map_err(|_| SecretError::Locked)?;
        let key = derive_key(self.passphrase.expose_secret(), &vault.salt)?;
        use chacha20poly1305::aead::{Aead, KeyInit};
        let cipher =
            chacha20poly1305::ChaCha20Poly1305::new(chacha20poly1305::Key::from_slice(&key));
        let nonce = chacha20poly1305::Nonce::from_slice(&vault.nonce);
        let plain = cipher
            .decrypt(nonce, vault.data.as_ref())
            .map_err(|_| SecretError::Locked)?;
        serde_json::from_slice(&plain).map_err(|_| SecretError::Locked)
    }

    fn save_map(&self, map: &HashMap<String, HashMap<String, String>>) -> SecretResult<()> {
        use chacha20poly1305::aead::{Aead, KeyInit};
        let plain = serde_json::to_vec(map).map_err(|e| SecretError::Encode(e.to_string()))?;
        // Keep the existing salt so the same passphrase keeps working.
        let salt = match std::fs::read(&self.path) {
            Ok(raw) => serde_json::from_slice::<VaultFile>(&raw)
                .map(|v| v.salt)
                .unwrap_or_else(|_| random_bytes(16).unwrap_or(vec![0; 16])),
            Err(_) => random_bytes(16)?,
        };
        let key = derive_key(self.passphrase.expose_secret(), &salt)?;
        let cipher =
            chacha20poly1305::ChaCha20Poly1305::new(chacha20poly1305::Key::from_slice(&key));
        let nonce_bytes = random_bytes(12)?;
        let nonce = chacha20poly1305::Nonce::from_slice(&nonce_bytes);
        let data = cipher
            .encrypt(nonce, plain.as_ref())
            .map_err(|e| SecretError::Encode(e.to_string()))?;
        let vault = VaultFile {
            salt,
            nonce: nonce_bytes,
            data,
        };
        let out = serde_json::to_vec(&vault).map_err(|e| SecretError::Encode(e.to_string()))?;
        // Atomic write: temp file + rename, never partial secrets on disk.
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &out).map_err(|e| SecretError::Io(e.to_string()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| SecretError::Io(e.to_string()))?;
        Ok(())
    }
}

impl SecretStore for EncryptedFileStore {
    fn set(&self, kind: &str, account: &str, value: &str) -> SecretResult<()> {
        let mut map = self.load_map()?;
        map.entry(kind.to_string())
            .or_default()
            .insert(account.to_string(), value.to_string());
        self.save_map(&map)
    }

    fn get(&self, kind: &str, account: &str) -> SecretResult<Option<SecretString>> {
        Ok(self
            .load_map()?
            .get(kind)
            .and_then(|m| m.get(account))
            .map(|v| SecretString::new(v.clone().into_boxed_str())))
    }

    fn delete(&self, kind: &str, account: &str) -> SecretResult<()> {
        let mut map = self.load_map()?;
        if let Some(m) = map.get_mut(kind) {
            m.remove(account);
        }
        self.save_map(&map)
    }

    fn accounts(&self, kind: &str) -> Vec<String> {
        self.load_map()
            .ok()
            .and_then(|m| m.get(kind).cloned())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}

// -- provider record (DB holds only key_ref) ---------------------------------

/// Row mirror of the `providers` table (ARCHITECTURE.md §3): the key itself
/// never appears here — only `key_ref`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub id: String,
    pub catalog_id: String,
    pub base_url: String,
    pub enabled: bool,
    /// Keychain entry name (`ai.synapse.provider/<id>`), never the key.
    pub key_ref: Option<String>,
}

impl ProviderRecord {
    pub fn new(id: &str, catalog_id: &str, base_url: &str) -> Self {
        Self {
            id: id.to_string(),
            catalog_id: catalog_id.to_string(),
            base_url: base_url.to_string(),
            enabled: true,
            key_ref: Some(key_ref_for("provider", id)),
        }
    }

    pub fn without_key(id: &str, catalog_id: &str, base_url: &str) -> Self {
        Self {
            id: id.to_string(),
            catalog_id: catalog_id.to_string(),
            base_url: base_url.to_string(),
            enabled: true,
            key_ref: None,
        }
    }

    /// Remove provider: clears the ref and deletes the keychain entry.
    pub fn remove_from(&mut self, store: &dyn SecretStore) -> SecretResult<()> {
        if self.key_ref.is_some() {
            store.delete("provider", &self.id)?;
            self.key_ref = None;
        }
        Ok(())
    }

    pub fn ensure_providers_table(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY,
                catalog_id TEXT NOT NULL DEFAULT '',
                base_url TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1,
                key_ref TEXT
            )",
        )?;
        Ok(())
    }

    pub fn save(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        Self::ensure_providers_table(conn)?;
        conn.execute(
            "INSERT INTO providers (id, catalog_id, base_url, enabled, key_ref)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET catalog_id=excluded.catalog_id,
               base_url=excluded.base_url, enabled=excluded.enabled, key_ref=excluded.key_ref",
            rusqlite::params![
                self.id,
                self.catalog_id,
                self.base_url,
                self.enabled as i32,
                self.key_ref
            ],
        )?;
        Ok(())
    }

    /// Export/backup payload: ids, URLs and refs only — never secret values.
    pub fn export_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "catalog_id": self.catalog_id,
            "base_url": self.base_url,
            "enabled": self.enabled,
            "key_ref": self.key_ref,
            "has_key": self.key_ref.is_some(),
        })
    }
}

// -- per-host key sending -----------------------------------------------------

fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    // Strip userinfo and keep host[:port], lowercased.
    let host_port = host_port.rsplit('@').next().unwrap_or(host_port);
    if host_port.is_empty() {
        return None;
    }
    Some(host_port.to_lowercase())
}

/// Reads secrets just before a request and attaches a key only when the
/// request URL's host matches the provider's own base-URL host.
pub struct KeyClient<'a> {
    store: &'a dyn SecretStore,
    hosts: HashMap<String, String>,
}

impl<'a> KeyClient<'a> {
    pub fn new(store: &'a dyn SecretStore) -> Self {
        Self {
            store,
            hosts: HashMap::new(),
        }
    }

    /// Register a provider's own host from its catalog/custom base URL.
    pub fn register(&mut self, provider_id: &str, base_url: &str) {
        if let Some(h) = host_of(base_url) {
            self.hosts.insert(provider_id.to_string(), h);
        }
    }

    /// `Some(Bearer value)` only when `request_url` is on the provider's own
    /// host; `None` otherwise (never sent to another host).
    pub fn auth_header_for(&self, provider_id: &str, request_url: &str) -> Option<SecretString> {
        let want = self.hosts.get(provider_id)?;
        let got = host_of(request_url)?;
        if got != *want {
            return None;
        }
        self.store.get("provider", provider_id).ok()?
    }
}

/// Build a [`synapse_core::Redactor`] stocked with every secret in `store`
/// (plus the given kinds), for logs / tool args / exports / error messages.
pub fn redactor_for_store(
    store: &dyn SecretStore,
    kinds: &[&str],
    extra: &[(&str, &str)],
) -> synapse_core::Redactor {
    let mut r = synapse_core::Redactor::new();
    for kind in kinds {
        for account in store.accounts(kind) {
            if let Ok(Some(v)) = store.get(kind, &account) {
                r.add_secret(format!("{kind}/{account}"), v.expose_secret().to_string());
            }
        }
    }
    for (name, value) in extra {
        r.add_secret(*name, *value);
    }
    r
}

#[cfg(test)]
mod keys_local_tests {
    use super::*;
    use secrecy::ExposeSecret;

    const KIND: &str = "provider";

    #[test]
    fn keys_local_memory_roundtrip_delete_and_mask() {
        let store = MemorySecretStore::new();
        assert!(store.get(KIND, "openai").unwrap().is_none());
        store.set(KIND, "openai", "sk-test-secret-value").unwrap();
        let got = store.get(KIND, "openai").unwrap().unwrap();
        assert_eq!(got.expose_secret(), "sk-test-secret-value");
        // In memory: Debug never prints the value.
        assert!(!format!("{got:?}").contains("sk-test-secret-value"));
        assert!(store.accounts(KIND).contains(&"openai".to_string()));
        // Masked UI value.
        let masked = mask_key("sk-abcdef1234a1b2");
        assert!(!masked.contains("abcdef1234"));
        assert!(
            masked.starts_with("sk-") && masked.ends_with("a1b2"),
            "{masked}"
        );
        // Key refs carry no value.
        let r = key_ref_for(KIND, "openai");
        assert!(!r.contains("sk-test"));
        assert_eq!(split_key_ref(&r), Some(("ai.synapse.provider", "openai")));
        assert_eq!(service_name(KIND), "ai.synapse.provider");
        // Delete on remove.
        store.delete(KIND, "openai").unwrap();
        assert!(store.get(KIND, "openai").unwrap().is_none());
    }

    #[test]
    fn keys_local_db_export_log_hold_only_ref() {
        let secret = "sk-proj-f4k3T3stOpenAIKey0123456789abcdefXYZ";
        let store = MemorySecretStore::new();
        store.set(KIND, "openai", secret).unwrap();

        // DB row holds only the ref.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let mut rec = ProviderRecord::new("openai", "openai", "https://api.openai.com/v1");
        rec.save(&conn).unwrap();
        let raw: String = conn
            .query_row(
                "SELECT id || '|' || catalog_id || '|' || base_url || '|' || ifnull(key_ref,'') FROM providers",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(raw.contains("ai.synapse.provider/openai"), "{raw}");
        assert!(!raw.contains(secret), "secret value reached the DB");

        // Export/backup holds no value.
        let export = rec.export_json().to_string();
        assert!(!export.contains(secret), "secret value reached the export");
        assert!(export.contains("ai.synapse.provider/openai"));

        // Logs: everything is scrubbed through the store-fed redactor.
        let redactor = redactor_for_store(&store, &[KIND], &[]);
        let log_line = format!("POST https://api.openai.com/v1/chat key={secret} done");
        let scrubbed = redactor.redact(&log_line);
        assert!(scrubbed.count >= 1);
        assert!(
            !scrubbed.text.contains(secret),
            "secret value reached the logs"
        );

        // Serialized turn requests skip the key field entirely.
        let req =
            crate::TurnRequest::new("gpt-4o-mini").with_message(crate::TurnMessage::user("hi"));
        let req_json = serde_json::to_string(&req).unwrap();
        assert!(!req_json.contains("api_key"));

        // Remove provider deletes the secret.
        rec.remove_from(&store).unwrap();
        assert!(rec.key_ref.is_none());
        assert!(store.get(KIND, "openai").unwrap().is_none());
    }

    #[tokio::test]
    async fn keys_local_key_only_to_own_host() {
        use wiremock::{
            matchers::{header, method, path},
            Mock, MockServer, ResponseTemplate,
        };

        let secret = "sk-live-owner-key-AAAABBBBCCCC";
        let store = MemorySecretStore::new();
        store.set(KIND, "openai", secret).unwrap();

        let owner = MockServer::start().await;
        let other = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat"))
            .and(header("authorization", format!("Bearer {secret}")))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&owner)
            .await;
        // The other host must never see the key: any request carrying it fails.
        Mock::given(method("POST"))
            .and(header("authorization", format!("Bearer {secret}")))
            .respond_with(ResponseTemplate::new(500).set_body_string("key leaked"))
            .expect(0)
            .mount(&other)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok-no-auth"))
            .mount(&other)
            .await;

        let mut keys = KeyClient::new(&store);
        keys.register("openai", &format!("{}/v1", owner.uri()));

        let client = reqwest::Client::new();
        // Own host: key attached.
        let auth = keys
            .auth_header_for("openai", &format!("{}/v1/chat", owner.uri()))
            .expect("key must be sent to its own host");
        let resp = client
            .post(format!("{}/v1/chat", owner.uri()))
            .header("authorization", format!("Bearer {}", auth.expose_secret()))
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // Another host: no key, even for the same provider id.
        assert!(
            keys.auth_header_for("openai", &format!("{}/v1/chat", other.uri()))
                .is_none(),
            "key must never be sent to another host"
        );
        let resp = client
            .post(format!("{}/v1/chat", other.uri()))
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        // Unregistered provider: no key anywhere.
        assert!(keys
            .auth_header_for("nope", &format!("{}/v1/chat", owner.uri()))
            .is_none());
    }

    #[test]
    fn keys_local_encrypted_file_roundtrip_and_locked() {
        use secrecy::SecretString;
        let dir = std::env::temp_dir().join(format!("synapse-keys-{}", std::process::id()));
        let path = dir.join("vault.json");
        let _ = std::fs::remove_file(&path);
        let secret = "sk-file-fallback-secret-12345";

        let store =
            EncryptedFileStore::open(&path, SecretString::new("correct horse".into())).unwrap();
        store.set(KIND, "openai", secret).unwrap();
        assert_eq!(
            store.get(KIND, "openai").unwrap().unwrap().expose_secret(),
            secret
        );
        assert!(store.accounts(KIND).contains(&"openai".to_string()));

        // File bytes never contain the plaintext secret.
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(secret.len()).any(|w| w == secret.as_bytes()));
        assert!(!String::from_utf8_lossy(&raw).contains(secret));

        // Wrong passphrase: locked, no value.
        let wrong =
            EncryptedFileStore::open(&path, SecretString::new("wrong passphrase".into())).unwrap();
        assert!(matches!(
            wrong.get(KIND, "openai"),
            Err(SecretError::Locked)
        ));

        // Delete removes the value (ciphertext rotates on save).
        store.delete(KIND, "openai").unwrap();
        assert!(store.get(KIND, "openai").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_local_redaction_scrubs_fixture_corpus() {
        // Every fake-key fixture must be fully redacted once it lives in the store.
        let store = MemorySecretStore::new();
        for (name, value) in synapse_core::redact::fixtures::all() {
            if value.len() >= 4 {
                store.set("provider", name, value).unwrap();
            }
        }
        let redactor = redactor_for_store(&store, &["provider"], &[]);
        for (name, value) in synapse_core::redact::fixtures::all() {
            let out = redactor.redact(&format!("tool args {value} end"));
            assert!(out.count >= 1, "fixture {name} was not redacted");
            assert!(!out.text.contains(value), "fixture {name} leaked");
        }
    }
}
