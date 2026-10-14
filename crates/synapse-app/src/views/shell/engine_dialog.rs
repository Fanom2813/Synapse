//! The engine picker (Settings → Agents): one searchable popup. The first
//! list shows what can run an agent (the default model, installed CLIs, and
//! the providers you have switched on) with a button to add a provider;
//! picking a provider or a CLI opens its models. Lists only build the rows
//! in view.

use super::ShellView;
use crate::team_setup::Engine;
use crate::views::onboarding::provider_dialog::open_add_provider_dialog;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    h_flex, v_flex, ActiveTheme as _, Icon as KitIcon, Sizable as _, WindowExt as _,
};
use gpui_kit::{
    div, px, uniform_list, AppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window,
};
use std::sync::Arc;

const ROW_HEIGHT: f32 = 48.0;

/// Whose engine is being chosen.
#[derive(Clone)]
pub(crate) enum EngineTarget {
    /// The default for every agent without its own engine.
    Default,
    Agent(String),
    /// A provider's default model (Models page): straight to its models.
    ProviderDefault(String),
    /// A CLI's default model (Models page): straight to its models.
    CliDefault(String),
}

impl EngineTarget {
    /// Sets a default on the Models page instead of choosing an agent's engine.
    fn is_default_setting(&self) -> bool {
        matches!(self, Self::ProviderDefault(_) | Self::CliDefault(_))
    }
}

impl ShellView {
    pub(crate) fn open_engine_dialog(
        &mut self,
        target: EngineTarget,
        clis: Vec<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = cx.entity().downgrade();
        let title = if target.is_default_setting() { "Default model" } else { "Engine" };
        let form = cx.new(|cx| EngineForm::new(parent, target, clis, window, cx));
        window.open_dialog(cx, move |dialog, window, cx| {
            let top = ((window.viewport_size().height.as_f32() - 520.0) / 2.0).max(24.0);
            let t = cx.theme();
            dialog
                .title(div().typeset(TextRole::Title, cx).child(title))
                .w(px(440.0))
                .margin_top(px(top))
                .entrance(DialogEntrance::FadeSlide)
                .bg(t.colors.popover)
                .border_1()
                .border_color(t.colors.border)
                .shadow_lg()
                .child(form.clone())
        });
    }

    /// An engine was picked (`None` = follow the default again).
    pub(crate) fn engine_chosen(
        &mut self,
        target: EngineTarget,
        engine: Option<Engine>,
        cx: &mut Context<Self>,
    ) {
        use crate::team_setup::{
            set_agent_engine, set_cli_default, set_default_engine, set_provider_default,
        };
        self.change_engines(cx, |store| match (&target, &engine) {
            (EngineTarget::Default, Some(engine)) => set_default_engine(store, engine),
            (EngineTarget::Default, None) => Ok(()),
            (EngineTarget::Agent(id), engine) => set_agent_engine(store, id, engine.as_ref()),
            (EngineTarget::ProviderDefault(id), Some(Engine::Api { model, .. })) => {
                set_provider_default(store, id, model)
            }
            (EngineTarget::CliDefault(id), Some(Engine::Cli { model, .. })) => {
                set_cli_default(store, id, model.as_deref())
            }
            _ => Ok(()),
        });
        self.reload_models_page(cx);
    }
}

/// One selectable line in either list.
#[derive(Clone)]
enum Row {
    FollowDefault,
    DefaultModel,
    Cli { id: String, label: String },
    Provider { id: String, name: String },
    /// `logo` is the owning CLI or provider, so its mark shows on every model.
    Model { id: String, name: String, detail: String, logo: String },
    /// Follow the CLI's default model (Models page), or its own when none is set.
    CliDefault,
    /// Follow the provider's default model (Models page).
    ProviderDefault { id: String },
    /// Clear a CLI's default so it picks its own model.
    CliOwn,
}

impl Row {
    fn label(&self) -> String {
        match self {
            Self::FollowDefault => "Use the default engine".into(),
            Self::DefaultModel => "Automatic".into(),
            Self::CliDefault => "CLI default".into(),
            Self::ProviderDefault { .. } => "Provider default".into(),
            Self::CliOwn => "Let the CLI choose".into(),
            Self::Cli { label, .. } => label.clone(),
            Self::Provider { name, .. } | Self::Model { name, .. } => name.clone(),
        }
    }

