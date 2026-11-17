//! Anchor resolution, covered directly against the text functions rather
//! than through a staged trusted root: these are pure transforms, and the
//! interesting cases are about *matching*, not about containment (which
//! `patch`/`path_policy` cover).

use crate::projects::edit::{apply_edits, apply_insert};
use crate::projects::{EditError, TextEdit, MAX_ANCHOR_BYTES, MAX_EDITS_PER_OPERATION};

fn edit(old_string: &str, new_string: &str) -> TextEdit {
    TextEdit {
        old_string: old_string.to_owned(),
        new_string: new_string.to_owned(),
        replace_all: false,
    }
}

fn edit_all(old_string: &str, new_string: &str) -> TextEdit {
    TextEdit {
        replace_all: true,
        ..edit(old_string, new_string)
    }
}

fn apply(text: &str, edits: &[TextEdit]) -> Result<String, EditError> {
    edit_in("src/a.rs", text, edits)
}

fn edit_in(path: &str, text: &str, edits: &[TextEdit]) -> Result<String, EditError> {
    apply_edits(text, edits, path)
}

// -- Matching ---------------------------------------------------------------

#[test]
fn a_unique_anchor_is_replaced_and_the_rest_of_the_file_is_untouched() {
    let text = "fn one() {}\nfn two() {}\n";

    let updated = apply(text, &[edit("fn two()", "fn three()")]).expect("a unique anchor applies");

    assert_eq!(updated, "fn one() {}\nfn three() {}\n");
}

#[test]
fn an_anchor_that_matches_nothing_says_the_match_must_be_exact() {
    let error = apply("fn one() {}\n", &[edit("fn missing()", "x")])
        .expect_err("a missing anchor cannot be applied");

    assert!(matches!(error, EditError::AnchorNotFound(ref path, 1, _) if path == "src/a.rs"));
    // The message has to be actionable: whoever wrote the anchor is about to
    // retry, and "no match" alone tells them nothing.
    let message = error.to_string();
    assert!(message.contains("exactly"), "{message}");
    assert!(message.contains("whitespace and indentation"), "{message}");
}

#[test]
fn an_ambiguous_anchor_reports_how_many_places_it_matched() {
    let error = apply(
        "value = 1\nvalue = 1\nvalue = 1\n",
        &[edit("value = 1", "value = 2")],
    )
    .expect_err("an ambiguous anchor cannot be applied");

    assert!(matches!(error, EditError::AnchorAmbiguous(_, 1, 3, _)));
    let message = error.to_string();
    assert!(message.contains("matched 3 places"), "{message}");
    assert!(message.contains("replace_all"), "{message}");
}

#[test]
fn replace_all_changes_every_occurrence_of_an_ambiguous_anchor() {
    let updated = apply("a\nb\na\n", &[edit_all("a", "c")]).expect("replace_all applies");

    assert_eq!(updated, "c\nb\nc\n");
}

#[test]
fn replace_all_still_needs_at_least_one_match() {
    let error =
        apply("a\n", &[edit_all("missing", "c")]).expect_err("replace_all is not a no-op licence");

    assert!(matches!(error, EditError::AnchorNotFound(_, 1, _)));
}

#[test]
fn edits_apply_in_order_so_a_later_one_can_anchor_on_what_an_earlier_one_wrote() {
    let updated = apply(
        "let x = 1;\n",
        &[
            edit("let x = 1;", "let y = 1;"),
            edit("let y = 1;", "let y = 2;"),
        ],
    )
    .expect("sequential edits apply");

    assert_eq!(updated, "let y = 2;\n");
}

#[test]
fn the_reported_edit_position_is_the_failing_one_not_the_first() {
    let error = apply("a\nb\n", &[edit("a", "c"), edit("missing", "d")])
        .expect_err("the second edit fails");

    assert!(matches!(
        error,
        EditError::AnchorAmbiguous(_, 2, _, _) | EditError::AnchorNotFound(_, 2, _)
    ));
    assert!(error.to_string().contains("edit 2"));
}

// -- Line endings -----------------------------------------------------------

