//! Foundation window: renders [`AppShell`](crate::shell::AppShell) in a
//! native GPUI Kit window (M1 task 2, UI.md §1 + §1b).
//!
//! The toolkit-free state lives in `shell`/`layout`/`titlebar`; this module
//! is the thin view over it: sidebar / main / right-panel dock with the
//! empty states, plus a custom title bar (sidebar + panel toggles, project
//! switcher, run pill, command-palette button, approvals inbox, budget).
//! Window behaviour (close hides, destroy after 60 s hidden, tray, single
//! instance) is modelled in [`crate::lifecycle`] and wired here.
//!
//! The main column hosts the single-agent [`ChatView`](crate::chat_view::ChatView):
//! a real composer (input + Send/Stop) driving the selected [`ModelBackend`]
//! with streamed replies, errors, and store persistence (see `chat_config`).

use crate::app_dirs;
use crate::chat_view::ChatPersist;
use crate::settings_providers::{ModelRole, ProvidersSettings};
use crate::shell::{AppShell, ShellPersist};

/// Headless description of what the window shows. Toolkit-free so tests
/// run without a display; the GPUI view renders from the same source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellScreen {
    pub window_title: String,
    pub sidebar_text: String,
    pub main_text: String,
    pub panel_text: String,
}

impl ShellScreen {
    pub fn describe(shell: &AppShell) -> Self {
        let window_title = window_title(shell);
        let sidebar_text = match shell.sidebar_empty() {
            Some(e) => format!("{} | {} [{}]", e.title, e.body, e.action),
            None => format!(
                "Projects: {} · [Settings]",
                shell
                    .projects
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let main_text = if shell.is_providers_settings_open() {
            "Settings · Providers | Search providers, enable them, enter keys, test, defaults [Back]".to_string()
        } else {
            match shell.room_empty() {
                Some(e) => format!("{} | {} [{}]", e.title, e.body, e.action),
                None => format!(
                    "Team room: {}",
                    shell.open_project_id.as_deref().unwrap_or("")
                ),
            }
        };
        let panel_text = format!("Panel: {}", shell.layout.right_panel_tab.title());
        Self {
            window_title,
            sidebar_text,
            main_text,
            panel_text,
        }
    }
}

/// Title shown in the OS window frame.
pub fn window_title(shell: &AppShell) -> String {
    match &shell.title.project_name {
        Some(name) => format!("{name} — Synapse"),
        None => "Synapse".to_string(),
    }
}

/// First-launch routing (UI.md §9): a fresh install opens the seven-step
/// onboarding wizard; returning users (completed flow or existing projects)
/// bypass it straight into the normal workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupRoute {
    Onboarding,
    Workspace,
}

/// Decide which window to open first. Toolkit-free so tests run headless.
pub fn resolve_startup(store: &synapse_store::Store) -> StartupRoute {
    if crate::onboarding::FirstLaunch::should_show(store) {
        StartupRoute::Onboarding
    } else {
        StartupRoute::Workspace
    }
}

/// Background backend probe for the Brains step (UI.md §9.4): installed CLI
/// agents (with login state) plus local Ollama / LM Studio models.
///
/// Runs on the worker thread started by [`crate::onboarding::FirstLaunch::start_background_detection`]
/// — never on the UI thread — so it may block on the network. Unreachable
/// servers are skipped, never errors; an empty snapshot means "nothing found".
pub fn onboarding_detection_probe() -> crate::onboarding::DetectionSnapshot {
    use crate::onboarding::{CliEntry, DetectionSnapshot, LocalModelEntry};
    let mut snap = DetectionSnapshot::default();
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return snap,
    };
    rt.block_on(async {
        for agent in synapse_acp::detect_installed().await {
            snap.cli_agents.push(CliEntry {
                id: agent.id,
                label: agent.label,
                logged_in: agent.login == synapse_acp::LoginState::LoggedIn,
            });
        }
        if let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(4))
            .build()
        {
            for server in synapse_llm::local::detect_local_servers(&client).await {
                for model in server.models {
                    snap.local_models.push(LocalModelEntry {
                        server: server.server.label().to_string(),
                        model: model.id,
                    });
                }
            }
        }
        snap.done = true;
    });
    snap
}

/// Open the shell with persisted state from the data dir. Never fails:
/// missing or corrupt files fall back to defaults so the window always opens.
pub fn open_persisted_shell() -> AppShell {
    let dir = app_dirs::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut shell = AppShell::open_with_persist(dir);
    shell.initial_workspace();
    shell
}

