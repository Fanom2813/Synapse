//! Local model detection: Ollama (:11434) and LM Studio (:1234).
//!
//! Both are probed on localhost and their models listed dynamically (they
//! are not in the static models.dev catalog). `test_connection` backs the
//! Settings → Providers "Test" button for local servers.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const OLLAMA_DEFAULT_URL: &str = "http://127.0.0.1:11434";
pub const LMSTUDIO_DEFAULT_URL: &str = "http://127.0.0.1:1234";

#[derive(Debug, Error)]
pub enum LocalError {
    #[error("unreachable ({0})")]
    Unreachable(String),
    #[error("bad response ({0})")]
    BadResponse(String),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
}

pub type LocalResult<T> = std::result::Result<T, LocalError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalServer {
    Ollama,
    LmStudio,
}

impl LocalServer {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lm-studio",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
        }
    }

    pub fn default_url(&self) -> &'static str {
        match self {
            Self::Ollama => OLLAMA_DEFAULT_URL,
            Self::LmStudio => LMSTUDIO_DEFAULT_URL,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalModel {
    pub server: LocalServer,
    /// `base_url` of the server this model was listed from (own host only).
    pub base_url: String,
    pub id: String,
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

/// `GET {base}/api/tags` (Ollama).
pub async fn list_ollama_models(
    client: &reqwest::Client,
    base_url: &str,
) -> LocalResult<Vec<LocalModel>> {
    #[derive(Deserialize)]
    struct Tags {
        #[serde(default)]
        models: Vec<Tag>,
    }
    #[derive(Deserialize)]
    struct Tag {
        #[serde(default)]
        name: String,
        #[serde(default)]
        size: Option<u64>,
    }
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_connect() || e.is_timeout() {
            LocalError::Unreachable(trim_err(&e.to_string()))
        } else {
            LocalError::Http(e)
        }
    })?;
    if !resp.status().is_success() {
        return Err(LocalError::BadResponse(format!("status {}", resp.status())));
    }
    let tags: Tags = resp
        .json()
        .await
        .map_err(|e| LocalError::BadResponse(trim_err(&e.to_string())))?;
    Ok(tags
        .models
        .into_iter()
        .filter(|m| !m.name.is_empty())
        .map(|m| LocalModel {
            server: LocalServer::Ollama,
            base_url: base_url.to_string(),
            id: m.name,
            size_bytes: m.size,
        })
        .collect())
}

