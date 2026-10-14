//! Single-agent chat view: virtualised message list, streaming markdown,
//! composer with send/stop morph and kept draft (see docs/UI.md §1, §3, §5).
//!
//! Text deltas are coalesced per frame: [`ChatState::push_delta`] buffers and
//! [`ChatState::flush_frame`] applies at most one update per message per
//! frame (PERFORMANCE.md rule 3).

use std::collections::HashMap;

use synapse_llm::{AgentEvent, LlmError, ModelBackend, Role, TurnMessage, TurnRequest};

/// Who sent a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Agent,
    System,
}

/// One chat message. `streaming` is true while deltas are still arriving.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: usize,
    pub role: ChatRole,
    pub author: String,
    pub body: String,
    pub streaming: bool,
}

impl ChatMessage {
    pub fn new(
        id: usize,
        role: ChatRole,
        author: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            id,
            role,
            author: author.into(),
            body: body.into(),
            streaming: false,
        }
    }
}

/// A fenced code block parsed out of markdown (for code highlighting).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatCodeBlock {
    /// Lower-cased language tag (`""` when the fence has none).
    pub language: String,
    pub code: String,
}

/// Parse ```` ``` ```` fenced code blocks out of markdown.
pub fn fenced_code_blocks(md: &str) -> Vec<ChatCodeBlock> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut language = String::new();
    let mut code = String::new();
    for line in md.lines() {
        let trimmed = line.trim_start();
        if let Some(fence) = trimmed.strip_prefix("```") {
            if in_block {
                out.push(ChatCodeBlock {
                    language: std::mem::take(&mut language),
                    code: std::mem::take(&mut code),
                });
                in_block = false;
            } else {
                language = fence.trim().to_lowercase();
                code.clear();
                in_block = true;
            }
        } else if in_block {
            if !code.is_empty() {
                code.push('\n');
            }
            code.push_str(line);
        }
    }
    // Unclosed fence still counts as a streaming block.
    if in_block {
        out.push(ChatCodeBlock { language, code });
    }
    out
}

/// Resolve a fence language tag against the tree-sitter grammars GPUI Kit
/// highlights (STACK.md: rust, python, js/ts, json, bash, markdown).
/// Returns the canonical language name, or `None` when unknown.
pub fn highlight_language(tag: &str) -> Option<String> {
    let tag = tag.trim().to_lowercase();
    let canonical = match tag.as_str() {
        "" => return None,
        "rs" => "rust",
        "py" => "python",
        "js" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "sh" | "shell" | "zsh" => "bash",
        "md" | "mdx" => "markdown",
        "rust" | "python" | "javascript" | "typescript" | "json" | "bash" | "markdown" => {
            tag.as_str()
        }
        _ => return None,
    };
    Some(canonical.to_string())
}

/// Buffers text deltas between frames; one flush per frame per message.
#[derive(Debug, Default)]
pub struct DeltaCoalescer {
    pending: String,
    pushes: usize,
}

