use super::*;

fn draw(
    name: &str,
    arguments: &Value,
    result: Option<&Value>,
    change: Option<&Change>,
    columns: usize,
    presentation: &Presentation,
) -> Vec<Line> {
    render(
        Tool {
            name,
            arguments,
            summary: "complete",
            result,
            change,
            presentation,
        },
        columns,
    )
}

fn plain(rows: &[Line]) -> String {
    rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n")
}

#[test]
fn canonical_batch_without_full_file_diff_shows_staged_replacements() {
    let arguments = Value::object([
        ("path", Value::string("large.rs")),
        (
            "edits",
            Value::Array(vec![
                Value::object([
                    ("old_text", Value::string("alpha")),
                    ("new_text", Value::string("gamma")),
                ]),
                Value::object([
                    ("old_text", Value::string("gamma beta")),
                    ("new_text", Value::string("delta")),
                ]),
            ]),
        ),
    ]);
    let result = Value::object([("bytes_written", Value::number(1_048_577))]);
    let rows = draw(
        "edit",
        &arguments,
        Some(&result),
        None,
        80,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    );
    let text = plain(&rows);
    assert!(
        text.contains("2 replacements") && text.contains("full-file diff unavailable"),
        "{text}"
    );
    assert!(
        text.contains("Replacement 1") && text.contains("Replacement 2"),
        "{text}"
    );
    assert!(
        text.contains("alpha") && text.contains("gamma beta") && text.contains("delta"),
        "{text}"
    );
    assert!(!text.contains("No changes"), "{text}");
    assert!(!text.contains("1 +") && !text.contains("1 −"), "{text}");
}

#[test]
fn successful_large_file_batch_reaches_the_fallback_card() {
    let fixture = crate::test_support::Directory::new();
    let path = fixture.path().join("large.txt");
    let original = format!(
        "alpha beta\n{}",
        "x".repeat(crate::file_change::MAX_DIFF_BYTES)
    );
    std::fs::write(&path, original).unwrap();
    let arguments = Value::object([
        ("path", Value::string("large.txt")),
        (
            "edits",
            Value::Array(vec![
                Value::object([
                    ("old_text", Value::string("alpha")),
                    ("new_text", Value::string("gamma")),
                ]),
                Value::object([
                    ("old_text", Value::string("gamma beta")),
                    ("new_text", Value::string("delta")),
                ]),
            ]),
        ),
    ]);
    let tools = crate::tools::Tools::new(fixture.path()).unwrap();
    let encoded = arguments.encode();
    let cancellation = crate::cancel::Cancellation::default();
    let execution = tools.execute_begun(
        tools.begin_cancellable("edit", &encoded, &cancellation),
        "edit",
        &encoded,
        &cancellation,
    );
    assert_eq!(execution.result.get("error"), None);
    assert!(execution.change.is_none());
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("delta\n")
    );
    let text = plain(&draw(
        "edit",
        &arguments,
        Some(&execution.result),
        execution.change.as_ref(),
        80,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    ));
    assert!(
        text.contains("Applied 2 replacements") && text.contains("Replacement 2"),
        "{text}"
    );
    assert!(!text.contains("No changes"), "{text}");

    let single = Value::object([
        ("path", Value::string("large.txt")),
        (
            "edits",
            Value::Array(vec![Value::object([
                (
                    "old_text",
                    Value::string(std::fs::read_to_string(&path).unwrap()),
                ),
                ("new_text", Value::string("short")),
            ])]),
        ),
    ]);
    let encoded = single.encode();
    let execution = tools.execute_begun(
        tools.begin_cancellable("edit", &encoded, &cancellation),
        "edit",
        &encoded,
        &cancellation,
    );
    assert_eq!(execution.result.get("error"), None);
    assert!(execution.change.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "short");
    let text = plain(&draw(
        "edit",
        &single,
        Some(&execution.result),
        execution.change.as_ref(),
        80,
        &Presentation::default(),
    ));
    assert!(
        text.contains("Applied 1 replacement · full-file diff unavailable"),
        "{text}"
    );
    assert!(
        text.contains("Snippet diff unavailable within display limits."),
        "{text}"
    );
}

