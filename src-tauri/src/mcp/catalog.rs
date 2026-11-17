//! Tool discovery, schema-validator cache, and duplicate/malformed-schema
//! rejection for the bundled DA MCP server.
//!
//! Advertised tool metadata is untrusted input (`docs/threat-model.md`
//! TM-006): every schema is compiled through a pinned JSON Schema
//! implementation (`jsonschema`, with remote/file `$ref` resolution disabled
//! — see `../../Cargo.toml` and the decision record) before this
//! catalog will call the tool it belongs to, and a malformed schema or a
//! duplicate tool name fails the whole refresh closed rather than silently
//! dropping one tool.

use std::sync::Arc;

use indexmap::IndexMap;
use rmcp::model::{ProtocolVersion, Tool};
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use super::client::McpSession;

/// Hard ceiling on how many tools a single bundled server may advertise.
/// Not a product limit — a defense against an unbounded/hostile listing
/// exhausting memory before policy ever sees it.
const MAX_TOOLS: usize = 512;
const MAX_TOOL_NAME_BYTES: usize = 256;

/// What one tool's `annotations` block claims about its effects, reduced to the
/// two questions this app asks of it.
///
/// The annotations are self-declared by the server and validated by nobody, so
/// they are read the conservative way at every ambiguity — they can only ever
/// *relax* review, never tighten it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct McpToolHints {
    /// `readOnlyHint`, absent meaning `false`.
    ///
    /// Defaulting to "not read-only" is what makes an unannotated tool land in
    /// the approval-requiring risk tier
    /// (`crate::mcp::runtime::descriptor_for`). A server that says nothing about
    /// a tool must not get the safer classification by omission.
    pub(crate) read_only: bool,
    /// Whether a call may destroy something.
    ///
    /// `destructiveHint` defaults to `true` in the protocol, so silence here
    /// means "assume it destroys". The protocol also says the field is
    /// "meaningful only when `readOnlyHint == false`" — this build deliberately
    /// does **not** take that reading. A tool advertising *both* read-only and
    /// destructive has made contradictory claims, and the branch that acts on
    /// the read-only half is the branch that runs it with no human present.
    pub(crate) destructive: bool,
    /// `idempotentHint`, or `readOnlyHint` implying it: a call that can be
    /// repeated with the same arguments without further effect.
    pub(crate) idempotent: bool,
}

impl McpToolHints {
    pub(crate) fn from_annotations(annotations: Option<&rmcp::model::ToolAnnotations>) -> Self {
        let read_only = annotations
            .and_then(|annotations| annotations.read_only_hint)
            .unwrap_or(false);
        Self {
            read_only,
            destructive: annotations
                .and_then(|annotations| annotations.destructive_hint)
                .unwrap_or(!read_only),
            idempotent: read_only
                || annotations
                    .and_then(|annotations| annotations.idempotent_hint)
                    .unwrap_or(false),
        }
    }

    /// Whether the tool may run without a human reviewing the call, before the
    /// server's `permission_mode` is applied.
    #[must_use]
    pub(crate) fn unattendable(self) -> bool {
        self.read_only && !self.destructive
    }
}

