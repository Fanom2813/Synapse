//! Typed models.dev catalog: embedded gz snapshot + daily ETag refresh.
//!
//! Snapshot lives in `assets/models.json.gz` (trimmed to the fields below,
//! see docs/STACK.md). Refresh state lives in the `catalog_cache` table
//! (ARCHITECTURE.md §3): single row `id=1 (etag, fetched_at, blob gz)`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CATALOG_URL: &str = "https://models.dev/api.json";
/// Refresh at most once per day (PERFORMANCE.md §2.9).
pub const STALE_AFTER_SECS: i64 = 24 * 60 * 60;
const EMBEDDED_GZ: &[u8] = include_bytes!("../../../assets/models.json.gz");

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("gzip: {0}")]
    Gzip(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("unknown model: {0}/{1}")]
    UnknownModel(String, String),
}

pub type Result<T> = std::result::Result<T, CatalogError>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelLimit {
    #[serde(default)]
    pub context: Option<u64>,
    #[serde(default)]
    pub output: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelCost {
    /// USD per 1M tokens.
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Modalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CatalogModel {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub attachment: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub structured_output: bool,
    #[serde(default)]
    pub temperature: bool,
    #[serde(default)]
    pub open_weights: bool,
    #[serde(default)]
    pub modalities: Modalities,
    #[serde(default)]
    pub limit: ModelLimit,
    #[serde(default)]
    pub cost: ModelCost,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Provider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub models: HashMap<String, CatalogModel>,
}

/// Capability summary used by pickers and the orchestrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub tool_call: bool,
    pub reasoning: bool,
    pub attachment: bool,
    pub structured_output: bool,
    pub vision: bool,
    pub open_weights: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(flatten)]
    pub providers: HashMap<String, Provider>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshOutcome {
    pub updated: bool,
    pub from_cache: bool,
    pub etag: String,
}

fn gunzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut d = flate2::read::GzDecoder::new(data);
    let mut out = Vec::new();
    d.read_to_end(&mut out)?;
    Ok(out)
}

fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data)?;
    Ok(e.finish()?)
}

impl Catalog {
    pub fn from_json(json: &[u8]) -> Result<Self> {
        Ok(serde_json::from_slice(json)?)
    }

    pub fn from_gz(gz: &[u8]) -> Result<Self> {
        Self::from_json(&gunzip(gz)?)
    }

    /// Offline-first snapshot embedded in the binary.
    pub fn embedded() -> Result<Self> {
        Self::from_gz(EMBEDDED_GZ)
    }

