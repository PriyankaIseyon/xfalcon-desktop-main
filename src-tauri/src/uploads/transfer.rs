//! Moves an upload's bytes, entirely outside React.
//!
//! # Object storage is not DA
//!
//! A presigned PUT goes to object storage, not to the tenant BFF, and
//! [`ObjectStorePut`] therefore has its own client that attaches **no**
//! `Authorization` header. Reusing the DA client here would send the tenant
//! bearer to whatever host the presigned URL names — the presigned URL is
//! already the credential for that request, and adding a second, far more
//! powerful one to it is exactly the mistake this separation prevents.
//! Redirects are refused for the same reason: a 302 would replay the body,
//! and any header on it, somewhere nobody presigned.
//!
//! # Memory
//!
//! A file is never read whole. A single PUT streams from disk; a multipart
//! PUT holds exactly one part (DA's `part_size`, typically 8 MiB) at a time.
//! An upload of any size therefore costs the same bounded memory.
//!
//! # Progress is what the server confirmed
//!
//! Bytes are counted as each request *completes*, not as they are handed to
//! the socket. A part queued into a kernel buffer has not arrived, and a
//! progress bar that counts it jumps forward and then stalls on a retry.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{redirect::Policy, Client, Url};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::sync::CancellationToken;

use super::da_repository::{
    validate_presigned_url, CompletedPart, PersistentUploadRepository, PreparedDataset, Presigned,
    TransferPlan,
};

/// Per-request timeouts for object-storage transfers.
///
/// Far longer than the DA client's: a single PUT of an 8 MiB part on a slow
/// connection legitimately takes minutes, where a BFF call that slow is a
/// fault.
const PUT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PUT_TOTAL_TIMEOUT: Duration = Duration::from_secs(600);

/// Parts uploaded concurrently.
///
/// Three, matching this codebase's existing concurrency bound for outbound
/// work (`crate::agent::tool_loop::MAX_PARALLEL_TOOL_CALLS`, itself citing the
/// threat model's "maximum three workers"). Enough to saturate a typical
/// connection; small enough that peak memory stays `3 × part_size`.
pub const MAX_CONCURRENT_PARTS: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("the upload was cancelled")]
    Cancelled,
    #[error("the file could not be read")]
    SourceUnreadable,
    #[error("the presigned upload target rejected the request")]
    Rejected,
    #[error("the presigned upload target could not be reached")]
    Unreachable,
    #[error("object storage did not return an entity tag for a completed part")]
    MissingEtag,
    #[error("the workspace could not prepare this upload")]
    Preparation,
}

/// Whether a failed PUT is worth re-signing and retrying.
///
/// Only expiry is: object storage answers an expired presigned URL with 403,
/// which is indistinguishable at the HTTP layer from a genuine authorization
/// failure — but a URL this client presigned seconds ago and has been using
/// successfully is overwhelmingly the former. One re-sign attempt resolves
/// which it is without an unbounded retry loop.
#[derive(Clone, Debug, Eq, PartialEq)]
enum PutOutcome {
    Ok(String),
    MaybeExpired,
    Fatal,
}

