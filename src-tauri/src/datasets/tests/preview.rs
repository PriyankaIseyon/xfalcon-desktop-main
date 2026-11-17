use xfalcon_contracts::datasets::{DatasetFormat, DatasetPreviewUnavailable};

use crate::datasets::preview::{
    data_rows, dedup_column_names, header_at, header_candidates, label_columns, read_sample,
    suggest_header_row, worksheet_names, HEADER_PICKER_SAMPLE_ROWS, MAX_PREVIEW_COLUMNS,
    PREVIEW_CELL_MAX_CHARS, PREVIEW_ROW_LIMIT, WORKBOOK_PREVIEW_MAX_BYTES,
};
use crate::datasets::tests::fixtures;

fn sample_of(contents: &str) -> crate::datasets::preview::Sample {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_file(directory.path(), "data.csv", contents.as_bytes());
    let size = contents.len() as u64;
    read_sample(&path, DatasetFormat::Csv, size, 0).expect("sample")
}

fn names(sample: &crate::datasets::preview::Sample, header_row: usize) -> Vec<String> {
    header_at(sample, header_row)
        .columns
        .into_iter()
        .map(|column| column.name)
        .collect()
}

#[test]
fn a_csv_reads_its_header_and_rows() {
    let sample = sample_of("region,revenue\nEMEA,10\nAPAC,20\n");
    assert_eq!(names(&sample, 0), vec!["region", "revenue"]);
    let (rows, more) = data_rows(&sample, 0);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][0].as_deref(), Some("EMEA"));
    assert!(!more);
}

#[test]
fn an_absent_cell_is_null_and_not_an_empty_string() {
    // The difference decides whether a remove-nulls step drops the row, so it
    // has to survive the read rather than being flattened to "".
    let sample = sample_of("region,revenue\nEMEA,\n");
    let (rows, _) = data_rows(&sample, 0);
    assert_eq!(rows[0][1], None);
}

#[test]
fn rfc4180_quoting_survives_the_read() {
    let sample = sample_of("a,b\n\"has, comma\",\"line\nbreak\"\n\"quote\"\"inside\",x\n");
    let (rows, _) = data_rows(&sample, 0);
    assert_eq!(rows[0][0].as_deref(), Some("has, comma"));
    assert_eq!(rows[0][1].as_deref(), Some("line\nbreak"));
    assert_eq!(rows[1][0].as_deref(), Some("quote\"inside"));
}

#[test]
fn a_tab_separated_file_is_read_as_one() {
    let sample = sample_of("region\trevenue\nEMEA\t10\n");
    assert_eq!(names(&sample, 0), vec!["region", "revenue"]);
}

#[test]
fn a_semicolon_separated_file_is_read_as_one() {
    let sample = sample_of("region;revenue\nEMEA;10\n");
    assert_eq!(names(&sample, 0), vec!["region", "revenue"]);
}

#[test]
fn a_single_column_file_is_still_a_table() {
    let sample = sample_of("region\nEMEA\nAPAC\n");
    assert_eq!(names(&sample, 0), vec!["region"]);
    assert_eq!(data_rows(&sample, 0).0.len(), 2);
}

#[test]
fn more_rows_than_the_limit_report_truncation() {
    let mut contents = String::from("n\n");
    for row in 0..(PREVIEW_ROW_LIMIT + 20) {
        contents.push_str(&format!("{row}\n"));
    }
    let sample = sample_of(&contents);
    let (rows, more) = data_rows(&sample, 0);
    assert_eq!(rows.len(), PREVIEW_ROW_LIMIT);
    assert!(
        more,
        "a prefix presented as the whole file is the one lie here"
    );
}

#[test]
fn a_row_wider_than_the_column_cap_is_cut_and_reported() {
    let header: Vec<String> = (0..(MAX_PREVIEW_COLUMNS + 10))
        .map(|index| format!("c{index}"))
        .collect();
    let contents = format!("{}\n", header.join(","));
    let sample = sample_of(&contents);
    assert!(sample.truncated_columns);
    assert_eq!(sample.rows[0].len(), MAX_PREVIEW_COLUMNS);
}

