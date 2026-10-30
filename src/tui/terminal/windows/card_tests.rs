use super::{Terminal, probe};
use std::io::{self, Write};

pub(super) fn console() {
    use crate::{events::Event, json::Value, tui::state::State};
    let terminal = Terminal::open().unwrap();
    let (width, height) = terminal.size;
    let mut state = State {
        width,
        height,
        ..State::default()
    };
    state.begin_tool_turn();
    state.event(Event::ToolStarted {
        id: "update".into(),
        name: "write".into(),
        arguments: Value::object([("path", Value::string("src/limit.rs"))]),
    });
    state.event(Event::ToolChanged {
        id: "update".into(),
        change: crate::file_change::Change::between(
            Some("let limit = 10;\n"),
            "let limit = LOGIN_LIMIT;\n",
        )
        .unwrap(),
    });
    state.event(Event::ToolFinished {
        id: "update".into(),
        name: "write".into(),
        summary: "wrote file".into(),
        result: Value::object([("bytes_written", Value::number(25))]),
    });
    state.event(Event::ToolStarted {
        id: "verify".into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string("cargo test --locked --offline"))]),
    });
    let result = Value::object([
        ("exit_code", Value::number(0)),
        (
            "checkpoint",
            Value::object([("status", Value::string("partial"))]),
        ),
    ]);
    state.event(Event::ToolFinished {
        id: "verify".into(),
        name: "bash".into(),
        summary: crate::events::tool_summary("bash", &result),
        result,
    });
    for (id, name, arguments, result) in [
        (
            "fetch",
            "web",
            Value::object([
                ("action", Value::string("fetch")),
                ("url", Value::string("https://example.org/")),
            ]),
            Value::object([
                ("status", Value::number(200)),
                ("bytes", Value::number(12)),
                ("path", Value::string("tmp:web/fetches/page.body")),
            ]),
        ),
        (
            "capture",
            "browser",
            Value::object([
                ("action", Value::string("screenshot")),
                ("tab_id", Value::string("tab-1")),
            ]),
            Value::object([
                ("tab_id", Value::string("tab-1")),
                (
                    "screenshot_path",
                    Value::string("tmp:browser/screenshot.png"),
                ),
            ]),
        ),
    ] {
        state.event(Event::ToolStarted {
            id: id.into(),
            name: name.into(),
            arguments,
        });
        state.event(Event::ToolFinished {
            id: id.into(),
            name: name.into(),
            summary: crate::events::tool_summary(name, &result),
            result,
        });
    }
    let rows: Vec<_> = state.rows().into_iter().flatten().collect();
    assert!(
        rows.iter()
            .any(|row| row.plain().starts_with(" UPDATE  src/limit.rs"))
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("1 − let limit = 10;"))
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("1 + let limit = LOGIN_LIMIT;"))
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("Activity  3 calls"))
    );
    assert!(rows.iter().any(|row| row.plain().contains("1 screenshot")));
    state.finish_tool_turn();
    assert_eq!(state.toggle_tools(), Some((true, 4)));
    let rows: Vec<_> = state.rows().into_iter().flatten().collect();
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("Checkpoint partial")),
        "expanded Bash details lost the partial checkpoint status"
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("WEB  fetch https://example.org/"))
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("BROWSER  screenshot"))
    );
    let plain: Vec<_> = rows.iter().map(crate::tui::line::Line::plain).collect();
    assert!(plain.iter().any(|row| row.contains("tab-1")), "{plain:#?}");
    assert!(rows.iter().any(|row| {
        row.plain()
            .contains("\"screenshot_path\": \"tmp:browser/screenshot.png\"")
    }));
    let expected: String = (0..height)
        .map(|index| {
            let text = rows.get(index).map_or_else(String::new, |row| row.plain());
            format!(
                "{text}{}",
                " ".repeat(width.saturating_sub(crate::tui::text::cells(&text)))
            )
        })
        .collect();
    let frame = crate::tui::view::Frame {
        header: vec![],
        history: vec![],
        live: rows,
        cursor: None,
        composer: height - 1,
    };
    let output = crate::tui::render::Renderer::new().paint(&frame, width, height);
    write!(io::stdout(), "{output}").unwrap();
    io::stdout().flush().unwrap();
    let snapshot = probe();
    assert_eq!(snapshot.lines().nth(1).unwrap(), expected);
    discovery_console(&terminal);
    concurrent_cards(&terminal);
    drop(terminal);
}

/// Verify native cells while a later read finishes before its earlier peer.
fn concurrent_cards(terminal: &Terminal) {
    use crate::{events::Event, json::Value, tui::state::State};

    let (physical_width, height) = terminal.size;
    for columns in [40, 80] {
        let mut state = State {
            width: columns,
            height,
            ..State::default()
        };
        state.begin_tool_turn();
        for (id, path) in [("first", "src/first.rs"), ("second", "src/second.rs")] {
            state.event(Event::ToolStarted {
                id: id.into(),
                name: "read".into(),
                arguments: Value::object([
                    ("path", Value::string(path)),
                    ("work", Value::string("Inspect sources")),
                ]),
            });
        }
        for phase in 0..3 {
            if phase > 0 {
                state.event(Event::ToolFinished {
                    id: if phase == 1 { "second" } else { "first" }.into(),
                    name: "read".into(),
                    summary: "1 line".into(),
                    result: Value::object([("lines_returned", Value::number(1))]),
                });
            }
            let rows: Vec<_> = state.rows().into_iter().flatten().collect();
            let plain: Vec<_> = rows.iter().map(crate::tui::line::Line::plain).collect();
            let all = plain.join("\n");
            assert!(all.contains(" WORK  Inspect sources  2 calls"), "{all}");
            if phase == 0 {
                assert!(
                    all.contains("first.rs") && all.contains("second.rs"),
                    "{all}"
                );
                assert!(all.contains("2 running"), "{all}");
            } else if phase == 1 {
                assert!(
                    all.contains("first.rs") && all.contains("second.rs"),
                    "{all}"
                );
                assert!(all.contains("1 running"), "{all}");
            }
            let expected: String = (0..height)
                .map(|index| {
                    let line = plain.get(index).map_or("", String::as_str);
                    format!(
                        "{line}{}",
                        " ".repeat(physical_width.saturating_sub(crate::tui::text::cells(line)))
                    )
                })
                .collect();
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
            assert_eq!(
                probe().lines().nth(1).unwrap(),
                expected,
                "{columns} columns, phase {phase}"
            );
        }
    }
}

