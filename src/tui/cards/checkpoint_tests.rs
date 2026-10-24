use super::*;

fn draw(
    name: &str,
    arguments: &Value,
    result: Option<&Value>,
    columns: usize,
    presentation: &Presentation,
) -> Vec<Line> {
    render(
        Tool {
            name,
            arguments,
            summary: "complete",
            result,
            change: None,
            presentation,
        },
        columns,
    )
}

fn plain(rows: &[Line]) -> Vec<String> {
    rows.iter().map(Line::plain).collect()
}

fn saved() -> Value {
    Value::object([
        ("bytes", Value::number(6)),
        (
            "checkpoint",
            Value::object([
                ("status", Value::string("saved")),
                ("ref", Value::string("rc-3")),
            ]),
        ),
    ])
}

#[test]
fn a_running_command_shows_its_checkpoint_in_the_running_line() {
    let presentation = Presentation {
        note: Some("checkpoint rc-12 · saved".into()),
        ..Presentation::default()
    };
    let rows = plain(&draw(
        "bash",
        &Value::object([("command", Value::string("cargo fmt"))]),
        None,
        80,
        &presentation,
    ));
    assert_eq!(
        rows,
        ["● Bash(cargo fmt)", "  ⎿ ▰▰▱▱▱▱ · checkpoint rc-12 · saved"]
    );
}

#[test]
fn saved_states_stay_quiet_until_expanded() {
    let arguments = Value::object([("path", Value::string("notes.txt"))]);
    let collapsed = plain(&draw(
        "write",
        &arguments,
        Some(&saved()),
        80,
        &Presentation::default(),
    ));
    assert!(
        collapsed.iter().all(|row| !row.contains("rc-")),
        "{collapsed:?}"
    );
    let expanded = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    let rows = plain(&draw("write", &arguments, Some(&saved()), 80, &expanded));
    assert!(
        rows.iter()
            .any(|row| row.trim_start().ends_with("Checkpoint rc-3")),
        "{rows:?}"
    );
}

#[test]
fn an_uncaptured_change_always_says_so_without_claiming_a_save() {
    let result = Value::object([
        ("bytes", Value::number(6)),
        (
            "checkpoint",
            Value::object([
                ("status", Value::string("none")),
                ("reason", Value::string("disk full")),
            ]),
        ),
    ]);
    let rows = plain(&draw(
        "write",
        &Value::object([("path", Value::string("notes.txt"))]),
        Some(&result),
        80,
        &Presentation::default(),
    ));
    let warning: Vec<&String> = rows
        .iter()
        .filter(|row| row.contains("Checkpoint none"))
        .collect();
    assert_eq!(warning.len(), 1, "{rows:?}");
    assert!(warning[0].contains("disk full"));
    assert!(rows.iter().all(|row| !row.contains("rc-")));
}

#[test]
fn checkpoint_cards_fit_narrow_and_wide_terminals() {
    let arguments = Value::object([
        ("action", Value::string("restore")),
        ("ref", Value::string("rc-7")),
        (
            "paths",
            Value::Array(vec![Value::string(
                "src/very/long/path/to/a/module/file.rs",
            )]),
        ),
    ]);
    let result = Value::object([
        ("ref", Value::string("rc-7")),
        ("preview", Value::Bool(false)),
        (
            "restored",
            Value::Array(vec![Value::object([
                (
                    "path",
                    Value::string("src/very/long/path/to/a/module/file.rs"),
                ),
                ("action", Value::string("replace")),
            ])]),
        ),
        (
            "undo",
            Value::object([
                ("action", Value::string("restore")),
                ("ref", Value::string("rc-9")),
            ]),
        ),
    ]);
    for columns in [32, 100] {
        let rows = draw(
            "checkpoint",
            &arguments,
            Some(&result),
            columns,
            &Presentation::default(),
        );
        let text = plain(&rows);
        assert!(text[0].starts_with("● Checkpoint(restore"), "{text:?}");
        assert!(text.iter().any(|row| row.starts_with("  ⎿ ")), "{text:?}");
        for row in &text {
            assert!(row.chars().count() <= columns, "{columns}: {row:?}");
        }
    }
}
