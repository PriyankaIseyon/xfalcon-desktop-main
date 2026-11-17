//! DA's upload APIs, behind two narrow ports.
//!
//! Two ports rather than one because the two flows genuinely differ. A
//! persistent dataset is presigned, transferred (possibly in parts), finalized
//! into queryable tenant data, and can be listed, re-signed, aborted, and
//! deleted. A chat attachment is presigned once, transferred whole, finalized,
//! and afterwards only ever read back through a short-lived signed GET. Fusing
//! them would give each flow methods that are meaningless for it.
//!
//! # Presigned URLs and storage keys are credentials
//!
//! Everything this module returns that carries a presigned URL is wrapped in
//! [`Presigned`], whose `Debug` prints nothing but the field's presence. That
//! is deliberate and load-bearing: a URL like this is a bearer credential for
//! tenant object storage, and the single most likely way to leak one is a
//! `{:?}` on an error path. The same applies to `storage_key`, which describes
//! where other tenants' objects live too.
//!
//! Nothing here is `Serialize`. These types cannot reach the webview even by
//! accident; `crate::uploads::domain::UploadRecord::projection` is the only
//! path out.

use std::net::IpAddr;

use async_trait::async_trait;
use reqwest::Url;

use crate::clients::da::chat_api::DaChatApiError;
use crate::clients::da::generated::{
    AbortRequest, ChatAttachment, ChatAttachmentCreate, ChatAttachmentCreateKind,
    ChatAttachmentCreateStatus, ChatAttachmentFinalizeRequest, ChatAttachmentPresignRequest,
    ChatAttachmentPresignResponse, ChatAttachmentSignGetResponse, DatasetDetailResponse,
    DatasetListResponse, DownloadUrlResponse, FinalizePart, FinalizeRequest, PresignRequest,
    PresignResponse, PresignedPart, ResignRequest, ResignResponse, UploadErrorResponse,
    UploadResponse,
};
use crate::clients::da::mapping::{build_hardened_client, TenantBaseUrl};
use crate::mcp::registry::address_is_dialable;
use crate::uploads::domain::DatasetIngest;

/// Wraps a value that contains a presigned URL or storage key so it cannot be
/// logged.
///
/// `Debug` is the whole point: `tracing::error!(?response)` on a failed
/// finalize is an ordinary thing to write and would otherwise put a live
/// credential in a log file.
pub(crate) struct Presigned<T>(T);

impl<T> Presigned<T> {
    #[must_use]
    pub(crate) fn new(value: T) -> Self {
        Self(value)
    }

    /// Unwraps deliberately. Named so a reviewer sees where a credential
    /// escapes its wrapper.
    #[must_use]
    pub(crate) fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> std::fmt::Debug for Presigned<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Presigned(<redacted>)")
    }
}

/// How DA wants the bytes delivered.
#[derive(Debug)]
pub(crate) enum TransferPlan {
    /// One presigned PUT for the whole file.
    Single { url: Presigned<Url> },
    /// Presigned PUTs per part, each `part_size` bytes except the last.
    Multipart {
        upload_id: String,
        part_size: u64,
        parts: Presigned<Vec<PresignedPart>>,
    },
    /// The backend has no object store (on-prem `file://`), so bytes go
    /// in-band through DA itself.
    ///
    /// Not an error and not a fallback this client chooses — DA decides, and
    /// the client must honour whichever it is told, or an on-prem install
    /// simply cannot upload.
    Legacy,
}

/// What DA assigned when a persistent upload was presigned.
#[derive(Debug)]
pub(crate) struct PreparedDataset {
    /// `None` in legacy mode, where no dataset row exists until the in-band
    /// upload completes.
    pub dataset_id: Option<String>,
    pub plan: TransferPlan,
    /// Seconds until the presigned URLs stop working.
    ///
    /// Unread today: the transfer re-signs reactively, on the 403 object
    /// storage actually returns, rather than pre-emptively on a clock this
    /// client cannot trust to match the server's. Kept because it is the
    /// signal a future pre-emptive re-sign would need.
    #[allow(dead_code)]
    pub expires_in: Option<i64>,
}

/// What DA assigned when a chat attachment was presigned.
#[derive(Debug)]
pub(crate) struct PreparedAttachment {
    pub storage_key: Presigned<String>,
    /// `None` in legacy mode.
    pub url: Option<Presigned<Url>>,
    #[allow(dead_code)]
    pub expires_in: Option<i64>,
}

