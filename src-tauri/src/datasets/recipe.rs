//! The cleanup a person applies to a staged file before it is uploaded.
//!
//! # One implementation, not two
//!
//! xfalcon-web has this transform twice: `applyRecipe` over the 50-row sample
//! to redraw the preview, and `applyRecipeToFile` over the real file to
//! produce what is uploaded. The preview is then only as honest as the two
//! agreeing. Here [`apply`] does both — the preview command runs it over the
//! sample, the upload runs it over every row — so the preview cannot disagree
//! with the result.
//!
//! # A refused step is not a skipped step
//!
//! Web's applier silently no-ops a step naming a column that does not exist,
//! an empty rename, or a colliding rename. That uploads data which differs
//! from the reviewed preview with nothing on screen to say so. Here
//! [`validate`] refuses, and names the step's position.
//!
//! # The output of a recipe is always a CSV
//!
//! Applied to a workbook, the result is written as a `.csv`. A recipe that
//! renames, splits, or casts has already changed the column shape, so
//! re-emitting a workbook would only add a second dialect for DA's reader to
//! disagree with. This matches web, whose cleaned upload is always
//! `<stem>_cleaned.csv`.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use calamine::{Reader, Xlsx};
use xfalcon_contracts::datasets::{
    DatasetCastType, DatasetColumnView, DatasetErrorCode, DatasetFilterOp, DatasetFormat,
    DatasetOperationError, DatasetRecipe, DatasetRecipeStep,
};

use crate::datasets::staging::StagedSource;

use crate::datasets::preview::{self, MAX_CELL_CHARS};

/// Steps one recipe may hold.
///
/// A recipe is a fixup for a file that is nearly right, not a transformation
/// language. The cap bounds both the work and the review a person has to do
/// before pressing upload.
pub const MAX_RECIPE_STEPS: usize = 32;

/// Largest file a recipe may be applied to.
///
/// Applying one means reading every row, so the ceiling is the read. Above it
/// the file still uploads — unshaped, which is what the 2 GiB path is for.
/// Matches web's `WRANGLE_FULL_READ_MAX_BYTES`, so the same file offers the
/// same controls in both products.
pub const RECIPE_FULL_READ_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// Columns one split step may produce. Web's own ceiling.
const MAX_SPLIT_PARTS: u32 = 32;

/// Joins a row's cells into its duplicate key.
///
/// The ASCII unit separator, because no cell can contain it: without a
/// separator `["ab", "c"]` and `["a", "bc"]` would key the same.
const DEDUP_SEPARATOR: char = '\u{1f}';

