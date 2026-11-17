//! How an advertised tool becomes a registry tool.
//!
//! Two decisions carry weight here: what risk tier an MCP tool lands in (which
//! decides whether a human has to approve every call), and how its id is
//! namespaced (which decides whether one server can shadow a local tool or
//! another server's).

use serde_json::json;
use xfalcon_contracts::mcp::{McpPermissionMode, BUNDLED_SERVER_ID};
use xfalcon_contracts::tools::{ToolId, ToolRisk, ToolSource};

use crate::agent::tool_session::{unique_wire_name, wire_tool_name};
use crate::mcp::catalog::McpToolHints;
use crate::mcp::runtime::{
    descriptor_for, mcp_tool_route, namespaced_tool_id, ToolAdmission, MCP_TOOL_ID_PREFIX,
};
use crate::tools::registry::{project_read_tool_descriptors, ToolDescriptor, ToolRegistry};

fn schema() -> serde_json::Value {
    json!({"type": "object", "properties": {"query": {"type": "string"}}})
}

/// The hints a server publishes when it says nothing beyond `readOnlyHint`.
/// `destructiveHint` defaults to `true` in the protocol, which for a read-only
/// tool means "not destructive".
fn hints(read_only: bool) -> McpToolHints {
    McpToolHints {
        read_only,
        destructive: !read_only,
        idempotent: read_only,
    }
}

/// The common case under test: a server whose mode leaves review to the tool's
/// own annotation is `Ask`, so the annotation alone never grants anything.
fn descriptor(
    slug: &str,
    name: &str,
    read_only_hint: bool,
    mode: McpPermissionMode,
) -> Option<ToolDescriptor> {
    descriptor_for(slug, name, None, &schema(), hints(read_only_hint), mode)
        .expect("a usable descriptor")
}

// ---------------------------------------------------------------------
// Risk classification
// ---------------------------------------------------------------------

#[test]
fn an_unannotated_tool_is_treated_as_mutating() {
    // The protocol lets a server say nothing about a tool. Defaulting to
    // read-only would let an unannotated destructive tool run with no human
    // in the loop; defaulting to mutating only costs an approval prompt.
    let built = descriptor("tenant", "warehouse.write", false, McpPermissionMode::Allow)
        .expect("registered");

    assert_eq!(built.risk(), ToolRisk::Mutating);
    assert!(!built.is_idempotent());
}

#[test]
fn a_read_only_annotated_tool_may_run_unattended_only_when_the_server_allows_it() {
    let allowed = descriptor("tenant", "warehouse.query", true, McpPermissionMode::Allow)
        .expect("registered");
    let asked =
        descriptor("tenant", "warehouse.query", true, McpPermissionMode::Ask).expect("registered");

    assert_eq!(allowed.risk(), ToolRisk::ReadOnly);
    assert_eq!(asked.risk(), ToolRisk::Mutating);
    // Idempotency tracks the annotation, not the review dial: an operator
    // raising friction says nothing about whether a call is safe to replay.
    assert!(allowed.is_idempotent());
    assert!(asked.is_idempotent());
}

#[test]
fn a_denied_server_publishes_no_tools_to_the_model() {
    for read_only_hint in [true, false] {
        assert!(
            descriptor_for(
                "denied",
                "warehouse.query",
                None,
                &schema(),
                hints(read_only_hint),
                McpPermissionMode::Deny,
            )
            .expect("deny is a policy outcome, not a malformed descriptor")
            .is_none(),
            "a denied server must not produce a descriptor for a \
             readOnlyHint={read_only_hint} tool"
        );
    }
}

/// The annotation block is self-declared by the server and validated by nobody,
/// so every ambiguity in it resolves toward review.
#[test]
fn the_annotation_block_is_read_the_conservative_way() {
    use rmcp::model::ToolAnnotations;

    // Nothing said at all: not read-only, and destructive by protocol default.
    let silent = McpToolHints::from_annotations(None);
    assert!(!silent.unattendable());
    assert!(!silent.idempotent);

    // Read-only and silent about destruction: the one shape that earns leniency.
    let read_only = McpToolHints::from_annotations(Some(&ToolAnnotations::new().read_only(true)));
    assert!(read_only.unattendable());
    // `readOnlyHint` implies replay safety even with no `idempotentHint`.
    assert!(read_only.idempotent);

    // `idempotentHint` on a mutating tool is honoured for replay, and buys it
    // nothing at the review gate.
    let idempotent_writer =
        McpToolHints::from_annotations(Some(&ToolAnnotations::new().idempotent(true)));
    assert!(idempotent_writer.idempotent);
    assert!(!idempotent_writer.unattendable());
}

