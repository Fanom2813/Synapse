//! Artifact panel state (see docs/UI.md Done/synthesis row,
//! docs/ARCHITECTURE.md §3, docs/DEEP-RESEARCH.md §5–6).
//!
//! Pure state over [`ArtifactStore`]: artifact list, selected version, diff
//! against the previous version, mirror status for the project folder, and
//! the "user edited the report file" import offer. The GPUI view renders
//! this; tests drive the state without GPUI.

use std::path::{Path, PathBuf};

use synapse_core::{
    import_user_edit, mirror_artifact, needs_import, read_mirrored_body, ArtifactError,
    ArtifactStore,
};

/// Owned artifact-panel state.
#[derive(Debug, Default)]
pub struct ArtifactPanel {
    selected_id: Option<String>,
    selected_version: Option<u32>,
    show_diff: bool,
    last_mirror: Option<PathBuf>,
    mirror_error: Option<String>,
    pending_import: Option<String>,
}

impl ArtifactPanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected_id.as_deref()
    }

    pub fn selected_version(&self) -> Option<u32> {
        self.selected_version
    }

    pub fn show_diff(&self) -> bool {
        self.show_diff
    }

    pub fn toggle_diff(&mut self) {
        self.show_diff = !self.show_diff;
    }

    pub fn last_mirror(&self) -> Option<&Path> {
        self.last_mirror.as_deref()
    }

    pub fn mirror_error(&self) -> Option<&str> {
        self.mirror_error.as_deref()
    }

    /// Import offer text when the mirrored file differs from the version.
    pub fn pending_import(&self) -> Option<&str> {
        self.pending_import.as_deref()
    }

    pub fn is_empty(&self, store: &ArtifactStore, project_id: &str) -> bool {
        store.list_for_project(project_id).is_empty()
    }

    pub fn empty_state(&self) -> (&'static str, &'static str) {
        ("No report yet", "Run the team to produce a synthesis")
    }

    pub fn select(&mut self, store: &ArtifactStore, artifact_id: &str) -> bool {
        let Some(a) = store.get(artifact_id) else {
            return false;
        };
        self.selected_id = Some(a.id.clone());
        self.selected_version = Some(a.current_version);
        self.pending_import = None;
        true
    }

    /// Select the first artifact of the project when nothing is selected.
    pub fn ensure_selection(&mut self, store: &ArtifactStore, project_id: &str) {
        let still_valid = self
            .selected_id
            .as_deref()
            .is_some_and(|id| store.get(id).is_some());
        if still_valid {
            return;
        }
        if let Some(first) = store.list_for_project(project_id).first() {
            self.selected_id = Some(first.id.clone());
            self.selected_version = Some(first.current_version);
        } else {
            self.selected_id = None;
            self.selected_version = None;
        }
        self.pending_import = None;
    }

    pub fn select_version(&mut self, store: &ArtifactStore, version: u32) -> bool {
        let Some(id) = self.selected_id.clone() else {
            return false;
        };
        if store.get_version(&id, version).is_none() {
            return false;
        }
        self.selected_version = Some(version);
        true
    }

    fn effective_version(&self, store: &ArtifactStore) -> Option<u32> {
        let id = self.selected_id.as_deref()?;
        let a = store.get(id)?;
        Some(self.selected_version.unwrap_or(a.current_version))
    }

    /// Body of the selected version (falls back to current).
    pub fn visible_body(&self, store: &ArtifactStore) -> Option<String> {
        let id = self.selected_id.as_deref()?;
        let v = self.effective_version(store)?;
        store.get_version(id, v).map(|ver| ver.body_md.clone())
    }

    pub fn visible_changelog(&self, store: &ArtifactStore) -> Option<String> {
        let id = self.selected_id.as_deref()?;
        let v = self.effective_version(store)?;
        store.get_version(id, v).map(|ver| ver.changelog.clone())
    }

    /// Version numbers for the version picker, oldest first.
    pub fn version_list(&self, store: &ArtifactStore) -> Vec<u32> {
        let Some(id) = self.selected_id.as_deref() else {
            return Vec::new();
        };
        store.versions_of(id).iter().map(|v| v.version).collect()
    }

    /// Unified diff of the selected version against its predecessor.
    /// Empty when viewing v1 or when nothing changed.
    pub fn diff_text(&self, store: &ArtifactStore) -> String {
        let (Some(id), Some(v)) = (self.selected_id.as_deref(), self.effective_version(store))
        else {
            return String::new();
        };
        if v <= 1 {
            return String::new();
        }
        store.diff(id, v - 1, v).unwrap_or_default()
    }

    /// Mirror the selected artifact into the project folder atomically.
    pub fn mirror_selected(
        &mut self,
        store: &ArtifactStore,
        folder: &Path,
    ) -> Result<PathBuf, ArtifactError> {
        let Some(id) = self.selected_id.clone() else {
            let e = ArtifactError::NotFound("no artifact selected".into());
            self.mirror_error = Some(e.to_string());
            return Err(e);
        };
        match mirror_artifact(folder, store, &id) {
            Ok(res) => {
                self.last_mirror = Some(res.report_path.clone());
                self.mirror_error = None;
                Ok(res.report_path)
            }
            Err(e) => {
                self.mirror_error = Some(e.to_string());
                Err(e)
            }
        }
    }

    /// Compare the mirrored file with the current version; when it differs,
    /// stash the edited body so the view can offer "Import as new version".
    pub fn check_user_edit(&mut self, store: &ArtifactStore, folder: &Path) -> bool {
        self.pending_import = None;
        let Some(id) = self.selected_id.as_deref() else {
            return false;
        };
        let (Some(artifact), Some(current)) =
            (store.get(id), store.current_body(id).map(str::to_string))
        else {
            return false;
        };
        let Some(mirrored) = read_mirrored_body(folder, artifact) else {
            return false;
        };
        if needs_import(&current, &mirrored) {
            // `read_mirrored_body` already stripped frontmatter.
            self.pending_import = Some(mirrored);
            true
        } else {
            false
        }
    }

    /// Accept the pending user edit as a new version; clears the offer.
    pub fn import_pending(
        &mut self,
        store: &mut ArtifactStore,
        created_by_run: &str,
        created_at: i64,
    ) -> Result<u32, ArtifactError> {
        let id = self
            .selected_id
            .clone()
            .ok_or_else(|| ArtifactError::NotFound("no artifact selected".into()))?;
        let body = self
            .pending_import
            .clone()
            .ok_or(ArtifactError::NoChanges(0))?;
        let v = import_user_edit(store, &id, &body, created_by_run, created_at)?;
        self.selected_version = Some(v.version);
        self.pending_import = None;
        Ok(v.version)
    }

    pub fn dismiss_pending(&mut self) {
        self.pending_import = None;
    }
}

