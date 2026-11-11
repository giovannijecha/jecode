//! Renamed tool badges through the real renderer and native console.
use super::{Terminal, probe};
use crate::{
    json::Value,
    tui::{
        cards::{self, Presentation, Tool},
        line::Line,
        render::Renderer,
        test_screen::Screen,
        text, theme,
        view::Frame,
    },
};
use std::io::{self, Write};

pub(super) fn console(terminal: &Terminal) {
    let (physical_width, height) = terminal.size;
    let presentation = Presentation::default();
    for light in [false, true] {
        theme::set_light(light);
        for columns in [40, 80] {
            for state in ["running", "done", "failed", "stopped"] {
                let result = Value::object([
                    ("exit_code", Value::number(0)),
                    ("bytes_written", Value::number(1)),
                    ("status", Value::string("ready")),
                ]);
                let result = match state {
                    "failed" => Value::object([("error", Value::string("Fixture failure"))]),
                    "stopped" => Value::object([("cancelled", Value::Bool(true))]),
                    _ => result,
                };
                let mut rows = Vec::new();
                for (name, label) in [
                    ("bash", "BASH"),
                    ("process", "PROC"),
                    ("write", "SAVE"),
                    ("web", "HTTP"),
                    ("browser", "BROWSE"),
                    ("read", "SKILLS"),
                    ("mcp", "MCP CALL"),
                    ("lsp", "LSP CALL"),
                    ("steps", "SEQUENCE"),
                ] {
                    let arguments = Value::object([
                        (
                            "path",
                            Value::string(if name == "read" {
                                "skill:review"
                            } else {
                                "new.rs"
                            }),
                        ),
                        ("command", Value::string("cargo check")),
                        ("action", Value::string("snapshot")),
                    ]);
                    let tool = Tool {
                        name,
                        arguments: &arguments,
                        summary: "",
                        result: (state != "running").then_some(&result),
                        change: None,
                        presentation: &presentation,
                    };
                    let row = cards::card(tool, columns, false).rows.remove(0);
                    let filled: String = row
                        .spans
                        .iter()
                        .take_while(|span| span.background.is_some())
                        .map(|span| span.text.as_str())
                        .collect();
                    assert_eq!(text::cells(&filled), 8, "{name}, {state}");
                    assert_eq!(filled.trim(), label);
                    assert_eq!(
                        filled.len() - filled.trim_start().len(),
                        filled.len() - filled.trim_end().len()
                    );
                    rows.push(row);
                }
                for name in ["AGENT 1", "AGENT 12", "AGENT 123", "MAIN AGENT"] {
                    rows.push(cards::agent_badge(name, state));
                }
                paint(rows, columns, physical_width, height);
            }
        }
    }
    theme::set_light(false);
}

fn paint(rows: Vec<Line>, columns: usize, physical_width: usize, height: usize) {
    let expected: String = (0..height)
        .map(|index| {
            let value = rows.get(index).map_or_else(String::new, Line::plain);
            format!(
                "{value}{}",
                " ".repeat(physical_width - text::cells(&value))
            )
        })
        .collect();
    let count = rows.len();
    let frame = Frame {
        header: vec![],
        history: vec![],
        live: rows,
        cursor: None,
        composer: height - 1,
    };
    let output = Renderer::new().paint(&frame, columns, height);
    // Check the actual VT background runs, including the unfilled first cell after each badge.
    let mut screen = Screen::new(columns, height);
    screen.feed(&output);
    for row in &screen.backs[..count] {
        assert!(
            row[..8]
                .iter()
                .all(|background| background.is_some() && *background == row[0])
        );
        assert!(row[8..].iter().all(Option::is_none));
    }
    write!(io::stdout(), "{output}").unwrap();
    io::stdout().flush().unwrap();
    assert_eq!(probe().lines().nth(1).unwrap(), expected);
}