/// DA's stable upload error codes.
///
/// Mirrors `UploadErrorCode` in DA's own models, which xfalcon-web also
/// switches on. The point of reading the code is that `detail` must never be
/// forwarded: it is written for a person but can carry a presigned URL, a
/// storage key, or a bucket name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UploadRefusalCode {
    UnsupportedFormat,
    MimeMismatch,
    FileTooLarge,
    DatasetTooLarge,
    DecompressedTooLarge,
    InvalidEncoding,
    InvalidStructure,
    EncryptedFile,
    EmptyFile,
    CrossTenantUrl,
    UploadsDisabled,
    SourceDisabled,
    UserIdentityRequired,
    UploadBusy,
    StorageError,
    ConversionError,
    PresignFailed,
    PartUploadFailed,
    FinalizeFailed,
    UploadExpired,
    MultipartAborted,
    InternalError,
    /// A code this build does not know. Kept as an arm rather than dropped, so
    /// a newer server still produces "the workspace refused this" instead of
    /// a message that guesses.
    Unrecognized,
}

impl UploadRefusalCode {
    fn parse(code: &str) -> Self {
        match code {
            "unsupported_format" => Self::UnsupportedFormat,
            "mime_mismatch" => Self::MimeMismatch,
            "file_too_large" => Self::FileTooLarge,
            "dataset_too_large" => Self::DatasetTooLarge,
            "decompressed_too_large" => Self::DecompressedTooLarge,
            "invalid_encoding" => Self::InvalidEncoding,
            "invalid_structure" => Self::InvalidStructure,
            "encrypted_file" => Self::EncryptedFile,
            "empty_file" => Self::EmptyFile,
            "cross_tenant_url" => Self::CrossTenantUrl,
            "uploads_disabled" => Self::UploadsDisabled,
            "source_disabled" => Self::SourceDisabled,
            "user_identity_required" => Self::UserIdentityRequired,
            "upload_busy" => Self::UploadBusy,
            "storage_error" => Self::StorageError,
            "conversion_error" => Self::ConversionError,
            "presign_failed" => Self::PresignFailed,
            "part_upload_failed" => Self::PartUploadFailed,
            "finalize_failed" => Self::FinalizeFailed,
            "upload_expired" => Self::UploadExpired,
            "multipart_aborted" => Self::MultipartAborted,
            "internal_error" => Self::InternalError,
            _ => Self::Unrecognized,
        }
    }
}

/// What to tell a person about a refused dataset transfer.
///
/// Written here rather than forwarded from DA: DA's `detail` is prose for a
/// person, but prose that can name a presigned URL, a storage key, or a bucket.
/// The stable code is the part this app acts on, and an unrecognised one still
/// produces a sentence that is true.
pub(crate) fn refusal_message(refusal: &UploadRefusal) -> &'static str {
    use UploadRefusalCode as Code;
    match refusal.code {
        Some(Code::UnsupportedFormat | Code::MimeMismatch) => {
            "This workspace does not accept that kind of file."
        }
        Some(Code::FileTooLarge | Code::DecompressedTooLarge) => {
            "That file is larger than this workspace accepts."
        }
        Some(Code::DatasetTooLarge) => "This workspace has no room for another dataset.",
        Some(Code::InvalidEncoding) => {
            "That file's text encoding could not be read. Re-save it as UTF-8 and try again."
        }
        Some(Code::InvalidStructure) => {
            "That file's rows and columns could not be read. Check the export and try again."
        }
        Some(Code::EncryptedFile) => {
            "That file is password-protected. Remove the password and try again."
        }
        Some(Code::EmptyFile) => "That file has no rows.",
        Some(Code::UploadsDisabled | Code::SourceDisabled) => {
            "Uploads are turned off for this workspace. An administrator can turn them back on."
        }
        Some(Code::UserIdentityRequired) => "Sign in again to upload files.",
        Some(Code::UploadBusy) => "Another upload is still finishing. Try again in a moment.",
        Some(Code::UploadExpired | Code::MultipartAborted) => {
            "The upload timed out before it finished. You can try again."
        }
        Some(
            Code::StorageError
            | Code::ConversionError
            | Code::PresignFailed
            | Code::PartUploadFailed
            | Code::FinalizeFailed
            | Code::InternalError
            | Code::CrossTenantUrl
            | Code::Unrecognized,
        )
        | None => match refusal.error {
            DaChatApiError::Unauthorized => "Sign in again to upload files.",
            _ => "The workspace could not accept this file. You can try again.",
        },
    }
}

