use super::{Terminal, probe};
use crate::{
    events::Event,
    tui::{activity::Activity, render::Renderer, state::State, text, view::Frame},
};
use std::{
    io::{self, Write},
    path::Path,
    time::Duration,
};

pub(super) fn console(bash: &Path) {
    let terminal = Terminal::open(bash).unwrap();
    let (width, height) = terminal.size;
    let mut activity = Activity::new();
    activity.started -= Duration::from_secs(70 * 60);
    let started = activity.started;
    let mut state = State {
        activity: Some(activity),
        ..State::default()
    };
    let mut rows = Vec::new();
    for (event, label) in [
        (Event::Preparing, "Preparing"),
        (
            Event::Waiting {
                model: "fixture/model".into(),
            },
            "Waiting for the model",
        ),
        (Event::Reasoning, "Thinking"),
        (Event::Working, "Working"),
        (
            Event::Streaming {
                text: "Native response".into(),
            },
            "Writing the response",
        ),
        (
            Event::Recovering {
                attempt: 1,
                delay: Duration::from_secs(2),
                error: "fixture".into(),
            },
            "Reconnecting",
        ),
    ] {
        state.event(event);
        let activity = state.activity.as_ref().unwrap();
        assert_eq!(activity.started, started);
        let row = activity.line(width, "Esc stops");
        assert!(row.plain().contains(label));
        assert!(!row.plain().contains("70m"));
        rows.push(row);
    }
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
    drop(terminal);
}
