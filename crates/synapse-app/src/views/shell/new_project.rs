//! New project: no form. The project is made at once and named by a model
//! from the first prompt (`project_goal`).

use super::ShellView;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{Context, Window};

impl ShellView {
    pub(super) fn create_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let made = synapse_store::Store::open(crate::app_dirs::db_path())
            .map_err(|e| format!("The project database could not be opened: {e}"))
            .and_then(|store| crate::project_setup::create_project(&store));
        match made {
            Ok(made) => self.project_created(made, window, cx),
            Err(e) => window.push_notification(e, cx),
        }
    }
}