#[test]
fn canonical_single_edit_uses_snippet_diff_and_noop_batch_stays_empty() {
    let single = Value::object([
        ("path", Value::string("large.rs")),
        (
            "edits",
            Value::Array(vec![Value::object([
                ("old_text", Value::string("old()")),
                ("new_text", Value::string("new()")),
            ])]),
        ),
    ]);
    let result = Value::object([("bytes_written", Value::number(1_048_577))]);
    let text = plain(&draw(
        "edit",
        &single,
        Some(&result),
        None,
        80,
        &Presentation::default(),
    ));
    assert!(
        text.contains("Added 1 line, removed 1 line") && text.contains("new()"),
        "{text}"
    );
    assert!(
        text.contains("Original file coordinates unavailable."),
        "{text}"
    );

    let batch = Value::object([
        ("path", Value::string("large.rs")),
        (
            "edits",
            Value::Array(vec![
                Value::object([
                    ("old_text", Value::string("a")),
                    ("new_text", Value::string("b")),
                ]),
                Value::object([
                    ("old_text", Value::string("b")),
                    ("new_text", Value::string("a")),
                ]),
            ]),
        ),
    ]);
    let noop = Value::object([("bytes_written", Value::number(0))]);
    let text = plain(&draw(
        "edit",
        &batch,
        Some(&noop),
        None,
        80,
        &Presentation::default(),
    ));
    assert!(text.contains("No changes"), "{text}");
    assert!(
        !text.contains("Replacement 1") && !text.contains("full-file diff unavailable"),
        "{text}"
    );
}

#[test]
fn collapsed_batch_limits_replacement_previews_and_expansion_shows_all() {
    let arguments = Value::object([
        ("path", Value::string("large.txt")),
        (
            "edits",
            Value::Array(
                [("first", "one"), ("second", "two"), ("third", "three")]
                    .map(|(old, new)| {
                        Value::object([
                            ("old_text", Value::string(old)),
                            ("new_text", Value::string(new)),
                        ])
                    })
                    .into(),
            ),
        ),
    ]);
    let result = Value::object([("bytes_written", Value::number(1_048_577))]);
    let collapsed = plain(&draw(
        "edit",
        &arguments,
        Some(&result),
        None,
        80,
        &Presentation::default(),
    ));
    assert!(collapsed.contains("1 more replacement"), "{collapsed}");
    assert!(
        !collapsed.contains("Replacement 3") && !collapsed.contains("three"),
        "{collapsed}"
    );
    let expanded = plain(&draw(
        "edit",
        &arguments,
        Some(&result),
        None,
        80,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    ));
    assert!(
        expanded.contains("Replacement 3") && expanded.contains("three"),
        "{expanded}"
    );
}

#[test]
fn cancelled_batch_does_not_show_unsaved_replacement_snippets() {
    let arguments = Value::object([
        ("path", Value::string("large.txt")),
        (
            "edits",
            Value::Array(vec![
                Value::object([
                    ("old_text", Value::string("first")),
                    ("new_text", Value::string("one")),
                ]),
                Value::object([
                    ("old_text", Value::string("second")),
                    ("new_text", Value::string("two")),
                ]),
            ]),
        ),
    ]);
    let cancelled = Value::object([("cancelled", Value::Bool(true))]);
    let text = plain(&draw(
        "edit",
        &arguments,
        Some(&cancelled),
        None,
        80,
        &Presentation::default(),
    ));
    assert!(text.contains("Cancelled"), "{text}");
    assert!(
        !text.contains("Replacement") && !text.contains("one"),
        "{text}"
    );

    let legacy = Value::object([
        ("path", Value::string("large.txt")),
        ("old_text", Value::string("before")),
        ("new_text", Value::string("after")),
    ]);
    let text = plain(&draw(
        "edit",
        &legacy,
        Some(&cancelled),
        None,
        80,
        &Presentation::default(),
    ));
    assert!(
        text.contains("Cancelled") && !text.contains("after"),
        "{text}"
    );
}

#[test]
fn oversized_single_replacements_report_the_unavailable_snippet_diff() {
    let canonical = Value::object([
        ("path", Value::string("large.txt")),
        (
            "edits",
            Value::Array(vec![Value::object([
                (
                    "old_text",
                    Value::string("x".repeat(crate::file_change::MAX_DIFF_BYTES + 1)),
                ),
                ("new_text", Value::string("short")),
            ])]),
        ),
    ]);
    let result = Value::object([("bytes_written", Value::number(5))]);
    let text = plain(&draw(
        "edit",
        &canonical,
        Some(&result),
        None,
        80,
        &Presentation::default(),
    ));
    assert!(
        text.contains("Applied 1 replacement · full-file diff unavailable"),
        "{text}"
    );
    assert!(
        text.contains("Snippet diff unavailable within display limits."),
        "{text}"
    );
    assert!(
        !text.contains("No changes") && !text.contains("xxxxx"),
        "{text}"
    );

    let legacy = Value::object([
        ("path", Value::string("large.txt")),
        ("old_text", Value::string("line\n".repeat(20_001))),
        ("new_text", Value::string("short")),
    ]);
    let text = plain(&draw(
        "edit",
        &legacy,
        Some(&result),
        None,
        80,
        &Presentation::default(),
    ));
    assert!(
        text.contains("Applied 1 replacement · full-file diff unavailable"),
        "{text}"
    );
    assert!(
        text.contains("Snippet diff unavailable within display limits."),
        "{text}"
    );
    assert!(
        !text.contains("No changes") && !text.contains("line\nline"),
        "{text}"
    );
}
