//! Team message attachments (see docs/TEAM-PROTOCOL.md §3a).
//!
//! Bytes live once in the content store (`<project>/.synapse/blobs/<hash>`,
//! de-duplicated by content hash); envelopes carry only [`AttachmentRef`].
//! Extracted text is redacted before storage or model use, and delivery is
//! adapted to each recipient's capabilities.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bus::AttachmentRef;
use crate::redact::{Redacted, Redactor};

/// Limits (TEAM-PROTOCOL.md §3a). Configurable per project; these are defaults.
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;
/// Text-like files inline fully up to this many bytes; larger ones summarise.
pub const INLINE_TEXT_LIMIT: usize = 4096;

/// Attachment kind (TEAM-PROTOCOL.md §3a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    #[default]
    File,
    Table,
    Artifact,
    Source,
    Screenshot,
}

/// One stored attachment: bytes kept once, keyed by content hash.
#[derive(Debug, Clone)]
pub struct StoredAttachment {
    /// Content hash (hex); also the blob file name and the [`AttachmentRef`] id.
    pub id: String,
    pub name: String,
    pub mime: String,
    pub kind: AttachmentKind,
    pub bytes: Vec<u8>,
    /// Extracted text after redaction (empty for pure images).
    pub redacted_text: String,
    /// Secrets hidden during extraction (chip count).
    pub redacted_count: usize,
    /// One-line generated description (vision caption / schema summary).
    pub description: String,
}

/// What a recipient can consume (resolved per recipient by the context builder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveredAttachment {
    /// Send as a native image block (recipient model supports images).
    Image { id: String, mime: String },
    /// Small text-like file inlined fully (already redacted).
    InlineText { id: String, text: String },
    /// Large file: summary + `read_attachment(id, range)` affordance.
    Summary { id: String, summary: String },
    /// CLI agent without image support: file path inside the project folder.
    FilePath { id: String, path: String },
    /// Non-vision model: cached text description + OCR/excerpt.
    Description { id: String, text: String },
}

/// Recipient capabilities used to pick a [`DeliveredAttachment`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecipientCapabilities {
    /// models.dev `modalities.input` contains `image`.
    pub supports_image: bool,
    /// CLI agent advertising ACP resource/image blocks.
    pub supports_acp_resources: bool,
    /// Whether full text may be inlined regardless of size (small contexts).
    pub prefers_full_text: bool,
}

impl RecipientCapabilities {
    pub fn vision_model() -> Self {
        Self {
            supports_image: true,
            supports_acp_resources: false,
            prefers_full_text: false,
        }
    }

    pub fn text_only_model() -> Self {
        Self {
            supports_image: false,
            supports_acp_resources: false,
            prefers_full_text: false,
        }
    }

