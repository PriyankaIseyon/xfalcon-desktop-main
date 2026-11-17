use xfalcon_contracts::datasets::{
    DatasetCastType, DatasetErrorCode, DatasetFilterOp, DatasetFormat, DatasetOperationError,
    DatasetRecipe, DatasetRecipeStep,
};

use crate::datasets::recipe::{
    apply, materialize as materialize_with_lease, validate, MAX_RECIPE_STEPS,
    RECIPE_FULL_READ_MAX_BYTES,
};
use crate::datasets::staging::StagedSource;
use crate::datasets::tests::fixtures;
use crate::security::temp_cleanup::TempCleanup;

fn columns(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn rows(grid: &[&[&str]]) -> Vec<Vec<Option<String>>> {
    grid.iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    if cell.is_empty() {
                        None
                    } else {
                        Some((*cell).to_owned())
                    }
                })
                .collect()
        })
        .collect()
}

fn recipe(steps: Vec<DatasetRecipeStep>) -> DatasetRecipe {
    DatasetRecipe { steps }
}

// Byte-focused recipe tests still exercise the production writer with an
// owned lease. The test helper drops the lease after returning the path.
fn materialize(
    source: &std::path::Path,
    format: DatasetFormat,
    size: u64,
    worksheet_index: u32,
    header_row_index: usize,
    recipe: &DatasetRecipe,
    staging: &std::path::Path,
) -> Result<std::path::PathBuf, DatasetOperationError> {
    let cleanup = TempCleanup::new([staging.to_path_buf()]);
    let staged = StagedSource {
        path: source.to_path_buf(),
        size,
        format,
        file_name: source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("dataset")
            .to_owned(),
    };
    materialize_with_lease(
        &staged,
        worksheet_index,
        header_row_index,
        recipe,
        staging,
        &cleanup,
    )
    .map(|(path, _lease)| path)
}

fn shaped_cells(shaped: &crate::datasets::recipe::Shaped) -> Vec<Vec<&str>> {
    shaped
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.as_deref().unwrap_or(""))
                .collect()
        })
        .collect()
}

#[test]
fn a_step_naming_an_unknown_column_is_refused_with_its_position() {
    let error = validate(
        &recipe(vec![
            DatasetRecipeStep::Trim {
                column: "region".to_owned(),
            },
            DatasetRecipeStep::Trim {
                column: "revenue".to_owned(),
            },
            DatasetRecipeStep::Rename {
                column: "missing".to_owned(),
                new_name: "x".to_owned(),
            },
        ]),
        &columns(&["region", "revenue"]),
    )
    .expect_err("a step naming a missing column is not applicable");
    assert_eq!(error.code, DatasetErrorCode::RecipeInvalidStep);
    assert_eq!(
        error.step_index,
        Some(2),
        "the person has to know which step"
    );
}

#[test]
fn a_step_naming_a_column_an_earlier_step_created_validates() {
    validate(
        &recipe(vec![
            DatasetRecipeStep::Rename {
                column: "region".to_owned(),
                new_name: "market".to_owned(),
            },
            DatasetRecipeStep::Upper {
                column: "market".to_owned(),
            },
        ]),
        &columns(&["region", "revenue"]),
    )
    .expect("a rename's output is addressable by the next step");
}

#[test]
fn a_step_naming_a_column_an_earlier_rename_consumed_is_refused() {
    let error = validate(
        &recipe(vec![
            DatasetRecipeStep::Rename {
                column: "region".to_owned(),
                new_name: "market".to_owned(),
            },
            DatasetRecipeStep::Upper {
                column: "region".to_owned(),
            },
        ]),
        &columns(&["region"]),
    )
    .expect_err("the old name is gone at that point in the recipe");
    assert_eq!(error.step_index, Some(1));
}

#[test]
fn a_split_columns_output_is_addressable_by_a_later_step() {
    validate(
        &recipe(vec![
            DatasetRecipeStep::Split {
                column: "name".to_owned(),
                delimiter: " ".to_owned(),
                parts: 2,
            },
            DatasetRecipeStep::Upper {
                column: "name_1".to_owned(),
            },
        ]),
        &columns(&["name"]),
    )
    .expect("split names are known to later steps");
}

