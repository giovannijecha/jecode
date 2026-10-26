// Work chapters painted from real TUI events and rows in a native terminal:
// cargo test --locked --offline specimen::work_chapters -- --ignored --nocapture
use crate::{
    events::{Event, tool_summary},
    file_change::Change,
    json::Value,
    tui::{
        line::Line,
        state::{Kind, State},
        theme::STRONG,
    },
};

fn call(state: &mut State, id: &str, name: &str, arguments: Value, result: Value) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: name.into(),
        arguments,
    });
    state.event(Event::ToolFinished {
        id: id.into(),
        name: name.into(),
        summary: tool_summary(name, &result),
        result,
    });
}

fn changed_call(
    state: &mut State,
    id: &str,
    name: &str,
    arguments: Value,
    result: Value,
    before: Option<&str>,
    after: &str,
) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: name.into(),
        arguments,
    });
    state.event(Event::ToolChanged {
        id: id.into(),
        change: Change::between(before, after).unwrap(),
    });
    state.event(Event::ToolFinished {
        id: id.into(),
        name: name.into(),
        summary: tool_summary(name, &result),
        result,
    });
}

fn scenario(columns: usize) -> State {
    let mut state = State {
        width: columns,
        ..State::default()
    };
    state.message(
        Kind::User,
        "Inspect the parser, update it, and run the focused checks.",
    );
    state.begin_tool_turn();
    call(
        &mut state,
        "read-parser",
        "read",
        Value::object([
            ("path", Value::string("src/parser.rs")),
            ("work", Value::string("Inspect the parser")),
        ]),
        Value::object([
            (
                "content",
                Value::string("1: fn parse() {\n2:     old();\n3: }\n"),
            ),
            ("lines_returned", Value::number(3)),
            ("offset", Value::number(1)),
        ]),
    );
    call(
        &mut state,
        "check-tree",
        "bash",
        Value::object([
            ("command", Value::string("git status --short")),
            ("work", Value::string("Inspect the parser")),
        ]),
        Value::object([
            ("stdout", Value::string(" M src/parser.rs\n")),
            ("exit_code", Value::number(0)),
        ]),
    );
    call(
        &mut state,
        "fetch-spec",
        "web",
        Value::object([
            ("action", Value::string("fetch")),
            ("url", Value::string("https://example.org/parser")),
            ("work", Value::string("Inspect the parser")),
        ]),
        Value::object([
            ("status", Value::number(200)),
            ("bytes", Value::number(432)),
        ]),
    );
    call(
        &mut state,
        "browser-snapshot",
        "browser",
        Value::object([
            ("action", Value::string("snapshot")),
            ("tab_id", Value::string("tab-1")),
            ("work", Value::string("Inspect the parser")),
        ]),
        Value::object([("snapshot_path", Value::string("tmp:browser/parser.txt"))]),
    );
    call(
        &mut state,
        "process-list",
        "process",
        Value::object([
            ("action", Value::string("list")),
            ("work", Value::string("Inspect the parser")),
        ]),
        Value::object([("processes", Value::Array(vec![]))]),
    );

    changed_call(
        &mut state,
        "update-parser",
        "edit",
        Value::object([
            ("path", Value::string("src/parser.rs")),
            ("old_text", Value::string("old();")),
            ("new_text", Value::string("parse_tokens();")),
        ]),
        Value::object([("bytes_written", Value::number(36))]),
        Some("fn parse() {\n    old();\n}\n"),
        "fn parse() {\n    parse_tokens();\n}\n",
    );
    changed_call(
        &mut state,
        "write-test",
        "write",
        Value::object([
            ("path", Value::string("tests/parser.rs")),
            ("content", Value::string("#[test]\nfn parses_tokens() {}\n")),
        ]),
        Value::object([("bytes_written", Value::number(29))]),
        None,
        "#[test]\nfn parses_tokens() {}\n",
    );
    state.message(
        Kind::Assistant,
        "The parser and its focused test are updated.",
    );
    call(
        &mut state,
        "verify-parser",
        "bash",
        Value::object([
            (
                "command",
                Value::string("cargo test --locked --offline parser"),
            ),
            ("work", Value::string("Verify the parser")),
        ]),
        Value::object([
            ("stderr", Value::string("error: parser test failed\n")),
            ("exit_code", Value::number(101)),
        ]),
    );
    state.close_tools();
    state.finish_tool_turn();
    state.message(
        Kind::Assistant,
        "The focused parser test failed; I will inspect the error.",
    );
    state
}

fn plain(state: &State) -> String {
    state
        .rows()
        .into_iter()
        .flatten()
        .map(|line| line.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[ignore = "prints compact and expanded Work chapters in a native terminal"]
fn work_chapters() {
    let mut printed = Vec::<(Line, usize)>::new();
    for columns in [80, 40] {
        let mut state = scenario(columns);
        for (title, expanded) in [("Compact", false), ("Expanded", true)] {
            if expanded {
                assert!(state.toggle_tools().is_some());
            }
            let output = plain(&state);
            assert!(
                output.contains("Inspect the parser"),
                "{title} at {columns}: {output}"
            );
            assert!(
                output.contains("Verify the parser"),
                "{title} at {columns}: {output}"
            );
            if !expanded {
                assert!(
                    output.contains("1 file read"),
                    "{title} at {columns}: {output}"
                );
                assert!(
                    output.contains("1 web call"),
                    "{title} at {columns}: {output}"
                );
                assert!(
                    output.contains("1 snapshot"),
                    "{title} at {columns}: {output}"
                );
            }
            assert!(output.contains("Update") && output.contains("Write"));
            assert!(
                output.contains("Exit 101"),
                "{title} at {columns}: {output}"
            );
            printed.push((
                Line::new(
                    &format!("Work chapters · {title} · {columns} columns"),
                    STRONG.bold(),
                ),
                columns,
            ));
            for block in state.rows() {
                for line in block {
                    printed.push((line, columns));
                }
            }
            printed.push((Line::default(), columns));
        }
    }
    let painted: String = printed
        .iter()
        .map(|(line, columns)| format!("{}\n", line.paint(*columns)))
        .collect();
    print!("{painted}");
}
