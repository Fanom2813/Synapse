//! Attachment composer + chip UI state (see docs/TEAM-PROTOCOL.md §3a).
//!
//! Pure state over [`synapse_core::AttachmentRef`]: pending picks, image
//! previews, file chips (open/reveal), limits. The GPUI view renders this;
//! tests drive the state without GPUI.

use synapse_core::{AttachmentRef, MAX_ATTACHMENTS_PER_MESSAGE, MAX_ATTACHMENT_BYTES};

/// One pending attachment in the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAttachment {
    pub attachment: AttachmentRef,
    pub size_bytes: usize,
    pub is_image: bool,
}

impl PendingAttachment {
    pub fn new(id: &str, name: &str, mime: &str, size_bytes: usize) -> Self {
        Self {
            attachment: AttachmentRef {
                id: id.to_string(),
                name: name.to_string(),
                mime: mime.to_string(),
            },
            size_bytes,
            is_image: mime.starts_with("image/"),
        }
    }

    /// Preview affordance: thumbnails for images, file chips otherwise.
    pub fn preview_kind(&self) -> &'static str {
        if self.is_image {
            "thumbnail"
        } else {
            "file-chip"
        }
    }

    /// Chip label, e.g. `report.pdf (12 KB)`.
    pub fn chip_label(&self) -> String {
        format!("{} ({})", self.attachment.name, human_size(self.size_bytes))
    }

    /// Blob path the chip opens (`ARCHITECTURE.md` §3).
    pub fn open_path(&self) -> String {
        format!(".synapse/blobs/{}", self.attachment.id)
    }
}

fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Composer attachment tray state.
#[derive(Debug, Default)]
pub struct AttachmentTray {
    pending: Vec<PendingAttachment>,
    last_error: Option<String>,
}

impl AttachmentTray {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pending(&self) -> &[PendingAttachment] {
        &self.pending
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Attach a stored blob by reference. Enforces per-message/size limits.
    pub fn attach(&mut self, file: PendingAttachment) -> bool {
        if file.size_bytes > MAX_ATTACHMENT_BYTES {
            self.last_error = Some(format!("{} is too large (max 20 MB)", file.attachment.name));
            return false;
        }
        if self.pending.len() >= MAX_ATTACHMENTS_PER_MESSAGE {
            self.last_error = Some(format!(
                "at most {MAX_ATTACHMENTS_PER_MESSAGE} attachments per message"
            ));
            return false;
        }
        if self
            .pending
            .iter()
            .any(|p| p.attachment.id == file.attachment.id)
        {
            return true; // already attached: deduped like the store
        }
        self.pending.push(file);
        self.last_error = None;
        true
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.pending.len();
        self.pending.retain(|p| p.attachment.id != id);
        before != self.pending.len()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.last_error = None;
    }

    /// References to send with the next message; clears the tray.
    pub fn take_for_send(&mut self) -> Vec<AttachmentRef> {
        let refs = self.pending.iter().map(|p| p.attachment.clone()).collect();
        self.pending.clear();
        refs
    }

    /// Chip labels for the pending tray.
    pub fn chips(&self) -> Vec<String> {
        self.pending.iter().map(|p| p.chip_label()).collect()
    }

    /// Open target for a chip (blob path), if present.
    pub fn open_chip(&self, id: &str) -> Option<String> {
        self.pending
            .iter()
            .find(|p| p.attachment.id == id)
            .map(|p| p.open_path())
    }
}

#[cfg(test)]
mod attachments_ui_tests {
    use super::*;

    fn img(id: &str) -> PendingAttachment {
        PendingAttachment::new(id, "chart.png", "image/png", 48_000)
    }

    fn file(id: &str) -> PendingAttachment {
        PendingAttachment::new(id, "report.pdf", "application/pdf", 12_288)
    }

    #[test]
    fn attachments_ui_shows_image_preview_and_file_chips() {
        let mut tray = AttachmentTray::new();
        assert!(tray.is_empty());
        assert!(tray.attach(img("h1")));
        assert!(tray.attach(file("h2")));
        assert_eq!(tray.pending()[0].preview_kind(), "thumbnail");
        assert_eq!(tray.pending()[1].preview_kind(), "file-chip");
        let chips = tray.chips();
        assert_eq!(chips.len(), 2);
        assert!(chips[1].contains("report.pdf"), "{chips:?}");
        // Same content hash attaches once.
        assert!(tray.attach(img("h1")));
        assert_eq!(tray.pending().len(), 2);
        // Chips open the stored blob; removing drops the chip.
        assert_eq!(tray.open_chip("h2").as_deref(), Some(".synapse/blobs/h2"));
        assert!(tray.remove("h2"));
        assert_eq!(tray.pending().len(), 1);
        assert!(!tray.remove("missing"));
    }

    #[test]
    fn attachments_ui_enforces_limits_with_clear_errors() {
        let mut tray = AttachmentTray::new();
        let big = PendingAttachment::new("big", "movie.bin", "video/mp4", MAX_ATTACHMENT_BYTES + 1);
        assert!(!tray.attach(big));
        assert!(tray.last_error().unwrap().contains("too large"));
        for i in 0..MAX_ATTACHMENTS_PER_MESSAGE {
            assert!(tray.attach(file(&format!("f{i}"))));
        }
        assert!(!tray.attach(file("extra")));
        assert!(tray.last_error().unwrap().contains("at most 10"));
    }

    #[test]
    fn attachments_ui_take_for_send_clears_tray() {
        let mut tray = AttachmentTray::new();
        tray.attach(img("h1"));
        tray.attach(file("h2"));
        let refs = tray.take_for_send();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].id, "h1");
        assert!(tray.is_empty());
        tray.attach(file("h3"));
        tray.clear();
        assert!(tray.is_empty());
        assert!(tray.last_error().is_none());
    }
}