#[test]
fn a_recipe_past_the_step_cap_is_refused_as_too_long() {
    let steps = (0..=MAX_RECIPE_STEPS)
        .map(|_| DatasetRecipeStep::Dedup)
        .collect();
    let error = validate(&recipe(steps), &columns(&["a"])).expect_err("over the cap");
    assert_eq!(error.code, DatasetErrorCode::RecipeTooLong);
    assert_eq!(error.step_index, None);
}

#[test]
fn a_filter_without_a_value_is_refused_and_a_null_test_is_not() {
    let error = validate(
        &recipe(vec![DatasetRecipeStep::Filter {
            column: "a".to_owned(),
            op: DatasetFilterOp::Equals,
            value: None,
        }]),
        &columns(&["a"]),
    )
    .expect_err("nothing to compare against");
    assert_eq!(error.step_index, Some(0));

    validate(
        &recipe(vec![DatasetRecipeStep::Filter {
            column: "a".to_owned(),
            op: DatasetFilterOp::IsNull,
            value: None,
        }]),
        &columns(&["a"]),
    )
    .expect("a null test reads no value");
}

#[test]
fn an_empty_or_colliding_rename_is_refused_rather_than_ignored() {
    for new_name in ["", "   ", "revenue"] {
        let error = validate(
            &recipe(vec![DatasetRecipeStep::Rename {
                column: "region".to_owned(),
                new_name: new_name.to_owned(),
            }]),
            &columns(&["region", "revenue"]),
        )
        .expect_err("web no-ops this; a no-op is a silent divergence");
        assert_eq!(error.step_index, Some(0), "{new_name:?}");
    }
}

#[test]
fn an_empty_find_or_delimiter_is_refused() {
    let error = validate(
        &recipe(vec![DatasetRecipeStep::Replace {
            column: "a".to_owned(),
            find: String::new(),
            replace_with: "x".to_owned(),
        }]),
        &columns(&["a"]),
    )
    .expect_err("replacing nothing is not a transform");
    assert_eq!(error.step_index, Some(0));

    let error = validate(
        &recipe(vec![DatasetRecipeStep::Split {
            column: "a".to_owned(),
            delimiter: String::new(),
            parts: 2,
        }]),
        &columns(&["a"]),
    )
    .expect_err("splitting on nothing is not a transform");
    assert_eq!(error.step_index, Some(0));
}

#[test]
fn a_filter_compares_numbers_as_numbers() {
    let shaped = apply(
        &columns(&["n"]),
        &rows(&[&["9"], &["10"], &["100"]]),
        &recipe(vec![DatasetRecipeStep::Filter {
            column: "n".to_owned(),
            op: DatasetFilterOp::GreaterThan,
            value: Some("9".to_owned()),
        }]),
    )
    .expect("filter applies");
    assert_eq!(
        shaped_cells(&shaped),
        vec![vec!["10"], vec!["100"]],
        "lexically, \"10\" sorts below \"9\""
    );
}

#[test]
fn a_filter_does_not_coerce_text_that_merely_contains_separators() {
    // "1,2,3" is not 123, "50%" is not 0.5, and "12abc" is not 12. Only a
    // clean literal — here currency plus well-formed grouping — is a number.
    let shaped = apply(
        &columns(&["n"]),
        &rows(&[&["1,2,3"], &["50%"], &["12abc"], &["$1,000"]]),
        &recipe(vec![DatasetRecipeStep::Filter {
            column: "n".to_owned(),
            op: DatasetFilterOp::AtLeast,
            value: Some("500".to_owned()),
        }]),
    )
    .expect("filter applies");
    assert_eq!(
        shaped_cells(&shaped),
        vec![vec!["$1,000"]],
        "the three text values compare lexically below \"500\" and drop out"
    );

    // The sharper form of the same claim: a coercing parser would keep this
    // row, because it would read "1,2,3" as 123.
    let coerced = apply(
        &columns(&["n"]),
        &rows(&[&["1,2,3"]]),
        &recipe(vec![DatasetRecipeStep::Filter {
            column: "n".to_owned(),
            op: DatasetFilterOp::Equals,
            value: Some("123".to_owned()),
        }]),
    )
    .expect("filter applies");
    assert!(coerced.rows.is_empty());
}

