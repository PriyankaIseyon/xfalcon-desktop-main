//! Platform-specific adapters behind the shared containment API.
//!
//! [`super::domain`] and [`super::path_policy`] never match on
//! `cfg(windows)`/`cfg(unix)` themselves; every OS difference in "what is
//! this root's stable identity" and "is this a link/reparse point" is
//! decided here, mirroring how `crate::skills::discovery` keeps its
//! symlink-escape guard behind a narrow helper rather than scattering
//! platform checks through the module. See
//! `docs/decisions.md` for
//! the dependency and design evidence behind this file.

use std::io;
use std::path::Path;

use cap_std::fs::{Dir, Metadata, Permissions};

/// Stable identity of a trusted root's directory, captured once at trust
/// time ([`root_identity`]) and revalidated before every containment
/// decision. A changed value proves the root was deleted and recreated, or a
/// different volume/filesystem was mounted at the same path, even though the
/// path string itself is unchanged (`docs/threat-model.md`,
/// TM-007).
///
/// `primary`/`secondary` are an opaque 128-bit platform fingerprint. Linux
/// and macOS hash the filesystem identity, inode, and birth time so immediate
/// inode reuse after deletion cannot make a recreated root look unchanged.
/// Windows packs `(volume_serial_number, file_index)` directly via
/// `GetFileInformationByHandle`. A trust record is never portable across
/// platforms, so the encodings do not need a public platform tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootIdentity {
    primary: u64,
    secondary: u64,
}

impl RootIdentity {
    #[must_use]
    pub const fn from_parts(primary: u64, secondary: u64) -> Self {
        Self { primary, secondary }
    }

