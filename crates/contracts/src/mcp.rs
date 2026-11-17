//! Wire vocabulary for user-registered MCP servers.
//!
//! Server definitions live in DA (`/api/v1/chat/mcp-servers`) so desktop and
//! xFalcon Web operate on one registry rather than two. What crosses *this*
//! boundary is a bounded projection of that row, and never DA's opaque
//! `auth_config` column: DA stores it encrypted under a key that never leaves
//! xfalcon-web, so desktop could not read it even if this boundary carried it.
//!
//! A registered server's own credential is sealed by
//! `src-tauri/src/credentials::CredentialVault` into the OS keychain and stays
//! on the device. [`McpServerInput`] therefore has no field for it — the
//! command surface takes it as a separate argument, the same way
//! `credential_create` does, so it cannot ride along inside a struct that gets
//! logged or echoed back. An `oauth` row's operator-issued client pair travels
//! the same way, as two separate arguments to `mcp_server_create`,
//! `mcp_server_update` and `mcp_server_sign_in`.
//!
//! [`McpServerView`] does report the client *identifier* a device holds
//! ([`McpServerView::oauth_client_id`]), because an edit sheet that cannot show
//! it presents an empty field over a saved value and reads as data loss. The
//! confidential half is reported only as the boolean
//! [`McpServerView::oauth_client_confidential`].
//!
//! Doc comments on the types below are emitted into
//! `src/lib/generated/contracts.ts` by `ts-rs`, and
//! `crates/contracts/tests/contract_snapshots.rs` scans that whole file for
//! credential-shaped vocabulary as a leak tripwire. Keep the wording here
//! clear of it; the reasoning that needs those words belongs in this module
//! comment (which is not emitted) or in `src-tauri`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

/// A registered server's DA row id, or the reserved literal `tenant` for the
/// bundled server (which has no DA row — it is derived from the configured DA
/// origin, so nothing registers or deletes it).
#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct McpServerId(String);

/// The bundled tenant DA MCP server's reserved id and tool-namespace slug.
/// Registration rejects this name so a DA row can never shadow it.
pub const BUNDLED_SERVER_ID: &str = "tenant";

impl McpServerId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_bundled(&self) -> bool {
        self.0 == BUNDLED_SERVER_ID
    }
}

impl From<&str> for McpServerId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for McpServerId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// DA's full transport vocabulary.
///
/// Desktop speaks one wire protocol, MCP Streamable HTTP, and dials any row
/// that names a remote HTTP endpoint — [`McpTransport::Http`] and
/// [`McpTransport::Sse`] alike. The two are not distinguished because no client
/// writing these rows distinguishes them: xFalcon Web labels every server it
/// registers `sse` and then speaks Streamable HTTP to it. See
/// `crate::mcp::registry::verdict_for` in the desktop crate for the full
/// reasoning. [`McpTransport::Stdio`] and [`McpTransport::Other`] name no
/// remote endpoint and are still listed honestly, with
/// [`McpServerUnavailableReason::UnsupportedTransport`], rather than hidden.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum McpTransport {
    Http,
    Sse,
    Stdio,
    /// DA named a transport outside the vocabulary this build knows. A later
    /// DA adding one must leave a server list readable, not break it.
    Other,
}

/// How a server identifies this client.
///
/// Maps onto DA's `auth_type` column (`none` / `api_key` / `bearer` /
/// `oauth`), but names each arm for what the desktop actually does with it:
/// [`Self::Header`] puts the device-held value in the custom header named by
/// [`McpServerInput::header_name`], [`Self::Bearer`] uses the standard bearer
/// scheme, and [`Self::Oauth`] runs the MCP spec's own delegated flow in the
/// system browser and keeps the resulting grant on this device
/// ([`McpSignInStatus`]).
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum McpAuthType {
    None,
    Header,
    Bearer,
    Oauth,
    /// DA named a method outside the vocabulary this build knows. Same reason
    /// as [`McpTransport::Other`].
    Other,
}

impl McpAuthType {
    /// Whether this method needs a value the user pastes in and this device
    /// holds verbatim.
    ///
    /// Deliberately false for [`Self::Oauth`]: that method also needs
    /// device-held material, but the user never sees or enters it — it is
    /// obtained by [`needs_sign_in`](Self::needs_sign_in)'s browser flow, so a
    /// surface that asks for a pasted value would be asking the wrong question.
    #[must_use]
    pub const fn needs_device_credential(self) -> bool {
        matches!(self, Self::Header | Self::Bearer)
    }

    /// Whether this method is established by a delegated browser flow whose
    /// result is held per device.
    #[must_use]
    pub const fn needs_sign_in(self) -> bool {
        matches!(self, Self::Oauth)
    }

    /// Whether this build can dial a server using this method at all.
    #[must_use]
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::None | Self::Header | Self::Bearer | Self::Oauth)
    }
}