#[test]
fn a_filter_compares_iso_dates_chronologically() {
    let shaped = apply(
        &columns(&["d"]),
        &rows(&[&["2026-01-05"], &["2025-12-31"]]),
        &recipe(vec![DatasetRecipeStep::Filter {
            column: "d".to_owned(),
            op: DatasetFilterOp::GreaterThan,
            value: Some("2026-01-01".to_owned()),
        }]),
    )
    .expect("filter applies");
    assert_eq!(shaped_cells(&shaped), vec![vec!["2026-01-05"]]);
}

#[test]
fn contains_is_case_insensitive() {
    let shaped = apply(
        &columns(&["s"]),
        &rows(&[&["EMEA"], &["apac"]]),
        &recipe(vec![DatasetRecipeStep::Filter {
            column: "s".to_owned(),
            op: DatasetFilterOp::Contains,
            value: Some("Em".to_owned()),
        }]),
    )
    .expect("filter applies");
    assert_eq!(shaped_cells(&shaped), vec![vec!["EMEA"]]);
}

#[test]
fn a_sort_puts_empty_cells_last_in_both_directions() {
    for descending in [false, true] {
        let shaped = apply(
            &columns(&["n"]),
            &rows(&[&["2"], &[""], &["1"]]),
            &recipe(vec![DatasetRecipeStep::Sort {
                column: "n".to_owned(),
                descending,
            }]),
        )
        .expect("sort applies");
        assert_eq!(
            shaped_cells(&shaped).last(),
            Some(&vec![""]),
            "a column of gaps must not fill the top of a descending sort"
        );
    }
}

#[test]
fn a_sort_orders_numbers_numerically() {
    let shaped = apply(
        &columns(&["n"]),
        &rows(&[&["10"], &["9"], &["100"]]),
        &recipe(vec![DatasetRecipeStep::Sort {
            column: "n".to_owned(),
            descending: false,
        }]),
    )
    .expect("sort applies");
    assert_eq!(
        shaped_cells(&shaped),
        vec![vec!["9"], vec!["10"], vec!["100"]]
    );
}

#[test]
fn a_cast_to_number_reads_currency_and_grouping_and_percent() {
    let shaped = apply(
        &columns(&["n"]),
        &rows(&[&["$1,000"], &["50%"], &["1,000.5"], &["not a number"]]),
        &recipe(vec![DatasetRecipeStep::Cast {
            column: "n".to_owned(),
            cast_to: DatasetCastType::Number,
        }]),
    )
    .expect("cast applies");
    assert_eq!(
        shaped_cells(&shaped),
        vec![
            vec!["1000"],
            vec!["0.5"],
            vec!["1000.5"],
            // A value the cast cannot read is left alone rather than emptied.
            vec!["not a number"],
        ]
    );
}

#[test]
fn a_cast_to_integer_truncates() {
    let shaped = apply(
        &columns(&["n"]),
        &rows(&[&["3.7"], &["-3.7"]]),
        &recipe(vec![DatasetRecipeStep::Cast {
            column: "n".to_owned(),
            cast_to: DatasetCastType::Integer,
        }]),
    )
    .expect("cast applies");
    assert_eq!(shaped_cells(&shaped), vec![vec!["3"], vec!["-3"]]);
}

#[test]
fn a_cast_to_boolean_reads_the_common_spellings() {
    let shaped = apply(
        &columns(&["b"]),
        &rows(&[&["Yes"], &["N"], &["1"], &["0"], &["maybe"]]),
        &recipe(vec![DatasetRecipeStep::Cast {
            column: "b".to_owned(),
            cast_to: DatasetCastType::Boolean,
        }]),
    )
    .expect("cast applies");
    assert_eq!(
        shaped_cells(&shaped),
        vec![
            vec!["true"],
            vec!["false"],
            vec!["true"],
            vec!["false"],
            vec!["maybe"]
        ]
    );
}

