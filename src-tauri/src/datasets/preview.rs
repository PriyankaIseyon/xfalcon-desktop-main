//! The leading rows of a staged file, so a person confirms what they picked
//! before any byte moves.
//!
//! Every rule and every number here is xfalcon-web's, from
//! `src/lib/datasets/preview.ts` and `src/lib/datasets/preview-constants.ts`.
//! The two products describe the same file to the same person; a preview that
//! showed 50 rows in one and 500 in the other, or named a duplicate column
//! differently, would make the confirmation step untrustworthy in whichever
//! one the person used second.
//!
//! # Why this does not reuse `projects::schema::parse_csv_document`
//!
//! That parser takes the whole file as a `&str` and returns every row, which
//! is right for a project file being summarised for the agent and wrong here:
//! a dataset is up to 2 GiB, and the preview needs 60 rows. It also trims
//! unquoted fields and drops blank rows, behaviour its own schema summaries
//! depend on. `infer_type` **is** shared, so a column is labelled the same way
//! on both surfaces.
//!
//! # Bounds
//!
//! [`PREVIEW_ROW_LIMIT`] rows are shown and [`HEADER_PICKER_SAMPLE_ROWS`] more
//! are read so the header picker can show every candidate. The rest bound what
//! one pathological file can cost: a row wider than
//! [`MAX_PREVIEW_COLUMNS`], a cell longer than [`MAX_CELL_CHARS`], a file with
//! no line break in its first [`NO_NEWLINE_SCAN_BYTES`], and a workbook past
//! [`WORKBOOK_PREVIEW_MAX_BYTES`] whose shared-string table would be loaded
//! whole.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use calamine::{Reader, Xlsx};
use xfalcon_contracts::datasets::{
    DatasetColumnView, DatasetErrorCode, DatasetFormat, DatasetPreviewUnavailable,
};

use crate::projects::schema::infer_type;

/// Rows shown in the confirmation step. Enough to recognise a file and to see
/// a header row; small enough to cross the IPC boundary as one message.
pub const PREVIEW_ROW_LIMIT: usize = 50;

/// Leading raw rows retained for the header-row picker. Wide enough for the
/// title, banner, and metadata rows real exports put above their headers.
pub const HEADER_PICKER_SAMPLE_ROWS: usize = 10;

/// Widest row the preview will describe. Mirrors DA's own column ceiling: a
/// wider sheet is refused server-side anyway, and rendering a 100,000-cell row
/// costs more than the answer is worth.
pub const MAX_PREVIEW_COLUMNS: usize = 1024;

/// Longest cell the preview will carry. The original file still uploads
/// byte-exact — only the copy a person reads is cut.
pub const PREVIEW_CELL_MAX_CHARS: usize = 16_384;

/// Longest single value the parser will accept. Mirrors DA's per-value
/// ceiling. A delimited file with an unterminated quote is one cell to any
/// parser, and buffering it is how a 2 GiB file becomes one string.
pub const MAX_CELL_CHARS: usize = 10 * 1024 * 1024;

/// How far to look for a line break before concluding a file is not
/// line-delimited text. Far beyond any legitimate single record.
pub const NO_NEWLINE_SCAN_BYTES: u64 = 8 * 1024 * 1024;

/// Largest workbook this module will open. `calamine` loads the whole
/// shared-string table before any cell can be resolved, so the ceiling is that
/// load rather than the rows. Matches web's, so the same file gets the same
/// answer in both products.
pub const WORKBOOK_PREVIEW_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Delimiters guessed for a `.csv`, in the order web's parser tries them.
const CANDIDATE_DELIMITERS: [u8; 4] = *b",\t;|";

/// Rows read from a staged file, before a header row is chosen.
///
/// Held so switching the header row re-summarises cached rows instead of
/// re-reading the file — for a workbook that would mean re-loading the
/// shared-string table on every click.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sample {
    pub rows: Vec<Vec<Option<String>>>,
    /// The file holds more rows than were read.
    pub truncated: bool,
    /// At least one cell was cut to [`PREVIEW_CELL_MAX_CHARS`].
    pub truncated_cell: bool,
    /// At least one row was wider than [`MAX_PREVIEW_COLUMNS`].
    pub truncated_columns: bool,
    /// Which worksheet the rows came from. Zero for a format with none.
    pub worksheet_index: u32,
}

/// A header row, resolved into column names.
#[derive(Clone, Debug)]
pub struct Header {
    pub columns: Vec<DatasetColumnView>,
    /// Names that had to be disambiguated, so the surface can say the names
    /// shown are not a one-to-one copy of the file's.
    pub renamed: u32,
}