/// Backend, model and persistence for the main window's single-agent chat.
///
/// The model comes from the lead default in provider settings (assumed: the
/// single-agent chat speaks with the lead model); the key is read from the
/// OS keychain just before the backend is built. Anything missing falls back
/// to a keyless backend so the window always opens — turns then surface the
/// actionable missing-key error in the chat itself.
pub fn chat_config() -> (
    std::sync::Arc<synapse_llm::GenaiBackend>,
    String,
    ChatPersist,
) {
    use secrecy::ExposeSecret as _;
    use synapse_llm::keys::SecretStore as _;
    let catalog = synapse_llm::Catalog::embedded().ok();
    let store = synapse_store::Store::open(app_dirs::db_path()).ok();
    let mut provider_id = String::new();
    let mut model = String::from("gpt-4o-mini");
    let mut api_key = None;
    let mut base_url = None;
    if let (Some(catalog), Some(store)) = (catalog.as_ref(), store.as_ref()) {
        if let Ok(settings) = ProvidersSettings::load(store) {
            if let Some((p, m)) = settings.default_for(ModelRole::Lead) {
                provider_id = p.clone();
                model = m.clone();
                base_url = settings.base_url_for(catalog, &p);
                api_key = synapse_llm::keys::KeychainSecretStore
                    .get("provider", &p)
                    .ok()
                    .flatten()
                    .map(|s| s.expose_secret().to_string());
            }
        }
    }
    let backend = std::sync::Arc::new(synapse_llm::GenaiBackend::new(
        model.clone(),
        api_key,
        base_url,
    ));
    let persist = ChatPersist {
        store: store.map(std::sync::Arc::new),
        catalog: catalog.map(std::sync::Arc::new),
        provider_id,
        model_id: model.clone(),
    };
    (backend, model, persist)
}

/// Project + conversation rows backing the main window chat. Created on
/// first use; existing rows (and their messages) are reused so chat results
/// survive reopening. Best-effort: persistence is skipped when the store is
/// unavailable.
pub fn ensure_chat_conversation(store: &synapse_store::Store) -> Option<String> {
    const PROJECT_ID: &str = "chat";
    const CONVERSATION_ID: &str = "chat-main";
    if store.get_project(PROJECT_ID).is_err() {
        store
            .create_project(&synapse_store::NewProject {
                id: PROJECT_ID.into(),
                name: "Chat".into(),
                goal: String::new(),
                folder: String::new(),
                lead_agent_id: String::new(),
                template: "custom".into(),
            })
            .ok()?;
    }
    let exists = store
        .list_conversations(PROJECT_ID)
        .map(|rows| rows.iter().any(|c| c.id == CONVERSATION_ID))
        .unwrap_or(false);
    if !exists {
        store
            .create_conversation(CONVERSATION_ID, PROJECT_ID, "chat", "Chat")
            .ok()?;
    }
    Some(CONVERSATION_ID.into())
}

