//! Wire vocabulary for file uploads.
//!
//! # What deliberately never crosses this boundary
//!
//! An upload's real machinery is a local absolute path, an open file handle, a
//! short-lived presigned S3 URL, and a storage key. None of them appear in any
//! type here, and that is the point of the type existing at all.
//!
//! - **The presigned URL is a bearer credential.** Anyone holding it can write
//!   to (or read from) tenant object storage until it expires, without an
//!   identity. Handing one to the webview would put a credential inside the
//!   one process that renders untrusted model output.
//! - **The local path is filesystem reach.** The webview has no business
//!   knowing where on disk the user picked a file from; the display name is
//!   what a person needs to see.
//! - **The storage key is tenant-shaped.** It describes where other tenants'
//!   objects live too.
//!
//! So the transfer itself runs entirely in `src-tauri/src/uploads`, and the
//! webview sees only [`UploadProjectionV1`]: what is happening, how far along,
//! and what to call it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct UploadId(String);

impl UploadId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for UploadId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for UploadId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// What an upload is for.
///
/// The two have genuinely different lifetimes and destinations — a chat
/// attachment belongs to one message and is fetched back through a signed GET,
/// while a persistent dataset becomes queryable tenant data — so they are not
/// one kind with a flag.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum UploadPurpose {
    /// Attached to a chat message.
    ChatAttachment,
    /// A dataset the workspace keeps and can query.
    PersistentDataset,
}

/// Where an upload is in its lifecycle.
///
/// ```text
/// selected ─▶ preparing ─▶ uploading ─▶ finalizing ─▶ ready
///                 │            │  ▲          │
///                 │            ▼  │          │
///                 │        resigning         │
///                 ▼            ▼             ▼
///              failed ◀────────┴─────────────┘
///                 │            │
///                 ▼            ▼
///             preparing    cancelled
/// ```
///
/// `Ready` and `Cancelled` are terminal. `Failed` is not: a failed upload can
/// be retried, which returns it to `Preparing` rather than resuming mid-flight
/// — a presigned URL that failed may simply have expired, and reusing it would
/// fail the same way.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum UploadState {
    /// Chosen by the user; nothing has been sent or even asked for yet.
    Selected,
    /// Asking DA to presign the transfer.
    Preparing,
    /// Sending bytes.
    Uploading,
    /// Re-presigning mid-transfer because URLs expired. Distinct from
    /// `Preparing` so progress is not misread as restarting from zero.
    Resigning,
    /// Bytes are sent; DA is completing the object and recording it.
    Finalizing,
    Ready,
    Failed,
    Cancelled,
}

impl UploadState {
    /// Whether no further transition is possible.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Ready | Self::Cancelled)
    }

    /// Whether bytes are moving or about to.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Uploading | Self::Resigning | Self::Finalizing
        )
    }
}

/// Everything the UI may know about one upload.
///
/// `safe_error` is a message written for a person, never a transport or
/// storage error forwarded verbatim: those can carry a URL, a key, or a
/// bucket name.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct UploadProjectionV1 {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: UploadId,
    pub purpose: UploadPurpose,
    pub state: UploadState,
    // Bytes confirmed sent. Never decreases within one attempt.
    //
    // `number`, not `bigint`: a file size that exceeds 2^53 bytes (9 PB) is
    // not a case this app has, and `bigint` would force every progress
    // calculation in the UI into a separate numeric tower.
    #[ts(type = "number")]
    pub bytes_sent: u64,
    // Total size in bytes, as measured locally before the transfer began.
    #[ts(type = "number")]
    pub size: u64,
    // The filename to show, already sanitized for display.
    pub display_name: String,
    pub safe_error: Option<String>,
    /// DA's chat-attachment or dataset id once the transfer is ready. The
    /// webview uses this to bind a chat attachment onto the next send. It is
    /// not a path, URL, or storage key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// Conversation this upload belongs to, when it is a chat attachment.
    /// Not a path, URL, or storage key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

impl UploadProjectionV1 {
    #[must_use]
    pub fn new(
        id: UploadId,
        purpose: UploadPurpose,
        state: UploadState,
        bytes_sent: u64,
        size: u64,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id,
            purpose,
            state,
            bytes_sent,
            size,
            display_name: display_name.into(),
            safe_error: None,
            resource_id: None,
            conversation_id: None,
        }
    }

    #[must_use]
    pub fn with_error(mut self, safe_error: impl Into<String>) -> Self {
        self.safe_error = Some(safe_error.into());
        self
    }

    #[must_use]
    pub fn with_resource_id(mut self, resource_id: impl Into<String>) -> Self {
        self.resource_id = Some(resource_id.into());
        self
    }

    #[must_use]
    pub fn with_conversation_id(mut self, conversation_id: impl Into<String>) -> Self {
        self.conversation_id = Some(conversation_id.into());
        self
    }

    /// Fraction complete in `0.0..=1.0`.
    ///
    /// A zero-byte file is complete the moment it is sent, not divided by
    /// zero.
    #[must_use]
    pub fn progress(&self) -> f64 {
        if self.size == 0 {
            return if self.state == UploadState::Ready {
                1.0
            } else {
                0.0
            };
        }
        #[allow(clippy::cast_precision_loss)]
        {
            (self.bytes_sent.min(self.size) as f64) / (self.size as f64)
        }
    }
}

/// The queue as the UI sees it, newest first.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct UploadQueueV1 {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    /** Monotonic for this process. Consumers ignore an older snapshot. */
    pub revision: u32,
    pub uploads: Vec<UploadProjectionV1>,
}

impl UploadQueueV1 {
    #[must_use]
    pub fn new(revision: u32, uploads: Vec<UploadProjectionV1>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            revision,
            uploads,
        }
    }
}
