// Notices painted by the real composer renderer for review in a native
// terminal: cargo test --locked --offline specimen::notices -- --ignored --nocapture
use std::time::{Duration, Instant};

use crate::tui::{
    activity::Activity,
    feedback::{Ending, Feedback},
    line::Line,
    selector::Selector,
    state::{Kind, Recent, State},
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
    with(columns, Feedback::result(kind, text))
}

fn with(columns: usize, notice: Feedback) -> State {
    let mut state = state(columns);
    state.notify(notice);
    state
}

fn working(state: &mut State, seconds: f64) {
    state.status = "Working…".into();
    let activity = Activity::with_elapsed(Duration::from_secs_f64(seconds));
    state.activity = Some(activity);
}

fn states(columns: usize) -> Vec<(&'static str, State)> {
    let mut progress = state(columns);
    progress.progress = Some(Feedback::progress(Kind::Notice, "Attaching 2 files"));

    let mut compacting = state(columns);
    working(&mut compacting, 92.0);
    compacting.progress = Some(Feedback::progress(
        Kind::Notice,
        "Compacting context · anthropic/claude-sonnet-5.5 · original conversation kept",
    ));

    let done = with(columns, Feedback::ending(Ending::Done, "48s · 6 tools"));

    let mut both = noticed(columns, Kind::Notice, "Checkpoint saved · rc-12");
    both.copy_notice = Some(Feedback::result(
        Kind::Notice,
        "Copied Response 3 to clipboard",
    ));

    let mut startup = state(columns);
    startup.opening.recent.push(Recent {
        title: "Rate limiting for /login".into(),
        age: "yesterday".into(),
    });

    let mut turn = noticed(
        columns,
        Kind::Notice,
        "Context compacted · 24 messages summarized",
    );
    working(&mut turn, 92.0);

    let mut reconnecting = state(columns);
    working(&mut reconnecting, 64.0);
    reconnecting.recovery = Some(
        Feedback::progress(
            Kind::Warning,
            "Rate limited · attempt 2 · OpenRouter HTTP 429: rate limit exceeded",
        )
        .until(Instant::now() + Duration::from_millis(3500)),
    );

    let mut panel = noticed(
        columns,
        Kind::Error,
        "Settings could not be saved: access is denied. (os error 5)",
    );
    panel.selector = Some(Selector::settings(
        MODEL,
        crate::effort::Effort::High,
        80,
        &crate::backend::openrouter::Routing::Auto,
        Default::default(),
        std::path::Path::new("fixture/config.json"),
    ));

    vec![
        (
            "Result",
            noticed(columns, Kind::Notice, "Checkpoint saved · rc-12"),
        ),
        ("Done", done),
        (
            "Setting changed",
            with(
                columns,
                Feedback::result(
                    Kind::Notice,
                    "Auto-compact: 80% → 55% · this and new conversations",
                )
                .change(),
            ),
        ),
        (
            "Model changed",
            with(
                columns,
                Feedback::result(
                    Kind::Notice,
                    "Model: anthropic/claude-sonnet-5.5 (high) → openai/gpt-5.5 (medium) · this conversation",
                )
                .change(),
            ),
        ),
        ("Startup · recent", startup),
        ("Progress", progress),
        (
            "Tool details confirmation",
            noticed(columns, Kind::Notice, "Tool details expanded · 6 tools"),
        ),
        ("Compacting", compacting),
        (
            "Stopped",
            with(
                columns,
                Feedback::ending(Ending::Stopped, "17s · your next message continues from here"),
            ),
        ),
        (
            "Error",
            with(
                columns,
                Feedback::ending(
                    Ending::Failed,
                    "OpenRouter HTTP 401: User not found. · prompt restored",
                )
                .action("/settings", "update key"),
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
        ("Rate limited", reconnecting),
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
            let lines = if state.opening.recent.is_empty() {
                view::composer_frame(&state, MODEL, DIRECTORY).live
            } else {
                let frame = view::frame(&state, MODEL, DIRECTORY);
                frame
                    .header
                    .into_iter()
                    .chain(frame.live.into_iter().skip(frame.composer))
                    .collect()
            };
            for line in lines {
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
