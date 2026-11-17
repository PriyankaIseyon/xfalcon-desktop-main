//! Best-effort cleanup for app-owned, process-temporary data.

use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use uuid::Uuid;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
struct RegistryState {
    roots: HashSet<PathBuf>,
    entries: HashMap<PathBuf, EntryState>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum EntryState {
    Leased(usize),
    Cleaning,
}

struct CleanupInner {
    registry: Mutex<RegistryState>,
}

/// Coordinates temporary writers with periodic cleanup.
#[derive(Clone)]
pub struct TempCleanup {
    inner: Arc<CleanupInner>,
}

/// Protects one UUID-named directory for the complete write and transfer life.
pub struct TempLease {
    inner: Arc<CleanupInner>,
    root: PathBuf,
    directory: PathBuf,
}

impl TempCleanup {
    #[must_use]
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let roots = roots.into_iter().collect();
        Self {
            inner: Arc::new(CleanupInner {
                registry: Mutex::new(RegistryState {
                    roots,
                    entries: HashMap::new(),
                }),
            }),
        }
    }

    /// Adds a root before it is used by a temporary writer.
    pub fn add_root(&self, root: PathBuf) {
        self.inner
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .roots
            .insert(root);
    }

    /// Reserves a UUID directory before any writer creates it.
    pub fn lease_directory(&self, root: &Path, id: Uuid) -> io::Result<TempLease> {
        let directory = root.join(id.to_string());
        // This lock protects only a few map operations. Do not make a foreground
        // writer fail because a background pass briefly read the registry.
        let mut state = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.roots.contains(root) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "temporary directory is outside registered roots",
            ));
        }
        match state.entries.get_mut(&directory) {
            Some(EntryState::Cleaning) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "temporary cleanup owns this directory",
                ));
            }
            Some(EntryState::Leased(count)) => *count += 1,
            None => {
                state
                    .entries
                    .insert(directory.clone(), EntryState::Leased(1));
            }
        }
        Ok(TempLease {
            inner: Arc::clone(&self.inner),
            root: root.to_path_buf(),
            directory,
        })
    }

    /// Runs one nonblocking filesystem pass. Each child is isolated from its siblings.
    pub fn sweep_pass(&self) -> usize {
        let roots = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .roots
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut removed = 0;
        for root in roots {
            let Ok(metadata) = fs::symlink_metadata(&root) else {
                continue;
            };
            if !safe_std_directory(&metadata) {
                continue;
            }
            let Ok(root_dir) = open_cleanup_root(&root) else {
                continue;
            };
            let Ok(entries) = root_dir.read_dir(".") else {
                continue;
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    continue;
                };
                let name = entry.file_name();
                let directory = root.join(&name);
                let is_uuid = directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| Uuid::parse_str(name).ok())
                    .is_some();
                if !is_uuid || !safe_owned_tree(&root_dir, &name) || !self.claim(&directory) {
                    continue;
                }
                // Resolve deletion through an open root capability. A raced
                // child cannot redirect the recursive walk outside this root.
                let result = root_dir.remove_dir_all(&name);
                self.release_cleaning(&directory);
                match result {
                    Ok(()) => removed += 1,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => tracing::debug!(
                        error_kind = ?error.kind(),
                        "temporary cleanup retained an entry"
                    ),
                }
            }
        }
        removed
    }

    fn claim(&self, directory: &Path) -> bool {
        let mut state = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.entries.contains_key(directory) {
            return false;
        }
        state
            .entries
            .insert(directory.to_path_buf(), EntryState::Cleaning);
        true
    }

    fn release_cleaning(&self, directory: &Path) {
        let mut state = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.entries.get(directory) == Some(&EntryState::Cleaning) {
            state.entries.remove(directory);
        }
    }

    /// Starts the delayed-first-tick worker on Tauri's runtime.
    pub fn start_background(&self) {
        let cleanup = self.clone();
        tauri::async_runtime::spawn(async move {
            run_periodic(
                CLEANUP_INTERVAL,
                CLEANUP_INTERVAL,
                move || {
                    let cleanup = cleanup.clone();
                    tokio::task::spawn_blocking(move || {
                        cleanup.sweep_pass();
                    })
                },
                tokio::time::sleep,
            )
            .await;
        });
    }
}