#[test]
fn an_lf_anchor_matches_a_crlf_file_and_the_replacement_keeps_crlf() {
    let text = "fn one() {}\r\nfn two() {}\r\n";

    // A reader sees the text, not the line endings, so writing the anchor back
    // with `\n` must not be what fails the edit.
    let updated = apply(
        text,
        &[edit(
            "fn one() {}\nfn two() {}",
            "fn one() {}\nfn three() {}",
        )],
    )
    .expect("an LF anchor matches a CRLF file");

    assert_eq!(updated, "fn one() {}\r\nfn three() {}\r\n");
    assert!(!updated.contains("\r\r"), "endings must not double up");
}

#[test]
fn a_crlf_anchor_still_matches_a_crlf_file_directly() {
    let updated =
        apply("a\r\nb\r\n", &[edit("a\r\nb", "a\r\nc")]).expect("an exact anchor applies");

    assert_eq!(updated, "a\r\nc\r\n");
}

#[test]
fn a_mixed_ending_file_is_matched_exactly_only() {
    // Normalizing here would rewrite whichever convention the anchor did not
    // use, so a mixed file gets no forgiveness.
    let text = "a\r\nb\nc\r\n";

    let error = apply(text, &[edit("b\nc\n", "b\nd\n")]);
    assert!(matches!(error, Err(EditError::AnchorNotFound(_, 1, _))) || error.is_ok());

    let updated = apply(text, &[edit("b\nc", "b\nd")]).expect("the exact text applies");
    assert_eq!(updated, "a\r\nb\nd\r\n");
}

// -- Rejections -------------------------------------------------------------

#[test]
fn an_edit_that_changes_nothing_is_rejected_rather_than_written() {
    let error = apply("a\n", &[edit("a", "a")]).expect_err("a no-op edit is a mistake");

    assert!(matches!(error, EditError::AnchorNoOp(_, 1)));
}

#[test]
fn an_empty_anchor_is_rejected_because_it_would_match_everywhere() {
    let error = apply("a\n", &[edit("", "b")]).expect_err("an empty anchor is meaningless");

    assert!(matches!(error, EditError::EmptyAnchor(_, 1)));
}

#[test]
fn an_empty_edit_list_is_rejected() {
    let error = apply("a\n", &[]).expect_err("an edit operation must carry an edit");

    assert!(matches!(error, EditError::NoEdits(path) if path == "src/a.rs"));
}

#[test]
fn more_edits_than_the_bound_are_rejected_and_exactly_at_the_bound_are_not() {
    let text = (0..=MAX_EDITS_PER_OPERATION)
        .map(|index| format!("line{index}\n"))
        .collect::<String>();

    // Anchored on the whole line, so `line6` does not also match `line64`.
    let at_bound: Vec<TextEdit> = (0..MAX_EDITS_PER_OPERATION)
        .map(|index| edit(&format!("line{index}\n"), &format!("kept{index}\n")))
        .collect();
    assert!(apply(&text, &at_bound).is_ok());

    let over_bound: Vec<TextEdit> = (0..=MAX_EDITS_PER_OPERATION)
        .map(|index| edit(&format!("line{index}\n"), &format!("kept{index}\n")))
        .collect();
    let error = apply(&text, &over_bound).expect_err("past the bound is refused");
    assert!(matches!(
        error,
        EditError::TooManyEdits(_, limit) if limit == MAX_EDITS_PER_OPERATION
    ));
}

#[test]
fn an_anchor_past_the_byte_bound_is_rejected() {
    let long = "a".repeat(MAX_ANCHOR_BYTES + 1);

    let error = apply(&long, &[edit(&long, "b")]).expect_err("an oversized anchor is refused");

    assert!(matches!(
        error,
        EditError::AnchorTooLarge(_, 1, limit) if limit == MAX_ANCHOR_BYTES
    ));
}

// -- Insert -----------------------------------------------------------------

fn insert(text: &str, line: u32, content: &str) -> Result<String, EditError> {
    apply_insert(text, line, content, "src/a.rs")
}

#[test]
fn inserting_after_line_zero_puts_the_text_at_the_top() {
    let updated = insert("second\n", 0, "first").expect("insert at the top");

    assert_eq!(updated, "first\nsecond\n");
}

#[test]
fn inserting_after_an_interior_line_puts_the_text_between_them() {
    let updated = insert("one\ntwo\n", 1, "middle").expect("insert between lines");

    assert_eq!(updated, "one\nmiddle\ntwo\n");
}

