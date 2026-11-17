//! Platform-neutral canonical containment policy for one relative path
//! *string* within a trusted project root.
//!
//! Everything here is pure and filesystem-free: no `Path::canonicalize`, no
//! I/O, no `cfg(windows)`/`cfg(unix)` branch.
//!
//! # Security contract
//!
//! This module only proves "this string is a structurally safe relative
//! path" -- rejecting `..` traversal, absolute-path injection (in any of
//! Unix/Windows/UNC form, regardless of the host OS actually running), and
//! platform-unsafe component forms (Windows alternate-data-stream syntax,
//! reserved device names, trailing dot/space). It never decides whether a
//! path is contained in a *specific* root: that requires asking the OS
//! (symlink/reparse/mount escape, changed root identity), which is
//! [`super::domain::TrustedRoot::resolve_existing`]/[`resolve_new`], backed
//! by `cap-std` and [`super::platform`]. Callers MUST go through
//! [`RelativeProjectPath::new`] plus that containment API -- never a raw
//! path join -- mirroring how `crate::skills::discovery` keeps its interim
//! symlink-escape guard behind a narrow helper rather than scattering
//! `canonicalize` calls through the module.
//!
//! [`resolve_new`]: super::domain::TrustedRoot::resolve_new

use std::path::{Component, Path};

/// Hard ceiling on a caller-supplied relative path string, matching common
/// filesystem `PATH_MAX`-order-of-magnitude limits (Linux's 4096-byte
/// `PATH_MAX`) rather than inventing a new bound.
const MAX_PATH_BYTES: usize = 4096;
/// Hard ceiling on one path component, matching the common 255-byte
/// `NAME_MAX` filesystem limit shared by ext4/NTFS/APFS.
const MAX_COMPONENT_BYTES: usize = 255;
/// Hard ceiling on the number of components, defending against a
/// pathologically deep (but individually short) relative path.
const MAX_COMPONENTS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PathPolicyError {
    #[error("project-relative path must not be empty")]
    Empty,
    #[error("project-relative path exceeds {0} bytes")]
    TooLong(usize),
    #[error("project-relative path has more than {0} components")]
    TooManyComponents(usize),
    #[error("project-relative path component exceeds {0} bytes")]
    ComponentTooLong(usize),
    #[error("project-relative path must not be absolute")]
    Absolute,
    #[error("project-relative path must not contain a `..` (parent) component")]
    ParentTraversal,
    #[error("project-relative path must not contain a NUL byte")]
    NulByte,
    #[error("project-relative path component must not be empty")]
    EmptyComponent,
    #[error("project-relative path component is a reserved device name")]
    ReservedDeviceName,
    #[error("project-relative path component ends with a trailing dot or space")]
    TrailingDotOrSpace,
    #[error("project-relative path component contains an alternate-data-stream marker")]
    AlternateDataStream,
    #[error("project-relative path must not contain a bare `.` component")]
    DotComponent,
    #[error("project-relative path component contains a `\\` separator from the other platform's convention")]
    SeparatorConfusion,
}

/// A caller-supplied path string, already proven structurally safe relative
/// to *some* root. Does not by itself prove containment in any specific
/// trusted root -- see the module doc comment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelativeProjectPath {
    value: String,
}

impl RelativeProjectPath {
    /// Validates `raw` as a relative, structurally safe path. Every check is
    /// pure text/structure analysis; no filesystem access occurs.
    pub fn new(raw: &str) -> Result<Self, PathPolicyError> {
        if raw.is_empty() {
            return Err(PathPolicyError::Empty);
        }
        if raw.len() > MAX_PATH_BYTES {
            return Err(PathPolicyError::TooLong(MAX_PATH_BYTES));
        }
        if raw.as_bytes().contains(&0) {
            return Err(PathPolicyError::NulByte);
        }
        // Reject absolute forms from *any* platform's convention regardless
        // of the OS this build actually targets: `Path::is_absolute` below is
        // Unix-blind to `C:\...`/`\\server\share` and Windows-blind to a bare
        // leading `/`, so a hostile string authored for the "other" platform
        // must be caught here first.
        if looks_absolute(raw) {
            return Err(PathPolicyError::Absolute);
        }

        // `\` is rejected here, before `Path::components()`, rather than
        // inside `validate_component`. On Windows `components()` treats `\`
        // as a separator and consumes it, so a per-component check can never
        // see one — leaving `a\..\b` classified as `ParentTraversal` on
        // Windows but `SeparatorConfusion` on Unix, and (worse) leaving `\`
        // silently *accepted* as a separator on Windows while Unix rejects
        // it. That asymmetry contradicts this module's stated design, which
        // rejects every other platform's conventions unconditionally (see the
        // ADS and trailing-dot checks) precisely so one hostile string is
        // judged identically on all three targets. `/` is the canonical
        // separator for this type; use `from_os_relative` to build one from
        // an OS path.
        if raw.contains('\\') {
            return Err(PathPolicyError::SeparatorConfusion);
        }

        let path = Path::new(raw);
        if path.is_absolute() {
            return Err(PathPolicyError::Absolute);
        }

        // Rebuilt from the components rather than kept as typed:
        // `Path::components` folds `a/./b`, `a//b` and `b.html/` into the same
        // components as `a/b` and `b.html`, so storing the raw string would
        // let one file be named several ways. Two operations on it would then
        // pass the duplicate-target check and the later write would win
        // without a word.
        let mut normalized = String::with_capacity(raw.len());
        let mut component_count = 0usize;
        for component in path.components() {
            component_count += 1;
            if component_count > MAX_COMPONENTS {
                return Err(PathPolicyError::TooManyComponents(MAX_COMPONENTS));
            }
            match component {
                Component::ParentDir => return Err(PathPolicyError::ParentTraversal),
                Component::RootDir | Component::Prefix(_) => return Err(PathPolicyError::Absolute),
                Component::CurDir => return Err(PathPolicyError::DotComponent),
                Component::Normal(name) => {
                    let name = name.to_str().ok_or(PathPolicyError::NulByte)?;
                    validate_component(name)?;
                    if !normalized.is_empty() {
                        normalized.push('/');
                    }
                    normalized.push_str(name);
                }
            }
        }
        if component_count == 0 {
            return Err(PathPolicyError::Empty);
        }

        Ok(Self { value: normalized })
    }