impl TempLease {
    /// Retains the lease while an asynchronous transfer is using the file.
    pub fn try_clone(&self) -> io::Result<Self> {
        let mut state = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match state.entries.get_mut(&self.directory) {
            Some(EntryState::Leased(count)) => {
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("temporary lease count overflow"))?;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "temporary directory lease is no longer active",
                ));
            }
        }
        Ok(Self {
            inner: Arc::clone(&self.inner),
            root: self.root.clone(),
            directory: self.directory.clone(),
        })
    }

    /// Removes the one file and only removes its directory when empty.
    pub async fn remove_file(&self, path: &Path) {
        if path.parent() != Some(self.directory.as_path())
            || self.directory.parent() != Some(self.root.as_path())
            || !is_uuid_directory(&self.directory)
        {
            tracing::warn!("temporary cleanup refused an unowned path");
            return;
        }
        {
            let state = self
                .inner
                .registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(state.entries.get(&self.directory), Some(EntryState::Leased(count)) if *count > 1)
            {
                tracing::debug!("temporary file cleanup deferred while an operation is active");
                return;
            }
        }
        let root = self.root.clone();
        let directory_name = self.directory.file_name().map(std::ffi::OsStr::to_owned);
        let file_name = path.file_name().map(std::ffi::OsStr::to_owned);
        let (Some(directory_name), Some(file_name)) = (directory_name, file_name) else {
            tracing::warn!("temporary cleanup refused an invalid owned path");
            return;
        };
        let result = tokio::task::spawn_blocking(move || {
            let root = open_cleanup_root(&root)?;
            let directory = Path::new(&directory_name);
            match root.symlink_metadata(directory) {
                Ok(metadata) if safe_cap_directory(&metadata) => {}
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "unsafe directory",
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            }
            let file = directory.join(file_name);
            match root.symlink_metadata(&file) {
                Ok(metadata) if metadata.is_file() && !is_cap_reparse_point(&metadata) => {
                    root.remove_file(&file)?;
                }
                Ok(_) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe file")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            match root.remove_dir(directory) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) if error.kind() == io::ErrorKind::InvalidInput => {
                tracing::warn!("temporary cleanup refused an unsafe entry");
            }
            Ok(Err(error)) => {
                tracing::debug!(error_kind = ?error.kind(), "temporary file cleanup will retry")
            }
            Err(_) => tracing::debug!("temporary file cleanup worker failed; cleanup will retry"),
        }
    }
}

impl Drop for TempLease {
    fn drop(&mut self) {
        let mut state = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match state.entries.get_mut(&self.directory) {
            Some(EntryState::Leased(count)) if *count > 1 => *count -= 1,
            Some(EntryState::Leased(_)) => {
                state.entries.remove(&self.directory);
            }
            _ => {}
        }
    }
}

fn safe_owned_tree(root: &cap_std::fs::Dir, name: &std::ffi::OsStr) -> bool {
    let Ok(metadata) = root.symlink_metadata(Path::new(name)) else {
        return false;
    };
    if !safe_cap_directory(&metadata) {
        return false;
    }
    let Ok(directory) = root.open_dir(Path::new(name)) else {
        return false;
    };
    let Ok(entries) = directory.read_dir(".") else {
        return false;
    };
    entries.into_iter().all(|entry| {
        let Ok(entry) = entry else {
            return false;
        };
        let Ok(metadata) = directory.symlink_metadata(entry.file_name()) else {
            return false;
        };
        metadata.is_file() && !is_cap_reparse_point(&metadata)
    })
}

