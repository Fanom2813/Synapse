//! Settings → Agents: the default engine for every agent, and the curated
//! agent library (same list as the Add-agent popup) where each agent can get
//! its own engine and avatar. An engine is a provider or coding CLI default
//! (chosen under Models) or a specific model.

use crate::team_setup::{self, Engine};
use crate::theme::TextRole;
use crate::typography::Typography as _;
use crate::views::shell::engine_dialog::EngineTarget;
use crate::views::shell::persona_list::{PersonaList, ROW_HEIGHT};
use crate::views::shell::ShellView;
use std::collections::HashMap;
use std::rc::Rc;
use synapse_personas::PersonaDraft;
use gpui_kit::component::button::Button;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Sizable as _};
use gpui_kit::{
    div, px, AnyElement, App, AppContext as _, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
};

/// What the page shows, read from the store when it opens.
pub(crate) struct AgentsPage {
    default: Engine,
    /// Library agents that already exist, by agent id: avatar JSON and own engine.
    known: HashMap<String, (String, Option<Engine>)>,
    /// Installed CLIs as `(id, label)`, found on a worker thread.
    clis: Vec<(String, String)>,
}

impl AgentsPage {
    fn load(clis: Vec<(String, String)>) -> Option<Self> {
        let store = crate::window::open_store()?;
        let known = store
            .list_agents()
            .unwrap_or_default()
            .into_iter()
            .map(|a| {
                let own = team_setup::agent_override(&store, &a.id);
                (a.id, (a.avatar, own))
            })
            .collect();
        Some(Self {
            default: team_setup::default_engine(&store),
            known,
            clis,
        })
    }
}

impl ShellView {
    pub(crate) fn open_agents_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agents_list.is_none() {
            let shell = cx.weak_entity();
            let row = Rc::new(move |index: usize, persona: &PersonaDraft, app: &App| {
                library_row(index, persona, shell.clone(), app)
            });
            self.agents_list = Some(cx.new(|cx| PersonaList::new(480.0, row, window, cx)));
        }
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

/// One button showing the current engine; `open` shows the two-step picker.
fn engine_button(
    id: impl Into<SharedString>,
    label: String,
    open: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id.into())
        .small()
        .outline()
        .label(label)
        .icon(crate::icons::CHEVRON_DOWN)
        .on_click(move |_, window, app| open(window, app))
}

impl ShellView {
    /// Make sure the persona has an agent in the store, then run `then` with its id.
    fn with_library_agent(
        &mut self,
        persona: &PersonaDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, String, &mut Window, &mut Context<Self>),
    ) {
        let Some(store) = crate::window::open_store() else {
            return;
        };
        if let Ok(agent) = team_setup::ensure_agent(&store, persona) {
            self.change_engines(cx, |_| Ok(()));
            then(self, agent.id, window, cx);
        }
    }

    fn clis(&self) -> Vec<(String, String)> {
        self.agents_page.as_ref().map(|p| p.clis.clone()).unwrap_or_default()
    }
}

pub(super) fn page(view: &ShellView, cx: &mut Context<ShellView>) -> AnyElement {
    let t = cx.theme().clone();
    let (Some(page), Some(list)) = (&view.agents_page, &view.agents_list) else {
        return div()
            .typeset(TextRole::Body, cx)
            .text_color(t.colors.muted_foreground)
            .child("The agent database is unavailable.")
            .into_any_element();
    };
    let shell = cx.weak_entity();
    let clis = page.clis.clone();
    let default_button = engine_button(
        "default-engine",
        engine_label(&page.default, &page.clis),
        move |window, app| {
            let clis = clis.clone();
            let _ = shell.update(app, |this, cx| {
                this.open_engine_dialog(EngineTarget::Default, clis, window, cx)
            });
        },
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
    v_flex()
        .gap_4()
        .child(default_card)
        .child(
            div()
                .typeset(TextRole::Overline, cx)
                .text_color(t.colors.muted_foreground)
                .child("AGENT LIBRARY"),
        )
        .child(list.clone())
        .into_any_element()
}

/// One library agent: avatar (click to change it), name, role and its engine.
fn library_row(
    index: usize,
    persona: &PersonaDraft,
    shell: gpui_kit::WeakEntity<ShellView>,
    app: &App,
) -> AnyElement {
    let t = app.theme().clone();
    let id = team_setup::agent_id(&persona.name);
    let state = shell.upgrade().and_then(|s| s.read(app).agents_page.as_ref().map(|p| {
        let known = p.known.get(&id);
        let avatar = known.map(|(a, _)| a.clone());
        let label = match known.and_then(|(_, own)| own.as_ref()) {
            Some(engine) => engine_label(engine, &p.clis),
            None => format!("Default ({})", engine_label(&p.default, &p.clis)),
        };
        (avatar, label)
    }));
    let (avatar, label) = state.unwrap_or_default();
    let avatar = avatar.unwrap_or_else(|| crate::avatar::AvatarSpec::from_name(&persona.name).to_json());
    let role = persona.role.clone().unwrap_or_else(|| persona.description.clone());
    let (for_avatar, for_engine) = (persona.clone(), persona.clone());
    let (shell_avatar, shell_engine) = (shell.clone(), shell);
    h_flex()
        .w_full()
        .min_w_0()
        .h(px(ROW_HEIGHT))
        .items_center()
        .justify_between()
        .gap_4()
        .px_3()
        .child(
            h_flex()
                .flex_1()
                .items_center()
                .gap_3()
                .min_w_0()
                .overflow_hidden()
                .child(
                    div()
                        .id(("library-avatar", index))
                        .flex_none()
                        .cursor_pointer()
                        .child(crate::avatar_view::avatar_img(&avatar, &persona.name, 32.0))
                        .on_click(move |_, window, app| {
                            let persona = for_avatar.clone();
                            let _ = shell_avatar.update(app, |this, cx| {
                                this.with_library_agent(&persona, window, cx, |this, id, window, cx| {
                                    this.open_avatar_dialog(&id, window, cx)
                                })
                            });
                        }),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(div().typeset(TextRole::Label, app).truncate().child(persona.name.clone()))
                        .child(
                            div()
                                .typeset(TextRole::Caption, app)
                                .text_color(t.colors.muted_foreground)
                                .w_full()
                                .truncate()
                                .child(role),
                        ),
                ),
        )
        .child(div().flex_none().child(engine_button(
            SharedString::from(format!("library-engine-{index}")),
            label,
            move |window, app| {
                let persona = for_engine.clone();
                let _ = shell_engine.update(app, |this, cx| {
                    this.with_library_agent(&persona, window, cx, |this, id, window, cx| {
                        let clis = this.clis();
                        this.open_engine_dialog(EngineTarget::Agent(id), clis, window, cx)
                    })
                });
            },
        )))
        .into_any_element()
}
