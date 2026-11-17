//! Redacted panic hook boundary.
//!
//! Panics append and synchronize structured crash diagnostics (location,
//! thread, backtrace) to the daily log file while completely redacting and
//! discarding the payload to prevent leaking decrypted content, auth tokens,
//! or paths (CWE-532).

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const REDACTED_PANIC_MESSAGE: &str = "xfalcon: internal failure";

static PANIC_LOG_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

/// Format structured panic record string, ensuring payload is never included.
pub fn format_panic_record(location: &str, thread_name: &str, backtrace: &str) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    format!("{now} ERROR panic occurred: {REDACTED_PANIC_MESSAGE} at {location} in thread '{thread_name}'\n{backtrace}\n")
}

/// Set the directory used by the panic hook once app-local data is known.
pub fn configure_panic_log_directory(directory: &Path) -> io::Result<()> {
    if let Some(configured) = PANIC_LOG_DIRECTORY.get() {
        return if configured == directory {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "panic log directory already configured",
            ))
        };
    }
    PANIC_LOG_DIRECTORY
        .set(directory.to_path_buf())
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "panic log directory already configured",
            )
        })
}

/// Append and sync one panic record without relying on the asynchronous
/// tracing writer, whose queue can be lost when release panic handling aborts.
pub fn write_panic_record_to_directory(directory: &Path, record: &str) -> io::Result<()> {
    let filename = format!(
        "xfalcon-desktop.{}.log",
        chrono::Utc::now().format("%Y-%m-%d")
    );
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join(filename))?;
    file.write_all(record.as_bytes())?;
    file.sync_data()
}

pub fn install_redacted_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");
        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let backtrace = std::backtrace::Backtrace::capture();
        let backtrace_str = format!("{backtrace}");

        let record = format_panic_record(&location, thread_name, &backtrace_str);
        let persisted = PANIC_LOG_DIRECTORY
            .get()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "panic log directory unavailable")
            })
            .and_then(|directory| write_panic_record_to_directory(directory, &record));
        if let Err(error) = persisted {
            eprintln!("xfalcon: failed to persist panic record: {error}");
        }
        eprintln!("{record}");
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redacted_panic_constant_is_fixed() {
        assert_eq!(REDACTED_PANIC_MESSAGE, "xfalcon: internal failure");
    }

    #[test]
    fn test_panic_record_names_location_thread_and_fixed_redacted_message() {
        let location = "src-tauri/src/main.rs:42:10";
        let thread = "tokio-worker-1";
        let backtrace = "   0: std::backtrace::Backtrace::capture\n   1: xfalcon_desktop::test";
        let record = format_panic_record(location, thread, backtrace);

        assert!(record.contains(REDACTED_PANIC_MESSAGE));
        assert!(record.contains(location));
        assert!(record.contains(thread));
        assert!(record.contains("std::backtrace"));
    }

    #[test]
    fn test_panic_record_is_synchronously_persisted_to_the_daily_log() {
        let temp = tempfile::tempdir().expect("tempdir");
        let record = format_panic_record("src/main.rs:7:9", "panic-test", "test backtrace");

        write_panic_record_to_directory(temp.path(), &record).expect("persist panic record");

        let filename = format!(
            "xfalcon-desktop.{}.log",
            chrono::Utc::now().format("%Y-%m-%d")
        );
        let persisted =
            std::fs::read_to_string(temp.path().join(filename)).expect("read panic log");
        assert_eq!(persisted, record);
    }
}
