//! Main window: sidebar, title bar, main area and right panel (UI.md §1, §1b).
//! One file per region; this one owns state, navigation and the frame.

mod actions;
mod add_agent;
mod avatar_dialog;
pub(crate) mod engine_dialog;
mod key_dialog;
mod main_area;
mod nav_row;
mod new_project;
mod right_panel;
mod sidebar;
mod title_bar;

pub use actions::bind_keys;

use crate::chat_view::ChatView;
use crate::icons;
use crate::layout::PanelTab;
use crate::navigation::{AppScreen, SettingsSection};
use crate::reveal::Reveal;
use crate::shell::AppShell;
use crate::theme::Appearance;
use crate::theme_apply::apply_theme;
use crate::views::settings::providers::ProvidersPage;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _, TitleBar};
use gpui_kit::{
    div, px, AnimationExt as _, App, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SpringAnimation, Styled as _,
    Window,
};

/// Space left of the card when the sidebar is closed.
const SIDEBAR_GAP: f32 = 8.0;
/// Space between the main card, the right panel and the window edge.
const PANEL_GAP: f32 = 8.0;

pub(crate) fn tab_icon(tab: PanelTab) -> icons::Icon {
    match tab {
        PanelTab::Team => icons::TAB_TEAM,
        PanelTab::Sources => icons::TAB_SOURCES,
        PanelTab::Artifact => icons::TAB_ARTIFACT,
        PanelTab::Activity => icons::TAB_ACTIVITY,
        PanelTab::Map => icons::TAB_MAP,
        PanelTab::Scheduled => icons::TAB_SCHEDULED,
    }
}

pub struct ShellView {
    pub shell: AppShell,
    pub chat: Entity<ChatView<synapse_llm::GenaiBackend>>,
    /// Built the first time the Providers page opens (loads the catalog).
    pub(crate) providers: Option<ProvidersPage>,
    /// Agents on the open project's team (Team tab).
    pub(crate) team: Vec<synapse_store::Agent>,
    /// Settings → Agents, loaded when that page opens.
    /// Settings → Models, loaded when that page opens.
    pub(crate) models_page: Option<crate::views::settings::models::ModelsPage>,
    pub(crate) agents_page: Option<crate::views::settings::agents::AgentsPage>,
    focus: FocusHandle,
}