fn sample_cap() -> usize {
    PREVIEW_ROW_LIMIT + HEADER_PICKER_SAMPLE_ROWS
}

/// The worksheet names in a staged workbook, for the sheet selector.
///
/// Empty when the workbook is past [`WORKBOOK_PREVIEW_MAX_BYTES`] or cannot be
/// opened: the person can still upload it and name the worksheet index by
/// hand, which is what DA's own default would otherwise decide for them.
pub fn worksheet_names(path: &Path, size: u64) -> Vec<String> {
    if size > WORKBOOK_PREVIEW_MAX_BYTES {
        return Vec::new();
    }
    match open_workbook(path) {
        Ok(workbook) => workbook.sheet_names(),
        Err(_) => Vec::new(),
    }
}

/// Reads the leading rows of a staged file.
pub fn read_sample(
    path: &Path,
    format: DatasetFormat,
    size: u64,
    worksheet_index: u32,
) -> Result<Sample, DatasetPreviewUnavailable> {
    match format {
        DatasetFormat::Csv => read_delimited_sample(path, size),
        DatasetFormat::Xlsx => read_workbook_sample(path, size, worksheet_index),
    }
}

fn read_delimited_sample(path: &Path, size: u64) -> Result<Sample, DatasetPreviewUnavailable> {
    let delimiter = guess_delimiter(path, size)?;
    let file = File::open(path).map_err(|_| DatasetPreviewUnavailable::Unreadable)?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .has_headers(false)
        .from_reader(BufReader::new(file));

    let cap = sample_cap();
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut truncated = false;
    let mut truncated_cell = false;
    let mut truncated_columns = false;
    for record in reader.records() {
        let record = record.map_err(|_| DatasetPreviewUnavailable::Unreadable)?;
        if rows.len() == cap {
            // One record past the cap is all it takes to know the file has
            // more; reading the rest would read the whole file to learn
            // nothing else.
            truncated = true;
            break;
        }
        if record.iter().any(|field| field.len() > MAX_CELL_CHARS) {
            return Err(DatasetPreviewUnavailable::NotDelimited);
        }
        if record.len() > MAX_PREVIEW_COLUMNS {
            truncated_columns = true;
        }
        let mut row = Vec::with_capacity(record.len().min(MAX_PREVIEW_COLUMNS));
        for field in record.iter().take(MAX_PREVIEW_COLUMNS) {
            let (cell, cut) = display_cell(field);
            truncated_cell |= cut;
            row.push(cell);
        }
        rows.push(row);
    }

    if rows.is_empty() {
        return Err(DatasetPreviewUnavailable::Unreadable);
    }
    Ok(Sample {
        rows,
        truncated,
        truncated_cell,
        truncated_columns,
        worksheet_index: 0,
    })
}

/// Picks the delimiter by counting candidates in the file's first line, and
/// refuses a file with no line break in its leading bytes.
///
/// A file with no record separator is not a delimited table, and every
/// row-oriented parser answers that by buffering until it runs out of memory.
///
/// Shared with [`crate::datasets::recipe`], which reads the whole file: a
/// recipe that ran against a comma while the preview was parsed with a
/// semicolon would upload one column where the person reviewed twelve.
pub(crate) fn guess_delimiter(path: &Path, size: u64) -> Result<u8, DatasetPreviewUnavailable> {
    let file = File::open(path).map_err(|_| DatasetPreviewUnavailable::Unreadable)?;
    let scan = NO_NEWLINE_SCAN_BYTES.min(size);
    let mut head = Vec::new();
    file.take(scan)
        .read_to_end(&mut head)
        .map_err(|_| DatasetPreviewUnavailable::Unreadable)?;

    let first_break = head
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r');
    let Some(end) = first_break else {
        // Only conclusive once the whole scan window was read: a short file
        // with no trailing newline is a single valid record.
        if size > scan {
            return Err(DatasetPreviewUnavailable::NotDelimited);
        }
        return Ok(best_delimiter(&head));
    };
    Ok(best_delimiter(&head[..end]))
}