impl DeltaCoalescer {
    pub fn push(&mut self, delta: &str) {
        self.pending.push_str(delta);
        self.pushes += 1;
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn push_count(&self) -> usize {
        self.pushes
    }

    pub fn take_pending(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        self.pushes = 0;
        Some(std::mem::take(&mut self.pending))
    }
}

/// Owned state for the single-agent chat view.
///
/// The GPUI view (`ChatView` below) renders this with a virtualised
/// [`MessageScroller`](gpui_kit::component::message_scroller::MessageScroller);
/// `item_count` mirrors the scroller row count.
#[derive(Debug, Default)]
pub struct ChatState {
    messages: Vec<ChatMessage>,
    pending: HashMap<usize, DeltaCoalescer>,
    next_id: usize,
    draft: String,
    running: bool,
    frame_flushes: u64,
    last_error: Option<String>,
}

impl ChatState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of rows the virtualised list must show.
    pub fn item_count(&self) -> usize {
        self.messages.len()
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn message(&self, id: usize) -> Option<&ChatMessage> {
        self.messages.iter().find(|m| m.id == id)
    }

    pub fn draft(&self) -> &str {
        &self.draft
    }

    pub fn set_draft(&mut self, draft: impl Into<String>) {
        self.draft = draft.into();
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Actionable provider error from the last failed turn, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    /// Composer button label: morphs between send and stop.
    pub fn send_label(&self) -> &'static str {
        if self.running {
            "Stop"
        } else {
            "Send"
        }
    }

    fn alloc_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Push a user message from the current draft; clears the draft.
    pub fn send(&mut self) -> Option<usize> {
        let body = self.draft.trim().to_string();
        if body.is_empty() || self.running {
            return None;
        }
        let id = self.alloc_id();
        self.messages
            .push(ChatMessage::new(id, ChatRole::User, "You", body));
        self.draft.clear();
        self.running = true;
        // Agent placeholder the stream fills in.
        let agent = self.alloc_id();
        self.messages.push(ChatMessage {
            id: agent,
            role: ChatRole::Agent,
            author: "Agent".to_string(),
            body: String::new(),
            streaming: true,
        });
        Some(agent)
    }

    /// Stop generation; the draft is kept (UI.md §5).
    pub fn stop(&mut self) {
        self.running = false;
        self.pending.clear();
        for m in &mut self.messages {
            m.streaming = false;
        }
    }

    /// Mark the run finished (stream complete).
    pub fn finish(&mut self) {
        self.flush_frame();
        self.running = false;
        for m in &mut self.messages {
            m.streaming = false;
        }
    }

    /// Buffer a streaming text delta for `id`. Coalesced until [`Self::flush_frame`].
    pub fn push_delta(&mut self, id: usize, delta: &str) {
        if self.messages.iter().any(|m| m.id == id) {
            self.pending.entry(id).or_default().push(delta);
        }
    }

    /// Apply buffered deltas: at most one body update per message per frame.
    /// Returns the ids that changed (one `notify` per id at the view layer).
    pub fn flush_frame(&mut self) -> Vec<usize> {
        let mut changed = Vec::new();
        for (id, coalescer) in self.pending.iter_mut() {
            if let Some(text) = coalescer.take_pending() {
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == *id) {
                    m.body.push_str(&text);
                    changed.push(*id);
                }
            }
        }
        // Drop emptied entries.
        self.pending.retain(|_, c| c.pending_len() > 0);
        if !changed.is_empty() {
            self.frame_flushes += 1;
        }
        changed
    }

    pub fn frame_flushes(&self) -> u64 {
        self.frame_flushes
    }

    pub fn pending_delta_len(&self, id: usize) -> usize {
        self.pending.get(&id).map(|c| c.pending_len()).unwrap_or(0)
    }

    /// Build a [`TurnRequest`] from the visible history (user/assistant/system
    /// messages with non-empty bodies). The streaming placeholder (empty,
    /// `streaming == true`) is excluded. Call after [`Self::send`].
    pub fn history_request(&self, model: &str) -> TurnRequest {
        let mut req = TurnRequest::new(model);
        for m in &self.messages {
            if m.streaming && m.body.is_empty() {
                continue;
            }
            if m.body.is_empty() {
                continue;
            }
            let msg = match m.role {
                ChatRole::User => TurnMessage {
                    role: Role::User,
                    text: m.body.clone(),
                },
                ChatRole::Agent => TurnMessage {
                    role: Role::Assistant,
                    text: m.body.clone(),
                },
                ChatRole::System => TurnMessage {
                    role: Role::System,
                    text: m.body.clone(),
                },
            };
            req.messages.push(msg);
        }
        req
    }

    /// Send the draft and return the agent placeholder id plus the backend
    /// request for the turn. Returns `None` when there is nothing to send.
    pub fn send_request(&mut self, model: &str) -> Option<(usize, TurnRequest)> {
        let agent = self.send()?;
        self.clear_error();
        Some((agent, self.history_request(model)))
    }

    /// Apply one backend stream event to the agent message. Text deltas are
    /// buffered (see [`Self::flush_frame`]); `Done` flushes and finishes the
    /// run. Tool calls and usage are ignored by this single-agent view.
    /// No-op when the run was stopped.
    pub fn apply_event(&mut self, id: usize, event: AgentEvent) {
        if !self.running {
            return;
        }
        match event {
            AgentEvent::TextDelta(delta) => {
                self.push_delta(id, &delta);
                self.flush_frame();
            }
            AgentEvent::ToolCall(_) | AgentEvent::Usage(_) => {}
            AgentEvent::Done { .. } => {
                self.finish();
            }
        }
    }

