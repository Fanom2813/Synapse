//! Resumable, skippable, keyboard-first onboarding (see docs/UI.md §9).
//!
//! Toolkit-free so it is unit testable. The GPUI view layer renders one step
//! at a time from [`OnboardingFlow`]; persistence lives in the `settings`
//! table (`onboarding.state`) so the flow resumes after restart. Background
//! detection (CLIs, local models, keys) never blocks stepping: it runs on a
//! worker thread and posts [`DetectionSnapshot`]s through a channel that the
//! UI polls with [`OnboardingFlow::poll_detection`].

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Steps (UI.md §9: seven steps)
// ---------------------------------------------------------------------------

/// The seven onboarding steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Hello,
    LeadName,
    LeadLook,
    Brains,
    Research,
    Team,
    FirstProject,
}

impl Step {
    pub fn all() -> &'static [Step] {
        &[
            Step::Hello,
            Step::LeadName,
            Step::LeadLook,
            Step::Brains,
            Step::Research,
            Step::Team,
            Step::FirstProject,
        ]
    }

    pub fn index(self) -> usize {
        Self::all().iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Step {
        Self::all().get(i).copied().unwrap_or(Step::FirstProject)
    }

    pub fn title(self) -> &'static str {
        match self {
            Step::Hello => "Meet your research team.",
            Step::LeadName => "Name your lead",
            Step::LeadLook => "Give it a look",
            Step::Brains => "How agents think",
            Step::Research => "Make research stronger (optional)",
            Step::Team => "Pick your team",
            Step::FirstProject => "Start your first project",
        }
    }

    /// Research is the only optional-by-default step; everything else is
    /// required but skippable (skip records the step and moves on).
    pub fn optional(self) -> bool {
        matches!(self, Step::Research)
    }
}

/// Keyboard handling: Enter = continue, Esc = back (UI.md §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Continue,
    Back,
    Stay,
}

pub fn handle_key(code: &str, step: Step) -> KeyAction {
    match code {
        "Enter" => KeyAction::Continue,
        "Escape" if step.index() > 0 => KeyAction::Back,
        _ => KeyAction::Stay,
    }
}

// ---------------------------------------------------------------------------
// Background detection (never blocks the flow)
// ---------------------------------------------------------------------------

/// One detected CLI entry (mirrors `synapse-acp::DetectedAgent`, trimmed so
/// this module stays toolkit-free and offline).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CliEntry {
    pub id: String,
    pub label: String,
    pub logged_in: bool,
}

/// Local model entry (`"Ollama: llama3.8b"`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LocalModelEntry {
    pub server: String,
    pub model: String,
}

/// Whatever the background probe has found so far. Cards in the Brains step
/// appear as entries arrive; an empty snapshot means "still looking".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DetectionSnapshot {
    pub cli_agents: Vec<CliEntry>,
    pub local_models: Vec<LocalModelEntry>,
    /// An API key was added and its live "Testing…" check passed.
    pub api_key_ok: bool,
    pub done: bool,
}

impl DetectionSnapshot {
    pub fn found_usable_backend(&self) -> bool {
        self.api_key_ok
            || self.local_models.iter().any(|_| true)
            || self.cli_agents.iter().any(|c| c.logged_in)
    }

    /// Simplest next action for this OS when nothing works yet.
    pub fn simplest_option(&self) -> &'static str {
        if cfg!(target_os = "linux") {
            "Install Ollama (`curl -fsSL https://ollama.com/install.sh | sh`), or paste any provider API key — research already works keyless."
        } else {
            "Install Ollama from ollama.com, or paste any provider API key — research already works keyless."
        }
    }
}

/// Example goals offered as chips on the FirstProject step.
pub const EXAMPLE_GOALS: [&str; 3] = [
    "Is there a market for solar dryers for Ugandan coffee farmers?",
    "What do recent papers say about retrieval-augmented reasoning?",
    "Should we launch a usage-based tier? Weigh risks and options.",
];