/// The candidate that appears most often outside quotes, defaulting to a comma
/// when none appears — a single-column file is still a table.
fn best_delimiter(line: &[u8]) -> u8 {
    let mut best = b',';
    let mut best_count = 0_usize;
    for candidate in CANDIDATE_DELIMITERS {
        let mut count = 0_usize;
        let mut in_quotes = false;
        for byte in line {
            match *byte {
                b'"' => in_quotes = !in_quotes,
                other if other == candidate && !in_quotes => count += 1,
                _ => {}
            }
        }
        if count > best_count {
            best_count = count;
            best = candidate;
        }
    }
    best
}

fn open_workbook(path: &Path) -> Result<Xlsx<BufReader<File>>, DatasetErrorCode> {
    let file = File::open(path).map_err(|_| DatasetErrorCode::Unreadable)?;
    Xlsx::new(BufReader::new(file)).map_err(|_| DatasetErrorCode::ContentMismatch)
}

fn read_workbook_sample(
    path: &Path,
    size: u64,
    worksheet_index: u32,
) -> Result<Sample, DatasetPreviewUnavailable> {
    if size > WORKBOOK_PREVIEW_MAX_BYTES {
        return Err(DatasetPreviewUnavailable::FileTooLarge);
    }
    let mut workbook = open_workbook(path).map_err(|_| DatasetPreviewUnavailable::Unreadable)?;
    let names = workbook.sheet_names();
    if names.is_empty() {
        return Err(DatasetPreviewUnavailable::Unreadable);
    }
    let index = (worksheet_index as usize).min(names.len() - 1);
    let name = names[index].clone();
    let mut cells = workbook
        .worksheet_cells_reader(&name)
        .map_err(|_| DatasetPreviewUnavailable::Unreadable)?;

    let cap = sample_cap();
    // The reader emits cells in XML order with their coordinates, so rows are
    // assembled by position rather than assumed dense: a sheet with gaps has
    // no cell for an empty coordinate at all.
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    let mut truncated = false;
    let mut truncated_cell = false;
    let mut truncated_columns = false;
    let mut first_row: Option<u32> = None;
    let mut first_column: Option<usize> = None;
    loop {
        let cell = match cells.next_cell() {
            Ok(Some(cell)) => cell,
            Ok(None) => break,
            Err(_) => return Err(DatasetPreviewUnavailable::Unreadable),
        };
        let (absolute_row, absolute_column) = cell.get_position();
        let origin = *first_row.get_or_insert(absolute_row);
        let Some(row_offset) = absolute_row.checked_sub(origin) else {
            // Rows arrive in ascending order; one that does not means the
            // sheet is not something this reader understands.
            return Err(DatasetPreviewUnavailable::Unreadable);
        };
        let row_offset = row_offset as usize;
        if row_offset >= cap {
            truncated = true;
            break;
        }
        let column = absolute_column as usize;
        if column >= MAX_PREVIEW_COLUMNS {
            truncated_columns = true;
            continue;
        }
        first_column = Some(first_column.map_or(column, |first| first.min(column)));
        while rows.len() <= row_offset {
            rows.push(Vec::new());
        }
        let row = &mut rows[row_offset];
        while row.len() <= column {
            row.push(None);
        }
        let (value, cut) = display_cell(&stringify_data(cell.get_value()));
        truncated_cell |= cut;
        row[column] = value;
    }

    if rows.is_empty() {
        return Err(DatasetPreviewUnavailable::Unreadable);
    }
    // Leading empty columns are dropped, because the full-file read drops them
    // too: `calamine`'s `worksheet_range` reports a sheet's *used* range, so a
    // block of data starting at column C is three columns wide there and would
    // be five here — two of them named `col_1` and `col_2`. The preview has to
    // show the table that uploads, and a recipe step naming one of those
    // phantom columns would pass here and be refused at upload.
    //
    // Measured over the rows this reader saw, which is the sample. A cell
    // further left than every sampled row, below the window, is the one case
    // the two can still differ — the same limit `truncated` already reports.
    if let Some(origin) = first_column.filter(|first| *first > 0) {
        for row in &mut rows {
            row.drain(..origin.min(row.len()));
        }
    }
    Ok(Sample {
        rows,
        truncated,
        truncated_cell,
        truncated_columns,
        worksheet_index: index as u32,
    })
}

