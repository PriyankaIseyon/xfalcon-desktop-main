//! The Datasets destination: a file a person uploads into tenant storage,
//! where it becomes queryable data (spec: dataset-uploads).
//!
//! A dataset is not a composer attachment. An attachment is 10 MiB, belongs to
//! one message, and comes back through a signed GET; a dataset is up to 2 GiB
//! and outlives every conversation. They share the transfer runtime in
//! [`crate::uploads`] and nothing else.
//!
//! # The shape of the flow
//!
//! ```text
//! pick / drop ─▶ validate ─▶ stage ─▶ preview ─┬─▶ recipe ─▶ materialize ─┐
//!                                              │                          ▼
//!                                              └──────────────────▶ UploadService
//! ```
//!
//! Picking does not start an upload. A person previews the file, chooses a
//! worksheet and a header row, optionally shapes the data, and then confirms —
//! so between the picker and the confirm button the file is *staged*, its path
//! held here and never handed to the webview ([`staging`]).
//!
//! [`validate`] decides what a file has to be before any of it reaches DA.
//! [`preview`] reads its leading rows. [`recipe`] is the cleanup a person
//! applies, applied by one function whether it is redrawing the preview or
//! producing the uploaded file.

pub mod catalog;
pub mod commands;
pub mod errors;
pub mod preview;
pub mod recipe;
pub mod staging;
pub mod validate;

pub use commands::{
    dataset_delete, dataset_detail, dataset_download, dataset_list, dataset_pick, dataset_preview,
    dataset_stage_discard, dataset_upload_start,
};
pub use drop::register_drop_handler;
pub use staging::Staging;

mod drop;

#[cfg(test)]
mod tests;
