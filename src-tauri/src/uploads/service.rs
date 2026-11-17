//! The upload runtime: choosing a file, transferring it, and reporting the
//! queue.
//!
//! # The webview never names a path
//!
//! [`UploadService::choose`] opens the native picker *in Rust* and copies the
//! chosen file into an app-owned snapshot. There is deliberately no
//! `start(path)` command.
//!
//! A command that took a path would let whatever runs in the webview name any
//! file on the user's disk and have this process upload it to tenant storage.
//! The webview is the one process that renders untrusted model output, so the
//! difference between "the user picked this file" and "something asked for
//! this file" has to be enforced somewhere the model cannot reach. Here, the
//! only way an external path enters the system is a person clicking through
//! an OS dialog or copying a file reference in the native file manager.
//! Renderer clipboard uploads carry bounded bytes and a display name only;
//! they have no path authority.
//!
//! # Bounds
//!
//! [`MAX_CONCURRENT_UPLOADS`] caps transfers in flight and
//! [`MAX_UPLOAD_BYTES`] caps a single file. Both fail fast, before any byte
//! is read or any DA call is made.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use tauri_plugin_dialog::DialogExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use xfalcon_contracts::uploads::{UploadId, UploadPurpose, UploadQueueV1, UploadState};

use crate::auth::{ensure_fresh_session, AuthConfig, AuthSessionStore};
use crate::clients::da::generated::ChatAttachmentCreateKind;
use crate::security::temp_cleanup::{TempCleanup, TempLease};
use crate::uploads::da_repository::{
    refusal_message, ChatAttachmentRepository, DaUploadRepository, PersistentUploadRepository,
};
use crate::uploads::domain::{
    sanitize_display_name, DatasetIngest, LocalUploadSource, UploadRecord,
};
use crate::uploads::kind::{
    classify_chat_attachment, verify_chat_attachment_content, AttachmentKind,
    ChatAttachmentContentError, MAX_CHAT_ATTACHMENT_BYTES,
};
use crate::uploads::transfer::{HttpObjectStore, ObjectStorePut, TransferError, UploadTransfer};

/// Transfers running at once.
///
/// Matches `crate::chat::service::MAX_IN_FLIGHT_RUNS`'s reasoning: a bound
/// that keeps a user's own queue from saturating their connection, not a
/// product limit.
pub const MAX_CONCURRENT_UPLOADS: usize = 3;

/// Largest single file accepted.
///
/// Refused before the file is opened, so a mistakenly-picked disk image costs
/// a message rather than a long transfer the user then has to cancel.
pub const MAX_UPLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// A file the person picked for a chat, not yet queued.
pub struct PickedAttachment {
    path: std::path::PathBuf,
    account_generation: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum UploadServiceError {
    #[error("sign in to upload files")]
    AuthRequired,
    #[error("uploads are not configured on this device")]
    NotConfigured,
    #[error("no file was chosen")]
    NoFileChosen,
    #[error("that file could not be read")]
    Unreadable,
    #[error("that file is empty")]
    Empty,
    #[error("that file's contents do not match its filename")]
    ContentMismatch,
    #[error("{0}")]
    OfficeExtraction(String),
    #[error("that file is larger than this workspace accepts")]
    TooLarge,
    #[error("remove or retry an existing attachment before adding another")]
    TooManyUploads,
    #[error("no upload with that id is in the queue")]
    NotFound,
    #[error("that file type cannot be attached to a chat message")]
    UnsupportedType,
    #[error("attach a file to a conversation")]
    ConversationRequired,
    #[error("that upload cannot be retried")]
    NotRetryable,
}

/// Reports the queue after every change.
pub type QueueSink = Arc<dyn Fn(&UploadQueueV1) + Send + Sync>;

struct AttachmentSnapshot {
    path: PathBuf,
    lease: TempLease,
    size: u64,
    detected_type: String,
    display_name: String,
}

pub struct UploadService {
    sessions: Arc<AuthSessionStore>,
    /// Insertion-ordered so the queue renders newest-first deterministically.
    records: Arc<AsyncMutex<indexmap::IndexMap<String, UploadRecord>>>,
    cancels: Arc<AsyncMutex<HashMap<String, CancellationToken>>>,
    /// Local paths, held apart from the records so nothing that serializes a
    /// record can reach one even by mistake.
    sources: Arc<AsyncMutex<HashMap<String, PathBuf>>>,
    temporary_leases: Arc<AsyncMutex<HashMap<String, TempLease>>>,
    store: Arc<dyn ObjectStorePut>,
    on_queue: QueueSink,
    revision: Arc<AtomicU32>,
    /// Invalidates every transfer that belongs to the account being left.
    ///
    /// Cancellation stops transfer I/O. The generation also fences work that
    /// is between awaits and has not registered its cancellation token yet.
    account_generation: Arc<AtomicU64>,
    /// Serializes queue admission with an account boundary.
    account_admission: Arc<AsyncMutex<()>>,
    /// App-owned immutable copies used by picker and clipboard attachments.
    snapshot_root: Arc<PathBuf>,
    temp_cleanup: Arc<TempCleanup>,
}

impl Clone for UploadService {
    fn clone(&self) -> Self {
        Self {
            sessions: Arc::clone(&self.sessions),
            records: Arc::clone(&self.records),
            cancels: Arc::clone(&self.cancels),
            sources: Arc::clone(&self.sources),
            temporary_leases: Arc::clone(&self.temporary_leases),
            store: Arc::clone(&self.store),
            on_queue: Arc::clone(&self.on_queue),
            revision: Arc::clone(&self.revision),
            account_generation: Arc::clone(&self.account_generation),
            account_admission: Arc::clone(&self.account_admission),
            snapshot_root: Arc::clone(&self.snapshot_root),
            temp_cleanup: Arc::clone(&self.temp_cleanup),
        }
    }
}

impl UploadService {
    pub fn new(
        sessions: Arc<AuthSessionStore>,
        on_queue: QueueSink,
    ) -> Result<Self, TransferError> {
        let snapshot_root = std::env::temp_dir().join("xfalcon-upload-snapshots");
        let temp_cleanup = Arc::new(TempCleanup::new([snapshot_root.clone()]));
        Ok(Self {
            sessions,
            records: Arc::new(AsyncMutex::new(indexmap::IndexMap::new())),
            cancels: Arc::new(AsyncMutex::new(HashMap::new())),
            sources: Arc::new(AsyncMutex::new(HashMap::new())),
            temporary_leases: Arc::new(AsyncMutex::new(HashMap::new())),
            store: Arc::new(HttpObjectStore::new()?),
            on_queue,
            revision: Arc::new(AtomicU32::new(0)),
            account_generation: Arc::new(AtomicU64::new(0)),
            account_admission: Arc::new(AsyncMutex::new(())),
            snapshot_root: Arc::new(snapshot_root),
            temp_cleanup,
        })
    }