/// Launch the native window. Called from the `synapse` binary; blocks on
/// the GPUI event loop until explicit quit (tray Quit / Cmd-Ctrl-Q).
///
/// Wiring (ARCHITECTURE.md §5, [`crate::lifecycle`]):
/// - Single instance: a second launch sends [`crate::lifecycle::FOCUS_MESSAGE`]
///   to the primary over the local socket and exits; the primary focuses or
///   rebuilds its window.
/// - `QuitMode::Explicit`: closing the last window never quits; the close is
///   recorded as hidden in the [`crate::lifecycle::WindowLifecycle`] and a
///   background task marks it destroyed after
///   [`crate::lifecycle::DESTROY_HIDDEN_AFTER_SECS`].
/// - Tray (gpui-tray) menu Open/Quit plus matching dock menu entries drive
///   [`crate::lifecycle::apply_tray_command`].
#[cfg(not(test))]
pub fn launch() {
    use crate::chat_view::ChatView;
    use gpui_kit::component::input::{InputEvent, InputState};
    use crate::lifecycle::{
        apply_tray_command, bind_single_instance, handle_second_launch, notify_existing_instance,
        read_instance_message, tray_menu_items, OpenEffect, TrayCommand, WindowLifecycle,
        DESTROY_HIDDEN_AFTER_SECS,
    };
    use gpui_kit::component::button::{Button, ButtonVariants};
    use gpui_kit::*;

    actions!(
        synapse_tray,
        [
            TrayOpen,
            TrayQuickAsk,
            TrayPauseAll,
            TrayApprovals,
            TrayQuit
        ]
    );

    struct ShellView {
        shell: AppShell,
        chat: Entity<ChatView<synapse_llm::GenaiBackend>>,
    }

    impl ShellView {
        fn screen(&self) -> ShellScreen {
            ShellScreen::describe(&self.shell)
        }
    }

    impl Render for ShellView {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let screen = self.screen();
            let sidebar_visible = self.shell.layout.sidebar.visible;
            let panel_visible = self.shell.layout.right_panel_visible;
            div()
                .flex()
                .flex_col()
                .size_full()
                .child(
                    // Custom title bar (UI.md §1b): toggles + project + run + palette + inbox.
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .p_2()
                        .child(
                            Button::new("sidebar-toggle")
                                .label(if sidebar_visible {
                                    "≡ Hide"
                                } else {
                                    "≡ Show"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.shell.toggle_sidebar();
                                    cx.notify();
                                })),
                        )
                        .child(div().child(screen.window_title.clone()))
                        .child(div().child(run_pill_text(&self.shell)))
                        .child(Button::new("palette").label("⌘K").on_click(|_, _, _| {}))
                        .child(div().child(inbox_text(&self.shell)))
                        .child(
                            Button::new("panel-toggle")
                                .label(if panel_visible {
                                    "◧ Hide"
                                } else {
                                    "◧ Show"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.shell.toggle_right_panel();
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .child(div().p_2().child(screen.sidebar_text.clone()))
                        .child(
                            div()
                                .flex_1()
                                .p_2()
                                .child(screen.main_text.clone())
                                .child(self.chat.clone()),
                        )
                        .child(div().p_2().child(screen.panel_text.clone())),
                )
                .child(
                    Button::new("theme-cycle")
                        .label("Theme")
                        .secondary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let mut a = this.shell.appearance.clone();
                            a.mode = match a.mode {
                                crate::theme::ThemeMode::System => crate::theme::ThemeMode::Light,
                                crate::theme::ThemeMode::Light => crate::theme::ThemeMode::Dark,
                                crate::theme::ThemeMode::Dark => crate::theme::ThemeMode::System,
                            };
                            this.shell.set_appearance(a);
                            cx.notify();
                        })),
                )
        }
    }

    struct LifecycleHost {
        lifecycle: WindowLifecycle,
        window: Option<AnyWindowHandle>,
    }

    impl Global for LifecycleHost {}

    struct TrayHandle(gpui_tray::Tray);

    impl Global for TrayHandle {}

    fn open_store() -> Option<synapse_store::Store> {
        synapse_store::Store::open(crate::app_dirs::db_path()).ok()
    }

    /// First window (UI.md §9): the seven-step onboarding wizard for fresh
    /// installs, the normal workspace for returning users.
    fn open_startup_window(cx: &mut App) -> AnyWindowHandle {
        let route = open_store()
            .map(|s| resolve_startup(&s))
            .unwrap_or(StartupRoute::Workspace);
        match route {
            StartupRoute::Workspace => {
                let shell = open_persisted_shell();
                open_main_window(cx, shell)
            }
            StartupRoute::Onboarding => {
                let (handle, _) = open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| OnboardingView::new(window, cx))
                })
                .expect("failed to open Synapse onboarding");
                handle
            }
        }
    }

    /// First-launch wizard (UI.md §9): one step at a time with a progress
    /// row, real inputs for every step, Enter = continue / Esc = back, skip
    /// on every step, progress persisted after each step (resume after
    /// restart). Backend detection runs on a worker thread and is polled
    /// here without ever blocking the UI. On Start, the first project is
    /// created, the lead introduction posted, and this same window morphs
    /// into the normal workspace.
    struct OnboardingView {
        flow: crate::onboarding::FirstLaunch,
        name_input: Entity<InputState>,
        key_input: Entity<InputState>,
        goal_input: Entity<InputState>,
        searxng_input: Entity<InputState>,
        provider: String,
        status: String,
        main: Option<(AppShell, Entity<ChatView<synapse_llm::GenaiBackend>>)>,
        _subs: Vec<Subscription>,
    }

    impl OnboardingView {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            use crate::onboarding::FirstLaunch;
            let flow = match open_store() {
                Some(s) => FirstLaunch::open(&s),
                None => {
                    let mem = synapse_store::Store::open_in_memory()
                        .expect("in-memory store");
                    FirstLaunch::open(&mem)
                }
            };
            let snap = flow.flow().snapshot().clone();
            let name_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Lead name")
                    .default_value(snap.lead_name.clone())
            });
            let key_input =
                cx.new(|cx| InputState::new(window, cx).placeholder("API key").masked(true));
            let goal_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("What should the team figure out?")
                    .default_value(snap.project_goal.clone())
            });
            let searxng_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("SearXNG URL (optional)")
                    .default_value(snap.searxng_url.clone())
            });
            let _subs = vec![
                cx.subscribe_in(&name_input, window, Self::on_name_event),
                cx.subscribe_in(&goal_input, window, Self::on_goal_event),
                cx.subscribe_in(&searxng_input, window, Self::on_searxng_event),
            ];
            // Backend detection runs off the UI thread; poll it here without
            // ever waiting on the network (UI.md §9: nothing blocks).
            flow.start_background_detection(onboarding_detection_probe);
            cx.spawn(async move |this, cx| {
                for _ in 0..240 {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(500))
                        .await;
                    let alive = this.update(cx, |this, cx| {
                        this.poll_detection();
                        cx.notify();
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            })
            .detach();
            Self {
                flow,
                name_input,
                key_input,
                goal_input,
                searxng_input,
                provider: String::from("anthropic"),
                status: String::new(),
                main: None,
                _subs,
            }
        }

        fn on_name_event(
            &mut self,
            _: &Entity<InputState>,
            event: &InputEvent,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) {
            if matches!(event, InputEvent::Change) {
                let name = self.name_input.read(cx).value().to_string();
                if let Some(store) = open_store() {
                    self.flow.set_lead_name(&store, &name);
                }
                cx.notify();
            }
        }

        fn on_goal_event(
            &mut self,
            _: &Entity<InputState>,
            event: &InputEvent,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) {
            if matches!(event, InputEvent::Change) {
                let goal = self.goal_input.read(cx).value().to_string();
                if let Some(store) = open_store() {
                    self.flow.set_goal(&store, &goal);
                }
                cx.notify();
            }
        }

        fn on_searxng_event(
            &mut self,
            _: &Entity<InputState>,
            event: &InputEvent,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) {
            if matches!(event, InputEvent::Change) {
                let url = self.searxng_input.read(cx).value().to_string();
                if let Some(store) = open_store() {
                    let browser = self.flow.flow().snapshot().browser_connected;
                    self.flow.set_research(&store, browser, &url);
                }
                cx.notify();
            }
        }

        fn poll_detection(&mut self) {
            if let Some(store) = open_store() {
                self.flow.poll_detection(&store);
            }
        }

        fn set_name_chip(&mut self, window: &mut Window, cx: &mut Context<Self>, name: &str) {
            if let Some(store) = open_store() {
                self.flow.set_lead_name(&store, name);
                self.name_input.update(cx, |input, cx| {
                    input.set_value(name, window, cx);
                });
            }
            cx.notify();
        }

        fn set_goal_chip(&mut self, window: &mut Window, cx: &mut Context<Self>, goal: &str) {
            if let Some(store) = open_store() {
                self.flow.set_goal(&store, goal);
                self.goal_input.update(cx, |input, cx| {
                    input.set_value(goal, window, cx);
                });
            }
            cx.notify();
        }

        fn do_back(&mut self, cx: &mut Context<Self>) {
            if let Some(store) = open_store() {
                self.flow.back(&store);
            }
            self.status.clear();
            cx.notify();
        }

        fn do_skip(&mut self, cx: &mut Context<Self>) {
            if let Some(store) = open_store() {
                self.flow.skip(&store);
            }
            self.status.clear();
            cx.notify();
        }

        /// Enter = continue (or Start on the last step). Persists every move
        /// so an unfinished flow resumes after restart.
        fn do_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            use crate::onboarding::Step;
            if self.flow.flow().step() == Step::FirstProject {
                if let Some(store) = open_store() {
                    match self.flow.complete(&store) {
                        Ok(done) => {
                            let project_name = store
                                .get_project(&done.project_id)
                                .map(|p| p.name)
                                .unwrap_or_else(|_| "First project".to_string());
                            let mut shell = open_persisted_shell();
                            self.flow.enter_workspace(
                                &mut shell,
                                &done.project_id,
                                &project_name,
                                true,
                            );
                            let (backend, model, persist) = chat_config();
                            let conversation_id = done.conversation_id.clone();
                            let chat = cx.new(|cx| {
                                let mut view = ChatView::new(window, cx, backend, model)
                                    .with_persist(persist);
                                if let Some(store) = view.persist_store() {
                                    view.open_conversation(&store, &conversation_id);
                                }
                                view
                            });
                            self.status.clear();
                            self.main = Some((shell, chat));
                        }
                        Err(e) => {
                            self.status = e;
                        }
                    }
                }
            } else if self.flow.can_continue() {
                if let Some(store) = open_store() {
                    self.flow.advance(&store);
                }
                self.status.clear();
            } else {
                self.status = self
                    .flow
                    .flow()
                    .blocked_hint()
                    .unwrap_or("Not yet — see above.")
                    .to_string();
            }
            cx.notify();
        }

        /// API-key path: the key itself goes to the OS keychain (never the
        /// DB); only the live-test flag persists with the flow. Assumed: a
        /// non-empty key that the keychain accepts counts as tested here;
        /// the full live model-list check lives in Settings → Providers.
        fn do_test_key(&mut self, cx: &mut Context<Self>) {
            use synapse_llm::keys::SecretStore as _;
            let key = self.key_input.read(cx).value().to_string();
            if key.trim().is_empty() {
                self.status = "Paste a key first.".to_string();
            } else if synapse_llm::keys::KeychainSecretStore
                .set("provider", &self.provider, key.trim())
                .is_err()
            {
                self.status = "Could not save that key (keychain unavailable).".to_string();
            } else {
                if let Some(store) = open_store() {
                    self.flow.set_api_result(&store, &self.provider.clone(), true);
                }
                self.status = format!("{} key saved ✓", self.provider);
            }
            cx.notify();
        }
    }

    impl Render for OnboardingView {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            use crate::onboarding::{Step, EXAMPLE_GOALS};
            use gpui_kit::component::button::{Button, ButtonVariants};
            use gpui_kit::component::input::Input;
            // Completed: this window is now the normal workspace.
            if let Some((shell, chat)) = &self.main {
                let screen = ShellScreen::describe(shell);
                return div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(div().p_2().child(screen.window_title.clone()))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_1()
                            .child(div().p_2().child(screen.sidebar_text.clone()))
                            .child(div().flex_1().p_2().child(chat.clone())),
                    );
            }
            let step = self.flow.flow().step();
            let (n, total) = self.flow.flow().progress();
            let snap = self.flow.flow().snapshot().clone();
            let can_continue = self.flow.can_continue();
            let continue_label = if step == Step::FirstProject {
                "Start"
            } else {
                "Continue"
            };
            let mut col = div()
                .flex()
                .flex_col()
                .size_full()
                .p_4()
                .gap_3()
                .on_key_down(cx.listener(
                    |this: &mut Self, event: &KeyDownEvent, window, cx| {
                        let key: &str = &event.keystroke.key;
                        match key {
                            "enter" => this.do_continue(window, cx),
                            "escape" => this.do_back(cx),
                            _ => {}
                        }
                    },
                ))
                .child(div().child(format!("Step {n}/{total}")))
                .child(div().child(step.title().to_string()));
            match step {
                Step::Hello => {
                    col = col
                        .child(div().child("Your research team lives here. One question at a time."))
                        .child(
                            Button::new("ob-import")
                                .label("I've used Synapse before → skip")
                                .secondary()
                                .on_click(cx.listener(|this, _, _, cx| this.do_skip(cx))),
                        );
                }
                Step::LeadName => {
                    col = col
                        .child(Input::new(&self.name_input))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_2()
                                .child(
                                    Button::new("ob-name-dottie")
                                        .label("Dottie")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_name_chip(window, cx, "Dottie");
                                        })),
                                )
                                .child(
                                    Button::new("ob-name-mira")
                                        .label("Mira")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_name_chip(window, cx, "Mira");
                                        })),
                                )
                                .child(
                                    Button::new("ob-name-zara")
                                        .label("Zara")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_name_chip(window, cx, "Zara");
                                        })),
                                ),
                        );
                }
                Step::LeadLook => {
                    let look = format!(
                        "{} · {} · {} {}",
                        snap.shape, snap.palette, snap.eyes, snap.accessory
                    );
                    col = col
                        .child(div().child(look))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_2()
                                .child(Button::new("ob-shape-bean").label("bean").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        if let Some(store) = open_store() {
                                            let s = this.flow.flow().snapshot().clone();
                                            this.flow.set_look(
                                                &store,
                                                "bean",
                                                &s.palette,
                                                &s.eyes,
                                                &s.accessory,
                                            );
                                        }
                                        cx.notify();
                                    }),
                                ))
                                .child(Button::new("ob-shape-round").label("round").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        if let Some(store) = open_store() {
                                            let s = this.flow.flow().snapshot().clone();
                                            this.flow.set_look(
                                                &store,
                                                "round",
                                                &s.palette,
                                                &s.eyes,
                                                &s.accessory,
                                            );
                                        }
                                        cx.notify();
                                    }),
                                ))
                                .child(
                                    Button::new("ob-surprise")
                                        .label("Surprise me")
                                        .secondary()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(store) = open_store() {
                                                // Deterministic shuffle from the name so it is
                                                // stable across restarts, bounced by the view.
                                                let n: usize = this
                                                    .flow
                                                    .flow()
                                                    .snapshot()
                                                    .lead_name
                                                    .bytes()
                                                    .map(|b| b as usize)
                                                    .sum::<usize>()
                                                    + this.flow.flow().snapshot().skipped.len();
                                                let shapes =
                                                    ["bean", "round", "pear", "cloud", "drop"];
                                                let palettes =
                                                    ["green", "amber", "violet", "blue"];
                                                this.flow.set_look(
                                                    &store,
                                                    shapes[n % shapes.len()],
                                                    palettes[(n / shapes.len()) % palettes.len()],
                                                    "diamond",
                                                    "",
                                                );
                                            }
                                            cx.notify();
                                        })),
                                ),
                        );
                }
                Step::Brains => {
                    let found = format!(
                        "Found: {} CLIs ({} logged in), {} local models{}",
                        snap.detection.cli_agents.len(),
                        snap.detection
                            .cli_agents
                            .iter()
                            .filter(|c| c.logged_in)
                            .count(),
                        snap.detection.local_models.len(),
                        if snap.detection.done {
                            ""
                        } else {
                            " — still looking…"
                        }
                    );
                    col = col
                        .child(div().child(found))
                        .child(Input::new(&self.key_input))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_2()
                                .child(
                                    Button::new("ob-key-test")
                                        .label("Save key & test")
                                        .on_click(cx.listener(|this, _, _, cx| this.do_test_key(cx))),
                                )
                                .child(
                                    Button::new("ob-backend-keyless")
                                        .label("Try free, no keys")
                                        .secondary()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(store) = open_store() {
                                                this.flow.set_brains_backend(&store, "keyless");
                                            }
                                            cx.notify();
                                        })),
                                ),
                        );
                    if !can_continue {
                        col = col.child(div().child(
                            self.flow
                                .flow()
                                .blocked_hint()
                                .unwrap_or("Still looking…")
                                .to_string(),
                        ));
                    }
                }
                Step::Research => {
                    let browser = if snap.browser_connected {
                        "Browser connected ✓"
                    } else {
                        "Browser not connected (keyless search already works)"
                    };
                    col = col
                        .child(div().child(browser.to_string()))
                        .child(Input::new(&self.searxng_input))
                        .child(
                            Button::new("ob-browser-toggle")
                                .label(if snap.browser_connected {
                                    "Disconnect browser"
                                } else {
                                    "Connect browser"
                                })
                                .secondary()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(store) = open_store() {
                                        let s = this.flow.flow().snapshot().clone();
                                        this.flow.set_research(
                                            &store,
                                            !s.browser_connected,
                                            &s.searxng_url,
                                        );
                                    }
                                    cx.notify();
                                })),
                        );
                }
                Step::Team => {
                    let mut row = div().flex().flex_row().gap_2();
                    for t in synapse_core::builtin_templates() {
                        let id = t.id.clone();
                        let label = t.name.clone();
                        let btn_id = format!("ob-template-{}", t.id);
                        row = row.child(
                            Button::new(btn_id)
                                .label(label)
                                .secondary()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(store) = open_store() {
                                        this.flow.set_template(&store, &id);
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    col = col
                        .child(div().child(format!("Team: {}", self.flow.flow().template_name())))
                        .child(row);
                }
                Step::FirstProject => {
                    let mut chips = div().flex().flex_row().gap_2();
                    for (i, goal) in EXAMPLE_GOALS.iter().enumerate() {
                        let g = goal.to_string();
                        chips = chips.child(
                            Button::new(format!("ob-goal-{i}"))
                                .label(g.clone())
                                .secondary()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_goal_chip(window, cx, &g.clone());
                                })),
                        );
                    }
                    col = col.child(Input::new(&self.goal_input)).child(chips).child(
                        div().child("Keep Synapse in the menu bar: toggle after the first run starts."),
                    );
                }
            }
            if !self.status.is_empty() {
                col = col.child(div().child(self.status.clone()));
            }
            col.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        Button::new("ob-back")
                            .label("Back (Esc)")
                            .secondary()
                            .on_click(cx.listener(|this, _, _, cx| this.do_back(cx))),
                    )
                    .child(
                        Button::new("ob-skip")
                            .label("Skip")
                            .secondary()
                            .on_click(cx.listener(|this, _, _, cx| this.do_skip(cx))),
                    )
                    .child(if can_continue || step == Step::FirstProject {
                        Button::new("ob-continue")
                            .label(continue_label)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.do_continue(window, cx);
                            }))
                    } else {
                        Button::new("ob-continue-disabled").label(continue_label).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.status = this
                                    .flow
                                    .flow()
                                    .blocked_hint()
                                    .unwrap_or("Not yet — see above.")
                                    .to_string();
                                cx.notify();
                            }),
                        )
                    }),
            )
        }
    }

    fn open_main_window(cx: &mut App, shell: AppShell) -> AnyWindowHandle {
        let (handle, _) = open_window(WindowOptions::default(), cx, |window, cx| {
            let (backend, model, persist) = chat_config();
            let chat = cx.new(|cx| {
                let mut view = ChatView::new(window, cx, backend, model).with_persist(persist);
                if let Some(store) = view.persist_store() {
                    if let Some(conversation) = ensure_chat_conversation(&store) {
                        view.open_conversation(&store, &conversation);
                    }
                }
                view
            });
            cx.new(|_| ShellView { shell, chat })
        })
        .expect("failed to open Synapse window");
        handle
    }

    /// Focus the live window, or (re)build it when it was hidden/destroyed.
    /// Reports the [`OpenEffect`] so tray and IPC paths share one behaviour.
    fn show_or_rebuild(cx: &mut App) -> OpenEffect {
        let effect = cx.global_mut::<LifecycleHost>().lifecycle.open_window();
        let live = cx
            .global::<LifecycleHost>()
            .window
            .and_then(|h| h.update(cx, |_, window, _| window.activate_window()).ok())
            .is_some();
        if live {
            cx.activate(true);
            return OpenEffect::Focus;
        }
        // No live native window (closed, or never opened): rebuild from store.
        let handle = open_startup_window(cx);
        cx.global_mut::<LifecycleHost>().window = Some(handle);
        cx.activate(true);
        effect
    }

    fn on_tray_open(_: &TrayOpen, cx: &mut App) {
        // Keep the state machine as the source of truth for the effect.
        let _ = apply_tray_command(
            &mut cx.global_mut::<LifecycleHost>().lifecycle,
            TrayCommand::Open,
        );
        show_or_rebuild(cx);
    }

    fn on_tray_quick_ask(_: &TrayQuickAsk, cx: &mut App) {
        let _ = apply_tray_command(
            &mut cx.global_mut::<LifecycleHost>().lifecycle,
            TrayCommand::QuickAsk,
        );
        show_or_rebuild(cx);
    }

    fn on_tray_pause_all(_: &TrayPauseAll, cx: &mut App) {
        let _ = apply_tray_command(
            &mut cx.global_mut::<LifecycleHost>().lifecycle,
            TrayCommand::PauseAll,
        );
        show_or_rebuild(cx);
    }

    fn on_tray_approvals(_: &TrayApprovals, cx: &mut App) {
        let _ = apply_tray_command(
            &mut cx.global_mut::<LifecycleHost>().lifecycle,
            TrayCommand::ShowApprovals,
        );
        show_or_rebuild(cx);
    }

    fn on_tray_quit(_: &TrayQuit, cx: &mut App) {
        let _ = apply_tray_command(
            &mut cx.global_mut::<LifecycleHost>().lifecycle,
            TrayCommand::Quit,
        );
        let tray = cx.try_global::<TrayHandle>().map(|t| t.0.clone());
        if let Some(tray) = tray {
            let _ = tray.close(cx);
        }
        cx.quit();
    }

    fn build_tray_menu(_cx: &mut App) -> Vec<MenuItem> {
        use crate::lifecycle::parse_tray_menu_item;
        tray_menu_items()
            .into_iter()
            .filter_map(|label| {
                let cmd = parse_tray_menu_item(&label)?;
                Some(match cmd {
                    TrayCommand::Open | TrayCommand::ShowRun(_) => {
                        MenuItem::action(label, TrayOpen)
                    }
                    TrayCommand::QuickAsk => MenuItem::action("Quick Ask", TrayQuickAsk),
                    TrayCommand::PauseAll | TrayCommand::ResumeAll => {
                        MenuItem::action("Pause All", TrayPauseAll)
                    }
                    TrayCommand::ShowApprovals => {
                        MenuItem::action("Pending approvals", TrayApprovals)
                    }
                    TrayCommand::Quit => MenuItem::action("Quit", TrayQuit),
                })
            })
            .collect()
    }

    /// Tray dot colour per icon state: idle blue, working green,
    /// needs-you amber. Toolkit-free state lives in `lifecycle`.
    fn tray_icon_for(state: crate::lifecycle::TrayIconState) -> gpui_tray::Result<gpui_tray::Icon> {
        let rgb = match state {
            crate::lifecycle::TrayIconState::Idle => [45, 105, 220],
            crate::lifecycle::TrayIconState::Working => [46, 160, 90],
            crate::lifecycle::TrayIconState::NeedsYou => [220, 140, 30],
        };
        tray_dot_icon(rgb)
    }

    fn tray_dot_icon(rgb: [u8; 3]) -> gpui_tray::Result<gpui_tray::Icon> {
        const SIZE: u32 = 32;
        let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
        for y in 0..SIZE {
            for x in 0..SIZE {
                let offset = ((y * SIZE + x) * 4) as usize;
                let dx = x as i32 - 16;
                let dy = y as i32 - 16;
                if dx * dx + dy * dy <= 14 * 14 {
                    rgba[offset..offset + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                }
            }
        }
        gpui_tray::Icon::from_rgba(rgba, SIZE, SIZE)
    }

    fn tray_icon() -> gpui_tray::Result<gpui_tray::Icon> {
        tray_icon_for(crate::lifecycle::TrayIconState::Idle)
    }

    // --- single instance: second launch focuses the primary and exits ---
    let dir = crate::app_dirs::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if notify_existing_instance(&dir) {
        return;
    }
    let listener = match bind_single_instance(&dir) {
        Ok(listener) => Some(listener),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // Lost the race: a primary is live, ask it to focus and exit.
            let _ = notify_existing_instance(&dir);
            return;
        }
        Err(e) => {
            eprintln!("single-instance socket unavailable ({e}); continuing without it");
            None
        }
    };
    let (focus_tx, mut focus_rx) = futures::channel::mpsc::unbounded::<String>();
    if let Some(listener) = listener {
        std::thread::Builder::new()
            .name("synapse-instance-ipc".to_string())
            .spawn(move || {
                use interprocess::local_socket::traits::ListenerExt as _;
                for mut conn in listener.incoming().flatten() {
                    let msg = read_instance_message(&mut conn);
                    if focus_tx.unbounded_send(msg).is_err() {
                        break;
                    }
                }
            })
            .expect("failed to spawn single-instance IPC thread");
    }

    application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            use futures::StreamExt as _;
            init(cx);
            cx.set_quit_mode(QuitMode::Explicit);
            cx.set_global(LifecycleHost {
                lifecycle: WindowLifecycle::new_visible(),
                window: None,
            });
            cx.on_action(on_tray_open);
            cx.on_action(on_tray_quick_ask);
            cx.on_action(on_tray_pause_all);
            cx.on_action(on_tray_approvals);
            cx.on_action(on_tray_quit);
            cx.set_dock_menu(vec![
                MenuItem::action("Open", TrayOpen),
                MenuItem::action("Quit", TrayQuit),
            ]);
            match gpui_tray::Tray::builder()
                .icon(tray_icon().expect("tray icon"))
                .title("Synapse")
                .tooltip("Synapse")
                .menu(build_tray_menu)
                .build(cx)
            {
                Ok(tray) => cx.set_global(TrayHandle(tray)),
                Err(e) => eprintln!("tray unavailable ({e}); Open/Quit remain in the dock menu"),
            }
            let handle = open_startup_window(cx);
            cx.global_mut::<LifecycleHost>().window = Some(handle);
            // Close hides: Explicit quit mode keeps the app alive; record the
            // hidden state and destroy it after the threshold (frees GPU).
            // GPUI gives no close-interception hook, so a closed native window
            // is rebuilt from the store on the next open (tray/dock/IPC).
            // (`Subscription` deactivates on drop; the app owns this observer
            // for its whole lifetime, so it is intentionally forgotten.)
            std::mem::forget(cx.on_window_closed(|cx: &mut App, _id| {
                let host = cx.global_mut::<LifecycleHost>();
                host.lifecycle.close_window();
                host.window = None;
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(DESTROY_HIDDEN_AFTER_SECS))
                        .await;
                    cx.update(|cx| {
                        let host = cx.global_mut::<LifecycleHost>();
                        host.lifecycle.advance(DESTROY_HIDDEN_AFTER_SECS);
                    });
                })
                .detach();
            }));
            // Second launches arrive here: focus the running instance.
            cx.spawn(async move |cx| {
                while let Some(msg) = focus_rx.next().await {
                    cx.update(|cx| {
                        let (_, _) = handle_second_launch(
                            &mut cx.global_mut::<LifecycleHost>().lifecycle,
                            msg.as_str(),
                        );
                        show_or_rebuild(cx);
                    });
                }
            })
            .detach();
        });
}