    pub fn cli_agent() -> Self {
        Self {
            supports_image: false,
            supports_acp_resources: true,
            prefers_full_text: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AttachmentError {
    #[error("attachment too large ({0} bytes, max {1})")]
    TooLarge(usize, usize),
    #[error("too many attachments on one message (max {0})")]
    TooMany(usize),
    #[error("executables cannot be attached: {0}")]
    Executable(String),
    #[error("unknown attachment: {0}")]
    Unknown(String),
    #[error("empty attachment: {0}")]
    Empty(String),
}

/// Content store: bytes once, by hash. Clone shares nothing; use one per project.
#[derive(Debug, Default)]
pub struct AttachmentStore {
    blobs: HashMap<String, StoredAttachment>,
    redactor: Redactor,
    max_bytes: usize,
}

impl AttachmentStore {
    pub fn new() -> Self {
        Self {
            blobs: HashMap::new(),
            redactor: Redactor::new(),
            max_bytes: MAX_ATTACHMENT_BYTES,
        }
    }

    pub fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            ..Self::new()
        }
    }

    /// Register a known secret so extracted text redacts it by name.
    pub fn add_secret(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.redactor.add_secret(name, value);
    }

    pub fn len(&self) -> usize {
        self.blobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&StoredAttachment> {
        self.blobs.get(id)
    }

    /// Blob path under the project folder (`ARCHITECTURE.md` §3).
    pub fn blob_path(&self, id: &str) -> String {
        format!(".synapse/blobs/{id}")
    }

    /// Store `bytes` once, keyed by content hash. Same bytes → same id, no copy.
    pub fn insert(
        &mut self,
        name: &str,
        mime: &str,
        bytes: Vec<u8>,
    ) -> Result<StoredAttachment, AttachmentError> {
        if bytes.is_empty() {
            return Err(AttachmentError::Empty(name.to_string()));
        }
        if bytes.len() > self.max_bytes {
            return Err(AttachmentError::TooLarge(bytes.len(), self.max_bytes));
        }
        reject_executable(name, mime)?;
        let id = content_hash(&bytes);
        if let Some(existing) = self.blobs.get(&id) {
            return Ok(existing.clone());
        }
        let kind = infer_kind(name, mime);
        let raw_text = extract_text(&kind, mime, &bytes);
        let Redacted {
            text: redacted_text,
            count: redacted_count,
            ..
        } = self.redactor.redact(&raw_text);
        let description = describe_attachment(name, &kind, mime, &redacted_text, &bytes);
        let stored = StoredAttachment {
            id: id.clone(),
            name: name.to_string(),
            mime: mime.to_string(),
            kind,
            bytes,
            redacted_text,
            redacted_count,
            description,
        };
        self.blobs.insert(id, stored.clone());
        Ok(stored)
    }

    /// Envelope reference for a stored attachment (no inline bytes).
    pub fn reference(&self, stored: &StoredAttachment) -> AttachmentRef {
        AttachmentRef {
            id: stored.id.clone(),
            name: stored.name.clone(),
            mime: stored.mime.clone(),
        }
    }

    /// Validate an outgoing message's attachment list (count limit).
    pub fn check_message(&self, refs: &[AttachmentRef]) -> Result<(), AttachmentError> {
        if refs.len() > MAX_ATTACHMENTS_PER_MESSAGE {
            return Err(AttachmentError::TooMany(MAX_ATTACHMENTS_PER_MESSAGE));
        }
        for r in refs {
            if !self.blobs.contains_key(&r.id) {
                return Err(AttachmentError::Unknown(r.id.clone()));
            }
        }
        Ok(())
    }

    /// Deliver one attachment in the form a recipient can consume.
    pub fn deliver(
        &self,
        id: &str,
        caps: RecipientCapabilities,
    ) -> Result<DeliveredAttachment, AttachmentError> {
        let stored = self
            .blobs
            .get(id)
            .ok_or_else(|| AttachmentError::Unknown(id.to_string()))?;
        Ok(deliver_stored(stored, caps, &self.blob_path(id)))
    }
}

/// Stable content hash (FNV-1a 64-bit, hex). No new dependency; binary size
/// unchanged (PERFORMANCE.md §1b). Same bytes always give the same id.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn infer_kind(name: &str, mime: &str) -> AttachmentKind {
    let lower = name.to_lowercase();
    if mime.starts_with("image/") {
        return AttachmentKind::Image;
    }
    if mime == "text/csv"
        || mime.contains("csv")
        || lower.ends_with(".csv")
        || lower.ends_with(".tsv")
    {
        return AttachmentKind::Table;
    }
    AttachmentKind::File
}

fn reject_executable(name: &str, mime: &str) -> Result<(), AttachmentError> {
    let lower = name.to_lowercase();
    let bad_ext = [
        ".exe", ".msi", ".bat", ".cmd", ".com", ".scr", ".ps1", ".dll", ".so", ".dylib",
    ];
    if bad_ext.iter().any(|e| lower.ends_with(e)) {
        return Err(AttachmentError::Executable(name.to_string()));
    }
    if mime.contains("x-executable")
        || mime.contains("x-msdownload")
        || mime.contains("x-sh")
        || mime == "application/x-sh"
    {
        return Err(AttachmentError::Executable(name.to_string()));
    }
    Ok(())
}

/// Extract searchable text from text-like bytes. Images return an empty string
/// (their description is generated once and cached on the attachment).
fn extract_text(kind: &AttachmentKind, mime: &str, bytes: &[u8]) -> String {
    match kind {
        AttachmentKind::Image | AttachmentKind::Screenshot => String::new(),
        _ => {
            let text_like = mime.starts_with("text/")
                || mime.contains("json")
                || mime.contains("csv")
                || mime.contains("markdown")
                || mime.starts_with("application/json");
            if text_like || (bytes.len() <= INLINE_TEXT_LIMIT && is_mostly_text(bytes)) {
                String::from_utf8_lossy(bytes).into_owned()
            } else {
                String::new()
            }
        }
    }
}

fn is_mostly_text(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(512)];
    if sample.is_empty() {
        return false;
    }
    let printable = sample
        .iter()
        .filter(|b| b.is_ascii_graphic() || b.is_ascii_whitespace())
        .count();
    printable * 10 >= sample.len() * 8
}

