//! File uploads: picking a local file, transferring it to tenant storage, and
//! projecting only what the UI may see.
//!
//! See [`domain`] for the three-way split between the local file, the
//! in-flight transfer, and the webview-facing projection — and for why it is
//! three types rather than one struct with unserialized fields.

pub mod clipboard;
pub mod commands;
pub mod da_repository;
pub mod domain;
pub mod kind;
pub mod service;
pub mod transfer;

pub use commands::{upload_cancel, upload_choose, upload_paste, upload_queue, upload_retry};
pub use domain::{
    sanitize_display_name, LocalUploadSource, UploadRecord, UploadTransitionError,
    FALLBACK_DISPLAY_NAME, MAX_DISPLAY_NAME_BYTES,
};

#[cfg(test)]
mod tests;