#[test]
fn a_cast_to_date_never_shifts_a_day() {
    let shaped = apply(
        &columns(&["d"]),
        &rows(&[
            &["2026-03-15"],
            &["3/15/2026"],
            &["2026/03/15"],
            &["2026-02-31"],
        ]),
        &recipe(vec![DatasetRecipeStep::Cast {
            column: "d".to_owned(),
            cast_to: DatasetCastType::Date,
        }]),
    )
    .expect("cast applies");
    assert_eq!(
        shaped_cells(&shaped),
        vec![
            vec!["2026-03-15"],
            vec!["2026-03-15"],
            vec!["2026-03-15"],
            // A rolled-over date stays text rather than silently becoming
            // March 2nd.
            vec!["2026-02-31"],
        ]
    );
}

#[test]
fn a_cast_leaves_an_empty_cell_empty() {
    let shaped = apply(
        &columns(&["n"]),
        &rows(&[&[""]]),
        &recipe(vec![DatasetRecipeStep::Cast {
            column: "n".to_owned(),
            cast_to: DatasetCastType::Number,
        }]),
    )
    .expect("cast applies");
    assert_eq!(shaped.rows[0][0], None);
}

#[test]
fn trim_upper_lower_and_replace_map_one_column() {
    let shaped = apply(
        &columns(&["a", "b"]),
        &rows(&[&["  x  ", "keep"]]),
        &recipe(vec![
            DatasetRecipeStep::Trim {
                column: "a".to_owned(),
            },
            DatasetRecipeStep::Upper {
                column: "a".to_owned(),
            },
            DatasetRecipeStep::Replace {
                column: "a".to_owned(),
                find: "X".to_owned(),
                replace_with: "y".to_owned(),
            },
            DatasetRecipeStep::Lower {
                column: "a".to_owned(),
            },
        ]),
    )
    .expect("maps apply");
    assert_eq!(shaped_cells(&shaped), vec![vec!["y", "keep"]]);
}

#[test]
fn a_split_inserts_its_columns_after_the_source() {
    let shaped = apply(
        &columns(&["name", "tail"]),
        &rows(&[&["ada lovelace", "z"], &["", "y"]]),
        &recipe(vec![DatasetRecipeStep::Split {
            column: "name".to_owned(),
            delimiter: " ".to_owned(),
            parts: 2,
        }]),
    )
    .expect("split applies");
    assert_eq!(
        shaped.columns,
        columns(&["name", "name_1", "name_2", "tail"])
    );
    assert_eq!(
        shaped_cells(&shaped),
        vec![
            vec!["ada lovelace", "ada", "lovelace", "z"],
            vec!["", "", "", "y"],
        ]
    );
}

#[test]
fn a_split_name_that_collides_probes_upward() {
    let shaped = apply(
        &columns(&["name", "NAME_1"]),
        &rows(&[&["a b", "x"]]),
        &recipe(vec![DatasetRecipeStep::Split {
            column: "name".to_owned(),
            delimiter: " ".to_owned(),
            parts: 1,
        }]),
    )
    .expect("split applies");
    assert_eq!(
        shaped.columns,
        columns(&["name", "name_1_2", "NAME_1"]),
        "collision is case-insensitive, matching DA's header rule"
    );
}

#[test]
fn dedup_ignores_a_trailing_run_of_empty_cells() {
    // A row the export cut short and a row with an empty last cell are the same
    // row: `apply` squares both to the header width, and a missing cell and an
    // empty one are one value in the CSV this writes.
    let shaped = apply(
        &columns(&["a", "b"]),
        &rows(&[&["x", ""], &["x"], &["y", "z"]]),
        &recipe(vec![DatasetRecipeStep::Dedup]),
    )
    .expect("dedup applies");
    assert_eq!(shaped.rows.len(), 2);
}

#[test]
fn dedup_keeps_rows_that_differ_only_in_cell_boundaries() {
    let shaped = apply(
        &columns(&["a", "b"]),
        &rows(&[&["ab", "c"], &["a", "bc"]]),
        &recipe(vec![DatasetRecipeStep::Dedup]),
    )
    .expect("dedup applies");
    assert_eq!(
        shaped.rows.len(),
        2,
        "the key needs a separator no cell holds"
    );
}

#[test]
fn remove_nulls_treats_whitespace_as_empty() {
    let shaped = apply(
        &columns(&["a"]),
        &rows(&[&["x"], &["   "], &[""]]),
        &recipe(vec![DatasetRecipeStep::RemoveNulls {
            column: "a".to_owned(),
        }]),
    )
    .expect("filter applies");
    assert_eq!(shaped_cells(&shaped), vec![vec!["x"]]);
}