/// Run-pill line for the title bar (mode + phase, or "Start a run").
/// Paused runs prefix "Paused · "; narrow windows shrink to icon + avatars.
pub fn run_pill_text(shell: &AppShell) -> String {
    if shell
        .title
        .layout_for_width(window_width_hint())
        .compact_run_pill
    {
        return compact_run_pill_text(shell);
    }
    shell.title.run_label()
}

/// Compact run pill for narrow windows: icon + avatar count.
pub fn compact_run_pill_text(shell: &AppShell) -> String {
    match &shell.title.run {
        crate::titlebar::RunPill::Idle => "Start a run".to_string(),
        crate::titlebar::RunPill::Running { .. } => {
            let (shown, overflow) = shell.title.run.visible_agents();
            let extra = if overflow > 0 {
                format!(" +{overflow}")
            } else {
                String::new()
            };
            if shell.title.run_paused {
                format!("Paused · {} agents{extra}", shown.len())
            } else {
                format!("{} agents{extra}", shown.len())
            }
        }
    }
}

/// Width hint for headless layout decisions (real view passes the window
/// width instead). Narrow enough that tests exercise the overflow path.
fn window_width_hint() -> f32 {
    1280.0
}

/// Run popover body: phase timeline + budget + available actions.
pub fn run_popover_text(shell: &AppShell) -> String {
    match &shell.title.run {
        crate::titlebar::RunPill::Idle => "No active run [Start a run]".to_string(),
        crate::titlebar::RunPill::Running { .. } => {
            let actions = shell
                .title
                .run_actions()
                .iter()
                .map(|a| match a {
                    crate::titlebar::RunAction::Pause => "Pause",
                    crate::titlebar::RunAction::Resume => "Resume",
                    crate::titlebar::RunAction::Stop => "Stop",
                    crate::titlebar::RunAction::OpenActivity => "Activity",
                })
                .collect::<Vec<_>>()
                .join(" · ");
            let timeline = if shell.title.phase_history.is_empty() {
                shell.title.run_label()
            } else {
                shell.title.phase_history.join(" → ")
            };
            format!("{timeline} | {} | [{actions}]", shell.title.budget.text())
        }
    }
}