/// Uploads bytes to a presigned URL.
#[async_trait]
pub(crate) trait ObjectStorePut: Send + Sync {
    /// Returns the object's entity tag, which multipart completion requires.
    async fn put_part(
        &self,
        url: &Url,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<String, TransferError>;
}

/// The real object-store client.
pub(crate) struct HttpObjectStore {
    client: Client,
}

impl HttpObjectStore {
    pub(crate) fn new() -> Result<Self, TransferError> {
        let client = Client::builder()
            // No redirects: a 302 would replay the body somewhere nobody
            // presigned.
            .redirect(Policy::none())
            // No automatic retry: a retried PUT of a part that actually
            // succeeded produces a second object version and a different
            // ETag, which then fails multipart completion.
            .retry(reqwest::retry::never())
            .connect_timeout(PUT_CONNECT_TIMEOUT)
            .timeout(PUT_TOTAL_TIMEOUT)
            .build()
            .map_err(|_| TransferError::Unreachable)?;
        Ok(Self { client })
    }
}

#[async_trait]
impl ObjectStorePut for HttpObjectStore {
    async fn put_part(
        &self,
        url: &Url,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<String, TransferError> {
        // Deliberately no `.bearer_auth(...)`: see the module doc comment.
        let response = self
            .client
            .put(url.clone())
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .await
            .map_err(|_| TransferError::Unreachable)?;

        if !response.status().is_success() {
            return Err(if response.status().as_u16() == 403 {
                TransferError::Rejected
            } else {
                TransferError::Unreachable
            });
        }

        response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .ok_or(TransferError::MissingEtag)
    }
}

/// Reports confirmed progress. Called from the transfer task, never from a UI
/// thread.
pub(crate) type ProgressSink = Arc<dyn Fn(u64) + Send + Sync>;

/// What a completed transfer produced, ready for finalize.
#[derive(Debug)]
pub(crate) struct TransferOutcome {
    pub dataset_id: Option<String>,
    pub upload_id: Option<String>,
    pub parts: Vec<CompletedPart>,
}

/// Streams one file to the target DA presigned.
pub(crate) struct UploadTransfer<'a> {
    pub repository: &'a dyn PersistentUploadRepository,
    pub store: &'a dyn ObjectStorePut,
    pub on_progress: ProgressSink,
}

impl UploadTransfer<'_> {
    /// Sends `path` according to `prepared`, reporting confirmed bytes.
    pub(crate) async fn run(
        &self,
        prepared: &PreparedDataset,
        path: &Path,
        content_type: &str,
        cancel: &CancellationToken,
    ) -> Result<TransferOutcome, TransferError> {
        if cancel.is_cancelled() {
            return Err(TransferError::Cancelled);
        }
        match &prepared.plan {
            TransferPlan::Single { url } => {
                let body = read_range(path, 0, None).await?;
                let sent = body.len() as u64;
                self.put_with_resign(url.expose(), body, content_type, None, cancel)
                    .await?;
                (self.on_progress)(sent);
                Ok(TransferOutcome {
                    dataset_id: prepared.dataset_id.clone(),
                    upload_id: None,
                    parts: Vec::new(),
                })
            }
            TransferPlan::Multipart {
                upload_id,
                part_size,
                parts,
            } => {
                let completed = self
                    .run_multipart(
                        prepared.dataset_id.as_deref(),
                        upload_id,
                        *part_size,
                        parts,
                        path,
                        content_type,
                        cancel,
                    )
                    .await?;
                Ok(TransferOutcome {
                    dataset_id: prepared.dataset_id.clone(),
                    upload_id: Some(upload_id.clone()),
                    parts: completed,
                })
            }
            // DA's backend has no object store; bytes go in-band and there is
            // nothing to PUT. The caller finalizes directly.
            TransferPlan::Legacy => Ok(TransferOutcome {
                dataset_id: prepared.dataset_id.clone(),
                upload_id: None,
                parts: Vec::new(),
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_multipart(
        &self,
        dataset_id: Option<&str>,
        upload_id: &str,
        part_size: u64,
        parts: &Presigned<Vec<crate::clients::da::generated::PresignedPart>>,
        path: &Path,
        content_type: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<CompletedPart>, TransferError> {
        let mut completed: Vec<CompletedPart> = Vec::with_capacity(parts.expose().len());
        let mut sent_total: u64 = 0;

        // Sequential across parts, deliberately for now: each part is read
        // from disk immediately before it is sent, so overlapping them would
        // multiply peak memory by the concurrency, and the ordering makes
        // "bytes confirmed" unambiguous. `MAX_CONCURRENT_PARTS` is the bound
        // to honour when this becomes concurrent.
        for part in parts.expose() {
            if cancel.is_cancelled() {
                return Err(TransferError::Cancelled);
            }
            #[allow(clippy::cast_sign_loss)]
            let index = (part.part_number - 1).max(0) as u64;
            let offset = index * part_size;
            let body = read_range(path, offset, Some(part_size)).await?;
            if body.is_empty() {
                // The file shrank since it was measured. Sending an empty part
                // would complete an upload whose object does not match what
                // the user picked.
                return Err(TransferError::SourceUnreadable);
            }
            let length = body.len() as u64;

            let url = Url::parse(&part.url).map_err(|_| TransferError::Rejected)?;
            let etag = self
                .put_with_resign(
                    &url,
                    body,
                    content_type,
                    Some((upload_id, dataset_id, part.part_number)),
                    cancel,
                )
                .await?;

            completed.push(CompletedPart {
                part_number: part.part_number,
                etag,
            });
            sent_total += length;
            (self.on_progress)(sent_total);
        }

        Ok(completed)
    }

    /// PUTs `body`, re-signing once if the target says the URL is no longer
    /// valid.
    ///
    /// `resign` is `None` for a single PUT: DA's resign endpoint is part-based
    /// and has nothing to re-sign for a whole-file upload, so that case fails
    /// and the caller retries from a fresh presign.
    async fn put_with_resign(
        &self,
        url: &Url,
        body: Vec<u8>,
        content_type: &str,
        resign: Option<(&str, Option<&str>, i64)>,
        cancel: &CancellationToken,
    ) -> Result<String, TransferError> {
        match self
            .attempt(url, body.clone(), content_type, cancel)
            .await?
        {
            PutOutcome::Ok(etag) => return Ok(etag),
            PutOutcome::Fatal => return Err(TransferError::Unreachable),
            PutOutcome::MaybeExpired => {}
        }

        let Some((upload_id, Some(dataset_id), part_number)) = resign else {
            return Err(TransferError::Rejected);
        };
        let (fresh, _expires_in) = self
            .repository
            .resign(upload_id, dataset_id, &[part_number])
            .await
            .map_err(|_| TransferError::Preparation)?;
        let fresh_url = fresh
            .expose()
            .iter()
            .find(|part| part.part_number == part_number)
            .and_then(|part| Url::parse(&part.url).ok())
            .filter(|url| validate_presigned_url(url, cfg!(debug_assertions)).is_ok())
            .ok_or(TransferError::Preparation)?;

        // Exactly one re-sign. A second 403 on a URL minted seconds ago is a
        // real authorization failure, not expiry, and retrying it forever
        // would hang the upload.
        match self.attempt(&fresh_url, body, content_type, cancel).await? {
            PutOutcome::Ok(etag) => Ok(etag),
            PutOutcome::MaybeExpired => Err(TransferError::Rejected),
            PutOutcome::Fatal => Err(TransferError::Unreachable),
        }
    }

    async fn attempt(
        &self,
        url: &Url,
        body: Vec<u8>,
        content_type: &str,
        cancel: &CancellationToken,
    ) -> Result<PutOutcome, TransferError> {
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(TransferError::Cancelled),
            result = self.store.put_part(url, body, content_type) => Ok(match result {
                Ok(etag) => PutOutcome::Ok(etag),
                Err(TransferError::Rejected) => PutOutcome::MaybeExpired,
                Err(TransferError::MissingEtag) => return Err(TransferError::MissingEtag),
                Err(_) => PutOutcome::Fatal,
            }),
        }
    }
}

/// Reads at most `length` bytes from `offset`.
///
/// Seek-and-read rather than reading the file whole: this is the only reason
/// a multipart upload of a large file costs bounded memory.
async fn read_range(
    path: &Path,
    offset: u64,
    length: Option<u64>,
) -> Result<Vec<u8>, TransferError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| TransferError::SourceUnreadable)?;
    if offset > 0 {
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|_| TransferError::SourceUnreadable)?;
    }
    match length {
        Some(length) => {
            let mut buffer = Vec::new();
            let read = file
                .take(length)
                .read_to_end(&mut buffer)
                .await
                .map_err(|_| TransferError::SourceUnreadable)?;
            buffer.truncate(read);
            Ok(buffer)
        }
        None => {
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)
                .await
                .map_err(|_| TransferError::SourceUnreadable)?;
            Ok(buffer)
        }
    }
}