    /// Builds a canonical project-relative path from an OS-native relative
    /// path (typically a `strip_prefix` of the trusted root during a
    /// filesystem walk).
    ///
    /// Windows yields `dir\file` from such a walk while Unix yields
    /// `dir/file`. Storing whichever the host produced would make the index —
    /// and therefore every search result, every classification key, and every
    /// path echoed to the model — platform-dependent for the same project
    /// tree. This re-joins the path's components with the canonical `/` so
    /// one project indexes identically everywhere, then applies the full
    /// policy in [`Self::new`]; a component that is not valid UTF-8, or any
    /// non-`Normal` component, is rejected rather than lossily coerced.
    pub fn from_os_relative(relative: &Path) -> Result<Self, PathPolicyError> {
        let mut canonical = String::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(match component {
                    Component::ParentDir => PathPolicyError::ParentTraversal,
                    Component::CurDir => PathPolicyError::DotComponent,
                    _ => PathPolicyError::Absolute,
                });
            };
            let name = name.to_str().ok_or(PathPolicyError::NulByte)?;
            if !canonical.is_empty() {
                canonical.push('/');
            }
            canonical.push_str(name);
        }
        Self::new(&canonical)
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.value)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

fn looks_absolute(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    if matches!(bytes.first(), Some(b'/') | Some(b'\\')) {
        return true;
    }
    // Drive-letter prefix, e.g. `C:\...`, `C:/...`, or bare `C:`.
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    // Home-directory shorthand: never a project-relative form.
    raw.starts_with('~')
}

fn validate_component(name: &str) -> Result<(), PathPolicyError> {
    if name.is_empty() {
        return Err(PathPolicyError::EmptyComponent);
    }
    if name.len() > MAX_COMPONENT_BYTES {
        return Err(PathPolicyError::ComponentTooLong(MAX_COMPONENT_BYTES));
    }
    if name == "." || name == ".." {
        return Err(PathPolicyError::DotComponent);
    }
    // A literal `\` inside one already-split component only happens when the
    // *other* platform's separator convention was used against this build's
    // OS (e.g. a Windows-style `a\..\b` string parsed on a Unix build, where
    // `Path::components` does not treat `\` as a separator and would
    // otherwise pass this through as one oddly-named, harmless-looking
    // component). Reject it unconditionally rather than rely on it being
    // inert on this particular OS.
    if name.contains('\\') {
        return Err(PathPolicyError::SeparatorConfusion);
    }
    // Windows alternate-data-stream syntax (`name:stream`, `name:$DATA`).
    // Rejected unconditionally, not only `#[cfg(windows)]`: no legitimate
    // component name on any of the three target platforms contains a colon,
    // and a project directory synced onto Windows later must not suddenly
    // become escapable.
    if name.contains(':') {
        return Err(PathPolicyError::AlternateDataStream);
    }
    // Windows silently strips a trailing dot or space when resolving a path,
    // so `"CON. "` and `"CON"` can name the same reserved device, and
    // `"notes.txt."` can silently alias `"notes.txt"`. Reject both
    // unconditionally rather than only under `#[cfg(windows)]`.
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(PathPolicyError::TrailingDotOrSpace);
    }
    if is_reserved_device_name(name) {
        return Err(PathPolicyError::ReservedDeviceName);
    }
    Ok(())
}

/// Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM0`-`COM9`,
/// `LPT0`-`LPT9`), case-insensitive, matched against the component with any
/// extension stripped (`"con.txt"` names the same device as `"CON"`).
/// Rejected unconditionally for the same cross-platform-safety reason as the
/// ADS/trailing-dot checks above.
fn is_reserved_device_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
        "LPT9",
    ];
    let base = name.split('.').next().unwrap_or(name);
    RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(base))
}