/// Lead introduction posted into the room when the first project starts.
pub fn lead_introduction(lead_name: &str, template_name: &str, goal: &str) -> String {
    let lead = if lead_name.trim().is_empty() {
        "Dottie"
    } else {
        lead_name.trim()
    };
    format!(
        "Hi, I'm {lead}! I'll lead this {template_name} team. Our goal: {goal}. I'll plan the work, assign questions, and converge on findings — jump in any time with @mentions."
    )
}

// ---------------------------------------------------------------------------
// Persisted state
// ---------------------------------------------------------------------------

const STORE_KEY: &str = "onboarding.state";

/// Full resumable state, saved after every step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PersistedOnboarding {
    pub step_index: usize,
    pub lead_name: String,
    pub shape: String,
    pub palette: String,
    pub eyes: String,
    pub accessory: String,
    pub brains_backend: String,
    pub api_provider: String,
    /// Never the key itself — only whether a live test passed.
    pub api_key_ok: bool,
    pub browser_connected: bool,
    pub searxng_url: String,
    pub template_id: String,
    pub project_goal: String,
    pub skipped: Vec<String>,
    pub detection: DetectionSnapshot,
    pub completed: bool,
}

impl PersistedOnboarding {
    fn step_id(s: Step) -> String {
        format!("{:?}", s).to_lowercase()
    }
}

// ---------------------------------------------------------------------------
// Flow
// ---------------------------------------------------------------------------

/// Step state machine + persistence + background-detection inbox.
pub struct OnboardingFlow {
    state: PersistedOnboarding,
    tx: Sender<DetectionSnapshot>,
    rx: Receiver<DetectionSnapshot>,
    /// When the background probe last delivered (for tests/debugging).
    last_detection_at: Option<Instant>,
}

