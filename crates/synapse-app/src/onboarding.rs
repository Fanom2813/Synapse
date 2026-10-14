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
}