/// A recipe's effect on one table.
#[derive(Clone, Debug, Default)]
pub struct Shaped {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

/// Whether every step can be applied, in order, to the columns as they exist
/// at that point in the recipe.
///
/// Validated against the *evolving* column set: a step may name a column an
/// earlier rename or split created.
pub fn validate(recipe: &DatasetRecipe, columns: &[String]) -> Result<(), DatasetOperationError> {
    if recipe.steps.len() > MAX_RECIPE_STEPS {
        return Err(DatasetOperationError::new(DatasetErrorCode::RecipeTooLong));
    }
    let mut current: Vec<String> = columns.to_vec();
    for (position, step) in recipe.steps.iter().enumerate() {
        let index = u32::try_from(position).unwrap_or(u32::MAX);
        validate_step(step, &current).map_err(|()| DatasetOperationError::invalid_step(index))?;
        current = columns_after(step, &current);
    }
    Ok(())
}

fn validate_step(step: &DatasetRecipeStep, columns: &[String]) -> Result<(), ()> {
    let known = |name: &String| columns.iter().any(|existing| existing == name);
    match step {
        DatasetRecipeStep::Dedup => Ok(()),
        DatasetRecipeStep::Filter { column, op, value } => {
            if !known(column) {
                return Err(());
            }
            if op.takes_value() && value.as_deref().unwrap_or("").is_empty() {
                return Err(());
            }
            Ok(())
        }
        DatasetRecipeStep::Sort { column, .. }
        | DatasetRecipeStep::Cast { column, .. }
        | DatasetRecipeStep::Trim { column }
        | DatasetRecipeStep::Upper { column }
        | DatasetRecipeStep::Lower { column }
        | DatasetRecipeStep::RemoveNulls { column }
        | DatasetRecipeStep::FillNulls { column, .. } => {
            if known(column) {
                Ok(())
            } else {
                Err(())
            }
        }
        DatasetRecipeStep::Rename { column, new_name } => {
            if !known(column) {
                return Err(());
            }
            let trimmed = new_name.trim();
            if trimmed.is_empty() {
                return Err(());
            }
            // A collision would either overwrite a column or force a
            // suffix nobody asked for. Refusing lets the person choose.
            if columns
                .iter()
                .any(|existing| existing != column && existing == trimmed)
            {
                return Err(());
            }
            Ok(())
        }
        DatasetRecipeStep::Replace { column, find, .. } => {
            if !known(column) || find.is_empty() {
                return Err(());
            }
            Ok(())
        }
        DatasetRecipeStep::Split {
            column,
            delimiter,
            parts,
        } => {
            if !known(column) || delimiter.is_empty() || *parts < 1 {
                return Err(());
            }
            Ok(())
        }
    }
}

/// The column set a step leaves behind. Used by [`validate`] to check later
/// steps against the names they will actually see.
fn columns_after(step: &DatasetRecipeStep, columns: &[String]) -> Vec<String> {
    match step {
        DatasetRecipeStep::Rename { column, new_name } => {
            let trimmed = new_name.trim().to_owned();
            columns
                .iter()
                .map(|existing| {
                    if existing == column {
                        trimmed.clone()
                    } else {
                        existing.clone()
                    }
                })
                .collect()
        }
        DatasetRecipeStep::Split {
            column,
            parts,
            delimiter: _,
        } => {
            let Some(index) = columns.iter().position(|existing| existing == column) else {
                return columns.to_vec();
            };
            let mut next = columns[..=index].to_vec();
            next.extend(split_column_names(columns, column, *parts));
            next.extend_from_slice(&columns[index + 1..]);
            next
        }
        _ => columns.to_vec(),
    }
}

/// Names for a split step's new columns: `base_1`, `base_2`, … , probing
/// upward past any name already taken.
///
/// Case-insensitive, matching [`preview::dedup_column_names`] and DA's own
/// header normalisation, so a generated `Name_1` still collides with an
/// existing `name_1`.
fn split_column_names(columns: &[String], base: &str, parts: u32) -> Vec<String> {
    let capped = parts.min(MAX_SPLIT_PARTS);
    let mut taken: HashSet<String> = columns.iter().map(|name| name.to_lowercase()).collect();
    let mut names = Vec::with_capacity(capped as usize);
    for part in 1..=capped {
        let mut candidate = format!("{base}_{part}");
        let mut suffix = 2_u32;
        while taken.contains(&candidate.to_lowercase()) {
            candidate = format!("{base}_{part}_{suffix}");
            suffix += 1;
        }
        taken.insert(candidate.to_lowercase());
        names.push(candidate);
    }
    names
}

/// Applies a validated recipe.
///
/// Steps run in order, because two steps touching one column do not commute.
/// A step that cannot be applied returns an error rather than passing the
/// table through unchanged — [`validate`] should have caught it, and a silent
/// pass here would be the exact divergence this module exists to prevent.
pub fn apply(
    columns: &[String],
    rows: &[Vec<Option<String>>],
    recipe: &DatasetRecipe,
) -> Result<Shaped, DatasetOperationError> {
    // Every row is squared to the header's width first. A source row that is
    // short — a trailing comma the export omitted — has no cell for the last
    // column, and a step that addresses that column by position would then act
    // on some rows and silently skip others. The full-file path already squares
    // its rows, so doing it here is what makes the preview show the file.
    let width = columns.len();
    let mut shaped = Shaped {
        columns: columns.to_vec(),
        rows: rows
            .iter()
            .map(|row| {
                let mut row = row.clone();
                row.resize(width, None);
                row
            })
            .collect(),
    };
    for (position, step) in recipe.steps.iter().enumerate() {
        let index = u32::try_from(position).unwrap_or(u32::MAX);
        shaped =
            apply_step(shaped, step).map_err(|()| DatasetOperationError::invalid_step(index))?;
    }
    Ok(shaped)
}

fn apply_step(mut shaped: Shaped, step: &DatasetRecipeStep) -> Result<Shaped, ()> {
    match step {
        DatasetRecipeStep::Filter { column, op, value } => {
            let index = column_index(&shaped.columns, column)?;
            let target = value.as_deref().unwrap_or("");
            shaped
                .rows
                .retain(|row| keeps_row(row.get(index), *op, target));
            Ok(shaped)
        }
        DatasetRecipeStep::Sort { column, descending } => {
            let index = column_index(&shaped.columns, column)?;
            sort_rows(&mut shaped.rows, index, *descending);
            Ok(shaped)
        }
        DatasetRecipeStep::Rename { column, new_name } => {
            let index = column_index(&shaped.columns, column)?;
            let trimmed = new_name.trim();
            if trimmed.is_empty() {
                return Err(());
            }
            shaped.columns[index] = trimmed.to_owned();
            Ok(shaped)
        }
        DatasetRecipeStep::Cast { column, cast_to } => {
            let index = column_index(&shaped.columns, column)?;
            for row in &mut shaped.rows {
                if let Some(cell) = row.get_mut(index) {
                    *cell = coerce(cell.as_deref(), *cast_to);
                }
            }
            Ok(shaped)
        }
        DatasetRecipeStep::Trim { column } => {
            map_column(&mut shaped, column, |value| value.trim().to_owned())?;
            Ok(shaped)
        }
        DatasetRecipeStep::Upper { column } => {
            map_column(&mut shaped, column, |value| value.to_uppercase())?;
            Ok(shaped)
        }
        DatasetRecipeStep::Lower { column } => {
            map_column(&mut shaped, column, |value| value.to_lowercase())?;
            Ok(shaped)
        }
        DatasetRecipeStep::Replace {
            column,
            find,
            replace_with,
        } => {
            if find.is_empty() {
                return Err(());
            }
            map_column(&mut shaped, column, |value| {
                value.replace(find, replace_with)
            })?;
            Ok(shaped)
        }
        DatasetRecipeStep::Split {
            column,
            delimiter,
            parts,
        } => split_column(shaped, column, delimiter, *parts),
        DatasetRecipeStep::Dedup => {
            dedup_rows(&mut shaped.rows);
            Ok(shaped)
        }
        DatasetRecipeStep::RemoveNulls { column } => {
            let index = column_index(&shaped.columns, column)?;
            shaped.rows.retain(|row| !is_empty(row.get(index)));
            Ok(shaped)
        }
        DatasetRecipeStep::FillNulls { column, value } => {
            let index = column_index(&shaped.columns, column)?;
            for row in &mut shaped.rows {
                if let Some(cell) = row.get_mut(index) {
                    if is_empty(Some(&*cell)) {
                        *cell = Some(value.clone());
                    }
                }
            }
            Ok(shaped)
        }
    }
}

fn column_index(columns: &[String], name: &String) -> Result<usize, ()> {
    columns
        .iter()
        .position(|existing| existing == name)
        .ok_or(())
}

fn map_column(
    shaped: &mut Shaped,
    column: &String,
    transform: impl Fn(&str) -> String,
) -> Result<(), ()> {
    let index = column_index(&shaped.columns, column)?;
    for row in &mut shaped.rows {
        if let Some(cell) = row.get_mut(index) {
            if let Some(value) = cell.as_deref() {
                *cell = Some(transform(value));
            }
        }
    }
    Ok(())
}

fn split_column(
    mut shaped: Shaped,
    column: &String,
    delimiter: &str,
    parts: u32,
) -> Result<Shaped, ()> {
    if delimiter.is_empty() || parts < 1 {
        return Err(());
    }
    let index = column_index(&shaped.columns, column)?;
    let names = split_column_names(&shaped.columns, column, parts);
    let width = names.len();

    let mut columns = shaped.columns[..=index].to_vec();
    columns.extend(names);
    columns.extend_from_slice(&shaped.columns[index + 1..]);

    for row in &mut shaped.rows {
        let source = row.get(index).cloned().flatten();
        let pieces: Vec<Option<String>> = match source {
            Some(value) if !value.trim().is_empty() => {
                let mut tokens: Vec<Option<String>> = value
                    .split(delimiter)
                    .take(width)
                    .map(|token| Some(token.to_owned()))
                    .collect();
                tokens.resize(width, None);
                tokens
            }
            _ => vec![None; width],
        };
        let tail = if row.len() > index + 1 {
            row.split_off(index + 1)
        } else {
            Vec::new()
        };
        row.extend(pieces);
        row.extend(tail);
    }
    shaped.columns = columns;
    Ok(shaped)
}

fn dedup_rows(rows: &mut Vec<Vec<Option<String>>>) {
    let mut seen: HashSet<String> = HashSet::new();
    rows.retain(|row| seen.insert(dedup_key(row)));
}

/// A row's duplicate key.
///
/// [`apply`] squares every row to the header's width before the first step
/// runs, so two rows key identically exactly when their cells do. A cell the
/// source row did not have and an empty cell key the same, because they are the
/// same value in the CSV this writes.
fn dedup_key(row: &[Option<String>]) -> String {
    row.iter()
        .map(|cell| cell.as_deref().unwrap_or(""))
        .collect::<Vec<&str>>()
        .join(&DEDUP_SEPARATOR.to_string())
}

fn is_empty(cell: Option<&Option<String>>) -> bool {
    match cell {
        None | Some(None) => true,
        Some(Some(value)) => value.trim().is_empty(),
    }
}

fn keeps_row(cell: Option<&Option<String>>, op: DatasetFilterOp, target: &str) -> bool {
    match op {
        DatasetFilterOp::IsNull => return is_empty(cell),
        DatasetFilterOp::IsNotNull => return !is_empty(cell),
        _ => {}
    }
    let value = cell.and_then(Option::as_deref).unwrap_or("");
    if op == DatasetFilterOp::Contains {
        return value.to_lowercase().contains(&target.to_lowercase());
    }
    let ordering = compare_cells(value, target);
    match op {
        DatasetFilterOp::Equals => ordering == std::cmp::Ordering::Equal,
        DatasetFilterOp::NotEquals => ordering != std::cmp::Ordering::Equal,
        DatasetFilterOp::GreaterThan => ordering == std::cmp::Ordering::Greater,
        DatasetFilterOp::LessThan => ordering == std::cmp::Ordering::Less,
        DatasetFilterOp::AtLeast => ordering != std::cmp::Ordering::Less,
        DatasetFilterOp::AtMost => ordering != std::cmp::Ordering::Greater,
        // Handled above; listed so a new operator cannot compile into a
        // silent `true`.
        DatasetFilterOp::Contains | DatasetFilterOp::IsNull | DatasetFilterOp::IsNotNull => true,
    }
}

fn sort_rows(rows: &mut [Vec<Option<String>>], index: usize, descending: bool) {
    // Stable, so rows that compare equal keep the file's order.
    rows.sort_by(|left, right| {
        let left_cell = left.get(index);
        let right_cell = right.get(index);
        // Empty cells sort last whichever direction is asked for: a column of
        // gaps otherwise fills the top of a descending sort with nothing.
        match (is_empty(left_cell), is_empty(right_cell)) {
            (true, true) => return std::cmp::Ordering::Equal,
            (true, false) => return std::cmp::Ordering::Greater,
            (false, true) => return std::cmp::Ordering::Less,
            (false, false) => {}
        }
        let ordering = compare_cells(
            left_cell.and_then(Option::as_deref).unwrap_or(""),
            right_cell.and_then(Option::as_deref).unwrap_or(""),
        );
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
}

/// Three-way compare for filter and sort: numeric when both sides are clean
/// numeric literals, chronological when both are ISO dates, lexical
/// otherwise.
///
/// Never compares two numbers as text — `"10" > "9"` is the classic wrong
/// answer — and never compares two dates as text either.
fn compare_cells(left: &str, right: &str) -> std::cmp::Ordering {
    if let (Some(left_number), Some(right_number)) =
        (parse_strict_number(left), parse_strict_number(right))
    {
        return left_number
            .partial_cmp(&right_number)
            .unwrap_or(std::cmp::Ordering::Equal);
    }
    if let (Some(left_date), Some(right_date)) = (iso_date_key(left), iso_date_key(right)) {
        return left_date.cmp(&right_date);
    }
    left.cmp(right)
}

/// A number only when the whole trimmed value is one: an optional currency
/// sign, optional well-formed thousands grouping, and nothing else.
///
/// `1,2,3` is not 123. `50%` is not 0.5 — a percent means something only when
/// a person explicitly casts the column. `12abc` is not 12.
fn parse_strict_number(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let body = trimmed
        .strip_prefix(['$', '£', '€', '¥'])
        .map(str::trim_start)
        .unwrap_or(trimmed);
    if body.is_empty() || body.contains('%') {
        return None;
    }
    let candidate = if is_grouped_number(body) {
        body.replace(',', "")
    } else {
        body.to_owned()
    };
    if !is_plain_number(&candidate) {
        return None;
    }
    candidate.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// `1,234` or `1,234.56`: one to three digits, then groups of exactly three.
/// `1,23` and `1,2,3` are not numbers.
fn is_grouped_number(body: &str) -> bool {
    let unsigned = body.strip_prefix(['+', '-']).unwrap_or(body);
    let (integer, fraction) = match unsigned.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (unsigned, None),
    };
    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    let mut groups = integer.split(',');
    let Some(first) = groups.next() else {
        return false;
    };
    if first.is_empty() || first.len() > 3 || !first.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let mut had_group = false;
    for group in groups {
        if group.len() != 3 || !group.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        had_group = true;
    }
    had_group
}

/// `12`, `-3.14`, `.5`, `1e3` — a bare decimal literal, optionally in
/// scientific notation.
fn is_plain_number(candidate: &str) -> bool {
    let unsigned = candidate.strip_prefix(['+', '-']).unwrap_or(candidate);
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (unsigned, None),
    };
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    let (integer, fraction) = match mantissa.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (mantissa, ""),
    };
    if integer.is_empty() && fraction.is_empty() {
        return false;
    }
    integer.chars().all(|c| c.is_ascii_digit()) && fraction.chars().all(|c| c.is_ascii_digit())
}

/// A sortable key for an ISO-8601 date or date-time, and nothing else.
///
/// Only ISO shapes qualify, so `2024` is a number rather than a year, and a
/// locale-ordered `03/04/2024` stays a string — guessing whether that is March
/// or April is not this module's decision to make.
fn iso_date_key(value: &str) -> Option<(i32, u32, u32, u32, u32, u32)> {
    let trimmed = value.trim();
    let (date, time) = match trimmed.split_once(['T', ' ']) {
        Some((date, time)) => (date, Some(time)),
        None => (trimmed, None),
    };
    let mut parts = date.split('-');
    let year = parts.next()?;
    let month = parts.next()?;
    let day = parts.next()?;
    if parts.next().is_some() || year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let year = year.parse::<i32>().ok()?;
    let month = month.parse::<u32>().ok().filter(|m| (1..=12).contains(m))?;
    let day = day.parse::<u32>().ok().filter(|d| (1..=31).contains(d))?;
    let Some(time) = time else {
        return Some((year, month, day, 0, 0, 0));
    };
    // Trailing zone designators do not change the ordering of values from one
    // file, and honouring an offset here would need a calendar this module
    // does not carry.
    let time = time
        .trim_end_matches('Z')
        .split(['+'])
        .next()
        .unwrap_or(time);
    let mut clock = time.split(':');
    let hour = clock.next()?.parse::<u32>().ok().filter(|h| *h < 24)?;
    let minute = clock.next()?.parse::<u32>().ok().filter(|m| *m < 60)?;
    let second = clock
        .next()
        .and_then(|second| second.split('.').next())
        .map_or(Some(0), |second| {
            second.parse::<u32>().ok().filter(|s| *s < 60)
        })?;
    Some((year, month, day, hour, minute, second))
}

/// Casts one cell. A value the cast cannot read is left as it was, so an
/// explicit cast never destroys data it did not understand.
fn coerce(value: Option<&str>, cast_to: DatasetCastType) -> Option<String> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    match cast_to {
        DatasetCastType::Text => Some(value.to_owned()),
        DatasetCastType::Number => {
            Some(parse_number_for_cast(value).map_or_else(|| value.to_owned(), format_number))
        }
        DatasetCastType::Integer => Some(parse_number_for_cast(value).map_or_else(
            || value.to_owned(),
            |number| format!("{}", number.trunc() as i64),
        )),
        DatasetCastType::Boolean => Some(match value.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "t" => "true".to_owned(),
            "false" | "0" | "no" | "n" | "f" => "false".to_owned(),
            _ => value.to_owned(),
        }),
        DatasetCastType::Date => Some(coerce_date(value).map_or_else(
            || value.to_owned(),
            |(y, m, d)| format!("{y:04}-{m:02}-{d:02}"),
        )),
    }
}