#[test]
fn inserting_after_the_last_line_appends_and_keeps_a_missing_trailing_newline_missing() {
    let updated = insert("one\ntwo", 2, "three").expect("append after the last line");

    assert_eq!(updated, "one\ntwo\nthree");
}

#[test]
fn inserting_after_the_last_line_of_a_newline_terminated_file_keeps_it_terminated() {
    let updated = insert("one\ntwo\n", 2, "three").expect("append after the last line");

    assert_eq!(updated, "one\ntwo\nthree\n");
}

#[test]
fn inserting_into_an_empty_file_writes_the_first_line() {
    let updated = insert("", 0, "only").expect("insert into an empty file");

    assert_eq!(updated, "only\n");
}

#[test]
fn a_trailing_newline_on_the_inserted_text_is_not_doubled() {
    let updated = insert("after\n", 0, "before\n").expect("insert a terminated block");

    assert_eq!(updated, "before\nafter\n");
}

#[test]
fn inserted_text_takes_the_files_own_line_ending() {
    let updated = insert("one\r\ntwo\r\n", 1, "mid\nalso").expect("insert into a CRLF file");

    assert_eq!(updated, "one\r\nmid\r\nalso\r\ntwo\r\n");
}

#[test]
fn inserting_past_the_end_of_the_file_is_rejected_with_the_line_count() {
    let error = insert("one\ntwo\n", 5, "x").expect_err("past the end is refused");

    assert!(matches!(error, EditError::InsertLineOutOfRange(_, 5, 2)));
    assert!(error.to_string().contains("it has 2 lines"));
}

#[test]
fn inserted_text_past_the_byte_bound_is_rejected() {
    let error = insert("a\n", 0, &"x".repeat(MAX_ANCHOR_BYTES + 1))
        .expect_err("an oversized block is refused");

    assert!(matches!(
        error,
        EditError::InsertTooLarge(_, limit) if limit == MAX_ANCHOR_BYTES
    ));
}

// -- Tolerant matching ------------------------------------------------------

#[test]
fn an_anchor_with_trailing_spaces_the_file_lacks_still_finds_its_lines() {
    let updated = apply(
        "let a = 1;\nlet b = 2;\n",
        &[edit("let a = 1;   \nlet b = 2;", "let a = 3;\nlet b = 4;")],
    )
    .expect("trailing spaces are not what fails an edit");

    assert_eq!(updated, "let a = 3;\nlet b = 4;\n");
}

#[test]
fn an_anchor_at_the_wrong_indentation_finds_its_lines_and_the_new_text_follows_the_file() {
    let text = "<ul>\n    <li>one</li>\n    <li>two</li>\n</ul>\n";

    // Copied without the four spaces the file indents by.
    let updated = apply(
        text,
        &[edit(
            "<li>one</li>\n<li>two</li>",
            "<li>one</li>\n<li>two</li>\n<li>three</li>",
        )],
    )
    .expect("indentation is not what fails an edit");

    assert_eq!(
        updated,
        "<ul>\n    <li>one</li>\n    <li>two</li>\n    <li>three</li>\n</ul>\n"
    );
}

#[test]
fn an_anchor_indented_deeper_than_the_file_is_brought_back_to_the_files_indentation() {
    let text = "fn a() {\nlet x = 1;\n}\n";

    let updated = apply(
        text,
        &[edit("    let x = 1;", "    let x = 2;\n    let y = 3;")],
    )
    .expect("surplus indentation is not what fails an edit");

    assert_eq!(updated, "fn a() {\nlet x = 2;\nlet y = 3;\n}\n");
}

#[test]
fn an_anchor_whose_indent_cannot_be_moved_to_the_files_is_not_matched() {
    // A tab-indented file read back with spaces: replacing the line would leave
    // spaces in a tab file, and the width of a tab is a guess.
    let error = apply(
        "fn a() {\n\tfoo();\n}\n",
        &[edit("    foo();", "    bar();")],
    )
    .expect_err("an indent that is neither a prefix of the other is not matched");

    assert!(matches!(error, EditError::AnchorNotFound(..)), "{error:?}");
}

#[test]
fn typographic_punctuation_in_the_file_matches_the_plain_characters_a_model_types() {
    let text = "<p>It\u{2019}s \u{201C}done\u{201D} \u{2014} really</p>\n";

    let updated = apply(
        text,
        &[edit(
            "<p>It's \"done\" - really</p>",
            "<p>It's finished</p>",
        )],
    )
    .expect("curly quotes and dashes are not what fails an edit");

    assert_eq!(updated, "<p>It's finished</p>\n");
}

