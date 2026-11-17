//! Native file-reference clipboard readers.
//!
//! The renderer supplies bytes for DOM clipboard `File` objects. This module
//! covers the OS file-manager formats that WebView clipboard events omit:
//! `CF_HDROP` on Windows and file `NSURL`s on macOS. Paths never cross IPC.

use std::path::PathBuf;

/// A single paste is deliberately no larger than the upload concurrency cap.
pub const MAX_CLIPBOARD_FILES: usize = 3;
#[cfg(windows)]
const MAX_NATIVE_PATH_UNITS: usize = 32_767;

#[derive(Debug, thiserror::Error)]
pub enum ClipboardReadError {
    #[error("the clipboard could not be read; try copying the files again")]
    Unavailable,
    #[error("paste up to three files at a time")]
    TooManyFiles,
    #[error("the clipboard contains an invalid file reference")]
    InvalidFileReference,
}

#[cfg(windows)]
pub fn native_file_paths() -> Result<Vec<PathBuf>, ClipboardReadError> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    // `IsClipboardFormatAvailable` reports a false result as an error in the
    // generated binding. Absence is the normal empty result for a DOM paste.
    // SAFETY: the only argument is a plain format id, so no pointer or handle
    // is passed and the call carries no memory invariant.
    if unsafe { IsClipboardFormatAvailable(u32::from(CF_HDROP.0)) }.is_err() {
        return Ok(Vec::new());
    }
    // SAFETY: `None` is a null owner window. On success this thread holds the
    // clipboard open, and `ClipboardGuard` closes it on every later return. The
    // guard is created only after a successful open.
    unsafe { OpenClipboard(None) }.map_err(|_| ClipboardReadError::Unavailable)?;
    struct ClipboardGuard;
    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            // SAFETY: this guard exists only after `OpenClipboard` succeeded, and it
            // is a local of this function, so it drops on the thread that opened the
            // clipboard. This call closes that open clipboard.
            let _ = unsafe { CloseClipboard() };
        }
    }
    let _guard = ClipboardGuard;

    // SAFETY: the clipboard stays open until `_guard` drops at the end of this
    // function, so the HDROP returned here is valid for every later use. The
    // clipboard owns this handle, so this code neither frees nor closes it.
    let handle = unsafe { GetClipboardData(u32::from(CF_HDROP.0)) }
        .map_err(|_| ClipboardReadError::Unavailable)?;
    let drop = HDROP(handle.0);
    // SAFETY: `drop` is the HDROP above, valid while the clipboard is open. Index
    // `u32::MAX` with a null buffer makes the call return the file count and
    // write nothing.
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) } as usize;
    if count > MAX_CLIPBOARD_FILES {
        return Err(ClipboardReadError::TooManyFiles);
    }

    let mut paths = Vec::with_capacity(count);
    for index in 0..count {
        // SAFETY: `index` is below `count`, the count reported for this same
        // HDROP, and `drop` is still valid. A null buffer makes the call return
        // the file name's length in characters, without the terminating null.
        let characters = unsafe { DragQueryFileW(drop, index as u32, None) } as usize;
        if characters == 0 || characters > MAX_NATIVE_PATH_UNITS {
            return Err(ClipboardReadError::InvalidFileReference);
        }
        let mut buffer = vec![0u16; characters + 1];
        // SAFETY: the buffer holds `characters + 1` units, the length the query
        // reported plus the terminating null. The binding passes that slice length
        // to the API as the buffer size, so the write stays inside `buffer`.
        let written = unsafe { DragQueryFileW(drop, index as u32, Some(buffer.as_mut_slice())) };
        if written as usize != characters {
            return Err(ClipboardReadError::InvalidFileReference);
        }
        buffer.truncate(characters);
        paths.push(PathBuf::from(std::ffi::OsString::from_wide(&buffer)));
    }
    Ok(paths)
}

#[cfg(target_os = "macos")]
pub fn native_file_paths() -> Result<Vec<PathBuf>, ClipboardReadError> {
    use objc2::{runtime::AnyClass, ClassType};
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSArray, NSURL};

    let pasteboard = NSPasteboard::generalPasteboard();
    let classes: objc2::rc::Retained<NSArray<AnyClass>> = NSArray::from_slice(&[NSURL::class()]);
    // SAFETY: `classes` contains exactly NSURL's class object and no options
    // dictionary is supplied. The returned untyped array is checked item by
    // item before conversion.
    let Some(objects) = (unsafe { pasteboard.readObjectsForClasses_options(&classes, None) })
    else {
        return Ok(Vec::new());
    };
    if objects.len() > MAX_CLIPBOARD_FILES {
        return Err(ClipboardReadError::TooManyFiles);
    }

    let mut paths = Vec::with_capacity(objects.len());
    for object in objects.to_vec() {
        let url = object
            .downcast::<NSURL>()
            .map_err(|_| ClipboardReadError::InvalidFileReference)?;
        if !url.isFileURL() {
            continue;
        }
        paths.push(
            url.to_file_path()
                .ok_or(ClipboardReadError::InvalidFileReference)?,
        );
    }
    Ok(paths)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn native_file_paths() -> Result<Vec<PathBuf>, ClipboardReadError> {
    Ok(Vec::new())
}