    #[must_use]
    pub fn with_snapshot_root(mut self, root: PathBuf) -> Self {
        self.temp_cleanup.add_root(root.clone());
        self.snapshot_root = Arc::new(root);
        self
    }

    #[must_use]
    pub fn with_temp_cleanup(mut self, cleanup: Arc<TempCleanup>) -> Self {
        cleanup.add_root((*self.snapshot_root).clone());
        self.temp_cleanup = cleanup;
        self
    }

    /// Cancels and forgets every upload owned by the account being left.
    ///
    /// The admission boundary closes before the generation changes. An
    /// operation that is currently between awaits therefore becomes stale
    /// before it can enter the queue. The final empty publication has a newer
    /// revision than every event emitted before this boundary, so a delayed
    /// renderer event cannot resurrect the old queue.
    pub async fn clear_account_scope(&self) {
        let _admission = self.account_admission.lock().await;
        self.account_generation.fetch_add(1, Ordering::SeqCst);

        let tokens: Vec<CancellationToken> = self
            .cancels
            .lock()
            .await
            .drain()
            .map(|(_, token)| token)
            .collect();
        for token in tokens {
            token.cancel();
        }

        let temporary_sources = {
            let mut records = self.records.lock().await;
            let mut sources = self.sources.lock().await;
            let paths = records
                .iter()
                .filter(|(_, record)| record.has_temporary_source())
                .filter_map(|(id, _)| sources.get(id).cloned().map(|path| (id.clone(), path)))
                .collect::<Vec<_>>();
            records.clear();
            sources.clear();
            let mut leases = self.temporary_leases.lock().await;
            paths
                .into_iter()
                .map(|(id, path)| (path, leases.remove(&id)))
                .collect::<Vec<_>>()
        };
        for (path, lease) in temporary_sources {
            if let Some(lease) = lease {
                lease.remove_file(&path).await;
            }
        }

        let _ = self.publish().await;
    }

    fn generation_is_current(&self, generation: u64) -> bool {
        self.account_generation.load(Ordering::SeqCst) == generation
    }

    pub(crate) fn current_account_generation(&self) -> u64 {
        self.account_generation.load(Ordering::SeqCst)
    }

    /// Opens the native picker for a chat attachment.
    ///
    /// Only picks: nothing is queued, and no conversation needs to exist yet,
    /// so a cancelled picker leaves nothing behind. The account generation is
    /// captured before the picker opens, so a sign-out while it is open cannot
    /// admit the file under the next account.
    ///
    /// Attachment-only, and takes no purpose: a dataset is picked through
    /// `datasets::commands::dataset_pick`, which stages the file for a
    /// confirmation step instead of starting a transfer. A purpose parameter
    /// here would have exactly one legal value.
    pub fn pick_attachment<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<PickedAttachment, UploadServiceError> {
        let account_generation = self.current_account_generation();
        let picked = app
            .dialog()
            .file()
            .blocking_pick_file()
            .ok_or(UploadServiceError::NoFileChosen)?;
        let path = picked
            .into_path()
            .map_err(|_| UploadServiceError::Unreadable)?;
        Ok(PickedAttachment {
            path,
            account_generation,
        })
    }

    /// Queues a picked file for `conversation_id`.
    ///
    /// Returns without transferring: the caller starts the transfer, so a user
    /// who picked the wrong file can cancel it from the queue before any byte
    /// leaves the machine.
    pub async fn enqueue_picked(
        &self,
        picked: PickedAttachment,
        conversation_id: String,
    ) -> Result<UploadQueueV1, UploadServiceError> {
        self.enqueue_snapshot_paths_for_generation(
            vec![picked.path],
            conversation_id,
            picked.account_generation,
        )
        .await
        .map(|(_, queue)| queue)
    }

    /// Queues an already-resolved attachment path.
    ///
    /// Not reachable from a command — see the module doc comment. Split out so
    /// tests can exercise queueing without an OS dialog.
    #[cfg(test)]
    pub(crate) async fn enqueue_attachment(
        &self,
        path: PathBuf,
        conversation_id: String,
    ) -> Result<UploadQueueV1, UploadServiceError> {
        let account_generation = self.current_account_generation();
        self.enqueue_snapshot_paths_for_generation(vec![path], conversation_id, account_generation)
            .await
            .map(|(_, queue)| queue)
    }

    #[cfg(test)]
    pub(crate) async fn enqueue_attachment_for_generation(
        &self,
        path: PathBuf,
        conversation_id: String,
        account_generation: u64,
    ) -> Result<UploadQueueV1, UploadServiceError> {
        self.enqueue_snapshot_paths_for_generation(vec![path], conversation_id, account_generation)
            .await
            .map(|(_, queue)| queue)
    }

    /// Copies renderer-provided bytes into an app-owned snapshot and queues it.
    pub async fn enqueue_attachment_bytes(
        &self,
        display_name: String,
        bytes: Vec<u8>,
        conversation_id: String,
    ) -> Result<(Vec<UploadId>, UploadQueueV1), UploadServiceError> {
        let generation = self.current_account_generation();
        self.enqueue_attachment_bytes_for_generation(
            display_name,
            bytes,
            conversation_id,
            generation,
        )
        .await
    }

