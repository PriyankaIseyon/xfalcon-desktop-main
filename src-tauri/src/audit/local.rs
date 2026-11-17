//! Local, tamper-evident, bounded security audit log.
//!
//! Each event is one JSON line in a segment file under `<app-local-data>/audit`.
//! Every record carries a keyed BLAKE3 MAC over its format version, sequence,
//! timestamp, the previous record's MAC and its own event, so the log forms a
//! chain. The MAC key lives in the OS keychain (service `ai.xfalcon.desktop`,
//! account `device-audit-chain`).
//!
//! # What `verify` proves
//!
//! - Changing a record's content, version, sequence, timestamp or links,
//!   reordering records, or removing a record from the middle of the retained
//!   log is detected by anyone who does not hold the keychain key.
//! - Removing records from the start of a segment is detected too: each
//!   segment's file name is the sequence of its first record.
//! - A record that fails to parse or to chain is reported with its sequence
//!   number and a reason. The report never includes event content.
//!
//! # What `verify` does not prove
//!
//! - Removing the newest records is not detected: the chain has no external
//!   anchor for its head.
//! - Someone who holds the keychain key can rebuild the whole log so that it
//!   verifies. The key is the trust root; this log does not defend against
//!   the account's own machine.
//! - Retention pruning removes the oldest segments. The first retained record
//!   then anchors the chain to a deleted predecessor, which is reported as
//!   `anchored_after_prune`, not as tampering. Only a first record whose
//!   sequence matches its segment's file name, and whose link is not the zero
//!   link, is accepted that way.
//! - Bytes after the final newline (a write that never finished) are truncated
//!   when the log is opened. That repair is reported as
//!   `torn_tail_repaired`, but it is indistinguishable from a deliberate
//!   truncation of the last record. A complete line that does not parse is
//!   never removed: it makes the log damaged.
//! - Opening parses only the newest segment. A damaged newest segment (a
//!   complete line that does not parse) is set aside by
//!   [`LocalAuditStore::open_or_quarantine`] and a fresh chain starts. Damage
//!   in an older segment is found by `verify()`, which checks every retained
//!   segment. An I/O failure is not damage: nothing is set aside, auditing is
//!   off for the session, and the next launch retries with the files
//!   untouched. A log that opens but fails `verify` is not set aside; it is
//!   reported.
//!
//! The log is content-free: it stores identifiers, actions and outcomes only.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::AuditEvent;
use crate::domain::error::{AppError, AppResult};
use crate::persistence::SecretStore;

const AUDIT_KEY_ACCOUNT: &str = "device-audit-chain";
const DEVICE_ID_FILE: &str = "device-id";
pub(super) const FORMAT_VERSION: u32 = 1;
const SEGMENT_PREFIX: &str = "audit-";
const SEGMENT_SUFFIX: &str = ".jsonl";
const SEGMENT_NUMBER_DIGITS: usize = 20;
const MILLIS_PER_DAY: i64 = 86_400_000;
const SEGMENT_MAX_BYTES: u64 = 4 * 1024 * 1024;
const TOTAL_MAX_BYTES: u64 = 128 * 1024 * 1024;
const RETENTION_DAYS: i64 = 400;

/// Rollover and pruning limits. Tests override these; production uses
/// [`Limits::default`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub(crate) segment_max_bytes: u64,
    pub(crate) total_max_bytes: u64,
    pub(crate) retention_days: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            segment_max_bytes: SEGMENT_MAX_BYTES,
            total_max_bytes: TOTAL_MAX_BYTES,
            retention_days: RETENTION_DAYS,
        }
    }
}

/// One line of a segment file.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditRecord {
    v: u32,
    seq: u64,
    ts_ms: i64,
    /// 64 lower-hex characters: the previous record's MAC (zeros for the
    /// first record ever written).
    prev: String,
    /// 64 lower-hex characters.
    mac: String,
    event: AuditEvent,
}

impl AuditRecord {
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    #[must_use]
    pub const fn ts_ms(&self) -> i64 {
        self.ts_ms
    }

