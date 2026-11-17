use std::sync::Arc;

use base64::Engine as _;
use tauri::Manager;
use tokio::sync::OwnedRwLockReadGuard;
use xfalcon_contracts::uploads::{UploadId, UploadQueueV1};
use xfalcon_contracts::{mcp::McpServerId, ConversationId};

use crate::app::commands::AppRuntimeState;
use crate::uploads::clipboard;
use crate::uploads::kind::MAX_CHAT_ATTACHMENT_BYTES;
use crate::uploads::service::UploadService;
use crate::work_barrier::WorkBarrier;

/// Picks a chat attachment and starts sending it.
///
/// With no `conversation_id` -- a new chat that has nothing sent yet -- the
/// picker opens first, and only a chosen file reserves a conversation to hold
/// it: DA takes an attachment only into a conversation, but the person stays
/// on the screen they were on, and a cancelled picker creates nothing (spec:
/// chat-attachments). The queued upload names the reserved conversation.
///
/// Attachment-only, and takes no purpose. A dataset is picked by
/// `dataset_pick`, which stages the file so a person can preview and confirm
/// it before any byte moves; a purpose parameter here would have exactly one
/// legal value (spec: dataset-uploads).
#[tauri::command]
pub async fn upload_choose(
    app: tauri::AppHandle,
    conversation_id: Option<String>,
    mcp_server_id: Option<McpServerId>,
) -> Result<UploadQueueV1, String> {
    let barrier = app.state::<Arc<WorkBarrier>>().inner().clone();
    let admission = barrier.admit_owned().await.map_err(str::to_owned)?;
    let service = app.state::<Arc<UploadService>>().inner().clone();
    let picked = service
        .pick_attachment(&app)
        .map_err(|error| error.to_string())?;
    let target = ConversationTarget::resolve(&app, conversation_id, mcp_server_id).await?;
    let queue = match service.enqueue_picked(picked, target.id().to_owned()).await {
        Ok(queue) => queue,
        Err(error) => {
            target.release(&app).await;
            return Err(error.to_string());
        }
    };
    if let Some(upload) = queue.uploads.first() {
        let id = upload.id.as_str().to_owned();
        tokio::spawn(async move {
            let _admission = admission;
            if let Err(error) = service.start(&id).await {
                tracing::warn!(%error, upload = %id, "chat attachment start failed");
            }
        });
    }
    Ok(queue)
}

/// Queues validated renderer clipboard bytes or native file-manager references.
///
/// The renderer may provide bytes and a filename together. It can never
/// provide a filesystem path. When both values are absent, the native reader
/// handles CF_HDROP/Finder references that WebView paste events omit.
#[tauri::command]
pub async fn upload_paste(
    app: tauri::AppHandle,
    conversation_id: Option<String>,
    name: Option<String>,
    data_base64: Option<String>,
    mcp_server_id: Option<McpServerId>,
) -> Result<UploadQueueV1, String> {
    let focused = app
        .get_webview_window("main")
        .ok_or_else(|| "the chat window is unavailable".to_owned())?
        .is_focused()
        .map_err(|_| "the chat window focus could not be verified".to_owned())?;
    if !focused {
        return Err("focus the chat window before pasting files".to_owned());
    }

    let service = app.state::<Arc<UploadService>>().inner().clone();
    // Capture before the native clipboard read or work-barrier wait. A logout
    // and login during either await must not admit the old clipboard under the
    // new account.
    let account_generation = service.current_account_generation();
    let input = match (name, data_base64) {
        (Some(name), Some(encoded)) => {
            let max_encoded = (MAX_CHAT_ATTACHMENT_BYTES as usize).div_ceil(3) * 4;
            if encoded.len() > max_encoded {
                return Err("that file is larger than this workspace accepts".to_owned());
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| "the clipboard file data is invalid".to_owned())?;
            PasteInput::Bytes { name, bytes }
        }
        (None, None) => {
            let paths = tokio::task::spawn_blocking(clipboard::native_file_paths)
                .await
                .map_err(|_| "the clipboard could not be read".to_owned())?
                .map_err(|error| error.to_string())?;
            if paths.is_empty() {
                return Ok(service.queue().await);
            }
            PasteInput::NativePaths(paths)
        }
        _ => return Err("clipboard file name and data must be provided together".to_owned()),
    };

    let barrier = app.state::<Arc<WorkBarrier>>().inner().clone();
    let admission = barrier.admit_owned().await.map_err(str::to_owned)?;
    // Reserved only now: the clipboard held a file, so there is something for
    // the conversation to hold.
    let target = ConversationTarget::resolve(&app, conversation_id, mcp_server_id).await?;
    let conversation_id = target.id().to_owned();
    let queued = match input {
        PasteInput::Bytes { name, bytes } => {
            service
                .enqueue_attachment_bytes_for_generation(
                    name,
                    bytes,
                    conversation_id,
                    account_generation,
                )
                .await
        }
        PasteInput::NativePaths(paths) => {
            service
                .enqueue_snapshot_paths_for_generation(paths, conversation_id, account_generation)
                .await
        }
    };
    let (ids, queue) = match queued {
        Ok(queued) => queued,
        Err(error) => {
            target.release(&app).await;
            return Err(error.to_string());
        }
    };
    start_all(service, ids, admission);
    Ok(queue)
}