#[test]
fn a_cell_longer_than_the_display_cap_is_cut_and_reported() {
    let long = "x".repeat(PREVIEW_CELL_MAX_CHARS + 100);
    let sample = sample_of(&format!("a\n{long}\n"));
    assert!(sample.truncated_cell);
    let (rows, _) = data_rows(&sample, 0);
    assert_eq!(
        rows[0][0].as_deref().map(str::len),
        Some(PREVIEW_CELL_MAX_CHARS)
    );
}

#[test]
fn a_file_with_no_line_break_in_its_scan_window_is_not_delimited() {
    let directory = tempfile::tempdir().expect("temp dir");
    // Larger than the scan window, and without a single record separator, so
    // the answer is conclusive rather than "a short file with no trailing
    // newline".
    let contents = vec![b'a'; (super::super::preview::NO_NEWLINE_SCAN_BYTES + 16) as usize];
    let path = fixtures::write_file(directory.path(), "blob.csv", &contents);
    assert_eq!(
        read_sample(&path, DatasetFormat::Csv, contents.len() as u64, 0),
        Err(DatasetPreviewUnavailable::NotDelimited)
    );
}

#[test]
fn a_short_file_with_no_trailing_newline_is_one_record() {
    let sample = sample_of("region,revenue");
    assert_eq!(names(&sample, 0), vec!["region", "revenue"]);
}

#[test]
fn a_title_row_above_the_header_is_suggested_past() {
    let sample = sample_of(
        "Daily product spend,,\n,,\nregion,product,revenue\nEMEA,widget,10\nAPAC,widget,20\n",
    );
    assert_eq!(
        suggest_header_row(&sample.rows),
        2,
        "a one-cell title row is not a header"
    );
    assert_eq!(names(&sample, 2), vec!["region", "product", "revenue"]);
    let (rows, _) = data_rows(&sample, 2);
    assert_eq!(rows.len(), 2, "rows above the header are not data");
}

#[test]
fn a_header_row_wider_than_its_data_is_still_found() {
    // Trailing blanks in the data leave the header as the widest row, which is
    // the case a "header must be as wide as the data" rule gets wrong.
    let sample = sample_of("a,b,c,d\n1,2,,\n3,4,,\n");
    assert_eq!(suggest_header_row(&sample.rows), 0);
}

#[test]
fn a_numeric_first_row_is_not_read_as_a_header() {
    let sample = sample_of("1,2,3\nregion,product,revenue\nEMEA,widget,10\n");
    assert_eq!(suggest_header_row(&sample.rows), 1);
}

#[test]
fn a_suggested_header_row_is_always_one_the_picker_offers() {
    // The picker offers the first HEADER_PICKER_SAMPLE_ROWS rows. A suggestion
    // below that window could not be selected, so the control would show a
    // different row than the preview was built from.
    let mut contents = String::new();
    for row in 0..HEADER_PICKER_SAMPLE_ROWS + 4 {
        contents.push_str(&format!("banner {row}\n"));
    }
    contents.push_str("region,product,revenue\nEMEA,widget,10\n");
    let sample = sample_of(&contents);

    let suggested = suggest_header_row(&sample.rows);
    assert!(
        (suggested as usize) < HEADER_PICKER_SAMPLE_ROWS,
        "suggested row {suggested} is outside the picker's candidates"
    );
    assert_eq!(
        header_candidates(&sample).len(),
        HEADER_PICKER_SAMPLE_ROWS,
        "the candidates are the window the suggestion has to fall inside"
    );
}

#[test]
fn a_single_row_file_keeps_the_first_row_as_its_header() {
    let sample = sample_of("region,revenue\n");
    assert_eq!(suggest_header_row(&sample.rows), 0);
}