#[test]
fn fill_nulls_fills_absent_and_blank_cells() {
    let shaped = apply(
        &columns(&["a"]),
        &rows(&[&["x"], &["  "], &[""]]),
        &recipe(vec![DatasetRecipeStep::FillNulls {
            column: "a".to_owned(),
            value: "0".to_owned(),
        }]),
    )
    .expect("fill applies");
    assert_eq!(shaped_cells(&shaped), vec![vec!["x"], vec!["0"], vec!["0"]]);
}

#[test]
fn steps_apply_in_order() {
    // Filter-then-sort and sort-then-filter give the same rows here; rename
    // after upper and upper after rename do not, which is the case that
    // proves order is honoured rather than assumed.
    let shaped = apply(
        &columns(&["a"]),
        &rows(&[&["x"]]),
        &recipe(vec![
            DatasetRecipeStep::Rename {
                column: "a".to_owned(),
                new_name: "b".to_owned(),
            },
            DatasetRecipeStep::Upper {
                column: "b".to_owned(),
            },
        ]),
    )
    .expect("steps apply");
    assert_eq!(shaped.columns, columns(&["b"]));
    assert_eq!(shaped_cells(&shaped), vec![vec!["X"]]);
}

#[test]
fn a_split_keeps_the_source_column_and_names_its_parts_after_it() {
    let shaped = apply(
        &columns(&["name", "tail"]),
        &rows(&[&["Ada Lovelace", "x"]]),
        &recipe(vec![
            DatasetRecipeStep::Split {
                column: "name".to_owned(),
                delimiter: " ".to_owned(),
                parts: 2,
            },
            DatasetRecipeStep::Rename {
                column: "tail".to_owned(),
                new_name: "rest".to_owned(),
            },
        ]),
    )
    .expect("steps apply");
    assert_eq!(
        shaped.columns,
        columns(&["name", "name_1", "name_2", "rest"])
    );
    assert_eq!(
        shaped_cells(&shaped),
        vec![vec!["Ada Lovelace", "Ada", "Lovelace", "x"]]
    );
}

#[test]
fn a_materialized_recipe_writes_the_rows_the_preview_showed() {
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = directory.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging dir");
    let contents =
        "Report title,,\nregion,revenue,note\nEMEA,10,keep\n,,\nAPAC,20,keep\nAPAC,20,keep\n";
    let source = fixtures::write_file(directory.path(), "sales.csv", contents.as_bytes());

    let written = materialize(
        &source,
        DatasetFormat::Csv,
        contents.len() as u64,
        0,
        1,
        &recipe(vec![
            DatasetRecipeStep::Dedup,
            DatasetRecipeStep::Cast {
                column: "revenue".to_owned(),
                cast_to: DatasetCastType::Integer,
            },
        ]),
        &staging,
    )
    .expect("recipe materializes");

    assert_eq!(
        written.file_name().and_then(|name| name.to_str()),
        Some("sales_cleaned.csv"),
        "a recipe's output is always a CSV, whatever went in"
    );
    let out = std::fs::read_to_string(&written).expect("written file");
    assert_eq!(
        out, "region,revenue,note\r\nEMEA,10,keep\r\nAPAC,20,keep\r\n",
        "the blank row is dropped, the duplicate is dropped, the header is the chosen row"
    );
}

#[test]
fn materialized_recipe_output_is_leased_until_the_upload_owner_releases_it() {
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = directory.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging dir");
    let contents = "region,revenue\nEMEA,10\n";
    let source = fixtures::write_file(directory.path(), "sales.csv", contents.as_bytes());
    let cleanup = TempCleanup::new([staging.clone()]);
    let staged = StagedSource {
        path: source.clone(),
        size: contents.len() as u64,
        format: DatasetFormat::Csv,
        file_name: "sales.csv".to_owned(),
    };

    let (written, lease) =
        materialize_with_lease(&staged, 0, 0, &recipe(Vec::new()), &staging, &cleanup)
            .expect("recipe output materializes with a lease");

    assert_eq!(
        cleanup.sweep_pass(),
        0,
        "in-progress output stays protected"
    );
    assert!(written.exists());
    drop(lease);
    assert_eq!(cleanup.sweep_pass(), 1, "released output is abandoned");
    assert!(!written.exists());
    assert!(source.exists(), "the picked original is never swept");
}

