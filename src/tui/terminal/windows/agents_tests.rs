use super::{Terminal, probe};
use crate::{
    context::Status,
    json::Value,
    tui::{
        line::Line,
        render::Renderer,
        state::State,
        text,
        theme::{BADGE_IDLE, LIT, METER_FILL},
        view::Frame,
    },
};
use std::io::{self, Write};

fn record() -> Value {
    Value::object([
        ("id", Value::string("a1")),
        ("number", Value::number(1)),
        ("title", Value::string("Inspect the search view")),
        ("state", Value::string("returned")),
        ("workspace", Value::string("isolated")),
        ("directory", Value::string("fixture")),
        ("sent_version", Value::number(2)),
        ("received_version", Value::number(2)),
        (
            "instructions",
            Value::string("Preserve ordering for empty queries"),
        ),
        ("activity", Value::string("Report returned")),
        ("tools", Value::Array(vec![])),
        ("files", Value::Array(vec![Value::string("src/search.rs")])),
        (
            "checks",
            Value::Array(vec![Value::string("cargo test passed")]),
        ),
        (
            "report",
            Value::object([
                ("version", Value::number(2)),
                ("text", Value::string("Search behavior implemented")),
            ]),
        ),
        ("reviewed", Value::Bool(false)),
    ])
}

fn paint(rows: Vec<Line>, width: usize, height: usize) {
    assert!(rows.len() <= height);
    let expected: String = (0..height)
        .map(|index| {
            let row = rows.get(index).map_or_else(String::new, Line::plain);
            format!(
                "{row}{}",
                " ".repeat(width.saturating_sub(text::cells(&row)))
            )
        })
        .collect();
    let frame = Frame {
        header: vec![],
        history: vec![],
        live: rows,
        cursor: None,
        composer: height - 1,
    };
    let output = Renderer::new().paint(&frame, width, height);
    write!(io::stdout(), "{output}").unwrap();
    io::stdout().flush().unwrap();
    assert_eq!(probe().lines().nth(1).unwrap(), expected);
}

pub(super) fn console() {
    let terminal = Terminal::open().unwrap();
    let (width, height) = terminal.size;
    let mut state = State {
        width,
        height,
        collaboration: vec![record()],
        context: Some(Status {
            tokens: 34_000,
            capacity: Some(100_000),
            auto_compact: 80,
        }),
        ..State::default()
    };
    state.editor.insert("main draft");
    state.editor.cursor = 4;
    let draft = state.editor.clone();
    let receipt = Value::object([
        ("type", Value::string("collaboration")),
        ("sequence", Value::number(1)),
        ("kind", Value::string("created")),
        ("id", Value::Null),
        ("title", Value::string("Create agents")),
        ("details", Value::Array(vec![Value::string("a1")])),
        ("records", Value::Array(vec![record()])),
    ]);
    state.collaboration_event(receipt);
    assert_eq!(state.toggle_tools(), Some((true, 1)));
    let expanded = crate::tui::view::frame(&state, "fixture/model", "fixture");
    let footer = expanded.live.last().unwrap();
    assert!(
        footer.plain().contains("AGENTS") && footer.plain().contains("1 for main agent review")
    );
    assert!(footer.plain().contains("34%"));
    assert!(footer.spans.iter().any(|span| span.text == " AGENTS "
        && span.style == LIT
        && span.background == Some(BADGE_IDLE)));
    assert!(footer.spans.iter().any(|span| span.style == METER_FILL));
    assert_eq!(expanded.live.len(), height);
    assert!(
        expanded
            .live
            .iter()
            .any(|row| row.plain().contains("AGENT 1"))
    );
    paint(expanded.live, width, height);
    state.agents_panel = Some(crate::tui::agents::Panel::default());
    let list = crate::tui::view::frame(&state, "fixture/model", "fixture");
    let shown = list
        .live
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(shown.contains("Agents · 1 open") && shown.contains("returned · main review"));
    paint(list.live, width, height);
    state.agents_panel.as_mut().unwrap().inside = Some("a1".into());
    let inside = crate::tui::view::frame(&state, "fixture/model", "fixture");
    let shown = inside
        .live
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(shown.contains("Preserve ordering for empty queries"));
    assert!(shown.contains("AGENT 1  Returned · Inspect the search view"));
    assert!(
        inside
            .live
            .last()
            .unwrap()
            .plain()
            .contains("Esc  back to agents")
    );
    assert!(!shown.contains("fixture/model") && !shown.contains("34%"));
    assert_eq!(inside.live.len(), height);
    assert_eq!(state.editor, draft);
    paint(inside.live, width, height);
    state.agents_panel = None;
    if let Value::Object(fields) = &mut state.collaboration[0] {
        fields.insert("reviewed".into(), Value::Bool(true));
    }
    let reviewed = crate::tui::view::frame(&state, "fixture/model", "fixture");
    let footer = reviewed.live.last().unwrap().plain();
    assert!(!footer.contains("AGENTS"));
    assert!(footer.contains("fixture") && footer.contains("34%"));
    assert_eq!(reviewed.live.len(), height);
    paint(reviewed.live, width, height);
    drop(terminal);
}