#[test]
fn blank_rows_below_the_header_are_dropped_before_the_window() {
    // Filtering after windowing would show fewer than the limit and would
    // miscount whether more rows exist.
    let mut contents = String::from("n\n");
    for row in 0..PREVIEW_ROW_LIMIT {
        contents.push_str(&format!("{row}\n"));
        contents.push('\n');
    }
    let sample = sample_of(&contents);
    let (rows, _) = data_rows(&sample, 0);
    assert!(rows.iter().all(|row| row.iter().any(|cell| cell.is_some())));
}

#[test]
fn a_blank_header_cell_becomes_an_addressable_name() {
    let sample = sample_of("region,,revenue\nEMEA,x,10\n");
    assert_eq!(names(&sample, 0), vec!["region", "col_2", "revenue"]);
}

#[test]
fn trailing_empty_header_cells_are_dropped() {
    let sample = sample_of("region,revenue,,\nEMEA,10,,\n");
    assert_eq!(names(&sample, 0), vec!["region", "revenue"]);
}

#[test]
fn duplicate_column_names_are_disambiguated_the_way_da_does_it() {
    let (names, renamed) = dedup_column_names(&[
        "foo".to_owned(),
        "foo".to_owned(),
        "FOO".to_owned(),
        "bar".to_owned(),
    ]);
    assert_eq!(names, vec!["foo", "foo_2", "FOO_3", "bar"]);
    assert_eq!(renamed, 2);
}

#[test]
fn a_name_already_suffixed_in_the_source_is_respected() {
    let (names, renamed) =
        dedup_column_names(&["foo".to_owned(), "foo_2".to_owned(), "foo".to_owned()]);
    assert_eq!(names, vec!["foo", "foo_2", "foo_3"]);
    assert_eq!(renamed, 1);
}

#[test]
fn the_header_picker_gets_the_leading_raw_rows() {
    let mut contents = String::new();
    for row in 0..(HEADER_PICKER_SAMPLE_ROWS + 5) {
        contents.push_str(&format!("r{row},x\n"));
    }
    let sample = sample_of(&contents);
    let candidates = header_candidates(&sample);
    assert_eq!(candidates.len(), HEADER_PICKER_SAMPLE_ROWS);
    assert_eq!(candidates[0][0].as_deref(), Some("r0"));
}

#[test]
fn a_column_is_labelled_with_the_same_inference_the_projects_surface_uses() {
    let sample = sample_of("name,count,ratio,flag\na,1,1.5,true\nb,2,2.5,false\n");
    let mut header = header_at(&sample, 0);
    let (rows, _) = data_rows(&sample, 0);
    label_columns(&mut header.columns, &rows);
    let labels: Vec<&str> = header
        .columns
        .iter()
        .map(|column| column.inferred_type.as_str())
        .collect();
    assert_eq!(labels, vec!["string", "integer", "float", "boolean"]);
}

#[test]
fn a_workbook_reads_its_worksheet_names() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_workbook(
        directory.path(),
        "book.xlsx",
        &[
            fixtures::Sheet {
                name: "Q3 Actuals",
                rows: &[&["region", "revenue"], &["EMEA", "10"]],
            },
            fixtures::Sheet {
                name: "Notes",
                rows: &[&["note"], &["draft"]],
            },
        ],
    );
    let size = std::fs::metadata(&path).expect("metadata").len();
    assert_eq!(worksheet_names(&path, size), vec!["Q3 Actuals", "Notes"]);
}

#[test]
fn a_workbook_reads_the_chosen_worksheet() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_workbook(
        directory.path(),
        "book.xlsx",
        &[
            fixtures::Sheet {
                name: "First",
                rows: &[&["a"], &["1"]],
            },
            fixtures::Sheet {
                name: "Second",
                rows: &[&["region", "revenue"], &["EMEA", "10"]],
            },
        ],
    );
    let size = std::fs::metadata(&path).expect("metadata").len();
    let sample = read_sample(&path, DatasetFormat::Xlsx, size, 1).expect("sample");
    assert_eq!(sample.worksheet_index, 1);
    assert_eq!(names(&sample, 0), vec!["region", "revenue"]);
}