#[test]
fn a_materialized_workbook_recipe_writes_a_csv() {
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = directory.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging dir");
    let source = fixtures::write_workbook(
        directory.path(),
        "book.xlsx",
        &[
            fixtures::Sheet {
                name: "Ignore",
                rows: &[&["x"], &["1"]],
            },
            fixtures::Sheet {
                name: "Data",
                rows: &[&["region", "revenue"], &["EMEA", "10"], &["APAC", "20"]],
            },
        ],
    );
    let size = std::fs::metadata(&source).expect("metadata").len();

    let written = materialize(
        &source,
        DatasetFormat::Xlsx,
        size,
        1,
        0,
        &recipe(vec![DatasetRecipeStep::Upper {
            column: "region".to_owned(),
        }]),
        &staging,
    )
    .expect("recipe materializes");
    let out = std::fs::read_to_string(&written).expect("written file");
    assert_eq!(out, "region,revenue\r\nEMEA,10\r\nAPAC,20\r\n");
}

#[test]
fn a_worksheet_that_starts_past_column_a_previews_as_the_file_that_uploads() {
    // `calamine`'s `worksheet_range` reports a sheet's *used* range, so the
    // full-file read has no cell for a leading empty column. A preview that
    // indexed by absolute column would show two extra columns named `col_1` and
    // `col_2`, offer them in the cleanup editor, and refuse the step at upload.
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = directory.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging dir");
    let source = fixtures::write_workbook(
        directory.path(),
        "offset.xlsx",
        &[fixtures::Sheet {
            name: "Data",
            rows: &[
                &["", "", "region", "revenue"],
                &["", "", "EMEA", "10"],
                &["", "", "APAC", "20"],
            ],
        }],
    );
    let size = std::fs::metadata(&source).expect("metadata").len();

    let sample = crate::datasets::preview::read_sample(&source, DatasetFormat::Xlsx, size, 0)
        .expect("sample");
    let header = crate::datasets::preview::header_at(&sample, 0);
    let previewed: Vec<&str> = header
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect();
    assert_eq!(previewed, ["region", "revenue"]);

    let written = materialize(
        &source,
        DatasetFormat::Xlsx,
        size,
        0,
        0,
        &recipe(vec![DatasetRecipeStep::Upper {
            column: "region".to_owned(),
        }]),
        &staging,
    )
    .expect("recipe materializes");
    let out = std::fs::read_to_string(&written).expect("written file");
    assert_eq!(out, "region,revenue\r\nEMEA,10\r\nAPAC,20\r\n");
}

#[test]
fn a_materialized_row_is_padded_to_the_header_width() {
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = directory.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging dir");
    let contents = "a,b,c\n1\n2,3,4,5\n";
    let source = fixtures::write_file(directory.path(), "ragged.csv", contents.as_bytes());
    let written = materialize(
        &source,
        DatasetFormat::Csv,
        contents.len() as u64,
        0,
        0,
        &recipe(Vec::new()),
        &staging,
    )
    .expect("recipe materializes");
    let out = std::fs::read_to_string(&written).expect("written file");
    assert_eq!(
        out, "a,b,c\r\n1,,\r\n2,3,4\r\n",
        "a stray trailing cell must not make the column count vary by row"
    );
}

#[test]
fn a_file_past_the_recipe_ceiling_is_refused_with_its_own_code() {
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = directory.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging dir");
    let source = fixtures::write_file(directory.path(), "big.csv", b"a\n1\n");
    let error = materialize(
        &source,
        DatasetFormat::Csv,
        RECIPE_FULL_READ_MAX_BYTES + 1,
        0,
        0,
        &recipe(vec![DatasetRecipeStep::Dedup]),
        &staging,
    )
    .expect_err("over the ceiling");
    assert_eq!(error.code, DatasetErrorCode::RecipeFileTooLarge);
}