/// One worksheet cell, in the lexical form a person would read.
///
/// A date becomes ISO 8601 rather than its serial number: the serial is what
/// the file stores, but nobody confirms a spreadsheet by reading `45943.5`,
/// and DA's own reader will see a date too.
fn stringify_data(value: &calamine::DataRef<'_>) -> String {
    use calamine::DataRef;
    match value {
        DataRef::Empty => String::new(),
        DataRef::String(text) => text.clone(),
        DataRef::SharedString(text) => (*text).to_owned(),
        DataRef::Int(number) => number.to_string(),
        DataRef::Float(number) => number.to_string(),
        DataRef::Bool(flag) => flag.to_string(),
        DataRef::DateTimeIso(text) | DataRef::DurationIso(text) => text.clone(),
        DataRef::DateTime(stamp) => {
            if stamp.is_duration() {
                // A duration has no calendar date to render, so its serial
                // value is the only honest form.
                return stamp.as_f64().to_string();
            }
            let (year, month, day, hour, minute, second, _milli) = stamp.to_ymd_hms_milli();
            if hour == 0 && minute == 0 && second == 0 {
                format!("{year:04}-{month:02}-{day:02}")
            } else {
                format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
            }
        }
        DataRef::Error(error) => error.to_string(),
    }
}

/// A cell as the preview carries it: absent when the file has nothing there,
/// and cut to the display ceiling when it is longer than anyone reads.
///
/// Returns whether the value was cut, so the surface can say so once for the
/// whole preview rather than per cell.
fn display_cell(value: &str) -> (Option<String>, bool) {
    if value.is_empty() {
        return (None, false);
    }
    if value.chars().count() <= PREVIEW_CELL_MAX_CHARS {
        return (Some(value.to_owned()), false);
    }
    let cut: String = value.chars().take(PREVIEW_CELL_MAX_CHARS).collect();
    (Some(cut), true)
}

/// Disambiguates a header row's names so every column can be addressed.
///
/// DA suffixes duplicates `foo`, `foo_2`, `foo_3` before insertion. Mirroring
/// the rule means a recipe step can name the second `foo` — without it, a step
/// targeting a later duplicate silently applies to the first, and the columns
/// a person sees here are the ones the dataset ends up with whether or not a
/// recipe ran.
///
/// Comparison is case-insensitive, matching DA, but the kept name preserves
/// the file's own casing. A file that already contains `foo` and `foo_2`
/// keeps both: the probe walks upward until the candidate is free.
pub fn dedup_column_names(names: &[String]) -> (Vec<String>, u32) {
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(names.len());
    let mut renamed = 0_u32;
    for original in names {
        let mut candidate = original.clone();
        let mut suffix = 2_u32;
        while taken.contains(&candidate.to_lowercase()) {
            candidate = format!("{original}_{suffix}");
            suffix += 1;
        }
        if &candidate != original {
            renamed += 1;
        }
        taken.insert(candidate.to_lowercase());
        out.push(candidate);
    }
    (out, renamed)
}

/// The row most likely to hold the column names.
///
/// Real exports put a title or a banner above the header. The rule: find the
/// widest populated row in the sample, then take the first row that is at
/// least half that wide, is itself at least two cells, and reads like a header
/// — mostly text, at most one numeric or date cell. That skips a one-cell
/// title without mistaking a sparse data row for the header, and still finds a
/// header that is *wider* than the data under it, which is what trailing
/// blanks in the data produce.
///
/// Falls back to the first row. The person keeps the manual picker either way.
///
/// Only the rows [`header_candidates`] offers are considered. A suggestion
/// below that window could not be selected in the picker, which would leave
/// the control showing a different row than the preview was built from.
pub fn suggest_header_row(rows: &[Vec<Option<String>>]) -> u32 {
    if rows.len() < 2 {
        return 0;
    }
    let rows = &rows[..rows.len().min(HEADER_PICKER_SAMPLE_ROWS)];
    let widths: Vec<usize> = rows.iter().map(|row| populated_cells(row)).collect();
    let max_width = widths.iter().copied().max().unwrap_or(0);
    if max_width < 2 {
        return 0;
    }
    let min_width = max_width.div_ceil(2);
    for (index, row) in rows.iter().enumerate() {
        if widths[index] < min_width || widths[index] < 2 {
            continue;
        }
        if !reads_like_a_header(row) {
            continue;
        }
        return u32::try_from(index).unwrap_or(0);
    }
    0
}

fn populated_cells(row: &[Option<String>]) -> usize {
    row.iter()
        .filter(|cell| {
            cell.as_deref()
                .map(str::trim)
                .is_some_and(|v| !v.is_empty())
        })
        .count()
}

fn reads_like_a_header(row: &[Option<String>]) -> bool {
    let mut text_cells = 0_usize;
    let mut data_cells = 0_usize;
    for cell in row {
        let Some(value) = cell.as_deref().map(str::trim) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        if looks_numeric(value) || looks_like_a_date(value) {
            data_cells += 1;
        } else {
            text_cells += 1;
        }
    }
    text_cells > 0 && data_cells <= 1
}