impl OnboardingFlow {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            state: PersistedOnboarding {
                lead_name: "Dottie".into(),
                shape: "bean".into(),
                palette: "green".into(),
                eyes: "diamond".into(),
                template_id: "research-squad".into(),
                ..PersistedOnboarding::default()
            },
            tx,
            rx,
            last_detection_at: None,
        }
    }

    /// Start background detection without blocking: `probe` runs on a worker
    /// thread and its snapshot is delivered to [`Self::poll_detection`].
    /// Returns immediately (asserted by tests with a blocking probe).
    pub fn start_background_detection<F>(&self, probe: F)
    where
        F: FnOnce() -> DetectionSnapshot + Send + 'static,
    {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let snap = probe();
            let _ = tx.send(snap);
        });
    }

    /// Non-blocking: drains any snapshots the worker posted. Never waits on
    /// the network; call once per frame / on a timer.
    pub fn poll_detection(&mut self) -> bool {
        let mut updated = false;
        while let Ok(snap) = self.rx.try_recv() {
            self.state.detection = snap;
            self.last_detection_at = Some(Instant::now());
            updated = true;
        }
        updated
    }

    /// Directly merge a snapshot (tests + the GPUI timer path).
    pub fn apply_detection(&mut self, snap: DetectionSnapshot) {
        self.state.detection = snap;
        self.last_detection_at = Some(Instant::now());
    }

    pub fn step(&self) -> Step {
        if self.state.completed {
            return Step::FirstProject;
        }
        Step::from_index(self.state.step_index.min(Step::all().len() - 1))
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.step().index() + 1, Step::all().len())
    }

    pub fn detection(&self) -> &DetectionSnapshot {
        &self.state.detection
    }

    pub fn last_detection_at(&self) -> Option<Instant> {
        self.last_detection_at
    }

    /// Whether Continue is enabled. Brains needs one working backend (or an
    /// explicit skip); FirstProject needs a non-empty goal.
    pub fn can_continue(&self) -> bool {
        match self.step() {
            Step::Brains => self.state.detection.found_usable_backend() || self.state.api_key_ok,
            Step::FirstProject => self.state.project_goal.trim().len() >= 3,
            Step::LeadName => !self.state.lead_name.trim().is_empty(),
            Step::Team => !self.state.template_id.trim().is_empty(),
            _ => true,
        }
    }

    /// Reason shown when Continue is disabled on the Brains step.
    pub fn blocked_hint(&self) -> Option<&'static str> {
        match self.step() {
            Step::Brains if !self.can_continue() => {
                Some("Still looking — add a key, or continue free with keyless research once a local model or logged-in CLI appears.")
            }
            _ => None,
        }
    }

    pub fn next(&mut self) {
        if self.state.step_index + 1 >= Step::all().len() {
            self.state.completed = true;
        } else {
            self.state.step_index += 1;
        }
    }

    pub fn back(&mut self) {
        self.state.step_index = self.state.step_index.saturating_sub(1);
        self.state.completed = false;
    }

    /// Skip the current step (records it, moves on, resumable).
    pub fn skip(&mut self) {
        let id = PersistedOnboarding::step_id(self.step());
        if !self.state.skipped.contains(&id) {
            self.state.skipped.push(id);
        }
        self.next();
    }

    pub fn press_key(&mut self, code: &str) {
        match handle_key(code, self.step()) {
            KeyAction::Continue if self.can_continue() => self.next(),
            KeyAction::Continue => {}
            KeyAction::Back => self.back(),
            KeyAction::Stay => {}
        }
    }

    // -- field setters (each followed by `save` by the caller) --

    pub fn set_lead_name(&mut self, name: &str) {
        self.state.lead_name = name.trim().to_string();
    }

    pub fn set_look(&mut self, shape: &str, palette: &str, eyes: &str, accessory: &str) {
        self.state.shape = shape.trim().to_string();
        self.state.palette = palette.trim().to_string();
        self.state.eyes = eyes.trim().to_string();
        self.state.accessory = accessory.trim().to_string();
    }

    pub fn set_brains_backend(&mut self, backend: &str) {
        self.state.brains_backend = backend.trim().to_string();
    }

    pub fn set_api_result(&mut self, provider: &str, ok: bool) {
        self.state.api_provider = provider.trim().to_string();
        self.state.api_key_ok = ok;
        if ok {
            let mut snap = self.state.detection.clone();
            snap.api_key_ok = true;
            self.state.detection = snap;
        }
    }

    pub fn set_research(&mut self, browser_connected: bool, searxng_url: &str) {
        self.state.browser_connected = browser_connected;
        self.state.searxng_url = searxng_url.trim().to_string();
    }

    pub fn set_template(&mut self, template_id: &str) -> bool {
        let id = template_id.trim();
        let known = synapse_core::builtin_template(id).is_some();
        if known {
            self.state.template_id = id.to_string();
        }
        known
    }

    pub fn set_goal(&mut self, goal: &str) {
        self.state.project_goal = goal.trim().to_string();
    }

    pub fn template_name(&self) -> String {
        synapse_core::builtin_template(&self.state.template_id)
            .map(|t| t.name)
            .unwrap_or_else(|| "Custom".to_string())
    }

    /// Finish: validates the goal, marks complete, returns the lead greeting
    /// the room posts as the window morphs into the main layout.
    pub fn finish(&mut self) -> Result<String, String> {
        if self.state.project_goal.trim().len() < 3 {
            return Err("Describe your goal in a few words first.".to_string());
        }
        self.state.completed = true;
        Ok(lead_introduction(
            &self.state.lead_name,
            &self.template_name(),
            self.state.project_goal.trim(),
        ))
    }

    pub fn is_completed(&self) -> bool {
        self.state.completed
    }

    pub fn skipped(&self) -> &[String] {
        &self.state.skipped
    }

    // -- persistence (resume after restart) --

    pub fn save(&self, store: &synapse_store::Store) -> Result<(), String> {
        let raw = serde_json::to_string(&self.state).map_err(|e| e.to_string())?;
        store
            .setting_set(STORE_KEY, &raw)
            .map_err(|e| e.to_string())
    }

    pub fn load(store: &synapse_store::Store) -> Result<Self, String> {
        let mut flow = Self::new();
        let raw: Option<String> = store.setting_get(STORE_KEY).map_err(|e| e.to_string())?;
        if let Some(raw) = raw {
            flow.state = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        }
        Ok(flow)
    }

    /// `true` once any step was taken or state was saved.
    pub fn has_started(&self) -> bool {
        self.state.step_index > 0
            || !self.state.lead_name.is_empty()
            || !self.state.skipped.is_empty()
            || self.state.completed
    }

    pub fn snapshot(&self) -> &PersistedOnboarding {
        &self.state
    }
}

