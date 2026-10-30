use super::{Terminal, probe};
use crate::{
    events::Event,
    json::Value,
    tui::{
        activity::Activity,
        cards::{self, Presentation, Tool},
        line::Line,
        render::Renderer,
        state::State,
        text,
        view::Frame,
    },
};
use std::{
    io::{self, Write},
    time::Duration,
};

pub(super) fn console() {
    let terminal = Terminal::open().unwrap();
    let (width, height) = terminal.size;
    let activity = Activity::with_elapsed(Duration::from_secs(70 * 60));
    let started = activity.started;
    let mut state = State {
        activity: Some(activity),
        width,
        height,
        ..State::default()
    };
    let mut rows = Vec::new();
    // Phases never restart the turn's timer, which reads in hours past 60 minutes.
    for event in [
        Event::Preparing,
        Event::Waiting {
            model: "fixture/model".into(),
        },
        Event::Reasoning,
        Event::Working,
        Event::Streaming {
            text: "Native response".into(),
        },
        Event::Recovering {
            attempt: 1,
            delay: Duration::from_secs(2),
            error: "fixture".into(),
            reason: None,
        },
    ] {
        state.event(event);
        let activity = state.activity.as_ref().unwrap();
        assert_eq!(activity.started, started);
        let row = crate::tui::view::composer_frame(&state, "fixture/model", "project")
            .live
            .remove(0);
        assert!(row.plain().contains("1h 10m"), "{}", row.plain());
        rows.push(row);
    }
    paint_rows(rows, width, height);

    let arguments = Value::object([
        ("path", Value::string("large.txt")),
        (
            "edits",
            Value::Array(vec![
                Value::object([
                    ("old_text", Value::string("alpha")),
                    ("new_text", Value::string("gamma")),
                ]),
                Value::object([
                    ("old_text", Value::string("gamma beta")),
                    ("new_text", Value::string("delta")),
                ]),
            ]),
        ),
    ]);
    let result = Value::object([("bytes_written", Value::number(1_048_577))]);
    let presentation = Presentation {
        expanded: true,
        ..Presentation::default()
    };
    let rows = cards::render(
        Tool {
            name: "edit",
            arguments: &arguments,
            result: Some(&result),
            change: None,
            summary: "complete",
            presentation: &presentation,
        },
        width,
    );
    assert!(
        rows.iter()
            .any(|row| row.plain().contains("Applied 2 replacements"))
    );
    assert!(rows.iter().any(|row| row.plain().contains("Replacement 2")));
    assert!(!rows.iter().any(|row| row.plain().contains("No changes")));
    paint_rows(rows, width, height);
    drop(terminal);
}

fn paint_rows(rows: Vec<Line>, width: usize, height: usize) {
    assert!(rows.len() <= height);
    let expected: String = (0..height)
        .map(|index| {
            let row = rows.get(index).map_or_else(String::new, |row| row.plain());
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
