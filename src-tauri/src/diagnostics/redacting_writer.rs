//! Write-time redaction for the application's own log sinks.
//!
//! The `tracing` formatter writes one event through several `write` calls, so
//! a secret can straddle a boundary. [`RedactingWriter`] therefore buffers a
//! whole event and sanitizes it once, when it is dropped.

use super::{
    is_active_log_file, sanitize_complete_diagnostics_prefix, sanitize_diagnostics_text,
    LINE_CHUNK_BYTES,
};
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;
use tracing_subscriber::fmt::MakeWriter;

/// Most bytes one event buffers. Bytes past this are dropped, see [`RedactingWriter::write`].
const MAX_EVENT_BYTES: usize = 1024 * 1024;
/// Maximum raw bytes retained for one existing log line during startup cleanup.
const MAX_REDACTED_LINE_BYTES: u64 = LINE_CHUNK_BYTES * 2;

/// Appended to an event whose tail was dropped at [`MAX_EVENT_BYTES`].
const TRUNCATED_MARKER: &str = "…[event truncated]\n";

/// Wraps a [`MakeWriter`] so every event it writes is redacted on the way out.
pub(crate) struct RedactingMakeWriter<M> {
    inner: M,
}

impl<M> RedactingMakeWriter<M> {
    pub(crate) fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<'a, M> MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: for<'b> MakeWriter<'b> + Send + Sync + 'static,
{
    type Writer = RedactingWriter<<M as MakeWriter<'a>>::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter {
            inner: self.inner.make_writer(),
            buffer: Vec::new(),
            truncated: false,
        }
    }
}

/// Buffers one event's bytes and writes them to `inner` redacted.
pub(crate) struct RedactingWriter<W: Write> {
    inner: W,
    buffer: Vec<u8>,
    /// Set once the event outgrew [`MAX_EVENT_BYTES`] and its tail was dropped.
    truncated: bool,
}

impl<W: Write> RedactingWriter<W> {
    fn emit(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            // A capped event may end inside any supported credential syntax.
            // Keep only complete lines, then mark the discarded tail.
            let text = String::from_utf8_lossy(&self.buffer);
            let sanitized = if self.truncated {
                sanitize_complete_diagnostics_prefix(&text, self.buffer.len())
            } else {
                sanitize_diagnostics_text(&text)
            };
            self.buffer.clear();
            self.inner.write_all(sanitized.as_bytes())?;
            if self.truncated {
                self.inner.write_all(TRUNCATED_MARKER.as_bytes())?;
                self.truncated = false;
            }
        }
        self.inner.flush()
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    /// Always accepts the whole write, so the formatter never sees a short write. Once
    /// an event passes [`MAX_EVENT_BYTES`] its tail is dropped, and the event is marked
    /// truncated when it is emitted.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = MAX_EVENT_BYTES - self.buffer.len();
        if buf.len() > room {
            self.truncated = true;
        }
        self.buffer.extend_from_slice(&buf[..buf.len().min(room)]);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // Nothing is written before the event is complete: emitting here would
        // sanitize the two halves of a split secret separately. The event is
        // emitted on drop.
        Ok(())
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        // Nowhere to report a failed log write from inside `drop`.
        let _ = self.emit();
    }
}

/// Rewrite existing active log files in `logs_dir` with secrets redacted.
///
/// Returns how many files changed. Idempotent: a second run over the same
/// directory returns 0. Only files named like the active daily logs are
/// touched. Every such file is redacted whatever its size: it is streamed line
/// by line, with a fixed memory bound; an oversized line is discarded whole.
/// A file that is not valid UTF-8 is redacted from its lossy text
/// (each invalid byte becomes U+FFFD), and it is rewritten only when redaction
/// changed something. A file that needs no change keeps its bytes exactly.
///
/// Stale `*.redacting` temp files left by an interrupted pass are removed first.
/// Every active log is attempted even when one fails; the first error is
/// returned after the pass.
///
/// Callers must not run this while a [`RollingFileAppender`] holds an open
/// handle to one of these files: the replacement is a rename, and the handle
/// would keep writing to the unlinked old file.
///
/// [`RollingFileAppender`]: tracing_appender::rolling::RollingFileAppender
pub(crate) fn redact_existing_logs(logs_dir: &Path) -> io::Result<usize> {
    let entries = match fs::read_dir(logs_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };

    let mut first_error = None;
    let mut log_names = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                keep_first_error(&mut first_error, error);
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().to_string();
        // A directory that happens to have a temp file's name is not ours to remove.
        if is_stale_temp_file(&name) && entry.path().is_file() {
            if let Err(error) = fs::remove_file(entry.path()) {
                keep_first_error(&mut first_error, error);
            }
        } else if is_active_log_file(&name) {
            log_names.push(name);
        }
    }

    let mut changed = 0;
    for name in &log_names {
        match redact_log_file(logs_dir, name) {
            Ok(true) => changed += 1,
            Ok(false) => {}
            Err(error) => keep_first_error(&mut first_error, error),
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(changed),
    }
}

