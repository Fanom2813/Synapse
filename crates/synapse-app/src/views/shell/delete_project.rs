//! Delete a project and everything in it, after a confirmation. Destructive
//! and permanent, so the dialog names the project and says what goes.

use super::ShellView;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{Context, Window};

impl ShellView {
    pub(super) fn open_delete_project_dialog(
        &mut self,
        id: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (parent, id) = (parent.clone(), id.clone());
            alert
                .title(format!("Delete \"{name}\"?"))
                .description(
                    "Its chat, sources, artifacts, memories and team assignments are deleted \
                     for good. The agents themselves stay, and so do your files. This cannot \
                     be undone.",
                )
                .confirm()
                .ok_text("Delete project")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .on_ok(move |_, _, app| {
                    let _ = parent.update(app, |view, cx| view.delete_project(&id, cx));
                    true
                })
        });
    }

    /// Delete from the store, then leave the app showing something valid: the
    /// next project, or the plain chat when none is left.
    fn delete_project(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(store) = crate::window::open_store() else {
            return;
        };
        if store.delete_project(id).is_err() {
            return;
        }
        if self.shell.forget_project(id) {
            match self.shell.open_project_id.clone() {
                Some(next) => self.show_project(&next, None, cx),
                None => {
                    self.team.clear();
                    self.track_goal(cx);
                    let conversation = crate::window::ensure_chat_conversation(&store);
                    self.chat.update(cx, |chat, cx| {
                        if let (Some(conversation), Some(store)) = (conversation, chat.persist_store()) {
                            chat.open_conversation(&store, &conversation);
                        }
                        cx.notify();
                    });
                }
            }
        }
        cx.notify();
    }
}