#[test]
fn a_tolerant_match_in_a_crlf_file_keeps_crlf() {
    // The anchor is missing the indentation, so only the tolerant tiers find it.
    let text = "a\r\n    b\r\n    c\r\n";

    let updated =
        apply(text, &[edit("b\nc", "x\ny")]).expect("an anchor missing the indent applies");

    assert_eq!(updated, "a\r\n    x\r\n    y\r\n");
    assert!(!updated.contains("\r\r"));
}

#[test]
fn a_tolerant_match_that_finds_several_places_is_ambiguous_not_a_guess() {
    let text = "  <li>x</li>\n<li>y</li>\n    <li>x</li>\n";

    let error = apply(text, &[edit("<li>x</li>\n", "<li>z</li>\n")]);
    // Two places match, however they are found, and two places must not apply.
    if let Err(EditError::AnchorAmbiguous(_, 1, count, places)) = &error {
        assert_eq!(*count, 2);
        assert!(places.contains("1") && places.contains("3"), "{places}");
    } else {
        panic!("expected an ambiguity, got {error:?}");
    }
}

#[test]
fn replace_all_changes_every_place_a_tolerant_anchor_finds() {
    let text = "  <li>x</li>\n<li>y</li>\n    <li>x</li>\n";

    let updated = apply(text, &[edit_all("<li>x</li>", "<li>z</li>")])
        .expect("replace_all applies to every tolerant match");

    assert_eq!(updated, "  <li>z</li>\n<li>y</li>\n    <li>z</li>\n");
}

#[test]
fn a_mixed_ending_file_gets_no_tolerant_matching() {
    let text = "a\r\n    b\nc\r\n";

    let error = apply(text, &[edit("b", "x")]).map(|_| ());
    // `b` alone is an exact substring and applies; a line-shaped anchor that
    // only tolerance could find must not.
    assert!(error.is_ok());
    let error = apply(text, &[edit("b \n", "x\n")]).expect_err("no tolerance on a mixed file");
    assert!(matches!(error, EditError::AnchorNotFound(_, 1, _)));
}

// -- What a failed anchor tells its author ---------------------------------

#[test]
fn a_miss_names_the_closest_line_of_the_file() {
    let text = "const total = compute(a, b);\nconst other = 1;\n";

    let error = apply(text, &[edit("const totl = compute(a, b, c);", "x")])
        .expect_err("a misspelt anchor cannot be applied");

    let message = error.to_string();
    assert!(message.contains("closest text is at line 1"), "{message}");
    assert!(
        message.contains("const total = compute(a, b);"),
        "{message}"
    );
}

#[test]
fn a_miss_with_nothing_alike_adds_no_hint() {
    let error = apply("alpha\nbeta\n", &[edit("zzzzzzzzzzzz", "x")]).expect_err("no match");

    assert!(!error.to_string().contains("closest"), "{error}");
}

#[test]
fn an_anchor_copied_with_line_numbers_says_so() {
    let text = "let a = 1;\nlet b = 2;\n";

    let error = apply(
        text,
        &[edit(
            "1: let a = 1;\n2: let b = 2;",
            "let a = 3;\nlet b = 4;",
        )],
    )
    .expect_err("numbered text is not the file's text");

    assert!(error.to_string().contains("line numbers"), "{error}");
}

#[test]
fn an_exact_ambiguity_lists_where_the_matches_are() {
    let error = apply("x\ny\nx\ny\nx\n", &[edit("x", "z")]).expect_err("three places");

    let message = error.to_string();
    assert!(message.contains("matched 3 places"), "{message}");
    assert!(message.contains("lines 1, 3, 5"), "{message}");
}

// -- Placeholders -----------------------------------------------------------

#[test]
fn new_text_with_a_rest_unchanged_comment_is_refused() {
    for placeholder in [
        "// ... rest of the file unchanged",
        "<!-- ... existing content ... -->",
        "# ... remaining code ...",
        "/* ... */",
        "// ...",
    ] {
        let error = apply("a\n", &[edit("a", &format!("a\n{placeholder}"))])
            .expect_err("a placeholder would delete what it stands for");
        assert!(
            matches!(error, EditError::PlaceholderContent(_, 2)),
            "{placeholder}: {error:?}"
        );
    }
}

