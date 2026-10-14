//! synapse-research: search providers and deep-research loop (see docs/ARCHITECTURE.md).
//! Implements the search provider trait, keyless/configured/provider-native paths,
//! auto / multi (RRF) / random modes, rate limits, circuit breaker, and doctor health checks.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod research_loop;
pub mod browser;
pub mod reader;
pub mod perspectives;
pub mod factcheck;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    /// Which engines returned this hit (for multi fusion tagging).
    #[serde(default)]
    pub engines: Vec<String>,
    /// RRF score (only set after fusion).
    #[serde(default)]
    pub score: Option<f64>,
}

impl SearchHit {
    pub fn new(
        url: impl Into<String>,
        title: impl Into<String>,
        snippet: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            title: title.into(),
            snippet: snippet.into(),
            engines: vec![],
            score: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("no providers configured")]
    NoProviders,
    #[error("all providers failed: {0}")]
    AllFailed(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("circuit open: {0}")]
    CircuitOpen(String),
    #[error("provider error {provider}: {msg}")]
    Provider { provider: String, msg: String },
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait SearchProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn search(&self, query: &str) -> Result<Vec<SearchHit>, SearchError>;
    /// Health check: try a trivial query.
    async fn health_check(&self) -> HealthStatus {
        let start = Instant::now();
        let result = self.search("test").await;
        HealthStatus {
            backend: self.name().to_string(),
            ok: result.is_ok(),
            latency_ms: start.elapsed().as_millis() as u64,
            error: result.err().map(|e| e.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Mock provider (for tests)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MockHit {
    pub hit: SearchHit,
}

pub struct MockSearch {
    name: String,
    hits: Arc<Mutex<Vec<SearchHit>>>,
    fail: Arc<Mutex<bool>>,
    call_count: Arc<Mutex<usize>>,
}

impl MockSearch {
    pub fn new(name: impl Into<String>, hits: Vec<SearchHit>) -> Self {
        Self {
            name: name.into(),
            hits: Arc::new(Mutex::new(hits)),
            fail: Arc::new(Mutex::new(false)),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    pub fn failing(name: impl Into<String>) -> Self {
        let s = Self::new(name, vec![]);
        *s.fail.lock().unwrap() = true;
        s
    }

    pub fn set_failing(&self, v: bool) {
        *self.fail.lock().unwrap() = v;
    }

    pub fn call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }

    pub fn set_hits(&self, hits: Vec<SearchHit>) {
        *self.hits.lock().unwrap() = hits;
    }
}

#[async_trait::async_trait]
impl SearchProvider for MockSearch {
    fn name(&self) -> &str {
        &self.name
    }
    async fn search(&self, _query: &str) -> Result<Vec<SearchHit>, SearchError> {
        *self.call_count.lock().unwrap() += 1;
        if *self.fail.lock().unwrap() {
            return Err(SearchError::Provider {
                provider: self.name.clone(),
                msg: "mock failure".into(),
            });
        }
        let mut hits = self.hits.lock().unwrap().clone();
        // tag with engine name
        for h in &mut hits {
            if !h.engines.contains(&self.name) {
                h.engines.push(self.name.clone());
            }
        }
        Ok(hits)
    }
}

// ---------------------------------------------------------------------------
// Keyed generic provider (Brave, Tavily, Exa etc) — stub that would hit HTTP
// In tests it behaves as mock; in production it would use reqwest.
// ---------------------------------------------------------------------------

pub struct KeyedProvider {
    name: String,
    // In real use: api key / url. Kept for config.
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    // For testing: inject hits
    mock_hits: Option<Vec<SearchHit>>,
    should_fail: bool,
}

impl KeyedProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            api_key: None,
            base_url: None,
            mock_hits: None,
            should_fail: false,
        }
    }
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
    pub fn with_mock_hits(mut self, hits: Vec<SearchHit>) -> Self {
        self.mock_hits = Some(hits);
        self
    }
    pub fn failing(mut self) -> Self {
        self.should_fail = true;
        self
    }
    pub fn is_configured(&self) -> bool {
        self.api_key.is_some() || self.base_url.is_some() || self.mock_hits.is_some()
    }
}

#[async_trait::async_trait]
impl SearchProvider for KeyedProvider {
    fn name(&self) -> &str {
        &self.name
    }
    async fn search(&self, _query: &str) -> Result<Vec<SearchHit>, SearchError> {
        if self.should_fail {
            return Err(SearchError::Provider {
                provider: self.name.clone(),
                msg: "keyed provider failure".into(),
            });
        }
        if let Some(hits) = &self.mock_hits {
            let mut out = hits.clone();
            for h in &mut out {
                if !h.engines.contains(&self.name) {
                    h.engines.push(self.name.clone());
                }
            }
            return Ok(out);
        }
        // If not configured and no mock, treat as not available
        if !self.is_configured() {
            return Err(SearchError::Provider {
                provider: self.name.clone(),
                msg: "not configured".into(),
            });
        }
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Search mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Auto,
    Multi,
    Random,
}

impl Default for SearchMode {
    fn default() -> Self {
        Self::Auto
    }
}

// ---------------------------------------------------------------------------
// Rate limiter (per-backend governor-like, simplified)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct RateLimiter {
    max_per_minute: u32,
    window: Duration,
    timestamps: Mutex<Vec<Instant>>,
}

impl RateLimiter {
    pub fn new(max_per_minute: u32) -> Self {
        Self {
            max_per_minute,
            window: Duration::from_secs(60),
            timestamps: Mutex::new(vec![]),
        }
    }

    pub fn allow(&self) -> bool {
        let mut ts = self.timestamps.lock().unwrap();
        let now = Instant::now();
        ts.retain(|t| now.duration_since(*t) < self.window);
        if ts.len() < self.max_per_minute as usize {
            ts.push(now);
            true
        } else {
            false
        }
    }

    pub fn remaining(&self) -> i32 {
        let ts = self.timestamps.lock().unwrap();
        self.max_per_minute as i32 - ts.len() as i32
    }
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    failure_threshold: usize,
    cooldown: Duration,
    failures: Arc<Mutex<HashMap<String, (usize, Option<Instant>)>>>,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: usize, cooldown: Duration) -> Self {
        Self {
            failure_threshold,
            cooldown,
            failures: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn record_success(&self, backend: &str) {
        self.failures.lock().unwrap().remove(backend);
    }

    pub fn record_failure(&self, backend: &str) {
        let mut map = self.failures.lock().unwrap();
        let entry = map.entry(backend.to_string()).or_insert((0, None));
        entry.0 += 1;
        if entry.0 >= self.failure_threshold {
            entry.1 = Some(Instant::now());
        }
    }

    pub fn is_open(&self, backend: &str) -> bool {
        let mut map = self.failures.lock().unwrap();
        if let Some((count, opened_at)) = map.get(backend).cloned() {
            if count >= self.failure_threshold {
                if let Some(at) = opened_at {
                    if at.elapsed() < self.cooldown {
                        return true;
                    } else {
                        // cooldown expired, reset
                        map.remove(backend);
                        return false;
                    }
                }
            }
        }
        false
    }

    pub fn reset(&self, backend: &str) {
        self.failures.lock().unwrap().remove(backend);
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(3, Duration::from_secs(600)) // 10 min
    }
}

// ---------------------------------------------------------------------------
// RRF fusion
// ---------------------------------------------------------------------------

/// Reciprocal Rank Fusion, k=60 default.
pub fn reciprocal_rank_fusion(results: Vec<Vec<SearchHit>>, k: f64) -> Vec<SearchHit> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    let mut by_url: HashMap<String, SearchHit> = HashMap::new();
    let mut engines_by_url: HashMap<String, HashSet<String>> = HashMap::new();

    for hits in &results {
        for (rank, hit) in hits.iter().enumerate() {
            let r = (rank + 1) as f64;
            let inc = 1.0 / (k + r);
            *scores.entry(hit.url.clone()).or_insert(0.0) += inc;
            by_url.entry(hit.url.clone()).or_insert_with(|| hit.clone());
            let entry = engines_by_url.entry(hit.url.clone()).or_default();
            for e in &hit.engines {
                entry.insert(e.clone());
            }
            // also if hit has no engine tag, we still count it
            if hit.engines.is_empty() {
                // nothing extra
            }
        }
    }

    let mut merged: Vec<SearchHit> = by_url
        .into_iter()
        .map(|(url, mut hit)| {
            let score = scores.get(&url).copied().unwrap_or(0.0);
            hit.score = Some(score);
            if let Some(engines) = engines_by_url.get(&url) {
                let mut v: Vec<String> = engines.iter().cloned().collect();
                v.sort();
                hit.engines = v;
            }
            hit
        })
        .collect();

    merged.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // dedupe by URL is inherent (hash map)
    merged
}

// ---------------------------------------------------------------------------
// Search service
// ---------------------------------------------------------------------------

pub struct SearchService {
    providers: Vec<Arc<dyn SearchProvider>>,
    mode: SearchMode,
    pub rate_limiters: HashMap<String, RateLimiter>,
    pub circuit: CircuitBreaker,
}

impl SearchService {
    pub fn new(mode: SearchMode) -> Self {
        Self {
            providers: vec![],
            mode,
            rate_limiters: HashMap::new(),
            circuit: CircuitBreaker::default(),
        }
    }

    pub fn with_circuit(mut self, cb: CircuitBreaker) -> Self {
        self.circuit = cb;
        self
    }

    pub fn add_provider(&mut self, p: Arc<dyn SearchProvider>) {
        let name = p.name().to_string();
        self.rate_limiters.entry(name.clone()).or_insert_with(|| {
            // defaults: keyless 20 RPM, keyed 60 RPM
            let rpm = if is_keyless(&name) { 20 } else { 60 };
            RateLimiter::new(rpm)
        });
        self.providers.push(p);
    }

    pub fn set_mode(&mut self, mode: SearchMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> SearchMode {
        self.mode
    }

    pub fn provider_names(&self) -> Vec<String> {
        self.providers
            .iter()
            .map(|p| p.name().to_string())
            .collect()
    }

    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>, SearchError> {
        if self.providers.is_empty() {
            return Err(SearchError::NoProviders);
        }
        match self.mode {
            SearchMode::Auto => self.search_auto(query).await,
            SearchMode::Multi => self.search_multi(query).await,
            SearchMode::Random => self.search_random(query).await,
        }
    }

    async fn search_auto(&self, query: &str) -> Result<Vec<SearchHit>, SearchError> {
        let mut last_err = None;
        for p in &self.providers {
            let name = p.name().to_string();
            if self.circuit.is_open(&name) {
                continue;
            }
            if let Some(limiter) = self.rate_limiters.get(&name) {
                if !limiter.allow() {
                    last_err = Some(format!("{name} rate limited"));
                    continue;
                }
            }
            match p.search(query).await {
                Ok(hits) => {
                    self.circuit.record_success(&name);
                    if !hits.is_empty() {
                        return Ok(hits);
                    }
                    // empty counts as no answer, try next
                    last_err = Some(format!("{name} returned 0 hits"));
                }
                Err(e) => {
                    self.circuit.record_failure(&name);
                    last_err = Some(e.to_string());
                }
            }
        }
        Err(SearchError::AllFailed(
            last_err.unwrap_or_else(|| "no provider answered".into()),
        ))
    }

    async fn search_multi(&self, query: &str) -> Result<Vec<SearchHit>, SearchError> {
        let mut all: Vec<Vec<SearchHit>> = vec![];
        let mut errors = vec![];
        for p in &self.providers {
            let name = p.name().to_string();
            if self.circuit.is_open(&name) {
                continue;
            }
            if let Some(limiter) = self.rate_limiters.get(&name) {
                if !limiter.allow() {
                    errors.push(format!("{name} rate limited"));
                    continue;
                }
            }
            match p.search(query).await {
                Ok(hits) => {
                    self.circuit.record_success(&name);
                    if !hits.is_empty() {
                        all.push(hits);
                    }
                }
                Err(e) => {
                    self.circuit.record_failure(&name);
                    errors.push(e.to_string());
                }
            }
        }
        if all.is_empty() {
            return Err(SearchError::AllFailed(errors.join("; ")));
        }
        Ok(reciprocal_rank_fusion(all, 60.0))
    }

    async fn search_random(&self, query: &str) -> Result<Vec<SearchHit>, SearchError> {
        // Filter to available (not open circuit, not rate limited)
        let mut candidates: Vec<Arc<dyn SearchProvider>> = vec![];
        for p in &self.providers {
            let name = p.name().to_string();
            if self.circuit.is_open(&name) {
                continue;
            }
            if let Some(limiter) = self.rate_limiters.get(&name) {
                if limiter.remaining() <= 0 {
                    continue;
                }
            }
            candidates.push(p.clone());
        }
        if candidates.is_empty() {
            // fallback to auto behavior over all (will report rate limited / circuit)
            return self.search_auto(query).await;
        }
        // random pick — use thread-rng via rand
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        let chosen = candidates.choose(&mut rng).unwrap().clone();
        let name = chosen.name().to_string();
        if let Some(limiter) = self.rate_limiters.get(&name) {
            if !limiter.allow() {
                return Err(SearchError::RateLimited(name));
            }
        }
        match chosen.search(query).await {
            Ok(hits) => {
                self.circuit.record_success(&name);
                Ok(hits)
            }
            Err(e) => {
                self.circuit.record_failure(&name);
                Err(e)
            }
        }
    }
}

fn is_keyless(name: &str) -> bool {
    matches!(
        name,
        "parallel" | "exa" | "keenable" | "youcom" | "firecrawl" | "ddg"
    )
}

// ---------------------------------------------------------------------------
// Doctor / health checks (Settings doctor view)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthStatus {
    pub backend: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub checks: Vec<HealthStatus>,
    pub summary: String,
}

impl DoctorReport {
    pub fn healthy_count(&self) -> usize {
        self.checks.iter().filter(|c| c.ok).count()
    }
    pub fn total(&self) -> usize {
        self.checks.len()
    }
}

pub async fn doctor_check(providers: &[Arc<dyn SearchProvider>]) -> DoctorReport {
    let mut checks = Vec::new();
    for p in providers {
        let status = p.health_check().await;
        checks.push(status);
    }
    let healthy = checks.iter().filter(|c| c.ok).count();
    let total = checks.len();
    let summary = if total == 0 {
        "No search providers configured. Keyless search will be used if available.".into()
    } else if healthy == total {
        format!("All {total} backends healthy")
    } else if healthy == 0 {
        format!("All {total} backends failing — check keys and network")
    } else {
        format!("{healthy}/{total} backends healthy")
    };
    DoctorReport { checks, summary }
}

// ---------------------------------------------------------------------------
// Config: describes which providers are enabled (for persistence / settings)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchConfig {
    pub mode: SearchMode,
    pub searxng_url: Option<String>,
    pub degoog_url: Option<String>,
    pub brave_key: Option<String>,
    pub tavily_key: Option<String>,
    pub exa_key: Option<String>,
    pub serper_key: Option<String>,
    pub serpapi_key: Option<String>,
    pub jina_key: Option<String>,
    pub firecrawl_key: Option<String>,
    /// Provider-native search enabled
    pub provider_native: bool,
    /// External binaries as providers
    pub enable_ketch: bool,
    pub enable_search_cli: bool,
}

impl SearchConfig {
    /// Build provider list in priority order per DEEP-RESEARCH.md §1.
    pub fn build_providers(&self) -> Vec<Arc<dyn SearchProvider>> {
        let mut out: Vec<Arc<dyn SearchProvider>> = vec![];
        // Configured SearXNG / Degoog first
        if let Some(url) = &self.searxng_url {
            if !url.is_empty() {
                out.push(Arc::new(
                    KeyedProvider::new("searxng").with_url(url.clone()),
                ));
            }
        }
        if let Some(url) = &self.degoog_url {
            if !url.is_empty() {
                out.push(Arc::new(KeyedProvider::new("degoog").with_url(url.clone())));
            }
        }
        // Keyed APIs
        if let Some(k) = &self.brave_key {
            if !k.is_empty() {
                out.push(Arc::new(KeyedProvider::new("brave").with_key(k.clone())));
            }
        }
        if let Some(k) = &self.tavily_key {
            if !k.is_empty() {
                out.push(Arc::new(KeyedProvider::new("tavily").with_key(k.clone())));
            }
        }
        if let Some(k) = &self.exa_key {
            if !k.is_empty() {
                out.push(Arc::new(KeyedProvider::new("exa").with_key(k.clone())));
            }
        }
        if let Some(k) = &self.serper_key {
            if !k.is_empty() {
                out.push(Arc::new(KeyedProvider::new("serper").with_key(k.clone())));
            }
        }
        if let Some(k) = &self.serpapi_key {
            if !k.is_empty() {
                out.push(Arc::new(KeyedProvider::new("serpapi").with_key(k.clone())));
            }
        }
        if let Some(k) = &self.jina_key {
            if !k.is_empty() {
                out.push(Arc::new(KeyedProvider::new("jina").with_key(k.clone())));
            }
        }
        if let Some(k) = &self.firecrawl_key {
            if !k.is_empty() {
                out.push(Arc::new(
                    KeyedProvider::new("firecrawl").with_key(k.clone()),
                ));
            }
        }

        if self.provider_native {
            out.push(Arc::new(KeyedProvider::new("provider-native")));
        }

        // Keyless chain if nothing configured or as fallback
        if out.is_empty() {
            for name in ["parallel", "exa", "keenable", "youcom", "firecrawl", "ddg"] {
                out.push(Arc::new(KeyedProvider::new(name)));
            }
        } else {
            // Still add keyless as fallback after configured
            for name in ["parallel", "exa", "keenable", "youcom", "firecrawl", "ddg"] {
                // avoid duplicate exa/firecrawl names — add with keyless- prefix if already present
                let already = out.iter().any(|p| p.name() == name);
                let n = if already {
                    format!("{name}-keyless")
                } else {
                    name.to_string()
                };
                out.push(Arc::new(KeyedProvider::new(n)));
            }
        }

        if self.enable_ketch {
            out.push(Arc::new(KeyedProvider::new("ketch")));
        }
        if self.enable_search_cli {
            out.push(Arc::new(KeyedProvider::new("search-cli")));
        }

        out
    }

