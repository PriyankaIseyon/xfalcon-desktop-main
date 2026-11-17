use serde::Deserialize;

/// Handwritten RFC 9728 overlay — generated OpenAPI declares an empty object.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct ProtectedResourceMetadata {
    pub(crate) resource: String,
    pub(crate) authorization_servers: Vec<String>,
}

impl ProtectedResourceMetadata {
    pub(crate) fn primary_authorization_server(&self) -> Option<&str> {
        self.authorization_servers.first().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_protected_resource_with_extension_fields() {
        let json = r#"{
            "resource":"https://tenant.example/mcp",
            "authorization_servers":["https://auth.example.com"],
            "bearer_methods_supported":["header"],
            "scopes_supported":["openid"]
        }"#;
        let parsed: ProtectedResourceMetadata = serde_json::from_str(json).expect("parse");
        assert_eq!(
            parsed.primary_authorization_server(),
            Some("https://auth.example.com")
        );
    }
}