fn describe_attachment(
    name: &str,
    kind: &AttachmentKind,
    mime: &str,
    redacted_text: &str,
    bytes: &[u8],
) -> String {
    match kind {
        AttachmentKind::Image | AttachmentKind::Screenshot => {
            format!("Image {name} ({mime}, {} bytes)", bytes.len())
        }
        AttachmentKind::Table => {
            let rows: Vec<&str> = redacted_text.lines().take(4).collect();
            if rows.is_empty() {
                format!("Table {name} ({} bytes)", bytes.len())
            } else {
                format!("Table {name}: header `{}` ({} bytes)", rows[0], bytes.len())
            }
        }
        _ => {
            if redacted_text.trim().is_empty() {
                format!("File {name} ({mime}, {} bytes)", bytes.len())
            } else {
                let head: String = redacted_text.chars().take(160).collect();
                format!("File {name}: {head}")
            }
        }
    }
}

fn deliver_stored(
    stored: &StoredAttachment,
    caps: RecipientCapabilities,
    blob_path: &str,
) -> DeliveredAttachment {
    let is_image = matches!(
        stored.kind,
        AttachmentKind::Image | AttachmentKind::Screenshot
    );
    if is_image {
        if caps.supports_image || caps.supports_acp_resources {
            return DeliveredAttachment::Image {
                id: stored.id.clone(),
                mime: stored.mime.clone(),
            };
        }
        // Text-only model: cached description + redacted excerpt (generated once).
        let mut text = stored.description.clone();
        if !stored.redacted_text.trim().is_empty() {
            let excerpt: String = stored.redacted_text.chars().take(500).collect();
            text.push_str(&format!(" OCR/excerpt: {excerpt}"));
        } else {
            text.push_str(" (no extractable text; describe visually on request)");
        }
        return DeliveredAttachment::Description {
            id: stored.id.clone(),
            text,
        };
    }
    // CLI agents get a project-local path they can read.
    if !caps.supports_image && caps.supports_acp_resources {
        return DeliveredAttachment::FilePath {
            id: stored.id.clone(),
            path: blob_path.to_string(),
        };
    }
    if stored.redacted_text.is_empty() {
        return DeliveredAttachment::Summary {
            id: stored.id.clone(),
            summary: stored.description.clone(),
        };
    }
    if caps.prefers_full_text || stored.redacted_text.len() <= INLINE_TEXT_LIMIT {
        return DeliveredAttachment::InlineText {
            id: stored.id.clone(),
            text: stored.redacted_text.clone(),
        };
    }
    let head: String = stored.redacted_text.chars().take(500).collect();
    DeliveredAttachment::Summary {
        id: stored.id.clone(),
        summary: format!(
            "{}… [{} bytes total; use read_attachment({}, range)]",
            head,
            stored.redacted_text.len(),
            stored.id
        ),
    }
}

/// One-line model context for an attachment id (redacted, capability-aware).
pub fn context_line(
    store: &AttachmentStore,
    id: &str,
    caps: RecipientCapabilities,
) -> Result<String, AttachmentError> {
    Ok(match store.deliver(id, caps)? {
        DeliveredAttachment::Image { id, mime } => {
            format!("[image attachment {id} ({mime})]")
        }
        DeliveredAttachment::InlineText { id, text } => {
            format!("[attachment {id}]: {text}")
        }
        DeliveredAttachment::Summary { id, summary } => {
            format!("[attachment {id} summary]: {summary}")
        }
        DeliveredAttachment::FilePath { id, path } => {
            format!("[attachment {id} at {path}]")
        }
        DeliveredAttachment::Description { id, text } => {
            format!("[image {id} description]: {text}")
        }
    })
}

#[cfg(test)]
mod attachments_tests {
    use super::*;
    use crate::redact::fixtures;

    fn small_store() -> AttachmentStore {
        AttachmentStore::new()
    }

    #[test]
    fn attachments_store_dedupes_by_content_hash() {
        let mut store = small_store();
        let a = store
            .insert("notes.md", "text/markdown", b"hello team".to_vec())
            .unwrap();
        let b = store
            .insert("copy-of-notes.md", "text/markdown", b"hello team".to_vec())
            .unwrap();
        assert_eq!(a.id, b.id, "same bytes must share one id");
        assert_eq!(store.len(), 1, "bytes stored once");
        assert_eq!(a.id, content_hash(b"hello team"));
        // Envelope carries only the reference, never inline bytes.
        let r = store.reference(&a);
        assert_eq!(r.id, a.id);
        assert_eq!(r.name, "notes.md");
        store.check_message(&[r]).unwrap();
        // Blob lives at one path.
        assert_eq!(store.blob_path(&a.id), format!(".synapse/blobs/{}", a.id));
    }

