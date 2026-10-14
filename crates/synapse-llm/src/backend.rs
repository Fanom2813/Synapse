//! `ModelBackend` over genai: one turn (messages + tools) -> stream of [`AgentEvent`].
//!
//! Covers streaming text, tool calls, usage, mapped [`LlmError`]s, custom
//! OpenAI-compatible base URLs, plus the scripted [`MockLlm`] used by tests.

use std::collections::VecDeque;
use std::sync::Mutex;

use futures::StreamExt as _;
use serde::{Deserialize, Serialize};

// -- request / event types -------------------------------------------------

/// Role of a turn message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// One message in a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnMessage {
    pub role: Role,
    pub text: String,
}

impl TurnMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            text: text.into(),
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: text.into(),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            text: text.into(),
        }
    }

    /// A tool result fed back to the model (`tool_call_id` + JSON/string result).
    pub fn tool_result(call_id: impl Into<String>, result: impl Into<String>) -> ToolResultMsg {
        ToolResultMsg {
            call_id: call_id.into(),
            result: result.into(),
        }
    }
}

/// Tool result message (kept separate so [`TurnRequest`] stays small).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultMsg {
    pub call_id: String,
    pub result: String,
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub schema: Option<serde_json::Value>,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            schema: None,
        }
    }

    pub fn with_description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }

    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.schema = Some(schema);
        self
    }
}

/// A tool call requested by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

/// Token usage for a turn (provider-reported when available).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

impl Usage {
    pub fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
        }
    }
}

impl From<&genai::chat::Usage> for Usage {
    fn from(u: &genai::chat::Usage) -> Self {
        let input = u.prompt_tokens.unwrap_or(0).max(0) as u64;
        let output = u.completion_tokens.unwrap_or(0).max(0) as u64;
        let total = u.total_tokens.unwrap_or(0).max(0) as u64;
        Self {
            input_tokens: input,
            output_tokens: output,
            total_tokens: if total > 0 { total } else { input + output },
        }
    }
}

/// Stream events for one model turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentEvent {
    /// A slice of streamed assistant text.
    TextDelta(String),
    /// The model requested a tool call (emitted with full args).
    ToolCall(ToolCallRequest),
    /// Token usage for the turn (emitted when the provider reports it).
    Usage(Usage),
    /// Turn finished: full text, all tool calls, usage.
    Done {
        text: String,
        tool_calls: Vec<ToolCallRequest>,
        usage: Usage,
    },
}

/// One model turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TurnRequest {
    /// genai model id, e.g. `"gpt-4o-mini"`, `"claude-sonnet-4-5"`, `"llama3.1"`.
    pub model: String,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub messages: Vec<TurnMessage>,
    #[serde(default)]
    pub tool_results: Vec<ToolResultMsg>,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Custom OpenAI-compatible base URL (e.g. LM Studio, llama.cpp, proxy).
    #[serde(default)]
    pub base_url: Option<String>,
    /// API key sent only to the provider's own host (never logged).
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
}

impl TurnRequest {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Self::default()
        }
    }

    pub fn with_system(mut self, s: impl Into<String>) -> Self {
        self.system = Some(s.into());
        self
    }

    pub fn with_message(mut self, m: TurnMessage) -> Self {
        self.messages.push(m);
        self
    }

    pub fn with_tool(mut self, t: ToolSpec) -> Self {
        self.tools.push(t);
        self
    }
}

/// Collected result of a turn (what [`AgentEvent::Done`] carries).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TurnOutput {
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRequest>,
    #[serde(default)]
    pub usage: Usage,
}

// -- errors ----------------------------------------------------------------

/// Mapped, user-actionable LLM error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum LlmError {
    #[error("missing API key: {0}")]
    Auth(String),
    #[error("rate limited, retry later: {0}")]
    RateLimited(String),
    #[error("model not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("stream error: {0}")]
    Stream(String),
}

