//! Content-addressed, bounded, in-memory preview document store.

use std::sync::Mutex;

use indexmap::IndexMap;
use sha2::{Digest, Sha256};

/// Per-document ceiling for any document, the largest a project file previews
/// at (`crate::projects::MAX_PROJECT_PREVIEW_BYTES`).
pub const PREVIEW_DOC_MAX_BYTES: usize = 32 * 1024 * 1024;
/// A document the renderer itself supplies — a chat card's — is bounded
/// tighter. Mirrors the frontend's `HTML_PREVIEW_MAX_BYTES`: content past this
/// renders as source, so it never reaches registration.
pub const RENDERER_PREVIEW_MAX_BYTES: usize = 2 * 1024 * 1024;
/// Total store ceiling; least-recently-served documents evict past it. Room
/// for several full-size project previews at once.
pub const PREVIEW_STORE_MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewKind {
    Html,
    Svg,
}

impl PreviewKind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "html" => Some(Self::Html),
            "svg" => Some(Self::Svg),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Svg => "svg",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Html => "text/html; charset=utf-8",
            Self::Svg => "image/svg+xml; charset=utf-8",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PreviewStoreError {
    #[error("preview document exceeds the {limit}-byte limit")]
    DocumentTooLarge { limit: usize },
    #[error("preview kind is not supported")]
    UnsupportedKind,
}

struct StoredDocument {
    kind: PreviewKind,
    content: String,
}

#[derive(Default)]
struct StoreState {
    /// Insertion order doubles as recency order: entries move to the back on
    /// every serve, so eviction pops the front.
    documents: IndexMap<String, StoredDocument>,
    total_bytes: usize,
}

/// Managed Tauri state. One store per process; documents die with it —
/// nothing about previews is ever persisted.
pub struct PreviewDocumentStore {
    state: Mutex<StoreState>,
    doc_max_bytes: usize,
    total_max_bytes: usize,
}

impl Default for PreviewDocumentStore {
    fn default() -> Self {
        Self::with_caps(PREVIEW_DOC_MAX_BYTES, PREVIEW_STORE_MAX_BYTES)
    }
}

impl PreviewDocumentStore {
    /// Cap-parameterized constructor so tests can exercise eviction without
    /// multi-megabyte fixtures. Production always uses `default()`.
    pub fn with_caps(doc_max_bytes: usize, total_max_bytes: usize) -> Self {
        Self {
            state: Mutex::new(StoreState::default()),
            doc_max_bytes,
            total_max_bytes,
        }
    }

    /// Registers `content` and returns its content-addressed id. Identical
    /// (kind, content) pairs land on the same id — re-rendering a message
    /// re-registers the same bytes idempotently instead of leaking a copy
    /// per render pass. Takes the content owned, so a caller that already
    /// holds it (a project file read from disk) is not made to keep a copy.
    pub fn register(
        &self,
        kind: PreviewKind,
        content: impl Into<String>,
    ) -> Result<String, PreviewStoreError> {
        let content = content.into();
        if content.len() > self.doc_max_bytes {
            return Err(PreviewStoreError::DocumentTooLarge {
                limit: self.doc_max_bytes,
            });
        }
        let id = document_id(kind, &content);
        let mut state = self.state.lock().expect("preview store poisoned");
        if let Some(index) = state.documents.get_index_of(&id) {
            let last = state.documents.len() - 1;
            state.documents.move_index(index, last);
            return Ok(id);
        }
        while state.total_bytes + content.len() > self.total_max_bytes {
            let Some((_, evicted)) = state.documents.shift_remove_index(0) else {
                break;
            };
            state.total_bytes -= evicted.content.len();
        }
        state.total_bytes += content.len();
        state
            .documents
            .insert(id.clone(), StoredDocument { kind, content });
        Ok(id)
    }

    /// The document for `id`, freshening its recency. `None` for unknown or
    /// evicted ids — the protocol layer answers those with 404.
    pub fn serve(&self, id: &str) -> Option<(PreviewKind, String)> {
        let mut state = self.state.lock().expect("preview store poisoned");
        let index = state.documents.get_index_of(id)?;
        let last = state.documents.len() - 1;
        state.documents.move_index(index, last);
        let document = state.documents.get(id)?;
        Some((document.kind, document.content.clone()))
    }

    #[cfg(test)]
    pub(crate) fn resident_bytes(&self) -> usize {
        self.state
            .lock()
            .expect("preview store poisoned")
            .total_bytes
    }
}

fn document_id(kind: PreviewKind, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_str().as_bytes());
    hasher.update([0u8]);
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let mut id = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(id, "{byte:02x}");
    }
    id
}
