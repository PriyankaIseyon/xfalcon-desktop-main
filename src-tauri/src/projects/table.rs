//! Windows of rows from a delimited project file too large to parse in the
//! webview (spec: project-file-editing, "Large tables page from the backend").
//!
//! One parser — the `csv` crate, which honours quoted delimiters and quoted
//! newlines, so a row is never split on a bare `\n`. A window is found through
//! a sparse row index: the byte position of every 1,024th data row, recorded as
//! reads pass it. A later window seeks to the nearest checkpoint instead of
//! re-reading the file from its start, so paging a 10 GiB file costs one pass
//! over the part read so far, once.
//!
//! The index is keyed by the file's size and modified time, read from the
//! handle that is parsed, and by the delimiter, so a changed file gets a fresh
//! one; a handful are kept for the files most recently paged. The cache lock is
//! never held across I/O: an index is taken out, used, and put back.
//!
//! Memory stays bounded whatever the file holds: one row may span at most
//! [`MAX_ROW_BYTES`] of the file (an unclosed quote cannot become a 10 GiB
//! row), cells and columns are cut for display, and a window stops early past
//! [`MAX_WINDOW_BYTES`], with `next_row` continuing from there.

use std::cell::Cell;
use std::io::{self, Read, Seek, SeekFrom};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use csv::{ByteRecord, Position, ReaderBuilder};
use xfalcon_contracts::RunId;

use super::domain::{ProjectId, TrustedRoot};
use super::path_policy::RelativeProjectPath;
use super::sensitive::SecretClassifier;
use super::viewer::ViewerError;

/// The most rows one window returns.
pub const MAX_TABLE_WINDOW_ROWS: u32 = 1_000;
/// Rows between index checkpoints.
const CHECKPOINT_EVERY: u64 = 1_024;
/// A cell past this is cut for display and the window marked `truncated`.
const MAX_CELL_BYTES: usize = 16 * 1024;
/// Columns past this are dropped for display, and the window marked.
const MAX_COLUMNS: usize = 1_024;
/// The most raw bytes one row may span. The parser holds a whole row in memory
/// before it yields it, so an unclosed quote near the start of a 10 GiB file
/// would otherwise become one 10 GiB row.
const MAX_ROW_BYTES: u64 = 64 * 1024 * 1024;
/// Displayed bytes one window carries at most. A window stops early past this
/// (always holding at least one row) and `next_row` continues from there, so
/// a thousand wide rows cannot become one multi-gigabyte reply.
const MAX_WINDOW_BYTES: usize = 16 * 1024 * 1024;
/// Row indexes kept, most recently used first.
const INDEX_CACHE_ENTRIES: usize = 8;
/// Buffer for the reader: large sequential reads, few syscalls.
const READ_BUFFER_BYTES: usize = 1024 * 1024;

/// A reader that refuses to read past a moving limit, so the parser cannot
/// grow one row without bound. [`table_rows`] moves the limit to
/// [`Bounds::row_bytes`] past the start of each row before it reads the row.
struct RowBoundedReader<R> {
    inner: R,
    position: u64,
    limit: Rc<Cell<u64>>,
}

/// The payload of the I/O error [`RowBoundedReader`] raises at its limit.
#[derive(Debug, thiserror::Error)]
#[error("a row is longer than the limit for one row")]
struct RowTooLong;

/// The memory bounds one window is read under: the production constants, or
/// smaller ones so a test can reach them without a 64 MiB fixture.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bounds {
    pub(crate) row_bytes: u64,
    pub(crate) window_bytes: usize,
}

impl Bounds {
    const PRODUCTION: Self = Self {
        row_bytes: MAX_ROW_BYTES,
        window_bytes: MAX_WINDOW_BYTES,
    };
}

impl<R: Read> Read for RowBoundedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let allowed = self.limit.get().saturating_sub(self.position);
        if allowed == 0 {
            // A row that ends exactly at the limit, at the end of the file, is
            // not too long: only a byte that is actually there is refused.
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, RowTooLong)),
            };
        }
        let len = buf
            .len()
            .min(usize::try_from(allowed).unwrap_or(usize::MAX));
        let read = self.inner.read(&mut buf[..len])?;
        self.position += read as u64;
        Ok(read)
    }
}