    fn hint(&self) -> String {
        match self {
            Self::Model { detail, .. } => detail.clone(),
            other => other.fixed_hint().to_string(),
        }
    }

    fn fixed_hint(&self) -> &'static str {
        match self {
            Self::FollowDefault => "Changes whenever the default changes",
            Self::DefaultModel => "The model chosen when you first connected a provider",
            Self::CliDefault => "Its default from the Models page, else the CLI's own",
            Self::ProviderDefault { .. } => "Its default from the Models page",
            Self::CliOwn => "The CLI keeps using its own model",
            Self::Cli { .. } => "Coding CLI on this computer",
            Self::Provider { .. } => "Pick one of its models",
            Self::Model { .. } => "",
        }
    }

    /// The logo tile: CLI glyph, provider letter, or an icon.
    fn glyph(&self, app: &gpui_kit::App) -> gpui_kit::Div {
        match self {
            Self::Cli { id, label } => crate::cli_logo::tile(id, label, 28.0, app),
            Self::Provider { id, name } => crate::cli_logo::tile(id, name, 28.0, app),
            Self::Model { name, logo, .. } => crate::cli_logo::tile(logo, name, 28.0, app),
            Self::FollowDefault
            | Self::DefaultModel
            | Self::CliDefault
            | Self::CliOwn
            | Self::ProviderDefault { .. } => {
                let icon = match self {
                    Self::FollowDefault => crate::icons::BACK,
                    _ => crate::icons::TAB_TEAM,
                };
                div()
                    .size(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(KitIcon::new(icon).small())
            }
        }
    }
}

/// What the second list belongs to.
enum Owner {
    Provider { id: String },
    Cli { id: String },
}

enum Step {
    Source,
    Models { owner: Owner, title: String },
}

struct EngineForm {
    parent: WeakEntity<ShellView>,
    target: EngineTarget,
    step: Step,
    query: Entity<InputState>,
    clis: Vec<(String, String)>,
    /// Every row of the current list; `rows` is the searched subset.
    all: Vec<Row>,
    /// The first list, kept so Back does not rebuild it.
    source_rows: Vec<Row>,
    rows: Arc<Vec<Row>>,
    /// Setting a default from the Models page: no first list, no "follow" rows.
    locked: bool,
    /// Shown under the list while a CLI is asked for its models.
    status: Option<String>,
    _sub: Subscription,
}

fn source_rows(target: &EngineTarget, clis: &[(String, String)]) -> Vec<Row> {
    let mut all = Vec::new();
    if matches!(target, EngineTarget::Agent(_)) {
        all.push(Row::FollowDefault);
    }
    all.push(Row::DefaultModel);
    all.extend(clis.iter().map(|(id, label)| Row::Cli { id: id.clone(), label: label.clone() }));
    if let Some(store) = crate::window::open_store() {
        all.extend(
            crate::provider_setup::list_added(&store)
                .into_iter()
                .map(|p| Row::Provider { id: p.id, name: p.name }),
        );
    }
    all
}