/// A refused dataset operation: the transport-shaped failure, plus DA's own
/// code when the body carried one.
///
/// Chat attachments keep [`DaChatApiError`] alone. Their surface has one
/// message for every failure, so a code there would be surface with no
/// consumer; the datasets page has a control to disable and a ceiling to
/// name, so it needs to know which refusal it got.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{error}")]
pub(crate) struct UploadRefusal {
    pub error: DaChatApiError,
    pub code: Option<UploadRefusalCode>,
}

impl UploadRefusal {
    fn plain(error: DaChatApiError) -> Self {
        Self { error, code: None }
    }
}

impl From<DaChatApiError> for UploadRefusal {
    fn from(error: DaChatApiError) -> Self {
        Self::plain(error)
    }
}

/// One part's confirmed upload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletedPart {
    pub part_number: i64,
    pub etag: String,
}

/// Persistent dataset uploads.
#[async_trait]
pub(crate) trait PersistentUploadRepository: Send + Sync {
    async fn begin(
        &self,
        filename: &str,
        content_type: &str,
        size_bytes: u64,
        display_name: &str,
        ingest: DatasetIngest,
    ) -> Result<PreparedDataset, UploadRefusal>;

    async fn finalize(
        &self,
        dataset_id: &str,
        upload_id: Option<&str>,
        parts: &[CompletedPart],
        display_name: &str,
        filename: &str,
        ingest: DatasetIngest,
    ) -> Result<UploadResponse, UploadRefusal>;

    async fn abort(&self, upload_id: &str, dataset_id: &str) -> Result<(), UploadRefusal>;

    async fn resign(
        &self,
        upload_id: &str,
        dataset_id: &str,
        part_numbers: &[i64],
    ) -> Result<(Presigned<Vec<PresignedPart>>, i64), UploadRefusal>;

    /// The account's datasets, one page at a time.
    async fn list(&self, limit: i64, offset: i64) -> Result<DatasetListResponse, UploadRefusal>;

    /// One dataset's detail, including its files and their inferred schema.
    async fn get(&self, dataset_id: &str) -> Result<DatasetDetailResponse, UploadRefusal>;

    /// Removes a dataset and everything DA holds for it.
    async fn delete(&self, dataset_id: &str) -> Result<(), UploadRefusal>;

    /// A short-lived URL for one of a dataset's files.
    ///
    /// Wrapped in [`Presigned`] because it is a bearer credential: it is
    /// fetched inside this process and never returned to the webview.
    async fn download_url(
        &self,
        dataset_id: &str,
        file_id: &str,
    ) -> Result<(Presigned<String>, String), UploadRefusal>;
}

/// Chat attachment uploads.
///
/// Method names carry the `_attachment` suffix because
/// [`DaUploadRepository`] implements both ports, and a bare `finalize` on it
/// would be ambiguous at every call site — the kind of ambiguity that gets
/// resolved by whichever trait happens to be in scope.
#[async_trait]
pub(crate) trait ChatAttachmentRepository: Send + Sync {
    async fn presign_attachment(
        &self,
        conversation_id: &str,
        filename: &str,
        content_type: &str,
        size_bytes: u64,
        kind: &str,
    ) -> Result<PreparedAttachment, DaChatApiError>;

    async fn finalize_attachment(&self, storage_key: &str) -> Result<(), DaChatApiError>;

    #[allow(dead_code)]
    async fn sign_get(&self, storage_key: &str) -> Result<Presigned<Url>, DaChatApiError>;

    async fn record_ready_attachment(
        &self,
        conversation_id: uuid::Uuid,
        kind: ChatAttachmentCreateKind,
        file_name: &str,
        mime_type: &str,
        size_bytes: i64,
        storage_key: &str,
    ) -> Result<uuid::Uuid, DaChatApiError>;
}

/// Both ports over one authenticated DA session.
pub(crate) struct DaUploadRepository {
    base: TenantBaseUrl,
    client: reqwest::Client,
    bearer: String,
}

impl DaUploadRepository {
    pub(crate) fn new(da_origin: &str, bearer: impl Into<String>) -> Result<Self, DaChatApiError> {
        Self::with_base(
            TenantBaseUrl::parse(da_origin).map_err(|_| DaChatApiError::RequestFailed)?,
            bearer,
        )
    }