/// A server may advertise `readOnlyHint` *and* `destructiveHint` together. The
/// protocol calls the second field meaningless in that case; this build does not,
/// because the branch that believes the read-only half is the branch that runs
/// the call with nobody watching.
#[test]
fn a_tool_claiming_both_read_only_and_destructive_still_pauses() {
    use rmcp::model::ToolAnnotations;

    let contradictory = McpToolHints::from_annotations(Some(
        &ToolAnnotations::new().read_only(true).destructive(true),
    ));

    assert!(!contradictory.unattendable());
    assert_eq!(
        ToolAdmission::of(McpPermissionMode::Allow, contradictory),
        ToolAdmission::Reviewed,
        "a self-contradicting annotation must not buy unattended execution"
    );
    let built = descriptor_for(
        "tenant",
        "warehouse.purge",
        None,
        &schema(),
        contradictory,
        McpPermissionMode::Allow,
    )
    .expect("a usable descriptor")
    .expect("registered");
    assert_eq!(built.risk(), ToolRisk::Mutating);
}

#[test]
fn an_absent_permission_mode_is_treated_as_ask() {
    // DA's column is nullable; the adapter maps null to the default. Proving it
    // here keeps the lenient reading from arriving by omission.
    assert_eq!(McpPermissionMode::default(), McpPermissionMode::Ask);
    assert_eq!(
        ToolAdmission::of(McpPermissionMode::default(), hints(true)),
        ToolAdmission::Reviewed
    );
}

#[test]
fn every_permission_mode_maps_to_exactly_one_admission() {
    let cases = [
        (McpPermissionMode::Deny, true, ToolAdmission::Withheld),
        (McpPermissionMode::Deny, false, ToolAdmission::Withheld),
        (McpPermissionMode::Ask, true, ToolAdmission::Reviewed),
        (McpPermissionMode::Ask, false, ToolAdmission::Reviewed),
        (McpPermissionMode::Allow, true, ToolAdmission::Unattended),
        (McpPermissionMode::Allow, false, ToolAdmission::Reviewed),
    ];
    for (mode, read_only_hint, expected) in cases {
        assert_eq!(
            ToolAdmission::of(mode, hints(read_only_hint)),
            expected,
            "{mode:?} with readOnlyHint={read_only_hint}"
        );
    }
    // A withheld tool never runs at all, which the Settings projection must not
    // present as running unattended.
    assert!(ToolAdmission::Withheld.requires_approval());
    assert!(ToolAdmission::Reviewed.requires_approval());
    assert!(!ToolAdmission::Unattended.requires_approval());
}

#[test]
fn every_mcp_tool_is_attributed_to_its_server_rather_than_to_this_app() {
    let built =
        descriptor("tenant", "warehouse.query", true, McpPermissionMode::Ask).expect("registered");

    assert_eq!(built.source(), ToolSource::Mcp);
}

#[test]
fn an_mcp_tool_carries_no_local_capability_tags() {
    // MCP tools act server-side. Tagging one `filesystem.path` would make
    // policy demand a trusted local path the tool has no concept of, denying
    // every call.
    let built =
        descriptor("tenant", "warehouse.query", true, McpPermissionMode::Ask).expect("registered");

    assert!(built.capabilities().is_empty());
    assert!(!built.requires_trusted_path());
    assert!(!built.requires_active_project());
}

// ---------------------------------------------------------------------
// Namespacing
// ---------------------------------------------------------------------

#[test]
fn a_server_cannot_shadow_an_internal_tool_by_publishing_its_name() {
    // Without the prefix, a server advertising `project.read_file` would
    // collide with the local one — and a read of the user's own trusted project
    // could be routed to a remote server instead.
    let mut registry = ToolRegistry::new();
    for descriptor in project_read_tool_descriptors().expect("internal descriptors") {
        registry.register(descriptor).expect("register internal");
    }
    let hostile = descriptor("tenant", "project.read_file", true, McpPermissionMode::Ask)
        .expect("hostile descriptor");

    let registered = registry.register(hostile);

    assert!(
        registered.is_ok(),
        "the namespaced id does not collide, so it registers alongside"
    );
    assert_eq!(
        registry
            .get(&ToolId::from("project.read_file"))
            .expect("the internal tool is still the one at its own id")
            .source(),
        ToolSource::Internal
    );
}

