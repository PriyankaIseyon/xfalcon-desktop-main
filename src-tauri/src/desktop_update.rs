use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::StreamExt as _;
use minisign_verify::{PublicKey, Signature};
use reqwest::{redirect, Url};
use sha2::{Digest as _, Sha256};
use tauri::{Emitter as _, Manager as _};
use tauri_plugin_updater::{Update, UpdaterExt as _};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::Mutex;
use xfalcon_contracts::update::{UpdateCheckResult, UpdateDownloadProgress};

use crate::app::commands::AppRuntimeState;
use crate::audit::{emit, AuditAction, AuditResourceType};
use crate::uploads::service::UploadService;
use crate::work_barrier::WorkBarrier;

pub const UPDATE_PROGRESS_EVENT: &str = "xfalcon://update-download-progress";
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RELEASE_NOTES_CHARS: usize = 32 * 1024;
const UPDATE_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDRDQkMzMzA4RkE2RkU1NTAKUldSUTVXLzZDRE84VElXSVA2OWJLT2N5d1hWVThtaElNQjkrV3VQTUY5NkVxYWFpd0JINU92czUK";

pub struct DesktopUpdateService {
    operation: Mutex<()>,
    pending: Mutex<Option<Update>>,
    installed: AtomicBool,
    staged: Mutex<Option<PathBuf>>,
}

struct UnblockOnDrop {
    barrier: Arc<WorkBarrier>,
    armed: bool,
}

impl UnblockOnDrop {
    fn persist_block(&mut self) {
        self.armed = false;
    }
}

impl Drop for UnblockOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.barrier.unblock();
        }
    }
}

impl Default for DesktopUpdateService {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
            pending: Mutex::new(None),
            installed: AtomicBool::new(false),
            staged: Mutex::new(None),
        }
    }
}

impl DesktopUpdateService {
    async fn check(&self, app: &tauri::AppHandle) -> Result<UpdateCheckResult, String> {
        let _operation = self.operation.lock().await;
        if self.installed.load(Ordering::SeqCst) {
            return Err("Restart xFalcon to finish the installed update.".to_owned());
        }
        // Always read the manifest, even when a package is staged. A staged
        // package is reused below only when its signature is still the one the
        // manifest names. A newer release replaces it, so the app never installs
        // an older version first and a newer one after it.
        let updater = app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            .build()
            .map_err(safe_updater_error)?;
        let update = updater.check().await.map_err(safe_updater_error)?;
        let current_version = app.package_info().version.to_string();
        let Some(update) = update else {
            prune_staged(&staging_directory(app)?, None).await?;
            *self.pending.lock().await = None;
            *self.staged.lock().await = None;
            return Ok(UpdateCheckResult::new(current_version, None, None, false));
        };
        validate_artifact_url(&update.download_url)?;
        let release_notes = update.body.as_deref().map(bounded_release_notes);
        let available_version = update.version.clone();
        let path = staged_path(app, &update.signature)?;
        prune_staged(
            path.parent().expect("staged path has a directory"),
            Some(&path),
        )
        .await?;
        let is_staged = read_staged(&path, &update.signature).await.is_ok();
        *self.staged.lock().await = is_staged.then_some(path);
        *self.pending.lock().await = Some(update);
        Ok(UpdateCheckResult::new(
            current_version,
            Some(available_version),
            release_notes,
            is_staged,
        ))
    }

    // This command prepares an update; applying it is explicitly deferred to restart.
    async fn install(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let _operation = self.operation.lock().await;
        let update = self
            .pending
            .lock()
            .await
            .clone()
            .ok_or_else(|| "No checked update is available.".to_owned())?;
        let path = staged_path(app, &update.signature)?;
        prune_staged(
            path.parent().expect("staged path has a directory"),
            Some(&path),
        )
        .await?;
        prepare_staged(&path, &update.signature, download_bounded(app, &update)).await?;
        *self.staged.lock().await = Some(path);
        Ok(())
    }