    #[must_use]
    pub const fn event(&self) -> &AuditEvent {
        &self.event
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationReport {
    pub ok: bool,
    pub records: u64,
    pub segments: usize,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    pub first_ts_ms: Option<i64>,
    pub last_ts_ms: Option<i64>,
    pub anchored_after_prune: bool,
    pub torn_tail_repaired: bool,
    pub problem: Option<String>,
}

impl VerificationReport {
    const fn empty(segments: usize, torn_tail_repaired: bool) -> Self {
        Self {
            ok: true,
            records: 0,
            segments,
            first_seq: None,
            last_seq: None,
            first_ts_ms: None,
            last_ts_ms: None,
            anchored_after_prune: false,
            torn_tail_repaired,
            problem: None,
        }
    }

    fn fail(mut self, problem: String) -> Self {
        self.ok = false;
        self.problem = Some(problem);
        self
    }
}

/// The chain head and the segment currently receiving appends.
struct ChainState {
    last_seq: u64,
    last_mac: [u8; 32],
    active: Option<ActiveSegment>,
    torn_tail_repaired: bool,
}

struct ActiveSegment {
    path: PathBuf,
    bytes: u64,
    month: (i32, u32),
}

struct SegmentFile {
    path: PathBuf,
    bytes: u64,
    /// The sequence number in the file name, which is the sequence of the
    /// segment's first record.
    first_seq: u64,
}

/// Why a log could not be opened. Only `Damaged` sets the log aside.
enum OpenFailure {
    /// The content is structurally wrong: a complete line that does not parse.
    Damaged(AppError),
    /// A read, write, listing or the key store failed. The log may be intact.
    Io(AppError),
}

impl From<OpenFailure> for AppError {
    fn from(failure: OpenFailure) -> Self {
        match failure {
            OpenFailure::Damaged(error) | OpenFailure::Io(error) => error,
        }
    }
}

fn damaged() -> OpenFailure {
    OpenFailure::Damaged(AppError::PersistenceInvalid("audit record is malformed"))
}

fn io_failure(error: io::Error) -> OpenFailure {
    OpenFailure::Io(storage_error(error))
}

pub struct LocalAuditStore {
    root: PathBuf,
    key: Zeroizing<[u8; 32]>,
    device_id: String,
    limits: Limits,
    recovered_from_damage: bool,
    state: Mutex<ChainState>,
}

/// How many set-aside damaged logs are kept next to the live one.
const MAX_DAMAGED_SIBLINGS: usize = 3;
const DAMAGED_MARKER: &str = ".damaged-";

impl LocalAuditStore {
    /// Opens (or creates) the log under `root`, which is the
    /// `<app-local-data>/audit` directory.
    pub fn open(root: PathBuf, secrets: Arc<dyn SecretStore>) -> AppResult<Self> {
        Self::open_with_limits(root, secrets, Limits::default())
    }

    /// Like [`Self::open`], but a damaged log (a complete line that does not
    /// parse) is set aside as `<root>.damaged-<unix seconds>` and a fresh log
    /// is started in `root`. An I/O or keychain failure is returned and never
    /// sets the log aside: the files stay as they were and the next open
    /// retries.
    pub fn open_or_quarantine(root: PathBuf, secrets: Arc<dyn SecretStore>) -> AppResult<Self> {
        Self::open_or_quarantine_with_limits(root, secrets, Limits::default())
    }

    pub(crate) fn open_or_quarantine_with_limits(
        root: PathBuf,
        secrets: Arc<dyn SecretStore>,
        limits: Limits,
    ) -> AppResult<Self> {
        match Self::open_classified(root.clone(), secrets.clone(), limits) {
            Ok(store) => Ok(store),
            Err(OpenFailure::Io(error)) => Err(error),
            Err(OpenFailure::Damaged(error)) => {
                // The id belongs to the install, not to the damaged log, so it
                // moves with the fresh log rather than being set aside with it.
                let device_id = read_device_id(&root);
                if set_aside(&root).is_err() {
                    return Err(error);
                }
                if let Some(id) = device_id {
                    restore_device_id(&root, &id);
                }
                let mut store =
                    Self::open_classified(root, secrets, limits).map_err(AppError::from)?;
                store.recovered_from_damage = true;
                Ok(store)
            }
        }
    }

    /// True when this store was started after the previous log was set aside
    /// as damaged.
    #[must_use]
    pub const fn recovered_from_damage(&self) -> bool {
        self.recovered_from_damage
    }

    pub(crate) fn open_with_limits(
        root: PathBuf,
        secrets: Arc<dyn SecretStore>,
        limits: Limits,
    ) -> AppResult<Self> {
        Self::open_classified(root, secrets, limits).map_err(AppError::from)
    }

    /// Opens the log and says why when it cannot. Reads and checks everything
    /// first and writes last, so an I/O failure leaves every file as it was.
    fn open_classified(
        root: PathBuf,
        secrets: Arc<dyn SecretStore>,
        limits: Limits,
    ) -> Result<Self, OpenFailure> {
        fs::create_dir_all(&root).map_err(io_failure)?;
        let (_, key) = secrets
            .load_or_create_active_key(AUDIT_KEY_ACCOUNT)
            .map_err(OpenFailure::Io)?;
        let device_id = load_or_create_device_id(&root)?;

        let mut torn_tail_repaired = false;
        let mut truncate = None;
        let mut removals = Vec::new();
        let mut newest = None;
        for segment in list_segments(&root)
            .map_err(OpenFailure::Io)?
            .into_iter()
            .rev()
        {
            let mut bytes = fs::read(&segment.path).map_err(io_failure)?;
            let kept = complete_len(&bytes);
            torn_tail_repaired |= kept < bytes.len();
            if kept == 0 {
                removals.push(segment.path);
                continue;
            }
            if kept < bytes.len() {
                truncate = Some((segment.path.clone(), kept as u64));
            }
            bytes.truncate(kept);
            newest = Some((segment.path, bytes));
            break;
        }

        let mut state = ChainState {
            last_seq: 0,
            last_mac: [0; 32],
            active: None,
            torn_tail_repaired,
        };
        if let Some((path, bytes)) = newest {
            // Every complete line of the newest segment must parse. A bad
            // line is damage and is kept as evidence, never repaired away.
            let lines = split_lines(&bytes);
            let records: Vec<AuditRecord> = lines
                .iter()
                .map(|line| parse_line(line))
                .collect::<Option<_>>()
                .ok_or_else(damaged)?;
            let first = records.first().ok_or_else(damaged)?;
            let last = records.last().ok_or_else(damaged)?;
            state.last_seq = last.seq;
            state.last_mac = from_hex(&last.mac).ok_or_else(damaged)?;
            // Only a segment whose first record has a valid timestamp can keep
            // receiving appends; otherwise the next append starts a fresh
            // segment.
            state.active = month_key(first.ts_ms).ok().map(|month| ActiveSegment {
                path,
                bytes: bytes.len() as u64,
                month,
            });
        }

        // Only now are files changed. Truncation removes bytes after the final
        // newline; removal deletes segments that held no complete record.
        if let Some((path, len)) = truncate {
            truncate_segment(&path, len).map_err(io_failure)?;
        }
        for path in removals {
            fs::remove_file(&path).map_err(io_failure)?;
        }

        Ok(Self {
            root,
            key,
            device_id,
            limits,
            recovered_from_damage: false,
            state: Mutex::new(state),
        })
    }

    /// The stable per-install device id (a random UUID v4 stored in
    /// `root/device-id`).
    #[must_use]
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Appends one event as the next record in the chain. The whole
    /// read-head, compute, write and flush path runs under one lock, so
    /// concurrent callers keep the chain valid.
    pub fn append(&self, event: AuditEvent) -> AppResult<()> {
        let mut state = self.lock_state()?;
        let seq = state
            .last_seq
            .checked_add(1)
            .ok_or(AppError::AuditStorageFailed)?;
        let ts_ms = event.occurred_at_ms();
        let month = month_key(ts_ms)?;
        let event_json = serde_json::to_vec(&event).map_err(|_| AppError::AuditStorageFailed)?;
        let prev = state.last_mac;
        let mac = compute_mac(&self.key, FORMAT_VERSION, seq, ts_ms, &prev, &event_json);
        let record = AuditRecord {
            v: FORMAT_VERSION,
            seq,
            ts_ms,
            prev: to_hex(&prev),
            mac: to_hex(mac.as_bytes()),
            event,
        };
        let mut line = serde_json::to_vec(&record).map_err(|_| AppError::AuditStorageFailed)?;
        line.push(b'\n');

        let (path, previous_len, create) = match state.active.as_ref() {
            Some(active) if !self.needs_rollover(active, month) => {
                (active.path.clone(), active.bytes, false)
            }
            _ => (self.root.join(segment_file_name(seq)), 0, true),
        };
        write_line(&path, &line, previous_len, create)?;

        state.last_seq = seq;
        state.last_mac = *mac.as_bytes();
        state.active = Some(ActiveSegment {
            path,
            bytes: previous_len + line.len() as u64,
            month,
        });
        if create {
            // The record is already durable; a failed prune must not turn it
            // into a reported drop. It is logged and retried at the next
            // rollover.
            if let Err(error) = self.prune(chrono::Utc::now().timestamp_millis()) {
                tracing::warn!(kind = error.code(), "audit retention pruning failed");
            }
        }
        Ok(())
    }

    fn needs_rollover(&self, active: &ActiveSegment, month: (i32, u32)) -> bool {
        active.bytes > self.limits.segment_max_bytes || active.month != month
    }

    /// Checks the whole retained log. Stops at the first problem.
    ///
    /// The append lock is held only to take the snapshot: the segment list and
    /// each segment's length. Appends only ever extend the newest segment and
    /// pruning only removes the oldest, so the first `bytes` bytes of each
    /// snapshotted segment are a fixed prefix of the chain. Reading and checking
    /// that prefix outside the lock gives the same result as holding it, and
    /// records appended meanwhile are not part of this report. The one
    /// difference: a rollover that prunes a snapshotted segment before it is read
    /// makes this call fail with `AuditStorageFailed`. The next call sees the
    /// pruned set.
    pub fn verify(&self) -> AppResult<VerificationReport> {
        let (torn_tail_repaired, segments) = {
            let state = self.lock_state()?;
            (state.torn_tail_repaired, list_segments(&self.root)?)
        };
        let mut report = VerificationReport::empty(segments.len(), torn_tail_repaired);
        // (seq, mac) of the last verified record; None before the first one.
        let mut previous: Option<(u64, [u8; 32])> = None;

        for segment in &segments {
            let bytes = read_snapshot(segment)?;
            for (index, line) in split_lines(&bytes).into_iter().enumerate() {
                let Some(record) = parse_line(line) else {
                    let expected = previous.map_or_else(
                        || "the first retained record".to_owned(),
                        |(last_seq, _)| format!("record {}", last_seq.saturating_add(1)),
                    );
                    return Ok(report.fail(format!("{expected} is not a valid audit record")));
                };
                let seq = record.seq;
                // The record's place in the chain. A record whose stored
                // sequence was edited is still named by its position.
                let position = previous.map_or(seq, |(last_seq, _)| last_seq.saturating_add(1));
                let prev_bytes = from_hex(&record.prev).unwrap_or([0; 32]);
                let event_json =
                    serde_json::to_vec(&record.event).map_err(|_| AppError::AuditStorageFailed)?;
                let mac = compute_mac(
                    &self.key,
                    record.v,
                    seq,
                    record.ts_ms,
                    &prev_bytes,
                    &event_json,
                );
                if record.v != FORMAT_VERSION || to_hex(mac.as_bytes()) != record.mac {
                    return Ok(report.fail(format!("record {position} fails its integrity check")));
                }

                // The file name fixes the sequence of a segment's first record. A
                // head cut from a segment shows here: the record that now comes
                // first no longer has the sequence its name gives it.
                if index == 0 && seq != segment.first_seq {
                    return Ok(report.fail(format!(
                        "segment {} begins with record {seq}",
                        segment.first_seq
                    )));
                }

                match previous {
                    None => {
                        if seq == 0 {
                            return Ok(report.fail("the first record has sequence 0".to_owned()));
                        }
                        let anchor = from_hex(&record.prev).unwrap_or([0; 32]);
                        if anchor == [0; 32] {
                            // Nothing precedes it, so the log starts here.
                            if seq != 1 {
                                return Ok(report.fail(format!(
                                    "record {seq} has no predecessor and is not record 1"
                                )));
                            }
                        } else {
                            if seq == 1 {
                                return Ok(report.fail(
                                    "record 1 does not chain from the start of the log".to_owned(),
                                ));
                            }
                            report.anchored_after_prune = true;
                        }
                    }
                    Some((last_seq, last_mac)) => {
                        if last_seq.checked_add(1) != Some(seq) {
                            let problem = if seq > last_seq {
                                format!("sequence jumps from {last_seq} to {seq}")
                            } else {
                                format!("record {seq} is out of order after record {last_seq}")
                            };
                            return Ok(report.fail(problem));
                        }
                        if record.prev != to_hex(&last_mac) {
                            return Ok(report.fail(format!(
                                "record {seq} does not chain from record {last_seq}"
                            )));
                        }
                    }
                }

                if report.first_seq.is_none() {
                    report.first_seq = Some(seq);
                    report.first_ts_ms = Some(record.ts_ms);
                }
                report.last_seq = Some(seq);
                report.last_ts_ms = Some(record.ts_ms);
                report.records += 1;
                previous = Some((seq, *mac.as_bytes()));
            }
        }
        Ok(report)
    }

    /// The newest `limit` records, oldest first. Reads only the trailing
    /// segments it needs.
    pub fn recent(&self, limit: usize) -> AppResult<Vec<AuditRecord>> {
        let _state = self.lock_state()?;
        let mut newest_first = Vec::new();
        if limit == 0 {
            return Ok(newest_first);
        }
        let segments = list_segments(&self.root)?;
        for segment in segments.iter().rev() {
            let bytes = fs::read(&segment.path).map_err(storage_error)?;
            for line in split_lines(&bytes).into_iter().rev() {
                let record = parse_line(line)
                    .ok_or(AppError::PersistenceInvalid("audit record is malformed"))?;
                newest_first.push(record);
                if newest_first.len() == limit {
                    break;
                }
            }
            if newest_first.len() == limit {
                break;
            }
        }
        newest_first.reverse();
        Ok(newest_first)
    }

    /// Deletes the oldest segments that are past retention, then the oldest
    /// segments while the log exceeds its total size. The newest segment is
    /// never deleted.
    fn prune(&self, now_ms: i64) -> AppResult<()> {
        let mut segments = list_segments(&self.root)?;
        let cutoff_ms =
            now_ms.saturating_sub(self.limits.retention_days.saturating_mul(MILLIS_PER_DAY));
        while segments.len() > 1 {
            match last_record_ts(&segments[0].path)? {
                Some(ts_ms) if ts_ms < cutoff_ms => {
                    let oldest = segments.remove(0);
                    fs::remove_file(&oldest.path).map_err(storage_error)?;
                }
                _ => break,
            }
        }
        let mut total: u64 = segments.iter().map(|segment| segment.bytes).sum();
        while total > self.limits.total_max_bytes && segments.len() > 1 {
            let oldest = segments.remove(0);
            fs::remove_file(&oldest.path).map_err(storage_error)?;
            total = total.saturating_sub(oldest.bytes);
        }
        Ok(())
    }

    fn lock_state(&self) -> AppResult<MutexGuard<'_, ChainState>> {
        self.state.lock().map_err(|_| AppError::AuditStorageFailed)
    }
}

/// Reads the device id, or creates one when the file is missing, empty or
/// malformed. The id file is not covered by the MAC: each record stores its
/// device id inside its MACed event, so a new id only changes records written
/// after it.
fn load_or_create_device_id(root: &Path) -> Result<String, OpenFailure> {
    let path = root.join(DEVICE_ID_FILE);
    match fs::read_to_string(&path) {
        Ok(value) => {
            if let Ok(id) = Uuid::parse_str(value.trim()) {
                return Ok(id.to_string());
            }
            tracing::warn!("audit device id is malformed; a new one is generated");
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            tracing::warn!("audit device id is not valid text; a new one is generated");
        }
        Err(error) => return Err(io_failure(error)),
    }
    let id = Uuid::new_v4().to_string();
    write_device_id(root, &id).map_err(io_failure)?;
    Ok(id)
}

/// The stored device id, when the file exists and holds a valid one.
fn read_device_id(root: &Path) -> Option<String> {
    let value = fs::read_to_string(root.join(DEVICE_ID_FILE)).ok()?;
    Uuid::parse_str(value.trim()).ok().map(|id| id.to_string())
}

/// Writes `id` into a fresh log folder. A failure is logged and leaves the
/// folder to generate a new id, which is what happens without a stored one.
fn restore_device_id(root: &Path, id: &str) {
    let written = fs::create_dir_all(root).and_then(|()| write_device_id(root, id));
    if written.is_err() {
        tracing::warn!("audit device id could not be carried over; a new one is generated");
    }
}

/// Writes the device id through a temporary file in the same folder, so a
/// crash leaves the old file or the new one, never an empty file.
fn write_device_id(root: &Path, id: &str) -> io::Result<()> {
    let mut temporary = tempfile::Builder::new()
        .prefix(".device-id-")
        .tempfile_in(root)?;
    temporary.write_all(id.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(root.join(DEVICE_ID_FILE))
        .map_err(|error| error.error)?;
    Ok(())
}

fn storage_error(_: io::Error) -> AppError {
    AppError::AuditStorageFailed
}

fn segment_file_name(first_seq: u64) -> String {
    format!("{SEGMENT_PREFIX}{first_seq:0SEGMENT_NUMBER_DIGITS$}{SEGMENT_SUFFIX}")
}

fn segment_first_seq(file_name: &str) -> Option<u64> {
    let digits = file_name
        .strip_prefix(SEGMENT_PREFIX)?
        .strip_suffix(SEGMENT_SUFFIX)?;
    if digits.len() != SEGMENT_NUMBER_DIGITS || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Segment files in ascending first-sequence order, with their sizes.
fn list_segments(root: &Path) -> AppResult<Vec<SegmentFile>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(root).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        let name = entry.file_name();
        let Some(first_seq) = name.to_str().and_then(segment_first_seq) else {
            continue;
        };
        let bytes = entry.metadata().map_err(storage_error)?.len();
        found.push(SegmentFile {
            path: entry.path(),
            bytes,
            first_seq,
        });
    }
    found.sort_by_key(|segment| segment.first_seq);
    Ok(found)
}

/// How many leading bytes are complete lines: everything up to and including
/// the final newline. Bytes after it are a write that never finished.
fn complete_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1)
}