    /// Record a provider failure on the agent placeholder and surface the
    /// actionable message (no key material; see ARCHITECTURE.md §6).
    pub fn fail_turn(&mut self, id: usize, err: &LlmError) {
        self.flush_frame();
        self.running = false;
        for m in &mut self.messages {
            m.streaming = false;
        }
        let message = err.user_message();
        if let Some(m) = self.messages.iter_mut().find(|m| m.id == id) {
            if m.body.is_empty() {
                m.body = message.clone();
            }
        }
        self.last_error = Some(message);
    }

    /// Drive one backend turn into the agent placeholder: streams text deltas
    /// into the markdown state, finishes on `Done`, and shows actionable
    /// provider errors. Stopping mid-turn keeps the draft and discards late
    /// events (the view layer aborts this future on stop).
    pub async fn drive_turn<B: ModelBackend>(
        &mut self,
        backend: &B,
        agent_id: usize,
        req: TurnRequest,
    ) {
        let events = match backend.turn_stream(req).await {
            Ok(events) => events,
            Err(err) => {
                self.fail_turn(agent_id, &err);
                return;
            }
        };
        for event in events {
            if !self.running {
                break;
            }
            self.apply_event(agent_id, event);
        }
    }
}

// ---- GPUI view (virtualised list + streaming markdown) ----

use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::v_flex;
use gpui_kit::{
    div, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Window,
};

/// GPUI view rendering [`ChatState`] as a virtualised message list.
///
/// - Rows are virtualised by GPUI Kit's `MessageScroller` (off-screen
///   markdown is not laid out).
/// - Bodies render with `TextView::markdown(...).stream_fade(true)` (word
///   fade-in) plus tree-sitter code highlighting via `code_block_actions`.
/// - The composer morphs Send ⇄ Stop; the draft is kept on stop.
pub struct ChatView {
    state: ChatState,
    scroller: Entity<MessageScrollerState>,
}

impl ChatView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        Self {
            state: ChatState::new(),
            scroller,
        }
    }

    pub fn chat_state(&self) -> &ChatState {
        &self.state
    }

    pub fn chat_state_mut(&mut self) -> &mut ChatState {
        &mut self.state
    }

    fn sync_scroller(&mut self, cx: &mut Context<Self>) {
        let count = self.state.item_count();
        if self.scroller.read(cx).item_count() != count {
            self.scroller.update(cx, |s, cx| s.reset(count, cx));
        }
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_scroller(cx);
        let bodies: Vec<String> = self
            .state
            .messages()
            .iter()
            .map(|m| m.body.clone())
            .collect();
        let authors: Vec<String> = self
            .state
            .messages()
            .iter()
            .map(|m| m.author.clone())
            .collect();
        let send_label = self.state.send_label().to_string();
        let draft = self.state.draft().to_string();

        v_flex()
            .size_full()
            .child(div().flex_1().child(MessageScroller::new(
                "chat-view-scroller",
                self.scroller.clone(),
                move |ix, _window: &mut Window, _cx: &mut App| {
                    let body = bodies.get(ix).cloned().unwrap_or_default();
                    let author = authors.get(ix).cloned().unwrap_or_default();
                    v_flex().child(div().child(author)).child(
                        TextView::markdown(format!("chat-msg-{ix}"), body)
                            .stream_fade(true)
                            .code_block_actions(|_block, _, _| div()),
                    )
                },
            )))
            .child(
                div()
                    .child(div().child(format!("Draft: {draft}")))
                    .child(div().child(format!("Button: {send_label}"))),
            )
    }
}

#[cfg(test)]
mod chat_view_tests {
    use super::*;

    #[test]
    fn chat_view_send_starts_run_and_clears_draft() {
        let mut s = ChatState::new();
        s.set_draft("hello agent");
        let agent = s.send().expect("send");
        assert_eq!(s.draft(), "");
        assert!(s.is_running());
        assert_eq!(s.send_label(), "Stop");
        assert_eq!(s.item_count(), 2);
        assert!(s.message(agent).unwrap().streaming);
    }

    #[test]
    fn chat_view_stop_keeps_draft() {
        let mut s = ChatState::new();
        s.set_draft("first");
        s.send();
        s.set_draft("follow-up typed while running");
        s.stop();
        assert!(!s.is_running());
        assert_eq!(s.send_label(), "Send");
        assert_eq!(s.draft(), "follow-up typed while running");
    }