impl LlmError {
    /// Whether retrying the same turn may succeed.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited(_) | Self::Network(_) | Self::Provider(_)
        )
    }

    /// What to show the user (no key material; see ARCHITECTURE.md §6).
    pub fn user_message(&self) -> String {
        match self {
            Self::Auth(_) => "Missing or invalid API key. Add it in Settings → Providers.".into(),
            Self::RateLimited(_) => {
                "The provider is rate-limiting us. Wait a moment and retry.".into()
            }
            Self::NotFound(m) => {
                format!("Model not found: {m}. Pick another model in Settings → Providers.")
            }
            Self::BadRequest(m) => format!("The request was rejected: {m}"),
            Self::Provider(m) => format!("The provider failed: {m}"),
            Self::Network(m) => format!("Network error talking to the provider: {m}"),
            Self::Stream(m) => format!("The stream broke off: {m}"),
        }
    }

    pub fn from_genai(err: &genai::Error) -> Self {
        if matches!(
            err,
            genai::Error::RequiresApiKey { .. }
                | genai::Error::NoAuthResolver { .. }
                | genai::Error::NoAuthData { .. }
                | genai::Error::StreamParse { .. }
                | genai::Error::StreamEventError { .. }
                | genai::Error::WebStream { .. }
                | genai::Error::ReqwestEventSource(_)
        ) {
            return Self::from_genai_variant(err);
        }
        Self::classify(&err.to_string())
    }

    fn from_genai_variant(err: &genai::Error) -> Self {
        let msg = short(&err.to_string());
        match err {
            genai::Error::RequiresApiKey { .. }
            | genai::Error::NoAuthResolver { .. }
            | genai::Error::NoAuthData { .. } => Self::Auth(msg),
            _ => Self::Stream(msg),
        }
    }

    /// Classify a provider/HTTP error message (status codes embedded in text).
    pub fn classify(msg: &str) -> Self {
        if msg.contains("401") || msg.contains("ApiKeyEnvNotFound") || msg.contains("API key") {
            return Self::Auth(short(msg));
        }
        if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
            return Self::RateLimited(short(msg));
        }
        if msg.contains("404") || msg.to_lowercase().contains("not found") {
            return Self::NotFound(short(msg));
        }
        if msg.contains("400") || msg.contains("422") {
            return Self::BadRequest(short(msg));
        }
        Self::Provider(short(msg))
    }
}

fn short(msg: &str) -> String {
    const MAX: usize = 300;
    let mut s: String = msg.chars().take(MAX).collect();
    if msg.len() > MAX {
        s.push('…');
    }
    s
}

// -- backend trait ---------------------------------------------------------

/// One turn: messages + tools -> stream of [`AgentEvent`] (ARCHITECTURE.md §2).
pub trait ModelBackend: Send + Sync {
    /// Run a turn and collect the result.
    fn turn(
        &self,
        req: TurnRequest,
    ) -> impl std::future::Future<Output = Result<TurnOutput, LlmError>> + Send;

    /// Run a turn and return every stream event (text deltas + tool calls + usage + done).
    fn turn_stream(
        &self,
        req: TurnRequest,
    ) -> impl std::future::Future<Output = Result<Vec<AgentEvent>, LlmError>> + Send;
}

// -- genai backend ---------------------------------------------------------

/// API-model backend over genai (STACK.md: custom OpenAI-compatible endpoints,
///
/// Ollama, streaming tool calls).
pub struct GenaiBackend {
    client: genai::Client,
    default_model: String,
}

impl GenaiBackend {
    /// `api_key` is wired through genai's `AuthResolver` so it is only sent to
    /// the provider's own host. `base_url` overrides the endpoint (custom
    /// OpenAI-compatible server) and forces the OpenAI adapter.
    pub fn new(
        default_model: impl Into<String>,
        api_key: Option<String>,
        base_url: Option<String>,
    ) -> Self {
        let mut builder = genai::Client::builder();
        if let Some(key) = api_key {
            builder = builder.with_auth_resolver_fn(
                move |_model: genai::ModelIden| -> Result<Option<genai::resolver::AuthData>, genai::resolver::Error> {
                    Ok(Some(genai::resolver::AuthData::from_single(key.clone())))
                },
            );
        }
        if let Some(url) = base_url {
            builder = builder.with_service_target_resolver_fn(
                move |target: genai::ServiceTarget| -> Result<genai::ServiceTarget, genai::resolver::Error> {
                    let genai::ServiceTarget { model, .. } = target;
                    let model = genai::ModelIden::new(genai::adapter::AdapterKind::OpenAI, model.model_name);
                    Ok(genai::ServiceTarget {
                        endpoint: genai::resolver::Endpoint::from_owned(url.clone()),
                        auth: target.auth,
                        model,
                    })
                },
            );
        }
        Self {
            client: builder.build(),
            default_model: default_model.into(),
        }
    }