fn truncate_segment(path: &Path, len: u64) -> io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(len)?;
    file.sync_data()
}

/// Reads the first `bytes` bytes of a segment: the length it had when the
/// verify snapshot was taken.
fn read_snapshot(segment: &SegmentFile) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let read =
        File::open(&segment.path).and_then(|file| file.take(segment.bytes).read_to_end(&mut bytes));
    read.map_err(storage_error)?;
    Ok(bytes)
}

/// Writes one line. On a failed write the file is restored to its previous
/// length, or removed when this call created it, so no partial line stays
/// in the middle of the log.
fn write_line(path: &Path, line: &[u8], previous_len: u64, create: bool) -> AppResult<()> {
    let mut options = OpenOptions::new();
    options.append(true);
    if create {
        options.create_new(true);
    }
    let mut file = options.open(path).map_err(storage_error)?;
    let written = file
        .write_all(line)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_data());
    if let Err(error) = written {
        if create {
            let _ = fs::remove_file(path);
        } else {
            let _ = file.set_len(previous_len);
        }
        return Err(storage_error(error));
    }
    Ok(())
}

/// Splits bytes into lines without their `\n`. A trailing newline does not
/// produce an empty final line.
fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

/// Parses and structurally checks one record. The MAC is checked separately
/// by [`LocalAuditStore::verify`].
fn parse_line(line: &[u8]) -> Option<AuditRecord> {
    let record: AuditRecord = serde_json::from_slice(line).ok()?;
    (from_hex(&record.prev).is_some()
        && from_hex(&record.mac).is_some()
        && record.event.validate().is_ok())
    .then_some(record)
}

