//! Global hotkey + quick-ask window (M9 task 3, PRD §5.7, UI.md §2.6).
//!
//! Toolkit-free state machine so it runs headless in tests. The GPUI shell
//! wires it up: `global-hotkey` on the main thread (X11/macOS/Windows),
//! XDG GlobalShortcuts portal via `ashpd` on Wayland, otherwise a desktop
//! keybinding that runs `synapse --toggle` through single-instance IPC
//! (see `research/gpui.md`). The window is small and frameless, created on
//! hotkey press and destroyed on blur (ARCHITECTURE.md §5).

use synapse_llm::{AgentEvent, ModelBackend, TurnMessage, TurnRequest};

/// Default combo shown in Settings → Background.
pub const DEFAULT_QUICK_ASK_HOTKEY: &str = "Ctrl+Shift+Space";

/// Store keys (one source of truth lives here, not in `settings.rs`).
pub const QUICK_ASK_HOTKEY_KEY: &str = "quick_ask_hotkey";
pub const QUICK_ASK_LAST_PROJECT_KEY: &str = "quick_ask_last_project";

/// Parsed configurable hotkey combo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickAskHotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    /// Upper-cased key name (`"SPACE"`, `"K"`, `"F12"`, ...).
    pub key: String,
}

impl QuickAskHotkey {
    pub fn default_combo() -> Self {
        DEFAULT_QUICK_ASK_HOTKEY.parse().expect("default hotkey parses")
    }

    /// Canonical display string, e.g. `"Ctrl+Shift+Space"`.
    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.meta {
            parts.push("Meta".to_string());
        }
        parts.push(capitalise_key(&self.key));
        parts.join("+")
    }

    /// Validate against the keys `global-hotkey` can bind (single
    /// non-modifier key plus at least one modifier).
    pub fn validate(&self) -> Result<(), String> {
        if self.key.is_empty() {
            return Err("hotkey needs a key (e.g. Ctrl+Shift+Space)".to_string());
        }
        if !(self.ctrl || self.shift || self.alt || self.meta) {
            return Err("hotkey needs at least one modifier (Ctrl/Shift/Alt/Meta)".to_string());
        }
        Ok(())
    }
}

impl std::str::FromStr for QuickAskHotkey {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut meta = false;
        let mut key = String::new();
        for part in s.split('+') {
            let p = part.trim();
            match p.to_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                "meta" | "cmd" | "command" | "super" | "win" => meta = true,
                "" => {}
                _ => {
                    if !key.is_empty() {
                        return Err(format!("hotkey has two keys: '{key}' and '{p}'"));
                    }
                    key = p.to_uppercase();
                }
            }
        }
        let hotkey = Self {
            ctrl,
            shift,
            alt,
            meta,
            key,
        };
        hotkey.validate()?;
        Ok(hotkey)
    }
}

fn capitalise_key(key: &str) -> String {
    if key.eq_ignore_ascii_case("space") {
        return "Space".to_string();
    }
    if key.len() == 1 {
        return key.to_uppercase();
    }
    // F12, Enter, Escape...: first letter upper, rest lower.
    let mut chars = key.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
        }
    }
}

/// How the hotkey is captured on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyBackend {
    /// `global-hotkey` natively (macOS, Windows, X11).
    Native,
    /// Wayland: XDG GlobalShortcuts portal via `ashpd`.
    Portal,
    /// No capture available; desktop keybinding runs `synapse --toggle`.
    ExternalBinding,
}

/// Pick the capture backend. Wayland compositors ignore global grabs, so
/// Wayland always goes through the portal (or the external fallback when
/// the portal is unavailable).
pub fn hotkey_backend_for(is_wayland: bool, portal_available: bool) -> HotkeyBackend {
    if !is_wayland {
        HotkeyBackend::Native
    } else if portal_available {
        HotkeyBackend::Portal
    } else {
        HotkeyBackend::ExternalBinding
    }
}

/// Compact quick-ask window lifecycle: created on hotkey, destroyed on blur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickAskWindow {
    open: bool,
}