/// Approvals inbox badge text ("2" or "" when none).
pub fn inbox_text(shell: &AppShell) -> String {
    let b = shell.title.approvals_badge();
    if b.is_empty() {
        "Inbox".to_string()
    } else {
        format!("Inbox ({b})")
    }
}

/// Persist handle for the data dir (used by the binary for tests/headless).
pub fn data_persist() -> ShellPersist {
    ShellPersist {
        dir: app_dirs::data_dir(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::ShellProject;
    use crate::titlebar::RunPill;

    #[test]
    fn startup_routes_fresh_users_to_onboarding() {
        let store = synapse_store::Store::open_in_memory().expect("store");
        assert_eq!(resolve_startup(&store), StartupRoute::Onboarding);
    }

    #[test]
    fn startup_bypasses_returning_users_into_workspace() {
        let store = synapse_store::Store::open_in_memory().expect("store");
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
        assert_eq!(resolve_startup(&store), StartupRoute::Workspace);
    }

    #[test]
    fn onboarding_detection_probe_returns_without_network_wait() {
        // The probe may touch localhost/the network; it must still return
        // promptly and never fail — the wizard never waits on it.
        let started = std::time::Instant::now();
        let snap = onboarding_detection_probe();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "probe blocked the caller"
        );
        // Shape check only: entries, if any, are well-formed.
        for cli in &snap.cli_agents {
            assert!(!cli.id.is_empty());
        }
    }

    #[test]
    fn window_chat_conversation_is_reused_across_opens() {
        let store = synapse_store::Store::open_in_memory().expect("store");
        let first = ensure_chat_conversation(&store).expect("conversation");
        store
            .insert_message(&synapse_store::NewMessage {
                id: "chat-main-m0".into(),
                conversation_id: first.clone(),
                run_id: String::new(),
                from_id: "user".into(),
                to_json: "[]".into(),
                kind: "ask".into(),
                reply_to: String::new(),
                thread_id: String::new(),
                body: "hello".into(),
                meta: "{}".into(),
            })
            .expect("message");
        // Reopening reuses the same conversation, so history survives.
        let second = ensure_chat_conversation(&store).expect("conversation");
        assert_eq!(second, first);
        let rows = store.list_messages(&first, 10, 0).expect("messages");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn shell_window_shows_sidebar_and_room_empty_states() {
        let mut shell = AppShell::open();
        shell.initial_workspace();
        let screen = ShellScreen::describe(&shell);
        assert_eq!(screen.window_title, "Synapse");
        assert!(screen.sidebar_text.contains("Name your first project"));
        assert!(screen.sidebar_text.contains("[New project]"));
        assert!(screen.main_text.contains("[New project]"));
        assert!(screen.panel_text.contains("Team"));
        assert_eq!(window_title(&shell), "Synapse");
    }

    #[test]
    fn shell_window_lists_projects_and_room_state() {
        let mut shell = AppShell::open();
        shell.projects.push(ShellProject {
            id: "p1".to_string(),
            name: "Market study".to_string(),
            has_messages: false,
        });
        shell.projects.push(ShellProject {
            id: "p2".to_string(),
            name: "Uganda fintech".to_string(),
            has_messages: true,
        });
        shell.initial_workspace();
        let screen = ShellScreen::describe(&shell);
        assert_eq!(screen.window_title, "Market study — Synapse");
        assert!(screen.sidebar_text.contains("Market study"));
        // p1 has no messages: room empty state with the run action.
        assert!(screen.main_text.contains("[Start a run]"));
        assert!(shell.open_project("p2"));
        let screen = ShellScreen::describe(&shell);
        assert!(screen.main_text.contains("Team room: p2"));

        shell.set_run(RunPill::Running {
            mode: "Deep research".to_string(),
            phase: "Searching".to_string(),
            progress: 0.4,
            live_agent_ids: vec!["lead".to_string()],
        });
        assert_eq!(run_pill_text(&shell), "Deep research · Searching");
        shell.title.pending_approvals = 2;
        assert_eq!(inbox_text(&shell), "Inbox (2)");
        shell.title.pending_approvals = 0;
        assert_eq!(run_pill_text(&AppShell::open()), "Start a run");
        assert_eq!(inbox_text(&shell), "Inbox");
    }
}
