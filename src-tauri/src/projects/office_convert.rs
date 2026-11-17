//! Hidden conversion of one Office file to a PDF, by a program the person
//! already has installed (spec: add-office-chat-files).
//!
//! Nothing is bundled or installed. A copy of the file goes into a private
//! temporary folder, Microsoft Word, PowerPoint or Excel is asked to write a PDF
//! with macros forced off and the file opened read-only, and the PDF is read back
//! within a size limit. The temporary folder is removed afterwards, and the
//! original file is never touched.
//!
//! Windows drives the program through COM from a fixed PowerShell script;
//! macOS drives it through a fixed AppleScript. Both receive the paths as
//! arguments, never as script text. Other platforms report the preview as
//! unavailable.

// Linux is not a shipping platform (only a development host), where converting
// reports the preview as unavailable and the helpers below have no caller.
#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

/// Largest source copied for conversion; the same bound as the text readers.
pub(crate) const MAX_SOURCE_BYTES: u64 = crate::documents::MAX_DOCUMENT_BYTES;
/// Largest PDF accepted back.
pub(crate) const MAX_PDF_BYTES: u64 = 64 * 1024 * 1024;
/// A cold Office start can take a minute on some machines, and one that is
/// still shutting down from the previous preview can delay the next start.
const CONVERT_TIMEOUT: Duration = Duration::from_secs(120);
/// After the script has reported its result, the program may still be closing.
/// It is given this long to finish before it is stopped.
const TEARDOWN_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const TEARDOWN_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Longest process id written by the platform script, in digits.
const MAX_PID_DIGITS: usize = 10;
/// Most bytes read from the file the script reports its result code in.
const MAX_CODE_BYTES: u64 = 16;
/// Most bytes read from the platform script's error output. Only the start is
/// kept for the log, so a long run of errors cannot grow the read.
const MAX_SCRIPT_ERROR_BYTES: u64 = 4096;
/// Longest script error text written to the log, in bytes.
const MAX_SCRIPT_ERROR_LOG_BYTES: usize = 512;
const STAGING_PREFIX: &str = "xfalcon-office-";
/// A staging folder this old was left by a run that never finished (the app
/// closed or crashed), far past the longest run plus its teardown.
const STALE_AFTER: Duration = Duration::from_secs(30 * 60);

/// The Office program that opens a given kind of file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OfficeApp {
    Word,
    PowerPoint,
    Excel,
}

impl OfficeApp {
    pub(crate) fn for_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "doc" | "docx" => Some(Self::Word),
            "ppt" | "pptx" => Some(Self::PowerPoint),
            "xls" | "xlsx" => Some(Self::Excel),
            _ => None,
        }
    }

    pub(crate) fn for_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|extension| extension.to_str())
            .and_then(Self::for_extension)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Word => "Microsoft Word",
            Self::PowerPoint => "Microsoft PowerPoint",
            Self::Excel => "Microsoft Excel",
        }
    }

    /// The word the Windows script expects.
    #[cfg(windows)]
    fn script_kind(self) -> &'static str {
        match self {
            Self::Word => "word",
            Self::PowerPoint => "powerpoint",
            Self::Excel => "excel",
        }
    }
}

/// Whether the project viewer can offer a PDF preview for this path.
pub(crate) fn is_convertible_path(path: &Path) -> bool {
    OfficeApp::for_path(path).is_some()
}

/// The extension as one of the six known suffixes, lower-cased.
pub(crate) fn known_extension(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    ["doc", "docx", "ppt", "pptx", "xls", "xlsx"]
        .into_iter()
        .find(|known| *known == extension)
}

