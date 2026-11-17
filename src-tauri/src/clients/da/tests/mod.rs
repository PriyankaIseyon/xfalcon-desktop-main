use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::generated;

mod connection_reuse;
mod conversation_update_wire;
mod custom_models_wire;
mod mcp_servers_wire;
mod model_catalog;
mod model_selection;
mod models;
mod native_context_wire;
mod provider_keys;
mod provider_keys_wire;
mod repository_connection_reuse;
mod revision_wire;
mod run_cancel_wire;
mod truncate_wire;
mod workspace_summary;

const EXPECTED_SOURCE_URL: &str = "https://iseyon.mcp-staging.iseyon.com/openapi.json";
const EXPECTED_TITLE: &str = "IDA MCP Server";
const EXPECTED_VERSION: &str = "0.1.10";
const EXPECTED_SHA256: &str = "048be51a7eab7f651f9f801ed30017f88daf9b33542228e501b6d85fa5ba7ac4";
const EXPECTED_DA_SOURCE_REVISION: &str = "beb7ebcfa2ff43ddf4e0ebd657c460755abae940";

fn contract_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("contracts")
        .join("da")
}

#[test]
fn committed_contract_matches_the_approved_immutable_pin() {
    let root = contract_root();
    let bytes = fs::read(root.join("openapi.json")).expect("committed DA contract");
    let actual_sha = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(actual_sha, EXPECTED_SHA256);

    let document: Value = serde_json::from_slice(&bytes).expect("valid OpenAPI JSON");
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(document["info"]["title"], EXPECTED_TITLE);
    assert_eq!(document["info"]["version"], EXPECTED_VERSION);
    assert!(
        document["paths"]
            .as_object()
            .is_some_and(|paths| paths.len() >= 160),
        "the complete DA contract must be pinned, not a hand-selected subset"
    );
    assert!(
        document["components"]["schemas"]
            .as_object()
            .is_some_and(|schemas| schemas.len() >= 280),
        "the complete DA schema set must be pinned"
    );

    let provenance: Value = serde_json::from_slice(
        &fs::read(root.join("provenance.json")).expect("contract provenance"),
    )
    .expect("valid provenance JSON");
    assert_eq!(provenance["source_url"], EXPECTED_SOURCE_URL);
    assert_eq!(provenance["sha256"], EXPECTED_SHA256);
    assert_eq!(
        provenance["da_source_revision"],
        EXPECTED_DA_SOURCE_REVISION
    );

    let sums = fs::read_to_string(root.join("SHA256SUMS")).expect("contract checksum file");
    assert_eq!(
        sums.lines().collect::<Vec<_>>(),
        vec![format!("{EXPECTED_SHA256}  openapi.json")]
    );
}

#[test]
fn regeneration_script_and_generated_header_match_the_approved_pin() {
    let repository_root = contract_root()
        .parent()
        .and_then(std::path::Path::parent)
        .expect("contract directory is inside the repository")
        .to_owned();
    let script = fs::read_to_string(repository_root.join("scripts/sync-da-contract.ps1"))
        .expect("DA contract regeneration script");
    for expected in [
        format!("$SourceUrl = \"{EXPECTED_SOURCE_URL}\""),
        format!("$ExpectedVersion = \"{EXPECTED_VERSION}\""),
        format!("$PinnedSha256 = \"{EXPECTED_SHA256}\""),
        format!("$DaSourceRevision = \"{EXPECTED_DA_SOURCE_REVISION}\""),
    ] {
        assert!(
            script.contains(&expected),
            "regeneration script drifted from the reviewed contract pin: {expected}"
        );
    }

    let generated_source =
        fs::read_to_string(repository_root.join("src-tauri/src/clients/da/generated.rs"))
            .expect("generated DA models");
    assert!(generated_source.contains(&format!("//! Source: {EXPECTED_SOURCE_URL}")));
    assert!(generated_source.contains(&format!("//! Source SHA-256: {EXPECTED_SHA256}")));
    assert!(
        !generated_source.contains("Debug"),
        "generated DA models must not expose secret-bearing schemas through Debug"
    );
}

#[test]
fn generated_models_cover_every_required_da_domain() {
    fn assert_model<T>() {}

    assert_model::<generated::IdentityResponse>();
    assert_model::<generated::Conversation>();
    assert_model::<generated::ChatMessage>();
    assert_model::<generated::Run>();
    assert_model::<generated::RunEvent>();
    assert_model::<generated::ToolCallApproval>();
    assert_model::<generated::ModelEntity>();
    assert_model::<generated::ProviderKey>();
    assert_model::<generated::ProviderKeySecretInput>();
    assert_model::<generated::ProviderKeySecretResolved>();
    assert_model::<generated::ProviderKeySecretResolveRequest>();
    assert_model::<generated::ReplaceProviderKeySecretApiV1ChatProviderKeysRowIdSecretPutRequest>();
    assert_model::<generated::ResolveProviderKeySecretApiV1ChatProviderKeysRowIdResolvePostRequest>(
    );
    assert_model::<generated::McpServer>();
    assert_model::<generated::UserMcpServerPreference>();
    assert_model::<generated::SkillEntity>();
    assert_model::<generated::UploadResponse>();
    assert_model::<generated::ChatAttachment>();
    assert_model::<generated::ChatAuditEvent>();
    assert_model::<generated::TokenBudget>();
    assert_model::<generated::RateLimitResult>();
}
