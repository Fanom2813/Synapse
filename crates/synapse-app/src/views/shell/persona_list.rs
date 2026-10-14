//! The curated agent library as a searchable, virtualised list that loads ten
//! more rows each time the end is reached. It starts with the bundled starter
//! team and merges the open collections as they load (downloading them on
//! first use). Shared by the "Add agent" popup and Settings → Agents; each
//! supplies how a row looks and what clicking it does.

use crate::persona_fetch;
use crate::persona_library::search;
use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{v_flex, ActiveTheme as _};
use gpui_kit::{
    div, px, uniform_list, AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window,
};
use std::rc::Rc;
use std::sync::Arc;
use synapse_personas::PersonaDraft;

/// Fixed row height: the list only builds the rows that are on screen.
pub(crate) const ROW_HEIGHT: f32 = 52.0;
/// Rows added each time the list is scrolled to its end.
const PAGE: usize = 10;

/// Builds one row for the persona at the given library index.
pub(crate) type RowFn = Rc<dyn Fn(usize, &PersonaDraft, &App) -> AnyElement>;

pub(crate) struct PersonaList {
    query: Entity<InputState>,
    entries: Arc<Vec<PersonaDraft>>,
    /// Indexes into `entries` that match the search, recomputed on change only.
    matches: Arc<Vec<usize>>,
    /// How many of the matches are in the list so far; grows by `PAGE` at the end.
    shown: usize,
    /// Progress of the first-use download of the open collections.
    status: Option<String>,
    height: f32,
    row: RowFn,
    _sub: Subscription,
}

impl PersonaList {
    pub(crate) fn new(height: f32, row: RowFn, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search agents: researcher, designer, security...")
        });
        let sub = cx.subscribe_in(&query, window, |this, _, _: &InputEvent, _, cx| {
            this.refilter(cx);
        });
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
            query,
            entries: Arc::new(entries),
            matches: Arc::new(matches),
            shown: PAGE,
            status: None,
            height,
            row,
            _sub: sub,
        }
    }

    /// Put the cursor in the search box.
    pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |input, cx| input.focus(window, cx));
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
}

impl Render for PersonaList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let mut out = v_flex().gap_3().child(Input::new(&self.query));
        if self.matches.is_empty() {
            out = out.child(
                div()
                    .typeset(TextRole::Body, cx)
                    .text_color(t.colors.muted_foreground)
                    .child("No agent matches that search."),
            );
        } else {
            // Virtualised: only the rows in view are built, however long the list is.
            let (entries, matches, row) = (self.entries.clone(), self.matches.clone(), self.row.clone());
            let this = cx.entity();
            let count = self.shown.min(matches.len());
            let more = count < matches.len();
            let list = uniform_list("persona-list", count, move |range, _, app| {
                // Reached the last loaded rows: load the next page after this frame.
                if more && range.end + 2 >= count {
                    let list = this.clone();
                    app.defer(move |app| {
                        list.update(app, |list, cx| {
                            list.shown += PAGE;
                            cx.notify();
                        });
                    });
                }
                range
                    .filter_map(|n| {
                        let index = *matches.get(n)?;
                        Some(row(index, entries.get(index)?, app))
                    })
                    .collect::<Vec<_>>()
            })
            .h(px(self.height));
            out = out.child(list);
        }
        if let Some(status) = &self.status {
            out = out.child(
                div()
                    .typeset(TextRole::Caption, cx)
                    .text_color(t.colors.muted_foreground)
                    .child(status.clone()),
            );
        }
        out
    }
}