    #[test]
    fn chat_view_deltas_coalesced_per_frame() {
        let mut s = ChatState::new();
        s.set_draft("go");
        let agent = s.send().unwrap();
        // Many deltas arrive within one frame...
        for w in ["Hello", " ", "big", " ", "world", "!"] {
            s.push_delta(agent, w);
        }
        // ...buffered, body untouched until the frame flush.
        assert_eq!(s.message(agent).unwrap().body, "");
        assert!(s.pending_delta_len(agent) > 0);
        let changed = s.flush_frame();
        assert_eq!(changed, vec![agent]);
        assert_eq!(s.message(agent).unwrap().body, "Hello big world!");
        assert_eq!(s.frame_flushes(), 1);
        // Nothing pending: flush is a no-op.
        assert!(s.flush_frame().is_empty());
    }

    #[test]
    fn chat_view_code_blocks_highlight() {
        let md = "try it:\n```rust\nfn main() {}\n```\ndone\n```python\nprint(1)\n```";
        let blocks = fenced_code_blocks(md);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].language, "rust");
        assert_eq!(highlight_language("rust").as_deref(), Some("rust"));
        assert_eq!(highlight_language("python").as_deref(), Some("python"));
        assert_eq!(highlight_language(""), None);
        assert_eq!(highlight_language("not-a-lang-xyz"), None);
        // Streaming: unclosed fence still parses.
        let partial = "half:\n```js\nconsole.log(";
        assert_eq!(fenced_code_blocks(partial).len(), 1);
    }

    #[test]
    fn chat_view_streaming_markdown_flag_and_finish() {
        let mut s = ChatState::new();
        s.set_draft("summarise");
        let agent = s.send().unwrap();
        s.push_delta(agent, "# Title\nsome **bold** text");
        s.flush_frame();
        assert!(s.message(agent).unwrap().streaming);
        s.finish();
        assert!(!s.is_running());
        assert!(!s.message(agent).unwrap().streaming);
        assert_eq!(
            s.message(agent).unwrap().body,
            "# Title\nsome **bold** text"
        );
    }

    #[tokio::test]
    async fn chat_view_send_drives_backend_turn_and_streams() {
        use synapse_llm::MockLlm;
        let backend = MockLlm::with_text("hello world ");
        let mut s = ChatState::new();
        s.set_draft("hi there");
        let (agent, req) = s.send_request("test-model").expect("send");
        assert_eq!(s.draft(), "");
        assert!(s.is_running());
        // The prompt reaches the backend.
        assert!(req.messages.iter().any(|m| m.text == "hi there"));
        s.drive_turn(&backend, agent, req).await;
        assert!(!s.is_running());
        assert!(s.last_error().is_none());
        assert_eq!(s.message(agent).unwrap().body, "hello world ");
        assert!(!s.message(agent).unwrap().streaming);
        assert_eq!(backend.seen_requests().len(), 1);
    }

    #[tokio::test]
    async fn chat_view_stop_discards_late_stream_and_keeps_draft() {
        use synapse_llm::MockLlm;
        let backend = MockLlm::with_text("late reply");
        let mut s = ChatState::new();
        s.set_draft("go");
        let (agent, req) = s.send_request("test-model").expect("send");
        s.set_draft("follow-up typed while running");
        s.stop();
        s.drive_turn(&backend, agent, req).await;
        // Stopped: no late deltas applied, draft kept.
        assert_eq!(s.message(agent).unwrap().body, "");
        assert_eq!(s.draft(), "follow-up typed while running");
        assert!(!s.is_running());
    }

    #[tokio::test]
    async fn chat_view_backend_error_shows_actionable_message() {
        use synapse_llm::{LlmError, MockLlm};
        let backend = MockLlm::with_error(LlmError::Auth("no key".into()));
        let mut s = ChatState::new();
        s.set_draft("hello");
        let (agent, req) = s.send_request("test-model").expect("send");
        s.drive_turn(&backend, agent, req).await;
        assert!(!s.is_running());
        assert!(!s.message(agent).unwrap().streaming);
        let shown = s.last_error().expect("error shown");
        assert!(shown.contains("Settings"), "{shown}");
        assert_eq!(s.message(agent).unwrap().body, shown);
    }
}