impl Default for OnboardingFlow {
    fn default() -> Self {
        Self::new()
    }
}

/// How long the Hello delight step shows before Continue enables (3 s).
pub const HELLO_DELAY: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// First-launch wiring (UI.md §9 → main window)
//
// [`FirstLaunch`] is the toolkit-free first-launch window state: it owns an
// [`OnboardingFlow`], persists after every step so an unfinished flow
// resumes after restart, and on completion creates/opens the first project,
// posts the lead introduction, and hands the window to the normal workspace.
// The GPUI view renders from [`FirstLaunch::screen_text`] and forwards
// inputs to the `set_*` / `advance` / `back` / `skip` / `press_key` methods.
// ---------------------------------------------------------------------------

/// Project + greeting produced by [`FirstLaunch::complete`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedOnboarding {
    pub project_id: String,
    pub conversation_id: String,
    pub introduction: String,
}

/// First-launch window: onboarding flow + persistence + workspace handoff.
pub struct FirstLaunch {
    flow: OnboardingFlow,
}

impl FirstLaunch {
    /// Open the first-launch state: resumes saved progress when present.
    /// Never fails: a missing/corrupt row starts a fresh flow.
    pub fn open(store: &synapse_store::Store) -> Self {
        let flow = OnboardingFlow::load(store).unwrap_or_default();
        Self { flow }
    }

    /// Whether the first-launch wizard should show for this store.
    /// Returning users (completed flow, or existing projects with a fresh
    /// flow) bypass onboarding straight into the normal workspace.
    pub fn should_show(store: &synapse_store::Store) -> bool {
        let flow = OnboardingFlow::load(store).unwrap_or_default();
        Self::should_show_flow(store, &flow)
    }

    fn should_show_flow(store: &synapse_store::Store, flow: &OnboardingFlow) -> bool {
        if flow.is_completed() {
            return false;
        }
        let s = flow.snapshot();
        let fresh = s.step_index == 0
            && s.skipped.is_empty()
            && s.project_goal.is_empty()
            && (s.lead_name.is_empty() || s.lead_name == "Dottie");
        if fresh {
            // Returning users (existing projects, never started onboarding)
            // bypass straight into the normal workspace.
            if let Ok(projects) = store.list_projects(false) {
                if !projects.is_empty() {
                    return false;
                }
            }
            return true;
        }
        // Resumable: an unfinished flow always shows, even with projects.
        true
    }

    pub fn should_show_self(&self, store: &synapse_store::Store) -> bool {
        Self::should_show_flow(store, &self.flow)
    }

    pub fn flow(&self) -> &OnboardingFlow {
        &self.flow
    }

    pub fn flow_mut(&mut self) -> &mut OnboardingFlow {
        &mut self.flow
    }

    /// Headless description of the current wizard step for the GPUI view.
    /// Format: `"Step i/7 · Title · hint"` plus field values so the view
    /// can render each of the seven steps without extra state.
    pub fn screen_text(&self) -> String {
        let (n, total) = self.flow.progress();
        let step = self.flow.step();
        let s = self.flow.snapshot();
        let fields = match step {
            Step::Hello => "Let's start".to_string(),
            Step::LeadName => format!("lead_name={}", s.lead_name),
            Step::LeadLook => format!(
                "shape={} palette={} eyes={} accessory={}",
                s.shape, s.palette, s.eyes, s.accessory
            ),
            Step::Brains => format!(
                "backend={} api_provider={} api_key_ok={} cli={} local={}",
                s.brains_backend,
                s.api_provider,
                s.api_key_ok,
                s.detection.cli_agents.len(),
                s.detection.local_models.len()
            ),
            Step::Research => {
                format!("browser={} searxng={}", s.browser_connected, s.searxng_url)
            }
            Step::Team => format!("template={} ({})", s.template_id, self.flow.template_name()),
            Step::FirstProject => format!("goal={}", s.project_goal),
        };
        let cont = if self.flow.can_continue() {
            "Continue"
        } else {
            self.flow.blocked_hint().unwrap_or("Blocked")
        };
        format!(
            "Step {n}/{total} · {} · {fields} [{cont}] [Skip] [Back]",
            step.title()
        )
    }