impl EngineForm {
    fn new(
        parent: WeakEntity<ShellView>,
        target: EngineTarget,
        clis: Vec<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let sub = cx.subscribe_in(&query, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.refilter(cx);
            }
        });
        query.update(cx, |input, cx| input.focus(window, cx));
        let all = source_rows(&target, &clis);
        let locked = target.is_default_setting();
        let start = match &target {
            EngineTarget::ProviderDefault(id) => Some((id.clone(), true)),
            EngineTarget::CliDefault(id) => Some((id.clone(), false)),
            _ => None,
        };
        let mut form = Self {
            parent,
            target,
            step: Step::Source,
            query,
            clis,
            rows: Arc::new(all.clone()),
            source_rows: all.clone(),
            all,
            locked,
            status: None,
            _sub: sub,
        };
        if let Some((id, is_provider)) = start {
            if is_provider {
                let name = crate::provider_setup::list_added(&crate::window::open_store().expect("store"))
                    .into_iter()
                    .find(|p| p.id == id)
                    .map_or_else(|| id.clone(), |p| p.name);
                form.open_provider(id, name, window, cx);
            } else {
                let label = form
                    .clis
                    .iter()
                    .find(|(cid, _)| *cid == id)
                    .map_or_else(|| id.clone(), |(_, l)| l.clone());
                form.open_cli(id, label, window, cx);
            }
        }
        form
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let q = self.query.read(cx).value().trim().to_lowercase();
        self.rows = Arc::new(
            self.all
                .iter()
                .filter(|r| {
                    q.is_empty()
                        || r.label().to_lowercase().contains(&q)
                        || matches!(r, Row::Model { id, .. } if id.to_lowercase().contains(&q))
                })
                .cloned()
                .collect(),
        );
        cx.notify();
    }

    fn show_list(&mut self, step: Step, rows: Vec<Row>, window: &mut Window, cx: &mut Context<Self>) {
        self.step = step;
        self.all = rows;
        self.query.update(cx, |input, cx| input.set_value("", window, cx));
        self.refilter(cx);
    }

    /// A provider was added in the nested popup: list it.
    fn providers_changed(&mut self, cx: &mut Context<Self>) {
        self.source_rows = source_rows(&self.target, &self.clis);
        if matches!(self.step, Step::Source) {
            self.all = self.source_rows.clone();
            self.refilter(cx);
        }
    }

    fn open_provider(&mut self, id: String, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let store = crate::window::open_store();
        let rows = crate::custom_providers::catalog_with_custom(store.as_ref())
            .and_then(|c| {
                c.models(&id).ok().map(|models| {
                    models
                        .into_iter()
                        .map(|m| Row::Model {
                            id: m.id.clone(),
                            name: if m.name.is_empty() { m.id.clone() } else { m.name.clone() },
                            detail: crate::cli_models::model_detail(m),
                            logo: id.clone(),
                        })
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        let mut rows = rows;
        if !self.locked {
            rows.insert(0, Row::ProviderDefault { id: id.clone() });
        }
        self.show_list(Step::Models { owner: Owner::Provider { id }, title: name }, rows, window, cx);
    }

    /// Models a CLI can run. OpenCode lists its own by running it, which
    /// takes a moment, so that list fills in when it is ready.
    fn open_cli(&mut self, id: String, label: String, window: &mut Window, cx: &mut Context<Self>) {
        let store = crate::window::open_store();
        let catalog = crate::custom_providers::catalog_with_custom(store.as_ref());
        let found = catalog
            .map(|c| crate::cli_models::catalog_models(&id, &c))
            .unwrap_or_default();
        let by_command = crate::cli_models::lists_by_command(&id);
        if found.is_empty() && !by_command && !self.locked {
            // This CLI picks its own model: nothing more to ask.
            self.apply(Some(Engine::Cli { id, model: None }), window, cx);
            return;
        }
        let mut rows = vec![if self.locked { Row::CliOwn } else { Row::CliDefault }];
        rows.extend(found.into_iter().map(|(mid, name, detail)| Row::Model {
            id: mid,
            name,
            detail,
            logo: id.clone(),
        }));
        self.show_list(Step::Models { owner: Owner::Cli { id: id.clone() }, title: label }, rows, window, cx);
        if by_command {
            self.status = Some("Asking OpenCode for its models...".into());
            cx.spawn(async move |this, cx| {
                let models = cx
                    .background_executor()
                    .spawn(async { crate::cli_models::opencode_models() })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.status = models.is_empty().then(|| "OpenCode did not list any models.".to_string());
                    if let Step::Models { owner: Owner::Cli { id }, .. } = &this.step {
                        if id == "opencode" {
                            this.all.extend(models.into_iter().map(|(mid, name, detail)| Row::Model {
                                id: mid,
                                name,
                                detail,
                                logo: "opencode".to_string(),
                            }));
                            this.refilter(cx);
                        }
                    }
                });
            })
            .detach();
        } else {
            self.status = None;
        }
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.status = None;
        let rows = self.source_rows.clone();
        self.show_list(Step::Source, rows, window, cx);
    }

    fn apply(&mut self, engine: Option<Engine>, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.target.clone();
        let _ = self.parent.update(cx, |view, cx| view.engine_chosen(target, engine, cx));
        window.close_dialog(cx);
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        match row {
            Row::Provider { id, name } => self.open_provider(id, name, window, cx),
            Row::Cli { id, label } => self.open_cli(id, label, window, cx),
            Row::FollowDefault => self.apply(None, window, cx),
            Row::DefaultModel => self.apply(Some(Engine::Model), window, cx),
            Row::ProviderDefault { id } => self.apply(Some(Engine::Provider(id)), window, cx),
            Row::CliDefault | Row::CliOwn | Row::Model { .. } => {
                let model = match &row {
                    Row::Model { id, .. } => Some(id.clone()),
                    _ => None,
                };
                let engine = match &self.step {
                    Step::Models { owner: Owner::Cli { id }, .. } => {
                        Engine::Cli { id: id.clone(), model }
                    }
                    Step::Models { owner: Owner::Provider { id }, .. } => match model {
                        Some(model) => Engine::Api { provider: id.clone(), model },
                        None => return,
                    },
                    Step::Source => return,
                };
                self.apply(Some(engine), window, cx);
            }
        }
    }

    fn add_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        open_add_provider_dialog(
            window,
            cx,
            std::rc::Rc::new(move |_, app| {
                let _ = this.update(app, |form, cx| form.providers_changed(cx));
            }),
        );
    }
}

