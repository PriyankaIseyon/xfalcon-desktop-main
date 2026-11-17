//! Device-local plan providers. Their models are not in DA's catalog, so their
//! rows are built on this device and merged into the catalog at read time.

mod adapters;
pub(crate) mod chat_effort;
pub(crate) mod commands;
pub(crate) mod models;
mod service;
pub(crate) mod settings;

pub(crate) use service::{PlanProviders, WORK_ROOT_DIR};