impl ShellView {
    pub fn new(
        shell: AppShell,
        chat: Entity<ChatView<synapse_llm::GenaiBackend>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            shell,
            chat,
            providers: None,
            team: Vec::new(),
            models_page: None,
            agents_page: None,
            focus: cx.focus_handle(),
        };
        view.load_team();
        view
    }

    /// Reload the team of the open project from the store.
    fn load_team(&mut self) {
        self.team = match (&self.shell.open_project_id, crate::window::open_store()) {
            (Some(id), Some(store)) => {
                crate::team_setup::ensure_default_team(&store, id);
                crate::team_setup::project_team(&store, id)
            }
            _ => Vec::new(),
        };
    }

    /// Take an agent off the open project's team.
    pub(crate) fn remove_agent(&mut self, agent_id: &str, cx: &mut Context<Self>) {
        if let (Some(project), Some(store)) =
            (&self.shell.open_project_id, crate::window::open_store())
        {
            let _ = crate::team_setup::remove_from_team(&store, project, agent_id);
        }
        self.refresh_team(cx);
    }

    pub(crate) fn refresh_team(&mut self, cx: &mut Context<Self>) {
        self.load_team();
        cx.notify();
    }

    /// Go to a screen, preparing it first (the Providers page loads lazily).
    pub(crate) fn navigate(
        &mut self,
        screen: AppScreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if screen == AppScreen::Settings(SettingsSection::Providers) && self.providers.is_none() {
            self.providers = Some(ProvidersPage::new(window, cx));
        }
        if screen == AppScreen::Settings(SettingsSection::Models) {
            self.reload_models_page(cx);
        }
        if screen == AppScreen::Settings(SettingsSection::Agents) {
            self.open_agents_page(cx);
        }
        match screen {
            AppScreen::Room => self.shell.open_room(),
            AppScreen::Settings(section) => self.shell.open_settings(section),
        }
        cx.notify();
    }

    /// A project was created: list it, open it and show its (empty) room.
    pub(crate) fn project_created(
        &mut self,
        made: crate::project_setup::CreatedProject,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = made.project.id.clone();
        self.shell.projects.push(made.project);
        self.show_project(&id, Some(made.conversation_id), cx);
    }

    /// Open a project and point the chat at its room conversation.
    pub(crate) fn show_project(
        &mut self,
        id: &str,
        conversation: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.shell.open_project(id);
        self.shell.open_room();
        self.load_team();
        let name = self.shell.title.project_name.clone().unwrap_or_default();
        let conversation = conversation.or_else(|| {
            crate::window::open_store()
                .and_then(|store| crate::project_setup::room_conversation(&store, id, &name))
        });
        if let Some(conversation) = conversation {
            self.chat.update(cx, |chat, cx| {
                if let Some(store) = chat.persist_store() {
                    chat.open_conversation(&store, &conversation);
                    cx.notify();
                }
            });
        }
        cx.notify();
    }

    /// Change appearance, apply it to the toolkit theme and persist it.
    pub(crate) fn update_appearance(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Appearance),
    ) {
        let mut appearance = self.shell.appearance.clone();
        change(&mut appearance);
        apply_theme(cx, &appearance);
        self.shell.set_appearance(appearance);
        cx.notify();
    }

    fn on_go_back(&mut self, _: &actions::GoBack, _: &mut Window, cx: &mut Context<Self>) {
        if self.shell.go_back() {
            cx.notify();
        } else {
            // Nothing to go back to: let Esc reach the composer (stop a run).
            cx.propagate();
        }
    }
}