/// A number for an explicit cast. Same rules as [`parse_strict_number`] plus a
/// trailing percent, which becomes a fraction — the person asked for a number,
/// so `50%` means 0.5 here where it means "not a number" in a comparison.
fn parse_number_for_cast(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if let Some(body) = trimmed.strip_suffix('%') {
        return parse_strict_number(body).map(|number| number / 100.0);
    }
    parse_strict_number(trimmed)
}

/// Renders a cast number without a trailing `.0`, so an integral value does
/// not gain a decimal point it did not have in the file.
fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        return format!("{}", number as i64);
    }
    number.to_string()
}

/// The calendar date a value denotes, as year, month, day.
///
/// A date is built from its parts rather than through any local-time
/// conversion: a date read as local midnight and written back as UTC is off by
/// one for half the world, and it would be off by one *in the uploaded file*.
/// A rolled-over date — `2024-02-31` — is refused rather than silently moved.
fn coerce_date(value: &str) -> Option<(i32, u32, u32)> {
    let trimmed = value.trim();
    if let Some((year, month, day, ..)) = iso_date_key(trimmed) {
        return valid_date(year, month, day);
    }
    let mut parts = trimmed.split(['/', '-']);
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if [first, second, third]
        .iter()
        .any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let first_number = first.parse::<i32>().ok()?;
    let second_number = second.parse::<u32>().ok()?;
    let third_number = third.parse::<u32>().ok()?;
    // A leading four-digit field is a year, so the layout is Y/M/D.
    // Otherwise the year is trailing, which leaves M/D/Y as the only reading
    // that does not require guessing.
    if first.len() == 4 {
        valid_date(first_number, second_number, third_number)
    } else {
        valid_date(third_number as i32, first_number as u32, second_number)
    }
}

