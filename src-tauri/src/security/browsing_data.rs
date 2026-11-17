use std::{
    fmt, fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use uuid::Uuid;

use super::capabilities::is_exact_app_origin;

pub const DEVELOPMENT_MIGRATION_MARKER: &str = "migrations/webview-storage-v1-development";
pub const PACKAGED_MIGRATION_MARKER: &str = "migrations/webview-storage-v1-packaged";
pub const MIGRATION_PAGE: &str = "storage-migration.html";
pub const MAIN_PAGE: &str = "index.html";
pub const ACKNOWLEDGEMENT_TIMEOUT: Duration = Duration::from_secs(30);
const ACK_QUERY: &str = "xfalcon-storage-migration=v1";
const DEVELOPMENT_MARKER_CONTENTS: &[u8] = b"xfalcon-webview-storage-v1-development\n";
const PACKAGED_MARKER_CONTENTS: &[u8] = b"xfalcon-webview-storage-v1-packaged\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageOrigin {
    Development,
    Packaged,
}

impl StorageOrigin {
    pub const fn for_runtime(development: bool) -> Self {
        if development {
            Self::Development
        } else {
            Self::Packaged
        }
    }

    const fn is_development(self) -> bool {
        matches!(self, Self::Development)
    }

    const fn marker(self) -> &'static str {
        match self {
            Self::Development => DEVELOPMENT_MIGRATION_MARKER,
            Self::Packaged => PACKAGED_MIGRATION_MARKER,
        }
    }

    const fn marker_contents(self) -> &'static [u8] {
        match self {
            Self::Development => DEVELOPMENT_MARKER_CONTENTS,
            Self::Packaged => PACKAGED_MARKER_CONTENTS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationMarkerStatus {
    Required,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowsingDataMigrationError;

impl fmt::Display for BrowsingDataMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("webview browsing-data migration failed")
    }
}

impl std::error::Error for BrowsingDataMigrationError {}

pub struct StartupMigration {
    app_local_data: PathBuf,
    origin: StorageOrigin,
    marker_status: MigrationMarkerStatus,
    ready: Arc<AtomicBool>,
}

impl StartupMigration {
    pub fn new(
        app_local_data: PathBuf,
        origin: StorageOrigin,
    ) -> Result<Self, BrowsingDataMigrationError> {
        let marker_status = marker_status(&app_local_data, origin)?;
        Ok(Self {
            app_local_data,
            origin,
            marker_status,
            ready: Arc::new(AtomicBool::new(false)),
        })
    }

    pub const fn initial_page(&self) -> &'static str {
        match self.marker_status {
            MigrationMarkerStatus::Required => MIGRATION_PAGE,
            MigrationMarkerStatus::Complete => MAIN_PAGE,
        }
    }

    pub fn handle_loaded_url(&self, url: &tauri::Url) -> Result<bool, BrowsingDataMigrationError> {
        let recognized = match self.marker_status {
            MigrationMarkerStatus::Required => {
                if !is_exact_storage_migration_ack(url, self.origin) {
                    return Ok(false);
                }
                acknowledge_webview_storage_migration(&self.app_local_data, self.origin)?;
                true
            }
            MigrationMarkerStatus::Complete => is_exact_main_page(url, self.origin),
        };
        Ok(recognized)
    }

    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    pub fn ready_flag(&self) -> Arc<AtomicBool> {
        self.ready.clone()
    }
}

pub fn spawn_acknowledgement_timeout(
    ready: Arc<AtomicBool>,
    timeout: Duration,
    on_timeout: impl FnOnce() + Send + 'static,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("xfalcon-storage-migration-timeout".to_owned())
        .spawn(move || {
            thread::sleep(timeout);
            if !ready.load(Ordering::Acquire) {
                on_timeout();
            }
        })
}

