use super::{Terminal, probe};
use std::io::{self, Write};

pub(super) fn console(bash: &std::path::Path) {
    use crate::{events::Event, json::Value, tui::state::State};
    let terminal = Terminal::open(bash).unwrap();
    let (width, height) = terminal.size;
    let mut state = State {
        width,
        height,
        ..State::default()
    };
    state.begin_tool_turn();
    state.event(Event::ToolStarted {
        id: "update".into(),
        name: "edit".into(),
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
        name: "edit".into(),
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
    drop(terminal);
}