fn valid_date(year: i32, month: u32, day: u32) -> Option<(i32, u32, u32)> {
    if !(1..=12).contains(&month) || day < 1 {
        return None;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day > length {
        return None;
    }
    Some((year, month, day))
}

/// Reads every row of a staged file, applies a recipe, and writes the result
/// as a CSV beside it in the staging directory.
///
/// Returns the written path and a lease. The upload queue holds the lease
/// through transfer retries and releases it after success or cancellation.
pub fn materialize(
    source: &StagedSource,
    worksheet_index: u32,
    header_row_index: usize,
    recipe: &DatasetRecipe,
    staging_dir: &Path,
    cleanup: &crate::security::temp_cleanup::TempCleanup,
) -> Result<(PathBuf, crate::security::temp_cleanup::TempLease), DatasetOperationError> {
    if source.size > RECIPE_FULL_READ_MAX_BYTES {
        return Err(DatasetOperationError::new(
            DatasetErrorCode::RecipeFileTooLarge,
        ));
    }
    let raw = read_all_rows(&source.path, source.format, source.size, worksheet_index)
        .map_err(DatasetOperationError::new)?;
    if raw.is_empty() {
        return Err(DatasetOperationError::new(DatasetErrorCode::EmptyFile));
    }
    let clamped = header_row_index.min(raw.len().saturating_sub(1));
    let columns = header_names(&raw[clamped]);
    if columns.is_empty() {
        return Err(DatasetOperationError::new(DatasetErrorCode::EmptyFile));
    }
    let rows = data_rows(&raw, clamped, columns.len());
    validate(recipe, &columns)?;
    let shaped = apply(&columns, &rows, recipe)?;

    let stem = source
        .path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("dataset");
    // Its own directory, named by nothing but a fresh id. Two files with the
    // same stem — `sales.csv` from two folders, or the same file cleaned twice
    // — would otherwise write the same path, and the second write would
    // truncate a file the first transfer is still reading. The name inside is
    // kept readable because DA records it as the dataset's `original_filename`.
    let id = uuid::Uuid::new_v4();
    let directory = staging_dir.join(id.to_string());
    let lease = cleanup
        .lease_directory(staging_dir, id)
        .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Storage))?;
    std::fs::create_dir_all(staging_dir)
        .and_then(|()| std::fs::create_dir(&directory))
        .map_err(|_| DatasetOperationError::new(DatasetErrorCode::Storage))?;
    let destination = directory.join(format!("{stem}_cleaned.csv"));
    write_csv(&destination, &shaped).map_err(DatasetOperationError::new)?;
    Ok((destination, lease))
}