    pub async fn enqueue_attachment_bytes_for_generation(
        &self,
        display_name: String,
        bytes: Vec<u8>,
        conversation_id: String,
        generation: u64,
    ) -> Result<(Vec<UploadId>, UploadQueueV1), UploadServiceError> {
        self.validate_conversation(&conversation_id)?;
        let snapshot = self.snapshot_bytes(display_name, bytes).await?;
        match self
            .admit_snapshot_attachments(vec![snapshot], conversation_id, generation)
            .await
        {
            Ok(result) => Ok(result),
            Err(error) => {
                // Admission can fail after the snapshot write if the account
                // changed or another upload took the last slot.
                // `snapshot` moved into the call, so remove every still-unowned
                // directory in the call itself on its error path.
                Err(error)
            }
        }
    }

    /// Copies native clipboard file references into app-owned snapshots and
    /// admits the entire paste atomically.
    pub async fn enqueue_snapshot_paths(
        &self,
        paths: Vec<PathBuf>,
        conversation_id: String,
    ) -> Result<(Vec<UploadId>, UploadQueueV1), UploadServiceError> {
        let generation = self.current_account_generation();
        self.enqueue_snapshot_paths_for_generation(paths, conversation_id, generation)
            .await
    }

    pub async fn enqueue_snapshot_paths_for_generation(
        &self,
        paths: Vec<PathBuf>,
        conversation_id: String,
        account_generation: u64,
    ) -> Result<(Vec<UploadId>, UploadQueueV1), UploadServiceError> {
        self.validate_conversation(&conversation_id)?;
        if paths.len() > MAX_CONCURRENT_UPLOADS {
            return Err(UploadServiceError::TooManyUploads);
        }
        let mut snapshots = Vec::with_capacity(paths.len());
        for path in paths {
            if let Err(error) = validate_native_path(&path) {
                discard_snapshots(&snapshots).await;
                return Err(error);
            }
            match self.snapshot_path(&path).await {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(error) => {
                    discard_snapshots(&snapshots).await;
                    return Err(error);
                }
            }
        }
        self.admit_snapshot_attachments(snapshots, conversation_id, account_generation)
            .await
    }

    fn validate_conversation(&self, conversation_id: &str) -> Result<(), UploadServiceError> {
        if Uuid::parse_str(conversation_id).is_err() {
            Err(UploadServiceError::ConversationRequired)
        } else {
            Ok(())
        }
    }