impl Render for EngineForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut form = v_flex().gap_3().pt_3();
        form = match &self.step {
            Step::Source => form.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.query)))
                    .child(
                        Button::new("engine-add-provider")
                            .small()
                            .outline()
                            .icon(crate::icons::PLUS)
                            .label("Add provider")
                            .on_click(cx.listener(|this, _, window, cx| this.add_provider(window, cx))),
                    ),
            ),
            Step::Models { title, .. } => form
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .children((!self.locked).then(|| {
                            Button::new("engine-back")
                                .ghost()
                                .small()
                                .icon(crate::icons::BACK)
                                .tooltip("Back")
                                .on_click(cx.listener(|this, _, window, cx| this.back(window, cx)))
                        }))
                        .child(div().typeset(TextRole::Label, cx).child(title.clone())),
                )
                .child(Input::new(&self.query)),
        };
        if self.rows.is_empty() {
            form = form.child(
                div()
                    .typeset(TextRole::Body, cx)
                    .text_color(t.colors.muted_foreground)
                    .child("Nothing matches."),
            );
        } else {
            let (rows, this) = (self.rows.clone(), cx.entity());
            form = form.child(
                uniform_list("engine-list", rows.len(), move |range, _, app| {
                    let t = app.theme().clone();
                    let hover = t.colors.list_hover;
                    range
                        .filter_map(|i| {
                            let row = rows.get(i)?;
                            let this = this.clone();
                            Some(
                                h_flex()
                                    .id(("engine-row", i))
                                    .h(px(ROW_HEIGHT))
                                    .items_center()
                                    .gap_3()
                                    .px_3()
                                    .rounded(t.radius)
                                    .cursor_pointer()
                                    .hover(move |r| r.bg(hover))
                                    .on_click(move |_, window, app| {
                                        this.update(app, |form, cx| form.choose(i, window, cx));
                                    })
                                    .child(row.glyph(app))
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .justify_center()
                                            .child(div().typeset(TextRole::Label, app).truncate().child(row.label()))
                                            .children((!row.hint().is_empty()).then(|| {
                                                div()
                                                    .typeset(TextRole::Caption, app)
                                                    .text_color(t.colors.muted_foreground)
                                                    .truncate()
                                                    .child(row.hint())
                                            })),
                                    ),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .h(px(320.0)),
            );
        }
        if let Some(status) = &self.status {
            form = form.child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(t.colors.muted_foreground)
                    .child(status.clone()),
            );
        }
        form.child(
            h_flex().justify_end().child(
                Button::new("engine-cancel")
                    .label("Cancel")
                    .ghost()
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            ),
        )
    }
}