    /// Same repository against a loopback `http://` origin.
    ///
    /// The production constructor requires `https` and rejects anything else —
    /// a tenant origin is where a DA bearer gets sent, so downgrading it is
    /// not a configuration mistake to tolerate. Tests need a local mock
    /// server, so they get a separate, `cfg(test)`-only door rather than a
    /// relaxed check on the real one.
    #[cfg(test)]
    pub(crate) fn for_test(
        da_origin: &str,
        bearer: impl Into<String>,
    ) -> Result<Self, DaChatApiError> {
        Self::with_base(
            TenantBaseUrl::parse_loopback(da_origin).map_err(|_| DaChatApiError::RequestFailed)?,
            bearer,
        )
    }

    fn with_base(base: TenantBaseUrl, bearer: impl Into<String>) -> Result<Self, DaChatApiError> {
        let client = build_hardened_client().map_err(|_| DaChatApiError::RequestFailed)?;
        Ok(Self {
            base,
            client,
            bearer: bearer.into(),
        })
    }

    fn url(&self, path: &str) -> Url {
        self.base.url_with_path(path)
    }

    /// Sends a JSON request and decodes the response, mapping DA's status
    /// codes onto this crate's error vocabulary.
    async fn post_json<B: serde::Serialize + Sync, R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R, DaChatApiError> {
        let response = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.bearer)
            .json(body)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        decode(response).await
    }

    /// Sends a JSON request and decodes the response, keeping DA's stable
    /// error code when it refuses.
    async fn post_json_upload<B: serde::Serialize + Sync, R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R, UploadRefusal> {
        let response = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.bearer)
            .json(body)
            .send()
            .await
            .map_err(|_| UploadRefusal::plain(DaChatApiError::RequestFailed))?;
        decode_upload(response).await
    }

    async fn get_json_upload<R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<R, UploadRefusal> {
        let response = self
            .client
            .get(self.query_url(path, query))
            .bearer_auth(&self.bearer)
            .send()
            .await
            .map_err(|_| UploadRefusal::plain(DaChatApiError::RequestFailed))?;
        decode_upload(response).await
    }

    /// A request whose success carries no body, which is what DA's dataset
    /// delete answers with.
    async fn delete_upload(&self, path: &str) -> Result<(), UploadRefusal> {
        let response = self
            .client
            .delete(self.url(path))
            .bearer_auth(&self.bearer)
            .send()
            .await
            .map_err(|_| UploadRefusal::plain(DaChatApiError::RequestFailed))?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(refusal(response).await)
    }

    fn query_url(&self, path: &str, query: &[(&str, String)]) -> Url {
        let mut url = self.url(path);
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        url
    }

    async fn get_json<R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<R, DaChatApiError> {
        let mut url = self.url(path);
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.bearer)
            .send()
            .await
            .map_err(|_| DaChatApiError::RequestFailed)?;
        decode(response).await
    }
}

/// Maps a DA response onto this crate's errors.
///
/// The body is never included in an error: a validation failure can echo the
/// request, and the request carries a filename the user may not want in a log.
async fn decode<R: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<R, DaChatApiError> {
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => DaChatApiError::Unauthorized,
            // A conflict on finalize means the object was already completed —
            // the caller's retry is redundant, not wrong.
            409 => DaChatApiError::Conflict,
            _ => DaChatApiError::RequestFailed,
        });
    }
    response
        .json()
        .await
        .map_err(|_| DaChatApiError::InvalidResponse)
}

/// Maps a DA response onto [`UploadRefusal`], reading the stable `code` from a
/// refusal body and discarding everything else in it.
async fn decode_upload<R: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<R, UploadRefusal> {
    if !response.status().is_success() {
        return Err(refusal(response).await);
    }
    response
        .json()
        .await
        .map_err(|_| UploadRefusal::plain(DaChatApiError::InvalidResponse))
}

/// Builds the refusal for a non-success response.
///
/// `detail` and `issues` are deliberately dropped. `detail` is DA's prose, and
/// DA writes prose that can name a URL, a key, or a bucket; the code is the
/// part this app is allowed to act on.
async fn refusal(response: reqwest::Response) -> UploadRefusal {
    let status = response.status().as_u16();
    let error = match status {
        401 | 403 => DaChatApiError::Unauthorized,
        404 => DaChatApiError::NotFound,
        409 => DaChatApiError::Conflict,
        _ => DaChatApiError::RequestFailed,
    };
    let code = response
        .text()
        .await
        .ok()
        .and_then(|body| serde_json::from_str::<UploadErrorResponse>(&body).ok())
        .map(|body| UploadRefusalCode::parse(&body.code));
    UploadRefusal { error, code }
}

