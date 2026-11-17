use reqwest::Url;
use serde_json::json;

use super::super::{chat_api::summarize_mcp_servers, generated::WorkspaceBootstrap};

#[test]
fn workspace_mcp_summary_excludes_only_the_bundled_endpoint_alias() {
    let bootstrap = WorkspaceBootstrap {
        mcp_servers: Some(vec![
            json!({
                "id": "tenant-alias",
                "name": "staging",
                "transport": "sse",
                "url": "https://TENANT.example:443/mcp",
                "is_enabled": true
            }),
            json!({
                "id": "additional",
                "name": "Acme CRM",
                "transport": "http",
                "url": "https://mcp.acme.example/mcp",
                "is_enabled": true
            }),
        ]),
        ..WorkspaceBootstrap::default()
    };

    let summaries = summarize_mcp_servers(
        &bootstrap,
        &Url::parse("https://tenant.example/mcp").expect("bundled endpoint"),
    );

    assert_eq!(
        summaries,
        vec![(
            "additional".to_owned(),
            "Acme CRM".to_owned(),
            "http".to_owned(),
            true,
        )]
    );
}

#[test]
fn malformed_or_distinct_workspace_rows_keep_the_existing_summary_behavior() {
    let bootstrap = WorkspaceBootstrap {
        mcp_servers: Some(vec![
            json!({"id": "missing-url", "name": "Missing URL"}),
            json!({"id": "bad-url", "name": "Bad URL", "url": "not a url"}),
            json!({
                "id": "distinct-path",
                "name": "Distinct Path",
                "url": "https://tenant.example/mcp/"
            }),
        ]),
        ..WorkspaceBootstrap::default()
    };

    let summaries = summarize_mcp_servers(
        &bootstrap,
        &Url::parse("https://tenant.example/mcp").expect("bundled endpoint"),
    );

    assert_eq!(summaries.len(), 3);
}