/// Why a conversion did not produce a PDF. The text is shown to the person.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("The Office preview was cancelled.")]
    Cancelled,
    /// Produced only on a platform without a converter, so it is unused on the
    /// two that ship.
    #[allow(dead_code)]
    #[error("Previews are available on Windows and macOS.")]
    Unsupported,
    /// One program is not installed. Internal: the next program is tried, and
    /// the person sees `NoProgram` only when every one was missing.
    #[error("{0} is needed for this preview and is not installed.")]
    NotInstalled(&'static str),
    #[error("This preview needs {0}, and none of them was found.")]
    NoProgram(&'static str),
    #[error("{0} could not open this file. It may be password-protected, damaged, or blocked by your Office security settings.")]
    CouldNotOpen(&'static str),
    #[error("{0} could not turn this file into a PDF.")]
    Failed(&'static str),
    #[error("The preview took too long and was stopped.")]
    TimedOut,
    #[error("This file is too large to preview.")]
    TooLarge,
    #[error("The preview could not be created.")]
    InvalidOutput,
    /// Produced only by the macOS converter.
    #[allow(dead_code)]
    #[error("macOS did not let xFalcon share this file with {0}. Allow it under System Settings > Privacy & Security, then try again.")]
    AccessBlocked(&'static str),
    #[error("Temporary preview files could not be accessed.")]
    Io(#[source] std::io::Error),
}

impl From<std::io::Error> for ConvertError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Converts `bytes`, a copy of an Office file with the given extension, to a PDF.
///
/// Blocking; call it from a blocking thread. `cancel` stops the work and the
/// Office program it started. Every outcome is written to the app log: a
/// finished conversion with its size, or a failure with its kind.
pub(crate) fn convert_to_pdf(
    bytes: &[u8],
    extension: &str,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, ConvertError> {
    let started = Instant::now();
    let result = convert_checked(bytes, extension, cancel);
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match &result {
        Ok(pdf) => tracing::info!(
            extension,
            elapsed_ms,
            pdf_bytes = pdf.len(),
            "office conversion finished"
        ),
        Err(ConvertError::Cancelled) => {
            tracing::info!(extension, elapsed_ms, "office conversion cancelled");
        }
        Err(error) => tracing::warn!(
            extension,
            elapsed_ms,
            error = ?error,
            "office conversion failed"
        ),
    }
    result
}

/// The checks and the platform conversion behind `convert_to_pdf`.
fn convert_checked(
    bytes: &[u8],
    extension: &str,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, ConvertError> {
    let app = OfficeApp::for_extension(extension).ok_or(ConvertError::Failed("The preview"))?;
    if bytes.is_empty() {
        return Err(ConvertError::InvalidOutput);
    }
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err(ConvertError::TooLarge);
    }
    if cancel.is_cancelled() {
        return Err(ConvertError::Cancelled);
    }
    platform::convert(app, bytes, &extension.to_ascii_lowercase(), cancel)
}

/// A private folder holding the copy of the source and the PDF the program
/// writes. It is removed when dropped.
struct Staging {
    /// Held so the folder is removed when this value drops; LibreOffice's
    /// profile and the macOS exchange also live under its path.
    dir: tempfile::TempDir,
    input: PathBuf,
    output: PathBuf,
    pid_file: PathBuf,
    /// Where the script writes its result code once it knows it.
    done_file: PathBuf,
    /// Where the platform script's error output is written, on a platform that
    /// captures it (macOS).
    script_errors: PathBuf,
}

impl Staging {
    /// Creates the folder under `parent` and writes the read-only source copy.
    fn create(parent: &Path, bytes: &[u8], extension: &str) -> std::io::Result<Self> {
        remove_stale(parent);
        let dir = tempfile::Builder::new()
            .prefix(STAGING_PREFIX)
            .tempdir_in(parent)?;
        let input = dir.path().join(format!("source.{extension}"));
        let mut source = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&input)?;
        source.write_all(bytes)?;
        source.sync_all()?;
        drop(source);
        let mut permissions = fs::metadata(&input)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&input, permissions)?;
        Ok(Self {
            output: dir.path().join("preview.pdf"),
            pid_file: dir.path().join("office.pid"),
            done_file: dir.path().join("office.done"),
            script_errors: dir.path().join("script.stderr"),
            input,
            dir,
        })
    }
}

/// Removes staging folders an earlier run left behind. They hold a copy of a
/// person's document, so they must not outlive a crash. Best effort: a folder
/// that cannot be removed is tried again on the next run.
fn remove_stale(parent: &Path) {
    remove_older_than(parent, STALE_AFTER);
}

fn remove_older_than(parent: &Path, limit: Duration) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let is_staging = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(STAGING_PREFIX));
        // The temp folder can hold thousands of other entries: never stat them.
        if !is_staging {
            continue;
        }
        let path = entry.path();
        // `symlink_metadata`, so a link named like a staging folder is never followed.
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > limit);
        if !metadata.is_dir() || !stale {
            continue;
        }
        // The source copy is read-only; clear that so it can be removed.
        if let Ok(files) = fs::read_dir(&path) {
            for file in files.flatten() {
                if let Ok(metadata) = fs::symlink_metadata(file.path()) {
                    if metadata.is_file() {
                        make_writable(&file.path(), &metadata);
                    }
                }
            }
        }
        let _ = fs::remove_dir_all(&path);
    }
}

#[allow(clippy::permissions_set_readonly_false)]
fn make_writable(path: &Path, metadata: &fs::Metadata) {
    let mut permissions = metadata.permissions();
    permissions.set_readonly(false);
    let _ = fs::set_permissions(path, permissions);
}

impl Drop for Staging {
    fn drop(&mut self) {
        // The source copy is read-only so the program cannot save over it; make
        // it removable again before the folder is deleted.
        if let Ok(metadata) = fs::metadata(&self.input) {
            make_writable(&self.input, &metadata);
        }
    }
}

/// How a script run ended.
struct Finished {
    code: Option<i32>,
    /// The script, when it reported its result before it exited. Closing the
    /// Office program can take a minute, and the result is already known, so
    /// the caller need not wait for it (see `finish_in_background`).
    running: Option<Child>,
}

