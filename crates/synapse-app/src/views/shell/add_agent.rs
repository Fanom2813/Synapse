//! The "Add agent" popup: search the persona library (the starter team plus
//! the bundled open collection) and add one to the open project's team. What
//! runs the agent is set in Settings → Agents, not here.

use super::ShellView;
use crate::persona_fetch;
use crate::persona_library::search;
use std::sync::Arc;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, WindowExt as _};
use gpui_kit::{
    div, px, uniform_list, AppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window,
};
use synapse_personas::PersonaDraft;

/// Fixed row height: the list only builds the rows that are on screen.
const ROW_HEIGHT: f32 = 52.0;
/// Rows added each time the list is scrolled to its end.
const PAGE: usize = 10;

impl ShellView {
    pub(super) fn open_add_agent_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.shell.open_project_id.clone() else {
            return;
        };
        let parent = cx.entity().downgrade();
        let form = cx.new(|cx| AddAgentForm::new(parent, project, window, cx));
        window.open_dialog(cx, move |dialog, window, cx| {
            let top = ((window.viewport_size().height.as_f32() - 520.0) / 2.0).max(24.0);
            let t = cx.theme();
            dialog
                .title(div().typeset(TextRole::Title, cx).child("Add agent"))
                .w(px(480.0))
                .margin_top(px(top))
                .entrance(DialogEntrance::FadeSlide)
                .bg(t.colors.popover)
                .border_1()
                .border_color(t.colors.border)
                .shadow_lg()
                .child(form.clone())
        });
    }
}

struct AddAgentForm {
    parent: WeakEntity<ShellView>,
    project: String,
    query: Entity<InputState>,
    entries: Arc<Vec<PersonaDraft>>,
    /// Indexes into `entries` that match the search, recomputed on change only.
    matches: Arc<Vec<usize>>,
    /// How many of the matches are in the list so far; grows by `PAGE` at the end.
    shown: usize,
    /// Progress of the first-use download of the open collections.
    status: Option<String>,
    note: Option<String>,
    _sub: Subscription,
}

impl AddAgentForm {
    fn new(
        parent: WeakEntity<ShellView>,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search agents: researcher, designer, security...")
        });
        let sub = cx.subscribe_in(&query, window, |this, _, _: &InputEvent, _, cx| {
            this.refilter(cx);
        });
        query.update(cx, |input, cx| input.focus(window, cx));
        // Start with the bundled starter team; everything else loads off the UI thread.
        let entries = synapse_personas::starter_personas();
        let matches = (0..entries.len()).collect();
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async { persona_fetch::load_saved() })
                .await;
            let _ = this.update(cx, |this, cx| this.merge(saved, cx));
            let missing = persona_fetch::missing();
            if missing.is_empty() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.status = Some("Downloading open agent collections...".into());
                cx.notify();
            });
            let mut failed = false;
            for collection in missing {
                let loaded = match persona_fetch::download(collection).await {
                    Ok(Ok(files)) => {
                        let repo = collection.repo;
                        Some(
                            cx.background_executor()
                                .spawn(async move {
                                    persona_fetch::save_cached(repo, &files);
                                    persona_fetch::parse(repo, files)
                                })
                                .await,
                        )
                    }
                    _ => None,
                };
                failed |= loaded.is_none();
                if let Some(found) = loaded {
                    let _ = this.update(cx, |this, cx| this.merge(found, cx));
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.status = failed.then(|| {
                    "Could not reach GitHub for some collections; they load next time you are online."
                        .to_string()
                });
                cx.notify();
            });
        })
        .detach();
        Self {
            parent,
            project,
            query,
            entries: Arc::new(entries),
            matches: Arc::new(matches),
            shown: PAGE,
            status: None,
            note: None,
            _sub: sub,
        }
    }

    /// Add newly loaded personas (one per name, sorted) and refresh the matches.
    fn merge(&mut self, found: Vec<PersonaDraft>, cx: &mut Context<Self>) {
        let mut all = self.entries.as_ref().clone();
        let mut seen: std::collections::HashSet<String> =
            all.iter().map(|p| p.name.to_lowercase()).collect();
        all.extend(found.into_iter().filter(|p| seen.insert(p.name.to_lowercase())));
        all.sort_by(|a, b| a.name.cmp(&b.name));
        self.entries = Arc::new(all);
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        let found = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, p)| !search(std::slice::from_ref(*p), &query, None, None).is_empty())
            .map(|(i, _)| i)
            .collect();
        self.matches = Arc::new(found);
        self.shown = PAGE;
        cx.notify();
    }

    fn add(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(persona) = self.entries.get(index) else {
            return;
        };
        let result = crate::window::open_store()
            .ok_or_else(|| "The project database is unavailable.".to_string())
            .and_then(|store| crate::team_setup::add_persona(&store, &self.project, persona));
        match result {
            Ok(_) => {
                let _ = self.parent.update(cx, |view, cx| view.refresh_team(cx));
                window.close_dialog(cx);
            }
            Err(e) => {
                self.note = Some(e);
                cx.notify();
            }
        }
    }
}

