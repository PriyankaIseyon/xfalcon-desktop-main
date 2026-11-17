//! Runs a chat turn on the user's own ChatGPT plan through OpenAI's official
//! "Sign in with ChatGPT" flow and the Responses API.
//!
//! Desktop signs in itself, stores its own tokens, and calls the Responses API
//! directly. It never reads Codex's credential cache and never starts `codex`.
//! See `openspec/changes/add-plan-model-providers/design.md`.

pub(crate) mod adapter;
pub(crate) mod callback;
pub(crate) mod identity;
pub(crate) mod models;
pub(crate) mod oauth;
pub(crate) mod request;
pub(crate) mod session;
pub(crate) mod sse;
pub(crate) mod store;