impl QuickAskWindow {
    pub fn closed() -> Self {
        Self { open: false }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Hotkey pressed: toggle the window (focus when closed, hide when open).
    /// Returns the new visibility.
    pub fn on_hotkey_pressed(&mut self) -> bool {
        self.open = !self.open;
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    /// Focus lost: destroy the window to free its surfaces.
    /// Returns true when a live window was destroyed by this call.
    pub fn on_blur(&mut self) -> bool {
        let was = self.open;
        self.open = false;
        was
    }
}

/// Effect of "Open in app": the main window opens this conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickAskOpenEffect {
    pub project_id: String,
    pub conversation_id: String,
}

/// Persisted quick-ask prefs (hotkey combo + last project lead).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickAskSettings {
    pub hotkey: String,
    pub last_project_id: String,
}

impl Default for QuickAskSettings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_QUICK_ASK_HOTKEY.to_string(),
            last_project_id: String::new(),
        }
    }
}

impl QuickAskSettings {
    pub fn load(store: &synapse_store::Store) -> Self {
        let mut out = Self::default();
        if let Ok(Some(raw)) = store.setting_get(QUICK_ASK_HOTKEY_KEY) {
            let s: String = serde_json::from_str(&raw).unwrap_or(raw);
            if s.parse::<QuickAskHotkey>().is_ok() {
                out.hotkey = s;
            }
        }
        if let Ok(Some(raw)) = store.setting_get(QUICK_ASK_LAST_PROJECT_KEY) {
            let s: String = serde_json::from_str(&raw).unwrap_or(raw);
            out.last_project_id = s;
        }
        out
    }