#[cfg(test)]
mod artifact_panel_tests {
    use super::*;
    use synapse_core::{
        render_synthesis_report, ArtifactCitation, Dissent, ReportDraft, SourceEntry,
    };

    fn seeded_store() -> (ArtifactStore, String) {
        let mut store = ArtifactStore::new();
        let draft = ReportDraft {
            summary: "Summary.".into(),
            findings: vec![("Q?".into(), "Found it. [s1]".into())],
            analysis: "Analysis.".into(),
            dissent: vec![Dissent {
                author: "ivan".into(),
                statement: "Doubt.".into(),
                resolution: "Kept as caveat.".into(),
            }],
            open_questions: vec!["Next?".into()],
            sources: vec![SourceEntry {
                id: "s1".into(),
                title: "Source".into(),
                url: "https://example.com".into(),
            }],
        };
        let body = render_synthesis_report("Study", &draft);
        let a = store
            .create(
                "project_1",
                "Study",
                "report",
                &body,
                "Initial synthesis.",
                vec![ArtifactCitation {
                    source_id: "s1".into(),
                    quote: "Found it.".into(),
                }],
                "run-1",
                1,
            )
            .unwrap();
        store
            .new_version(
                &a.id,
                &format!("{body}\n\nRevision note.\n"),
                "Revised after critique.",
                Vec::new(),
                "run-1",
                2,
            )
            .unwrap();
        let id = a.id.clone();
        (store, id)
    }