/// Whether there is a usable grant for an [`McpAuthType::Oauth`] server, and how
/// far it reaches. `None` on [`McpServerView::sign_in`] for every other method.
///
/// An aged-out access token is still signed in: it is renewed on the next connect
/// without involving the user. Only a grant that cannot be renewed at all becomes
/// [`Self::NotSignedIn`], and that is discovered when the renewal is refused.
///
/// The distinction between the two signed-in arms is not cosmetic — it decides
/// what signing out does to other people. A device grant is this machine's alone;
/// an account grant is the row xFalcon on the web uses too, so signing out of it
/// signs out every client in the workspace.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum McpSignInStatus {
    NotSignedIn,
    /// A grant held by this device only.
    SignedIn,
    /// The workspace-wide grant, shared with every other client signed in to this
    /// account.
    SignedInForAccount,
}

impl McpSignInStatus {
    /// Whether a server with this status can be connected to at all.
    #[must_use]
    pub const fn is_usable(self) -> bool {
        matches!(self, Self::SignedIn | Self::SignedInForAccount)
    }
}

/// How much review a server's tool calls get. This is an operator dial layered
/// *above* the MCP tool annotations, never a replacement for them: `Allow` still
/// routes a mutating, destructive, or unannotated call through approval, because
/// a server's self-declaration is not authority to mutate unreviewed.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum McpPermissionMode {
    /// Tools annotated read-only and not destructive run unattended; everything
    /// else still pauses.
    Allow,
    /// Every call pauses for approval. The default, including when DA's column
    /// is null.
    #[default]
    Ask,
    /// Tools are not registered at all — the model never sees them.
    Deny,
}

/// Why a listed server cannot be used from this device. `None` on
/// [`McpServerView::unavailable`] means usable.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum McpServerUnavailableReason {
    /// Not MCP Streamable HTTP.
    UnsupportedTransport,
    /// The row names a sign-in method outside this build's vocabulary
    /// ([`McpAuthType::Other`]), or names a custom header without saying which
    /// header.
    UnsupportedAuthMethod,
    /// An [`McpAuthType::Oauth`] server this device has not signed in to yet.
    /// Recoverable here — the flow runs in the system browser and stores its
    /// grant on this device.
    NeedsSignIn,
    /// The server expects a value this device does not hold — typically a row
    /// registered on another device or on the web.
    MissingDeviceCredential,
    /// The row's `url` is absent, or fails the connect-boundary check
    /// (not https, embedded userinfo, a query or fragment, or a host this
    /// build refuses to dial).
    InvalidUrl,
}

/// The result of one connection attempt, held per device and never written
/// back to DA's shared `status` columns — a laptop behind a captive portal
/// must not mark a healthy server broken for the whole tenant.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum McpConnectionState {
    Connected,
    /// Reached, and it refused this client.
    Rejected,
    /// Could not be reached, or the MCP handshake failed.
    Unreachable,
    /// Never dialed — see [`McpServerView::unavailable`].
    NotAttempted,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct McpServerView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: McpServerId,
    /// Unique per account in DA, and the basis of this server's tool namespace.
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub transport: McpTransport,
    pub url: Option<String>,
    pub auth_type: McpAuthType,
    pub permission_mode: McpPermissionMode,
    pub enabled: bool,
    /// True only for the bundled tenant server: it has no DA row, cannot be
    /// edited or deleted, and rides the signed-in session.
    pub builtin: bool,
    /// The tool-id namespace segment (`mcp.<slug>.<tool>`).
    pub slug: String,
    pub unavailable: Option<McpServerUnavailableReason>,
    /// Present only when `auth_type` is [`McpAuthType::Oauth`]: whether this
    /// device already holds a grant, so the surface can offer signing in or
    /// signing out rather than guessing which applies.
    pub sign_in: Option<McpSignInStatus>,
    /// The client identifier this device holds for an [`McpAuthType::Oauth`]
    /// row, when an operator supplied one. `None` means this device has none
    /// and sign-in resolves a client on its own.
    ///
    /// Not confidential -- it names a registration, it does not authenticate
    /// one -- so the edit sheet can show what is stored and edit it in place
    /// instead of presenting an empty field over a saved value.
    pub oauth_client_id: Option<String>,
    /// Whether this device also holds the confidential value issued for
    /// [`Self::oauth_client_id`], so the surface can say that leaving the
    /// field blank keeps it.
    pub oauth_client_confidential: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct McpServerListResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub servers: Vec<McpServerView>,
}

impl McpServerListResult {
    pub fn new(servers: Vec<McpServerView>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            servers,
        }
    }
}

/// Create/update input. Deliberately carries nothing confidential (see the
/// module comment) and no transport field: desktop registers Streamable HTTP
/// servers, so the transport is not a choice the UI gets to make wrong.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct McpServerInput {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub url: String,
    pub auth_type: McpAuthType,
    /// Custom header name, required when `auth_type` is
    /// [`McpAuthType::Header`] and ignored otherwise.
    pub header_name: Option<String>,
    pub permission_mode: McpPermissionMode,
    pub enabled: bool,
}