    async fn restart(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let _operation = self.operation.lock().await;
        if self.installed.load(Ordering::SeqCst) && app.state::<Arc<WorkBarrier>>().is_blocked() {
            app.restart();
        }
        let update = self
            .pending
            .lock()
            .await
            .clone()
            .ok_or_else(|| "No checked update is available.".to_owned())?;
        let path = self
            .staged
            .lock()
            .await
            .clone()
            .ok_or_else(|| "Download the update before restarting.".to_owned())?;
        // Read and verify before acquiring the work barrier. A cache mutation must
        // never reach the installer, and ordinary work remains available meanwhile.
        let bytes = read_staged(&path, &update.signature).await?;
        let barrier = app.state::<Arc<WorkBarrier>>().inner().clone();
        let runtime = app.state::<AppRuntimeState>();
        let uploads = app.state::<Arc<UploadService>>();
        let mut unblock_on_drop = guard_install(barrier, async {
            runtime.chat().has_active_work().await || uploads.has_active_work().await
        })
        .await?;

        // Windows launches its installer and exits inside install(). On macOS it
        // replaces the bundle and returns, after which we explicitly relaunch.
        // Keep the barrier guard in the blocking task so cancellation of the IPC
        // future cannot admit work while the installer is still running.
        let restart_app = app.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            #[cfg(target_os = "macos")]
            preflight_macos_install()?;
            update.install(bytes).map_err(safe_updater_error)?;
            restart_app
                .state::<Arc<DesktopUpdateService>>()
                .installed
                .store(true, Ordering::SeqCst);
            unblock_on_drop.persist_block();
            restart_app.restart();
        })
        .await
        .map_err(|_| "The update installer stopped unexpectedly.".to_owned())?
    }
}

#[tauri::command]
pub async fn update_check(app: tauri::AppHandle) -> Result<UpdateCheckResult, String> {
    app.state::<Arc<DesktopUpdateService>>().check(&app).await
}

#[tauri::command]
pub async fn update_install(app: tauri::AppHandle) -> Result<(), String> {
    let result = app.state::<Arc<DesktopUpdateService>>().install(&app).await;
    emit::record(
        &app,
        AuditAction::UpdateInstall,
        AuditResourceType::Update,
        "desktop",
        emit::outcome_for(&result),
    );
    result
}

#[tauri::command]
pub async fn update_restart(app: tauri::AppHandle) -> Result<(), String> {
    app.state::<Arc<DesktopUpdateService>>().restart(&app).await
}

#[cfg(target_os = "macos")]
fn preflight_macos_install() -> Result<(), String> {
    let executable = tauri::utils::platform::current_exe()
        .map_err(|_| "The running application location is unavailable.".to_owned())?;
    let bundle = tauri_plugin_updater::extract_path_from_executable(&executable)
        .map_err(|_| "The running application bundle could not be located.".to_owned())?;
    preflight_bundle_location(&bundle, &std::env::temp_dir())
}

// The pinned plugin stages both backup and replacement in the system temp
// directory. Reject unsafe locations before it moves the running bundle.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn preflight_bundle_location(bundle: &Path, temporary: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;
    let guidance = "Move xFalcon to a writable Applications folder on the system disk and relaunch it, or install the new release manually.";
    if bundle
        .extension()
        .is_none_or(|extension| extension != "app")
        || !bundle.is_dir()
    {
        return Err(format!(
            "The running app is not an installed macOS bundle. {guidance}"
        ));
    }
    let parent = bundle.parent().ok_or_else(|| guidance.to_owned())?;
    let inspect = |path: &Path| {
        std::fs::metadata(path)
            .map_err(|_| format!("The update location could not be inspected. {guidance}"))
    };
    let bundle_metadata = inspect(bundle)?;
    let parent_metadata = inspect(parent)?;
    let temporary_metadata = inspect(temporary)?;
    if bundle_metadata.dev() != temporary_metadata.dev()
        || parent_metadata.dev() != temporary_metadata.dev()
    {
        return Err(format!(
            "This app and the update temporary folder are on different volumes. {guidance}"
        ));
    }
    if parent_metadata.permissions().readonly() || parent_metadata.mode() & 0o1000 != 0 {
        return Err(format!(
            "The application folder does not allow safe replacement. {guidance}"
        ));
    }
    let probe = tempfile::Builder::new()
        .prefix(".xfalcon-update-check-")
        .tempdir_in(parent)
        .map_err(|_| format!("The application folder is not writable. {guidance}"))?;
    probe
        .close()
        .map_err(|_| format!("The update location check could not be cleaned up. {guidance}"))?;
    Ok(())
}