/// `-12`, `3.5`, `1,5`, `40%` — a value that is a measurement rather than a
/// name. Both decimal separators count: a header row is being told apart from
/// a data row, and which separator the file uses is not known here.
fn looks_numeric(value: &str) -> bool {
    let body = value.strip_suffix('%').unwrap_or(value);
    let body = body.strip_prefix('-').unwrap_or(body);
    if body.is_empty() {
        return false;
    }
    let mut seen_separator = false;
    let mut digits_before = 0_usize;
    let mut digits_after = 0_usize;
    for character in body.chars() {
        match character {
            '0'..='9' if seen_separator => digits_after += 1,
            '0'..='9' => digits_before += 1,
            '.' | ',' if !seen_separator && digits_before > 0 => seen_separator = true,
            _ => return false,
        }
    }
    digits_before > 0 && (!seen_separator || digits_after > 0)
}

/// `2026-08-24`, `24/08/2026`, `8.24.2026` — the separated numeric forms a
/// spreadsheet writes a date as.
fn looks_like_a_date(value: &str) -> bool {
    let mut parts = value.split(['-', '/', '.']);
    let mut counted = 0_usize;
    for part in parts.by_ref() {
        if part.is_empty() || part.len() > 4 || !part.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        counted += 1;
        if counted > 3 {
            return false;
        }
    }
    counted == 3
}

/// Resolves a sample's chosen header row into column names.
///
/// Trailing empty header cells are dropped — a spreadsheet's used range often
/// runs past its last named column — and a blank name inside the row becomes
/// `col_N` so the column can still be addressed.
pub fn header_at(sample: &Sample, header_row_index: usize) -> Header {
    let clamped = header_row_index.min(sample.rows.len().saturating_sub(1));
    let raw = sample.rows.get(clamped).cloned().unwrap_or_default();
    let trimmed = trim_trailing_empty(&raw);
    let names: Vec<String> = trimmed
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let value = cell.as_deref().unwrap_or("").trim();
            if value.is_empty() {
                format!("col_{}", index + 1)
            } else {
                value.to_owned()
            }
        })
        .collect();
    let (names, renamed) = dedup_column_names(&names);
    Header {
        columns: names
            .into_iter()
            .map(|name| DatasetColumnView {
                name,
                inferred_type: String::new(),
            })
            .collect(),
        renamed,
    }
}

/// The sample's data rows: everything below the header row that is not blank.
///
/// Blank rows are dropped before the window is taken, so the window holds rows
/// that are actually shown. Slicing first would show fewer than the limit and
/// would compare the wrong count when deciding whether more rows exist.
pub fn data_rows(sample: &Sample, header_row_index: usize) -> (Vec<Vec<Option<String>>>, bool) {
    let clamped = header_row_index.min(sample.rows.len().saturating_sub(1));
    let non_blank: Vec<Vec<Option<String>>> = sample
        .rows
        .iter()
        .skip(clamped + 1)
        .filter(|row| populated_cells(row) > 0)
        .cloned()
        .collect();
    let more = sample.truncated || non_blank.len() > PREVIEW_ROW_LIMIT;
    let mut rows = non_blank;
    rows.truncate(PREVIEW_ROW_LIMIT);
    (rows, more)
}

/// Labels each column from the values under it, using the same inference the
/// projects surface uses so one file is described the same way everywhere.
pub fn label_columns(columns: &mut [DatasetColumnView], rows: &[Vec<Option<String>>]) {
    for (index, column) in columns.iter_mut().enumerate() {
        let values: Vec<&str> = rows
            .iter()
            .filter_map(|row| row.get(index))
            .filter_map(|cell| cell.as_deref())
            .collect();
        column.inferred_type = infer_type(&values).to_owned();
    }
}

fn trim_trailing_empty(row: &[Option<String>]) -> Vec<Option<String>> {
    let mut end = row.len();
    while end > 0 {
        let empty = row[end - 1]
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty);
        if !empty {
            break;
        }
        end -= 1;
    }
    row[..end].to_vec()
}

/// The leading raw rows the header picker offers as candidates.
pub fn header_candidates(sample: &Sample) -> Vec<Vec<Option<String>>> {
    sample
        .rows
        .iter()
        .take(HEADER_PICKER_SAMPLE_ROWS)
        .cloned()
        .collect()
}