/// The header row's names, resolved the same way the preview resolves them so
/// the cleaned file's columns are the ones the person reviewed.
fn header_names(header: &[Option<String>]) -> Vec<String> {
    let mut end = header.len();
    while end > 0 && header[end - 1].as_deref().unwrap_or("").trim().is_empty() {
        end -= 1;
    }
    let raw: Vec<String> = header[..end]
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
    preview::dedup_column_names(&raw).0
}

/// Every populated row below the header, padded or cut to the header's width.
///
/// A stray trailing cell in the source would otherwise make the column count
/// vary row by row, and a recipe that addresses a column by position would
/// then act on different data in different rows.
fn data_rows(
    raw: &[Vec<Option<String>>],
    header_row_index: usize,
    width: usize,
) -> Vec<Vec<Option<String>>> {
    raw.iter()
        .skip(header_row_index + 1)
        .filter(|row| {
            row.iter()
                .any(|cell| !cell.as_deref().unwrap_or("").trim().is_empty())
        })
        .map(|row| {
            let mut row = row.clone();
            row.resize(width, None);
            row
        })
        .collect()
}

fn read_all_rows(
    source: &Path,
    format: DatasetFormat,
    size: u64,
    worksheet_index: u32,
) -> Result<Vec<Vec<Option<String>>>, DatasetErrorCode> {
    match format {
        DatasetFormat::Csv => read_all_delimited(source, size),
        DatasetFormat::Xlsx => read_all_worksheet(source, worksheet_index),
    }
}

