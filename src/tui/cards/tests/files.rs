use super::*;

#[test]
fn new_file_previews_six_numbered_lines_and_expands_all_source() {
    let source = (1..=9)
        .map(|line| format!("const value_{line} = {line};\n"))
        .collect::<String>();
    let change = Change::between(None, &source).unwrap();
    let arguments = args("path", "new.rs");
    let compact = draw(
        "write",
        &arguments,
        Some(&ok()),
        Some(&change),
        80,
        &Presentation::default(),
    );
    let text = plain(&compact);
    assert!(text.starts_with(" WRITE  new.rs  New file · 9 lines\n"));
    assert!(text.contains("  │ 1   const value_1 = 1;"));
    assert!(text.contains("  │ 6   const value_6 = 6;"));
    assert!(!text.contains("value_7"));
    assert!(text.ends_with("  └ +3 lines"));
    assert!(compact.iter().all(|row| row.background.is_none()));

    let expanded = draw(
        "write",
        &arguments,
        Some(&ok()),
        Some(&change),
        80,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    );
    let text = plain(&expanded);
    assert!(text.ends_with("  └ 9   const value_9 = 9;"));
    assert!(!text.contains("+3 lines"));
}

#[test]
fn compact_existing_file_diff_names_hidden_hunks_and_rows() {
    let old = (0..60).map(|n| format!("line {n}\n")).collect::<String>();
    let new = old
        .replace("line 5\n", "changed 5\n")
        .replace("line 30\n", "changed 30\n")
        .replace("line 55\n", "changed 55\n");
    let change = Change::between(Some(&old), &new).unwrap();
    let rows = draw(
        "edit",
        &args("path", "notes.txt"),
        Some(&ok()),
        Some(&change),
        80,
        &Presentation::default(),
    );
    let hidden = rows
        .iter()
        .find(|row| row.plain().contains("+2 hunks · 16 rows"))
        .unwrap_or_else(|| panic!("{}", plain(&rows)));
    assert!(hidden.spans.iter().all(|span| span.style == DIM));
    assert!(
        rows.iter()
            .any(|row| row.background == Some(DIFF_ADDED_SURFACE))
    );
}

#[test]
fn file_changes_survive_collapse_and_errors_do_not_show_success_diffs() {
    let change = Change::between(
        Some("fn login() {\n    old();\n}\n"),
        "fn login() {\n    new();\n    save();\n}\n",
    )
    .unwrap();
    let arguments = args("path", "src/auth.rs");
    let initial = draw(
        "edit",
        &arguments,
        Some(&ok()),
        Some(&change),
        80,
        &Presentation::default(),
    );
    // The fact joins the title; the diff starts at column 2, not hung.
    assert!(plain(&initial).starts_with(" UPDATE  src/auth.rs  +2 −1 lines\n    1   fn login"));
    assert!(
        initial
            .iter()
            .any(|row| row.background == Some(DIFF_ADDED_SURFACE))
    );
    // Too narrow to join, the fact hangs on its result row above the diff.
    let narrow = plain(&draw(
        "edit",
        &arguments,
        Some(&ok()),
        Some(&change),
        28,
        &Presentation::default(),
    ));
    assert!(
        narrow.starts_with(" UPDATE  src/auth.rs\n  └ +2 −1 lines\n    1"),
        "{narrow}"
    );
    assert!(!narrow.contains('⎿'));
    let collapsed = draw(
        "edit",
        &arguments,
        Some(&ok()),
        Some(&change),
        80,
        &Presentation {
            expanded: false,
            ..Presentation::default()
        },
    );
    assert_eq!(initial, collapsed);
    let failed = Value::object([(
        "error",
        Value::string("File is read-only; its content was not changed"),
    )]);
    let rows = draw(
        "edit",
        &arguments,
        Some(&failed),
        Some(&change),
        80,
        &Presentation::default(),
    );
    assert!(!plain(&rows).contains("new();") && !plain(&rows).contains("+2"));
    assert!(plain(&rows).contains("src/auth.rs  File is read-only"));
}

#[test]
fn old_sessions_keep_snippet_diffs_without_fake_file_coordinates_or_eof() {
    let arguments = Value::object([
        ("path", Value::string("legacy.rs")),
        ("old_text", Value::string("same\nold")),
        ("new_text", Value::string("same\nnew")),
    ]);
    let rows = draw(
        "edit",
        &arguments,
        Some(&ok()),
        None,
        80,
        &Presentation::default(),
    );
    let text = plain(&rows);
    assert!(
        text.contains("+1 −1 lines") && text.contains("Original file coordinates unavailable.")
    );
    assert!(
        !text.contains("No newline at end of file")
            && !text.contains("    1 −")
            && !text.contains("    1 +")
    );
    assert_eq!(text.matches("same").count(), 1);
}