    pub fn with_client(default_model: impl Into<String>, client: genai::Client) -> Self {
        Self {
            client: client.clone(),
            default_model: default_model.into(),
        }
    }

    fn model_for(&self, req: &TurnRequest) -> String {
        if req.model.is_empty() {
            self.default_model.clone()
        } else {
            req.model.clone()
        }
    }
}

fn to_chat_request(req: &TurnRequest) -> genai::chat::ChatRequest {
    use genai::chat::{ChatMessage, ChatRequest, MessageContent, Tool, ToolResponse};
    let mut chat = match req.system.clone() {
        Some(s) => ChatRequest::from_system(s),
        None => ChatRequest::default(),
    };
    let mut msgs: Vec<ChatMessage> = Vec::new();
    for m in &req.messages {
        let content = MessageContent::from_text(m.text.clone());
        msgs.push(match m.role {
            Role::System => ChatMessage::system(content),
            Role::User => ChatMessage::user(content),
            Role::Assistant => ChatMessage::assistant(content),
            Role::Tool => ChatMessage::user(content),
        });
    }
    for r in &req.tool_results {
        msgs.push(ChatMessage::from(ToolResponse::new(
            r.call_id.clone(),
            r.result.clone(),
        )));
    }
    chat = chat.append_messages(msgs);
    if !req.tools.is_empty() {
        chat = chat.with_tools(
            req.tools
                .iter()
                .map(|t| {
                    let mut tool = Tool::new(t.name.clone());
                    if let Some(d) = &t.description {
                        tool = tool.with_description(d.clone());
                    }
                    if let Some(s) = &t.schema {
                        tool = tool.with_schema(s.clone());
                    }
                    tool
                })
                .collect(),
        );
    }
    chat
}

fn to_chat_options(req: &TurnRequest) -> genai::chat::ChatOptions {
    let mut opts = genai::chat::ChatOptions::default()
        .with_capture_usage(true)
        .with_capture_content(true);
    if let Some(t) = req.temperature {
        opts = opts.with_temperature(t);
    }
    if let Some(m) = req.max_tokens {
        opts = opts.with_max_tokens(m);
    }
    opts
}