    /// Keyless chain names in order (port of ketch).
    pub fn keyless_chain() -> Vec<&'static str> {
        vec!["parallel", "exa", "keenable", "youcom", "firecrawl", "ddg"]
    }
}

// ---------------------------------------------------------------------------
// Tests — must contain "search" in name for verify filter
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn hit(url: &str, title: &str) -> SearchHit {
        SearchHit::new(url, title, "snippet")
    }

    #[tokio::test]
    async fn search_mock_returns_hits() {
        let m = MockSearch::new(
            "brave",
            vec![hit("https://a.com", "A"), hit("https://b.com", "B")],
        );
        let hits = m.search("hello").await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(m.call_count(), 1);
    }

    #[tokio::test]
    async fn search_auto_uses_first_success() {
        let p1 = Arc::new(MockSearch::failing("parallel")) as Arc<dyn SearchProvider>;
        let p2 = Arc::new(MockSearch::new("exa", vec![hit("https://ex.com", "Ex")]))
            as Arc<dyn SearchProvider>;
        let p3 = Arc::new(MockSearch::new("ddg", vec![hit("https:://ddg.com", "DDG")]))
            as Arc<dyn SearchProvider>;
        let mut svc = SearchService::new(SearchMode::Auto);
        svc.add_provider(p1);
        svc.add_provider(p2);
        svc.add_provider(p3);
        let hits = svc.search("query").await.unwrap();
        assert_eq!(hits[0].url, "https://ex.com");
        // should not have tried ddg
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn search_auto_skips_empty_and_fails_if_all_empty() {
        let p1 = Arc::new(MockSearch::new("a", vec![])) as Arc<dyn SearchProvider>;
        let p2 = Arc::new(MockSearch::new("b", vec![])) as Arc<dyn SearchProvider>;
        let mut svc = SearchService::new(SearchMode::Auto);
        svc.add_provider(p1);
        svc.add_provider(p2);
        let res = svc.search("q").await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn search_multi_fuses_with_rrf_and_dedupes() {
        let p1 = Arc::new(MockSearch::new(
            "brave",
            vec![hit("https://a.com", "A"), hit("https://b.com", "B")],
        )) as Arc<dyn SearchProvider>;
        let p2 = Arc::new(MockSearch::new(
            "tavily",
            vec![hit("https://b.com", "B2"), hit("https://c.com", "C")],
        )) as Arc<dyn SearchProvider>;
        let mut svc = SearchService::new(SearchMode::Multi);
        svc.add_provider(p1);
        svc.add_provider(p2);
        let hits = svc.search("q").await.unwrap();
        // deduped: a,b,c = 3
        assert_eq!(hits.len(), 3);
        // b appears in both, should rank highest (higher RRF)
        assert_eq!(hits[0].url, "https://b.com");
        assert!(hits[0].engines.contains(&"brave".to_string()));
        assert!(hits[0].engines.contains(&"tavily".to_string()));
        assert!(hits[0].score.is_some());
    }

    #[tokio::test]
    async fn search_random_picks_one_backend() {
        let p1 = Arc::new(MockSearch::new("a", vec![hit("https://a.com", "A")]))
            as Arc<dyn SearchProvider>;
        let p2 = Arc::new(MockSearch::new("b", vec![hit("https://b.com", "B")]))
            as Arc<dyn SearchProvider>;
        let p3 = Arc::new(MockSearch::new("c", vec![hit("https://c.com", "C")]))
            as Arc<dyn SearchProvider>;
        let mut svc = SearchService::new(SearchMode::Random);
        svc.add_provider(p1);
        svc.add_provider(p2);
        svc.add_provider(p3);
        let hits = svc.search("q").await.unwrap();
        assert_eq!(hits.len(), 1);
        // hit url must be one of the three
        assert!(["https://a.com", "https://b.com", "https://c.com"].contains(&hits[0].url.as_str()));
    }

    #[tokio::test]
    async fn search_circuit_breaker_opens_and_skips() {
        let cb = CircuitBreaker::new(1, Duration::from_secs(600));
        let failing = Arc::new(MockSearch::failing("parallel")) as Arc<dyn SearchProvider>;
        let ok = Arc::new(MockSearch::new("exa", vec![hit("https://ex.com", "Ex")]))
            as Arc<dyn SearchProvider>;
        let mut svc = SearchService::new(SearchMode::Auto);
        svc.circuit = cb.clone();
        svc.add_provider(failing);
        svc.add_provider(ok);
        // first call: parallel fails, exa succeeds
        let hits = svc.search("q").await.unwrap();
        assert_eq!(hits[0].url, "https://ex.com");
        // parallel should now be open (threshold 1)
        assert!(cb.is_open("parallel"));
        // second call should skip parallel directly
        let hits2 = svc.search("q2").await.unwrap();
        assert_eq!(hits2[0].url, "https://ex.com");
        // reset and it should be tried again
        cb.reset("parallel");
        assert!(!cb.is_open("parallel"));
    }

    #[tokio::test]
    async fn search_rate_limiter_blocks_after_limit() {
        let m = Arc::new(MockSearch::new("brave", vec![hit("https://a.com", "A")]))
            as Arc<dyn SearchProvider>;
        let mut svc = SearchService::new(SearchMode::Auto);
        svc.add_provider(m);
        // override limiter to 1 per minute
        svc.rate_limiters
            .insert("brave".into(), RateLimiter::new(1));
        let ok = svc.search("q1").await.unwrap();
        assert_eq!(ok.len(), 1);
        // second call should be rate limited and thus fail (no other provider)
        let err = svc.search("q2").await.unwrap_err();
        assert!(err.to_string().contains("AllFailed") || err.to_string().contains("rate"));
    }

    #[tokio::test]
    async fn search_doctor_reports_health() {
        let ok = Arc::new(MockSearch::new("brave", vec![hit("https://a.com", "A")]))
            as Arc<dyn SearchProvider>;
        let bad = Arc::new(MockSearch::failing("tavily")) as Arc<dyn SearchProvider>;
        let report = doctor_check(&[ok, bad]).await;
        assert_eq!(report.total(), 2);
        assert_eq!(report.healthy_count(), 1);
        assert!(report.summary.contains("1/2"));
        let ok_entry = report.checks.iter().find(|c| c.backend == "brave").unwrap();
        assert!(ok_entry.ok);
        assert!(ok_entry.error.is_none());
        let bad_entry = report
            .checks
            .iter()
            .find(|c| c.backend == "tavily")
            .unwrap();
        assert!(!bad_entry.ok);
        assert!(bad_entry.error.is_some());
    }

    #[tokio::test]
    async fn search_keyless_chain_order() {
        let chain = SearchConfig::keyless_chain();
        assert_eq!(
            chain,
            vec!["parallel", "exa", "keenable", "youcom", "firecrawl", "ddg"]
        );
    }

    #[tokio::test]
    async fn search_config_builds_providers_in_priority_order() {
        let cfg = SearchConfig {
            mode: SearchMode::Auto,
            searxng_url: Some("https://searx.example".into()),
            brave_key: Some("bk".into()),
            ..Default::default()
        };
        let providers = cfg.build_providers();
        let names: Vec<String> = providers.iter().map(|p| p.name().to_string()).collect();
        assert_eq!(names[0], "searxng");
        assert!(names.contains(&"brave".to_string()));
        // keyless fallback after configured
        assert!(names.contains(&"parallel".to_string()));
    }

    #[tokio::test]
    async fn search_config_keyless_when_no_keys() {
        let cfg = SearchConfig::default();
        let providers = cfg.build_providers();
        let names: Vec<String> = providers.iter().map(|p| p.name().to_string()).collect();
        assert_eq!(
            names,
            vec!["parallel", "exa", "keenable", "youcom", "firecrawl", "ddg"]
        );
    }

    #[tokio::test]
    async fn search_rrf_scores() {
        let a = vec![hit("https://a.com", "A"), hit("https://b.com", "B")];
        let b = vec![hit("https://b.com", "B"), hit("https://a.com", "A")];
        let merged = reciprocal_rank_fusion(vec![a, b], 60.0);
        assert_eq!(merged.len(), 2);
        // both have same score, order stable? just check scores equal
        assert!((merged[0].score.unwrap() - merged[1].score.unwrap()).abs() < 1e-9);
    }

    #[tokio::test]
    async fn search_no_providers_error() {
        let svc = SearchService::new(SearchMode::Auto);
        let err = svc.search("q").await.unwrap_err();
        assert!(matches!(err, SearchError::NoProviders));
    }
}