/// Reads every row with the delimiter the preview guessed.
///
/// Guessed rather than assumed: a semicolon-separated export read as
/// comma-separated is one column wide, so a recipe validated against the
/// twelve columns the person reviewed would be refused — or, if it named no
/// column at all, would quietly upload a single-column table.
fn read_all_delimited(
    source: &Path,
    size: u64,
) -> Result<Vec<Vec<Option<String>>>, DatasetErrorCode> {
    let delimiter =
        preview::guess_delimiter(source, size).map_err(|_| DatasetErrorCode::ContentMismatch)?;
    let file = File::open(source).map_err(|_| DatasetErrorCode::Unreadable)?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .has_headers(false)
        .from_reader(BufReader::new(file));
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|_| DatasetErrorCode::Unreadable)?;
        if record.iter().any(|field| field.len() > MAX_CELL_CHARS) {
            return Err(DatasetErrorCode::Unreadable);
        }
        rows.push(
            record
                .iter()
                .map(|field| {
                    if field.is_empty() {
                        None
                    } else {
                        Some(field.to_owned())
                    }
                })
                .collect(),
        );
    }
    Ok(rows)
}

fn read_all_worksheet(
    source: &Path,
    worksheet_index: u32,
) -> Result<Vec<Vec<Option<String>>>, DatasetErrorCode> {
    let file = File::open(source).map_err(|_| DatasetErrorCode::Unreadable)?;
    let mut workbook: Xlsx<BufReader<File>> =
        Xlsx::new(BufReader::new(file)).map_err(|_| DatasetErrorCode::ContentMismatch)?;
    let names = workbook.sheet_names();
    if names.is_empty() {
        return Err(DatasetErrorCode::ContentMismatch);
    }
    let index = (worksheet_index as usize).min(names.len() - 1);
    let range = workbook
        .worksheet_range(&names[index])
        .map_err(|_| DatasetErrorCode::Unreadable)?;
    Ok(range
        .rows()
        .map(|row| row.iter().map(cell_to_owned).collect())
        .collect())
}