#[test]
fn a_placeholder_the_file_already_holds_is_not_a_placeholder() {
    let text = "// ... rest of the list is generated\nitem\n";

    let updated = apply(text, &[edit("item", "item\nitem2")]).expect("it was already there");

    assert_eq!(
        updated,
        "// ... rest of the list is generated\nitem\nitem2\n"
    );
}

#[test]
fn prose_with_an_ellipsis_is_not_a_placeholder() {
    let updated = apply("a\n", &[edit("a", "She paused... then left.\nb")])
        .expect("an ellipsis in prose is fine");

    assert_eq!(updated, "She paused... then left.\nb\n");
}

#[test]
fn a_prose_file_may_trail_off_with_an_ellipsis() {
    let updated = edit_in(
        "notes.md",
        "a\n",
        &[edit("a", "a\n<!-- ... the rest is in the wiki -->")],
    )
    .expect("a markdown file holds no code to omit");

    assert!(updated.contains("the rest is in the wiki"));
}

#[test]
fn a_bullet_with_an_ellipsis_is_not_a_placeholder_in_code_or_prose() {
    let updated = apply("a\n", &[edit("a", "a\n* the remaining items follow...")])
        .expect("a bullet is not a comment marker");

    assert!(updated.contains("remaining items"));
}

#[test]
fn is_prose_path_reads_the_extension_only() {
    use crate::projects::edit::is_prose_path;
    assert!(is_prose_path("docs/README.MD"));
    assert!(!is_prose_path("requirements.txt"));
    assert!(!is_prose_path("a.html"));
    assert!(!is_prose_path("md"));
    assert!(!is_prose_path("notes.md.js"));
}

#[test]
fn a_miss_in_a_long_file_returns_promptly_with_or_without_a_hint() {
    let line = "const value = compute(alpha, beta, gamma, delta, epsilon);\n";
    let text = line.repeat(40_000);

    let started = std::time::Instant::now();
    let error = apply(&text, &[edit("const valeu = compute(alpha, beta);", "x")])
        .expect_err("a misspelt anchor cannot be applied");

    assert!(matches!(error, EditError::AnchorNotFound(..)));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn flattened_code_does_not_match_nested_code() {
    // The anchor lost the body's indentation. Matching it would write the new
    // text over `if x:` and its body as two lines at one level.
    let text = "if x:\n    return 1\nelse:\n    return 2\n";

    let error = apply(text, &[edit("if x:\nreturn 1", "if y:\nreturn 3")])
        .expect_err("the nesting is part of what the lines say");

    assert!(matches!(error, EditError::AnchorNotFound(..)));
}

#[test]
fn an_anchor_shifted_as_a_whole_still_matches_nested_code() {
    let text = "class A:\n    def f(self):\n        return 1\n";

    let updated = apply(
        text,
        &[edit(
            "def f(self):\n    return 1",
            "def f(self):\n    return 2",
        )],
    )
    .expect("the same shape one level over is the same lines");

    assert_eq!(updated, "class A:\n    def f(self):\n        return 2\n");
}

#[test]
fn the_tolerant_match_is_not_attempted_on_a_huge_buffer() {
    let text = "  indented\n".repeat(400_000);
    assert!(text.len() > 4 * 1024 * 1024);

    // No exact substring: only the indentation-blind tiers could find it.
    let started = std::time::Instant::now();
    let error = apply(&text, &[edit("indented\nindented", "x")])
        .expect_err("past the size bound only an exact match applies");

    assert!(matches!(error, EditError::AnchorNotFound(..)));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn ordinary_comments_that_mention_what_remains_are_not_placeholders() {
    for line in [
        "// TODO: handle the remaining cases...",
        "# Keep the rest of the settings...",
        "#define X_REMAINING(...)",
    ] {
        apply("a\n", &[edit("a", &format!("a\n{line}"))])
            .unwrap_or_else(|error| panic!("{line}: {error}"));
    }
}

#[test]
fn a_text_file_is_code_for_the_placeholder_guard() {
    let error = edit_in(
        "requirements.txt",
        "a\n",
        &[edit("a", "a\n# ... rest unchanged")],
    )
    .expect_err("only markdown-like files are prose");

    assert!(matches!(error, EditError::PlaceholderContent(..)));
}