impl<R: Seek> Seek for RowBoundedReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.position = self.inner.seek(pos)?;
        Ok(self.position)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IndexKey {
    project_id: ProjectId,
    path: String,
    size: u64,
    modified_ns: Option<u128>,
    delimiter: u8,
}

struct RowIndex {
    key: IndexKey,
    headers: Vec<String>,
    headers_truncated: bool,
    /// `checkpoints[k]` is where data row `k * CHECKPOINT_EVERY` starts.
    checkpoints: Vec<Position>,
    /// The data-row count, once a read reached the end.
    total_rows: Option<u64>,
}

fn cache() -> &'static Mutex<Vec<RowIndex>> {
    static CACHE: OnceLock<Mutex<Vec<RowIndex>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn take_index(key: &IndexKey) -> Option<RowIndex> {
    let mut entries = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let position = entries.iter().position(|index| &index.key == key)?;
    Some(entries.remove(position))
}

fn keep_index(index: RowIndex) {
    let mut entries = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    entries.retain(|entry| entry.key != index.key);
    entries.insert(0, index);
    entries.truncate(INDEX_CACHE_ENTRIES);
}

/// One window, as [`table_rows`] returns it.
#[derive(Debug, Eq, PartialEq)]
pub struct TableWindow {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub start_row: u64,
    pub next_row: Option<u64>,
    pub total_rows: Option<u64>,
    pub truncated: bool,
}

/// A record's cells for display: lossy UTF-8, each cell and the column count
/// bounded. Returns whether anything was cut.
fn display_cells(record: &ByteRecord) -> (Vec<String>, bool) {
    let mut cut = record.len() > MAX_COLUMNS;
    let cells = record
        .iter()
        .take(MAX_COLUMNS)
        .map(|field| {
            // Cut the raw bytes first, on a character boundary, so a huge cell
            // is never decoded whole just to be shortened.
            let mut end = field.len().min(MAX_CELL_BYTES);
            if end < field.len() {
                while end > 0 && field[end] & 0b1100_0000 == 0b1000_0000 {
                    end -= 1;
                }
            }
            let text = String::from_utf8_lossy(&field[..end]);
            if end == field.len() && text.len() <= MAX_CELL_BYTES {
                return text.into_owned();
            }
            cut = true;
            // Lossy decoding can widen invalid bytes past the bound again.
            let mut end = text.len().min(MAX_CELL_BYTES);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…", &text[..end])
        })
        .collect();
    (cells, cut)
}

fn csv_error(error: csv::Error, row_bytes: u64) -> ViewerError {
    match error.into_kind() {
        csv::ErrorKind::Io(error)
            if error
                .get_ref()
                .is_some_and(|inner| inner.is::<RowTooLong>()) =>
        {
            ViewerError::RowTooLong { limit: row_bytes }
        }
        csv::ErrorKind::Io(error) => ViewerError::Io(error),
        other => ViewerError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{other:?}"),
        )),
    }
}

/// Up to `limit` data rows from `start_row` (zero-based, after the header).
pub fn table_rows(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
    delimiter: u8,
    start_row: u64,
    limit: u32,
) -> Result<TableWindow, ViewerError> {
    table_rows_bounded(
        root,
        classifier,
        path,
        run_id,
        delimiter,
        start_row,
        limit,
        Bounds::PRODUCTION,
    )
}

