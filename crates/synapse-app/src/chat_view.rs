//! Single-agent chat view: virtualised message list, streaming markdown,
//! composer with send/stop morph and kept draft (see docs/UI.md §1, §3, §5).
//!
//! Text deltas are coalesced per frame: [`ChatState::push_delta`] buffers and
//! [`ChatState::flush_frame`] applies at most one update per message per
//! frame (PERFORMANCE.md rule 3).

use std::collections::HashMap;

use synapse_llm::{
    AgentEvent, Catalog, ConversationCost, LlmError, MessageCost, ModelBackend, Role, TurnMessage,
    TurnRequest, Usage,
};

/// Who sent a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    /// The lead: the voice the user is talking to.
    Agent,
    /// Another member of the team, speaking in the chat.
    Teammate,
    /// A short line about what the team is doing ("Lead asked Mira ...").
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
    /// Teammates this message pinged, shown as small avatars beneath it.
    pub pings: Vec<Ping>,
}

/// A request one teammate sent to another in a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ping {
    pub to: String,
    pub body: String,
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
            pings: Vec::new(),
        }
    }
}

/// The pings stored in a message's meta JSON.
fn pings_from_meta(meta: &str) -> Vec<Ping> {
    serde_json::from_str::<serde_json::Value>(meta)
        .ok()
        .and_then(|m| m["pings"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| {
            Some(Ping { to: p["to"].as_str()?.to_string(), body: p["body"].as_str()?.to_string() })
        })
        .collect()
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
    /// Provider-reported usage per agent message (stream `Usage`/`Done`).
    usages: HashMap<usize, Usage>,
    /// Priced costs per agent message (see [`Self::price_turn`]).
    costs: HashMap<usize, MessageCost>,
    conversation_id: String,
    /// Leading messages already written by [`Self::save_to_store`].
    saved: usize,
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
            pings: Vec::new(),
        });
        Some(agent)
    }

    /// Like [`Self::send`] for a team turn: push the user message only (each
    /// speaker adds their own message as they start) and return the text plus
    /// the conversation so far for the team to answer.
    pub fn send_team(&mut self) -> Option<(String, Vec<TurnMessage>)> {
        let body = self.draft.trim().to_string();
        if body.is_empty() || self.running {
            return None;
        }
        let id = self.alloc_id();
        self.messages
            .push(ChatMessage::new(id, ChatRole::User, "You", body.clone()));
        self.draft.clear();
        self.running = true;
        self.clear_error();
        Some((body, self.team_history()))
    }

    /// The conversation as a team member should read it: what the user and the
    /// lead said, and what teammates reported (ask blocks and status lines
    /// left out).
    pub fn team_history(&self) -> Vec<TurnMessage> {
        use synapse_core::team_dialogue::visible_text;
        let mut out = Vec::new();
        for m in &self.messages {
            let text = match m.role {
                ChatRole::User => m.body.clone(),
                ChatRole::Agent | ChatRole::Teammate => visible_text(&m.body),
                ChatRole::System => continue,
            };
            if text.trim().is_empty() {
                continue;
            }
            out.push(match m.role {
                ChatRole::User => TurnMessage::user(text),
                ChatRole::Agent => TurnMessage::assistant(text),
                _ => TurnMessage::user(format!("[{} replied earlier]\n{text}", m.author)),
            });
        }
        out
    }

    /// A new message under `author`'s name that deltas will fill.
    pub fn begin_speaker(&mut self, author: &str, role: ChatRole) -> usize {
        let id = self.alloc_id();
        self.messages.push(ChatMessage {
            id,
            role,
            author: author.to_string(),
            body: String::new(),
            streaming: true,
            pings: Vec::new(),
        });
        id
    }

    /// `from`'s latest message pinged `to` with `body`.
    pub fn add_ping(&mut self, from: &str, to: &str, body: &str) {
        let latest = self
            .messages
            .iter_mut()
            .rev()
            .find(|m| m.author == from && matches!(m.role, ChatRole::Agent | ChatRole::Teammate));
        if let Some(m) = latest {
            m.pings.push(Ping { to: to.to_string(), body: body.to_string() });
        }
    }

    /// One message is complete (others may still be streaming).
    pub fn end_message(&mut self, id: usize) {
        self.flush_frame();
        if let Some(m) = self.messages.iter_mut().find(|m| m.id == id) {
            m.streaming = false;
        }
    }

    /// Record the usage reported for a message.
    pub fn set_usage(&mut self, id: usize, usage: Usage) {
        self.usages.insert(id, usage);
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
                ChatRole::Teammate => TurnMessage {
                    role: Role::User,
                    text: format!("[{} replied earlier]\n{}", m.author, m.body),
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
    /// buffered (see [`Self::flush_frame`]); usage is recorded for pricing;
    /// `Done` flushes and finishes the run. Tool calls are ignored by this
    /// single-agent view. No-op when the run was stopped.
    pub fn apply_event(&mut self, id: usize, event: AgentEvent) {
        if !self.running {
            return;
        }
        match event {
            AgentEvent::TextDelta(delta) => {
                self.push_delta(id, &delta);
                self.flush_frame();
            }
            AgentEvent::ToolCall(_) => {}
            AgentEvent::Usage(usage) => {
                if usage != Usage::default() {
                    self.usages.insert(id, usage);
                }
            }
            AgentEvent::Done { usage, .. } => {
                if usage != Usage::default() {
                    self.usages.insert(id, usage);
                }
                self.finish();
            }
        }
    }

    /// Provider-reported usage for an agent message, if the stream sent one.
    pub fn usage_for(&self, id: usize) -> Option<Usage> {
        self.usages.get(&id).copied()
    }

    /// Priced cost for an agent message, if [`Self::price_turn`] ran.
    pub fn cost_for(&self, id: usize) -> Option<&MessageCost> {
        self.costs.get(&id)
    }

    /// Conversation this chat persists to (see [`Self::save_to_store`]).
    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub fn set_conversation_id(&mut self, id: impl Into<String>) {
        self.conversation_id = id.into();
    }

    /// Price a finished turn from its recorded usage and remember the cost
    /// for the header label and [`Self::save_to_store`]. Unknown models
    /// price at 0.0 rather than failing the turn. Returns `None` when the
    /// stream reported no usage.
    pub fn price_turn(
        &mut self,
        id: usize,
        catalog: &Catalog,
        provider_id: &str,
        model_id: &str,
    ) -> Option<MessageCost> {
        let usage = self.usages.get(&id).copied()?;
        let message_id = self.persisted_id(id);
        let cost = MessageCost::new(
            catalog,
            message_id,
            self.conversation_id.clone(),
            provider_id,
            model_id,
            usage,
        );
        self.costs.insert(id, cost.clone());
        Some(cost)
    }

    /// Totals over the priced turns, for the chat header / title bar.
    pub fn conversation_cost(&self) -> ConversationCost {
        let ordered: Vec<MessageCost> = self
            .messages
            .iter()
            .filter_map(|m| self.costs.get(&m.id).cloned())
            .collect();
        ConversationCost::summarize(self.conversation_id.clone(), &ordered)
    }

    fn persisted_id(&self, id: usize) -> String {
        if self.conversation_id.is_empty() {
            format!("msg-{id}")
        } else {
            format!("{}-m{id}", self.conversation_id)
        }
    }

    /// Persist messages not yet saved to the conversation (user rows plus
    /// agent rows with `{"cost": ...}` meta), so the chat survives
    /// reopening (see [`Self::load_from_store`]). No-op when no
    /// conversation is set. Returns the number of newly saved rows.
    pub fn save_to_store(
        &mut self,
        store: &synapse_store::Store,
    ) -> Result<usize, synapse_store::StoreError> {
        if self.conversation_id.is_empty() || self.saved >= self.messages.len() {
            return Ok(0);
        }
        let mut saved = 0;
        for m in self.messages.iter().skip(self.saved) {
            let (from_id, kind) = match m.role {
                ChatRole::User => ("user".to_string(), "ask"),
                ChatRole::Agent => ("agent".to_string(), "answer"),
                ChatRole::Teammate => (format!("agent:{}", m.author), "answer"),
                ChatRole::System => ("system".to_string(), "note"),
            };
            let mut meta = serde_json::json!({});
            if m.role == ChatRole::Agent {
                meta["author"] = m.author.clone().into();
            }
            if !m.pings.is_empty() {
                meta["pings"] = m.pings.iter().map(|p| serde_json::json!({"to": p.to, "body": p.body})).collect();
            }
            if let Some(cost) = self.costs.get(&m.id) {
                if let Ok(cost) = serde_json::from_str::<serde_json::Value>(&cost.meta_json()) {
                    meta["cost"] = cost;
                }
            }
            let body = match m.role {
                ChatRole::Agent | ChatRole::Teammate => {
                    synapse_core::team_dialogue::visible_text(&m.body)
                }
                _ => m.body.clone(),
            };
            // A teammate who passed said nothing; keep nothing.
            if body.trim().is_empty() && m.role != ChatRole::User && m.pings.is_empty() {
                saved += 1;
                continue;
            }
            store.insert_message(&synapse_store::NewMessage {
                id: self.persisted_id(m.id),
                conversation_id: self.conversation_id.clone(),
                run_id: String::new(),
                from_id,
                to_json: "[]".into(),
                kind: kind.into(),
                reply_to: String::new(),
                thread_id: String::new(),
                body,
                meta: meta.to_string(),
            })?;
            saved += 1;
        }
        self.saved += saved;
        Ok(saved)
    }

    /// Reload a persisted conversation, replacing the visible history
    /// (the draft is kept). Costs and usage rebuild from the stored
    /// message meta so the header totals match.
    pub fn load_from_store(
        &mut self,
        store: &synapse_store::Store,
        conversation_id: &str,
    ) -> Result<ConversationCost, synapse_store::StoreError> {
        let rows = store.list_messages(conversation_id, i64::MAX, 0)?;
        self.messages.clear();
        self.pending.clear();
        self.usages.clear();
        self.costs.clear();
        self.next_id = 0;
        self.running = false;
        self.conversation_id = conversation_id.to_string();
        for row in &rows {
            let pings = pings_from_meta(&row.meta);
            if row.body.trim().is_empty() && pings.is_empty() {
                continue;
            }
            let role = match row.from_id.as_str() {
                "user" => ChatRole::User,
                "system" => ChatRole::System,
                from if from.starts_with("agent:") => ChatRole::Teammate,
                _ => ChatRole::Agent,
            };
            let named = serde_json::from_str::<serde_json::Value>(&row.meta)
                .ok()
                .and_then(|m| m["author"].as_str().map(str::to_string));
            let author = match role {
                ChatRole::User => "You".to_string(),
                ChatRole::Agent => named.unwrap_or_else(|| "Agent".to_string()),
                ChatRole::Teammate => row.from_id.trim_start_matches("agent:").to_string(),
                ChatRole::System => "System".to_string(),
            };
            let id = self.alloc_id();
            self.messages.push(ChatMessage {
                id,
                role,
                author,
                body: row.body.clone(),
                streaming: false,
                pings,
            });
            if role == ChatRole::Agent {
                if let Some(cost) =
                    MessageCost::from_meta(row.id.clone(), conversation_id, &row.meta)
                {
                    let has_cost =
                        cost.input_tokens > 0 || cost.output_tokens > 0 || cost.cost_usd > 0.0;
                    self.usages
                        .insert(id, Usage::new(cost.input_tokens, cost.output_tokens));
                    if has_cost {
                        let mut keyed = cost;
                        keyed.message_id = self.persisted_id(id);
                        keyed.conversation_id = conversation_id.to_string();
                        self.costs.insert(id, keyed);
                    }
                }
            }
        }
        self.saved = self.messages.len();
        Ok(self.conversation_cost())
    }

    /// A team turn failed: stop, and show what to do about it.
    pub fn fail_team_turn(&mut self, err: &LlmError) {
        self.flush_frame();
        self.running = false;
        for m in &mut self.messages {
            m.streaming = false;
        }
        self.last_error = Some(err.user_message());
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

// ---- GPUI view (virtualised list + streaming markdown + composer) ----

use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::Sizable as _;
use gpui_kit::InteractiveElement as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _};
use gpui_kit::{
    div, px, App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window,
};

/// Run one turn on the tokio runtime: the provider's HTTP client needs it,
/// and GPUI's own executor has none.
async fn run_turn<B: ModelBackend + 'static>(
    backend: std::sync::Arc<B>,
    req: TurnRequest,
) -> Result<Vec<AgentEvent>, LlmError> {
    crate::runtime::spawn(async move { backend.turn_stream(req).await })
        .await
        .map_err(|e| LlmError::Stream(e.to_string()))?
}

/// A model wrote the goal of this project from its first prompt, and named
/// it when it still had the placeholder name.
pub struct GoalWritten {
    pub project_id: String,
    pub goal: String,
    pub name: Option<String>,
}

/// What the view needs to price and persist a finished turn. All fields are
/// optional so the chat still works (without costs/persistence) when the
/// store or catalog is unavailable.
#[derive(Clone, Default)]
pub struct ChatPersist {
    pub store: Option<std::sync::Arc<synapse_store::Store>>,
    pub catalog: Option<std::sync::Arc<Catalog>>,
    pub provider_id: String,
    pub model_id: String,
}

/// GPUI view rendering [`ChatState`] as a virtualised message list with a
/// real composer.
///
/// - Rows are virtualised by GPUI Kit's `MessageScroller` (off-screen
///   markdown is not laid out).
/// - Bodies render with `TextView::markdown(...).stream_fade(true)` (word
///   fade-in) plus tree-sitter code highlighting via `code_block_actions`.
/// - The composer is a real single-line [`Input`] whose text mirrors
///   [`ChatState::draft`]; the Send button morphs into Stop while the
///   selected [`ModelBackend`] streams, and clicking it sends or stops.
///   Provider errors render above the composer; the draft is kept on stop.
/// - Finished turns are priced ([`ChatPersist`]) and persisted to the store
///   so the chat survives reopening.
pub struct ChatView<B> {
    state: ChatState,
    scroller: Entity<MessageScrollerState>,
    composer: Entity<TextareaState>,
    backend: std::sync::Arc<B>,
    model: String,
    persist: ChatPersist,
    /// The small model for quick jobs (Models page); the Lead model when unset.
    small: Option<(std::sync::Arc<B>, String)>,
    /// Project still without a goal: the next prompt is summarised into one.
    goal_for: Option<String>,
    /// System prompt every turn starts with (project, goal, team).
    system: Option<String>,
    /// The open project's team: when set, messages go to the lead (or the
    /// member the user @mentions) and teammates answer in the chat.
    team: Option<std::sync::Arc<crate::team_run::TeamEnv<B>>>,
    /// Teammates' folded working notes (by message id) the user opened.
    opened: std::collections::HashSet<usize>,
    _composer_sub: Subscription,
}

impl<B> EventEmitter<GoalWritten> for ChatView<B> where B: ModelBackend + 'static {}

impl<B> ChatView<B>
where
    B: ModelBackend + 'static,
{
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        backend: std::sync::Arc<B>,
        model: impl Into<String>,
    ) -> Self {
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("Message the agent…")
        });
        let composer_sub = cx.subscribe_in(&composer, window, Self::on_composer_event);
        Self {
            state: ChatState::new(),
            scroller,
            composer,
            backend,
            model: model.into(),
            persist: ChatPersist::default(),
            small: None,
            goal_for: None,
            system: None,
            team: None,
            opened: std::collections::HashSet::new(),
            _composer_sub: composer_sub,
        }
    }

    /// Use `small` for quick jobs such as naming a project.
    pub fn set_small(&mut self, small: Option<(std::sync::Arc<B>, String)>) {
        self.small = small;
    }

    /// Switch to a freshly built backend (key, model or provider changed).
    pub fn reconfigure(&mut self, backend: std::sync::Arc<B>, model: String, persist: ChatPersist) {
        self.backend = backend;
        self.model = model;
        self.persist = persist;
    }

    /// The team that answers (`None`: a plain single-model chat).
    pub fn set_team(&mut self, team: Option<std::sync::Arc<crate::team_run::TeamEnv<B>>>) {
        self.team = team;
    }

    /// The system prompt for the next turns (`None`: send none).
    pub fn set_system(&mut self, system: Option<String>) {
        self.system = system;
    }

    /// Summarise the next prompt into this project's goal (`None`: stop).
    pub fn write_goal_for(&mut self, project_id: Option<String>) {
        self.goal_for = project_id;
    }

    pub fn with_persist(mut self, persist: ChatPersist) -> Self {
        self.persist = persist;
        self
    }

    /// Point this chat at a persisted conversation and reload its history
    /// so results survive reopening.
    pub fn open_conversation(
        &mut self,
        store: &synapse_store::Store,
        conversation_id: &str,
    ) -> ConversationCost {
        self.state
            .load_from_store(store, conversation_id)
            .unwrap_or_else(|_| self.state.conversation_cost())
    }

    pub fn chat_state(&self) -> &ChatState {
        &self.state
    }

    pub fn chat_state_mut(&mut self) -> &mut ChatState {
        &mut self.state
    }

    /// Store backing this chat's persistence, if configured.
    pub fn persist_store(&self) -> Option<std::sync::Arc<synapse_store::Store>> {
        self.persist.store.clone()
    }

    fn on_composer_event(
        &mut self,
        _: &Entity<TextareaState>,

        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            let text = self.composer.read(cx).value().to_string();
            self.state.set_draft(text);
            cx.notify();
        }
        // Enter sends; Shift+Enter inserts a newline (the textarea handles it).
        if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
            self.on_send_click(window, cx);
        }
    }

    fn sync_scroller(&mut self, cx: &mut Context<Self>) {
        let count = self.state.item_count();
        if self.scroller.read(cx).item_count() != count {
            self.scroller.update(cx, |s, cx| s.reset(count, cx));
        }
    }

    /// Open or fold a teammate's working note.
    fn toggle_note(&mut self, id: usize, cx: &mut Context<Self>) {
        if !self.opened.remove(&id) {
            self.opened.insert(id);
        }
        let count = self.state.item_count();
        self.scroller.update(cx, |s, cx| s.reset(count, cx));
        cx.notify();
    }

    /// Have a model write the project's goal from `text` when it has none.
    fn start_goal_summary(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(project_id) = self.goal_for.take() else {
            return;
        };
        let (backend, model) = self
            .small
            .clone()
            .unwrap_or_else(|| (self.backend.clone(), self.model.clone()));
        let store = self.persist.store.clone();
        let req = crate::project_goal::request(&model, text);
        cx.spawn(async move |this, cx| {
            let events = match run_turn(backend, req).await {
                Ok(events) => events,
                Err(e) => {
                    eprintln!("project naming failed: {e}");
                    return;
                }
            };
            let dump = format!("{events:?}");
            let Some(drafted) = crate::project_goal::from_events(events) else {
                eprintln!("project naming: the model returned no name: {dump}");
                return;
            };
            let Some(store) = store else {
                return;
            };
            if store.set_project_goal(&project_id, &drafted.goal).is_err() {
                return;
            }
            let untitled = store
                .get_project(&project_id)
                .is_ok_and(|p| p.name == crate::project_setup::UNTITLED);
            let name = untitled
                .then(|| store.rename_project(&project_id, &drafted.name).ok())
                .flatten()
                .map(|p| p.name);
            let goal = drafted.goal;
            _ = this.update(cx, |_, cx| cx.emit(GoalWritten { project_id, goal, name }));
        })
        .detach();
    }

    /// Send the user's message to the team: the lead (or the @mentioned
    /// member) answers, asks teammates when it helps, and every speaker
    /// streams into the chat under their own name.
    fn send_to_team(
        &mut self,
        team: std::sync::Arc<crate::team_run::TeamEnv<B>>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::team_run::{run_team_turn, TeamEvent};
        let Some((body, history)) = self.state.send_team() else {
            return;
        };
        self.composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
        self.start_goal_summary(&text, cx);
        let speaker = synapse_core::team_dialogue::mentioned(&team.members, &body)
            .map_or_else(|| team.lead_id.clone(), |m| m.id.clone());
        let persist = self.persist.clone();
        cx.spawn_in(window, async move |this, cx| {
            let (tx, mut rx) = tokio::sync::mpsc::channel(128);
            let task = crate::runtime::spawn(async move {
                run_team_turn(team, &speaker, history, tx).await
            });
            let mut messages: HashMap<u32, usize> = HashMap::new();
            let mut stopped = false;
            while let Some(event) = rx.recv().await {
                let running = this
                    .update_in(cx, |this, _, _| this.state.is_running())
                    .unwrap_or(false);
                if !running {
                    stopped = true;
                    break;
                }
                _ = this.update_in(cx, |this, _, cx| {
                    match event {
                        TeamEvent::Start { stream, speaker, lead } => {
                            let role = if lead { ChatRole::Agent } else { ChatRole::Teammate };
                            let id = this.state.begin_speaker(&speaker, role);
                            messages.insert(stream, id);
                        }
                        TeamEvent::Delta { stream, text } => {
                            if let Some(&id) = messages.get(&stream) {
                                this.state.push_delta(id, &text);
                                this.state.flush_frame();
                            }
                        }
                        TeamEvent::Usage { stream, usage } => {
                            if let Some(&id) = messages.get(&stream) {
                                this.state.set_usage(id, usage);
                            }
                        }
                        TeamEvent::Finish { stream } => {
                            if let Some(&id) = messages.get(&stream) {
                                this.state.end_message(id);
                            }
                        }
                        TeamEvent::Ping { from, to, body } => this.state.add_ping(&from, &to, &body),
                    }
                    cx.notify();
                });
            }
            if stopped {
                drop(task);
            } else {
                let failure = match task.await {
                    Ok(Ok(())) => None,
                    Ok(Err(err)) => Some(err),
                    Err(err) => Some(LlmError::Stream(err.to_string())),
                };
                _ = this.update_in(cx, |this, _, cx| {
                    match &failure {
                        Some(err) => this.state.fail_team_turn(err),
                        None => this.state.finish(),
                    }
                    cx.notify();
                });
            }
            _ = this.update_in(cx, |this, _, cx| {
                if let Some(store) = persist.store.as_deref() {
                    let _ = this.state.save_to_store(store);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Send-button click: mirror the composer into the draft and drive one
    /// backend turn into a new agent message. Stop-button click: stop the
    /// run and keep the draft (UI.md §5).
    fn on_send_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.is_running() {
            self.state.stop();
            cx.notify();
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        self.state.set_draft(text.clone());
        let model = self.model.clone();
        if let Some(team) = self.team.clone() {
            self.send_to_team(team, text, window, cx);
            return;
        }
        let Some((agent, mut req)) = self.state.send_request(&model) else {
            return;
        };
        req.system = self.system.clone();
        self.composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
        let backend = self.backend.clone();
        let persist = self.persist.clone();
        self.start_goal_summary(&text, cx);
        cx.spawn_in(window, async move |this, cx| {
            // The model runs on the tokio runtime and sends events as they
            // arrive; text shows up while the model is still writing.
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            let task = crate::runtime::spawn(async move { backend.turn_live(req, tx).await });
            let mut stopped = false;
            while let Some(event) = rx.recv().await {
                let running = this
                    .update_in(cx, |this, _, _| this.state.is_running())
                    .unwrap_or(false);
                if !running {
                    stopped = true;
                    break;
                }
                _ = this.update_in(cx, |this, _, cx| {
                    this.state.apply_event(agent, event);
                    cx.notify();
                });
            }
            if stopped {
                // Dropping the task cancels the request.
                drop(task);
            } else {
                let failure = match task.await {
                    Ok(Ok(())) => None,
                    Ok(Err(err)) => Some(err),
                    Err(err) => Some(LlmError::Stream(err.to_string())),
                };
                _ = this.update_in(cx, |this, _, cx| {
                    match &failure {
                        Some(err) => this.state.fail_turn(agent, err),
                        None if this.state.is_running() => this.state.finish(),
                        None => {}
                    }
                    cx.notify();
                });
            }
            _ = this.update_in(cx, |this, _, cx| {
                if let Some(catalog) = persist.catalog.as_deref() {
                    this.state
                        .price_turn(agent, catalog, &persist.provider_id, &persist.model_id);
                }
                if let Some(store) = persist.store.as_deref() {
                    let _ = this.state.save_to_store(store);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl<B> Render for ChatView<B>
where
    B: ModelBackend + 'static,
{
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_scroller(cx);
        // Ask blocks are for teammates, not for reading.
        let bodies: Vec<String> = self
            .state
            .messages()
            .iter()
            .map(|m| match m.role {
                ChatRole::Agent | ChatRole::Teammate => {
                    synapse_core::team_dialogue::visible_text(&m.body)
                }
                _ => m.body.clone(),
            })
            .collect();
        let notes: Vec<bool> = self
            .state
            .messages()
            .iter()
            .map(|m| m.role == ChatRole::System)
            .collect();
        let authors: Vec<String> = self
            .state
            .messages()
            .iter()
            .map(|m| m.author.clone())
            .collect();
        let avatars: Vec<Option<String>> = authors
            .iter()
            .map(|name| {
                if name == "You" {
                    return None;
                }
                let stored = self
                    .team
                    .as_ref()
                    .and_then(|t| t.members.iter().find(|m| &m.name == name))
                    .map(|m| m.avatar.clone())
                    .filter(|a| !a.is_empty());
                Some(stored.unwrap_or_else(|| {
                    crate::avatar::AvatarSpec::from_name(name).to_json()
                }))
            })
            .collect();
        let ping_rows: Vec<Vec<(String, String, String)>> = self
            .state
            .messages()
            .iter()
            .map(|m| {
                m.pings
                    .iter()
                    .map(|p| {
                        let avatar = self
                            .team
                            .as_ref()
                            .and_then(|t| t.members.iter().find(|x| x.name == p.to))
                            .map(|x| x.avatar.clone())
                            .filter(|a| !a.is_empty())
                            .unwrap_or_else(|| crate::avatar::AvatarSpec::from_name(&p.to).to_json());
                        (p.to.clone(), avatar, p.body.clone())
                    })
                    .collect()
            })
            .collect();
        let waiting: Vec<bool> = self
            .state
            .messages()
            .iter()
            .map(|m| {
                m.streaming
                    && synapse_core::team_dialogue::visible_text(&m.body).is_empty()
            })
            .collect();
        // Teammates' turns fold into one line; (message id, open) per row.
        let folded: Vec<Option<(usize, bool)>> = self
            .state
            .messages()
            .iter()
            .map(|m| {
                (m.role == ChatRole::Teammate).then(|| (m.id, self.opened.contains(&m.id)))
            })
            .collect();
        let streaming: Vec<bool> = self.state.messages().iter().map(|m| m.streaming).collect();
        let view = cx.entity().downgrade();
        let send_label = self.state.send_label().to_string();
        let running = self.state.is_running();
        let footers: Vec<Option<String>> = self
            .state
            .messages()
            .iter()
            .map(|m| self.state.cost_for(m.id).map(MessageCost::footer_label))
            .collect();
        let error = self.state.last_error().map(str::to_string);

        let theme = cx.theme().clone();
        let muted = theme.colors.muted_foreground;
        let (bubble_bg, radius_lg) = (theme.colors.muted, theme.radius_lg);

        let mut column = v_flex().size_full().gap_2();
        column = column.child(div().flex_1().min_h_0().child(MessageScroller::new(
            "chat-view-scroller",
            self.scroller.clone(),
            move |ix, _window: &mut Window, app: &mut App| {
                let body = bodies.get(ix).cloned().unwrap_or_default();
                let author = authors.get(ix).cloned().unwrap_or_default();
                let is_user = author == "You";
                let sender = author.clone();
                if notes.get(ix).copied().unwrap_or(false) {
                    return v_flex()
                        .w_full()
                        .px_4()
                        .py_1()
                        .child(
                            div()
                                .typeset(TextRole::Caption, app)
                                .text_color(muted)
                                .child(body),
                        )
                        .into_any_element();
                }
                let pinged = ping_rows.get(ix).cloned().unwrap_or_default();
                if let Some((id, open)) = folded.get(ix).copied().flatten() {
                    if body.is_empty() && !streaming.get(ix).copied().unwrap_or(false) {
                        return div().into_any_element();
                    }
                    let view = view.clone();
                    let note = crate::work_note::WorkNote {
                        index: ix,
                        avatar: avatars.get(ix).cloned().flatten().unwrap_or_default(),
                        author,
                        working: streaming.get(ix).copied().unwrap_or(false),
                        body,
                        open,
                    };
                    return v_flex()
                        .w_full()
                        .px_4()
                        .py_0p5()
                        .child(note.render(app, move |app| {
                            _ = view.update(app, |this, cx| this.toggle_note(id, cx));
                        }))
                        .into_any_element();
                }
                if body.is_empty() && pinged.is_empty() && !waiting.get(ix).copied().unwrap_or(false) {
                    return div().into_any_element();
                }
                let plain = body.clone();
                let text = TextView::markdown(format!("chat-msg-{ix}"), body)
                    .stream_fade(true)
                    .code_block_actions(|_block, _, _| div());
                let content = v_flex().gap_1().max_w(px(680.0));
                let content = if is_user {
                    content.items_end()
                } else {
                    content
                };
                let content = content
                    .child(match avatars.get(ix).cloned().flatten() {
                        Some(json) => h_flex()
                            .gap_2()
                            .items_center()
                            .child(crate::avatar_view::avatar_img(&json, &author, 24.0))
                            .child(
                                div()
                                    .typeset(TextRole::Label, app)
                                    .text_color(muted)
                                    .child(author),
                            )
                            .into_any_element(),
                        None => div()
                            .typeset(TextRole::Label, app)
                            .text_color(muted)
                            .child(author)
                            .into_any_element(),
                    })
                    .child(if is_user {
                        div()
                            .px_3()
                            .py_2()
                            .rounded(radius_lg)
                            .bg(bubble_bg)
                            .child(text)
                    } else if waiting.get(ix).copied().unwrap_or(false) {
                        div().child(crate::thinking::indicator(("thinking", ix), app))
                    } else {
                        div().child(text)
                    });
                let content = if pinged.is_empty() {
                    content
                } else {
                    content.child(crate::ping_row::ping_row(ix, &sender, pinged))
                };
                // The copy button shows while the pointer is over the message.
                let group = format!("chat-row-{ix}");
                let copy = Button::new(("chat-copy", ix))
                    .ghost()
                    .small()
                    .icon(crate::icons::COPY)
                    .tooltip("Copy")
                    .on_click(move |_, _, app: &mut App| {
                        app.write_to_clipboard(gpui_kit::ClipboardItem::new_string(plain.clone()));
                    });
                let mut foot = h_flex().gap_2().items_center();
                if let Some(footer) = footers.get(ix).cloned().flatten() {
                    foot = foot.child(
                        div().typeset(TextRole::Caption, app).figures().text_color(muted).child(footer),
                    );
                }
                let content = if waiting.get(ix).copied().unwrap_or(false) {
                    content
                } else {
                    content.child(
                        foot.child(div().invisible().group_hover(group.clone(), |s| s.visible()).child(copy)),
                    )
                };
                // Column + cross-axis alignment keeps a user turn on the right
                // however the row was measured.
                let row = v_flex().w_full().py_2().px_4().group(group);
                if is_user {
                    row.items_end().child(content).into_any_element()
                } else {
                    row.items_start().child(content).into_any_element()
                }
            },
        )));
        if let Some(message) = error {
            column = column.child(
                div()
                    .mx_4()
                    .px_3()
                    .py_2()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.colors.danger)
                    .typeset(TextRole::Label, cx)
                    .text_color(theme.colors.danger)
                    .child(format!("Error: {message}")),
            );
        }
        let can_send = running || !self.state.draft().trim().is_empty();
        let send = Button::new("chat-send-stop")
            .primary()
            .rounded(px(999.0))
            .icon(if running {
                crate::icons::STOP
            } else {
                crate::icons::SEND
            })
            .disabled(!can_send)
            .tooltip(send_label)
            .on_click(cx.listener(|this, _, window, cx| {
                this.on_send_click(window, cx);
            }));
        // One rounded card, like Claude Code: the text grows with the draft,
        // the round send/stop button sits bottom right, a quiet hint below.
        column.child(
            v_flex()
                .gap_1()
                .px_4()
                .child(
                    v_flex()
                        .gap_1()
                        .px_4()
                        .pt_3()
                        .pb_2()
                        .rounded(px(20.0))
                        .border_1()
                        .border_color(theme.colors.border)
                        .bg(theme.colors.background)
                        .shadow_sm()
                        .child(Textarea::new(&self.composer).appearance(false))
                        .child(h_flex().justify_end().child(send)),
                )
                .child(
                    div()
                        .px_4()
                        .typeset(TextRole::Caption, cx)
                        .text_color(muted)
                        .child("Enter to send · Shift+Enter for a new line"),
                ),
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
    async fn chat_view_turn_records_usage_prices_and_persists() {
        use synapse_llm::{MockLlm, TurnOutput, Usage};
        let catalog = synapse_llm::Catalog::embedded().expect("embedded catalog");
        let (provider_id, model_id) = {
            let mut found = None;
            for provider in catalog.providers() {
                for (id, model) in &provider.models {
                    if model.cost.input.unwrap_or(0.0) > 0.0 {
                        found = Some((provider.id.clone(), id.clone()));
                        break;
                    }
                }
            }
            found.expect("priced model")
        };
        let usage = Usage::new(120, 60);
        let backend = MockLlm::with_outputs(vec![TurnOutput {
            text: "persisted reply".to_string(),
            tool_calls: Vec::new(),
            usage,
        }]);
        let store = synapse_store::Store::open_in_memory().expect("store");
        store
            .create_project(&synapse_store::NewProject {
                id: "chat-p1".into(),
                name: "Chat".into(),
                goal: "persist".into(),
                folder: "/tmp/synapse-chat".into(),
                lead_agent_id: String::new(),
                template: "custom".into(),
            })
            .expect("project");
        store
            .create_conversation("chat-conv", "chat-p1", "chat", "Chat")
            .expect("conversation");

        let mut s = ChatState::new();
        s.set_conversation_id("chat-conv");
        s.set_draft("remember this");
        let (agent, req) = s.send_request("chat-model").expect("send");
        s.drive_turn(&backend, agent, req).await;
        assert_eq!(s.message(agent).unwrap().body, "persisted reply");
        // Usage from the stream is recorded and priced.
        assert_eq!(s.usage_for(agent), Some(usage));
        let cost = s
            .price_turn(agent, &catalog, &provider_id, &model_id)
            .expect("priced");
        assert_eq!(cost.input_tokens, 120);
        assert!(cost.cost_usd > 0.0);
        assert!(s.conversation_cost().header_label().contains("1 msg"));

        // Persisted rows carry the cost meta; a fresh state reloads them.
        assert_eq!(s.save_to_store(&store).expect("save"), 2);
        assert_eq!(s.save_to_store(&store).expect("save again"), 0);
        let mut reopened = ChatState::new();
        let total = reopened
            .load_from_store(&store, "chat-conv")
            .expect("reload");
        assert_eq!(reopened.item_count(), 2);
        assert_eq!(reopened.messages()[0].body, "remember this");
        assert_eq!(reopened.messages()[1].body, "persisted reply");
        assert_eq!(total.messages, 1);
        assert_eq!(total.input_tokens, 120);
        assert_eq!(reopened.conversation_cost(), total);
        assert!(reopened
            .conversation_cost()
            .header_label()
            .contains("1 msg"));
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
