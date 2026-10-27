// Information and process panels painted by the real composer renderer for
// review in a native terminal:
// cargo test --locked --offline specimen::panels -- --ignored --nocapture
use std::collections::VecDeque;

use crate::tools::processes::{Page, Process};
use crate::tui::{
    information::Information,
    line::Line,
    processes::{Logs, Panel},
    state::State,
    theme::{MUTED, STRONG},
    view,
};

const WIDE: usize = 80;
const NARROW: usize = 44;
const HEIGHT: usize = 30;
const MODEL: &str = "anthropic/claude-sonnet-5.5";
const DIRECTORY: &str = r"C:\Projects\example";

fn state(columns: usize) -> State {
    State {
        width: columns,
        height: HEIGHT,
        effort: "high".into(),
        ..State::default()
    }
}

fn informed(columns: usize, information: Information) -> State {
    let mut state = state(columns);
    state.information = Some(information);
    state
}

fn process(id: &str, command: &str, status: &'static str, exit_code: Option<i32>) -> Process {
    Process {
        id: id.into(),
        command: command.into(),
        status,
        exit_code,
        error: None,
    }
}

fn running() -> Vec<Process> {
    vec![
        process("p1", "npm run dev", "running", None),
        process("p2", "cargo watch -x \"test --locked\"", "running", None),
        process("p3", "python -m http.server 8000", "running", None),
    ]
}

fn listed(columns: usize, panel: Panel) -> State {
    let mut state = state(columns);
    state.processes = Some(panel);
    state
}

fn logs(process: Process, stream: usize, streams: [&str; 2], follow: bool) -> Logs {
    let mut logs = Logs {
        id: process.id.clone(),
        process: Some(process),
        stream,
        views: Default::default(),
        interactive: false,
        input_mode: false,
    };
    for (view, text) in logs.views.iter_mut().zip(streams) {
        let length = text.len() as u64;
        if length > 0 {
            view.pages = VecDeque::from([Page {
                text: text.into(),
                start: 0,
                end: length,
                length,
            }]);
        }
        view.follow = follow;
    }
    logs
}

fn output() -> String {
    let mut text = String::from("> acme-api@1.4.0 dev\n> vite --port 5173\n\n");
    text.push_str("  VITE v5.4.0  ready in 412 ms\n\n  Local:   http://localhost:5173/\n\n");
    for second in 1..=24 {
        text.push_str(&format!(
            "12:04:{second:02} [vite] hmr update /src/routes/auth.ts, /src/routes/limiter.ts (x{second})\n"
        ));
    }
    text
}

fn viewing(columns: usize, logs: Logs) -> State {
    let mut panel = Panel::new(running());
    panel.logs = Some(logs);
    listed(columns, panel)
}

fn states(columns: usize) -> Vec<(&'static str, State)> {
    let help = |section, offset| {
        let mut information = Information::help();
        information.section = section;
        information.offset = offset;
        informed(columns, information)
    };
    let temporary = Information::new(
        "Temporary files".into(),
        vec![
            (
                "Path".into(),
                r"C:\Users\example\.jecode\sessions\example-session\tmp".into(),
            ),
            ("Files".into(), "3".into()),
            ("Directories".into(), "1".into()),
            ("Size".into(), "18432 bytes".into()),
            (
                "Retention".into(),
                "Kept until /tmp clean, including after resume".into(),
            ),
        ],
    )
    .context("this session");
    let skills = Information::new(
        "Skills".into(),
        vec![
            (
                "release-notes".into(),
                "Draft release notes from the changes merged since the last tag".into(),
            ),
            (
                "review".into(),
                "Review the working tree for correctness bugs before a commit".into(),
            ),
            (
                "Skipped".into(),
                "skills/broken/SKILL.md has no description".into(),
            ),
        ],
    )
    .aside("2 available skills · 1 skipped");

    let mut list = Panel::new(running());
    list.selected = 1;
    let mut stop = Panel::new(running());
    stop.selected = 1;
    stop.confirm_stop = Some(running().remove(1));

    let following = logs(running().remove(0), 0, [&output(), ""], true);
    let paused = logs(running().remove(0), 0, [&output(), ""], false);
    let failed = process("p4", "cargo test --locked limiter", "exited", Some(101));
    let errors = logs(
        failed,
        1,
        [
            "running 2 tests\ntest limiter::allows ... ok\ntest limiter::blocks ... FAILED\n",
            "error: test failed, to rerun pass `--lib`\n",
        ],
        true,
    );

    vec![
        ("Help", help(0, 0)),
        ("Help · scrolled", help(0, 10)),
        ("Help · keys", help(1, 0)),
        ("Skills", informed(columns, skills)),
        ("Temporary files", informed(columns, temporary)),
        ("Processes · none", listed(columns, Panel::new(vec![]))),
        ("Processes", listed(columns, list)),
        ("Processes · stop", listed(columns, stop)),
        ("Output · following", viewing(columns, following)),
        ("Output · paused at the start", viewing(columns, paused)),
        ("Errors · finished process", viewing(columns, errors)),
    ]
}

fn heading(rows: &mut Vec<(Line, usize)>, title: &str) {
    rows.push((Line::default(), WIDE));
    rows.push((Line::new(title, STRONG.bold()), WIDE));
    rows.push((Line::default(), WIDE));
}

#[test]
#[ignore = "prints the panels specimen for review in a native terminal"]
fn panels() {
    let mut rows = Vec::new();
    for columns in [WIDE, NARROW] {
        heading(&mut rows, &format!("Panels  ·  {columns} columns"));
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