fn last_record_ts(path: &Path) -> AppResult<Option<i64>> {
    let bytes = fs::read(path).map_err(storage_error)?;
    Ok(split_lines(&bytes)
        .last()
        .and_then(|line| parse_line(line))
        .map(|record| record.ts_ms))
}

fn month_key(ts_ms: i64) -> AppResult<(i32, u32)> {
    use chrono::{Datelike, TimeZone, Utc};
    let instant = Utc
        .timestamp_millis_opt(ts_ms)
        .single()
        .ok_or(AppError::PersistenceInvalid(
            "audit timestamp is out of range",
        ))?;
    Ok((instant.year(), instant.month()))
}

/// MAC over every stored field of a record: version, sequence, timestamp
/// (all little-endian), the previous MAC, and the serialized event.
pub(super) fn compute_mac(
    key: &[u8; 32],
    version: u32,
    seq: u64,
    ts_ms: i64,
    prev: &[u8; 32],
    event_json: &[u8],
) -> blake3::Hash {
    let mut input = Vec::with_capacity(4 + 8 + 8 + prev.len() + event_json.len());
    input.extend_from_slice(&version.to_le_bytes());
    input.extend_from_slice(&seq.to_le_bytes());
    input.extend_from_slice(&ts_ms.to_le_bytes());
    input.extend_from_slice(prev);
    input.extend_from_slice(event_json);
    blake3::keyed_hash(key, &input)
}