#[test]
fn a_materialized_recipe_refuses_an_invalid_step_rather_than_writing_a_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = directory.path().join("staging");
    std::fs::create_dir_all(&staging).expect("staging dir");
    let contents = "a,b\n1,2\n";
    let source = fixtures::write_file(directory.path(), "sales.csv", contents.as_bytes());
    let error = materialize(
        &source,
        DatasetFormat::Csv,
        contents.len() as u64,
        0,
        0,
        &recipe(vec![DatasetRecipeStep::Trim {
            column: "missing".to_owned(),
        }]),
        &staging,
    )
    .expect_err("the step cannot apply");
    assert_eq!(error.code, DatasetErrorCode::RecipeInvalidStep);
    assert_eq!(
        std::fs::read_dir(&staging)
            .expect("staging readable")
            .count(),
        0,
        "a refused recipe leaves nothing behind"
    );
}

#[test]
fn a_semicolon_export_materializes_the_columns_the_preview_showed() {
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = directory.path().join("staging");
    // A semicolon-separated export read as comma-separated is one column wide,
    // so a step naming `region` would be refused for a file whose preview
    // showed it.
    let contents = "region;revenue\nEMEA;10\nEMEA;10\n";
    let source = fixtures::write_file(directory.path(), "sales.csv", contents.as_bytes());

    let written = materialize(
        &source,
        DatasetFormat::Csv,
        contents.len() as u64,
        0,
        0,
        &recipe(vec![
            DatasetRecipeStep::Dedup,
            DatasetRecipeStep::Upper {
                column: "region".to_owned(),
            },
        ]),
        &staging,
    )
    .expect("the file's own delimiter is read");

    let out = std::fs::read_to_string(&written).expect("written file");
    assert_eq!(
        out, "region,revenue\r\nEMEA,10\r\n",
        "the columns are the file's, and the output is always comma-separated"
    );
}

#[test]
fn two_recipes_over_one_stem_do_not_write_one_path() {
    let directory = tempfile::tempdir().expect("temp dir");
    let staging = directory.path().join("staging");
    let first = fixtures::write_file(directory.path(), "sales.csv", b"region,revenue\nEMEA,10\n");
    let second_dir = directory.path().join("archive");
    std::fs::create_dir_all(&second_dir).expect("second dir");
    let second = fixtures::write_file(&second_dir, "sales.csv", b"region,revenue\nAPAC,20\n");
    let steps = recipe(vec![DatasetRecipeStep::Upper {
        column: "region".to_owned(),
    }]);

    let one = materialize(&first, DatasetFormat::Csv, 24, 0, 0, &steps, &staging)
        .expect("first materializes");
    let two = materialize(&second, DatasetFormat::Csv, 24, 0, 0, &steps, &staging)
        .expect("second materializes");

    // One path for both would mean the second write truncating a file the
    // first transfer is still reading.
    assert_ne!(one, two);
    assert_eq!(one.file_name(), two.file_name(), "the name stays readable");
    assert!(std::fs::read_to_string(&one)
        .expect("first still readable")
        .contains("EMEA"));
    assert!(std::fs::read_to_string(&two)
        .expect("second readable")
        .contains("APAC"));
}

#[test]
fn a_row_shorter_than_the_header_is_squared_before_a_step_runs() {
    // The full-file path pads every row to the header's width, so the preview
    // has to as well: a step that filled only the rows that happened to carry
    // a trailing cell would upload data the preview never showed.
    let shaped = apply(
        &columns(&["region", "revenue"]),
        &[vec![Some("EMEA".to_owned())]],
        &recipe(vec![DatasetRecipeStep::FillNulls {
            column: "revenue".to_owned(),
            value: "0".to_owned(),
        }]),
    )
    .expect("the step applies");
    assert_eq!(shaped_cells(&shaped), vec![vec!["EMEA", "0"]]);
}

#[test]
fn a_row_wider_than_the_header_is_cut_to_it() {
    let shaped = apply(
        &columns(&["region"]),
        &[rows(&[&["EMEA", "stray"]])[0].clone()],
        &recipe(vec![DatasetRecipeStep::Dedup]),
    )
    .expect("the step applies");
    assert_eq!(shaped_cells(&shaped), vec![vec!["EMEA"]]);
}