/// `GET {base}/v1/models` (LM Studio, OpenAI-compatible).
pub async fn list_lmstudio_models(
    client: &reqwest::Client,
    base_url: &str,
) -> LocalResult<Vec<LocalModel>> {
    #[derive(Deserialize)]
    struct Models {
        #[serde(default)]
        data: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        #[serde(default)]
        id: String,
    }
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_connect() || e.is_timeout() {
            LocalError::Unreachable(trim_err(&e.to_string()))
        } else {
            LocalError::Http(e)
        }
    })?;
    if !resp.status().is_success() {
        return Err(LocalError::BadResponse(format!("status {}", resp.status())));
    }
    let models: Models = resp
        .json()
        .await
        .map_err(|e| LocalError::BadResponse(trim_err(&e.to_string())))?;
    Ok(models
        .data
        .into_iter()
        .filter(|m| !m.id.is_empty())
        .map(|m| LocalModel {
            server: LocalServer::LmStudio,
            base_url: base_url.to_string(),
            id: m.id,
            size_bytes: None,
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedServer {
    pub server: LocalServer,
    pub base_url: String,
    pub models: Vec<LocalModel>,
}

/// Probe both defaults; unreachable servers are skipped (no error).
pub async fn detect_local_servers(client: &reqwest::Client) -> Vec<DetectedServer> {
    detect_local_servers_at(client, OLLAMA_DEFAULT_URL, LMSTUDIO_DEFAULT_URL).await
}

/// Same as [`detect_local_servers`] but against override URLs (tests).
pub async fn detect_local_servers_at(
    client: &reqwest::Client,
    ollama_url: &str,
    lmstudio_url: &str,
) -> Vec<DetectedServer> {
    let mut out = Vec::new();
    if let Ok(models) = list_ollama_models(client, ollama_url).await {
        out.push(DetectedServer {
            server: LocalServer::Ollama,
            base_url: ollama_url.to_string(),
            models,
        });
    }
    if let Ok(models) = list_lmstudio_models(client, lmstudio_url).await {
        out.push(DetectedServer {
            server: LocalServer::LmStudio,
            base_url: lmstudio_url.to_string(),
            models,
        });
    }
    out
}

/// Result of the Settings "Test" button for a local server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionStatus {
    Ok { models: usize },
    Failed { reason: String },
}

impl ConnectionStatus {
    pub fn ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }

    pub fn user_message(&self, label: &str) -> String {
        match self {
            Self::Ok { models } => format!("{label} is running ({models} model(s) found)."),
            Self::Failed { reason } => format!("Could not reach {label}: {reason}"),
        }
    }
}

/// `test_connection`: list the server's models; `Ok` when reachable.
pub async fn test_connection(
    client: &reqwest::Client,
    server: LocalServer,
    base_url: &str,
) -> ConnectionStatus {
    let res = match server {
        LocalServer::Ollama => list_ollama_models(client, base_url).await.map(|m| m.len()),
        LocalServer::LmStudio => list_lmstudio_models(client, base_url)
            .await
            .map(|m| m.len()),
    };
    match res {
        Ok(models) => ConnectionStatus::Ok { models },
        Err(e) => ConnectionStatus::Failed {
            reason: e.to_string(),
        },
    }
}

fn trim_err(msg: &str) -> String {
    const MAX: usize = 160;
    let mut s: String = msg.chars().take(MAX).collect();
    if msg.len() > MAX {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod keys_local_tests {
    use super::*;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[tokio::test]
    async fn keys_local_ollama_lists_models() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "models": [
                {"name": "llama3.1:8b", "size": 5000000000u64},
                {"name": "qwen2.5:7b", "size": 4000000000u64},
            ]
        });
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let models = list_ollama_models(&client, &server.uri()).await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "llama3.1:8b");
        assert_eq!(models[0].server, LocalServer::Ollama);
        assert_eq!(models[0].size_bytes, Some(5000000000));
        assert_eq!(models[0].base_url, server.uri());
    }

    #[tokio::test]
    async fn keys_local_lmstudio_lists_models() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "object": "list",
            "data": [{"id": "local-model-1"}, {"id": "local-model-2"}]
        });
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let models = list_lmstudio_models(&client, &server.uri()).await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[1].id, "local-model-2");
        assert_eq!(models[1].server, LocalServer::LmStudio);
    }

    #[tokio::test]
    async fn keys_local_detect_skips_unreachable() {
        let ollama = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [{"name": "m1"}]
            })))
            .mount(&ollama)
            .await;
        let client = reqwest::Client::new();
        // LM Studio URL points at a closed port: skipped, not an error.
        let found = detect_local_servers_at(&client, &ollama.uri(), "http://127.0.0.1:9").await;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].server, LocalServer::Ollama);
        assert_eq!(found[0].models.len(), 1);
        let none =
            detect_local_servers_at(&client, "http://127.0.0.1:9", "http://127.0.0.1:9").await;
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn keys_local_test_connection_ok_and_failed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "a"}, {"id": "b"}, {"id": "c"}]
            })))
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let ok = test_connection(&client, LocalServer::LmStudio, &server.uri()).await;
        assert_eq!(ok, ConnectionStatus::Ok { models: 3 });
        assert!(ok.ok());
        assert!(ok.user_message("LM Studio").contains("3 model(s)"));

        let down = test_connection(&client, LocalServer::Ollama, "http://127.0.0.1:9").await;
        assert!(!down.ok());
        assert!(matches!(down, ConnectionStatus::Failed { .. }));
        assert!(down.user_message("Ollama").contains("Ollama"));
    }
}