    async fn snapshot_path(&self, path: &Path) -> Result<AttachmentSnapshot, UploadServiceError> {
        let display_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or(UploadServiceError::UnsupportedType)?;
        let path_metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|_| UploadServiceError::Unreadable)?;
        if !path_metadata.file_type().is_file() {
            return Err(UploadServiceError::Unreadable);
        }
        let file = open_snapshot_source(path)
            .await
            .map_err(|_| UploadServiceError::Unreadable)?;
        let metadata = file
            .metadata()
            .await
            .map_err(|_| UploadServiceError::Unreadable)?;
        if !metadata.is_file() {
            return Err(UploadServiceError::Unreadable);
        }
        if metadata.len() > MAX_CHAT_ATTACHMENT_BYTES {
            return Err(UploadServiceError::TooLarge);
        }
        use tokio::io::AsyncReadExt as _;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_CHAT_ATTACHMENT_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| UploadServiceError::Unreadable)?;
        if bytes.len() as u64 > MAX_CHAT_ATTACHMENT_BYTES {
            return Err(UploadServiceError::TooLarge);
        }
        self.snapshot_bytes(display_name, bytes).await
    }

    async fn snapshot_bytes(
        &self,
        display_name: String,
        bytes: Vec<u8>,
    ) -> Result<AttachmentSnapshot, UploadServiceError> {
        if bytes.len() as u64 > MAX_CHAT_ATTACHMENT_BYTES {
            return Err(UploadServiceError::TooLarge);
        }
        let display_name = sanitize_display_name(&display_name);
        let display_path = Path::new(&display_name);
        let Some((detected_type, _)) = classify_chat_attachment(display_path) else {
            return Err(UploadServiceError::UnsupportedType);
        };
        let bytes = if crate::documents::is_office_path(display_path) {
            validate_office_attachment(display_path.to_path_buf(), bytes).await?
        } else {
            verify_chat_attachment_content(display_path, &bytes)
                .map_err(map_chat_attachment_content_error)?;
            bytes
        };
        let extension = display_path
            .extension()
            .and_then(|extension| extension.to_str())
            .ok_or(UploadServiceError::UnsupportedType)?
            .to_ascii_lowercase();
        let id = Uuid::new_v4();
        let directory = self.snapshot_root.join(id.to_string());
        let lease = self
            .temp_cleanup
            .lease_directory(&self.snapshot_root, id)
            .map_err(|_| UploadServiceError::Unreadable)?;
        tokio::fs::create_dir_all(self.snapshot_root.as_ref())
            .await
            .map_err(|_| UploadServiceError::Unreadable)?;
        tokio::fs::create_dir(&directory)
            .await
            .map_err(|_| UploadServiceError::Unreadable)?;
        let path = directory.join(format!("attachment.{extension}"));
        let write = async {
            use tokio::io::AsyncWriteExt as _;
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await?;
            file.write_all(&bytes).await?;
            file.flush().await?;
            Ok::<(), std::io::Error>(())
        }
        .await;
        if write.is_err() {
            lease.remove_file(&path).await;
            return Err(UploadServiceError::Unreadable);
        }
        Ok(AttachmentSnapshot {
            path,
            lease,
            size: bytes.len() as u64,
            detected_type: detected_type.to_owned(),
            display_name,
        })
    }

    async fn admit_snapshot_attachments(
        &self,
        snapshots: Vec<AttachmentSnapshot>,
        conversation_id: String,
        account_generation: u64,
    ) -> Result<(Vec<UploadId>, UploadQueueV1), UploadServiceError> {
        if snapshots.is_empty() {
            return Ok((Vec::new(), self.queue().await));
        }
        let _admission = self.account_admission.lock().await;
        if !self.generation_is_current(account_generation) || self.sessions.access_token().is_none()
        {
            discard_snapshots(&snapshots).await;
            return Err(UploadServiceError::AuthRequired);
        }
        let mut records = self.records.lock().await;
        let mut sources = self.sources.lock().await;
        let occupied = records
            .iter()
            .filter(|(id, record)| {
                record.state().is_active()
                    || (record.has_temporary_source() && sources.contains_key(id.as_str()))
            })
            .count();
        if occupied + snapshots.len() > MAX_CONCURRENT_UPLOADS {
            drop(sources);
            drop(records);
            discard_snapshots(&snapshots).await;
            return Err(UploadServiceError::TooManyUploads);
        }

        let mut ids = Vec::with_capacity(snapshots.len());
        for snapshot in snapshots {
            let AttachmentSnapshot {
                path,
                lease,
                size,
                detected_type,
                display_name,
            } = snapshot;
            let id = UploadId::from(Uuid::new_v4().to_string());
            let source = LocalUploadSource::new(path.clone(), size, detected_type);
            let mut record = UploadRecord::new(id.clone(), UploadPurpose::ChatAttachment, source);
            record.set_display_name(&display_name);
            record.set_file_name(&display_name);
            record.set_conversation_id(&conversation_id);
            record.mark_temporary_source();
            sources.insert(id.as_str().to_owned(), path);
            self.temporary_leases
                .lock()
                .await
                .insert(id.as_str().to_owned(), lease);
            records.insert(id.as_str().to_owned(), record);
            ids.push(id);
        }
        drop(sources);
        drop(records);
        let queue = self.publish().await;
        Ok((ids, queue))
    }

    /// Queues a dataset the person has already previewed and confirmed.
    ///
    /// `upload_path` is what actually goes to tenant storage: the picked file,
    /// or the CSV a cleanup recipe produced. A lease protects temporary output
    /// until transfer cleanup or the last retryable queue entry is forgotten.
    pub(crate) async fn enqueue_dataset(
        &self,
        upload_path: PathBuf,
        display_name: &str,
        ingest: DatasetIngest,
        temporary_lease: Option<TempLease>,
        account_generation: u64,
    ) -> Result<(UploadId, UploadQueueV1), UploadServiceError> {
        let temporary_source = temporary_lease.is_some();
        let size = match self.readable_size(&upload_path).await {
            Ok(size) => size,
            Err(error) => {
                if let Some(lease) = temporary_lease {
                    lease.remove_file(&upload_path).await;
                }
                return Err(error);
            }
        };
        if size > MAX_UPLOAD_BYTES {
            if let Some(lease) = temporary_lease {
                lease.remove_file(&upload_path).await;
            }
            return Err(UploadServiceError::TooLarge);
        }
        let detected_type = guess_content_type(&upload_path).to_owned();
        let source = LocalUploadSource::new(upload_path, size, detected_type);
        let display_name = display_name.to_owned();
        self.admit(
            source,
            UploadPurpose::PersistentDataset,
            move |record| {
                record.set_display_name(&display_name);
                record.set_ingest(ingest);
                if temporary_source {
                    record.mark_temporary_source();
                }
            },
            temporary_lease,
            account_generation,
        )
        .await
    }

    /// The file's size, or why it cannot be read.
    async fn readable_size(&self, path: &Path) -> Result<u64, UploadServiceError> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|_| UploadServiceError::Unreadable)?;
        if !metadata.is_file() {
            return Err(UploadServiceError::Unreadable);
        }
        Ok(metadata.len())
    }

    /// Adds a record to the queue, holding its path apart from anything that
    /// serializes.
    async fn admit(
        &self,
        source: LocalUploadSource,
        purpose: UploadPurpose,
        describe: impl FnOnce(&mut UploadRecord),
        temporary_lease: Option<TempLease>,
        account_generation: u64,
    ) -> Result<(UploadId, UploadQueueV1), UploadServiceError> {
        let path = source.private_path().to_path_buf();
        let _admission = self.account_admission.lock().await;
        if !self.generation_is_current(account_generation) || self.sessions.access_token().is_none()
        {
            if let Some(lease) = temporary_lease {
                lease.remove_file(&path).await;
            }
            return Err(UploadServiceError::AuthRequired);
        }
        if self.active_count().await >= MAX_CONCURRENT_UPLOADS {
            if let Some(lease) = temporary_lease {
                lease.remove_file(&path).await;
            }
            return Err(UploadServiceError::TooManyUploads);
        }
        let id = UploadId::from(Uuid::new_v4().to_string());
        let mut record = UploadRecord::new(id.clone(), purpose, source);
        describe(&mut record);

        self.sources
            .lock()
            .await
            .insert(id.as_str().to_owned(), path);
        if let Some(lease) = temporary_lease {
            self.temporary_leases
                .lock()
                .await
                .insert(id.as_str().to_owned(), lease);
        }
        self.records
            .lock()
            .await
            .insert(id.as_str().to_owned(), record);
        let queue = self.publish().await;
        Ok((id, queue))
    }

    /// Cancels an upload, whether or not it has started.
    ///
    /// A multipart upload that has sent parts is also aborted server-side.
    /// Without that, S3 keeps the uploaded parts indefinitely and bills the
    /// tenant for storage nobody can reach — an abandoned multipart upload is
    /// invisible in every normal listing.
    pub async fn cancel(&self, upload_id: &str) -> Result<UploadQueueV1, UploadServiceError> {
        let outstanding = {
            let _admission = self.account_admission.lock().await;
            if let Some(token) = self.cancels.lock().await.remove(upload_id) {
                token.cancel();
            }
            let mut records = self.records.lock().await;
            let record = records
                .get_mut(upload_id)
                .ok_or(UploadServiceError::NotFound)?;
            let outstanding = record
                .multipart_upload_id()
                .zip(record.da_upload_id())
                .map(|(upload, dataset)| (upload.to_owned(), dataset.to_owned()));
            record.cancel();
            outstanding
        };

        if let Some((multipart_id, dataset_id)) = outstanding {
            // Best-effort: the local cancellation is already recorded, and a
            // failed abort must not turn a successful cancel into an error the
            // user has to act on. DA expires abandoned uploads on its own
            // schedule as the backstop.
            if let Ok((config, bearer)) = self.authenticated().await {
                if let Ok(repository) = DaUploadRepository::new(config.da_base_url.as_str(), bearer)
                {
                    let _ = repository.abort(&multipart_id, &dataset_id).await;
                }
            }
        }

        self.discard_temporary_source(upload_id).await;
        self.forget(upload_id).await
    }

    /// Drops a queue row. Used after a successful send binds ready
    /// attachments, and after cancel so a dropped chip does not linger.
    pub async fn forget(&self, upload_id: &str) -> Result<UploadQueueV1, UploadServiceError> {
        self.cancels.lock().await.remove(upload_id);
        let source = self.sources.lock().await.remove(upload_id);
        let lease = self.temporary_leases.lock().await.remove(upload_id);
        if let (Some(path), Some(lease)) = (source, lease) {
            lease.remove_file(&path).await;
        }
        self.records
            .lock()
            .await
            .shift_remove(upload_id)
            .ok_or(UploadServiceError::NotFound)?;
        Ok(self.publish().await)
    }

    /// Removes ready chat attachments that were just bound onto a message.
    pub async fn forget_by_resource_ids(&self, resource_ids: &[Uuid]) -> UploadQueueV1 {
        let wanted: std::collections::HashSet<String> =
            resource_ids.iter().map(ToString::to_string).collect();
        let drop_keys: Vec<String> = {
            let records = self.records.lock().await;
            records
                .iter()
                .filter_map(|(key, record)| {
                    if record.state() != UploadState::Ready {
                        return None;
                    }
                    let resource_id = record.da_upload_id()?;
                    wanted.contains(resource_id).then(|| key.clone())
                })
                .collect()
        };
        {
            let mut records = self.records.lock().await;
            let mut sources = self.sources.lock().await;
            for key in &drop_keys {
                records.shift_remove(key);
                sources.remove(key);
            }
        }
        {
            let mut leases = self.temporary_leases.lock().await;
            for key in &drop_keys {
                leases.remove(key);
            }
        }
        {
            let mut cancels = self.cancels.lock().await;
            for key in &drop_keys {
                cancels.remove(key);
            }
        }
        self.publish().await
    }

    /// Returns a failed upload to the queue, ready to start over.
    pub async fn retry(&self, upload_id: &str) -> Result<UploadQueueV1, UploadServiceError> {
        let mut records = self.records.lock().await;
        let record = records
            .get_mut(upload_id)
            .ok_or(UploadServiceError::NotFound)?;
        record
            .transition(UploadState::Preparing)
            .map_err(|_| UploadServiceError::NotRetryable)?;
        // Back to `Selected` semantics: `start` drives the whole flow from
        // `Preparing`, and leaving it here would mean two entry points into
        // the same state.
        drop(records);
        Ok(self.publish().await)
    }

    pub async fn queue(&self) -> UploadQueueV1 {
        self.snapshot().await
    }

    #[cfg(test)]
    pub(crate) async fn source_path_for_test(&self, upload_id: &str) -> Option<PathBuf> {
        self.sources.lock().await.get(upload_id).cloned()
    }

    #[cfg(test)]
    pub(crate) async fn has_cancel_for_test(&self, upload_id: &str) -> bool {
        self.cancels.lock().await.contains_key(upload_id)
    }

    #[cfg(test)]
    pub(crate) async fn operation_lease_for_test(
        &self,
        upload_id: &str,
    ) -> Result<Option<TempLease>, UploadServiceError> {
        self.operation_lease(upload_id).await
    }

    #[cfg(test)]
    pub(crate) async fn register_start_for_test(
        &self,
        upload_id: &str,
        account_generation: u64,
    ) -> Result<Option<CancellationToken>, UploadServiceError> {
        self.register_start(upload_id, account_generation).await
    }

    #[cfg(test)]
    pub(crate) async fn fail_for_test(&self, upload_id: &str) {
        self.fail(upload_id, "test failure").await;
    }

    #[cfg(test)]
    pub(crate) async fn complete_chat_attachment_for_test(
        &self,
        upload_id: &str,
        attachment_id: String,
        size: u64,
    ) {
        self.advance(upload_id, UploadState::Preparing).await;
        self.advance(upload_id, UploadState::Uploading).await;
        self.advance(upload_id, UploadState::Finalizing).await;
        self.complete_chat_attachment(upload_id, attachment_id, size)
            .await;
    }

    async fn active_count(&self) -> usize {
        self.records
            .lock()
            .await
            .values()
            .filter(|record| record.state().is_active())
            .count()
    }

    pub async fn has_active_work(&self) -> bool {
        self.active_count().await > 0
    }

    async fn snapshot(&self) -> UploadQueueV1 {
        let records = self.records.lock().await;
        // Newest first: the thing a user just did is the thing they are
        // looking for.
        UploadQueueV1::new(
            self.revision.load(Ordering::SeqCst),
            records
                .values()
                .rev()
                .map(UploadRecord::projection)
                .collect(),
        )
    }

    async fn publish(&self) -> UploadQueueV1 {
        let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let records = self.records.lock().await;
        let queue = UploadQueueV1::new(
            revision,
            records
                .values()
                .rev()
                .map(UploadRecord::projection)
                .collect(),
        );
        drop(records);
        (self.on_queue)(&queue);
        queue
    }

    /// Runs one queued upload to completion.
    ///
    /// Every state change publishes, so the queue the UI shows is always this
    /// process's own record of what happened rather than an optimistic guess.
    pub async fn start(&self, upload_id: &str) -> Result<(), UploadServiceError> {
        let account_generation = self.account_generation.load(Ordering::SeqCst);
        let (config, bearer) = self.authenticated().await?;
        if !self.generation_is_current(account_generation) {
            return Ok(());
        }
        // Keep the generated source alive while this command opens or reads
        // it. Cancel and account-clear may remove the queue's owning lease.
        let operation_lease = self.operation_lease(upload_id).await?;
        let path = self
            .sources
            .lock()
            .await
            .get(upload_id)
            .cloned()
            .ok_or(UploadServiceError::NotFound)?;

        let purpose = {
            let records = self.records.lock().await;
            let record = records.get(upload_id).ok_or(UploadServiceError::NotFound)?;
            if record.has_temporary_source() && operation_lease.is_none() {
                return Err(UploadServiceError::NotFound);
            }
            record.purpose()
        };
        let Some(cancel) = self.register_start(upload_id, account_generation).await? else {
            return Ok(());
        };
        if purpose == UploadPurpose::ChatAttachment {
            return self
                .start_chat_attachment(
                    upload_id,
                    &path,
                    (config, bearer),
                    operation_lease,
                    account_generation,
                    cancel,
                )
                .await;
        }

        let (display_name, file_name, content_type, size, ingest) = {
            let records = self.records.lock().await;
            let record = records.get(upload_id).ok_or(UploadServiceError::NotFound)?;
            (
                record.display_name().to_owned(),
                record.file_name().to_owned(),
                record.source().detected_type().to_owned(),
                record.source().size(),
                record.ingest(),
            )
        };

        self.advance(upload_id, UploadState::Preparing).await;
        if cancel.is_cancelled() || !self.generation_is_current(account_generation) {
            return Ok(());
        }

        let repository = DaUploadRepository::new(config.da_base_url.as_str(), bearer)
            .map_err(|_| UploadServiceError::NotConfigured)?;
        let prepared = match repository
            .begin(&file_name, &content_type, size, &display_name, ingest)
            .await
        {
            Ok(prepared) => prepared,
            Err(refusal) => {
                self.fail(upload_id, refusal_message(&refusal)).await;
                return Ok(());
            }
        };
        if !self.generation_is_current(account_generation) || cancel.is_cancelled() {
            return Ok(());
        }

        // Recorded before any byte moves, not after: a cancel arriving
        // mid-transfer needs both ids to abort the multipart upload
        // server-side, and by then it is too late to learn them.
        {
            let mut records = self.records.lock().await;
            if let Some(record) = records.get_mut(upload_id) {
                if let Some(dataset_id) = prepared.dataset_id.as_deref() {
                    record.set_da_upload_id(dataset_id);
                }
                if let crate::uploads::da_repository::TransferPlan::Multipart {
                    upload_id: multipart_id,
                    ..
                } = &prepared.plan
                {
                    record.set_multipart_upload_id(multipart_id);
                }
            }
        }

        self.advance(upload_id, UploadState::Uploading).await;

        let records = Arc::clone(&self.records);
        let sink = Arc::clone(&self.on_queue);
        let revision = Arc::clone(&self.revision);
        let progress_id = upload_id.to_owned();
        let progress_generation = account_generation;
        let account_generation = Arc::clone(&self.account_generation);
        let on_progress: crate::uploads::transfer::ProgressSink = Arc::new(move |bytes| {
            // Progress arrives from the transfer task; the queue lock is async,
            // so hand the update to the runtime rather than blocking here.
            let records = Arc::clone(&records);
            let sink = Arc::clone(&sink);
            let revision = Arc::clone(&revision);
            let account_generation = Arc::clone(&account_generation);
            let id = progress_id.clone();
            tokio::spawn(async move {
                if account_generation.load(Ordering::SeqCst) != progress_generation {
                    return;
                }
                let mut guard = records.lock().await;
                if account_generation.load(Ordering::SeqCst) != progress_generation {
                    return;
                }
                if let Some(record) = guard.get_mut(&id) {
                    let _ = record.advance(bytes);
                }
                let queue = UploadQueueV1::new(
                    revision.fetch_add(1, Ordering::SeqCst) + 1,
                    guard.values().rev().map(UploadRecord::projection).collect(),
                );
                drop(guard);
                sink(&queue);
            });
        });

        let transfer = UploadTransfer {
            repository: &repository,
            store: self.store.as_ref(),
            on_progress,
        };
        let outcome = match transfer.run(&prepared, &path, &content_type, &cancel).await {
            Ok(outcome) => outcome,
            Err(TransferError::Cancelled) => {
                drop(operation_lease);
                self.cancel(upload_id).await.ok();
                return Ok(());
            }
            Err(_) => {
                self.fail(upload_id, "The upload was interrupted. You can try again.")
                    .await;
                return Ok(());
            }
        };
        if !self.generation_is_current(progress_generation) || cancel.is_cancelled() {
            return Ok(());
        }

        self.advance(upload_id, UploadState::Finalizing).await;

        let Some(dataset_id) = outcome.dataset_id.as_deref() else {
            // Legacy mode assigns no dataset until the in-band upload lands,
            // which this client does not implement. Failing loudly beats
            // reporting a ready dataset that does not exist.
            self.fail(
                upload_id,
                "This workspace requires an upload mode this app does not support yet.",
            )
            .await;
            return Ok(());
        };

        if let Err(refusal) = repository
            .finalize(
                dataset_id,
                outcome.upload_id.as_deref(),
                &outcome.parts,
                &display_name,
                &file_name,
                ingest,
            )
            .await
        {
            self.fail(upload_id, refusal_message(&refusal)).await;
            return Ok(());
        }
        if !self.generation_is_current(progress_generation) || cancel.is_cancelled() {
            return Ok(());
        }

        {
            let mut records = self.records.lock().await;
            if let Some(record) = records.get_mut(upload_id) {
                let _ = record.advance(size);
            }
        }
        self.cancels.lock().await.remove(upload_id);
        drop(operation_lease);
        self.discard_temporary_source(upload_id).await;
        self.advance(upload_id, UploadState::Ready).await;
        Ok(())
    }

    /// Removes a file this app wrote for the transfer.
    ///
    /// Called when a source no longer needs retrying. Failed uploads retain
    /// their source and lease; ready and cancelled uploads release both.
    async fn discard_temporary_source(&self, upload_id: &str) {
        let temporary = {
            let records = self.records.lock().await;
            records
                .get(upload_id)
                .is_some_and(UploadRecord::has_temporary_source)
        };
        if !temporary {
            return;
        }
        let path = self.sources.lock().await.remove(upload_id);
        let lease = self.temporary_leases.lock().await.remove(upload_id);
        if let (Some(path), Some(lease)) = (path, lease) {
            lease.remove_file(&path).await;
        }
    }

    async fn start_chat_attachment(
        &self,
        upload_id: &str,
        path: &Path,
        auth: (AuthConfig, String),
        operation_lease: Option<TempLease>,
        account_generation: u64,
        cancel: CancellationToken,
    ) -> Result<(), UploadServiceError> {
        let (config, bearer) = auth;
        let (display_name, conversation_id, size) = {
            let records = self.records.lock().await;
            let record = records.get(upload_id).ok_or(UploadServiceError::NotFound)?;
            (
                record.projection().display_name,
                record
                    .conversation_id()
                    .ok_or(UploadServiceError::ConversationRequired)?
                    .to_owned(),
                record.source().size(),
            )
        };
        let (content_type, kind) =
            classify_chat_attachment(path).ok_or(UploadServiceError::UnsupportedType)?;
        let create_kind = match kind {
            AttachmentKind::Image => ChatAttachmentCreateKind::Image,
            AttachmentKind::Tabular => ChatAttachmentCreateKind::Tabular,
            AttachmentKind::Text => ChatAttachmentCreateKind::Text,
        };

        self.advance(upload_id, UploadState::Preparing).await;
        if cancel.is_cancelled() || !self.generation_is_current(account_generation) {
            return Ok(());
        }

        let repository = DaUploadRepository::new(config.da_base_url.as_str(), bearer)
            .map_err(|_| UploadServiceError::NotConfigured)?;
        let prepared = match repository
            .presign_attachment(
                &conversation_id,
                &display_name,
                content_type,
                size,
                kind.as_str(),
            )
            .await
        {
            Ok(prepared) => prepared,
            Err(_) => {
                self.fail(
                    upload_id,
                    "The workspace could not accept this file. You can try again.",
                )
                .await;
                return Ok(());
            }
        };
        if !self.generation_is_current(account_generation) || cancel.is_cancelled() {
            return Ok(());
        }
        let Some(url) = prepared.url else {
            self.fail(
                upload_id,
                "This workspace requires an upload mode this app does not support yet.",
            )
            .await;
            return Ok(());
        };

        self.advance(upload_id, UploadState::Uploading).await;
        let body = match read_snapshot_for_transfer(path, size).await {
            Ok(body) => body,
            Err(_) => {
                self.fail(upload_id, "the staged file changed; paste it again")
                    .await;
                return Ok(());
            }
        };
        let path = Path::new(&display_name);
        let body = if crate::documents::is_office_path(path) {
            match validate_office_attachment(path.to_path_buf(), body).await {
                Ok(body) => body,
                Err(_) => {
                    self.fail(upload_id, "the staged file changed; paste it again")
                        .await;
                    return Ok(());
                }
            }
        } else {
            if verify_chat_attachment_content(path, &body).is_err() {
                self.fail(upload_id, "the staged file changed; paste it again")
                    .await;
                return Ok(());
            }
            body
        };
        if !self.generation_is_current(account_generation) || cancel.is_cancelled() {
            drop(operation_lease);
            self.cancel(upload_id).await.ok();
            return Ok(());
        }
        match self.store.put_part(url.expose(), body, content_type).await {
            Ok(_) | Err(TransferError::MissingEtag) => {}
            Err(TransferError::Cancelled) => {
                drop(operation_lease);
                self.cancel(upload_id).await.ok();
                return Ok(());
            }
            Err(_) => {
                self.fail(upload_id, "The upload was interrupted. You can try again.")
                    .await;
                return Ok(());
            }
        }
        if !self.generation_is_current(account_generation) || cancel.is_cancelled() {
            return Ok(());
        }
        (self.on_queue)(&self.snapshot().await);

        self.advance(upload_id, UploadState::Finalizing).await;
        let storage_key = prepared.storage_key.expose().to_owned();
        if repository.finalize_attachment(&storage_key).await.is_err() {
            self.fail(
                upload_id,
                "The file was uploaded but the workspace could not record it. You can try again.",
            )
            .await;
            return Ok(());
        }
        if !self.generation_is_current(account_generation) || cancel.is_cancelled() {
            return Ok(());
        }

        let conversation_uuid = uuid::Uuid::parse_str(&conversation_id)
            .map_err(|_| UploadServiceError::ConversationRequired)?;
        let size_i64 = i64::try_from(size).map_err(|_| UploadServiceError::TooLarge)?;
        let attachment_id = match repository
            .record_ready_attachment(
                conversation_uuid,
                create_kind,
                &display_name,
                content_type,
                size_i64,
                &storage_key,
            )
            .await
        {
            Ok(id) => id,
            Err(_) => {
                self.fail(
                    upload_id,
                    "The file was uploaded but the workspace could not record it. You can try again.",
                )
                .await;
                return Ok(());
            }
        };
        if !self.generation_is_current(account_generation) || cancel.is_cancelled() {
            return Ok(());
        }

        drop(operation_lease);
        self.complete_chat_attachment(upload_id, attachment_id.to_string(), size)
            .await;
        Ok(())
    }

    async fn operation_lease(
        &self,
        upload_id: &str,
    ) -> Result<Option<TempLease>, UploadServiceError> {
        self.temporary_leases
            .lock()
            .await
            .get(upload_id)
            .map(TempLease::try_clone)
            .transpose()
            .map_err(|_| UploadServiceError::Unreadable)
    }

    /// Reserves the single start attempt while holding the same short
    /// admission boundary used by cancellation and account changes. A queued
    /// row can be removed while start performs authentication and file
    /// lookups; it must still be present and startable when this commits.
    async fn register_start(
        &self,
        upload_id: &str,
        account_generation: u64,
    ) -> Result<Option<CancellationToken>, UploadServiceError> {
        let _admission = self.account_admission.lock().await;
        if !self.generation_is_current(account_generation) {
            return Ok(None);
        }
        {
            let records = self.records.lock().await;
            let record = records.get(upload_id).ok_or(UploadServiceError::NotFound)?;
            if !matches!(
                record.state(),
                UploadState::Selected | UploadState::Preparing
            ) {
                return Ok(None);
            }
        }
        let mut cancels = self.cancels.lock().await;
        if cancels.contains_key(upload_id) {
            return Ok(None);
        }
        let cancel = CancellationToken::new();
        cancels.insert(upload_id.to_owned(), cancel.clone());
        Ok(Some(cancel))
    }

    async fn complete_chat_attachment(&self, upload_id: &str, attachment_id: String, size: u64) {
        {
            let mut records = self.records.lock().await;
            if let Some(record) = records.get_mut(upload_id) {
                record.set_da_upload_id(attachment_id);
                let _ = record.advance(size);
            }
        }
        // Ready lets the renderer bind and forget this queue entry. Detach
        // and delete the snapshot before publishing that state so a send
        // cannot race cleanup and orphan the private copy.
        self.discard_temporary_source(upload_id).await;
        self.cancels.lock().await.remove(upload_id);
        self.advance(upload_id, UploadState::Ready).await;
    }

    async fn advance(&self, upload_id: &str, next: UploadState) {
        {
            let mut records = self.records.lock().await;
            if let Some(record) = records.get_mut(upload_id) {
                let _ = record.transition(next);
            }
        }
        let _ = self.publish().await;
    }

    async fn fail(&self, upload_id: &str, message: &str) {
        // Failed datasets remain retryable. Keep their generated CSV and lease
        // until success, cancellation, or the queue row is forgotten.
        {
            let mut records = self.records.lock().await;
            if let Some(record) = records.get_mut(upload_id) {
                record.fail(message);
            }
        }
        self.cancels.lock().await.remove(upload_id);
        let _ = self.publish().await;
    }

    /// The session an upload will be sent with, renewed first if its bearer is
    /// about to expire. A multipart upload can outlive the token it started
    /// with, so this is the difference between a large transfer completing and
    /// failing at part 40 of 60.
    /// The tenant configuration and a fresh bearer.
    ///
    /// `pub(crate)` because the datasets destination builds its own repository
    /// for catalog reads: those are not transfers, so routing them through the
    /// queue would give the queue a second, unrelated job.
    pub(crate) async fn authenticated(&self) -> Result<(AuthConfig, String), UploadServiceError> {
        let config = AuthConfig::from_env().map_err(|_| UploadServiceError::NotConfigured)?;
        let _ = ensure_fresh_session(&self.sessions).await;
        let bearer = self
            .sessions
            .access_token()
            .ok_or(UploadServiceError::AuthRequired)?;
        Ok((config, bearer.to_string()))
    }
}

