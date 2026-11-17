//! The registration command backing the chat preview cards.

use std::sync::Arc;

use tauri::Manager;

use super::store::{
    PreviewDocumentStore, PreviewKind, PreviewStoreError, RENDERER_PREVIEW_MAX_BYTES,
};

/// Registers one previewable document and returns its content-addressed id
/// for `xf-preview://` loading. Idempotent per (kind, content).
#[tauri::command]
pub async fn preview_document_register(
    app: tauri::AppHandle,
    kind: String,
    content: String,
) -> Result<String, String> {
    let kind =
        PreviewKind::parse(&kind).ok_or_else(|| PreviewStoreError::UnsupportedKind.to_string())?;
    // A document the renderer supplies is a chat card's, bounded where the card
    // itself stops previewing. A project file previews from disk instead
    // (`crate::projects::project_preview_register`), up to the store's own cap.
    if content.len() > RENDERER_PREVIEW_MAX_BYTES {
        return Err(PreviewStoreError::DocumentTooLarge {
            limit: RENDERER_PREVIEW_MAX_BYTES,
        }
        .to_string());
    }
    app.state::<Arc<PreviewDocumentStore>>()
        .register(kind, content)
        .map_err(|error| error.to_string())
}