/// The conversation an attachment goes into: the open one, or one reserved
/// for it because none is open yet.
enum ConversationTarget {
    Open(String),
    Reserved(String),
}

impl ConversationTarget {
    async fn resolve(
        app: &tauri::AppHandle,
        conversation_id: Option<String>,
        mcp_server_id: Option<McpServerId>,
    ) -> Result<Self, String> {
        match conversation_id {
            Some(conversation_id) => {
                app.state::<AppRuntimeState>()
                    .chat()
                    .mcp_selection_validate(
                        ConversationId::from(conversation_id.clone()),
                        mcp_server_id,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(Self::Open(conversation_id))
            }
            None => app
                .state::<AppRuntimeState>()
                .chat()
                .conversation_reserve(mcp_server_id)
                .await
                .map(|reserved| Self::Reserved(reserved.as_str().to_owned()))
                .map_err(|error| error.to_string()),
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::Open(id) | Self::Reserved(id) => id,
        }
    }

    /// Deletes a conversation reserved for a file that was then refused, so a
    /// failed attach leaves no empty chat behind. The open one is never
    /// touched.
    async fn release(self, app: &tauri::AppHandle) {
        let Self::Reserved(id) = self else {
            return;
        };
        if let Err(error) = app
            .state::<AppRuntimeState>()
            .chat()
            .conversation_delete(xfalcon_contracts::ConversationId::from(id), None)
            .await
        {
            tracing::warn!(%error, "reserved conversation could not be released after a refused attachment");
        }
    }
}

enum PasteInput {
    Bytes { name: String, bytes: Vec<u8> },
    NativePaths(Vec<std::path::PathBuf>),
}

fn start_all(service: Arc<UploadService>, ids: Vec<UploadId>, admission: OwnedRwLockReadGuard<()>) {
    tokio::spawn(async move {
        let _admission = admission;
        let starts = ids.into_iter().map(|id| {
            let service = Arc::clone(&service);
            async move {
                if let Err(error) = service.start(id.as_str()).await {
                    tracing::warn!(%error, upload = %id.as_str(), "pasted attachment start failed");
                }
            }
        });
        futures_util::future::join_all(starts).await;
    });
}

#[tauri::command]
pub async fn upload_cancel(
    app: tauri::AppHandle,
    upload_id: String,
) -> Result<UploadQueueV1, String> {
    app.state::<Arc<UploadService>>()
        .cancel(&upload_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn upload_retry(
    app: tauri::AppHandle,
    upload_id: String,
) -> Result<UploadQueueV1, String> {
    let barrier = app.state::<Arc<WorkBarrier>>().inner().clone();
    let admission = barrier.admit_owned().await.map_err(str::to_owned)?;
    let service = app.state::<Arc<UploadService>>().inner().clone();
    let queue = service
        .retry(&upload_id)
        .await
        .map_err(|error| error.to_string())?;
    let id = upload_id.clone();
    tokio::spawn(async move {
        let _admission = admission;
        if let Err(error) = service.start(&id).await {
            tracing::warn!(%error, upload = %id, "upload retry start failed");
        }
    });
    Ok(queue)
}

#[tauri::command]
pub async fn upload_queue(app: tauri::AppHandle) -> UploadQueueV1 {
    app.state::<Arc<UploadService>>().queue().await
}