/// Whether plain `http` presigned URLs are refused unless their host is private.
///
/// Off on purpose. On-prem and private deployments run object storage such as
/// MinIO without TLS, on their own network, often at an IP address or an
/// internal DNS name that does not look private by spelling, so the private-host
/// test would refuse working tenants. A presigned URL authenticates by its
/// signature, so the scheme rule is hardening, not the access control. Setting
/// this to `true` restores "https, or plain http only toward private hosts".
pub(crate) const ENFORCE_HTTPS_PRESIGNED_URLS: bool = false;

/// Refuses a presigned URL that is not a bare, absolute object-storage target.
///
/// Uses [`ENFORCE_HTTPS_PRESIGNED_URLS`] for the plain `http` rule. See
/// [`validate_presigned_url_with`].
pub(crate) fn validate_presigned_url(url: &Url, allow_http: bool) -> Result<(), DaChatApiError> {
    validate_presigned_url_with(url, allow_http, ENFORCE_HTTPS_PRESIGNED_URLS)
}

/// The presigned URL rule, with the plain `http` enforcement passed in.
///
/// The URL is a bearer credential that this client is about to send a body to,
/// so it must be `https` or `http`, and must not carry userinfo. When `enforce`
/// is off, plain `http` is accepted to any host. When `enforce` is on, plain
/// `http` is accepted only in a debug build (`allow_http`, dev MinIO) or toward
/// a host that is not on the public internet, since nothing public may receive
/// a body in cleartext. A non-web scheme, a URL without a host and userinfo are
/// refused in every mode.
///
/// The error never includes the URL.
pub(crate) fn validate_presigned_url_with(
    url: &Url,
    allow_http: bool,
    enforce: bool,
) -> Result<(), DaChatApiError> {
    let scheme_ok = match url.scheme() {
        "https" => true,
        "http" => allow_http || !enforce || host_is_private_network(url),
        _ => false,
    };
    if !scheme_ok || url.host().is_none() || !url.username().is_empty() || url.password().is_some()
    {
        return Err(DaChatApiError::InvalidResponse);
    }
    Ok(())
}