impl Focusable for ShellView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ShellView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Shortcuts dispatch from the focused element up; keep something focused.
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        let t = cx.theme().clone();
        let sidebar_visible = self.shell.layout.sidebar.visible;
        let in_room = self.shell.screen() == AppScreen::Room;
        let panel_visible = self.shell.layout.right_panel_visible && in_room;
        let under = crate::platform::SIDEBAR_UNDER_TITLEBAR;
        let bar_height = px(crate::window_frame::BAR_HEIGHT);

        // The sidebar toggle sits in the sidebar's own strip on macOS (traffic
        // lights over the sidebar), otherwise in the title bar.
        let toggle = Button::new("sidebar-toggle")
            .ghost()
            .small()
            .icon(icons::SIDEBAR)
            .on_click(cx.listener(|this, _, _, cx| {
                this.shell.toggle_sidebar();
                cx.notify();
            }));
        let in_strip = under && sidebar_visible;
        let (strip_toggle, bar_toggle) = if in_strip {
            (Some(toggle), None)
        } else {
            (None, Some(toggle))
        };

        let bar_row = title_bar::bar_row(self, bar_toggle, cx);
        let mut bar = TitleBar::new()
            .h(bar_height)
            .bg(if under {
                t.colors.background
            } else {
                t.colors.sidebar
            })
            .border_b_0();
        // macOS: the toolkit insets the bar 80 px for the traffic lights; with
        // the sidebar open the lights are over the sidebar, so use normal padding.
        if under && sidebar_visible {
            bar = bar.pl_3();
        }
        // The card clips to a rectangle, so a bar with its own background must
        // follow the card's curve or its corners show outside it.
        let bar = bar.rounded_t(t.radius_lg - px(1.0)).child(bar_row);
        let (card_bar, root_bar) = if under {
            (Some(bar), None)
        } else {
            (None, Some(bar))
        };

        let content = h_flex()
            .flex_1()
            .min_h_0()
            .child(main_area::main_area(self, cx));
        let mut card = v_flex()
            .size_full()
            .rounded(t.radius_lg)
            .border_1()
            .border_color(t.colors.border)
            .bg(t.colors.background)
            .overflow_hidden();
        if let Some(bar) = card_bar {
            card = card.child(bar);
        }
        card = card.child(content);

        let mut slot = div().flex_1().min_w_0().h_full().pb_2().pr_2();
        if under {
            slot = slot.pt_2();
        }
        // The sidebar always exists; a spring moves its width between the
        // full sidebar and a thin gap, and the content slides left inside the
        // clip so it leaves the way it came (one soft spring, interruptible).
        let motion = Reveal::new(
            self.shell.appearance.motion,
            self.shell.appearance.reduce_motion,
        );
        let (spring_config, playback) = motion.spring_snappy();
        let spring = move |open: bool| {
            SpringAnimation::new(spring_config)
                .to(open)
                .playback(playback)
        };
        let sidebar = sidebar::sidebar(self, strip_toggle, cx).with_spring(
            "sidebar-slide",
            spring(sidebar_visible),
            |column, phase| {
                column
                    .relative()
                    .left(px(phase.interpolate(-sidebar::WIDTH, 0.0)))
            },
        );
        let sidebar = div()
            .h_full()
            .flex_none()
            .overflow_hidden()
            .child(sidebar)
            .with_spring("sidebar-width", spring(sidebar_visible), |wrap, phase| {
                wrap.w(px(phase.interpolate(SIDEBAR_GAP, sidebar::WIDTH)))
            });
        // The right panel is its own card beside the main one, with a gap.
        // It opens and closes the same way: width springs, content slides.
        let panel_room = right_panel::WIDTH + PANEL_GAP;
        let mut panel_slot = div().w(px(panel_room)).h_full().pb_2().pr_2();
        if under {
            panel_slot = panel_slot.pt_2();
        }
        let panel = div()
            .h_full()
            .flex_none()
            .overflow_hidden()
            .child(
                panel_slot
                    .child(right_panel::right_panel(self, cx))
                    .with_spring("panel-slide", spring(panel_visible), move |slot, phase| {
                        slot.relative().left(px(phase.interpolate(panel_room, 0.0)))
                    }),
            )
            .with_spring("panel-width", spring(panel_visible), move |wrap, phase| {
                wrap.w(px(phase.interpolate(0.0, panel_room)))
            });
        let columns = h_flex()
            .flex_1()
            .min_h_0()
            .child(sidebar)
            .child(slot.child(card))
            .child(panel);

        let mut root = v_flex()
            .id("synapse-shell")
            .key_context(actions::CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_go_back))
            .on_action(cx.listener(|this, _: &actions::ToggleSidebar, _, cx| {
                this.shell.toggle_sidebar();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &actions::TogglePanel, _, cx| {
                this.shell.toggle_right_panel();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &actions::OpenSettings, window, cx| {
                this.navigate(AppScreen::Settings(SettingsSection::Models), window, cx);
            }))
            .size_full()
            .bg(t.colors.sidebar)
            .text_color(t.colors.foreground);
        for (n, action) in panel_tab_actions().into_iter().enumerate() {
            root = action(root, n, cx);
        }
        if let Some(bar) = root_bar {
            root = root.child(bar);
        }
        root.child(columns)
    }
}

type TabBinder = fn(
    gpui_kit::Stateful<gpui_kit::Div>,
    usize,
    &mut Context<ShellView>,
) -> gpui_kit::Stateful<gpui_kit::Div>;

/// Cmd/Ctrl-1..6 select the right-panel tabs. One binder per action type.
fn panel_tab_actions() -> [TabBinder; 6] {
    macro_rules! binder {
        ($action:ty) => {
            |root, n, cx| {
                root.on_action(cx.listener(move |this, _: &$action, _, cx| {
                    if let Some(tab) = PanelTab::from_index(n) {
                        this.shell.select_panel_tab(tab);
                        cx.notify();
                    }
                }))
            }
        };
    }
    [
        binder!(actions::PanelTab1),
        binder!(actions::PanelTab2),
        binder!(actions::PanelTab3),
        binder!(actions::PanelTab4),
        binder!(actions::PanelTab5),
        binder!(actions::PanelTab6),
    ]
}
