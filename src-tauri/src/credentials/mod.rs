#![allow(
    dead_code,
    reason = "Task 3 credential foundation remains internal until fixed-origin provider verification and DA wire tests are complete"
)]

pub(crate) mod commands;
pub(crate) mod envelope;
pub(crate) mod service;
pub(crate) mod vault;
pub mod verifier;

pub use commands::{
    credential_create, credential_delete, credential_list, credential_replace, CredentialRuntime,
};

#[cfg(test)]
mod tests;
