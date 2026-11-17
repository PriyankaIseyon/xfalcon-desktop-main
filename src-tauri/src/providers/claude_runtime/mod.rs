//! Runs a chat turn on the user's own Claude account by starting their
//! installed `claude` program.
//!
//! `claude` is used as a model for one turn at a time. Desktop's tools are
//! declared to it but never executed by it: the tool loop stays in charge.
//! See `openspec/changes/add-plan-model-providers/design.md`.

pub(crate) mod account;
pub(crate) mod adapter;
pub(crate) mod declare_server;
#[cfg(test)]
mod live_smoke;
pub(crate) mod locate;
pub(crate) mod process;
pub(crate) mod prompt;
pub(crate) mod stream;