fn validate_native_path(path: &Path) -> Result<(), UploadServiceError> {
    if !path.is_absolute() {
        return Err(UploadServiceError::Unreadable);
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        match path.components().next() {
            Some(Component::Prefix(prefix))
                if matches!(
                    prefix.kind(),
                    Prefix::UNC(..)
                        | Prefix::VerbatimUNC(..)
                        | Prefix::DeviceNS(..)
                        | Prefix::Verbatim(..)
                        | Prefix::VerbatimDisk(..)
                ) =>
            {
                return Err(UploadServiceError::Unreadable);
            }
            _ => {}
        }
    }
    Ok(())
}

async fn discard_snapshots(snapshots: &[AttachmentSnapshot]) {
    for snapshot in snapshots {
        snapshot.lease.remove_file(&snapshot.path).await;
    }
}

#[cfg(target_os = "macos")]
async fn open_snapshot_source(path: &Path) -> Result<tokio::fs::File, std::io::Error> {
    use rustix::fs::{Mode, OFlags};

    // NONBLOCK prevents a raced FIFO from hanging the command. NOFOLLOW
    // preserves the explicit regular-file check across the open itself.
    let descriptor = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW,
        Mode::empty(),
    )?;
    Ok(tokio::fs::File::from_std(std::fs::File::from(descriptor)))
}