/// Renames `root` to a sibling `<name>.damaged-<unix seconds>` (with `-1`,
/// `-2`, ... appended on a name clash), then keeps only the newest
/// [`MAX_DAMAGED_SIBLINGS`] set-aside logs. Pruning failures are logged, not
/// returned, because the damaged log is already out of the way.
fn set_aside(root: &Path) -> AppResult<()> {
    let parent = root.parent().ok_or(AppError::AuditStorageFailed)?;
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(AppError::AuditStorageFailed)?;
    let stamp = chrono::Utc::now().timestamp();
    let mut target = parent.join(format!("{name}{DAMAGED_MARKER}{stamp}"));
    let mut suffix = 1_u32;
    while target.exists() {
        target = parent.join(format!("{name}{DAMAGED_MARKER}{stamp}-{suffix}"));
        suffix += 1;
    }
    fs::rename(root, &target).map_err(storage_error)?;
    if let Err(error) = prune_damaged(parent, name) {
        tracing::warn!(kind = error.code(), "set-aside audit logs were not pruned");
    }
    Ok(())
}

fn prune_damaged(parent: &Path, name: &str) -> AppResult<()> {
    let prefix = format!("{name}{DAMAGED_MARKER}");
    let mut found: Vec<((i64, u32), PathBuf)> = Vec::new();
    for entry in fs::read_dir(parent).map_err(storage_error)? {
        let entry = entry.map_err(storage_error)?;
        let file_name = entry.file_name();
        let Some(rest) = file_name
            .to_str()
            .and_then(|value| value.strip_prefix(prefix.as_str()))
        else {
            continue;
        };
        let (stamp, suffix) = match rest.split_once('-') {
            Some((stamp, suffix)) => (stamp, suffix.parse().ok()),
            None => (rest, Some(0)),
        };
        let (Ok(stamp), Some(suffix)) = (stamp.parse::<i64>(), suffix) else {
            continue;
        };
        if entry.file_type().map_err(storage_error)?.is_dir() {
            found.push(((stamp, suffix), entry.path()));
        }
    }
    found.sort_by_key(|(order, _)| *order);
    let excess = found.len().saturating_sub(MAX_DAMAGED_SIBLINGS);
    for (_, path) in found.into_iter().take(excess) {
        if let Err(error) = fs::remove_dir_all(&path) {
            tracing::warn!(
                kind = storage_error(error).code(),
                "set-aside audit log was not removed"
            );
        }
    }
    Ok(())
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// Decodes exactly 64 lower-hex characters into 32 bytes.
fn from_hex(value: &str) -> Option<[u8; 32]> {
    let bytes = value.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        let high = hex_nibble(*bytes.get(index * 2)?)?;
        let low = hex_nibble(*bytes.get(index * 2 + 1)?)?;
        *slot = (high << 4) | low;
    }
    Some(out)
}

const fn hex_nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}