/// Runs `command` until it ends, is cancelled, or runs out of time. With a
/// `done_file`, a result code written there also ends the wait, leaving the
/// process running. On cancel or timeout the process and any Office process it
/// reported in `pid_file` are stopped. Standard error goes where the caller's
/// command sends it.
fn run_to_completion(
    mut command: Command,
    pid_file: &Path,
    done_file: Option<&Path>,
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<Finished, ConvertError> {
    command.stdin(Stdio::null()).stdout(Stdio::null());
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(Finished {
                    code: status.code(),
                    running: None,
                })
            }
            Ok(None) => {}
            Err(error) => {
                stop(&mut child, pid_file);
                return Err(ConvertError::Io(error));
            }
        }
        if let Some(code) = done_file.and_then(read_done) {
            return Ok(Finished {
                code: Some(code),
                running: Some(child),
            });
        }
        if cancel.is_cancelled() {
            stop(&mut child, pid_file);
            return Err(ConvertError::Cancelled);
        }
        if Instant::now() >= deadline {
            stop(&mut child, pid_file);
            return Err(ConvertError::TimedOut);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Runs the platform script for `staging` and reads the PDF it wrote. With
/// `reports_done`, the script is not waited for once it has reported its result.
fn run_and_read(
    program: &'static str,
    command: Command,
    staging: Staging,
    reports_done: bool,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, ConvertError> {
    let done_file = reports_done.then_some(staging.done_file.as_path());
    let finished = run_to_completion(
        command,
        &staging.pid_file,
        done_file,
        cancel,
        CONVERT_TIMEOUT,
    )?;
    let result = classify_exit(program, finished.code).and_then(|()| read_pdf(&staging.output));
    if let Err(error) = &result {
        let script_error = script_error_summary(&staging.script_errors);
        tracing::warn!(
            program,
            exit_code = ?finished.code,
            error = ?error,
            script_error = script_error.as_deref().unwrap_or("none"),
            "office conversion program ended without a PDF"
        );
    }
    if let Some(child) = finished.running {
        finish_in_background(child, staging, TEARDOWN_TIMEOUT);
    }
    result
}

/// Lets a script that has reported its result finish closing the Office
/// program without holding the preview up. The folder it works in is removed
/// once it ends; if it is still running after `timeout`, it and the Office
/// program it reported are stopped first. If no thread can be started, the
/// script still ends on its own.
fn finish_in_background(mut child: Child, staging: Staging, timeout: Duration) {
    let _ = std::thread::Builder::new()
        .name("office-teardown".into())
        .spawn(move || {
            let deadline = Instant::now() + timeout;
            while matches!(child.try_wait(), Ok(None)) {
                if Instant::now() >= deadline {
                    stop(&mut child, &staging.pid_file);
                    break;
                }
                std::thread::sleep(TEARDOWN_POLL_INTERVAL);
            }
        });
}

/// The result code the script reported, once it has.
fn read_done(path: &Path) -> Option<i32> {
    let mut text = String::new();
    File::open(path)
        .ok()?
        .take(MAX_CODE_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    text.trim().parse().ok()
}

/// The platform script's error output, as one bounded, sanitized line for the
/// log, or `None` when it wrote none.
fn script_error_summary(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_SCRIPT_ERROR_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let joined = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if joined.is_empty() {
        return None;
    }
    let sanitized = crate::diagnostics::sanitize_diagnostics_text(&joined);
    let mut end = sanitized.len().min(MAX_SCRIPT_ERROR_LOG_BYTES);
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    Some(sanitized[..end].to_owned())
}

fn stop(child: &mut Child, pid_file: &Path) {
    // A program such as LibreOffice starts its real process under a launcher;
    // ending the launcher alone would leave that running.
    platform::kill_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
    if let Some(pid) = read_pid(pid_file) {
        platform::kill_process(pid);
    }
}

#[cfg(unix)]
fn kill_unix(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-9", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// The Office process id the script reported, if it wrote one.
fn read_pid(pid_file: &Path) -> Option<u32> {
    let mut text = String::new();
    File::open(pid_file)
        .ok()?
        .take(MAX_PID_DIGITS as u64 + 2)
        .read_to_string(&mut text)
        .ok()?;
    let digits = text.trim();
    if digits.is_empty()
        || digits.len() > MAX_PID_DIGITS
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    digits.parse().ok().filter(|pid| *pid > 4)
}

/// Reads the PDF the program wrote, within the size limit.
fn read_pdf(path: &Path) -> Result<Vec<u8>, ConvertError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(reason = "missing", "office conversion output is not a PDF");
            return Err(ConvertError::InvalidOutput);
        }
        Err(error) => return Err(ConvertError::Io(error)),
    };
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        tracing::warn!(
            reason = "empty or not a regular file",
            "office conversion output is not a PDF"
        );
        return Err(ConvertError::InvalidOutput);
    }
    if metadata.len() > MAX_PDF_BYTES {
        return Err(ConvertError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    File::open(path)?
        .take(MAX_PDF_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PDF_BYTES {
        return Err(ConvertError::TooLarge);
    }
    if !bytes.starts_with(b"%PDF-") {
        tracing::warn!(
            reason = "no PDF header",
            "office conversion output is not a PDF"
        );
        return Err(ConvertError::InvalidOutput);
    }
    Ok(bytes)
}

/// What a platform script's exit code means.
fn classify_exit(program: &'static str, code: Option<i32>) -> Result<(), ConvertError> {
    match code {
        Some(0) => Ok(()),
        Some(10) => Err(ConvertError::NotInstalled(program)),
        Some(12) => Err(ConvertError::CouldNotOpen(program)),
        _ => Err(ConvertError::Failed(program)),
    }
}

/// Tries each program in turn. Only a program that is not installed hands the
/// file on to the next: one that is installed and fails (a protected file, a
/// format its policy blocks) gives the answer, so a stricter program's refusal
/// is never worked around by a looser one.
fn first_available(
    programs: &[&dyn Fn() -> Result<Vec<u8>, ConvertError>],
    names: &'static str,
) -> Result<Vec<u8>, ConvertError> {
    for program in programs {
        match program() {
            Err(ConvertError::NotInstalled(_)) => continue,
            other => return other,
        }
    }
    Err(ConvertError::NoProgram(names))
}

/// Settings written into LibreOffice's fresh, private profile before it runs:
/// no macro may run (the highest security level, and execution switched off).
const LIBREOFFICE_LOCKDOWN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="DisableMacrosExecution" oor:op="fuse"><value>true</value></prop></item>
</oor:items>
"#;

/// `file:///` URL of an absolute path, as LibreOffice's `-env:UserInstallation`
/// wants it: forward slashes, and everything but unreserved characters escaped.
fn file_url(path: &str) -> String {
    let text = path.replace('\\', "/");
    let mut url = String::from("file://");
    if !text.starts_with('/') {
        url.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                url.push(char::from(byte));
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

/// Headless, with its own fresh profile so it neither joins nor disturbs a
/// LibreOffice the person has open, and no extension, recovery or lock check.
fn libreoffice_arguments(
    profile_url: &str,
    out_dir: &Path,
    input: &Path,
) -> Vec<std::ffi::OsString> {
    let mut arguments: Vec<std::ffi::OsString> = [
        "--headless",
        "--invisible",
        "--norestore",
        "--nologo",
        "--nodefault",
        "--nolockcheck",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    arguments.push(format!("-env:UserInstallation={profile_url}").into());
    arguments.push("--convert-to".into());
    arguments.push("pdf".into());
    arguments.push("--outdir".into());
    arguments.push(out_dir.as_os_str().to_owned());
    arguments.push(input.as_os_str().to_owned());
    arguments
}

/// Converts with an installed LibreOffice, if there is one. It runs on its own
/// copy of the file, with macros switched off in a profile made for this run.
fn convert_libreoffice(
    bytes: &[u8],
    extension: &str,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, ConvertError> {
    let Some(soffice) = platform::libreoffice_path() else {
        return Err(ConvertError::NotInstalled(LIBREOFFICE));
    };
    let mut staging = Staging::create(&std::env::temp_dir(), bytes, extension)?;
    // LibreOffice names the PDF after the file it was given.
    staging.output = staging.input.with_extension("pdf");
    let profile = staging.dir.path().join("lo-profile");
    fs::create_dir_all(profile.join("user"))?;
    fs::write(
        profile.join("user").join("registrymodifications.xcu"),
        LIBREOFFICE_LOCKDOWN,
    )?;
    let mut command = Command::new(soffice);
    command.args(libreoffice_arguments(
        &file_url(&profile.to_string_lossy()),
        staging.dir.path(),
        &staging.input,
    ));
    command.stderr(Stdio::null());
    platform::hide_window(&mut command);
    run_and_read(LIBREOFFICE, command, staging, false, cancel)
}

const LIBREOFFICE: &str = "LibreOffice";

/// Whether a program with no switch to force macros off may be given this file.
/// The extension is only a name, so the content is checked too: it must be an
/// Office Open XML package (a zip), and must not carry a macro project, which
/// would be listed in the package's directory under its fixed name.
#[cfg(any(windows, test))]
fn cannot_run_a_macro(extension: &str, bytes: &[u8]) -> bool {
    const MACRO_PROJECT: &[u8] = b"vbaProject.bin";
    matches!(extension, "docx" | "pptx" | "xlsx")
        && bytes.starts_with(b"PK\x03\x04")
        && !bytes
            .windows(MACRO_PROJECT.len())
            .any(|window| window == MACRO_PROJECT)
}

#[cfg(windows)]
mod platform {
    use std::os::windows::process::CommandExt;

    use base64::Engine as _;

    use super::*;

    const SCRIPT: &str = include_str!("office_convert/convert.ps1");
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    const PROGRAMS: &str = "Microsoft Office, LibreOffice or WPS Office";

    /// Microsoft Office first, then LibreOffice, then WPS Office; each is tried
    /// only when the one before it is not installed.
    pub(super) fn convert(
        app: OfficeApp,
        bytes: &[u8],
        extension: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ConvertError> {
        first_available(
            &[
                &|| run_script(app.name(), app, bytes, extension, None, cancel),
                &|| convert_libreoffice(bytes, extension, cancel),
                &|| convert_wps(app, bytes, extension, cancel),
            ],
            PROGRAMS,
        )
    }

    /// WPS Office has the same object model as Microsoft Office, so the same
    /// script drives it under its own names. It has no switch that forces macros
    /// off, so it is given only files that cannot run one (`cannot_run_a_macro`);
    /// for any other file it counts as absent.
    fn convert_wps(
        app: OfficeApp,
        bytes: &[u8],
        extension: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ConvertError> {
        const WPS: &str = "WPS Office";
        if !cannot_run_a_macro(extension, bytes) {
            return Err(ConvertError::NotInstalled(WPS));
        }
        let names = match app {
            OfficeApp::Word => ("KWPS.Application", "wps"),
            OfficeApp::PowerPoint => ("KWPP.Application", "wpp"),
            OfficeApp::Excel => ("KET.Application", "et"),
        };
        run_script(WPS, app, bytes, extension, Some(names), cancel)
    }

    /// Runs the fixed script. `names` is the program's COM class and process
    /// name when it is not Microsoft Office.
    fn run_script(
        program: &'static str,
        app: OfficeApp,
        bytes: &[u8],
        extension: &str,
        names: Option<(&str, &str)>,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ConvertError> {
        let staging = Staging::create(&std::env::temp_dir(), bytes, extension)?;
        let mut command = Command::new(system32().join(r"WindowsPowerShell\v1.0\powershell.exe"));
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &encoded_script(),
            ])
            .env("XF_IN", &staging.input)
            .env("XF_OUT", &staging.output)
            .env("XF_KIND", app.script_kind())
            .env("XF_PIDFILE", &staging.pid_file)
            .env("XF_DONEFILE", &staging.done_file)
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        if let Some((class, process)) = names {
            command.env("XF_PROGID", class).env("XF_PROCESS", process);
        }
        run_and_read(program, command, staging, true, cancel)
    }

    /// The installed LibreOffice, in the places its installer uses.
    pub(super) fn libreoffice_path() -> Option<PathBuf> {
        const RELATIVE: &str = r"LibreOffice\program\soffice.exe";
        let roots = [
            std::env::var_os("ProgramFiles").map(PathBuf::from),
            std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
            std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Programs")),
        ];
        roots
            .into_iter()
            .flatten()
            .map(|root| root.join(RELATIVE))
            .find(|candidate| candidate.is_file())
    }

    pub(super) fn hide_window(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    /// Ends a process and everything it started.
    pub(super) fn kill_tree(pid: u32) {
        let _ = Command::new(system32().join("taskkill.exe"))
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }

    /// `-EncodedCommand` carries the script as UTF-16LE base64, so no quoting or
    /// script file (and no execution-policy setting) is involved.
    fn encoded_script() -> String {
        let utf16: Vec<u8> = SCRIPT.encode_utf16().flat_map(u16::to_le_bytes).collect();
        base64::engine::general_purpose::STANDARD.encode(utf16)
    }

    fn system32() -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        PathBuf::from(root).join("System32")
    }

    /// Stops `pid` only if it is still a Word, PowerPoint, Excel or WPS process. The
    /// id was written by a script that may have ended minutes ago, and Windows
    /// reuses ids, so it must not be trusted to name the same program.
    pub(super) fn kill_process(pid: u32) {
        for image in [
            "WINWORD.EXE",
            "POWERPNT.EXE",
            "EXCEL.EXE",
            "wps.exe",
            "wpp.exe",
            "et.exe",
        ] {
            let _ = Command::new(system32().join("taskkill.exe"))
                .args([
                    "/F",
                    "/PID",
                    &pid.to_string(),
                    "/FI",
                    &format!("IMAGENAME eq {image}"),
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
    }

    #[cfg(test)]
    pub(super) fn script() -> &'static str {
        SCRIPT
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    /// One script per program, each aimed at a literal app id: AppleScript reads
    /// a program's terms from its dictionary only when the target is known as
    /// the script is compiled, so a single script with a variable target would
    /// not compile.
    fn script(app: OfficeApp) -> &'static str {
        match app {
            OfficeApp::Word => include_str!("office_convert/convert-word.applescript"),
            OfficeApp::PowerPoint => include_str!("office_convert/convert-powerpoint.applescript"),
            OfficeApp::Excel => include_str!("office_convert/convert-excel.applescript"),
        }
    }

    /// Office for Mac runs in a sandbox and can read and write only inside its
    /// own container without asking. The copy and the PDF are therefore
    /// exchanged through the program's `Documents` folder in its container.
    fn container(app: OfficeApp) -> (&'static str, &'static str) {
        match app {
            OfficeApp::Word => ("Microsoft Word.app", "com.microsoft.Word"),
            OfficeApp::PowerPoint => ("Microsoft PowerPoint.app", "com.microsoft.Powerpoint"),
            OfficeApp::Excel => ("Microsoft Excel.app", "com.microsoft.Excel"),
        }
    }

    fn installed(bundle: &str) -> bool {
        let mut homes = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            homes.push(PathBuf::from(home).join("Applications"));
        }
        homes.iter().any(|root| root.join(bundle).is_dir())
    }

    const PROGRAMS: &str = "Microsoft Office or LibreOffice";

    /// Microsoft Office first, then LibreOffice if it is not installed. WPS
    /// Office for Mac has no scripting interface to drive.
    pub(super) fn convert(
        app: OfficeApp,
        bytes: &[u8],
        extension: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ConvertError> {
        first_available(
            &[
                &|| convert_microsoft(app, bytes, extension, cancel),
                &|| convert_libreoffice(bytes, extension, cancel),
            ],
            PROGRAMS,
        )
    }

    fn convert_microsoft(
        app: OfficeApp,
        bytes: &[u8],
        extension: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ConvertError> {
        let (bundle, container_id) = container(app);
        // Asking an app that is not installed would open a "Where is it?" dialog.
        if !installed(bundle) {
            return Err(ConvertError::NotInstalled(app.name()));
        }
        let home = std::env::var_os("HOME").ok_or(ConvertError::Failed(app.name()))?;
        let exchange = PathBuf::from(home)
            .join("Library/Containers")
            .join(container_id)
            .join("Data/Documents");
        fs::create_dir_all(&exchange).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                ConvertError::AccessBlocked(app.name())
            } else {
                ConvertError::Io(error)
            }
        })?;
        let staging = Staging::create(&exchange, bytes, extension).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                ConvertError::AccessBlocked(app.name())
            } else {
                ConvertError::Io(error)
            }
        })?;
        // The script is fixed text from this binary and goes in with `-e`, so
        // nothing is written into the program's container but the file and the
        // PDF. The two paths are arguments to it, never part of it.
        let script_errors = File::create(&staging.script_errors)?;
        let mut command = Command::new("/usr/bin/osascript");
        command
            .arg("-e")
            .arg(script(app))
            .arg(&staging.input)
            .arg(&staging.output)
            .stderr(script_errors);
        run_and_read(app.name(), command, staging, false, cancel)
    }

    /// The installed LibreOffice, in the places its installer puts it.
    pub(super) fn libreoffice_path() -> Option<PathBuf> {
        let relative = "LibreOffice.app/Contents/MacOS/soffice";
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        roots
            .into_iter()
            .map(|root| root.join(relative))
            .find(|candidate| candidate.is_file())
    }

    pub(super) fn hide_window(_command: &mut Command) {}

    pub(super) fn kill_tree(_pid: u32) {}

    pub(super) fn kill_process(pid: u32) {
        super::kill_unix(pid);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::*;

    pub(super) fn convert(
        _app: OfficeApp,
        _bytes: &[u8],
        _extension: &str,
        _cancel: &CancellationToken,
    ) -> Result<Vec<u8>, ConvertError> {
        Err(ConvertError::Unsupported)
    }

    pub(super) fn libreoffice_path() -> Option<PathBuf> {
        None
    }

    pub(super) fn hide_window(_command: &mut Command) {}

    pub(super) fn kill_tree(_pid: u32) {}

    pub(super) fn kill_process(pid: u32) {
        #[cfg(unix)]
        super::kill_unix(pid);
        #[cfg(not(unix))]
        let _ = pid;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_extension_maps_to_its_program_case_insensitively() {
        for (extension, app) in [
            ("doc", OfficeApp::Word),
            ("DOCX", OfficeApp::Word),
            ("ppt", OfficeApp::PowerPoint),
            ("PptX", OfficeApp::PowerPoint),
            ("xls", OfficeApp::Excel),
            ("xlsx", OfficeApp::Excel),
        ] {
            assert_eq!(
                OfficeApp::for_extension(extension),
                Some(app),
                "{extension}"
            );
        }
        assert_eq!(OfficeApp::for_extension("pdf"), None);
        assert_eq!(OfficeApp::for_extension("docm"), None);
        assert!(is_convertible_path(Path::new("deck.PPTX")));
        assert!(!is_convertible_path(Path::new("notes.txt")));
        assert!(!is_convertible_path(Path::new("no-extension")));
        assert_eq!(known_extension(Path::new("a/b/Deck.PPT")), Some("ppt"));
        assert_eq!(known_extension(Path::new("a.csv")), None);
    }

    #[test]
    fn exit_codes_become_specific_messages() {
        assert!(classify_exit("Microsoft Word", Some(0)).is_ok());
        assert!(matches!(
            classify_exit("Microsoft Excel", Some(10)),
            Err(ConvertError::NotInstalled("Microsoft Excel"))
        ));
        assert!(matches!(
            classify_exit("WPS Office", Some(12)),
            Err(ConvertError::CouldNotOpen("WPS Office"))
        ));
        assert!(matches!(
            classify_exit("LibreOffice", Some(1)),
            Err(ConvertError::Failed("LibreOffice"))
        ));
        assert!(matches!(
            classify_exit("Microsoft Word", None),
            Err(ConvertError::Failed(_))
        ));
    }

    #[test]
    fn a_pdf_must_be_a_real_bounded_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.pdf");
        fs::write(&good, b"%PDF-1.7\nbody").unwrap();
        assert_eq!(read_pdf(&good).unwrap(), b"%PDF-1.7\nbody");

        let missing = dir.path().join("missing.pdf");
        assert!(matches!(
            read_pdf(&missing),
            Err(ConvertError::InvalidOutput)
        ));

        let empty = dir.path().join("empty.pdf");
        fs::write(&empty, b"").unwrap();
        assert!(matches!(read_pdf(&empty), Err(ConvertError::InvalidOutput)));

        let not_pdf = dir.path().join("not.pdf");
        fs::write(&not_pdf, b"PK\x03\x04 not a pdf").unwrap();
        assert!(matches!(
            read_pdf(&not_pdf),
            Err(ConvertError::InvalidOutput)
        ));

        let folder = dir.path().join("folder.pdf");
        fs::create_dir(&folder).unwrap();
        assert!(matches!(
            read_pdf(&folder),
            Err(ConvertError::InvalidOutput)
        ));

        let huge = dir.path().join("huge.pdf");
        let file = File::create(&huge).unwrap();
        file.set_len(MAX_PDF_BYTES + 1).unwrap();
        assert!(matches!(read_pdf(&huge), Err(ConvertError::TooLarge)));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_the_pdf_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("secret.pdf");
        fs::write(&target, b"%PDF-1.7\nsecret").unwrap();
        let link = dir.path().join("preview.pdf");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(matches!(read_pdf(&link), Err(ConvertError::InvalidOutput)));
    }

    #[test]
    fn only_a_plausible_process_id_is_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("office.pid");
        for (text, expected) in [
            ("4242\r\n", Some(4242)),
            ("4242", Some(4242)),
            ("", None),
            ("abc", None),
            ("-5", None),
            ("1 2", None),
            ("0", None),
            ("4", None),
            ("12345678901", None),
        ] {
            fs::write(&file, text).unwrap();
            assert_eq!(read_pid(&file), expected, "{text:?}");
        }
        assert_eq!(read_pid(&dir.path().join("none")), None);
    }

    #[test]
    fn staging_keeps_the_source_read_only_and_cleans_up() {
        let parent = tempfile::tempdir().unwrap();
        let folder;
        {
            let staging = Staging::create(parent.path(), b"original", "docx").unwrap();
            folder = staging.dir.path().to_path_buf();
            assert_eq!(fs::read(&staging.input).unwrap(), b"original");
            assert!(fs::metadata(&staging.input)
                .unwrap()
                .permissions()
                .readonly());
            assert!(staging.input.ends_with("source.docx"));
        }
        assert!(!folder.exists(), "the private folder must be removed");
    }

    #[test]
    fn folders_left_by_a_run_that_never_finished_are_removed() {
        let parent = tempfile::tempdir().unwrap();
        let make = |name: &str| {
            let folder = parent.path().join(name);
            fs::create_dir(&folder).unwrap();
            let source = folder.join("source.docx");
            fs::write(&source, b"copy of a document").unwrap();
            let mut permissions = fs::metadata(&source).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&source, permissions).unwrap();
            folder
        };
        let abandoned = make("xfalcon-office-abandoned");
        let unrelated = make("someone-elses-folder");

        // Nothing is as old as the real limit yet: a run may still be using it.
        remove_stale(parent.path());
        assert!(abandoned.exists(), "a recent staging folder may be in use");

        // Once it is older than the limit it goes, read-only copy and all.
        std::thread::sleep(Duration::from_millis(60));
        remove_older_than(parent.path(), Duration::from_millis(10));
        assert!(!abandoned.exists(), "an abandoned staging folder goes");
        assert!(unrelated.exists(), "only staging folders are touched");
    }

    fn missing(name: &'static str) -> Result<Vec<u8>, ConvertError> {
        Err(ConvertError::NotInstalled(name))
    }

    #[test]
    fn the_next_program_is_tried_only_when_the_one_before_is_not_installed() {
        let calls = std::cell::RefCell::new(Vec::new());
        let step = |name: &'static str,
                    result: fn(&'static str) -> Result<Vec<u8>, ConvertError>| {
            calls.borrow_mut().push(name);
            result(name)
        };
        let ok = |name: &'static str| Ok(name.as_bytes().to_vec());

        // The first program is missing: the second answers, the third is not asked.
        let pdf = first_available(
            &[
                &|| step("microsoft", missing),
                &|| step("libreoffice", ok),
                &|| step("wps", ok),
            ],
            "all",
        )
        .unwrap();
        assert_eq!(pdf, b"libreoffice");
        assert_eq!(*calls.borrow(), ["microsoft", "libreoffice"]);
    }

    #[test]
    fn an_installed_program_that_fails_is_never_worked_around_by_another() {
        let asked_next = std::cell::Cell::new(false);
        let result = first_available(
            &[
                &|| Err(ConvertError::CouldNotOpen("Microsoft Word")),
                &|| {
                    asked_next.set(true);
                    Ok(b"%PDF-".to_vec())
                },
            ],
            "all",
        );
        assert!(matches!(
            result,
            Err(ConvertError::CouldNotOpen("Microsoft Word"))
        ));
        assert!(
            !asked_next.get(),
            "a refusal by an installed program is the answer"
        );
    }

    #[test]
    fn with_no_program_installed_the_person_is_told_which_to_install() {
        let result = first_available(
            &[&|| missing("Microsoft Word"), &|| missing("LibreOffice")],
            "Microsoft Office or LibreOffice",
        );
        let error = result.unwrap_err();
        assert!(matches!(error, ConvertError::NoProgram(_)));
        assert_eq!(
            error.to_string(),
            "This preview needs Microsoft Office or LibreOffice, and none of them was found."
        );
    }

    #[test]
    fn a_program_without_a_macro_switch_is_given_only_files_that_cannot_run_one() {
        let package = |extra: &[u8]| {
            let mut bytes = b"PK\x03\x04 package ".to_vec();
            bytes.extend_from_slice(extra);
            bytes
        };
        assert!(cannot_run_a_macro("docx", &package(b"word/document.xml")));
        assert!(cannot_run_a_macro(
            "pptx",
            &package(b"ppt/slides/slide1.xml")
        ));
        assert!(cannot_run_a_macro("xlsx", &package(b"xl/workbook.xml")));
        // A macro project inside the package.
        assert!(!cannot_run_a_macro(
            "docx",
            &package(b"word/vbaProject.bin")
        ));
        // The legacy binary formats can carry macros.
        for extension in ["doc", "ppt", "xls"] {
            assert!(
                !cannot_run_a_macro(extension, &package(b"x")),
                "{extension}"
            );
        }
        // An .xlsx name on a legacy compound file, or on anything that is not a zip.
        assert!(!cannot_run_a_macro(
            "xlsx",
            &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]
        ));
        assert!(!cannot_run_a_macro("docx", b"plain text"));
    }

    #[test]
    fn a_path_becomes_the_file_url_libreoffice_expects() {
        assert_eq!(
            file_url("/tmp/a b/lo-profile"),
            "file:///tmp/a%20b/lo-profile"
        );
        assert_eq!(
            file_url(r"C:\Users\Jeevan TM\Temp\xfalcon-office-x\lo-profile"),
            "file:///C:/Users/Jeevan%20TM/Temp/xfalcon-office-x/lo-profile"
        );
        assert_eq!(file_url("/tmp/é#%"), "file:///tmp/%C3%A9%23%25");
    }

    #[test]
    fn libreoffice_runs_headless_in_its_own_profile_on_the_staged_copy() {
        let arguments: Vec<String> = libreoffice_arguments(
            "file:///tmp/p",
            Path::new("/tmp/out"),
            Path::new("/tmp/out/source.xlsx"),
        )
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
        assert!(arguments.contains(&"--headless".to_owned()));
        assert!(arguments.contains(&"-env:UserInstallation=file:///tmp/p".to_owned()));
        let convert = arguments.iter().position(|a| a == "--convert-to").unwrap();
        assert_eq!(arguments[convert + 1], "pdf");
        assert_eq!(arguments.last().unwrap(), "/tmp/out/source.xlsx");
        assert!(LIBREOFFICE_LOCKDOWN.contains("MacroSecurityLevel"));
        assert!(LIBREOFFICE_LOCKDOWN.contains("DisableMacrosExecution"));
    }

    #[test]
    fn a_source_over_the_limit_or_empty_is_refused_before_any_work() {
        let cancel = CancellationToken::new();
        assert!(matches!(
            convert_to_pdf(&[], "docx", &cancel),
            Err(ConvertError::InvalidOutput)
        ));
        let too_big = vec![0u8; (MAX_SOURCE_BYTES + 1) as usize];
        assert!(matches!(
            convert_to_pdf(&too_big, "docx", &cancel),
            Err(ConvertError::TooLarge)
        ));
        assert!(matches!(
            convert_to_pdf(b"data", "txt", &cancel),
            Err(ConvertError::Failed(_))
        ));
        cancel.cancel();
        assert!(matches!(
            convert_to_pdf(b"data", "docx", &cancel),
            Err(ConvertError::Cancelled)
        ));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn other_platforms_report_previews_unavailable() {
        assert!(matches!(
            convert_to_pdf(b"data", "docx", &CancellationToken::new()),
            Err(ConvertError::Unsupported)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_finished_process_reports_its_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 12"]);
        let finished = run_to_completion(
            command,
            &dir.path().join("office.pid"),
            None,
            &CancellationToken::new(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(finished.code, Some(12));
        assert!(finished.running.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_script_that_has_reported_is_not_waited_for_while_it_closes() {
        let parent = tempfile::tempdir().unwrap();
        let staging = Staging::create(parent.path(), b"data", "xlsx").unwrap();
        // Writes its result the way the Windows script does, then takes far
        // longer than the test to "close the program".
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                r#"printf 0 > "$1.tmp" && mv "$1.tmp" "$1"; sleep 60"#,
                "sh",
            ])
            .arg(&staging.done_file);
        let started = Instant::now();
        let finished = run_to_completion(
            command,
            &staging.pid_file,
            Some(&staging.done_file),
            &CancellationToken::new(),
            Duration::from_secs(60),
        )
        .unwrap();
        assert_eq!(finished.code, Some(0));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the result must not wait for the process to exit"
        );

        // The folder stays until the script is gone, then goes with it.
        let folder = staging.dir.path().to_path_buf();
        let child = finished.running.expect("the script is still running");
        finish_in_background(child, staging, Duration::from_millis(300));
        let deadline = Instant::now() + Duration::from_secs(10);
        while folder.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !folder.exists(),
            "the folder must be removed once the script has been stopped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_script_that_ends_on_its_own_after_reporting_leaves_nothing_behind() {
        let parent = tempfile::tempdir().unwrap();
        let staging = Staging::create(parent.path(), b"data", "xlsx").unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                r#"printf 0 > "$1.tmp" && mv "$1.tmp" "$1"; sleep 1"#,
                "sh",
            ])
            .arg(&staging.done_file);
        let finished = run_to_completion(
            command,
            &staging.pid_file,
            Some(&staging.done_file),
            &CancellationToken::new(),
            Duration::from_secs(30),
        )
        .unwrap();
        let folder = staging.dir.path().to_path_buf();
        finish_in_background(finished.running.unwrap(), staging, Duration::from_secs(30));
        let deadline = Instant::now() + Duration::from_secs(10);
        while folder.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!folder.exists());
    }

    #[test]
    fn only_a_whole_result_code_is_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("office.done");
        for (text, expected) in [
            ("0", Some(0)),
            ("12\r\n", Some(12)),
            ("", None),
            ("abc", None),
            ("1 2", None),
            ("123456789012345678901234567890", None),
        ] {
            fs::write(&file, text).unwrap();
            assert_eq!(read_done(&file), expected, "{text:?}");
        }
        assert_eq!(read_done(&dir.path().join("none")), None);
    }

    #[test]
    fn script_error_output_becomes_one_bounded_log_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("script.stderr");
        assert_eq!(script_error_summary(&file), None);

        fs::write(&file, "\n  \n").unwrap();
        assert_eq!(script_error_summary(&file), None);

        fs::write(
            &file,
            "2026-10-28 osascript[1:2] CFURLGetFSRef was passed a URL which has no scheme\n1100:1109: execution error: Microsoft PowerPoint got an error: AppleEvent timed out. (-1712)\n",
        )
        .unwrap();
        let summary = script_error_summary(&file).unwrap();
        assert!(summary.contains("(-1712)"), "{summary}");
        assert!(summary.contains(" | "), "{summary}");
        assert!(!summary.contains('\n'), "{summary}");

        // Multi-byte text cut at the limit must not split a character.
        fs::write(&file, "é".repeat(10_000)).unwrap();
        let summary = script_error_summary(&file).unwrap();
        assert!(summary.len() <= MAX_SCRIPT_ERROR_LOG_BYTES);
    }

    #[cfg(unix)]
    #[test]
    fn a_stuck_process_is_stopped_at_the_time_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30"]);
        let started = Instant::now();
        let result = run_to_completion(
            command,
            &dir.path().join("office.pid"),
            None,
            &CancellationToken::new(),
            Duration::from_millis(300),
        );
        assert!(matches!(result, Err(ConvertError::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_stops_the_process_and_the_office_program_it_reported() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("office.pid");
        // Stands in for the Office program the script started.
        let mut stand_in = Command::new("/bin/sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .unwrap();
        fs::write(&pid_file, stand_in.id().to_string()).unwrap();

        let cancel = CancellationToken::new();
        let canceller = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            canceller.cancel();
        });
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30"]);
        let result = run_to_completion(command, &pid_file, None, &cancel, Duration::from_secs(30));
        assert!(matches!(result, Err(ConvertError::Cancelled)));

        let deadline = Instant::now() + Duration::from_secs(5);
        let ended = loop {
            if let Some(status) = stand_in.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() > deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let _ = stand_in.kill();
        assert!(
            ended.is_some(),
            "the reported Office process must be stopped"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_windows_script_takes_its_inputs_from_the_environment() {
        let script = platform::script();
        for variable in [
            "XF_IN",
            "XF_OUT",
            "XF_KIND",
            "XF_PIDFILE",
            "XF_DONEFILE",
            "XF_PROGID",
            "XF_PROCESS",
        ] {
            assert!(script.contains(variable), "{variable}");
        }
        assert!(script.contains("AutomationSecurity = 3"));
    }

    /// A one-sheet workbook with synthetic cells, so Excel is covered without a
    /// binary fixture.
    #[cfg(windows)]
    fn synthetic_workbook() -> Vec<u8> {
        use zip::write::SimpleFileOptions;

        let parts: [(&str, &str); 5] = [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Region</t></is></c><c r="B1" t="inlineStr"><is><t>Revenue</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>North</t></is></c><c r="B2"><v>120</v></c></row></sheetData></worksheet>"#,
            ),
        ];
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, text) in parts {
            archive
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(text.as_bytes()).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    /// Needs Microsoft Excel on this machine. Excel can take a minute to shut
    /// down after a conversion on some machines; the PDF must not wait for it.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn native_excel_converts_a_workbook_without_waiting_for_it_to_close() {
        let started = Instant::now();
        let result = convert_to_pdf(&synthetic_workbook(), "xlsx", &CancellationToken::new());
        let took = started.elapsed();
        match result {
            Ok(pdf) => assert!(pdf.starts_with(b"%PDF-")),
            Err(ConvertError::NotInstalled(_)) => return,
            Err(error) => panic!("{error}"),
        }
        assert!(
            took < Duration::from_secs(45),
            "took {took:?}; the PDF must not wait for Excel to shut down"
        );
    }

    /// Needs Microsoft Office on Windows or macOS on this machine, so it is run
    /// by hand: `cargo test -p xfalcon-desktop --lib native_office -- --ignored`.
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    #[ignore]
    fn native_office_converts_every_supported_format() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/office");
        for (name, extension) in [
            ("producer.docx", "docx"),
            ("producer.pptx", "pptx"),
            ("legacy-poi/simple.doc", "doc"),
            ("legacy-poi/text_shapes.ppt", "ppt"),
        ] {
            let bytes = fs::read(fixtures.join(name)).unwrap();
            let result = convert_to_pdf(&bytes, extension, &CancellationToken::new());
            // A machine may block the legacy formats by policy; any other
            // outcome must be a real PDF or a specific, readable error.
            match result {
                Ok(pdf) => assert!(pdf.starts_with(b"%PDF-"), "{name}"),
                Err(error) => assert!(
                    matches!(error, ConvertError::CouldNotOpen(_)),
                    "{name}: {error}"
                ),
            }
        }
    }
}