fn safe_std_directory(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_dir() && !is_reparse_point(metadata)
}

fn safe_cap_directory(metadata: &cap_std::fs::Metadata) -> bool {
    metadata.is_dir() && !is_cap_reparse_point(metadata)
}

fn is_uuid_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| Uuid::parse_str(name).ok())
        .is_some()
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn is_cap_reparse_point(metadata: &cap_std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

#[cfg(unix)]
fn open_cleanup_root(path: &Path) -> io::Result<cap_std::fs::Dir> {
    use rustix::fs::{Mode, OFlags};

    // Refuse a root replaced with a symlink at open time. The descriptor then
    // anchors all child inspection and deletion for this pass.
    let descriptor = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io::Error::from)?;
    Ok(cap_std::fs::Dir::from_std_file(std::fs::File::from(
        descriptor,
    )))
}

#[cfg(windows)]
fn open_cleanup_root(path: &Path) -> io::Result<cap_std::fs::Dir> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let metadata = file.metadata()?;
    if !safe_std_directory(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe cleanup root",
        ));
    }
    Ok(cap_std::fs::Dir::from_std_file(file))
}

#[cfg(not(any(unix, windows)))]
fn open_cleanup_root(path: &Path) -> io::Result<cap_std::fs::Dir> {
    cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority())
}