/// Whether the server marked the tool as callable only from its own interface.
///
/// MCP Apps (`_meta.ui.visibility`) lets a server publish a tool for the HTML
/// view it ships, and not for the model: `["app"]` alone. A host must not offer
/// such a tool to the model, and this app hosts no MCP Apps view that could call
/// it, so it is left out of the catalog entirely. An absent or unreadable
/// visibility keeps the protocol default, which includes the model.
fn hidden_from_model(tool: &Tool) -> bool {
    tool.meta
        .as_ref()
        .and_then(|meta| meta.get("ui"))
        .and_then(|ui| ui.get("visibility"))
        .and_then(Value::as_array)
        .is_some_and(|visibility| {
            !visibility
                .iter()
                .any(|surface| surface.as_str() == Some("model"))
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct McpToolDescriptor {
    name: String,
    description: Option<String>,
    input_schema: Value,
    hints: McpToolHints,
}

impl McpToolDescriptor {
    #[must_use]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub(crate) fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    #[must_use]
    pub(crate) fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    #[must_use]
    pub(crate) fn hints(&self) -> McpToolHints {
        self.hints
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum McpCatalogError {
    #[error("bundled server advertised tool {0:?} more than once")]
    DuplicateTool(String),
    #[error("bundled server advertised a malformed input schema for tool {0:?}")]
    MalformedSchema(String),
    #[error("bundled server advertised a tool name exceeding {0} bytes")]
    NameTooLong(usize),
    #[error("bundled server advertised more than {0} tools")]
    TooManyTools(usize),
    #[error("listing tools from the bundled server failed: {0}")]
    Listing(String),
}

struct CachedTool {
    descriptor: McpToolDescriptor,
    validator: jsonschema::Validator,
}

/// A tool catalog snapshot, cached by the exact session generation, server
/// identity, and negotiated protocol version it was built from
/// (`docs/architecture.md` 9.1:
/// "Tool catalogs are cached by server identity and protocol version, then
/// refreshed on connection or capability change"). A caller compares
/// [`Self::is_valid_for`] against the *current* session before reusing a
/// cached instance. The opaque generation makes every successful reconnect a
/// cache miss even when it reaches the same endpoint and negotiates the same
/// protocol version.
pub(crate) struct McpToolCatalog {
    server_identity: String,
    protocol_version: ProtocolVersion,
    session_generation: Arc<()>,
    tools: IndexMap<String, CachedTool>,
}

impl McpToolCatalog {
    /// Lists every tool the session's peer advertises (`Peer::list_all_tools`
    /// already follows the `nextCursor` pagination loop to completion) and
    /// compiles/caches a schema validator for each.
    pub(crate) async fn refresh(session: &McpSession) -> Result<Self, McpCatalogError> {
        let raw_tools = session
            .peer()
            .list_all_tools()
            .await
            .map_err(|error| McpCatalogError::Listing(error.to_string()))?;
        let protocol_version = session
            .negotiated_protocol_version()
            .unwrap_or(super::client::PINNED_PROTOCOL_VERSION);
        Self::from_raw_for_generation(
            session.server_identity().to_owned(),
            protocol_version,
            Arc::clone(session.cache_generation()),
            raw_tools,
        )
    }

    fn from_raw(
        server_identity: String,
        protocol_version: ProtocolVersion,
        raw_tools: Vec<Tool>,
    ) -> Result<Self, McpCatalogError> {
        Self::from_raw_for_generation(server_identity, protocol_version, Arc::new(()), raw_tools)
    }

    fn from_raw_for_generation(
        server_identity: String,
        protocol_version: ProtocolVersion,
        session_generation: Arc<()>,
        raw_tools: Vec<Tool>,
    ) -> Result<Self, McpCatalogError> {
        if raw_tools.len() > MAX_TOOLS {
            return Err(McpCatalogError::TooManyTools(MAX_TOOLS));
        }
        let mut tools = IndexMap::with_capacity(raw_tools.len());
        for tool in raw_tools {
            if hidden_from_model(&tool) {
                continue;
            }
            let name = tool.name.to_string();
            if name.len() > MAX_TOOL_NAME_BYTES {
                return Err(McpCatalogError::NameTooLong(MAX_TOOL_NAME_BYTES));
            }
            if tools.contains_key(&name) {
                return Err(McpCatalogError::DuplicateTool(name));
            }
            let schema_value = Value::Object(tool.input_schema.as_ref().clone());
            let validator = jsonschema::validator_for(&schema_value)
                .map_err(|_| McpCatalogError::MalformedSchema(name.clone()))?;
            let descriptor = McpToolDescriptor {
                name: name.clone(),
                description: tool.description.map(|value| value.to_string()),
                input_schema: schema_value,
                hints: McpToolHints::from_annotations(tool.annotations.as_ref()),
            };
            tools.insert(
                name,
                CachedTool {
                    descriptor,
                    validator,
                },
            );
        }
        Ok(Self {
            server_identity,
            protocol_version,
            session_generation,
            tools,
        })
    }

    #[must_use]
    pub(crate) fn is_valid_for(
        &self,
        server_identity: &str,
        protocol_version: &ProtocolVersion,
        session_generation: &Arc<()>,
    ) -> bool {
        self.server_identity == server_identity
            && &self.protocol_version == protocol_version
            && Arc::ptr_eq(&self.session_generation, session_generation)
    }

    #[must_use]
    pub(crate) fn get(&self, name: &str) -> Option<&McpToolDescriptor> {
        self.tools.get(name).map(|cached| &cached.descriptor)
    }

    /// Validates `arguments` against the tool's advertised, compiled schema.
    /// Returns `Ok(())` only if the tool is known and the schema accepts the
    /// arguments — model/skill-supplied arguments never bypass this.
    pub(crate) fn validate_arguments(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<(), ArgumentsError> {
        let cached = self
            .tools
            .get(name)
            .ok_or_else(|| ArgumentsError::UnknownTool(name.to_owned()))?;
        if cached.validator.is_valid(arguments) {
            Ok(())
        } else {
            Err(ArgumentsError::SchemaViolation)
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &McpToolDescriptor> {
        self.tools.values().map(|cached| &cached.descriptor)
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.tools.len()
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/// The actual cache [`McpToolCatalog::is_valid_for`] exists to serve: holds
/// the last successfully-built catalog and reuses it across calls instead of
/// re-fetching, per the design spec's "Tool catalogs are cached by server
/// identity and protocol version, then refreshed on connection or capability
/// change" (`docs/architecture.md`
/// 9.1).
///
/// Two independent triggers force a refetch, matching that sentence
/// exactly:
/// - **"on connection"** — [`McpToolCatalog::is_valid_for`] returns `false`
///   whenever `session`'s opaque generation, `server_identity`, or negotiated
///   protocol version no longer match what the cached catalog was built from.
///   A fresh connection therefore refreshes even at the same endpoint with
///   the same pinned protocol version.
/// - **"on capability change"** — a `notifications/tools/list_changed` the
///   server pushed since the cache was last checked
///   ([`super::client::McpNotifications::take_tool_list_changed`], which is
///   destructive-once by design: reading it here is exactly "have I already
///   accounted for the most recent change").
///
/// Holds an `Arc<McpToolCatalog>` rather than the catalog by value so a
/// cache hit never needs to clone `jsonschema::Validator`s (not proven
/// cheaply `Clone`). The async lock intentionally spans one bounded refresh,
/// so concurrent callers cannot start duplicate `tools/list` cycles or race a
/// stale snapshot back into the cache.
#[derive(Default)]
pub(crate) struct McpToolCatalogCache {
    cached: AsyncMutex<Option<Arc<McpToolCatalog>>>,
}

impl McpToolCatalogCache {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the cached catalog if it is still valid for `session` and no
    /// capability-change notification arrived since the last call;
    /// otherwise refreshes from `session` and caches the result.
    pub(crate) async fn get_or_refresh(
        &self,
        session: &McpSession,
    ) -> Result<Arc<McpToolCatalog>, McpCatalogError> {
        let mut cached = self.cached.lock().await;

        // Destructive-once, under the refresh lock: no concurrent caller can
        // consume the notification and let this caller return the old value.
        let capability_changed = session.notifications().take_tool_list_changed();
        if capability_changed {
            // The notification is now consumed. Remove the old validators
            // before the fallible await so a failed or dropped refresh cannot
            // make the next caller treat that old snapshot as clean.
            *cached = None;
        }

        if !capability_changed {
            let hit = cached
                .as_ref()
                .filter(|catalog| {
                    catalog.is_valid_for(
                        session.server_identity(),
                        &session
                            .negotiated_protocol_version()
                            .unwrap_or(super::client::PINNED_PROTOCOL_VERSION),
                        session.cache_generation(),
                    )
                })
                .cloned();
            if let Some(catalog) = hit {
                return Ok(catalog);
            }
        }

        let fresh = Arc::new(McpToolCatalog::refresh(session).await?);
        *cached = Some(fresh.clone());
        Ok(fresh)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ArgumentsError {
    #[error("unknown mcp tool {0:?}")]
    UnknownTool(String),
    #[error("tool arguments do not match the advertised schema")]
    SchemaViolation,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ProtocolVersion;
    use serde_json::json;

    fn tool(name: &str, schema: Value) -> Tool {
        Tool::new(
            name.to_owned(),
            "a test tool".to_owned(),
            schema.as_object().cloned().unwrap_or_default(),
        )
    }

    /// `McpToolCatalog` deliberately does not derive `Debug` (it holds
    /// compiled `jsonschema::Validator`s), so tests unwrap failure results
    /// manually rather than via `Result::unwrap_err`/`expect_err`, both of
    /// which require the `Ok` type to implement `Debug`.
    fn expect_err(result: Result<McpToolCatalog, McpCatalogError>) -> McpCatalogError {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(err) => err,
        }
    }

    #[test]
    fn compiles_valid_schemas_and_validates_arguments() {
        let schema = json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"],
            "additionalProperties": false,
        });
        let catalog = McpToolCatalog::from_raw(
            "https://tenant.example/mcp".to_owned(),
            ProtocolVersion::V_2025_11_25,
            vec![tool("search", schema)],
        )
        .expect("valid schema compiles");

        assert_eq!(catalog.len(), 1);
        assert!(catalog
            .validate_arguments("search", &json!({"query": "hello"}))
            .is_ok());
        assert_eq!(
            catalog.validate_arguments("search", &json!({"query": 5})),
            Err(ArgumentsError::SchemaViolation)
        );
        assert_eq!(
            catalog.validate_arguments("search", &json!({})),
            Err(ArgumentsError::SchemaViolation)
        );
    }

    fn with_visibility(name: &str, visibility: Value) -> Tool {
        let mut raw = tool(name, json!({"type": "object"}));
        raw.meta = Some(rmcp::model::Meta(
            json!({ "ui": { "resourceUri": "ui://kit-viewer", "visibility": visibility } })
                .as_object()
                .cloned()
                .expect("object"),
        ));
        raw
    }

    #[test]
    fn a_tool_only_its_own_app_may_call_is_left_out() {
        let catalog = McpToolCatalog::from_raw(
            "https://tenant.example/mcp".to_owned(),
            ProtocolVersion::V_2025_11_25,
            vec![
                tool("plain", json!({"type": "object"})),
                with_visibility("both", json!(["model", "app"])),
                with_visibility("model_only", json!(["model"])),
                with_visibility("app_only", json!(["app"])),
                with_visibility("nowhere", json!([])),
                // Unreadable: the protocol default, which includes the model.
                with_visibility("garbled", json!("app")),
            ],
        )
        .expect("catalog builds");

        let names: Vec<&str> = catalog.iter().map(McpToolDescriptor::name).collect();
        assert_eq!(names, ["plain", "both", "model_only", "garbled"]);
        assert_eq!(
            catalog.validate_arguments("app_only", &json!({})),
            Err(ArgumentsError::UnknownTool("app_only".to_owned()))
        );
    }

    #[test]
    fn rejects_unknown_tool_name() {
        let catalog = McpToolCatalog::from_raw(
            "https://tenant.example/mcp".to_owned(),
            ProtocolVersion::V_2025_11_25,
            vec![tool("search", json!({"type": "object"}))],
        )
        .expect("valid schema compiles");

        assert_eq!(
            catalog.validate_arguments("delete_everything", &json!({})),
            Err(ArgumentsError::UnknownTool("delete_everything".to_owned()))
        );
    }

    #[test]
    fn rejects_duplicate_tool_names() {
        let schema = json!({"type": "object"});
        let err = expect_err(McpToolCatalog::from_raw(
            "https://tenant.example/mcp".to_owned(),
            ProtocolVersion::V_2025_11_25,
            vec![tool("search", schema.clone()), tool("search", schema)],
        ));

        assert!(matches!(err, McpCatalogError::DuplicateTool(name) if name == "search"));
    }

    #[test]
    fn rejects_malformed_schema() {
        // `type` must be a string or array of strings per JSON Schema; a
        // number is malformed and must fail to compile, not be silently
        // accepted as "anything goes".
        let schema = json!({"type": 12345});
        let err = expect_err(McpToolCatalog::from_raw(
            "https://tenant.example/mcp".to_owned(),
            ProtocolVersion::V_2025_11_25,
            vec![tool("broken", schema)],
        ));

        assert!(matches!(err, McpCatalogError::MalformedSchema(name) if name == "broken"));
    }

    #[test]
    fn rejects_schema_with_unresolvable_remote_ref() {
        // `resolve-http`/`resolve-file` are disabled in Cargo.toml
        // deliberately (see the decision record): a schema that
        // requires fetching a remote/local `$ref` must fail to compile
        // rather than the client making an SSRF-shaped outbound request.
        let schema = json!({"$ref": "https://attacker.example/schema.json"});
        let err = expect_err(McpToolCatalog::from_raw(
            "https://tenant.example/mcp".to_owned(),
            ProtocolVersion::V_2025_11_25,
            vec![tool("remote_ref", schema)],
        ));

        assert!(matches!(err, McpCatalogError::MalformedSchema(name) if name == "remote_ref"));
    }

    #[test]
    fn rejects_too_many_tools() {
        let schema = json!({"type": "object"});
        let tools: Vec<Tool> = (0..(MAX_TOOLS + 1))
            .map(|index| tool(&format!("tool_{index}"), schema.clone()))
            .collect();
        let err = expect_err(McpToolCatalog::from_raw(
            "https://tenant.example/mcp".to_owned(),
            ProtocolVersion::V_2025_11_25,
            tools,
        ));
        assert!(matches!(err, McpCatalogError::TooManyTools(MAX_TOOLS)));
    }

    #[test]
    fn is_valid_for_checks_identity_and_protocol_version() {
        let catalog = McpToolCatalog::from_raw(
            "https://tenant.example/mcp".to_owned(),
            ProtocolVersion::V_2025_11_25,
            vec![],
        )
        .expect("empty catalog compiles");
        let generation = Arc::clone(&catalog.session_generation);

        assert!(catalog.is_valid_for(
            "https://tenant.example/mcp",
            &ProtocolVersion::V_2025_11_25,
            &generation,
        ));
        assert!(!catalog.is_valid_for(
            "https://other.example/mcp",
            &ProtocolVersion::V_2025_11_25,
            &generation,
        ));
        assert!(!catalog.is_valid_for(
            "https://tenant.example/mcp",
            &ProtocolVersion::V_2025_06_18,
            &generation,
        ));
        assert!(!catalog.is_valid_for(
            "https://tenant.example/mcp",
            &ProtocolVersion::V_2025_11_25,
            &Arc::new(()),
        ));
    }
}