async fn guard_install(
    barrier: Arc<WorkBarrier>,
    active_work: impl std::future::Future<Output = bool>,
) -> Result<UnblockOnDrop, String> {
    let update_guard = barrier.begin_update().await;
    let unblock_on_drop = UnblockOnDrop {
        barrier: barrier.clone(),
        armed: true,
    };
    if active_work.await {
        return Err("Finish the active run or transfer before restarting to update.".to_owned());
    }
    drop(update_guard);
    Ok(unblock_on_drop)
}

fn staging_directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|path| path.join("desktop-updates"))
        .map_err(|_| "The update cache directory is unavailable.".to_owned())
}

fn staged_path(app: &tauri::AppHandle, signature: &str) -> Result<PathBuf, String> {
    Ok(staging_directory(app)?.join(format!(
        "{:x}.package",
        Sha256::digest(signature.as_bytes())
    )))
}

async fn prune_staged(directory: &Path, keep: Option<&Path>) -> Result<(), String> {
    let mut entries = match tokio::fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("The update cache could not be inspected.".to_owned()),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|_| "The update cache could not be inspected.".to_owned())?
    {
        let path = entry.path();
        if Some(path.as_path()) == keep {
            continue;
        }
        if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("package" | "part")
        ) {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|_| "An obsolete update download could not be removed.".to_owned())?;
        }
    }
    Ok(())
}

async fn prepare_staged(
    path: &Path,
    signature: &str,
    download: impl std::future::Future<Output = Result<Vec<u8>, String>>,
) -> Result<(), String> {
    if read_staged(path, signature).await.is_ok() {
        return Ok(());
    }
    let bytes = download.await?;
    verify_signature(&bytes, signature)?;
    save_staged(path, &bytes).await
}

async fn read_staged(path: &Path, signature: &str) -> Result<Vec<u8>, String> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| "The prepared update is unavailable. Download it again.".to_owned())?;
    let mut bytes = Vec::new();
    // Bound the read itself, including a file that grows after metadata is read.
    let mut limited = tokio::io::AsyncReadExt::take(file, MAX_UPDATE_BYTES + 1);
    tokio::io::AsyncReadExt::read_to_end(&mut limited, &mut bytes)
        .await
        .map_err(|_| "The prepared update could not be read.".to_owned())?;
    if bytes.len() as u64 > MAX_UPDATE_BYTES {
        return Err("The update package exceeds the 512 MiB safety limit.".to_owned());
    }
    verify_signature(&bytes, signature)?;
    Ok(bytes)
}

async fn save_staged(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or_else(|| "Invalid update cache path.".to_owned())?;
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|_| "The update cache could not be created.".to_owned())?;
    let temporary = directory.join(format!("{}.part", uuid::Uuid::new_v4()));
    let result = async {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        tokio::fs::rename(&temporary, path).await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, "could not stage desktop update");
        match tokio::fs::remove_file(&temporary).await {
            Ok(()) => {}
            Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => {}
            Err(cleanup) => tracing::warn!(%cleanup, "could not remove partial update"),
        }
        return Err("The update could not be saved. Check free disk space and retry.".to_owned());
    }
    Ok(())
}

async fn download_bounded(app: &tauri::AppHandle, update: &Update) -> Result<Vec<u8>, String> {
    validate_artifact_url(&update.download_url)?;
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        .redirect(redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("too many update redirects");
            }
            if artifact_url_is_allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("update redirect left the allowed delivery hosts")
            }
        }))
        .build()
        .map_err(|_| "The bounded update client could not start.".to_owned())?;
    let response = client
        .get(update.download_url.clone())
        .send()
        .await
        .map_err(|_| "The update download failed.".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "The update server returned HTTP {}.",
            response.status()
        ));
    }
    collect_download(response, |progress| {
        app.emit(UPDATE_PROGRESS_EVENT, progress)
            .map_err(|_| "Update progress could not be reported.".to_owned())
    })
    .await
}