async fn run_periodic(
    first_delay: Duration,
    interval: Duration,
    mut task: impl FnMut() -> tokio::task::JoinHandle<()>,
    mut sleep: impl FnMut(Duration) -> tokio::time::Sleep,
) {
    sleep(first_delay).await;
    loop {
        match task().await {
            Ok(()) => {}
            Err(error) if error.is_panic() => {
                tracing::warn!("temporary cleanup pass panicked; the next pass will retry")
            }
            Err(_) => tracing::debug!("temporary cleanup worker was cancelled"),
        }
        sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const FIRST_ID: &str = "11111111-1111-4111-8111-111111111111";
    const SECOND_ID: &str = "22222222-2222-4222-8222-222222222222";

    #[test]
    fn sweep_removes_only_abandoned_uuid_files_and_keeps_other_entries() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("staging");
        fs::create_dir_all(&root).expect("root");
        let abandoned = root.join(FIRST_ID);
        fs::create_dir_all(&abandoned).expect("owned directory");
        fs::write(abandoned.join("sales_cleaned.csv"), b"private").expect("file");
        let unrelated = root.join("notes");
        fs::create_dir_all(&unrelated).expect("unrelated directory");
        fs::write(unrelated.join("keep"), b"keep").expect("unrelated file");
        let cleanup = TempCleanup::new([root]);

        assert_eq!(cleanup.sweep_pass(), 1);
        assert!(!abandoned.exists());
        assert!(unrelated.join("keep").exists());
    }

    #[test]
    fn lease_protects_a_directory_until_transfer_and_retry_release_it() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("upload-snapshots");
        fs::create_dir_all(&root).expect("root");
        let cleanup = TempCleanup::new([root.clone()]);
        let lease = cleanup
            .lease_directory(&root, Uuid::parse_str(FIRST_ID).unwrap())
            .expect("lease before creation");
        let directory = root.join(FIRST_ID);
        fs::create_dir(&directory).expect("directory");
        let file = directory.join("attachment.txt");
        fs::write(&file, b"retry data").expect("file");

        assert_eq!(cleanup.sweep_pass(), 0);
        assert!(file.exists());
        drop(lease);
        assert_eq!(cleanup.sweep_pass(), 1);
        assert!(!directory.exists());
    }

    #[test]
    fn one_invalid_tree_does_not_stop_later_cleanup_targets() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("staging");
        let linked = root.join(FIRST_ID);
        let abandoned = root.join(SECOND_ID);
        fs::create_dir_all(&linked).expect("linked directory");
        fs::create_dir_all(&abandoned).expect("abandoned directory");
        fs::write(abandoned.join("result_cleaned.csv"), b"data").expect("file");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).expect("outside");
        fs::write(outside.join("keep"), b"keep").expect("outside file");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, linked.join("link")).expect("symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(outside.join("keep"), linked.join("link"))
            .expect("symlink");
        let cleanup = TempCleanup::new([root]);

        assert_eq!(cleanup.sweep_pass(), 1);
        assert!(linked.exists());
        assert!(outside.join("keep").exists());
        assert!(!abandoned.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_root_is_never_swept() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("temp");
        let outside = temp.path().join("outside");
        let root_link = temp.path().join("staging");
        let abandoned = outside.join(FIRST_ID);
        fs::create_dir_all(&abandoned).expect("outside UUID directory");
        fs::write(abandoned.join("result.csv"), b"data").expect("outside file");
        symlink(&outside, &root_link).expect("linked root");
        let cleanup = TempCleanup::new([root_link]);

        assert_eq!(cleanup.sweep_pass(), 0);
        assert!(abandoned.join("result.csv").exists());
    }

    #[cfg(windows)]
    #[test]
    fn an_in_use_file_is_retained_and_removed_on_a_later_pass() {
        use std::os::windows::fs::OpenOptionsExt;

        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("staging");
        let directory = root.join(FIRST_ID);
        fs::create_dir_all(&directory).expect("owned directory");
        let file = directory.join("result.csv");
        fs::write(&file, b"data").expect("file");
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .expect("exclusive open");
        let cleanup = TempCleanup::new([root]);

        assert_eq!(cleanup.sweep_pass(), 0);
        assert!(file.exists(), "a failed deletion preserves the target");
        drop(handle);
        assert_eq!(cleanup.sweep_pass(), 1);
        assert!(!directory.exists());
    }

    #[tokio::test(start_paused = true)]
    async fn schedule_delays_first_pass_repeats_and_never_overlaps() {
        let runs = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (sleep_tx, mut sleep_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut started_tx = Some(started_tx);
        let mut release_rx = Some(release_rx);
        let mut sleep_number = 0;
        let sleep = move |duration| {
            let _ = sleep_tx.send(sleep_number);
            sleep_number += 1;
            tokio::time::sleep(duration)
        };
        let task_runs = Arc::clone(&runs);
        let task_active = Arc::clone(&active);
        let task_max = Arc::clone(&max_active);
        let worker = tokio::spawn(run_periodic(
            Duration::from_millis(80),
            Duration::from_millis(80),
            move || {
                let runs = Arc::clone(&task_runs);
                let active = Arc::clone(&task_active);
                let max_active = Arc::clone(&task_max);
                let started_tx = started_tx.take();
                let release_rx = release_rx.take();
                tokio::spawn(async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(current, Ordering::SeqCst);
                    runs.fetch_add(1, Ordering::SeqCst);
                    if let Some(started_tx) = started_tx {
                        let _ = started_tx.send(());
                    }
                    if let Some(release_rx) = release_rx {
                        let _ = release_rx.await;
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                })
            },
            sleep,
        ));
        assert_eq!(
            sleep_rx.recv().await,
            Some(0),
            "initial timer is registered"
        );
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(79)).await;
        tokio::task::yield_now().await;
        assert_eq!(runs.load(Ordering::SeqCst), 0, "first tick is delayed");
        tokio::time::advance(Duration::from_millis(1)).await;
        started_rx.await.expect("first pass starts after the delay");
        tokio::time::advance(Duration::from_millis(400)).await;
        tokio::task::yield_now().await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(active.load(Ordering::SeqCst), 1, "pass is still held open");
        release_tx.send(()).expect("release first pass");
        assert_eq!(
            sleep_rx.recv().await,
            Some(1),
            "interval timer is registered after the pass"
        );
        tokio::time::advance(Duration::from_millis(79)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "the next pass waits a full interval"
        );
        tokio::time::advance(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        worker.abort();

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }
}