/// Redact one active log in place. Returns `Ok(true)` when it was rewritten.
fn redact_log_file(logs_dir: &Path, name: &str) -> io::Result<bool> {
    let path = logs_dir.join(name);
    if !fs::metadata(&path)?.is_file() {
        return Ok(false);
    }

    // The redacted copy replaces the log only when redaction changed something.
    // A crash can leave a truncated multi-byte tail, which is judged in its lossy
    // form like the rest, so a clean file keeps its bytes.
    let temp_path = logs_dir.join(format!("{name}.redacting"));
    let changed = match write_redacted_copy(&path, &temp_path) {
        Ok(changed) => changed,
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
    };
    if !changed {
        fs::remove_file(&temp_path)?;
        return Ok(false);
    }
    if let Err(error) = fs::rename(&temp_path, &path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(true)
}

/// Streams `source` into `dest`, redacting complete lines up to a fixed bound.
/// Oversized lines are drained and discarded as a whole, so a credential cut
/// at the bound cannot be followed by raw text.
/// Returns whether any piece changed.
fn write_redacted_copy(source: &Path, dest: &Path) -> io::Result<bool> {
    let mut reader = BufReader::new(fs::File::open(source)?);
    let mut writer = BufWriter::new(fs::File::create(dest)?);
    let mut piece = Vec::new();
    let mut changed = false;
    loop {
        piece.clear();
        let read = (&mut reader)
            .take(MAX_REDACTED_LINE_BYTES)
            .read_until(b'\n', &mut piece)?;
        if read == 0 {
            break;
        }
        let oversized =
            piece.len() as u64 == MAX_REDACTED_LINE_BYTES && piece.last() != Some(&b'\n');
        if oversized {
            drain_through_newline(&mut reader)?;
        }
        if oversized {
            changed = true;
            writer.write_all("…[line truncated]\n".as_bytes())?;
        } else {
            let text = String::from_utf8_lossy(&piece);
            let sanitized = sanitize_diagnostics_text(&text);
            changed |= sanitized != text.as_ref();
            writer.write_all(sanitized.as_bytes())?;
        }
    }
    writer.flush()?;
    Ok(changed)
}

/// Discard the remainder of an oversized line without buffering it.
fn drain_through_newline(reader: &mut BufReader<fs::File>) -> io::Result<()> {
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(());
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        let finished = available.get(consumed.saturating_sub(1)) == Some(&b'\n');
        reader.consume(consumed);
        if finished {
            return Ok(());
        }
    }
}

/// `{active log name}.redacting`: the temp file a pass writes before its rename.
fn is_stale_temp_file(name: &str) -> bool {
    name.strip_suffix(".redacting")
        .is_some_and(is_active_log_file)
}