/// The rule used only while enforcing plain `http` (see
/// [`ENFORCE_HTTPS_PRESIGNED_URLS`]): whether the URL names a host that is not
/// on the public internet, a loopback, private or link-local address, a
/// single-label name (`minio`), or a name under a suffix that is private by
/// convention. A name is judged by its spelling only; it is not resolved here.
fn host_is_private_network(url: &Url) -> bool {
    const PRIVATE_SUFFIXES: [&str; 7] = [
        ".local",
        ".internal",
        ".lan",
        ".corp",
        ".intranet",
        ".localdomain",
        ".home.arpa",
    ];
    let Some(host) = url.host_str() else {
        return false;
    };
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(address) = literal.parse::<IpAddr>() {
        return !address_is_dialable(address, false);
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    !name.contains('.') || PRIVATE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// Reads DA's `mode` plus the fields it populates for that mode.
///
/// DA's presign response is a union expressed as nullable fields, so the mode
/// string alone is not enough — a `multipart` response missing its parts is
/// malformed, and treating it as an empty part list would upload nothing and
/// report success.
fn transfer_plan(response: &PresignResponse) -> Result<TransferPlan, DaChatApiError> {
    match response.mode.as_str() {
        "single" => {
            let url = response
                .url
                .as_deref()
                .and_then(|raw| Url::parse(raw).ok())
                .ok_or(DaChatApiError::InvalidResponse)?;
            validate_presigned_url(&url, cfg!(debug_assertions))?;
            Ok(TransferPlan::Single {
                url: Presigned::new(url),
            })
        }
        "multipart" => {
            let upload_id = response
                .upload_id
                .clone()
                .ok_or(DaChatApiError::InvalidResponse)?;
            let part_size = response
                .part_size
                .filter(|size| *size > 0)
                .ok_or(DaChatApiError::InvalidResponse)?;
            let parts = response
                .parts
                .clone()
                .filter(|parts| !parts.is_empty())
                .ok_or(DaChatApiError::InvalidResponse)?;
            // One bad part makes the whole plan invalid: a partial upload that
            // skips a part would complete with a file that is not the one picked.
            for part in &parts {
                let url = Url::parse(&part.url).map_err(|_| DaChatApiError::InvalidResponse)?;
                validate_presigned_url(&url, cfg!(debug_assertions))?;
            }
            Ok(TransferPlan::Multipart {
                upload_id,
                #[allow(clippy::cast_sign_loss)]
                part_size: part_size as u64,
                parts: Presigned::new(parts),
            })
        }
        "legacy" => Ok(TransferPlan::Legacy),
        _ => Err(DaChatApiError::InvalidResponse),
    }
}

#[async_trait]
impl PersistentUploadRepository for DaUploadRepository {
    async fn begin(
        &self,
        filename: &str,
        content_type: &str,
        size_bytes: u64,
        display_name: &str,
        ingest: DatasetIngest,
    ) -> Result<PreparedDataset, UploadRefusal> {
        let response: PresignResponse = self
            .post_json_upload(
                "/api/v1/user/uploads/presign",
                &PresignRequest {
                    filename: filename.to_owned(),
                    content_type: content_type.to_owned(),
                    size_bytes: i64::try_from(size_bytes)
                        .map_err(|_| UploadRefusal::plain(DaChatApiError::RequestFailed))?,
                    display_name: display_name.to_owned(),
                    selected_sheet_index: ingest.worksheet_index.map(i64::from),
                    header_row_index: ingest.header_row_index.map(i64::from),
                },
            )
            .await?;

        let plan = transfer_plan(&response)?;
        Ok(PreparedDataset {
            dataset_id: response.dataset_id,
            plan,
            expires_in: response.expires_in,
        })
    }

    async fn finalize(
        &self,
        dataset_id: &str,
        upload_id: Option<&str>,
        parts: &[CompletedPart],
        display_name: &str,
        filename: &str,
        ingest: DatasetIngest,
    ) -> Result<UploadResponse, UploadRefusal> {
        self.post_json_upload(
            "/api/v1/user/uploads/finalize",
            &FinalizeRequest {
                dataset_id: dataset_id.to_owned(),
                upload_id: upload_id.map(str::to_owned),
                // Null, not an empty list, for a single PUT: DA distinguishes
                // "no multipart" from "multipart with zero parts", and the
                // latter is a request to complete an upload that sent nothing.
                parts: if parts.is_empty() {
                    None
                } else {
                    Some(
                        parts
                            .iter()
                            .map(|part| FinalizePart {
                                part_number: part.part_number,
                                etag: part.etag.clone(),
                            })
                            .collect(),
                    )
                },
                display_name: display_name.to_owned(),
                filename: Some(filename.to_owned()),
                // Carried here as well as on presign: DA reads the pair from
                // whichever call created the row it is completing, and a
                // worksheet chosen at presign that vanished at finalize would
                // ingest a different sheet than the person confirmed.
                selected_sheet_index: ingest.worksheet_index.map(i64::from),
                header_row_index: ingest.header_row_index.map(i64::from),
            },
        )
        .await
    }

    async fn abort(&self, upload_id: &str, dataset_id: &str) -> Result<(), UploadRefusal> {
        let _: serde_json::Value = self
            .post_json_upload(
                &format!("/api/v1/user/uploads/{upload_id}/abort"),
                &AbortRequest {
                    dataset_id: dataset_id.to_owned(),
                    filename: None,
                },
            )
            .await?;
        Ok(())
    }

    async fn resign(
        &self,
        upload_id: &str,
        dataset_id: &str,
        part_numbers: &[i64],
    ) -> Result<(Presigned<Vec<PresignedPart>>, i64), UploadRefusal> {
        let response: ResignResponse = self
            .post_json_upload(
                &format!("/api/v1/user/uploads/{upload_id}/resign"),
                &ResignRequest {
                    dataset_id: dataset_id.to_owned(),
                    part_numbers: part_numbers.to_vec(),
                },
            )
            .await?;
        Ok((Presigned::new(response.parts), response.expires_in))
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<DatasetListResponse, UploadRefusal> {
        self.get_json_upload(
            "/api/v1/user/uploads",
            &[("limit", limit.to_string()), ("offset", offset.to_string())],
        )
        .await
    }

    async fn get(&self, dataset_id: &str) -> Result<DatasetDetailResponse, UploadRefusal> {
        self.get_json_upload(&format!("/api/v1/user/uploads/{dataset_id}"), &[])
            .await
    }

    async fn delete(&self, dataset_id: &str) -> Result<(), UploadRefusal> {
        self.delete_upload(&format!("/api/v1/user/uploads/{dataset_id}"))
            .await
    }

    async fn download_url(
        &self,
        dataset_id: &str,
        file_id: &str,
    ) -> Result<(Presigned<String>, String), UploadRefusal> {
        // `as=url` asks DA for the presigned URL as JSON. The alternative is a
        // 302 to the same URL, which would put it in a redirect chain this
        // client would have to be trusted not to follow anywhere else.
        let response: DownloadUrlResponse = self
            .get_json_upload(
                &format!("/api/v1/user/uploads/{dataset_id}/files/{file_id}/download"),
                &[("as", "url".to_owned())],
            )
            .await?;
        let url = Url::parse(&response.url).map_err(|_| DaChatApiError::InvalidResponse)?;
        validate_presigned_url(&url, cfg!(debug_assertions))?;
        Ok((Presigned::new(response.url), response.filename))
    }
}

#[async_trait]
impl ChatAttachmentRepository for DaUploadRepository {
    async fn presign_attachment(
        &self,
        conversation_id: &str,
        filename: &str,
        content_type: &str,
        size_bytes: u64,
        kind: &str,
    ) -> Result<PreparedAttachment, DaChatApiError> {
        let response: ChatAttachmentPresignResponse = self
            .post_json(
                "/api/v1/user/chat-attachments/presign",
                &ChatAttachmentPresignRequest {
                    filename: filename.to_owned(),
                    content_type: content_type.to_owned(),
                    size_bytes: i64::try_from(size_bytes)
                        .map_err(|_| DaChatApiError::RequestFailed)?,
                    conversation_id: conversation_id.to_owned(),
                    kind: kind.to_owned(),
                },
            )
            .await?;

        // `mode` defaults to `s3` when absent, and only `legacy` means "no
        // presigned URL". Any other mode with no URL is malformed rather than
        // an in-band fallback — guessing would silently upload nothing.
        let legacy = response.mode.as_deref() == Some("legacy");
        let url = match (legacy, response.url.as_deref()) {
            (true, _) => None,
            (false, Some(raw)) => {
                let url = Url::parse(raw).map_err(|_| DaChatApiError::InvalidResponse)?;
                validate_presigned_url(&url, cfg!(debug_assertions))?;
                Some(Presigned::new(url))
            }
            (false, None) => return Err(DaChatApiError::InvalidResponse),
        };

        Ok(PreparedAttachment {
            storage_key: Presigned::new(response.storage_key),
            url,
            expires_in: response.expires_in,
        })
    }

    async fn finalize_attachment(&self, storage_key: &str) -> Result<(), DaChatApiError> {
        let _: serde_json::Value = self
            .post_json(
                "/api/v1/user/chat-attachments/finalize",
                &ChatAttachmentFinalizeRequest {
                    storage_key: storage_key.to_owned(),
                },
            )
            .await?;
        Ok(())
    }

    async fn sign_get(&self, storage_key: &str) -> Result<Presigned<Url>, DaChatApiError> {
        let response: ChatAttachmentSignGetResponse = self
            .get_json(
                "/api/v1/user/chat-attachments/sign-get",
                &[("storage_key", storage_key.to_owned())],
            )
            .await?;
        let url = Url::parse(&response.url).map_err(|_| DaChatApiError::InvalidResponse)?;
        validate_presigned_url(&url, cfg!(debug_assertions))?;
        Ok(Presigned::new(url))
    }

    async fn record_ready_attachment(
        &self,
        conversation_id: uuid::Uuid,
        kind: ChatAttachmentCreateKind,
        file_name: &str,
        mime_type: &str,
        size_bytes: i64,
        storage_key: &str,
    ) -> Result<uuid::Uuid, DaChatApiError> {
        let created: ChatAttachment = self
            .post_json(
                "/api/v1/chat/chat-attachments",
                &ChatAttachmentCreate {
                    conversation_id,
                    kind,
                    file_name: file_name.to_owned(),
                    mime_type: mime_type.to_owned(),
                    size_bytes,
                    storage_key: storage_key.to_owned(),
                    message_id: None,
                    status: Some(ChatAttachmentCreateStatus::Ready),
                    metadata: None,
                },
            )
            .await?;
        Ok(created.id)
    }
}