    #[test]
    fn artifact_panel_shows_versions_and_diffs() {
        let (store, id) = seeded_store();
        let mut panel = ArtifactPanel::new();
        assert!(panel.is_empty(&store, "project_9"));
        assert!(!panel.is_empty(&store, "project_1"));
        assert!(panel.select(&store, &id));
        panel.ensure_selection(&store, "project_1");
        assert_eq!(panel.selected_id(), Some(id.as_str()));
        assert_eq!(panel.version_list(&store), vec![1, 2]);
        // Newest selected by default.
        assert_eq!(panel.selected_version(), Some(2));
        assert!(panel
            .visible_body(&store)
            .unwrap()
            .contains("Revision note."));
        assert_eq!(
            panel.visible_changelog(&store).as_deref(),
            Some("Revised after critique.")
        );
        // Diff of v2 vs v1 shows the added line.
        assert!(panel.diff_text(&store).contains("+Revision note."));
        // v1 has no predecessor: no diff.
        assert!(panel.select_version(&store, 1));
        assert!(panel.diff_text(&store).is_empty());
        assert!(!panel.select_version(&store, 99));
        assert!(!panel.select(&store, "artifact_999"));
        // Diff toggle for the view.
        assert!(!panel.show_diff());
        panel.toggle_diff();
        assert!(panel.show_diff());
    }

    #[test]
    fn artifact_panel_mirrors_reports_atomically() {
        let (store, id) = seeded_store();
        let mut panel = ArtifactPanel::new();
        assert!(panel.select(&store, &id));
        let dir = std::env::temp_dir().join(format!("synapse-panel-mirror-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = panel.mirror_selected(&store, &dir).expect("mirror");
        assert_eq!(path, dir.join("reports").join("study.md"));
        assert_eq!(panel.last_mirror(), Some(path.as_path()));
        assert!(panel.mirror_error().is_none());
        assert!(!path.with_extension("tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifact_panel_offers_to_import_user_edits_as_new_versions() {
        let (mut store, id) = seeded_store();
        let mut panel = ArtifactPanel::new();
        assert!(panel.select(&store, &id));
        let dir = std::env::temp_dir().join(format!("synapse-panel-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        panel.mirror_selected(&store, &dir).expect("mirror");
        // Untouched mirror: no offer.
        assert!(!panel.check_user_edit(&store, &dir));
        assert!(panel.pending_import().is_none());
        // User edits the mirrored report file.
        let report_path = dir.join("reports").join("study.md");
        let content = std::fs::read_to_string(&report_path).unwrap();
        std::fs::write(&report_path, format!("{content}\n\nUser correction.\n")).unwrap();
        assert!(panel.check_user_edit(&store, &dir));
        let offer = panel.pending_import().expect("offer").to_string();
        assert!(offer.contains("User correction."));
        let v = panel
            .import_pending(&mut store, "run-1", 3)
            .expect("import");
        assert_eq!(v, 3);
        assert!(panel.pending_import().is_none());
        assert_eq!(panel.selected_version(), Some(3));
        assert!(store
            .current_body(&id)
            .unwrap()
            .contains("User correction."));
        // Dismiss path: edit again, then dismiss without importing.
        std::fs::write(&report_path, format!("{content}\n\nAnother note.\n")).unwrap();
        assert!(panel.check_user_edit(&store, &dir));
        panel.dismiss_pending();
        assert!(panel.pending_import().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifact_panel_empty_and_error_states() {
        let store = ArtifactStore::new();
        let mut panel = ArtifactPanel::new();
        panel.ensure_selection(&store, "project_1");
        assert!(panel.selected_id().is_none());
        let (title, action) = panel.empty_state();
        assert!(!title.is_empty() && !action.is_empty());
        assert!(panel.visible_body(&store).is_none());
        assert!(panel.diff_text(&store).is_empty());
        assert!(panel.mirror_selected(&store, Path::new("/tmp")).is_err());
        assert!(panel.mirror_error().is_some());
    }
}