async fn collect_download(
    response: reqwest::Response,
    mut report: impl FnMut(UpdateDownloadProgress) -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let total = response.content_length();
    if total.is_some_and(|length| length > MAX_UPDATE_BYTES) {
        return Err("The update package exceeds the 512 MiB safety limit.".to_owned());
    }
    let total_u32 = total.and_then(|length| u32::try_from(length).ok());
    let initial_capacity = total.unwrap_or(0).min(8 * 1024 * 1024) as usize;
    let mut bytes = Vec::with_capacity(initial_capacity);
    let mut stream = response.bytes_stream();
    let mut last_report = Instant::now();
    let mut last_percent = Some(0);
    let mut last_bytes = None;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "The update download was interrupted.".to_owned())?;
        append_bounded(&mut bytes, &chunk)?;
        let downloaded = u32::try_from(bytes.len())
            .map_err(|_| "The update package is too large.".to_owned())?;
        let progress = UpdateDownloadProgress::new(downloaded, total_u32);
        if progress
            .percent
            .is_some_and(|percent| Some(percent) != last_percent)
            || last_report.elapsed() >= Duration::from_millis(250)
        {
            last_percent = progress.percent;
            last_bytes = Some(downloaded);
            last_report = Instant::now();
            report(progress)?;
        }
    }
    let downloaded =
        u32::try_from(bytes.len()).map_err(|_| "The update package is too large.".to_owned())?;
    if last_bytes != Some(downloaded) {
        report(UpdateDownloadProgress::new(downloaded, total_u32))?;
    }
    Ok(bytes)
}

fn append_bounded(buffer: &mut Vec<u8>, chunk: &[u8]) -> Result<(), String> {
    append_bounded_with_limit(buffer, chunk, MAX_UPDATE_BYTES)
}

fn append_bounded_with_limit(
    buffer: &mut Vec<u8>,
    chunk: &[u8],
    maximum_bytes: u64,
) -> Result<(), String> {
    let next_len = buffer.len() as u64 + chunk.len() as u64;
    if next_len > maximum_bytes {
        return Err("The update package exceeds the 512 MiB safety limit.".to_owned());
    }
    buffer.extend_from_slice(chunk);
    Ok(())
}

fn verify_signature(bytes: &[u8], encoded_signature: &str) -> Result<(), String> {
    let public_key_text = decode_base64_text(UPDATE_PUBLIC_KEY, "public key")?;
    let signature_text = decode_base64_text(encoded_signature, "signature")?;
    let public_key = PublicKey::decode(&public_key_text)
        .map_err(|_| "The embedded update public key is invalid.".to_owned())?;
    let signature = Signature::decode(&signature_text)
        .map_err(|_| "The update signature is invalid.".to_owned())?;
    public_key
        .verify(bytes, &signature, true)
        .map_err(|_| "The update package signature did not verify.".to_owned())
}

fn decode_base64_text(value: &str, label: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| format!("The update {label} encoding is invalid."))?;
    String::from_utf8(bytes).map_err(|_| format!("The update {label} is not UTF-8."))
}

fn validate_artifact_url(url: &Url) -> Result<(), String> {
    if artifact_url_is_allowed(url) {
        Ok(())
    } else {
        Err("The update manifest named an untrusted artifact location.".to_owned())
    }
}

fn artifact_url_is_allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.port(), None | Some(443))
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "objects.githubusercontent.com"
                    | "release-assets.githubusercontent.com"
                    | "d3rdmiksm582v0.cloudfront.net"
            )
        )
}

fn bounded_release_notes(notes: &str) -> String {
    notes.chars().take(MAX_RELEASE_NOTES_CHARS).collect()
}

