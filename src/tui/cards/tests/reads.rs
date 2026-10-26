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
    assert!(plain(&compact).contains("Read 2 lines · from line 41"));
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
    assert!(plain(&rows).contains("Read 5 bytes · from byte 16"));
    assert_eq!(rows.last().unwrap().plain(), "    hello");
}
