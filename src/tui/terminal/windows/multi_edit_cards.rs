use super::{Terminal, probe};
use std::io::{self, Write};

/// A multi-file edit with a failed file (B19) through the native console and
/// the full frame renderer, collapsed and after Ctrl+O.
pub(super) fn console(terminal: &Terminal) {
    use crate::{events::Event, file_change::Change, json::Value, tui::state::State};

    let (physical_width, height) = terminal.size;
    let files = [
        (
            "src/limit.rs",
            Some(("let limit = 10;\n", "let limit = LIMIT;\n")),
        ),
        ("src/window.rs", None),
        (
            "tests/limit.rs",
            Some(("check();\n", "check();\nblock();\n")),
        ),
    ];
    let entry = |path: &str| {
        Value::object([
            ("path", Value::string(path)),
            (
                "edits",
                Value::Array(vec![Value::object([
                    ("old_text", Value::string("old")),
                    ("new_text", Value::string("new")),
                ])]),
            ),
        ])
    };
    let arguments = Value::object([(
        "files",
        Value::Array(files.iter().map(|(path, _)| entry(path)).collect()),
    )]);
    let mut changes = Vec::new();
    let entries = files
        .iter()
        .map(|(path, texts)| match texts {
            Some((before, after)) => {
                changes.push((
                    path.to_string(),
                    Change::between(Some(before), after).unwrap(),
                ));
                Value::object([
                    ("path", Value::string(*path)),
                    ("bytes_written", Value::number(after.len())),
                ])
            }
            None => Value::object([
                ("path", Value::string(*path)),
                (
                    "error",
                    Value::string("edit 1: old text was not found; read the file"),
                ),
            ]),
        })
        .collect();
    let result = Value::object([("files", Value::Array(entries))]);
    let change = Change::several(changes).unwrap();
    for columns in [44, 80] {
        for expanded in [false, true] {
            let mut state = State {
                width: columns,
                height,
                ..State::default()
            };
            state.begin_tool_turn();
            state.event(Event::ToolStarted {
                id: "edits".into(),
                name: "edit".into(),
                arguments: arguments.clone(),
            });
            state.event(Event::ToolChanged {
                id: "edits".into(),
                change: change.clone(),
            });
            state.event(Event::ToolFinished {
                id: "edits".into(),
                name: "edit".into(),
                summary: crate::events::tool_summary("edit", &result),
                result: result.clone(),
            });
            if expanded {
                state.finish_tool_turn();
                assert_eq!(state.toggle_tools(), Some((true, 1)));
            }
            let rows: Vec<_> = state.rows().into_iter().flatten().collect();
            let plain: Vec<_> = rows.iter().map(crate::tui::line::Line::plain).collect();
            for expected in [
                " UPDATE  src/limit.rs, src/window.rs,",
                "2 files · +2 −1 lines · 1 failed",
                "  src/limit.rs  +1 −1",
                "1 + let limit = LIMIT;",
                "  src/window.rs · not changed · edit 1:",
                "  tests/limit.rs  +1",
                "2 + block();",
            ] {
                assert!(
                    plain.iter().any(|row| row.contains(expected)),
                    "{expected} at {columns} columns: {plain:#?}"
                );
            }
            assert!(
                plain
                    .iter()
                    .all(|row| crate::tui::text::cells(row) <= columns)
            );
            assert!(plain.len() < height);
            let frame = crate::tui::view::Frame {
                header: vec![],
                history: vec![],
                live: rows,
                cursor: None,
                composer: height - 1,
            };
            let output = crate::tui::render::Renderer::new().paint(&frame, columns, height);
            write!(io::stdout(), "{output}").unwrap();
            io::stdout().flush().unwrap();
            let expected: String = (0..height)
                .map(|index| {
                    let text = plain.get(index).map_or("", String::as_str);
                    format!(
                        "{text}{}",
                        " ".repeat(physical_width.saturating_sub(crate::tui::text::cells(text)))
                    )
                })
                .collect();
            assert_eq!(
                probe().lines().nth(1).unwrap(),
                expected,
                "native multi-file edit {} at {columns} columns",
                if expanded { "expanded" } else { "collapsed" }
            );
        }
    }
}