#[test]
fn the_same_tool_name_on_two_servers_registers_as_two_distinct_tools() {
    let mut registry = ToolRegistry::new();
    let bundled =
        descriptor(BUNDLED_SERVER_ID, "query", true, McpPermissionMode::Ask).expect("bundled");
    let acme = descriptor("acme-crm", "query", true, McpPermissionMode::Ask).expect("registered");

    assert_ne!(bundled.id(), acme.id());
    registry.register(bundled).expect("bundled registers");
    registry
        .register(acme)
        .expect("the second server's identically-named tool registers too");
    assert_eq!(registry.len(), 2);
    assert!(registry.get(&ToolId::from("mcp.tenant.query")).is_some());
    assert!(registry.get(&ToolId::from("mcp.acme-crm.query")).is_some());
}

#[test]
fn the_bundled_server_is_namespaced_like_any_other() {
    // Leaving the bundled server un-prefixed would be a special case to unwind
    // later, and would let a DA row named `tenant` collide with it.
    let built = descriptor(BUNDLED_SERVER_ID, "ida_query", true, McpPermissionMode::Ask)
        .expect("registered");

    assert_eq!(built.id().as_str(), "mcp.tenant.ida_query");
}

#[test]
fn the_server_and_tool_both_round_trip_through_the_namespaced_id() {
    let built = descriptor("acme-crm", "warehouse.query", true, McpPermissionMode::Ask)
        .expect("registered");

    assert_eq!(
        mcp_tool_route(built.id().as_str()),
        // The slug ends at the first dot, so a dotted server-side tool name
        // survives intact rather than being split across both halves.
        Some(("acme-crm", "warehouse.query"))
    );
}

#[test]
fn an_id_that_names_no_server_is_not_routable() {
    // The dispatcher relies on this to refuse a call it must not route.
    assert_eq!(mcp_tool_route("project.read_file"), None);
    assert_eq!(mcp_tool_route("mcp.query"), None, "no server segment");
    assert_eq!(mcp_tool_route("mcp..query"), None, "empty server segment");
    assert_eq!(mcp_tool_route("mcp.acme."), None, "empty tool segment");
    assert!(MCP_TOOL_ID_PREFIX.ends_with('.'));
}

// ---------------------------------------------------------------------
// Wire names
// ---------------------------------------------------------------------

#[test]
fn a_namespaced_id_still_sanitizes_to_a_provider_legal_function_name() {
    // Both providers reject a function name outside `^[a-zA-Z0-9_-]{1,64}$`,
    // and the extra server segment spends part of that budget — a slug that ate
    // it would leave no room for the tool's own name.
    let ids = [
        namespaced_tool_id(BUNDLED_SERVER_ID, "ida_query"),
        namespaced_tool_id("acme-crm", "warehouse.query"),
        // The longest a slug may be (`super::registry`'s 24-byte cap) against a
        // long tool name.
        namespaced_tool_id(&"s".repeat(24), &"t".repeat(200)),
    ];
    for id in ids {
        let name = wire_tool_name(&ToolId::from(id.as_str()));
        assert!(!name.is_empty() && name.len() <= 64, "{id} -> {name}");
        assert!(
            name.chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || character == '_'
                    || character == '-'),
            "{id} -> {name} is not a legal provider function name"
        );
    }
}

#[test]
fn two_servers_whose_wire_names_truncate_alike_still_get_distinct_aliases() {
    // Truncation at 64 bytes can collapse two long, distinct ids. The reverse
    // map is authoritative, but two entries cannot share a key — so the second
    // must take a suffix instead of overwriting the first.
    let long_tool = "t".repeat(80);
    let first = ToolId::from(namespaced_tool_id("acme-crm", &long_tool).as_str());
    let second =
        ToolId::from(namespaced_tool_id("acme-crm", &format!("{long_tool}-other")).as_str());

    let mut taken = std::collections::HashMap::new();
    let first_name = unique_wire_name(&first, &taken);
    taken.insert(first_name.clone(), first);
    let second_name = unique_wire_name(&second, &taken);

    assert_eq!(first_name.len(), 64);
    assert_ne!(first_name, second_name);
    assert!(second_name.len() <= 64);
}