#[test]
fn a_worksheet_index_past_the_last_sheet_clamps() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_workbook(
        directory.path(),
        "book.xlsx",
        &[fixtures::Sheet {
            name: "Only",
            rows: &[&["a"], &["1"]],
        }],
    );
    let size = std::fs::metadata(&path).expect("metadata").len();
    let sample = read_sample(&path, DatasetFormat::Xlsx, size, 9).expect("sample");
    assert_eq!(sample.worksheet_index, 0);
}

#[test]
fn a_gap_in_a_worksheet_row_reads_as_a_null_cell() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_workbook(
        directory.path(),
        "book.xlsx",
        &[fixtures::Sheet {
            name: "Data",
            rows: &[&["a", "b", "c"], &["1", "", "3"]],
        }],
    );
    let size = std::fs::metadata(&path).expect("metadata").len();
    let sample = read_sample(&path, DatasetFormat::Xlsx, size, 0).expect("sample");
    let (rows, _) = data_rows(&sample, 0);
    assert_eq!(rows[0][1], None);
    assert_eq!(rows[0][2].as_deref(), Some("3"));
}

#[test]
fn a_workbook_past_the_parse_ceiling_reports_that_and_nothing_else() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = fixtures::write_workbook(
        directory.path(),
        "book.xlsx",
        &[fixtures::Sheet {
            name: "Data",
            rows: &[&["a"], &["1"]],
        }],
    );
    let oversize = WORKBOOK_PREVIEW_MAX_BYTES + 1;
    assert_eq!(
        read_sample(&path, DatasetFormat::Xlsx, oversize, 0),
        Err(DatasetPreviewUnavailable::FileTooLarge)
    );
    assert!(
        worksheet_names(&path, oversize).is_empty(),
        "no sheet names either, so the surface offers manual entry"
    );
}

#[test]
fn a_large_file_is_read_to_its_bounds_and_no_further() {
    // Half a million rows, which is the shape a real export has and far past
    // anything the preview shows. The claim is not about speed: it is that the
    // reader stops, so a 2 GiB pick costs a sample rather than a full read.
    let directory = tempfile::tempdir().expect("temp dir");
    let mut contents = String::from("region,month,revenue\n");
    for row in 0..500_000 {
        contents.push_str(&format!("EMEA,{},{}\n", row % 12, row));
    }
    let path = fixtures::write_file(directory.path(), "big.csv", contents.as_bytes());
    let size = contents.len() as u64;

    let sample = read_sample(&path, DatasetFormat::Csv, size, 0).expect("sample");
    assert_eq!(
        sample.rows.len(),
        PREVIEW_ROW_LIMIT + HEADER_PICKER_SAMPLE_ROWS,
        "the reader stops at the sample cap, header candidates included"
    );
    assert!(sample.truncated);

    let (rows, more) = data_rows(&sample, 0);
    assert_eq!(rows.len(), PREVIEW_ROW_LIMIT);
    assert!(more);
}

#[test]
fn a_workbook_with_many_worksheets_names_them_all_and_reads_only_one() {
    let directory = tempfile::tempdir().expect("temp dir");
    let names: Vec<String> = (0..50).map(|index| format!("Sheet{index}")).collect();
    let rows: Vec<&[&str]> = vec![&["a", "b"], &["1", "2"]];
    let sheets: Vec<fixtures::Sheet<'_>> = names
        .iter()
        .map(|name| fixtures::Sheet {
            name: name.as_str(),
            rows: &rows,
        })
        .collect();
    let path = fixtures::write_workbook(directory.path(), "many.xlsx", &sheets);
    let size = std::fs::metadata(&path).expect("metadata").len();

    assert_eq!(worksheet_names(&path, size).len(), 50);
    let sample = read_sample(&path, DatasetFormat::Xlsx, size, 49).expect("sample");
    assert_eq!(sample.worksheet_index, 49);
    assert_eq!(sample.rows.len(), 2, "one worksheet's rows, not fifty");
}