/// [`table_rows`] under explicit [`Bounds`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn table_rows_bounded(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
    delimiter: u8,
    start_row: u64,
    limit: u32,
    bounds: Bounds,
) -> Result<TableWindow, ViewerError> {
    let map_csv = |error: csv::Error| csv_error(error, bounds.row_bytes);
    let limit = u64::from(limit.clamp(1, MAX_TABLE_WINDOW_ROWS));
    // The same gates as every other view read — sensitivity from the path, a
    // regular file, the size cap, a text sniff — and the index is keyed by the
    // metadata of the handle that is parsed, never of a path looked up apart
    // from it.
    let (file, metadata) = super::viewer::open_text(root, classifier, path, run_id)?;
    let key = IndexKey {
        project_id: *root.project_id(),
        path: path.as_str().to_owned(),
        size: metadata.len(),
        modified_ns: metadata
            .modified()
            .ok()
            .and_then(|time| time.into_std().duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_nanos()),
        delimiter,
    };

    // Every read starts from position 0 (the header) or a recorded row start,
    // and each row is allowed `bounds.row_bytes` from where it begins.
    let row_limit = Rc::new(Cell::new(bounds.row_bytes));
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .buffer_capacity(READ_BUFFER_BYTES)
        .from_reader(RowBoundedReader {
            inner: file,
            position: 0,
            limit: Rc::clone(&row_limit),
        });
    let mut record = ByteRecord::new();

    let mut index = match take_index(&key) {
        Some(index) => index,
        None => {
            let (mut headers, headers_truncated) =
                if reader.read_byte_record(&mut record).map_err(map_csv)? {
                    display_cells(&record)
                } else {
                    (Vec::new(), false)
                };
            if let Some(first) = headers.first_mut() {
                if let Some(stripped) = first.strip_prefix('\u{feff}') {
                    *first = stripped.to_owned();
                }
            }
            RowIndex {
                key: key.clone(),
                headers,
                headers_truncated,
                checkpoints: vec![reader.position().clone()],
                total_rows: None,
            }
        }
    };
    if let Some(total) = index.total_rows {
        if start_row > total {
            keep_index(index);
            return Err(ViewerError::OffsetPastEnd {
                offset: start_row,
                size: total,
            });
        }
    }

    // Start from the nearest checkpoint at or before the window. On a reader
    // that has not read the header yet, `seek` reads it first, from position
    // 0 under the limit set above.
    let checkpoint = usize::try_from(start_row / CHECKPOINT_EVERY)
        .unwrap_or(usize::MAX)
        .min(index.checkpoints.len() - 1);
    reader
        .seek(index.checkpoints[checkpoint].clone())
        .map_err(map_csv)?;
    let mut row = checkpoint as u64 * CHECKPOINT_EVERY;

    let mut rows = Vec::new();
    let mut window_bytes = 0usize;
    let mut truncated = index.headers_truncated;
    let mut reached_end = false;
    // Exclusive end of the window; pulled in if the byte budget runs out.
    let mut window_end = start_row.saturating_add(limit);
    loop {
        if row.is_multiple_of(CHECKPOINT_EVERY)
            && row / CHECKPOINT_EVERY == index.checkpoints.len() as u64
        {
            index.checkpoints.push(reader.position().clone());
        }
        row_limit.set(reader.position().byte().saturating_add(bounds.row_bytes));
        if !reader.read_byte_record(&mut record).map_err(map_csv)? {
            reached_end = true;
            break;
        }
        // One row past the window is read, to know whether another follows.
        if row >= window_end {
            break;
        }
        if row >= start_row {
            let (cells, cut) = display_cells(&record);
            truncated |= cut;
            window_bytes = window_bytes.saturating_add(cells.iter().map(String::len).sum());
            rows.push(cells);
            if window_bytes >= bounds.window_bytes {
                window_end = row + 1;
            }
        }
        row += 1;
    }
    if reached_end {
        index.total_rows = Some(row);
        if start_row > row {
            let total = row;
            keep_index(index);
            return Err(ViewerError::OffsetPastEnd {
                offset: start_row,
                size: total,
            });
        }
    }
    let next_row = (!reached_end).then_some(window_end);
    let window = TableWindow {
        headers: index.headers.clone(),
        rows,
        start_row,
        next_row,
        total_rows: index.total_rows,
        truncated,
    };
    keep_index(index);
    Ok(window)
}
