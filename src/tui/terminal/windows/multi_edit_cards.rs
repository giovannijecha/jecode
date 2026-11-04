use super::{Terminal, probe};
use crate::{
    events::Event,
    file_change::Change,
    json::Value,
    tui::{line::Line, state::State},
};
use std::io::{self, Write};

/// One finished call, as the turn's only card.
struct Call<'a> {
    name: &'a str,
    arguments: &'a Value,
    change: Option<&'a Change>,
    result: &'a Value,
}

impl Call<'_> {
    /// Its rows in `columns`, opened with Ctrl+O when `expanded`.
    fn rows(&self, columns: usize, height: usize, expanded: bool) -> Vec<Line> {
        let mut state = State {
            width: columns,
            height,
            ..State::default()
        };
        state.begin_tool_turn();
        state.event(Event::ToolStarted {
            id: "call".into(),
            name: self.name.into(),
            arguments: self.arguments.clone(),
        });
        if let Some(change) = self.change {
            state.event(Event::ToolChanged {
                id: "call".into(),
                change: change.clone(),
            });
        }
        state.event(Event::ToolFinished {
            id: "call".into(),
            name: self.name.into(),
            summary: crate::events::tool_summary(self.name, self.result),
            result: self.result.clone(),
        });
        if expanded {
            state.finish_tool_turn();
            assert_eq!(state.toggle_tools(), Some((true, 1)));
        }
        state.rows().into_iter().flatten().collect()
    }
}

/// Paints `rows` through the full frame renderer and reads the native console
/// back, after checking each `expected` text reached a row.
fn shown(terminal: &Terminal, rows: Vec<Line>, columns: usize, expected: &[&str]) {
    let (physical_width, height) = terminal.size;
    let plain: Vec<_> = rows.iter().map(Line::plain).collect();
    for expected in expected {
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
    let painted: String = (0..height)
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
        painted,
        "native frame at {columns} columns: {plain:#?}"
    );
}

/// A multi-file edit with a failed file (B19), the same call refused at Read
/// (B12), and a multi-path read naming a terminal escape (B17), through the
/// native console and the full frame renderer, collapsed and after Ctrl+O.
pub(super) fn console(terminal: &Terminal) {
    let (_, height) = terminal.size;
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
    let refused = Value::object([
        ("outcome", Value::string("unavailable")),
        (
            "reason",
            Value::string(
                "Not available at the Read permission level: editing src/limit.rs, src/window.rs, tests/limit.rs.",
            ),
        ),
    ]);
    let hostile = "src/\x1b[2Jlimit.rs";
    let read = Value::object([(
        "paths",
        Value::Array(vec![Value::string(hostile), Value::string("src/gone.rs")]),
    )]);
    let pages = Value::object([(
        "files",
        Value::Array(vec![
            Value::object([
                ("path", Value::string(hostile)),
                ("content", Value::string("1: let limit = 10;\n")),
                ("offset", Value::number(1)),
            ]),
            Value::object([
                ("path", Value::string("src/gone.rs")),
                (
                    "error",
                    Value::string("Could not resolve path\nsecond line"),
                ),
            ]),
        ]),
    )]);
    let edit = Call {
        name: "edit",
        arguments: &arguments,
        change: Some(&change),
        result: &result,
    };
    let gated = Call {
        name: "edit",
        arguments: &arguments,
        change: None,
        result: &refused,
    };
    let reads = Call {
        name: "read",
        arguments: &read,
        change: None,
        result: &pages,
    };
    for columns in [44, 80] {
        for expanded in [false, true] {
            shown(
                terminal,
                edit.rows(columns, height, expanded),
                columns,
                &[
                    " UPDATE  src/limit.rs, src/window.rs,",
                    "2 files · +2 −1 lines · 1 failed",
                    "  src/limit.rs  +1 −1",
                    "1 + let limit = LIMIT;",
                    "  src/window.rs · not changed · edit 1:",
                    "  tests/limit.rs  +1",
                    "2 + block();",
                ],
            );
            shown(
                terminal,
                gated.rows(columns, height, expanded),
                columns,
                &[" UPDATE  src/limit.rs", "  └ "],
            );
            let mut pages = vec!["[ESC][2Jlimit.rs", "1 file · 1 line · 1 not read"];
            if expanded {
                pages.extend([
                    "1   let limit = 10;",
                    "src/gone.rs · Could not",
                    "second line",
                ]);
            }
            shown(
                terminal,
                reads.rows(columns, height, expanded),
                columns,
                &pages,
            );
        }
    }
}