impl Render for AddAgentForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut form = v_flex().gap_3().pt_3().child(Input::new(&self.query));
        if self.matches.is_empty() {
            form = form.child(
                div()
                    .typeset(TextRole::Body, cx)
                    .text_color(t.colors.muted_foreground)
                    .child("No agent matches that search."),
            );
        } else {
            // Virtualised: only the rows in view are built, however long the list is.
            let (entries, matches, this) = (self.entries.clone(), self.matches.clone(), cx.entity());
            let count = self.shown.min(matches.len());
            let more = count < matches.len();
            let list = uniform_list("persona-list", count, move |range, _, app| {
                let t = app.theme().clone();
                // Reached the last loaded rows: load the next page after this frame.
                if more && range.end + 2 >= count {
                    let form = this.clone();
                    app.defer(move |app| {
                        form.update(app, |form, cx| {
                            form.shown += PAGE;
                            cx.notify();
                        });
                    });
                }
                range
                    .filter_map(|row| {
                        let index = *matches.get(row)?;
                        let persona = entries.get(index)?;
                        Some(persona_row(index, persona, &t, &this, app))
                    })
                    .collect::<Vec<_>>()
            })
            .h(px(360.0));
            form = form.child(list);
        }
        if let Some(status) = &self.status {
            form = form.child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(t.colors.muted_foreground)
                    .child(status.clone()),
            );
        }
        if let Some(note) = &self.note {
            form = form.child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(t.colors.danger)
                    .child(note.clone()),
            );
        }
        form.child(
            h_flex().justify_end().child(
                Button::new("add-agent-cancel")
                    .label("Cancel")
                    .ghost()
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            ),
        )
    }
}

fn persona_row(
    index: usize,
    persona: &PersonaDraft,
    t: &gpui_kit::component::Theme,
    form: &Entity<AddAgentForm>,
    app: &gpui_kit::App,
) -> impl IntoElement {
    let hover = t.colors.list_hover;
    let form = form.clone();
    let blurb = persona.vibe.clone().unwrap_or_else(|| persona.description.clone());
    h_flex()
        .id(("persona-row", index))
        .h(px(ROW_HEIGHT))
        .items_center()
        .gap_3()
        .px_3()
        .rounded(t.radius)
        .cursor_pointer()
        .hover(move |row| row.bg(hover))
        .on_click(move |_, window, app| {
            form.update(app, |this, cx| this.add(index, window, cx));
        })
        .child(
            div()
                .w(px(24.0))
                .flex_none()
                .child(persona.emoji.clone().unwrap_or_else(|| "•".into())),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().typeset(TextRole::Label, app).child(persona.name.clone()))
                        .child(
                            div()
                                .typeset(TextRole::Caption, app)
                                .text_color(t.colors.muted_foreground)
                                .child(persona.category.clone().unwrap_or_default()),
                        ),
                )
                .child(
                    div()
                        .typeset(TextRole::Caption, app)
                        .text_color(t.colors.muted_foreground)
                        .truncate()
                        .child(blurb),
                ),
        )
}