#[cfg(not(target_os = "macos"))]
async fn open_snapshot_source(path: &Path) -> Result<tokio::fs::File, std::io::Error> {
    tokio::fs::File::open(path).await
}

async fn read_snapshot_for_transfer(
    path: &Path,
    expected_size: u64,
) -> Result<Vec<u8>, std::io::Error> {
    use tokio::io::AsyncReadExt as _;

    if expected_size > MAX_CHAT_ATTACHMENT_BYTES {
        return Err(std::io::Error::other("snapshot exceeds attachment bound"));
    }
    let file = tokio::fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() || metadata.len() != expected_size {
        return Err(std::io::Error::other("snapshot size changed"));
    }
    let mut bytes = Vec::with_capacity(expected_size as usize);
    file.take(expected_size + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 != expected_size {
        return Err(std::io::Error::other("snapshot size changed"));
    }
    Ok(bytes)
}

fn map_chat_attachment_content_error(error: ChatAttachmentContentError) -> UploadServiceError {
    match error {
        ChatAttachmentContentError::Empty => UploadServiceError::Empty,
        ChatAttachmentContentError::UnsupportedType => UploadServiceError::UnsupportedType,
        ChatAttachmentContentError::ContentMismatch => UploadServiceError::ContentMismatch,
        ChatAttachmentContentError::OfficeExtraction(message) => {
            UploadServiceError::OfficeExtraction(message)
        }
    }
}

async fn validate_office_attachment(
    path: PathBuf,
    bytes: Vec<u8>,
) -> Result<Vec<u8>, UploadServiceError> {
    let (bytes, result) = tokio::task::spawn_blocking(move || {
        let result = verify_chat_attachment_content(&path, &bytes);
        (bytes, result)
    })
    .await
    .map_err(|_| UploadServiceError::Unreadable)?;
    result.map_err(map_chat_attachment_content_error)?;
    Ok(bytes)
}

/// App-owned attachment snapshots never survive a process restart.
#[must_use]
pub fn snapshot_directory(app_local_data: &Path) -> PathBuf {
    app_local_data.join("upload-snapshots")
}

/// Best-effort content type from the file extension.
///
/// Deliberately conservative: DA re-derives what a file actually is on
/// ingest, and an over-confident guess here would only mislabel an object.
fn guess_content_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("csv") => "text/csv",
        Some("tsv") => "text/tab-separated-values",
        Some("json") => "application/json",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("xls") => "application/vnd.ms-excel",
        Some("parquet") => "application/vnd.apache.parquet",
        Some("txt") => "text/plain",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}