    pub fn provider_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.providers.keys().map(String::as_str).collect();
        ids.sort_unstable();
        ids
    }

    pub fn providers(&self) -> Vec<&Provider> {
        let mut v: Vec<&Provider> = self.providers.values().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        v
    }

    pub fn provider(&self, id: &str) -> Result<&Provider> {
        self.providers
            .get(id)
            .ok_or_else(|| CatalogError::UnknownProvider(id.into()))
    }

    pub fn model(&self, provider_id: &str, model_id: &str) -> Result<&CatalogModel> {
        let p = self.provider(provider_id)?;
        p.models
            .get(model_id)
            .ok_or_else(|| CatalogError::UnknownModel(provider_id.into(), model_id.into()))
    }

    pub fn models(&self, provider_id: &str) -> Result<Vec<&CatalogModel>> {
        let p = self.provider(provider_id)?;
        let mut v: Vec<&CatalogModel> = p.models.values().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(v)
    }

    pub fn model_count(&self) -> usize {
        self.providers.values().map(|p| p.models.len()).sum()
    }

    /// Case-insensitive substring search over `provider/model id + name`.
    pub fn search_models(&self, query: &str, limit: usize) -> Vec<(&str, &CatalogModel)> {
        let q = query.to_lowercase();
        let mut hits = Vec::new();
        for (pid, p) in &self.providers {
            for m in p.models.values() {
                let hay = format!("{pid}/{} {} {}", m.id, m.name, p.name).to_lowercase();
                if q.is_empty() || hay.contains(&q) {
                    hits.push((pid.as_str(), m));
                }
            }
        }
        hits.sort_by(|a, b| format!("{}/{}", a.0, a.1.id).cmp(&format!("{}/{}", b.0, b.1.id)));
        hits.truncate(limit);
        hits
    }

    pub fn context_limit(&self, provider_id: &str, model_id: &str) -> Result<Option<u64>> {
        Ok(self.model(provider_id, model_id)?.limit.context)
    }

    pub fn output_limit(&self, provider_id: &str, model_id: &str) -> Result<Option<u64>> {
        Ok(self.model(provider_id, model_id)?.limit.output)
    }

    pub fn cost(&self, provider_id: &str, model_id: &str) -> Result<&ModelCost> {
        Ok(&self.model(provider_id, model_id)?.cost)
    }

    pub fn capabilities(&self, provider_id: &str, model_id: &str) -> Result<Capabilities> {
        let m = self.model(provider_id, model_id)?;
        Ok(Capabilities {
            tool_call: m.tool_call,
            reasoning: m.reasoning,
            attachment: m.attachment,
            structured_output: m.structured_output,
            vision: m.modalities.input.iter().any(|x| x == "image"),
            open_weights: m.open_weights,
        })
    }

    /// USD cost for a turn. Prices are per 1M tokens.
    pub fn estimate_cost(
        &self,
        provider_id: &str,
        model_id: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<f64> {
        let c = self.cost(provider_id, model_id)?;
        let i = c.input.unwrap_or(0.0) * input_tokens as f64 / 1_000_000.0;
        let o = c.output.unwrap_or(0.0) * output_tokens as f64 / 1_000_000.0;
        Ok(i + o)
    }

    /// Base URL for API calls (`provider.api`, with `${VAR}` left for the caller).
    pub fn base_url(&self, provider_id: &str) -> Result<Option<&str>> {
        Ok(self.provider(provider_id)?.api.as_deref())
    }

    /// Env var names that hold this provider's key.
    pub fn env_keys(&self, provider_id: &str) -> Result<&[String]> {
        Ok(self.provider(provider_id)?.env.as_slice())
    }
}

// -- cache (catalog_cache table) -------------------------------------------

pub fn ensure_cache_table(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS catalog_cache (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            etag TEXT NOT NULL DEFAULT '',
            fetched_at INTEGER NOT NULL DEFAULT 0,
            blob BLOB NOT NULL DEFAULT x''
        )",
    )?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct CachedSnapshot {
    pub etag: String,
    pub fetched_at: i64,
    pub blob: Vec<u8>,
}

