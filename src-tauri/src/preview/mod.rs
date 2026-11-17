//! Sandboxed document previews (spec: preview-protocol).
//!
//! Chat's rich-content cards preview assistant-authored documents (HTML
//! reports, SVG) in an iframe. Local-scheme frames (`data:`, `blob:`,
//! `srcdoc`) inherit the app's CSP in WebView2 (CSP3 local-scheme
//! inheritance), which refuses the document's own inline styles and
//! scripts — the "Times-on-white report" defect. This module gives
//! previews a real origin instead: the `xf-preview` URI scheme, backed by
//! a bounded in-memory store, serving each document with its OWN response
//! CSP — inline style/script allowed, ALL network denied.
//!
//! Isolation contract (kept in lockstep with `chat-rich-content`):
//! - the preview origin is distinct from the app origin, so a document can
//!   never reach the app DOM, storage, or IPC;
//! - the iframe keeps `sandbox="allow-scripts"` (never
//!   `allow-same-origin`) as defense in depth;
//! - the per-document CSP has `default-src 'none'; connect-src 'none'` —
//!   a hostile document cannot exfiltrate over the network;
//! - documents are content-addressed (sha-256) and held in memory only.

mod commands;
mod protocol;
mod store;

#[cfg(test)]
mod tests;

pub use commands::preview_document_register;
pub use protocol::{handle_preview_request, PREVIEW_HTTP_HOST, PREVIEW_URI_SCHEME};
pub use store::{
    PreviewDocumentStore, PreviewKind, PreviewStoreError, PREVIEW_DOC_MAX_BYTES,
    PREVIEW_STORE_MAX_BYTES, RENDERER_PREVIEW_MAX_BYTES,
};