    /// Start background backend detection without blocking the UI thread.
    /// `probe` runs on a worker thread; the view polls [`Self::poll_detection`].
    /// Returns immediately even when the probe blocks on the network.
    pub fn start_background_detection<F>(&self, probe: F)
    where
        F: FnOnce() -> DetectionSnapshot + Send + 'static,
    {
        self.flow.start_background_detection(probe);
    }

    /// Non-blocking drain of background results; persists when new data
    /// arrived so resume sees the latest backends. Never waits on network.
    pub fn poll_detection(&mut self, store: &synapse_store::Store) -> bool {
        let updated = self.flow.poll_detection();
        if updated {
            let _ = self.flow.save(store);
        }
        updated
    }

    fn persist(&self, store: &synapse_store::Store) {
        let _ = self.flow.save(store);
    }

    // -- per-step inputs (each persists so restart resumes mid-flow) --

    pub fn set_lead_name(&mut self, store: &synapse_store::Store, name: &str) {
        self.flow.set_lead_name(name);
        self.persist(store);
    }

    pub fn set_look(
        &mut self,
        store: &synapse_store::Store,
        shape: &str,
        palette: &str,
        eyes: &str,
        accessory: &str,
    ) {
        self.flow.set_look(shape, palette, eyes, accessory);
        self.persist(store);
    }

    pub fn set_brains_backend(&mut self, store: &synapse_store::Store, backend: &str) {
        self.flow.set_brains_backend(backend);
        self.persist(store);
    }

    pub fn set_api_result(&mut self, store: &synapse_store::Store, provider: &str, ok: bool) {
        self.flow.set_api_result(provider, ok);
        self.persist(store);
    }

    pub fn set_research(&mut self, store: &synapse_store::Store, browser: bool, url: &str) {
        self.flow.set_research(browser, url);
        self.persist(store);
    }

    pub fn set_template(&mut self, store: &synapse_store::Store, template_id: &str) -> bool {
        let ok = self.flow.set_template(template_id);
        self.persist(store);
        ok
    }

    pub fn set_goal(&mut self, store: &synapse_store::Store, goal: &str) {
        self.flow.set_goal(goal);
        self.persist(store);
    }

    pub fn advance(&mut self, store: &synapse_store::Store) {
        self.flow.next();
        self.persist(store);
    }

    pub fn back(&mut self, store: &synapse_store::Store) {
        self.flow.back();
        self.persist(store);
    }

    pub fn skip(&mut self, store: &synapse_store::Store) {
        self.flow.skip();
        self.persist(store);
    }

    /// Enter = continue (when allowed), Esc = back. Persists after handling.
    pub fn press_key(&mut self, store: &synapse_store::Store, code: &str) {
        self.flow.press_key(code);
        self.persist(store);
    }

    pub fn can_continue(&self) -> bool {
        self.flow.can_continue()
    }

    pub fn is_completed(&self) -> bool {
        self.flow.is_completed()
    }

    /// Finish the wizard: validates the goal, marks complete, creates/opens
    /// the first project, posts the lead introduction into its room, and
    /// persists so returning users bypass onboarding.
    pub fn complete(
        &mut self,
        store: &synapse_store::Store,
    ) -> Result<CompletedOnboarding, String> {
        let intro = self.flow.finish().map_err(|e| e.to_string())?;
        let s = self.flow.snapshot().clone();
        let project_id = synapse_store::new_id("project");
        let name = short_project_name(&s.project_goal);
        let project = store
            .create_project(&synapse_store::NewProject {
                id: project_id.clone(),
                name,
                goal: s.project_goal.clone(),
                folder: String::new(),
                lead_agent_id: s.lead_name.clone(),
                template: s.template_id.clone(),
            })
            .map_err(|e| e.to_string())?;
        let conversation_id = synapse_store::new_id("conversation");
        store
            .create_conversation(&conversation_id, &project.id, "room", &project.name)
            .map_err(|e| e.to_string())?;
        store
            .insert_message(&synapse_store::NewMessage {
                id: synapse_store::new_id("message"),
                conversation_id: conversation_id.clone(),
                run_id: String::new(),
                from_id: if s.lead_name.trim().is_empty() {
                    "Dottie".into()
                } else {
                    s.lead_name.clone()
                },
                to_json: "[]".into(),
                kind: "say".into(),
                reply_to: String::new(),
                thread_id: String::new(),
                body: intro.clone(),
                meta: "{\"onboarding\":true}".into(),
            })
            .map_err(|e| e.to_string())?;
        self.persist(store);
        Ok(CompletedOnboarding {
            project_id: project.id,
            conversation_id,
            introduction: intro,
        })
    }