pub fn read_cache(conn: &rusqlite::Connection) -> Result<Option<CachedSnapshot>> {
    ensure_cache_table(conn)?;
    let mut st = conn.prepare("SELECT etag, fetched_at, blob FROM catalog_cache WHERE id=1")?;
    let mut rows = st.query([])?;
    if let Some(r) = rows.next()? {
        Ok(Some(CachedSnapshot {
            etag: r.get(0)?,
            fetched_at: r.get(1)?,
            blob: r.get(2)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn write_cache(
    conn: &rusqlite::Connection,
    etag: &str,
    fetched_at: i64,
    gz_blob: &[u8],
) -> Result<()> {
    ensure_cache_table(conn)?;
    conn.execute(
        "INSERT INTO catalog_cache (id, etag, fetched_at, blob) VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET etag=excluded.etag, fetched_at=excluded.fetched_at, blob=excluded.blob",
        rusqlite::params![etag, fetched_at, gz_blob],
    )?;
    Ok(())
}

pub fn touch_cache_time(conn: &rusqlite::Connection, fetched_at: i64) -> Result<()> {
    ensure_cache_table(conn)?;
    conn.execute(
        "INSERT INTO catalog_cache (id, etag, fetched_at, blob) VALUES (1, '', ?1, x'')
         ON CONFLICT(id) DO UPDATE SET fetched_at=excluded.fetched_at",
        rusqlite::params![fetched_at],
    )?;
    Ok(())
}

pub fn is_stale(fetched_at: i64, now_unix: i64) -> bool {
    now_unix - fetched_at >= STALE_AFTER_SECS
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Load the best catalog available without network: cache blob, else embedded.
pub fn load_cached_or_embedded(conn: &rusqlite::Connection) -> Result<Catalog> {
    if let Some(snap) = read_cache(conn)? {
        if !snap.blob.is_empty() {
            if let Ok(cat) = Catalog::from_gz(&snap.blob) {
                return Ok(cat);
            }
        }
    }
    Catalog::embedded()
}

/// Refresh from `url` when the cache is missing or older than a day.
/// Sends `If-None-Match` when we have an ETag; a `304` just bumps `fetched_at`.
pub async fn refresh_if_stale(
    conn: &rusqlite::Connection,
    client: &reqwest::Client,
    url: &str,
    now_unix_time: i64,
) -> Result<(Catalog, RefreshOutcome)> {
    let snap = read_cache(conn)?;
    if let Some(s) = snap.clone() {
        if !s.blob.is_empty() && !is_stale(s.fetched_at, now_unix_time) {
            if let Ok(cat) = Catalog::from_gz(&s.blob) {
                return Ok((
                    cat,
                    RefreshOutcome {
                        updated: false,
                        from_cache: true,
                        etag: s.etag,
                    },
                ));
            }
        }
    }
    let mut req = client.get(url);
    if let Some(s) = &snap {
        if !s.etag.is_empty() {
            req = req.header("If-None-Match", s.etag.clone());
        }
    }
    let resp = req.send().await?;
    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        touch_cache_time(conn, now_unix_time)?;
        let cat = load_cached_or_embedded(conn)?;
        return Ok((
            cat,
            RefreshOutcome {
                updated: false,
                from_cache: true,
                etag: snap.map(|s| s.etag).unwrap_or_default(),
            },
        ));
    }
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = resp.error_for_status()?.bytes().await?;
    let cat = Catalog::from_json(&body)?;
    write_cache(conn, &etag, now_unix_time, &gzip(&body)?)?;
    Ok((
        cat,
        RefreshOutcome {
            updated: true,
            from_cache: false,
            etag,
        },
    ))
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    #[test]
    fn catalog_embedded_loads_providers_and_models() {
        let cat = Catalog::embedded().unwrap();
        assert!(
            cat.providers.len() >= 200,
            "providers: {}",
            cat.providers.len()
        );
        assert!(cat.model_count() >= 5000, "models: {}", cat.model_count());
        let p = cat.provider("anthropic").unwrap();
        assert!(!p.models.is_empty());
        assert!(p.env.iter().any(|e| e.contains("ANTHROPIC")));
    }

    #[test]
    fn catalog_typed_lookups_limits_cost_capabilities() {
        let cat = Catalog::embedded().unwrap();
        // Find a well-known model dynamically so the snapshot can evolve.
        let (pid, mid) = cat
            .search_models("claude sonnet", 5)
            .into_iter()
            .next()
            .map(|(p, m)| (p.to_string(), m.id.clone()))
            .expect("expected a claude sonnet model in snapshot");
        let ctx = cat.context_limit(&pid, &mid).unwrap();
        assert!(ctx.unwrap_or(0) >= 8000);
        let cost = cat.cost(&pid, &mid).unwrap();
        assert!(cost.input.unwrap_or(0.0) >= 0.0);
        assert!(cost.output.unwrap_or(0.0) >= 0.0);
        let caps = cat.capabilities(&pid, &mid).unwrap();
        assert!(caps.tool_call, "sonnet-class models support tool calls");
        let price = cat.estimate_cost(&pid, &mid, 1_000_000, 1_000_000).unwrap();
        assert_eq!(
            price,
            cost.input.unwrap_or(0.0) + cost.output.unwrap_or(0.0)
        );
        assert!(cat.base_url(&pid).unwrap().is_some());
        assert!(!cat.models(&pid).unwrap().is_empty());
        assert!(cat.search_models("gpt", 3).len() <= 3);
        assert!(matches!(
            cat.provider("no-such-provider"),
            Err(CatalogError::UnknownProvider(_))
        ));
        assert!(matches!(
            cat.model(&pid, "no-such-model"),
            Err(CatalogError::UnknownModel(_, _))
        ));
    }

    #[test]
    fn catalog_cache_roundtrip_and_staleness() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        assert!(read_cache(&conn).unwrap().is_none());
        assert!(is_stale(0, STALE_AFTER_SECS + 1));
        assert!(!is_stale(1000, 1000 + 10));
        let cat = Catalog::embedded().unwrap();
        let raw = serde_json::to_vec(&cat).unwrap();
        write_cache(&conn, "\"abc\"", 1234, &gzip(&raw).unwrap()).unwrap();
        let snap = read_cache(&conn).unwrap().unwrap();
        assert_eq!(snap.etag, "\"abc\"");
        assert_eq!(snap.fetched_at, 1234);
        let back = Catalog::from_gz(&snap.blob).unwrap();
        assert_eq!(back.model_count(), cat.model_count());
        let loaded = load_cached_or_embedded(&conn).unwrap();
        assert_eq!(loaded.model_count(), cat.model_count());
    }

    #[tokio::test]
    async fn catalog_refresh_uses_etag_and_daily_cache() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let body = serde_json::json!({
            "demo": {
                "id": "demo", "name": "Demo", "env": ["DEMO_API_KEY"],
                "api": "https://demo.example/v1",
                "models": {
                    "m1": {
                        "id": "m1", "name": "M1", "tool_call": true,
                        "limit": {"context": 32000, "output": 4096},
                        "cost": {"input": 1.0, "output": 2.0}
                    }
                }
            }
        });
        let body_str = serde_json::to_string(&body).unwrap();
        let server = wiremock::MockServer::start().await;
        // First fetch: 200 with ETag.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_string(body_str.clone())
                    .insert_header("etag", "\"v1\""),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let url = format!("{}/api.json", server.uri());
        let (cat, out) = refresh_if_stale(&conn, &client, &url, 10_000)
            .await
            .unwrap();
        assert!(out.updated);
        assert_eq!(cat.model_count(), 1);
        assert_eq!(cat.context_limit("demo", "m1").unwrap(), Some(32000));
        assert_eq!(cat.estimate_cost("demo", "m1", 1_000_000, 0).unwrap(), 1.0);
        // Fresh cache: no HTTP request needed.
        let (cat2, out2) = refresh_if_stale(&conn, &client, &url, 10_000 + 60)
            .await
            .unwrap();
        assert!(!out2.updated && out2.from_cache);
        assert_eq!(cat2.model_count(), 1);
    }

    #[tokio::test]
    async fn catalog_refresh_handles_304_not_modified() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let body = serde_json::json!({
            "demo": {
                "id": "demo", "name": "Demo", "env": [],
                "models": {"m1": {"id": "m1", "name": "M1"}}
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        write_cache(&conn, "\"v9\"", 0, &gzip(&raw).unwrap()).unwrap();
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(304))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let url = format!("{}/api.json", server.uri());
        // far future => stale, forces conditional GET which returns 304
        let (cat, out) = refresh_if_stale(&conn, &client, &url, STALE_AFTER_SECS + 5)
            .await
            .unwrap();
        assert!(!out.updated && out.from_cache);
        assert_eq!(cat.model_count(), 1);
        let snap = read_cache(&conn).unwrap().unwrap();
        assert_eq!(snap.etag, "\"v9\"");
        assert_eq!(snap.fetched_at, STALE_AFTER_SECS + 5);
    }
}