fn keep_first_error(first_error: &mut Option<io::Error>, error: io::Error) {
    if first_error.is_none() {
        *first_error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;
    use tracing_subscriber::layer::SubscriberExt as _;

    const JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";

    /// A `MakeWriter` whose writers append to one shared file.
    fn file_maker(path: &Path) -> impl for<'b> MakeWriter<'b> + Send + Sync + 'static {
        let path = path.to_path_buf();
        move || {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .expect("open log file")
        }
    }

    #[test]
    fn secret_split_across_writes_is_redacted() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("out.log");
        let maker = RedactingMakeWriter::new(file_maker(&path));
        {
            let mut writer = maker.make_writer();
            writer.write_all(b"auth=Bear").expect("write");
            writer
                .write_all(b"er abcdefghijklmnopqrstuvwxyz0123456789 end\n")
                .expect("write");
        }
        let written = fs::read_to_string(&path).expect("read");
        assert!(!written.contains("abcdefghijklmnopqrstuvwxyz0123456789"));
        assert_eq!(written, "auth=Bearer [REDACTED_BEARER_TOKEN] end\n");
    }

    #[test]
    fn invalid_utf8_does_not_panic_and_is_lossy_replaced() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("out.log");
        let maker = RedactingMakeWriter::new(file_maker(&path));
        {
            let mut writer = maker.make_writer();
            writer.write_all(b"ok \xff\xfe bytes\n").expect("write");
        }
        let written = fs::read(&path).expect("read");
        assert_eq!(written, "ok \u{fffd}\u{fffd} bytes\n".as_bytes());
    }

    #[test]
    fn write_reports_full_length_consumed() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("out.log");
        let maker = RedactingMakeWriter::new(file_maker(&path));
        let mut writer = maker.make_writer();
        assert_eq!(writer.write(b"hello").expect("write"), 5);
    }

    #[test]
    fn output_equals_sanitized_input() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("out.log");
        let input = "INFO sign-in user=alice@example.com path=/home/alice/x.log\n";
        let maker = RedactingMakeWriter::new(file_maker(&path));
        {
            let mut writer = maker.make_writer();
            writer.write_all(input.as_bytes()).expect("write");
        }
        let written = fs::read_to_string(&path).expect("read");
        assert_eq!(written, sanitize_diagnostics_text(input));
    }

    #[test]
    fn event_is_timestamped_redacted_and_keeps_its_message() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("events.log");
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(RedactingMakeWriter::new(file_maker(&path)))
                .with_ansi(false),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                authorization = "Bearer abcdefghijklmnopqrstuvwxyz0123456789",
                who = "alice@example.com",
                "sign-in"
            );
        });

        let written = fs::read_to_string(&path).expect("read");
        let timestamp = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z").expect("regex");
        assert!(timestamp.is_match(&written), "no timestamp: {written}");
        assert!(!written.contains("abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(!written.contains("alice@example.com"));
        assert!(written.contains("sign-in"));
        assert!(written.contains("[REDACTED_EMAIL]"));
    }

    #[test]
    fn redact_existing_logs_rewrites_only_dirty_active_logs() {
        let temp = tempdir().expect("tempdir");
        let dir = temp.path();

        let dirty = dir.join("xfalcon-desktop.2026-10-12.log");
        fs::write(&dirty, format!("INFO jwt={JWT} who=bob@example.org\n")).expect("write");

        let clean_name = dir.join("xfalcon-desktop.2026-10-13.log");
        let clean =
            "2026-10-13T01:00:00.000000Z INFO run id=123e4567-e89b-12d3-a456-426614174000\n";
        fs::write(&clean_name, clean).expect("write");

        let other = dir.join("notes.txt");
        let other_content = "bob@example.org and Bearer abcdefghijklmnopqrstuvwxyz0123456789\n";
        fs::write(&other, other_content).expect("write");

        assert_eq!(redact_existing_logs(dir).expect("redact"), 1);

        let rewritten = fs::read_to_string(&dirty).expect("read");
        assert!(!rewritten.contains(JWT));
        assert!(!rewritten.contains("bob@example.org"));
        assert!(rewritten.contains("[REDACTED_JWT]"));
        assert!(rewritten.contains("[REDACTED_EMAIL]"));
        assert_eq!(fs::read_to_string(&clean_name).expect("read"), clean);
        assert_eq!(fs::read_to_string(&other).expect("read"), other_content);
        assert!(!dir
            .join("xfalcon-desktop.2026-10-12.log.redacting")
            .exists());

        assert_eq!(redact_existing_logs(dir).expect("redact again"), 0);
    }

    #[test]
    fn a_flush_between_writes_does_not_emit_half_an_event() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("out.log");
        let maker = RedactingMakeWriter::new(file_maker(&path));
        let mut writer = maker.make_writer();
        writer.write_all(b"auth=Bear").expect("write");
        writer.flush().expect("flush");
        assert_eq!(fs::read_to_string(&path).expect("read"), "");
        writer
            .write_all(b"er abcdefghijklmnopqrstuvwxyz0123456789 end\n")
            .expect("write");
        drop(writer);
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            "auth=Bearer [REDACTED_BEARER_TOKEN] end\n"
        );
    }

    #[test]
    fn one_failing_log_does_not_stop_the_others_and_its_error_is_returned() {
        let temp = tempdir().expect("tempdir");
        let dir = temp.path();
        let first = dir.join("xfalcon-desktop.2026-10-10.log");
        let second = dir.join("xfalcon-desktop.2026-10-11.log");
        let third = dir.join("xfalcon-desktop.2026-10-12.log");
        fs::write(&first, "a bob@example.org\n").expect("write");
        fs::write(&second, "b carol@example.org\n").expect("write");
        fs::write(&third, "c dave@example.org\n").expect("write");
        // A directory at the second log's temp path makes its rewrite fail.
        fs::create_dir(dir.join("xfalcon-desktop.2026-10-11.log.redacting")).expect("mkdir");

        assert!(redact_existing_logs(dir).is_err());

        assert_eq!(
            fs::read_to_string(&first).expect("read"),
            "a [REDACTED_EMAIL]\n"
        );
        assert_eq!(
            fs::read_to_string(&third).expect("read"),
            "c [REDACTED_EMAIL]\n"
        );
        assert_eq!(
            fs::read_to_string(&second).expect("read"),
            "b carol@example.org\n"
        );
    }

    #[test]
    fn a_clean_log_with_a_cut_off_tail_keeps_its_bytes() {
        let temp = tempdir().expect("tempdir");
        let dir = temp.path();
        let log = dir.join("xfalcon-desktop.2026-10-12.log");
        // The final multi-byte sequence is cut off, as after a crash mid-write.
        let original = b"nothing secret here \xe2\x82\n".to_vec();
        fs::write(&log, &original).expect("write");

        assert_eq!(redact_existing_logs(dir).expect("redact"), 0);

        assert_eq!(fs::read(&log).expect("read"), original);
        assert!(!dir
            .join("xfalcon-desktop.2026-10-12.log.redacting")
            .exists());
    }

    #[test]
    fn a_secret_in_a_log_with_a_cut_off_tail_is_still_redacted() {
        let temp = tempdir().expect("tempdir");
        let dir = temp.path();
        let log = dir.join("xfalcon-desktop.2026-10-12.log");
        fs::write(&log, b"bob@example.org \xe2\x82 truncated\n").expect("write");

        assert_eq!(redact_existing_logs(dir).expect("redact"), 1);

        let text = fs::read_to_string(&log).expect("rewritten as valid UTF-8");
        assert!(!text.contains("bob@example.org"), "{text}");
        assert!(text.contains("[REDACTED_EMAIL]"), "{text}");
    }

    #[test]
    fn stale_temp_files_from_an_interrupted_pass_are_pruned() {
        let temp = tempdir().expect("tempdir");
        let dir = temp.path();
        let stale = dir.join("xfalcon-desktop.2026-10-12.log.redacting");
        let unrelated = dir.join("notes.log.redacting");
        fs::write(&stale, "half written").expect("write");
        fs::write(&unrelated, "not ours").expect("write");

        assert_eq!(redact_existing_logs(dir).expect("redact"), 0);

        assert!(!stale.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn redact_existing_logs_on_missing_directory_is_a_no_op() {
        let temp = tempdir().expect("tempdir");
        assert_eq!(
            redact_existing_logs(&temp.path().join("absent")).expect("redact"),
            0
        );
    }

    #[test]
    fn consecutive_events_are_written_independently() {
        let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = buffer.clone();
        let maker = move || SharedSink(sink.clone());
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(RedactingMakeWriter::new(maker))
                .with_ansi(false),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("first");
            tracing::info!("second");
        });
        let text = String::from_utf8(buffer.lock().expect("lock").clone()).expect("utf8");
        assert_eq!(text.lines().count(), 2);
        assert!(text
            .lines()
            .all(|line| line.ends_with("first") || line.ends_with("second")));
    }

    #[test]
    fn an_oversized_event_is_cut_at_the_cap_and_marked() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("out.log");
        let maker = RedactingMakeWriter::new(file_maker(&path));
        {
            let mut writer = maker.make_writer();
            writer.write_all(b"password=hunter2\n").expect("write");
            writer.write_all(b"earlier event context\n").expect("write");
            let filler = [b'x'; 4096];
            for _ in 0..MAX_EVENT_BYTES / filler.len() {
                writer.write_all(&filler).expect("write");
            }
            writer.write_all(b" token=tail-secret\n").expect("write");
        }
        let written = fs::read_to_string(&path).expect("read");
        assert!(written.starts_with("password=[REDACTED]\nearlier event context\n"));
        assert!(written.ends_with("…[event truncated]\n"));
        assert!(!written.contains("hunter2"));
        assert!(!written.contains("tail-secret"));
        assert!(
            written.len() <= MAX_EVENT_BYTES + 64,
            "len {}",
            written.len()
        );
    }

    #[test]
    fn capped_events_discard_incomplete_supported_secret_forms() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("out.log");
        let maker = RedactingMakeWriter::new(file_maker(&path));
        for (field, canary) in [
            ("\npassword = \"", "plain-cap-canary-long-secret"),
            ("\n{\"password\":\"", "json-cap-canary-long-secret"),
            ("\n{\\\"password\\\":\\\"", "escaped-cap-canary-long-secret"),
            ("\ntoken=Some(\"", "constructor-cap-canary-long-secret"),
        ] {
            let mut writer = maker.make_writer();
            let start_of_value = MAX_EVENT_BYTES - 16;
            let context = b"earlier event context\n";
            let mut filler = context.to_vec();
            filler.resize(start_of_value - field.len(), b'x');
            writer.write_all(&filler).expect("write filler");
            writer
                .write_all(field.as_bytes())
                .expect("write field prefix");
            writer
                .write_all(&canary.as_bytes()[..16])
                .expect("write partial secret");
            writer
                .write_all(&canary.as_bytes()[16..])
                .expect("write overflow secret");
            writer
                .write_all(b"\"\nnext event context\n")
                .expect("overflow event");
        }

        let written = fs::read_to_string(&path).expect("read");
        for canary in [
            "plain-cap-canary-long-secret",
            "json-cap-canary-long-secret",
            "escaped-cap-canary-long-secret",
            "constructor-cap-canary-long-secret",
        ] {
            assert!(!written.contains(canary));
            assert!(!written.contains(&canary[..16]));
        }
        assert_eq!(written.matches("…[event truncated]\n").count(), 4);
        assert_eq!(written.matches("earlier event context\n").count(), 4);
        assert!(!written.contains("next event context"));
    }

    #[test]
    fn capped_event_with_a_split_utf8_character_stays_valid() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("out.log");
        let maker = RedactingMakeWriter::new(file_maker(&path));
        {
            let mut writer = maker.make_writer();
            let mut bytes = b"earlier event context\n".to_vec();
            bytes.resize(MAX_EVENT_BYTES - 2, b'x');
            bytes.extend_from_slice("🎉secret-tail".as_bytes());
            writer.write_all(&bytes).expect("write capped event");
            writer.write_all(b"\n").expect("write overflow");
        }

        let written = fs::read_to_string(&path).expect("rewritten log is valid UTF-8");
        assert!(written.starts_with("earlier event context\n"));
        assert!(written.ends_with("…[event truncated]\n"));
        assert!(!written.contains("secret-tail"));
    }

    #[test]
    fn an_event_under_the_cap_is_not_marked() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("out.log");
        let maker = RedactingMakeWriter::new(file_maker(&path));
        {
            let mut writer = maker.make_writer();
            writer.write_all(b"token=abc end\n").expect("write");
        }
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            "token=[REDACTED] end\n"
        );
    }

    #[test]
    fn startup_redaction_streams_long_lines_and_redacts_every_line() {
        let temp = tempdir().expect("tempdir");
        let dir = temp.path();
        let dirty = dir.join("xfalcon-desktop.2026-10-12.log");
        // The email sits past the first piece of a line longer than LINE_CHUNK_BYTES.
        // Secrets sit on the first, middle and last lines.
        let long_line = format!(
            "{} carol@example.org\n",
            "y".repeat(LINE_CHUNK_BYTES as usize + 10)
        );
        let content = format!(
            "first alice@example.com\nmiddle token=abc123 here\n{long_line}last DB_PASSWORD=hunter2\n"
        );
        fs::write(&dirty, &content).expect("write");

        let clean = dir.join("xfalcon-desktop.2026-10-13.log");
        let clean_content = format!("{}\n", "z".repeat(LINE_CHUNK_BYTES as usize + 10));
        fs::write(&clean, &clean_content).expect("write");

        assert_eq!(redact_existing_logs(dir).expect("redact"), 1);

        let rewritten = fs::read_to_string(&dirty).expect("read");
        for secret in [
            "alice@example.com",
            "abc123",
            "carol@example.org",
            "hunter2",
        ] {
            assert!(!rewritten.contains(secret), "{secret} survived");
        }
        assert!(rewritten.contains("token=[REDACTED] here\n"));
        assert!(rewritten.contains("DB_PASSWORD=[REDACTED]\n"));
        assert!(rewritten.contains("[REDACTED_EMAIL]"));
        assert_eq!(fs::read(&clean).expect("read"), clean_content.as_bytes());
        assert!(!dir
            .join("xfalcon-desktop.2026-10-13.log.redacting")
            .exists());
    }

    #[test]
    fn startup_redaction_discards_oversized_partial_fields_and_keeps_neighbors() {
        let temp = tempdir().expect("tempdir");
        let dir = temp.path();
        let log = dir.join("xfalcon-desktop.2026-10-12.log");
        let cases = [
            (" password = \"", "plain-startup-oversized-secret"),
            (" {\"password\":\"", "json-startup-oversized-secret"),
            (
                " {\\\"password\\\":\\\"",
                "escaped-startup-oversized-secret",
            ),
            (" token=Some(\"", "constructor-startup-oversized-secret"),
        ];
        let mut content = b"earlier context survives\n".to_vec();
        for (field, canary) in cases {
            let mut line = b"oversized diagnostic".to_vec();
            line.resize(MAX_REDACTED_LINE_BYTES as usize - 16 - field.len(), b'x');
            line.extend_from_slice(field.as_bytes());
            line.extend_from_slice(&canary.as_bytes()[..16]);
            assert_eq!(line.len(), MAX_REDACTED_LINE_BYTES as usize);
            line.extend_from_slice(&canary.as_bytes()[16..]);
            line.extend_from_slice(b"\"\n");
            content.extend_from_slice(&line);
            content.extend_from_slice(b"following context survives\n");
        }

        // The cut lands in the middle of a multibyte character. The entire
        // oversized line must be discarded without decoding or panicking.
        let mut utf8_line = b"utf8 boundary".to_vec();
        utf8_line.resize(MAX_REDACTED_LINE_BYTES as usize - 2, b'u');
        utf8_line.extend_from_slice("🎉utf8-tail-secret".as_bytes());
        utf8_line.push(b'\n');
        content.extend_from_slice(&utf8_line);
        content.extend_from_slice(b"final context survives\n");
        fs::write(&log, content).expect("write log");

        assert_eq!(redact_existing_logs(dir).expect("redact"), 1);

        let rewritten = fs::read_to_string(&log).expect("read rewritten log");
        for canary in [
            "plain-startup-oversized-secret",
            "json-startup-oversized-secret",
            "escaped-startup-oversized-secret",
            "constructor-startup-oversized-secret",
            "utf8-tail-secret",
        ] {
            assert!(!rewritten.contains(canary));
        }
        assert_eq!(rewritten.matches("…[line truncated]\n").count(), 5);
        assert!(rewritten.contains("earlier context survives\n"));
        assert_eq!(rewritten.matches("following context survives\n").count(), 4);
        assert!(rewritten.contains("final context survives\n"));
    }

    #[derive(Clone)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