    #[must_use]
    pub const fn into_parts(self) -> (u64, u64) {
        (self.primary, self.secondary)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("failed to open the trusted root")]
    Open(#[source] io::Error),
    #[error("failed to read the trusted root's stable identity")]
    Identity(#[source] io::Error),
    #[error("this platform has no supported stable root identity")]
    UnsupportedPlatform,
    #[error("failed to flush a directory's metadata to disk")]
    DirectorySync(#[source] io::Error),
}

/// Opens `path` as a capability-scoped directory handle. Every subsequent
/// containment decision for this root goes through the returned [`Dir`]
/// (`super::domain::TrustedRoot::resolve_existing`/`resolve_new`), never a
/// raw absolute-path open: `cap_std::fs::Dir` guarantees (CWE-22) that
/// relative opens through it cannot resolve outside the directory it was
/// opened for, including through a symlink/junction/reparse point planted
/// inside it after the fact.
pub(super) fn open_root(path: &Path) -> Result<Dir, PlatformError> {
    Dir::open_ambient_dir(path, cap_std::ambient_authority()).map_err(PlatformError::Open)
}

pub(super) struct RootObserver {
    // Drop the queue before closing the descriptor registered with it.
    #[cfg(target_os = "macos")]
    deletion_queue: std::os::fd::OwnedFd,
    pub(super) root: Dir,
    #[cfg(target_os = "macos")]
    deletion_observed: bool,
}

/// An observation handle must not prevent the rename/deletion it observes.
pub(super) fn open_observer_root(path: &Path) -> Result<RootObserver, PlatformError> {
    #[cfg(windows)]
    let root = {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE,
        };
        std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .map(Dir::from_std_file)
            .map_err(PlatformError::Open)?
    };
    #[cfg(not(windows))]
    let root = open_root(path)?;
    #[cfg(target_os = "macos")]
    let deletion_queue = watch_unlink(&root)?;
    Ok(RootObserver {
        root,
        #[cfg(target_os = "macos")]
        deletion_queue,
        #[cfg(target_os = "macos")]
        deletion_observed: false,
    })
}

#[cfg(target_os = "macos")]
fn watch_unlink(root: &Dir) -> Result<std::os::fd::OwnedFd, PlatformError> {
    use rustix::event::kqueue::{kevent, kqueue, Event, EventFilter, EventFlags, VnodeEvents};
    use std::os::fd::AsRawFd;
    let queue = kqueue().map_err(|error| PlatformError::Open(error.into()))?;
    rustix::io::fcntl_setfd(&queue, rustix::io::FdFlags::CLOEXEC)
        .map_err(|error| PlatformError::Open(error.into()))?;
    let change = Event::new(
        EventFilter::Vnode {
            vnode: root.as_raw_fd(),
            flags: VnodeEvents::DELETE,
        },
        EventFlags::ADD | EventFlags::ENABLE | EventFlags::CLEAR,
        std::ptr::null_mut(),
    );
    let mut no_events: [Event; 0] = [];
    // SAFETY: RootObserver retains this root descriptor until after queue drops.
    // Registration has no output slots and a zero timeout, so never waits.
    unsafe {
        kevent(
            &queue,
            &[change],
            &mut no_events,
            Some(std::time::Duration::ZERO),
        )
    }
    .map_err(|error| PlatformError::Open(error.into()))?;
    Ok(queue)
}

#[cfg(target_os = "macos")]
fn macos_unlink_observed(observer: &mut RootObserver) -> Result<bool, PlatformError> {
    use rustix::event::kqueue::{kevent, Event, EventFilter, EventFlags, VnodeEvents};
    use std::os::fd::AsRawFd;
    if observer.deletion_observed {
        return Ok(true);
    }
    let mut events = [Event::new(
        EventFilter::Vnode {
            vnode: observer.root.as_raw_fd(),
            flags: VnodeEvents::empty(),
        },
        EventFlags::empty(),
        std::ptr::null_mut(),
    )];
    // SAFETY: the observed descriptor outlives this queue. A zero timeout makes
    // this a nonblocking poll; rustix owns the platform-specific event layout.
    let count = unsafe {
        kevent(
            &observer.deletion_queue,
            &[],
            &mut events,
            Some(std::time::Duration::ZERO),
        )
    }
    .map_err(|error| PlatformError::Identity(error.into()))?;
    if count == 0 {
        return Ok(false);
    }
    let event = events[0];
    if event.flags().contains(EventFlags::ERROR) {
        return Err(PlatformError::Identity(io::Error::from_raw_os_error(
            event.data() as i32,
        )));
    }
    // APFS does not reliably report nlink == 0 for an open removed directory.
    // Latch NOTE_DELETE before persistence: EV_CLEAR consumes the notification,
    // but failed database writes must be able to retry without losing proof.
    observer.deletion_observed = matches!(event.filter(), EventFilter::Vnode { vnode, flags }
        if vnode == observer.root.as_raw_fd() && flags.contains(VnodeEvents::DELETE)
            && !flags.contains(VnodeEvents::REVOKE));
    Ok(observer.deletion_observed)
}

pub(super) enum RootDeletionState {
    NotDeleted,
    #[cfg(unix)]
    Confirmed,
    #[cfg(windows)]
    Pending,
}

/// Linux link counts and macOS vnode events prove unlinking. Windows exposes
/// only reversible pending deletion here. Unsupported platforms retain projects instead of guessing.
pub(super) fn root_deletion_state(
    observer: &mut RootObserver,
) -> Result<RootDeletionState, PlatformError> {
    #[cfg(target_os = "macos")]
    {
        Ok(if macos_unlink_observed(observer)? {
            RootDeletionState::Confirmed
        } else {
            RootDeletionState::NotDeleted
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use cap_std::fs::MetadataExt;
        Ok(
            if observer
                .root
                .dir_metadata()
                .map_err(PlatformError::Identity)?
                .nlink()
                == 0
            {
                RootDeletionState::Confirmed
            } else {
                RootDeletionState::NotDeleted
            },
        )
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileStandardInfo, GetFileInformationByHandleEx, FILE_STANDARD_INFO,
        };
        let mut information = FILE_STANDARD_INFO::default();
        // SAFETY: the directory owns the borrowed handle and the initialized
        // buffer has the exact layout and size required for FileStandardInfo.
        let succeeded = unsafe {
            GetFileInformationByHandleEx(
                observer.root.as_raw_handle(),
                FileStandardInfo,
                std::ptr::from_mut(&mut information).cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        };
        if succeeded == 0 {
            return Err(PlatformError::Identity(io::Error::last_os_error()));
        }
        Ok(if information.Directory && information.DeletePending {
            RootDeletionState::Pending
        } else {
            RootDeletionState::NotDeleted
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = observer;
        Ok(RootDeletionState::NotDeleted)
    }
}

/// Computes `dir`'s current stable identity by asking the OS directly --
/// never derived from the path string, so a renamed-then-recreated root at
/// the same path is caught.
pub(super) fn root_identity(dir: &Dir) -> Result<RootIdentity, PlatformError> {
    #[cfg(unix)]
    {
        unix_identity(dir)
    }
    #[cfg(windows)]
    {
        windows_identity(dir)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        Err(PlatformError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
fn unix_identity(dir: &Dir) -> Result<RootIdentity, PlatformError> {
    #[cfg(not(target_os = "linux"))]
    use cap_std::fs::MetadataExt;

    #[cfg(not(target_os = "linux"))]
    let metadata = dir.dir_metadata().map_err(PlatformError::Identity)?;
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{AtFlags, StatxFlags};

        let stat = rustix::fs::statx(
            dir,
            "",
            AtFlags::EMPTY_PATH | AtFlags::SYMLINK_NOFOLLOW,
            StatxFlags::BASIC_STATS | StatxFlags::BTIME,
        )
        .map_err(|error| PlatformError::Identity(error.into()))?;
        if stat.stx_mask & StatxFlags::BTIME.bits() == 0 {
            return Err(PlatformError::UnsupportedPlatform);
        }
        Ok(fingerprint(
            b"xfalcon.root-identity.linux.v1",
            &[
                u64::from(stat.stx_dev_major),
                u64::from(stat.stx_dev_minor),
                stat.stx_ino,
                stat.stx_btime.tv_sec as u64,
                u64::from(stat.stx_btime.tv_nsec),
            ],
        ))
    }
    #[cfg(target_os = "macos")]
    {
        let created = metadata
            .created()
            .map_err(PlatformError::Identity)?
            .into_std()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| {
                PlatformError::Identity(io::Error::new(io::ErrorKind::InvalidData, error))
            })?;
        Ok(fingerprint(
            b"xfalcon.root-identity.macos.v1",
            &[
                metadata.dev(),
                metadata.ino(),
                created.as_secs(),
                u64::from(created.subsec_nanos()),
            ],
        ))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok(RootIdentity::from_parts(metadata.dev(), metadata.ino()))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fingerprint(domain: &[u8], fields: &[u64]) -> RootIdentity {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    for field in fields {
        hasher.update(&field.to_le_bytes());
    }
    let digest = hasher.finalize();
    let bytes = digest.as_bytes();
    let mut primary = [0; 8];
    let mut secondary = [0; 8];
    primary.copy_from_slice(&bytes[..8]);
    secondary.copy_from_slice(&bytes[8..16]);
    RootIdentity::from_parts(u64::from_le_bytes(primary), u64::from_le_bytes(secondary))
}

#[cfg(windows)]
fn windows_identity(dir: &Dir) -> Result<RootIdentity, PlatformError> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let handle = dir.as_raw_handle() as HANDLE;
    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: `handle` is a valid, open directory handle owned by `dir` and
    // kept alive for the duration of this call (it is borrowed, never
    // closed here); `information` has the exact layout
    // `GetFileInformationByHandle` expects to write into.
    let succeeded = unsafe { GetFileInformationByHandle(handle, information.as_mut_ptr()) };
    if succeeded == 0 {
        return Err(PlatformError::Identity(io::Error::last_os_error()));
    }
    // SAFETY: a nonzero return value means the API fully initialized the
    // output structure.
    let information = unsafe { information.assume_init() };
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok(RootIdentity::from_parts(
        u64::from(information.dwVolumeSerialNumber),
        file_index,
    ))
}

/// Flushes `dir`'s own metadata to disk after a same-directory atomic
/// rename/replace/delete landed inside it (`super::writer`'s atomic-write
/// sequence). This is the durability step for the *rename itself* (the
/// directory entry that now points at the new content), distinct from
/// flushing the file content, which happens separately via
/// `cap_std::fs::File::sync_all` on the temp file before it is ever renamed
/// into place.
///
/// # Platform contract
///
/// - **Unix**: `fsync(2)` on the directory's own file descriptor is the only
///   documented way to make a completed `rename(2)` durable against a power
///   loss/kernel panic immediately afterward (the rename syscall itself is
///   atomic -- another thread/process never observes a half-renamed state --
///   but "atomic" and "durable" are different guarantees; without this fsync,
///   a rename that already returned success could still be rolled back by an
///   unclean shutdown before the containing directory's own metadata is
///   written back). Delegated to [`rustix::fs::openat`] and
///   [`rustix::fs::fsync`], safe, `std::io`-compatible wrappers cap-std's own
///   dependency tree already vets for exactly this class of syscall (see
///   `Cargo.toml`'s dependency comment).
/// - **Windows**: NTFS has no directory-handle analog to Unix's directory
///   fsync that this codebase relies on elsewhere (attempting
///   `FlushFileBuffers` on a directory handle is undocumented and
///   historically unreliable across Windows versions). This is not silently
///   ignored: NTFS is a journaling filesystem that commits a completed
///   rename's metadata transactionally as part of the rename operation
///   itself (the same durability argument SQLite's own Windows VFS
///   documentation relies on for `MoveFileExW`-based commits), so the
///   content-flush (`sync_all` on the temp file, always performed before any
///   rename here) plus the OS's own rename-transaction durability is this
///   platform's complete story -- there is deliberately no second Windows
///   call pretending to add a guarantee the platform does not expose.
pub(super) fn fsync_dir_after_replace(dir: &Dir) -> Result<(), PlatformError> {
    #[cfg(unix)]
    {
        use rustix::fs::{Mode, OFlags};

        // cap-std opens directories with O_PATH on Linux. That descriptor is
        // suitable for capability-relative operations, but fsync rejects it
        // with EBADF. Reopen the same directory through its capability handle
        // with a syncable access mode; never recover or reopen an ambient
        // path here.
        let syncable = rustix::fs::openat(
            dir,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| PlatformError::DirectorySync(error.into()))?;
        rustix::fs::fsync(syncable).map_err(|error| PlatformError::DirectorySync(error.into()))
    }
    #[cfg(windows)]
    {
        let _ = dir;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        Err(PlatformError::UnsupportedPlatform)
    }
}

/// Applies a replaced file's own `permissions` to the staged replacement that
/// will be renamed over it.
///
/// The atomic-write sequence in `super::writer` replaces a file by renaming a
/// freshly created temp file onto it, so the file that ends up on disk is a
/// different one and carries the temp file's permissions unless they are
/// copied across. Without this, editing a `0755` script strips its executable
/// bit and editing a `0600` file widens it to `0644` -- a private file becomes
/// world-readable because something edited it.
///
/// [`Permissions`] is already the correctly platform-shaped value (mode bits on
/// Unix, the read-only flag on Windows), so the copy itself needs no platform
/// split; only the Windows skip below does.
///
/// # Platform contract
///
/// - **Unix**: the mode is applied to the staged file. Doing so cannot strand
///   it: removing a file needs write permission on the containing *directory*,
///   not on the file, so `super::writer`'s cleanup path still works on a temp
///   file left at `0400`.
/// - **Windows**: a read-only attribute is deliberately *not* applied. Marking
///   the staged file read-only would make it undeletable, breaking
///   `super::writer`'s promise to leave no orphaned temp file on any failure
///   path. Nothing is lost by skipping it: a read-only destination cannot be
///   replaced by a rename at all, so such a write fails at the rename with the
///   original file (and its attribute) intact. Per-file ACLs are not cloned --
///   `cap-std` exposes no ACL API -- so the replacement inherits the parent
///   directory's ACL, exactly as any newly created file there would.
pub(super) fn apply_replacement_permissions(
    file: &cap_std::fs::File,
    permissions: &Permissions,
) -> io::Result<()> {
    #[cfg(windows)]
    if permissions.readonly() {
        return Ok(());
    }
    file.set_permissions(permissions.clone())
}

/// Whether `metadata` describes a symlink or (Windows) reparse point.
/// Mirrors `crate::security::account_cleanup::is_link_or_reparse`'s idiom.
pub(super) fn is_link_or_reparse(metadata: &Metadata) -> bool {
    if metadata.is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(all(test, target_os = "macos"))]
mod deletion_tests {
    use super::*;

    #[test]
    fn vnode_unlink_proof_survives_polls_and_does_not_confuse_rename() {
        let parent = tempfile::tempdir().expect("parent");
        let path = parent.path().join("project");
        let renamed = parent.path().join("renamed");
        std::fs::create_dir(&path).expect("directory");
        let mut observer = open_observer_root(&path).expect("register before unlink");
        assert!(matches!(
            root_deletion_state(&mut observer).unwrap(),
            RootDeletionState::NotDeleted
        ));
        std::fs::rename(&path, &renamed).expect("rename original object");
        assert!(matches!(
            root_deletion_state(&mut observer).unwrap(),
            RootDeletionState::NotDeleted
        ));
        std::fs::remove_dir(&renamed).expect("unlink original object");
        assert!(matches!(
            root_deletion_state(&mut observer).unwrap(),
            RootDeletionState::Confirmed
        ));
        // EV_CLEAR consumed the event. A failed trust-store persistence attempt
        // must still get the same proof when it retries the observer.
        assert!(matches!(
            root_deletion_state(&mut observer).unwrap(),
            RootDeletionState::Confirmed
        ));
    }
}