/// Exercise discovery cards through the native console and the full frame renderer.
fn discovery_console(terminal: &Terminal) {
    use crate::{events::Event, json::Value, tui::state::State};

    let (physical_width, height) = terminal.size;
    for columns in [40, 80] {
        for (name, arguments, result) in [
            (
                "list",
                Value::object([
                    ("path", Value::string("src")),
                    ("description", Value::string("Inspect source tree")),
                ]),
                Value::object([
                    ("entries_returned", Value::number(2)),
                    ("complete", Value::Bool(true)),
                    (
                        "entries",
                        Value::Array(vec![
                            Value::object([
                                ("path", Value::string("src/agent.rs")),
                                ("type", Value::string("file")),
                                ("depth", Value::number(0)),
                                ("size", Value::number(100)),
                            ]),
                            Value::object([
                                ("path", Value::string("src/tui")),
                                ("type", Value::string("directory")),
                                ("depth", Value::number(0)),
                            ]),
                        ]),
                    ),
                ]),
            ),
            (
                "search",
                Value::object([
                    ("query", Value::string("needle")),
                    ("path", Value::string("src")),
                    ("description", Value::string("Find callers")),
                ]),
                Value::object([
                    ("matches_returned", Value::number(1)),
                    ("files_searched", Value::number(3)),
                    ("complete", Value::Bool(false)),
                    ("errors_count", Value::number(1)),
                    (
                        "matches",
                        Value::Array(vec![Value::object([
                            ("path", Value::string("src/agent.rs")),
                            ("line", Value::number(27)),
                            ("text", Value::string("let needle = find();")),
                            ("text_truncated", Value::Bool(false)),
                            (
                                "before",
                                Value::Array(vec![Value::object([
                                    ("line", Value::number(26)),
                                    ("text", Value::string("// search")),
                                ])]),
                            ),
                        ])]),
                    ),
                    (
                        "errors",
                        Value::Array(vec![Value::object([
                            ("path", Value::string("src/private.rs")),
                            ("error", Value::string("Permission denied")),
                        ])]),
                    ),
                ]),
            ),
            (
                "map-empty",
                Value::object([("path", Value::string("src"))]),
                Value::object([
                    ("entries_returned", Value::number(0)),
                    ("files", Value::number(0)),
                    ("entries", Value::Array(vec![])),
                    ("complete", Value::Bool(true)),
                ]),
            ),
            (
                "map-note",
                Value::object([("path", Value::string("src"))]),
                Value::object([
                    ("entries_returned", Value::number(1)),
                    ("files", Value::number(1)),
                    ("complete", Value::Bool(true)),
                    (
                        "entries",
                        Value::Array(vec![Value::object([
                            ("kind", Value::string("note")),
                            ("id", Value::string("n1")),
                            ("title", Value::string("Renderer boundary")),
                            ("origin", Value::string("model")),
                            ("freshness", Value::string("changed")),
                            (
                                "sources",
                                Value::Array(vec![Value::object([
                                    ("path", Value::string("src/agent.rs")),
                                    ("line", Value::number(12)),
                                    ("status", Value::string("changed")),
                                ])]),
                            ),
                        ])]),
                    ),
                ]),
            ),
        ] {
            let tool_name = if name.starts_with("map-") {
                "map"
            } else {
                name
            };
            for expanded in [false, true] {
                let mut state = State {
                    width: columns,
                    height,
                    ..State::default()
                };
                state.begin_tool_turn();
                state.event(Event::ToolStarted {
                    id: name.into(),
                    name: tool_name.into(),
                    arguments: arguments.clone(),
                });
                state.event(Event::ToolFinished {
                    id: name.into(),
                    name: tool_name.into(),
                    summary: crate::events::tool_summary(tool_name, &result),
                    result: result.clone(),
                });
                if expanded {
                    state.finish_tool_turn();
                    assert_eq!(state.toggle_tools(), Some((true, 1)));
                }
                let rows: Vec<_> = state.rows().into_iter().flatten().collect();
                let plain: Vec<_> = rows.iter().map(crate::tui::line::Line::plain).collect();
                let detail = match name {
                    "list" => "src/agent.rs",
                    "search" => "src/agent.rs:27",
                    "map-empty" => "No map entries",
                    _ => "src/agent.rs:12",
                };
                assert_eq!(plain.iter().any(|row| row.contains(detail)), expanded);
                assert!(
                    plain
                        .iter()
                        .all(|row| crate::tui::text::cells(row) <= columns)
                );
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
                            " ".repeat(
                                physical_width.saturating_sub(crate::tui::text::cells(text))
                            )
                        )
                    })
                    .collect();
                let snapshot = probe();
                assert_eq!(
                    snapshot.lines().nth(1).unwrap(),
                    expected,
                    "native {name} {} at {columns} columns",
                    if expanded { "expanded" } else { "compact" }
                );
            }
        }
    }
}