/// One tool a connected server advertises, as shown in its Settings detail.
/// The advertised JSON Schema stays in the runtime — the UI has no use for it
/// and it is untrusted input.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct McpToolView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    /// The namespaced tool id the model and the approval prompt both see.
    pub tool_id: String,
    /// The bare name the server advertised.
    pub name: String,
    pub description: Option<String>,
    /// Whether a call pauses for approval under this server's
    /// [`McpPermissionMode`] and the tool's own annotations.
    pub requires_approval: bool,
}

impl McpToolView {
    pub fn new(
        tool_id: impl Into<String>,
        name: impl Into<String>,
        description: Option<String>,
        requires_approval: bool,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            tool_id: tool_id.into(),
            name: name.into(),
            description,
            requires_approval,
        }
    }
}

/// Outcome of `mcp_server_test`: what one connection attempt found.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct McpConnectionReport {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: McpServerId,
    pub state: McpConnectionState,
    /// Tools advertised, when the handshake succeeded.
    pub tools: Vec<McpToolView>,
    /// A short, already-redacted explanation for a non-`Connected` state.
    pub message: Option<String>,
}

impl McpConnectionReport {
    pub fn connected(id: impl Into<McpServerId>, tools: Vec<McpToolView>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id: id.into(),
            state: McpConnectionState::Connected,
            tools,
            message: None,
        }
    }

    pub fn failed(
        id: impl Into<McpServerId>,
        state: McpConnectionState,
        message: impl Into<String>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id: id.into(),
            state,
            tools: Vec::new(),
            message: Some(message.into()),
        }
    }
}

/// Why a server refused the credential it was given.
///
/// Two variants rather than one because the words differ: a refused grant is
/// replaced, while a grant that is merely too narrow was accepted as genuine
/// and needs a wider one. The action — sign in again — is the same, and the
/// prompt says so either way.
///
/// Deliberately spelled without the word this repo's contract guard forbids
/// anywhere in generated TypeScript, including doc comments: nothing describing
/// a credential may look like a field that carries one.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum McpReauthReason {
    /// The server refused the credential: expired, revoked, or minted for the
    /// wrong audience.
    CredentialRefused,
    /// The credential is real and does not cover what the server requires.
    InsufficientScope,
}

/// A server refused its credential during a run, and the user is being asked to
/// sign in again.
///
/// Carried as its own event rather than as a field on a tool-invocation
/// failure: the prompt is app-global, and the refusal can happen while
/// *establishing* the session, which belongs to no invocation. Naming the
/// server is the point — a prompt that cannot say which server to reconnect
/// leaves the user to find it.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct McpReauthRequired {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    /// The server to reconnect — the id `mcp_server_sign_in` takes.
    pub id: McpServerId,
    /// What to call it in the prompt: the row's display name when it has one,
    /// otherwise its name.
    pub label: String,
    pub reason: McpReauthReason,
}

impl McpReauthRequired {
    pub fn new(
        id: impl Into<McpServerId>,
        label: impl Into<String>,
        reason: McpReauthReason,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id: id.into(),
            label: label.into(),
            reason,
        }
    }
}

/// Redacted failure vocabulary for the `mcp_server_*` commands, mirroring
/// `crate::credentials::CredentialErrorCode`: a code the UI can branch on,
/// never a DA or server message passed through verbatim.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum McpServerErrorCode {
    /// Malformed input: bad name, bad URL, or a reserved name.
    InvalidRequest,
    /// The URL is well-formed but this build refuses to dial it.
    UrlRejected,
    /// A server with that name already exists for this account.
    Conflict,
    /// The bundled server, or a row this account does not own.
    NotEditable,
    NotFound,
    /// No signed-in session.
    Unauthorized,
    /// The device keychain could not store or retrieve the registered value.
    StorageUnavailable,
    /// DA could not be reached, or answered outside its contract.
    ServiceUnavailable,
    /// The server published no usable OAuth description: no RFC 9728 Protected
    /// Resource Metadata, or metadata this build will not act on (an
    /// authorization server it cannot reach, or a resource identifier that is
    /// not the server being dialed).
    DiscoveryFailed,
    /// The server's authorization server neither offers RFC 7591 dynamic client
    /// registration nor has a client this device already knows, so the sign-in
    /// cannot start until the user supplies the client id the operator issued.
    ClientRegistrationUnavailable,
    /// The user closed the browser, or the authorization server reported that
    /// consent was refused.
    SignInDeclined,
    /// No callback arrived before the flow's deadline.
    SignInTimedOut,
    /// The callback arrived and the exchange for a grant was refused.
    SignInFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct McpServerOperationError {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub code: McpServerErrorCode,
}

impl McpServerOperationError {
    pub fn new(code: McpServerErrorCode) -> Self {
        Self {
            version: CONTRACT_VERSION,
            code,
        }
    }
}