fn safe_updater_error(error: impl std::fmt::Display) -> String {
    tracing::warn!(%error, "desktop update operation failed");
    "The signed update operation failed.".to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        append_bounded_with_limit, artifact_url_is_allowed, bounded_release_notes,
        verify_signature, MAX_RELEASE_NOTES_CHARS,
    };
    use crate::work_barrier::WorkBarrier;

    #[cfg(unix)]
    #[test]
    fn bundle_preflight_preserves_app_and_rejects_unsafe_locations() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let root = tempfile::tempdir().unwrap();
        let bundle = root.path().join("xFalcon.app");
        std::fs::create_dir(&bundle).unwrap();
        let original = bundle.join("original");
        std::fs::write(&original, b"installed app").unwrap();
        super::preflight_bundle_location(&bundle, root.path()).unwrap();
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        assert!(super::preflight_bundle_location(&bundle, root.path())
            .unwrap_err()
            .contains("safe replacement"));
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        // Linux CI supplies a separate tmpfs volume; macOS uses the same guard.
        let other_volume = std::path::Path::new("/dev/shm");
        if other_volume.exists()
            && other_volume.metadata().unwrap().dev() != root.path().metadata().unwrap().dev()
        {
            assert!(super::preflight_bundle_location(&bundle, other_volume)
                .unwrap_err()
                .contains("different volumes"));
        }
        assert_eq!(std::fs::read(&original).unwrap(), b"installed app");
        assert!(super::preflight_bundle_location(root.path(), root.path()).is_err());
    }

    #[tokio::test]
    async fn download_consumer_throttles_chunks_and_reports_final_bytes() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        for known_length in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut chunk = [0; 1024];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert!(read > 0, "client closed before sending its request");
                    request.extend_from_slice(&chunk[..read]);
                }
                let length = if known_length {
                    "Content-Length: 1048576\r\n"
                } else {
                    ""
                };
                socket
                    .write_all(
                        format!("HTTP/1.0 200 OK\r\n{length}Connection: close\r\n\r\n").as_bytes(),
                    )
                    .await
                    .unwrap();
                for _ in 0..1024 {
                    socket.write_all(&[42; 1024]).await.unwrap();
                    tokio::task::yield_now().await;
                }
            });
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}"))
                .send()
                .await
                .unwrap();
            let started = std::time::Instant::now();
            let mut events = Vec::new();
            let bytes = super::collect_download(response, |event| {
                events.push(event);
                Ok(())
            })
            .await
            .unwrap();
            server.await.unwrap();
            assert_eq!(bytes, vec![42; 1024 * 1024]);
            assert_eq!(events.last().unwrap().downloaded_bytes, 1024 * 1024);
            assert_eq!(events.last().unwrap().percent, known_length.then_some(100));
            let timed_events = started.elapsed().as_millis() / 250;
            assert!(events.len() as u128 <= if known_length { 101 } else { 1 } + timed_events);
            assert!(events
                .windows(2)
                .all(|pair| pair[0].downloaded_bytes < pair[1].downloaded_bytes));
        }
    }

    #[test]
    fn artifact_urls_are_https_and_delivery_host_bounded() {
        assert!(artifact_url_is_allowed(
            &"https://github.com/org/repo/releases/download/v1/app.tar.gz"
                .parse()
                .unwrap()
        ));
        assert!(artifact_url_is_allowed(
            &"https://release-assets.githubusercontent.com/file"
                .parse()
                .unwrap()
        ));
        assert!(artifact_url_is_allowed(
            &"https://d3rdmiksm582v0.cloudfront.net/desktop/releases/0.5.5/app.exe"
                .parse()
                .unwrap()
        ));
        assert!(!artifact_url_is_allowed(
            &"https://different.cloudfront.net/file".parse().unwrap()
        ));
        assert!(!artifact_url_is_allowed(
            &"https://d3rdmiksm582v0.cloudfront.net.evil.example/file"
                .parse()
                .unwrap()
        ));
        assert!(!artifact_url_is_allowed(
            &"http://github.com/file".parse().unwrap()
        ));
        assert!(!artifact_url_is_allowed(
            &"https://github.com.evil.example/file".parse().unwrap()
        ));
        assert!(!artifact_url_is_allowed(
            &"https://127.0.0.1/file".parse().unwrap()
        ));
    }

    #[test]
    fn release_notes_are_bounded_before_they_cross_ipc() {
        let notes = "x".repeat(MAX_RELEASE_NOTES_CHARS + 1);
        assert_eq!(
            bounded_release_notes(&notes).chars().count(),
            MAX_RELEASE_NOTES_CHARS
        );
    }

    #[test]
    fn package_bound_is_enforced_before_a_chunk_is_appended() {
        let mut bytes = vec![0; 8];
        assert!(append_bounded_with_limit(&mut bytes, &[1], 8).is_err());
        assert_eq!(bytes.len(), 8);
    }

    #[test]
    fn malformed_signatures_fail_closed() {
        assert!(verify_signature(b"package", "not-base64").is_err());
    }

    #[tokio::test]
    async fn staged_package_is_complete_private_and_replaceable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cache/package");
        super::save_staged(&path, b"first download").await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"first download");
        super::save_staged(&path, b"replacement").await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"replacement");
        let mut entries = tokio::fs::read_dir(path.parent().unwrap()).await.unwrap();
        assert!(entries.next_entry().await.unwrap().is_some());
        assert!(entries.next_entry().await.unwrap().is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                tokio::fs::metadata(&path)
                    .await
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(super::read_staged(&path, "invalid signature")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn failed_staging_preserves_existing_package_and_removes_partial() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("package");
        tokio::fs::create_dir(&path).await.unwrap();
        tokio::fs::write(path.join("existing"), b"keep")
            .await
            .unwrap();
        assert!(super::save_staged(&path, b"replacement").await.is_err());
        assert_eq!(
            tokio::fs::read(path.join("existing")).await.unwrap(),
            b"keep"
        );
        let mut entries = tokio::fs::read_dir(directory.path()).await.unwrap();
        assert!(entries.next_entry().await.unwrap().is_some());
        assert!(entries.next_entry().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn corrupted_cache_retries_download_and_never_stages_an_unverified_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("current.package");
        tokio::fs::write(&path, b"corrupt cached bytes")
            .await
            .unwrap();
        let mut downloaded = false;
        let result = super::prepare_staged(&path, "invalid signature", async {
            downloaded = true;
            Ok(b"unverified replacement".to_vec())
        })
        .await;
        assert!(downloaded);
        assert!(result.is_err());
        assert_eq!(
            tokio::fs::read(&path).await.unwrap(),
            b"corrupt cached bytes"
        );
    }

    #[tokio::test]
    async fn cache_cleanup_keeps_only_current_package_and_removes_partial_downloads() {
        let directory = tempfile::tempdir().unwrap();
        let current = directory.path().join("current.package");
        for name in [
            "current.package",
            "previous.package",
            "interrupted.part",
            "unrelated",
        ] {
            tokio::fs::write(directory.path().join(name), b"bytes")
                .await
                .unwrap();
        }
        super::prune_staged(directory.path(), Some(&current))
            .await
            .unwrap();
        assert!(current.exists());
        assert!(!directory.path().join("previous.package").exists());
        assert!(!directory.path().join("interrupted.part").exists());
        assert!(directory.path().join("unrelated").exists());
        super::prune_staged(directory.path(), None).await.unwrap();
        assert!(!current.exists());
    }

    #[tokio::test]
    async fn refusing_apply_during_active_work_reopens_admission() {
        let barrier = Arc::new(WorkBarrier::default());
        let result = super::guard_install(barrier.clone(), async { true }).await;
        assert!(result.is_err());
        assert!(barrier.admit().await.is_ok());
    }

    #[tokio::test]
    async fn failed_apply_reopens_admission_after_guarded_installer_scope() {
        let barrier = Arc::new(WorkBarrier::default());
        let guard = super::guard_install(barrier.clone(), async { false })
            .await
            .unwrap();
        assert!(barrier.admit().await.is_err());
        let result = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            Err::<(), _>("installer failed")
        })
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(barrier.admit().await.is_ok());
    }

    #[tokio::test]
    async fn cancelled_install_scope_reopens_work_admission() {
        let barrier = Arc::new(WorkBarrier::default());
        let update = barrier.begin_update().await;
        let reset = super::UnblockOnDrop {
            barrier: barrier.clone(),
            armed: true,
        };
        drop(update);
        drop(reset);
        assert!(barrier.admit().await.is_ok());
    }
}