fn cell_to_owned(cell: &calamine::Data) -> Option<String> {
    use calamine::Data;
    let text = match cell {
        Data::Empty => return None,
        Data::String(text) => text.clone(),
        Data::Int(number) => number.to_string(),
        Data::Float(number) => format_number(*number),
        Data::Bool(flag) => flag.to_string(),
        Data::DateTimeIso(text) | Data::DurationIso(text) => text.clone(),
        Data::DateTime(stamp) => {
            if stamp.is_duration() {
                stamp.as_f64().to_string()
            } else {
                let (year, month, day, hour, minute, second, _) = stamp.to_ymd_hms_milli();
                if hour == 0 && minute == 0 && second == 0 {
                    format!("{year:04}-{month:02}-{day:02}")
                } else {
                    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
                }
            }
        }
        Data::Error(error) => error.to_string(),
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn write_csv(destination: &Path, shaped: &Shaped) -> Result<(), DatasetErrorCode> {
    let file = File::create(destination).map_err(|_| DatasetErrorCode::Storage)?;
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_writer(BufWriter::new(file));
    writer
        .write_record(&shaped.columns)
        .map_err(|_| DatasetErrorCode::Storage)?;
    for row in &shaped.rows {
        writer
            .write_record(row.iter().map(|cell| cell.as_deref().unwrap_or("")))
            .map_err(|_| DatasetErrorCode::Storage)?;
    }
    writer.flush().map_err(|_| DatasetErrorCode::Storage)?;
    Ok(())
}

/// Turns the preview's column views into the plain names the recipe addresses.
pub fn column_names(columns: &[DatasetColumnView]) -> Vec<String> {
    columns.iter().map(|column| column.name.clone()).collect()
}
