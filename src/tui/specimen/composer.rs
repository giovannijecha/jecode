// The lower area painted by the real composer renderer for review in a native
// terminal: cargo test --locked --offline specimen::composer -- --ignored --nocapture
use std::time::{Duration, Instant};

use crate::context::Status;
use crate::tui::{
    activity::Activity,
    feedback::Feedback,
    line::Line,
    state::{Kind, State},
    theme::{MUTED, STRONG},
    view,
};

const WIDE: usize = 80;
const NARROW: usize = 44;
const MODEL: &str = "anthropic/claude-sonnet-5.5";
const DIRECTORY: &str = r"C:\Projects\example";

/// The engine projection shown by the footer: 122,400 of 200,000 tokens.
const STATUS: Status = Status {
    tokens: 122_400,
    capacity: Some(200_000),
    auto_compact: 80,
};

fn state(columns: usize) -> State {
    State {
        width: columns,
        effort: "high".into(),
        context: Some(STATUS),
        ..State::default()
    }
}

fn working(state: &mut State, seconds: f64) {
    let mut activity = Activity::new();
    activity.started = Instant::now() - Duration::from_secs_f64(seconds);
    state.activity = Some(activity);
}

fn queued(state: &mut State) {
    state.queue.push("also update the README").unwrap();
    state
        .queue
        .push("and bump the version\nin Cargo.toml")
        .unwrap();
}

fn states(columns: usize) -> Vec<(&'static str, State)> {
    let idle = state(columns);

    let mut typing = state(columns);
    typing.editor.insert(
        "add rate limiting to /login\nreturn 429 with a Retry-After header\nkeep the window in config\nlog blocked attempts\ncover it with tests\nupdate the docs\nmention it in the changelog",
    );

    let mut work = state(columns);
    working(&mut work, 12.4);
    work.editor.insert("next draft");

    let mut queue = state(columns);
    working(&mut queue, 31.7);
    queued(&mut queue);

    let mut long = state(columns);
    working(&mut long, 4252.0);
    queued(&mut long);

    let mut editing = state(columns);
    working(&mut editing, 48.2);
    queued(&mut editing);
    let slot = editing.queue.slot(0).unwrap();
    editing.queue.begin_edit(slot, &mut editing.editor).unwrap();

    let mut history = state(columns);
    history.history.record("explain the session store");
    history.history.navigate(&mut history.editor, true);

    let mut inspection = state(columns);
    inspection.begin_tool_turn();
    inspection.event(crate::events::Event::ToolStarted {
        id: "read".into(),
        name: "read".into(),
        arguments: crate::json::Value::object([(
            "path",
            crate::json::Value::string("src/main.rs"),
        )]),
    });
    inspection.event(crate::events::Event::ToolFinished {
        id: "read".into(),
        name: "read".into(),
        summary: "Read 1 line".into(),
        result: crate::json::Value::object([(
            "content",
            crate::json::Value::string("1: fn main() {}"),
        )]),
    });
    inspection.finish_tool_turn();

    let mut reconnecting = state(columns);
    working(&mut reconnecting, 64.0);
    reconnecting.progress = Some(
        Feedback::progress(
            Kind::Warning,
            "Reconnecting · attempt 2 · connection reset by peer",
        )
        .until(Instant::now() + Duration::from_millis(3500)),
    );

    let mut unknown = state(columns);
    unknown.context = Some(Status {
        capacity: None,
        ..STATUS
    });

    vec![
        ("Idle", idle),
        ("Context capacity unknown", unknown),
        ("Typing a long draft", typing),
        ("Working", work),
        ("Working with queued messages", queue),
        ("Working past an hour", long),
        ("Editing a queued message", editing),
        ("Prompt history", history),
        ("Completed turn tools", inspection),
        ("Reconnecting", reconnecting),
    ]
}

fn heading(rows: &mut Vec<(Line, usize)>, title: &str) {
    rows.push((Line::default(), WIDE));
    rows.push((Line::new(title, STRONG.bold()), WIDE));
    rows.push((Line::default(), WIDE));
}

#[test]
#[ignore = "prints the composer specimen for review in a native terminal"]
fn composer() {
    let mut rows = Vec::new();
    for columns in [WIDE, NARROW] {
        heading(&mut rows, &format!("Composer  ·  {columns} columns"));
        for (title, state) in states(columns) {
            rows.push((Line::new(title, MUTED), columns));
            for line in view::composer_frame(&state, MODEL, DIRECTORY).live {
                rows.push((line, columns));
            }
            rows.push((Line::default(), columns));
        }
    }
    let painted: String = rows
        .iter()
        .map(|(line, columns)| format!("{}\n", line.paint(*columns)))
        .collect();
    print!("{painted}");
}
