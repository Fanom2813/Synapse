//! First-launch wizard (UI.md §9). Logic and window morph live here; the
//! per-step card is drawn in `steps`.

mod detected;
pub(crate) mod provider_dialog;
mod provider_list;
mod steps;

use crate::chat_view::ChatView;
use crate::reveal::Reveal;
use crate::views::shell::ShellView;
use crate::window::{chat_config, onboarding_detection_probe, open_persisted_shell, open_store};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{v_flex, ActiveTheme as _, TitleBar};
use gpui_kit::*;

/// First-launch wizard (UI.md §9): one step at a time with a progress
/// row, real inputs for every step, Enter = continue / Esc = back, skip
/// on every step, progress persisted after each step (resume after
/// restart). Backend detection runs on a worker thread and is polled
/// here without ever blocking the UI. On Start, the first project is
/// created, the lead introduction posted, and this same window morphs
/// into the normal workspace.
pub struct OnboardingView {
    flow: crate::onboarding::FirstLaunch,
    name_input: Entity<InputState>,
    /// Providers switched on, loaded off the UI thread.
    added: Vec<crate::provider_setup::AddedProvider>,
    status: String,
    main: Option<Entity<ShellView>>,
    reveal: Reveal,
    /// Direction of the last move, so steps slide the way you are going.
    forward: bool,
    _subs: Vec<Subscription>,
}

impl OnboardingView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        use crate::onboarding::FirstLaunch;
        let flow = match open_store() {
            Some(s) => FirstLaunch::open(&s),
            None => {
                let mem = synapse_store::Store::open_in_memory().expect("in-memory store");
                FirstLaunch::open(&mem)
            }
        };
        let snap = flow.flow().snapshot().clone();
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Lead name")
                .default_value(snap.lead_name.clone())
        });
        let _subs = vec![cx.subscribe_in(&name_input, window, Self::on_name_event)];
        // Backend detection runs off the UI thread; poll it here without
        // ever waiting on the network (UI.md §9: nothing blocks).
        flow.start_background_detection(onboarding_detection_probe);
        // Redraw only when the probe found something new, and stop once it
        // is done: no idle timer left running (AGENTS.md §6).
        cx.spawn(async move |this, cx| {
            for _ in 0..240 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                let done = this.update(cx, |this, cx| {
                    let before = this.flow.flow().snapshot().detection.clone();
                    this.poll_detection();
                    let after = &this.flow.flow().snapshot().detection;
                    if *after != before {
                        cx.notify();
                    }
                    after.done
                });
                if done.unwrap_or(true) {
                    break;
                }
            }
        })
        .detach();
        // Providers already switched on: read on a worker, which also warms
        // the catalog so opening the Add popup later is instant.
        cx.spawn(async move |this, cx| {
            let added = cx
                .background_executor()
                .spawn(async { open_store().map(|s| crate::provider_setup::list_added(&s)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(added) = added {
                    this.added = added;
                    cx.notify();
                }
            });
        })
        .detach();
        let appearance = crate::shell::ShellPersist {
            dir: crate::app_dirs::data_dir(),
        }
        .load_appearance();
        Self {
            flow,
            name_input,
            added: Vec::new(),
            status: String::new(),
            main: None,
            reveal: Reveal::new(appearance.motion, appearance.reduce_motion),
            forward: true,
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

    fn do_back(&mut self, cx: &mut Context<Self>) {
        if let Some(store) = open_store() {
            self.flow.back(&store);
        }
        self.forward = false;
        self.status.clear();
        cx.notify();
    }

    fn do_skip(&mut self, cx: &mut Context<Self>) {
        if let Some(store) = open_store() {
            self.flow.skip(&store);
        }
        self.forward = true;
        self.status.clear();
        cx.notify();
    }

    /// Enter = continue (or Start on the last step). Persists every move
    /// so an unfinished flow resumes after restart.
    fn do_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::onboarding::Step;
        if self.flow.flow().step() == Step::AllSet {
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
                            let mut view =
                                ChatView::new(window, cx, backend, model).with_persist(persist);
                            if let Some(store) = view.persist_store() {
                                view.open_conversation(&store, &conversation_id);
                            }
                            view
                        });
                        self.status.clear();
                        self.main = Some(cx.new(|cx| ShellView::new(shell, chat, cx)));
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
            self.forward = true;
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
}

impl Render for OnboardingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Completed: this window is now the normal workspace.
        if let Some(workspace) = &self.main {
            return self
                .reveal
                .enter(
                    div().size_full().child(workspace.clone()),
                    "ob-done",
                    0,
                    (0.0, 0.0),
                )
                .into_any_element();
        }
        let theme = cx.theme().clone();
        let first_step = self.flow.flow().step() == crate::onboarding::Step::Hello;
        // Anchor the content a fixed distance below the bar so steps do not
        // jump; sized to centre the first step (logo + card) in the window.
        let width = window.viewport_size().width.as_f32();
        let top =
            ((window.viewport_size().height.as_f32() - crate::window_frame::BAR_HEIGHT - 380.0)
                / 2.0)
                .max(24.0);
        let card = self.render_step_card(window, cx);
        v_flex()
            .size_full()
            .bg(theme.colors.background)
            .text_color(theme.colors.foreground)
            .child(
                TitleBar::new()
                    .h(px(crate::window_frame::BAR_HEIGHT))
                    .bg(theme.colors.background)
                    .border_b_0(),
            )
            .child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .pt(px(top))
                    .child(self.render_logo_slot(first_step, top, width))
                    .child(card),
            )
            .into_any_element()
    }
}