    #[test]
    fn attachments_redact_extracted_text_before_storage() {
        let mut store = small_store();
        for (name, value) in fixtures::all() {
            store.add_secret(name, value);
        }
        let body = format!(
            "deploy with {} and token {} end",
            fixtures::OPENAI.1,
            fixtures::GITHUB.1
        );
        let stored = store
            .insert("config.txt", "text/plain", body.into_bytes())
            .unwrap();
        assert!(!stored.redacted_text.contains(fixtures::OPENAI.1));
        assert!(!stored.redacted_text.contains(fixtures::GITHUB.1));
        assert!(stored.redacted_text.contains("[redacted:"));
        assert!(stored.redacted_count >= 2);
        // Model delivery also never leaks the raw value.
        let delivered = store
            .deliver(&stored.id, RecipientCapabilities::text_only_model())
            .unwrap();
        let line = context_line(&store, &stored.id, RecipientCapabilities::vision_model()).unwrap();
        let rendered = format!("{delivered:?} {line}");
        assert!(!rendered.contains(fixtures::OPENAI.1));
        assert!(!rendered.contains(fixtures::GITHUB.1));
    }

    #[test]
    fn attachments_deliver_matches_recipient_capabilities() {
        let mut store = small_store();
        let img = store
            .insert(
                "chart.png",
                "image/png",
                vec![0x89, 0x50, 0x4e, 0x47, 1, 2, 3],
            )
            .unwrap();
        // Vision model gets a native image block.
        assert!(matches!(
            store.deliver(&img.id, RecipientCapabilities::vision_model()),
            Ok(DeliveredAttachment::Image { .. })
        ));
        // Text-only model gets the cached description, generated once.
        let desc = store
            .deliver(&img.id, RecipientCapabilities::text_only_model())
            .unwrap();
        assert!(matches!(desc, DeliveredAttachment::Description { .. }));
        let first =
            context_line(&store, &img.id, RecipientCapabilities::text_only_model()).unwrap();
        let second =
            context_line(&store, &img.id, RecipientCapabilities::text_only_model()).unwrap();
        assert_eq!(first, second, "description is cached on the attachment");

        // Small text file inlines; CLI agents get a project-local path.
        let small = store
            .insert("n.md", "text/markdown", b"tiny".to_vec())
            .unwrap();
        assert!(matches!(
            store.deliver(&small.id, RecipientCapabilities::vision_model()),
            Ok(DeliveredAttachment::InlineText { .. })
        ));
        assert!(matches!(
            store.deliver(&small.id, RecipientCapabilities::cli_agent()),
            Ok(DeliveredAttachment::FilePath { .. })
        ));

        // Large text file summarises with a read_attachment affordance.
        let big = "x".repeat(INLINE_TEXT_LIMIT + 100);
        let large = store
            .insert("big.csv", "text/csv", big.into_bytes())
            .unwrap();
        match store
            .deliver(&large.id, RecipientCapabilities::vision_model())
            .unwrap()
        {
            DeliveredAttachment::Summary { summary, .. } => {
                assert!(summary.contains("read_attachment"));
            }
            other => panic!("expected summary, got {other:?}"),
        }
    }

    #[test]
    fn attachments_reject_oversize_and_executables() {
        let mut store = AttachmentStore::with_max_bytes(16);
        assert!(matches!(
            store.insert("big.bin", "application/octet-stream", vec![0u8; 17]),
            Err(AttachmentError::TooLarge(..))
        ));
        let mut store = small_store();
        for name in ["run.exe", "setup.msi", "evil.bat", "lib.so"] {
            assert!(
                matches!(
                    store.insert(name, "application/octet-stream", b"bytes".to_vec()),
                    Err(AttachmentError::Executable(_))
                ),
                "{name} must be rejected"
            );
        }
        assert!(matches!(
            store.insert("tool", "application/x-executable", b"bytes".to_vec()),
            Err(AttachmentError::Executable(_))
        ));
    }

    #[test]
    fn attachments_message_limits_and_unknown() {
        let mut store = small_store();
        let stored = store.insert("a.txt", "text/plain", b"hi".to_vec()).unwrap();
        let r = store.reference(&stored);
        let many = vec![r.clone(); MAX_ATTACHMENTS_PER_MESSAGE + 1];
        assert_eq!(
            store.check_message(&many),
            Err(AttachmentError::TooMany(MAX_ATTACHMENTS_PER_MESSAGE))
        );
        assert!(matches!(
            store.deliver("nope", RecipientCapabilities::vision_model()),
            Err(AttachmentError::Unknown(_))
        ));
        assert!(matches!(
            store.check_message(&[AttachmentRef {
                id: "nope".into(),
                name: "x".into(),
                mime: "text/plain".into(),
            }]),
            Err(AttachmentError::Unknown(_))
        ));
    }
}
