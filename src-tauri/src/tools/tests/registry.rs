//! Coverage for `super::super::registry::project_read_tool_descriptors`: the
//! shape every one of the four advertised internal trusted-project
//! read/search tool descriptors must have.

use xfalcon_contracts::tools::{ToolId, ToolRisk};

use crate::tools::registry::{
    project_propose_patch_tool_descriptor, project_read_tool_descriptors,
    CAPABILITY_REQUIRES_PROJECT, CAPABILITY_REQUIRES_TRUSTED_PATH,
};

#[test]
fn project_read_tool_descriptors_are_four_read_only_idempotent_project_and_path_scoped_tools() {
    let descriptors =
        project_read_tool_descriptors().expect("well-known descriptors are valid by construction");
    assert_eq!(descriptors.len(), 4);

    let expected_ids = [
        "project.search_files",
        "project.search_text",
        "project.read_file",
        "project.list_directory",
    ];
    assert_eq!(descriptors.len(), expected_ids.len());

    for (descriptor, expected_id) in descriptors.iter().zip(expected_ids) {
        assert_eq!(
            descriptor.id(),
            &ToolId::from(expected_id),
            "unexpected descriptor id"
        );
        assert_eq!(
            descriptor.risk(),
            ToolRisk::ReadOnly,
            "{expected_id} must be advertised as read-only"
        );
        assert!(
            descriptor.is_idempotent(),
            "{expected_id} must be advertised as idempotent"
        );
        assert!(
            descriptor.has_valid_schema(),
            "{expected_id}'s advertised schema must be a valid bounded shape"
        );
        assert!(
            descriptor
                .capabilities()
                .iter()
                .any(|capability| capability == CAPABILITY_REQUIRES_PROJECT),
            "{expected_id} must require an active trusted project"
        );
        assert!(
            descriptor
                .capabilities()
                .iter()
                .any(|capability| capability == CAPABILITY_REQUIRES_TRUSTED_PATH),
            "{expected_id} must require a trusted-project-contained path"
        );
        assert!(
            descriptor.input_schema()["properties"]
                .get("project_id")
                .is_none(),
            "{expected_id} must use the run-bound project instead of asking the model for an id"
        );
        assert!(!descriptor.input_schema()["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "project_id")));
    }
}

#[test]
fn project_propose_patch_is_read_only_non_idempotent_and_project_scoped_only() {
    let descriptor = project_propose_patch_tool_descriptor()
        .expect("the well-known descriptor is valid by construction");

    assert_eq!(descriptor.id(), &ToolId::from("project.propose_patch"));
    assert_eq!(
        descriptor.risk(),
        ToolRisk::ReadOnly,
        "propose_patch only plans; it performs no mutation itself, so it carries no third risk \
         tier beyond this crate's closed ReadOnly/Mutating enum"
    );
    assert!(
        !descriptor.is_idempotent(),
        "propose_patch persists a new pending-approval row as a side effect, unlike a pure read"
    );
    assert!(descriptor.has_valid_schema());
    assert!(descriptor
        .capabilities()
        .iter()
        .any(|capability| capability == CAPABILITY_REQUIRES_PROJECT));
    assert!(
        !descriptor
            .capabilities()
            .iter()
            .any(|capability| capability == CAPABILITY_REQUIRES_TRUSTED_PATH),
        "a multi-path patch proposal cannot be validated by the single-path PathAuthority check \
         this capability tag drives -- see the descriptor's own doc comment"
    );

    let schema = descriptor.input_schema();
    assert!(schema["properties"].get("project_id").is_none());
    assert_eq!(schema["required"], serde_json::json!(["operations"]));
    let variants = schema["properties"]["operations"]["items"]["oneOf"]
        .as_array()
        .expect("patch operations use a discriminated union");
    assert_eq!(variants.len(), 8);
    // create, modify
    assert_eq!(
        variants[0]["required"],
        serde_json::json!(["kind", "path", "content"])
    );
    assert_eq!(
        variants[1]["required"],
        serde_json::json!(["kind", "path", "content"])
    );
    // The anchored kinds. `replace` takes an edit list rather than content,
    // which is what lets an existing file be changed without re-sending it.
    assert_eq!(
        variants[2]["required"],
        serde_json::json!(["kind", "path", "edits"])
    );
    let edit = &variants[2]["properties"]["edits"]["items"];
    assert_eq!(
        edit["required"],
        serde_json::json!(["old_string", "new_string"]),
        "replace_all is optional: the default is the unique-match rule"
    );
    assert!(
        edit["properties"]["old_string"]["minLength"] == serde_json::json!(1),
        "an empty anchor would match everywhere"
    );
    assert_eq!(
        variants[3]["required"],
        serde_json::json!(["kind", "path", "insert_line", "content"])
    );
    assert_eq!(
        variants[3]["properties"]["insert_line"]["minimum"],
        serde_json::json!(0),
        "0 is a legal insert point: the top of the file"
    );
    // rename, delete
    assert_eq!(
        variants[4]["required"],
        serde_json::json!(["kind", "from", "to"])
    );
    assert_eq!(variants[5]["required"], serde_json::json!(["kind", "path"]));
    // The directory kinds, which take the same paths as their file
    // counterparts -- a directory delete carries no extra argument because it
    // only ever removes an empty directory.
    assert_eq!(
        variants[6]["properties"]["kind"]["enum"],
        serde_json::json!(["rename_directory"])
    );
    assert_eq!(
        variants[6]["required"],
        serde_json::json!(["kind", "from", "to"])
    );
    assert_eq!(
        variants[7]["properties"]["kind"]["enum"],
        serde_json::json!(["delete_directory"])
    );
    assert_eq!(variants[7]["required"], serde_json::json!(["kind", "path"]));
}
