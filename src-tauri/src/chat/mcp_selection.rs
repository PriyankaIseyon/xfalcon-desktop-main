use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use xfalcon_contracts::chat::ChatMcpSelectionView;
use xfalcon_contracts::mcp::{McpServerId, BUNDLED_SERVER_ID};

const METADATA_KEY: &str = "xfalcon_desktop_mcp";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpConversationBinding {
    server_id: Option<McpServerId>,
    endpoint: Option<String>,
}

#[derive(Debug, Error)]
pub(crate) enum BindingError {
    #[error("stored MCP chat selection is invalid")]
    Invalid,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredBinding {
    version: u8,
    #[serde(deserialize_with = "deserialize_required")]
    server_id: Option<String>,
    #[serde(deserialize_with = "deserialize_required")]
    endpoint: Option<String>,
}

fn deserialize_required<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

impl McpConversationBinding {
    pub(crate) fn all() -> Self {
        Self {
            server_id: None,
            endpoint: None,
        }
    }

    pub(crate) fn server(server_id: McpServerId, endpoint: String) -> Result<Self, BindingError> {
        validate_server_id(server_id.as_str())?;
        let endpoint = crate::mcp::registry::validate_endpoint(&endpoint)
            .map_err(|_| BindingError::Invalid)?
            .to_string();
        Ok(Self {
            server_id: Some(server_id),
            endpoint: Some(endpoint),
        })
    }

    pub(crate) fn from_metadata(metadata: &Map<String, Value>) -> Result<Self, BindingError> {
        let Some(value) = metadata.get(METADATA_KEY) else {
            return Ok(Self::all());
        };
        let stored: StoredBinding =
            serde_json::from_value(value.clone()).map_err(|_| BindingError::Invalid)?;
        if stored.version != 1 {
            return Err(BindingError::Invalid);
        }
        match (stored.server_id, stored.endpoint) {
            (None, None) => Ok(Self::all()),
            (Some(server_id), Some(endpoint)) => {
                validate_server_id(&server_id)?;
                let normalized = crate::mcp::registry::validate_endpoint(&endpoint)
                    .map_err(|_| BindingError::Invalid)?;
                if normalized.as_str() != endpoint {
                    return Err(BindingError::Invalid);
                }
                Ok(Self {
                    server_id: Some(McpServerId::from(server_id)),
                    endpoint: Some(endpoint),
                })
            }
            _ => Err(BindingError::Invalid),
        }
    }

    pub(crate) fn metadata(&self) -> Map<String, Value> {
        let stored = StoredBinding {
            version: 1,
            server_id: self.server_id.as_ref().map(|id| id.as_str().to_owned()),
            endpoint: self.endpoint.clone(),
        };
        let mut metadata = Map::new();
        metadata.insert(
            METADATA_KEY.to_owned(),
            serde_json::to_value(stored).expect("binding fields always serialize"),
        );
        metadata
    }

    pub(crate) fn server_id(&self) -> Option<&McpServerId> {
        self.server_id.as_ref()
    }

    pub(crate) fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    pub(crate) fn view(&self) -> ChatMcpSelectionView {
        ChatMcpSelectionView::new(self.server_id.clone())
    }

    pub(crate) fn matches_selection(&self, selected: Option<&McpServerId>) -> bool {
        self.server_id.as_ref() == selected
    }
}

fn validate_server_id(server_id: &str) -> Result<(), BindingError> {
    if server_id == BUNDLED_SERVER_ID || uuid::Uuid::parse_str(server_id).is_ok() {
        Ok(())
    } else {
        Err(BindingError::Invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SERVER_ID: &str = "9c7ed740-8149-4f0c-8717-e4c99897e128";
    const ENDPOINT: &str = "https://mcp.example.com/mcp";

    fn metadata_with(value: Value) -> Map<String, Value> {
        Map::from_iter([(METADATA_KEY.to_owned(), value)])
    }

    #[test]
    fn all_and_specific_bindings_round_trip() {
        let all = McpConversationBinding::all();
        assert_eq!(
            McpConversationBinding::from_metadata(&all.metadata()).unwrap(),
            all
        );

        for server_id in [SERVER_ID, BUNDLED_SERVER_ID] {
            let binding =
                McpConversationBinding::server(McpServerId::from(server_id), ENDPOINT.to_owned())
                    .unwrap();
            assert_eq!(
                binding.server_id().map(McpServerId::as_str),
                Some(server_id)
            );
            assert_eq!(binding.endpoint(), Some(ENDPOINT));
            assert_eq!(
                McpConversationBinding::from_metadata(&binding.metadata()).unwrap(),
                binding
            );
        }
    }

    #[test]
    fn absent_reserved_field_is_legacy_all() {
        assert_eq!(
            McpConversationBinding::from_metadata(&Map::new()).unwrap(),
            McpConversationBinding::all()
        );
    }

    #[test]
    fn malformed_null_unknown_and_incoherent_bindings_are_rejected() {
        for value in [
            Value::Null,
            json!({"version": 2, "server_id": null, "endpoint": null}),
            json!({"version": 1, "server_id": null, "endpoint": null, "extra": true}),
            json!({"version": 1}),
            json!({"version": 1, "server_id": SERVER_ID, "endpoint": null}),
            json!({"version": 1, "server_id": null, "endpoint": ENDPOINT}),
            json!({"version": 1, "server_id": "not-a-uuid", "endpoint": ENDPOINT}),
            json!({"version": 1, "server_id": SERVER_ID, "endpoint": "https://user:secret@mcp.example.com/mcp"}),
            json!({"version": 1, "server_id": SERVER_ID, "endpoint": "https://mcp.example.com/mcp?token=secret"}),
        ] {
            assert!(McpConversationBinding::from_metadata(&metadata_with(value)).is_err());
        }
    }

    #[test]
    fn constructor_normalizes_endpoint_but_metadata_requires_canonical_endpoint() {
        let binding = McpConversationBinding::server(
            McpServerId::from(SERVER_ID),
            "https://MCP.EXAMPLE.COM:443/mcp".to_owned(),
        )
        .unwrap();
        assert_eq!(binding.endpoint(), Some(ENDPOINT));
        let stored = json!({"version": 1, "server_id": SERVER_ID, "endpoint": "https://MCP.EXAMPLE.COM:443/mcp"});
        assert!(McpConversationBinding::from_metadata(&metadata_with(stored)).is_err());
    }

    #[test]
    fn view_and_selection_matching_preserve_server_scope() {
        let binding =
            McpConversationBinding::server(McpServerId::from(SERVER_ID), ENDPOINT.to_owned())
                .unwrap();
        let selected = McpServerId::from(SERVER_ID);
        assert_eq!(binding.view().server_id, Some(selected.clone()));
        assert!(binding.matches_selection(Some(&selected)));
        assert!(!binding.matches_selection(None));
        assert!(!binding.matches_selection(Some(&McpServerId::from(BUNDLED_SERVER_ID))));
    }
}