    pub fn save(&self, store: &synapse_store::Store) -> Result<(), String> {
        self.hotkey.parse::<QuickAskHotkey>().map_err(|e| e.to_string())?;
        store
            .setting_set(QUICK_ASK_HOTKEY_KEY, &serde_json::to_string(&self.hotkey).unwrap_or_default())
            .map_err(|e| e.to_string())?;
        store
            .setting_set(
                QUICK_ASK_LAST_PROJECT_KEY,
                &serde_json::to_string(&self.last_project_id).unwrap_or_default(),
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Compact streaming quick-ask state: one input, project selector, streamed
/// answer, "Open in app" (UI.md §2.6).
#[derive(Debug, Default)]
pub struct QuickAskState {
    project_id: String,
    query: String,
    answer: String,
    pending: String,
    running: bool,
    error: Option<String>,
    conversation_id: String,
    flushes: u64,
}

impl QuickAskState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn answer(&self) -> &str {
        &self.answer
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn last_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub fn frame_flushes(&self) -> u64 {
        self.flushes
    }

    /// Project the question goes to: the explicit selection, else the last
    /// project's lead, else `""` (window shows the selector empty state).
    pub fn resolve_project(selected: &str, last_project_id: &str) -> String {
        if !selected.trim().is_empty() {
            selected.trim().to_string()
        } else {
            last_project_id.trim().to_string()
        }
    }

    /// Start asking. `selected` is the project selector value (may be empty
    /// to mean "last project"). Returns false when there is nothing to send
    /// (empty query/project or already running).
    pub fn start(&mut self, query: &str, selected_project: &str, last_project_id: &str) -> bool {
        if self.running {
            return false;
        }
        let query = query.trim().to_string();
        let project = Self::resolve_project(selected_project, last_project_id);
        if query.is_empty() || project.is_empty() {
            return false;
        }
        self.project_id = project;
        self.query = query;
        self.answer.clear();
        self.pending.clear();
        self.error = None;
        self.running = true;
        self.conversation_id = format!("quick-{}-{}", self.project_id, self.query.len());
        true
    }

    /// Buffer a streamed delta; call [`Self::flush_frame`] per frame.
    pub fn push_delta(&mut self, delta: &str) {
        if self.running {
            self.pending.push_str(delta);
        }
    }

    /// Apply buffered deltas (one update per frame). Returns true when the
    /// visible answer changed.
    pub fn flush_frame(&mut self) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        self.answer.push_str(&self.pending);
        self.pending.clear();
        self.flushes += 1;
        true
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn finish(&mut self) {
        self.flush_frame();
        self.running = false;
    }

    pub fn stop(&mut self) {
        self.pending.clear();
        self.running = false;
    }

    pub fn fail(&mut self, message: impl Into<String>) {
        self.flush_frame();
        self.running = false;
        self.error = Some(message.into());
    }

    /// "Open in app": main window opens this quick-ask as a conversation.
    /// Returns `None` when nothing was asked yet.
    pub fn open_in_app(&self) -> Option<QuickAskOpenEffect> {
        if self.query.is_empty() || self.project_id.is_empty() {
            return None;
        }
        Some(QuickAskOpenEffect {
            project_id: self.project_id.clone(),
            conversation_id: self.conversation_id.clone(),
        })
    }

    /// Build the backend request for the current query.
    pub fn request(&self, model: &str) -> Option<TurnRequest> {
        if self.query.is_empty() {
            return None;
        }
        let mut req = TurnRequest::new(model);
        req.messages.push(TurnMessage {
            role: synapse_llm::Role::User,
            text: self.query.clone(),
        });
        Some(req)
    }

    /// Apply one backend stream event to the compact answer. No-op after stop.
    pub fn apply_event(&mut self, event: AgentEvent) {
        if !self.running {
            return;
        }
        match event {
            AgentEvent::TextDelta(delta) => {
                self.push_delta(&delta);
                self.flush_frame();
            }
            AgentEvent::ToolCall(_) | AgentEvent::Usage(_) => {}
            AgentEvent::Done { .. } => self.finish(),
        }
    }

    /// Drive one backend turn into the compact answer, streaming deltas and
    /// surfacing actionable provider errors (no key material).
    pub async fn drive_turn<B: ModelBackend>(&mut self, backend: &B, req: TurnRequest) {
        let events = match backend.turn_stream(req).await {
            Ok(events) => events,
            Err(err) => {
                self.fail(err.user_message());
                return;
            }
        };
        for event in events {
            if !self.running {
                break;
            }
            self.apply_event(event);
        }
    }
}

#[cfg(test)]
mod quick_ask_tests {
    use super::*;

    #[test]
    fn quick_ask_hotkey_parses_validates_and_roundtrips() {
        let hotkey: QuickAskHotkey = DEFAULT_QUICK_ASK_HOTKEY.parse().expect("default parses");
        assert!(hotkey.ctrl && hotkey.shift);
        assert_eq!(hotkey.display(), "Ctrl+Shift+Space");
        let custom: QuickAskHotkey = "Meta+K".parse().expect("meta combo");
        assert!(custom.meta);
        assert_eq!(custom.key, "K");
        assert!("Space".parse::<QuickAskHotkey>().is_err());
        assert!("Ctrl+Shift".parse::<QuickAskHotkey>().is_err());
        assert_eq!(
            QuickAskHotkey::default_combo().display(),
            DEFAULT_QUICK_ASK_HOTKEY
        );
    }

    #[test]
    fn quick_ask_hotkey_backend_prefers_portal_on_wayland() {
        assert_eq!(
            hotkey_backend_for(false, false),
            HotkeyBackend::Native
        );
        assert_eq!(hotkey_backend_for(true, true), HotkeyBackend::Portal);
        assert_eq!(
            hotkey_backend_for(true, false),
            HotkeyBackend::ExternalBinding
        );
    }

    #[test]
    fn quick_ask_window_toggles_on_hotkey_and_destroys_on_blur() {
        let mut w = QuickAskWindow::closed();
        assert!(!w.is_open());
        assert!(w.on_hotkey_pressed());
        assert!(w.is_open());
        // Second press hides it again.
        assert!(!w.on_hotkey_pressed());
        w.open();
        assert!(w.on_blur());
        assert!(!w.is_open());
        // Blur with no window is a no-op.
        assert!(!w.on_blur());
    }

    #[test]
    fn quick_ask_state_uses_selected_or_last_project_lead() {
        assert_eq!(
            QuickAskState::resolve_project("p-selected", "p-last"),
            "p-selected"
        );
        assert_eq!(QuickAskState::resolve_project("", "p-last"), "p-last");
        assert_eq!(QuickAskState::resolve_project("", ""), "");
        let mut s = QuickAskState::new();
        assert!(!s.start("  ", "p1", "p2"));
        assert!(!s.start("hello", "", ""));
        assert!(s.start("hello lead", "", "p-last"));
        assert_eq!(s.project_id(), "p-last");
        assert!(s.is_running());
        assert!(!s.start("again", "p1", "p1"));
    }

    #[test]
    fn quick_ask_answer_streams_compact_and_opens_in_app() {
        let mut s = QuickAskState::new();
        assert!(s.start("summarise", "p1", "p2"));
        assert_eq!(s.project_id(), "p1");
        for delta in ["Hello", " ", "world"] {
            s.push_delta(delta);
        }
        assert_eq!(s.answer(), "");
        assert!(s.pending_len() > 0);
        assert!(s.flush_frame());
        assert_eq!(s.answer(), "Hello world");
        assert!(!s.flush_frame());
        s.finish();
        assert!(!s.is_running());
        assert_eq!(s.frame_flushes(), 1);
        let effect = s.open_in_app().expect("open effect");
        assert_eq!(effect.project_id, "p1");
        assert!(!effect.conversation_id.is_empty());
        // Nothing asked: no open effect.
        assert!(QuickAskState::new().open_in_app().is_none());
    }

    #[test]
    fn quick_ask_settings_roundtrip_persists_hotkey_and_last_project() {
        let store = synapse_store::Store::open_in_memory().expect("store");
        let loaded = QuickAskSettings::load(&store);
        assert_eq!(loaded.hotkey, DEFAULT_QUICK_ASK_HOTKEY);
        let settings = QuickAskSettings {
            hotkey: "Alt+Space".to_string(),
            last_project_id: "p-last".to_string(),
        };
        settings.save(&store).expect("save");
        let again = QuickAskSettings::load(&store);
        assert_eq!(again, settings);
        // Invalid hotkey is rejected on save and ignored on load.
        let bad = QuickAskSettings {
            hotkey: "Space".to_string(),
            last_project_id: String::new(),
        };
        assert!(bad.save(&store).is_err());
        store.setting_set(QUICK_ASK_HOTKEY_KEY, "\"Space\"").unwrap();
        assert_eq!(
            QuickAskSettings::load(&store).hotkey,
            DEFAULT_QUICK_ASK_HOTKEY
        );
    }

    #[tokio::test]
    async fn quick_ask_drive_turn_streams_answer_from_lead() {
        use synapse_llm::MockLlm;
        let backend = MockLlm::with_text("lead says hi");
        let mut s = QuickAskState::new();
        assert!(s.start("hello", "", "p-last"));
        let req = s.request("test-model").expect("request");
        assert!(req.messages.iter().any(|m| m.text == "hello"));
        s.drive_turn(&backend, req).await;
        assert!(!s.is_running());
        assert_eq!(s.answer(), "lead says hi");
        assert!(s.last_error().is_none());
        assert!(s.open_in_app().is_some());
    }

    #[tokio::test]
    async fn quick_ask_drive_turn_surfaces_missing_key_error() {
        use synapse_llm::{LlmError, MockLlm};
        let backend = MockLlm::with_error(LlmError::Auth("no key".into()));
        let mut s = QuickAskState::new();
        assert!(s.start("hello", "p1", ""));
        let req = s.request("test-model").expect("request");
        s.drive_turn(&backend, req).await;
        assert!(!s.is_running());
        let shown = s.last_error().expect("error shown");
        assert!(shown.contains("Settings"), "{shown}");
    }
}
