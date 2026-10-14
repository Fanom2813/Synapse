//! Ask a provider which models it serves right now. The bundled catalog is a
//! snapshot and goes stale, so a model picked from it can 404 (the provider
//! retired it, or never offered it under that id).
//!
//! Three wire formats cover the providers: the OpenAI-style `GET {base}/models`
//! (OpenAI, gateways, local servers, most others), Anthropic's, and Gemini's.
//! Keys are only sent to the provider's own address and never logged.

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DiscoveryError {
    #[error("The provider rejected the key.")]
    Auth,
    #[error("The provider does not list its models ({0}).")]
    Unsupported(String),
    #[error("Could not reach the provider: {0}")]
    Network(String),
    #[error("The provider's model list was not understood.")]
    BadReply,
}

/// How a provider lists its models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormat {
    OpenAi,
    Anthropic,
    Gemini,
}

impl ListFormat {
    pub fn for_provider(id: &str) -> Self {
        match id {
            "anthropic" => Self::Anthropic,
            "google" => Self::Gemini,
            _ => Self::OpenAi,
        }
    }
}

/// Where to ask: the provider's own address, or the well-known one for the
/// big three whose catalog entry has none.
pub fn list_url(provider_id: &str, base_url: Option<&str>) -> Option<String> {
    let base = match (base_url, provider_id) {
        (Some(base), _) => base.trim_end_matches('/').to_string(),
        (None, "openai") => "https://api.openai.com/v1".into(),
        (None, "anthropic") => "https://api.anthropic.com/v1".into(),
        (None, "google") => "https://generativelanguage.googleapis.com/v1beta".into(),
        (None, _) => return None,
    };
    Some(match ListFormat::for_provider(provider_id) {
        ListFormat::Anthropic => format!("{base}/models?limit=1000"),
        ListFormat::Gemini => format!("{base}/models?pageSize=1000"),
        ListFormat::OpenAi => format!("{base}/models"),
    })
}

/// Fetch the chat model ids the provider serves.
pub async fn discover(
    client: &reqwest::Client,
    provider_id: &str,
    base_url: Option<&str>,
    key: Option<&str>,
) -> Result<Vec<String>, DiscoveryError> {
    let format = ListFormat::for_provider(provider_id);
    let url = list_url(provider_id, base_url)
        .ok_or_else(|| DiscoveryError::Unsupported("no address".into()))?;
    let mut request = client.get(url).timeout(std::time::Duration::from_secs(20));
    if let Some(key) = key.filter(|k| !k.is_empty()) {
        request = match format {
            ListFormat::OpenAi => request.bearer_auth(key),
            ListFormat::Anthropic => request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
            ListFormat::Gemini => request.header("x-goog-api-key", key),
        };
    }
    let response = request
        .send()
        .await
        .map_err(|e| DiscoveryError::Network(e.without_url().to_string()))?;
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Err(DiscoveryError::Auth);
    }
    if !status.is_success() {
        return Err(DiscoveryError::Unsupported(format!("HTTP {}", status.as_u16())));
    }
    let body: Value = response.json().await.map_err(|_| DiscoveryError::BadReply)?;
    parse(format, &body).ok_or(DiscoveryError::BadReply)
}

/// Model ids from a list reply; `None` when the reply has no list at all.
pub fn parse(format: ListFormat, body: &Value) -> Option<Vec<String>> {
    let mut ids: Vec<String> = match format {
        ListFormat::OpenAi | ListFormat::Anthropic => body
            .get("data")?
            .as_array()?
            .iter()
            .filter_map(|m| m.get("id")?.as_str().map(str::to_string))
            .collect(),
        ListFormat::Gemini => body
            .get("models")?
            .as_array()?
            .iter()
            .filter(|m| {
                m.get("supportedGenerationMethods")
                    .and_then(Value::as_array)
                    .map_or(true, |ways| ways.iter().any(|w| w == "generateContent"))
            })
            .filter_map(|m| {
                let name = m.get("name")?.as_str()?;
                Some(name.strip_prefix("models/").unwrap_or(name).to_string())
            })
            .collect(),
    };
    ids.retain(|id| is_chat_model(id));
    ids.sort();
    ids.dedup();
    Some(ids)
}

/// Lists mix in embedding, speech, image and moderation models that cannot
/// chat; drop the obvious ones.
fn is_chat_model(id: &str) -> bool {
    const NOT_CHAT: [&str; 9] = [
        "embed", "whisper", "tts", "dall-e", "moderation", "transcribe", "rerank", "imagen",
        "text-to-image",
    ];
    let id = id.to_lowercase();
    !NOT_CHAT.iter().any(|word| id.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_style_lists_keep_chat_models_only_and_sorted() {
        let body = json!({"data": [
            {"id": "hf:zai-org/GLM-5.3"}, {"id": "text-embedding-3-small"},
            {"id": "gpt-5"}, {"id": "gpt-5"}, {"id": "whisper-1"}
        ]});
        assert_eq!(
            parse(ListFormat::OpenAi, &body).unwrap(),
            ["gpt-5", "hf:zai-org/GLM-5.3"]
        );
        assert_eq!(parse(ListFormat::OpenAi, &json!({"error": "x"})), None);
    }

    #[test]
    fn gemini_lists_drop_the_models_prefix_and_non_chat_methods() {
        let body = json!({"models": [
            {"name": "models/gemini-3-pro", "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/embedding-001", "supportedGenerationMethods": ["embedContent"]}
        ]});
        assert_eq!(parse(ListFormat::Gemini, &body).unwrap(), ["gemini-3-pro"]);
    }

    #[test]
    fn urls_follow_the_provider_address() {
        assert_eq!(
            list_url("synthetic", Some("https://api.synthetic.new/openai/v1/")).unwrap(),
            "https://api.synthetic.new/openai/v1/models"
        );
        assert_eq!(
            list_url("anthropic", None).unwrap(),
            "https://api.anthropic.com/v1/models?limit=1000"
        );
        assert_eq!(list_url("mystery", None), None);
    }
}
