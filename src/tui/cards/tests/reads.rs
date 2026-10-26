use super::*;

#[test]
fn chapter_expansion_keeps_distinct_windows_read_targets() {
    let arguments = [
        args("path", r"C:\shared\mod.rs"),
        args("path", r"D:\shared\mod.rs"),
    ];
    let result = Value::object([
        ("content", Value::string("1: item\n")),
        ("offset", Value::number(1)),
        ("lines_returned", Value::number(1)),
    ]);
    let presentation = Presentation::default();
    let reads: Vec<_> = arguments
        .iter()
        .map(|arguments| Tool {
            name: "read",
            arguments,
            summary: "complete",
            result: Some(&result),
            change: None,
            presentation: &presentation,
        })
        .collect();
    let chapter = chapter_header(&reads, 100, false);
    assert!(plain(&chapter.rows).contains("2 files read"));
    let expanded = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    let titles: Vec<_> = arguments
        .iter()
        .map(|arguments| {
            card(
                Tool {
                    name: "read",
                    arguments,
                    summary: "complete",
                    result: Some(&result),
                    change: None,
                    presentation: &expanded,
                },
                100,
                false,
            )
            .rows[0]
                .plain()
        })
        .collect();
    assert_eq!(
        titles,
        [r"● Read(C:\shared\mod.rs)", r"● Read(D:\shared\mod.rs)"]
    );
}

#[test]
fn read_expansion_preserves_numbered_source_and_syntax() {
    let result = Value::object([
        (
            "content",
            Value::string("41: pub fn login() {\n42:     let limit = 10;\n"),
        ),
        ("offset", Value::number(41)),
        ("lines_returned", Value::number(2)),
    ]);
    let arguments = args("path", "src/auth.rs");
    let compact = draw(
        "read",
        &arguments,
        Some(&result),
        None,
        80,
        &Presentation::default(),
    );
    assert_eq!(compact.len(), 2);
    assert!(plain(&compact).contains("2 lines · from line 41"));
    let rows = draw(
        "read",
        &arguments,
        Some(&result),
        None,
        40,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    );
    assert!(plain(&rows).contains("41   pub fn login()"));
    assert!(
        rows.iter()
            .flat_map(|row| &row.spans)
            .any(|span| span.style == KEYWORD && span.text == "pub")
    );
    assert!(!plain(&rows).contains("41:"));
    assert!(rows.iter().all(|row| text::cells(&row.plain()) <= 40));
}

#[test]
fn byte_reads_do_not_invent_line_numbers() {
    let arguments = Value::object([
        ("path", Value::string("output:s1:c1:stdout")),
        ("byte_offset", Value::number(16)),
    ]);
    let result = Value::object([
        ("offset", Value::Null),
        ("byte_offset", Value::number(16)),
        ("bytes_returned", Value::number(5)),
        ("content", Value::string("hello")),
    ]);
    let rows = draw(
        "read",
        &arguments,
        Some(&result),
        None,
        80,
        &Presentation {
            expanded: true,
            ..Presentation::default()
        },
    );
    assert!(plain(&rows).contains("5 bytes · from byte 16"));
    assert_eq!(rows.last().unwrap().plain(), "    hello");
}

#[test]
fn compact_read_pagination_stays_on_one_quiet_row() {
    let arguments = args("path", "src/auth.rs");
    for (total, next, wide) in [
        (
            Value::number(212),
            Value::Null,
            "9 lines · from line 33 of 212",
        ),
        (
            Value::Null,
            Value::number(42),
            "9 lines · from line 33 · more from 42",
        ),
    ] {
        let result = Value::object([
            ("content", Value::string("33: first\n")),
            ("offset", Value::number(33)),
            ("lines_returned", Value::number(9)),
            ("total_lines", total),
            ("next_offset", next),
        ]);
        for columns in [40, 80] {
            let rows = draw(
                "read",
                &arguments,
                Some(&result),
                None,
                columns,
                &Presentation::default(),
            );
            assert_eq!(rows.len(), 2, "{}", plain(&rows));
            assert!(rows[1].plain().starts_with("  ⎿ 9 lines"));
            assert!(rows[1].spans.iter().skip(1).all(|span| span.style == MUTED));
            assert!(!plain(&rows).contains("More lines available"));
            assert!(text::cells(&rows[1].plain()) <= columns);
            if columns == 80 {
                assert_eq!(rows[1].plain(), format!("  ⎿ {wide}"));
            } else if wide.contains("more from") {
                assert_eq!(rows[1].plain(), "  ⎿ 9 lines · more from 42");
            } else {
                assert_eq!(rows[1].plain(), format!("  ⎿ {wide}"));
            }
        }
    }
}

#[test]
fn read_byte_and_partial_line_continuations_do_not_invent_totals() {
    let byte_result = Value::object([
        ("offset", Value::Null),
        ("byte_offset", Value::number(16)),
        ("bytes_returned", Value::number(5)),
        ("next_byte_offset", Value::number(21)),
        ("content", Value::string("hello")),
    ]);
    for columns in [40, 80] {
        let rows = draw(
            "read",
            &args("path", "output:s1:c1:stdout"),
            Some(&byte_result),
            None,
            columns,
            &Presentation::default(),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1].plain(),
            if columns == 80 {
                "  ⎿ 5 bytes · from byte 16 · more from 21"
            } else {
                "  ⎿ 5 bytes · more from 21"
            }
        );
        assert!(!plain(&rows).contains("of "));
    }
    let partial = Value::object([
        ("offset", Value::number(33)),
        ("lines_returned", Value::number(1)),
        ("next_offset", Value::number(33)),
        ("next_byte_offset", Value::number(123)),
        ("partial_line", Value::Bool(true)),
        ("content", Value::string("33: fragment")),
    ]);
    for columns in [40, 80] {
        let rows = draw(
            "read",
            &args("path", "notes.txt"),
            Some(&partial),
            None,
            columns,
            &Presentation::default(),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1].plain(),
            if columns == 80 {
                "  ⎿ 1 line · from line 33 · continues at byte 123"
            } else {
                "  ⎿ 1 line · continues at byte 123"
            }
        );
    }
}

#[test]
fn expanded_media_read_shows_metadata_without_attachment_json() {
    for (view, metadata, expected) in [
        (
            "image",
            Value::object([
                ("id", Value::string("att-image")),
                ("width", Value::number(640)),
                ("height", Value::number(480)),
                ("size", Value::number(12_288)),
            ]),
            "Opened image · 640×480 · 12 KiB",
        ),
        (
            "pdf",
            Value::object([
                ("id", Value::string("att-pdf")),
                ("pages", Value::number(3)),
                ("size", Value::number(2_048)),
            ]),
            "Opened PDF · 3 pages · 2 KiB",
        ),
    ] {
        let result = Value::object([
            ("view", Value::string(view)),
            ("attachment", metadata),
            ("local_path", Value::string("C:/private/attachment")),
            (
                "note",
                Value::string("The attachment follows in the next user message."),
            ),
        ]);
        for columns in [40, 80] {
            let rows = draw(
                "read",
                &args("path", "attachment:fixture"),
                Some(&result),
                None,
                columns,
                &Presentation {
                    expanded: true,
                    ..Presentation::default()
                },
            );
            let shown = plain(&rows);
            assert!(shown.contains(expected), "{shown}");
            assert!(shown.contains("The attachment follows"), "{shown}");
            assert!(!shown.contains("att-image") && !shown.contains("att-pdf"));
            assert!(!shown.contains("local_path") && !shown.contains("C:/private"));
            assert!(rows.iter().all(|row| text::cells(&row.plain()) <= columns));
        }
    }
}
