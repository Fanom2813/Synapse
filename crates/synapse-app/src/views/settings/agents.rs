//! Settings → Agents: the default engine for every agent, and an optional
//! engine per agent. An engine is the default model (chosen under Providers)
//! or an installed coding CLI.

use crate::team_setup::{self, Engine};
use crate::theme::TextRole;
use crate::typography::Typography as _;
use crate::views::shell::engine_dialog::EngineTarget;
use crate::views::shell::ShellView;
use gpui_kit::component::button::Button;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _};
use gpui_kit::{
    div, AnyElement, Context, Div, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _,
};
use synapse_store::Agent;

/// What the page shows, read from the store when it opens.
pub(crate) struct AgentsPage {
    default: Engine,
    agents: Vec<(Agent, Option<Engine>)>,
    /// Installed CLIs as `(id, label)`, found on a worker thread.
    clis: Vec<(String, String)>,
}

impl AgentsPage {
    fn load(clis: Vec<(String, String)>) -> Option<Self> {
        let store = crate::window::open_store()?;
        let agents = store
            .list_agents()
            .unwrap_or_default()
            .into_iter()
            .map(|a| {
                let own = team_setup::agent_override(&store, &a.id);
                (a, own)
            })
            .collect();
        Some(Self {
            default: team_setup::default_engine(&store),
            agents,
            clis,
        })
    }
}

impl ShellView {
    pub(crate) fn open_agents_page(&mut self, cx: &mut Context<Self>) {
        let clis = self
            .agents_page
            .as_ref()
            .map(|p| p.clis.clone())
            .unwrap_or_default();
        self.agents_page = AgentsPage::load(clis);
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async { crate::window::onboarding_detection_probe().cli_agents })
                .await;
            let _ = this.update(cx, |this, cx| {
                let clis = found.into_iter().map(|c| (c.id, c.label)).collect();
                this.agents_page = AgentsPage::load(clis);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn change_engines(
        &mut self,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&synapse_store::Store) -> Result<(), String>,
    ) {
        if let Some(store) = crate::window::open_store() {
            let _ = apply(&store);
        }
        let clis = self
            .agents_page
            .as_ref()
            .map(|p| p.clis.clone())
            .unwrap_or_default();
        self.agents_page = AgentsPage::load(clis);
        cx.notify();
    }
}

fn engine_label(engine: &Engine, clis: &[(String, String)]) -> String {
    match engine {
        Engine::Model => "Automatic".into(),
        Engine::Provider(id) => format!("{id} · default"),
        Engine::Api { provider, model } => format!("{provider} / {model}"),
        Engine::Cli { id, model } => {
            let name = clis
                .iter()
                .find(|(cid, _)| cid == id)
                .map_or_else(|| id.clone(), |(_, label)| label.clone());
            match model {
                Some(model) => format!("{name} · {model}"),
                None => name,
            }
        }
    }
}

/// One button showing the current engine; it opens the two-step picker.
fn engine_button(
    id: impl Into<SharedString>,
    label: String,
    target: EngineTarget,
    clis: Vec<(String, String)>,
    cx: &mut Context<ShellView>,
) -> Button {
    Button::new(id.into())
        .small()
        .outline()
        .label(label)
        .icon(crate::icons::CHEVRON_DOWN)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_engine_dialog(target.clone(), clis.clone(), window, cx)
        }))
}

pub(super) fn page(view: &ShellView, cx: &mut Context<ShellView>) -> AnyElement {
    let t = cx.theme().clone();
    let Some(page) = &view.agents_page else {
        return div()
            .typeset(TextRole::Body, cx)
            .text_color(t.colors.muted_foreground)
            .child("The agent database is unavailable.")
            .into_any_element();
    };
    let default_button = engine_button(
        "default-engine",
        engine_label(&page.default, &page.clis),
        EngineTarget::Default,
        page.clis.clone(),
        cx,
    );
    let default_card = super::setting_row::card(&t).child(
        h_flex()
            .items_center()
            .justify_between()
            .gap_4()
            .px_4()
            .py_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().typeset(TextRole::Label, cx).child("Default engine"))
                    .child(
                        div()
                            .typeset(TextRole::Caption, cx)
                            .text_color(t.colors.muted_foreground)
                            .child("Used by every agent that has no engine of its own."),
                    ),
            )
            .child(default_button),
    );

    let mut rows = super::setting_row::card(&t);
    for (n, (agent, own)) in page.agents.iter().enumerate() {
        rows = rows.child(agent_row(n, agent, own.as_ref(), page, cx));
    }
    if page.agents.is_empty() {
        rows = rows.child(
            div()
                .px_4()
                .py_3()
                .typeset(TextRole::Body, cx)
                .text_color(t.colors.muted_foreground)
                .child("No agents yet. Create a project and its team appears here."),
        );
    }
    v_flex()
        .gap_4()
        .child(default_card)
        .child(div().typeset(TextRole::Overline, cx).text_color(t.colors.muted_foreground).child("PER AGENT"))
        .child(rows)
        .into_any_element()
}

fn agent_row(
    n: usize,
    agent: &Agent,
    own: Option<&Engine>,
    page: &AgentsPage,
    cx: &mut Context<ShellView>,
) -> Div {
    let t = cx.theme().clone();
    let id = agent.id.clone();
    let current = match own {
        Some(engine) => engine_label(engine, &page.clis),
        None => format!("Default ({})", engine_label(&page.default, &page.clis)),
    };
    let buttons = engine_button(
        SharedString::from(format!("engine-{n}")),
        current,
        EngineTarget::Agent(id.clone()),
        page.clis.clone(),
        cx,
    );
    h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(t.colors.border)
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .min_w_0()
                .child(
                    div()
                        .id(("agent-avatar", n))
                        .cursor_pointer()
                        .child(crate::avatar_view::avatar_img(&agent.avatar, &agent.name, 32.0))
                        .on_click({
                            let id = id.clone();
                            cx.listener(move |this, _, window, cx| {
                                this.open_avatar_dialog(&id, window, cx)
                            })
                        }),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .child(div().typeset(TextRole::Label, cx).child(agent.name.clone()))
                        .child(
                            div()
                                .typeset(TextRole::Caption, cx)
                                .text_color(t.colors.muted_foreground)
                                .child(agent.role.clone()),
                        ),
                ),
        )
        .child(buttons)
}