fn output_from_response(res: &genai::chat::ChatResponse) -> TurnOutput {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    if let Some(content) = &res.content {
        match content {
            genai::chat::MessageContent::Text(t) => text = t.clone(),
            genai::chat::MessageContent::ToolCalls(calls) => {
                tool_calls = calls.iter().map(into_tool_call).collect();
            }
            genai::chat::MessageContent::Parts(parts) => {
                text = parts
                    .iter()
                    .filter_map(|p| match p {
                        genai::chat::ContentPart::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
            }
            genai::chat::MessageContent::ToolResponses(_) => {}
        }
    }
    TurnOutput {
        text,
        tool_calls,
        usage: Usage::from(&res.usage),
    }
}

fn into_tool_call(c: &genai::chat::ToolCall) -> ToolCallRequest {
    ToolCallRequest {
        id: c.call_id.clone(),
        name: c.fn_name.clone(),
        args: c.fn_arguments.clone(),
    }
}

fn events_from_stream(
    chunks: Vec<genai::chat::ChatStreamEvent>,
    end: Option<genai::chat::StreamEnd>,
) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    for chunk in chunks {
        if let genai::chat::ChatStreamEvent::Chunk(c) = chunk {
            if !c.content.is_empty() {
                events.push(AgentEvent::TextDelta(c.content));
            }
        }
    }
    let (text, tool_calls) = match end.as_ref().and_then(|e| e.captured_content.clone()) {
        Some(genai::chat::MessageContent::Text(t)) => (t, Vec::new()),
        Some(genai::chat::MessageContent::ToolCalls(calls)) => {
            (String::new(), calls.iter().map(into_tool_call).collect())
        }
        Some(genai::chat::MessageContent::Parts(parts)) => (
            parts
                .iter()
                .filter_map(|p| match p {
                    genai::chat::ContentPart::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
            Vec::new(),
        ),
        _ => {
            // Fall back to concatenated deltas when the provider (or an old
            // adapter) captures no content.
            let t = events
                .iter()
                .filter_map(|e| match e {
                    AgentEvent::TextDelta(d) => Some(d.as_str()),
                    _ => None,
                })
                .collect::<String>();
            (t, Vec::new())
        }
    };
    let usage = end
        .as_ref()
        .and_then(|e| e.captured_usage.as_ref())
        .map(Usage::from)
        .unwrap_or_default();
    for tc in &tool_calls {
        events.push(AgentEvent::ToolCall(tc.clone()));
    }
    if usage != Usage::default() {
        events.push(AgentEvent::Usage(usage));
    }
    events.push(AgentEvent::Done {
        text,
        tool_calls,
        usage,
    });
    events
}

impl ModelBackend for GenaiBackend {
    async fn turn(&self, req: TurnRequest) -> Result<TurnOutput, LlmError> {
        let model = self.model_for(&req);
        let res = self
            .client
            .exec_chat(&model, to_chat_request(&req), Some(&to_chat_options(&req)))
            .await;
        match res {
            Ok(r) => Ok(output_from_response(&r)),
            Err(e) => Err(LlmError::from_genai(&e)),
        }
    }

    async fn turn_stream(&self, req: TurnRequest) -> Result<Vec<AgentEvent>, LlmError> {
        let model = self.model_for(&req);
        let res = self
            .client
            .exec_chat_stream(&model, to_chat_request(&req), Some(&to_chat_options(&req)))
            .await
            .map_err(|e| LlmError::from_genai(&e))?;
        let mut stream = res.stream;
        let mut chunks = Vec::new();
        let mut end: Option<genai::chat::StreamEnd> = None;
        while let Some(ev) = stream.next().await {
            match ev {
                Ok(
                    genai::chat::ChatStreamEvent::Chunk(_) | genai::chat::ChatStreamEvent::Start,
                ) => {
                    if let Ok(genai::chat::ChatStreamEvent::Chunk(c)) = ev {
                        chunks.push(genai::chat::ChatStreamEvent::Chunk(c));
                    }
                }
                Ok(genai::chat::ChatStreamEvent::ReasoningChunk(_)) => {}
                Ok(genai::chat::ChatStreamEvent::End(e)) => {
                    end = Some(e);
                    break;
                }
                Err(e) => return Err(LlmError::from_genai(&e)),
            }
        }
        Ok(events_from_stream(chunks, end))
    }
}

// -- scripted mock (tests) -------------------------------------------------

/// Scripted backend used by all offline tests (ARCHITECTURE.md §2, MILESTONES.md).
///
/// Each [`TurnRequest`] pops the next scripted [`TurnOutput`]; when the script
/// runs out the last output repeats. `turn_stream` splits the text into word
/// chunks so streaming UI code can be tested without a network.
pub struct MockLlm {
    script: Mutex<VecDeque<Result<TurnOutput, LlmError>>>,
    last: Mutex<Option<Result<TurnOutput, LlmError>>>,
    seen: Mutex<Vec<TurnRequest>>,
}

impl Default for MockLlm {
    fn default() -> Self {
        Self::with_text("mock reply")
    }
}

impl MockLlm {
    pub fn with_text(text: impl Into<String>) -> Self {
        Self::with_outputs(vec![TurnOutput {
            text: text.into(),
            tool_calls: Vec::new(),
            usage: Usage::new(10, 5),
        }])
    }

    pub fn with_outputs(outputs: Vec<TurnOutput>) -> Self {
        Self {
            script: Mutex::new(outputs.into_iter().map(Ok).collect()),
            last: Mutex::new(None),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub fn with_error(err: LlmError) -> Self {
        Self {
            script: Mutex::new(VecDeque::from([Err(err)])),
            last: Mutex::new(None),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub fn with_tool_call(text: impl Into<String>, call: ToolCallRequest) -> Self {
        Self::with_outputs(vec![TurnOutput {
            text: text.into(),
            tool_calls: vec![call],
            usage: Usage::new(12, 7),
        }])
    }

    /// Requests seen so far (assert on prompts/tools in tests).
    pub fn seen_requests(&self) -> Vec<TurnRequest> {
        self.seen.lock().expect("mock seen").clone()
    }

    pub fn next_output(&self) -> Result<TurnOutput, LlmError> {
        let mut script = self.script.lock().expect("mock script");
        let out = script.pop_front().unwrap_or_else(|| {
            self.last
                .lock()
                .expect("mock last")
                .clone()
                .unwrap_or_else(|| {
                    Ok(TurnOutput {
                        text: "mock reply".into(),
                        ..Default::default()
                    })
                })
        });
        *self.last.lock().expect("mock last") = Some(out.clone());
        out
    }
}

impl ModelBackend for MockLlm {
    async fn turn(&self, req: TurnRequest) -> Result<TurnOutput, LlmError> {
        self.seen.lock().expect("mock seen").push(req);
        self.next_output()
    }

    async fn turn_stream(&self, req: TurnRequest) -> Result<Vec<AgentEvent>, LlmError> {
        let out = self.turn(req).await?;
        let mut events = Vec::new();
        for word in out.text.split_inclusive(' ') {
            if !word.is_empty() {
                events.push(AgentEvent::TextDelta(word.to_string()));
            }
        }
        if out.text.is_empty() && out.tool_calls.is_empty() {
            // keep the stream non-empty so UI code always sees progress
        }
        for tc in &out.tool_calls {
            events.push(AgentEvent::ToolCall(tc.clone()));
        }
        if out.usage != Usage::default() {
            events.push(AgentEvent::Usage(out.usage));
        }
        events.push(AgentEvent::Done {
            text: out.text.clone(),
            tool_calls: out.tool_calls.clone(),
            usage: out.usage,
        });
        Ok(events)
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;
    use serde_json::json;

    fn user_req(text: &str) -> TurnRequest {
        TurnRequest::new("test-model").with_message(TurnMessage::user(text))
    }

    #[tokio::test]
    async fn backend_mock_streams_text_tool_calls_and_usage() {
        let mock = MockLlm::with_tool_call(
            "hello world ",
            ToolCallRequest {
                id: "c1".into(),
                name: "ask_agent".into(),
                args: json!({"to": "bob"}),
            },
        );
        let events = mock.turn_stream(user_req("hi")).await.unwrap();
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::TextDelta(_))),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCall(c) if c.name == "ask_agent")),
            "{events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::Usage(_))),
            "{events:?}"
        );
        let done = events.iter().find_map(|e| match e {
            AgentEvent::Done {
                text,
                tool_calls,
                usage,
            } => Some((text.clone(), tool_calls.len(), *usage)),
            _ => None,
        });
        let (text, n_calls, usage) = done.expect("Done event");
        assert_eq!(text, "hello world ");
        assert_eq!(n_calls, 1);
        assert_eq!(usage.input_tokens, 12);
        // collected text round-trips through deltas
        let joined: String = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::TextDelta(d) => Some(d.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(joined, text);
        // script repeats + requests are recorded
        let out = mock.turn(user_req("again")).await.unwrap();
        assert_eq!(out.text, "hello world ");
        assert_eq!(mock.seen_requests().len(), 2);
    }

    #[tokio::test]
    async fn backend_mock_error_propagates() {
        let mock = MockLlm::with_error(LlmError::RateLimited("slow down".into()));
        let err = mock.turn(user_req("hi")).await.unwrap_err();
        assert!(err.retryable());
        assert!(err.user_message().contains("rate-limiting"));
        let err2 = mock.turn_stream(user_req("hi")).await.unwrap_err();
        assert_eq!(err, err2);
        assert!(!LlmError::Auth("x".into()).retryable());
    }

    #[test]
    fn backend_error_mapping_covers_genai_variants() {
        assert!(matches!(
            LlmError::from_genai(&genai::Error::RequiresApiKey {
                model_iden: genai::ModelIden::new(genai::adapter::AdapterKind::OpenAI, "m")
            }),
            LlmError::Auth(_)
        ));
        // stream variants map to Stream
        let stream_err = LlmError::from_genai(&genai::Error::WebStream {
            model_iden: genai::ModelIden::new(genai::adapter::AdapterKind::OpenAI, "m"),
            cause: "disconnected".to_string(),
        });
        assert!(matches!(stream_err, LlmError::Stream(_)));
        // string-based fallbacks (status codes embedded in provider messages)
        for (probe, expect) in [
            ("401 unauthorized", "auth"),
            ("429 too many requests", "rate"),
            ("404 model missing", "notfound"),
            ("400 bad schema", "bad"),
            ("weird failure", "provider"),
        ] {
            let err = LlmError::classify(probe);
            let tag = match err {
                LlmError::Auth(_) => "auth",
                LlmError::RateLimited(_) => "rate",
                LlmError::NotFound(_) => "notfound",
                LlmError::BadRequest(_) => "bad",
                LlmError::Stream(_) => "stream",
                LlmError::Provider(_) => "provider",
                LlmError::Network(_) => "network",
            };
            assert_eq!(tag, expect, "{probe}");
        }
    }

    /// Custom `base_url` must hit our own OpenAI-compatible server (not the
    /// real provider host) and return streaming text + usage.
    #[tokio::test]
    async fn backend_custom_base_url_turn_and_stream() {
        let server = wiremock::MockServer::start().await;
        let body = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hi from stub"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}
        });
        // non-streaming turn
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path_regex(".*chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;
        let backend = GenaiBackend::new(
            "test-model",
            Some("stub-key".into()),
            Some(format!("{}/v1/", server.uri())),
        );
        let out = backend.turn(user_req("hello")).await.unwrap();
        assert_eq!(out.text, "hi from stub");
        assert_eq!(
            out.usage,
            Usage {
                input_tokens: 3,
                output_tokens: 4,
                total_tokens: 7
            }
        );

        // streaming turn (SSE) — at least the text must arrive as deltas
        let sse = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi \"}}]}\n\n\
                   data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"there\"}}]}\n\n\
                   data: [DONE]\n\n";
        // reset server for the stream call
        server.reset().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path_regex(".*chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_bytes(sse.as_bytes().to_vec())
                    .insert_header("content-type", "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let events = backend.turn_stream(user_req("hello")).await.unwrap();
        let joined: String = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::TextDelta(d) => Some(d.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(joined, "hi there");
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Done { .. })));
    }

    #[tokio::test]
    async fn backend_tool_call_turn_from_openai_stub() {
        let server = wiremock::MockServer::start().await;
        let body = json!({
            "id": "chatcmpl-2",
            "object": "chat.completion",
            "created": 1,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"Tokyo\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 8, "total_tokens": 28}
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&server)
            .await;
        let backend = GenaiBackend::new(
            "test-model",
            Some("stub-key".into()),
            Some(format!("{}/v1/", server.uri())),
        );
        let req = TurnRequest::new("test-model")
            .with_message(TurnMessage::user("weather in Tokyo?"))
            .with_tool(
                ToolSpec::new("get_weather")
                    .with_description("weather")
                    .with_schema(json!({
                        "type": "object",
                        "properties": {"city": {"type": "string"}},
                        "required": ["city"]
                    })),
            );
        let out = backend.turn(req).await.unwrap();
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "get_weather");
        assert_eq!(out.tool_calls[0].args, json!({"city": "Tokyo"}));
        assert_eq!(out.usage.total_tokens, 28);
    }

    #[tokio::test]
    async fn backend_provider_error_maps_to_retryable() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(429).set_body_string("rate limited"))
            .expect(1)
            .mount(&server)
            .await;
        let backend = GenaiBackend::new(
            "test-model",
            Some("stub-key".into()),
            Some(format!("{}/v1/", server.uri())),
        );
        let err = backend.turn(user_req("hi")).await.unwrap_err();
        assert!(
            matches!(err, LlmError::RateLimited(_) | LlmError::Provider(_)),
            "{err:?}"
        );
    }
}
