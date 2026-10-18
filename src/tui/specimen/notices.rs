// Notices painted by the real composer renderer for review in a native
// terminal: cargo test --locked --offline specimen::notices -- --ignored --nocapture
use std::time::{Duration, Instant};

use crate::tui::{
    activity::Activity,
    feedback::Feedback,
    line::Line,
    selector::Selector,
    state::{Kind, State},
    theme::{MUTED, STRONG},
    view,
};

const WIDE: usize = 80;
const NARROW: usize = 44;
const MODEL: &str = "anthropic/claude-sonnet-5.5";
const DIRECTORY: &str = r"C:\Projects\example";

fn state(columns: usize) -> State {
    State {
        width: columns,
        effort: "high".into(),
        ..State::default()
    }
}

fn noticed(columns: usize, kind: Kind, text: &str) -> State {
    let mut state = state(columns);
    state.notice = Some(Feedback::result(kind, text));
    state
}

fn working(state: &mut State, label: &'static str, seconds: f64) {
    let mut activity = Activity::new();
    activity.label = label;
    activity.started = Instant::now() - Duration::from_secs_f64(seconds);
    state.activity = Some(activity);
}

fn states(columns: usize) -> Vec<(&'static str, State)> {
    let mut progress = state(columns);
    progress.notice = Some(Feedback::progress(Kind::Notice, "Attaching 2 files…"));

    let mut both = noticed(columns, Kind::Notice, "Complete · 48s · 6 tools");
    both.copy_notice = Some(Feedback::result(
        Kind::Notice,
        "Copied Response 3 to clipboard",
    ));

    let mut turn = noticed(
        columns,
        Kind::Notice,
        "Context compacted · 24 messages summarized",
    );
    working(&mut turn, "Writing the response", 92.0);

    let mut reconnecting = state(columns);
    working(&mut reconnecting, "Waiting", 64.0);
    reconnecting.notice = Some(Feedback::progress(
        Kind::Warning,
        "Reconnecting · attempt 2 in 4.0s · connection reset by peer",
    ));

    let mut panel = noticed(
        columns,
        Kind::Error,
        "Settings could not be saved: access is denied. (os error 5)",
    );
    panel.selector = Some(Selector::settings(MODEL, crate::effort::Effort::High, 80));

    vec![
        (
            "Result",
            noticed(columns, Kind::Notice, "Complete · 48s · 6 tools"),
        ),
        (
            "Startup",
            noticed(
                columns,
                Kind::Notice,
                "Previous session: Rate limiting for /login · /resume",
            ),
        ),
        ("Progress", progress),
        (
            "Warning",
            noticed(
                columns,
                Kind::Warning,
                "Interrupted · pending drafts paused · alt+↑ to review",
            ),
        ),
        (
            "Error",
            noticed(
                columns,
                Kind::Error,
                "Account disconnected · /settings to update the OpenRouter key",
            ),
        ),
        (
            "Long error",
            noticed(
                columns,
                Kind::Error,
                "Autosave failed: The process cannot access the file because it is being used by another process. (os error 32). Session kept in memory.",
            ),
        ),
        ("Result and copy", both),
        ("During a turn", turn),
        ("Reconnecting", reconnecting),
        ("Error over a panel", panel),
    ]
}

fn heading(rows: &mut Vec<(Line, usize)>, title: &str) {
    rows.push((Line::default(), WIDE));
    rows.push((Line::new(title, STRONG.bold()), WIDE));
    rows.push((Line::default(), WIDE));
}

#[test]
#[ignore = "prints the notices specimen for review in a native terminal"]
fn notices() {
    let mut rows = Vec::new();
    for columns in [WIDE, NARROW] {
        heading(&mut rows, &format!("Notices  ·  {columns} columns"));
        for (title, state) in states(columns) {
            rows.push((Line::new(title, MUTED), columns));
            rows.push((Line::default(), columns));
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