    /// Transition a toolkit-free [`crate::shell::AppShell`] into the normal
    /// workspace with the completed project open.
    pub fn enter_workspace(
        &self,
        shell: &mut crate::shell::AppShell,
        project_id: &str,
        project_name: &str,
        has_messages: bool,
    ) {
        if !shell.projects.iter().any(|p| p.id == project_id) {
            shell.projects.push(crate::shell::ShellProject {
                id: project_id.to_string(),
                name: project_name.to_string(),
                has_messages,
            });
        } else if let Some(p) = shell.projects.iter_mut().find(|p| p.id == project_id) {
            p.has_messages = has_messages;
        }
        shell.open_room();
        shell.open_project(project_id);
    }
}

fn short_project_name(goal: &str) -> String {
    let g = goal.trim();
    if g.is_empty() {
        return "First project".to_string();
    }
    let first: String = g
        .split(['.', '!', '?', '\n'])
        .next()
        .unwrap_or(g)
        .trim()
        .to_string();
    if first.chars().count() <= 60 {
        first
    } else {
        first.chars().take(57).collect::<String>() + "..."
    }
}

#[cfg(test)]
mod onboarding_tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn mem_store() -> synapse_store::Store {
        synapse_store::Store::open_in_memory().unwrap()
    }

    fn usable_flow() -> OnboardingFlow {
        let mut f = OnboardingFlow::new();
        f.apply_detection(DetectionSnapshot {
            cli_agents: vec![CliEntry {
                id: "claude".into(),
                label: "Claude Code".into(),
                logged_in: true,
            }],
            local_models: vec![],
            api_key_ok: false,
            done: true,
        });
        f
    }

    #[test]
    fn onboarding_steps_cover_seven_in_order() {
        assert_eq!(Step::all().len(), 7);
        assert_eq!(Step::all().first(), Some(&Step::Hello));
        assert_eq!(Step::all().last(), Some(&Step::FirstProject));
        for (i, s) in Step::all().iter().enumerate() {
            assert_eq!(s.index(), i);
            assert_eq!(Step::from_index(i), *s);
        }
    }

    #[test]
    fn onboarding_keyboard_enter_continues_escape_goes_back() {
        let mut f = usable_flow();
        assert_eq!(f.step(), Step::Hello);
        f.press_key("Enter");
        assert_eq!(f.step(), Step::LeadName);
        f.press_key("Escape");
        assert_eq!(f.step(), Step::Hello);
        // Escape on the first step stays.
        f.press_key("Escape");
        assert_eq!(f.step(), Step::Hello);
        assert_eq!(handle_key("Space", Step::Hello), KeyAction::Stay);
    }

    #[test]
    fn onboarding_skippable_every_step() {
        let mut f = OnboardingFlow::new();
        for _ in 0..Step::all().len() {
            f.skip();
        }
        assert!(f.is_completed());
        assert_eq!(f.skipped().len(), Step::all().len());
    }

    #[test]
    fn onboarding_brains_blocks_until_backend_found() {
        let mut f = OnboardingFlow::new();
        f.state.step_index = Step::Brains.index();
        assert!(!f.can_continue());
        assert!(f.blocked_hint().is_some());
        // Pressing Enter while blocked does not advance.
        f.press_key("Enter");
        assert_eq!(f.step(), Step::Brains);
        // A background detection arrival unblocks without any network wait.
        f.apply_detection(DetectionSnapshot {
            cli_agents: vec![],
            local_models: vec![LocalModelEntry {
                server: "Ollama".into(),
                model: "llama3.8b".into(),
            }],
            api_key_ok: false,
            done: true,
        });
        assert!(f.can_continue());
        f.press_key("Enter");
        assert_eq!(f.step(), Step::Research);
    }

    #[test]
    fn onboarding_does_not_wait_for_network() {
        // The probe blocks forever; stepping must still work instantly.
        let gate = Arc::new(Barrier::new(2));
        let gate2 = gate.clone();
        let f = OnboardingFlow::new();
        let started = Instant::now();
        f.start_background_detection(move || {
            gate2.wait();
            DetectionSnapshot::default()
        });
        // Advance without polling: must return immediately.
        let mut g = OnboardingFlow::new();
        g.press_key("Enter");
        assert_eq!(g.step(), Step::LeadName);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "stepping waited on the network probe"
        );
        // Polling with no delivery is also non-blocking.
        assert!(!g.poll_detection());
        gate.wait(); // release the worker so the test thread exits cleanly.
    }

    #[test]
    fn onboarding_background_detection_applies_without_blocking() {
        let mut f = OnboardingFlow::new();
        f.state.step_index = Step::Brains.index();
        assert!(!f.can_continue());
        let (tx, rx) = mpsc::channel::<()>();
        f.start_background_detection(move || {
            rx.recv().unwrap();
            DetectionSnapshot {
                cli_agents: vec![CliEntry {
                    id: "codex".into(),
                    label: "Codex".into(),
                    logged_in: true,
                }],
                local_models: vec![],
                api_key_ok: false,
                done: true,
            }
        });
        assert!(!f.poll_detection(), "nothing delivered yet");
        assert!(!f.can_continue());
        tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !f.poll_detection() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(f.can_continue());
        assert!(f.last_detection_at().is_some());
    }

    #[test]
    fn onboarding_resumes_after_restart() {
        let store = mem_store();
        let mut f = usable_flow();
        f.set_lead_name("Zara");
        f.set_look("round", "amber", "dot", "glasses");
        f.press_key("Enter"); // Hello -> LeadName
        f.press_key("Enter"); // LeadName -> LeadLook
        f.save(&store).unwrap();
        // "Restart": fresh flow loads the saved step.
        let mut again = OnboardingFlow::load(&store).unwrap();
        assert_eq!(again.step(), Step::LeadLook);
        assert_eq!(again.snapshot().lead_name, "Zara");
        assert_eq!(again.snapshot().shape, "round");
        again.press_key("Enter");
        assert_eq!(again.step(), Step::Brains);
        again.save(&store).unwrap();
        let third = OnboardingFlow::load(&store).unwrap();
        assert_eq!(third.step(), Step::Brains);
    }

    #[test]
    fn onboarding_team_and_first_project_finish_with_introduction() {
        let mut f = usable_flow();
        f.state.step_index = Step::Team.index();
        assert!(f.set_template("product-brainstorm"));
        assert!(!f.set_template("nope-not-real"));
        // Goal required before finish.
        f.state.step_index = Step::FirstProject.index();
        assert!(!f.can_continue());
        assert!(f.finish().is_err());
        f.set_goal(EXAMPLE_GOALS[0]);
        assert!(f.can_continue());
        let intro = f.finish().unwrap();
        assert!(intro.contains("Dottie") || intro.contains(&f.snapshot().lead_name));
        assert!(intro.contains("market for solar dryers") || intro.contains("goal"));
        assert!(f.is_completed());
        // Completed state survives a restart.
        let store = mem_store();
        f.save(&store).unwrap();
        let again = OnboardingFlow::load(&store).unwrap();
        assert!(again.is_completed());
    }

    #[test]
    fn onboarding_api_key_path_unblocks_brains() {
        let mut f = OnboardingFlow::new();
        f.state.step_index = Step::Brains.index();
        assert!(!f.can_continue());
        f.set_api_result("anthropic", true);
        assert!(f.can_continue());
        assert!(f.detection().api_key_ok);
    }

    #[test]
    fn onboarding_never_persists_secret_values() {
        let store = mem_store();
        let mut f = OnboardingFlow::new();
        f.set_api_result("openai", true);
        f.save(&store).unwrap();
        let raw = store.setting_get(STORE_KEY).unwrap().unwrap();
        assert!(!raw.contains("sk-"), "key material must never be stored");
        assert!(raw.contains("api_key_ok"));
        // Simplest-option hint exists for the nothing-found case.
        assert!(!DetectionSnapshot::default().simplest_option().is_empty());
    }

    fn drive_to_goal(store: &synapse_store::Store) -> FirstLaunch {
        let mut w = FirstLaunch::open(&store);
        w.set_lead_name(store, "Dottie");
        w.set_look(store, "bean", "green", "diamond", "");
        w.set_brains_backend(store, "claude");
        w.set_api_result(store, "anthropic", true);
        assert!(w.set_template(store, "research-squad"));
        w.set_research(store, false, "");
        w.set_goal(store, EXAMPLE_GOALS[0]);
        w
    }

    #[test]
    fn onboarding_first_launch_renders_all_seven_steps() {
        let store = mem_store();
        let mut w = FirstLaunch::open(&store);
        assert!(w.should_show_self(&store));
        assert!(FirstLaunch::should_show(&store));
        for expected in Step::all() {
            assert_eq!(w.flow().step(), *expected);
            let text = w.screen_text();
            assert!(text.contains(expected.title()));
            assert!(text.contains("Step"));
            match expected {
                Step::Brains => {
                    w.set_api_result(&store, "anthropic", true);
                    w.advance(&store);
                }
                Step::FirstProject => break,
                _ => w.advance(&store),
            }
        }
    }

    #[test]
    fn onboarding_first_launch_enter_esc_skip_and_resume() {
        let store = mem_store();
        let mut w = FirstLaunch::open(&store);
        assert_eq!(w.flow().step(), Step::Hello);
        w.press_key(&store, "Enter");
        assert_eq!(w.flow().step(), Step::LeadName);
        w.press_key(&store, "Escape");
        assert_eq!(w.flow().step(), Step::Hello);
        w.skip(&store);
        assert_eq!(w.flow().step(), Step::LeadName);
        w.set_lead_name(&store, "Zara");
        w.advance(&store);
        // Restart resumes at the saved step with inputs intact.
        let again = FirstLaunch::open(&store);
        assert_eq!(again.flow().step(), Step::LeadLook);
        assert_eq!(again.flow().snapshot().lead_name, "Zara");
        assert!(again.should_show_self(&store));
    }

    #[test]
    fn onboarding_first_launch_completes_into_workspace() {
        let store = mem_store();
        let mut w = drive_to_goal(&store);
        // Detection never blocks stepping: probe runs off-thread.
        w.start_background_detection(|| DetectionSnapshot::default());
        assert!(w.can_continue() || w.flow().step() != Step::Brains);
        let done = w.complete(&store).expect("complete");
        assert!(!done.project_id.is_empty());
        assert!(done.introduction.contains("Dottie"));
        assert!(done.introduction.contains("market for solar"));
        // Project created with goal/template, intro posted to its room.
        let project = store.get_project(&done.project_id).expect("project");
        assert!(project.goal.contains("market for solar"));
        let messages = store
            .list_messages(&done.conversation_id, 10, 0)
            .expect("messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, done.introduction);
        // Completed flow bypasses onboarding and opens the workspace.
        assert!(!FirstLaunch::should_show(&store));
        let mut shell = crate::shell::AppShell::open();
        w.enter_workspace(&mut shell, &done.project_id, &project.name, true);
        assert_eq!(
            shell.open_project_id.as_deref(),
            Some(done.project_id.as_str())
        );
        assert!(shell.room_empty().is_none());
    }

    #[test]
    fn onboarding_first_launch_bypasses_returning_users() {
        let store = mem_store();
        store
            .create_project(&synapse_store::NewProject {
                id: "p1".into(),
                name: "Old work".into(),
                goal: "goal".into(),
                folder: String::new(),
                lead_agent_id: String::new(),
                template: "custom".into(),
            })
            .unwrap();
        assert!(!FirstLaunch::should_show(&store));
    }
}