pub fn marker_status(
    app_local_data: &Path,
    origin: StorageOrigin,
) -> Result<MigrationMarkerStatus, BrowsingDataMigrationError> {
    validate_or_create_app_root(app_local_data)?;
    let marker = app_local_data.join(origin.marker());
    match fs::symlink_metadata(&marker) {
        Ok(metadata) if is_link_or_reparse(&metadata) || !metadata.is_file() => {
            Err(BrowsingDataMigrationError)
        }
        Ok(_) => {
            let contents = fs::read(&marker).map_err(|_| BrowsingDataMigrationError)?;
            if contents == origin.marker_contents() {
                Ok(MigrationMarkerStatus::Complete)
            } else {
                Err(BrowsingDataMigrationError)
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(MigrationMarkerStatus::Required)
        }
        Err(_) => Err(BrowsingDataMigrationError),
    }
}

pub fn acknowledge_webview_storage_migration(
    app_local_data: &Path,
    origin: StorageOrigin,
) -> Result<(), BrowsingDataMigrationError> {
    if marker_status(app_local_data, origin)? == MigrationMarkerStatus::Complete {
        return Ok(());
    }

    let marker = app_local_data.join(origin.marker());
    let parent = marker.parent().ok_or(BrowsingDataMigrationError)?;
    fs::create_dir_all(parent).map_err(|_| BrowsingDataMigrationError)?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|_| BrowsingDataMigrationError)?;
    if is_link_or_reparse(&parent_metadata) || !parent_metadata.is_dir() {
        return Err(BrowsingDataMigrationError);
    }

    let temporary = parent.join(format!(".webview-storage-v1-{}.tmp", Uuid::new_v4()));
    let persist_result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(origin.marker_contents())?;
        file.sync_all()?;
        fs::rename(&temporary, &marker)?;
        sync_directory(parent)?;
        Ok::<(), io::Error>(())
    })();
    if persist_result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(BrowsingDataMigrationError);
    }
    Ok(())
}

pub fn is_exact_storage_migration_ack(url: &tauri::Url, origin: StorageOrigin) -> bool {
    is_exact_app_origin(url, origin.is_development())
        && is_app_document_path(url.path())
        && url.query() == Some(ACK_QUERY)
        && url.fragment().is_none()
}

pub fn is_exact_main_page(url: &tauri::Url, origin: StorageOrigin) -> bool {
    if !is_exact_app_origin(url, origin.is_development()) {
        return false;
    }
    if url.query().is_some() || url.fragment().is_some() {
        return false;
    }
    is_app_document_path(url.path())
}

/// The three spellings of the app's own entry document.
///
/// All three are required, in packaged builds as much as in development. A
/// navigation the page itself issues (the storage-migration acknowledgement)
/// keeps the file name, so `/index.html` is needed. The other two are the app
/// root, and which spelling arrives depends on the platform:
///
/// `WebviewUrl::App("index.html")` never reaches the webview as a joined URL.
/// Tauri special-cases exactly that file name and hands the webview its base URL
/// unchanged (tauri 2.11.3 `manager/webview.rs:451`, "ignore `index.html` just to
/// simplify the url"). The base differs per platform
/// (`manager/mod.rs::tauri_protocol_url`):
///
/// | platform | base URL | `url.path()` |
/// |---|---|---|
/// | Windows | `http://tauri.localhost` | `/` — `http` is a special scheme, so the path normalizes |
/// | macOS, Linux | `tauri://localhost` | `""` — `tauri` is not special, so the path stays empty |
///
/// That asymmetry is the whole bug this list exists to prevent. Accepting only
/// `/index.html` broke Windows once the migration marker existed; adding `/`
/// fixed Windows and left macOS broken in the same way, because macOS arrives
/// with an empty path. In both cases the window was never shown and the app
/// exited on its acknowledgement deadline, with no window and no crash — see
/// `spawn_acknowledgement_timeout`.
///
/// Nothing is loosened by accepting the empty path: the origin check above is
/// what constrains which document may be shown, and Tauri itself resolves the
/// empty path to this same document ("if the url is `tauri://localhost`, we
/// should load `index.html`" — `manager/mod.rs::get_asset`).
fn is_app_document_path(path: &str) -> bool {
    matches!(path, "" | "/" | "/index.html")
}

fn validate_or_create_app_root(app_local_data: &Path) -> Result<(), BrowsingDataMigrationError> {
    match fs::symlink_metadata(app_local_data) {
        Ok(metadata) if is_link_or_reparse(&metadata) || !metadata.is_dir() => {
            Err(BrowsingDataMigrationError)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(app_local_data).map_err(|_| BrowsingDataMigrationError)?;
            let metadata =
                fs::symlink_metadata(app_local_data).map_err(|_| BrowsingDataMigrationError)?;
            if is_link_or_reparse(&metadata) || !metadata.is_dir() {
                Err(BrowsingDataMigrationError)
            } else {
                Ok(())
            }
        }
        Err(_) => Err(BrowsingDataMigrationError),
    }
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}
